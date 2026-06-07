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
//! * [`IdentityError`] — domain errors (email validation, weak password,
//!   duplicate email, credential mismatch, etc.).
//! * [`UserStore`] — async persistence trait.
//! * [`InMemoryUserStore`] — always-compiled in-memory reference
//!   implementation (unit tests, offline scenarios).
//! * [`PgUserStore`] — Postgres-backed implementation behind the `postgres`
//!   feature flag.
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
pub use pg::PgUserStore;

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
}
