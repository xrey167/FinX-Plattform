//! Route resolution bounded by the catalog (the partner design §1.3 step 1, §1.4).
//!
//! This is the **anti-over-engineering line**: route resolution is LLM
//! tool-selection *bounded by [`tdw_endpoint_catalog::catalog`]* plus the
//! `tdw.kg.*` knowledge verb set — NOT a free-form planner. The model picks from
//! the 268 catalog routes + the knowledge verbs, and an invalid pick is rejected
//! by [`tdw_endpoint_catalog::is_valid_route`] (data routes) or the known-verb
//! set (knowledge routes) **before any I/O**. There is no graph executor and no
//! agent loop here — just "classify intent, choose route(s), guard them".
//!
//! # Parameters (Gemini #438 critical fix)
//!
//! Resolution extracts BOTH the route AND its parameters from the utterance. The
//! model may append a JSON object after a route (`equity/price/historical
//! {"symbol":"AAPL"}`); whatever it omits is back-filled by a deterministic
//! [`extract_params`] pass that mines a symbol and a date window from the
//! utterance. The resolved [`ResolvedRoute`] carries those params so
//! `PartnerCore::turn` threads them into [`crate::DataPlane::fetch`] — a real
//! data route (e.g. `equity/price/historical`, which needs a `symbol`) is no
//! longer fetched with empty params.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tdw_llm::{ChatMessage, ChatRequest, LanguageModel, MessageRole};

/// The knowledge verbs the resolver may select alongside catalog data routes.
///
/// These are the read-side `tdw.kg.*` tools Partner Core composes for the
/// memory-context step (the partner design §1.3 step 2). They are not catalog routes
/// (so `is_valid_route` does not apply); the resolver guards them against this
/// fixed set instead.
pub const KNOWLEDGE_VERBS: &[&str] = &["tdw.kg.search", "tdw.kg.answer", "tdw.kg.why"];

/// Whether `route` is a knowledge verb the resolver may select.
#[must_use]
pub fn is_knowledge_verb(route: &str) -> bool {
    KNOWLEDGE_VERBS.contains(&route)
}

/// Whether `route` is a selectable target: a valid catalog data route OR a
/// known knowledge verb. Anything else is rejected before dispatch.
#[must_use]
pub fn is_selectable(route: &str) -> bool {
    is_knowledge_verb(route) || tdw_endpoint_catalog::is_valid_route(route)
}

/// One resolved catalog data route plus the parameters extracted for it
/// (the partner design §1.3 step 1; Gemini #438 critical fix).
///
/// The `params` object is the request arguments threaded into
/// [`crate::DataPlane::fetch`]. It is bounded by the route's catalog schema
/// shape (`symbol` plus the `StandardParams` window) and is never free-form.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedRoute {
    /// The catalog route (passed [`tdw_endpoint_catalog::is_valid_route`]).
    pub route: String,
    /// The request parameters for the route (e.g. `{"symbol":"AAPL"}`).
    pub params: Value,
}

impl ResolvedRoute {
    /// Whether the resolved params object carries no fields.
    #[must_use]
    pub fn params_empty(&self) -> bool {
        self.params.as_object().is_none_or(Map::is_empty)
    }
}

/// The resolved targets for a turn: the guarded data routes (each with its
/// extracted params) and knowledge verbs the turn will execute, in selection
/// order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedRoutes {
    /// Catalog data routes (each one passed [`tdw_endpoint_catalog::is_valid_route`]
    /// AND [`tdw_endpoint_catalog::lookup`]), each carrying its parameters.
    pub data: Vec<ResolvedRoute>,
    /// Knowledge verbs (`tdw.kg.*`) selected for the memory-context step.
    pub knowledge: Vec<String>,
}

impl ResolvedRoutes {
    /// Whether nothing resolved (a pure knowledge-free, data-free chat turn).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.data.is_empty() && self.knowledge.is_empty()
    }

    /// The resolved route names (without params), in selection order.
    ///
    /// A convenience for surfaces that only render the route ids (the citation
    /// line, the structured `resolved_routes` block).
    #[must_use]
    pub fn data_routes(&self) -> Vec<String> {
        self.data.iter().map(|r| r.route.clone()).collect()
    }
}

/// Resolve `utterance` into a guarded set of routes (with params) using `model`
/// for selection.
///
/// The model is shown the candidate vocabulary and asked to echo the routes it
/// would use, one per line, optionally followed by a JSON params object. Every
/// line is then guarded:
/// - a data route is kept only if [`tdw_endpoint_catalog::lookup`] finds it
///   (which also implies `is_valid_route`); its params are the model-supplied
///   object merged over a deterministic [`extract_params`] pass so a real data
///   route (needing e.g. `symbol`) is never fetched with empty params;
/// - a knowledge verb is kept only if it is in [`KNOWLEDGE_VERBS`];
/// - anything else (a hallucinated or malformed route) is dropped.
///
/// So an invalid pick can never reach the dispatcher. On a model error the
/// resolver returns an empty set (the turn answers directly without data),
/// never a panic.
#[must_use]
pub fn resolve_routes(utterance: &str, model: &dyn LanguageModel) -> ResolvedRoutes {
    let request = build_select_request(utterance);
    let Ok(response) = model.complete(request) else {
        return ResolvedRoutes::default();
    };
    guard_selection(&response.message.content, utterance)
}

/// Build the bounded tool-selection prompt. The system message lists the
/// knowledge verbs explicitly and instructs the model to pick only from the
/// catalog grammar; the guard enforces it regardless of what the model returns.
fn build_select_request(utterance: &str) -> ChatRequest {
    let system = format!(
        "You are a route selector for a financial data warehouse. Given the user's question, \
         list the data routes and knowledge verbs needed to answer it, ONE PER LINE, and nothing \
         else. Data routes are slash-namespaced catalog routes (e.g. equity/price/historical); \
         you MAY append a JSON object of parameters after a data route on the same line \
         (e.g. equity/price/historical {{\"symbol\":\"AAPL\"}}). Knowledge verbs are exactly: \
         {verbs}. If no data or knowledge lookup is needed, output nothing.",
        verbs = KNOWLEDGE_VERBS.join(", "),
    );
    ChatRequest {
        messages: vec![
            ChatMessage {
                role: MessageRole::System,
                content: system,
            },
            ChatMessage {
                role: MessageRole::User,
                content: utterance.to_string(),
            },
        ],
        max_output_tokens: 256,
    }
}

/// Guard a newline-delimited model selection against the catalog + verb set,
/// extracting per-route params from the line and the originating `utterance`.
fn guard_selection(selection: &str, utterance: &str) -> ResolvedRoutes {
    let mut resolved = ResolvedRoutes::default();
    for raw in selection.lines() {
        let (candidate, inline_params) = split_line(raw);
        if candidate.is_empty() {
            continue;
        }
        if is_knowledge_verb(&candidate) {
            if !resolved.knowledge.iter().any(|v| v == &candidate) {
                resolved.knowledge.push(candidate);
            }
        } else if tdw_endpoint_catalog::lookup(&candidate).is_some() {
            // lookup() implies is_valid_route(): an invalid route never reaches
            // the dispatcher.
            if resolved.data.iter().any(|r| r.route == candidate) {
                continue;
            }
            // Params: the deterministic extraction from the utterance, with any
            // inline JSON the model supplied merged ON TOP (model wins per key).
            let mut params = extract_params(&candidate, utterance);
            merge_params(&mut params, inline_params);
            resolved.data.push(ResolvedRoute {
                route: candidate,
                params,
            });
        }
        // Anything else (hallucinated / malformed) is silently dropped — the
        // guard is the safety net, not the model.
    }
    resolved
}

/// Split one selection line into `(route, inline_params)`.
///
/// The route is the leading token, run through [`clean_candidate`] so numbered
/// (`1. route`), bulleted (`- route`), and backticked / quoted (`` `route` ``)
/// list formats all yield the bare route (Gemini #438 medium fix). Any `{...}`
/// JSON object after the route token is parsed as the inline params.
fn split_line(raw: &str) -> (String, Option<Value>) {
    let trimmed = raw.trim();
    // Carve off an inline JSON params object if present. Parse only the substring
    // from the FIRST `{` to the LAST `}` (Gemini #439 fix): a model may append
    // trailing prose after the JSON object (`… {"symbol":"AAPL"} for last month`),
    // and `serde_json::from_str` rejects any trailing content. Bounding the parse
    // to `[first '{' ..= last '}']` tolerates that trailing text; the result is
    // still filtered to an object, so a non-object span is dropped, not threaded.
    let (head, params) = trimmed.find('{').map_or((trimmed, None), |open| {
        let before = &trimmed[..open];
        let parsed = trimmed[open..]
            .rfind('}')
            .and_then(|close_rel| {
                // close_rel is relative to the `[open..]` slice; the end-exclusive
                // index into `trimmed[open..]` is `close_rel + 1`.
                serde_json::from_str::<Value>(&trimmed[open..][..=close_rel]).ok()
            })
            .filter(Value::is_object);
        (before, parsed)
    });
    (clean_candidate(head), params)
}

/// Normalize a route token from an LLM list item to its bare form.
///
/// Robust against the formats a model actually emits (Gemini #438 medium fix):
/// strips a leading ordered-list marker (`1.`, `2)`), bullet markers
/// (`-`, `*`, `•`), and any surrounding backticks / single or double quotes /
/// whitespace. The previous trimmer only stripped `-`/`*`/space bullets, so
/// numbered and backticked items were silently dropped.
fn clean_candidate(raw: &str) -> String {
    let mut s = raw.trim();

    // Strip a leading ordered-list marker: digits then a `.` or `)` separator.
    let digits = s.trim_start_matches(|c: char| c.is_ascii_digit());
    if digits.len() < s.len() {
        let after = digits.trim_start_matches(['.', ')']);
        if after.len() < digits.len() {
            s = after;
        }
    }

    // Strip bullet markers and surrounding whitespace/backticks/quotes from
    // both ends.
    s.trim()
        .trim_start_matches(['-', '*', '•'])
        .trim()
        .trim_matches(['`', '"', '\'', ' '])
        .to_string()
}

/// Merge `overlay` (the model's inline params) on top of `base` (the
/// deterministic extraction). Object keys in `overlay` win.
fn merge_params(base: &mut Value, overlay: Option<Value>) {
    let (Some(base_obj), Some(Value::Object(overlay_obj))) = (base.as_object_mut(), overlay) else {
        return;
    };
    for (key, value) in overlay_obj {
        base_obj.insert(key, value);
    }
}

/// Deterministically extract the parameters a `route` needs from the raw
/// `utterance` (Gemini #438 critical fix).
///
/// Catalog routes standardize on a `symbol` plus the `StandardParams` window
/// (`start_date` / `end_date` / `interval` / `limit`). The catalog itself does
/// not enumerate `symbol` per route, so it is mined here from the utterance: the
/// first bare uppercase ticker-shaped token (1–5 A–Z chars) becomes `symbol`.
/// A coarse relative-window phrase (`last month` / `last week` / `last year`)
/// sets a `window` hint the dispatcher can normalize. The result is always a
/// JSON object (possibly empty for a route that genuinely needs no params).
#[must_use]
pub fn extract_params(route: &str, utterance: &str) -> Value {
    let mut params = Map::new();

    // Symbol: the first uppercase ticker-shaped token (1–5 A–Z). Routes that do
    // not take a symbol simply ignore it downstream, but most equity/crypto/etf
    // data routes need exactly this.
    if route_takes_symbol(route)
        && let Some(symbol) = extract_symbol(utterance)
    {
        params.insert("symbol".to_string(), Value::String(symbol));
    }

    // Coarse relative window hint (the dispatcher normalizes to start/end dates).
    if let Some(window) = extract_window(utterance) {
        params.insert("window".to_string(), Value::String(window));
    }

    Value::Object(params)
}

/// Whether a catalog `route` is one that takes a `symbol` parameter.
///
/// Conservative and structural: the equity / crypto / etf / index / derivative
/// data families are all symbol-keyed. Anything else (e.g. an economy macro
/// route) does not get a synthesized symbol.
fn route_takes_symbol(route: &str) -> bool {
    const SYMBOL_PREFIXES: &[&str] = &[
        "equity/",
        "crypto/",
        "etf/",
        "index/",
        "currency/",
        "derivatives/",
        "fixedincome/",
    ];
    SYMBOL_PREFIXES
        .iter()
        .any(|prefix| route.starts_with(prefix))
}

/// Extract the first ticker-shaped token (1–5 ASCII uppercase letters) from the
/// utterance, skipping common English stop-words that happen to be uppercase
/// (e.g. a leading "I"). Returns `None` when no plausible ticker is present.
fn extract_symbol(utterance: &str) -> Option<String> {
    utterance
        .split(|c: char| !c.is_ascii_alphanumeric())
        .find(|token| is_ticker_shaped(token))
        .map(ToString::to_string)
}

/// Whether `token` looks like a ticker: 1–5 chars, all ASCII uppercase letters,
/// and not a trivial English word that would be a false positive.
fn is_ticker_shaped(token: &str) -> bool {
    let len = token.chars().count();
    (1..=5).contains(&len)
        && token.chars().all(|c| c.is_ascii_uppercase())
        && !TICKER_STOP_WORDS.contains(&token)
}

/// Uppercase English words that look like tickers but almost never are.
///
/// Includes common macro / geo / role acronyms (`US`, `EU`, `CEO`, `CPI`, `GDP`,
/// `ETF`, …) so a question like "What is US CPI doing?" does not mine `US` as a
/// symbol on a symbol-keyed route (post-release LOW finding, v1.7.1). The
/// route-family guard (`route_takes_symbol`) already excludes economy routes; this
/// narrows the residual equity/* false-positive surface.
const TICKER_STOP_WORDS: &[&str] = &[
    "I", "A", "AN", "THE", "OK", "PE", "P", "US", "EU", "UK", "CEO", "CFO", "CPI", "GDP", "Q",
    "YTD", "ETF", "IPO", "FED", "ECB",
];

/// Coarse relative-window phrases mapped to their normalized window codes.
const RELATIVE_WINDOWS: &[(&str, &str)] = &[
    ("last month", "1m"),
    ("past month", "1m"),
    ("last week", "1w"),
    ("past week", "1w"),
    ("last year", "1y"),
    ("past year", "1y"),
    ("year to date", "ytd"),
    ("ytd", "ytd"),
];

/// Extract a coarse relative window phrase from the utterance.
fn extract_window(utterance: &str) -> Option<String> {
    let lower = utterance.to_ascii_lowercase();
    RELATIVE_WINDOWS
        .iter()
        .find(|(phrase, _)| lower.contains(phrase))
        .map(|(_, code)| (*code).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_llm::{ChatResponse, Result as LlmResult, StreamingLanguageModel, Usage};

    /// A scripted model that returns a fixed selection string verbatim — the
    /// offline golden-query stub for the routing eval.
    struct ScriptedModel(&'static str);

    impl LanguageModel for ScriptedModel {
        fn model_id(&self) -> &'static str {
            "scripted-resolver"
        }

        fn complete(&self, request: ChatRequest) -> LlmResult<ChatResponse> {
            let _ = tdw_llm::last_user_message(&request)?;
            Ok(ChatResponse {
                model_id: self.model_id().to_string(),
                message: ChatMessage {
                    role: MessageRole::Assistant,
                    content: self.0.to_string(),
                },
                usage: Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
            })
        }
    }

    impl StreamingLanguageModel for ScriptedModel {}

    #[test]
    fn keeps_only_valid_catalog_routes_and_known_verbs() {
        // The model proposes a real route, a knowledge verb, and a hallucinated
        // route; the guard keeps the first two and drops the third.
        let model = ScriptedModel(
            "equity/price/historical\ntdw.kg.answer\nnot/a/real/route/that/exists/anywhere",
        );
        let resolved = resolve_routes("How did AAPL do and what do we know?", &model);
        assert!(
            resolved
                .data_routes()
                .contains(&"equity/price/historical".to_string())
        );
        assert!(resolved.knowledge.contains(&"tdw.kg.answer".to_string()));
        assert!(
            !resolved.data.iter().any(|r| r.route.contains("not/a/real")),
            "hallucinated route is guarded out: {resolved:?}"
        );
    }

    #[test]
    fn invalid_route_is_never_selectable() {
        // The grammar guard: an uppercase / malformed route is rejected even if
        // the model emits it.
        assert!(!is_selectable("Equity/Price"));
        assert!(!is_selectable("equity//price"));
        assert!(!is_selectable("tdw.kg.unknownverb"));
        // A real catalog route and a real verb are selectable.
        assert!(is_selectable("equity/price/historical"));
        assert!(is_selectable("tdw.kg.search"));
    }

    #[test]
    fn empty_selection_resolves_to_nothing() {
        let model = ScriptedModel("");
        let resolved = resolve_routes("What is a P/E ratio?", &model);
        assert!(resolved.is_empty());
    }

    #[test]
    fn dedupes_repeated_picks() {
        let model = ScriptedModel("equity/price/historical\nequity/price/historical");
        let resolved = resolve_routes("AAPL?", &model);
        assert_eq!(resolved.data.len(), 1);
    }

    // ── Gemini #438 CRITICAL: parameterized fetch ───────────────────────────

    #[test]
    fn parameterized_query_yields_symbol_in_params() {
        // "AAPL price last month" must resolve a data route whose params carry
        // the symbol (so the DataPlane fetch is no longer parameterless).
        let model = ScriptedModel("equity/price/historical");
        let resolved = resolve_routes("AAPL price last month", &model);
        let route = resolved
            .data
            .iter()
            .find(|r| r.route == "equity/price/historical")
            .expect("route resolved");
        assert!(
            !route.params_empty(),
            "params must be non-empty for a parameterized query: {:?}",
            route.params
        );
        assert_eq!(
            route.params.get("symbol").and_then(Value::as_str),
            Some("AAPL"),
            "symbol extracted from the utterance: {:?}",
            route.params
        );
        // The relative window is captured too.
        assert_eq!(
            route.params.get("window").and_then(Value::as_str),
            Some("1m"),
            "relative window extracted: {:?}",
            route.params
        );
    }

    #[test]
    fn model_inline_params_win_over_extraction() {
        // The model may pin params explicitly; those override the extraction.
        let model = ScriptedModel("equity/price/historical {\"symbol\":\"MSFT\",\"limit\":5}");
        let resolved = resolve_routes("how did AAPL do", &model);
        let route = &resolved.data[0];
        assert_eq!(
            route.params.get("symbol").and_then(Value::as_str),
            Some("MSFT"),
            "model-supplied symbol wins: {:?}",
            route.params
        );
        assert_eq!(route.params.get("limit").and_then(Value::as_i64), Some(5));
    }

    #[test]
    fn extract_params_skips_symbol_for_non_symbol_routes() {
        // A macro/economy route is not symbol-keyed, so no symbol is synthesized
        // even when the utterance contains an uppercase token.
        let params = extract_params("economy/cpi", "What is US CPI doing?");
        assert!(
            params.get("symbol").is_none(),
            "non-symbol route gets no symbol: {params:?}"
        );
    }

    // ── Gemini #438 MEDIUM: robust list trimmer ─────────────────────────────

    #[test]
    fn numbered_and_backticked_list_formats_are_kept() {
        // The previous trimmer dropped numbered (`1.`) and backticked items.
        let model = ScriptedModel(
            "1. equity/price/historical\n2) equity/profile\n`tdw.kg.answer`\n- \"tdw.kg.why\"",
        );
        let resolved = resolve_routes("AAPL deep dive", &model);
        let routes = resolved.data_routes();
        assert!(
            routes.contains(&"equity/price/historical".to_string()),
            "numbered '1.' item kept: {routes:?}"
        );
        assert!(
            routes.contains(&"equity/profile".to_string()),
            "numbered '2)' item kept: {routes:?}"
        );
        assert!(
            resolved.knowledge.contains(&"tdw.kg.answer".to_string()),
            "backticked verb kept: {:?}",
            resolved.knowledge
        );
        assert!(
            resolved.knowledge.contains(&"tdw.kg.why".to_string()),
            "quoted verb kept: {:?}",
            resolved.knowledge
        );
    }

    // ── Gemini #439: tolerate trailing text after the inline JSON object ──────

    #[test]
    fn inline_params_parse_with_trailing_text() {
        // The model appends prose after the JSON object; the parser must still
        // extract the params (bounded to first `{` .. last `}`) rather than
        // failing the whole parse and dropping the params.
        let model =
            ScriptedModel("equity/price/historical {\"symbol\":\"MSFT\"} for the last month");
        let resolved = resolve_routes("how did it do", &model);
        let route = resolved
            .data
            .iter()
            .find(|r| r.route == "equity/price/historical")
            .expect("route resolved despite trailing text");
        assert_eq!(
            route.params.get("symbol").and_then(Value::as_str),
            Some("MSFT"),
            "symbol parsed from JSON preceding trailing prose: {:?}",
            route.params
        );
    }

    #[test]
    fn split_line_bounds_json_to_first_open_and_last_close() {
        // Directly: the JSON span is [first '{' ..= last '}'], trailing text after
        // the closing brace is ignored.
        let (route, params) = split_line("equity/profile {\"symbol\":\"AAPL\"} trailing junk");
        assert_eq!(route, "equity/profile");
        let params = params.expect("params parsed past trailing text");
        assert_eq!(params.get("symbol").and_then(Value::as_str), Some("AAPL"));
    }

    #[test]
    fn split_line_drops_non_object_json_span() {
        // A `{...}` span that is not a JSON object is dropped, not threaded as
        // params (the `is_object` filter is preserved through the new parser).
        let (route, params) = split_line("equity/quote {not valid json at all}");
        assert_eq!(route, "equity/quote");
        assert!(
            params.is_none(),
            "non-object span yields no params: {params:?}"
        );
    }

    #[test]
    fn clean_candidate_strips_decorations() {
        assert_eq!(
            clean_candidate("1. equity/price/historical"),
            "equity/price/historical"
        );
        assert_eq!(clean_candidate("2) equity/profile"), "equity/profile");
        assert_eq!(clean_candidate("- equity/quote"), "equity/quote");
        assert_eq!(clean_candidate("`tdw.kg.answer`"), "tdw.kg.answer");
        assert_eq!(clean_candidate("\"tdw.kg.why\""), "tdw.kg.why");
        assert_eq!(
            clean_candidate("  equity/price/historical  "),
            "equity/price/historical"
        );
    }
}
