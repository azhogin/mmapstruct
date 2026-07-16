use std::fs;
use std::path::PathBuf;

use mmapstruct::MmapStruct;

#[derive(MmapStruct, Debug)]
enum Shape {
    Circle { radius: f64 },
    Rect { width: f64, height: f64 },
    Empty,
}

#[derive(MmapStruct, Debug)]
struct MyEnums {
    shapes: Vec<Shape>,
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

    let shapes: Vec<Shape> = (0..count)
        .map(|i| match i % 3 {
            0 => Shape::Circle { radius: i as f64 * 0.1 },
            1 => Shape::Rect { width: i as f64 * 0.5, height: (i + 1) as f64 * 0.3 },
            _ => Shape::Empty,
        })
        .collect();

    let root = MyEnums { shapes };

    let buf = mmapstruct_schema::write_to_file_with_embedded_schema(&root, false);
    let path = PathBuf::from("/tmp/enums.mms");
    fs::write(&path, &buf).expect("write file");

    let file_size = buf.len() as u64;
    println!("Written 1 enum array × {count} entries to {}", path.display());
    println!();
    println!("{:<30} {:>14}", "Total file size", format_bytes(file_size));
}