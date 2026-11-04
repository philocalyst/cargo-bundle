# First bundle

To install cargo-bundle, run:

```sh
cargo install cargo-bundle --locked
cargo bundle --version
```

This installs the release on [crates.io](https://crates.io/crates/cargo-bundle)
as a Cargo subcommand. If you're working from a checkout of this repository,
use `cargo install --path .` instead. The book describes that checkout, so an
older release may not have all the options shown here. `cargo bundle --help`
will show what your installed version accepts.

## Give it something to bundle

Any Cargo package with a binary will do. We'll use the program Cargo creates
for a new project:

```sh
cargo new postcard
cd postcard
```

Add the following to `Cargo.toml`:

```toml
[package.metadata.bundle]
name = "Postcard"
identifier = "com.example.postcard"
short_description = "A small postcard application"
category = "Utility"
```

`name` is the application's display name; it needn't match the Cargo package
name. `identifier` should distinguish your application from everyone else's.
The usual convention is a domain name in reverse order, followed by the app
name. Replace `com.example` with a domain you control before distributing an app.

## Build the bundle

On macOS:

```sh
cargo bundle --release
open target/release/bundle/osx/Postcard.app
```

There's no window to see yet: this is still Cargo's “Hello, world!” program.
Opening an app from Finder doesn't give it a terminal. You can run the copy
inside the bundle to check that it works:

```sh
target/release/bundle/osx/Postcard.app/Contents/MacOS/postcard
```

On Linux, the same command produces a Debian package:

```sh
cargo bundle --release
```

The package will be at `target/release/bundle/deb/postcard_0.1.0_amd64.deb`
on x86-64, or `postcard_0.1.0_arm64.deb` on ARM64. The program is still a
terminal application. To inspect the package without installing it, see the
[Debian package checks](platforms/linux.md#debian-packages).

On Windows, start with an executable copy:

```sh
cargo bundle --release
```

You'll find it at `target/release/bundle/exe/postcard.exe`. Cargo-bundle also
prints the paths it produced, which is useful when you're building for a
particular target or using a different Cargo target directory.

Run the copy from PowerShell to see `Hello, world!`:

```powershell
.\target\release\bundle\exe\postcard.exe
```

Cargo-bundle chooses one default output for your platform. To make a different
artifact, select it explicitly—for example, `--format dmg` on macOS or
`--format appimage` on Linux. See [Choose a format](formats.md).

## Where to go from here

To package your own application, add [icons and resources](resources.md) to
its bundle table. The [platform chapters](platforms/index.md) cover tools and
settings specific to each OS. For example, `resources` goes in
`[package.metadata.bundle]`, but `use_terminal` goes in
`[package.metadata.bundle.linux]`.

## Try the repository examples

In a checkout of this repository, the `hello` example opens a blank window;
`goodbye` prints `Goodbye, World!` to a terminal. Their icons and bundle metadata
are already included. Run these from the repository root, on the host shown:

```sh
cargo bundle --example hello --release
```

Use `--example goodbye` instead to try the terminal program. The default output
follows your host platform.

Their example settings in
[Cargo.toml](https://github.com/philocalyst/cargo-bundle/blob/master/Cargo.toml)
show SVG and PNG icons, Linux translations, and AppStream configuration.
`hello` also includes a [DMG background example](platforms/macos.md#disk-images).
The `goodbye` program prints to standard output; launching it from a Linux
desktop needs `linux.use_terminal = true` if you want a terminal window.
