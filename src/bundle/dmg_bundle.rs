// A macOS DMG contains an application bundle, an Applications symlink, and
// optional Finder presentation files on an HFS+ volume.

use super::common;
use super::signing;
use crate::Settings;
use crate::bundle::osx_bundle;
use anyhow::Context;

use std::fs;
use std::path::{Path, PathBuf};

pub fn bundle_project(settings: &Settings) -> crate::Result<Vec<PathBuf>> {
    common::validate_bundle_name(settings.bundle_name())?;
    let disk_image_name = format!("{}.dmg", settings.bundle_name());
    common::print_bundling(&disk_image_name)?;

    let base_directory = prepare_base_directory(settings)?;
    let final_disk_image_path = base_directory.join(&disk_image_name);

    // Work in a fresh temporary tree. In particular, building/signing the app
    // must not remove a previously completed DMG (or app bundle) if a later
    // step fails.
    let staging = tempfile::tempdir()
        .context("Failed to create temporary directory for staging disk image")?;
    let app_directory = staging.path().join("application");
    fs::create_dir(&app_directory)
        .context("Failed to create temporary directory for application bundle")?;
    let application_bundle_path = osx_bundle::bundle_project_at(settings, &app_directory)
        .context("Failed to create application bundle for disk image")?;

    let volume_tree = staging.path().join("volume");
    fs::create_dir(&volume_tree).context("Failed to create staging HFS+ volume tree")?;
    let application_name = application_bundle_path
        .file_name()
        .context("Application bundle path is missing a file name")?;
    fs::rename(&application_bundle_path, volume_tree.join(application_name))
        .context("Failed to move application into disk image staging tree")?;
    create_applications_link(&volume_tree.join("Applications"))?;

    let finder_decoration =
        super::dmg_finder::prepare_volume(settings, &volume_tree, application_name)?;

    // Build in the output directory so the final rename stays on one volume.
    // NamedTempFile cleans up the candidate image on every early-return path.
    let candidate = tempfile::NamedTempFile::new_in(&base_directory)
        .context("Failed to create temporary disk image output")?
        .into_temp_path();
    super::hfs_image::write_image(
        &volume_tree,
        candidate.as_ref(),
        settings.bundle_name(),
        |volume| {
            finder_decoration
                .as_ref()
                .map(|decoration| {
                    super::dmg_finder::write_ds_store(application_name, decoration, volume)
                })
                .transpose()
        },
    )
    .context("Failed to build compressed HFS+ disk image")?;

    // Signing is done on the candidate, then persist atomically replaces an
    // existing final image only after the candidate is complete.
    signing::sign_apple_path(settings, candidate.as_ref())?;
    candidate
        .persist(&final_disk_image_path)
        .map_err(|error| error.error)
        .with_context(|| format!("Failed to persist disk image at {final_disk_image_path:?}"))?;

    Ok(vec![final_disk_image_path])
}

/// Sets up the base directory where the bundle will be staged.
fn prepare_base_directory(settings: &Settings) -> crate::Result<PathBuf> {
    let base_directory = settings.project_out_directory().join("bundle/dmg");
    fs::create_dir_all(&base_directory)
        .with_context(|| format!("Failed to create bundle directory {base_directory:?}"))?;
    Ok(base_directory)
}

#[cfg(unix)]
fn create_applications_link(path: &Path) -> crate::Result<()> {
    std::os::unix::fs::symlink("/Applications", path)
        .context("Failed to create /Applications symbolic link")?;
    Ok(())
}

#[cfg(windows)]
fn create_applications_link(path: &Path) -> crate::Result<()> {
    std::os::windows::fs::symlink_dir("/Applications", path)
        .context("Failed to create /Applications symbolic link")?;
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn create_applications_link(_path: &Path) -> crate::Result<()> {
    anyhow::bail!("Creating a macOS DMG requires symbolic-link support")
}
