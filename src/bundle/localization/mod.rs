//! Shared localization API for platform bundles.
//!
//! Cargo.toml stores locale tables as:
//!
//! ```toml
//! [package.metadata.bundle.osx.localizations.fr]
//! CFBundleDisplayName = "Mon Application"
//!
//! [package.metadata.bundle.linux.localizations.fr]
//! Name = "Mon App"
//! Comment = "Une description"
//! ```
//!
//! Both shapes are a locale-code → platform-specific strings map. Platform
//! writers wrap that map and expose methods directly as inherent implementations.

mod linux;
mod osx;

pub use linux::{LinuxDesktopLocale, LinuxDesktopLocalizations};
pub use osx::OsxLocalizations;

// Re-exported for unit tests and any future desktop-entry helpers.
#[cfg(test)]
pub use linux::DesktopKeywords;

// Used by the FreeDesktop desktop-file renderer.
pub(crate) use linux::escape_desktop_value;

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

/// Locales sorted by code for deterministic artifact output.
pub(crate) fn sorted_locales<E>(map: &HashMap<String, E>) -> BTreeMap<Cow<'_, str>, &E> {
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    entries
        .into_iter()
        .map(|(locale, entry)| (Cow::Owned(normalized_locale_code(locale)), entry))
        .collect()
}

pub(crate) fn normalized_locale_code(locale: &str) -> String {
    let (base, modifier) = locale.split_once('@').unwrap_or((locale, ""));
    let base = base.split_once('.').map_or(base, |(prefix, _)| prefix);
    if modifier.is_empty() {
        base.to_owned()
    } else {
        format!("{base}@{modifier}")
    }
}

/// Reject path-like locale codes and normalization collisions before a bundle
/// writer maps locale names to directories or platform keys.
pub(crate) fn validate_locale_codes<'a>(
    locales: impl IntoIterator<Item = &'a str>,
) -> crate::Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for locale in locales {
        if locale.is_empty()
            || locale == "."
            || locale == ".."
            || locale.contains(['/', '\\'])
            || !valid_locale_syntax(locale)
        {
            anyhow::bail!(
                "Invalid localization code {locale:?}: expected a single safe locale path component"
            );
        }
        let normalized = normalized_locale_code(locale);
        if !seen.insert(normalized.clone()) {
            anyhow::bail!("Multiple localization codes resolve to {normalized:?}");
        }
    }
    Ok(())
}

fn valid_locale_syntax(locale: &str) -> bool {
    let (base, modifier) = locale.split_once('@').unwrap_or((locale, ""));
    if locale.matches('@').count() > 1
        || base.is_empty()
        || (locale.contains('@') && modifier.is_empty())
    {
        return false;
    }
    let (language, encoding) = base.split_once('.').unwrap_or((base, ""));
    if base.matches('.').count() > 1
        || language.is_empty()
        || (base.contains('.') && encoding.is_empty())
    {
        return false;
    }
    language
        .chars()
        .chain(encoding.chars())
        .chain(modifier.chars())
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

/// Preferred fallback locales when a platform needs an unlocalized base value.
pub(crate) const FALLBACK_LOCALE_PREFERENCE: &[&str] = &["C", "en"];
