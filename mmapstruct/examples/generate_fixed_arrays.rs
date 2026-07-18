use std::fs;
use std::path::PathBuf;

use mmapstruct::MmapStruct;

/// Nested fixed array: 16×16 matrix of u32 — all data in fixed section.
#[derive(MmapStruct, Debug, PartialEq)]
struct MatrixFile {
    matrix: [[u32; 16]; 16],
}

/// Vector of fixed arrays (dynamic-length list of [u32; 4] rows).
#[derive(MmapStruct, Debug, PartialEq)]
struct RowsFile {
    #[pod]
    rows: Vec<[u32; 4]>,
    label: String,
}

#[derive(MmapStruct, Debug, PartialEq)]
pub struct FixedArrayDemo {
    matrix: [[u32; 16]; 16],
    #[pod]
    rows: Vec<[u32; 4]>,
    label: String,
    #[mmap(byref)]
    desc: String,
}

fn main() {
    let out_dir = std::env::var("MMAPSTRUCT_OUT_DIR").unwrap_or_else(|_| "/tmp".into());
    let path = PathBuf::from(&out_dir).join("fixed_arrays.mms");

    // ── MatrixFile: pure fixed-array struct ──

    let mut m = MatrixFile { matrix: [[0u32; 16]; 16] };
    m.matrix[0][0] = 1;
    m.matrix[0][1] = 2;
    m.matrix[7][8] = 99;
    m.matrix[15][15] = 42;

    let m_buf = mmapstruct::write_to_buf(&m, false);
    {
        let r = mmapstruct::file_reader::BufReader::new(m_buf.clone());
        let reader = r.read::<MatrixFile>();
        assert_eq!(reader.matrix[0][0], 1);
        assert_eq!(reader.matrix[7][8], 99);
        assert_eq!(reader.matrix[15][15], 42);
        let restored: MatrixFile = reader.into();
        assert_eq!(restored, m);
    }
    println!("MatrixFile ([[u32; 16]; 16]): {} bytes, roundtrip OK", m_buf.len());

    // ── RowsFile: Vec<[u32; 4]> ──

    let r = RowsFile {
        rows: vec![[1, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12]],
        label: "rows".into(),
    };
    let r_buf = mmapstruct::write_to_buf(&r, false);
    {
        let r_reader = mmapstruct::file_reader::BufReader::new(r_buf.clone());
        let reader = r_reader.read::<RowsFile>();
        assert_eq!(reader.rows.len(), 3);
        assert_eq!(reader.rows[0][0], 1);
        assert_eq!(reader.rows[2][3], 12);
        let restored: RowsFile = reader.into();
        assert_eq!(restored, r);
    }
    println!("RowsFile (Vec<[u32; 4]>): {} bytes, roundtrip OK", r_buf.len());

    // ── FixedArrayDemo with schema ──

    let demo = FixedArrayDemo {
        matrix: m.matrix,
        rows: r.rows,
        label: "fixed-array demo".into(),
        desc: "A 16×16 matrix with Vec<[u32;4]> rows".into(),
    };

    let buf = mmapstruct_schema::write_to_file_with_embedded_schema(&demo, false);
    fs::write(&path, &buf).expect("write file");
    println!("FixedArrayDemo: {} bytes", buf.len());
    println!("Written to {}", path.display());
}
