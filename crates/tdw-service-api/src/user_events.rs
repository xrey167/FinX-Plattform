//! Domain event payloads emitted by the identity (user-registration) op.
//!
//! The dispatcher emits a [`UserCreatedPayload`] under the
//! [`USER_CREATED_EVENT_TYPE`] event type onto the daemon outbox after a
//! successful [`Op::RegisterUser`](tdw_protocol::Op::RegisterUser), using the
//! same `tdw-event` / outbox relay path the rest of the service uses.
//!
//! # Security
//!
//! The payload deliberately carries only the **non-secret** projection of the
//! created user: id, normalized email, and creation timestamp. It never
//! includes the password or password hash.

use serde::{Deserialize, Serialize};

/// Event type string for the user-registration domain event.
pub const USER_CREATED_EVENT_TYPE: &str = "user.created";

/// Payload for the [`USER_CREATED_EVENT_TYPE`] event.
///
/// Emitted once per successful user registration. Contains only non-secret
/// fields — never the password or password hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserCreatedPayload {
    /// Stable identifier of the newly created user.
    pub user_id: String,
    /// Normalized (trimmed, lowercased) email address of the new user.
    pub email: String,
    /// Unix-epoch millisecond timestamp when the user record was created.
    pub created_at_ms: i64,
}
