//! Stable corpus-replay fuzz harness (TEST-POLICY-004 part 1).
//!
//! Loads every committed seed under `tests/corpus/mcp_jsonrpc/` and
//! `tests/corpus/mcp_http/`, feeding them to the `__fuzz_mcp_jsonrpc` and
//! `__fuzz_mcp_http` shims respectively. Asserts the request parsers never
//! panic on arbitrary bytes. The same shims are reused by the nightly
//! cargo-fuzz targets.

use std::fs;
use std::path::PathBuf;

fn corpus_dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
        .join(name)
}

fn read_corpus(name: &str) -> Vec<(PathBuf, Vec<u8>)> {
    let dir = corpus_dir(name);
    let entries = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("read corpus dir {}: {error}", dir.display()));
    let mut files = Vec::new();
    for entry in entries {
        let path = entry.expect("corpus dir entry").path();
        if path.is_file() {
            let bytes = fs::read(&path)
                .unwrap_or_else(|error| panic!("read corpus file {}: {error}", path.display()));
            files.push((path, bytes));
        }
    }
    assert!(!files.is_empty(), "corpus {name} must contain seed files");
    files
}

#[test]
fn mcp_jsonrpc_corpus_never_panics() {
    for (path, bytes) in read_corpus("mcp_jsonrpc") {
        tdw_mcp::__fuzz_mcp_jsonrpc(&bytes);
        let _ = path;
    }
}

#[test]
fn mcp_http_corpus_never_panics() {
    for (path, bytes) in read_corpus("mcp_http") {
        tdw_mcp::__fuzz_mcp_http(&bytes);
        let _ = path;
    }
}
