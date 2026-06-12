//! Catalog-driven clap command tree (WS4 / gap-matrix item **L5.3**).
//!
//! The CLI's per-route command surface is generated at runtime from the
//! [`tdw_endpoint_catalog`] table — the single source of truth every other layer
//! (REST, `OpenAPI`, MCP) also derives from — so a new catalog route is callable
//! from the CLI without touching this crate. Each route's slash segments become a
//! nested subcommand path (`equity/price/historical` -> `equity price
//! historical`), and each route's `params_schema` properties become leaf
//! `--flag` args.
//!
//! The mapping is intentionally narrow and forward-compatible:
//! * every leaf takes a free-form `--symbol` (the universal instrument key the
//!   provider fetchers read out of the params object; it is not in the shared
//!   `StandardParams` schema, so it is added unconditionally on `Fetch` routes),
//! * `Fetch` routes additionally take `--provider` to pin a candidate,
//! * each `params_schema` property becomes a `--<name>` value arg (string,
//!   number, or bool by its JSON-schema `type`), and
//! * every leaf takes the shared output flags (`--json`, `--export`, `--addr`)
//!   plus `--chart`, which is passed straight through into the params object as
//!   `"chart": true`. The chart envelope slot is G014 (in-flight); if today's
//!   dispatcher ignores the key that is harmless and forward-compatible.

use clap::{Arg, ArgAction, Command};
use tdw_endpoint_catalog::{CatalogEntry, EndpointKind, catalog};

/// Schema-derived argument kinds the tree understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind {
    /// A string value flag (also the fallback for enums / `$ref` / unknown).
    Str,
    /// A numeric value flag.
    Num,
    /// A boolean flag (`true` when present).
    Bool,
}

/// One schema-derived parameter to expose as a `--<name>` flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamArg {
    /// Property name (becomes the long flag and the params-object key).
    pub name: String,
    /// JSON kind, deciding the clap arg shape and the JSON value coercion.
    pub kind: ParamKind,
}

/// Extract the `--flag` params for a route from its `params_schema`.
///
/// Reads the schema's top-level `properties` map and classifies each property by
/// its declared JSON `type` (an array type such as `["string","null"]` for an
/// `Option<T>` is reduced to its first non-null member). Enum / `$ref` / `anyOf`
/// shapes (e.g. `Interval`, `Period`) carry no scalar `type`, so they map to
/// [`ParamKind::Str`] — the daemon parses the token. JSON object key order is
/// not guaranteed, so the list is sorted by name for a deterministic,
/// snapshot-stable command surface.
#[must_use]
pub fn params_for(entry: &CatalogEntry) -> Vec<ParamArg> {
    let schema = (entry.params_schema)();
    let Ok(value) = serde_json::to_value(&schema) else {
        return Vec::new();
    };
    let Some(props) = value.get("properties").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let mut args: Vec<ParamArg> = props
        .iter()
        .map(|(name, spec)| ParamArg {
            name: name.clone(),
            kind: classify(spec),
        })
        .collect();
    args.sort_by(|a, b| a.name.cmp(&b.name));
    args
}

/// Classify one property spec into a [`ParamKind`].
fn classify(spec: &serde_json::Value) -> ParamKind {
    match spec.get("type") {
        Some(serde_json::Value::String(t)) => kind_from_token(t),
        Some(serde_json::Value::Array(types)) => types
            .iter()
            .filter_map(serde_json::Value::as_str)
            .find(|t| *t != "null")
            .map_or(ParamKind::Str, kind_from_token),
        _ => ParamKind::Str,
    }
}

fn kind_from_token(token: &str) -> ParamKind {
    match token {
        "integer" | "number" => ParamKind::Num,
        "boolean" => ParamKind::Bool,
        _ => ParamKind::Str,
    }
}

/// Build the full root [`Command`]: the catalog-derived route namespaces plus
/// the static `routes` (catalog listing) and `routine` (record/run/list)
/// subcommands.
///
/// The legacy entrypoints (`run-query`, `--smoke`, `kg reindex`, the default
/// shutdown probe) are dispatched ahead of clap in `main` and are intentionally
/// *not* modeled here, so they keep working byte-for-byte unchanged.
#[must_use]
pub fn build_root() -> Command {
    let mut root = Command::new("tdw")
        .about("Catalog-driven TDW data-warehouse CLI (OpenBB-parity command tree)")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("routes").about("List every catalog route with its kind and providers"),
        )
        .subcommand(routine_command());
    for cmd in route_commands() {
        root = root.subcommand(cmd);
    }
    root
}

/// The static `routine record|run|list` subcommand group.
fn routine_command() -> Command {
    Command::new("routine")
        .about("Record, list, and replay event-spine routines (.tdw/routines/<name>.jsonl)")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("record")
                .about("Run a CLI command and append its OpEnvelope to a routine file")
                .arg(Arg::new("name").required(true).help("Routine name"))
                .arg(
                    Arg::new("command")
                        .help("The route subcommand to record (after `--`)")
                        .num_args(0..)
                        .last(true)
                        .allow_hyphen_values(true),
                ),
        )
        .subcommand(
            Command::new("run")
                .about("Re-submit every recorded envelope in a routine")
                .arg(Arg::new("name").required(true).help("Routine name"))
                .arg(
                    Arg::new("var")
                        .long("var")
                        .help("Substitution k=v applied as ${k} across params (repeatable)")
                        .action(ArgAction::Append)
                        .num_args(1),
                )
                .arg(
                    Arg::new("addr")
                        .long("addr")
                        .help("Daemon address")
                        .default_value("127.0.0.1:7878")
                        .num_args(1),
                ),
        )
        .subcommand(Command::new("list").about("List recorded routine names"))
}

/// An intermediate prefix-tree node, built before conversion to clap commands.
///
/// Each node is either a namespace (has `children`, ordered by first-seen) or a
/// leaf (carries the catalog `entry` it dispatches). Building this plain tree
/// first — instead of mutating `clap::Command`s in place, which has no
/// remove-child operation — keeps prefix merging correct and simple: a shared
/// prefix like `equity/price` is created once and reused.
#[derive(Default)]
struct Node {
    /// Child segment name -> child node, in first-seen insertion order.
    children: Vec<(&'static str, Self)>,
    /// The catalog entry, when this node is a route leaf.
    entry: Option<CatalogEntry>,
}

impl Node {
    /// Find or create the child named `segment`, returning a mutable reference.
    fn child(&mut self, segment: &'static str) -> &mut Self {
        if let Some(idx) = self.children.iter().position(|(name, _)| *name == segment) {
            return &mut self.children[idx].1;
        }
        self.children.push((segment, Self::default()));
        let last = self.children.len() - 1;
        &mut self.children[last].1
    }

    /// Convert this namespace node's children into clap [`Command`]s.
    fn to_commands(&self) -> Vec<Command> {
        self.children
            .iter()
            .map(|(name, node)| node.to_command(name))
            .collect()
    }

    /// Convert one node into a clap [`Command`] named `name`.
    ///
    /// Three shapes are possible because a catalog route can be both a leaf and a
    /// prefix (e.g. `economy/money_measures` is a route, and
    /// `economy/money_measures/m1` and `/m2` are also routes):
    /// * pure leaf — the route command with its schema args;
    /// * pure namespace — a `subcommand_required` parent; and
    /// * hybrid — the route command *plus* nested subcommands, with the
    ///   subcommand made optional so the parent route is still directly callable.
    fn to_command(&self, name: &'static str) -> Command {
        match (&self.entry, self.children.is_empty()) {
            (Some(entry), true) => build_leaf(entry, name),
            (Some(entry), false) => {
                let mut cmd = build_leaf(entry, name);
                for child in self.to_commands() {
                    cmd = cmd.subcommand(child);
                }
                cmd
            }
            (None, _) => {
                let mut cmd = Command::new(name)
                    .subcommand_required(true)
                    .arg_required_else_help(true);
                for child in self.to_commands() {
                    cmd = cmd.subcommand(child);
                }
                cmd
            }
        }
    }
}

/// Build the per-route portion of the clap command tree from the live catalog.
///
/// Returns the list of top-level namespace [`Command`]s (e.g. `equity`,
/// `crypto`, `technical`). The caller grafts these onto the root command
/// alongside the static `routes` / `routine` subcommands. Routes are inserted in
/// catalog order; shared prefixes are merged so each segment is created once.
#[must_use]
pub fn route_commands() -> Vec<Command> {
    let mut root = Node::default();
    for entry in catalog() {
        // `entry.route` is `&'static str`, so each split segment is `&'static
        // str` — exactly what clap's `Command::new` / `Arg::long` require.
        let segments: Vec<&'static str> = entry.route.split('/').collect();
        let mut cursor = &mut root;
        for segment in &segments {
            cursor = cursor.child(segment);
        }
        cursor.entry = Some(entry);
    }
    root.to_commands()
}

/// Build the executable leaf command for one route, with the schema-derived
/// flags plus the shared `--symbol` / `--provider` / output flags.
fn build_leaf(entry: &CatalogEntry, name: &'static str) -> Command {
    let mut cmd = Command::new(name).about(entry.doc);
    cmd = cmd.arg(
        Arg::new("symbol")
            .long("symbol")
            .help("Instrument symbol / identifier passed in params")
            .num_args(1),
    );
    if entry.kind == EndpointKind::Fetch {
        cmd = cmd.arg(
            Arg::new("provider")
                .long("provider")
                .help("Pin a specific provider candidate (default: catalog fallback order)")
                .num_args(1),
        );
    }
    for param in params_for(entry) {
        cmd = cmd.arg(param_to_arg(&param));
    }
    cmd.arg(
        Arg::new("chart")
            .long("chart")
            .help("Request a chart spec (forwarded as params.chart=true; slot is G014)")
            .action(ArgAction::SetTrue),
    )
    .arg(
        Arg::new("json")
            .long("json")
            .help("Print the raw result envelope as JSON instead of a table")
            .action(ArgAction::SetTrue),
    )
    .arg(
        Arg::new("export")
            .long("export")
            .help("Write rows to a file in this format: csv | json | xlsx")
            .value_parser(["csv", "json", "xlsx"])
            .num_args(1),
    )
    .arg(
        Arg::new("out")
            .long("out")
            .help("Export output path (default: <route>.<ext>)")
            .num_args(1),
    )
    .arg(
        Arg::new("addr")
            .long("addr")
            .help("Daemon address")
            .default_value("127.0.0.1:7878")
            .num_args(1),
    )
}

/// Turn a schema-derived [`ParamArg`] into a clap [`Arg`].
///
/// clap's `Arg::new` / `Arg::long` want a `'static` string for the id and flag.
/// Param names come from the route's params schema (owned `String`s), so they are
/// leaked into `'static` storage. This is bounded: the set of schema property
/// names is small, fixed, and built once per process, so the leak is effectively
/// a one-time interning of the static command surface — not unbounded growth.
fn param_to_arg(param: &ParamArg) -> Arg {
    let name: &'static str = Box::leak(param.name.clone().into_boxed_str());
    let arg = Arg::new(name).long(name);
    match param.kind {
        ParamKind::Bool => arg.action(ArgAction::SetTrue),
        ParamKind::Num | ParamKind::Str => arg.num_args(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_for(route: &str) -> CatalogEntry {
        tdw_endpoint_catalog::lookup(route).expect("seeded route present")
    }

    #[test]
    fn standard_params_map_to_expected_kinds() {
        let params = params_for(&entry_for("equity/price/historical"));
        let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
        // StandardParams: start_date, end_date, interval, period, limit.
        assert!(names.contains(&"start_date"), "got {names:?}");
        assert!(names.contains(&"limit"), "got {names:?}");
        let limit = params
            .iter()
            .find(|p| p.name == "limit")
            .expect("limit param");
        assert_eq!(limit.kind, ParamKind::Num);
        // `interval` is an enum (no scalar JSON type) -> Str.
        let interval = params
            .iter()
            .find(|p| p.name == "interval")
            .expect("interval param");
        assert_eq!(interval.kind, ParamKind::Str);
    }

    #[test]
    fn params_are_sorted_for_determinism() {
        let params = params_for(&entry_for("equity/price/historical"));
        let mut sorted = params.clone();
        sorted.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(params, sorted);
    }

    #[test]
    fn route_segments_become_nested_subcommand_path() {
        let roots = route_commands();
        let equity = roots
            .iter()
            .find(|c| c.get_name() == "equity")
            .expect("equity namespace present");
        let price = equity
            .get_subcommands()
            .find(|c| c.get_name() == "price")
            .expect("equity price present");
        let historical = price
            .get_subcommands()
            .find(|c| c.get_name() == "historical")
            .expect("equity price historical present");
        // Leaf carries the shared flags.
        let longs: Vec<&str> = historical
            .get_arguments()
            .filter_map(clap::Arg::get_long)
            .collect();
        assert!(longs.contains(&"symbol"), "got {longs:?}");
        assert!(longs.contains(&"provider"), "got {longs:?}");
        assert!(longs.contains(&"json"), "got {longs:?}");
        assert!(longs.contains(&"export"), "got {longs:?}");
    }

    #[test]
    fn shared_price_prefix_is_merged_not_duplicated() {
        let roots = route_commands();
        let equity = roots
            .iter()
            .find(|c| c.get_name() == "equity")
            .expect("equity present");
        let price_children = equity
            .get_subcommands()
            .filter(|c| c.get_name() == "price")
            .count();
        assert_eq!(price_children, 1, "equity/price must appear once");
        let price = equity
            .get_subcommands()
            .find(|c| c.get_name() == "price")
            .expect("price present");
        // equity/price/{historical,quote,performance} all live under one `price`.
        for leaf in ["historical", "quote", "performance"] {
            assert!(
                price.get_subcommands().any(|c| c.get_name() == leaf),
                "missing equity price {leaf}"
            );
        }
    }

    #[test]
    fn compute_routes_have_no_provider_flag() {
        let roots = route_commands();
        let technical = roots
            .iter()
            .find(|c| c.get_name() == "technical")
            .expect("technical namespace present");
        let sma = technical
            .get_subcommands()
            .find(|c| c.get_name() == "sma")
            .expect("technical sma present");
        let longs: Vec<&str> = sma
            .get_arguments()
            .filter_map(clap::Arg::get_long)
            .collect();
        assert!(!longs.contains(&"provider"), "compute route got --provider");
        assert!(longs.contains(&"symbol"), "got {longs:?}");
    }

    #[test]
    fn every_catalog_route_is_reachable_in_the_tree() {
        let roots = route_commands();
        for entry in catalog() {
            let segments: Vec<&str> = entry.route.split('/').collect();
            let mut cursor = roots
                .iter()
                .find(|c| c.get_name() == segments[0])
                .unwrap_or_else(|| panic!("missing root for {}", entry.route));
            for segment in &segments[1..] {
                cursor = cursor
                    .get_subcommands()
                    .find(|c| c.get_name() == *segment)
                    .unwrap_or_else(|| panic!("missing segment {segment} for {}", entry.route));
            }
        }
    }
}
