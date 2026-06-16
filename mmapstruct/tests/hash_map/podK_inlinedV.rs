use mmapstruct::hash_map::podK_inlinedV::{HashMapReader, write_hash_map};
use mmapstruct::traits::FileRange;
use mmapstruct::WriteContext;
use mmapstruct::MmapStruct;

#[derive(MmapStruct, Debug, PartialEq)]
struct Item {
    id: u32,
    #[mmap(byref)]
    label: String,
}

#[derive(MmapStruct, Debug, PartialEq)]
struct ItemDedup {
    id: u32,
    #[mmap(byref, dedup)]
    label: String,
}

#[derive(MmapStruct, Debug, PartialEq)]
struct OnlyString {
    #[mmap(byref, dedup)]
    label: String,
}

#[derive(MmapStruct, Debug, PartialEq)]
struct User {
    id: u64,
    score: u32,
    #[mmap(byref)]
    name: String,
}

#[derive(MmapStruct, Debug, PartialEq)]
struct MultiByref {
    #[mmap(byref, dedup)]
    first: String,
    #[mmap(byref)]
    last: String,
}

#[test]
fn roundtrip_with_byref_string() {
    use std::collections::HashMap;

    let map = HashMap::from([
        (1u32, Item { id: 10, label: "foo".into() }),
        (2u32, Item { id: 20, label: "bar".into() }),
    ]);

    let mut ctx = WriteContext::new();
    write_hash_map(&map, &mut ctx);
    let buf = ctx.finish();
    let range = FileRange { start: 0, end: buf.len() };
    let reader = HashMapReader::<u32, Item>::new(&buf, range);

    assert_eq!(reader.len(), 2);

    let v1 = reader.get(&1).unwrap();
    assert_eq!(*v1.id, 10);
    assert_eq!(v1.label, "foo");

    let v2 = reader.get(&2).unwrap();
    assert_eq!(*v2.id, 20);
    assert_eq!(v2.label, "bar");

    assert!(reader.get(&99).is_none());
}

#[test]
fn roundtrip_with_dedup_string() {
    use std::collections::HashMap;

    let map = HashMap::from([
        (1u32, ItemDedup { id: 10, label: "hello".into() }),
        (2u32, ItemDedup { id: 20, label: "hello".into() }),
        (3u32, ItemDedup { id: 30, label: "world".into() }),
    ]);

    let mut ctx = WriteContext::new();
    write_hash_map(&map, &mut ctx);
    let buf = ctx.finish();
    let range = FileRange { start: 0, end: buf.len() };
    let reader = HashMapReader::<u32, ItemDedup>::new(&buf, range);

    assert_eq!(reader.len(), 3);

    let v1 = reader.get(&1).unwrap();
    assert_eq!(*v1.id, 10);
    assert_eq!(v1.label, "hello");

    let v2 = reader.get(&2).unwrap();
    assert_eq!(*v2.id, 20);
    assert_eq!(v2.label, "hello");

    let v3 = reader.get(&3).unwrap();
    assert_eq!(*v3.id, 30);
    assert_eq!(v3.label, "world");
}

#[test]
fn roundtrip_only_dedup_string() {
    use std::collections::HashMap;

    let map = HashMap::from([
        (1u32, OnlyString { label: "x".into() }),
        (2u32, OnlyString { label: "x".into() }),
        (3u32, OnlyString { label: "y".into() }),
    ]);

    let mut ctx = WriteContext::new();
    write_hash_map(&map, &mut ctx);
    let buf = ctx.finish();
    let range = FileRange { start: 0, end: buf.len() };
    let reader = HashMapReader::<u32, OnlyString>::new(&buf, range);

    assert_eq!(reader.len(), 3);

    assert_eq!(reader.get(&1).unwrap().label, "x");
    assert_eq!(reader.get(&2).unwrap().label, "x");
    assert_eq!(reader.get(&3).unwrap().label, "y");
}

#[test]
fn roundtrip_custom_struct() {
    use std::collections::HashMap;

    let map = HashMap::from([
        (1u32, User { id: 100, score: 10, name: "alice".into() }),
        (2u32, User { id: 200, score: 20, name: "bob".into() }),
    ]);

    let mut ctx = WriteContext::new();
    write_hash_map(&map, &mut ctx);
    let buf = ctx.finish();
    let range = FileRange { start: 0, end: buf.len() };
    let reader = HashMapReader::<u32, User>::new(&buf, range);

    let v1 = reader.get(&1).unwrap();
    assert_eq!(*v1.id, 100);
    assert_eq!(*v1.score, 10);
    assert_eq!(v1.name, "alice");

    let v2 = reader.get(&2).unwrap();
    assert_eq!(*v2.id, 200);
    assert_eq!(*v2.score, 20);
    assert_eq!(v2.name, "bob");
}

#[test]
fn roundtrip_multi_byref() {
    use std::collections::HashMap;

    let map = HashMap::from([
        (1u32, MultiByref { first: "John".into(), last: "Doe".into() }),
        (2u32, MultiByref { first: "John".into(), last: "Smith".into() }),
    ]);

    let mut ctx = WriteContext::new();
    write_hash_map(&map, &mut ctx);
    let buf = ctx.finish();
    let range = FileRange { start: 0, end: buf.len() };
    let reader = HashMapReader::<u32, MultiByref>::new(&buf, range);

    let v1 = reader.get(&1).unwrap();
    assert_eq!(v1.first, "John");
    assert_eq!(v1.last, "Doe");

    let v2 = reader.get(&2).unwrap();
    assert_eq!(v2.first, "John");
    assert_eq!(v2.last, "Smith");
}
