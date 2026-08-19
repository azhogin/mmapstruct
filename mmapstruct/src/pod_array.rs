use crate::WriteContext;
use bytemuck::Pod;

/// Write a pod array (contiguous elements). Used by proc-macro generated code.
pub fn write_pod_array<T: Pod>(items: &[T], ctx: &mut WriteContext) {
    ctx.write_raw(bytemuck::cast_slice(items));
}

/// Write a pod `Vec` with an internal element-count prefix (`u64`) followed
/// by the contiguous element bytes.
///
/// The count prefix makes the field's length self-describing, so it no longer
/// depends on the surrounding offset table. This avoids the bug where
/// alignment padding inserted before the *next* pod `Vec` field leaked into
/// the previous field's data range (see bugs/pod-Vec-of-arrays-with-pod-Vec-problem.md).
pub fn write_pod_vec<T: Pod>(items: &[T], ctx: &mut WriteContext) {
    ctx.write_u64_le(items.len() as u64);
    ctx.write_raw(bytemuck::cast_slice(items));
}
