# Bundle manifest

Add bundle settings to `[package.metadata.bundle]` in the selected package's
`Cargo.toml`. Put common settings at the table root and platform settings in
`linux`, `osx` (alias `macos`), `windows`, `ios`, or `android`. See the guides for
[Linux](platforms/linux.md), [AppImage](platforms/appimage.md),
[macOS](platforms/macos.md), [Windows](platforms/windows.md), and
[iOS](platforms/ios.md), and [Android](platforms/android.md).

Unknown keys in bundle and platform settings tables cause an error. The
old `script` key is unsupported. You can omit the whole bundle table;
cargo-bundle warns and uses defaults.

## General fields

| Key | Type | Default and effect |
| --- | --- | --- |
| `name` | string | Cargo package name. Used as the displayed bundle name. |
| `identifier` | string | Main artifact: empty string. A selected binary: `<bin>.<package>`; an example: `<example>.example.<package>`. Supply your own reverse-DNS identifier, especially for OS formats that require one. |
| `icon` | array of paths or glob patterns | No icons. Directories are not recursively searched. |
| `version` | string | Cargo package version. |
| `resources` | array of paths or glob patterns | No resources. Matching directories are copied recursively. |
| `localization_files` | array of TOML file paths | No external translations. Paths are relative to the package manifest. See [Accessibility and localization](accessibility-localization.md). |
| `copyright` | string | Unset. |
| `category` | category string | Unset. Accepts a human-readable name, Apple category ID, or GNOME category; see below. |
| `short_description` | string | Cargo package `description`, or empty if absent. |
| `long_description` | string | Unset. |
| `deb_depends` | array of strings | No Debian package dependencies. See [Linux](platforms/linux.md). |
| `appimage_metainfo_path` | path string | Unset. AppStream XML to include in an AppImage. |
| `appimage_compression` | string | AppImage default is `gzip`; supported values are `gzip`, `lz4`, `lzo`, and `none`. |
| `linux` | table | No Linux-specific settings. See below. |
| `osx` or `macos` | table | No macOS-specific settings. `osx` is the canonical spelling. See below. |
| `ios` | table | Shared-field overrides and `localizations` for localized plist strings. |
| `windows` | table | Platform overrides for shared fields. |
| `android` | table | NativeActivity APK settings, signing key and shared-field overrides; see [Android](platforms/android.md). |
| `bin` | table of named tables | No per-binary settings. Each key is a binary target name and its value is a complete bundle settings table. |
| `example` | table of named tables | No per-example settings. Each key is an example target name and its value is a complete bundle settings table. |

Cargo package metadata supplies other bundle details, including authors,
homepage, and license. If the package sets `license-file`, cargo-bundle reads
it relative to that package's manifest directory. If it cannot read the file,
it warns and uses the package's `license` value when available.

`Puzzle Game`, `public.app-category.puzzle-games`, and `LogicGame` all select
the puzzle-game category. `Utility` and `Developer Tool` are other accepted
values. The
[category mapping](https://github.com/philocalyst/cargo-bundle/blob/master/src/bundle/category.rs)
lists all accepted categories. An unrecognized value is an error, with a
suggestion when a close match is available.

## Platform shadowing

Shared values go at the root. A platform table replaces only the fields it
sets; omitted fields keep their root value. A supplied array or table replaces
the whole value. An empty array explicitly removes the shared list:

```toml
[package.metadata.bundle]
name = "Postcard"
identifier = "com.example.postcard"
icon = ["assets/app.png"]
resources = ["assets/data"]

[package.metadata.bundle.macos]
icon = ["assets/app.svg"]
minimum_system_version = "12.0"

[package.metadata.bundle.windows]
icon = ["assets/windows.svg"]

[package.metadata.bundle.ios]
icon = ["assets/ios.svg"]
resources = []

[package.metadata.bundle.linux]
icon = ["assets/app.svg"]
use_terminal = false
```

The first `--target` selects the platform, or the host OS when no target is
given. `--format` changes the output writer, not the target platform.
`osx` and `macos` name the same table; use one spelling per bundle.
Linux and macOS tables also accept the backend fields listed below.
The iOS table also accepts `localizations`, in the same form as macOS.
Windows accepts shared fields.

For a named binary or example, the same platform tables go beneath its own
bundle table, such as `[package.metadata.bundle.bin.editor.windows]`. That
artifact table still does not inherit the package's root bundle settings.

## Signing fields

Signing is optional. See [Artifact signing](signing.md) for complete recipes,
feature requirements, secrets, and verification.

| Key | Type | Default |
| --- | --- | --- |
| `apple_signing_p12` | path string | Unset; no Apple signing. |
| `apple_signing_password_env` | string | Unset; use an empty P12 password. |
| `apple_signing_timestamp_url` | URL string | No timestamp service. |
| `apple_signing_entitlements` | path string | No additional entitlements. |
| `apple_signing_hardened_runtime` | boolean | `false`. |
| `apple_notarization_api_key` | path string | Unset. API-key JSON file for optional macOS notarization and stapling; requires the `apple-notarization` Cargo feature. |
| `apple_notarization_timeout` | positive integer | `600` seconds of processing wait. See [Apple distribution](signing/apple.md#notarize-and-staple-on-macos). |
| `windows_signing` | table | No Authenticode signing. |
| `linux_signing` | table | No detached Sigstore signature. |

## Linux fields

Put these keys in `[package.metadata.bundle.linux]`.

| Key | Type | Default and effect |
| --- | --- | --- |
| `mime_types` | array of strings | No `MimeType` entry in the desktop file. |
| `exec_args` | string | No extra arguments in the desktop file's `Exec` value. |
| `use_terminal` | boolean | `false`, emitted as `Terminal=false`. Set true for terminal applications. |
| `localizations` | tables by locale | No translations. Each locale can define FreeDesktop fields such as `Name`, `GenericName`, `Comment`, `Keywords`, and `Icon`; see [Linux](platforms/linux.md). |
| `startup_wm_class` | string | No `StartupWMClass` desktop-file key. |
| `desktop_actions` | tables by action id | No launcher actions. Each action has a required `Name`, optional `Exec` and `Icon`, and optional `NameLocalized` table mapping locale to translated name. If `Exec` is omitted, the app binary is used. |

For example, this adds a French desktop action name:

```toml
[package.metadata.bundle.linux]
mime_types = ["text/plain"]
exec_args = "%f"
use_terminal = false
startup_wm_class = "my-app"

[package.metadata.bundle.linux.desktop_actions.new-window]
Name = "New Window"
Exec = "my-app --new-window"

[package.metadata.bundle.linux.desktop_actions.new-window.NameLocalized]
fr = "Nouvelle fenêtre"
```

## macOS fields

Put these keys in `[package.metadata.bundle.osx]`. The accepted `macos` alias
works here too.

| Key | Type | Default and effect |
| --- | --- | --- |
| `frameworks` | array of strings | No frameworks copied into the app. |
| `plugins` | array of strings | No plugins copied into the app. |
| `minimum_system_version` | string | Unset. |
| `url_schemes` | array of strings | No URL schemes registered. |
| `info_plist_exts` | array of paths or glob patterns | No extra plist content. Matched files are appended to `Info.plist`; directories are not recursively searched. |
| `localizations` | nested tables of strings | No localized `InfoPlist.strings` files. See [macOS](platforms/macos.md). |
| `dmg_background` | path string | No custom DMG background. When supplied, the SVG needs elements with IDs `app` and `applications`. |

`frameworks` accepts framework names found in standard macOS framework
directories, or explicit paths to `.framework` directories and `.dylib` files.
`plugins` contains paths to plugin directories. Both are copied into the app;
they are not glob patterns. Explicit framework and plugin paths, the AppImage
metainfo path and the DMG background path are read relative to
the process's current working directory.

## Binaries and examples

Without `--bin` or `--example`, cargo-bundle chooses the package's first binary
and uses the root `[package.metadata.bundle]` settings. `--bin NAME` selects
`[package.metadata.bundle.bin.NAME]`, even if NAME is the first binary.
`--example NAME` selects `[package.metadata.bundle.example.NAME]`. Each
artifact table is complete; it does not inherit values from the root table.
Repeat any common values you need. If the selected table is missing,
cargo-bundle warns and uses defaults. Artifact tables accept the supported
general and platform keys. Further nested `bin` or `example` tables are
parsed but are not used for target selection.

```toml
[package.metadata.bundle]
name = "My App"
identifier = "com.example.my-app"
icon = ["assets/icon.png"]
resources = ["assets"]

[package.metadata.bundle.bin.editor]
name = "My App Editor"
identifier = "com.example.my-app.editor"
icon = ["assets/editor.png"]
resources = ["assets"]
```

For example, `cargo bundle --bin editor` uses the `bin.editor` table shown
above, even when `editor` is the package's first binary. Because that table
does not inherit from the root table, repeat `resources` and any other shared
settings you need.

## Paths and globs

`localization_files` paths are relative to the selected package's manifest
directory. They are explicit TOML filenames, without glob expansion.

Icon, resource, and plist-extension paths and globs are expanded from the
process's current working directory. Run `cargo bundle` from a directory where
those relative paths resolve; running from the package directory is the usual
choice for a workspace member. Resource directories are traversed recursively;
icon and plist-extension matches must be files. See [Resources and icons](resources.md)
for destination paths and runtime lookup guidance.
