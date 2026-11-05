use std::ops::Deref;
use std::path::Path;

#[cfg(target_os = "linux")]
use crate::error::{At, Operation};
use crate::{AppImage, Result};

impl AppImage {
    /// Mount the payload read-only on a new temporary directory.
    ///
    /// Dropping the [`Mount`] unmounts it and removes the directory.
    ///
    /// # Errors
    ///
    /// [`Error::UnsupportedPlatform`](crate::Error::UnsupportedPlatform) anywhere but Linux.
    /// [`Error::Mount`](crate::Error::Mount) if the kernel refuses, for instance
    /// without `/dev/fuse`, or without permission to mount.
    pub fn mount(&self) -> Result<Mount> {
        Mount::new(self, None)
    }

    /// Mount the payload read-only on `mountpoint`, which must be an existing
    /// empty directory. Dropping the [`Mount`] unmounts it and leaves the
    /// directory alone.
    ///
    /// # Errors
    ///
    /// As for [`AppImage::mount`].
    pub fn mount_at(&self, mountpoint: impl AsRef<Path>) -> Result<Mount> {
        Mount::new(self, Some(mountpoint.as_ref()))
    }
}

/// A live FUSE mount of an AppImage's payload; see [`AppImage::mount`].
///
/// Dereferences to the mountpoint's [`Path`]. The payload is served by a
/// background thread of this process, so it stays available exactly as long as
/// this value lives.
///
/// Mounting is implemented for Linux only, in pure Rust: `fuser` opens
/// `/dev/fuse` and calls `mount(2)` itself, so there is no libfuse to build or
/// link. A process without the privilege to mount is the one case where it
/// still asks the system's setuid `fusermount3` for the mount. On other
/// targets this type exists but cannot be constructed.
///
/// # Example
///
/// ```no_run
/// use appimage::AppImage;
///
/// let mount = AppImage::open("Hello.AppImage")?.mount()?;
/// let app_run = std::fs::read_to_string(mount.join("AppRun"))?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct Mount {
    // Unmounted by `Drop`, before the directory field goes away.
    #[cfg(target_os = "linux")]
    session: Option<fuser::BackgroundSession>,
    #[cfg(not(target_os = "linux"))]
    _never: std::convert::Infallible,
    mountpoint: Mountpoint,
}

#[derive(Debug)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
enum Mountpoint {
    Temp(tempfile::TempDir),
    Given(std::path::PathBuf),
}

impl Mount {
    #[cfg(target_os = "linux")]
    fn new(image: &AppImage, given: Option<&Path>) -> Result<Self> {
        let mountpoint = match given {
            Some(path) => Mountpoint::Given(path.to_owned()),
            None => Mountpoint::Temp(
                tempfile::Builder::new()
                    .prefix(".appimage-mount-")
                    .tempdir()
                    .at(Operation::Create, &std::env::temp_dir())?,
            ),
        };
        let path: &Path = match &mountpoint {
            Mountpoint::Temp(dir) => dir.path(),
            Mountpoint::Given(path) => path,
        };
        let session = Some(crate::fuse::serve(image, path)?);
        Ok(Self {
            session,
            mountpoint,
        })
    }

    #[cfg(not(target_os = "linux"))]
    fn new(_: &AppImage, _: Option<&Path>) -> Result<Self> {
        Err(crate::Error::UnsupportedPlatform)
    }

    /// Unmount now and report why it could not be.
    ///
    /// Dropping a [`Mount`] unmounts it too, but has nowhere to send a
    /// failure, and a mount that stays behind keeps its directory busy.
    ///
    /// # Errors
    ///
    /// [`Error::Unmount`](crate::Error::Unmount) if the filesystem is in use or the kernel refuses.
    #[cfg(target_os = "linux")]
    pub fn unmount(mut self) -> Result<()> {
        match self.session.take() {
            Some(session) => session
                .umount_and_join()
                .map_err(|source| crate::Error::Unmount {
                    mountpoint: self.to_path_buf(),
                    source,
                }),
            None => Ok(()),
        }
    }

    /// Unmount now and report why it could not be.
    ///
    /// # Errors
    ///
    /// Never constructed on this target, so never returns.
    #[cfg(not(target_os = "linux"))]
    pub fn unmount(self) -> Result<()> {
        match self._never {}
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some(session) = self.session.take() {
            // A destructor cannot report; `unmount` is the way to hear of this.
            let _ = session.umount_and_join();
        }
    }
}

impl Deref for Mount {
    type Target = Path;

    fn deref(&self) -> &Path {
        match &self.mountpoint {
            Mountpoint::Temp(dir) => dir.path(),
            Mountpoint::Given(path) => path,
        }
    }
}

impl AsRef<Path> for Mount {
    fn as_ref(&self) -> &Path {
        self
    }
}
