#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
pub const CRATE_NAME: &str = "tdw-udf-python";
pub const RUNTIME_NAME: &str = "python";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PythonUdfModule {
    pub module_name: String,
    pub function_name: String,
    pub source: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PythonUdfError {
    EmptyModuleName,
    InvalidFunctionName,
    EmptySource,
    FilesystemAccessDenied,
    ProcessAccessDenied,
    NetworkAccessDenied,
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_module(module: &PythonUdfModule) -> Result<(), PythonUdfError> {
    if module.module_name.trim().is_empty() {
        return Err(PythonUdfError::EmptyModuleName);
    }
    if !is_identifier(&module.function_name) {
        return Err(PythonUdfError::InvalidFunctionName);
    }
    if module.source.trim().is_empty() {
        return Err(PythonUdfError::EmptySource);
    }
    if module.source.contains("open(") || module.source.contains("import pathlib") {
        return Err(PythonUdfError::FilesystemAccessDenied);
    }
    if module.source.contains("import subprocess") || module.source.contains("os.system") {
        return Err(PythonUdfError::ProcessAccessDenied);
    }
    if module.source.contains("import socket") || module.source.contains("requests.") {
        return Err(PythonUdfError::NetworkAccessDenied);
    }

    Ok(())
}

fn is_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_safe_module_contract() {
        let module = PythonUdfModule {
            module_name: CRATE_NAME.to_string(),
            function_name: "upper".to_string(),
            source: "def upper(input):\n    return input.upper()\n".to_string(),
        };

        assert_eq!(RUNTIME_NAME, "python");
        assert_eq!(validate_module(&module), Ok(()));
    }

    #[test]
    fn rejects_filesystem_process_and_network_access() {
        let module = PythonUdfModule {
            module_name: CRATE_NAME.to_string(),
            function_name: "upper".to_string(),
            source: "def upper(input):\n    return open('/etc/passwd').read()\n".to_string(),
        };

        assert_eq!(
            validate_module(&module),
            Err(PythonUdfError::FilesystemAccessDenied)
        );
        assert_eq!(
            validate_module(&PythonUdfModule {
                source: "import subprocess\nsubprocess.run(['id'])\n".to_string(),
                ..module.clone()
            }),
            Err(PythonUdfError::ProcessAccessDenied)
        );
        assert_eq!(
            validate_module(&PythonUdfModule {
                source: "import socket\n".to_string(),
                ..module
            }),
            Err(PythonUdfError::NetworkAccessDenied)
        );
    }
}
