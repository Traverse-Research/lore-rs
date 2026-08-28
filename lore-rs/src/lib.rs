pub use lore_sys;
pub use lore_sys::libloading;

use lore_sys::{lore_string_array_t, lore_string_t};

mod call;
pub use call::{
    call_with_callback, file_info, repository_info, revision_tree_close, revision_tree_load,
    storage_close, storage_get, storage_open, FileInfoArgs, GlobalArgs, LoreError,
    RepositoryInfoArgs, StorageGetArgs, StorageGetItem, StorageOpenArgs,
};

mod event;
pub use event::{log_event, Event};

pub fn error_code_name(code: lore_sys::lore_error_code_t) -> &'static str {
    match code {
        lore_sys::LORE_ERROR_CODE_NONE => "none",
        lore_sys::LORE_ERROR_CODE_INVALID_ARGUMENTS => "invalid arguments",
        lore_sys::LORE_ERROR_CODE_ADDRESS_NOT_FOUND => "address not found",
        lore_sys::LORE_ERROR_CODE_INTERNAL => "internal error",
        lore_sys::LORE_ERROR_CODE_SLOW_DOWN => "slow down (rate limited)",
        _ => "unknown error code",
    }
}

pub trait LoreStringExt {
    const EMPTY: Self;

    /// The returned struct borrows `s` through a raw pointer without a
    /// lifetime; `s` must stay alive for as long as the result is used.
    fn from_str(s: &str) -> Self;

    /// A null pointer is deliberately conflated with the empty string.
    ///
    /// # Safety
    ///
    /// The string must either have a null pointer or point to a buffer valid
    /// for reads of `length` bytes for the duration of the borrow.
    unsafe fn try_to_str(&self) -> Result<&str, std::str::Utf8Error>;
}

impl LoreStringExt for lore_string_t {
    const EMPTY: Self = Self {
        string: std::ptr::null(),
        length: 0,
    };

    fn from_str(s: &str) -> Self {
        Self {
            string: s.as_ptr().cast(),
            length: s.len(),
        }
    }

    unsafe fn try_to_str(&self) -> Result<&str, std::str::Utf8Error> {
        if self.string.is_null() {
            return Ok("");
        }
        std::str::from_utf8(unsafe {
            std::slice::from_raw_parts(self.string.cast::<u8>(), self.length)
        })
    }
}

pub trait LoreStringArrayExt {
    const EMPTY: Self;
}

impl LoreStringArrayExt for lore_string_array_t {
    const EMPTY: Self = Self {
        ptr: std::ptr::null(),
        count: 0,
    };
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
    ///
    /// Create at most one `Lore` per process: instances share the loaded
    /// library, and dropping one shuts it down for all of them.
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
