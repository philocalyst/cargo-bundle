//! IPA creation using a ZIP archive of the completed, optionally signed app.

use super::{Settings, common, ios_bundle};
use anyhow::{Context, ensure};
use std::fs::{self, File};
use std::io::{Seek, Write};
use std::path::{Component, Path, PathBuf};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

pub fn bundle_project(settings: &Settings) -> crate::Result<Vec<PathBuf>> {
    ensure!(
        settings.target_os() == "ios"
            && settings.target_triples().count() <= 1
            && !settings.binary_arch().starts_with("x86")
            && settings.binary_arch() != "i386"
            && !settings
                .target_triples()
                .any(|target| target.ends_with("-sim") || target.ends_with("-macabi")),
        "IPA packaging requires an iOS device target; use --target aarch64-apple-ios"
    );
    let directory = settings.project_out_directory().join("bundle/ipa");
    fs::create_dir_all(&directory).context("Failed to create IPA output directory")?;
    let staging = tempfile::tempdir_in(&directory).context("Failed to stage IPA payload")?;
    let app = ios_bundle::create_app(settings, &staging.path().join("Payload"))?;
    let destination = directory.join(format!("{}.ipa", settings.bundle_name()));
    common::print_bundling(&destination.file_name().unwrap().to_string_lossy())?;
    let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
    write_archive(temporary.as_file_mut(), &app, settings.binary_name())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))?;
    }
    temporary.as_file().sync_all()?;
    temporary
        .persist(&destination)
        .map_err(|error| error.error)
        .with_context(|| format!("Failed to save IPA {destination:?}"))?;
    Ok(vec![destination])
}

fn write_archive(writer: impl Write + Seek, app: &Path, binary_name: &str) -> crate::Result<()> {
    let parent = app.parent().context("IPA app has no parent directory")?;
    let mut archive = ZipWriter::new(writer);
    archive.add_directory("Payload/", options(0o755))?;
    // Never follow links: archive their targets as links, not outside files.
    for entry in walkdir::WalkDir::new(app)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry?;
        let path = entry.path();
        let relative = path.strip_prefix(parent)?;
        let name = archive_path(relative)?;
        let metadata = fs::symlink_metadata(path)?;
        if metadata.is_dir() {
            archive.add_directory(
                format!("Payload/{name}/"),
                options(permissions(&metadata, true)),
            )?;
        } else if metadata.is_file() {
            let mode = if path == app.join(binary_name) {
                0o755
            } else {
                permissions(&metadata, false)
            };
            archive.start_file(format!("Payload/{name}"), options(mode))?;
            std::io::copy(&mut File::open(path)?, &mut archive)?;
        } else if metadata.file_type().is_symlink() {
            let target = fs::read_link(path)?;
            validate_link(relative, &target)?;
            let target = target.to_str().context("IPA symlink target is not UTF-8")?;
            archive.add_symlink(format!("Payload/{name}"), target, options(0o777))?;
        } else {
            anyhow::bail!("IPA cannot contain special file {path:?}");
        }
    }
    archive.finish().context("Failed to finish IPA archive")?;
    Ok(())
}

fn options(mode: u32) -> SimpleFileOptions {
    SimpleFileOptions::default()
        .last_modified_time(zip::DateTime::default())
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(mode)
}

fn archive_path(path: &Path) -> crate::Result<String> {
    path.components()
        .map(|component| {
            let Component::Normal(name) = component else {
                anyhow::bail!("Unsafe IPA path {path:?}");
            };
            let name = name.to_str().context("IPA filenames must be UTF-8")?;
            ensure!(
                !name.contains('\\'),
                "IPA filenames cannot contain backslashes: {path:?}"
            );
            Ok(name)
        })
        .collect::<crate::Result<Vec<_>>>()
        .map(|parts| parts.join("/"))
}

fn validate_link(path: &Path, target: &Path) -> crate::Result<()> {
    let mut depth = path
        .parent()
        .map_or(0, |parent| parent.components().count());
    for component in target.components() {
        match component {
            Component::Normal(name) => {
                ensure!(
                    !name.to_string_lossy().contains('\\'),
                    "Unsafe IPA symlink {path:?}"
                );
                depth += 1;
            }
            Component::CurDir => {}
            Component::ParentDir if depth > 1 => depth -= 1,
            _ => anyhow::bail!("IPA symlink {path:?} points outside the app: {target:?}"),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn permissions(metadata: &fs::Metadata, _directory: bool) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn permissions(_metadata: &fs::Metadata, directory: bool) -> u32 {
    if directory { 0o755 } else { 0o644 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};

    #[test]
    fn ipa_contains_one_payload_app_with_executable_and_profile() {
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path().join("Post Card.app");
        fs::create_dir_all(app.join("fr.lproj")).unwrap();
        fs::write(app.join("postcard"), "executable bytes").unwrap();
        fs::write(app.join("embedded.mobileprovision"), "profile bytes").unwrap();
        fs::write(app.join("fr.lproj/InfoPlist.strings"), "translated name").unwrap();
        let mut bytes = Cursor::new(Vec::new());
        write_archive(&mut bytes, &app, "postcard").unwrap();
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes.into_inner())).unwrap();
        let mut executable = zip.by_name("Payload/Post Card.app/postcard").unwrap();
        assert_eq!(executable.unix_mode().unwrap() & 0o777, 0o755);
        let mut content = String::new();
        executable.read_to_string(&mut content).unwrap();
        assert_eq!(content, "executable bytes");
        drop(executable);
        assert!(
            zip.by_name("Payload/Post Card.app/embedded.mobileprovision")
                .is_ok()
        );
        assert!(
            zip.by_name("Payload/Post Card.app/fr.lproj/InfoPlist.strings")
                .is_ok()
        );
        assert!(zip.file_names().all(|name| name.starts_with("Payload/")));
    }

    #[cfg(unix)]
    #[test]
    fn ipa_preserves_internal_symlinks_and_rejects_escape_links() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path().join("Example.app");
        fs::create_dir_all(app.join("data")).unwrap();
        fs::write(app.join("file"), "contents").unwrap();
        symlink("../file", app.join("data/link")).unwrap();
        let mut bytes = Cursor::new(Vec::new());
        write_archive(&mut bytes, &app, "file").unwrap();
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes.into_inner())).unwrap();
        let mut link = zip.by_name("Payload/Example.app/data/link").unwrap();
        assert_eq!(link.unix_mode().unwrap() & 0o170000, 0o120000);
        let mut target = String::new();
        link.read_to_string(&mut target).unwrap();
        assert_eq!(target, "../file");
        symlink("../../outside", app.join("data/escape")).unwrap();
        assert!(write_archive(Cursor::new(Vec::new()), &app, "file").is_err());
    }
}
