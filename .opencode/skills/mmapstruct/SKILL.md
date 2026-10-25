---
name: mmapstruct
description: Authoring mmapstruct file formats and read/write/patch code — container selection, derive attributes, zero-copy reads, write APIs, reserves, PatchSession/SchemaPatchSession; verification workflows; made for agents
---

## When to use me

- Writing struct definitions for `.mms` file formats (`#[derive(MmapStruct)]`)
- Writing read / write / patch code against those formats
- Choosing container types and attributes; adding in-place patching

For inspecting existing `.mms` files use the `mmapview` (headless) or
`mmapview-tui` (visual) skills instead.

## Zero-copy vs materialization (read-side cost model)

- **Readers are zero-copy.** `T::Reader<'a>` (generated as `XReader<'a>`) is a
  view over the mmaped bytes: pod fields are `&'a T`, pod vec slices are
  `&'a [T]`, strings are `&'a str`, containers are cursors
  (`IndexedArrayReader`, `ListReader`, `HashMapReader`). Walking fields never
  allocates or copies.
- **`.into()` materializes.** `From<XReader<'a>> for X` (and
  `From<IndexedArrayReader<'a, T>> for Vec<T>`, `From<ListReader> for List`,
  …) builds the RAM struct and copies every value (Strings, Vecs, nested
  structs). Use readers on hot paths; materialize only at API boundaries or
  when the RAM type is genuinely needed.
- `MmapReader::read::<T>()` borrows the mapped payload (zero-copy).
  `BufReader::from_file` first copies the file into a `Vec<u8>`, then still
  serves zero-copy views over that buffer.

## Container cheat-sheet

| Field type | File layout | Reader view | Notes |
|---|---|---|---|
| Pod struct / primitive (`#[pod]` or auto) | fixed section, raw bytes | `&'a T` | must be padding-free |
| `String` | dynamic, raw UTF-8 | `&'a str` | `#[mmap(inline(N))]` → fixed, null-padded |
| `Vec<T>` pod (`#[pod]`/primitives) | `[count u64][T; count]` | `&'a [T]` | patchable (tail / reserved gap) |
| `Vec<T>` non-fixed | `[count][offsets][values]` (Indexed) | `IndexedArrayReader` | append needs reserve slots |
| `Vec<T>` `#[fixed]` | `[count][total_size][T; n]` | `FixedArrayReader` | fixed stride, not `#[mmap(scope)]` |
| `Vec<T>` `#[mmap(byref)]` | `[count][ByRefRange; n]`, data postponed | `ByRefArrayReader` | append/set needs reserve |
| `List<T>` | `[head][tail][count]` + linked nodes | `ListReader` | unbounded append anywhere |
| `Option<T>` | `[present u8][aligned data]` | `Option<&T>` / `Option<&str>` | `{0,0}` range = None |
| `#[mmap(byref)]` scalar | `ByRefRange` in fixed, data postponed | `&'a T` / `&'a str` | `Option<byref>`: `{0,0}` = None |
| `HashMap<K, V>` | sorted keys, binary search | `HashMapReader` | `#[mmap(key=pod, value=inlined)]` |

## Attribute cheat-sheet

- `#[pod]` — field is a bytemuck pod type (required for custom structs, incl.
  enums inside pod containers: the type itself needs `Pod, Zeroable` derives)
- `#[mmap(byref)]` / `#[mmap(byref, dedup)]` — value stored in the postponed
  tail, fixed section keeps an 8-byte `ByRefRange`; dedup merges equal values
- `#[mmap(inline(N))]` — String only; fixed N-byte null-padded slot
- `#[fixed]` — on `Vec<T>`: fixed-stride array layout
- `#[mmap(scope)]` — per-instance postponed data locality (incompatible with `#[fixed]`)
- `#[mmap(key = pod, value = inlined)]` — explicit HashMap layout choice
- `#[mmap(bool)]` — u32 fields only: getter/setter + schema Bool type

## Writing

- `write_to_file(value, path, write_crc)` — 24-byte header (magic, flags,
  layout_hash, crc64); whole-payload CRC-64-ECMA when `write_crc`
- `write_to_file_split(path, value, write_crc, embed_schema)` — headerless
  O_DIRECT-aligned payload + `<path>.mmshdr` sidecar
- `write_to_file_with_embedded_schema(value, write_crc)` — v2 header with the
  runtime schema embedded (what `mmapview`/TUI read without a registry)
- `*_with` variants (`write_to_file_with`, `write_split_with`,
  `write_to_file_with_embedded_schema_with`) take a `ReservePlan`:
  `Reserve { slots, bytes }` per field makes that container patchable in
  place later (see Patching)
- Durability is explicit: writes land in the page cache; patch sessions
  flush on demand

## Reading

- `MmapReader::from_file` / `from_file_with_sidecar` / `from_file_raw` — mmap
  backed; `read::<T>()` validates `layout_hash` (and CRC when present)
- `BufReader::from_file` / `BufReader::new(bytes)` — same checks over RAM
- Schema resolution order everywhere: embedded v2 header → registry
  (`~/.config/mmapstruct/schemas/<hash>.mms`) → `.mmshdr` sidecar
- Get data without RAM structs: `reader.fields...` (zero-copy); with the
  `dyn` feature: `GetSchema` + DSL queries (`mmapview q` skill)

## Patching

- `PatchSession::open_typed::<T>(path)` (+ `open_with_sidecar*`): validates
  header/`layout_hash`; **CRC-protected files are rejected** (whole-payload
  CRC cannot survive a partial patch)
- `offset_of(&reader.field)` → payload offset; `patch_pod::<T>(off, &v)` —
  scalars and `Vec<Pod>` elements
- Appends: `append_pod_vec` (tail without reserve / anywhere with reserve),
  `append_indexed` (reserve required: spare slots + gap), `append_byref_vec`
  (reserve), `append_list` (unbounded, any position), `set_byref` (standalone
  byref field, copy-on-write into the marked gap)
- Ordering caveat: unreserved **tail** appends must run BEFORE any mid-file
  list append (a mid-file list node lands at the payload end and ends the
  previous tail's grow-path)
- Every protocol is crash-consistent: readers always see the old or the new
  view; torn states self-heal on the next append
- `SchemaPatchSession` (mmapstruct-schema): the same operations addressed by
  field paths (`"events"`, `"inner.items"`) through the runtime schema — for
  schema-only contexts (HTTP/TUI/CLI) without compile-time types; element
  payloads are raw serialized bytes (`append_list_raw(path, &[u8])`, …)
- All ops work on struct roots and mixed-enum roots (per-variant layout); byref
  ops are struct-root only

## Pitfalls

- Pod structs must be padding-free (`derive(Pod)` fails on padding) — add
  explicit `_pad` fields; Rust `align_of` is mirrored by the format
- Enums in pod containers: `#[pod]` on the field + `Pod, Zeroable` derives on
  the enum; no padding inside variants
- Byref postponed data always trails the file → an unreserved `Vec<Pod>` in a
  file with any byref field can never tail-append (give it a reserve)
- `counts` grow monotonically in patches; crossing 2^32 is rejected (not
  crash-atomic)
- `inline(N)` only on String; `bool` only on u32; generic type parameters are
  not supported
- `List<T>` random access is O(n) — it is a sequential log container, not a
  Vec replacement

## New-format checklist

1. Define the RAM struct(s), choose containers (log/growth → `List<T>`;
   O(1) random access → `Vec<T>` + reserve)
2. `#[derive(MmapStruct)]` (+ `Pod/Zeroable` on pod-only types)
3. Write/read roundtrip test: `write_to_file` → `MmapReader` → compare
4. Patchable fields? add a `ReservePlan` at write time + a patch test
5. Verify from the outside: `mmapview describe` / `mmapview q` (see the
   `mmapview` skill), visual check via `mmapview-tui` skill
6. Full verification: `cargo test --workspace` + `cargo test -p mmapstruct
   --features dyn` + `cargo test -p mmapstruct --features strict-validation`

## Where to dig deeper

- `.agents/AGENTS.md` — the full reference: layout details, derive internals,
  DSL, patching section, known limitations
- `.agents/ideas/design/patching_format_decisions.md` — patching format
  rationale (crash-consistency protocols, self-describing reserves, enum
  roots, HDD I/O profile)
- Examples: `mmapstruct-schemagen/examples/list_demo.rs` (format + patch
  demo), `mmapview-tui` generate_* example generators