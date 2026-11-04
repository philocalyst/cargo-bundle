# Artifact signing

Configure signing in the selected bundle's metadata. Without signing settings,
cargo-bundle leaves the artifact unsigned.

| Output | Signing guide |
| --- | --- |
| macOS app or disk image; experimental iOS app | [Apple signing and distribution](signing/apple.md) |
| Windows executable or MSI | [Windows Authenticode](signing/windows.md) |
| Debian package or AppImage | [Linux Sigstore](signing/linux.md) |

Signing settings follow the same [platform shadowing](manifest.md#platform-shadowing)
rules as other bundle settings. Named binaries and examples use their own
configuration. Certificate and entitlement paths are relative to the directory
where you run cargo-bundle; keep passwords and tokens in your secret store.
