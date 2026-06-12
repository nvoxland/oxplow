#!/usr/bin/env python3
"""Enforce per-crate line-coverage floors.

Reads `cargo llvm-cov report --json` from stdin and fails (exit 1)
if any crate's aggregated line coverage is below its floor.

Tracking per-crate floors (in addition to the workspace floor in
ci.yml) keeps a regression in one crate from hiding behind a high
average. Floors are deliberately conservative — set just below the
current measured coverage so a real drop fails CI but normal
fluctuation doesn't. Raise them as crate coverage grows.
"""
from __future__ import annotations

import json
import re
import sys
from collections import defaultdict

# Floors are line-coverage percentages. Crates not listed are not
# enforced individually (still counted in the workspace total).
FLOORS = {
    # oxplow-app: baseline ~68% after the wiki_pages + work_item_service
    # edge-case test batch landed. Floor sits 3pt below to absorb churn
    # without flaking; raise as integration coverage continues to grow.
    "oxplow-app": 65.0,
    "oxplow-config": 70.0,
    "oxplow-db": 70.0,
    "oxplow-domain": 70.0,
    "oxplow-fs-watch": 60.0,
    "oxplow-git": 80.0,
    "oxplow-lsp": 75.0,
    # oxplow-lsp-installer: pure-helper test batch lifted this from 66
    # → 80%. Pinned at 75 (5pt cushion) to lock the gain.
    "oxplow-lsp-installer": 75.0,
    "oxplow-runtime": 90.0,
    "oxplow-session": 70.0,
    # Subprocess-heavy crates exercised through oxplow-app integration:
    "oxplow-pty": 80.0,
    "oxplow-tmux": 70.0,
    # Adapter crates currently with light coverage. Floors are set at
    # the current baseline and should be raised as integration tests
    # land. The MCP + IPC adapters are mostly thin handlers that need a
    # fuller mock-runtime harness (Tauri State + MCP transport) to
    # exercise; the helpers and error mapping inside them are now
    # tested directly.
    # oxplow-mcp: pure-helper batch (parse_kind/status/priority/
    # link_type, expect_id_kind, compose_*_brief) lifted this from
    # 25% → 40%. 35 leaves cushion for the next batch.
    "oxplow-mcp": 35.0,
    # oxplow-tauri-ipc: a broad read-command batch (git/branch/stream/
    # config/thread/comment/note/snapshot/effort/workspace/lsp adapters,
    # driven through the mock-runtime harness) lifted this from 27% →
    # ~48%. Pinned at 42 (≈5pt cushion) to lock the gain; raise as the
    # remaining process-bound adapters (launch/menu/terminal/webview)
    # get covered.
    "oxplow-tauri-ipc": 42.0,
}

CRATE_RE = re.compile(r"/crates/([^/]+)/")


def crate_of(path: str) -> str | None:
    m = CRATE_RE.search(path)
    return m.group(1) if m else None


def main() -> int:
    data = json.load(sys.stdin)
    by_crate: dict[str, list[int]] = defaultdict(lambda: [0, 0])
    for export in data.get("data", []):
        for f in export.get("files", []):
            crate = crate_of(f["filename"])
            if not crate:
                continue
            lines = f["summary"]["lines"]
            by_crate[crate][0] += lines["count"]
            by_crate[crate][1] += lines["covered"]

    failures: list[str] = []
    print(f"{'crate':<24} {'lines':>10} {'covered':>10} {'pct':>8} {'floor':>8} {'ok'}")
    for crate, floor in sorted(FLOORS.items()):
        total, covered = by_crate.get(crate, [0, 0])
        pct = (covered / total * 100.0) if total else 0.0
        ok = pct >= floor
        flag = "OK" if ok else "FAIL"
        print(f"{crate:<24} {total:>10} {covered:>10} {pct:>7.2f}% {floor:>7.2f}% {flag}")
        if not ok:
            failures.append(f"{crate}: {pct:.2f}% < floor {floor:.2f}%")

    if failures:
        print("\nPer-crate coverage floor failures:", file=sys.stderr)
        for line in failures:
            print(f"  - {line}", file=sys.stderr)
        return 1
    print("\nAll per-crate floors OK.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
