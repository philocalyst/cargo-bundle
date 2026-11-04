//! Manifest schema and platform overrides.

use super::super::category::AppCategory;
use super::super::localization::{LinuxDesktopLocale, validate_locale_codes};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

/// A single FreeDesktop desktop action.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct DesktopAction {
    /// Localizable display name for the action (required by the spec).
    #[serde(rename = "Name")]
    pub name: String,

    /// Command to execute; falls back to the app binary when absent.
    #[serde(rename = "Exec", default)]
    pub exec: Option<String>,

    /// Icon for this action.
    #[serde(rename = "Icon", default)]
    pub icon: Option<String>,

    /// Per-locale names.
    #[serde(rename = "NameLocalized", default)]
    pub name_localized: Option<HashMap<String, String>>, // local code to translation
}

/// Windows Authenticode signing configuration. This is available only with
/// cargo-bundle's `windows-signing` feature because its implementation links
/// GPL-3.0-or-later code.
#[derive(Clone, Debug, serde::Deserialize)]
#[cfg_attr(not(feature = "windows-signing"), allow(dead_code))]
pub struct WindowsSigningSettings {
    /// Path to a PKCS#12 (`.p12`/`.pfx`) signing certificate.
    pub certificate_path: PathBuf,
    /// Name of the environment variable containing the certificate password.
    #[serde(default)]
    pub certificate_password_env: Option<String>,
    /// Optional RFC 3161 timestamp service URL.
    #[serde(default)]
    pub timestamp_url: Option<String>,
}

/// Keyless Sigstore configuration for Linux release artifacts.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct LinuxSigningSettings {
    /// Environment variable containing an OIDC token minted for the
    /// `sigstore` audience.
    pub identity_token_env: String,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LinuxSettings {
    pub(super) mime_types: Option<Vec<String>>,
    pub(super) exec_args: Option<String>,
    pub(super) use_terminal: Option<bool>,
    pub(super) localizations: Option<HashMap<String, LinuxDesktopLocale>>,
    pub(super) startup_wm_class: Option<String>,
    pub(super) desktop_actions: Option<HashMap<String, DesktopAction>>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OsxSettings {
    pub(super) frameworks: Option<Vec<String>>,
    pub(super) plugins: Option<Vec<String>>,
    pub(super) minimum_system_version: Option<String>,
    pub(super) url_schemes: Option<Vec<String>>,
    pub(super) info_plist_exts: Option<Vec<String>>,
    pub(super) localizations: Option<HashMap<String, HashMap<String, String>>>,
    pub(super) dmg_background: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IosSettings {
    pub(super) localizations: Option<HashMap<String, HashMap<String, String>>>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BundleSettings {
    /// Package-manifest-relative TOML files containing platform localization tables.
    pub(super) localization_files: Option<Vec<PathBuf>>,
    // General settings:
    pub(super) name: Option<String>,
    pub(super) identifier: Option<String>,
    pub(super) icon: Option<Vec<String>>,
    pub(super) version: Option<String>,
    pub(super) resources: Option<Vec<String>>,
    pub(super) copyright: Option<String>,
    pub(super) category: Option<AppCategory>,
    pub(super) short_description: Option<String>,
    pub(super) long_description: Option<String>,
    /// Path to an AppStream metainfo XML file to bundle in the AppImage.
    pub(super) appimage_metainfo_path: Option<String>,
    /// SquashFS compression codec: `"gzip"` (default), `"lz4"`, `"lzo"`, or `"none"`.
    pub(super) appimage_compression: Option<String>,
    pub(super) deb_depends: Option<Vec<String>>,
    /// Linux-only packaging configuration.
    pub(super) linux: Option<LinuxSettings>,
    /// macOS-only packaging configuration.
    #[serde(alias = "macos")]
    pub(super) osx: Option<OsxSettings>,
    /// iOS Info.plist localizations.
    pub(super) ios: Option<IosSettings>,
    /// PKCS#12 certificate used by the pure-Rust Apple signing backend.
    /// The bundle is left unsigned when this is not configured.
    pub(super) apple_signing_p12: Option<PathBuf>,
    /// Environment variable containing the PKCS#12 certificate password.
    pub(super) apple_signing_password_env: Option<String>,
    /// Optional RFC 3161 timestamp service URL for Apple code signatures.
    pub(super) apple_signing_timestamp_url: Option<String>,
    /// Optional entitlements plist embedded in Apple code signatures.
    pub(super) apple_signing_entitlements: Option<PathBuf>,
    /// Enable the hardened runtime in Apple code signatures.
    pub(super) apple_signing_hardened_runtime: Option<bool>,
    /// Apple-issued provisioning profile embedded before code signing.
    pub(super) apple_provisioning_profile: Option<PathBuf>,
    /// App Store Connect API key used for Apple notarization.
    pub(super) apple_notarization_api_key: Option<PathBuf>,
    /// Maximum time to wait for Apple's notarization service, in seconds.
    pub(super) apple_notarization_timeout: Option<u64>,
    /// Optional Authenticode configuration for `.exe` and `.msi` output.
    pub(super) windows_signing: Option<WindowsSigningSettings>,
    /// Optional keyless Sigstore configuration for Linux release artifacts.
    pub(super) linux_signing: Option<LinuxSigningSettings>,
    // Bundles for other binaries/examples:
    pub(super) bin: Option<HashMap<String, BundleSettings>>,
    pub(super) example: Option<HashMap<String, BundleSettings>>,
}

/// Apply the selected target's table as a shallow settings overlay before
/// deserializing into the strict typed settings structs. Arrays and scalar
/// values are replaced as a whole, including empty arrays. Platform tables
/// for other operating systems are discarded so they cannot leak across.
pub(super) fn normalize_platform_settings(
    bundle: &mut Value,
    target_os: &str,
) -> crate::Result<()> {
    let Some(root) = bundle.as_object_mut() else {
        anyhow::bail!("package.metadata.bundle must be a table");
    };

    // Named targets use the same syntax and validation as the package root.
    for key in ["bin", "example"] {
        if let Some(Value::Object(targets)) = root.get_mut(key) {
            for target in targets.values_mut() {
                normalize_platform_settings(target, target_os)?;
            }
        }
    }

    if root.contains_key("osx") && root.contains_key("macos") {
        anyhow::bail!(
            "Use only one of [package.metadata.bundle.osx] or [package.metadata.bundle.macos]"
        );
    }
    let platform = match target_os {
        "linux" => "linux",
        "macos" => "osx",
        "windows" => "windows",
        "ios" => "ios",
        _ => return Ok(()),
    };
    let alias = (platform == "osx").then_some("macos");
    let mut overlay = alias
        .and_then(|alias| root.remove(alias))
        .or_else(|| root.remove(platform));

    // Remove every platform table after extracting the selected one. The
    // legacy Linux and macOS tables also contain backend-specific settings;
    // retain only those recognized fields for their typed backend structs.
    for other in ["linux", "osx", "macos", "windows", "ios", "android"] {
        if other != platform && Some(other) != alias {
            root.remove(other);
        }
    }
    let Some(overlay) = overlay.take() else {
        return Ok(());
    };
    let Value::Object(values) = overlay else {
        anyhow::bail!("[package.metadata.bundle.{platform}] must be a table");
    };

    let mut backend = serde_json::Map::new();
    let backend_keys: &[&str] = match platform {
        "linux" => &[
            "mime_types",
            "exec_args",
            "use_terminal",
            "localizations",
            "startup_wm_class",
            "desktop_actions",
        ],
        "osx" => &[
            "frameworks",
            "plugins",
            "minimum_system_version",
            "url_schemes",
            "info_plist_exts",
            "localizations",
            "dmg_background",
        ],
        "ios" => &["localizations"],
        _ => &[],
    };
    for (key, value) in values {
        if backend_keys.contains(&key.as_str()) {
            backend.insert(key, value);
        } else {
            if matches!(
                key.as_str(),
                "bin" | "example" | "linux" | "osx" | "macos" | "windows" | "ios" | "android"
            ) {
                anyhow::bail!("Nested settings table `{key}` is not a platform override");
            }
            root.insert(key, value);
        }
    }
    if !backend.is_empty() {
        root.insert(platform.to_owned(), Value::Object(backend));
    }
    serde_json::from_value::<BundleSettings>(bundle.clone())?;
    Ok(())
}

/// Validate every declared platform table, even when the current build only
/// selects one of them, so malformed release configurations fail clearly.
pub(super) fn validate_platform_settings(bundle: &Value) -> crate::Result<()> {
    for os in ["linux", "macos", "windows", "ios"] {
        let mut candidate = bundle.clone();
        normalize_platform_settings(&mut candidate, os)?;
        let settings: BundleSettings = serde_json::from_value(candidate)?;
        validate_bundle_settings(&settings)?;
    }
    Ok(())
}

pub(super) fn validate_bundle_settings(settings: &BundleSettings) -> crate::Result<()> {
    if settings.apple_notarization_timeout == Some(0) {
        anyhow::bail!("apple_notarization_timeout must be greater than zero");
    }
    if let Some(locales) = settings
        .linux
        .as_ref()
        .and_then(|linux| linux.localizations.as_ref())
    {
        validate_locale_codes(locales.keys().map(String::as_str))?;
    }
    if let Some(locales) = settings
        .osx
        .as_ref()
        .and_then(|osx| osx.localizations.as_ref())
    {
        validate_locale_codes(locales.keys().map(String::as_str))?;
    }
    if let Some(locales) = settings
        .ios
        .as_ref()
        .and_then(|ios| ios.localizations.as_ref())
    {
        validate_locale_codes(locales.keys().map(String::as_str))?;
    }
    for targets in [&settings.bin, &settings.example].into_iter().flatten() {
        for target in targets.values() {
            validate_bundle_settings(target)?;
        }
    }
    Ok(())
}
