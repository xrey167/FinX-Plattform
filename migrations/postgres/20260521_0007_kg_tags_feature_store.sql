create table if not exists system.kg_entity (
  entity_id text primary key,
  kind text not null,
  label text not null,
  aliases jsonb not null default '[]'::jsonb
);

create table if not exists system.kg_relationship (
  source_id text not null references system.kg_entity(entity_id),
  target_id text not null references system.kg_entity(entity_id),
  rel_type text not null,
  provenance text not null,
  primary key (source_id, target_id, rel_type)
);

create table if not exists system.kg_merge_audit (
  source_id text not null,
  target_id text not null,
  approved_by text not null,
  approved_at timestamptz not null default now(),
  primary key (source_id, target_id, approved_by)
);

create table if not exists system.tag_definition (
  tag_id text primary key,
  parent_tag_id text references system.tag_definition(tag_id),
  ttl_days integer
);

create table if not exists system.tag_assignment (
  entity_id text not null,
  tag_id text not null references system.tag_definition(tag_id),
  assigned_at timestamptz not null,
  expires_at timestamptz,
  provenance text not null,
  primary key (entity_id, tag_id, provenance)
);

create table if not exists system.tag_rule (
  rule_id text primary key,
  tag_id text not null references system.tag_definition(tag_id),
  predicate jsonb not null,
  version bigint not null default 1,
  enabled boolean not null default true
);

create table if not exists system.feature_snapshot (
  entity_id text not null,
  as_of date not null,
  features jsonb not null,
  tags jsonb not null,
  primary key (entity_id, as_of)
);
