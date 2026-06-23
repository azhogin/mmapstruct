# HashMap Layouts

Four variants, depending on `key_is_pod` / `val_is_pod`.
Keys are always sorted; lookup uses binary search.

## podK_podV

```
Offset  Size           Field
------  -------------   --------------------------------------------
 0       8             count: u64 LE
 8       align(K)      padding to align_of::<K>()
 8+P     count × S_K   Keys: dense [K; count]
 8+P+S_K align(V)      padding to align_of::<V>()
 ...     count × S_V   Values: dense [V; count]
```

## podK_inlinedV

```
Offset  Size              Field
------  ----------------   --------------------------------------------
 0       8                count: u64 LE
 8       align(K)         padding to align_of::<K>()
 8+P     count × S_K      Keys: dense [K; count]
 8+P+S_K align(4)         padding to 4
 ...     (count+1) × 4    val offset table: [u32; count+1]
 ...     align(V)         padding to align_of::<V>()
 ...     ??               vals data (variable size)
```

## inlinedK_podV

```
Offset  Size              Field
------  ----------------   --------------------------------------------
 0       8                count: u64 LE
 8       (count+1) × 4    key offset table: [u32; count+1]
 8+4C    align(K)         padding to align_of::<K>()
 8+4C+P  ??               keys data (variable size)
 8+4C+P  + total_keys     align(V) padding to align_of::<V>()
 ...     count × S_V      Values: dense [V; count]
```

## inlinedK_inlinedV

```
Offset  Size                Field
------  ------------------   --------------------------------------------
 0       8                  count: u64 LE
 8       (count+1) × 4      key offset table: [u32; count+1]
 8+4C    (count+1) × 4      val offset table: [u32; count+1]
 8+8C    align(K)           padding to align_of::<K>()
 8+8C+P  ??                 keys data (variable size)
 ...     align(V)           padding to align_of::<V>()
 ...     ??                 vals data (variable size)
```

Where `C = count + 1`, `S_K = size_of::<K>()`, `S_V = size_of::<V>()`.

Offset table entry i is the offset from the start of the data area to element i. Entry `count` is the total data size. Offsets are `u32` (not u64).
