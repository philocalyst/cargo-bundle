//! Filesystem operations confined to an open directory.
//!
//! Everything here goes through a [`Dir`] handle, never through a path that
//! the operating system would resolve again, so a symbolic link swapped in
//! behind our back cannot send a write somewhere else.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use cap_std::fs::{Dir, File, OpenOptions, Permissions};

use crate::AppDir;
use crate::error::{At, Operation, Result};

/// Directory handles kept so that restrictive payload modes cannot get in the
/// way of deleting the tree.
#[derive(Debug, Default)]
pub(crate) struct DirectoryCleanup {
    pub(crate) directories: Vec<std::fs::File>,
}

impl DirectoryCleanup {
    pub(crate) fn keep(mut self) {
        self.directories.clear();
    }
}

impl Drop for DirectoryCleanup {
    fn drop(&mut self) {
        #[cfg(unix)]
        for directory in &self.directories {
            use std::os::unix::fs::PermissionsExt;
            // A destructor cannot report. Whatever this fails to unlock, the
            // deletion that follows it reports.
            if let Ok(metadata) = directory.metadata() {
                // Change the opened directory itself, without resolving a path or a link.
                let _ = directory.set_permissions(std::fs::Permissions::from_mode(
                    (metadata.permissions().mode() & 0o777) | 0o700,
                ));
            }
        }
    }
}

/// The directory that holds `path`, which is created if need be.
fn parent_of(path: &Path) -> &Path {
    path.parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

pub(crate) fn create_parent(tree: &AppDir, path: &Path) -> Result<()> {
    let parent = parent_of(path);
    tree.root
        .create_dir_all(parent)
        .within(Operation::Create, &tree.path, parent)
}

pub(crate) fn parent_dir(tree: &AppDir, path: &Path) -> Result<Dir> {
    create_parent(tree, path)?;
    let parent = parent_of(path);
    tree.root
        .open_dir(parent)
        .within(Operation::Open, &tree.path, parent)
}

pub(crate) fn make_executable(file: &File, path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt;
        let mut permissions = file.metadata().at(Operation::Inspect, path)?.permissions();
        permissions.set_mode((permissions.mode() & 0o777) | 0o111);
        file.set_permissions(permissions)
            .at(Operation::SetPermissions, path)?;
    }
    #[cfg(not(unix))]
    let _ = (file, path);
    Ok(())
}

fn ordinary_permissions(permissions: Permissions) -> Permissions {
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt;
        Permissions::from_mode(permissions.mode() & 0o777)
    }
    #[cfg(not(unix))]
    permissions
}

pub(crate) fn symlink(root: &Dir, target: &Path, path: &Path, directory: bool) -> io::Result<()> {
    #[cfg(unix)]
    {
        let _ = directory;
        root.symlink(target, path)
    }
    #[cfg(windows)]
    if directory {
        root.symlink_dir(target, path)
    } else {
        root.symlink_file(target, path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (root, target, path, directory);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "symbolic links are unavailable",
        ))
    }
}

#[cfg(windows)]
pub(crate) fn clear_readonly(root: &Dir, path: &Path) -> io::Result<()> {
    let mut pending = vec![path.to_owned()];
    while let Some(path) = pending.pop() {
        let metadata = root.symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            for entry in root.read_dir(&path)? {
                pending.push(path.join(entry?.file_name()));
            }
        }
        let mut permissions = metadata.permissions();
        permissions.set_readonly(false);
        root.set_permissions(&path, permissions)?;
    }
    Ok(())
}

/// Files created in a directory, which are removed again unless the work they
/// belong to is committed.
pub(crate) struct NewFiles<'a> {
    dir: &'a Dir,
    /// Where `dir` is, for naming it in errors.
    location: &'a Path,
    paths: Vec<PathBuf>,
}

impl<'a> NewFiles<'a> {
    pub(crate) fn new(dir: &'a Dir, location: &'a Path) -> Self {
        Self {
            dir,
            location,
            paths: Vec::new(),
        }
    }

    pub(crate) fn create(&mut self, path: &Path) -> Result<File> {
        let file = self
            .dir
            .open_with(path, OpenOptions::new().write(true).create_new(true))
            .within(Operation::Create, self.location, path)?;
        self.paths.push(path.to_owned());
        Ok(file)
    }

    pub(crate) fn copy(&mut self, path: &Path, source: &mut File) -> Result<()> {
        let mut output = self.create(path)?;
        let copied = io::copy(source, &mut output)
            .and_then(|_| source.metadata())
            .and_then(|metadata| {
                output.set_permissions(ordinary_permissions(metadata.permissions()))
            });
        copied.within(Operation::Copy, self.location, path)
    }

    pub(crate) fn write(&mut self, path: &Path, bytes: &[u8], executable: bool) -> Result<()> {
        let mut output = self.create(path)?;
        output
            .write_all(bytes)
            .within(Operation::Write, self.location, path)?;
        if executable {
            make_executable(&output, &self.location.join(path))?;
        }
        Ok(())
    }

    pub(crate) fn symlink(&mut self, target: &Path, path: &Path, directory: bool) -> Result<()> {
        symlink(self.dir, target, path, directory).within(
            Operation::Symlink,
            self.location,
            path,
        )?;
        self.record(path);
        Ok(())
    }

    pub(crate) fn record(&mut self, path: &Path) {
        self.paths.push(path.to_owned());
    }

    pub(crate) fn link_or_copy(&mut self, source: &Path, destination: &Path) -> Result<()> {
        #[cfg(unix)]
        self.symlink(source, destination, false)?;
        #[cfg(not(unix))]
        {
            let mut source =
                self.dir
                    .open(source)
                    .within(Operation::Open, self.location, source)?;
            self.copy(destination, &mut source)?;
        }
        Ok(())
    }

    pub(crate) fn commit(mut self) {
        self.paths.clear();
    }
}

impl Drop for NewFiles<'_> {
    /// Rolling back is best effort: it runs because something else failed,
    /// and that is the error worth reporting.
    fn drop(&mut self) {
        for path in self.paths.iter().rev() {
            #[cfg(windows)]
            if let Ok(metadata) = self.dir.symlink_metadata(path) {
                if !metadata.is_symlink() {
                    let mut permissions = metadata.permissions();
                    permissions.set_readonly(false);
                    let _ = self.dir.set_permissions(path, permissions);
                }
            }
            let _ = self.dir.remove_file(path);
        }
    }
}
