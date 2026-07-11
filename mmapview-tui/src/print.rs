use mmapstruct_schema::context::RuntimeSchemaCtx;
use mmapstruct_schema::schema::{PrimitiveDef, TypeDefReader};
use mmapstruct_schema::SchemaValue;

pub fn type_annotation(t: &TypeDefReader) -> &'static str {
    match t {
        TypeDefReader::Primitive(p) => match p {
            PrimitiveDef::U32 => ": u32",
            PrimitiveDef::I32 => ": i32",
            PrimitiveDef::U64 => ": u64",
            PrimitiveDef::I64 => ": i64",
            PrimitiveDef::F32 => ": f32",
            PrimitiveDef::F64 => ": f64",
            PrimitiveDef::Bool => ": bool",
            PrimitiveDef::U16 => ": u16",
            PrimitiveDef::I16 => ": i16",
            PrimitiveDef::U8 => ": u8",
            PrimitiveDef::I8 => ": i8",
            PrimitiveDef::Usize => ": usize",
            PrimitiveDef::Isize => ": isize",
        },
        TypeDefReader::Str { .. } => ": str",
        TypeDefReader::PodStruct { .. } => ": [pod]",
        TypeDefReader::Struct { .. } => "",
        TypeDefReader::Enum { .. } => "",
        TypeDefReader::Array { .. } => "",
        TypeDefReader::Option { .. } => "",
        TypeDefReader::ByRef { .. } => ": byref",
        TypeDefReader::HashMap { .. } => "",
    }
}

pub fn inline_value_text(ctx: &RuntimeSchemaCtx, val: &SchemaValue) -> String {
    let t = ctx.type_reader(val.type_idx);
    match t {
        TypeDefReader::Primitive(p) => match p {
            PrimitiveDef::U32 => ctx.read_u32(val).to_string(),
            PrimitiveDef::I32 => ctx.read_i32(val).to_string(),
            PrimitiveDef::U64 => ctx.read_u64(val).to_string(),
            PrimitiveDef::I64 => ctx.read_i64(val).to_string(),
            PrimitiveDef::F32 => ctx.read_f32(val).to_string(),
            PrimitiveDef::F64 => ctx.read_f64(val).to_string(),
            PrimitiveDef::Bool => {
                if ctx.read_bool(val) { "true".to_string() } else { "false".to_string() }
            }
            PrimitiveDef::U16 => ctx.read_u16(val).to_string(),
            PrimitiveDef::I16 => ctx.read_i16(val).to_string(),
            PrimitiveDef::U8 => ctx.read_u8(val).to_string(),
            PrimitiveDef::I8 => ctx.read_i8(val).to_string(),
            PrimitiveDef::Usize => ctx.read_usize(val).to_string(),
            PrimitiveDef::Isize => ctx.read_isize(val).to_string(),
        },
        TypeDefReader::Str { .. } => {
            let s = if val.mmap_end as usize <= ctx.data().len() {
                ctx.read_str(val).to_string()
            } else {
                "<bad str>".to_string()
            };
            if s.len() > 80 {
                format!("\"{}...\"", &s[..77])
            } else {
                format!("\"{}\"", s)
            }
        }
        TypeDefReader::Struct { name, .. } => format!("{} {{ ... }}", name),
        TypeDefReader::Enum { .. } => {
            if let Some((variant_type_idx, _)) = ctx.enum_variant(val) {
                let vt = ctx.type_reader(variant_type_idx);
                if let TypeDefReader::Struct { name, .. } = vt {
                    name.to_string()
                } else {
                    "<enum>".to_string()
                }
            } else {
                "<enum>".to_string()
            }
        }
        TypeDefReader::Array { .. } => {
            if val.mmap_end as usize <= ctx.data().len() {
                format!("[{} elements]", ctx.array_len(val))
            } else {
                "[?]".to_string()
            }
        }
        TypeDefReader::Option { inner_type } => {
            if val.mmap_begin == val.mmap_end {
                "None".to_string()
            } else {
                let inner = SchemaValue::new(val.mmap_begin, val.mmap_end, **inner_type);
                format!("Some({})", inline_value_text(ctx, &inner))
            }
        }
        TypeDefReader::ByRef { inner_type } => {
            let inner = SchemaValue::new(val.mmap_begin, val.mmap_end, **inner_type);
            inline_value_text(ctx, &inner)
        }
        TypeDefReader::HashMap { .. } => {
            if val.mmap_end as usize <= ctx.data().len() {
                format!("[{} entries]", ctx.hashmap_len(val))
            } else {
                "[?]".to_string()
            }
        }
        TypeDefReader::PodStruct { .. } => "[pod]".to_string(),
    }
}
