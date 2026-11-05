//! Read, unpack, pack and mount type-2 [AppImage]s in pure Rust.
//!
//! An AppImage is an ELF executable with a SquashFS filesystem appended, so
//! everything here is a view of those two halves. Here is a quick example that
//! packs a directory, reopens the result, and unpacks part of it:
//!
//! ```
//! use appimage::{AppDir, AppImage, Packer, Runtime};
//!
//! let dir = tempfile::tempdir()?;
//! let app_dir = dir.path().join("Hello.AppDir");
//! std::fs::create_dir_all(app_dir.join("usr/bin"))?;
//! std::fs::write(app_dir.join("usr/bin/hello"), "#!/bin/sh\necho hello\n")?;
//! std::fs::write(app_dir.join("AppRun"), "#!/bin/sh\nexec usr/bin/hello\n")?;
//!
//! let app_dir = AppDir::open(&app_dir)?;
//! let image = Packer::new(&app_dir, &Runtime::inert()).write_to(dir.path().join("Hello.AppImage"))?;
//! let image = AppImage::open(&image.path)?;
//!
//! let bin = image.unpack().matching("usr/bin/*").to_temp()?;
//! assert!(bin.join("usr/bin/hello").is_file());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! [AppImage]: https://appimage.org/
//!
//! # Overview
//!
//! The primary type in this crate is [`AppImage`]: a cheap handle on an opened
//! image, from which everything else starts.
//!
//! * [`AppImage::unpack`] begins an [`Unpack`], which ends in an [`AppDir`] you
//!   keep or a [`Scratch`] that deletes itself.
//! * [`AppImage::mount`] serves the payload through FUSE as a [`Mount`].
//! * [`AppImage::update_information`] and [`AppImage::section`] read the ELF.
//! * [`Sidecar`] locates the [`Home`] and [`Config`] directories that sit
//!   beside the image.
//!
//! Going the other way, an [`AppDir`] is built up from files, directories and a
//! typed [`DesktopEntry`], and a [`Packer`] turns it and a [`Runtime`] into an
//! `AppImage`. The runtime is the executable that runs when someone launches the
//! file; this crate doesn't bundle one, and instead *is* one. See [`launcher`]
//! (behind the `launcher` crate feature).
//!
//! The remainder of this documentation is organized as follows:
//!
//! * [Features](#features) summarizes what the crate does and does not do.
//! * [Usage](#usage) shows how to add it to your project.
//! * [Examples](#examples) is a small cookbook.
//! * [Crate features](#crate-features) documents the Cargo features.
//!
//! # Features
//!
//! * Pure Rust, including every compressor and the FUSE implementation. There is
//!   no C library to build, link or find at runtime.
//! * Opening reads only ELF headers, however large the image.
//! * Unpacking works on every platform, and treats the image as hostile. Writes go
//!   through an open handle on the destination, so no path in the archive and no
//!   link already on disk can reach outside it; links inside the archive are
//!   resolved before anything is written; `setuid` and `setgid` are dropped.
//!   Each refusal is a distinct [`AppDirError`].
//! * Building an AppDir is checked the same way: paths cannot leave it, desktop
//!   entries cannot be made to inject keys, files are never overwritten, and a
//!   failed write is rolled back.
//! * Packing visits files in name order, writes the image beside its destination,
//!   and moves it into place only once it reads back as an AppImage.
//! * Every error says what was being done and to which path ([`Error`]); the
//!   reasons for refusing a file are enums ([`Malformed`], [`AppDirError`]).
//! * A complete AppImage launcher in [`launcher`], with signal forwarding and
//!   terminal job control, that can be built static for any Linux target or
//!   embedded in your own program.
//! * Typed update information ([`UpdateInformation`]) that round-trips unknown
//!   transports instead of discarding them.
//! * gzip, LZ4 and LZO are always available for reading; xz and zstd are opt-in.
//!   Packing supports gzip, LZ4, LZO and uncompressed.
//!
//! Things this crate doesn't do:
//!
//! * Verify signatures. [`section::SIGNATURE`] and [`section::SIGNING_KEY`] give
//!   you the bytes.
//! * Apply updates. [`UpdateInformation`] says where they would come from.
//! * Run an app *inside* the process. [`launcher`] starts `AppRun` as a child.
//! * Mount anywhere but Linux.
//! * Restore ownership, or extract device nodes, FIFOs or sockets; the last
//!   three are an error.
//!
//! # Usage
//!
//! Add it with `cargo add appimage-rs`. The library is called `appimage`:
//!
//! ```text
//! $ cargo new appimage-example
//! $ cd appimage-example
//! $ cargo add appimage-rs
//! ```
//!
//! Then replace `src/main.rs` with:
//!
//! ```no_run
//! use appimage::AppImage;
//!
//! fn main() -> Result<(), appimage::Error> {
//!     let image = AppImage::open("Hello.AppImage")?;
//!     println!("payload at byte {}", image.offset);
//!     Ok(())
//! }
//! ```
//!
//! # Examples
//!
//! * [Read update information](#read-update-information)
//! * [Unpack to a directory you keep](#unpack-to-a-directory-you-keep)
//! * [Unpack just what you need](#unpack-just-what-you-need)
//! * [Run from a temporary directory](#run-from-a-temporary-directory)
//! * [Build an AppDir](#build-an-appdir)
//! * [Pack an AppDir](#pack-an-appdir)
//! * [Make your own program an AppImage runtime](#make-your-own-program-an-appimage-runtime)
//! * [Mount an AppImage](#mount-an-appimage)
//!
//! ## Read update information
//!
//! The `.upd_info` section says where new versions come from. Unknown transports
//! are kept as they are.
//!
//! ```no_run
//! use appimage::{AppImage, UpdateInformation};
//!
//! let image = AppImage::open("Subsurface.AppImage")?;
//! match image.update_information()? {
//!     Some(UpdateInformation::GitHubReleases { username, repository, .. }) => {
//!         println!("updates from github.com/{username}/{repository}");
//!     }
//!     Some(other) => println!("updates via {}", other.transport()),
//!     None => println!("no update information"),
//! }
//! # Ok::<(), appimage::Error>(())
//! ```
//!
//! ## Unpack to a directory you keep
//!
//! ```no_run
//! use appimage::{APP_RUN, AppImage};
//!
//! let app_dir = AppImage::open("Hello.AppImage")?.unpack().to("Hello.AppDir")?;
//! assert!(app_dir.join(APP_RUN).is_file());
//! # Ok::<(), appimage::Error>(())
//! ```
//!
//! ## Unpack just what you need
//!
//! The pattern is a glob over payload-relative paths, and `*` crosses `/`.
//!
//! ```no_run
//! use appimage::AppImage;
//!
//! let image = AppImage::open("Hello.AppImage")?;
//! image.unpack().matching("usr/share/applications/*.desktop").to("desktop-files")?;
//! # Ok::<(), appimage::Error>(())
//! ```
//!
//! ## Run from a temporary directory
//!
//! A [`Scratch`] is deleted when it goes out of scope, even if the app made some of
//! its directories read-only. [`Scratch::close`] does the same and tells you if
//! it failed.
//!
//! ```no_run
//! use std::process::Command;
//! use appimage::{APP_RUN, AppImage};
//!
//! let scratch = AppImage::open("Hello.AppImage")?.unpack().to_temp()?;
//! Command::new(scratch.join(APP_RUN)).status()?;
//! // `scratch` is deleted here.
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Build an AppDir
//!
//! An [`AppDir`] is created empty, filled, and checked. Each call writes inside
//! the directory and refuses to replace anything. [`AppDirFile`] borrows the
//! directory, so the compiler makes sure a file is finished with before the next
//! one is touched.
//!
//! ```no_run
//! use appimage::{AppDir, AppId, AppPath, AppRun, DesktopCommand, DesktopEntry, Icon};
//! use std::path::Path;
//!
//! let mut app = AppDir::create("MyApp.AppDir")?;
//! app.copy_file("target/release/myapp", AppPath::new("usr/bin/myapp")?)?
//!     .set_apprun(AppRun::Script)?;
//!
//! let mut entry = DesktopEntry::new("My App", DesktopCommand::new("myapp"));
//! entry.categories = &["Utility"];
//! app.write_desktop_entry(&AppId::new("myapp")?, &entry, Icon::Png(Path::new("icon.png")))?;
//! app.validate()?;
//! # Ok::<(), appimage::Error>(())
//! ```
//!
//! [`AppDir::copy_directory`] brings in a whole tree, keeping links that stay
//! inside it and refusing ones that do not.
//!
//! ## Pack an AppDir
//!
//! The [`Runtime`] is the executable that will run the AppImage. Build the
//! `appimage-runtime` binary for the architecture you target (see
//! [`launcher`]) and read it in:
//!
//! ```no_run
//! use appimage::{AppDir, BlockSize, Compression, Packer, Runtime};
//!
//! let runtime = Runtime::try_from(std::fs::read(
//!     "target/x86_64-unknown-linux-musl/runtime/appimage-runtime",
//! )?)?;
//! Packer::new(&AppDir::open("Hello.AppDir")?, &runtime)
//!     .compression(Compression::Lz4)
//!     .block_size(BlockSize::new(64 * 1024)?)
//!     .write_to("Hello-x86_64.AppImage")?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Make your own program an AppImage runtime
//!
//! Call [`launcher::init`] first in `main`, and the program can pack copies of
//! itself:
//!
//! ```no_run
//! use appimage::{AppDir, Packer, Runtime};
//!
//! # #[cfg(feature = "launcher")]
//! fn main() -> Result<(), appimage::Error> {
//!     appimage::launcher::init(); // does not return if we are an AppImage
//!     Packer::new(&AppDir::open("Hello.AppDir")?, &Runtime::current_exe()?)
//!         .write_to("Hello.AppImage")?;
//!     Ok(())
//! }
//! # #[cfg(not(feature = "launcher"))]
//! # fn main() {}
//! ```
//!
//! ## Mount an AppImage
//!
//! The mount lives as long as the [`Mount`] does. Linux only. The kernel is
//! asked directly with `mount(2)`; a user without that privilege goes through
//! the system's setuid `fusermount3` instead, which `fuser` runs on its behalf.
//!
//! ```no_run
//! use appimage::AppImage;
//!
//! let mount = AppImage::open("Hello.AppImage")?.mount()?;
//! let app_run = std::fs::read_to_string(mount.join("AppRun"))?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Crate features
//!
//! * **launcher** -
//!   Enables the [`launcher`] module and the `appimage-runtime` binary, and
//!   depends on `signal-hook`. Unix only.
//! * **xz** -
//!   Reads SquashFS images compressed with xz, using the pure-Rust `lzma-rs`.
//! * **zstd** -
//!   Reads SquashFS images compressed with zstd, using the pure-Rust `ruzstd`.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

mod appdir;
mod codec;
mod desktop;
mod elf;
mod error;
mod extract;
mod fs;
mod image;
#[cfg(feature = "launcher")]
#[cfg_attr(docsrs, doc(cfg(feature = "launcher")))]
pub mod launcher;
mod mount;
mod pack;
mod runtime;
mod sidecar;
mod unpack;
pub mod update;

#[cfg(target_os = "linux")]
mod fuse;

pub use appdir::{AppDir, AppDirFile, AppPath, AppRun, Icon};
pub use desktop::{AppId, DesktopCommand, DesktopEntry, FileArgument};
pub use error::{AppDirError, Error, Malformed, Operation, Result};
pub use extract::ExistingFiles;
pub use image::{AppImage, section};
pub use mount::Mount;
pub use pack::{BlockSize, Compression, Packer};
pub use runtime::Runtime;
pub use sidecar::{Config, Home, Sidecar, SidecarKind};
pub use unpack::{APP_RUN, Scratch, Unpack};
pub use update::UpdateInformation;
