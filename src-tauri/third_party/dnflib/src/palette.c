/*
 * palette.c - Color board (palette) implementation for IMG Ver4/Ver6
 *
 * File format: color_count(int32LE) + count × 4 bytes (R,G,B,A order).
 * Verified against PyDnfEx ColorBoard.open (reads '<4B' as-is and uses as RGBA)
 * and ExtractorSharp Colors.ReadPalette (data[0]=R, data[1]=G, data[2]=B, data[3]=A).
 */
#include "palette.h"
#include <string.h>
#include <stdlib.h>

DnfError palette_read(DnfIO *io, ColorBoard *cb) {
    int32_t count;
    DnfError e = dnf_io_read_i32(io, &count);
    if (e) return e;
    if (count < 0 || count > DNF_PALETTE_MAX) return DNF_ERR_INVALID_FORMAT;

    cb->count = count;
    int i;
    for (i = 0; i < count; i++) {
        uint8_t rgba[4];
        e = dnf_io_read(io, rgba, 4);
        if (e) return e;
        /* stored as R,G,B,A in file */
        cb->r[i] = rgba[0];
        cb->g[i] = rgba[1];
        cb->b[i] = rgba[2];
        cb->a[i] = rgba[3];
    }
    return DNF_OK;
}

DnfError palette_write(DnfIO *io, const ColorBoard *cb) {
    DnfError e = dnf_io_write_i32(io, cb->count);
    if (e) return e;
    int i;
    for (i = 0; i < cb->count; i++) {
        uint8_t rgba[4] = { cb->r[i], cb->g[i], cb->b[i], cb->a[i] };
        e = dnf_io_write(io, rgba, 4);
        if (e) return e;
    }
    return DNF_OK;
}

DnfError palette_decode(const ColorBoard *cb, const uint8_t *src, int src_len,
                         uint8_t *out_rgba) {
    int i;
    if (!cb || !src || !out_rgba || src_len < 0) return DNF_ERR_INVALID_FORMAT;
    if (cb->count <= 0) {
        memset(out_rgba, 0, (size_t)src_len * 4);
        return DNF_OK;
    }
    for (i = 0; i < src_len; i++) {
        int idx = src[i] % cb->count;  /* guard against bad indices */
        out_rgba[i*4+0] = cb->r[idx];
        out_rgba[i*4+1] = cb->g[idx];
        out_rgba[i*4+2] = cb->b[idx];
        out_rgba[i*4+3] = cb->a[idx];
    }
    return DNF_OK;
}

DnfError palette_encode(const uint8_t *rgba, int pixel_count,
                         uint8_t *out_indices, ColorBoard *cb) {
    cb->count = 0;
    int i;
    for (i = 0; i < pixel_count; i++) {
        uint8_t r = rgba[i*4+0], g = rgba[i*4+1];
        uint8_t b = rgba[i*4+2], a = rgba[i*4+3];

        /* Linear search for existing color */
        int found = -1, j;
        for (j = 0; j < cb->count; j++) {
            if (cb->r[j]==r && cb->g[j]==g && cb->b[j]==b && cb->a[j]==a) {
                found = j; break;
            }
        }
        if (found < 0) {
            if (cb->count >= DNF_PALETTE_MAX) {
                /* Palette overflow: map to nearest (use last entry) */
                found = cb->count - 1;
            } else {
                found = cb->count;
                cb->r[found] = r; cb->g[found] = g;
                cb->b[found] = b; cb->a[found] = a;
                cb->count++;
            }
        }
        out_indices[i] = (uint8_t)found;
    }
    return DNF_OK;
}
