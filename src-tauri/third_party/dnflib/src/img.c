/*
 * img.c - IMG file parser for all versions (Ver1/2/4/5/6)
 *
 * Format reference:
 *   New magic: "Neople Img File\0" (16B) + images_size(i32) + keep(i32) + version(i32) + count(i32)
 *   Old magic: "Neople Image File\0" (18B) + unknown(i16) + keep(i32) + version(i32) + count(i32)
 *
 * Frame header (non-LINK):  format(i32) + compress(i32) + w(i32) + h(i32) + size(i32)
 *                            + x(i32) + y(i32) + mw(i32) + mh(i32)   = 36 bytes
 * LINK frame:               format(i32) + link_index(i32)              = 8 bytes
 * SpriteZlib extra (Ver5):  keep(i32)+sprite_index(i32)+l(i32)+t(i32)+r(i32)+b(i32)+rotate(i32) = 28B
 *
 * Ver2 layout: header section + image data section (separated by images_size offset)
 * Ver1 layout: frame header immediately followed by frame data (interleaved)
 * Ver4: Ver2 + single ColorBoard before frame headers
 * Ver5: Ver4 + sprite_count(i32)+file_size(i32) before color board, then N Sprite(28B) entries
 * Ver6: Ver2 + color_board_count(i32) + N ColorBoards before frame headers
 */
#include "img.h"
#include <stdlib.h>
#include <string.h>

/* miniz: declarations only; implementation compiled separately via miniz.c */
#include "miniz.h"
/* miniz.h defines `compress` / `uncompress` / `compressBound` macros for zlib
 * compatibility. Undefine them to prevent clashes with our struct fields and
 * local variables also named `compress`. */
#ifdef compress
#  undef compress
#endif
#ifdef uncompress
#  undef uncompress
#endif
#ifdef compressBound
#  undef compressBound
#endif

/* ------------------------------------------------------------------ */
/* zlib helpers (PyDnfEx-compatible)                                   */
/* ------------------------------------------------------------------ */

DnfError dnf_zlib_decompress(const uint8_t *src, size_t src_len,
                               uint8_t **out, size_t *out_len) {
    /* Heuristic: try to find the zlib stream start (byte 0x78).
     * Port of PyDnfEx common.py zlib_decompress(). */
    if (src_len == 0) return DNF_ERR_DECOMPRESS;

    /* Initial output buffer: 16x input + 1MB headroom, minimum 4MB */
    mz_ulong dest_cap = (mz_ulong)(src_len * 16 + 1024 * 1024);
    if (dest_cap < 4 * 1024 * 1024) dest_cap = 4 * 1024 * 1024;

    if (src[0] == 0x78) {
        /* Standard zlib stream */
        uint8_t *dest = (uint8_t *)malloc(dest_cap);
        if (!dest) return DNF_ERR_OOM;
        mz_ulong dest_len = dest_cap;
        int r = mz_uncompress(dest, &dest_len, src, (mz_ulong)src_len);
        if (r != MZ_OK) { free(dest); return DNF_ERR_DECOMPRESS; }
        *out = dest;
        *out_len = (size_t)dest_len;
        return DNF_OK;
    }

    /* Search backwards for last 0x78 byte */
    int found = -1;
    int i;
    for (i = (int)src_len - 1; i >= 0; i--) {
        if (src[i] == 0x78) { found = i; break; }
    }
    if (found < 0) return DNF_ERR_DECOMPRESS;

    /* Try: data[found:] + data (PyDnfEx first attempt) */
    size_t tail_len  = src_len - (size_t)found;
    size_t concat_len = tail_len + src_len;
    uint8_t *concat = (uint8_t *)malloc(concat_len);
    if (!concat) return DNF_ERR_OOM;
    memcpy(concat, src + found, tail_len);
    memcpy(concat + tail_len, src, src_len);

    uint8_t *dest = (uint8_t *)malloc(dest_cap);
    if (!dest) { free(concat); return DNF_ERR_OOM; }

    mz_ulong dest_len = dest_cap;
    int r = mz_uncompress(dest, &dest_len, concat, (mz_ulong)concat_len);
    if (r != MZ_OK) {
        /* Fallback: data[found:found+2] + data */
        if ((size_t)found + 2 <= src_len) {
            memcpy(concat, src + found, 2);
            memcpy(concat + 2, src, src_len);
            dest_len = dest_cap;
            r = mz_uncompress(dest, &dest_len, concat, (mz_ulong)(2 + src_len));
        }
    }
    free(concat);
    if (r != MZ_OK) { free(dest); return DNF_ERR_DECOMPRESS; }
    *out = dest;
    *out_len = (size_t)dest_len;
    return DNF_OK;
}

DnfError dnf_zlib_compress(const uint8_t *src, size_t src_len,
                             uint8_t **out, size_t *out_len) {
    mz_ulong dest_len = (mz_ulong)(src_len * 1.001 + 1024 + 12);
    uint8_t *dest = (uint8_t *)malloc(dest_len);
    if (!dest) return DNF_ERR_OOM;
    int r = mz_compress(dest, &dest_len, src, (mz_ulong)src_len);
    if (r != MZ_OK) { free(dest); return DNF_ERR_DECOMPRESS; }
    *out = dest;
    *out_len = (size_t)dest_len;
    return DNF_OK;
}

/* ------------------------------------------------------------------ */
/* ImgPriv helpers                                                     */
/* ------------------------------------------------------------------ */

static ImgPriv *priv_alloc(void) {
    ImgPriv *p = (ImgPriv *)calloc(1, sizeof(ImgPriv));
    return p;
}

void img_priv_free(ImgPriv *priv, int frame_count) {
    if (!priv) return;
    if (priv->palettes) free(priv->palettes);
    if (priv->sprites) {
        int i;
        for (i = 0; i < priv->sprite_count; i++) {
            if (priv->sprites[i].zip_data) free(priv->sprites[i].zip_data);
            if (priv->sprites[i].raw_data) free(priv->sprites[i].raw_data);
        }
        free(priv->sprites);
    }
    if (priv->sprite_exts) free(priv->sprite_exts);
    free(priv);
    (void)frame_count;
}

/* ------------------------------------------------------------------ */
/* Read a single frame header from IO                                  */
/* frames array is already allocated to [count].                      */
/* For Ver1: data immediately follows header (interleaved).           */
/* For Ver2+: data is in a separate section; set data=NULL, remember offset. */
/* ------------------------------------------------------------------ */

/* Read common frame header fields (after format has been read).
 * Returns the declared data size in *data_size_out. */
static DnfError read_frame_body(DnfIO *io, ImageFrame *f, int32_t *data_size_out) {
    DnfError e;
    int32_t compress, w, h, sz, x, y, mw, mh;
    if ((e = dnf_io_read_i32(io, &compress))) return e;
    if ((e = dnf_io_read_i32(io, &w)))        return e;
    if ((e = dnf_io_read_i32(io, &h)))        return e;
    if ((e = dnf_io_read_i32(io, &sz)))       return e;
    if ((e = dnf_io_read_i32(io, &x)))        return e;
    if ((e = dnf_io_read_i32(io, &y)))        return e;
    if ((e = dnf_io_read_i32(io, &mw)))       return e;
    if ((e = dnf_io_read_i32(io, &mh)))       return e;
    f->compress   = (DnfCompressMode)compress;
    f->w = w; f->h = h; f->x = x; f->y = y; f->mw = mw; f->mh = mh;
    *data_size_out = sz;
    return DNF_OK;
}

/* Fix-up size for NONE-compress frames (Ver1 behaviour) */
static int32_t frame_real_size(const ImageFrame *f) {
    int ps = 0;
    switch (f->format) {
        case DNF_FMT_ARGB_1555: case DNF_FMT_ARGB_4444: ps = 2; break;
        case DNF_FMT_ARGB_8888: ps = 4; break;
        default: ps = 2; break;
    }
    return f->w * f->h * ps;
}

/* ------------------------------------------------------------------ */
/* Ver1 read                                                           */
/* Data immediately follows each frame header.                        */
/* ------------------------------------------------------------------ */
static DnfError img_read_v1(DnfIO *io, ImgFile *img, ImgPriv *priv) {
    int i;
    (void)priv;
    for (i = 0; i < img->frame_count; i++) {
        ImageFrame *f = &img->frames[i];
        int32_t fmt;
        DnfError e;
        if ((e = dnf_io_read_i32(io, &fmt))) return e;
        f->format = (DnfImageFormat)fmt;

        if (fmt == DNF_FMT_LINK) {
            if ((e = dnf_io_read_i32(io, &f->link_index))) return e;
            f->data = NULL; f->data_size = 0;
            continue;
        }

        int32_t sz;
        if ((e = read_frame_body(io, f, &sz))) return e;

        /* Ver1: NONE compress -> real size, otherwise use declared */
        if (f->compress == DNF_COMPRESS_NONE) {
            sz = frame_real_size(f);
        }
        f->data_size = sz;

        /* Data immediately follows header */
        if (sz > 0) {
            f->data = (uint8_t *)malloc((size_t)sz);
            if (!f->data) return DNF_ERR_OOM;
            if ((e = dnf_io_read(io, f->data, (size_t)sz))) return e;
        } else {
            f->data = NULL;
        }
    }
    return DNF_OK;
}

/* ------------------------------------------------------------------ */
/* Ver2 read (and base for Ver4/5/6)                                  */
/* Header section first, then data section.                           */
/* images_size = size of header section (bytes after keep/ver/count). */
/* ------------------------------------------------------------------ */
static DnfError img_read_v2(DnfIO *io, ImgFile *img, ImgPriv *priv) {
    int i;
    long data_section_start;

    if (priv->images_size > 0) {
        /* New-magic format: data section starts at fixed offset */
        /* images_size is the byte count of the header region;
         * we note the current position (after keep/ver/count) and
         * compute the data start. */
        long header_start = dnf_io_tell(io);
        data_section_start = header_start + (long)priv->images_size;
    } else {
        data_section_start = 0; /* will be computed after headers */
    }

    /* Remember offsets (frame sizes) so we can seek to data section */
    int32_t *declared_sizes = (int32_t *)calloc((size_t)img->frame_count, sizeof(int32_t));
    if (!declared_sizes) return DNF_ERR_OOM;

    DnfError e = DNF_OK;
    for (i = 0; i < img->frame_count; i++) {
        ImageFrame *f = &img->frames[i];
        int32_t fmt;
        if ((e = dnf_io_read_i32(io, &fmt))) goto fail;
        f->format = (DnfImageFormat)fmt;

        if (fmt == DNF_FMT_LINK) {
            if ((e = dnf_io_read_i32(io, &f->link_index))) goto fail;
            declared_sizes[i] = 0;
            continue;
        }

        int32_t sz;
        if ((e = read_frame_body(io, f, &sz))) goto fail;
        declared_sizes[i] = sz;

        /* Ver5: SpriteZlib frames have 28 extra bytes */
        if (f->compress == DNF_COMPRESS_DDS_ZLIB && priv->sprite_exts) {
            SpriteFrameExt *ext = &priv->sprite_exts[i];
            if ((e = dnf_io_read_i32(io, &ext->keep)))         goto fail;
            if ((e = dnf_io_read_i32(io, &ext->sprite_index))) goto fail;
            if ((e = dnf_io_read_i32(io, &ext->left)))         goto fail;
            if ((e = dnf_io_read_i32(io, &ext->top)))          goto fail;
            if ((e = dnf_io_read_i32(io, &ext->right)))        goto fail;
            if ((e = dnf_io_read_i32(io, &ext->bottom)))       goto fail;
            if ((e = dnf_io_read_i32(io, &ext->rotate)))       goto fail;
        }
    }

    if (priv->images_size <= 0) {
        data_section_start = dnf_io_tell(io);
    }

    /* Seek to data section and read each frame's data */
    {
        long data_pos = data_section_start;
        /* For Ver5: sprite data comes first */
        if (priv->sprite_count > 0) {
            for (i = 0; i < priv->sprite_count; i++) {
                ImgSprite *sp = &priv->sprites[i];
                if (sp->data_size > 0) {
                    sp->zip_data = (uint8_t *)malloc((size_t)sp->data_size);
                    if (!sp->zip_data) { e = DNF_ERR_OOM; goto fail; }
                    if ((e = dnf_io_seek(io, data_pos, SEEK_SET))) goto fail;
                    if ((e = dnf_io_read(io, sp->zip_data, (size_t)sp->data_size))) goto fail;
                }
                data_pos += sp->data_size;
            }
        }

        for (i = 0; i < img->frame_count; i++) {
            ImageFrame *f = &img->frames[i];
            if (f->format == DNF_FMT_LINK) continue;
            if (f->compress == DNF_COMPRESS_DDS_ZLIB) continue; /* sprite ref, no data */

            int32_t sz = declared_sizes[i];
            if (f->compress == DNF_COMPRESS_NONE) {
                sz = frame_real_size(f);
            }
            f->data_size = sz;

            if (sz > 0) {
                if ((e = dnf_io_seek(io, data_pos, SEEK_SET))) goto fail;
                f->data = (uint8_t *)malloc((size_t)sz);
                if (!f->data) { e = DNF_ERR_OOM; goto fail; }
                if ((e = dnf_io_read(io, f->data, (size_t)sz))) goto fail;
                data_pos += sz;
            }
        }
    }

fail:
    free(declared_sizes);
    return e;
}

/* ------------------------------------------------------------------ */
/* Ver4 read: ColorBoard + Ver2 frames                                */
/* ------------------------------------------------------------------ */
static DnfError img_read_v4(DnfIO *io, ImgFile *img, ImgPriv *priv) {
    priv->palettes = (ColorBoard *)malloc(sizeof(ColorBoard));
    if (!priv->palettes) return DNF_ERR_OOM;
    priv->palette_count = 1;
    DnfError e = palette_read(io, &priv->palettes[0]);
    if (e) return e;
    return img_read_v2(io, img, priv);
}

/* ------------------------------------------------------------------ */
/* Ver5 read: sprite_count + file_size + ColorBoard + Sprites + Ver2  */
/* ------------------------------------------------------------------ */
static DnfError img_read_v5(DnfIO *io, ImgFile *img, ImgPriv *priv) {
    DnfError e;
    int32_t sprite_count, file_size;
    if ((e = dnf_io_read_i32(io, &sprite_count))) return e;
    if ((e = dnf_io_read_i32(io, &file_size)))    return e;
    priv->file_size_v5 = file_size;

    /* ColorBoard (same as Ver4) */
    priv->palettes = (ColorBoard *)malloc(sizeof(ColorBoard));
    if (!priv->palettes) return DNF_ERR_OOM;
    priv->palette_count = 1;
    if ((e = palette_read(io, &priv->palettes[0]))) return e;

    /* Sprite descriptors */
    if (sprite_count > 0) {
        priv->sprites = (ImgSprite *)calloc((size_t)sprite_count, sizeof(ImgSprite));
        if (!priv->sprites) return DNF_ERR_OOM;
        priv->sprite_count = sprite_count;
        int i;
        for (i = 0; i < sprite_count; i++) {
            ImgSprite *sp = &priv->sprites[i];
            if ((e = dnf_io_read_i32(io, &sp->keep)))      return e;
            if ((e = dnf_io_read_i32(io, &sp->format)))    return e;
            if ((e = dnf_io_read_i32(io, &sp->index)))     return e;
            if ((e = dnf_io_read_i32(io, &sp->data_size))) return e;
            if ((e = dnf_io_read_i32(io, &sp->raw_size)))  return e;
            if ((e = dnf_io_read_i32(io, &sp->w)))         return e;
            if ((e = dnf_io_read_i32(io, &sp->h)))         return e;
        }
    }

    /* Allocate sprite_exts for frames */
    priv->sprite_exts = (SpriteFrameExt *)calloc((size_t)img->frame_count, sizeof(SpriteFrameExt));
    if (!priv->sprite_exts) return DNF_ERR_OOM;

    return img_read_v2(io, img, priv);
}

/* ------------------------------------------------------------------ */
/* Ver6 read: color_board_count + N ColorBoards + Ver2 frames         */
/* ------------------------------------------------------------------ */
static DnfError img_read_v6(DnfIO *io, ImgFile *img, ImgPriv *priv) {
    DnfError e;
    int32_t cb_count;
    if ((e = dnf_io_read_i32(io, &cb_count))) return e;
    if (cb_count < 0) return DNF_ERR_INVALID_FORMAT;

    priv->palettes = (ColorBoard *)calloc((size_t)cb_count, sizeof(ColorBoard));
    if (cb_count > 0 && !priv->palettes) return DNF_ERR_OOM;
    priv->palette_count = cb_count;
    int i;
    for (i = 0; i < cb_count; i++) {
        if ((e = palette_read(io, &priv->palettes[i]))) return e;
    }
    return img_read_v2(io, img, priv);
}

static ImageFrame *resolve_link_frame_target(ImgFile *img, int frame_index) {
    int guard = 0;
    ImageFrame *target;

    if (!img || frame_index < 0 || frame_index >= img->frame_count) return NULL;
    target = &img->frames[frame_index];
    while (target->format == DNF_FMT_LINK) {
        int next = target->link_index;
        if (next < 0 || next >= img->frame_count) return NULL;
        if (++guard > img->frame_count) return NULL;
        target = &img->frames[next];
    }
    return target;
}

static void resolve_link_frame_metadata(ImgFile *img) {
    int i;
    if (!img || !img->frames) return;

    for (i = 0; i < img->frame_count; i++) {
        ImageFrame *f = &img->frames[i];
        ImageFrame *target;
        if (f->format != DNF_FMT_LINK) continue;
        target = resolve_link_frame_target(img, f->link_index);
        if (!target) continue;
        f->compress = target->compress;
        f->w = target->w;
        f->h = target->h;
        f->x = target->x;
        f->y = target->y;
        f->mw = target->mw;
        f->mh = target->mh;
        f->data_size = target->data_size;
    }
}

/* ------------------------------------------------------------------ */
/* Version dispatch table                                             */
/* ------------------------------------------------------------------ */
typedef DnfError (*VersionReadFn)(DnfIO *io, ImgFile *img, ImgPriv *priv);

typedef struct { int version; VersionReadFn read; } VersionEntry;

static const VersionEntry VERSION_TABLE[] = {
    { IMG_VER1, img_read_v1 },
    { IMG_VER2, img_read_v2 },
    { IMG_VER4, img_read_v4 },
    { IMG_VER5, img_read_v5 },
    { IMG_VER6, img_read_v6 },
};
#define VERSION_TABLE_SIZE ((int)(sizeof(VERSION_TABLE)/sizeof(VERSION_TABLE[0])))

/* ------------------------------------------------------------------ */
/* img_open                                                            */
/* ------------------------------------------------------------------ */
DnfError img_open(DnfIO *io, ImgFile **out) {
    /* Read magic: try 18-byte read first to check both old and new magic */
    char magic_buf[19] = {0};
    DnfError e = dnf_io_read(io, magic_buf, 18);
    if (e) return e;

    int is_new = 0, is_old = 0;
    if (memcmp(magic_buf, IMG_MAGIC_NEW, 15) == 0) is_new = 1;
    else if (memcmp(magic_buf, IMG_MAGIC_OLD, 17) == 0) is_old = 1;
    else return DNF_ERR_INVALID_MAGIC;

    int32_t images_size = 0;
    if (is_new) {
        /* New magic is 16 bytes (15+NUL), then images_size(i32).
         * We read 18 bytes; the extra 2 bytes are part of images_size. */
        /* Seek back 2, re-read images_size properly */
        if ((e = dnf_io_seek(io, -2, SEEK_CUR))) return e;
        if ((e = dnf_io_read_i32(io, &images_size))) return e;
    } else {
        /* Old magic: 18 bytes consumed (17+NUL), then 2-byte unknown (already read as part of 18B)
         * Actually: "Neople Image File\0" is 18 bytes, then int16 unknown.
         * We read 18 bytes which exactly covers the magic+NUL. Next read = i16 unknown. */
        int16_t unknown;
        if ((e = dnf_io_read_i16(io, &unknown))) return e;
        images_size = 0;
    }

    /* Common header: keep(i32) + version(i32) + count(i32) */
    int32_t keep, version, count;
    if ((e = dnf_io_read_i32(io, &keep)))    return e;
    if ((e = dnf_io_read_i32(io, &version))) return e;
    if ((e = dnf_io_read_i32(io, &count)))   return e;

    if (count < 0 || count > 65535) return DNF_ERR_INVALID_FORMAT;

    /* Find version handler */
    VersionReadFn read_fn = NULL;
    int vi;
    for (vi = 0; vi < VERSION_TABLE_SIZE; vi++) {
        if (VERSION_TABLE[vi].version == version) {
            read_fn = VERSION_TABLE[vi].read;
            break;
        }
    }
    if (!read_fn) return DNF_ERR_UNSUPPORTED_VERSION;

    /* Allocate ImgFile */
    ImgFile *img = (ImgFile *)calloc(1, sizeof(ImgFile));
    if (!img) return DNF_ERR_OOM;

    img->version     = version;
    img->frame_count = count;
    if (count > 0) {
        img->frames = (ImageFrame *)calloc((size_t)count, sizeof(ImageFrame));
        if (!img->frames) { free(img); return DNF_ERR_OOM; }
    }

    ImgPriv *priv = priv_alloc();
    if (!priv) { free(img->frames); free(img); return DNF_ERR_OOM; }
    priv->io          = *io;
    priv->keep        = keep;
    priv->images_size = images_size;
    img->_priv        = priv;

    e = read_fn(io, img, priv);
    if (e) {
        /* Free frame data on error */
        int i;
        for (i = 0; i < count; i++) if (img->frames[i].data) free(img->frames[i].data);
        free(img->frames);
        img_priv_free(priv, count);
        free(img);
        return e;
    }
    resolve_link_frame_metadata(img);

    *out = img;
    return DNF_OK;
}

/* ------------------------------------------------------------------ */
/* img_save (Ver2 layout)                                             */
/* Only Ver2 write is implemented; other versions save as Ver2.       */
/* ------------------------------------------------------------------ */
DnfError img_save(DnfIO *io, ImgFile *img) {
    ImgPriv *priv = (ImgPriv *)img->_priv;
    int i;
    DnfError e;

    /* Write new magic + images_size placeholder */
    if ((e = dnf_io_write_ascii(io, IMG_MAGIC_NEW))) return e;

    /* Compute images_size (header section size) */
    int32_t header_sz = 0;
    for (i = 0; i < img->frame_count; i++) {
        ImageFrame *f = &img->frames[i];
        header_sz += 4; /* format */
        if (f->format == DNF_FMT_LINK) header_sz += 4;
        else header_sz += 32; /* compress+w+h+size+x+y+mw+mh = 8 fields * 4 */
    }
    if ((e = dnf_io_write_i32(io, header_sz))) return e;
    if ((e = dnf_io_write_i32(io, priv ? priv->keep : 0))) return e;
    if ((e = dnf_io_write_i32(io, img->version))) return e;
    if ((e = dnf_io_write_i32(io, img->frame_count))) return e;

    /* Write frame headers */
    for (i = 0; i < img->frame_count; i++) {
        ImageFrame *f = &img->frames[i];
        if ((e = dnf_io_write_i32(io, (int32_t)f->format))) return e;
        if (f->format == DNF_FMT_LINK) {
            if ((e = dnf_io_write_i32(io, f->link_index))) return e;
            continue;
        }
        if ((e = dnf_io_write_i32(io, (int32_t)f->compress))) return e;
        if ((e = dnf_io_write_i32(io, f->w)))          return e;
        if ((e = dnf_io_write_i32(io, f->h)))          return e;
        if ((e = dnf_io_write_i32(io, f->data_size)))  return e;
        if ((e = dnf_io_write_i32(io, f->x)))          return e;
        if ((e = dnf_io_write_i32(io, f->y)))          return e;
        if ((e = dnf_io_write_i32(io, f->mw)))         return e;
        if ((e = dnf_io_write_i32(io, f->mh)))         return e;
    }

    /* Write frame data */
    for (i = 0; i < img->frame_count; i++) {
        ImageFrame *f = &img->frames[i];
        if (f->format == DNF_FMT_LINK) continue;
        if (f->data && f->data_size > 0) {
            if ((e = dnf_io_write(io, f->data, (size_t)f->data_size))) return e;
        }
    }
    return DNF_OK;
}
