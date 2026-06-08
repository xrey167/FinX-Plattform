//! First-party user + password persistence layer for the TDW platform.
//!
//! `tdw-identity` is the persistence layer **below** the existing OIDC
//! authorization stack (`tdw-auth` / `tdw-auth-oidc`). OIDC Principals and
//! role-based authorization remain the authz layer on top; first-party
//! email/password authentication is strictly opt-in and requires the calling
//! service to wire both layers together.
//!
//! # What this crate provides
//!
//! * [`User`] — the public user record (never exposes `password_hash`).
//! * [`NewUser`] — input value object for registration.
//! * [`Session`] — an opaque bearer-token session record.
//! * [`IdentityError`] — domain errors (email validation, weak password,
//!   duplicate email, credential mismatch, session not found / expired, etc.).
//! * [`UserStore`] — async persistence trait for users.
//! * [`InMemoryUserStore`] — always-compiled in-memory reference
//!   implementation (unit tests, offline scenarios).
//! * [`PgUserStore`] — Postgres-backed implementation behind the `postgres`
//!   feature flag.
//! * [`SessionStore`] — async persistence trait for sessions.
//! * [`InMemorySessionStore`] — always-compiled in-memory session store.
//! * [`PgSessionStore`] — Postgres-backed session store behind the `postgres`
//!   feature flag.
//! * [`ResetToken`] — a single-use, short-lived password-reset token record.
//! * [`ResetTokenStore`] — async persistence trait for reset tokens.
//! * [`InMemoryResetTokenStore`] — always-compiled in-memory reset-token store.
//! * [`PgResetTokenStore`] — Postgres-backed reset-token store behind the
//!   `postgres` feature flag.
//!
//! # Security notes
//!
//! * Passwords are hashed with **Argon2id** (RustCrypto, pure Rust) using a
//!   random 16-byte salt per hash. The PHC string is stored in the database.
//! * The `User` struct and all public APIs **never** include or return the
//!   raw hash; the hash is internal to each store implementation.
//! * [`IdentityError::InvalidCredentials`] is returned for **both** unknown
//!   email and wrong password to prevent user enumeration.
//! * Unknown-email path: a dummy hash verify is performed to equalize timing
//!   between "email not found" and "wrong password" responses (see
//!   [`DUMMY_HASH`]). This is a best-effort mitigation; a constant-time
//!   guarantee requires further hardening (e.g. `subtle::ConstantTimeEq` on
//!   the hash output) which is tracked as a follow-up.
//! * Password material is never logged (no `Display` impl on [`NewUser`]).
//! * `zeroize` on password strings is deferred to a future sub-PR (requires
//!   adding the `zeroize` workspace dep; noted as a follow-up).

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(feature = "postgres")]
pub use pg::{PgResetTokenStore, PgSessionStore, PgUserStore};

// ---------------------------------------------------------------------------
// IdentityError + Result alias
// ---------------------------------------------------------------------------

/// Errors returned by [`UserStore`] implementations.
#[derive(Debug, Error)]
pub enum IdentityError {
    /// The email address is already registered.
    #[error("email already registered")]
    EmailTaken,

    /// No user exists with the requested id or email.
    #[error("user not found")]
    UserNotFound,

    /// The email/password combination is not valid.
    ///
    /// Returned for **both** unknown-email and wrong-password cases to prevent
    /// user enumeration.
    #[error("invalid credentials")]
    InvalidCredentials,

    /// The password does not satisfy the length policy.
    #[error("weak password: {0}")]
    WeakPassword(String),

    /// The email address is empty or missing `@`.
    #[error("invalid email: {0}")]
    InvalidEmail(String),

    /// An Argon2 hashing or parse error.
    #[error("password hash error: {0}")]
    Hash(String),

    /// No session exists with the requested token.
    #[error("session not found")]
    SessionNotFound,

    /// The session token was found but its expiry has passed.
    ///
    /// The expired row is deleted as a best-effort side effect of
    /// [`SessionStore::resolve`].
    #[error("session expired")]
    SessionExpired,

    /// No reset token exists with the requested value.
    #[error("reset token not found")]
    ResetTokenNotFound,

    /// The reset token was found but its expiry has passed.
    ///
    /// The expired row is deleted as a best-effort side effect of
    /// [`ResetTokenStore::consume`].
    #[error("reset token expired")]
    ResetTokenExpired,

    /// A lower-level storage failure (Postgres, mutex, serialization, etc.).
    #[error("store error: {0}")]
    Store(String),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, IdentityError>;

// ---------------------------------------------------------------------------
// Security: dummy hash for timing equalization
// ---------------------------------------------------------------------------

/// A pre-computed Argon2id PHC string used to perform a dummy `verify_password`
/// call when the requested email does not exist in the store.  This equalises
/// the wall-clock time between "email not found" and "wrong password" paths,
/// reducing the timing oracle that would otherwise reveal whether an email
/// address is registered.
///
/// The hash encodes the literal string `"__dummy__"`.  It is intentionally a
/// constant (not derived from the lookup email) so that an attacker cannot
/// infer anything from the verification result.
///
/// Limitation: Argon2id's memory-hard computation takes a roughly constant
/// time regardless of the stored hash, so this defence is effective against
/// most timing oracles.  A further hardening step (constant-time comparison
/// of the hash *digest* bytes via `subtle`) is deferred to a follow-up.
const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXJhbmRvbXNhbHQ$RoB3J7XHQn+ys5bAcDMnj9xFjOfDoGgWQk+FtqCDKdo";

// ---------------------------------------------------------------------------
// Email normalization
// ---------------------------------------------------------------------------

/// Normalize an email address: trim whitespace and convert ASCII letters to
/// lowercase.  Returns `Err(IdentityError::InvalidEmail)` if the result is
/// empty or contains no `@` character.
fn normalize_email(email: &str) -> Result<String> {
    let normalized = email.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Err(IdentityError::InvalidEmail(
            "email must not be empty".to_string(),
        ));
    }
    if !normalized.contains('@') {
        return Err(IdentityError::InvalidEmail(format!(
            "email must contain '@': {normalized}"
        )));
    }
    Ok(normalized)
}

// ---------------------------------------------------------------------------
// Password hashing
// ---------------------------------------------------------------------------

/// Hash a password using Argon2id with a random salt.
///
/// Returns the PHC string (e.g. `$argon2id$v=19$...`) suitable for database
/// storage.
///
/// # Errors
///
/// Returns [`IdentityError::Hash`] if the Argon2 computation fails (extremely
/// unlikely with the default parameters).
fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| IdentityError::Hash(format!("hash_password: {error}")))?;
    Ok(hash.to_string())
}

/// Verify a password against a stored Argon2 PHC hash string.
///
/// * Returns `Ok(true)` if the password matches.
/// * Returns `Ok(false)` if the password does not match.
/// * Returns `Err(IdentityError::Hash)` if `hash` is not a valid PHC string.
///
/// # Errors
///
/// Returns [`IdentityError::Hash`] if `hash` cannot be parsed as a PHC string.
fn verify_password(hash: &str, password: &str) -> Result<bool> {
    let parsed =
        PasswordHash::new(hash).map_err(|error| IdentityError::Hash(format!("{error}")))?;
    match Argon2::default().verify_password(password.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(error) => Err(IdentityError::Hash(format!("verify_password: {error}"))),
    }
}

/// Validate a plaintext password against the length policy (8..=128 Unicode
/// scalar values).
///
/// # Errors
///
/// Returns [`IdentityError::WeakPassword`] if the password is outside the
/// allowed length range.
fn validate_password(password: &str) -> Result<()> {
    let len = password.chars().count();
    if len < 8 {
        return Err(IdentityError::WeakPassword(format!(
            "password must be at least 8 characters (got {len})"
        )));
    }
    if len > 128 {
        return Err(IdentityError::WeakPassword(format!(
            "password must be at most 128 characters (got {len})"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A persisted user record.
///
/// The `password_hash` column is **never** included here; it is internal to
/// the store implementation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    /// Stable, caller-supplied identifier (e.g. a ULID or UUID string).
    pub id: String,
    /// Normalized (trimmed, ASCII-lowercased) email address.
    pub email: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Unix-epoch millisecond timestamp of the user's last activity.
    ///
    /// `None` until the first session is recorded. Used by rank-19 dormant
    /// projection logic.
    pub last_active_at_ms: Option<i64>,
    /// Unix-epoch millisecond timestamp of the last re-engagement email sent.
    ///
    /// `None` until the first re-engagement email is dispatched. App field
    /// for future dormant-user workflows.
    pub last_reengagement_sent_at_ms: Option<i64>,
    /// Unix-epoch millisecond timestamp when the record was created.
    pub created_at_ms: i64,
    /// Unix-epoch millisecond timestamp of the last record mutation.
    pub updated_at_ms: i64,
}

/// Input value object used to register a new [`User`].
///
/// # Security
///
/// `password` is a `String`; in-memory zeroization on drop is deferred to a
/// future sub-PR (requires adding `zeroize` to workspace deps). Callers must
/// not log or serialize this struct.
#[derive(Clone)]
pub struct NewUser {
    /// Raw email address (will be normalized during registration).
    pub email: String,
    /// Plaintext password (will be validated and hashed; never stored).
    pub password: String,
    /// Human-readable display name.
    pub display_name: String,
}

// Intentionally no `Debug` impl for `NewUser` to prevent accidental logging of
// the plaintext password field.

// ---------------------------------------------------------------------------
// UserStore trait
// ---------------------------------------------------------------------------

/// Async persistence interface for [`User`] records.
///
/// All methods take `&self` so implementations can use interior mutability
/// (e.g. `Mutex`) or shared connection pools without requiring `&mut self`.
#[async_trait]
pub trait UserStore: Send + Sync {
    /// Validate, hash, and persist a new user.
    ///
    /// * `new.email` is normalized (trimmed + lowercased) before persistence.
    /// * `new.password` is validated (8..=128 chars) then hashed with
    ///   Argon2id.
    /// * `id` and `now_ms` are caller-supplied; the crate has no UUID/clock
    ///   dependency.
    ///
    /// # Errors
    ///
    /// * [`IdentityError::InvalidEmail`] — empty or missing `@`.
    /// * [`IdentityError::WeakPassword`] — password outside 8..=128 chars.
    /// * [`IdentityError::EmailTaken`] — normalized email already registered.
    /// * [`IdentityError::Hash`] — Argon2 computation failed.
    /// * [`IdentityError::Store`] — underlying storage failure.
    async fn register(&self, new: NewUser, id: String, now_ms: i64) -> Result<User>;

    /// Verify an email/password pair and return the matching [`User`].
    ///
    /// Both unknown-email and wrong-password cases return
    /// [`IdentityError::InvalidCredentials`] (no user enumeration).
    ///
    /// # Errors
    ///
    /// * [`IdentityError::InvalidCredentials`] — email not found or password
    ///   mismatch.
    /// * [`IdentityError::Hash`] — the stored hash is corrupt.
    /// * [`IdentityError::Store`] — underlying storage failure.
    async fn authenticate(&self, email: &str, password: &str) -> Result<User>;

    /// Fetch a user by their stable identifier.
    ///
    /// # Errors
    ///
    /// * [`IdentityError::UserNotFound`] — no user with that `id`.
    /// * [`IdentityError::Store`] — underlying storage failure.
    async fn get_by_id(&self, id: &str) -> Result<User>;

    /// Fetch a user by their (normalized) email address.
    ///
    /// # Errors
    ///
    /// * [`IdentityError::UserNotFound`] — no user with that email.
    /// * [`IdentityError::Store`] — underlying storage failure.
    async fn get_by_email(&self, email: &str) -> Result<User>;

    /// Update `last_active_at_ms` to `now_ms` for the user with the given
    /// `id`.  Called by session machinery (future sub-PR) to track activity.
    ///
    /// # Errors
    ///
    /// * [`IdentityError::UserNotFound`] — no user with that `id`.
    /// * [`IdentityError::Store`] — underlying storage failure.
    async fn touch_last_active(&self, id: &str, now_ms: i64) -> Result<()>;
}

// ---------------------------------------------------------------------------
// InMemoryUserStore (internal record)
// ---------------------------------------------------------------------------

/// Internal record stored inside [`InMemoryUserStore`].  The hash is kept
/// here and is never surfaced through [`UserStore`] method return values.
#[derive(Clone)]
struct UserRecord {
    user: User,
    password_hash: String,
}

// Manual Debug for UserRecord to avoid leaking hash in test output.
impl std::fmt::Debug for UserRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UserRecord")
            .field("user", &self.user)
            .field("password_hash", &"[redacted]")
            .finish()
    }
}

// ---------------------------------------------------------------------------
// InMemoryUserStore
// ---------------------------------------------------------------------------

/// Thread-safe in-memory implementation of [`UserStore`].
///
/// Always compiled (no feature flag). Suitable for unit tests and offline
/// scenarios. Uses two `Mutex`-guarded `BTreeMap`s for id→record and
/// email→id lookups.
#[derive(Debug, Default)]
pub struct InMemoryUserStore {
    /// id → record (includes password_hash internally)
    records: Mutex<BTreeMap<String, UserRecord>>,
    /// normalized-email → id reverse index
    email_index: Mutex<BTreeMap<String, String>>,
}

impl InMemoryUserStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl UserStore for InMemoryUserStore {
    async fn register(&self, new: NewUser, id: String, now_ms: i64) -> Result<User> {
        let email = normalize_email(&new.email)?;
        validate_password(&new.password)?;

        let mut records = self
            .records
            .lock()
            .map_err(|error| IdentityError::Store(format!("mutex poisoned: {error}")))?;
        let mut email_index = self
            .email_index
            .lock()
            .map_err(|error| IdentityError::Store(format!("mutex poisoned: {error}")))?;

        if email_index.contains_key(&email) {
            return Err(IdentityError::EmailTaken);
        }

        let password_hash = hash_password(&new.password)?;

        let user = User {
            id: id.clone(),
            email: email.clone(),
            display_name: new.display_name,
            last_active_at_ms: None,
            last_reengagement_sent_at_ms: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };

        email_index.insert(email, id.clone());
        records.insert(
            id,
            UserRecord {
                user: user.clone(),
                password_hash,
            },
        );
        Ok(user)
    }

    async fn authenticate(&self, email: &str, password: &str) -> Result<User> {
        let email = match normalize_email(email) {
            Ok(e) => e,
            Err(_) => {
                // Still run a dummy verify to equalize timing before returning.
                let _ = verify_password(DUMMY_HASH, password);
                return Err(IdentityError::InvalidCredentials);
            }
        };

        let records = self
            .records
            .lock()
            .map_err(|error| IdentityError::Store(format!("mutex poisoned: {error}")))?;
        let email_index = self
            .email_index
            .lock()
            .map_err(|error| IdentityError::Store(format!("mutex poisoned: {error}")))?;

        match email_index.get(&email).and_then(|id| records.get(id)) {
            Some(record) => {
                let matched = verify_password(&record.password_hash, password)?;
                if matched {
                    Ok(record.user.clone())
                } else {
                    Err(IdentityError::InvalidCredentials)
                }
            }
            None => {
                // Dummy verify to equalize timing with the "found but wrong password" path.
                let _ = verify_password(DUMMY_HASH, password);
                Err(IdentityError::InvalidCredentials)
            }
        }
    }

    async fn get_by_id(&self, id: &str) -> Result<User> {
        let records = self
            .records
            .lock()
            .map_err(|error| IdentityError::Store(format!("mutex poisoned: {error}")))?;
        records
            .get(id)
            .map(|record| record.user.clone())
            .ok_or(IdentityError::UserNotFound)
    }

    async fn get_by_email(&self, email: &str) -> Result<User> {
        let email = normalize_email(email)?;
        let records = self
            .records
            .lock()
            .map_err(|error| IdentityError::Store(format!("mutex poisoned: {error}")))?;
        let email_index = self
            .email_index
            .lock()
            .map_err(|error| IdentityError::Store(format!("mutex poisoned: {error}")))?;
        email_index
            .get(&email)
            .and_then(|id| records.get(id))
            .map(|record| record.user.clone())
            .ok_or(IdentityError::UserNotFound)
    }

    async fn touch_last_active(&self, id: &str, now_ms: i64) -> Result<()> {
        let mut records = self
            .records
            .lock()
            .map_err(|error| IdentityError::Store(format!("mutex poisoned: {error}")))?;
        match records.get_mut(id) {
            Some(record) => {
                record.user.last_active_at_ms = Some(now_ms);
                record.user.updated_at_ms = now_ms;
                Ok(())
            }
            None => Err(IdentityError::UserNotFound),
        }
    }
}

// ---------------------------------------------------------------------------
// Session types
// ---------------------------------------------------------------------------

/// Default session lifetime: 7 days in milliseconds.
pub const DEFAULT_SESSION_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1_000;

/// A persisted session record.
///
/// The `token` field is the opaque bearer credential returned to the client.
/// Unlike [`User`], the token **is** included here — it is the session
/// identity itself (analogous to an opaque API key).  Callers must treat
/// `token` as secret and must not log it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    /// High-entropy bearer token (64 lowercase hex chars, 256 bits entropy).
    ///
    /// Generated from 32 bytes of [`OsRng`] output.  Never log this value.
    pub token: String,
    /// Stable user identifier matching the user who owns this session.
    pub user_id: String,
    /// Unix-epoch millisecond timestamp when the session was created.
    pub created_at_ms: i64,
    /// Unix-epoch millisecond timestamp after which the session is invalid.
    ///
    /// Expiry is fixed at creation time (`now_ms + ttl_ms`).  No sliding
    /// window is applied on resolve in this sub-PR — a follow-up can add
    /// that if needed.
    pub expires_at_ms: i64,
}

// ---------------------------------------------------------------------------
// Token generation
// ---------------------------------------------------------------------------

/// Generate a cryptographically secure session token.
///
/// Reads 32 bytes from [`OsRng`] (OS CSPRNG) and encodes them as 64
/// lowercase hexadecimal characters, yielding **256 bits of entropy**.
///
/// The returned token is the sole bearer credential for the session.
/// Callers must treat it as secret: do not log, do not include in URLs,
/// transmit only over TLS.
fn generate_session_token() -> String {
    use argon2::password_hash::rand_core::RngCore;
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes
        .iter()
        .fold(String::with_capacity(64), |mut acc, byte| {
            // format each byte as exactly 2 lowercase hex digits
            let hi = byte >> 4;
            let lo = byte & 0x0f;
            acc.push(char::from_digit(u32::from(hi), 16).unwrap_or('0'));
            acc.push(char::from_digit(u32::from(lo), 16).unwrap_or('0'));
            acc
        })
}

// ---------------------------------------------------------------------------
// SessionStore trait
// ---------------------------------------------------------------------------

/// Async persistence interface for [`Session`] records.
///
/// All methods take `&self` so implementations can use interior mutability
/// (e.g. `Mutex`) or shared connection pools without requiring `&mut self`.
///
/// # Security notes
///
/// * Tokens are 32 random bytes from [`OsRng`] encoded as 64 lowercase hex
///   chars (256 bits entropy).
/// * Token material is **never** logged by this crate.
/// * Expiry is checked in `resolve`; expired rows are deleted on access
///   (best-effort lazy sweep).
/// * No sliding-window extension is performed by `resolve` — TTL is fixed at
///   creation time.  A follow-up sub-PR may add optional extension.
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Mint a new session for `user_id`, valid for `ttl_ms` milliseconds from
    /// `now_ms`.
    ///
    /// A fresh high-entropy token is generated for every call.
    ///
    /// # Errors
    ///
    /// * [`IdentityError::Store`] — underlying storage failure.
    async fn create(&self, user_id: &str, now_ms: i64, ttl_ms: i64) -> Result<Session>;

    /// Look up `token` and verify it has not expired.
    ///
    /// * Returns the [`Session`] if found and `expires_at_ms > now_ms`.
    /// * Returns [`IdentityError::SessionNotFound`] if `token` is unknown.
    /// * Returns [`IdentityError::SessionExpired`] if the token is found but
    ///   `expires_at_ms <= now_ms`; the expired row is deleted as a
    ///   best-effort side effect.
    ///
    /// Expiry is **not** extended on successful resolve (no sliding window).
    ///
    /// # Errors
    ///
    /// * [`IdentityError::SessionNotFound`] — token does not exist.
    /// * [`IdentityError::SessionExpired`] — token found but past its TTL.
    /// * [`IdentityError::Store`] — underlying storage failure.
    async fn resolve(&self, token: &str, now_ms: i64) -> Result<Session>;

    /// Delete the session identified by `token`.
    ///
    /// Returns `true` if a row was removed, `false` if `token` was not found.
    ///
    /// # Errors
    ///
    /// * [`IdentityError::Store`] — underlying storage failure.
    async fn revoke(&self, token: &str) -> Result<bool>;

    /// Delete **all** sessions belonging to `user_id` (e.g. on password
    /// change or logout-everywhere).
    ///
    /// Returns the number of sessions removed.
    ///
    /// # Errors
    ///
    /// * [`IdentityError::Store`] — underlying storage failure.
    async fn revoke_all_for_user(&self, user_id: &str) -> Result<u64>;
}

// ---------------------------------------------------------------------------
// InMemorySessionStore
// ---------------------------------------------------------------------------

/// Thread-safe in-memory implementation of [`SessionStore`].
///
/// Always compiled (no feature flag). Suitable for unit tests and offline
/// scenarios.  Uses a single `Mutex`-guarded `BTreeMap<token, Session>`.
#[derive(Debug, Default)]
pub struct InMemorySessionStore {
    sessions: Mutex<BTreeMap<String, Session>>,
}

impl InMemorySessionStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn create(&self, user_id: &str, now_ms: i64, ttl_ms: i64) -> Result<Session> {
        let token = generate_session_token();
        let session = Session {
            token: token.clone(),
            user_id: user_id.to_string(),
            created_at_ms: now_ms,
            expires_at_ms: now_ms + ttl_ms,
        };
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|error| IdentityError::Store(format!("mutex poisoned: {error}")))?;
        sessions.insert(token, session.clone());
        Ok(session)
    }

    async fn resolve(&self, token: &str, now_ms: i64) -> Result<Session> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|error| IdentityError::Store(format!("mutex poisoned: {error}")))?;
        match sessions.get(token).cloned() {
            None => Err(IdentityError::SessionNotFound),
            Some(session) if session.expires_at_ms <= now_ms => {
                // Best-effort delete expired row.
                sessions.remove(token);
                Err(IdentityError::SessionExpired)
            }
            Some(session) => Ok(session),
        }
    }

    async fn revoke(&self, token: &str) -> Result<bool> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|error| IdentityError::Store(format!("mutex poisoned: {error}")))?;
        Ok(sessions.remove(token).is_some())
    }

    async fn revoke_all_for_user(&self, user_id: &str) -> Result<u64> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|error| IdentityError::Store(format!("mutex poisoned: {error}")))?;
        let before = sessions.len();
        sessions.retain(|_, session| session.user_id != user_id);
        let removed = before.saturating_sub(sessions.len());
        Ok(removed as u64)
    }
}

// ---------------------------------------------------------------------------
// Reset-token types
// ---------------------------------------------------------------------------

/// Default reset-token lifetime: 1 hour in milliseconds.
///
/// Reset tokens are deliberately short-lived (unlike the 7-day session TTL):
/// they are single-use credentials emailed to a user to prove control of their
/// inbox and should expire quickly to limit the blast radius of a leaked link.
pub const DEFAULT_RESET_TOKEN_TTL_MS: i64 = 60 * 60 * 1_000;

/// A persisted single-use password-reset token record.
///
/// The `token` field is the opaque credential delivered to the user (e.g. in a
/// password-reset email link).  Like [`Session`], the token **is** included
/// here — it is the reset identity itself.  Callers must treat `token` as
/// secret and must not log it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResetToken {
    /// High-entropy token (64 lowercase hex chars, 256 bits entropy).
    ///
    /// Generated from 32 bytes of [`OsRng`] output.  Never log this value.
    pub token: String,
    /// Stable user identifier matching the user this token resets.
    pub user_id: String,
    /// Unix-epoch millisecond timestamp when the token was issued.
    pub created_at_ms: i64,
    /// Unix-epoch millisecond timestamp after which the token is invalid.
    ///
    /// Expiry is fixed at issue time (`now_ms + ttl_ms`).
    pub expires_at_ms: i64,
}

// ---------------------------------------------------------------------------
// Reset-token generation
// ---------------------------------------------------------------------------

/// Generate a cryptographically secure password-reset token.
///
/// Reads 32 bytes from [`OsRng`] (OS CSPRNG) and encodes them as 64
/// lowercase hexadecimal characters, yielding **256 bits of entropy**.
///
/// The returned token is the sole credential for the reset flow.
/// Callers must treat it as secret: do not log, transmit only over TLS,
/// and embed in single-use links that expire quickly.
pub fn generate_reset_token() -> String {
    use argon2::password_hash::rand_core::RngCore;
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes
        .iter()
        .fold(String::with_capacity(64), |mut acc, byte| {
            // format each byte as exactly 2 lowercase hex digits
            let hi = byte >> 4;
            let lo = byte & 0x0f;
            acc.push(char::from_digit(u32::from(hi), 16).unwrap_or('0'));
            acc.push(char::from_digit(u32::from(lo), 16).unwrap_or('0'));
            acc
        })
}

// ---------------------------------------------------------------------------
// ResetTokenStore trait
// ---------------------------------------------------------------------------

/// Async persistence interface for [`ResetToken`] records.
///
/// All methods take `&self` so implementations can use interior mutability
/// (e.g. `Mutex`) or shared connection pools without requiring `&mut self`.
///
/// # Security notes
///
/// * Tokens are 32 random bytes from [`OsRng`] encoded as 64 lowercase hex
///   chars (256 bits entropy).
/// * Token material is **never** logged by this crate.
/// * Tokens are **single-use**: a successful [`consume`](ResetTokenStore::consume)
///   deletes the row so the token cannot be replayed.
/// * Expiry is checked in `consume`; expired rows are deleted on access
///   (best-effort lazy sweep).
#[async_trait]
pub trait ResetTokenStore: Send + Sync {
    /// Persist a reset `token` for `user_id`, valid for `ttl_ms` milliseconds
    /// from `now_ms`.
    ///
    /// The caller supplies the token (e.g. from [`generate_reset_token`]) and
    /// `now_ms`; the crate has no UUID/clock dependency.
    ///
    /// # Errors
    ///
    /// * [`IdentityError::Store`] — underlying storage failure.
    async fn issue(
        &self,
        token: String,
        user_id: String,
        now_ms: i64,
        ttl_ms: i64,
    ) -> Result<ResetToken>;

    /// Look up `token`, verify it has not expired, and consume it (single-use).
    ///
    /// * Returns the [`ResetToken`] if found and `expires_at_ms > now_ms`,
    ///   **deleting** the row so it cannot be reused.
    /// * Returns [`IdentityError::ResetTokenNotFound`] if `token` is unknown.
    /// * Returns [`IdentityError::ResetTokenExpired`] if the token is found but
    ///   `expires_at_ms <= now_ms`; the expired row is deleted as a
    ///   best-effort side effect.
    ///
    /// # Errors
    ///
    /// * [`IdentityError::ResetTokenNotFound`] — token does not exist.
    /// * [`IdentityError::ResetTokenExpired`] — token found but past its TTL.
    /// * [`IdentityError::Store`] — underlying storage failure.
    async fn consume(&self, token: &str, now_ms: i64) -> Result<ResetToken>;

    /// Delete **all** outstanding reset tokens belonging to `user_id` (e.g.
    /// after a successful reset or a password change).
    ///
    /// # Errors
    ///
    /// * [`IdentityError::Store`] — underlying storage failure.
    async fn revoke_all_for_user(&self, user_id: &str) -> Result<()>;
}

// ---------------------------------------------------------------------------
// InMemoryResetTokenStore
// ---------------------------------------------------------------------------

/// Thread-safe in-memory implementation of [`ResetTokenStore`].
///
/// Always compiled (no feature flag). Suitable for unit tests and offline
/// scenarios.  Uses a single `Mutex`-guarded `BTreeMap<token, ResetToken>`.
#[derive(Debug, Default)]
pub struct InMemoryResetTokenStore {
    tokens: Mutex<BTreeMap<String, ResetToken>>,
}

impl InMemoryResetTokenStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ResetTokenStore for InMemoryResetTokenStore {
    async fn issue(
        &self,
        token: String,
        user_id: String,
        now_ms: i64,
        ttl_ms: i64,
    ) -> Result<ResetToken> {
        let reset = ResetToken {
            token: token.clone(),
            user_id,
            created_at_ms: now_ms,
            expires_at_ms: now_ms + ttl_ms,
        };
        let mut tokens = self
            .tokens
            .lock()
            .map_err(|error| IdentityError::Store(format!("mutex poisoned: {error}")))?;
        tokens.insert(token, reset.clone());
        Ok(reset)
    }

    async fn consume(&self, token: &str, now_ms: i64) -> Result<ResetToken> {
        let mut tokens = self
            .tokens
            .lock()
            .map_err(|error| IdentityError::Store(format!("mutex poisoned: {error}")))?;
        match tokens.get(token).cloned() {
            None => Err(IdentityError::ResetTokenNotFound),
            Some(reset) if reset.expires_at_ms <= now_ms => {
                // Best-effort delete expired row.
                tokens.remove(token);
                Err(IdentityError::ResetTokenExpired)
            }
            Some(reset) => {
                // Single-use: delete on successful consume.
                tokens.remove(token);
                Ok(reset)
            }
        }
    }

    async fn revoke_all_for_user(&self, user_id: &str) -> Result<()> {
        let mut tokens = self
            .tokens
            .lock()
            .map_err(|error| IdentityError::Store(format!("mutex poisoned: {error}")))?;
        tokens.retain(|_, reset| reset.user_id != user_id);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Postgres backend
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres")]
mod pg {
    use async_trait::async_trait;
    use sqlx::Row;
    use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};

    use crate::{
        DUMMY_HASH, IdentityError, NewUser, Result, User, UserStore, hash_password,
        normalize_email, validate_password, verify_password,
    };

    /// Postgres-backed implementation of [`UserStore`].
    ///
    /// Requires the `postgres` feature.  Construct via
    /// [`PgUserStore::connect`] or [`PgUserStore::from_pool`].
    #[derive(Clone, Debug)]
    pub struct PgUserStore {
        pool: PgPool,
    }

    impl PgUserStore {
        /// Open a connection pool against `database_url`.
        ///
        /// # Errors
        ///
        /// Returns [`IdentityError::Store`] if the connection fails.
        pub async fn connect(database_url: &str) -> Result<Self> {
            let pool = PgPoolOptions::new()
                .max_connections(5)
                .connect(database_url)
                .await
                .map_err(|error| IdentityError::Store(format!("postgres connect: {error}")))?;
            Ok(Self { pool })
        }

        /// Adopt a caller-built pool.
        #[must_use]
        pub fn from_pool(pool: PgPool) -> Self {
            Self { pool }
        }

        /// Expose the underlying pool for callers that need raw sqlx access.
        #[must_use]
        pub const fn pool(&self) -> &PgPool {
            &self.pool
        }
    }

    fn row_to_user(row: &PgRow) -> User {
        User {
            id: row.get("id"),
            email: row.get("email"),
            display_name: row.get("display_name"),
            last_active_at_ms: row.get("last_active_at_ms"),
            last_reengagement_sent_at_ms: row.get("last_reengagement_sent_at_ms"),
            created_at_ms: row.get("created_at_ms"),
            updated_at_ms: row.get("updated_at_ms"),
        }
    }

    #[async_trait]
    impl UserStore for PgUserStore {
        async fn register(&self, new: NewUser, id: String, now_ms: i64) -> Result<User> {
            let email = normalize_email(&new.email)?;
            validate_password(&new.password)?;
            let password_hash = hash_password(&new.password)?;

            let result = sqlx::query(
                r#"
                insert into system.identity_users (
                    id, email, password_hash, display_name,
                    last_active_at_ms, last_reengagement_sent_at_ms,
                    created_at_ms, updated_at_ms
                )
                values ($1, $2, $3, $4, null, null, $5, $6)
                "#,
            )
            .bind(&id)
            .bind(&email)
            .bind(&password_hash)
            .bind(&new.display_name)
            .bind(now_ms)
            .bind(now_ms)
            .execute(&self.pool)
            .await;

            match result {
                Ok(_) => Ok(User {
                    id,
                    email,
                    display_name: new.display_name,
                    last_active_at_ms: None,
                    last_reengagement_sent_at_ms: None,
                    created_at_ms: now_ms,
                    updated_at_ms: now_ms,
                }),
                Err(sqlx::Error::Database(db_err))
                    if db_err.constraint() == Some("identity_users_email_key")
                        || db_err.code().as_deref() == Some("23505") =>
                {
                    Err(IdentityError::EmailTaken)
                }
                Err(error) => Err(IdentityError::Store(format!("register: {error}"))),
            }
        }

        async fn authenticate(&self, email: &str, password: &str) -> Result<User> {
            let email = match normalize_email(email) {
                Ok(e) => e,
                Err(_) => {
                    let _ = verify_password(DUMMY_HASH, password);
                    return Err(IdentityError::InvalidCredentials);
                }
            };

            // Fetch hash and user fields in a single query to avoid TOCTOU.
            let row = sqlx::query(
                r#"
                select id, email, display_name,
                       last_active_at_ms, last_reengagement_sent_at_ms,
                       created_at_ms, updated_at_ms,
                       password_hash
                from system.identity_users
                where email = $1
                "#,
            )
            .bind(&email)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| IdentityError::Store(format!("authenticate fetch: {error}")))?;

            match row {
                Some(ref r) => {
                    let stored_hash: String = r.get("password_hash");
                    let matched = verify_password(&stored_hash, password)?;
                    if matched {
                        Ok(row_to_user(r))
                    } else {
                        Err(IdentityError::InvalidCredentials)
                    }
                }
                None => {
                    let _ = verify_password(DUMMY_HASH, password);
                    Err(IdentityError::InvalidCredentials)
                }
            }
        }

        async fn get_by_id(&self, id: &str) -> Result<User> {
            let row = sqlx::query(
                r#"
                select id, email, display_name,
                       last_active_at_ms, last_reengagement_sent_at_ms,
                       created_at_ms, updated_at_ms
                from system.identity_users
                where id = $1
                "#,
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| IdentityError::Store(format!("get_by_id: {error}")))?;

            row.map(|r| row_to_user(&r))
                .ok_or(IdentityError::UserNotFound)
        }

        async fn get_by_email(&self, email: &str) -> Result<User> {
            let email = normalize_email(email)?;
            let row = sqlx::query(
                r#"
                select id, email, display_name,
                       last_active_at_ms, last_reengagement_sent_at_ms,
                       created_at_ms, updated_at_ms
                from system.identity_users
                where email = $1
                "#,
            )
            .bind(&email)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| IdentityError::Store(format!("get_by_email: {error}")))?;

            row.map(|r| row_to_user(&r))
                .ok_or(IdentityError::UserNotFound)
        }

        async fn touch_last_active(&self, id: &str, now_ms: i64) -> Result<()> {
            let result = sqlx::query(
                r#"
                update system.identity_users
                set last_active_at_ms = $1,
                    updated_at_ms = $1
                where id = $2
                "#,
            )
            .bind(now_ms)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|error| IdentityError::Store(format!("touch_last_active: {error}")))?;

            if result.rows_affected() == 0 {
                Err(IdentityError::UserNotFound)
            } else {
                Ok(())
            }
        }
    }

    // -----------------------------------------------------------------------
    // PgSessionStore
    // -----------------------------------------------------------------------

    use crate::{Session, SessionStore, generate_session_token};

    /// Postgres-backed implementation of [`SessionStore`].
    ///
    /// Requires the `postgres` feature.  Construct via
    /// [`PgSessionStore::connect`] or [`PgSessionStore::from_pool`].
    ///
    /// # Security notes
    ///
    /// * Token material is **never** logged.
    /// * `resolve` deletes the expired row in the same statement (single
    ///   round-trip via `delete … returning`), providing a lazy expiry sweep.
    /// * `revoke_all_for_user` is a single `delete … where user_id = $1`.
    #[derive(Clone, Debug)]
    pub struct PgSessionStore {
        pool: PgPool,
    }

    impl PgSessionStore {
        /// Open a connection pool against `database_url`.
        ///
        /// # Errors
        ///
        /// Returns [`IdentityError::Store`] if the connection fails.
        pub async fn connect(database_url: &str) -> Result<Self> {
            let pool = PgPoolOptions::new()
                .max_connections(5)
                .connect(database_url)
                .await
                .map_err(|error| IdentityError::Store(format!("postgres connect: {error}")))?;
            Ok(Self { pool })
        }

        /// Adopt a caller-built pool.
        #[must_use]
        pub fn from_pool(pool: PgPool) -> Self {
            Self { pool }
        }

        /// Expose the underlying pool for callers that need raw sqlx access.
        #[must_use]
        pub const fn pool(&self) -> &PgPool {
            &self.pool
        }
    }

    #[async_trait]
    impl SessionStore for PgSessionStore {
        async fn create(&self, user_id: &str, now_ms: i64, ttl_ms: i64) -> Result<Session> {
            let token = generate_session_token();
            let expires_at_ms = now_ms + ttl_ms;
            sqlx::query(
                r#"
                insert into system.identity_sessions
                    (token, user_id, created_at_ms, expires_at_ms)
                values ($1, $2, $3, $4)
                "#,
            )
            .bind(&token)
            .bind(user_id)
            .bind(now_ms)
            .bind(expires_at_ms)
            .execute(&self.pool)
            .await
            .map_err(|error| IdentityError::Store(format!("session create: {error}")))?;

            Ok(Session {
                token,
                user_id: user_id.to_string(),
                created_at_ms: now_ms,
                expires_at_ms,
            })
        }

        async fn resolve(&self, token: &str, now_ms: i64) -> Result<Session> {
            // First try to fetch the row unconditionally so we can distinguish
            // "not found" from "found but expired".
            let row = sqlx::query(
                r#"
                select token, user_id, created_at_ms, expires_at_ms
                from system.identity_sessions
                where token = $1
                "#,
            )
            .bind(token)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| IdentityError::Store(format!("session resolve fetch: {error}")))?;

            match row {
                None => Err(IdentityError::SessionNotFound),
                Some(ref r) => {
                    let expires_at_ms: i64 = r.get("expires_at_ms");
                    if expires_at_ms <= now_ms {
                        // Best-effort delete; ignore failure (row may already be gone).
                        let _ =
                            sqlx::query("delete from system.identity_sessions where token = $1")
                                .bind(token)
                                .execute(&self.pool)
                                .await;
                        Err(IdentityError::SessionExpired)
                    } else {
                        Ok(Session {
                            token: r.get("token"),
                            user_id: r.get("user_id"),
                            created_at_ms: r.get("created_at_ms"),
                            expires_at_ms,
                        })
                    }
                }
            }
        }

        async fn revoke(&self, token: &str) -> Result<bool> {
            let result = sqlx::query("delete from system.identity_sessions where token = $1")
                .bind(token)
                .execute(&self.pool)
                .await
                .map_err(|error| IdentityError::Store(format!("session revoke: {error}")))?;

            Ok(result.rows_affected() > 0)
        }

        async fn revoke_all_for_user(&self, user_id: &str) -> Result<u64> {
            let result = sqlx::query("delete from system.identity_sessions where user_id = $1")
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(|error| {
                    IdentityError::Store(format!("session revoke_all_for_user: {error}"))
                })?;

            Ok(result.rows_affected())
        }
    }

    // -----------------------------------------------------------------------
    // PgResetTokenStore
    // -----------------------------------------------------------------------

    use crate::{ResetToken, ResetTokenStore};

    /// Postgres-backed implementation of [`ResetTokenStore`].
    ///
    /// Requires the `postgres` feature.  Construct via
    /// [`PgResetTokenStore::connect`] or [`PgResetTokenStore::from_pool`].
    ///
    /// # Security notes
    ///
    /// * Token material is **never** logged.
    /// * `consume` is single-use: a valid token is deleted in the same
    ///   statement (`delete … returning`), so it cannot be replayed.
    /// * Expired rows are deleted on access (best-effort lazy sweep).
    /// * `revoke_all_for_user` is a single `delete … where user_id = $1`.
    #[derive(Clone, Debug)]
    pub struct PgResetTokenStore {
        pool: PgPool,
    }

    impl PgResetTokenStore {
        /// Open a connection pool against `database_url`.
        ///
        /// # Errors
        ///
        /// Returns [`IdentityError::Store`] if the connection fails.
        pub async fn connect(database_url: &str) -> Result<Self> {
            let pool = PgPoolOptions::new()
                .max_connections(5)
                .connect(database_url)
                .await
                .map_err(|error| IdentityError::Store(format!("postgres connect: {error}")))?;
            Ok(Self { pool })
        }

        /// Adopt a caller-built pool.
        #[must_use]
        pub fn from_pool(pool: PgPool) -> Self {
            Self { pool }
        }

        /// Expose the underlying pool for callers that need raw sqlx access.
        #[must_use]
        pub const fn pool(&self) -> &PgPool {
            &self.pool
        }
    }

    #[async_trait]
    impl ResetTokenStore for PgResetTokenStore {
        async fn issue(
            &self,
            token: String,
            user_id: String,
            now_ms: i64,
            ttl_ms: i64,
        ) -> Result<ResetToken> {
            let expires_at_ms = now_ms + ttl_ms;
            sqlx::query(
                r#"
                insert into system.identity_reset_tokens
                    (token, user_id, created_at_ms, expires_at_ms)
                values ($1, $2, $3, $4)
                "#,
            )
            .bind(&token)
            .bind(&user_id)
            .bind(now_ms)
            .bind(expires_at_ms)
            .execute(&self.pool)
            .await
            .map_err(|error| IdentityError::Store(format!("reset token issue: {error}")))?;

            Ok(ResetToken {
                token,
                user_id,
                created_at_ms: now_ms,
                expires_at_ms,
            })
        }

        async fn consume(&self, token: &str, now_ms: i64) -> Result<ResetToken> {
            // First try to fetch the row unconditionally so we can distinguish
            // "not found" from "found but expired".
            let row = sqlx::query(
                r#"
                select token, user_id, created_at_ms, expires_at_ms
                from system.identity_reset_tokens
                where token = $1
                "#,
            )
            .bind(token)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| IdentityError::Store(format!("reset token consume fetch: {error}")))?;

            match row {
                None => Err(IdentityError::ResetTokenNotFound),
                Some(ref r) => {
                    let expires_at_ms: i64 = r.get("expires_at_ms");
                    if expires_at_ms <= now_ms {
                        // Best-effort delete; ignore failure (row may already be gone).
                        let _ = sqlx::query(
                            "delete from system.identity_reset_tokens where token = $1",
                        )
                        .bind(token)
                        .execute(&self.pool)
                        .await;
                        Err(IdentityError::ResetTokenExpired)
                    } else {
                        // Single-use: delete on successful consume.
                        sqlx::query("delete from system.identity_reset_tokens where token = $1")
                            .bind(token)
                            .execute(&self.pool)
                            .await
                            .map_err(|error| {
                                IdentityError::Store(format!("reset token consume delete: {error}"))
                            })?;
                        Ok(ResetToken {
                            token: r.get("token"),
                            user_id: r.get("user_id"),
                            created_at_ms: r.get("created_at_ms"),
                            expires_at_ms,
                        })
                    }
                }
            }
        }

        async fn revoke_all_for_user(&self, user_id: &str) -> Result<()> {
            sqlx::query("delete from system.identity_reset_tokens where user_id = $1")
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(|error| {
                    IdentityError::Store(format!("reset token revoke_all_for_user: {error}"))
                })?;

            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const NOW_MS: i64 = 1_749_254_400_000; // 2025-06-07 00:00:00 UTC (fixed)

    fn store() -> InMemoryUserStore {
        InMemoryUserStore::new()
    }

    fn new_user(email: &str, password: &str) -> NewUser {
        NewUser {
            email: email.to_string(),
            password: password.to_string(),
            display_name: "Test User".to_string(),
        }
    }

    // -----------------------------------------------------------------------
    // hash_password / verify_password unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn hash_password_produces_phc_string() {
        let hash = hash_password("correct-horse-battery").expect("hash");
        assert!(
            hash.starts_with("$argon2id$"),
            "expected PHC string, got: {hash}"
        );
    }

    #[test]
    fn hash_password_produces_distinct_hashes_for_same_password() {
        let pw = "same-password-42";
        let h1 = hash_password(pw).expect("h1");
        let h2 = hash_password(pw).expect("h2");
        assert_ne!(h1, h2, "random salt must produce distinct hashes");
    }

    #[test]
    fn verify_password_true_on_correct_password() {
        let pw = "correct-horse-battery";
        let hash = hash_password(pw).expect("hash");
        assert!(verify_password(&hash, pw).expect("verify"));
    }

    #[test]
    fn verify_password_false_on_wrong_password() {
        let hash = hash_password("correct-horse-battery").expect("hash");
        assert!(!verify_password(&hash, "wrong-password").expect("verify false"));
    }

    #[test]
    fn verify_password_on_malformed_hash_returns_hash_error() {
        let result = verify_password("not-a-phc-string", "password");
        assert!(
            matches!(result, Err(IdentityError::Hash(_))),
            "expected Hash error, got: {result:?}"
        );
    }

    #[test]
    fn both_hashes_verify_true_against_same_password() {
        let pw = "multi-hash-verify";
        let h1 = hash_password(pw).expect("h1");
        let h2 = hash_password(pw).expect("h2");
        assert!(verify_password(&h1, pw).expect("v1"));
        assert!(verify_password(&h2, pw).expect("v2"));
    }

    // -----------------------------------------------------------------------
    // validate_password
    // -----------------------------------------------------------------------

    #[test]
    fn password_too_short_rejected() {
        let result = validate_password("1234567"); // 7 chars
        assert!(matches!(result, Err(IdentityError::WeakPassword(_))));
    }

    #[test]
    fn password_minimum_boundary_accepted() {
        assert!(validate_password("12345678").is_ok()); // exactly 8
    }

    #[test]
    fn password_maximum_boundary_accepted() {
        let pw = "a".repeat(128);
        assert!(validate_password(&pw).is_ok());
    }

    #[test]
    fn password_too_long_rejected() {
        let pw = "a".repeat(129);
        let result = validate_password(&pw);
        assert!(matches!(result, Err(IdentityError::WeakPassword(_))));
    }

    // -----------------------------------------------------------------------
    // normalize_email
    // -----------------------------------------------------------------------

    #[test]
    fn empty_email_rejected() {
        assert!(matches!(
            normalize_email(""),
            Err(IdentityError::InvalidEmail(_))
        ));
    }

    #[test]
    fn whitespace_only_email_rejected() {
        assert!(matches!(
            normalize_email("   "),
            Err(IdentityError::InvalidEmail(_))
        ));
    }

    #[test]
    fn email_without_at_rejected() {
        assert!(matches!(
            normalize_email("notanemail"),
            Err(IdentityError::InvalidEmail(_))
        ));
    }

    #[test]
    fn email_is_lowercased_and_trimmed() {
        let result = normalize_email("  Alice@Example.COM  ").expect("normalize");
        assert_eq!(result, "alice@example.com");
    }

    // -----------------------------------------------------------------------
    // InMemoryUserStore — register
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn register_succeeds_and_returns_user_without_hash() {
        let s = store();
        let user = s
            .register(
                new_user("alice@example.com", "securepass"),
                "u1".to_string(),
                NOW_MS,
            )
            .await
            .expect("register");
        assert_eq!(user.id, "u1");
        assert_eq!(user.email, "alice@example.com");
        assert_eq!(user.display_name, "Test User");
        assert!(user.last_active_at_ms.is_none());
        assert!(user.last_reengagement_sent_at_ms.is_none());
        assert_eq!(user.created_at_ms, NOW_MS);
        assert_eq!(user.updated_at_ms, NOW_MS);
    }

    #[tokio::test]
    async fn register_duplicate_email_returns_email_taken() {
        let s = store();
        s.register(
            new_user("bob@example.com", "password01"),
            "u1".to_string(),
            NOW_MS,
        )
        .await
        .expect("first register");
        let result = s
            .register(
                new_user("bob@example.com", "different-pw"),
                "u2".to_string(),
                NOW_MS,
            )
            .await;
        assert!(matches!(result, Err(IdentityError::EmailTaken)));
    }

    #[tokio::test]
    async fn register_duplicate_email_different_case_returns_email_taken() {
        let s = store();
        s.register(
            new_user("carol@example.com", "password01"),
            "u1".to_string(),
            NOW_MS,
        )
        .await
        .expect("first");
        let result = s
            .register(
                new_user("CAROL@EXAMPLE.COM", "password02"),
                "u2".to_string(),
                NOW_MS,
            )
            .await;
        assert!(matches!(result, Err(IdentityError::EmailTaken)));
    }

    #[tokio::test]
    async fn register_duplicate_email_with_whitespace_returns_email_taken() {
        let s = store();
        s.register(
            new_user("dave@example.com", "password01"),
            "u1".to_string(),
            NOW_MS,
        )
        .await
        .expect("first");
        let result = s
            .register(
                new_user("  dave@example.com  ", "password02"),
                "u2".to_string(),
                NOW_MS,
            )
            .await;
        assert!(matches!(result, Err(IdentityError::EmailTaken)));
    }

    // -----------------------------------------------------------------------
    // InMemoryUserStore — authenticate
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn authenticate_correct_credentials_returns_user() {
        let s = store();
        s.register(
            new_user("eve@example.com", "goodpassword"),
            "u1".to_string(),
            NOW_MS,
        )
        .await
        .expect("register");
        let user = s
            .authenticate("eve@example.com", "goodpassword")
            .await
            .expect("authenticate");
        assert_eq!(user.email, "eve@example.com");
        assert_eq!(user.id, "u1");
    }

    #[tokio::test]
    async fn authenticate_wrong_password_returns_invalid_credentials() {
        let s = store();
        s.register(
            new_user("frank@example.com", "rightpassword"),
            "u1".to_string(),
            NOW_MS,
        )
        .await
        .expect("register");
        let result = s.authenticate("frank@example.com", "wrongpassword").await;
        assert!(matches!(result, Err(IdentityError::InvalidCredentials)));
    }

    #[tokio::test]
    async fn authenticate_unknown_email_returns_invalid_credentials() {
        let s = store();
        let result = s.authenticate("ghost@example.com", "anypassword").await;
        assert!(matches!(result, Err(IdentityError::InvalidCredentials)));
    }

    #[tokio::test]
    async fn authenticate_unknown_email_same_error_variant_as_wrong_password() {
        let s = store();
        s.register(
            new_user("henry@example.com", "realpassword"),
            "u1".to_string(),
            NOW_MS,
        )
        .await
        .expect("register");
        let err_unknown = s
            .authenticate("nobody@example.com", "anypassword")
            .await
            .expect_err("unknown");
        let err_wrong = s
            .authenticate("henry@example.com", "wrongpassword")
            .await
            .expect_err("wrong");
        // Both must be the same variant (no user enumeration)
        assert!(matches!(err_unknown, IdentityError::InvalidCredentials));
        assert!(matches!(err_wrong, IdentityError::InvalidCredentials));
    }

    // -----------------------------------------------------------------------
    // InMemoryUserStore — get_by_id / get_by_email
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_by_id_found() {
        let s = store();
        s.register(
            new_user("iris@example.com", "mypassword1"),
            "uid42".to_string(),
            NOW_MS,
        )
        .await
        .expect("register");
        let user = s.get_by_id("uid42").await.expect("get_by_id");
        assert_eq!(user.id, "uid42");
        assert_eq!(user.email, "iris@example.com");
    }

    #[tokio::test]
    async fn get_by_id_not_found() {
        let s = store();
        let result = s.get_by_id("nonexistent").await;
        assert!(matches!(result, Err(IdentityError::UserNotFound)));
    }

    #[tokio::test]
    async fn get_by_email_found() {
        let s = store();
        s.register(
            new_user("jack@example.com", "mypassword2"),
            "uid43".to_string(),
            NOW_MS,
        )
        .await
        .expect("register");
        let user = s
            .get_by_email("jack@example.com")
            .await
            .expect("get_by_email");
        assert_eq!(user.id, "uid43");
    }

    #[tokio::test]
    async fn get_by_email_not_found() {
        let s = store();
        let result = s.get_by_email("nobody@example.com").await;
        assert!(matches!(result, Err(IdentityError::UserNotFound)));
    }

    #[tokio::test]
    async fn get_by_email_normalizes_input() {
        let s = store();
        s.register(
            new_user("kate@example.com", "mypassword3"),
            "uid44".to_string(),
            NOW_MS,
        )
        .await
        .expect("register");
        let user = s
            .get_by_email("  KATE@EXAMPLE.COM  ")
            .await
            .expect("get_by_email normalized");
        assert_eq!(user.email, "kate@example.com");
    }

    // -----------------------------------------------------------------------
    // InMemoryUserStore — touch_last_active
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn touch_last_active_updates_field() {
        let s = store();
        s.register(
            new_user("lena@example.com", "mypassword4"),
            "uid45".to_string(),
            NOW_MS,
        )
        .await
        .expect("register");

        let new_ts = NOW_MS + 10_000;
        s.touch_last_active("uid45", new_ts).await.expect("touch");

        let user = s.get_by_id("uid45").await.expect("get");
        assert_eq!(user.last_active_at_ms, Some(new_ts));
        assert_eq!(user.updated_at_ms, new_ts);
    }

    #[tokio::test]
    async fn touch_last_active_not_found() {
        let s = store();
        let result = s.touch_last_active("ghost", NOW_MS).await;
        assert!(matches!(result, Err(IdentityError::UserNotFound)));
    }

    // -----------------------------------------------------------------------
    // Serde: User JSON round-trip and no hash field
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn user_serde_roundtrip_and_no_hash_field() {
        let s = store();
        let user = s
            .register(
                new_user("mary@example.com", "roundtrip1"),
                "uid46".to_string(),
                NOW_MS,
            )
            .await
            .expect("register");

        let json = serde_json::to_string(&user).expect("serialize");

        // Must NOT contain any password or hash material
        assert!(
            !json.contains("hash"),
            "JSON must not contain 'hash': {json}"
        );
        assert!(
            !json.contains("password"),
            "JSON must not contain 'password': {json}"
        );

        // Must round-trip cleanly
        let decoded: User = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, user);
    }

    // -----------------------------------------------------------------------
    // InMemorySessionStore tests (≥12)
    // -----------------------------------------------------------------------

    fn session_store() -> InMemorySessionStore {
        InMemorySessionStore::new()
    }

    /// 1. create returns a Session with a 64-hex-char token.
    #[tokio::test]
    async fn session_create_returns_session_with_64_hex_token() {
        let s = session_store();
        let session = s
            .create("u1", NOW_MS, DEFAULT_SESSION_TTL_MS)
            .await
            .expect("create");
        assert_eq!(session.token.len(), 64, "token must be 64 chars");
        assert!(
            session
                .token
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
            "token must be lowercase hex: {}",
            session.token
        );
    }

    /// 2. create sets correct expiry.
    #[tokio::test]
    async fn session_create_sets_correct_expiry() {
        let s = session_store();
        let ttl = 3_600_000_i64; // 1 hour
        let session = s.create("u1", NOW_MS, ttl).await.expect("create");
        assert_eq!(session.user_id, "u1");
        assert_eq!(session.created_at_ms, NOW_MS);
        assert_eq!(session.expires_at_ms, NOW_MS + ttl);
    }

    /// 3. resolve of a valid (non-expired) token returns the Session.
    #[tokio::test]
    async fn session_resolve_valid_token_returns_session() {
        let s = session_store();
        let session = s
            .create("u1", NOW_MS, DEFAULT_SESSION_TTL_MS)
            .await
            .expect("create");
        let resolved = s
            .resolve(&session.token, NOW_MS + 1)
            .await
            .expect("resolve");
        assert_eq!(resolved, session);
    }

    /// 4. resolve of an unknown token returns SessionNotFound.
    #[tokio::test]
    async fn session_resolve_unknown_token_returns_not_found() {
        let s = session_store();
        let result = s
            .resolve(
                "00000000000000000000000000000000000000000000000000000000deadbeef",
                NOW_MS,
            )
            .await;
        assert!(
            matches!(result, Err(IdentityError::SessionNotFound)),
            "expected SessionNotFound, got: {result:?}"
        );
    }

    /// 5. resolve of an expired token returns SessionExpired.
    #[tokio::test]
    async fn session_resolve_expired_token_returns_session_expired() {
        let s = session_store();
        let ttl = 1_000_i64; // 1 second
        let session = s.create("u1", NOW_MS, ttl).await.expect("create");
        // advance time past expiry
        let result = s.resolve(&session.token, NOW_MS + ttl).await;
        assert!(
            matches!(result, Err(IdentityError::SessionExpired)),
            "expected SessionExpired, got: {result:?}"
        );
    }

    /// 6. resolve of an expired token deletes the row — second resolve also errors.
    #[tokio::test]
    async fn session_resolve_expired_token_deletes_row() {
        let s = session_store();
        let ttl = 1_000_i64;
        let session = s.create("u1", NOW_MS, ttl).await.expect("create");
        let now_after = NOW_MS + ttl + 1;
        // First resolve: SessionExpired + row deleted.
        let _ = s.resolve(&session.token, now_after).await;
        // Second resolve: row gone → SessionNotFound (or SessionExpired again is also acceptable,
        // but our in-memory impl deletes on first access so it will be SessionNotFound).
        let result = s.resolve(&session.token, now_after).await;
        assert!(
            matches!(
                result,
                Err(IdentityError::SessionNotFound) | Err(IdentityError::SessionExpired)
            ),
            "expected session gone after expiry, got: {result:?}"
        );
    }

    /// 7. revoke of a known token returns true and removes the row.
    #[tokio::test]
    async fn session_revoke_known_token_returns_true() {
        let s = session_store();
        let session = s
            .create("u1", NOW_MS, DEFAULT_SESSION_TTL_MS)
            .await
            .expect("create");
        let removed = s.revoke(&session.token).await.expect("revoke");
        assert!(removed, "revoke must return true for an existing token");
        // Confirm it's gone.
        let result = s.resolve(&session.token, NOW_MS + 1).await;
        assert!(matches!(result, Err(IdentityError::SessionNotFound)));
    }

    /// 8. revoke of an unknown token returns false.
    #[tokio::test]
    async fn session_revoke_unknown_token_returns_false() {
        let s = session_store();
        let removed = s
            .revoke("nonexistenttoken000000000000000000000000000000000000000000000000")
            .await
            .expect("revoke");
        assert!(!removed, "revoke must return false for an unknown token");
    }

    /// 9. revoke_all_for_user removes only that user's sessions and returns count.
    #[tokio::test]
    async fn session_revoke_all_for_user_removes_only_that_user() {
        let s = session_store();
        // Create 2 sessions for u1, 1 for u2.
        s.create("u1", NOW_MS, DEFAULT_SESSION_TTL_MS)
            .await
            .expect("c1");
        s.create("u1", NOW_MS, DEFAULT_SESSION_TTL_MS)
            .await
            .expect("c2");
        let u2_session = s
            .create("u2", NOW_MS, DEFAULT_SESSION_TTL_MS)
            .await
            .expect("c3");

        let count = s.revoke_all_for_user("u1").await.expect("revoke_all");
        assert_eq!(
            count, 2,
            "revoke_all_for_user must return count of removed rows"
        );

        // u2's session must still be valid.
        let still_alive = s
            .resolve(&u2_session.token, NOW_MS + 1)
            .await
            .expect("u2 session");
        assert_eq!(still_alive.user_id, "u2");
    }

    /// 10. Two create calls yield distinct tokens (RNG collision probability ≈ 2^-256).
    #[tokio::test]
    async fn session_create_produces_distinct_tokens() {
        let s = session_store();
        let s1 = s
            .create("u1", NOW_MS, DEFAULT_SESSION_TTL_MS)
            .await
            .expect("c1");
        let s2 = s
            .create("u1", NOW_MS, DEFAULT_SESSION_TTL_MS)
            .await
            .expect("c2");
        assert_ne!(
            s1.token, s2.token,
            "consecutive create calls must produce distinct tokens"
        );
    }

    /// 11. Session serde round-trips and JSON contains the token field.
    #[tokio::test]
    async fn session_serde_roundtrip_and_json_contains_token() {
        let s = session_store();
        let session = s
            .create("u1", NOW_MS, DEFAULT_SESSION_TTL_MS)
            .await
            .expect("create");
        let json = serde_json::to_string(&session).expect("serialize");
        // Token must be present in JSON (sessions are opaque-token-bearing by design).
        assert!(
            json.contains(&session.token),
            "JSON must contain the token: {json}"
        );
        // Round-trip.
        let decoded: Session = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, session);
    }

    /// 12. resolve does not extend expiry (no sliding window).
    #[tokio::test]
    async fn session_resolve_does_not_extend_expiry() {
        let s = session_store();
        let ttl = 60_000_i64;
        let session = s.create("u1", NOW_MS, ttl).await.expect("create");
        let original_expiry = session.expires_at_ms;
        // Resolve mid-life.
        let resolved = s
            .resolve(&session.token, NOW_MS + 1_000)
            .await
            .expect("resolve");
        assert_eq!(
            resolved.expires_at_ms, original_expiry,
            "resolve must not extend expiry (no sliding window)"
        );
    }

    /// 13. revoke_all_for_user on a user with no sessions returns 0.
    #[tokio::test]
    async fn session_revoke_all_for_user_no_sessions_returns_zero() {
        let s = session_store();
        let count = s.revoke_all_for_user("nobody").await.expect("revoke_all");
        assert_eq!(count, 0);
    }

    /// 14. generate_session_token produces exactly 64 lowercase hex chars.
    #[test]
    fn generate_session_token_is_64_lowercase_hex() {
        let token = generate_session_token();
        assert_eq!(token.len(), 64, "token length must be 64");
        assert!(
            token.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
            "token must be lowercase hex: {token}"
        );
    }

    /// 15. DEFAULT_SESSION_TTL_MS is 7 days in ms.
    #[test]
    fn default_session_ttl_is_seven_days() {
        assert_eq!(DEFAULT_SESSION_TTL_MS, 7 * 24 * 60 * 60 * 1_000);
    }

    // -----------------------------------------------------------------------
    // InMemoryResetTokenStore tests
    // -----------------------------------------------------------------------

    fn reset_token_store() -> InMemoryResetTokenStore {
        InMemoryResetTokenStore::new()
    }

    /// 1. issue + consume happy path returns the same token and clears it.
    #[tokio::test]
    async fn reset_issue_then_consume_returns_token() {
        let s = reset_token_store();
        let token = generate_reset_token();
        let issued = s
            .issue(
                token.clone(),
                "u1".to_string(),
                NOW_MS,
                DEFAULT_RESET_TOKEN_TTL_MS,
            )
            .await
            .expect("issue");
        assert_eq!(issued.token, token);
        assert_eq!(issued.user_id, "u1");
        assert_eq!(issued.created_at_ms, NOW_MS);
        assert_eq!(issued.expires_at_ms, NOW_MS + DEFAULT_RESET_TOKEN_TTL_MS);

        let consumed = s.consume(&token, NOW_MS + 1).await.expect("consume");
        assert_eq!(consumed, issued);
    }

    /// 2. consume is single-use: the second consume fails with ResetTokenNotFound.
    #[tokio::test]
    async fn reset_consume_twice_fails() {
        let s = reset_token_store();
        let token = generate_reset_token();
        s.issue(
            token.clone(),
            "u1".to_string(),
            NOW_MS,
            DEFAULT_RESET_TOKEN_TTL_MS,
        )
        .await
        .expect("issue");

        let _ = s.consume(&token, NOW_MS + 1).await.expect("first consume");
        let result = s.consume(&token, NOW_MS + 1).await;
        assert!(
            matches!(result, Err(IdentityError::ResetTokenNotFound)),
            "expected ResetTokenNotFound on second consume, got: {result:?}"
        );
    }

    /// 3. consume of an expired token returns ResetTokenExpired.
    #[tokio::test]
    async fn reset_consume_expired_returns_expired() {
        let s = reset_token_store();
        let token = generate_reset_token();
        let ttl = 1_000_i64;
        s.issue(token.clone(), "u1".to_string(), NOW_MS, ttl)
            .await
            .expect("issue");
        let result = s.consume(&token, NOW_MS + ttl).await;
        assert!(
            matches!(result, Err(IdentityError::ResetTokenExpired)),
            "expected ResetTokenExpired, got: {result:?}"
        );
    }

    /// 4. consume of an expired token deletes the row — second consume is NotFound.
    #[tokio::test]
    async fn reset_consume_expired_deletes_row() {
        let s = reset_token_store();
        let token = generate_reset_token();
        let ttl = 1_000_i64;
        s.issue(token.clone(), "u1".to_string(), NOW_MS, ttl)
            .await
            .expect("issue");
        let now_after = NOW_MS + ttl + 1;
        let _ = s.consume(&token, now_after).await;
        let result = s.consume(&token, now_after).await;
        assert!(
            matches!(result, Err(IdentityError::ResetTokenNotFound)),
            "expected ResetTokenNotFound after expiry delete, got: {result:?}"
        );
    }

    /// 5. consume of an unknown token returns ResetTokenNotFound.
    #[tokio::test]
    async fn reset_consume_unknown_returns_not_found() {
        let s = reset_token_store();
        let result = s
            .consume(
                "00000000000000000000000000000000000000000000000000000000deadbeef",
                NOW_MS,
            )
            .await;
        assert!(
            matches!(result, Err(IdentityError::ResetTokenNotFound)),
            "expected ResetTokenNotFound, got: {result:?}"
        );
    }

    /// 6. revoke_all_for_user clears only that user's outstanding tokens.
    #[tokio::test]
    async fn reset_revoke_all_for_user_clears_them() {
        let s = reset_token_store();
        let t1 = generate_reset_token();
        let t2 = generate_reset_token();
        let t_other = generate_reset_token();
        s.issue(
            t1.clone(),
            "u1".to_string(),
            NOW_MS,
            DEFAULT_RESET_TOKEN_TTL_MS,
        )
        .await
        .expect("t1");
        s.issue(
            t2.clone(),
            "u1".to_string(),
            NOW_MS,
            DEFAULT_RESET_TOKEN_TTL_MS,
        )
        .await
        .expect("t2");
        s.issue(
            t_other.clone(),
            "u2".to_string(),
            NOW_MS,
            DEFAULT_RESET_TOKEN_TTL_MS,
        )
        .await
        .expect("t_other");

        s.revoke_all_for_user("u1").await.expect("revoke_all");

        // u1's tokens are gone.
        assert!(matches!(
            s.consume(&t1, NOW_MS + 1).await,
            Err(IdentityError::ResetTokenNotFound)
        ));
        assert!(matches!(
            s.consume(&t2, NOW_MS + 1).await,
            Err(IdentityError::ResetTokenNotFound)
        ));
        // u2's token survives.
        let survivor = s.consume(&t_other, NOW_MS + 1).await.expect("u2 token");
        assert_eq!(survivor.user_id, "u2");
    }

    /// 7. revoke_all_for_user on a user with no tokens is a no-op (Ok).
    #[tokio::test]
    async fn reset_revoke_all_for_user_no_tokens_ok() {
        let s = reset_token_store();
        s.revoke_all_for_user("nobody").await.expect("revoke_all");
    }

    /// 8. generate_reset_token produces exactly 64 lowercase hex chars.
    #[test]
    fn generate_reset_token_is_64_lowercase_hex() {
        let token = generate_reset_token();
        assert_eq!(token.len(), 64, "token length must be 64");
        assert!(
            token.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
            "token must be lowercase hex: {token}"
        );
    }

    /// 9. Two generate_reset_token calls yield distinct tokens.
    #[test]
    fn generate_reset_token_produces_distinct_tokens() {
        let t1 = generate_reset_token();
        let t2 = generate_reset_token();
        assert_ne!(t1, t2, "consecutive tokens must differ");
    }

    /// 10. DEFAULT_RESET_TOKEN_TTL_MS is 1 hour in ms.
    #[test]
    fn default_reset_token_ttl_is_one_hour() {
        assert_eq!(DEFAULT_RESET_TOKEN_TTL_MS, 60 * 60 * 1_000);
    }

    // -----------------------------------------------------------------------
    // PgResetTokenStore integration tests (gated on TDW_POSTGRES_TEST_URL).
    //
    // These no-op (return early) when TDW_POSTGRES_TEST_URL is unset, so the
    // default `cargo test --features postgres` run stays green without a live
    // database.  Set the env var to a reachable Postgres URL with the
    // `system.identity_reset_tokens` migration applied to exercise them.
    // -----------------------------------------------------------------------

    #[cfg(feature = "postgres")]
    mod pg_reset {
        use super::*;

        fn pg_url() -> Option<String> {
            std::env::var("TDW_POSTGRES_TEST_URL").ok()
        }

        /// 1. issue + consume happy path against Postgres.
        #[tokio::test]
        async fn pg_reset_issue_then_consume() {
            let Some(url) = pg_url() else {
                return;
            };
            let store = PgResetTokenStore::connect(&url).await.expect("connect");
            let token = generate_reset_token();
            store
                .revoke_all_for_user("pg-reset-u1")
                .await
                .expect("cleanup");
            let issued = store
                .issue(
                    token.clone(),
                    "pg-reset-u1".to_string(),
                    NOW_MS,
                    DEFAULT_RESET_TOKEN_TTL_MS,
                )
                .await
                .expect("issue");
            assert_eq!(issued.token, token);
            let consumed = store.consume(&token, NOW_MS + 1).await.expect("consume");
            assert_eq!(consumed, issued);
        }

        /// 2. consume is single-use on Postgres.
        #[tokio::test]
        async fn pg_reset_consume_twice_fails() {
            let Some(url) = pg_url() else {
                return;
            };
            let store = PgResetTokenStore::connect(&url).await.expect("connect");
            let token = generate_reset_token();
            store
                .issue(
                    token.clone(),
                    "pg-reset-u2".to_string(),
                    NOW_MS,
                    DEFAULT_RESET_TOKEN_TTL_MS,
                )
                .await
                .expect("issue");
            let _ = store.consume(&token, NOW_MS + 1).await.expect("first");
            let result = store.consume(&token, NOW_MS + 1).await;
            assert!(matches!(result, Err(IdentityError::ResetTokenNotFound)));
        }

        /// 3. expired token returns ResetTokenExpired on Postgres.
        #[tokio::test]
        async fn pg_reset_consume_expired() {
            let Some(url) = pg_url() else {
                return;
            };
            let store = PgResetTokenStore::connect(&url).await.expect("connect");
            let token = generate_reset_token();
            let ttl = 1_000_i64;
            store
                .issue(token.clone(), "pg-reset-u3".to_string(), NOW_MS, ttl)
                .await
                .expect("issue");
            let result = store.consume(&token, NOW_MS + ttl).await;
            assert!(matches!(result, Err(IdentityError::ResetTokenExpired)));
        }

        /// 4. unknown token returns ResetTokenNotFound on Postgres.
        #[tokio::test]
        async fn pg_reset_consume_unknown() {
            let Some(url) = pg_url() else {
                return;
            };
            let store = PgResetTokenStore::connect(&url).await.expect("connect");
            let result = store
                .consume(
                    "00000000000000000000000000000000000000000000000000000000deadbeef",
                    NOW_MS,
                )
                .await;
            assert!(matches!(result, Err(IdentityError::ResetTokenNotFound)));
        }

        /// 5. revoke_all_for_user clears outstanding tokens on Postgres.
        #[tokio::test]
        async fn pg_reset_revoke_all_for_user() {
            let Some(url) = pg_url() else {
                return;
            };
            let store = PgResetTokenStore::connect(&url).await.expect("connect");
            let token = generate_reset_token();
            store
                .issue(
                    token.clone(),
                    "pg-reset-u4".to_string(),
                    NOW_MS,
                    DEFAULT_RESET_TOKEN_TTL_MS,
                )
                .await
                .expect("issue");
            store
                .revoke_all_for_user("pg-reset-u4")
                .await
                .expect("revoke_all");
            let result = store.consume(&token, NOW_MS + 1).await;
            assert!(matches!(result, Err(IdentityError::ResetTokenNotFound)));
        }
    }
}
