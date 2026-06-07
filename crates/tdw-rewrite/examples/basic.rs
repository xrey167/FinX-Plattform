//! Offline `tdw-rewrite` example: input -> rewritten output (enabled rules only),
//! plus rejection of an unsafe pattern.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p tdw-rewrite --example tdw_rewrite_basic
//! ```

#![forbid(unsafe_code)]

use tdw_rewrite::{RewriteError, RewritePlan, RewriteRule, apply_rewrites};

fn main() {
    // Only the enabled rule applies: "research aapl" -> "research AAPL".
    let plan = RewritePlan {
        rules: vec![
            RewriteRule {
                rule_id: "normalize-symbol".to_string(),
                find: "aapl".to_string(),
                replace: "AAPL".to_string(),
                enabled: true,
            },
            RewriteRule {
                rule_id: "disabled".to_string(),
                find: "AAPL".to_string(),
                replace: "MSFT".to_string(),
                enabled: false,
            },
        ],
    };

    let output = apply_rewrites("research aapl", &plan).expect("plan runs");
    assert_eq!(output, "research AAPL");
    println!("rewritten: {output:?}");

    // A shell-metacharacter replacement is rejected as unsafe before running.
    let unsafe_plan = RewritePlan {
        rules: vec![RewriteRule {
            rule_id: "unsafe".to_string(),
            find: "AAPL".to_string(),
            replace: "AAPL;DROP".to_string(),
            enabled: true,
        }],
    };
    assert_eq!(
        apply_rewrites("AAPL", &unsafe_plan),
        Err(RewriteError::UnsafePattern),
    );
    println!("unsafe rewrite rejected");
}
