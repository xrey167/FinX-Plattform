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
use tdw_partner::{
    AuditInputs, BriefInputs, DataPlane, DataPlaneError, EscalationConfig, PartnerCore,
    PartnerEvent, PartnerTurn, Principal, audit_feed, build_brief,
};

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

/// Route a `partner {ask,brief,audit}` subcommand (partner-system W2.8/W3.6/W5.5).
///
/// Returns `Some(result)` when `args` names a partner subcommand (so `main`
/// returns it), or `None` when it is not a partner command (so `main` falls
/// through to the daemon dispatch). Keeps the three offline partner surfaces in
/// one place and `main` slim.
pub async fn dispatch(args: &[String]) -> Option<Result<(), CliError>> {
    let is = |verb: &str| {
        args.windows(2)
            .any(|pair| pair[0] == "partner" && pair[1] == verb)
    };
    if is("ask") {
        Some(run(args).await)
    } else if is("brief") {
        Some(run_brief(args))
    } else if is("audit") {
        Some(run_audit(args))
    } else {
        None
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

/// Run `tdw partner brief [<inputs-json>]`: assemble and render the proactive
/// morning brief (partner-system W3.6).
///
/// A *thin adapter* over the pure [`build_brief`] assembler: it parses the
/// gathered signal sources ([`BriefInputs`]) from the optional trailing JSON
/// token (defaulting to an empty brief when none is supplied — the offline
/// smoke), ranks them, and prints the ranked nudges. The daily-scheduled variant
/// runs the same assembler off a `tdw-cron` trigger in the daemon (W3.3); this
/// command is the on-demand CLI surface.
///
/// # Errors
///
/// Returns a [`CliError`] when the supplied inputs JSON is malformed.
pub fn run_brief(args: &[String]) -> Result<(), CliError> {
    let inputs = parse_brief_inputs(args)?;
    let principal = Principal::new("cli-session", "agent:partner");
    let brief = build_brief(&inputs, &principal);

    if brief.is_empty() {
        println!("(no nudges — nothing needs you right now)");
        return Ok(());
    }
    println!("Partner brief — {} nudge(s):", brief.len());
    for nudge in &brief {
        println!(
            "• [{severity:?}] {headline}",
            severity = nudge.severity,
            headline = nudge.headline,
        );
    }
    Ok(())
}

/// Run `tdw partner audit [<inputs-json>]`: project and render the audit-only
/// autonomy feed (partner-system W5.5).
///
/// A *thin adapter* over the pure [`audit_feed`] projection: it parses the
/// gathered source records ([`AuditInputs`]) from the optional trailing JSON
/// token (defaulting to an empty feed when none is supplied — the offline smoke),
/// applies the default escalation predicate, and prints each action with its
/// status and a one-line `why`. The audit feed is a projection over records that
/// already exist (no new store); this command is the on-demand CLI surface.
///
/// # Errors
///
/// Returns a [`CliError`] when the supplied inputs JSON is malformed.
pub fn run_audit(args: &[String]) -> Result<(), CliError> {
    let inputs = parse_audit_inputs(args)?;
    let principal = Principal::new("cli-session", "agent:partner");
    let feed = audit_feed(&inputs, &principal, &EscalationConfig::default());

    if feed.is_empty() {
        println!("(no audited actions — the partner has done nothing to review)");
        return Ok(());
    }
    let awaiting = feed
        .iter()
        .filter(|record| record.status == tdw_partner::ActionStatus::AwaitingHuman)
        .count();
    println!(
        "Partner audit — {total} action(s), {awaiting} awaiting you:",
        total = feed.len(),
    );
    for record in &feed {
        let why = record.why.routes.first().map_or("", String::as_str);
        println!(
            "• [{status:?}] {kind:?} — why: {why}",
            status = record.status,
            kind = record.what,
        );
    }
    Ok(())
}

/// Parse the optional `AuditInputs` JSON token after `partner audit`.
///
/// No token (or an empty one) yields the default empty inputs so the command is a
/// self-contained offline smoke. A malformed token is a [`CliError`].
fn parse_audit_inputs(args: &[String]) -> Result<AuditInputs, CliError> {
    let Some(pos) = args
        .windows(2)
        .position(|pair| pair[0] == "partner" && pair[1] == "audit")
    else {
        return Ok(AuditInputs::default());
    };
    let rest = args[pos + 2..].join(" ");
    if rest.trim().is_empty() {
        return Ok(AuditInputs::default());
    }
    serde_json::from_str::<AuditInputs>(&rest)
        .map_err(|error| format!("malformed audit inputs JSON: {error}").into())
}

/// Parse the optional `BriefInputs` JSON token after `partner brief`.
///
/// No token (or an empty one) yields the default empty inputs so the command is
/// a self-contained offline smoke. A malformed token is a [`CliError`].
fn parse_brief_inputs(args: &[String]) -> Result<BriefInputs, CliError> {
    let Some(pos) = args
        .windows(2)
        .position(|pair| pair[0] == "partner" && pair[1] == "brief")
    else {
        return Ok(BriefInputs::default());
    };
    let rest = args[pos + 2..].join(" ");
    if rest.trim().is_empty() {
        return Ok(BriefInputs::default());
    }
    serde_json::from_str::<BriefInputs>(&rest)
        .map_err(|error| format!("malformed brief inputs JSON: {error}").into())
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

    #[test]
    fn brief_runs_offline_with_no_inputs() {
        // The empty brief is the offline smoke — it renders the honest "nothing
        // needs you" line without panicking.
        let args = argv(&["tdw", "partner", "brief"]);
        run_brief(&args).expect("offline partner brief smoke runs");
    }

    #[test]
    fn brief_parses_inline_inputs_json() {
        let json = r#"{"alerts":[{"id":"a-1","symbol":"AAPL","headline":"AAPL crossed $200","fired_at":"2026-06-14"}]}"#;
        let args = argv(&["tdw", "partner", "brief", json]);
        let inputs = parse_brief_inputs(&args).expect("parses inputs");
        assert_eq!(inputs.alerts.len(), 1);
        assert_eq!(inputs.alerts[0].symbol, "AAPL");
        // And it renders without error.
        run_brief(&args).expect("brief renders the parsed inputs");
    }

    #[test]
    fn brief_malformed_json_is_an_error() {
        let args = argv(&["tdw", "partner", "brief", "{not json"]);
        assert!(parse_brief_inputs(&args).is_err());
    }

    #[test]
    fn audit_runs_offline_with_no_inputs() {
        // The empty audit feed is the offline smoke — it renders the honest
        // "nothing to review" line without panicking.
        let args = argv(&["tdw", "partner", "audit"]);
        run_audit(&args).expect("offline partner audit smoke runs");
    }

    #[test]
    fn audit_parses_inline_inputs_json_and_renders() {
        // A materialized Forget proposal projects to one audited action.
        let json = r#"{"proposals":[{"id":"p1","kind":{"kind":"forget","from":"company:ACME","rel":"hq_in","to":"city:old","reason":"stale"},"agent_id":"agent:partner","status":"validated","materialized":true,"history":["2026-06-14 draft by agent:partner"]}]}"#;
        let args = argv(&["tdw", "partner", "audit", json]);
        let inputs = parse_audit_inputs(&args).expect("parses inputs");
        assert_eq!(inputs.proposals.len(), 1);
        run_audit(&args).expect("audit renders the parsed inputs");
    }

    #[test]
    fn audit_malformed_json_is_an_error() {
        let args = argv(&["tdw", "partner", "audit", "{not json"]);
        assert!(parse_audit_inputs(&args).is_err());
    }
}
