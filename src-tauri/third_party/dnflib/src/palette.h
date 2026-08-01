/*
 * palette.h - Color board (palette) read/write for IMG Ver4/Ver6
 *
 * File format: color_count(int32LE) + count × 4 bytes (R,G,B,A order).
 * Verified: PyDnfEx reads '<4B' as-is and uses directly as RGBA;
 *           ExtractorSharp Colors.ReadPalette: data[0]=R, data[1]=G, data[2]=B, data[3]=A.
 * Lookup: each pixel index byte maps to an R,G,B,A entry -> output RGBA.
 */
#ifndef DNF_PALETTE_H
#define DNF_PALETTE_H

#include <stdint.h>
#include "dnflib.h"
#include "io_helper.h"

/* Up to 256 colors per palette */
#define DNF_PALETTE_MAX 256

typedef struct {
    uint8_t  r[DNF_PALETTE_MAX];
    uint8_t  g[DNF_PALETTE_MAX];
    uint8_t  b[DNF_PALETTE_MAX];
    uint8_t  a[DNF_PALETTE_MAX];
    int      count;
} ColorBoard;

/* Read a ColorBoard from IO: color_count(i32) + count*4B (BGRA) */
DnfError palette_read(DnfIO *io, ColorBoard *cb);

/* Write a ColorBoard to IO */
DnfError palette_write(DnfIO *io, const ColorBoard *cb);

/* Convert index-encoded pixel data to RGBA8888.
 * src: one byte per pixel (palette index), src_len = w*h.
 * out_rgba: pre-allocated w*h*4 bytes. */
DnfError palette_decode(const ColorBoard *cb, const uint8_t *src, int src_len,
                         uint8_t *out_rgba);

/* Convert RGBA8888 pixels to palette index bytes.
 * Builds palette on-the-fly (up to 256 unique colors).
 * out_indices: pre-allocated w*h bytes.
 * cb: output palette (populated by this function). */
DnfError palette_encode(const uint8_t *rgba, int pixel_count,
                         uint8_t *out_indices, ColorBoard *cb);

#endif /* DNF_PALETTE_H */
