//! Offline `tdw-udf-python` example: validate a safe Python UDF module and show
//! the static filesystem / process / network denials.
//!
//! No Python interpreter, no network — this is the static validation gate only.
//!
//! ```text
//! cargo run --example tdw_udf_python_basic -p tdw-udf-python
//! ```

use tdw_udf_python::{PythonUdfError, PythonUdfModule, RUNTIME_NAME, validate_module};

fn main() {
    let module = PythonUdfModule {
        module_name: "tdw-udf-python".to_string(),
        function_name: "upper".to_string(),
        source: "def upper(input):\n    return input.upper()\n".to_string(),
    };

    validate_module(&module).expect("safe module should validate");
    println!("runtime: {RUNTIME_NAME}");
    println!("validated function: {}", module.function_name);

    // Filesystem access is denied.
    let fs = PythonUdfModule {
        source: "def upper(input):\n    return open('/etc/passwd').read()\n".to_string(),
        ..module.clone()
    };
    assert_eq!(
        validate_module(&fs),
        Err(PythonUdfError::FilesystemAccessDenied)
    );

    // Process access is denied.
    let proc = PythonUdfModule {
        source: "import subprocess\nsubprocess.run(['id'])\n".to_string(),
        ..module.clone()
    };
    assert_eq!(
        validate_module(&proc),
        Err(PythonUdfError::ProcessAccessDenied)
    );

    // Network access is denied.
    let net = PythonUdfModule {
        source: "import socket\n".to_string(),
        ..module
    };
    assert_eq!(
        validate_module(&net),
        Err(PythonUdfError::NetworkAccessDenied)
    );
    println!("filesystem, process, and network access are denied, as expected");
}
