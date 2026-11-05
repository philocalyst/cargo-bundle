//! The program that runs when you execute an AppImage.
//!
//! A type-2 AppImage is a *runtime*, an ordinary executable, with a SquashFS
//! glued on behind it. This module is that runtime, written in Rust against the
//! rest of this crate, so there is no C runtime to download and no binary blob
//! to trust. There are two ways to get one.
//!
//! # A standalone runtime
//!
//! The `appimage-runtime` binary is nothing but [`main`]. Build it small and
//! static for the architecture you are packing for, and hand it to the packer:
//!
//! ```text
//! $ cargo build --profile runtime --features launcher --bin appimage-runtime \
//!       --target x86_64-unknown-linux-musl
//! ```
//!
//! ```no_run
//! use appimage::{AppDir, Packer, Runtime};
//!
//! let runtime = Runtime::try_from(std::fs::read(
//!     "target/x86_64-unknown-linux-musl/runtime/appimage-runtime",
//! )?)?;
//! Packer::new(&AppDir::open("Hello.AppDir")?, &runtime).write_to("Hello-x86_64.AppImage")?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Your own program as the runtime
//!
//! Call [`init`] first thing in `main`, and your program is also its own
//! runtime: [`Runtime::current_exe`](crate::Runtime::current_exe) packs a copy
//! of it, and when that copy starts it finds the payload appended to itself and
//! runs the app instead of your `main`. No one has to ship a second executable.
//!
//! ```no_run
//! # #[cfg(feature = "launcher")]
//! fn main() {
//!     appimage::launcher::init(); // returns unless we are carrying a payload
//!     // ... the program as usual ...
//! }
//! # #[cfg(not(feature = "launcher"))]
//! # fn main() {}
//! ```
//!
//! # What it does
//!
//! Unlike the official runtime, this one never mounts: it unpacks the payload
//! into a private temporary directory, runs `AppRun` there, and deletes the
//! directory when `AppRun` exits. That needs neither FUSE nor a helper
//! program, but it does need disk space for the unpacked app and a temporary
//! filesystem that allows executables.
//!
//! `AppRun` is started without a shell, in its own process group, in the
//! caller's working directory, with the caller's arguments and
//! environment plus:
//!
//! | Variable   | Value                                  |
//! |------------|----------------------------------------|
//! | `APPIMAGE` | absolute path of the AppImage          |
//! | `APPDIR`   | the temporary directory it unpacked to |
//! | `OWD`      | the working directory it was started in |
//!
//! Termination and job-control signals are forwarded to `AppRun`'s process
//! group, `AppRun` gets the foreground of the terminal while it runs, and
//! stopping or resuming the AppImage from the shell stops and resumes the app.
//! The exit status is `AppRun`'s, or `128 + n` if it died of signal `n`.
//!
//! These options are handled by the runtime itself and never reach the app:
//!
//! * `--appimage-help`, `--appimage-version`
//! * `--appimage-offset`: print the byte offset of the SquashFS
//! * `--appimage-extract [PATTERN]`: unpack to `./squashfs-root`, optionally
//!   only paths matching a glob
//! * `--appimage-extract-and-run`: accepted for compatibility with the
//!   official runtime; it is dropped, since this runtime always extracts

#![allow(clippy::needless_doctest_main)]

mod job;

use std::borrow::Cow;
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{self, Command, ExitCode, ExitStatus};

use crate::{APP_RUN, AppImage, ExistingFiles};

const RUNTIME_NAME: &str = "appimage-runtime";
const EXTRACTION_DIRECTORY: &str = "squashfs-root";
const SIGNAL_EXIT_OFFSET: i32 = 128;
const HELP: &str = "\
Usage: APPIMAGE [OPTIONS] [ARGUMENTS...]

Run this AppImage, or use one of these runtime options:
  --appimage-help                 Show this help
  --appimage-version              Show runtime version
  --appimage-offset               Print the SquashFS payload byte offset
  --appimage-extract [PATTERN]    Extract to ./squashfs-root (optional glob)
  --appimage-extract-and-run      Extract to a temporary directory and run";

/// Probe the current executable before interpreting any host arguments.
/// If it opens as an AppImage, run the requested operation and exit.
/// If the probe fails, return control to the host program.
pub fn init() {
    if let Ok(image) = current_image() {
        let status = runtime_status(env!("CARGO_PKG_NAME"), || Ok(image));
        process::exit(i32::from(status));
    }
}

/// Entry point for a standalone runtime. Help and version work without a payload.
pub fn main() -> ExitCode {
    ExitCode::from(runtime_status(RUNTIME_NAME, current_image))
}

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error("cannot locate the running executable: {0}")]
    Executable(#[source] io::Error),
    #[error("cannot locate the working directory: {0}")]
    WorkingDirectory(#[source] io::Error),
    #[error("the extraction pattern must be valid UTF-8")]
    InvalidPatternEncoding,
    #[error(transparent)]
    Image(#[from] crate::Error),
    #[error("the payload has no regular {APP_RUN} file")]
    NoAppRun,
    #[error("cannot inspect {APP_RUN}: {0}")]
    InspectEntryPoint(#[source] io::Error),
    #[error(transparent)]
    Job(#[from] job::Error),
    #[error("the job returned an unexpected process status: {0}")]
    UnexpectedStatus(ExitStatus),
    #[error("cannot write runtime output: {0}")]
    Output(#[source] io::Error),
}

fn runtime_status(name: &str, open_image: impl FnOnce() -> Result<AppImage, Failure>) -> u8 {
    match execute(env::args_os().skip(1), open_image) {
        Ok(status) => status,
        Err(error) => {
            let _ = writeln!(io::stderr().lock(), "{name}: {error}");
            1
        }
    }
}

fn current_image() -> Result<AppImage, Failure> {
    let path = env::current_exe().map_err(Failure::Executable)?;
    Ok(AppImage::open(path)?)
}

fn execute(
    arguments: impl Iterator<Item = OsString>,
    open_image: impl FnOnce() -> Result<AppImage, Failure>,
) -> Result<u8, Failure> {
    let mut arguments = arguments.peekable();
    match arguments.peek().and_then(|argument| argument.to_str()) {
        Some("--appimage-help") => write_line(HELP)?,
        Some("--appimage-version") => {
            write_line(format_args!("{RUNTIME_NAME} {}", env!("CARGO_PKG_VERSION")))?;
        }
        Some("--appimage-offset") => write_line(open_image()?.offset)?,
        Some("--appimage-extract") => {
            arguments.next();
            let pattern = extraction_pattern(arguments.next())?;
            extract(&open_image()?, pattern.as_deref())?;
        }
        option => {
            if option == Some("--appimage-extract-and-run") {
                arguments.next();
            }
            return launch(&open_image()?, arguments);
        }
    }
    Ok(0)
}

fn write_line(message: impl fmt::Display) -> Result<(), Failure> {
    let mut output = io::stdout().lock();
    writeln!(output, "{message}")
        .and_then(|()| output.flush())
        .map_err(Failure::Output)
}

fn extraction_pattern(argument: Option<OsString>) -> Result<Option<String>, Failure> {
    let Some(argument) = argument else {
        return Ok(None);
    };
    let pattern = argument
        .into_string()
        .map_err(|_| Failure::InvalidPatternEncoding)?;
    Ok((!pattern.starts_with('-')).then_some(pattern))
}

fn extract(image: &AppImage, pattern: Option<&str>) -> Result<(), Failure> {
    let target = env::current_dir()
        .map_err(Failure::WorkingDirectory)?
        .join(EXTRACTION_DIRECTORY);
    // Extracting again over an earlier extraction is how it is repeated.
    let mut unpack = image.unpack().existing(ExistingFiles::Replace);
    if let Some(pattern) = pattern {
        unpack = unpack.matching(pattern);
    }
    unpack.to(&target)?;
    write_line(format_args!(
        "AppImage payload extracted to {}",
        target.display()
    ))
}

fn launch(image: &AppImage, arguments: impl IntoIterator<Item = OsString>) -> Result<u8, Failure> {
    let working_directory = env::current_dir().map_err(Failure::WorkingDirectory)?;
    let payload = image.unpack().to_temp()?;
    let directory = absolute_directory(&payload, &working_directory);
    let mut command = Command::new(entry_point(&directory)?);
    command
        .args(arguments)
        .env("APPIMAGE", &image.path)
        .env("APPDIR", directory.as_ref())
        .env("OWD", working_directory);
    // Inherit the actual working directory, even if its name changes before spawn.
    // The payload guard stays in scope until the job returns.
    exit_code(job::run(&mut command)?)
}

fn absolute_directory<'path>(directory: &'path Path, working_directory: &Path) -> Cow<'path, Path> {
    if directory.is_absolute() {
        Cow::Borrowed(directory)
    } else {
        Cow::Owned(working_directory.join(directory))
    }
}

fn entry_point(directory: &Path) -> Result<PathBuf, Failure> {
    let path = directory.join(APP_RUN);
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => Ok(path),
        Ok(_) => Err(Failure::NoAppRun),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(Failure::NoAppRun),
        Err(error) => Err(Failure::InspectEntryPoint(error)),
    }
}

fn exit_code(status: ExitStatus) -> Result<u8, Failure> {
    status
        .code()
        .or_else(|| status.signal().map(|signal| SIGNAL_EXIT_OFFSET + signal))
        .and_then(|code| u8::try_from(code).ok())
        .ok_or(Failure::UnexpectedStatus(status))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn informational_options_need_neither_an_image_nor_another_argument() {
        for option in ["--appimage-help", "--appimage-version"] {
            let arguments = std::iter::once(OsString::from(option))
                .chain(std::iter::from_fn(|| panic!("read beyond {option}")));
            assert_eq!(
                execute(arguments, || panic!("opened image for {option}")).unwrap(),
                0
            );
        }
    }

    #[test]
    fn extraction_interprets_one_optional_text_pattern() {
        for (argument, expected) in [
            (None, None),
            (Some(""), Some("")),
            (Some("usr/bin/*"), Some("usr/bin/*")),
            (Some("--other"), None),
        ] {
            assert_eq!(
                extraction_pattern(argument.map(OsString::from))
                    .unwrap()
                    .as_deref(),
                expected
            );
        }

        for bytes in [&b"\xff"[..], b"-\xff"] {
            let arguments = [
                "--appimage-extract".into(),
                OsString::from_vec(bytes.to_vec()),
            ];
            let result = execute(arguments.into_iter(), || {
                panic!("opened image for invalid pattern")
            });
            assert!(matches!(result, Err(Failure::InvalidPatternEncoding)));
        }
    }

    #[test]
    fn entry_point_follows_links_and_preserves_lookup_errors() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path();
        let path = directory.join(APP_RUN);
        assert!(matches!(entry_point(directory), Err(Failure::NoAppRun)));
        fs::create_dir(&path).unwrap();
        assert!(matches!(entry_point(directory), Err(Failure::NoAppRun)));
        fs::remove_dir(&path).unwrap();

        symlink("application", &path).unwrap();
        assert!(matches!(entry_point(directory), Err(Failure::NoAppRun)));
        fs::write(directory.join("application"), b"").unwrap();
        assert_eq!(entry_point(directory).unwrap(), path);

        fs::remove_file(&path).unwrap();
        symlink(APP_RUN, &path).unwrap();
        assert!(matches!(
            entry_point(directory),
            Err(Failure::InspectEntryPoint(_))
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn wait_status_preserves_every_exit_code_and_rejects_a_stopped_job() {
        for code in u8::MIN..=u8::MAX {
            let status = ExitStatus::from_raw(libc::W_EXITCODE(i32::from(code), 0));
            assert_eq!(exit_code(status).unwrap(), code);
        }
        let stopped = ExitStatus::from_raw(libc::W_STOPCODE(libc::SIGSTOP));
        assert!(
            matches!(exit_code(stopped), Err(Failure::UnexpectedStatus(status)) if status == stopped)
        );
    }

    #[test]
    fn fatal_signal_uses_the_shell_status() {
        let status = Command::new("/bin/sh")
            .args(["-c", "kill -KILL $$"])
            .status()
            .unwrap();
        assert_eq!(exit_code(status).unwrap(), 137);
    }

    fn pack_script(directory: &Path, script: &str) -> AppImage {
        let source = directory.join("Example.AppDir");
        fs::create_dir(&source).unwrap();
        let entry = source.join(APP_RUN);
        fs::write(&entry, script).unwrap();
        fs::set_permissions(entry, fs::Permissions::from_mode(0o755)).unwrap();
        crate::Packer::new(
            &crate::AppDir::open(&source).unwrap(),
            &crate::Runtime::inert(),
        )
        .write_to(directory.join("Example.AppImage"))
        .unwrap()
    }

    #[test]
    fn packed_application_observes_its_paths_and_exact_arguments() {
        let _turn = job::SIGNAL_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let temporary = tempfile::tempdir().unwrap();
        let image = pack_script(
            temporary.path(),
            r#"#!/bin/sh
set -eu
[ "$APPIMAGE" = "$1" ]
[ "$OWD" -ef . ]
[ "$APPDIR/AppRun" -ef "$0" ]
record=$2
shift 2
printf '%s\000' "$APPDIR" "$@" > "$record"
exit 23
"#,
        );
        let record = temporary.path().join("observed arguments");
        let native_arguments = [
            &b""[..],
            b"two words",
            b"\xff",
            b"--appimage-help",
            b"--appimage-extract-and-run",
        ];
        let arguments = std::iter::once(OsString::from("--appimage-extract-and-run"))
            .chain([&image.path, &record].map(|path| path.as_os_str().to_owned()))
            .chain(native_arguments.map(|bytes| OsStr::from_bytes(bytes).to_owned()));
        assert_eq!(execute(arguments, || Ok(image)).unwrap(), 23);

        let contents = fs::read(record).unwrap();
        let mut fields = contents
            .strip_suffix(&[0])
            .unwrap()
            .split(|byte| *byte == 0);
        let directory = Path::new(OsStr::from_bytes(fields.next().unwrap()));
        assert!(directory.is_absolute());
        assert!(
            !directory.try_exists().unwrap(),
            "payload survived application exit"
        );
        assert_eq!(fields.collect::<Vec<_>>(), native_arguments);
    }
}
