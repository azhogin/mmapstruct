use std::fs;
use std::path::PathBuf;

use bytemuck::{Pod, Zeroable};

use mmapstruct::MmapStruct;

// Derived pod struct → rich Struct schema, expandable in viewers.
#[derive(MmapStruct, Debug, Clone, Copy, Pod, Zeroable, Default)]
#[repr(C)]
struct PodPoint {
    x: f32,
    y: f32,
}

// Plain bytemuck pod WITHOUT MmapStruct derive → opaque PodStruct{size}.
#[derive(Debug, Clone, Copy, Pod, Zeroable, Default)]
#[repr(C)]
struct RawWeight([[u8; 4]; 2]);

#[derive(MmapStruct, Debug)]
#[repr(C)]
struct PodDemoFile {
    id: u32,
    // WITH derive — was [pod] before the fix, now expands.
    #[pod]
    origin: PodPoint,
    // WITHOUT derive — honest opaque blob.
    #[pod]
    weight: RawWeight,
    #[pod]
    points: Vec<PodPoint>,
    #[pod]
    weights: Vec<RawWeight>,
}

fn main() {
    let root = PodDemoFile {
        id: 42,
        origin: PodPoint { x: 1.5, y: -2.5 },
        weight: RawWeight([[1, 2, 3, 4], [5, 6, 7, 8]]),
        points: vec![
            PodPoint { x: 10.0, y: 20.0 },
            PodPoint { x: 30.0, y: 40.0 },
        ],
        weights: vec![RawWeight([[9; 4]; 2]), RawWeight([[8; 4]; 2]), RawWeight([[7; 4]; 2])],
    };

    let buf = mmapstruct_schema::write_to_file_with_embedded_schema(&root, false);
    let path = PathBuf::from("/tmp/pod_demo.mms");
    fs::write(&path, &buf).expect("write file");
    println!("Written {} ({} bytes)", path.display(), buf.len());
}
