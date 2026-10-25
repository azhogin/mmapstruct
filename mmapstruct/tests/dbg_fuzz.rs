#![cfg(feature = "fuzzing")]
use mmapstruct::file_reader::BufReader;
use mmapstruct::fuzzing::FuzzFile;

#[test]
fn dbg_fuzz_read() {
    // empty payload
    let br = BufReader::new(Vec::new());
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let reader = br.read::<FuzzFile>();
        let _ = reader.id;
    }));
    eprintln!("empty payload: {:?}", r.is_err());
    // also the fuzzer artifact
    let mut seeds: Vec<(String, Vec<u8>)> = std::fs::read_dir("../fuzz/seeds/typed_read")
        .unwrap()
        .flatten()
        .map(|e| {
            (
                e.file_name().to_string_lossy().to_string(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    if let Ok(entries) = std::fs::read_dir("../fuzz/artifacts/typed_read") {
        for e in entries.flatten() {
            seeds.push((
                format!("artifact-{}", e.file_name().to_string_lossy()),
                std::fs::read(e.path()).unwrap_or_default(),
            ));
        }
    }
    seeds.sort();
    for (name, data) in seeds {
        let br = BufReader::new(data);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let reader = br.read::<FuzzFile>();
            let _ = reader.id;
        }));
        eprintln!("seed {:?}: aborted={}", name, r.is_err());
    }
}
