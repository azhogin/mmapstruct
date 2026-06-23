# String Layout

No length prefix. The length is derived from the container's range boundaries.

## Dynamic String (default)

```
[UTF-8 bytes]  — variable length, no null terminator
```

Range provided by the struct's offset table: `[offset_table[i], offset_table[i+1])`.

## Inline String (`#[mmap(inline(N))]` on a String field)

```
[UTF-8 bytes] [0x00 padding to N]
```

Fixed size N bytes. If the string is shorter than N, the remainder is zero-filled.
On read, trailing null bytes are stripped.
