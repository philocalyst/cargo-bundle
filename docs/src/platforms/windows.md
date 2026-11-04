# Windows

`cargo bundle` offers three Windows formats: `exe` copies an executable, while `msi` and `wxsmsi` generate Windows Installer packages. The two MSI writers are experimental. The default is an executable copy; select an MSI format when you need an installer:

```sh
cargo bundle
cargo bundle --format msi
cargo bundle --format wxsmsi
```

Build a Windows target, for example with `--target x86_64-pc-windows-msvc`, when running from another operating system. The packaging format does not select the compilation target. Without `--format`, a Windows target selects `exe` and looks for the `.exe` binary produced by Cargo.

Output directories are below [Cargo's target directory](../cli.md#targets-and-output-directories). A main binary uses `<target-dir>/[<target-triple>/]<profile>/bundle/<format>/`; an example adds `examples/` before `bundle`. The target-triple component is omitted for a host build. The default development profile and `--profile dev` use `debug`; `--release` uses `release`, and other profile names use their own directory.

## Copy an executable

`--format exe` copies the built program to `bundle/exe/<binary-name>.exe`. It does not install the configured `resources` files beside the executable. On Windows, cargo-bundle writes PE version information using the bundle name, version, short description, copyright, authors, and binary name. It embeds an icon only from a configured SVG, rasterized to several Windows icon sizes. On another host, it copies the executable and skips PE resource editing, with a warning. This format does not create an installer. It signs the executable only when the optional [`windows-signing` feature and signing settings](../signing/windows.md) are enabled.

## Native MSI

`--format msi` writes `bundle/msi/<bundle-name>.msi`. The generated package places the executable in `Program Files/<bundle-name>` and configured resources below `Resources/`, preserving their mapped paths. It emits custom welcome, progress, removal, exit, cancel, and error dialogs. The MSI database writer is incomplete; treat the output as experimental and verify installation and removal on the Windows versions you support.

The MSI `ProductCode` is a UUID derived from the bundle `identifier`. The identifier is optional in Cargo metadata, but its fallback is empty for the main target and target-dependent for binaries and examples. Set an explicit, stable reverse-DNS identifier. The native writer reuses the same ProductCode when the identifier stays the same and does not generate upgrade metadata or a `RemoveExistingProducts` action; do not rely on it for upgrading an existing installation. It writes the configured version string directly to `ProductVersion`, so use a simple numeric MSI-compatible version such as `1.2.3`.

The writer copies the first configured `.ico` into an MSI `Icon` table stream; it does not convert other formats. That table entry is not connected to an Add/Remove Programs property or a shortcut, so do not assume Windows will display it as an installed-app icon. The generated package does not add Start Menu or Desktop shortcuts. Authenticode signing can be enabled for the MSI; see [Windows signing](../signing/windows.md).

### Install and remove an MSI

Test MSI output on a disposable Windows machine or VM. From PowerShell, use
[`msiexec`](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/msiexec)
to install with a verbose log:

```powershell
$msi = ".\path\to\MyApp.msi"  # Set this to the generated MSI path.
msiexec.exe /i $msi /L*V .\install.log
```

Check that the expected files and resources appear under `Program Files\<bundle-name>`, launch the executable, then remove the package and inspect the removal log:

```powershell
msiexec.exe /x $msi /L*V .\uninstall.log
```

Confirm the installed files are removed. For the native `msi` format, also verify that users can find and launch the app without Start Menu or Desktop shortcuts. Test an upgrade separately: install version A, build version B with the same identifier, and install B over A. The native writer does not author upgrade sequencing, so do not ship it as an upgrade path based on a successful fresh install alone. Run this check separately for `wxsmsi`; its generated `MajorUpgrade` element does not replace testing on the Windows versions you support.

## WiX MSI

`--format wxsmsi` writes `bundle/wxsmsi/installer.wixproj` and invokes `dotnet build` on it. The project pins both `WixToolset.Sdk` and `WixToolset.UI.wixext` to 6.0.2, so `dotnet` needs to restore those packages from NuGet.

Look under `bundle/wxsmsi/bin/<configuration>/` for the installer. Its filename comes from the bundle name, lowercased with non-alphanumeric characters replaced by hyphens: `My App` becomes `my-app.msi`. The configuration is `Release` for the `release` profile and `Debug` for every other profile.

The generated project describes the executable, DLL files beside it, and configured resources. It requests the WiX install-directory interface, license dialog, Start Menu and Desktop shortcuts, and an exit-dialog launch checkbox, initially checked. WiX converts the version to numeric components, strips any `-prerelease` or `+build` suffix, and uses at most four components. Cargo-bundle runs `dotnet build` and reports its expected MSI path on success without checking that the file exists; inspect the output and test the installer on Windows.

For WiX, run `cargo bundle` from the package directory and keep resources in a subdirectory. This puts the matched files under `assets/` in the install folder:

```toml
[package.metadata.bundle]
resources = ["assets/**"]
```

WiX preserves the matched directory tree instead of applying the resource-path mapping used by the other writers. A root-level resource such as `terms.txt` currently produces a component reference without a corresponding component, which can make the generated WiX project fail to link. Avoid absolute paths and `..` segments, which are not safely remapped in the WiX backend.

WiX uses the first matched icon path. ICO, EXE, and DLL files are passed through; other raster files are converted to ICO when the image decoder supports them. SVG conversion is not implemented. If the first matched path is unsupported or cannot be converted, later icon entries are not tried and the executable is used as the icon source.

The license RTF is generated at the profile's target directory. Cargo's `license-file` contents are used when configured; otherwise the Cargo `license` value itself is shown as text. If neither provides content, the writer searches conventional license filenames in the process working directory, then falls back to a short MIT-license statement. Set `license-file` if the installer should show the full license text.

The generated WiX source sets `Package/@Id` from the authors and bundle name and sets `UpgradeCode` from the manufacturer and bundle name. It does not set `ProductCode`; WiX 6 generates one for each build by default ([Package element](https://docs.firegiant.com/wix/schema/wxs/package/)). The source includes `MajorUpgrade`; the test steps above cover validating upgrades on Windows. When configured, the WiX backend signs the generated MSI after `dotnet build`; see [Windows signing](../signing/windows.md).
