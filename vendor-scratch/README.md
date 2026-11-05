# appimage-rs

Read, unpack, pack, and mount type-2 [AppImage]s in pure Rust. The crate needs no
C library to build and includes launcher source instead of prebuilt binaries.

```rust
use appimage::{AppDir, AppImage, Compression, Packer, Runtime};

// Inspect and unpack, on any platform.
let image = AppImage::open("Hello.AppImage")?;
println!("payload at byte {}", image.offset);
image.unpack().matching("usr/share/*").to("share")?;

// Pack, with a runtime you built (see below).
let runtime = Runtime::try_from(std::fs::read(
    "target/x86_64-unknown-linux-musl/runtime/appimage-runtime",
)?)?;
Packer::new(&AppDir::open("Hello.AppDir")?, &runtime)
    .compression(Compression::Lz4)
    .write_to("Hello-x86_64.AppImage")?;

// Mount, on Linux.
let mount = image.mount()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Building an AppDir

An `AppDir` can be assembled in code. Every write stays inside it, nothing is
overwritten, and a failed write is rolled back.

```rust
use appimage::{AppDir, AppId, AppPath, AppRun, DesktopCommand, DesktopEntry, Icon};
use std::path::Path;

let mut app = AppDir::create("MyApp.AppDir")?;
app.copy_file("target/release/myapp", AppPath::new("usr/bin/myapp")?)?
    .set_apprun(AppRun::Script)?;

let mut entry = DesktopEntry::new("My App", DesktopCommand::new("myapp"));
entry.categories = &["Utility"];
app.write_desktop_entry(&AppId::new("myapp")?, &entry, Icon::Png(Path::new("icon.png")))?;
app.validate()?;
# Ok::<(), appimage::Error>(())
```

## Hostile images

Unpacking treats the image as untrusted. Writes go through an open handle on the
destination, so no archived path and no link already on disk can reach outside
it. Links inside the archive are resolved before anything is written. Pass
`.existing(ExistingFiles::Replace)` to replace files through staged writes; a
copy that fails leaves the old file alone. Errors name the operation and the
path, and the reasons for refusing a file are enums (`Malformed`,
`AppDirError`).

The API documentation includes a tour and cookbook. The [examples](examples)
provide small command-line tools: `inspect`, `unpack`, `pack`, and `mount`.

## The runtime

An AppImage is a *runtime* executable with a SquashFS filesystem appended. Enable
the `launcher` feature and build `appimage-runtime` for the target architecture:

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --profile runtime --features launcher --bin appimage-runtime \
    --target x86_64-unknown-linux-musl
```

To use your own program as the runtime, call `appimage::launcher::init()` at the
start of `main` and pack it with `Runtime::current_exe()`. A copy with an
appended payload runs the app; otherwise, your program continues normally.

The launcher unpacks to a private temporary directory, runs `AppRun`, and cleans
up after it exits. It forwards signals and supports terminal job control. It
does not need FUSE, but requires enough temporary disk space and a filesystem
that permits execution. See the `launcher` module documentation for details.

## Features

| Feature    | Adds                                                              |
|------------|-------------------------------------------------------------------|
| `launcher` | the `launcher` module and the `appimage-runtime` binary           |
| `xz`       | reading xz-compressed images (pure Rust, `lzma-rs`)               |
| `zstd`     | reading zstd-compressed images (pure Rust, `ruzstd`)              |

## License

MIT. See [`LICENSE`](LICENSE).

[AppImage]: https://appimage.org/
