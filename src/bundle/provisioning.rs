//! Apple provisioning profiles, checked before they are sealed into a bundle.
//!
//! A profile is a CMS envelope around a plist. It is accepted when Apple
//! signed it (see [`trust`]), it has not expired, it covers the bundle
//! identifier and target platform, and, when a signing certificate is
//! configured, it lists that certificate. The original bytes are embedded
//! unchanged.

mod trust;

use super::entitlements::{self, Entitlements, Grant, Grants};
use super::settings::TargetOs;
use crate::Settings;
use anyhow::Context;
use apple_codesign::AppleCertificate;
use plist::Value;
use serde::{Deserialize, Deserializer, de};
use std::path::Path;
use std::time::SystemTime;
use x509_certificate::CapturedX509Certificate;

/// The platforms a profile can be issued for.
#[derive(Debug, Deserialize, PartialEq)]
enum Platform {
    #[serde(rename = "iOS")]
    Ios,
    #[serde(rename = "OSX")]
    MacOs,
    #[serde(other)]
    Other,
}

impl TryFrom<TargetOs> for Platform {
    type Error = anyhow::Error;

    fn try_from(os: TargetOs) -> Result<Self, Self::Error> {
        match os {
            TargetOs::Ios => Ok(Self::Ios),
            TargetOs::MacOs => Ok(Self::MacOs),
            _ => anyhow::bail!("Provisioning profiles need an iOS or macOS target"),
        }
    }
}

/// A DER certificate, stored in a profile as plist data.
#[derive(Debug)]
struct Der(Vec<u8>);

impl<'de> Deserialize<'de> for Der {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Bytes;
        impl de::Visitor<'_> for Bytes {
            type Value = Der;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("certificate data")
            }

            fn visit_bytes<E: de::Error>(self, bytes: &[u8]) -> Result<Der, E> {
                Ok(Der(bytes.to_vec()))
            }
        }
        deserializer.deserialize_bytes(Bytes)
    }
}

/// The parts of a profile plist that are used.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct File {
    entitlements: Grants,
    expiration_date: plist::Date,
    #[serde(default)]
    team_identifier: Vec<String>,
    application_identifier_prefix: Option<Vec<String>>,
    developer_certificates: Option<Vec<Der>>,
    platform: Option<Vec<Platform>>,
}

/// A validated profile and the entitlements to sign with.
#[derive(Debug)]
pub(super) struct Profile {
    bytes: Vec<u8>,
    file: File,
    application_prefix: String,
    application_identifier: String,
    /// Entitlements reconciled with the profile, replacing the plain file.
    pub(super) entitlements: Entitlements,
}

impl Profile {
    /// Read and check `apple_provisioning_profile`, if one is configured.
    /// Call before touching existing output, so a bad profile loses nothing.
    pub(super) fn load(settings: &Settings) -> crate::Result<Option<Self>> {
        let Some(path) = settings.apple_provisioning_profile() else {
            return Ok(None);
        };
        let bytes = std::fs::read(path)
            .with_context(|| format!("Failed to read provisioning profile {path:?}"))?;
        let bundle_identifier = settings.bundle_identifier();
        let mut profile = Self::parse(bytes, &bundle_identifier, settings.target_os())
            .with_context(|| format!("Invalid provisioning profile {path:?}"))?;
        if let Some(certificate) = super::signing::apple_certificate(settings)? {
            profile.check_signing_certificate(&certificate)?;
        }
        let requested = match settings.apple_signing_entitlements() {
            Some(path) => entitlements::read(path)?,
            None => Entitlements::new(),
        };
        profile.entitlements = entitlements::reconcile(
            requested,
            &profile,
            &bundle_identifier,
            settings.target_os(),
        )?;
        Ok(Some(profile))
    }

    /// Verify who signed `bytes`, then check what they say.
    fn parse(bytes: Vec<u8>, bundle_identifier: &str, os: TargetOs) -> crate::Result<Self> {
        let now = chrono::Utc::now();
        let content =
            trust::verified_content(&bytes, now).context("Invalid CMS signature or Apple trust")?;
        let plist = Value::from_reader(std::io::Cursor::new(content))
            .context("The profile does not contain a plist")?;
        // Current profiles carry a second, DER copy with its own Apple signature.
        if let Some(der) = plist
            .as_dictionary()
            .and_then(|values| values.get("DER-Encoded-Profile"))
        {
            let der = der.as_data().context("DER-Encoded-Profile must be data")?;
            trust::verified_content(der, now)
                .context("Invalid DER-Encoded-Profile signature or Apple trust")?;
        }
        Self::from_plist(bytes, &plist, bundle_identifier, os, now)
    }

    /// The checks that need no trust: the profile must fit this app, today.
    fn from_plist(
        bytes: Vec<u8>,
        plist: &Value,
        bundle_identifier: &str,
        os: TargetOs,
        now: chrono::DateTime<chrono::Utc>,
    ) -> crate::Result<Self> {
        let file: File =
            plist::from_value(plist).context("The profile does not have the expected fields")?;

        let application_identifier = ["application-identifier", "com.apple.application-identifier"]
            .into_iter()
            .find_map(|key| match file.entitlements.get(key) {
                Some(Grant::Known(entitlements::Entitlement::Text(text))) => Some(text.as_str()),
                _ => None,
            })
            .context("The profile Entitlements have no application identifier")?;
        let (prefix, pattern) = application_identifier
            .split_once('.')
            .filter(|(prefix, pattern)| !prefix.is_empty() && !pattern.is_empty())
            .with_context(|| {
                format!("Application identifier `{application_identifier}` has no team prefix")
            })?;
        anyhow::ensure!(
            entitlements::grants(pattern, bundle_identifier),
            "Application identifier `{application_identifier}` does not match bundle identifier `{bundle_identifier}`"
        );
        let prefix = prefix.to_owned();

        let expires = chrono::DateTime::<chrono::Utc>::from(SystemTime::from(file.expiration_date));
        anyhow::ensure!(
            expires > now,
            "The profile expired at {}",
            expires.to_rfc3339()
        );

        let platform = Platform::try_from(os)?;
        if let Some(platforms) = &file.platform {
            anyhow::ensure!(
                platforms.contains(&platform),
                "The profile's Platform does not include {platform:?}"
            );
        }
        if let Some(prefixes) = &file.application_identifier_prefix {
            anyhow::ensure!(
                prefixes.contains(&prefix),
                "Prefix `{prefix}` is missing from ApplicationIdentifierPrefix"
            );
        }
        Ok(Self {
            bytes,
            application_identifier: format!("{prefix}.{bundle_identifier}"),
            application_prefix: prefix,
            file,
            entitlements: Entitlements::new(),
        })
    }

    /// The signing certificate must be Apple's, current, and in the profile.
    fn check_signing_certificate(
        &self,
        certificate: &CapturedX509Certificate,
    ) -> crate::Result<()> {
        anyhow::ensure!(
            certificate.apple_root_certificate_chain().is_some(),
            "The Apple signing certificate was not issued by Apple"
        );
        anyhow::ensure!(
            certificate.time_constraints_valid(Some(chrono::Utc::now())),
            "The Apple signing certificate is expired or not yet valid"
        );
        self.check_listed(certificate)
    }

    /// The certificate must be one the profile was issued for, on its team.
    fn check_listed(&self, certificate: &CapturedX509Certificate) -> crate::Result<()> {
        let listed = self
            .file
            .developer_certificates
            .as_ref()
            .context("The profile lists no DeveloperCertificates")?;
        anyhow::ensure!(
            listed
                .iter()
                .any(|Der(listed)| listed == certificate.constructed_data()),
            "The Apple signing certificate is not listed in the profile's DeveloperCertificates"
        );
        if let Some(team) = certificate.apple_team_id() {
            anyhow::ensure!(
                self.file.team_identifier.contains(&team),
                "The signing certificate's team `{team}` is not the profile's team"
            );
        }
        Ok(())
    }

    /// Copy the original bytes into the bundle, to be sealed by the signature.
    pub(super) fn embed(&self, destination: &Path) -> crate::Result<()> {
        std::fs::write(destination, &self.bytes)
            .with_context(|| format!("Failed to embed the provisioning profile at {destination:?}"))
    }

    /// What the profile grants, by entitlement key.
    pub(super) fn granted(&self) -> &Grants {
        &self.file.entitlements
    }

    pub(super) fn application_prefix(&self) -> &str {
        &self.application_prefix
    }

    /// `<prefix>.<bundle identifier>`.
    pub(super) fn application_identifier(&self) -> &str {
        &self.application_identifier
    }

    pub(super) fn team_identifier(&self) -> crate::Result<&str> {
        match self.file.team_identifier.as_slice() {
            [team] if !team.is_empty() => Ok(team),
            _ => anyhow::bail!("The profile must have exactly one TeamIdentifier"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::entitlements::Entitlement;
    use super::trust::test_certificate;
    use super::*;
    use plist::Dictionary;

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }
    fn date(text: &str) -> Value {
        Value::Date(plist::Date::from_xml_format(text).unwrap())
    }

    /// A profile plist for `app_id`, valid until 2100 unless changed.
    fn plist(app_id: &str, certificate: &CapturedX509Certificate) -> Dictionary {
        let prefix = app_id.split_once('.').map_or(app_id, |(prefix, _)| prefix);
        let granted = Dictionary::from_iter([
            ("application-identifier", Value::String(app_id.into())),
            (
                "com.apple.developer.team-identifier",
                Value::String("TEAM123".into()),
            ),
            (
                "keychain-access-groups",
                Value::Array(vec![Value::String("TEAM123.com.example.*".into())]),
            ),
            ("aps-environment", Value::String("development".into())),
        ]);
        Dictionary::from_iter([
            ("Entitlements", Value::Dictionary(granted)),
            (
                "TeamIdentifier",
                Value::Array(vec![Value::String("TEAM123".into())]),
            ),
            (
                "ApplicationIdentifierPrefix",
                Value::Array(vec![Value::String(prefix.into())]),
            ),
            (
                "DeveloperCertificates",
                Value::Array(vec![Value::Data(certificate.constructed_data().to_vec())]),
            ),
            ("ExpirationDate", date("2100-01-01T00:00:00Z")),
            ("Platform", Value::Array(vec![Value::String("iOS".into())])),
        ])
    }

    fn profile(values: Dictionary, id: &str, os: TargetOs) -> crate::Result<Profile> {
        Profile::from_plist(
            b"profile".to_vec(),
            &Value::Dictionary(values),
            id,
            os,
            now(),
        )
    }

    #[test]
    fn the_application_identifier_must_cover_the_bundle_identifier() {
        let cert = test_certificate("TEAM123");
        for (app_id, bundle, ok) in [
            ("TEAM123.com.example.app", "com.example.app", true),
            ("TEAM123.com.example.*", "com.example.app", true),
            ("TEAM123.*", "anything.at.all", true),
            ("TEAM123.com.example.*", "com.other.app", false),
            ("TEAM123.com.example.app", "com.example.app.extra", false),
            ("TEAM123.com.example*", "com.example.app", true),
            ("TEAM123", "com.example.app", false),
        ] {
            let result = profile(plist(app_id, &cert), bundle, TargetOs::Ios);
            assert_eq!(
                result.is_ok(),
                ok,
                "{app_id} for {bundle}: {:?}",
                result.err()
            );
        }
        let profile = profile(
            plist("TEAM123.com.example.*", &cert),
            "com.example.app",
            TargetOs::Ios,
        )
        .unwrap();
        assert_eq!(profile.application_identifier(), "TEAM123.com.example.app");
        assert_eq!(profile.application_prefix(), "TEAM123");
        assert_eq!(profile.team_identifier().unwrap(), "TEAM123");
    }

    #[test]
    fn a_profile_must_be_current_and_for_the_target_platform() {
        let cert = test_certificate("TEAM123");
        let id = "com.example.app";
        let mut expired = plist("TEAM123.com.example.app", &cert);
        expired.insert("ExpirationDate".into(), date("2020-01-01T00:00:00Z"));
        let error = profile(expired, id, TargetOs::Ios).unwrap_err();
        assert!(error.to_string().contains("expired"), "{error:#}");

        let mut wrong_prefix = plist("TEAM123.com.example.app", &cert);
        wrong_prefix.insert(
            "ApplicationIdentifierPrefix".into(),
            Value::Array(vec![Value::String("OTHER".into())]),
        );
        assert!(profile(wrong_prefix, id, TargetOs::Ios).is_err());

        for (platforms, ios, macos) in [
            (vec!["iOS"], true, false),
            (vec!["OSX"], false, true),
            (vec!["tvOS"], false, false),
            (vec![], false, false),
            (vec!["iOS", "OSX"], true, true),
        ] {
            let mut values = plist("TEAM123.com.example.app", &cert);
            values.insert(
                "Platform".into(),
                Value::Array(
                    platforms
                        .iter()
                        .map(|p| Value::String(p.to_string()))
                        .collect(),
                ),
            );
            assert_eq!(
                profile(values.clone(), id, TargetOs::Ios).is_ok(),
                ios,
                "{platforms:?}"
            );
            assert_eq!(
                profile(values, id, TargetOs::MacOs).is_ok(),
                macos,
                "{platforms:?}"
            );
        }
        // Older profiles do not say; a declaration is binding, silence is not.
        let mut silent = plist("TEAM123.com.example.app", &cert);
        silent.remove("Platform");
        assert!(profile(silent, id, TargetOs::Ios).is_ok());
        assert!(profile(plist("TEAM123.com.example.app", &cert), id, TargetOs::Linux).is_err());
    }

    #[test]
    fn the_signing_certificate_must_be_listed_for_the_profiles_team() {
        let cert = test_certificate("TEAM123");
        let profile = profile(
            plist("TEAM123.com.example.app", &cert),
            "com.example.app",
            TargetOs::Ios,
        )
        .unwrap();
        profile.check_listed(&cert).unwrap();
        assert!(
            profile.check_listed(&test_certificate("TEAM123")).is_err(),
            "another certificate"
        );
        // Not Apple's, so the full check refuses it even though it is listed.
        assert!(profile.check_signing_certificate(&cert).is_err());
    }

    #[test]
    fn embedding_writes_the_original_bytes() {
        let cert = test_certificate("TEAM123");
        let profile = profile(
            plist("TEAM123.com.example.app", &cert),
            "com.example.app",
            TargetOs::Ios,
        )
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("embedded.mobileprovision");
        profile.embed(&destination).unwrap();
        assert_eq!(std::fs::read(destination).unwrap(), b"profile");
    }

    fn reconciled(requested: Entitlements, os: TargetOs) -> crate::Result<Entitlements> {
        let cert = test_certificate("TEAM123");
        let mut values = plist("TEAM123.com.example.*", &cert);
        if os == TargetOs::MacOs {
            let granted = values
                .get_mut("Entitlements")
                .unwrap()
                .as_dictionary_mut()
                .unwrap();
            granted.insert(
                "com.apple.application-identifier".into(),
                Value::String("TEAM123.com.example.*".into()),
            );
            values.insert(
                "Platform".into(),
                Value::Array(vec![Value::String("OSX".into())]),
            );
        }
        let profile = profile(values, "com.example.app", os)?;
        entitlements::reconcile(requested, &profile, "com.example.app", os)
    }

    fn text(value: &str) -> Entitlement {
        Entitlement::Text(value.into())
    }

    fn list(items: &[&str]) -> Entitlement {
        Entitlement::List(items.iter().map(|item| item.to_string()).collect())
    }

    fn request<const N: usize>(entries: [(&str, Entitlement); N]) -> Entitlements {
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect()
    }

    #[test]
    fn requested_entitlements_are_reconciled_with_the_profile() {
        let resolved = reconciled(
            request([
                (
                    "keychain-access-groups",
                    list(&["TEAM123.com.example.shared"]),
                ),
                ("aps-environment", text("development")),
            ]),
            TargetOs::Ios,
        )
        .unwrap();
        assert_eq!(
            resolved["application-identifier"],
            text("TEAM123.com.example.app")
        );
        assert_eq!(
            resolved["com.apple.developer.team-identifier"],
            text("TEAM123")
        );
        assert_eq!(
            resolved["keychain-access-groups"],
            list(&["TEAM123.com.example.shared"])
        );
        assert_eq!(
            resolved.len(),
            4,
            "grants that were not requested are not signed"
        );

        // Placeholders expand; anything the profile does not grant is refused.
        let expanded = reconciled(
            request([(
                "keychain-access-groups",
                list(&["$(AppIdentifierPrefix)com.example.$(CFBundleIdentifier)"]),
            )]),
            TargetOs::Ios,
        )
        .unwrap();
        assert_eq!(
            expanded["keychain-access-groups"],
            list(&["TEAM123.com.example.com.example.app"])
        );
        for refused in [
            request([("aps-environment", text("production"))]),
            request([("com.apple.developer.healthkit", Entitlement::Flag(true))]),
            request([("keychain-access-groups", list(&["OTHER.com.example.x"]))]),
            request([("application-identifier", text("TEAM123.com.evil"))]),
            request([("keychain-access-groups", list(&["$(Unknown)"]))]),
            // A pattern the profile grants is still not a value to sign.
            request([("keychain-access-groups", list(&["TEAM123.com.example.*"]))]),
        ] {
            assert!(
                reconciled(refused.clone(), TargetOs::Ios).is_err(),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn macos_allows_its_unrestricted_entitlements_without_a_grant() {
        let sandbox = request([("com.apple.security.app-sandbox", Entitlement::Flag(true))]);
        assert!(reconciled(sandbox.clone(), TargetOs::MacOs).is_ok());
        assert!(
            reconciled(sandbox, TargetOs::Ios).is_err(),
            "iOS has no such exemption"
        );
        // Only the listed ones: anything else still needs a grant on macOS.
        let camera = request([("com.apple.developer.healthkit", Entitlement::Flag(true))]);
        assert!(reconciled(camera, TargetOs::MacOs).is_err());
        let resolved = reconciled(Entitlements::new(), TargetOs::MacOs).unwrap();
        assert_eq!(
            resolved["com.apple.application-identifier"],
            text("TEAM123.com.example.app")
        );
    }
}
