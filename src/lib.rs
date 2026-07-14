#![doc = include_str!("../README.md")]
#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
#![allow(clippy::missing_safety_doc)]

pub use libloading;

// Pregenerated from lore/lore-capi/lore.h; see "Updating the bindings" in the
// README for the bindgen invocation that regenerates this file.
include!("bindings.rs");

mod capture;
pub use capture::{capture, CapturedEvents};

/// Returns the platform-specific file name of the Lore dynamic library:
/// `lore.dll` on Windows, `liblore.so` on Linux and `liblore.dylib` on macOS.
///
/// Join this onto a directory to build the path for [`Lore::load`], or pass it
/// as-is to resolve the library from the system search path.
pub fn library_filename() -> std::ffi::OsString {
    libloading::library_filename("lore")
}

/// Path to the prebuilt `lore.dll` checked into this repository, built by the
/// `generator` tool from the same `lore` submodule revision the bindings were
/// generated from. Windows-only for now.
pub const DLL_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/bin/lore.dll");

pub const PDB_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/bin/lore.pdb");

impl lore_string_t {
    pub const EMPTY: Self = Self {
        string: std::ptr::null(),
        length: 0,
    };

    pub fn new(s: &std::ffi::CStr) -> Self {
        Self {
            string: s.as_ptr(),
            length: s.to_bytes().len(),
        }
    }
}

impl Lore {
    /// Loads the Lore dynamic library from `path` and resolves all C API
    /// symbols.
    pub unsafe fn load<P: AsRef<std::ffi::OsStr>>(path: P) -> Result<Self, libloading::Error> {
        Self::new(path)
    }
}
