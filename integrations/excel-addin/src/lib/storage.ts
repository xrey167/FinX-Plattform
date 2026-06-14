// Host-agnostic key/value storage. The task pane and (indirectly) the custom
// functions persist the base URL / API key through here. Inside the Office host
// the shared `OfficeRuntime.storage` is used; in a plain browser or a Node/
// vitest harness `OfficeRuntime` is undefined, so we fall back to `localStorage`
// and finally to a no-op/null. Guarding the globals keeps this importable
// without the Office runtime (it must never throw a ReferenceError).
//
// Every access is additionally wrapped in try/catch: in a restricted host (e.g.
// Excel on the web inside an iframe with third-party storage blocked) even
// touching `localStorage`, or an `OfficeRuntime.storage` promise rejection, can
// throw a `SecurityError`. Persistence here is best-effort — a store failure
// degrades to "nothing stored" / a no-op rather than crashing the caller, and
// the `OfficeRuntime` promise is awaited inside the guard so a rejection is
// caught locally instead of escaping as an unhandled rejection.

/* global OfficeRuntime */

/** Read a value for `key`, or null when nothing is stored / no store exists. */
export async function getStorageItem(key: string): Promise<string | null> {
  try {
    if (typeof OfficeRuntime !== "undefined" && OfficeRuntime.storage) {
      return await OfficeRuntime.storage.getItem(key);
    }
  } catch {
    // OfficeRuntime store unavailable/blocked — fall through to localStorage.
  }
  try {
    if (typeof localStorage !== "undefined") {
      return localStorage.getItem(key);
    }
  } catch {
    // Storage blocked (e.g. third-party iframe) — treat as nothing stored.
  }
  return null;
}

/** Persist `value` under `key`; a no-op when no store is available. */
export async function setStorageItem(key: string, value: string): Promise<void> {
  try {
    if (typeof OfficeRuntime !== "undefined" && OfficeRuntime.storage) {
      await OfficeRuntime.storage.setItem(key, value);
      return;
    }
  } catch {
    // OfficeRuntime store unavailable/blocked — fall through to localStorage.
  }
  try {
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(key, value);
    }
  } catch {
    // Storage blocked — persistence is best-effort, so silently degrade.
  }
}
