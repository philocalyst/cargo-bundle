// Resource embedding is only performed when cargo-bundle runs on Windows.
// On other hosts the executable is still copied, but no icon or version information
// is injected into the PE binary.

use crate::Settings;
use crate::bundle::common;
use anyhow::Context;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

pub fn bundle_project(settings: &Settings) -> crate::Result<Vec<PathBuf>> {
    let executable_name = format!("{}.exe", settings.binary_name());
    common::print_bundling(&executable_name)?;

    let base_directory = settings.project_out_directory().join("bundle/exe");
    fs::create_dir_all(&base_directory)
        .with_context(|| format!("Failed to create output directory {base_directory:?}"))?;

    let output_executable = base_directory.join(&executable_name);
    fs::copy(settings.binary_path(), &output_executable)
        .with_context(|| format!("Failed to copy executable to {output_executable:?}"))?;

    let svg_icon_path = settings
        .icon_files()
        .filter_map(Result::ok)
        .find(|path| path.extension() == Some(OsStr::new("svg")));

    embed_resources(settings, &output_executable, svg_icon_path.as_deref())?;

    Ok(vec![output_executable])
}

fn embed_resources(
    settings: &Settings,
    executable_path: &Path,
    svg_icon: Option<&Path>,
) -> crate::Result<()> {
    #[cfg(target_os = "windows")]
    {
        windows_embedding::embed_windows_resources(settings, executable_path, svg_icon)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (settings, executable_path, svg_icon);
        common::print_warning(
            "Windows PE resource embedding (icons, version information) is only performed \
             when cargo-bundle itself is run on Windows.",
        )?;
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod windows_embedding {
    use super::*;
    use crate::bundle::group_icon::GroupIcon;
    use crate::bundle::icon::Icon;
    use resvg::tiny_skia::{Pixmap, Transform};
    use resvg::usvg::{Options, Tree};
    use winres_edit::{Id, Resource, Resources, resource_type};

    const ICON_PIXEL_SIZES: &[u32] = &[16, 24, 32, 48, 64, 96, 128, 256, 512];
    const LANGUAGE_IDENTIFIER_ENGLISH_US: u16 = 0x0409;
    const VERSION_NODE_DATA_TYPE_BINARY: u16 = 0;
    const VERSION_NODE_DATA_TYPE_TEXT: u16 = 1;
    const RESOURCE_TYPE_GROUP_ICON: u16 = 14;
    const APPLICATION_ICON_GROUP_RESOURCE_IDENTIFIER: u16 = 1;
    const FIRST_INDIVIDUAL_ICON_RESOURCE_IDENTIFIER: u16 = 1;
    const VERSION_RESOURCE_IDENTIFIER: u16 = 1;
    const STRING_TABLE_LOCALE_ENGLISH_US_UNICODE: &str = "040904B0";
    const TRANSLATION_ENTRY_ENGLISH_US_UNICODE: [u8; 4] = [0x09, 0x04, 0xB0, 0x04];
    const WINDOWS_VERSION_COMPONENT_COUNT: usize = 4;

    pub fn embed_windows_resources(
        settings: &Settings,
        executable_path: &Path,
        svg_icon: Option<&Path>,
    ) -> crate::Result<()> {
        let mut resources = Resources::new(executable_path);
        resources
            .load()
            .with_context(|| "Failed to load PE resources")?;
        resources
            .open()
            .with_context(|| "Failed to open PE resources for writing")?;

        embed_version_information(settings, &resources)?;

        if let Some(svg_path) = svg_icon {
            embed_svg_icons(svg_path, &resources)?;
        }

        resources.close();
        Ok(())
    }

    fn embed_version_information(settings: &Settings, resources: &Resources) -> crate::Result<()> {
        let version_string = settings.version_string().to_string();
        let executable_name = format!("{}.exe", settings.binary_name());

        let string_pairs: &[(&str, String)] = &[
            ("ProductName", settings.bundle_name().to_owned()),
            ("FileDescription", settings.short_description().to_owned()),
            ("FileVersion", version_string.clone()),
            ("ProductVersion", version_string),
            (
                "LegalCopyright",
                settings.copyright_string().unwrap_or_default().to_owned(),
            ),
            (
                "CompanyName",
                settings.authors_comma_separated().unwrap_or_default(),
            ),
            ("InternalName", settings.binary_name().to_owned()),
            ("OriginalFilename", executable_name),
        ];

        let version_information_data =
            build_version_information_resource(settings, string_pairs)
                .with_context(|| "Failed to build VS_VERSIONINFO resource")?;

        Resource::new(
            resources,
            resource_type::VERSION.into(),
            Id::Integer(VERSION_RESOURCE_IDENTIFIER).into(),
            LANGUAGE_IDENTIFIER_ENGLISH_US,
            &version_information_data,
        )
        .update()
        .with_context(|| "Failed to update version information resource")?;

        Ok(())
    }

    fn embed_svg_icons(svg_path: &Path, resources: &Resources) -> crate::Result<()> {
        let svg_tree = load_svg_tree(svg_path)?;
        let mut group_icon = GroupIcon::default();

        for (size_index, &pixel_size) in ICON_PIXEL_SIZES.iter().enumerate() {
            let icon_resource_identifier =
                FIRST_INDIVIDUAL_ICON_RESOURCE_IDENTIFIER + size_index as u16;
            let (icon, encoded_icon) = render_and_encode_icon(&svg_tree, pixel_size)?;

            let icon_entry = icon
                .group_icon_entry(icon_resource_identifier)
                .with_context(|| "Failed to build group icon entry")?;

            group_icon.push_icon(icon_entry);

            Resource::new(
                resources,
                resource_type::ICON.into(),
                Id::Integer(icon_resource_identifier).into(),
                LANGUAGE_IDENTIFIER_ENGLISH_US,
                &encoded_icon,
            )
            .update()
            .with_context(|| format!("Failed to embed {pixel_size}×{pixel_size} icon resource"))?;
        }

        let group_icon_data = group_icon
            .encode()
            .with_context(|| "Failed to encode RT_GROUP_ICON")?;

        Resource::new(
            resources,
            Id::Integer(RESOURCE_TYPE_GROUP_ICON).into(),
            Id::Integer(APPLICATION_ICON_GROUP_RESOURCE_IDENTIFIER).into(),
            LANGUAGE_IDENTIFIER_ENGLISH_US,
            &group_icon_data,
        )
        .update()
        .with_context(|| "Failed to embed RT_GROUP_ICON resource")?;

        Ok(())
    }

    fn load_svg_tree(svg_path: &Path) -> crate::Result<Tree> {
        let svg_text = std::fs::read_to_string(svg_path)
            .with_context(|| format!("Failed to read SVG icon {svg_path:?}"))?;

        Tree::from_data(svg_text.as_bytes(), &Options::default())
            .with_context(|| "Failed to parse SVG icon data")
    }

    fn render_and_encode_icon(svg_tree: &Tree, pixel_size: u32) -> crate::Result<(Icon, Vec<u8>)> {
        let mut pixmap = Pixmap::new(pixel_size, pixel_size)
            .with_context(|| format!("Failed to create {pixel_size}×{pixel_size} pixmap"))?;

        let scale_x = pixel_size as f32 / svg_tree.size().width();
        let scale_y = pixel_size as f32 / svg_tree.size().height();

        resvg::render(
            svg_tree,
            Transform::from_scale(scale_x, scale_y),
            &mut pixmap.as_mut(),
        );

        let icon = Icon::new_from_rgba(pixel_size, pixel_size, pixmap.data().to_vec());
        let encoded_icon = icon
            .encode()
            .with_context(|| format!("Failed to encode {pixel_size}×{pixel_size} icon"))?;

        Ok((icon, encoded_icon))
    }

    fn build_version_information_resource(
        settings: &Settings,
        string_pairs: &[(&str, String)],
    ) -> crate::Result<Vec<u8>> {
        let version = parse_version_string(&settings.version_string().to_string());

        let fixed_file_information = build_fixed_file_information(version, version);
        let string_file_information = build_string_file_information(string_pairs);
        let variable_file_information = build_variable_file_information();

        let children = [
            string_file_information.as_slice(),
            variable_file_information.as_slice(),
        ]
        .concat();

        Ok(build_version_information_node(
            "VS_VERSION_INFO",
            &fixed_file_information,
            VERSION_NODE_DATA_TYPE_BINARY,
            &children,
        ))
    }

    fn parse_version_string(version_string: &str) -> u64 {
        let mut version_components = version_string
            .splitn(WINDOWS_VERSION_COMPONENT_COUNT, '.')
            .map(|component| component.parse::<u64>().unwrap_or(0));

        let major = version_components.next().unwrap_or(0);
        let minor = version_components.next().unwrap_or(0);
        let patch = version_components.next().unwrap_or(0);
        let build = version_components.next().unwrap_or(0);

        (major << 48) | (minor << 32) | (patch << 16) | build
    }

    fn build_fixed_file_information(file_version: u64, product_version: u64) -> Vec<u8> {
        const VS_FIXED_FILE_INFO_SIGNATURE: u32 = 0xFEEF_04BD;
        const VS_FIXED_FILE_INFO_STRUCT_VERSION_1_0: u32 = 0x0001_0000;
        const VS_FILE_FLAGS_MASK_ALL: u32 = 0xFFFF_FFFF;
        const VS_FILE_FLAGS_NONE: u32 = 0x0000_0000;
        const VOS_NT_WINDOWS32: u32 = 0x0000_0004;
        const VFT_APPLICATION: u32 = 0x0000_0001;
        const VFT2_UNKNOWN_SUBTYPE: u32 = 0x0000_0000;
        const VS_FILE_DATE_UNUSED: u32 = 0x0000_0000;

        let fields = [
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
        ];

        fields
            .iter()
            .flat_map(|field| field.to_le_bytes())
            .collect()
    }

    fn build_version_information_node(
        key: &str,
        value_bytes: &[u8],
        data_type: u16,
        children: &[u8],
    ) -> Vec<u8> {
        let key_encoded = encode_null_terminated_utf16_little_endian(key);
        let header_byte_size = 2 + 2 + 2 + key_encoded.len();
        let total_byte_size = header_byte_size + value_bytes.len() + children.len();

        let mut buffer = Vec::with_capacity(total_byte_size + 8);
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

    fn build_string_entry(key: &str, value: &str) -> Vec<u8> {
        let key_encoded = encode_null_terminated_utf16_little_endian(key);
        let value_encoded = encode_null_terminated_utf16_little_endian(value);
        let value_character_count = (value.encode_utf16().count() + 1) as u16;
        let node_byte_length = (2 + 2 + 2 + key_encoded.len() + value_encoded.len()) as u16;

        let mut buffer = Vec::with_capacity(node_byte_length as usize + 4);
        buffer.extend_from_slice(&node_byte_length.to_le_bytes());
        buffer.extend_from_slice(&value_character_count.to_le_bytes());
        buffer.extend_from_slice(&VERSION_NODE_DATA_TYPE_TEXT.to_le_bytes());
        buffer.extend_from_slice(&key_encoded);

        pad_to_four_byte_alignment(&mut buffer);
        buffer.extend_from_slice(&value_encoded);

        buffer
    }

    fn build_string_file_information(pairs: &[(&str, String)]) -> Vec<u8> {
        let mut string_entries = Vec::new();

        for (key, value) in pairs {
            pad_to_four_byte_alignment(&mut string_entries);
            string_entries.extend(build_string_entry(key, value));
        }

        let string_table = build_version_information_node(
            STRING_TABLE_LOCALE_ENGLISH_US_UNICODE,
            &[],
            VERSION_NODE_DATA_TYPE_TEXT,
            &string_entries,
        );

        build_version_information_node(
            "StringFileInfo",
            &[],
            VERSION_NODE_DATA_TYPE_TEXT,
            &string_table,
        )
    }

    fn build_variable_file_information() -> Vec<u8> {
        let translation_node = build_version_information_node(
            "Translation",
            &TRANSLATION_ENTRY_ENGLISH_US_UNICODE,
            VERSION_NODE_DATA_TYPE_BINARY,
            &[],
        );

        build_version_information_node(
            "VarFileInfo",
            &[],
            VERSION_NODE_DATA_TYPE_TEXT,
            &translation_node,
        )
    }

    fn pad_to_four_byte_alignment(buffer: &mut Vec<u8>) {
        let remainder = buffer.len() % 4;
        if remainder != 0 {
            buffer.resize(buffer.len() + (4 - remainder), 0);
        }
    }

    fn encode_null_terminated_utf16_little_endian(text: &str) -> Vec<u8> {
        text.encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(|utf16_code_unit| utf16_code_unit.to_le_bytes())
            .collect()
    }
}
