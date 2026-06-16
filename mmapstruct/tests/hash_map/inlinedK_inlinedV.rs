use mmapstruct::hash_map::inlinedK_inlinedV::{HashMapReader, write_hash_map};
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

#[derive(MmapStruct, Debug, PartialEq)]
struct Val {
    score: u32,
    #[mmap(byref)]
    tag: String,
}

#[test]
fn roundtrip_string_string() {
    let map = HashMap::from([
        ("gamma".to_string(), "GAMMA".to_string()),
        ("alpha".to_string(), "ALPHA".to_string()),
        ("beta".to_string(), "BETA".to_string()),
    ]);

    let mut ctx = WriteContext::new();
    write_hash_map(&map, &mut ctx);
    let buf = ctx.finish();
    let range = FileRange { start: 0, end: buf.len() };
    let reader = HashMapReader::<String, String>::new(&buf, range);

    assert_eq!(reader.len(), 3);
    assert_eq!(reader.get(&"alpha".to_string()).unwrap(), "ALPHA");
    assert_eq!(reader.get(&"beta".to_string()).unwrap(), "BETA");
    assert_eq!(reader.get(&"gamma".to_string()).unwrap(), "GAMMA");
    assert!(reader.get(&"delta".to_string()).is_none());
}

#[test]
fn empty_map() {
    let map: HashMap<String, String> = HashMap::new();
    let mut ctx = WriteContext::new();
    write_hash_map(&map, &mut ctx);
    let buf = ctx.finish();
    let range = FileRange { start: 0, end: buf.len() };
    let reader = HashMapReader::<String, String>::new(&buf, range);

    assert!(reader.is_empty());
    assert_eq!(reader.len(), 0);
}

#[test]
fn roundtrip_struct_key_string_val() {
    let map = HashMap::from([
        (Key { id: 2, name: "beta".into() }, "B".to_string()),
        (Key { id: 1, name: "alpha".into() }, "A".to_string()),
        (Key { id: 3, name: "gamma".into() }, "C".to_string()),
    ]);

    let mut ctx = WriteContext::new();
    write_hash_map(&map, &mut ctx);
    let buf = ctx.finish();
    let range = FileRange { start: 0, end: buf.len() };
    let reader = HashMapReader::<Key, String>::new(&buf, range);

    assert_eq!(reader.len(), 3);
    assert_eq!(reader.get(&Key { id: 1, name: "alpha".into() }).unwrap(), "A");
    assert_eq!(reader.get(&Key { id: 2, name: "beta".into() }).unwrap(), "B");
    assert_eq!(reader.get(&Key { id: 3, name: "gamma".into() }).unwrap(), "C");
}

#[test]
fn roundtrip_string_key_struct_val() {
    let map = HashMap::from([
        ("beta".to_string(), Val { score: 20, tag: "B-tag".into() }),
        ("alpha".to_string(), Val { score: 10, tag: "A-tag".into() }),
        ("gamma".to_string(), Val { score: 30, tag: "C-tag".into() }),
    ]);

    let mut ctx = WriteContext::new();
    write_hash_map(&map, &mut ctx);
    let buf = ctx.finish();
    let range = FileRange { start: 0, end: buf.len() };
    let reader = HashMapReader::<String, Val>::new(&buf, range);

    let v1 = reader.get(&"alpha".to_string()).unwrap();
    assert_eq!(*v1.score, 10);
    assert_eq!(v1.tag, "A-tag");

    let v2 = reader.get(&"beta".to_string()).unwrap();
    assert_eq!(*v2.score, 20);
    assert_eq!(v2.tag, "B-tag");
}

#[test]
fn iterate_keys_values() {
    let map = HashMap::from([
        ("c".to_string(), "3".to_string()),
        ("a".to_string(), "1".to_string()),
        ("b".to_string(), "2".to_string()),
    ]);

    let mut ctx = WriteContext::new();
    write_hash_map(&map, &mut ctx);
    let buf = ctx.finish();
    let range = FileRange { start: 0, end: buf.len() };
    let reader = HashMapReader::<String, String>::new(&buf, range);

    let keys: Vec<String> = reader.keys().map(|s| s.to_string()).collect();
    assert_eq!(keys, vec!["a", "b", "c"]);

    let vals: Vec<String> = reader.values().map(|s| s.to_string()).collect();
    assert_eq!(vals, vec!["1", "2", "3"]);

    let pairs: Vec<(String, String)> = reader.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    assert_eq!(pairs, vec![
        ("a".to_string(), "1".to_string()),
        ("b".to_string(), "2".to_string()),
        ("c".to_string(), "3".to_string()),
    ]);
}
