# File Header (24 bytes)

Prepended by `write_to_buf` / `write_to_file`.
`write_to_buf_raw` / `write_raw` — no header.

```
Offset  Size  Field        Description
------  ----  -----------  ---------------------------------------------------
 0       4    magic        u32 LE = 0x50414D4D ("MMAP")
 4       2    hdr_ver      u16 LE = 1 (v1) or 2 (v2)
 6       2    flags        u16 LE:
                              bit 0 (0x01) = HEADER_FLAG_HASH_PRESENT
                              bit 1 (0x02) = HEADER_FLAG_HAS_SCHEMA
 8       8    layout_hash  u64 LE — FNV-1a hash of the struct layout
16       8    crc64        u64 LE — CRC-64-ECMA over payload (0 = skip check)
```

Header version 2: if `flags & 0x02`, the header is followed by:

```
Offset  Size   Field
------  -----  ---------------------------------------------------
 24      4     schema_size: u32 LE
 28      S     schema data (S bytes)
 28+S    P     zero-padding to 8 (P = (S+7)&!7 - S)
 28+S+P       payload
```

CRC always covers the payload only, not the header or schema.
