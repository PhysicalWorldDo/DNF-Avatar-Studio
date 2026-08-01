/*
 * pixel.c - Batch pixel format conversion implementation
 */
#include "pixel.h"
#include <stdlib.h>
#include <string.h>

int pixel_size(DnfImageFormat fmt) {
    switch (fmt) {
        case DNF_FMT_ARGB_1555: return 2;
        case DNF_FMT_ARGB_4444: return 2;
        case DNF_FMT_ARGB_8888: return 4;
        default: return 0; /* LINK, DXT variants: not raw pixel */
    }
}

DnfError pixel_to_rgba(const uint8_t *raw, int w, int h, DnfImageFormat fmt,
                        uint8_t *out_rgba) {
    int n = w * h;
    int i;
    switch (fmt) {
        case DNF_FMT_ARGB_1555:
            for (i = 0; i < n; i++) {
                argb1555_to_rgba(raw[i*2], raw[i*2+1],
                                 &out_rgba[i*4+0], &out_rgba[i*4+1],
                                 &out_rgba[i*4+2], &out_rgba[i*4+3]);
            }
            return DNF_OK;
        case DNF_FMT_ARGB_4444:
            for (i = 0; i < n; i++) {
                argb4444_to_rgba(raw[i*2], raw[i*2+1],
                                 &out_rgba[i*4+0], &out_rgba[i*4+1],
                                 &out_rgba[i*4+2], &out_rgba[i*4+3]);
            }
            return DNF_OK;
        case DNF_FMT_ARGB_8888:
            for (i = 0; i < n; i++) {
                argb8888_to_rgba(raw[i*4], raw[i*4+1], raw[i*4+2], raw[i*4+3],
                                 &out_rgba[i*4+0], &out_rgba[i*4+1],
                                 &out_rgba[i*4+2], &out_rgba[i*4+3]);
            }
            return DNF_OK;
        default:
            return DNF_ERR_INVALID_FORMAT;
    }
}

DnfError pixel_from_rgba(const uint8_t *rgba, int w, int h, DnfImageFormat fmt,
                          uint8_t *out_raw) {
    int n = w * h;
    int i;
    switch (fmt) {
        case DNF_FMT_ARGB_1555:
            for (i = 0; i < n; i++) {
                rgba_to_argb1555(rgba[i*4+0], rgba[i*4+1], rgba[i*4+2], rgba[i*4+3],
                                 &out_raw[i*2], &out_raw[i*2+1]);
            }
            return DNF_OK;
        case DNF_FMT_ARGB_4444:
            for (i = 0; i < n; i++) {
                rgba_to_argb4444(rgba[i*4+0], rgba[i*4+1], rgba[i*4+2], rgba[i*4+3],
                                 &out_raw[i*2], &out_raw[i*2+1]);
            }
            return DNF_OK;
        case DNF_FMT_ARGB_8888:
            for (i = 0; i < n; i++) {
                rgba_to_argb8888(rgba[i*4+0], rgba[i*4+1], rgba[i*4+2], rgba[i*4+3],
                                 &out_raw[i*4], &out_raw[i*4+1],
                                 &out_raw[i*4+2], &out_raw[i*4+3]);
            }
            return DNF_OK;
        default:
            return DNF_ERR_INVALID_FORMAT;
    }
}
