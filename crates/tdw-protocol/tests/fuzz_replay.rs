//! Stable corpus-replay fuzz harness (TEST-POLICY-004 part 1).
//!
//! Loads every committed seed under `tests/corpus/protocol_json/` and feeds it
//! to the `__fuzz_protocol_json` shim, asserting the parser never panics on
//! arbitrary bytes. The same shim is reused by the nightly cargo-fuzz target.

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
fn protocol_json_corpus_never_panics() {
    for (path, bytes) in read_corpus("protocol_json") {
        // The assertion is that this call returns without panicking.
        tdw_protocol::__fuzz_protocol_json(&bytes);
        let _ = path;
    }
}
