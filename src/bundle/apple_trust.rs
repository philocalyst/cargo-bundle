//! Offline Apple certificate path checks used for provisioning-backed signing.
//!
//! Apple roots and known intermediates come from `apple-codesign`'s embedded
//! certificate set. The current inner DER-profile signer uses Apple System
//! Integration CA 4, which is pinned here by SHA-256 because it is not in that
//! embedded set. No system trust store or network access is consulted.

use anyhow::Context;
use apple_codesign::{AppleCertificate, KnownCertificate};
use cryptographic_message_syntax::{SignedData, SignerInfo};
use sha2::{Digest, Sha256};
use x509_certificate::CapturedX509Certificate;

// Apple currently uses this certificate to sign the inner DER profile CMS.
// It is not in apple-codesign's embedded intermediate set, so pin its exact
// SHA-256 fingerprint. The hash is from the CA certificate included in the
// public Apple-signed fixtures at
// https://github.com/dotnet/macios/tree/27b2ccb2f13349d3767f4cf69c904e46876bc81a/tests/msbuild/Xamarin.MacDev.Tasks.Tests/Resources
// and the chain must still verify to an exact root in apple-codesign's set.
const DER_PROFILE_INTERMEDIATE_SHA256: [u8; 32] = [
    0x7d, 0xa7, 0x9b, 0xa4, 0x8b, 0x1c, 0x16, 0x08, 0x52, 0x43, 0x3c, 0x15, 0xa6, 0x84, 0xdf, 0xe4,
    0x74, 0x7b, 0x81, 0xd1, 0x72, 0x07, 0x87, 0xc3, 0x39, 0x60, 0x9c, 0xab, 0x7c, 0x68, 0x32, 0x6c,
];

const DER_PROFILE_SIGNER_MARKER_OID: &[u8] = &[42, 134, 72, 134, 247, 99, 100, 12, 19];

/// Return embedded profile content after verifying its CMS signer and Apple chain.
/// Original certificate bytes are preserved: the CMS dependency's certificate
/// round-trip can change signature input. Re-encoded certificates never escape
/// through this interface.
pub(super) fn verified_profile_content(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let cms = parse_provisioning_cms(bytes)?;
    cms.signed_content()
        .map(<[u8]>::to_vec)
        .context("Provisioning profile CMS has no embedded content")
}

fn parse_provisioning_cms(bytes: &[u8]) -> anyhow::Result<SignedData> {
    let cms = SignedData::parse_ber(bytes).context("Invalid provisioning profile CMS")?;
    let original_certificates = extract_original_cms_certificates(bytes)?;
    validate_provisioning_cms_with_certificates(&cms, &original_certificates)?;
    Ok(cms)
}

fn extract_original_cms_certificates(bytes: &[u8]) -> anyhow::Result<Vec<CapturedX509Certificate>> {
    use bcder::{Mode, Tag};

    let raw_certificates: Vec<Vec<u8>> = Mode::Ber
        .decode(bytes, |content| {
            content.take_sequence(|content| {
                // ContentInfo ::= SEQUENCE { signedData OID, [0] EXPLICIT ... }
                content.skip_one()?;
                content.take_constructed_if(Tag::CTX_0, |content| {
                    content.take_sequence(|content| {
                        // version, digestAlgorithms, and encapContentInfo.
                        content.skip_one()?;
                        content.skip_one()?;
                        content.skip_one()?;
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
                                        break;
                                    }
                                    certificates.push(AsRef::<[u8]>::as_ref(&captured).to_vec());
                                }
                                Ok(certificates)
                            })?;
                        content.skip_all()?;
                        Ok(certificates.unwrap_or_default())
                    })
                })
            })
        })
        .context("Unable to extract original provisioning CMS certificates")?;
    raw_certificates
        .into_iter()
        .map(|bytes| {
            CapturedX509Certificate::from_ber(bytes)
                .context("Invalid original embedded provisioning CMS certificate")
        })
        .collect()
}

fn validate_provisioning_cms_with_certificates(
    cms: &SignedData,
    original_certificates: &[CapturedX509Certificate],
) -> anyhow::Result<()> {
    let signers = cms.signers().collect::<Vec<_>>();
    anyhow::ensure!(
        !signers.is_empty(),
        "Provisioning profile CMS must contain at least one signer"
    );

    for signer in signers {
        let certificate = signer_certificate(signer, original_certificates)?;
        if signer.signed_attributes().is_some() {
            signer
                .verify_message_digest_with_signed_data(cms)
                .context("Provisioning profile CMS message digest verification failed")?;
        }
        let signed = signer.signed_content_with_signed_data(cms);
        signer
            .signature_verifier(std::iter::once(&certificate))?
            .verify(&signed, signer.signature())
            .map_err(|_| {
                anyhow::anyhow!("Provisioning profile CMS signature verification failed")
            })?;
        let (policy, expected_intermediate) = provisioning_profile_signer_purpose(&certificate)?;
        validate_signing_leaf_constraints(&certificate)?;
        validate_apple_chain_with_candidates(
            &certificate,
            original_certificates.to_vec(),
            expected_intermediate,
            policy,
            chrono::Utc::now(),
            !matches!(policy, ProfileSignerPolicy::Ios),
            "Provisioning profile CMS signer",
        )?;
    }
    Ok(())
}

/// Validate a P12 signing identity for use with an Apple provisioning profile.
pub(super) fn validate_provisioning_signing_certificate(
    certificate: &CapturedX509Certificate,
) -> anyhow::Result<()> {
    validate_provisioning_signing_certificate_at(certificate, chrono::Utc::now())
}

fn validate_provisioning_signing_certificate_at(
    certificate: &CapturedX509Certificate,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<()> {
    let extensions = certificate.apple_code_signing_extensions();
    let is_developer_id = extensions
        .contains(&apple_codesign::CodeSigningCertificateExtension::DeveloperIdApplication);
    let has_supported_application_purpose = has_supported_application_signing_purpose(&extensions);
    anyhow::ensure!(
        has_supported_application_purpose,
        "Apple signing certificate is not an application development, distribution, or Developer ID signing certificate"
    );
    anyhow::ensure!(
        certificate
            .apple_extended_key_usage_purposes()
            .contains(&apple_codesign::ExtendedKeyUsagePurpose::CodeSigning),
        "Apple signing certificate does not permit code signing"
    );
    validate_signing_leaf_constraints(certificate)?;
    let expected_intermediate = if is_developer_id {
        "Developer ID Certification Authority"
    } else {
        "Apple Worldwide Developer Relations Certification Authority"
    };
    validate_apple_chain_with_candidates(
        certificate,
        Vec::new(),
        expected_intermediate,
        ProfileSignerPolicy::Macos,
        now,
        true,
        "Apple signing certificate",
    )
}

fn signer_certificate(
    signer: &SignerInfo,
    certificates: &[CapturedX509Certificate],
) -> anyhow::Result<CapturedX509Certificate> {
    let (issuer, serial) = signer
        .certificate_issuer_and_serial()
        .context("Provisioning profile CMS signer has no issuer and serial identifier")?;
    let matches = certificates
        .iter()
        .filter(|candidate| {
            candidate.serial_number_asn1() == serial && candidate.issuer_name() == issuer
        })
        .collect::<Vec<_>>();
    anyhow::ensure!(
        matches.len() == 1,
        "Provisioning profile CMS signer must identify exactly one embedded certificate"
    );
    Ok(matches[0].clone())
}

fn provisioning_profile_signer_purpose(
    certificate: &CapturedX509Certificate,
) -> anyhow::Result<(ProfileSignerPolicy, &'static str)> {
    let signer_cn = certificate.subject_common_name().unwrap_or_default();
    match signer_cn.as_str() {
        "Apple iPhone OS Provisioning Profile Signing" => Ok((
            ProfileSignerPolicy::Ios,
            "Apple iPhone Certification Authority",
        )),
        "Mac OS X Provisioning Profile Signing" => {
            anyhow::ensure!(
                has_extension(certificate, &[42, 134, 72, 134, 247, 99, 100, 4, 11]),
                "macOS provisioning profile signer is missing Apple's Mac provisioning-profile signing marker"
            );
            Ok((
                ProfileSignerPolicy::Macos,
                "Apple Worldwide Developer Relations Certification Authority",
            ))
        }
        "WWDR Provisioning Profile Signing" => {
            anyhow::ensure!(
                has_extension(certificate, DER_PROFILE_SIGNER_MARKER_OID),
                "DER profile signer is missing Apple's provisioning-profile signing marker"
            );
            Ok((
                ProfileSignerPolicy::DerEncoded,
                "Apple System Integration CA 4",
            ))
        }
        _ => anyhow::bail!(
            "Provisioning profile CMS signer has an unsupported signing purpose ({signer_cn})"
        ),
    }
}

fn has_supported_application_signing_purpose(
    extensions: &[apple_codesign::CodeSigningCertificateExtension],
) -> bool {
    use apple_codesign::CodeSigningCertificateExtension as Ext;
    [
        Ext::DeveloperIdApplication,
        Ext::IPhoneDeveloper,
        Ext::AppleDeveloperCertificateSubmission,
        Ext::AppleMacAppSigningDevelopment,
        Ext::AppleMacAppSigningSubmission,
        Ext::MacDeveloper,
    ]
    .iter()
    .any(|purpose| extensions.contains(purpose))
}

#[derive(Clone, Copy, Debug)]
enum ProfileSignerPolicy {
    Ios,
    Macos,
    DerEncoded,
}

fn validate_apple_chain_with_candidates(
    certificate: &CapturedX509Certificate,
    supplied: Vec<CapturedX509Certificate>,
    expected_intermediate: &str,
    policy: ProfileSignerPolicy,
    now: chrono::DateTime<chrono::Utc>,
    check_validity: bool,
    description: &str,
) -> anyhow::Result<()> {
    let mut candidates = supplied;
    candidates.extend(KnownCertificate::all().iter().map(|cert| (*cert).clone()));
    let mut chain = resolve_chain_to_pinned_apple_root(certificate, &candidates)?;
    // The resolver can continue past a trusted root through cross-signed or
    // self-signed root certificates. A pinned root terminates the path.
    if let Some(root_index) = chain.iter().position(|cert| {
        KnownCertificate::all_roots()
            .iter()
            .any(|root| root.constructed_data() == cert.constructed_data())
    }) {
        chain.truncate(root_index + 1);
    }
    anyhow::ensure!(
        chain.len() >= 3,
        "{description} must have a leaf, Apple intermediate(s), and a pinned Apple root"
    );

    for (index, cert) in chain.iter().enumerate() {
        // Apple's iOS provisioning-profile signing policy intentionally omits
        // certificate validity-time checking; see Apple's SecPolicyPriv.h:
        // https://github.com/apple-oss-distributions/Security/blob/main/trust/SecPolicyPriv.h
        // (SecPolicyCreateiPhoneProvisioningProfileSigning). The iOS
        // profile's own ExpirationDate is still checked by provisioning.rs.
        // The legacy iOS signer follows Apple's no-validity-check policy. The
        // modern DER signer is independently scoped to the pinned CA4 path and
        // is checked for current validity, as are macOS profiles and P12 IDs.
        if check_validity {
            anyhow::ensure!(
                cert.time_constraints_valid(Some(now)),
                "{description} certificate in Apple chain is not currently valid"
            );
        }

        let is_root = index + 1 == chain.len();
        if is_root {
            anyhow::ensure!(
                KnownCertificate::all_roots()
                    .iter()
                    .any(|root| root.constructed_data() == cert.constructed_data()),
                "{description} chain does not terminate at an exact pinned Apple root certificate"
            );
        } else if index == 1 {
            // The immediate issuer must have the Apple CA name required by the
            // platform-specific profile policy. iOS's profile-signing CA is
            // not in apple-codesign's embedded set, so its trust comes from
            // the verified path to an exact pinned root plus CA constraints.
            anyhow::ensure!(
                cert.subject_common_name().as_deref() == Some(expected_intermediate),
                "{description} has an unexpected Apple issuing authority"
            );
            if matches!(policy, ProfileSignerPolicy::Macos) {
                anyhow::ensure!(
                    KnownCertificate::all()
                        .iter()
                        .any(|known| known.constructed_data() == cert.constructed_data())
                        && cert.is_apple_intermediate_ca(),
                    "{description} macOS issuer is not an exact pinned Apple intermediate"
                );
            } else if matches!(policy, ProfileSignerPolicy::DerEncoded) {
                anyhow::ensure!(
                    is_pinned_der_profile_intermediate(cert),
                    "{description} DER profile issuer is not the pinned Apple System Integration CA 4"
                );
            }
        } else if index > 1 && !is_root {
            anyhow::ensure!(
                KnownCertificate::all()
                    .iter()
                    .any(|known| known.constructed_data() == cert.constructed_data())
                    && cert.is_apple_intermediate_ca(),
                "{description} chain contains an unpinned Apple intermediate"
            );
        }
        if index > 0 {
            ensure_ca_constraints(cert).with_context(|| {
                format!("{description} Apple chain contains a certificate without CA constraints")
            })?;
        }
        if let Some(issuer) = chain.get(index + 1) {
            verify_certificate_signature(cert, issuer).with_context(|| {
                format!("{description} Apple certificate chain signature is invalid")
            })?;
        }
    }
    Ok(())
}

fn resolve_chain_to_pinned_apple_root(
    leaf: &CapturedX509Certificate,
    candidates: &[CapturedX509Certificate],
) -> anyhow::Result<Vec<CapturedX509Certificate>> {
    const MAX_CANDIDATES: usize = 128;
    const MAX_SEARCH_STEPS: usize = 256;

    fn find_path(
        chain: &mut Vec<CapturedX509Certificate>,
        candidates: &[CapturedX509Certificate],
        remaining_steps: &mut usize,
    ) -> bool {
        let current = chain.last().expect("chain begins with a leaf").clone();
        if KnownCertificate::all_roots()
            .iter()
            .any(|root| root.constructed_data() == current.constructed_data())
        {
            return true;
        }
        if chain.len() >= 8 {
            return false;
        }

        for candidate in candidates {
            if candidate == &current
                || candidate.subject_name() != current.issuer_name()
                || chain.contains(candidate)
            {
                continue;
            }
            if *remaining_steps == 0 {
                return false;
            }
            *remaining_steps -= 1;
            let verified = verify_certificate_signature(&current, candidate);
            if verified.is_err() {
                continue;
            }

            chain.push(candidate.clone());
            if find_path(chain, candidates, remaining_steps) {
                return true;
            }
            chain.pop();
        }
        false
    }

    anyhow::ensure!(
        candidates.len() <= MAX_CANDIDATES,
        "Apple certificate chain has too many candidate certificates"
    );
    let mut chain = vec![leaf.clone()];
    let mut remaining_steps = MAX_SEARCH_STEPS;
    anyhow::ensure!(
        find_path(&mut chain, candidates, &mut remaining_steps),
        "Apple certificate chain does not verify to a pinned Apple root"
    );
    Ok(chain)
}

fn verify_certificate_signature(
    certificate: &CapturedX509Certificate,
    issuer: &CapturedX509Certificate,
) -> anyhow::Result<()> {
    let parsed: &x509_certificate::rfc5280::Certificate = certificate.as_ref();
    anyhow::ensure!(
        &parsed.tbs_certificate.signature.algorithm
            == certificate.signature_signature_algorithm_oid(),
        "Apple certificate TBS and outer signature algorithms disagree"
    );
    let signed_data = parsed
        .tbs_certificate
        .raw_data
        .as_deref()
        .context("Apple certificate has no original TBS bytes")?;
    let signature_algorithm = certificate
        .signature_signature_algorithm()
        .context("Apple certificate has an unsupported signature algorithm")?;
    let issuer_key_algorithm = issuer
        .key_algorithm()
        .context("Apple certificate issuer has an unsupported public key algorithm")?;
    let algorithm = signature_algorithm
        .resolve_verification_algorithm(issuer_key_algorithm)
        .context("Apple certificate signature and issuer key algorithms do not match")?;

    issuer
        .verify_signed_data_with_algorithm(signed_data, parsed.signature.octet_bytes(), algorithm)
        .context("Apple certificate signature verification failed")
}

fn is_pinned_der_profile_intermediate(certificate: &CapturedX509Certificate) -> bool {
    certificate.subject_common_name().as_deref() == Some("Apple System Integration CA 4")
        && Sha256::digest(certificate.constructed_data()).as_slice()
            == DER_PROFILE_INTERMEDIATE_SHA256
}

fn has_extension(certificate: &CapturedX509Certificate, oid: &[u8]) -> bool {
    certificate
        .iter_extensions()
        .any(|extension| extension.id.as_ref() == oid)
}

fn ensure_ca_constraints(certificate: &CapturedX509Certificate) -> anyhow::Result<()> {
    let basic = certificate
        .iter_extensions()
        .find(|extension| extension.id.as_ref() == [85, 29, 19])
        .map(|extension| extension.value.clone().into_bytes());
    let usage = certificate
        .iter_extensions()
        .find(|extension| extension.id.as_ref() == [85, 29, 15])
        .map(|extension| extension.value.clone().into_bytes());
    ensure_ca_constraint_data(basic.as_deref(), usage.as_deref())
}

fn ensure_ca_constraint_data(
    basic_constraints: Option<&[u8]>,
    key_usage: Option<&[u8]>,
) -> anyhow::Result<()> {
    // BasicConstraints (2.5.29.19) must explicitly assert cA=TRUE.
    let basic = basic_constraints.ok_or_else(|| anyhow::anyhow!("missing BasicConstraints"))?;
    let (tag, sequence, consumed) = der_item(basic)?;
    anyhow::ensure!(
        tag == 0x30 && consumed == basic.len(),
        "invalid BasicConstraints DER"
    );
    anyhow::ensure!(
        sequence.first() == Some(&0x01),
        "invalid BasicConstraints sequence"
    );
    let (tag, ca, _) = der_item(sequence)?;
    anyhow::ensure!(
        tag == 0x01 && ca == [0xff],
        "BasicConstraints does not assert cA=TRUE"
    );

    // When KeyUsage is present, RFC 5280 requires keyCertSign for a CA.
    if let Some(usage) = key_usage {
        let (tag, bits, consumed) = der_item(usage)?;
        anyhow::ensure!(
            tag == 0x03 && bits.len() >= 2 && consumed == usage.len(),
            "invalid KeyUsage bit string"
        );
        anyhow::ensure!(bits[1] & 0x04 != 0, "KeyUsage omits keyCertSign");
    }
    Ok(())
}

fn validate_signing_leaf_constraints(certificate: &CapturedX509Certificate) -> anyhow::Result<()> {
    let basic = certificate
        .iter_extensions()
        .find(|extension| extension.id.as_ref() == [85, 29, 19])
        .map(|extension| extension.value.clone().into_bytes());
    let usage = certificate
        .iter_extensions()
        .find(|extension| extension.id.as_ref() == [85, 29, 15])
        .map(|extension| extension.value.clone().into_bytes());
    validate_signing_leaf_constraint_data(basic.as_deref(), usage.as_deref())
}

fn validate_signing_leaf_constraint_data(
    basic_constraints: Option<&[u8]>,
    key_usage: Option<&[u8]>,
) -> anyhow::Result<()> {
    if let Some(basic) = basic_constraints {
        let (tag, sequence, consumed) = der_item(basic)?;
        anyhow::ensure!(
            tag == 0x30 && consumed == basic.len(),
            "invalid BasicConstraints DER"
        );
        if sequence.first() == Some(&0x01) {
            let (tag, ca, _) = der_item(sequence)?;
            anyhow::ensure!(
                tag == 0x01 && ca == [0x00],
                "signing certificate must not be a CA"
            );
        }
    }
    let usage =
        key_usage.ok_or_else(|| anyhow::anyhow!("signing certificate is missing KeyUsage"))?;
    let (tag, bits, consumed) = der_item(usage)?;
    anyhow::ensure!(
        tag == 0x03 && bits.len() >= 2 && consumed == usage.len(),
        "invalid KeyUsage bit string"
    );
    anyhow::ensure!(bits[1] & 0x80 != 0, "KeyUsage omits digitalSignature");
    Ok(())
}

fn der_item(input: &[u8]) -> anyhow::Result<(u8, &[u8], usize)> {
    anyhow::ensure!(input.len() >= 2, "truncated DER value");
    let tag = input[0];
    let first_length = input[1];
    let (length, header) = if first_length & 0x80 == 0 {
        (first_length as usize, 2)
    } else {
        let count = (first_length & 0x7f) as usize;
        anyhow::ensure!(
            count > 0 && count <= std::mem::size_of::<usize>() && input.len() >= 2 + count,
            "invalid DER length"
        );
        let mut length = 0usize;
        for byte in &input[2..2 + count] {
            length = length
                .checked_mul(256)
                .and_then(|n| n.checked_add(*byte as usize))
                .ok_or_else(|| anyhow::anyhow!("DER length overflow"))?;
        }
        (length, 2 + count)
    };
    let end = header
        .checked_add(length)
        .ok_or_else(|| anyhow::anyhow!("DER length overflow"))?;
    anyhow::ensure!(end <= input.len(), "truncated DER contents");
    Ok((tag, &input[header..end], end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use apple_codesign::CodeSigningCertificateExtension as Ext;
    use cryptographic_message_syntax::{Bytes, Oid, SignedData, SignedDataBuilder, SignerBuilder};
    use x509_certificate::certificate::KeyUsage;
    use x509_certificate::{EcdsaCurve, KeyAlgorithm, X509CertificateBuilder};

    const APPLE_DEVELOPMENT: &[u8] =
        include_bytes!("../../tests/fixtures/apple/apple-signed-apple-development.cer");
    const DEVELOPER_ID: &[u8] =
        include_bytes!("../../tests/fixtures/apple/apple-signed-developer-id-application.cer");
    const DER_PROFILE_FIXTURES: &[&[u8]] = &[
        include_bytes!("../../tests/fixtures/apple/der-ios.mobileprovision"),
        include_bytes!("../../tests/fixtures/apple/der-ios-signin.mobileprovision"),
        include_bytes!("../../tests/fixtures/apple/der-macos.provisionprofile"),
    ];

    fn cert_at(bytes: &[u8]) -> CapturedX509Certificate {
        CapturedX509Certificate::from_der(bytes.to_vec()).unwrap()
    }

    fn at(year: i32, month: u32, day: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
    }

    #[test]
    fn accepts_real_apple_signing_chains_at_a_fixed_valid_time() {
        let date = at(2021, 6, 1);
        validate_provisioning_signing_certificate_at(&cert_at(APPLE_DEVELOPMENT), date).unwrap();
        validate_provisioning_signing_certificate_at(&cert_at(DEVELOPER_ID), date).unwrap();

        assert!(
            validate_provisioning_signing_certificate_at(
                &cert_at(APPLE_DEVELOPMENT),
                at(2022, 4, 27),
            )
            .is_err()
        );
    }

    #[test]
    fn accepts_public_apple_signed_ios_provisioning_cms_fixture() {
        parse_provisioning_cms(include_bytes!(
            "../../tests/fixtures/apple/fastlane-test.mobileprovision"
        ))
        .unwrap();
    }

    #[test]
    fn accepts_real_apple_signed_der_profile_cms_fixtures() {
        for bytes in DER_PROFILE_FIXTURES {
            let outer = parse_provisioning_cms(bytes).unwrap();

            let content = outer.signed_content().unwrap();
            let profile = plist::Value::from_reader(std::io::Cursor::new(content)).unwrap();
            let der_profile = profile
                .as_dictionary()
                .unwrap()
                .get("DER-Encoded-Profile")
                .unwrap()
                .as_data()
                .unwrap();
            parse_provisioning_cms(der_profile).unwrap();
        }
    }

    #[test]
    fn rejects_der_profile_with_tampered_embedded_certificates() {
        let outer = SignedData::parse_ber(DER_PROFILE_FIXTURES[0]).unwrap();
        let profile =
            plist::Value::from_reader(std::io::Cursor::new(outer.signed_content().unwrap()))
                .unwrap();
        let inner_bytes = profile
            .as_dictionary()
            .unwrap()
            .get("DER-Encoded-Profile")
            .unwrap()
            .as_data()
            .unwrap();
        let inner_certificates = extract_original_cms_certificates(inner_bytes).unwrap();
        for subject in [
            "WWDR Provisioning Profile Signing",
            "Apple System Integration CA 4",
        ] {
            let certificate = inner_certificates
                .iter()
                .find(|certificate| certificate.subject_common_name().as_deref() == Some(subject))
                .unwrap();
            let parsed: &x509_certificate::rfc5280::Certificate = certificate.as_ref();
            let signature = parsed.signature.octet_bytes();
            let mut tampered = inner_bytes.to_vec();
            mutate_first_occurrence(&mut tampered, signature.as_ref());
            let error = parse_provisioning_cms(&tampered).unwrap_err();
            assert!(
                error.to_string().contains("pinned Apple root"),
                "{subject}: {error:#}"
            );
        }
    }

    #[test]
    fn requires_the_specific_der_profile_signer_marker_and_intermediate_pin() {
        let mut builder = X509CertificateBuilder::default();
        builder
            .subject()
            .append_common_name_utf8_string("WWDR Provisioning Profile Signing")
            .unwrap();
        builder.constraint_not_ca();
        builder.key_usage(KeyUsage::DigitalSignature);
        let (certificate, _) = builder
            .create_with_random_keypair(KeyAlgorithm::Ecdsa(EcdsaCurve::Secp256r1))
            .unwrap();
        let error = provisioning_profile_signer_purpose(&certificate)
            .err()
            .unwrap();
        assert!(error.to_string().contains("signing marker"));

        let outer = SignedData::parse_ber(DER_PROFILE_FIXTURES[0]).unwrap();
        let content = outer.signed_content().unwrap();
        let profile = plist::Value::from_reader(std::io::Cursor::new(content)).unwrap();
        let der_profile = profile
            .as_dictionary()
            .unwrap()
            .get("DER-Encoded-Profile")
            .unwrap()
            .as_data()
            .unwrap();
        let inner = extract_original_cms_certificates(der_profile).unwrap();
        let intermediate = inner
            .iter()
            .find(|candidate| {
                candidate.subject_common_name().as_deref() == Some("Apple System Integration CA 4")
            })
            .unwrap();
        assert!(is_pinned_der_profile_intermediate(intermediate));

        let mut changed = intermediate.constructed_data().to_vec();
        *changed.last_mut().unwrap() ^= 1;
        let changed = CapturedX509Certificate::from_der(changed).unwrap();
        assert!(!is_pinned_der_profile_intermediate(&changed));
    }

    fn mutate_first_occurrence(bytes: &mut [u8], needle: &[u8]) {
        let matches = bytes
            .windows(needle.len())
            .enumerate()
            .filter_map(|(index, window)| (window == needle).then_some(index))
            .collect::<Vec<_>>();
        assert_eq!(matches.len(), 1, "expected one occurrence in CMS fixture");
        bytes[matches[0]] ^= 1;
    }

    #[test]
    fn rejects_public_fixture_with_modified_signed_content() {
        let original = include_bytes!("../../tests/fixtures/apple/fastlane-test.mobileprovision");
        let cms = SignedData::parse_ber(original).unwrap();
        let content = cms.signed_content().unwrap();
        let mut tampered = original.to_vec();
        mutate_first_occurrence(&mut tampered, content);

        let error = parse_provisioning_cms(&tampered).unwrap_err();
        assert!(error.to_string().contains("message digest"));
    }

    #[test]
    fn rejects_public_fixture_with_modified_signer_signature() {
        let original = include_bytes!("../../tests/fixtures/apple/fastlane-test.mobileprovision");
        let cms = SignedData::parse_ber(original).unwrap();
        let signature = cms.signers().next().unwrap().signature();
        let mut tampered = original.to_vec();
        mutate_first_occurrence(&mut tampered, signature);

        let error = parse_provisioning_cms(&tampered).unwrap_err();
        assert!(error.to_string().contains("signature verification failed"));
    }

    #[test]
    fn rejects_a_self_signed_certificate_named_like_apple_profile_signer() {
        let mut builder = X509CertificateBuilder::default();
        builder
            .subject()
            .append_common_name_utf8_string("Apple iPhone OS Provisioning Profile Signing")
            .unwrap();
        builder.constraint_not_ca();
        builder.key_usage(KeyUsage::DigitalSignature);
        let (certificate, key) = builder
            .create_with_random_keypair(KeyAlgorithm::Ecdsa(EcdsaCurve::Secp256r1))
            .unwrap();
        let cms_bytes = SignedDataBuilder::default()
            .certificate(certificate.clone())
            .content_inline(b"synthetic profile".to_vec())
            .signer(SignerBuilder::new(&key, certificate))
            .build_der()
            .unwrap();
        let error = parse_provisioning_cms(&cms_bytes).unwrap_err();
        assert!(error.to_string().contains("pinned Apple root"));
    }

    #[test]
    fn rejects_self_signed_der_profile_signer_even_with_apple_marker() {
        let mut builder = X509CertificateBuilder::default();
        builder
            .subject()
            .append_common_name_utf8_string("WWDR Provisioning Profile Signing")
            .unwrap();
        builder.constraint_not_ca();
        builder.key_usage(KeyUsage::DigitalSignature);
        builder.add_extension_der_data(
            Oid(Bytes::copy_from_slice(DER_PROFILE_SIGNER_MARKER_OID)),
            false,
            [0x05, 0x00],
        );
        let (certificate, key) = builder
            .create_with_random_keypair(KeyAlgorithm::Ecdsa(EcdsaCurve::Secp256r1))
            .unwrap();
        assert!(provisioning_profile_signer_purpose(&certificate).is_ok());

        let cms_bytes = SignedDataBuilder::default()
            .certificate(certificate.clone())
            .content_inline(b"synthetic DER profile".to_vec())
            .signer(SignerBuilder::new(&key, certificate))
            .build_der()
            .unwrap();
        let error = parse_provisioning_cms(&cms_bytes).unwrap_err();
        assert!(error.to_string().contains("pinned Apple root"));
    }

    #[test]
    fn rejects_application_signing_certificates_as_profile_signers() {
        let developer_id = cert_at(DEVELOPER_ID);
        assert!(provisioning_profile_signer_purpose(&developer_id).is_err());

        let application_signer = cert_at(APPLE_DEVELOPMENT);
        assert!(provisioning_profile_signer_purpose(&application_signer).is_err());
    }

    #[test]
    fn accepts_legacy_application_signing_markers() {
        assert!(has_supported_application_signing_purpose(&[
            Ext::IPhoneDeveloper
        ]));
        assert!(has_supported_application_signing_purpose(&[
            Ext::MacDeveloper
        ]));
        assert!(has_supported_application_signing_purpose(&[
            Ext::AppleDeveloperCertificateSubmission
        ]));
        assert!(!has_supported_application_signing_purpose(&[
            Ext::SafariDeveloper
        ]));
    }

    #[test]
    fn rejects_missing_or_invalid_ca_and_leaf_usage_constraints() {
        const CA_TRUE: &[u8] = &[0x30, 0x03, 0x01, 0x01, 0xff];
        const CA_FALSE: &[u8] = &[0x30, 0x03, 0x01, 0x01, 0x00];
        const KEY_CERT_SIGN: &[u8] = &[0x03, 0x02, 0x07, 0x04];
        const DIGITAL_SIGNATURE: &[u8] = &[0x03, 0x02, 0x07, 0x80];

        assert!(ensure_ca_constraint_data(None, Some(KEY_CERT_SIGN)).is_err());
        assert!(ensure_ca_constraint_data(Some(CA_FALSE), Some(KEY_CERT_SIGN)).is_err());
        assert!(ensure_ca_constraint_data(Some(CA_TRUE), None).is_ok());
        assert!(ensure_ca_constraint_data(Some(CA_TRUE), Some(KEY_CERT_SIGN)).is_ok());
        assert!(ensure_ca_constraint_data(Some(CA_TRUE), Some(DIGITAL_SIGNATURE)).is_err());

        assert!(validate_signing_leaf_constraint_data(None, None).is_err());
        assert!(
            validate_signing_leaf_constraint_data(Some(CA_TRUE), Some(DIGITAL_SIGNATURE)).is_err()
        );
        assert!(
            validate_signing_leaf_constraint_data(Some(CA_FALSE), Some(DIGITAL_SIGNATURE)).is_ok()
        );
        assert!(validate_signing_leaf_constraint_data(None, Some(DIGITAL_SIGNATURE)).is_ok());
    }

    #[test]
    fn der_decoder_rejects_truncation_and_length_overflow_without_panicking() {
        assert!(der_item(&[0x30, 0x03, 0x01]).is_err());
        assert!(der_item(&[0x30, 0x88, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]).is_err());
    }
}
