use std::fs;
use std::path::PathBuf;

use mmapstruct::MmapStruct;

#[derive(MmapStruct, Debug)]
struct FieldTypes {
    #[mmap(inline(32))]
    code: String,
    maybe_count: Option<u32>,
    #[mmap(byref)]
    maybe_label: Option<String>,
    #[mmap(bool)]
    enabled: u32,
    tags: Vec<String>,
}

#[derive(MmapStruct, Debug)]
struct FieldTypesFile {
    items: Vec<FieldTypes>,
}

fn format_bytes(n: u64) -> String {
    if n >= 1_048_576 {
        format!("{:.2} MiB", n as f64 / 1_048_576.0)
    } else if n >= 1024 {
        format!("{:.2} KiB", n as f64 / 1024.0)
    } else {
        format!("{n} B")
    }
}

fn main() {
    let count = 100_000;

    let items: Vec<FieldTypes> = (0..count)
        .map(|i| {
            let has_count = i % 3 != 0;
            let has_label = i % 5 != 0;
            let tag_count = (i % 10) as usize;
            FieldTypes {
                code: format!("CODE_{:04}", i % 10000),
                maybe_count: if has_count { Some(i as u32) } else { None },
                maybe_label: if has_label {
                    Some(format!(
                        "v{}.{}.{}",
                        (i / 1000) % 10,
                        (i / 100) % 10,
                        i % 100
                    ))
                } else {
                    None
                },
                enabled: if i % 2 == 0 { 1 } else { 0 },
                tags: (0..tag_count).map(|j| format!("tag_{}", j)).collect(),
            }
        })
        .collect();

    // Sample entries before moving
    let samples: Vec<_> = [0, 1, 2, 5, 10]
        .iter()
        .map(|&i| {
            format!(
                "  [{i:>3}] code={:?} maybe_count={:?} maybe_label={:?} enabled={} tags=[{}]",
                items[i].code,
                items[i].maybe_count,
                items[i].maybe_label,
                items[i].enabled(),
                items[i]
                    .tags
                    .iter()
                    .map(|s| format!("\"{}\"", s))
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        })
        .collect();

    let root = FieldTypesFile { items };
    let buf = mmapstruct_schema::write_to_file_with_embedded_schema(&root, false);
    let path = PathBuf::from("/tmp/field_types.mms");
    fs::write(&path, &buf).expect("write file");

    let file_size = buf.len() as u64;

    // Fixed section per element: inline(32) String (32 bytes) + Option<u32> (4)
    // + Option<byref String> (ByRefRange = 8 bytes) + u32 (4)
    // Total fixed: 32 + 4 + 8 + 4 = 48
    let fixed_per = 48u64;
    let fixed_total = count as u64 * fixed_per;

    // Each element has an inline-dyn offset table: elements with Vec<String> add
    // (inline_dyn_count + 1) * 8 = 3 * 8 = 24 bytes header per element
    let dyn_header_total = count as u64 * 24;

    println!("Written {count} FieldTypes entries to {}", path.display());
    println!();
    println!("{:<30} {:>14}", "Total file size", format_bytes(file_size));
    println!(
        "{:<30} {:>14}",
        "Expected fixed payload",
        format_bytes(fixed_total)
    );
    println!(
        "{:<30} {:>14}",
        "Expected dyn header overhead",
        format_bytes(dyn_header_total)
    );
    println!();
    println!("{:<30} {:>8}", "field", "bytes");
    println!("{:<30} {:>8}", "code [mmap(inline(32))]", "32");
    println!("{:<30} {:>8}", "maybe_count (Option<u32>)", "4");
    println!("{:<30} {:>8}", "maybe_label (Option<byref String>)", "8");
    println!("{:<30} {:>8}", "enabled [mmap(bool)] (u32)", "4");
    println!("{:<30} {:>8}", "tags (Vec<String>)", "dyn");
    println!();
    for s in &samples {
        println!("{s}");
    }
}
