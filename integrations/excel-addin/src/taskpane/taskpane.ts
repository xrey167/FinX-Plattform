// Task-pane controller: persist the base URL / API key into the shared
// OfficeRuntime storage the custom functions read. All validation lives in the
// pure `resolveConfig`, and storage access is funnelled through the guarded
// `getStorageItem`/`setStorageItem` helpers, so this file only wires DOM events
// to storage and never touches `OfficeRuntime` directly (which is undefined
// outside the Office host).

/* global Office */

import {
  API_KEY_KEY,
  BASE_URL_KEY,
  getStorageItem,
  resolveConfig,
  setStorageItem,
} from "../lib/index.js";

function el<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) {
    throw new Error(`FinX task pane: missing element #${id}`);
  }
  return node as T;
}

async function loadInto(baseUrlInput: HTMLInputElement, apiKeyInput: HTMLInputElement): Promise<void> {
  const baseUrl = await getStorageItem(BASE_URL_KEY);
  const apiKey = await getStorageItem(API_KEY_KEY);
  baseUrlInput.value = baseUrl ?? "";
  apiKeyInput.value = apiKey ?? "";
}

async function save(
  baseUrlInput: HTMLInputElement,
  apiKeyInput: HTMLInputElement,
  status: HTMLElement,
): Promise<void> {
  try {
    // Validate via the same pure resolver the functions use.
    resolveConfig(baseUrlInput.value, apiKeyInput.value);
    await setStorageItem(BASE_URL_KEY, baseUrlInput.value.trim());
    await setStorageItem(API_KEY_KEY, apiKeyInput.value.trim());
    status.textContent = "Saved. Functions will use these settings.";
    status.style.color = "#107c10";
  } catch (err) {
    status.textContent = err instanceof Error ? err.message : String(err);
    status.style.color = "#a4262c";
  }
}

Office.onReady(() => {
  const baseUrlInput = el<HTMLInputElement>("baseUrl");
  const apiKeyInput = el<HTMLInputElement>("apiKey");
  const status = el<HTMLElement>("status");
  void loadInto(baseUrlInput, apiKeyInput);
  el<HTMLButtonElement>("save").addEventListener("click", () => {
    void save(baseUrlInput, apiKeyInput, status);
  });
});
