# Resources and icons

Put files the program needs at runtime in `resources`; put application artwork in `icon`. An image can go in both arrays if the program reads the original file. For example, this configuration bundles an assets directory and JSON data files, and supplies artwork in formats commonly used on Linux and macOS:

```toml
[package.metadata.bundle]
resources = ["assets", "data/**/*.json", "config/default.toml"]
icon = ["icons/app.svg", "icons/app-128.png", "icons/app-256@2x.png"]
```

From the package directory, build a Debian package with:

```sh
cargo bundle --format deb
```

Paths and glob patterns are expanded from the working directory. WiX (`wxsmsi`)
also resolves resource and icon paths against the package's manifest directory
when it generates the installer. Run from the package directory when packaging
a workspace member so the inputs resolve the same way across formats.

A resource pattern can match a file or directory. Matched directories are walked
recursively. Icon patterns must match files. A glob that matches nothing adds
nothing, so check the pattern when an expected file is missing. Most backends
report malformed patterns and filesystem errors; WiX currently discards errors
from resource enumeration, so inspect its generated file list as well.

## Where files go

In the manifest example above, `assets/images/logo.png` stays under `assets/`
in the bundle. Matched directory names are retained; the `assets` component is
not stripped.

| Bundle format | Resource destination |
| --- | --- |
| macOS `.app` | `Contents/Resources/<mapped-path>` |
| iOS `.app` | `<app-root>/<mapped-path>` |
| Debian `.deb` | `usr/lib/<binary-name>/<mapped-path>` |
| AppImage AppDir | `usr/lib/<binary-name>/<mapped-path>` |
| Native Windows `.msi` | `Resources/<mapped-path>` |
| WiX Windows `.msi` (`wxsmsi`) | Nested source-relative paths below the install folder; put resources in a directory such as `assets/` |
| Windows `.exe` bundle | Resource files are not copied beside the executable |

For example, given this project:

```text
my-app/
  Cargo.toml
  assets/
    images/logo.png
    templates/welcome.html
  data/
    defaults.json
  config/
    default.toml
```

the manifest example above maps these files to the following Linux package paths:

```text
usr/lib/my-app/assets/images/logo.png
usr/lib/my-app/assets/templates/welcome.html
usr/lib/my-app/data/defaults.json
usr/lib/my-app/config/default.toml
```

Use package-relative paths such as `assets/data` in normal configurations.
For paths outside the package, the macOS, iOS, Debian, AppImage, and native MSI
writers replace an absolute root with the literal directory `_root_` and each
`..` component with `_up_`. This keeps the destination inside the bundle; it
does not preserve an absolute runtime path. For example,
`/opt/shared/key.dat` becomes `_root_/opt/shared/key.dat` below the resource
directory. WiX uses a different directory builder: use paths within the package,
and keep files in a subdirectory. A bare path such as `terms.txt` is referenced
by the generated WiX feature but its component is omitted, which can cause the
WiX build to fail. Use `assets/terms.txt` instead.

At runtime, build resource paths from the executable's location, not the
process's current working directory: a desktop launcher may start the program
from anywhere. For a macOS app, an executable at
`MyApp.app/Contents/MacOS/my-app` has resources under
`MyApp.app/Contents/Resources`. In a Debian install, `/usr/bin/my-app` has
resources under `/usr/lib/my-app`; the AppImage AppDir uses the same `usr/bin`
and `usr/lib` relationship. On Windows, native MSI resources are under
`Resources` beside the installed executable, while `wxsmsi` preserves their
source-relative directories. The `exe` format does not copy resources.

`cargo-bundle` does not provide a lookup API, set an environment variable, or
rewrite application paths. Your program must find its executable or bundle
directory and append the relative resource path, such as
`assets/images/logo.png`. Do not assume the same base directory on every
format; use the destination table and the platform's bundle layout.

## Icon files

The `icon` value is an array of file paths or globs. Each matched icon must be a file. A filename ending in `@2x` before its extension is treated as a high-density image on platforms that use that convention.

| Input | Linux `.deb` / AppImage | macOS `.app` | iOS `.app` |
| --- | --- | --- | --- |
| PNG | Installed as hicolor PNG at its pixel dimensions | Included in generated ICNS; supported sizes are assembled and nonstandard sizes resized | Copied as PNG at its dimensions |
| SVG | Copied as scalable hicolor icon | Rasterized into an ICNS icon | Rasterized into standard iOS PNG icon sizes |
| ICNS | Decoded and its contained sizes installed as PNG icons | Copied into the app's Resources and used as its icon | Decoded into PNG icons |
| JPEG, GIF, WebP, BMP, TIFF and other raster inputs | Converted to PNG | Converted to ICNS | Converted to PNG |

macOS uses the first SVG if present, otherwise the first ICNS, otherwise builds
an ICNS from raster inputs; for duplicate supported sizes, the first raster
input wins. iOS also uses the first SVG as its complete icon source, generating
PNG sizes for iPhone, iPad, and the 1024-pixel App Store icon. Without an SVG,
iOS processes PNGs before other formats, so a supplied PNG takes precedence
for a duplicate size.

Linux names installed icons after the binary and uses that name in the desktop
entry. For raster and ICNS inputs with the same dimensions and density, the
first encountered input wins. SVGs all share one scalable destination, so a
later SVG overwrites an earlier one there. Supply one SVG per bundle to avoid
that ambiguity. The AppImage root icon prefers SVG; without one it selects a
PNG from the largest available icon directory.

Windows has three icon paths:

| Format | Icon behavior |
| --- | --- |
| Native MSI (`msi`) | Copies the first lowercase `.ico` file into an MSI Icon table entry. Other formats are not converted, and that entry is not connected to a shortcut or an `ARPPRODUCTICON` property. |
| WiX MSI (`wxsmsi`) | Examines only the first matched icon. ICO, EXE, and DLL are used directly; supported raster images are converted to ICO. SVG or a failed conversion causes a fallback to the executable; later icons are not tried. |
| Executable copy (`exe`) | On Windows, embeds an icon only when an SVG is configured, rasterizing it to several PE icon sizes. Other icon formats are ignored for PE icon embedding. On non-Windows hosts, PE resource embedding is skipped. |

Raster inputs are decoded by the `image` crate and encoded in the format the
platform expects: ICNS on macOS, PNG on Linux and iOS. Conversion is intentional;
renaming a JPEG to `.png` would not produce a valid PNG. A corrupt or unsupported
image is an error. PNG is the most portable choice. For cross-platform projects, include PNGs plus any platform-specific files you need, such as ICNS for macOS or ICO for native MSI.
