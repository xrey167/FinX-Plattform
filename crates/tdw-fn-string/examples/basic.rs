//! Offline `tdw-fn-string` example: input -> normalized output via a pipeline,
//! plus rejection of an unsafe `Replace` pattern.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p tdw-fn-string --example tdw_fn_string_basic
//! ```

#![forbid(unsafe_code)]

use tdw_fn_string::{StringFn, StringFnError, StringPipeline, apply_pipeline, validate_pipeline};

fn main() {
    // Normalize " aapl equity " -> "AAPL_EQUITY".
    let pipeline = StringPipeline {
        name: "normalize-symbol".to_string(),
        steps: vec![
            StringFn::Trim,
            StringFn::Replace {
                from: " ".to_string(),
                to: "_".to_string(),
            },
            StringFn::Uppercase,
        ],
    };

    let output = apply_pipeline(" aapl equity ", &pipeline).expect("pipeline runs");
    assert_eq!(output, "AAPL_EQUITY");
    println!("normalized: {output:?}");

    // An empty `from` pattern is rejected before any transform runs.
    let empty = StringPipeline {
        name: "bad".to_string(),
        steps: vec![StringFn::Replace {
            from: String::new(),
            to: "x".to_string(),
        }],
    };
    assert_eq!(validate_pipeline(&empty), Err(StringFnError::EmptyPattern));

    // A shell-metacharacter replacement is rejected as unsafe.
    let unsafe_pipeline = StringPipeline {
        name: "bad".to_string(),
        steps: vec![StringFn::Replace {
            from: "AAPL".to_string(),
            to: "AAPL;DROP".to_string(),
        }],
    };
    assert_eq!(
        apply_pipeline("AAPL", &unsafe_pipeline),
        Err(StringFnError::UnsafePattern),
    );
    println!("unsafe replacement rejected");
}
