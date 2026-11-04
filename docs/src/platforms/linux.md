# Linux

For a Linux target, `cargo bundle` creates a Debian package by default. To
build a type-2 AppImage, select it explicitly:

```sh
cargo bundle --format deb
```

The AppImage backend includes its own Rust launcher:

```sh
cargo bundle --format appimage
```

Add `--release` for a release build; otherwise Cargo's development profile is
used.

Use `--target <triple>` to build for another Linux target. AppImage filenames
use these architecture names: `x86_64` stays `x86_64`; `aarch64` and `arm64`
become `aarch64`; `i686`, `x86`, `i586`, and `i386` become `i686`; `armhf`,
`arm`, `armv7`, and `armv7l` become `armhf`. Other architectures are not
supported by the AppImage backend. Target builds are written under the target
triple directory; see [command-line output directories](../cli.md#targets-and-output-directories).

Both formats include the executable, a generated FreeDesktop desktop entry,
and supported icons. Configure desktop fields under
`[package.metadata.bundle.linux]`; put Debian package dependencies in
`deb_depends` at the bundle table root. The [AppImage chapter](appimage.md)
covers the embedded launcher and shared libraries.

RPM is accepted as a format name but has no implementation.

## Desktop entries

Set `category` in `[package.metadata.bundle]` to choose the desktop entry's
category. If omitted, cargo-bundle warns and writes `Categories=Utility;`.
Linux-specific settings include MIME types, extra executable arguments,
terminal behavior, startup window class, localizations, and desktop actions:

```toml
[package.metadata.bundle]
name = "Example"
identifier = "com.example.app"
category = "Utility"
deb_depends = ["libgtk-3-0"]

[package.metadata.bundle.linux]
mime_types = ["text/plain"]
exec_args = "%f"
use_terminal = false
startup_wm_class = "example"

[package.metadata.bundle.linux.desktop_actions.new-window]
Name = "New Window"
Exec = "example --new-window"
```

`exec_args` is appended to the desktop entry's `Exec` value. MIME types are
written with semicolons. `use_terminal` defaults to false. Each desktop action
requires `Name`; `Exec` defaults to the application binary. Debian's
`deb_depends` becomes the package's `Depends` field and does not configure an
AppImage.

Translations can live in [separate TOML files](../accessibility-localization.md#keep-translations-in-separate-files)
or in the manifest. Localization tables accept `Name`, `GenericName`, `Comment`, `Keywords`, and
`Icon`. These become localized desktop-entry keys such as `Name[fr]`. For
example:

```toml
[package.metadata.bundle.linux.localizations."fr.UTF-8"]
Name = "Mon application"
Comment = "Une description courte"
GenericName = "Utilitaire"
Keywords = ["outil", "utilitaire"]

[package.metadata.bundle.linux.localizations.de]
Name = "Meine Anwendung"
Comment = "Eine kurze Beschreibung"
GenericName = "Dienstprogramm"
Keywords = "werkzeug;dienstprogramm"
```

The base (unlocalized) fields remain useful to desktops that do not select a
translation:

| Field | Base value |
| --- | --- |
| `Name` | Always the bundle name. Localized `Name` values are still written. |
| `Comment` | The effective short description (package `description` by default); a non-empty translation is used if the description is empty. |
| `GenericName` | A non-empty localized value, when supplied. |
| `Keywords` | A localized value, when supplied. |

Fallback selection checks `C`, then `en`, then the remaining locales in sorted
order. `Comment` and `GenericName` skip empty values; `Keywords` uses the first
configured value, even an empty list.

Desktop string values escape backslashes, tabs, carriage returns, and newlines;
keyword-list items also escape embedded semicolons.

## Debian packages

The `.deb` installs the executable under `usr/bin`, resources under
`usr/lib/<binary-name>`, icons under `usr/share/icons/hicolor`, and its
desktop entry under `usr/share/applications`. Use `deb_depends` to list
runtime packages. Cargo-bundle does not infer Debian dependencies from linked
libraries and does not create post-install or removal scripts.

The package file is named
`<binary-name>_<version>_<architecture>.deb` and is written to
`<target-dir>/<profile>/bundle/deb/`. With `--target`, the target triple is
inserted before the profile directory. Example targets also add `examples/`
after the profile. Cargo-bundle prints the package path when it finishes.

The adjacent `<binary-name>_<version>_<architecture>/` directory is the
staging tree used to assemble the package. Its `data/` and `control/`
directories let you inspect the installed files and package metadata before
installing. You can also inspect the completed package with Debian tools:

```sh
dpkg-deb --info target/release/bundle/deb/example_1.0.0_amd64.deb
dpkg-deb --contents target/release/bundle/deb/example_1.0.0_amd64.deb
```

Replace the sample path with the one cargo-bundle prints. Check the package
name, version, architecture, dependencies, and installed file paths.
To validate the generated desktop entry, extract the package and run
`desktop-file-validate` (from the `desktop-file-utils` package):

```sh
mkdir -p /tmp/example-deb
dpkg-deb --extract target/release/bundle/deb/example_1.0.0_amd64.deb /tmp/example-deb
desktop-file-validate /tmp/example-deb/usr/share/applications/example.desktop
```

Replace the example path and desktop filename with the values in your package.
Install the package with APT, which also resolves declared dependencies:

```sh
sudo apt install ./target/release/bundle/deb/example_1.0.0_amd64.deb
```

To remove it, read the Debian package name from the package itself, then pass
that name to `apt remove`:

```sh
dpkg-deb --field target/release/bundle/deb/example_1.0.0_amd64.deb Package
sudo apt remove example
```

Replace `example` with the name printed by the first command. Removing the
package does not remove user data stored outside the package's installed files.
