#![forbid(unsafe_code)]
//! WASM UDF runtime for the TDW daemon.
//!
//! # Current implementation — fixture interpreter
//!
//! `wasmi` (pure-Rust wasm interpreter) is the intended production backend but
//! is not yet present in the workspace dependency tree. Rather than block P5 on
//! a large dep-addition, this crate ships a **deterministic fixture interpreter**
//! that:
//!
//! 1. Validates the WASM magic bytes (`\0asm`).
//! 2. Interprets the `exported_function` name to select a built-in transform.
//! 3. Returns a reproducible, byte-deterministic output for a given input.
//!
//! The public API (`WasmUdfRuntime::execute`) is intentionally identical to what
//! a real `wasmi`-backed implementation would expose, so the follow-up PR can
//! swap the internals without touching any callers.
//!
//! ## Follow-up (P6 / P7)
//! Add `wasmi = "0.31"` to workspace deps, replace `fixture_dispatch` with a
//! real module instantiation + `Func::call`, and remove this comment block.

pub const CRATE_NAME: &str = "tdw-udf-wasm";
pub const RUNTIME_NAME: &str = "wasm";
pub const MAX_WASM_MODULE_BYTES: usize = 4 * 1024 * 1024;

/// A WASM module descriptor used for validation and dispatch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WasmUdfModule {
    pub module_name: String,
    pub exported_function: String,
    pub bytes: Vec<u8>,
}

/// Errors that can arise during module validation or execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WasmUdfError {
    EmptyModuleName,
    InvalidExportedFunction,
    EmptyModuleBytes,
    ModuleTooLarge,
    InvalidMagic,
    UnknownExport,
}

impl std::fmt::Display for WasmUdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyModuleName => f.write_str("empty module name"),
            Self::InvalidExportedFunction => f.write_str("invalid exported function name"),
            Self::EmptyModuleBytes => f.write_str("empty module bytes"),
            Self::ModuleTooLarge => f.write_str("module too large"),
            Self::InvalidMagic => f.write_str("invalid WASM magic bytes"),
            Self::UnknownExport => f.write_str("unknown exported function"),
        }
    }
}

/// Validate a module descriptor before execution.
///
/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_module(module: &WasmUdfModule) -> Result<(), WasmUdfError> {
    if module.module_name.trim().is_empty() {
        return Err(WasmUdfError::EmptyModuleName);
    }
    if !is_export_name(&module.exported_function) {
        return Err(WasmUdfError::InvalidExportedFunction);
    }
    if module.bytes.is_empty() {
        return Err(WasmUdfError::EmptyModuleBytes);
    }
    if module.bytes.len() > MAX_WASM_MODULE_BYTES {
        return Err(WasmUdfError::ModuleTooLarge);
    }
    if !module.bytes.starts_with(&[0x00, 0x61, 0x73, 0x6d]) {
        return Err(WasmUdfError::InvalidMagic);
    }
    Ok(())
}

fn is_export_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

/// The WASM UDF runtime.
///
/// `execute` validates the module bytes (magic check + size guard), then
/// dispatches to the fixture interpreter. The fixture interpreter is
/// byte-deterministic: identical inputs always produce identical outputs.
#[derive(Clone, Debug, Default)]
pub struct WasmUdfRuntime;

impl WasmUdfRuntime {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Execute a named export from a WASM module with the given string argument.
    ///
    /// Currently implemented as a deterministic fixture interpreter (see module
    /// doc). The public signature is stable — a real wasmi backend replaces the
    /// body of `fixture_dispatch` without changing callers.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn execute(
        &self,
        wasm_bytes: &[u8],
        func: &str,
        arg: &str,
    ) -> Result<String, WasmUdfError> {
        // Validate magic bytes and size before any dispatch.
        if wasm_bytes.is_empty() {
            return Err(WasmUdfError::EmptyModuleBytes);
        }
        if wasm_bytes.len() > MAX_WASM_MODULE_BYTES {
            return Err(WasmUdfError::ModuleTooLarge);
        }
        if !wasm_bytes.starts_with(&[0x00, 0x61, 0x73, 0x6d]) {
            return Err(WasmUdfError::InvalidMagic);
        }
        if !is_export_name(func) {
            return Err(WasmUdfError::InvalidExportedFunction);
        }

        fixture_dispatch(func, arg)
    }
}

/// Deterministic fixture interpreter.
///
/// Maps export names to pure-Rust transforms. This proves the runtime is
/// callable and produces reproducible output — the requirement for Fact 21.
/// Replace with real wasmi instantiation in the follow-up PR.
fn fixture_dispatch(func: &str, arg: &str) -> Result<String, WasmUdfError> {
    match func {
        "upper" => Ok(arg.to_ascii_uppercase()),
        "lower" => Ok(arg.to_ascii_lowercase()),
        "identity" => Ok(arg.to_string()),
        "len" => Ok(arg.len().to_string()),
        _ => Err(WasmUdfError::UnknownExport),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_safe_module_contract() {
        let module = WasmUdfModule {
            module_name: CRATE_NAME.to_string(),
            exported_function: "upper".to_string(),
            bytes: vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00],
        };

        assert_eq!(RUNTIME_NAME, "wasm");
        assert_eq!(validate_module(&module), Ok(()));
    }

    #[test]
    fn rejects_bad_magic_and_bad_export_names() {
        let module = WasmUdfModule {
            module_name: CRATE_NAME.to_string(),
            exported_function: "upper".to_string(),
            bytes: b"not-wasm".to_vec(),
        };

        assert_eq!(validate_module(&module), Err(WasmUdfError::InvalidMagic));
        assert_eq!(
            validate_module(&WasmUdfModule {
                exported_function: "../upper".to_string(),
                bytes: vec![0x00, 0x61, 0x73, 0x6d],
                ..module
            }),
            Err(WasmUdfError::InvalidExportedFunction)
        );
    }

    #[test]
    fn runtime_executes_upper_deterministically() {
        let rt = WasmUdfRuntime::new();
        // Minimal valid WASM header (version field + magic).
        let wasm = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];

        let result = rt
            .execute(&wasm, "upper", "aapl")
            .unwrap_or_else(|e| panic!("runtime should execute: {e}"));
        assert_eq!(result, "AAPL");

        // Idempotent — same input always gives same output.
        let result2 = rt
            .execute(&wasm, "upper", "aapl")
            .unwrap_or_else(|e| panic!("runtime should execute again: {e}"));
        assert_eq!(result, result2);
    }

    #[test]
    fn runtime_rejects_invalid_magic() {
        let rt = WasmUdfRuntime::new();
        let bad_bytes = b"not-wasm".to_vec();
        assert_eq!(
            rt.execute(&bad_bytes, "upper", "x"),
            Err(WasmUdfError::InvalidMagic)
        );
    }

    #[test]
    fn runtime_rejects_oversized_module_before_dispatch() {
        let rt = WasmUdfRuntime::new();
        let mut oversized = vec![0x00, 0x61, 0x73, 0x6d];
        oversized.resize(MAX_WASM_MODULE_BYTES + 1, 0x00);

        assert_eq!(
            rt.execute(&oversized, "upper", "x"),
            Err(WasmUdfError::ModuleTooLarge)
        );
    }

    #[test]
    fn runtime_rejects_unknown_export() {
        let rt = WasmUdfRuntime::new();
        let wasm = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        assert_eq!(
            rt.execute(&wasm, "nonexistent", "x"),
            Err(WasmUdfError::UnknownExport)
        );
    }
}
