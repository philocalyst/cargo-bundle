//! Read borrowed bytes directly and disk records in bounded, whole-record blocks.

use super::{BLOCK_BYTES, Fallible};
use crate::error::Malformed;
use object::{Endianness, Pod, ReadRef, elf::FileHeader64};
use std::{fs::File, mem::size_of, ops::Range};

#[derive(Clone, Copy)]
pub(super) enum Source<'a> {
    Memory(&'a [u8]),
    File(&'a File),
}

impl Source<'_> {
    pub(super) fn read<'a>(&'a self, offset: u64, buffer: &'a mut [u8]) -> Fallible<&'a [u8]> {
        match *self {
            Self::Memory(bytes) => Ok(bytes
                .read_bytes_at(offset, buffer.len() as u64)
                .map_err(|()| Malformed::Truncated)?),
            Self::File(file) => {
                read_exact_at(file, buffer, offset)?;
                Ok(buffer)
            }
        }
    }

    /// Match only the requested bytes and their NUL terminator.
    pub(super) fn matches_name(
        self,
        mut offset: u64,
        mut name: &[u8],
        scratch: &mut [u8],
    ) -> Fallible<bool> {
        loop {
            let size = (name.len() + 1).min(scratch.len());
            let bytes = self.read(offset, &mut scratch[..size])?;
            if size > name.len() {
                return Ok(bytes.strip_suffix(&[0]) == Some(name));
            }
            let (prefix, rest) = name.split_at(size);
            if bytes != prefix {
                return Ok(false);
            }
            name = rest;
            offset += size as u64;
        }
    }

    pub(super) fn record<Record: Pod>(&self, offset: u64) -> Fallible<Record> {
        // The largest individual record we read is an ELF64 file or section header.
        const { assert!(size_of::<Record>() <= size_of::<FileHeader64<Endianness>>()) };
        let mut buffer = [0; size_of::<FileHeader64<Endianness>>()];
        let bytes = self.read(offset, &mut buffer[..size_of::<Record>()])?;
        Ok(bytes
            .read_at::<Record>(0)
            .copied()
            .map_err(|()| Malformed::Truncated)?)
    }

    /// Return as soon as a batch produces a result. An in-memory table is one batch.
    #[inline]
    pub(super) fn scan<Record: Pod, Found>(
        self,
        mut table: Range<u64>,
        mut inspect: impl FnMut(&[Record]) -> Fallible<Option<Found>>,
    ) -> Fallible<Option<Found>> {
        let stride = size_of::<Record>();
        if let Self::Memory(bytes) = self {
            let count = ((table.end - table.start) / stride as u64) as usize;
            return inspect(
                bytes
                    .read_slice_at(table.start, count)
                    .map_err(|()| Malformed::Truncated)?,
            );
        }
        let mut buffer = [0; BLOCK_BYTES];
        let capacity = buffer.len() / stride * stride;
        while table.start < table.end {
            let length = (table.end - table.start).min(capacity as u64) as usize;
            let bytes = self.read(table.start, &mut buffer[..length])?;
            let records = bytes
                .read_slice_at(0, length / stride)
                .map_err(|()| Malformed::Truncated)?;
            if let Some(found) = inspect(records)? {
                return Ok(Some(found));
            }
            table.start += length as u64;
        }
        Ok(None)
    }
}

/// Fill `buffer` from `file` at `offset`, leaving the file's cursor alone where
/// the platform allows.
#[cfg(unix)]
pub(super) fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buffer, offset)
}

/// Fill `buffer` from `file` at `offset`.
#[cfg(not(unix))]
pub(super) fn read_exact_at(
    mut file: &File,
    buffer: &mut [u8],
    offset: u64,
) -> std::io::Result<()> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(buffer)
}
