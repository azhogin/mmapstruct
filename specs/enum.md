# Enum Layout

## Pod-enum (`#[repr(Int)]` with unit variants)

Behaves as `MmapPod` — a single integer of the repr type's size.

```
size_of::<repr_type>() — one integer (discriminant)
```

## Enum with data-carrying variants

Each variant follows a layout analogous to a struct (fixed section + offset table + inline values), positioned after the discriminant.

```
Offset  Size                   Field
------  ---------------------   -------------------------------------------------------
 0       4                     discriminant: u32 LE (0, 1, 2…)
 4       align(4, max(8,          padding to variant_data_align
                  max_field_align))
 4+P                            variant data (layout like struct — fixed + offset table + inline)
```

`variant_data_align = max(8, max_field_align)` — data always starts at a minimum offset of 8.

## Unit variant (no fields)

```
Offset  Size  Field
------  ----  -------------------------------------------------------
 0       4    discriminant: u32 LE
 4       4    padding to 8 (always 4 bytes)
```
