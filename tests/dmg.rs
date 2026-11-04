#![cfg(unix)]

use dmg_layout::AliasRecord;
use ds_parser::{FieldKey, Value as DsValue, parse as parse_ds_store};
use std::{
    fs,
    io::Read,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output},
};

const BACKGROUND_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="500" height="300">
  <rect width="500" height="300" fill="#e8ecf4"/>
  <rect id="app" x="80" y="120" width="96" height="96" fill="none"/>
  <rect id="applications" x="320" y="120" width="96" height="96" fill="none"/>
</svg>
"##;

fn run_bundle(project: &Path, binary: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-bundle"))
        .args(["bundle", "--format", "dmg", "--binary-path"])
        .arg(binary)
        .current_dir(project)
        .output()
        .expect("Failed to execute cargo-bundle")
}

fn read_file(
    volume: &fstool::fs::hfs_plus::HfsPlus,
    device: &mut dyn fstool::block::BlockDevice,
    path: &str,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    volume
        .open_file_reader(device, path)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}

#[test]
fn dmg_cli_writes_finder_alias_from_image_catalog_and_preserves_old_image_on_failure() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path();
    fs::create_dir(project.join("src")).unwrap();
    fs::write(project.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(
        project.join("Cargo.toml"),
        r#"[package]
name = "dmg-oracle"
version = "1.0.0"
edition = "2024"

[package.metadata.bundle]
name = "Dmg Oracle"
identifier = "com.example.dmgoracle"

[package.metadata.bundle.osx]
dmg_background = "background.svg"
"#,
    )
    .unwrap();
    let background = project.join("background.svg");
    fs::write(&background, BACKGROUND_SVG).unwrap();
    let binary = project.join("prebuilt");
    fs::write(&binary, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();

    let output = run_bundle(project, &binary);
    assert!(
        output.status.success(),
        "cargo-bundle failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let image = project.join("target/debug/bundle/dmg/Dmg Oracle.dmg");
    assert!(image.is_file(), "DMG was not created at {image:?}");

    use fstool::{
        block::dmg::{DmgBackend, verify_udif},
        fs::{
            Filesystem,
            hfs_plus::{HfsPlus, verify::verify},
        },
    };
    assert!(verify_udif(&image).unwrap().is_valid());
    let mut device = DmgBackend::open(&image).unwrap();
    let report = verify(&mut device, &Default::default()).unwrap();
    assert!(
        report.is_clean(),
        "HFS+ consistency errors: {:?}",
        report.diagnostics
    );
    let mut volume = HfsPlus::open(&mut device).unwrap();
    let executable = Path::new("/Dmg Oracle.app/Contents/MacOS/dmg-oracle");
    assert_eq!(
        volume.getattr(&mut device, executable).unwrap().mode & 0o777,
        0o755
    );
    assert_eq!(
        read_file(&volume, &mut device, executable.to_str().unwrap()),
        fs::read(&binary).unwrap()
    );
    assert_eq!(
        volume
            .read_symlink_target_path(&mut device, "/Applications")
            .unwrap(),
        "/Applications"
    );
    let background_bytes = read_file(&volume, &mut device, "/.background/background.tiff");
    assert!(!background_bytes.is_empty());
    let ds_store = read_file(&volume, &mut device, "/.DS_Store");
    let parsed = parse_ds_store(&ds_store).expect("generated .DS_Store did not parse");
    assert!(
        parsed.warnings.is_empty(),
        "Finder store warnings: {:?}",
        parsed.warnings
    );
    let icon_view = parsed
        .records
        .iter()
        .find(|record| record.name == "." && record.field == FieldKey::icvp)
        .expect(".DS_Store has no icon view properties");
    let alias_bytes = match &icon_view.value {
        DsValue::IconViewProperties(properties) => properties
            .unknown
            .get("backgroundImageAlias")
            .and_then(plist::Value::as_data)
            .expect("icon view properties have no background image alias"),
        other => panic!("unexpected .DS_Store icvp value: {other:?}"),
    };
    let DsValue::IconViewProperties(properties) = &icon_view.value else {
        unreachable!()
    };
    assert_eq!(properties.background_type, Some(2));
    assert_eq!(properties.icon_size, Some(48.0));
    let alias = AliasRecord::from_bytes(alias_bytes).expect("Finder alias did not parse");
    let parent = volume
        .getattr(&mut device, Path::new("/.background"))
        .unwrap();
    let target = volume
        .getattr(&mut device, Path::new("/.background/background.tiff"))
        .unwrap();
    assert_eq!(alias.header.parent_catalog_node_id.0, parent.inode);
    assert_eq!(alias.header.target_catalog_node_id.0, target.inode);
    assert_eq!(alias.header.volume_name, b"Dmg Oracle");
    assert_eq!(
        alias.header.volume_date.0,
        volume.volume_create_date().unwrap()
    );
    assert_eq!(
        alias.header.target_date.0,
        volume
            .entry_info("/.background/background.tiff")
            .unwrap()
            .create_date
    );
    drop(device);

    let original = fs::read(&image).unwrap();
    fs::write(&background, "<svg broken").unwrap();
    let failed = run_bundle(project, &binary);
    assert!(!failed.status.success(), "malformed SVG unexpectedly built");
    assert_eq!(fs::read(&image).unwrap(), original);
}
