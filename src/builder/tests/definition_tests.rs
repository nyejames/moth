//! Builder definition registration tests.
//!
//! WHAT: checks that the public definition helpers register source and host packages through the
//! existing registries and reject invalid registrations at `finish`.
//! WHY: `finish` is the only path from builder-authored descriptions into compiler-owned
//! registries, so every rejection must happen there rather than during a later build.

use crate::builder::{BuilderDefinition, ErrorKind, HostPackage, SourcePackage};
use crate::builder_surface::PackageOrigin;
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalAccessKind, ExternalSignatureType,
};
use crate::host::{HostFunction, HostType};
use moth_lexical::numeric::fixed_scalar::FixedScalar;

fn package_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("should create package directory")
}

fn invalid_definition_message(
    case: &str,
    result: Result<BuilderDefinition, crate::builder::Error>,
) -> String {
    let error = result.expect_err(&format!("{case}: definition should be rejected"));
    assert_eq!(error.kind(), ErrorKind::InvalidDefinition, "{case}");
    error.to_string()
}

#[test]
fn finish_registers_source_and_host_packages_with_builder_provenance() {
    let ui_dir = package_dir();
    let definition = BuilderDefinition::new("game")
        .source_package(SourcePackage::new("@ui").from_dir(ui_dir.path()))
        .host_package(
            HostPackage::new("@platform").function(
                HostFunction::new("parse_count")
                    .parameter(HostType::String)
                    .parameter(HostType::U8)
                    .returns(HostType::U32)
                    .fallible(),
            ),
        )
        .finish()
        .expect("definition should finish");

    assert_eq!(definition.identity(), "game");
    let surface = definition.surface();

    let source_root = surface
        .source_packages
        .get_root("ui")
        .expect("source package should register by its prefix");
    assert_eq!(source_root.metadata.origin, PackageOrigin::Builder);

    let package = surface
        .binding_packages
        .get_package("@platform")
        .expect("host package should register");
    assert_eq!(package.metadata.origin, PackageOrigin::Builder);

    let (_, function) = surface
        .binding_packages
        .resolve_package_function("@platform", "parse_count")
        .expect("host function should register");
    let parameter_types = function
        .parameters
        .iter()
        .map(|parameter| (&parameter.language_type, parameter.access_kind))
        .collect::<Vec<_>>();
    assert_eq!(
        parameter_types,
        vec![
            (
                &ExternalSignatureType::Abi(ExternalAbiType::Utf8Str),
                ExternalAccessKind::Shared
            ),
            (
                &ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::U8)),
                ExternalAccessKind::Shared
            ),
        ]
    );
    let returns = function
        .return_contract
        .ordinary()
        .expect("host function has an ordinary return contract");
    let return_types = returns
        .success
        .iter()
        .map(|slot| &slot.value_type)
        .collect::<Vec<_>>();
    assert_eq!(
        return_types,
        vec![&ExternalSignatureType::Abi(ExternalAbiType::Fixed(
            FixedScalar::U32
        ))]
    );
    assert_eq!(returns.error, Some(&ExternalSignatureType::BuiltinError));
}

#[test]
fn byte_and_u8_keep_distinct_identities_and_optionals_keep_their_payload() {
    let definition = BuilderDefinition::new("game")
        .host_package(
            HostPackage::new("@platform")
                .function(HostFunction::new("octet").parameter(HostType::Byte))
                .function(HostFunction::new("number").parameter(HostType::U8))
                .function(
                    HostFunction::new("maybe_name").returns(HostType::optional(HostType::String)),
                ),
        )
        .finish()
        .expect("definition should finish");
    let registry = &definition.surface().binding_packages;

    let parameter = |name: &str| {
        registry
            .resolve_package_function("@platform", name)
            .expect("function should register")
            .1
            .parameters[0]
            .language_type
            .clone()
    };
    assert_eq!(
        parameter("octet"),
        ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::Byte))
    );
    assert_eq!(
        parameter("number"),
        ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::U8))
    );

    let (_, maybe_name) = registry
        .resolve_package_function("@platform", "maybe_name")
        .expect("function should register");
    let maybe_name_returns = maybe_name
        .return_contract
        .ordinary()
        .expect("host function has an ordinary return contract");
    assert_eq!(
        maybe_name_returns.success[0].value_type,
        ExternalSignatureType::Optional(Box::new(ExternalSignatureType::Abi(
            ExternalAbiType::Utf8Str
        )))
    );
    assert_eq!(maybe_name_returns.error, None);
}

#[test]
fn finish_rejects_host_signatures_outside_the_v1_profile() {
    let cases = [
        (
            "nested optional",
            "nested",
            HostFunction::new("nested")
                .returns(HostType::optional(HostType::optional(HostType::I32))),
        ),
        (
            "two success values",
            "pair",
            HostFunction::new("pair")
                .returns(HostType::I32)
                .returns(HostType::I32),
        ),
        (
            "function name with a path separator",
            "read/file",
            HostFunction::new("read/file"),
        ),
    ];

    for (case, offender, function) in cases {
        let message = invalid_definition_message(
            case,
            BuilderDefinition::new("game")
                .host_package(HostPackage::new("@platform").function(function))
                .finish(),
        );
        assert!(message.contains(offender), "{case}: {message}");
    }
}

#[test]
fn finish_rejects_duplicate_and_overlapping_registrations() {
    let ui_dir = package_dir();
    let source = |path: &str| SourcePackage::new(path).from_dir(ui_dir.path());
    let cases = [
        (
            "duplicate source package",
            "@ui",
            BuilderDefinition::new("game")
                .source_package(source("@ui"))
                .source_package(source("@ui")),
        ),
        (
            "case-only duplicate source package",
            "@UI",
            BuilderDefinition::new("game")
                .source_package(source("@ui"))
                .source_package(source("@UI")),
        ),
        (
            "duplicate host package",
            "@platform",
            BuilderDefinition::new("game")
                .host_package(HostPackage::new("@platform"))
                .host_package(HostPackage::new("@platform")),
        ),
        (
            "case-only duplicate host package",
            "@PLATFORM",
            BuilderDefinition::new("game")
                .host_package(HostPackage::new("@platform"))
                .host_package(HostPackage::new("@PLATFORM")),
        ),
        (
            "conflicting host signatures",
            "read",
            BuilderDefinition::new("game").host_package(
                HostPackage::new("@platform")
                    .function(HostFunction::new("read").returns(HostType::String))
                    .function(HostFunction::new("read").returns(HostType::I32)),
            ),
        ),
        (
            "source and host at one path",
            "@ui",
            BuilderDefinition::new("game")
                .source_package(source("@ui"))
                .host_package(HostPackage::new("@ui")),
        ),
        (
            "host package nested in a source package",
            "@ui/native",
            BuilderDefinition::new("game")
                .source_package(source("@ui"))
                .host_package(HostPackage::new("@ui/native")),
        ),
        (
            "source package nested in a source package",
            "@ui/widgets",
            BuilderDefinition::new("game")
                .source_package(source("@ui"))
                .source_package(source("@ui/widgets")),
        ),
        (
            "source and host differing only by case",
            "@UI",
            BuilderDefinition::new("game")
                .source_package(source("@UI"))
                .host_package(HostPackage::new("@ui")),
        ),
    ];

    for (case, offender, draft) in cases {
        let message = invalid_definition_message(case, draft.finish());
        assert!(message.contains(offender), "{case}: {message}");
    }

    BuilderDefinition::new("game")
        .source_package(source("@ui"))
        .host_package(HostPackage::new("@uikit"))
        .finish()
        .expect("sibling paths sharing a prefix do not overlap");

    let definition = BuilderDefinition::new("game")
        .source_package(source("@web/site"))
        .host_package(HostPackage::new("@web/page"))
        .finish()
        .expect("nested sibling source and host packages do not overlap");
    assert!(
        definition
            .surface()
            .source_packages
            .get_root("web/site")
            .is_some()
    );
}

#[test]
fn finish_rejects_reserved_and_malformed_package_paths() {
    let dir = package_dir();
    for path in ["@project", "@Project", "@core", "@CORE/game", "ui", "@ui#x"] {
        let message = invalid_definition_message(
            &format!("source package {path}"),
            BuilderDefinition::new("game")
                .source_package(SourcePackage::new(path).from_dir(dir.path()))
                .finish(),
        );
        assert!(message.contains(path), "{message}");
        let message = invalid_definition_message(
            &format!("host package {path}"),
            BuilderDefinition::new("game")
                .host_package(HostPackage::new(path))
                .finish(),
        );
        assert!(message.contains(path), "{message}");
    }
}

#[test]
fn finish_rejects_source_packages_without_an_installed_directory() {
    let dir = package_dir();
    let missing = dir.path().join("missing");

    invalid_definition_message(
        "no directory",
        BuilderDefinition::new("game")
            .source_package(SourcePackage::new("@ui"))
            .finish(),
    );
    invalid_definition_message(
        "missing directory",
        BuilderDefinition::new("game")
            .source_package(SourcePackage::new("@ui").from_dir(&missing))
            .finish(),
    );
    for (case, root) in [
        ("relative existing directory", "."),
        ("relative missing directory", "installed-packages/ui"),
    ] {
        let message = invalid_definition_message(
            case,
            BuilderDefinition::new("game")
                .source_package(SourcePackage::new("@ui").from_dir(root))
                .finish(),
        );
        assert!(message.contains("@ui"), "{case}: {message}");
        assert!(message.contains(root), "{case}: {message}");
    }
}

#[test]
fn core_source_package_paths_reject_builder_source_and_host_packages() {
    let custom_html_dir = package_dir();

    for (case, path) in [
        ("html", "@html"),
        ("html nested", "@html/docs"),
        ("styles", "@styles"),
        ("styles nested", "@styles/docs"),
    ] {
        let message = invalid_definition_message(
            case,
            BuilderDefinition::new("game")
                .source_package(SourcePackage::new(path).from_dir(custom_html_dir.path()))
                .finish(),
        );
        assert!(
            message.contains("overlaps registered source package"),
            "{case}: {message}"
        );
        let message = invalid_definition_message(
            &format!("host {case}"),
            BuilderDefinition::new("game")
                .host_package(HostPackage::new(path))
                .finish(),
        );
        assert!(
            message.contains("overlaps host package"),
            "host {case}: {message}"
        );
    }
}

#[test]
fn minimal_sdk_builder_preserves_official_core_source_roots() {
    use crate::builder_surface::BuilderSurface;
    use crate::builder_surface::PackageMetadata;
    use crate::builder_surface::definition::CORE_SOURCE_PACKAGE_PREFIXES;
    use crate::builder_surface::source_package_registry::ProvidedSourceRoot;

    let definition = BuilderDefinition::new("game")
        .finish()
        .expect("a minimal SDK builder should finish");
    let surface = definition.surface();
    for prefix in CORE_SOURCE_PACKAGE_PREFIXES {
        let root = surface
            .source_packages
            .get_root(prefix)
            .unwrap_or_else(|| panic!("minimal SDK builder should keep official @{prefix}"));
        assert_eq!(root.metadata, PackageMetadata::source(PackageOrigin::Core));
        assert!(matches!(
            &root.root,
            ProvidedSourceRoot::Filesystem(path)
                if path == &BuilderSurface::builtin_source_package_root(prefix)
        ));
    }
    assert!(
        surface
            .binding_packages
            .get_package("@web/canvas")
            .is_none(),
        "official source roots must not load the HTML canvas binding"
    );
    assert!(surface.source_packages.get_root("web/site").is_none());
}

#[test]
fn builder_identity_must_be_a_moth_identifier() {
    for identity in ["", "game builder", "1game", "game/ui"] {
        invalid_definition_message(
            &format!("identity {identity:?}"),
            BuilderDefinition::new(identity).finish(),
        );
    }
}

#[test]
fn first_party_builder_identities_are_reserved() {
    for identity in ["html", "HTML", "Html"] {
        let message = invalid_definition_message(
            &format!("identity {identity:?}"),
            BuilderDefinition::new(identity).finish(),
        );
        assert!(message.contains("first-party"), "{identity:?}: {message}");
    }
    BuilderDefinition::new("html_site")
        .finish()
        .expect("an identity that only starts with a reserved name is valid");
}

#[test]
fn source_kinds_are_opt_in_for_markdown_and_fixed_for_templates() {
    let plain = BuilderDefinition::new("game").finish().expect("valid");
    assert_eq!(
        plain.surface().source_file_kinds.kind_for_extension("mtf"),
        Some(crate::builder_surface::SourceFileKind::MothTemplate),
        "`.mtf` is compiler-supported under a builder that selects no source kinds"
    );
    assert_eq!(
        plain.surface().source_file_kinds.kind_for_extension("md"),
        None
    );

    let documents = BuilderDefinition::new("game")
        .markdown()
        .finish()
        .expect("markdown needs no template prelude");
    assert_eq!(
        documents
            .surface()
            .source_file_kinds
            .kind_for_extension("mtf"),
        Some(crate::builder_surface::SourceFileKind::MothTemplate)
    );
    assert_eq!(
        documents
            .surface()
            .source_file_kinds
            .kind_for_extension("md"),
        Some(crate::builder_surface::SourceFileKind::PlainMarkdown)
    );
}
