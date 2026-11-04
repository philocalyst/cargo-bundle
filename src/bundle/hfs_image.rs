//! Connect fstool's tree importer, catalog identities and UDIF encoder.
use anyhow::Context;
use fstool::{
    block::{
        BlockDevice, FileBackend,
        dmg::{UdifWriteOptions, write_udif},
    },
    fs::hfs_plus::{
        FormatOpts, HfsPlus, hfs_date_from_unix,
        size_plan::{GeneratedBudget, HfsPlusSizePlan},
    },
    repack::{FsSink, RepackMeta, RepackSink, Source, walk_source_into_sink},
};
use std::{fs, path::Path, time::UNIX_EPOCH};

pub(super) fn write_image(
    tree: &Path,
    output: &Path,
    volume_name: &str,
    decorate: impl FnOnce(&HfsPlus) -> crate::Result<Option<Vec<u8>>>,
) -> crate::Result<()> {
    let mut options = FormatOpts::default();
    options.volume_name = volume_name.into();
    let source = Source::HostDir(tree.into());
    let mut plan = HfsPlusSizePlan::new(&options);
    fstool::analyze::plan_size(&source, &mut plan)?;
    // Import modes and links through fstool. Add the
    // metadata it does not import, and charge it before formatting.
    let mut metadata = Vec::new();
    for entry in walkdir::WalkDir::new(tree).follow_links(false) {
        let entry = entry?;
        let relative = entry.path().strip_prefix(tree)?;
        let path = if relative.as_os_str().is_empty() {
            "/".into()
        } else {
            format!(
                "/{}",
                relative
                    .to_str()
                    .context("Non-UTF-8 HFS+ path")?
                    .replace(std::path::MAIN_SEPARATOR, "/")
            )
        };
        let attrs = host_xattrs(entry.path())?;
        for (name, value) in &attrs {
            plan.add_xattr(name, value.len() as u64);
        }
        let birth = fs::symlink_metadata(entry.path())?
            .created()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|time| hfs_date_from_unix(time.as_secs() as i64));
        metadata.push((path, attrs, birth));
    }
    let mut budget = GeneratedBudget::default();
    budget.files = 1;
    budget.data_bytes = 1024 * 1024;
    plan.add_budget(&budget);
    plan.apply_to(&mut options);
    use fstool::fs::FsSizePlan;
    let raw = tempfile::NamedTempFile::new()?;
    let mut device = FileBackend::create(raw.path(), plan.total_size())?;
    let mut volume = HfsPlus::format(&mut device, &options)?;
    walk_source_into_sink(
        &source,
        &mut HfsSink {
            volume: &mut volume,
            device: &mut device,
        },
    )?;
    // fstool 0.5.2 cannot set POSIX attributes on the volume root. This
    // staging root is a newly created directory; retain fstool's defaults.
    // Finder metadata, xattrs and creation dates are still preserved below.
    for (path, attrs, birth) in metadata {
        if let Some(birth) = birth {
            volume.set_create_date(&path, birth)?;
        }
        for (name, value) in attrs {
            volume.set_xattr(&mut device, &path, &name, &value)?;
        }
    }
    if let Some(bytes) = decorate(&volume)? {
        anyhow::ensure!(
            bytes.len() as u64 <= budget.data_bytes,
            "Finder metadata exceeds reserved capacity"
        );
        volume.create_file(
            &mut device,
            "/.DS_Store",
            &mut bytes.as_slice(),
            bytes.len() as u64,
            0o644,
            0,
            0,
            0,
        )?;
    }
    volume.flush(&mut device)?;
    device.sync()?;
    let (file, _) = write_udif(
        &mut device,
        fs::File::create(output)?,
        &UdifWriteOptions::default(),
    )?;
    file.sync_all()?;
    Ok(())
}

// fstool 0.5.2 exposes HFS+ hard links through its typed API, but its
// Filesystem trait implementation falls back to copying. Route that one
// operation explicitly so source sizing and import share the same semantics.
struct HfsSink<'a> {
    volume: &'a mut HfsPlus,
    device: &'a mut dyn BlockDevice,
}
impl HfsSink<'_> {
    fn generic(&mut self) -> FsSink<'_> {
        FsSink::new(self.volume, self.device)
    }
}
impl RepackSink for HfsSink<'_> {
    fn put_dir(
        &mut self,
        path: &str,
        meta: RepackMeta,
        xattrs: &[fstool::fs::XattrPair],
    ) -> fstool::Result<()> {
        self.generic().put_dir(path, meta, xattrs)
    }
    fn put_file(
        &mut self,
        path: &str,
        body: &mut dyn std::io::Read,
        len: u64,
        meta: RepackMeta,
        xattrs: &[fstool::fs::XattrPair],
    ) -> fstool::Result<()> {
        self.generic().put_file(path, body, len, meta, xattrs)
    }
    fn put_symlink(
        &mut self,
        path: &str,
        target: &str,
        meta: RepackMeta,
        xattrs: &[fstool::fs::XattrPair],
    ) -> fstool::Result<()> {
        self.generic().put_symlink(path, target, meta, xattrs)
    }
    fn put_device(
        &mut self,
        path: &str,
        kind: fstool::fs::DeviceKind,
        major: u32,
        minor: u32,
        meta: RepackMeta,
        xattrs: &[fstool::fs::XattrPair],
    ) -> fstool::Result<()> {
        self.generic()
            .put_device(path, kind, major, minor, meta, xattrs)
    }
    fn put_hardlink(
        &mut self,
        path: &str,
        target: &str,
        _meta: RepackMeta,
        _xattrs: &[fstool::fs::XattrPair],
    ) -> fstool::Result<bool> {
        self.volume.create_hardlink(self.device, target, path)?;
        Ok(true)
    }
    fn finish(&mut self) -> fstool::Result<()> {
        Ok(())
    }
}

fn host_xattrs(path: &Path) -> crate::Result<Vec<(String, Vec<u8>)>> {
    let mut attrs = Vec::new();
    #[cfg(unix)]
    let compressed = xattr::get(path, "com.apple.decmpfs")?.is_some();
    #[cfg(unix)]
    for name in
        xattr::list(path).with_context(|| format!("Failed to list attributes on {path:?}"))?
    {
        let name = name
            .into_string()
            .map_err(|_| anyhow::anyhow!("Non-UTF-8 attribute on {path:?}"))?;
        // Host reads decompress the data fork. Carrying compression storage
        // metadata would make the resulting file describe a different fork.
        if matches!(name.as_str(), "com.apple.quarantine" | "com.apple.decmpfs")
            || (compressed && name == "com.apple.ResourceFork")
        {
            continue;
        }
        if let Some(value) = xattr::get(path, &name)? {
            attrs.push((name, value));
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(attrs)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use fstool::{block::dmg::DmgBackend, fs::Filesystem};
    use std::{
        io::Read,
        os::unix::fs::{PermissionsExt, symlink},
    };

    #[test]
    fn imports_metadata_links_and_large_catalog_before_generating_finder_files() {
        let temp = tempfile::tempdir().unwrap();
        let tree = temp.path().join("tree");
        fs::create_dir(&tree).unwrap();
        fs::write(tree.join("payload"), b"payload bytes").unwrap();
        fs::set_permissions(tree.join("payload"), fs::Permissions::from_mode(0o751)).unwrap();
        fs::hard_link(tree.join("payload"), tree.join("linked")).unwrap();
        symlink("/Applications", tree.join("Applications")).unwrap();
        let attr = if cfg!(target_os = "macos") {
            "com.example.cargo-bundle"
        } else {
            "user.cargo-bundle"
        };
        let value = vec![0x5a; 6000];
        xattr::set(tree.join("payload"), attr, &value).unwrap();
        xattr::set(&tree, attr, b"root metadata").unwrap();
        #[cfg(target_os = "macos")]
        xattr::set(
            tree.join("payload"),
            "com.apple.ResourceFork",
            b"resource fork",
        )
        .unwrap();
        for n in 0..800 {
            fs::write(tree.join(format!("entry-{n:04}")), []).unwrap();
        }
        let image = temp.path().join("test.dmg");
        let mut identity = None;
        write_image(&tree, &image, "Test Volume", |volume| {
            identity = Some(volume.entry_info("/payload")?);
            Ok(Some(b"generated after identities".to_vec()))
        })
        .unwrap();
        let mut device = DmgBackend::open(&image).unwrap();
        let report =
            fstool::fs::hfs_plus::verify::verify(&mut device, &Default::default()).unwrap();
        assert!(report.is_clean(), "{:?}", report.diagnostics);
        let mut volume = HfsPlus::open(&mut device).unwrap();
        assert_eq!(volume.entry_info("/payload").unwrap(), identity.unwrap());
        assert_eq!(
            volume
                .getattr(&mut device, Path::new("/payload"))
                .unwrap()
                .mode
                & 0o777,
            0o751
        );
        assert_eq!(
            volume.entry_info("/payload").unwrap().cnid,
            volume.entry_info("/linked").unwrap().cnid
        );
        assert_eq!(
            volume
                .read_symlink_target_path(&mut device, "/Applications")
                .unwrap(),
            "/Applications"
        );
        let attrs = volume
            .list_xattrs(&mut device, Path::new("/payload"))
            .unwrap();
        assert!(attrs.iter().any(|x| x.name == attr && x.value == value));
        let root_attrs = volume.list_xattrs(&mut device, Path::new("/")).unwrap();
        assert!(
            root_attrs
                .iter()
                .any(|x| x.name == attr && x.value == b"root metadata")
        );
        #[cfg(target_os = "macos")]
        assert!(
            attrs
                .iter()
                .any(|x| x.name == "com.apple.ResourceFork" && x.value == b"resource fork")
        );
        let mut generated = Vec::new();
        volume
            .open_file_reader(&mut device, "/.DS_Store")
            .unwrap()
            .read_to_end(&mut generated)
            .unwrap();
        assert_eq!(generated, b"generated after identities");
    }

    #[test]
    fn rejects_hfs_case_collisions_and_propagates_decoration_failures() {
        let temp = tempfile::tempdir().unwrap();
        let tree = temp.path().join("tree");
        fs::create_dir(&tree).unwrap();
        fs::write(tree.join("Payload"), b"one").unwrap();
        let image = temp.path().join("test.dmg");
        let error = write_image(&tree, &image, "Test", |_| {
            anyhow::bail!("decoration failed")
        })
        .unwrap_err();
        assert!(error.to_string().contains("decoration failed"));
        // A case-insensitive host cannot materialize this collision.
        fs::write(tree.join("payload"), b"two").unwrap();
        if fs::read_dir(&tree).unwrap().count() == 2 {
            assert!(write_image(&tree, &image, "Test", |_| Ok(None)).is_err());
        }
    }
}
