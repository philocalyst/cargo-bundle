#!/usr/bin/env python3
"""Build the getting-started Cargo.toml example and smoke-test its native bundle."""

from __future__ import annotations

import os
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
GETTING_STARTED = ROOT / "docs/src/getting-started.md"


def fail(message: str) -> int:
    print(f"Documentation smoke check failed: {message}", file=sys.stderr)
    return 1


def first_toml_fence(markdown: str) -> str:
    match = re.search(r"(?ms)^```toml\s*\n(.*?)^```\s*$", markdown)
    if not match:
        raise ValueError("getting-started.md has no TOML code fence")
    snippet = match.group(1)
    tomllib.loads(snippet)
    return snippet


def reported_artifact(output: str) -> Path:
    lines = output.splitlines()
    for index, line in enumerate(lines):
        if re.search(r"Finished\s+\d+\s+bundles? at:", line):
            for candidate in lines[index + 1 :]:
                candidate = candidate.strip()
                if candidate:
                    return Path(candidate)
            break
    raise ValueError("cargo-bundle did not print a finished artifact path")


def run_command(args: list[str], *, cwd: Path, env: dict[str, str]) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(args, cwd=cwd, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    if result.returncode:
        raise RuntimeError(
            f"command failed ({result.returncode}): {' '.join(args)}\n"
            f"output:\n{result.stdout}"
        )
    return result


def main() -> int:
    if sys.platform == "darwin":
        package_format = "osx"
    elif sys.platform.startswith("linux"):
        package_format = "deb"
    elif sys.platform == "win32":
        package_format = "exe"
    else:
        return fail(f"no native smoke format is configured for {sys.platform}")

    bundler = ROOT / "target/debug/cargo-bundle"
    if sys.platform == "win32":
        bundler = bundler.with_suffix(".exe")
    if not bundler.is_file():
        return fail(f"prebuilt cargo-bundle executable not found: {bundler}")
    if not GETTING_STARTED.is_file():
        return fail(f"documentation example not found: {GETTING_STARTED}")

    try:
        snippet = first_toml_fence(GETTING_STARTED.read_text(encoding="utf-8"))
        with tempfile.TemporaryDirectory(prefix="cargo-bundle-docs-") as temporary:
            temp_root = Path(temporary)
            project = temp_root / "postcard"
            env = os.environ.copy()
            env.pop("CARGO_BUNDLE_SKIP_BUILD", None)
            env.pop("CARGO_BUILD_TARGET", None)
            env["CARGO_TARGET_DIR"] = str(temp_root / "target")
            cargo = shutil.which("cargo")
            if cargo:
                env["CARGO"] = cargo

            run_command(["cargo", "new", "postcard", "--bin"], cwd=temp_root, env=env)
            manifest = project / "Cargo.toml"
            with manifest.open("a", encoding="utf-8") as output:
                output.write("\n")
                output.write(snippet)
                output.write("\n")

            result = run_command([str(bundler)], cwd=project, env=env)
            artifact = reported_artifact(result.stdout)
            if not artifact.is_absolute():
                artifact = project / artifact
            if not artifact.is_file() and not artifact.is_dir():
                return fail(f"printed artifact path does not exist: {artifact}")

            if package_format == "osx":
                plist_path = artifact / "Contents/Info.plist"
                executable_dir = artifact / "Contents/MacOS"
                if not plist_path.is_file():
                    return fail(f"app bundle is missing {plist_path}")
                with plist_path.open("rb") as plist_file:
                    info = plistlib.load(plist_file)
                identifier = info.get("CFBundleIdentifier")
                if identifier != "com.example.postcard":
                    return fail(f"expected CFBundleIdentifier com.example.postcard, got {identifier!r}")
                executable = executable_dir / info.get("CFBundleExecutable", "postcard")
                if not executable.is_file():
                    return fail(f"app bundle is missing executable: {executable}")
                greeting = run_command([str(executable)], cwd=project, env=env)
                if "Hello, world!" not in greeting.stdout:
                    return fail(f"bundled executable did not print greeting: {greeting.stdout!r}")
            elif package_format == "deb":
                if artifact.read_bytes()[:8] != b"!<arch>\n":
                    return fail(f"Debian package does not have ar archive magic: {artifact}")
                staged = artifact.parent / artifact.stem
                desktop = staged / "data/usr/share/applications/postcard.desktop"
                if not desktop.is_file():
                    return fail(f"Debian staging tree is missing desktop entry: {desktop}")
                desktop_text = desktop.read_text(encoding="utf-8")
                if "Name=Postcard" not in desktop_text:
                    return fail(f"desktop entry is missing Name=Postcard: {desktop}")
            else:
                if artifact.read_bytes()[:2] != b"MZ":
                    return fail(f"Windows bundle does not have PE executable magic: {artifact}")

        print(f"Documentation smoke check passed for native {package_format} bundle.")
        return 0
    except (OSError, ValueError, RuntimeError, tomllib.TOMLDecodeError) as exc:
        return fail(str(exc))


if __name__ == "__main__":
    raise SystemExit(main())
