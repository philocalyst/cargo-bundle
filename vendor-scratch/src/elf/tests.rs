mod fixture;

use super::{BLOCK_BYTES, payload_offset, section};
use crate::error::{Error, Malformed};
use fixture::{ABSENT, Format, formats};
use object::Endianness;
use object::elf::{
    ET_CORE, ET_DYN, ET_REL, FileHeader32, FileHeader64, PN_XNUM, PT_LOAD, PT_NULL, SHN_LORESERVE,
    SHN_XINDEX, SHT_NOBITS, SHT_NULL, SHT_PROGBITS, SHT_STRTAB, SectionType,
};
use std::fs::{self, File};
use std::io::Write;
use std::mem::{offset_of, size_of};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

fn elf_len(data: &[u8]) -> Option<u64> {
    super::elf_len(data).ok()
}

fn assert_format_error<T>(path: &Path, result: crate::error::Result<T>, expected: Malformed) {
    match result {
        Err(Error::NotAppImage { path: bad, reason }) => {
            assert_eq!((bad.as_path(), reason), (path, expected));
        }
        Err(other) => panic!("expected {expected:?}, got {other}"),
        Ok(_) => panic!("expected {expected:?}, got success"),
    }
}

fn with_file(bytes: &[u8], check: impl FnOnce(&Path)) {
    static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);
    struct TemporaryFile(PathBuf);
    impl Drop for TemporaryFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let path = std::env::temp_dir().join(format!(
        "appimage-elf-test-{}-{}",
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = File::create_new(&path).unwrap();
    let guard = TemporaryFile(path);
    file.write_all(bytes).unwrap();
    drop(file);
    check(&guard.0);
}

fn image_with_records(
    format: Format,
    programs: &[Vec<u8>],
    sections: &[Vec<u8>],
    names: u16,
) -> Vec<u8> {
    let program_start = format.header_size();
    let section_start = program_start + programs.len() * format.program_size();
    let table = |start, count| {
        if count == 0 {
            (0, 0)
        } else {
            (start, u16::try_from(count).unwrap())
        }
    };
    let mut image = format.header(
        table(program_start, programs.len()),
        table(section_start, sections.len()),
        names,
    );
    for record in programs.iter().chain(sections) {
        image.extend(record);
    }
    image
}

fn assert_exact_extent(image: &[u8]) {
    let expected = Some(image.len() as u64);
    assert_eq!(elf_len(image), expected);
    for length in 0..image.len() {
        assert_eq!(elf_len(&image[..length]), None, "prefix of {length} bytes");
    }
    let mut unaligned = vec![0];
    unaligned.extend_from_slice(image);
    assert_eq!(elf_len(&unaligned[1..]), expected);
    let mut padded = image.to_vec();
    padded.extend(b"hsqs\0trailing bytes");
    assert_eq!(elf_len(&padded), expected);
}

#[test]
fn headers_tables_and_file_backed_data_can_each_end_the_elf() {
    for format in formats() {
        let null = format.section(SHT_NULL, 0, 0, 0);
        assert_exact_extent(&format.header(ABSENT, ABSENT, 0));
        assert_exact_extent(&image_with_records(
            format,
            &[format.program(PT_NULL, 0, 0)],
            &[],
            0,
        ));
        assert_exact_extent(&image_with_records(
            format,
            &[],
            &[null.clone(), format.section(SHT_NOBITS, 0, u64::MAX, 9)],
            0,
        ));
        let after_tables = format.header_size() + format.program_size() + 2 * format.section_size();
        for (program_size, section_size) in [(1, 3), (3, 1), (3, 3)] {
            let offset = after_tables as u64;
            let mut image = image_with_records(
                format,
                &[format.program(PT_LOAD, offset, program_size)],
                &[
                    null.clone(),
                    format.section(SHT_PROGBITS, 0, offset, section_size),
                ],
                0,
            );
            image.extend([0; 3]);
            assert_exact_extent(&image);
        }
    }
}

#[test]
fn unused_and_empty_records_claim_no_file_bytes() {
    for format in formats() {
        let programs = [
            format.program(PT_NULL, u64::MAX, u64::MAX),
            format.program(PT_LOAD, u64::MAX, 0),
        ];
        let sections = [
            format.section(SHT_NULL, 0, 0, 0),
            format.section(SHT_NULL, 0, u64::MAX, u64::MAX),
            format.section(SHT_NOBITS, 0, u64::MAX, u64::MAX),
            format.section(SHT_PROGBITS, 0, u64::MAX, 0),
        ];
        assert_exact_extent(&image_with_records(format, &programs, &sections, 0));
    }
}

#[test]
fn nonempty_ranges_must_fit_without_wrapping() {
    for format in formats() {
        for (offset, size) in [(u64::MAX, 1), (1, u64::MAX), (u64::MAX, u64::MAX)] {
            let segment = format.program(PT_LOAD, offset, size);
            let section = format.section(SHT_PROGBITS, 0, offset, size);
            assert_eq!(
                elf_len(&image_with_records(format, &[segment], &[], 0)),
                None
            );
            assert_eq!(
                elf_len(&image_with_records(
                    format,
                    &[],
                    &[format.section(SHT_NULL, 0, 0, 0), section],
                    0
                )),
                None
            );
        }
    }
}

#[test]
fn table_counts_need_corresponding_records() {
    for format in formats() {
        let start = format.header_size();
        let past_eof = format.header(ABSENT, (usize::MAX, 1), 0);
        with_file(&past_eof, |path| {
            assert_format_error(path, payload_offset(path), Malformed::Truncated);
            assert_format_error(path, section(path, ".name"), Malformed::Truncated);
        });
        let mut zero_program_count = format.header((0, PN_XNUM), (start, 1), 0);
        zero_program_count.extend(format.extended_counts(0, 0, 0));
        assert_eq!(elf_len(&zero_program_count), None);
        for (programs, sections) in [
            ((0, 1), ABSENT),
            (ABSENT, (0, 1)),
            ((start, 0), ABSENT),
            ((start, PN_XNUM), ABSENT),
            (ABSENT, (start, 0)),
        ] {
            assert_eq!(elf_len(&format.header(programs, sections, 0)), None);
        }
    }
}

#[test]
fn declared_header_size_is_checked_and_contributes_to_the_extent() {
    for format in formats() {
        let mut image = format.header(ABSENT, ABSENT, 0);
        let position = if format.wide {
            offset_of!(FileHeader64<Endianness>, e_ehsize)
        } else {
            offset_of!(FileHeader32<Endianness>, e_ehsize)
        };
        let width = size_of::<u16>();
        for size in [image.len() - 1, image.len() + 1] {
            image[position..position + width].copy_from_slice(&format.half_word(size as u16));
            assert_eq!(elf_len(&image), None);
        }
        image.push(0);
        assert_exact_extent(&image);
    }
}

#[test]
fn extended_counts_include_the_last_record_of_both_tables() {
    for format in formats() {
        let program_count = usize::from(PN_XNUM);
        let section_count = usize::from(SHN_LORESERVE);
        let program_start = format.header_size();
        let section_start = program_start + program_count * format.program_size();
        let after_tables = section_start + section_count * format.section_size();
        let mut image = format.header((program_start, PN_XNUM), (section_start, 0), 0);
        image.resize(section_start, 0);
        image.extend(format.extended_counts(section_count, program_count, 0));
        image.resize(after_tables, 0);
        image.extend([0; 2]);
        let last_program = section_start - format.program_size();
        let last_section = after_tables - format.section_size();
        image[last_program..section_start].copy_from_slice(&format.program(
            PT_LOAD,
            after_tables as u64,
            2,
        ));
        image[last_section..after_tables].copy_from_slice(&format.section(
            SHT_PROGBITS,
            0,
            after_tables as u64,
            1,
        ));
        assert_eq!(elf_len(&image), Some((after_tables + 2) as u64));
        image[last_program..section_start].fill(0);
        assert_eq!(elf_len(&image), Some((after_tables + 1) as u64));
        assert_eq!(elf_len(&image[..after_tables - 1]), None);
    }
}

#[test]
fn extended_string_table_index_resolves_through_section_zero() {
    let names = b"\0.note\0";
    let names_index = usize::from(SHN_LORESERVE);
    let section_count = names_index + 1;
    for format in formats() {
        let names_offset = format.header_size() + section_count * format.section_size();
        let value_offset = names_offset + names.len();
        let mut image = format.header(ABSENT, (format.header_size(), 0), SHN_XINDEX.0);
        image.extend(format.extended_counts(section_count, 0, names_index));
        image.extend(format.section(SHT_PROGBITS, 1, value_offset as u64, 1));
        image.resize(names_offset - format.section_size(), 0);
        image.extend(format.section(SHT_STRTAB, 0, names_offset as u64, names.len() as u64));
        image.extend(names);
        image.push(b'x');
        with_file(&image, |path| {
            assert_eq!(section(path, ".note").unwrap(), Some(vec![b'x']))
        });
    }
}

#[test]
fn payload_requires_the_type_two_marker_and_magic_at_the_exact_boundary() {
    for format in formats() {
        let header = format.header(ABSENT, ABSENT, 0);
        let offset = header.len() as u64;
        for (suffix, valid) in [
            (&b"hsqs"[..], true),
            (b"hsq", false),
            (b"\0hsqs", false),
            (b"sqsh", false),
            (b"", false),
        ] {
            let mut image = header.clone();
            image.extend(suffix);
            with_file(&image, |path| {
                if valid {
                    assert_eq!(payload_offset(path).unwrap(), offset);
                } else {
                    assert_format_error(path, payload_offset(path), Malformed::MissingPayload);
                }
            });
        }
        // e_type follows the common identification record in either ELF class.
        let type_offset = offset_of!(FileHeader32<Endianness>, e_type);
        for kind in [ET_REL, ET_CORE, ET_DYN] {
            let mut image = header.clone();
            image[type_offset..type_offset + size_of::<u16>()]
                .copy_from_slice(&format.half_word(kind.0));
            image.extend(b"hsqs");
            assert_eq!(elf_len(&image), Some(offset));
            with_file(&image, |path| {
                if kind == ET_DYN {
                    assert_eq!(payload_offset(path).unwrap(), offset);
                } else {
                    assert_format_error(path, payload_offset(path), Malformed::MissingMarker);
                }
            });
        }
        let marker = offset_of!(object::elf::Ident, abi_version);
        for wrong in [b"AI\x01", b"\0\0\0"] {
            let mut image = header.clone();
            image[marker..marker + wrong.len()].copy_from_slice(wrong);
            image.extend(b"hsqs");
            assert_eq!(elf_len(&image), Some(offset));
            with_file(&image, |path| {
                assert_format_error(path, payload_offset(path), Malformed::MissingMarker)
            });
        }
    }
}

// Offsets in the requested sections are relative to the start of `contents`.
fn image_with_names(
    format: Format,
    names: &[u8],
    contents: &[u8],
    sections: &[(SectionType, u32, usize, u64)],
) -> Vec<u8> {
    let names_index = sections.len() + 1;
    let names_offset = format.header_size() + (names_index + 1) * format.section_size();
    let contents_offset = names_offset + names.len();
    let mut headers = vec![format.section(SHT_NULL, 0, 0, 0)];
    for &(kind, name, offset, size) in sections {
        headers.push(format.section(kind, name, (contents_offset + offset) as u64, size));
    }
    headers.push(format.section(SHT_STRTAB, 0, names_offset as u64, names.len() as u64));
    let mut image = image_with_records(format, &[], &headers, names_index as u16);
    image.extend(names);
    image.extend(contents);
    image
}

fn metadata_image(format: Format, kind: SectionType, contents: &[u8]) -> Vec<u8> {
    let size = if kind == SHT_NOBITS {
        u64::MAX
    } else {
        contents.len() as u64
    };
    image_with_names(
        format,
        b"\0.metadata\0.extra\0",
        contents,
        &[(kind, 1, 0, size)],
    )
}

#[test]
fn name_indexes_must_point_inside_small_and_large_string_tables() {
    let name = ".boundary";
    for format in formats() {
        let long = "z".repeat(2 * BLOCK_BYTES);
        // Traverse both table modes, a name split across a boundary, and
        // a terminator that requires another read after two full blocks.
        for (name, start) in [
            (name, 1),
            (name, BLOCK_BYTES - name.len() / 2),
            (long.as_str(), 1),
        ] {
            let mut names = vec![0; start];
            names.extend(name.as_bytes());
            names.push(0);
            let image =
                |index| image_with_names(format, &names, b"x", &[(SHT_PROGBITS, index, 0, 1)]);
            with_file(&image(start as u32), |path| {
                assert_eq!(section(path, name).unwrap(), Some(vec![b'x']));
                assert_eq!(section(path, &name[..name.len() - 1]).unwrap(), None);
            });
            for invalid in [names.len(), names.len() + 1] {
                with_file(&image(invalid as u32), |path| {
                    assert_format_error(path, section(path, name), Malformed::StringTable)
                });
            }
        }
    }
}

#[test]
fn first_duplicate_section_determines_the_result() {
    let contents = b"A\0B";
    let names = b"\0.duplicate\0";
    for format in formats() {
        for (kind, offset, size, expected) in [
            (SHT_PROGBITS, 0, 1, Some(&b"A"[..])),
            (SHT_PROGBITS, 1, 1, None),
            (SHT_PROGBITS, 0, 0, None),
            (SHT_NOBITS, 0, u64::MAX, None),
        ] {
            let sections = [(kind, 1, offset, size), (SHT_PROGBITS, 1, 2, 1)];
            with_file(
                &image_with_names(format, names, contents, &sections),
                |path| {
                    assert_eq!(section(path, ".duplicate").unwrap().as_deref(), expected);
                },
            );
        }
        let broken = image_with_names(
            format,
            names,
            contents,
            &[(SHT_PROGBITS, 1, contents.len(), 1)],
        );
        with_file(&broken, |path| {
            assert_format_error(path, section(path, ".duplicate"), Malformed::Truncated)
        });
    }
}

#[test]
fn section_lookup_preserves_every_byte_and_recognizes_unset_values() {
    for format in formats() {
        for (kind, contents, expected) in [
            (SHT_PROGBITS, &b"\0x\0"[..], Some(&b"\0x\0"[..])),
            (SHT_PROGBITS, b"\0\0\0", None),
            (SHT_PROGBITS, b"", None),
            (SHT_NOBITS, b"", None),
        ] {
            with_file(&metadata_image(format, kind, contents), |path| {
                assert_eq!(section(path, ".metadata").unwrap().as_deref(), expected);
                for missing in [
                    ".meta",
                    "metadata",
                    ".metadata.v2",
                    ".metadata\0.extra",
                    ".extra",
                ] {
                    assert_eq!(section(path, missing).unwrap(), None, "matched {missing:?}");
                }
            });
        }
    }
}

#[test]
fn large_metadata_preserves_zeros_across_read_blocks() {
    let mut data = vec![0; 3 * BLOCK_BYTES];
    for index in [0, BLOCK_BYTES - 1, BLOCK_BYTES, data.len() - 1] {
        data[index] = 1;
        let format = formats().next().unwrap();
        with_file(&metadata_image(format, SHT_PROGBITS, &data), |path| {
            assert_eq!(
                section(path, ".metadata").unwrap().as_deref(),
                Some(data.as_slice())
            );
        });
        data[index] = 0;
    }
}

#[test]
fn absent_or_empty_name_table_has_no_named_sections() {
    for format in formats() {
        let null = format.section(SHT_NULL, 0, 0, 0);
        let strings = format.section(SHT_STRTAB, 0, 0, 0);
        for image in [
            format.header(ABSENT, ABSENT, 0),
            image_with_records(format, &[], std::slice::from_ref(&null), 0),
            image_with_records(format, &[], &[null, strings], 1),
        ] {
            with_file(&image, |path| {
                assert_eq!(section(path, ".name").unwrap(), None)
            });
        }
    }
}

#[test]
fn bad_name_table_index_range_or_type_is_a_format_error() {
    for format in formats() {
        let null = format.section(SHT_NULL, 0, 0, 0);
        let start = format.header_size() + 2 * format.section_size();
        for (index, kind, offset, reason) in [
            (2, SHT_STRTAB, start as u64, Malformed::StringTable),
            (1, SHT_STRTAB, u64::MAX, Malformed::Truncated),
            (1, SHT_PROGBITS, start as u64, Malformed::StringTable),
        ] {
            let strings = format.section(kind, 0, offset, 1);
            let mut image = image_with_records(format, &[], &[null.clone(), strings], index);
            image.push(0);
            with_file(&image, |path| {
                assert_format_error(path, section(path, ".missing"), reason)
            });
        }
    }
}

#[test]
fn version_and_entry_widths_are_checked() {
    for format in formats() {
        macro_rules! position {
            ($field:ident) => {
                if format.wide {
                    offset_of!(FileHeader64<Endianness>, $field)
                } else {
                    offset_of!(FileHeader32<Endianness>, $field)
                }
            };
        }
        let segment = format.program(PT_NULL, 0, 0);
        for (mut image, offset, width) in [
            (
                format.header(ABSENT, ABSENT, 0),
                position!(e_version),
                size_of::<u32>(),
            ),
            (
                metadata_image(format, SHT_PROGBITS, b"x"),
                position!(e_shentsize),
                size_of::<u16>(),
            ),
            (
                image_with_records(format, &[segment], &[], 0),
                position!(e_phentsize),
                size_of::<u16>(),
            ),
        ] {
            assert_eq!(elf_len(&image), Some(image.len() as u64));
            image[offset..offset + width].fill(0);
            assert_eq!(elf_len(&image), None);
        }
    }
}

#[test]
fn empty_and_non_elf_inputs_get_the_format_error() {
    for bytes in [&b""[..], &[0; 128], b"!<arch>\n"] {
        assert_eq!(elf_len(bytes), None);
        with_file(bytes, |path| {
            assert_format_error(path, payload_offset(path), Malformed::NotElf);
            assert_format_error(path, section(path, ".name"), Malformed::NotElf);
        });
    }
}
