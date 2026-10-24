use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use mmapstruct::MmapStruct;

#[derive(MmapStruct, Debug)]
struct HashMaps {
    // podK_podV: u64 — Pod, u64 — Pod
    pod_map: HashMap<u64, u64>,
    // podK_inlinedV: u64 — Pod, String — Inlined
    fixed_map: HashMap<u64, String>,
    // inlinedK_podV: String — Inlined, u64 — Pod
    pod_val_map: HashMap<String, u64>,
    // inlinedK_inlinedV: String — Inlined, String — Inlined
    inlined_map: HashMap<String, String>,
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
    let buckets = ["alpha", "beta", "gamma", "delta", "epsilon"];

    let pod_map: HashMap<u64, u64> = (0..count).map(|i| (i, i * 1000)).collect();

    let fixed_map: HashMap<u64, String> = (0..count)
        .map(|i| (i, buckets[i as usize % buckets.len()].to_string()))
        .collect();

    let pod_val_map: HashMap<String, u64> = (0..count)
        .map(|i| (format!("key_{i:05}"), i))
        .collect();

    let inlined_map: HashMap<String, String> = (0..count)
        .map(|i| (format!("key_{i:05}"), format!("value_{i:05}")))
        .collect();

    let root = HashMaps {
        pod_map,
        fixed_map,
        pod_val_map,
        inlined_map,
    };

    let buf = mmapstruct_schema::write_to_file_with_embedded_schema(&root, false);
    let path = PathBuf::from("/tmp/hash_maps.mms");
    fs::write(&path, &buf).expect("write file");

    let file_size = buf.len() as u64;
    let entries = count;

    println!("Written 4 maps × {entries} entries to {}", path.display());
    println!("bucket cycles through 5 short names.\n");
    println!("{:<30} {:>14}", "Total file size", format_bytes(file_size));
    println!("{:<30} {:>14}", "pod_map (u64→u64)", format_bytes(entries * 16));
    println!("{:<30} {:>14}", "fixed_map (u64→String)", format_bytes(entries * 16));
    println!("{:<30} {:>14}", "pod_val_map (String→u64)", format_bytes(entries * 16));
    println!("{:<30} {:>14}", "inlined_map (String→String)", format_bytes(entries * 16));
    println!("{:<30} {:>14}", "Overhead", format_bytes(file_size - entries * 4 * 16));
}
