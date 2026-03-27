// A macOS DMG (disk image) bundle is a compressed disk image that contains the
// application bundle and a symlink to /Applications so the user can simply
// drag-and-drop to install.
//
// The layout inside the mounted volume is:
//
//   <AppName>.dmg  (read-only compressed UDZO image)
//     <AppName>.app   # the application bundle
//     Applications -> /Applications  # drag-and-drop install target
//
// Building requires macOS because the `hdiutil` command is used to create and
// convert the disk image.

/// Walk a directory and sum the sizes of all contained files.
fn dir_size(dir: &std::path::Path) -> crate::Result<u64> {
    let mut total: u64 = 0;
    for entry in walkdir::WalkDir::new(dir) {
        let entry = entry?;
        if entry.file_type().is_file() {
            total += entry.metadata()?.len();
        }
    }
    Ok(total)
}

/// Extract the mount point path from `hdiutil attach` stdout.
fn parse_mount_point(stdout: &[u8]) -> crate::Result<PathBuf> {
    parse_mount_point_impl(stdout)
}

