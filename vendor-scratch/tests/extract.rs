mod common;

use std::fs;
use std::io::Cursor;
#[cfg(unix)]
use std::path::Path;

use appimage::{AppDirError, AppImage, Error, ExistingFiles};
use backhand::compression::Compressor;
use backhand::{FilesystemCompressor, FilesystemWriter, NodeHeader};
use common::Fixture;

fn fixture(build: impl FnOnce(&mut FilesystemWriter<'_, '_, '_>)) -> Fixture {
    let mut writer = FilesystemWriter::default();
    writer.set_compressor(FilesystemCompressor::new(Compressor::Gzip, None).unwrap());
    build(&mut writer);
    let mut bytes = Cursor::new(Vec::new());
    writer.write(&mut bytes).unwrap();
    Fixture::wrap(&bytes.into_inner())
}

fn header(mode: u16) -> NodeHeader {
    NodeHeader::new(mode, 12345, 12345, common::MTIME)
}

#[cfg(unix)]
#[test]
fn destination_parent_links_cannot_redirect_writes_outside() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let image = AppImage::open(&fixture.path).unwrap();
    let destination = fixture.dir.path().join("out");
    let outside = fixture.dir.path().join("outside");
    fs::create_dir(&destination).unwrap();
    fs::create_dir(&outside).unwrap();
    symlink(&outside, destination.join("dir")).unwrap();
    assert!(
        image
            .unpack()
            .existing(ExistingFiles::Replace)
            .to(&destination)
            .is_err()
    );
    assert!(!outside.join("nested").exists());
}

#[cfg(unix)]
#[test]
fn replacement_replaces_links_without_modifying_their_targets() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    for hardlink in [false, true] {
        let fixture = Fixture::new();
        let image = AppImage::open(&fixture.path).unwrap();
        let destination = fixture.dir.path().join("out");
        let outside = fixture.dir.path().join("outside");
        fs::create_dir(&destination).unwrap();
        fs::write(&outside, b"keep me").unwrap();
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o600)).unwrap();
        if hardlink {
            fs::hard_link(&outside, destination.join("data")).unwrap();
        } else {
            symlink(&outside, destination.join("data")).unwrap();
        }
        image
            .unpack()
            .existing(ExistingFiles::Replace)
            .to(&destination)
            .unwrap();
        assert_eq!(fs::read(&outside).unwrap(), b"keep me");
        let old = fs::metadata(&outside).unwrap();
        let new = fs::symlink_metadata(destination.join("data")).unwrap();
        assert_eq!(old.permissions().mode() & 0o777, 0o600);
        assert!(new.is_file());
        assert_ne!(new.ino(), old.ino());
        assert_eq!(
            fs::read(destination.join("data")).unwrap(),
            common::DATA_CONTENTS.as_bytes()
        );
        // Existing archive symlinks also support explicit replacement.
        image
            .unpack()
            .existing(ExistingFiles::Replace)
            .to(&destination)
            .unwrap();
        assert_eq!(
            fs::read_link(destination.join("symlink")).unwrap(),
            Path::new("data")
        );
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 4);
    }
}

#[test]
fn escaping_and_cyclic_archive_links_are_rejected_before_writing() {
    for (target, cycle) in [
        ("/outside", false),
        ("../outside", false),
        ("b/../outside", false),
        ("b", true),
    ] {
        let fixture = fixture(|writer| {
            writer.push_symlink(target, "a", header(0o777)).unwrap();
            writer
                .push_symlink(if cycle { "a" } else { "." }, "b", header(0o777))
                .unwrap();
        });
        let image = AppImage::open(&fixture.path).unwrap();
        let destination = fixture.dir.path().join("out");
        let error = image.unpack().to(&destination).unwrap_err();
        assert!(
            matches!(
                error,
                Error::AppDir(AppDirError::EscapingSymlink(_) | AppDirError::SymlinkLoop(_))
            ),
            "{target}: {error:?}"
        );
        assert!(!destination.exists());
    }
}

#[cfg(unix)]
#[test]
fn internal_link_chains_and_parent_components_are_preserved() {
    let fixture = fixture(|writer| {
        writer
            .push_file(Cursor::new(b"contents"), "data", header(0o644))
            .unwrap();
        writer.push_dir("dir", header(0o755)).unwrap();
        writer
            .push_symlink("../data", "dir/one", header(0o777))
            .unwrap();
        writer
            .push_symlink("dir/one", "two", header(0o777))
            .unwrap();
        writer
            .push_symlink("missing", "dangling", header(0o777))
            .unwrap();
    });
    let image = AppImage::open(&fixture.path).unwrap();
    let scratch = image.unpack().to_temp().unwrap();
    assert_eq!(fs::read(scratch.join("two")).unwrap(), b"contents");
    assert_eq!(
        fs::read_link(scratch.join("dir/one")).unwrap(),
        Path::new("../data")
    );
    assert_eq!(
        fs::read_link(scratch.join("dangling")).unwrap(),
        Path::new("missing")
    );
}

#[cfg(unix)]
#[test]
fn directory_metadata_is_restored_after_children_without_privileged_file_bits() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::time::{Duration, UNIX_EPOCH};
    let fixture = fixture(|writer| {
        writer.push_dir("dir", header(0o500)).unwrap();
        writer
            .push_file(Cursor::new(b"contents"), "dir/file", header(0o6755))
            .unwrap();
    });
    let image = AppImage::open(&fixture.path).unwrap();
    let destination = fixture.dir.path().join("out");
    image.unpack().to(&destination).unwrap();
    let directory = fs::metadata(destination.join("dir")).unwrap();
    assert_eq!(directory.permissions().mode() & 0o7777, 0o500);
    assert_eq!(
        directory.modified().unwrap(),
        UNIX_EPOCH + Duration::from_secs(u64::from(common::MTIME))
    );
    let file = fs::metadata(destination.join("dir/file")).unwrap();
    assert_eq!(file.permissions().mode() & 0o7777, 0o755);
    assert_eq!(file.uid(), fs::metadata(&destination).unwrap().uid());
    fs::set_permissions(destination.join("dir"), fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn corrupt_file_data_leaves_existing_output_and_no_staging_files() {
    let fixture = fixture(|writer| {
        writer
            .push_file(Cursor::new(vec![b'x'; 200_000]), "data", header(0o644))
            .unwrap();
    });
    let image = AppImage::open(&fixture.path).unwrap();
    let mut bytes = fs::read(&fixture.path).unwrap();
    // SquashFS file data follows its 96-byte superblock; leave metadata intact.
    bytes[fixture.offset as usize + 96..fixture.offset as usize + 100].fill(0xff);
    fs::write(&fixture.path, bytes).unwrap();
    let destination = fixture.dir.path().join("out");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("data"), b"keep me").unwrap();
    assert!(
        image
            .unpack()
            .existing(ExistingFiles::Replace)
            .to(&destination)
            .is_err()
    );
    assert_eq!(fs::read(destination.join("data")).unwrap(), b"keep me");
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
    fs::remove_file(destination.join("data")).unwrap();
    assert!(image.unpack().to(&destination).is_err());
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn links_that_escape_through_existing_links_are_rolled_back() {
    use std::os::unix::fs::symlink;
    let fixture = fixture(|writer| {
        writer
            .push_symlink("b/../outside", "a", header(0o777))
            .unwrap();
    });
    let image = AppImage::open(&fixture.path).unwrap();
    for existing in [ExistingFiles::Error, ExistingFiles::Replace] {
        let destination = fixture.dir.path().join(format!("{existing:?}"));
        fs::create_dir(&destination).unwrap();
        symlink(".", destination.join("b")).unwrap();
        assert!(image.unpack().existing(existing).to(&destination).is_err());
        assert!(fs::symlink_metadata(destination.join("a")).is_err());
    }
}

#[cfg(unix)]
#[test]
fn scratch_cleanup_handles_restrictive_directories_without_following_links() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let fixture = fixture(|writer| {
        writer.push_dir("dir", header(0o500)).unwrap();
        writer
            .push_file(Cursor::new(b"contents"), "dir/file", header(0o400))
            .unwrap();
    });
    let image = AppImage::open(&fixture.path).unwrap();
    let outside = fixture.dir.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::set_permissions(&outside, fs::Permissions::from_mode(0o500)).unwrap();
    let scratch = image.unpack().to_temp().unwrap();
    let path = scratch.to_path_buf();
    symlink(&outside, path.join("outside")).unwrap();
    fs::set_permissions(path.join("dir"), fs::Permissions::from_mode(0o000)).unwrap();
    drop(scratch);
    assert!(!path.exists());
    assert_eq!(
        fs::metadata(&outside).unwrap().permissions().mode() & 0o777,
        0o500
    );
    fs::set_permissions(&outside, fs::Permissions::from_mode(0o700)).unwrap();

    let persistent = image.unpack().to_temp().unwrap().persist();
    assert_eq!(
        fs::metadata(persistent.join("dir"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o500
    );
    let path = std::path::PathBuf::from(persistent);
    fs::set_permissions(path.join("dir"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_dir_all(path).unwrap();
}
