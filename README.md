# mmapstruct

Memory-mapped binary serialization for Rust. Define your file format as a Rust struct, derive `MmapStruct`, and get zero-copy readers + efficient writers.

```rust
use mmapstruct::MmapStruct;

#[derive(MmapStruct)]
struct MyFile {
    id: u32,
    name: String,
    values: Vec<u32>,
}

// Write
let w = MyFile { id: 42, name: "hello".into(), values: vec![1, 2, 3] };
let buf = mmapstruct::write_to_buf(&w, false);

// Read — returns a Reader with &'a references into the mmap'ed bytes
let reader = mmapstruct::BufReader::new(buf).read::<MyFile>();
assert_eq!(*reader.id, 42);
assert_eq!(reader.name, "hello");
assert_eq!(reader.values, &[1u32, 2, 3]);
```

No serde, no deserialization — the reader is a projection over `&[u8]` via `bytemuck`.

## Crates

| Crate | Description |
|-------|-------------|
| `mmapstruct` | Core library: traits, readers, writers, schema system |
| `mmapstruct-derive` | Proc-macro: `#[derive(MmapStruct)]` for structs and enums |
| `mmapview` | Zero-copy DSL query language (`/items[?id>10] \| avg(.price)`) |
| `mmapview-http` | Axum web UI for browsing mmapstruct files in a browser |

## Field types

| Type | Layout | Reader |
|------|--------|--------|
| Pod primitives (`u32`, `f32`, …) | Dense, fixed offset | `&'a T` |
| `String` | Raw bytes (no length prefix) | `&'a str` |
| `#[mmap(inline(N))]` `String` | Null-padded, N bytes in fixed section | `&'a str` |
| `Vec<T>` | `[count: u64, offsets: [u64; count], values...]` | `IndexedArrayReader<'a, T>` |
| `#[pod]` `Vec<T>` | Dense `[T; count]` | `&'a [T]` |
| `#[fixed]` `Vec<T>` | Fixed stride `[count, total_size, elements...]` | `FixedArrayReader<'a, T>` |
| `#[mmap(byref)]` `Vec<T>` | ByRef ranges in postponed section | `ByRefArrayReader<'a, T>` |
| `Option<T>` | `[present: u8, data...]` | `Option<T::Reader<'a>>` |
| `#[mmap(byref)]` field | `ByRefRange` placeholder → postponed data | `&'a T` / `&'a str` |
| `#[mmap(byref)] Option<T>` | `ByRefRange{0,0}` = None sentinel | `Option<&'a T>` |
| `HashMap<K, V>` | Sorted keys, binary search. 4 variants | `HashMapReader<'a, K, V>` |

### HashMap auto-detection

The derive macro selects the optimal layout at compile time:

| Keys | Values | Layout |
|------|--------|--------|
| Pod | Pod | `[count, keys…, vals…]` |
| Pod | Inlined | `[count, keys…, offset_table, vals_data…]` |
| Inlined | Pod | `[count, offset_table, keys_data…, vals…]` |
| Inlined | Inlined | `[count, 2×offset_table, keys_data…, vals_data…]` |

Override with `#[mmap(key = pod, value = inlined)]` on the field.

## Struct layout

```
[fixed section] — pod fields, inline strings, ByRefRange placeholders
[offset table]  — u64 offsets for dynamic fields
[inline values] — strings, vecs, options, custom structs
[postponed]     — byref data (or per-scope if #[mmap(scope)])
```

## File header (24 bytes)

```
magic: u32 = "MMAP" | hdr_ver: u16 = 1 | flags: u16 | layout_hash: u64 | crc64: u64
```

- Layout hash: compile-time structural fingerprint, verified at read time
- CRC-64-ECMA: optional integrity check (`write_to_buf(value, true)`)
- v0 (8-byte) headers auto-detected for backward compatibility

## DSL query language (`mmapview`)

```
# Field access
/name
/items/records

# Indexing and slicing
/items[0]
/items[0:10]

# Filters
/items[?price>100]
/items[?name~sensor]           # contains
/items[?name^pre]              # starts with
/items[?name$suf]              # ends with
/items[?x>1.0&y<5.0]           # AND
/items[?tag="urgent"\|?tag="high"] # OR

# Aggregations
/items \| count()
/items \| avg(.price)
/items \| min(.price)
/items \| max(.price)

# Projections
/items \| {id, name}
/items \| select(.price>10)
```

## Feature flags

| Flag | Description |
|------|-------------|
| `dyn` | Schema system: `DynContext`, `Value`, `GetSchema`, HashMap DSL access |
| `safe-strings` | Validate UTF-8 instead of `from_utf8_unchecked` |
| `strict-validation` | Assert all bounds with descriptive messages (implies `safe-strings`) |

## What's not implemented

- Generic type parameters in `#[derive(MmapStruct)]`
- Recursive types (`Vec<Self>`) — needs `OnceLock` in schema
- Error handling — currently panics on corrupt data (use `strict-validation` for asserts)
- Diff utility (`src/diff.rs` — stub)
- `ReaderOrd` for complex fields (`Vec`, `HashMap`, `Option<complex>`)

## Tests

```
> cargo test

mmapstruct:    89 tests
mmapview:      65 tests (tokenizer + parser + interpreter + format_value)
mmapview-http:  8 tests (7 integration + 1 doctest)
Total:        162 tests
```
