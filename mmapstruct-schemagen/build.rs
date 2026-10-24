use std::path::PathBuf;

fn registry_dir() -> PathBuf {

    std::env::var("MMAPSTRUCT_SCHEMA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home =
                std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".config/mmapstruct/schemas")
        })
}

fn save_schema(layout_hash: u64, schema_bytes: &[u8]) {
    let dir = registry_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{:016x}.mms", layout_hash));
    let _ = std::fs::write(path, schema_bytes);
}

fn main() {
    let schema = mmapstruct_schema::build_serialized_schema::<mmapstruct_schema::Schema>();
    let bytes = mmapstruct_schema::write_to_file_with_embedded_schema(&schema, false);
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let path = std::path::Path::new(&out_dir).join("self-schema.mms");
    std::fs::write(&path, &bytes).unwrap();

    // Also copy to target/debug/ or target/release/ for easy access
    let target_dir =
        std::path::Path::new(&out_dir).parent().and_then(|p| p.parent()).and_then(|p| p.parent());
    if let Some(td) = target_dir {
        let _ = std::fs::write(td.join("self-schema.mms"), &bytes);
    }

    // Register raw schema bytes (payload only) in registry for tree viewer
    let layout_hash = <mmapstruct_schema::Schema as mmapstruct::LayoutHash>::HASH;
    use mmapstruct::file_reader::parse_header;
    let (payload, _, _, _) = parse_header(&bytes);
    save_schema(layout_hash, payload);

    println!("cargo::rerun-if-changed=build.rs");
}
