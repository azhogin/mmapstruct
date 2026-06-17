use proc_macro::TokenStream;
use syn::{parse_macro_input, Data, DeriveInput};

mod utils;
mod attrs;
mod category;
mod field_info;
mod layout;
mod reader;
mod writer;
mod from_default;
mod layout_hash;
#[cfg(feature = "dyn")]
mod schema;
mod write_helpers;
mod struct_handler;
mod enum_handler;

#[proc_macro_derive(MmapStruct, attributes(mmap, pod, fixed))]
pub fn derive_mmap_struct(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match &input.data {
        Data::Struct(ds) => struct_handler::handle_struct(&input, ds),
        Data::Enum(de) => enum_handler::handle_enum(&input, de),
        _ => panic!("MmapStruct only supports structs and enums"),
    }
}
