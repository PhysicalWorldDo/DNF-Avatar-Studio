/*
 * dnflib.c - Public API implementation
 *
 * Connects NPK / IMG / pixel / DXT modules to the public C API
 * declared in include/dnflib.h.
 */
#include "dnflib.h"
#include "io_helper.h"
#include "npk.h"
#include "img.h"
#include "pixel.h"
#include "dxt.h"
#include "palette.h"
#include <stdlib.h>
#include <string.h>

/* ------------------------------------------------------------------ */
/* NPK API                                                             */
/* ------------------------------------------------------------------ */

DNFLIB_API DnfError dnf_npk_open_file(const char *path, NpkFile **out) {
    FILE *fp = fopen(path, "rb");
    if (!fp) return DNF_ERR_IO;
    DnfIO io;
    dnf_io_from_file(&io, fp);
    DnfError e = npk_open(&io, out, 1 /* owns_fp */);
    if (e) fclose(fp);
    return e;
}

DNFLIB_API DnfError dnf_npk_open_mem(const uint8_t *buf, size_t len, NpkFile **out) {
    DnfIO io;
    dnf_io_from_mem(&io, buf, len);
    return npk_open(&io, out, 0);
}

DNFLIB_API DnfError dnf_npk_read_img(NpkFile *npk, int index, ImgFile **out) {
    if (!npk || index < 0 || index >= npk->count) return DNF_ERR_INVALID_INDEX;
    NpkEntry *ent = &npk->entries[index];
    NpkPriv  *priv = (NpkPriv *)npk->_priv;
    if (!priv) return DNF_ERR_IO;

    /* Seek to IMG data region in NPK */
    DnfError e = dnf_io_seek(&priv->io, (long)ent->offset, SEEK_SET);
    if (e) return e;

    /* Wrap a sub-view of the NPK IO as an IMG memory buffer */
    /* Read the IMG bytes into a temp buffer, then parse */
    uint8_t *img_buf = (uint8_t *)malloc((size_t)ent->size);
    if (!img_buf) return DNF_ERR_OOM;

    e = dnf_io_read(&priv->io, img_buf, (size_t)ent->size);
    if (e) { free(img_buf); return e; }

    DnfIO img_io;
    dnf_io_from_mem(&img_io, img_buf, (size_t)ent->size);
    e = img_open(&img_io, out);
    free(img_buf);
    return e;
}

DNFLIB_API DnfError dnf_npk_save_file(const char *path, NpkFile *npk, int group_by_md5) {
    return npk_save(path, npk, group_by_md5);
}

DNFLIB_API DnfError dnf_npk_create(NpkFile **out) {
    return npk_create(out);
}

DNFLIB_API DnfError dnf_npk_add_entry(NpkFile *npk, const char *name,
                                       const uint8_t *data, int32_t size) {
    return npk_add_entry(npk, name, data, size);
}

DNFLIB_API void dnf_npk_free(NpkFile *npk) {
    if (!npk) return;
    NpkPriv *priv = (NpkPriv *)npk->_priv;
    if (priv) {
        if (priv->owns_fp && priv->io.type == DNF_IO_FILE && priv->io.u.fp) {
            fclose(priv->io.u.fp);
        }
        if (priv->entry_bufs) {
            int i;
            for (i = 0; i < priv->entry_bufs_count; i++) {
                if (priv->entry_bufs[i]) free(priv->entry_bufs[i]);
            }
            free(priv->entry_bufs);
        }
        free(priv);
    }
    if (npk->entries) free(npk->entries);
    free(npk);
}

/* ------------------------------------------------------------------ */
/* IMG API                                                             */
/* ------------------------------------------------------------------ */

DNFLIB_API DnfError dnf_img_open_file(const char *path, ImgFile **out) {
    FILE *fp = fopen(path, "rb");
    if (!fp) return DNF_ERR_IO;
    DnfIO io;
    dnf_io_from_file(&io, fp);
    DnfError e = img_open(&io, out);
    /* All frame data is eagerly loaded by img_open; we can close the file now.
     * If img_open fails, fp was never stored anywhere — also safe to close. */
    fclose(fp);
    if (!e) {
        /* Nullify the now-closed fp inside ImgPriv to prevent accidental reuse */
        ImgPriv *priv = (ImgPriv *)(*out)->_priv;
        if (priv && priv->io.type == DNF_IO_FILE) {
            priv->io.u.fp = NULL;
        }
    }
    return e;
}

DNFLIB_API DnfError dnf_img_open_mem(const uint8_t *buf, size_t len, ImgFile **out) {
    DnfIO io;
    dnf_io_from_mem(&io, buf, len);
    return img_open(&io, out);
}

DNFLIB_API DnfError dnf_img_save_file(const char *path, ImgFile *img) {
    FILE *fp = fopen(path, "wb");
    if (!fp) return DNF_ERR_IO;
    DnfIO io;
    dnf_io_from_file(&io, fp);
    DnfError e = img_save(&io, img);
    fclose(fp);
    return e;
}

DNFLIB_API void dnf_img_free(ImgFile *img) {
    if (!img) return;
    ImgPriv *priv = (ImgPriv *)img->_priv;
    if (img->frames) {
        int i;
        for (i = 0; i < img->frame_count; i++) {
            if (img->frames[i].data) free(img->frames[i].data);
        }
        free(img->frames);
    }
    img_priv_free(priv, img->frame_count);
    free(img);
}

DNFLIB_API int dnf_img_palette_count(ImgFile *img) {
    if (!img) return 0;
    ImgPriv *priv = (ImgPriv *)img->_priv;
    if (!priv || priv->palette_count <= 0) return 0;
    return priv->palette_count;
}

/* ------------------------------------------------------------------ */
/* Image decode                                                        */
/* ------------------------------------------------------------------ */

static int is_canvas_placeholder_frame(const ImageFrame *f) {
    if (!f || f->w != 1 || f->h != 1 || f->mw <= 0 || f->mh <= 0)
        return 0;

    return f->x >= f->mw || f->y >= f->mh ||
           f->x + f->w <= 0 || f->y + f->h <= 0;
}

static DnfError decode_transparent_frame(const ImageFrame *f,
                                         uint8_t **out_rgba,
                                         int *out_w,
                                         int *out_h) {
    if (!f || f->w <= 0 || f->h <= 0)
        return DNF_ERR_INVALID_FORMAT;

    size_t len = (size_t)f->w * (size_t)f->h * 4u;
    uint8_t *rgba = (uint8_t *)calloc(1, len);
    if (!rgba) return DNF_ERR_OOM;

    if (out_rgba) *out_rgba = rgba; else free(rgba);
    if (out_w) *out_w = f->w;
    if (out_h) *out_h = f->h;
    return DNF_OK;
}

DNFLIB_API DnfError dnf_image_decode(ImgFile *img, int frame_index,
                                     uint8_t **out_rgba, int *out_w, int *out_h) {
    return dnf_image_decode_with_palette(img, frame_index, 0, out_rgba, out_w, out_h);
}

DNFLIB_API DnfError dnf_image_decode_with_palette(ImgFile *img, int frame_index,
                                                  int palette_index,
                                                  uint8_t **out_rgba,
                                                  int *out_w,
                                                  int *out_h) {
    if (!img || frame_index < 0 || frame_index >= img->frame_count)
        return DNF_ERR_INVALID_INDEX;
    if (palette_index < 0)
        return DNF_ERR_INVALID_INDEX;

    ImageFrame *f = &img->frames[frame_index];

    /* LINK frames redirect to another frame */
    if (f->format == DNF_FMT_LINK) {
        return dnf_image_decode_with_palette(img, f->link_index, palette_index,
                                             out_rgba, out_w, out_h);
    }

    /* Ver5 Sprite Atlas frames: data == NULL, data comes from ImgPriv.sprites[] */
    ImgPriv *priv = (ImgPriv *)img->_priv;
    if (!f->data || f->data_size <= 0) {
        if (f->compress != DNF_COMPRESS_DDS_ZLIB || !priv ||
            !priv->sprite_exts || !priv->sprites) {
            return DNF_ERR_INVALID_FORMAT;
        }
        SpriteFrameExt *ext = &priv->sprite_exts[frame_index];
        if (ext->sprite_index < 0 || ext->sprite_index >= priv->sprite_count)
            return DNF_ERR_INVALID_INDEX;
        ImgSprite *sp = &priv->sprites[ext->sprite_index];

        /* Decompress sprite DDS data if not yet cached */
        if (!sp->raw_data) {
            if (!sp->zip_data || sp->data_size <= 0) return DNF_ERR_INVALID_FORMAT;
            DnfError e2 = dnf_zlib_decompress(sp->zip_data, (size_t)sp->data_size,
                                               &sp->raw_data, &sp->raw_len);
            if (e2) return e2;
        }

        /* Decode full sprite texture */
        uint8_t *full_rgba = NULL;
        int sw = 0, sh = 0;
        DnfError e2 = dxt_decode_dds(sp->raw_data, sp->raw_len,
                                      &full_rgba, &sw, &sh);
        if (e2) return e2;

        /* Crop the frame region from full texture */
        int fl = ext->left, ft = ext->top, fr = ext->right, fb = ext->bottom;
        if (fl < 0) fl = 0;
        if (ft < 0) ft = 0;
        if (fr > sw) fr = sw;
        if (fb > sh) fb = sh;
        int fw = fr - fl;
        int fh = fb - ft;
        if (fw <= 0 || fh <= 0) { free(full_rgba); return DNF_ERR_INVALID_FORMAT; }

        uint8_t *crop = (uint8_t *)malloc((size_t)(fw * fh * 4));
        if (!crop) { free(full_rgba); return DNF_ERR_OOM; }
        int row;
        for (row = 0; row < fh; row++) {
            memcpy(crop + row * fw * 4,
                   full_rgba + ((ft + row) * sw + fl) * 4,
                   (size_t)(fw * 4));
        }
        free(full_rgba);

        if (out_rgba) *out_rgba = crop; else free(crop);
        if (out_w) *out_w = fw;
        if (out_h) *out_h = fh;
        return DNF_OK;
    }

    int w = f->w, h = f->h;
    if (w <= 0 || h <= 0) return DNF_ERR_INVALID_FORMAT;
    DnfError e;

    /* Step 1: decompress if needed */
    uint8_t *raw = NULL;
    size_t   raw_len = 0;
    int      raw_owned = 0;

    if (f->compress == DNF_COMPRESS_ZLIB || f->compress == DNF_COMPRESS_DDS_ZLIB) {
        e = dnf_zlib_decompress(f->data, (size_t)f->data_size, &raw, &raw_len);
        if (e) {
            if (is_canvas_placeholder_frame(f))
                return decode_transparent_frame(f, out_rgba, out_w, out_h);
            return e;
        }
        raw_owned = 1;
    } else {
        raw = f->data;
        raw_len = (size_t)f->data_size;
    }

    /* Step 2: DDS formats → use dxt decoder (raw is a DDS file) */
    if (f->format == DNF_FMT_DXT_1 ||
        f->format == DNF_FMT_DXT_3 ||
        f->format == DNF_FMT_DXT_5) {
        int dw = 0, dh = 0;
        e = dxt_decode_dds(raw, raw_len, out_rgba, &dw, &dh);
        if (raw_owned) free(raw);
        if (e) return e;
        if (out_w) *out_w = dw;
        if (out_h) *out_h = dh;
        return DNF_OK;
    }

    /* Step 3: palette-indexed formats (Ver4/5/6).
     * ARGB1555 frames with NONE compression store native 16-bit pixels even
     * when the IMG has a ColorBoard; only ZLIB payloads store byte indices. */
    if (f->format == DNF_FMT_ARGB_1555 &&
        f->compress == DNF_COMPRESS_ZLIB &&
        priv && priv->palette_count > 0) {
        if (palette_index >= priv->palette_count) {
            if (raw_owned) free(raw);
            return DNF_ERR_INVALID_INDEX;
        }
        /* palette indexed: raw bytes are 1-byte indices */
        int pixel_count = w * h;
        if (pixel_count <= 0 || raw_len < (size_t)pixel_count) {
            if (raw_owned) free(raw);
            return DNF_ERR_INVALID_FORMAT;
        }
        uint8_t *rgba = (uint8_t *)malloc((size_t)(pixel_count * 4));
        if (!rgba) { if (raw_owned) free(raw); return DNF_ERR_OOM; }
        e = palette_decode(&priv->palettes[palette_index], raw, pixel_count, rgba);
        if (raw_owned) free(raw);
        if (e) { free(rgba); return e; }
        if (out_rgba) *out_rgba = rgba;
        if (out_w) *out_w = w;
        if (out_h) *out_h = h;
        return DNF_OK;
    }

    /* Step 4: direct pixel format conversion */
    int ps = pixel_size(f->format);
    if (ps <= 0 || raw_len < (size_t)w * (size_t)h * (size_t)ps) {
        if (raw_owned) free(raw);
        return DNF_ERR_INVALID_FORMAT;
    }
    uint8_t *rgba = (uint8_t *)malloc((size_t)(w * h * 4));
    if (!rgba) { if (raw_owned) free(raw); return DNF_ERR_OOM; }

    e = pixel_to_rgba(raw, w, h, f->format, rgba);
    if (raw_owned) free(raw);
    if (e) { free(rgba); return e; }

    if (out_rgba) *out_rgba = rgba;
    if (out_w) *out_w = w;
    if (out_h) *out_h = h;
    return DNF_OK;
}

/* ------------------------------------------------------------------ */
/* Image encode                                                        */
/* ------------------------------------------------------------------ */

DNFLIB_API DnfError dnf_image_encode(const uint8_t *rgba, int w, int h,
                                     DnfImageFormat fmt, DnfCompressMode compress,
                                     uint8_t **out_data, int32_t *out_size) {
    if (!rgba || w <= 0 || h <= 0 || !out_data || !out_size)
        return DNF_ERR_INVALID_FORMAT;

    /* DXT encoding not supported (write-back only for uncompressed formats) */
    if (fmt == DNF_FMT_DXT_1 || fmt == DNF_FMT_DXT_3 || fmt == DNF_FMT_DXT_5)
        return DNF_ERR_UNSUPPORTED_VERSION;

    int ps = pixel_size(fmt);
    if (ps <= 0) return DNF_ERR_INVALID_FORMAT;

    size_t raw_len = (size_t)(w * h * ps);
    uint8_t *raw = (uint8_t *)malloc(raw_len);
    if (!raw) return DNF_ERR_OOM;

    DnfError e = pixel_from_rgba(rgba, w, h, fmt, raw);
    if (e) { free(raw); return e; }

    if (compress == DNF_COMPRESS_ZLIB) {
        uint8_t *zlib_data = NULL;
        size_t   zlib_len  = 0;
        e = dnf_zlib_compress(raw, raw_len, &zlib_data, &zlib_len);
        free(raw);
        if (e) return e;
        *out_data = zlib_data;
        *out_size = (int32_t)zlib_len;
    } else {
        *out_data = raw;
        *out_size = (int32_t)raw_len;
    }
    return DNF_OK;
}

/* ------------------------------------------------------------------ */
/* Generic free                                                        */
/* ------------------------------------------------------------------ */

DNFLIB_API void dnf_free(void *ptr) {
    free(ptr);
}
