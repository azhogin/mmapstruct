use proc_macro::TokenStream;
use syn::{parse_macro_input, Data, DeriveInput};

mod attrs;
mod category;
mod enum_handler;
mod field_info;
mod from_default;
mod layout;
mod layout_hash;
mod reader;
#[cfg(feature = "dyn")]
mod schema;
mod struct_handler;
mod utils;
mod write_helpers;
mod writer;

#[proc_macro_derive(MmapStruct, attributes(mmap, pod, fixed))]
pub fn derive_mmap_struct(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match &input.data {
        Data::Struct(ds) => struct_handler::handle_struct(&input, ds),
        Data::Enum(de) => enum_handler::handle_enum(&input, de),
        _ => panic!("MmapStruct only supports structs and enums"),
    }
}
