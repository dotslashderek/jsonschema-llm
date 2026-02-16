//! # jsonschema-llm-wasi
//!
//! WASI universal binary for jsonschema-llm. Exposes `convert` and `rehydrate`
//! via a stable C ABI for consumption by any WASM runtime (wasmtime, wazero,
//! wasmer, etc.).
//!
//! ## Target
//!
//! Compile with: `cargo build --target wasm32-wasip1 --release -p jsonschema-llm-wasi`
//!
//! ## ABI Contract
//!
//! All exports are prefixed with `jsl_` to prevent symbol collisions.
//!
//! ### Memory Management
//!
//! - `jsl_alloc(len) → ptr` — allocate `len` bytes in guest linear memory
//! - `jsl_free(ptr, len)` — free a guest allocation
//!
//! ### Operations
//!
//! - `jsl_convert(schema_ptr, schema_len, opts_ptr, opts_len) → result_ptr`
//! - `jsl_rehydrate(data_ptr, data_len, codec_ptr, codec_len, schema_ptr, schema_len) → result_ptr`
//!
//! ### Result Protocol
//!
//! Both operations return a pointer to a `JslResult` struct in linear memory:
//!
//! ```text
//! #[repr(C)]
//! struct JslResult {
//!     status: u32,  // 0 = ok, 1 = error (bridge JSON), 2 = panic (captured)
//!     ptr: u32,     // pointer to UTF-8 JSON string
//!     len: u32,     // byte length of JSON string
//! }
//! ```
//!
//! The host must: read the result → copy the JSON bytes → call `jsl_result_free`.

use std::panic;

// ---------------------------------------------------------------------------
// Result protocol
// ---------------------------------------------------------------------------

/// Status codes for the JslResult protocol.
const STATUS_OK: u32 = 0;
const STATUS_ERROR: u32 = 1;
const STATUS_PANIC: u32 = 2;

/// C-ABI result struct returned from `jsl_convert` and `jsl_rehydrate`.
///
/// Layout: 12 bytes (3 × u32), `#[repr(C)]` for stable ABI.
#[repr(C)]
struct JslResult {
    status: u32,
    ptr: u32,
    len: u32,
}

impl JslResult {
    /// Allocate a `JslResult` on the heap and leak it, returning a raw pointer.
    /// The host must call `jsl_result_free` to reclaim this memory.
    fn into_raw(self) -> *mut JslResult {
        Box::into_raw(Box::new(self))
    }
}

/// Build a `JslResult` from a `Result<String, String>` returned by core bridge
/// functions.
fn result_from_bridge(outcome: Result<String, String>) -> *mut JslResult {
    let (status, payload) = match outcome {
        Ok(json) => (STATUS_OK, json),
        Err(json) => (STATUS_ERROR, json),
    };
    let bytes = payload.into_bytes();
    let len = bytes.len() as u32;
    let ptr = bytes.as_ptr() as u32;
    std::mem::forget(bytes);

    JslResult { status, ptr, len }.into_raw()
}

/// Build a `JslResult` for a captured panic.
fn result_from_panic(info: &str) -> *mut JslResult {
    let payload = serde_json::json!({
        "code": "internal_panic",
        "message": info,
        "path": null
    })
    .to_string();
    let bytes = payload.into_bytes();
    let len = bytes.len() as u32;
    let ptr = bytes.as_ptr() as u32;
    std::mem::forget(bytes);

    JslResult {
        status: STATUS_PANIC,
        ptr,
        len,
    }
    .into_raw()
}

// ---------------------------------------------------------------------------
// Memory management exports
// ---------------------------------------------------------------------------

/// Allocate `len` bytes in guest linear memory.
///
/// Returns a pointer to the allocated buffer. The host writes input data
/// (JSON strings) into this buffer before calling `jsl_convert`/`jsl_rehydrate`.
#[no_mangle]
pub extern "C" fn jsl_alloc(len: u32) -> u32 {
    let mut buf: Vec<u8> = Vec::with_capacity(len as usize);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr as u32
}

/// Free `len` bytes at `ptr` in guest linear memory.
///
/// The host calls this to release input buffers after the operation completes.
///
/// # Safety
///
/// - `ptr` must have been returned by [`jsl_alloc`] with the same `len`.
/// - Must not be called twice for the same `(ptr, len)` pair.
#[no_mangle]
pub unsafe extern "C" fn jsl_free(ptr: u32, len: u32) {
    if ptr == 0 || len == 0 {
        return;
    }
    let _ = Vec::from_raw_parts(ptr as *mut u8, len as usize, len as usize);
}

/// Free a `JslResult` and its associated payload string.
///
/// The host must call this after reading the result struct and copying the
/// JSON payload bytes. Frees both the JslResult struct and the payload buffer.
///
/// # Safety
///
/// - `result_ptr` must have been returned by [`jsl_convert`] or [`jsl_rehydrate`].
/// - Must not be called twice for the same pointer.
#[no_mangle]
pub unsafe extern "C" fn jsl_result_free(result_ptr: u32) {
    if result_ptr == 0 {
        return;
    }
    let result = Box::from_raw(result_ptr as *mut JslResult);
    if result.ptr != 0 && result.len != 0 {
        let _ = Vec::from_raw_parts(
            result.ptr as *mut u8,
            result.len as usize,
            result.len as usize,
        );
    }
    // Box dropped here, freeing the JslResult struct
}

// ---------------------------------------------------------------------------
// Operation exports
// ---------------------------------------------------------------------------

/// Convert a JSON Schema into an LLM-compatible structured output schema.
///
/// # Arguments
///
/// - `schema_ptr` / `schema_len`: JSON Schema document (UTF-8 bytes in linear memory)
/// - `opts_ptr` / `opts_len`: Conversion options JSON (UTF-8 bytes). Pass 0/0 for defaults.
///
/// # Returns
///
/// Pointer to a `JslResult` in linear memory. The host must read, copy, then
/// call `jsl_result_free`.
#[no_mangle]
pub extern "C" fn jsl_convert(
    schema_ptr: u32,
    schema_len: u32,
    opts_ptr: u32,
    opts_len: u32,
) -> u32 {
    let result = panic::catch_unwind(|| {
        let schema_str = unsafe {
            let slice = std::slice::from_raw_parts(schema_ptr as *const u8, schema_len as usize);
            std::str::from_utf8_unchecked(slice)
        };

        let opts_str = if opts_ptr == 0 || opts_len == 0 {
            "{}"
        } else {
            unsafe {
                let slice = std::slice::from_raw_parts(opts_ptr as *const u8, opts_len as usize);
                std::str::from_utf8_unchecked(slice)
            }
        };

        jsonschema_llm_core::convert_json(schema_str, opts_str)
    });

    match result {
        Ok(bridge_result) => result_from_bridge(bridge_result) as u32,
        Err(panic_info) => {
            let msg = panic_info
                .downcast_ref::<String>()
                .map(|s| s.as_str())
                .or_else(|| panic_info.downcast_ref::<&str>().copied())
                .unwrap_or("unknown panic");
            result_from_panic(msg) as u32
        }
    }
}

/// Rehydrate LLM output back to the original schema shape.
///
/// # Arguments
///
/// - `data_ptr` / `data_len`: LLM-generated JSON data (UTF-8 bytes)
/// - `codec_ptr` / `codec_len`: Codec sidecar JSON (UTF-8 bytes)
/// - `schema_ptr` / `schema_len`: Original JSON Schema (UTF-8 bytes)
///
/// # Returns
///
/// Pointer to a `JslResult` in linear memory.
#[no_mangle]
pub extern "C" fn jsl_rehydrate(
    data_ptr: u32,
    data_len: u32,
    codec_ptr: u32,
    codec_len: u32,
    schema_ptr: u32,
    schema_len: u32,
) -> u32 {
    let result = panic::catch_unwind(|| {
        let data_str = unsafe {
            let slice = std::slice::from_raw_parts(data_ptr as *const u8, data_len as usize);
            std::str::from_utf8_unchecked(slice)
        };
        let codec_str = unsafe {
            let slice = std::slice::from_raw_parts(codec_ptr as *const u8, codec_len as usize);
            std::str::from_utf8_unchecked(slice)
        };
        let schema_str = unsafe {
            let slice = std::slice::from_raw_parts(schema_ptr as *const u8, schema_len as usize);
            std::str::from_utf8_unchecked(slice)
        };

        jsonschema_llm_core::rehydrate_json(data_str, codec_str, schema_str)
    });

    match result {
        Ok(bridge_result) => result_from_bridge(bridge_result) as u32,
        Err(panic_info) => {
            let msg = panic_info
                .downcast_ref::<String>()
                .map(|s| s.as_str())
                .or_else(|| panic_info.downcast_ref::<&str>().copied())
                .unwrap_or("unknown panic");
            result_from_panic(msg) as u32
        }
    }
}
