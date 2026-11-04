#!/usr/bin/env python3
"""Check out configured sibling repositories at exact commit IDs for CI."""

from __future__ import annotations

import os
import re
import subprocess
from pathlib import Path

SOURCES = {
    "appimage-rs": ("APPIMAGE_RS_URL", "APPIMAGE_RS_REF"),
}
SHA_RE = re.compile(r"^[0-9a-fA-F]{40}$")


def run(*args: str, cwd: Path | None = None) -> str:
    result = subprocess.run(
        args,
        cwd=cwd,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    )
    return result.stdout.strip()


def main() -> None:
    workspace = Path(os.environ["GITHUB_WORKSPACE"])
    for directory, (url_name, ref_name) in SOURCES.items():
        url = os.environ[url_name].strip()
        commit = os.environ[ref_name].strip()
        if not url:
            raise SystemExit(f"{url_name} must contain a Git clone URL")
        if not SHA_RE.fullmatch(commit):
            raise SystemExit(f"{ref_name} must contain a full 40-character commit SHA")

        checkout = workspace / directory
        if checkout.exists():
            raise SystemExit(f"Refusing to replace existing checkout: {checkout}")
        checkout.mkdir()
        run("git", "init", "--quiet", str(checkout))
        run("git", "remote", "add", "origin", url, cwd=checkout)
        run("git", "fetch", "--depth=1", "origin", commit, cwd=checkout)
        run("git", "checkout", "--detach", "FETCH_HEAD", cwd=checkout)
        actual = run("git", "rev-parse", "HEAD", cwd=checkout)
        if actual.lower() != commit.lower():
            raise SystemExit(f"{directory} checked out {actual}, expected {commit}")
        print(f"Checked out {directory} at {actual}")


if __name__ == "__main__":
    main()
