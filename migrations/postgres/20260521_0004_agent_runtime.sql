create table if not exists agents.agent_card (
  agent_id text primary key,
  name text not null,
  version text not null,
  description text not null,
  endpoint text,
  payload jsonb not null,
  updated_at timestamptz not null default now()
);

create table if not exists agents.agent_skill (
  skill_id text primary key,
  agent_id text not null references agents.agent_card(agent_id),
  name text not null,
  description text not null,
  input_schema jsonb not null,
  output_schema jsonb not null,
  tags text[] not null default '{}'
);

create table if not exists agents.slash_command (
  command_name text primary key,
  workflow_id text,
  description text not null,
  args jsonb not null default '[]'::jsonb
);

create table if not exists agents.content_ref (
  uri text primary key,
  kind text not null,
  checksum text,
  tags text[] not null default '{}'
);

create table if not exists agents.workflow_definition (
  workflow_id text primary key,
  payload jsonb not null,
  updated_at timestamptz not null default now()
);

create table if not exists agents.gotcha (
  gotcha_id text primary key,
  title text not null,
  severity text not null,
  applies_to text[] not null default '{}',
  remediation text not null,
  source_ref_uri text references agents.content_ref(uri)
);

create table if not exists evals.eval_case (
  dataset_id text not null,
  case_id text not null,
  prompt text not null,
  expected_refs jsonb not null default '[]'::jsonb,
  primary key (dataset_id, case_id)
);

create table if not exists evals.eval_metric (
  run_id text not null references raw.eval_run(run_id),
  metric_name text not null,
  metric_value double precision not null,
  primary key (run_id, metric_name)
);
