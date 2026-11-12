//! Safe ownership boundary for the vendored native SketchUp reader.
//!
//! Browser imports use the JavaScript reader in `benchup-io` instead. This crate
//! is empty on WebAssembly so workspace-wide checks need no native C/C++ tools.
#![cfg(not(target_arch = "wasm32"))]

use std::ffi::{CStr, c_char};

unsafe extern "C" {
    fn benchup_skp_parse(bytes: *const u8, len: usize, success: *mut i32) -> *mut c_char;
    fn benchup_skp_free(value: *mut c_char);
}

/// Reader failures remain structured so callers can localize them.
#[derive(Debug)]
pub enum ParseError {
    OutOfMemory,
    Reader(String),
}

/// Parse a SketchUp container into BenchUp model JSON.
///
/// The reader borrows `bytes` only during this call, catches C++ exceptions,
/// and returns a buffer that must be released with its matching allocator.
pub fn parse(bytes: &[u8]) -> Result<String, ParseError> {
    let mut success = 0;
    // SAFETY: The slice is valid for the duration of the call, and success is
    // writable. The C++ adapter never retains either pointer.
    let value = unsafe { benchup_skp_parse(bytes.as_ptr(), bytes.len(), &mut success) };
    if value.is_null() {
        return Err(ParseError::OutOfMemory);
    }
    // SAFETY: A non-null result is an owned, NUL-terminated UTF-8 buffer from
    // the adapter. Copy it before releasing it with the matching function.
    let text = unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned();
    unsafe { benchup_skp_free(value) };
    if success == 1 {
        Ok(text)
    } else {
        Err(ParseError::Reader(text))
    }
}
