//! Read explicit entitlement dictionaries using the plist crate.

use anyhow::{Context, ensure};
use std::path::Path;

pub(super) fn read(path: &Path) -> crate::Result<String> {
    let value = plist::Value::from_file(path)
        .with_context(|| format!("Failed to read Apple signing entitlements {path:?}"))?;
    ensure!(
        value.as_dictionary().is_some(),
        "Apple signing entitlements must be a plist dictionary"
    );
    let mut xml = Vec::new();
    value
        .to_writer_xml(&mut xml)
        .context("Failed to encode Apple signing entitlements")?;
    Ok(String::from_utf8(xml)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use plist::{Dictionary, Value};

    #[test]
    fn preserves_explicit_claims_from_xml_and_binary_plists() {
        let temp = tempfile::tempdir().unwrap();
        let claims = Value::Dictionary(Dictionary::from_iter([
            ("com.apple.security.app-sandbox", Value::Boolean(true)),
            (
                "com.apple.security.application-groups",
                Value::Array(vec![Value::String("group.example.app".into())]),
            ),
            (
                "custom",
                Value::Dictionary(Dictionary::from_iter([
                    ("count", Value::Integer(3.into())),
                    ("data", Value::Data(vec![1, 2, 3])),
                ])),
            ),
        ]));
        let xml = temp.path().join("claims.xml");
        let binary = temp.path().join("claims.binary");
        claims.to_file_xml(&xml).unwrap();
        claims.to_file_binary(&binary).unwrap();
        for path in [&xml, &binary] {
            let normalized = read(path).unwrap();
            assert_eq!(
                plist::Value::from_reader_xml(normalized.as_bytes()).unwrap(),
                claims
            );
        }
    }

    #[test]
    fn rejects_non_dictionary_and_missing_inputs() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("claims.plist");
        assert!(read(&path).is_err());
        Value::Array(Vec::new()).to_file_binary(&path).unwrap();
        assert!(read(&path).unwrap_err().to_string().contains("dictionary"));
    }
}
