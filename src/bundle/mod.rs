mod category;
mod common;
mod dmg_bundle;
mod ios_bundle;
mod linux;
mod msi_bundle;
mod osx_bundle;
mod settings;
#[cfg(target_os = "windows")]
mod windows;
mod wxsmsi_bundle;

pub use self::common::{print_error, print_finished};
use self::linux::appimage_bundle;
pub use self::settings::{BuildArtifact, PackageType, Settings};
use crate::bundle::linux::{deb_bundle, rpm_bundle};
use std::path::PathBuf;

pub fn bundle_project(settings: Settings) -> crate::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for package_type in settings.package_types()? {
        paths.append(&mut match package_type {
            PackageType::OsxBundle => osx_bundle::bundle_project(&settings)?,
            PackageType::OsxDmg => dmg_bundle::bundle_project(&settings)?,
            PackageType::IosBundle => ios_bundle::bundle_project(&settings)?,
            PackageType::WindowsMsi => msi_bundle::bundle_project(&settings)?,
            PackageType::WxsMsi => wxsmsi_bundle::bundle_project(&settings)?,
            PackageType::WindowsBundle => {
                #[cfg(target_os = "windows")]
                {
                    windows::exe_bundle::bundle_project(&settings)?
                }
                #[cfg(not(target_os = "windows"))]
                {
                    anyhow::bail!(".exe bundling is only supported on Windows hosts");
                }
            }
            PackageType::Deb => deb_bundle::bundle_project(&settings)?,
            PackageType::Rpm => rpm_bundle::bundle_project(&settings)?,
            PackageType::AppImage => appimage_bundle::bundle_project(&settings)?,
        });
    }
    Ok(paths)
}
