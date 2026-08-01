/*
 * dxt.c - DXT1/3/5 decoder
 *
 * DDS file layout (128-byte header):
 *   [0..3]   magic "DDS " (0x20534444 LE)
 *   [4..7]   header size (always 124)
 *   [8..11]  flags
 *   [12..15] dwHeight   <- standard DDS: height first
 *   [16..19] dwWidth    <- then width
 *   [20..23] pitch/linear size
 *   [24..27] depth
 *   [28..31] mipmap count
 *   [32..75] reserved (11 dwords)
 *   [76..79] pfSize (always 32)
 *   [80..83] pfFlags
 *   [84..87] fourCC  <- DXT1/3/5 identifier
 *
 * Output pixel layout: row-major, (row * width + col) * 4.
 * Pixel (col, row) within a 4x4 block at block (bx, by):
 *   col = bx + i%4,  row = by + i/4,  i = 0..15 in natural scan order
 *
 * NOTE: ExtractorSharp DdsDecoder.cs swaps Width/Height field names and uses
 * column-major pos = height*(x+i/4)+y+i%4 which matches a GDI column-major
 * Bitmap. PyDnfEx uses PIL's standard DDS decoder (row-major), which is the
 * correct reference for our output format (PIL Image.frombytes row-major).
 */
#include "dxt.h"
#include <stdlib.h>
#include <string.h>

#define DDS_MAGIC      0x20534444u
#define FOURCC_DXT1    0x31545844u
#define FOURCC_DXT3    0x33545844u
#define FOURCC_DXT5    0x35545844u
#define DDS_MIPMAP_FLAG 0x00020000u

static uint16_t read_u16le(const uint8_t *p) {
    return (uint16_t)(p[0] | ((uint16_t)p[1] << 8));
}
static uint32_t read_u32le(const uint8_t *p) {
    return (uint32_t)p[0] | ((uint32_t)p[1]<<8) | ((uint32_t)p[2]<<16) | ((uint32_t)p[3]<<24);
}

/* Decode RGB565 to R,G,B components.
 * RGB565 bit layout: [15:11]=R(5), [10:5]=G(6), [4:0]=B(5)
 * Verified against PyDnfEx Color.from_1555 and ExtractorSharp DecodeRgb565.
 * NOTE: ES DecodeRgb565 reads low5=r, high5=b but outputs {r,g,b,a} for GDI
 * BGRA Bitmap (so visually R↔B swap cancels). We output true RGBA for PIL,
 * so we must correctly map: high5->R, low5->B. */
static void decode_rgb565(uint16_t color, uint8_t *r, uint8_t *g, uint8_t *b) {
    uint8_t rv = (uint8_t)((color >> 11) & 0x1f);  /* bits [15:11] = R */
    uint8_t gv = (uint8_t)((color >>  5) & 0x3f);  /* bits [10:5]  = G */
    uint8_t bv = (uint8_t)( color        & 0x1f);  /* bits [4:0]   = B */
    *r = (uint8_t)((rv << 3) | (rv >> 2));
    *g = (uint8_t)((gv << 2) | (gv >> 4));
    *b = (uint8_t)((bv << 3) | (bv >> 2));
}

/*
 * DXT1: 8 bytes per 4x4 block
 *   2 bytes color0 (RGB565)
 *   2 bytes color1 (RGB565)
 *   4 bytes color index (2 bits per pixel, row-major within block)
 *
 * Output: row-major RGBA, pos = (row * width + col) * 4
 *   row = by + i/4,  col = bx + i%4
 */
DnfError dxt_decode_dxt1(const uint8_t *data, int width, int height, uint8_t *out_rgba) {
    const uint8_t *p = data;
    int bx, by, i;
    for (by = 0; by < height; by += 4)
    for (bx = 0; bx < width; bx += 4) {
        uint16_t co0 = read_u16le(p); p += 2;
        uint16_t co1 = read_u16le(p); p += 2;

        uint8_t cr[4], cg[4], cb_[4], ca[4];
        decode_rgb565(co0, &cr[0], &cg[0], &cb_[0]); ca[0] = 0xff;
        decode_rgb565(co1, &cr[1], &cg[1], &cb_[1]); ca[1] = 0xff;

        if (co0 > co1) {
            cr[2] = (uint8_t)((cr[0]*2 + cr[1]) / 3);
            cg[2] = (uint8_t)((cg[0]*2 + cg[1]) / 3);
            cb_[2]= (uint8_t)((cb_[0]*2+ cb_[1]) / 3);
            ca[2] = 0xff;
            cr[3] = (uint8_t)((cr[0] + cr[1]*2) / 3);
            cg[3] = (uint8_t)((cg[0] + cg[1]*2) / 3);
            cb_[3]= (uint8_t)((cb_[0]+ cb_[1]*2) / 3);
            ca[3] = 0xff;
        } else {
            cr[2] = (uint8_t)((cr[0] + cr[1]) / 2);
            cg[2] = (uint8_t)((cg[0] + cg[1]) / 2);
            cb_[2]= (uint8_t)((cb_[0]+ cb_[1]) / 2);
            ca[2] = (uint8_t)((ca[0] + ca[1]) / 2);
            cr[3] = 0; cg[3] = 0; cb_[3] = 0; ca[3] = 0;
        }

        uint32_t idx = read_u32le(p); p += 4;
        for (i = 0; i < 16; i++, idx >>= 2) {
            int j   = idx & 0x3;
            int row = by + i/4;
            int col = bx + i%4;
            if (row >= height || col >= width) continue;
            int pos = (row * width + col) * 4;
            out_rgba[pos+0] = cr[j];
            out_rgba[pos+1] = cg[j];
            out_rgba[pos+2] = cb_[j];
            out_rgba[pos+3] = ca[j];
        }
    }
    return DNF_OK;
}

/*
 * DXT3: 16 bytes per 4x4 block
 * DNF block layout (matches ExtractorSharp DdsDecoder.cs):
 *   c0(u16), c1(u16), color_index(u32), alpha[0..3](u16 x4)
 * NOTE: DNF stores color data BEFORE alpha data within each block
 *       (reversed vs standard DXT3 which has alpha first).
 *
 * Alpha: each u16 holds 4×4-bit alpha values for one pixel row.
 *   pixel i gets alpha bits [(i%4)*4 .. (i%4)*4+3] of alphas[i/4].
 *   (standard bit-extraction, not the buggy ES &0xf approach)
 *
 * Output: row-major RGBA, pos = (row * width + col) * 4
 */
DnfError dxt_decode_dxt3(const uint8_t *data, int width, int height, uint8_t *out_rgba) {
    const uint8_t *p = data;
    int bx, by, i;
    for (by = 0; by < height; by += 4)
    for (bx = 0; bx < width; bx += 4) {
        /* DNF order: color first, then alpha */
        uint16_t c0  = read_u16le(p); p += 2;
        uint16_t c1  = read_u16le(p); p += 2;
        uint32_t idx = read_u32le(p); p += 4;

        uint16_t alphas[4];
        alphas[0] = read_u16le(p); p += 2;
        alphas[1] = read_u16le(p); p += 2;
        alphas[2] = read_u16le(p); p += 2;
        alphas[3] = read_u16le(p); p += 2;

        uint8_t cr[4], cg[4], cb_[4];
        decode_rgb565(c0, &cr[0], &cg[0], &cb_[0]);
        decode_rgb565(c1, &cr[1], &cg[1], &cb_[1]);
        cr[2] = (uint8_t)((cr[0]*2 + cr[1]) / 3);
        cg[2] = (uint8_t)((cg[0]*2 + cg[1]) / 3);
        cb_[2]= (uint8_t)((cb_[0]*2+ cb_[1]) / 3);
        cr[3] = (uint8_t)((cr[0] + cr[1]*2) / 3);
        cg[3] = (uint8_t)((cg[0] + cg[1]*2) / 3);
        cb_[3]= (uint8_t)((cb_[0]+ cb_[1]*2) / 3);

        uint32_t idxcopy = idx;
        for (i = 0; i < 16; i++, idxcopy >>= 2) {
            int j   = idxcopy & 0x3;
            int row = by + i/4;
            int col = bx + i%4;
            if (row >= height || col >= width) continue;
            int pos = (row * width + col) * 4;
            uint8_t a = (uint8_t)((alphas[i/4] >> ((i%4)*4)) & 0xf);
            out_rgba[pos+0] = cr[j];
            out_rgba[pos+1] = cg[j];
            out_rgba[pos+2] = cb_[j];
            out_rgba[pos+3] = (uint8_t)(a | (a << 4));
        }
    }
    return DNF_OK;
}

/*
 * DXT5: 16 bytes per 4x4 block
 *   alpha0(u8), alpha1(u8), alpha_indices(6 bytes = 48 bits, 3 bits/pixel)
 *   c0(u16), c1(u16), color_indices(u32, 2 bits/pixel)
 *
 * Output: row-major RGBA, pos = (row * width + col) * 4
 */
DnfError dxt_decode_dxt5(const uint8_t *data, int width, int height, uint8_t *out_rgba) {
    const uint8_t *p = data;
    int bx, by, i;
    for (by = 0; by < height; by += 4)
    for (bx = 0; bx < width; bx += 4) {
        uint8_t a0 = *p++;
        uint8_t a1 = *p++;

        uint8_t alphas[8];
        alphas[0] = a0; alphas[1] = a1;
        if (a0 > a1) {
            alphas[2] = (uint8_t)((6*a0 + 1*a1) / 7);
            alphas[3] = (uint8_t)((5*a0 + 2*a1) / 7);
            alphas[4] = (uint8_t)((4*a0 + 3*a1) / 7);
            alphas[5] = (uint8_t)((3*a0 + 4*a1) / 7);
            alphas[6] = (uint8_t)((2*a0 + 5*a1) / 7);
            alphas[7] = (uint8_t)((1*a0 + 6*a1) / 7);
        } else {
            alphas[2] = (uint8_t)((4*a0 + 1*a1) / 5);
            alphas[3] = (uint8_t)((3*a0 + 2*a1) / 5);
            alphas[4] = (uint8_t)((2*a0 + 3*a1) / 5);
            alphas[5] = (uint8_t)((1*a0 + 4*a1) / 5);
            alphas[6] = 0x00;
            alphas[7] = 0xff;
        }

        /* Read 6-byte alpha index table into uint64 (little-endian) */
        uint64_t abit = 0;
        for (i = 0; i < 6; i++) abit |= ((uint64_t)(*p++)) << (i * 8);

        uint16_t c0  = read_u16le(p); p += 2;
        uint16_t c1  = read_u16le(p); p += 2;
        uint32_t idx = read_u32le(p); p += 4;

        uint8_t cr[4], cg[4], cb_[4];
        decode_rgb565(c0, &cr[0], &cg[0], &cb_[0]);
        decode_rgb565(c1, &cr[1], &cg[1], &cb_[1]);
        cr[2] = (uint8_t)((cr[0]*2 + cr[1]) / 3);
        cg[2] = (uint8_t)((cg[0]*2 + cg[1]) / 3);
        cb_[2]= (uint8_t)((cb_[0]*2+ cb_[1]) / 3);
        cr[3] = (uint8_t)((cr[0] + cr[1]*2) / 3);
        cg[3] = (uint8_t)((cg[0] + cg[1]*2) / 3);
        cb_[3]= (uint8_t)((cb_[0]+ cb_[1]*2) / 3);

        uint64_t abitcopy = abit;
        uint32_t idxcopy  = idx;
        for (i = 0; i < 16; i++, idxcopy >>= 2, abitcopy >>= 3) {
            int j   = idxcopy & 0x3;
            int row = by + i/4;
            int col = bx + i%4;
            if (row >= height || col >= width) continue;
            int pos = (row * width + col) * 4;
            out_rgba[pos+0] = cr[j];
            out_rgba[pos+1] = cg[j];
            out_rgba[pos+2] = cb_[j];
            out_rgba[pos+3] = alphas[abitcopy & 0x7];
        }
    }
    return DNF_OK;
}

/*
 * dxt_decode_dds: parse DDS header, dispatch to DXT1/3/5 decoder.
 * Reads width/height per DDS standard: offset+12=dwHeight, offset+16=dwWidth.
 */
DnfError dxt_decode_dds(const uint8_t *dds_data, size_t dds_size,
                         uint8_t **out_rgba, int *out_w, int *out_h) {
    if (dds_size < 128) return DNF_ERR_INVALID_FORMAT;

    uint32_t magic = read_u32le(dds_data);
    if (magic != DDS_MAGIC) return DNF_ERR_INVALID_MAGIC;

    uint32_t header_len = read_u32le(dds_data + 4);
    /* Standard DDS: offset+12 = dwHeight, offset+16 = dwWidth */
    int height = (int)read_u32le(dds_data + 12);
    int width  = (int)read_u32le(dds_data + 16);
    uint32_t mip_count = read_u32le(dds_data + 28);
    if (width <= 0 || height <= 0) return DNF_ERR_INVALID_FORMAT;

    /* pfFlags at offset 80, fourCC at offset 84 */
    uint32_t pf_flags = read_u32le(dds_data + 80);
    uint32_t fourcc   = read_u32le(dds_data + 84);

    int count = 1;
    if (pf_flags & DDS_MIPMAP_FLAG) {
        count = (mip_count > 1) ? (int)mip_count : 1;
    }

    int block_bytes;
    int dxt_type;
    switch (fourcc) {
        case FOURCC_DXT1: block_bytes = 8;  dxt_type = 1; break;
        case FOURCC_DXT3: block_bytes = 16; dxt_type = 3; break;
        case FOURCC_DXT5: block_bytes = 16; dxt_type = 5; break;
        default: return DNF_ERR_UNSUPPORTED_VERSION;
    }

    size_t pixel_data_offset = (size_t)(header_len + 4);
    if (pixel_data_offset >= dds_size) return DNF_ERR_INVALID_FORMAT;

    size_t blocks_w = (size_t)((width + 3) / 4);
    size_t blocks_h = (size_t)((height + 3) / 4);
    size_t block_data_len = blocks_w * blocks_h * (size_t)block_bytes;
    if (pixel_data_offset + block_data_len > dds_size) return DNF_ERR_INVALID_FORMAT;

    uint8_t *rgba = (uint8_t *)malloc((size_t)(width * height * 4));
    if (!rgba) return DNF_ERR_OOM;

    DnfError err;
    const uint8_t *block_data = dds_data + pixel_data_offset;
    switch (dxt_type) {
        case 1: err = dxt_decode_dxt1(block_data, width, height, rgba); break;
        case 3: err = dxt_decode_dxt3(block_data, width, height, rgba); break;
        case 5: err = dxt_decode_dxt5(block_data, width, height, rgba); break;
        default: free(rgba); return DNF_ERR_INVALID_FORMAT;
    }
    if (err) { free(rgba); return err; }

    *out_rgba = rgba;
    *out_w    = width;
    *out_h    = height;
    (void)count; /* only decode first mip */
    return DNF_OK;
}
