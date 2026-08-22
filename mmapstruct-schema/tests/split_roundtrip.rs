use bytemuck::Pod;
use mmapstruct::{MmapReader, MmapStruct};
use mmapstruct_schema::{write_split, write_to_file_split};
use std::fs;

#[derive(MmapStruct, Debug, PartialEq)]
#[repr(C)]
struct Rec {
    a: u32,
    b: f32,
    #[pod]
    c: Vec<[u8; 8]>,
    d: Vec<f32>,
}

fn tmp(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(name);
    let _ = fs::remove_file(&p);
    let _ = fs::remove_file(p.with_extension("mmshdr"));
    p
}

#[test]
fn split_roundtrip_embedded() {
    let rec = Rec {
        a: 7,
        b: 1.5,
        c: vec![[1u8, 2, 3, 4, 5, 6, 7, 8]; 4],
        d: vec![1.0, 2.0, 3.0],
    };
    let data_path = tmp("split_rec_embed.mms");
    write_to_file_split(&data_path, &rec, true, true).unwrap();
    assert!(data_path.exists());
    assert!(data_path.with_extension("mmshdr").exists());

    // headerless data file must NOT begin with the mmapstruct magic
    let raw = fs::read(&data_path).unwrap();
    assert!(raw.len() > 0);
    assert_ne!(&raw[0..4], b"MMAP");
    // sidecar must begin with magic and carry an embedded schema
    let sb = fs::read(data_path.with_extension("mmshdr")).unwrap();
    assert_eq!(&sb[0..4], b"MMAP");
    let (_p, schema, hash, _crc) = mmapstruct::file_reader::parse_header(&sb);
    assert!(schema.is_some(), "sidecar should embed schema");
    assert!(hash.is_some());

    // read via sidecar (MmapReader)
    let r = MmapReader::from_file_with_sidecar(&data_path).unwrap();
    let got: Rec = r.read::<Rec>().into();
    assert_eq!(got, rec);

    // reassemble sidecar + data => a valid standalone embedded file
    let mut combined = sb.clone();
    combined.extend_from_slice(&raw);
    let combined_path = tmp("split_rec_combined.mms");
    fs::write(&combined_path, &combined).unwrap();
    let r2 = MmapReader::from_file(&combined_path).unwrap();
    let got2: Rec = r2.read::<Rec>().into();
    assert_eq!(got2, rec);
}

#[test]
fn split_roundtrip_hash_only() {
    let rec = Rec {
        a: 9,
        b: -2.0,
        c: vec![[9u8; 8]; 2],
        d: vec![4.0, 5.0],
    };
    let (data, sidecar) = write_split(&rec, false);
    // data is pure payload, no magic
    assert_ne!(&data[0..4], b"MMAP");
    // sidecar carries hash but no embedded schema
    let (_p, schema, hash, _crc) = mmapstruct::file_reader::parse_header(&sidecar);
    assert!(schema.is_none(), "hash-only sidecar must not embed schema");
    assert!(hash.is_some());

    // typed read works via sidecar (no schema needed for typed read)
    let data_path = tmp("split_rec_hash.mms");
    fs::write(&data_path, &data).unwrap();
    fs::write(data_path.with_extension("mmshdr"), &sidecar).unwrap();
    let r = MmapReader::from_file_with_sidecar(&data_path).unwrap();
    let got: Rec = r.read::<Rec>().into();
    assert_eq!(got, rec);
}
