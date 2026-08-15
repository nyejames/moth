//! Host function metadata regression tests.
//!
//! WHAT: exercises host return-slot derivation and registry uniqueness rules.
//! WHY: host metadata feeds both AST lowering and borrow-check call summaries, so small
//! regressions here can break multiple frontend stages at once.

use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::{
    CanonicalBindingSymbolIdentity, ExternalAbiType, ExternalAccessKind, ExternalConstantDef,
    ExternalConstantId, ExternalConstantValue, ExternalFunctionDef, ExternalFunctionId,
    ExternalFunctionLowerings, ExternalJsLowering, ExternalPackageRegistry, ExternalParameter,
    ExternalReturnAlias, ExternalReturnSlot, ExternalSignatureType, ExternalSymbolCategory,
    ExternalSymbolId, ExternalSymbolPath, ExternalTypeDef, ExternalTypeId,
    IO_INPUT_EXTERNAL_TYPE_ID, external_success_returns,
};
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

fn import_path(components: &[&str], string_table: &mut StringTable) -> InternedPath {
    InternedPath::from_components(
        components
            .iter()
            .map(|component| string_table.intern(component))
            .collect(),
    )
}

#[test]
fn return_slots_preserve_alias_metadata() {
    let host_function = ExternalFunctionDef {
        name: "concat_like".to_owned(),
        parameters: vec![
            ExternalParameter {
                language_type: ExternalSignatureType::Abi(ExternalAbiType::Utf8Str),
                access_kind: ExternalAccessKind::Shared,
            },
            ExternalParameter {
                language_type: ExternalSignatureType::Abi(ExternalAbiType::Utf8Str),
                access_kind: ExternalAccessKind::Shared,
            },
        ],
        returns: vec![ExternalReturnSlot::alias_args(
            ExternalAbiType::Utf8Str,
            vec![1],
        )],
        error_return_type: None,
        lowerings: ExternalFunctionLowerings::default(),
    };

    let returns = &host_function.returns;
    assert_eq!(returns.len(), 1);
    assert_eq!(
        returns[0].value_type,
        ExternalSignatureType::Abi(ExternalAbiType::Utf8Str)
    );
    assert!(matches!(
        &returns[0].alias,
        ExternalReturnAlias::AliasArgs(parameter_indices) if parameter_indices == &[1usize]
    ));
}

#[test]
fn register_function_rejects_duplicates() {
    let mut registry = ExternalPackageRegistry::new();
    registry
        .register_function(ExternalFunctionDef {
            name: "test_func".to_owned(),
            parameters: Vec::new(),
            returns: external_success_returns(ExternalAbiType::Void, ExternalReturnAlias::Fresh),
            error_return_type: None,
            lowerings: ExternalFunctionLowerings::default(),
        })
        .unwrap();

    let result = registry.register_function(ExternalFunctionDef {
        name: "test_func".to_owned(),
        parameters: Vec::new(),
        returns: external_success_returns(ExternalAbiType::Void, ExternalReturnAlias::Fresh),
        error_return_type: None,
        lowerings: ExternalFunctionLowerings::default(),
    });

    assert!(result.is_err());
}

#[test]
fn collection_helpers_keep_receiver_parameter_access_modes() {
    let registry = ExternalPackageRegistry::new();
    let push = registry
        .get_function_by_id(ExternalFunctionId::CollectionPush)
        .unwrap();
    assert_eq!(push.parameters[0].access_kind, ExternalAccessKind::Mutable);

    let set = registry
        .get_function_by_id(ExternalFunctionId::CollectionSet)
        .unwrap();
    assert_eq!(set.parameters[0].access_kind, ExternalAccessKind::Mutable);

    let length = registry
        .get_function_by_id(ExternalFunctionId::CollectionLength)
        .unwrap();
    assert_eq!(length.parameters[0].access_kind, ExternalAccessKind::Shared);
}

#[test]
fn same_symbol_name_across_packages_is_allowed() {
    let registry = ExternalPackageRegistry::new().with_test_packages_for_integration();

    // Both @test/pkg-a and @test/pkg-b expose a function named "open".
    let a_result = registry.resolve_package_function("@test/pkg-a", "open");
    assert!(a_result.is_some(), "@test/pkg-a/open should resolve");

    let b_result = registry.resolve_package_function("@test/pkg-b", "open");
    assert!(b_result.is_some(), "@test/pkg-b/open should resolve");

    // They must map to distinct IDs.
    let (a_id, _) = a_result.unwrap();
    let (b_id, _) = b_result.unwrap();
    assert_ne!(
        a_id, b_id,
        "same symbol in different packages must have distinct IDs"
    );
}

#[test]
fn resolve_package_function_selects_correct_package() {
    let registry = ExternalPackageRegistry::new().with_test_packages_for_integration();

    let (a_id, a_def) = registry
        .resolve_package_function("@test/pkg-a", "open")
        .unwrap();
    let (b_id, b_def) = registry
        .resolve_package_function("@test/pkg-b", "open")
        .unwrap();

    assert_eq!(a_def.name, "open");
    assert_eq!(b_def.name, "open");
    assert_ne!(a_id, b_id);
}

// ------------------------------------------------------------------
// Package identity refactor tests
// ------------------------------------------------------------------

#[test]
fn builtin_packages_resolve_by_path_and_symbol_name() {
    let registry = ExternalPackageRegistry::new();

    let io = registry.resolve_package_function("@core/io", "line");
    assert!(
        io.is_some(),
        "@core/io/line should resolve by path and name"
    );

    let get = registry.resolve_package_function("@core/collections", "__moth_collection_get");
    assert!(
        get.is_some(),
        "@core/collections/__moth_collection_get should resolve"
    );
}

#[test]
fn package_ids_are_stable_within_one_registry_build() {
    let registry_a = ExternalPackageRegistry::new();
    let registry_b = ExternalPackageRegistry::new();

    let io_a = registry_a.get_package("@core/io").unwrap();
    let io_b = registry_b.get_package("@core/io").unwrap();

    assert_eq!(
        io_a.id, io_b.id,
        "builtin package IDs must be deterministic"
    );
}

#[test]
fn package_origin_recorded_for_builtins() {
    let registry = ExternalPackageRegistry::new();

    let io = registry.get_package("@core/io").unwrap();
    assert_eq!(
        io.metadata,
        crate::builder_surface::PackageMetadata::binding(
            crate::builder_surface::PackageOrigin::Core
        )
    );
    assert_eq!(io.path, "@core/io");

    let collections = registry.get_package("@core/collections").unwrap();
    assert_eq!(
        collections.metadata,
        crate::builder_surface::PackageMetadata::binding(
            crate::builder_surface::PackageOrigin::Core
        )
    );
}

#[test]
fn package_origin_recorded_for_integration_test_packages() {
    let registry = ExternalPackageRegistry::new().with_test_packages_for_integration();

    let pkg_a = registry.get_package("@test/pkg-a").unwrap();
    assert_eq!(
        pkg_a.metadata,
        crate::builder_surface::PackageMetadata::binding(
            crate::builder_surface::PackageOrigin::Builder
        )
    );

    let pkg_b = registry.get_package("@test/pkg-b").unwrap();
    assert_eq!(
        pkg_b.metadata,
        crate::builder_surface::PackageMetadata::binding(
            crate::builder_surface::PackageOrigin::Builder
        )
    );
}

#[test]
fn resolve_function_package_returns_readable_path() {
    let registry = ExternalPackageRegistry::new();

    let package_path = registry.resolve_function_package(ExternalFunctionId::IoLine);
    assert_eq!(package_path, Some("@core/io"));
}

#[test]
fn package_path_to_id_index_is_consistent() {
    let registry = ExternalPackageRegistry::new();

    let io_id = registry.resolve_package_id("@core/io");
    assert!(io_id.is_some());

    let by_id = registry.get_package_by_id(io_id.unwrap());
    assert!(by_id.is_some());
    assert_eq!(by_id.unwrap().path, "@core/io");
}

#[test]
fn package_prefix_lookup_returns_longest_registered_package() {
    let mut registry = ExternalPackageRegistry::new();
    registry
        .register_package("@test", crate::builder_surface::PackageOrigin::Builder)
        .expect("parent test package should register");
    let child_id = registry
        .register_package("@test/pkg", crate::builder_surface::PackageOrigin::Builder)
        .expect("child test package should register");

    let mut string_table = StringTable::new();
    let path = import_path(&["test", "pkg", "open"], &mut string_table);
    let matched = registry
        .longest_package_prefix_for_dependency(&path, &string_table)
        .expect("package prefix should match");

    assert_eq!(matched.package_path, "@test/pkg");
    assert_eq!(matched.package_id, child_id);
    assert_eq!(matched.matched_component_count, 2);
}

#[test]
fn package_prefix_lookup_supports_exact_namespace_bindings() {
    let mut registry = ExternalPackageRegistry::new();
    crate::builder_surface::core_packages::register_core_math_package(&mut registry);

    let mut string_table = StringTable::new();
    let path = import_path(&["core", "math"], &mut string_table);
    let matched = registry
        .longest_package_prefix_for_dependency(&path, &string_table)
        .expect("core math package should match");

    assert_eq!(matched.package_path, "@core/math");
    assert_eq!(matched.matched_component_count, path.len());
}

#[test]
fn virtual_package_detection_uses_symbol_suffixes() {
    let mut registry = ExternalPackageRegistry::new();
    crate::builder_surface::core_packages::register_core_math_package(&mut registry);

    let mut string_table = StringTable::new();
    let package_symbol = import_path(&["core", "math", "sin"], &mut string_table);
    let source_path = import_path(&["core", "missing", "sin"], &mut string_table);

    assert!(registry.is_virtual_package_dependency(&package_symbol, &string_table));
    assert!(!registry.is_virtual_package_dependency(&source_path, &string_table));
}

// ------------------------------------------------------------------
// Phase 1.1 path-aware external package surface tests
// ------------------------------------------------------------------

fn empty_void_function(name: &str) -> ExternalFunctionDef {
    ExternalFunctionDef {
        name: name.to_owned(),
        parameters: Vec::new(),
        returns: external_success_returns(ExternalAbiType::Void, ExternalReturnAlias::Fresh),
        error_return_type: None,
        lowerings: ExternalFunctionLowerings::default(),
    }
}

fn scalar_int_constant(name: &str, value: i32) -> ExternalConstantDef {
    ExternalConstantDef {
        name: name.to_owned(),
        data_type: ExternalAbiType::I32,
        value: ExternalConstantValue::Int(value),
    }
}

#[test]
fn duplicate_explicit_function_id_is_rejected_without_mutation() {
    let mut registry = ExternalPackageRegistry::default();
    let package_id = registry
        .register_package(
            "@test/functions",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("test package should register");
    let function_id = ExternalFunctionId::Synthetic(700);
    let original_path =
        ExternalSymbolPath::from_components(vec!["nested".to_owned(), "original".to_owned()]);
    let duplicate_path = ExternalSymbolPath::from_single("duplicate");

    registry
        .register_function_at_path(
            package_id,
            original_path.clone(),
            function_id,
            empty_void_function("original"),
        )
        .expect("the original function should register");
    let result = registry.register_function_at_path(
        package_id,
        duplicate_path.clone(),
        function_id,
        empty_void_function("duplicate"),
    );

    assert!(result.is_err(), "an explicit function ID must be injective");
    assert!(
        registry
            .resolve_package_function_by_path("@test/functions", &duplicate_path)
            .is_none(),
        "the rejected path must not mutate the package surface"
    );
    let identity = registry
        .canonical_symbol_identity(ExternalSymbolId::Function(function_id))
        .expect("the original reverse identity must remain intact");
    assert_eq!(identity.symbol_path, original_path);
}

#[test]
fn duplicate_explicit_constant_id_is_rejected_without_mutation() {
    let mut registry = ExternalPackageRegistry::default();
    let package_id = registry
        .register_package(
            "@test/constants",
            crate::builder_surface::PackageOrigin::Core,
        )
        .expect("test package should register");
    let constant_id = ExternalConstantId(701);
    let original_path =
        ExternalSymbolPath::from_components(vec!["nested".to_owned(), "ORIGINAL".to_owned()]);
    let duplicate_path = ExternalSymbolPath::from_single("DUPLICATE");

    registry
        .register_constant_at_path(
            package_id,
            original_path.clone(),
            constant_id,
            scalar_int_constant("ORIGINAL", 1),
        )
        .expect("the original constant should register");
    let result = registry.register_constant_at_path(
        package_id,
        duplicate_path.clone(),
        constant_id,
        scalar_int_constant("DUPLICATE", 2),
    );

    assert!(result.is_err(), "an explicit constant ID must be injective");
    assert!(
        registry
            .resolve_package_constant_by_path("@test/constants", &duplicate_path)
            .is_none(),
        "the rejected path must not mutate the package surface"
    );
    let identity = registry
        .canonical_symbol_identity(ExternalSymbolId::Constant(constant_id))
        .expect("the original reverse identity must remain intact");
    assert_eq!(identity.symbol_path, original_path);
}

#[test]
fn canonical_binding_identity_resolves_across_independent_registries() {
    let function_path =
        ExternalSymbolPath::from_components(vec!["nested".to_owned(), "call".to_owned()]);
    let type_path =
        ExternalSymbolPath::from_components(vec!["nested".to_owned(), "Thing".to_owned()]);
    let constant_path =
        ExternalSymbolPath::from_components(vec!["nested".to_owned(), "VALUE".to_owned()]);
    let mut provider = ExternalPackageRegistry::default();
    let provider_package = provider
        .register_package(
            "@test/shared",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("provider package should register");
    provider
        .register_function_at_path(
            provider_package,
            function_path.clone(),
            ExternalFunctionId::Synthetic(702),
            empty_void_function("call"),
        )
        .expect("provider function should register");
    provider
        .register_type_at_path(
            provider_package,
            type_path.clone(),
            ExternalTypeId(704),
            ExternalTypeDef {
                name: "Thing".to_owned(),
                package_id: provider_package,
                abi_type: ExternalAbiType::Handle,
            },
        )
        .expect("provider type should register");
    provider
        .register_constant_at_path(
            provider_package,
            constant_path.clone(),
            ExternalConstantId(705),
            scalar_int_constant("VALUE", 1),
        )
        .expect("provider constant should register");

    let function_identity = provider
        .canonical_symbol_identity(ExternalSymbolId::Function(ExternalFunctionId::Synthetic(
            702,
        )))
        .expect("provider should publish a canonical identity");
    let type_identity = provider
        .canonical_symbol_identity(ExternalSymbolId::Type(ExternalTypeId(704)))
        .expect("provider should publish a canonical type identity");
    let constant_identity = provider
        .canonical_symbol_identity(ExternalSymbolId::Constant(ExternalConstantId(705)))
        .expect("provider should publish a canonical constant identity");

    let mut consumer = ExternalPackageRegistry::default();
    consumer
        .register_package(
            "@test/unrelated",
            crate::builder_surface::PackageOrigin::Core,
        )
        .expect("unrelated package should shift the local package ID");
    let consumer_package = consumer
        .register_package(
            "@test/shared",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("consumer package should register independently");
    consumer
        .register_function_at_path(
            consumer_package,
            function_path.clone(),
            ExternalFunctionId::Synthetic(703),
            empty_void_function("call"),
        )
        .expect("consumer function should register with a different local ID");
    consumer
        .register_type_at_path(
            consumer_package,
            type_path,
            ExternalTypeId(706),
            ExternalTypeDef {
                name: "Thing".to_owned(),
                package_id: consumer_package,
                abi_type: ExternalAbiType::Handle,
            },
        )
        .expect("consumer type should register with a different local ID");
    consumer
        .register_constant_at_path(
            consumer_package,
            constant_path,
            ExternalConstantId(707),
            scalar_int_constant("VALUE", 1),
        )
        .expect("consumer constant should register with a different local ID");

    assert_eq!(
        consumer.resolve_canonical_symbol(&function_identity),
        Some(ExternalSymbolId::Function(ExternalFunctionId::Synthetic(
            703
        )))
    );
    assert_eq!(
        consumer.resolve_canonical_symbol(&type_identity),
        Some(ExternalSymbolId::Type(ExternalTypeId(706)))
    );
    assert_eq!(
        consumer.resolve_canonical_symbol(&constant_identity),
        Some(ExternalSymbolId::Constant(ExternalConstantId(707)))
    );

    let mismatched_origin = CanonicalBindingSymbolIdentity {
        package: StablePackageIdentity::binding(
            crate::builder_surface::PackageOrigin::ProjectLocal,
            "@test/shared",
        ),
        symbol_path: function_path,
        category: ExternalSymbolCategory::Function,
    };
    assert!(
        consumer
            .resolve_canonical_symbol(&mismatched_origin)
            .is_none(),
        "the same package path from another origin must not resolve"
    );
}

#[test]
fn register_function_at_path_rejects_duplicate_path() {
    let mut registry = ExternalPackageRegistry::new();
    let package_id = registry
        .register_package("@test/path", crate::builder_surface::PackageOrigin::Builder)
        .expect("test package registration should not collide");

    registry
        .register_function_at_path(
            package_id,
            ExternalSymbolPath::from_single("foo"),
            ExternalFunctionId::Synthetic(10),
            empty_void_function("foo"),
        )
        .expect("first registration at foo should succeed");

    let result = registry.register_function_at_path(
        package_id,
        ExternalSymbolPath::from_single("foo"),
        ExternalFunctionId::Synthetic(11),
        empty_void_function("foo"),
    );

    assert!(
        result.is_err(),
        "duplicate function path in one package must be rejected"
    );
}

#[test]
fn same_function_leaf_under_different_namespaces_allowed() {
    let mut registry = ExternalPackageRegistry::new();
    let package_id = registry
        .register_package("@test/path", crate::builder_surface::PackageOrigin::Builder)
        .expect("test package registration should not collide");

    registry
        .register_function_at_path(
            package_id,
            ExternalSymbolPath::from_components(vec!["a".to_owned(), "foo".to_owned()]),
            ExternalFunctionId::Synthetic(20),
            empty_void_function("foo"),
        )
        .expect("a.foo should register");

    registry
        .register_function_at_path(
            package_id,
            ExternalSymbolPath::from_components(vec!["b".to_owned(), "foo".to_owned()]),
            ExternalFunctionId::Synthetic(21),
            empty_void_function("foo"),
        )
        .expect("b.foo should register");

    let a_foo = registry
        .resolve_package_function_by_path(
            "@test/path",
            &ExternalSymbolPath::from_components(vec!["a".to_owned(), "foo".to_owned()]),
        )
        .map(|(id, _)| id);
    let b_foo = registry
        .resolve_package_function_by_path(
            "@test/path",
            &ExternalSymbolPath::from_components(vec!["b".to_owned(), "foo".to_owned()]),
        )
        .map(|(id, _)| id);

    assert_eq!(a_foo, Some(ExternalFunctionId::Synthetic(20)));
    assert_eq!(b_foo, Some(ExternalFunctionId::Synthetic(21)));
    assert_ne!(
        a_foo, b_foo,
        "same leaf under different namespaces must have distinct IDs"
    );
}

#[test]
fn function_type_collision_at_same_path_rejected() {
    let mut registry = ExternalPackageRegistry::new();
    let package_id = registry
        .register_package("@test/path", crate::builder_surface::PackageOrigin::Builder)
        .expect("test package registration should not collide");

    registry
        .register_function_at_path(
            package_id,
            ExternalSymbolPath::from_single("foo"),
            ExternalFunctionId::Synthetic(30),
            empty_void_function("foo"),
        )
        .expect("function at foo should register");

    let result = registry.register_type_at_path(
        package_id,
        ExternalSymbolPath::from_single("foo"),
        ExternalTypeId(31),
        ExternalTypeDef {
            name: "foo".to_owned(),
            package_id,
            abi_type: ExternalAbiType::Handle,
        },
    );

    assert!(
        result.is_err(),
        "function and type at the same path must be rejected"
    );
}

#[test]
fn function_constant_collision_at_same_path_rejected() {
    let mut registry = ExternalPackageRegistry::new();
    let package_id = registry
        .register_package("@test/path", crate::builder_surface::PackageOrigin::Builder)
        .expect("test package registration should not collide");

    registry
        .register_function_at_path(
            package_id,
            ExternalSymbolPath::from_single("foo"),
            ExternalFunctionId::Synthetic(40),
            empty_void_function("foo"),
        )
        .expect("function at foo should register");

    let result = registry.register_constant_at_path(
        package_id,
        ExternalSymbolPath::from_single("foo"),
        ExternalConstantId(41),
        ExternalConstantDef {
            name: "foo".to_owned(),
            data_type: ExternalAbiType::F64,
            value: ExternalConstantValue::Float(1.0),
        },
    );

    assert!(
        result.is_err(),
        "function and constant at the same path must be rejected"
    );
}

#[test]
fn nested_function_path_registers_and_resolves() {
    let mut registry = ExternalPackageRegistry::new();
    let package_id = registry
        .register_package("@test/path", crate::builder_surface::PackageOrigin::Builder)
        .expect("test package registration should not collide");

    let path = ExternalSymbolPath::from_components(vec![
        "io".to_owned(),
        "input".to_owned(),
        "new".to_owned(),
    ]);

    registry
        .register_function_at_path(
            package_id,
            path.clone(),
            ExternalFunctionId::Synthetic(50),
            empty_void_function("new"),
        )
        .expect("nested path registration should succeed");

    let resolved = registry
        .resolve_package_function_by_path("@test/path", &path)
        .map(|(id, _)| id);
    assert_eq!(resolved, Some(ExternalFunctionId::Synthetic(50)));

    // One-component lookup for the same leaf should not find the nested symbol.
    let flat = registry.resolve_package_function("@test/path", "new");
    assert!(
        flat.is_none(),
        "nested symbol must not resolve through flat leaf lookup"
    );
}

#[test]
fn one_component_external_imports_still_resolve_by_path() {
    let registry = ExternalPackageRegistry::new();

    let io = registry
        .resolve_package_function_by_path("@core/io", &ExternalSymbolPath::from_single("line"))
        .map(|(id, _)| id);
    assert_eq!(io, Some(ExternalFunctionId::IoLine));

    let io_flat = registry.resolve_package_function("@core/io", "line");
    assert!(io_flat.is_some(), "one-component lookup must still resolve");
}

#[test]
fn package_surface_iteration_exposes_one_component_paths_only() {
    let mut registry = ExternalPackageRegistry::new();
    let package_id = registry
        .register_package("@test/path", crate::builder_surface::PackageOrigin::Builder)
        .expect("test package registration should not collide");

    registry
        .register_function_at_path(
            package_id,
            ExternalSymbolPath::from_single("line"),
            ExternalFunctionId::Synthetic(60),
            empty_void_function("line"),
        )
        .unwrap();
    registry
        .register_function_at_path(
            package_id,
            ExternalSymbolPath::from_components(vec!["input".to_owned(), "new".to_owned()]),
            ExternalFunctionId::Synthetic(61),
            empty_void_function("new"),
        )
        .unwrap();

    let package = registry.get_package("@test/path").unwrap();
    let flat_names: Vec<&str> = package
        .function_symbol_ids()
        .filter(|(path, _)| path.is_single())
        .map(|(path, _)| path.leaf())
        .collect();
    assert_eq!(flat_names, vec!["line"]);
}

// ------------------------------------------------------------------
// StringContent signature tests
// ------------------------------------------------------------------

#[test]
fn string_content_resolves_to_string_datatype() {
    assert_eq!(
        ExternalSignatureType::StringContent.to_datatype(),
        Some(DataType::StringSlice)
    );
}

#[test]
fn string_content_resolves_to_canonical_string_type_id() {
    let mut type_environment = TypeEnvironment::new();
    let string_type_id = type_environment.builtins().string;

    assert_eq!(
        ExternalSignatureType::StringContent.to_parameter_type_id(&mut type_environment),
        Some(string_type_id)
    );

    let none_type_id = type_environment.builtins().none;
    let return_type_id =
        ExternalSignatureType::StringContent.to_type_id(&mut type_environment, none_type_id);
    assert_eq!(return_type_id, Some(string_type_id));
}

#[test]
fn prelude_symbols_and_namespace_aliases_cannot_share_public_name() {
    let mut registry = ExternalPackageRegistry::new();

    registry
        .register_prelude_namespace_alias("shared", "@core/io")
        .expect("first prelude namespace alias should register");
    let symbol_result =
        registry.register_prelude_symbol("shared", ExternalSymbolId::Type(ExternalTypeId(9000)));
    assert!(
        symbol_result.is_err(),
        "prelude symbol should reject a namespace alias name"
    );

    let mut registry = ExternalPackageRegistry::new();
    registry
        .register_prelude_symbol("shared", ExternalSymbolId::Type(ExternalTypeId(9001)))
        .expect("first prelude symbol should register");
    let alias_result = registry.register_prelude_namespace_alias("shared", "@core/io");
    assert!(
        alias_result.is_err(),
        "prelude namespace alias should reject a symbol name"
    );
}

// ------------------------------------------------------------------
// Core IO console namespace tests
// ------------------------------------------------------------------

/// Verifies that the five V1 console functions are registered under `@core/io`.
/// WHAT: ensures the public `io.print/line/debug/warn/error` surface is present.
/// WHY: the previous flat `io(...)` function and `IO` type are gone; these replace them.
#[test]
fn core_io_console_functions_are_registered_at_namespace_path() {
    let registry = ExternalPackageRegistry::new();

    let print = registry
        .resolve_package_function_by_path("@core/io", &ExternalSymbolPath::from_single("print"))
        .map(|(id, _)| id);
    let line = registry
        .resolve_package_function_by_path("@core/io", &ExternalSymbolPath::from_single("line"))
        .map(|(id, _)| id);
    let debug = registry
        .resolve_package_function_by_path("@core/io", &ExternalSymbolPath::from_single("debug"))
        .map(|(id, _)| id);
    let warn = registry
        .resolve_package_function_by_path("@core/io", &ExternalSymbolPath::from_single("warn"))
        .map(|(id, _)| id);
    let error = registry
        .resolve_package_function_by_path("@core/io", &ExternalSymbolPath::from_single("error"))
        .map(|(id, _)| id);

    assert_eq!(print, Some(ExternalFunctionId::IoPrint));
    assert_eq!(line, Some(ExternalFunctionId::IoLine));
    assert_eq!(debug, Some(ExternalFunctionId::IoDebug));
    assert_eq!(warn, Some(ExternalFunctionId::IoWarn));
    assert_eq!(error, Some(ExternalFunctionId::IoError));
}

/// Verifies that each console function accepts only string-content input.
/// WHAT: console output must reject non-string values at the external boundary.
/// WHY: this replaces the old IO-specific scalar-rendering validation branch with the
/// reusable `StringContent` signature path.
#[test]
fn core_io_console_functions_use_string_content_parameter() {
    let registry = ExternalPackageRegistry::new();

    for function_id in [
        ExternalFunctionId::IoPrint,
        ExternalFunctionId::IoLine,
        ExternalFunctionId::IoDebug,
        ExternalFunctionId::IoWarn,
        ExternalFunctionId::IoError,
    ] {
        let function = registry
            .get_function_by_id(function_id)
            .expect("console function should be registered");
        assert_eq!(
            function.parameters.len(),
            1,
            "{} should take exactly one argument",
            function_id.name()
        );
        assert_eq!(
            function.parameters[0].language_type,
            ExternalSignatureType::StringContent,
            "{} should use StringContent parameter",
            function_id.name()
        );
    }
}

/// Verifies that console functions return Void and have no error slot.
/// WHAT: V1 console output is infallible and produces no value.
/// WHY: callers must not be able to postfix `!` or `catch` these calls.
#[test]
fn core_io_console_functions_return_void() {
    let registry = ExternalPackageRegistry::new();

    for function_id in [
        ExternalFunctionId::IoPrint,
        ExternalFunctionId::IoLine,
        ExternalFunctionId::IoDebug,
        ExternalFunctionId::IoWarn,
        ExternalFunctionId::IoError,
    ] {
        let function = registry
            .get_function_by_id(function_id)
            .expect("console function should be registered");
        assert!(
            function.returns.is_empty(),
            "{} should return no slots because Void maps to zero returns, got {}",
            function_id.name(),
            function.returns.len()
        );
        assert!(
            function.error_return_type.is_none(),
            "{} should not have an error return type",
            function_id.name()
        );
    }
}

/// Verifies that every Core IO V1 function is JS-backed and intentionally unsupported on Wasm.
/// WHAT: backend validation only needs lowering metadata; unsupported targets are represented by
///       an absent lowering entry rather than a hand-written IO special case.
/// WHY: Phase 5 depends on this metadata shape so reachable HTML-Wasm IO calls fail before
///      lowering while HTML-JS continues to emit runtime helpers.
#[test]
fn core_io_functions_have_js_lowerings_and_no_wasm_lowerings() {
    let registry = ExternalPackageRegistry::new();

    for function_id in [
        ExternalFunctionId::IoPrint,
        ExternalFunctionId::IoLine,
        ExternalFunctionId::IoDebug,
        ExternalFunctionId::IoWarn,
        ExternalFunctionId::IoError,
        ExternalFunctionId::IoInputNew,
        ExternalFunctionId::IoInputUpdate,
        ExternalFunctionId::IoInputClose,
        ExternalFunctionId::IoInputKeyDown,
        ExternalFunctionId::IoInputKeyPressed,
        ExternalFunctionId::IoInputKeyReleased,
        ExternalFunctionId::IoInputPointerX,
        ExternalFunctionId::IoInputPointerY,
        ExternalFunctionId::IoInputPointerDown,
        ExternalFunctionId::IoInputPointerPressed,
        ExternalFunctionId::IoInputPointerReleased,
        ExternalFunctionId::IoInputLastKeyPressed,
        ExternalFunctionId::IoInputLastKeyReleased,
        ExternalFunctionId::IoInputLastPointerPressed,
        ExternalFunctionId::IoInputLastPointerReleased,
    ] {
        let function = registry
            .get_function_by_id(function_id)
            .expect("core IO function should be registered");
        match function.lowerings.js.as_ref() {
            Some(ExternalJsLowering::RuntimeFunction(helper_name)) => {
                assert_eq!(
                    helper_name,
                    function_id.name(),
                    "{} should lower to the matching JS runtime helper",
                    function_id.name()
                );
            }
            other => panic!(
                "{} should lower to a JS runtime helper, got {other:?}",
                function_id.name()
            ),
        }
        assert!(
            function.lowerings.wasm.is_none(),
            "{} should rely on backend validation for Wasm rejection",
            function_id.name()
        );
    }
}

/// Verifies that the old public `IO` type is no longer registered in `@core/io`.
/// WHAT: the `IO` external opaque type was part of the callable `io(...)` API.
/// WHY: removing it confirms the public type surface is gone.
#[test]
fn core_io_public_type_is_not_registered() {
    let registry = ExternalPackageRegistry::new();
    let io_type = registry.resolve_package_type("@core/io", "IO");
    assert!(
        io_type.is_none(),
        "public IO type should no longer be registered in @core/io"
    );
}

/// Verifies that the prelude registers `io` as a namespace alias to `@core/io`.
/// WHAT: source files can write `io.line(...)` without an explicit dependency clause.
/// WHY: the prelude must expose the lowercase namespace, not a bare function or type.
#[test]
fn prelude_registers_io_namespace_alias_to_core_io() {
    let registry = ExternalPackageRegistry::new();
    let aliases = registry.prelude_namespace_aliases_by_name();
    assert!(
        aliases.contains_key("io"),
        "prelude should register io namespace alias"
    );
    assert_eq!(aliases["io"], "@core/io");
}

// ------------------------------------------------------------------
// Optional external signature tests
// ------------------------------------------------------------------

/// Verifies that an external `String?` signature maps to the diagnostic option datatype.
///
/// WHAT: `ExternalSignatureType::Optional` carries the inner type through the same
///       conversion path as scalar signatures.
/// WHY: host boundaries must describe optional returns without inventing backend sentinels.
#[test]
fn optional_external_resolves_to_option_datatype() {
    let optional_string = ExternalSignatureType::Optional(Box::new(ExternalSignatureType::Abi(
        ExternalAbiType::Utf8Str,
    )));

    assert_eq!(
        optional_string.to_datatype(),
        Some(DataType::Option(Box::new(DataType::StringSlice)))
    );
}

/// Verifies that an external `String?` signature interns the canonical option TypeId.
///
/// WHAT: optional external returns share the same `TypeId` as source-authored `String?`.
/// WHY: pattern matching and type annotations must treat external and source options as
///      the same type.
#[test]
fn optional_external_resolves_to_canonical_option_type_id() {
    let mut type_environment = TypeEnvironment::new();
    let string_type_id = type_environment.builtins().string;
    let ordinary_option = type_environment.intern_option(string_type_id);
    let none_type_id = type_environment.builtins().none;

    let optional_string = ExternalSignatureType::Optional(Box::new(ExternalSignatureType::Abi(
        ExternalAbiType::Utf8Str,
    )));
    let external_option = optional_string
        .to_type_id(&mut type_environment, none_type_id)
        .expect("external String? should resolve to a TypeId");

    assert_eq!(
        external_option, ordinary_option,
        "external String? must use the same TypeId as ordinary String?"
    );
}

/// Verifies that an external `String?` parameter also resolves to the canonical option TypeId.
///
/// WHAT: parameter and return contexts share the same optional signature conversion.
/// WHY: keeps parameter compatibility and return typing consistent at host boundaries.
#[test]
fn optional_external_parameter_resolves_to_canonical_option_type_id() {
    let mut type_environment = TypeEnvironment::new();
    let string_type_id = type_environment.builtins().string;
    let ordinary_option = type_environment.intern_option(string_type_id);

    let optional_string = ExternalSignatureType::Optional(Box::new(ExternalSignatureType::Abi(
        ExternalAbiType::Utf8Str,
    )));
    let parameter_option = optional_string
        .to_parameter_type_id(&mut type_environment)
        .expect("external String? parameter should resolve to a TypeId");

    assert_eq!(
        parameter_option, ordinary_option,
        "external String? parameter must use the same TypeId as ordinary String?"
    );
}

// ------------------------------------------------------------------
// Core IO input metadata tests
// ------------------------------------------------------------------

/// Verifies that `io.input.Input` is registered as a nested external type.
///
/// WHAT: the opaque input handle type lives under the `input` namespace in `@core/io`.
/// WHY: HIR and backends must refer to one stable external type ID, not path text.
#[test]
fn core_io_input_type_is_registered_at_nested_path() {
    let registry = ExternalPackageRegistry::new();
    let input_type = registry
        .resolve_package_type_by_path(
            "@core/io",
            &ExternalSymbolPath::from_components(vec!["input".to_owned(), "Input".to_owned()]),
        )
        .map(|(id, _)| id);

    assert_eq!(input_type, Some(IO_INPUT_EXTERNAL_TYPE_ID));
}

/// Verifies that the input functions are registered under `input.*` nested paths.
///
/// WHAT: each polling helper is reachable through `io.input.<name>` namespace traversal.
/// WHY: stable IDs and nested paths replace any flat or IO-special-cased lookup.
#[test]
fn core_io_input_functions_are_registered_at_nested_paths() {
    let registry = ExternalPackageRegistry::new();

    let cases = [
        ("new", ExternalFunctionId::IoInputNew),
        ("update", ExternalFunctionId::IoInputUpdate),
        ("close", ExternalFunctionId::IoInputClose),
        ("key_down", ExternalFunctionId::IoInputKeyDown),
        ("key_pressed", ExternalFunctionId::IoInputKeyPressed),
        ("key_released", ExternalFunctionId::IoInputKeyReleased),
        ("pointer_x", ExternalFunctionId::IoInputPointerX),
        ("pointer_y", ExternalFunctionId::IoInputPointerY),
        ("pointer_down", ExternalFunctionId::IoInputPointerDown),
        ("pointer_pressed", ExternalFunctionId::IoInputPointerPressed),
        (
            "pointer_released",
            ExternalFunctionId::IoInputPointerReleased,
        ),
        (
            "last_key_pressed",
            ExternalFunctionId::IoInputLastKeyPressed,
        ),
        (
            "last_key_released",
            ExternalFunctionId::IoInputLastKeyReleased,
        ),
        (
            "last_pointer_pressed",
            ExternalFunctionId::IoInputLastPointerPressed,
        ),
        (
            "last_pointer_released",
            ExternalFunctionId::IoInputLastPointerReleased,
        ),
    ];

    for (leaf_name, expected_id) in cases {
        let path =
            ExternalSymbolPath::from_components(vec!["input".to_owned(), leaf_name.to_owned()]);
        let actual_id = registry
            .resolve_package_function_by_path("@core/io", &path)
            .map(|(id, _)| id);
        assert_eq!(
            actual_id,
            Some(expected_id),
            "input.{leaf_name} should resolve to {expected_id:?}"
        );
    }
}

/// Verifies that diagnostics can recover the package-local path from a stable input function ID.
///
/// WHAT: `IoInputNew` is stored in HIR/backend reachability as a stable ID, but diagnostics should
/// render its package path as `input.new`.
/// WHY: nested external package symbols often share leaf names, so backend errors need the full
/// package-local path to stay unambiguous.
#[test]
fn core_io_input_function_id_resolves_to_nested_symbol_path() {
    let registry = ExternalPackageRegistry::new();
    let path = registry
        .resolve_function_symbol_path(ExternalFunctionId::IoInputNew)
        .expect("input.new should have a package-local symbol path");

    assert_eq!(path.display_text(), "input.new");
}

/// Verifies that `input.new` is fallible and returns the input handle type.
///
/// WHAT: creation returns `io.input.Input` on success and `Error!` on failure.
/// WHY: callers must use postfix `!` or `catch`; the function cannot be treated as infallible.
#[test]
fn core_io_input_new_is_fallible_input_return() {
    let registry = ExternalPackageRegistry::new();
    let new_function = registry
        .get_function_by_id(ExternalFunctionId::IoInputNew)
        .expect("input.new should be registered");

    assert!(new_function.parameters.is_empty());
    assert_eq!(new_function.returns.len(), 1);
    assert_eq!(
        new_function.returns[0].value_type,
        ExternalSignatureType::External(IO_INPUT_EXTERNAL_TYPE_ID)
    );
    assert_eq!(
        new_function.error_return_type,
        Some(ExternalSignatureType::BuiltinError)
    );
}

/// Verifies that `input.last_key_pressed` returns `String?` using the canonical option shape.
///
/// WHAT: the `last_*` helpers expose optional string results.
/// WHY: confirms the reusable optional external signature is wired for input metadata.
#[test]
fn core_io_input_last_key_pressed_returns_optional_string() {
    let registry = ExternalPackageRegistry::new();
    let last_key = registry
        .get_function_by_id(ExternalFunctionId::IoInputLastKeyPressed)
        .expect("input.last_key_pressed should be registered");

    assert_eq!(last_key.returns.len(), 1);
    assert_eq!(
        last_key.returns[0].value_type,
        ExternalSignatureType::Optional(Box::new(ExternalSignatureType::Abi(
            ExternalAbiType::Utf8Str,
        )))
    );
    assert!(last_key.error_return_type.is_none());
}

/// Verifies that every package registered through `register_package` always carries
/// `PackageBacking::ExternalBinding` and preserves the supplied origin.
///
/// WHAT: the `ExternalPackageRegistry` API only accepts `PackageOrigin`, constructing
///       `PackageMetadata::binding(origin)` internally so a `MothSource`-backed
///       package is unrepresentable through this API.
/// WHY: protects the boundary between source-backed packages (owned by
///      `SourcePackageRegistry`) and binding-backed packages (owned here).
#[test]
fn register_package_always_produces_binding_backed_metadata() {
    let mut registry = ExternalPackageRegistry::new();

    let id = registry
        .register_package(
            "@test/binding-only",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("test package should register");

    let package = registry
        .get_package_by_id(id)
        .expect("registered package should be retrievable by ID");

    assert_eq!(
        package.metadata.backing,
        crate::builder_surface::package_metadata::PackageBacking::ExternalBinding,
        "register_package must always produce ExternalBinding backing"
    );
    assert_eq!(
        package.metadata.origin,
        crate::builder_surface::PackageOrigin::Builder,
        "register_package must preserve the supplied origin"
    );
}

// ---------------------------------------------------------------------------
//  Reverse lookup: ExternalTypeId -> owned stable package/symbol path
// ---------------------------------------------------------------------------

/// Verifies the O(1) reverse lookup from `ExternalTypeId` to the owned stable package path and
/// structured symbol path for the built-in `io.input.Input` type.
///
/// WHAT: `IO_INPUT_EXTERNAL_TYPE_ID` resolves to `@core/io` and the structured path
/// `input.Input`, never exposing `ExternalPackageId` or `ExternalTypeId` in the returned
/// identity.
/// WHY: canonical binding-backed type identity embeds the owned package path and symbol path,
/// not build-local IDs.
#[test]
fn reverse_lookup_resolves_builtin_external_type_to_owned_identity() {
    let registry = ExternalPackageRegistry::new();

    let (package, symbol_path) = registry
        .resolve_type_package_and_symbol_path(IO_INPUT_EXTERNAL_TYPE_ID)
        .expect("io.input.Input must resolve to its stable identity");

    assert_eq!(package.name(), "@core/io");
    assert_eq!(
        package.origin(),
        crate::builder_surface::PackageOrigin::Core
    );
    assert_eq!(
        symbol_path.display_text(),
        "input.Input",
        "the structured symbol path must preserve namespace components"
    );
}

/// Verifies that a dynamically registered external type populates the reverse lookup through the
/// single registration path.
#[test]
fn reverse_lookup_resolves_dynamically_registered_external_type() {
    let mut registry = ExternalPackageRegistry::default();
    let package_id = registry
        .register_package(
            "@test/canvas",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("test package should register");

    let path = ExternalSymbolPath::from_components(vec!["render".to_owned(), "Context".to_owned()]);
    registry
        .register_type_at_path(
            package_id,
            path,
            ExternalTypeId(42),
            ExternalTypeDef {
                name: "Context".to_owned(),
                package_id,
                abi_type: ExternalAbiType::Handle,
            },
        )
        .expect("test type should register at a structured path");

    let (package, symbol_path) = registry
        .resolve_type_package_and_symbol_path(ExternalTypeId(42))
        .expect("dynamically registered type must resolve through the reverse lookup");

    assert_eq!(package.name(), "@test/canvas");
    assert_eq!(
        package.origin(),
        crate::builder_surface::PackageOrigin::Builder
    );
    assert_eq!(symbol_path.display_text(), "render.Context");
}

/// Verifies that an unregistered `ExternalTypeId` returns `None` from the reverse lookup rather
/// than panicking or inventing an identity.
#[test]
fn reverse_lookup_returns_none_for_unregistered_external_type() {
    let registry = ExternalPackageRegistry::new();
    assert!(
        registry
            .resolve_type_package_and_symbol_path(ExternalTypeId(9999))
            .is_none(),
        "an unregistered ExternalTypeId must not resolve to a stable identity"
    );
}

/// Verifies that cloning the registry preserves the type reverse lookup so a cloned registry
/// remains self-consistent.
#[test]
fn clone_preserves_type_reverse_lookup() {
    let mut registry = ExternalPackageRegistry::default();
    let package_id = registry
        .register_package(
            "@test/clone",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("test package should register");
    registry
        .register_type_in_package(
            package_id,
            ExternalTypeId(7),
            ExternalTypeDef {
                name: "Handle".to_owned(),
                package_id,
                abi_type: ExternalAbiType::Handle,
            },
        )
        .expect("test type should register");

    let cloned = registry.clone();
    let (package, symbol_path) = cloned
        .resolve_type_package_and_symbol_path(ExternalTypeId(7))
        .expect("cloned registry must preserve the type reverse lookup");

    assert_eq!(package.name(), "@test/clone");
    assert_eq!(symbol_path.display_text(), "Handle");
}

/// Verifies that duplicate type registration at the same path remains a `CompilerError`, so the
/// reverse lookup never observes inconsistent state.
#[test]
fn duplicate_type_registration_remains_compiler_error() {
    let mut registry = ExternalPackageRegistry::default();
    let package_id = registry
        .register_package("@test/dup", crate::builder_surface::PackageOrigin::Builder)
        .expect("test package should register");

    let first = registry.register_type_in_package(
        package_id,
        ExternalTypeId(1),
        ExternalTypeDef {
            name: "Thing".to_owned(),
            package_id,
            abi_type: ExternalAbiType::Handle,
        },
    );
    assert!(first.is_ok(), "first registration should succeed");

    let second = registry.register_type_in_package(
        package_id,
        ExternalTypeId(2),
        ExternalTypeDef {
            name: "Thing".to_owned(),
            package_id,
            abi_type: ExternalAbiType::Handle,
        },
    );
    assert!(
        second.is_err(),
        "duplicate type registration at the same path must remain a CompilerError"
    );
}

/// Verifies that reusing one `ExternalTypeId` at a second distinct path is a `CompilerError` and
/// leaves the original reverse identity intact. The duplicate is rejected before any registry
/// state mutates, so the first registration's package path and symbol path survive unchanged.
#[test]
fn duplicate_external_type_id_at_distinct_path_is_rejected_and_preserves_original_identity() {
    let mut registry = ExternalPackageRegistry::default();
    let package_id = registry
        .register_package(
            "@test/reuse",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("test package should register");

    let first_path =
        ExternalSymbolPath::from_components(vec!["shapes".to_owned(), "Origin".to_owned()]);
    registry
        .register_type_at_path(
            package_id,
            first_path,
            ExternalTypeId(11),
            ExternalTypeDef {
                name: "Origin".to_owned(),
                package_id,
                abi_type: ExternalAbiType::Handle,
            },
        )
        .expect("first type registration should succeed");

    // Reuse the same ExternalTypeId at a different path in the same package.
    let second_path =
        ExternalSymbolPath::from_components(vec!["render".to_owned(), "Duplicate".to_owned()]);
    let duplicate = registry.register_type_at_path(
        package_id,
        second_path,
        ExternalTypeId(11),
        ExternalTypeDef {
            name: "Duplicate".to_owned(),
            package_id,
            abi_type: ExternalAbiType::Handle,
        },
    );
    assert!(
        duplicate.is_err(),
        "reusing an ExternalTypeId at a second path must be a CompilerError"
    );

    // The original reverse identity must remain intact after the failed duplicate.
    let (package, symbol_path) = registry
        .resolve_type_package_and_symbol_path(ExternalTypeId(11))
        .expect("the original reverse identity must survive the failed duplicate");
    assert_eq!(package.name(), "@test/reuse");
    assert_eq!(
        symbol_path.display_text(),
        "shapes.Origin",
        "the original symbol path must not be overwritten by the failed duplicate"
    );

    // The second path must not resolve to any registered type identity.
    assert!(
        registry
            .resolve_type_package_and_symbol_path(ExternalTypeId(12))
            .is_none(),
        "no new ExternalTypeId should have been created by the rejected duplicate"
    );
}

/// Verifies that reusing one `ExternalTypeId` across two distinct packages is a `CompilerError`
/// and preserves the first package's reverse identity.
#[test]
fn duplicate_external_type_id_across_distinct_packages_is_rejected() {
    let mut registry = ExternalPackageRegistry::default();
    let alpha_id = registry
        .register_package(
            "@test/alpha",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("alpha package should register");
    let beta_id = registry
        .register_package("@test/beta", crate::builder_surface::PackageOrigin::Builder)
        .expect("beta package should register");

    registry
        .register_type_in_package(
            alpha_id,
            ExternalTypeId(5),
            ExternalTypeDef {
                name: "Handle".to_owned(),
                package_id: alpha_id,
                abi_type: ExternalAbiType::Handle,
            },
        )
        .expect("alpha type registration should succeed");

    let duplicate = registry.register_type_in_package(
        beta_id,
        ExternalTypeId(5),
        ExternalTypeDef {
            name: "Handle".to_owned(),
            package_id: beta_id,
            abi_type: ExternalAbiType::Handle,
        },
    );
    assert!(
        duplicate.is_err(),
        "reusing an ExternalTypeId in a second package must be a CompilerError"
    );

    let (package, symbol_path) = registry
        .resolve_type_package_and_symbol_path(ExternalTypeId(5))
        .expect("the original alpha reverse identity must survive the failed duplicate");
    assert_eq!(package.name(), "@test/alpha");
    assert_eq!(symbol_path.display_text(), "Handle");
}
