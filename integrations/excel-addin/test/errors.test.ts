import { describe, expect, it } from "vitest";
import {
  configError,
  formatCellError,
  httpStatusToError,
  toCellError,
} from "../src/lib/errors.js";

describe("formatCellError", () => {
  it("renders code: message", () => {
    expect(formatCellError({ code: "FINX#CONFIG", message: "nope" })).toBe("FINX#CONFIG: nope");
  });
});

describe("httpStatusToError", () => {
  it("encodes the status and trims the body", () => {
    expect(httpStatusToError(400, "  unknown route  ")).toEqual({
      code: "FINX#HTTP400",
      message: "unknown route",
    });
  });

  it("falls back to a default message on empty body", () => {
    expect(httpStatusToError(502, "")).toEqual({
      code: "FINX#HTTP502",
      message: "request failed",
    });
  });
});

describe("configError", () => {
  it("uses the FINX#CONFIG code", () => {
    expect(configError("missing")).toEqual({ code: "FINX#CONFIG", message: "missing" });
  });
});

describe("toCellError", () => {
  it("passes through a CellError-shaped throw", () => {
    expect(toCellError({ code: "FINX#HTTP400", message: "bad" })).toEqual({
      code: "FINX#HTTP400",
      message: "bad",
    });
  });

  it("classifies network-ish errors", () => {
    expect(toCellError(new Error("Failed to fetch")).code).toBe("FINX#NETWORK");
    expect(toCellError(new Error("network timeout")).code).toBe("FINX#NETWORK");
  });

  it("classifies other errors as FINX#ERROR", () => {
    expect(toCellError(new Error("boom"))).toEqual({ code: "FINX#ERROR", message: "boom" });
  });

  it("stringifies a non-Error throw", () => {
    expect(toCellError("plain string")).toEqual({
      code: "FINX#ERROR",
      message: "plain string",
    });
  });
});
