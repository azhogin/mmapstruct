# Vec Layouts

Four variants of `Vec<T>`, selected automatically by the derive or via attributes.

## Pod Vec (`#[mmap(pod)]` on Vec, or T is a primitive/pod)

Dense array with no metadata.

```
         S = size_of::<T>()
[N × S]  Contiguous T elements. No alignment between elements.
```

Read: `&'a [T]` via `bytemuck::cast_slice`.

## Fixed Vec (`#[fixed]` on Vec<T>)

For fixed-size T (pod + byref, no inline-dynamic).

```
Offset  Size       Field
------  --------   --------------------------------------------
 0       8         count: u64 LE
 8       8         total_data_size: u64 LE
16       N × S     Elements, S = total_data_size / count
```

Read: `FixedArrayReader`, stride = total / count, each element at `base + i × stride`.

## Indexed Vec (default for T with dynamic fields)

```
Offset  Size         Field
------  ----------   --------------------------------------------
 0       8           count: u64 LE
 8       N × 8       offset table: [u64; N] — offsets from values_start
 8+N×8   ??          values area: element data, padded to 8
```

Sentinel offset `offset[N]` = total data size.

Read: `IndexedArrayReader`, element i at `values_start + offset[i] .. values_start + offset[i+1]`.

## ByRef Vec (`#[mmap(byref)]` on Vec<T>)

Elements live in the postponed section; inline data stores `ByRefRange` placeholders.

```
Offset  Size          Field
------  -----------   --------------------------------------------
 0       8            count: u64 LE
 8       8            total_size: u64 LE (= N × 8)
16       N × 8        [ByRefRange; N] — { begin: u32, end: u32 }
...                   postponed data (begin/end relative to struct base)
```

Read: `ByRefArrayReader`, each `ByRefRange` resolved as `base + begin .. base + end`.
