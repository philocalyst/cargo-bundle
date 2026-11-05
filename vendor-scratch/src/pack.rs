use std::io::Write;
use std::path::{Path, PathBuf};

use backhand::compression::{CompressionOptions, Compressor, Lz4};
use backhand::{FilesystemCompressor, FilesystemWriter, NodeHeader};
use cap_std::fs::Metadata;

use crate::error::{At, Error, Operation, Result};
use crate::{AppDir, AppDirError, AppImage, ExistingFiles, Runtime};

/// A SquashFS block size: a power of two from 4 KiB through 1 MiB.
///
/// ```
/// use appimage::BlockSize;
///
/// assert_eq!(BlockSize::new(65536)?.bytes(), 65536);
/// assert!(BlockSize::new(3000).is_err());
/// assert_eq!(BlockSize::default().bytes(), 131072);
/// # Ok::<(), appimage::Error>(())
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct BlockSize(u32);

impl BlockSize {
    /// Check that `bytes` is a size SquashFS allows.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidBlockSize`] otherwise.
    pub fn new(bytes: u32) -> Result<Self> {
        if !bytes.is_power_of_two()
            || !(backhand::MIN_BLOCK_SIZE..=backhand::MAX_BLOCK_SIZE).contains(&bytes)
        {
            return Err(Error::InvalidBlockSize(bytes));
        }
        Ok(Self(bytes))
    }

    /// The size in bytes.
    #[must_use]
    pub fn bytes(self) -> u32 {
        self.0
    }
}

impl Default for BlockSize {
    /// 128 KiB, the size `mksquashfs` uses.
    fn default() -> Self {
        Self(backhand::DEFAULT_BLOCK_SIZE)
    }
}

/// How the SquashFS payload is compressed when packing.
///
/// Every variant is pure Rust. (Reading also supports xz and zstd, behind the
/// `xz` and `zstd` crate features, but there is no pure-Rust encoder for them.)
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum Compression {
    /// DEFLATE: compact, and understood by every SquashFS reader.
    #[default]
    Gzip,
    /// LZ4: the fastest to read, at the cost of size.
    Lz4,
    /// LZO.
    Lzo,
    /// Stored as is.
    None,
}

impl Compression {
    const fn backend(self) -> (Compressor, Option<CompressionOptions>) {
        match self {
            Self::Gzip => (Compressor::Gzip, None),
            // backhand refuses `(Lz4, None)`, though it never reads the fields.
            Self::Lz4 => (
                Compressor::Lz4,
                Some(CompressionOptions::Lz4(Lz4 {
                    version: 1,
                    flags: 0,
                })),
            ),
            Self::Lzo => (Compressor::Lzo, None),
            Self::None => (Compressor::Uncompressed, None),
        }
    }
}

/// Builds an AppImage from an AppDir; see [`Packer::new`].
///
/// An AppImage is a [`Runtime`] with a SquashFS of the AppDir appended, made
/// executable. Pure Rust, no `mksquashfs`, no `appimagetool`.
///
/// The image is written beside its destination and moved into place only once
/// it has been read back successfully, so a failure never leaves a partial
/// AppImage behind.
///
/// # Example
///
/// ```
/// use appimage::{AppDir, AppImage, BlockSize, Compression, Packer, Runtime};
///
/// let dir = tempfile::tempdir()?;
/// let app_dir = dir.path().join("Hello.AppDir");
/// std::fs::create_dir(&app_dir)?;
/// std::fs::write(app_dir.join("AppRun"), "#!/bin/sh\necho hello\n")?;
///
/// let image = Packer::new(&AppDir::open(&app_dir)?, &Runtime::inert())
///     .compression(Compression::Lz4)
///     .block_size(BlockSize::new(65536)?)
///     .write_to(dir.path().join("Hello.AppImage"))?;
///
/// assert_eq!(image, AppImage::open(&image.path)?);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use = "call `write_to` to produce the AppImage"]
#[derive(Clone, Copy, Debug)]
pub struct Packer<'a> {
    app_dir: &'a AppDir,
    runtime: &'a Runtime,
    compression: Compression,
    block_size: BlockSize,
    existing: ExistingFiles,
}

impl<'a> Packer<'a> {
    /// Pack `app_dir` behind `runtime`.
    ///
    /// The runtime is a required argument because this crate does not bundle
    /// one: see [`Runtime`] for where to get it. [`AppDir::validate`] checks
    /// that the directory is complete, if you want that checked.
    pub fn new(app_dir: &'a AppDir, runtime: &'a Runtime) -> Self {
        Self {
            app_dir,
            runtime,
            compression: Compression::default(),
            block_size: BlockSize::default(),
            existing: ExistingFiles::default(),
        }
    }

    /// Compress the payload with this codec (default [`Compression::Gzip`]).
    pub fn compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// SquashFS data block size (default [`BlockSize::default`]).
    pub fn block_size(mut self, block_size: BlockSize) -> Self {
        self.block_size = block_size;
        self
    }

    /// What to do if `output` exists. The default is to fail.
    pub fn existing(mut self, existing: ExistingFiles) -> Self {
        self.existing = existing;
        self
    }

    /// Write the AppImage to `output` and open it.
    ///
    /// Files are visited in name order. Links are stored as links, never
    /// followed. A device node, FIFO or socket is an error.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the AppDir cannot be read or the output cannot be
    /// written or already exists. [`Error::Squashfs`] if the payload cannot be
    /// encoded. [`AppDirError::UnsupportedFile`] for a special file.
    pub fn write_to(self, output: impl AsRef<Path>) -> Result<AppImage> {
        let output =
            std::path::absolute(output.as_ref()).at(Operation::Resolve, output.as_ref())?;
        let mut writer = FilesystemWriter::default();
        let (compressor, options) = self.compression.backend();
        writer.set_compressor(FilesystemCompressor::new(compressor, options)?);
        writer.set_block_size(self.block_size.bytes());
        add_tree(&mut writer, self.app_dir)?;

        let parent = output.parent().unwrap_or(Path::new("."));
        let mut staged = tempfile::Builder::new()
            .prefix(".appimage-")
            .tempfile_in(parent)
            .at(Operation::Create, parent)?;
        let staged_path = staged.path().to_owned();
        staged
            .write_all(self.runtime)
            .at(Operation::Write, &staged_path)?;
        writer.write_with_offset(staged.as_file_mut(), self.runtime.len() as u64)?;
        staged.flush().at(Operation::Write, &staged_path)?;
        set_executable(staged.as_file()).at(Operation::SetPermissions, &staged_path)?;
        // Only a file that reads back as an AppImage is let out.
        let offset = AppImage::open(&staged_path)?.offset;
        match self.existing {
            ExistingFiles::Error => staged.persist_noclobber(&output),
            ExistingFiles::Replace => staged.persist(&output),
        }
        .map_err(|error| error.error)
        .at(Operation::Create, &output)?;
        Ok(AppImage {
            path: output,
            offset,
        })
    }
}

fn add_tree(writer: &mut FilesystemWriter, tree: &AppDir) -> Result<()> {
    let mut directories = vec![PathBuf::from(".")];
    while let Some(current) = directories.pop() {
        let mut entries = tree
            .root
            .read_dir(&current)
            .and_then(|entries| entries.collect::<std::io::Result<Vec<_>>>())
            .within(Operation::List, &tree.path, &current)?;
        entries.sort_by_key(cap_std::fs::DirEntry::file_name);
        for entry in entries {
            let path = current.join(entry.file_name());
            let metadata =
                tree.root
                    .symlink_metadata(&path)
                    .within(Operation::Inspect, &tree.path, &path)?;
            let kind = metadata.file_type();
            let header = header_of(&metadata);

            if kind.is_symlink() {
                let target = tree.root.read_link_contents(&path).within(
                    Operation::ReadLink,
                    &tree.path,
                    &path,
                )?;
                writer.push_symlink(target, &path, header)?;
            } else if kind.is_dir() {
                writer.push_dir(&path, header)?;
                directories.push(path);
            } else if kind.is_file() {
                let source = tree
                    .root
                    .open(&path)
                    .within(Operation::Open, &tree.path, &path)?;
                writer.push_file(source, &path, header)?;
            } else {
                return Err(AppDirError::UnsupportedFile(tree.path.join(path)).into());
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn header_of(metadata: &Metadata) -> NodeHeader {
    use cap_std::fs::MetadataExt;
    NodeHeader::new(
        (metadata.mode() & 0o7777) as u16,
        metadata.uid(),
        metadata.gid(),
        u32::try_from(metadata.mtime()).unwrap_or(0),
    )
}

#[cfg(not(unix))]
fn header_of(metadata: &Metadata) -> NodeHeader {
    use std::time::UNIX_EPOCH;
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|time| time.into_std().duration_since(UNIX_EPOCH).ok())
        .and_then(|elapsed| u32::try_from(elapsed.as_secs()).ok())
        .unwrap_or(0);
    NodeHeader::new(if metadata.is_dir() { 0o755 } else { 0o644 }, 0, 0, mtime)
}

#[cfg(unix)]
fn set_executable(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn set_executable(_: &std::fs::File) -> std::io::Result<()> {
    Ok(())
}
