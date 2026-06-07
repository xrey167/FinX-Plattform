create schema if not exists system;

create table if not exists system.function_runs (
  run_id       text primary key,
  function_id  text not null,
  payload      jsonb not null,
  status       text not null check (status in ('Running', 'Completed', 'Failed')),
  result       jsonb,
  created_at_ms bigint not null,
  updated_at_ms bigint not null
);

create index if not exists idx_function_runs_function_id
  on system.function_runs (function_id, created_at_ms desc);

create index if not exists idx_function_runs_status
  on system.function_runs (status);

create table if not exists system.function_steps (
  run_id        text not null,
  step_name     text not null,
  result_json   jsonb not null,
  created_at_ms bigint not null,
  primary key (run_id, step_name)
);

create index if not exists idx_function_steps_run_id
  on system.function_steps (run_id);
