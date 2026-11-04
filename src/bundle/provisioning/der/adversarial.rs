//! Adversarial inputs for Apple's DER provisioning profiles.
//!
//! These tests intentionally work below CMS so malformed signed-content bytes
//! can be exercised without manufacturing signatures.

use super::*;
use plist::{Dictionary, Value};
use sha2::Digest;

const DICTIONARY_TAG: &[u8] = &[0xb0]; // [CONTEXT 16] IMPLICIT
const WRAPPER_TAG: &[u8] = &[0x70]; // [APPLICATION 16] IMPLICIT

fn tlv(tag: &[u8], content: &[u8]) -> Vec<u8> {
    let mut encoded = tag.to_vec();
    encode_length(content.len(), &mut encoded);
    encoded.extend_from_slice(content);
    encoded
}

fn encode_length(length: usize, output: &mut Vec<u8>) {
    if length < 128 {
        output.push(length as u8);
        return;
    }
    let bytes = length.to_be_bytes();
    let first = bytes.iter().position(|byte| *byte != 0).unwrap();
    output.push(0x80 | (bytes.len() - first) as u8);
    output.extend_from_slice(&bytes[first..]);
}

fn integer(value: u8) -> Vec<u8> {
    tlv(&[0x02], &[value])
}

fn string(value: &str) -> Vec<u8> {
    tlv(&[0x0c], value.as_bytes())
}

fn pair(key: &str, value: &[u8]) -> Vec<u8> {
    let mut content = string(key);
    content.extend_from_slice(value);
    tlv(&[0x30], &content)
}

fn dictionary(pairs: &[Vec<u8>]) -> Vec<u8> {
    tlv(DICTIONARY_TAG, &sorted(pairs))
}

fn root(pairs: &[Vec<u8>]) -> Vec<u8> {
    tlv(&[0x31], &sorted(pairs))
}

fn unsorted_set(pairs: &[Vec<u8>]) -> Vec<u8> {
    tlv(&[0x31], &pairs.concat())
}

fn unsorted_context_dictionary(pairs: &[Vec<u8>]) -> Vec<u8> {
    tlv(DICTIONARY_TAG, &pairs.concat())
}

fn sorted(pairs: &[Vec<u8>]) -> Vec<u8> {
    let mut values = pairs.to_vec();
    values.sort();
    values.concat()
}

fn validate_payload(payload: &[u8]) -> anyhow::Result<()> {
    validate(&Dictionary::new(), payload)
}

fn decode_tlv(encoded: &[u8]) -> anyhow::Result<Value> {
    super::decode(AnyRef::from_der(encoded)?, 0)
}

#[test]
fn rejects_duplicate_root_dictionary_keys() {
    let payload = root(&[pair("UUID", &string("one")), pair("UUID", &string("two"))]);
    let error = validate_payload(&payload).unwrap_err().to_string();
    assert!(error.contains("Duplicate DER provisioning profile key `UUID`"));
}

#[test]
fn rejects_duplicate_keys_in_nested_and_wrapped_entitlement_dictionaries() {
    let nested = dictionary(&[
        pair("enabled", &tlv(&[0x01], &[0xff])),
        pair("enabled", &tlv(&[0x01], &[0x00])),
    ]);
    let payload = root(&[pair("Nested", &nested)]);
    let error = validate_payload(&payload).unwrap_err().to_string();
    assert!(error.contains("Duplicate DER provisioning profile key `enabled`"));

    let mut wrapper_content = integer(1);
    wrapper_content.extend_from_slice(&dictionary(&[
        pair("application-identifier", &string("TEAM.com.example")),
        pair("application-identifier", &string("TEAM.com.other")),
    ]));
    let wrapper = tlv(WRAPPER_TAG, &wrapper_content);
    let payload = root(&[pair("Entitlements", &wrapper)]);
    let error = validate_payload(&payload).unwrap_err().to_string();
    assert!(error.contains("Duplicate DER provisioning profile key `application-identifier`"));
}

#[test]
fn rejects_unsupported_entitlement_wrapper_version_and_trailing_values() {
    for version in [0, 2] {
        let mut contents = integer(version);
        contents.extend_from_slice(&dictionary(&[]));
        let wrapper = tlv(WRAPPER_TAG, &contents);
        let error = decode_tlv(&wrapper).unwrap_err().to_string();
        assert!(error.contains("Unsupported DER entitlement version"));
    }

    let mut contents = integer(1);
    contents.extend_from_slice(&dictionary(&[]));
    contents.extend_from_slice(&integer(1));
    let wrapper = tlv(WRAPPER_TAG, &contents);
    let error = decode_tlv(&wrapper).unwrap_err().to_string();
    assert!(error.contains("Trailing DER entitlement wrapper data"));
}

#[test]
fn rejects_noncanonical_booleans_and_integers() {
    for boolean in [&[0x01][..], &[0x7f][..], &[0x01, 0xff][..]] {
        let encoded = tlv(&[0x01], boolean);
        assert!(
            decode_tlv(&encoded).is_err(),
            "accepted BOOLEAN {boolean:x?}"
        );
    }

    // DER INTEGER must be minimally encoded: neither redundant positive nor
    // redundant negative sign-extension bytes are permitted.
    for integer in [&[0x00, 0x01][..], &[0xff, 0xff][..], &[][..]] {
        let encoded = tlv(&[0x02], integer);
        assert!(
            decode_tlv(&encoded).is_err(),
            "accepted INTEGER {integer:x?}"
        );
    }
}

#[test]
fn accepts_canonical_boolean_integer_and_nested_entitlement_values() {
    assert_eq!(
        decode_tlv(&tlv(&[0x01], &[0xff])).unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        decode_tlv(&tlv(&[0x01], &[0x00])).unwrap(),
        Value::Boolean(false)
    );
    assert_eq!(
        decode_tlv(&tlv(&[0x02], &[0x2a])).unwrap(),
        Value::Integer(42.into())
    );

    let mut expected_nested = Dictionary::new();
    expected_nested.insert("enabled".into(), Value::Boolean(true));
    let mut expected_entitlements = Dictionary::new();
    expected_entitlements.insert(
        "names".into(),
        Value::Array(vec![
            Value::String("alpha".into()),
            Value::String("beta".into()),
        ]),
    );
    expected_entitlements.insert("nested".into(), Value::Dictionary(expected_nested));

    let nested = dictionary(&[pair("enabled", &tlv(&[0x01], &[0xff]))]);
    let names = tlv(&[0x30], &[string("alpha"), string("beta")].concat());
    let entitlements = dictionary(&[pair("names", &names), pair("nested", &nested)]);
    let mut wrapper = integer(1);
    wrapper.extend_from_slice(&entitlements);
    let payload = root(&[pair("Entitlements", &tlv(WRAPPER_TAG, &wrapper))]);

    let mut outer = Dictionary::new();
    outer.insert(
        "Entitlements".into(),
        Value::Dictionary(expected_entitlements.clone()),
    );
    validate(&outer, &payload).unwrap();

    let mut mismatched = expected_entitlements;
    mismatched.insert(
        "names".into(),
        Value::Array(vec![Value::String("changed".into())]),
    );
    outer.insert("Entitlements".into(), Value::Dictionary(mismatched));
    assert!(validate(&outer, &payload).is_err());
}

#[test]
fn rejects_malformed_and_noncanonical_lengths() {
    let valid_pair = pair("Value", &string("ok"));
    let mut short_length = vec![0x31, (valid_pair.len() - 1) as u8];
    short_length.extend_from_slice(&valid_pair);

    let mut indefinite_length = vec![0x31, 0x80];
    indefinite_length.extend_from_slice(&valid_pair);
    indefinite_length.extend_from_slice(&[0x00, 0x00]);

    let mut long_form_for_short = vec![0x31, 0x81, valid_pair.len() as u8];
    long_form_for_short.extend_from_slice(&valid_pair);

    for payload in [short_length, indefinite_length, long_form_for_short] {
        assert!(
            AnyRef::from_der(&payload).is_err(),
            "accepted malformed length {payload:x?}"
        );
    }
}

#[test]
fn accepts_canonical_outer_set_and_rejects_reversed_order_even_when_fields_match() {
    let entries = [pair("A", &string("x")), pair("B", &string("x"))];
    let mut outer = Dictionary::new();
    outer.insert("A".into(), Value::String("x".into()));
    outer.insert("B".into(), Value::String("x".into()));

    validate(&outer, &root(&entries)).unwrap();

    let reversed = [entries[1].clone(), entries[0].clone()];
    let error = validate(&outer, &unsorted_set(&reversed))
        .unwrap_err()
        .to_string();
    assert!(error.contains("Noncanonical DER provisioning profile SET ordering"));
}

#[test]
fn accepts_lexical_context_sixteen_entitlement_order_even_when_pair_lengths_descend() {
    let app_identifier = pair("application-identifier", &string("TEAM.com.example"));
    let aps_environment = pair("aps-environment", &string("production"));
    assert!(app_identifier.len() > aps_environment.len());

    let mut wrapped = integer(1);
    // Apple's context-16 dictionary is sequence-like and fixture pairs use
    // lexical key order, so this deliberately has descending pair lengths.
    wrapped.extend_from_slice(&unsorted_context_dictionary(&[
        app_identifier,
        aps_environment,
    ]));
    let payload = root(&[pair("Entitlements", &tlv(WRAPPER_TAG, &wrapped))]);

    let mut entitlements = Dictionary::new();
    entitlements.insert(
        "application-identifier".into(),
        Value::String("TEAM.com.example".into()),
    );
    entitlements.insert("aps-environment".into(), Value::String("production".into()));
    let mut outer = Dictionary::new();
    outer.insert("Entitlements".into(), Value::Dictionary(entitlements));
    validate(&outer, &payload).unwrap();
}

#[test]
fn rejects_reversed_order_in_nested_universal_set() {
    let entries = [pair("A", &string("x")), pair("B", &string("x"))];
    let reversed = [entries[1].clone(), entries[0].clone()];
    let nested_set = tlv(&[0x31], &reversed.concat());
    let payload = root(&[pair("Nested", &nested_set)]);

    let mut nested = Dictionary::new();
    nested.insert("A".into(), Value::String("x".into()));
    nested.insert("B".into(), Value::String("x".into()));
    let mut outer = Dictionary::new();
    outer.insert("Nested".into(), Value::Dictionary(nested));
    let error = validate(&outer, &payload).unwrap_err().to_string();
    assert!(error.contains("Noncanonical DER provisioning profile SET ordering"));
}

fn nested_dictionary(levels: usize) -> Vec<u8> {
    let mut value = string("leaf");
    for _ in 0..levels {
        value = dictionary(&[pair("next", &value)]);
    }
    root(&[pair("Root", &value)])
}

#[test]
fn enforces_nesting_limit_without_panicking() {
    // Exercise the decoder directly so the expected outer-plist mismatch does
    // not obscure whether a particular nesting depth was accepted.
    let accepted_depth = nested_dictionary(30);
    let root = AnyRef::from_der(&accepted_depth).unwrap();
    super::dictionary(root.value(), 0).unwrap();

    let too_deep = nested_dictionary(31);
    let root = AnyRef::from_der(&too_deep).unwrap();
    let error = super::dictionary(root.value(), 0).unwrap_err().to_string();
    assert!(error.contains("nesting exceeds 32 levels"), "{error}");
}

#[test]
fn rejects_entitlement_dictionary_that_is_not_context_sixteen() {
    let invalid_wrapper = tlv(WRAPPER_TAG, &[integer(1), tlv(&[0x30], &[])].concat());
    let error = decode_tlv(&invalid_wrapper).unwrap_err().to_string();
    assert!(error.contains("Invalid DER entitlement dictionary"));
}

#[test]
fn rejects_noncanonical_high_tag_aliases_for_low_tag_numbers() {
    // Tag numbers below 31 must use the one-octet form. These encode the same
    // nominal tags as the valid one-octet application/context-16 tags.
    let high_tag_wrapper_alias = [0x7f, 0x10];
    let mut wrapper_contents = integer(1);
    wrapper_contents.extend_from_slice(&dictionary(&[]));
    let malformed_wrapper = tlv(&high_tag_wrapper_alias, &wrapper_contents);
    assert!(AnyRef::from_der(&malformed_wrapper).is_err());

    let high_tag_dictionary_alias = [0xbf, 0x10];
    let malformed_dictionary = tlv(&high_tag_dictionary_alias, &[]);
    assert!(AnyRef::from_der(&malformed_dictionary).is_err());
}

#[test]
fn rejects_mismatched_certificate_fingerprints_and_array_order() {
    let certificates = Value::Array(vec![
        Value::Data(b"certificate one".to_vec()),
        Value::Data(b"certificate two".to_vec()),
    ]);
    let mut wrong = Dictionary::new();
    wrong.insert("DeveloperCertificates".into(), certificates.clone());

    let fingerprints = certificates
        .as_array()
        .unwrap()
        .iter()
        .map(|certificate| {
            let Value::Data(bytes) = certificate else {
                unreachable!()
            };
            let fingerprint = sha2::Sha256::digest(bytes);
            tlv(&[0x04], fingerprint.as_slice())
        })
        .collect::<Vec<_>>();
    let der = root(&[pair(
        "DeveloperCertificates",
        &tlv(&[0x30], &fingerprints.concat()),
    )]);
    validate(&wrong, &der).unwrap();

    let reversed = fingerprints.iter().rev().cloned().collect::<Vec<_>>();
    let der = root(&[pair(
        "DeveloperCertificates",
        &tlv(&[0x30], &reversed.concat()),
    )]);
    assert!(validate(&wrong, &der).is_err());

    wrong.insert(
        "DeveloperCertificates".into(),
        Value::Array(vec![Value::Data(b"certificate one".to_vec())]),
    );
    let der = root(&[pair(
        "DeveloperCertificates",
        &tlv(&[0x30], &fingerprints.concat()),
    )]);
    assert!(validate(&wrong, &der).is_err());
}

#[test]
fn rejects_semantically_mismatched_dates_and_platforms() {
    let mut outer = Dictionary::new();
    outer.insert(
        "Platform".into(),
        Value::Array(vec![Value::String("iOS".into())]),
    );
    let platform = tlv(&[0x30], &string("OSX"));
    let payload = root(&[pair("Platform", &platform)]);
    assert!(validate(&outer, &payload).is_err());

    let mut outer = Dictionary::new();
    outer.insert(
        "CreationDate".into(),
        Value::Date(std::time::UNIX_EPOCH.into()),
    );
    let time = tlv(&[0x18], b"20240101000000Z");
    let payload = root(&[pair("CreationDate", &time)]);
    assert!(validate(&outer, &payload).is_err());

    let date = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_704_067_200);
    let mut outer = Dictionary::new();
    outer.insert(
        "Platform".into(),
        Value::Array(vec![Value::String("OSX".into())]),
    );
    outer.insert("CreationDate".into(), Value::Date(date.into()));
    outer.insert("ExpirationDate".into(), Value::Date(date.into()));
    let platform = tlv(&[0x30], &string("OSX"));
    let generalized = tlv(&[0x18], b"20240101000000Z");
    let utc = tlv(&[0x17], b"240101000000Z");
    let payload = root(&[
        pair("Platform", &platform),
        pair("CreationDate", &generalized),
        pair("ExpirationDate", &utc),
    ]);
    validate(&outer, &payload).unwrap();
}
