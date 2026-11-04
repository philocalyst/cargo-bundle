#![cfg(target_os = "macos")]

use std::{fs, process::Command};

#[test]
fn signed_macos_app_preserves_xml_and_binary_claims_and_runs() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("src")).unwrap();
    fs::write(
        temp.path().join("src/main.rs"),
        "fn main() { println!(\"signed macOS app\"); }\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "signed-app"
version = "1.0.0"
edition = "2024"
[package.metadata.bundle]
name = "Signed App"
identifier = "com.example.signed-app"
apple_signing_p12 = "identity.p12"
apple_signing_hardened_runtime = true
apple_signing_entitlements = "claims.plist"
"#,
    )
    .unwrap();
    let binary = temp.path().join("prebuilt");
    let compiled = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg("--crate-name=signed_app")
        .arg(temp.path().join("src/main.rs"))
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let (certificate, key) = apple_codesign::create_self_signed_code_signing_certificate(
        x509_certificate::KeyAlgorithm::Ecdsa(x509_certificate::EcdsaCurve::Secp256r1),
        apple_codesign::CertificateProfile::AppleDevelopment,
        "TESTTEAM01",
        "macOS packaging test",
        "US",
        chrono::Duration::hours(1),
    )
    .unwrap();
    let identity = p12::PFX::new(
        certificate.constructed_data(),
        &key.to_pkcs8_one_asymmetric_key_der(),
        None,
        "",
        "test",
    )
    .unwrap();
    fs::write(temp.path().join("identity.p12"), identity.to_der()).unwrap();
    let claims = plist::Value::Dictionary(plist::Dictionary::from_iter([(
        "com.apple.security.network.client",
        plist::Value::Boolean(true),
    )]));
    for binary_plist in [false, true] {
        let path = temp.path().join("claims.plist");
        if binary_plist {
            claims.to_file_binary(&path).unwrap();
        } else {
            claims.to_file_xml(&path).unwrap();
        }
        let built = Command::new(env!("CARGO_BIN_EXE_cargo-bundle"))
            .args(["bundle", "--format", "osx", "--binary-path"])
            .arg(&binary)
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let app = temp.path().join("target/debug/bundle/osx/Signed App.app");
        assert!(!app.join("Contents/embedded.provisionprofile").exists());
        let executable = app.join("Contents/MacOS/signed-app");
        let bytes = fs::read(&executable).unwrap();
        let macho = apple_codesign::MachOBinary::parse(&bytes).unwrap();
        let signature = macho.code_signature().unwrap().unwrap();
        let signed_claims = signature.entitlements().unwrap().unwrap();
        assert_eq!(
            plist::Value::from_reader_xml(signed_claims.as_str().as_bytes()).unwrap(),
            claims
        );
        assert!(apple_codesign::verify_macho_data(&bytes).is_empty());
        let verified = Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(&app)
            .output()
            .unwrap();
        assert!(
            verified.status.success(),
            "{}",
            String::from_utf8_lossy(&verified.stderr)
        );
        let ran = Command::new(&executable).output().unwrap();
        assert!(
            ran.status.success(),
            "signed executable failed: {:?}: {}",
            ran.status,
            String::from_utf8_lossy(&ran.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&ran.stdout).trim(),
            "signed macOS app"
        );
    }
}
