use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

fn signing_identity(path: &Path) {
    let (certificate, key) = apple_codesign::create_self_signed_code_signing_certificate(
        x509_certificate::KeyAlgorithm::Ed25519,
        apple_codesign::CertificateProfile::AppleDevelopment,
        "TESTTEAM01",
        "IPA test",
        "US",
        chrono::Duration::hours(1),
    )
    .unwrap();
    let pfx = p12::PFX::new(
        certificate.constructed_data(),
        &key.to_pkcs8_one_asymmetric_key_der(),
        None,
        "",
        "IPA test",
    )
    .unwrap();
    fs::write(path, pfx.to_der()).unwrap();
}

fn project(directory: &Path) -> PathBuf {
    fs::create_dir(directory.join("src")).unwrap();
    fs::write(directory.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(
        directory.join("Cargo.toml"),
        r#"[package]
name = "postcard"
version = "1.0.0"
edition = "2024"
[package.metadata.bundle]
name = "Post.Card"
identifier = "com.example.postcard"
[package.metadata.bundle.ios]
[package.metadata.bundle.macos]
[package.metadata.bundle.ios.localizations.fr]
CFBundleDisplayName = "Carte postale"
"#,
    )
    .unwrap();
    let binary = directory.join("prebuilt");
    fs::write(&binary, "prebuilt executable fixture\n").unwrap();
    binary
}

fn bundle(directory: &Path, binary: &Path, format: &str, target: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-bundle"))
        .args([
            "bundle",
            "--format",
            format,
            "--target",
            target,
            "--binary-path",
        ])
        .arg(binary)
        .current_dir(directory)
        .output()
        .unwrap()
}

#[test]
fn ipa_cli_preserves_permissions_and_previous_archive_on_failure() {
    let temp = tempfile::tempdir().unwrap();
    let binary = project(temp.path());
    let output = bundle(temp.path(), &binary, "ipa", "aarch64-apple-ios");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let ipa_path = temp
        .path()
        .join("target/aarch64-apple-ios/debug/bundle/ipa/Post.Card.ipa");
    let original = fs::read(&ipa_path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&ipa_path).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&original)).unwrap();
    assert_eq!(
        archive
            .by_name("Payload/Post.Card.app/postcard")
            .unwrap()
            .unix_mode()
            .unwrap()
            & 0o777,
        0o755
    );
    assert!(
        archive
            .by_name("Payload/Post.Card.app/fr.lproj/InfoPlist.strings")
            .is_ok()
    );
    let entries = fs::read_dir(ipa_path.parent().unwrap()).unwrap().count();
    assert_eq!(entries, 1, "temporary staging and ZIP files were removed");

    let repeated = bundle(temp.path(), &binary, "ipa", "aarch64-apple-ios");
    assert!(repeated.status.success());
    assert_eq!(
        fs::read(&ipa_path).unwrap(),
        original,
        "unchanged unsigned input produces the same archive"
    );

    let manifest = temp.path().join("Cargo.toml");
    let contents = fs::read_to_string(&manifest).unwrap().replace(
        "[package.metadata.bundle.ios]\n",
        "[package.metadata.bundle.ios]\napple_provisioning_profile = \"device.mobileprovision\"\n",
    );
    fs::write(manifest, contents).unwrap();
    let output = bundle(temp.path(), &binary, "ipa", "aarch64-apple-ios");
    assert!(!output.status.success());
    assert_eq!(fs::read(ipa_path).unwrap(), original);
}

#[test]
fn apple_bundles_embed_original_profile_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let binary = project(temp.path());
    let profile = include_bytes!("fixtures/apple/fastlane-test.mobileprovision");
    fs::write(temp.path().join("device.mobileprovision"), profile).unwrap();
    let manifest = temp.path().join("Cargo.toml");
    let contents = fs::read_to_string(&manifest).unwrap().replace(
        "identifier = \"com.example.postcard\"",
        "identifier = \"com.example.postcard\"\napple_provisioning_profile = \"device.mobileprovision\"",
    );
    fs::write(manifest, contents).unwrap();
    for (format, target, embedded) in [
        ("ios", "aarch64-apple-ios", "embedded.mobileprovision"),
        (
            "osx",
            "aarch64-apple-darwin",
            "Contents/embedded.provisionprofile",
        ),
    ] {
        let output = bundle(temp.path(), &binary, format, target);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let path = temp
            .path()
            .join("target")
            .join(target)
            .join("debug/bundle")
            .join(format)
            .join("Post.Card.app")
            .join(embedded);
        assert_eq!(fs::read(path).unwrap(), profile);
    }
    // Profiles are opaque inputs: cargo-bundle does not attempt to authenticate them.
    let opaque = b"arbitrary profile bytes\0\xff";
    fs::write(temp.path().join("device.mobileprovision"), opaque).unwrap();
    let output = bundle(temp.path(), &binary, "ipa", "aarch64-apple-ios");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let archive_path = temp
        .path()
        .join("target/aarch64-apple-ios/debug/bundle/ipa/Post.Card.ipa");
    let mut archive = zip::ZipArchive::new(fs::File::open(archive_path).unwrap()).unwrap();
    let mut embedded = Vec::new();
    archive
        .by_name("Payload/Post.Card.app/embedded.mobileprovision")
        .unwrap()
        .read_to_end(&mut embedded)
        .unwrap();
    assert_eq!(embedded, opaque);
}

#[test]
fn ipa_rejects_simulator_and_non_ios_targets() {
    let temp = tempfile::tempdir().unwrap();
    let binary = project(temp.path());
    for target in [
        "aarch64-apple-ios-sim",
        "x86_64-apple-ios",
        "aarch64-apple-ios-macabi",
        "aarch64-apple-darwin",
    ] {
        let output = bundle(temp.path(), &binary, "ipa", target);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("requires an iOS device target"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn unsafe_app_names_fail_before_removing_existing_output() {
    let temp = tempfile::tempdir().unwrap();
    let binary = project(temp.path());
    let manifest = temp.path().join("Cargo.toml");
    fs::write(
        &manifest,
        fs::read_to_string(&manifest)
            .unwrap()
            .replace("Post.Card", "../victim"),
    )
    .unwrap();
    for (format, target) in [
        ("ios", "aarch64-apple-ios"),
        ("ipa", "aarch64-apple-ios"),
        ("osx", "aarch64-apple-darwin"),
        ("dmg", "aarch64-apple-darwin"),
    ] {
        let victim = temp
            .path()
            .join("target")
            .join(target)
            .join("debug/bundle/victim.app");
        fs::create_dir_all(&victim).unwrap();
        fs::write(victim.join("keep"), "existing output").unwrap();
        let output = bundle(temp.path(), &binary, format, target);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("single filename"));
        assert_eq!(
            fs::read_to_string(victim.join("keep")).unwrap(),
            "existing output"
        );
    }
}

#[test]
fn ipa_preserves_display_names_that_need_xml_escaping() {
    let temp = tempfile::tempdir().unwrap();
    let binary = project(temp.path());
    let manifest = temp.path().join("Cargo.toml");
    fs::write(
        &manifest,
        fs::read_to_string(&manifest)
            .unwrap()
            .replace("Post.Card", "Post.Card & Co"),
    )
    .unwrap();
    let output = bundle(temp.path(), &binary, "ipa", "aarch64-apple-ios");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let file = fs::File::open(
        temp.path()
            .join("target/aarch64-apple-ios/debug/bundle/ipa/Post.Card & Co.ipa"),
    )
    .unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    let info = plist::Value::from_reader(std::io::Cursor::new({
        let mut bytes = Vec::new();
        archive
            .by_name("Payload/Post.Card & Co.app/Info.plist")
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    }))
    .unwrap();
    assert_eq!(
        info.as_dictionary()
            .unwrap()
            .get("CFBundleName")
            .unwrap()
            .as_string(),
        Some("Post.Card & Co")
    );
}

#[test]
fn ipa_signing_seals_the_app_before_archiving() {
    let temp = tempfile::tempdir().unwrap();
    let binary = project(temp.path());
    let macho = apple_codesign::macho_builder::MachOBuilder::new_aarch64(2)
        .macho_target(apple_codesign::MachoTarget {
            platform: apple_codesign::Platform::IOs,
            minimum_os_version: "17.0.0".parse().unwrap(),
            sdk_version: "17.0.0".parse().unwrap(),
        })
        .write_macho()
        .unwrap();
    fs::write(&binary, macho).unwrap();
    signing_identity(&temp.path().join("identity.p12"));
    fs::write(
        temp.path().join("device.mobileprovision"),
        b"opaque profile",
    )
    .unwrap();
    let mut requested = plist::Dictionary::new();
    requested.insert("get-task-allow".into(), plist::Value::Boolean(true));
    plist::Value::Dictionary(requested.clone())
        .to_file_xml(temp.path().join("entitlements.plist"))
        .unwrap();
    let manifest = temp.path().join("Cargo.toml");
    let contents = fs::read_to_string(&manifest).unwrap().replace(
        "[package.metadata.bundle.ios]\n",
        "[package.metadata.bundle.ios]\napple_signing_p12 = \"identity.p12\"\napple_provisioning_profile = \"device.mobileprovision\"\napple_signing_entitlements = \"entitlements.plist\"\n",
    );
    fs::write(manifest, contents).unwrap();
    let output = bundle(temp.path(), &binary, "ipa", "aarch64-apple-ios");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let file = fs::File::open(
        temp.path()
            .join("target/aarch64-apple-ios/debug/bundle/ipa/Post.Card.ipa"),
    )
    .unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    let mut embedded = Vec::new();
    archive
        .by_name("Payload/Post.Card.app/embedded.mobileprovision")
        .unwrap()
        .read_to_end(&mut embedded)
        .unwrap();
    assert_eq!(embedded, b"opaque profile");
    let mut resources = Vec::new();
    archive
        .by_name("Payload/Post.Card.app/_CodeSignature/CodeResources")
        .unwrap()
        .read_to_end(&mut resources)
        .unwrap();
    let resources = plist::Value::from_reader(std::io::Cursor::new(resources)).unwrap();
    assert!(resources.as_dictionary().unwrap().contains_key("files2"));
    let mut executable = Vec::new();
    archive
        .by_name("Payload/Post.Card.app/postcard")
        .unwrap()
        .read_to_end(&mut executable)
        .unwrap();
    let macho = apple_codesign::MachOBinary::parse(&executable).unwrap();
    let signature = macho.code_signature().unwrap().unwrap();
    let signed = signature.entitlements().unwrap().unwrap();
    let signed = plist::Value::from_reader_xml(signed.as_str().as_bytes()).unwrap();
    assert_eq!(signed, plist::Value::Dictionary(requested));
    let problems = apple_codesign::verify_macho_data(&executable);
    assert!(
        problems.is_empty(),
        "Mach-O signature problems: {problems:?}"
    );
}

#[test]
fn ios_rejects_invalid_identifiers_before_replacing_output() {
    let temp = tempfile::tempdir().unwrap();
    let binary = project(temp.path());
    let manifest = temp.path().join("Cargo.toml");
    let original = fs::read_to_string(&manifest).unwrap();
    let app = temp
        .path()
        .join("target/aarch64-apple-ios/debug/bundle/ios/Post.Card.app");
    fs::create_dir_all(&app).unwrap();
    fs::write(app.join("keep"), b"previous output").unwrap();
    for identifier in [
        None,
        Some(""),
        Some("com..example"),
        Some("com.example app"),
        Some("com.example_foo"),
        Some("com.example.*"),
    ] {
        let replacement = identifier
            .map(|value| format!("identifier = {value:?}"))
            .unwrap_or_default();
        fs::write(
            &manifest,
            original.replace("identifier = \"com.example.postcard\"", &replacement),
        )
        .unwrap();
        for format in ["ios", "ipa"] {
            let output = bundle(temp.path(), &binary, format, "aarch64-apple-ios");
            assert!(!output.status.success(), "accepted {identifier:?}");
            assert!(String::from_utf8_lossy(&output.stderr).contains("bundle identifier"));
        }
        assert_eq!(fs::read(app.join("keep")).unwrap(), b"previous output");
    }
}

#[test]
fn ios_preserves_previous_app_when_resource_or_signing_fails() {
    let temp = tempfile::tempdir().unwrap();
    let binary = project(temp.path());
    let manifest = temp.path().join("Cargo.toml");
    let original = fs::read_to_string(&manifest).unwrap();
    let app = temp
        .path()
        .join("target/aarch64-apple-ios/debug/bundle/ios/Post.Card.app");
    let output = bundle(temp.path(), &binary, "ios", "aarch64-apple-ios");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let info = fs::read(app.join("Info.plist")).unwrap();
    let plist = plist::Value::from_reader(std::io::Cursor::new(&info)).unwrap();
    assert!(
        !plist
            .as_dictionary()
            .unwrap()
            .contains_key("UILaunchStoryboardName")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(app.join("postcard"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }
    fs::write(app.join("keep"), b"previous output").unwrap();
    fs::write(temp.path().join("broken.svg"), "invalid SVG").unwrap();
    for setting in [
        "icon = [\"broken.svg\"]",
        "apple_signing_p12 = \"missing.p12\"",
        "apple_provisioning_profile = \"missing.mobileprovision\"",
    ] {
        fs::write(
            &manifest,
            original.replace(
                "[package.metadata.bundle.ios]",
                &format!("[package.metadata.bundle.ios]\n{setting}"),
            ),
        )
        .unwrap();
        let output = bundle(temp.path(), &binary, "ios", "aarch64-apple-ios");
        assert!(!output.status.success());
        assert_eq!(fs::read(app.join("Info.plist")).unwrap(), info);
        assert_eq!(fs::read(app.join("keep")).unwrap(), b"previous output");
        assert_eq!(fs::read_dir(app.parent().unwrap()).unwrap().count(), 1);
    }
}

#[test]
fn ios_rejects_non_ios_target() {
    let temp = tempfile::tempdir().unwrap();
    let binary = project(temp.path());
    let output = bundle(temp.path(), &binary, "ios", "aarch64-apple-darwin");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires an iOS target"));
}
