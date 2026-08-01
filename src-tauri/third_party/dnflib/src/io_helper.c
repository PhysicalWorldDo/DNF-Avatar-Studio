/*
 * io_helper.c - IO abstraction implementation
 */
#include "io_helper.h"
#include <string.h>
#include <stdlib.h>
#ifdef _WIN32
#include <windows.h>
#include <wchar.h>
#endif

FILE *dnf_fopen_utf8(const char *path, const char *mode) {
#ifdef _WIN32
    int path_len;
    int mode_len;
    wchar_t *wide_path;
    wchar_t *wide_mode;
    FILE *fp = NULL;

    if (!path || !mode) return NULL;
    path_len = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, path, -1, NULL, 0);
    mode_len = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, mode, -1, NULL, 0);
    if (path_len <= 0 || mode_len <= 0) return NULL;

    wide_path = (wchar_t *)malloc((size_t)path_len * sizeof(wchar_t));
    wide_mode = (wchar_t *)malloc((size_t)mode_len * sizeof(wchar_t));
    if (!wide_path || !wide_mode) {
        free(wide_path);
        free(wide_mode);
        return NULL;
    }
    if (!MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, path, -1, wide_path, path_len) ||
        !MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, mode, -1, wide_mode, mode_len)) {
        free(wide_path);
        free(wide_mode);
        return NULL;
    }
#ifdef _MSC_VER
    if (_wfopen_s(&fp, wide_path, wide_mode) != 0) fp = NULL;
#else
    fp = _wfopen(wide_path, wide_mode);
#endif
    free(wide_path);
    free(wide_mode);
    return fp;
#else
    return fopen(path, mode);
#endif
}

void dnf_io_from_file(DnfIO *io, FILE *fp) {
    io->type  = DNF_IO_FILE;
    io->u.fp  = fp;
}

void dnf_io_from_mem(DnfIO *io, const uint8_t *buf, size_t len) {
    io->type        = DNF_IO_MEM;
    io->u.mem.buf   = buf;
    io->u.mem.len   = len;
    io->u.mem.pos   = 0;
}

DnfError dnf_io_read(DnfIO *io, void *out, size_t n) {
    if (n == 0) return DNF_OK;
    if (io->type == DNF_IO_FILE) {
        if (fread(out, 1, n, io->u.fp) != n) return DNF_ERR_IO;
    } else {
        if (io->u.mem.pos + n > io->u.mem.len) return DNF_ERR_IO;
        memcpy(out, io->u.mem.buf + io->u.mem.pos, n);
        io->u.mem.pos += n;
    }
    return DNF_OK;
}

DnfError dnf_io_write(DnfIO *io, const void *data, size_t n) {
    if (n == 0) return DNF_OK;
    if (io->type == DNF_IO_FILE) {
        if (fwrite(data, 1, n, io->u.fp) != n) return DNF_ERR_IO;
    } else {
        /* Writing to a const memory buffer is not supported */
        return DNF_ERR_IO;
    }
    return DNF_OK;
}

DnfError dnf_io_seek(DnfIO *io, long offset, int whence) {
    if (io->type == DNF_IO_FILE) {
        if (fseek(io->u.fp, offset, whence) != 0) return DNF_ERR_IO;
    } else {
        long newpos;
        switch (whence) {
            case SEEK_SET: newpos = offset; break;
            case SEEK_CUR: newpos = (long)io->u.mem.pos + offset; break;
            case SEEK_END: newpos = (long)io->u.mem.len + offset; break;
            default: return DNF_ERR_IO;
        }
        if (newpos < 0 || (size_t)newpos > io->u.mem.len) return DNF_ERR_IO;
        io->u.mem.pos = (size_t)newpos;
    }
    return DNF_OK;
}

long dnf_io_tell(DnfIO *io) {
    if (io->type == DNF_IO_FILE)
        return ftell(io->u.fp);
    return (long)io->u.mem.pos;
}

/* ---- little-endian typed reads ---- */

DnfError dnf_io_read_u8(DnfIO *io, uint8_t *v) {
    return dnf_io_read(io, v, 1);
}

DnfError dnf_io_read_u16(DnfIO *io, uint16_t *v) {
    uint8_t b[2];
    DnfError e = dnf_io_read(io, b, 2);
    if (e) return e;
    *v = (uint16_t)(b[0] | ((uint16_t)b[1] << 8));
    return DNF_OK;
}

DnfError dnf_io_read_u32(DnfIO *io, uint32_t *v) {
    uint8_t b[4];
    DnfError e = dnf_io_read(io, b, 4);
    if (e) return e;
    *v = (uint32_t)b[0] | ((uint32_t)b[1]<<8) | ((uint32_t)b[2]<<16) | ((uint32_t)b[3]<<24);
    return DNF_OK;
}

DnfError dnf_io_read_i32(DnfIO *io, int32_t *v) {
    uint32_t u;
    DnfError e = dnf_io_read_u32(io, &u);
    if (e) return e;
    *v = (int32_t)u;
    return DNF_OK;
}

DnfError dnf_io_read_i16(DnfIO *io, int16_t *v) {
    uint16_t u;
    DnfError e = dnf_io_read_u16(io, &u);
    if (e) return e;
    *v = (int16_t)u;
    return DNF_OK;
}

/* ---- little-endian typed writes ---- */

DnfError dnf_io_write_u8(DnfIO *io, uint8_t v) {
    return dnf_io_write(io, &v, 1);
}

DnfError dnf_io_write_u16(DnfIO *io, uint16_t v) {
    uint8_t b[2] = { (uint8_t)(v), (uint8_t)(v>>8) };
    return dnf_io_write(io, b, 2);
}

DnfError dnf_io_write_u32(DnfIO *io, uint32_t v) {
    uint8_t b[4] = { (uint8_t)(v), (uint8_t)(v>>8), (uint8_t)(v>>16), (uint8_t)(v>>24) };
    return dnf_io_write(io, b, 4);
}

DnfError dnf_io_write_i32(DnfIO *io, int32_t v) {
    return dnf_io_write_u32(io, (uint32_t)v);
}

/* ---- string helpers ---- */

DnfError dnf_io_read_ascii(DnfIO *io, char *out, size_t n) {
    DnfError e = dnf_io_read(io, out, n);
    if (e) return e;
    out[n] = '\0'; /* caller must ensure out has n+1 bytes */
    return DNF_OK;
}

DnfError dnf_io_write_ascii(DnfIO *io, const char *str) {
    size_t len = strlen(str);
    DnfError e = dnf_io_write(io, str, len);
    if (e) return e;
    return dnf_io_write_u8(io, 0);
}
