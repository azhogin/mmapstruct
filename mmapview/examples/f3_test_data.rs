use std::collections::HashMap;
use std::fs;

use mmapstruct::{bytemuck, MmapStruct};
use mmapstruct_schema::write_to_file_with_embedded_schema;

#[derive(MmapStruct)]
enum Category {
    Weapon { damage: u64 },
    Consumable { heal: u64, charges: u64 },
}

#[derive(MmapStruct)]
struct TagInfo {
    name: String,
    count: u64,
}

#[derive(MmapStruct, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Metadata {
    generated_at: u64,
    item_count: u64,
}

#[derive(MmapStruct)]
struct Item {
    name: String,
    price: u64,
    tags: Vec<TagInfo>,
    category: Category,
}

#[derive(MmapStruct)]
struct ItemCatalog {
    version: u64,
    items: Vec<Item>,
    metadata: Metadata,
    labels: HashMap<String, String>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = if args.len() > 1 {
        &args[1]
    } else {
        "/tmp/mmv_test.mms"
    };

    let catalog = ItemCatalog {
        version: 42,
        items: vec![
            Item {
                name: "Sword".into(),
                price: 100,
                tags: vec![
                    TagInfo {
                        name: "weapon".into(),
                        count: 1,
                    },
                    TagInfo {
                        name: "melee".into(),
                        count: 2,
                    },
                ],
                category: Category::Weapon { damage: 50 },
            },
            Item {
                name: "Shield".into(),
                price: 200,
                tags: vec![TagInfo {
                    name: "armor".into(),
                    count: 3,
                }],
                category: Category::Consumable {
                    heal: 0,
                    charges: 1,
                },
            },
            Item {
                name: "Potion".into(),
                price: 50,
                tags: vec![],
                category: Category::Consumable {
                    heal: 30,
                    charges: 3,
                },
            },
        ],
        metadata: Metadata {
            generated_at: 1718712000,
            item_count: 3,
        },
        labels: {
            let mut m = HashMap::new();
            m.insert("author".into(), "test".into());
            m.insert("version".into(), "1.0".into());
            m
        },
    };

    let buf = write_to_file_with_embedded_schema(&catalog, false);
    fs::write(path, &buf).expect("write failed");
    eprintln!("Wrote {} bytes to {}", buf.len(), path);
}
