#![forbid(unsafe_code)]

pub const CRATE_NAME: &str = "tdw-udf-wasm";
pub const RUNTIME_NAME: &str = "wasm";
pub const MAX_WASM_MODULE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WasmUdfModule {
    pub module_name: String,
    pub exported_function: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WasmUdfError {
    EmptyModuleName,
    InvalidExportedFunction,
    EmptyModuleBytes,
    ModuleTooLarge,
    InvalidMagic,
}

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
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
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
}
