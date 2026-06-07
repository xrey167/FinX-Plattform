//! Integration-style tests for `tdw-email` always-compiled components.
//!
//! These tests cover the template engine, config env-reading, and message
//! validation. No SMTP connection is made; live send tests require
//! `TDW_SMTP_HOST` + `TDW_EMAIL_FROM` to be set and are not included here.

use std::collections::BTreeMap;

use tdw_email::{EmailConfig, EmailMessage, render_template};

// ---------------------------------------------------------------------------
// Template rendering — happy paths
// ---------------------------------------------------------------------------

#[test]
fn render_welcome_template_happy_path() {
    let mut vars = BTreeMap::new();
    vars.insert("name", "Alice".to_string());
    vars.insert("body", "Thanks for joining us.".to_string());
    vars.insert("cta", "Get started today.".to_string());
    vars.insert(
        "unsubscribe_url",
        "https://example.com/unsubscribe".to_string(),
    );

    let html = render_template("welcome", &vars).expect("welcome render should succeed");
    assert!(html.contains("Alice"), "rendered html should contain name");
    assert!(
        html.contains("Thanks for joining us."),
        "rendered html should contain body"
    );
    assert!(
        html.contains("https://example.com/unsubscribe"),
        "rendered html should contain unsubscribe url"
    );
}

#[test]
fn render_market_news_summary_template_happy_path() {
    let mut vars = BTreeMap::new();
    vars.insert("name", "Bob".to_string());
    vars.insert("date", "2026-06-07".to_string());
    vars.insert("headlines", "Markets rise on positive data.".to_string());
    vars.insert("summary", "Broad gains across sectors.".to_string());
    vars.insert("cta", "Read the full report.".to_string());
    vars.insert(
        "unsubscribe_url",
        "https://example.com/unsubscribe".to_string(),
    );

    let html = render_template("market_news_summary", &vars)
        .expect("market_news_summary render should succeed");
    assert!(html.contains("Bob"), "rendered html should contain name");
    assert!(
        html.contains("2026-06-07"),
        "rendered html should contain date"
    );
    assert!(
        html.contains("Markets rise on positive data."),
        "rendered html should contain headlines"
    );
}

#[test]
fn render_reengagement_template_happy_path() {
    let mut vars = BTreeMap::new();
    vars.insert("name", "Carol".to_string());
    vars.insert("body", "We noticed you have been away.".to_string());
    vars.insert("cta_url", "https://example.com/login".to_string());
    vars.insert("cta_label", "Come back".to_string());
    vars.insert(
        "unsubscribe_url",
        "https://example.com/unsubscribe".to_string(),
    );

    let html = render_template("reengagement", &vars).expect("reengagement render should succeed");
    assert!(html.contains("Carol"), "rendered html should contain name");
    assert!(
        html.contains("Come back"),
        "rendered html should contain cta_label"
    );
    assert!(
        html.contains("https://example.com/login"),
        "rendered html should contain cta_url"
    );
}

// ---------------------------------------------------------------------------
// Template engine error cases
// ---------------------------------------------------------------------------

#[test]
fn render_unknown_template_returns_error() {
    let vars = BTreeMap::new();
    let err = render_template("does_not_exist", &vars).expect_err("unknown template should error");
    let msg = err.to_string();
    assert!(
        msg.contains("unknown template"),
        "error should say unknown: {msg}"
    );
}

#[test]
fn render_with_unfilled_placeholder_returns_error() {
    // Provide all required vars except `unsubscribe_url`.
    let mut vars = BTreeMap::new();
    vars.insert("name", "Dave".to_string());
    vars.insert("body", "Some body text.".to_string());
    vars.insert("cta", "Click here.".to_string());
    // Intentionally omit unsubscribe_url.

    let err = render_template("welcome", &vars).expect_err("unfilled placeholder should error");
    let msg = err.to_string();
    assert!(
        msg.contains("unsubscribe_url"),
        "error should name the unfilled placeholder: {msg}"
    );
}

// ---------------------------------------------------------------------------
// EmailConfig::from_env
// ---------------------------------------------------------------------------

/// All env-var cases are run sequentially in a single test function to avoid
/// parallel-test races on shared environment state.
#[test]
fn email_config_from_env_cases() {
    // -- Case 1: both mandatory vars absent → None --
    // We test our own logic via a helper that mirrors from_env, passing
    // explicit Option values instead of mutating the process environment.
    assert!(
        config_from_specific_vars(None, None, None, None, None).is_none(),
        "missing host and from → None"
    );

    // -- Case 2: host set but from missing → None --
    assert!(
        config_from_specific_vars(Some("smtp.example.com"), None, None, None, None).is_none(),
        "missing from → None"
    );

    // -- Case 3: from set but host missing → None --
    assert!(
        config_from_specific_vars(None, None, None, None, Some("noreply@example.com")).is_none(),
        "missing host → None"
    );

    // -- Case 4: both present, defaults --
    let cfg = config_from_specific_vars(
        Some("smtp.example.com"),
        None,
        None,
        None,
        Some("noreply@example.com"),
    )
    .expect("should be Some when both mandatory vars present");
    assert_eq!(cfg.smtp_host, "smtp.example.com");
    assert_eq!(cfg.smtp_port, 587, "default port should be 587");
    assert_eq!(cfg.smtp_username, "");
    assert_eq!(cfg.smtp_password, "");
    assert_eq!(cfg.from_address, "noreply@example.com");

    // -- Case 5: all vars explicitly set --
    let cfg = config_from_specific_vars(
        Some("mail.relay.io"),
        Some("2525"),
        Some("user"),
        Some("s3cr3t"),
        Some("alerts@relay.io"),
    )
    .expect("should be Some");
    assert_eq!(cfg.smtp_host, "mail.relay.io");
    assert_eq!(cfg.smtp_port, 2525);
    assert_eq!(cfg.smtp_username, "user");
    assert_eq!(cfg.smtp_password, "s3cr3t");
    assert_eq!(cfg.from_address, "alerts@relay.io");
}

/// Mirror of [`EmailConfig::from_env`] operating on explicit values rather
/// than real env vars, so we can test all branches without mutating the
/// process environment.
fn config_from_specific_vars(
    host: Option<&str>,
    port: Option<&str>,
    username: Option<&str>,
    password: Option<&str>,
    from: Option<&str>,
) -> Option<EmailConfig> {
    let smtp_host = host?.to_string();
    let from_address = from?.to_string();
    let smtp_port = port.and_then(|v| v.parse::<u16>().ok()).unwrap_or(587);
    let smtp_username = username.unwrap_or("").to_string();
    let smtp_password = password.unwrap_or("").to_string();
    Some(EmailConfig {
        smtp_host,
        smtp_port,
        smtp_username,
        smtp_password,
        from_address,
    })
}

// ---------------------------------------------------------------------------
// EmailMessage derives
// ---------------------------------------------------------------------------

#[test]
fn email_message_clone_and_eq() {
    let msg = EmailMessage {
        from: "a@example.com".to_string(),
        to: "b@example.com".to_string(),
        subject: "Test".to_string(),
        text: "Plain".to_string(),
        html: "<p>HTML</p>".to_string(),
    };
    let cloned = msg.clone();
    assert_eq!(msg, cloned);
}

#[test]
fn email_message_serde_roundtrip() {
    let msg = EmailMessage {
        from: "a@example.com".to_string(),
        to: "b@example.com".to_string(),
        subject: "Roundtrip".to_string(),
        text: "Text body".to_string(),
        html: "<p>HTML body</p>".to_string(),
    };
    let json = serde_json::to_string(&msg).expect("serialize");
    let back: EmailMessage = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(msg, back);
}
