// Pure error mapping: turn a thrown error or a non-2xx REST response into a
// stable, human-readable cell string the custom function can return. Keeping
// this here (not in the Office.js wrapper) makes the mapping unit-testable.

/** A structured error message the add-in surfaces to a worksheet cell. */
export interface CellError {
  /** Short machine-ish code, e.g. "FINX#CONFIG" / "FINX#HTTP400". */
  code: string;
  /** Human-readable detail. */
  message: string;
}

/** Render a CellError as the single string Excel shows in the cell. */
export function formatCellError(err: CellError): string {
  return `${err.code}: ${err.message}`;
}

/**
 * Map an HTTP status + body text to a CellError. The server returns 400 for
 * unknown route / invalid params and 502 for a provider failure
 * (`crates/tdw-app-server/src/rest_route.rs`); message bodies are passed
 * through trimmed.
 */
export function httpStatusToError(status: number, bodyText: string): CellError {
  const detail = (bodyText ?? "").trim() || "request failed";
  return { code: `FINX#HTTP${status}`, message: detail };
}

/** Map a configuration problem (missing base URL, empty route) to a CellError. */
export function configError(message: string): CellError {
  return { code: "FINX#CONFIG", message };
}

/**
 * Normalize any thrown value into a CellError. A fetch/network rejection maps
 * to FINX#NETWORK; everything else to FINX#ERROR. Used by the thin custom-
 * function wrappers so a rejected promise still lands as a readable cell value.
 */
export function toCellError(err: unknown): CellError {
  if (err && typeof err === "object" && "code" in err && "message" in err) {
    const maybe = err as Partial<CellError>;
    if (typeof maybe.code === "string" && typeof maybe.message === "string") {
      return { code: maybe.code, message: maybe.message };
    }
  }
  const message =
    err instanceof Error ? err.message : typeof err === "string" ? err : String(err);
  const isNetwork = /network|fetch|failed to fetch|ECONN|timeout/i.test(message);
  return { code: isNetwork ? "FINX#NETWORK" : "FINX#ERROR", message };
}
