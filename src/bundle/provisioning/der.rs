//! Compare Apple's signed DER profile with its XML/binary plist view.
//!
//! Profile dictionaries are SETs of key/value pairs. Entitlements instead use
//! Apple's versioned application-16 wrapper and context-16 dictionaries.

use anyhow::{Context, ensure};
use der::{Decode, DerOrd, Reader, SliceReader, Tag, TagNumber, Tagged, asn1::AnyRef};
use plist::{Dictionary, Value};
use sha2::{Digest, Sha256};

const DICTIONARY: Tag = Tag::ContextSpecific {
    constructed: true,
    number: TagNumber::N16,
};
const WRAPPER: Tag = Tag::Application {
    constructed: true,
    number: TagNumber::N16,
};

pub(super) fn validate(outer: &Dictionary, payload: &[u8]) -> anyhow::Result<()> {
    ensure!(
        payload.len() <= 1024 * 1024,
        "DER provisioning profile exceeds 1 MiB"
    );
    let root = AnyRef::from_der(payload).context("Invalid DER provisioning profile")?;
    ensure!(
        root.tag() == Tag::Set,
        "DER provisioning profile must be a SET"
    );
    // Context-specific entitlement dictionaries are sequences, but a universal
    // SET must be sorted. Check the original order; SetOfVec sorts on decoding.
    validate_set(root.value())?;
    let decoded = dictionary(root.value(), 0)?;
    for (key, value) in outer {
        if key == "DER-Encoded-Profile" {
            continue;
        }
        let expected = if key == "DeveloperCertificates" {
            certificate_fingerprints(value)?
        } else {
            value.clone()
        };
        ensure!(
            decoded.get(key) == Some(&expected),
            "DER provisioning profile disagrees with plist field `{key}`"
        );
    }
    for (key, value) in &decoded {
        // Apple includes this classification only in the DER representation.
        // It must remain a string; it does not replace any plist authorization.
        if key == "ProfileDistributionType" && !outer.contains_key(key) {
            ensure!(
                value.as_string().is_some(),
                "Invalid DER ProfileDistributionType"
            );
        } else {
            ensure!(
                outer.contains_key(key),
                "DER provisioning profile has unmatched field `{key}`"
            );
        }
    }
    Ok(())
}

fn certificate_fingerprints(value: &Value) -> anyhow::Result<Value> {
    let certificates = value
        .as_array()
        .context("DeveloperCertificates must be an array")?;
    Ok(Value::Array(
        certificates
            .iter()
            .map(|certificate| {
                let bytes = certificate
                    .as_data()
                    .context("DeveloperCertificates must contain data")?;
                Ok(Value::Data(Sha256::digest(bytes).to_vec()))
            })
            .collect::<anyhow::Result<_>>()?,
    ))
}

fn validate_set(bytes: &[u8]) -> anyhow::Result<()> {
    let mut reader = SliceReader::new(bytes)?;
    let mut previous: Option<AnyRef<'_>> = None;
    while !reader.is_finished() {
        let value = reader.decode::<AnyRef>()?;
        if let Some(previous) = previous {
            ensure!(
                previous.der_cmp(&value)? != std::cmp::Ordering::Greater,
                "Noncanonical DER provisioning profile SET ordering"
            );
        }
        previous = Some(value);
    }
    Ok(())
}

fn dictionary(bytes: &[u8], depth: usize) -> anyhow::Result<Dictionary> {
    ensure!(
        depth < 32,
        "DER provisioning profile nesting exceeds 32 levels"
    );
    let mut reader = SliceReader::new(bytes)?;
    let mut values = Dictionary::new();
    while !reader.is_finished() {
        let pair = reader.decode::<AnyRef>()?;
        let (key, value) = pair.sequence(|reader| {
            let key = reader.decode::<der::asn1::Utf8StringRef<'_>>()?;
            let value = reader.decode::<AnyRef>()?;
            Ok((key.as_str(), value))
        })?;
        ensure!(
            !values.contains_key(key),
            "Duplicate DER provisioning profile key `{key}`"
        );
        values.insert(key.to_owned(), decode(value, depth + 1)?);
    }
    Ok(values)
}

fn decode(value: AnyRef<'_>, depth: usize) -> anyhow::Result<Value> {
    ensure!(
        depth < 32,
        "DER provisioning profile nesting exceeds 32 levels"
    );
    Ok(match value.tag() {
        Tag::Utf8String => Value::String(
            value
                .decode_as::<der::asn1::Utf8StringRef<'_>>()?
                .as_str()
                .to_owned(),
        ),
        Tag::Boolean => Value::Boolean(value.decode_as::<bool>()?),
        Tag::Integer => {
            if let Ok(integer) = value.decode_as::<i64>() {
                Value::Integer(integer.into())
            } else {
                Value::Integer(value.decode_as::<u64>()?.into())
            }
        }
        Tag::OctetString => Value::Data(
            value
                .decode_as::<der::asn1::OctetStringRef<'_>>()?
                .as_bytes()
                .to_vec(),
        ),
        Tag::UtcTime => Value::Date(
            value
                .decode_as::<der::asn1::UtcTime>()?
                .to_system_time()
                .into(),
        ),
        Tag::GeneralizedTime => Value::Date(
            value
                .decode_as::<der::asn1::GeneralizedTime>()?
                .to_system_time()
                .into(),
        ),
        Tag::Sequence => {
            let mut reader = SliceReader::new(value.value())?;
            let mut values = Vec::new();
            while !reader.is_finished() {
                values.push(decode(reader.decode::<AnyRef>()?, depth + 1)?);
            }
            Value::Array(values)
        }
        Tag::Set => {
            validate_set(value.value())?;
            Value::Dictionary(dictionary(value.value(), depth)?)
        }
        DICTIONARY => Value::Dictionary(dictionary(value.value(), depth)?),
        WRAPPER => {
            let mut reader = SliceReader::new(value.value())?;
            ensure!(
                reader.decode::<u64>()? == 1,
                "Unsupported DER entitlement version"
            );
            let dictionary = reader.decode::<AnyRef>()?;
            ensure!(
                dictionary.tag() == DICTIONARY,
                "Invalid DER entitlement dictionary"
            );
            ensure!(
                reader.is_finished(),
                "Trailing DER entitlement wrapper data"
            );
            Value::Dictionary(self::dictionary(dictionary.value(), depth + 1)?)
        }
        tag => anyhow::bail!("Unsupported DER provisioning profile tag {tag:?}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &[&[u8]] = &[
        include_bytes!("../../../tests/fixtures/apple/der-ios.mobileprovision"),
        include_bytes!("../../../tests/fixtures/apple/der-ios-signin.mobileprovision"),
        include_bytes!("../../../tests/fixtures/apple/der-macos.provisionprofile"),
    ];

    fn unpack(bytes: &[u8]) -> (Dictionary, Vec<u8>) {
        let content = super::super::super::apple_trust::verified_profile_content(bytes).unwrap();
        let outer = Value::from_reader(std::io::Cursor::new(content))
            .unwrap()
            .into_dictionary()
            .unwrap();
        let payload = super::super::super::apple_trust::verified_profile_content(
            outer["DER-Encoded-Profile"].as_data().unwrap(),
        )
        .unwrap();
        (outer, payload)
    }

    #[test]
    fn real_apple_signed_ios_and_macos_profiles_match() {
        for bytes in FIXTURES {
            let (outer, payload) = unpack(bytes);
            validate(&outer, &payload).unwrap();
        }
    }

    #[test]
    fn rejects_substitution_of_another_valid_apple_signed_profile() {
        let (outer, _) = unpack(FIXTURES[0]);
        let (_, other_payload) = unpack(FIXTURES[1]);
        assert!(validate(&outer, &other_payload).is_err());
    }

    #[test]
    fn consistency_checks_do_not_bypass_profile_expiration() {
        for bytes in FIXTURES {
            let (outer, _) = unpack(bytes);
            let app_id = outer["Entitlements"]
                .as_dictionary()
                .unwrap()
                .get("application-identifier")
                .or_else(|| {
                    outer["Entitlements"]
                        .as_dictionary()
                        .unwrap()
                        .get("com.apple.application-identifier")
                })
                .unwrap()
                .as_string()
                .unwrap()
                .split_once('.')
                .unwrap()
                .1
                .replace('*', "example");
            let file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(file.path(), bytes).unwrap();
            let error = match super::super::ProvisioningProfile::read(file.path(), &app_id) {
                Ok(_) => panic!("expired fixture was accepted"),
                Err(error) => error,
            };
            assert!(error.to_string().contains("expired"), "{error:#}");
        }
    }

    #[test]
    fn rejects_changed_removed_and_added_authorization_fields() {
        for bytes in FIXTURES {
            let (outer, payload) = unpack(bytes);
            for key in [
                "ExpirationDate",
                "CreationDate",
                "Entitlements",
                "DeveloperCertificates",
                "Platform",
                "TeamIdentifier",
                "ApplicationIdentifierPrefix",
                "ProvisionedDevices",
                "UUID",
            ] {
                let mut changed = outer.clone();
                changed.insert(key.into(), Value::String("changed".into()));
                assert!(
                    validate(&changed, &payload).is_err(),
                    "accepted changed {key}"
                );
                let mut missing = outer.clone();
                missing.remove(key);
                assert!(
                    validate(&missing, &payload).is_err(),
                    "accepted missing {key}"
                );
            }
            let mut extra = outer.clone();
            extra.insert("ProvisionsAllDevices".into(), true.into());
            assert!(validate(&extra, &payload).is_err());
            let mut cert = outer.clone();
            let Value::Data(bytes) = &mut cert
                .get_mut("DeveloperCertificates")
                .unwrap()
                .as_array_mut()
                .unwrap()[0]
            else {
                panic!("fixture certificate is not data");
            };
            bytes[0] ^= 1;
            assert!(validate(&cert, &payload).is_err());
            let mut trailing = payload.clone();
            trailing.push(0);
            assert!(validate(&outer, &trailing).is_err());
            for length in 0..payload.len() {
                assert!(validate(&outer, &payload[..length]).is_err());
            }
        }
    }
}

#[cfg(test)]
mod adversarial;
