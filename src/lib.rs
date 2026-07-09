#![doc = include_str!("../README.md")]
#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
#![allow(clippy::missing_safety_doc)]

pub use libloading;

// Pregenerated from lore/lore-capi/lore.h; see "Updating the bindings" in the
// README for the bindgen invocation that regenerates this file.
mod bindings;
pub use bindings::*;

mod event;
pub use event::Event;

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

    pub fn new(s: &str) -> Self {
        Self {
            string: s.as_ptr().cast(),
            length: s.len(),
        }
    }

    pub unsafe fn as_str(&self) -> Result<&str, std::str::Utf8Error> {
        if self.string.is_null() {
            return Ok("");
        }
        std::str::from_utf8(unsafe {
            std::slice::from_raw_parts(self.string.cast::<u8>(), self.length)
        })
    }
}

impl Drop for Lore {
    fn drop(&mut self) {
        // Stop lore's worker threads before the library unloads.
        let status = unsafe { self.lore_shutdown() };
        if status != 0 {
            log::error!(target: "lore", "lore_shutdown failed with status {status}");
        }
    }
}
