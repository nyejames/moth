//! Tests for namespace-record construction in the header import environment.
//!
//! WHAT: covers recursive external package records and source receiver-method filtering.
//! WHY: AST must consume namespace visibility without rebuilding import surfaces, so this
//! header-stage data shape needs direct coverage.

use super::*;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::compiler_messages::{DiagnosticKind, ImportDiagnosticKind};
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalConstantDef, ExternalConstantId, ExternalConstantValue,
    ExternalFunctionDef, ExternalFunctionId, ExternalFunctionLowerings, ExternalPackageRegistry,
    ExternalReturnAlias, ExternalSymbolId, ExternalSymbolPath, ExternalTypeDef, ExternalTypeId,
    external_success_returns,
};
use crate::compiler_frontend::headers::import_environment::{
    ImportEnvironmentInput, prepare_import_environment,
};
use crate::compiler_frontend::headers::module_symbols::{ModuleRootBoundary, ModuleSymbols};
use crate::compiler_frontend::headers::types::{FileImport, HeaderExportMode};
use crate::compiler_frontend::paths::const_paths::StructuralProviderReference;
use crate::compiler_frontend::symbols::identity::ImportShellId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use rustc_hash::FxHashSet;

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
    let provider = StructuralProviderReference {
        path: header_path,
        path_location: location_for(&["src", "@page.moth"], string_table),
        import_shell_id: None,
        from_grouped: false,
    };
    FileImport {
        import_shell_id: ImportShellId::new(None, 0),
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

    assert_duplicate_import_surface_member(*error);
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

    assert_duplicate_import_surface_member(*error);
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

    assert_duplicate_import_surface_member(*error);
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
    let provider = StructuralProviderReference {
        path: intern_path(&["test", "explicit_symbols", "run"], &mut string_table),
        path_location: import_location.clone(),
        import_shell_id: None,
        from_grouped: true,
    };
    let import = FileImport {
        import_shell_id: ImportShellId::new(None, 1),
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

    let provider = StructuralProviderReference {
        path: import_path,
        path_location: location_for(&["src", "@page.moth"], &mut string_table),
        import_shell_id: None,
        from_grouped: false,
    };
    let import = FileImport {
        import_shell_id: ImportShellId::new(None, 2),
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
