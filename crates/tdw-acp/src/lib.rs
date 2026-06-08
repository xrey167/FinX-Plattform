#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::error::Error;
use std::fmt;
use tdw_protocol::{ApprovalDecision, EventMsg, Op, PermissionId, SessionId};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpServerInfo {
    pub name: String,
    pub protocol_version: String,
    pub supports_streaming: bool,
    pub supports_approvals: bool,
}

impl Default for AcpServerInfo {
    fn default() -> Self {
        Self {
            name: "tdw-acp".to_string(),
            protocol_version: "0.1.0".to_string(),
            supports_streaming: true,
            supports_approvals: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcpRequest {
    Initialize {
        client_name: String,
    },
    SubmitOp {
        session_id: SessionId,
        op: Op,
    },
    ResolveApproval {
        permission_id: String,
        decision: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcpResponse {
    Initialized {
        server: AcpServerInfo,
    },
    Event {
        session_id: SessionId,
        event: EventMsg,
    },
    Error {
        message: String,
        data: Option<Value>,
    },
}

pub type Result<T> = std::result::Result<T, AcpValidationError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcpValidationError {
    EmptyField { field: &'static str },
    UnsafeField { field: &'static str },
    InvalidApprovalDecision { decision: String },
    InvalidQuery { reason: &'static str },
}

impl fmt::Display for AcpValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField { field } => write!(formatter, "{field} must not be empty"),
            Self::UnsafeField { field } => write!(formatter, "{field} contains unsafe characters"),
            Self::InvalidApprovalDecision { decision } => {
                write!(formatter, "unsupported approval decision: {decision}")
            }
            Self::InvalidQuery { reason } => write!(formatter, "invalid query: {reason}"),
        }
    }
}

impl Error for AcpValidationError {}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_request(request: &AcpRequest) -> Result<()> {
    match request {
        AcpRequest::Initialize { client_name } => validate_token("client_name", client_name),
        AcpRequest::SubmitOp { op, .. } => validate_op(op),
        AcpRequest::ResolveApproval {
            permission_id,
            decision,
        } => {
            validate_token("permission_id", permission_id)?;
            PermissionId::new(permission_id.clone()).map_err(|_| {
                AcpValidationError::EmptyField {
                    field: "permission_id",
                }
            })?;
            parse_approval_decision(decision).map(|_| ())
        }
    }
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn parse_approval_decision(value: &str) -> Result<ApprovalDecision> {
    let normalized = value.trim().replace('-', "_").to_ascii_lowercase();
    match normalized.as_str() {
        "allow_once" => Ok(ApprovalDecision::AllowOnce),
        "always_allow" | "allow_always" => Ok(ApprovalDecision::AlwaysAllow),
        "deny" => Ok(ApprovalDecision::Deny),
        _ => Err(AcpValidationError::InvalidApprovalDecision {
            decision: value.to_string(),
        }),
    }
}

fn validate_op(op: &Op) -> Result<()> {
    match op {
        Op::AppendUserMessage { message } => validate_display_text("message", message),
        Op::RunQuery { sql, .. } => validate_read_only_sql(sql),
        Op::IngestBatch {
            provider,
            endpoint,
            symbols,
            range,
        } => {
            validate_token("provider", provider)?;
            validate_token("endpoint", endpoint)?;
            for symbol in symbols {
                validate_display_text("symbol", symbol)?;
            }
            if let Some(range) = range {
                validate_display_text("range.start", &range.start)?;
                validate_display_text("range.end", &range.end)?;
            }
            Ok(())
        }
        Op::ToolCall { tool_name, .. } => validate_token("tool_name", tool_name),
        Op::ApprovalResponse { reason, .. } => {
            if let Some(reason) = reason {
                validate_display_text("reason", reason)?;
            }
            Ok(())
        }
        Op::CompactContext { target_tokens } => {
            if *target_tokens == 0 {
                Err(AcpValidationError::InvalidQuery {
                    reason: "target token budget must be positive",
                })
            } else {
                Ok(())
            }
        }
        Op::StreamStart {
            provider,
            symbol,
            table,
        } => {
            validate_token("provider", provider)?;
            validate_token("symbol", symbol)?;
            if let Some(table) = table {
                validate_token("table", table)?;
            }
            Ok(())
        }
        Op::StreamStop { stream_id } => validate_token("stream_id", stream_id),
        Op::GetQuoteSnapshot { provider, symbol } => {
            validate_token("provider", provider)?;
            validate_token("symbol", symbol)
        }
        Op::CreateAlert {
            symbol, condition, ..
        } => {
            validate_token("symbol", symbol)?;
            validate_token("condition", condition)
        }
        Op::ListAlerts {} => Ok(()),
        Op::DeleteAlert { id } => validate_token("id", id),
        Op::SetAlertActive { id, .. } => validate_token("id", id),
        Op::RegisterUser {
            id,
            email,
            display_name,
            ..
        } => {
            // `password` is deliberately not validated here: it is opaque
            // credential material (may legitimately contain spaces and the
            // punctuation `validate_token` rejects), and the identity store
            // enforces its own length policy. `id` is a structural token;
            // `email`/`display_name` are free-form display text.
            validate_token("id", id)?;
            validate_display_text("email", email)?;
            validate_display_text("display_name", display_name)
        }
        Op::Cancel { .. } | Op::Shutdown => Ok(()),
    }
}

fn validate_token(field: &'static str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(AcpValidationError::EmptyField { field });
    }
    if value
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '/' | '\\' | ';' | '|' | '&'))
        || value.contains("..")
    {
        return Err(AcpValidationError::UnsafeField { field });
    }
    Ok(())
}

fn validate_display_text(field: &'static str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(AcpValidationError::EmptyField { field });
    }
    if value.chars().any(char::is_control) {
        return Err(AcpValidationError::UnsafeField { field });
    }
    Ok(())
}

fn validate_read_only_sql(sql: &str) -> Result<()> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err(AcpValidationError::EmptyField { field: "sql" });
    }
    if trimmed.chars().any(char::is_control) {
        return Err(AcpValidationError::UnsafeField { field: "sql" });
    }
    let semicolon_count = trimmed.matches(';').count();
    if semicolon_count > 1 || (semicolon_count == 1 && !trimmed.ends_with(';')) {
        return Err(AcpValidationError::InvalidQuery {
            reason: "multiple statements are not allowed",
        });
    }
    let without_trailing_semicolon = trimmed.strip_suffix(';').unwrap_or(trimmed).trim_end();
    let lower = without_trailing_semicolon.to_ascii_lowercase();
    if !lower.starts_with("select ") && lower != "select" {
        return Err(AcpValidationError::InvalidQuery {
            reason: "only read-only select statements are supported",
        });
    }
    if [
        "--", "/*", "*/", " drop ", " delete ", " insert ", " update ",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return Err(AcpValidationError::InvalidQuery {
            reason: "unsafe SQL tokens are not allowed",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_protocol::{ActorKind, ActorRef, OpEnvelope};

    #[test]
    fn acp_request_serializes_protocol_op() {
        let request = AcpRequest::SubmitOp {
            session_id: SessionId::new("session-1").expect("session id"),
            op: Op::AppendUserMessage {
                message: "hello".to_string(),
            },
        };

        let encoded = serde_json::to_value(&request).expect("request serializes");
        assert_eq!(encoded["type"], "submit_op");
        assert_eq!(encoded["op"]["type"], "append_user_message");
    }

    #[test]
    fn acp_response_can_wrap_event_msg() {
        let session_id = SessionId::new("session-1").expect("session id");
        let envelope = OpEnvelope::new(
            session_id.clone(),
            1,
            ActorRef {
                actor_id: "client".to_string(),
                kind: ActorKind::User,
                tenant_id: None,
            },
            Op::Shutdown,
        );
        let response = AcpResponse::Event {
            session_id,
            event: EventMsg::Started {
                op_id: envelope.op_id,
            },
        };

        let encoded = serde_json::to_value(&response).expect("response serializes");
        assert_eq!(encoded["type"], "event");
        assert_eq!(encoded["event"]["type"], "started");
    }

    #[test]
    fn validates_client_boundary_requests() {
        let request = AcpRequest::Initialize {
            client_name: "tdw-cli".to_string(),
        };
        assert!(validate_request(&request).is_ok());

        let rejected = AcpRequest::ResolveApproval {
            permission_id: "../approval".to_string(),
            decision: "allow_once".to_string(),
        };
        assert_eq!(
            validate_request(&rejected),
            Err(AcpValidationError::UnsafeField {
                field: "permission_id"
            })
        );
    }

    #[test]
    fn rejects_unsafe_submit_op_queries() {
        let request = AcpRequest::SubmitOp {
            session_id: SessionId::new("session-1").expect("session id"),
            op: Op::RunQuery {
                sql: "select 1; drop table raw.orders".to_string(),
                plan_id: None,
                cost_hint: None,
            },
        };

        assert_eq!(
            validate_request(&request),
            Err(AcpValidationError::InvalidQuery {
                reason: "multiple statements are not allowed"
            })
        );
    }

    #[test]
    fn read_only_sql_gate_accepts_selects_and_rejects_writes_and_comments() {
        let q = |sql: &str| AcpRequest::SubmitOp {
            session_id: SessionId::new("session-1").expect("session id"),
            op: Op::RunQuery {
                sql: sql.to_string(),
                plan_id: None,
                cost_hint: None,
            },
        };

        // A plain select and a single trailing-semicolon select are allowed.
        assert!(validate_request(&q("select 1")).is_ok());
        assert!(validate_request(&q("select * from raw.orders;")).is_ok());

        // A non-select statement is rejected by the read-only prefix check.
        assert_eq!(
            validate_request(&q("update raw.orders set filled = 1")),
            Err(AcpValidationError::InvalidQuery {
                reason: "only read-only select statements are supported"
            })
        );
        // An inline comment token is rejected as an unsafe SQL token.
        assert_eq!(
            validate_request(&q("select 1 -- sneaky")),
            Err(AcpValidationError::InvalidQuery {
                reason: "unsafe SQL tokens are not allowed"
            })
        );
        // Empty SQL is an empty field.
        assert_eq!(
            validate_request(&q("   ")),
            Err(AcpValidationError::EmptyField { field: "sql" })
        );
    }
}
