//! Provisioning profile input validation and bundle embedding.

mod der;

use anyhow::Context;
use plist::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// A CMS-wrapped Apple provisioning profile ready to embed into a bundle.
///
/// Reading checks the CMS envelope, Apple's signing certificate chain, and
/// profile fields. Bundle builders reconcile signing entitlements separately
/// before embedding and signing.
pub struct ProvisioningProfile {
    source: PathBuf,
    bytes: Vec<u8>,
    team_identifiers: Option<Vec<String>>,
    developer_certificates: Option<Vec<Vec<u8>>>,
    entitlement_values: plist::Dictionary,
    platforms: Option<Vec<String>>,
    application_prefix: String,
    application_identifier: String,
}

/// A validated profile paired with concrete entitlements for one bundle.
/// Signing cannot consume a profile that has skipped reconciliation.
pub(super) struct PreparedProfile {
    profile: ProvisioningProfile,
    pub(super) entitlements: plist::Dictionary,
}

impl PreparedProfile {
    pub(super) fn for_settings(settings: &crate::Settings) -> anyhow::Result<Option<Self>> {
        let Some(path) = settings.apple_provisioning_profile() else {
            return Ok(None);
        };
        let profile = ProvisioningProfile::read(path, &settings.bundle_identifier())?;
        profile.validate_platform(settings.target_os())?;
        let entitlements = super::entitlements::reconcile(settings, &profile)?;
        if let Some(p12_path) = settings.apple_signing_p12() {
            let password = super::signing::apple_signing_password(settings)?;
            profile.validate_signing_identity(p12_path, &password)?;
        }
        Ok(Some(Self {
            profile,
            entitlements,
        }))
    }

    pub(super) fn profile(&self) -> &ProvisioningProfile {
        &self.profile
    }
}

impl ProvisioningProfile {
    /// Read and validate a profile for the configured application identifier.
    pub fn read(path: &Path, bundle_identifier: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !bundle_identifier.is_empty(),
            "Apple bundle identifier must not be empty when using a provisioning profile"
        );
        let bytes = fs::read(path)
            .with_context(|| format!("Failed to read provisioning profile {path:?}"))?;
        let content = super::apple_trust::verified_profile_content(&bytes)
            .context("Invalid provisioning profile CMS signature or trust")?;
        Self::parse_verified_profile_content(path, bytes, &content, bundle_identifier)
    }

    fn parse_verified_profile_content(
        path: &Path,
        bytes: Vec<u8>,
        content: &[u8],
        bundle_identifier: &str,
    ) -> anyhow::Result<Self> {
        let profile = Value::from_reader(std::io::Cursor::new(content))
            .context("Provisioning profile does not contain a valid plist")?;
        let outer_dictionary = profile
            .as_dictionary()
            .ok_or_else(|| anyhow::anyhow!("Provisioning profile plist must be a dictionary"))?;

        if let Some(der_profile) = outer_dictionary.get("DER-Encoded-Profile") {
            let bytes = der_profile.as_data().ok_or_else(|| {
                anyhow::anyhow!("Provisioning profile DER-Encoded-Profile must contain data")
            })?;
            let payload = super::apple_trust::verified_profile_content(bytes)
                .context("Invalid DER-Encoded-Profile CMS signature or trust")?;
            der::validate(outer_dictionary, &payload)
                .context("Invalid or inconsistent DER provisioning profile")?;
        }
        let dictionary = outer_dictionary;

        let entitlements = dictionary
            .get("Entitlements")
            .and_then(Value::as_dictionary)
            .ok_or_else(|| {
                anyhow::anyhow!("Provisioning profile is missing an Entitlements dictionary")
            })?;
        let application_identifier = entitlements
            .get("application-identifier")
            .or_else(|| entitlements.get("com.apple.application-identifier"))
            .and_then(Value::as_string)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Provisioning profile Entitlements is missing an application identifier"
                )
            })?;
        let (application_prefix, team_prefixed_identifier) = application_identifier
            .split_once('.')
            .ok_or_else(|| anyhow::anyhow!("Provisioning profile application identifier `{application_identifier}` has no application prefix"))?;
        anyhow::ensure!(
            !application_prefix.is_empty() && !team_prefixed_identifier.is_empty(),
            "Provisioning profile application identifier `{application_identifier}` must contain an application prefix and app identifier"
        );
        anyhow::ensure!(
            identifier_matches(team_prefixed_identifier, bundle_identifier),
            "Provisioning profile application identifier `{application_identifier}` does not match bundle identifier `{bundle_identifier}`"
        );

        let expiration = dictionary
            .get("ExpirationDate")
            .and_then(Value::as_date)
            .ok_or_else(|| {
                anyhow::anyhow!("Provisioning profile is missing a valid ExpirationDate")
            })?;
        let expiration = chrono::DateTime::parse_from_rfc3339(&expiration.to_xml_format())
            .context("Provisioning profile has an invalid ExpirationDate")?;
        anyhow::ensure!(
            expiration > chrono::Utc::now(),
            "Provisioning profile expired at {}",
            expiration.to_rfc3339()
        );

        let team_identifiers = optional_string_array(dictionary, "TeamIdentifier")?;
        let application_identifier_prefixes =
            optional_string_array(dictionary, "ApplicationIdentifierPrefix")?;
        if let Some(prefixes) = &application_identifier_prefixes {
            anyhow::ensure!(
                prefixes.iter().any(|prefix| prefix == application_prefix),
                "Provisioning profile application identifier prefix `{application_prefix}` is missing from ApplicationIdentifierPrefix"
            );
        }
        let developer_certificates = match dictionary.get("DeveloperCertificates") {
            Some(Value::Array(values)) => Some(
                values
                    .iter()
                    .map(|value| {
                        value.as_data().map(ToOwned::to_owned).ok_or_else(|| {
                            anyhow::anyhow!(
                                "Provisioning profile DeveloperCertificates must contain data"
                            )
                        })
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?,
            ),
            Some(_) => anyhow::bail!("Provisioning profile DeveloperCertificates must be an array"),
            None => None,
        };

        Ok(Self {
            source: path.to_path_buf(),
            bytes,
            team_identifiers,
            developer_certificates,
            entitlement_values: entitlements.clone(),
            platforms: optional_string_array(dictionary, "Platform")?,
            application_prefix: application_prefix.to_owned(),
            application_identifier: format!("{application_prefix}.{bundle_identifier}"),
        })
    }

    fn validate_platform(&self, target_os: &str) -> anyhow::Result<()> {
        let expected = match target_os {
            "ios" => "iOS",
            "macos" => "OSX",
            _ => anyhow::bail!("Provisioning profiles require an Apple bundle target"),
        };
        if let Some(platforms) = &self.platforms {
            anyhow::ensure!(
                platforms.iter().any(|platform| platform == expected),
                "Provisioning profile Platform does not authorize {target_os}"
            );
        }
        Ok(())
    }

    pub(super) fn entitlement_values(&self) -> &plist::Dictionary {
        &self.entitlement_values
    }

    pub(super) fn application_prefix(&self) -> &str {
        &self.application_prefix
    }

    pub(super) fn application_identifier(&self) -> &str {
        &self.application_identifier
    }

    pub(super) fn team_identifier(&self) -> anyhow::Result<&str> {
        let teams = self.team_identifiers.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "Provisioning profile is missing TeamIdentifier required for signing entitlements"
            )
        })?;
        anyhow::ensure!(
            teams.len() == 1 && !teams[0].is_empty(),
            "Provisioning profile must contain exactly one TeamIdentifier to derive signing entitlements"
        );
        Ok(&teams[0])
    }

    /// Check the configured signing certificate against profile metadata and
    /// Apple's offline trust anchors.
    pub fn validate_signing_identity(&self, p12_path: &Path, password: &str) -> anyhow::Result<()> {
        let p12 = fs::read(p12_path)
            .with_context(|| format!("Failed to read Apple signing certificate {p12_path:?}"))?;
        let (certificate, _) = apple_codesign::cryptography::parse_pfx_data(&p12, password)
            .map_err(|error| {
                anyhow::anyhow!("Failed to read Apple signing certificate: {error}")
            })?;
        self.validate_signing_certificate(&certificate)
    }

    pub(super) fn validate_signing_certificate(
        &self,
        certificate: &x509_certificate::CapturedX509Certificate,
    ) -> anyhow::Result<()> {
        super::apple_trust::validate_provisioning_signing_certificate(certificate)?;
        self.validate_signing_certificate_metadata(certificate)
    }

    fn validate_signing_certificate_metadata(
        &self,
        certificate: &x509_certificate::CapturedX509Certificate,
    ) -> anyhow::Result<()> {
        use apple_codesign::AppleCertificate;

        let developer_certificates = self.developer_certificates.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Provisioning profile is missing DeveloperCertificates required to validate the signing identity")
        })?;
        anyhow::ensure!(
            !developer_certificates.is_empty(),
            "Provisioning profile DeveloperCertificates is empty"
        );
        let certificate_der = certificate.constructed_data();
        anyhow::ensure!(
            developer_certificates
                .iter()
                .any(|candidate| candidate.as_slice() == certificate_der),
            "Apple signing certificate is not listed in the provisioning profile DeveloperCertificates"
        );

        if let Some(team_identifiers) = &self.team_identifiers {
            let team_id = certificate.apple_team_id().ok_or_else(|| {
                anyhow::anyhow!("Apple signing certificate does not contain a team identifier")
            })?;
            anyhow::ensure!(
                team_identifiers
                    .iter()
                    .any(|candidate| candidate == &team_id),
                "Apple signing certificate team identifier `{team_id}` does not match the provisioning profile TeamIdentifier"
            );
        }
        Ok(())
    }

    /// Copy the original CMS bytes to the bundle using an atomic replacement.
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

fn optional_string_array(
    dictionary: &plist::Dictionary,
    key: &str,
) -> anyhow::Result<Option<Vec<String>>> {
    match dictionary.get(key) {
        Some(Value::Array(values)) => Ok(Some(
            values
                .iter()
                .map(|value| {
                    value.as_string().map(str::to_owned).ok_or_else(|| {
                        anyhow::anyhow!("Provisioning profile {key} must contain strings")
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()?,
        )),
        Some(_) => anyhow::bail!("Provisioning profile {key} must be an array"),
        None => Ok(None),
    }
}

fn identifier_matches(profile_identifier: &str, bundle_identifier: &str) -> bool {
    if profile_identifier == "*" {
        true
    } else if let Some(prefix) = profile_identifier.strip_suffix(".*") {
        bundle_identifier.starts_with(&format!("{prefix}."))
    } else {
        profile_identifier == bundle_identifier
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use apple_codesign::{CertificateProfile, create_self_signed_code_signing_certificate};
    use cryptographic_message_syntax::{SignedData, SignedDataBuilder, SignerBuilder};
    use plist::{Date, Dictionary};
    use std::time::{Duration, SystemTime};
    use x509_certificate::{CapturedX509Certificate, EcdsaCurve, KeyAlgorithm};

    fn write_profile(
        directory: &Path,
        application_identifier: &str,
        expiration: SystemTime,
    ) -> (PathBuf, CapturedX509Certificate) {
        let application_prefix = application_identifier
            .split_once('.')
            .map(|(prefix, _)| prefix)
            .unwrap_or_default();
        write_profile_with_prefix(
            directory,
            application_identifier,
            application_prefix,
            expiration,
        )
    }

    fn write_profile_with_prefix(
        directory: &Path,
        application_identifier: &str,
        application_prefix: &str,
        expiration: SystemTime,
    ) -> (PathBuf, CapturedX509Certificate) {
        let (certificate, signing_key) = create_self_signed_code_signing_certificate(
            KeyAlgorithm::Ecdsa(EcdsaCurve::Secp256r1),
            CertificateProfile::AppleDevelopment,
            "TEAM123",
            "Test Developer",
            "US",
            chrono::Duration::hours(1),
        )
        .unwrap();
        let mut entitlements = Dictionary::new();
        entitlements.insert(
            "application-identifier".into(),
            Value::String(application_identifier.into()),
        );
        entitlements.insert(
            "com.apple.developer.team-identifier".into(),
            Value::String("TEAM123".into()),
        );
        entitlements.insert(
            "keychain-access-groups".into(),
            Value::Array(vec![Value::String("TEAM123.com.example.*".into())]),
        );
        entitlements.insert(
            "aps-environment".into(),
            Value::String("development".into()),
        );
        let mut profile = Dictionary::new();
        profile.insert("Entitlements".into(), Value::Dictionary(entitlements));
        profile.insert(
            "TeamIdentifier".into(),
            Value::Array(vec![Value::String("TEAM123".into())]),
        );
        profile.insert(
            "ApplicationIdentifierPrefix".into(),
            Value::Array(vec![Value::String(application_prefix.into())]),
        );
        profile.insert(
            "DeveloperCertificates".into(),
            Value::Array(vec![Value::Data(certificate.constructed_data().to_vec())]),
        );
        profile.insert(
            "ExpirationDate".into(),
            Value::Date(
                Date::from_xml_format(
                    &chrono::DateTime::<chrono::Utc>::from(expiration).to_rfc3339(),
                )
                .unwrap(),
            ),
        );
        let mut plist = Vec::new();
        plist::to_writer_xml(&mut plist, &Value::Dictionary(profile)).unwrap();
        let cms = SignedDataBuilder::default()
            .certificate(certificate.clone())
            .content_inline(plist)
            .signer(SignerBuilder::new(&signing_key, certificate.clone()))
            .build_der()
            .unwrap();
        let path = directory.join("test.mobileprovision");
        fs::write(&path, cms).unwrap();
        (path, certificate)
    }

    // These tests isolate plist policy from trust. Production `read` always
    // verifies both the CMS signature and its Apple certificate chain.
    fn parse_untrusted_fixture(
        path: &Path,
        bundle_identifier: &str,
    ) -> anyhow::Result<ProvisioningProfile> {
        let bytes = fs::read(path).unwrap();
        let cms = SignedData::parse_ber(&bytes).unwrap();
        let content = cms.signed_content().unwrap();
        ProvisioningProfile::parse_verified_profile_content(path, bytes, content, bundle_identifier)
    }

    #[test]
    fn profile_platform_must_authorize_the_bundle_target() {
        let directory = tempfile::tempdir().unwrap();
        let (path, _) = write_profile(
            directory.path(),
            "TEAM123.com.example.app",
            SystemTime::now() + Duration::from_secs(3600),
        );
        let mut profile = parse_untrusted_fixture(&path, "com.example.app").unwrap();
        // Older profiles can omit Platform; a present declaration is binding.
        profile.validate_platform("ios").unwrap();
        for (platforms, ios, macos) in [
            (vec!["iOS"], true, false),
            (vec!["OSX"], false, true),
            (vec!["tvOS"], false, false),
            (vec![], false, false),
            (vec!["iOS", "OSX"], true, true),
        ] {
            profile.platforms = Some(platforms.into_iter().map(str::to_owned).collect());
            assert_eq!(profile.validate_platform("ios").is_ok(), ios);
            assert_eq!(profile.validate_platform("macos").is_ok(), macos);
            assert!(profile.validate_platform("linux").is_err());
        }
    }

    #[test]
    fn accepts_exact_and_wildcard_identifiers() {
        let directory = tempfile::tempdir().unwrap();
        let future = SystemTime::now() + Duration::from_secs(3600);
        let (exact, _) = write_profile(directory.path(), "TEAM123.com.example.app", future);
        assert!(
            parse_untrusted_fixture(&exact, "com.example.app")
                .unwrap()
                .application_identifier()
                .contains("com.example.app")
        );
        assert!(ProvisioningProfile::read(&exact, "com.example.app").is_err());

        let (wildcard, _) = write_profile(directory.path(), "TEAM123.com.example.*", future);
        assert!(
            parse_untrusted_fixture(&wildcard, "com.example.app")
                .unwrap()
                .application_identifier()
                .contains("com.example.app")
        );
        assert!(parse_untrusted_fixture(&wildcard, "com.other.app").is_err());
        let (legacy_prefix, certificate) =
            write_profile(directory.path(), "LEGACY_PREFIX.com.example.app", future);
        let legacy_profile = parse_untrusted_fixture(&legacy_prefix, "com.example.app").unwrap();
        legacy_profile
            .validate_signing_certificate_metadata(&certificate)
            .unwrap();
        assert!(identifier_matches("*", "com.any.bundle"));
        assert!(!identifier_matches("com.example*", "com.example.app"));
    }

    #[test]
    fn rejects_mismatch_expiration_and_malformed_cms() {
        let directory = tempfile::tempdir().unwrap();
        let future = SystemTime::now() + Duration::from_secs(3600);
        let (mismatch, _) = write_profile(directory.path(), "TEAM123.com.other.app", future);
        assert!(parse_untrusted_fixture(&mismatch, "com.example.app").is_err());

        let (bad_prefix, _) = write_profile_with_prefix(
            directory.path(),
            "TEAM123.com.example.app",
            "WRONG_PREFIX",
            future,
        );
        assert!(parse_untrusted_fixture(&bad_prefix, "com.example.app").is_err());

        let (expired, _) = write_profile(
            directory.path(),
            "TEAM123.com.example.app",
            SystemTime::now() - Duration::from_secs(3600),
        );
        assert!(parse_untrusted_fixture(&expired, "com.example.app").is_err());

        let malformed = directory.path().join("malformed.mobileprovision");
        fs::write(&malformed, b"not CMS").unwrap();
        assert!(ProvisioningProfile::read(&malformed, "com.example.app").is_err());
    }

    #[test]
    fn reconciles_requested_entitlements_as_authorized_concrete_subsets() {
        use clap::Parser;

        let directory = tempfile::tempdir().unwrap();
        let (profile_path, _) = write_profile(
            directory.path(),
            "TEAM123.com.example.app",
            SystemTime::now() + Duration::from_secs(3600),
        );
        let profile = parse_untrusted_fixture(&profile_path, "com.example.app").unwrap();
        fs::create_dir(directory.path().join("src")).unwrap();
        fs::write(directory.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        let request_path = directory.path().join("requested.plist");
        let mut requested = Dictionary::new();
        requested.insert(
            "keychain-access-groups".into(),
            Value::Array(vec![Value::String("TEAM123.com.example.shared".into())]),
        );
        requested.insert("aps-environment".into(), Value::String("production".into()));
        Value::Dictionary(requested)
            .to_file_xml(&request_path)
            .unwrap();
        let manifest = directory.path().join("Cargo.toml");
        fs::write(
            &manifest,
            format!(
                "[package]\nname='example'\nversion='1.0.0'\nedition='2024'\n[package.metadata.bundle]\nname='Example'\nidentifier='com.example.app'\n[package.metadata.bundle.ios]\napple_signing_entitlements={:?}\n",
                request_path.to_string_lossy()
            ),
        )
        .unwrap();
        let cli = crate::Cli::parse_from([
            "cargo-bundle",
            "--format",
            "ipa",
            "--target",
            "aarch64-apple-ios",
        ]);
        let settings = crate::bundle::Settings::new(directory.path().to_path_buf(), &cli).unwrap();

        assert!(super::super::entitlements::reconcile(&settings, &profile).is_err());

        let mut requested = Dictionary::new();
        requested.insert(
            "keychain-access-groups".into(),
            Value::Array(vec![Value::String("TEAM123.com.example.shared".into())]),
        );
        Value::Dictionary(requested)
            .to_file_xml(&request_path)
            .unwrap();
        let resolved = super::super::entitlements::reconcile(&settings, &profile).unwrap();
        assert_eq!(
            resolved
                .get("application-identifier")
                .and_then(Value::as_string),
            Some("TEAM123.com.example.app")
        );
        assert_eq!(
            resolved
                .get("com.apple.developer.team-identifier")
                .and_then(Value::as_string),
            Some("TEAM123")
        );
        assert_eq!(
            resolved
                .get("keychain-access-groups")
                .and_then(Value::as_array)
                .unwrap()[0]
                .as_string(),
            Some("TEAM123.com.example.shared")
        );
    }

    #[cfg(unix)]
    #[test]
    fn embed_replaces_destination_symlink_without_following_it() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let (profile_path, _) = write_profile(
            directory.path(),
            "TEAM123.com.example.app",
            SystemTime::now() + Duration::from_secs(3600),
        );
        let profile = parse_untrusted_fixture(&profile_path, "com.example.app").unwrap();
        let outside = directory.path().join("outside");
        fs::write(&outside, b"untouched").unwrap();
        let destination = directory.path().join("embedded.provisionprofile");
        symlink(&outside, &destination).unwrap();

        profile.embed(&destination).unwrap();

        assert_eq!(fs::read(&outside).unwrap(), b"untouched");
        assert_eq!(
            fs::read(destination).unwrap(),
            fs::read(profile_path).unwrap()
        );
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(directory.path().join("embedded.provisionprofile"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o644
        );
    }

    #[test]
    fn requires_cms_signer_and_matches_signing_certificate_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let (path, certificate) = write_profile(
            directory.path(),
            "TEAM123.com.example.app",
            SystemTime::now() + Duration::from_secs(3600),
        );
        let profile = parse_untrusted_fixture(&path, "com.example.app").unwrap();
        assert!(profile.validate_signing_certificate(&certificate).is_err());
        profile
            .validate_signing_certificate_metadata(&certificate)
            .unwrap();

        let (other_certificate, _) = create_self_signed_code_signing_certificate(
            KeyAlgorithm::Ecdsa(EcdsaCurve::Secp256r1),
            CertificateProfile::AppleDevelopment,
            "TEAM456",
            "Other Developer",
            "US",
            chrono::Duration::hours(1),
        )
        .unwrap();
        assert!(
            profile
                .validate_signing_certificate_metadata(&other_certificate)
                .is_err()
        );

        assert!(ProvisioningProfile::read(&path, "com.example.app").is_err());

        let unsigned = SignedDataBuilder::default()
            .content_inline(b"<plist/>".to_vec())
            .build_der()
            .unwrap();
        let unsigned_path = directory.path().join("unsigned.mobileprovision");
        fs::write(&unsigned_path, unsigned).unwrap();
        assert!(ProvisioningProfile::read(&unsigned_path, "com.example.app").is_err());

        let mut tampered = fs::read(&path).unwrap();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        let tampered_path = directory.path().join("tampered.mobileprovision");
        fs::write(&tampered_path, tampered).unwrap();
        assert!(ProvisioningProfile::read(&tampered_path, "com.example.app").is_err());
    }
}
