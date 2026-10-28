#!/usr/bin/env python3
"""Reject the oxplow checkout's dev loop in the user docs.

`docs/` and `README.md` are read by people who installed oxplow. They have no
checkout: no `cargo`, no `bun run`, no `.context/`, no `crates/` or
`examples/extensions` folder to open. The dev loop belongs in `DEV.md`.

`DEV.md` is the one file that may name them. The docs may say `oxplow
extension ...`: the app's "Install `oxplow` command line…" puts it on PATH.

Exit 1 and list the lines when any are found.
"""

from __future__ import annotations

import pathlib
import sys

NEEDLES = [
    "examples/extensions",
    "extensions/oxplow-",
    "crates/",
    "tests-e2e",
    "cargo ",
    "bun run",
    ".context",
]
ROOTS = ["docs", "README.md"]
EXTS = {".md", ".json", ".yaml", ".yml"}


def files() -> list[pathlib.Path]:
    found: list[pathlib.Path] = []
    for root in ROOTS:
        p = pathlib.Path(root)
        if p.is_file():
            found.append(p)
        elif p.is_dir():
            found += sorted(f for f in p.rglob("*") if f.is_file() and f.suffix in EXTS)
    return found


def main() -> int:
    hits: list[str] = []
    for path in files():
        for n, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            for needle in NEEDLES:
                if needle in line:
                    hits.append(f"  {path}:{n}: `{needle}`")
    if not hits:
        return 0
    print("The user docs name the oxplow checkout:", file=sys.stderr)
    print("\n".join(hits), file=sys.stderr)
    print(
        "\nUser docs are read by people who installed oxplow. Move the dev loop "
        "to DEV.md, or rewrite it for an installed user.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
