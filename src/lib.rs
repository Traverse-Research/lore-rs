#![doc = include_str!("../README.md")]
#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
#![allow(clippy::missing_safety_doc)]

pub use libloading;

// Pregenerated from lore/lore-capi/lore.h; see "Updating the bindings" in the
// README for the bindgen invocation that regenerates this file.
include!("bindings.rs");

impl Lore {
    /// Loads the Lore dynamic library from `path` and resolves all C API
    /// symbols.
    ///
    /// # Safety
    ///
    /// Loading a dynamic library executes its initialization routines; the
    /// caller must ensure `path` refers to a trusted Lore library that matches
    /// the interface version of the submodule's `lore-capi/lore.h`
    /// ([`LORE_INTERFACE_VERSION`]).
    pub unsafe fn load<P: AsRef<std::ffi::OsStr>>(path: P) -> Result<Self, libloading::Error> {
        Self::new(path)
    }

    /// Loads the Lore dynamic library by its platform-specific default name
    /// (`lore.dll`, `liblore.so` or `liblore.dylib`) from the system library
    /// search path.
    ///
    /// # Safety
    ///
    /// See [`Self::load`].
    pub unsafe fn load_default() -> Result<Self, libloading::Error> {
        Self::new(libloading::library_filename("lore"))
    }
}
