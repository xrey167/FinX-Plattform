//! `tdw partner ask "<question>"` — the Partner Core front door on the CLI
//! (partner-system W2.8).
//!
//! A *thin adapter*: it builds one [`PartnerTurn`], runs it through
//! [`PartnerCore::turn`], and renders the streamed [`PartnerEvent`]s to the TTY.
//! No routing/retrieval/autonomy logic lives here — it maps events to lines,
//! exactly as the MCP and Workspace adapters map them to their wire formats. The
//! default core drives the offline deterministic `StubLanguageModel` so the
//! command is a self-contained smoke with no network or credentials.

use std::io::Write as _;
use std::sync::Arc;

use serde_json::Value;
use tdw_eval_runner::StubLanguageModel;
use tdw_partner::{DataPlane, DataPlaneError, PartnerCore, PartnerEvent, PartnerTurn, Principal};

use crate::CliError;

/// A no-op data plane for the offline CLI default.
///
/// The CLI smoke answers from the model's own context (the offline stub), so no
/// server-side fetch is wired; an honest fetch error is returned if a route is
/// ever resolved. A daemon-connected CLI would inject a real [`DataPlane`] over
/// the dispatcher here.
struct NoopDataPlane;

#[async_trait::async_trait]
impl DataPlane for NoopDataPlane {
    async fn fetch(&self, route: &str, _params: Value) -> Result<Value, DataPlaneError> {
        Err(DataPlaneError::Fetch {
            route: route.to_string(),
            message: "the offline CLI partner has no server-side data plane".to_string(),
        })
    }
}

/// Run `tdw partner ask "<question>"`: render one Partner Core turn to the TTY.
///
/// Parses the utterance from `args` (the token(s) after `ask`), runs the turn on
/// the offline core, and prints the reasoning steps, the streamed answer, and a
/// closing citation line.
///
/// # Errors
///
/// Returns a [`CliError`] when no utterance is supplied or the turn fails.
pub async fn run(args: &[String]) -> Result<(), CliError> {
    let utterance = parse_utterance(args).ok_or("usage: tdw partner ask \"<question>\"")?;

    let core = PartnerCore::new(Arc::new(StubLanguageModel), Arc::new(NoopDataPlane));
    let principal = Principal::new("cli-session", "agent:partner");
    let turn = PartnerTurn::new(principal, utterance);

    let outcome = core
        .turn(&turn, &mut render_event)
        .await
        .map_err(|error| format!("partner turn failed: {error}"))?;

    // A trailing newline after the streamed answer fragments, then the citation.
    println!();
    if outcome.provenance.is_empty() {
        println!("(no external sources cited)");
    } else {
        println!(
            "cited: routes=[{}] kg_nodes=[{}]",
            outcome.provenance.routes.join(", "),
            outcome.provenance.kg_nodes.join(", "),
        );
    }
    Ok(())
}

/// Render one [`PartnerEvent`] to the TTY.
///
/// Reasoning steps print as dimmed status lines; answer fragments stream inline
/// (no newline) so the answer renders token-by-token; the citation is handled by
/// the caller after the stream closes.
fn render_event(event: PartnerEvent) {
    match event {
        PartnerEvent::Reasoning(message) => println!("• {message}"),
        PartnerEvent::Answer(fragment) => {
            print!("{fragment}");
            // Best-effort flush so streaming is visible; ignore flush errors.
            let _ = std::io::stdout().flush();
        }
        // The closing citation is rendered by `run` from the outcome so the line
        // ordering is deterministic; nothing to print here.
        PartnerEvent::Citation(_) => {}
    }
}

/// Extract the utterance: every token after `partner ask`, joined with spaces.
fn parse_utterance(args: &[String]) -> Option<String> {
    let ask_pos = args
        .windows(2)
        .position(|pair| pair[0] == "partner" && pair[1] == "ask")?;
    let rest = &args[ask_pos + 2..];
    if rest.is_empty() {
        return None;
    }
    let joined = rest.join(" ");
    if joined.trim().is_empty() {
        None
    } else {
        Some(joined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn parses_utterance_after_ask() {
        let args = argv(&["tdw", "partner", "ask", "How", "did", "AAPL", "do?"]);
        assert_eq!(parse_utterance(&args).as_deref(), Some("How did AAPL do?"));
    }

    #[test]
    fn missing_utterance_is_none() {
        assert!(parse_utterance(&argv(&["tdw", "partner", "ask"])).is_none());
        assert!(parse_utterance(&argv(&["tdw", "partner"])).is_none());
    }

    #[tokio::test]
    async fn run_answers_offline_smoke() {
        // The smoke proves the command path runs end-to-end on the offline stub.
        let args = argv(&["tdw", "partner", "ask", "What", "is", "a", "P/E", "ratio?"]);
        run(&args).await.expect("offline partner ask smoke runs");
    }
}
