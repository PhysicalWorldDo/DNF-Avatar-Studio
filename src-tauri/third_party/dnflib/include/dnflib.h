/*
 * dnflib.h - Public API for DNF NPK/IMG file library
 * Pure C99, zero external dependencies.
 */
#ifndef DNFLIB_H
#define DNFLIB_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ------------------------------------------------------------------ */
/* DLL export/import macros                                            */
/* ------------------------------------------------------------------ */
#if defined(_WIN32) || defined(_WIN64)
#  ifdef DNFLIB_BUILD_DLL
#    define DNFLIB_API __declspec(dllexport)
#  elif defined(DNFLIB_USE_DLL)
#    define DNFLIB_API __declspec(dllimport)
#  else
#    define DNFLIB_API
#  endif
#else
#  define DNFLIB_API
#endif

/* ------------------------------------------------------------------ */
/* Error codes                                                         */
/* ------------------------------------------------------------------ */
typedef enum {
    DNF_OK                    = 0,
    DNF_ERR_INVALID_MAGIC     = 1,
    DNF_ERR_IO                = 2,
    DNF_ERR_OOM               = 3,
    DNF_ERR_UNSUPPORTED_VERSION = 4,
    DNF_ERR_INVALID_FORMAT    = 5,
    DNF_ERR_DECOMPRESS        = 6,
    DNF_ERR_INVALID_INDEX     = 7,
} DnfError;

/* ------------------------------------------------------------------ */
/* Image pixel format                                                  */
/* ------------------------------------------------------------------ */
typedef enum {
    DNF_FMT_ARGB_1555 = 0x0e,
    DNF_FMT_ARGB_4444 = 0x0f,
    DNF_FMT_ARGB_8888 = 0x10,
    DNF_FMT_LINK      = 0x11,
    DNF_FMT_DXT_1     = 0x12,
    DNF_FMT_DXT_3     = 0x13,
    DNF_FMT_DXT_5     = 0x14,
} DnfImageFormat;

/* ------------------------------------------------------------------ */
/* Compression mode                                                    */
/* ------------------------------------------------------------------ */
typedef enum {
    DNF_COMPRESS_NONE     = 0x05,
    DNF_COMPRESS_ZLIB     = 0x06,
    DNF_COMPRESS_DDS_ZLIB = 0x07,
} DnfCompressMode;

/* ------------------------------------------------------------------ */
/* NPK structures                                                      */
/* ------------------------------------------------------------------ */

/* One entry in the NPK index table.
 * name: raw EUC-KR bytes after XOR decryption, NUL-terminated. */
typedef struct {
    char     name[256];
    int32_t  offset;
    int32_t  size;
} NpkEntry;

typedef struct {
    NpkEntry *entries;
    int       count;
    void     *_priv;   /* internal: NpkPriv* */
} NpkFile;

/* ------------------------------------------------------------------ */
/* IMG structures                                                      */
/* ------------------------------------------------------------------ */

/* One image frame.
 * data: raw (possibly compressed) pixel bytes, NULL until loaded.
 * Call dnf_image_decode() to get RGBA8888. */
typedef struct {
    DnfImageFormat  format;
    DnfCompressMode compress;
    int32_t  w, h;       /* image width/height in pixels */
    int32_t  x, y;       /* draw offset */
    int32_t  mw, mh;     /* frame (canvas) width/height */
    uint8_t *data;       /* raw data (compressed or plain), heap-alloc */
    int32_t  data_size;
    int32_t  link_index; /* valid when format == DNF_FMT_LINK */
} ImageFrame;

typedef struct {
    int          version;
    ImageFrame  *frames;
    int          frame_count;
    void        *_priv;  /* internal: ImgPriv* */
} ImgFile;

/* ------------------------------------------------------------------ */
/* NPK API                                                             */
/* ------------------------------------------------------------------ */
DNFLIB_API DnfError dnf_npk_open_file(const char *path, NpkFile **out);
DNFLIB_API DnfError dnf_npk_open_mem(const uint8_t *buf, size_t len, NpkFile **out);
DNFLIB_API DnfError dnf_npk_read_img(NpkFile *npk, int index, ImgFile **out);
DNFLIB_API DnfError dnf_npk_save_file(const char *path, NpkFile *npk, int group_by_md5);
DNFLIB_API void     dnf_npk_free(NpkFile *npk);

/* NPK construction API: create an empty NpkFile and populate it entry by entry,
 * then call dnf_npk_save_file() to write it out.
 *
 * dnf_npk_create()    -- allocate a new empty NpkFile
 * dnf_npk_add_entry() -- append one entry (data is copied internally)
 *                        name: NUL-terminated EUC-KR string, max 255 bytes
 *                        data: raw IMG bytes; size must be > 0
 * After saving, free with dnf_npk_free() as usual.
 */
DNFLIB_API DnfError dnf_npk_create(NpkFile **out);
DNFLIB_API DnfError dnf_npk_add_entry(NpkFile *npk, const char *name,
                                       const uint8_t *data, int32_t size);

/* ------------------------------------------------------------------ */
/* IMG API                                                             */
/* ------------------------------------------------------------------ */
DNFLIB_API DnfError dnf_img_open_file(const char *path, ImgFile **out);
DNFLIB_API DnfError dnf_img_open_mem(const uint8_t *buf, size_t len, ImgFile **out);
DNFLIB_API DnfError dnf_img_save_file(const char *path, ImgFile *img);
DNFLIB_API int      dnf_img_palette_count(ImgFile *img);
DNFLIB_API void     dnf_img_free(ImgFile *img);

/* ------------------------------------------------------------------ */
/* Image decode/encode API                                             */
/* ------------------------------------------------------------------ */

/* Decode frame frame_index to RGBA8888.
 * Allocates *out_rgba (w*h*4 bytes); caller must free with dnf_free(). */
DNFLIB_API DnfError dnf_image_decode(ImgFile *img, int frame_index,
                                     uint8_t **out_rgba, int *out_w, int *out_h);

/* Decode frame frame_index with a requested palette index for multi-palette
 * IMG files. Non-palette frame formats ignore palette_index. */
DNFLIB_API DnfError dnf_image_decode_with_palette(ImgFile *img, int frame_index,
                                                  int palette_index,
                                                  uint8_t **out_rgba,
                                                  int *out_w, int *out_h);

/* Encode RGBA8888 pixels to the given format/compress mode.
 * Allocates *out_data; caller must free with dnf_free(). */
DNFLIB_API DnfError dnf_image_encode(const uint8_t *rgba, int w, int h,
                                     DnfImageFormat fmt, DnfCompressMode compress,
                                     uint8_t **out_data, int32_t *out_size);

/* Free a buffer returned by this library */
DNFLIB_API void dnf_free(void *ptr);

#ifdef __cplusplus
}
#endif

#endif /* DNFLIB_H */
