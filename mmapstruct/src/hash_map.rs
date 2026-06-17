//! HashMap with sorted keys, binary search lookup.
//!
//! Layout depends on whether key/value types are pod (stored as dense array)
//! or inlined (stored with offset table for variable-size elements).
//!
//! Selection between variants is done at derive time via
//! `#[mmap(key = pod, value = inlined)]` on the field, or auto-detected.

pub mod common;
#[allow(non_snake_case)]
pub mod inlinedK_inlinedV;
#[allow(non_snake_case)]
pub mod inlinedK_podV;
#[allow(non_snake_case)]
pub mod podK_inlinedV;
#[allow(non_snake_case)]
pub mod podK_podV;

pub use inlinedK_inlinedV::HashMapReader as HashMapReaderInlinedKInlinedV;
pub use inlinedK_podV::HashMapReader as HashMapReaderInlinedKPodV;
pub use podK_inlinedV::HashMapReader as HashMapReaderPodKInlinedV;
pub use podK_podV::HashMapReader as HashMapReaderPodKPodV;
