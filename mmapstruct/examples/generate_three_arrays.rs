use bytemuck::{Pod, Zeroable};
use std::fs;
use std::path::PathBuf;

use mmapstruct::MmapStruct;

// ── Pod element ──

#[derive(MmapStruct, Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C, packed)]
struct PodPoint {
    id: u64,
    x: f32,
    y: f32,
    z: f32,
}

// ── Fixed element (only pod + byref, no inline-dynamic) ──

#[derive(MmapStruct, Debug)]
struct FixedRecord {
    id: u64,
    #[mmap(byref, dedup)]
    bucket: String,
}

// ── Indexed element (adds Vec<u8> to force indexed layout) ──

#[derive(MmapStruct, Debug)]
struct IndexedRecord {
    id: u64,
    #[mmap(byref, dedup)]
    bucket: String,
    extra: Vec<u8>,
}

// ── Root struct with all three arrays ──

#[derive(MmapStruct, Debug)]
struct ThreeArrays {
    #[pod]
    pod_points: Vec<PodPoint>,
    #[fixed]
    fixed_items: Vec<FixedRecord>,
    indexed_items: Vec<IndexedRecord>,
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
    let count = 1_000_000;
    let buckets = ["alpha", "beta", "gamma", "delta", "epsilon"];

    let pod_points: Vec<PodPoint> = (0..count)
        .map(|i| {
            let t = i as f32 * 0.001;
            PodPoint {
                id: i as u64,
                x: (t * 2.0).sin(),
                y: (t * 3.0).cos(),
                z: (t * 5.0).sin() * (t * 7.0).cos(),
            }
        })
        .collect();

    let fixed_items: Vec<FixedRecord> = (0..count)
        .map(|i| FixedRecord {
            id: i as u64,
            bucket: buckets[i % buckets.len()].to_string(),
        })
        .collect();

    let indexed_items: Vec<IndexedRecord> = (0..count)
        .map(|i| {
            let extra_len = i % 100;
            IndexedRecord {
                id: i as u64,
                bucket: buckets[i % buckets.len()].to_string(),
                extra: (0..extra_len as u8).collect(),
            }
        })
        .collect();

    let root = ThreeArrays {
        pod_points,
        fixed_items,
        indexed_items,
    };

    let buf = mmapstruct_schema::write_to_file_with_embedded_schema(&root, false);
    let path = PathBuf::from("/tmp/three_arrays.mms");
    fs::write(&path, &buf).expect("write file");

    let file_size = buf.len() as u64;

    // Expected payload bytes (element fixed-section sizes, no overhead)
    let pod_payload = count as u64 * size_of::<PodPoint>() as u64; // 20
    let fixed_payload = count as u64 * 16; // id(8) + ByRefRange(8)
                                           // IndexedRecord fixed section = 16 B, but each has avg 49.5 B of extra + 8 B offset table + ...
    let indexed_fixed = count as u64 * 16;

    let overhead = file_size - pod_payload - fixed_payload - indexed_fixed;

    println!("Written 3 arrays × {count} elements to {}", path.display());
    println!("bucket cycles through 5 short names → byref+dedup stores each only once.\n");

    println!("{:<30} {:>14}", "Total file size", format_bytes(file_size));
    println!(
        "{:<30} {:>14}",
        "Expected raw payload (all fixed sections)",
        format_bytes(pod_payload + fixed_payload + indexed_fixed)
    );
    println!(
        "{:<30} {:>14}",
        "Overhead (schema + framing + postponed + extra)",
        format_bytes(overhead)
    );
    println!();
    println!("Section payload estimates:");
    println!(
        "  {:12}  {:>12} ({:.0} B/elem)",
        "pod:",
        format_bytes(pod_payload),
        size_of::<PodPoint>()
    );
    println!(
        "  {:12}  {:>12} (16 B/elem + dedup strings)",
        "fixed:",
        format_bytes(fixed_payload)
    );
    println!(
        "  {:12}  {:>12} (16 B/elem + dedup strings + avg 49.5 B extra + offset table)",
        "indexed:",
        format_bytes(indexed_fixed)
    );
}
