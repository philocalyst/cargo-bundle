# Command line

Run `cargo bundle` from a package or workspace directory. It builds the
selected binary or example and packages it. Choose a format with
`--format`, or leave that option out to use the default for the target
platform. `cargo bundle --help` lists the options supported by your install.

## Options

| Option | Meaning |
| --- | --- |
| `-b, --bin NAME` | Bundle a named binary target. |
| `-e, --example NAME` | Bundle a named example target. Cannot be combined with `--bin`. |
| `-f, --format FORMAT` | Produce one format: `deb`, `ios`, `ipa`, `msi`, `wxsmsi`, `osx`, `dmg`, `rpm`, `appimage`, or `exe`. |
| `-r, --release` | Use the release profile. Conflicts with `--profile`. |
| `--profile NAME` | Use a Cargo profile by name. The name `debug` is reserved; the default development profile is selected by omitting this option. |
| `--binary-path PATH` | Package an existing executable without building it. Relative paths are read from the current directory; the file must exist. |
| `-t, --target TRIPLE` | Build for a target triple. Repeat to create a macOS universal binary with `lipo`. |
| `--features FEATURES` | Enable the given Cargo feature list, such as `--features "gui,media"`. |
| `--all-features` | Enable all crate features. |
| `--no-default-features` | Disable default crate features. |
| `-p, --package NAME` | Select a package by exact name from the metadata for the current directory's `Cargo.toml`. Without it, cargo-bundle selects that metadata command's root package. |
| `-h, --help` | Print help. |
| `-V, --version` | Print the cargo-bundle version. |

Use `--bin` or `--example` to choose a target and its matching manifest
settings table. Without either option, cargo-bundle uses the first binary
target in the selected package. It errors if the package has no binary target;
it does not use Cargo's `default-run`. See [Binaries and examples](manifest.md#binaries-and-examples)
to configure a particular target.

Without `--format`, cargo-bundle produces a macOS app, Debian package, Windows
executable, or iOS app, according to the first `--target` triple or, if absent,
the host OS. Use `--format` for another output such as a DMG, AppImage, or MSI.
See [Choose a format](formats.md) for backend requirements and status.

## Package an existing executable

Build with your own Cargo options, then pass the resulting file directly:

```sh
cargo build --release --locked
cargo bundle --release --binary-path target/release/my-app
```

Replace `my-app` with your binary name (including `.exe` on Windows).
`--binary-path` skips compilation and universal-binary combination. It still
reads Cargo metadata and uses `--bin` or `--example` to select bundle settings;
it does not infer the target architecture from the file. Pass `--target` when
packaging a cross-compiled executable. The selected profile determines the
output directory even when the input file is elsewhere.

## Targets and output directories

With the default Cargo target directory, one `--target` puts the binary under
`target/<triple>/<profile-dir>`. Without `--target`, it goes under
`target/<profile-dir>`. When Cargo metadata resolves the target directory, it
supplies the base path, including when `target-dir` or `CARGO_TARGET_DIR`
changes it. For multiple targets, cargo-bundle builds each one and uses
`lipo` to combine them under
`target/universal/<profile-dir>`; this requires macOS and `lipo`. Example
binaries add an `examples` directory after the profile directory.

The default development profile and `--profile dev` both use the `debug`
directory. `--release` uses `release`; other profile names use a directory
with that name. `--profile debug` is rejected because `debug` is reserved for
the default development profile.

Pass `--target` to cargo-bundle when you need a target-triple subdirectory.
If Cargo gets the target only from its configuration or environment,
cargo-bundle may look in a different directory than Cargo used.

## Workspace packages

Use `--package` with the exact package name; cargo-bundle does not accept
Cargo package specs such as `name@version`. Without it, cargo-bundle selects
the root package reported for the current directory's `Cargo.toml`. A virtual
workspace root has no root package, so name a member with `--package`.

`--package` selects both the manifest metadata and the package passed to
`cargo build`. Resource and icon paths still resolve from the working
directory. If a build needs additional Cargo options, build it first and use
`--binary-path` to package the result.

## Environment variables

| Variable | Effect |
| --- | --- |
| `CARGO` | Path to the Cargo executable used for metadata and builds; defaults to `cargo` on `PATH`. |
| `CARGO_BUNDLE_SKIP_BUILD` | If set to any value, skips `cargo build` (and universal-binary combination). The expected binary must already exist at the path for the selected target and profile. |
| `CARGO_TARGET_DIR` | Cargo may use this as the target directory; cargo-bundle reads the resolved directory from `cargo metadata`. |

Even `CARGO_BUNDLE_SKIP_BUILD=0` skips the build, because cargo-bundle checks
whether the variable is set, not its value. Unset it to have cargo-bundle run
`cargo build` again.

Options such as `--manifest-path`, `--locked`, `--offline`, `--workspace`, and
`--out-dir` belong to Cargo. When a build needs one of them, run `cargo build`
with that option first, then set `CARGO_BUNDLE_SKIP_BUILD` when packaging.
cargo-bundle skips its build and expects the binary at the selected target and
profile path.

For other common selections:

```sh
cargo bundle --release --format appimage
cargo bundle --bin editor --features "gui,wayland" --format deb
cargo bundle --example hello --format osx
cargo bundle -t aarch64-apple-darwin -t x86_64-apple-darwin --format osx
```
