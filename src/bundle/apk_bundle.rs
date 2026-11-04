//! Android packages are compiled, aligned and optionally signed by apk-rs.
use super::common;
use crate::Settings;
use anyhow::Context;
use apk::{
    Abi,
    build::ApkBuilder,
    manifest::Manifest,
    res::Density,
    sign::{Signer, SigningIdentity},
};
use std::{fs, path::PathBuf};

pub fn bundle_project(settings: &Settings) -> crate::Result<Vec<PathBuf>> {
    common::validate_bundle_name(settings.bundle_name())?;
    let abi = match settings.binary_arch() {
        "aarch64" => Abi::Arm64V8a,
        "arm" => Abi::ArmeabiV7a,
        "x86" => Abi::X86,
        "x86_64" => Abi::X86_64,
        other => anyhow::bail!("Unsupported Android architecture: {other}"),
    };
    let library = settings
        .binary_name()
        .strip_prefix("lib")
        .and_then(|name| name.strip_suffix(".so"))
        .context("APK input must be a NativeActivity shared library named lib<name>.so")?;
    let android = settings.android();
    let project = settings
        .manifest_path()
        .parent()
        .context("Missing manifest directory")?;
    let mut builder = if let Some(manifest) = android.manifest {
        ApkBuilder::new(Manifest::parse(&fs::read_to_string(
            project.join(manifest),
        )?)?)
    } else {
        ApkBuilder::native_activity(settings.bundle_identifier().parse()?, library.parse()?)
    };
    builder.label(settings.bundle_name()).version(
        android.version_code.unwrap_or(1).try_into()?,
        &settings.version_string().to_string(),
    );
    if let Some(sdk) = android.min_sdk {
        builder.min_sdk(sdk.try_into()?);
    }
    if let Some(sdk) = android.target_sdk {
        builder.target_sdk(sdk.try_into()?);
    }
    for permission in android.permissions.unwrap_or_default() {
        builder.permission(permission.parse()?);
    }
    builder.native_library_bytes(abi, library.parse()?, settings.binary_path())?;
    for resource in settings.resource_files() {
        let resource = resource?;
        let destination = common::resource_relpath(&resource);
        builder.asset(
            destination
                .to_str()
                .context("Non-UTF-8 Android asset path")?,
            project.join(&resource),
        )?;
    }
    // Let the image crate normalize supported input formats to PNG.
    if let Some(icon) = settings.icon_files().next() {
        let image = image::open(project.join(icon?))?;
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png)?;
        builder.icon(Density::XXXHDPI, png.into_inner())?;
    }
    if let Some(key) = android.signing_key {
        builder.sign_with(Signer::new(SigningIdentity::from_pem_file(
            project.join(key),
        )?));
    }
    let directory = settings.project_out_directory().join("bundle/apk");
    fs::create_dir_all(&directory)?;
    let output = directory.join(format!("{}.apk", settings.bundle_name()));
    common::print_bundling(output.file_name().unwrap().to_string_lossy().as_ref())?;
    builder
        .write(&output)
        .context("Failed to assemble Android APK")?;
    Ok(vec![output])
}
