//! [`Trigger`] — the two ways a function can be activated.

use serde::{Deserialize, Serialize};

/// Describes what activates a [`super::FunctionDef`].
///
/// Triggers are metadata only in this sub-PR; nothing consumes them yet.
/// [`Trigger::Cron`] uses a 5-field expression (`min hour dom month dow`)
/// matching the format accepted by `tdw-cron`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trigger {
    /// Activated by a named event.
    Event(String),
    /// Activated on a schedule expressed as a 5-field cron string.
    ///
    /// Example: `"0 9 * * 1-5"` fires at 09:00 every weekday.
    Cron(String),
}
