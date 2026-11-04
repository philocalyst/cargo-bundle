use crate::Settings;
use anyhow::Context;
use dmg_layout::{
    AliasHeader, AliasKind, AliasMetadata, AliasRecord, AliasTarget, AliasVolume, CatalogNodeId,
    DiskType, FilesystemSignature, HfsTimestampSeconds, VolumeMountPath,
};
use ds_parser::{
    BrowserWindowSettings, FieldKey, IconLocation, IconViewProperties, PropertyList, Record,
    WindowBounds, write,
};
use fstool::fs::hfs_plus::HfsPlus;
use resvg::tiny_skia::Pixmap;
use resvg::usvg::{Options, Transform, Tree};
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use tiff::encoder::{Rational, TiffEncoder, colortype};
use tiff::tags::ResolutionUnit;

const ICON_SIZE: f64 = 48.0;

const WINDOW_POSITION: (u32, u32) = (100, 100);

/// Finder window settings retained until the HFS+ catalog has assigned its
/// actual IDs. Alias metadata must describe the image volume, not this host tree.
pub(super) struct VolumeDecoration {
    window_size: (u32, u32),
    app_position: (u32, u32),
    applications_position: (u32, u32),
    background_path: String,
}

pub(super) fn prepare_volume(
    settings: &Settings,
    volume_tree: &Path,
    app_bundle_name: &OsStr,
) -> crate::Result<Option<VolumeDecoration>> {
    app_bundle_name
        .to_str()
        .with_context(|| "Application bundle name is not valid UTF-8")?;

    let Some(background_path) = settings.osx_dmg_background() else {
        return Ok(None);
    };

    let svg_data = fs::read(background_path)
        .with_context(|| format!("Failed to read DMG background {background_path:?}"))?;
    let tree = Tree::from_data(&svg_data, &Options::default())
        .with_context(|| format!("Failed to parse DMG background SVG {background_path:?}"))?;

    let app_position = center_of_element(&tree, "app")?;
    let applications_position = center_of_element(&tree, "applications")?;

    let background_file = volume_tree.join(".background").join("background.tiff");
    render_background(&tree, &background_file)?;

    Ok(Some(VolumeDecoration {
        window_size: (tree.size().width() as u32, tree.size().height() as u32),
        app_position,
        applications_position,
        background_path: "/.background/background.tiff".into(),
    }))
}

pub(super) fn write_ds_store(
    app_bundle_name: &OsStr,
    decoration: &VolumeDecoration,
    index: &HfsPlus,
) -> crate::Result<Vec<u8>> {
    let app_bundle_name = app_bundle_name
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("application bundle name is not UTF-8"))?;
    let app_catalog_name = index.entry_info(&format!("/{app_bundle_name}"))?.name;
    let background_alias = background_alias_record(decoration, index)?
        .to_bytes()
        .map_err(|error| anyhow::anyhow!("Finder alias: {error}"))?;

    let records = vec![
        Record::new(".", FieldKey::vSrn, 1u32),
        Record::new(
            ".",
            FieldKey::bwsp,
            window_settings(WINDOW_POSITION, decoration.window_size),
        ),
        Record::new(".", FieldKey::icvp, icon_view(background_alias)),
        icon_location_record("Applications", decoration.applications_position),
        icon_location_record(&app_catalog_name, decoration.app_position),
    ];

    let ds_store = write(&records).map_err(|error| anyhow::anyhow!(".DS_Store: {error}"))?;
    Ok(ds_store)
}

fn render_background(tree: &Tree, output_path: &Path) -> crate::Result<()> {
    let mut pixmap = Pixmap::new(
        tree.size().width() as u32 * 2,
        tree.size().height() as u32 * 2,
    )
    .with_context(|| "Failed to allocate pixmap for DMG background")?;
    resvg::render(
        tree,
        Transform::from_scale(
            pixmap.width() as f32 / tree.size().width(),
            pixmap.height() as f32 / tree.size().height(),
        ),
        &mut pixmap.as_mut(),
    );

    fs::create_dir_all(output_path.parent().unwrap())
        .with_context(|| "Failed to create .background directory in DMG volume")?;
    let mut file = fs::File::create(output_path)
        .with_context(|| format!("Failed to create {output_path:?}"))?;
    let mut encoder = TiffEncoder::new(&mut file)?;
    let mut tiff_image = encoder.new_image::<colortype::RGBA8>(pixmap.width(), pixmap.height())?;
    tiff_image.resolution(ResolutionUnit::Inch, Rational { n: 144, d: 1 });
    tiff_image
        .write_data(pixmap.data())
        .with_context(|| "Failed to encode DMG background TIFF")?;
    Ok(())
}

fn center_of_element(tree: &Tree, id: &str) -> crate::Result<(u32, u32)> {
    let node = tree.node_by_id(id).with_context(|| {
        format!("DMG background SVG does not contain an element with id \"{id}\"")
    })?;
    let bounds = node.abs_stroke_bounding_box();
    Ok((
        (bounds.x() + bounds.width() / 2.0) as u32,
        (bounds.y() + bounds.height() / 2.0) as u32,
    ))
}

fn icon_location_record(file_name: &str, position: (u32, u32)) -> Record {
    Record::new(
        file_name,
        FieldKey::Iloc,
        IconLocation::new(position.0, position.1),
    )
}

fn window_settings(position: (u32, u32), size: (u32, u32)) -> BrowserWindowSettings {
    BrowserWindowSettings {
        window_bounds: Some(WindowBounds {
            x: position.0.into(),
            y: position.1.into(),
            width: size.0.into(),
            height: size.1.into(),
        }),
        container_show_sidebar: Some(true),
        show_sidebar: Some(true),
        show_path_bar: Some(false),
        show_status_bar: Some(false),
        show_tab_view: Some(false),
        show_toolbar: Some(false),
        sidebar_width: Some(0.0),
        ..Default::default()
    }
}

fn icon_view(background_alias: Vec<u8>) -> IconViewProperties {
    IconViewProperties {
        background_type: Some(2),
        background_color: Some((1.0, 1.0, 1.0)),
        show_icon_preview: Some(true),
        show_item_info: Some(false),
        text_size: Some(12.0),
        icon_size: Some(ICON_SIZE),
        view_options_version: Some(1),
        grid_spacing: Some(100.0),
        grid_offset: Some((0.0, 0.0)),
        label_on_bottom: Some(true),
        arrange_by: Some("none".into()),
        unknown: [(
            "backgroundImageAlias".into(),
            PropertyList::Data(background_alias),
        )]
        .into(),
        ..Default::default()
    }
}

fn background_alias_record(
    decoration: &VolumeDecoration,
    index: &HfsPlus,
) -> crate::Result<AliasRecord> {
    let parent = index.entry_info("/.background")?;
    let target = index.entry_info(&decoration.background_path)?;
    anyhow::ensure!(
        parent.kind == fstool::fs::EntryKind::Dir,
        "HFS+ background parent is not a directory"
    );
    anyhow::ensure!(
        target.kind == fstool::fs::EntryKind::Regular,
        "HFS+ background is not a file"
    );
    let volume_name = index.volume_name();
    let volume = AliasVolume::new(
        volume_name.as_bytes(),
        HfsTimestampSeconds::from(index.volume_create_date()?),
        FilesystemSignature::HIERARCHICAL_FILE_SYSTEM_PLUS,
        DiskType::Ejectable,
    )
    .map_err(alias_image_error)?;
    let alias_target = AliasTarget::new(
        AliasKind::File,
        CatalogNodeId(parent.cnid),
        b"background.tiff",
        CatalogNodeId(target.cnid),
        HfsTimestampSeconds::from(target.create_date),
    )
    .map_err(alias_image_error)?;
    let volume_path = format!("/Volumes/{volume_name}");
    let metadata = AliasMetadata::builder()
        .header(AliasHeader::new(volume, alias_target))
        .parent_folder_name(Some(b".background".to_vec()))
        .catalog_node_id_path(vec![parent.cnid])
        .unicode_target_name(Some("background.tiff".into()))
        .unicode_volume_name(Some(volume_name.into()))
        .target_path(Some(decoration.background_path.as_bytes().to_vec()))
        .volume_path(Some(
            VolumeMountPath::new(volume_path.as_bytes()).map_err(alias_image_error)?,
        ))
        .build();
    AliasRecord::from_metadata(metadata).map_err(alias_image_error)
}

fn alias_image_error(error: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("Finder alias: {error}")
}
