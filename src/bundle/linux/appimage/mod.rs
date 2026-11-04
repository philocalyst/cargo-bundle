//! Type-2 AppImage bundler.
//!
//! # Format
//!
//! A type-2 AppImage is:
//!   `[ELF type2 runtime][SquashFS 4.0 payload]`
//!
//! The embedded, statically linked Rust launcher extracts the SquashFS into a
//! private temporary directory, runs `AppRun`, and cleans up after it exits.
//! It does not require FUSE, mount helpers, or external extraction tools.
//!
//! # SquashFS strategy
//!
//! SquashFS images are assembled by the pure-Rust [`appimage`] crate. No
//! external `mksquashfs` binary or runtime download is required. The library
//! embeds launchers for each supported target architecture.
//!
//! # Layout
//!
//! ```text
//! AppDir/
//!   AppRun                      -> usr/bin/<bin>
//!   <bin>.desktop               -> usr/share/applications/<bin>.desktop
//!   <bin>.png | <bin>.svg       root icon (basename matches Icon=)
//!   .DirIcon                    PNG thumbnail
//!   usr/bin/<bin>
//!   usr/share/applications/<bin>.desktop
//!   usr/share/icons/hicolor/...
//!   usr/lib/<bin>/...           bundled resources (if any)
//!   usr/share/doc/<bin>/copyright
//! ```

mod app_dir;
mod arch;
mod context;

use crate::bundle::{Settings, common};
use anyhow::Context;
use app_dir::AppDirectory;
use context::AppImageContext;
use std::path::PathBuf;

pub fn bundle_project(settings: &Settings) -> crate::Result<Vec<PathBuf>> {
    let context = AppImageContext::new(settings)?;

    let package_name = context
        .package_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    common::print_bundling(&package_name)?;

    context.clean_previous_build()?;

    let app_directory = AppDirectory::build(settings, &context)?;
    let architecture = arch::determine_appimage_architecture(settings.binary_arch())?;
    let candidate = tempfile::NamedTempFile::new_in(
        context
            .package_path
            .parent()
            .context("AppImage output has no parent")?,
    )?
    .into_temp_path();
    appimage::AppImage::pack(&app_directory.path)
        .target_arch(architecture)
        .compression(compression(settings)?)
        .write_to(&candidate)
        .map_err(anyhow::Error::from)
        .with_context(|| format!("Failed to assemble {}", context.package_path.display()))?;

    std::fs::OpenOptions::new()
        .write(true)
        .open(&candidate)?
        .sync_all()?;
    candidate
        .persist(&context.package_path)
        .map_err(|error| error.error)
        .context("Failed to publish completed AppImage")?;

    Ok(vec![context.package_path.clone()])
}

fn compression(settings: &Settings) -> crate::Result<appimage::Compression> {
    match settings.appimage_compression().unwrap_or("gzip") {
        "gzip" => Ok(appimage::Compression::Gzip),
        "lz4" => Ok(appimage::Compression::Lz4),
        "lzo" => Ok(appimage::Compression::Lzo),
        "none" => Ok(appimage::Compression::None),
        other => anyhow::bail!(
            "appimage_compression must be \"gzip\", \"lz4\", \"lzo\", or \"none\", got {other:?}"
        ),
    }
}

fn set_executable_permissions(path: &std::path::Path) -> crate::Result<()> {
    #[cfg(unix)]
    {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)?;
    }
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::arch::determine_appimage_architecture;

    #[test]
    fn maps_common_cargo_arches() {
        assert_eq!(determine_appimage_architecture("x86_64").unwrap(), "x86_64");
        assert_eq!(
            determine_appimage_architecture("aarch64").unwrap(),
            "aarch64"
        );
        assert_eq!(determine_appimage_architecture("arm64").unwrap(), "aarch64");
        assert_eq!(determine_appimage_architecture("i686").unwrap(), "i686");
        assert_eq!(determine_appimage_architecture("x86").unwrap(), "i686");
        assert_eq!(determine_appimage_architecture("arm").unwrap(), "armhf");
        assert_eq!(determine_appimage_architecture("armv7").unwrap(), "armhf");
    }

    #[test]
    fn rejects_unsupported_arches() {
        assert!(determine_appimage_architecture("riscv64").is_err());
        assert!(determine_appimage_architecture("wasm32").is_err());
    }
}
