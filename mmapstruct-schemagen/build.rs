fn main() {
    let schema = mmapstruct_schema::build_serialized_schema::<mmapstruct_schema::Schema>();
    let bytes = mmapstruct::write_to_buf_raw(&schema);
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let path = std::path::Path::new(&out_dir).join("self-schema.mms");
    std::fs::write(&path, &bytes).unwrap();

    // Also copy to target/debug/ or target/release/ for easy access
    let target_dir = std::path::Path::new(&out_dir)
        .parent().and_then(|p| p.parent())
        .and_then(|p| p.parent());
    if let Some(td) = target_dir {
        let _ = std::fs::write(td.join("self-schema.mms"), &bytes);
    }

    println!("cargo::rerun-if-changed=build.rs");
}
