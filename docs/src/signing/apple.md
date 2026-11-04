# Apple signing and distribution

Cargo-bundle signs macOS apps, disk images, and experimental iOS apps with
the Rust `apple-codesign` library. It does not invoke `codesign`.

## Sign an app

Supply a PKCS#12 certificate and private key. Keep its password in an
environment variable:

```toml
[package.metadata.bundle]
name = "Example"
identifier = "com.example.app"
apple_signing_p12 = "certs/developer-id.p12"
apple_signing_password_env = "APPLE_SIGNING_PASSWORD"
apple_signing_timestamp_url = "http://timestamp.apple.com/ts01"
apple_signing_hardened_runtime = true
# Optional XML or binary entitlements plist:
# apple_signing_entitlements = "packaging/Example.entitlements"
```

Without a password setting, cargo-bundle tries an empty password. A configured
password variable must exist. Paths are relative to your working directory.

```sh
export APPLE_SIGNING_PASSWORD='value supplied by your secret store'
cargo bundle --release --format osx
```

The signer loads the certificate and key, chains Apple certificates, derives
the team ID, and signs the app in place. A configured timestamp URL requests a
timestamp. The hardened-runtime flag and supplied entitlements apply to the
main signing scope; entitlements do not grant permissions by themselves.

For a disk image, use `--format dmg` on macOS. Cargo-bundle signs both the
app inside it and the final image.

## Notarize and staple on macOS

Enable the optional Rust notarization backend when installing from this checkout:

```sh
cargo install --path . --force --features apple-notarization
```

Alongside the signing settings above, configure an App Store Connect API-key
file and a processing timeout in seconds:

```toml
[package.metadata.bundle]
apple_notarization_api_key = "secrets/notary-key.json"
apple_notarization_timeout = 600
```

The key file uses `apple-codesign`'s JSON format. Its
[setup guide](https://gregoryszorc.com/docs/apple-codesign/0.29.0/apple_codesign_getting_started.html#obtaining-an-app-store-connect-api-key)
shows how to create it from your issuer ID, key ID, and downloaded private
key. Keep it out of version control.

With these settings, `cargo bundle --release --format osx` signs the app,
uploads it to Apple's notary service, waits for acceptance, and staples the
ticket. For `dmg`, it submits and staples the final disk image. Authentication
does not prompt. Submission needs network access and an authorized Apple
developer account.

The timeout defaults to 600 seconds and must be greater than zero. A rejected
submission, timeout, or failed stapling step fails the command. Read the
reported error before retrying; a timed-out submission may still be processing
at Apple. The timeout bounds processing waits, not all network operations.

Notarization requires a signing certificate, hardened runtime, and timestamp
URL in the configuration. Use an appropriate Developer ID identity and
entitlements for your app. Apple's
[notarization guide](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)
covers its acceptance requirements.

## Check the result

On macOS, inspect the distributed app with:

```sh
codesign --verify --deep --strict --verbose=2 target/release/bundle/osx/Example.app
codesign --display --entitlements :- target/release/bundle/osx/Example.app
spctl --assess --type execute --verbose=2 target/release/bundle/osx/Example.app
```

Use the printed output path if yours differs. For a disk image, also run
the fstool UDIF checksum verifier on the final DMG.

## Provisioning profiles

Set `apple_provisioning_profile` to an existing Apple-issued profile. It is
embedded before signing: `embedded.mobileprovision` for iOS, or
`Contents/embedded.provisionprofile` for macOS. Paths are relative to the
working directory, and platform tables can override the setting.

Cargo-bundle embeds the supplied profile bytes unchanged. It does not check
the profile's signature, expiration, signing identity, platform, or entitlement
authorization. Supply a profile appropriate for your app, signing certificate,
platform, and distribution channel. Apple validates provisioning when the app
is installed or distributed.

For signing, provide an XML or binary plist dictionary with the concrete claims your
app needs, including any required app and team identifiers. Cargo-bundle does
not derive or expand those claims from the profile. See Apple's
[profile reference](https://developer.apple.com/documentation/technotes/tn3125-inside-code-signing-provisioning-profiles)
for platform rules; macOS apps without restricted entitlements generally do not
need a profile, except for TestFlight distribution.

The `plist` crate reads the dictionary and `apple-codesign` encodes the signing
entitlements. Cargo-bundle preserves explicitly supplied values, including
arrays and nested dictionaries; it does not infer grants or expand placeholders.

The [`ipa` format](../platforms/ios.md#create-an-ipa) archives the iOS app after
embedding and signing. Notarization is for macOS; it is rejected for both
`ios` and `ipa`. Creating an IPA does not submit it to App Store Connect.
