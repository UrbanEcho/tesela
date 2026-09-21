#!/usr/bin/env python3
"""Fail when a maintained source file exceeds the repository size limit."""

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SOURCE_ROOTS = (ROOT / "crates", ROOT / "python", ROOT / "scripts")
EXTENSIONS = {".rs", ".py", ".pyi"}
MAX_LINES = 500


def main() -> int:
    oversized: list[tuple[Path, int]] = []
    for source_root in SOURCE_ROOTS:
        for path in source_root.rglob("*"):
            if not path.is_file() or path.suffix not in EXTENSIONS:
                continue
            lines = sum(1 for _ in path.open(encoding="utf-8"))
            if lines > MAX_LINES:
                oversized.append((path.relative_to(ROOT), lines))

    for path, lines in sorted(oversized):
        print(f"{path}: {lines} lines (maximum {MAX_LINES})")
    return 1 if oversized else 0


if __name__ == "__main__":
    raise SystemExit(main())
