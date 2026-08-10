//! Phase 1 Stage 0 filesystem-identity tests.
//!
//! WHAT: exercises the strict UTF-8 filesystem-identity contract added by the codebase
//!      integrity cleanup plan: non-UTF-8 module roots, source names, folder names,
//!      extensions, source-package prefixes and single-file entries must surface as File
//!      infrastructure errors, and source-package canonicalization must be mandatory.
//! WHY: these invariants are Stage 0 subsystem-local facts that integration output cannot
//!      inspect directly, so they own a focused test file beside the create-project-modules
//!      module rather than living in the oversized Stage 0 orchestration test file.

use super::*;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_tests::test_support::temp_dir;

#[cfg(target_os = "linux")]
mod non_utf8_filesystem_identity {
    use super::*;
    use crate::compiler_frontend::compiler_errors::ErrorType;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::PathBuf;

    fn assert_file_infrastructure_error(messages: &CompilerMessages) {
        let (error_type, message, _location) = messages
            .first_infrastructure_error_for_tests()
            .expect("expected an infrastructure file error");
        assert_eq!(
            *error_type,
            ErrorType::File,
            "non-UTF-8 filesystem name should be a File infrastructure error"
        );
        assert!(
            message.contains("Non-UTF-8"),
            "error message should mention non-UTF-8: {message}"
        );
    }

    #[test]
    fn source_tree_rejects_non_utf8_file_name() {
        let root = temp_dir("source_tree_non_utf8_file");
        let entry_root = root.join("src");
        fs::create_dir_all(&entry_root).expect("should create entry root");
        fs::write(entry_root.join("@home.moth"), "").expect("should write entry root");

        let bad_name = OsString::from_vec(vec![0xC3, 0x28]);
        let bad_file = entry_root.join(bad_name);
        fs::write(&bad_file, "x ~= 1\n").expect("should write non-UTF-8 named file");

        let mut config = Config::new(root.clone());
        config.entry_root = PathBuf::from("src");
        let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
        let canonical_entry_root =
            fs::canonicalize(&entry_root).expect("entry root should canonicalize");
        let mut string_table = StringTable::new();

        let messages = super::source_tree_index::SourceTreeIndex::discover(
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

        assert_file_infrastructure_error(&messages);
        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn source_tree_rejects_non_utf8_folder_name() {
        let root = temp_dir("source_tree_non_utf8_folder");
        let entry_root = root.join("src");
        fs::create_dir_all(&entry_root).expect("should create entry root");
        fs::write(entry_root.join("@home.moth"), "").expect("should write entry root");

        let bad_name = OsString::from_vec(vec![0xC3, 0x28]);
        let bad_folder = entry_root.join(bad_name);
        fs::create_dir_all(&bad_folder).expect("should create non-UTF-8 named folder");

        let mut config = Config::new(root.clone());
        config.entry_root = PathBuf::from("src");
        let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
        let canonical_entry_root =
            fs::canonicalize(&entry_root).expect("entry root should canonicalize");
        let mut string_table = StringTable::new();

        let messages = super::source_tree_index::SourceTreeIndex::discover(
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

        assert_file_infrastructure_error(&messages);
        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn facade_discovery_rejects_non_utf8_direct_child_of_project_root() {
        let root = temp_dir("facade_non_utf8_child");
        let entry_root = root.join("src");
        fs::create_dir_all(&entry_root).expect("should create entry root");
        fs::write(entry_root.join("@page.moth"), "").expect("should write entry root");

        // A non-UTF-8 named direct child of the project root must not be silently skipped while
        // scanning for the optional project package facade.
        let bad_name = OsString::from_vec(vec![0xC3, 0x28]);
        let bad_file = root.join(bad_name);
        fs::write(&bad_file, "x ~= 1\n").expect("should write non-UTF-8 named file");

        let mut config = Config::new(root.clone());
        config.entry_root = PathBuf::from("src");
        let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
        let canonical_entry_root =
            fs::canonicalize(&entry_root).expect("entry root should canonicalize");
        let mut string_table = StringTable::new();

        let messages = super::source_tree_index::SourceTreeIndex::discover(
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
        .expect_err("non-UTF-8 project-root child should be rejected during facade discovery");

        assert_file_infrastructure_error(&messages);
        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn package_boundary_traversal_rejects_non_utf8_name() {
        let root = temp_dir("package_boundary_non_utf8_name");
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");

        let bad_name = OsString::from_vec(vec![0xC3, 0x28]);
        let bad_file = package_root.join(bad_name);
        fs::write(&bad_file, "x ~= 1\n").expect("should write non-UTF-8 named file");

        let mut source_packages = crate::builder_surface::SourcePackageRegistry::new();
        source_packages.register_filesystem_root(
            "pkg",
            package_root,
            crate::builder_surface::PackageOrigin::ProjectLocal,
        );

        let mut string_table = StringTable::new();
        let messages = super::source_package_discovery::build_source_package_boundary_indexes(
            &source_packages,
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::
                ExternalImportProviderRegistry::default(),
            &mut string_table,
        )
        .expect_err("non-UTF-8 name in package boundary traversal should be rejected");

        assert_file_infrastructure_error(&messages);
        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn legacy_package_folder_does_not_scan_non_utf8_names() {
        let root = temp_dir("package_prefix_non_utf8");
        let packages_folder = root.join("packages");
        fs::create_dir_all(&packages_folder).expect("should create packages folder");

        let bad_name = OsString::from_vec(vec![0xC3, 0x28]);
        let bad_package = packages_folder.join(bad_name);
        fs::create_dir_all(&bad_package).expect("should create non-UTF-8 named package directory");
        fs::create_dir_all(root.join("src")).expect("should create entry root");
        fs::write(root.join("src/@page.moth"), "x ~= 1\n").expect("should write entry");

        let mut config = Config::new(root.clone());
        config.entry_root = PathBuf::from("src");
        config.package_folders = vec![PathBuf::from("packages")];
        config.has_explicit_package_folders = true;

        let mut string_table = StringTable::new();
        let resolver = super::project_roots::build_project_path_resolver(
            &config,
            &crate::builder_surface::SourcePackageRegistry::default(),
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &mut string_table,
        )
        .expect("legacy package folders must not be scanned");

        assert!(resolver.source_package_roots().is_empty());
        fs::remove_dir_all(&root).expect("should remove temp root");
    }
}

#[cfg(target_os = "linux")]
mod non_utf8_single_file_identity {
    use super::*;
    use crate::compiler_frontend::compiler_errors::ErrorType;
    use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    fn assert_file_infrastructure_error(messages: &CompilerMessages) {
        let (error_type, message, _location) = messages
            .first_infrastructure_error_for_tests()
            .expect("expected an infrastructure file error");
        assert_eq!(
            *error_type,
            ErrorType::File,
            "non-UTF-8 single-file input should be a File infrastructure error"
        );
        assert!(
            message.contains("UTF-8"),
            "error message should mention UTF-8: {message}"
        );
    }

    #[test]
    fn single_file_rejects_non_utf8_extension() {
        let root = temp_dir("single_file_non_utf8_ext");
        let entry = root.join("main.");
        let bad_ext = OsString::from_vec(vec![0xC3, 0x28]);
        let entry_with_bad_ext = entry.with_extension(bad_ext);
        fs::write(&entry_with_bad_ext, "x ~= 1\n").expect("should write entry file");

        let config = Config::new(entry_with_bad_ext.clone());
        let mut builder_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
        let mut string_table = StringTable::new();

        let extension = entry_with_bad_ext
            .extension()
            .expect("entry should have an extension");
        let messages = super::compilation::compile_single_file_frontend(
            &config,
            crate::compiler_frontend::FrontendBuildProfile::Dev,
            &StyleDirectiveRegistry::default(),
            &mut builder_surface,
            extension,
            &mut string_table,
        );
        let Err(messages) = messages else {
            panic!("non-UTF-8 extension should be rejected");
        };

        assert_file_infrastructure_error(&messages);
        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn single_file_rejects_non_utf8_entry_name() {
        let root = temp_dir("single_file_non_utf8_name");
        let bad_name = OsString::from_vec(vec![0xC3, 0x28]);
        let bad_file = root.join(bad_name).with_extension("moth");
        fs::write(&bad_file, "x ~= 1\n").expect("should write entry file");

        let config = Config::new(bad_file.clone());
        let mut builder_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
        let mut string_table = StringTable::new();

        let extension = bad_file
            .extension()
            .expect("entry should have a .moth extension");
        let messages = super::compilation::compile_single_file_frontend(
            &config,
            crate::compiler_frontend::FrontendBuildProfile::Dev,
            &StyleDirectiveRegistry::default(),
            &mut builder_surface,
            extension,
            &mut string_table,
        );
        let Err(messages) = messages else {
            panic!("non-UTF-8 entry file name should be rejected");
        };

        assert_file_infrastructure_error(&messages);
        fs::remove_dir_all(&root).expect("should remove temp root");
    }
}

mod source_package_boundary_indexes_tests {
    use super::*;
    use crate::builder_surface::SourcePackageRegistry;
    use crate::builder_surface::external_import_providers::provider::ExternalFileExtension;
    use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
    use crate::compiler_frontend::compiler_errors::ErrorType;
    use crate::compiler_frontend::compiler_messages::{DiagnosticPayload, InvalidConfigReason};
    use crate::projects::html_project::external_js::js_import_provider::JsExternalImportProvider;
    use std::path::Path;
    use std::sync::Arc;

    fn build_indexes(
        source_packages: &SourcePackageRegistry,
        string_table: &mut StringTable,
    ) -> Result<super::source_package_discovery::SourcePackageBoundaryIndexes, CompilerMessages>
    {
        super::source_package_discovery::build_source_package_boundary_indexes(
            source_packages,
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::
                ExternalImportProviderRegistry::default(),
            string_table,
        )
    }

    fn register_pkg(source_packages: &mut SourcePackageRegistry, package_root: &Path) {
        source_packages.register_filesystem_root(
            "pkg",
            package_root.to_path_buf(),
            crate::builder_surface::PackageOrigin::ProjectLocal,
        );
    }

    fn assert_invalid_config_reason(
        messages: &CompilerMessages,
        matcher: impl Fn(&InvalidConfigReason) -> bool,
    ) {
        assert!(
            messages.error_diagnostics().any(|diagnostic| {
                matches!(&diagnostic.payload,
                    DiagnosticPayload::InvalidConfig { reason, .. } if matcher(reason))
            }),
            "expected an InvalidConfig diagnostic, got {messages:?}"
        );
    }

    #[test]
    fn canonical_root_with_single_hash_file_derives_unique_view() {
        let root = temp_dir("package_boundary_canonical_success");
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

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn canonicalization_failure_returns_file_error() {
        let root = temp_dir("package_boundary_canonical_failure");
        fs::create_dir_all(&root).expect("should create temp root");
        let nonexistent = root.join("does_not_exist");

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &nonexistent);

        let mut string_table = StringTable::new();
        let messages = build_indexes(&source_packages, &mut string_table)
            .expect_err("nonexistent root should fail canonicalization");

        let (error_type, message, _location) = messages
            .first_infrastructure_error_for_tests()
            .expect("expected an infrastructure file error");
        assert_eq!(*error_type, ErrorType::File);
        assert!(
            message.contains("canonicalize"),
            "error message should mention canonicalization: {message}"
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn missing_normal_module_root_rejected_during_boundary_indexing() {
        let root = temp_dir("package_boundary_missing_root");
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &package_root);

        let mut string_table = StringTable::new();
        let messages = build_indexes(&source_packages, &mut string_table)
            .expect_err("package root without a normal module root should fail boundary indexing");

        assert_invalid_config_reason(&messages, |reason| {
            matches!(reason, InvalidConfigReason::SourcePackageMissingRoot { .. })
        });

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn support_root_does_not_satisfy_package_normal_module_root_requirement() {
        let root = temp_dir("package_boundary_support_only");
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");
        fs::write(package_root.join("+support.moth"), "").expect("should write support root");

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &package_root);

        let mut string_table = StringTable::new();
        let messages = build_indexes(&source_packages, &mut string_table)
            .expect_err("a support root cannot replace the package normal module root");

        assert_invalid_config_reason(&messages, |reason| {
            matches!(reason, InvalidConfigReason::SourcePackageMissingRoot { .. })
        });

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn support_root_beside_package_normal_module_root_uses_shared_root_collision_diagnostic() {
        let root = temp_dir("package_boundary_hash_and_support");
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");
        fs::write(package_root.join("@mod.moth"), "").expect("should write normal module root");
        fs::write(package_root.join("+support.moth"), "").expect("should write support root");

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &package_root);

        let mut string_table = StringTable::new();
        let messages = build_indexes(&source_packages, &mut string_table)
            .expect_err("one directory cannot contain hash and support roots");

        assert_invalid_config_reason(&messages, |reason| {
            matches!(reason, InvalidConfigReason::MultipleModuleRootFiles { .. })
        });

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn multiple_normal_module_roots_rejected_during_boundary_indexing() {
        let root = temp_dir("package_boundary_multiple_roots");
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");
        fs::write(package_root.join("@home.moth"), "")
            .expect("should write first normal module root");
        fs::write(package_root.join("@page.moth"), "")
            .expect("should write second normal module root");

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &package_root);

        let mut string_table = StringTable::new();
        let messages = build_indexes(&source_packages, &mut string_table).expect_err(
            "package root with multiple normal module roots should fail boundary indexing",
        );

        assert_invalid_config_reason(&messages, |reason| {
            matches!(
                reason,
                InvalidConfigReason::SourcePackageMultipleRoots { .. }
            )
        });

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_normal_module_root_returns_file_error() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_dir("package_boundary_unreadable");
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");
        fs::write(package_root.join("@home.moth"), "").expect("should write normal module root");

        // Remove read permission so the package boundary traversal cannot read the directory.
        // Canonicalization still succeeds because it only traverses the parent.
        fs::set_permissions(&package_root, fs::Permissions::from_mode(0o000))
            .expect("should remove read permissions");

        let mut source_packages = SourcePackageRegistry::new();
        register_pkg(&mut source_packages, &package_root);

        let mut string_table = StringTable::new();
        let messages = build_indexes(&source_packages, &mut string_table)
            .expect_err("unreadable package root should fail boundary indexing");

        let (error_type, _message, _location) = messages
            .first_infrastructure_error_for_tests()
            .expect("expected an infrastructure file error");
        assert_eq!(*error_type, ErrorType::File);

        // Restore permissions so cleanup can remove the directory.
        fs::set_permissions(&package_root, fs::Permissions::from_mode(0o755))
            .expect("should restore permissions");
        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn independent_package_indexes_have_boundary_local_module_ids() {
        let root = temp_dir("package_boundary_local_ids");
        let alpha_root = root.join("alpha");
        let beta_root = root.join("beta");
        fs::create_dir_all(&alpha_root).expect("should create alpha root");
        fs::create_dir_all(&beta_root).expect("should create beta root");
        fs::write(alpha_root.join("@mod.moth"), "").expect("should write alpha root");
        fs::write(beta_root.join("@mod.moth"), "").expect("should write beta root");

        let mut source_packages = SourcePackageRegistry::new();
        source_packages.register_filesystem_root(
            "alpha",
            alpha_root.clone(),
            crate::builder_surface::PackageOrigin::ProjectLocal,
        );
        source_packages.register_filesystem_root(
            "beta",
            beta_root.clone(),
            crate::builder_surface::PackageOrigin::ProjectLocal,
        );

        let mut string_table = StringTable::new();
        let indexes = build_indexes(&source_packages, &mut string_table)
            .expect("two package roots should build two boundary indexes");

        // Deterministic import-prefix order.
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

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn package_boundary_indexes_classify_provider_files_under_nearest_module() {
        let root = temp_dir("package_boundary_provider_file");
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
            .owned_source_ids(module_id)
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

        fs::remove_dir_all(&root).expect("should remove temp root");
    }
}

/// Linux preparation tests for non-UTF-8 direct-child normal module-root candidates.
///
/// Directory, single-file and config compilation all delegate source-package boundary indexing
/// to `build_source_package_boundary_indexes`, so these tests cover the shared traversal owner
/// instead of duplicating the same assertion at each orchestration boundary. macOS rejects the
/// invalid-byte fixture before the traversal can inspect it.
#[cfg(target_os = "linux")]
mod non_utf8_package_boundary_candidate_tests {
    use super::*;
    use crate::builder_surface::SourcePackageRegistry;
    use crate::compiler_frontend::compiler_errors::ErrorType;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::PathBuf;

    fn assert_non_utf8_file_error(messages: &CompilerMessages) {
        let (error_type, message, _location) = messages
            .first_infrastructure_error_for_tests()
            .expect("expected an infrastructure file error");
        assert_eq!(
            *error_type,
            ErrorType::File,
            "non-UTF-8 package boundary candidate should be a File infrastructure error"
        );
        assert!(
            message.contains("Non-UTF-8"),
            "error message should mention non-UTF-8: {message}"
        );
    }

    fn build_indexes(
        source_packages: &SourcePackageRegistry,
        string_table: &mut StringTable,
    ) -> Result<super::source_package_discovery::SourcePackageBoundaryIndexes, CompilerMessages>
    {
        super::source_package_discovery::build_source_package_boundary_indexes(
            source_packages,
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::
                ExternalImportProviderRegistry::default(),
            string_table,
        )
    }

    fn package_with_non_utf8_child() -> (PathBuf, PathBuf) {
        let root = temp_dir("package_boundary_non_utf8_candidate");
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("should create package root");

        let bad_name = OsString::from_vec(vec![0xC3, 0x28]);
        let bad_file = package_root.join(bad_name);
        fs::write(&bad_file, b"").expect("should write non-UTF-8 named file");

        (root, package_root)
    }

    #[test]
    fn invalid_candidate_without_valid_normal_module_root_returns_file_error() {
        let (root, package_root) = package_with_non_utf8_child();

        let mut source_packages = SourcePackageRegistry::new();
        source_packages.register_filesystem_root(
            "pkg",
            package_root,
            crate::builder_surface::PackageOrigin::ProjectLocal,
        );

        let mut string_table = StringTable::new();
        let messages = build_indexes(&source_packages, &mut string_table)
            .expect_err("non-UTF-8 candidate should fail boundary indexing");

        assert_non_utf8_file_error(&messages);
        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn valid_normal_module_root_plus_invalid_candidate_still_returns_file_error() {
        let (root, package_root) = package_with_non_utf8_child();
        fs::write(package_root.join("@home.moth"), b"")
            .expect("should write valid normal module root");

        let mut source_packages = SourcePackageRegistry::new();
        source_packages.register_filesystem_root(
            "pkg",
            package_root,
            crate::builder_surface::PackageOrigin::ProjectLocal,
        );

        let mut string_table = StringTable::new();
        let messages = build_indexes(&source_packages, &mut string_table)
            .expect_err("valid root plus invalid candidate should still fail");

        assert_non_utf8_file_error(&messages);
        fs::remove_dir_all(&root).expect("should remove temp root");
    }
}

/// Phase 2a module-identity and structural-ancestry tests.
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
    use crate::compiler_frontend::compiler_errors::ErrorType;
    use crate::compiler_frontend::semantic_identity::{
        ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
    };
    use std::path::{Path, PathBuf};

    fn discover_index(
        root: &std::path::Path,
        entry_root_relative: &str,
    ) -> (
        super::source_tree_index::SourceTreeIndex,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let entry_root = root.join(entry_root_relative);
        fs::create_dir_all(&entry_root).expect("should create entry root");

        let mut config = Config::new(root.to_path_buf());
        config.entry_root = PathBuf::from(entry_root_relative);
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

        (index, canonical_root, canonical_entry_root)
    }

    #[test]
    fn assigns_module_ids_in_canonical_logical_path_order() {
        let root = temp_dir("module_id_canonical_order");
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
            .map(|id| table.record(id).logical_module_path())
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

        fs::remove_dir_all(&root).expect("should remove temp root");
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
        let root = temp_dir("module_id_cosmetic_suffix");
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
            table.record(page_id).logical_module_path(),
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
            table_two.record(page_id_two).logical_module_path(),
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

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn records_explicit_root_roles_for_normal_and_support_roots() {
        let root = temp_dir("module_root_roles");
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

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn records_structural_ancestry_by_nearest_module_containment() {
        let root = temp_dir("module_ancestry");
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

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn support_roots_participate_in_structural_ancestry() {
        let root = temp_dir("module_support_ancestry");
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

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn discovers_project_package_facade_outside_entry_root_containment() {
        let root = temp_dir("module_facade_separation");
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        fs::write(src.join("@page.moth"), "").expect("should write entry module");
        fs::write(root.join("+package.moth"), "").expect("should write project facade");

        let (index, project_root, _entry_root) = discover_index(&root, "src");
        let table = index.module_identities();

        let facade_dir = project_root;
        let facade_id = table
            .module_id_for_directory(&facade_dir)
            .expect("facade should have a module id");
        assert_eq!(
            table.record(facade_id).role(),
            ModuleRootRole::ProjectPackageFacade,
        );

        // The facade is outside the entry-root containment tree.
        assert_eq!(
            table.nearest_ancestor_module(facade_id),
            None,
            "facade must have no ancestor"
        );
        assert!(
            table.direct_child_modules(facade_id).is_empty(),
            "facade must have no children"
        );

        // The facade is not an entry module. Assert against the canonical facade and entry
        // root-file paths via the project module graph (the production entry-classification
        // owner) so the exclusion is genuinely protected, not just a filename-stem check.
        let facade_root_file = fs::canonicalize(root.join("+package.moth"))
            .expect("facade root file should canonicalize");
        let entry_root_file =
            fs::canonicalize(src.join("@page.moth")).expect("entry root file should canonicalize");
        let graph = ProjectModuleGraph::from_source_tree_index(&index);
        let entry_root_files: Vec<&std::path::Path> = graph
            .entry_modules()
            .iter()
            .map(|module_id| graph.node(*module_id).root_file())
            .collect();
        assert!(
            entry_root_files.contains(&entry_root_file.as_path()),
            "entry module {entry_root_file:?} should be an entry module: {entry_root_files:?}"
        );
        assert!(
            !entry_root_files.contains(&facade_root_file.as_path()),
            "facade {facade_root_file:?} must not be an entry module: {entry_root_files:?}"
        );

        assert!(
            index.stats().project_package_facade_found,
            "facade discovery should be recorded in stats"
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn missing_project_root_surfaces_file_error_not_missing_facade() {
        let root = temp_dir("facade_missing_project_root");
        let entry_root = root.join("src");
        fs::create_dir_all(&entry_root).expect("should create entry root");
        fs::write(entry_root.join("@page.moth"), "").expect("should write entry root");

        let mut config = Config::new(root.clone());
        config.entry_root = PathBuf::from("src");
        let canonical_entry_root =
            fs::canonicalize(&entry_root).expect("entry root should canonicalize");
        let missing_project_root = root.join("does_not_exist");
        let mut string_table = StringTable::new();

        let messages = super::source_tree_index::SourceTreeIndex::discover(
            canonical_entry_root,
            super::source_tree_index::SourceTreeProjectContext {
                project_root: &missing_project_root,
                validated_output_settings: None,
            },
            &config,
            &SourcePackageRegistry::default(),
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
            &mut string_table,
        )
        .expect_err("missing project root should surface a file error, not a missing facade");

        assert_file_infrastructure_error(&messages, "discovering package facade");

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_project_root_surfaces_file_error_not_missing_facade() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_dir("facade_unreadable_project_root");
        let entry_root = root.join("src");
        fs::create_dir_all(&entry_root).expect("should create entry root");
        fs::write(entry_root.join("@page.moth"), "").expect("should write entry root");

        let mut config = Config::new(root.clone());
        config.entry_root = PathBuf::from("src");
        let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
        let canonical_entry_root =
            fs::canonicalize(&entry_root).expect("entry root should canonicalize");

        // Drop read permission so facade discovery cannot read the project root directory.
        // Execute permission is retained so the earlier canonicalization already succeeded.
        fs::set_permissions(&root, fs::Permissions::from_mode(0o300))
            .expect("should drop read permission");

        let mut string_table = StringTable::new();
        let messages = super::source_tree_index::SourceTreeIndex::discover(
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
        .expect_err("unreadable project root should surface a file error, not a missing facade");

        assert_file_infrastructure_error(&messages, "discovering package facade");

        // Restore permissions so cleanup can remove the directory.
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755))
            .expect("should restore permissions");
        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    fn assert_file_infrastructure_error(messages: &CompilerMessages, expected_text: &str) {
        use crate::compiler_frontend::compiler_errors::ErrorType;

        let (error_type, message, _location) = messages
            .first_infrastructure_error_for_tests()
            .expect("expected an infrastructure file error");
        assert_eq!(
            *error_type,
            ErrorType::File,
            "project root read failure should be a File infrastructure error"
        );
        assert!(
            message.contains(expected_text),
            "error message should mention {expected_text:?}: {message}"
        );
    }

    #[test]
    fn facade_outside_entry_root_is_not_classified_as_a_support_root() {
        let root = temp_dir("module_facade_not_support");
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        fs::write(src.join("@page.moth"), "").expect("should write entry module");
        fs::write(root.join("+package.moth"), "").expect("should write project facade");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let table = index.module_identities();

        let support_count = table
            .module_ids()
            .filter(|id| table.record(*id).role() == ModuleRootRole::Support)
            .count();
        assert_eq!(
            support_count, 0,
            "facade outside entry root must not be a support root"
        );

        let facade_count = table
            .module_ids()
            .filter(|id| table.record(*id).role() == ModuleRootRole::ProjectPackageFacade)
            .count();
        assert_eq!(facade_count, 1, "exactly one facade should be discovered");

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn rejects_multiple_normal_module_roots_in_one_directory() {
        let root = temp_dir("module_multiple_normal_module_roots");
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        fs::write(src.join("@page.moth"), "").expect("should write first root");
        fs::write(src.join("@mod.moth"), "").expect("should write second root");

        let entry_root = root.join("src");
        let mut config = Config::new(root.clone());
        config.entry_root = PathBuf::from("src");
        let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
        let canonical_entry_root =
            fs::canonicalize(&entry_root).expect("entry root should canonicalize");
        let mut string_table = StringTable::new();

        let messages = super::source_tree_index::SourceTreeIndex::discover(
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
        .expect_err("multiple normal module roots should be rejected");

        assert_eq!(first_diagnostic_code(&messages), "MOTH-CONFIG-0001");

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn rejects_mixed_normal_and_support_roots_in_one_directory() {
        let root = temp_dir("module_mixed_roots");
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        fs::write(src.join("@page.moth"), "").expect("should write normal root");
        fs::write(src.join("+pkg.moth"), "").expect("should write support root");

        let entry_root = root.join("src");
        let mut config = Config::new(root.clone());
        config.entry_root = PathBuf::from("src");
        let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
        let canonical_entry_root =
            fs::canonicalize(&entry_root).expect("entry root should canonicalize");
        let mut string_table = StringTable::new();

        let messages = super::source_tree_index::SourceTreeIndex::discover(
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
        .expect_err("mixed normal and support roots should be rejected");

        assert_eq!(first_diagnostic_code(&messages), "MOTH-CONFIG-0001");

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn rejects_multiple_support_roots_in_one_directory() {
        let root = temp_dir("module_multiple_support_roots");
        let src = root.join("src");
        fs::create_dir_all(src.join("pkg")).expect("should create support directory");
        fs::write(src.join("pkg/+one.moth"), "").expect("should write first support root");
        fs::write(src.join("pkg/+two.moth"), "").expect("should write second support root");

        let entry_root = root.join("src");
        let mut config = Config::new(root.clone());
        config.entry_root = PathBuf::from("src");
        let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
        let canonical_entry_root =
            fs::canonicalize(&entry_root).expect("entry root should canonicalize");
        let mut string_table = StringTable::new();

        let messages = super::source_tree_index::SourceTreeIndex::discover(
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

        assert_eq!(first_diagnostic_code(&messages), "MOTH-CONFIG-0001");

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn empty_table_has_no_identities_or_ancestry() {
        let table = ModuleIdentityTable::empty();
        assert_eq!(table.module_ids().count(), 0);
    }

    // ---- Phase 2b: stable cross-build origin identity ----

    /// Discover the module identity table for one checkout root with a configured project name.
    fn discover_table_with_name(
        root: &Path,
        entry_root_relative: &str,
        project_name: &str,
    ) -> (ModuleIdentityTable, std::path::PathBuf, std::path::PathBuf) {
        let entry_root = root.join(entry_root_relative);
        fs::create_dir_all(&entry_root).expect("should create entry root");

        let mut config = Config::new(root.to_path_buf());
        config.entry_root = PathBuf::from(entry_root_relative);
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
        let root_a = temp_dir("stable_identity_root_a");
        let root_b = temp_dir("stable_identity_root_b");
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

        fs::remove_dir_all(&root_a).expect("should remove root a");
        fs::remove_dir_all(&root_b).expect("should remove root b");
    }

    #[test]
    fn changing_project_name_changes_stable_identity() {
        let root_a = temp_dir("stable_identity_name_a");
        let root_b = temp_dir("stable_identity_name_b");
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

        fs::remove_dir_all(&root_a).expect("should remove root a");
        fs::remove_dir_all(&root_b).expect("should remove root b");
    }

    #[test]
    fn changing_logical_module_path_changes_stable_identity() {
        let root = temp_dir("stable_identity_path_change");
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

        fs::remove_dir_all(&root).expect("should remove root");
    }

    #[test]
    fn changing_root_role_changes_stable_identity() {
        // The facade shares the project root directory, whose logical path is empty just like the
        // entry root's, so role is the differentiator.
        let root_a = temp_dir("stable_identity_role_a");
        let root_b = temp_dir("stable_identity_role_b");
        for root in [&root_a, &root_b] {
            let src = root.join("src");
            fs::create_dir_all(&src).expect("should create entry root");
            fs::write(src.join("@home.moth"), "").expect("should write entry root");
            fs::write(root.join("+package.moth"), "").expect("should write facade");
        }

        let (table_a, project_a, entry_a) = discover_table_with_name(&root_a, "src", "my-project");
        let (table_b, project_b, entry_b) = discover_table_with_name(&root_b, "src", "my-project");

        let entry_origin = entry_module_origin(&table_a, &entry_a);
        let facade_id = table_a
            .module_id_for_directory(&project_a)
            .expect("facade should have an id");
        let facade_origin = table_a.record(facade_id).stable_origin();

        assert_eq!(
            entry_origin.logical_module_path(),
            facade_origin.logical_module_path(),
            "both the entry root and the facade have the empty logical path"
        );
        assert_eq!(entry_origin.role(), ModuleRootRole::Normal);
        assert_eq!(facade_origin.role(), ModuleRootRole::ProjectPackageFacade);
        assert_ne!(
            entry_origin, facade_origin,
            "different root roles must yield different identities even with the same logical path"
        );

        // The facade identity is itself stable across checkout roots.
        let facade_id_b = table_b
            .module_id_for_directory(&project_b)
            .expect("facade should have an id");
        assert_eq!(
            facade_origin,
            table_b.record(facade_id_b).stable_origin(),
            "facade identity must be equal across distinct absolute checkout roots"
        );
        assert_ne!(
            facade_origin,
            entry_module_origin(&table_b, &entry_b),
            "facade and entry identities must differ in the second tree too"
        );

        fs::remove_dir_all(&root_a).expect("should remove root a");
        fs::remove_dir_all(&root_b).expect("should remove root b");
    }

    #[test]
    fn cosmetic_root_suffix_rename_does_not_change_stable_identity() {
        let root_a = temp_dir("stable_identity_cosmetic_a");
        let root_b = temp_dir("stable_identity_cosmetic_b");
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

        fs::remove_dir_all(&root_a).expect("should remove root a");
        fs::remove_dir_all(&root_b).expect("should remove root b");
    }

    #[test]
    fn project_local_package_identity_preserves_configured_name_verbatim() {
        // No validation or normalization of the project name is added in this slice; the exact
        // configured name is preserved as the stable package name input.
        let identity = StablePackageIdentity::project_local("  weird/name  ");
        assert_eq!(identity.name(), "  weird/name  ");
        assert_eq!(identity.origin(), PackageOrigin::ProjectLocal);
    }

    // ---- Phase 2b correction: invalid logical-path components are rejected ----

    fn stable_origin_from_path(
        relative: &Path,
    ) -> Result<StableModuleOriginIdentity, crate::compiler_frontend::compiler_errors::CompilerError>
    {
        StableModuleOriginIdentity::from_relative_logical_path(
            StablePackageIdentity::project_local("my-project"),
            relative,
            ModuleRootRole::Normal,
        )
    }

    fn assert_internal_identity_error(
        result: Result<
            StableModuleOriginIdentity,
            crate::compiler_frontend::compiler_errors::CompilerError,
        >,
        fragment: &str,
    ) {
        let error = result.expect_err("an invalid logical path component must be rejected");
        assert_eq!(
            error.error_type,
            ErrorType::Compiler,
            "an invalid logical path component must use the internal compiler-error lane"
        );
        assert!(
            error.msg.contains(fragment),
            "internal error message should mention `{fragment}`: {}",
            error.msg
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

    fn first_diagnostic_code(messages: &CompilerMessages) -> String {
        let diagnostic = messages
            .error_diagnostics()
            .next()
            .expect("expected at least one typed error diagnostic");
        diagnostic.kind.code().to_owned()
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
    use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerMessages;
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
        config.entry_root = PathBuf::from(entry_root_relative);
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
            .owned_source_ids(module_id)
            .iter()
            .map(|source_id| owned_relative_path(index, *source_id))
            .collect()
    }

    /// Resolve one owned source's portable module-relative path through the central index.
    fn owned_relative_path(
        index: &SourceTreeIndex,
        source_id: super::source_tree_index::SourceId,
    ) -> String {
        match index.source(source_id).logical_identity() {
            SourceLogicalIdentity::Owned(identity) => identity.relative_source_path().to_owned(),
            other => {
                panic!("owned source {source_id:?} has a non-owned logical identity: {other:?}")
            }
        }
    }

    /// Resolve the source kinds for one module's owned source IDs through the central index.
    fn owned_kinds(index: &SourceTreeIndex, module_id: ModuleId) -> Vec<SourceFileKind> {
        index
            .owned_source_ids(module_id)
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
            .owned_source_ids(module_id)
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
            .unrooted_source_ids()
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
        let root = temp_dir("owned_source_nearest_owner");
        build_nested_module_tree(&root);

        let index =
            discover_index_with_kinds(&root, "src", "my-project", &html_source_file_kinds());
        let table = index.module_identities();

        let entry_id = table
            .module_ids()
            .find(|id| {
                table
                    .record(*id)
                    .logical_module_path()
                    .as_os_str()
                    .is_empty()
            })
            .expect("entry root module should exist");
        let alpha_id = table
            .module_ids()
            .find(|id| table.record(*id).logical_module_path() == Path::new("alpha"))
            .expect("alpha module should exist");
        let inner_id = table
            .module_ids()
            .find(|id| table.record(*id).logical_module_path() == Path::new("alpha/inner"))
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

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn nested_module_files_transfer_to_nested_module_not_ancestor() {
        let root = temp_dir("owned_source_nested_transfer");
        build_nested_module_tree(&root);

        let index =
            discover_index_with_kinds(&root, "src", "my-project", &html_source_file_kinds());
        let table = index.module_identities();

        let alpha_id = table
            .module_ids()
            .find(|id| table.record(*id).logical_module_path() == Path::new("alpha"))
            .expect("alpha module should exist");

        // `alpha/inner/deep.moth` is beneath alpha on the filesystem but belongs to inner because
        // the nearest-module walk finds the inner root first.
        let alpha_paths = owned_relative_paths(&index, alpha_id);
        assert!(
            !alpha_paths.contains(&"inner/deep.moth".to_owned())
                && !alpha_paths.contains(&"deep.moth".to_owned()),
            "files beneath a nested module root must transfer to the nested module: {alpha_paths:?}"
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn registered_bd_and_md_kinds_are_included() {
        let root = temp_dir("owned_source_registered_kinds");
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
                    .logical_module_path()
                    .as_os_str()
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

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn known_but_unselected_sources_are_indexed_as_unsupported() {
        let root = temp_dir("owned_source_excluded_kinds");
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
                    .logical_module_path()
                    .as_os_str()
                    .is_empty()
            })
            .expect("entry root module should exist");

        assert_eq!(
            owned_relative_paths(&index, entry_id),
            vec!["@page.moth", "content.md", "page.mtf"],
            "recognized source kinds remain indexed while unknown extensions stay excluded"
        );
        let unsupported_paths = index
            .owned_source_ids(entry_id)
            .iter()
            .filter(|source_id| !index.source(**source_id).supported())
            .map(|source_id| owned_relative_path(&index, *source_id))
            .collect::<Vec<_>>();
        assert_eq!(unsupported_paths, vec!["content.md", "page.mtf"]);
        assert!(
            index.unrooted_source_ids().is_empty(),
            "excluded files are not unrooted facts"
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn owned_entries_have_deterministic_logical_order_independent_of_creation() {
        let root = temp_dir("owned_source_deterministic_order");
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
                    .logical_module_path()
                    .as_os_str()
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

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn project_facade_owns_its_root_source() {
        let root = temp_dir("owned_source_facade_root");
        let src = root.join("src");
        fs::create_dir_all(&src).expect("should create entry root");
        fs::write(src.join("@page.moth"), "").expect("should write entry root file");
        fs::write(root.join("+package.moth"), "").expect("should write facade root file");

        let index =
            discover_index_with_kinds(&root, "src", "my-project", &html_source_file_kinds());
        let table = index.module_identities();
        let facade_id = table
            .module_ids()
            .find(|id| table.record(*id).role() == ModuleRootRole::ProjectPackageFacade)
            .expect("project package facade should exist");

        let facade_ids = index.owned_source_ids(facade_id);
        assert_eq!(
            facade_ids.len(),
            1,
            "facade module owns exactly its root source file"
        );
        let facade_record = index.source(facade_ids[0]);
        assert_eq!(
            owned_relative_path(&index, facade_ids[0]),
            "+package.moth",
            "facade root file identity is module-relative to the facade root directory"
        );
        assert_eq!(
            facade_record.classification(),
            &SourceClassification::CompilerSemantic(SourceFileKind::Moth)
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn unrooted_supported_candidates_remain_explicit_facts() {
        let root = temp_dir("owned_source_unrooted");
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
        let unrooted = index.unrooted_source_ids();
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

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn owned_source_identity_is_independent_of_checkout_root() {
        let root_a = temp_dir("owned_source_checkout_a");
        let root_b = temp_dir("owned_source_checkout_b");
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
            .find(|id| table_a.record(*id).logical_module_path() == Path::new("alpha"))
            .expect("alpha module should exist in tree a");
        let alpha_b = table_b
            .module_ids()
            .find(|id| table_b.record(*id).logical_module_path() == Path::new("alpha"))
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

        fs::remove_dir_all(&root_a).expect("should remove root a");
        fs::remove_dir_all(&root_b).expect("should remove root b");
    }

    #[test]
    fn unknown_registered_extension_is_excluded() {
        let root = temp_dir("owned_source_unknown_registered_extension");
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
                    .logical_module_path()
                    .as_os_str()
                    .is_empty()
            })
            .expect("entry root module should exist");

        assert_eq!(
            owned_relative_paths(&index, entry_id),
            vec!["@page.moth"],
            "an arbitrary registered unknown extension must not enter owned source sets"
        );
        assert!(
            index.unrooted_source_ids().is_empty(),
            "an excluded unknown registered extension is not an unrooted fact"
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn mismatched_known_extension_mapping_is_indexed_as_unsupported() {
        let root = temp_dir("owned_source_mismatched_mapping");
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
                    .logical_module_path()
                    .as_os_str()
                    .is_empty()
            })
            .expect("entry root module should exist");

        assert_eq!(
            owned_relative_paths(&index, entry_id),
            vec!["@page.moth", "page.mtf"],
            "recognized source identity is independent of active builder support"
        );
        let page_source_id = index
            .owned_source_ids(entry_id)
            .iter()
            .copied()
            .find(|source_id| owned_relative_path(&index, *source_id) == "page.mtf")
            .expect("recognized Moth template should have a source ID");
        assert!(!index.source(page_source_id).supported());
        assert!(
            index.unrooted_source_ids().is_empty(),
            "an excluded mismatched mapping is not an unrooted fact"
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn unrooted_source_ids_are_ordered_by_portable_logical_path_across_roots() {
        // Two distinct checkout roots with unrooted files created in reverse-logical order.
        // The unrooted source IDs must sort by portable entry-root-relative logical path,
        // not by absolute checkout path or creation order.
        let root_a = temp_dir("unrooted_logical_order_a");
        let root_b = temp_dir("unrooted_logical_order_b");

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

        fs::remove_dir_all(&root_a).expect("should remove root a");
        fs::remove_dir_all(&root_b).expect("should remove root b");
    }

    #[test]
    fn facade_file_inside_entry_root_is_owned_exactly_once_by_facade() {
        // The current compatibility case: project root equals entry root, so the facade root
        // file lies inside the traversal. It must appear exactly once, owned only by the facade
        // module, and must not also appear in the entry-root module's owned source set.
        let root = temp_dir("facade_exact_once_same_root");
        fs::create_dir_all(&root).expect("should create entry root");
        fs::write(root.join("@page.moth"), "").expect("should write entry root file");
        fs::write(root.join("+package.moth"), "").expect("should write facade root file");

        let index = discover_index_with_kinds(&root, ".", "my-project", &html_source_file_kinds());
        let table = index.module_identities();

        let facade_id = table
            .module_ids()
            .find(|id| table.record(*id).role() == ModuleRootRole::ProjectPackageFacade)
            .expect("project package facade should exist");
        let entry_id = table
            .module_ids()
            .find(|id| {
                table.record(*id).role() == ModuleRootRole::Normal
                    && table
                        .record(*id)
                        .logical_module_path()
                        .as_os_str()
                        .is_empty()
            })
            .expect("entry root normal module should exist");

        let facade_ids = index.owned_source_ids(facade_id);
        assert_eq!(
            facade_ids.len(),
            1,
            "facade module owns exactly its root source file"
        );
        assert_eq!(
            owned_relative_path(&index, facade_ids[0]),
            "+package.moth",
            "facade root file identity is module-relative to the facade root directory"
        );

        let entry_paths = owned_relative_paths(&index, entry_id);
        assert!(
            !entry_paths.contains(&"+package.moth".to_owned()),
            "the facade file must not appear in the entry-root normal module's owned set: \
             {entry_paths:?}"
        );
        assert_eq!(
            entry_paths,
            vec!["@page.moth"],
            "entry-root normal module owns only its own root file"
        );

        let facade_record_count = index
            .sources()
            .iter()
            .filter(|record| match record.logical_identity() {
                SourceLogicalIdentity::Owned(identity) => {
                    identity.relative_source_path() == "+package.moth"
                        && identity.module_origin() == table.record(facade_id).stable_origin()
                }
                SourceLogicalIdentity::Unrooted(_) => false,
            })
            .count();
        assert_eq!(
            facade_record_count, 1,
            "the facade source must appear exactly once in the central source table"
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn compilation_root_table_excludes_facade_when_it_shares_entry_root() {
        let root = temp_dir("facade_resolver_root_table");
        fs::create_dir_all(&root).expect("should create entry root");
        fs::write(root.join("@page.moth"), "").expect("should write entry root file");
        fs::write(root.join("+package.moth"), "").expect("should write facade root file");

        let index = discover_index_with_kinds(&root, ".", "my-project", &html_source_file_kinds());
        let table = index.module_identities();
        let compilation_roots = table.derive_compilation_root_table();
        let canonical_root = fs::canonicalize(&root).expect("test root should canonicalize");

        assert_eq!(
            compilation_roots.root_file_for_directory(&canonical_root),
            Some(canonical_root.join("@page.moth").as_path()),
            "the normal module remains authoritative for the shared project directory"
        );
        assert!(
            !compilation_roots.is_root_file(&canonical_root.join("+package.moth")),
            "the project facade must not become a resolver module root"
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn source_ids_equal_their_contiguous_table_index() {
        let root = temp_dir("source_id_contiguous_index");
        build_nested_module_tree(&root);
        let index =
            discover_index_with_kinds(&root, "src", "my-project", &html_source_file_kinds());

        let sources = index.sources();
        for (position, record) in sources.iter().enumerate() {
            assert_eq!(
                record.id().index(),
                position,
                "each SourceId must equal its contiguous table index"
            );
        }

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn every_supported_file_appears_exactly_once() {
        let root = temp_dir("source_id_exact_once");
        build_nested_module_tree(&root);
        let index =
            discover_index_with_kinds(&root, "src", "my-project", &html_source_file_kinds());

        let mut referenced_ids = Vec::new();
        for module_id in index.module_identities().module_ids() {
            referenced_ids.extend(
                index
                    .owned_source_ids(module_id)
                    .iter()
                    .map(|source_id| source_id.index()),
            );
        }
        referenced_ids.extend(
            index
                .unrooted_source_ids()
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

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn owned_and_unrooted_sets_reference_valid_records_with_matching_state() {
        let root = temp_dir("source_id_matching_state");
        build_nested_module_tree(&root);
        let index =
            discover_index_with_kinds(&root, "src", "my-project", &html_source_file_kinds());

        for module_id in index.module_identities().module_ids() {
            for source_id in index.owned_source_ids(module_id) {
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

        let unrooted_root = temp_dir("source_id_matching_unrooted_state");
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
            unrooted_index.unrooted_source_ids().len(),
            1,
            "the focused unrooted tree must exercise one unrooted record"
        );
        for source_id in unrooted_index.unrooted_source_ids() {
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

        fs::remove_dir_all(&root).expect("should remove temp root");
        fs::remove_dir_all(&unrooted_root).expect("should remove unrooted temp root");
    }

    #[test]
    fn source_ids_are_deterministic_across_creation_order_and_checkout_roots() {
        // The same logical tree built in two distinct checkout roots with reverse creation order
        // must assign identical SourceId -> logical identity mappings, because SourceIds are
        // derived from portable logical identity, not traversal or absolute paths.
        let root_a = temp_dir("source_id_deterministic_a");
        let root_b = temp_dir("source_id_deterministic_b");

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

        fs::remove_dir_all(&root_a).expect("should remove root a");
        fs::remove_dir_all(&root_b).expect("should remove root b");
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
        ) -> Result<Option<ResolvedExternalImport>, CompilerMessages> {
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
        config.entry_root = PathBuf::from(entry_root_relative);
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

    #[test]
    fn provider_owned_files_are_classified_and_owned_by_nearest_module() {
        let root = temp_dir("provider_owned_classification");
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
                    .logical_module_path()
                    .as_os_str()
                    .is_empty()
            })
            .expect("entry root module should exist");
        let feature_id = table
            .module_ids()
            .find(|id| table.record(*id).logical_module_path() == Path::new("feature"))
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

        // The logical-path lookup map resolves both provider targets by entry-root-relative path.
        let helper_id = index
            .source_id_for_entry_root_relative_logical_path("helper.js")
            .expect("helper.js logical path resolves to a SourceId");
        assert_eq!(
            index.source(helper_id).canonical_path(),
            helper_record.canonical_path()
        );
        let util_id = index
            .source_id_for_entry_root_relative_logical_path("feature/util.js")
            .expect("feature/util.js logical path resolves to a SourceId");
        assert_eq!(
            index.source(util_id).canonical_path(),
            util_record.canonical_path()
        );

        // The canonical-path lookup map resolves an importer to its owning record.
        let page_canonical = page_record.canonical_path();
        let page_lookup_id = index
            .source_id_for_canonical_path(page_canonical)
            .expect("page canonical path resolves to a SourceId");
        assert_eq!(
            index.source(page_lookup_id).ownership(),
            SourceOwnership::Owned(entry_id)
        );

        // SourceIds are assigned in deterministic portable logical identity order. The root
        // module precedes the child module, so helper.js precedes feature/util.js regardless of
        // traversal or creation order.
        assert!(
            helper_id.index() < util_id.index(),
            "provider SourceIds must follow deterministic logical identity order"
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn provider_owned_files_without_a_registered_provider_are_not_indexed() {
        let root = temp_dir("provider_owned_unregistered_extension");
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
                    .logical_module_path()
                    .as_os_str()
                    .is_empty()
            })
            .expect("entry root module should exist");

        assert!(
            owned_source_for_path(&index, entry_id, "drawing.wit").is_none(),
            "files whose extension has no registered provider must not be indexed"
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }
}

// ---- Phase 5a: canonical structural project module graph ----

mod project_module_graph_tests {
    use super::*;
    use crate::builder_surface::SourcePackageRegistry;
    use crate::compiler_frontend::compiler_errors::ErrorType;
    use crate::compiler_frontend::semantic_identity::ModuleRootRole;
    use crate::compiler_frontend::symbols::string_interning::StringTable;
    use std::path::PathBuf;

    use super::module_identity::ModuleId;
    use super::project_module_graph::{DependencyEdgeOutcome, ProjectModuleGraph};
    use super::source_tree_index::SourceTreeIndex;

    fn discover_index(
        root: &std::path::Path,
        entry_root_relative: &str,
    ) -> (SourceTreeIndex, std::path::PathBuf, std::path::PathBuf) {
        let entry_root = root.join(entry_root_relative);
        fs::create_dir_all(&entry_root).expect("should create entry root");

        let mut config = Config::new(root.to_path_buf());
        config.entry_root = PathBuf::from(entry_root_relative);
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
                    && table
                        .record(*id)
                        .logical_module_path()
                        .to_str()
                        .map(|path| path == logical_path)
                        .unwrap_or(false)
            })
            .unwrap_or_else(|| {
                panic!("expected a {role:?} module with logical path {logical_path:?}")
            })
    }

    #[test]
    fn nodes_are_stored_in_deterministic_module_id_order() {
        let root = temp_dir("graph_node_order");
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
            assert_eq!(graph_node.root_directory(), record.root_directory());
            assert_eq!(graph_node.root_file(), record.root_file());
            assert_eq!(
                graph_node.nearest_parent(),
                index.module_identities().nearest_ancestor_module(table_id)
            );
            assert_eq!(
                graph_node.direct_children(),
                index.module_identities().direct_child_modules(table_id)
            );
            // The graph node carries no source records: owned source data lives in the central
            // index. Verify the index's owned source IDs for this module all resolve to records
            // owned by this module, proving the graph delegates ownership to the index.
            for source_id in index.owned_source_ids(table_id) {
                let record = index.source(*source_id);
                assert_eq!(
                    record.ownership(),
                    super::source_tree_index::SourceOwnership::Owned(table_id),
                    "each owned source ID must resolve to a record owned by its module"
                );
            }
        }

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn entry_modules_are_normal_only_and_facade_is_separate() {
        let root = temp_dir("graph_entries_and_facade");
        let src = root.join("src");
        fs::create_dir_all(src.join("pages")).expect("should create pages");
        fs::create_dir_all(src.join("components")).expect("should create components");

        fs::write(src.join("@site.moth"), "").expect("should write entry normal root");
        fs::write(src.join("pages/@pages.moth"), "").expect("should write child normal root");
        fs::write(src.join("components/+ui.moth"), "").expect("should write support root");
        fs::write(root.join("+package.moth"), "").expect("should write project facade");

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

        let facade_id = graph
            .facade()
            .expect("project package facade should be classified");
        assert_eq!(
            graph.node(facade_id).role(),
            ModuleRootRole::ProjectPackageFacade
        );
        assert!(
            !graph.entry_modules().contains(&facade_id),
            "facade must never be an entry candidate"
        );
        assert_eq!(
            graph.node(facade_id).nearest_parent(),
            None,
            "facade stays outside the normal ancestry tree"
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn support_visibility_is_visible_to_owner_and_normal_descendants_outside_subtree() {
        let root = temp_dir("graph_support_visibility");
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

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn support_visibility_enforces_private_same_scope_and_outer_scope_boundaries() {
        let root = temp_dir("graph_support_visibility_rejections");
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
        // A support facade in a strictly nested normal scope may import outer support packages.
        assert!(
            graph.is_support_visible_to_consumer(support_id, extras_id),
            "nested support facade should see a support package from a strictly outer scope"
        );
        // A non-support module id is not a valid support argument.
        assert!(
            !graph.is_support_visible_to_consumer(parser_id, support_id),
            "querying visibility for a non-support module returns false"
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn support_visibility_rejects_modules_outside_owner_subtree() {
        let root = temp_dir("graph_support_visibility_outside");
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

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn independent_ready_modules_share_wave_zero_in_module_id_order() {
        let root = temp_dir("graph_independent_waves");
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

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn facade_is_ordered_by_real_edges_not_a_fake_dependency() {
        let root = temp_dir("graph_facade_order");
        let src = root.join("src");
        fs::create_dir_all(src.join("pages")).expect("should create pages");

        fs::write(src.join("@site.moth"), "").expect("should write entry normal root");
        fs::write(src.join("pages/@pages.moth"), "").expect("should write child normal root");
        fs::write(root.join("+package.moth"), "").expect("should write project facade");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");

        let facade_id = ProjectModuleGraph::from_source_tree_index(&index)
            .facade()
            .expect("project package facade should be classified");
        let pages_id = module_id_for(&index, ModuleRootRole::Normal, "pages");
        let site_id = module_id_for(&index, ModuleRootRole::Normal, "");

        // Without edges the facade is independent and joins wave zero with the normal modules.
        // The no-edge graph completes before scheduling so wave scheduling reads only frozen
        // adjacency.
        let mut no_edge_graph = ProjectModuleGraph::from_source_tree_index(&index);
        no_edge_graph
            .complete()
            .expect("no-edge graph should complete before scheduling");
        let waves = no_edge_graph
            .compile_waves()
            .expect("completed no-edge graph waves cleanly");
        assert_eq!(waves.len(), 1, "no edges means one wave");
        assert!(
            waves[0].contains(&facade_id),
            "facade with no edges is independent and ready in wave zero"
        );

        // Once a real edge targets the facade, it is ordered after its providers without any
        // hard-coded fake dependency. Edges are inserted while the graph is under construction,
        // then completion freezes the adjacency before scheduling reads it.
        let mut graph = ProjectModuleGraph::from_source_tree_index(&index);
        graph
            .add_dependency_edge(pages_id, facade_id)
            .expect("pages -> facade edge should insert");
        graph
            .add_dependency_edge(site_id, facade_id)
            .expect("site -> facade edge should insert");
        graph
            .complete()
            .expect("populated graph should complete before scheduling");

        let waves = graph
            .compile_waves()
            .expect("facade-ordered graph waves cleanly");
        let facade_wave = waves
            .iter()
            .position(|wave| wave.contains(&facade_id))
            .expect("facade should appear in a wave");
        let pages_wave = waves
            .iter()
            .position(|wave| wave.contains(&pages_id))
            .expect("pages should appear in a wave");
        let site_wave = waves
            .iter()
            .position(|wave| wave.contains(&site_id))
            .expect("site should appear in a wave");
        assert!(
            facade_wave > pages_wave && facade_wave > site_wave,
            "facade must compile after both providers that target it"
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn dependency_cycle_reports_blocked_modules_as_internal_error() {
        let root = temp_dir("graph_cycle_detection");
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

        let cycle_error = graph
            .compile_waves()
            .expect_err("a dependency cycle must surface as an internal graph failure");
        assert_eq!(
            cycle_error.error_type,
            ErrorType::Compiler,
            "a defensive cycle is an internal compiler graph failure"
        );
        let message = &cycle_error.msg;
        assert!(
            message.contains("cycle"),
            "cycle error must name the cycle: {message}"
        );
        // Deterministic blocked-module reporting includes both modules on the cycle.
        assert!(
            message.contains("alpha") && message.contains("beta"),
            "cycle error must name the involved modules: {message}"
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn self_edge_and_invalid_ids_are_rejected_without_panicking() {
        let root = temp_dir("graph_edge_validation");
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
        assert_eq!(self_error.error_type, ErrorType::Compiler);

        let out_of_range = ModuleId::from_index(graph.node_count() + 10);
        let invalid_error = graph
            .add_dependency_edge(out_of_range, alpha_id)
            .expect_err("an out-of-range module id must be rejected");
        assert_eq!(invalid_error.error_type, ErrorType::Compiler);

        // The graph remains usable for deterministic waves after rejected edges. Rejected edges
        // do not mutate the graph, so it still completes cleanly with no accepted edges.
        graph
            .complete()
            .expect("graph should complete before scheduling after rejected edges");
        let waves = graph
            .compile_waves()
            .expect("rejected edges do not mutate the graph");
        assert_eq!(waves.len(), 1, "no accepted edges means one ready wave");

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn frozen_adjacency_orders_consumers_in_module_id_order() {
        // Hidden invariant: completion freezes provider/consumer adjacency into sorted
        // `Vec<ModuleId>` storage, so the consumer wave preserves `ModuleId` order regardless of
        // edge insertion order. The frozen order is observable through the wave that follows the
        // provider.
        let root = temp_dir("graph_frozen_consumer_order");
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

        fs::remove_dir_all(&root).expect("should remove temp root");
    }

    #[test]
    fn invalid_graph_phase_use_reports_internal_errors() {
        // Hidden invariant: the construction-to-completion phase is an explicit boundary.
        // Scheduling before completion, mutation after completion and double completion all
        // surface as internal `CompilerError`s without panicking, and a completed graph still
        // schedules cleanly from its frozen adjacency.
        let root = temp_dir("graph_phase_validation");
        let src = root.join("src");
        fs::create_dir_all(src.join("alpha")).expect("should create alpha");
        fs::create_dir_all(src.join("beta")).expect("should create beta");

        fs::write(src.join("@home.moth"), "").expect("should write entry root");
        fs::write(src.join("alpha/@alpha.moth"), "").expect("should write alpha root");
        fs::write(src.join("beta/@beta.moth"), "").expect("should write beta root");

        let (index, _project_root, _entry_root) = discover_index(&root, "src");
        let alpha_id = module_id_for(&index, ModuleRootRole::Normal, "alpha");
        let beta_id = module_id_for(&index, ModuleRootRole::Normal, "beta");

        // Scheduling before completion is an internal compiler failure.
        let uncompleted = ProjectModuleGraph::from_source_tree_index(&index);
        let scheduling_error = uncompleted
            .compile_waves()
            .expect_err("scheduling before completion must be rejected");
        assert_eq!(scheduling_error.error_type, ErrorType::Compiler);
        assert!(
            scheduling_error.msg.contains("before completion"),
            "scheduling error must name the phase violation: {}",
            scheduling_error.msg
        );

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
        assert_eq!(mutation_error.error_type, ErrorType::Compiler);
        assert!(
            mutation_error.msg.contains("after completion"),
            "mutation error must name the phase violation: {}",
            mutation_error.msg
        );

        let double_complete_error = completed
            .complete()
            .expect_err("double completion must be rejected as mutation after completion");
        assert_eq!(double_complete_error.error_type, ErrorType::Compiler);
        assert!(
            double_complete_error.msg.contains("after completion"),
            "double completion error must name the phase violation: {}",
            double_complete_error.msg
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

        fs::remove_dir_all(&root).expect("should remove temp root");
    }
}
