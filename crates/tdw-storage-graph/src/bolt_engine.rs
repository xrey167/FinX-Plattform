//! Real graph-database backend for [`GraphEngine`] over the Bolt protocol
//! (knowledge-system A4).
//!
//! Memgraph is the primary server; because the engine speaks plain Bolt +
//! Cypher through `neo4rs`, a Neo4j server works with the same code — the
//! cross-backend conformance suite keeps both honest against the in-memory
//! reference engine.
//!
//! # Storage model
//!
//! * Every node carries the single label `E`; every relationship the single
//!   type `R` with the logical relationship name in the `rel` property.
//!   Cypher cannot parameterize labels/relationship types, so keeping them
//!   fixed means NO caller string is ever interpolated into query text —
//!   every caller value travels as a bind parameter (the injection posture
//!   the A2 security review required ahead of this slice).
//! * `props` and `provenance` are stored as JSON strings (Bolt properties
//!   cannot hold maps) and round-trip through serde.
//! * Open validity bounds are stored as the empty-string sentinel `""` (Bolt
//!   `MERGE` keys cannot be null); `""` sorts before every RFC 3339 timestamp,
//!   so the half-open `[valid_from, valid_to)` comparisons stay lexicographic.
//! * `expand`/`shortest_path` run the SAME client-side BFS as the in-memory
//!   engine over the `neighbors` Cypher primitive — one round-trip per hop.
//!   That guarantees behavioral parity (the property the conformance suite
//!   pins); per-hop latency optimization is deliberately deferred.

use std::collections::{BTreeSet, VecDeque};

use async_trait::async_trait;
use neo4rs::{Graph, Row, query};
use tdw_core::{
    Direction, Error, GraphEdge, GraphEngine, GraphNode, MergeDecision, MergeReport, Provenance,
    Result, Subgraph, TraversalFilter, validate_graph_edge, validate_graph_node,
    validate_traversal_filter,
};

/// Open-bound sentinel: Bolt `MERGE` keys cannot be null, and `""` sorts
/// before every RFC 3339 timestamp.
const OPEN_BOUND: &str = "";

/// Bolt/Cypher [`GraphEngine`] backend (Memgraph primary, Neo4j-compatible).
pub struct BoltGraphEngine {
    graph: Graph,
}

impl std::fmt::Debug for BoltGraphEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BoltGraphEngine")
            .finish_non_exhaustive()
    }
}

impl BoltGraphEngine {
    /// Connect to a Bolt endpoint (e.g. `bolt://127.0.0.1:7687`). Memgraph
    /// without auth accepts empty credentials.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Storage`] if the connection cannot be established.
    pub async fn connect(uri: &str, user: &str, password: &str) -> Result<Self> {
        let graph = Graph::new(uri, user, password)
            .await
            .map_err(|error| Error::Storage(format!("bolt connect: {error}")))?;
        Ok(Self { graph })
    }

    /// Delete every `E` node and its relationships. Test-support only: the
    /// conformance suite needs a clean graph per run against a persistent
    /// server. Deliberately not part of [`GraphEngine`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Storage`] if the delete fails.
    pub async fn wipe_for_tests(&self) -> Result<()> {
        self.graph
            .run(query("MATCH (n:E) DETACH DELETE n"))
            .await
            .map_err(|error| Error::Storage(format!("bolt wipe: {error}")))
    }

    /// One filtered hop via Cypher. `ORDER BY r.rel, n.id, r.vf` is part of
    /// the [`GraphEngine`] contract (see the BOLT-A4 note in the conformance
    /// suite) — never drop it.
    async fn hop(&self, id: &str, filter: &TraversalFilter) -> Result<Vec<(GraphEdge, GraphNode)>> {
        let mut reached = Vec::new();
        let directions: &[&str] = match filter.direction {
            Direction::Out => &["out"],
            Direction::In => &["in"],
            Direction::Both => &["out", "in"],
        };
        for direction in directions {
            let pattern = if *direction == "out" {
                "MATCH (a:E {id:$id})-[r:R]->(n:E)"
            } else {
                "MATCH (a:E {id:$id})<-[r:R]-(n:E)"
            };
            let text = format!(
                "{pattern} \
                 WHERE n.id <> $id \
                 AND ($rels_all OR r.rel IN $rels) \
                 AND ($kinds_all OR n.kind IN $kinds) \
                 AND ($as_of = '' OR ((r.vf = '' OR r.vf <= $as_of) AND (r.vt = '' OR r.vt > $as_of))) \
                 AND ($as_of = '' OR ((n.vf = '' OR n.vf <= $as_of) AND (n.vt = '' OR n.vt > $as_of))) \
                 RETURN r.rel AS rel, r.props AS rprops, r.provenance AS rprov, \
                        r.vf AS rvf, r.vt AS rvt, \
                        startNode(r).id AS efrom, endNode(r).id AS eto, \
                        n.id AS nid, n.kind AS nkind, n.label AS nlabel, \
                        n.aliases AS naliases, n.props AS nprops, n.vf AS nvf, n.vt AS nvt \
                 ORDER BY r.rel, n.id, r.vf"
            );
            let q = query(&text)
                .param("id", id)
                .param("rels_all", filter.rels.is_none())
                .param("rels", filter.rels.clone().unwrap_or_default())
                .param("kinds_all", filter.kinds.is_none())
                .param(
                    "kinds",
                    filter
                        .kinds
                        .as_ref()
                        .map(|kinds| {
                            kinds
                                .iter()
                                .map(|kind| kind_token(*kind))
                                .collect::<Result<Vec<_>>>()
                        })
                        .transpose()?
                        .unwrap_or_default(),
                )
                .param("as_of", filter.as_of.clone().unwrap_or_default());
            let mut stream = self
                .graph
                .execute(q)
                .await
                .map_err(|error| Error::Storage(format!("bolt neighbors: {error}")))?;
            while let Some(row) = stream
                .next()
                .await
                .map_err(|error| Error::Storage(format!("bolt neighbors row: {error}")))?
            {
                reached.push(row_to_pair(&row)?);
            }
        }
        if filter.direction == Direction::Both {
            // The two directional queries are each ordered; merge to the
            // contract order and drop duplicate edges (an edge can only show
            // up twice if both endpoints anchor it, which `n.id <> $id`
            // already prevents — sort is still re-applied for determinism).
            reached.sort_by(|(left_edge, left_node), (right_edge, right_node)| {
                (&left_edge.rel, &left_node.id, &left_edge.valid_from).cmp(&(
                    &right_edge.rel,
                    &right_node.id,
                    &right_edge.valid_from,
                ))
            });
        }
        Ok(reached)
    }
}

#[async_trait]
impl GraphEngine for BoltGraphEngine {
    async fn upsert_nodes(&self, nodes: Vec<GraphNode>) -> Result<()> {
        if nodes.is_empty() {
            return Err(Error::Storage(
                "node upsert must include at least one node".to_string(),
            ));
        }
        for node in &nodes {
            validate_graph_node(node)?;
        }
        let mut txn = self
            .graph
            .start_txn()
            .await
            .map_err(|error| Error::Storage(format!("bolt txn: {error}")))?;
        for node in nodes {
            let q = query(
                "MERGE (e:E {id:$id}) \
                 SET e.kind=$kind, e.label=$label, e.aliases=$aliases, \
                     e.props=$props, e.vf=$vf, e.vt=$vt",
            )
            .param("id", node.id.clone())
            .param("kind", kind_token(node.kind)?)
            .param("label", node.label)
            .param("aliases", node.aliases)
            .param("props", encode_json(&node.props)?)
            .param("vf", node.valid_from.unwrap_or_else(|| OPEN_BOUND.into()))
            .param("vt", node.valid_to.unwrap_or_else(|| OPEN_BOUND.into()));
            txn.run(q)
                .await
                .map_err(|error| Error::Storage(format!("bolt node upsert: {error}")))?;
        }
        txn.commit()
            .await
            .map_err(|error| Error::Storage(format!("bolt commit: {error}")))
    }

    async fn upsert_edges(&self, edges: Vec<GraphEdge>) -> Result<()> {
        if edges.is_empty() {
            return Err(Error::Storage(
                "edge upsert must include at least one edge".to_string(),
            ));
        }
        for edge in &edges {
            validate_graph_edge(edge)?;
        }
        // Endpoint existence is checked BEFORE any write so the batch is
        // all-or-nothing (the in-memory engine checks under one lock); the
        // writes then commit in one transaction, matching upsert_nodes.
        for edge in &edges {
            for endpoint in [&edge.from, &edge.to] {
                if self.node(endpoint).await?.is_none() {
                    return Err(Error::Storage(format!(
                        "edge endpoint missing: {} -{}-> {}",
                        edge.from, edge.rel, edge.to
                    )));
                }
            }
        }
        let mut txn = self
            .graph
            .start_txn()
            .await
            .map_err(|error| Error::Storage(format!("bolt txn: {error}")))?;
        for edge in edges {
            txn.run(edge_merge_query(&edge)?)
                .await
                .map_err(|error| Error::Storage(format!("bolt edge upsert: {error}")))?;
        }
        txn.commit()
            .await
            .map_err(|error| Error::Storage(format!("bolt commit: {error}")))
    }

    async fn node(&self, id: &str) -> Result<Option<GraphNode>> {
        let q = query(
            "MATCH (e:E {id:$id}) \
             RETURN e.id AS nid, e.kind AS nkind, e.label AS nlabel, \
                    e.aliases AS naliases, e.props AS nprops, e.vf AS nvf, e.vt AS nvt",
        )
        .param("id", id);
        let mut stream = self
            .graph
            .execute(q)
            .await
            .map_err(|error| Error::Storage(format!("bolt node: {error}")))?;
        stream
            .next()
            .await
            .map_err(|error| Error::Storage(format!("bolt node row: {error}")))?
            .map(|row| row_to_node(&row))
            .transpose()
    }

    async fn neighbors(
        &self,
        id: &str,
        filter: &TraversalFilter,
    ) -> Result<Vec<(GraphEdge, GraphNode)>> {
        validate_traversal_filter(filter)?;
        self.hop(id, filter).await
    }

    async fn expand(&self, seeds: &[String], filter: &TraversalFilter) -> Result<Subgraph> {
        validate_traversal_filter(filter)?;
        if seeds.is_empty() {
            return Err(Error::InvalidQuery(
                "expand requires at least one seed".to_string(),
            ));
        }
        // Same BFS as the in-memory engine, over the Cypher hop primitive.
        let mut node_ids = BTreeSet::new();
        let mut nodes = Vec::new();
        let mut edge_keys = BTreeSet::new();
        let mut edges = Vec::new();
        let mut frontier = VecDeque::new();
        for seed in seeds {
            if let Some(node) = self.node(seed).await?
                && node_passes(&node, filter)
                && node_ids.insert(node.id.clone())
            {
                frontier.push_back((node.id.clone(), 0_u8));
                nodes.push(node);
            }
        }
        while let Some((anchor, depth)) = frontier.pop_front() {
            if depth >= filter.max_hops {
                continue;
            }
            for (edge, node) in self.hop(&anchor, filter).await? {
                let key = (
                    edge.from.clone(),
                    edge.to.clone(),
                    edge.rel.clone(),
                    edge.valid_from.clone(),
                );
                if edge_keys.insert(key) {
                    edges.push(edge);
                }
                if node_ids.insert(node.id.clone()) {
                    frontier.push_back((node.id.clone(), depth + 1));
                    nodes.push(node);
                }
            }
        }
        nodes.sort_by(|left, right| left.id.cmp(&right.id));
        edges.sort_by(|left, right| {
            (&left.from, &left.rel, &left.to, &left.valid_from).cmp(&(
                &right.from,
                &right.rel,
                &right.to,
                &right.valid_from,
            ))
        });
        Ok(Subgraph { nodes, edges })
    }

    async fn shortest_path(
        &self,
        from: &str,
        to: &str,
        filter: &TraversalFilter,
    ) -> Result<Option<Vec<GraphEdge>>> {
        validate_traversal_filter(filter)?;
        if self.node(from).await?.is_none() || self.node(to).await?.is_none() {
            return Ok(None);
        }
        if from == to {
            return Ok(Some(Vec::new()));
        }
        let mut visited = BTreeSet::from([from.to_string()]);
        let mut frontier = VecDeque::from([(from.to_string(), Vec::<GraphEdge>::new())]);
        while let Some((anchor, path)) = frontier.pop_front() {
            if path.len() >= usize::from(filter.max_hops) {
                continue;
            }
            for (edge, node) in self.hop(&anchor, filter).await? {
                if !visited.insert(node.id.clone()) {
                    continue;
                }
                let mut next_path = path.clone();
                next_path.push(edge);
                if node.id == to {
                    return Ok(Some(next_path));
                }
                frontier.push_back((node.id, next_path));
            }
        }
        Ok(None)
    }

    async fn merge_entities(
        &self,
        source: &str,
        target: &str,
        decision: &MergeDecision,
    ) -> Result<MergeReport> {
        if source == target {
            return Err(Error::Storage(format!("self-merge rejected: {source}")));
        }
        if decision.approved_by.trim().is_empty()
            || decision.approved_by.chars().any(char::is_control)
        {
            return Err(Error::Storage(
                "merge approved_by must be non-empty and control-char-free".to_string(),
            ));
        }
        let source_node = self
            .node(source)
            .await?
            .ok_or_else(|| Error::Storage(format!("merge source missing: {source}")))?;
        if source_node.props.get("merged_into").is_some() {
            return Err(Error::Storage(format!(
                "merge source already merged: {source}"
            )));
        }
        let mut target_node = self
            .node(target)
            .await?
            .ok_or_else(|| Error::Storage(format!("merge target missing: {target}")))?;

        // Alias union (client-side, identical to the reference engine).
        let mut absorbed_aliases = 0;
        for alias in source_node
            .aliases
            .iter()
            .chain(std::iter::once(&source_node.label))
        {
            if alias != &target_node.label && !target_node.aliases.contains(alias) {
                target_node.aliases.push(alias.clone());
                absorbed_aliases += 1;
            }
        }
        target_node.aliases.sort();

        // Rewire: read every edge touching the source, compute the surviving
        // identities on the target side, then delete + recreate in Cypher.
        // Reads happen before the writes; the engine assumes the platform's
        // single-writer daemon model (documented), as does the upsert path.
        let any = TraversalFilter {
            direction: Direction::Both,
            max_hops: 1,
            ..TraversalFilter::default()
        };
        let source_edges = self.source_touching_edges(source).await?;
        let target_identities: BTreeSet<_> = self
            .hop(target, &any)
            .await?
            .into_iter()
            .map(|(edge, _)| (edge.from, edge.to, edge.rel, edge.valid_from))
            .collect();

        let (rewired, rewired_edges) =
            compute_rewired(source, target, source_edges, &target_identities);

        // Tombstone marker inside props (the shape the conformance suite pins).
        let mut tombstone_props = source_node.props.clone();
        if let serde_json::Value::Object(map) = &mut tombstone_props {
            map.insert(
                "merged_into".to_string(),
                serde_json::Value::String(target.to_string()),
            );
        } else {
            tombstone_props = serde_json::json!({ "merged_into": target });
        }

        // Apply ALL writes in ONE transaction: a mid-sequence failure must
        // never leave a half-merged graph (edges gone, no tombstone), which
        // would both corrupt reads and wedge retries on the already-merged
        // guard. Rewired/audit edge endpoints are known to exist from the
        // reads above, so the inline MERGE needs no endpoint probe.
        let mut txn = self
            .graph
            .start_txn()
            .await
            .map_err(|error| Error::Storage(format!("bolt txn: {error}")))?;
        txn.run(query("MATCH (s:E {id:$id})-[r:R]-() DELETE r").param("id", source))
            .await
            .map_err(|error| Error::Storage(format!("bolt merge delete: {error}")))?;
        txn.run(
            query("MATCH (s:E {id:$id}) SET s.props=$props")
                .param("id", source)
                .param("props", encode_json(&tombstone_props)?),
        )
        .await
        .map_err(|error| Error::Storage(format!("bolt merge tombstone: {error}")))?;
        txn.run(
            query("MATCH (t:E {id:$id}) SET t.aliases=$aliases")
                .param("id", target)
                .param("aliases", target_node.aliases.clone()),
        )
        .await
        .map_err(|error| Error::Storage(format!("bolt merge aliases: {error}")))?;
        let audit = GraphEdge {
            from: source.to_string(),
            to: target.to_string(),
            rel: "merged_into".to_string(),
            props: serde_json::Value::Null,
            provenance: Provenance::Manual {
                approved_by: decision.approved_by.clone(),
            },
            valid_from: None,
            valid_to: None,
        };
        for edge in rewired.into_iter().chain(std::iter::once(audit)) {
            txn.run(edge_merge_query(&edge)?)
                .await
                .map_err(|error| Error::Storage(format!("bolt merge edge: {error}")))?;
        }
        txn.commit()
            .await
            .map_err(|error| Error::Storage(format!("bolt commit: {error}")))?;

        Ok(MergeReport {
            source: source.to_string(),
            target: target.to_string(),
            rewired_edges,
            absorbed_aliases,
        })
    }
}

impl BoltGraphEngine {
    /// Every edge with `id` as either endpoint, unfiltered (merge support).
    async fn source_touching_edges(&self, id: &str) -> Result<Vec<GraphEdge>> {
        let q = query(
            "MATCH (a:E {id:$id})-[r:R]-(n:E) \
             RETURN r.rel AS rel, r.props AS rprops, r.provenance AS rprov, \
                    r.vf AS rvf, r.vt AS rvt, \
                    startNode(r).id AS efrom, endNode(r).id AS eto \
             ORDER BY r.rel, efrom, eto, r.vf",
        )
        .param("id", id);
        let mut stream = self
            .graph
            .execute(q)
            .await
            .map_err(|error| Error::Storage(format!("bolt source edges: {error}")))?;
        let mut edges = Vec::new();
        let mut keys = BTreeSet::new();
        while let Some(row) = stream
            .next()
            .await
            .map_err(|error| Error::Storage(format!("bolt source edges row: {error}")))?
        {
            let edge = row_to_edge(&row)?;
            // The undirected match yields self-loops twice; dedup by identity.
            if keys.insert((
                edge.from.clone(),
                edge.to.clone(),
                edge.rel.clone(),
                edge.valid_from.clone(),
            )) {
                edges.push(edge);
            }
        }
        Ok(edges)
    }
}

/// Re-point every source-touching edge at the target, dropping would-be
/// self-loops and duplicates of surviving target edges — the A2 rewiring
/// semantics, extracted pure so it is unit-testable without a server.
fn compute_rewired(
    source: &str,
    target: &str,
    source_edges: Vec<GraphEdge>,
    target_identities: &BTreeSet<(String, String, String, Option<String>)>,
) -> (Vec<GraphEdge>, usize) {
    let mut rewired = Vec::new();
    let mut seen = BTreeSet::new();
    let mut rewired_edges = 0;
    for mut edge in source_edges {
        if edge.from == source {
            edge.from = target.to_string();
        }
        if edge.to == source {
            edge.to = target.to_string();
        }
        if edge.from == edge.to {
            continue;
        }
        let key = (
            edge.from.clone(),
            edge.to.clone(),
            edge.rel.clone(),
            edge.valid_from.clone(),
        );
        if target_identities.contains(&key) || !seen.insert(key) {
            continue;
        }
        rewired_edges += 1;
        rewired.push(edge);
    }
    (rewired, rewired_edges)
}

/// The shared edge-upsert Cypher: MERGE on the A2 identity (from, to, rel,
/// valid_from - endpoints via the pattern, vf as a key property) with all
/// mutable fields SET on both branches. Endpoints must be pre-checked.
fn edge_merge_query(edge: &GraphEdge) -> Result<neo4rs::Query> {
    Ok(query(
        "MATCH (a:E {id:$from}), (b:E {id:$to}) \
         MERGE (a)-[r:R {rel:$rel, vf:$vf}]->(b) \
         SET r.props=$props, r.provenance=$prov, r.vt=$vt",
    )
    .param("from", edge.from.clone())
    .param("to", edge.to.clone())
    .param("rel", edge.rel.clone())
    .param(
        "vf",
        edge.valid_from.clone().unwrap_or_else(|| OPEN_BOUND.into()),
    )
    .param(
        "vt",
        edge.valid_to.clone().unwrap_or_else(|| OPEN_BOUND.into()),
    )
    .param("props", encode_json(&edge.props)?)
    .param("prov", encode_provenance(&edge.provenance)?))
}

fn node_passes(node: &GraphNode, filter: &TraversalFilter) -> bool {
    filter
        .kinds
        .as_ref()
        .is_none_or(|kinds| kinds.contains(&node.kind))
        && filter.as_of.as_deref().is_none_or(|as_of| {
            tdw_core::active_at(node.valid_from.as_deref(), node.valid_to.as_deref(), as_of)
        })
}

fn kind_token(kind: tdw_taxonomy::EntityKind) -> Result<String> {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .ok_or_else(|| Error::Storage("kind token encode failed".to_string()))
}

fn kind_from_token(token: &str) -> Result<tdw_taxonomy::EntityKind> {
    serde_json::from_value(serde_json::Value::String(token.to_string()))
        .map_err(|error| Error::Storage(format!("unknown kind token {token:?}: {error}")))
}

fn encode_json(value: &serde_json::Value) -> Result<String> {
    serde_json::to_string(value).map_err(|error| Error::Storage(format!("props encode: {error}")))
}

fn decode_json(text: &str) -> Result<serde_json::Value> {
    serde_json::from_str(text).map_err(|error| Error::Storage(format!("props decode: {error}")))
}

fn encode_provenance(provenance: &Provenance) -> Result<String> {
    serde_json::to_string(provenance)
        .map_err(|error| Error::Storage(format!("provenance encode: {error}")))
}

fn decode_provenance(text: &str) -> Result<Provenance> {
    serde_json::from_str(text)
        .map_err(|error| Error::Storage(format!("provenance decode: {error}")))
}

fn get_string(row: &Row, key: &str) -> Result<String> {
    row.get::<String>(key)
        .map_err(|error| Error::Storage(format!("bolt column {key}: {error}")))
}

fn get_strings(row: &Row, key: &str) -> Result<Vec<String>> {
    row.get::<Vec<String>>(key)
        .map_err(|error| Error::Storage(format!("bolt column {key}: {error}")))
}

fn opt_bound(value: String) -> Option<String> {
    if value == OPEN_BOUND {
        None
    } else {
        Some(value)
    }
}

fn row_to_node(row: &Row) -> Result<GraphNode> {
    Ok(GraphNode {
        id: get_string(row, "nid")?,
        kind: kind_from_token(&get_string(row, "nkind")?)?,
        label: get_string(row, "nlabel")?,
        aliases: get_strings(row, "naliases")?,
        props: decode_json(&get_string(row, "nprops")?)?,
        valid_from: opt_bound(get_string(row, "nvf")?),
        valid_to: opt_bound(get_string(row, "nvt")?),
    })
}

fn row_to_edge(row: &Row) -> Result<GraphEdge> {
    Ok(GraphEdge {
        from: get_string(row, "efrom")?,
        to: get_string(row, "eto")?,
        rel: get_string(row, "rel")?,
        props: decode_json(&get_string(row, "rprops")?)?,
        provenance: decode_provenance(&get_string(row, "rprov")?)?,
        valid_from: opt_bound(get_string(row, "rvf")?),
        valid_to: opt_bound(get_string(row, "rvt")?),
    })
}

fn row_to_pair(row: &Row) -> Result<(GraphEdge, GraphNode)> {
    Ok((row_to_edge(row)?, row_to_node(row)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn edge(from: &str, rel: &str, to: &str) -> GraphEdge {
        GraphEdge {
            from: from.to_string(),
            to: to.to_string(),
            rel: rel.to_string(),
            props: json!({}),
            provenance: Provenance::Ingest {
                source: "fixture".to_string(),
            },
            valid_from: None,
            valid_to: None,
        }
    }

    #[test]
    fn compute_rewired_drops_self_loops_and_duplicates() {
        // account->SRC rewires to account->TGT, but an identical surviving
        // edge exists, so it drops; SRC->TGT becomes a self-loop-free rewire;
        // SRC->SRC self-loop drops entirely.
        let surviving: BTreeSet<_> = [(
            "account:alpha".to_string(),
            "instrument:TGT".to_string(),
            "holds".to_string(),
            None,
        )]
        .into();
        let (rewired, count) = compute_rewired(
            "instrument:SRC",
            "instrument:TGT",
            vec![
                edge("account:alpha", "holds", "instrument:SRC"),
                edge("instrument:SRC", "tracks", "instrument:SRC"),
                edge("instrument:SRC", "peer_of", "instrument:OTHER"),
            ],
            &surviving,
        );
        assert_eq!(count, 1);
        assert_eq!(rewired.len(), 1);
        assert_eq!(rewired[0].from, "instrument:TGT");
        assert_eq!(rewired[0].to, "instrument:OTHER");
        assert_eq!(rewired[0].rel, "peer_of");
    }

    #[test]
    fn open_bounds_round_trip_through_the_sentinel() {
        assert_eq!(opt_bound(OPEN_BOUND.to_string()), None);
        assert_eq!(
            opt_bound("2026-01-01T00:00:00Z".to_string()),
            Some("2026-01-01T00:00:00Z".to_string())
        );
        // "" sorts before every timestamp, keeping lexicographic comparisons
        // in Cypher consistent with the half-open contract.
        assert!(OPEN_BOUND < "2026-01-01T00:00:00Z");
    }
}
