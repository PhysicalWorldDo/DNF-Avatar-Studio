/*
 * npk.h - Internal NPK module
 */
#ifndef DNF_NPK_H
#define DNF_NPK_H

#include "dnflib.h"
#include "io_helper.h"

#define NPK_MAGIC     "NeoplePack_Bill"  /* 15 chars + NUL = 16 bytes */
#define NPK_INDEX_ENTRY_SIZE  264        /* offset(4) + size(4) + name(256) */
#define NPK_HEADER_BASE       20         /* magic(16) + count(4) */
#define NPK_HASH_SIZE         32         /* SHA-256 = 32 bytes */

/* XOR key: "puchikon@neople dungeon and fighter " + "DNF" repeating + NUL */
#define NPK_KEY_HEADER "puchikon@neople dungeon and fighter "

/* NpkPriv: holds open file handle and optional inline entry data buffers.
 *
 * entry_bufs[i]: when non-NULL, npk_save reads data for entry i from this
 *                buffer instead of from io. Set by npk_add_entry().
 * entry_bufs_count: mirrors NpkFile.count at allocation time.
 */
typedef struct {
    DnfIO     io;                /* IO handle for reading from source file */
    int       owns_fp;           /* whether we should fclose fp on free */
    uint8_t **entry_bufs;        /* per-entry inline data (may be NULL per slot) */
    int       entry_bufs_count;  /* allocated length of entry_bufs */
} NpkPriv;

/* Encrypt/decrypt a 256-byte name buffer in-place using XOR key */
void npk_xor_name(uint8_t name[256]);

/* Open an NPK from a DnfIO source */
DnfError npk_open(DnfIO *io, NpkFile **out, int owns_fp);

/* Create a new empty NpkFile (no source file; add entries via npk_add_entry) */
DnfError npk_create(NpkFile **out);

/* Append one entry to npk. name must be a NUL-terminated EUC-KR string (<= 255 bytes).
 * data is copied internally; caller may free after the call. */
DnfError npk_add_entry(NpkFile *npk, const char *name, const uint8_t *data, int32_t size);

/* Save an NPK to file path */
DnfError npk_save(const char *path, NpkFile *npk, int group_by_md5);

#endif /* DNF_NPK_H */
