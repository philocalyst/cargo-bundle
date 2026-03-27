// Resource embedding is only performed when cargo-bundle runs on Windows.
// On other hosts the executable is still copied, but no icon or version info
// is injected into the PE binary.

use crate::Settings;
use crate::bundle::common;
use anyhow::Context;
use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use winres_edit::Resources;

use super::group_icon::GroupIcon;
use super::icon::Icon;
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{Options, Tree};

const ICON_PIXEL_SIZES: &[u32] = &[16, 24, 32, 48, 64, 96, 128, 256, 512];

const LANGUAGE_ID_ENGLISH_US: u16 = 0x0409;

const VERSION_NODE_DATA_TYPE_BINARY: u16 = 0;
const VERSION_NODE_DATA_TYPE_TEXT: u16 = 1;

/// Windows PE resource type identifier for RT_GROUP_ICON.
const RT_GROUP_ICON_RESOURCE_TYPE: u16 = 14;

const APPLICATION_ICON_GROUP_RESOURCE_ID: u16 = 1;
const FIRST_INDIVIDUAL_ICON_RESOURCE_ID: u16 = 1;
const VERSION_RESOURCE_ID: u16 = 1;

/// String table key encoding language 0x0409 (en-US) and code page 0x04B0 (Unicode UTF-16).
const STRING_TABLE_LOCALE_ENGLISH_US_UNICODE: &str = "040904B0";

/// Translation record for English (US), code page 1200 (Unicode UTF-16 LE).
/// Layout: [language_id_low, language_id_high, codepage_low, codepage_high]
const TRANSLATION_ENTRY_ENGLISH_US_UNICODE: [u8; 4] = [0x09, 0x04, 0xB0, 0x04];

const WINDOWS_VERSION_COMPONENT_COUNT: usize = 4;

fn build_fixed_file_info(file_version: u64, product_version: u64) -> Vec<u8> {
    /// Magic signature that identifies a VS_FIXEDFILEINFO structure.
    const VS_FIXED_FILE_INFO_SIGNATURE: u32 = 0xFEEF_04BD;

    /// VS_FIXEDFILEINFO structure layout version 1.0.
    const VS_FIXED_FILE_INFO_STRUCT_VERSION_1_0: u32 = 0x0001_0000;

    const VS_FILE_FLAGS_MASK_ALL: u32 = 0xFFFF_FFFF;
    const VS_FILE_FLAGS_NONE: u32 = 0x0000_0000;

    /// Operating system identifier: Windows NT Win32 subsystem.
    const VOS_NT_WINDOWS32: u32 = 0x0000_0004;

    /// File type identifier: application executable.
    const VFT_APPLICATION: u32 = 0x0000_0001;

    const VFT2_UNKNOWN_SUBTYPE: u32 = 0x0000_0000;
    const VS_FILE_DATE_UNUSED: u32 = 0x0000_0000;

    let mut buffer = Vec::new();
    for field_value in [
        VS_FIXED_FILE_INFO_SIGNATURE,
        VS_FIXED_FILE_INFO_STRUCT_VERSION_1_0,
        (file_version >> 32) as u32,
        file_version as u32,
        (product_version >> 32) as u32,
        product_version as u32,
        VS_FILE_FLAGS_MASK_ALL,
        VS_FILE_FLAGS_NONE,
        VOS_NT_WINDOWS32,
        VFT_APPLICATION,
        VFT2_UNKNOWN_SUBTYPE,
        VS_FILE_DATE_UNUSED,
        VS_FILE_DATE_UNUSED,
    ] {
        buffer.extend_from_slice(&field_value.to_le_bytes());
    }
    buffer
}
/// Builds a VS_VERSIONINFO node per the Windows SDK specification:
/// <https://learn.microsoft.com/en-us/windows/win32/menurc/vs-versioninfo>
fn build_version_info_node(
    key: &str,
    value_bytes: &[u8],
    data_type: u16,
    children: &[u8],
) -> Vec<u8> {
    let key_encoded = encode_null_terminated_utf16_le(key);
    let header_byte_size = 2 + 2 + 2 + key_encoded.len();
    let total_byte_size = header_byte_size + value_bytes.len() + children.len();

    let mut buffer = Vec::new();
    buffer.extend_from_slice(&(total_byte_size as u16).to_le_bytes());
    buffer.extend_from_slice(&(value_bytes.len() as u16).to_le_bytes());
    buffer.extend_from_slice(&data_type.to_le_bytes());
    buffer.extend_from_slice(&key_encoded);

    pad_to_four_byte_alignment(&mut buffer);
    buffer.extend_from_slice(value_bytes);

    pad_to_four_byte_alignment(&mut buffer);
    buffer.extend_from_slice(children);
    buffer
}

fn build_string_file_info(pairs: &[(&str, String)]) -> Vec<u8> {
    let mut string_entries = Vec::new();
    for (key, value) in pairs {
        pad_to_four_byte_alignment(&mut string_entries);
        string_entries.extend(build_string_entry(key, value));
    }

    let string_table = build_version_info_node(
        STRING_TABLE_LOCALE_ENGLISH_US_UNICODE,
        &[],
        VERSION_NODE_DATA_TYPE_TEXT,
        &string_entries,
    );
    build_version_info_node(
        "StringFileInfo",
        &[],
        VERSION_NODE_DATA_TYPE_TEXT,
        &string_table,
    )
}

fn build_var_file_info() -> Vec<u8> {
    let translation_node = build_version_info_node(
        "Translation",
        &TRANSLATION_ENTRY_ENGLISH_US_UNICODE,
        VERSION_NODE_DATA_TYPE_BINARY,
        &[],
    );
    build_version_info_node(
        "VarFileInfo",
        &[],
        VERSION_NODE_DATA_TYPE_TEXT,
        &translation_node,
    )
}
