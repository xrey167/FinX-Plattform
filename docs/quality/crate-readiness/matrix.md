# Crate Readiness Matrix

Initialized during G001 from cargo metadata and updated by tranche audits. Pending rows have baseline inventory only; non-pending rows include crate-level audit evidence in their worksheets.

| Crate | Owner tranche | Targets | Local deps | Reverse deps | Features | Test attrs | Tests dir | Docs/examples | Baseline scan signals | Verdict |
| --- | --- | --- | --- | --- | --- | ---: | --- | --- | ---: | --- |
| [tdw-acp](tdw-acp.md) | G007-client-service-mcp-acp-runtime-and-worker-crates | lib | tdw-protocol | tdw-service-api | none | 4 | no | none | 6 | Ready with follow-ups |
| [tdw-actor](tdw-actor.md) | G002-core-contracts-event-session-and-replay-crates | lib | tdw-event | tdw-service-api | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-agent](tdw-agent.md) | G005-agent-auth-hooks-tools-and-udf-crates | lib | none | tdw-agent-store, tdw-eval-runner, tdw-service-api, tdw-workflow-engine, xtask | none | 6 | yes | none | 7 | Ready with follow-ups |
| [tdw-agent-store](tdw-agent-store.md) | G005-agent-auth-hooks-tools-and-udf-crates | lib | tdw-agent | tdw-eval-runner, tdw-service-api | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-app-client](tdw-app-client.md) | G007-client-service-mcp-acp-runtime-and-worker-crates | lib | tdw-app-server, tdw-protocol | tdw-service-api | none | 2 | no | none | 2 | Ready with follow-ups |
| [tdw-app-server](tdw-app-server.md) | G007-client-service-mcp-acp-runtime-and-worker-crates | lib | tdw-protocol | tdw-app-client, tdw-service-api | none | 2 | no | none | 3 | Ready with follow-ups |
| [tdw-auth](tdw-auth.md) | G005-agent-auth-hooks-tools-and-udf-crates | lib | none | tdw-service-api | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-auth-oidc](tdw-auth-oidc.md) | G005-agent-auth-hooks-tools-and-udf-crates | lib | none | tdw-service-api | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-bus](tdw-bus.md) | G002-core-contracts-event-session-and-replay-crates | lib | tdw-event | tdw-service-api | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-cdc](tdw-cdc.md) | G002-core-contracts-event-session-and-replay-crates | lib | tdw-event, tdw-outbox | tdw-replay, tdw-service-api | none | 1 | no | none | 0 | Ready with follow-ups |
| [tdw-cli](tdw-cli.md) | G007-client-service-mcp-acp-runtime-and-worker-crates | bin | tdw-service-api | none | none | 0 | no | none | 2 | Ready with follow-ups |
| [tdw-config](tdw-config.md) | G002-core-contracts-event-session-and-replay-crates | lib | none | tdw-llm, tdw-service-api, xtask | none | 4 | no | none | 4 | Ready with follow-ups |
| [tdw-core](tdw-core.md) | G002-core-contracts-event-session-and-replay-crates | lib, test | none | tdw-knowledge, tdw-provider-fileset, tdw-provider-ws-mock, tdw-provider-yahoo, tdw-runtime, tdw-service-api, tdw-storage-clickhouse, tdw-storage-fs, tdw-storage-meilisearch, tdw-storage-postgres, tdw-storage-qdrant, tdw-storage-router, tdw-storage-s3 | default; inventory-registration=[dep:inventory] | 7 | yes | none | 9 | Ready with follow-ups |
| [tdw-dbt-runner](tdw-dbt-runner.md) | G003-data-storage-pipeline-and-sql-crates | lib | none | none | none | 2 | no | none | 2 | Ready with follow-ups |
| [tdw-define](tdw-define.md) | G005-agent-auth-hooks-tools-and-udf-crates | lib | tdw-hooks | tdw-service-api | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-domain](tdw-domain.md) | G002-core-contracts-event-session-and-replay-crates | lib, test | none | tdw-provider-fileset, tdw-provider-ws-mock, tdw-provider-yahoo, tdw-service-api, tdw-sql-codegen, tdw-test-utils | none | 4 | yes | none | 2 | Ready with follow-ups |
| [tdw-embed](tdw-embed.md) | G004-provider-embedding-and-model-adapter-crates | lib | none | tdw-embed-local, tdw-knowledge, tdw-service-api | none | 1 | no | none | 1 | Ready with follow-ups |
| [tdw-embed-google](tdw-embed-google.md) | G004-provider-embedding-and-model-adapter-crates | lib | tdw-embed | none | none | 1 | no | none | 2 | Ready with follow-ups |
| [tdw-embed-local](tdw-embed-local.md) | G004-provider-embedding-and-model-adapter-crates | lib | tdw-embed | tdw-knowledge, tdw-service-api | none | 1 | no | none | 2 | Ready with follow-ups |
| [tdw-embed-openai](tdw-embed-openai.md) | G004-provider-embedding-and-model-adapter-crates | lib | tdw-embed | none | none | 1 | no | none | 2 | Ready with follow-ups |
| [tdw-entity-resolver](tdw-entity-resolver.md) | G006-knowledge-graph-tags-ml-eval-and-utility-crates | lib | tdw-kg | tdw-service-api | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-eval-runner](tdw-eval-runner.md) | G006-knowledge-graph-tags-ml-eval-and-utility-crates | lib | tdw-agent, tdw-agent-store | tdw-service-api | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-event](tdw-event.md) | G002-core-contracts-event-session-and-replay-crates | lib | none | tdw-actor, tdw-bus, tdw-cdc, tdw-hooks, tdw-outbox, tdw-service-api, xtask | none | 3 | no | none | 1 | Ready with follow-ups |
| [tdw-exec](tdw-exec.md) | G007-client-service-mcp-acp-runtime-and-worker-crates | lib | tdw-protocol | tdw-service-api | none | 2 | no | none | 3 | Ready with follow-ups |
| [tdw-feature-store](tdw-feature-store.md) | G006-knowledge-graph-tags-ml-eval-and-utility-crates | lib | tdw-tags | tdw-service-api | none | 2 | no | none | 2 | Ready with follow-ups |
| [tdw-fn-string](tdw-fn-string.md) | G006-knowledge-graph-tags-ml-eval-and-utility-crates | lib | none | none | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-graph](tdw-graph.md) | G006-knowledge-graph-tags-ml-eval-and-utility-crates | lib | none | tdw-service-api | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-hooks](tdw-hooks.md) | G005-agent-auth-hooks-tools-and-udf-crates | lib | tdw-event, tdw-protocol | tdw-define, tdw-mask, tdw-service-api, tdw-session, tdw-tools | none | 11 | no | none | 4 | Ready with follow-ups |
| [tdw-kg](tdw-kg.md) | G006-knowledge-graph-tags-ml-eval-and-utility-crates | lib | none | tdw-entity-resolver, tdw-knowledge, tdw-service-api | none | 2 | no | none | 1 | Ready with follow-ups |
| [tdw-knowledge](tdw-knowledge.md) | G006-knowledge-graph-tags-ml-eval-and-utility-crates | lib | tdw-core, tdw-embed, tdw-embed-local, tdw-kg, tdw-storage-qdrant, tdw-tags | tdw-service-api | none | 4 | no | none | 4 | Ready with follow-ups |
| [tdw-llm](tdw-llm.md) | G004-provider-embedding-and-model-adapter-crates | lib | tdw-config | tdw-llm-anthropic, tdw-llm-openai-compat, tdw-service-api | none | 1 | no | none | 1 | Ready with follow-ups |
| [tdw-llm-anthropic](tdw-llm-anthropic.md) | G004-provider-embedding-and-model-adapter-crates | lib | tdw-llm | tdw-service-api | none | 1 | no | none | 2 | Ready with follow-ups |
| [tdw-llm-openai-compat](tdw-llm-openai-compat.md) | G004-provider-embedding-and-model-adapter-crates | lib | tdw-llm | tdw-service-api | none | 1 | no | none | 2 | Ready with follow-ups |
| [tdw-mask](tdw-mask.md) | G005-agent-auth-hooks-tools-and-udf-crates | lib | tdw-hooks | tdw-service-api | none | 3 | no | none | 1 | Ready with follow-ups |
| [tdw-mcp](tdw-mcp.md) | G007-client-service-mcp-acp-runtime-and-worker-crates | bin | tdw-service-api | none | none | 0 | no | none | 4 | Ready with follow-ups |
| [tdw-migration](tdw-migration.md) | G003-data-storage-pipeline-and-sql-crates | lib | none | xtask | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-ml-registry](tdw-ml-registry.md) | G006-knowledge-graph-tags-ml-eval-and-utility-crates | lib | none | none | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-outbox](tdw-outbox.md) | G002-core-contracts-event-session-and-replay-crates | lib | tdw-event | tdw-cdc, tdw-service-api | none | 1 | no | none | 0 | Ready with follow-ups |
| [tdw-pipe](tdw-pipe.md) | G003-data-storage-pipeline-and-sql-crates | lib | tdw-stage | tdw-service-api | none | 1 | no | none | 1 | Ready with follow-ups |
| [tdw-pipeline](tdw-pipeline.md) | G003-data-storage-pipeline-and-sql-crates | lib | none | none | none | 2 | no | none | 1 | Ready with follow-ups |
| [tdw-protocol](tdw-protocol.md) | G002-core-contracts-event-session-and-replay-crates | lib | none | tdw-acp, tdw-app-client, tdw-app-server, tdw-exec, tdw-hooks, tdw-replay, tdw-rollout, tdw-service-api, tdw-session, tdw-tools, tdw-tui, xtask | none | 4 | no | none | 7 | Ready with follow-ups |
| [tdw-provider-alpaca](tdw-provider-alpaca.md) | G004-provider-embedding-and-model-adapter-crates | lib | none | none | none | 1 | no | none | 1 | Ready with follow-ups |
| [tdw-provider-binance](tdw-provider-binance.md) | G004-provider-embedding-and-model-adapter-crates | lib | none | none | none | 1 | no | none | 1 | Ready with follow-ups |
| [tdw-provider-fileset](tdw-provider-fileset.md) | G004-provider-embedding-and-model-adapter-crates | lib | tdw-core, tdw-domain | tdw-provider-yahoo, tdw-service-api | none | 2 | no | none | 1 | Ready with follow-ups |
| [tdw-provider-fred](tdw-provider-fred.md) | G004-provider-embedding-and-model-adapter-crates | lib | none | none | none | 1 | no | none | 1 | Ready with follow-ups |
| [tdw-provider-huggingface](tdw-provider-huggingface.md) | G004-provider-embedding-and-model-adapter-crates | lib | none | none | none | 1 | no | none | 1 | Ready with follow-ups |
| [tdw-provider-polygon](tdw-provider-polygon.md) | G004-provider-embedding-and-model-adapter-crates | lib | none | none | none | 1 | no | none | 1 | Ready with follow-ups |
| [tdw-provider-ws-mock](tdw-provider-ws-mock.md) | G004-provider-embedding-and-model-adapter-crates | lib | tdw-core, tdw-domain | tdw-service-api | none | 2 | no | none | 6 | Ready with follow-ups |
| [tdw-provider-yahoo](tdw-provider-yahoo.md) | G004-provider-embedding-and-model-adapter-crates | lib | tdw-core, tdw-domain, tdw-provider-fileset | tdw-service-api | none | 2 | no | none | 4 | Ready with follow-ups |
| [tdw-replay](tdw-replay.md) | G002-core-contracts-event-session-and-replay-crates | lib | tdw-cdc, tdw-protocol, tdw-rollout | tdw-service-api | none | 2 | no | none | 1 | Ready with follow-ups |
| [tdw-rewrite](tdw-rewrite.md) | G006-knowledge-graph-tags-ml-eval-and-utility-crates | lib | none | none | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-rollout](tdw-rollout.md) | G002-core-contracts-event-session-and-replay-crates | lib | tdw-protocol | tdw-replay, tdw-service-api | none | 2 | no | none | 5 | Ready with follow-ups |
| [tdw-runtime](tdw-runtime.md) | G007-client-service-mcp-acp-runtime-and-worker-crates | lib | tdw-core | tdw-service-api | none | 2 | no | none | 4 | Ready with follow-ups |
| [tdw-sandbox](tdw-sandbox.md) | G005-agent-auth-hooks-tools-and-udf-crates | lib | tdw-udf | tdw-service-api | none | 3 | no | none | 1 | Ready with follow-ups |
| [tdw-service](tdw-service.md) | G007-client-service-mcp-acp-runtime-and-worker-crates | bin | tdw-service-api | none | none | 0 | no | none | 2 | Ready with follow-ups |
| [tdw-service-api](tdw-service-api.md) | G007-client-service-mcp-acp-runtime-and-worker-crates | lib | tdw-acp, tdw-actor, tdw-agent, tdw-agent-store, tdw-app-client, tdw-app-server, tdw-auth, tdw-auth-oidc, tdw-bus, tdw-cdc, tdw-config, tdw-core, tdw-define, tdw-domain, tdw-embed, tdw-embed-local, tdw-entity-resolver, tdw-eval-runner, tdw-event, tdw-exec, tdw-feature-store, tdw-graph, tdw-hooks, tdw-kg, tdw-knowledge, tdw-llm, tdw-llm-anthropic, tdw-llm-openai-compat, tdw-mask, tdw-outbox, tdw-pipe, tdw-protocol, tdw-provider-fileset, tdw-provider-ws-mock, tdw-provider-yahoo, tdw-replay, tdw-rollout, tdw-runtime, tdw-sandbox, tdw-snapshot, tdw-spatial, tdw-stage, tdw-storage-meilisearch, tdw-storage-qdrant, tdw-storage-s3, tdw-table-format, tdw-tag-rules, tdw-tags, tdw-tools, tdw-tui, tdw-udf, tdw-workflow-engine | tdw-cli, tdw-mcp, tdw-service, tdw-worker | none | 16 | no | none | 51 | Ready with follow-ups |
| [tdw-session](tdw-session.md) | G002-core-contracts-event-session-and-replay-crates | lib | tdw-core, tdw-hooks, tdw-protocol, tdw-storage-postgres | none | postgres, g013-cross-store | 4 | yes | none | 19 | Ready with follow-ups |
| [tdw-snapshot](tdw-snapshot.md) | G002-core-contracts-event-session-and-replay-crates | lib | none | tdw-service-api | none | 1 | no | none | 0 | Ready with follow-ups |
| [tdw-spatial](tdw-spatial.md) | G006-knowledge-graph-tags-ml-eval-and-utility-crates | lib | none | tdw-service-api | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-sql-codegen](tdw-sql-codegen.md) | G003-data-storage-pipeline-and-sql-crates | lib | tdw-domain | xtask | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-stage](tdw-stage.md) | G003-data-storage-pipeline-and-sql-crates | lib | none | tdw-pipe, tdw-service-api | none | 2 | no | none | 2 | Ready with follow-ups |
| [tdw-storage-clickhouse](tdw-storage-clickhouse.md) | G003-data-storage-pipeline-and-sql-crates | lib | tdw-core | none | none | 2 | no | none | 7 | Ready with follow-ups |
| [tdw-storage-fs](tdw-storage-fs.md) | G003-data-storage-pipeline-and-sql-crates | lib | tdw-core | none | none | 2 | no | none | 2 | Ready with follow-ups |
| [tdw-storage-meilisearch](tdw-storage-meilisearch.md) | G003-data-storage-pipeline-and-sql-crates | lib | tdw-core | tdw-service-api | none | 2 | no | none | 2 | Ready with follow-ups |
| [tdw-storage-parquet](tdw-storage-parquet.md) | G003-data-storage-pipeline-and-sql-crates | lib | none | none | none | 2 | no | none | 4 | Ready with follow-ups |
| [tdw-storage-postgres](tdw-storage-postgres.md) | G003-data-storage-pipeline-and-sql-crates | lib | tdw-core | none | none | 2 | no | none | 7 | Ready with follow-ups |
| [tdw-storage-qdrant](tdw-storage-qdrant.md) | G003-data-storage-pipeline-and-sql-crates | lib | tdw-core | tdw-knowledge, tdw-service-api | none | 2 | no | none | 2 | Ready with follow-ups |
| [tdw-storage-router](tdw-storage-router.md) | G003-data-storage-pipeline-and-sql-crates | lib | tdw-core | none | none | 3 | no | none | 4 | Ready with follow-ups |
| [tdw-storage-s3](tdw-storage-s3.md) | G003-data-storage-pipeline-and-sql-crates | lib | tdw-core | tdw-service-api | none | 3 | no | none | 2 | Ready with follow-ups |
| [tdw-table-format](tdw-table-format.md) | G003-data-storage-pipeline-and-sql-crates | lib | none | tdw-service-api | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-tag-rules](tdw-tag-rules.md) | G006-knowledge-graph-tags-ml-eval-and-utility-crates | lib | tdw-tags | tdw-service-api | none | 2 | no | none | 4 | Ready with follow-ups |
| [tdw-tags](tdw-tags.md) | G006-knowledge-graph-tags-ml-eval-and-utility-crates | lib | none | tdw-feature-store, tdw-knowledge, tdw-service-api, tdw-tag-rules | none | 2 | no | none | 4 | Ready with follow-ups |
| [tdw-test-utils](tdw-test-utils.md) | G006-knowledge-graph-tags-ml-eval-and-utility-crates | lib | tdw-domain | none | default; e2e; integration; property | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-tools](tdw-tools.md) | G005-agent-auth-hooks-tools-and-udf-crates | lib | tdw-hooks, tdw-protocol | tdw-service-api | none | 4 | no | none | 8 | Ready with follow-ups |
| [tdw-tui](tdw-tui.md) | G007-client-service-mcp-acp-runtime-and-worker-crates | lib | tdw-protocol | tdw-service-api | none | 2 | no | none | 2 | Ready with follow-ups |
| [tdw-udf](tdw-udf.md) | G005-agent-auth-hooks-tools-and-udf-crates | lib | none | tdw-sandbox, tdw-service-api | none | 2 | no | none | 1 | Ready with follow-ups |
| [tdw-udf-external](tdw-udf-external.md) | G005-agent-auth-hooks-tools-and-udf-crates | lib | none | none | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-udf-js](tdw-udf-js.md) | G005-agent-auth-hooks-tools-and-udf-crates | lib | none | none | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-udf-python](tdw-udf-python.md) | G005-agent-auth-hooks-tools-and-udf-crates | lib | none | none | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-udf-wasm](tdw-udf-wasm.md) | G005-agent-auth-hooks-tools-and-udf-crates | lib | none | none | none | 2 | no | none | 0 | Ready with follow-ups |
| [tdw-worker](tdw-worker.md) | G007-client-service-mcp-acp-runtime-and-worker-crates | bin | tdw-service-api | none | none | 0 | no | none | 1 | Ready with follow-ups |
| [tdw-workflow-engine](tdw-workflow-engine.md) | G006-knowledge-graph-tags-ml-eval-and-utility-crates | lib | tdw-agent | tdw-service-api | none | 2 | no | none | 1 | Ready with follow-ups |
| [xtask](xtask.md) | G008-aggregate-production-readiness-gate | bin | tdw-agent, tdw-config, tdw-event, tdw-migration, tdw-protocol, tdw-sql-codegen | none | none | 2 | no | none | 8 | Ready with follow-ups |
