//! A read-only FUSE view of a SquashFS.
//!
//! The whole tree is indexed up front into a flat `Vec<Inode>` (inode `n` is
//! `inodes[n - 1]`), so every request after that is a lookup.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, UNIX_EPOCH};

use backhand::{FilesystemReader, InnerNode, NodeHeader, SquashfsFileReader};
use fuser::{
    BackgroundSession, Config, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags,
    Generation, INodeNo, LockOwner, MountOption, OpenFlags, ReplyAttr, ReplyData, ReplyDirectory,
    ReplyEntry, ReplyOpen, ReplyStatfs, Request, Session,
};

use crate::error::{At, Operation};
use crate::{AppImage, Error, Result, codec};

const TTL: Duration = Duration::from_secs(1);
const BLOCK_SIZE: u32 = 512;

/// Mount `image` on `mountpoint` and serve it from a background thread.
///
/// `fuser` calls `mount(2)` itself and only asks the setuid `fusermount3`
/// helper when the kernel refuses. Dropping the session unmounts.
pub(crate) fn serve(image: &AppImage, mountpoint: &Path) -> Result<BackgroundSession> {
    let reader = BufReader::new(File::open(&image.path).at(Operation::Open, &image.path)?);
    let filesystem = Squash::index(codec::open(reader, image.offset)?);
    let mut config = Config::default();
    config.mount_options = vec![
        MountOption::RO,
        MountOption::NoSuid,
        MountOption::NoDev,
        MountOption::FSName("squashfuse".into()),
        MountOption::Subtype("appimage".into()),
    ];
    Session::new(filesystem, mountpoint, &config)
        .and_then(Session::spawn)
        .map_err(|source| Error::Mount {
            mountpoint: mountpoint.to_owned(),
            source,
        })
}

/// A directory entry: its name and the inode it names.
type Child = (OsString, INodeNo);

enum Content {
    Dir(Vec<Child>),
    File(SquashfsFileReader),
    Symlink(OsString),
    Special(FileType),
}

struct Inode {
    parent: INodeNo,
    header: NodeHeader,
    content: Content,
}

impl Inode {
    fn kind(&self) -> FileType {
        match &self.content {
            Content::Dir(_) => FileType::Directory,
            Content::File(_) => FileType::RegularFile,
            Content::Symlink(_) => FileType::Symlink,
            Content::Special(kind) => *kind,
        }
    }

    fn size(&self) -> u64 {
        match &self.content {
            Content::File(file) => file.file_len() as u64,
            Content::Symlink(target) => target.len() as u64,
            _ => 0,
        }
    }

    fn attr(&self, ino: INodeNo) -> FileAttr {
        let mtime = UNIX_EPOCH + Duration::from_secs(self.header.mtime.into());
        let size = self.size();
        FileAttr {
            ino,
            size,
            blocks: size.div_ceil(BLOCK_SIZE.into()),
            atime: mtime,
            mtime,
            ctime: mtime,
            crtime: mtime,
            kind: self.kind(),
            perm: self.header.permissions & 0o7777,
            nlink: if matches!(self.content, Content::Dir(_)) {
                2
            } else {
                1
            },
            uid: self.header.uid,
            gid: self.header.gid,
            rdev: 0,
            blksize: BLOCK_SIZE,
            flags: 0,
        }
    }
}

struct Squash {
    filesystem: FilesystemReader<'static>,
    inodes: Vec<Inode>,
    /// The last file read, fully decompressed. Reads of one file are
    /// sequential, so one slot is the whole cache.
    cache: Mutex<Option<(INodeNo, Arc<[u8]>)>>,
}

impl Squash {
    fn index(filesystem: FilesystemReader<'static>) -> Self {
        let root = Inode {
            parent: INodeNo::ROOT,
            header: NodeHeader::new(0o755, 0, 0, 0),
            content: Content::Dir(Vec::new()),
        };
        let mut inodes = vec![root];
        let mut by_path = HashMap::from([(PathBuf::from("/"), INodeNo::ROOT)]);

        // `files()` lists parents before their children.
        for node in filesystem.files() {
            if node.fullpath == Path::new("/") {
                inodes[0].header = node.header;
                continue;
            }
            let ino = INodeNo(inodes.len() as u64 + 1);
            let parent = node
                .fullpath
                .parent()
                .and_then(|dir| by_path.get(dir))
                .copied()
                .unwrap_or(INodeNo::ROOT);
            let content = match &node.inner {
                InnerNode::Dir(_) => Content::Dir(Vec::new()),
                InnerNode::File(file) => Content::File(file.clone()),
                InnerNode::Symlink(symlink) => {
                    Content::Symlink(symlink.link.as_os_str().to_owned())
                }
                InnerNode::CharacterDevice(_) => Content::Special(FileType::CharDevice),
                InnerNode::BlockDevice(_) => Content::Special(FileType::BlockDevice),
                InnerNode::NamedPipe => Content::Special(FileType::NamedPipe),
                InnerNode::Socket => Content::Special(FileType::Socket),
            };
            let name = node.fullpath.file_name().unwrap_or_default().to_owned();
            if let Content::Dir(children) = &mut inodes[parent.0 as usize - 1].content {
                children.push((name, ino));
            }
            by_path.insert(node.fullpath.clone(), ino);
            inodes.push(Inode {
                parent,
                header: node.header,
                content,
            });
        }
        Self {
            filesystem,
            inodes,
            cache: Mutex::new(None),
        }
    }

    fn inode(&self, ino: INodeNo) -> Result<&Inode, Errno> {
        usize::try_from(ino.0)
            .ok()
            .and_then(|ino| ino.checked_sub(1))
            .and_then(|index| self.inodes.get(index))
            .ok_or(Errno::ENOENT)
    }

    fn dir(&self, ino: INodeNo) -> Result<(&Inode, &[Child]), Errno> {
        let inode = self.inode(ino)?;
        match &inode.content {
            Content::Dir(children) => Ok((inode, children)),
            _ => Err(Errno::ENOTDIR),
        }
    }

    /// The decompressed contents of file `ino`.
    fn contents(&self, ino: INodeNo, file: &SquashfsFileReader) -> io::Result<Arc<[u8]>> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, bytes)) = cache.as_ref().filter(|(cached, _)| *cached == ino) {
            return Ok(Arc::clone(bytes));
        }
        let mut bytes = Vec::with_capacity(file.file_len());
        self.filesystem
            .file(file)
            .reader()
            .read_to_end(&mut bytes)?;
        let bytes: Arc<[u8]> = bytes.into();
        *cache = Some((ino, Arc::clone(&bytes)));
        Ok(bytes)
    }
}

// The mount is read-only, so the kernel never sends a write. Operations left
// at their default answer `ENOSYS`, which for `access`, `flush`, `release` and
// the xattr calls means "nothing to do".
impl Filesystem for Squash {
    fn lookup(&self, _: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let found = self.dir(parent).and_then(|(_, children)| {
            let &(_, ino) = children
                .iter()
                .find(|(child, _)| child == name)
                .ok_or(Errno::ENOENT)?;
            Ok(self.inode(ino)?.attr(ino))
        });
        match found {
            Ok(attr) => reply.entry(&TTL, &attr, Generation(0)),
            Err(errno) => reply.error(errno),
        }
    }

    fn getattr(&self, _: &Request, ino: INodeNo, _: Option<FileHandle>, reply: ReplyAttr) {
        match self.inode(ino) {
            Ok(inode) => reply.attr(&TTL, &inode.attr(ino)),
            Err(errno) => reply.error(errno),
        }
    }

    fn readlink(&self, _: &Request, ino: INodeNo, reply: ReplyData) {
        match self.inode(ino).map(|inode| &inode.content) {
            Ok(Content::Symlink(target)) => reply.data(target.as_bytes()),
            Ok(_) => reply.error(Errno::EINVAL),
            Err(errno) => reply.error(errno),
        }
    }

    fn open(&self, _: &Request, ino: INodeNo, _: OpenFlags, reply: ReplyOpen) {
        match self.inode(ino).map(|inode| &inode.content) {
            Ok(Content::File(_)) => reply.opened(FileHandle(ino.0), FopenFlags::empty()),
            Ok(Content::Dir(_)) => reply.error(Errno::EISDIR),
            Ok(_) => reply.error(Errno::EACCES),
            Err(errno) => reply.error(errno),
        }
    }

    fn read(
        &self,
        _: &Request,
        ino: INodeNo,
        _: FileHandle,
        offset: u64,
        size: u32,
        _: OpenFlags,
        _: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let Ok(Inode {
            content: Content::File(file),
            ..
        }) = self.inode(ino)
        else {
            return reply.error(Errno::EISDIR);
        };
        match self.contents(ino, file) {
            Ok(bytes) => {
                let start = usize::try_from(offset).map_or(bytes.len(), |o| o.min(bytes.len()));
                let end = start.saturating_add(size as usize).min(bytes.len());
                reply.data(&bytes[start..end]);
            }
            Err(error) => reply.error(error.into()),
        }
    }

    fn readdir(
        &self,
        _: &Request,
        ino: INodeNo,
        _: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let (inode, children) = match self.dir(ino) {
            Ok(dir) => dir,
            Err(errno) => return reply.error(errno),
        };
        let dots = [
            (ino, FileType::Directory, OsStr::new(".")),
            (inode.parent, FileType::Directory, OsStr::new("..")),
        ];
        let children = children.iter().map(|(name, child)| {
            let kind = self
                .inode(*child)
                .map_or(FileType::RegularFile, Inode::kind);
            (*child, kind, name.as_os_str())
        });
        let entries = dots
            .into_iter()
            .chain(children)
            .enumerate()
            .skip(offset as usize);
        for (index, (ino, kind, name)) in entries {
            if reply.add(ino, index as u64 + 1, kind, name) {
                break;
            }
        }
        reply.ok();
    }

    fn statfs(&self, _: &Request, _: INodeNo, reply: ReplyStatfs) {
        let files = self.inodes.len() as u64;
        reply.statfs(0, 0, 0, files, 0, BLOCK_SIZE, 255, BLOCK_SIZE);
    }
}
