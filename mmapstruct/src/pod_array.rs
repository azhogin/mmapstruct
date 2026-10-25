use crate::traits::BLOCK_FLAG_RESERVED;
use crate::write_context::{Reserve, WriteContext};
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

/// [`write_pod_vec`] with an optional reserve.
///
/// Reserved blocks are self-describing:
/// `[count | BLOCK_FLAG_RESERVED][reserve word][elements][gap bytes]` where
/// the reserve word holds the spare data bytes in its high 32 bits (the low
/// 32 are unused for pod vecs — there is no index table). The gap lets a
/// patch session append elements in place, anywhere in the file.
pub fn write_pod_vec_r<T: Pod>(items: &[T], ctx: &mut WriteContext, reserve: Reserve) {
    if reserve.is_none() {
        write_pod_vec(items, ctx);
        return;
    }
    ctx.write_u64_le((items.len() as u64) | BLOCK_FLAG_RESERVED);
    ctx.write_u64_le((reserve.bytes << 32) | reserve.slots as u64);
    ctx.write_raw(bytemuck::cast_slice(items));
    if reserve.bytes > 0 {
        ctx.write_padding(reserve.bytes as usize);
    }
}
