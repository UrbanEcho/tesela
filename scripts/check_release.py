"""Small release preflight: versions and local Markdown links."""

import os
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def version(path: Path) -> str:
    match = re.search(r'^version\s*=\s*"([^"]+)"', path.read_text(), re.MULTILINE)
    if match is None:
        raise SystemExit(f"version missing in {path}")
    return match.group(1)


def main() -> None:
    rust = version(ROOT / "Cargo.toml")
    python = version(ROOT / "pyproject.toml")
    if rust != python:
        raise SystemExit(f"Cargo version {rust} differs from PyPI version {python}")
    manifest = (ROOT / "Cargo.toml").read_text()
    for dependency_version in re.findall(r'^tesela(?:-[a-z]+)?\s*=\s*\{[^\n]*version\s*=\s*"([^"]+)"', manifest, re.MULTILINE):
        if dependency_version != rust:
            raise SystemExit(f"workspace dependency version {dependency_version} differs from {rust}")
    for crate in ROOT.glob("crates/*/Cargo.toml"):
        if "version.workspace = true" not in crate.read_text():
            raise SystemExit(f"crate version does not inherit workspace version: {crate}")
    tag = os.environ.get("VERSION_TAG")
    if tag and tag != f"v{rust}":
        raise SystemExit(f"tag {tag} differs from package version v{rust}")
    for document in (ROOT / "README.md", ROOT / "SECURITY.md", ROOT / "python/README.md", ROOT / "benchmarks/README.md"):
        for target in re.findall(r'\]\(([^)]+)\)', document.read_text()):
            if "://" in target or target.startswith("#"):
                continue
            local = (document.parent / target.split("#", 1)[0]).resolve()
            if not local.exists():
                raise SystemExit(f"broken local link in {document}: {target}")
    print(f"release preflight passed for v{rust}")


if __name__ == "__main__":
    main()
