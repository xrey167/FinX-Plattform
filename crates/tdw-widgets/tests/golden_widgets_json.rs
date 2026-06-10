//! Golden snapshot of the full derived `widgets.json` / `apps.json` documents.
//!
//! The derivation is pure and deterministic, so the assembled documents are
//! reproducible. These tests pin the **entire** rendered `JSON` against a
//! checked-in golden file so any drift in the derivation engine (new route, a
//! changed param mapping, a renamed field) surfaces as a reviewable diff.
//!
//! To refresh after an intentional change, run with `TDW_WIDGETS_BLESS=1`:
//! `TDW_WIDGETS_BLESS=1 cargo test -p tdw-widgets --test golden_widgets_json`.

use std::path::Path;

/// Pretty-print a value with a trailing newline (the on-disk golden form).
fn render(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).expect("value serializes") + "\n"
}

/// Compare `rendered` against the golden file at `rel_path`, blessing it when
/// `TDW_WIDGETS_BLESS` is set so a deliberate change refreshes the snapshot.
fn assert_golden(rel_path: &str, rendered: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(rel_path);
    if std::env::var("TDW_WIDGETS_BLESS").is_ok() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create golden dir");
        }
        std::fs::write(&path, rendered).expect("write golden");
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "golden file missing at {}: {error}; run with TDW_WIDGETS_BLESS=1 to create it",
            path.display()
        )
    });
    assert_eq!(
        rendered,
        expected,
        "derived document drifted from {}; run with TDW_WIDGETS_BLESS=1 to refresh",
        path.display()
    );
}

#[test]
fn widgets_json_matches_golden() {
    let rendered = render(&tdw_widgets::widgets_json());
    assert_golden("widgets.json", &rendered);
}

#[test]
fn apps_json_matches_golden() {
    let rendered = render(&tdw_widgets::apps_json());
    assert_golden("apps.json", &rendered);
}

#[test]
fn golden_widgets_contains_marquee_routes() {
    // A coarse invariant alongside the byte-exact golden: the document includes
    // the marquee routes the default app lays out, so the snapshot can never go
    // empty without these tripping too.
    let document = tdw_widgets::widgets_json();
    for id in [
        "equity_price_historical",
        "equity_price_quote",
        "fixedincome_government_yield_curve",
        "economy_cpi",
    ] {
        assert!(
            document.get(id).is_some(),
            "golden widgets.json must contain {id}"
        );
    }
}
