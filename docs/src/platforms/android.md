# Android

`cargo bundle --format apk --target aarch64-linux-android --release` packages
an Android NativeActivity shared library. Build with the Android NDK linker
configured for your Rust target, or supply an existing `.so` with
`--binary-path`. The library must export `ANativeActivity_onCreate`.

```toml
[lib]
crate-type = ["cdylib"]

[package.metadata.bundle]
name = "Postcard"
identifier = "com.example.postcard"
resources = ["assets/*"]

[package.metadata.bundle.android]
version_code = 1
min_sdk = 24
target_sdk = 35
permissions = ["android.permission.INTERNET"]
signing_key = "release.pem"
```

Output is `<target>/<profile>/bundle/apk/Postcard.apk`. `rust-apk` compiles
binary Android XML and resources, checks the library ABI and entry point,
aligns native libraries, and writes the APK atomically. Supported Rust
architectures are aarch64, arm, x86 and x86_64. Each invocation packages one ABI.

`version_code` defaults to 1; the version name comes from the bundle/package
version. Without explicit SDK settings, apk-rs supplies its defaults (minimum
24, target 36). `permissions` adds manifest permissions. `manifest` can name a
complete AndroidManifest.xml relative to the selected package manifest; the
builder still applies the label, version and explicit SDK overrides.

`signing_key` is a PEM file containing the private key and certificate. Without
it the APK remains unsigned and must be signed before installation. General
resources become files under `assets/`; the first configured icon is decoded
and written as a PNG launcher icon. Package identifiers, native input and
resource paths are validated by apk-rs. A failed build preserves an existing APK.
