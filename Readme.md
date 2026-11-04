# Cargo bundle

Wrap Rust executables in OS-specific app bundles.

<div badges="all">
<a href="https://crates.io/crates/cargo-bundle"><img src="https://raw.githubusercontent.com/philocalyst/cargo-bundle/master/.badges/crate.svg" alt="crates.io: cargo-bundle"></a>
<a href="https://github.com/philocalyst/cargo-bundle/blob/master/docs/src/index.md"><img src="https://raw.githubusercontent.com/philocalyst/cargo-bundle/master/.badges/guide.svg" alt="guide: mdBook"></a>
</div>

[![CI](https://github.com/philocalyst/cargo-bundle/actions/workflows/ci.yml/badge.svg)](https://github.com/philocalyst/cargo-bundle/actions/workflows/ci.yml)
[![Docs](https://github.com/philocalyst/cargo-bundle/actions/workflows/docs.yml/badge.svg)](https://github.com/philocalyst/cargo-bundle/actions/workflows/docs.yml)

`cargo-bundle` builds installers and app bundles from Cargo executables. It
supports macOS apps and disk images, Debian packages, AppImages, and Windows
executables. Windows installers and iOS bundles are experimental; RPM is not
implemented.

## Get started

Install the published release from [crates.io](https://crates.io/crates/cargo-bundle):

```sh
cargo install cargo-bundle --locked
```

In a Cargo package with a `[package.metadata.bundle]` table, run:

```sh
cargo bundle --release
```

This creates a macOS app, Debian package, or Windows executable for your
platform and prints its path. Use `--format` for another output. The
[first-bundle guide](docs/src/getting-started.md) walks through a complete
example; the [format guide](docs/src/formats.md) covers other outputs.

## Documentation and contributing

This book describes the current repository checkout; the crates.io release may
have different options. To install the checkout, run `cargo install --path .`
from its root. Browse the [guide](docs/src/index.md), [manifest reference](docs/src/manifest.md),
or [platform chapters](docs/src/platforms/index.md). For development and docs
checks, see [Contributing](CONTRIBUTING.md). Cargo-bundle welcomes contributions
from anyone; please follow the [Rust code of conduct](https://www.rust-lang.org/policies/code-of-conduct).

## License

This program is licensed under either the [Apache License 2.0](License_Apache.md)
or the [MIT License](License_MIT.md), at your option.
