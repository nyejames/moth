//! Unit tests for compile-time path resolution.

use crate::builder_surface::PackageOrigin;
use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry, SourcePackageRegistry};
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, terse::format_terse_diagnostic_with_context,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, ImportDiagnosticKind, InvalidCompileTimePathReason, InvalidImportPathReason,
    RuleDiagnosticKind,
};
use crate::compiler_frontend::paths::compile_time_paths::{
    CompileTimePathBase, CompileTimePathKind, CompileTimePathResolutionError,
};
use crate::compiler_frontend::paths::import_resolution::ImportPathResolutionError;
use crate::compiler_frontend::paths::module_roots::{ModuleRootRecord, ModuleRootTable};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::source_packages::root_file::{
    HashRootFileDiscovery, PreparedSourcePackageRoots,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::fs;
use std::path::PathBuf;

/// Creates a temp directory tree and a resolver for testing.
struct TestHarness {
    project_root: PathBuf,
    resolver: ProjectPathResolver,
    string_table: StringTable,
    _temp_dir: tempfile::TempDir,
}

fn prepared_source_package_roots(
    source_packages: &SourcePackageRegistry,
) -> PreparedSourcePackageRoots {
    let mut prep_string_table = StringTable::new();
    crate::build_system::create_project_modules::source_package_discovery::
        prepare_source_package_roots(source_packages, &mut prep_string_table)
        .expect("test source package roots should prepare")
}

impl TestHarness {
    fn new() -> Self {
        Self::with_source_packages(&crate::builder_surface::SourcePackageRegistry::default())
    }

    fn with_source_packages(
        source_packages: &crate::builder_surface::SourcePackageRegistry,
    ) -> Self {
        Self::with_packages_and_source_file_kinds(
            source_packages,
            &SourceFileKindRegistry::default(),
        )
    }

    fn with_source_file_kinds(source_file_kinds: &SourceFileKindRegistry) -> Self {
        Self::with_packages_and_source_file_kinds(
            &crate::builder_surface::SourcePackageRegistry::default(),
            source_file_kinds,
        )
    }

    fn with_packages_and_source_file_kinds(
        source_packages: &crate::builder_surface::SourcePackageRegistry,
        source_file_kinds: &SourceFileKindRegistry,
    ) -> Self {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let project_root = temp_dir.path().to_path_buf();
        let entry_root = project_root.join("src");

        // Create entry root and some fixtures.
        fs::create_dir_all(&entry_root).unwrap();
        fs::create_dir_all(entry_root.join("assets/images")).unwrap();
        fs::create_dir_all(entry_root.join("pages")).unwrap();
        fs::create_dir_all(project_root.join("docs")).unwrap();
        fs::write(entry_root.join("assets/images/logo.png"), b"").unwrap();
        fs::write(entry_root.join("pages/about.moth"), b"").unwrap();
        fs::write(entry_root.join("index.moth"), b"").unwrap();
        fs::write(project_root.join("docs/readme.txt"), b"").unwrap();

        let resolver = ProjectPathResolver::new(
            project_root.clone(),
            entry_root,
            prepared_source_package_roots(source_packages),
            source_file_kinds,
        )
        .expect("resolver creation should succeed");

        TestHarness {
            project_root,
            resolver,
            string_table: StringTable::new(),
            _temp_dir: temp_dir,
        }
    }

    fn make_path(&mut self, components: &[&str]) -> InternedPath {
        let mut path = InternedPath::new();
        for c in components {
            path.push_str(c, &mut self.string_table);
        }
        path
    }

    fn importer(&self) -> PathBuf {
        self.project_root.join("src/index.moth")
    }
}

fn rendered_error_msg(error: &ImportPathResolutionError, string_table: &StringTable) -> String {
    match error {
        ImportPathResolutionError::Diagnostic(diagnostic) => format_terse_diagnostic_with_context(
            diagnostic.as_ref(),
            DiagnosticRenderContext::new(string_table),
        ),
        ImportPathResolutionError::Infrastructure(error) => error.msg.clone(),
    }
}

fn import_diagnostic_payload(error: &ImportPathResolutionError) -> &DiagnosticPayload {
    let diagnostic = typed_import_diagnostic(error);

    assert_eq!(
        diagnostic.kind,
        crate::compiler_frontend::compiler_messages::DiagnosticKind::Import(
            ImportDiagnosticKind::InvalidImportPath
        )
    );

    &diagnostic.payload
}

fn typed_import_diagnostic(
    error: &ImportPathResolutionError,
) -> &crate::compiler_frontend::compiler_messages::CompilerDiagnostic {
    let ImportPathResolutionError::Diagnostic(diagnostic) = error else {
        panic!("expected typed import diagnostic, got infrastructure error");
    };

    diagnostic.as_ref()
}

fn compile_time_path_diagnostic_payload(
    error: &CompileTimePathResolutionError,
) -> &DiagnosticPayload {
    let CompileTimePathResolutionError::Diagnostic(diagnostic) = error else {
        panic!("expected typed compile-time path diagnostic, got infrastructure error");
    };

    assert_eq!(
        diagnostic.kind,
        crate::compiler_frontend::compiler_messages::DiagnosticKind::Rule(
            RuleDiagnosticKind::InvalidCompileTimePath
        )
    );

    &diagnostic.payload
}

fn prepared_module_root_table(root_file: &std::path::Path) -> ModuleRootTable {
    let canonical_root_file = fs::canonicalize(root_file).expect("root file should canonicalize");
    let root_directory = canonical_root_file
        .parent()
        .expect("root file should have a parent")
        .to_path_buf();
    ModuleRootTable::from_records(vec![ModuleRootRecord::new(
        root_directory,
        canonical_root_file,
    )])
}

// -----------------------------------------------------------------------
// Relative file resolution
// -----------------------------------------------------------------------

#[test]
fn relative_file_resolves_from_importer_directory() {
    let mut h = TestHarness::new();
    let path = h.make_path(&[".", "pages", "about.moth"]);
    let importer = h.importer();

    let result = h
        .resolver
        .resolve_compile_time_path(&path, &importer, &mut h.string_table)
        .expect("relative file should resolve");

    assert_eq!(result.base, CompileTimePathBase::RelativeToFile);
    assert_eq!(result.kind, CompileTimePathKind::File);
    assert!(result.filesystem_path.ends_with("src/pages/about.moth"));
}

// -----------------------------------------------------------------------
// Relative directory resolution
// -----------------------------------------------------------------------

#[test]
fn relative_directory_resolves_and_classifies_as_directory() {
    let mut h = TestHarness::new();
    let path = h.make_path(&[".", "pages"]);
    let importer = h.importer();

    let result = h
        .resolver
        .resolve_compile_time_path(&path, &importer, &mut h.string_table)
        .expect("relative directory should resolve");

    assert_eq!(result.base, CompileTimePathBase::RelativeToFile);
    assert_eq!(result.kind, CompileTimePathKind::Directory);
}

// -----------------------------------------------------------------------
// Entry root fallback resolution
// -----------------------------------------------------------------------

#[test]
fn entry_root_file_resolves_through_fallback() {
    let mut h = TestHarness::new();
    let path = h.make_path(&["pages", "about.moth"]);
    let importer = h.importer();

    let result = h
        .resolver
        .resolve_compile_time_path(&path, &importer, &mut h.string_table)
        .expect("entry root file should resolve");

    assert_eq!(result.base, CompileTimePathBase::EntryRoot);
    assert_eq!(result.kind, CompileTimePathKind::File);
}

// -----------------------------------------------------------------------
// Non-existent target rejection
// -----------------------------------------------------------------------

#[test]
fn non_existent_target_is_rejected() {
    let mut h = TestHarness::new();
    let path = h.make_path(&["pages", "does_not_exist.moth"]);
    let importer = h.importer();

    let err = h
        .resolver
        .resolve_compile_time_path(&path, &importer, &mut h.string_table)
        .expect_err("missing file should produce error");

    assert!(matches!(
        compile_time_path_diagnostic_payload(&err),
        DiagnosticPayload::InvalidCompileTimePath {
            reason: InvalidCompileTimePathReason::MissingTarget,
            ..
        }
    ));
}

// -----------------------------------------------------------------------
// Project root escape rejection
// -----------------------------------------------------------------------

#[test]
fn path_escaping_project_root_is_rejected() {
    let mut h = TestHarness::new();
    // From src/index.moth, going ../../.. escapes the project root.
    let path = h.make_path(&[".", "..", "..", "..", "escape.txt"]);
    let importer = h.importer();

    let err = h
        .resolver
        .resolve_compile_time_path(&path, &importer, &mut h.string_table)
        .expect_err("escape should produce error");

    assert!(matches!(
        compile_time_path_diagnostic_payload(&err),
        DiagnosticPayload::InvalidCompileTimePath {
            reason: InvalidCompileTimePathReason::EscapesProjectRoot,
            ..
        }
    ));
}

// -----------------------------------------------------------------------
// File vs directory classification
// -----------------------------------------------------------------------

#[test]
fn entry_root_directory_classifies_correctly() {
    let mut h = TestHarness::new();
    let path = h.make_path(&["assets", "images"]);
    let importer = h.importer();

    let result = h
        .resolver
        .resolve_compile_time_path(&path, &importer, &mut h.string_table)
        .expect("directory should resolve");

    assert_eq!(result.kind, CompileTimePathKind::Directory);
}

// -----------------------------------------------------------------------
// Public path segment preservation
// -----------------------------------------------------------------------

#[test]
fn relative_path_public_path_keeps_dot_prefix() {
    let mut h = TestHarness::new();
    let path = h.make_path(&[".", "pages", "about.moth"]);
    let importer = h.importer();

    let result = h
        .resolver
        .resolve_compile_time_path(&path, &importer, &mut h.string_table)
        .expect("should resolve");

    let public = result.public_path.to_portable_string(&h.string_table);
    assert!(public.starts_with("./"));
}

// -----------------------------------------------------------------------
// Multi-path resolution (`resolve_compile_time_paths`)
// -----------------------------------------------------------------------

#[test]
fn resolve_compile_time_paths_resolves_multiple_paths() {
    let mut h = TestHarness::new();
    let path_a = h.make_path(&["assets", "images", "logo.png"]);
    let path_b = h.make_path(&[".", "pages", "about.moth"]);
    let importer = h.importer();

    let result = h
        .resolver
        .resolve_compile_time_paths(&[path_a, path_b], &importer, &mut h.string_table)
        .expect("multi-path resolution should succeed");

    assert_eq!(result.paths.len(), 2);
    assert_eq!(result.paths[0].base, CompileTimePathBase::EntryRoot);
    assert_eq!(result.paths[0].kind, CompileTimePathKind::File);
    assert_eq!(result.paths[1].base, CompileTimePathBase::RelativeToFile);
    assert_eq!(result.paths[1].kind, CompileTimePathKind::File);
}

#[test]
fn resolve_compile_time_paths_fails_if_any_path_missing() {
    let mut h = TestHarness::new();
    let good = h.make_path(&["assets", "images", "logo.png"]);
    let bad = h.make_path(&["pages", "nonexistent.txt"]);
    let importer = h.importer();

    let err = h
        .resolver
        .resolve_compile_time_paths(&[good, bad], &importer, &mut h.string_table)
        .expect_err("should fail when any path is missing");

    assert!(matches!(
        compile_time_path_diagnostic_payload(&err),
        DiagnosticPayload::InvalidCompileTimePath {
            reason: InvalidCompileTimePathReason::MissingTarget,
            ..
        }
    ));
}

#[test]
fn empty_path_resolves_as_entry_root_public_directory() {
    let mut h = TestHarness::new();
    let path = InternedPath::new();
    let importer = h.importer();

    let result = h
        .resolver
        .resolve_compile_time_path(&path, &importer, &mut h.string_table)
        .expect("empty path should resolve to entry root");

    assert_eq!(result.base, CompileTimePathBase::EntryRoot);
    assert_eq!(result.kind, CompileTimePathKind::Directory);
    assert_eq!(result.filesystem_path, h.project_root.join("src"));
    assert!(result.public_path.as_components().is_empty());
}

#[test]
fn source_package_import_resolves_to_package_root() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");
    let package_root = project_root.join("lib/helper");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(&package_root).unwrap();
    fs::write(package_root.join("utils.moth"), b"").unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();

    let mut source_packages = crate::builder_surface::SourcePackageRegistry::new();
    source_packages.register_filesystem_root(
        "helper",
        package_root.clone(),
        PackageOrigin::Builder,
    );

    let resolver = ProjectPathResolver::new(
        project_root.clone(),
        entry_root.clone(),
        prepared_source_package_roots(&source_packages),
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
    .expect("resolver creation should succeed");

    let mut string_table = StringTable::new();
    let mut path = InternedPath::new();
    path.push_str("helper", &mut string_table);
    path.push_str("utils", &mut string_table);

    let importer = entry_root.join("index.moth");
    let result = resolver
        .resolve_import_as_compile_time_path(&path, &importer, &mut string_table)
        .expect("source-backed package import should resolve");

    assert_eq!(result.0.base, CompileTimePathBase::SourcePackageRoot);
    assert_eq!(
        result.1,
        fs::canonicalize(package_root.join("utils.moth")).unwrap(),
        "should resolve to source-backed package root file"
    );
}

#[test]
fn source_package_folder_import_uses_generic_hash_root_public_surface() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");
    let package_root = project_root.join("lib/helper");
    let root_file = package_root.join("#mod.moth");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(&package_root).unwrap();
    fs::write(&root_file, b"").unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();

    let mut source_packages = crate::builder_surface::SourcePackageRegistry::new();
    source_packages.register_filesystem_root(
        "helper",
        package_root.clone(),
        PackageOrigin::Builder,
    );

    let resolver = ProjectPathResolver::new(
        project_root.clone(),
        entry_root.clone(),
        prepared_source_package_roots(&source_packages),
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
    .expect("resolver creation should succeed");

    let canonical_root_file = fs::canonicalize(&root_file).unwrap();
    assert_eq!(
        resolver
            .source_package_public_surface_files()
            .find(|(prefix, _)| prefix.as_str() == "helper")
            .map(|(_, path)| path),
        Some(&canonical_root_file)
    );

    let mut string_table = StringTable::new();
    let mut path = InternedPath::new();
    path.push_str("helper", &mut string_table);

    let resolved = resolver
        .resolve_import_to_source_file_with_public_surface_fallback(
            &path,
            &entry_root.join("index.moth"),
            &mut string_table,
        )
        .expect("source-backed package folder import should use its generic root");

    assert_eq!(resolved.path, canonical_root_file);
}

#[test]
fn source_package_prefix_takes_priority_over_entry_root() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");
    let package_root = project_root.join("lib/helper");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(&package_root).unwrap();
    fs::write(package_root.join("utils.moth"), b"").unwrap();
    // Also create a conflicting file under entry root.
    fs::create_dir_all(entry_root.join("helper")).unwrap();
    fs::write(entry_root.join("helper/utils.moth"), b"").unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();

    let mut source_packages = crate::builder_surface::SourcePackageRegistry::new();
    source_packages.register_filesystem_root(
        "helper",
        package_root.clone(),
        PackageOrigin::Builder,
    );

    let resolver = ProjectPathResolver::new(
        project_root.clone(),
        entry_root.clone(),
        prepared_source_package_roots(&source_packages),
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
    .expect("resolver creation should succeed");

    let mut string_table = StringTable::new();
    let mut path = InternedPath::new();
    path.push_str("helper", &mut string_table);
    path.push_str("utils", &mut string_table);

    let importer = entry_root.join("index.moth");
    let result = resolver
        .resolve_import_as_compile_time_path(&path, &importer, &mut string_table)
        .expect("source-backed package import should resolve");

    assert_eq!(result.0.base, CompileTimePathBase::SourcePackageRoot);
    assert_eq!(
        result.1,
        fs::canonicalize(package_root.join("utils.moth")).unwrap()
    );
}

#[test]
fn extensionless_import_resolves_supported_moth_template_candidate() {
    let mut registry = SourceFileKindRegistry::new();
    registry.register("mtf", SourceFileKind::MothTemplate);
    let mut h = TestHarness::with_source_file_kinds(&registry);
    fs::create_dir_all(h.project_root.join("src/docs")).unwrap();
    fs::write(h.project_root.join("src/docs/intro.mtf"), "hello").unwrap();

    let path = h.make_path(&["docs", "intro"]);
    let importer = h.importer();

    let result = h
        .resolver
        .resolve_import_to_source_file(&path, &importer, &mut h.string_table)
        .expect("supported .mtf import should resolve");

    assert_eq!(result.kind, SourceFileKind::MothTemplate);
    assert!(result.path.ends_with("src/docs/intro.mtf"));
}

#[test]
fn recognized_unsupported_moth_template_candidate_reports_source_kind_diagnostic() {
    let mut h = TestHarness::new();
    fs::create_dir_all(h.project_root.join("src/docs")).unwrap();
    fs::write(h.project_root.join("src/docs/intro.mtf"), "hello").unwrap();

    let path = h.make_path(&["docs", "intro"]);
    let importer = h.importer();

    let error = h
        .resolver
        .resolve_import_to_source_file(&path, &importer, &mut h.string_table)
        .expect_err("unsupported .mtf import should fail");
    let diagnostic = typed_import_diagnostic(&error);

    assert_eq!(
        diagnostic.kind,
        crate::compiler_frontend::compiler_messages::DiagnosticKind::Import(
            ImportDiagnosticKind::UnsupportedSourceFileKind
        )
    );
    assert!(matches!(
        &diagnostic.payload,
        DiagnosticPayload::UnsupportedSourceFileKind { .. }
    ));
}

#[test]
fn direct_moth_template_extension_import_is_rejected_as_source_extension() {
    let mut registry = SourceFileKindRegistry::new();
    registry.register("mtf", SourceFileKind::MothTemplate);
    let mut h = TestHarness::with_source_file_kinds(&registry);
    fs::create_dir_all(h.project_root.join("src/docs")).unwrap();
    fs::write(h.project_root.join("src/docs/intro.mtf"), "hello").unwrap();

    let path = h.make_path(&["docs", "intro.mtf"]);
    let importer = h.importer();

    let error = h
        .resolver
        .resolve_import_to_source_file(&path, &importer, &mut h.string_table)
        .expect_err("direct .mtf import should fail");
    let diagnostic = typed_import_diagnostic(&error);

    assert_eq!(
        diagnostic.kind,
        crate::compiler_frontend::compiler_messages::DiagnosticKind::Import(
            ImportDiagnosticKind::ExplicitSourceExtension
        )
    );
    assert!(matches!(
        &diagnostic.payload,
        DiagnosticPayload::ExplicitSourceExtension { .. }
    ));
}

#[test]
fn moth_template_and_moth_same_stem_are_ambiguous() {
    let mut registry = SourceFileKindRegistry::new();
    registry.register("mtf", SourceFileKind::MothTemplate);
    let mut h = TestHarness::with_source_file_kinds(&registry);
    fs::create_dir_all(h.project_root.join("src/docs")).unwrap();
    fs::write(h.project_root.join("src/docs/intro.moth"), "").unwrap();
    fs::write(h.project_root.join("src/docs/intro.mtf"), "hello").unwrap();

    let path = h.make_path(&["docs", "intro"]);
    let importer = h.importer();

    let error = h
        .resolver
        .resolve_import_to_source_file(&path, &importer, &mut h.string_table)
        .expect_err("same-stem .moth and .mtf should be ambiguous");
    let diagnostic = typed_import_diagnostic(&error);

    assert_eq!(
        diagnostic.kind,
        crate::compiler_frontend::compiler_messages::DiagnosticKind::Import(
            ImportDiagnosticKind::AmbiguousImportTarget
        )
    );
}

#[test]
fn moth_template_and_folder_same_stem_are_ambiguous() {
    let mut registry = SourceFileKindRegistry::new();
    registry.register("mtf", SourceFileKind::MothTemplate);
    let mut h = TestHarness::with_source_file_kinds(&registry);
    fs::create_dir_all(h.project_root.join("src/docs/intro")).unwrap();
    fs::write(h.project_root.join("src/docs/intro.mtf"), "hello").unwrap();

    let path = h.make_path(&["docs", "intro"]);
    let importer = h.importer();

    let error = h
        .resolver
        .resolve_import_to_source_file(&path, &importer, &mut h.string_table)
        .expect_err(".mtf and folder with same stem should be ambiguous");
    let diagnostic = typed_import_diagnostic(&error);

    assert_eq!(
        diagnostic.kind,
        crate::compiler_frontend::compiler_messages::DiagnosticKind::Import(
            ImportDiagnosticKind::AmbiguousImportTarget
        )
    );
}

#[test]
fn public_surface_fallback_preserves_moth_template_folder_ambiguity() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");

    fs::create_dir_all(entry_root.join("docs/intro")).unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();
    fs::write(entry_root.join("docs/intro.mtf"), b"hello").unwrap();
    fs::write(entry_root.join("docs/intro/#content.moth"), b"").unwrap();

    let mut registry = SourceFileKindRegistry::new();
    registry.register("mtf", SourceFileKind::MothTemplate);
    let source_packages = crate::builder_surface::SourcePackageRegistry::new();
    let resolver = ProjectPathResolver::new(
        project_root.clone(),
        entry_root.clone(),
        prepared_source_package_roots(&source_packages),
        &registry,
    )
    .expect("resolver creation should succeed");

    let mut string_table = StringTable::new();
    let mut path = InternedPath::new();
    path.push_str("docs", &mut string_table);
    path.push_str("intro", &mut string_table);

    let importer = entry_root.join("index.moth");
    let error = resolver
        .resolve_import_to_source_file_with_public_surface_fallback(
            &path,
            &importer,
            &mut string_table,
        )
        .expect_err("public-surface fallback must not hide .mtf/folder ambiguity");
    let diagnostic = typed_import_diagnostic(&error);

    assert_eq!(
        diagnostic.kind,
        crate::compiler_frontend::compiler_messages::DiagnosticKind::Import(
            ImportDiagnosticKind::AmbiguousImportTarget
        )
    );
}

#[cfg(windows)]
#[test]
fn canonicalized_source_package_file_resolves_to_package_prefixed_logical_path() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");
    let package_root = project_root.join("lib/html");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(&package_root).unwrap();
    fs::write(package_root.join("#mod.moth"), b"").unwrap();
    fs::write(package_root.join("helpers.moth"), b"").unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();

    let mut source_packages = crate::builder_surface::SourcePackageRegistry::new();
    source_packages.register_filesystem_root("html", package_root.clone(), PackageOrigin::Builder);

    let resolver = ProjectPathResolver::new(
        project_root,
        entry_root,
        prepared_source_package_roots(&source_packages),
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
    .expect("resolver creation should succeed");

    let canonical_root = fs::canonicalize(&package_root).expect("should canonicalize package root");
    assert_eq!(
        resolver.source_package_roots().get("html"),
        Some(&canonical_root)
    );

    let canonical_file = fs::canonicalize(package_root.join("helpers.moth"))
        .expect("should canonicalize source-backed package file");
    let mut string_table = StringTable::new();
    let logical_path = resolver
        .logical_path_for_canonical_file(&canonical_file, &mut string_table)
        .expect("canonical source-backed package file should resolve");

    assert_eq!(logical_path, PathBuf::from("html").join("helpers.moth"));
}

// -----------------------------------------------------------------------
// Scan-root vs import-prefix behavior
// -----------------------------------------------------------------------

#[test]
fn package_scan_root_name_is_not_import_prefix() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");
    let package_root = project_root.join("lib/helper");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(&package_root).unwrap();
    fs::write(package_root.join("utils.moth"), b"").unwrap();
    fs::create_dir_all(entry_root.join("lib")).unwrap();
    fs::write(entry_root.join("lib/thing.moth"), b"").unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();

    let mut source_packages = crate::builder_surface::SourcePackageRegistry::new();
    source_packages.register_filesystem_root(
        "helper",
        package_root.clone(),
        PackageOrigin::Builder,
    );

    let resolver = ProjectPathResolver::new(
        project_root.clone(),
        entry_root.clone(),
        prepared_source_package_roots(&source_packages),
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
    .expect("resolver creation should succeed");

    let mut string_table = StringTable::new();
    let mut path = InternedPath::new();
    path.push_str("lib", &mut string_table);
    path.push_str("thing", &mut string_table);

    let importer = entry_root.join("index.moth");
    let result = resolver
        .resolve_import_as_compile_time_path(&path, &importer, &mut string_table)
        .expect("entry-root fallback import should resolve");

    assert_eq!(
        result.0.base,
        CompileTimePathBase::EntryRoot,
        "scan root name 'lib' must not be treated as an import prefix"
    );
}

#[test]
fn package_direct_child_is_import_prefix() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");
    let package_root = project_root.join("lib/helper");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(&package_root).unwrap();
    fs::write(package_root.join("utils.moth"), b"").unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();

    let mut source_packages = crate::builder_surface::SourcePackageRegistry::new();
    source_packages.register_filesystem_root(
        "helper",
        package_root.clone(),
        PackageOrigin::Builder,
    );

    let resolver = ProjectPathResolver::new(
        project_root.clone(),
        entry_root.clone(),
        prepared_source_package_roots(&source_packages),
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
    .expect("resolver creation should succeed");

    let mut string_table = StringTable::new();
    let mut path = InternedPath::new();
    path.push_str("helper", &mut string_table);
    path.push_str("utils", &mut string_table);

    let importer = entry_root.join("index.moth");
    let result = resolver
        .resolve_import_as_compile_time_path(&path, &importer, &mut string_table)
        .expect("source-backed package import should resolve");

    assert_eq!(
        result.0.base,
        CompileTimePathBase::SourcePackageRoot,
        "direct child of scan root must be a valid import prefix"
    );
}

#[test]
fn entry_root_import_fallback_success() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(entry_root.join("pages")).unwrap();
    fs::write(entry_root.join("pages/about.moth"), b"").unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();

    let source_packages = crate::builder_surface::SourcePackageRegistry::new();
    let resolver = ProjectPathResolver::new(
        project_root.clone(),
        entry_root.clone(),
        prepared_source_package_roots(&source_packages),
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
    .expect("resolver creation should succeed");

    let mut string_table = StringTable::new();
    let mut path = InternedPath::new();
    path.push_str("pages", &mut string_table);
    path.push_str("about", &mut string_table);

    let importer = entry_root.join("index.moth");
    let result = resolver
        .resolve_import_as_compile_time_path(&path, &importer, &mut string_table)
        .expect("entry-root fallback import should resolve");

    assert_eq!(
        result.0.base,
        CompileTimePathBase::EntryRoot,
        "non-relative imports without a package prefix must fall back to entry root"
    );
}

#[test]
fn source_package_prefix_wins_consistently() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");
    let package_root = project_root.join("lib/helper");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(&package_root).unwrap();
    fs::write(package_root.join("utils.moth"), b"").unwrap();
    // Also create a conflicting file under entry root.
    fs::create_dir_all(entry_root.join("helper")).unwrap();
    fs::write(entry_root.join("helper/utils.moth"), b"").unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();

    let mut source_packages = crate::builder_surface::SourcePackageRegistry::new();
    source_packages.register_filesystem_root(
        "helper",
        package_root.clone(),
        PackageOrigin::Builder,
    );

    let resolver = ProjectPathResolver::new(
        project_root.clone(),
        entry_root.clone(),
        prepared_source_package_roots(&source_packages),
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
    .expect("resolver creation should succeed");

    let mut string_table = StringTable::new();
    let mut path = InternedPath::new();
    path.push_str("helper", &mut string_table);
    path.push_str("utils", &mut string_table);

    let importer = entry_root.join("index.moth");
    let result = resolver
        .resolve_import_as_compile_time_path(&path, &importer, &mut string_table)
        .expect("source-backed package import should resolve");

    assert_eq!(
        result.0.base,
        CompileTimePathBase::SourcePackageRoot,
        "source-backed package prefix must consistently win over entry-root collision"
    );
    assert_eq!(
        result.1,
        fs::canonicalize(package_root.join("utils.moth")).unwrap()
    );
}

// -----------------------------------------------------------------------
// Phase 4 — Import path restriction and canonicalization hardening
// -----------------------------------------------------------------------

#[test]
fn import_dotdot_rejected() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");

    fs::create_dir_all(&entry_root).unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();

    let source_packages = crate::builder_surface::SourcePackageRegistry::new();
    let resolver = ProjectPathResolver::new(
        project_root.clone(),
        entry_root.clone(),
        prepared_source_package_roots(&source_packages),
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
    .expect("resolver creation should succeed");

    let mut string_table = StringTable::new();
    let mut path = InternedPath::new();
    path.push_str("..", &mut string_table);
    path.push_str("shared", &mut string_table);
    path.push_str("math", &mut string_table);

    let importer = entry_root.join("index.moth");
    let err = resolver
        .resolve_import_as_compile_time_path(&path, &importer, &mut string_table)
        .expect_err("'..' in imports should be rejected");
    let rendered_msg = rendered_error_msg(&err, &string_table);

    assert!(
        rendered_msg.contains("'..' are not supported"),
        "expected '..' rejection, got: {}",
        rendered_msg
    );
}

#[test]
fn missing_import_target_is_typed_diagnostic() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");

    fs::create_dir_all(&entry_root).unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();

    let source_packages = crate::builder_surface::SourcePackageRegistry::new();
    let resolver = ProjectPathResolver::new(
        project_root.clone(),
        entry_root.clone(),
        prepared_source_package_roots(&source_packages),
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
    .expect("resolver creation should succeed");

    let mut string_table = StringTable::new();
    let mut path = InternedPath::new();
    path.push_str("missing", &mut string_table);
    path.push_str("target", &mut string_table);

    let importer = entry_root.join("index.moth");
    let err = resolver
        .resolve_import_as_compile_time_path(&path, &importer, &mut string_table)
        .expect_err("missing import should be rejected");
    let diagnostic = typed_import_diagnostic(&err);

    assert_eq!(
        diagnostic.kind,
        crate::compiler_frontend::compiler_messages::DiagnosticKind::Import(
            ImportDiagnosticKind::MissingImportTarget
        )
    );
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::MissingImportTarget { .. }
    ));
}

#[test]
fn import_escape_project_root_rejected() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");

    fs::create_dir_all(&entry_root).unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();

    let source_packages = crate::builder_surface::SourcePackageRegistry::new();
    let resolver = ProjectPathResolver::new(
        project_root.clone(),
        entry_root.clone(),
        prepared_source_package_roots(&source_packages),
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
    .expect("resolver creation should succeed");

    let mut string_table = StringTable::new();
    let mut path = InternedPath::new();
    path.push_str(".", &mut string_table);
    path.push_str("..", &mut string_table);
    path.push_str("..", &mut string_table);
    path.push_str("escape", &mut string_table);

    let importer = entry_root.join("index.moth");
    let err = resolver
        .resolve_import_as_compile_time_path(&path, &importer, &mut string_table)
        .expect_err("import escaping project root should be rejected");
    assert!(matches!(
        import_diagnostic_payload(&err),
        DiagnosticPayload::InvalidImportPath {
            reason: InvalidImportPathReason::ParentDirectorySegment,
            ..
        }
    ));
    let rendered_msg = rendered_error_msg(&err, &string_table);

    assert!(
        rendered_msg.contains("'..' are not supported"),
        "expected '..' rejection, got: {}",
        rendered_msg
    );
}

#[test]
fn import_escape_package_root_rejected() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");
    let package_root = project_root.join("lib/helper");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(&package_root).unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();

    let mut source_packages = crate::builder_surface::SourcePackageRegistry::new();
    source_packages.register_filesystem_root(
        "helper",
        package_root.clone(),
        PackageOrigin::Builder,
    );

    let resolver = ProjectPathResolver::new(
        project_root.clone(),
        entry_root.clone(),
        prepared_source_package_roots(&source_packages),
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
    .expect("resolver creation should succeed");

    let mut string_table = StringTable::new();
    let mut path = InternedPath::new();
    path.push_str("helper", &mut string_table);
    path.push_str("..", &mut string_table);
    path.push_str("escape", &mut string_table);

    let importer = entry_root.join("index.moth");
    let err = resolver
        .resolve_import_as_compile_time_path(&path, &importer, &mut string_table)
        .expect_err("import escaping package root should be rejected");
    assert!(matches!(
        import_diagnostic_payload(&err),
        DiagnosticPayload::InvalidImportPath {
            reason: InvalidImportPathReason::ParentDirectorySegment,
            ..
        }
    ));
    let rendered_msg = rendered_error_msg(&err, &string_table);

    assert!(
        rendered_msg.contains("'..' are not supported"),
        "expected '..' rejection, got: {}",
        rendered_msg
    );
}

#[test]
fn module_root_public_surface_fallback_resolves_plain_folder_import() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(entry_root.join("helper")).unwrap();
    fs::write(entry_root.join("helper/#home.moth"), b"").unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();

    let source_packages = crate::builder_surface::SourcePackageRegistry::new();
    let resolver = ProjectPathResolver::new_with_module_roots(
        project_root.clone(),
        entry_root.clone(),
        prepared_source_package_roots(&source_packages),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        prepared_module_root_table(&entry_root.join("helper/#home.moth")),
    )
    .expect("resolver creation should succeed");

    let mut string_table = StringTable::new();
    let mut path = InternedPath::new();
    path.push_str("helper", &mut string_table);

    let importer = entry_root.join("index.moth");
    let result = resolver.resolve_import_to_source_file_with_public_surface_fallback(
        &path,
        &importer,
        &mut string_table,
    );

    assert!(
        result.is_ok(),
        "plain folder import should resolve to module root public surface"
    );
    assert_eq!(
        result.unwrap().path,
        fs::canonicalize(entry_root.join("helper/#home.moth")).unwrap()
    );
}

#[test]
fn disabled_module_root_discovery_does_not_register_plain_folder_public_surfaces() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(entry_root.join("helper")).unwrap();
    fs::write(entry_root.join("helper/#home.moth"), b"").unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();

    let source_packages = crate::builder_surface::SourcePackageRegistry::new();
    let resolver = ProjectPathResolver::new(
        project_root.clone(),
        entry_root.clone(),
        prepared_source_package_roots(&source_packages),
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
    .expect("resolver creation should succeed");

    assert!(
        resolver.module_roots().next().is_none(),
        "disabled discovery should not traverse and register sibling module roots"
    );

    let mut string_table = StringTable::new();
    let mut path = InternedPath::new();
    path.push_str("helper", &mut string_table);

    let importer = entry_root.join("index.moth");
    let result = resolver.resolve_import_to_source_file_with_public_surface_fallback(
        &path,
        &importer,
        &mut string_table,
    );

    assert!(
        result.is_err(),
        "single-file resolver policy should not use directory-project public-surface fallback"
    );
}

#[test]
fn plain_folder_import_to_module_root_uses_any_hash_root() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(entry_root.join("helper")).unwrap();
    fs::write(entry_root.join("helper/#home.moth"), b"").unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();

    let source_packages = crate::builder_surface::SourcePackageRegistry::new();
    let resolver = ProjectPathResolver::new_with_module_roots(
        project_root.clone(),
        entry_root.clone(),
        prepared_source_package_roots(&source_packages),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        prepared_module_root_table(&entry_root.join("helper/#home.moth")),
    )
    .expect("resolver creation should succeed");

    let mut string_table = StringTable::new();
    let mut path = InternedPath::new();
    path.push_str("helper", &mut string_table);

    let importer = entry_root.join("index.moth");
    let result = resolver
        .resolve_import_to_source_file_with_public_surface_fallback(
            &path,
            &importer,
            &mut string_table,
        )
        .expect("any hash root should be the module public surface");

    assert_eq!(
        result.path,
        fs::canonicalize(entry_root.join("helper/#home.moth")).unwrap()
    );
}

#[test]
fn concrete_file_import_inside_module_root_is_accepted() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(entry_root.join("helper")).unwrap();
    fs::write(entry_root.join("helper/#home.moth"), b"").unwrap();
    fs::write(entry_root.join("helper/thing.moth"), b"").unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();

    let source_packages = crate::builder_surface::SourcePackageRegistry::new();
    let resolver = ProjectPathResolver::new(
        project_root.clone(),
        entry_root.clone(),
        prepared_source_package_roots(&source_packages),
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
    .expect("resolver creation should succeed");

    let mut string_table = StringTable::new();
    let mut path = InternedPath::new();
    path.push_str("helper", &mut string_table);
    path.push_str("thing", &mut string_table);

    let importer = entry_root.join("index.moth");
    let result = resolver.resolve_import_to_source_file_with_public_surface_fallback(
        &path,
        &importer,
        &mut string_table,
    );

    assert!(
        result.is_ok(),
        "concrete file import inside a module root should resolve at Stage 0"
    );
}

#[test]
fn nearest_root_parent_walk_chooses_nested_module_root_over_ancestor() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let entry_root = temp_dir.path().join("src");

    fs::create_dir_all(entry_root.join("outer/inner/deep")).unwrap();
    fs::write(entry_root.join("outer/#outer.moth"), b"").unwrap();
    fs::write(entry_root.join("outer/inner/#inner.moth"), b"").unwrap();
    fs::write(entry_root.join("outer/inner/deep/page.moth"), b"").unwrap();

    let outer_root_file = fs::canonicalize(entry_root.join("outer/#outer.moth"))
        .expect("outer root file should canonicalize");
    let inner_root_file = fs::canonicalize(entry_root.join("outer/inner/#inner.moth"))
        .expect("inner root file should canonicalize");
    let outer_root_dir = outer_root_file
        .parent()
        .expect("outer root file should have a parent")
        .to_path_buf();
    let inner_root_dir = inner_root_file
        .parent()
        .expect("inner root file should have a parent")
        .to_path_buf();

    let table = ModuleRootTable::from_records(vec![
        ModuleRootRecord::new(outer_root_dir, outer_root_file),
        ModuleRootRecord::new(inner_root_dir.clone(), inner_root_file),
    ]);

    let deep_file = fs::canonicalize(entry_root.join("outer/inner/deep/page.moth"))
        .expect("deep file should canonicalize");

    let resolved = table
        .module_root_for_file(&deep_file)
        .expect("deep file should resolve to a module root");

    assert_eq!(
        resolved, inner_root_dir,
        "nearest-root parent-walk should choose the nested module root over its ancestor"
    );
}

#[test]
fn import_case_sensitive_symbol_mismatch_rejected() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(entry_root.join("pages")).unwrap();
    fs::write(entry_root.join("pages/about.moth"), b"").unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();

    let source_packages = crate::builder_surface::SourcePackageRegistry::new();
    let resolver = ProjectPathResolver::new(
        project_root.clone(),
        entry_root.clone(),
        prepared_source_package_roots(&source_packages),
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
    .expect("resolver creation should succeed");

    let mut string_table = StringTable::new();
    let mut path = InternedPath::new();
    path.push_str("pages", &mut string_table);
    path.push_str("About", &mut string_table);

    let importer = entry_root.join("index.moth");
    let result = resolver.resolve_import_as_compile_time_path(&path, &importer, &mut string_table);

    #[cfg(target_os = "macos")]
    {
        let err = result.expect_err("case mismatch should be rejected on macOS");
        assert!(matches!(
            import_diagnostic_payload(&err),
            DiagnosticPayload::InvalidImportPath {
                reason: InvalidImportPathReason::CaseMismatch { .. },
                ..
            }
        ));
        let rendered_msg = rendered_error_msg(&err, &string_table);
        assert!(
            rendered_msg.contains("case mismatch"),
            "expected case mismatch error, got: {}",
            rendered_msg
        );
    }

    #[cfg(not(target_os = "macos"))]
    {
        // On case-sensitive filesystems the file simply won't be found.
        assert!(
            result.is_err(),
            "case mismatch should fail on case-sensitive filesystems"
        );
    }
}

// -----------------------------------------------------------------------
// Plain Markdown import discovery
// -----------------------------------------------------------------------

#[test]
fn markdown_import_resolves_when_registered() {
    let mut registry = SourceFileKindRegistry::new();
    registry.register("md", SourceFileKind::PlainMarkdown);
    let mut h = TestHarness::with_source_file_kinds(&registry);
    fs::create_dir_all(h.project_root.join("src/docs")).unwrap();
    fs::write(h.project_root.join("src/docs/intro.md"), "# Hello").unwrap();

    let path = h.make_path(&["docs", "intro"]);
    let importer = h.importer();

    let result = h
        .resolver
        .resolve_import_to_source_file(&path, &importer, &mut h.string_table)
        .expect("registered .md import should resolve");

    assert_eq!(result.kind, SourceFileKind::PlainMarkdown);
    assert!(result.path.ends_with("src/docs/intro.md"));
}

#[test]
fn markdown_import_rejected_when_unsupported() {
    let mut h = TestHarness::new();
    fs::create_dir_all(h.project_root.join("src/docs")).unwrap();
    fs::write(h.project_root.join("src/docs/intro.md"), "# Hello").unwrap();

    let path = h.make_path(&["docs", "intro"]);
    let importer = h.importer();

    let error = h
        .resolver
        .resolve_import_to_source_file(&path, &importer, &mut h.string_table)
        .expect_err("unregistered .md import should be rejected");
    let diagnostic = typed_import_diagnostic(&error);

    assert_eq!(
        diagnostic.kind,
        crate::compiler_frontend::compiler_messages::DiagnosticKind::Import(
            ImportDiagnosticKind::UnsupportedSourceFileKind
        )
    );
}

#[test]
fn markdown_and_moth_same_stem_are_ambiguous() {
    let mut registry = SourceFileKindRegistry::new();
    registry.register("md", SourceFileKind::PlainMarkdown);
    let mut h = TestHarness::with_source_file_kinds(&registry);
    fs::create_dir_all(h.project_root.join("src/docs")).unwrap();
    fs::write(h.project_root.join("src/docs/intro.moth"), "").unwrap();
    fs::write(h.project_root.join("src/docs/intro.md"), "# Hello").unwrap();

    let path = h.make_path(&["docs", "intro"]);
    let importer = h.importer();

    let error = h
        .resolver
        .resolve_import_to_source_file(&path, &importer, &mut h.string_table)
        .expect_err("same-stem .moth and .md should be ambiguous");
    let diagnostic = typed_import_diagnostic(&error);

    assert_eq!(
        diagnostic.kind,
        crate::compiler_frontend::compiler_messages::DiagnosticKind::Import(
            ImportDiagnosticKind::AmbiguousImportTarget
        )
    );
}

#[test]
fn markdown_and_folder_same_stem_are_ambiguous() {
    let mut registry = SourceFileKindRegistry::new();
    registry.register("md", SourceFileKind::PlainMarkdown);
    let mut h = TestHarness::with_source_file_kinds(&registry);
    fs::create_dir_all(h.project_root.join("src/docs/intro")).unwrap();
    fs::write(h.project_root.join("src/docs/intro.md"), "# Hello").unwrap();

    let path = h.make_path(&["docs", "intro"]);
    let importer = h.importer();

    let error = h
        .resolver
        .resolve_import_to_source_file(&path, &importer, &mut h.string_table)
        .expect_err(".md and folder with same stem should be ambiguous");
    let diagnostic = typed_import_diagnostic(&error);

    assert_eq!(
        diagnostic.kind,
        crate::compiler_frontend::compiler_messages::DiagnosticKind::Import(
            ImportDiagnosticKind::AmbiguousImportTarget
        )
    );
}

#[test]
fn markdown_and_moth_template_same_stem_are_ambiguous() {
    let mut registry = SourceFileKindRegistry::new();
    registry.register("mtf", SourceFileKind::MothTemplate);
    registry.register("md", SourceFileKind::PlainMarkdown);
    let mut h = TestHarness::with_source_file_kinds(&registry);
    fs::create_dir_all(h.project_root.join("src/docs")).unwrap();
    fs::write(h.project_root.join("src/docs/intro.mtf"), "hello").unwrap();
    fs::write(h.project_root.join("src/docs/intro.md"), "# Hello").unwrap();

    let path = h.make_path(&["docs", "intro"]);
    let importer = h.importer();

    let error = h
        .resolver
        .resolve_import_to_source_file(&path, &importer, &mut h.string_table)
        .expect_err("same-stem .mtf and .md should be ambiguous");
    let diagnostic = typed_import_diagnostic(&error);

    assert_eq!(
        diagnostic.kind,
        crate::compiler_frontend::compiler_messages::DiagnosticKind::Import(
            ImportDiagnosticKind::AmbiguousImportTarget
        )
    );
}

fn resolver_with_prepared_source_package_roots(roots: &[(&str, &str)]) -> ProjectPathResolver {
    let entries = roots.iter().map(|(prefix, root)| {
        (
            (*prefix).to_owned(),
            PathBuf::from(root),
            HashRootFileDiscovery::Missing,
        )
    });

    ProjectPathResolver::new(
        PathBuf::from("/project"),
        PathBuf::from("/project/src"),
        PreparedSourcePackageRoots::from_entries(entries),
        &SourceFileKindRegistry::default(),
    )
    .expect("resolver should build")
}

#[test]
fn source_package_for_file_chooses_the_deepest_matching_root() {
    let resolver = resolver_with_prepared_source_package_roots(&[
        ("outer", "/packages/outer"),
        ("inner", "/packages/outer/inner"),
    ]);

    let selected = resolver
        .source_package_for_file(std::path::Path::new("/packages/outer/inner/file.moth"))
        .expect("nested source-backed package file should have a boundary");

    assert_eq!(selected.0, "inner");
    assert_eq!(selected.1, std::path::Path::new("/packages/outer/inner"));
}

#[test]
fn source_package_for_file_breaks_equal_root_ties_by_prefix() {
    let resolver = resolver_with_prepared_source_package_roots(&[
        ("zeta", "/packages/shared"),
        ("alpha", "/packages/shared"),
    ]);

    let selected = resolver
        .source_package_for_file(std::path::Path::new("/packages/shared/file.moth"))
        .expect("shared source-backed package file should have a boundary");

    assert_eq!(selected.0, "alpha");
}

#[test]
fn source_package_logical_paths_use_the_deepest_matching_root() {
    let resolver = resolver_with_prepared_source_package_roots(&[
        ("outer", "/packages/outer"),
        ("inner", "/packages/outer/inner"),
    ]);
    let mut string_table = StringTable::new();

    let logical_path = resolver
        .logical_path_for_canonical_file(
            std::path::Path::new("/packages/outer/inner/file.moth"),
            &mut string_table,
        )
        .expect("nested source-backed package file should have a logical path");

    assert_eq!(logical_path, PathBuf::from("inner/file.moth"));
}
