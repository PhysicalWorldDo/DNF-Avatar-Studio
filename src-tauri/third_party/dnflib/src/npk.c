/*
 * npk.c - NPK file read/write
 *
 * NPK file layout:
 *   magic[16]          "NeoplePack_Bill\0"
 *   count[4]           int32LE
 *   index[264 * count] { offset(i32) + size(i32) + name[256](XOR encrypted) }
 *   sha256[32]         SHA-256 of header section (floor(N/17)*17 bytes)
 *   data...            IMG file data concatenated
 *
 * Header section for hashing = magic(16) + count(4) + index(264*count)
 *   total = 20 + 264*count bytes
 * Hash covers: floor(total / 17) * 17  bytes
 *
 * XOR key: first 36 bytes = "puchikon@neople dungeon and fighter "
 *          bytes [36..254] = "DNF" repeating (cycle of 3)
 *          byte  [255]     = 0x00
 */
#include "npk.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include "sha256.h"

/* ------------------------------------------------------------------ */
/* XOR key (cached, built once)                                        */
/* ------------------------------------------------------------------ */
static uint8_t g_xor_key[256];
static int     g_xor_key_init = 0;

static void build_xor_key(void) {
    if (g_xor_key_init) return;
    const char *header = NPK_KEY_HEADER;  /* 36 bytes */
    size_t hlen = strlen(header);
    memcpy(g_xor_key, header, hlen);
    const char dnf[3] = {'D','N','F'};
    size_t i;
    for (i = hlen; i < 255; i++) {
        g_xor_key[i] = (uint8_t)dnf[i % 3];
    }
    g_xor_key[255] = 0x00;
    g_xor_key_init = 1;
}

void npk_xor_name(uint8_t name[256]) {
    build_xor_key();
    int i;
    for (i = 0; i < 256; i++) name[i] ^= g_xor_key[i];
}

/* ------------------------------------------------------------------ */
/* npk_open                                                            */
/* ------------------------------------------------------------------ */
DnfError npk_open(DnfIO *io, NpkFile **out, int owns_fp) {
    char magic[17] = {0};
    DnfError e;

    /* Read 16-byte magic */
    if ((e = dnf_io_read(io, magic, 16))) return e;
    if (memcmp(magic, NPK_MAGIC, 15) != 0) return DNF_ERR_INVALID_MAGIC;

    int32_t count;
    if ((e = dnf_io_read_i32(io, &count))) return e;
    if (count < 0 || count > 65535) return DNF_ERR_INVALID_FORMAT;

    NpkFile *npk = (NpkFile *)calloc(1, sizeof(NpkFile));
    if (!npk) return DNF_ERR_OOM;

    if (count > 0) {
        npk->entries = (NpkEntry *)calloc((size_t)count, sizeof(NpkEntry));
        if (!npk->entries) { free(npk); return DNF_ERR_OOM; }
    }
    npk->count = count;

    int i;
    for (i = 0; i < count; i++) {
        NpkEntry *ent = &npk->entries[i];
        if ((e = dnf_io_read_i32(io, &ent->offset))) goto fail;
        if ((e = dnf_io_read_i32(io, &ent->size)))   goto fail;

        uint8_t enc_name[256];
        if ((e = dnf_io_read(io, enc_name, 256))) goto fail;
        npk_xor_name(enc_name);
        /* Copy decrypted name (raw EUC-KR bytes, NUL-terminated) */
        memcpy(ent->name, enc_name, 256);
        ent->name[255] = '\0';
    }

    /* Allocate and store NpkPriv */
    NpkPriv *priv = (NpkPriv *)calloc(1, sizeof(NpkPriv));
    if (!priv) { e = DNF_ERR_OOM; goto fail; }
    priv->io      = *io;
    priv->owns_fp = owns_fp;
    npk->_priv    = priv;

    *out = npk;
    return DNF_OK;

fail:
    if (npk->entries) free(npk->entries);
    free(npk);
    return e;
}

/* ------------------------------------------------------------------ */
/* Simple MD5 for deduplication (only need equality comparison)       */
/* We use a fast 128-bit hash; using SHA-256 would also work but MD5  */
/* is what PyDnfEx uses. We implement a minimal MD5 here.             */
/* ------------------------------------------------------------------ */

/* RFC 1321 MD5 implementation (minimal, self-contained) */
typedef struct { uint32_t s[4]; uint32_t n[2]; uint8_t buf[64]; } md5_ctx;

static const uint32_t md5_T[64] = {
    0xd76aa478,0xe8c7b756,0x242070db,0xc1bdceee,0xf57c0faf,0x4787c62a,0xa8304613,0xfd469501,
    0x698098d8,0x8b44f7af,0xffff5bb1,0x895cd7be,0x6b901122,0xfd987193,0xa679438e,0x49b40821,
    0xf61e2562,0xc040b340,0x265e5a51,0xe9b6c7aa,0xd62f105d,0x02441453,0xd8a1e681,0xe7d3fbc8,
    0x21e1cde6,0xc33707d6,0xf4d50d87,0x455a14ed,0xa9e3e905,0xfcefa3f8,0x676f02d9,0x8d2a4c8a,
    0xfffa3942,0x8771f681,0x6d9d6122,0xfde5380c,0xa4beea44,0x4bdecfa9,0xf6bb4b60,0xbebfbc70,
    0x289b7ec6,0xeaa127fa,0xd4ef3085,0x04881d05,0xd9d4d039,0xe6db99e5,0x1fa27cf8,0xc4ac5665,
    0xf4292244,0x432aff97,0xab9423a7,0xfc93a039,0x655b59c3,0x8f0ccc92,0xffeff47d,0x85845dd1,
    0x6fa87e4f,0xfe2ce6e0,0xa3014314,0x4e0811a1,0xf7537e82,0xbd3af235,0x2ad7d2bb,0xeb86d391
};
static const uint8_t md5_s[64] = {
    7,12,17,22,7,12,17,22,7,12,17,22,7,12,17,22,
    5, 9,14,20,5, 9,14,20,5, 9,14,20,5, 9,14,20,
    4,11,16,23,4,11,16,23,4,11,16,23,4,11,16,23,
    6,10,15,21,6,10,15,21,6,10,15,21,6,10,15,21
};
#define MD5_ROL(x,n) (((x)<<(n))|((x)>>(32-(n))))
#define MD5_F(b,c,d) (((b)&(c))|(~(b)&(d)))
#define MD5_G(b,c,d) (((d)&(b))|(~(d)&(c)))
#define MD5_H(b,c,d) ((b)^(c)^(d))
#define MD5_I(b,c,d) ((c)^((b)|(~(d))))

static void md5_transform(uint32_t s[4], const uint8_t *block) {
    uint32_t w[16], a=s[0], b=s[1], c=s[2], d=s[3];
    int i;
    for (i=0;i<16;i++) w[i]=(uint32_t)block[i*4]|((uint32_t)block[i*4+1]<<8)|((uint32_t)block[i*4+2]<<16)|((uint32_t)block[i*4+3]<<24);
    for (i=0;i<64;i++) {
        uint32_t f,g;
        if (i<16){f=MD5_F(b,c,d);g=(uint32_t)i;}
        else if(i<32){f=MD5_G(b,c,d);g=(uint32_t)(5*i+1)%16;}
        else if(i<48){f=MD5_H(b,c,d);g=(uint32_t)(3*i+5)%16;}
        else{f=MD5_I(b,c,d);g=(uint32_t)(7*i)%16;}
        f=f+a+md5_T[i]+w[g];
        a=d; d=c; c=b; b=b+MD5_ROL(f,md5_s[i]);
    }
    s[0]+=a; s[1]+=b; s[2]+=c; s[3]+=d;
}

static void md5_hash(const uint8_t *data, size_t len, uint8_t digest[16]) {
    md5_ctx ctx;
    ctx.s[0]=0x67452301u; ctx.s[1]=0xefcdab89u; ctx.s[2]=0x98badcfeu; ctx.s[3]=0x10325476u;
    ctx.n[0]=ctx.n[1]=0;
    size_t i=0;
    while (i+64<=len) { md5_transform(ctx.s, data+i); i+=64; }
    uint8_t pad[128]; size_t rest=len-i;
    memcpy(pad, data+i, rest);
    pad[rest]=0x80; memset(pad+rest+1,0,64-rest-1);
    uint64_t bits=(uint64_t)len*8;
    if (rest>=56) {
        md5_transform(ctx.s, pad);
        memset(pad,0,56);
    }
    for (i=0;i<8;i++) pad[56+i]=(uint8_t)(bits>>(i*8));
    md5_transform(ctx.s, pad);
    for (i=0;i<4;i++) { digest[i*4+0]=(uint8_t)(ctx.s[i]); digest[i*4+1]=(uint8_t)(ctx.s[i]>>8); digest[i*4+2]=(uint8_t)(ctx.s[i]>>16); digest[i*4+3]=(uint8_t)(ctx.s[i]>>24); }
}

/* ------------------------------------------------------------------ */
/* npk_create                                                          */
/* ------------------------------------------------------------------ */
DnfError npk_create(NpkFile **out) {
    NpkFile *npk = (NpkFile *)calloc(1, sizeof(NpkFile));
    if (!npk) return DNF_ERR_OOM;

    NpkPriv *priv = (NpkPriv *)calloc(1, sizeof(NpkPriv));
    if (!priv) { free(npk); return DNF_ERR_OOM; }

    /* No source IO — io is zeroed, owns_fp = 0 */
    npk->_priv = priv;
    *out = npk;
    return DNF_OK;
}

/* ------------------------------------------------------------------ */
/* npk_add_entry                                                       */
/* ------------------------------------------------------------------ */
DnfError npk_add_entry(NpkFile *npk, const char *name, const uint8_t *data, int32_t size) {
    if (!npk || !name || !data || size <= 0) return DNF_ERR_INVALID_FORMAT;

    NpkPriv *priv = (NpkPriv *)npk->_priv;
    if (!priv) return DNF_ERR_IO;

    int new_count = npk->count + 1;

    /* Grow entries array */
    NpkEntry *new_entries = (NpkEntry *)realloc(npk->entries,
                                                 (size_t)new_count * sizeof(NpkEntry));
    if (!new_entries) return DNF_ERR_OOM;
    npk->entries = new_entries;

    /* Grow entry_bufs array */
    uint8_t **new_bufs = (uint8_t **)realloc(priv->entry_bufs,
                                              (size_t)new_count * sizeof(uint8_t *));
    if (!new_bufs) return DNF_ERR_OOM;
    priv->entry_bufs = new_bufs;
    priv->entry_bufs_count = new_count;

    /* Copy data */
    uint8_t *buf = (uint8_t *)malloc((size_t)size);
    if (!buf) return DNF_ERR_OOM;
    memcpy(buf, data, (size_t)size);
    priv->entry_bufs[npk->count] = buf;

    /* Fill entry (offset will be computed at save time; set 0 for now) */
    NpkEntry *ent = &npk->entries[npk->count];
    memset(ent->name, 0, 256);
    strncpy(ent->name, name, 255);
    ent->name[255] = '\0';
    ent->offset = 0;
    ent->size   = size;

    npk->count = new_count;
    return DNF_OK;
}

/* ------------------------------------------------------------------ */
/* npk_save helper: get data pointer for entry i                       */
/* ------------------------------------------------------------------ */
/* Returns inline buf if available; otherwise NULL (caller uses priv->io). */
static const uint8_t *npk_entry_data_ptr(NpkPriv *priv, int i) {
    if (!priv) return NULL;
    if (priv->entry_bufs && i < priv->entry_bufs_count)
        return priv->entry_bufs[i];
    return NULL;
}

/* ------------------------------------------------------------------ */
/* npk_save                                                            */
/* ------------------------------------------------------------------ */
DnfError npk_save(const char *path, NpkFile *npk, int group_by_md5) {
    FILE *fp = dnf_fopen_utf8(path, "wb");
    if (!fp) return DNF_ERR_IO;

    DnfIO io;
    dnf_io_from_file(&io, fp);
    DnfError e = DNF_OK;

    /* Compute header size */
    size_t header_size = (size_t)(NPK_HEADER_BASE + npk->count * NPK_INDEX_ENTRY_SIZE);

    /* src_offsets: original offsets in source file (for io-backed entries) */
    int32_t *src_offsets = NULL;
    /* new_offsets: final offsets in output file */
    int32_t *new_offsets = NULL;
    if (npk->count > 0) {
        src_offsets = (int32_t *)malloc((size_t)npk->count * sizeof(int32_t));
        new_offsets = (int32_t *)malloc((size_t)npk->count * sizeof(int32_t));
        if (!src_offsets || !new_offsets) {
            free(src_offsets); free(new_offsets);
            fclose(fp); return DNF_ERR_OOM;
        }
        for (int k = 0; k < npk->count; k++) {
            src_offsets[k] = npk->entries[k].offset;
        }
    }

    NpkPriv *priv = (NpkPriv *)npk->_priv;

    /* group_by_md5: map md5 -> new_offset for dedup */
    typedef struct { uint8_t md5[16]; int32_t new_offset; } Md5Map;
    Md5Map *md5map = NULL;
    int md5map_count = 0;
    if (group_by_md5 && npk->count > 0) {
        md5map = (Md5Map *)calloc((size_t)npk->count, sizeof(Md5Map));
        if (!md5map) { free(src_offsets); free(new_offsets); fclose(fp); return DNF_ERR_OOM; }
    }

    int32_t current_offset = (int32_t)(header_size + NPK_HASH_SIZE);
    int i;

    if (group_by_md5) {
        for (i = 0; i < npk->count; i++) {
            NpkEntry *ent = &npk->entries[i];
            uint8_t *img_data = NULL;
            int      img_owned = 0;

            const uint8_t *inline_buf = npk_entry_data_ptr(priv, i);
            if (inline_buf) {
                img_data = (uint8_t *)inline_buf;
            } else if (priv) {
                img_data = (uint8_t *)malloc((size_t)ent->size);
                if (!img_data) { e = DNF_ERR_OOM; goto done; }
                if (dnf_io_seek(&priv->io, src_offsets[i], SEEK_SET) ||
                    dnf_io_read(&priv->io, img_data, (size_t)ent->size)) {
                    free(img_data); e = DNF_ERR_IO; goto done;
                }
                img_owned = 1;
            } else {
                e = DNF_ERR_IO; goto done;
            }

            uint8_t digest[16];
            md5_hash(img_data, (size_t)ent->size, digest);
            if (img_owned) free(img_data);

            int found = -1, j;
            for (j = 0; j < md5map_count; j++) {
                if (memcmp(md5map[j].md5, digest, 16) == 0) { found = j; break; }
            }
            if (found >= 0) {
                new_offsets[i] = md5map[found].new_offset;
            } else {
                new_offsets[i] = current_offset;
                memcpy(md5map[md5map_count].md5, digest, 16);
                md5map[md5map_count].new_offset = current_offset;
                md5map_count++;
                current_offset += ent->size;
            }
        }
    } else {
        for (i = 0; i < npk->count; i++) {
            new_offsets[i] = current_offset;
            current_offset += npk->entries[i].size;
        }
    }

    /* --- Build header in memory for SHA-256 --- */
    uint8_t *header_buf = (uint8_t *)malloc(header_size);
    if (!header_buf) { e = DNF_ERR_OOM; goto done; }

    /* Write magic + count */
    memcpy(header_buf, NPK_MAGIC, 15); header_buf[15] = 0;
    header_buf[16] = (uint8_t)(npk->count);
    header_buf[17] = (uint8_t)(npk->count >> 8);
    header_buf[18] = (uint8_t)(npk->count >> 16);
    header_buf[19] = (uint8_t)(npk->count >> 24);

    for (i = 0; i < npk->count; i++) {
        uint8_t *entry_ptr = header_buf + 20 + i * 264;
        NpkEntry *ent = &npk->entries[i];
        int32_t out_offset = new_offsets[i];
        entry_ptr[0]=(uint8_t)(out_offset);       entry_ptr[1]=(uint8_t)(out_offset>>8);
        entry_ptr[2]=(uint8_t)(out_offset>>16);   entry_ptr[3]=(uint8_t)(out_offset>>24);
        entry_ptr[4]=(uint8_t)(ent->size);         entry_ptr[5]=(uint8_t)(ent->size>>8);
        entry_ptr[6]=(uint8_t)(ent->size>>16);     entry_ptr[7]=(uint8_t)(ent->size>>24);
        uint8_t enc_name[256];
        memcpy(enc_name, ent->name, 256);
        npk_xor_name(enc_name);
        memcpy(entry_ptr + 8, enc_name, 256);
    }

    /* Write header */
    if ((e = dnf_io_write(&io, header_buf, header_size))) { free(header_buf); goto done; }

    /* Write SHA-256 hash */
    {
        uint8_t hash[32];
        size_t hash_input_len = header_size / 17 * 17;
        sha256_hash(header_buf, hash_input_len, hash);
        free(header_buf);
        if ((e = dnf_io_write(&io, hash, 32))) goto done;
    }

    /* Write IMG data */
    for (i = 0; i < npk->count; i++) {
        NpkEntry *ent = &npk->entries[i];
        /* Skip dedup'd entries */
        if (group_by_md5) {
            int is_dup = 0, j;
            for (j = 0; j < i; j++) {
                if (new_offsets[j] == new_offsets[i]) { is_dup = 1; break; }
            }
            if (is_dup) continue;
        }

        const uint8_t *inline_buf = npk_entry_data_ptr(priv, i);
        if (inline_buf) {
            /* Inline data: write directly */
            e = dnf_io_write(&io, inline_buf, (size_t)ent->size);
            if (e) goto done;
        } else if (priv) {
            /* IO-backed: read then write */
            uint8_t *img_data = (uint8_t *)malloc((size_t)ent->size);
            if (!img_data) { e = DNF_ERR_OOM; goto done; }
            if (dnf_io_seek(&priv->io, src_offsets[i], SEEK_SET) ||
                dnf_io_read(&priv->io, img_data, (size_t)ent->size)) {
                free(img_data); e = DNF_ERR_IO; goto done;
            }
            e = dnf_io_write(&io, img_data, (size_t)ent->size);
            free(img_data);
            if (e) goto done;
        }
    }

done:
    if (src_offsets) free(src_offsets);
    if (new_offsets)  free(new_offsets);
    if (md5map)       free(md5map);
    fclose(fp);
    return e;
}
