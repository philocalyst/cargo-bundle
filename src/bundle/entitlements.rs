//! Least-privilege reconciliation of requested Apple signing entitlements.
//!
//! Entitlements are read into [`Entitlement`], so a value of a shape that
//! cannot be signed or checked against a profile never gets this far.

use super::provisioning::Profile;
use super::settings::TargetOs;
use anyhow::Context;
use serde::{Deserialize, Serialize, de::IntoDeserializer};
use std::collections::BTreeMap;
use std::path::Path;

/// A signable entitlement value.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub(super) enum Entitlement {
    Flag(bool),
    Text(String),
    List(Vec<String>),
}

pub(super) type Entitlements = BTreeMap<String, Entitlement>;

/// What a profile grants for one key. A grant of a shape that cannot be
/// reasoned about (a number, a dictionary) authorizes nothing.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum Grant {
    Known(Entitlement),
    Opaque(#[allow(dead_code)] serde::de::IgnoredAny),
}

pub(super) type Grants = BTreeMap<String, Grant>;

/// A granted string: `*`, a fixed prefix then `*`, or exactly one value.
enum Pattern<'a> {
    Any,
    Prefix(&'a str),
    Exact(&'a str),
}

impl<'a> From<&'a str> for Pattern<'a> {
    fn from(grant: &'a str) -> Self {
        match grant.strip_suffix('*') {
            Some("") => Self::Any,
            Some(prefix) if !prefix.contains('*') => Self::Prefix(prefix),
            // A `*` anywhere else is Apple's notation for nothing we know, so
            // it only equals itself, which no concrete value contains.
            _ => Self::Exact(grant),
        }
    }
}

impl Pattern<'_> {
    fn matches(&self, value: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Prefix(prefix) => value.starts_with(prefix),
            Self::Exact(exact) => *exact == value,
        }
    }
}

/// Whether `grant` allows `value`; the grant may be a pattern.
pub(super) fn grants(grant: &str, value: &str) -> bool {
    Pattern::from(grant).matches(value)
}

/// Entitlements Apple treats as unrestricted on macOS, so that no profile
/// grant is needed (TN3125 lists them). An explicit list rather than a whole
/// security domain.
#[derive(Deserialize)]
enum Unrestricted {
    #[serde(rename = "com.apple.security.get-task-allow")]
    GetTaskAllow,
    #[serde(rename = "com.apple.security.application-groups")]
    ApplicationGroups,
    #[serde(rename = "com.apple.security.app-sandbox")]
    AppSandbox,
    #[serde(rename = "com.apple.security.inherit")]
    Inherit,
    #[serde(rename = "com.apple.security.network.client")]
    NetworkClient,
    #[serde(rename = "com.apple.security.network.server")]
    NetworkServer,
    #[serde(rename = "com.apple.security.files.user-selected.read-only")]
    UserSelectedReadOnly,
    #[serde(rename = "com.apple.security.files.user-selected.read-write")]
    UserSelectedReadWrite,
    #[serde(rename = "com.apple.security.files.downloads.read-only")]
    DownloadsReadOnly,
    #[serde(rename = "com.apple.security.files.downloads.read-write")]
    DownloadsReadWrite,
    #[serde(rename = "com.apple.security.files.pictures.read-only")]
    PicturesReadOnly,
    #[serde(rename = "com.apple.security.files.pictures.read-write")]
    PicturesReadWrite,
    #[serde(rename = "com.apple.security.files.movies.read-only")]
    MoviesReadOnly,
    #[serde(rename = "com.apple.security.files.movies.read-write")]
    MoviesReadWrite,
    #[serde(rename = "com.apple.security.files.music.read-only")]
    MusicReadOnly,
    #[serde(rename = "com.apple.security.files.music.read-write")]
    MusicReadWrite,
    #[serde(rename = "com.apple.security.cs.allow-jit")]
    AllowJit,
    #[serde(rename = "com.apple.security.cs.allow-unsigned-executable-memory")]
    AllowUnsignedExecutableMemory,
    #[serde(rename = "com.apple.security.cs.disable-executable-page-protection")]
    DisableExecutablePageProtection,
    #[serde(rename = "com.apple.security.cs.disable-library-validation")]
    DisableLibraryValidation,
    #[serde(rename = "com.apple.security.cs.allow-dyld-environment-variables")]
    AllowDyldEnvironmentVariables,
    #[serde(rename = "com.apple.security.cs.debugger")]
    Debugger,
    #[serde(rename = "com.apple.security.cs.restrict")]
    Restrict,
    #[serde(rename = "com.apple.security.cs.hard")]
    Hard,
    #[serde(rename = "com.apple.security.cs.kill")]
    Kill,
    #[serde(rename = "com.apple.security.cs.get-task-allow")]
    CsGetTaskAllow,
}

impl Unrestricted {
    fn is(key: &str) -> bool {
        Self::deserialize(IntoDeserializer::<serde::de::value::Error>::into_deserializer(key))
            .is_ok()
    }
}

/// Read the entitlements a user asks to sign with, from an XML or binary plist.
pub(super) fn read(path: &Path) -> anyhow::Result<Entitlements> {
    plist::from_file(path).with_context(|| {
        format!(
            "Failed to read Apple signing entitlements {path:?}; they must be a plist \
             dictionary of booleans, strings and lists of strings"
        )
    })
}

/// Return a concrete entitlement set authorized for this app: what was
/// requested, checked against what `profile` grants, plus the application and
/// team identity claims.
pub(super) fn reconcile(
    mut requested: Entitlements,
    profile: &Profile,
    bundle_id: &str,
    os: TargetOs,
) -> anyhow::Result<Entitlements> {
    anyhow::ensure!(
        !bundle_id.is_empty(),
        "Apple bundle identifier must not be empty when using a provisioning profile"
    );
    let macos = os == TargetOs::MacOs;
    let (app_key, alternate_app_key) = if macos {
        ("com.apple.application-identifier", "application-identifier")
    } else {
        ("application-identifier", "com.apple.application-identifier")
    };
    let app_id = profile.application_identifier();
    let team_id = profile.team_identifier()?;
    let granted = profile.granted();
    let text = |key: &str| match granted.get(key) {
        Some(Grant::Known(Entitlement::Text(text))) => Some(text.as_str()),
        _ => None,
    };

    let profile_app_id = text(app_key).with_context(|| {
        format!(
            "Provisioning profile is missing platform entitlement `{app_key}` required for signing"
        )
    })?;
    anyhow::ensure!(
        grants(profile_app_id, app_id),
        "Provisioning profile `{app_key}` does not authorize `{app_id}`"
    );
    if let Some(alternate) = granted.get(alternate_app_key) {
        let Grant::Known(Entitlement::Text(alternate)) = alternate else {
            anyhow::bail!(
                "Provisioning profile entitlement `{alternate_app_key}` must be a string"
            );
        };
        anyhow::ensure!(
            grants(alternate, app_id),
            "Provisioning profile `{alternate_app_key}` conflicts with `{app_key}` for `{app_id}`"
        );
    }
    let profile_team = text("com.apple.developer.team-identifier").context(
        "Provisioning profile is missing `com.apple.developer.team-identifier` required for signing",
    )?;
    anyhow::ensure!(
        profile_team == team_id,
        "Provisioning profile team entitlement `{profile_team}` conflicts with TeamIdentifier `{team_id}`"
    );

    // The identity claims are ours to write; a request may only repeat them.
    for key in [alternate_app_key, app_key] {
        if let Some(value) = requested.remove(key) {
            anyhow::ensure!(
                expand(&value, profile, bundle_id)? == Entitlement::Text(app_id.to_owned()),
                "Requested `{key}` must resolve to the application identifier `{app_id}`"
            );
        }
    }
    let mut resolved = Entitlements::new();
    for (key, value) in &requested {
        let value = expand(value, profile, bundle_id)
            .with_context(|| format!("Invalid requested Apple entitlement `{key}`"))?;
        if !(macos && Unrestricted::is(key)) {
            let authorized = granted
                .get(key)
                .is_some_and(|grant| authorizes(grant, &value));
            anyhow::ensure!(
                authorized,
                "Requested Apple entitlement `{key}` is not authorized by the provisioning profile"
            );
        }
        ensure_concrete(key, &value)?;
        resolved.insert(key.clone(), value);
    }

    // Profile grants that were not requested are deliberately not signed.
    resolved.insert(app_key.to_owned(), Entitlement::Text(app_id.to_owned()));
    resolved.insert(
        "com.apple.developer.team-identifier".to_owned(),
        Entitlement::Text(team_id.to_owned()),
    );
    Ok(resolved)
}

/// Replace `$(AppIdentifierPrefix)`, `$(TeamIdentifierPrefix)` and
/// `$(CFBundleIdentifier)`; any other variable is an error.
fn expand(value: &Entitlement, profile: &Profile, bundle_id: &str) -> anyhow::Result<Entitlement> {
    let one = |text: &str| -> anyhow::Result<String> {
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
        Ok(expanded)
    };
    Ok(match value {
        Entitlement::Flag(flag) => Entitlement::Flag(*flag),
        Entitlement::Text(text) => Entitlement::Text(one(text)?),
        Entitlement::List(items) => Entitlement::List(
            items
                .iter()
                .map(|item| one(item))
                .collect::<Result<_, _>>()?,
        ),
    })
}

/// Whether `grant` covers `requested`: strings and every list item must match
/// a granted pattern, and a flag may be switched off but not on.
fn authorizes(grant: &Grant, requested: &Entitlement) -> bool {
    let Grant::Known(allowed) = grant else {
        return false;
    };
    match (requested, allowed) {
        (Entitlement::Text(requested), Entitlement::Text(allowed)) => grants(allowed, requested),
        (Entitlement::Flag(false), Entitlement::Flag(true)) => true,
        (Entitlement::Flag(requested), Entitlement::Flag(allowed)) => requested == allowed,
        (Entitlement::List(requested), Entitlement::List(allowed)) => requested
            .iter()
            .all(|item| allowed.iter().any(|allowed| grants(allowed, item))),
        _ => false,
    }
}

/// A signed value must have no wildcard or variable left in it.
fn ensure_concrete(key: &str, value: &Entitlement) -> anyhow::Result<()> {
    let concrete = |text: &str| !text.contains(['*', '$']);
    let ok = match value {
        Entitlement::Flag(_) => true,
        Entitlement::Text(text) => concrete(text),
        Entitlement::List(items) => items.iter().all(|item| concrete(item)),
    };
    anyhow::ensure!(
        ok,
        "Apple entitlement `{key}` contains an unresolved wildcard or variable"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_are_whole_values_or_one_trailing_wildcard() {
        assert!(grants("TEAM123.com.example.*", "TEAM123.com.example.app"));
        assert!(!grants("TEAM123.com.example.*", "OTHER.com.example.app"));
        assert!(grants("*", "anything"));
        assert!(grants("exact", "exact"));
        assert!(!grants("exact", "exact.more"));
        // Wildcards anywhere else are not patterns.
        assert!(!grants("TEAM*123*", "TEAMx123y"));
        assert!(!grants("TEAM*123*", "TEAM*123*x"));
    }

    #[test]
    fn a_flag_may_be_switched_off_but_not_on_by_a_request() {
        let allows = |requested, allowed| {
            authorizes(
                &Grant::Known(Entitlement::Flag(allowed)),
                &Entitlement::Flag(requested),
            )
        };
        assert!(allows(false, true));
        assert!(allows(true, true));
        assert!(!allows(true, false));
    }

    #[test]
    fn lists_are_authorized_item_by_item() {
        let grant = Grant::Known(Entitlement::List(vec![
            "TEAM123.group.*".into(),
            "com.apple.token".into(),
        ]));
        let list =
            |items: &[&str]| Entitlement::List(items.iter().map(|i| i.to_string()).collect());
        assert!(authorizes(&grant, &list(&["TEAM123.group.one"])));
        assert!(!authorizes(&grant, &list(&["TEAM123.group.one", "other"])));
        assert!(!authorizes(&grant, &Entitlement::Flag(true)));
    }

    #[test]
    fn only_signable_shapes_can_be_requested() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("entitlements.plist");
        let parse = |plist: &str| {
            std::fs::write(&path, plist).unwrap();
            read(&path)
        };
        let wrap = |body: &str| {
            format!("<?xml version=\"1.0\"?><plist version=\"1.0\"><dict>{body}</dict></plist>")
        };
        let ok = parse(&wrap(
            "<key>a</key><true/><key>b</key><string>x</string>\
             <key>c</key><array><string>y</string></array>",
        ))
        .unwrap();
        assert_eq!(ok["b"], Entitlement::Text("x".into()));
        for bad in [
            "<key>a</key><integer>1</integer>",
            "<key>a</key><dict/>",
            "<key>a</key><array><true/></array>",
        ] {
            assert!(parse(&wrap(bad)).is_err(), "{bad}");
        }
        assert!(parse("<plist version=\"1.0\"><array/></plist>").is_err());
    }

    #[test]
    fn macos_unrestricted_entitlements_are_a_fixed_list() {
        assert!(Unrestricted::is("com.apple.security.app-sandbox"));
        assert!(Unrestricted::is("com.apple.security.cs.allow-jit"));
        assert!(!Unrestricted::is("com.apple.security.device.camera"));
        assert!(!Unrestricted::is("com.apple.developer.healthkit"));
    }
}
