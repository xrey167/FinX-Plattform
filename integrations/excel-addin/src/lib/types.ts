// Shared, framework-free types for the FinX Excel add-in pure library.
//
// Nothing here touches the Office.js / Excel runtime: every type models the
// FinX platform REST contract or the slicing options, so the whole module is
// unit-testable under Node without a host application.

/** A JSON scalar a query parameter can hold. */
export type ParamScalar = string | number | boolean;

/** A bag of query parameters keyed by name (the `?k=v&...` of a REST call). */
export type ParamMap = Record<string, ParamScalar | null | undefined>;

/**
 * One standardized record as emitted in the REST envelope's `results` array.
 * Values are JSON-ish; objects/arrays are flattened to a display string when
 * placed on the grid.
 */
export type ResultRecord = Record<string, unknown>;

/**
 * The `ResultEnvelope`-shaped JSON body the daemon returns from
 * `GET /api/v1/{route}`. Only the fields the add-in consumes are modeled; the
 * envelope may carry more (`id`, `provider`, `extra`, `chart`).
 */
export interface ResultEnvelope {
  /** Stable identifier / correlation id for the result. */
  id?: string;
  /** The standardized records produced for the request — the grid rows. */
  results?: ResultRecord[];
  /** The provider key that served the request (e.g. "yahoo", "fmp"). */
  provider?: string | null;
  /** Non-fatal warnings accumulated while producing the result. */
  warnings?: unknown[];
  /** Route / timestamp / arguments provenance. */
  extra?: unknown;
}

/**
 * Slicing / shaping options accepted by FINX.GET (and FINX.BYOD). All optional;
 * an omitted field means "no slicing on that axis".
 */
export interface SliceOptions {
  /**
   * Restrict the output columns to these field names, in this order. Names not
   * present in the data are emitted as empty columns so the caller's layout is
   * stable.
   */
  fields?: string[];
  /** Keep only the first `rows` data rows (after any field projection). */
  rows?: number;
  /** Keep only the first `columns` columns (after field projection). */
  columns?: number;
  /** When false, omit the header row of field names. Defaults to true. */
  header?: boolean;
  /**
   * Transpose the grid (fields become the first column, records become the
   * subsequent columns). Useful for a single record laid out vertically.
   */
  transpose?: boolean;
}

/** A 2-D array of cell values ready to hand back to Excel. */
export type Grid = (string | number | boolean)[][];

/** Configuration the custom functions read to reach the backend. */
export interface BackendConfig {
  /** Base URL of the FinX REST API, e.g. "https://api.example.com". */
  baseUrl: string;
  /** Optional bearer API key sent as `Authorization: Bearer <key>`. */
  apiKey?: string;
}
