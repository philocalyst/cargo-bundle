//! End-to-end tests against a self-contained synthetic AppImage.
//!
//! The fixture (see `common`) is a real SquashFS appended to a hand-crafted
//! ELF, so these tests run everywhere and actually exercise ELF parsing,
//! section reads, and SquashFS extraction — no external image required.

mod common;

use std::fs;

use appimage::{
    APP_RUN, AppDir, AppDirError, AppImage, BlockSize, Compression, Config, Error, ExistingFiles,
    Home, Malformed, Operation, Packer, Runtime, Sidecar, UpdateInformation, section,
};
use common::{APPRUN_CONTENTS, DATA_CONTENTS, Fixture, NESTED_CONTENTS};

#[test]
fn open_locates_the_squashfs_offset() {
    let fixture = Fixture::new();
    let image = AppImage::open(&fixture.path).expect("open synthetic appimage");

    let image_size = fs::metadata(&fixture.path).expect("stat").len();
    assert_eq!(image.offset, fixture.offset);
    assert!(image.offset > 0 && image.offset < image_size);
    assert_eq!(image.path, fixture.path);
}

#[test]
fn open_rejects_non_elf_input_everywhere() {
    let error = AppImage::open("/dev/null").expect_err("/dev/null is not an ELF");
    assert!(
        matches!(
            error,
            Error::NotAppImage {
                reason: Malformed::NotElf | Malformed::Truncated,
                ..
            }
        ),
        "unexpected error: {error:?}"
    );
}

#[test]
fn open_rejects_an_elf_with_no_payload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bare");
    fs::write(&path, &*Runtime::inert()).unwrap();
    assert!(matches!(
        AppImage::open(&path),
        Err(Error::NotAppImage {
            reason: Malformed::MissingPayload,
            ..
        })
    ));
}

#[test]
fn reads_named_elf_sections() {
    let fixture = Fixture::new();
    let image = AppImage::open(&fixture.path).expect("open");

    let upd = image
        .section(section::UPDATE_INFO)
        .expect("read .upd_info")
        .expect("has .upd_info");
    assert_eq!(upd, fixture.upd_info.as_bytes());

    // Present-but-empty and absent sections both read as None.
    assert_eq!(
        image.section(section::SIGNATURE).expect("read .sha256_sig"),
        None
    );
    assert_eq!(image.section(".missing").expect("read missing"), None);
}

#[test]
fn unpacks_the_full_tree_with_contents_and_symlink() {
    let fixture = Fixture::new();
    let image = AppImage::open(&fixture.path).expect("open");

    let out = fixture.dir.path().join("full");
    let unpacked = image.unpack().to(&out).expect("unpack");
    // The directory is held open by its resolved path.
    assert_eq!(&*unpacked, fs::canonicalize(&out).unwrap());

    assert_eq!(
        fs::read_to_string(unpacked.join("AppRun")).unwrap(),
        APPRUN_CONTENTS
    );
    assert!(unpacked.join(APP_RUN).is_file());
    assert_eq!(
        fs::read_to_string(unpacked.join("data")).unwrap(),
        DATA_CONTENTS
    );
    assert_eq!(
        fs::read_to_string(unpacked.join("dir/nested")).unwrap(),
        NESTED_CONTENTS
    );

    let link = fs::read_link(unpacked.join("symlink")).expect("symlink is a symlink");
    assert_eq!(link, std::path::Path::new("data"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(unpacked.join("AppRun"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755, "executable bit preserved");
        assert_eq!(mode & 0o6000, 0, "setuid/setgid bits are stripped");
    }
}

/// Assert that a fixture's full tree extracts with the expected contents. Shared
/// by the xz and zstd cases so every compressor path is checked the same way.
#[cfg(any(feature = "xz", feature = "zstd"))]
fn assert_tree_extracts(fixture: &Fixture) {
    let image = AppImage::open(&fixture.path).expect("open");
    let out = fixture.dir.path().join("tree");
    let unpacked = image.unpack().to(&out).expect("unpack");

    assert_eq!(
        fs::read_to_string(unpacked.join("AppRun")).unwrap(),
        APPRUN_CONTENTS
    );
    assert_eq!(
        fs::read_to_string(unpacked.join("data")).unwrap(),
        DATA_CONTENTS
    );
    assert_eq!(
        fs::read_to_string(unpacked.join("dir/nested")).unwrap(),
        NESTED_CONTENTS
    );
    assert_eq!(
        fs::read_link(unpacked.join("symlink")).unwrap(),
        std::path::Path::new("data")
    );
}

// The payloads below are real SquashFS images compressed with xz / zstd; the
// crate must decode them with its pure-Rust decoders (no C backend enabled).

#[cfg(feature = "zstd")]
#[test]
fn unpacks_zstd_compressed_payload_pure_rust() {
    let fixture = Fixture::wrap(include_bytes!("fixtures/tree-zstd.sqfs"));
    assert_tree_extracts(&fixture);
}

#[cfg(feature = "xz")]
#[test]
fn unpacks_xz_compressed_payload_pure_rust() {
    let fixture = Fixture::wrap(include_bytes!("fixtures/tree-xz.sqfs"));
    assert_tree_extracts(&fixture);
}

#[test]
fn matching_pattern_extracts_only_selected_paths() {
    let fixture = Fixture::new();
    let image = AppImage::open(&fixture.path).expect("open");

    let out = fixture.dir.path().join("pattern");
    image
        .unpack()
        .matching("data")
        .to(&out)
        .expect("unpack matching");

    assert!(out.join("data").is_file());
    assert!(!out.join("AppRun").exists());
    assert!(!out.join("symlink").exists());
}

#[cfg(unix)]
#[test]
fn extraction_cannot_be_redirected_through_links_already_in_the_destination() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let image = AppImage::open(&fixture.path).expect("open");
    let outside = fixture.dir.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("nested"), "keep me\n").unwrap();
    let out = fixture.dir.path().join("unpack-through-link");
    fs::create_dir(&out).unwrap();
    symlink(&outside, out.join("dir")).unwrap();

    for existing in [ExistingFiles::Error, ExistingFiles::Replace] {
        assert!(image.unpack().existing(existing).to(&out).is_err());
        assert_eq!(
            fs::read_to_string(outside.join("nested")).unwrap(),
            "keep me\n"
        );
    }

    let leaf_out = fixture.dir.path().join("unpack-leaf-link");
    fs::create_dir(&leaf_out).unwrap();
    fs::write(outside.join("data-target"), DATA_CONTENTS).unwrap();
    symlink(outside.join("data-target"), leaf_out.join("data")).unwrap();
    // A link in the way is refused, or replaced by the file, never written through.
    let _ = image.unpack().to(&leaf_out);
    let _ = image
        .unpack()
        .existing(ExistingFiles::Replace)
        .to(&leaf_out);
    assert_eq!(
        fs::read_to_string(outside.join("data-target")).unwrap(),
        DATA_CONTENTS
    );
}

#[cfg(unix)]
#[test]
fn a_destination_that_is_a_link_is_followed_once_and_no_further() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let image = AppImage::open(&fixture.path).expect("open");
    let real = fixture.dir.path().join("real");
    fs::create_dir(&real).unwrap();
    let out = fixture.dir.path().join("root-link");
    symlink(&real, &out).unwrap();

    let unpacked = image
        .unpack()
        .to(&out)
        .expect("unpack through the root link");
    assert_eq!(&*unpacked, fs::canonicalize(&real).unwrap());
    assert_eq!(
        fs::read_to_string(real.join("data")).unwrap(),
        DATA_CONTENTS
    );
}

#[cfg(unix)]
#[test]
fn replacing_a_hard_linked_file_leaves_its_other_names_alone() {
    let fixture = Fixture::new();
    let image = AppImage::open(&fixture.path).expect("open");
    let outside = fixture.dir.path().join("outside-hardlink");
    fs::write(&outside, "outside data that must survive").unwrap();
    let out = fixture.dir.path().join("hardlink-destination");
    fs::create_dir(&out).unwrap();
    fs::hard_link(&outside, out.join("data")).unwrap();

    image
        .unpack()
        .existing(ExistingFiles::Replace)
        .to(&out)
        .expect("extract over hard link");
    assert_eq!(
        fs::read_to_string(&outside).unwrap(),
        "outside data that must survive"
    );
    assert_eq!(fs::read_to_string(out.join("data")).unwrap(), DATA_CONTENTS);
}

#[cfg(unix)]
#[test]
fn extraction_keeps_readonly_directories_writable_until_children_are_written() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    let src = fixture.dir.path().join("Readonly.AppDir");
    fs::create_dir_all(src.join("readonly")).unwrap();
    fs::write(src.join("readonly/child"), "child contents\n").unwrap();
    fs::set_permissions(src.join("readonly"), fs::Permissions::from_mode(0o555)).unwrap();
    let image = pack(&src, &fixture.dir.path().join("readonly.AppImage"));

    let out = fixture.dir.path().join("readonly-extracted");
    let unpacked = image
        .unpack()
        .to(&out)
        .expect("extract read-only directory");
    assert_eq!(
        fs::read_to_string(unpacked.join("readonly/child")).unwrap(),
        "child contents\n"
    );
    assert_eq!(
        fs::metadata(unpacked.join("readonly"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o555
    );
}

#[test]
fn existing_files_are_an_error_unless_they_are_to_be_replaced() {
    let fixture = Fixture::new();
    let image = AppImage::open(&fixture.path).expect("open");
    let out = fixture.dir.path().join("overwrite");
    image.unpack().to(&out).expect("first unpack");

    // Same length as the real contents, different bytes: size alone settles nothing.
    let stale = "x".repeat(DATA_CONTENTS.len());
    fs::write(out.join("data"), &stale).unwrap();
    let error = image.unpack().to(&out).unwrap_err();
    assert!(
        matches!(&error, Error::Io { operation: Operation::Create, source, .. }
            if source.kind() == std::io::ErrorKind::AlreadyExists),
        "{error}"
    );
    assert_eq!(fs::read_to_string(out.join("data")).unwrap(), stale);

    image
        .unpack()
        .existing(ExistingFiles::Replace)
        .to(&out)
        .expect("replacing unpack");
    assert_eq!(fs::read_to_string(out.join("data")).unwrap(), DATA_CONTENTS);
}

#[test]
fn a_bad_pattern_is_reported_before_anything_is_written() {
    let fixture = Fixture::new();
    let image = AppImage::open(&fixture.path).expect("open");
    let out = fixture.dir.path().join("never");
    let result = image.unpack().matching("[").to(&out);
    assert!(matches!(result, Err(Error::Pattern(_))));
    assert!(!out.exists());
}

#[test]
fn unpack_to_temp_yields_self_cleaning_scratch() {
    let fixture = Fixture::new();
    let image = AppImage::open(&fixture.path).expect("open");

    let scratch = image.unpack().to_temp().expect("unpack to scratch");
    let path = scratch.to_path_buf();
    assert!(path.join("data").is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path.join("dir"), fs::Permissions::from_mode(0o555)).unwrap();
    }
    drop(scratch);
    assert!(!path.exists(), "scratch removed on drop");
}

#[test]
fn scratch_can_be_persisted_without_losing_its_directory_handle() {
    let fixture = Fixture::new();
    let image = AppImage::open(&fixture.path).unwrap();
    let scratch = image.unpack().to_temp().unwrap();
    let path = scratch.to_path_buf();
    let mut app = scratch.persist();
    app.file(appimage::AppPath::new("data").unwrap()).unwrap();
    drop(app);
    assert!(path.join("data").is_file());
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn parses_typed_update_information() {
    let fixture = Fixture::new();
    let image = AppImage::open(&fixture.path).expect("open");
    let info = image
        .update_information()
        .expect("read")
        .expect("has update info");
    assert_eq!(
        info,
        UpdateInformation::Zsync {
            url: "https://example.test/App-latest.zsync".into()
        }
    );
    assert_eq!(info.transport(), "zsync");
}

#[test]
fn extraction_preserves_mtime() {
    use std::time::{Duration, UNIX_EPOCH};
    let fixture = Fixture::new();
    let image = AppImage::open(&fixture.path).expect("open");
    let out = fixture.dir.path().join("mtime");
    let unpacked = image.unpack().to(&out).expect("unpack");

    let modified = fs::metadata(unpacked.join("data"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        modified,
        UNIX_EPOCH + Duration::from_secs(u64::from(common::MTIME))
    );
}

#[test]
fn pack_then_open_and_unpack_roundtrips() {
    let fixture = Fixture::new(); // reused only for its temp dir
    let src = fixture.dir.path().join("AppDir");
    fs::create_dir_all(src.join("dir")).unwrap();
    fs::write(src.join("AppRun"), APPRUN_CONTENTS).unwrap();
    fs::write(src.join("data"), DATA_CONTENTS).unwrap();
    fs::write(src.join("dir/nested"), NESTED_CONTENTS).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("data", src.join("symlink")).unwrap();

    let out = fixture.dir.path().join("Packed.AppImage");
    let image = pack(&src, &out);

    assert_eq!(image.offset, common::runtime_elf().len() as u64);

    let extracted = fixture.dir.path().join("unpacked");
    let unpacked = image.unpack().to(&extracted).expect("unpack packed image");
    assert_eq!(
        fs::read_to_string(unpacked.join("AppRun")).unwrap(),
        APPRUN_CONTENTS
    );
    assert_eq!(
        fs::read_to_string(unpacked.join("data")).unwrap(),
        DATA_CONTENTS
    );
    assert_eq!(
        fs::read_to_string(unpacked.join("dir/nested")).unwrap(),
        NESTED_CONTENTS
    );
    #[cfg(unix)]
    assert_eq!(
        fs::read_link(unpacked.join("symlink")).unwrap(),
        std::path::Path::new("data")
    );
}

#[test]
fn pack_roundtrips_with_every_compression_backend() {
    // Each compressor and a small block size, end to end: pack, open, unpack.
    for compression in [
        Compression::Gzip,
        Compression::Lz4,
        Compression::Lzo,
        Compression::None,
    ] {
        let fixture = Fixture::new(); // reused only for its temp dir
        let src = fixture.dir.path().join("AppDir");
        fs::create_dir_all(src.join("dir")).unwrap();
        fs::write(src.join("AppRun"), APPRUN_CONTENTS).unwrap();
        fs::write(src.join("data"), DATA_CONTENTS).unwrap();
        fs::write(src.join("dir/nested"), NESTED_CONTENTS).unwrap();

        let out = fixture.dir.path().join("Packed.AppImage");
        let image = pack_with(&src, &out, compression);

        let extracted = fixture.dir.path().join("unpacked");
        let unpacked = image
            .unpack()
            .to(&extracted)
            .unwrap_or_else(|error| panic!("unpack with {compression:?} failed: {error}"));
        assert_eq!(
            fs::read_to_string(unpacked.join("AppRun")).unwrap(),
            APPRUN_CONTENTS
        );
        assert_eq!(
            fs::read_to_string(unpacked.join("data")).unwrap(),
            DATA_CONTENTS
        );
        assert_eq!(
            fs::read_to_string(unpacked.join("dir/nested")).unwrap(),
            NESTED_CONTENTS,
            "compression {compression:?} round-trip mismatch",
        );
    }
}

#[test]
fn runtime_is_validated_trimmed_and_marked() {
    assert!(matches!(
        Runtime::try_from(b"not an elf".to_vec()),
        Err(Error::InvalidRuntime(_))
    ));

    // Bytes after the ELF are where the payload goes, so they are dropped, and
    // the type-2 marker is stamped whatever the runtime started with.
    let mut bytes = common::runtime_elf();
    let elf_len = bytes.len();
    bytes.extend_from_slice(b"trailing garbage");
    let runtime = Runtime::try_from(bytes).expect("valid ELF with trailing data");
    assert_eq!(runtime.len(), elf_len);
    assert_eq!(&runtime[8..11], b"AI\x02");
}

#[test]
fn an_app_dir_that_is_missing_or_not_a_directory_cannot_be_opened() {
    let fixture = Fixture::new();
    let missing = fixture.dir.path().join("missing");
    let error = AppDir::open(&missing).unwrap_err();
    assert!(
        matches!(&error, Error::Io { operation: Operation::Resolve, path, .. } if *path == missing),
        "{error}"
    );
    let error = AppDir::open(&fixture.path).unwrap_err();
    assert!(
        matches!(
            error,
            Error::Io {
                operation: Operation::Open,
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn block_sizes_are_validated_without_panicking() {
    for size in [0, 1, 1024, 4095, 4097, 2_097_152, u32::MAX] {
        assert!(matches!(
            BlockSize::new(size),
            Err(Error::InvalidBlockSize(_))
        ));
    }
    for size in [4096, 131072, 1048576] {
        assert_eq!(BlockSize::new(size).unwrap().bytes(), size);
    }
}

#[test]
fn packing_never_replaces_an_existing_output_unless_asked_and_leaves_no_staging_files() {
    let fixture = Fixture::new();
    let src = fixture.dir.path().join("AppDir");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("AppRun"), APPRUN_CONTENTS).unwrap();
    let output = fixture.dir.path().join("Out.AppImage");
    fs::write(&output, b"precious").unwrap();

    let dir = AppDir::open(&src).unwrap();
    let runtime = Runtime::inert();
    let error = Packer::new(&dir, &runtime).write_to(&output).unwrap_err();
    assert!(
        matches!(&error, Error::Io { operation: Operation::Create, source, .. }
            if source.kind() == std::io::ErrorKind::AlreadyExists),
        "{error}"
    );
    assert_eq!(fs::read(&output).unwrap(), b"precious");

    Packer::new(&dir, &runtime)
        .existing(ExistingFiles::Replace)
        .write_to(&output)
        .expect("replace");
    assert!(AppImage::open(&output).is_ok());
    let leftovers: Vec<_> = fs::read_dir(fixture.dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().starts_with(".appimage-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn a_special_file_in_the_app_dir_is_refused() {
    let fixture = Fixture::new();
    let src = fixture.dir.path().join("AppDir");
    fs::create_dir(&src).unwrap();
    #[cfg(unix)]
    {
        let status = std::process::Command::new("mkfifo")
            .arg(src.join("pipe"))
            .status();
        if !status.is_ok_and(|status| status.success()) {
            return;
        }
        let error = Packer::new(&AppDir::open(&src).unwrap(), &Runtime::inert())
            .write_to(fixture.dir.path().join("x.AppImage"))
            .unwrap_err();
        assert!(
            matches!(error, Error::AppDir(AppDirError::UnsupportedFile(_))),
            "{error}"
        );
    }
}

#[test]
fn a_temporary_payload_can_be_borrowed_for_packing() {
    let fixture = Fixture::new();
    let image = AppImage::open(&fixture.path).unwrap();
    let scratch = image.unpack().to_temp().unwrap();
    let repacked = Packer::new(&scratch, &Runtime::inert())
        .write_to(fixture.dir.path().join("Repacked.AppImage"))
        .unwrap();
    let again = repacked.unpack().to_temp().unwrap();
    assert_eq!(
        fs::read_to_string(again.join("dir/nested")).unwrap(),
        NESTED_CONTENTS
    );
}

#[test]
fn sidecars_are_adjacent_paths_and_must_be_directories() {
    let fixture = Fixture::new();
    let image = AppImage::open(&fixture.path).expect("open");

    let home = Sidecar::<Home>::of(&image);
    assert!(home.ends_with("App.AppImage.home"));
    assert!(Sidecar::<Config>::of(&image).ends_with("App.AppImage.config"));
    assert_eq!(home.existing().unwrap(), None);

    assert_eq!(home.ensure().unwrap(), &*home);
    assert_eq!(home.ensure().unwrap(), &*home, "making it twice is fine");
    assert_eq!(home.existing().unwrap(), Some(&*home));

    let config = Sidecar::<Config>::of(&image);
    fs::write(&config, "not a directory").unwrap();
    assert!(matches!(config.existing(), Err(Error::NotADirectory(_))));
    assert!(matches!(config.ensure(), Err(Error::NotADirectory(_))));
}

#[cfg(target_os = "linux")]
#[test]
fn mount_serves_the_payload_when_fuse_is_available() {
    let fixture = Fixture::new();
    let image = AppImage::open(&fixture.path).expect("open");

    // FUSE may be unavailable (no /dev/fuse, no permission to mount) in sandboxes;
    // a mount failure is tolerated, but a successful mount must serve files.
    match image.mount() {
        Ok(mount) => {
            let mountpoint = mount.to_path_buf();
            assert!(mountpoint.is_dir());
            assert_eq!(
                fs::read_to_string(mountpoint.join("data")).unwrap(),
                DATA_CONTENTS
            );
            assert_eq!(
                fs::read_to_string(mountpoint.join("dir/nested")).unwrap(),
                NESTED_CONTENTS
            );
            assert_eq!(
                fs::read_link(mountpoint.join("symlink")).unwrap(),
                std::path::Path::new("data")
            );
            drop(mount);
            assert!(!mountpoint.exists(), "owned mountpoint removed on unmount");
        }
        Err(error) => {
            eprintln!("skipping mount assertions: {error}");
        }
    }
}

/// Pack `src` behind the fixture's hand-made runtime.
fn pack(src: &std::path::Path, output: &std::path::Path) -> AppImage {
    pack_with(src, output, Compression::Gzip)
}

fn pack_with(
    src: &std::path::Path,
    output: &std::path::Path,
    compression: Compression,
) -> AppImage {
    let runtime = Runtime::try_from(common::runtime_elf()).expect("fixture runtime");
    Packer::new(&AppDir::open(src).expect("open AppDir"), &runtime)
        .compression(compression)
        .block_size(BlockSize::new(4096).unwrap())
        .write_to(output)
        .unwrap_or_else(|error| panic!("pack with {compression:?} failed: {error}"))
}

#[test]
fn update_information_that_is_not_text_is_an_error_and_blank_is_none() {
    let binary = Fixture::with_update_info(b"zsync|\xff\xfe");
    let image = AppImage::open(&binary.path).expect("open");
    assert!(matches!(
        image.update_information(),
        Err(Error::SectionEncoding {
            section: appimage::section::UPDATE_INFO,
            ..
        })
    ));

    let blank = Fixture::with_update_info(b"  \n\0 ");
    let image = AppImage::open(&blank.path).expect("open");
    assert!(matches!(image.update_information(), Ok(None)));
}

#[test]
fn filesystem_errors_name_the_operation_and_the_path() {
    let fixture = Fixture::new();
    let output = fixture.dir.path().join("missing").join("out.AppImage");
    let dir = AppDir::open(fixture.dir.path()).unwrap();
    let error = Packer::new(&dir, &Runtime::inert())
        .write_to(&output)
        .unwrap_err();
    let Error::Io {
        operation,
        path,
        source,
    } = &error
    else {
        panic!("expected an I/O error, got {error}");
    };
    assert_eq!(*operation, Operation::Create);
    // The image is staged beside its destination, which is what is missing.
    assert_eq!(path.as_path(), output.parent().unwrap());
    assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn closing_scratch_removes_a_locked_tree_and_says_so() {
    let fixture = Fixture::new();
    let image = AppImage::open(&fixture.path).expect("open");
    let scratch = image.unpack().to_temp().expect("unpack to scratch");
    let path = scratch.to_path_buf();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path.join("dir"), fs::Permissions::from_mode(0o555)).unwrap();
    }
    scratch.close().expect("close");
    assert!(!path.exists());
}
