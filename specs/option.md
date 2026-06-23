# Option Layout

```
         T: align_of<T>()
         │<──────>│
None:    [0x00]
Some:    [0x01] [padding to align(T)] [T data]
         1       align(T)-1             size_of(T)
```

None — 1 byte (`0x00`), no data.
Some — 1 byte (`0x01`) + padding to T's alignment + T data.
