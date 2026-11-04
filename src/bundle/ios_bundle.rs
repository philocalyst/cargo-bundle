// An iOS package is laid out like:
//
// Foobar.app         # Actually a directory
//     Foobar             # The main binary executable of the app
//     Info.plist         # An XML file containing the app's metadata
//     ...                # Icons and other resource files
//
// See https://developer.apple.com/go/?id=bundle-structure for a full
// explanation.

use super::common;
use super::signing;
use crate::Settings;
use anyhow::Context;
use image::{self, GenericImageView};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

pub fn bundle_project(settings: &Settings) -> crate::Result<Vec<PathBuf>> {
    let directory = settings.project_out_directory().join("bundle/ios");
    fs::create_dir_all(&directory)?;
    let staging = tempfile::tempdir_in(&directory).context("Failed to stage iOS bundle")?;
    let app = create_app(settings, staging.path())?;
    let destination = directory.join(app.file_name().context("iOS app has no filename")?);
    publish_app(&app, &destination, staging)?;
    Ok(vec![destination])
}

fn publish_app(app: &Path, destination: &Path, staging: tempfile::TempDir) -> crate::Result<()> {
    let backup = staging.path().join("previous-output");
    let previous = match fs::rename(destination, &backup) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error).context("Failed to preserve previous iOS bundle"),
    };
    if let Err(error) = fs::rename(app, destination) {
        if previous && let Err(restore) = fs::rename(&backup, destination) {
            let recovery = staging.keep();
            anyhow::bail!(
                "Failed to publish iOS bundle: {error}; failed to restore it: {restore}. Previous output remains in {}",
                recovery.join("previous-output").display()
            );
        }
        return Err(error).context("Failed to publish iOS bundle");
    }
    Ok(())
}

/// Build and sign an app in the given parent directory before any archiving.
pub(super) fn create_app(settings: &Settings, directory: &Path) -> crate::Result<PathBuf> {
    common::print_warning("iOS bundle support is still experimental.")?;
    anyhow::ensure!(
        settings.target_os() == "ios",
        "iOS bundling requires an iOS target"
    );
    common::validate_bundle_name(settings.bundle_name())?;
    validate_bundle_identifier(&settings.bundle_identifier())?;
    let app_bundle_name = format!("{}.app", settings.bundle_name());
    common::print_bundling(&app_bundle_name)?;
    let bundle_dir = directory.join(&app_bundle_name);
    if bundle_dir.exists() {
        fs::remove_dir_all(&bundle_dir)
            .with_context(|| format!("Failed to remove old {app_bundle_name}"))?;
    }
    fs::create_dir_all(&bundle_dir)
        .with_context(|| format!("Failed to create bundle directory at {bundle_dir:?}"))?;

    for src in settings.resource_files() {
        let src = src?;
        let dest = bundle_dir.join(common::resource_relpath(&src));
        common::copy_file(&src, &dest)
            .with_context(|| format!("Failed to copy resource file {src:?}"))?;
    }

    let icon_filenames =
        generate_icon_files(&bundle_dir, settings).with_context(|| "Failed to create app icons")?;
    generate_info_plist(&bundle_dir, settings, &icon_filenames)
        .with_context(|| "Failed to create Info.plist")?;
    if let Some(localizations) = settings.ios_localizations() {
        localizations
            .write_to_directory(&bundle_dir)
            .with_context(|| "Failed to write iOS localizations")?;
    }
    let bin_path = bundle_dir.join(settings.binary_name());
    common::copy_file(settings.binary_path(), &bin_path)
        .with_context(|| format!("Failed to copy binary from {:?}", settings.binary_path()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bin_path, fs::Permissions::from_mode(0o755))?;
    }
    signing::sign_apple_bundle(settings, &bundle_dir)?;
    Ok(bundle_dir)
}

fn validate_bundle_identifier(identifier: &str) -> crate::Result<()> {
    anyhow::ensure!(
        !identifier.is_empty()
            && identifier.split('.').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            }),
        "iOS requires a bundle identifier containing only letters, digits, hyphens, and nonempty dot-separated components; set package.metadata.bundle.identifier"
    );
    Ok(())
}

/// Generate the icon files and store them under the `bundle_dir`.
fn generate_icon_files(bundle_dir: &Path, settings: &Settings) -> crate::Result<Vec<String>> {
    let mut filenames = Vec::new();
    {
        let mut get_dest_path = |width: u32, height: u32, is_retina: bool| {
            let filename = format!(
                "icon_{}x{}{}.png",
                width,
                height,
                if is_retina { "@2x" } else { "" }
            );
            let path = bundle_dir.join(&filename);
            filenames.push(filename);
            path
        };
        let mut sizes = BTreeSet::new();
        // As on macOS, the first configured SVG is the authoritative source
        // and is rasterized at every standard iOS icon size. Later SVGs are
        // ignored, giving duplicate inputs a stable, documented precedence.
        if let Some(svg_path) = settings
            .icon_files()
            .find_map(|path| match path {
                Ok(path) if path.extension() == Some(OsStr::new("svg")) => Some(Ok(path)),
                Err(error) => Some(Err(error)),
                _ => None,
            })
            .transpose()?
        {
            filenames = generate_svg_icon_files(bundle_dir, &svg_path)?;
            // Mirror macOS precedence: once an SVG is selected, its complete
            // icon family is used and raster/ICNS inputs do not add variants.
            return Ok(filenames);
        }
        // Prefer PNG files.
        for icon_path in settings.icon_files() {
            let icon_path = icon_path?;
            if icon_path.extension() != Some(OsStr::new("png")) {
                continue;
            }
            let img = image::ImageReader::open(&icon_path)?
                .with_guessed_format()?
                .decode()?;
            let (width, height) = img.dimensions();
            let is_retina = common::is_retina(&icon_path);
            if !sizes.contains(&(width, height, is_retina)) {
                sizes.insert((width, height, is_retina));
                let dest_path = get_dest_path(width, height, is_retina);
                common::copy_file(&icon_path, &dest_path)?;
            }
        }
        // Fall back to non-PNG files for any missing sizes.
        for icon_path in settings.icon_files() {
            let icon_path = icon_path?;
            if icon_path.extension() == Some(OsStr::new("png")) {
                continue;
            } else if icon_path.extension() == Some(OsStr::new("icns")) {
                let icon_family = icns::IconFamily::read(File::open(&icon_path)?)?;
                for icon_type in icon_family.available_icons() {
                    let width = icon_type.screen_width();
                    let height = icon_type.screen_height();
                    let is_retina = icon_type.pixel_density() > 1;
                    if !sizes.contains(&(width, height, is_retina)) {
                        sizes.insert((width, height, is_retina));
                        let dest_path = get_dest_path(width, height, is_retina);
                        let icon = icon_family.get_icon_with_type(icon_type)?;
                        icon.write_png(File::create(dest_path)?)?;
                    }
                }
            } else if icon_path.extension() == Some(OsStr::new("svg")) {
                // SVGs are handled above, where the first one takes precedence.
                continue;
            } else {
                let icon = image::open(&icon_path)?;
                let (width, height) = icon.dimensions();
                let is_retina = common::is_retina(&icon_path);
                if !sizes.contains(&(width, height, is_retina)) {
                    sizes.insert((width, height, is_retina));
                    let dest_path = get_dest_path(width, height, is_retina);
                    let mut file = common::create_file(&dest_path)?;
                    icon.write_to(&mut file, image::ImageFormat::Png)?;
                }
            }
        }
    }
    Ok(filenames)
}

// (logical points, scale factor), covering the standard iPhone/iPad icon
// slots and the 1024px App Store icon. Each raster is written at points*scale.
const IOS_ICON_SIZES: &[(f32, u32)] = &[
    (20.0, 1),
    (20.0, 2),
    (20.0, 3),
    (29.0, 1),
    (29.0, 2),
    (29.0, 3),
    (40.0, 1),
    (40.0, 2),
    (40.0, 3),
    (60.0, 2),
    (60.0, 3),
    (76.0, 1),
    (76.0, 2),
    (83.5, 2),
    (1024.0, 1),
];

fn generate_svg_icon_files(bundle_dir: &Path, svg_path: &Path) -> crate::Result<Vec<String>> {
    let mut filenames = Vec::with_capacity(IOS_ICON_SIZES.len());
    for &(logical_size, scale) in IOS_ICON_SIZES {
        let pixel_size = (logical_size * scale as f32) as u32;
        let filename = format!(
            "icon_{logical_size}x{logical_size}{}.png",
            if scale > 1 {
                format!("@{scale}x")
            } else {
                String::new()
            }
        );
        render_svg_icon(
            svg_path,
            pixel_size,
            pixel_size,
            &bundle_dir.join(&filename),
        )?;
        filenames.push(filename);
    }
    Ok(filenames)
}

fn render_svg_icon(
    svg_path: &Path,
    width: u32,
    height: u32,
    dest_path: &Path,
) -> crate::Result<()> {
    let svg_data =
        fs::read(svg_path).with_context(|| format!("Failed to read SVG icon {svg_path:?}"))?;
    let tree = resvg::usvg::Tree::from_data(&svg_data, &resvg::usvg::Options::default())
        .with_context(|| format!("Failed to parse SVG icon {svg_path:?}"))?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .with_context(|| format!("Failed to create {width}x{height} icon pixmap"))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(
            width as f32 / tree.size().width(),
            height as f32 / tree.size().height(),
        ),
        &mut pixmap.as_mut(),
    );
    let png = pixmap
        .encode_png()
        .with_context(|| "Failed to encode rendered SVG icon")?;
    fs::write(dest_path, png)
        .with_context(|| format!("Failed to write rendered PNG icon to {dest_path:?}"))?;
    Ok(())
}

fn generate_info_plist(
    bundle_dir: &Path,
    settings: &Settings,
    icon_filenames: &[String],
) -> crate::Result<()> {
    let mut values = plist::Dictionary::new();
    for (key, value) in [
        (
            "CFBundleIdentifier",
            settings.bundle_identifier().into_owned(),
        ),
        ("CFBundleDisplayName", settings.bundle_name().to_owned()),
        ("CFBundleName", settings.bundle_name().to_owned()),
        ("CFBundleExecutable", settings.binary_name().to_owned()),
        ("CFBundleVersion", settings.version_string().to_string()),
        (
            "CFBundleShortVersionString",
            settings.version_string().to_string(),
        ),
        ("CFBundleDevelopmentRegion", "en_US".to_owned()),
        ("CFBundlePackageType", "APPL".to_owned()),
    ] {
        values.insert(key.into(), plist::Value::String(value));
    }
    values.insert("LSRequiresIPhoneOS".into(), plist::Value::Boolean(true));
    if !icon_filenames.is_empty() {
        values.insert(
            "CFBundleIconFiles".into(),
            plist::Value::Array(
                icon_filenames
                    .iter()
                    .cloned()
                    .map(plist::Value::String)
                    .collect(),
            ),
        );
    }
    plist::Value::Dictionary(values).to_writer_xml(File::create(bundle_dir.join("Info.plist"))?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_publication_restores_previous_bundle() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("Example.app");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("keep"), b"previous bundle").unwrap();
        let staging = tempfile::tempdir_in(root.path()).unwrap();
        let missing = staging.path().join("Example.app");
        assert!(publish_app(&missing, &destination, staging).is_err());
        assert_eq!(
            fs::read(destination.join("keep")).unwrap(),
            b"previous bundle"
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn publication_replaces_previous_app_without_backup_name_collision() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("previous.app");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("old"), b"old").unwrap();
        let staging = tempfile::tempdir_in(root.path()).unwrap();
        let app = staging.path().join("previous.app");
        fs::create_dir(&app).unwrap();
        fs::write(app.join("new"), b"new").unwrap();
        publish_app(&app, &destination, staging).unwrap();
        assert_eq!(fs::read(destination.join("new")).unwrap(), b"new");
        assert!(!destination.join("old").exists());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn renders_svg_as_png_at_ios_sizes_and_lists_them_for_plist() {
        let temp = tempfile::tempdir().unwrap();
        let svg_path = temp.path().join("icon.svg");
        fs::write(
            &svg_path,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="red"/></svg>"#,
        )
        .unwrap();

        let filenames = generate_svg_icon_files(temp.path(), &svg_path).unwrap();
        for (filename, &(logical_size, scale)) in filenames.iter().zip(IOS_ICON_SIZES) {
            let pixel_size = (logical_size * scale as f32) as u32;
            let destination = temp.path().join(filename);
            let decoded = image::open(&destination).unwrap();
            assert_eq!(decoded.dimensions(), (pixel_size, pixel_size));
        }
    }
}
