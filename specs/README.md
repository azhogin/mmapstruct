# mmapstruct Binary Layout Specifications

- [File Header](file-header.md) — 24-byte header + v2 schema extension
- [Struct](struct.md) — mixed struct layout (fixed section, offset table, inline values, postponed)
- [String](string.md) — dynamic and inline(N) strings
- [Vec](vec.md) — pod, fixed, indexed, byref vec layouts
- [Enum](enum.md) — pod-enum and variant-with-data enum layouts
- [Option](option.md) — None/Some with alignment padding
- [ByRef](byref.md) — ByRefRange (begin/end) layout
- [HashMap](hash_map.md) — 4 layout variants (podK/podV, inlinedK/inlinedV, …)
