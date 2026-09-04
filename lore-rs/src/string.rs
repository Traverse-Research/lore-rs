//! Lore's borrowed string and array types, and the crate's conventions for
//! filling them.

use lore_sys::{lore_string_array_t, lore_string_t};

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

/// An empty string is passed as a null pointer rather than as a pointer to
/// zero bytes, so that Lore cannot tell "unset" and "set to the empty string"
/// apart. Lore's header documents the empty string that way, and the other
/// SDKs do the same.
pub(crate) fn raw_str(string: &str) -> lore_string_t {
    if string.is_empty() {
        lore_string_t::EMPTY
    } else {
        lore_string_t::from_str(string)
    }
}

/// An empty `Vec` has a dangling pointer, which is not what "no strings"
/// should reach Lore as; this hands over a null pointer instead. Borrows
/// `strings`, which must outlive the call.
pub(crate) fn raw_str_array(strings: &[lore_string_t]) -> lore_string_array_t {
    if strings.is_empty() {
        lore_string_array_t::EMPTY
    } else {
        lore_string_array_t {
            ptr: strings.as_ptr(),
            count: strings.len(),
        }
    }
}
