//! Building an AppDir: a directory made so that it packs into a working AppImage.
//!
//! Every method writes through the directory handle it was opened with, never
//! outside it, and never over a file that is already there.
//!
//! ```
//! use appimage::{AppDir, AppId, AppPath, AppRun, DesktopCommand, DesktopEntry, Icon};
//!
//! let scratch = tempfile::tempdir()?;
//! std::fs::write(scratch.path().join("hello"), "#!/bin/sh\necho hello\n")?;
//! std::fs::write(scratch.path().join("hello.svg"), "<svg/>")?;
//!
//! let mut app = AppDir::create(scratch.path().join("Hello.AppDir"))?;
//! app.copy_file(scratch.path().join("hello"), AppPath::new("usr/bin/hello")?)?
//!     .set_apprun(AppRun::Script)?;
//!
//! let id = AppId::new("hello")?;
//! let entry = DesktopEntry::new("Hello", DesktopCommand::new("hello"));
//! app.write_desktop_entry(&id, &entry, Icon::Svg(&scratch.path().join("hello.svg")))?;
//! app.validate()?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::io;
use std::ops::Deref;
use std::path::{Component, Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::{Dir, File};

#[cfg(windows)]
use crate::fs::clear_readonly;
use crate::fs::{NewFiles, create_parent, make_executable, symlink};

use crate::error::{At, Operation};
use crate::{AppDirError, AppId, DesktopEntry, Error, Result};

/// A directory tree, held open.
///
/// It is what [`Unpack::to`](crate::Unpack::to) leaves behind and what a
/// [`Packer`](crate::Packer) reads. It dereferences to its [`Path`].
#[derive(Debug)]
pub struct AppDir {
    pub(crate) path: PathBuf,
    pub(crate) root: Dir,
}

impl AppDir {
    /// Open an existing directory. It need not be a complete AppDir yet.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if `path` is not a directory that can be opened.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let path = path.canonicalize().at(Operation::Resolve, path)?;
        let root = Dir::open_ambient_dir(&path, ambient_authority()).at(Operation::Open, &path)?;
        Ok(Self { path, root })
    }

    /// Create a new directory. An existing path is an error; its parent must
    /// exist.
    ///
    /// # Errors
    ///
    /// [`AppDirError::InvalidPath`] if the path has no final component or
    /// contains `..`. [`Error::Io`] if the directory exists or cannot be made.
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        // Resolve the parent first, so changing the working directory cannot redirect later writes.
        let name = path
            .file_name()
            .ok_or_else(|| AppDirError::InvalidPath(path.to_owned()))?;
        if path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        {
            return Err(AppDirError::InvalidPath(path.to_owned()).into());
        }
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = parent.canonicalize().at(Operation::Resolve, parent)?;
        let dir =
            Dir::open_ambient_dir(&parent, ambient_authority()).at(Operation::Open, &parent)?;
        let path = parent.join(name);
        dir.create_dir(name).at(Operation::Create, &path)?;
        let root = dir.open_dir(name).at(Operation::Open, &path)?;
        Ok(Self { path, root })
    }

    /// Copy a regular file to a new destination, keeping its ordinary
    /// permission bits. Parent directories are created as needed; existing
    /// files are never replaced.
    ///
    /// # Errors
    ///
    /// [`AppDirError::UnsupportedFile`] if `source` is not a regular file.
    /// [`Error::Io`] if the destination exists or either file fails.
    pub fn copy_file(
        &mut self,
        source: impl AsRef<Path>,
        destination: AppPath,
    ) -> Result<AppDirFile<'_>> {
        let mut source = open_source(source.as_ref())?;
        create_parent(self, &destination)?;
        let mut writes = NewFiles::new(&self.root, &self.path);
        writes.copy(&destination, &mut source)?;
        writes.commit();
        Ok(AppDirFile {
            path: destination,
            appdir: self,
        })
    }

    /// Borrow an existing regular file inside this AppDir.
    ///
    /// # Errors
    ///
    /// [`AppDirError::MissingFile`] if there is no regular file at `path`.
    pub fn file(&mut self, path: AppPath) -> Result<AppDirFile<'_>> {
        require_file(self, &path)?;
        Ok(AppDirFile { path, appdir: self })
    }

    /// Copy a directory's contents into a new destination. The source and
    /// this AppDir must be disjoint. Chains of links become relative links to
    /// their resolved targets; broken or escaping links fail. The new
    /// destination is removed again on failure, though parent directories
    /// created for it stay.
    ///
    /// # Errors
    ///
    /// [`AppDirError::OverlappingDirectories`], [`AppDirError::EscapingSymlink`],
    /// [`AppDirError::MissingFile`] for a broken link, or
    /// [`AppDirError::UnsupportedFile`] for a device, FIFO or socket.
    pub fn copy_directory(&mut self, source: impl AsRef<Path>, destination: AppPath) -> Result<()> {
        let source = AppDir::open(source)?;
        if contains_directory(&source, self)? || contains_directory(self, &source)? {
            return Err(AppDirError::OverlappingDirectories.into());
        }
        create_parent(self, &destination)?;
        self.root
            .create_dir(&*destination)
            .within(Operation::Create, &self.path, &destination)?;
        let result = self
            .root
            .open_dir(&*destination)
            .within(Operation::Open, &self.path, &destination)
            .and_then(|root| {
                copy_tree(
                    &source,
                    &AppDir {
                        path: self.path.join(&destination),
                        root,
                    },
                )
            });
        if result.is_err() {
            // The copy failed, and that is the error worth reporting.
            #[cfg(windows)]
            let _ = clear_readonly(&self.root, &destination);
            let _ = self.root.remove_dir_all(&*destination);
        }
        result
    }

    /// Write the desktop entry, the icon named after `id`, and `.DirIcon`
    /// together. Invalid metadata or a collision leaves existing files as they
    /// were.
    ///
    /// # Errors
    ///
    /// [`AppDirError::DesktopField`] for a value that cannot be written,
    /// [`AppDirError::DesktopFileCount`] if there already is a desktop file,
    /// [`Error::Io`] if the icon cannot be read or a file exists.
    pub fn write_desktop_entry(
        &mut self,
        id: &AppId,
        entry: &DesktopEntry<'_>,
        icon: Icon<'_>,
    ) -> Result<()> {
        let text = entry.render(id)?;
        let (source, extension) = icon.parts();
        let mut source = open_source(source)?;
        let count = desktop_files(self)?.len();
        if count != 0 {
            return Err(AppDirError::DesktopFileCount(count).into());
        }
        let filename = PathBuf::from(format!("{}.{extension}", id.as_str()));
        let mut writes = NewFiles::new(&self.root, &self.path);
        writes.copy(&filename, &mut source)?;
        writes.link_or_copy(&filename, Path::new(".DirIcon"))?;
        writes.write(
            Path::new(&format!("{}.desktop", id.as_str())),
            text.as_bytes(),
            false,
        )?;
        writes.commit();
        Ok(())
    }

    /// Check the required files, the launcher's permissions, and that the
    /// desktop file and the icon agree. Other handles and processes can still
    /// change the contents afterwards.
    ///
    /// # Errors
    ///
    /// [`AppDirError::MissingFile`], [`AppDirError::NotExecutable`],
    /// [`AppDirError::DesktopFileCount`] or [`AppDirError::DesktopField`].
    pub fn validate(&self) -> Result<()> {
        let launcher = require_file(self, Path::new(crate::APP_RUN))?;
        #[cfg(unix)]
        {
            use cap_std::fs::PermissionsExt;
            let mode = launcher
                .metadata()
                .within(Operation::Inspect, &self.path, Path::new(crate::APP_RUN))?
                .permissions()
                .mode();
            if mode & 0o111 == 0 {
                return Err(AppDirError::NotExecutable(self.path.join(crate::APP_RUN)).into());
            }
        }
        #[cfg(not(unix))]
        drop(launcher);
        let desktops = desktop_files(self)?;
        let [desktop] = desktops.as_slice() else {
            return Err(AppDirError::DesktopFileCount(desktops.len()).into());
        };
        require_file(self, desktop)?;
        let text =
            self.root
                .read_to_string(desktop)
                .within(Operation::Read, &self.path, desktop)?;
        let icon = desktop_icon(&text)?;
        require_file(self, Path::new(".DirIcon"))?;
        for extension in ["png", "svg", "svgz", "xpm"] {
            let name = PathBuf::from(format!("{}.{extension}", icon.as_str()));
            match require_file(self, &name) {
                Ok(_) => return Ok(()),
                Err(Error::AppDir(AppDirError::MissingFile(_))) => {}
                Err(error) => return Err(error),
            }
        }
        Err(AppDirError::MissingFile(PathBuf::from(icon.as_str())).into())
    }
}

impl Deref for AppDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for AppDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl From<AppDir> for PathBuf {
    fn from(dir: AppDir) -> Self {
        dir.path
    }
}

/// A nonempty path inside an AppDir, without absolute components or `..`.
///
/// ```
/// use appimage::AppPath;
///
/// assert_eq!(AppPath::new("./usr//bin/app")?.to_str(), Some("usr/bin/app"));
/// assert!(AppPath::new("../outside").is_err());
/// assert!(AppPath::new("/etc/passwd").is_err());
/// # Ok::<(), appimage::Error>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppPath(PathBuf);

impl AppPath {
    /// Validate a destination relative to an AppDir and tidy it.
    ///
    /// # Errors
    ///
    /// [`AppDirError::InvalidPath`] if it is empty, absolute, contains `..` or
    /// contains a NUL.
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if path.as_os_str().as_encoded_bytes().contains(&0) {
            return Err(AppDirError::InvalidPath(path.to_owned()).into());
        }
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Normal(value) => normalized.push(value),
                Component::CurDir => {}
                _ => return Err(AppDirError::InvalidPath(path.to_owned()).into()),
            }
        }
        if normalized.as_os_str().is_empty() {
            return Err(AppDirError::InvalidPath(path.to_owned()).into());
        }
        Ok(Self(normalized))
    }
}

impl Deref for AppPath {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for AppPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

/// How an installed executable is launched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppRun {
    /// A relative symlink on Unix; a copy on other platforms.
    Direct,
    /// A shell script resolving the executable relative to the AppDir.
    Script,
}

/// An icon's source file, tagged with its format. Image contents are copied
/// without being decoded.
#[derive(Clone, Copy, Debug)]
pub enum Icon<'a> {
    /// A PNG image.
    Png(&'a Path),
    /// An SVG image.
    Svg(&'a Path),
    /// A gzip-compressed SVG image.
    Svgz(&'a Path),
    /// An XPM image.
    Xpm(&'a Path),
}

impl<'a> Icon<'a> {
    /// Tell the format from the filename extension, whatever its case.
    ///
    /// # Errors
    ///
    /// [`AppDirError::UnsupportedIcon`] for any other extension.
    pub fn from_path(path: &'a Path) -> Result<Self> {
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("png") => Ok(Self::Png(path)),
            Some("svg") => Ok(Self::Svg(path)),
            Some("svgz") => Ok(Self::Svgz(path)),
            Some("xpm") => Ok(Self::Xpm(path)),
            _ => Err(AppDirError::UnsupportedIcon(path.to_owned()).into()),
        }
    }

    fn parts(self) -> (&'a Path, &'static str) {
        match self {
            Self::Png(path) => (path, "png"),
            Self::Svg(path) => (path, "svg"),
            Self::Svgz(path) => (path, "svgz"),
            Self::Xpm(path) => (path, "xpm"),
        }
    }
}

/// An installed file, borrowing its AppDir exclusively.
///
/// It dereferences to the file's path inside the AppDir.
///
/// ```compile_fail,E0499
/// use appimage::{AppDir, AppPath, AppRun};
/// let mut app = AppDir::create("App.AppDir")?;
/// let mut file = app.copy_file("app", AppPath::new("usr/bin/app")?)?;
/// app.copy_file("data", AppPath::new("data")?)?;
/// file.set_apprun(AppRun::Script)?; // `app` is still borrowed by `file`.
/// # Ok::<(), appimage::Error>(())
/// ```
///
/// ```compile_fail,E0505
/// use appimage::{AppDir, AppPath, AppRun};
/// let mut app = AppDir::create("App.AppDir")?;
/// let mut file = app.copy_file("app", AppPath::new("usr/bin/app")?)?;
/// drop(app);
/// file.set_apprun(AppRun::Script)?; // The owner must outlive its file.
/// # Ok::<(), appimage::Error>(())
/// ```
#[derive(Debug)]
pub struct AppDirFile<'a> {
    path: AppPath,
    appdir: &'a mut AppDir,
}

impl Deref for AppDirFile<'_> {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AppDirFile<'_> {
    /// Create `AppRun` for this file and mark the executable as runnable.
    /// An existing `AppRun` is an error.
    ///
    /// # Errors
    ///
    /// [`AppDirError::NonUtf8ScriptPath`] for a script whose path is not UTF-8.
    /// [`Error::Io`] if `AppRun` exists or cannot be written.
    pub fn set_apprun(&mut self, mode: AppRun) -> Result<()> {
        let tree = &*self.appdir;
        let source = require_file(tree, &self.path)?;
        let script = match mode {
            AppRun::Direct => None,
            AppRun::Script => Some(apprun_script(&self.path)?),
        };
        let apprun = Path::new(crate::APP_RUN);
        let mut writes = NewFiles::new(&tree.root, &tree.path);
        match script {
            Some(script) => writes.write(apprun, script.as_bytes(), true)?,
            None => writes.link_or_copy(&self.path, apprun)?,
        }
        make_executable(&source, &tree.path.join(&self.path))?;
        #[cfg(not(unix))]
        make_executable(
            &tree
                .root
                .open(apprun)
                .within(Operation::Open, &tree.path, apprun)?,
            &tree.path.join(apprun),
        )?;
        writes.commit();
        Ok(())
    }
}

/// Whether `child` is `parent`, or lies below it.
fn contains_directory(parent: &AppDir, child: &AppDir) -> Result<bool> {
    fn identity(dir: &AppDir) -> Result<same_file::Handle> {
        let clone = dir
            .root
            .try_clone()
            .and_then(|root| same_file::Handle::from_file(root.into_std_file()));
        clone.at(Operation::Inspect, &dir.path)
    }

    let parent = identity(parent)?;
    let mut child = AppDir {
        path: child.path.clone(),
        root: child.root.try_clone().at(Operation::Open, &child.path)?,
    };
    loop {
        let current = identity(&child)?;
        if current == parent {
            return Ok(true);
        }
        let next = child
            .root
            .open_parent_dir(ambient_authority())
            .at(Operation::Open, &child.path)?;
        let next = AppDir {
            path: child.path.parent().unwrap_or(&child.path).to_owned(),
            root: next,
        };
        if identity(&next)? == current {
            return Ok(false);
        }
        child = next;
    }
}

fn open_source(path: &Path) -> Result<File> {
    let metadata = std::fs::metadata(path).at(Operation::Inspect, path)?;
    if !metadata.is_file() {
        return Err(AppDirError::UnsupportedFile(path.to_owned()).into());
    }
    let file = File::from_std(std::fs::File::open(path).at(Operation::Open, path)?);
    if !file.metadata().at(Operation::Inspect, path)?.is_file() {
        return Err(AppDirError::UnsupportedFile(path.to_owned()).into());
    }
    Ok(file)
}

/// Open a regular file of the tree, or say that it is missing.
pub(crate) fn require_file(tree: &AppDir, path: &Path) -> Result<File> {
    match tree.root.metadata(path) {
        Ok(metadata) if metadata.is_file() => {
            tree.root
                .open(path)
                .within(Operation::Open, &tree.path, path)
        }
        Ok(_) => Err(AppDirError::MissingFile(path.to_owned()).into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err(AppDirError::MissingFile(path.to_owned()).into())
        }
        Err(error) => Err(error).within(Operation::Inspect, &tree.path, path),
    }
}

fn copy_tree(source: &AppDir, output: &AppDir) -> Result<()> {
    let mut directories = vec![PathBuf::from(".")];
    while let Some(current) = directories.pop() {
        let mut entries = source
            .root
            .read_dir(&current)
            .and_then(|entries| entries.collect::<io::Result<Vec<_>>>())
            .within(Operation::List, &source.path, &current)?;
        entries.sort_by_key(cap_std::fs::DirEntry::file_name);
        for entry in entries {
            let path = current.join(entry.file_name());
            let kind = source
                .root
                .symlink_metadata(&path)
                .within(Operation::Inspect, &source.path, &path)?
                .file_type();
            if kind.is_dir() {
                output
                    .root
                    .create_dir(&path)
                    .within(Operation::Create, &output.path, &path)?;
                directories.push(path);
            } else if kind.is_file() {
                let mut file = require_file(source, &path)?;
                let mut writes = NewFiles::new(&output.root, &output.path);
                writes.copy(&path, &mut file)?;
                writes.commit();
            } else if kind.is_symlink() {
                let target = source.root.read_link_contents(&path).within(
                    Operation::ReadLink,
                    &source.path,
                    &path,
                )?;
                if target.has_root()
                    || target
                        .components()
                        .any(|part| matches!(part, Component::Prefix(_)))
                {
                    return Err(AppDirError::EscapingSymlink(path).into());
                }
                let parent = path.parent().unwrap_or(Path::new("."));
                let resolved = source
                    .root
                    .canonicalize(parent.join(target))
                    .map_err(|error| match error.kind() {
                        io::ErrorKind::PermissionDenied => {
                            Error::AppDir(AppDirError::EscapingSymlink(path.clone()))
                        }
                        io::ErrorKind::NotFound => {
                            Error::AppDir(AppDirError::MissingFile(path.clone()))
                        }
                        _ => Error::Io {
                            operation: Operation::Resolve,
                            path: source.path.join(&path),
                            source: error,
                        },
                    })?;
                let target = relative_link(parent, &resolved);
                let is_dir = source
                    .root
                    .metadata(&resolved)
                    .within(Operation::Inspect, &source.path, &resolved)?
                    .is_dir();
                symlink(&output.root, &target, &path, is_dir).within(
                    Operation::Symlink,
                    &output.path,
                    &path,
                )?;
            } else {
                return Err(AppDirError::UnsupportedFile(path).into());
            }
        }
    }
    Ok(())
}

// Both inputs are paths relative to the copied tree's root.
fn relative_link(from: &Path, to: &Path) -> PathBuf {
    let from: Vec<_> = from
        .components()
        .filter(|part| matches!(part, Component::Normal(_)))
        .collect();
    let to: Vec<_> = to
        .components()
        .filter(|part| matches!(part, Component::Normal(_)))
        .collect();
    let common = from
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left == right)
        .count();
    let mut target = PathBuf::new();
    for _ in common..from.len() {
        target.push("..");
    }
    for part in &to[common..] {
        target.push(part.as_os_str());
    }
    if target.as_os_str().is_empty() {
        target.push(".");
    }
    target
}

fn desktop_files(tree: &AppDir) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let entries = tree
        .root
        .entries()
        .within(Operation::List, &tree.path, Path::new("."))?;
    for entry in entries {
        let entry = entry.within(Operation::List, &tree.path, Path::new("."))?;
        let path = PathBuf::from(entry.file_name());
        if path
            .extension()
            .is_some_and(|extension| extension == "desktop")
        {
            files.push(path);
        }
    }
    Ok(files)
}

fn desktop_icon(text: &str) -> Result<AppId> {
    let mut in_entry = false;
    let mut seen_entry = false;
    let mut values = [None; 4];
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            if in_entry && seen_entry {
                return Err(desktop_error("duplicate Desktop Entry group"));
            }
            seen_entry |= in_entry;
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if let Some(index) = ["Type", "Name", "Exec", "Icon"]
                .iter()
                .position(|name| *name == key)
            {
                if values[index].replace(value).is_some() {
                    return Err(desktop_error("duplicate required key"));
                }
            }
        }
    }
    if values[0] != Some("Application")
        || values[1..]
            .iter()
            .any(|value| value.is_none_or(|value| value.trim().is_empty()))
    {
        return Err(desktop_error(
            "expected Type=Application and nonempty Name, Exec and Icon",
        ));
    }
    AppId::new(values[3].ok_or_else(|| desktop_error("missing Icon"))?)
}

fn desktop_error(reason: &'static str) -> Error {
    AppDirError::DesktopField {
        field: "Desktop Entry",
        reason,
    }
    .into()
}

fn apprun_script(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for part in path.components() {
        let part = part
            .as_os_str()
            .to_str()
            .ok_or_else(|| AppDirError::NonUtf8ScriptPath(path.to_owned()))?;
        parts.push(part);
    }
    let command = parts.join("/").replace('\'', "'\\''");
    Ok(format!(
        "#!/bin/sh\nHERE=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd -P) || exit 1\nexec \"$HERE\"/'{command}' \"$@\"\n"
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn script_paths_are_never_converted_lossily() {
        let path = PathBuf::from(OsString::from_vec(b"app\xff".to_vec()));
        assert!(matches!(
            apprun_script(&path),
            Err(Error::AppDir(AppDirError::NonUtf8ScriptPath(_)))
        ));
    }
}
