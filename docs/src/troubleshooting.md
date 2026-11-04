# Troubleshooting

Start with an explicit format and keep the complete error output. Cargo-bundle
prints contextual errors to standard error and output paths to standard output.

## An unknown field in Cargo.toml

Check the table containing the setting. `frameworks` belongs in
`[package.metadata.bundle.osx]`, and `use_terminal` belongs in
`[package.metadata.bundle.linux]`. The former `script` field is not accepted.
The [manifest reference](manifest.md) lists every supported field.

## Missing settings with --bin or --example

Named targets use separate metadata tables and do not inherit the main
settings. Put the configuration under `bundle.bin.<name>` or
`bundle.example.<name>`, including its resources and identifier.

## No root package found in workspace

A virtual workspace has no root package. Select an exact member name with
`--package`, preferably from that member's directory. See
[workspace packages](cli.md#workspace-packages).

## No bin target is found

The implicit target must be a binary; cargo-bundle cannot package a library
on its own. If the package has an executable example, select it with
`--example NAME`. That example needs its own
[bundle settings](manifest.md#binaries-and-examples).

## The executable was not found

Check the selected binary, profile, target, and Cargo target directory.
Examples live in an extra `examples/` directory. A set
`CARGO_BUNDLE_SKIP_BUILD` skips compilation even if its value is `0`.
Unset it unless you have already built the expected file. Pass `--target`
explicitly when the binary lives under a target-triple directory.
If the selected executable is missing, verify that the target name, profile,
and target directory match the built file.

## AppImage cannot start from the temporary directory

The embedded runtime extracts the app before launching it. A temporary
filesystem mounted `noexec` prevents execution; choose an executable filesystem
with enough free space using `TMPDIR`. Shared-library errors are covered in
[AppImage shared libraries](#appimage-shared-libraries-are-missing).

## Missing icons or resources

Relative paths and globs are evaluated from the working directory. Unmatched
globs can produce no files without an error. Check the files from the same
directory where you run `cargo bundle`. See [Icons and resources](resources.md)
for destination paths and format-specific support.

## A framework was copied, but the app will not start

Copying a framework does not link the executable against it. `otool -L`
shows the linked library paths; `otool -l` shows load commands, including any
`LC_RPATH` entries. For the app from [First bundle](getting-started.md):

```sh
app_binary=target/release/bundle/osx/Postcard.app/Contents/MacOS/postcard
otool -L "$app_binary"
otool -l "$app_binary"
```

Set `app_binary` to your app's executable path. See the
[macOS frameworks guide](platforms/macos.md#frameworks-and-plugins) for the
run path cargo-bundle adds. Also test the app from Finder; a shell may provide
environment variables that Finder does not.

## AppImage shared libraries are missing

Library collection uses the build host's `/etc/ld.so.cache`, and the excluded
system libraries remain the user's responsibility. Read the warnings about
unresolved sonames. Libraries opened at runtime through plugins may not appear
in the executable's ELF dependencies. Test on the distributions and architectures
you intend to support; see [AppImage](platforms/appimage.md).

## RPM panics or an installer fails

RPM packaging is not implemented. Windows MSI and iOS packaging remain
experimental. Check the [format table](formats.md) and use a supported backend
for your release. Installer generation alone does not establish that installation,
upgrades, or uninstallation work correctly on Windows.

## Report a bug

Include the cargo-bundle version (and commit for a source install), host OS,
Rust target, exact command, relevant metadata, and full error output. A small
project that reproduces the problem is especially useful. Remove private data
before attaching logs. Use the repository's
[bug report form](https://github.com/philocalyst/cargo-bundle/issues/new?template=bug_report.yml).
