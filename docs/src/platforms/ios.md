# iOS

Use `--format ios` for an iOS `.app` directory or `--format ipa` to archive it.
Support is experimental. Compiling and linking device executables require
[the iOS SDK from Xcode](https://doc.rust-lang.org/rustc/platform-support/apple-ios.html#requirements);
installing a Rust target does not supply the SDK. Set `SDKROOT` to select an
existing SDK without launching Xcode. Packaging and signing a prebuilt iOS
executable with `--binary-path` do not need the SDK or Xcode tools.

## Configure the app

The iOS writer uses the [shared bundle settings](../manifest.md). For example, in `Cargo.toml`:

```toml
[package.metadata.bundle]
name = "Example"
identifier = "com.example.app"
version = "1.2.0"
icon = ["assets/icon.svg"]
resources = ["assets/data"]
```

Set a stable reverse-DNS `identifier`. iOS packaging rejects empty identifiers,
empty dot-separated components, and characters other than ASCII letters,
digits, hyphens, and dots.

## Build and inspect the bundle

With that configuration in place, build for an ARM64 iOS device:

```sh
rustup target add aarch64-apple-ios
cargo bundle --format ios --release --target aarch64-apple-ios
```

Cargo-bundle prints the app's path when it finishes. For this example, inspect the files and check the plist on macOS with:

```sh
app_path=target/aarch64-apple-ios/release/bundle/ios/Example.app
find "$app_path" -type f -print
plutil -lint "$app_path/Info.plist"
```

The app contains the executable, `Info.plist`, resources, and generated PNG icons. The resource `assets/data/config.json` ends up at `Example.app/assets/data/config.json`; the program must locate it itself. See [Resource paths](../resources.md#where-files-go) for other path mappings.

Bundles go below the selected target and profile directory in `bundle/ios/`. Bundling an example adds `examples/` before `bundle`. If you change the Cargo target directory, use the printed output path for `app_path` above; [Command line](../cli.md#targets-and-output-directories) explains how that path is chosen.

A failed packaging or signing attempt preserves the previous output. The
main executable is marked executable in both `.app` and `.ipa` output.

The plist uses the bundle name, identifier, and executable name. It writes the configured `version`, or Cargo's package version when omitted, to both `CFBundleVersion` and `CFBundleShortVersionString`. These other entries are generated as follows:

| Plist key | Value |
| --- | --- |
| `CFBundleDevelopmentRegion` | `en_US` |
| `LSRequiresIPhoneOS` | `true` |
| `CFBundlePackageType` | `APPL` |
| `CFBundleIconFiles` | Generated icon filenames; omitted when no icons were generated |

The first configured SVG supplies a complete set of standard iPhone, iPad,
and 1024-pixel App Store PNG icons. Without an SVG, PNG inputs take precedence
for duplicate sizes; ICNS and other raster inputs are converted to PNG.
See [Resources and icons](../resources.md#icon-files) for the shared conversion
rules. Inspect the generated files before distribution. macOS-specific
framework settings do not affect iOS output.

## Localized plist strings

Use `[package.metadata.bundle.ios.localizations.fr]` to supply French plist
strings, or keep translations in [separate TOML files](../accessibility-localization.md#keep-translations-in-separate-files).
Each locale creates `<locale>.lproj/InfoPlist.strings` at the app bundle root:

```toml
[package.metadata.bundle.ios.localizations.fr]
CFBundleDisplayName = "Exemple"
```

The iOS and macOS writers use the same escaping and file format. Their
configuration tables remain separate, so each platform can have its own name.
Application UI translations must be supplied by the app and its resources.

## Signing and distribution

Supply your certificate and entitlements to package a signed app
without prompts:

```toml
[package.metadata.bundle.ios]
apple_signing_p12 = "secrets/distribution.p12"
apple_signing_password_env = "APPLE_SIGNING_PASSWORD"
apple_signing_entitlements = "packaging/App.entitlements"
```

Certificate and entitlement paths are relative to the working directory.
Supply concrete identity and capability entitlements for your distribution
workflow; cargo-bundle passes the requested values to `apple-codesign`.

## Create an IPA

```sh
cargo bundle --format ipa --release --target aarch64-apple-ios
```

The Rust ZIP writer places the finished app at `Payload/Example.app/` inside
`target/aarch64-apple-ios/release/bundle/ipa/Example.ipa`. It preserves
executable permissions and internal symlinks, and needs no `zip` or Xcode export command.
Use the printed path for the actual output.

IPA packaging requires an iOS device target; simulator targets are rejected.
Without signing settings, it creates an unsigned archive for inspection.
Cargo-bundle does not compile storyboards or asset catalogs, configure a launch
screen, or provision app extensions. Supply compiled application resources
through `resources` where applicable. Physical-device installation and App
Store acceptance have not been verified end to end.

Apple device installation and distribution still require authorized signing
and provisioning. Cargo-bundle does not create developer-account assets or
submit the IPA to App Store Connect.
