//! Builds a self-contained type-2 AppImage for the integration tests: a real
//! SquashFS payload (written by `backhand`) appended to a hand-crafted ELF64
//! whose computed size lands exactly on the payload — so `AppImage::open`
//! reports the right offset and the sections/tree are genuinely parsed.

#![allow(dead_code)]

use std::io::Cursor;
use std::path::PathBuf;

use backhand::compression::Compressor;
use backhand::{FilesystemCompressor, FilesystemWriter, NodeHeader};
use tempfile::TempDir;

/// A written-to-disk synthetic AppImage plus the values tests assert against.
pub struct Fixture {
    pub dir: TempDir,
    pub path: PathBuf,
    /// Offset where the SquashFS payload begins (== the ELF's own size).
    pub offset: u64,
    pub upd_info: &'static str,
}

pub const UPD_INFO: &str = "zsync|https://example.test/App-latest.zsync";
pub const DATA_CONTENTS: &str = "payload\n";
pub const NESTED_CONTENTS: &str = "nested payload\n";
pub const APPRUN_CONTENTS: &str = "#!/bin/sh\nexec true\n";
/// mtime stamped into the gzip fixture's nodes (2020-09-13T12:26:40Z).
pub const MTIME: u32 = 1_600_000_000;

impl Fixture {
    /// Write a fresh gzip-compressed AppImage into a new temp dir.
    pub fn new() -> Self {
        Self::wrap(&build_squashfs())
    }

    /// Wrap an arbitrary SquashFS payload (e.g. a committed xz/zstd fixture) in a
    /// synthetic ELF and write the resulting AppImage to a new temp dir.
    pub fn wrap(squashfs: &[u8]) -> Self {
        Self::wrap_with(squashfs, UPD_INFO.as_bytes())
    }

    /// Like [`Fixture::new`], with `update_info` as the `.upd_info` section.
    pub fn with_update_info(update_info: &[u8]) -> Self {
        Self::wrap_with(&build_squashfs(), update_info)
    }

    fn wrap_with(squashfs: &[u8], update_info: &[u8]) -> Self {
        let elf = build_elf(&[(".upd_info", update_info), (".sha256_sig", b"")]);
        let offset = elf.len() as u64;

        let mut bytes = elf;
        bytes.extend_from_slice(squashfs);

        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("App.AppImage");
        std::fs::write(&path, &bytes).expect("write appimage");

        Self {
            dir,
            path,
            offset,
            upd_info: UPD_INFO,
        }
    }
}

/// A minimal valid runtime ELF suitable for [`appimage::AppImage::pack`] — its
/// computed size equals its byte length, so an appended payload lands right.
pub fn runtime_elf() -> Vec<u8> {
    build_elf(&[(".upd_info", UPD_INFO.as_bytes()), (".sha256_sig", b"")])
}

/// A SquashFS with a small tree: `AppRun`, `data`, `dir/nested`, and a
/// `symlink -> data`.
fn build_squashfs() -> Vec<u8> {
    let mut writer = FilesystemWriter::default();
    writer.set_compressor(
        FilesystemCompressor::new(Compressor::Gzip, None).expect("gzip compressor"),
    );

    let dir = NodeHeader::new(0o755, 0, 0, MTIME);
    // Deliberately include setuid in the untrusted payload; extraction must
    // retain executability while dropping privilege bits.
    let exe = NodeHeader::new(0o4755, 0, 0, MTIME);
    let file = NodeHeader::new(0o644, 0, 0, MTIME);
    let link = NodeHeader::new(0o777, 0, 0, MTIME);

    writer
        .push_file(Cursor::new(APPRUN_CONTENTS), "AppRun", exe)
        .unwrap();
    writer
        .push_file(Cursor::new(DATA_CONTENTS), "data", file)
        .unwrap();
    writer.push_dir("dir", dir).unwrap();
    writer
        .push_file(Cursor::new(NESTED_CONTENTS), "dir/nested", file)
        .unwrap();
    writer.push_symlink("data", "symlink", link).unwrap();

    let mut out = Cursor::new(Vec::new());
    writer.write(&mut out).expect("write squashfs");
    out.into_inner()
}

/// A minimal but valid ELF64 little-endian file whose section-header table sits
/// at the very end, so `appimage_get_elf_size` == the file length.
fn build_elf(sections: &[(&str, &[u8])]) -> Vec<u8> {
    elf(ElfClass::Elf64, ByteOrder::Little, sections)
}

#[derive(Clone, Copy, Debug)]
pub enum ElfClass {
    Elf32,
    Elf64,
}

#[derive(Clone, Copy, Debug)]
pub enum ByteOrder {
    Little,
    Big,
}

impl ByteOrder {
    fn u16(self, buffer: &mut [u8], offset: usize, value: u16) {
        buffer[offset..offset + 2].copy_from_slice(&match self {
            Self::Little => value.to_le_bytes(),
            Self::Big => value.to_be_bytes(),
        });
    }
    fn u32(self, buffer: &mut [u8], offset: usize, value: u32) {
        buffer[offset..offset + 4].copy_from_slice(&match self {
            Self::Little => value.to_le_bytes(),
            Self::Big => value.to_be_bytes(),
        });
    }
    fn u64(self, buffer: &mut [u8], offset: usize, value: u64) {
        buffer[offset..offset + 8].copy_from_slice(&match self {
            Self::Little => value.to_le_bytes(),
            Self::Big => value.to_be_bytes(),
        });
    }
}

pub fn elf(class: ElfClass, order: ByteOrder, sections: &[(&str, &[u8])]) -> Vec<u8> {
    let (header_size, section_size) = match class {
        ElfClass::Elf32 => (52, 40),
        ElfClass::Elf64 => (64, 64),
    };

    // Section-name string table: leading NUL, then each name, then ".shstrtab".
    let mut shstrtab = vec![0u8];
    let mut name_offsets = Vec::new();
    for (name, _) in sections {
        name_offsets.push(shstrtab.len() as u32);
        shstrtab.extend_from_slice(name.as_bytes());
        shstrtab.push(0);
    }
    let shstrtab_name = shstrtab.len() as u32;
    shstrtab.extend_from_slice(b".shstrtab\0");

    // Section data laid out right after the ELF header.
    let mut body = Vec::new();
    let mut shdrs = vec![Shdr::default()]; // index 0: SHT_NULL
    for ((_, data), &name) in sections.iter().zip(&name_offsets) {
        let offset = (header_size + body.len()) as u64;
        body.extend_from_slice(data);
        shdrs.push(Shdr {
            name,
            kind: 1, // SHT_PROGBITS
            offset,
            size: data.len() as u64,
        });
    }
    let shstrtab_offset = (header_size + body.len()) as u64;
    body.extend_from_slice(&shstrtab);
    let shstrndx = shdrs.len() as u16;
    shdrs.push(Shdr {
        name: shstrtab_name,
        kind: 3, // SHT_STRTAB
        offset: shstrtab_offset,
        size: shstrtab.len() as u64,
    });

    let e_shoff = (header_size + body.len()) as u64;
    let e_shnum = shdrs.len() as u16;

    let mut out = vec![0u8; header_size];
    out[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
    out[4] = match class {
        ElfClass::Elf32 => 1,
        ElfClass::Elf64 => 2,
    };
    out[5] = match order {
        ByteOrder::Little => 1,
        ByteOrder::Big => 2,
    };
    out[6] = 1; // EV_CURRENT
    out[8..11].copy_from_slice(b"AI\x02"); // the type-2 marker
    order.u16(&mut out, 16, 2); // ET_EXEC
    order.u16(
        &mut out,
        18,
        match class {
            ElfClass::Elf32 => 3,
            ElfClass::Elf64 => 62,
        },
    );
    order.u32(&mut out, 20, 1);
    match class {
        ElfClass::Elf32 => {
            order.u32(&mut out, 32, e_shoff as u32);
            order.u16(&mut out, 40, header_size as u16);
            order.u16(&mut out, 46, section_size as u16);
            order.u16(&mut out, 48, e_shnum);
            order.u16(&mut out, 50, shstrndx);
        }
        ElfClass::Elf64 => {
            order.u64(&mut out, 40, e_shoff);
            order.u16(&mut out, 52, header_size as u16);
            order.u16(&mut out, 58, section_size as u16);
            order.u16(&mut out, 60, e_shnum);
            order.u16(&mut out, 62, shstrndx);
        }
    }

    out.extend_from_slice(&body);
    for shdr in &shdrs {
        out.extend_from_slice(&shdr.encode(class, order));
    }
    out
}

#[derive(Default)]
struct Shdr {
    name: u32,
    kind: u32,
    offset: u64,
    size: u64,
}

impl Shdr {
    fn encode(&self, class: ElfClass, order: ByteOrder) -> Vec<u8> {
        let mut buf = vec![
            0u8;
            match class {
                ElfClass::Elf32 => 40,
                ElfClass::Elf64 => 64,
            }
        ];
        order.u32(&mut buf, 0, self.name);
        order.u32(&mut buf, 4, self.kind);
        match class {
            ElfClass::Elf32 => {
                order.u32(&mut buf, 16, self.offset as u32);
                order.u32(&mut buf, 20, self.size as u32);
                order.u32(&mut buf, 32, 1);
            }
            ElfClass::Elf64 => {
                order.u64(&mut buf, 24, self.offset);
                order.u64(&mut buf, 32, self.size);
                order.u64(&mut buf, 48, 1);
            }
        }
        buf
    }
}
