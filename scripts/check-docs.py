#!/usr/bin/env python3
"""Check source Markdown links, mdBook coverage, docs metadata, and optional HTML."""

from __future__ import annotations

import argparse
import html
import os
import re
import sys
import tomllib
import urllib.parse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SKIP_DIRS = {".git", "target", "book", "node_modules", "vendor"}
LINK_RE = re.compile(r"!?\[[^\]]*\]\(\s*(?:<([^>]+)>|([^\s)]+))(?:\s+[^)]*)?\)")
FENCE_RE = re.compile(r"^\s*```([^\s`]*)\s*$")


def source_markdown() -> list[Path]:
    files: list[Path] = []
    for directory, children, names in os.walk(ROOT):
        children[:] = [name for name in children if name not in SKIP_DIRS]
        files.extend(Path(directory) / name for name in names if name.endswith(".md"))
    return sorted(files)


def markdown_anchor_ids(text: str) -> set[str]:
    ids: set[str] = set()
    counts: dict[str, int] = {}
    for heading in re.findall(r"(?m)^\s{0,3}#{1,6}\s+(.+?)\s*#*\s*$", text):
        heading = re.sub(r"`([^`]*)`", r"\1", heading)
        slug = re.sub(r"[^\w -]", "", heading.lower(), flags=re.UNICODE)
        slug = re.sub(r"\s", "-", slug.strip())
        count = counts.get(slug, 0)
        counts[slug] = count + 1
        ids.add(slug if count == 0 else f"{slug}-{count}")
    ids.update(re.findall(r"\{#([\w-]+)\}", text))
    return ids


def without_inline_code(line: str) -> str:
    """Remove inline code spans so example links are not treated as Markdown links."""
    return re.sub(r"(`+).*?\1", "", line)


def check_markdown(files: list[Path], errors: list[str]) -> None:
    texts = {path: path.read_text(encoding="utf-8") for path in files}
    anchors = {path: markdown_anchor_ids(text) for path, text in texts.items()}

    for path, text in texts.items():
        rel = path.relative_to(ROOT)
        in_fence = False
        in_toml = False
        toml_lines: list[str] = []
        toml_start = 0
        for number, line in enumerate(text.splitlines(), 1):
            fence = FENCE_RE.match(line)
            if fence:
                language = fence.group(1).lower()
                if not in_fence:
                    in_fence = True
                    in_toml = language in {"toml", "cargo-toml"}
                    if in_toml:
                        toml_lines, toml_start = [], number + 1
                else:
                    if in_toml:
                        try:
                            tomllib.loads("\n".join(toml_lines))
                        except tomllib.TOMLDecodeError as exc:
                            errors.append(f"{rel}:{toml_start}: invalid TOML fence: {exc}")
                    in_fence = False
                    in_toml = False
                continue

            if in_fence:
                if in_toml:
                    toml_lines.append(line)
                continue

            visible_line = without_inline_code(line)
            for match in LINK_RE.finditer(visible_line):
                destination = match.group(1) or match.group(2) or ""
                parsed = urllib.parse.urlsplit(html.unescape(destination))
                if parsed.scheme or parsed.netloc:
                    continue
                target = path if not parsed.path else (path.parent / urllib.parse.unquote(parsed.path)).resolve()
                if not target.exists():
                    errors.append(f"{rel}:{number}: broken local link: {destination}")
                    continue
                if parsed.fragment and target in anchors:
                    fragment = urllib.parse.unquote(parsed.fragment)
                    if fragment not in anchors[target]:
                        errors.append(f"{rel}:{number}: missing anchor #{fragment} in {target.relative_to(ROOT)}")

        if in_toml:
            try:
                tomllib.loads("\n".join(toml_lines))
            except tomllib.TOMLDecodeError as exc:
                errors.append(f"{rel}:{toml_start}: invalid TOML fence: {exc}")


def check_book(files: list[Path], errors: list[str]) -> None:
    summary = ROOT / "docs/src/SUMMARY.md"
    if not summary.exists():
        errors.append("docs/src/SUMMARY.md: mdBook summary is missing")
        return
    summary_text = summary.read_text(encoding="utf-8")
    listed: set[Path] = set()
    for number, line in enumerate(summary_text.splitlines(), 1):
        for match in LINK_RE.finditer(line):
            destination = match.group(1) or match.group(2) or ""
            parsed = urllib.parse.urlsplit(destination)
            if parsed.scheme or parsed.netloc or not parsed.path:
                continue
            target = (summary.parent / urllib.parse.unquote(parsed.path)).resolve()
            if target.suffix.lower() == ".md":
                listed.add(target)
                if not target.is_file():
                    errors.append(f"docs/src/SUMMARY.md:{number}: listed page does not exist: {destination}")
    for path in files:
        if path == summary or path.relative_to(ROOT).as_posix().startswith("docs/src/") is False:
            continue
        if path not in listed:
            errors.append(f"{path.relative_to(ROOT)}: Markdown page is not linked from docs/src/SUMMARY.md")

    book_text = "\n".join(
        text for path, text in ((p, p.read_text(encoding="utf-8")) for p in files)
        if path.relative_to(ROOT).as_posix().startswith("docs/src/")
    )
    settings_dir = ROOT / "src/bundle/settings"
    settings = "\n".join(
        path.read_text(encoding="utf-8")
        for path in sorted(settings_dir.glob("*.rs"))
        if path.name != "tests.rs"
    )
    # Format spellings come from the same ValueEnum attributes as CLI parsing.
    checks = {"PackageType": re.findall(r'#\[value\(name = "([a-z]+)"\)\]', settings)}
    for kind in ("BundleSettings", "LinuxSettings", "OsxSettings", "AndroidSettings"):
        schema = re.search(rf"struct {kind} \{{(.*?)\n}}", settings, re.S)
        if schema is None:
            errors.append(f"settings schema `{kind}` was not found")
            continue
        checks[kind] = re.findall(
            r"^\s{4}(?:pub(?:\([^)]*\))?\s+)?(\w+): Option<",
            schema.group(1), re.M,
        )
    if not checks["PackageType"]:
        errors.append("settings schema has no CLI format names")
    for kind, fields in checks.items():
        for field in fields:
            # TOML identifiers may be shown in code or prose; exact text coverage is the intent.
            if not re.search(rf"(?<![\w-]){re.escape(field)}(?![\w-])", book_text):
                errors.append(f"docs/src: book does not mention {kind} field/format `{field}`")

    main = (ROOT / "src/main.rs").read_text(encoding="utf-8")
    cli_body = re.search(r"pub struct Cli \{(.*?)\n}", main, re.S).group(1)
    cli_options = re.findall(r"pub (\w+):", cli_body)
    for option in cli_options:
        spelling = "--" + option.replace("_", "-")
        if spelling not in book_text:
            errors.append(f"docs/src: book does not document CLI option `{spelling}`")


def check_html(book_dir: Path, errors: list[str]) -> None:
    if not book_dir.is_dir():
        errors.append(f"--html: rendered book directory does not exist: {book_dir}")
        return
    pages = sorted(book_dir.rglob("*.html"))
    if not pages:
        errors.append(f"--html: no HTML pages found in {book_dir}")
        return
    ids_by_page: dict[Path, set[str]] = {}
    book_config = ROOT / "docs/book.toml"
    site_url = tomllib.loads(book_config.read_text(encoding="utf-8")).get("output", {}).get("html", {}).get("site-url", "/")
    site_path = "/" + site_url.strip("/") + "/" if site_url.strip("/") else "/"
    for page in pages:
        text = page.read_text(encoding="utf-8", errors="replace")
        ids_by_page[page] = set(re.findall(r'\b(?:id|name)=["\']([^"\']+)', text))
    href_re = re.compile(r'\b(?:href|src)=["\']([^"\']+)["\']', re.I)
    for page in pages:
        text = page.read_text(encoding="utf-8", errors="replace")
        for destination in href_re.findall(text):
            parsed = urllib.parse.urlsplit(html.unescape(destination))
            if parsed.scheme or parsed.netloc or destination.startswith(("data:", "javascript:")):
                continue
            decoded_path = urllib.parse.unquote(parsed.path)
            if decoded_path.startswith("/"):
                if not decoded_path.startswith(site_path):
                    # Root-absolute links outside mdBook's configured mount are outside this book.
                    continue
                relative_path = decoded_path[len(site_path):]
                target = (book_dir / relative_path).resolve()
            else:
                target = (page.parent / decoded_path).resolve() if decoded_path else page
            if not target.is_relative_to(book_dir.resolve()):
                errors.append(f"{page.relative_to(book_dir)}: HTML link escapes the published book: {destination}")
                continue
            if target.is_dir():
                target = target / "index.html"
            if not target.exists():
                errors.append(f"{page.relative_to(book_dir)}: broken HTML link: {destination}")
                continue
            if parsed.fragment and target in ids_by_page and urllib.parse.unquote(parsed.fragment) not in ids_by_page[target]:
                errors.append(f"{page.relative_to(book_dir)}: missing HTML anchor: {destination}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--html", type=Path, help="also validate links and anchors in rendered HTML")
    args = parser.parse_args()
    errors: list[str] = []
    files = source_markdown()
    check_markdown(files, errors)
    check_book(files, errors)
    if args.html:
        html_dir = args.html if args.html.is_absolute() else ROOT / args.html
        check_html(html_dir, errors)
    if errors:
        print("Documentation checks failed:", file=sys.stderr)
        for error in errors:
            print(f"  {error}", file=sys.stderr)
        return 1
    print(f"Documentation checks passed ({len(files)} Markdown files).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
