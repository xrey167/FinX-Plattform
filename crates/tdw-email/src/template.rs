#![forbid(unsafe_code)]

//! Minimal `{{key}}` template engine with strict unknown-placeholder detection.
//!
//! Three HTML shells are embedded at compile time via [`include_str!`]:
//!
//! | Name | File |
//! |------|------|
//! | `welcome` | `templates/welcome.html` |
//! | `market_news_summary` | `templates/market_news_summary.html` |
//! | `reengagement` | `templates/reengagement.html` |
//!
//! All template variables use the `{{name}}` syntax. Unknown placeholders
//! remaining after substitution cause [`EmailError::Template`] listing the
//! unfilled keys — this catches typos at send time rather than silently
//! producing broken HTML.

use std::collections::BTreeMap;

use crate::EmailError;

const WELCOME_HTML: &str = include_str!("../templates/welcome.html");
const MARKET_NEWS_SUMMARY_HTML: &str = include_str!("../templates/market_news_summary.html");
const REENGAGEMENT_HTML: &str = include_str!("../templates/reengagement.html");

/// Render a named HTML template, substituting `{{key}}` placeholders from `vars`.
///
/// # Errors
///
/// - [`EmailError::Template`] when `name` does not match a known template.
/// - [`EmailError::Template`] when any `{{key}}` placeholder remains after
///   substitution (strict mode — the caller must supply all variables the
///   template declares).
pub fn render_template(name: &str, vars: &BTreeMap<&str, String>) -> Result<String, EmailError> {
    let raw = template_source(name)?;
    fill(raw, vars)
}

/// Look up the raw template source for `name`.
fn template_source(name: &str) -> Result<&'static str, EmailError> {
    match name {
        "welcome" => Ok(WELCOME_HTML),
        "market_news_summary" => Ok(MARKET_NEWS_SUMMARY_HTML),
        "reengagement" => Ok(REENGAGEMENT_HTML),
        other => Err(EmailError::Template(format!(
            "unknown template `{other}`; available: welcome, market_news_summary, reengagement"
        ))),
    }
}

/// Fill `{{key}}` placeholders in `source` with values from `vars`.
///
/// After all substitutions, any remaining `{{...}}` blocks are collected and
/// returned as a [`EmailError::Template`] error.
fn fill(source: &'static str, vars: &BTreeMap<&str, String>) -> Result<String, EmailError> {
    let mut result = source.to_owned();
    for (key, value) in vars {
        let placeholder = format!("{{{{{key}}}}}");
        result = result.replace(&placeholder, value);
    }

    // Strict check: find any leftover {{...}} placeholders.
    let unfilled = collect_unfilled(&result);
    if unfilled.is_empty() {
        Ok(result)
    } else {
        Err(EmailError::Template(format!(
            "unfilled template placeholders: {}",
            unfilled.join(", ")
        )))
    }
}

/// Collect all `{{key}}` tokens still present in `text`.
fn collect_unfilled(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut remaining = text;
    while let Some(open) = remaining.find("{{") {
        let after_open = &remaining[open + 2..];
        if let Some(close) = after_open.find("}}") {
            let key = &after_open[..close];
            let token = format!("{{{{{key}}}}}");
            if !found.contains(&token) {
                found.push(token);
            }
            remaining = &after_open[close + 2..];
        } else {
            break;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_substitutes_all_vars() {
        let source: &'static str = "Hello {{name}}, your code is {{code}}.";
        // SAFETY: we are lending a static string slice for the test.
        let mut vars = BTreeMap::new();
        vars.insert("name", "Alice".to_string());
        vars.insert("code", "XYZ".to_string());
        // Use fill directly to test the engine in isolation.
        let result = fill(source, &vars).expect("fill should succeed");
        assert_eq!(result, "Hello Alice, your code is XYZ.");
    }

    #[test]
    fn fill_strict_error_on_unfilled_placeholder() {
        let source: &'static str = "Hello {{name}}, click {{link}}.";
        let mut vars = BTreeMap::new();
        vars.insert("name", "Bob".to_string());
        // `link` is intentionally not supplied.
        let err = fill(source, &vars).expect_err("should fail with unfilled placeholder");
        let msg = err.to_string();
        assert!(
            msg.contains("{{link}}"),
            "error should name the unfilled key: {msg}"
        );
    }

    #[test]
    fn unknown_template_error() {
        let vars = BTreeMap::new();
        let err = render_template("no_such_template", &vars)
            .expect_err("should fail for unknown template");
        let msg = err.to_string();
        assert!(
            msg.contains("unknown template"),
            "error should say unknown: {msg}"
        );
    }

    #[test]
    fn duplicate_unfilled_keys_reported_once() {
        let source: &'static str = "{{x}} and {{x}} again";
        let vars = BTreeMap::new();
        let err = fill(source, &vars).expect_err("should fail");
        // Only one occurrence of {{x}} in the error message list.
        let msg = err.to_string();
        let count = msg.matches("{{x}}").count();
        assert_eq!(
            count, 1,
            "duplicate key should appear only once in error: {msg}"
        );
    }
}
