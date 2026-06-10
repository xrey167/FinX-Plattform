#!/usr/bin/env python3
"""Ratchet check for clippy `pedantic` + `nursery` warnings.

The default clippy build is already gated by `-D warnings` in CI, so the *default*
lint level stays clean. The `pedantic` and `nursery` groups, however, are NOT
denied anywhere — without a guard they silently re-accumulate as the codebase
grows (a one-shot cleanup does not stick). This check turns that treadmill into a
one-way ratchet: a PR may hold or lower the count, never raise it.

Mechanism:
  * Count distinct `warning`-level, coded diagnostics from
    `cargo clippy --workspace --all-targets -- -W clippy::pedantic -W clippy::nursery`.
  * Compare against the integer committed in `ci/clippy-pedantic-baseline.txt`.
  * Fail (exit 1) if the count exceeds the baseline.
  * If the count is below the baseline, pass and ask the author to lower the
    baseline in the same PR so the reduction is locked in.

Bootstrap: the baseline file ships as the sentinel `AUTO`. On the first run this
script just prints the measured count (and passes) so it can never break CI before
the authoritative CI-measured number is known. Replace `AUTO` with that number to
turn enforcement on.

Counting note: diagnostics are deduplicated by (lint, file, line, column) so the
same finding surfaced across multiple `--all-targets` targets is counted once.
"""
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
BASELINE_FILE = ROOT / "ci" / "clippy-pedantic-baseline.txt"


def count_warnings() -> int:
    proc = subprocess.run(
        [
            "cargo", "clippy", "--workspace", "--all-targets",
            "--message-format=json",
            "--", "-W", "clippy::pedantic", "-W", "clippy::nursery",
        ],
        cwd=ROOT, capture_output=True, text=True,
    )
    seen = set()
    saw_output = False
    for line in proc.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            continue
        saw_output = True
        if obj.get("reason") != "compiler-message":
            continue
        msg = obj.get("message", {})
        code = msg.get("code")
        if msg.get("level") == "warning" and code:
            span = (msg.get("spans") or [{}])[0]
            key = (code.get("code"), span.get("file_name"),
                   span.get("line_start"), span.get("column_start"))
            seen.add(key)
    if not saw_output and proc.returncode != 0:
        sys.stderr.write(proc.stderr[-4000:])
        sys.stderr.write("\nclippy produced no diagnostics (compile error?)\n")
        sys.exit(2)
    return seen


def main() -> int:
    raw = BASELINE_FILE.read_text(encoding="utf-8").strip()
    warnings = count_warnings()
    actual = len(warnings)
    if raw == "AUTO":
        print(f"::notice::clippy pedantic+nursery calibration — measured count = "
              f"{actual}. Commit this number to {BASELINE_FILE.relative_to(ROOT)} "
              f"to enable enforcement.")
        print(f"pedantic+nursery warnings: {actual} (baseline: AUTO / not yet enforcing)")
        return 0
    baseline = int(raw)
    print(f"pedantic+nursery warnings: actual={actual} baseline={baseline}")
    if actual > baseline:
        # List every counted diagnostic so the offending warnings are
        # identifiable from the CI log alone (the baseline records only a
        # number, and counts can differ across platforms via cfg-gated code).
        for lint, file_name, line_start, column_start in sorted(
            warnings, key=lambda key: (key[1] or "", key[2] or 0, key[0] or "")
        ):
            print(f"  {lint} {file_name}:{line_start}:{column_start}")
        print(f"::error::pedantic+nursery warnings increased: {actual} > baseline "
              f"{baseline}. Resolve the new warning(s) or, for a genuine false "
              f"positive, add a documented #[allow(...)] with a reason. Do not raise "
              f"the baseline.")
        return 1
    if actual < baseline:
        print(f"::notice::{baseline - actual} fewer than baseline — lower "
              f"{BASELINE_FILE.relative_to(ROOT)} to {actual} in this PR to lock it in.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
