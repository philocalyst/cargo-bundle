# Cargo bundle

Wrap Rust executables in OS-specific app bundles.

Use `cargo-bundle` to package executables built with Cargo as macOS apps or
disk images, Debian packages, AppImages, and Windows executables or installers.
iOS bundles and Windows installers are still experimental. RPM packaging is not implemented.

Start with [First bundle](getting-started.md): install the command, configure
a small Cargo project, and inspect the result on macOS, Linux, or Windows.

For an existing application:

- [Choose a format](formats.md) for its host requirements and limitations.
- Configure [bundle metadata](manifest.md), including settings for named binaries
  and examples, then add [icons and resource files](resources.md).
- Add [localizations and accessibility checks](accessibility-localization.md)
  for the languages and ways of interacting your application supports.
- Follow the [platform guide](platforms/index.md) to inspect the package, then
  [prepare a release](releasing.md).

The [command reference](cli.md) covers workspaces, profiles, and target paths.
[Troubleshooting](troubleshooting.md) starts from common errors and missing files.

The book describes this repository's code, which may differ from the release
on crates.io. Installation instructions distinguish the two. Cargo-bundle
supports [artifact signing](signing.md) and optional macOS notarization.
Provisioning and store submissions need additional release steps.
