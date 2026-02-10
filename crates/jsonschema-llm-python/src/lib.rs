//! Python bindings for jsonschema-llm.
//!
//! Exposes `convert` and `rehydrate` via PyO3 for use from Python.
//! Uses `pythonize` for zero-copy dict ↔ serde_json::Value conversion.
//!
//! ## Python API Contract
//!
//! - Results are Python dicts with an `api_version: "1.0"` key.
//! - Errors raise `JsonSchemaLlmError` with `.code`, `.message`, `.path` attributes.
//! - Options use snake_case (`max_depth`, `recursion_limit`).

use pyo3::prelude::*;
use pyo3::types::PyDict;
use pythonize::{depythonize, pythonize};
use serde::Deserialize;

use jsonschema_llm_core::{
    ConvertError, ConvertOptions, PolymorphismStrategy, Target, API_VERSION,
};

// ---------------------------------------------------------------------------
// Python-local DTOs (Anti-Corruption Layer)
// ---------------------------------------------------------------------------

/// Python-local options DTO accepting snake_case from Python callers.
///
/// Core `ConvertOptions` uses kebab-case serde, but Python consumers
/// naturally use snake_case. This DTO bridges that gap.
///
/// NOTE: Keep in sync with `jsonschema_llm_core::ConvertOptions`.
/// Defaults are sourced from `ConvertOptions::default()` (single source of truth).
#[derive(Default, Deserialize)]
struct PyConvertOptions {
    target: Option<Target>,
    max_depth: Option<usize>,
    recursion_limit: Option<usize>,
    polymorphism: Option<PolymorphismStrategy>,
}

impl From<PyConvertOptions> for ConvertOptions {
    fn from(py_opts: PyConvertOptions) -> Self {
        let defaults = ConvertOptions::default();
        ConvertOptions {
            target: py_opts.target.unwrap_or(defaults.target),
            max_depth: py_opts.max_depth.unwrap_or(defaults.max_depth),
            recursion_limit: py_opts.recursion_limit.unwrap_or(defaults.recursion_limit),
            polymorphism: py_opts.polymorphism.unwrap_or(defaults.polymorphism),
        }
    }
}

// ---------------------------------------------------------------------------
// Custom Python Exception
// ---------------------------------------------------------------------------

pyo3::create_exception!(
    jsonschema_llm,
    JsonSchemaLlmError,
    pyo3::exceptions::PyException,
    "Error raised by jsonschema-llm operations.\n\nAttributes:\n    code: Stable error code (e.g. 'schema_error', 'json_parse_error')\n    message: Human-readable error description\n    path: JSON pointer path where the error occurred (None if not applicable)"
);

/// Convert a `ConvertError` into a `JsonSchemaLlmError` Python exception
/// with structured `.code`, `.message`, `.path` attributes.
fn to_py_error(py: Python<'_>, e: &ConvertError) -> PyErr {
    // Use serde serialization for consistent snake_case codes
    let code = serde_json::to_value(e.error_code())
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| format!("{:?}", e.error_code()).to_lowercase());
    let message = e.to_string();
    let path = e.path().map(String::from);

    let err = PyErr::new::<JsonSchemaLlmError, _>(message.clone());
    // Set structured attributes on the exception instance
    let _ = err.value(py).setattr("code", &code);
    let _ = err.value(py).setattr("message", &message);
    let _ = err.value(py).setattr("path", path.as_deref());
    err
}

/// Convert a pythonize deserialization error to a `JsonSchemaLlmError`.
fn to_py_deser_error(py: Python<'_>, e: pythonize::PythonizeError) -> PyErr {
    let message = e.to_string();
    let err = PyErr::new::<JsonSchemaLlmError, _>(message.clone());
    let _ = err.value(py).setattr("code", "json_parse_error");
    let _ = err.value(py).setattr("message", &message);
    let _ = err.value(py).setattr("path", py.None());
    err
}

// ---------------------------------------------------------------------------
// Public Python API
// ---------------------------------------------------------------------------

/// Convert a JSON Schema into an LLM-compatible structured output schema.
///
/// Args:
///     schema: A JSON Schema as a Python dict (or bool for trivial schemas).
///     options: Optional conversion options dict with keys:
///         - target: "openai-strict" | "gemini" | "claude"
///         - max_depth: Maximum traversal depth (default: 50)
///         - recursion_limit: Max recursive type inlining (default: 3)
///         - polymorphism: "any-of" | "flatten"
///
/// Returns:
///     A dict with keys: api_version, schema, codec
///
/// Raises:
///     JsonSchemaLlmError: If the schema is invalid or conversion fails.
#[pyfunction]
#[pyo3(signature = (schema, options=None))]
fn convert(
    py: Python<'_>,
    schema: &Bound<'_, PyAny>,
    options: Option<&Bound<'_, PyAny>>,
) -> PyResult<Py<PyAny>> {
    let schema: serde_json::Value = depythonize(schema).map_err(|e| to_py_deser_error(py, e))?;

    let opts: ConvertOptions = match options {
        Some(opts_obj) => {
            let py_opts: PyConvertOptions =
                depythonize(opts_obj).map_err(|e| to_py_deser_error(py, e))?;
            py_opts.into()
        }
        None => ConvertOptions::default(),
    };

    let result = jsonschema_llm_core::convert(&schema, &opts).map_err(|e| to_py_error(py, &e))?;

    // Build the result dict with api_version injected
    let dict = PyDict::new(py);
    dict.set_item("api_version", API_VERSION)?;
    dict.set_item(
        "schema",
        pythonize(py, &result.schema).map_err(|e| to_py_deser_error(py, e))?,
    )?;
    dict.set_item(
        "codec",
        pythonize(py, &result.codec).map_err(|e| to_py_deser_error(py, e))?,
    )?;
    Ok(dict.into())
}

/// Rehydrate LLM output back to the original schema shape.
///
/// Args:
///     data: The LLM-generated data as a Python dict.
///     codec: The codec sidecar from a prior convert() call.
///
/// Returns:
///     A dict with keys: api_version, data, warnings
///
/// Raises:
///     JsonSchemaLlmError: If rehydration fails.
#[pyfunction]
fn rehydrate(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
    codec: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let data: serde_json::Value = depythonize(data).map_err(|e| to_py_deser_error(py, e))?;
    let codec: jsonschema_llm_core::Codec =
        depythonize(codec).map_err(|e| to_py_deser_error(py, e))?;

    let result = jsonschema_llm_core::rehydrate(&data, &codec).map_err(|e| to_py_error(py, &e))?;

    let dict = PyDict::new(py);
    dict.set_item("api_version", API_VERSION)?;
    dict.set_item(
        "data",
        pythonize(py, &result.data).map_err(|e| to_py_deser_error(py, e))?,
    )?;
    dict.set_item(
        "warnings",
        pythonize(py, &result.warnings).map_err(|e| to_py_deser_error(py, e))?,
    )?;
    Ok(dict.into())
}

/// Python module definition.
#[pymodule]
fn jsonschema_llm(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(convert, m)?)?;
    m.add_function(wrap_pyfunction!(rehydrate, m)?)?;
    m.add(
        "JsonSchemaLlmError",
        m.py().get_type::<JsonSchemaLlmError>(),
    )?;
    Ok(())
}
