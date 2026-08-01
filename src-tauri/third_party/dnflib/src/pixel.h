/*
 * pixel.h - Pixel format conversion (ARGB1555/4444/8888 <-> RGBA8888)
 * All conversions output RGBA8888 (R,G,B,A byte order).
 * Input pixel formats store data in little-endian byte order.
 *
 * Bit layouts (verified against PyDnfEx color.py and ExtractorSharp Colors.cs):
 *   ARGB1555: [byte1: gggbbbbb] [byte2: arrrrrgg]  -> a:1 r:5 g:5 b:5
 *   ARGB4444: [byte1: ggggbbbb] [byte2: aaaarrrr]  -> a:4 r:4 g:4 b:4
 *   ARGB8888: [byte0:B][byte1:G][byte2:R][byte3:A]  (BGRA storage -> RGBA output)
 */
#ifndef DNF_PIXEL_H
#define DNF_PIXEL_H

#include <stdint.h>
#include <stddef.h>
#include "dnflib.h"

/* ---- Single pixel inline converters (input: raw bytes, output: R,G,B,A) ---- */

static inline void argb1555_to_rgba(uint8_t v0, uint8_t v1,
                                    uint8_t *r, uint8_t *g, uint8_t *b, uint8_t *a) {
    uint8_t bv = v0 & 0x1f;
    uint8_t gv = (v0 >> 5) | ((v1 & 0x03) << 3);
    uint8_t rv = (v1 >> 2) & 0x1f;
    uint8_t av = v1 >> 7;
    /* 5-bit -> 8-bit: x = (x<<3)|(x>>2) */
    *b = (uint8_t)((bv << 3) | (bv >> 2));
    *g = (uint8_t)((gv << 3) | (gv >> 2));
    *r = (uint8_t)((rv << 3) | (rv >> 2));
    *a = (uint8_t)(av * 0xff);
}

static inline void argb4444_to_rgba(uint8_t v0, uint8_t v1,
                                    uint8_t *r, uint8_t *g, uint8_t *b, uint8_t *a) {
    *b = (uint8_t)((v0 & 0x0f) << 4);
    *g = (uint8_t)(v0 & 0xf0);
    *r = (uint8_t)((v1 & 0x0f) << 4);
    *a = (uint8_t)(v1 & 0xf0);
}

static inline void argb8888_to_rgba(uint8_t v0, uint8_t v1, uint8_t v2, uint8_t v3,
                                    uint8_t *r, uint8_t *g, uint8_t *b, uint8_t *a) {
    /* stored as BGRA -> output RGBA */
    *b = v0; *g = v1; *r = v2; *a = v3;
}

/* ---- Reverse converters (RGBA -> raw bytes) ---- */

static inline void rgba_to_argb1555(uint8_t r, uint8_t g, uint8_t b, uint8_t a,
                                    uint8_t *v0, uint8_t *v1) {
    uint8_t bv = b >> 3, gv = g >> 3, rv = r >> 3, av = a >> 7;
    *v0 = (uint8_t)(((gv & 0x07) << 5) | bv);
    *v1 = (uint8_t)((av << 7) | (rv << 2) | (gv >> 3));
}

static inline void rgba_to_argb4444(uint8_t r, uint8_t g, uint8_t b, uint8_t a,
                                    uint8_t *v0, uint8_t *v1) {
    *v0 = (uint8_t)(g | (b >> 4));
    *v1 = (uint8_t)(a | (r >> 4));
}

static inline void rgba_to_argb8888(uint8_t r, uint8_t g, uint8_t b, uint8_t a,
                                    uint8_t *v0, uint8_t *v1, uint8_t *v2, uint8_t *v3) {
    *v0 = b; *v1 = g; *v2 = r; *v3 = a;
}

/* ---- Batch converters ---- */

/* Convert raw_data (format pixel bytes) to RGBA8888 into out_rgba.
 * out_rgba must be pre-allocated: w * h * 4 bytes. */
DnfError pixel_to_rgba(const uint8_t *raw, int w, int h, DnfImageFormat fmt,
                        uint8_t *out_rgba);

/* Convert RGBA8888 pixels to format raw bytes into out_raw.
 * out_raw must be pre-allocated: w * h * pixel_size(fmt) bytes. */
DnfError pixel_from_rgba(const uint8_t *rgba, int w, int h, DnfImageFormat fmt,
                          uint8_t *out_raw);

/* Returns bytes-per-pixel for raw formats (0 for DXT/LINK) */
int pixel_size(DnfImageFormat fmt);

#endif /* DNF_PIXEL_H */
