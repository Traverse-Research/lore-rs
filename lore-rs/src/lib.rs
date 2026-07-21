pub use lore_sys;
pub use lore_sys::libloading;

use lore_sys::{lore_string_array_t, lore_string_t};

mod event;
pub use event::Event;

pub const EMPTY_STRING: lore_string_t = lore_string_t {
    string: std::ptr::null(),
    length: 0,
};

pub const EMPTY_STRING_ARRAY: lore_string_array_t = lore_string_array_t {
    ptr: std::ptr::null(),
    count: 0,
};

pub fn from_str(s: &str) -> lore_string_t {
    lore_string_t {
        string: s.as_ptr().cast(),
        length: s.len(),
    }
}

/// # Safety
///
/// The raw string must either have a null pointer or point to a buffer valid
/// for reads of `length` bytes for the duration of the borrow.
pub unsafe fn as_str(s: &lore_string_t) -> Result<&str, std::str::Utf8Error> {
    if s.string.is_null() {
        return Ok("");
    }
    std::str::from_utf8(unsafe { std::slice::from_raw_parts(s.string.cast::<u8>(), s.length) })
}

/// Owning wrapper around the loaded Lore library. The raw functions are
/// reachable through [`Deref`](std::ops::Deref).
pub struct Lore(lore_sys::Lore);

impl Lore {
    /// See [`lore_sys::Lore::new`].
    ///
    /// # Safety
    ///
    /// `path` must refer to a Lore dynamic library matching the version these
    /// bindings were generated for; loading it executes the library's
    /// initialization code, and mismatched function signatures are undefined
    /// behaviour on any later call.
    pub unsafe fn new<P: AsRef<std::ffi::OsStr>>(path: P) -> Result<Self, libloading::Error> {
        Ok(Self(unsafe { lore_sys::Lore::new(path)? }))
    }
}

impl std::ops::Deref for Lore {
    type Target = lore_sys::Lore;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for Lore {
    fn drop(&mut self) {
        // Stop lore's worker threads before the library unloads.
        let status = unsafe { self.0.lore_shutdown() };
        if status != 0 {
            log::error!(target: "lore", "lore_shutdown failed with status {status}");
        }
    }
}
