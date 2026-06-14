// Pure URL + query-string construction for FinX REST calls.
//
// Mirrors the server contract (`crates/tdw-app-server/src/rest_route.rs`):
//   GET {baseUrl}/api/v1/{route}?{k=v&...}
// where each value is percent-encoded and scalars are stringified. No Office.js
// or network dependency, so it is fully unit-testable.

import type { ParamMap, ParamScalar } from "./types.js";

/** The fixed REST API prefix the server mounts catalog routes under. */
export const API_PREFIX = "/api/v1/";

/**
 * Normalize a user-supplied route into a clean catalog path segment:
 * trim whitespace, strip a leading/trailing slash, and strip an accidentally
 * included `api/v1/` prefix so both "equity/price/historical" and
 * "/api/v1/equity/price/historical" resolve identically.
 */
export function normalizeRoute(route: string): string {
  let r = (route ?? "").trim();
  r = r.replace(/^\/+/, "").replace(/\/+$/, "");
  // Strip an accidental api/v1 prefix (with or without a leading slash).
  r = r.replace(/^api\/v1\//i, "");
  return r;
}

/** Stringify a scalar param value the way the server's query parser expects. */
function scalarToString(value: ParamScalar): string {
  if (typeof value === "boolean") {
    return value ? "true" : "false";
  }
  return String(value);
}

/**
 * Encode a parameter map to a sorted, percent-encoded query string (without a
 * leading `?`). Null/undefined values are skipped. Keys are sorted so the same
 * params always produce the same string (stable, cache-friendly, testable).
 */
export function encodeParams(params: ParamMap | undefined): string {
  if (!params) {
    return "";
  }
  const parts: string[] = [];
  for (const key of Object.keys(params).sort()) {
    const value = params[key];
    if (value === null || value === undefined) {
      continue;
    }
    parts.push(`${encodeURIComponent(key)}=${encodeURIComponent(scalarToString(value))}`);
  }
  return parts.join("&");
}

/**
 * Build the full request URL for a catalog route.
 *
 * @throws Error when `baseUrl` is empty or `route` normalizes to empty.
 */
export function buildUrl(baseUrl: string, route: string, params?: ParamMap): string {
  const base = (baseUrl ?? "").trim().replace(/\/+$/, "");
  if (base.length === 0) {
    throw new Error("FinX: base URL is not configured");
  }
  const normalized = normalizeRoute(route);
  if (normalized.length === 0) {
    throw new Error("FinX: route is empty");
  }
  const query = encodeParams(params);
  const path = `${base}${API_PREFIX}${normalized}`;
  return query.length > 0 ? `${path}?${query}` : path;
}

/**
 * Parse a loosely-typed params argument coming from a worksheet cell into a
 * ParamMap. Accepts:
 *   - an already-structured object (returned as-is),
 *   - a `"k=v;k2=v2"` or `"k=v&k2=v2"` string (the spreadsheet-friendly form),
 *   - empty / null (=> empty map).
 * Values are kept as strings; the server coerces numeric/boolean scalars.
 */
export function coerceParams(input: unknown): ParamMap {
  if (input === null || input === undefined || input === "") {
    return {};
  }
  if (typeof input === "object" && !Array.isArray(input)) {
    return input as ParamMap;
  }
  const text = String(input).trim();
  if (text.length === 0) {
    return {};
  }
  const map: ParamMap = {};
  for (const pair of text.split(/[;&]/)) {
    const trimmed = pair.trim();
    if (trimmed.length === 0) {
      continue;
    }
    const eq = trimmed.indexOf("=");
    if (eq < 0) {
      map[trimmed] = "";
      continue;
    }
    const key = trimmed.slice(0, eq).trim();
    const value = trimmed.slice(eq + 1).trim();
    if (key.length > 0) {
      map[key] = value;
    }
  }
  return map;
}
