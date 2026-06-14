// Host-agnostic key/value storage. The task pane and (indirectly) the custom
// functions persist the base URL / API key through here. Inside the Office host
// the shared `OfficeRuntime.storage` is used; in a plain browser or a Node/
// vitest harness `OfficeRuntime` is undefined, so we fall back to `localStorage`
// and finally to a no-op/null. Guarding the globals keeps this importable
// without the Office runtime (it must never throw a ReferenceError).

/* global OfficeRuntime */

/** Read a value for `key`, or null when nothing is stored / no store exists. */
export async function getStorageItem(key: string): Promise<string | null> {
  if (typeof OfficeRuntime !== "undefined" && OfficeRuntime.storage) {
    return OfficeRuntime.storage.getItem(key);
  }
  if (typeof localStorage !== "undefined") {
    return localStorage.getItem(key);
  }
  return null;
}

/** Persist `value` under `key`; a no-op when no store is available. */
export async function setStorageItem(key: string, value: string): Promise<void> {
  if (typeof OfficeRuntime !== "undefined" && OfficeRuntime.storage) {
    await OfficeRuntime.storage.setItem(key, value);
    return;
  }
  if (typeof localStorage !== "undefined") {
    localStorage.setItem(key, value);
  }
}
