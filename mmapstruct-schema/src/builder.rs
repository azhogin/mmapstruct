use std::collections::HashMap;

use mmapstruct::{
    write_to_buf_raw, ArrayLayout, DynValueType, GetSchema, MmapWrite, StructSchema,
    schema::EnumSchema,
};

use crate::schema;

// ---------------------------------------------------------------------------
// Builder — runtime DynValueType tree → Schema
// ---------------------------------------------------------------------------

/// Build a serialized `Schema` by walking the static `DynValueType` tree
/// for a type `T` that implements `GetSchema`.
///
/// The walker deduplicates types by pointer identity (`*const DynValueType`),
/// producing a compact flat type table.
pub fn build_serialized_schema<T: GetSchema>() -> schema::Schema {
    let mut builder = Builder::new();
    let root_idx = builder.visit(T::root_value_type());
    schema::Schema {
        types: builder.types,
        root_type: root_idx,
    }
}

struct Builder {
    types: Vec<schema::TypeDef>,
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
            DynValueType::U8 => schema::TypeDef::Primitive(schema::PrimitiveDef::U8),
            DynValueType::U16 => schema::TypeDef::Primitive(schema::PrimitiveDef::U16),
            DynValueType::U32 => schema::TypeDef::Primitive(schema::PrimitiveDef::U32),
            DynValueType::U64 => schema::TypeDef::Primitive(schema::PrimitiveDef::U64),
            DynValueType::I8 => schema::TypeDef::Primitive(schema::PrimitiveDef::I8),
            DynValueType::I16 => schema::TypeDef::Primitive(schema::PrimitiveDef::I16),
            DynValueType::I32 => schema::TypeDef::Primitive(schema::PrimitiveDef::I32),
            DynValueType::I64 => schema::TypeDef::Primitive(schema::PrimitiveDef::I64),
            DynValueType::F32 => schema::TypeDef::Primitive(schema::PrimitiveDef::F32),
            DynValueType::F64 => schema::TypeDef::Primitive(schema::PrimitiveDef::F64),
            DynValueType::Bool => schema::TypeDef::Primitive(schema::PrimitiveDef::Bool),
            DynValueType::Usize => schema::TypeDef::Primitive(schema::PrimitiveDef::Usize),
            DynValueType::Isize => schema::TypeDef::Primitive(schema::PrimitiveDef::Isize),
            DynValueType::Str(inline) => schema::TypeDef::Str {
                size: inline.map(|n| n as u32),
            },
            DynValueType::PodStruct { size } => schema::TypeDef::PodStruct { size: *size },
            DynValueType::Struct(resolve) => {
                let s = resolve();
                self.visit_struct_schema(s)
            }
            DynValueType::Array(arr) => {
                let elem_idx = self.visit(arr.elem_type);
                let layout = match &arr.layout {
                    ArrayLayout::Pod { stride } => schema::ArrayLayoutDef::Pod { stride: *stride },
                    ArrayLayout::FixedPod { stride, count } => schema::ArrayLayoutDef::FixedPod { stride: *stride, count: *count },
                    ArrayLayout::Indexed => schema::ArrayLayoutDef::Indexed,
                    ArrayLayout::Fixed { stride } => schema::ArrayLayoutDef::Fixed { stride: *stride },
                };
                schema::TypeDef::Array {
                    elem_type: elem_idx,
                    layout,
                }
            }
            DynValueType::Option(inner) => {
                let inner_idx = self.visit(inner);
                schema::TypeDef::Option {
                    inner_type: inner_idx,
                }
            }
            DynValueType::ByRef(inner) => {
                let inner_idx = self.visit(inner);
                schema::TypeDef::ByRef {
                    inner_type: inner_idx,
                }
            }
            DynValueType::HashMap(hs) => {
                let key_idx = self.visit(hs.key_type);
                let val_idx = self.visit(hs.value_type);
                schema::TypeDef::HashMap {
                    key_type: key_idx,
                    val_type: val_idx,
                    key_is_pod: hs.key_is_pod as u32,
                    val_is_pod: hs.val_is_pod as u32,
                }
            }
            DynValueType::Enum(es) => self.visit_enum_schema(es),
            DynValueType::Lazy(_) => unreachable!(),
        };
        self.types[idx as usize] = actual;
        idx
    }

    fn visit_struct_schema(&mut self, s: &StructSchema) -> schema::TypeDef {
        let fields: Vec<schema::FieldDef> = s
            .fields
            .iter()
            .map(|fd| {
                let type_idx = self.visit(fd.value_type);
                schema::FieldDef {
                    name: fd.name.to_string(),
                    type_index: type_idx,
                    offset_in_fixed: fd.offset_in_fixed,
                    inline_dyn_index: fd.inline_dyn_index.map(|i| i as u32),
                }
            })
            .collect();
        schema::TypeDef::Struct {
            name: s.name.to_string(),
            fields,
            fixed_section_size: s.fixed_section_size,
            header_size: s.header_size,
            inline_dyn_count: s.inline_dyn_count as u32,
        }
    }

    fn visit_enum_schema(&mut self, es: &EnumSchema) -> schema::TypeDef {
        let mut variants = Vec::with_capacity(es.variants.len());
        let mut variant_type_indices = Vec::with_capacity(es.variants.len());

        for vs in es.variants.iter() {
            let fields: Vec<schema::FieldDef> = vs
                .fields
                .iter()
                .map(|fd| {
                    let type_idx = self.visit(fd.value_type);
                    schema::FieldDef {
                        name: fd.name.to_string(),
                        type_index: type_idx,
                        offset_in_fixed: fd.offset_in_fixed,
                        inline_dyn_index: fd.inline_dyn_index.map(|i| i as u32),
                    }
                })
                .collect();

            // Create the Struct type for this variant in the types table
            let variant_type = schema::TypeDef::Struct {
                name: vs.name.to_string(),
                fields: vs.fields.iter().map(|fd| {
                    let type_idx = self.visit(fd.value_type);
                    schema::FieldDef {
                        name: fd.name.to_string(),
                        type_index: type_idx,
                        offset_in_fixed: fd.offset_in_fixed,
                        inline_dyn_index: fd.inline_dyn_index.map(|i| i as u32),
                    }
                }).collect(),
                fixed_section_size: vs.fixed_section_size,
                header_size: vs.header_size,
                inline_dyn_count: vs.inline_dyn_count as u32,
            };
            let idx = self.types.len() as u32;
            self.types.push(variant_type);
            variant_type_indices.push(idx);

            variants.push(schema::VariantDef {
                name: vs.name.to_string(),
                fields,
                fixed_section_size: vs.fixed_section_size,
                header_size: vs.header_size,
                inline_dyn_count: vs.inline_dyn_count as u32,
            });
        }

        schema::TypeDef::Enum {
            name: es.name.to_string(),
            variants,
            variant_type_indices,
        }
    }
}

fn type_placeholder(vt: &DynValueType) -> schema::TypeDef {
    // Return a structurally valid but semantically empty placeholder.
    // The actual type will replace it after children are visited.
    match vt {
        DynValueType::U8 => schema::TypeDef::Primitive(schema::PrimitiveDef::U8),
        DynValueType::U16 => schema::TypeDef::Primitive(schema::PrimitiveDef::U16),
        DynValueType::U32 => schema::TypeDef::Primitive(schema::PrimitiveDef::U32),
        DynValueType::U64 => schema::TypeDef::Primitive(schema::PrimitiveDef::U64),
        DynValueType::I8 => schema::TypeDef::Primitive(schema::PrimitiveDef::I8),
        DynValueType::I16 => schema::TypeDef::Primitive(schema::PrimitiveDef::I16),
        DynValueType::I32 => schema::TypeDef::Primitive(schema::PrimitiveDef::I32),
        DynValueType::I64 => schema::TypeDef::Primitive(schema::PrimitiveDef::I64),
        DynValueType::F32 => schema::TypeDef::Primitive(schema::PrimitiveDef::F32),
        DynValueType::F64 => schema::TypeDef::Primitive(schema::PrimitiveDef::F64),
        DynValueType::Bool => schema::TypeDef::Primitive(schema::PrimitiveDef::Bool),
        DynValueType::Usize => schema::TypeDef::Primitive(schema::PrimitiveDef::Usize),
        DynValueType::Isize => schema::TypeDef::Primitive(schema::PrimitiveDef::Isize),
        DynValueType::Str(_) => schema::TypeDef::Str { size: None },
        DynValueType::PodStruct { size } => schema::TypeDef::PodStruct { size: *size },
        DynValueType::Struct(_) => schema::TypeDef::Struct {
            name: String::new(),
            fields: Vec::new(),
            fixed_section_size: 0,
            header_size: 0,
            inline_dyn_count: 0,
        },
        DynValueType::Array(_) => schema::TypeDef::Array {
            elem_type: 0,
            layout: schema::ArrayLayoutDef::Indexed,
        },
        DynValueType::Option(_) => schema::TypeDef::Option { inner_type: 0 },
        DynValueType::ByRef(_) => schema::TypeDef::ByRef { inner_type: 0 },
        DynValueType::HashMap(_) => schema::TypeDef::HashMap {
            key_type: 0,
            val_type: 0,
            key_is_pod: 0,
            val_is_pod: 0,
        },
        DynValueType::Enum(_) => schema::TypeDef::Enum {
            name: String::new(),
            variants: Vec::new(),
            variant_type_indices: Vec::new(),
        },
        DynValueType::Lazy(resolve) => type_placeholder(resolve()),
    }
}

// ---------------------------------------------------------------------------
// Write helper — data + schema in one buffer
// ---------------------------------------------------------------------------

/// Write a value with its serialized schema as a separate section.
/// Returns `(data_buf, schema_buf)`.
pub fn write_with_schema<T: MmapWrite + GetSchema>(value: &T) -> (Vec<u8>, Vec<u8>) {
    let data = write_to_buf_raw(value);
    let schema = build_serialized_schema::<T>();
    let schema_bytes = write_to_buf_raw(&schema);
    (data, schema_bytes)
}

/// Write a value with its schema embedded in an extended v2 header.
///
/// The output is a complete mmapstruct file that can be read by any
/// v2-aware reader. The schema is retrievable via
/// `MmapReader::embedded_schema()` / `BufReader::embedded_schema()`.
pub fn write_to_file_with_embedded_schema<T: MmapWrite + GetSchema + mmapstruct::LayoutHash>(
    value: &T,
    write_crc: bool,
) -> Vec<u8> {
    let schema = build_serialized_schema::<T>();
    let schema_bytes = write_to_buf_raw(&schema);
    mmapstruct::write_to_buf_with_schema(value, write_crc, &schema_bytes)
}
