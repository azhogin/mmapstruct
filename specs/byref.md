# ByRef Layout (`#[mmap(byref)]`)

The struct's fixed section stores a `ByRefRange`; actual data lives in the postponed section.

## ByRefRange (8 bytes)

```
Offset  Size  Field   Description
------  ----  ------  ---------------------------------------------------
 0       4    begin   u32 LE — offset from struct base to data start
 4       4    end     u32 LE — offset from struct base to data end
```

Data: `base + begin .. base + end`.

## Option with byref

`#[mmap(byref)]` on `Option<String>` / `Option<T>`: `ByRefRange { 0, 0 }` = None sentinel.
