//! Unit tests for compile-time path resolution.

use crate::builder_surface::PackageOrigin;
use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry, SourcePackageRegistry};
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, terse::format_terse_diagnostic_with_context,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, ImportDiagnosticKind, InvalidImportPathReason,
};
use crate::compiler_frontend::paths::compile_time_paths::CompileTimePathBase;
use crate::compiler_frontend::paths::dependency_resolution::DependencyPathResolutionError;
use crate::compiler_frontend::paths::module_roots::{ModuleRootRecord, ModuleRootTable};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
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
        build_source_package_boundary_indexes(
            source_packages,
            &SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::
                ExternalImportProviderRegistry::default(),
            &mut prep_string_table,
        )
        .expect("test source package boundary indexes should build")
        .prepared_source_package_roots()
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

    fn declaring_source(&self) -> PathBuf {
        self.project_root.join("src/index.moth")
    }
}

fn rendered_error_msg(error: &DependencyPathResolutionError, string_table: &StringTable) -> String {
    match error {
        DependencyPathResolutionError::Diagnostic(diagnostic) => {
            format_terse_diagnostic_with_context(
                diagnostic.as_ref(),
                DiagnosticRenderContext::new(string_table),
            )
        }
        DependencyPathResolutionError::Infrastructure(error) => error.msg.clone(),
    }
}

fn dependency_diagnostic_payload(error: &DependencyPathResolutionError) -> &DiagnosticPayload {
    let diagnostic = typed_dependency_diagnostic(error);

    assert_eq!(
        diagnostic.kind,
        crate::compiler_frontend::compiler_messages::DiagnosticKind::Import(
            ImportDiagnosticKind::InvalidImportPath
        )
    );

    &diagnostic.payload
}

fn typed_dependency_diagnostic(
    error: &DependencyPathResolutionError,
) -> &crate::compiler_frontend::compiler_messages::CompilerDiagnostic {
    let DependencyPathResolutionError::Diagnostic(diagnostic) = error else {
        panic!("expected typed dependency diagnostic, got infrastructure error");
    };

    diagnostic.as_ref()
}

#[test]
fn source_package_dependency_resolves_to_package_root() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");
    let package_root = project_root.join("lib/helper");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(&package_root).unwrap();
    fs::write(package_root.join("@mod.moth"), b"").unwrap();
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

    let declaring_source = entry_root.join("index.moth");
    let result = resolver
        .resolve_dependency_as_compile_time_path(&path, &declaring_source, &mut string_table)
        .expect("source-backed package dependency should resolve");

    assert_eq!(result.0, CompileTimePathBase::SourcePackageRoot);
    assert_eq!(
        result.1,
        fs::canonicalize(package_root.join("utils.moth")).unwrap(),
        "should resolve to source-backed package root file"
    );
}

#[test]
fn source_package_prefix_takes_priority_over_entry_root() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");
    let package_root = project_root.join("lib/helper");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(&package_root).unwrap();
    fs::write(package_root.join("@mod.moth"), b"").unwrap();
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

    let declaring_source = entry_root.join("index.moth");
    let result = resolver
        .resolve_dependency_as_compile_time_path(&path, &declaring_source, &mut string_table)
        .expect("source-backed package dependency should resolve");

    assert_eq!(result.0, CompileTimePathBase::SourcePackageRoot);
    assert_eq!(
        result.1,
        fs::canonicalize(package_root.join("utils.moth")).unwrap()
    );
}

#[test]
fn extensionless_dependency_resolves_supported_moth_template_candidate() {
    let mut registry = SourceFileKindRegistry::new();
    registry.register("mtf", SourceFileKind::MothTemplate);
    let mut h = TestHarness::with_source_file_kinds(&registry);
    fs::create_dir_all(h.project_root.join("src/docs")).unwrap();
    fs::write(h.project_root.join("src/docs/intro.mtf"), "hello").unwrap();

    let path = h.make_path(&["docs", "intro"]);
    let declaring_source = h.declaring_source();

    let result = h
        .resolver
        .resolve_dependency_to_source_file(&path, &declaring_source, &mut h.string_table)
        .expect("supported .mtf dependency should resolve");

    assert_eq!(result.kind, SourceFileKind::MothTemplate);
    assert!(result.path.ends_with("src/docs/intro.mtf"));
}

#[test]
fn recognized_unsupported_moth_template_candidate_reports_source_kind_diagnostic() {
    let mut h = TestHarness::new();
    fs::create_dir_all(h.project_root.join("src/docs")).unwrap();
    fs::write(h.project_root.join("src/docs/intro.mtf"), "hello").unwrap();

    let path = h.make_path(&["docs", "intro"]);
    let declaring_source = h.declaring_source();

    let error = h
        .resolver
        .resolve_dependency_to_source_file(&path, &declaring_source, &mut h.string_table)
        .expect_err("unsupported .mtf dependency should fail");
    let diagnostic = typed_dependency_diagnostic(&error);

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
fn direct_moth_template_extension_dependency_is_rejected_as_source_extension() {
    let mut registry = SourceFileKindRegistry::new();
    registry.register("mtf", SourceFileKind::MothTemplate);
    let mut h = TestHarness::with_source_file_kinds(&registry);
    fs::create_dir_all(h.project_root.join("src/docs")).unwrap();
    fs::write(h.project_root.join("src/docs/intro.mtf"), "hello").unwrap();

    let path = h.make_path(&["docs", "intro.mtf"]);
    let declaring_source = h.declaring_source();

    let error = h
        .resolver
        .resolve_dependency_to_source_file(&path, &declaring_source, &mut h.string_table)
        .expect_err("direct .mtf dependency should fail");
    let diagnostic = typed_dependency_diagnostic(&error);

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
    let declaring_source = h.declaring_source();

    let error = h
        .resolver
        .resolve_dependency_to_source_file(&path, &declaring_source, &mut h.string_table)
        .expect_err("same-stem .moth and .mtf should be ambiguous");
    let diagnostic = typed_dependency_diagnostic(&error);

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
    let declaring_source = h.declaring_source();

    let error = h
        .resolver
        .resolve_dependency_to_source_file(&path, &declaring_source, &mut h.string_table)
        .expect_err(".mtf and folder with same stem should be ambiguous");
    let diagnostic = typed_dependency_diagnostic(&error);

    assert_eq!(
        diagnostic.kind,
        crate::compiler_frontend::compiler_messages::DiagnosticKind::Import(
            ImportDiagnosticKind::AmbiguousImportTarget
        )
    );
}

#[test]
fn source_dependency_resolution_preserves_moth_template_folder_ambiguity() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");

    fs::create_dir_all(entry_root.join("docs/intro")).unwrap();
    fs::write(entry_root.join("index.moth"), b"").unwrap();
    fs::write(entry_root.join("docs/intro.mtf"), b"hello").unwrap();
    fs::write(entry_root.join("docs/intro/@content.moth"), b"").unwrap();

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

    let declaring_source = entry_root.join("index.moth");
    let error = resolver
        .resolve_dependency_to_source_file(&path, &declaring_source, &mut string_table)
        .expect_err("source resolution must not hide .mtf/folder ambiguity");
    let diagnostic = typed_dependency_diagnostic(&error);

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
    fs::write(package_root.join("@mod.moth"), b"").unwrap();
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
// Scan-root vs package-prefix behavior
// -----------------------------------------------------------------------

#[test]
fn package_scan_root_name_is_not_package_prefix() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");
    let package_root = project_root.join("lib/helper");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(&package_root).unwrap();
    fs::write(package_root.join("@mod.moth"), b"").unwrap();
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

    let declaring_source = entry_root.join("index.moth");
    let result = resolver
        .resolve_dependency_as_compile_time_path(&path, &declaring_source, &mut string_table)
        .expect("entry-root fallback dependency should resolve");

    assert_eq!(
        result.0,
        CompileTimePathBase::EntryRoot,
        "scan root name 'lib' must not be treated as a package prefix"
    );
}

#[test]
fn package_direct_child_is_package_prefix() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");
    let package_root = project_root.join("lib/helper");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(&package_root).unwrap();
    fs::write(package_root.join("@mod.moth"), b"").unwrap();
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

    let declaring_source = entry_root.join("index.moth");
    let result = resolver
        .resolve_dependency_as_compile_time_path(&path, &declaring_source, &mut string_table)
        .expect("source-backed package dependency should resolve");

    assert_eq!(
        result.0,
        CompileTimePathBase::SourcePackageRoot,
        "direct child of scan root must be a valid package prefix"
    );
}

#[test]
fn entry_root_dependency_fallback_success() {
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

    let declaring_source = entry_root.join("index.moth");
    let result = resolver
        .resolve_dependency_as_compile_time_path(&path, &declaring_source, &mut string_table)
        .expect("entry-root fallback dependency should resolve");

    assert_eq!(
        result.0,
        CompileTimePathBase::EntryRoot,
        "non-relative dependencies without a package prefix must fall back to entry root"
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
    fs::write(package_root.join("@mod.moth"), b"").unwrap();
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

    let declaring_source = entry_root.join("index.moth");
    let result = resolver
        .resolve_dependency_as_compile_time_path(&path, &declaring_source, &mut string_table)
        .expect("source-backed package dependency should resolve");

    assert_eq!(
        result.0,
        CompileTimePathBase::SourcePackageRoot,
        "source-backed package prefix must consistently win over entry-root collision"
    );
    assert_eq!(
        result.1,
        fs::canonicalize(package_root.join("utils.moth")).unwrap()
    );
}

// -----------------------------------------------------------------------
// Phase 4 — Dependency path restriction and canonicalization hardening
// -----------------------------------------------------------------------

#[test]
fn dependency_dotdot_rejected() {
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

    let declaring_source = entry_root.join("index.moth");
    let err = resolver
        .resolve_dependency_as_compile_time_path(&path, &declaring_source, &mut string_table)
        .expect_err("'..' in dependencies should be rejected");
    let rendered_msg = rendered_error_msg(&err, &string_table);

    assert!(
        rendered_msg.contains("'..' are not supported"),
        "expected '..' rejection, got: {}",
        rendered_msg
    );
}

#[test]
fn missing_dependency_target_is_typed_diagnostic() {
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

    let declaring_source = entry_root.join("index.moth");
    let err = resolver
        .resolve_dependency_as_compile_time_path(&path, &declaring_source, &mut string_table)
        .expect_err("missing dependency should be rejected");
    let diagnostic = typed_dependency_diagnostic(&err);

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
fn dependency_escape_project_root_rejected() {
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

    let declaring_source = entry_root.join("index.moth");
    let err = resolver
        .resolve_dependency_as_compile_time_path(&path, &declaring_source, &mut string_table)
        .expect_err("dependency escaping project root should be rejected");
    assert!(matches!(
        dependency_diagnostic_payload(&err),
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
fn dependency_escape_package_root_rejected() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");
    let package_root = project_root.join("lib/helper");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(&package_root).unwrap();
    fs::write(package_root.join("@mod.moth"), b"").unwrap();
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

    let declaring_source = entry_root.join("index.moth");
    let err = resolver
        .resolve_dependency_as_compile_time_path(&path, &declaring_source, &mut string_table)
        .expect_err("dependency escaping package root should be rejected");
    assert!(matches!(
        dependency_diagnostic_payload(&err),
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
fn concrete_file_dependency_inside_module_root_is_accepted() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();
    let entry_root = project_root.join("src");

    fs::create_dir_all(&entry_root).unwrap();
    fs::create_dir_all(entry_root.join("helper")).unwrap();
    fs::write(entry_root.join("helper/@home.moth"), b"").unwrap();
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

    let declaring_source = entry_root.join("index.moth");
    let result =
        resolver.resolve_dependency_to_source_file(&path, &declaring_source, &mut string_table);

    assert!(
        result.is_ok(),
        "concrete file dependency inside a module root should resolve at Stage 0"
    );
}

#[test]
fn nearest_root_parent_walk_chooses_nested_module_root_over_ancestor() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let entry_root = temp_dir.path().join("src");

    fs::create_dir_all(entry_root.join("outer/inner/deep")).unwrap();
    fs::write(entry_root.join("outer/@outer.moth"), b"").unwrap();
    fs::write(entry_root.join("outer/inner/@inner.moth"), b"").unwrap();
    fs::write(entry_root.join("outer/inner/deep/page.moth"), b"").unwrap();

    let outer_root_file = fs::canonicalize(entry_root.join("outer/@outer.moth"))
        .expect("outer root file should canonicalize");
    let inner_root_file = fs::canonicalize(entry_root.join("outer/inner/@inner.moth"))
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
fn dependency_case_sensitive_symbol_mismatch_rejected() {
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

    let declaring_source = entry_root.join("index.moth");
    let result = resolver.resolve_dependency_as_compile_time_path(
        &path,
        &declaring_source,
        &mut string_table,
    );

    #[cfg(target_os = "macos")]
    {
        let err = result.expect_err("case mismatch should be rejected on macOS");
        assert!(matches!(
            dependency_diagnostic_payload(&err),
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
// Plain Markdown dependency discovery
// -----------------------------------------------------------------------

#[test]
fn markdown_dependency_resolves_when_registered() {
    let mut registry = SourceFileKindRegistry::new();
    registry.register("md", SourceFileKind::PlainMarkdown);
    let mut h = TestHarness::with_source_file_kinds(&registry);
    fs::create_dir_all(h.project_root.join("src/docs")).unwrap();
    fs::write(h.project_root.join("src/docs/intro.md"), "# Hello").unwrap();

    let path = h.make_path(&["docs", "intro"]);
    let declaring_source = h.declaring_source();

    let result = h
        .resolver
        .resolve_dependency_to_source_file(&path, &declaring_source, &mut h.string_table)
        .expect("registered .md dependency should resolve");

    assert_eq!(result.kind, SourceFileKind::PlainMarkdown);
    assert!(result.path.ends_with("src/docs/intro.md"));
}

#[test]
fn markdown_dependency_rejected_when_unsupported() {
    let mut h = TestHarness::new();
    fs::create_dir_all(h.project_root.join("src/docs")).unwrap();
    fs::write(h.project_root.join("src/docs/intro.md"), "# Hello").unwrap();

    let path = h.make_path(&["docs", "intro"]);
    let declaring_source = h.declaring_source();

    let error = h
        .resolver
        .resolve_dependency_to_source_file(&path, &declaring_source, &mut h.string_table)
        .expect_err("unregistered .md dependency should be rejected");
    let diagnostic = typed_dependency_diagnostic(&error);

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
    let declaring_source = h.declaring_source();

    let error = h
        .resolver
        .resolve_dependency_to_source_file(&path, &declaring_source, &mut h.string_table)
        .expect_err("same-stem .moth and .md should be ambiguous");
    let diagnostic = typed_dependency_diagnostic(&error);

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
    let declaring_source = h.declaring_source();

    let error = h
        .resolver
        .resolve_dependency_to_source_file(&path, &declaring_source, &mut h.string_table)
        .expect_err(".md and folder with same stem should be ambiguous");
    let diagnostic = typed_dependency_diagnostic(&error);

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
    let declaring_source = h.declaring_source();

    let error = h
        .resolver
        .resolve_dependency_to_source_file(&path, &declaring_source, &mut h.string_table)
        .expect_err("same-stem .mtf and .md should be ambiguous");
    let diagnostic = typed_dependency_diagnostic(&error);

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
            PathBuf::from(root).join("@mod.moth"),
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
