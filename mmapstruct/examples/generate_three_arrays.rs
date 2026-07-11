use std::fs;
use std::path::PathBuf;
use bytemuck::{Pod, Zeroable};

use mmapstruct::MmapStruct;

// ── Pod element: pure scalars ──

#[derive(MmapStruct, Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C, packed)]
struct PodPoint {
    id: u64,
    x: f32,
    y: f32,
    z: f32,
}

#[derive(MmapStruct, Debug)]
struct PodSection {
    #[pod]
    points: Vec<PodPoint>,
}

// ── Fixed element: pod + byref string (no inline-dynamic fields) ──

#[derive(MmapStruct, Debug)]
struct FixedRecord {
    id: u64,
    #[mmap(byref, dedup)]
    bucket: String,
}

#[derive(MmapStruct, Debug)]
struct FixedSection {
    #[fixed]
    items: Vec<FixedRecord>,
}

// ── Indexed element: same pod + byref, plus Vec<u8> to force indexed layout ──

#[derive(MmapStruct, Debug)]
struct IndexedRecord {
    id: u64,
    #[mmap(byref, dedup)]
    bucket: String,
    extra: Vec<u8>,
}

#[derive(MmapStruct, Debug)]
struct IndexedSection {
    items: Vec<IndexedRecord>,
}

// ── Helpers ──

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

    // ── Pod ──
    let pod = PodSection {
        points: (0..count)
            .map(|i| {
                let t = i as f32 * 0.001;
                PodPoint {
                    id: i as u64,
                    x: (t * 2.0).sin(),
                    y: (t * 3.0).cos(),
                    z: (t * 5.0).sin() * (t * 7.0).cos(),
                }
            })
            .collect(),
    };
    let pod_buf = mmapstruct_schema::write_to_file_with_embedded_schema(&pod, false);
    let pod_path = PathBuf::from("/tmp/pod_array.mms");
    fs::write(&pod_path, &pod_buf).expect("write file");

    // ── Fixed ──
    let fixed = FixedSection {
        items: (0..count)
            .map(|i| FixedRecord {
                id: i as u64,
                bucket: buckets[i % buckets.len()].to_string(),
            })
            .collect(),
    };
    let fixed_buf = mmapstruct_schema::write_to_file_with_embedded_schema(&fixed, false);
    let fixed_path = PathBuf::from("/tmp/fixed_array.mms");
    fs::write(&fixed_path, &fixed_buf).expect("write file");

    // ── Indexed ──
    let indexed = IndexedSection {
        items: (0..count)
            .map(|i| {
                let extra_len = i % 100;
                IndexedRecord {
                    id: i as u64,
                    bucket: buckets[i % buckets.len()].to_string(),
                    extra: (0..extra_len as u8).collect(),
                }
            })
            .collect(),
    };
    let indexed_buf = mmapstruct_schema::write_to_file_with_embedded_schema(&indexed, false);
    let indexed_path = PathBuf::from("/tmp/indexed_array.mms");
    fs::write(&indexed_path, &indexed_buf).expect("write file");

    // ── Stats ──
    let pod_size = pod_buf.len() as u64;
    let fixed_size = fixed_buf.len() as u64;
    let indexed_size = indexed_buf.len() as u64;

    println!("Generated 3 files with {count} elements each.");
    println!("bucket cycles through 5 short names → byref+dedup stores each only once.\n");

    // Overhead = file size − raw fixed-section bytes of elements
    let pod_elem = size_of::<PodPoint>() as u64; // 20 (packed)
    let fixed_elem = 16u64; // id(8) + ByRefRange(8)
    let indexed_elem = 16u64; // id(8) + ByRefRange(8)

    let pod_overhead = pod_size - count as u64 * pod_elem;
    let fixed_overhead = fixed_size - count as u64 * fixed_elem;
    let indexed_overhead = indexed_size - count as u64 * indexed_elem;

    println!("{:<14} {:>12} {:>12} {:>12} {:>14}",
        "Section", "File size", "Overhead", "Bytes/elem", "Payload/elem");
    println!("{:-<14} {:-<12} {:-<12} {:-<12} {:-<14}", "", "", "", "", "");

    println!("{:<14} {:>12} {:>12} {:>11.1} B  {:>11.1} B",
        "pod", format_bytes(pod_size), format_bytes(pod_overhead),
        pod_size as f64 / count as f64,
        pod_elem as f64);

    println!("{:<14} {:>12} {:>12} {:>11.1} B  {:>11.1} B",
        "fixed", format_bytes(fixed_size), format_bytes(fixed_overhead),
        fixed_size as f64 / count as f64,
        fixed_elem as f64);

    println!("{:<14} {:>12} {:>12} {:>11.1} B  {:>11.1} B",
        "indexed", format_bytes(indexed_size), format_bytes(indexed_overhead),
        indexed_size as f64 / count as f64,
        indexed_elem as f64);

    println!();
    println!("Overhead breakdown (header + schema + array framing + postponed):");
    println!("  pod:     {oh:>12}  — dense pod-array, free of framing", oh = format_bytes(pod_overhead));
    println!("  fixed:   {oh:>12}  — count+total_size(16) + postponed strings", oh = format_bytes(fixed_overhead));
    println!("  indexed: {oh:>12}  — offset table + Vec<u8> per element + postponed", oh = format_bytes(indexed_overhead));
}
