use std::fs;
use std::path::PathBuf;
use bytemuck::{Pod, Zeroable};

use mmapstruct::MmapStruct;

// ── Pod tuple element ──

#[derive(MmapStruct, Clone, Copy, Debug, Default, Pod, Zeroable)]
#[repr(C)]
struct Rgb(u8, u8, u8);

// ── Fixed tuple element (only pod + byref, no inline-dynamic) ──

#[derive(MmapStruct, Debug)]
struct LabelledPoint(u32, u32, #[mmap(byref, dedup)] String);

// ── Indexed tuple element (has inline-dynamic fields) ──

#[derive(MmapStruct, Debug)]
struct TaggedPoint(u32, String, Vec<u8>);

// ── Root struct with all three arrays ──

#[derive(MmapStruct, Debug)]
struct TupleArrays {
    #[pod]
    colors: Vec<Rgb>,
    #[fixed]
    labels: Vec<LabelledPoint>,
    points: Vec<TaggedPoint>,
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

    let colors: Vec<Rgb> = (0..count)
        .map(|i| Rgb((i % 256) as u8, ((i * 3) % 256) as u8, ((i * 7) % 256) as u8))
        .collect();

    let labels: Vec<LabelledPoint> = (0..count)
        .map(|i| LabelledPoint(i as u32, (i * 2) as u32, buckets[i % buckets.len()].to_string()))
        .collect();

    let points: Vec<TaggedPoint> = (0..count)
        .map(|i| {
            let extra_len = i % 100;
            TaggedPoint(
                i as u32,
                buckets[i % buckets.len()].to_string(),
                (0..extra_len as u8).collect(),
            )
        })
        .collect();

    let root = TupleArrays {
        colors,
        labels,
        points,
    };

    let buf = mmapstruct_schema::write_to_file_with_embedded_schema(&root, false);
    let path = PathBuf::from("/tmp/tuple_arrays.mms");
    fs::write(&path, &buf).expect("write file");

    let file_size = buf.len() as u64;

    // Expected payload bytes (element fixed-section sizes, no framing/overhead)
    let colors_payload = count as u64 * 3;           // 3 bytes per Rgb
    let labels_payload = count as u64 * 16;          // u32 + u32 + ByRefRange(8) = 16
    let points_fixed = count as u64 * 4;             // u32 only in fixed section

    let overhead = file_size - colors_payload - labels_payload - points_fixed;

    println!("Written 3 tuple arrays × {count} elements to {}", path.display());
    println!("bucket cycles through 5 short names → byref+dedup stores each only once.\n");

    println!("{:<30} {:>14}", "Total file size", format_bytes(file_size));
    println!("{:<30} {:>14}", "Expected raw payload (all fixed sections)", format_bytes(colors_payload + labels_payload + points_fixed));
    println!("{:<30} {:>14}", "Overhead (schema + framing + postponed + extra)", format_bytes(overhead));
    println!();
    println!("Section payload estimates:");
    println!("  {:12}  {:>12} (3 B/elem)", "pod:", format_bytes(colors_payload));
    println!("  {:12}  {:>12} (16 B/elem + dedup strings)", "fixed:", format_bytes(labels_payload));
    println!("  {:12}  {:>12} (4 B/elem + avg 49.5 B extra + offset table)",
        "indexed:", format_bytes(points_fixed));
}
