use std::fs;
use std::path::PathBuf;
use bytemuck::{Pod, Zeroable};

use mmapstruct::MmapStruct;

#[derive(MmapStruct, Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct Point3d {
    id: u32,
    x: f32,
    y: f32,
    z: f32,
}

#[derive(MmapStruct, Debug)]
struct PointCloud {
    #[pod]
    points: Vec<Point3d>,
}

fn main() {
    let count = 1_000_000;

    let points: Vec<Point3d> = (0..count)
        .map(|i| {
            let t = i as f32 * 0.001;
            Point3d {
                id: i as u32,
                x: (t * 2.0).sin(),
                y: (t * 3.0).cos(),
                z: (t * 5.0).sin() * (t * 7.0).cos(),
            }
        })
        .collect();

    let cloud = PointCloud { points };

    let buf = mmapstruct_schema::write_to_file_with_embedded_schema(&cloud, false);

    let path = PathBuf::from("/tmp/million_points.mms");
    fs::write(&path, &buf).expect("write file");

    let file_size = buf.len();
    let point_size = 16; // id(u32) + 3 * f32
    println!("Written {} points to {}", count, path.display());
    println!("File size: {} bytes ({} bytes payload, {} bytes per point)",
        file_size, count * point_size, point_size);
}
