use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

use glob::Pattern;
use tempfile::TempDir;

use crate::error::{At, Operation, Result};
use crate::extract::{self, ExistingFiles, Options};
use crate::fs::DirectoryCleanup;
use crate::{AppDir, AppImage};

/// The file an unpacked AppImage is launched through.
pub const APP_RUN: &str = "AppRun";

impl AppImage {
    /// Start extracting the payload. Finish with [`Unpack::to`] or
    /// [`Unpack::to_temp`].
    pub fn unpack(&self) -> Unpack<'_> {
        Unpack {
            image: self,
            pattern: None,
            existing: ExistingFiles::default(),
        }
    }
}

/// An extraction that has not run yet; see [`AppImage::unpack`].
///
/// Extraction is safe to point at an untrusted image. Nothing is written
/// outside the destination, whatever the archive or the destination holds:
/// paths with `..` are refused, links that leave the tree are refused, and no
/// link is followed out of it. `setuid` and `setgid` bits are dropped and
/// ownership is not restored. A device node, FIFO or socket in the archive is
/// an error ([`AppDirError::UnsupportedFile`](crate::AppDirError::UnsupportedFile)).
///
/// Extraction is not a transaction: if it fails, the files already written
/// stay.
///
/// # Example
///
/// ```
/// use appimage::{AppDir, AppImage, ExistingFiles, Packer, Runtime};
///
/// let dir = tempfile::tempdir()?;
/// let source = dir.path().join("App.AppDir");
/// std::fs::create_dir_all(source.join("usr/bin"))?;
/// std::fs::write(source.join("usr/bin/hello"), "hi")?;
/// std::fs::write(source.join("AppRun"), "#!/bin/sh\n")?;
/// let image = Packer::new(&AppDir::open(&source)?, &Runtime::inert())
///     .write_to(dir.path().join("App.AppImage"))?;
///
/// // Everything, into a directory you keep.
/// let everything = image.unpack().to(dir.path().join("all"))?;
/// assert!(everything.join("usr/bin/hello").is_file());
///
/// // Again over the same files, which is refused unless you say otherwise.
/// assert!(image.unpack().to(dir.path().join("all")).is_err());
/// image.unpack().existing(ExistingFiles::Replace).to(dir.path().join("all"))?;
///
/// // Just what matches, into one that cleans up after itself.
/// let some = image.unpack().matching("usr/bin/*").to_temp()?;
/// assert!(some.join("usr/bin/hello").is_file());
/// assert!(!some.join(appimage::APP_RUN).exists());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use = "an extraction does nothing until `to` or `to_temp` is called"]
#[derive(Clone, Debug)]
pub struct Unpack<'a> {
    image: &'a AppImage,
    pattern: Option<String>,
    existing: ExistingFiles,
}

impl Unpack<'_> {
    /// Extract only the paths that match a glob such as `usr/lib/*`.
    ///
    /// The pattern is checked when the extraction runs, and a bad one is an
    /// [`Error::Pattern`](crate::Error::Pattern) before anything is written.
    pub fn matching(mut self, pattern: impl Into<String>) -> Self {
        self.pattern = Some(pattern.into());
        self
    }

    /// What to do with a file or link that is already there. The default is
    /// to fail.
    pub fn existing(mut self, existing: ExistingFiles) -> Self {
        self.existing = existing;
        self
    }

    /// Extract into `destination`, which is created if it does not exist.
    ///
    /// # Errors
    ///
    /// See [`Error`](crate::Error): a bad pattern, an unreadable image, an
    /// archive path or link that is refused, or a filesystem failure.
    pub fn to(self, destination: impl AsRef<Path>) -> Result<AppDir> {
        let (tree, cleanup) = self.run(destination.as_ref())?;
        cleanup.keep();
        Ok(tree)
    }

    /// Extract into a new temporary directory, which is deleted when the
    /// [`Scratch`] is dropped.
    ///
    /// If extraction fails, the directory is deleted right here.
    ///
    /// # Errors
    ///
    /// As for [`Unpack::to`], and [`Error::Io`](crate::Error::Io) if the
    /// directory cannot be made.
    pub fn to_temp(self) -> Result<Scratch> {
        let directory = tempfile::Builder::new()
            .prefix(".appimage-")
            .tempdir()
            .at(Operation::Create, &std::env::temp_dir())?;
        let (inner, cleanup) = self.run(directory.path())?;
        Ok(Scratch {
            inner,
            cleanup,
            directory,
        })
    }

    fn run(&self, destination: &Path) -> Result<(AppDir, DirectoryCleanup)> {
        let options = Options {
            pattern: self.pattern.as_deref().map(Pattern::new).transpose()?,
            existing: self.existing,
        };
        extract::extract(self.image, destination, &options)
    }
}

/// An unpacked payload in a temporary directory, deleted when it is dropped.
///
/// It dereferences to its [`AppDir`], so it can be added to and packed, and
/// from there to its [`Path`].
///
/// ```compile_fail,E0505
/// use appimage::{AppImage, AppPath, AppRun};
/// let image = AppImage::open("App.AppImage")?;
/// let mut scratch = image.unpack().to_temp()?;
/// let mut file = scratch.file(AppPath::new("usr/bin/app")?)?;
/// drop(scratch);
/// file.set_apprun(AppRun::Script)?;
/// # Ok::<(), appimage::Error>(())
/// ```
#[derive(Debug)]
pub struct Scratch {
    // Fields drop in this order: the directory handle closes, restrictive
    // modes are lifted through the handles kept for it, then the tree goes.
    inner: AppDir,
    cleanup: DirectoryCleanup,
    directory: TempDir,
}

impl Scratch {
    /// Keep the directory: stop it from being deleted and hand it over.
    #[must_use]
    pub fn persist(self) -> AppDir {
        let Self {
            inner,
            cleanup,
            directory,
        } = self;
        cleanup.keep();
        // Kept on purpose; the `AppDir` now says where it is.
        let _ = directory.keep();
        inner
    }

    /// Delete the directory now and report why it could not be.
    ///
    /// Dropping a [`Scratch`] deletes it too, but has nowhere to send a
    /// failure.
    ///
    /// # Errors
    ///
    /// [`Error::Io`](crate::Error::Io) with [`Operation::Remove`] if part of
    /// the tree could not be deleted.
    pub fn close(self) -> Result<()> {
        let Self {
            inner,
            cleanup,
            directory,
        } = self;
        drop((inner, cleanup));
        let path = directory.path().to_owned();
        directory.close().at(Operation::Remove, &path)
    }
}

impl Deref for Scratch {
    type Target = AppDir;

    fn deref(&self) -> &AppDir {
        &self.inner
    }
}

impl DerefMut for Scratch {
    fn deref_mut(&mut self) -> &mut AppDir {
        &mut self.inner
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.inner
    }
}

impl From<Scratch> for PathBuf {
    /// Keep the directory and take its path; see [`Scratch::persist`].
    fn from(scratch: Scratch) -> Self {
        scratch.persist().into()
    }
}
