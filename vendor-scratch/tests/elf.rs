mod common;

use appimage::{AppImage, Runtime};
use common::{ByteOrder, ElfClass};
use std::fs;

#[test]
fn elf32_and_elf64_support_both_byte_orders_and_section_access() {
    let fixture = common::Fixture::new();
    let image = fs::read(&fixture.path).unwrap();
    let payload = &image[fixture.offset as usize..];
    for class in [ElfClass::Elf32, ElfClass::Elf64] {
        for order in [ByteOrder::Little, ByteOrder::Big] {
            let mut bytes = common::elf(
                class,
                order,
                &[(".upd_info", common::UPD_INFO.as_bytes()), (".empty", b"")],
            );
            Runtime::try_from(bytes.clone()).unwrap();
            let offset = bytes.len() as u64;
            bytes.extend_from_slice(payload);
            let path = fixture
                .dir
                .path()
                .join(format!("{class:?}-{order:?}.AppImage"));
            fs::write(&path, bytes).unwrap();
            let image = AppImage::open(path).unwrap();
            assert_eq!(image.offset, offset);
            assert_eq!(
                image.section(".upd_info").unwrap().unwrap(),
                common::UPD_INFO.as_bytes()
            );
            assert!(image.section(".empty").unwrap().is_none());
            assert!(image.section(".absent").unwrap().is_none());
            let unpacked = image.unpack().to_temp().unwrap();
            assert_eq!(
                fs::read_to_string(unpacked.join("data")).unwrap(),
                common::DATA_CONTENTS
            );
        }
    }
}

#[test]
fn section_names_longer_than_read_caches_string_limit_still_work() {
    let fixture = common::Fixture::new();
    let name = format!(".{}", "long-name-".repeat(600));
    let mut bytes = common::elf(ElfClass::Elf64, ByteOrder::Little, &[(&name, b"found")]);
    bytes.extend_from_slice(b"hsqs");
    let path = fixture.dir.path().join("long-name.AppImage");
    fs::write(&path, bytes).unwrap();
    let image = AppImage::open(path).unwrap();
    assert_eq!(image.section(&name).unwrap().unwrap(), b"found");
    assert!(image.section(&(name + "different")).unwrap().is_none());
}

#[test]
fn extended_section_counts_and_name_table_indexes_work() {
    let temp = tempfile::TempDir::new().unwrap();
    let mut bytes = common::runtime_elf();
    let table = u64::from_le_bytes(bytes[40..48].try_into().unwrap()) as usize;
    let count = u16::from_le_bytes(bytes[60..62].try_into().unwrap());
    let names = u16::from_le_bytes(bytes[62..64].try_into().unwrap());
    bytes[60..62].copy_from_slice(&0u16.to_le_bytes());
    bytes[62..64].copy_from_slice(&u16::MAX.to_le_bytes());
    bytes[table + 32..table + 40].copy_from_slice(&u64::from(count).to_le_bytes());
    bytes[table + 40..table + 44].copy_from_slice(&u32::from(names).to_le_bytes());
    let offset = bytes.len() as u64;
    bytes.extend_from_slice(b"hsqs");
    let path = temp.path().join("extended.AppImage");
    fs::write(&path, bytes).unwrap();
    let image = AppImage::open(path).unwrap();
    assert_eq!(image.offset, offset);
    assert_eq!(
        image.section(".upd_info").unwrap().unwrap(),
        common::UPD_INFO.as_bytes()
    );
}

#[cfg(unix)]
#[test]
fn large_sparse_images_preserve_offset_and_named_section_reads() {
    use std::io::Write;
    let temp = tempfile::TempDir::new().unwrap();
    let path = temp.path().join("large.AppImage");
    let bytes = common::runtime_elf();
    let mut file = fs::File::create(&path).unwrap();
    file.write_all(&bytes).unwrap();
    file.write_all(b"hsqs").unwrap();
    file.set_len(8 << 30).unwrap();
    let image = AppImage::open(&path).unwrap();
    assert_eq!(image.offset, bytes.len() as u64);
    assert_eq!(
        image.section(".upd_info").unwrap().unwrap(),
        common::UPD_INFO.as_bytes()
    );
}
