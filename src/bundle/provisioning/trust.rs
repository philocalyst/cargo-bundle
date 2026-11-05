//! Offline Apple trust for provisioning profiles: who signed one, and whether
//! Apple's roots stand behind that signer. Apple's roots come from
//! `apple-codesign`; no system trust store or network is consulted.

use anyhow::Context;
use apple_codesign::KnownCertificate;
use bcder::{Mode, Tag};
use cryptographic_message_syntax::SignedData;
use x509_certificate::CapturedX509Certificate;

/// The plist (or DER) content of a profile CMS envelope, once its signature
/// and the Apple certificate chain behind it check out at `now`.
pub(super) fn verified_content(
    bytes: &[u8],
    now: chrono::DateTime<chrono::Utc>,
) -> crate::Result<Vec<u8>> {
    let cms = SignedData::parse_ber(bytes).context("Not a CMS envelope")?;
    // The CMS crate re-encodes certificates, which can change what their
    // issuers signed, so chains are built from the bytes as shipped.
    let certificates = original_certificates(bytes)?;
    let signers: Vec<_> = cms.signers().collect();
    anyhow::ensure!(!signers.is_empty(), "The envelope has no signer");
    for signer in signers {
        let (issuer, serial) = signer
            .certificate_issuer_and_serial()
            .context("The signer is not identified by issuer and serial")?;
        let mut candidates = certificates
            .iter()
            .filter(|cert| cert.serial_number_asn1() == serial && cert.issuer_name() == issuer);
        let (Some(leaf), None) = (candidates.next(), candidates.next()) else {
            anyhow::bail!("The signer must match exactly one embedded certificate");
        };
        if signer.signed_attributes().is_some() {
            signer
                .verify_message_digest_with_signed_data(&cms)
                .context("Message digest verification failed")?;
        }
        signer
            .signature_verifier(std::iter::once(leaf))?
            .verify(
                &signer.signed_content_with_signed_data(&cms),
                signer.signature(),
            )
            .map_err(|_| anyhow::anyhow!("Signature verification failed"))?;
        verify_profile_signer(leaf, &certificates, now)?;
    }
    cms.signed_content()
        .map(<[u8]>::to_vec)
        .context("The envelope has no content")
}

/// Only Apple's profile-signing identities may sign a profile; a developer's
/// own Apple-issued certificate chains to the same root but must not.
fn verify_profile_signer(
    leaf: &CapturedX509Certificate,
    supplied: &[CapturedX509Certificate],
    now: chrono::DateTime<chrono::Utc>,
) -> crate::Result<()> {
    let signer = leaf.subject_common_name().unwrap_or_default();
    // (issuing CA, whether certificate dates apply). Apple's iOS profile policy
    // does not check dates, and the profile's own ExpirationDate is checked.
    let (issuer, dated) = match signer.as_str() {
        "Apple iPhone OS Provisioning Profile Signing" => {
            ("Apple iPhone Certification Authority", false)
        }
        "Mac OS X Provisioning Profile Signing" => (
            "Apple Worldwide Developer Relations Certification Authority",
            true,
        ),
        "WWDR Provisioning Profile Signing" => ("Apple System Integration CA 4", true),
        _ => anyhow::bail!("The signer `{signer}` is not an Apple provisioning profile signer"),
    };
    let known: Vec<_> = KnownCertificate::all()
        .iter()
        .map(|cert| (*cert).clone())
        .collect();
    let pool: Vec<_> = supplied.iter().chain(&known).collect();
    let mut chain = vec![leaf];
    anyhow::ensure!(
        extend_to_root(&mut chain, &pool),
        "The signer does not chain to a pinned Apple root"
    );
    check_shape(&chain, issuer, dated.then_some(now))
}

/// A chain `[signer, issuing CA, ..., root]` must be issued by `issuer`, be
/// made of certificate authorities, and, given a time, be valid then.
fn check_shape(
    chain: &[&CapturedX509Certificate],
    issuer: &str,
    valid_at: Option<chrono::DateTime<chrono::Utc>>,
) -> crate::Result<()> {
    anyhow::ensure!(
        chain.len() >= 3 && chain[1].subject_common_name().as_deref() == Some(issuer),
        "The signer was not issued by {issuer}"
    );
    for (index, cert) in chain.iter().enumerate() {
        anyhow::ensure!(
            valid_at.is_none() || cert.time_constraints_valid(valid_at),
            "A certificate in the signer's chain is not currently valid"
        );
        anyhow::ensure!(
            index == 0 || is_ca(cert),
            "A certificate in the signer's chain is not a certificate authority"
        );
    }
    Ok(())
}

/// Grow `chain` with issuers from `pool` until it ends at a pinned Apple root.
fn extend_to_root<'a>(
    chain: &mut Vec<&'a CapturedX509Certificate>,
    pool: &[&'a CapturedX509Certificate],
) -> bool {
    let current = *chain.last().expect("a chain starts with its leaf");
    if is_known_root(current) {
        return true;
    }
    if chain.len() >= 6 {
        return false;
    }
    for &candidate in pool {
        if candidate.subject_name() == current.issuer_name()
            && !chain.contains(&candidate)
            && issued_by(current, candidate).is_ok()
        {
            chain.push(candidate);
            if extend_to_root(chain, pool) {
                return true;
            }
            chain.pop();
        }
    }
    false
}

/// Check that `issuer` signed `certificate`. `x509-certificate`'s own check
/// cannot verify an ECDSA P-384 issuer such as Apple Root CA - G3, so the
/// verification algorithm is resolved from both certificates' key types.
fn issued_by(
    certificate: &CapturedX509Certificate,
    issuer: &CapturedX509Certificate,
) -> crate::Result<()> {
    let parsed: &x509_certificate::rfc5280::Certificate = certificate.as_ref();
    let signed = parsed
        .tbs_certificate
        .raw_data
        .as_deref()
        .context("A certificate has no original TBS bytes")?;
    let algorithm = certificate
        .signature_signature_algorithm()
        .context("Unsupported certificate signature algorithm")?
        .resolve_verification_algorithm(
            issuer
                .key_algorithm()
                .context("Unsupported issuer key algorithm")?,
        )
        .context("The signature and issuer key algorithms do not match")?;
    issuer
        .verify_signed_data_with_algorithm(signed, parsed.signature.octet_bytes(), algorithm)
        .context("Certificate signature verification failed")
}

fn is_known_root(cert: &CapturedX509Certificate) -> bool {
    KnownCertificate::all_roots()
        .iter()
        .any(|root| root.constructed_data() == cert.constructed_data())
}

/// BasicConstraints (2.5.29.19) must start with `cA = TRUE`.
fn is_ca(cert: &CapturedX509Certificate) -> bool {
    cert.iter_extensions()
        .find(|extension| extension.id.as_ref() == [85, 29, 19])
        .is_some_and(|extension| {
            let value = extension.value.clone().into_bytes();
            value.len() >= 5 && value[0] == 0x30 && value[2..5] == [0x01, 0x01, 0xff]
        })
}

/// The certificates of a CMS envelope exactly as they were encoded.
fn original_certificates(bytes: &[u8]) -> crate::Result<Vec<CapturedX509Certificate>> {
    let raw: Vec<Vec<u8>> = Mode::Ber
        .decode(bytes, |content| {
            content.take_sequence(|content| {
                // ContentInfo ::= SEQUENCE { contentType, [0] EXPLICIT SignedData }
                content.skip_one()?;
                content.take_constructed_if(Tag::CTX_0, |content| {
                    content.take_sequence(|content| {
                        // version, digestAlgorithms, encapContentInfo
                        for _ in 0..3 {
                            content.skip_one()?;
                        }
                        let certificates =
                            content.take_opt_constructed_if(Tag::CTX_0, |content| {
                                let mut certificates = Vec::new();
                                loop {
                                    let mut present = false;
                                    let captured = content.capture(|content| {
                                        present = content.skip_one()?.is_some();
                                        Ok(())
                                    })?;
                                    if !present {
                                        return Ok(certificates);
                                    }
                                    certificates.push(AsRef::<[u8]>::as_ref(&captured).to_vec());
                                }
                            })?;
                        content.skip_all()?;
                        Ok(certificates.unwrap_or_default())
                    })
                })
            })
        })
        .context("Unable to read the envelope's certificates")?;
    raw.into_iter()
        .map(|cert| CapturedX509Certificate::from_ber(cert).context("Invalid embedded certificate"))
        .collect()
}

#[cfg(test)]
pub(super) fn test_certificate(team: &str) -> CapturedX509Certificate {
    use apple_codesign::{CertificateProfile, create_self_signed_code_signing_certificate};
    use x509_certificate::{EcdsaCurve, KeyAlgorithm};
    create_self_signed_code_signing_certificate(
        KeyAlgorithm::Ecdsa(EcdsaCurve::Secp256r1),
        CertificateProfile::AppleDevelopment,
        team,
        "Test Developer",
        "US",
        chrono::Duration::hours(1),
    )
    .unwrap()
    .0
}

#[cfg(test)]
mod tests {
    use super::*;
    use cryptographic_message_syntax::{SignedDataBuilder, SignerBuilder};
    use plist::Value;
    use x509_certificate::{
        EcdsaCurve, KeyAlgorithm, X509CertificateBuilder, certificate::KeyUsage,
    };

    const FASTLANE_IOS: &[u8] =
        include_bytes!("../../../tests/fixtures/apple/fastlane-test.mobileprovision");
    const DER_PROFILES: [&[u8]; 3] = [
        include_bytes!("../../../tests/fixtures/apple/der-ios.mobileprovision"),
        include_bytes!("../../../tests/fixtures/apple/der-ios-signin.mobileprovision"),
        include_bytes!("../../../tests/fixtures/apple/der-macos.provisionprofile"),
    ];
    const DEVELOPER_ID: &[u8] =
        include_bytes!("../../../tests/fixtures/apple/apple-signed-developer-id-application.cer");

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }

    /// The part of `bytes` that `needle` points at, flipped by one bit.
    fn tamper(bytes: &[u8], needle: &[u8]) -> Vec<u8> {
        let at: Vec<_> = bytes
            .windows(needle.len())
            .enumerate()
            .filter_map(|(index, window)| (window == needle).then_some(index))
            .collect();
        assert_eq!(at.len(), 1, "the fixture holds the bytes once");
        let mut tampered = bytes.to_vec();
        tampered[at[0]] ^= 1;
        tampered
    }

    #[test]
    fn profiles_signed_by_apple_verify_and_yield_their_plist() {
        for bytes in [
            FASTLANE_IOS,
            DER_PROFILES[0],
            DER_PROFILES[1],
            DER_PROFILES[2],
        ] {
            let content = verified_content(bytes, now()).unwrap();
            assert!(Value::from_reader(std::io::Cursor::new(content)).is_ok());
        }
        // The DER copy inside current profiles has its own signature.
        for bytes in DER_PROFILES {
            let content = verified_content(bytes, now()).unwrap();
            let plist = Value::from_reader(std::io::Cursor::new(content)).unwrap();
            let der = plist.as_dictionary().unwrap()["DER-Encoded-Profile"]
                .as_data()
                .unwrap();
            verified_content(der, now()).unwrap();
        }
    }

    #[test]
    fn tampered_profiles_are_rejected() {
        let cms = SignedData::parse_ber(FASTLANE_IOS).unwrap();
        let content = cms.signed_content().unwrap();
        let signature = cms.signers().next().unwrap().signature();
        for (needle, message) in [
            (content, "Message digest"),
            (signature, "Signature verification failed"),
        ] {
            let error = verified_content(&tamper(FASTLANE_IOS, needle), now()).unwrap_err();
            assert!(error.to_string().contains(message), "{error:#}");
        }
        assert!(verified_content(b"not a CMS envelope", now()).is_err());
    }

    #[test]
    fn a_chain_that_is_altered_stops_reaching_an_apple_root() {
        let plist = Value::from_reader(std::io::Cursor::new(
            verified_content(DER_PROFILES[0], now()).unwrap(),
        ))
        .unwrap();
        let der = plist.as_dictionary().unwrap()["DER-Encoded-Profile"]
            .as_data()
            .unwrap();
        for subject in [
            "WWDR Provisioning Profile Signing",
            "Apple System Integration CA 4",
        ] {
            let certificates = original_certificates(der).unwrap();
            let certificate = certificates
                .iter()
                .find(|cert| cert.subject_common_name().as_deref() == Some(subject))
                .unwrap();
            let parsed: &x509_certificate::rfc5280::Certificate = certificate.as_ref();
            let error =
                verified_content(&tamper(der, parsed.signature.octet_bytes().as_ref()), now())
                    .unwrap_err();
            assert!(
                format!("{error:#}").contains("pinned Apple root"),
                "{subject}: {error:#}"
            );
        }
    }

    fn self_signed_signer(name: &str) -> (CapturedX509Certificate, Vec<u8>) {
        let mut builder = X509CertificateBuilder::default();
        builder
            .subject()
            .append_common_name_utf8_string(name)
            .unwrap();
        builder.constraint_not_ca();
        builder.key_usage(KeyUsage::DigitalSignature);
        let (certificate, key) = builder
            .create_with_random_keypair(KeyAlgorithm::Ecdsa(EcdsaCurve::Secp256r1))
            .unwrap();
        let cms = SignedDataBuilder::default()
            .certificate(certificate.clone())
            .content_inline(b"forged profile".to_vec())
            .signer(SignerBuilder::new(&key, certificate.clone()))
            .build_der()
            .unwrap();
        (certificate, cms)
    }

    #[test]
    fn only_apples_profile_signers_are_accepted() {
        // A certificate named like Apple's signer, but not issued by Apple.
        for name in [
            "Apple iPhone OS Provisioning Profile Signing",
            "Mac OS X Provisioning Profile Signing",
            "WWDR Provisioning Profile Signing",
        ] {
            let (_, cms) = self_signed_signer(name);
            let error = verified_content(&cms, now()).unwrap_err();
            assert!(
                format!("{error:#}").contains("pinned Apple root"),
                "{name}: {error:#}"
            );
        }
        // A real Apple-issued certificate that is not a profile signer.
        let developer = CapturedX509Certificate::from_der(DEVELOPER_ID.to_vec()).unwrap();
        let error = verify_profile_signer(&developer, &[], now()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("not an Apple provisioning profile signer")
        );
        let (_, cms) = self_signed_signer("Some Other Name");
        assert!(verified_content(&cms, now()).is_err());
    }

    #[test]
    fn a_chain_must_be_issued_by_the_expected_authority_of_authorities() {
        let signer = test_certificate("TEAM123");
        let developer = CapturedX509Certificate::from_der(DEVELOPER_ID.to_vec()).unwrap();
        let current = |cert: &&&CapturedX509Certificate| cert.time_constraints_valid(Some(now()));
        let root = KnownCertificate::all_roots().iter().find(current).unwrap();
        let ca = KnownCertificate::all()
            .iter()
            .filter(|cert| is_ca(cert) && !is_known_root(cert))
            .find(current)
            .unwrap();
        let name = ca.subject_common_name().unwrap();

        check_shape(&[&signer, ca, root], &name, Some(now())).unwrap();
        let error = check_shape(&[&signer, ca, root], "Some Other CA", None).unwrap_err();
        assert!(error.to_string().contains("not issued by"), "{error}");
        let error = check_shape(
            &[&signer, &developer, root],
            &developer.subject_common_name().unwrap(),
            None,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("not a certificate authority"),
            "{error}"
        );
        assert!(
            check_shape(&[&signer, root], &name, None).is_err(),
            "too short"
        );
        // Real certificates are checked against the time given.
        let long_ago = chrono::DateTime::from_timestamp(0, 0);
        assert!(check_shape(&[&signer, ca, root], &name, long_ago).is_err());
    }
}
