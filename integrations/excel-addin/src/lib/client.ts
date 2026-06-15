// Pure REST client orchestration for the FinX add-in.
//
// `fetchRoute` builds the URL, performs the GET via an injectable fetch
// implementation (the real `globalThis.fetch` in the host, a stub in tests),
// maps non-2xx responses to a thrown CellError, and returns the parsed JSON
// body. Flattening to a grid is the caller's job (see flatten.ts) so this stays
// transport-only and host-free.

import { httpStatusToError } from "./errors.js";
import { buildUrl } from "./url.js";
import type { BackendConfig, ParamMap } from "./types.js";

/** The minimal subset of the Fetch API this client depends on. */
export type FetchLike = (
  url: string,
  init?: {
    method?: string;
    headers?: Record<string, string>;
  },
) => Promise<FetchResponseLike>;

/** The minimal Response shape the client reads. */
export interface FetchResponseLike {
  ok: boolean;
  status: number;
  text(): Promise<string>;
}

/** Build the request headers, attaching a bearer token when configured. */
export function buildHeaders(config: BackendConfig): Record<string, string> {
  const headers: Record<string, string> = { Accept: "application/json" };
  if (config.apiKey && config.apiKey.trim().length > 0) {
    headers["Authorization"] = `Bearer ${config.apiKey.trim()}`;
  }
  return headers;
}

/**
 * Fetch a catalog route and return the parsed JSON body.
 *
 * @throws CellError (`{ code, message }`) on a non-2xx response, mapped via
 *   {@link httpStatusToError}. URL/config errors from {@link buildUrl}
 *   propagate as plain `Error`s for the wrapper's {@link toCellError} to
 *   normalize.
 */
export async function fetchRoute(
  fetchImpl: FetchLike,
  config: BackendConfig,
  route: string,
  params?: ParamMap,
): Promise<unknown> {
  const url = buildUrl(config.baseUrl, route, params);
  const response = await fetchImpl(url, {
    method: "GET",
    headers: buildHeaders(config),
  });
  if (!response.ok) {
    const body = await response.text();
    throw httpStatusToError(response.status, body);
  }
  const text = await response.text();
  if (text.trim().length === 0) {
    return {};
  }
  return JSON.parse(text);
}
