use std::path::{Path, PathBuf};

use crate::error::{At, Operation};
use crate::update::{BlankError, UpdateInformation};
use crate::{Error, Result, elf};

/// Names of the ELF sections the AppImage specification reserves.
pub mod section {
    /// Where updates come from; parsed by
    /// [`AppImage::update_information`](crate::AppImage::update_information).
    pub const UPDATE_INFO: &str = ".upd_info";
    /// A signature over the image.
    pub const SIGNATURE: &str = ".sha256_sig";
    /// The public key the signature verifies against.
    pub const SIGNING_KEY: &str = ".sig_key";
}

/// An opened type-2 AppImage.
///
/// An AppImage is an ELF executable with a SquashFS filesystem appended. Opening
/// one parses the ELF headers to find where the SquashFS begins; nothing else
/// is read until you ask. That makes `AppImage` a cheap, cloneable handle: a
/// path and an offset, both public.
///
/// What you do with it is one of:
///
/// * [`section`](Self::section) and
///   [`update_information`](Self::update_information) read from the ELF.
/// * [`unpack`](Self::unpack) extracts the payload to disk, on any platform.
/// * [`mount`](Self::mount) serves it through FUSE, on Linux.
/// * [`Sidecar`](crate::Sidecar) locates the `.home` and `.config` directories
///   that sit next to it.
///
/// To make one, see [`Packer`](crate::Packer).
///
/// # Example
///
/// ```
/// use appimage::{AppDir, AppImage, Error, Packer, Runtime};
///
/// let dir = tempfile::tempdir()?;
/// let app_dir = dir.path().join("App.AppDir");
/// std::fs::create_dir(&app_dir)?;
/// std::fs::write(app_dir.join("AppRun"), "#!/bin/sh\n")?;
/// Packer::new(&AppDir::open(&app_dir)?, &Runtime::inert()).write_to(dir.path().join("App.AppImage"))?;
///
/// let image = AppImage::open(dir.path().join("App.AppImage"))?;
/// assert_eq!(image.offset, 64); // just past `Runtime::inert()`
///
/// // Anything that is not an AppImage is rejected.
/// assert!(matches!(AppImage::open(&app_dir.join("AppRun")), Err(Error::NotAppImage { .. })));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub struct AppImage {
    /// The path this image was opened from.
    pub path: PathBuf,
    /// Byte offset of the SquashFS payload: the length of the ELF runtime.
    pub offset: u64,
}

impl AppImage {
    /// Open the AppImage at `path`.
    ///
    /// The path is made absolute, without following links, so the image can be
    /// found again after the working directory changes.
    ///
    /// # Errors
    ///
    /// [`Error::NotAppImage`](crate::Error::NotAppImage) unless the file is an
    /// ELF with a SquashFS right behind it; its `reason` says which part is
    /// missing. [`Error::Io`](crate::Error::Io) if it cannot be read.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let path = std::path::absolute(&path).at(Operation::Resolve, &path)?;
        let offset = elf::payload_offset(&path)?;
        Ok(Self { path, offset })
    }

    /// The contents of the ELF section called `name`; see [`section`] for the
    /// reserved ones.
    ///
    /// A section that is missing, empty or all NULs is `Ok(None)`: the
    /// reserved sections are padded out when unused.
    pub fn section(&self, name: &str) -> Result<Option<Vec<u8>>> {
        elf::section(&self.path, name)
    }

    /// The parsed [`section::UPDATE_INFO`] section, if the image has one.
    ///
    /// # Errors
    ///
    /// [`Error::SectionEncoding`] if the section is not UTF-8. A section with
    /// only padding in it is `Ok(None)`.
    pub fn update_information(&self) -> Result<Option<UpdateInformation>> {
        let Some(bytes) = self.section(section::UPDATE_INFO)? else {
            return Ok(None);
        };
        let text = std::str::from_utf8(&bytes).map_err(|source| Error::SectionEncoding {
            section: section::UPDATE_INFO,
            source,
        })?;
        match text.parse() {
            Ok(information) => Ok(Some(information)),
            // Padding with nothing in it is the same as no section.
            Err(BlankError) => Ok(None),
        }
    }
}

impl AsRef<Path> for AppImage {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}
