// Pure backend-config resolution. The Office.js wrappers read the raw values
// out of the host's persisted settings (or localStorage) and hand them here;
// resolveConfig validates and normalizes them into a BackendConfig. Keeping the
// validation pure makes it testable without a host.

import { configError } from "./errors.js";
import type { BackendConfig } from "./types.js";

/** Settings keys the task pane writes and the functions read. */
export const BASE_URL_KEY = "finx:baseUrl";
export const API_KEY_KEY = "finx:apiKey";

/** The default base URL when nothing is configured (a no-op placeholder). */
export const DEFAULT_BASE_URL = "";

/**
 * Validate and normalize raw stored values into a BackendConfig.
 *
 * @throws CellError (FINX#CONFIG) when the base URL is missing or not http(s).
 */
export function resolveConfig(
  rawBaseUrl: unknown,
  rawApiKey: unknown,
): BackendConfig {
  const baseUrl = typeof rawBaseUrl === "string" ? rawBaseUrl.trim() : "";
  if (baseUrl.length === 0) {
    throw configError("base URL is not set — open the FinX task pane to configure it");
  }
  if (!/^https?:\/\//i.test(baseUrl)) {
    throw configError(`base URL must start with http(s)://, got "${baseUrl}"`);
  }
  const apiKey = typeof rawApiKey === "string" && rawApiKey.trim().length > 0 ? rawApiKey.trim() : undefined;
  return apiKey === undefined ? { baseUrl } : { baseUrl, apiKey };
}
