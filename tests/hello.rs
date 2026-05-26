mod common;

use std::process::Command;

#[test]
#[cfg(target_os = "macos")]
fn osx() {
    use std::fs;
    common::setup_example_binary("hello");

    let root = common::project_root();
    let output = Command::new(common::cargo_bundle_bin())
        .args(["bundle", "--example", "hello", "--format", "osx"])
        .current_dir(&root)
        .env("CARGO_BUNDLE_SKIP_BUILD", "1")
        .output()
        .expect("Failed to execute cargo-bundle");

    assert!(
        output.status.success(),
        "cargo-bundle failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let app_path = root.join("target/debug/examples/bundle/osx/hello.app");
    assert!(app_path.exists(), "App bundle not found at {:?}", app_path);

    // SVG icon should have been converted to ICNS.
    let icns_path = app_path.join("Contents/Resources/hello.icns");
    assert!(icns_path.exists(), "ICNS not found at {:?}", icns_path);
    assert!(
        fs::metadata(&icns_path).unwrap().len() > 0,
        "ICNS file is empty"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn deb() {
    common::setup_example_binary("hello");

    let root = common::project_root();
    let output = Command::new(common::cargo_bundle_bin())
        .args(["bundle", "--example", "hello", "--format", "deb"])
        .current_dir(&root)
        .env("CARGO_BUNDLE_SKIP_BUILD", "1")
        .output()
        .expect("Failed to execute cargo-bundle");

    assert!(
        output.status.success(),
        "cargo-bundle failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let arch = if cfg!(target_arch = "x86_64") {
        "amd64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };

    let deb_path = root.join(format!(
        "target/debug/examples/bundle/deb/hello_0.9.1_{arch}.deb"
    ));
    assert!(
        deb_path.exists(),
        "Debian package not found at {:?}",
        deb_path
    );

    // SVG should be copied to the scalable hicolor directory.
    let svg_path = root.join(format!(
        "target/debug/examples/bundle/deb/hello_0.9.1_{arch}/data/usr/share/icons/hicolor/scalable/apps/hello.svg"
    ));
    assert!(
        svg_path.exists(),
        "SVG icon not found in deb data at {:?}",
        svg_path
    );
}
