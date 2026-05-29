-- Representative reference-data seed for the `ref` schema.
--
-- Scope: a handful of countries / currencies / MICs plus one open ICB-style
-- classification tree and a couple of example instruments. This is intentionally
-- SMALL -- enough to exercise FK relations and the ClickHouse dictionaries end
-- to end.
--
-- FOLLOW-UP (documented, not in scope here): bulk-load the full ISO 3166
-- country list, the ISO 4217 currency list, the ISO 10383 MIC registry, and the
-- OpenFIGI instrument universe via the seeder pipeline (see the OpenFIGI mapping
-- parser in crates/tdw-entity-resolver). GICS is licensed -- seed ICB/open now
-- and swap GICS in later by adding a 'GICS' classification_scheme + node tree.

insert into ref.country (code_alpha2, code_alpha3, name) values
  ('US', 'USA', 'United States'),
  ('GB', 'GBR', 'United Kingdom'),
  ('DE', 'DEU', 'Germany')
on conflict (code_alpha2) do nothing;

insert into ref.currency (code, name, minor_units) values
  ('USD', 'US Dollar', 2),
  ('GBP', 'Pound Sterling', 2),
  ('EUR', 'Euro', 2)
on conflict (code) do nothing;

insert into ref.exchange (mic, name, country_alpha2, operating_mic) values
  ('XNAS', 'Nasdaq', 'US', 'XNAS'),
  ('XNYS', 'New York Stock Exchange', 'US', 'XNYS'),
  ('XLON', 'London Stock Exchange', 'GB', 'XLON'),
  ('XETR', 'Deutsche Boerse Xetra', 'DE', 'XETR')
on conflict (mic) do nothing;

insert into ref.issuer (issuer_id, legal_name, country_alpha2) values
  ('ISS-APPLE', 'Apple Inc.', 'US'),
  ('ISS-SAP', 'SAP SE', 'DE')
on conflict (issuer_id) do nothing;

-- Open ICB-style classification tree (Industry -> Supersector -> Sector).
insert into ref.classification_scheme (scheme, description) values
  ('ICB', 'Industry Classification Benchmark (open seed; swap GICS later)')
on conflict (scheme) do nothing;

insert into ref.classification_node (scheme, code, level, name, parent_code) values
  ('ICB', '10', 1, 'Technology', null),
  ('ICB', '1010', 2, 'Technology Hardware & Equipment', '10'),
  ('ICB', '101010', 3, 'Computer Hardware', '1010'),
  ('ICB', '101020', 3, 'Software', '1010')
on conflict (scheme, code) do nothing;

insert into ref.instrument (instrument_id, symbol, mic, currency, issuer_id, asset_class) values
  ('INST-AAPL-XNAS', 'AAPL', 'XNAS', 'USD', 'ISS-APPLE', 'equity'),
  ('INST-SAP-XETR', 'SAP', 'XETR', 'EUR', 'ISS-SAP', 'equity')
on conflict (instrument_id) do nothing;

insert into ref.figi (figi, instrument_id, figi_type) values
  ('BBG000B9XRY4', 'INST-AAPL-XNAS', 'figi'),
  ('BBG000BWRKR8', 'INST-SAP-XETR', 'figi')
on conflict (figi) do nothing;

insert into ref.identifier_xref (instrument_id, scheme, value) values
  ('INST-AAPL-XNAS', 'FIGI', 'BBG000B9XRY4'),
  ('INST-AAPL-XNAS', 'ISIN', 'US0378331005'),
  ('INST-AAPL-XNAS', 'TICKER', 'AAPL'),
  ('INST-SAP-XETR', 'FIGI', 'BBG000BWRKR8'),
  ('INST-SAP-XETR', 'ISIN', 'DE0007164600'),
  ('INST-SAP-XETR', 'TICKER', 'SAP')
on conflict (scheme, value) do nothing;

insert into ref.instrument_classification (instrument_id, scheme, code) values
  ('INST-AAPL-XNAS', 'ICB', '101010'),
  ('INST-SAP-XETR', 'ICB', '101020')
on conflict (instrument_id, scheme) do nothing;
