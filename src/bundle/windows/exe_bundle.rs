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

