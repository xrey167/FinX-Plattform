//! Event-driven registry hot-reload, backed by the `notify` crate.
//!
//! [`RegistryWatcher`] watches a registry's source directory and reloads it in place on
//! filesystem events — the event-driven complement to the dependency-free polling
//! [`crate::registry::Registry::reload_if_changed`]. The reload is atomic (a bad file
//! leaves the live registry intact) and best-effort (transient errors mid-write are
//! ignored; the next event reloads).

use std::path::Path;
use std::sync::{Arc, Mutex};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use thiserror::Error;

use crate::registry::Registry;

/// Error starting a [`RegistryWatcher`].
#[derive(Debug, Error)]
pub enum WatchError {
    /// The registry was not built from a directory, so there is nothing to watch.
    #[error("the registry has no source directory to watch")]
    NoSource,
    /// The underlying `notify` watcher could not be created or started.
    #[error("notify error: {0}")]
    Notify(String),
}

/// Watches a registry's source directory and reloads it on filesystem events. The watch
/// stops when this value is dropped.
pub struct RegistryWatcher {
    _watcher: RecommendedWatcher,
}

impl RegistryWatcher {
    /// Start watching the shared registry's source directory; on any filesystem event the
    /// registry is reloaded in place via
    /// [`reload_if_changed`](crate::registry::Registry::reload_if_changed).
    ///
    /// # Errors
    ///
    /// Returns [`WatchError::NoSource`] if the registry has no source directory, or
    /// [`WatchError::Notify`] if the watcher cannot be created or started.
    pub fn watch(registry: &Arc<Mutex<Registry>>) -> Result<Self, WatchError> {
        let dir = registry
            .lock()
            .map_err(|_| WatchError::Notify("registry mutex poisoned".to_string()))?
            .source()
            .map(Path::to_path_buf)
            .ok_or(WatchError::NoSource)?;

        let registry_for_callback = Arc::clone(registry);
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            if event.is_err() {
                return;
            }
            match registry_for_callback.lock() {
                // The reload is atomic on success (a bad file leaves the live registry
                // intact). A persistent reload error is logged so silent staleness on a
                // live server is observable, rather than swallowed.
                Ok(mut guard) => {
                    if let Err(error) = guard.reload_if_changed() {
                        eprintln!("tdw-agent: registry hot-reload failed: {error}");
                    }
                }
                Err(_) => eprintln!("tdw-agent: registry watcher stopped — mutex poisoned"),
            }
        })
        .map_err(|error| WatchError::Notify(error.to_string()))?;

        watcher
            .watch(&dir, RecursiveMode::NonRecursive)
            .map_err(|error| WatchError::Notify(error.to_string()))?;

        Ok(Self { _watcher: watcher })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watching_without_source_errors() {
        let registry = Arc::new(Mutex::new(Registry::new()));
        assert!(matches!(
            RegistryWatcher::watch(&registry),
            Err(WatchError::NoSource)
        ));
    }

    #[test]
    fn watcher_starts_on_a_sourced_registry() {
        // Event delivery is async and platform-specific, so it is not asserted here; this
        // confirms the watcher constructs and attaches to a real source directory.
        let dir = Path::new("tests/registry");
        let registry = Arc::new(Mutex::new(Registry::load_dir(dir).expect("load")));
        let _watcher = RegistryWatcher::watch(&registry).expect("watcher should start");
    }
}
