use super::super::file_reference_resolution::{
    FileReferenceResolver, SingleFileReferenceOutcome, SingleFileReferenceResolver,
    stable_case_mismatch,
};
use super::super::resource_inputs::ResourceInputRegistry;
use super::super::source_tree_index::{SourceTreeIndex, SourceTreeProjectContext};
use crate::builder_surface::SourcePackageRegistry;
use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
use crate::builder_surface::package_metadata::PackageSetMembership;
use crate::builder_surface::{PackageOrigin, SourceFileKindRegistry};
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::{
    InvalidCompileTimePathCode, InvalidCompileTimePathView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_resolved_failure_for_test;
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReference, PreparedFileReferenceClass, ResolvedFileReferenceFailure,
    ResolvedFileReferenceOutcome, ResolvedFileReferenceTarget,
};
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::{LocalSpan, SourceDatabase, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::Config;

use std::fs;

fn invalid_path_reason(
    failure: ResolvedFileReferenceFailure,
) -> Option<InvalidCompileTimePathCode> {
    let compact = compact_resolved_failure_for_test(failure);
    compact
        .views()
        .next()?
        .family_view::<InvalidCompileTimePathView>()
        .map(|facts| facts.reason)
}

fn has_invalid_path_reason(
    outcome: SingleFileReferenceOutcome,
    expected: InvalidCompileTimePathCode,
) -> bool {
    match outcome {
        SingleFileReferenceOutcome::Diagnostic(diagnostic) => {
            invalid_path_reason(diagnostic) == Some(expected)
        }
        _ => false,
    }
}

#[cfg(unix)]
fn has_resolved_outcome_reason(
    outcome: ResolvedFileReferenceOutcome,
    expected: InvalidCompileTimePathCode,
) -> bool {
    match outcome {
        ResolvedFileReferenceOutcome::Diagnostic(diagnostic) => {
            invalid_path_reason(diagnostic) == Some(expected)
        }
        _ => false,
    }
}

#[test]
fn case_mismatch_evidence_is_sorted_and_unicode_lowercased() {
    let mut ambiguous = vec!["data".to_owned(), "Data".to_owned()];
    assert_eq!(
        stable_case_mismatch("DATA", &mut ambiguous),
        Some("Data".to_owned())
    );

    let mut unicode = vec!["Éclair".to_owned()];
    assert_eq!(
        stable_case_mismatch("éCLAIR", &mut unicode),
        Some("Éclair".to_owned())
    );
}

#[test]
fn synthetic_physical_resolution_cache_reuses_settled_outcome() {
    let temp = tempfile::tempdir().expect("test root should be created");
    let resource_directory = temp.path().join("assets");
    fs::create_dir_all(&resource_directory).expect("resource directory should be created");
    let resource_path = resource_directory.join("logo.svg");
    fs::write(&resource_path, "resource").expect("resource should be created");

    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let mut resource_inputs = ResourceInputRegistry::new();
    let canonical_root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
    let mut resolver = SingleFileReferenceResolver::new(
        canonical_root.clone(),
        &source_file_kinds,
        &mut resource_inputs,
    );
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let path = path_fork
        .try_intern_portable_path("assets/logo.svg", &mut strings)
        .expect("test path fits");
    let path_syntax_id = path_syntax
        .try_push_for_source(path, SourceId::COMPILATION_ROOT, LocalSpan::source_start())
        .expect("test path syntax row should fit");
    let reference = PreparedFileReference {
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        source_file: SourceId::COMPILATION_ROOT,
        path_syntax: path_syntax_id,
        class: PreparedFileReferenceClass::ResourceFile,
    };

    let owner = canonical_root.join("main.moth");
    let first = resolver
        .resolve(&owner, &path_syntax, &reference, &mut strings, &path_fork)
        .expect("first resource reference should resolve");
    let second = resolver
        .resolve(&owner, &path_syntax, &reference, &mut strings, &path_fork)
        .expect("repeated resource reference should resolve");

    assert!(
        matches!(first.outcome, SingleFileReferenceOutcome::Resource { .. }),
        "first outcome: {:?}",
        first.outcome
    );
    assert!(matches!(
        second.outcome,
        SingleFileReferenceOutcome::Resource { .. }
    ));
    assert_eq!(resource_inputs.records().len(), 1);
}

#[test]
fn synthetic_moth_value_skips_physical_resolution() {
    // `@"@config.moth"` is classified `SourceKindNoFileValue` from its `.moth` suffix like any
    // other source value. AST owns that class and rejects it with `MothFileHasNoValue`
    // (`MOTH-SYNTAX-0023`), so Stage 0 must not create a resource record or a watch interest
    // for the config-file spelling.
    let temp = tempfile::tempdir().expect("test root should be created");
    let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut resolver =
        SingleFileReferenceResolver::new(root.clone(), &source_file_kinds, &mut resource_inputs);
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let owner = root.join("main.moth");

    for authored in ["missing.moth", "@config.moth"] {
        let path = path_fork
            .try_intern_portable_path(authored, &mut strings)
            .expect("test path fits");
        let path_syntax_id = path_syntax
            .try_push_for_source(path, SourceId::COMPILATION_ROOT, LocalSpan::source_start())
            .expect("test path syntax row should fit");
        let reference = PreparedFileReference {
            span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
            source_file: SourceId::COMPILATION_ROOT,
            path_syntax: path_syntax_id,
            class: PreparedFileReferenceClass::SourceKindNoFileValue,
        };

        let resolved = resolver
            .resolve(&owner, &path_syntax, &reference, &mut strings, &path_fork)
            .expect("a .moth value should be identified without a physical target");

        assert!(
            matches!(
                resolved.outcome,
                SingleFileReferenceOutcome::IdentifiedSourceKind
            ),
            "{authored} outcome: {:?}",
            resolved.outcome
        );
    }
    assert!(resource_inputs.records().is_empty());
    assert!(resource_inputs.missing_watch_interests().is_empty());
}

#[test]
fn synthetic_config_named_directory_resolves_resources_and_content() {
    let temp = tempfile::tempdir().expect("test root should be created");
    let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
    let config_directory = root.join("@config.moth");
    fs::create_dir_all(&config_directory).expect("config-named directory should be created");
    fs::write(config_directory.join("logo.svg"), "resource")
        .expect("resource inside the config-named directory should be created");
    fs::write(config_directory.join("notes.mtf"), "# Notes")
        .expect("content inside the config-named directory should be created");

    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut resolver =
        SingleFileReferenceResolver::new(root.clone(), &source_file_kinds, &mut resource_inputs);
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let owner = root.join("main.moth");

    let mut resolve = |authored: &str, class: PreparedFileReferenceClass| {
        let path = path_fork
            .try_intern_portable_path(authored, &mut strings)
            .expect("test path fits");
        let path_syntax_id = path_syntax
            .try_push_for_source(path, SourceId::COMPILATION_ROOT, LocalSpan::source_start())
            .expect("test path syntax row should fit");
        let reference = PreparedFileReference {
            span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
            source_file: SourceId::COMPILATION_ROOT,
            path_syntax: path_syntax_id,
            class,
        };
        resolver
            .resolve(&owner, &path_syntax, &reference, &mut strings, &path_fork)
            .expect("config-directory references should stay a typed outcome")
    };

    assert!(matches!(
        resolve(
            "@config.moth/logo.svg",
            PreparedFileReferenceClass::ResourceFile
        )
        .outcome,
        SingleFileReferenceOutcome::Resource { .. }
    ));
    assert!(matches!(
        resolve("@config.moth/notes.mtf", PreparedFileReferenceClass::ContentSource).outcome,
        SingleFileReferenceOutcome::Source { canonical } if canonical == config_directory.join("notes.mtf")
    ));
    assert!(has_invalid_path_reason(
        resolve(
            "@config.moth/missing.svg",
            PreparedFileReferenceClass::ResourceFile
        )
        .outcome,
        InvalidCompileTimePathCode::MissingTarget,
    ));
    assert_eq!(resource_inputs.records().len(), 1);
    assert_eq!(resource_inputs.missing_watch_interests().len(), 1);
}

#[test]
fn directory_resolution_accepts_targets_inside_a_config_named_directory() {
    let temp = tempfile::tempdir().expect("test root should be created");
    let root = temp.path().to_path_buf();
    let src = root.join("src");
    let config_directory = src.join("@config.moth");
    fs::create_dir_all(&config_directory).expect("config-named directory should be created");
    fs::write(src.join("@entry.moth"), "").expect("entry root should be created");
    fs::write(config_directory.join("logo.png"), "not source")
        .expect("resource file should be created");
    #[cfg(unix)]
    std::os::unix::fs::symlink("@config.moth", src.join("alias"))
        .expect("the config-named directory should also be reachable under an alias");

    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_src = fs::canonicalize(&src).expect("entry root should canonicalize");
    let mut discovery_strings = StringTable::new();
    let mut config = Config::new(canonical_root.clone());
    config.project_name = String::from("config_directory_fixture");
    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let source_packages = SourcePackageRegistry::default();
    let external_import_providers = ExternalImportProviderRegistry::default();
    let index = SourceTreeIndex::discover(
        canonical_src.clone(),
        SourceTreeProjectContext {
            validated_output_settings: None,
            config_source: None,
        },
        &config,
        &source_packages,
        &source_file_kinds,
        &external_import_providers,
        &mut discovery_strings,
    )
    .expect("source tree index should build");
    let consumer_module = index
        .module_identities()
        .module_id_for_directory(&canonical_src)
        .expect("the selected entry root should be a module");

    let mut resource_inputs = ResourceInputRegistry::new();
    let source_files = SourceDatabase::empty();
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let mut discovered_content_sources = Vec::new();

    let mut resolve = |authored: &str, class: PreparedFileReferenceClass| {
        let path = path_fork
            .try_intern_portable_path(authored, &mut strings)
            .expect("test path fits");
        let path_syntax_id = path_syntax
            .try_push_for_source(path, SourceId::COMPILATION_ROOT, LocalSpan::source_start())
            .expect("test path syntax row should fit");
        let reference = PreparedFileReference {
            span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
            source_file: SourceId::COMPILATION_ROOT,
            path_syntax: path_syntax_id,
            class,
        };
        FileReferenceResolver::new(&index, &mut resource_inputs)
            .resolve(
                consumer_module,
                &path_syntax,
                &reference,
                &source_files,
                &mut strings,
                &path_fork,
                &mut discovered_content_sources,
            )
            .expect("config-directory references should stay a typed outcome")
    };

    let resource = resolve(
        "@config.moth/logo.png",
        PreparedFileReferenceClass::ResourceFile,
    );
    let ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::ResourceSource {
        source: resource_source,
        ..
    }) = resource.outcome
    else {
        panic!("expected a resource target, got {:?}", resource.outcome);
    };
    #[cfg(unix)]
    {
        let alias_resource = resolve("alias/logo.png", PreparedFileReferenceClass::ResourceFile);
        assert!(matches!(
            alias_resource.outcome,
            ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::ResourceSource {
                source, ..
            }) if source == resource_source
        ));
    }
    assert_eq!(
        resource_inputs
            .read_source(resource_source)
            .expect("resource should be readable"),
        b"not source",
    );
    assert_eq!(resource_inputs.records().len(), 1);
    assert!(discovered_content_sources.is_empty());
}

#[cfg(unix)]
#[test]
fn directory_physical_aliases_to_config_file_are_rejected_before_dispatch() {
    // Discovery omits the canonical config target, so dispatch would report an internal failure
    // for content and publish the config file as a resource. The shared physical resolver must
    // reject the canonical name first.
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("test root should be created");
    let root = temp.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("source root should be created");
    fs::write(src.join("@entry.moth"), "").expect("entry root should be created");
    fs::write(src.join("@config.moth"), "value #= 1").expect("config should be created");
    symlink("@config.moth", src.join("alias.md")).expect("content alias should be created");
    symlink("@config.moth", src.join("alias.svg")).expect("resource alias should be created");

    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_src = fs::canonicalize(&src).expect("entry root should canonicalize");
    let mut discovery_strings = StringTable::new();
    let mut config = Config::new(canonical_root.clone());
    config.project_name = String::from("config_file_alias_fixture");
    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let source_packages = SourcePackageRegistry::default();
    let external_import_providers = ExternalImportProviderRegistry::default();
    let index = SourceTreeIndex::discover(
        canonical_src.clone(),
        SourceTreeProjectContext {
            validated_output_settings: None,
            config_source: None,
        },
        &config,
        &source_packages,
        &source_file_kinds,
        &external_import_providers,
        &mut discovery_strings,
    )
    .expect("source tree index should build");
    let consumer_module = index
        .module_identities()
        .module_id_for_directory(&canonical_src)
        .expect("the selected entry root should be a module");

    let mut resource_inputs = ResourceInputRegistry::new();
    let source_files = SourceDatabase::empty();
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let mut discovered_content_sources = Vec::new();

    let mut resolve = |authored: &str, class: PreparedFileReferenceClass| {
        let path = path_fork
            .try_intern_portable_path(authored, &mut strings)
            .expect("test path fits");
        let path_syntax_id = path_syntax
            .try_push_for_source(path, SourceId::COMPILATION_ROOT, LocalSpan::source_start())
            .expect("test path syntax row should fit");
        let reference = PreparedFileReference {
            span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
            source_file: SourceId::COMPILATION_ROOT,
            path_syntax: path_syntax_id,
            class,
        };
        FileReferenceResolver::new(&index, &mut resource_inputs)
            .resolve(
                consumer_module,
                &path_syntax,
                &reference,
                &source_files,
                &mut strings,
                &path_fork,
                &mut discovered_content_sources,
            )
            .expect("a config-file alias should stay a typed outcome")
    };

    for (authored, class) in [
        ("alias.md", PreparedFileReferenceClass::ContentSource),
        ("alias.svg", PreparedFileReferenceClass::ResourceFile),
    ] {
        assert!(
            has_resolved_outcome_reason(
                resolve(authored, class).outcome,
                InvalidCompileTimePathCode::ReservedConfigFile
            ),
            "{authored} must not dispatch to its authored extension"
        );
    }

    assert!(resource_inputs.records().is_empty());
    assert!(resource_inputs.missing_watch_interests().is_empty());
    assert!(discovered_content_sources.is_empty());
}

#[cfg(unix)]
#[test]
fn directory_content_aliases_to_moth_sources_are_rejected_before_admission() {
    // A visible alias can give a Moth source an authored `.mtf` spelling. Admitting it as content
    // would publish a `content` lookup against the root file's own declarations, so the shared
    // physical resolver must reject the kind mismatch before any target or worklist entry exists.
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("test root should be created");
    let root = temp.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("source root should be created");
    fs::write(src.join("@entry.moth"), "content #= \"private\"\n")
        .expect("entry root should be created");
    symlink("@entry.moth", src.join("alias.mtf")).expect("content alias should be created");

    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_src = fs::canonicalize(&src).expect("entry root should canonicalize");
    let mut discovery_strings = StringTable::new();
    let mut config = Config::new(canonical_root);
    config.project_name = String::from("moth_content_alias_fixture");
    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let source_packages = SourcePackageRegistry::default();
    let external_import_providers = ExternalImportProviderRegistry::default();
    let index = SourceTreeIndex::discover(
        canonical_src.clone(),
        SourceTreeProjectContext {
            validated_output_settings: None,
            config_source: None,
        },
        &config,
        &source_packages,
        &source_file_kinds,
        &external_import_providers,
        &mut discovery_strings,
    )
    .expect("source tree index should build");
    let consumer_module = index
        .module_identities()
        .module_id_for_directory(&canonical_src)
        .expect("the selected entry root should be a module");

    let mut resource_inputs = ResourceInputRegistry::new();
    let source_files = SourceDatabase::empty();
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let path = path_fork
        .try_intern_portable_path("alias.mtf", &mut strings)
        .expect("test path fits");
    let path_syntax_id = path_syntax
        .try_push_for_source(path, SourceId::COMPILATION_ROOT, LocalSpan::source_start())
        .expect("test path syntax row should fit");
    let reference = PreparedFileReference {
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        source_file: SourceId::COMPILATION_ROOT,
        path_syntax: path_syntax_id,
        class: PreparedFileReferenceClass::ContentSource,
    };
    let mut discovered_content_sources = Vec::new();
    let resolved = FileReferenceResolver::new(&index, &mut resource_inputs)
        .resolve(
            consumer_module,
            &path_syntax,
            &reference,
            &source_files,
            &mut strings,
            &path_fork,
            &mut discovered_content_sources,
        )
        .expect("a Moth content alias should stay a typed outcome");

    assert!(
        matches!(
            resolved.outcome,
            ResolvedFileReferenceOutcome::Diagnostic(
                ResolvedFileReferenceFailure::MothTargetHasNoValue { .. }
            )
        ),
        "a `.mtf` alias to a Moth source must report the Moth-file value diagnostic, got {:?}",
        resolved.outcome
    );
    assert!(
        discovered_content_sources.is_empty(),
        "a rejected kind mismatch must not enter the content worklist"
    );
    assert!(resource_inputs.records().is_empty());
}

#[cfg(unix)]
#[test]
fn conflicting_content_aliases_follow_the_physical_source_kind() {
    // Two visible spellings can resolve to one physical file with different extensions. The
    // retained record must follow the recognised physical entry, not whichever alias sorts
    // first, so a template helper stays content and a Moth helper stays a non-content source.
    use super::super::source_tree_index::SourceClassification;
    use crate::builder_surface::SourceFileKind;
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("test root should be created");
    let root = temp.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("source root should be created");
    fs::write(src.join("@entry.moth"), "value = 1\n").expect("entry root should be created");
    fs::write(src.join("helper.moth"), "helper #= 1\n").expect("Moth helper should be created");
    fs::write(src.join("template.mtf"), "[:template]\n")
        .expect("template helper should be created");
    symlink("helper.moth", src.join("alias.mtf")).expect("Moth content alias should be created");
    symlink("template.mtf", src.join("alias.moth")).expect("template Moth alias should be created");

    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_src = fs::canonicalize(&src).expect("entry root should canonicalize");
    let mut discovery_strings = StringTable::new();
    let mut config = Config::new(canonical_root);
    config.project_name = String::from("moth_conflicting_alias_fixture");
    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let source_packages = SourcePackageRegistry::default();
    let external_import_providers = ExternalImportProviderRegistry::default();
    let index = SourceTreeIndex::discover(
        canonical_src.clone(),
        SourceTreeProjectContext {
            validated_output_settings: None,
            config_source: None,
        },
        &config,
        &source_packages,
        &source_file_kinds,
        &external_import_providers,
        &mut discovery_strings,
    )
    .expect("source tree index should build");
    let consumer_module = index
        .module_identities()
        .module_id_for_directory(&canonical_src)
        .expect("the selected entry root should be a module");

    let canonical_helper = fs::canonicalize(src.join("helper.moth")).expect("helper should exist");
    let canonical_template =
        fs::canonicalize(src.join("template.mtf")).expect("template should exist");
    let helper_index = index
        .source_index_for_canonical_path(&canonical_helper)
        .expect("the Moth helper should be indexed");
    assert!(
        matches!(
            index.source(helper_index).classification(),
            SourceClassification::CompilerSemantic(SourceFileKind::Moth)
        ),
        "the physical `.moth` entry must decide the retained kind"
    );
    let template_index = index
        .source_index_for_canonical_path(&canonical_template)
        .expect("the template helper should be indexed");
    assert!(
        matches!(
            index.source(template_index).classification(),
            SourceClassification::CompilerSemantic(SourceFileKind::MothTemplate)
        ),
        "a visible `.moth` alias must not disguise the physical `.mtf` entry"
    );

    let mut resource_inputs = ResourceInputRegistry::new();
    let source_files = SourceDatabase::empty();
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let path = path_fork
        .try_intern_portable_path("alias.mtf", &mut strings)
        .expect("test path fits");
    let path_syntax_id = path_syntax
        .try_push_for_source(path, SourceId::COMPILATION_ROOT, LocalSpan::source_start())
        .expect("test path syntax row should fit");
    let reference = PreparedFileReference {
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        source_file: SourceId::COMPILATION_ROOT,
        path_syntax: path_syntax_id,
        class: PreparedFileReferenceClass::ContentSource,
    };
    let mut discovered_content_sources = Vec::new();
    let resolved = FileReferenceResolver::new(&index, &mut resource_inputs)
        .resolve(
            consumer_module,
            &path_syntax,
            &reference,
            &source_files,
            &mut strings,
            &path_fork,
            &mut discovered_content_sources,
        )
        .expect("a conflicting-kind alias should stay a typed outcome");

    let ResolvedFileReferenceOutcome::Diagnostic(failure) = resolved.outcome else {
        panic!(
            "the physical Moth helper must not be admitted as content, got {:?}",
            resolved.outcome
        );
    };
    assert!(matches!(
        failure,
        ResolvedFileReferenceFailure::MothTargetHasNoValue { .. }
    ));
    assert_eq!(
        compact_resolved_failure_for_test(failure)
            .views()
            .next()
            .and_then(|view| view.reason_key()),
        Some("invalid_expression.moth_file_has_no_value")
    );
    assert!(discovered_content_sources.is_empty());
    assert!(resource_inputs.records().is_empty());
}

#[cfg(unix)]
#[test]
fn physical_root_surfaces_keep_their_authored_moth_kind() {
    // A symlinked root surface has a differently named canonical target. The authored root role
    // must keep the record's Moth kind so a direct content reference to the physical target is
    // still rejected as a Moth file.
    use super::super::source_tree_index::SourceClassification;
    use crate::builder_surface::SourceFileKind;
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("test root should be created");
    let root = temp.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("source root should be created");
    fs::write(src.join("entry.mtf"), "[:entry]\n").expect("entry target should be created");
    symlink("entry.mtf", src.join("@entry.moth")).expect("entry root should be created");

    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_src = fs::canonicalize(&src).expect("entry root should canonicalize");
    let mut discovery_strings = StringTable::new();
    let mut config = Config::new(canonical_root);
    config.project_name = String::from("moth_root_alias_fixture");
    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let source_packages = SourcePackageRegistry::default();
    let external_import_providers = ExternalImportProviderRegistry::default();
    let index = SourceTreeIndex::discover(
        canonical_src.clone(),
        SourceTreeProjectContext {
            validated_output_settings: None,
            config_source: None,
        },
        &config,
        &source_packages,
        &source_file_kinds,
        &external_import_providers,
        &mut discovery_strings,
    )
    .expect("source tree index should build");
    let consumer_module = index
        .module_identities()
        .module_id_for_directory(&canonical_src)
        .expect("the selected entry root should be a module");

    let canonical_entry = fs::canonicalize(src.join("entry.mtf")).expect("entry should exist");
    let entry_index = index
        .source_index_for_canonical_path(&canonical_entry)
        .expect("the entry surface should be indexed");
    let entry_record = index.source(entry_index);
    assert!(
        matches!(
            entry_record.classification(),
            SourceClassification::CompilerSemantic(SourceFileKind::Moth)
        ),
        "the authored root role must keep the Moth kind"
    );

    let mut resource_inputs = ResourceInputRegistry::new();
    let source_files = SourceDatabase::empty();
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let path = path_fork
        .try_intern_portable_path("entry.mtf", &mut strings)
        .expect("test path fits");
    let path_syntax_id = path_syntax
        .try_push_for_source(path, SourceId::COMPILATION_ROOT, LocalSpan::source_start())
        .expect("test path syntax row should fit");
    let reference = PreparedFileReference {
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        source_file: SourceId::COMPILATION_ROOT,
        path_syntax: path_syntax_id,
        class: PreparedFileReferenceClass::ContentSource,
    };
    let mut discovered_content_sources = Vec::new();
    let resolved = FileReferenceResolver::new(&index, &mut resource_inputs)
        .resolve(
            consumer_module,
            &path_syntax,
            &reference,
            &source_files,
            &mut strings,
            &path_fork,
            &mut discovered_content_sources,
        )
        .expect("a root-surface alias should stay a typed outcome");

    assert!(
        matches!(
            resolved.outcome,
            ResolvedFileReferenceOutcome::Diagnostic(
                ResolvedFileReferenceFailure::MothTargetHasNoValue { .. }
            )
        ),
        "the root surface must keep Moth semantics, got {:?}",
        resolved.outcome
    );
    assert!(discovered_content_sources.is_empty());
}

#[cfg(unix)]
#[test]
fn directory_rejects_content_aliases_to_provider_owned_files() {
    // A content-spelled alias can resolve to an ordinary provider-owned input. Reaching the index
    // invariant with an InfrastructureFailure would blame the compiler for authored source; the
    // resolver must instead publish a typed user diagnostic before admission.
    use std::os::unix::fs::symlink;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    let temp = tempfile::tempdir().expect("test root should be created");
    let root = temp.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("source root should be created");
    fs::write(src.join("@entry.moth"), "value = 1\n").expect("entry root should be created");
    fs::write(src.join("drawing.js"), "export const drawing = 1;\n")
        .expect("provider source should be created");
    symlink("drawing.js", src.join("alias.mtf")).expect("provider content alias should be created");

    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_src = fs::canonicalize(&src).expect("entry root should canonicalize");
    let mut discovery_strings = StringTable::new();
    let mut config = Config::new(canonical_root);
    config.project_name = String::from("moth_provider_alias_fixture");
    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let source_packages = SourcePackageRegistry::default();
    let mut external_import_providers = ExternalImportProviderRegistry::default();
    external_import_providers.register(Arc::new(super::ResolvingCountingProvider::new(Arc::new(
        AtomicUsize::new(0),
    ))));
    let index = SourceTreeIndex::discover(
        canonical_src.clone(),
        SourceTreeProjectContext {
            validated_output_settings: None,
            config_source: None,
        },
        &config,
        &source_packages,
        &source_file_kinds,
        &external_import_providers,
        &mut discovery_strings,
    )
    .expect("source tree index should build");
    let consumer_module = index
        .module_identities()
        .module_id_for_directory(&canonical_src)
        .expect("the selected entry root should be a module");

    let canonical_drawing =
        fs::canonicalize(src.join("drawing.js")).expect("the provider source should exist");
    let drawing_index = index
        .source_index_for_canonical_path(&canonical_drawing)
        .expect("the provider source should be indexed");
    assert!(
        matches!(
            index.source(drawing_index).classification(),
            super::super::source_tree_index::SourceClassification::ProviderOwned(_)
        ),
        "the physical provider entry must keep provider ownership"
    );

    let mut resource_inputs = ResourceInputRegistry::new();
    let source_files = SourceDatabase::empty();
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let path = path_fork
        .try_intern_portable_path("alias.mtf", &mut strings)
        .expect("test path fits");
    let path_syntax_id = path_syntax
        .try_push_for_source(path, SourceId::COMPILATION_ROOT, LocalSpan::source_start())
        .expect("test path syntax row should fit");
    let reference = PreparedFileReference {
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        source_file: SourceId::COMPILATION_ROOT,
        path_syntax: path_syntax_id,
        class: PreparedFileReferenceClass::ContentSource,
    };
    let mut discovered_content_sources = Vec::new();
    let resolved = FileReferenceResolver::new(&index, &mut resource_inputs)
        .resolve(
            consumer_module,
            &path_syntax,
            &reference,
            &source_files,
            &mut strings,
            &path_fork,
            &mut discovered_content_sources,
        )
        .expect("a provider-owned alias should stay a typed outcome");

    let ResolvedFileReferenceOutcome::Diagnostic(failure) = resolved.outcome else {
        panic!(
            "a provider-owned target must not resolve as content, got {:?}",
            resolved.outcome
        );
    };
    assert_eq!(
        invalid_path_reason(failure),
        Some(InvalidCompileTimePathCode::ProviderOwnedContentTarget)
    );
    assert_eq!(
        compact_resolved_failure_for_test(failure)
            .views()
            .next()
            .and_then(|view| view.reason_key()),
        Some("invalid_compile_time_path.provider_owned_content_target")
    );
    assert!(discovered_content_sources.is_empty());
    assert!(resource_inputs.records().is_empty());
}

#[cfg(unix)]
#[test]
fn synthetic_content_alias_to_a_moth_helper_is_rejected() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("test root should be created");
    let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
    fs::write(root.join("helper.moth"), "helper #= 1\n").expect("Moth helper should be created");
    symlink("helper.moth", root.join("alias.mtf")).expect("content alias should be created");

    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut resolver =
        SingleFileReferenceResolver::new(root.clone(), &source_file_kinds, &mut resource_inputs);
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let owner = root.join("main.moth");
    let path = path_fork
        .try_intern_portable_path("alias.mtf", &mut strings)
        .expect("test path fits");
    let path_syntax_id = path_syntax
        .try_push_for_source(path, SourceId::COMPILATION_ROOT, LocalSpan::source_start())
        .expect("test path syntax row should fit");
    let reference = PreparedFileReference {
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        source_file: SourceId::COMPILATION_ROOT,
        path_syntax: path_syntax_id,
        class: PreparedFileReferenceClass::ContentSource,
    };

    let resolved = resolver
        .resolve(&owner, &path_syntax, &reference, &mut strings, &path_fork)
        .expect("a Moth alias should stay a typed outcome");
    let SingleFileReferenceOutcome::Diagnostic(failure) = resolved.outcome else {
        panic!(
            "a `.mtf` alias to a Moth helper must not resolve as content, got {:?}",
            resolved.outcome
        );
    };
    assert!(matches!(
        failure,
        ResolvedFileReferenceFailure::MothTargetHasNoValue { .. }
    ));
    assert_eq!(
        compact_resolved_failure_for_test(failure)
            .views()
            .next()
            .and_then(|view| view.reason_key()),
        Some("invalid_expression.moth_file_has_no_value")
    );
    assert!(resource_inputs.records().is_empty());
    assert!(resource_inputs.missing_watch_interests().is_empty());
}

#[cfg(unix)]
#[test]
fn synthetic_content_alias_to_a_provider_owned_file_stays_typed() {
    // Single-file resolution has no provider registry, so it classifies the physical `.js` entry
    // as an unsupported content kind rather than reaching the directory index invariant. Keep that
    // user-facing outcome typed, with no resource record or watch interest.
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("test root should be created");
    let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
    fs::write(root.join("drawing.js"), "export const drawing = 1;\n")
        .expect("provider source should be created");
    symlink("drawing.js", root.join("alias.mtf")).expect("content alias should be created");

    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut resolver =
        SingleFileReferenceResolver::new(root.clone(), &source_file_kinds, &mut resource_inputs);
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let owner = root.join("main.moth");
    let path = path_fork
        .try_intern_portable_path("alias.mtf", &mut strings)
        .expect("test path fits");
    let path_syntax_id = path_syntax
        .try_push_for_source(path, SourceId::COMPILATION_ROOT, LocalSpan::source_start())
        .expect("test path syntax row should fit");
    let reference = PreparedFileReference {
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        source_file: SourceId::COMPILATION_ROOT,
        path_syntax: path_syntax_id,
        class: PreparedFileReferenceClass::ContentSource,
    };

    let resolved = resolver
        .resolve(&owner, &path_syntax, &reference, &mut strings, &path_fork)
        .expect("a provider-owned alias should stay a typed outcome");
    let SingleFileReferenceOutcome::Diagnostic(
        ResolvedFileReferenceFailure::UnsupportedSourceFileKind { extension, .. },
    ) = resolved.outcome
    else {
        panic!(
            "a provider-owned target must not resolve as content, got {:?}",
            resolved.outcome
        );
    };
    assert_eq!(strings.resolve(extension), "js");
    assert!(resource_inputs.records().is_empty());
    assert!(resource_inputs.missing_watch_interests().is_empty());
}

#[cfg(unix)]
#[test]
fn synthetic_physical_aliases_to_config_file_are_rejected_before_dispatch() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("test root should be created");
    let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
    fs::write(root.join("@config.moth"), "value #= 1").expect("config should be created");
    symlink("@config.moth", root.join("alias.md")).expect("content alias should be created");
    symlink("@config.moth", root.join("alias.svg")).expect("resource alias should be created");
    // A dangling alias whose terminal physical spelling is the config file must not publish a
    // missing-target watch either.
    fs::create_dir_all(root.join("other")).expect("other directory should be created");
    symlink("other/@config.moth", root.join("alias-missing.md"))
        .expect("dangling content alias should be created");

    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut resolver =
        SingleFileReferenceResolver::new(root.clone(), &source_file_kinds, &mut resource_inputs);
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let owner = root.join("main.moth");

    let mut resolve = |authored: &str, class: PreparedFileReferenceClass| {
        let path = path_fork
            .try_intern_portable_path(authored, &mut strings)
            .expect("test path fits");
        let path_syntax_id = path_syntax
            .try_push_for_source(path, SourceId::COMPILATION_ROOT, LocalSpan::source_start())
            .expect("test path syntax row should fit");
        let reference = PreparedFileReference {
            span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
            source_file: SourceId::COMPILATION_ROOT,
            path_syntax: path_syntax_id,
            class,
        };
        resolver
            .resolve(&owner, &path_syntax, &reference, &mut strings, &path_fork)
            .expect("a config-file alias should stay a typed outcome")
    };

    for (authored, class) in [
        ("alias.md", PreparedFileReferenceClass::ContentSource),
        ("alias.svg", PreparedFileReferenceClass::ResourceFile),
        (
            "alias-missing.md",
            PreparedFileReferenceClass::ContentSource,
        ),
    ] {
        assert!(
            has_invalid_path_reason(
                resolve(authored, class).outcome,
                InvalidCompileTimePathCode::ReservedConfigFile
            ),
            "{authored} must not dispatch to its authored extension"
        );
    }

    assert!(resource_inputs.records().is_empty());
    assert!(resource_inputs.missing_watch_interests().is_empty());
}

#[test]
fn package_boundary_resources_inherit_registration_provenance() {
    for (membership, origin, prefix, expects_damaged_set) in [
        (
            PackageSetMembership::CompilerShipped,
            PackageOrigin::Core,
            "core/html",
            true,
        ),
        (
            PackageSetMembership::Ordinary,
            PackageOrigin::ProjectLocal,
            "fixture",
            false,
        ),
    ] {
        let temp = tempfile::tempdir().expect("test root should be created");
        let package_root = temp.path().join("pkg");
        fs::create_dir_all(&package_root).expect("package root should be created");
        fs::write(package_root.join("@package.moth"), "$export\nvalue #= 1\n")
            .expect("support facade should be created");
        let resource_path = package_root.join("logo.svg");
        fs::write(&resource_path, "resource").expect("resource should be created");

        let canonical_root =
            fs::canonicalize(&package_root).expect("package root should canonicalize");
        let mut discovery_strings = StringTable::new();
        let index = SourceTreeIndex::discover_package(
            canonical_root.clone(),
            StablePackageIdentity::source_package(origin, prefix),
            prefix,
            membership,
            &SourceFileKindRegistry::new(),
            &ExternalImportProviderRegistry::default(),
            &mut discovery_strings,
        )
        .expect("package boundary index should build");
        let consumer_module = index
            .module_identities()
            .module_id_for_directory(&canonical_root)
            .expect("the package root should own a module");

        let mut resource_inputs = ResourceInputRegistry::new();
        let source_files = SourceDatabase::empty();
        let mut strings = StringTable::new();
        let mut path_fork = PathInternerFork::empty();
        let mut path_syntax = PathSyntaxTable::new();
        let mut discovered_content_sources = Vec::new();
        let path = path_fork
            .try_intern_portable_path("logo.svg", &mut strings)
            .expect("test path fits");
        let path_syntax_id = path_syntax
            .try_push_for_source(path, SourceId::COMPILATION_ROOT, LocalSpan::source_start())
            .expect("test path syntax row should fit");
        let reference = PreparedFileReference {
            span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
            source_file: SourceId::COMPILATION_ROOT,
            path_syntax: path_syntax_id,
            class: PreparedFileReferenceClass::ResourceFile,
        };
        let resolved = FileReferenceResolver::new(&index, &mut resource_inputs)
            .resolve(
                consumer_module,
                &path_syntax,
                &reference,
                &source_files,
                &mut strings,
                &path_fork,
                &mut discovered_content_sources,
            )
            .expect("a package resource should resolve");
        let ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::ResourceSource {
            source,
            ..
        }) = resolved.outcome
        else {
            panic!(
                "expected a package resource target, got {:?}",
                resolved.outcome
            );
        };

        fs::remove_file(&resource_path)
            .expect("the resource should be removable before the deferred read");
        let failure = resource_inputs
            .hash_source(source)
            .expect_err("the deferred resource read should fail");
        assert_eq!(
            failure.message.contains("incomplete"),
            expects_damaged_set,
            "resource provenance must decide the read-failure lane: {}",
            failure.message
        );
    }
}

#[test]
fn synthetic_not_a_directory_is_a_typed_path_failure() {
    let temp = tempfile::tempdir().expect("test root should be created");
    let blocker = temp.path().join("not_a_directory");
    fs::write(&blocker, "regular file").expect("blocker file should be created");

    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut resolver = SingleFileReferenceResolver::new(
        fs::canonicalize(temp.path()).expect("test root should canonicalize"),
        &source_file_kinds,
        &mut resource_inputs,
    );
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let path = path_fork
        .try_intern_portable_path("not_a_directory/value.mtf", &mut strings)
        .expect("test path fits");
    let path_syntax_id = path_syntax
        .try_push_for_source(path, SourceId::COMPILATION_ROOT, LocalSpan::source_start())
        .expect("test path syntax row should fit");
    let reference = PreparedFileReference {
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        source_file: SourceId::COMPILATION_ROOT,
        path_syntax: path_syntax_id,
        class: PreparedFileReferenceClass::ContentSource,
    };
    let resolved = resolver
        .resolve(
            &temp.path().join("main.moth"),
            &path_syntax,
            &reference,
            &mut strings,
            &path_fork,
        )
        .expect("NotADirectory should remain a typed path outcome");
    let SingleFileReferenceOutcome::Diagnostic(diagnostic) = resolved.outcome else {
        panic!("NotADirectory should remain a typed path diagnostic");
    };
    let compact = compact_resolved_failure_for_test(diagnostic);
    let diagnostic = compact
        .first()
        .expect("file-reference diagnostic should have a compact view");
    let primary_span = match diagnostic.primary_site() {
        SiteRef::Absent => panic!("file-reference diagnostic should retain its authored span"),
        SiteRef::Local(span) | SiteRef::Foreign { span, .. } => span,
    };
    assert_eq!(
        primary_span, reference.span,
        "synthetic file-reference diagnostics must retain the authored source span"
    );
    assert_eq!(
        diagnostic
            .family_view::<InvalidCompileTimePathView>()
            .expect("file-reference diagnostic should retain typed path facts")
            .reason,
        InvalidCompileTimePathCode::TargetNotRegular,
    );
    assert!(resource_inputs.missing_watch_interests().is_empty());
}

#[cfg(unix)]
#[test]
fn synthetic_in_owner_dangling_alias_watches_resolved_physical_prefix() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("test root should be created");
    let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
    let real = root.join("real");
    fs::create_dir_all(&real).expect("real directory should be created");
    symlink(real.join("missing-target"), root.join("alias"))
        .expect("dangling in-owner alias should be created");

    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut resolver =
        SingleFileReferenceResolver::new(root.clone(), &source_file_kinds, &mut resource_inputs);
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let path = path_fork
        .try_intern_portable_path("alias/leaf.svg", &mut strings)
        .expect("test path fits");
    let path_syntax_id = path_syntax
        .try_push_for_source(path, SourceId::COMPILATION_ROOT, LocalSpan::source_start())
        .expect("test path syntax row should fit");
    let reference = PreparedFileReference {
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        source_file: SourceId::COMPILATION_ROOT,
        path_syntax: path_syntax_id,
        class: PreparedFileReferenceClass::ResourceFile,
    };

    let owner = root.join("main.moth");
    for _ in 0..2 {
        let resolved = resolver
            .resolve(&owner, &path_syntax, &reference, &mut strings, &path_fork)
            .expect("in-owner dangling alias should settle");
        assert!(has_invalid_path_reason(
            resolved.outcome,
            InvalidCompileTimePathCode::MissingTarget,
        ));
    }

    assert_eq!(resource_inputs.missing_watch_interests().len(), 1);
    assert_eq!(
        resource_inputs.missing_watch_interests()[0].canonical_path(),
        real.join("missing-target")
    );
}

#[cfg(unix)]
#[test]
fn synthetic_dangling_alias_resolves_nested_symlink_targets_before_parent_segments() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("test root should be created");
    let outside_temp = tempfile::tempdir().expect("outside root should be created");
    let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
    let outside_root =
        fs::canonicalize(outside_temp.path()).expect("outside root should canonicalize");
    fs::create_dir_all(root.join("inside")).expect("inside directory should be created");
    symlink(&outside_root, root.join("inside/redirect"))
        .expect("nested external target should be created");
    symlink("inside/redirect/..", root.join("alias"))
        .expect("nested dangling alias should be created");

    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut resolver =
        SingleFileReferenceResolver::new(root.clone(), &source_file_kinds, &mut resource_inputs);
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let path = path_fork
        .try_intern_portable_path("alias/missing.svg", &mut strings)
        .expect("test path fits");
    let path_syntax_id = path_syntax
        .try_push_for_source(path, SourceId::COMPILATION_ROOT, LocalSpan::source_start())
        .expect("test path syntax row should fit");
    let reference = PreparedFileReference {
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        source_file: SourceId::COMPILATION_ROOT,
        path_syntax: path_syntax_id,
        class: PreparedFileReferenceClass::ResourceFile,
    };

    let resolved = resolver
        .resolve(
            &root.join("main.moth"),
            &path_syntax,
            &reference,
            &mut strings,
            &path_fork,
        )
        .expect("nested dangling alias should settle");
    assert!(has_invalid_path_reason(
        resolved.outcome,
        InvalidCompileTimePathCode::EscapesSymlink,
    ));
    assert!(resource_inputs.missing_watch_interests().is_empty());
}

#[cfg(unix)]
#[test]
fn synthetic_dangling_symlink_cycles_and_excessive_depth_are_infrastructure_errors() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("test root should be created");
    let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
    symlink(root.join("cycle-b"), root.join("cycle-a"))
        .expect("first cycle link should be created");
    symlink(root.join("cycle-a"), root.join("cycle-b"))
        .expect("second cycle link should be created");
    for index in 0..=41 {
        let target = if index == 41 {
            root.join("missing-target")
        } else {
            root.join(format!("deep-{}", index + 1))
        };
        symlink(target, root.join(format!("deep-{index}")))
            .expect("deep symlink should be created");
    }

    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut resolver =
        SingleFileReferenceResolver::new(root.clone(), &source_file_kinds, &mut resource_inputs);
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();

    let mut resolve = |resolver: &mut SingleFileReferenceResolver<'_>,
                       path: &str,
                       strings: &mut StringTable,
                       path_syntax: &mut PathSyntaxTable| {
        let authored_path = path_fork
            .try_intern_portable_path(path, strings)
            .expect("test path fits");
        let path_syntax_id = path_syntax
            .try_push_for_source(
                authored_path,
                SourceId::COMPILATION_ROOT,
                LocalSpan::source_start(),
            )
            .expect("test path syntax row should fit");
        let reference = PreparedFileReference {
            span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
            source_file: SourceId::COMPILATION_ROOT,
            path_syntax: path_syntax_id,
            class: PreparedFileReferenceClass::ResourceFile,
        };
        resolver.resolve(
            &root.join("main.moth"),
            path_syntax,
            &reference,
            strings,
            &path_fork,
        )
    };

    assert!(
        resolve(
            &mut resolver,
            "cycle-a/leaf.svg",
            &mut strings,
            &mut path_syntax
        )
        .is_err()
    );
    assert!(
        resolve(
            &mut resolver,
            "deep-0/leaf.svg",
            &mut strings,
            &mut path_syntax
        )
        .is_err()
    );
    assert!(resource_inputs.missing_watch_interests().is_empty());
}

#[cfg(unix)]
#[test]
fn synthetic_rejects_support_and_missing_symlink_boundaries_without_watches() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("test root should be created");
    let outside_temp = tempfile::tempdir().expect("outside root should be created");
    let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
    let support_root = root.join("support");
    let child_root = root.join("child");
    let outside_root =
        fs::canonicalize(outside_temp.path()).expect("outside root should canonicalize");
    fs::create_dir_all(&support_root).expect("support root should be created");
    fs::create_dir_all(&child_root).expect("child root should be created");
    fs::write(support_root.join("@package.moth"), "support root")
        .expect("support root file should be created");
    fs::write(child_root.join("@mod.moth"), "child root")
        .expect("child root file should be created");
    fs::write(support_root.join("existing.mtf"), "template")
        .expect("support content should be created");
    fs::write(support_root.join("existing.svg"), "resource")
        .expect("support resource should be created");
    symlink(&support_root, root.join("support-alias")).expect("support alias should be created");
    symlink(&child_root, root.join("child-alias")).expect("child alias should be created");
    symlink(&outside_root, root.join("outside-alias")).expect("outside alias should be created");
    symlink(
        outside_root.join("missing-target"),
        root.join("dangling-external"),
    )
    .expect("dangling external alias should be created");
    symlink(
        child_root.join("missing-target"),
        root.join("dangling-child"),
    )
    .expect("dangling child alias should be created");
    symlink(
        support_root.join("missing-target"),
        root.join("dangling-support"),
    )
    .expect("dangling support alias should be created");

    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut resolver =
        SingleFileReferenceResolver::new(root.clone(), &source_file_kinds, &mut resource_inputs);
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();

    let mut resolve = |resolver: &mut SingleFileReferenceResolver<'_>,
                       path: &str,
                       class: PreparedFileReferenceClass,
                       strings: &mut StringTable,
                       path_syntax: &mut PathSyntaxTable| {
        let authored_path = path_fork
            .try_intern_portable_path(path, strings)
            .expect("test path fits");
        let path_syntax_id = path_syntax
            .try_push_for_source(
                authored_path,
                SourceId::COMPILATION_ROOT,
                LocalSpan::source_start(),
            )
            .expect("test path syntax row should fit");
        let reference = PreparedFileReference {
            span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
            source_file: SourceId::COMPILATION_ROOT,
            path_syntax: path_syntax_id,
            class,
        };
        resolver
            .resolve(
                &root.join("main.moth"),
                path_syntax,
                &reference,
                strings,
                &path_fork,
            )
            .expect("synthetic reference should settle")
    };

    for (path, class) in [
        (
            "support/existing.mtf",
            PreparedFileReferenceClass::ContentSource,
        ),
        (
            "support/existing.svg",
            PreparedFileReferenceClass::ResourceFile,
        ),
        (
            "support/missing.mtf",
            PreparedFileReferenceClass::ContentSource,
        ),
        (
            "support/missing.svg",
            PreparedFileReferenceClass::ResourceFile,
        ),
        (
            "support-alias/missing.mtf",
            PreparedFileReferenceClass::ContentSource,
        ),
        (
            "child-alias/missing.svg",
            PreparedFileReferenceClass::ResourceFile,
        ),
        (
            "dangling-child/missing.svg",
            PreparedFileReferenceClass::ResourceFile,
        ),
        (
            "dangling-support/missing.mtf",
            PreparedFileReferenceClass::ContentSource,
        ),
    ] {
        let resolved = resolve(&mut resolver, path, class, &mut strings, &mut path_syntax);
        assert!(
            has_invalid_path_reason(
                resolved.outcome,
                InvalidCompileTimePathCode::EscapesModuleBoundary,
            ),
            "expected module-boundary rejection for {path}"
        );
    }

    for path in [
        "outside-alias/missing.mtf",
        "outside-alias/missing.svg",
        "dangling-external/missing.mtf",
        "dangling-external/missing.svg",
    ] {
        let resolved = resolve(
            &mut resolver,
            path,
            PreparedFileReferenceClass::ResourceFile,
            &mut strings,
            &mut path_syntax,
        );
        assert!(
            has_invalid_path_reason(resolved.outcome, InvalidCompileTimePathCode::EscapesSymlink,),
            "expected symlink escape rejection for {path}"
        );
    }
    assert!(resource_inputs.missing_watch_interests().is_empty());
}

#[cfg(unix)]
#[test]
fn underscore_files_resolve_but_hidden_directories_and_aliases_do_not() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("test root should be created");
    let root = fs::canonicalize(temp.path()).expect("test root should canonicalize");
    fs::write(root.join("_logo.svg"), "resource").expect("underscore resource should exist");
    fs::create_dir_all(root.join("_private")).expect("hidden directory should exist");
    symlink(root.join("_private"), root.join("visible"))
        .expect("visible alias to hidden directory should exist");
    symlink(root.join("_missing"), root.join("dangling"))
        .expect("dangling alias to a missing hidden directory should exist");
    symlink(root.join("missing/_private"), root.join("deeper"))
        .expect("dangling alias through a visible missing directory should exist");

    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut resolver =
        SingleFileReferenceResolver::new(root.clone(), &source_file_kinds, &mut resource_inputs);
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut path_syntax = PathSyntaxTable::new();
    let owner = root.join("main.moth");

    let mut resolve = |authored: &str| {
        let path = path_fork
            .try_intern_portable_path(authored, &mut strings)
            .expect("test path fits");
        let path_syntax_id = path_syntax
            .try_push_for_source(path, SourceId::COMPILATION_ROOT, LocalSpan::source_start())
            .expect("test path syntax row should fit");
        let reference = PreparedFileReference {
            span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
            source_file: SourceId::COMPILATION_ROOT,
            path_syntax: path_syntax_id,
            class: PreparedFileReferenceClass::ResourceFile,
        };
        resolver
            .resolve(&owner, &path_syntax, &reference, &mut strings, &path_fork)
            .expect("resource reference should resolve")
    };

    assert!(
        matches!(
            resolve("_logo.svg").outcome,
            SingleFileReferenceOutcome::Resource { .. }
        ),
        "underscore-prefixed files remain valid resources"
    );
    assert!(
        has_invalid_path_reason(
            resolve("_private/logo.svg").outcome,
            InvalidCompileTimePathCode::HiddenPath
        ),
        "hidden directories are rejected for resources"
    );
    assert!(
        has_invalid_path_reason(
            resolve("visible/missing.svg").outcome,
            InvalidCompileTimePathCode::HiddenPath
        ),
        "a missing target behind an alias into a hidden directory is diagnosed as hidden"
    );
    assert!(
        has_invalid_path_reason(
            resolve("dangling/missing.svg").outcome,
            InvalidCompileTimePathCode::HiddenPath
        ),
        "a dangling alias whose first missing component is a hidden directory is diagnosed as hidden"
    );
    assert!(
        has_invalid_path_reason(
            resolve("deeper/logo.svg").outcome,
            InvalidCompileTimePathCode::HiddenPath
        ),
        "a hidden directory later in a dangling alias target is still diagnosed as hidden"
    );
    assert!(
        resource_inputs.missing_watch_interests().is_empty(),
        "excluded missing targets gain no watcher edge"
    );
}
