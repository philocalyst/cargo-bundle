use std::{fs, process::Command};

#[test]
fn apk_cli_builds_signed_native_activity_with_assets_and_preserves_output_on_failure() {
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join("src")).unwrap();
    fs::write(project.path().join("src/lib.rs"), "").unwrap();
    fs::write(
        project.path().join("Cargo.toml"),
        r#"[package]
name = "postcard"
version = "1.2.3"
edition = "2024"
[lib]
crate-type = ["cdylib"]
[package.metadata.bundle]
name = "Postcard"
identifier = "com.example.postcard"
resources = ["greeting.txt"]
[package.metadata.bundle.android]
version_code = 7
min_sdk = 24
target_sdk = 35
permissions = ["android.permission.INTERNET"]
signing_key = "test.pem"
"#,
    )
    .unwrap();
    fs::write(project.path().join("greeting.txt"), "hello Android").unwrap();
    fs::write(
        project.path().join("test.pem"),
        include_bytes!("fixtures/android/test-only.pem"),
    )
    .unwrap();
    let library = project.path().join("prebuilt.so");
    fs::write(&library, include_bytes!("fixtures/android/libpostcard.so")).unwrap();
    let bundle = || {
        Command::new(env!("CARGO_BIN_EXE_cargo-bundle"))
            .args([
                "bundle",
                "--format",
                "apk",
                "--target",
                "aarch64-linux-android",
                "--binary-path",
            ])
            .arg(&library)
            .current_dir(project.path())
            .output()
            .unwrap()
    };
    let result = bundle();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = project
        .path()
        .join("target/aarch64-linux-android/debug/bundle/apk/Postcard.apk");
    let verification = apk::sign::verify(&output).unwrap();
    assert!(verification.schemes.contains(apk::sign::Schemes::V2));
    let mut archive = apk::Apk::open(&output).unwrap();
    assert_eq!(
        archive.read("assets/greeting.txt").unwrap(),
        b"hello Android"
    );
    assert_eq!(
        archive.read("lib/arm64-v8a/libpostcard.so").unwrap(),
        include_bytes!("fixtures/android/libpostcard.so")
    );
    let manifest = archive.manifest().unwrap().to_string();
    assert!(manifest.contains("com.example.postcard"));
    assert!(manifest.contains("versionCode=\"7\""));
    assert!(manifest.contains("android.app.NativeActivity"));
    drop(archive);
    let original = fs::read(&output).unwrap();
    fs::write(&library, b"not an ELF").unwrap();
    assert!(!bundle().status.success());
    assert_eq!(fs::read(&output).unwrap(), original);
}
