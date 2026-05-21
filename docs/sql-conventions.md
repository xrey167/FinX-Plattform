# SQL Conventions

- Raw tables live in `raw`.
- dbt silver models live in `staging`.
- dbt gold models live in `analytics` and `marts`.
- Agent metadata lives in `agents`; eval and observability data lives in `evals`.
- System lineage lives in `system`.
- Generated DDL must remain idempotent and use `create ... if not exists`.
