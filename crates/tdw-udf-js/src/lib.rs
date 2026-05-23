#![forbid(unsafe_code)]

pub const CRATE_NAME: &str = "tdw-udf-js";
pub const RUNTIME_NAME: &str = "javascript";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaScriptUdfModule {
    pub module_name: String,
    pub entrypoint: String,
    pub source: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JavaScriptUdfError {
    EmptyModuleName,
    InvalidEntrypoint,
    EmptySource,
    DynamicImportDenied,
    NetworkAccessDenied,
}

pub fn validate_module(module: &JavaScriptUdfModule) -> Result<(), JavaScriptUdfError> {
    if module.module_name.trim().is_empty() {
        return Err(JavaScriptUdfError::EmptyModuleName);
    }
    if !is_identifier_path(&module.entrypoint) {
        return Err(JavaScriptUdfError::InvalidEntrypoint);
    }
    if module.source.trim().is_empty() {
        return Err(JavaScriptUdfError::EmptySource);
    }
    if module.source.contains("import(") {
        return Err(JavaScriptUdfError::DynamicImportDenied);
    }
    if module.source.contains("fetch(") || module.source.contains("XMLHttpRequest") {
        return Err(JavaScriptUdfError::NetworkAccessDenied);
    }

    Ok(())
}

fn is_identifier_path(value: &str) -> bool {
    !value.is_empty()
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.chars().all(is_identifier_character))
}

fn is_identifier_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_' || character == '$'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_safe_module_contract() {
        let module = JavaScriptUdfModule {
            module_name: CRATE_NAME.to_string(),
            entrypoint: "tdw.transform.upper".to_string(),
            source: "export function upper(input) { return input.toUpperCase(); }".to_string(),
        };

        assert_eq!(RUNTIME_NAME, "javascript");
        assert_eq!(validate_module(&module), Ok(()));
    }

    #[test]
    fn rejects_dynamic_import_and_network_access() {
        let module = JavaScriptUdfModule {
            module_name: CRATE_NAME.to_string(),
            entrypoint: "tdw.upper".to_string(),
            source: "export async function upper() { return fetch('https://example.com'); }"
                .to_string(),
        };

        assert_eq!(
            validate_module(&module),
            Err(JavaScriptUdfError::NetworkAccessDenied)
        );
        assert_eq!(
            validate_module(&JavaScriptUdfModule {
                source: "import('fs')".to_string(),
                ..module
            }),
            Err(JavaScriptUdfError::DynamicImportDenied)
        );
    }
}
