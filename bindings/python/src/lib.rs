// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-binding-python` — Python delivery wrapper over the engine ABI.
//!
//! The crate exposes a small `PyO3` module for the `invoicekit` Python package
//! while keeping the byte-level behavior delegated to `invoicekit-engine`.
//! Python callers use the same Engine ABI JSON contract and golden fixtures as
//! every other binding.

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyModule};

const STATUS_OK: u32 = 0;
const STATUS_ERROR: u32 = 1;
const STATUS_INVALID_HANDLE: u32 = 2;

/// Process an Engine ABI JSON request through the Python binding wrapper.
///
/// # Examples
///
/// ```
/// let response = invoicekit_binding_python::process_engine_abi_json(
///     br#"{"abi_version":1,"operation":"unknown","payload":{}}"#,
/// );
/// assert!(std::str::from_utf8(&response).unwrap().contains(r#""status":"error""#));
/// ```
#[must_use]
pub fn process_engine_abi_json(request_bytes: &[u8]) -> Vec<u8> {
    invoicekit_engine::process_abi_json(request_bytes)
}

/// Return the Engine ABI version exposed by the Python package.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_binding_python::engine_abi_version(), 1);
/// ```
#[must_use]
pub const fn engine_abi_version() -> u32 {
    invoicekit_engine::ENGINE_ABI_VERSION
}

/// Python-owned Engine ABI result handle.
///
/// This mirrors the C ABI result-handle contract but lets Python own the
/// response bytes safely. Calling `EngineResult.free()` marks the handle as
/// invalid; Python garbage collection also releases it normally.
#[pyclass(name = "EngineResult")]
#[derive(Debug)]
pub struct EngineResult {
    status: u32,
    bytes: Vec<u8>,
    freed: bool,
}

impl EngineResult {
    fn new(bytes: Vec<u8>) -> Self {
        let status = if is_success_response(&bytes) {
            STATUS_OK
        } else {
            STATUS_ERROR
        };
        Self {
            status,
            bytes,
            freed: false,
        }
    }

    fn ensure_live(&self) -> PyResult<()> {
        if self.freed {
            Err(pyo3::exceptions::PyRuntimeError::new_err(
                "EngineResult has been freed",
            ))
        } else {
            Ok(())
        }
    }
}

#[pymethods]
impl EngineResult {
    /// Numeric status code: 0 for `ok`, 1 for engine error, 2 after free.
    #[getter]
    fn status(&self) -> u32 {
        if self.freed {
            STATUS_INVALID_HANDLE
        } else {
            self.status
        }
    }

    /// Response bytes copied out of the engine result.
    #[getter]
    fn bytes(&self, py: Python<'_>) -> PyResult<Py<PyBytes>> {
        self.ensure_live()?;
        Ok(PyBytes::new(py, &self.bytes).unbind())
    }

    /// Response byte length.
    #[getter]
    fn len(&self) -> PyResult<usize> {
        self.ensure_live()?;
        Ok(self.bytes.len())
    }

    /// Mark the result handle as freed.
    fn free(&mut self) {
        self.bytes.clear();
        self.freed = true;
        self.status = STATUS_INVALID_HANDLE;
    }

    fn __len__(&self) -> PyResult<usize> {
        self.len()
    }

    fn __repr__(&self) -> String {
        format!(
            "EngineResult(status={}, len={}, freed={})",
            self.status(),
            self.bytes.len(),
            self.freed
        )
    }
}

/// Python wrapper for `invoicekit_engine_abi_version`.
#[pyfunction]
fn py_engine_abi_version() -> u32 {
    engine_abi_version()
}

/// Python wrapper for `invoicekit_engine_process_json`.
#[pyfunction]
fn py_engine_process_json(request_bytes: &[u8]) -> EngineResult {
    EngineResult::new(process_engine_abi_json(request_bytes))
}

/// Python helper that returns only the Engine ABI response bytes.
#[pyfunction]
fn py_process_engine_abi_json(py: Python<'_>, request_bytes: &[u8]) -> Py<PyBytes> {
    PyBytes::new(py, &process_engine_abi_json(request_bytes)).unbind()
}

/// Python wrapper for `invoicekit_engine_result_status`.
#[pyfunction]
fn py_engine_result_status(result: Option<PyRef<'_, EngineResult>>) -> u32 {
    result.map_or(STATUS_INVALID_HANDLE, |result| result.status())
}

/// Python wrapper for `invoicekit_engine_result_bytes`.
#[pyfunction]
fn py_engine_result_bytes(
    py: Python<'_>,
    result: Option<PyRef<'_, EngineResult>>,
) -> PyResult<Py<PyBytes>> {
    result.map_or_else(
        || Ok(PyBytes::new(py, b"").unbind()),
        |result| result.bytes(py),
    )
}

/// Python wrapper for `invoicekit_engine_result_len`.
#[pyfunction]
fn py_engine_result_len(result: Option<PyRef<'_, EngineResult>>) -> PyResult<usize> {
    result.map_or_else(|| Ok(0), |result| result.len())
}

/// Python wrapper for `invoicekit_engine_result_free`.
#[pyfunction]
fn py_engine_result_free(result: Option<PyRefMut<'_, EngineResult>>) {
    if let Some(mut result) = result {
        result.free();
    }
}

/// Native module for the `invoicekit` Python package.
#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("ENGINE_ABI_VERSION", engine_abi_version())?;
    m.add_class::<EngineResult>()?;
    m.add_function(wrap_pyfunction!(py_engine_abi_version, m)?)?;
    m.add_function(wrap_pyfunction!(py_engine_process_json, m)?)?;
    m.add_function(wrap_pyfunction!(py_process_engine_abi_json, m)?)?;
    m.add_function(wrap_pyfunction!(py_engine_result_status, m)?)?;
    m.add_function(wrap_pyfunction!(py_engine_result_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(py_engine_result_len, m)?)?;
    m.add_function(wrap_pyfunction!(py_engine_result_free, m)?)?;
    Ok(())
}

/// Canonical Cargo package name of this crate.
///
/// Used by the InvoiceKit release tooling and by the bead-correlation
/// reports to map runtime log records back to the originating crate
/// without parsing `Cargo.toml` at runtime.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_binding_python::crate_name(), "invoicekit-binding-python");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-binding-python"
}

fn is_success_response(bytes: &[u8]) -> bool {
    bytes
        .windows(br#""status":"ok""#.len())
        .any(|window| window == br#""status":"ok""#)
}

#[cfg(test)]
mod tests {
    use super::{crate_name, engine_abi_version, process_engine_abi_json, EngineResult};
    use serde::Deserialize;

    const GOLDEN_FIXTURE: &str =
        include_str!("../../../conformance-corpus/golden/engine-abi-v1-commercial-document.json");

    #[derive(Debug, Deserialize)]
    struct GoldenFixture {
        request_bytes: String,
        expected_response_bytes: String,
    }

    fn golden_fixture() -> GoldenFixture {
        serde_json::from_str(GOLDEN_FIXTURE).expect("golden fixture is valid JSON")
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-binding-python");
    }

    #[test]
    fn crate_name_is_non_empty() {
        assert!(!crate_name().is_empty());
    }

    #[test]
    fn crate_name_is_lowercase_kebab() {
        let n = crate_name();
        for c in n.chars() {
            assert!(
                c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-',
                "non-kebab char in {n}: {c:?}"
            );
        }
    }

    #[test]
    fn crate_name_carries_invoicekit_prefix() {
        let n = crate_name();
        assert!(
            n == "invoicekit" || n.starts_with("invoicekit-") || n.starts_with("invoicekit_"),
            "crate name does not advertise InvoiceKit family: {n}"
        );
    }

    #[test]
    fn python_sdk_reports_engine_abi_version() {
        assert_eq!(engine_abi_version(), 1);
    }

    #[test]
    fn python_wrapper_matches_engine_abi_golden_fixture() {
        let fixture = golden_fixture();
        assert_eq!(
            process_engine_abi_json(fixture.request_bytes.as_bytes()),
            fixture.expected_response_bytes.as_bytes()
        );
    }

    #[test]
    fn python_result_handle_exposes_status_and_len() {
        let fixture = golden_fixture();
        let result = EngineResult::new(process_engine_abi_json(fixture.request_bytes.as_bytes()));

        assert_eq!(result.status(), 0);
        assert_eq!(result.len().unwrap(), fixture.expected_response_bytes.len());
    }

    #[test]
    fn python_result_free_marks_handle_invalid() {
        let fixture = golden_fixture();
        let mut result =
            EngineResult::new(process_engine_abi_json(fixture.request_bytes.as_bytes()));

        result.free();

        assert_eq!(result.status(), 2);
        assert!(result.len().is_err());
    }
}
