//! Merge split translation files, then apply inline values last.

use super::super::localization::{LinuxDesktopLocale, validate_locale_codes};
use super::manifest::{
    BundleSettings, IosSettings, LinuxSettings, OsxSettings, validate_bundle_settings,
};
use anyhow::Context;
use std::collections::HashMap;
use std::path::Path;

/// Read and merge the selected bundle's external locale tables. Files are
/// package-manifest-relative; later files override earlier files, and inline
/// manifest values are applied last.
pub(super) fn load_localization_files(
    bundle: &mut BundleSettings,
    package_dir: &Path,
    target_os: &str,
) -> crate::Result<()> {
    let Some(files) = bundle.localization_files.take() else {
        return Ok(());
    };
    let inline_linux = bundle
        .linux
        .as_mut()
        .and_then(|linux| linux.localizations.take());
    let inline_macos = bundle.osx.as_mut().and_then(|osx| osx.localizations.take());
    let inline_ios = bundle.ios.as_mut().and_then(|ios| ios.localizations.take());
    for file in files {
        let path = package_dir.join(&file);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read localization file {path:?}"))?;
        let external = toml::from_str(&text)
            .with_context(|| format!("Failed to parse localization file {path:?}"))?;
        let tables = parse_localization_file(external)
            .with_context(|| format!("Invalid localization file {path:?}"))?;
        match target_os {
            "linux" => {
                if let Some(values) = tables.linux {
                    merge_linux_localizations(&mut bundle.linux, values);
                }
            }
            "macos" => {
                if let Some(values) = tables.macos {
                    merge_apple_localizations(&mut bundle.osx, values);
                }
            }
            "ios" => {
                if let Some(values) = tables.ios {
                    merge_ios_localizations(&mut bundle.ios, values);
                }
            }
            _ => {}
        }
    }

    // Inline settings have precedence over every external file.
    if let Some(inline) = inline_linux {
        merge_linux_localizations(&mut bundle.linux, inline);
    }
    if let Some(inline) = inline_macos {
        merge_apple_localizations(&mut bundle.osx, inline);
    }
    if let Some(inline) = inline_ios {
        merge_ios_localizations(&mut bundle.ios, inline);
    }
    validate_bundle_settings(bundle)
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExternalLocalizations {
    pub(super) linux: Option<HashMap<String, LinuxDesktopLocale>>,
    #[serde(alias = "osx")]
    pub(super) macos: Option<HashMap<String, HashMap<String, String>>>,
    pub(super) ios: Option<HashMap<String, HashMap<String, String>>>,
}

pub(super) fn parse_localization_file(value: toml::Value) -> crate::Result<ExternalLocalizations> {
    let tables: ExternalLocalizations = value.try_into()?;
    if let Some(locales) = &tables.linux {
        validate_locale_codes(locales.keys().map(String::as_str))?;
    }
    if let Some(locales) = &tables.macos {
        validate_locale_codes(locales.keys().map(String::as_str))?;
    }
    if let Some(locales) = &tables.ios {
        validate_locale_codes(locales.keys().map(String::as_str))?;
    }
    Ok(tables)
}

fn merge_linux_localizations(
    target: &mut Option<LinuxSettings>,
    values: HashMap<String, LinuxDesktopLocale>,
) {
    let localizations = &mut target
        .get_or_insert_with(LinuxSettings::default)
        .localizations;
    let destination = localizations.get_or_insert_with(HashMap::new);
    for (locale, source) in values {
        let entry = destination.entry(locale).or_default();
        if source.name.is_some() {
            entry.name = source.name;
        }
        if source.generic_name.is_some() {
            entry.generic_name = source.generic_name;
        }
        if source.comment.is_some() {
            entry.comment = source.comment;
        }
        if source.keywords.is_some() {
            entry.keywords = source.keywords;
        }
        if source.icon.is_some() {
            entry.icon = source.icon;
        }
    }
}

fn merge_apple_localizations(
    target: &mut Option<OsxSettings>,
    values: HashMap<String, HashMap<String, String>>,
) {
    merge_string_localizations(
        &mut target
            .get_or_insert_with(OsxSettings::default)
            .localizations,
        values,
    );
}

fn merge_ios_localizations(
    target: &mut Option<IosSettings>,
    values: HashMap<String, HashMap<String, String>>,
) {
    merge_string_localizations(
        &mut target
            .get_or_insert_with(IosSettings::default)
            .localizations,
        values,
    );
}

fn merge_string_localizations(
    target: &mut Option<HashMap<String, HashMap<String, String>>>,
    values: HashMap<String, HashMap<String, String>>,
) {
    let destination = target.get_or_insert_with(HashMap::new);
    for (locale, strings) in values {
        destination.entry(locale).or_default().extend(strings);
    }
}
