import { describe, expect, it } from "vitest";
import {
  extractRecords,
  flattenToGrid,
  resolveFields,
  toCell,
  transpose,
} from "../src/lib/flatten.js";

const envelope = {
  id: "req-1",
  provider: "yahoo",
  results: [
    { date: "2024-01-01", close: 100, volume: 5 },
    { date: "2024-01-02", close: 101, volume: 6 },
    { date: "2024-01-03", close: 102, volume: 7 },
  ],
};

describe("toCell", () => {
  it("passes scalars through", () => {
    expect(toCell("x")).toBe("x");
    expect(toCell(3)).toBe(3);
    expect(toCell(true)).toBe(true);
  });

  it("maps null/undefined to empty string", () => {
    expect(toCell(null)).toBe("");
    expect(toCell(undefined)).toBe("");
  });

  it("maps non-finite numbers to empty string", () => {
    expect(toCell(Number.NaN)).toBe("");
    expect(toCell(Number.POSITIVE_INFINITY)).toBe("");
  });

  it("JSON-stringifies nested objects/arrays into one cell", () => {
    expect(toCell({ a: 1 })).toBe('{"a":1}');
    expect(toCell([1, 2])).toBe("[1,2]");
  });
});

describe("extractRecords", () => {
  it("pulls results out of an envelope", () => {
    expect(extractRecords(envelope)).toHaveLength(3);
  });

  it("accepts a bare array", () => {
    expect(extractRecords([{ a: 1 }])).toEqual([{ a: 1 }]);
  });

  it("wraps a non-envelope object as a single record", () => {
    expect(extractRecords({ a: 1 })).toEqual([{ a: 1 }]);
  });

  it("returns empty for null / non-record", () => {
    expect(extractRecords(null)).toEqual([]);
    expect(extractRecords(42)).toEqual([]);
  });
});

describe("resolveFields", () => {
  it("uses explicit fields verbatim", () => {
    expect(resolveFields(envelope.results, ["close", "date"])).toEqual(["close", "date"]);
  });

  it("unions keys in first-seen order when no fields given", () => {
    const recs = [{ a: 1 }, { b: 2 }, { a: 3, c: 4 }];
    expect(resolveFields(recs)).toEqual(["a", "b", "c"]);
  });
});

describe("flattenToGrid", () => {
  it("produces a header row then one row per record", () => {
    const grid = flattenToGrid(envelope);
    expect(grid[0]).toEqual(["date", "close", "volume"]);
    expect(grid).toHaveLength(4);
    expect(grid[1]).toEqual(["2024-01-01", 100, 5]);
  });

  it("projects and orders by explicit fields, padding absent ones", () => {
    const grid = flattenToGrid(envelope, { fields: ["close", "missing"] });
    expect(grid[0]).toEqual(["close", "missing"]);
    expect(grid[1]).toEqual([100, ""]);
  });

  it("caps rows", () => {
    const grid = flattenToGrid(envelope, { rows: 1 });
    // header + 1 row
    expect(grid).toHaveLength(2);
  });

  it("caps columns", () => {
    const grid = flattenToGrid(envelope, { columns: 2 });
    expect(grid[0]).toEqual(["date", "close"]);
  });

  it("suppresses the header when header=false", () => {
    const grid = flattenToGrid(envelope, { header: false });
    expect(grid).toHaveLength(3);
    expect(grid[0]).toEqual(["2024-01-01", 100, 5]);
  });

  it("returns a (no data) cell for an empty result set", () => {
    expect(flattenToGrid({ results: [] })).toEqual([["(no data)"]]);
    expect(flattenToGrid(null)).toEqual([["(no data)"]]);
  });

  it("transposes when requested", () => {
    const grid = flattenToGrid(envelope, { fields: ["date", "close"], rows: 2, transpose: true });
    // rows become columns: first row is the field names down the first column.
    expect(grid[0]).toEqual(["date", "2024-01-01", "2024-01-02"]);
    expect(grid[1]).toEqual(["close", 100, 101]);
  });
});

describe("transpose", () => {
  it("flips rows and columns", () => {
    expect(transpose([[1, 2, 3], [4, 5, 6]])).toEqual([[1, 4], [2, 5], [3, 6]]);
  });

  it("returns empty for empty input", () => {
    expect(transpose([])).toEqual([]);
  });
});
