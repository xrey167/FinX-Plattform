import { describe, expect, it } from "vitest";
import { resolveConfig } from "../src/lib/config.js";

describe("resolveConfig", () => {
  it("accepts a valid https base URL", () => {
    expect(resolveConfig("https://api.example.com", undefined)).toEqual({
      baseUrl: "https://api.example.com",
    });
  });

  it("attaches a trimmed api key when present", () => {
    expect(resolveConfig("https://api.example.com", "  tok  ")).toEqual({
      baseUrl: "https://api.example.com",
      apiKey: "tok",
    });
  });

  it("trims the base URL", () => {
    expect(resolveConfig("  http://localhost:8080  ", "").baseUrl).toBe("http://localhost:8080");
  });

  it("throws FINX#CONFIG when the base URL is missing", () => {
    expect(() => resolveConfig("", undefined)).toThrow();
    try {
      resolveConfig(null, undefined);
    } catch (err) {
      expect((err as { code: string }).code).toBe("FINX#CONFIG");
    }
  });

  it("rejects a non-http(s) base URL", () => {
    expect(() => resolveConfig("ftp://x", undefined)).toThrow(/http/i);
  });
});
