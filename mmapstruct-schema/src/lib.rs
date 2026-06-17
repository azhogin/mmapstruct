use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use mmapstruct::{
    write_to_buf_raw, ArrayLayout, DynValueType, GetSchema, MmapStruct, MmapWrite, StructSchema,
    schema::EnumSchema,
};

pub mod context;

// ---------------------------------------------------------------------------
// Serialized schema types — MmapStruct-backed binary format
// ---------------------------------------------------------------------------

/// Top-level schema descriptor.
#[derive(MmapStruct, Debug, PartialEq)]
pub struct Schema {
    /// Flat table of all type definitions referenced by the struct.
    pub types: Vec<SchemaTypeDef>,
    /// Index into `types` for the root type.
    pub root_type: u32,
}

/// A single type definition in the schema table.
#[derive(MmapStruct, Debug, PartialEq)]
pub enum SchemaTypeDef {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Bool,
    Usize,
    Isize,
    Str { def: StrDef },
    PodStruct { size: u64 },
    Struct { def: StructDef },
    Enum { def: EnumDef },
    Array { def: ArrayDef },
    Option { def: OptionDef },
    ByRef { def: ByRefDef },
    HashMap { def: HashMapDef },
}

#[derive(MmapStruct, Debug, PartialEq)]
pub struct StrDef {
    pub inline_size: Option<u32>,
}

#[derive(MmapStruct, Debug, PartialEq)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
    pub fixed_section_size: u64,
    pub header_size: u64,
    pub inline_dyn_count: u32,
}

#[derive(MmapStruct, Debug, PartialEq)]
pub struct EnumDef {
    pub name: String,
    pub variants: Vec<VariantDef>,
}

#[derive(MmapStruct, Debug, PartialEq)]
pub struct VariantDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
    pub fixed_section_size: u64,
    pub header_size: u64,
    pub inline_dyn_count: u32,
}

#[derive(MmapStruct, Debug, PartialEq)]
pub struct FieldDef {
    pub name: String,
    pub type_index: u32,
    pub offset_in_fixed: u64,
    pub inline_dyn_index: Option<u32>,
    /// 0 or 1 — whether this field is stored by-reference.
    pub is_byref: u8,
}

#[derive(MmapStruct, Debug, PartialEq)]
pub enum ArrayLayoutDef {
    Pod { stride: u64 },
    Indexed,
    Fixed { stride: u64 },
}

#[derive(MmapStruct, Debug, PartialEq)]
pub struct ArrayDef {
    pub elem_type: u32,
    pub layout: ArrayLayoutDef,
}

#[derive(MmapStruct, Debug, PartialEq, Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct OptionDef {
    pub inner_type: u32,
}

#[derive(MmapStruct, Debug, PartialEq, Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct ByRefDef {
    pub inner_type: u32,
}

#[derive(MmapStruct, Debug, PartialEq, Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct HashMapDef {
    pub key_type: u32,
    pub val_type: u32,
    /// 0 or 1 — whether keys are stored as a dense pod array.
    pub key_is_pod: u32,
    /// 0 or 1 — whether values are stored as a dense pod array.
    pub val_is_pod: u32,
}

// Pod structs used as DynamicCustom fields in SchemaTypeDef generate
// readers as `&'a Self`. The derive doesn't generate `From<&'a Self>`
// for pod types, so we add manual impls for the `From<Reader>` chain.
macro_rules! impl_from_ref {
    ($t:ty) => {
        impl<'a> From<&'a $t> for $t {
            fn from(v: &'a $t) -> Self {
                *v
            }
        }
    };
}
impl_from_ref!(OptionDef);
impl_from_ref!(ByRefDef);
impl_from_ref!(HashMapDef);

// ---------------------------------------------------------------------------
// Builder — runtime DynValueType tree → Schema
// ---------------------------------------------------------------------------

/// Build a serialized `Schema` by walking the static `DynValueType` tree
/// for a type `T` that implements `GetSchema`.
///
/// The walker deduplicates types by pointer identity (`*const DynValueType`),
/// producing a compact flat type table.
pub fn build_serialized_schema<T: GetSchema>() -> Schema {
    let mut builder = Builder::new();
    let root_idx = builder.visit(T::root_value_type());
    Schema {
        types: builder.types,
        root_type: root_idx,
    }
}

struct Builder {
    types: Vec<SchemaTypeDef>,
    seen: HashMap<*const DynValueType, u32>,
}

impl Builder {
    fn new() -> Self {
        Self {
            types: Vec::new(),
            seen: HashMap::new(),
        }
    }

    fn visit(&mut self, vt: &DynValueType) -> u32 {
        // Lazy is transparent: resolve immediately and visit the real type.
        if let DynValueType::Lazy(resolve) = vt {
            return self.visit(resolve());
        }
        let key: *const DynValueType = vt;
        if let Some(&idx) = self.seen.get(&key) {
            return idx;
        }
        let idx = self.types.len() as u32;
        // Placeholder — will be updated after visiting children
        let placeholder = type_placeholder(vt);
        self.types.push(placeholder);
        self.seen.insert(key, idx);

        let actual = match vt {
            DynValueType::U8 => SchemaTypeDef::U8,
            DynValueType::U16 => SchemaTypeDef::U16,
            DynValueType::U32 => SchemaTypeDef::U32,
            DynValueType::U64 => SchemaTypeDef::U64,
            DynValueType::I8 => SchemaTypeDef::I8,
            DynValueType::I16 => SchemaTypeDef::I16,
            DynValueType::I32 => SchemaTypeDef::I32,
            DynValueType::I64 => SchemaTypeDef::I64,
            DynValueType::F32 => SchemaTypeDef::F32,
            DynValueType::F64 => SchemaTypeDef::F64,
            DynValueType::Bool => SchemaTypeDef::Bool,
            DynValueType::Usize => SchemaTypeDef::Usize,
            DynValueType::Isize => SchemaTypeDef::Isize,
            DynValueType::Str(inline) => SchemaTypeDef::Str {
                def: StrDef {
                    inline_size: inline.map(|n| n as u32),
                },
            },
            DynValueType::PodStruct { size } => SchemaTypeDef::PodStruct { size: *size },
            DynValueType::Struct(resolve) => {
                let s = resolve();
                self.visit_struct_schema(s)
            }
            DynValueType::Array(arr) => {
                let elem_idx = self.visit(arr.elem_type);
                let layout = match &arr.layout {
                    ArrayLayout::Pod { stride } => ArrayLayoutDef::Pod { stride: *stride },
                    ArrayLayout::Indexed => ArrayLayoutDef::Indexed,
                    ArrayLayout::Fixed { stride } => ArrayLayoutDef::Fixed { stride: *stride },
                };
                SchemaTypeDef::Array {
                    def: ArrayDef {
                        elem_type: elem_idx,
                        layout,
                    },
                }
            }
            DynValueType::Option(inner) => {
                let inner_idx = self.visit(inner);
                SchemaTypeDef::Option {
                    def: OptionDef {
                        inner_type: inner_idx,
                    },
                }
            }
            DynValueType::ByRef(inner) => {
                let inner_idx = self.visit(inner);
                SchemaTypeDef::ByRef {
                    def: ByRefDef {
                        inner_type: inner_idx,
                    },
                }
            }
            DynValueType::HashMap(hs) => {
                let key_idx = self.visit(hs.key_type);
                let val_idx = self.visit(hs.value_type);
                SchemaTypeDef::HashMap {
                    def: HashMapDef {
                        key_type: key_idx,
                        val_type: val_idx,
                        key_is_pod: hs.key_is_pod as u32,
                        val_is_pod: hs.val_is_pod as u32,
                    },
                }
            }
            DynValueType::Enum(es) => self.visit_enum_schema(es),
            DynValueType::Lazy(_) => unreachable!(),
        };
        self.types[idx as usize] = actual;
        idx
    }

    fn visit_struct_schema(&mut self, s: &StructSchema) -> SchemaTypeDef {
        let fields: Vec<FieldDef> = s
            .fields
            .iter()
            .map(|fd| {
                let type_idx = self.visit(fd.value_type);
                FieldDef {
                    name: fd.name.to_string(),
                    type_index: type_idx,
                    offset_in_fixed: fd.offset_in_fixed,
                    inline_dyn_index: fd.inline_dyn_index.map(|i| i as u32),
                    is_byref: fd.is_byref as u8,
                }
            })
            .collect();
        SchemaTypeDef::Struct {
            def: StructDef {
                name: s.name.to_string(),
                fields,
                fixed_section_size: s.fixed_section_size,
                header_size: s.header_size,
                inline_dyn_count: s.inline_dyn_count as u32,
            },
        }
    }

    fn visit_enum_schema(&mut self, es: &EnumSchema) -> SchemaTypeDef {
        let variants: Vec<VariantDef> = es
            .variants
            .iter()
            .map(|vs| {
                let fields: Vec<FieldDef> = vs
                    .fields
                    .iter()
                    .map(|fd| {
                        let type_idx = self.visit(fd.value_type);
                        FieldDef {
                            name: fd.name.to_string(),
                            type_index: type_idx,
                            offset_in_fixed: fd.offset_in_fixed,
                            inline_dyn_index: fd.inline_dyn_index.map(|i| i as u32),
                            is_byref: fd.is_byref as u8,
                        }
                    })
                    .collect();
                VariantDef {
                    name: vs.name.to_string(),
                    fields,
                    fixed_section_size: vs.fixed_section_size,
                    header_size: vs.header_size,
                    inline_dyn_count: vs.inline_dyn_count as u32,
                }
            })
            .collect();
        SchemaTypeDef::Enum {
            def: EnumDef {
                name: es.name.to_string(),
                variants,
            },
        }
    }
}

fn type_placeholder(vt: &DynValueType) -> SchemaTypeDef {
    // Return a structurally valid but semantically empty placeholder.
    // The actual type will replace it after children are visited.
    match vt {
        DynValueType::U8 => SchemaTypeDef::U8,
        DynValueType::U16 => SchemaTypeDef::U16,
        DynValueType::U32 => SchemaTypeDef::U32,
        DynValueType::U64 => SchemaTypeDef::U64,
        DynValueType::I8 => SchemaTypeDef::I8,
        DynValueType::I16 => SchemaTypeDef::I16,
        DynValueType::I32 => SchemaTypeDef::I32,
        DynValueType::I64 => SchemaTypeDef::I64,
        DynValueType::F32 => SchemaTypeDef::F32,
        DynValueType::F64 => SchemaTypeDef::F64,
        DynValueType::Bool => SchemaTypeDef::Bool,
        DynValueType::Usize => SchemaTypeDef::Usize,
        DynValueType::Isize => SchemaTypeDef::Isize,
        DynValueType::Str(_) => SchemaTypeDef::Str {
            def: StrDef { inline_size: None },
        },
        DynValueType::PodStruct { size } => SchemaTypeDef::PodStruct { size: *size },
        DynValueType::Struct(_) => SchemaTypeDef::Struct {
            def: StructDef {
                name: String::new(),
                fields: Vec::new(),
                fixed_section_size: 0,
                header_size: 0,
                inline_dyn_count: 0,
            },
        },
        DynValueType::Array(_) => SchemaTypeDef::Array {
            def: ArrayDef {
                elem_type: 0,
                layout: ArrayLayoutDef::Indexed,
            },
        },
        DynValueType::Option(_) => SchemaTypeDef::Option {
            def: OptionDef { inner_type: 0 },
        },
        DynValueType::ByRef(_) => SchemaTypeDef::ByRef {
            def: ByRefDef { inner_type: 0 },
        },
        DynValueType::HashMap(_) => SchemaTypeDef::HashMap {
            def: HashMapDef {
                key_type: 0,
                val_type: 0,
                key_is_pod: 0,
                val_is_pod: 0,
            },
        },
        DynValueType::Enum(_) => SchemaTypeDef::Enum {
            def: EnumDef {
                name: String::new(),
                variants: Vec::new(),
            },
        },
        DynValueType::Lazy(resolve) => type_placeholder(resolve()),
    }
}

// ---------------------------------------------------------------------------
// Write helper — data + schema in one buffer
// ---------------------------------------------------------------------------

/// Write a value with its serialized schema prepended as a separate section.
///
/// The output buffer layout:
///   [data written by `write_to_buf_raw`] ++ [schema bytes]
///
/// The caller is responsible for embedding the schema offset/length (e.g.
/// in a file header). Returns `(data_buf, schema_buf)`.
pub fn write_with_schema<T: MmapWrite + GetSchema>(value: &T) -> (Vec<u8>, Vec<u8>) {
    let data = write_to_buf_raw(value);
    let schema = build_serialized_schema::<T>();
    let schema_bytes = write_to_buf_raw(&schema);
    (data, schema_bytes)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mmapstruct::{MmapStruct, write_to_buf_raw};
    use mmapstruct::traits::MmapRead;

    #[test]
    fn test_schema_build_simple() {
        #[derive(MmapStruct)]
        struct Point {
            x: u64,
            y: String,
        }

        let schema = build_serialized_schema::<Point>();
        // Root struct type at index 0
        match &schema.types[0] {
            SchemaTypeDef::Struct { def } => {
                assert_eq!(def.name, "Point");
                assert_eq!(def.fields.len(), 2);
                assert_eq!(def.fields[0].name, "x");
                assert_eq!(def.fields[0].type_index, 1);
                assert_eq!(def.fields[1].name, "y");
                assert_eq!(def.fields[1].type_index, 2);
            }
            _ => panic!("expected struct"),
        }
        // Index 1: u64
        assert!(matches!(schema.types[1], SchemaTypeDef::U64));
        // Index 2: Str (inline String)
        assert!(matches!(schema.types[2], SchemaTypeDef::Str { .. }));
        assert_eq!(schema.root_type, 0);
    }

    #[test]
    fn test_schema_build_mixed() {
        #[derive(MmapStruct)]
        struct Rec {
            id: u64,
            name: String,
            tags: Vec<String>,
        }

        let schema = build_serialized_schema::<Rec>();
        // Root struct at index 0
        match &schema.types[0] {
            SchemaTypeDef::Struct { def } => {
                assert_eq!(def.name, "Rec");
                assert_eq!(def.fields.len(), 3);
                assert_eq!(def.fields[0].name, "id");
                assert_eq!(def.fields[1].name, "name");
                assert_eq!(def.fields[2].name, "tags");
            }
            _ => panic!("expected struct"),
        }
        assert!(matches!(schema.types[1], SchemaTypeDef::U64));
        assert!(matches!(schema.types[2], SchemaTypeDef::Str { .. }));

        // tags should be Array(String) at index 3.
        // Note: the inner String type is NOT deduped with the one at index 2
        // because pointer-identity dedup compares static variable addresses,
        // and each field's DynValueType is a separate static.
        match &schema.types[3] {
            SchemaTypeDef::Array { def } => {
                assert!(matches!(def.layout, ArrayLayoutDef::Indexed));
            }
            _ => panic!("expected array"),
        }
        assert_eq!(schema.root_type, 0);
    }

    #[test]
    fn test_schema_build_enum() {
        #[derive(MmapStruct)]
        enum MyEnum {
            Alpha { id: u64 },
            Beta { name: String, val: u64 },
        }

        let schema = build_serialized_schema::<MyEnum>();
        match &schema.types[0] {
            SchemaTypeDef::Enum { def } => {
                assert_eq!(def.name, "MyEnum");
                assert_eq!(def.variants.len(), 2);
                // Variant names include the enum path prefix
                assert_eq!(def.variants[0].name, "MyEnum::Alpha");
                assert_eq!(def.variants[0].fields.len(), 1);
                assert_eq!(def.variants[1].name, "MyEnum::Beta");
                assert_eq!(def.variants[1].fields.len(), 2);
            }
            _ => panic!("expected enum"),
        }
        assert_eq!(schema.root_type, 0);
    }

    #[test]
    fn test_schema_build_with_option() {
        #[derive(MmapStruct)]
        struct WithOpt {
            id: u64,
            label: Option<String>,
        }

        let schema = build_serialized_schema::<WithOpt>();
        match &schema.types[0] {
            SchemaTypeDef::Struct { def } => {
                assert_eq!(def.fields.len(), 2);
                assert_eq!(def.fields[0].name, "id");
                assert_eq!(def.fields[1].name, "label");
                // label's type_index points to the Option type at index 2
                assert_eq!(def.fields[1].type_index, 2);
            }
            _ => panic!("expected struct"),
        }
        // Index 1: u64
        assert!(matches!(schema.types[1], SchemaTypeDef::U64));
        // Index 2: Option — wraps the inner String type
        match &schema.types[2] {
            SchemaTypeDef::Option { def } => {
                // Inner type is Str at index 3
                assert_eq!(def.inner_type, 3);
            }
            _ => panic!("expected option"),
        }
        // Index 3: Str
        assert!(matches!(schema.types[3], SchemaTypeDef::Str { .. }));
        assert_eq!(schema.root_type, 0);
    }

    #[test]
    fn test_write_with_schema() {
        #[derive(MmapStruct)]
        struct Rec {
            id: u64,
            name: String,
        }

        let pt = Rec {
            id: 10,
            name: "hello".into(),
        };
        let (data, schema_bytes) = write_with_schema(&pt);

        // Data should not be empty
        assert!(!data.is_empty());
        // Schema bytes should contain a valid schema (we verify by rebuilding)
        let schema = build_serialized_schema::<Rec>();
        assert_eq!(schema.root_type, 0);
        // Schema bytes should be non-empty (the serialized form)
        assert!(!schema_bytes.is_empty());

        // Verify that data + header is consistent: write_to_buf should produce same data
        let full = mmapstruct::write_to_buf(&pt, false);
        assert_eq!(&full[24..], &data[..]); // skip 24-byte header
    }

    // -----------------------------------------------------------------------
    // Roundtrip tests — write Schema to binary and read it back.
    // These work now because enum variants self-align (align_to at end of each
    // variant write body), so Vec<SchemaTypeDef> elements are properly aligned.
    // -----------------------------------------------------------------------

    #[test]
    fn test_schema_roundtrip_simple() {
        #[derive(MmapStruct)]
        struct Point { x: u64, y: String }

        let schema = build_serialized_schema::<Point>();
        let buf = write_to_buf_raw(&schema);

        // Check alignment of the buffer data
        let ptr = buf.as_ptr() as usize;
        eprintln!("test_schema_roundtrip_simple: buf ptr=0x{:x}, aligned_to_8={}, aligned_to_4={}", 
            ptr, ptr % 8 == 0, ptr % 4 == 0);

        let schema2: Schema = <Schema as mmapstruct::traits::MmapRead>::read(&buf, 0..buf.len()).into();
        assert_eq!(schema.types.len(), schema2.types.len());
        assert_eq!(schema.root_type, schema2.root_type);
    }

    #[test]
    fn test_schema_roundtrip_mixed() {
        #[derive(MmapStruct)]
        struct Rec { id: u64, name: String, tags: Vec<String> }

        let schema = build_serialized_schema::<Rec>();
        let buf = write_to_buf_raw(&schema);

        let ptr = buf.as_ptr() as usize;
        eprintln!("test_schema_roundtrip_mixed: buf ptr=0x{:x}, aligned_to_8={}, aligned_to_4={}", 
            ptr, ptr % 8 == 0, ptr % 4 == 0);

        let schema2: Schema = <Schema as mmapstruct::traits::MmapRead>::read(&buf, 0..buf.len()).into();
        assert_eq!(schema.types.len(), schema2.types.len());
    }

    #[test]
    fn test_schema_roundtrip_enum() {
        #[derive(MmapStruct)]
        enum MyEnum { Alpha { id: u64 }, Beta { name: String, val: u64 } }

        let schema = build_serialized_schema::<MyEnum>();
        let buf = write_to_buf_raw(&schema);

        let ptr = buf.as_ptr() as usize;
        eprintln!("test_schema_roundtrip_enum: buf ptr=0x{:x}, aligned_to_8={}, aligned_to_4={}", 
            ptr, ptr % 8 == 0, ptr % 4 == 0);

        let schema2: Schema = <Schema as mmapstruct::traits::MmapRead>::read(&buf, 0..buf.len()).into();
        assert_eq!(schema.types.len(), schema2.types.len());
    }

    #[test]
    fn test_schema_roundtrip_option() {
        #[derive(MmapStruct)]
        struct WithOpt { id: u64, label: Option<String> }

        let schema = build_serialized_schema::<WithOpt>();
        let buf = write_to_buf_raw(&schema);

        let ptr = buf.as_ptr() as usize;
        eprintln!("test_schema_roundtrip_option: buf ptr=0x{:x}, aligned_to_8={}, aligned_to_4={}", 
            ptr, ptr % 8 == 0, ptr % 4 == 0);

        let schema2: Schema = <Schema as mmapstruct::traits::MmapRead>::read(&buf, 0..buf.len()).into();
        assert_eq!(schema.types.len(), schema2.types.len());
        assert_eq!(schema.root_type, schema2.root_type);
    }

    #[test]
    fn test_schema_write_with_schema_readback() {
        #[derive(MmapStruct)]
        struct Rec { id: u64, name: String }

        let pt = Rec { id: 10, name: "hello".into() };
        let (_data, schema_bytes) = write_with_schema(&pt);

        let ptr = schema_bytes.as_ptr() as usize;
        eprintln!("test_schema_write_with_schema_readback: buf ptr=0x{:x}, aligned_to_8={}, aligned_to_4={}", 
            ptr, ptr % 8 == 0, ptr % 4 == 0);

        let schema: Schema = <Schema as mmapstruct::traits::MmapRead>::read(&schema_bytes, 0..schema_bytes.len()).into();
        assert!(schema.types.len() >= 1);
        assert_eq!(schema.root_type, 0);
    }

    #[test]
    fn test_schema_context_field_get() {
        use crate::context::{SchemaDynContext, SchemaValue};

        #[derive(MmapStruct)]
        struct Point {
            x: u64,
            y: String,
        }

        let pt = Point {
            x: 42,
            y: "hello".into(),
        };
        let data = mmapstruct::write_to_buf_raw(&pt);
        let schema = build_serialized_schema::<Point>();
        let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

        // Read root type index zero-copy
        let schema_reader = <Schema as MmapRead>::read(&schema_bytes, 0..schema_bytes.len());
        let root_type = *schema_reader.root_type;

        let ctx = SchemaDynContext::new(&data, &schema_bytes);
        let root = SchemaValue::new(0, data.len() as u64, root_type);

        let x = ctx.field_get(&root, "x").unwrap();
        assert_eq!(ctx.read_u64(&x), 42);

        let y = ctx.field_get(&root, "y").unwrap();
        assert_eq!(ctx.read_str(&y), "hello");
    }

    #[test]
    fn test_schema_context_enum_variant() {
        use crate::context::{SchemaDynContext, SchemaValue};

        #[derive(MmapStruct)]
        enum MyEnum {
            Alpha { id: u32 },
            Beta { name: String, val: u64 },
        }

        // Write Alpha variant
        let w = MyEnum::Alpha { id: 42 };
        let data = mmapstruct::write_to_buf_raw(&w);
        let schema = build_serialized_schema::<MyEnum>();
        let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

        let schema_reader = <Schema as MmapRead>::read(&schema_bytes, 0..schema_bytes.len());
        let root_type = *schema_reader.root_type;

        let ctx = SchemaDynContext::new(&data, &schema_bytes);
        let root = SchemaValue::new(0, data.len() as u64, root_type);

        // Navigate into variant field
        let id = ctx.field_get(&root, "id").unwrap();
        assert_eq!(ctx.read_u32(&id), 42);

        // Check enum_variant
        let (_variant, _data_off) = ctx.enum_variant(&root).unwrap();

        // Write and navigate Beta variant
        let w = MyEnum::Beta {
            name: "test".into(),
            val: 99,
        };
        let data = mmapstruct::write_to_buf_raw(&w);
        let ctx = SchemaDynContext::new(&data, &schema_bytes);
        let root = SchemaValue::new(0, data.len() as u64, root_type);

        let name = ctx.field_get(&root, "name").unwrap();
        assert_eq!(ctx.read_str(&name), "test");
        let val = ctx.field_get(&root, "val").unwrap();
        assert_eq!(ctx.read_u64(&val), 99);
    }

    #[test]
    fn test_schema_context_array() {
        use crate::context::{SchemaDynContext, SchemaValue};

        #[derive(MmapStruct)]
        struct Item {
            id: u32,
            label: String,
        }

        #[derive(MmapStruct)]
        struct ItemFile {
            items: Vec<Item>,
        }

        let w = ItemFile {
            items: vec![
                Item { id: 10, label: "a".into() },
                Item { id: 20, label: "b".into() },
                Item { id: 30, label: "c".into() },
            ],
        };
        let data = mmapstruct::write_to_buf_raw(&w);
        let schema = build_serialized_schema::<ItemFile>();
        let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

        let schema_reader = <Schema as MmapRead>::read(&schema_bytes, 0..schema_bytes.len());
        let root_type = *schema_reader.root_type;

        let ctx = SchemaDynContext::new(&data, &schema_bytes);
        let root = SchemaValue::new(0, data.len() as u64, root_type);

        let items = ctx.field_get(&root, "items").unwrap();
        assert_eq!(ctx.array_len(&items), 3);

        let elem0 = ctx.array_get(&items, 0).unwrap();
        let id0 = ctx.field_get(&elem0, "id").unwrap();
        assert_eq!(ctx.read_u32(&id0), 10);
        let label0 = ctx.field_get(&elem0, "label").unwrap();
        assert_eq!(ctx.read_str(&label0), "a");

        let elem2 = ctx.array_get(&items, 2).unwrap();
        let id2 = ctx.field_get(&elem2, "id").unwrap();
        assert_eq!(ctx.read_u32(&id2), 30);
    }

    #[test]
    fn test_schema_context_read_as() {
        use crate::context::{SchemaDynContext, SchemaValue};

        #[derive(MmapStruct)]
        struct Stats {
            avg: f64,
            count: u32,
            name: String,
        }

        let w = Stats {
            avg: 3.5,
            count: 100,
            name: "test".into(),
        };
        let data = mmapstruct::write_to_buf_raw(&w);
        let schema = build_serialized_schema::<Stats>();
        let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

        let schema_reader = <Schema as MmapRead>::read(&schema_bytes, 0..schema_bytes.len());
        let root_type = *schema_reader.root_type;

        let ctx = SchemaDynContext::new(&data, &schema_bytes);
        let root = SchemaValue::new(0, data.len() as u64, root_type);

        let avg = ctx.field_get(&root, "avg").unwrap();
        assert!((ctx.read_f64(&avg) - 3.5).abs() < 1e-10);
        assert!((ctx.read_as_f64(&avg).unwrap() - 3.5).abs() < 1e-10);

        let count = ctx.field_get(&root, "count").unwrap();
        assert_eq!(ctx.read_u32(&count), 100);
        assert_eq!(ctx.read_as_i64(&count), Some(100i64));

        let name = ctx.field_get(&root, "name").unwrap();
        assert_eq!(ctx.read_str(&name), "test");
    }

    #[test]
    fn test_schema_context_byref_string() {
        use crate::context::{SchemaDynContext, SchemaValue};

        #[derive(MmapStruct)]
        struct Rec {
            name: String,
        }

        let w = Rec {
            name: "world".into(),
        };
        let data = mmapstruct::write_to_buf_raw(&w);
        let schema = build_serialized_schema::<Rec>();
        let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

        let schema_reader = <Schema as MmapRead>::read(&schema_bytes, 0..schema_bytes.len());
        let root_type = *schema_reader.root_type;

        let ctx = SchemaDynContext::new(&data, &schema_bytes);
        let root = SchemaValue::new(0, data.len() as u64, root_type);

        let name = ctx.field_get(&root, "name").unwrap();
        assert_eq!(ctx.read_str(&name), "world");
    }

    #[test]
    fn test_schema_context_option_none() {
        use crate::context::{SchemaDynContext, SchemaValue};

        #[derive(MmapStruct)]
        struct WithOpt {
            label: Option<String>,
        }

        let w = WithOpt { label: None };
        let data = mmapstruct::write_to_buf_raw(&w);
        let schema = build_serialized_schema::<WithOpt>();
        let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

        let schema_reader = <Schema as MmapRead>::read(&schema_bytes, 0..schema_bytes.len());
        let root_type = *schema_reader.root_type;

        let ctx = SchemaDynContext::new(&data, &schema_bytes);
        let root = SchemaValue::new(0, data.len() as u64, root_type);

        let label = ctx.field_get(&root, "label").unwrap();
        // None byref → returns a Value with offset == limit (zero-length)
        assert_eq!(label.offset, label.limit);
    }

    #[test]
    fn test_schema_context_option_some() {
        use crate::context::{SchemaDynContext, SchemaValue};

        #[derive(MmapStruct)]
        struct WithOpt {
            label: Option<String>,
        }

        let w = WithOpt {
            label: Some("present".into()),
        };
        let data = mmapstruct::write_to_buf_raw(&w);
        let schema = build_serialized_schema::<WithOpt>();
        let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

        let schema_reader = <Schema as MmapRead>::read(&schema_bytes, 0..schema_bytes.len());
        let root_type = *schema_reader.root_type;

        let ctx = SchemaDynContext::new(&data, &schema_bytes);
        let root = SchemaValue::new(0, data.len() as u64, root_type);

        let label = ctx.field_get(&root, "label").unwrap();
        assert_eq!(ctx.read_str(&label), "present");
    }

    #[test]
    fn test_schema_format_value_struct() {
        use crate::context::{format_schema_value, SchemaDynContext, SchemaValue};

        #[derive(MmapStruct)]
        struct Point {
            x: u64,
            y: String,
        }

        let pt = Point {
            x: 42,
            y: "hello".into(),
        };
        let data = mmapstruct::write_to_buf_raw(&pt);
        let schema = build_serialized_schema::<Point>();
        let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

        let schema_reader = <Schema as MmapRead>::read(&schema_bytes, 0..schema_bytes.len());
        let root_type = *schema_reader.root_type;

        let ctx = SchemaDynContext::new(&data, &schema_bytes);
        let root = SchemaValue::new(0, data.len() as u64, root_type);

        let s = format_schema_value(&ctx, &root);
        assert_eq!(s, "{\n  \"x\": 42,\n  \"y\": \"hello\"\n}");
    }

    #[test]
    fn test_schema_format_value_enum() {
        use crate::context::{format_schema_value, SchemaDynContext, SchemaValue};

        #[derive(MmapStruct)]
        enum MyEnum {
            Alpha { id: u32 },
            Beta { name: String, val: u64 },
        }

        let w = MyEnum::Beta {
            name: "test".into(),
            val: 99,
        };
        let data = mmapstruct::write_to_buf_raw(&w);
        let schema = build_serialized_schema::<MyEnum>();
        let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

        let schema_reader = <Schema as MmapRead>::read(&schema_bytes, 0..schema_bytes.len());
        let root_type = *schema_reader.root_type;

        let ctx = SchemaDynContext::new(&data, &schema_bytes);
        let root = SchemaValue::new(0, data.len() as u64, root_type);

        let s = format_schema_value(&ctx, &root);
        assert_eq!(s, "MyEnum::Beta {\n  \"name\": \"test\",\n  \"val\": 99\n}");
    }

    #[test]
    fn test_schema_format_value_array() {
        use crate::context::{format_schema_value, SchemaDynContext, SchemaValue};

        #[derive(MmapStruct)]
        struct Item {
            id: u32,
            label: String,
        }

        #[derive(MmapStruct)]
        struct ItemFile {
            items: Vec<Item>,
        }

        let w = ItemFile {
            items: vec![Item { id: 1, label: "a".into() }, Item { id: 2, label: "b".into() }],
        };
        let data = mmapstruct::write_to_buf_raw(&w);
        let schema = build_serialized_schema::<ItemFile>();
        let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

        let schema_reader = <Schema as MmapRead>::read(&schema_bytes, 0..schema_bytes.len());
        let root_type = *schema_reader.root_type;

        let ctx = SchemaDynContext::new(&data, &schema_bytes);
        let root = SchemaValue::new(0, data.len() as u64, root_type);

        let items = ctx.field_get(&root, "items").unwrap();
        let s = format_schema_value(&ctx, &items);
        assert_eq!(s, "[\n  {\n    \"id\": 1,\n    \"label\": \"a\"\n  },\n  {\n    \"id\": 2,\n    \"label\": \"b\"\n  }\n]");
    }

    #[test]
    fn test_schema_query_path() {
        use crate::context::{SchemaDynContext, SchemaValue, navigate_path};

        #[derive(MmapStruct)]
        struct Item {
            id: u32,
            label: String,
        }
        #[derive(MmapStruct)]
        struct ItemFile {
            items: Vec<Item>,
        }

        let w = ItemFile {
            items: vec![
                Item { id: 10, label: "x".into() },
                Item { id: 20, label: "y".into() },
            ],
        };
        let data = mmapstruct::write_to_buf_raw(&w);
        let schema = build_serialized_schema::<ItemFile>();
        let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

        let schema_reader = <Schema as MmapRead>::read(&schema_bytes, 0..schema_bytes.len());
        let root_type = *schema_reader.root_type;

        let ctx = SchemaDynContext::new(&data, &schema_bytes);
        let root = SchemaValue::new(0, data.len() as u64, root_type);

        // Navigate /items[0]/label
        let result = navigate_path(&ctx, &root, "/items[0]/label").unwrap();
        match result {
            crate::context::SchemaEvalResult::Buf(v) => {
                assert_eq!(ctx.read_str(&v), "x");
            }
            _ => panic!("expected buf"),
        }

        // Navigate /items[1]/id
        let result = navigate_path(&ctx, &root, "/items[1]/id").unwrap();
        match result {
            crate::context::SchemaEvalResult::Buf(v) => {
                assert_eq!(ctx.read_u32(&v), 20);
            }
            _ => panic!("expected buf"),
        }
    }
}
