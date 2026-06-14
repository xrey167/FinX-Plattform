// Pure parsing of the loosely-typed `options` argument a worksheet passes to
// FINX.GET into a structured SliceOptions. Accepts an object (passed through)
// or a "key=value;key2=value2" string, which is the form a single cell can
// hold. No host dependency.

import type { SliceOptions } from "./types.js";

/** Split a comma/pipe/semicolon-delimited field list into trimmed names. */
function parseFieldList(value: string): string[] {
  return value
    .split(/[,|]/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

/** Parse a boolean-ish token ("true"/"1"/"yes" => true). */
function parseBool(value: string): boolean {
  const v = value.trim().toLowerCase();
  return v === "true" || v === "1" || v === "yes" || v === "y";
}

/** Parse a non-negative integer; NaN/negative => undefined (no cap). */
function parseCount(value: string): number | undefined {
  const n = Number.parseInt(value.trim(), 10);
  return Number.isFinite(n) && n >= 0 ? n : undefined;
}

/**
 * Parse the `options` argument into SliceOptions.
 *
 * Object form is returned shallow-copied. String form understands the keys
 * `fields`, `rows`, `columns`, `header`, `transpose`, e.g.
 *   "fields=date,close;rows=10;transpose=true".
 * Unknown keys are ignored. Empty / null input yields an empty options object.
 */
export function parseOptions(input: unknown): SliceOptions {
  if (input === null || input === undefined || input === "") {
    return {};
  }
  if (typeof input === "object" && !Array.isArray(input)) {
    return { ...(input as SliceOptions) };
  }

  const text = String(input).trim();
  if (text.length === 0) {
    return {};
  }

  const opts: SliceOptions = {};
  for (const pair of text.split(/[;&]/)) {
    const trimmed = pair.trim();
    if (trimmed.length === 0) {
      continue;
    }
    const eq = trimmed.indexOf("=");
    if (eq < 0) {
      continue;
    }
    const key = trimmed.slice(0, eq).trim().toLowerCase();
    const value = trimmed.slice(eq + 1).trim();
    switch (key) {
      case "fields":
      case "field":
        opts.fields = parseFieldList(value);
        break;
      case "rows":
      case "row":
        opts.rows = parseCount(value);
        break;
      case "columns":
      case "cols":
      case "col":
        opts.columns = parseCount(value);
        break;
      case "header":
        opts.header = parseBool(value);
        break;
      case "transpose":
        opts.transpose = parseBool(value);
        break;
      default:
        // Unknown option key: ignore so a typo never aborts the call.
        break;
    }
  }
  return opts;
}
