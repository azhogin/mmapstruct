//! Name-addressed patching over the runtime schema — [`SchemaPatchSession`].
//!
//! Patch operations addressed by field PATH (`"events"`, `"inner.items"`)
//! instead of byte offsets from typed reader views. This is the entry point
//! for schema-only contexts (HTTP / TUI / CLI tools) where no compile-time
//! root type exists: the file's runtime schema provides every layout fact
//! the patch operations need.
//!
//! The schema-driven resolution reads only **immutable** structure (offset
//! table entries, reserve flags, Option present bytes, discriminants) — all
//! of it unchanged by appends — so the schema context may work from the
//! payload snapshot taken at `open`; mutable state (counts, chain links,
//! gap markers) is read by the underlying [`PatchSession`] from the live
//! mmap.
//!
//! Payloads for dynamic elements are **raw serialized bytes** (a
//! schema-only caller has no compile-time element type); Pod-typed helpers
//! cover the fully typed cases. Alignment comes from the runtime schema
//! (`type_align` of the element type), matching the writer.

use std::io;

use mmapstruct::patch::PatchSession;
use mmapstruct::traits::{MmapPod, RootLayout, BLOCK_FLAG_RESERVED};

use crate::context::RuntimeSchemaCtx;
use crate::helpers::TypeDefReaderExt;
use crate::schema::{FieldDef, TypeDefReader};
use crate::schema_source::{load_schema_full, SchemaResult};
use crate::value::SchemaValue;

/// A patch session addressed by field paths over the file's runtime schema.
pub struct SchemaPatchSession<'a> {
    session: PatchSession,
    ctx: RuntimeSchemaCtx<'a>,
    root: SchemaValue,
    root_layout: RootLayout,
}

/// A resolved patch target.
struct ResolvedTarget {
    /// Payload offset of the struct instance that owns the field (the base
    /// for fixed-section slots such as byref placeholders).
    parent_base: u64,
    field: FieldDef,
    /// The field's value (for containers: the block range starting at
    /// `mmap_begin`).
    value: SchemaValue,
}

impl<'a> SchemaPatchSession<'a> {
    /// Open a file for schema-driven patching, resolving the schema from the
    /// file itself (embedded → registry → sidecar).
    pub fn open(path: impl AsRef<std::path::Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let data = std::fs::read(path)
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
        let resolved = load_schema_full(&data, None, Some(&path.to_string_lossy()))
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Self::from_parts(path, &data, &resolved)
    }

    /// Open a file for schema-driven patching with an explicitly provided
    /// schema (raw schema bytes, possibly wrapped in a header).
    pub fn open_with_schema(
        path: impl AsRef<std::path::Path>,
        schema_bytes: &[u8],
    ) -> io::Result<Self> {
        let path = path.as_ref();
        let data = std::fs::read(path)
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
        let (payload, _, _, _) = mmapstruct::file_reader::parse_header(&data);
        let payload_start = payload.as_ptr() as usize - data.as_ptr() as usize;
        let resolved = SchemaResult {
            schema_bytes: schema_bytes.to_vec(),
            payload_range: payload_start..payload_start + payload.len(),
        };
        Self::from_parts(path, &data, &resolved)
    }

    fn from_parts(
        path: &std::path::Path,
        data: &[u8],
        resolved: &SchemaResult,
    ) -> io::Result<Self> {
        // Headered files open directly; headerless split-layout payloads
        // (pure payload at offset 0) take the sidecar session, whose header
        // carries the identity hash.
        let session = if mmapstruct::file_reader::stored_layout_hash(path)?.is_some() {
            PatchSession::open(path)?
        } else {
            PatchSession::open_with_sidecar(path)?
        };
        // Payload-relative coordinates: the patch operations work in
        // payload space, so the schema context must too.
        let payload: &'static [u8] = Box::leak(
            data[resolved.payload_range.clone()]
                .to_vec()
                .into_boxed_slice(),
        );
        let schema_bytes: &'static [u8] =
            Box::leak(resolved.schema_bytes.clone().into_boxed_slice());
        let ctx = RuntimeSchemaCtx::new(payload, schema_bytes);
        let schema_reader = <crate::schema::Schema as mmapstruct::traits::MmapRead>::read(
            schema_bytes,
            0..schema_bytes.len(),
        );
        let root_type = *schema_reader.root_type;
        let root = SchemaValue::new(0, payload.len() as u64, root_type);
        let root_layout = root_layout_from_ctx(&ctx, root_type, payload).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "root type has no layout facts (not a struct/enum root?)",
            )
        })?;
        Ok(Self {
            session,
            ctx,
            root,
            root_layout,
        })
    }

    /// The resolved root layout (struct, or the file's active enum variant).
    pub fn root_layout(&self) -> &RootLayout {
        &self.root_layout
    }

    /// Flush dirty mmap pages to the backing file.
    pub fn flush(&mut self) -> io::Result<()> {
        self.session.flush()
    }

    // ------------------------------------------------------------------
    // Operations
    // ------------------------------------------------------------------

    /// Append pod elements to a `Vec<T>` (`#[pod]`) field addressed by path.
    pub fn append_pod_vec<T: MmapPod>(&mut self, path: &str, add: &[T]) -> io::Result<()> {
        let target = self.resolve_path(path)?;
        let (block_off, elem_type_idx) = self.container_target(&target, path, "pod")?;
        let raw = self.count_word(block_off)?;
        let elems_off = block_off
            + if raw & BLOCK_FLAG_RESERVED != 0 {
                16
            } else {
                8
            };
        let _ = elem_type_idx; // alignment comes from T
        self.session
            .append_pod_vec_at(&self.root_layout, elems_off, add)
    }

    /// Append one serialized element to a `Vec<T>` (non-fixed-size elements)
    /// field addressed by path.
    pub fn append_indexed_raw(&mut self, path: &str, value_bytes: &[u8]) -> io::Result<()> {
        let target = self.resolve_path(path)?;
        let (block_off, elem_type_idx) = self.container_target(&target, path, "indexed")?;
        let align = self.ctx.type_align(elem_type_idx).max(1) as usize;
        self.session
            .append_indexed_bytes_at(&self.root_layout, block_off, align, value_bytes)
    }

    /// Append one serialized element to a `List<T>` field addressed by path.
    pub fn append_list_raw(&mut self, path: &str, value_bytes: &[u8]) -> io::Result<()> {
        let target = self.resolve_path(path)?;
        let (block_off, _) = self.container_target(&target, path, "list")?;
        self.session
            .append_list_bytes_at(&self.root_layout, block_off, value_bytes)
    }

    /// Append one serialized element to a reserved `Vec<T>` with
    /// `#[mmap(byref)]`, addressed by path.
    pub fn append_byref_vec_raw(&mut self, path: &str, value_bytes: &[u8]) -> io::Result<()> {
        let target = self.resolve_path(path)?;
        let (block_off, elem_type_idx) = self.container_target(&target, path, "indexed")?;
        // Elements are ByRef<T>: the appended VALUE bytes align by T.
        let t = self.ctx.type_reader(elem_type_idx);
        let inner = match t {
            TypeDefReader::ByRef { inner_type } => **inner_type,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("path {path:?}: byref append requires ByRef elements"),
                ))
            }
        };
        let align = self.ctx.type_align(inner).max(1) as usize;
        self.session
            .append_byref_vec_bytes_at(&self.root_layout, block_off, align, value_bytes)
    }

    /// Replace the value of a standalone `#[mmap(byref)]` field addressed by
    /// path (copy-on-write into the field's reserved gap).
    pub fn set_byref_raw(&mut self, path: &str, value_bytes: &[u8]) -> io::Result<()> {
        let target = self.resolve_path(path)?;
        let t = self.ctx.type_reader(target.field.type_index);
        let inner = match t {
            TypeDefReader::ByRef { inner_type } => **inner_type,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("path {path:?}: set_byref requires a ByRef field"),
                ))
            }
        };
        let slot_off = (target.parent_base + target.field.offset_in_fixed) as usize;
        let align = self.ctx.type_align(inner).max(1) as usize;
        self.session
            .set_byref_bytes_at(slot_off, align, value_bytes)
    }

    // ------------------------------------------------------------------
    // Resolution helpers
    // ------------------------------------------------------------------

    /// Validate that the target is an inline-dyn array field of the expected
    /// layout and return `(block_offset, element_type_index)`.
    fn container_target(
        &self,
        target: &ResolvedTarget,
        path: &str,
        want: &str,
    ) -> io::Result<(usize, u32)> {
        let t = self.ctx.type_reader(target.value.type_idx);
        let (elem_type, layout_kind) = match t {
            TypeDefReader::Array { elem_type, layout } => (
                **elem_type,
                match layout {
                    crate::schema::ArrayLayoutDefReader::List => "list",
                    crate::schema::ArrayLayoutDefReader::Indexed => "indexed",
                    crate::schema::ArrayLayoutDefReader::Pod { .. } => "pod",
                    crate::schema::ArrayLayoutDefReader::FixedPod { .. } => "fixedpod",
                    crate::schema::ArrayLayoutDefReader::Fixed { .. } => "fixed",
                },
            ),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("path {path:?}: field is not an array"),
                ))
            }
        };
        if layout_kind != want {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("path {path:?}: field is a {layout_kind} array, not {want}"),
            ));
        }
        Ok((target.value.mmap_begin as usize, elem_type))
    }

    /// The count/flag word at a container block start (live mmap).
    fn count_word(&self, block_off: usize) -> io::Result<u64> {
        let p = self.session.payload();
        if block_off + 8 > p.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "container block start is outside the payload",
            ));
        }
        Ok(u64::from_le_bytes(
            p[block_off..block_off + 8].try_into().unwrap(),
        ))
    }

    /// Resolve a dotted field path to its target.
    ///
    /// Walks struct fields and the active variant of enum fields. A `None`
    /// option in the middle of a path is an error (no data anchor).
    fn resolve_path(&self, path: &str) -> io::Result<ResolvedTarget> {
        let segments: Vec<&str> = path.split('.').collect();
        if segments.iter().any(|s| s.is_empty()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("empty path segment in {path:?}"),
            ));
        }
        let mut parent = self.root;
        let mut parent_base = 0u64;
        let mut field: Option<FieldDef> = None;
        for (i, seg) in segments.iter().enumerate() {
            // A `None` option cannot be traversed: no data anchor.
            if self.ctx.is_option_type(parent.type_idx) && parent.mmap_begin >= parent.mmap_end {
                let prev = if i > 0 { segments[i - 1] } else { "?" };
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "path {path:?}: segment {:?} holds a None option (no data to traverse)",
                        prev
                    ),
                ));
            }
            parent_base = parent.mmap_begin;
            let Some((field_idx, value)) = self.ctx.field_index_and_value(&parent, seg) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("path {path:?}: no such field {seg:?}"),
                ));
            };
            field = Some(self.field_def_of(&parent, field_idx)?);
            parent = value;
        }
        let field =
            field.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty path"))?;
        Ok(ResolvedTarget {
            parent_base,
            field,
            value: parent,
        })
    }

    /// The `FieldDef` of the field at `field_idx` within `parent` (enum-aware:
    /// for an enum parent the active variant's field list is used).
    fn field_def_of(&self, parent: &SchemaValue, field_idx: usize) -> io::Result<FieldDef> {
        let type_idx = self.ctx.unwrap_type(parent.type_idx);
        let t = self.ctx.type_reader(type_idx);
        let fields = match t {
            TypeDefReader::Struct { fields, .. } => fields,
            TypeDefReader::Enum { .. } => {
                let (variant, _) = self.ctx.enum_variant(parent).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "cannot resolve enum variant")
                })?;
                let vt = self.ctx.type_reader(variant);
                match vt {
                    TypeDefReader::Struct { fields, .. } => fields,
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "active enum variant is not a struct",
                        ))
                    }
                }
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "path traverses a non-struct value",
                ))
            }
        };
        let f = fields
            .get(field_idx)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad field index"))?;
        Ok(FieldDef {
            name: (*f.name).to_string(),
            type_index: *f.type_index,
            offset_in_fixed: *f.offset_in_fixed,
            inline_dyn_index: f.inline_dyn_index.copied(),
        })
    }
}

/// Root layout facts from the runtime schema: struct roots resolve directly,
/// enum roots read the discriminant (per the schema's `disc_size`) and use
/// the active variant's struct facts with `disc_prefix = 8`.
fn root_layout_from_ctx(
    ctx: &RuntimeSchemaCtx<'_>,
    root_type_idx: u32,
    payload: &[u8],
) -> Option<RootLayout> {
    let t = ctx.type_reader(root_type_idx);
    match t {
        TypeDefReader::Struct {
            name: _,
            fields,
            fixed_section_size,
            header_size,
            inline_dyn_count,
        } => Some(layout_from_struct_fields(
            ctx,
            fields,
            0,
            Some((
                **fixed_section_size as usize,
                **header_size as usize,
                **inline_dyn_count as usize,
            )),
        )),
        TypeDefReader::Enum {
            variant_type_indices,
            disc_size,
            ..
        } => {
            let disc_size = **disc_size as usize;
            if disc_size == 0 || disc_size > 8 || payload.len() < disc_size {
                return None;
            }
            let mut disc_bytes = [0u8; 8];
            disc_bytes[..disc_size].copy_from_slice(&payload[..disc_size]);
            let disc = u64::from_le_bytes(disc_bytes);
            let variant_idx = *variant_type_indices.get(disc as usize)?;
            let vt = ctx.type_reader(variant_idx);
            match vt {
                TypeDefReader::Struct {
                    name: _,
                    fields,
                    fixed_section_size,
                    header_size,
                    inline_dyn_count,
                } => Some(layout_from_struct_fields(
                    ctx,
                    fields,
                    8,
                    Some((
                        **fixed_section_size as usize,
                        **header_size as usize,
                        **inline_dyn_count as usize,
                    )),
                )),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Collect `byref_slots` from a struct's fields.
fn layout_from_struct_fields(
    ctx: &RuntimeSchemaCtx<'_>,
    fields: &mmapstruct::indexed_array::IndexedArrayReader<'_, FieldDef>,
    disc_prefix: usize,
    facts: Option<(usize, usize, usize)>,
) -> RootLayout {
    let mut byref_slots = Vec::new();
    for i in 0..fields.len() {
        let f = fields.get(i).unwrap();
        if ctx.type_reader(*f.type_index).is_byref_kind() {
            byref_slots.push(((*f.name).to_string(), *f.offset_in_fixed as usize));
        }
    }
    let (fs, hs, n) = facts.unwrap_or((0, 0, 0));
    RootLayout {
        disc_prefix,
        fixed_section_size: fs,
        header_size: hs,
        inline_dyn_count: n,
        byref_slots,
    }
}
