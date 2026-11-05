//! Keeps the guide in step with the code: every setting and format must be
//! mentioned, and every page and link must exist.

use std::{collections::BTreeSet, fs, path::Path};

fn read(path: impl AsRef<Path>) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"))
}

/// All pages of the guide, as (path relative to docs/src, text).
fn pages() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/src");
    let mut pages = Vec::new();
    let mut directories = vec![root.clone()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                directories.push(path);
            } else if path.extension().is_some_and(|extension| extension == "md") {
                let name = path
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                pages.push((name, fs::read_to_string(&path).unwrap()));
            }
        }
    }
    pages
}

/// Field names of `struct <name> { ... }` in the settings source.
fn fields(source: &str, name: &str) -> Vec<String> {
    let start = source
        .find(&format!("struct {name} {{"))
        .unwrap_or_else(|| panic!("struct {name} not found"));
    source[start..]
        .lines()
        .skip(1)
        .take_while(|line| *line != "}")
        .filter_map(|line| {
            let line = line.trim_start();
            let line = line.strip_prefix("pub ").unwrap_or(line);
            let (field, _) = line.split_once(": ")?;
            field
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_')
                .then(|| field.to_owned())
        })
        .collect()
}

#[test]
fn every_setting_is_documented() {
    let source = read("src/bundle/settings.rs");
    let guide: String = pages().into_iter().map(|(_, text)| text).collect();
    let mut missing = BTreeSet::new();
    for table in [
        "BundleSettings",
        "OsxSettings",
        "IosSettings",
        "AndroidSettings",
        "LinuxSettings",
        "WindowsSigningSettings",
        "LinuxSigningSettings",
    ] {
        let found = fields(&source, table);
        assert!(!found.is_empty(), "no fields parsed for {table}");
        for field in found {
            // As a table cell, as an assignment, or as a table header.
            let mentioned = [
                format!("`{field}`"),
                format!("{field} = "),
                format!("bundle.{field}"),
            ]
            .iter()
            .any(|form| guide.contains(form));
            if !mentioned {
                missing.insert(format!("{table}.{field}"));
            }
        }
    }
    assert!(missing.is_empty(), "undocumented settings: {missing:?}");
}

#[test]
fn every_format_is_documented() {
    let source = read("src/bundle/settings.rs");
    let formats = read("docs/src/formats.md");
    let names: Vec<_> = source
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("#[value(name = \"")?
                .strip_suffix("\")]")
        })
        .collect();
    assert!(names.len() >= 11, "{names:?}");
    for name in names {
        assert!(
            formats.contains(&format!("| `{name}` |")),
            "format {name} is not in formats.md"
        );
    }
}

#[test]
fn pages_are_listed_and_links_resolve() {
    let summary = read("docs/src/SUMMARY.md");
    let pages = pages();
    for (name, text) in &pages {
        if name != "SUMMARY.md" {
            assert!(
                summary.contains(&format!("({name})")),
                "{name} is not in SUMMARY.md"
            );
        }
        let directory = Path::new(name).parent().unwrap();
        for target in text
            .split("](")
            .skip(1)
            .filter_map(|rest| rest.split(')').next())
        {
            let file = target.split('#').next().unwrap();
            if file.is_empty() || file.contains("://") {
                continue;
            }
            let resolved = directory.join(file);
            let mut parts = Vec::new();
            for part in resolved.components() {
                match part.as_os_str().to_str().unwrap() {
                    ".." => drop(parts.pop()),
                    part => parts.push(part.to_owned()),
                }
            }
            let resolved = parts.join("/");
            assert!(
                pages.iter().any(|(page, _)| *page == resolved),
                "{name} links to missing page {target}"
            );
        }
    }
}
