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

