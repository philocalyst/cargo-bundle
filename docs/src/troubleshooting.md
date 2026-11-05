# Troubleshooting

| Message or symptom | Cause and fix |
| --- | --- |
| `The bundle name "..." must be a single file name` | `name` contains `/`, `\`, or is `.` or `..`. |
| `unknown field` when reading `Cargo.toml` | A key is misspelled or in the wrong table; unknown keys are errors. See [Manifest reference](manifest.md). |
| `No [package.metadata.bundle.bin.<name>] section` | `--bin` or `--example` tables are read instead of the main one; add the table. |
| `IPA packaging requires an iOS device target` | Use `--target aarch64-apple-ios`. |
| `iOS needs a bundle identifier` | Set `identifier`, using letters, digits and hyphens. |
| `Android bundling needs a cdylib` | Pass `--binary-path <dir>/lib<name>.so`. |
| `The APK is unsigned and cannot be installed` | Set `android.signing_key`. |
| `Invalid android.min_sdk ...` | The error names the setting that holds the bad value. |
| `Invalid locale code` / `both mean` | See [Locale codes](localization.md#locale-codes). |
| `Apple notarization requires ...` | Add the certificate, hardened runtime and timestamp. |
| `ELF machine type ... does not match` | `appimage_runtime` is for another CPU than `--target`. |
| `Failed to build the AppImage launcher` | Install the target's standard library (`rustup target add ...`) and linker, as for building the app itself, or set `appimage_runtime`. |
| `RPM packages are not implemented yet` | Use `deb` or `appimage`. |
| `built without the windows-signing feature` | Install with `--features windows-signing`. |
| `--format` value rejected | The error lists the valid formats; see [Formats](formats.md). |
| Bundle step builds the wrong package in a workspace | Pass `--package <name>`; it is forwarded to `cargo build`. |
| Missing resource | A glob that matches nothing adds nothing; check the pattern and run from the package directory. |
