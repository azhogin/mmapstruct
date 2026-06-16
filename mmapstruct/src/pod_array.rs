use bytemuck::Pod;
use crate::WriteContext;

/// Write a pod array (contiguous elements). Used by proc-macro generated code.
pub fn write_pod_array<T: Pod>(items: &[T], ctx: &mut WriteContext) {
    ctx.write_raw(bytemuck::cast_slice(items));
}
