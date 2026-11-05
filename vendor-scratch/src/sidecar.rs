use std::ffi::OsString;
use std::marker::PhantomData;
use std::ops::Deref;
use std::path::{Path, PathBuf};

use std::io;

use crate::error::{At, Operation};
use crate::{AppImage, Error, Result};

/// A kind of directory that lives next to an AppImage, named
/// `<AppImage>.<SUFFIX>`.
pub trait SidecarKind {
    /// The part after the AppImage's file name, without the dot.
    const SUFFIX: &'static str;
}

/// `<AppImage>.home`: a directory the AppImage uses as `$HOME`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Home {}

/// `<AppImage>.config`: a directory the AppImage uses as `$XDG_CONFIG_HOME`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Config {}

impl SidecarKind for Home {
    const SUFFIX: &'static str = "home";
}

impl SidecarKind for Config {
    const SUFFIX: &'static str = "config";
}

/// The path of a [`SidecarKind`] directory next to an AppImage.
///
/// Its kind is part of its type, so a function that takes a `Sidecar<Home>`
/// cannot be handed the config directory by mistake. Holding one says nothing
/// about whether the directory exists: [`Sidecar::existing`] asks, and
/// [`Sidecar::ensure`] makes it. Nothing is created unless you call that.
///
/// # Example
///
/// ```
/// use appimage::{AppDir, Home, Packer, Runtime, Sidecar};
///
/// let dir = tempfile::tempdir()?;
/// let app_dir = dir.path().join("App.AppDir");
/// std::fs::create_dir(&app_dir)?;
/// let image = Packer::new(&AppDir::open(&app_dir)?, &Runtime::inert())
///     .write_to(dir.path().join("App.AppImage"))?;
///
/// let home = Sidecar::<Home>::of(&image);
/// assert!(home.ends_with("App.AppImage.home"));
/// assert!(!home.exists());
///
/// assert_eq!(home.existing()?, None);
/// assert_eq!(home.ensure()?, &*home);
/// assert_eq!(home.existing()?, Some(&*home));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Sidecar<K> {
    path: PathBuf,
    kind: PhantomData<fn() -> K>,
}

impl<K: SidecarKind> Sidecar<K> {
    /// The sidecar of `image`.
    #[must_use]
    pub fn of(image: &AppImage) -> Self {
        let mut path = OsString::from(&image.path);
        path.push(".");
        path.push(K::SUFFIX);
        Self {
            path: path.into(),
            kind: PhantomData,
        }
    }

    /// The directory, if it exists.
    ///
    /// # Errors
    ///
    /// [`Error::NotADirectory`] if something else is at the path, because
    /// quietly ignoring it would run the AppImage with the wrong `$HOME`.
    pub fn existing(&self) -> Result<Option<&Path>> {
        match std::fs::metadata(&self.path) {
            Ok(metadata) if metadata.is_dir() => Ok(Some(&self.path)),
            Ok(_) => Err(Error::NotADirectory(self.path.clone())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).at(Operation::Inspect, &self.path),
        }
    }

    /// The directory, made if it does not exist.
    ///
    /// # Errors
    ///
    /// [`Error::NotADirectory`] as for [`Sidecar::existing`], and
    /// [`Error::Io`] if it cannot be made.
    pub fn ensure(&self) -> Result<&Path> {
        match std::fs::create_dir(&self.path) {
            Ok(()) => Ok(&self.path),
            // Someone else may have made it, or something else may be there.
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => self
                .existing()?
                .ok_or_else(|| Error::NotADirectory(self.path.clone())),
            Err(error) => Err(error).at(Operation::Create, &self.path),
        }
    }
}

impl<K> Deref for Sidecar<K> {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl<K> AsRef<Path> for Sidecar<K> {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl<K> From<Sidecar<K>> for PathBuf {
    fn from(sidecar: Sidecar<K>) -> Self {
        sidecar.path
    }
}
