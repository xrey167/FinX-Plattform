import { describe, expect, it } from "vitest";
import { buildHeaders, fetchRoute, type FetchLike } from "../src/lib/client.js";
import { flattenToGrid } from "../src/lib/flatten.js";

function stubFetch(status: number, body: string): { fn: FetchLike; calls: string[] } {
  const calls: string[] = [];
  const fn: FetchLike = (url) => {
    calls.push(url);
    return Promise.resolve({
      ok: status >= 200 && status < 300,
      status,
      text: () => Promise.resolve(body),
    });
  };
  return { fn, calls };
}

describe("buildHeaders", () => {
  it("includes Accept and no auth without a key", () => {
    expect(buildHeaders({ baseUrl: "https://x.io" })).toEqual({ Accept: "application/json" });
  });

  it("adds a bearer token when an api key is set", () => {
    expect(buildHeaders({ baseUrl: "https://x.io", apiKey: "secret" })).toEqual({
      Accept: "application/json",
      Authorization: "Bearer secret",
    });
  });
});

describe("fetchRoute", () => {
  const config = { baseUrl: "https://api.example.com" };

  it("builds the URL and returns parsed JSON on 200", async () => {
    const body = JSON.stringify({ results: [{ a: 1 }] });
    const { fn, calls } = stubFetch(200, body);
    const out = await fetchRoute(fn, config, "equity/quote", { symbol: "AAPL" });
    expect(calls[0]).toBe("https://api.example.com/api/v1/equity/quote?symbol=AAPL");
    expect(out).toEqual({ results: [{ a: 1 }] });
  });

  it("returns an empty object for an empty 200 body", async () => {
    const { fn } = stubFetch(200, "   ");
    expect(await fetchRoute(fn, config, "meta/routes")).toEqual({});
  });

  it("throws a mapped CellError on a non-2xx status", async () => {
    const { fn } = stubFetch(400, "unknown route");
    await expect(fetchRoute(fn, config, "bad/route")).rejects.toMatchObject({
      code: "FINX#HTTP400",
      message: "unknown route",
    });
  });

  it("composes with flattenToGrid end-to-end", async () => {
    const body = JSON.stringify({
      results: [
        { date: "2024-01-01", close: 100 },
        { date: "2024-01-02", close: 101 },
      ],
    });
    const { fn } = stubFetch(200, body);
    const parsed = await fetchRoute(fn, config, "equity/price/historical");
    const grid = flattenToGrid(parsed, { fields: ["date", "close"] });
    expect(grid).toEqual([
      ["date", "close"],
      ["2024-01-01", 100],
      ["2024-01-02", 101],
    ]);
  });
});
