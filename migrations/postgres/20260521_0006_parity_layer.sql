create table if not exists system.snapshot_version (
  table_name text not null,
  version bigint not null,
  created_at timestamptz not null,
  row_ids jsonb not null,
  primary key (table_name, version)
);

create table if not exists system.stage_location (
  stage_name text primary key,
  uri text not null
);

create table if not exists system.pipe_definition (
  pipe_name text primary key,
  stage_name text not null references system.stage_location(stage_name),
  target_table text not null,
  last_offset bigint not null default 0
);

create table if not exists system.table_manifest (
  table_name text not null,
  version bigint not null,
  table_format text not null,
  files jsonb not null,
  primary key (table_name, version, table_format)
);

create table if not exists system.udf_definition (
  udf_name text primary key,
  runtime text not null,
  source text not null,
  allow_network boolean not null default false,
  allow_filesystem boolean not null default false
);

create table if not exists system.auth_policy (
  table_name text primary key,
  required_role text not null,
  row_filter text
);

create table if not exists system.mask_rule (
  field_name text primary key,
  mode text not null
);

create table if not exists system.define_statement (
  idempotency_key text primary key,
  statement_kind text not null,
  payload jsonb not null
);

create table if not exists system.graph_edge (
  source_id text not null,
  target_id text not null,
  primary key (source_id, target_id)
);

create table if not exists system.spatial_box (
  box_id text primary key,
  min_lat double precision not null,
  min_lon double precision not null,
  max_lat double precision not null,
  max_lon double precision not null
);
