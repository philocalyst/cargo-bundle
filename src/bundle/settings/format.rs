//! Supported output formats and Cargo artifact selection.

use clap::ValueEnum;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum PackageType {
    #[value(name = "apk")]
    AndroidApk,
    #[value(name = "deb")]
    Deb,
    #[value(name = "ios")]
    IosBundle,
    #[value(name = "ipa")]
    IosIpa,
    #[value(name = "msi")]
    WindowsMsi,
    #[value(name = "wxsmsi")]
    WxsMsi,
    #[value(name = "osx")]
    OsxBundle,
    #[value(name = "dmg")]
    OsxDmg,
    #[value(name = "rpm")]
    Rpm,
    #[value(name = "appimage")]
    AppImage,
    #[value(name = "exe")]
    WindowsBundle,
}

impl std::fmt::Display for PackageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.to_possible_value().ok_or(std::fmt::Error)?.get_name())
    }
}

impl PackageType {
    pub(super) fn default_for_os(os: &str) -> crate::Result<Self> {
        match os {
            "macos" => Ok(Self::OsxBundle),
            "ios" => Ok(Self::IosBundle),
            "linux" => Ok(Self::Deb),
            "android" => Ok(Self::AndroidApk),
            "windows" => Ok(Self::WindowsBundle),
            os => anyhow::bail!("Native {os} bundles not yet supported."),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum BuildArtifact {
    #[default]
    Main,
    Bin(String),
    Example(String),
}
