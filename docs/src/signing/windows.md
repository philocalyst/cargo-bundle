# Windows signing


Windows signing uses a PKCS#12 certificate and the optional `osslsigncode` dependency. It is opt-in because the vendored implementation is GPL-3.0-or-later. Enable the feature when building or installing cargo-bundle; otherwise, configured signing fails.

```toml
[package.metadata.bundle.windows_signing]
certificate_path = "certs/windows-signing.pfx"
certificate_password_env = "WINDOWS_SIGNING_PASSWORD"
# timestamp_url = "YOUR_RFC3161_SERVICE_URL" # Optional
```

Install cargo-bundle with the feature, then package on a host supported by the selected backend. The `wxsmsi` format also needs `dotnet`:

```powershell
cargo install --path . --force --features windows-signing
$env:WINDOWS_SIGNING_PASSWORD = "value supplied by your secret store"
cargo bundle --release --format exe
cargo bundle --release --format msi
cargo bundle --release --format wxsmsi
```

The signer uses SHA-256 and requests an RFC 3161 timestamp only when
`timestamp_url` is set. It signs the generated EXE or MSI in place. Without a
password setting, it tries an empty password; a configured variable must exist.
Signing does not prompt. `wxsmsi` is signed after `dotnet build` produces its MSI.

Verify the final files on Windows with the Windows SDK SignTool:

```powershell
signtool.exe verify /pa /v .\target\release\bundle\exe\Example.exe
signtool.exe verify /pa /v .\target\release\bundle\msi\Example.msi
```

Use the actual output paths printed by cargo-bundle; WiX MSI names and directories differ from native MSI. Microsoft's [SignTool documentation](https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool) describes verification, and its [Authenticode timestamp guidance](https://learn.microsoft.com/en-us/windows/win32/seccrypto/time-stamping-authenticode-signatures) recommends RFC 3161 timestamps and SHA-256.
