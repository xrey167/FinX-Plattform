# Crate Dependency And Feature Topology

Baseline generated during G001 and refreshed during tranche audits. Treat this as the manifest topology snapshot for crate-readiness planning, not as proof that every dependency is architecturally final.

## Tranche Assignment

| Tranche | Crates |
| --- | --- |
| G002-core-contracts-event-session-and-replay-crates | tdw-core, tdw-domain, tdw-protocol, tdw-config, tdw-event, tdw-actor, tdw-bus, tdw-cdc, tdw-outbox, tdw-snapshot, tdw-replay, tdw-rollout, tdw-session |
| G003-data-storage-pipeline-and-sql-crates | tdw-dbt-runner, tdw-migration, tdw-pipe, tdw-pipeline, tdw-sql-codegen, tdw-stage, tdw-table-format, tdw-storage-clickhouse, tdw-storage-fs, tdw-storage-meilisearch, tdw-storage-parquet, tdw-storage-postgres, tdw-storage-qdrant, tdw-storage-router, tdw-storage-s3 |
| G004-provider-embedding-and-model-adapter-crates | tdw-provider-alpaca, tdw-provider-binance, tdw-provider-fileset, tdw-provider-fred, tdw-provider-huggingface, tdw-provider-polygon, tdw-provider-ws-mock, tdw-provider-yahoo, tdw-embed, tdw-embed-local, tdw-embed-openai, tdw-embed-google, tdw-llm, tdw-llm-anthropic, tdw-llm-openai-compat |
| G005-agent-auth-hooks-tools-and-udf-crates | tdw-agent, tdw-agent-store, tdw-auth, tdw-auth-oidc, tdw-define, tdw-hooks, tdw-mask, tdw-tools, tdw-sandbox, tdw-udf, tdw-udf-external, tdw-udf-js, tdw-udf-python, tdw-udf-wasm |
| G006-knowledge-graph-tags-ml-eval-and-utility-crates | tdw-entity-resolver, tdw-eval-runner, tdw-feature-store, tdw-fn-string, tdw-graph, tdw-kg, tdw-knowledge, tdw-ml-registry, tdw-rewrite, tdw-spatial, tdw-tag-rules, tdw-tags, tdw-test-utils, tdw-workflow-engine |
| G007-client-service-mcp-acp-runtime-and-worker-crates | tdw-acp, tdw-app-client, tdw-app-server, tdw-cli, tdw-exec, tdw-mcp, tdw-runtime, tdw-service, tdw-service-api, tdw-tui, tdw-worker |
| G008-aggregate-production-readiness-gate | xtask |

## Local Dependency Edges

| Crate | Depends on workspace crates | Used by workspace crates |
| --- | --- | --- |
| [tdw-acp](tdw-acp.md) | tdw-protocol | tdw-service-api |
| [tdw-actor](tdw-actor.md) | tdw-event | tdw-service-api |
| [tdw-agent](tdw-agent.md) | none | tdw-agent-store, tdw-eval-runner, tdw-service-api, tdw-workflow-engine, xtask |
| [tdw-agent-store](tdw-agent-store.md) | tdw-agent | tdw-eval-runner, tdw-service-api |
| [tdw-app-client](tdw-app-client.md) | tdw-app-server, tdw-protocol | tdw-service-api |
| [tdw-app-server](tdw-app-server.md) | tdw-protocol | tdw-app-client, tdw-service-api |
| [tdw-auth](tdw-auth.md) | none | tdw-service-api |
| [tdw-auth-oidc](tdw-auth-oidc.md) | none | tdw-service-api |
| [tdw-bus](tdw-bus.md) | tdw-event | tdw-service-api |
| [tdw-cdc](tdw-cdc.md) | tdw-event, tdw-outbox | tdw-replay, tdw-service-api |
| [tdw-cli](tdw-cli.md) | tdw-service-api | none |
| [tdw-config](tdw-config.md) | none | tdw-llm, tdw-service-api, xtask |
| [tdw-core](tdw-core.md) | none | tdw-knowledge, tdw-provider-fileset, tdw-provider-ws-mock, tdw-provider-yahoo, tdw-runtime, tdw-service-api, tdw-storage-clickhouse, tdw-storage-fs, tdw-storage-meilisearch, tdw-storage-postgres, tdw-storage-qdrant, tdw-storage-router, tdw-storage-s3 |
| [tdw-dbt-runner](tdw-dbt-runner.md) | none | none |
| [tdw-define](tdw-define.md) | tdw-hooks | tdw-service-api |
| [tdw-domain](tdw-domain.md) | none | tdw-provider-fileset, tdw-provider-ws-mock, tdw-provider-yahoo, tdw-service-api, tdw-sql-codegen, tdw-test-utils |
| [tdw-embed](tdw-embed.md) | none | tdw-embed-local, tdw-knowledge, tdw-service-api |
| [tdw-embed-google](tdw-embed-google.md) | tdw-embed | none |
| [tdw-embed-local](tdw-embed-local.md) | tdw-embed | tdw-knowledge, tdw-service-api |
| [tdw-embed-openai](tdw-embed-openai.md) | tdw-embed | none |
| [tdw-entity-resolver](tdw-entity-resolver.md) | tdw-kg | tdw-service-api |
| [tdw-eval-runner](tdw-eval-runner.md) | tdw-agent, tdw-agent-store | tdw-service-api |
| [tdw-event](tdw-event.md) | none | tdw-actor, tdw-bus, tdw-cdc, tdw-hooks, tdw-outbox, tdw-service-api, xtask |
| [tdw-exec](tdw-exec.md) | tdw-protocol | tdw-service-api |
| [tdw-feature-store](tdw-feature-store.md) | tdw-tags | tdw-service-api |
| [tdw-fn-string](tdw-fn-string.md) | none | none |
| [tdw-graph](tdw-graph.md) | none | tdw-service-api |
| [tdw-hooks](tdw-hooks.md) | tdw-event, tdw-protocol | tdw-define, tdw-mask, tdw-service-api, tdw-session, tdw-tools |
| [tdw-kg](tdw-kg.md) | none | tdw-entity-resolver, tdw-knowledge, tdw-service-api |
| [tdw-knowledge](tdw-knowledge.md) | tdw-core, tdw-embed, tdw-embed-local, tdw-kg, tdw-storage-qdrant, tdw-tags | tdw-service-api |
| [tdw-llm](tdw-llm.md) | tdw-config | tdw-llm-anthropic, tdw-llm-openai-compat, tdw-service-api |
| [tdw-llm-anthropic](tdw-llm-anthropic.md) | tdw-llm | tdw-service-api |
| [tdw-llm-openai-compat](tdw-llm-openai-compat.md) | tdw-llm | tdw-service-api |
| [tdw-mask](tdw-mask.md) | tdw-hooks | tdw-service-api |
| [tdw-mcp](tdw-mcp.md) | tdw-service-api | none |
| [tdw-migration](tdw-migration.md) | none | xtask |
| [tdw-ml-registry](tdw-ml-registry.md) | none | none |
| [tdw-outbox](tdw-outbox.md) | tdw-event | tdw-cdc, tdw-service-api |
| [tdw-pipe](tdw-pipe.md) | tdw-stage | tdw-service-api |
| [tdw-pipeline](tdw-pipeline.md) | none | none |
| [tdw-protocol](tdw-protocol.md) | none | tdw-acp, tdw-app-client, tdw-app-server, tdw-exec, tdw-hooks, tdw-replay, tdw-rollout, tdw-service-api, tdw-session, tdw-tools, tdw-tui, xtask |
| [tdw-provider-alpaca](tdw-provider-alpaca.md) | none | none |
| [tdw-provider-binance](tdw-provider-binance.md) | none | none |
| [tdw-provider-fileset](tdw-provider-fileset.md) | tdw-core, tdw-domain | tdw-provider-yahoo, tdw-service-api |
| [tdw-provider-fred](tdw-provider-fred.md) | none | none |
| [tdw-provider-huggingface](tdw-provider-huggingface.md) | none | none |
| [tdw-provider-polygon](tdw-provider-polygon.md) | none | none |
| [tdw-provider-ws-mock](tdw-provider-ws-mock.md) | tdw-core, tdw-domain | tdw-service-api |
| [tdw-provider-yahoo](tdw-provider-yahoo.md) | tdw-core, tdw-domain, tdw-provider-fileset | tdw-service-api |
| [tdw-replay](tdw-replay.md) | tdw-cdc, tdw-protocol, tdw-rollout | tdw-service-api |
| [tdw-rewrite](tdw-rewrite.md) | none | none |
| [tdw-rollout](tdw-rollout.md) | tdw-protocol | tdw-replay, tdw-service-api |
| [tdw-runtime](tdw-runtime.md) | tdw-core | tdw-service-api |
| [tdw-sandbox](tdw-sandbox.md) | tdw-udf | tdw-service-api |
| [tdw-service](tdw-service.md) | tdw-service-api | none |
| [tdw-service-api](tdw-service-api.md) | tdw-acp, tdw-actor, tdw-agent, tdw-agent-store, tdw-app-client, tdw-app-server, tdw-auth, tdw-auth-oidc, tdw-bus, tdw-cdc, tdw-config, tdw-core, tdw-define, tdw-domain, tdw-embed, tdw-embed-local, tdw-entity-resolver, tdw-eval-runner, tdw-event, tdw-exec, tdw-feature-store, tdw-graph, tdw-hooks, tdw-kg, tdw-knowledge, tdw-llm, tdw-llm-anthropic, tdw-llm-openai-compat, tdw-mask, tdw-outbox, tdw-pipe, tdw-protocol, tdw-provider-fileset, tdw-provider-ws-mock, tdw-provider-yahoo, tdw-replay, tdw-rollout, tdw-runtime, tdw-sandbox, tdw-snapshot, tdw-spatial, tdw-stage, tdw-storage-meilisearch, tdw-storage-qdrant, tdw-storage-s3, tdw-table-format, tdw-tag-rules, tdw-tags, tdw-tools, tdw-tui, tdw-udf, tdw-workflow-engine | tdw-cli, tdw-mcp, tdw-service, tdw-worker |
| [tdw-session](tdw-session.md) | tdw-hooks, tdw-protocol | none |
| [tdw-snapshot](tdw-snapshot.md) | none | tdw-service-api |
| [tdw-spatial](tdw-spatial.md) | none | tdw-service-api |
| [tdw-sql-codegen](tdw-sql-codegen.md) | tdw-domain | xtask |
| [tdw-stage](tdw-stage.md) | none | tdw-pipe, tdw-service-api |
| [tdw-storage-clickhouse](tdw-storage-clickhouse.md) | tdw-core | none |
| [tdw-storage-fs](tdw-storage-fs.md) | tdw-core | none |
| [tdw-storage-meilisearch](tdw-storage-meilisearch.md) | tdw-core | tdw-service-api |
| [tdw-storage-parquet](tdw-storage-parquet.md) | none | none |
| [tdw-storage-postgres](tdw-storage-postgres.md) | tdw-core | none |
| [tdw-storage-qdrant](tdw-storage-qdrant.md) | tdw-core | tdw-knowledge, tdw-service-api |
| [tdw-storage-router](tdw-storage-router.md) | tdw-core | none |
| [tdw-storage-s3](tdw-storage-s3.md) | tdw-core | tdw-service-api |
| [tdw-table-format](tdw-table-format.md) | none | tdw-service-api |
| [tdw-tag-rules](tdw-tag-rules.md) | tdw-tags | tdw-service-api |
| [tdw-tags](tdw-tags.md) | none | tdw-feature-store, tdw-knowledge, tdw-service-api, tdw-tag-rules |
| [tdw-test-utils](tdw-test-utils.md) | tdw-domain | none |
| [tdw-tools](tdw-tools.md) | tdw-hooks, tdw-protocol | tdw-service-api |
| [tdw-tui](tdw-tui.md) | tdw-protocol | tdw-service-api |
| [tdw-udf](tdw-udf.md) | none | tdw-sandbox, tdw-service-api |
| [tdw-udf-external](tdw-udf-external.md) | none | none |
| [tdw-udf-js](tdw-udf-js.md) | none | none |
| [tdw-udf-python](tdw-udf-python.md) | none | none |
| [tdw-udf-wasm](tdw-udf-wasm.md) | none | none |
| [tdw-worker](tdw-worker.md) | tdw-protocol, tdw-service-api | none |
| [tdw-workflow-engine](tdw-workflow-engine.md) | tdw-agent | tdw-service-api |
| [xtask](xtask.md) | tdw-agent, tdw-config, tdw-event, tdw-migration, tdw-protocol, tdw-sql-codegen | none |

## Feature Flags

| Crate | Features |
| --- | --- |
| [tdw-core](tdw-core.md) | default; inventory-registration=[dep:inventory] |
| [tdw-test-utils](tdw-test-utils.md) | default; e2e; integration; property |

## Baseline Scan Notes

Baseline scan signals count conservative matches for TODO/todo!/unimplemented!/panic!/unwrap(/expect(/Bootstrap stub/stub/fallback in Rust files. Tranche audits classify each signal as test-only, acceptable, fixed, or a blocker.

| Crate | Baseline scan signals | Stub signals |
| --- | ---: | ---: |
| [tdw-acp](tdw-acp.md) | 6 | 0 |
| [tdw-agent](tdw-agent.md) | 7 | 0 |
| [tdw-app-client](tdw-app-client.md) | 2 | 0 |
| [tdw-app-server](tdw-app-server.md) | 3 | 0 |
| [tdw-config](tdw-config.md) | 4 | 0 |
| [tdw-core](tdw-core.md) | 9 | 0 |
| [tdw-dbt-runner](tdw-dbt-runner.md) | 2 | 0 |
| [tdw-domain](tdw-domain.md) | 2 | 0 |
| [tdw-embed](tdw-embed.md) | 1 | 0 |
| [tdw-embed-google](tdw-embed-google.md) | 2 | 0 |
| [tdw-embed-local](tdw-embed-local.md) | 2 | 0 |
| [tdw-embed-openai](tdw-embed-openai.md) | 2 | 0 |
| [tdw-event](tdw-event.md) | 1 | 0 |
| [tdw-cli](tdw-cli.md) | 2 | 0 |
| [tdw-exec](tdw-exec.md) | 3 | 0 |
| [tdw-feature-store](tdw-feature-store.md) | 2 | 0 |
| [tdw-fn-string](tdw-fn-string.md) | 0 | 0 |
| [tdw-hooks](tdw-hooks.md) | 4 | 0 |
| [tdw-knowledge](tdw-knowledge.md) | 4 | 0 |
| [tdw-kg](tdw-kg.md) | 1 | 0 |
| [tdw-llm](tdw-llm.md) | 1 | 0 |
| [tdw-llm-anthropic](tdw-llm-anthropic.md) | 2 | 0 |
| [tdw-llm-openai-compat](tdw-llm-openai-compat.md) | 2 | 0 |
| [tdw-mask](tdw-mask.md) | 1 | 0 |
| [tdw-ml-registry](tdw-ml-registry.md) | 0 | 0 |
| [tdw-pipe](tdw-pipe.md) | 1 | 0 |
| [tdw-pipeline](tdw-pipeline.md) | 1 | 0 |
| [tdw-protocol](tdw-protocol.md) | 7 | 0 |
| [tdw-provider-alpaca](tdw-provider-alpaca.md) | 1 | 0 |
| [tdw-provider-binance](tdw-provider-binance.md) | 1 | 0 |
| [tdw-provider-fileset](tdw-provider-fileset.md) | 1 | 0 |
| [tdw-provider-fred](tdw-provider-fred.md) | 1 | 0 |
| [tdw-provider-huggingface](tdw-provider-huggingface.md) | 1 | 0 |
| [tdw-provider-polygon](tdw-provider-polygon.md) | 1 | 0 |
| [tdw-provider-ws-mock](tdw-provider-ws-mock.md) | 6 | 0 |
| [tdw-provider-yahoo](tdw-provider-yahoo.md) | 4 | 0 |
| [tdw-replay](tdw-replay.md) | 1 | 0 |
| [tdw-rewrite](tdw-rewrite.md) | 0 | 0 |
| [tdw-rollout](tdw-rollout.md) | 5 | 0 |
| [tdw-mcp](tdw-mcp.md) | 4 | 0 |
| [tdw-runtime](tdw-runtime.md) | 4 | 0 |
| [tdw-service](tdw-service.md) | 2 | 0 |
| [tdw-sandbox](tdw-sandbox.md) | 1 | 0 |
| [tdw-service-api](tdw-service-api.md) | 51 | 0 |
| [tdw-tui](tdw-tui.md) | 2 | 0 |
| [tdw-worker](tdw-worker.md) | 51 | 0 |
| [tdw-session](tdw-session.md) | 19 | 0 |
| [tdw-stage](tdw-stage.md) | 2 | 0 |
| [tdw-storage-clickhouse](tdw-storage-clickhouse.md) | 7 | 0 |
| [tdw-storage-fs](tdw-storage-fs.md) | 2 | 0 |
| [tdw-storage-meilisearch](tdw-storage-meilisearch.md) | 2 | 0 |
| [tdw-storage-parquet](tdw-storage-parquet.md) | 4 | 0 |
| [tdw-storage-postgres](tdw-storage-postgres.md) | 7 | 0 |
| [tdw-storage-qdrant](tdw-storage-qdrant.md) | 2 | 0 |
| [tdw-storage-router](tdw-storage-router.md) | 4 | 0 |
| [tdw-storage-s3](tdw-storage-s3.md) | 2 | 0 |
| [tdw-tag-rules](tdw-tag-rules.md) | 4 | 0 |
| [tdw-tags](tdw-tags.md) | 4 | 0 |
| [tdw-tools](tdw-tools.md) | 8 | 0 |
| [tdw-udf](tdw-udf.md) | 1 | 0 |
| [tdw-udf-external](tdw-udf-external.md) | 0 | 0 |
| [tdw-udf-js](tdw-udf-js.md) | 0 | 0 |
| [tdw-udf-python](tdw-udf-python.md) | 0 | 0 |
| [tdw-udf-wasm](tdw-udf-wasm.md) | 0 | 0 |
| [tdw-workflow-engine](tdw-workflow-engine.md) | 1 | 0 |
| [xtask](xtask.md) | 8 | 0 |
