//! Generates `MmapWrite` implementations.

use quote::quote;

use crate::utils::*;
use crate::category::*;
use crate::field_info::*;
use crate::layout::*;
use crate::write_helpers;

/// Compute maximum field alignment for a list of fields.
pub(crate) fn compute_max_field_align(fields: &[FieldInfo]) -> usize {
    let mut align = 1usize;
    for fi in fields {
        if is_pod_field(&fi.category) {
            let sz = known_primitive_size(&fi.ty).unwrap_or(8);
            let al = if sz <= 1 { 1 } else { sz.min(8) };
            if al > align {
                align = al;
            }
        } else if fi.is_byref {
            if 4 > align {
                align = 4;
            }
        }
    }
    align
}

/// Generate the full `impl MmapWrite` block.
pub(crate) fn generate_write_impl(
    name: &proc_macro2::Ident,
    fields: &[FieldInfo],
    layout: &StructLayout,
    inline_dyn_indices: &[usize],
    scoped: bool,
) -> proc_macro2::TokenStream {
    let field_ref = |id: &proc_macro2::Ident| quote! { &self.#id };
    let fixed_writes = write_helpers::generate_fixed_writes(fields, field_ref);
    let inline_dyn_writes = write_helpers::generate_inline_dyn_writes(fields, inline_dyn_indices, field_ref);
    let inline_dyn_count = layout.inline_dyn_count;

    let inner_body = if inline_dyn_count > 0 {
        quote! {
            let start = ctx.current_offset();

            #(#fixed_writes)*
            ctx.align_from(start, 8);

            let ot_start = ctx.current_offset();
            ctx.write_padding((#inline_dyn_count + 1) * 8);
            let vs = ctx.current_offset();
            let mut dyn_offsets: Vec<u64> = Vec::new();
            #(#inline_dyn_writes)*
            let total_dyn_size = (ctx.current_offset() - vs) as u64;
            for (i, &off) in dyn_offsets.iter().enumerate() {
                ctx.write_u64_le_at(off, ot_start + i * 8);
            }
            ctx.write_u64_le_at(total_dyn_size, ot_start + #inline_dyn_count * 8);
        }
    } else {
        quote! {
            let start = ctx.current_offset();

            #(#fixed_writes)*
            ctx.align_from(start, 8);
        }
    };

    let body = if scoped {
        let max_align = compute_max_field_align(fields);
        quote! {
            let _saved = ctx.scope_begin();
            #inner_body
            ctx.scope_end(_saved);
            ctx.align_to(#max_align);
        }
    } else {
        inner_body
    };

    quote! {
        impl ::mmapstruct::traits::MmapWrite for #name {
            fn write<'data>(&'data self, ctx: &mut ::mmapstruct::WriteContext<'data>) {
                #body
            }
        }
    }
}
