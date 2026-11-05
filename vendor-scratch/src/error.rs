use std::io;
use std::path::{Path, PathBuf};
use std::str::Utf8Error;

/// A [`Result`](std::result::Result) whose error defaults to [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything that can go wrong while opening, unpacking, packing or mounting
/// an AppImage.
///
/// Every variant says what was being done and to which path, so an error can
/// be shown as it is:
///
/// ```
/// use appimage::{AppImage, Error, Operation};
///
/// let error = AppImage::open("/nonexistent.AppImage").unwrap_err();
/// let Error::Io { operation, path, .. } = &error else {
///     panic!("expected an I/O error, got {error}");
/// };
/// assert_eq!(*operation, Operation::Open);
/// assert_eq!(path.to_str(), Some("/nonexistent.AppImage"));
/// assert!(error.to_string().starts_with("cannot open /nonexistent.AppImage"));
/// ```
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A filesystem operation failed.
    #[error("cannot {operation} {}: {source}", path.display())]
    Io {
        /// What was being attempted.
        operation: Operation,
        /// The path it was attempted on.
        path: PathBuf,
        /// The operating system's answer.
        #[source]
        source: io::Error,
    },
    /// The file is not a type-2 AppImage: an ELF followed by a SquashFS.
    #[error("{}: not a type-2 AppImage: {reason}", path.display())]
    NotAppImage {
        /// The file that was inspected.
        path: PathBuf,
        /// The first thing found wrong with it.
        reason: Malformed,
    },
    /// The bytes converted into a [`Runtime`](crate::Runtime) are not a
    /// well-formed ELF executable.
    #[error("the runtime is not a well-formed ELF executable: {0}")]
    InvalidRuntime(Malformed),
    /// The running executable could not be located.
    #[error("cannot locate the running executable: {0}")]
    CurrentExecutable(#[source] io::Error),
    /// A section holds text, and the text is not UTF-8.
    #[error("the {section} section is not UTF-8: {source}")]
    SectionEncoding {
        /// The section's name.
        section: &'static str,
        /// Where the text stops being UTF-8.
        #[source]
        source: Utf8Error,
    },
    /// The SquashFS could not be read or written. This includes images that use
    /// a compressor whose feature (`xz`, `zstd`) is not enabled.
    #[error("squashfs: {0}")]
    Squashfs(#[from] backhand::BackhandError),
    /// A pattern given to [`Unpack::matching`](crate::Unpack::matching) is not a
    /// valid glob.
    #[error("invalid pattern: {0}")]
    Pattern(#[from] glob::PatternError),
    /// A path, link or file in an AppDir or a payload is not acceptable; see
    /// [`AppDirError`].
    #[error(transparent)]
    AppDir(#[from] AppDirError),
    /// A SquashFS block size must be a power of two from 4 KiB through 1 MiB.
    #[error("{0}: block size must be a power of two from 4096 through 1048576 bytes")]
    InvalidBlockSize(u32),
    /// A path that must be a directory is something else.
    #[error("{} is not a directory", .0.display())]
    NotADirectory(PathBuf),
    /// The kernel or the FUSE helper refused to mount.
    #[error("cannot mount on {}: {source}", mountpoint.display())]
    Mount {
        /// Where the mount was attempted.
        mountpoint: PathBuf,
        /// The reason given.
        #[source]
        source: io::Error,
    },
    /// The mount could not be taken down; it may still be in place.
    #[error("cannot unmount {}: {source}", mountpoint.display())]
    Unmount {
        /// Where the mount is.
        mountpoint: PathBuf,
        /// The reason given.
        #[source]
        source: io::Error,
    },
    /// Mounting is only implemented for Linux targets.
    #[error("mounting an AppImage requires a Linux target")]
    UnsupportedPlatform,
}

/// What an [`Error::Io`] was trying to do to its path.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum Operation {
    /// Open a file for reading.
    #[error("open")]
    Open,
    /// Read from a file.
    #[error("read")]
    Read,
    /// Create a file or directory.
    #[error("create")]
    Create,
    /// Write to a file.
    #[error("write to")]
    Write,
    /// Copy decompressed contents into a file.
    #[error("copy data into")]
    Copy,
    /// Delete a file or directory.
    #[error("remove")]
    Remove,
    /// Read a path's metadata.
    #[error("inspect")]
    Inspect,
    /// List a directory.
    #[error("list")]
    List,
    /// Read where a symbolic link points.
    #[error("read the link at")]
    ReadLink,
    /// Create a symbolic link.
    #[error("create a symbolic link at")]
    Symlink,
    /// Resolve a path to its canonical form.
    #[error("resolve")]
    Resolve,
    /// Move a staged file over its destination.
    #[error("replace")]
    Replace,
    /// Change a file's permission bits.
    #[error("set the permissions of")]
    SetPermissions,
    /// Change a file's modification time.
    #[error("set the modification time of")]
    SetModified,
}

/// What made a file fail to read as an ELF runtime followed by a payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum Malformed {
    /// The file does not start with an ELF identification.
    #[error("not an ELF file")]
    NotElf,
    /// A header, table or section reaches past the end of the file.
    #[error("a header, table or section extends past the end of the file")]
    Truncated,
    /// The ELF header names an unsupported class, byte order or version, or
    /// has an impossible size.
    #[error("unsupported or inconsistent ELF header")]
    Header,
    /// The section or program header table has an impossible shape.
    #[error("inconsistent section or program header table")]
    HeaderTable,
    /// The table of section names is missing, mistyped or indexed out of range.
    #[error("invalid section name table")]
    StringTable,
    /// The ELF header does not carry the type-2 marker, `AI\x02`.
    #[error("the type-2 marker is missing")]
    MissingMarker,
    /// No SquashFS image starts where the ELF runtime ends.
    #[error("no SquashFS image follows the runtime")]
    MissingPayload,
}

/// Why a path, link or file was refused.
///
/// An AppImage is untrusted input, so extraction never follows a link out of
/// the destination, and an AppDir is built from inputs that are checked before
/// anything is written.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum AppDirError {
    /// The path is empty, absolute, or climbs out with `..`.
    #[error("{}: expected a nonempty relative path without '..'", .0.display())]
    InvalidPath(PathBuf),
    /// A script needs the path as text, and this one is not UTF-8.
    #[error("{}: AppRun scripts require a UTF-8 executable path", .0.display())]
    NonUtf8ScriptPath(PathBuf),
    /// An application ID is the stem of a desktop file and an icon.
    #[error(
        "{0:?}: expected an application ID starting with a letter or digit, containing only letters, digits, '.', '_' or '-'"
    )]
    InvalidId(String),
    /// A desktop entry field would not survive being written.
    #[error("invalid desktop field {field}: {reason}")]
    DesktopField {
        /// The key concerned.
        field: &'static str,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// An icon must be a png, svg, svgz or xpm file.
    #[error("{}: unsupported icon extension (expected png, svg, svgz or xpm)", .0.display())]
    UnsupportedIcon(PathBuf),
    /// Only regular files, directories and links are copied or extracted.
    #[error("{}: expected a regular file or directory", .0.display())]
    UnsupportedFile(PathBuf),
    /// A symbolic link points out of the tree it lives in.
    #[error("{}: symlink target must stay inside the payload", .0.display())]
    EscapingSymlink(PathBuf),
    /// Following symbolic links never ends, or takes more than 40 steps.
    #[error("{}: symlink resolution exceeds 40 links", .0.display())]
    SymlinkLoop(PathBuf),
    /// One directory contains the other.
    #[error("source and destination directories overlap")]
    OverlappingDirectories,
    /// A required file is absent, or is not a regular file.
    #[error("{}: required AppDir file is missing or not a regular file", .0.display())]
    MissingFile(PathBuf),
    /// `AppRun` has no execute permission.
    #[error("{}: AppRun is not executable", .0.display())]
    NotExecutable(PathBuf),
    /// An AppDir has exactly one `.desktop` file at its root.
    #[error("expected one root desktop file, found {0}")]
    DesktopFileCount(usize),
}

/// Name the operation and path that an [`io::Error`] came from.
pub(crate) trait At<T> {
    fn at(self, operation: Operation, path: &Path) -> Result<T>;

    /// Like [`At::at`], for a path below an open directory that was reached as
    /// `root`. The two are only joined if there is an error to name.
    fn within(self, operation: Operation, root: &Path, relative: &Path) -> Result<T>;
}

impl<T> At<T> for io::Result<T> {
    fn at(self, operation: Operation, path: &Path) -> Result<T> {
        self.map_err(|source| Error::Io {
            operation,
            path: path.to_owned(),
            source,
        })
    }

    fn within(self, operation: Operation, root: &Path, relative: &Path) -> Result<T> {
        self.map_err(|source| Error::Io {
            operation,
            path: root.join(relative),
            source,
        })
    }
}
