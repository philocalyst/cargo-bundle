mod format;
mod localizations;
mod manifest;
mod resources;

pub use format::{BuildArtifact, PackageType};
pub use manifest::{AndroidSettings, DesktopAction, LinuxSigningSettings, WindowsSigningSettings};
pub use resources::ResourcePaths;

use super::category::AppCategory;
use super::common::print_warning;
use super::localization::{LinuxDesktopLocalizations, OsxLocalizations};
use cargo_metadata::{Metadata, MetadataCommand, Package, TargetKind};
use localizations::load_localization_files;
use manifest::{BundleSettings, normalize_platform_settings, validate_platform_settings};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use target_build_utils::TargetInfo;

#[derive(Clone, Debug)]
pub struct Settings {
    package: cargo_metadata::Package,
    package_type: PackageType,
    /// Explicit target triples; empty means "build for the host". More than
    /// one produces a universal binary combined with `lipo` (macOS only).
    targets: Vec<(String, TargetInfo)>,
    features: Option<String>,
    project_out_directory: PathBuf,
    build_artifact: BuildArtifact,
    profile: String,
    all_features: bool,
    no_default_features: bool,
    prebuilt_binary: bool,
    binary_path: PathBuf,
    /// Per-target binaries that `lipo` combines into `binary_path` when more
    /// than one target triple was requested; empty otherwise.
    universal_input_binary_paths: Vec<PathBuf>,
    binary_name: String,
    bundle_settings: BundleSettings,
}

/// Try to load `Cargo.toml` file in the specified directory
fn load_metadata(dir: &Path) -> crate::Result<Metadata> {
    let cargo_file_path = dir.join("Cargo.toml");
    Ok(MetadataCommand::new()
        .manifest_path(cargo_file_path)
        .current_dir(dir)
        .no_deps()
        .exec()?)
}

impl Settings {
    pub fn new(current_dir: PathBuf, cli: &crate::Cli) -> crate::Result<Self> {
        let requested_package_type = cli.format;
        let profile = if cli.release {
            "release".to_string()
        } else if let Some(profile) = cli.profile.as_ref() {
            if profile == "debug" {
                anyhow::bail!("Profile name `debug` is reserved")
            }
            profile.to_string()
        } else {
            "dev".to_string()
        };
        let all_features = cli.all_features;
        let no_default_features = cli.no_default_features;
        let targets = cli
            .target
            .iter()
            .map(|triple| Ok((triple.to_string(), TargetInfo::from_str(triple)?)))
            .collect::<crate::Result<Vec<_>>>()?;
        if let Some((_, first_target)) = targets.first() {
            let first_os = first_target.target_os();
            if targets
                .iter()
                .any(|(_, target)| target.target_os() != first_os)
            {
                anyhow::bail!("All target triples must use the same operating system");
            }
            if targets.len() > 1 && first_os != "macos" {
                anyhow::bail!(
                    "Multiple target triples are supported only for universal macOS bundles"
                );
            }
        }
        let build_artifact = if let Some(bin) = cli.bin.as_ref() {
            BuildArtifact::Bin(bin.to_string())
        } else if let Some(example) = cli.example.as_ref() {
            BuildArtifact::Example(example.to_string())
        } else {
            BuildArtifact::Main
        };
        let features = cli.features.as_ref().map(|features| features.into());
        let cargo_settings = load_metadata(&current_dir)?;
        let package = Settings::find_bundle_package(cli.package.as_deref(), &cargo_settings)?;
        let target_os = targets
            .first()
            .map(|(_, info)| info.target_os())
            .unwrap_or(std::env::consts::OS);
        let package_type = match requested_package_type {
            Some(format) => format,
            None => PackageType::default_for_os(target_os)?,
        };
        if package_type == PackageType::AndroidApk && target_os != "android" {
            anyhow::bail!(
                "APK packaging requires an Android target, e.g. --target aarch64-linux-android"
            );
        }
        let bundle_settings = Settings::bundle_settings_of_package(package, target_os)?;
        let target_base = cargo_settings.target_directory.as_std_path();
        // With multiple targets the per-target binaries are combined into a
        // universal binary living under its own `universal` directory.
        let target_dir_name = match targets.as_slice() {
            [] => None,
            [(triple, _)] => Some(triple.as_str()),
            _ => Some("universal"),
        };
        let target_dir =
            Settings::get_target_dir(target_base, target_dir_name, &profile, &build_artifact);
        let (mut bundle_settings, mut binary_name) = match &build_artifact {
            BuildArtifact::Main if package_type == PackageType::AndroidApk => {
                let target = package
                    .targets
                    .iter()
                    .find(|target| {
                        target
                            .crate_types
                            .iter()
                            .any(|kind| kind.to_string() == "cdylib")
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "APK packaging requires a [lib] target with crate-type = [\"cdylib\"]"
                        )
                    })?;
                (
                    bundle_settings,
                    format!("lib{}.so", target.name.replace('-', "_")),
                )
            }
            BuildArtifact::Main => {
                if let Some(target) = package
                    .targets
                    .iter()
                    .find(|target| target.kind.contains(&TargetKind::Bin))
                {
                    (bundle_settings, target.name.clone())
                } else {
                    anyhow::bail!("No `bin` target is found in package '{}'", package.name)
                }
            }
            BuildArtifact::Bin(name) => (
                bundle_settings_from_table(&bundle_settings.bin, "bin", name)?,
                name.clone(),
            ),
            BuildArtifact::Example(name) => (
                bundle_settings_from_table(&bundle_settings.example, "example", name)?,
                name.clone(),
            ),
        };
        load_localization_files(
            &mut bundle_settings,
            Path::new(&package.manifest_path)
                .parent()
                .unwrap_or_else(|| Path::new(".")),
            target_os,
        )?;
        let binary_extension = match package_type {
            PackageType::WindowsMsi | PackageType::WxsMsi | PackageType::WindowsBundle => ".exe",
            _ if target_os == "windows" => ".exe",
            _ => "",
        };
        binary_name += binary_extension;
        let prebuilt_binary = cli.binary_path.is_some();
        let binary_path = if let Some(path) = &cli.binary_path {
            let path = if path.is_absolute() {
                path.clone()
            } else {
                current_dir.join(path)
            };
            let metadata = std::fs::metadata(&path).map_err(|error| {
                anyhow::anyhow!("Failed to read prebuilt binary {path:?}: {error}")
            })?;
            if !metadata.is_file() {
                anyhow::bail!("Prebuilt binary path is not a file: {path:?}");
            }
            path
        } else {
            target_dir.join(&binary_name)
        };
        let universal_input_binary_paths = if !prebuilt_binary && targets.len() > 1 {
            targets
                .iter()
                .map(|(triple, _)| {
                    Settings::get_target_dir(target_base, Some(triple), &profile, &build_artifact)
                        .join(&binary_name)
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(Settings {
            package: package.clone(),
            package_type,
            targets,
            features,
            build_artifact,
            profile,
            all_features,
            no_default_features,
            prebuilt_binary,
            project_out_directory: target_dir,
            binary_path,
            universal_input_binary_paths,
            binary_name,
            bundle_settings,
        })
    }

    pub fn android(&self) -> AndroidSettings {
        self.bundle_settings.android.clone().unwrap_or_default()
    }

    pub fn manifest_path(&self) -> &Path {
        Path::new(&self.package.manifest_path)
    }

    /// Name of the selected Cargo package to build.
    pub fn package_name(&self) -> &str {
        &self.package.name
    }

    /// Resolve a build artifact beneath Cargo's already-resolved target directory.
    fn get_target_dir(
        target_base: &Path,
        target_dir_name: Option<&str>,
        profile: &str,
        build_artifact: &BuildArtifact,
    ) -> PathBuf {
        let mut path = target_base.to_path_buf();
        if let Some(name) = target_dir_name {
            path.push(name);
        }
        path.push(if profile == "dev" { "debug" } else { profile });
        if matches!(build_artifact, BuildArtifact::Example(_)) {
            path.push("examples");
        }
        path
    }

    fn find_bundle_package<'a>(
        package: Option<&'a str>,
        metadata: &'a Metadata,
    ) -> crate::Result<&'a Package> {
        match package {
            Some(package) => metadata
                .packages
                .iter()
                .find(|p| p.name.as_str() == package)
                .ok_or_else(|| anyhow::anyhow!("Package '{package}' not found in workspace")),
            None => metadata
                .root_package()
                .ok_or_else(|| anyhow::anyhow!("No root package found in workspace")),
        }
    }

    fn bundle_settings_of_package(
        package: &Package,
        target_os: &str,
    ) -> crate::Result<BundleSettings> {
        if let Some(bundle) = package.metadata.get("bundle") {
            validate_platform_settings(bundle)?;
            let mut bundle = bundle.clone();
            normalize_platform_settings(&mut bundle, target_os)?;
            return Ok(serde_json::from_value::<BundleSettings>(bundle)?);
        }
        print_warning(&format!(
            "No [package.metadata.bundle] section in package \"{}\"",
            package.name
        ))?;
        Ok(BundleSettings::default())
    }

    /// Returns the directory where the bundle should be placed.
    pub fn project_out_directory(&self) -> &Path {
        &self.project_out_directory
    }

    /// Returns the architecture for the binary being bundled (e.g. "arm" or
    /// "x86" or "x86_64").
    pub fn binary_arch(&self) -> &str {
        if let Some((_, info)) = self.targets.first() {
            info.target_arch()
        } else {
            std::env::consts::ARCH
        }
    }

    pub fn target_os(&self) -> &str {
        self.targets
            .first()
            .map(|(_, info)| info.target_os())
            .unwrap_or(std::env::consts::OS)
    }

    /// Returns the file name of the binary being bundled.
    pub fn binary_name(&self) -> &str {
        &self.binary_name
    }

    /// Returns the path to the binary being bundled.
    pub fn binary_path(&self) -> &Path {
        &self.binary_path
    }

    /// Whether the executable was supplied with `--binary-path` rather than
    /// being produced by cargo-bundle's own `cargo build` invocation.
    pub fn uses_prebuilt_binary(&self) -> bool {
        self.prebuilt_binary
    }

    /// The format selected explicitly or inferred when settings were loaded.
    pub fn package_type(&self) -> PackageType {
        self.package_type
    }

    /// Every target triple requested on the command line; empty when
    /// building for the host.
    pub fn target_triples(&self) -> impl Iterator<Item = &str> {
        self.targets.iter().map(|(triple, _)| triple.as_str())
    }

    /// The per-target binaries that must be combined with `lipo` into
    /// `binary_path`; empty unless more than one target was requested.
    pub fn universal_input_binary_paths(&self) -> &[PathBuf] {
        &self.universal_input_binary_paths
    }

    pub fn features(&self) -> Option<&str> {
        match self.features {
            Some(ref features) => Some(features.as_str()),
            None => None,
        }
    }

    /// Returns the artifact that is being bundled.
    pub fn build_artifact(&self) -> &BuildArtifact {
        &self.build_artifact
    }

    /// Returns `release`, 'dev` or other profile.
    pub fn build_profile(&self) -> &str {
        &self.profile
    }

    pub fn all_features(&self) -> bool {
        self.all_features
    }

    pub fn no_default_features(&self) -> bool {
        self.no_default_features
    }

    pub fn bundle_name(&self) -> &str {
        self.bundle_settings
            .name
            .as_ref()
            .unwrap_or(&self.package.name)
    }

    pub fn bundle_identifier(&self) -> Cow<'_, str> {
        if let Some(identifier) = &self.bundle_settings.identifier {
            identifier.into()
        } else {
            match &self.build_artifact {
                BuildArtifact::Main => "".into(),
                BuildArtifact::Bin(name) => format!("{name}.{}", self.package.name).into(),
                BuildArtifact::Example(name) => {
                    format!("{name}.example.{}", self.package.name).into()
                }
            }
        }
    }

    /// Returns an iterator over the icon files to be used for this bundle.
    pub fn icon_files(&self) -> ResourcePaths<'_> {
        match self.bundle_settings.icon {
            Some(ref paths) => ResourcePaths::new(paths.as_slice(), false),
            None => ResourcePaths::new(&[], false),
        }
    }

    /// Returns an iterator over the resource files to be included in this
    /// bundle.
    pub fn resource_files(&self) -> ResourcePaths<'_> {
        match self.bundle_settings.resources {
            Some(ref paths) => ResourcePaths::new(paths.as_slice(), true),
            None => ResourcePaths::new(&[], true),
        }
    }

    pub fn version_string(&self) -> &dyn Display {
        match self.bundle_settings.version.as_ref() {
            Some(v) => v,
            None => &self.package.version,
        }
    }

    pub fn copyright_string(&self) -> Option<&str> {
        self.bundle_settings.copyright.as_deref()
    }

    pub fn author_names(&self) -> &[String] {
        &self.package.authors
    }

    pub fn authors_comma_separated(&self) -> Option<String> {
        let names = self.author_names();
        if names.is_empty() {
            None
        } else {
            Some(names.join(", "))
        }
    }

    pub fn homepage_url(&self) -> &str {
        self.package.homepage.as_deref().unwrap_or("")
    }

    pub fn app_category(&self) -> Option<AppCategory> {
        self.bundle_settings.category
    }

    pub fn short_description(&self) -> &str {
        self.bundle_settings
            .short_description
            .as_deref()
            .unwrap_or_else(|| self.package.description.as_deref().unwrap_or(""))
    }

    pub fn long_description(&self) -> Option<&str> {
        self.bundle_settings.long_description.as_deref()
    }

    pub fn license_content(&self) -> Option<String> {
        self.package
            .license_file
            .as_ref()
            .and_then(|license_file| {
                let dir = self
                    .manifest_path()
                    .parent()
                    .unwrap_or_else(|| Path::new("."));

                let license_path = dir.join(license_file);
                match std::fs::read_to_string(&license_path) {
                    Ok(content) => Some(content),
                    Err(err) => {
                        print_warning(&format!(
                            "Failed to read license file '{license_path:?}': {err} -- ignoring",
                        ))
                        .ok();
                        None
                    }
                }
            })
            .or_else(|| self.package.license.as_ref().map(|s| s.to_string()))
    }

    pub fn debian_dependencies(&self) -> &[String] {
        match self.bundle_settings.deb_depends {
            Some(ref dependencies) => dependencies.as_slice(),
            None => &[],
        }
    }

    pub fn linux_mime_types(&self) -> &[String] {
        self.bundle_settings
            .linux
            .as_ref()
            .and_then(|linux| linux.mime_types.as_deref())
            .unwrap_or_default()
    }

    pub fn linux_use_terminal(&self) -> Option<bool> {
        self.bundle_settings
            .linux
            .as_ref()
            .and_then(|linux| linux.use_terminal)
    }

    pub fn linux_exec_args(&self) -> Option<&str> {
        self.bundle_settings
            .linux
            .as_ref()
            .and_then(|linux| linux.exec_args.as_deref())
    }

    /// Path to an AppStream metainfo XML to bundle in the AppImage.
    pub fn appimage_metainfo_path(&self) -> Option<&str> {
        self.bundle_settings.appimage_metainfo_path.as_deref()
    }

    /// SquashFS compression codec: `"gzip"` (default), `"lz4"`, `"lzo"`, or `"none"`.
    pub fn appimage_compression(&self) -> Option<&str> {
        self.bundle_settings.appimage_compression.as_deref()
    }

    /// `StartupWMClass` for the `.desktop` entry.
    pub fn linux_startup_wm_class(&self) -> Option<&str> {
        self.bundle_settings
            .linux
            .as_ref()
            .and_then(|linux| linux.startup_wm_class.as_deref())
    }

    /// Desktop actions to emit as `[Desktop Action <id>]` groups.
    pub fn linux_desktop_actions(&self) -> Option<&HashMap<String, DesktopAction>> {
        self.bundle_settings
            .linux
            .as_ref()
            .and_then(|linux| linux.desktop_actions.as_ref())
    }

    pub fn osx_frameworks(&self) -> &[String] {
        self.bundle_settings
            .osx
            .as_ref()
            .and_then(|osx| osx.frameworks.as_deref())
            .unwrap_or_default()
    }

    pub fn osx_plugins(&self) -> &[String] {
        self.bundle_settings
            .osx
            .as_ref()
            .and_then(|osx| osx.plugins.as_deref())
            .unwrap_or_default()
    }

    pub fn osx_minimum_system_version(&self) -> Option<&str> {
        self.bundle_settings
            .osx
            .as_ref()
            .and_then(|osx| osx.minimum_system_version.as_deref())
    }

    pub fn osx_url_schemes(&self) -> &[String] {
        self.bundle_settings
            .osx
            .as_ref()
            .and_then(|osx| osx.url_schemes.as_deref())
            .unwrap_or_default()
    }

    /// Returns an iterator over the plist files for this bundle
    pub fn osx_info_plist_exts(&self) -> ResourcePaths<'_> {
        ResourcePaths::new(
            self.bundle_settings
                .osx
                .as_ref()
                .and_then(|osx| osx.info_plist_exts.as_deref())
                .unwrap_or_default(),
            false,
        )
    }

    /// macOS localizations as an [`OsxLocalizations`] wrapper.
    ///
    /// Writes `*.lproj/InfoPlist.strings` under the Resources directory.
    pub fn osx_localizations(&self) -> Option<OsxLocalizations<'_>> {
        self.bundle_settings
            .osx
            .as_ref()
            .and_then(|osx| osx.localizations.as_ref())
            .map(OsxLocalizations::new)
    }

    /// iOS Info.plist localizations, using the same strings writer as macOS.
    pub fn ios_localizations(&self) -> Option<OsxLocalizations<'_>> {
        self.bundle_settings
            .ios
            .as_ref()
            .and_then(|ios| ios.localizations.as_ref())
            .map(OsxLocalizations::new)
    }

    /// Linux desktop localizations as a [`LinuxDesktopLocalizations`] wrapper.
    ///
    /// Keys are inlined into the `.desktop` file as `Name[locale]=…` etc.
    /// Unlocalized `Name` / `Comment` still come from
    /// [`bundle_name`](Self::bundle_name) / [`short_description`](Self::short_description).
    pub fn linux_localizations(&self) -> Option<LinuxDesktopLocalizations<'_>> {
        self.bundle_settings
            .linux
            .as_ref()
            .and_then(|linux| linux.localizations.as_ref())
            .map(LinuxDesktopLocalizations::new)
    }

    /// An SVG that becomes the Finder window background of the DMG. It must
    /// contain elements with ids `app` and `applications`, whose centers
    /// become the icon positions.
    pub fn osx_dmg_background(&self) -> Option<&Path> {
        self.bundle_settings
            .osx
            .as_ref()
            .and_then(|osx| osx.dmg_background.as_deref())
    }

    /// PKCS#12 certificate used by the pure-Rust Apple signing backend.
    pub fn apple_signing_p12(&self) -> Option<&Path> {
        self.bundle_settings.apple_signing_p12.as_deref()
    }

    pub fn apple_provisioning_profile(&self) -> Option<&Path> {
        self.bundle_settings.apple_provisioning_profile.as_deref()
    }

    /// Environment variable containing the Apple PKCS#12 certificate password.
    pub fn apple_signing_password_env(&self) -> Option<&str> {
        self.bundle_settings.apple_signing_password_env.as_deref()
    }

    /// Optional RFC 3161 timestamp service URL for Apple code signatures.
    pub fn apple_signing_timestamp_url(&self) -> Option<&str> {
        self.bundle_settings.apple_signing_timestamp_url.as_deref()
    }

    /// Entitlements plist embedded in Apple code signatures.
    pub fn apple_signing_entitlements(&self) -> Option<&Path> {
        self.bundle_settings.apple_signing_entitlements.as_deref()
    }

    /// Whether to enable the hardened runtime during Apple code signing.
    pub fn apple_signing_hardened_runtime(&self) -> bool {
        self.bundle_settings
            .apple_signing_hardened_runtime
            .unwrap_or(false)
    }

    /// App Store Connect API key used for Apple notarization.
    pub fn apple_notarization_api_key(&self) -> Option<&Path> {
        self.bundle_settings.apple_notarization_api_key.as_deref()
    }

    /// Maximum notarization wait in seconds (600 seconds by default).
    #[cfg(feature = "apple-notarization")]
    pub fn apple_notarization_timeout(&self) -> u64 {
        self.bundle_settings
            .apple_notarization_timeout
            .unwrap_or(600)
    }

    /// Authenticode configuration, when Windows signing has been requested.
    pub fn windows_signing(&self) -> Option<&WindowsSigningSettings> {
        self.bundle_settings.windows_signing.as_ref()
    }

    /// Keyless Sigstore configuration for Linux release artifacts.
    pub fn linux_signing(&self) -> Option<&LinuxSigningSettings> {
        self.bundle_settings.linux_signing.as_ref()
    }
}

fn bundle_settings_from_table(
    opt_map: &Option<HashMap<String, BundleSettings>>,
    map_name: &str,
    bundle_name: &str,
) -> crate::Result<BundleSettings> {
    if let Some(bundle_settings) = opt_map.as_ref().and_then(|map| map.get(bundle_name)) {
        Ok(bundle_settings.clone())
    } else {
        print_warning(&format!(
            "No [package.metadata.bundle.{map_name}.{bundle_name}] section in Cargo.toml"
        ))?;
        Ok(BundleSettings::default())
    }
}

#[cfg(test)]
mod tests;
