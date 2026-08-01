/*
 * img.h - Internal IMG module structures and version dispatch table
 */
#ifndef DNF_IMG_H
#define DNF_IMG_H

#include "dnflib.h"
#include "io_helper.h"
#include "palette.h"

/* ------------------------------------------------------------------ */
/* IMG version constants                                               */
/* ------------------------------------------------------------------ */
#define IMG_VER1 1
#define IMG_VER2 2
#define IMG_VER4 4
#define IMG_VER5 5
#define IMG_VER6 6

#define IMG_MAGIC_NEW "Neople Img File"    /* 15 chars + NUL = 16 bytes */
#define IMG_MAGIC_OLD "Neople Image File"  /* 17 chars + NUL = 18 bytes */

/* ------------------------------------------------------------------ */
/* Sprite (Ver5 texture atlas entry)                                   */
/* ------------------------------------------------------------------ */
typedef struct {
    int32_t  keep;
    int32_t  format;     /* DnfImageFormat */
    int32_t  index;
    int32_t  data_size;  /* compressed size */
    int32_t  raw_size;   /* uncompressed size (from file header) */
    int32_t  w, h;
    uint8_t *zip_data;   /* compressed DDS+zlib data (heap-alloc) */
    uint8_t *raw_data;   /* decompressed data (heap-alloc, NULL until loaded) */
    size_t   raw_len;    /* actual byte length of raw_data after decompression */
} ImgSprite;

/* Extended frame fields for Ver5 SpriteZlib frames */
typedef struct {
    int32_t  keep;
    int32_t  sprite_index;
    int32_t  left, top, right, bottom;
    int32_t  rotate;
} SpriteFrameExt;

/* Internal IMG private data */
typedef struct {
    DnfIO       io;           /* IO handle (copy) */
    int32_t     keep;
    int32_t     images_size;  /* header size field from new-magic IMGs */

    /* Ver4/5 single palette, Ver6 multiple palettes */
    ColorBoard *palettes;
    int         palette_count;

    /* Ver5 sprite atlas */
    ImgSprite  *sprites;
    int         sprite_count;
    int32_t     file_size_v5; /* full file size stored in Ver5 header */

    /* Per-frame sprite extension data (Ver5 DDS_ZLIB frames) */
    SpriteFrameExt *sprite_exts; /* array of frame_count, NULL for non-Ver5 */
} ImgPriv;

/* ------------------------------------------------------------------ */
/* Version ops dispatch                                                */
/* ------------------------------------------------------------------ */
typedef DnfError (*ImgReadFn)(DnfIO *io, ImgFile *img, ImgPriv *priv);
typedef DnfError (*ImgWriteFn)(DnfIO *io, ImgFile *img, ImgPriv *priv);

typedef struct {
    int        version;
    ImgReadFn  read;
    ImgWriteFn write;
} ImgVersionOps;

/* ------------------------------------------------------------------ */
/* Internal API used by dnflib.c                                       */
/* ------------------------------------------------------------------ */

/* Open an IMG from a DnfIO source */
DnfError img_open(DnfIO *io, ImgFile **out);

/* Save an IMG to a DnfIO sink (FILE mode only) */
DnfError img_save(DnfIO *io, ImgFile *img);

/* Free all resources held by ImgPriv */
void img_priv_free(ImgPriv *priv, int frame_count);

/* zlib decompress with PyDnfEx-compatible header detection */
DnfError dnf_zlib_decompress(const uint8_t *src, size_t src_len,
                               uint8_t **out, size_t *out_len);

/* zlib compress (standard) */
DnfError dnf_zlib_compress(const uint8_t *src, size_t src_len,
                             uint8_t **out, size_t *out_len);

#endif /* DNF_IMG_H */
