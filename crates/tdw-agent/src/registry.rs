//! In-memory registry: load a folder of JSON5 definition files into an index keyed by
//! `(kind, name)`.
//!
//! This is the concrete "registry" that sits on top of the JSON5 [`crate::loader`] — a
//! directory of `*.json5` definitions becomes a queryable index. It is the lead-in to the
//! plugin / hot-reload direction (re-run [`Registry::load_dir`] to reload).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::kind::EntityKind;
use crate::loader::{LoadError, load_resource};
use crate::resource::{Resource, entity_from_resource};

/// An index of loaded resource definitions, keyed by `kind` then `name`.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    entries: BTreeMap<EntityKind, BTreeMap<String, Resource<Value>>>,
    /// The directory this registry was loaded from, if any (enables [`Registry::reload`]).
    source: Option<PathBuf>,
    /// Per-file modified times captured at load, for change detection in
    /// [`Registry::reload_if_changed`].
    stamps: BTreeMap<PathBuf, std::time::SystemTime>,
}

impl Registry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a registry from an iterator of resources.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::Duplicate`] if two resources share a `(kind, name)` key.
    pub fn from_resources(
        resources: impl IntoIterator<Item = Resource<Value>>,
    ) -> Result<Self, LoadError> {
        let mut registry = Self::new();
        for resource in resources {
            registry.insert(resource)?;
        }
        Ok(registry)
    }

    /// Insert one resource, indexed by `(kind, metadata.name)`.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::Invariant`] if a non-autonomy-capable kind is marked
    /// `autonomous`, or [`LoadError::Duplicate`] if the `(kind, name)` key is already
    /// present.
    pub fn insert(&mut self, resource: Resource<Value>) -> Result<(), LoadError> {
        if !resource.kind.is_autonomy_capable() && resource.metadata.autonomous {
            return Err(LoadError::Invariant {
                kind: resource.kind,
                detail: "non-agent kind cannot be autonomous".into(),
            });
        }
        let name = resource.metadata.base.name.clone();
        let by_name = self.entries.entry(resource.kind).or_default();
        if by_name.contains_key(&name) {
            return Err(LoadError::Duplicate {
                kind: resource.kind,
                name,
            });
        }
        by_name.insert(name, resource);
        Ok(())
    }

    /// Load every `*.json5` file in `dir` (non-recursive) and index it.
    ///
    /// Loading does not check plugin integrity — a loaded registry may contain a `plugin`
    /// with a dangling member. Call [`Registry::validate_plugins`] explicitly for that.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::Io`] on filesystem errors, [`LoadError::Parse`] on invalid
    /// JSON5, or [`LoadError::Duplicate`] on a `(kind, name)` collision.
    pub fn load_dir(dir: &Path) -> Result<Self, LoadError> {
        let mut registry = Self::new();
        let read_dir = std::fs::read_dir(dir).map_err(|source| LoadError::Io {
            path: dir.display().to_string(),
            source,
        })?;
        for entry in read_dir {
            let entry = entry.map_err(|source| LoadError::Io {
                path: dir.display().to_string(),
                source,
            })?;
            let path = entry.path();
            if path.extension().and_then(std::ffi::OsStr::to_str) != Some("json5") {
                continue;
            }
            let text = std::fs::read_to_string(&path).map_err(|source| LoadError::Io {
                path: path.display().to_string(),
                source,
            })?;
            registry.insert(load_resource(&text)?)?;
        }
        registry.source = Some(dir.to_path_buf());
        registry.stamps = Self::scan_stamps(dir)?;
        Ok(registry)
    }

    /// Merge another registry's entries into this one, dup-detecting. Used to combine
    /// several plugin directories into one index.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::Invariant`] / [`LoadError::Duplicate`] from [`Registry::insert`]
    /// if a merged resource violates an invariant or collides on `(kind, name)`.
    pub fn merge(&mut self, other: Self) -> Result<(), LoadError> {
        for by_name in other.entries.into_values() {
            for resource in by_name.into_values() {
                self.insert(resource)?;
            }
        }
        Ok(())
    }

    /// Load a plugin root where each immediate subdirectory is a plugin (its own `*.json5`
    /// definitions, including a `plugin` manifest). All plugins merge into one index, then
    /// every plugin's declared members are validated.
    ///
    /// The result has no single `source`, so [`Registry::reload`] / `reload_if_changed` are
    /// no-ops on it and it cannot be watched by `RegistryWatcher`; re-call `load_plugins`
    /// to refresh.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::Io`] / [`LoadError::Parse`] on load failure, [`LoadError::Duplicate`]
    /// on a cross-plugin `(kind, name)` collision, or [`LoadError::MissingMember`] if a plugin
    /// references an unregistered member.
    pub fn load_plugins(root: &Path) -> Result<Self, LoadError> {
        let mut registry = Self::new();
        let read_dir = std::fs::read_dir(root).map_err(|source| LoadError::Io {
            path: root.display().to_string(),
            source,
        })?;
        for entry in read_dir {
            let entry = entry.map_err(|source| LoadError::Io {
                path: root.display().to_string(),
                source,
            })?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            registry.merge(Self::load_dir(&path)?)?;
        }
        registry.validate_plugins()?;
        Ok(registry)
    }

    /// Scan a directory for `*.json5` files and their modified times (no parsing).
    fn scan_stamps(dir: &Path) -> Result<BTreeMap<PathBuf, std::time::SystemTime>, LoadError> {
        let mut stamps = BTreeMap::new();
        let read_dir = std::fs::read_dir(dir).map_err(|source| LoadError::Io {
            path: dir.display().to_string(),
            source,
        })?;
        for entry in read_dir {
            let entry = entry.map_err(|source| LoadError::Io {
                path: dir.display().to_string(),
                source,
            })?;
            let path = entry.path();
            if path.extension().and_then(std::ffi::OsStr::to_str) != Some("json5") {
                continue;
            }
            let modified = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .map_err(|source| LoadError::Io {
                    path: path.display().to_string(),
                    source,
                })?;
            stamps.insert(path, modified);
        }
        Ok(stamps)
    }

    /// Reload only if the source directory's `*.json5` files changed (added, removed, or
    /// modified). Poll this for dependency-free hot-reload — no file-watcher needed. The
    /// swap is atomic (a bad file leaves current contents intact). Returns `true` if a
    /// reload happened, `false` if nothing changed or there is no source directory.
    ///
    /// Change detection compares file modified-times, so a rewrite that preserves a file's
    /// mtime (rare — coarse-resolution filesystems under rapid successive writes) can be
    /// missed until the next genuine change. Use [`Registry::reload`] to force a re-scan.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Registry::load_dir`].
    pub fn reload_if_changed(&mut self) -> Result<bool, LoadError> {
        let Some(dir) = self.source.clone() else {
            return Ok(false);
        };
        if Self::scan_stamps(&dir)? == self.stamps {
            return Ok(false);
        }
        *self = Self::load_dir(&dir)?;
        Ok(true)
    }

    /// Re-scan the source directory and atomically replace the contents.
    ///
    /// If the registry was not built via [`Registry::load_dir`] this is a no-op. On a
    /// reload error (IO/parse/duplicate) the current contents are left unchanged — the
    /// fresh index is only swapped in once it loads cleanly.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Registry::load_dir`].
    pub fn reload(&mut self) -> Result<(), LoadError> {
        if let Some(dir) = self.source.clone() {
            *self = Self::load_dir(&dir)?;
        }
        Ok(())
    }

    /// The directory this registry was loaded from, if any.
    #[must_use]
    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }

    /// Validate that every `plugin`'s declared members resolve to a registered entity
    /// (by name, any kind) — the integrity check for the plugin system. Client-side and
    /// server-side plugins are validated identically; the `client_side`/`server_side`
    /// flags on the `plugin` spec say *where* the members run.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::MissingMember`] for the first plugin member with no matching
    /// entity, or [`LoadError::Parse`] if a plugin spec is malformed.
    pub fn validate_plugins(&self) -> Result<(), LoadError> {
        for resource in self.by_kind(EntityKind::Plugin) {
            let plugin: crate::Plugin = entity_from_resource(resource)
                .map_err(|error| LoadError::Parse(error.to_string()))?;
            for member in &plugin.members {
                if !self.contains_name(member) {
                    return Err(LoadError::MissingMember {
                        plugin: plugin.meta.base.name.clone(),
                        member: member.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Whether any registered entity (of any kind) has the given name.
    #[must_use]
    pub fn contains_name(&self, name: &str) -> bool {
        self.entries
            .values()
            .any(|by_name| by_name.contains_key(name))
    }

    /// Look up a resource by kind and name (no allocation).
    #[must_use]
    pub fn get(&self, kind: EntityKind, name: &str) -> Option<&Resource<Value>> {
        self.entries.get(&kind)?.get(name)
    }

    /// Number of indexed resources.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.values().map(BTreeMap::len).sum()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.values().all(BTreeMap::is_empty)
    }

    /// Iterate over all indexed resources.
    pub fn iter(&self) -> impl Iterator<Item = &Resource<Value>> {
        self.entries.values().flat_map(BTreeMap::values)
    }

    /// Iterate over the resources of a single kind.
    pub fn by_kind(&self, kind: EntityKind) -> impl Iterator<Item = &Resource<Value>> {
        self.entries
            .get(&kind)
            .into_iter()
            .flat_map(BTreeMap::values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/registry")
    }

    #[test]
    fn loads_directory_of_json5_definitions() {
        let registry = Registry::load_dir(&fixtures_dir()).expect("registry should load");
        assert_eq!(registry.len(), 2);
        assert!(registry.get(EntityKind::Tool, "search").is_some());
        assert!(
            registry
                .get(EntityKind::Agent, "market-researcher")
                .is_some()
        );
        assert!(registry.get(EntityKind::Tool, "missing").is_none());
    }

    #[test]
    fn reload_rescans_source_and_is_noop_without_source() {
        let mut registry = Registry::load_dir(&fixtures_dir()).expect("load");
        assert_eq!(registry.len(), 2);
        assert!(registry.source().is_some());
        // re-scan: same fixture dir -> same count, no error.
        registry.reload().expect("reload");
        assert_eq!(registry.len(), 2);

        // a registry without a source: reload is a no-op.
        let mut empty = Registry::new();
        assert!(empty.source().is_none());
        empty.reload().expect("noop reload");
        assert!(empty.is_empty());
    }

    #[test]
    fn reload_if_changed_detects_stamp_drift() {
        let mut registry = Registry::load_dir(&fixtures_dir()).expect("load");
        // Unchanged directory -> no reload.
        assert!(!registry.reload_if_changed().expect("scan"));
        assert_eq!(registry.len(), 2);
        // Simulate a change by dropping recorded stamps; the next poll reloads.
        registry.stamps.clear();
        assert!(registry.reload_if_changed().expect("reload"));
        assert_eq!(registry.len(), 2);
        // A registry with no source never reloads.
        let mut empty = Registry::new();
        assert!(!empty.reload_if_changed().expect("noop"));
    }

    #[test]
    fn reload_if_changed_picks_up_a_new_file_on_disk() {
        // Real filesystem: load a dir, add a definition, confirm the poll reloads it.
        let dir = std::env::temp_dir().join(format!("tdw_agent_reload_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");

        let tool = |name: &str| {
            format!(
                r#"{{ apiVersion:"tdw.finx/v1", kind:"tool",
                     metadata:{{ name:"{name}", id:"{name}", version:"0.1.0",
                       origin:{{tier:"Domain",source:"Internal"}}, adaptivity:"None", autonomous:false }},
                     spec:{{ input_schema:{{ type:"object" }} }} }}"#
            )
        };
        std::fs::write(dir.join("a.json5"), tool("a")).expect("write a");

        let mut registry = Registry::load_dir(&dir).expect("load");
        assert_eq!(registry.len(), 1);
        assert!(!registry.reload_if_changed().expect("no change yet"));

        // Add a second definition file; the file set changes, so the poll reloads.
        std::fs::write(dir.join("b.json5"), tool("b")).expect("write b");
        assert!(registry.reload_if_changed().expect("reload"));
        assert_eq!(registry.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_plugins_checks_members_resolve() {
        let plugin = load_resource(
            r#"{ apiVersion:"tdw.finx/v1", kind:"plugin",
                 metadata:{ name:"myplugin", id:"myplugin", version:"0.1.0",
                   origin:{tier:"Custom",source:"Internal"}, adaptivity:"None", autonomous:false },
                 spec:{ members:["search"], client_side:true, server_side:false } }"#,
        )
        .expect("plugin parse");
        let tool = load_resource(
            r#"{ apiVersion:"tdw.finx/v1", kind:"tool",
                 metadata:{ name:"search", id:"search", version:"0.1.0",
                   origin:{tier:"Domain",source:"Internal"}, adaptivity:"None", autonomous:false },
                 spec:{ input_schema:{ type:"object" } } }"#,
        )
        .expect("tool parse");

        // Member resolves -> ok.
        let resolved = Registry::from_resources([plugin.clone(), tool]).expect("build");
        resolved.validate_plugins().expect("members should resolve");

        // Member missing -> MissingMember.
        let missing = Registry::from_resources([plugin]).expect("build");
        assert!(matches!(
            missing.validate_plugins(),
            Err(LoadError::MissingMember { .. })
        ));
    }

    #[test]
    fn load_plugins_merges_plugin_subdirs_and_validates() {
        let root = std::env::temp_dir().join(format!("tdw_agent_plugins_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let plugin_dir = root.join("alpha");
        std::fs::create_dir_all(&plugin_dir).expect("mkdir");

        std::fs::write(
            plugin_dir.join("plugin.json5"),
            r#"{ apiVersion:"tdw.finx/v1", kind:"plugin",
                 metadata:{ name:"alpha", id:"alpha", version:"0.1.0",
                   origin:{tier:"Custom",source:"Internal"}, adaptivity:"None", autonomous:false },
                 spec:{ members:["alpha.search"], client_side:false, server_side:true } }"#,
        )
        .expect("write plugin");
        std::fs::write(
            plugin_dir.join("tool.json5"),
            r#"{ apiVersion:"tdw.finx/v1", kind:"tool",
                 metadata:{ name:"alpha.search", id:"alpha.search", version:"0.1.0",
                   origin:{tier:"Domain",source:"Internal"}, adaptivity:"None", autonomous:false },
                 spec:{ input_schema:{ type:"object" } } }"#,
        )
        .expect("write tool");

        let registry = Registry::load_plugins(&root).expect("load plugins");
        assert_eq!(registry.len(), 2); // the plugin manifest + its member tool
        assert!(registry.get(EntityKind::Plugin, "alpha").is_some());
        assert!(registry.get(EntityKind::Tool, "alpha.search").is_some());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn duplicate_kind_name_is_rejected() {
        let text = r#"{
            apiVersion: "tdw.finx/v1",
            kind: "tool",
            metadata: {
                name: "x", id: "x", version: "0.1.0",
                origin: { tier: "Domain", source: "Internal" },
                adaptivity: "None", autonomous: false,
            },
            spec: {},
        }"#;
        let mut registry = Registry::new();
        registry
            .insert(load_resource(text).expect("first parse"))
            .expect("first insert ok");
        assert!(matches!(
            registry.insert(load_resource(text).expect("second parse")),
            Err(LoadError::Duplicate { .. })
        ));
    }

    #[test]
    fn autonomous_non_agent_kind_is_rejected() {
        // A `tool` resource claiming `autonomous: true` violates the taxonomy invariant.
        let text = r#"{
            apiVersion: "tdw.finx/v1",
            kind: "tool",
            metadata: {
                name: "x", id: "x", version: "0.1.0",
                origin: { tier: "Domain", source: "Internal" },
                adaptivity: "None", autonomous: true,
            },
            spec: { inputSchema: { type: "object" } },
        }"#;
        let mut registry = Registry::new();
        assert!(matches!(
            registry.insert(load_resource(text).expect("parse")),
            Err(LoadError::Invariant {
                kind: EntityKind::Tool,
                ..
            })
        ));
    }

    #[test]
    fn merge_rejects_duplicate_kind_name_across_registries() {
        // `merge` (the cross-plugin combine path) had no direct test; the
        // (kind, name) collision was only exercised at the `insert` level.
        let tool = |id: &str| {
            load_resource(&format!(
                r#"{{ apiVersion:"tdw.finx/v1", kind:"tool",
                     metadata:{{ name:"dup", id:"{id}", version:"0.1.0",
                       origin:{{tier:"Domain",source:"Internal"}}, adaptivity:"None", autonomous:false }},
                     spec:{{ input_schema:{{ type:"object" }} }} }}"#
            ))
            .expect("tool parse")
        };
        let mut first = Registry::from_resources([tool("a")]).expect("first registry");
        let second = Registry::from_resources([tool("b")]).expect("second registry");
        // Both hold a tool named "dup": merging collides on (Tool, "dup").
        assert!(matches!(
            first.merge(second),
            Err(LoadError::Duplicate { .. })
        ));
    }

    #[test]
    fn load_plugins_reports_missing_member() {
        // The end-to-end MissingMember failure through load_plugins (not just
        // via from_resources/validate_plugins) was untested.
        let root =
            std::env::temp_dir().join(format!("tdw_agent_plugins_missing_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let plugin_dir = root.join("beta");
        std::fs::create_dir_all(&plugin_dir).expect("mkdir");
        // The plugin references a member no resource provides.
        std::fs::write(
            plugin_dir.join("plugin.json5"),
            r#"{ apiVersion:"tdw.finx/v1", kind:"plugin",
                 metadata:{ name:"beta", id:"beta", version:"0.1.0",
                   origin:{tier:"Custom",source:"Internal"}, adaptivity:"None", autonomous:false },
                 spec:{ members:["beta.absent"], client_side:false, server_side:true } }"#,
        )
        .expect("write plugin");

        let result = Registry::load_plugins(&root);
        assert!(matches!(result, Err(LoadError::MissingMember { .. })));

        let _ = std::fs::remove_dir_all(&root);
    }
}
