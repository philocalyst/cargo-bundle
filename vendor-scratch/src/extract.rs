//! Writing a SquashFS tree to disk without trusting it.
//!
//! Every write goes through an open handle on the destination, so no archived
//! path and no link already in the destination can reach outside it. Links the
//! archive itself contains are resolved before anything is written, and
//! written last.

use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File};
use std::io::{self, BufReader};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use backhand::{InnerNode, NodeHeader};
use glob::Pattern;

use crate::error::{At, Operation};
use crate::fs::{DirectoryCleanup, NewFiles, create_parent, parent_dir, symlink};
use crate::{AppDir, AppDirError, AppImage, AppPath, Result, codec};

/// What to do when an extracted file or link already exists.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum ExistingFiles {
    /// Fail. Existing directories are reused.
    #[default]
    Error,
    /// Replace it with a staged copy, put in place only once complete. A copy
    /// that fails leaves the old file as it was, and a hard link to the old
    /// file keeps the old contents.
    Replace,
}

pub(crate) struct Options {
    pub pattern: Option<Pattern>,
    pub existing: ExistingFiles,
}

/// Extract into `destination`, which is created if need be. The cleanup that
/// comes back holds the directories created, and keeps their modes from
/// getting in the way of deleting them.
pub(crate) fn extract(
    image: &AppImage,
    destination: &Path,
    options: &Options,
) -> Result<(AppDir, DirectoryCleanup)> {
    let reader = BufReader::new(File::open(&image.path).at(Operation::Open, &image.path)?);
    let filesystem = codec::open(reader, image.offset)?;

    let mut selected = Vec::new();
    let mut links = BTreeMap::new();
    for node in filesystem.files() {
        let path = node.fullpath.strip_prefix("/").unwrap_or(&node.fullpath);
        if path.as_os_str().is_empty() {
            continue; // the SquashFS root itself
        }
        let path = AppPath::new(path)?;
        if options
            .pattern
            .as_ref()
            .is_some_and(|pattern| !pattern.matches_path(&path))
        {
            continue;
        }
        if let InnerNode::Symlink(link) = &node.inner {
            links.insert(path.to_path_buf(), link.link.as_path());
        }
        selected.push((path, node));
    }
    let links: Vec<_> = links
        .iter()
        .map(|(path, target)| {
            resolve_link(path, target, &links).map(|resolved| (path, *target, resolved))
        })
        .collect::<Result<_>>()?;

    fs::create_dir_all(destination).at(Operation::Create, destination)?;
    let tree = AppDir::open(destination)?;
    let mut directories = Vec::new();
    let mut cleanup = DirectoryCleanup::default();
    for (path, node) in selected {
        match &node.inner {
            InnerNode::Dir(_) => {
                create_parent(&tree, &path)?;
                match tree.root.create_dir(&*path) {
                    Ok(()) => {
                        let directory = tree.root.open_dir(&*path).within(
                            Operation::Open,
                            &tree.path,
                            &path,
                        )?;
                        let handle =
                            directory
                                .try_clone()
                                .within(Operation::Open, &tree.path, &path)?;
                        cleanup.directories.push(handle.into_std_file());
                        directories.push((directory, path, node.header));
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        tree.root
                            .open_dir(&*path)
                            .within(Operation::Open, &tree.path, &path)?;
                    }
                    Err(error) => {
                        return Err(error).within(Operation::Create, &tree.path, &path);
                    }
                }
            }
            InnerNode::File(file) => {
                let parent = parent_dir(&tree, &path)?;
                let name = file_name(&path)?;
                let location = tree.path.join(path.parent().unwrap_or(Path::new("")));
                let mut source = filesystem.file(file).reader();
                let mut copy = |output: &mut cap_std::fs::File| -> Result<()> {
                    let copied = io::copy(&mut source, output)
                        .at(Operation::Copy, &tree.path.join(&path))?;
                    if copied != file.file_len() as u64 {
                        let truncated =
                            io::Error::new(io::ErrorKind::UnexpectedEof, "truncated payload file");
                        return Err(truncated).at(Operation::Copy, &tree.path.join(&path));
                    }
                    let output =
                        output
                            .try_clone()
                            .within(Operation::Inspect, &tree.path, &path)?;
                    restore_metadata(&output.into_std(), &node.header, &tree.path.join(&path))
                };
                match options.existing {
                    ExistingFiles::Error => {
                        let mut writes = NewFiles::new(&parent, &location);
                        copy(&mut writes.create(name)?)?;
                        writes.commit();
                    }
                    ExistingFiles::Replace => {
                        let mut staged = cap_tempfile::TempFile::new(&parent).within(
                            Operation::Create,
                            &location,
                            Path::new(""),
                        )?;
                        copy(staged.as_file_mut())?;
                        staged
                            .replace(name)
                            .within(Operation::Replace, &location, name)?;
                    }
                }
            }
            InnerNode::Symlink(_) => {}
            _ => return Err(AppDirError::UnsupportedFile(path.to_path_buf()).into()),
        }
    }

    // Files and directories exist before links can affect path resolution.
    let mut new_links = NewFiles::new(&tree.root, &tree.path);
    for (path, target, resolved) in &links {
        match tree.root.canonicalize(resolved) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).within(Operation::Resolve, &tree.path, resolved),
        }
        let path = path.as_path();
        create_parent(&tree, path)?;
        #[cfg(windows)]
        let directory = tree
            .root
            .metadata(resolved)
            .within(Operation::Inspect, &tree.path, resolved)?
            .is_dir();
        #[cfg(not(windows))]
        let directory = false;
        match options.existing {
            ExistingFiles::Error => new_links.symlink(target, path, directory)?,
            ExistingFiles::Replace => {
                let parent = parent_dir(&tree, path)?;
                let location = tree.path.join(path.parent().unwrap_or(Path::new("")));
                let staged = cap_tempfile::TempDir::new_in(&parent).within(
                    Operation::Create,
                    &location,
                    Path::new(""),
                )?;
                symlink(&staged, target, Path::new("link"), directory).within(
                    Operation::Symlink,
                    &location,
                    Path::new("link"),
                )?;
                let name = file_name(path)?;
                staged
                    .rename("link", &parent, name)
                    .within(Operation::Replace, &location, name)?;
                new_links.record(path);
            }
        }
    }
    for (path, _, _) in &links {
        match tree.root.canonicalize(path) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).within(Operation::Resolve, &tree.path, path),
        }
    }
    new_links.commit();

    // Creating children changes a directory's mtime, and a restrictive mode
    // would stop them being created, so both wait until everything is in.
    for (directory, path, header) in directories.into_iter().rev() {
        let handle = restorable(directory).within(Operation::Open, &tree.path, &path)?;
        restore_metadata(&handle, &header, &tree.path.join(path))?;
    }
    Ok((tree, cleanup))
}

/// A directory handle whose times and mode can be set. On Linux the handle may
/// be an `O_PATH` descriptor, which rejects both with EBADF, so it is reopened
/// as an ordinary descriptor. Elsewhere the handle already works, and on
/// Windows reopening a directory is denied.
fn restorable(directory: cap_std::fs::Dir) -> io::Result<File> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        directory.open(".").map(cap_std::fs::File::into_std)
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        Ok(directory.into_std_file())
    }
}

fn file_name(path: &Path) -> Result<&Path> {
    path.file_name()
        .map(Path::new)
        .ok_or_else(|| AppDirError::InvalidPath(path.to_owned()).into())
}

/// Give a file the mode and modification time it had in the archive. The mode
/// loses `setuid` and `setgid`, and the owner is not restored.
fn restore_metadata(file: &File, header: &NodeHeader, path: &Path) -> Result<()> {
    file.set_modified(UNIX_EPOCH + Duration::from_secs(u64::from(header.mtime)))
        .at(Operation::SetModified, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = u32::from(header.permissions) & 0o1777;
        file.set_permissions(fs::Permissions::from_mode(mode))
            .at(Operation::SetPermissions, path)?;
    }
    Ok(())
}

/// Resolve archive links before writing them, including links in intermediate components.
fn resolve_link(path: &Path, target: &Path, links: &BTreeMap<PathBuf, &Path>) -> Result<PathBuf> {
    let mut remaining: VecDeque<_> = path
        .parent()
        .unwrap_or(Path::new(""))
        .components()
        .chain(target.components())
        .collect();
    let mut resolved = PathBuf::new();
    let mut followed = 0;
    while let Some(component) = remaining.pop_front() {
        match component {
            Component::CurDir => {}
            Component::ParentDir if resolved.pop() => {}
            Component::Normal(name) => {
                resolved.push(name);
                if let Some(target) = links.get(&resolved) {
                    followed += 1;
                    if followed > 40 {
                        return Err(AppDirError::SymlinkLoop(path.to_owned()).into());
                    }
                    resolved.pop();
                    for component in target.components().rev() {
                        remaining.push_front(component);
                    }
                }
            }
            _ => return Err(AppDirError::EscapingSymlink(path.to_owned()).into()),
        }
    }
    if resolved.as_os_str().is_empty() {
        resolved.push(".");
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    fn links<'a>(pairs: &[(&str, &'a str)]) -> BTreeMap<PathBuf, &'a Path> {
        pairs
            .iter()
            .map(|&(path, target)| (PathBuf::from(path), Path::new(target)))
            .collect()
    }

    #[test]
    fn links_resolve_through_other_links() {
        let links = links(&[("a/to-b", "../b"), ("b", "c/d")]);
        let resolved = resolve_link(Path::new("a/to-b"), Path::new("../b"), &links).unwrap();
        assert_eq!(resolved, Path::new("c/d"));
    }

    #[test]
    fn links_cannot_climb_out_or_loop() {
        let escaping = links(&[("a", "../../out")]);
        assert!(matches!(
            resolve_link(Path::new("a"), Path::new("../../out"), &escaping),
            Err(Error::AppDir(AppDirError::EscapingSymlink(_)))
        ));
        let cycle = links(&[("a", "b"), ("b", "a")]);
        assert!(matches!(
            resolve_link(Path::new("a"), Path::new("b"), &cycle),
            Err(Error::AppDir(AppDirError::SymlinkLoop(_)))
        ));
    }
}
