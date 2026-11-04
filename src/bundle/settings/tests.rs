use super::{AppCategory, BundleSettings, normalize_platform_settings, validate_platform_settings};
use crate::bundle::localization::DesktopKeywords;
use std::path::PathBuf;

#[test]
fn inferred_formats_are_single_native_outputs() {
    use super::PackageType;
    for (os, expected) in [
        ("macos", PackageType::OsxBundle),
        ("linux", PackageType::Deb),
        ("windows", PackageType::WindowsBundle),
        ("ios", PackageType::IosBundle),
    ] {
        assert_eq!(PackageType::default_for_os(os).unwrap(), expected);
    }
}

#[test]
fn mixed_operating_system_targets_are_rejected_in_any_order() {
    use clap::Parser;
    let project = tempfile::tempdir().unwrap();
    for (first, second) in [
        ("x86_64-unknown-linux-gnu", "aarch64-linux-android"),
        ("aarch64-linux-android", "x86_64-unknown-linux-gnu"),
    ] {
        let cli =
            crate::Cli::try_parse_from(["cargo-bundle", "--target", first, "--target", second])
                .unwrap();
        let error = super::Settings::new(project.path().to_path_buf(), &cli).unwrap_err();
        assert!(error.to_string().contains("same operating system"));
    }
}

#[test]
fn every_platform_can_shadow_common_values_and_clear_resources() {
    for (os, table) in [
        ("macos", "macos"),
        ("linux", "linux"),
        ("windows", "windows"),
        ("ios", "ios"),
    ] {
        let mut value = serde_json::json!({"name": "Shared", "resources": ["shared"]});
        value[table] = serde_json::json!({"name": os, "resources": []});
        normalize_platform_settings(&mut value, os).unwrap();
        let bundle: BundleSettings = serde_json::from_value(value).unwrap();
        assert_eq!(bundle.name.as_deref(), Some(os));
        assert_eq!(bundle.resources, Some(Vec::new()));
    }
}

#[test]
fn windows_target_without_format_uses_exe_input_and_output() {
    use clap::Parser;
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("src")).unwrap();
    std::fs::write(project.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(project.path().join("Cargo.toml"),
        "[package]\nname = \"inferred-app\"\nversion = \"1.0.0\"\nedition = \"2024\"\n[package.metadata.bundle]\nname = \"Shared\"\n[package.metadata.bundle.windows]\nname = \"Windows App\"\n").unwrap();
    let cli =
        crate::Cli::try_parse_from(["cargo-bundle", "--target", "x86_64-pc-windows-msvc"]).unwrap();
    let settings = super::Settings::new(project.path().to_path_buf(), &cli).unwrap();
    assert!(settings.binary_path().ends_with("inferred-app.exe"));
    assert_eq!(settings.package_type(), super::PackageType::WindowsBundle);
    assert_eq!(settings.bundle_name(), "Windows App");
}

#[test]
fn parse_cargo_toml() {
    let toml_str = "\
        name = \"Example Application\"\n\
        identifier = \"com.example.app\"\n\
        resources = [\"data\", \"foo/bar\"]\n\
        category = \"Puzzle Game\"\n\
        long_description = \"\"\"\n\
        This is an example of a\n\
        simple application.\n\
        \"\"\"\n";
    let bundle: BundleSettings = toml::from_str(toml_str).unwrap();
    assert_eq!(bundle.name, Some("Example Application".to_string()));
    assert_eq!(bundle.identifier, Some("com.example.app".to_string()));
    assert_eq!(bundle.icon, None);
    assert_eq!(bundle.version, None);
    assert_eq!(
        bundle.resources,
        Some(vec!["data".to_string(), "foo/bar".to_string()])
    );
    assert_eq!(bundle.category, Some(AppCategory::PuzzleGame));
    assert_eq!(
        bundle.long_description,
        Some(
            "This is an example of a\n\
                     simple application.\n"
                .to_string()
        )
    );
}

#[test]
fn platform_tables_are_typed() {
    let bundle: BundleSettings = toml::from_str(
        r#"
            name = "Cross-platform name"
            identifier = "com.example.shared"
            resources = ["shared"]

            [osx]
            frameworks = ["WebKit.framework"]
        "#,
    )
    .unwrap();

    assert_eq!(bundle.name.as_deref(), Some("Cross-platform name"));
    assert_eq!(
        bundle.osx.unwrap().frameworks,
        Some(vec!["WebKit.framework".into()])
    );
    assert!(toml::from_str::<BundleSettings>("frameworks = [\"WebKit.framework\"]").is_err());
    assert!(toml::from_str::<BundleSettings>("osx_frameworks = [\"WebKit.framework\"]").is_err());
}

#[test]
fn platform_tables_shadow_common_settings_and_replace_arrays() {
    let mut value: serde_json::Value = toml::from_str(
        r#"
            name = "Shared"
            resources = ["root"]
            icon = ["root.png"]
            apple_signing_hardened_runtime = true

            [linux]
            name = "Linux"
            resources = []
            mime_types = ["application/example"]
            apple_signing_hardened_runtime = false
            apple_signing_p12 = "linux-cert.p12"

            [windows]
            name = "Windows"
            icon = ["windows.ico"]
        "#,
    )
    .unwrap();
    normalize_platform_settings(&mut value, "linux").unwrap();
    let settings: BundleSettings = serde_json::from_value(value).unwrap();
    assert_eq!(settings.name.as_deref(), Some("Linux"));
    assert_eq!(settings.resources, Some(Vec::new()));
    assert_eq!(settings.icon, Some(vec!["root.png".into()]));
    assert_eq!(settings.apple_signing_hardened_runtime, Some(false));
    assert_eq!(
        settings.apple_signing_p12,
        Some(PathBuf::from("linux-cert.p12"))
    );
    assert_eq!(
        settings.linux.unwrap().mime_types,
        Some(vec!["application/example".into()])
    );
}

#[test]
fn named_target_platform_settings_shadow_target_common_settings() {
    let mut value: serde_json::Value = toml::from_str(
        r#"
            name = "Root"
            short_description = "Root description"

            [bin.cli]
            resources = ["cli"]

            [bin.cli.windows]
            name = "CLI for Windows"
            resources = []

            [bin.gui]
            name = "GUI"
        "#,
    )
    .unwrap();
    normalize_platform_settings(&mut value, "windows").unwrap();
    let settings: BundleSettings = serde_json::from_value(value).unwrap();
    let cli = settings.bin.unwrap().remove("cli").unwrap();
    assert_eq!(cli.name.as_deref(), Some("CLI for Windows"));
    assert_eq!(cli.resources, Some(Vec::new()));
    assert_eq!(cli.short_description, None);
    assert_eq!(settings.name.as_deref(), Some("Root"));
}

#[test]
fn macos_alias_preserves_backend_fields_and_rejects_unknown_platform_keys() {
    let mut value: serde_json::Value = toml::from_str(
        r#"
            name = "Shared"
            [macos]
            name = "Mac"
            frameworks = ["Cocoa.framework"]
        "#,
    )
    .unwrap();
    normalize_platform_settings(&mut value, "macos").unwrap();
    let settings: BundleSettings = serde_json::from_value(value).unwrap();
    assert_eq!(settings.name.as_deref(), Some("Mac"));
    assert_eq!(
        settings.osx.unwrap().frameworks,
        Some(vec!["Cocoa.framework".into()])
    );

    let mut bad: serde_json::Value = toml::from_str("[windows]\nmade_up = true").unwrap();
    assert!(
        normalize_platform_settings(&mut bad, "windows")
            .unwrap_err()
            .to_string()
            .contains("made_up")
    );
}

#[test]
fn validates_inactive_platforms_and_rejects_osx_alias_conflicts() {
    let inactive_bad: serde_json::Value = toml::from_str(
        r#"
            [linux]
            mime_types = ["application/example"]
            [windows]
            unsupported_setting = true
        "#,
    )
    .unwrap();
    assert!(
        validate_platform_settings(&inactive_bad)
            .unwrap_err()
            .to_string()
            .contains("unsupported_setting")
    );

    let conflict: serde_json::Value = toml::from_str(
        r#"
            [osx]
            frameworks = ["Cocoa.framework"]
            [macos]
            name = "Mac"
        "#,
    )
    .unwrap();
    assert!(
        validate_platform_settings(&conflict)
            .unwrap_err()
            .to_string()
            .contains("Use only one")
    );
}

#[test]
fn parses_signing_configuration() {
    let bundle: BundleSettings = toml::from_str(
        r#"
            apple_signing_p12 = "certs/apple.p12"
            apple_signing_password_env = "APPLE_SIGNING_PASSWORD"

            [windows_signing]
            certificate_path = "certs/windows.p12"
            certificate_password_env = "WINDOWS_SIGNING_PASSWORD"

            [linux_signing]
            identity_token_env = "SIGSTORE_ID_TOKEN"
        "#,
    )
    .unwrap();

    assert_eq!(
        bundle.apple_signing_p12,
        Some(PathBuf::from("certs/apple.p12"))
    );
    assert_eq!(
        bundle
            .windows_signing
            .as_ref()
            .unwrap()
            .certificate_password_env
            .as_deref(),
        Some("WINDOWS_SIGNING_PASSWORD")
    );
    assert_eq!(
        bundle.linux_signing.as_ref().unwrap().identity_token_env,
        "SIGSTORE_ID_TOKEN"
    );
}

#[test]
fn apple_notarization_settings_parse_and_platform_values_shadow() {
    let mut value: serde_json::Value = toml::from_str(
        r#"
            apple_notarization_api_key = "root/AuthKey.p8"
            apple_notarization_timeout = 900
            [macos]
            apple_notarization_api_key = "macos/AuthKey.p8"
            apple_notarization_timeout = 1200
        "#,
    )
    .unwrap();
    normalize_platform_settings(&mut value, "macos").unwrap();
    let bundle: BundleSettings = serde_json::from_value(value).unwrap();
    assert_eq!(
        bundle.apple_notarization_api_key,
        Some(PathBuf::from("macos/AuthKey.p8"))
    );
    assert_eq!(bundle.apple_notarization_timeout, Some(1200));

    let default: BundleSettings = toml::from_str("").unwrap();
    assert_eq!(default.apple_notarization_timeout.unwrap_or(600), 600);

    let invalid: serde_json::Value = toml::from_str("apple_notarization_timeout = 0").unwrap();
    assert!(
        validate_platform_settings(&invalid)
            .unwrap_err()
            .to_string()
            .contains("greater than zero")
    );
}

#[test]
fn parse_bin_and_example_bundles() {
    let toml_str = "\
        [bin.foo]\n\
        name = \"Foo App\"\n\
        \n\
        [bin.bar]\n\
        name = \"Bar App\"\n\
        \n\
        [example.baz]\n\
        name = \"Baz Example\"\n";
    let bundle: BundleSettings = toml::from_str(toml_str).unwrap();
    assert!(bundle.example.is_some());

    let bins = bundle.bin.as_ref().unwrap();
    assert!(bins.contains_key("foo"));
    let foo: &BundleSettings = bins.get("foo").unwrap();
    assert_eq!(foo.name, Some("Foo App".to_string()));
    assert!(bins.contains_key("bar"));
    let bar: &BundleSettings = bins.get("bar").unwrap();
    assert_eq!(bar.name, Some("Bar App".to_string()));

    let examples = bundle.example.as_ref().unwrap();
    assert!(examples.contains_key("baz"));
    let baz: &BundleSettings = examples.get("baz").unwrap();
    assert_eq!(baz.name, Some("Baz Example".to_string()));
}

#[test]
fn provisioning_profile_follows_platform_shadowing() {
    let mut value = serde_json::json!({
        "apple_provisioning_profile": "shared.mobileprovision",
        "ios": { "apple_provisioning_profile": "device.mobileprovision" }
    });
    validate_platform_settings(&value).unwrap();
    normalize_platform_settings(&mut value, "ios").unwrap();
    let bundle: BundleSettings = serde_json::from_value(value).unwrap();
    assert_eq!(
        bundle.apple_provisioning_profile.unwrap(),
        std::path::PathBuf::from("device.mobileprovision")
    );
}

#[test]
fn osx_localizations_parses_from_toml() {
    let toml_str = r#"
        [osx.localizations.fr]
        CFBundleDisplayName = "Mon Application"

        [osx.localizations.de]
        CFBundleDisplayName = "Meine Anwendung"
    "#;
    let bundle: BundleSettings = toml::from_str(toml_str).unwrap();
    let locs = bundle.osx.unwrap().localizations.unwrap();
    assert_eq!(locs["fr"]["CFBundleDisplayName"], "Mon Application");
    assert_eq!(locs["de"]["CFBundleDisplayName"], "Meine Anwendung");
}

#[test]
fn linux_localizations_parses_from_toml() {
    let toml_str = r#"
        [linux.localizations.fr]
        Name = "Mon App"
        Comment = "Une description"
        GenericName = "Utilitaire"
        Keywords = ["outil", "utilitaire"]

        [linux.localizations.de]
        Name = "Meine App"
        Comment = "Eine Beschreibung"
        Keywords = "werkzeug;dienstprogramm"

        [linux.localizations.pt_BR]
        Name = "Meu App"
        Comment = "Uma descrição"
    "#;
    let bundle: BundleSettings = toml::from_str(toml_str).unwrap();
    let locs = bundle.linux.unwrap().localizations.unwrap();

    assert_eq!(locs["fr"].name.as_deref(), Some("Mon App"));
    assert_eq!(locs["fr"].comment.as_deref(), Some("Une description"));
    assert_eq!(locs["fr"].generic_name.as_deref(), Some("Utilitaire"));
    match locs["fr"].keywords.as_ref().unwrap() {
        DesktopKeywords::List(items) => {
            assert_eq!(items, &["outil".to_string(), "utilitaire".to_string()]);
        }
        DesktopKeywords::String(_) => panic!("expected Keywords list for fr"),
    }

    assert_eq!(locs["de"].name.as_deref(), Some("Meine App"));
    match locs["de"].keywords.as_ref().unwrap() {
        DesktopKeywords::String(s) => assert_eq!(s, "werkzeug;dienstprogramm"),
        DesktopKeywords::List(_) => panic!("expected Keywords string for de"),
    }

    assert_eq!(locs["pt_BR"].name.as_deref(), Some("Meu App"));
    assert!(locs["pt_BR"].generic_name.is_none());
    assert!(locs["pt_BR"].keywords.is_none());
}

#[test]
fn external_localizations_merge_in_order_with_inline_precedence() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("first.toml"),
        r#"
            [linux.fr]
            Name = "First"
            Comment = "from first"
            [macos.fr]
            CFBundleDisplayName = "Mac first"
            [ios.fr]
            CFBundleDisplayName = "iOS first"
        "#,
    )
    .unwrap();
    std::fs::write(
        directory.path().join("second.toml"),
        r#"
            [linux.fr]
            Name = "Second"
            GenericName = "Generic"
            [macos.fr]
            CFBundleName = "Mac second"
        "#,
    )
    .unwrap();
    let mut bundle: BundleSettings = toml::from_str(
        r#"
            localization_files = ["first.toml", "second.toml"]
            [linux.localizations.fr]
            Name = "Inline"
        "#,
    )
    .unwrap();

    super::load_localization_files(&mut bundle, directory.path(), "linux").unwrap();
    let locale = &bundle.linux.unwrap().localizations.unwrap()["fr"];
    assert_eq!(locale.name.as_deref(), Some("Inline"));
    assert_eq!(locale.comment.as_deref(), Some("from first"));
    assert_eq!(locale.generic_name.as_deref(), Some("Generic"));
}

#[test]
fn external_apple_localizations_support_macos_and_ios_tables() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("locales.toml"),
        r#"
            [macos.fr]
            CFBundleDisplayName = "macOS name"
            [ios.fr]
            CFBundleDisplayName = "iOS name"
        "#,
    )
    .unwrap();
    let mut macos: BundleSettings =
        toml::from_str("localization_files = [\"locales.toml\"]").unwrap();
    super::load_localization_files(&mut macos, directory.path(), "macos").unwrap();
    assert_eq!(
        macos.osx.unwrap().localizations.unwrap()["fr"]["CFBundleDisplayName"],
        "macOS name"
    );

    let mut ios: BundleSettings =
        toml::from_str("localization_files = [\"locales.toml\"]").unwrap();
    super::load_localization_files(&mut ios, directory.path(), "ios").unwrap();
    assert_eq!(
        ios.ios.unwrap().localizations.unwrap()["fr"]["CFBundleDisplayName"],
        "iOS name"
    );
}

#[test]
fn named_target_uses_only_its_localization_file_list() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("target.toml"),
        "[linux.fr]\nName = \"Target file\"\n",
    )
    .unwrap();
    let mut value: serde_json::Value = toml::from_str(
        r#"
            localization_files = ["root-file-must-not-be-read.toml"]
            [bin.demo]
            localization_files = ["target.toml"]
            [bin.demo.linux.localizations.fr]
            Comment = "inline target value"
        "#,
    )
    .unwrap();
    super::normalize_platform_settings(&mut value, "linux").unwrap();
    let root: BundleSettings = serde_json::from_value(value).unwrap();
    let mut target = super::bundle_settings_from_table(&root.bin, "bin", "demo").unwrap();
    super::load_localization_files(&mut target, directory.path(), "linux").unwrap();
    let locale = &target.linux.unwrap().localizations.unwrap()["fr"];
    assert_eq!(locale.name.as_deref(), Some("Target file"));
    assert_eq!(locale.comment.as_deref(), Some("inline target value"));
}

#[test]
fn external_localization_files_validate_inactive_tables_and_locale_codes() {
    assert!(
        super::localizations::parse_localization_file(
            toml::from_str(
                r#"[windows.fr]
        Name = "unsupported""#
            )
            .unwrap()
        )
        .is_err()
    );
    assert!(
        super::localizations::parse_localization_file(
            toml::from_str(
                r#"[linux."../fr"]
        Name = "unsafe""#
            )
            .unwrap()
        )
        .is_err()
    );
    assert!(
        super::localizations::parse_localization_file(
            toml::from_str(
                r#"[ios.fr]
        CFBundleDisplayName = 42"#
            )
            .unwrap()
        )
        .is_err()
    );
}

#[test]
fn parse_appimage_feature_settings() {
    let toml_str = r#"
        name = "AppImage App"
        identifier = "com.example.appimage"
        appimage_metainfo_path = "assets/metainfo.xml"
        appimage_compression = "lz4"

        [linux]
        startup_wm_class = "myapp"

        [linux.desktop_actions.new-window]
        Name = "New Window"
        Exec = "myapp --new-window"

        [linux.desktop_actions.new-window.NameLocalized]
        fr = "Nouvelle fenetre"
    "#;
    let bundle: BundleSettings = toml::from_str(toml_str).unwrap();
    assert_eq!(
        bundle.appimage_metainfo_path.as_deref(),
        Some("assets/metainfo.xml")
    );
    assert_eq!(bundle.appimage_compression.as_deref(), Some("lz4"));
    assert_eq!(
        bundle.linux.as_ref().unwrap().startup_wm_class.as_deref(),
        Some("myapp")
    );

    let actions = bundle.linux.unwrap().desktop_actions.unwrap();
    let action = &actions["new-window"];
    assert_eq!(action.name, "New Window");
    assert_eq!(action.exec.as_deref(), Some("myapp --new-window"));
    assert!(action.icon.is_none());
    assert_eq!(
        action.name_localized.as_ref().unwrap()["fr"],
        "Nouvelle fenetre"
    );
}

#[test]
fn format_names_round_trip_through_the_cli() {
    use super::PackageType;
    use clap::Parser;
    for (name, expected) in [
        ("deb", PackageType::Deb),
        ("ios", PackageType::IosBundle),
        ("ipa", PackageType::IosIpa),
        ("msi", PackageType::WindowsMsi),
        ("wxsmsi", PackageType::WxsMsi),
        ("osx", PackageType::OsxBundle),
        ("dmg", PackageType::OsxDmg),
        ("rpm", PackageType::Rpm),
        ("appimage", PackageType::AppImage),
        ("exe", PackageType::WindowsBundle),
    ] {
        let cli = crate::Cli::try_parse_from(["cargo-bundle", "--format", name]).unwrap();
        assert_eq!(cli.format, Some(expected));
        assert_eq!(expected.to_string(), name);
    }
}

#[test]
fn metadata_uses_the_selected_projects_target_directory() {
    use clap::Parser;
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("src")).unwrap();
    std::fs::create_dir(project.path().join(".cargo")).unwrap();
    std::fs::write(project.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(
        project.path().join("Cargo.toml"),
        "[package]\nname = \"target-path-check\"\nversion = \"1.0.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    std::fs::write(
        project.path().join(".cargo/config.toml"),
        "[build]\ntarget-dir = \"built\"\n",
    )
    .unwrap();
    let cli = crate::Cli::try_parse_from(["cargo-bundle", "--format", "deb"]).unwrap();
    let settings = super::Settings::new(project.path().to_owned(), &cli).unwrap();
    let base = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                project.path().join(path)
            }
        })
        .unwrap_or_else(|| project.path().join("built"));
    // macOS exposes /var through /private/var; compare the resolved existing parent.
    let expected_parent = base.parent().unwrap().canonicalize().unwrap();
    let expected = expected_parent
        .join(base.file_name().unwrap())
        .join("debug");
    assert_eq!(settings.project_out_directory(), expected);
}

#[test]
fn split_localization_aliases_and_unknown_fields_remain_strict() {
    for text in [
        "[macos.fr]\nName = 'one'\n[osx.fr]\nName = 'two'\n",
        "[android.fr]\napp_name = 'deferred'\n",
        "[linux.fr]\nMisspelledName = 'wrong'\n",
    ] {
        assert!(
            super::localizations::parse_localization_file(toml::from_str(text).unwrap()).is_err()
        );
    }
}
