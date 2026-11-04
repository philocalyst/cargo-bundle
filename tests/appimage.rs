use std::{fs, process::Command};

#[test]
fn appimage_cli_uses_embedded_launcher_and_preserves_output_on_failure() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    let manifest = r#"[package]
name = "postcard"
version = "1.2.3"
edition = "2024"
[package.metadata.bundle]
name = "Postcard"
identifier = "com.example.postcard"
appimage_compression = "gzip"
"#;
    fs::write(temp.path().join("Cargo.toml"), manifest).unwrap();
    // Use the architecture-matched ELF fixture without invoking a cross compiler.
    let binary = temp.path().join("prebuilt");
    fs::write(&binary, include_bytes!("fixtures/android/libpostcard.so")).unwrap();
    let bundle = || {
        Command::new(env!("CARGO_BIN_EXE_cargo-bundle"))
            .args([
                "bundle",
                "--format",
                "appimage",
                "--target",
                "aarch64-unknown-linux-gnu",
                "--binary-path",
            ])
            .arg(&binary)
            .current_dir(temp.path())
            .output()
            .unwrap()
    };
    let built = bundle();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let output = temp.path().join(
        "target/aarch64-unknown-linux-gnu/debug/bundle/appimage/postcard_1.2.3_aarch64.AppImage",
    );
    let image = appimage::AppImage::open(&output).unwrap();
    let extracted = temp.path().join("extracted");
    image.unpack().to(&extracted).unwrap();
    assert_eq!(
        fs::read(extracted.join("usr/bin/postcard")).unwrap(),
        fs::read(&binary).unwrap()
    );
    let bytes = fs::read(&output).unwrap();
    assert_eq!(&bytes[8..11], b"AI\x02");
    assert_eq!(
        u16::from_le_bytes([bytes[18], bytes[19]]),
        183,
        "launcher must target aarch64"
    );
    fs::write(
        temp.path().join("Cargo.toml"),
        manifest.replace(
            "appimage_compression = \"gzip\"",
            "appimage_compression = \"invalid\"",
        ),
    )
    .unwrap();
    assert!(!bundle().status.success());
    assert_eq!(fs::read(&output).unwrap(), bytes);
}
