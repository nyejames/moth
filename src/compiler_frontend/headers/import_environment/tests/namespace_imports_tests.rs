//! Tests for namespace-record construction in the header import environment.
//!
//! WHAT: covers recursive external package records and source receiver-method filtering.
//! WHY: AST must consume namespace visibility without rebuilding import surfaces, so this
//! header-stage data shape needs direct coverage.

use super::*;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalCoreTraitIdentity, CanonicalEvidenceIdentity,
    CanonicalTraitIdentity, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticKind, DiagnosticPayload, ImportDiagnosticKind,
};
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalConstantDef, ExternalConstantId, ExternalConstantValue,
    ExternalFunctionDef, ExternalFunctionId, ExternalFunctionLowerings, ExternalPackageRegistry,
    ExternalReturnAlias, ExternalSymbolId, ExternalSymbolPath, ExternalTypeDef, ExternalTypeId,
    external_success_returns,
};
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use crate::compiler_frontend::headers::import_environment::{
    ImportEnvironmentInput, prepare_import_environment,
};
use crate::compiler_frontend::headers::module_symbols::{ModuleRootBoundary, ModuleSymbols};
use crate::compiler_frontend::headers::types::{FileImport, HeaderExportMode};
use crate::compiler_frontend::paths::const_paths::RetainedProviderReference;
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallSummary,
};
use crate::compiler_frontend::public_interface::{
    ConcreteCallSummaryRecord, ProviderImportKind, PublicConstantSemantics,
    PublicDeclarationRecord, PublicDeclarationSemantics, PublicEvidenceOwnership,
    PublicEvidenceRecord, PublicReceiverMethodCategory, PublicReceiverMethodSemantics,
    PublicSemanticInterface, PublicStructSemantics, SourceProviderImport, SourceProviderImportSet,
};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, ModuleRootRole, OriginConstantId, OriginDeclarationId, OriginFunctionId,
    OriginTypeCategory, OriginTypeId, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::identity::{FileId, ImportShellId};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use rustc_hash::{FxHashMap, FxHashSet};

fn intern_path(components: &[&str], string_table: &mut StringTable) -> InternedPath {
    InternedPath::from_components(
        components
            .iter()
            .map(|component| string_table.intern(component))
            .collect(),
    )
}

fn location_for(path_components: &[&str], string_table: &mut StringTable) -> SourceLocation {
    SourceLocation::new(
        intern_path(path_components, string_table),
        Default::default(),
        Default::default(),
    )
}

fn empty_void_function(name: &str) -> ExternalFunctionDef {
    ExternalFunctionDef {
        name: name.to_owned(),
        parameters: Vec::new(),
        returns: external_success_returns(ExternalAbiType::Void, ExternalReturnAlias::Fresh),
        error_return_type: None,
        lowerings: ExternalFunctionLowerings::default(),
    }
}

fn test_import(header_path: InternedPath, string_table: &mut StringTable) -> FileImport {
    let provider = RetainedProviderReference {
        path: header_path,
        path_location: location_for(&["src", "@page.moth"], string_table),
        from_grouped: false,
        import_shell_id: ImportShellId::new(FileId(0), 0),
    };
    FileImport {
        authored_provider: provider.clone(),
        provider,
        alias: None,
        location: location_for(&["src", "@page.moth"], string_table),
        alias_location: None,
        from_grouped: false,
        export_mode: HeaderExportMode::Private,
    }
}

fn assert_duplicate_import_surface_member(error: CompilerDiagnostic) {
    assert_eq!(
        error.kind,
        DiagnosticKind::Import(ImportDiagnosticKind::DuplicateImportSurfaceMember)
    );
}

fn import_error_diagnostic(
    error: crate::compiler_frontend::headers::import_environment::ImportEnvironmentError,
) -> CompilerDiagnostic {
    match error {
        crate::compiler_frontend::headers::import_environment::ImportEnvironmentError::Diagnostic(
            diagnostic,
        ) => *diagnostic,
        crate::compiler_frontend::headers::import_environment::ImportEnvironmentError::Internal(
            error,
        ) => panic!("expected import diagnostic, got internal error: {error:?}"),
    }
}

#[test]
fn external_nested_namespace_tree_builds_correctly() {
    let mut registry = ExternalPackageRegistry::new();
    let package_id = registry
        .register_package("@test/path", crate::builder_surface::PackageOrigin::Builder)
        .expect("test package should register");

    registry
        .register_function_at_path(
            package_id,
            ExternalSymbolPath::from_components(vec!["input".to_owned(), "new".to_owned()]),
            ExternalFunctionId::Synthetic(100),
            empty_void_function("new"),
        )
        .expect("nested function should register");
    registry
        .register_function_at_path(
            package_id,
            ExternalSymbolPath::from_components(vec!["debug".to_owned(), "new".to_owned()]),
            ExternalFunctionId::Synthetic(103),
            empty_void_function("new"),
        )
        .expect("same leaf under a different child namespace should register");
    registry
        .register_type_at_path(
            package_id,
            ExternalSymbolPath::from_components(vec!["input".to_owned(), "Input".to_owned()]),
            ExternalTypeId(101),
            ExternalTypeDef {
                name: "Input".to_owned(),
                package_id,
                abi_type: ExternalAbiType::Handle,
            },
        )
        .expect("nested type should register");
    registry
        .register_constant_at_path(
            package_id,
            ExternalSymbolPath::from_components(vec!["input".to_owned(), "DEFAULT".to_owned()]),
            ExternalConstantId(102),
            ExternalConstantDef {
                name: "DEFAULT".to_owned(),
                data_type: ExternalAbiType::I32,
                value: ExternalConstantValue::Int(1),
            },
        )
        .expect("nested constant should register");

    let mut string_table = StringTable::new();
    let source_file = intern_path(&["src", "@page.moth"], &mut string_table);
    let import_path = intern_path(&["test", "path"], &mut string_table);
    let import = test_import(import_path, &mut string_table);

    let mut module_symbols = ModuleSymbols::empty();
    module_symbols.module_file_paths.insert(source_file.clone());
    module_symbols
        .file_imports_by_source
        .insert(source_file.clone(), vec![import]);

    let external_import_resolution_table = ExternalImportResolutionTable::new();
    let environment = prepare_import_environment(ImportEnvironmentInput {
        module_symbols: &mut module_symbols,
        external_package_registry: &registry,
        external_import_resolution_table: &external_import_resolution_table,
        source_provider_imports: &Default::default(),
        string_table: &mut string_table,
    })
    .expect("external namespace import should prepare");

    let namespace_name = string_table.intern("path");
    let input_name = string_table.intern("input");
    let debug_name = string_table.intern("debug");
    let new_name = string_table.intern("new");
    let input_type_name = string_table.intern("Input");
    let default_name = string_table.intern("DEFAULT");

    let visibility = environment
        .visibility_for(&source_file)
        .expect("source file visibility should exist");
    let record = visibility
        .visible_namespace_records
        .get(&namespace_name)
        .expect("bare package import should create a namespace record");

    assert!(!record.value_members.contains_key(&input_name));
    assert!(!record.type_members.contains_key(&input_name));

    let input_record = record
        .child_namespaces
        .get(&input_name)
        .expect("input child namespace should exist");

    assert!(matches!(
        input_record.value_members.get(&new_name),
        Some(NamespaceValueMember::ExternalSymbol(
            ExternalSymbolId::Function(ExternalFunctionId::Synthetic(100))
        ))
    ));
    assert!(matches!(
        input_record.type_members.get(&input_type_name),
        Some(NamespaceTypeMember::ExternalSymbol(ExternalSymbolId::Type(
            ExternalTypeId(101)
        )))
    ));
    assert!(matches!(
        input_record.value_members.get(&default_name),
        Some(NamespaceValueMember::ExternalSymbol(
            ExternalSymbolId::Constant(ExternalConstantId(102))
        ))
    ));

    let debug_record = record
        .child_namespaces
        .get(&debug_name)
        .expect("debug child namespace should exist");
    assert!(matches!(
        debug_record.value_members.get(&new_name),
        Some(NamespaceValueMember::ExternalSymbol(
            ExternalSymbolId::Function(ExternalFunctionId::Synthetic(103))
        ))
    ));
}

#[test]
fn duplicate_external_namespace_value_and_type_slot_is_rejected() {
    let mut string_table = StringTable::new();
    let location = location_for(&["src", "@page.moth"], &mut string_table);
    let surface_path = intern_path(&["test", "path"], &mut string_table);
    let test_package = string_table.intern("@test");
    let mut record = NamespaceRecord::empty(NamespaceRecordSource::ExternalPackage(test_package));

    let mut inserter = ExternalNamespaceRecordInserter {
        string_table: &mut string_table,
        location: &location,
    };

    inserter
        .insert(
            &mut record,
            &ExternalSymbolPath::from_single("same"),
            ExternalSymbolId::Function(ExternalFunctionId::Synthetic(200)),
            &surface_path,
        )
        .expect("first value member should insert");

    let error = inserter
        .insert(
            &mut record,
            &ExternalSymbolPath::from_single("same"),
            ExternalSymbolId::Type(ExternalTypeId(201)),
            &surface_path,
        )
        .expect_err("value/type slot collision should fail");

    assert_duplicate_import_surface_member(import_error_diagnostic(error));
}

#[test]
fn duplicate_external_namespace_and_value_slot_is_rejected() {
    let mut string_table = StringTable::new();
    let location = location_for(&["src", "@page.moth"], &mut string_table);
    let surface_path = intern_path(&["test", "path"], &mut string_table);
    let test_package = string_table.intern("@test");
    let mut record = NamespaceRecord::empty(NamespaceRecordSource::ExternalPackage(test_package));

    let mut inserter = ExternalNamespaceRecordInserter {
        string_table: &mut string_table,
        location: &location,
    };

    inserter
        .insert(
            &mut record,
            &ExternalSymbolPath::from_single("input"),
            ExternalSymbolId::Function(ExternalFunctionId::Synthetic(300)),
            &surface_path,
        )
        .expect("first value member should insert");

    let error = inserter
        .insert(
            &mut record,
            &ExternalSymbolPath::from_components(vec!["input".to_owned(), "new".to_owned()]),
            ExternalSymbolId::Function(ExternalFunctionId::Synthetic(301)),
            &surface_path,
        )
        .expect_err("namespace/value slot collision should fail");

    assert_duplicate_import_surface_member(import_error_diagnostic(error));
}

#[test]
fn duplicate_external_namespace_and_type_slot_is_rejected() {
    let mut string_table = StringTable::new();
    let location = location_for(&["src", "@page.moth"], &mut string_table);
    let surface_path = intern_path(&["test", "path"], &mut string_table);
    let test_package = string_table.intern("@test");
    let mut record = NamespaceRecord::empty(NamespaceRecordSource::ExternalPackage(test_package));

    let mut inserter = ExternalNamespaceRecordInserter {
        string_table: &mut string_table,
        location: &location,
    };

    inserter
        .insert(
            &mut record,
            &ExternalSymbolPath::from_single("input"),
            ExternalSymbolId::Type(ExternalTypeId(400)),
            &surface_path,
        )
        .expect("first type member should insert");

    let error = inserter
        .insert(
            &mut record,
            &ExternalSymbolPath::from_components(vec!["input".to_owned(), "new".to_owned()]),
            ExternalSymbolId::Function(ExternalFunctionId::Synthetic(401)),
            &surface_path,
        )
        .expect_err("namespace/type slot collision should fail");

    assert_duplicate_import_surface_member(import_error_diagnostic(error));
}

#[test]
fn source_receiver_methods_remain_absent_from_namespace_records() {
    let mut string_table = StringTable::new();
    let helper_file = intern_path(&["src", "helper.moth"], &mut string_table);
    let method_path = intern_path(&["src", "helper", "tick"], &mut string_table);
    let location = location_for(&["src", "@page.moth"], &mut string_table);
    let method_name = method_path
        .name()
        .expect("method path should have a leaf name");

    let mut declared_paths = FxHashSet::default();
    declared_paths.insert(method_path.clone());

    let mut module_symbols = ModuleSymbols::empty();
    module_symbols
        .declared_paths_by_file
        .insert(helper_file.clone(), declared_paths);
    module_symbols
        .importable_source_symbol_paths
        .insert(method_path.clone());
    module_symbols.receiver_method_paths.insert(method_path);

    let registry = ExternalPackageRegistry::new();
    let external_import_resolution_table = ExternalImportResolutionTable::new();
    let source_provider_imports = Default::default();
    let builder = ImportEnvironmentBuilder {
        module_symbols: &module_symbols,
        external_package_registry: &registry,
        external_import_resolution_table: &external_import_resolution_table,
        source_provider_imports: &source_provider_imports,
        string_table: &mut string_table,
        environment: Default::default(),
        warnings: Vec::new(),
        provider_semantics_imported: Default::default(),
    };

    let record = builder
        .build_source_namespace_record(&helper_file, &location)
        .expect("source namespace record should build");

    assert!(!record.value_members.contains_key(&method_name));
    assert!(!record.type_members.contains_key(&method_name));
    assert!(record.child_namespaces.is_empty());
}

#[test]
fn module_root_namespace_uses_prepared_root_file_identity() {
    let mut string_table = StringTable::new();
    let source_file = intern_path(&["src", "@page.moth"], &mut string_table);
    let module_root = intern_path(&["helper-root"], &mut string_table);
    let root_file = intern_path(&["helper", "@home.moth"], &mut string_table);
    let import = test_import(
        intern_path(&["helper"], &mut string_table),
        &mut string_table,
    );

    let mut module_symbols = ModuleSymbols::empty();
    module_symbols.module_file_paths.insert(source_file.clone());
    module_symbols.module_file_paths.insert(root_file.clone());
    module_symbols.file_module_membership.insert(
        source_file.clone(),
        intern_path(&["entry-root"], &mut string_table),
    );
    module_symbols
        .file_module_membership
        .insert(root_file.clone(), module_root.clone());
    module_symbols
        .module_root_boundaries
        .push(ModuleRootBoundary {
            import_prefix: intern_path(&["helper"], &mut string_table),
            module_root,
            root_file: root_file.clone(),
        });

    let registry = ExternalPackageRegistry::new();
    let external_import_resolution_table = ExternalImportResolutionTable::new();
    let source_provider_imports = Default::default();
    let mut builder = ImportEnvironmentBuilder {
        module_symbols: &module_symbols,
        external_package_registry: &registry,
        external_import_resolution_table: &external_import_resolution_table,
        source_provider_imports: &source_provider_imports,
        string_table: &mut string_table,
        environment: Default::default(),
        warnings: Vec::new(),
        provider_semantics_imported: Default::default(),
    };

    let Some(ResolvedNamespaceTarget::SourceFile(path)) =
        builder.resolve_module_root_public_export(&import.provider.path, &source_file)
    else {
        panic!("module root namespace should use the prepared root file");
    };
    assert_eq!(path, root_file);
}

#[test]
fn prelude_symbol_visibility_has_no_authored_location() {
    let mut registry = ExternalPackageRegistry::new();
    let package_id = registry
        .register_package(
            "@test/prelude_symbols",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("test package registration should not collide");
    let function_id = ExternalFunctionId::Synthetic(4_900);
    registry
        .register_function_at_path(
            package_id,
            ExternalSymbolPath::from_single("prelude_fn"),
            function_id,
            empty_void_function("prelude_fn"),
        )
        .expect("test function registration should not collide");
    registry
        .register_prelude_symbol("prelude_fn", ExternalSymbolId::Function(function_id))
        .expect("prelude symbol registration should not collide");

    let mut string_table = StringTable::new();
    let source_file = intern_path(&["src", "@page.moth"], &mut string_table);
    let mut module_symbols = ModuleSymbols::empty();
    module_symbols.module_file_paths.insert(source_file.clone());

    let environment = prepare_import_environment(ImportEnvironmentInput {
        module_symbols: &mut module_symbols,
        external_package_registry: &registry,
        external_import_resolution_table: &ExternalImportResolutionTable::new(),
        source_provider_imports: &Default::default(),
        string_table: &mut string_table,
    })
    .expect("prelude symbol visibility should prepare");

    let prelude_name = string_table.intern("prelude_fn");
    let visibility = environment
        .visibility_for(&source_file)
        .expect("source file visibility should exist");
    assert_eq!(
        visibility.visible_external_symbols.get(&prelude_name),
        Some(&ExternalSymbolId::Function(function_id))
    );
    assert!(
        !visibility
            .visible_external_symbol_locations
            .contains_key(&prelude_name),
        "compiler-injected prelude symbols must not manufacture authored locations"
    );
}

#[test]
fn explicit_external_symbol_import_retains_authored_location() {
    let mut registry = ExternalPackageRegistry::new();
    let package_id = registry
        .register_package(
            "@test/explicit_symbols",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("test package registration should not collide");
    let function_id = ExternalFunctionId::Synthetic(4_901);
    registry
        .register_function_at_path(
            package_id,
            ExternalSymbolPath::from_single("run"),
            function_id,
            empty_void_function("run"),
        )
        .expect("test function registration should not collide");

    let mut string_table = StringTable::new();
    let source_file = intern_path(&["src", "@page.moth"], &mut string_table);
    let import_location = location_for(&["src", "@page.moth"], &mut string_table);
    let provider = RetainedProviderReference {
        path: intern_path(&["test", "explicit_symbols", "run"], &mut string_table),
        path_location: import_location.clone(),
        from_grouped: true,
        import_shell_id: ImportShellId::new(FileId(0), 1),
    };
    let import = FileImport {
        authored_provider: provider.clone(),
        provider,
        alias: None,
        location: import_location.clone(),
        alias_location: None,
        from_grouped: true,
        export_mode: HeaderExportMode::Private,
    };

    let mut module_symbols = ModuleSymbols::empty();
    module_symbols.module_file_paths.insert(source_file.clone());
    module_symbols
        .file_imports_by_source
        .insert(source_file.clone(), vec![import]);

    let environment = prepare_import_environment(ImportEnvironmentInput {
        module_symbols: &mut module_symbols,
        external_package_registry: &registry,
        external_import_resolution_table: &ExternalImportResolutionTable::new(),
        source_provider_imports: &Default::default(),
        string_table: &mut string_table,
    })
    .expect("explicit external symbol visibility should prepare");

    let run_name = string_table.intern("run");
    let visibility = environment
        .visibility_for(&source_file)
        .expect("source file visibility should exist");
    assert_eq!(
        visibility.visible_external_symbol_locations.get(&run_name),
        Some(&import_location)
    );
}

// ------------------------------------------------------------------
// Prelude namespace alias tests
// ------------------------------------------------------------------

fn register_prelude_namespace_test_package(registry: &mut ExternalPackageRegistry) {
    let package_id = registry
        .register_package(
            "@test/prelude_ns",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("test package registration should not collide");

    registry
        .register_function_at_path(
            package_id,
            ExternalSymbolPath::from_components(vec!["tools".to_owned(), "greet".to_owned()]),
            ExternalFunctionId::Synthetic(5000),
            empty_void_function("greet"),
        )
        .expect("test function registration should not collide");
}

#[test]
fn prelude_namespace_alias_injects_unshadowed_record() {
    let mut registry = ExternalPackageRegistry::new();
    register_prelude_namespace_test_package(&mut registry);
    registry
        .register_prelude_namespace_alias("prelude_ns", "@test/prelude_ns")
        .expect("prelude alias registration should not collide");

    let mut string_table = StringTable::new();
    let source_file = intern_path(&["src", "@page.moth"], &mut string_table);

    let mut module_symbols = ModuleSymbols::empty();
    module_symbols.module_file_paths.insert(source_file.clone());

    let environment = prepare_import_environment(ImportEnvironmentInput {
        module_symbols: &mut module_symbols,
        external_package_registry: &registry,
        external_import_resolution_table: &ExternalImportResolutionTable::new(),
        source_provider_imports: &Default::default(),
        string_table: &mut string_table,
    })
    .expect("import environment should build");

    let visibility = environment.visibility_for(&source_file).unwrap();
    let prelude_ns_name = string_table.intern("prelude_ns");
    let record = visibility
        .visible_namespace_records
        .get(&prelude_ns_name)
        .expect("prelude namespace alias should be visible");

    let tools_name = string_table.intern("tools");
    let greet_name = string_table.intern("greet");
    let child = record
        .child_namespaces
        .get(&tools_name)
        .expect("tools child namespace should exist");

    assert!(
        matches!(
            child.value_members.get(&greet_name),
            Some(NamespaceValueMember::ExternalSymbol(
                ExternalSymbolId::Function(ExternalFunctionId::Synthetic(5000))
            ))
        ),
        "prelude alias record should resolve nested namespace function"
    );
}

#[test]
fn prelude_namespace_alias_collides_with_same_file_declaration() {
    let mut registry = ExternalPackageRegistry::new();
    register_prelude_namespace_test_package(&mut registry);
    registry
        .register_prelude_namespace_alias("prelude_ns", "@test/prelude_ns")
        .expect("prelude alias registration should not collide");

    let mut string_table = StringTable::new();
    let source_file = intern_path(&["src", "@page.moth"], &mut string_table);
    let declaration_path = intern_path(&["src", "prelude_ns"], &mut string_table);

    let mut declared_paths = FxHashSet::default();
    declared_paths.insert(declaration_path);

    let mut module_symbols = ModuleSymbols::empty();
    module_symbols.module_file_paths.insert(source_file.clone());
    module_symbols
        .declared_paths_by_file
        .insert(source_file, declared_paths);

    let result = prepare_import_environment(ImportEnvironmentInput {
        module_symbols: &mut module_symbols,
        external_package_registry: &registry,
        external_import_resolution_table: &ExternalImportResolutionTable::new(),
        source_provider_imports: &Default::default(),
        string_table: &mut string_table,
    });

    let error =
        result.expect_err("same-file declaration should collide with prelude namespace alias");
    assert_eq!(
        error.diagnostics[0].kind,
        DiagnosticKind::Import(ImportDiagnosticKind::ImportNameCollision)
    );
}

#[test]
fn prelude_namespace_alias_coexists_with_explicit_import_of_same_target() {
    let mut registry = ExternalPackageRegistry::new();
    register_prelude_namespace_test_package(&mut registry);
    registry
        .register_prelude_namespace_alias("prelude_ns", "@test/prelude_ns")
        .expect("prelude alias registration should not collide");

    let mut string_table = StringTable::new();
    let source_file = intern_path(&["src", "@page.moth"], &mut string_table);
    let import_path = intern_path(&["test", "prelude_ns"], &mut string_table);

    let provider = RetainedProviderReference {
        path: import_path,
        path_location: location_for(&["src", "@page.moth"], &mut string_table),
        from_grouped: false,
        import_shell_id: ImportShellId::new(FileId(0), 2),
    };
    let import = FileImport {
        authored_provider: provider.clone(),
        provider,
        alias: None,
        location: location_for(&["src", "@page.moth"], &mut string_table),
        alias_location: None,
        from_grouped: false,
        export_mode: HeaderExportMode::Private,
    };

    let mut module_symbols = ModuleSymbols::empty();
    module_symbols.module_file_paths.insert(source_file.clone());
    module_symbols
        .file_imports_by_source
        .insert(source_file.clone(), vec![import]);

    let environment = prepare_import_environment(ImportEnvironmentInput {
        module_symbols: &mut module_symbols,
        external_package_registry: &registry,
        external_import_resolution_table: &ExternalImportResolutionTable::new(),
        source_provider_imports: &Default::default(),
        string_table: &mut string_table,
    })
    .expect("explicit import of same package should coexist with prelude alias");

    let visibility = environment.visibility_for(&source_file).unwrap();
    let prelude_ns_name = string_table.intern("prelude_ns");
    assert!(
        visibility
            .visible_namespace_records
            .contains_key(&prelude_ns_name),
        "prelude namespace record should be present"
    );
}

/// A nested module root that namespace-imports a deeper child module facade must resolve
/// to that child's prepared root file. The effective path becomes `<importer-prefix>/child`,
/// matching the child module root's import prefix.
#[test]
fn nested_module_root_imports_child_facade_resolves_child_root() {
    let mut string_table = StringTable::new();
    let helper_root = intern_path(&["helper-root"], &mut string_table);
    let helper_mod_file = intern_path(&["helper", "@mod.moth"], &mut string_table);
    let grandchild_root = intern_path(&["helper", "child-root"], &mut string_table);
    let grandchild_mod_file = intern_path(&["helper", "child", "@mod.moth"], &mut string_table);

    // The helper module root namespace-imports its grandchild module by bare name `child`.
    let import = test_import(
        intern_path(&["child"], &mut string_table),
        &mut string_table,
    );

    let mut module_symbols = ModuleSymbols::empty();
    module_symbols
        .module_file_paths
        .insert(helper_mod_file.clone());
    module_symbols
        .module_file_paths
        .insert(grandchild_mod_file.clone());
    module_symbols
        .file_module_membership
        .insert(helper_mod_file.clone(), helper_root.clone());
    module_symbols
        .file_module_membership
        .insert(grandchild_mod_file.clone(), grandchild_root.clone());
    module_symbols
        .module_root_boundaries
        .push(ModuleRootBoundary {
            import_prefix: intern_path(&["helper"], &mut string_table),
            module_root: helper_root.clone(),
            root_file: helper_mod_file.clone(),
        });
    module_symbols
        .module_root_boundaries
        .push(ModuleRootBoundary {
            import_prefix: intern_path(&["helper", "child"], &mut string_table),
            module_root: grandchild_root.clone(),
            root_file: grandchild_mod_file.clone(),
        });

    let registry = ExternalPackageRegistry::new();
    let external_import_resolution_table = ExternalImportResolutionTable::new();
    let source_provider_imports = Default::default();
    let mut builder = ImportEnvironmentBuilder {
        module_symbols: &module_symbols,
        external_package_registry: &registry,
        external_import_resolution_table: &external_import_resolution_table,
        source_provider_imports: &source_provider_imports,
        string_table: &mut string_table,
        environment: Default::default(),
        warnings: Vec::new(),
        provider_semantics_imported: Default::default(),
    };

    let Some(ResolvedNamespaceTarget::SourceFile(path)) =
        builder.resolve_module_root_public_export(&import.provider.path, &helper_mod_file)
    else {
        panic!("nested module root importing a child facade should resolve to the child root file");
    };
    assert_eq!(
        path, grandchild_mod_file,
        "nested child namespace import should resolve to the grandchild module's root file"
    );
}

#[test]
fn provider_semantics_import_once_across_many_shells() {
    let mut string_table = StringTable::new();
    let names = (0..10)
        .map(|index| format!("CONST_{index}"))
        .collect::<Vec<_>>();
    let name_refs = names.iter().map(String::as_str).collect::<Vec<_>>();
    let provider = constant_provider("provider", &name_refs);

    // Ten authored shells reference the same provider, one grouped constant import each.
    let mut module_symbols = ModuleSymbols::empty();
    let source_file = intern_path(&["src", "@page.moth"], &mut string_table);
    module_symbols.module_file_paths.insert(source_file.clone());
    let imports = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let mut import = test_import(
                intern_path(&["provider", name], &mut string_table),
                &mut string_table,
            );
            import.provider.import_shell_id = ImportShellId::new(FileId(0), index as u32);
            import.from_grouped = true;
            import
        })
        .collect();
    module_symbols
        .file_imports_by_source
        .insert(source_file.clone(), imports);

    let provider_imports = SourceProviderImportSet::new(
        names
            .iter()
            .enumerate()
            .map(|(index, _)| SourceProviderImport {
                kind: ProviderImportKind::Authored {
                    shell_id: ImportShellId::new(FileId(0), index as u32),
                },
                interface: &provider,
            })
            .collect(),
    )
    .expect("ten distinct shells should register");

    let environment = prepare_import_environment(ImportEnvironmentInput {
        module_symbols: &mut module_symbols,
        external_package_registry: &ExternalPackageRegistry::new(),
        external_import_resolution_table: &ExternalImportResolutionTable::new(),
        source_provider_imports: &provider_imports,
        string_table: &mut string_table,
    })
    .expect("provider imports should bind");

    assert_eq!(
        environment.imported_declarations_by_origin.len(),
        10,
        "one semantic record per origin"
    );
    assert_eq!(
        environment.imported_declarations_by_local_path.len(),
        10,
        "each alias stores its stable origin, never a cloned declaration"
    );
    assert_eq!(
        environment.imported_evidence_by_identity.len(),
        1,
        "ten shells from one provider must not duplicate evidence"
    );
    assert!(
        environment.imported_call_summaries_by_origin.is_empty(),
        "constant-only provider contributes no summaries"
    );
}

#[test]
fn differing_evidence_records_with_one_identity_fail_before_projection() {
    let mut string_table = StringTable::new();
    let identity = CanonicalEvidenceIdentity::new(
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
        CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Displayable),
    );
    let alpha_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("alpha"),
        "alpha/@mod.moth".to_string(),
        ModuleRootRole::Normal,
    );
    let alpha_value = OriginDeclarationId::Constant(OriginConstantId::new(
        alpha_origin.clone(),
        "VALUE".to_owned(),
    ));
    let first_provider = PublicSemanticInterface {
        module_origin: alpha_origin.clone(),
        export_bindings: vec![ExportBinding::new(
            alpha_origin.clone(),
            "VALUE".to_owned(),
            alpha_value.clone(),
        )],
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![PublicDeclarationRecord {
            origin: alpha_value,
            semantics: PublicDeclarationSemantics::Constant(PublicConstantSemantics {
                type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
                folded_value: PublicFoldedValue::String("alpha".to_owned()),
            }),
        }],
        reusable_evidence: vec![PublicEvidenceRecord {
            identity: identity.clone(),
            ownership: PublicEvidenceOwnership::SourceCanonical,
            requirement_mappings: Vec::new(),
        }],
        concrete_call_summaries: Vec::new(),
    };
    let beta_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("beta"),
        "beta/@mod.moth".to_string(),
        ModuleRootRole::Normal,
    );
    let beta_value = OriginDeclarationId::Constant(OriginConstantId::new(
        beta_origin.clone(),
        "VALUE".to_owned(),
    ));
    let mut second_provider = PublicSemanticInterface {
        module_origin: beta_origin.clone(),
        export_bindings: vec![ExportBinding::new(
            beta_origin.clone(),
            "VALUE".to_owned(),
            beta_value.clone(),
        )],
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![PublicDeclarationRecord {
            origin: beta_value,
            semantics: PublicDeclarationSemantics::Constant(PublicConstantSemantics {
                type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
                folded_value: PublicFoldedValue::String("beta".to_owned()),
            }),
        }],
        reusable_evidence: vec![PublicEvidenceRecord {
            identity: identity.clone(),
            ownership: PublicEvidenceOwnership::SourceCanonical,
            requirement_mappings: Vec::new(),
        }],
        concrete_call_summaries: Vec::new(),
    };
    // Different requirement mappings claim the same canonical evidence identity.
    second_provider.reusable_evidence[0]
        .requirement_mappings
        .push(crate::compiler_frontend::public_interface::PublicEvidenceRequirementMapping {
            requirement_identity:
                crate::compiler_frontend::canonical_type_identity::StableTraitRequirementIdentity::new(
                    CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Displayable),
                    "show".to_owned(),
                ),
            method_origin: crate::compiler_frontend::semantic_identity::OriginFunctionId::new_free(
                alpha_origin.clone(),
                "show".to_owned(),
            ),
        });

    let mut module_symbols = ModuleSymbols::empty();
    let source_file = intern_path(&["src", "@page.moth"], &mut string_table);
    module_symbols.module_file_paths.insert(source_file.clone());
    let mut alpha_import = test_import(
        intern_path(&["alpha", "VALUE"], &mut string_table),
        &mut string_table,
    );
    alpha_import.provider.import_shell_id = ImportShellId::new(FileId(0), 0);
    alpha_import.from_grouped = true;
    let mut beta_import = test_import(
        intern_path(&["beta", "VALUE"], &mut string_table),
        &mut string_table,
    );
    beta_import.provider.import_shell_id = ImportShellId::new(FileId(0), 1);
    beta_import.from_grouped = true;
    let imports = vec![alpha_import, beta_import];
    module_symbols
        .file_imports_by_source
        .insert(source_file.clone(), imports);

    let provider_imports = SourceProviderImportSet::new(vec![
        SourceProviderImport {
            kind: ProviderImportKind::Authored {
                shell_id: ImportShellId::new(FileId(0), 0),
            },
            interface: &first_provider,
        },
        SourceProviderImport {
            kind: ProviderImportKind::Authored {
                shell_id: ImportShellId::new(FileId(0), 1),
            },
            interface: &second_provider,
        },
    ])
    .expect("two distinct providers should register");

    let messages = prepare_import_environment(ImportEnvironmentInput {
        module_symbols: &mut module_symbols,
        external_package_registry: &ExternalPackageRegistry::new(),
        external_import_resolution_table: &ExternalImportResolutionTable::new(),
        source_provider_imports: &provider_imports,
        string_table: &mut string_table,
    })
    .expect_err("differing evidence records with one identity must fail before AST projection");

    let diagnostic = &messages.diagnostics[0];
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InfrastructureError { msg, .. }
                if msg.contains("evidence identity")
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostic.payload
    );
}

fn constant_provider(prefix: &str, names: &[&str]) -> PublicSemanticInterface {
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local(prefix),
        format!("{prefix}/@mod.moth"),
        ModuleRootRole::Normal,
    );
    let mut export_bindings = Vec::new();
    let mut declarations = Vec::new();
    for name in names {
        let origin = OriginDeclarationId::Constant(OriginConstantId::new(
            module_origin.clone(),
            (*name).to_owned(),
        ));
        export_bindings.push(ExportBinding::new(
            module_origin.clone(),
            (*name).to_owned(),
            origin.clone(),
        ));
        declarations.push(PublicDeclarationRecord {
            origin,
            semantics: PublicDeclarationSemantics::Constant(PublicConstantSemantics {
                type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
                folded_value: PublicFoldedValue::String((*name).to_owned()),
            }),
        });
    }
    PublicSemanticInterface {
        module_origin,
        export_bindings,
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations,
        reusable_evidence: vec![PublicEvidenceRecord {
            identity: CanonicalEvidenceIdentity::new(
                CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
                CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Displayable),
            ),
            ownership: PublicEvidenceOwnership::SourceCanonical,
            requirement_mappings: Vec::new(),
        }],
        concrete_call_summaries: Vec::new(),
    }
}

fn struct_provider_with_receiver_method() -> PublicSemanticInterface {
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("shapes"),
        "shapes/@mod.moth".to_string(),
        ModuleRootRole::Normal,
    );
    let box_origin = OriginTypeId::new(
        module_origin.clone(),
        "Box".to_owned(),
        OriginTypeCategory::Struct,
    );
    let method_origin = OriginFunctionId::new_receiver(
        module_origin.clone(),
        "size".to_owned(),
        box_origin.clone(),
    );
    PublicSemanticInterface {
        module_origin: module_origin.clone(),
        export_bindings: vec![ExportBinding::new(
            module_origin.clone(),
            "Box".to_owned(),
            OriginDeclarationId::Type(box_origin.clone()),
        )],
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![PublicDeclarationRecord {
            origin: OriginDeclarationId::Type(box_origin),
            semantics: PublicDeclarationSemantics::Struct(PublicStructSemantics {
                generic_parameters: Vec::new(),
                fields: Vec::new(),
                receiver_methods: vec![PublicReceiverMethodSemantics {
                    method_origin: method_origin.clone(),
                    category: PublicReceiverMethodCategory::ConcreteLocal,
                    parameters: Vec::new(),
                    returns: Vec::new(),
                    error_return: None,
                }],
            }),
        }],
        reusable_evidence: Vec::new(),
        concrete_call_summaries: vec![ConcreteCallSummaryRecord {
            origin: method_origin,
            summary: PublicCallSummary {
                parameters: Vec::new(),
                return_alias: FunctionReturnAliasSummary::Fresh,
            },
        }],
    }
}

fn single_file_module_symbols(
    imports: Vec<FileImport>,
    string_table: &mut StringTable,
) -> ModuleSymbols {
    let mut module_symbols = ModuleSymbols::empty();
    let source_file = intern_path(&["src", "@page.moth"], string_table);
    module_symbols.module_file_paths.insert(source_file.clone());
    module_symbols
        .file_imports_by_source
        .insert(source_file, imports);
    module_symbols
}

fn bind_environment(
    provider_imports: &SourceProviderImportSet<'_>,
    module_symbols: &mut ModuleSymbols,
    string_table: &mut StringTable,
) -> Result<
    crate::compiler_frontend::headers::import_environment::HeaderImportEnvironment,
    crate::compiler_frontend::compiler_errors::CompilerMessages,
> {
    prepare_import_environment(ImportEnvironmentInput {
        module_symbols,
        external_package_registry: &ExternalPackageRegistry::new(),
        external_import_resolution_table: &ExternalImportResolutionTable::new(),
        source_provider_imports: provider_imports,
        string_table,
    })
}

#[test]
fn namespace_and_grouped_imports_share_provider_semantics() {
    let mut string_table = StringTable::new();
    let provider = constant_provider("provider", &["CONST_0", "CONST_1"]);

    let mut grouped = test_import(
        intern_path(&["provider", "CONST_0"], &mut string_table),
        &mut string_table,
    );
    grouped.provider.import_shell_id = ImportShellId::new(FileId(0), 0);
    grouped.from_grouped = true;
    let mut namespace = test_import(
        intern_path(&["provider"], &mut string_table),
        &mut string_table,
    );
    namespace.provider.import_shell_id = ImportShellId::new(FileId(0), 1);

    let mut module_symbols =
        single_file_module_symbols(vec![grouped, namespace], &mut string_table);
    let provider_imports = SourceProviderImportSet::new(vec![
        SourceProviderImport {
            kind: ProviderImportKind::Authored {
                shell_id: ImportShellId::new(FileId(0), 0),
            },
            interface: &provider,
        },
        SourceProviderImport {
            kind: ProviderImportKind::Authored {
                shell_id: ImportShellId::new(FileId(0), 1),
            },
            interface: &provider,
        },
    ])
    .expect("two shells should collapse to one provider");

    let environment = bind_environment(&provider_imports, &mut module_symbols, &mut string_table)
        .expect("grouped and namespace imports should bind");

    assert_eq!(environment.imported_declarations_by_origin.len(), 2);
    assert_eq!(
        environment.imported_evidence_by_identity.len(),
        1,
        "grouped and namespace imports must share one evidence record"
    );
    assert!(environment.imported_call_summaries_by_origin.is_empty());
}

#[test]
fn two_aliases_of_one_declaration_retain_one_record() {
    let mut string_table = StringTable::new();
    let provider = constant_provider("provider", &["CONST_0"]);

    let mut first = test_import(
        intern_path(&["provider", "CONST_0"], &mut string_table),
        &mut string_table,
    );
    first.provider.import_shell_id = ImportShellId::new(FileId(0), 0);
    first.from_grouped = true;
    first.alias = Some(string_table.intern("first_alias"));
    let mut second = test_import(
        intern_path(&["provider", "CONST_0"], &mut string_table),
        &mut string_table,
    );
    second.provider.import_shell_id = ImportShellId::new(FileId(0), 1);
    second.from_grouped = true;
    second.alias = Some(string_table.intern("second_alias"));

    let mut module_symbols = single_file_module_symbols(vec![first, second], &mut string_table);
    let provider_imports = SourceProviderImportSet::new(vec![
        SourceProviderImport {
            kind: ProviderImportKind::Authored {
                shell_id: ImportShellId::new(FileId(0), 0),
            },
            interface: &provider,
        },
        SourceProviderImport {
            kind: ProviderImportKind::Authored {
                shell_id: ImportShellId::new(FileId(0), 1),
            },
            interface: &provider,
        },
    ])
    .expect("two shells should collapse to one provider");

    let environment = bind_environment(&provider_imports, &mut module_symbols, &mut string_table)
        .expect("two aliases of one declaration should bind");

    assert_eq!(
        environment.imported_declarations_by_origin.len(),
        1,
        "two aliases must retain one semantic record"
    );
    assert_eq!(environment.imported_declarations_by_local_path.len(), 1);
    let local_origin = environment
        .imported_declarations_by_local_path
        .values()
        .next()
        .expect("one local alias path");
    assert!(
        environment
            .imported_declarations_by_origin
            .contains_key(local_origin)
    );
}

#[test]
fn missing_provider_record_fails_deterministically() {
    let mut string_table = StringTable::new();
    let provider = constant_provider("provider", &["CONST_0"]);

    let mut missing = test_import(
        intern_path(&["provider", "MISSING"], &mut string_table),
        &mut string_table,
    );
    missing.provider.import_shell_id = ImportShellId::new(FileId(0), 0);
    missing.from_grouped = true;

    let mut module_symbols = single_file_module_symbols(vec![missing], &mut string_table);
    let provider_imports = SourceProviderImportSet::new(vec![SourceProviderImport {
        kind: ProviderImportKind::Authored {
            shell_id: ImportShellId::new(FileId(0), 0),
        },
        interface: &provider,
    }])
    .expect("one shell should register");

    let messages = bind_environment(&provider_imports, &mut module_symbols, &mut string_table)
        .expect_err("a missing provider record must fail binding deterministically");
    assert!(
        matches!(
            &messages.diagnostics[0].payload,
            DiagnosticPayload::NotExportedByPublicSurface { .. }
        ),
        "unexpected diagnostic payload: {:?}",
        messages.diagnostics[0].payload
    );
}

#[test]
fn receiver_methods_reuse_summary_by_origin_storage() {
    let mut string_table = StringTable::new();
    let provider = struct_provider_with_receiver_method();

    let imports = (0..2)
        .map(|index| {
            let mut import = test_import(
                intern_path(&["shapes", "Box"], &mut string_table),
                &mut string_table,
            );
            import.provider.import_shell_id = ImportShellId::new(FileId(0), index);
            import.from_grouped = true;
            import
        })
        .collect();
    let mut module_symbols = single_file_module_symbols(imports, &mut string_table);
    let provider_imports = SourceProviderImportSet::new(
        (0..2)
            .map(|index| SourceProviderImport {
                kind: ProviderImportKind::Authored {
                    shell_id: ImportShellId::new(FileId(0), index),
                },
                interface: &provider,
            })
            .collect(),
    )
    .expect("two shells should collapse to one provider");

    let environment = bind_environment(&provider_imports, &mut module_symbols, &mut string_table)
        .expect("receiver method imports should bind");

    assert_eq!(
        environment.imported_call_summaries_by_origin.len(),
        1,
        "receiver methods must reuse one summary by origin"
    );
    assert_eq!(
        environment.imported_functions_by_local_path.len(),
        1,
        "the repeated receiver contract must not duplicate the local function entry"
    );
}

#[test]
fn differing_provider_declarations_with_one_origin_fail_as_compiler_error() {
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("shared"),
        "shared/@mod.moth".to_owned(),
        ModuleRootRole::Normal,
    );
    let origin =
        OriginDeclarationId::Constant(OriginConstantId::new(module_origin, "VALUE".to_owned()));
    let first = PublicDeclarationRecord {
        origin: origin.clone(),
        semantics: PublicDeclarationSemantics::Constant(PublicConstantSemantics {
            type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
            folded_value: PublicFoldedValue::String("first".to_owned()),
        }),
    };
    let mut second = first.clone();
    let PublicDeclarationSemantics::Constant(second_constant) = &mut second.semantics else {
        unreachable!("declaration semantics is constant");
    };
    second_constant.folded_value = PublicFoldedValue::String("second".to_owned());

    let mut table = FxHashMap::default();
    super::super::builder::insert_agreed(&mut table, origin.clone(), &first, "declaration origin")
        .expect("first publisher should insert");
    let error =
        super::super::builder::insert_agreed(&mut table, origin, &second, "declaration origin")
            .expect_err("second publisher must disagree");
    assert!(error.msg.contains("declaration origin"));
}

#[test]
fn differing_provider_summaries_with_one_origin_fail_as_compiler_error() {
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("shared"),
        "shared/@mod.moth".to_owned(),
        ModuleRootRole::Normal,
    );
    let origin = OriginFunctionId::new_free(module_origin, "make".to_owned());
    let first = PublicCallSummary {
        parameters: Vec::new(),
        return_alias: FunctionReturnAliasSummary::Fresh,
    };
    let second = PublicCallSummary {
        parameters: Vec::new(),
        return_alias: FunctionReturnAliasSummary::Unknown,
    };

    let mut table = FxHashMap::default();
    super::super::builder::insert_agreed(&mut table, origin.clone(), &first, "summary origin")
        .expect("first publisher should insert");
    let error = super::super::builder::insert_agreed(&mut table, origin, &second, "summary origin")
        .expect_err("second publisher must disagree");
    assert!(error.msg.contains("summary origin"));
}

#[test]
fn differing_evidence_records_with_one_identity_fail_as_compiler_error() {
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("shared"),
        "shared/@mod.moth".to_owned(),
        ModuleRootRole::Normal,
    );
    let identity = CanonicalEvidenceIdentity::new(
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
        CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Displayable),
    );
    let first = PublicEvidenceRecord {
        identity: identity.clone(),
        ownership: PublicEvidenceOwnership::SourceCanonical,
        requirement_mappings: Vec::new(),
    };
    let mut second = first.clone();
    second.requirement_mappings.push(
        crate::compiler_frontend::public_interface::PublicEvidenceRequirementMapping {
            requirement_identity:
                crate::compiler_frontend::canonical_type_identity::StableTraitRequirementIdentity::new(
                    CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Displayable),
                    "show".to_owned(),
                ),
            method_origin: OriginFunctionId::new_free(module_origin, "show".to_owned()),
        },
    );

    let mut table = FxHashMap::default();
    super::super::builder::insert_agreed(&mut table, identity.clone(), &first, "evidence identity")
        .expect("first publisher should insert");
    let error =
        super::super::builder::insert_agreed(&mut table, identity, &second, "evidence identity")
            .expect_err("second publisher must disagree");
    assert!(error.msg.contains("evidence identity"));
}

#[test]
fn occupied_agreement_insertion_borrows_without_cloning() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug)]
    struct CloneCounting(String, Arc<AtomicUsize>);

    impl PartialEq for CloneCounting {
        fn eq(&self, other: &Self) -> bool {
            self.0 == other.0
        }
    }

    impl Eq for CloneCounting {}

    impl Clone for CloneCounting {
        fn clone(&self) -> Self {
            self.1.fetch_add(1, Ordering::Relaxed);
            Self(self.0.clone(), Arc::clone(&self.1))
        }
    }

    let clones = Arc::new(AtomicUsize::new(0));
    let first = CloneCounting("shared".to_owned(), Arc::clone(&clones));
    let equal = CloneCounting("shared".to_owned(), Arc::clone(&clones));
    let differing = CloneCounting("other".to_owned(), Arc::clone(&clones));

    let mut table = FxHashMap::default();
    super::super::builder::insert_agreed(&mut table, "key".to_owned(), &first, "clone counting")
        .expect("vacant insertion should clone once");
    assert_eq!(clones.load(Ordering::Relaxed), 1);

    super::super::builder::insert_agreed(&mut table, "key".to_owned(), &equal, "clone counting")
        .expect("occupied equal agreement should borrow");
    assert_eq!(
        clones.load(Ordering::Relaxed),
        1,
        "occupied agreement must not clone the candidate"
    );

    let error = super::super::builder::insert_agreed(
        &mut table,
        "key".to_owned(),
        &differing,
        "clone counting",
    )
    .expect_err("occupied disagreement must fail");
    assert!(error.msg.contains("clone counting"));
    assert_eq!(
        clones.load(Ordering::Relaxed),
        1,
        "disagreement must not clone the candidate either"
    );
}
