# Localization

Translations of the display name and similar strings go in a `localizations`
table per platform, keyed by locale code:

```toml
[package.metadata.bundle.osx.localizations.fr]
CFBundleDisplayName = "Mon Application"

[package.metadata.bundle.ios.localizations.fr]
CFBundleDisplayName = "Mon Application"

[package.metadata.bundle.linux.localizations.fr]
Name = "Mon App"
Comment = "Une description"
```

| Platform | Result |
| --- | --- |
| macOS | `Contents/Resources/<locale>.lproj/InfoPlist.strings` |
| iOS | `<locale>.lproj/InfoPlist.strings` at the root of the app, sealed by the signature |
| Linux | `Name[fr]=` style keys in the `.desktop` file; see [Linux](platforms/linux.md) |

The strings files are written with sorted, quoted and escaped entries, so the
same settings always produce the same bytes and values may contain quotes,
backslashes and newlines.

## Locale codes

Use letters, digits, `_` and `-`, such as `fr`, `pt_BR` or `zh-Hans`. A
trailing `.encoding` (`fr.UTF-8`) is dropped, and an `@modifier` (`sr@latin`)
is kept. Anything else, such as `../x` or `a/b`, is rejected when settings are
read, as is a pair like `en` and `en.UTF-8` that would mean the same locale.
