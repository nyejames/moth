//! Tests for namespace-record construction in the header header binding environment.
//!
//! WHAT: covers recursive external package records and source receiver-method filtering.
//! WHY: AST must consume namespace visibility without rebuilding dependency surfaces, so this
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
use crate::compiler_frontend::folded_value::{OwnedFoldedString, PublicFoldedValue};
use crate::compiler_frontend::headers::binding_environment::{
    BindingEnvironmentInput, prepare_binding_environment,
};
use crate::compiler_frontend::headers::dependency_clause_syntax::{
    DependencyAlias, RetainedDependencyPath,
};
use crate::compiler_frontend::headers::module_symbols::{
    ModuleRootBoundary, ModuleSymbols, PublicExportEntry, PublicExportTarget,
};
use crate::compiler_frontend::headers::types::{
    DependencyBindingSyntax, DependencySelection, DependencySelectionRange, HeaderExportMode,
    RetainedDependencyClause,
};
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallSummary,
};
use crate::compiler_frontend::public_interface::{
    ConcreteCallSummaryRecord, ProviderDependencyKind, PublicBindingExport,
    PublicConstantSemantics, PublicDeclarationRecord, PublicDeclarationSemantics,
    PublicEvidenceOwnership, PublicEvidenceRecord, PublicReceiverMethodCategory,
    PublicReceiverMethodSemantics, PublicSemanticInterface, PublicStructSemantics,
    SourceProviderDependency, SourceProviderDependencySet,
};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, ModuleRootRole, OriginConstantId, OriginDeclarationId, OriginFunctionId,
    OriginTypeCategory, OriginTypeId, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::identity::{
    DependencySelectionId, DependencyShellId, FileId,
};
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

fn test_dependency(
    header_path: InternedPath,
    string_table: &mut StringTable,
) -> RetainedDependencyClause {
    let provider = RetainedDependencyPath {
        path: header_path,
        path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
        target: crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source,
        location: location_for(&["src", "@page.moth"], string_table),
        dependency_shell_id: DependencyShellId::new(FileId(0), 0),
    };
    RetainedDependencyClause {
        dependency: provider.clone(),
        binding: DependencyBindingSyntax::Namespace { alias: None },
        location: location_for(&["src", "@page.moth"], string_table),
        export_mode: HeaderExportMode::Private,
    }
}

fn add_selection(
    dependency: &mut RetainedDependencyClause,
    selection_store: &mut Vec<DependencySelection>,
    source_name: &str,
    local_alias: Option<&str>,
    string_table: &mut StringTable,
) {
    let selection_start = dependency
        .binding
        .selection_range()
        .map_or(selection_store.len(), |range| range.start as usize);
    let local_alias = local_alias.map(|alias| DependencyAlias {
        name: string_table.intern(alias),
        location: dependency.location.clone(),
    });
    selection_store.push(DependencySelection {
        source_name: string_table.intern(source_name),
        source_location: dependency.location.clone(),
        local_alias,
    });
    dependency.binding = DependencyBindingSyntax::DirectSelections {
        range: DependencySelectionRange::new(selection_start, selection_store.len()),
    };
}

fn assert_duplicate_dependency_surface_member(error: CompilerDiagnostic) {
    assert_eq!(
        error.kind,
        DiagnosticKind::Import(ImportDiagnosticKind::DuplicateImportSurfaceMember)
    );
}

#[test]
#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
fn binding_counters_separate_namespace_clauses_from_selected_names() {
    use crate::compiler_frontend::instrumentation::{
        capture_frontend_counters_for_test, log_frontend_counters, reset_frontend_counters,
    };
    use crate::timing::start_benchmark_collection;

    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _counter_capture = capture_frontend_counters_for_test();
    reset_frontend_counters();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    let mut registry = ExternalPackageRegistry::new();
    let namespace_package = registry
        .register_package(
            "@test/namespace",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("namespace package should register");
    registry
        .register_function_at_path(
            namespace_package,
            ExternalSymbolPath::from_components(vec!["first".to_owned()]),
            ExternalFunctionId::Synthetic(7_001),
            empty_void_function("first"),
        )
        .expect("namespace member should register");

    let selection_package = registry
        .register_package(
            "@test/selections",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("selection package should register");
    for (name, id) in [("first", 7_002), ("second", 7_003), ("third", 7_004)] {
        registry
            .register_function_at_path(
                selection_package,
                ExternalSymbolPath::from_components(vec![name.to_owned()]),
                ExternalFunctionId::Synthetic(id),
                empty_void_function(name),
            )
            .expect("selected package member should register");
    }

    let mut string_table = StringTable::new();
    let source_file = intern_path(&["src", "@page.moth"], &mut string_table);
    let namespace_path = intern_path(&["test", "namespace"], &mut string_table);
    let selection_path = intern_path(&["test", "selections"], &mut string_table);
    let namespace_clause = test_dependency(namespace_path, &mut string_table);
    let mut selection_clause = test_dependency(selection_path, &mut string_table);
    let mut selection_store = Vec::new();
    for name in ["first", "second", "third"] {
        add_selection(
            &mut selection_clause,
            &mut selection_store,
            name,
            None,
            &mut string_table,
        );
    }

    let mut module_symbols = ModuleSymbols::empty();
    module_symbols.module_file_paths.insert(source_file.clone());
    module_symbols.file_dependency_clauses_by_source.insert(
        source_file.clone(),
        vec![namespace_clause, selection_clause],
    );
    module_symbols
        .dependency_selections_by_source
        .insert(source_file, selection_store);

    prepare_binding_environment(BindingEnvironmentInput {
        module_symbols: &module_symbols,
        external_package_registry: &registry,
        external_dependency_resolution_table: &ExternalImportResolutionTable::new(),
        source_provider_dependencies: &Default::default(),
        string_table: &mut string_table,
    })
    .expect("namespace and selected-name clauses should bind");

    log_frontend_counters();
    let observations = timing_session.finish();
    let counter_value = |name: &str| {
        observations
            .counters
            .iter()
            .find(|counter| counter.name == name)
            .map(|counter| counter.value)
            .unwrap_or(-1.0)
    };

    assert_eq!(counter_value("bound_namespace_clause_count"), 1.0);
    assert_eq!(counter_value("bound_selected_name_count"), 3.0);
}

fn dependency_error_diagnostic(
    error: crate::compiler_frontend::headers::binding_environment::BindingEnvironmentError,
) -> CompilerDiagnostic {
    match error {
        crate::compiler_frontend::headers::binding_environment::BindingEnvironmentError::Diagnostic(
            diagnostic,
        ) => *diagnostic,
        crate::compiler_frontend::headers::binding_environment::BindingEnvironmentError::Internal(
            error,
        ) => panic!("expected dependency diagnostic, got internal error: {error:?}"),
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
    let dependency_path = intern_path(&["test", "path"], &mut string_table);
    let dependency = test_dependency(dependency_path, &mut string_table);

    let mut module_symbols = ModuleSymbols::empty();
    module_symbols.module_file_paths.insert(source_file.clone());
    module_symbols
        .file_dependency_clauses_by_source
        .insert(source_file.clone(), vec![dependency]);

    let external_dependency_resolution_table = ExternalImportResolutionTable::new();
    let environment = prepare_binding_environment(BindingEnvironmentInput {
        module_symbols: &mut module_symbols,
        external_package_registry: &registry,
        external_dependency_resolution_table: &external_dependency_resolution_table,
        source_provider_dependencies: &Default::default(),
        string_table: &mut string_table,
    })
    .expect("external namespace dependency should prepare");

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
        .expect("bare package dependency should create a namespace record");

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

    assert_duplicate_dependency_surface_member(dependency_error_diagnostic(error));
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

    assert_duplicate_dependency_surface_member(dependency_error_diagnostic(error));
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

    assert_duplicate_dependency_surface_member(dependency_error_diagnostic(error));
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
        .dependency_bindable_source_symbol_paths
        .insert(method_path.clone());
    module_symbols.receiver_method_paths.insert(method_path);

    let registry = ExternalPackageRegistry::new();
    let external_dependency_resolution_table = ExternalImportResolutionTable::new();
    let source_provider_dependencies = Default::default();
    let builder = BindingEnvironmentBuilder {
        module_symbols: &module_symbols,
        external_package_registry: &registry,
        external_dependency_resolution_table: &external_dependency_resolution_table,
        source_provider_dependencies: &source_provider_dependencies,
        string_table: &mut string_table,
        environment: Default::default(),
        warnings: Vec::new(),
        provider_semantics_registered: Default::default(),
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
    let dependency = test_dependency(
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
            dependency_prefix: intern_path(&["helper"], &mut string_table),
            module_root,
            root_file: root_file.clone(),
        });

    let registry = ExternalPackageRegistry::new();
    let external_dependency_resolution_table = ExternalImportResolutionTable::new();
    let source_provider_dependencies = Default::default();
    let mut builder = BindingEnvironmentBuilder {
        module_symbols: &module_symbols,
        external_package_registry: &registry,
        external_dependency_resolution_table: &external_dependency_resolution_table,
        source_provider_dependencies: &source_provider_dependencies,
        string_table: &mut string_table,
        environment: Default::default(),
        warnings: Vec::new(),
        provider_semantics_registered: Default::default(),
    };

    let Some(ResolvedNamespaceTarget::SourceFile(path)) =
        builder.resolve_module_root_public_export(&dependency.dependency.path, &source_file)
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

    let environment = prepare_binding_environment(BindingEnvironmentInput {
        module_symbols: &mut module_symbols,
        external_package_registry: &registry,
        external_dependency_resolution_table: &ExternalImportResolutionTable::new(),
        source_provider_dependencies: &Default::default(),
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
fn explicit_external_symbol_binding_retains_authored_location() {
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
    let dependency_location = location_for(&["src", "@page.moth"], &mut string_table);
    let provider = RetainedDependencyPath {
        path: intern_path(&["test", "explicit_symbols"], &mut string_table),
        path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
        target: crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source,
        location: dependency_location.clone(),
        dependency_shell_id: DependencyShellId::new(FileId(0), 1),
    };
    let dependency_selections = vec![
        crate::compiler_frontend::headers::types::DependencySelection {
            source_name: string_table.intern("run"),
            source_location: dependency_location.clone(),
            local_alias: None,
        },
    ];
    let dependency = RetainedDependencyClause {
        dependency: provider.clone(),
        binding: DependencyBindingSyntax::DirectSelections {
            range: DependencySelectionRange::new(0, 1),
        },
        location: dependency_location.clone(),
        export_mode: HeaderExportMode::Private,
    };

    let mut module_symbols = ModuleSymbols::empty();
    module_symbols.module_file_paths.insert(source_file.clone());
    module_symbols
        .file_dependency_clauses_by_source
        .insert(source_file.clone(), vec![dependency]);
    module_symbols
        .dependency_selections_by_source
        .insert(source_file.clone(), dependency_selections);

    let environment = prepare_binding_environment(BindingEnvironmentInput {
        module_symbols: &mut module_symbols,
        external_package_registry: &registry,
        external_dependency_resolution_table: &ExternalImportResolutionTable::new(),
        source_provider_dependencies: &Default::default(),
        string_table: &mut string_table,
    })
    .expect("explicit external symbol visibility should prepare");

    let run_name = string_table.intern("run");
    let visibility = environment
        .visibility_for(&source_file)
        .expect("source file visibility should exist");
    assert_eq!(
        visibility.visible_external_symbol_locations.get(&run_name),
        Some(&dependency_location)
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

    let environment = prepare_binding_environment(BindingEnvironmentInput {
        module_symbols: &mut module_symbols,
        external_package_registry: &registry,
        external_dependency_resolution_table: &ExternalImportResolutionTable::new(),
        source_provider_dependencies: &Default::default(),
        string_table: &mut string_table,
    })
    .expect("header binding environment should build");

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

    let result = prepare_binding_environment(BindingEnvironmentInput {
        module_symbols: &mut module_symbols,
        external_package_registry: &registry,
        external_dependency_resolution_table: &ExternalImportResolutionTable::new(),
        source_provider_dependencies: &Default::default(),
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
fn prelude_namespace_alias_coexists_with_explicit_dependency_of_same_target() {
    let mut registry = ExternalPackageRegistry::new();
    register_prelude_namespace_test_package(&mut registry);
    registry
        .register_prelude_namespace_alias("prelude_ns", "@test/prelude_ns")
        .expect("prelude alias registration should not collide");

    let mut string_table = StringTable::new();
    let source_file = intern_path(&["src", "@page.moth"], &mut string_table);
    let dependency_path = intern_path(&["test", "prelude_ns"], &mut string_table);

    let provider = RetainedDependencyPath {
        path: dependency_path,
        path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
        target: crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source,
        location: location_for(&["src", "@page.moth"], &mut string_table),
        dependency_shell_id: DependencyShellId::new(FileId(0), 2),
    };
    let dependency = RetainedDependencyClause {
        dependency: provider.clone(),
        binding: DependencyBindingSyntax::Namespace { alias: None },
        location: location_for(&["src", "@page.moth"], &mut string_table),
        export_mode: HeaderExportMode::Private,
    };

    let mut module_symbols = ModuleSymbols::empty();
    module_symbols.module_file_paths.insert(source_file.clone());
    module_symbols
        .file_dependency_clauses_by_source
        .insert(source_file.clone(), vec![dependency]);

    let environment = prepare_binding_environment(BindingEnvironmentInput {
        module_symbols: &mut module_symbols,
        external_package_registry: &registry,
        external_dependency_resolution_table: &ExternalImportResolutionTable::new(),
        source_provider_dependencies: &Default::default(),
        string_table: &mut string_table,
    })
    .expect("explicit dependency of same package should coexist with prelude alias");

    let visibility = environment.visibility_for(&source_file).unwrap();
    let prelude_ns_name = string_table.intern("prelude_ns");
    assert!(
        visibility
            .visible_namespace_records
            .contains_key(&prelude_ns_name),
        "prelude namespace record should be present"
    );
}

/// A nested module root that namespace-dependencies a deeper child module facade must resolve
/// to that child's prepared root file. The effective path becomes `<consumer-prefix>/child`,
/// matching the child module root's dependency prefix.
#[test]
fn nested_module_root_depends_on_child_facade_resolves_child_root() {
    let mut string_table = StringTable::new();
    let helper_root = intern_path(&["helper-root"], &mut string_table);
    let helper_mod_file = intern_path(&["helper", "@mod.moth"], &mut string_table);
    let grandchild_root = intern_path(&["helper", "child-root"], &mut string_table);
    let grandchild_mod_file = intern_path(&["helper", "child", "@mod.moth"], &mut string_table);

    // The helper module root namespace-dependencies its grandchild module by bare name `child`.
    let dependency = test_dependency(
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
            dependency_prefix: intern_path(&["helper"], &mut string_table),
            module_root: helper_root.clone(),
            root_file: helper_mod_file.clone(),
        });
    module_symbols
        .module_root_boundaries
        .push(ModuleRootBoundary {
            dependency_prefix: intern_path(&["helper", "child"], &mut string_table),
            module_root: grandchild_root.clone(),
            root_file: grandchild_mod_file.clone(),
        });

    let registry = ExternalPackageRegistry::new();
    let external_dependency_resolution_table = ExternalImportResolutionTable::new();
    let source_provider_dependencies = Default::default();
    let mut builder = BindingEnvironmentBuilder {
        module_symbols: &module_symbols,
        external_package_registry: &registry,
        external_dependency_resolution_table: &external_dependency_resolution_table,
        source_provider_dependencies: &source_provider_dependencies,
        string_table: &mut string_table,
        environment: Default::default(),
        warnings: Vec::new(),
        provider_semantics_registered: Default::default(),
    };

    let Some(ResolvedNamespaceTarget::SourceFile(path)) =
        builder.resolve_module_root_public_export(&dependency.dependency.path, &helper_mod_file)
    else {
        panic!(
            "nested module root depending on a child facade should resolve to the child root file"
        );
    };
    assert_eq!(
        path, grandchild_mod_file,
        "nested child namespace dependency should resolve to the grandchild module's root file"
    );
}

#[test]
fn provider_semantics_bind_once_across_many_shells() {
    let mut string_table = StringTable::new();
    let names = (0..10)
        .map(|index| format!("CONST_{index}"))
        .collect::<Vec<_>>();
    let name_refs = names.iter().map(String::as_str).collect::<Vec<_>>();
    let provider = constant_provider("provider", &name_refs);

    // Ten authored shells reference the same provider, one direct constant selection each.
    let mut module_symbols = ModuleSymbols::empty();
    let source_file = intern_path(&["src", "@page.moth"], &mut string_table);
    module_symbols.module_file_paths.insert(source_file.clone());
    let mut dependency_selections = Vec::new();
    let dependencies = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let mut dependency = test_dependency(
                intern_path(&["provider"], &mut string_table),
                &mut string_table,
            );
            dependency.dependency.dependency_shell_id =
                DependencyShellId::new(FileId(0), index as u32);
            add_selection(
                &mut dependency,
                &mut dependency_selections,
                name,
                None,
                &mut string_table,
            );
            dependency
        })
        .collect();
    module_symbols
        .file_dependency_clauses_by_source
        .insert(source_file.clone(), dependencies);
    module_symbols
        .dependency_selections_by_source
        .insert(source_file.clone(), dependency_selections);

    let provider_dependencies = SourceProviderDependencySet::new(
        names
            .iter()
            .enumerate()
            .map(|(index, _)| SourceProviderDependency {
                kind: ProviderDependencyKind::Authored {
                    shell: DependencyShellId::new(FileId(0), index as u32),
                },
                interface: &provider,
            })
            .collect(),
    )
    .expect("ten distinct shells should register");

    let environment = prepare_binding_environment(BindingEnvironmentInput {
        module_symbols: &mut module_symbols,
        external_package_registry: &ExternalPackageRegistry::new(),
        external_dependency_resolution_table: &ExternalImportResolutionTable::new(),
        source_provider_dependencies: &provider_dependencies,
        string_table: &mut string_table,
    })
    .expect("provider dependencies should bind");

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
fn reversed_selection_order_keeps_one_provider_surface_identity() {
    let mut string_table = StringTable::new();
    let provider = constant_provider("provider", &["FIRST", "SECOND"]);
    let mut dependency_selections = Vec::new();

    let mut forward = test_dependency(
        intern_path(&["provider"], &mut string_table),
        &mut string_table,
    );
    forward.dependency.dependency_shell_id = DependencyShellId::new(FileId(0), 0);
    add_selection(
        &mut forward,
        &mut dependency_selections,
        "FIRST",
        None,
        &mut string_table,
    );
    add_selection(
        &mut forward,
        &mut dependency_selections,
        "SECOND",
        None,
        &mut string_table,
    );

    let mut reverse = test_dependency(
        intern_path(&["provider"], &mut string_table),
        &mut string_table,
    );
    reverse.dependency.dependency_shell_id = DependencyShellId::new(FileId(0), 1);
    add_selection(
        &mut reverse,
        &mut dependency_selections,
        "SECOND",
        None,
        &mut string_table,
    );
    add_selection(
        &mut reverse,
        &mut dependency_selections,
        "FIRST",
        None,
        &mut string_table,
    );

    let mut module_symbols = single_file_module_symbols(
        vec![forward, reverse],
        dependency_selections,
        &mut string_table,
    );
    let provider_dependencies = SourceProviderDependencySet::new(vec![
        SourceProviderDependency {
            kind: ProviderDependencyKind::Authored {
                shell: DependencyShellId::new(FileId(0), 0),
            },
            interface: &provider,
        },
        SourceProviderDependency {
            kind: ProviderDependencyKind::Authored {
                shell: DependencyShellId::new(FileId(0), 1),
            },
            interface: &provider,
        },
    ])
    .expect("repeated provider surfaces should register");

    let forward_provider = provider_dependencies
        .resolve_clause(DependencyShellId::new(FileId(0), 0))
        .expect("forward clause should resolve");
    let reverse_provider = provider_dependencies
        .resolve_clause(DependencyShellId::new(FileId(0), 1))
        .expect("reverse clause should resolve");
    assert_eq!(forward_provider.provider, reverse_provider.provider);
    assert_ne!(forward_provider.shell, reverse_provider.shell);

    let environment = bind_environment(
        &provider_dependencies,
        &mut module_symbols,
        &mut string_table,
    )
    .expect("both selection orders should bind through one provider surface");
    assert_eq!(environment.imported_declarations_by_origin.len(), 2);
    assert_eq!(environment.imported_evidence_by_identity.len(), 1);
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
                folded_value: PublicFoldedValue::String(OwnedFoldedString::Text(
                    "alpha".to_owned(),
                )),
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
                folded_value: PublicFoldedValue::String(OwnedFoldedString::Text("beta".to_owned())),
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
    let mut dependency_selections = Vec::new();
    let mut alpha_dependency = test_dependency(
        intern_path(&["alpha"], &mut string_table),
        &mut string_table,
    );
    alpha_dependency.dependency.dependency_shell_id = DependencyShellId::new(FileId(0), 0);
    add_selection(
        &mut alpha_dependency,
        &mut dependency_selections,
        "VALUE",
        None,
        &mut string_table,
    );
    let mut beta_dependency =
        test_dependency(intern_path(&["beta"], &mut string_table), &mut string_table);
    beta_dependency.dependency.dependency_shell_id = DependencyShellId::new(FileId(0), 1);
    add_selection(
        &mut beta_dependency,
        &mut dependency_selections,
        "VALUE",
        None,
        &mut string_table,
    );
    let dependencies = vec![alpha_dependency, beta_dependency];
    module_symbols
        .file_dependency_clauses_by_source
        .insert(source_file.clone(), dependencies);
    module_symbols
        .dependency_selections_by_source
        .insert(source_file.clone(), dependency_selections);

    let provider_dependencies = SourceProviderDependencySet::new(vec![
        SourceProviderDependency {
            kind: ProviderDependencyKind::Authored {
                shell: DependencyShellId::new(FileId(0), 0),
            },
            interface: &first_provider,
        },
        SourceProviderDependency {
            kind: ProviderDependencyKind::Authored {
                shell: DependencyShellId::new(FileId(0), 1),
            },
            interface: &second_provider,
        },
    ])
    .expect("two distinct providers should register");

    let messages = prepare_binding_environment(BindingEnvironmentInput {
        module_symbols: &mut module_symbols,
        external_package_registry: &ExternalPackageRegistry::new(),
        external_dependency_resolution_table: &ExternalImportResolutionTable::new(),
        source_provider_dependencies: &provider_dependencies,
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
                folded_value: PublicFoldedValue::String(OwnedFoldedString::Text(
                    (*name).to_owned(),
                )),
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

fn provider_selection_entry(
    export_name: StringId,
    source_name: StringId,
    shell: DependencyShellId,
    diagnostic_path: InternedPath,
) -> PublicExportEntry {
    PublicExportEntry {
        export_name,
        target: PublicExportTarget::ProviderSelection {
            selection: DependencySelectionId::new(shell, 0),
            source_name,
            diagnostic_path,
        },
    }
}

fn binding_provider(registry: &mut ExternalPackageRegistry) -> PublicSemanticInterface {
    binding_provider_with_members(registry, &[("BOUND", 5_100)])
}

fn binding_provider_with_members(
    registry: &mut ExternalPackageRegistry,
    members: &[(&str, u32)],
) -> PublicSemanticInterface {
    let package_id = registry
        .register_package(
            "@test/provider_binding",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("binding provider package should register");
    let mut binding_exports = Vec::new();
    for &(name, id) in members {
        let symbol_id = ExternalConstantId(id);
        registry
            .register_constant_at_path(
                package_id,
                ExternalSymbolPath::from_single(name),
                symbol_id,
                ExternalConstantDef {
                    name: name.to_owned(),
                    data_type: ExternalAbiType::I32,
                    value: ExternalConstantValue::Int(1),
                },
            )
            .expect("binding provider constant should register");
        binding_exports.push((name, symbol_id));
    }

    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("binding-provider"),
        "binding-provider/@mod.moth".to_owned(),
        ModuleRootRole::Normal,
    );
    PublicSemanticInterface {
        module_origin: module_origin.clone(),
        export_bindings: Vec::new(),
        export_diagnostic_provenance: Vec::new(),
        binding_exports: binding_exports
            .into_iter()
            .map(|(name, symbol_id)| PublicBindingExport {
                exporting_module: module_origin.clone(),
                public_name: name.to_owned(),
                target: registry
                    .canonical_symbol_identity(ExternalSymbolId::Constant(symbol_id))
                    .expect("registered binding should have a canonical identity"),
            })
            .collect(),
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    }
}

fn single_file_module_symbols(
    dependencies: Vec<RetainedDependencyClause>,
    dependency_selections: Vec<DependencySelection>,
    string_table: &mut StringTable,
) -> ModuleSymbols {
    let mut module_symbols = ModuleSymbols::empty();
    let source_file = intern_path(&["src", "@page.moth"], string_table);
    module_symbols.module_file_paths.insert(source_file.clone());
    module_symbols
        .file_dependency_clauses_by_source
        .insert(source_file.clone(), dependencies);
    module_symbols
        .dependency_selections_by_source
        .insert(source_file, dependency_selections);
    module_symbols
}

fn bind_environment(
    provider_dependencies: &SourceProviderDependencySet<'_>,
    module_symbols: &mut ModuleSymbols,
    string_table: &mut StringTable,
) -> Result<
    crate::compiler_frontend::headers::binding_environment::HeaderBindingEnvironment,
    crate::compiler_frontend::compiler_errors::CompilerMessages,
> {
    prepare_binding_environment(BindingEnvironmentInput {
        module_symbols,
        external_package_registry: &ExternalPackageRegistry::new(),
        external_dependency_resolution_table: &ExternalImportResolutionTable::new(),
        source_provider_dependencies: provider_dependencies,
        string_table,
    })
}

#[test]
fn provider_selection_public_namespace_member_joins_declaration_surface() {
    let mut string_table = StringTable::new();
    let root_file = intern_path(&["facade", "@page.moth"], &mut string_table);
    let diagnostic_path = intern_path(&["provider", "CONST_0"], &mut string_table);
    let location = location_for(&["facade", "@page.moth"], &mut string_table);
    let export_name = string_table.intern("PUBLIC_CONST");
    let source_name = string_table.intern("CONST_0");
    let shell = DependencyShellId::new(FileId(0), 0);
    let provider = constant_provider("provider", &["CONST_0"]);
    let provider_dependencies = SourceProviderDependencySet::new(vec![SourceProviderDependency {
        kind: ProviderDependencyKind::Authored { shell },
        interface: &provider,
    }])
    .expect("provider shell should resolve");
    let registry = ExternalPackageRegistry::new();
    let external_dependency_resolution_table = ExternalImportResolutionTable::new();
    let module_symbols = ModuleSymbols::empty();
    let mut builder = BindingEnvironmentBuilder {
        module_symbols: &module_symbols,
        external_package_registry: &registry,
        external_dependency_resolution_table: &external_dependency_resolution_table,
        source_provider_dependencies: &provider_dependencies,
        string_table: &mut string_table,
        environment: Default::default(),
        warnings: Vec::new(),
        provider_semantics_registered: Default::default(),
    };
    let mut file_visibility = FileVisibility::default();
    let mut exported_entries = FxHashSet::default();
    exported_entries.insert(provider_selection_entry(
        export_name,
        source_name,
        shell,
        diagnostic_path,
    ));

    let record = builder
        .build_public_export_namespace_record(
            &mut file_visibility,
            &root_file,
            &exported_entries,
            &location,
        )
        .expect("provider-backed public export should build a namespace member");

    let Some(NamespaceValueMember::SourceDeclaration(SourceDeclarationTarget::Imported {
        origin,
        local_path,
    })) = record.value_members.get(&export_name)
    else {
        panic!("provider declaration should be a source declaration namespace member");
    };
    assert_eq!(local_path, &root_file.append(export_name));
    assert_eq!(
        builder
            .environment
            .imported_declarations_by_local_path
            .get(local_path),
        Some(origin)
    );
}

#[test]
fn provider_selection_namespace_prefers_source_name_over_facade_alias() {
    let mut string_table = StringTable::new();
    let root_file = intern_path(&["facade", "@page.moth"], &mut string_table);
    let diagnostic_path = intern_path(&["provider", "SOURCE"], &mut string_table);
    let location = location_for(&["facade", "@page.moth"], &mut string_table);
    let export_name = string_table.intern("ALIAS");
    let source_name = string_table.intern("SOURCE");
    let shell = DependencyShellId::new(FileId(0), 0);
    let provider = constant_provider("provider", &["SOURCE", "ALIAS"]);
    let provider_dependencies = SourceProviderDependencySet::new(vec![SourceProviderDependency {
        kind: ProviderDependencyKind::Authored { shell },
        interface: &provider,
    }])
    .expect("provider shell should resolve");
    let registry = ExternalPackageRegistry::new();
    let external_dependency_resolution_table = ExternalImportResolutionTable::new();
    let module_symbols = ModuleSymbols::empty();
    let mut builder = BindingEnvironmentBuilder {
        module_symbols: &module_symbols,
        external_package_registry: &registry,
        external_dependency_resolution_table: &external_dependency_resolution_table,
        source_provider_dependencies: &provider_dependencies,
        string_table: &mut string_table,
        environment: Default::default(),
        warnings: Vec::new(),
        provider_semantics_registered: Default::default(),
    };
    let mut file_visibility = FileVisibility::default();
    let mut exported_entries = FxHashSet::default();
    exported_entries.insert(provider_selection_entry(
        export_name,
        source_name,
        shell,
        diagnostic_path,
    ));

    let record = builder
        .build_public_export_namespace_record(
            &mut file_visibility,
            &root_file,
            &exported_entries,
            &location,
        )
        .expect("ambiguous provider namespace export should build");
    let Some(NamespaceValueMember::SourceDeclaration(SourceDeclarationTarget::Imported {
        origin,
        ..
    })) = record.value_members.get(&export_name)
    else {
        panic!("ambiguous declaration export should resolve to a provider declaration");
    };
    let OriginDeclarationId::Constant(origin) = origin else {
        panic!("test provider exports should resolve to a constant declaration");
    };
    assert_eq!(origin.defining_name(), "SOURCE");
}

#[test]
fn provider_selection_public_namespace_member_joins_binding_surface() {
    let mut string_table = StringTable::new();
    let root_file = intern_path(&["facade", "@page.moth"], &mut string_table);
    let diagnostic_path = intern_path(&["provider", "BOUND"], &mut string_table);
    let location = location_for(&["facade", "@page.moth"], &mut string_table);
    let export_name = string_table.intern("PUBLIC_BOUND");
    let source_name = string_table.intern("BOUND");
    let shell = DependencyShellId::new(FileId(0), 0);
    let mut registry = ExternalPackageRegistry::new();
    let provider = binding_provider(&mut registry);
    let provider_dependencies = SourceProviderDependencySet::new(vec![SourceProviderDependency {
        kind: ProviderDependencyKind::Authored { shell },
        interface: &provider,
    }])
    .expect("provider shell should resolve");
    let external_dependency_resolution_table = ExternalImportResolutionTable::new();
    let module_symbols = ModuleSymbols::empty();
    let mut builder = BindingEnvironmentBuilder {
        module_symbols: &module_symbols,
        external_package_registry: &registry,
        external_dependency_resolution_table: &external_dependency_resolution_table,
        source_provider_dependencies: &provider_dependencies,
        string_table: &mut string_table,
        environment: Default::default(),
        warnings: Vec::new(),
        provider_semantics_registered: Default::default(),
    };
    let mut file_visibility = FileVisibility::default();
    let mut exported_entries = FxHashSet::default();
    exported_entries.insert(provider_selection_entry(
        export_name,
        source_name,
        shell,
        diagnostic_path,
    ));

    let record = builder
        .build_public_export_namespace_record(
            &mut file_visibility,
            &root_file,
            &exported_entries,
            &location,
        )
        .expect("provider-backed binding export should build a namespace member");

    assert!(matches!(
        record.value_members.get(&export_name),
        Some(NamespaceValueMember::ExternalSymbol(
            ExternalSymbolId::Constant(_)
        ))
    ));
}

#[test]
fn provider_selection_namespace_binding_prefers_source_name_over_facade_alias() {
    let mut string_table = StringTable::new();
    let root_file = intern_path(&["facade", "@page.moth"], &mut string_table);
    let diagnostic_path = intern_path(&["provider", "SOURCE"], &mut string_table);
    let location = location_for(&["facade", "@page.moth"], &mut string_table);
    let export_name = string_table.intern("ALIAS");
    let source_name = string_table.intern("SOURCE");
    let shell = DependencyShellId::new(FileId(0), 0);
    let mut registry = ExternalPackageRegistry::new();
    let provider =
        binding_provider_with_members(&mut registry, &[("SOURCE", 5_101), ("ALIAS", 5_102)]);
    let provider_dependencies = SourceProviderDependencySet::new(vec![SourceProviderDependency {
        kind: ProviderDependencyKind::Authored { shell },
        interface: &provider,
    }])
    .expect("provider shell should resolve");
    let external_dependency_resolution_table = ExternalImportResolutionTable::new();
    let module_symbols = ModuleSymbols::empty();
    let mut builder = BindingEnvironmentBuilder {
        module_symbols: &module_symbols,
        external_package_registry: &registry,
        external_dependency_resolution_table: &external_dependency_resolution_table,
        source_provider_dependencies: &provider_dependencies,
        string_table: &mut string_table,
        environment: Default::default(),
        warnings: Vec::new(),
        provider_semantics_registered: Default::default(),
    };
    let mut file_visibility = FileVisibility::default();
    let mut exported_entries = FxHashSet::default();
    exported_entries.insert(provider_selection_entry(
        export_name,
        source_name,
        shell,
        diagnostic_path,
    ));

    let record = builder
        .build_public_export_namespace_record(
            &mut file_visibility,
            &root_file,
            &exported_entries,
            &location,
        )
        .expect("ambiguous provider binding export should build");
    let expected = registry
        .resolve_package_symbol("@test/provider_binding", "SOURCE")
        .expect("source binding should be registered");
    assert!(matches!(
        record.value_members.get(&export_name),
        Some(NamespaceValueMember::ExternalSymbol(symbol_id)) if *symbol_id == expected
    ));
}

#[test]
fn provider_selection_public_namespace_member_rejects_missing_shell() {
    let mut string_table = StringTable::new();
    let root_file = intern_path(&["facade", "@page.moth"], &mut string_table);
    let diagnostic_path = intern_path(&["provider", "CONST_0"], &mut string_table);
    let location = location_for(&["facade", "@page.moth"], &mut string_table);
    let export_name = string_table.intern("PUBLIC_CONST");
    let source_name = string_table.intern("CONST_0");
    let shell = DependencyShellId::new(FileId(0), 7);
    let provider_dependencies = SourceProviderDependencySet::default();
    let registry = ExternalPackageRegistry::new();
    let external_dependency_resolution_table = ExternalImportResolutionTable::new();
    let module_symbols = ModuleSymbols::empty();
    let mut builder = BindingEnvironmentBuilder {
        module_symbols: &module_symbols,
        external_package_registry: &registry,
        external_dependency_resolution_table: &external_dependency_resolution_table,
        source_provider_dependencies: &provider_dependencies,
        string_table: &mut string_table,
        environment: Default::default(),
        warnings: Vec::new(),
        provider_semantics_registered: Default::default(),
    };
    let mut file_visibility = FileVisibility::default();
    let mut exported_entries = FxHashSet::default();
    exported_entries.insert(provider_selection_entry(
        export_name,
        source_name,
        shell,
        diagnostic_path,
    ));

    let error = builder
        .build_public_export_namespace_record(
            &mut file_visibility,
            &root_file,
            &exported_entries,
            &location,
        )
        .expect_err("missing provider shell should remain an internal error");
    match error {
        BindingEnvironmentError::Internal(error) => {
            assert!(error.msg.contains("has no resolved provider"));
        }
        BindingEnvironmentError::Diagnostic(diagnostic) => {
            panic!("missing provider shell became a source diagnostic: {diagnostic:?}");
        }
    }
}

#[test]
fn provider_selection_public_namespace_member_rejects_missing_member() {
    let mut string_table = StringTable::new();
    let root_file = intern_path(&["facade", "@page.moth"], &mut string_table);
    let diagnostic_path = intern_path(&["provider", "MISSING"], &mut string_table);
    let location = location_for(&["facade", "@page.moth"], &mut string_table);
    let export_name = string_table.intern("PUBLIC_CONST");
    let source_name = string_table.intern("MISSING");
    let shell = DependencyShellId::new(FileId(0), 0);
    let provider = constant_provider("provider", &["CONST_0"]);
    let provider_dependencies = SourceProviderDependencySet::new(vec![SourceProviderDependency {
        kind: ProviderDependencyKind::Authored { shell },
        interface: &provider,
    }])
    .expect("provider shell should resolve");
    let registry = ExternalPackageRegistry::new();
    let external_dependency_resolution_table = ExternalImportResolutionTable::new();
    let module_symbols = ModuleSymbols::empty();
    let mut builder = BindingEnvironmentBuilder {
        module_symbols: &module_symbols,
        external_package_registry: &registry,
        external_dependency_resolution_table: &external_dependency_resolution_table,
        source_provider_dependencies: &provider_dependencies,
        string_table: &mut string_table,
        environment: Default::default(),
        warnings: Vec::new(),
        provider_semantics_registered: Default::default(),
    };
    let mut file_visibility = FileVisibility::default();
    let mut exported_entries = FxHashSet::default();
    exported_entries.insert(provider_selection_entry(
        export_name,
        source_name,
        shell,
        diagnostic_path,
    ));

    let error = builder
        .build_public_export_namespace_record(
            &mut file_visibility,
            &root_file,
            &exported_entries,
            &location,
        )
        .expect_err("missing provider member should remain an internal error");
    match error {
        BindingEnvironmentError::Internal(error) => {
            assert!(error.msg.contains("has no exported provider member"));
        }
        BindingEnvironmentError::Diagnostic(diagnostic) => {
            panic!("missing provider member became a source diagnostic: {diagnostic:?}");
        }
    }
}

#[test]
fn namespace_and_direct_selection_share_provider_semantics() {
    let mut string_table = StringTable::new();
    let provider = constant_provider("provider", &["CONST_0", "CONST_1"]);

    let mut dependency_selections = Vec::new();
    let mut direct_selection = test_dependency(
        intern_path(&["provider"], &mut string_table),
        &mut string_table,
    );
    direct_selection.dependency.dependency_shell_id = DependencyShellId::new(FileId(0), 0);
    add_selection(
        &mut direct_selection,
        &mut dependency_selections,
        "CONST_0",
        None,
        &mut string_table,
    );
    let mut namespace = test_dependency(
        intern_path(&["provider"], &mut string_table),
        &mut string_table,
    );
    namespace.dependency.dependency_shell_id = DependencyShellId::new(FileId(0), 1);

    let mut module_symbols = single_file_module_symbols(
        vec![direct_selection, namespace],
        dependency_selections,
        &mut string_table,
    );
    let provider_dependencies = SourceProviderDependencySet::new(vec![
        SourceProviderDependency {
            kind: ProviderDependencyKind::Authored {
                shell: DependencyShellId::new(FileId(0), 0),
            },
            interface: &provider,
        },
        SourceProviderDependency {
            kind: ProviderDependencyKind::Authored {
                shell: DependencyShellId::new(FileId(0), 1),
            },
            interface: &provider,
        },
    ])
    .expect("two shells should collapse to one provider");

    let environment = bind_environment(
        &provider_dependencies,
        &mut module_symbols,
        &mut string_table,
    )
    .expect("direct-selection and namespace dependencies should bind");

    assert_eq!(environment.imported_declarations_by_origin.len(), 2);
    assert_eq!(
        environment.imported_evidence_by_identity.len(),
        1,
        "direct-selection and namespace dependencies must share one evidence record"
    );
    assert!(environment.imported_call_summaries_by_origin.is_empty());
}

#[test]
fn two_aliases_of_one_declaration_retain_one_record() {
    let mut string_table = StringTable::new();
    let provider = constant_provider("provider", &["CONST_0"]);

    let mut dependency_selections = Vec::new();
    let mut first = test_dependency(
        intern_path(&["provider"], &mut string_table),
        &mut string_table,
    );
    first.dependency.dependency_shell_id = DependencyShellId::new(FileId(0), 0);
    add_selection(
        &mut first,
        &mut dependency_selections,
        "CONST_0",
        Some("first_alias"),
        &mut string_table,
    );
    let mut second = test_dependency(
        intern_path(&["provider"], &mut string_table),
        &mut string_table,
    );
    second.dependency.dependency_shell_id = DependencyShellId::new(FileId(0), 1);
    add_selection(
        &mut second,
        &mut dependency_selections,
        "CONST_0",
        Some("second_alias"),
        &mut string_table,
    );

    let mut module_symbols = single_file_module_symbols(
        vec![first, second],
        dependency_selections,
        &mut string_table,
    );
    let provider_dependencies = SourceProviderDependencySet::new(vec![
        SourceProviderDependency {
            kind: ProviderDependencyKind::Authored {
                shell: DependencyShellId::new(FileId(0), 0),
            },
            interface: &provider,
        },
        SourceProviderDependency {
            kind: ProviderDependencyKind::Authored {
                shell: DependencyShellId::new(FileId(0), 1),
            },
            interface: &provider,
        },
    ])
    .expect("two shells should collapse to one provider");

    let environment = bind_environment(
        &provider_dependencies,
        &mut module_symbols,
        &mut string_table,
    )
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

    let mut missing = test_dependency(
        intern_path(&["provider"], &mut string_table),
        &mut string_table,
    );
    missing.dependency.dependency_shell_id = DependencyShellId::new(FileId(0), 0);
    let mut dependency_selections = Vec::new();
    add_selection(
        &mut missing,
        &mut dependency_selections,
        "MISSING",
        None,
        &mut string_table,
    );
    let selection_location = location_for(&["src", "selected.moth"], &mut string_table);
    dependency_selections[0].source_location = selection_location.clone();

    let mut module_symbols =
        single_file_module_symbols(vec![missing], dependency_selections, &mut string_table);
    let provider_dependencies = SourceProviderDependencySet::new(vec![SourceProviderDependency {
        kind: ProviderDependencyKind::Authored {
            shell: DependencyShellId::new(FileId(0), 0),
        },
        interface: &provider,
    }])
    .expect("one shell should register");

    let messages = bind_environment(
        &provider_dependencies,
        &mut module_symbols,
        &mut string_table,
    )
    .expect_err("a missing provider record must fail binding deterministically");
    let diagnostic = &messages.diagnostics[0];
    let DiagnosticPayload::NotExportedByPublicSurface { requested_path, .. } = &diagnostic.payload
    else {
        panic!("unexpected diagnostic payload: {:?}", diagnostic.payload);
    };
    assert_eq!(
        requested_path.to_portable_string(&string_table),
        "provider/MISSING"
    );
    assert_eq!(diagnostic.primary_location, selection_location);
}

#[test]
fn receiver_methods_reuse_summary_by_origin_storage() {
    let mut string_table = StringTable::new();
    let provider = struct_provider_with_receiver_method();

    let mut dependency_selections = Vec::new();
    let dependencies = (0..2)
        .map(|index| {
            let mut dependency = test_dependency(
                intern_path(&["shapes"], &mut string_table),
                &mut string_table,
            );
            dependency.dependency.dependency_shell_id = DependencyShellId::new(FileId(0), index);
            add_selection(
                &mut dependency,
                &mut dependency_selections,
                "Box",
                None,
                &mut string_table,
            );
            dependency
        })
        .collect();
    let mut module_symbols =
        single_file_module_symbols(dependencies, dependency_selections, &mut string_table);
    let provider_dependencies = SourceProviderDependencySet::new(
        (0..2)
            .map(|index| SourceProviderDependency {
                kind: ProviderDependencyKind::Authored {
                    shell: DependencyShellId::new(FileId(0), index),
                },
                interface: &provider,
            })
            .collect(),
    )
    .expect("two shells should collapse to one provider");

    let environment = bind_environment(
        &provider_dependencies,
        &mut module_symbols,
        &mut string_table,
    )
    .expect("receiver method dependencies should bind");

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
            folded_value: PublicFoldedValue::String(OwnedFoldedString::Text("first".to_owned())),
        }),
    };
    let mut second = first.clone();
    let PublicDeclarationSemantics::Constant(second_constant) = &mut second.semantics else {
        unreachable!("declaration semantics is constant");
    };
    second_constant.folded_value =
        PublicFoldedValue::String(OwnedFoldedString::Text("second".to_owned()));

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
