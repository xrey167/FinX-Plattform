#![forbid(unsafe_code)]
//! WASM UDF runtime for the TDW daemon.
//!
//! Two execution paths share one crate:
//!
//! # Default — deterministic fixture interpreter
//!
//! [`WasmUdfRuntime::execute`] validates WASM magic bytes + size, then maps a
//! small set of export names to pure-Rust transforms. It is byte-deterministic
//! (identical input → identical output) and needs no engine, so it is the
//! offline test default and requires no extra dependencies.
//!
//! # Hardened — real `wasmi` backend (feature `wasmi`)
//!
//! With the `wasmi` feature enabled, the runtime runs a real WebAssembly module
//! through the pure-Rust `wasmi` interpreter under explicit [`WasmLimits`]:
//!
//! - [`WasmUdfRuntime::execute_wasm_i64`] — an `(i64) -> i64` export.
//! - [`WasmUdfRuntime::execute_wasm_string`] — a string-in/string-out export
//!   over a linear-memory ABI (guest exports `memory` + `alloc(i32)->i32` +
//!   `<func>(in_ptr,in_len)->i64` returning packed `(out_ptr,out_len)`). All
//!   guest memory access uses wasmi's checked `Memory::read`/`write`, so a
//!   malformed pointer/length yields [`WasmRuntimeError::BadAbi`], never a host
//!   panic; output must be valid UTF-8.
//!
//! Both apply the same hardening:
//!
//! - **Fuel metering** bounds CPU; a runaway guest traps as
//!   [`WasmRuntimeError::FuelExhausted`].
//! - **Memory limits** cap linear memory; over-allocation fails as
//!   [`WasmRuntimeError::MemoryLimitExceeded`].
//! - **Deny-by-default imports**: the [`wasmi::Linker`] is empty, so any module
//!   that imports a host symbol is rejected before instantiation.
//!
//! These APIs are independent of the fixture [`WasmUdfRuntime::execute`], which
//! stays the default. Routing daemon UDF calls to the hardened runtime when a
//! profile enables it is the remaining integration step (see
//! `docs/quality/udf-runtime-hardening-scope.md`).

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

/// Resource limits applied to a real `wasmi`-backed UDF execution.
///
/// Part of the stable runtime contract regardless of whether the `wasmi`
/// feature is compiled in, so callers and config layers can construct limits
/// unconditionally. With the feature off the limits are inert (only the
/// deterministic fixture path exists).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WasmLimits {
    /// Maximum fuel (~executed bytecode operations) before the call traps.
    pub fuel: u64,
    /// Maximum linear memory, in bytes, the instance may allocate.
    pub max_memory_bytes: usize,
    /// Maximum number of linear memories the instance may define.
    pub max_memories: usize,
}

impl Default for WasmLimits {
    fn default() -> Self {
        Self {
            fuel: 1_000_000,
            max_memory_bytes: 16 * 1024 * 1024,
            max_memories: 1,
        }
    }
}

#[cfg(feature = "wasmi")]
pub use wasm_backend::WasmRuntimeError;

#[cfg(feature = "wasmi")]
mod wasm_backend {
    use super::{MAX_WASM_MODULE_BYTES, WasmLimits, WasmUdfRuntime, is_export_name};
    use wasmi::{Config, Engine, Instance, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};

    /// Errors from the real `wasmi`-backed execution path.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum WasmRuntimeError {
        InvalidExportedFunction,
        EmptyModuleBytes,
        ModuleTooLarge,
        InvalidMagic,
        /// The module failed to decode/validate as WebAssembly.
        Compile(String),
        /// The module imports a host symbol; imports are denied by default.
        DisallowedImport(String),
        /// Instantiation failed for a reason other than memory limits.
        Instantiate(String),
        /// Declared memory exceeded `WasmLimits::max_memory_bytes`/`max_memories`.
        MemoryLimitExceeded,
        /// The requested export does not exist or is not a function.
        MissingExport(String),
        /// The export exists but is not `(i64) -> i64`.
        BadSignature(String),
        /// Execution ran out of fuel (CPU-bound runaway guard).
        FuelExhausted,
        /// The guest trapped for some other reason.
        Trap(String),
        /// The guest violated the string ABI: missing `memory`/`alloc` export,
        /// an out-of-bounds pointer/length, or non-UTF-8 output.
        BadAbi(String),
    }

    impl std::fmt::Display for WasmRuntimeError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::InvalidExportedFunction => f.write_str("invalid exported function name"),
                Self::EmptyModuleBytes => f.write_str("empty module bytes"),
                Self::ModuleTooLarge => f.write_str("module too large"),
                Self::InvalidMagic => f.write_str("invalid WASM magic bytes"),
                Self::Compile(message) => write!(f, "module compile failed: {message}"),
                Self::DisallowedImport(name) => write!(f, "disallowed import: {name}"),
                Self::Instantiate(message) => write!(f, "instantiation failed: {message}"),
                Self::MemoryLimitExceeded => f.write_str("memory limit exceeded"),
                Self::MissingExport(name) => write!(f, "missing export: {name}"),
                Self::BadSignature(name) => write!(f, "export is not (i64) -> i64: {name}"),
                Self::FuelExhausted => f.write_str("fuel exhausted"),
                Self::Trap(message) => write!(f, "guest trap: {message}"),
                Self::BadAbi(message) => write!(f, "wasm ABI violation: {message}"),
            }
        }
    }

    impl std::error::Error for WasmRuntimeError {}

    /// Per-instance host state holding only the resource limiter.
    struct HostState {
        limits: StoreLimits,
    }

    impl WasmUdfRuntime {
        /// Execute a `(i64) -> i64` export from a real WebAssembly module under
        /// the given resource limits.
        ///
        /// Hardening guarantees:
        /// - fuel metering bounds CPU; a runaway guest traps with
        ///   [`WasmRuntimeError::FuelExhausted`];
        /// - linear memory is capped by [`WasmLimits`]; over-allocation fails
        ///   with [`WasmRuntimeError::MemoryLimitExceeded`];
        /// - imports are denied by default (empty [`Linker`]); a module that
        ///   imports any host symbol is rejected before instantiation.
        ///
        /// The deterministic fixture [`WasmUdfRuntime::execute`] is unaffected.
        ///
        /// # Errors
        ///
        /// Returns a [`WasmRuntimeError`] describing the validation, instantiation,
        /// or execution failure.
        pub fn execute_wasm_i64(
            &self,
            wasm_bytes: &[u8],
            func: &str,
            arg: i64,
            limits: WasmLimits,
        ) -> Result<i64, WasmRuntimeError> {
            let (mut store, instance) = instantiate_guarded(wasm_bytes, func, limits)?;
            let typed = instance
                .get_typed_func::<i64, i64>(&store, func)
                .map_err(|error| missing_or_bad_signature(&instance, &store, func, &error))?;
            typed.call(&mut store, arg).map_err(classify_runtime)
        }

        /// Execute a string-in / string-out export under the same hardening
        /// guarantees as [`WasmUdfRuntime::execute_wasm_i64`], using a
        /// linear-memory ABI.
        ///
        /// ABI contract — the guest module must export:
        /// - `memory`: linear memory;
        /// - `alloc(i32) -> i32`: returns a pointer to writable bytes;
        /// - `<func>(in_ptr: i32, in_len: i32) -> i64`: reads `in_len` input bytes
        ///   at `in_ptr`, writes the output into memory, and returns a packed
        ///   `i64` of `((out_ptr as u64) << 32) | (out_len as u64)`.
        ///
        /// All guest memory access goes through wasmi's checked
        /// `Memory::read`/`write`, so a malformed pointer/length yields
        /// [`WasmRuntimeError::BadAbi`] rather than a host panic. Output must be
        /// valid UTF-8.
        ///
        /// # Errors
        ///
        /// Returns a [`WasmRuntimeError`] for validation, instantiation, ABI, or
        /// execution failures.
        pub fn execute_wasm_string(
            &self,
            wasm_bytes: &[u8],
            func: &str,
            input: &str,
            limits: WasmLimits,
        ) -> Result<String, WasmRuntimeError> {
            let (mut store, instance) = instantiate_guarded(wasm_bytes, func, limits)?;

            let memory = instance
                .get_memory(&store, "memory")
                .ok_or_else(|| WasmRuntimeError::BadAbi("missing `memory` export".to_string()))?;
            let alloc = instance
                .get_typed_func::<i32, i32>(&store, "alloc")
                .map_err(|_| {
                    WasmRuntimeError::BadAbi(
                        "missing or mistyped `alloc` export (want (i32) -> i32)".to_string(),
                    )
                })?;

            let in_len = i32::try_from(input.len())
                .map_err(|_| WasmRuntimeError::BadAbi("input length exceeds i32".to_string()))?;
            let in_ptr = alloc.call(&mut store, in_len).map_err(classify_runtime)?;
            let in_ptr_usize = usize::try_from(in_ptr).map_err(|_| {
                WasmRuntimeError::BadAbi("alloc returned a negative pointer".to_string())
            })?;
            memory
                .write(&mut store, in_ptr_usize, input.as_bytes())
                .map_err(|error| {
                    WasmRuntimeError::BadAbi(format!("input write failed: {error}"))
                })?;

            let run = instance
                .get_typed_func::<(i32, i32), i64>(&store, func)
                .map_err(|error| missing_or_bad_signature(&instance, &store, func, &error))?;
            let packed = run
                .call(&mut store, (in_ptr, in_len))
                .map_err(classify_runtime)? as u64;
            let out_ptr = (packed >> 32) as u32 as usize;
            let out_len = (packed & 0xffff_ffff) as u32 as usize;

            let end = out_ptr
                .checked_add(out_len)
                .ok_or_else(|| WasmRuntimeError::BadAbi("output range overflow".to_string()))?;
            let memory_size = memory.data(&store).len();
            if end > memory_size {
                return Err(WasmRuntimeError::BadAbi(format!(
                    "output range {out_ptr}..{end} exceeds memory size {memory_size}"
                )));
            }

            let mut buffer = vec![0u8; out_len];
            memory.read(&store, out_ptr, &mut buffer).map_err(|error| {
                WasmRuntimeError::BadAbi(format!("output read failed: {error}"))
            })?;
            String::from_utf8(buffer)
                .map_err(|_| WasmRuntimeError::BadAbi("non-utf8 output".to_string()))
        }
    }

    /// Validate, compile (deny imports), and instantiate a module under the
    /// `WasmLimits` (fuel + memory). Shared by both execution entry points.
    fn instantiate_guarded(
        wasm_bytes: &[u8],
        func: &str,
        limits: WasmLimits,
    ) -> Result<(Store<HostState>, Instance), WasmRuntimeError> {
        if wasm_bytes.is_empty() {
            return Err(WasmRuntimeError::EmptyModuleBytes);
        }
        if wasm_bytes.len() > MAX_WASM_MODULE_BYTES {
            return Err(WasmRuntimeError::ModuleTooLarge);
        }
        if !wasm_bytes.starts_with(&[0x00, 0x61, 0x73, 0x6d]) {
            return Err(WasmRuntimeError::InvalidMagic);
        }
        if !is_export_name(func) {
            return Err(WasmRuntimeError::InvalidExportedFunction);
        }

        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);

        let module = Module::new(&engine, wasm_bytes)
            .map_err(|error| WasmRuntimeError::Compile(error.to_string()))?;

        // Deny-by-default imports: reject any module that imports a host symbol.
        if let Some(import) = module.imports().next() {
            return Err(WasmRuntimeError::DisallowedImport(format!(
                "{}::{}",
                import.module(),
                import.name()
            )));
        }

        let store_limits = StoreLimitsBuilder::new()
            .memory_size(limits.max_memory_bytes)
            .memories(limits.max_memories)
            .build();
        let mut store = Store::new(
            &engine,
            HostState {
                limits: store_limits,
            },
        );
        store.limiter(|state| &mut state.limits);
        store
            .add_fuel(limits.fuel)
            .map_err(|error| WasmRuntimeError::Instantiate(error.to_string()))?;

        // Empty linker => no host functions are available to the guest.
        let linker: Linker<HostState> = Linker::new(&engine);
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(classify_instantiate)?
            .start(&mut store)
            .map_err(classify_instantiate)?;
        Ok((store, instance))
    }

    fn missing_or_bad_signature(
        instance: &Instance,
        store: &Store<HostState>,
        func: &str,
        error: &wasmi::Error,
    ) -> WasmRuntimeError {
        if instance.get_func(store, func).is_some() {
            WasmRuntimeError::BadSignature(error.to_string())
        } else {
            WasmRuntimeError::MissingExport(func.to_string())
        }
    }

    fn classify_instantiate(error: wasmi::Error) -> WasmRuntimeError {
        let message = error.to_string();
        if message.contains("memory") || message.contains("limit") {
            WasmRuntimeError::MemoryLimitExceeded
        } else {
            WasmRuntimeError::Instantiate(message)
        }
    }

    fn classify_runtime(trap: wasmi::core::Trap) -> WasmRuntimeError {
        if matches!(trap.trap_code(), Some(wasmi::core::TrapCode::OutOfFuel)) {
            return WasmRuntimeError::FuelExhausted;
        }
        let message = trap.to_string();
        if message.contains("fuel") {
            WasmRuntimeError::FuelExhausted
        } else if message.contains("memory") || message.contains("limit") {
            WasmRuntimeError::MemoryLimitExceeded
        } else {
            WasmRuntimeError::Trap(message)
        }
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

#[cfg(all(test, feature = "wasmi"))]
mod wasmi_tests {
    use super::{WasmLimits, WasmRuntimeError, WasmUdfRuntime};

    fn wasm(text: &str) -> Vec<u8> {
        wat::parse_str(text).unwrap_or_else(|e| panic!("wat compiles: {e}"))
    }

    /// Guest implementing the string ABI: bump `alloc`, `memory`, and an `echo`
    /// that returns the input slice unchanged (packed ptr/len).
    const ECHO_GUEST: &str = r#"(module
        (memory (export "memory") 1)
        (global $bump (mut i32) (i32.const 1024))
        (func (export "alloc") (param $len i32) (result i32)
            (local $ptr i32)
            (local.set $ptr (global.get $bump))
            (global.set $bump (i32.add (global.get $bump) (local.get $len)))
            (local.get $ptr))
        (func (export "echo") (param $ptr i32) (param $len i32) (result i64)
            (i64.or
                (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
                (i64.extend_i32_u (local.get $len)))))"#;

    #[test]
    fn string_abi_echo_roundtrips() {
        let rt = WasmUdfRuntime::new();
        let out = rt
            .execute_wasm_string(
                &wasm(ECHO_GUEST),
                "echo",
                "hello world",
                WasmLimits::default(),
            )
            .unwrap_or_else(|e| panic!("echo should execute: {e}"));
        assert_eq!(out, "hello world");
    }

    #[test]
    fn string_abi_reads_distinct_output() {
        // `run` ignores its input and returns the constant "OK" written at offset 0.
        let guest = r#"(module
            (memory (export "memory") 1)
            (data (i32.const 0) "OK")
            (func (export "alloc") (param $len i32) (result i32) (i32.const 512))
            (func (export "run") (param $ptr i32) (param $len i32) (result i64)
                (i64.const 2)))"#;
        let rt = WasmUdfRuntime::new();
        let out = rt
            .execute_wasm_string(&wasm(guest), "run", "ignored", WasmLimits::default())
            .unwrap_or_else(|e| panic!("run should execute: {e}"));
        assert_eq!(out, "OK");
    }

    #[test]
    fn string_abi_fuel_exhaustion_traps() {
        let guest = r#"(module
            (memory (export "memory") 1)
            (func (export "alloc") (param $len i32) (result i32) (i32.const 0))
            (func (export "spin") (param $ptr i32) (param $len i32) (result i64)
                (loop (br 0)) (i64.const 0)))"#;
        let rt = WasmUdfRuntime::new();
        let limits = WasmLimits {
            fuel: 10_000,
            ..WasmLimits::default()
        };
        assert_eq!(
            rt.execute_wasm_string(&wasm(guest), "spin", "x", limits),
            Err(WasmRuntimeError::FuelExhausted)
        );
    }

    #[test]
    fn string_abi_missing_memory_is_bad_abi() {
        let guest = r#"(module
            (func (export "alloc") (param $len i32) (result i32) (i32.const 0))
            (func (export "run") (param $ptr i32) (param $len i32) (result i64) (i64.const 0)))"#;
        let rt = WasmUdfRuntime::new();
        assert!(matches!(
            rt.execute_wasm_string(&wasm(guest), "run", "x", WasmLimits::default()),
            Err(WasmRuntimeError::BadAbi(_))
        ));
    }

    #[test]
    fn string_abi_out_of_bounds_output_is_bad_abi() {
        // run returns out_ptr=0, out_len=0x0FFFFFFF (~268MB) >> one 64KiB page.
        let guest = r#"(module
            (memory (export "memory") 1)
            (func (export "alloc") (param $len i32) (result i32) (i32.const 0))
            (func (export "run") (param $ptr i32) (param $len i32) (result i64)
                (i64.const 268435455)))"#;
        let rt = WasmUdfRuntime::new();
        assert!(matches!(
            rt.execute_wasm_string(&wasm(guest), "run", "x", WasmLimits::default()),
            Err(WasmRuntimeError::BadAbi(_))
        ));
    }

    #[test]
    fn string_abi_non_utf8_output_is_bad_abi() {
        // Writes byte 0xFF at offset 0 and returns (ptr=0, len=1).
        let guest = r#"(module
            (memory (export "memory") 1)
            (func (export "alloc") (param $len i32) (result i32) (i32.const 16))
            (func (export "run") (param $ptr i32) (param $len i32) (result i64)
                (i32.store8 (i32.const 0) (i32.const 255))
                (i64.const 1)))"#;
        let rt = WasmUdfRuntime::new();
        assert!(matches!(
            rt.execute_wasm_string(&wasm(guest), "run", "x", WasmLimits::default()),
            Err(WasmRuntimeError::BadAbi(_))
        ));
    }

    #[test]
    fn executes_real_module_under_fuel() {
        let bytes = wasm(
            r#"(module (func (export "double") (param i64) (result i64)
                 local.get 0 i64.const 2 i64.mul))"#,
        );
        let rt = WasmUdfRuntime::new();
        let out = rt
            .execute_wasm_i64(&bytes, "double", 21, WasmLimits::default())
            .unwrap_or_else(|e| panic!("should execute: {e}"));
        assert_eq!(out, 42);
    }

    #[test]
    fn fuel_exhaustion_traps() {
        // Infinite loop: consumes fuel forever, must trap as FuelExhausted.
        let bytes = wasm(
            r#"(module (func (export "spin") (param i64) (result i64)
                 (loop (br 0)) i64.const 0))"#,
        );
        let rt = WasmUdfRuntime::new();
        let limits = WasmLimits {
            fuel: 10_000,
            ..WasmLimits::default()
        };
        assert_eq!(
            rt.execute_wasm_i64(&bytes, "spin", 0, limits),
            Err(WasmRuntimeError::FuelExhausted)
        );
    }

    #[test]
    fn disallowed_import_rejected() {
        let bytes = wasm(
            r#"(module (import "host" "secret" (func))
                 (func (export "x") (param i64) (result i64) i64.const 0))"#,
        );
        let rt = WasmUdfRuntime::new();
        assert_eq!(
            rt.execute_wasm_i64(&bytes, "x", 0, WasmLimits::default()),
            Err(WasmRuntimeError::DisallowedImport(
                "host::secret".to_string()
            ))
        );
    }

    #[test]
    fn memory_limit_exceeded_at_instantiation() {
        // Declares 100 pages (~6.5 MiB) of initial memory; limit allows ~1 page.
        let bytes = wasm(
            r#"(module (memory 100)
                 (func (export "x") (param i64) (result i64) i64.const 0))"#,
        );
        let rt = WasmUdfRuntime::new();
        let limits = WasmLimits {
            max_memory_bytes: 64 * 1024,
            ..WasmLimits::default()
        };
        assert_eq!(
            rt.execute_wasm_i64(&bytes, "x", 0, limits),
            Err(WasmRuntimeError::MemoryLimitExceeded)
        );
    }

    #[test]
    fn malformed_module_rejected() {
        // Valid magic + version, then a bogus section id => decode error.
        let bytes = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0xff];
        let rt = WasmUdfRuntime::new();
        assert!(matches!(
            rt.execute_wasm_i64(&bytes, "x", 0, WasmLimits::default()),
            Err(WasmRuntimeError::Compile(_))
        ));
    }

    #[test]
    fn missing_export_reported() {
        let bytes =
            wasm(r#"(module (func (export "present") (param i64) (result i64) local.get 0))"#);
        let rt = WasmUdfRuntime::new();
        assert_eq!(
            rt.execute_wasm_i64(&bytes, "absent", 0, WasmLimits::default()),
            Err(WasmRuntimeError::MissingExport("absent".to_string()))
        );
    }

    #[test]
    fn wrong_signature_reported() {
        // Export takes no params; calling it as (i64) -> i64 is a signature error.
        let bytes = wasm(r#"(module (func (export "noargs") (result i64) i64.const 0))"#);
        let rt = WasmUdfRuntime::new();
        assert!(matches!(
            rt.execute_wasm_i64(&bytes, "noargs", 0, WasmLimits::default()),
            Err(WasmRuntimeError::BadSignature(_))
        ));
    }

    #[test]
    fn rejects_invalid_magic_on_wasm_path() {
        let rt = WasmUdfRuntime::new();
        assert_eq!(
            rt.execute_wasm_i64(b"not-wasm", "x", 0, WasmLimits::default()),
            Err(WasmRuntimeError::InvalidMagic)
        );
    }
}
