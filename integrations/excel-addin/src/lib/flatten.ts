// Pure JSON-envelope -> 2-D grid flattening and slicing.
//
// Converts the FinX REST `ResultEnvelope.results` array of records into the
// rows x cols array Excel expects, with field projection, row/column limits,
// header toggling and transposition. No Office.js dependency: all of it runs
// and is tested under plain Node.

import type {
  Grid,
  ResultEnvelope,
  ResultRecord,
  SliceOptions,
} from "./types.js";

/** The cell value used for a missing / null field. */
export const EMPTY_CELL = "";

/**
 * Coerce an arbitrary JSON value into a single Excel cell value. Scalars pass
 * through; null/undefined become an empty string; objects and arrays are
 * JSON-stringified so a nested structure still lands in one cell rather than
 * throwing.
 */
export function toCell(value: unknown): string | number | boolean {
  if (value === null || value === undefined) {
    return EMPTY_CELL;
  }
  const t = typeof value;
  if (t === "number") {
    return Number.isFinite(value as number) ? (value as number) : EMPTY_CELL;
  }
  if (t === "boolean") {
    return value as boolean;
  }
  if (t === "string") {
    return value as string;
  }
  // Object / array: stringify into one cell.
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

/**
 * Pull the record array out of a parsed REST response. Accepts the full
 * envelope (`{ results: [...] }`), a bare array of records, or a single record
 * object (wrapped to a one-element array). Anything else yields an empty list.
 */
export function extractRecords(body: unknown): ResultRecord[] {
  if (Array.isArray(body)) {
    return body as ResultRecord[];
  }
  if (body && typeof body === "object") {
    const env = body as ResultEnvelope;
    if (Array.isArray(env.results)) {
      return env.results;
    }
    // A non-envelope object with no `results`: treat it as a single record.
    if (!("results" in (body as object))) {
      return [body as ResultRecord];
    }
  }
  return [];
}

/**
 * Determine the ordered column field names. When `fields` is supplied it wins
 * verbatim (stable layout, even for absent fields). Otherwise the union of keys
 * across all records is used, preserving first-seen order.
 */
export function resolveFields(
  records: ResultRecord[],
  fields?: string[],
): string[] {
  if (fields && fields.length > 0) {
    return fields.slice();
  }
  const seen = new Set<string>();
  const order: string[] = [];
  for (const rec of records) {
    if (rec && typeof rec === "object") {
      for (const key of Object.keys(rec)) {
        if (!seen.has(key)) {
          seen.add(key);
          order.push(key);
        }
      }
    }
  }
  return order;
}

/** Apply a non-negative integer cap to a length; undefined/invalid = no cap. */
function cap(length: number, limit: number | undefined): number {
  if (limit === undefined || !Number.isFinite(limit) || limit < 0) {
    return length;
  }
  return Math.min(length, Math.floor(limit));
}

/** Transpose a rectangular grid (assumes uniform row width; empty -> empty). */
export function transpose(grid: Grid): Grid {
  if (grid.length === 0) {
    return [];
  }
  const width = grid[0].length;
  const out: Grid = [];
  for (let c = 0; c < width; c++) {
    const row: (string | number | boolean)[] = [];
    for (const r of grid) {
      row.push(r[c] ?? EMPTY_CELL);
    }
    out.push(row);
  }
  return out;
}

/**
 * Flatten a REST response body into a 2-D grid with optional slicing.
 *
 * Layout: an optional header row of field names, then one row per record. Field
 * projection, row and column caps, header suppression and transposition are all
 * honored. An empty record set yields a single-cell `[["(no data)"]]` grid so a
 * spilled formula never collapses to a confusing blank.
 */
export function flattenToGrid(body: unknown, options: SliceOptions = {}): Grid {
  const records = extractRecords(body);
  let fields = resolveFields(records, options.fields);

  // Column cap applies to the field list before we materialize cells.
  fields = fields.slice(0, cap(fields.length, options.columns));

  if (records.length === 0 || fields.length === 0) {
    return [["(no data)"]];
  }

  const rowLimit = cap(records.length, options.rows);
  const withHeader = options.header !== false;

  const grid: Grid = [];
  if (withHeader) {
    grid.push(fields.map((f) => f));
  }
  for (let i = 0; i < rowLimit; i++) {
    const rec = records[i] ?? {};
    grid.push(fields.map((f) => toCell((rec as ResultRecord)[f])));
  }

  return options.transpose ? transpose(grid) : grid;
}
