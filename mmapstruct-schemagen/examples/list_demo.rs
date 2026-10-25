//! Generate `list_demo.mms` — a demo file exercising `List<T>` fields
//! (strings and pod structs) for manual viewer sessions:
//!
//! ```
//! cargo run -p mmapstruct-schemagen --example list_demo -- target/list_demo.mms
//! mmapview describe target/list_demo.mms
//! mmapview q target/list_demo.mms '.tags'
//! mmapview-http ... / mmapview-tui target/list_demo.mms
//! ```

use bytemuck::{Pod, Zeroable};
use mmapstruct::{List, MmapStruct};
use mmapstruct_schema::write_to_file_with_embedded_schema;

#[derive(MmapStruct, Debug, PartialEq, Clone, Copy, Pod, Zeroable, Default)]
#[repr(C)]
struct Probe {
    id: u64,
    score: f64,
}

#[derive(MmapStruct, Debug)]
struct DemoFile {
    id: u32,
    tags: List<String>,
    events: List<Probe>,
    vals: Vec<u32>,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "list_demo.mms".into());

    // Append mode: `list_demo append <file> <tag>` — patch one element onto
    // the `tags` list in place and exit.
    if out == "append" {
        let path = args.next().expect("append <file> <tag>");
        let tag = args.next().expect("append <file> <tag>");
        use mmapstruct::PatchSession;
        let mut session = PatchSession::open_typed::<DemoFile>(&path).unwrap();
        let reader = session.read::<DemoFile>().unwrap();
        let block_off = reader.tags.block_range().start;
        session
            .append_list::<DemoFile, String>(block_off, &tag)
            .unwrap();
        session.flush().unwrap();
        println!("appended {tag:?} to {path}");
        return;
    }

    let demo = DemoFile {
        id: 1,
        tags: List::from_vec(vec!["alpha".into(), "beta".into(), "gamma".into()]),
        events: List::from_vec(vec![
            Probe { id: 10, score: 1.5 },
            Probe {
                id: 20,
                score: -2.5,
            },
            Probe {
                id: 30,
                score: 3.25,
            },
        ]),
        vals: vec![7, 8, 9],
    };
    let bytes = write_to_file_with_embedded_schema(&demo, false);
    std::fs::write(&out, &bytes).unwrap();
    println!("wrote {out} ({} bytes)", bytes.len());
}
