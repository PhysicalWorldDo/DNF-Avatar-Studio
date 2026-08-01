/*
 * dxt.h - DXT1/3/5 (S3TC) decoder
 * Input: raw DDS file bytes (including 128-byte DDS header).
 * Output: RGBA8888 pixel buffer (w * h * 4 bytes), heap-allocated.
 */
#ifndef DNF_DXT_H
#define DNF_DXT_H

#include <stdint.h>
#include <stddef.h>
#include "dnflib.h"

/* Decode a full DDS file (with header) to RGBA8888.
 * Allocates *out_rgba (w*h*4 bytes); caller frees with dnf_free().
 * out_w / out_h receive the image dimensions. */
DnfError dxt_decode_dds(const uint8_t *dds_data, size_t dds_size,
                         uint8_t **out_rgba, int *out_w, int *out_h);

/* Low-level: decode raw DXT1 block data (no DDS header) to RGBA8888.
 * data must be width/height/16 * 8 bytes. */
DnfError dxt_decode_dxt1(const uint8_t *data, int width, int height, uint8_t *out_rgba);

/* Low-level: decode raw DXT3 block data (no DDS header) to RGBA8888.
 * data must be width/height/16 * 16 bytes. */
DnfError dxt_decode_dxt3(const uint8_t *data, int width, int height, uint8_t *out_rgba);

/* Low-level: decode raw DXT5 block data (no DDS header) to RGBA8888.
 * data must be width/height/16 * 16 bytes. */
DnfError dxt_decode_dxt5(const uint8_t *data, int width, int height, uint8_t *out_rgba);

#endif /* DNF_DXT_H */
