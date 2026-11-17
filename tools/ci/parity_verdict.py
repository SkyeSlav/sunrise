#!/usr/bin/env python3
"""The daily parity leg's verdict file (daily.yml `parity`, .github/workflows/parity.yml with `report`).

parity_verdict.py LOG REPORT reads tools/parity.py's log on the runner for harness names and PASS/FAIL only and
writes REPORT in tools/ci/ignored_tests.py's form, which daily_summary.py --parity reads: a problem (package
`parity`, kind `parity`) for each failing harness, every passing one, and the run as incomplete when the log has no
final count (the job ended before tools/parity.py did). A harness's shards (`name#2`) are one harness, failing if
any of its shards' last record failed. No other text of the log is copied: it holds vanilla's values.
"""
import json
import re
import sys
from pathlib import Path

PARITY = "parity"
RECORD = re.compile(r"^(PASS|FAIL|FLAKY) ([a-z0-9_]+(?:#\d+)?)(?=[ :]|$)")
FINISHED = re.compile(r"^\d+ passed, \d+ failed in [\d.]+ min\.$")


def verdict(log):
    """The report for tools/parity.py's output `log` (None: no log at all)."""
    if log is None:
        return {"shards": [PARITY], "problems": [], "passed": [],
                "incomplete": [{"shard": PARITY, "reason": "no parity log: the harness run did not start"}]}
    labels, finished = {}, False
    for line in log.splitlines():
        finished |= bool(FINISHED.match(line))
        if found := RECORD.match(line):
            labels[found[2]] = found[1] == "PASS"
    harnesses = {}
    for label, passed in labels.items():
        name = label.split("#", 1)[0]
        harnesses[name] = harnesses.get(name, True) and passed
    return {
        "shards": [PARITY],
        "problems": [{"package": PARITY, "test": name, "kind": PARITY, "panic": None, "policy": None}
                     for name, passed in sorted(harnesses.items()) if not passed],
        "passed": [{"package": PARITY, "test": name} for name, passed in sorted(harnesses.items()) if passed],
        "incomplete": [] if finished else [{"shard": PARITY, "reason": "the harness run did not finish"}],
    }


def main(log, report):
    path = Path(log)
    result = verdict(path.read_text(errors="replace") if path.is_file() else None)
    Path(report).write_text(json.dumps(result, indent=2) + "\n")
    print(f"parity verdict: {len(result['passed'])} harnesses passed, {len(result['problems'])} failed"
          + ("; the run did not finish" if result["incomplete"] else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main(*sys.argv[1:3]))
