//! Read and embed user-supplied provisioning profiles.

use anyhow::Context;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub(super) struct PreparedProfile {
    source: PathBuf,
    bytes: Vec<u8>,
}

impl PreparedProfile {
    pub(super) fn for_settings(settings: &crate::Settings) -> anyhow::Result<Option<Self>> {
        let Some(path) = settings.apple_provisioning_profile() else {
            return Ok(None);
        };
        let bytes = fs::read(path)
            .with_context(|| format!("Failed to read provisioning profile {path:?}"))?;
        Ok(Some(Self {
            source: path.to_path_buf(),
            bytes,
        }))
    }

    /// Copy the original profile bytes to the bundle using an atomic replacement.
    pub fn embed(&self, destination: &Path) -> anyhow::Result<()> {
        let parent = destination.parent().ok_or_else(|| {
            anyhow::anyhow!("Provisioning profile destination has no parent: {destination:?}")
        })?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).with_context(|| {
            format!("Failed to create temporary profile beside {destination:?}")
        })?;
        temporary.write_all(&self.bytes).with_context(|| {
            format!(
                "Failed to write provisioning profile from {:?}",
                self.source
            )
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temporary
                .as_file()
                .set_permissions(fs::Permissions::from_mode(0o644))
                .with_context(|| {
                    format!("Failed to set provisioning profile permissions for {destination:?}")
                })?;
        }
        temporary
            .as_file()
            .sync_all()
            .with_context(|| format!("Failed to flush provisioning profile to {destination:?}"))?;
        temporary
            .persist(destination)
            .map_err(|error| error.error)
            .with_context(|| format!("Failed to embed provisioning profile at {destination:?}"))?;
        Ok(())
    }
}
