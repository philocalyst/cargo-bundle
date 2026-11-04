//! macOS `*.lproj/InfoPlist.strings` localization.

use super::{normalized_locale_code, validate_locale_codes};
use crate::bundle::common;
use anyhow::Context;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write;
use std::path::Path;

/// Locale-keyed InfoPlist string tables for macOS app bundles.
///
/// Mirrors the Cargo.toml shape:
///
/// ```toml
/// [package.metadata.bundle.osx.localizations.fr]
/// CFBundleDisplayName = "Mon Application"
/// ```
#[derive(Clone, Copy, Debug)]
pub struct OsxLocalizations<'a> {
    localized_strings: &'a HashMap<String, HashMap<String, String>>,
}

impl<'a> OsxLocalizations<'a> {
    pub fn new(localized_strings: &'a HashMap<String, HashMap<String, String>>) -> Self {
        Self { localized_strings }
    }

    /// Writes `*.lproj/InfoPlist.strings` under the provided `output_root`
    /// (typically `Contents/Resources`).
    pub fn write_to_directory(&self, output_root: &Path) -> crate::Result<()> {
        let locales = normalized_locales(self.localized_strings)?;
        for (locale_code, strings_mapping) in locales {
            self.write_single_locale_directory(output_root, &locale_code, strings_mapping)?;
        }
        Ok(())
    }

    /// Handles the directory creation and file initialization for a single locale.
    fn write_single_locale_directory(
        &self,
        output_root: &Path,
        locale_code: &str,
        strings_mapping: &HashMap<String, String>,
    ) -> crate::Result<()> {
        let localization_directory = output_root.join(format!("{locale_code}.lproj"));

        fs::create_dir_all(&localization_directory).with_context(|| {
            format!("Failed to create localization directory at {localization_directory:?}")
        })?;

        let strings_file_path = localization_directory.join("InfoPlist.strings");
        let mut output_file = common::create_file(&strings_file_path)?;

        Self::write_key_value_pairs(&mut output_file, strings_mapping)?;
        output_file.flush()?;

        Ok(())
    }

    /// Formats and writes the specific key-value string pairs into the target file.
    fn write_key_value_pairs(
        output_file: &mut impl Write,
        strings_mapping: &HashMap<String, String>,
    ) -> crate::Result<()> {
        let mut pairs: Vec<_> = strings_mapping.iter().collect();
        pairs.sort_by(|a, b| a.0.cmp(b.0));
        for (key, value) in pairs {
            writeln!(
                output_file,
                "\"{}\" = \"{}\";",
                escape_legacy_apple_string(key),
                escape_legacy_apple_string(value)
            )?;
        }
        Ok(())
    }
}

/// Escapes embedded backslashes and double-quotes for the legacy `.strings` file format.
fn escape_legacy_apple_string(value: &str) -> String {
    let mut escaped_string = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped_string.push_str(r"\\"),
            '"' => escaped_string.push_str(r#"\""#),
            '\n' => escaped_string.push_str(r"\n"),
            '\r' => escaped_string.push_str(r"\r"),
            '\t' => escaped_string.push_str(r"\t"),
            '\u{0008}' => escaped_string.push_str(r"\b"),
            '\u{000c}' => escaped_string.push_str(r"\f"),
            c if c.is_control() => {
                use std::fmt::Write as _;
                write!(&mut escaped_string, "\\U{:04X}", c as u32).unwrap();
            }
            _ => escaped_string.push(character),
        }
    }
    escaped_string
}

/// Validate the path component and reject locale encodings that collapse to
/// the same Apple `.lproj` directory.
fn normalized_locales(
    strings: &HashMap<String, HashMap<String, String>>,
) -> crate::Result<BTreeMap<String, &HashMap<String, String>>> {
    let mut entries: Vec<_> = strings.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    validate_locale_codes(entries.iter().map(|(locale, _)| locale.as_str()))?;
    let mut locales = BTreeMap::new();
    for (locale, mapping) in entries {
        let normalized = normalized_locale_code(locale);
        if locales.insert(normalized.clone(), mapping).is_some() {
            anyhow::bail!(
                "Multiple localization codes resolve to the Apple locale directory {normalized:?}"
            );
        }
    }
    Ok(locales)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn write_to_directory_creates_lproj_strings() {
        let mut localized_strings = HashMap::new();
        localized_strings.insert(
            "fr".to_string(),
            HashMap::from([("CFBundleDisplayName".into(), "Mon Application".into())]),
        );
        localized_strings.insert(
            "de".to_string(),
            HashMap::from([("CFBundleDisplayName".into(), "Meine Anwendung".into())]),
        );

        let localizations = OsxLocalizations::new(&localized_strings);
        let temporary_directory = tempdir().unwrap();

        localizations
            .write_to_directory(temporary_directory.path())
            .unwrap();

        let french_output = std::fs::read_to_string(
            temporary_directory
                .path()
                .join("fr.lproj/InfoPlist.strings"),
        )
        .unwrap();
        assert!(french_output.contains(r#""CFBundleDisplayName" = "Mon Application";"#));

        let german_output = std::fs::read_to_string(
            temporary_directory
                .path()
                .join("de.lproj/InfoPlist.strings"),
        )
        .unwrap();
        assert!(german_output.contains(r#""CFBundleDisplayName" = "Meine Anwendung";"#));
    }

    #[test]
    fn write_to_directory_escapes_quotes() {
        let mut localized_strings = HashMap::new();
        localized_strings.insert(
            "en".to_string(),
            HashMap::from([("CFBundleName".into(), r#"Say "hi""#.into())]),
        );

        let localizations = OsxLocalizations::new(&localized_strings);
        let temporary_directory = tempdir().unwrap();

        localizations
            .write_to_directory(temporary_directory.path())
            .unwrap();

        let english_output = std::fs::read_to_string(
            temporary_directory
                .path()
                .join("en.lproj/InfoPlist.strings"),
        )
        .unwrap();
        assert!(english_output.contains(r#""CFBundleName" = "Say \"hi\"";"#));
    }

    #[test]
    fn sorts_and_escapes_keys_values_and_controls() {
        let localized_strings = HashMap::from([(
            "en".to_string(),
            HashMap::from([
                ("z\"key".into(), "line\n\t\u{0001}".into()),
                ("a".into(), "back\\slash".into()),
            ]),
        )]);
        let localizations = OsxLocalizations::new(&localized_strings);
        let temporary_directory = tempdir().unwrap();
        localizations
            .write_to_directory(temporary_directory.path())
            .unwrap();
        let output = fs::read_to_string(
            temporary_directory
                .path()
                .join("en.lproj/InfoPlist.strings"),
        )
        .unwrap();
        assert_eq!(
            output,
            "\"a\" = \"back\\\\slash\";\n\"z\\\"key\" = \"line\\n\\t\\U0001\";\n"
        );
    }

    #[test]
    fn rejects_unsafe_and_colliding_locale_codes() {
        for locale in ["../escape", "a/b", "a\\b", ".."] {
            let localized_strings = HashMap::from([(locale.to_string(), HashMap::new())]);
            let localizations = OsxLocalizations::new(&localized_strings);
            assert!(localizations.write_to_directory(Path::new("/tmp")).is_err());
        }
        let localized_strings = HashMap::from([
            ("en.UTF-8".to_string(), HashMap::new()),
            ("en".to_string(), HashMap::new()),
        ]);
        let localizations = OsxLocalizations::new(&localized_strings);
        assert!(localizations.write_to_directory(Path::new("/tmp")).is_err());
    }
}
