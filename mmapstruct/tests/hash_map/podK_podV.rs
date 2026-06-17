use mmapstruct::hash_map::podK_podV::{write_hash_map, HashMapReader};
use mmapstruct::traits::FileRange;
use mmapstruct::WriteContext;
use std::collections::HashMap;

#[test]
fn roundtrip_pod_keys_vals() {
    let map = HashMap::from([(10u32, 100u64), (20u32, 200u64), (5u32, 50u64)]);

    let mut ctx = WriteContext::new();
    write_hash_map(&map, &mut ctx);
    let buf = ctx.finish();
    let range = FileRange {
        start: 0,
        end: buf.len(),
    };
    let reader = HashMapReader::<u32, u64>::new(&buf, range);

    assert_eq!(reader.len(), 3);
    assert_eq!(*reader.get(&5).unwrap(), 50);
    assert_eq!(*reader.get(&10).unwrap(), 100);
    assert_eq!(*reader.get(&20).unwrap(), 200);
    assert!(reader.get(&99).is_none());
}

#[test]
fn empty_map() {
    let map: HashMap<u32, u64> = HashMap::new();
    let mut ctx = WriteContext::new();
    write_hash_map(&map, &mut ctx);
    let buf = ctx.finish();
    let range = FileRange {
        start: 0,
        end: buf.len(),
    };
    let reader = HashMapReader::<u32, u64>::new(&buf, range);

    assert!(reader.is_empty());
    assert_eq!(reader.len(), 0);
}
