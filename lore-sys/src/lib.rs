//! Raw bindings to the Lore C API, pregenerated with bindgen. Purely
//! machine-generated, ergonomic wrappers live in the `lore-rs` crate.
#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
#![allow(clippy::missing_safety_doc)]

pub use libloading;

mod bindings;
pub use bindings::*;
