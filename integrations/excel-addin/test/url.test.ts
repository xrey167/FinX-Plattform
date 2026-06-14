import { describe, expect, it } from "vitest";
import { buildUrl, coerceParams, encodeParams, normalizeRoute } from "../src/lib/url.js";

describe("normalizeRoute", () => {
  it("strips leading/trailing slashes", () => {
    expect(normalizeRoute("/equity/price/historical/")).toBe("equity/price/historical");
  });

  it("strips an accidental api/v1 prefix", () => {
    expect(normalizeRoute("/api/v1/equity/price/historical")).toBe("equity/price/historical");
    expect(normalizeRoute("api/v1/economy/gdp")).toBe("economy/gdp");
  });

  it("trims whitespace", () => {
    expect(normalizeRoute("  equity/quote  ")).toBe("equity/quote");
  });
});

describe("encodeParams", () => {
  it("returns empty string for undefined or empty", () => {
    expect(encodeParams(undefined)).toBe("");
    expect(encodeParams({})).toBe("");
  });

  it("sorts keys for stable output", () => {
    expect(encodeParams({ symbol: "AAPL", provider: "yahoo" })).toBe(
      "provider=yahoo&symbol=AAPL",
    );
  });

  it("percent-encodes keys and values", () => {
    expect(encodeParams({ "a b": "c&d" })).toBe("a%20b=c%26d");
  });

  it("stringifies booleans and numbers as the server expects", () => {
    expect(encodeParams({ chart: true, limit: 30 })).toBe("chart=true&limit=30");
  });

  it("skips null and undefined values", () => {
    expect(encodeParams({ a: "1", b: null, c: undefined })).toBe("a=1");
  });
});

describe("buildUrl", () => {
  it("composes base + prefix + route + query", () => {
    expect(
      buildUrl("https://api.example.com", "equity/price/historical", { symbol: "AAPL" }),
    ).toBe("https://api.example.com/api/v1/equity/price/historical?symbol=AAPL");
  });

  it("trims a trailing slash on the base URL", () => {
    expect(buildUrl("https://api.example.com/", "equity/quote")).toBe(
      "https://api.example.com/api/v1/equity/quote",
    );
  });

  it("omits the query when there are no params", () => {
    expect(buildUrl("https://x.io", "meta/routes")).toBe("https://x.io/api/v1/meta/routes");
  });

  it("throws on empty base URL", () => {
    expect(() => buildUrl("", "equity/quote")).toThrow(/base URL/i);
  });

  it("throws on empty route", () => {
    expect(() => buildUrl("https://x.io", "  /  ")).toThrow(/route is empty/i);
  });
});

describe("coerceParams", () => {
  it("returns an empty map for null/empty", () => {
    expect(coerceParams(null)).toEqual({});
    expect(coerceParams("")).toEqual({});
  });

  it("passes through an object", () => {
    expect(coerceParams({ symbol: "AAPL" })).toEqual({ symbol: "AAPL" });
  });

  it("parses a k=v;k2=v2 string", () => {
    expect(coerceParams("symbol=AAPL;provider=yahoo")).toEqual({
      symbol: "AAPL",
      provider: "yahoo",
    });
  });

  it("parses an ampersand-delimited string", () => {
    expect(coerceParams("a=1&b=2")).toEqual({ a: "1", b: "2" });
  });

  it("keeps a bare key with an empty value", () => {
    expect(coerceParams("flag")).toEqual({ flag: "" });
  });
});
