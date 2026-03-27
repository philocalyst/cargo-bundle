// A macOS DMG (disk image) bundle is a compressed disk image that contains the
// application bundle and a symlink to /Applications so the user can simply
// drag-and-drop to install.
//
// The layout inside the mounted volume is:
//
//   <AppName>.dmg  (read-only compressed UDZO image)
//     <AppName>.app   # the application bundle
//     Applications TO /Applications
//
// Building requires macOS because the `hdiutil` command is used to create and
// convert the disk image.

use super::common;
use crate::Settings;
use crate::bundle::osx_bundle;
use anyhow::Context;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

pub fn bundle_project(settings: &Settings) -> crate::Result<Vec<PathBuf>> {
    let dmg_name = format!("{}.dmg", settings.bundle_name());
    common::print_bundling(&dmg_name)?;

    let base_dir = settings.project_out_directory().join("bundle/dmg");
    fs::create_dir_all(&base_dir)
        .with_context(|| format!("Failed to create bundle directory {base_dir:?}"))?;

    // Build the .app bundle into the DMG staging directory.
    let app_bundle_name = format!("{}.app", settings.bundle_name());
    let app_bundle_path = base_dir.join(&app_bundle_name);
    if app_bundle_path.exists() {
        fs::remove_dir_all(&app_bundle_path)
            .with_context(|| format!("Failed to remove existing {app_bundle_name}"))?;
    }

    osx_bundle::bundle_project_at(settings, &base_dir)
        .with_context(|| "Failed to create app bundle for DMG")?;

    let dmg_path = base_dir.join(&dmg_name);
    if dmg_path.exists() {
        fs::remove_file(&dmg_path)
            .with_context(|| format!("Failed to remove existing {dmg_name}"))?;
    }

    // Determine the size of the app bundle and add a generous overhead.
    let bundle_size = dir_size(&app_bundle_path)?;
    let image_size_bytes = (bundle_size + 52_428_800).max(52_428_800); // at least 50 MiB

    let temp_dir = tempfile::tempdir()
        .with_context(|| "Failed to create temporary directory for DMG staging")?;

    let staging_dmg = temp_dir.path().join("staging.dmg");

    // Create a writable HFS+ disk image large enough for the bundle.
    let status = Command::new("hdiutil")
        .args([
            "create",
            staging_dmg.to_str().unwrap(),
            "-ov",
            "-fs",
            "HFS+",
            "-size",
            &image_size_bytes.to_string(),
            "-volname",
            settings.bundle_name(),
        ])
        .status()
        .with_context(|| "Failed to run hdiutil create (macOS only)")?;

    if !status.success() {
        anyhow::bail!("hdiutil create failed");
    }

    // Mount the writable image.
    let output = Command::new("hdiutil")
        .args([
            "attach",
            staging_dmg.to_str().unwrap(),
            "-nobrowse",
            "-noverify",
            "-noautoopen",
            "-noautofsck",
        ])
        .output()
        .with_context(|| "Failed to mount staging DMG")?;

    if !output.status.success() {
        anyhow::bail!("hdiutil attach failed");
    }

    let mount_point = parse_mount_point(&output.stdout)
        .with_context(|| "Could not determine DMG mount point from hdiutil output")?;

    // Copy the app bundle and create the /Applications symlink inside the image.
    let copy_result = (|| -> crate::Result<()> {
        common::copy_dir(&app_bundle_path, &mount_point.join(&app_bundle_name))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink("/Applications", mount_point.join("Applications"))
            .with_context(|| "Failed to create /Applications symlink")?;
        Ok(())
    })();

    // Always unmount, even if copying failed.
    let _ = Command::new("hdiutil")
        .args(["detach", mount_point.to_str().unwrap()])
        .status();

    copy_result?;

    // Convert the writable image to a read-only compressed UDZO image.
    let status = Command::new("hdiutil")
        .args([
            "convert",
            staging_dmg.to_str().unwrap(),
            "-ov",
            "-format",
            "UDZO",
            "-imagekey",
            "zlib-level=9",
            "-o",
            dmg_path.to_str().unwrap(),
        ])
        .status()
        .with_context(|| "Failed to run hdiutil convert")?;

    if !status.success() {
        anyhow::bail!("hdiutil convert failed");
    }

    Ok(vec![dmg_path])
}

/// Walk a directory and sum the sizes of all contained files.
fn dir_size(dir: &std::path::Path) -> crate::Result<u64> {
    let mut total: u64 = 0;
    for entry in walkdir::WalkDir::new(dir) {
        let entry = entry?;
        if entry.file_type().is_file() {
            total += entry.metadata()?.len();
        }
    }
    Ok(total)
}

/// Extract the mount point path from `hdiutil attach` stdout.
fn parse_mount_point(stdout: &[u8]) -> crate::Result<PathBuf> {
    parse_mount_point_impl(stdout)
}

fn parse_mount_point_impl(stdout: &[u8]) -> crate::Result<PathBuf> {
    let text = std::str::from_utf8(stdout)?;
    // hdiutil attach prints a tab-separated line whose last field is the mount
    // point, EXAMPLE:  /dev/disk2s1   Apple_HFS  /Volumes/MyApp
    for line in text.lines().rev() {
        let parts: Vec<&str> = line.split('\t').collect();
        if let Some(path) = parts.last() {
            let path = path.trim();
            if path.starts_with("/Volumes/") {
                return Ok(PathBuf::from(path));
            }
        }
    }
    anyhow::bail!("Could not find a /Volumes/… mount point in hdiutil output")
}
