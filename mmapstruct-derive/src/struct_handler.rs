//! Generates `MmapRead`/`MmapWrite`/`MmapFromDefault` impls for structs.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::DeriveInput;

use crate::attrs::*;
use crate::field_info::*;
use crate::layout::*;
use crate::reader::*;
use crate::writer::*;
use crate::from_default::*;
use crate::layout_hash::*;
#[cfg(feature = "dyn")]
use crate::schema::generate_schema_impl;

pub(crate) fn handle_struct(input: &DeriveInput, ds: &syn::DataStruct) -> TokenStream {
    let name = &input.ident;
    let reader_name = format_ident!("{}Reader", name);
    let is_scoped = has_struct_scope_attr(&input.attrs);

    let analysis = analyze_fields(&ds.fields);
    let all_fields = &analysis.fields;
    let has_dynamic = analysis.has_dynamic;

    if !has_dynamic {
        let layout = compute_layout(&all_fields);
        let fixed_layout_consts =
            generate_fixed_layout_consts(name, &all_fields, layout.inline_dyn_count);

        let pod_impl = quote! { impl ::mmapstruct::traits::MmapPod for #name {} };
        let layout_hash_impl = generate_layout_hash_impl(name, &all_fields);

        let schema_impl = {
            #[cfg(feature = "dyn")]
            {
                generate_schema_impl(name, &all_fields)
            }
            #[cfg(not(feature = "dyn"))]
            {
                quote! {}
            }
        };

        return (quote! {
            #fixed_layout_consts
            #pod_impl
            #layout_hash_impl
            #schema_impl
        })
        .into();
    }

    let layout = compute_layout(&all_fields);
    let inline_dyn_indices = collect_inline_dyn_indices(&all_fields);

    let fixed_layout_consts =
        generate_fixed_layout_consts(name, &all_fields, layout.inline_dyn_count);

    let reader_struct = generate_reader_struct(&reader_name, &all_fields);
    let read_impl = generate_read_impl(
        name,
        &reader_name,
        &all_fields,
        &layout,
        &inline_dyn_indices,
    );
    let write_impl =
        generate_write_impl(name, &all_fields, &layout, &inline_dyn_indices, is_scoped);
    let from_impl = generate_from_impl(name, &reader_name, &all_fields);
    let reader_owned_impl = generate_reader_owned_impl(name, &reader_name, &all_fields);
    let reader_ord_impl = generate_reader_ord_impl(name, &reader_name, &all_fields);
    let default_impl = generate_default_impl(name, &all_fields);
    let layout_hash_impl = generate_layout_hash_impl(name, &all_fields);

    let schema_impl = {
        #[cfg(feature = "dyn")]
        {
            generate_schema_impl(name, &all_fields)
        }
        #[cfg(not(feature = "dyn"))]
        {
            quote! {}
        }
    };

    (quote! {
        #fixed_layout_consts
        #layout_hash_impl
        #reader_struct
        #read_impl
        #write_impl
        #from_impl
        #reader_owned_impl
        #reader_ord_impl
        #default_impl
        #schema_impl
    })
    .into()
}
