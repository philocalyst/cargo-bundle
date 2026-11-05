mod common;

use std::fs;
use std::path::Path;

use appimage::{
    AppDir, AppDirError, AppId, AppPath, AppRun, DesktopCommand, DesktopEntry, Error, Icon, Packer,
    Runtime,
};
use tempfile::TempDir;

fn metadata(app: &mut AppDir, temp: &TempDir) {
    let icon = temp.path().join("icon.png");
    fs::write(&icon, b"icon contents").unwrap();
    let mut entry = DesktopEntry::new("My App", DesktopCommand::new("app"));
    entry.categories = &["Utility"];
    app.write_desktop_entry(&AppId::new("app").unwrap(), &entry, Icon::Png(&icon))
        .unwrap();
}

#[test]
fn appdir_roundtrips_with_launcher_and_matching_metadata() {
    let temp = TempDir::new().unwrap();
    let binary = temp.path().join("app");
    fs::write(&binary, common::APPRUN_CONTENTS).unwrap();
    let mut app = AppDir::create(temp.path().join("App.AppDir")).unwrap();
    let mut executable = app
        .copy_file(&binary, AppPath::new("usr/bin/app").unwrap())
        .unwrap();
    assert_eq!(&*executable, Path::new("usr/bin/app"));
    executable.set_apprun(AppRun::Script).unwrap();
    metadata(&mut app, &temp);
    app.validate().unwrap();

    let desktop = fs::read_to_string(app.join("app.desktop")).unwrap();
    assert!(desktop.contains("Name=My\\sApp\n"));
    assert!(desktop.contains("Icon=app\n"));
    assert!(desktop.contains("Categories=Utility;\n"));

    let runtime = Runtime::try_from(common::runtime_elf()).unwrap();
    let image = Packer::new(&app, &runtime)
        .write_to(temp.path().join("app.AppImage"))
        .unwrap();
    let unpacked = image.unpack().to(temp.path().join("unpacked")).unwrap();
    unpacked.validate().unwrap();
    assert_eq!(
        fs::read(unpacked.join("usr/bin/app")).unwrap(),
        common::APPRUN_CONTENTS.as_bytes()
    );
    assert_eq!(
        fs::read(unpacked.join(".DirIcon")).unwrap(),
        b"icon contents"
    );
}

#[test]
fn paths_require_a_real_destination_inside_the_appdir() {
    for invalid in [
        "",
        ".",
        "./",
        "..",
        "../escape",
        "usr/../../escape",
        "/absolute",
        "app\0",
    ] {
        assert!(matches!(
            AppPath::new(invalid),
            Err(Error::AppDir(AppDirError::InvalidPath(_)))
        ));
    }
    assert_eq!(
        &*AppPath::new("./usr/./bin/app").unwrap(),
        Path::new("usr/bin/app")
    );
}

#[cfg(unix)]
#[test]
fn operations_borrow_the_open_directory_even_if_its_path_is_replaced() {
    let temp = TempDir::new().unwrap();
    let original = temp.path().join("appdir");
    let moved = temp.path().join("moved");
    let source = temp.path().join("source");
    fs::write(&source, common::APPRUN_CONTENTS).unwrap();
    let mut app = AppDir::create(&original).unwrap();
    fs::rename(&original, &moved).unwrap();
    fs::create_dir(&original).unwrap();
    app.copy_file(&source, AppPath::new("bin/app").unwrap())
        .unwrap()
        .set_apprun(AppRun::Script)
        .unwrap();
    assert!(moved.join("bin/app").is_file());
    assert!(moved.join("AppRun").is_file());
    assert_eq!(fs::read_dir(&original).unwrap().count(), 0);
    assert!(matches!(
        app.copy_directory(&moved, AppPath::new("copy").unwrap()),
        Err(Error::AppDir(AppDirError::OverlappingDirectories))
    ));
    assert!(!moved.join("copy").exists());
}

#[test]
fn creation_and_copying_never_replace_existing_contents() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("appdir");
    let mut app = AppDir::create(&root).unwrap();
    assert!(
        matches!(AppDir::create(&root), Err(Error::Io { source: error, .. }) if error.kind() == std::io::ErrorKind::AlreadyExists)
    );
    let source = temp.path().join("source");
    fs::write(&source, "original").unwrap();
    app.copy_file(&source, AppPath::new("data").unwrap())
        .unwrap();
    fs::write(&source, "replacement").unwrap();
    assert!(
        matches!(app.copy_file(&source, AppPath::new("data").unwrap()), Err(Error::Io { source: error, .. }) if error.kind() == std::io::ErrorKind::AlreadyExists)
    );
    assert_eq!(fs::read_to_string(root.join("data")).unwrap(), "original");
    app.copy_file(&source, AppPath::new("other").unwrap())
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join("other")).unwrap(),
        "replacement"
    );
    assert!(matches!(
        app.copy_file(temp.path(), AppPath::new("bad/source").unwrap()),
        Err(Error::AppDir(AppDirError::UnsupportedFile(_)))
    ));
    assert!(!root.join("bad").exists());
}

#[test]
fn desktop_write_rolls_back_new_files_on_collision_and_can_be_retried() {
    let temp = TempDir::new().unwrap();
    let mut app = AppDir::create(temp.path().join("appdir")).unwrap();
    let icon = temp.path().join("icon.png");
    fs::write(&icon, "icon").unwrap();
    fs::write(app.join(".DirIcon"), "existing").unwrap();
    let id = AppId::new("app").unwrap();
    let entry = DesktopEntry::new("App", DesktopCommand::new("app"));
    assert!(
        app.write_desktop_entry(&id, &entry, Icon::Png(&icon))
            .is_err()
    );
    assert!(!app.join("app.png").exists());
    assert!(!app.join("app.desktop").exists());
    assert_eq!(
        fs::read_to_string(app.join(".DirIcon")).unwrap(),
        "existing"
    );
    fs::remove_file(app.join(".DirIcon")).unwrap();
    app.write_desktop_entry(&id, &entry, Icon::Png(&icon))
        .unwrap();
    assert!(matches!(
        app.write_desktop_entry(&AppId::new("second").unwrap(), &entry, Icon::Png(&icon)),
        Err(Error::AppDir(AppDirError::DesktopFileCount(1)))
    ));
    assert!(!app.join("second.png").exists());
}

#[test]
fn invalid_metadata_is_rejected_before_creating_files() {
    let temp = TempDir::new().unwrap();
    let mut app = AppDir::create(temp.path().join("appdir")).unwrap();
    let entry = DesktopEntry::new("App", DesktopCommand::new(""));
    assert!(matches!(
        app.write_desktop_entry(
            &AppId::new("app").unwrap(),
            &entry,
            Icon::Png(Path::new("missing.png"))
        ),
        Err(Error::AppDir(AppDirError::DesktopField {
            field: "Exec",
            ..
        }))
    ));
    assert_eq!(fs::read_dir(&*app).unwrap().count(), 0);
    assert!(matches!(
        Icon::from_path(Path::new("icon.bmp")),
        Err(Error::AppDir(AppDirError::UnsupportedIcon(_)))
    ));
    assert!(matches!(
        Icon::from_path(Path::new("ICON.PNG")),
        Ok(Icon::Png(_))
    ));
}

#[test]
fn validation_requires_all_four_layout_components_and_one_desktop_file() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    fs::write(&source, common::APPRUN_CONTENTS).unwrap();
    let mut app = AppDir::create(temp.path().join("appdir")).unwrap();
    assert!(matches!(
        app.validate(),
        Err(Error::AppDir(AppDirError::MissingFile(_)))
    ));
    app.copy_file(&source, AppPath::new("usr/bin/app").unwrap())
        .unwrap()
        .set_apprun(AppRun::Direct)
        .unwrap();
    assert!(matches!(
        app.validate(),
        Err(Error::AppDir(AppDirError::DesktopFileCount(0)))
    ));
    metadata(&mut app, &temp);
    app.validate().unwrap();
    fs::write(app.join("second.desktop"), "").unwrap();
    assert!(matches!(
        app.validate(),
        Err(Error::AppDir(AppDirError::DesktopFileCount(2)))
    ));
    fs::remove_file(app.join("second.desktop")).unwrap();
    fs::remove_file(app.join(".DirIcon")).unwrap();
    assert!(matches!(
        app.validate(),
        Err(Error::AppDir(AppDirError::MissingFile(_)))
    ));
    fs::write(app.join(".DirIcon"), "icon").unwrap();
    fs::remove_file(app.join("app.png")).unwrap();
    assert!(matches!(
        app.validate(),
        Err(Error::AppDir(AppDirError::MissingFile(_)))
    ));
}

#[test]
fn directories_and_duplicate_fields_are_not_valid_desktop_files() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    fs::write(&source, common::APPRUN_CONTENTS).unwrap();
    let mut app = AppDir::create(temp.path().join("appdir")).unwrap();
    app.copy_file(&source, AppPath::new("app").unwrap())
        .unwrap()
        .set_apprun(AppRun::Script)
        .unwrap();
    fs::create_dir(app.join("fake.desktop")).unwrap();
    assert!(matches!(
        app.validate(),
        Err(Error::AppDir(AppDirError::MissingFile(_)))
    ));
    fs::remove_dir(app.join("fake.desktop")).unwrap();
    metadata(&mut app, &temp);
    let desktop = app.join("app.desktop");
    let original = fs::read_to_string(&desktop).unwrap();
    for extra in [
        "Type=Link\n",
        "Icon=other\n",
        "[Desktop Entry]\nType=Application\n",
    ] {
        fs::write(&desktop, format!("{original}{extra}")).unwrap();
        assert!(matches!(
            app.validate(),
            Err(Error::AppDir(AppDirError::DesktopField { .. }))
        ));
    }
}

#[cfg(unix)]
#[test]
fn destination_symlinks_cannot_escape_or_overwrite_files() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    fs::write(&source, "new").unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), "original").unwrap();
    let mut app = AppDir::create(temp.path().join("appdir")).unwrap();
    symlink(&outside, app.join("escape")).unwrap();
    assert!(
        app.copy_file(&source, AppPath::new("escape/new").unwrap())
            .is_err()
    );
    assert!(!outside.join("new").exists());
    symlink(outside.join("keep"), app.join("leaf")).unwrap();
    assert!(
        app.copy_file(&source, AppPath::new("leaf").unwrap())
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(outside.join("keep")).unwrap(),
        "original"
    );
    assert!(AppDir::create(app.join("escape")).is_err());
}

#[cfg(unix)]
#[test]
fn script_survives_relocation_quotes_paths_and_forwards_arguments_and_status() {
    use std::process::Command;
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    fs::write(&source, "#!/bin/sh\nprintf '%s\\0' \"$@\"\nexit 17\n").unwrap();
    let mut app = AppDir::create(temp.path().join("original")).unwrap();
    let target = "usr/bin/app ' \" $(touch PWNED) `touch PWNED` \\ %\n";
    app.copy_file(&source, AppPath::new(target).unwrap())
        .unwrap()
        .set_apprun(AppRun::Script)
        .unwrap();
    let moved = temp.path().join("moved ' AppDir");
    fs::rename(&*app, &moved).unwrap();
    let output = Command::new(moved.join("AppRun"))
        .args(["a b", "", "$(touch PWNED)", "café\n"])
        .env("CDPATH", "/")
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(17),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, "a b\0\0$(touch PWNED)\0café\n\0".as_bytes());
    assert!(!temp.path().join("PWNED").exists());
    assert!(!moved.join("PWNED").exists());
}

#[cfg(unix)]
#[test]
fn apprun_collision_does_not_change_target_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    fs::write(&source, "payload").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
    let mut app = AppDir::create(temp.path().join("appdir")).unwrap();
    let root = app.to_path_buf();
    fs::write(root.join("AppRun"), "keep").unwrap();
    let mut file = app
        .copy_file(&source, AppPath::new("bin/app").unwrap())
        .unwrap();
    for mode in [AppRun::Direct, AppRun::Script] {
        assert!(file.set_apprun(mode).is_err());
        assert_eq!(
            fs::metadata(root.join("bin/app"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
        assert_eq!(fs::read_to_string(root.join("AppRun")).unwrap(), "keep");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn script_rejects_non_utf8_paths_without_lossy_conversion() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    fs::write(&source, common::APPRUN_CONTENTS).unwrap();
    let mut app = AppDir::create(temp.path().join("appdir")).unwrap();
    let root = app.to_path_buf();
    let path = OsString::from_vec(b"app\xff".to_vec());
    let mut file = app.copy_file(&source, AppPath::new(path).unwrap()).unwrap();
    assert!(matches!(
        file.set_apprun(AppRun::Script),
        Err(Error::AppDir(AppDirError::NonUtf8ScriptPath(_)))
    ));
    assert!(!root.join("AppRun").exists());
    file.set_apprun(AppRun::Direct).unwrap();
    assert_eq!(
        fs::read(root.join("AppRun")).unwrap(),
        common::APPRUN_CONTENTS.as_bytes()
    );
}

#[test]
fn directory_copy_is_exclusive_and_copied_executables_can_be_borrowed() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("bin")).unwrap();
    fs::create_dir(source.join("empty")).unwrap();
    fs::write(source.join("bin/app"), common::APPRUN_CONTENTS).unwrap();
    let mut app = AppDir::create(temp.path().join("appdir")).unwrap();
    app.copy_directory(&source, AppPath::new("usr").unwrap())
        .unwrap();
    assert!(app.join("usr/empty").is_dir());
    app.file(AppPath::new("usr/bin/app").unwrap())
        .unwrap()
        .set_apprun(AppRun::Script)
        .unwrap();
    metadata(&mut app, &temp);
    app.validate().unwrap();
    assert!(
        app.copy_directory(&source, AppPath::new("usr").unwrap())
            .is_err()
    );
    assert_eq!(
        fs::read(app.join("usr/bin/app")).unwrap(),
        common::APPRUN_CONTENTS.as_bytes()
    );
    assert!(matches!(
        app.file(AppPath::new("missing").unwrap()),
        Err(Error::AppDir(AppDirError::MissingFile(_)))
    ));
}

#[test]
fn directory_copy_rejects_overlapping_trees_before_creating_destinations() {
    let temp = TempDir::new().unwrap();
    let mut app = AppDir::create(temp.path().join("appdir")).unwrap();
    let root = app.to_path_buf();
    for source in [temp.path(), &root] {
        assert!(matches!(
            app.copy_directory(source, AppPath::new("nested/copy").unwrap()),
            Err(Error::AppDir(AppDirError::OverlappingDirectories))
        ));
    }
    fs::create_dir(root.join("inside")).unwrap();
    assert!(matches!(
        app.copy_directory(root.join("inside"), AppPath::new("copy").unwrap()),
        Err(Error::AppDir(AppDirError::OverlappingDirectories))
    ));
    assert!(!root.join("nested").exists());
    assert!(!root.join("copy").exists());
}

#[cfg(unix)]
#[test]
fn directory_copy_preserves_internal_links_without_following_directory_links() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("lib")).unwrap();
    fs::create_dir(source.join("bin")).unwrap();
    fs::write(source.join("lib/data"), "payload").unwrap();
    symlink("../lib/data", source.join("bin/data")).unwrap();
    symlink("bin/data", source.join("chain")).unwrap();
    symlink(".", source.join("loop")).unwrap();
    let mut app = AppDir::create(temp.path().join("appdir")).unwrap();
    app.copy_directory(&source, AppPath::new("usr").unwrap())
        .unwrap();
    assert_eq!(
        fs::read_link(app.join("usr/bin/data")).unwrap(),
        Path::new("../lib/data")
    );
    assert_eq!(
        fs::read_link(app.join("usr/chain")).unwrap(),
        Path::new("lib/data")
    );
    assert_eq!(fs::read_link(app.join("usr/loop")).unwrap(), Path::new("."));
    assert_eq!(
        fs::read_to_string(app.join("usr/chain")).unwrap(),
        "payload"
    );
}

#[cfg(unix)]
#[test]
fn directory_copy_rejects_escaping_broken_and_cyclic_links_and_cleans_up() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("a-file"), "copied before the bad link").unwrap();
    let outside = temp.path().join("outside");
    fs::write(&outside, "untouched").unwrap();
    let mut app = AppDir::create(temp.path().join("appdir")).unwrap();
    for target in [
        Path::new("../outside"),
        &outside,
        Path::new("missing"),
        Path::new("z-link"),
    ] {
        symlink(target, source.join("z-link")).unwrap();
        assert!(
            app.copy_directory(&source, AppPath::new("usr").unwrap())
                .is_err()
        );
        assert!(!app.join("usr").exists());
        assert_eq!(fs::read_to_string(&outside).unwrap(), "untouched");
        fs::remove_file(source.join("z-link")).unwrap();
    }
    app.copy_directory(&source, AppPath::new("usr").unwrap())
        .unwrap();
    assert!(app.join("usr/a-file").is_file());
}

#[cfg(unix)]
#[test]
fn lexical_link_checks_cannot_hide_an_escape_through_another_link() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(temp.path().join("outside"), "outside").unwrap();
    symlink(".", source.join("b")).unwrap();
    symlink("b/../outside", source.join("a")).unwrap();
    let mut app = AppDir::create(temp.path().join("appdir")).unwrap();
    assert!(matches!(
        app.copy_directory(&source, AppPath::new("usr").unwrap()),
        Err(Error::AppDir(AppDirError::EscapingSymlink(_)))
    ));
    assert!(!app.join("usr").exists());
}

#[cfg(unix)]
#[test]
fn directory_copy_rejects_special_files_without_opening_them() {
    use std::os::unix::net::UnixListener;
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let _socket = UnixListener::bind(source.join("socket")).unwrap();
    let mut app = AppDir::create(temp.path().join("appdir")).unwrap();
    assert!(matches!(
        app.copy_directory(&source, AppPath::new("usr").unwrap()),
        Err(Error::AppDir(AppDirError::UnsupportedFile(_)))
    ));
    assert!(!app.join("usr").exists());
}

#[cfg(unix)]
#[test]
fn file_permissions_are_preserved_without_privileged_bits_and_apprun_must_be_executable() {
    use std::os::unix::fs::PermissionsExt;
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    fs::write(&source, common::APPRUN_CONTENTS).unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o4640)).unwrap();
    let mut app = AppDir::create(temp.path().join("appdir")).unwrap();
    app.copy_file(&source, AppPath::new("AppRun").unwrap())
        .unwrap();
    assert_eq!(
        fs::metadata(app.join("AppRun"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o640
    );
    assert!(matches!(
        app.validate(),
        Err(Error::AppDir(AppDirError::NotExecutable(_)))
    ));
    let root = app.to_path_buf();
    fs::remove_file(root.join("AppRun")).unwrap();
    app.copy_file(&source, AppPath::new("bin/app").unwrap())
        .unwrap()
        .set_apprun(AppRun::Direct)
        .unwrap();
    assert_eq!(
        fs::metadata(root.join("AppRun"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o751
    );
    assert_eq!(
        fs::metadata(&source).unwrap().permissions().mode() & 0o7777,
        0o4640
    );
}
