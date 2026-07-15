#![doc = include_str!("../README.md")]
#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
#![allow(clippy::missing_safety_doc)]

pub use libloading;

// Pregenerated from lore/lore-capi/lore.h; see "Updating the bindings" in the
// README for the bindgen invocation that regenerates this file.
mod bindings;
pub use bindings::*;

mod capture;
pub use capture::Event;

/// Returns the platform-specific file name of the Lore dynamic library:
/// `lore.dll` on Windows, `liblore.so` on Linux and `liblore.dylib` on macOS.
///
/// Join this onto a directory to build the path for [`Lore::load`], or pass it
/// as-is to resolve the library from the system search path.
pub fn library_filename() -> std::ffi::OsString {
    libloading::library_filename("lore")
}

/// Path to the prebuilt lore dynamic library checked into this repository,
/// built by the `generator` tool from the same `lore` submodule revision the
/// bindings were generated from. Only the Windows `lore.dll` is checked in for now.
pub fn prebuilt_library_path() -> std::path::PathBuf {
    std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/bin")).join(library_filename())
}

/// Path to the `.pdb` matching the prebuilt `lore.dll`; debuggers reject
/// symbols from a different build than the library.
pub fn prebuilt_pdb_path() -> std::path::PathBuf {
    std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/bin")).join("lore.pdb")
}

impl lore_string_array_t {
    pub const EMPTY: Self = Self {
        ptr: std::ptr::null(),
        count: 0,
    };
}

impl lore_string_t {
    pub const EMPTY: Self = Self {
        string: std::ptr::null(),
        length: 0,
    };

    /// Borrows `s`; lore copies the bytes before the call it is passed to
    /// returns, and reads them by length, so no NUL terminator is needed.
    pub fn new(s: &str) -> Self {
        Self {
            string: s.as_ptr().cast(),
            length: s.len(),
        }
    }

    /// The borrowed text.
    ///
    /// # Safety
    ///
    /// The string a lore event delivers is only valid while the callback that
    /// delivered it runs.
    pub unsafe fn as_str(&self) -> &str {
        if self.string.is_null() {
            ""
        } else {
            std::str::from_utf8(unsafe {
                std::slice::from_raw_parts(self.string.cast::<u8>(), self.length)
            })
            .unwrap_or("<non-utf8 lore string>")
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

impl Drop for Lore {
    fn drop(&mut self) {
        // Stop lore's worker threads before the library unloads; they would
        // otherwise keep the process alive running unloaded code
        let status = unsafe { self.lore_shutdown() };
        if status != 0 {
            log::error!(target: "lore", "lore_shutdown failed with status {status}");
        }
    }
}
