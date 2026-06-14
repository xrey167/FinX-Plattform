// Office.js custom functions for the FinX Excel add-in.
//
// These wrappers are intentionally THIN: every piece of real logic (URL
// building, fetching, JSON->grid flattening, slicing, error mapping, config
// resolution) lives in `../lib` and is unit-tested there without the Excel
// runtime. Each function here just (1) resolves config, (2) parses the loose
// worksheet args, (3) delegates to the library, and (4) maps any failure to a
// readable cell string.
//
// Built only against the public Office.js custom-functions API and the FinX
// platform's own REST contract — no third-party add-in source is used and no
// `OBB.`-named functions are defined (that namespace belongs to OpenBB).

/* global CustomFunctions, OfficeRuntime */

import {
  API_KEY_KEY,
  BASE_URL_KEY,
  coerceParams,
  fetchRoute,
  flattenToGrid,
  formatCellError,
  type Grid,
  parseOptions,
  resolveConfig,
  toCellError,
} from "../lib/index.js";

/** A grid the host can always render: pure values, never a rejected promise. */
type CellGrid = (string | number | boolean)[][];

/**
 * Read the persisted base URL / API key from the host settings store and build
 * a validated BackendConfig. Uses `OfficeRuntime.storage` when present (the
 * shared custom-functions key/value store), falling back to `localStorage` for
 * non-host test harnesses.
 */
async function loadConfig() {
  let baseUrl: string | null = null;
  let apiKey: string | null = null;
  if (typeof OfficeRuntime !== "undefined" && OfficeRuntime.storage) {
    baseUrl = await OfficeRuntime.storage.getItem(BASE_URL_KEY);
    apiKey = await OfficeRuntime.storage.getItem(API_KEY_KEY);
  } else if (typeof localStorage !== "undefined") {
    baseUrl = localStorage.getItem(BASE_URL_KEY);
    apiKey = localStorage.getItem(API_KEY_KEY);
  }
  return resolveConfig(baseUrl, apiKey);
}

/** The real host fetch, adapted to the FetchLike shape the client expects. */
function hostFetch() {
  return (url: string, init?: { method?: string; headers?: Record<string, string> }) =>
    fetch(url, init);
}

/**
 * FINX.GET — fetch a FinX catalog route and spill it as a 2-D grid.
 *
 * The OBB.GET-equivalent over the FinX REST API: calls
 * `GET {baseUrl}/api/v1/{route}?{params}` and flattens the response's
 * `results` records to rows x cols. `options` slices the grid
 * (fields/rows/columns/header/transpose).
 *
 * @customfunction FINX.GET
 * @param route The catalog route, e.g. "equity/price/historical".
 * @param params Query parameters as "k=v;k2=v2" or an object, e.g. "symbol=AAPL".
 * @param options Optional slicing, e.g. "fields=date,close;rows=30".
 * @returns A 2-D array for the Excel grid.
 */
export async function get(
  route: string,
  params?: string,
  options?: string,
): Promise<CellGrid> {
  try {
    const config = await loadConfig();
    const body = await fetchRoute(hostFetch(), config, route, coerceParams(params));
    return flattenToGrid(body, parseOptions(options)) as Grid;
  } catch (err) {
    return [[formatCellError(toCellError(err))]];
  }
}

/**
 * FINX.BYOD — bring-your-own-data: fetch from an explicit base URL + route,
 * bypassing the persisted config so a sheet can pull from an arbitrary
 * configured FinX-compatible backend. Same slicing logic as FINX.GET.
 *
 * @customfunction FINX.BYOD
 * @param baseUrl The backend base URL, e.g. "https://my-finx.example.com".
 * @param route The catalog route, e.g. "equity/price/historical".
 * @param params Query parameters as "k=v;k2=v2" or an object.
 * @param options Optional slicing, e.g. "fields=date,close;rows=30".
 * @returns A 2-D array for the Excel grid.
 */
export async function byod(
  baseUrl: string,
  route: string,
  params?: string,
  options?: string,
): Promise<CellGrid> {
  try {
    const config = resolveConfig(baseUrl, undefined);
    const body = await fetchRoute(hostFetch(), config, route, coerceParams(params));
    return flattenToGrid(body, parseOptions(options)) as Grid;
  } catch (err) {
    return [[formatCellError(toCellError(err))]];
  }
}

/**
 * FINX.ROUTES — list catalog routes the backend exposes, one per row. Calls the
 * conventional discovery route `meta/routes`; if the backend does not implement
 * it the cell shows the mapped error rather than throwing.
 *
 * @customfunction FINX.ROUTES
 * @returns A single-column 2-D array of route strings.
 */
export async function routes(): Promise<CellGrid> {
  try {
    const config = await loadConfig();
    const body = await fetchRoute(hostFetch(), config, "meta/routes");
    // The discovery route returns records like { route: "..." }; project that
    // one field down to a single column.
    return flattenToGrid(body, { fields: ["route"], header: false }) as Grid;
  } catch (err) {
    return [[formatCellError(toCellError(err))]];
  }
}

// Register the functions with the host when the custom-functions runtime is
// available (it is not, in a unit-test/Node context — guarded so importing this
// module for type-checking never throws).
if (typeof CustomFunctions !== "undefined") {
  CustomFunctions.associate("GET", get);
  CustomFunctions.associate("BYOD", byod);
  CustomFunctions.associate("ROUTES", routes);
}
