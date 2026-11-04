# Platforms

Each chapter explains what its format produces and what the build host needs. Choose a format for the artifact you intend to distribute:

- [Linux](linux.md): Debian packages and desktop integration; [AppImage](appimage.md): an AppDir image with an embedded Rust launcher.
- [macOS](macos.md): `.app` bundles and compressed HFS+ disk images created in Rust.
- [Windows](windows.md): executable copies and two experimental MSI writers; the WiX format needs `dotnet` and package restore.
- [iOS](ios.md): experimental `.app` and IPA packaging with optional Apple signing and provisioning profiles.

- [Android](android.md): NativeActivity APK packaging.

`--format` selects the packaging backend; `--target` selects the Rust target and does not install the compiler, SDK, linker, or packaging tools. See [Choose a format](../formats.md) for defaults and status, and [Command line](../cli.md) for targets and output paths.
