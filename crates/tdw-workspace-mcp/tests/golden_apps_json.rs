//! Golden test: a canonical two-pane Workspace layout built through the control
//! plane must serialize to a stable `apps.json` shape.
//!
//! This pins the exact `apps.json` document the control plane emits for a known
//! sequence of operations, so a drift in the [`tdw_widgets`] `apps.json` projection
//! (the App/Tab/grid `LayoutItem` shape) or in `WorkspaceState::to_apps_json` is
//! caught. Re-bless with `TDW_WORKSPACE_MCP_BLESS=1`:
//!
//! ```text
//! TDW_WORKSPACE_MCP_BLESS=1 cargo test -p tdw-workspace-mcp --test golden_apps_json --target-dir target
//! ```

use std::path::PathBuf;

use tdw_workspace_mcp::{Placement, WorkspaceState};

/// Build a deterministic two-pane layout: two real catalog widgets placed
/// side-by-side on one dashboard, plus a second empty dashboard.
fn canonical_layout() -> WorkspaceState {
    // Two real catalog widget ids, taken from the catalog so the golden stays in
    // lockstep with the projected widget catalog rather than hard-coding ids.
    let widgets = tdw_widgets::catalog_widgets();
    let left = &widgets[0].id;
    let right = &widgets[1].id;

    let mut ws = WorkspaceState::new(
        "Canonical Workspace",
        "A two-pane starter layout emitted by the control plane.",
    );
    ws.create_dashboard("overview", "Overview")
        .expect("create overview");
    ws.add_widget(
        "overview",
        left,
        Placement {
            x: 0,
            y: 0,
            w: 20,
            h: 12,
        },
    )
    .expect("place left");
    ws.add_widget(
        "overview",
        right,
        Placement {
            x: 20,
            y: 0,
            w: 20,
            h: 12,
        },
    )
    .expect("place right");
    ws.create_dashboard("detail", "Detail")
        .expect("create detail");
    ws
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("apps.json")
}

#[test]
fn canonical_layout_matches_golden_apps_json() {
    let apps = canonical_layout().to_apps_json();
    let rendered = format!(
        "{}\n",
        serde_json::to_string_pretty(&apps).expect("apps.json serializes")
    );

    let path = golden_path();
    if std::env::var("TDW_WORKSPACE_MCP_BLESS").is_ok() {
        std::fs::create_dir_all(path.parent().expect("golden parent")).expect("mkdir golden");
        std::fs::write(&path, &rendered).expect("write golden");
    } else {
        let expected = std::fs::read_to_string(&path)
            .expect("golden file missing; run with TDW_WORKSPACE_MCP_BLESS=1");
        assert_eq!(
            rendered, expected,
            "emitted apps.json layout drifted from the golden"
        );
    }
}

#[test]
fn golden_layout_deserializes_back_to_an_app_config() {
    // The golden really is the apps.json shape: it round-trips into the
    // tdw-widgets AppConfig (one tab per dashboard, each carrying LayoutItems).
    let apps = canonical_layout().to_apps_json();
    let app_value = apps
        .as_object()
        .and_then(|m| m.get("Canonical Workspace"))
        .expect("app keyed by name");
    let app: tdw_widgets::AppConfig =
        serde_json::from_value(app_value.clone()).expect("apps.json round-trips to AppConfig");
    assert_eq!(app.tabs.len(), 2, "two dashboards become two tabs");
    assert_eq!(
        app.tabs["overview"].layout.len(),
        2,
        "overview carries two placed widgets"
    );
    assert!(app.tabs["detail"].layout.is_empty());
}
