//! ELF file extents and named sections in type-2 AppImages.
//!
//! A file extent ends at the furthest byte claimed by a header, table or
//! nonempty file-backed region. An AppImage must have SquashFS magic there.
//! Reading the extent touches only the ELF header and its two header tables.

use std::fs::File;
use std::io::{self, ErrorKind};
use std::mem::size_of;
use std::ops::Range;
use std::path::Path;

use object::elf::{self, FileHeader32, FileHeader64};
use object::read::elf::FileHeader;
use object::{Endianness, FileKind};

use crate::error::{At, Error, Malformed, Operation, Result};

mod input;
mod layout;
use input::{Source, read_exact_at};
use layout::Layout;

const SQUASHFS_MAGIC: &[u8; 4] = b"hsqs";
const TYPE_TWO_MARKER: &[u8; 3] = b"AI\x02";
// A working-set choice, independent of the ELF format. Every iteration must
// contain at least one complete record, even when the record size is not a divisor.
const BLOCK_BYTES: usize = 8 * 1024;

/// Why reading the ELF stopped: the bytes are wrong, or the read itself failed.
#[derive(Debug)]
enum Fault {
    Malformed(Malformed),
    Io(io::Error),
}

impl From<Malformed> for Fault {
    fn from(reason: Malformed) -> Self {
        Self::Malformed(reason)
    }
}

impl From<io::Error> for Fault {
    /// A read that runs off the end of the file is a truncated file.
    fn from(error: io::Error) -> Self {
        if error.kind() == ErrorKind::UnexpectedEof {
            Self::Malformed(Malformed::Truncated)
        } else {
            Self::Io(error)
        }
    }
}

type Fallible<T> = std::result::Result<T, Fault>;

/// A file offset denotes no bytes when the length is zero.
fn file_range(offset: u64, length: u64, file_length: u64) -> Fallible<Range<u64>> {
    if length == 0 {
        return Ok(0..0);
    }
    let end = offset
        .checked_add(length)
        .filter(|&end| end <= file_length)
        .ok_or(Malformed::Truncated)?;
    Ok(offset..end)
}

fn kind(source: Source<'_>) -> Fallible<FileKind> {
    let mut buffer = [0; size_of::<elf::Ident>()];
    // A file too short to hold an identification is not an ELF at all.
    let ident = match source.read(0, &mut buffer) {
        Err(Fault::Malformed(Malformed::Truncated)) => return Err(Malformed::NotElf.into()),
        read => read?,
    };
    Ok(FileKind::parse(ident).map_err(|_| Malformed::NotElf)?)
}

fn extent(source: Source<'_>, file_length: u64, require_marker: bool) -> Fallible<u64> {
    match kind(source)? {
        FileKind::Elf32 => {
            Layout::<FileHeader32<Endianness>>::read(source, file_length)?.extent(require_marker)
        }
        FileKind::Elf64 => {
            Layout::<FileHeader64<Endianness>>::read(source, file_length)?.extent(require_marker)
        }
        _ => Err(Malformed::NotElf.into()),
    }
}

/// End of the file-backed ELF regions. Trailing bytes are ignored.
pub(crate) fn elf_len(data: &[u8]) -> Result<u64, Malformed> {
    extent(Source::Memory(data), data.len() as u64, false).map_err(|fault| match fault {
        Fault::Malformed(reason) => reason,
        // Reads from a slice cannot fail for any reason but its length.
        Fault::Io(_) => Malformed::Truncated,
    })
}

fn with_file<T>(path: &Path, inspect: impl FnOnce(&File, u64) -> Fallible<T>) -> Result<T> {
    let file = File::open(path).at(Operation::Open, path)?;
    let length = file.metadata().at(Operation::Inspect, path)?.len();
    inspect(&file, length).map_err(|fault| match fault {
        Fault::Malformed(reason) => Error::NotAppImage {
            path: path.into(),
            reason,
        },
        Fault::Io(source) => Error::Io {
            operation: Operation::Read,
            path: path.into(),
            source,
        },
    })
}

/// Locate SquashFS in a type-2 marked ELF executable or shared-object runtime.
pub(crate) fn payload_offset(path: &Path) -> Result<u64> {
    with_file(path, |file, length| {
        let offset = extent(Source::File(file), length, true)?;
        let mut magic = [0; SQUASHFS_MAGIC.len()];
        // An ELF that ends at the end of the file has no payload after it.
        let found = match read_exact_at(file, &mut magic, offset) {
            Ok(()) => magic == *SQUASHFS_MAGIC,
            Err(error) if error.kind() == ErrorKind::UnexpectedEof => false,
            Err(error) => return Err(error.into()),
        };
        if found {
            Ok(offset)
        } else {
            Err(Malformed::MissingPayload.into())
        }
    })
}

/// Scan an unset reservation with fixed scratch space. Allocate the result
/// only after seeing a nonzero byte. Leading and trailing zeros are preserved.
fn nonblank_data(file: &File, mut range: Range<u64>) -> Fallible<Option<Vec<u8>>> {
    let length = range.end - range.start;
    let mut buffer = [0; BLOCK_BYTES];
    let origin = range.start;
    while range.start < range.end {
        let size = (range.end - range.start).min(buffer.len() as u64) as usize;
        let block = &mut buffer[..size];
        read_exact_at(file, block, range.start)?;
        if block.iter().any(|&byte| byte != 0) {
            let length = usize::try_from(length)
                .map_err(|error| io::Error::new(ErrorKind::OutOfMemory, error))?;
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(length)
                .map_err(|error| io::Error::new(ErrorKind::OutOfMemory, error))?;
            bytes.resize(length, 0);
            let start = (range.start - origin) as usize;
            let end = start + size;
            bytes[start..end].copy_from_slice(block);
            read_exact_at(file, &mut bytes[end..], range.start + size as u64)?;
            return Ok(Some(bytes));
        }
        range.start += size as u64;
    }
    Ok(None)
}

fn named_section<Header: FileHeader<Endian = Endianness>>(
    file: &File,
    length: u64,
    name: &[u8],
) -> Fallible<Option<Vec<u8>>> {
    let source = Source::File(file);
    let layout = Layout::<Header>::read(source, length)?;
    let names = layout.names()?;
    let name_bytes = names.end - names.start;
    if name.is_empty() || name.contains(&0) || name.len() as u64 >= name_bytes {
        return Ok(None);
    }

    // Most name tables fit in one read. Otherwise probe only candidate names.
    let mut block = [0; BLOCK_BYTES];
    let (strings, base) = if name_bytes <= block.len() as u64 {
        let bytes = source.read(names.start, &mut block[..name_bytes as usize])?;
        (Source::Memory(bytes), 0)
    } else {
        (source, names.start)
    };
    let mut scratch = [0; BLOCK_BYTES];
    let found = layout.find_section(|index| {
        let index = u64::from(index);
        if index >= name_bytes {
            return Err(Malformed::StringTable.into());
        }
        if name.len() as u64 >= name_bytes - index {
            return Ok(false);
        }
        strings.matches_name(base + index, name, &mut scratch)
    })?;
    match found {
        Some(range) => nonblank_data(file, range),
        None => Ok(None),
    }
}

/// Return the first section of this name, preserving its raw bytes. An absent,
/// empty, memory-only or entirely zero-filled section is represented by None.
pub(crate) fn section(path: &Path, name: &str) -> Result<Option<Vec<u8>>> {
    with_file(path, |file, length| match kind(Source::File(file))? {
        FileKind::Elf32 => named_section::<FileHeader32<Endianness>>(file, length, name.as_bytes()),
        FileKind::Elf64 => named_section::<FileHeader64<Endianness>>(file, length, name.as_bytes()),
        _ => Err(Malformed::NotElf.into()),
    })
}

#[cfg(test)]
mod tests;
