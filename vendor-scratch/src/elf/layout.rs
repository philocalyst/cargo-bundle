use super::{Fallible, TYPE_TWO_MARKER, file_range, input::Source};
use crate::error::Malformed;
use object::elf;
use object::read::elf::{FileHeader, ProgramHeader, SectionHeader};
use object::{Endianness, Pod};
use std::{mem::size_of, ops::Range};

/// Validate the table before reading entries, including the empty-table convention.
fn table<Record: Pod>(
    offset: u64,
    count: u64,
    entry_size: u16,
    file_length: u64,
) -> Fallible<Range<u64>> {
    if (offset == 0) != (count == 0)
        || (count != 0 && usize::from(entry_size) != size_of::<Record>())
    {
        return Err(Malformed::HeaderTable.into());
    }
    let length = count
        .checked_mul(u64::from(entry_size))
        .ok_or(Malformed::HeaderTable)?;
    file_range(offset, length, file_length)
}

fn read_header<Header: FileHeader<Endian = Endianness>>(
    source: Source<'_>,
    length: u64,
) -> Fallible<(Header, Endianness)> {
    let header: Header = source.record(0)?;
    let endian = header.endian().map_err(|_| Malformed::Header)?;
    let size = u64::from(header.e_ehsize(endian));
    if !header.is_supported()
        || header.e_version(endian) != u32::from(elf::EV_CURRENT.0)
        || !(size_of::<Header>() as u64..=length).contains(&size)
    {
        return Err(Malformed::Header.into());
    }
    Ok((header, endian))
}

fn section_zero<Header: FileHeader<Endian = Endianness>>(
    source: Source<'_>,
    endian: Endianness,
    offset: u64,
    length: u64,
) -> Fallible<Option<Header::SectionHeader>> {
    if offset == 0 {
        return Ok(None);
    }
    file_range(offset, size_of::<Header::SectionHeader>() as u64, length)?;
    let section: Header::SectionHeader = source.record(offset)?;
    if section.sh_type(endian) != elf::SHT_NULL {
        return Err(Malformed::HeaderTable.into());
    }
    Ok(Some(section))
}

pub(super) struct Layout<'a, Header: FileHeader<Endian = Endianness>> {
    source: Source<'a>,
    length: u64,
    header: Header,
    endian: Endianness,
    sections: Range<u64>,
    programs: Range<u64>,
    name_index: u32,
}

impl<'a, Header: FileHeader<Endian = Endianness>> Layout<'a, Header> {
    pub(super) fn read(source: Source<'a>, file_length: u64) -> Fallible<Self> {
        let (header, endian) = read_header::<Header>(source, file_length)?;
        let section_offset = header.e_shoff(endian).into();
        // Section zero doubles as overflow storage for these three ELF fields.
        let first = section_zero::<Header>(source, endian, section_offset, file_length)?;
        let section_count = match header.e_shnum(endian) {
            0 => first.map_or(0, |section| section.sh_size(endian).into()),
            count => u64::from(count),
        };
        let program_count = match header.e_phnum(endian) {
            elf::PN_XNUM => first
                .map(|section| section.sh_info(endian))
                .filter(|&count| count != 0)
                .ok_or(Malformed::HeaderTable)?,
            count => u32::from(count),
        };
        let name_index = match header.e_shstrndx(endian) {
            elf::SHN_XINDEX => first.ok_or(Malformed::HeaderTable)?.sh_link(endian),
            index => u32::from(index.0),
        };
        let sections = table::<Header::SectionHeader>(
            section_offset,
            section_count,
            header.e_shentsize(endian),
            file_length,
        )?;
        let programs = table::<Header::ProgramHeader>(
            header.e_phoff(endian).into(),
            u64::from(program_count),
            header.e_phentsize(endian),
            file_length,
        )?;
        Ok(Self {
            source,
            length: file_length,
            header,
            endian,
            sections,
            programs,
            name_index,
        })
    }

    pub(super) fn extent(&self, require_marker: bool) -> Fallible<u64> {
        if require_marker && !self.is_type_two_runtime() {
            return Err(Malformed::MissingMarker.into());
        }
        let sections = table_extent(
            self.source,
            self.sections.clone(),
            self.length,
            |section: &Header::SectionHeader| {
                if section.sh_type(self.endian) == elf::SHT_NULL {
                    None
                } else {
                    section.file_range(self.endian)
                }
            },
        )?;
        let programs = table_extent(
            self.source,
            self.programs.clone(),
            self.length,
            |program: &Header::ProgramHeader| {
                (program.p_type(self.endian) != elf::PT_NULL)
                    .then(|| program.file_range(self.endian))
            },
        )?;
        Ok(u64::from(self.header.e_ehsize(self.endian))
            .max(sections)
            .max(programs))
    }

    fn is_type_two_runtime(&self) -> bool {
        const MARKER_OFFSET: usize = std::mem::offset_of!(elf::Ident, abi_version);
        matches!(self.header.e_type(self.endian), elf::ET_EXEC | elf::ET_DYN)
            && object::bytes_of(&self.header)[MARKER_OFFSET..].starts_with(TYPE_TWO_MARKER)
    }

    pub(super) fn names(&self) -> Fallible<Range<u64>> {
        if self.name_index == u32::from(elf::SHN_UNDEF.0) {
            return Ok(0..0);
        }
        let stride = size_of::<Header::SectionHeader>() as u64;
        let index = u64::from(self.name_index);
        if index >= (self.sections.end - self.sections.start) / stride {
            return Err(Malformed::StringTable.into());
        }
        let strings: Header::SectionHeader =
            self.source.record(self.sections.start + index * stride)?;
        if strings.sh_type(self.endian) != elf::SHT_STRTAB {
            return Err(Malformed::StringTable.into());
        }
        file_range(
            strings.sh_offset(self.endian).into(),
            strings.sh_size(self.endian).into(),
            self.length,
        )
    }

    pub(super) fn find_section(
        &self,
        mut matches: impl FnMut(u32) -> Fallible<bool>,
    ) -> Fallible<Option<Range<u64>>> {
        self.source.scan(
            self.sections.clone(),
            |sections: &[Header::SectionHeader]| {
                for section in sections {
                    if section.sh_type(self.endian) != elf::SHT_NULL
                        && matches(section.sh_name(self.endian))?
                    {
                        let (offset, size) = section.file_range(self.endian).unwrap_or_default();
                        return file_range(offset, size, self.length).map(Some);
                    }
                }
                Ok(None)
            },
        )
    }
}

/// Empty records claim no bytes, even when their nominal offset is beyond EOF.
fn table_extent<Record: Pod>(
    source: Source<'_>,
    table: Range<u64>,
    file_length: u64,
    region: impl Fn(&Record) -> Option<(u64, u64)>,
) -> Fallible<u64> {
    let mut end = table.end;
    source.scan(table, |records: &[Record]| {
        for (offset, size) in records.iter().filter_map(&region) {
            if size != 0 {
                end = end.max(offset.checked_add(size).ok_or(Malformed::Truncated)?);
            }
        }
        if end > file_length {
            return Err(Malformed::Truncated.into());
        }
        Ok(None::<()>)
    })?;
    Ok(end)
}
