use std::{fs, path::Path, process::Command};

#[test]
fn incomplete_notarization_and_ios_requests_fail_before_writing_bundles() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"notary-check\"\nversion = \"1.0.0\"\nedition = \"2024\"\n\
         [package.metadata.bundle]\napple_notarization_api_key = \"notary.json\"\n",
    )
    .unwrap();
    let binary = temp.path().join("prebuilt");
    fs::write(&binary, b"prebuilt fixture\n").unwrap();
    for (format, message) in [
        ("osx", "Apple notarization requires"),
        ("ios", "not iOS bundles"),
        ("ipa", "not iOS bundles"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-bundle"))
            .args(["bundle", "--format", format, "--binary-path"])
            .arg(&binary)
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "unexpected error: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(!temp.path().join("target/debug/bundle").exists());
}

fn bundle(workspace: &Path, binary: &Path, format: &str, target: &str) {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-bundle"))
        .args([
            "bundle",
            "--package",
            "postcard",
            "--format",
            format,
            "--target",
            target,
            "--binary-path",
        ])
        .arg(binary)
        .current_dir(workspace)
        .output()
        .expect("run cargo-bundle");
    assert!(
        output.status.success(),
        "{format} packaging failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn split_locales_follow_selected_package_and_reach_linux_and_apple_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path();
    let package = workspace.join("postcard");
    fs::create_dir_all(package.join("src")).unwrap();
    fs::create_dir(package.join("locales")).unwrap();
    fs::write(
        workspace.join("Cargo.toml"),
        "[workspace]\nmembers = [\"postcard\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    fs::write(package.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(
        package.join("Cargo.toml"),
        r#"[package]
name = "postcard"
version = "1.0.0"
edition = "2024"
[package.metadata.bundle]
name = "Postcard"
identifier = "com.example.postcard"
localization_files = ["locales/fr.toml"]
[package.metadata.bundle.linux.localizations.fr]
Name = "Nom personnalisé"
[package.metadata.bundle.ios.localizations.fr]
CFBundleDisplayName = "Nom iOS"
"#,
    )
    .unwrap();
    fs::write(
        package.join("locales/fr.toml"),
        r#"[linux.fr]
Name = "Carte postale"
Comment = "Créez des cartes"
[macos.fr]
CFBundleDisplayName = "Carte postale"
[ios.fr]
CFBundleDisplayName = "Carte postale"
NSHumanReadableCopyright = "Droits réservés"
"#,
    )
    .unwrap();
    let binary = workspace.join("prebuilt");
    // These writers copy a prebuilt executable unchanged; this test inspects
    // metadata and resource placement rather than executable behavior.
    fs::write(&binary, b"prebuilt executable fixture\n").unwrap();

    bundle(workspace, &binary, "deb", "aarch64-unknown-linux-gnu");
    let desktop = fs::read_to_string(workspace.join(
        "target/aarch64-unknown-linux-gnu/debug/bundle/deb/postcard_1.0.0_arm64/data/usr/share/applications/postcard.desktop",
    ))
    .unwrap();
    assert!(desktop.contains("Name[fr]=Nom personnalisé\n"));
    assert!(desktop.contains("Comment[fr]=Créez des cartes\n"));

    bundle(workspace, &binary, "osx", "aarch64-apple-darwin");
    let apple = fs::read_to_string(workspace.join(
        "target/aarch64-apple-darwin/debug/bundle/osx/Postcard.app/Contents/Resources/fr.lproj/InfoPlist.strings",
    ))
    .unwrap();
    assert!(apple.contains("\"CFBundleDisplayName\" = \"Carte postale\";"));

    bundle(workspace, &binary, "ios", "aarch64-apple-ios");
    let ios =
        fs::read_to_string(workspace.join(
            "target/aarch64-apple-ios/debug/bundle/ios/Postcard.app/fr.lproj/InfoPlist.strings",
        ))
        .unwrap();
    assert!(ios.contains("\"CFBundleDisplayName\" = \"Nom iOS\";"));
    assert!(ios.contains("\"NSHumanReadableCopyright\" = \"Droits réservés\";"));
}
