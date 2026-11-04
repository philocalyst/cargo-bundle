# AppImage

Cargo-bundle builds an AppImage with a statically linked Rust launcher and a
SquashFS payload. Launchers for `x86_64`, `aarch64`, `i686`, and `armhf` are
embedded in the AppImage library. Building an image does not fetch a runtime,
use an external `mksquashfs`, or require a user-supplied ELF file.

On launch, the runtime extracts the payload into a private temporary directory,
runs `AppRun` with your arguments, and removes the directory after the app exits.
It needs no FUSE device or mount helper. The temporary filesystem needs enough
space for the unpacked application and must allow execution; set `TMPDIR` to a
suitable directory if your usual temporary filesystem is mounted `noexec`.

The launcher preserves arguments and the working directory. It sets
`APPIMAGE` to the image path, `APPDIR` to the extracted tree, and `OWD` to
the caller's working directory. Terminal input and shell job control are
supported. The tree is removed when `AppRun` exits; an application that
starts another process must wait for it if that process still needs the files.
The launcher is static; libraries required by your application remain a
separate concern, covered below.

You can configure AppImage metadata and compression in the bundle manifest:

```toml
[package.metadata.bundle]
name = "Example"
identifier = "com.example.app"
appimage_metainfo_path = "data/example.metainfo.xml"
appimage_compression = "gzip"
```

`appimage_metainfo_path` is optional and is also read relative to the process
working directory. When omitted, cargo-bundle warns that AppStream metainfo is
recommended. When supplied, the XML is copied to
`usr/share/metainfo/<identifier>.appdata.xml` in the AppDir. If Linux
localizations are configured, elements marked with `trkey="Name"`,
`trkey="Comment"`, or `trkey="GenericName"` receive translated siblings and
the `trkey` attribute is removed.

Compression may be `gzip` (the default), `lz4`, `lzo`, or `none`. Other values
are errors. Gzip is the broad-compatibility default.

## Build and inspect

Build one AppImage explicitly:

```sh
cargo bundle --format appimage --release
```

The filename is `<binary-name>_<version>_<architecture>.AppImage`; cargo-bundle
prints its path. A completed candidate replaces the previous image only after
assembly and syncing succeed. The file is under
`<target-dir>/<profile>/bundle/appimage/`. A target triple adds its directory
before the profile, and example targets add `examples/` after the profile.
The sibling `<binary-name>_<version>_<architecture>/AppDir/` staging tree is
left in place after packaging, so you can inspect its files directly.

For example, mark the image executable and run it:

```sh
chmod +x target/release/bundle/appimage/myapp_1.0.0_x86_64.AppImage
./target/release/bundle/appimage/myapp_1.0.0_x86_64.AppImage
```

Here `myapp` is the binary target name and `1.0.0` is its bundle version.
AppImage architecture names are `x86_64`, `aarch64`, `i686`, or `armhf`;
the [Linux chapter](linux.md) lists the Cargo architecture aliases.

The AppDir contains the executable at `usr/bin/<binary>` and an `AppRun`
symlink to it. The desktop entry is under `usr/share/applications`, icons are
under `usr/share/icons/hicolor`, resources are under `usr/lib/<binary>`, and
license text is under `usr/share/doc/<binary>/copyright` when available. A
root-level desktop-entry link and, when an icon is configured, an icon plus
`.DirIcon` support AppImage integration.

To inspect the SquashFS contents using the embedded runtime, extract the image on a
Linux machine of its target architecture:

```sh
./target/release/bundle/appimage/myapp_1.0.0_x86_64.AppImage --appimage-extract
ls -l squashfs-root/AppRun
```

The runtime extracts into `squashfs-root` in the current directory. Use this
to confirm that expected resources and libraries made it into the image; it
does not prove that the application starts on your supported distributions.

## Shared libraries

On Linux, cargo-bundle follows the executable's ELF `DT_NEEDED` dependencies
using the build host's `/etc/ld.so.cache` and copies resolved libraries into
`usr/lib`, with some common system libraries excluded. Unresolved libraries
produce warnings and packaging continues, so check the build output. If the
cache cannot be read, collection is skipped; a malformed cache can stop the
build. On non-Linux hosts, collection is skipped.

For cross-compilation, this still uses the build host's library cache and does
not check that copied libraries match the target architecture. Libraries
loaded later by plugins or other runtime lookup are not discovered. Review
the binary's `DT_NEEDED` and `RPATH`/`RUNPATH` entries with
`readelf -d target/release/myapp`, then run the image on each target
architecture and the oldest distribution you intend to support.

`AppRun` is a direct symlink to the executable. Cargo-bundle does not set
`LD_LIBRARY_PATH` or add an `$ORIGIN` rpath, so copied libraries are not
guaranteed to be found by the dynamic loader. If the executable needs a custom
library search path, set an rpath at link time so the executable searches the
AppDir's `usr/lib` directory relative to `usr/bin`:

```sh
RUSTFLAGS='-C link-arg=-Wl,-rpath,$ORIGIN/../lib' cargo bundle --release --format appimage
readelf -d target/release/myapp | grep -E 'RPATH|RUNPATH'
```

`$ORIGIN/../lib` resolves from `usr/bin/myapp` to `usr/lib`. Confirm the
resulting AppDir contains the required libraries, then test the AppImage on
the target systems; plugin-loaded libraries and dependencies of copied
libraries may need additional handling.

The desktop entry and icons use the same bundle metadata as other Linux
formats. Configure fields such as `mime_types`, `exec_args`, `use_terminal`,
localizations, and actions in `[package.metadata.bundle.linux]`.
