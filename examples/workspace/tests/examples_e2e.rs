//! CI verification for the whole example suite.
//!
//! Each test boots the relevant example's server path (or drives its pure
//! builder) exactly as the example's `main()` does, by calling the same library
//! functions, and asserts golden outputs — fully offline. This is the
//! repo-idiomatic shape used by `tdw-service-api/tests/*_e2e.rs`: a
//! `RestApiState`/`AgentBridgeState` over `AppState::in_memory_for_tests()`,
//! driven over an ephemeral loopback listener. Booting each example through its
//! library function (rather than spawning the bin) keeps the assertions precise
//! and the suite fast.

use tdw_openbb_agent::SseEvent;
use tdw_workspace_examples::{
    agent_echo, app, charts_tables, client, derived, graph, minimal, reasoning_citations,
    widget_data,
};

// --- Example 10: hand-written minimal backend -------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn example_10_minimal_serves_handwritten_widget_and_data() {
    // The pure documents are the contract.
    let widgets = minimal::widgets_document();
    let widget = &widgets["example_price"];
    assert_eq!(widget["type"], "chart");
    assert_eq!(widget["endpoint"], minimal::DATA_ENDPOINT);
    assert_eq!(widget["data"]["dataKey"], "results");

    // The booted server answers both routes and 404s anything else.
    let server = minimal::serve().await.expect("minimal server boots");
    let manifest = client::get(server.addr(), "/widgets.json", "")
        .await
        .expect("widgets.json request");
    assert_eq!(manifest.status, 200);
    assert!(manifest.json().get("example_price").is_some());

    let data = client::get(server.addr(), minimal::DATA_ENDPOINT, "")
        .await
        .expect("widget-data request");
    assert_eq!(data.status, 200);
    let rows = data.json();
    let results = rows["results"].as_array().expect("results array");
    assert_eq!(results.len(), 3, "hand-written backend returns 3 rows");

    let missing = client::get(server.addr(), "/nope", "")
        .await
        .expect("404 request");
    assert_eq!(missing.status, 404);
    server.shutdown().await;
}

// --- Example 20: catalog-derived backend ------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn example_20_derived_widgets_json_has_many_widgets() {
    let (document, count) = derived::fetch_widgets_json()
        .await
        .expect("derived widgets.json");
    assert!(count >= 60, "the catalog derives 60+ widgets, got {count}");
    let widget = &document["equity_price_historical"];
    assert_eq!(widget["type"], "chart");
    assert_eq!(widget["endpoint"], "/widget-data/equity/price/historical");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn example_20_widget_data_resolves_fileset_offline() {
    let envelope = derived::fetch_equity_historical()
        .await
        .expect("equity historical");
    let results = envelope["results"].as_array().expect("results array");
    assert!(!results.is_empty(), "fileset fixture yields rows offline");
    assert_eq!(results[0]["symbol"], "AAPL");
}

// --- Example 30: apps.json --------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn example_30_curated_apps_json_names_the_default_app() {
    let document = app::fetch_curated_apps_json()
        .await
        .expect("curated apps.json");
    assert!(
        document.get("FinX Market Overview").is_some(),
        "the curated default app is served"
    );
}

#[test]
fn example_30_custom_app_composes_two_tabs() {
    let custom = app::custom_app();
    assert_eq!(custom.name, "Rates & Macro Board");
    assert_eq!(custom.tabs.len(), 2, "macro + equity tabs");
    assert!(custom.tabs.contains_key("macro"));
    assert!(custom.tabs.contains_key("equity"));

    // The merged document carries both apps, keyed by name.
    let merged = app::apps_document();
    let names = merged.as_object().expect("apps object");
    assert!(names.contains_key("FinX Market Overview"));
    assert!(names.contains_key("Rates & Macro Board"));
}

// --- Example 40: minimal copilot --------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn example_40_agents_json_advertises_streaming_copilot() {
    let manifest = agent_echo::fetch_agents_json().await.expect("agents.json");
    let object = manifest.as_object().expect("agents.json object");
    assert_eq!(object.len(), 1, "exactly one default copilot");
    let (_id, entry) = object.iter().next().expect("one entry");
    assert_eq!(entry["features"]["streaming"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn example_40_no_widget_question_streams_reasoning_then_chunks() {
    let response = agent_echo::ask("What is a P/E ratio?").await.expect("ask");
    assert_eq!(response.status, 200);
    assert!(
        response.headers.contains("Content-Type: text/event-stream"),
        "SSE content type; headers: {}",
        response.headers
    );
    let names = response.sse_event_names();
    assert_eq!(names.first().map(String::as_str), Some("reasoning_step"));
    assert!(
        names.iter().any(|name| name == "message_chunk"),
        "streams the answer; events: {names:?}"
    );
    assert!(
        !names.iter().any(|name| name == "get_widget_data"),
        "no primary widget => no widget-data request"
    );
}

// --- Example 50: reasoning + citations (pure sequencer) ---------------------

#[test]
fn example_50_grounded_turn_emits_reasoning_chunks_and_citations() {
    let names: Vec<&str> = reasoning_citations::answered_turn()
        .iter()
        .map(SseEvent::event_name)
        .collect();
    assert_eq!(names.first(), Some(&"reasoning_step"));
    assert!(names.contains(&"message_chunk"), "streams the answer");
    assert!(names.contains(&"citations"), "attributes its sources");
    assert_eq!(
        names.last(),
        Some(&"prompt_suggestions"),
        "closes with follow-up suggestions"
    );

    // The golden SSE transcript ends with a citation to the example's widget.
    let transcript = reasoning_citations::sse_transcript();
    assert!(transcript.contains("event: citations"));
    assert!(
        transcript.contains(reasoning_citations::WIDGET_UUID),
        "citation attributes the source widget"
    );
}

// --- Example 60: two-request widget-data round trip -------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn example_60_two_request_round_trip() {
    let round_trip = widget_data::run().await.expect("round trip");

    // Leg 1: a get_widget_data request, no answer yet.
    assert_eq!(round_trip.leg1.status, 200);
    let names1 = round_trip.leg1.sse_event_names();
    assert!(
        names1.iter().any(|name| name == "get_widget_data"),
        "leg 1 requests widget data; events: {names1:?}"
    );
    assert!(
        !names1.iter().any(|name| name == "message_chunk"),
        "leg 1 does not answer yet"
    );

    // Leg 2: the grounded answer ending in citations, carrying the folded rows.
    assert_eq!(round_trip.leg2.status, 200);
    let names2 = round_trip.leg2.sse_event_names();
    assert!(
        names2.iter().any(|name| name == "message_chunk"),
        "leg 2 answers; events: {names2:?}"
    );
    assert!(
        names2.iter().any(|name| name == "citations"),
        "leg 2 attributes its sources; events: {names2:?}"
    );
    assert_eq!(
        names2.last().map(String::as_str),
        Some("prompt_suggestions"),
        "leg 2 closes with follow-up suggestions; events: {names2:?}"
    );
    assert!(
        round_trip.leg2.sse_data().contains("185.6"),
        "folded widget data reaches the answer context"
    );
}

// --- Example 80: knowledge graph-visualization widget (K-M6) ----------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn example_80_serves_a_bounded_ego_graph_from_the_live_backend() {
    let payload = graph::fetch_ego_graph().await.expect("ego-graph");
    let block = &payload["graph"];
    // Root + two neighbors, two edges — a bounded ego-graph, not unbounded.
    assert_eq!(block["node_count"], 3, "root + 2 neighbors");
    assert_eq!(block["edge_count"], 2);
    // The markdown widget's dataKey carries a rendered graph naming the root.
    let markdown = payload["results"].as_str().expect("results markdown");
    assert!(markdown.contains(graph::ROOT), "markdown names the root");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn example_80_absent_root_is_an_honest_empty_graph() {
    let payload = graph::fetch_empty_graph().await.expect("empty graph");
    let block = &payload["graph"];
    assert_eq!(
        block["node_count"], 0,
        "no fabricated nodes for an absent root"
    );
    assert_eq!(block["edge_count"], 0);
    assert!(
        block["nodes"].as_array().expect("nodes array").is_empty(),
        "honest empty: no nodes invented"
    );
}

// --- Example 70: table + chart artifacts ------------------------------------

#[test]
fn example_70_emits_table_then_chart_artifacts() {
    let names: Vec<&str> = charts_tables::artifacts()
        .iter()
        .map(SseEvent::event_name)
        .collect();
    assert_eq!(names, vec!["table", "chart"]);

    let transcript = charts_tables::sse_transcript();
    assert!(transcript.contains("event: table"));
    assert!(transcript.contains("event: chart"));

    // The reused Plotly builder produces a candlestick figure-spec.
    let figure = charts_tables::plotly_candlestick();
    let traces = figure["data"].as_array().expect("figure data array");
    assert!(
        traces.iter().any(|trace| trace["type"] == "candlestick"),
        "candlestick trace present"
    );
}
