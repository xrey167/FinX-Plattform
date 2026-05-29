-- Reference-entity master model (the `ref` schema).
--
-- This is the system-of-record for security master / reference data that the
-- ClickHouse `analytics.dict_*` dictionaries pull from for dictGet enrichment
-- (see migrations/clickhouse/20260528_0005_reference_dictionaries.sql).
--
-- Entity graph (parent -> child references):
--
--   country  <--- exchange.country_alpha2
--   country  <--- issuer.country_alpha2
--   currency <--- instrument.currency
--   exchange <--- instrument.mic
--   issuer   <--- instrument.issuer_id
--   instrument <--- figi.instrument_id
--   instrument <--- identifier_xref.instrument_id        (FIGI/ISIN/CUSIP/SEDOL/ticker crosswalk)
--   instrument <--- instrument_classification.instrument_id
--   classification_scheme <--- classification_node.scheme
--   classification_node   <--- instrument_classification(scheme, code)   (pluggable GICS-shaped hierarchy)
--
-- Licensing note: GICS (the S&P / MSCI sector taxonomy) is licensed and MUST
-- NOT be redistributed in seed data. The classification model here is scheme-
-- agnostic on purpose: seed an open hierarchy now (ICB-style is used in the
-- seed file) and swap in a licensed GICS tree later by inserting a new
-- classification_scheme row plus its classification_node tree -- no DDL change
-- required.

create schema if not exists ref;

create table if not exists ref.country (
  code_alpha2 char(2) primary key,
  code_alpha3 char(3),
  name text not null
);

create table if not exists ref.currency (
  code char(3) primary key,
  name text not null,
  minor_units smallint
);

create table if not exists ref.exchange (
  mic char(4) primary key,
  name text,
  country_alpha2 char(2) references ref.country(code_alpha2),
  operating_mic char(4)
);

create table if not exists ref.issuer (
  issuer_id text primary key,
  legal_name text not null,
  country_alpha2 char(2) references ref.country(code_alpha2)
);

create table if not exists ref.instrument (
  instrument_id text primary key,
  symbol text not null,
  mic char(4) references ref.exchange(mic),
  currency char(3) references ref.currency(code),
  issuer_id text references ref.issuer(issuer_id),
  asset_class text
);

-- composite / share-class / FIGI are distinguished by figi_type
-- (e.g. 'figi', 'composite', 'share_class').
create table if not exists ref.figi (
  figi char(12) primary key,
  instrument_id text references ref.instrument(instrument_id),
  figi_type text
);

-- FIGI/ISIN/CUSIP/SEDOL/ticker crosswalk. The (scheme, value) pair is globally
-- unique so a single identifier maps to exactly one instrument.
create table if not exists ref.identifier_xref (
  instrument_id text references ref.instrument(instrument_id),
  scheme text,
  value text,
  primary key (scheme, value)
);

-- Classification scheme registry, e.g. 'GICS', 'ICB', 'NAICS'.
create table if not exists ref.classification_scheme (
  scheme text primary key,
  description text
);

-- Pluggable GICS-shaped hierarchy: each node belongs to a scheme and may point
-- at a parent node within the same scheme via parent_code.
create table if not exists ref.classification_node (
  scheme text references ref.classification_scheme(scheme),
  code text,
  level smallint,
  name text,
  parent_code text,
  primary key (scheme, code)
);

create table if not exists ref.instrument_classification (
  instrument_id text references ref.instrument(instrument_id),
  scheme text,
  code text,
  primary key (instrument_id, scheme),
  foreign key (scheme, code) references ref.classification_node(scheme, code)
);

-- ---------------------------------------------------------------------------
-- Audit columns on every ref.* table. Added via `add column if not exists` so
-- this block is safe to re-run and leaves the create statements above intact.
-- `default now()` covers initial load; application writes (or a trigger) should
-- bump updated_at on change.
-- ---------------------------------------------------------------------------
alter table ref.country                   add column if not exists created_at timestamptz not null default now();
alter table ref.country                   add column if not exists updated_at timestamptz not null default now();
alter table ref.currency                  add column if not exists created_at timestamptz not null default now();
alter table ref.currency                  add column if not exists updated_at timestamptz not null default now();
alter table ref.exchange                  add column if not exists created_at timestamptz not null default now();
alter table ref.exchange                  add column if not exists updated_at timestamptz not null default now();
alter table ref.issuer                    add column if not exists created_at timestamptz not null default now();
alter table ref.issuer                    add column if not exists updated_at timestamptz not null default now();
alter table ref.instrument                add column if not exists created_at timestamptz not null default now();
alter table ref.instrument                add column if not exists updated_at timestamptz not null default now();
alter table ref.figi                      add column if not exists created_at timestamptz not null default now();
alter table ref.figi                      add column if not exists updated_at timestamptz not null default now();
alter table ref.identifier_xref           add column if not exists created_at timestamptz not null default now();
alter table ref.identifier_xref           add column if not exists updated_at timestamptz not null default now();
alter table ref.classification_scheme     add column if not exists created_at timestamptz not null default now();
alter table ref.classification_scheme     add column if not exists updated_at timestamptz not null default now();
alter table ref.classification_node       add column if not exists created_at timestamptz not null default now();
alter table ref.classification_node       add column if not exists updated_at timestamptz not null default now();
alter table ref.instrument_classification add column if not exists created_at timestamptz not null default now();
alter table ref.instrument_classification add column if not exists updated_at timestamptz not null default now();

-- ---------------------------------------------------------------------------
-- Secondary indexes for the hot lookup paths (PKs already cover the rest).
-- ---------------------------------------------------------------------------
create index if not exists idx_instrument_symbol     on ref.instrument (symbol);
create index if not exists idx_instrument_issuer      on ref.instrument (issuer_id);
create index if not exists idx_instrument_mic         on ref.instrument (mic);
create index if not exists idx_xref_instrument        on ref.identifier_xref (instrument_id);
create index if not exists idx_classnode_parent       on ref.classification_node (scheme, parent_code);
create index if not exists idx_instclass_scheme_code  on ref.instrument_classification (scheme, code);
create index if not exists idx_figi_instrument        on ref.figi (instrument_id);

-- ---------------------------------------------------------------------------
-- Enriched instrument view = the SOURCE for the ClickHouse dictionary
-- `analytics.dict_instrument_ref` (migrations/clickhouse/20260528_0005...).
-- The dictionary declares a `sector_code` attribute, but ref.instrument has no
-- such column, so pointing the dictionary straight at ref.instrument would fail
-- to load. This view supplies symbol + mic + currency + issuer_id + sector_code
-- in exactly one row per instrument.
--
-- An instrument may be classified under several schemes
-- (instrument_classification PK is (instrument_id, scheme)). To keep one row per
-- instrument we pick a single classification deterministically with DISTINCT ON,
-- preferring the scheme that sorts first. Change the ORDER BY to pin a specific
-- primary scheme (e.g. 'GICS') once a licensed tree is loaded. Finer
-- sector/industry levels are resolved downstream by walking
-- analytics.dict_classification.parent_code.
--
-- Dictionary key note: dict_instrument_ref keys on `symbol`, which assumes
-- symbol is unique in the active universe (true for single-listing equities).
-- For multi-listed symbols, key the dictionary on (symbol, mic) instead.
-- ---------------------------------------------------------------------------
create or replace view ref.v_instrument_enriched as
select distinct on (i.instrument_id)
  i.instrument_id,
  i.symbol,
  i.mic,
  i.currency,
  i.issuer_id,
  i.asset_class,
  ic.scheme as sector_scheme,
  ic.code   as sector_code
from ref.instrument i
left join ref.instrument_classification ic
  on ic.instrument_id = i.instrument_id
order by i.instrument_id, ic.scheme;
