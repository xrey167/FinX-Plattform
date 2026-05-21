create table if not exists raw.eval_run (
  run_id text primary key,
  agent_id text not null,
  dataset_id text not null,
  metric_name text not null,
  metric_value double precision not null,
  started_at timestamptz not null,
  finished_at timestamptz
);

create table if not exists evals.run_step (
  run_id text not null,
  node_id text not null,
  status text not null,
  execution_time_seconds double precision not null
);
