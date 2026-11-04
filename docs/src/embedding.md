# Embed the image libraries

Cargo-bundle uses the published `fstool` 0.5.2 library for HFS+ authoring,
filesystem inspection, UDIF checksum verification and compressed disk-image
encoding. Enable `std`, `hfs-plus`, `dmg` and `dmg-write-zlib` for those APIs.

`HfsPlusSizePlan` accounts for the imported tree, resource forks, xattrs and
files generated after catalog identities become available. `HfsPlus::entry_info`
and `volume_create_date` expose those identities before `flush`; `write_udif`
encodes the finished filesystem. Keep output in a temporary candidate and
publish it only after encoding, syncing and signing succeed.

Finder aliases remain in `alias_record` (library name `dmg_layout`), and
`.DS_Store` records remain in `ds_parser`. `appimage-rs` assembles AppImages
with embedded launchers. `rust-apk` (library name `apk`) compiles resources,
validates native libraries, aligns packages and signs Android APKs.

The retired `dmg-rs` adapter and patched fstool sibling checkout are no longer
required. See [release source setup](contributing.md#appimage-release-source)
for the AppImage crate awaiting publication.
