# Choose a format

Use `--format` to choose one output. A recognized name does not necessarily
mean a finished backend: `rpm` is accepted by the parser but panics when used.

| Format | Output | Status and requirements |
| --- | --- | --- |
| `deb` | Debian package | Implemented; archive creation is handled in Rust. Declare runtime package dependencies yourself. |
| `appimage` | Type-2 AppImage | Implemented; embeds a statically linked Rust launcher; no runtime download. Linux library collection uses the build host's cache. |
| `osx` | macOS `.app` | Implemented; copying frameworks uses `install_name_tool`. |
| `dmg` | macOS disk image | Rust HFS+ and compressed UDIF writer; no external image tools. |
| `exe` | Windows executable copy | Implemented; PE icon/version editing requires a Windows host. |
| `msi` | Windows installer | Experimental native MSI writer. |
| `wxsmsi` | Windows installer through WiX | Experimental; requires `dotnet` and restoration of the generated project's WiX SDK packages. |
| `ios` | iOS `.app` directory | Experimental; optional Apple signing. |
| `ipa` | iOS application archive | Experimental; Rust ZIP writer; requires an iOS device target. |
| `apk` | Android application | NativeActivity shared library; Rust resource compilation, alignment and optional signing. |
| `rpm` | None | Not implemented. |

See [Linux](platforms/linux.md), [AppImage](platforms/appimage.md),
[macOS](platforms/macos.md), [Windows](platforms/windows.md), and
[iOS](platforms/ios.md) for configuration and limitations.
See [Android](platforms/android.md) for APK configuration.

## Defaults

Without `--format`, cargo-bundle selects one format for the target platform.
Pass `--format` to select another supported format.

The defaults are `osx` on macOS, `deb` on Linux, `exe` on Windows, and
`ios` for iOS, and `apk` for Android. A `--target` triple takes precedence over the host OS.
Each invocation produces one format.

## Cross-compilation

`--target` is passed to Cargo and determines the binary architecture; it does
not install a compiler, platform SDK, linker, or packaging tools. Selecting a
format with `--format` does not choose a compilation target. Use both when
cross-compiling, and check whether that backend can run on the build host.

The target triples and minimum OS versions depend on your Rust toolchain and
your application's dependencies. Cargo-bundle does not provide its own
supported-target list. Repeated targets are intended for macOS universal
binaries and require `lipo`; see [macOS](platforms/macos.md).
