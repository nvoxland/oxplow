#!/usr/bin/env python3
"""Reject NUL bytes in tracked text sources.

A source file containing a NUL is classified as *binary* by git, grep, ugrep and
ripgrep alike. It still compiles and its tests still pass — it is simply
invisible to every codebase search, and the search reports success with zero
hits rather than an error. In a codebase an agent has to navigate by search,
"this file is findable" is a correctness property, not a style preference.

This actually happened (tsk168): two literal NULs in a `.tsx` file — written as
raw control characters instead of the `\\u0000` escape in a `join()` sentinel —
made the whole file unsearchable, and it was only noticed when a grep for a
symbol that was definitely there came back empty.

NUL is the exact signal to check: git's own binary heuristic is "a NUL in the
first 8000 bytes". Other C0 controls are deliberately NOT flagged — ESC in
particular appears legitimately in terminal-emulator test fixtures.

Exit 1 and list offenders when any are found.
"""

from __future__ import annotations

import subprocess
import sys

# Extensions whose contents are legitimately binary; a NUL there means nothing.
BINARY_EXTS = {
    "png", "jpg", "jpeg", "gif", "ico", "webp", "avif", "bmp", "tiff",
    "pdf", "woff", "woff2", "ttf", "otf", "eot",
    "zip", "gz", "tgz", "bz2", "xz", "zst", "7z", "tar",
    "so", "dylib", "dll", "a", "o", "wasm", "node", "exe", "bin",
    "sqlite", "sqlite3", "db", "icns", "dmg", "keystore", "jks",
    "mp3", "mp4", "mov", "wav", "ogg", "webm", "avi",
}


def tracked_files() -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "-z"],
        check=True,
        capture_output=True,
    ).stdout
    return [p for p in out.decode("utf-8", "surrogateescape").split("\0") if p]


def is_binary_by_ext(path: str) -> bool:
    _, _, ext = path.rpartition(".")
    return ext.lower() in BINARY_EXTS if "." in path else False


def main() -> int:
    offenders: list[tuple[str, int]] = []
    for path in tracked_files():
        if is_binary_by_ext(path):
            continue
        try:
            with open(path, "rb") as fh:
                data = fh.read()
        except (FileNotFoundError, IsADirectoryError, PermissionError):
            # Submodule, symlink to nowhere, or a path we can't read — not ours
            # to police.
            continue
        count = data.count(b"\0")
        if count:
            offenders.append((path, count))

    if not offenders:
        return 0

    print("NUL bytes found in tracked text sources:", file=sys.stderr)
    for path, count in offenders:
        print(f"  {path}: {count} NUL byte(s)", file=sys.stderr)
    print(
        "\nThese files are treated as BINARY by git and every grep-family tool, so "
        "nothing in them is searchable.\nIf the byte is intentional, write it as an "
        "escape (\\u0000 / \\0) instead of a raw control character.\n"
        "If the file really is binary, add its extension to BINARY_EXTS in "
        "scripts/check-control-chars.py.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
