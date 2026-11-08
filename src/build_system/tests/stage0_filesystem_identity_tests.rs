//! Stage 0 filesystem identity, structural root claims and module-topology tests.
//!
//! WHAT: exercises the filesystem-facing contracts that integration output cannot inspect
//!      directly: strict UTF-8 identity, root-marker ownership and aliases, skip-policy and
//!      hidden-subtree exclusions, source-package boundaries, and the canonical module graph
//!      derived from discovery.
//! WHY: these invariants are subsystem-local Stage 0 facts. Keeping their focused fixtures beside
//!      the create-project-modules module makes failure ordering and source ownership visible
//!      without duplicating them in the larger orchestration suites.

use super::*;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::{DiagnosticSeverity, PreparedFailure};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::Path;

fn prepared_failure_reports(
    failure: PreparedFailure,
) -> crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet {
    let PreparedFailure::Diagnosed(batch) = failure else {
        panic!("expected diagnosed reports, got {failure:?}");
    };
    assert!(
        batch.has_errors(),
        "expected a user-facing error diagnostic"
    );
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(batch, None);
    accumulator
        .finish(InstallMode::Unfrozen)
        .expect("diagnosed reports should freeze")
}

fn primary_infrastructure_error(failure: PreparedFailure) -> InfrastructureFailure {
    let PreparedFailure::Infrastructure(errors) = failure else {
        panic!("expected an infrastructure failure, got {failure:?}");
    };
    errors.primary
}

/// Resolve one expected path to the spelling the diagnostic retains.
///
/// Authored symlinks stay visible in diagnostics; ordinary paths use their canonical spelling.
fn diagnostic_path_spelling(expected: &Path) -> String {
    let metadata = fs::symlink_metadata(expected).expect("expected path should exist");
    if metadata.file_type().is_symlink() {
        fs::canonicalize(
            expected
                .parent()
                .expect("expected path should have a parent"),
        )
        .expect("expected path's parent should canonicalize")
        .join(
            expected
                .file_name()
                .expect("expected path should have a file name"),
        )
        .display()
        .to_string()
    } else {
        fs::canonicalize(expected)
            .expect("expected path should canonicalize")
            .display()
            .to_string()
    }
}

fn assert_not_regular_file_marker_rejected(
    failure: PreparedFailure,
    expected_marker: &Path,
    expected_directory: &Path,
    expected_target: &Path,
) {
    use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::{
        InvalidConfigCode, InvalidConfigView,
    };

    let reports = prepared_failure_reports(failure);
    let (report, diagnostic) = reports
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .expect("malformed root markers should retain a diagnostic");
    assert_eq!(diagnostic.descriptor().code, "MOTH-CONFIG-0001");
    let facts = diagnostic
        .family_view::<InvalidConfigView>()
        .expect("malformed root markers should retain typed config facts");
    assert_eq!(
        facts.reason,
        InvalidConfigCode::ModuleRootFileNotRegularFile
    );

    let strings = report.context().strings();
    let rendered = diagnostic
        .string_ids()
        .iter()
        .map(|id| strings.resolve(*id))
        .collect::<Vec<_>>();
    for expected in [expected_marker, expected_directory, expected_target] {
        let expected = diagnostic_path_spelling(expected);
        assert!(
            rendered.iter().any(|candidate| *candidate == expected),
            "diagnostic must name {expected}: {rendered:?}"
        );
    }
}

fn assert_first_boundary_error_code(failure: PreparedFailure, expected_code: &str) {
    let reports = prepared_failure_reports(failure);
    let (_, diagnostic) = reports
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .expect("expected a boundary error diagnostic");
    assert_eq!(diagnostic.descriptor().code, expected_code);
}

#[cfg(test)]
fn compile_single_file_for_filesystem_test(
    config: &crate::projects::settings::Config,
    builder_surface: &mut crate::builder_surface::BuilderSurface,
    extension: &std::ffi::OsStr,
    string_table: &mut StringTable,
) -> crate::compiler_frontend::compiler_messages::compiler_errors::CompilationResult<
    crate::build_system::create_project_modules::ProjectFrontendCompilation,
> {
    let root_string_table = std::mem::take(string_table);
    let mut warning_owner =
        crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator::with_root_string_table(root_string_table);
    let mut project_source_files = None;
    super::compilation::compile_single_file_frontend_with_inputs(
        config,
        crate::compiler_frontend::FrontendBuildProfile::Dev,
        &crate::compiler_frontend::style_directives::StyleDirectiveRegistry::default(),
        builder_surface,
        extension,
        &mut warning_owner,
        &mut project_source_files,
        &crate::compiler_frontend::build_config::BuildConfigInputSet::new(),
        crate::build_system::create_project_modules::FrontendCompilationMode::Canonical,
    )
}

#[cfg(target_os = "linux")]
mod non_utf8_filesystem_identity {
    use super::*;
    use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    fn assert_file_infrastructure_error(failure: PreparedFailure) {
        let error = primary_infrastructure_error(failure);
        assert_eq!(
            error.kind,
            InfrastructureFailureKind::Filesystem,
            "non-UTF-8 filesystem name should be a File infrastructure error"
        );
        assert!(
            error.message.contains("Non-UTF-8"),
            "error message should mention non-UTF-8: {}",
            error.message,
        );
    }

    #[test]
    fn source_tree_rejects_non_utf8_file_name() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let entry_root = root.join("src");
        fs::create_dir_all(&entry_root).expect("should create entry root");
        fs::write(entry_root.join("@home.moth"), "").expect("should write entry root");

        let bad_name = OsString::from_vec(vec![0xC3, 0x28]);
        let bad_file = entry_root.join(bad_name);
        fs::write(&bad_file, "x ~= 1\n").expect("should write non-UTF-8 named file");

        let config = Config::new(root.clone());
        let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
        let canonical_entry_root =
            fs::canonicalize(&entry_root).expect("entry root should canonicalize");
        let mut string_table = StringTable::new();

        let failure = super::source_tree_index::SourceTreeIndex::discover(
            canonical_entry_root,
            super::source_tree_index::SourceTreeProjectContext {
                project_root: &canonical_root,
                validated_output_settings: None,
            },
            &config,
            &crate::builder_surface::SourcePackageRegistry::default(),
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
            &mut string_table,
        )
        .expect_err("non-UTF-8 file name should be rejected");
        assert_file_infrastructure_error(failure);
    }

    #[test]
    fn source_tree_rejects_non_utf8_folder_name() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let entry_root = root.join("src");
        fs::create_dir_all(&entry_root).expect("should create entry root");
        fs::write(entry_root.join("@home.moth"), "").expect("should write entry root");

        let bad_name = OsString::from_vec(vec![0xC3, 0x28]);
        let bad_folder = entry_root.join(bad_name);
        fs::create_dir_all(&bad_folder).expect("should create non-UTF-8 named folder");

        let config = Config::new(root.clone());
        let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
        let canonical_entry_root =
            fs::canonicalize(&entry_root).expect("entry root should canonicalize");
        let mut string_table = StringTable::new();

        let failure = super::source_tree_index::SourceTreeIndex::discover(
            canonical_entry_root,
            super::source_tree_index::SourceTreeProjectContext {
                project_root: &canonical_root,
                validated_output_settings: None,
            },
            &config,
            &crate::builder_surface::SourcePackageRegistry::default(),
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
            &mut string_table,
        )
        .expect_err("non-UTF-8 folder name should be rejected");
        assert_file_infrastructure_error(failure);
    }

    #[test]
    fn non_utf8_direct_child_outside_entry_root_is_ignored() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let entry_root = root.join("src");
        fs::create_dir_all(&entry_root).expect("should create entry root");
        fs::write(entry_root.join("@page.moth"), "").expect("should write entry root");

        // The selected directory is the traversal boundary. A non-UTF-8 sibling of it is outside
        // that boundary and must not enter the inventory or force an error.
        let bad_name = OsString::from_vec(vec![0xC3, 0x28]);
        let bad_file = root.join(bad_name);
        fs::write(&bad_file, "x ~= 1\n").expect("should write non-UTF-8 named file");

        let config = Config::new(root.clone());
        let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
        let canonical_entry_root =
            fs::canonicalize(&entry_root).expect("entry root should canonicalize");
        let mut string_table = StringTable::new();

        let index = super::source_tree_index::SourceTreeIndex::discover(
            canonical_entry_root,
            super::source_tree_index::SourceTreeProjectContext {
                project_root: &canonical_root,
                validated_output_settings: None,
            },
            &config,
            &crate::builder_surface::SourcePackageRegistry::default(),
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
            &mut string_table,
        )
        .expect("a non-UTF-8 path outside the selected directory must be ignored");
        assert_eq!(index.module_identities().module_ids().count(), 1);
    }

    #[test]
    fn package_boundary_traversal_rejects_non_utf8_name() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");

        let bad_name = OsString::from_vec(vec![0xC3, 0x28]);
        let bad_file = package_root.join(bad_name);
        fs::write(&bad_file, "x ~= 1\n").expect("should write non-UTF-8 named file");

        let mut source_packages = crate::builder_surface::SourcePackageRegistry::new();
        source_packages
            .register_filesystem_root(
                "pkg",
                package_root,
                crate::builder_surface::PackageOrigin::ProjectLocal,
            )
            .expect("test source root should register");

        let mut string_table = StringTable::new();
        let failure = super::source_package_discovery::build_source_package_boundary_indexes(
            &source_packages,
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::
                ExternalImportProviderRegistry::default(),
            &mut string_table,
        )
        .expect_err("non-UTF-8 name in package boundary traversal should be rejected");
        assert_file_infrastructure_error(failure);
    }

    #[test]
    fn packages_folder_inside_entry_root_rejects_non_utf8_child() {
        let _temp = tempfile::tempdir().expect("should create temp dir");
        let root = _temp.path().to_path_buf();
        let packages_folder = root.join("packages");
        fs::create_dir_all(&packages_folder).expect("should create packages folder");
        fs::write(root.join("@page.moth"), "x ~= 1\n").expect("should write entry");

        // `packages/` is an ordinary source-tree directory unless a package registry claims it.
        // The selected boundary is the project root, so traversal must report its bad child.
        let bad_name = OsString::from_vec(vec![0xC3, 0x28]);
        let bad_package = packages_folder.join(bad_name);
        fs::create_dir_all(&bad_package).expect("should create non-UTF-8 named package directory");

        let config = Config::new(root.clone());
        let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
        let mut string_table = StringTable::new();

        let failure = super::source_tree_index::SourceTreeIndex::discover(
            canonical_root.clone(),
            super::source_tree_index::SourceTreeProjectContext {
                project_root: &canonical_root,
                validated_output_settings: None,
            },
            &config,
            &crate::builder_surface::SourcePackageRegistry::default(),
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
            &mut string_table,
        )
        .expect_err("an ordinary packages directory must be traversed");

        let error = primary_infrastructure_error(failure);
        assert_eq!(error.kind, InfrastructureFailureKind::Filesystem);
        assert!(
            error.message.contains("Non-UTF-8") && error.message.contains("packages"),
            "error should identify the non-UTF-8 packages child: {}",
            error.message,
        );
    }
}

#[cfg(unix)]
mod non_utf8_single_file_identity {
    use super::*;
    use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    fn assert_file_infrastructure_error<T>(failure: &CompilationResult<T>) {
        let errors = match failure {
            Err(errors) => errors,
            Ok(_) => panic!("expected filesystem infrastructure failure"),
        };
        assert_eq!(
            errors.primary.kind,
            InfrastructureFailureKind::Filesystem,
            "non-UTF-8 single-file input should be a File infrastructure error"
        );
        assert!(
            errors.primary.message.contains("UTF-8"),
            "error message should mention UTF-8: {}",
            errors.primary.message,
        );
    }
    #[test]
    fn single_file_rejects_non_utf8_extension() {
        let temp = tempfile::tempdir().expect("should create temp dir");
        let root = temp.path().to_path_buf();
        let entry = root.join("main.");
        let bad_ext = OsString::from_vec(vec![0xC3, 0x28]);
        let entry_with_bad_ext = entry.with_extension(bad_ext);

        let config = Config::new(entry_with_bad_ext.clone());
        let mut builder_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
        let mut string_table = StringTable::new();

        let extension = entry_with_bad_ext
            .extension()
            .expect("entry should have an extension");
        let messages = super::compile_single_file_for_filesystem_test(
            &config,
            &mut builder_surface,
            extension,
            &mut string_table,
        );
        assert_file_infrastructure_error(&messages);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn single_file_rejects_non_utf8_entry_name() {
        let temp = tempfile::tempdir().expect("should create temp dir");
        let root = temp.path().to_path_buf();
        let bad_name = OsString::from_vec(vec![0xC3, 0x28]);
        let bad_file = root.join(bad_name).with_extension("moth");
        fs::write(&bad_file, "x ~= 1\n").expect("should write entry file");

        let config = Config::new(bad_file.clone());
        let mut builder_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
        let mut string_table = StringTable::new();

        let extension = bad_file
            .extension()
            .expect("entry should have a .moth extension");
        let messages = super::compile_single_file_for_filesystem_test(
            &config,
            &mut builder_surface,
            extension,
            &mut string_table,
        );
        assert_file_infrastructure_error(&messages);
    }
}

#[cfg(windows)]
mod non_utf8_windows_single_file_identity {
    use super::*;
    use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    fn assert_file_infrastructure_error<T>(failure: &CompilationResult<T>) {
        let errors = match failure {
            Err(errors) => errors,
            Ok(_) => panic!("expected filesystem infrastructure failure"),
        };
        assert_eq!(
            errors.primary.kind,
            InfrastructureFailureKind::Filesystem,
            "unpaired-wide single-file input should be a File infrastructure error"
        );
        assert!(
            errors.primary.message.contains("UTF-8"),
            "error message should mention UTF-8: {}",
            errors.primary.message,
        );
    }

    #[test]
    fn single_file_rejects_unpaired_wide_extension() {
        let temp = tempfile::tempdir().expect("should create temp dir");
        let root = temp.path().to_path_buf();
        let entry = root.join("main.");
        let bad_ext = OsString::from_wide(&[0xD800]);
        let entry_with_bad_ext = entry.with_extension(bad_ext);

        let config = Config::new(entry_with_bad_ext.clone());
        let mut builder_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
        let mut string_table = StringTable::new();

        let extension = entry_with_bad_ext
            .extension()
            .expect("entry should have an extension");
        let messages = super::compile_single_file_for_filesystem_test(
            &config,
            &mut builder_surface,
            extension,
            &mut string_table,
        );
        assert_file_infrastructure_error(&messages);
    }
}

mod source_package_boundary_indexes_tests {
    use super::*;
    use crate::builder_surface::SourcePackageRegistry;
    use crate::builder_surface::external_import_providers::provider::ExternalFileExtension;
    use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
    use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
    use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
    use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::{
        InvalidConfigCode, InvalidConfigView,
    };
    use crate::compiler_frontend::semantic_identity::ModuleRootRole;
    #[cfg(unix)]
    use crate::compiler_tests::test_fs::RestrictedDirectory;
    use crate::projects::html_project::external_js::js_import_provider::JsExternalImportProvider;
    use std::path::Path;
    use std::sync::Arc;

    fn build_indexes(
        source_packages: &SourcePackageRegistry,
        string_table: &mut StringTable,
    ) -> Result<
        super::source_package_discovery::SourcePackageBoundaryIndexes,
        crate::compiler_frontend::compiler_messages::PreparedFailure,
    > {
        super::source_package_discovery::build_source_package_boundary_indexes(
            source_packages,
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::
                ExternalImportProviderRegistry::default(),
            string_table,
        )
    }

    fn register_pkg(source_packages: &mut SourcePackageRegistry, package_root: &Path) {
        source_packages
            .register_filesystem_root(
                "pkg",
                package_root.to_path_buf(),
                crate::builder_surface::PackageOrigin::ProjectLocal,
            )
            .expect("test source root should register");
    }

    fn assert_invalid_config_reason(failure: PreparedFailure, expected_reason: InvalidConfigCode) {
        let reports = prepared_failure_reports(failure);
        assert!(
            reports
                .iter_severity(DiagnosticSeverity::Error)
                .any(|(_, diagnostic)| {
                    diagnostic.descriptor().code == "MOTH-CONFIG-0001"
                        && diagnostic
                            .family_view::<InvalidConfigView>()
                            .is_some_and(|facts| facts.reason == expected_reason)
                }),
            "expected the typed config failure {expected_reason:?}",
        );
    }

    #[test]
    fn canonical_root_with_single_hash_file_derives_unique_view() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");
        fs::write(package_root.join("@home.moth"), "").expect("should write normal module root");

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &package_root);

        let mut string_table = StringTable::new();
        let indexes = build_indexes(&source_packages, &mut string_table)
            .expect("canonical package root should build a boundary index");

        // The package boundary index is the single owner; the resolver view is derived from it.
        let prepared = indexes.prepared_source_package_roots();
        let roots = prepared.roots();
        assert_eq!(roots.len(), 1, "one package root should be derived");
        let canonical = roots.get("pkg").expect("pkg root should exist");
        assert_eq!(
            *canonical,
            fs::canonicalize(&package_root).expect("root should canonicalize"),
            "derived root should be canonical"
        );

        let root_file = prepared
            .root_files()
            .get("pkg")
            .expect("pkg root file should exist");
        let canonical_root_file = fs::canonicalize(package_root.join("@home.moth"))
            .expect("normal module root file should canonicalize");
        assert_eq!(
            *root_file, canonical_root_file,
            "derived root file should be canonical"
        );

        // Boundary-local invariant: the package index owns exactly one module rooted at the
        // package root, with boundary-local dense IDs. Raw IDs never cross boundaries.
        let mut iter = indexes.iter();
        let (_, index) = iter.next().expect("one package index should exist");
        assert!(
            iter.next().is_none(),
            "exactly one package index should exist"
        );
        assert_eq!(
            index.root_file_for_entry_root(),
            Some(canonical_root_file.as_path()),
            "package root module file should match the derived public surface"
        );
    }

    #[test]
    fn support_only_package_root_selects_the_support_public_surface() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");
        fs::write(package_root.join("+home.moth"), "").expect("should write support module root");

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &package_root);

        let mut string_table = StringTable::new();
        let indexes = build_indexes(&source_packages, &mut string_table)
            .expect("support-only package root should build a boundary index");
        let (_, index) = indexes
            .iter()
            .next()
            .expect("one package index should exist");

        let canonical_root_file = fs::canonicalize(package_root.join("+home.moth"))
            .expect("support module root file should canonicalize");
        assert_eq!(
            index.root_file_for_entry_root(),
            Some(canonical_root_file.as_path()),
            "a support root is the package's external public surface"
        );

        let module_id = index
            .module_identities()
            .module_id_for_directory(index.entry_root())
            .expect("the package root directory should have a structural module");
        let record = index.module_identities().record(module_id);
        assert_eq!(record.public_surface_role(), Some(ModuleRootRole::Support));
        assert_eq!(
            record.public_root_file(),
            Some(canonical_root_file.as_path())
        );
        assert!(
            record.surface_origin(ModuleRootRole::Support).is_some(),
            "the selected public surface must retain its stable support origin"
        );
    }

    #[test]
    fn canonicalization_failure_returns_file_error() {
        let _temp = tempfile::tempdir().expect("should create temp dir");
        let root = _temp.path().to_path_buf();

        let nonexistent = root.join("does_not_exist");

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &nonexistent);

        let mut string_table = StringTable::new();
        let failure = build_indexes(&source_packages, &mut string_table)
            .expect_err("nonexistent root should fail canonicalization");
        let error = primary_infrastructure_error(failure);
        assert_eq!(error.kind, InfrastructureFailureKind::Filesystem);
        assert!(
            error.message.contains("canonicalize"),
            "error message should mention canonicalization: {}",
            error.message,
        );
    }

    #[test]
    fn missing_module_root_rejected_during_boundary_indexing() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &package_root);

        let mut string_table = StringTable::new();
        let failure = build_indexes(&source_packages, &mut string_table)
            .expect_err("package root without a normal module root should fail boundary indexing");
        assert_invalid_config_reason(failure, InvalidConfigCode::SourcePackageMissingRoot);
    }

    #[test]
    fn entry_only_package_root_is_rejected_as_missing_root() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");
        fs::write(package_root.join("@entry.moth"), "").expect("should write entry root");

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &package_root);

        let mut string_table = StringTable::new();
        let failure = build_indexes(&source_packages, &mut string_table)
            .expect_err("an entry-only package root publishes no importable facade");
        assert_invalid_config_reason(failure, InvalidConfigCode::SourcePackageMissingRoot);
    }

    #[test]
    fn package_facade_only_package_root_is_accepted_as_external_surface() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");
        fs::write(package_root.join("@package.moth"), "")
            .expect("should write package facade root");

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &package_root);

        let mut string_table = StringTable::new();
        let indexes = build_indexes(&source_packages, &mut string_table)
            .expect("a package-facade-only package should build a boundary index");
        let (_, index) = indexes.iter().next().expect("package index should exist");
        let canonical_root_file = fs::canonicalize(package_root.join("@package.moth"))
            .expect("package facade root should canonicalize");
        assert_eq!(
            index.root_file_for_entry_root(),
            Some(canonical_root_file.as_path()),
            "a package facade is the package's external public surface"
        );

        let module_id = index
            .module_identities()
            .module_id_for_directory(
                &fs::canonicalize(&package_root).expect("package root should canonicalize"),
            )
            .expect("the package facade should define one structural module");
        let record = index.module_identities().record(module_id);
        assert_eq!(record.normal_root_file(), None);
        assert_eq!(record.public_surface_role(), Some(ModuleRootRole::Support));
        assert_eq!(
            record.public_root_file(),
            Some(canonical_root_file.as_path())
        );

        let prepared = indexes.prepared_source_package_roots();
        assert_eq!(
            prepared.root_files().get("pkg"),
            Some(&canonical_root_file),
            "a package facade is the external package surface"
        );
    }

    #[test]
    fn support_only_package_is_accepted_as_an_external_surface() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");
        fs::write(package_root.join("+support.moth"), "").expect("should write support root");

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &package_root);

        let mut string_table = StringTable::new();
        let indexes = build_indexes(&source_packages, &mut string_table)
            .expect("a support-only package should build a boundary index");
        let (_, index) = indexes.iter().next().expect("package index should exist");
        let module_id = index
            .module_identities()
            .module_id_for_directory(
                &fs::canonicalize(&package_root).expect("package root should canonicalize"),
            )
            .expect("support root should define one structural module");
        let record = index.module_identities().record(module_id);
        assert_eq!(record.normal_root_file(), None);
        assert!(record.support_root_file().is_some());

        let prepared = indexes.prepared_source_package_roots();
        assert_eq!(
            prepared.root_files().get("pkg"),
            Some(
                &fs::canonicalize(package_root.join("+support.moth"))
                    .expect("support root should canonicalize")
            ),
            "a support root is the external package surface"
        );
    }

    #[test]
    fn paired_package_surfaces_select_the_support_root_as_external_surface() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");
        fs::write(package_root.join("@mod.moth"), "").expect("should write normal module root");
        fs::write(package_root.join("+support.moth"), "").expect("should write support root");

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &package_root);

        let mut string_table = StringTable::new();
        let indexes = build_indexes(&source_packages, &mut string_table)
            .expect("a paired package should build a boundary index");
        let (_, index) = indexes.iter().next().expect("package index should exist");
        let module_id = index
            .module_identities()
            .module_id_for_directory(
                &fs::canonicalize(&package_root).expect("package root should canonicalize"),
            )
            .expect("paired roots should define one structural module");
        let record = index.module_identities().record(module_id);
        assert!(record.normal_root_file().is_some());
        assert!(record.support_root_file().is_some());

        let prepared = indexes.prepared_source_package_roots();
        assert_eq!(
            prepared.root_files().get("pkg"),
            Some(
                &fs::canonicalize(package_root.join("+support.moth"))
                    .expect("support root should canonicalize")
            ),
            "a support root beside a normal root is the external package surface"
        );
    }

    #[cfg(unix)]
    #[test]
    fn aliased_package_roots_are_rejected_before_publication() {
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");
        let target = package_root.join("actual.moth");
        fs::write(&target, "").expect("should write shared target");
        symlink(&target, package_root.join("@page.moth")).expect("should create normal root");
        symlink(&target, package_root.join("+package.moth")).expect("should create support root");

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &package_root);

        let mut string_table = StringTable::new();
        let failure = build_indexes(&source_packages, &mut string_table).expect_err(
            "a source package whose normal and support roots alias one file must fail discovery",
        );
        assert_invalid_config_reason(failure, InvalidConfigCode::MultipleModuleRootFiles);
    }

    #[test]
    fn multiple_normal_module_roots_rejected_during_boundary_indexing() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");
        fs::write(package_root.join("@home.moth"), "")
            .expect("should write first normal module root");
        fs::write(package_root.join("@page.moth"), "")
            .expect("should write second normal module root");

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &package_root);

        let mut string_table = StringTable::new();
        let failure = build_indexes(&source_packages, &mut string_table).expect_err(
            "package root with multiple normal module roots should fail boundary indexing",
        );
        assert_invalid_config_reason(failure, InvalidConfigCode::SourcePackageMultipleRoots);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_normal_module_root_returns_file_error() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");
        fs::write(package_root.join("@home.moth"), "").expect("should write normal module root");

        // Remove read permission so the package boundary traversal cannot read the directory.
        // Canonicalization still succeeds because it only traverses the parent.
        let _restricted_package_root = RestrictedDirectory::deny_listing(&package_root);

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &package_root);

        let mut string_table = StringTable::new();
        let failure = build_indexes(&source_packages, &mut string_table)
            .expect_err("unreadable package root should fail boundary indexing");
        let error = primary_infrastructure_error(failure);
        assert_eq!(error.kind, InfrastructureFailureKind::Filesystem);
    }

    #[test]
    fn independent_package_indexes_have_boundary_local_module_ids() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let alpha_root = root.join("alpha");
        let beta_root = root.join("beta");
        fs::create_dir_all(&alpha_root).expect("should create alpha root");
        fs::create_dir_all(&beta_root).expect("should create beta root");
        fs::write(alpha_root.join("@mod.moth"), "").expect("should write alpha root");
        fs::write(beta_root.join("@mod.moth"), "").expect("should write beta root");

        let mut source_packages = SourcePackageRegistry::new();
        source_packages
            .register_filesystem_root(
                "alpha",
                alpha_root.clone(),
                crate::builder_surface::PackageOrigin::ProjectLocal,
            )
            .expect("test source root should register");
        source_packages
            .register_filesystem_root(
                "beta",
                beta_root.clone(),
                crate::builder_surface::PackageOrigin::ProjectLocal,
            )
            .expect("test source root should register");

        let mut string_table = StringTable::new();
        let indexes = build_indexes(&source_packages, &mut string_table)
            .expect("two package roots should build two boundary indexes");

        // Deterministic package-prefix order.
        let prefixes: Vec<&str> = indexes.iter().map(|(prefix, _)| prefix).collect();
        assert_eq!(
            prefixes,
            vec!["alpha", "beta"],
            "package indexes are prefix-ordered"
        );

        // Each boundary owns exactly one module rooted at its package root, with boundary-local
        // dense ModuleIds. Raw IDs never cross boundaries.
        for (expected_prefix, expected_root) in [("alpha", &alpha_root), ("beta", &beta_root)] {
            let (_, index) = indexes
                .iter()
                .find(|(prefix, _)| *prefix == expected_prefix)
                .expect("package index should exist");
            let module_id = index
                .module_identities()
                .module_ids()
                .next()
                .expect("one module rooted at the package root");
            assert_eq!(
                index.module_identities().record(module_id).root_directory(),
                &fs::canonicalize(expected_root).unwrap(),
                "module root directory should be the package root"
            );
        }
    }

    #[test]
    fn package_boundary_indexes_classify_provider_files_under_nearest_module() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");
        fs::write(package_root.join("@mod.moth"), "").expect("should write package root");
        fs::write(package_root.join("helper.js"), "").expect("should write provider file");

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &package_root);
        let mut providers = ExternalImportProviderRegistry::empty();
        providers.register(Arc::new(JsExternalImportProvider::new()));

        let mut string_table = StringTable::new();
        let indexes = super::source_package_discovery::build_source_package_boundary_indexes(
            &source_packages,
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &providers,
            &mut string_table,
        )
        .expect("provider-owned package source should be indexed");

        let (_, index) = indexes
            .iter()
            .next()
            .expect("one package index should exist");
        let module_id = index
            .module_identities()
            .module_ids()
            .next()
            .expect("package root module should exist");
        let provider_record = index
            .owned_source_indices(module_id)
            .iter()
            .map(|source_id| index.source(*source_id))
            .find(|record| {
                matches!(
                    record.classification(),
                    super::source_tree_index::SourceClassification::ProviderOwned(extension)
                        if extension == &ExternalFileExtension::from("js")
                )
            })
            .expect("helper.js should be a provider-owned record");

        assert_eq!(
            provider_record.ownership(),
            super::source_tree_index::SourceOwnership::Owned(module_id)
        );
    }

    #[cfg(unix)]
    #[test]
    fn package_root_marker_outside_package_root_is_rejected() {
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");
        let external = root.join("external.moth");
        fs::write(&external, "").expect("should write external source");
        let marker = package_root.join("@home.moth");
        symlink(&external, &marker).expect("should create package root marker symlink");

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &package_root);

        let mut string_table = StringTable::new();
        let failure = build_indexes(&source_packages, &mut string_table)
            .expect_err("a package root marker must resolve inside its package boundary");
        assert_invalid_config_reason(
            failure,
            InvalidConfigCode::ModuleRootFileOutsideClaimingModule,
        );
    }
}

/// Linux preparation test for non-UTF-8 direct-child normal module-root candidates.
///
/// Directory, single-file and config compilation all delegate source-package boundary indexing
/// to `build_source_package_boundary_indexes`, so this test covers the shared traversal owner
/// instead of duplicating the same assertion at each orchestration boundary. macOS rejects the
/// invalid-byte fixture before the traversal can inspect it.
#[cfg(target_os = "linux")]
mod non_utf8_package_boundary_candidate_tests {
    use super::*;
    use crate::builder_surface::SourcePackageRegistry;
    use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::PathBuf;

    fn assert_non_utf8_file_error(failure: PreparedFailure) {
        let error = primary_infrastructure_error(failure);
        assert_eq!(
            error.kind,
            InfrastructureFailureKind::Filesystem,
            "non-UTF-8 package boundary candidate should be a File infrastructure error"
        );
        assert!(
            error.message.contains("Non-UTF-8"),
            "error message should mention non-UTF-8: {}",
            error.message,
        );
    }

    fn build_indexes(
        source_packages: &SourcePackageRegistry,
        string_table: &mut StringTable,
    ) -> Result<
        super::source_package_discovery::SourcePackageBoundaryIndexes,
        crate::compiler_frontend::compiler_messages::PreparedFailure,
    > {
        super::source_package_discovery::build_source_package_boundary_indexes(
            source_packages,
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::
                ExternalImportProviderRegistry::default(),
            string_table,
        )
    }

    fn package_with_non_utf8_child() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("should create package temp dir");
        let root = temp.path().to_path_buf();
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");

        let bad_name = OsString::from_vec(vec![0xC3, 0x28]);
        let bad_file = package_root.join(bad_name);
        fs::write(&bad_file, b"").expect("should write non-UTF-8 named file");

        (temp, package_root)
    }

    #[test]
    fn valid_normal_module_root_plus_invalid_candidate_still_returns_file_error() {
        let (root, package_root) = package_with_non_utf8_child();
        fs::write(package_root.join("@home.moth"), b"")
            .expect("should write valid normal module root");

        let mut source_packages = SourcePackageRegistry::new();
        source_packages
            .register_filesystem_root(
                "pkg",
                package_root,
                crate::builder_surface::PackageOrigin::ProjectLocal,
            )
            .expect("test source root should register");

        let mut string_table = StringTable::new();
        let failure = build_indexes(&source_packages, &mut string_table)
            .expect_err("valid root plus invalid candidate should still fail");
        assert_non_utf8_file_error(failure);
        drop(root);
    }
}

/// Module-identity and structural-ancestry tests.
///
/// These tests exercise hidden Stage 0 invariants that integration output cannot inspect:
/// deterministic `ModuleId` ordering by canonical logical path, cosmetic root-filename
/// independence, explicit root roles, structural ancestry and project package facade separation.
mod module_identity_tests {
    use super::module_identity::{ModuleIdentityTable, module_root_role_for_file_name};
    use super::project_module_graph::ProjectModuleGraph;
    use super::*;
    use crate::builder_surface::PackageOrigin;
    use crate::builder_surface::SourcePackageRegistry;
    use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
    use crate::compiler_frontend::semantic_identity::{
        ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
    };
    use std::path::Path;

    fn discover_result(
        root: &std::path::Path,
        entry_root_relative: &str,
    ) -> Result<
        (
            super::source_tree_index::SourceTreeIndex,
            std::path::PathBuf,
            std::path::PathBuf,
        ),
        PreparedFailure,
    > {
        let entry_root = root.join(entry_root_relative);
        fs::create_dir_all(&entry_root).expect("should create entry root");

        let config = Config::new(root.to_path_buf());
        let canonical_root = fs::canonicalize(root).expect("project root should canonicalize");
        let canonical_entry_root =
            fs::canonicalize(&entry_root).expect("entry root should canonicalize");
        let mut string_table = StringTable::new();

        let index = super::source_tree_index::SourceTreeIndex::discover(
            canonical_entry_root.clone(),
            super::source_tree_index::SourceTreeProjectContext {
                project_root: &canonical_root,
                validated_output_settings: None,
            },
            &config,
            &SourcePackageRegistry::default(),
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
            &mut string_table,
        )?;

        Ok((index, canonical_root, canonical_entry_root))
    }

    fn discover_index(
        root: &std::path::Path,
        entry_root_relative: &str,
    ) -> (
        super::source_tree_index::SourceTreeIndex,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        discover_result(root, entry_root_relative).expect("source tree index should build")
    }

    #[cfg(unix)]
    fn assert_root_ownership_rejected(
        failure: PreparedFailure,
        expected_code: crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::InvalidConfigCode,
        expected_paths: &[&Path],
    ) {
        use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::InvalidConfigView;

        let reports = prepared_failure_reports(failure);
        let (report, diagnostic) = reports
            .iter_severity(DiagnosticSeverity::Error)
            .next()
            .expect("root ownership conflicts should retain a diagnostic");
        let facts = diagnostic
            .family_view::<InvalidConfigView>()
            .expect("root ownership conflicts should retain typed config facts");
        assert_eq!(facts.reason, expected_code);
        let strings = report.context().strings();
        let rendered = diagnostic
            .string_ids()
            .iter()
            .map(|id| strings.resolve(*id))
            .collect::<Vec<_>>();
        for expected in expected_paths {
            let expected = diagnostic_path_spelling(expected);
            assert!(
                rendered.iter().any(|candidate| *candidate == expected),
                "diagnostic must name {expected}: {rendered:?}"
            );
        }
    }

    #[cfg(unix)]
    fn assert_aliased_roots_rejected(
        failure: PreparedFailure,
        expected_normal_root: &Path,
        expected_support_root: &Path,
    ) {
        use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::InvalidConfigCode;

        assert_root_ownership_rejected(
            failure,
            InvalidConfigCode::MultipleModuleRootFiles,
            &[expected_normal_root, expected_support_root],
        );
    }

    #[cfg(unix)]
    #[test]
    fn aliased_normal_and_support_root_symlinks_are_rejected() {
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        let target = src.join("actual.moth");
        fs::write(&target, "").expect("should write shared target");
        let normal_root = src.join("@page.moth");
        let support_root = src.join("+package.moth");
        symlink(&target, &normal_root).expect("should create normal root symlink");
        symlink(&target, &support_root).expect("should create support root symlink");

        let failure = discover_result(&root, "src")
            .expect_err("two marker aliases to one physical source must be rejected");
        assert_aliased_roots_rejected(failure, &normal_root, &support_root);
    }

    #[cfg(unix)]
    #[test]
    fn support_root_symlink_to_regular_normal_root_is_rejected() {
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        let normal_root = src.join("@page.moth");
        let support_root = src.join("+package.moth");
        fs::write(&normal_root, "").expect("should write regular normal root");
        symlink(&normal_root, &support_root).expect("should create support root symlink");

        let failure = discover_result(&root, "src")
            .expect_err("a support alias to the normal root must be rejected");
        assert_aliased_roots_rejected(failure, &normal_root, &support_root);
    }

    #[cfg(unix)]
    #[test]
    fn normal_roots_in_different_directories_cannot_alias_one_source() {
        use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::InvalidConfigCode;
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("child")).expect("should create child directory");
        let site_root = src.join("@site.moth");
        let child_root = src.join("child/@child.moth");
        fs::write(&site_root, "").expect("should write site root");
        symlink(&site_root, &child_root).expect("should create child root symlink");

        let failure = discover_result(&root, "src")
            .expect_err("two structural modules must not claim one physical root file");
        assert_root_ownership_rejected(
            failure,
            InvalidConfigCode::AliasedModuleRootFiles,
            &[&site_root, &child_root],
        );
    }

    #[cfg(unix)]
    #[test]
    fn support_and_normal_roots_in_different_directories_cannot_alias_one_source() {
        use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::InvalidConfigCode;
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("child")).expect("should create child directory");
        let site_root = src.join("@site.moth");
        let child_support_root = src.join("child/+package.moth");
        fs::write(&site_root, "").expect("should write site root");
        symlink(&site_root, &child_support_root).expect("should create support root symlink");

        let failure = discover_result(&root, "src")
            .expect_err("a normal root cannot be claimed by another module's support root");
        assert_root_ownership_rejected(
            failure,
            InvalidConfigCode::AliasedModuleRootFiles,
            &[&site_root, &child_support_root],
        );
    }

    #[cfg(unix)]
    #[test]
    fn two_root_aliases_to_one_ordinary_source_are_rejected() {
        use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::InvalidConfigCode;
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("alpha")).expect("should create alpha");
        fs::create_dir_all(src.join("beta")).expect("should create beta");
        let target = src.join("ordinary.moth");
        let alpha_root = src.join("alpha/@alpha.moth");
        let beta_root = src.join("beta/@beta.moth");
        fs::write(&target, "").expect("should write ordinary target");
        symlink(&target, &alpha_root).expect("should create alpha root symlink");
        symlink(&target, &beta_root).expect("should create beta root symlink");

        let failure = discover_result(&root, "src")
            .expect_err("one ordinary source cannot back two module roots");
        assert_root_ownership_rejected(
            failure,
            InvalidConfigCode::AliasedModuleRootFiles,
            &[&alpha_root, &beta_root],
        );
    }

    #[cfg(unix)]
    #[test]
    fn root_symlink_into_another_modules_private_source_is_rejected() {
        use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::InvalidConfigCode;
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("private")).expect("should create private module");
        fs::create_dir_all(src.join("child")).expect("should create child directory");
        fs::write(src.join("@site.moth"), "").expect("should write site root");
        fs::write(src.join("private/@private.moth"), "").expect("should write private root");
        let private_source = src.join("private/source.moth");
        fs::write(&private_source, "").expect("should write private source");
        let child_root = src.join("child/@child.moth");
        symlink(&private_source, &child_root).expect("should create child root symlink");

        let failure = discover_result(&root, "src")
            .expect_err("a root cannot resolve into another module's private source");
        assert_root_ownership_rejected(
            failure,
            InvalidConfigCode::ModuleRootFileOutsideClaimingModule,
            &[&child_root, &src.join("child"), &private_source],
        );
    }

    #[cfg(unix)]
    #[test]
    fn distinct_symlinked_normal_and_support_roots_form_a_paired_module() {
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        let normal_target = src.join("actual.moth");
        fs::write(&normal_target, "").expect("should write normal target");
        let normal_root = src.join("@page.moth");
        let support_root = src.join("+package.moth");
        symlink(&normal_target, &normal_root).expect("should create normal root symlink");
        fs::write(&support_root, "").expect("should write support root");

        let (index, _project_root, entry_root) = discover_index(&root, "src");
        let module_id = index
            .module_identities()
            .module_id_for_directory(&entry_root)
            .expect("the entry root should own a paired structural module");
        let record = index.module_identities().record(module_id);
        assert_eq!(
            record.normal_root_file(),
            Some(fs::canonicalize(&normal_target).unwrap().as_path())
        );
        assert_eq!(
            record.support_root_file(),
            Some(fs::canonicalize(&support_root).unwrap().as_path())
        );
        assert_eq!(record.public_surface_role(), Some(ModuleRootRole::Support));

        let normal_source = index
            .source_index_for_canonical_path(&fs::canonicalize(&normal_target).unwrap())
            .expect("the normal root target should be indexed");
        let support_source = index
            .source_index_for_canonical_path(&fs::canonicalize(&support_root).unwrap())
            .expect("the support root should be indexed");
        assert_eq!(
            index
                .source(normal_source)
                .logical_identity()
                .module_origin()
                .expect("normal source should have an origin")
                .role(),
            ModuleRootRole::Normal
        );
        assert_eq!(
            index
                .source(support_source)
                .logical_identity()
                .module_origin()
                .expect("support source should have an origin")
                .role(),
            ModuleRootRole::Support
        );
    }

    #[cfg(unix)]
    #[test]
    fn root_symlink_retains_the_visible_normal_surface_role() {
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        let target = src.join("actual.moth");
        fs::write(&target, "").expect("should write symlink target");
        symlink(&target, src.join("@home.moth")).expect("should create root symlink");

        let (index, _project_root, entry_root) = discover_index(&root, "src");
        let module_id = index
            .module_identities()
            .module_id_for_directory(&entry_root)
            .expect("the entry root should own a structural module");
        let record = index.module_identities().record(module_id);
        assert_eq!(record.public_surface_role(), Some(ModuleRootRole::Normal));

        let source = index
            .source_index_for_canonical_path(&fs::canonicalize(&target).unwrap())
            .expect("the root target should be indexed");
        let origin = index
            .source(source)
            .logical_identity()
            .module_origin()
            .expect("the root target should have an owned origin");
        assert_eq!(
            origin.role(),
            ModuleRootRole::Normal,
            "root role must follow the visible root entry, not the canonical target filename"
        );
    }

    #[test]
    fn fixed_root_role_combinations_form_one_module_with_role_origins() {
        struct Case {
            roots: &'static [&'static str],
            has_normal: bool,
            has_support: bool,
            has_entry: bool,
            public_surface: Option<ModuleRootRole>,
            is_entry_module: bool,
        }

        let cases = [
            Case {
                roots: &["@mod.moth"],
                has_normal: true,
                has_support: false,
                has_entry: false,
                public_surface: Some(ModuleRootRole::Normal),
                // LEGACY ROOT MIGRATION BRIDGE: a normal root without an entry root still acts
                // as today's combined executable root until the Phase 4 cutover.
                is_entry_module: true,
            },
            Case {
                roots: &["@package.moth"],
                has_normal: false,
                has_support: true,
                has_entry: false,
                public_surface: Some(ModuleRootRole::Support),
                is_entry_module: false,
            },
            Case {
                roots: &["@entry.moth"],
                has_normal: false,
                has_support: false,
                has_entry: true,
                public_surface: None,
                is_entry_module: true,
            },
            Case {
                roots: &["@mod.moth", "@package.moth"],
                has_normal: true,
                has_support: true,
                has_entry: false,
                public_surface: Some(ModuleRootRole::Support),
                // LEGACY ROOT MIGRATION BRIDGE: the normal root is still the combined root.
                is_entry_module: true,
            },
            Case {
                roots: &["@mod.moth", "@entry.moth"],
                has_normal: true,
                has_support: false,
                has_entry: true,
                public_surface: Some(ModuleRootRole::Normal),
                is_entry_module: true,
            },
            Case {
                roots: &["@package.moth", "@entry.moth"],
                has_normal: false,
                has_support: true,
                has_entry: true,
                public_surface: Some(ModuleRootRole::Support),
                is_entry_module: true,
            },
            Case {
                roots: &["@mod.moth", "@package.moth", "@entry.moth"],
                has_normal: true,
                has_support: true,
                has_entry: true,
                public_surface: Some(ModuleRootRole::Support),
                is_entry_module: true,
            },
        ];

        for case in cases {
            let _tmp_root = tempfile::tempdir().expect("should create temp dir");
            let root = _tmp_root.path().to_path_buf();
            let src = root.join("src");
            fs::create_dir_all(&src).expect("should create entry root");
            for root_name in case.roots {
                fs::write(src.join(root_name), "").expect("should write authored root");
            }
            fs::write(src.join("helper.moth"), "").expect("should write shared helper");

            let (index, _project_root, entry_root) = discover_index(&root, "src");
            assert_eq!(
                index.module_identities().module_ids().count(),
                1,
                "one directory with any root-role combination is exactly one structural module"
            );
            let module_id = index
                .module_identities()
                .module_id_for_directory(&entry_root)
                .expect("the rooted directory should own the structural module");
            let record = index.module_identities().record(module_id);
            assert_eq!(
                record.has_normal_root(),
                case.has_normal,
                "{:?}",
                case.roots
            );
            assert_eq!(
                record.has_support_root(),
                case.has_support,
                "{:?}",
                case.roots
            );
            assert_eq!(record.has_entry_root(), case.has_entry, "{:?}", case.roots);
            assert_eq!(
                record.public_surface_role(),
                case.public_surface,
                "{:?}",
                case.roots
            );

            let shared_origin = record.shared_origin().clone();
            let mut present_origins = Vec::new();
            for (role, present) in [
                (ModuleRootRole::Normal, case.has_normal),
                (ModuleRootRole::Support, case.has_support),
                (ModuleRootRole::Entry, case.has_entry),
            ] {
                let origin = record.surface_origin(role);
                if !present {
                    assert!(origin.is_none(), "{role:?} must have no surface origin");
                    continue;
                }
                let origin = origin
                    .expect("a present root role should own a surface origin")
                    .clone();
                assert_eq!(origin.role(), role);
                assert_ne!(
                    origin, shared_origin,
                    "a role-qualified surface origin must stay distinct from the shared helper origin"
                );
                assert!(
                    !present_origins.contains(&origin),
                    "each present root role owns its own surface origin"
                );
                present_origins.push(origin);
            }

            let graph = ProjectModuleGraph::from_source_tree_index(&index);
            assert_eq!(
                graph.entry_modules().contains(&module_id),
                case.is_entry_module,
                "entry-module membership follows an entry root or the legacy combined normal root: {:?}",
                case.roots
            );
        }
    }

    #[test]
    fn config_only_directory_is_not_rooted() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        fs::write(src.join("config.moth"), "project #= (name = \"t\")\n")
            .expect("should write build-only config");

        let (index, _project_root, entry_root) = discover_index(&root, "src");
        assert_eq!(index.module_identities().module_ids().count(), 0);
        assert!(
            index
                .module_identities()
                .module_id_for_directory(&entry_root)
                .is_none(),
            "config.moth never creates a structural module"
        );
    }

    #[test]
    fn duplicate_support_role_roots_are_rejected() {
        use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::{
            InvalidConfigCode, InvalidConfigView,
        };

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        fs::write(src.join("@package.moth"), "").expect("should write canonical support root");
        fs::write(src.join("+package.moth"), "").expect("should write legacy support root");

        let failure = discover_result(&root, "src")
            .expect_err("one directory cannot claim the support role twice");
        let reports = prepared_failure_reports(failure);
        let (_, diagnostic) = reports
            .iter_severity(DiagnosticSeverity::Error)
            .next()
            .expect("duplicate root roles should diagnose");
        let facts = diagnostic
            .family_view::<InvalidConfigView>()
            .expect("duplicate root roles keep typed structure facts");
        assert_eq!(facts.reason, InvalidConfigCode::MultipleModuleRootFiles);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_alias_between_normal_and_entry_roots_is_rejected() {
        use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::{
            InvalidConfigCode, InvalidConfigView,
        };
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        let target = src.join("actual.moth");
        fs::write(&target, "").expect("should write shared physical source");
        symlink(&target, src.join("@mod.moth")).expect("should create normal root symlink");
        symlink(&target, src.join("@entry.moth")).expect("should create entry root symlink");

        let failure = discover_result(&root, "src")
            .expect_err("one physical file cannot claim two root roles");
        let reports = prepared_failure_reports(failure);
        let (_, diagnostic) = reports
            .iter_severity(DiagnosticSeverity::Error)
            .next()
            .expect("aliased root roles should diagnose");
        let facts = diagnostic
            .family_view::<InvalidConfigView>()
            .expect("aliased root roles keep typed structure facts");
        assert_eq!(facts.reason, InvalidConfigCode::MultipleModuleRootFiles);
    }

    #[test]
    fn shared_helper_origin_is_stable_across_root_role_changes() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        fs::write(src.join("helper.moth"), "").expect("should write shared helper");

        fs::write(src.join("@mod.moth"), "").expect("should write normal root");
        let (normal_index, _project_root, entry_root) = discover_index(&root, "src");
        let normal_module = normal_index
            .module_identities()
            .module_id_for_directory(&entry_root)
            .expect("normal root should own the module");
        let helper_source = normal_index
            .source_index_for_canonical_path(&fs::canonicalize(src.join("helper.moth")).unwrap())
            .expect("shared helper should be indexed");
        let helper_origin = normal_index
            .source(helper_source)
            .logical_identity()
            .module_origin()
            .expect("shared helper should own an origin")
            .clone();
        assert_eq!(helper_origin.role(), ModuleRootRole::Shared);
        assert_eq!(
            normal_index
                .module_identities()
                .record(normal_module)
                .shared_origin(),
            &helper_origin
        );
        fs::remove_file(src.join("@mod.moth")).expect("should remove normal root");

        fs::write(src.join("@entry.moth"), "").expect("should write entry root");
        let (entry_index, _project_root, entry_root) = discover_index(&root, "src");
        let entry_module = entry_index
            .module_identities()
            .module_id_for_directory(&entry_root)
            .expect("entry root should own the same structural module");
        let helper_source = entry_index
            .source_index_for_canonical_path(&fs::canonicalize(src.join("helper.moth")).unwrap())
            .expect("shared helper should remain indexed");
        let changed_origin = entry_index
            .source(helper_source)
            .logical_identity()
            .module_origin()
            .expect("shared helper should still own an origin");
        assert_eq!(
            changed_origin, &helper_origin,
            "renaming the authored root must not change the shared helper origin"
        );
        assert_eq!(
            entry_index
                .module_identities()
                .record(entry_module)
                .shared_origin(),
            &helper_origin
        );
    }

    #[cfg(unix)]
    #[test]
    fn root_markers_outside_entry_root_are_rejected() {
        use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::InvalidConfigCode;
        use std::os::unix::fs::symlink;

        for marker_name in ["@page.moth", "+package.moth"] {
            let _tmp_root = tempfile::tempdir().expect("should create temp dir");
            let root = _tmp_root.path().to_path_buf();
            let src = root.join("src");
            fs::create_dir_all(&src).expect("should create entry root");
            let external = root.join("external.moth");
            fs::write(&external, "").expect("should write external source");
            let marker = src.join(marker_name);
            symlink(&external, &marker).expect("should create root marker symlink");

            let failure = discover_result(&root, "src")
                .expect_err("a root marker must resolve inside the selected entry root");
            assert_root_ownership_rejected(
                failure,
                InvalidConfigCode::ModuleRootFileOutsideClaimingModule,
                &[&marker, &src, &external],
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn root_marker_into_hidden_source_subtree_is_rejected() {
        use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::InvalidConfigCode;
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("_hidden")).expect("should create hidden directory");
        let hidden_source = src.join("_hidden/helper.moth");
        fs::write(&hidden_source, "").expect("should write hidden source");
        let marker = src.join("@page.moth");
        symlink(&hidden_source, &marker).expect("should create root marker symlink");

        let failure = discover_result(&root, "src")
            .expect_err("a root marker must not resolve into an excluded source subtree");
        assert_root_ownership_rejected(
            failure,
            InvalidConfigCode::ModuleRootFileOutsideClaimingModule,
            &[&marker, &src, &hidden_source],
        );
    }

    #[cfg(unix)]
    #[test]
    fn root_marker_into_skipped_directory_is_rejected() {
        use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::InvalidConfigCode;
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("target")).expect("should create skipped directory");
        let skipped_source = src.join("target/helper.moth");
        fs::write(&skipped_source, "").expect("should write skipped source");
        let marker = src.join("@page.moth");
        symlink(&skipped_source, &marker).expect("should create root marker symlink");

        let failure = discover_result(&root, "src")
            .expect_err("a root marker must not resolve into a skipped source subtree");
        assert_root_ownership_rejected(
            failure,
            InvalidConfigCode::ModuleRootFileOutsideClaimingModule,
            &[&marker, &src, &skipped_source],
        );
    }

    #[cfg(unix)]
    #[test]
    fn ordinary_source_alias_into_skipped_directory_is_excluded() {
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("target")).expect("should create skipped directory");
        fs::create_dir_all(src.join("helper")).expect("should create colliding folder");
        fs::write(src.join("@home.moth"), "").expect("should write entry root");
        let skipped_source = src.join("target/helper.moth");
        fs::write(&skipped_source, "").expect("should write skipped source");
        symlink(&skipped_source, src.join("helper.moth"))
            .expect("should create ordinary alias into skipped directory");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        assert_eq!(
            index.sources().len(),
            1,
            "the skipped alias must not enter candidate inventory or collide with helper/"
        );
        let canonical_skipped_source =
            fs::canonicalize(&skipped_source).expect("skipped source should canonicalize");
        assert!(
            index
                .sources()
                .iter()
                .all(|source| source.canonical_path() != canonical_skipped_source),
            "a skipped-directory alias must not contribute a source record"
        );
    }

    #[cfg(unix)]
    #[test]
    fn ordinary_symlink_outside_entry_root_remains_excluded() {
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        fs::write(src.join("@home.moth"), "").expect("should write normal root");
        let external = root.join("external.moth");
        fs::write(&external, "").expect("should write external source");
        symlink(&external, src.join("helper.moth")).expect("should create ordinary symlink");

        let (index, _project_root, entry_root) = discover_index(&root, "src");
        assert!(
            index
                .module_identities()
                .module_id_for_directory(&entry_root)
                .is_some(),
            "the valid root should still form a structural module"
        );
        assert!(
            index
                .source_index_for_canonical_path(&fs::canonicalize(&external).unwrap())
                .is_none(),
            "ordinary non-root sources outside the entry root stay silently excluded"
        );
    }

    #[cfg(unix)]
    #[test]
    fn root_marker_to_same_module_unrooted_source_is_accepted() {
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("shared")).expect("should create unrooted helper directory");
        let target = src.join("shared/actual.moth");
        fs::write(&target, "").expect("should write helper source");
        let marker = src.join("@home.moth");
        symlink(&target, &marker).expect("should create root marker symlink");

        let (index, _project_root, entry_root) = discover_index(&root, "src");
        let module_id = index
            .module_identities()
            .module_id_for_directory(&entry_root)
            .expect("the entry root should own a structural module");
        assert_eq!(
            index
                .module_identities()
                .record(module_id)
                .normal_root_file(),
            Some(fs::canonicalize(&target).unwrap().as_path()),
            "a root may alias a source in an unrooted directory of the same module"
        );
    }

    #[test]
    fn directory_named_like_module_root_is_rejected() {
        for marker_name in ["@page.moth", "+package.moth"] {
            let _tmp_root = tempfile::tempdir().expect("should create temp dir");
            let root = _tmp_root.path().to_path_buf();
            let src = root.join("src");
            fs::create_dir_all(src.join(marker_name)).expect("should create marker directory");

            let failure = discover_result(&root, "src")
                .expect_err("a reserved root filename must not be a directory");
            let marker = src.join(marker_name);
            assert_not_regular_file_marker_rejected(
                failure,
                &marker,
                &src,
                &fs::canonicalize(&marker).expect("marker directory should canonicalize"),
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn root_marker_symlink_to_directory_is_rejected() {
        use std::os::unix::fs::symlink;

        for marker_name in ["@page.moth", "+package.moth"] {
            let _tmp_root = tempfile::tempdir().expect("should create temp dir");
            let root = _tmp_root.path().to_path_buf();
            let src = root.join("src");
            let target_directory = src.join("ordinary-directory");
            fs::create_dir_all(&target_directory).expect("should create target directory");
            let marker = src.join(marker_name);
            symlink(&target_directory, &marker).expect("should create directory marker symlink");

            let failure = discover_result(&root, "src")
                .expect_err("a root marker symlink must resolve to a regular file");
            assert_not_regular_file_marker_rejected(
                failure,
                &marker,
                &src,
                &fs::canonicalize(&target_directory).expect("target directory should canonicalize"),
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn socket_named_like_module_root_is_rejected() {
        use std::os::unix::net::UnixListener;

        for marker_name in ["@page.moth", "+package.moth"] {
            let _tmp_root = tempfile::tempdir().expect("should create temp dir");
            let root = _tmp_root.path().to_path_buf();
            let src = root.join("src");
            fs::create_dir_all(&src).expect("should create entry root");
            let marker = src.join(marker_name);
            let _listener =
                UnixListener::bind(&marker).expect("should bind socket with a root-marker name");

            let failure = discover_result(&root, "src")
                .expect_err("a socket root marker must not silently disappear");
            assert_not_regular_file_marker_rejected(
                failure,
                &marker,
                &src,
                &fs::canonicalize(&marker).expect("marker socket should canonicalize"),
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn ordinary_directory_symlink_remains_traversable() {
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("real")).expect("should create real directory");
        fs::write(src.join("@home.moth"), "").expect("should write entry root");
        fs::write(src.join("real/@real.moth"), "").expect("should write nested root");
        symlink(src.join("real"), src.join("alias")).expect("should create directory symlink");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let real_directory =
            fs::canonicalize(src.join("real")).expect("real directory should canonicalize");
        assert!(
            index
                .module_identities()
                .module_id_for_directory(&real_directory)
                .is_some(),
            "an ordinary directory symlink must still reach its nested module"
        );
        let graph = super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&index);
        assert_eq!(
            graph.entry_modules().len(),
            2,
            "the alias spelling must not create a duplicate or malformed module"
        );
    }

    #[cfg(unix)]
    #[test]
    fn dangling_root_marker_symlink_reports_missing_source() {
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        let marker = src.join("@page.moth");
        symlink(src.join("missing.moth"), &marker).expect("should create dangling root symlink");

        let failure = discover_result(&root, "src")
            .expect_err("a dangling root marker must not silently disappear");
        let error = primary_infrastructure_error(failure);
        assert_eq!(
            error.kind,
            InfrastructureFailureKind::Filesystem,
            "a dangling root marker is a filesystem failure"
        );
        assert_eq!(
            error.host_path.as_deref(),
            Some(
                fs::canonicalize(&src)
                    .expect("entry root should canonicalize")
                    .join("@page.moth")
                    .as_path()
            )
        );
        assert!(
            error.message.contains("Failed to canonicalize source path"),
            "error should report the unresolvable root: {}",
            error.message,
        );
    }

    #[test]
    fn assigns_module_ids_in_canonical_logical_path_order() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("zeta")).expect("should create zeta");
        fs::create_dir_all(src.join("alpha")).expect("should create alpha");
        fs::create_dir_all(src.join("alpha/inner")).expect("should create alpha/inner");

        fs::write(src.join("@home.moth"), "").expect("should write entry root");
        fs::write(src.join("zeta/@page.moth"), "").expect("should write zeta root");
        fs::write(src.join("alpha/@mod.moth"), "").expect("should write alpha root");
        fs::write(src.join("alpha/inner/@page.moth"), "").expect("should write inner root");

        let (index, _project_root, entry_root) = discover_index(&root, "src");
        let table = index.module_identities();

        let logical_paths: Vec<&std::path::Path> = table
            .module_ids()
            .map(|id| Path::new(table.record(id).stable_origin().logical_module_path()))
            .collect();

        assert_eq!(
            logical_paths,
            vec![
                std::path::Path::new(""),
                std::path::Path::new("alpha"),
                std::path::Path::new("alpha/inner"),
                std::path::Path::new("zeta"),
            ],
            "ModuleId order should follow canonical logical paths, not traversal order"
        );

        let entry_root_id = table
            .module_id_for_directory(&entry_root)
            .expect("entry root should have a module id");
        assert_eq!(table.record(entry_root_id).role(), ModuleRootRole::Normal);
    }

    #[test]
    fn module_root_role_classifier_maps_filename_markers_to_roles() {
        assert_eq!(
            module_root_role_for_file_name("@page.moth"),
            Some(ModuleRootRole::Normal)
        );
        assert_eq!(
            module_root_role_for_file_name("+pkg.moth"),
            Some(ModuleRootRole::Support)
        );
        assert_eq!(module_root_role_for_file_name("page.moth"), None);
        assert_eq!(module_root_role_for_file_name("config.moth"), None);
        assert_eq!(module_root_role_for_file_name("+.moth"), None);
        assert_eq!(module_root_role_for_file_name("#.moth"), None);
    }

    #[test]
    fn module_identity_is_independent_of_cosmetic_root_filename_suffix() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("page")).expect("should create page module");
        fs::create_dir_all(src.join("other")).expect("should create sibling module");

        fs::write(src.join("@home.moth"), "").expect("should write entry root");
        fs::write(src.join("page/@mod.moth"), "").expect("should write mod-named root");
        fs::write(src.join("other/@page.moth"), "").expect("should write sibling root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let table = index.module_identities();

        let page_dir = fs::canonicalize(src.join("page")).expect("page dir should canonicalize");
        let other_dir = fs::canonicalize(src.join("other")).expect("other dir should canonicalize");
        let page_id = table
            .module_id_for_directory(&page_dir)
            .expect("page module should have an id");
        let other_id = table
            .module_id_for_directory(&other_dir)
            .expect("other module should have an id");

        assert_eq!(
            table.record(page_id).stable_origin().logical_module_path(),
            std::path::Path::new("page")
        );
        // A sibling module is present so ModuleId ordering is non-trivial: the entry root,
        // `other` and `page` receive identities in canonical logical path order.
        assert_ne!(page_id, other_id, "page and other must have distinct ids");

        // Rewrite the same module with a cosmetic @page.moth name and confirm the ModuleId value
        // (not only the logical path text) is unchanged across rediscovery with the sibling
        // still present.
        drop(index);
        fs::remove_file(src.join("page/@mod.moth")).expect("should remove mod root");
        fs::write(src.join("page/@page.moth"), "").expect("should write page-named root");

        let (index_two, _project_root_two, _entry_root_two) = discover_index(&root, "src");
        let table_two = index_two.module_identities();
        let page_id_two = table_two
            .module_id_for_directory(&page_dir)
            .expect("page module should still have an id");
        let other_id_two = table_two
            .module_id_for_directory(&other_dir)
            .expect("other module should still have an id");

        assert_eq!(
            table_two
                .record(page_id_two)
                .stable_origin()
                .logical_module_path(),
            std::path::Path::new("page"),
        );
        assert_eq!(
            page_id_two, page_id,
            "ModuleId must be stable across cosmetic root-filename changes with a sibling present"
        );
        assert_eq!(
            other_id_two, other_id,
            "sibling ModuleId must also be stable across the cosmetic rename"
        );
    }

    #[test]
    fn records_explicit_root_roles_for_normal_and_support_roots() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("page")).expect("should create page module");
        fs::create_dir_all(src.join("components")).expect("should create support module");

        fs::write(src.join("page/@page.moth"), "").expect("should write normal root");
        fs::write(src.join("components/+ui.moth"), "").expect("should write support root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let table = index.module_identities();

        let page_dir = fs::canonicalize(src.join("page")).expect("page dir should canonicalize");
        let support_dir =
            fs::canonicalize(src.join("components")).expect("support dir should canonicalize");

        let page_id = table
            .module_id_for_directory(&page_dir)
            .expect("page module should have an id");
        let support_id = table
            .module_id_for_directory(&support_dir)
            .expect("support module should have an id");

        assert_eq!(table.record(page_id).role(), ModuleRootRole::Normal);
        assert_eq!(table.record(support_id).role(), ModuleRootRole::Support);

        // Only normal roots are entry modules. Assert against the canonical root-file paths via
        // the project module graph (the production entry-classification owner) so the
        // support-root exclusion is genuinely protected, not just a filename-stem check.
        let page_root_file = fs::canonicalize(src.join("page/@page.moth"))
            .expect("page root file should canonicalize");
        let support_root_file = fs::canonicalize(src.join("components/+ui.moth"))
            .expect("support root file should canonicalize");
        let graph = ProjectModuleGraph::from_source_tree_index(&index);
        let entry_root_files: Vec<&std::path::Path> = graph
            .entry_modules()
            .iter()
            .map(|module_id| graph.node(*module_id).root_file())
            .collect();
        assert!(
            entry_root_files.contains(&page_root_file.as_path()),
            "normal root {page_root_file:?} should be an entry module: {entry_root_files:?}"
        );
        assert!(
            !entry_root_files.contains(&support_root_file.as_path()),
            "support root {support_root_file:?} must not be an entry module: {entry_root_files:?}"
        );
    }

    #[test]
    fn records_structural_ancestry_by_nearest_module_containment() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("outer/inner")).expect("should create nested modules");
        fs::write(src.join("@page.moth"), "").expect("should write entry root");
        fs::write(src.join("outer/@mod.moth"), "").expect("should write outer root");
        fs::write(src.join("outer/inner/@page.moth"), "").expect("should write inner root");

        let (index, _project_root, entry_root) = discover_index(&root, "src");
        let table = index.module_identities();

        let outer_dir = fs::canonicalize(src.join("outer")).expect("outer dir should canonicalize");
        let inner_dir =
            fs::canonicalize(src.join("outer/inner")).expect("inner dir should canonicalize");

        let entry_id = table
            .module_id_for_directory(&entry_root)
            .expect("entry root should have an id");
        let outer_id = table
            .module_id_for_directory(&outer_dir)
            .expect("outer module should have an id");
        let inner_id = table
            .module_id_for_directory(&inner_dir)
            .expect("inner module should have an id");

        assert_eq!(table.nearest_ancestor_module(entry_id), None);
        assert_eq!(table.nearest_ancestor_module(outer_id), Some(entry_id));
        assert_eq!(table.nearest_ancestor_module(inner_id), Some(outer_id));

        let entry_children: Vec<_> = table.direct_child_modules(entry_id).to_vec();
        assert!(
            entry_children.contains(&outer_id),
            "outer should be a child of entry root"
        );
        let outer_children: Vec<_> = table.direct_child_modules(outer_id).to_vec();
        assert_eq!(
            outer_children,
            vec![inner_id],
            "inner should be the only child of outer"
        );
        assert!(
            table.direct_child_modules(inner_id).is_empty(),
            "inner should have no children"
        );
    }

    #[test]
    fn support_roots_participate_in_structural_ancestry() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("page/components")).expect("should create modules");
        fs::write(src.join("page/@page.moth"), "").expect("should write normal root");
        fs::write(src.join("page/components/+ui.moth"), "").expect("should write support root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let table = index.module_identities();

        let page_dir = fs::canonicalize(src.join("page")).expect("page dir should canonicalize");
        let support_dir =
            fs::canonicalize(src.join("page/components")).expect("support dir should canonicalize");

        let page_id = table
            .module_id_for_directory(&page_dir)
            .expect("page module should have an id");
        let support_id = table
            .module_id_for_directory(&support_dir)
            .expect("support module should have an id");

        assert_eq!(
            table.nearest_ancestor_module(support_id),
            Some(page_id),
            "support root's nearest ancestor should be the enclosing normal module"
        );
    }

    #[test]
    fn paired_normal_and_support_roots_share_one_structural_module() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        fs::write(src.join("@page.moth"), "").expect("should write normal root");
        fs::write(src.join("+package.moth"), "").expect("should write support root");
        fs::write(src.join("helper.moth"), "").expect("should write shared helper");

        let (index, _project_root, entry_root) = discover_index(&root, "src");
        let table = index.module_identities();
        let module_id = table
            .module_id_for_directory(&entry_root)
            .expect("paired roots should share one structural module id");

        assert_eq!(
            table
                .module_ids()
                .filter(|id| table.record(*id).root_directory() == entry_root)
                .count(),
            1,
            "one rooted directory must produce exactly one structural module"
        );

        let record = table.record(module_id);
        assert_eq!(
            record.normal_root_file(),
            Some(entry_root.join("@page.moth").as_path())
        );
        assert_eq!(
            record.support_root_file(),
            Some(entry_root.join("+package.moth").as_path())
        );
        assert_eq!(record.shared_origin().role(), ModuleRootRole::Shared);
        assert_eq!(
            record
                .surface_origin(ModuleRootRole::Normal)
                .expect("normal surface should exist")
                .role(),
            ModuleRootRole::Normal
        );
        assert_eq!(
            record
                .surface_origin(ModuleRootRole::Support)
                .expect("support surface should exist")
                .role(),
            ModuleRootRole::Support
        );

        let graph = ProjectModuleGraph::from_source_tree_index(&index);
        assert_eq!(graph.node_count(), 1);
        let node = graph.node(module_id);
        assert_eq!(
            node.normal_root_file(),
            Some(entry_root.join("@page.moth").as_path())
        );
        assert_eq!(
            node.support_root_file(),
            Some(entry_root.join("+package.moth").as_path())
        );
        assert_eq!(graph.entry_modules(), &[module_id]);
        assert_eq!(
            index.owned_source_indices(module_id).len(),
            3,
            "the structural module owns both roots and its shared helper exactly once"
        );
    }

    #[test]
    fn rejects_multiple_support_roots_in_one_directory() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("pkg")).expect("should create support directory");
        fs::write(src.join("pkg/+one.moth"), "").expect("should write first support root");
        fs::write(src.join("pkg/+two.moth"), "").expect("should write second support root");

        let entry_root = root.join("src");
        let config = Config::new(root.clone());
        let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
        let canonical_entry_root =
            fs::canonicalize(&entry_root).expect("entry root should canonicalize");
        let mut string_table = StringTable::new();

        let failure = super::source_tree_index::SourceTreeIndex::discover(
            canonical_entry_root,
            super::source_tree_index::SourceTreeProjectContext {
                project_root: &canonical_root,
                validated_output_settings: None,
            },
            &config,
            &SourcePackageRegistry::default(),
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
            &mut string_table,
        )
        .expect_err("multiple support roots should be rejected");
        assert_first_boundary_error_code(failure, "MOTH-CONFIG-0001");
    }

    #[test]
    fn empty_table_has_no_identities_or_ancestry() {
        let table = ModuleIdentityTable::empty();
        assert_eq!(table.module_ids().count(), 0);
    }

    // ---- Stable cross-build origin identity ----

    /// Discover the module identity table for one checkout root with a configured project name.
    fn discover_table_with_name(
        root: &Path,
        entry_root_relative: &str,
        project_name: &str,
    ) -> (ModuleIdentityTable, std::path::PathBuf, std::path::PathBuf) {
        let entry_root = root.join(entry_root_relative);
        fs::create_dir_all(&entry_root).expect("should create entry root");

        let mut config = Config::new(root.to_path_buf());
        config.project_name = String::from(project_name);

        let canonical_root = fs::canonicalize(root).expect("project root should canonicalize");
        let canonical_entry_root =
            fs::canonicalize(&entry_root).expect("entry root should canonicalize");
        let mut string_table = StringTable::new();

        let index = super::source_tree_index::SourceTreeIndex::discover(
            canonical_entry_root.clone(),
            super::source_tree_index::SourceTreeProjectContext {
                project_root: &canonical_root,
                validated_output_settings: None,
            },
            &config,
            &SourcePackageRegistry::default(),
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
            &mut string_table,
        )
        .expect("source tree index should build");

        (
            index.module_identities().clone(),
            canonical_root,
            canonical_entry_root,
        )
    }

    fn entry_module_origin<'a>(
        table: &'a ModuleIdentityTable,
        canonical_entry_root: &Path,
    ) -> &'a StableModuleOriginIdentity {
        let module_id = table
            .module_id_for_directory(canonical_entry_root)
            .expect("entry root should have a module id");
        table.record(module_id).stable_origin()
    }

    /// Equal project name and logical module path yield equal stable identities across two
    /// distinct absolute checkout roots; the identity carries no absolute or ordinary
    /// source-file path.
    #[test]
    fn stable_identity_is_equal_across_distinct_checkout_roots() {
        let _tmp_root_a = tempfile::tempdir().expect("should create temp dir");
        let root_a = _tmp_root_a.path().to_path_buf();
        let _tmp1_root_b = tempfile::tempdir().expect("should create temp dir");
        let root_b = _tmp1_root_b.path().to_path_buf();
        for root in [&root_a, &root_b] {
            let src = root.join("src");
            fs::create_dir_all(src.join("alpha/inner")).expect("should create nested modules");
            fs::write(src.join("@home.moth"), "").expect("should write entry root");
            fs::write(src.join("alpha/@mod.moth"), "").expect("should write alpha root");
            fs::write(src.join("alpha/inner/@page.moth"), "").expect("should write inner root");
        }

        let (table_a, project_a, entry_a) = discover_table_with_name(&root_a, "src", "my-project");
        let (table_b, project_b, entry_b) = discover_table_with_name(&root_b, "src", "my-project");

        let origin_a = entry_module_origin(&table_a, &entry_a);
        let origin_b = entry_module_origin(&table_b, &entry_b);
        assert_eq!(
            origin_a, origin_b,
            "equal project name and logical module path must yield equal identities across distinct absolute checkout roots"
        );

        // Hidden-invariant coverage: the stable identity is self-contained, so its debug
        // representation must not embed either absolute checkout root.
        let debug_a = format!("{origin_a:?}");
        let debug_b = format!("{origin_b:?}");
        assert!(
            !debug_a.contains(project_a.to_str().expect("project_a is UTF-8"))
                && !debug_a.contains(project_b.to_str().expect("project_b is UTF-8")),
            "stable identity debug representation must not contain an absolute checkout root: {debug_a}"
        );
        assert!(
            !debug_b.contains(project_a.to_str().expect("project_a is UTF-8"))
                && !debug_b.contains(project_b.to_str().expect("project_b is UTF-8")),
            "stable identity debug representation must not contain an absolute checkout root: {debug_b}"
        );

        // The nested module identity is equal too, and its logical path is the portable
        // forward-slash spelling rather than an absolute or ordinary source-file path.
        let alpha_a = table_a
            .module_id_for_directory(&entry_a.join("alpha"))
            .expect("alpha should have an id");
        let alpha_b = table_b
            .module_id_for_directory(&entry_b.join("alpha"))
            .expect("alpha should have an id");
        assert_eq!(
            table_a.record(alpha_a).stable_origin(),
            table_b.record(alpha_b).stable_origin(),
            "nested module identity must be equal across checkout roots"
        );
        assert_eq!(
            table_a
                .record(alpha_a)
                .stable_origin()
                .logical_module_path(),
            "alpha",
            "logical module path must be the portable forward-slash spelling"
        );
    }

    #[test]
    fn changing_project_name_changes_stable_identity() {
        let _tmp_root_a = tempfile::tempdir().expect("should create temp dir");
        let root_a = _tmp_root_a.path().to_path_buf();
        let _tmp1_root_b = tempfile::tempdir().expect("should create temp dir");
        let root_b = _tmp1_root_b.path().to_path_buf();
        for root in [&root_a, &root_b] {
            let src = root.join("src");
            fs::create_dir_all(&src).expect("should create entry root");
            fs::write(src.join("@home.moth"), "").expect("should write entry root");
        }

        let (table_a, _project_a, entry_a) = discover_table_with_name(&root_a, "src", "first");
        let (table_b, _project_b, entry_b) = discover_table_with_name(&root_b, "src", "second");

        let origin_a = entry_module_origin(&table_a, &entry_a);
        let origin_b = entry_module_origin(&table_b, &entry_b);
        assert_ne!(
            origin_a, origin_b,
            "changing the project/package name must change the stable identity"
        );
        assert_eq!(origin_a.package().name(), "first");
        assert_eq!(origin_b.package().name(), "second");
        assert_eq!(origin_a.package().origin(), PackageOrigin::ProjectLocal);
    }

    #[test]
    fn changing_logical_module_path_changes_stable_identity() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("alpha")).expect("should create nested module");
        fs::write(src.join("@home.moth"), "").expect("should write entry root");
        fs::write(src.join("alpha/@page.moth"), "").expect("should write alpha root");

        let (table, _project_root, entry_root) =
            discover_table_with_name(&root, "src", "my-project");

        let entry_origin = entry_module_origin(&table, &entry_root);
        let alpha_id = table
            .module_id_for_directory(&entry_root.join("alpha"))
            .expect("alpha should have an id");
        let alpha_origin = table.record(alpha_id).stable_origin();

        assert_ne!(
            entry_origin, alpha_origin,
            "different logical module paths must yield different identities"
        );
        assert_eq!(entry_origin.logical_module_path(), "");
        assert_eq!(alpha_origin.logical_module_path(), "alpha");
    }

    #[test]
    fn changing_root_role_changes_stable_identity() {
        // Both roots share one logical module path; the role-qualified surface origins must stay
        // distinct, and each must be stable across absolute checkout roots.
        let _tmp_root_a = tempfile::tempdir().expect("should create temp dir");
        let root_a = _tmp_root_a.path().to_path_buf();
        let _tmp_root_b = tempfile::tempdir().expect("should create temp dir");
        let root_b = _tmp_root_b.path().to_path_buf();
        for root in [&root_a, &root_b] {
            let src = root.join("src");
            fs::create_dir_all(&src).expect("should create entry root");
            fs::write(src.join("@home.moth"), "").expect("should write normal root");
            fs::write(src.join("+package.moth"), "").expect("should write support root");
        }

        let (table_a, _project_a, entry_a) = discover_table_with_name(&root_a, "src", "my-project");
        let (table_b, _project_b, entry_b) = discover_table_with_name(&root_b, "src", "my-project");
        let record_a = table_a.record(
            table_a
                .module_id_for_directory(&entry_a)
                .expect("paired roots should have an id"),
        );
        let record_b = table_b.record(
            table_b
                .module_id_for_directory(&entry_b)
                .expect("paired roots should have an id"),
        );

        let normal_origin = record_a
            .surface_origin(ModuleRootRole::Normal)
            .expect("normal root should have an origin");
        let support_origin = record_a
            .surface_origin(ModuleRootRole::Support)
            .expect("support root should have an origin");
        let shared_origin = record_a.shared_origin();

        assert_eq!(normal_origin.logical_module_path(), "");
        assert_eq!(support_origin.logical_module_path(), "");
        assert_eq!(shared_origin.logical_module_path(), "");
        assert_eq!(normal_origin.role(), ModuleRootRole::Normal);
        assert_eq!(support_origin.role(), ModuleRootRole::Support);
        assert_eq!(shared_origin.role(), ModuleRootRole::Shared);
        assert_ne!(normal_origin, support_origin);
        assert_ne!(shared_origin, normal_origin);
        assert_ne!(shared_origin, support_origin);

        assert_eq!(
            record_a.surface_origin(ModuleRootRole::Normal),
            record_b.surface_origin(ModuleRootRole::Normal),
            "normal surface identity must be equal across distinct absolute checkout roots"
        );
        assert_eq!(
            record_a.surface_origin(ModuleRootRole::Support),
            record_b.surface_origin(ModuleRootRole::Support),
            "support surface identity must be equal across distinct absolute checkout roots"
        );
        assert_eq!(
            record_a.shared_origin(),
            record_b.shared_origin(),
            "shared helper identity must be equal across distinct absolute checkout roots"
        );
    }

    #[test]
    fn cosmetic_root_suffix_rename_does_not_change_stable_identity() {
        let _temp = tempfile::tempdir().expect("should create temp dir");
        let root_a = _temp.path().to_path_buf();
        let _temp = tempfile::tempdir().expect("should create temp dir");
        let root_b = _temp.path().to_path_buf();
        fs::create_dir_all(root_a.join("src")).expect("should create entry root a");
        fs::create_dir_all(root_b.join("src")).expect("should create entry root b");
        fs::write(root_a.join("src/@page.moth"), "").expect("should write page-named root");
        fs::write(root_b.join("src/@mod.moth"), "").expect("should write mod-named root");

        let (table_a, _project_a, entry_a) = discover_table_with_name(&root_a, "src", "my-project");
        let (table_b, _project_b, entry_b) = discover_table_with_name(&root_b, "src", "my-project");

        assert_eq!(
            entry_module_origin(&table_a, &entry_a),
            entry_module_origin(&table_b, &entry_b),
            "cosmetic root filename suffix rename must not change the stable identity"
        );
    }

    #[test]
    fn project_local_package_identity_preserves_configured_name_verbatim() {
        // The project name is not validated or normalized; the exact configured name is preserved
        // as the stable package name input.
        let identity = StablePackageIdentity::project_local("  weird/name  ");
        assert_eq!(identity.name(), "  weird/name  ");
        assert_eq!(identity.origin(), PackageOrigin::ProjectLocal);
    }

    // ---- Invalid logical-path components are rejected ----

    fn stable_origin_from_path(
        relative: &Path,
    ) -> Result<
        StableModuleOriginIdentity,
        crate::compiler_frontend::compiler_errors::InfrastructureFailure,
    > {
        StableModuleOriginIdentity::from_relative_logical_path(
            StablePackageIdentity::project_local("my-project"),
            relative,
            ModuleRootRole::Normal,
        )
    }

    fn assert_internal_identity_error(
        result: Result<
            StableModuleOriginIdentity,
            crate::compiler_frontend::compiler_errors::InfrastructureFailure,
        >,
        fragment: &str,
    ) {
        let error = result.expect_err("an invalid logical path component must be rejected");
        assert_eq!(
            error.kind,
            InfrastructureFailureKind::Internal,
            "an invalid logical path component must use the internal compiler-error lane"
        );
        assert!(
            error.message.contains(fragment),
            "internal error message should mention `{fragment}`: {}",
            error.message
        );
    }

    #[test]
    fn absolute_logical_path_is_rejected() {
        // An absolute path carries a `RootDir` component, which must not be silently dropped.
        assert_internal_identity_error(
            stable_origin_from_path(Path::new("/alpha")),
            "invalid component",
        );
    }

    #[test]
    fn parent_component_logical_path_is_rejected() {
        // A `..` component must not be silently dropped, otherwise `a/../b` and `b` would collide.
        assert_internal_identity_error(
            stable_origin_from_path(Path::new("../alpha")),
            "invalid component",
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn non_utf8_logical_component_is_rejected() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        // A normal component that is not valid UTF-8 must surface as an internal error rather
        // than panicking. Stage 0's earlier UTF-8 validation makes this an invariant failure,
        // but the constructor stays total.
        let bad = OsString::from_vec(vec![0xC3, 0x28]);
        let relative = Path::new(bad.as_os_str());
        assert_internal_identity_error(stable_origin_from_path(relative), "not UTF-8");
    }

    #[test]
    fn valid_relative_logical_path_still_builds_identity() {
        let identity = stable_origin_from_path(Path::new("alpha/inner"))
            .expect("a normal relative logical path must build a stable identity");
        assert_eq!(identity.logical_module_path(), "alpha/inner");
        assert_eq!(identity.role(), ModuleRootRole::Normal);
    }

    #[test]
    fn synthetic_single_file_origin_is_deterministic_normal_with_empty_path() {
        // Single-file compilation is a synthetic-module mode: it builds one deterministic
        // normal-module origin from the configured project name, the empty logical module path
        // and `ModuleRootRole::Normal`. The empty path is the entry-root spelling and is always
        // valid, so construction never fails. Repeated construction yields the same identity and
        // the identity is independent of any cosmetic root filename or checkout root.
        let origin = StableModuleOriginIdentity::from_relative_logical_path(
            StablePackageIdentity::project_local("my-project"),
            Path::new(""),
            ModuleRootRole::Normal,
        )
        .expect("the empty logical path must build a synthetic single-file origin");
        assert_eq!(origin.logical_module_path(), "");
        assert_eq!(origin.role(), ModuleRootRole::Normal);
        assert_eq!(origin.package().name(), "my-project");

        let again = StableModuleOriginIdentity::from_relative_logical_path(
            StablePackageIdentity::project_local("my-project"),
            Path::new(""),
            ModuleRootRole::Normal,
        )
        .expect("repeated construction must yield the same identity");
        assert_eq!(
            origin, again,
            "synthetic single-file origin must be deterministic"
        );
    }
}
mod owned_source_inventory_tests {
    use super::module_identity::ModuleId;
    use super::source_tree_index::{
        SourceClassification, SourceLogicalIdentity, SourceOwnership, SourceTreeIndex,
    };
    use super::*;
    use crate::builder_surface::external_import_providers::provider::{
        ExternalFileExtension, ExternalImportProvider, ExternalImportProviderContext,
        ExternalImportProviderKind, ExternalImportRequest, ResolvedExternalImport,
    };
    use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
    use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry, SourcePackageRegistry};
    use crate::compiler_frontend::semantic_identity::ModuleRootRole;
    use crate::compiler_frontend::symbols::string_interning::StringTable;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    /// Discover the source tree index for one checkout root with a selected source-kind
    /// registry and configured project name.
    fn discover_index_with_kinds(
        root: &Path,
        entry_root_relative: &str,
        project_name: &str,
        source_file_kinds: &SourceFileKindRegistry,
    ) -> SourceTreeIndex {
        let entry_root = root.join(entry_root_relative);
        fs::create_dir_all(&entry_root).expect("should create entry root");

        let mut config = Config::new(root.to_path_buf());
        config.project_name = String::from(project_name);

        let canonical_root = fs::canonicalize(root).expect("project root should canonicalize");
        let canonical_entry_root =
            fs::canonicalize(&entry_root).expect("entry root should canonicalize");
        let mut string_table = StringTable::new();

        SourceTreeIndex::discover(
            canonical_entry_root,
            super::source_tree_index::SourceTreeProjectContext {
                project_root: &canonical_root,
                validated_output_settings: None,
            },
            &config,
            &SourcePackageRegistry::default(),
            source_file_kinds,
            &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
            &mut string_table,
        )
        .expect("source tree index should build")
    }

    fn html_source_file_kinds() -> SourceFileKindRegistry {
        let mut kinds = SourceFileKindRegistry::new();
        kinds.register("mtf", SourceFileKind::MothTemplate);
        kinds.register("md", SourceFileKind::PlainMarkdown);
        kinds
    }

    fn owned_relative_paths(index: &SourceTreeIndex, module_id: ModuleId) -> Vec<String> {
        index
            .owned_source_indices(module_id)
            .iter()
            .map(|source_id| owned_relative_path(index, *source_id))
            .collect()
    }

    /// Resolve one owned source's portable module-relative path through the central index.
    fn owned_relative_path(
        index: &SourceTreeIndex,
        source_index: super::source_tree_index::SourceRecordIndex,
    ) -> String {
        match index.source(source_index).logical_identity() {
            SourceLogicalIdentity::Owned(identity) => identity.relative_source_path().to_owned(),
            other => {
                panic!("owned source {source_index:?} has a non-owned logical identity: {other:?}")
            }
        }
    }

    /// Resolve the source kinds for one module's owned source IDs through the central index.
    fn owned_kinds(index: &SourceTreeIndex, module_id: ModuleId) -> Vec<SourceFileKind> {
        index
            .owned_source_indices(module_id)
            .iter()
            .filter_map(
                |source_id| match index.source(*source_id).classification() {
                    SourceClassification::CompilerSemantic(kind) => Some(*kind),
                    SourceClassification::ProviderOwned(_) => None,
                },
            )
            .collect()
    }

    /// Resolve the owned source record for one module-relative path, or `None` when absent.
    fn owned_source_for_path<'a>(
        index: &'a SourceTreeIndex,
        module_id: ModuleId,
        relative_path: &str,
    ) -> Option<&'a super::source_tree_index::SourceRecord> {
        index
            .owned_source_indices(module_id)
            .iter()
            .find_map(|source_id| {
                let record = index.source(*source_id);
                match record.logical_identity() {
                    SourceLogicalIdentity::Owned(identity) => {
                        (identity.relative_source_path() == relative_path).then_some(record)
                    }
                    _ => None,
                }
            })
    }

    /// Resolve the portable entry-root-relative logical paths for the unrooted source IDs.
    fn unrooted_logical_paths(index: &SourceTreeIndex) -> Vec<String> {
        index
            .unrooted_source_indices()
            .iter()
            .map(
                |source_id| match index.source(*source_id).logical_identity() {
                    SourceLogicalIdentity::Unrooted(path) => path.as_str().to_owned(),
                    other => panic!(
                        "unrooted source {source_id:?} has a non-unrooted identity: {other:?}"
                    ),
                },
            )
            .collect()
    }

    /// Build a two-module tree: an entry-root module plus a nested `alpha` module with a deeper
    /// `alpha/inner` module.
    fn build_nested_module_tree(root: &Path) {
        let src = root.join("src");
        fs::create_dir_all(src.join("alpha/inner")).expect("should create nested module dirs");
        fs::write(src.join("@page.moth"), "").expect("should write entry root file");
        fs::write(src.join("accounts.moth"), "").expect("should write entry module ordinary file");
        fs::write(src.join("alpha/@mod.moth"), "").expect("should write alpha root file");
        fs::write(src.join("alpha/helper.moth"), "").expect("should write alpha ordinary file");
        fs::write(src.join("alpha/inner/@page.moth"), "").expect("should write inner root file");
        fs::write(src.join("alpha/inner/deep.moth"), "").expect("should write inner ordinary file");
    }

    #[test]
    fn root_and_nested_files_receive_correct_nearest_owner() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        build_nested_module_tree(&root);

        let index =
            discover_index_with_kinds(&root, "src", "my-project", &html_source_file_kinds());
        let table = index.module_identities();

        let entry_id = table
            .module_ids()
            .find(|id| {
                table
                    .record(*id)
                    .stable_origin()
                    .logical_module_path()
                    .is_empty()
            })
            .expect("entry root module should exist");
        let alpha_id = table
            .module_ids()
            .find(|id| table.record(*id).stable_origin().logical_module_path() == "alpha")
            .expect("alpha module should exist");
        let inner_id = table
            .module_ids()
            .find(|id| table.record(*id).stable_origin().logical_module_path() == "alpha/inner")
            .expect("inner module should exist");

        assert_eq!(
            owned_relative_paths(&index, entry_id),
            vec!["@page.moth", "accounts.moth"],
            "entry root module owns its root file and same-module ordinary file"
        );
        assert_eq!(
            owned_relative_paths(&index, alpha_id),
            vec!["@mod.moth", "helper.moth"],
            "alpha module owns its root file and same-module ordinary file"
        );
        assert_eq!(
            owned_relative_paths(&index, inner_id),
            vec!["@page.moth", "deep.moth"],
            "inner module owns its root file and the file beneath it, not alpha"
        );

        // The inner root file and a same-named entry root file keep distinct stable identities.
        let entry_root_identity = owned_source_for_path(&index, entry_id, "@page.moth")
            .expect("entry root owned source should exist")
            .logical_identity()
            .clone();
        let inner_root_identity = owned_source_for_path(&index, inner_id, "@page.moth")
            .expect("inner root owned source should exist")
            .logical_identity()
            .clone();
        assert_ne!(
            entry_root_identity, inner_root_identity,
            "two @page.moth root files in different modules must keep distinct identities"
        );
    }

    #[test]
    fn registered_bd_and_md_kinds_are_included() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        fs::write(src.join("@page.moth"), "").expect("should write root file");
        fs::write(src.join("page.mtf"), "").expect("should write moth template file");
        fs::write(src.join("content.md"), "").expect("should write markdown file");

        let index =
            discover_index_with_kinds(&root, "src", "my-project", &html_source_file_kinds());
        let table = index.module_identities();
        let entry_id = table
            .module_ids()
            .find(|id| {
                table
                    .record(*id)
                    .stable_origin()
                    .logical_module_path()
                    .is_empty()
            })
            .expect("entry root module should exist");

        let kinds: Vec<SourceFileKind> = owned_kinds(&index, entry_id);
        assert!(
            kinds.contains(&SourceFileKind::MothTemplate),
            "registered .mtf files must enter the owned source set: {kinds:?}"
        );
        assert!(
            kinds.contains(&SourceFileKind::PlainMarkdown),
            "registered .md files must enter the owned source set: {kinds:?}"
        );
        assert_eq!(
            owned_relative_paths(&index, entry_id),
            vec!["@page.moth", "content.md", "page.mtf"],
            "registered builder-supported kinds are owned and sorted by relative path"
        );
    }

    #[test]
    fn known_but_unselected_sources_are_indexed_as_unsupported() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        fs::write(src.join("@page.moth"), "").expect("should write root file");
        fs::write(src.join("page.mtf"), "").expect("should write unselected moth template file");
        fs::write(src.join("content.md"), "").expect("should write unselected markdown file");
        fs::write(src.join("notes.txt"), "").expect("should write unknown-extension file");

        // Empty registry: .moth only. .mtf and .md are known-but-unselected; .txt is unknown.
        let index =
            discover_index_with_kinds(&root, "src", "my-project", &SourceFileKindRegistry::new());
        let table = index.module_identities();
        let entry_id = table
            .module_ids()
            .find(|id| {
                table
                    .record(*id)
                    .stable_origin()
                    .logical_module_path()
                    .is_empty()
            })
            .expect("entry root module should exist");

        assert_eq!(
            owned_relative_paths(&index, entry_id),
            vec!["@page.moth", "content.md", "page.mtf"],
            "recognized source kinds remain indexed while unknown extensions stay excluded"
        );
        let unsupported_paths = index
            .owned_source_indices(entry_id)
            .iter()
            .filter(|source_id| !index.source(**source_id).supported())
            .map(|source_id| owned_relative_path(&index, *source_id))
            .collect::<Vec<_>>();
        assert_eq!(unsupported_paths, vec!["content.md", "page.mtf"]);
        assert!(
            index.unrooted_source_indices().is_empty(),
            "excluded files are not unrooted facts"
        );
    }

    #[test]
    fn owned_entries_have_deterministic_logical_order_independent_of_creation() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("internal")).expect("should create internal dir");
        fs::write(src.join("@page.moth"), "").expect("should write root file");
        // Create files in reverse-sorted order so traversal order would differ from logical order.
        fs::write(src.join("zeta.moth"), "").expect("should write zeta");
        fs::write(src.join("alpha.moth"), "").expect("should write alpha");
        fs::write(src.join("internal/whisker.moth"), "").expect("should write whisker");
        fs::write(src.join("internal/beta.moth"), "").expect("should write beta");

        let index =
            discover_index_with_kinds(&root, "src", "my-project", &html_source_file_kinds());
        let table = index.module_identities();
        let entry_id = table
            .module_ids()
            .find(|id| {
                table
                    .record(*id)
                    .stable_origin()
                    .logical_module_path()
                    .is_empty()
            })
            .expect("entry root module should exist");

        assert_eq!(
            owned_relative_paths(&index, entry_id),
            vec![
                "@page.moth",
                "alpha.moth",
                "internal/beta.moth",
                "internal/whisker.moth",
                "zeta.moth"
            ],
            "owned entries must be sorted by portable module-relative path, not creation order"
        );
    }

    #[test]
    fn paired_module_owns_both_roots_and_shared_helpers_once() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        fs::write(src.join("@page.moth"), "").expect("should write normal root file");
        fs::write(src.join("+package.moth"), "").expect("should write support root file");
        fs::write(src.join("helper.moth"), "").expect("should write shared helper");

        let index =
            discover_index_with_kinds(&root, "src", "my-project", &html_source_file_kinds());
        let table = index.module_identities();
        let module_id = table
            .module_ids()
            .next()
            .expect("paired roots should define one structural module");

        assert_eq!(
            owned_relative_paths(&index, module_id),
            vec!["@page.moth", "+package.moth", "helper.moth"],
            "one structural module owns both roots and its shared helper exactly once"
        );

        let root_origins = index
            .owned_source_indices(module_id)
            .iter()
            .map(|source_id| {
                let record = index.source(*source_id);
                assert_eq!(
                    record.classification(),
                    &SourceClassification::CompilerSemantic(SourceFileKind::Moth)
                );
                let SourceLogicalIdentity::Owned(identity) = record.logical_identity() else {
                    panic!("an owned source must carry an owned logical identity");
                };
                identity.module_origin().role()
            })
            .collect::<Vec<_>>();
        assert!(
            root_origins.contains(&ModuleRootRole::Normal),
            "the normal root source must carry the normal surface origin: {root_origins:?}"
        );
        assert!(
            root_origins.contains(&ModuleRootRole::Support),
            "the support root source must carry the support surface origin: {root_origins:?}"
        );
        assert!(
            root_origins.contains(&ModuleRootRole::Shared),
            "the helper source must carry the role-neutral shared origin: {root_origins:?}"
        );
    }

    #[test]
    fn unrooted_supported_candidates_remain_explicit_facts() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        // No module root file in the entry root: the .moth files are unrooted.
        fs::write(src.join("orphan.moth"), "").expect("should write orphan source");
        fs::write(src.join("page.mtf"), "").expect("should write orphan moth template");

        let index =
            discover_index_with_kinds(&root, "src", "my-project", &html_source_file_kinds());

        // No modules were discovered, so no owned source IDs and no silent discard.
        assert_eq!(
            index.module_identities().module_ids().count(),
            0,
            "unrooted candidates must not be assigned to a module"
        );
        let unrooted = index.unrooted_source_indices();
        assert_eq!(
            unrooted.len(),
            2,
            "both supported unrooted files must remain explicit facts"
        );
        // Unrooted records are sorted by portable logical path.
        let unrooted_paths = unrooted_logical_paths(&index);
        assert!(
            unrooted_paths[0] < unrooted_paths[1],
            "unrooted candidates must sort by portable logical path"
        );
        assert_eq!(
            unrooted_paths[0], "orphan.moth",
            "the logical candidate path is entry-root-relative and portable"
        );
    }

    #[test]
    fn owned_source_identity_is_independent_of_checkout_root() {
        let _tmp_root_a = tempfile::tempdir().expect("should create temp dir");
        let root_a = _tmp_root_a.path().to_path_buf();
        let _tmp1_root_b = tempfile::tempdir().expect("should create temp dir");
        let root_b = _tmp1_root_b.path().to_path_buf();
        for root in [&root_a, &root_b] {
            build_nested_module_tree(root);
        }

        let index_a =
            discover_index_with_kinds(&root_a, "src", "my-project", &html_source_file_kinds());
        let index_b =
            discover_index_with_kinds(&root_b, "src", "my-project", &html_source_file_kinds());
        let table_a = index_a.module_identities();
        let table_b = index_b.module_identities();

        let alpha_a = table_a
            .module_ids()
            .find(|id| table_a.record(*id).stable_origin().logical_module_path() == "alpha")
            .expect("alpha module should exist in tree a");
        let alpha_b = table_b
            .module_ids()
            .find(|id| table_b.record(*id).stable_origin().logical_module_path() == "alpha")
            .expect("alpha module should exist in tree b");

        let helper_a = owned_source_for_path(&index_a, alpha_a, "helper.moth")
            .expect("alpha helper owned source should exist in tree a")
            .logical_identity()
            .clone();
        let helper_b = owned_source_for_path(&index_b, alpha_b, "helper.moth")
            .expect("alpha helper owned source should exist in tree b")
            .logical_identity()
            .clone();

        assert_eq!(
            helper_a, helper_b,
            "owned-source identity must be equal across distinct checkout roots"
        );
        // The identity debug representation must not embed either absolute checkout root.
        let debug = format!("{:?}", helper_a);
        assert!(
            !debug.contains(root_a.to_str().expect("root_a is UTF-8"))
                && !debug.contains(root_b.to_str().expect("root_b is UTF-8")),
            "owned-source identity must not embed an absolute checkout root: {debug}"
        );
    }

    #[test]
    fn unknown_registered_extension_is_excluded() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        fs::write(src.join("@page.moth"), "").expect("should write root file");
        fs::write(src.join("notes.txt"), "").expect("should write unknown-extension file");

        // Registering txt -> Moth template must not admit .txt: it is not a compiler-recognized
        // extension, so it stays out of owned source sets regardless of the registry entry.
        let mut kinds = SourceFileKindRegistry::new();
        kinds.register("txt", SourceFileKind::MothTemplate);
        let index = discover_index_with_kinds(&root, "src", "my-project", &kinds);
        let table = index.module_identities();
        let entry_id = table
            .module_ids()
            .find(|id| {
                table
                    .record(*id)
                    .stable_origin()
                    .logical_module_path()
                    .is_empty()
            })
            .expect("entry root module should exist");

        assert_eq!(
            owned_relative_paths(&index, entry_id),
            vec!["@page.moth"],
            "an arbitrary registered unknown extension must not enter owned source sets"
        );
        assert!(
            index.unrooted_source_indices().is_empty(),
            "an excluded unknown registered extension is not an unrooted fact"
        );
    }

    #[test]
    fn mismatched_known_extension_mapping_is_indexed_as_unsupported() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        fs::write(src.join("@page.moth"), "").expect("should write root file");
        fs::write(src.join("page.mtf"), "").expect("should write moth-template-extension file");

        // Registering mtf -> PlainMarkdown mismatches the compiler-recognized mapping (mtf ->
        // MothTemplate), so .mtf remains indexed but cannot become a prepared semantic source.
        let mut kinds = SourceFileKindRegistry::new();
        kinds.register("mtf", SourceFileKind::PlainMarkdown);
        let index = discover_index_with_kinds(&root, "src", "my-project", &kinds);
        let table = index.module_identities();
        let entry_id = table
            .module_ids()
            .find(|id| {
                table
                    .record(*id)
                    .stable_origin()
                    .logical_module_path()
                    .is_empty()
            })
            .expect("entry root module should exist");

        assert_eq!(
            owned_relative_paths(&index, entry_id),
            vec!["@page.moth", "page.mtf"],
            "recognized source identity is independent of active builder support"
        );
        let page_source_id = index
            .owned_source_indices(entry_id)
            .iter()
            .copied()
            .find(|source_id| owned_relative_path(&index, *source_id) == "page.mtf")
            .expect("recognized Moth template should have a source ID");
        assert!(!index.source(page_source_id).supported());
        assert!(
            index.unrooted_source_indices().is_empty(),
            "an excluded mismatched mapping is not an unrooted fact"
        );
    }

    #[test]
    fn unrooted_source_ids_are_ordered_by_portable_logical_path_across_roots() {
        // Two distinct checkout roots with unrooted files created in reverse-logical order.
        // The unrooted source IDs must sort by portable entry-root-relative logical path,
        // not by absolute checkout path or creation order.
        let _tmp_root_a = tempfile::tempdir().expect("should create temp dir");
        let root_a = _tmp_root_a.path().to_path_buf();
        let _tmp1_root_b = tempfile::tempdir().expect("should create temp dir");
        let root_b = _tmp1_root_b.path().to_path_buf();

        let build_tree = |root: &Path| {
            let src = root.join("src");
            fs::create_dir_all(src.join("zebra")).expect("should create zebra dir");
            fs::create_dir_all(src.join("alpha")).expect("should create alpha dir");
            // No module root: all files are unrooted. Create in reverse-logical order.
            fs::write(src.join("zebra/orphan.moth"), "").expect("should write zebra orphan");
            fs::write(src.join("alpha/orphan.moth"), "").expect("should write alpha orphan");
            fs::write(src.join("mismatch.moth"), "").expect("should write mismatch orphan");
        };
        build_tree(&root_a);
        build_tree(&root_b);

        let index_a =
            discover_index_with_kinds(&root_a, "src", "my-project", &html_source_file_kinds());
        let index_b =
            discover_index_with_kinds(&root_b, "src", "my-project", &html_source_file_kinds());

        let paths_a = unrooted_logical_paths(&index_a);
        let paths_b = unrooted_logical_paths(&index_b);

        assert_eq!(
            paths_a,
            vec!["alpha/orphan.moth", "mismatch.moth", "zebra/orphan.moth"],
            "unrooted candidates must sort by portable logical path, not creation order"
        );
        assert_eq!(
            paths_a, paths_b,
            "unrooted logical ordering must be identical across distinct checkout roots"
        );
    }

    #[test]
    fn paired_root_files_appear_exactly_once_in_the_central_source_table() {
        let _temp = tempfile::tempdir().expect("should create temp dir");
        let root = _temp.path().to_path_buf();

        fs::write(root.join("@page.moth"), "").expect("should write normal root file");
        fs::write(root.join("+package.moth"), "").expect("should write support root file");

        let index = discover_index_with_kinds(&root, ".", "my-project", &html_source_file_kinds());
        let table = index.module_identities();
        let module_id = table
            .module_ids()
            .next()
            .expect("paired roots should define one structural module");
        let record = table.record(module_id);

        for (relative_path, expected_origin) in [
            ("@page.moth", record.surface_origin(ModuleRootRole::Normal)),
            (
                "+package.moth",
                record.surface_origin(ModuleRootRole::Support),
            ),
        ] {
            let matching = index
                .sources()
                .iter()
                .filter(|source| match source.logical_identity() {
                    SourceLogicalIdentity::Owned(identity) => {
                        identity.relative_source_path() == relative_path
                            && Some(identity.module_origin()) == expected_origin
                    }
                    SourceLogicalIdentity::Unrooted(_) => false,
                })
                .count();
            assert_eq!(
                matching, 1,
                "{relative_path} must appear exactly once with its surface origin"
            );
        }
    }

    #[test]
    fn compilation_root_table_selects_the_normal_root_for_a_paired_directory() {
        let _temp = tempfile::tempdir().expect("should create temp dir");
        let root = _temp.path().to_path_buf();

        fs::write(root.join("@page.moth"), "").expect("should write normal root file");
        fs::write(root.join("+package.moth"), "").expect("should write support root file");

        let index = discover_index_with_kinds(&root, ".", "my-project", &html_source_file_kinds());
        let table = index.module_identities();
        let compilation_roots = table.derive_compilation_root_table();
        let canonical_root = fs::canonicalize(&root).expect("test root should canonicalize");

        assert_eq!(
            compilation_roots.api_facade_root_file_for_directory(&canonical_root),
            Some(canonical_root.join("@page.moth").as_path()),
            "the normal module remains the primary compilation root for the shared directory"
        );
    }

    #[test]
    fn source_row_handles_address_their_own_record() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        build_nested_module_tree(&root);
        let index =
            discover_index_with_kinds(&root, "src", "my-project", &html_source_file_kinds());

        // Every Stage 0 consumer keys parallel ownership and reachability data by this handle and
        // then reads the record back through `source`. A handle that drifted from its row would
        // silently return a different file's metadata rather than fail.
        for record in index.sources() {
            let handle = index
                .source_index_for_canonical_path(record.canonical_path())
                .expect("every classified source must be reachable by its canonical path");
            assert_eq!(
                index.source(handle).canonical_path(),
                record.canonical_path(),
                "a row handle must address the record it was resolved from"
            );
        }
    }

    #[test]
    fn every_supported_file_appears_exactly_once() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        build_nested_module_tree(&root);
        let index =
            discover_index_with_kinds(&root, "src", "my-project", &html_source_file_kinds());

        let mut referenced_ids = Vec::new();
        for module_id in index.module_identities().module_ids() {
            referenced_ids.extend(
                index
                    .owned_source_indices(module_id)
                    .iter()
                    .map(|source_id| source_id.index()),
            );
        }
        referenced_ids.extend(
            index
                .unrooted_source_indices()
                .iter()
                .map(|source_id| source_id.index()),
        );
        referenced_ids.sort_unstable();
        assert_eq!(
            referenced_ids,
            (0..index.sources().len()).collect::<Vec<_>>(),
            "every source record must appear exactly once in one owned set or the unrooted list"
        );

        // Canonical paths are unique across the whole table: no file is recorded twice.
        let mut paths: Vec<PathBuf> = index
            .sources()
            .iter()
            .map(|record| record.canonical_path().to_path_buf())
            .collect();
        let total = paths.len();
        paths.sort();
        paths.dedup();
        assert_eq!(
            paths.len(),
            total,
            "no canonical path may appear in two source records"
        );
    }

    #[test]
    fn owned_and_unrooted_sets_reference_valid_records_with_matching_state() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        build_nested_module_tree(&root);
        let index =
            discover_index_with_kinds(&root, "src", "my-project", &html_source_file_kinds());

        for module_id in index.module_identities().module_ids() {
            for source_id in index.owned_source_indices(module_id) {
                let record = index.source(*source_id);
                assert_eq!(
                    record.ownership(),
                    SourceOwnership::Owned(module_id),
                    "owned source ID {} must reference a record owned by its module",
                    source_id.index()
                );
                assert!(
                    matches!(record.logical_identity(), SourceLogicalIdentity::Owned(_)),
                    "an owned source record must carry an owned logical identity"
                );
            }
        }

        let _tmp1_unrooted_root = tempfile::tempdir().expect("should create temp dir");
        let unrooted_root = _tmp1_unrooted_root.path().to_path_buf();
        let unrooted_src = unrooted_root.join("src");
        fs::create_dir_all(&unrooted_src).expect("should create unrooted source directory");
        fs::write(unrooted_src.join("orphan.moth"), "").expect("should write unrooted source");
        let unrooted_index = discover_index_with_kinds(
            &unrooted_root,
            "src",
            "my-project",
            &html_source_file_kinds(),
        );
        assert_eq!(
            unrooted_index.unrooted_source_indices().len(),
            1,
            "the focused unrooted tree must exercise one unrooted record"
        );
        for source_id in unrooted_index.unrooted_source_indices() {
            let record = unrooted_index.source(*source_id);
            assert_eq!(
                record.ownership(),
                SourceOwnership::Unrooted,
                "unrooted source ID {} must reference an unrooted record",
                source_id.index()
            );
            assert!(
                matches!(
                    record.logical_identity(),
                    SourceLogicalIdentity::Unrooted(_)
                ),
                "an unrooted source record must carry an unrooted logical identity"
            );
        }
    }

    #[test]
    fn source_ids_are_deterministic_across_creation_order_and_checkout_roots() {
        // The same logical tree built in two distinct checkout roots with reverse creation order
        // must assign identical SourceId -> logical identity mappings, because SourceIds are
        // derived from portable logical identity, not traversal or absolute paths.
        let _tmp_root_a = tempfile::tempdir().expect("should create temp dir");
        let root_a = _tmp_root_a.path().to_path_buf();
        let _tmp1_root_b = tempfile::tempdir().expect("should create temp dir");
        let root_b = _tmp1_root_b.path().to_path_buf();

        let build_tree_reverse = |root: &Path| {
            let src = root.join("src");
            fs::create_dir_all(src.join("alpha/inner")).expect("should create nested dirs");
            // Reverse creation order relative to logical path order.
            fs::write(src.join("alpha/inner/deep.moth"), "").expect("should write deep");
            fs::write(src.join("alpha/inner/@page.moth"), "").expect("should write inner root");
            fs::write(src.join("alpha/helper.moth"), "").expect("should write alpha helper");
            fs::write(src.join("alpha/@mod.moth"), "").expect("should write alpha root");
            fs::write(src.join("accounts.moth"), "").expect("should write accounts");
            fs::write(src.join("@page.moth"), "").expect("should write entry root");
        };
        let build_tree_forward = |root: &Path| {
            let src = root.join("src");
            fs::create_dir_all(src.join("alpha/inner")).expect("should create nested dirs");
            fs::write(src.join("@page.moth"), "").expect("should write entry root");
            fs::write(src.join("accounts.moth"), "").expect("should write accounts");
            fs::write(src.join("alpha/@mod.moth"), "").expect("should write alpha root");
            fs::write(src.join("alpha/helper.moth"), "").expect("should write alpha helper");
            fs::write(src.join("alpha/inner/@page.moth"), "").expect("should write inner root");
            fs::write(src.join("alpha/inner/deep.moth"), "").expect("should write deep");
        };
        build_tree_reverse(&root_a);
        build_tree_forward(&root_b);

        let index_a =
            discover_index_with_kinds(&root_a, "src", "my-project", &html_source_file_kinds());
        let index_b =
            discover_index_with_kinds(&root_b, "src", "my-project", &html_source_file_kinds());

        let identities_a: Vec<SourceLogicalIdentity> = index_a
            .sources()
            .iter()
            .map(|record| record.logical_identity().clone())
            .collect();
        let identities_b: Vec<SourceLogicalIdentity> = index_b
            .sources()
            .iter()
            .map(|record| record.logical_identity().clone())
            .collect();
        assert_eq!(
            identities_a, identities_b,
            "SourceId -> logical identity mapping must be identical across creation order and \
             checkout roots"
        );

        // The logical identity debug representation must not embed either absolute checkout root.
        let root_a_str = root_a.to_str().expect("root_a is UTF-8");
        let root_b_str = root_b.to_str().expect("root_b is UTF-8");
        for identity in &identities_a {
            let identity_debug = format!("{identity:?}");
            assert!(
                !identity_debug.contains(root_a_str) && !identity_debug.contains(root_b_str),
                "source logical identity must not embed an absolute checkout root: {identity_debug}"
            );
        }
    }

    /// A minimal external import provider that supports `.js` files and declines every request.
    /// The classification test only needs the registry to recognise the extension.
    #[derive(Debug)]
    struct JsOnlyProvider {
        extensions: Vec<ExternalFileExtension>,
    }

    impl JsOnlyProvider {
        fn new() -> Self {
            Self {
                extensions: vec![ExternalFileExtension::from("js")],
            }
        }
    }

    impl ExternalImportProvider for JsOnlyProvider {
        fn kind(&self) -> ExternalImportProviderKind {
            ExternalImportProviderKind::new("js-only")
        }

        fn supported_extensions(&self) -> &[ExternalFileExtension] {
            &self.extensions
        }

        fn resolve_external_import(
            &self,
            _request: ExternalImportRequest,
            _context: &mut ExternalImportProviderContext,
        ) -> Result<Option<ResolvedExternalImport>, PreparedFailure> {
            Ok(None)
        }
    }

    /// Discover the source tree index with a `.js` provider registered so provider-owned files
    /// enter the central source table.
    fn discover_index_with_js_provider(
        root: &Path,
        entry_root_relative: &str,
        project_name: &str,
    ) -> SourceTreeIndex {
        let entry_root = root.join(entry_root_relative);
        let mut config = Config::new(root.to_path_buf());
        config.project_name = String::from(project_name);

        let canonical_root = fs::canonicalize(root).expect("project root should canonicalize");
        let canonical_entry_root =
            fs::canonicalize(&entry_root).expect("entry root should canonicalize");
        let mut string_table = StringTable::new();
        let mut providers = ExternalImportProviderRegistry::empty();
        providers.register(Arc::new(JsOnlyProvider::new()));

        SourceTreeIndex::discover(
            canonical_entry_root,
            super::source_tree_index::SourceTreeProjectContext {
                project_root: &canonical_root,
                validated_output_settings: None,
            },
            &config,
            &SourcePackageRegistry::default(),
            &SourceFileKindRegistry::default(),
            &providers,
            &mut string_table,
        )
        .expect("source tree index should build with a js provider")
    }

    #[cfg(unix)]
    #[test]
    fn provider_owned_alias_into_skipped_directory_is_excluded() {
        use std::os::unix::fs::symlink;

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("target")).expect("should create skipped directory");
        fs::create_dir_all(src.join("helper")).expect("should create colliding folder");
        fs::write(src.join("@page.moth"), "").expect("should write entry root");
        let skipped_source = src.join("target/helper.js");
        fs::write(&skipped_source, "").expect("should write skipped provider source");
        symlink(&skipped_source, src.join("helper.js"))
            .expect("should create provider alias into skipped directory");

        let index = discover_index_with_js_provider(&root, "src", "provider-project");
        assert_eq!(
            index.sources().len(),
            1,
            "the provider-owned skipped alias must not enter inventory or collide with helper/"
        );
        let canonical_skipped_source =
            fs::canonicalize(&skipped_source).expect("skipped source should canonicalize");
        assert!(
            index
                .sources()
                .iter()
                .all(|source| source.canonical_path() != canonical_skipped_source),
            "a skipped provider alias must not contribute a source record"
        );
    }

    #[test]
    fn provider_owned_files_are_classified_and_owned_by_nearest_module() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        let feature = src.join("feature");
        fs::create_dir_all(&feature).expect("should create feature module");
        fs::write(src.join("@page.moth"), "").expect("should write entry root");
        fs::write(src.join("helper.js"), "").expect("should write entry provider file");
        fs::write(feature.join("@mod.moth"), "").expect("should write feature root");
        fs::write(feature.join("util.js"), "").expect("should write feature provider file");

        let index = discover_index_with_js_provider(&root, "src", "provider-project");
        let table = index.module_identities();

        let entry_id = table
            .module_ids()
            .find(|id| {
                table
                    .record(*id)
                    .stable_origin()
                    .logical_module_path()
                    .is_empty()
            })
            .expect("entry root module should exist");
        let feature_id = table
            .module_ids()
            .find(|id| table.record(*id).stable_origin().logical_module_path() == "feature")
            .expect("feature module should exist");

        // The entry module owns both its compiler-semantic root and its provider-owned helper.
        let page_record =
            owned_source_for_path(&index, entry_id, "@page.moth").expect("page root is indexed");
        assert_eq!(
            page_record.classification(),
            &SourceClassification::CompilerSemantic(SourceFileKind::Moth),
            "compiler semantic files keep their SourceFileKind classification"
        );

        let helper_record =
            owned_source_for_path(&index, entry_id, "helper.js").expect("helper.js is indexed");
        assert_eq!(
            helper_record.classification(),
            &SourceClassification::ProviderOwned(ExternalFileExtension::from("js")),
            "provider-owned files are classified as ProviderOwned with their extension"
        );
        assert_eq!(
            helper_record.ownership(),
            SourceOwnership::Owned(entry_id),
            "provider-owned files are owned by their nearest module"
        );

        // The feature module owns its provider-owned file separately.
        let util_record =
            owned_source_for_path(&index, feature_id, "util.js").expect("util.js is indexed");
        assert_eq!(
            util_record.classification(),
            &SourceClassification::ProviderOwned(ExternalFileExtension::from("js"))
        );
        assert_eq!(
            util_record.ownership(),
            SourceOwnership::Owned(feature_id),
            "nested-module provider files are owned by the nested module"
        );

        let helper_index = index
            .source_index_for_canonical_path(helper_record.canonical_path())
            .expect("helper canonical path resolves to a source row");
        let util_index = index
            .source_index_for_canonical_path(util_record.canonical_path())
            .expect("util canonical path resolves to a source row");

        // The canonical-path lookup map resolves a consumer to its owning record.
        let page_canonical = page_record.canonical_path();
        let page_lookup_index = index
            .source_index_for_canonical_path(page_canonical)
            .expect("page canonical path resolves to a source row");
        assert_eq!(
            index.source(page_lookup_index).ownership(),
            SourceOwnership::Owned(entry_id)
        );

        // Source rows follow deterministic portable logical identity order. The root module
        // precedes the child module, so helper.js precedes feature/util.js regardless of traversal
        // or creation order.
        assert!(
            helper_index.index() < util_index.index(),
            "provider source rows must follow deterministic logical identity order"
        );
    }

    #[test]
    fn provider_owned_files_without_a_registered_provider_are_not_indexed() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        fs::write(src.join("@page.moth"), "").expect("should write entry root");
        fs::write(src.join("drawing.wit"), "").expect("should write unregistered provider file");

        // No provider registered for `.wit`, so the file must not enter the inventory.
        let index = discover_index_with_kinds(
            &root,
            "src",
            "unregistered",
            &SourceFileKindRegistry::default(),
        );
        let table = index.module_identities();
        let entry_id = table
            .module_ids()
            .find(|id| {
                table
                    .record(*id)
                    .stable_origin()
                    .logical_module_path()
                    .is_empty()
            })
            .expect("entry root module should exist");

        assert!(
            owned_source_for_path(&index, entry_id, "drawing.wit").is_none(),
            "files whose extension has no registered provider must not be indexed"
        );
    }
}

// ---- Canonical structural project module graph ----

mod project_module_graph_tests {
    use super::*;
    use crate::builder_surface::SourcePackageRegistry;
    use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
    use crate::compiler_frontend::semantic_identity::ModuleRootRole;
    use crate::compiler_frontend::symbols::string_interning::StringTable;

    use super::module_identity::ModuleId;
    use super::project_module_graph::{DependencyEdgeOutcome, ProjectModuleGraph};
    use super::source_tree_index::SourceTreeIndex;
    use crate::compiler_frontend::compiler_messages::CompilerStage;
    use crate::compiler_frontend::compiler_messages::compiler_bug::CompilerBugReport;

    fn discover_index(
        root: &std::path::Path,
        entry_root_relative: &str,
    ) -> (SourceTreeIndex, std::path::PathBuf, std::path::PathBuf) {
        let entry_root = root.join(entry_root_relative);
        fs::create_dir_all(&entry_root).expect("should create entry root");

        let config = Config::new(root.to_path_buf());
        let canonical_root = fs::canonicalize(root).expect("project root should canonicalize");
        let canonical_entry_root =
            fs::canonicalize(&entry_root).expect("entry root should canonicalize");
        let mut string_table = StringTable::new();

        let index = SourceTreeIndex::discover(
            canonical_entry_root.clone(),
            super::source_tree_index::SourceTreeProjectContext {
                project_root: &canonical_root,
                validated_output_settings: None,
            },
            &config,
            &SourcePackageRegistry::default(),
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
            &mut string_table,
        )
        .expect("source tree index should build");

        (index, canonical_root, canonical_entry_root)
    }

    /// Find the `ModuleId` whose identity table record has the given role and logical path.
    fn module_id_for(
        index: &SourceTreeIndex,
        role: ModuleRootRole,
        logical_path: &str,
    ) -> ModuleId {
        let table = index.module_identities();
        table
            .module_ids()
            .find(|id| {
                table.record(*id).role() == role
                    && table.record(*id).stable_origin().logical_module_path() == logical_path
            })
            .unwrap_or_else(|| {
                panic!("expected a {role:?} module with logical path {logical_path:?}")
            })
    }

    #[test]
    fn nodes_are_stored_in_deterministic_module_id_order() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("zeta")).expect("should create zeta");
        fs::create_dir_all(src.join("alpha")).expect("should create alpha");

        fs::write(src.join("@home.moth"), "").expect("should write entry root");
        fs::write(src.join("zeta/@page.moth"), "").expect("should write zeta root");
        fs::write(src.join("alpha/@mod.moth"), "").expect("should write alpha root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let graph = ProjectModuleGraph::from_source_tree_index(&index);

        let graph_ids: Vec<ModuleId> = graph.nodes().iter().map(|node| node.module_id()).collect();
        let table_ids: Vec<ModuleId> = index.module_identities().module_ids().collect();

        assert_eq!(
            graph_ids, table_ids,
            "graph node order must match deterministic ModuleId order from the identity table"
        );

        for (graph_node, table_id) in graph.nodes().iter().zip(table_ids.iter().copied()) {
            let record = index.module_identities().record(table_id);
            assert_eq!(graph_node.module_id(), table_id);
            assert_eq!(graph_node.role(), record.role());
            assert_eq!(graph_node.stable_origin(), record.stable_origin());
            assert_eq!(
                graph_node.root_file(),
                record
                    .normal_root_file()
                    .or_else(|| record.support_root_file())
                    .expect("a structural record has at least one root file")
            );
            // The graph node carries no source records: owned source data lives in the central
            // index. Verify the index's owned source IDs for this module all resolve to records
            // owned by this module, proving the graph delegates ownership to the index.
            for source_id in index.owned_source_indices(table_id) {
                let record = index.source(*source_id);
                assert_eq!(
                    record.ownership(),
                    super::source_tree_index::SourceOwnership::Owned(table_id),
                    "each owned source ID must resolve to a record owned by its module"
                );
            }
        }
    }

    #[test]
    fn entry_modules_are_normal_only_and_support_is_separate() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("pages")).expect("should create pages");
        fs::create_dir_all(src.join("components")).expect("should create components");

        fs::write(src.join("@site.moth"), "").expect("should write entry normal root");
        fs::write(src.join("pages/@pages.moth"), "").expect("should write child normal root");
        fs::write(src.join("components/+ui.moth"), "").expect("should write support root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let graph = ProjectModuleGraph::from_source_tree_index(&index);

        let entry_ids: Vec<ModuleRootRole> = graph
            .entry_modules()
            .iter()
            .map(|id| graph.node(*id).role())
            .collect();
        assert!(
            entry_ids.iter().all(|role| *role == ModuleRootRole::Normal),
            "entry candidates must all be normal modules: {entry_ids:?}"
        );
        assert_eq!(
            graph.entry_modules().len(),
            2,
            "two normal roots should be entry candidates"
        );

        let support_id = module_id_for(&index, ModuleRootRole::Support, "components");
        assert!(
            !graph.entry_modules().contains(&support_id),
            "support root must never be an entry candidate"
        );

        assert!(
            graph.project_package_surface().is_none(),
            "a nested support root is not a project package surface"
        );
    }

    #[test]
    fn project_package_surface_is_the_selected_root_support_surface() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        fs::write(root.join("@page.moth"), "").expect("should write normal root");
        fs::write(root.join("+package.moth"), "").expect("should write support root");

        let (index, _project_root, entry_root) = discover_index(&root, ".");
        let graph = ProjectModuleGraph::from_source_tree_index(&index);
        let module_id = index
            .module_identities()
            .module_id_for_directory(&entry_root)
            .expect("paired roots should define one structural module");
        assert_eq!(
            graph.project_package_surface(),
            Some(module_id),
            "a selected-root support surface is the project package surface"
        );

        let _support_only = tempfile::tempdir().expect("should create temp dir");
        let support_only_root = _support_only.path().to_path_buf();
        fs::write(support_only_root.join("+package.moth"), "")
            .expect("should write support-only root");
        let (support_index, _project_root, support_entry_root) =
            discover_index(&support_only_root, ".");
        let support_graph = ProjectModuleGraph::from_source_tree_index(&support_index);
        let support_module_id = support_index
            .module_identities()
            .module_id_for_directory(&support_entry_root)
            .expect("support-only root should define one structural module");
        assert_eq!(
            support_graph.project_package_surface(),
            Some(support_module_id),
            "a support-only selected root is still a valid project package surface"
        );
    }

    #[test]
    fn nested_support_without_a_structural_parent_is_not_a_project_package_surface() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        fs::create_dir_all(root.join("library")).expect("should create nested directory");
        fs::write(root.join("library/+api.moth"), "").expect("should write nested support root");

        let (index, _project_root, _entry_root) = discover_index(&root, ".");
        let graph = ProjectModuleGraph::from_source_tree_index(&index);
        assert_eq!(
            graph.project_package_surface(),
            None,
            "a nested support root with no structural parent is not rooted at the selected directory"
        );
    }

    #[test]
    fn support_visibility_is_visible_to_owner_and_normal_descendants_outside_subtree() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("markdown/parser")).expect("should create markdown parser");
        fs::create_dir_all(src.join("pages/article")).expect("should create pages article");

        fs::write(src.join("@site.moth"), "").expect("should write site normal root");
        fs::write(src.join("markdown/+package.moth"), "").expect("should write support root");
        fs::write(src.join("markdown/parser/@parser.moth"), "")
            .expect("should write private normal");
        fs::write(src.join("pages/@pages.moth"), "").expect("should write pages normal root");
        fs::write(src.join("pages/article/@article.moth"), "")
            .expect("should write article normal");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let graph = ProjectModuleGraph::from_source_tree_index(&index);

        let support_id = module_id_for(&index, ModuleRootRole::Support, "markdown");
        let site_id = module_id_for(&index, ModuleRootRole::Normal, "");
        let pages_id = module_id_for(&index, ModuleRootRole::Normal, "pages");
        let article_id = module_id_for(&index, ModuleRootRole::Normal, "pages/article");

        // Visible to the owning normal module and normal descendants outside the private subtree.
        assert!(
            graph.is_support_visible_to_consumer(support_id, site_id),
            "support is visible to its owning normal module"
        );
        assert!(
            graph.is_support_visible_to_consumer(support_id, pages_id),
            "support is visible to a normal sibling descendant of the owner"
        );
        assert!(
            graph.is_support_visible_to_consumer(support_id, article_id),
            "support is visible to a deeper normal descendant of the owner"
        );
    }

    #[test]
    fn support_visibility_enforces_private_same_scope_and_outer_scope_boundaries() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("markdown/parser")).expect("should create markdown parser");
        fs::create_dir_all(src.join("assets")).expect("should create same-scope support");
        fs::create_dir_all(src.join("pages/extras")).expect("should create pages extras support");

        fs::write(src.join("@site.moth"), "").expect("should write site normal root");
        fs::write(src.join("markdown/+package.moth"), "").expect("should write support root");
        fs::write(src.join("markdown/parser/@parser.moth"), "")
            .expect("should write private normal");
        fs::write(src.join("assets/+assets.moth"), "").expect("should write same-scope support");
        fs::write(src.join("pages/@pages.moth"), "").expect("should write pages normal root");
        fs::write(src.join("pages/extras/+extras.moth"), "").expect("should write sibling support");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let graph = ProjectModuleGraph::from_source_tree_index(&index);

        let support_id = module_id_for(&index, ModuleRootRole::Support, "markdown");
        let parser_id = module_id_for(&index, ModuleRootRole::Normal, "markdown/parser");
        let assets_id = module_id_for(&index, ModuleRootRole::Support, "assets");
        let extras_id = module_id_for(&index, ModuleRootRole::Support, "pages/extras");

        // Not visible to private descendants of the support package.
        assert!(
            !graph.is_support_visible_to_consumer(support_id, parser_id),
            "support must not be visible to its own private descendants"
        );
        // Not visible to itself.
        assert!(
            !graph.is_support_visible_to_consumer(support_id, support_id),
            "support must not be visible to itself"
        );
        // Not visible to another support package owned by the same normal scope.
        assert!(
            !graph.is_support_visible_to_consumer(support_id, assets_id),
            "support must not be visible to a same-scope support sibling"
        );
        // A support facade in a strictly nested normal scope may depend on outer support packages.
        assert!(
            graph.is_support_visible_to_consumer(support_id, extras_id),
            "nested support facade should see a support package from a strictly outer scope"
        );
        // A non-support module id is not a valid support argument.
        assert!(
            !graph.is_support_visible_to_consumer(parser_id, support_id),
            "querying visibility for a non-support module returns false"
        );
    }

    #[test]
    fn paired_support_visibility_excludes_self_in_graph_and_identity_table() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("pages/peer")).expect("should create pages peer");
        fs::write(src.join("@site.moth"), "").expect("should write site root");
        fs::write(src.join("pages/@pages.moth"), "").expect("should write pages normal root");
        fs::write(src.join("pages/+package.moth"), "").expect("should write pages support root");
        fs::write(src.join("pages/peer/+peer.moth"), "").expect("should write peer support root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let graph = ProjectModuleGraph::from_source_tree_index(&index);
        let identities = index.module_identities();
        let paired_id = module_id_for(&index, ModuleRootRole::Normal, "pages");
        let site_id = module_id_for(&index, ModuleRootRole::Normal, "");
        let peer_id = module_id_for(&index, ModuleRootRole::Support, "pages/peer");

        assert!(
            !graph.is_support_visible_to_consumer_role(
                paired_id,
                paired_id,
                ModuleRootRole::Normal
            ),
            "a paired normal surface must not import its own support surface"
        );
        assert!(
            !graph.is_support_visible_to_consumer_role(
                paired_id,
                paired_id,
                ModuleRootRole::Support
            ),
            "a paired support surface must not import itself"
        );
        assert!(
            graph.is_support_visible_to_consumer_role(paired_id, site_id, ModuleRootRole::Normal),
            "the owning normal parent must see the paired support surface"
        );
        assert!(
            !graph.is_support_visible_to_consumer_role(paired_id, peer_id, ModuleRootRole::Support),
            "a same-scope support sibling must not see the paired support surface"
        );
        assert!(
            !super::project_module_graph::is_support_visible_in_identity_table(
                identities, paired_id, paired_id
            ),
            "the identity-table visibility path must also exclude self"
        );
        assert!(
            super::project_module_graph::is_support_visible_in_identity_table(
                identities, paired_id, site_id
            ),
            "the identity-table visibility path must keep the owning parent visible"
        );
        assert!(
            !super::project_module_graph::is_support_visible_in_identity_table(
                identities, paired_id, peer_id
            ),
            "the identity-table visibility path must keep same-scope support siblings hidden"
        );
    }

    #[test]
    fn support_visibility_rejects_modules_outside_owner_subtree() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("other")).expect("should create other branch");
        fs::create_dir_all(src.join("pages/components")).expect("should create pages components");

        fs::write(src.join("@site.moth"), "").expect("should write site normal root");
        fs::write(src.join("other/@other.moth"), "").expect("should write unrelated normal root");
        fs::write(src.join("pages/@pages.moth"), "").expect("should write pages normal root");
        fs::write(src.join("pages/components/+ui.moth"), "").expect("should write support root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let graph = ProjectModuleGraph::from_source_tree_index(&index);

        let support_id = module_id_for(&index, ModuleRootRole::Support, "pages/components");
        let other_id = module_id_for(&index, ModuleRootRole::Normal, "other");

        assert!(
            !graph.is_support_visible_to_consumer(support_id, other_id),
            "support must not be visible outside the owning normal module's subtree"
        );
    }

    #[test]
    fn entry_owner_exposes_support_to_entry_and_normal_siblings() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("lib")).expect("should create lib");
        fs::create_dir_all(src.join("pages")).expect("should create pages");

        fs::write(src.join("@entry.moth"), "").expect("should write entry root");
        fs::write(src.join("lib/@package.moth"), "").expect("should write support root");
        fs::write(src.join("pages/@mod.moth"), "").expect("should write normal sibling root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let graph = ProjectModuleGraph::from_source_tree_index(&index);
        let identities = index.module_identities();

        let owner_id = module_id_for(&index, ModuleRootRole::Entry, "");
        let support_id = module_id_for(&index, ModuleRootRole::Support, "lib");
        let sibling_id = module_id_for(&index, ModuleRootRole::Normal, "pages");

        assert!(
            identities.record(owner_id).normal_root_file().is_none()
                && identities.record(owner_id).entry_root_file().is_some(),
            "an entry-only owner must stay entry-only instead of acquiring a normal facade"
        );
        assert!(
            graph.is_support_visible_to_consumer(support_id, owner_id),
            "support nested under an entry-only owner is visible to that owner"
        );
        assert!(
            graph.is_support_visible_to_consumer_role(support_id, owner_id, ModuleRootRole::Entry),
            "an entry source is a normal-position consumer of support packages"
        );
        assert!(
            graph.is_support_visible_to_consumer(support_id, sibling_id),
            "support nested under an entry-only owner is visible to its normal siblings"
        );
        assert!(
            super::project_module_graph::is_support_visible_in_identity_table(
                identities, support_id, owner_id
            ),
            "project and package topology must agree for an entry-only owner"
        );
        assert!(
            super::project_module_graph::is_support_visible_in_identity_table(
                identities, support_id, sibling_id
            ),
            "project and package topology must agree for a normal sibling"
        );
    }

    #[test]
    fn support_only_intermediate_directory_is_not_a_normal_owner() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("theme/parts")).expect("should create nested support");
        fs::create_dir_all(src.join("pages")).expect("should create pages");

        fs::write(src.join("@entry.moth"), "").expect("should write entry root");
        fs::write(src.join("theme/@package.moth"), "")
            .expect("should write support-only intermediate root");
        fs::write(src.join("theme/parts/@package.moth"), "")
            .expect("should write nested support root");
        fs::write(src.join("pages/@mod.moth"), "").expect("should write normal sibling root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let graph = ProjectModuleGraph::from_source_tree_index(&index);
        let identities = index.module_identities();

        let owner_id = module_id_for(&index, ModuleRootRole::Entry, "");
        let intermediate_id = module_id_for(&index, ModuleRootRole::Support, "theme");
        let nested_id = module_id_for(&index, ModuleRootRole::Support, "theme/parts");
        let pages_id = module_id_for(&index, ModuleRootRole::Normal, "pages");

        assert!(
            graph.is_support_visible_to_consumer(nested_id, owner_id),
            "owner resolution must skip a support-only intermediate directory"
        );
        assert!(
            graph.is_support_visible_to_consumer(nested_id, pages_id),
            "a normal sibling must see the nested support package through the skipped owner"
        );
        assert!(
            !graph.is_support_visible_to_consumer(nested_id, intermediate_id),
            "a support-only intermediate directory must not become the nested package's owner"
        );
        assert!(
            super::project_module_graph::is_support_visible_in_identity_table(
                identities, nested_id, owner_id
            ),
            "the package identity table must skip the support-only intermediate directory too"
        );
        assert!(
            !super::project_module_graph::is_support_visible_in_identity_table(
                identities,
                nested_id,
                intermediate_id
            ),
            "the package identity table must not treat the support-only directory as an owner"
        );
    }

    #[test]
    fn paired_entry_support_module_uses_normal_position_for_entry_and_strict_support_for_facade() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("app")).expect("should create app");
        fs::create_dir_all(src.join("lib")).expect("should create lib");

        fs::write(src.join("@entry.moth"), "").expect("should write project entry root");
        fs::write(src.join("app/@entry.moth"), "").expect("should write paired entry root");
        fs::write(src.join("app/@package.moth"), "").expect("should write paired support root");
        fs::write(src.join("lib/@package.moth"), "").expect("should write sibling support root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let graph = ProjectModuleGraph::from_source_tree_index(&index);
        let identities = index.module_identities();

        let paired_id = module_id_for(&index, ModuleRootRole::Support, "app");
        let support_id = module_id_for(&index, ModuleRootRole::Support, "lib");

        assert!(
            identities.record(paired_id).entry_root_file().is_some()
                && identities.record(paired_id).support_root_file().is_some()
                && identities.record(paired_id).normal_root_file().is_none(),
            "the paired module must own an entry and a support root and no normal root"
        );
        assert!(
            graph.is_support_visible_to_consumer(support_id, paired_id),
            "the paired module namespace must expose the normal-position support targets for its entry"
        );
        assert!(
            graph.is_support_visible_to_consumer_role(support_id, paired_id, ModuleRootRole::Entry),
            "the paired entry is a normal-position consumer"
        );
        assert!(
            !graph.is_support_visible_to_consumer_role(
                support_id,
                paired_id,
                ModuleRootRole::Support
            ),
            "the co-located support facade must keep the strict support scope and gain nothing from the entry"
        );
        assert!(
            super::project_module_graph::is_support_visible_in_identity_table(
                identities, support_id, paired_id
            ),
            "the package identity table must build the same normal-position namespace"
        );
    }

    #[test]
    fn triple_root_module_occupies_normal_position_and_publishes_two_facades() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("lib")).expect("should create lib");

        fs::write(src.join("@entry.moth"), "").expect("should write entry root");
        fs::write(src.join("@mod.moth"), "").expect("should write normal facade root");
        fs::write(src.join("@package.moth"), "").expect("should write support facade root");
        fs::write(src.join("lib/@package.moth"), "").expect("should write support root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let graph = ProjectModuleGraph::from_source_tree_index(&index);
        let identities = index.module_identities();

        let root_id = module_id_for(&index, ModuleRootRole::Normal, "");
        let support_id = module_id_for(&index, ModuleRootRole::Support, "lib");
        let node = graph.node(root_id);

        assert!(
            node.has_surface(ModuleRootRole::Normal)
                && node.has_surface(ModuleRootRole::Support)
                && node.has_surface(ModuleRootRole::Entry),
            "a triple-root module must keep all three root surfaces"
        );
        let (normal_origin, support_origin) = node.publication_surfaces();
        assert_eq!(
            normal_origin.map(|origin| origin.role()),
            Some(ModuleRootRole::Normal),
            "the normal facade must publish the normal surface"
        );
        assert_eq!(
            support_origin.map(|origin| origin.role()),
            Some(ModuleRootRole::Support),
            "the support facade must publish the support surface"
        );
        assert!(
            graph.entry_modules().contains(&root_id),
            "a triple-root module remains an entry candidate"
        );
        assert!(
            graph.is_support_visible_to_consumer_role(support_id, root_id, ModuleRootRole::Entry),
            "the triple-root entry is a normal-position consumer"
        );
        assert!(
            super::project_module_graph::is_support_visible_in_identity_table(
                identities, support_id, root_id
            ),
            "the package identity table must agree for a triple-root owner"
        );
    }

    #[test]
    fn entry_owner_permits_strictly_outer_support_scope_imports() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("lib")).expect("should create lib");
        fs::create_dir_all(src.join("pages/tools")).expect("should create pages tools");

        fs::write(src.join("@entry.moth"), "").expect("should write project entry root");
        fs::write(src.join("lib/@package.moth"), "").expect("should write outer support root");
        fs::write(src.join("pages/@entry.moth"), "").expect("should write pages entry root");
        fs::write(src.join("pages/tools/@package.moth"), "")
            .expect("should write nested support root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let graph = ProjectModuleGraph::from_source_tree_index(&index);
        let identities = index.module_identities();

        let outer_id = module_id_for(&index, ModuleRootRole::Support, "lib");
        let pages_id = module_id_for(&index, ModuleRootRole::Entry, "pages");
        let nested_id = module_id_for(&index, ModuleRootRole::Support, "pages/tools");

        assert!(
            graph.is_support_visible_to_consumer_role(outer_id, nested_id, ModuleRootRole::Support),
            "a support facade under an entry-only owner may import a strictly outer support package"
        );
        assert!(
            graph.is_support_visible_to_consumer(outer_id, nested_id),
            "the module-level namespace must expose the strictly outer support package"
        );
        assert!(
            graph.is_support_visible_to_consumer_role(outer_id, pages_id, ModuleRootRole::Entry),
            "the entry-only intermediate owner sees the outer support package"
        );
        assert!(
            !graph.is_support_visible_to_consumer(nested_id, outer_id),
            "an outer support package must not see a strictly nested support package"
        );
        assert!(
            super::project_module_graph::is_support_visible_in_identity_table(
                identities, outer_id, nested_id
            ),
            "the package identity table must apply the same strictly-outer rule"
        );
    }

    #[test]
    fn entry_owner_private_support_descendant_cannot_import_the_support_package() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("lib/inner")).expect("should create private descendant");

        fs::write(src.join("@entry.moth"), "").expect("should write project entry root");
        fs::write(src.join("lib/@package.moth"), "").expect("should write support root");
        fs::write(src.join("lib/inner/@mod.moth"), "").expect("should write private descendant");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let graph = ProjectModuleGraph::from_source_tree_index(&index);
        let identities = index.module_identities();

        let support_id = module_id_for(&index, ModuleRootRole::Support, "lib");
        let inner_id = module_id_for(&index, ModuleRootRole::Normal, "lib/inner");

        assert!(
            !graph.is_support_visible_to_consumer_role(
                support_id,
                inner_id,
                ModuleRootRole::Normal
            ),
            "a support package's private descendant must not import that support package"
        );
        assert!(
            !graph.is_support_visible_to_consumer(support_id, inner_id),
            "the module-level namespace must keep the private descendant outside the package"
        );
        assert!(
            !super::project_module_graph::is_support_visible_in_identity_table(
                identities, support_id, inner_id
            ),
            "the package identity table must preserve the private-descendant exclusion"
        );
    }

    #[test]
    fn independent_ready_modules_share_wave_zero_in_module_id_order() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("alpha")).expect("should create alpha");
        fs::create_dir_all(src.join("beta")).expect("should create beta");

        fs::write(src.join("@home.moth"), "").expect("should write entry root");
        fs::write(src.join("alpha/@alpha.moth"), "").expect("should write alpha root");
        fs::write(src.join("beta/@beta.moth"), "").expect("should write beta root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let alpha_id = module_id_for(&index, ModuleRootRole::Normal, "alpha");
        let beta_id = module_id_for(&index, ModuleRootRole::Normal, "beta");

        // No edges: every module is independent and shares wave zero, ordered by ModuleId. The
        // no-edge graph is completed before scheduling so wave scheduling reads only frozen
        // adjacency.
        let mut no_edge_graph = ProjectModuleGraph::from_source_tree_index(&index);
        no_edge_graph
            .complete()
            .expect("no-edge graph should complete before scheduling");
        let waves = no_edge_graph
            .compile_waves()
            .expect("completed no-edge graph should produce one wave");
        assert_eq!(waves.len(), 1, "no edges means one ready wave");
        let wave_zero: Vec<ModuleId> = waves[0].clone();
        assert_eq!(
            wave_zero.len(),
            no_edge_graph.node_count(),
            "every module should be ready in wave zero"
        );
        let mut sorted = wave_zero.clone();
        sorted.sort_by_key(|id| id.index());
        assert_eq!(wave_zero, sorted, "wave zero must be in ModuleId order");

        // Adding a provider-before-consumer edge splits the waves deterministically. Edge
        // insertion happens while the graph is under construction; completion freezes the
        // adjacency before scheduling reads it.
        let mut graph = ProjectModuleGraph::from_source_tree_index(&index);
        assert_eq!(
            graph.add_dependency_edge(alpha_id, beta_id).unwrap(),
            DependencyEdgeOutcome::Inserted,
            "inserting a fresh edge reports Inserted"
        );
        assert_eq!(
            graph.add_dependency_edge(alpha_id, beta_id).unwrap(),
            DependencyEdgeOutcome::AlreadyPresent,
            "inserting the same edge is idempotent"
        );
        assert!(graph.has_dependency_edge(alpha_id, beta_id));
        graph
            .complete()
            .expect("populated graph should complete before scheduling");
        assert!(graph.has_dependency_edge(alpha_id, beta_id));

        let waves = graph
            .compile_waves()
            .expect("ordered graph should wave cleanly");
        assert_eq!(waves.len(), 2, "provider then consumer is two waves");
        assert!(
            waves[0].contains(&alpha_id) && !waves[0].contains(&beta_id),
            "provider must compile in an earlier wave than its consumer"
        );
        assert!(
            waves[1].contains(&beta_id),
            "consumer must compile after its provider"
        );
    }

    #[test]
    fn support_surface_is_ordered_by_real_edges_not_a_fake_dependency() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("pages")).expect("should create pages");
        fs::create_dir_all(src.join("components")).expect("should create components");

        fs::write(src.join("@site.moth"), "").expect("should write entry normal root");
        fs::write(src.join("pages/@pages.moth"), "").expect("should write child normal root");
        fs::write(src.join("components/+ui.moth"), "").expect("should write support root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");

        let support_id = module_id_for(&index, ModuleRootRole::Support, "components");
        let pages_id = module_id_for(&index, ModuleRootRole::Normal, "pages");
        let site_id = module_id_for(&index, ModuleRootRole::Normal, "");

        // Without edges the support module is independent and joins wave zero with the normal
        // modules. The no-edge graph completes before scheduling so wave scheduling reads only
        // frozen adjacency.
        let mut no_edge_graph = ProjectModuleGraph::from_source_tree_index(&index);
        no_edge_graph
            .complete()
            .expect("no-edge graph should complete before scheduling");
        let waves = no_edge_graph
            .compile_waves()
            .expect("completed no-edge graph waves cleanly");
        assert_eq!(waves.len(), 1, "no edges means one wave");
        assert!(
            waves[0].contains(&support_id),
            "support module with no edges is independent and ready in wave zero"
        );

        // Once real edges target the support module, it is ordered after its providers without
        // any hard-coded fake dependency.
        let mut graph = ProjectModuleGraph::from_source_tree_index(&index);
        graph
            .add_dependency_edge(pages_id, support_id)
            .expect("pages -> support edge should insert");
        graph
            .add_dependency_edge(site_id, support_id)
            .expect("site -> support edge should insert");
        graph
            .complete()
            .expect("populated graph should complete before scheduling");

        let waves = graph
            .compile_waves()
            .expect("support-ordered graph waves cleanly");
        let support_wave = waves
            .iter()
            .position(|wave| wave.contains(&support_id))
            .expect("support should appear in a wave");
        let pages_wave = waves
            .iter()
            .position(|wave| wave.contains(&pages_id))
            .expect("pages should appear in a wave");
        let site_wave = waves
            .iter()
            .position(|wave| wave.contains(&site_id))
            .expect("site should appear in a wave");
        assert!(
            support_wave > pages_wave && support_wave > site_wave,
            "support module must compile after both providers that target it"
        );
    }

    #[test]
    fn dependency_cycle_reports_circular_dependency_diagnostic() {
        use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::CircularDependencyView;
        use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terse};

        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("alpha")).expect("should create alpha");
        fs::create_dir_all(src.join("beta")).expect("should create beta");

        fs::write(src.join("@home.moth"), "").expect("should write entry root");
        fs::write(src.join("alpha/@alpha.moth"), "").expect("should write alpha root");
        fs::write(src.join("beta/@beta.moth"), "").expect("should write beta root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let mut graph = ProjectModuleGraph::from_source_tree_index(&index);

        let alpha_id = module_id_for(&index, ModuleRootRole::Normal, "alpha");
        let beta_id = module_id_for(&index, ModuleRootRole::Normal, "beta");

        graph
            .add_dependency_edge(alpha_id, beta_id)
            .expect("alpha -> beta edge should insert");
        graph
            .add_dependency_edge(beta_id, alpha_id)
            .expect("beta -> alpha edge should insert");
        graph
            .complete()
            .expect("cyclic graph should still complete before scheduling");

        let error = graph
            .compile_waves()
            .expect_err("an authored module cycle must be diagnosed");
        let reports = prepared_failure_reports(error);
        assert_eq!(reports.counts().errors, 1);
        let (report, view) = reports
            .iter()
            .next()
            .expect("the cycle should retain one diagnostic");
        assert!(
            view.family_view::<CircularDependencyView>().is_some(),
            "module cycles must use the circular-dependency family"
        );
        let rendered =
            terse::format_terse_diagnostic_view(view, DiagnosticRenderContext::for_report(report));
        assert!(
            rendered.contains("Circular dependency at 'alpha'")
                || rendered.contains("Circular dependency at 'beta'"),
            "the reported module must lie on the cycle: {rendered}"
        );
    }

    #[test]
    fn self_edge_and_invalid_ids_are_rejected_without_panicking() {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("alpha")).expect("should create alpha");

        fs::write(src.join("@home.moth"), "").expect("should write entry root");
        fs::write(src.join("alpha/@alpha.moth"), "").expect("should write alpha root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let mut graph = ProjectModuleGraph::from_source_tree_index(&index);

        let alpha_id = module_id_for(&index, ModuleRootRole::Normal, "alpha");

        let self_error = graph
            .add_dependency_edge(alpha_id, alpha_id)
            .expect_err("a self-edge must be rejected");
        assert_eq!(self_error.kind, InfrastructureFailureKind::Internal);

        let out_of_range = ModuleId::from_index(graph.node_count() + 10);
        let invalid_error = graph
            .add_dependency_edge(out_of_range, alpha_id)
            .expect_err("an out-of-range module id must be rejected");
        assert_eq!(invalid_error.kind, InfrastructureFailureKind::Internal);

        // The graph remains usable for deterministic waves after rejected edges. Rejected edges
        // do not mutate the graph, so it still completes cleanly with no accepted edges.
        graph
            .complete()
            .expect("graph should complete before scheduling after rejected edges");
        let waves = graph
            .compile_waves()
            .expect("rejected edges do not mutate the graph");
        assert_eq!(waves.len(), 1, "no accepted edges means one ready wave");
    }

    #[test]
    fn frozen_adjacency_orders_consumers_in_module_id_order() {
        // Hidden invariant: completion freezes provider/consumer adjacency into sorted
        // `Vec<ModuleId>` storage, so the consumer wave preserves `ModuleId` order regardless of
        // edge insertion order. The frozen order is observable through the wave that follows the
        // provider.
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("alpha")).expect("should create alpha");
        fs::create_dir_all(src.join("beta")).expect("should create beta");
        fs::create_dir_all(src.join("gamma")).expect("should create gamma");

        fs::write(src.join("@home.moth"), "").expect("should write entry root");
        fs::write(src.join("alpha/@alpha.moth"), "").expect("should write alpha root");
        fs::write(src.join("beta/@beta.moth"), "").expect("should write beta root");
        fs::write(src.join("gamma/@gamma.moth"), "").expect("should write gamma root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let alpha_id = module_id_for(&index, ModuleRootRole::Normal, "alpha");
        let beta_id = module_id_for(&index, ModuleRootRole::Normal, "beta");
        let gamma_id = module_id_for(&index, ModuleRootRole::Normal, "gamma");

        // alpha is the provider; beta and gamma are its consumers. Insert the higher-`ModuleId`
        // consumer first so insertion order does not match the frozen `ModuleId` order the waves
        // must preserve.
        let (first_consumer, second_consumer) = if beta_id.index() < gamma_id.index() {
            (gamma_id, beta_id)
        } else {
            (beta_id, gamma_id)
        };
        let mut graph = ProjectModuleGraph::from_source_tree_index(&index);
        graph
            .add_dependency_edge(alpha_id, first_consumer)
            .expect("provider -> first-inserted consumer edge should insert");
        graph
            .add_dependency_edge(alpha_id, second_consumer)
            .expect("provider -> second-inserted consumer edge should insert");
        graph
            .complete()
            .expect("graph should complete before scheduling");

        let waves = graph
            .compile_waves()
            .expect("provider-with-consumers graph should wave cleanly");
        assert_eq!(waves.len(), 2, "provider then consumers is two waves");
        assert!(
            waves[0].contains(&alpha_id),
            "provider is ready in wave zero"
        );
        let consumer_wave = &waves[1];
        assert_eq!(
            consumer_wave.len(),
            2,
            "both consumers are ready in the second wave"
        );
        // The frozen consumer adjacency is sorted by `ModuleId`, so the consumer wave preserves
        // that order regardless of insertion order.
        let mut sorted_consumers = consumer_wave.clone();
        sorted_consumers.sort_by_key(|id| id.index());
        assert_eq!(
            consumer_wave, &sorted_consumers,
            "frozen consumer adjacency must be in ModuleId order"
        );
    }

    #[test]
    fn invalid_graph_phase_use_reports_internal_errors() {
        // Hidden invariant: the construction-to-completion phase is an explicit boundary.
        // Compile-wave scheduling before completion is a compiler bug, while mutation after
        // completion and double completion remain internal `InfrastructureFailure`s.
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        fs::create_dir_all(src.join("alpha")).expect("should create alpha");
        fs::create_dir_all(src.join("beta")).expect("should create beta");

        fs::write(src.join("@home.moth"), "").expect("should write entry root");
        fs::write(src.join("alpha/@alpha.moth"), "").expect("should write alpha root");
        fs::write(src.join("beta/@beta.moth"), "").expect("should write beta root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let alpha_id = module_id_for(&index, ModuleRootRole::Normal, "alpha");
        let beta_id = module_id_for(&index, ModuleRootRole::Normal, "beta");

        // Scheduling before completion is a compiler bug.
        let uncompleted = ProjectModuleGraph::from_source_tree_index(&index);
        let expected_message = "Project module graph compile waves were requested before completion; the graph must complete and freeze its adjacency before compile-wave scheduling reads it";
        let payload = std::panic::catch_unwind(|| uncompleted.compile_waves())
            .expect_err("scheduling before completion must raise a compiler bug");
        let report = payload
            .downcast::<CompilerBugReport>()
            .unwrap_or_else(|_| panic!("expected a CompilerBugReport payload"));
        assert_eq!(report.stage, CompilerStage::ProjectBuildAssembly);
        assert_eq!(report.message.as_ref(), expected_message);

        // Mutation after completion is an internal compiler failure. Edge insertion and double
        // completion both surface as the same lifecycle violation.
        let mut completed = ProjectModuleGraph::from_source_tree_index(&index);
        completed
            .add_dependency_edge(alpha_id, beta_id)
            .expect("construction edge should insert");
        completed.complete().expect("graph should complete once");

        let mutation_error = completed
            .add_dependency_edge(beta_id, alpha_id)
            .expect_err("mutation after completion must be rejected");
        assert_eq!(mutation_error.kind, InfrastructureFailureKind::Internal);
        assert!(
            mutation_error.message.contains("after completion"),
            "mutation error must name the phase violation: {}",
            mutation_error.message
        );

        let double_complete_error = completed
            .complete()
            .expect_err("double completion must be rejected as mutation after completion");
        assert_eq!(
            double_complete_error.kind,
            InfrastructureFailureKind::Internal
        );
        assert!(
            double_complete_error.message.contains("after completion"),
            "double completion error must name the phase violation: {}",
            double_complete_error.message
        );

        // The completed graph still schedules cleanly from its frozen adjacency.
        let waves = completed
            .compile_waves()
            .expect("completed graph should schedule from frozen adjacency");
        assert_eq!(
            waves.len(),
            2,
            "frozen adjacency preserves provider-then-consumer waves"
        );
        assert!(waves[0].contains(&alpha_id));
        assert!(waves[1].contains(&beta_id));
    }
}
