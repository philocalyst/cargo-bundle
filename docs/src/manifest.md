# Manifest reference

Settings live in `[package.metadata.bundle]` in the package's `Cargo.toml`.
Unknown keys are errors. Paths are resolved from the directory you run
`cargo bundle` in; run it from the package directory.

## General

| Key | Meaning |
| --- | --- |
| `name` | Display name, also the file name of most outputs. Defaults to the package name. It must be a single file name: no `/`, `\`, `.` or `..`. |
| `identifier` | Reverse-DNS identifier such as `com.example.app`. Set it explicitly. It becomes the macOS and iOS `CFBundleIdentifier`, the Android package name and the basis of the Windows product GUID. |
| `version` | Defaults to the package version. |
| `icon` | Icon files or globs. See [Resources and icons](resources.md). |
| `resources` | Files or directories (globs allowed) copied into the bundle. |
| `copyright` | Copyright line. |
| `category` | A free-form category (`"Puzzle game"`), an Apple category (`"public.app-category.puzzle-games"`) or a FreeDesktop one (`"LogicGame"`); it is converted per platform. |
| `short_description` | One line. Defaults to the package `description`. |
| `long_description` | Several lines. |

## Per-binary and per-example settings

The main table applies to the package's first binary. A named binary or example
uses its own table, which is read **instead of** the main one rather than on
top of it, so repeat the keys it needs:

```toml
[package.metadata.bundle.bin.my-tool]
name = "My Tool"
identifier = "com.example.my-tool"

[package.metadata.bundle.example.demo]
name = "Demo"
identifier = "com.example.demo"
```

Select one with `--bin my-tool` or `--example demo`. A missing table is a
warning and means all defaults.

## Platform tables

| Table | Keys | Page |
| --- | --- | --- |
| `osx` (or `macos`) | `frameworks`, `plugins`, `minimum_system_version`, `url_schemes`, `info_plist_exts`, `localizations`, `dmg_background` | [macOS](platforms/macos.md) |
| `ios` | `localizations` | [iOS](platforms/ios.md) |
| `android` | `version_code`, `min_sdk`, `target_sdk`, `permissions`, `signing_key` | [Android](platforms/android.md) |
| `linux` | `mime_types`, `exec_args`, `use_terminal`, `localizations`, `startup_wm_class`, `desktop_actions` | [Linux](platforms/linux.md) |

Top-level keys for single formats: `deb_depends` ([Linux](platforms/linux.md)),
`appimage_metainfo_path`, `appimage_compression`, `appimage_runtime`
([AppImage](platforms/linux.md#appimage)), and the signing keys listed in
[Signing](signing.md).

## Example

```toml
[package.metadata.bundle]
name = "ExampleApplication"
identifier = "com.doe.exampleapplication"
icon = ["32x32.png", "128x128.png", "128x128@2x.png"]
resources = ["assets", "images/**/*.png"]
copyright = "Copyright (c) Jane Doe 2016. All rights reserved."
category = "Developer Tool"
short_description = "An example application."
deb_depends = ["libgl1-mesa-glx", "libsdl2-2.0-0 (>= 2.0.5)"]

[package.metadata.bundle.osx]
frameworks = ["SDL2"]
url_schemes = ["com.doe.exampleapplication"]
```
