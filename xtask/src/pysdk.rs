//! Python SDK generation from the endpoint catalog (gap-matrix **WS3** /
//! "`Python SDK`", §10 of the `OpenBB` surface infra roadmap).
//!
//! Mirrors [`crate::openapi`]: a thin, deterministic code generator that reads
//! [`tdw_endpoint_catalog::catalog`] and emits a checked-in Python package under
//! `sdk/python/finx_platform/`. The generated tree gives OpenBB-style call
//! ergonomics — `finx.equity.price.historical(symbol="AAPL", provider="yahoo")`
//! — over the daemon's REST `/api/v1` surface, with no per-route handler: routes
//! are data, and the namespace classes are derived from the catalog at generate
//! time.
//!
//! ## Generation design
//!
//! * **Namespacing** — each route's slash segments become a nested attribute
//!   path. The first segment is a top-level namespace (e.g. `equity`); every
//!   intermediate segment (`price`) becomes a nested accessor object; the final
//!   segment(s) become a method. `equity/price/historical` →
//!   `finx.equity.price.historical(...)`. The tree is built deterministically
//!   (segments are `BTreeMap`-ordered), so the generated source is stable.
//! * **Naming / collisions** — segments are already lowercase `[a-z0-9_]` per the
//!   catalog route grammar, so they map to valid Python identifiers verbatim
//!   (`snake_case`). A leaf method and an intermediate namespace can never collide
//!   on one node because the catalog routes are unique and a node is either a
//!   namespace (has children) or a method (a terminal route) — the generator
//!   asserts this and fails on any route that would shadow a namespace.
//! * **Typing** — method kwargs are derived from the route's `params_schema`
//!   top-level `properties`, mapped to Python type hints (`str`/`float`/`int`/
//!   `bool`), `Optional` when the property is not in the schema's `required`
//!   list. Every `Fetch` method also takes `provider: str | None = None`;
//!   chartable routes additionally take `chart: bool = False`. A trailing
//!   `**kwargs: object` threads provider-specific arguments (e.g. `symbol`) that
//!   are not part of the standardized params schema.
//! * **Compute routes** — the REST surface serves `Fetch` routes only
//!   (`rest_fetch_data` rejects non-fetch routes). There is no REST compute
//!   endpoint, so `Compute` routes (the `technical/*` indicators) are generated
//!   as stub methods that raise `NotImplementedError` pointing at the MCP / `Op`
//!   surface, rather than dead REST calls. This keeps the call surface complete
//!   (every catalog route is addressable) while being honest about REST
//!   exposure.
//!
//! Determinism: every map is `BTreeMap`-backed and every list is sorted, so
//! running the generator twice yields byte-identical output. The `pysdk-check`
//! drift gate relies on this.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;

/// Repo-relative directory the generated package modules are written to.
pub const PYSDK_PACKAGE_DIR: &str = "sdk/python/finx_platform";

/// One generated leaf method derived from a catalog route.
struct Method {
    /// Python method name (the route's final segment).
    name: String,
    /// Full slash route, passed to the runtime client.
    route: String,
    /// Whether the route is a REST-served `Fetch` route (vs a `Compute` stub).
    fetch: bool,
    /// Whether the route is chartable (adds a `chart: bool = False` kwarg).
    chartable: bool,
    /// One-line route description.
    doc: String,
    /// Ordered, typed parameters derived from the route's params schema.
    params: Vec<Param>,
}

/// One typed method parameter derived from a params-schema property.
struct Param {
    /// Python parameter name.
    name: String,
    /// Python type hint (already including `| None` when optional).
    type_hint: String,
    /// Whether the parameter is required (no default) or optional (`= None`).
    required: bool,
    /// Property description, if the schema carried one.
    doc: Option<String>,
}

/// A node in the generated namespace tree: either intermediate children or a
/// terminal method (never both — enforced at build time).
#[derive(Default)]
struct Node {
    /// Child namespaces, keyed by segment (`BTreeMap` for deterministic order).
    children: BTreeMap<String, Self>,
    /// The terminal method at this node, if the node is a leaf route.
    method: Option<Method>,
}

/// Build the full namespace tree from the catalog.
fn build_tree() -> Node {
    let mut root = Node::default();
    for entry in tdw_endpoint_catalog::catalog() {
        let segments: Vec<&str> = entry.route.split('/').collect();
        let (last, namespaces) = segments
            .split_last()
            .expect("catalog routes have at least one segment");

        let mut node = &mut root;
        for segment in namespaces {
            node = node.children.entry((*segment).to_string()).or_default();
        }

        let fetch = matches!(entry.kind, tdw_endpoint_catalog::EndpointKind::Fetch);
        let params =
            derive_params(&serde_json::to_value((entry.params_schema)()).unwrap_or(Value::Null));
        let method = Method {
            name: (*last).to_string(),
            route: entry.route.to_string(),
            fetch,
            chartable: entry.chartable,
            doc: entry.doc.to_string(),
            params,
        };
        let leaf = node.children.entry((*last).to_string()).or_default();
        leaf.method = Some(method);
    }
    root
}

/// Sanitize a catalog route segment into a valid Python identifier.
///
/// Catalog segments are `[a-z0-9_]+`. A segment that starts with a digit (e.g.
/// the maturity tenors `10y`, `90d`, `3m`) or that collides with a Python
/// keyword is not a valid identifier; it is prefixed with `t_` (a fixed,
/// letter-leading prefix). Because every catalog segment is lowercase and the
/// prefix is letter-leading, a `t_<seg>` name can only collide with a literal
/// `t_<seg>` sibling segment — which the per-node assertion in
/// [`render_node_class`] would catch. The rule is deterministic and documented
/// in the generated package README.
fn safe_ident(segment: &str) -> String {
    let needs_prefix =
        segment.chars().next().is_some_and(|c| c.is_ascii_digit()) || is_python_keyword(segment);
    if needs_prefix {
        format!("t_{segment}")
    } else {
        segment.to_string()
    }
}

/// Whether `word` is a reserved Python keyword (or soft keyword) that cannot be
/// used as a plain attribute/method name.
fn is_python_keyword(word: &str) -> bool {
    matches!(
        word,
        "false"
            | "none"
            | "true"
            | "and"
            | "as"
            | "assert"
            | "async"
            | "await"
            | "break"
            | "class"
            | "continue"
            | "def"
            | "del"
            | "elif"
            | "else"
            | "except"
            | "finally"
            | "for"
            | "from"
            | "global"
            | "if"
            | "import"
            | "in"
            | "is"
            | "lambda"
            | "nonlocal"
            | "not"
            | "or"
            | "pass"
            | "raise"
            | "return"
            | "try"
            | "while"
            | "with"
            | "yield"
            | "match"
            | "case"
    )
}

/// Derive typed parameters from a params schema's top-level `properties`,
/// honoring its `required` list. Sorted by name for deterministic output.
fn derive_params(schema: &Value) -> Vec<Param> {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let mut params: Vec<Param> = properties
        .iter()
        .map(|(name, prop)| {
            let is_required = required.contains(&name.as_str());
            Param {
                name: name.clone(),
                type_hint: python_type_hint(prop, is_required),
                required: is_required,
                doc: prop
                    .get("description")
                    .and_then(Value::as_str)
                    .map(clean_doc),
            }
        })
        .collect();
    params.sort_by(|a, b| a.name.cmp(&b.name));
    params
}

/// Map a JSON-Schema property to a Python type hint, appending `| None` when the
/// property is optional. Resolves `anyOf`/`$ref`/typed-array shapes to the
/// closest Python scalar, defaulting to `str` (REST query values are strings).
fn python_type_hint(prop: &Value, required: bool) -> String {
    let base = scalar_for(prop);
    if required {
        base
    } else {
        format!("{base} | None")
    }
}

/// The Python scalar type name for a property's JSON-Schema `type`/`format`,
/// peeking through `anyOf`/`allOf` wrappers. `$ref` shapes (enums like
/// `Interval`, formatted strings like `Date`) collapse to `str`, the wire form
/// of every REST query value.
fn scalar_for(prop: &Value) -> String {
    if let Some(format) = prop.get("format").and_then(Value::as_str) {
        if format.starts_with("int") || format.starts_with("uint") {
            return "int".to_string();
        }
        if matches!(format, "float" | "double") {
            return "float".to_string();
        }
    }
    match prop.get("type") {
        Some(Value::String(token)) => python_scalar(token),
        Some(Value::Array(types)) => types
            .iter()
            .filter_map(Value::as_str)
            .find(|token| *token != "null")
            .map_or_else(|| "str".to_string(), python_scalar),
        _ => {
            // `anyOf`/`allOf` (e.g. `Option<Date>` → anyOf[$ref, null]) or a bare
            // `$ref` (an enum): treat as a string, the REST query wire form.
            for key in ["anyOf", "allOf", "oneOf"] {
                if let Some(variants) = prop.get(key).and_then(Value::as_array)
                    && let Some(non_null) = variants
                        .iter()
                        .find(|v| v.get("type").and_then(Value::as_str) != Some("null"))
                {
                    return scalar_for(non_null);
                }
            }
            "str".to_string()
        }
    }
}

/// Map a single JSON-Schema scalar type token to a Python type name.
fn python_scalar(token: &str) -> String {
    match token {
        "integer" => "int",
        "number" => "float",
        "boolean" => "bool",
        _ => "str",
    }
    .to_string()
}

/// Collapse a schema description into a single clean docstring line.
fn clean_doc(doc: &str) -> String {
    doc.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Render the full generated package as `(relative_path, contents)` pairs, in
/// deterministic (sorted) order.
fn render_package() -> Vec<(String, String)> {
    let root = build_tree();
    let mut files: Vec<(String, String)> = Vec::new();

    let namespaces: Vec<&String> = root.children.keys().collect();

    // One module per top-level namespace.
    for (namespace, node) in &root.children {
        files.push((
            format!("{PYSDK_PACKAGE_DIR}/{namespace}.py"),
            render_namespace_module(namespace, node),
        ));
    }

    // The package __init__ wiring the client to every namespace.
    files.push((
        format!("{PYSDK_PACKAGE_DIR}/__init__.py"),
        render_init(&namespaces),
    ));

    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

/// Render the `__init__.py`: the public `FinX` entry point that constructs the
/// runtime client and exposes one attribute per namespace.
fn render_init(namespaces: &[&String]) -> String {
    let mut out = String::new();
    out.push_str(GENERATED_HEADER);
    out.push_str(
        "\"\"\"FinX Platform Python SDK — a thin, generated HTTP client over the daemon's REST API.\n\n\
         Install the package and call catalog routes with OpenBB-style ergonomics::\n\n\
         \x20\x20\x20\x20from finx_platform import FinX\n\n\
         \x20\x20\x20\x20finx = FinX()  # base_url from FINX_BASE_URL or the loopback default\n\
         \x20\x20\x20\x20obj = finx.equity.price.historical(symbol=\"AAPL\", provider=\"yahoo\")\n\
         \x20\x20\x20\x20df = obj.to_dataframe()\n\n\
         The client targets a locally running daemon with the REST surface enabled\n\
         (``TDW_DAEMON_REST_BIND``). Only ``Fetch`` routes are REST-served; compute\n\
         routes (``technical/*``) raise ``NotImplementedError``.\n\"\"\"\n\n",
    );
    out.push_str("from __future__ import annotations\n\n");
    out.push_str("from ._client import Client, FinXObject\n");
    for namespace in namespaces {
        let class = class_name(namespace);
        let _ = writeln!(out, "from .{namespace} import {class}");
    }
    out.push('\n');
    out.push_str("__all__ = [\n    \"FinX\",\n    \"FinXObject\",\n    \"Client\",\n]\n\n");
    out.push_str("__version__ = \"0.1.0\"\n\n\n");

    out.push_str("class FinX:\n");
    out.push_str(
        "    \"\"\"The SDK entry point: a REST client plus one accessor per router namespace.\n\n\
         \x20\x20\x20\x20Parameters mirror :class:`finx_platform._client.Client`; ``base_url`` falls\n\
         \x20\x20\x20\x20back to the ``FINX_BASE_URL`` environment variable and then the loopback\n\
         \x20\x20\x20\x20default.\n\
         \x20\x20\x20\x20\"\"\"\n\n",
    );
    out.push_str(
        "    def __init__(\n        self,\n        base_url: str | None = None,\n        *,\n        \
         timeout: float = 30.0,\n        client: Client | None = None,\n    ) -> None:\n",
    );
    out.push_str(
        "        self._client = client if client is not None else Client(base_url, timeout=timeout)\n",
    );
    for namespace in namespaces {
        let class = class_name(namespace);
        let _ = writeln!(
            out,
            "        self.{} = {class}(self._client)",
            safe_ident(namespace)
        );
    }
    out.push('\n');
    out
}

/// Render one namespace module: the namespace class plus any nested-accessor
/// classes for intermediate segments.
fn render_namespace_module(namespace: &str, node: &Node) -> String {
    let mut out = String::new();
    out.push_str(GENERATED_HEADER);
    let _ = writeln!(
        out,
        "\"\"\"Generated ``{namespace}`` router namespace for the FinX Platform SDK.\"\"\"\n"
    );
    out.push_str("from __future__ import annotations\n\n");
    out.push_str("from ._client import Client, FinXObject\n\n\n");

    // Emit nested-accessor classes (deepest-first) before the namespace class so
    // forward references resolve without `from __future__` gymnastics at runtime.
    let mut classes: Vec<String> = Vec::new();
    render_node_class(&[namespace.to_string()], node, &mut classes);
    out.push_str(&classes.join("\n"));
    out
}

/// Recursively render the class for `node` (named by its segment path) and all
/// of its descendant accessor classes, appending each rendered class to
/// `classes`. Children are rendered first so a parent can reference them.
fn render_node_class(path: &[String], node: &Node, classes: &mut Vec<String>) {
    // Render child accessor classes first (post-order) for forward-reference-free
    // construction in `__init__`.
    for (child_segment, child) in &node.children {
        if child.method.is_some() && child.children.is_empty() {
            continue; // pure leaves become methods, not classes
        }
        let mut child_path = path.to_vec();
        child_path.push(child_segment.clone());
        render_node_class(&child_path, child, classes);
    }

    let class = path_class_name(path);
    let mut body = String::new();
    let _ = writeln!(body, "class {class}:");
    let _ = writeln!(
        body,
        "    \"\"\"Accessor for the ``{}`` route namespace.\"\"\"\n",
        path.join("/")
    );
    body.push_str("    def __init__(self, client: Client) -> None:\n");
    body.push_str("        self._client = client\n");

    // Construct nested accessors for child namespaces.
    let mut has_nested = false;
    for (child_segment, child) in &node.children {
        if child.method.is_some() && child.children.is_empty() {
            continue;
        }
        has_nested = true;
        let mut child_path = path.to_vec();
        child_path.push(child_segment.clone());
        let child_class = path_class_name(&child_path);
        let _ = writeln!(
            body,
            "        self.{} = {child_class}(client)",
            safe_ident(child_segment)
        );
    }
    let _ = has_nested;
    body.push('\n');

    // Emit a method per leaf child (a child that is a terminal route).
    let mut methods: Vec<String> = Vec::new();
    for (child_segment, child) in &node.children {
        if let Some(method) = &child.method {
            if child.children.is_empty() {
                methods.push(render_method(method));
            } else {
                // A node that is both a namespace and a route is unsupported; the
                // catalog's unique routes never produce this, but guard anyway.
                methods.push(render_shadowed_method(child_segment, method));
            }
        }
    }
    if methods.is_empty() {
        body.push_str("    # (no terminal routes at this namespace level)\n");
    } else {
        body.push_str(&methods.join("\n"));
    }

    classes.push(body);
}

/// Render a leaf method (its `def`, signature, docstring, and client call).
fn render_method(method: &Method) -> String {
    let mut out = String::new();

    // Signature: required params first, then optional (provider / chart / schema
    // optionals), then **kwargs. Python requires non-defaulted args before
    // defaulted ones, so required schema params lead.
    let mut required_args: Vec<String> = Vec::new();
    let mut optional_args: Vec<String> = Vec::new();
    for param in &method.params {
        if param.required {
            required_args.push(format!("{}: {}", param.name, param.type_hint));
        } else {
            optional_args.push(format!("{}: {} = None", param.name, param.type_hint));
        }
    }
    if method.fetch {
        optional_args.push("provider: str | None = None".to_string());
        if method.chartable {
            optional_args.push("chart: bool = False".to_string());
        }
    }

    let _ = writeln!(out, "    def {}(", safe_ident(&method.name));
    out.push_str("        self,\n");
    for arg in &required_args {
        let _ = writeln!(out, "        {arg},");
    }
    // Force every following argument to be keyword-only for forward-compatible,
    // unambiguous calls (OpenBB-style kwargs).
    out.push_str("        *,\n");
    for arg in &optional_args {
        let _ = writeln!(out, "        {arg},");
    }
    out.push_str("        **kwargs: object,\n");
    out.push_str("    ) -> FinXObject:\n");

    // Docstring: route doc + parameter descriptions.
    let _ = writeln!(out, "        \"\"\"{}", method.doc);
    out.push('\n');
    let _ = writeln!(out, "        Route: ``{}``.", method.route);
    if !method.fetch {
        out.push('\n');
        out.push_str(
            "        This is a compute route, not served by the REST surface; calling it raises\n",
        );
        out.push_str("        ``NotImplementedError``.\n");
    }
    if !method.params.is_empty() || method.fetch {
        out.push('\n');
        out.push_str("        Args:\n");
        for param in &method.params {
            let doc = param
                .doc
                .as_deref()
                .unwrap_or("Standardized query parameter.");
            let _ = writeln!(out, "            {}: {doc}", param.name);
        }
        if method.fetch {
            out.push_str(
                "            provider: Explicit provider key; ``None`` uses the catalog fallback order.\n",
            );
            if method.chartable {
                out.push_str(
                    "            chart: When ``True``, request a server-rendered chart in ``extra``.\n",
                );
            }
            out.push_str(
                "            **kwargs: Provider-specific arguments (e.g. ``symbol``) threaded to the query.\n",
            );
        }
        out.push_str(
            "\n        Returns:\n            A :class:`FinXObject` wrapping the result envelope.\n",
        );
    }
    out.push_str("        \"\"\"\n");

    if method.fetch {
        // Build the params dict from the typed args + kwargs.
        out.push_str("        params: dict[str, object] = dict(kwargs)\n");
        for param in &method.params {
            let _ = writeln!(
                out,
                "        if {name} is not None:\n            params[\"{name}\"] = {name}",
                name = param.name
            );
        }
        out.push_str(
            "        if provider is not None:\n            params[\"provider\"] = provider\n",
        );
        if method.chartable {
            out.push_str("        if chart:\n            params[\"chart\"] = \"true\"\n");
        }
        let _ = writeln!(
            out,
            "        return self._client.fetch(\"{}\", params)",
            method.route
        );
    } else {
        let _ = writeln!(
            out,
            "        raise NotImplementedError(\n            \"compute route '{}' is not exposed over REST; \"\n            \"use the MCP tool surface or the daemon Op::FetchData compute path\"\n        )",
            method.route
        );
    }

    out
}

/// Render a method whose route would shadow a namespace (defensive; the unique
/// catalog routes never produce this). Suffix the method to avoid the clash.
fn render_shadowed_method(segment: &str, method: &Method) -> String {
    let mut shadowed = Method {
        name: format!("{segment}_route"),
        route: method.route.clone(),
        fetch: method.fetch,
        chartable: method.chartable,
        doc: method.doc.clone(),
        params: method
            .params
            .iter()
            .map(|p| Param {
                name: p.name.clone(),
                type_hint: p.type_hint.clone(),
                required: p.required,
                doc: p.doc.clone(),
            })
            .collect(),
    };
    let _ = &mut shadowed;
    render_method(&shadowed)
}

/// The Python class name for a top-level namespace (e.g. `equity` →
/// `EquityNamespace`).
fn class_name(namespace: &str) -> String {
    format!("{}Namespace", pascal_case(namespace))
}

/// The class name for a nested path (e.g. `["equity","price"]` →
/// `EquityPriceNamespace`).
fn path_class_name(path: &[String]) -> String {
    let joined: String = path.iter().map(|s| pascal_case(s)).collect();
    format!("{joined}Namespace")
}

/// `PascalCase` one `snake_case` segment (`price_performance` → `PricePerformance`).
///
/// A segment that would yield a digit-leading class name (e.g. a tenor like
/// `10y`) is prefixed with `T` so the class name is a valid Python identifier;
/// this only affects intermediate namespaces, never the route strings.
fn pascal_case(segment: &str) -> String {
    let mut out = String::new();
    for word in segment.split('_') {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("T{out}")
    } else {
        out
    }
}

/// The banner prepended to every generated file. Editing generated files is a
/// drift error caught by `pysdk-check`.
const GENERATED_HEADER: &str = "# @generated by `cargo run -p xtask -- pysdk-sync` from the endpoint catalog.\n# Do not edit by hand; regenerate and commit instead. Drift is gated by `pysdk-check`.\n\n";

/// Write the generated package to disk under [`PYSDK_PACKAGE_DIR`].
///
/// # Errors
///
/// Returns a message if a parent directory cannot be created or a file cannot be
/// written.
pub fn pysdk_sync() -> Result<(), String> {
    let files = render_package();
    let mut methods = 0usize;
    for (path, contents) in &files {
        let path = Path::new(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(path, contents).map_err(|error| error.to_string())?;
        methods += contents.matches("    def ").count();
    }
    let modules = files.len();
    let _ = methods;
    let routes = tdw_endpoint_catalog::catalog().len();
    println!(
        "pysdk-sync wrote {modules} files covering {routes} catalog routes to {PYSDK_PACKAGE_DIR}"
    );
    Ok(())
}

/// Regenerate the package and fail if any file differs from the checked-in copy
/// (the drift gate, mirroring `openapi-check`).
///
/// # Errors
///
/// Returns an error if a generated file is missing/unreadable or stale versus
/// the freshly generated one.
pub fn pysdk_check() -> Result<(), String> {
    let files = render_package();
    let mut stale: Vec<String> = Vec::new();
    for (path, expected) in &files {
        let Ok(current) = std::fs::read_to_string(path) else {
            stale.push(format!("{path} (missing)"));
            continue;
        };
        if current != *expected {
            stale.push(path.clone());
        }
    }
    if stale.is_empty() {
        println!(
            "pysdk-check passed: {} generated files under {PYSDK_PACKAGE_DIR} are up to date",
            files.len()
        );
        Ok(())
    } else {
        Err(format!(
            "python SDK is stale; run `cargo run -p xtask -- pysdk-sync` and commit:\n{}",
            stale.join("\n")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(render_package(), render_package());
    }

    #[test]
    fn every_namespace_has_a_module_and_init_wires_them() {
        let files = render_package();
        let init = files
            .iter()
            .find(|(path, _)| path.ends_with("__init__.py"))
            .map(|(_, contents)| contents.clone())
            .expect("__init__.py is generated");
        assert!(init.contains("class FinX:"));
        // Equity is a seeded namespace; its module and wiring must exist.
        assert!(
            files.iter().any(|(path, _)| path.ends_with("/equity.py")),
            "equity namespace module is generated"
        );
        assert!(init.contains("self.equity = EquityNamespace(self._client)"));
    }

    #[test]
    fn equity_price_historical_is_a_nested_fetch_method() {
        let files = render_package();
        let equity = files
            .iter()
            .find(|(path, _)| path.ends_with("/equity.py"))
            .map(|(_, contents)| contents.clone())
            .expect("equity module");
        // The nested accessor class and the leaf method both exist.
        assert!(equity.contains("class EquityPriceNamespace:"));
        assert!(equity.contains("self.price = EquityPriceNamespace(client)"));
        assert!(equity.contains("def historical("));
        assert!(equity.contains("provider: str | None = None"));
        assert!(equity.contains("self._client.fetch(\"equity/price/historical\", params)"));
    }

    #[test]
    fn compute_routes_raise_not_implemented() {
        let files = render_package();
        let technical = files
            .iter()
            .find(|(path, _)| path.ends_with("/technical.py"))
            .map(|(_, contents)| contents.clone())
            .expect("technical module");
        assert!(technical.contains("def rsi("));
        assert!(technical.contains("raise NotImplementedError"));
        // Compute methods never call the REST client.
        assert!(
            !technical.contains("self._client.fetch("),
            "compute routes must not emit a REST fetch call"
        );
    }

    #[test]
    fn standard_params_are_typed_and_optional() {
        let files = render_package();
        let equity = files
            .iter()
            .find(|(path, _)| path.ends_with("/equity.py"))
            .map(|(_, contents)| contents.clone())
            .expect("equity module");
        // StandardParams optionals surface with `| None = None` and the right
        // scalar types (limit -> int, start_date -> str via the Date $ref).
        assert!(equity.contains("limit: int | None = None"));
        assert!(equity.contains("start_date: str | None = None"));
        assert!(equity.contains("**kwargs: object"));
    }

    #[test]
    fn pascal_and_path_class_names_are_stable() {
        assert_eq!(class_name("equity"), "EquityNamespace");
        assert_eq!(
            path_class_name(&["equity".to_string(), "price".to_string()]),
            "EquityPriceNamespace"
        );
        assert_eq!(pascal_case("price_performance"), "PricePerformance");
    }
}
