//! Optional platform code-signing helpers.

use crate::Settings;
use anyhow::Context;
use std::path::Path;

pub(super) fn apple_signing_password(settings: &Settings) -> crate::Result<String> {
    match settings.apple_signing_password_env() {
        Some(variable) => std::env::var(variable).with_context(|| {
            format!(
                "Apple signing certificate password environment variable `{variable}` is not set"
            )
        }),
        None => Ok(String::new()),
    }
}

/// Sign a disk image with the pure-Rust `apple-codesign` library.
pub fn sign_apple_path(settings: &Settings, path: &Path) -> crate::Result<()> {
    sign_apple_path_inner(settings, path, false)
}

/// Sign an application bundle with the explicitly configured entitlements.
pub(super) fn sign_apple_bundle(settings: &Settings, path: &Path) -> crate::Result<()> {
    sign_apple_path_inner(settings, path, true)
}

fn sign_apple_path_inner(settings: &Settings, path: &Path, bundle: bool) -> crate::Result<()> {
    use apple_codesign::{
        CodeSignatureFlags, SettingsScope, SigningSettings, UnifiedSigner,
        cryptography::{PrivateKey, parse_pfx_data},
    };

    let Some(p12_path) = settings.apple_signing_p12() else {
        return Ok(());
    };

    let password = apple_signing_password(settings)?;
    let certificate_data = std::fs::read(p12_path)
        .with_context(|| format!("Failed to read Apple signing certificate {p12_path:?}"))?;
    let (certificate, private_key) = parse_pfx_data(&certificate_data, &password)
        .map_err(|error| anyhow::anyhow!("Failed to read Apple signing certificate: {error}"))?;

    let mut signing_settings = SigningSettings::default();
    signing_settings.set_signing_key(private_key.as_key_info_signer(), certificate);
    signing_settings.chain_apple_certificates();
    signing_settings.set_team_id_from_signing_certificate();
    if let Some(timestamp_url) = settings.apple_signing_timestamp_url() {
        signing_settings
            .set_time_stamp_url(timestamp_url)
            .map_err(|error| anyhow::anyhow!("Invalid Apple signing timestamp URL: {error}"))?;
    }
    if bundle && let Some(entitlements_path) = settings.apple_signing_entitlements() {
        let entitlements = super::entitlements::read(entitlements_path)?;
        signing_settings
            .set_entitlements_xml(SettingsScope::Main, entitlements)
            .map_err(|error| anyhow::anyhow!("Invalid Apple signing entitlements: {error}"))?;
    }
    if settings.apple_signing_hardened_runtime() {
        signing_settings.add_code_signature_flags(SettingsScope::Main, CodeSignatureFlags::RUNTIME);
    }

    UnifiedSigner::new(signing_settings)
        .sign_path_in_place(path)
        .map_err(|error| anyhow::anyhow!("Apple code signing failed: {error}"))
}

/// Reject incomplete or unsupported notarization settings before packaging.
pub fn validate_apple_notarization(settings: &Settings) -> crate::Result<()> {
    if settings.apple_notarization_api_key().is_none() {
        return Ok(());
    }

    if settings.apple_signing_p12().is_none()
        || !settings.apple_signing_hardened_runtime()
        || settings.apple_signing_timestamp_url().is_none()
    {
        anyhow::bail!(
            "Apple notarization requires apple_signing_p12, apple_signing_hardened_runtime = true, and apple_signing_timestamp_url"
        );
    }

    #[cfg(not(feature = "apple-notarization"))]
    {
        anyhow::bail!(
            "Apple notarization was requested, but cargo-bundle was built without the `apple-notarization` feature. Rebuild it with `--features apple-notarization`."
        );
    }
    #[cfg(feature = "apple-notarization")]
    Ok(())
}

/// Submit a signed Apple bundle or DMG for notarization, wait for Apple's
/// result, then staple the ticket when notarization is accepted.
pub fn notarize_apple_path(settings: &Settings, path: &Path) -> crate::Result<()> {
    validate_apple_notarization(settings)?;
    let Some(api_key_path) = settings.apple_notarization_api_key() else {
        return Ok(());
    };

    #[cfg(not(feature = "apple-notarization"))]
    {
        let _ = (path, api_key_path);
        unreachable!("notarization feature checked above");
    }

    #[cfg(feature = "apple-notarization")]
    {
        use apple_codesign::{NotarizationUpload, Notarizer, stapling::Stapler};
        use std::time::Duration;

        let wait_seconds = settings.apple_notarization_timeout();
        let notarizer = Notarizer::from_api_key(api_key_path).map_err(|error| {
            anyhow::anyhow!("Failed to load Apple notarization API key: {error}")
        })?;
        let result = notarizer
            .notarize_path(path, None)
            .map_err(|error| anyhow::anyhow!("Apple notarization failed for {path:?}: {error}"))?;
        match result {
            NotarizationUpload::NotaryResponse(_) => {}
            NotarizationUpload::UploadId(id) => {
                eprintln!("Apple notarization submission ID: {id}");
                notarizer
                    .wait_on_notarization_and_fetch_log(&id, Duration::from_secs(wait_seconds))
                    .map_err(anyhow::Error::from)
                    .and_then(|response| response.into_result())
                    .map_err(|error| {
                        anyhow::anyhow!(
                            "Apple notarization failed for {path:?} (submission {id}): {error}"
                        )
                    })?;
            }
        }
        let stapler = Stapler::new().map_err(|error| {
            anyhow::anyhow!("Failed to initialize Apple ticket stapler: {error}")
        })?;
        stapler.staple_path(path).map_err(|error| {
            anyhow::anyhow!("Failed to staple Apple notarization ticket to {path:?}: {error}")
        })?;
        Ok(())
    }
}

/// Sign a Windows executable or installer when configured.
///
/// The implementation is deliberately feature-gated: the vendored
/// osslsigncode implementation is GPL-3.0-or-later.
pub fn sign_windows_artifact(settings: &Settings, artifact_path: &Path) -> crate::Result<()> {
    let Some(config) = settings.windows_signing() else {
        return Ok(());
    };

    #[cfg(not(feature = "windows-signing"))]
    {
        let _ = (artifact_path, config);
        anyhow::bail!(
            "Windows Authenticode signing was requested, but cargo-bundle was built without \
             the `windows-signing` feature. Rebuild it with `--features windows-signing`; \
             that feature links GPL-3.0-or-later code."
        );
    }

    #[cfg(feature = "windows-signing")]
    {
        use osslsigncode::{Credential, Digest, Secret, Timestamp, Unsigned};

        let secret = match &config.certificate_password_env {
            Some(variable) => Secret::value(std::env::var(variable).with_context(|| {
                format!(
                    "Windows signing certificate password environment variable `{variable}` is not set"
                )
            })?),
            // Passwordless P12 files use the empty password. Never prompt in a
            // build pipeline: the user may be running cargo-bundle unattended.
            None => Secret::value(String::new()),
        };
        let credential = Credential::pkcs12(&config.certificate_path, secret);
        let parent = artifact_path.parent().ok_or_else(|| {
            anyhow::anyhow!("Windows signing artifact has no parent directory: {artifact_path:?}")
        })?;
        let temporary_directory = tempfile::tempdir_in(parent)
            .with_context(|| "Failed to create temporary directory for Windows signing")?;
        let output_path = temporary_directory.path().join(
            artifact_path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Windows signing artifact has no file name"))?,
        );

        let mut signing_job = Unsigned::open(artifact_path)
            .map_err(|error| {
                anyhow::anyhow!("Failed to open Windows artifact for signing: {error}")
            })?
            .sign(credential)
            .digest(Digest::Sha256);
        if let Some(timestamp_url) = &config.timestamp_url {
            signing_job = signing_job.timestamp(Timestamp::rfc3161(timestamp_url));
        }
        signing_job
            .output(&output_path)
            .sign()
            .map_err(|error| anyhow::anyhow!("Windows Authenticode signing failed: {error}"))?;

        std::fs::copy(&output_path, artifact_path).with_context(|| {
            format!(
                "Failed to replace unsigned Windows artifact {artifact_path:?} with its signed copy"
            )
        })?;
        Ok(())
    }
}

/// Creates an adjacent Sigstore bundle for every Linux artifact.
pub fn sign_linux_artifacts(
    settings: &Settings,
    artifact_paths: &mut Vec<std::path::PathBuf>,
) -> crate::Result<()> {
    let Some(config) = settings.linux_signing() else {
        return Ok(());
    };

    {
        use sigstore_sign::{SigningContext, oidc::IdentityToken};

        let token = std::env::var(&config.identity_token_env).with_context(|| {
            format!(
                "Linux Sigstore identity token environment variable `{}` is not set",
                config.identity_token_env
            )
        })?;
        let token = IdentityToken::from_jwt(token.as_str())
            .map_err(|error| anyhow::anyhow!("Invalid Linux Sigstore identity token: {error}"))?;
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(async {
            let context = SigningContext::production()
                .await
                .map_err(|error| anyhow::anyhow!("Failed to initialize Sigstore: {error}"))?;
            let signer = context.signer(token);

            let mut signature_paths = Vec::new();
            for artifact in artifact_paths.iter() {
                signature_paths.push(sign_linux_artifact(&signer, artifact).await?);
            }
            artifact_paths.extend(signature_paths);
            Ok(())
        })
    }
}

async fn sign_linux_artifact(
    signer: &sigstore_sign::Signer,
    artifact_path: &Path,
) -> crate::Result<std::path::PathBuf> {
    let artifact = std::fs::File::open(artifact_path)
        .with_context(|| format!("Failed to open Linux artifact {artifact_path:?} for signing"))?;
    let bundle = signer
        .sign_reader(artifact)
        .await
        .map_err(|error| anyhow::anyhow!("Sigstore signing failed: {error}"))?;
    let bundle_path = artifact_path.with_file_name(format!(
        "{}.sigstore.json",
        artifact_path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Linux artifact has no file name: {artifact_path:?}"))?
            .to_string_lossy()
    ));
    let bundle_json = bundle
        .to_json_pretty()
        .with_context(|| "Failed to serialize Sigstore bundle")?;
    std::fs::write(&bundle_path, bundle_json)
        .with_context(|| format!("Failed to write Sigstore bundle {bundle_path:?}"))?;
    Ok(bundle_path)
}
