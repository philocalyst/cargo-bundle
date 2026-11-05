//! Write just the fields each fixture needs. The untouched bytes are zero.
use object::elf::{
    ELFCLASS32, ELFCLASS64, ELFDATA2LSB, ELFDATA2MSB, ELFMAG, ET_EXEC, EV_CURRENT, FileHeader32,
    FileHeader64, ProgramHeader32, ProgramHeader64, ProgramType, SectionHeader32, SectionHeader64,
    SectionType, SymbolSection,
};
use object::{Endianness, Pod, from_bytes_mut};
use std::mem::size_of;

#[derive(Clone, Copy)]
pub(super) struct Format {
    pub wide: bool,
    order: Endianness,
}

pub(super) fn formats() -> impl Iterator<Item = Format> {
    [false, true].into_iter().flat_map(|wide| {
        [Endianness::Little, Endianness::Big]
            .into_iter()
            .map(move |order| Format { wide, order })
    })
}

fn record<Record: Pod>(fill: impl FnOnce(&mut Record)) -> Vec<u8> {
    let mut bytes = vec![0; size_of::<Record>()];
    fill(from_bytes_mut::<Record>(&mut bytes).unwrap().0);
    bytes
}

// The ELF structures have identical field names but different field widths.
macro_rules! encode {
    ($format:expr, $narrow:ident, $wide:ident, |$entry:ident| $body:block) => {
        if $format.wide {
            record::<$wide<Endianness>>(|$entry| $body)
        } else {
            record::<$narrow<Endianness>>(|$entry| $body)
        }
    };
}

pub(super) const ABSENT: (usize, u16) = (0, 0);

impl Format {
    fn size<Narrow, Wide>(self) -> usize {
        if self.wide {
            size_of::<Wide>()
        } else {
            size_of::<Narrow>()
        }
    }

    pub fn header_size(self) -> usize {
        self.size::<FileHeader32<Endianness>, FileHeader64<Endianness>>()
    }
    pub fn program_size(self) -> usize {
        self.size::<ProgramHeader32<Endianness>, ProgramHeader64<Endianness>>()
    }
    pub fn section_size(self) -> usize {
        self.size::<SectionHeader32<Endianness>, SectionHeader64<Endianness>>()
    }

    pub fn half_word(self, value: u16) -> [u8; size_of::<u16>()] {
        match self.order {
            Endianness::Little => value.to_le_bytes(),
            Endianness::Big => value.to_be_bytes(),
        }
    }

    pub fn header(self, programs: (usize, u16), sections: (usize, u16), names: u16) -> Vec<u8> {
        encode!(self, FileHeader32, FileHeader64, |header| {
            header.e_ident.magic = ELFMAG;
            header.e_ident.class = if self.wide { ELFCLASS64 } else { ELFCLASS32 };
            header.e_ident.data = match self.order {
                Endianness::Little => ELFDATA2LSB,
                Endianness::Big => ELFDATA2MSB,
            };
            header.e_ident.version = EV_CURRENT;
            header.e_ident.abi_version = b'A';
            header.e_ident.padding[..2].copy_from_slice(b"I\x02");
            header.e_type.set(self.order, ET_EXEC);
            header.e_version.set(self.order, u32::from(EV_CURRENT.0));
            header.e_ehsize.set(self.order, self.header_size() as u16);
            header.e_phoff.set(self.order, programs.0 as _);
            header.e_phnum.set(self.order, programs.1);
            header
                .e_phentsize
                .set(self.order, self.program_size() as u16);
            header.e_shoff.set(self.order, sections.0 as _);
            header.e_shnum.set(self.order, sections.1);
            header
                .e_shentsize
                .set(self.order, self.section_size() as u16);
            header.e_shstrndx.set(self.order, SymbolSection(names));
        })
    }

    pub fn program(self, kind: ProgramType, offset: u64, size: u64) -> Vec<u8> {
        encode!(self, ProgramHeader32, ProgramHeader64, |program| {
            program.p_type.set(self.order, kind);
            program.p_offset.set(self.order, offset as _);
            program.p_filesz.set(self.order, size as _);
            program.p_memsz.set(self.order, size as _);
        })
    }

    pub fn extended_counts(self, sections: usize, programs: usize, names: usize) -> Vec<u8> {
        encode!(self, SectionHeader32, SectionHeader64, |first| {
            first.sh_size.set(self.order, sections as _);
            first.sh_info.set(self.order, programs as u32);
            first.sh_link.set(self.order, names as u32);
        })
    }

    pub fn section(self, kind: SectionType, name: u32, offset: u64, size: u64) -> Vec<u8> {
        encode!(self, SectionHeader32, SectionHeader64, |section| {
            section.sh_type.set(self.order, kind);
            section.sh_name.set(self.order, name);
            section.sh_offset.set(self.order, offset as _);
            section.sh_size.set(self.order, size as _);
        })
    }
}
