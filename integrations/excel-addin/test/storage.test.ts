import { afterEach, describe, expect, it, vi } from "vitest";
import { getStorageItem, setStorageItem } from "../src/lib/storage.js";

// The helpers read two ambient globals (`OfficeRuntime`, `localStorage`). Each
// test injects/removes them on `globalThis` and restores afterward, so we can
// exercise the host path, the browser-fallback path, and the no-store path
// without an Office runtime.
const G = globalThis as Record<string, unknown>;

afterEach(() => {
  delete G.OfficeRuntime;
  delete G.localStorage;
  vi.restoreAllMocks();
});

describe("storage helpers", () => {
  it("uses OfficeRuntime.storage when the host runtime is present", async () => {
    const getItem = vi.fn().mockResolvedValue("host-value");
    const setItem = vi.fn().mockResolvedValue(undefined);
    G.OfficeRuntime = { storage: { getItem, setItem } };

    expect(await getStorageItem("k")).toBe("host-value");
    expect(getItem).toHaveBeenCalledWith("k");

    await setStorageItem("k", "v");
    expect(setItem).toHaveBeenCalledWith("k", "v");
  });

  it("falls back to localStorage when OfficeRuntime is absent", async () => {
    const getItem = vi.fn().mockReturnValue("browser-value");
    const setItem = vi.fn();
    G.localStorage = { getItem, setItem };

    expect(await getStorageItem("k")).toBe("browser-value");
    expect(getItem).toHaveBeenCalledWith("k");

    await setStorageItem("k", "v");
    expect(setItem).toHaveBeenCalledWith("k", "v");
  });

  it("prefers OfficeRuntime over localStorage when both exist", async () => {
    const hostGet = vi.fn().mockResolvedValue("host");
    const browserGet = vi.fn().mockReturnValue("browser");
    G.OfficeRuntime = { storage: { getItem: hostGet, setItem: vi.fn() } };
    G.localStorage = { getItem: browserGet, setItem: vi.fn() };

    expect(await getStorageItem("k")).toBe("host");
    expect(browserGet).not.toHaveBeenCalled();
  });

  it("returns null and is a no-op when neither store exists", async () => {
    expect(await getStorageItem("k")).toBeNull();
    // Must not throw a ReferenceError outside the Office host (Gemini #453).
    await expect(setStorageItem("k", "v")).resolves.toBeUndefined();
  });
});
