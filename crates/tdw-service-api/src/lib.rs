#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tdw_actor::{ActorContext, OmcSpawn};
use tdw_agent::{
    EvalCase, EvalRunRequest, WorkflowDefinition, WorkflowEdge, WorkflowNode, gotcha_seed,
    parse_slash_command_invocation, sample_agent_card, schema_bundle,
};
use tdw_agent_store::AgentStore;
use tdw_auth::{AuthPolicy, Principal, authorize};
use tdw_auth_oidc::{JwksKey, JwtClaims, validate_claims};
use tdw_bus::EventBus;
use tdw_cdc::CdcStream;
use tdw_core::{
    BlobEngine, Error, LexicalDoc, LexicalEngine, OBBject, ProgressOrResult, ProgressStream,
    ProviderKind, ProviderRegistry, Result, ScoredDoc, ScoredPoint, TextQuery, VectorEngine,
    VectorPoint, VectorQuery,
};
use tdw_define::DefineEvent;
use tdw_domain::{EquityHistoricalData, ResearchNote};
use tdw_embed::EmbeddingProvider;
use tdw_embed_local::HashEmbeddingProvider;
use tdw_entity_resolver::{manual_merge_decision, resolve_symbol};
use tdw_eval_runner::EvalRunner;
use tdw_event::{EventEnvelope, event_schema_bundle};
use tdw_feature_store::FeatureStore;
use tdw_graph::DirectedGraph;
use tdw_hooks::{HookRegistry, TransactionMode, event_hook};
use tdw_kg::{Entity, EntityKind, KnowledgeGraph, Relationship};
use tdw_mask::{MaskMode, MaskRule, apply_masks, masking_hook};
use tdw_outbox::InMemoryOutbox;
use tdw_pipe::PipeDefinition;
use tdw_provider_fileset::FilesetEquityHistoricalFetcher;
use tdw_provider_ws_mock::MockEquityStreamer;
use tdw_provider_yahoo::YahooEquityHistoricalFetcher;
use tdw_replay::ReplayEngine;
use tdw_runtime::CommandRunner;
use tdw_snapshot::SnapshotStore;
use tdw_spatial::{BoundingBox, Point};
use tdw_stage::StageLocation;
use tdw_storage_meilisearch::InMemoryLexicalEngine;
use tdw_storage_qdrant::InMemoryVectorEngine;
use tdw_storage_s3::InMemoryS3BlobEngine;
use tdw_table_format::{TableFile, TableFormat, TableManifest, simple_checksum};
use tdw_tag_rules::{RuleEngine, RulePredicate, TagRule};
use tdw_tags::{TagAssignment, TagDefinition, TagStore};
use tdw_udf::{UdfDefinition, UdfRuntime, evaluate};
use tdw_workflow_engine::WorkflowEngine;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderSummary {
    pub provider: String,
    pub endpoint: String,
    pub kind: ProviderKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResearchIndexEvidence {
    pub note_id: String,
    pub model_id: String,
    pub vector_hits: Vec<ScoredPoint>,
    pub lexical_hits: Vec<ScoredDoc>,
    pub blob_bytes: usize,
}

pub fn default_registry() -> Result<ProviderRegistry> {
    let mut registry = ProviderRegistry::default();
    registry.register(FilesetEquityHistoricalFetcher::registry_entry())?;
    registry.register(YahooEquityHistoricalFetcher::registry_entry())?;
    registry.register(MockEquityStreamer::registry_entry())?;
    Ok(registry)
}

pub fn list_providers() -> Result<Vec<ProviderSummary>> {
    Ok(default_registry()?
        .entries()
        .iter()
        .map(|entry| ProviderSummary {
            provider: entry.provider.to_string(),
            endpoint: entry.endpoint.to_string(),
            kind: entry.kind,
        })
        .collect())
}

pub fn fetch_equity_historical(
    provider: &str,
    symbol: &str,
) -> Result<OBBject<EquityHistoricalData>> {
    let runner = CommandRunner::new(default_registry()?);
    let params = json!({ "symbol": symbol });

    match provider {
        "fileset" => block_on(runner.run(&FilesetEquityHistoricalFetcher, params)),
        "yahoo" => block_on(runner.run(&YahooEquityHistoricalFetcher, params)),
        other => Err(Error::Registry(format!("unknown provider: {other}"))),
    }
}

pub fn mcp_progress_sample(symbol: &str) -> Result<Vec<String>> {
    let runner = CommandRunner::new(default_registry()?);
    let mut stream = block_on(
        runner.run_streaming(&FilesetEquityHistoricalFetcher, json!({ "symbol": symbol })),
    )?;
    let mut events = Vec::new();

    while let Some(event) = poll_stream_next(&mut stream)? {
        match event {
            ProgressOrResult::Progress {
                stage, fraction, ..
            } => events.push(format!("progress:{stage}:{fraction:.1}")),
            ProgressOrResult::Partial(_) => events.push("partial".to_string()),
            ProgressOrResult::Done(object) => {
                events.push(format!("done:{}:{}", object.provider, object.rows.len()));
            }
            ProgressOrResult::Error(error) => events.push(format!("error:{error}")),
        }
    }

    Ok(events)
}

pub fn index_research_note(note: ResearchNote) -> Result<ResearchIndexEvidence> {
    let embedder = HashEmbeddingProvider::default();
    let embedding = embedder
        .embed(&format!("{} {}", note.title, note.body))
        .map_err(|error| Error::Provider(error.to_string()))?;
    let vector = InMemoryVectorEngine::default();
    let lexical = InMemoryLexicalEngine::default();
    let blob = InMemoryS3BlobEngine::default();
    let blob_key = format!("research-note/{}.json", note.id);
    let payload = json!({
        "id": note.id,
        "title": note.title,
        "tags": note.tags,
    });

    block_on(vector.upsert(
        "research_note__local_hash",
        vec![VectorPoint {
            id: note.id.clone(),
            vector: embedding.vector.clone(),
            payload: payload.clone(),
        }],
    ))?;
    block_on(lexical.index(
        "research_note",
        vec![LexicalDoc {
            id: note.id.clone(),
            body: note.body.clone(),
            fields: payload,
        }],
    ))?;
    let serialized =
        serde_json::to_vec(&note).map_err(|error| Error::Storage(error.to_string()))?;
    block_on(blob.put_object(&blob_key, Bytes::from(serialized), "application/json"))?;

    let vector_hits = block_on(vector.search_knn(
        "research_note__local_hash",
        VectorQuery {
            vector: embedding.vector,
            top_k: 1,
        },
    ))?;
    let lexical_hits = block_on(lexical.search_text(
        "research_note",
        TextQuery {
            text: "Fixture".to_string(),
            top_k: 1,
        },
    ))?;
    let blob_bytes = block_on(blob.get_object(&blob_key))?.len();

    Ok(ResearchIndexEvidence {
        note_id: note.id,
        model_id: embedding.model_id,
        vector_hits,
        lexical_hits,
        blob_bytes,
    })
}

pub fn endpoint_response(provider: &str, symbol: &str) -> Result<Value> {
    let object = fetch_equity_historical(provider, symbol)?;
    Ok(json!({
        "provider": object.provider,
        "endpoint": object.endpoint,
        "rows": object.rows,
    }))
}

pub fn agent_schema_names() -> Vec<String> {
    schema_bundle()
        .keys()
        .map(|name| (*name).to_string())
        .collect()
}

pub fn mcp_agent_tools() -> Vec<String> {
    vec![
        "agent.card.get".to_string(),
        "agent.skill.parse".to_string(),
        "agent.command.parse".to_string(),
        "agent.eval.run".to_string(),
        "agent.workflow.compile".to_string(),
        "agent.gotcha.list".to_string(),
    ]
}

pub fn agent_tool_sample() -> Result<Value> {
    let mut store = AgentStore::new();
    let card = sample_agent_card();
    store.upsert_agent(card.clone());
    for gotcha in gotcha_seed() {
        store.upsert_gotcha(gotcha);
    }

    let workflow = WorkflowDefinition {
        workflow_id: "research-flow".to_string(),
        nodes: vec![
            WorkflowNode {
                node_id: "retrieve".to_string(),
                task: "retrieve context".to_string(),
                skill_id: None,
            },
            WorkflowNode {
                node_id: "draft".to_string(),
                task: "draft note".to_string(),
                skill_id: Some("research.note".to_string()),
            },
        ],
        edges: vec![WorkflowEdge {
            from: "retrieve".to_string(),
            to: "draft".to_string(),
        }],
    };
    let plan =
        WorkflowEngine::compile(&workflow).map_err(|error| Error::Provider(error.to_string()))?;
    store.upsert_workflow(workflow);

    let eval = EvalRunner::run(
        EvalRunRequest {
            run_id: "eval-1".to_string(),
            agent_id: card.agent_id.clone(),
            dataset_id: "golden-market-notes".to_string(),
            cases: vec![EvalCase {
                case_id: "case-1".to_string(),
                prompt: "Summarize AAPL".to_string(),
                expected_refs: card.content_refs.clone(),
            }],
        },
        &mut store,
    );
    let command = parse_slash_command_invocation("/research symbol=AAPL horizon=1d")
        .map_err(|error| Error::Provider(error.to_string()))?;

    Ok(json!({
        "agent_id": card.agent_id,
        "schemas": agent_schema_names(),
        "tools": mcp_agent_tools(),
        "workflow_order": plan.ordered_node_ids,
        "eval_status": eval.status,
        "slash_command": command,
        "storage_mappings": store.storage_mappings(),
    }))
}

pub fn event_schema_names() -> Vec<String> {
    event_schema_bundle()
        .keys()
        .map(|name| (*name).to_string())
        .collect()
}

pub fn event_spine_sample(entrypoint: &str) -> Result<Value> {
    let context = ActorContext::service(entrypoint);
    let task = OmcSpawn::capture(&context, "ingress");
    let envelope = EventEnvelope::new(
        "ingress.received",
        task.actor.clone(),
        task.origin.clone(),
        task.trace.clone(),
        "2026-05-21T00:00:00Z",
        json!({ "entrypoint": entrypoint, "task": task.task_name }),
    );

    let mut bus = EventBus::new(16);
    let sequence = bus.publish(envelope.clone());
    let mut hooks = HookRegistry::default();
    hooks.register(event_hook!("audit", 10, TransactionMode::PostCommit));
    hooks.register(event_hook!("cdc", 20, TransactionMode::InTransaction));
    let hook_outcomes = hooks
        .execute(&envelope)
        .map_err(|error| Error::Provider(error.to_string()))?;

    let mut outbox = InMemoryOutbox::default();
    outbox.append(envelope.clone());
    let pending = outbox.pending_after(0);
    let cdc = CdcStream::from_outbox(&pending);
    let replay = ReplayEngine::dry_run(&cdc.records);

    Ok(json!({
        "entrypoint": entrypoint,
        "actor_id": envelope.actor.actor_id,
        "trace_id": envelope.trace.trace_id,
        "bus_sequence": sequence,
        "bus_lag": bus.lag_since(0),
        "hook_order": hook_outcomes.iter().map(|hook| hook.name.clone()).collect::<Vec<_>>(),
        "outbox_pending": pending.len(),
        "cdc_offsets": cdc.records.iter().map(|record| record.offset).collect::<Vec<_>>(),
        "replay_dry_run": replay.dry_run,
        "schemas": event_schema_names(),
    }))
}

pub fn parity_layer_sample() -> Result<Value> {
    let mut snapshots = SnapshotStore::default();
    snapshots.commit(
        "raw.market_data_bar",
        "2026-05-21T00:00:00Z",
        vec!["bar-1".into()],
    );
    let snapshot = snapshots.commit(
        "raw.market_data_bar",
        "2026-05-21T01:00:00Z",
        vec!["bar-1".into(), "bar-2".into()],
    );
    let time_travel_rows = snapshots
        .as_of_version("raw.market_data_bar", 1)
        .map(|snapshot| snapshot.row_ids.len())
        .unwrap_or_default();

    let mut event_bus = EventBus::new(8);
    let stream_offset = event_bus.publish(EventEnvelope::new(
        "stream.market_data_bar",
        tdw_event::sample_actor_context("parity").0,
        tdw_event::sample_actor_context("parity").1,
        tdw_event::sample_actor_context("parity").2,
        "2026-05-21T00:00:00Z",
        json!({ "snapshot_version": snapshot.version }),
    ));

    let mut graph = DirectedGraph::default();
    graph.add_edge("account", "position");
    graph.add_edge("position", "instrument");
    let bbox = BoundingBox {
        min: Point {
            lat: 40.0,
            lon: -75.0,
        },
        max: Point {
            lat: 41.0,
            lon: -73.0,
        },
    };
    let stage = StageLocation {
        name: "market-stage".to_string(),
        uri: "s3://bucket/market".to_string(),
    };
    let mut pipe = PipeDefinition {
        name: "market-pipe".to_string(),
        stage: stage.clone(),
        target_table: "raw.market_data_bar".to_string(),
        last_offset: 0,
    };
    let copy_plan = pipe.copy_plan(vec!["ohlcv.parquet".to_string()]);
    pipe.advance(stream_offset);
    let manifest = TableManifest {
        format: TableFormat::Iceberg,
        table: "raw.market_data_bar".to_string(),
        version: snapshot.version,
        files: vec![TableFile {
            path: "s3://bucket/market/ohlcv.parquet".to_string(),
            checksum: simple_checksum("s3://bucket/market/ohlcv.parquet"),
        }],
    };
    let udf_output = evaluate(
        &UdfDefinition {
            name: "upper".to_string(),
            runtime: UdfRuntime::Wasm,
            source: "upper(input)".to_string(),
            allow_network: false,
            allow_filesystem: false,
        },
        "aapl",
    )
    .map_err(|error| Error::Provider(error.to_string()))?;
    let claims = JwtClaims {
        sub: "alice".to_string(),
        iss: "https://issuer".to_string(),
        aud: "tdw".to_string(),
        kid: "k1".to_string(),
        roles: vec!["analyst".to_string()],
    };
    let auth_policy = AuthPolicy {
        table: "analytics.gold_daily_returns".to_string(),
        required_role: "analyst".to_string(),
        row_filter: Some("tenant_id = current_tenant()".to_string()),
    };
    let define = DefineEvent {
        event_name: "market_data_changed".to_string(),
        on_table: "raw.market_data_bar".to_string(),
        hook_name: "emit.market_data_changed".to_string(),
        transaction_mode: TransactionMode::PostCommit,
    };
    let mut row = BTreeMap::new();
    row.insert("account_id".to_string(), "ACC123456".to_string());
    let masked = apply_masks(
        &row,
        &[MaskRule {
            field: "account_id".to_string(),
            mode: MaskMode::Last4,
        }],
    );

    Ok(json!({
        "snapshot_version": snapshot.version,
        "time_travel_rows": time_travel_rows,
        "stream_offset": stream_offset,
        "live_query_event_type": "stream.market_data_bar",
        "graph_path": graph.traverse("account"),
        "spatial_contains": bbox.contains(Point { lat: 40.7, lon: -74.0 }),
        "copy_checksum": copy_plan.checksum,
        "pipe_offset": pipe.last_offset,
        "table_manifest_ok": manifest.verify_checksums(),
        "udf_output": udf_output,
        "jwt_valid": validate_claims(
            &claims,
            &[JwksKey { kid: "k1".to_string(), alg: "RS256".to_string() }],
            "https://issuer",
            "tdw"
        ),
        "authorized": authorize(
            &Principal { subject: "alice".to_string(), roles: claims.roles.clone() },
            &auth_policy
        ),
        "define_hook": define.compile_hook().name,
        "define_key": define.idempotency_key(),
        "mask_hook": masking_hook().name,
        "masked_account": masked.get("account_id").cloned().unwrap_or_default(),
    }))
}

pub fn mcp_tag_tools() -> Vec<String> {
    vec![
        "kg.entity.get".to_string(),
        "kg.neighbors".to_string(),
        "tag.define".to_string(),
        "tag.assign".to_string(),
        "tag.rule.reload".to_string(),
        "tag.live.subscribe".to_string(),
    ]
}

pub fn kg_tag_sample() -> Result<Value> {
    let instrument = Entity {
        entity_id: "instrument:AAPL".to_string(),
        kind: EntityKind::Instrument,
        label: "Apple".to_string(),
        aliases: vec!["AAPL".to_string()],
    };
    let dataset = Entity {
        entity_id: "dataset:ohlcv".to_string(),
        kind: EntityKind::Dataset,
        label: "OHLCV".to_string(),
        aliases: Vec::new(),
    };
    let mut kg = KnowledgeGraph::default();
    kg.upsert_entity(instrument.clone());
    kg.upsert_entity(dataset.clone());
    kg.add_relationship(Relationship {
        from: instrument.entity_id.clone(),
        to: dataset.entity_id.clone(),
        rel_type: "has_prices".to_string(),
        provenance: "dbt:bronze_ohlcv".to_string(),
    });
    let merge = manual_merge_decision("instrument:AAPL", "instrument:APPLE", true);
    if merge.approved {
        kg.manual_merge(&merge.source, &merge.target, "architect");
    }

    let resolved = resolve_symbol("aapl", std::slice::from_ref(&instrument));
    let mut tags = TagStore::default();
    tags.define(TagDefinition {
        tag_id: "asset:equity".to_string(),
        parent: None,
        ttl_days: None,
    })
    .map_err(|error| Error::Provider(error.to_string()))?;
    tags.define(TagDefinition {
        tag_id: "style:momentum".to_string(),
        parent: Some("asset:equity".to_string()),
        ttl_days: Some(30),
    })
    .map_err(|error| Error::Provider(error.to_string()))?;
    tags.assign(TagAssignment {
        entity_id: instrument.entity_id.clone(),
        tag_id: "asset:equity".to_string(),
        assigned_at: "2026-05-21".to_string(),
        expires_at: None,
        provenance: "manual:seed".to_string(),
    })
    .map_err(|error| Error::Provider(error.to_string()))?;

    let mut rules = RuleEngine::default();
    rules
        .hot_reload(vec![TagRule {
            rule_id: "momentum-label".to_string(),
            tag_id: "style:momentum".to_string(),
            predicate: RulePredicate::LabelContains {
                label: "AAPL".to_string(),
            },
        }])
        .map_err(|error| Error::Provider(error.to_string()))?;
    let rule_assignments = rules
        .apply(&instrument.entity_id, "AAPL momentum", &mut tags)
        .map_err(|error| Error::Provider(error.to_string()))?;

    let mut features = BTreeMap::new();
    features.insert("return_1d".to_string(), 0.012);
    let mut feature_store = FeatureStore::default();
    let feature_snapshot =
        feature_store.materialize(&instrument.entity_id, "2026-05-21", features, &tags);
    let mut live_bus = EventBus::new(8);
    let live_event = tdw_event::sample_event("tag-live").child_event(
        "tag.assignment.changed",
        json!({ "entity_id": instrument.entity_id.clone(), "tags": feature_snapshot.tags.clone() }),
    );
    let live_offset = live_bus.publish(live_event);

    Ok(json!({
        "entity": instrument.entity_id,
        "neighbors": kg.neighbors("instrument:AAPL").iter().map(|entity| entity.entity_id.clone()).collect::<Vec<_>>(),
        "resolved": resolved.iter().map(|candidate| candidate.entity_id.clone()).collect::<Vec<_>>(),
        "manual_merge_audited": merge.audited,
        "active_tags": tags.active_tags("instrument:AAPL", "2026-05-21"),
        "rule_version": rules.version(),
        "rule_assignments": rule_assignments.len(),
        "taxonomy_stats": tags.taxonomy_stats(),
        "hybrid_search_filter": "tag:asset:equity",
        "dbt_model": "meta_tag_assignments",
        "agent_tag_interests": ["asset:equity", "style:momentum"],
        "mcp_tools": mcp_tag_tools(),
        "live_offset": live_offset,
        "feature_tags": feature_snapshot.tags,
        "feature_count": feature_snapshot.features.len(),
    }))
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);

    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
    }
}

fn poll_stream_next<T: tdw_core::DataModel>(
    stream: &mut ProgressStream<T>,
) -> Result<Option<ProgressOrResult<T>>> {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    match stream.as_mut().poll_next(&mut context) {
        Poll::Ready(Some(item)) => item.map(Some),
        Poll::Ready(None) | Poll::Pending => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_fetchers_and_streamer() {
        let providers =
            list_providers().unwrap_or_else(|error| panic!("providers should list: {error}"));

        assert!(providers.iter().any(|provider| {
            provider.provider == "fileset" && provider.kind == ProviderKind::Fetcher
        }));
        assert!(
            providers
                .iter()
                .any(|provider| provider.provider == "mock-ws"
                    && provider.kind == ProviderKind::Streamer)
        );
    }

    #[test]
    fn fetch_endpoint_uses_command_runner() {
        let object = fetch_equity_historical("fileset", "aapl")
            .unwrap_or_else(|error| panic!("fetch should succeed: {error}"));

        assert_eq!(object.provider, "fileset");
        assert_eq!(object.rows[0].symbol, "AAPL");
    }

    #[test]
    fn research_note_indexes_across_retrieval_stores() {
        let evidence = index_research_note(ResearchNote {
            id: "note-1".to_string(),
            title: "Macro note".to_string(),
            body: "Fixture note for deterministic tests.".to_string(),
            tags: vec!["macro".to_string()],
        })
        .unwrap_or_else(|error| panic!("indexing should succeed: {error}"));

        assert_eq!(evidence.note_id, "note-1");
        assert_eq!(evidence.vector_hits.len(), 1);
        assert_eq!(evidence.lexical_hits.len(), 1);
        assert!(evidence.blob_bytes > 0);
    }

    #[test]
    fn mcp_progress_sample_emits_progress_and_done() {
        let events = mcp_progress_sample("aapl")
            .unwrap_or_else(|error| panic!("progress sample should succeed: {error}"));

        assert_eq!(events[0], "progress:fetch:0.0");
        assert_eq!(events[2], "done:fileset:2");
    }

    #[test]
    fn agent_tool_sample_wires_schema_store_eval_and_workflow() {
        let evidence = agent_tool_sample()
            .unwrap_or_else(|error| panic!("agent tool sample should succeed: {error}"));

        assert_eq!(evidence["agent_id"], "market-researcher");
        assert_eq!(evidence["eval_status"], "success");
        assert_eq!(evidence["workflow_order"][0], "retrieve");
        assert!(
            evidence["tools"]
                .as_array()
                .is_some_and(|tools| tools.len() >= 5)
        );
    }

    #[test]
    fn event_spine_sample_wires_actor_bus_hooks_outbox_cdc_and_replay() {
        let evidence = event_spine_sample("mcp")
            .unwrap_or_else(|error| panic!("event sample should succeed: {error}"));

        assert_eq!(evidence["entrypoint"], "mcp");
        assert_eq!(evidence["hook_order"][0], "audit");
        assert_eq!(evidence["outbox_pending"], 1);
        assert_eq!(evidence["replay_dry_run"], true);
    }

    #[test]
    fn parity_layer_sample_wires_layer_c_features() {
        let evidence = parity_layer_sample()
            .unwrap_or_else(|error| panic!("parity sample should succeed: {error}"));

        assert_eq!(evidence["snapshot_version"], 2);
        assert_eq!(evidence["time_travel_rows"], 1);
        assert_eq!(evidence["udf_output"], "AAPL");
        assert_eq!(evidence["table_manifest_ok"], true);
        assert_eq!(evidence["jwt_valid"], true);
        assert_eq!(evidence["authorized"], true);
        assert_eq!(evidence["masked_account"], "***3456");
    }

    #[test]
    fn kg_tag_sample_wires_kg_tags_rules_mcp_and_features() {
        let evidence =
            kg_tag_sample().unwrap_or_else(|error| panic!("kg sample should succeed: {error}"));

        assert_eq!(evidence["entity"], "instrument:AAPL");
        assert_eq!(evidence["neighbors"][0], "dataset:ohlcv");
        assert_eq!(evidence["manual_merge_audited"], true);
        assert_eq!(evidence["rule_assignments"], 1);
        assert_eq!(evidence["hybrid_search_filter"], "tag:asset:equity");
        assert_eq!(evidence["dbt_model"], "meta_tag_assignments");
        assert_eq!(evidence["feature_count"], 1);
    }
}
