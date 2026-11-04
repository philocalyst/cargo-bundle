# macOS

`cargo bundle --format osx` wraps the selected Cargo executable in a Finder-ready `.app` directory. Add a name and stable identifier to `[package.metadata.bundle]` in `Cargo.toml`. If your manifest already has this table, add the two keys there instead of declaring it again.

```toml
[package.metadata.bundle]
name = "Example"
identifier = "com.example.app"
```

Build the `.app` bundle:

```sh
cargo bundle --format osx --release
```

From the package directory, with Cargo's default target directory, the app is written to `target/release/bundle/osx/Example.app`. It contains `Contents/Info.plist` and `Contents/MacOS/<binary>`. Icons, configured resources, and localized strings are placed under `Contents/Resources/` when supplied. Check the generated plist with:

```sh
plutil -lint target/release/bundle/osx/Example.app/Contents/Info.plist
```

The command prints the resulting path. Other targets, profiles, or Cargo target-directory settings may change it; see [Targets and output directories](../cli.md#targets-and-output-directories). Use `--example NAME`, `--bin NAME`, or `-p NAME` to select a different target or package. Named binaries and examples use their own [bundle settings tables](../manifest.md#binaries-and-examples).

## Add app resources and icons

Add `icon` and `resources` to the existing `[package.metadata.bundle]` table when the app needs them. Keep its header and existing values; TOML tables cannot be declared twice.

```toml
icon = ["assets/app.icns"]
resources = ["assets/data"]
```

`resources` accepts files, directories, and globs. A matched directory is copied recursively, keeping its path: `assets/data/config.json` becomes `Contents/Resources/assets/data/config.json`. See [Resource paths](../resources.md#where-files-go) for icon formats and mapping details.

At runtime, your app must locate its own `.app` bundle and read the file relative to `Contents/Resources/`; cargo-bundle does not provide a resource lookup API. For example, the path beneath the resources directory is `assets/data/config.json`.

## macOS bundle settings

Add macOS-only options under `[package.metadata.bundle.osx]`. Create this table once, then add any further macOS options to it rather than repeating the header:

```toml
[package.metadata.bundle.osx]
minimum_system_version = "11.0"
url_schemes = ["example"]
```

`minimum_system_version` sets `LSMinimumSystemVersion`; use a value consistent with the deployment target used to compile the executable. `url_schemes` registers the listed schemes in `CFBundleURLTypes`.

The main bundle table's `version` supplies `CFBundleShortVersionString`; if omitted, cargo-bundle uses the Cargo package version. `CFBundleVersion` is set to the UTC build time in `YYYYMMDD.HHMMSS` form. `category` sets `LSApplicationCategoryType`. The bundle `identifier` is optional in the manifest, but should be a stable reverse-DNS value for a distributed app. Without it, the generated identifier is empty for the main target, `<bin>.<package>` for a named binary, or `<example>.example.<package>` for an example.

To add keys to the generated `Info.plist`, create an XML fragment file and add its path to the same osx table:

```toml
info_plist_exts = ["packaging/InfoPlistExtras.xml"]
```

For example, `packaging/InfoPlistExtras.xml` can contain:

```xml
<key>NSCameraUsageDescription</key>
<string>Camera access is used to scan a code.</string>
```

The file contents are inserted into the generated dictionary. cargo-bundle replaces every `&` with `&amp;` before writing them; do not pre-escape entities such as `&amp;`. Avoid redefining keys that cargo-bundle already writes.

## Frameworks and plugins

Add `frameworks` and `plugins` to the existing `[package.metadata.bundle.osx]` table only when you have those dependencies to copy. Replace the sample paths with files or directories that exist:

```toml
frameworks = ["ExampleFramework", "vendor/Helper.framework", "vendor/libhelper.dylib"]
plugins = ["vendor/Example.plugin"]
```

`frameworks` accepts framework names without `.framework`, paths ending in `.framework`, or paths ending in `.dylib`. Bare names are searched in `~/Library/Frameworks`, `/Library/Frameworks`, `/Network/Library/Frameworks`, and `/System/Library/Frameworks`. Explicit paths are relative to the directory where you run `cargo bundle` (or can be absolute); paths containing `/` must have one of the accepted suffixes. Frameworks and dylibs are copied to `Contents/Frameworks`; plugin directories are copied to `Contents/PlugIns`.

Copying a library does not link it into the executable or fix its install names. When at least one framework or dylib is copied, cargo-bundle ensures the executable has `@executable_path/../Frameworks` in its run paths, adding it with macOS's `install_name_tool` when needed. The executable must already link to libraries with install names that resolve through that run path. Plugins do not trigger this run-path update.

## Localized plist strings

Add locale tables beneath the existing osx table. For example:

```toml
[package.metadata.bundle.osx.localizations.fr]
CFBundleDisplayName = "Exemple"
NSHumanReadableCopyright = "Droits réservés"
```

Each locale creates `Contents/Resources/<locale>.lproj/InfoPlist.strings`.
Keys and values are quoted and escaped. This localizes plist strings only;
it does not translate application UI or other files. You can also keep these
translations in [separate TOML files](../accessibility-localization.md#keep-translations-in-separate-files),
using the same loading rules as Linux and iOS.

## Disk images

To make a compressed, read-only `.dmg` with the app and an `Applications` link, run:

```sh
cargo bundle --format dmg --release
```

The image goes under `bundle/dmg/` in the selected target and profile directory.
The Rust writer builds an HFS+ volume and wraps it in compressed, read-only
UDIF. It stages the app and Finder files privately; creation does not mount an
image or invoke external image tools. A failed build leaves the previous DMG in place.

The integration suite verifies UDIF checksums and checks the HFS+ filesystem
without mounting the image.

The writer preserves files, POSIX modes, links, FinderInfo, resource forks,
and supported extended attributes. Quarantine is omitted; attributes outside
HFS+ limits fail the build. Compressed source files are materialized as ordinary
files. The fresh staging root uses fstool’s default POSIX permissions; fstool
0.5.2 cannot change root permissions through `set_attrs`. Root Finder metadata
and creation dates are preserved. Keep inputs unchanged while the command runs.
Framework run-path editing and universal binary
combination still need the macOS tools described in this chapter.

To decorate the Finder window, set `dmg_background` under the existing `[package.metadata.bundle.osx]` table to an SVG file. The SVG needs elements with IDs `app` and `applications`; their centers set the app and Applications-link icon positions. cargo-bundle renders a TIFF background at 2x and writes Finder window and icon-position records to `.DS_Store`. See [`examples/hello/dmg-background.svg`](https://github.com/philocalyst/cargo-bundle/blob/master/examples/hello/dmg-background.svg).

## Universal builds and signing

On macOS, install both Rust targets, then repeat `--target` to build each architecture and combine the executables with `lipo`:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo bundle --format osx --release \
  --target aarch64-apple-darwin \
  --target x86_64-apple-darwin
lipo -info target/universal/release/bundle/osx/Example.app/Contents/MacOS/*
```

The final command inspects the executable inside the app, whose name comes from
the Cargo binary target, and should report both `arm64` and `x86_64`. Universal
binary combination requires macOS and `lipo`.

With `apple_signing_p12`, cargo-bundle signs the `.app`; the `dmg` format also
signs the final disk image. The optional `apple-notarization` feature adds
submission and ticket stapling. See [Apple signing and distribution](../signing/apple.md)
for configuration and verification.
