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
//!     status: u32,  // 0 = ok, 1 = error (bridge JSON)
//!     ptr: u32,     // pointer to UTF-8 JSON string
//!     len: u32,     // byte length of JSON string
//! }
//! ```
//!
//! The host must: read the result → copy the JSON bytes → call `jsl_result_free`.
//!
//! ### Panic Behavior
//!
//! This module compiles with `panic = "abort"` (the `wasm32-wasip1` default).
//! Panics will trap the WASM module — hosts should handle WASM traps at the
//! runtime level. All internal code paths return errors rather than panicking.

// ---------------------------------------------------------------------------
// Result protocol
// ---------------------------------------------------------------------------

/// Status codes for the JslResult protocol.
const STATUS_OK: u32 = 0;
const STATUS_ERROR: u32 = 1;

/// Current ABI version. Bump on breaking changes to the export contract.
const ABI_VERSION: u32 = 1;

/// Return the ABI version of this WASI binary.
///
/// Hosts should call this at init time and verify the returned value matches
/// their expected version. A mismatch indicates a binary/wrapper skew.
#[no_mangle]
pub extern "C" fn jsl_abi_version() -> u32 {
    ABI_VERSION
}

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

/// Leak a string as a `(ptr, len)` pair using a boxed slice.
///
/// Using `into_boxed_slice()` ensures capacity == len, so the deallocation
/// in `jsl_result_free` uses the correct layout.
fn leak_string(s: String) -> (u32, u32) {
    let boxed: Box<[u8]> = s.into_bytes().into_boxed_slice();
    let len = boxed.len() as u32;
    let ptr = Box::into_raw(boxed) as *mut u8 as u32;
    (ptr, len)
}

/// Build a `JslResult` from a `Result<String, String>` returned by core bridge
/// functions.
fn result_from_bridge(outcome: Result<String, String>) -> *mut JslResult {
    let (status, payload) = match outcome {
        Ok(json) => (STATUS_OK, json),
        Err(json) => (STATUS_ERROR, json),
    };
    let (ptr, len) = leak_string(payload);
    JslResult { status, ptr, len }.into_raw()
}

/// Build an error `JslResult` from a UTF-8 decoding failure or similar
/// pre-bridge error.
fn result_from_input_error(code: &str, message: &str) -> *mut JslResult {
    let payload = serde_json::json!({
        "code": code,
        "message": message,
        "path": null
    })
    .to_string();
    let (ptr, len) = leak_string(payload);
    JslResult {
        status: STATUS_ERROR,
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
    // jsl_alloc uses Vec::with_capacity which has capacity == len,
    // so reconstructing with capacity == len is correct.
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
        // Payload was created via `leak_string` which uses `into_boxed_slice()`,
        // guaranteeing capacity == len. Reconstruct as boxed slice for dealloc.
        let _ = Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            result.ptr as *mut u8,
            result.len as usize,
        ));
    }
    // Box dropped here, freeing the JslResult struct
}

/// Read a UTF-8 string from guest linear memory with checked decoding.
///
/// Returns `Ok(String)` on valid UTF-8, or `Err(JslResult ptr)` on invalid input.
///
/// # Null / zero-length handling
///
/// - `len == 0` → returns `Ok("")` (empty string, no memory access)
/// - `ptr == 0 && len > 0` → returns `Err` with `invalid_pointer` code
///
/// # Safety
///
/// `ptr` must point to `len` valid bytes in the WASM linear memory.
unsafe fn read_guest_str(ptr: u32, len: u32) -> Result<String, *mut JslResult> {
    if len == 0 {
        return Ok(String::new());
    }
    if ptr == 0 {
        return Err(result_from_input_error(
            "invalid_pointer",
            "null pointer with non-zero length",
        ));
    }
    let slice = std::slice::from_raw_parts(ptr as *const u8, len as usize);
    std::str::from_utf8(slice)
        .map(|s| s.to_owned())
        .map_err(|e| {
            result_from_input_error(
                "invalid_utf8",
                &format!("invalid UTF-8 at byte offset {}", e.valid_up_to()),
            )
        })
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
    // --- Read schema (checked UTF-8) ---
    let schema_str = match unsafe { read_guest_str(schema_ptr, schema_len) } {
        Ok(s) => s,
        Err(err_ptr) => return err_ptr as u32,
    };

    // --- Read options (checked UTF-8, with default fallback) ---
    let default_opts = serde_json::to_string(&jsonschema_llm_core::ConvertOptions::default())
        .expect("default options serialize");
    let effective_opts: String = if opts_ptr == 0 || opts_len == 0 {
        default_opts
    } else {
        let opts_str = match unsafe { read_guest_str(opts_ptr, opts_len) } {
            Ok(s) => s,
            Err(err_ptr) => return err_ptr as u32,
        };

        // Detect empty objects (e.g., "{}", "{ }", "{\n}") by parsing
        let is_empty = serde_json::from_str::<serde_json::Value>(&opts_str)
            .map(|v| matches!(&v, serde_json::Value::Object(m) if m.is_empty()))
            .unwrap_or(false);

        if is_empty {
            default_opts
        } else {
            opts_str
        }
    };

    result_from_bridge(jsonschema_llm_core::convert_json(
        &schema_str,
        &effective_opts,
    )) as u32
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
    // --- Read all three inputs (checked UTF-8) ---
    let data_str = match unsafe { read_guest_str(data_ptr, data_len) } {
        Ok(s) => s,
        Err(err_ptr) => return err_ptr as u32,
    };
    let codec_str = match unsafe { read_guest_str(codec_ptr, codec_len) } {
        Ok(s) => s,
        Err(err_ptr) => return err_ptr as u32,
    };
    let schema_str = match unsafe { read_guest_str(schema_ptr, schema_len) } {
        Ok(s) => s,
        Err(err_ptr) => return err_ptr as u32,
    };

    result_from_bridge(jsonschema_llm_core::rehydrate_json(
        &data_str,
        &codec_str,
        &schema_str,
    )) as u32
}

/// List all extractable component JSON Pointers in a schema.
///
/// # Arguments
///
/// - `schema_ptr` / `schema_len`: JSON Schema document (UTF-8 bytes in linear memory)
///
/// # Returns
///
/// Pointer to a `JslResult`. `status=0` payload: `{"apiVersion":"1.0","components":[...]}`.
#[no_mangle]
pub extern "C" fn jsl_list_components(schema_ptr: u32, schema_len: u32) -> u32 {
    let schema_str = match unsafe { read_guest_str(schema_ptr, schema_len) } {
        Ok(s) => s,
        Err(err_ptr) => return err_ptr as u32,
    };
    result_from_bridge(jsonschema_llm_core::list_components_json(&schema_str)) as u32
}

/// Extract a single component from a schema by JSON Pointer.
///
/// # Arguments
///
/// - `schema_ptr` / `schema_len`: JSON Schema document (UTF-8 bytes)
/// - `pointer_ptr` / `pointer_len`: RFC 6901 JSON Pointer string (UTF-8 bytes)
/// - `opts_ptr` / `opts_len`: Extraction options JSON (UTF-8 bytes). Pass 0/0 for defaults.
///
/// # Returns
///
/// Pointer to a `JslResult`. `status=0` payload: `{"apiVersion":"1.0","schema":{...},...}`.
#[no_mangle]
pub extern "C" fn jsl_extract_component(
    schema_ptr: u32,
    schema_len: u32,
    pointer_ptr: u32,
    pointer_len: u32,
    opts_ptr: u32,
    opts_len: u32,
) -> u32 {
    let schema_str = match unsafe { read_guest_str(schema_ptr, schema_len) } {
        Ok(s) => s,
        Err(err_ptr) => return err_ptr as u32,
    };
    let pointer_str = match unsafe { read_guest_str(pointer_ptr, pointer_len) } {
        Ok(s) => s,
        Err(err_ptr) => return err_ptr as u32,
    };
    let default_extract_opts =
        serde_json::to_string(&jsonschema_llm_core::ExtractOptions::default())
            .expect("default extract options serialize");
    let effective_opts = if opts_ptr == 0 || opts_len == 0 {
        default_extract_opts
    } else {
        let opts_str = match unsafe { read_guest_str(opts_ptr, opts_len) } {
            Ok(s) => s,
            Err(err_ptr) => return err_ptr as u32,
        };
        let is_empty = serde_json::from_str::<serde_json::Value>(&opts_str)
            .map(|v| matches!(&v, serde_json::Value::Object(m) if m.is_empty()))
            .unwrap_or(false);
        if is_empty {
            default_extract_opts
        } else {
            opts_str
        }
    };
    result_from_bridge(jsonschema_llm_core::extract_component_json(
        &schema_str,
        &pointer_str,
        &effective_opts,
    )) as u32
}

/// Convert a JSON Schema and all its discoverable components in one call.
///
/// # Arguments
///
/// - `schema_ptr` / `schema_len`: JSON Schema document (UTF-8 bytes)
/// - `conv_opts_ptr` / `conv_opts_len`: Conversion options JSON (UTF-8 bytes). Pass 0/0 for defaults.
/// - `ext_opts_ptr` / `ext_opts_len`: Extraction options JSON (UTF-8 bytes). Pass 0/0 for defaults.
///
/// # Returns
///
/// Pointer to a `JslResult`. `status=0` payload: `{"apiVersion":"1.0","full":{...},"components":[...]}`.
#[no_mangle]
pub extern "C" fn jsl_convert_all_components(
    schema_ptr: u32,
    schema_len: u32,
    conv_opts_ptr: u32,
    conv_opts_len: u32,
    ext_opts_ptr: u32,
    ext_opts_len: u32,
) -> u32 {
    let schema_str = match unsafe { read_guest_str(schema_ptr, schema_len) } {
        Ok(s) => s,
        Err(err_ptr) => return err_ptr as u32,
    };
    let default_conv_opts = serde_json::to_string(&jsonschema_llm_core::ConvertOptions::default())
        .expect("default convert options serialize");
    let effective_conv_opts = if conv_opts_ptr == 0 || conv_opts_len == 0 {
        default_conv_opts
    } else {
        let opts_str = match unsafe { read_guest_str(conv_opts_ptr, conv_opts_len) } {
            Ok(s) => s,
            Err(err_ptr) => return err_ptr as u32,
        };
        let is_empty = serde_json::from_str::<serde_json::Value>(&opts_str)
            .map(|v| matches!(&v, serde_json::Value::Object(m) if m.is_empty()))
            .unwrap_or(false);
        if is_empty {
            default_conv_opts
        } else {
            opts_str
        }
    };
    let default_ext_opts = serde_json::to_string(&jsonschema_llm_core::ExtractOptions::default())
        .expect("default extract options serialize");
    let effective_ext_opts = if ext_opts_ptr == 0 || ext_opts_len == 0 {
        default_ext_opts
    } else {
        let opts_str = match unsafe { read_guest_str(ext_opts_ptr, ext_opts_len) } {
            Ok(s) => s,
            Err(err_ptr) => return err_ptr as u32,
        };
        let is_empty = serde_json::from_str::<serde_json::Value>(&opts_str)
            .map(|v| matches!(&v, serde_json::Value::Object(m) if m.is_empty()))
            .unwrap_or(false);
        if is_empty {
            default_ext_opts
        } else {
            opts_str
        }
    };
    result_from_bridge(jsonschema_llm_core::convert_all_components_json(
        &schema_str,
        &effective_conv_opts,
        &effective_ext_opts,
    )) as u32
}
