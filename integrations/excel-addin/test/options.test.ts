import { describe, expect, it } from "vitest";
import { parseOptions } from "../src/lib/options.js";

describe("parseOptions", () => {
  it("returns an empty object for null/empty", () => {
    expect(parseOptions(null)).toEqual({});
    expect(parseOptions("")).toEqual({});
  });

  it("passes through an object form (shallow copy)", () => {
    const input = { fields: ["a"], rows: 2 };
    const out = parseOptions(input);
    expect(out).toEqual(input);
    expect(out).not.toBe(input);
  });

  it("parses fields as a comma list", () => {
    expect(parseOptions("fields=date,close,volume")).toEqual({
      fields: ["date", "close", "volume"],
    });
  });

  it("parses rows and columns as counts", () => {
    expect(parseOptions("rows=10;columns=3")).toEqual({ rows: 10, columns: 3 });
  });

  it("parses header and transpose as booleans", () => {
    expect(parseOptions("header=false;transpose=true")).toEqual({
      header: false,
      transpose: true,
    });
  });

  it("ignores unknown keys and malformed pairs", () => {
    expect(parseOptions("bogus=1;noequals;rows=5")).toEqual({ rows: 5 });
  });

  it("treats a negative or non-numeric count as no cap (undefined)", () => {
    expect(parseOptions("rows=-1")).toEqual({ rows: undefined });
    expect(parseOptions("columns=abc")).toEqual({ columns: undefined });
  });

  it("accepts cols and col aliases", () => {
    expect(parseOptions("cols=2")).toEqual({ columns: 2 });
    expect(parseOptions("col=2")).toEqual({ columns: 2 });
  });
});
