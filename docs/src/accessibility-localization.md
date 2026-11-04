# Accessibility and localization

Translate launcher names and bundle metadata here. Package UI translation
catalogs as resources, and use your application's toolkit for keyboard
navigation and screen-reader support.

## Keep translations in separate files

Use `localization_files` to keep bundle metadata translations out of
`Cargo.toml`:

```toml
[package.metadata.bundle]
name = "Postcard"
identifier = "com.example.postcard"
localization_files = ["locales/en.toml", "locales/fr.toml"]
```

Paths are relative to the selected package's `Cargo.toml`, including when
bundling from a workspace root. Each file can contain translations for several
platforms. For example, `locales/fr.toml`:

```toml
[linux.fr]
Name = "Carte postale"
GenericName = "Éditeur de cartes"
Comment = "Créez et envoyez des cartes postales"
Keywords = ["carte", "courrier"]

[macos.fr]
CFBundleDisplayName = "Carte postale"

[ios.fr]
CFBundleDisplayName = "Carte postale"
```

Files are read in list order; a later value replaces the same key in an
earlier file. Inline `localizations` values override file values. Other keys
in the same locale are retained. `osx` is an alias for `macos` in these files;
use one spelling per file.

Unknown platform sections, unrecognized Linux fields, invalid value types,
and unsafe locale names are errors, including in inactive platform sections.
Locale names that become
identical after removing their encoding suffix are also rejected.

The file list is a shared setting, so a platform table can replace it and
`localization_files = []` disables file loading for that platform. Named
binary and example configurations need their own list, as they do for other
[bundle settings](manifest.md#binaries-and-examples).

## What each platform receives

| Platform | Generated metadata |
| --- | --- |
| Linux | Localized `Name`, `GenericName`, `Comment`, `Keywords`, and `Icon` in the desktop entry. AppImage metainfo can reuse the name and description translations. |
| macOS | `Contents/Resources/<locale>.lproj/InfoPlist.strings`. |
| iOS | `<locale>.lproj/InfoPlist.strings` at the app bundle root. |
| Windows | No native localized metadata writer. Package application translation files as resources. |

Linux locale codes follow the desktop-entry convention, such as `fr`,
`pt_BR`, or `sr@latin`; an encoding suffix such as `.UTF-8` is omitted from
generated keys. See [localized desktop entries](platforms/linux.md#desktop-entries)
for base values and fallback behavior, and the
[Desktop Entry Specification](https://specifications.freedesktop.org/desktop-entry/latest-single/#localized-values)
for how desktops select a translation.

Apple's files contain localized plist values such as `CFBundleDisplayName`.
They do not translate your menus or other UI. Use the locale identifiers and
keys your Apple application expects; see Apple's
[localization guide](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPInternational/LocalizingYourApp/LocalizingYourApp.html).

For application translations, package your toolkit's catalogs, fonts, and other
data through `resources`. Cargo-bundle copies these files without compiling
catalogs or choosing a runtime language. Their installed locations differ by
platform; use the [resource path reference](resources.md#where-files-go) when
implementing lookup. Check a non-default language and a missing translation
in the installed application.

## Check accessibility after packaging

Use a clear application name and a short description of what it does. Check
that translated names still fit launchers and dialogs. Keep text in the UI
available to assistive technology rather than drawing it only into an image.

Run the installed app and check these tasks:

- Open it from the platform launcher. Confirm its name, icon, and description
  identify the application in each supported language.
- Complete its main task with the keyboard. Check focus visibility, tab order,
  dialogs, and returning focus after a dialog closes. W3C's
  [keyboard guidance](https://www.w3.org/WAI/WCAG21/Understanding/keyboard)
  explains the underlying interaction requirements.
- Repeat the task with the platform's screen reader. Check control names,
  roles, state changes, and error messages using your toolkit's accessibility
  support. Include any first-run and file-selection dialogs.
- Check larger text, high contrast, and a language with longer labels. Include
  right-to-left layout if the application supports a right-to-left language.

Run these checks on the artifact you distribute. Bundled resources, fonts,
missing shared libraries, or launch-time environment differences can change
behavior from a development build.
