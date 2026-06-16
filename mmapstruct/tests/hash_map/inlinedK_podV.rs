use mmapstruct::hash_map::inlinedK_podV::{HashMapReader, write_hash_map};
use mmapstruct::traits::FileRange;
use mmapstruct::WriteContext;
use mmapstruct::MmapStruct;
use std::collections::HashMap;

#[derive(MmapStruct, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Key {
    id: u32,
    #[mmap(byref)]
    name: String,
}

#[test]
fn roundtrip_struct_keys() {
    let map = HashMap::from([
        (Key { id: 2, name: "beta".into() }, 200u64),
        (Key { id: 1, name: "alpha".into() }, 100u64),
        (Key { id: 3, name: "gamma".into() }, 300u64),
    ]);

    let mut ctx = WriteContext::new();
    write_hash_map(&map, &mut ctx);
    let buf = ctx.finish();
    let range = FileRange { start: 0, end: buf.len() };
    let reader = HashMapReader::<Key, u64>::new(&buf, range);

    assert_eq!(reader.len(), 3);

    assert_eq!(*reader.get(&Key { id: 1, name: "alpha".into() }).unwrap(), 100);
    assert_eq!(*reader.get(&Key { id: 2, name: "beta".into() }).unwrap(), 200);
    assert_eq!(*reader.get(&Key { id: 3, name: "gamma".into() }).unwrap(), 300);

    assert!(reader.get(&Key { id: 4, name: "delta".into() }).is_none());
}

#[derive(MmapStruct, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct SimpleKey {
    value: u32,
    #[mmap(inline(16))]
    tag: String,
}

#[test]
fn roundtrip_inline_string_key() {
    let map = HashMap::from([
        (SimpleKey { value: 30, tag: "c".into() }, 300u64),
        (SimpleKey { value: 10, tag: "a".into() }, 100u64),
        (SimpleKey { value: 20, tag: "b".into() }, 200u64),
    ]);

    let mut ctx = WriteContext::new();
    write_hash_map(&map, &mut ctx);
    let buf = ctx.finish();
    let range = FileRange { start: 0, end: buf.len() };
    let reader = HashMapReader::<SimpleKey, u64>::new(&buf, range);

    assert_eq!(reader.len(), 3);

    let v1 = reader.get(&SimpleKey { value: 10, tag: "a".into() }).unwrap();
    assert_eq!(*v1, 100);

    let v2 = reader.get(&SimpleKey { value: 20, tag: "b".into() }).unwrap();
    assert_eq!(*v2, 200);

    assert!(reader.get(&SimpleKey { value: 99, tag: "z".into() }).is_none());
}

#[test]
fn iterate_keys_string() {
    let map = HashMap::from([
        ("gamma".to_string(), 30u32),
        ("alpha".to_string(), 10u32),
        ("beta".to_string(), 20u32),
    ]);

    let mut ctx = WriteContext::new();
    write_hash_map(&map, &mut ctx);
    let buf = ctx.finish();
    let range = FileRange { start: 0, end: buf.len() };
    let reader = HashMapReader::<String, u32>::new(&buf, range);

    let collected: Vec<String> = reader.keys().map(|s| s.to_string()).collect();
    assert_eq!(collected, vec!["alpha", "beta", "gamma"]);
}
