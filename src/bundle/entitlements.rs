//! Least-privilege reconciliation of requested Apple signing entitlements.

use super::provisioning::ProvisioningProfile;
use crate::Settings;
use anyhow::Context;
use plist::{Dictionary, Value};
use std::path::Path;

/// Return a concrete entitlement dictionary authorized for this app.
pub(super) fn reconcile(
    settings: &Settings,
    profile: &ProvisioningProfile,
) -> anyhow::Result<Dictionary> {
    let bundle_id = settings.bundle_identifier();
    anyhow::ensure!(
        !bundle_id.is_empty(),
        "Apple bundle identifier must not be empty when using a provisioning profile"
    );
    let macos = settings.target_os() == "macos";
    let app_key = if macos {
        "com.apple.application-identifier"
    } else {
        "application-identifier"
    };
    let alternate_app_key = if macos {
        "application-identifier"
    } else {
        "com.apple.application-identifier"
    };
    let concrete_app_id = profile.application_identifier().to_owned();
    let team_id = profile.team_identifier()?;
    let profile_app_id = profile.entitlement_values().get(app_key).and_then(Value::as_string)
        .ok_or_else(|| anyhow::anyhow!("Provisioning profile is missing platform entitlement `{app_key}` required for signing"))?;
    anyhow::ensure!(
        wildcard_matches(profile_app_id, &concrete_app_id),
        "Provisioning profile `{app_key}` does not authorize `{concrete_app_id}`"
    );
    if let Some(alternate) = profile.entitlement_values().get(alternate_app_key) {
        let alternate = alternate.as_string().ok_or_else(|| {
            anyhow::anyhow!(
                "Provisioning profile entitlement `{alternate_app_key}` must be a string"
            )
        })?;
        anyhow::ensure!(
            wildcard_matches(alternate, &concrete_app_id),
            "Provisioning profile `{alternate_app_key}` conflicts with `{app_key}` for `{concrete_app_id}`"
        );
    }
    let profile_team = profile.entitlement_values().get("com.apple.developer.team-identifier")
        .and_then(Value::as_string)
        .ok_or_else(|| anyhow::anyhow!("Provisioning profile is missing `com.apple.developer.team-identifier` required for signing"))?;
    anyhow::ensure!(
        profile_team == team_id,
        "Provisioning profile team entitlement `{profile_team}` conflicts with TeamIdentifier `{team_id}`"
    );
    let mut requested = if let Some(path) = settings.apple_signing_entitlements() {
        read_requested(path)?
    } else {
        Dictionary::new()
    };

    if let Some(value) = requested.remove(alternate_app_key) {
        let value = expand(&value, profile, &bundle_id)?;
        anyhow::ensure!(
            value.as_string() == Some(concrete_app_id.as_str()),
            "Requested `{alternate_app_key}` conflicts with the platform application identifier `{concrete_app_id}`"
        );
    }
    if let Some(value) = requested.remove(app_key) {
        let value = expand(&value, profile, &bundle_id)?;
        anyhow::ensure!(
            value.as_string() == Some(concrete_app_id.as_str()),
            "Requested `{app_key}` must resolve to `{concrete_app_id}`"
        );
    }
    let mut resolved = Dictionary::new();
    for (key, value) in &requested {
        let value = expand(value, profile, &bundle_id)
            .with_context(|| format!("Invalid requested Apple entitlement `{key}`"))?;
        let profile_value = profile.entitlement_values().get(key);
        let macos_unrestricted = macos && is_unrestricted_macos_entitlement(key);
        if !macos_unrestricted {
            let allowed = profile_value.ok_or_else(|| anyhow::anyhow!("Requested Apple entitlement `{key}` is not authorized by the provisioning profile"))?;
            anyhow::ensure!(
                authorized_value(&value, allowed),
                "Requested Apple entitlement `{key}` is not authorized by the provisioning profile"
            );
        }
        ensure_concrete(key, &value)?;
        resolved.insert(key.clone(), value);
    }
    requested = resolved;

    // Apple documents these as required identity claims; profile grants not
    // explicitly requested above are deliberately not copied into the signature.
    let application_identity = Value::String(concrete_app_id);
    let team_identity = Value::String(team_id.to_owned());
    ensure_concrete(app_key, &application_identity)?;
    ensure_concrete("com.apple.developer.team-identifier", &team_identity)?;
    requested.insert(app_key.to_owned(), application_identity);
    requested.insert(
        "com.apple.developer.team-identifier".to_owned(),
        team_identity,
    );
    Ok(requested)
}

fn read_requested(path: &Path) -> anyhow::Result<Dictionary> {
    let value = Value::from_file(path)
        .with_context(|| format!("Failed to read Apple signing entitlements {path:?}"))?;
    value
        .as_dictionary()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Apple signing entitlements must be a plist dictionary"))
}

fn expand(value: &Value, profile: &ProvisioningProfile, bundle_id: &str) -> anyhow::Result<Value> {
    match value {
        Value::String(text) => {
            let expanded = text
                .replace(
                    "$(AppIdentifierPrefix)",
                    &format!("{}.", profile.application_prefix()),
                )
                .replace(
                    "$(TeamIdentifierPrefix)",
                    &format!("{}.", profile.team_identifier()?),
                )
                .replace("$(CFBundleIdentifier)", bundle_id);
            anyhow::ensure!(
                !expanded.contains("$("),
                "Unsupported unresolved entitlement variable in `{text}`"
            );
            Ok(Value::String(expanded))
        }
        Value::Array(values) => Ok(Value::Array(
            values
                .iter()
                .map(|v| expand(v, profile, bundle_id))
                .collect::<anyhow::Result<_>>()?,
        )),
        _ => Ok(value.clone()),
    }
}

fn authorized_value(requested: &Value, allowed: &Value) -> bool {
    match (requested, allowed) {
        (Value::String(requested), Value::String(allowed)) => wildcard_matches(allowed, requested),
        (Value::Boolean(false), Value::Boolean(true)) => true,
        (Value::Boolean(requested), Value::Boolean(allowed)) => requested == allowed,
        (Value::Array(requested), Value::Array(allowed)) => {
            requested.iter().all(|item| match item {
                Value::String(item) => allowed.iter().any(|grant| {
                    grant
                        .as_string()
                        .is_some_and(|grant| wildcard_matches(grant, item))
                }),
                _ => false,
            })
        }
        _ => false,
    }
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    // Apple provisioning wildcards are a single trailing `*`, used to grant
    // all identifiers below a fixed prefix (for example TEAM.group.*).
    if let Some(prefix) = pattern.strip_suffix('*') {
        return !prefix.contains('*') && value.starts_with(prefix);
    }
    if pattern.contains('*') {
        return false;
    }
    pattern == value
}

fn ensure_concrete(key: &str, value: &Value) -> anyhow::Result<()> {
    match value {
        Value::String(text) => anyhow::ensure!(
            !text.contains('*') && !text.contains('$'),
            "Apple entitlement `{key}` contains an unresolved wildcard or variable"
        ),
        Value::Array(values) => {
            for item in values {
                let Value::String(text) = item else {
                    anyhow::bail!(
                        "Unsupported array value for Apple entitlement `{key}`; arrays must contain only strings"
                    )
                };
                anyhow::ensure!(
                    !text.contains('*') && !text.contains('$'),
                    "Apple entitlement `{key}` contains an unresolved wildcard or variable"
                );
            }
        }
        Value::Boolean(_) => {}
        _ => anyhow::bail!(
            "Unsupported value shape for Apple entitlement `{key}`; use strings, booleans, or arrays of strings"
        ),
    }
    Ok(())
}

fn is_unrestricted_macos_entitlement(key: &str) -> bool {
    // TN3125 lists get-task-allow, application-groups, App Sandbox, and
    // Hardened Runtime entitlements as unrestricted on macOS. Keep this an
    // explicit key allowlist rather than exempting the whole security domain.
    matches!(
        key,
        "com.apple.security.get-task-allow"
            | "com.apple.security.application-groups"
            | "com.apple.security.app-sandbox"
            | "com.apple.security.inherit"
            | "com.apple.security.network.client"
            | "com.apple.security.network.server"
            | "com.apple.security.files.user-selected.read-only"
            | "com.apple.security.files.user-selected.read-write"
            | "com.apple.security.files.downloads.read-only"
            | "com.apple.security.files.downloads.read-write"
            | "com.apple.security.files.pictures.read-only"
            | "com.apple.security.files.pictures.read-write"
            | "com.apple.security.files.movies.read-only"
            | "com.apple.security.files.movies.read-write"
            | "com.apple.security.files.music.read-only"
            | "com.apple.security.files.music.read-write"
            | "com.apple.security.cs.allow-jit"
            | "com.apple.security.cs.allow-unsigned-executable-memory"
            | "com.apple.security.cs.disable-executable-page-protection"
            | "com.apple.security.cs.disable-library-validation"
            | "com.apple.security.cs.allow-dyld-environment-variables"
            | "com.apple.security.cs.debugger"
            | "com.apple.security.cs.restrict"
            | "com.apple.security.cs.hard"
            | "com.apple.security.cs.kill"
            | "com.apple.security.cs.get-task-allow"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_authorization_is_subset_only() {
        assert!(authorized_value(
            &Value::String("TEAM123.com.example.app".into()),
            &Value::String("TEAM123.com.example.*".into()),
        ));
        assert!(!authorized_value(
            &Value::String("OTHER.com.example.app".into()),
            &Value::String("TEAM123.com.example.*".into()),
        ));
        assert!(!wildcard_matches("TEAM*123*", "TEAMx123y"));
        assert!(!wildcard_matches("TEAM.*", "TEAMxgroup"));
    }

    #[test]
    fn false_boolean_claim_is_allowed_by_true_grant_only() {
        assert!(authorized_value(
            &Value::Boolean(false),
            &Value::Boolean(true)
        ));
        assert!(!authorized_value(
            &Value::Boolean(true),
            &Value::Boolean(false)
        ));
    }

    #[test]
    fn requested_string_arrays_must_be_authorized_subsets() {
        let grant = Value::Array(vec![
            Value::String("TEAM123.group.*".into()),
            Value::String("com.apple.token".into()),
        ]);
        assert!(authorized_value(
            &Value::Array(vec![Value::String("TEAM123.group.one".into())]),
            &grant,
        ));
        assert!(!authorized_value(
            &Value::Array(vec![
                Value::String("TEAM123.group.one".into()),
                Value::String("other".into())
            ]),
            &grant,
        ));
    }

    #[test]
    fn output_rejects_unexpanded_patterns_and_unsupported_shapes() {
        assert!(ensure_concrete("example", &Value::String("TEAM.*".into())).is_err());
        assert!(ensure_concrete("example", &Value::String("$(OTHER)".into())).is_err());
        assert!(ensure_concrete("example", &Value::Dictionary(Dictionary::new())).is_err());
        assert!(ensure_concrete("example", &Value::Array(vec![Value::Boolean(true)])).is_err());
    }
}
