# Struct Layout (non-pod)

```
+──────────────────────────────────────────────────────────────────────+
| FIXED SECTION (8-byte aligned)                                       |
|   [pod fields, inline strings, ByRefRange-placeholders]              |
|   pod fields: aligned to min(size, 8)                                |
|   ByRef: aligned to 4 (ByRefRange — 2 × u32)                        |
|   Trailing padding to 8                                              |
+──────────────────────────────────────────────────────────────────────+
| OFFSET TABLE (only if inline-dynamic fields exist)                    |
|   (inline_dyn_count + 1) × 8 bytes:                                  |
|     [u64 LE offset_0, offset_1, …, offset_N]                        |
|   offset_i — relative offset of the i-th field from values area      |
|   offset_N — total size of all inline values                         |
+──────────────────────────────────────────────────────────────────────+
| INLINE VALUES AREA (8-byte aligned)                                   |
|   [dynamic_string, vec, hashmap, custom…]                            |
|   Each field aligned to 8 before writing                             |
+──────────────────────────────────────────────────────────────────────+
| POSTPONED DATA (appended in finish / scope_end)                       |
|   [byref fields, byref vecs…]                                        |
+──────────────────────────────────────────────────────────────────────+
```

## Fixed section alignment

| Field type | Alignment | Size |
|---|---|---|
| Pod primitive | min(known_size, 8) | 1,2,4,8 |
| Pod custom | min(align_of::<T>(), 8) | size_of::<T>() |
| Inline String | 1 | N (user-specified) |
| ByRef (ByRefRange) | 4 | 8 |

## Pure-pod struct

Entire struct is a flat `repr(C)`-compatible byte sequence via `bytemuck`. No header / offset table.
