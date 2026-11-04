# Building release artifacts in CI

Use a native runner for each package format. This workflow builds the
repository's `goodbye` example as a macOS app, Debian package, and Windows
executable, uploads each result, then attaches them to a GitHub Release for
tag pushes. A manual run builds and uploads artifacts without publishing a
release.

```yaml
name: Build packages

on:
  push:
    tags: ["v*.*.*"]
  workflow_dispatch:

permissions:
  contents: read

jobs:
  package:
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: macos-latest
            format: osx
            artifact: macos-app
            path: target/release/examples/bundle/osx/Goodbye.app.tar.gz
          - os: ubuntu-latest
            format: deb
            artifact: linux-deb
            path: target/release/examples/bundle/deb/*.deb
          - os: windows-latest
            format: exe
            artifact: windows-exe
            path: target/release/examples/bundle/exe/goodbye.exe
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable

      - name: Install cargo-bundle from this checkout
        run: cargo install --path . --force

      - name: Build package
        run: cargo bundle --example goodbye --release --format ${{ matrix.format }}

      - name: Archive macOS app bundle
        if: matrix.format == 'osx'
        run: tar -czf target/release/examples/bundle/osx/Goodbye.app.tar.gz -C target/release/examples/bundle/osx Goodbye.app

      - name: Upload package
        uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.artifact }}
          path: ${{ matrix.path }}
          if-no-files-found: error

  publish:
    if: github.event_name == 'push' && startsWith(github.ref, 'refs/tags/')
    needs: package
    runs-on: ubuntu-latest
    permissions:
      contents: write
    steps:
      - uses: actions/download-artifact@v4
        with:
          pattern: '*'
          path: dist
          merge-multiple: true

      - name: Attach artifacts to the tagged release
        env:
          GH_TOKEN: ${{ github.token }}
          GH_REPO: ${{ github.repository }}
        run: gh release create "$GITHUB_REF_NAME" dist/* --verify-tag --title "$GITHUB_REF_NAME" --generate-notes
```

The install uses this checkout. This repository does not commit `Cargo.lock`
and ignores it in `.gitignore`, so Cargo resolves dependencies during
installation. For reproducible dependency selection, pin the Rust toolchain
in `rust-toolchain.toml`, stop ignoring and commit `Cargo.lock`, and add `--locked` to the
install command. Pin GitHub Actions to reviewed full commit SHAs in your own
release workflow as well.

## Adapt the workflow

The checkout and commands run from the repository root, where the package's
`Cargo.toml` lives. In a workspace, cargo-bundle reads metadata for
`--package <name>` but does not pass that option to `cargo build`. Run from a
directory where Cargo's default build selection includes the package you want,
or build it separately and set `CARGO_BUNDLE_SKIP_BUILD` for packaging. See
[Workspace packages](cli.md#workspace-packages) for the details. Bundle options
for features, targets, and profiles are forwarded to `cargo build`.

Cargo-bundle writes these files and directories:

| Output | Path |
| --- | --- |
| macOS app | `target/release/examples/bundle/osx/Goodbye.app` |
| Debian package | `target/release/examples/bundle/deb/*.deb` |
| Windows executable | `target/release/examples/bundle/exe/goodbye.exe` |

The macOS job archives the app before upload because artifact upload does not
preserve the bundle's executable bits and symlinks as a native directory. The
Windows job uses a Windows runner because cargo-bundle embeds icon and version
resources only when it runs on Windows.

These paths assume Cargo's default `target` directory and the release profile.
With `--target`, Cargo adds the triple before the profile directory; multiple
macOS targets use `universal`. Examples add `examples` after the profile
directory. A configured `target-dir` or `CARGO_TARGET_DIR`, or a different
profile, changes the path. Set the upload paths to match. See
[Targets and output directories](cli.md#targets-and-output-directories).

If you build a binary in a separate step, set `CARGO_BUNDLE_SKIP_BUILD` to
prevent cargo-bundle from building again. On macOS:

```sh
cargo build --example goodbye --release
CARGO_BUNDLE_SKIP_BUILD=1 cargo bundle --example goodbye --release --format osx
```

In PowerShell:

```powershell
cargo build --example goodbye --release
$env:CARGO_BUNDLE_SKIP_BUILD = "1"
cargo bundle --example goodbye --release --format exe
Remove-Item Env:CARGO_BUNDLE_SKIP_BUILD
```

The prebuilt binary must match the example, features, target, and profile used
for packaging. When packaging multiple targets, skipping the build also skips
cargo-bundle's `lipo` step; create the universal binary at the expected
`universal` path yourself. For a custom profile, pass `--profile <name>` to
both commands. Use the same `--target` options in each step, and make sure
Cargo resolves the same target directory.

The `publish` job downloads all three named workflow artifacts and attaches
their files to the release for the pushed tag. It receives `contents: write`
only for that job; package jobs and manual runs retain `contents: read`. To sign
artifacts during packaging, configure the platform settings and required feature
or OIDC token described in [Artifact signing](signing.md). For macOS, the
optional [notarization backend](signing/apple.md#notarize-and-staple-on-macos)
can submit and staple the package in the same run.
