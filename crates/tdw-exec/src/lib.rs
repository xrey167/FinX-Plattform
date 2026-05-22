#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::error::Error;
use std::fmt;
use tdw_protocol::{EventMsg, Op, OpEnvelope};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExecRun {
    pub events: Vec<EventMsg>,
}

pub type Result<T> = std::result::Result<T, ExecError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecError {
    EmptySql,
    MultipleStatements,
    UnsafeSqlToken,
    NonReadOnlySql,
}

impl fmt::Display for ExecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySql => write!(formatter, "SQL must not be empty"),
            Self::MultipleStatements => {
                write!(formatter, "multiple SQL statements are not allowed")
            }
            Self::UnsafeSqlToken => write!(formatter, "unsafe SQL token is not allowed"),
            Self::NonReadOnlySql => write!(formatter, "only read-only SELECT SQL is supported"),
        }
    }
}

impl Error for ExecError {}

pub fn try_run_headless(envelope: OpEnvelope) -> Result<ExecRun> {
    validate_op(&envelope.op)?;
    Ok(run_headless(envelope))
}

pub fn run_headless(envelope: OpEnvelope) -> ExecRun {
    let mut events = vec![EventMsg::Started {
        op_id: envelope.op_id.clone(),
    }];
    events.push(match envelope.op {
        Op::RunQuery { sql, .. } => EventMsg::Completed {
            op_id: envelope.op_id,
            summary: Some("query planned".to_string()),
            result: Some(json!({ "sql": sql })),
        },
        other => EventMsg::Completed {
            op_id: envelope.op_id,
            summary: Some("op accepted".to_string()),
            result: Some(json!({ "op": other })),
        },
    });
    ExecRun { events }
}

pub fn validate_op(op: &Op) -> Result<()> {
    match op {
        Op::RunQuery { sql, .. } => validate_read_only_sql(sql),
        _ => Ok(()),
    }
}

fn validate_read_only_sql(sql: &str) -> Result<()> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err(ExecError::EmptySql);
    }
    if trimmed.chars().any(char::is_control) {
        return Err(ExecError::UnsafeSqlToken);
    }

    let semicolon_count = trimmed.matches(';').count();
    if semicolon_count > 1 || (semicolon_count == 1 && !trimmed.ends_with(';')) {
        return Err(ExecError::MultipleStatements);
    }

    let without_trailing_semicolon = trimmed.strip_suffix(';').unwrap_or(trimmed).trim_end();
    let lower = without_trailing_semicolon.to_ascii_lowercase();
    if !lower.starts_with("select ") && lower != "select" {
        return Err(ExecError::NonReadOnlySql);
    }
    if [
        "--", "/*", "*/", " drop ", " delete ", " insert ", " update ",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return Err(ExecError::UnsafeSqlToken);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_protocol::{ActorKind, ActorRef, SessionId};

    #[test]
    fn headless_exec_returns_protocol_events() {
        let envelope = OpEnvelope::new(
            SessionId::new("session-1").expect("session id"),
            1,
            ActorRef {
                actor_id: "user".to_string(),
                kind: ActorKind::User,
                tenant_id: None,
            },
            Op::RunQuery {
                sql: "select 1".to_string(),
                plan_id: None,
                cost_hint: None,
            },
        );
        let run = run_headless(envelope);

        assert!(matches!(run.events[0], EventMsg::Started { .. }));
        assert!(matches!(run.events[1], EventMsg::Completed { .. }));
    }

    #[test]
    fn checked_headless_exec_rejects_mutating_queries() {
        let envelope = OpEnvelope::new(
            SessionId::new("session-1").expect("session id"),
            1,
            ActorRef {
                actor_id: "user".to_string(),
                kind: ActorKind::User,
                tenant_id: None,
            },
            Op::RunQuery {
                sql: "delete from raw.orders".to_string(),
                plan_id: None,
                cost_hint: None,
            },
        );

        assert_eq!(try_run_headless(envelope), Err(ExecError::NonReadOnlySql));
    }
}
