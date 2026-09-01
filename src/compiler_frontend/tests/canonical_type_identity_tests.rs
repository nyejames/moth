//! Focused hidden-invariant tests for the canonical closed-type identity vocabulary and
//! projection.
//!
//! WHAT: exercises the structural invariants of `CanonicalTypeIdentity` values and the
//! `TypeId -> CanonicalTypeIdentity` projection that integration output cannot inspect:
//! equality/stability of every closed-type shape, and `CompilerError` for every unsupported or
//! incomplete state.
//! WHY: these are pure value and projection invariants owned by
//! `compiler_frontend::canonical_type_identity`, so they own a focused test beside the module
//! rather than an end-to-end case.

use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastTarget,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalCoreTraitIdentity, CanonicalTraitIdentity,
    CanonicalTypeIdentity, CanonicalTypeProjectionContext, CollectionTypeIdentity,
    ExportedGenericParameterIdentity, ExternalOpaqueTypeIdentity, FallibleCarrierTypeIdentity,
    GenericDeclarationOrigin, GenericInstanceTypeIdentity, GenericParameterOriginResolver,
    NominalOriginResolver, OrderedMapTypeIdentity, project_type_id_to_canonical_identity,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, FieldDefinition, FunctionParameterDefinition,
    FunctionTypeDefinition, StructTypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, GenericParameterId, GenericParameterListId, NominalTypeId,
    TypeConstructor, TypeId,
};
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalPackageRegistry, ExternalSymbolPath, ExternalTypeDef, ExternalTypeId,
    IO_INPUT_EXTERNAL_TYPE_ID,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, OriginFunctionId, OriginTraitId, OriginTypeCategory, OriginTypeId,
    StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use rustc_hash::FxHashMap;
use std::collections::HashSet;

// ---------------------------------------------------------------------------
//  Test fixtures
// ---------------------------------------------------------------------------

/// Map-backed nominal origin resolver for focused tests.
struct MapNominalOriginResolver {
    origins: FxHashMap<NominalTypeId, OriginTypeId>,
}

impl MapNominalOriginResolver {
    fn new() -> Self {
        Self {
            origins: FxHashMap::default(),
        }
    }

    fn register(&mut self, nominal_id: NominalTypeId, origin: OriginTypeId) {
        self.origins.insert(nominal_id, origin);
    }
}

impl NominalOriginResolver for MapNominalOriginResolver {
    fn resolve_nominal_origin(
        &self,
        nominal_id: NominalTypeId,
    ) -> Result<OriginTypeId, CompilerError> {
        self.origins.get(&nominal_id).cloned().ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "no source-nominal origin registered for NominalTypeId({})",
                nominal_id.0
            ))
        })
    }
}

/// Map-backed generic-parameter origin resolver for focused tests.
struct MapGenericParameterOriginResolver {
    origins: FxHashMap<GenericParameterId, ExportedGenericParameterIdentity>,
}

impl MapGenericParameterOriginResolver {
    fn new() -> Self {
        Self {
            origins: FxHashMap::default(),
        }
    }

    fn register(
        &mut self,
        parameter_id: GenericParameterId,
        identity: ExportedGenericParameterIdentity,
    ) {
        self.origins.insert(parameter_id, identity);
    }
}

impl GenericParameterOriginResolver for MapGenericParameterOriginResolver {
    fn resolve_generic_parameter_origin(
        &self,
        parameter_id: GenericParameterId,
    ) -> Result<ExportedGenericParameterIdentity, CompilerError> {
        self.origins.get(&parameter_id).cloned().ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "no exported generic-parameter identity registered for GenericParameterId({})",
                parameter_id.0
            ))
        })
    }
}

fn module_origin(logical_path: &str) -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        logical_path.to_owned(),
        ModuleRootRole::Normal,
    )
}

fn struct_origin(name: &str) -> OriginTypeId {
    OriginTypeId::new(
        module_origin("shapes"),
        name.to_owned(),
        OriginTypeCategory::Struct,
    )
}

fn choice_origin(name: &str) -> OriginTypeId {
    OriginTypeId::new(
        module_origin("shapes"),
        name.to_owned(),
        OriginTypeCategory::Choice,
    )
}

fn free_function_origin(name: &str) -> OriginFunctionId {
    OriginFunctionId::new_free(module_origin("functions"), name.to_owned())
}

/// Constructs an `ExportedGenericParameterIdentity` for a struct-owned parameter at position 0
/// named `T`.
fn struct_parameter_identity(
    struct_name: &str,
    position: u32,
    param_name: &str,
) -> ExportedGenericParameterIdentity {
    ExportedGenericParameterIdentity::new(
        GenericDeclarationOrigin::nominal_type(struct_origin(struct_name))
            .expect("struct origin must be a valid generic declaration owner"),
        position,
        param_name.to_owned(),
    )
}

fn empty_fields() -> Box<[FieldDefinition]> {
    Box::new([])
}

fn no_variants() -> Box<[ChoiceVariantDefinition]> {
    Box::new([])
}

/// Registers a struct in the environment and returns its `NominalTypeId` and `TypeId`.
fn register_struct(
    env: &mut TypeEnvironment,
    string_table: &mut StringTable,
    name: &str,
) -> (NominalTypeId, TypeId) {
    let path = InternedPath::from_single_str(name, string_table);
    env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path,
        fields: empty_fields(),
        generic_parameters: None,
        const_record: false,
    })
}

/// Registers a struct with a single generic parameter and returns its `NominalTypeId` and
/// `TypeId`.
fn register_generic_struct(
    env: &mut TypeEnvironment,
    string_table: &mut StringTable,
    name: &str,
    param_list_id: GenericParameterListId,
) -> (NominalTypeId, TypeId) {
    let path = InternedPath::from_single_str(name, string_table);
    env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path,
        fields: empty_fields(),
        generic_parameters: Some(param_list_id),
        const_record: false,
    })
}

fn register_choice(
    env: &mut TypeEnvironment,
    string_table: &mut StringTable,
    name: &str,
) -> (NominalTypeId, TypeId) {
    let path = InternedPath::from_single_str(name, string_table);
    env.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path,
        variants: no_variants(),
        generic_parameters: None,
    })
}

/// Registers a single generic parameter list with one parameter named `T`.
fn register_single_param_list(
    env: &mut TypeEnvironment,
    string_table: &mut StringTable,
) -> GenericParameterListId {
    use crate::compiler_frontend::datatypes::generic_parameters::{
        GenericParameter, GenericParameterList, TypeParameterId,
    };
    let list = GenericParameterList {
        parameters: vec![GenericParameter {
            id: TypeParameterId(0),
            name: string_table.intern("T"),
            location: SourceLocation::default(),
            trait_bounds: Vec::new(),
        }],
    };
    env.register_generic_parameter_list(&list, &FxHashMap::default())
        .list_id
}

/// Builds a test registry with one registered external type at `@test/canvas.Canvas`.
fn test_registry_with_canvas_type() -> ExternalPackageRegistry {
    let mut registry = ExternalPackageRegistry::default();
    let package_id = registry
        .register_package(
            "@test/canvas",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("test package should register");
    registry
        .register_type_in_package(
            package_id,
            ExternalTypeId(100),
            ExternalTypeDef {
                name: "Canvas".to_owned(),
                package_id,
                abi_type: ExternalAbiType::Handle,
            },
        )
        .expect("test type should register");
    registry
}

/// Builds a test registry using the built-in `@core/io` package, which already registers
/// `io.input.Input` at `IO_INPUT_EXTERNAL_TYPE_ID`.
fn builtin_registry() -> ExternalPackageRegistry {
    ExternalPackageRegistry::new()
}

fn projection_context<'a>(
    resolver: &'a MapNominalOriginResolver,
    generic_resolver: &'a MapGenericParameterOriginResolver,
    registry: &'a ExternalPackageRegistry,
) -> CanonicalTypeProjectionContext<'a> {
    CanonicalTypeProjectionContext::new(resolver, generic_resolver, registry)
}

// ---------------------------------------------------------------------------
//  Builtin projection
// ---------------------------------------------------------------------------

#[test]
fn projects_every_builtin_scalar() {
    let env = TypeEnvironment::new();
    let resolver = MapNominalOriginResolver::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let cases = [
        (env.builtins().bool, CanonicalBuiltinType::Bool),
        (env.builtins().int, CanonicalBuiltinType::Int),
        (env.builtins().float, CanonicalBuiltinType::Float),
        (env.builtins().decimal, CanonicalBuiltinType::Decimal),
        (env.builtins().string, CanonicalBuiltinType::String),
        (env.builtins().char, CanonicalBuiltinType::Char),
        (env.builtins().range, CanonicalBuiltinType::Range),
        (env.builtins().none, CanonicalBuiltinType::None),
    ];

    for (type_id, expected_builtin) in cases {
        let identity = project_type_id_to_canonical_identity(type_id, &env, &context)
            .expect("builtin scalar projection should succeed");
        assert_eq!(
            identity,
            CanonicalTypeIdentity::Builtin(expected_builtin),
            "builtin TypeId({}) should project to {:?}",
            type_id.0,
            expected_builtin
        );
    }
}

#[test]
fn builtin_none_identity_is_canonical() {
    let env = TypeEnvironment::new();
    let resolver = MapNominalOriginResolver::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let none_identity = project_type_id_to_canonical_identity(env.builtins().none, &env, &context)
        .expect("None builtin should project");

    assert_eq!(
        none_identity,
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::None),
        "the semantically seeded None identity must be a canonical builtin"
    );
}

#[test]
fn builtin_error_struct_projects_to_builtin_identity() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let (_, error_type_id) = register_struct(&mut env, &mut string_table, "Error");
    let resolver = MapNominalOriginResolver::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    env.register_canonical_identity(
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error),
        error_type_id,
    )
    .expect("builtin Error identity should register");
    let context = CanonicalTypeProjectionContext::new(&resolver, &generic_resolver, &registry);

    let identity = project_type_id_to_canonical_identity(error_type_id, &env, &context)
        .expect("builtin Error projection should succeed without a source origin");

    assert_eq!(
        identity,
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error)
    );
}

#[test]
fn durable_canonical_interner_preserves_exact_origin_and_reuses_type_id() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let (_, normal_type_id) = register_struct(&mut env, &mut string_table, "NormalCard");
    let (_, support_type_id) = register_struct(&mut env, &mut string_table, "SupportCard");
    let package = StablePackageIdentity::project_local("test-project");
    let normal_identity = CanonicalTypeIdentity::SourceNominal(OriginTypeId::new(
        StableModuleOriginIdentity::from_portable_path(
            package.clone(),
            "cards".to_owned(),
            ModuleRootRole::Normal,
        ),
        "Card".to_owned(),
        OriginTypeCategory::Struct,
    ));
    let support_identity = CanonicalTypeIdentity::SourceNominal(OriginTypeId::new(
        StableModuleOriginIdentity::from_portable_path(
            package,
            "cards".to_owned(),
            ModuleRootRole::Support,
        ),
        "Card".to_owned(),
        OriginTypeCategory::Struct,
    ));

    env.register_canonical_identity(normal_identity.clone(), normal_type_id)
        .expect("normal identity should register");
    env.register_canonical_identity(normal_identity.clone(), normal_type_id)
        .expect("repeated identity registration should reuse its TypeId");
    env.register_canonical_identity(support_identity.clone(), support_type_id)
        .expect("role-distinct identity should register independently");

    assert_eq!(
        env.type_id_for_canonical_identity(&normal_identity),
        Some(normal_type_id)
    );
    assert_eq!(
        env.type_id_for_canonical_identity(&support_identity),
        Some(support_type_id)
    );
    assert_eq!(
        env.canonical_identity_for_type_id(normal_type_id),
        Some(&normal_identity)
    );
}

// ---------------------------------------------------------------------------
//  Source nominal projection
// ---------------------------------------------------------------------------

#[test]
fn projects_direct_struct_to_source_nominal_origin() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let (nominal_id, type_id) = register_struct(&mut env, &mut string_table, "Button");

    let mut resolver = MapNominalOriginResolver::new();
    resolver.register(nominal_id, struct_origin("Button"));
    let registry = ExternalPackageRegistry::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let identity = project_type_id_to_canonical_identity(type_id, &env, &context)
        .expect("struct projection should succeed");

    assert_eq!(
        identity,
        CanonicalTypeIdentity::SourceNominal(struct_origin("Button")),
        "direct struct must project to its stable source-nominal origin"
    );
}

#[test]
fn projects_direct_choice_to_source_nominal_origin() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let (nominal_id, type_id) = register_choice(&mut env, &mut string_table, "Status");

    let mut resolver = MapNominalOriginResolver::new();
    resolver.register(nominal_id, choice_origin("Status"));
    let registry = ExternalPackageRegistry::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let identity = project_type_id_to_canonical_identity(type_id, &env, &context)
        .expect("choice projection should succeed");

    assert_eq!(
        identity,
        CanonicalTypeIdentity::SourceNominal(choice_origin("Status")),
        "direct choice must project to its stable source-nominal origin"
    );
}

#[test]
fn struct_and_choice_with_same_origin_category_are_distinct() {
    let struct_origin_id = struct_origin("Shape");
    let choice_origin_id = choice_origin("Shape");
    assert_ne!(
        struct_origin_id, choice_origin_id,
        "struct and choice origins with the same name must differ by category"
    );
}

// ---------------------------------------------------------------------------
//  External opaque projection
// ---------------------------------------------------------------------------

#[test]
fn projects_external_opaque_to_owned_package_and_symbol_path() {
    let mut env = TypeEnvironment::new();
    let registry = builtin_registry();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let external_type_id = IO_INPUT_EXTERNAL_TYPE_ID;
    let type_id = env.intern_external(external_type_id);

    let resolver = MapNominalOriginResolver::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let identity = project_type_id_to_canonical_identity(type_id, &env, &context)
        .expect("external opaque projection should succeed");

    let expected = CanonicalTypeIdentity::ExternalOpaque(ExternalOpaqueTypeIdentity::new(
        StablePackageIdentity::binding(crate::builder_surface::PackageOrigin::Core, "@core/io"),
        ExternalSymbolPath::from_components(vec!["input".to_owned(), "Input".to_owned()]),
    ));
    assert_eq!(
        identity, expected,
        "external opaque projection must retain the owned package and structured symbol path"
    );
}

#[test]
fn external_opaque_identity_is_equal_for_equal_package_and_symbol() {
    let a = ExternalOpaqueTypeIdentity::new(
        StablePackageIdentity::binding(crate::builder_surface::PackageOrigin::Core, "@core/io"),
        ExternalSymbolPath::from_components(vec!["input".to_owned(), "Input".to_owned()]),
    );
    let b = ExternalOpaqueTypeIdentity::new(
        StablePackageIdentity::binding(crate::builder_surface::PackageOrigin::Core, "@core/io"),
        ExternalSymbolPath::from_components(vec!["input".to_owned(), "Input".to_owned()]),
    );
    assert_eq!(
        a, b,
        "equal package path and symbol path must yield equal identity"
    );

    let mut set = HashSet::new();
    set.insert(a.clone());
    assert!(
        set.contains(&b),
        "equal identity must hash to the same slot"
    );
}

#[test]
fn external_opaque_identity_distinguishes_different_packages() {
    let a = ExternalOpaqueTypeIdentity::new(
        StablePackageIdentity::binding(crate::builder_surface::PackageOrigin::Core, "@core/io"),
        ExternalSymbolPath::from_single("Input"),
    );
    let b = ExternalOpaqueTypeIdentity::new(
        StablePackageIdentity::binding(
            crate::builder_surface::PackageOrigin::Builder,
            "@test/canvas",
        ),
        ExternalSymbolPath::from_single("Input"),
    );
    assert_ne!(a, b, "same symbol path in different packages must differ");
}

#[test]
fn external_opaque_identity_distinguishes_same_package_path_from_different_origins() {
    let symbol_path = ExternalSymbolPath::from_single("Handle");
    let builder = ExternalOpaqueTypeIdentity::new(
        StablePackageIdentity::binding(
            crate::builder_surface::PackageOrigin::Builder,
            "@shared/api",
        ),
        symbol_path.clone(),
    );
    let project = ExternalOpaqueTypeIdentity::new(
        StablePackageIdentity::binding(
            crate::builder_surface::PackageOrigin::ProjectLocal,
            "@shared/api",
        ),
        symbol_path,
    );

    assert_ne!(
        builder, project,
        "same-spelling packages from different origins must not share opaque type identity"
    );
}

#[test]
fn projects_external_opaque_from_test_registry() {
    let mut env = TypeEnvironment::new();
    let registry = test_registry_with_canvas_type();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let type_id = env.intern_external(ExternalTypeId(100));

    let resolver = MapNominalOriginResolver::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let identity = project_type_id_to_canonical_identity(type_id, &env, &context)
        .expect("test external opaque projection should succeed");

    let expected = CanonicalTypeIdentity::ExternalOpaque(ExternalOpaqueTypeIdentity::new(
        StablePackageIdentity::binding(
            crate::builder_surface::PackageOrigin::Builder,
            "@test/canvas",
        ),
        ExternalSymbolPath::from_single("Canvas"),
    ));
    assert_eq!(identity, expected);
}

// ---------------------------------------------------------------------------
//  Option, collection, map and fallible carrier projection
// ---------------------------------------------------------------------------

#[test]
fn projects_option_of_builtin() {
    let mut env = TypeEnvironment::new();
    let int_id = env.builtins().int;
    let option_id = env.intern_option(int_id);

    let resolver = MapNominalOriginResolver::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let identity = project_type_id_to_canonical_identity(option_id, &env, &context)
        .expect("option projection should succeed");

    assert_eq!(
        identity,
        CanonicalTypeIdentity::Option(Box::new(CanonicalTypeIdentity::Builtin(
            CanonicalBuiltinType::Int
        ))),
        "Option<Int> must project recursively"
    );
}

#[test]
fn projects_growable_collection() {
    let mut env = TypeEnvironment::new();
    let string_id = env.builtins().string;
    let collection_id = env.intern_collection(string_id, None);

    let resolver = MapNominalOriginResolver::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let identity = project_type_id_to_canonical_identity(collection_id, &env, &context)
        .expect("growable collection projection should succeed");

    assert_eq!(
        identity,
        CanonicalTypeIdentity::Collection(CollectionTypeIdentity::new(
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
            None
        )),
        "growable {{String}} must project with fixed_capacity = None"
    );
}

#[test]
fn projects_fixed_collection_distinct_from_growable() {
    let mut env = TypeEnvironment::new();
    let int_id = env.builtins().int;
    let fixed_id = env.intern_collection(int_id, Some(4));
    let growable_id = env.intern_collection(int_id, None);

    let resolver = MapNominalOriginResolver::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let fixed_identity = project_type_id_to_canonical_identity(fixed_id, &env, &context)
        .expect("fixed collection projection should succeed");
    let growable_identity = project_type_id_to_canonical_identity(growable_id, &env, &context)
        .expect("growable collection projection should succeed");

    assert_ne!(
        fixed_identity, growable_identity,
        "fixed and growable collections of the same element must be distinct"
    );

    let expected_fixed = CanonicalTypeIdentity::Collection(CollectionTypeIdentity::new(
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
        Some(4),
    ));
    assert_eq!(fixed_identity, expected_fixed);
}

#[test]
fn projects_ordered_map_preserving_key_value_order() {
    let mut env = TypeEnvironment::new();
    let key_id = env.builtins().string;
    let value_id = env.builtins().int;
    let map_id = env.intern_map(key_id, value_id);

    let resolver = MapNominalOriginResolver::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let identity = project_type_id_to_canonical_identity(map_id, &env, &context)
        .expect("map projection should succeed");

    let expected = CanonicalTypeIdentity::OrderedMap(OrderedMapTypeIdentity::new(
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
    ));
    assert_eq!(
        identity, expected,
        "{{String = Int}} must preserve key/value order"
    );

    // Swapping key and value must produce a distinct identity.
    let swapped_map_id = env.intern_map(value_id, key_id);
    let swapped_identity = project_type_id_to_canonical_identity(swapped_map_id, &env, &context)
        .expect("swapped map projection should succeed");
    assert_ne!(
        identity, swapped_identity,
        "swapping map key and value must change identity"
    );
}

#[test]
fn projects_fallible_carrier_preserving_success_error_order() {
    let mut env = TypeEnvironment::new();
    let success_id = env.builtins().int;
    let error_id = env.builtins().string;
    let carrier_id = env.intern_fallible_carrier(success_id, error_id);

    let resolver = MapNominalOriginResolver::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let identity = project_type_id_to_canonical_identity(carrier_id, &env, &context)
        .expect("fallible carrier projection should succeed");

    let expected = CanonicalTypeIdentity::FallibleCarrier(FallibleCarrierTypeIdentity::new(
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
    ));
    assert_eq!(
        identity, expected,
        "Int!String must preserve success/error order"
    );

    // Swapping success and error must produce a distinct identity.
    let swapped_carrier_id = env.intern_fallible_carrier(error_id, success_id);
    let swapped_identity =
        project_type_id_to_canonical_identity(swapped_carrier_id, &env, &context)
            .expect("swapped carrier projection should succeed");
    assert_ne!(
        identity, swapped_identity,
        "swapping success and error channels must change identity"
    );
}

// ---------------------------------------------------------------------------
//  Concrete generic nominal instance projection
// ---------------------------------------------------------------------------

#[test]
fn projects_concrete_generic_nominal_instance() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let param_list_id = register_single_param_list(&mut env, &mut string_table);
    let (box_nominal_id, _) =
        register_generic_struct(&mut env, &mut string_table, "Box", param_list_id);

    let int_id = env.builtins().int;
    let instance_id = env.intern_generic_instance(box_nominal_id, Box::new([int_id]));

    let mut resolver = MapNominalOriginResolver::new();
    resolver.register(box_nominal_id, struct_origin("Box"));
    let registry = ExternalPackageRegistry::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let identity = project_type_id_to_canonical_identity(instance_id, &env, &context)
        .expect("generic instance projection should succeed");

    let expected = CanonicalTypeIdentity::GenericInstance(GenericInstanceTypeIdentity::new(
        struct_origin("Box"),
        Box::new([CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int)]),
    ));
    assert_eq!(
        identity, expected,
        "Box<Int> must project to base origin plus canonical concrete arguments"
    );
}

#[test]
fn generic_instance_identity_is_equal_for_equal_base_and_arguments() {
    let base = struct_origin("Box");
    let args = Box::new([CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int)]);
    let a = GenericInstanceTypeIdentity::new(base.clone(), args.clone());
    let b = GenericInstanceTypeIdentity::new(base, args);
    assert_eq!(a, b, "equal base and arguments must yield equal identity");

    let mut set = HashSet::new();
    set.insert(a.clone());
    assert!(
        set.contains(&b),
        "equal identity must hash to the same slot"
    );
}

#[test]
fn generic_instance_distinguishes_different_concrete_arguments() {
    let base = struct_origin("Box");
    let int_args = Box::new([CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int)]);
    let string_args = Box::new([CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String)]);
    let int_instance = GenericInstanceTypeIdentity::new(base.clone(), int_args);
    let string_instance = GenericInstanceTypeIdentity::new(base, string_args);
    assert_ne!(
        int_instance, string_instance,
        "different concrete arguments must yield distinct identities"
    );
}

// ---------------------------------------------------------------------------
//  CompilerError states
// ---------------------------------------------------------------------------

#[test]
fn absent_nominal_origin_returns_compiler_error() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let (nominal_id, type_id) = register_struct(&mut env, &mut string_table, "Missing");

    let resolver = MapNominalOriginResolver::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let error = project_type_id_to_canonical_identity(type_id, &env, &context)
        .expect_err("absent nominal origin must return an error");

    assert_eq!(
        error.error_type,
        ErrorType::Compiler,
        "absent nominal origin must use the compiler-error lane"
    );
    assert!(
        error.msg.contains("source-nominal origin") && error.msg.contains("struct"),
        "error should name the struct nominal origin: {}",
        error.msg
    );
    // The NominalTypeId is part of the context but not embedded in the canonical identity.
    let _ = nominal_id;
}

#[test]
fn absent_external_identity_returns_compiler_error() {
    let mut env = TypeEnvironment::new();
    let unregistered_external_id = ExternalTypeId(999);
    let type_id = env.intern_external(unregistered_external_id);

    let resolver = MapNominalOriginResolver::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let error = project_type_id_to_canonical_identity(type_id, &env, &context)
        .expect_err("absent external identity must return an error");

    assert_eq!(
        error.error_type,
        ErrorType::Compiler,
        "absent external identity must use the compiler-error lane"
    );
    assert!(
        error.msg.contains("ExternalTypeId") && error.msg.contains("999"),
        "error should name the unregistered ExternalTypeId: {}",
        error.msg
    );
}

#[test]
fn unresolved_generic_parameter_returns_compiler_error() {
    let mut env = TypeEnvironment::new();
    let param_type_id =
        env.intern_generic_parameter(GenericParameterId(0), StringTable::new().intern("T"));

    let resolver = MapNominalOriginResolver::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let error = project_type_id_to_canonical_identity(param_type_id, &env, &context)
        .expect_err("generic parameter must return an error");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains("exported generic-parameter")
            && error.msg.contains("GenericParameterId"),
        "error should name the missing exported generic-parameter identity: {}",
        error.msg
    );
}

#[test]
fn function_type_returns_compiler_error() {
    let mut env = TypeEnvironment::new();
    let int_id = env.builtins().int;
    let function_id = env.insert_function_type_for_test(FunctionTypeDefinition {
        parameters: Box::new([FunctionParameterDefinition {
            name: None,
            type_id: int_id,
        }]),
        returns: Box::new([int_id]),
        error_return: None,
    });

    let resolver = MapNominalOriginResolver::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let error = project_type_id_to_canonical_identity(function_id, &env, &context)
        .expect_err("function type must return an error");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains("function"),
        "error should name the function type state: {}",
        error.msg
    );
}

#[test]
fn tuple_type_returns_compiler_error() {
    let mut env = TypeEnvironment::new();
    let int_id = env.builtins().int;
    let tuple_id = env.intern_tuple(vec![int_id, int_id]);

    let resolver = MapNominalOriginResolver::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let error = project_type_id_to_canonical_identity(tuple_id, &env, &context)
        .expect_err("tuple type must return an error");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains("tuple"),
        "error should name the tuple state: {}",
        error.msg
    );
}

#[test]
fn malformed_collection_arity_returns_compiler_error() {
    let mut env = TypeEnvironment::new();
    // Directly intern a collection with zero arguments, which is a malformed arity.
    let malformed_id = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: None,
        }),
        Box::new([]),
    );

    let resolver = MapNominalOriginResolver::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let error = project_type_id_to_canonical_identity(malformed_id, &env, &context)
        .expect_err("malformed collection arity must return an error");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains("collection") && error.msg.contains("arity"),
        "error should name the collection arity: {}",
        error.msg
    );
}

#[test]
fn malformed_map_arity_returns_compiler_error() {
    let mut env = TypeEnvironment::new();
    let int_id = env.builtins().int;
    // Directly intern an ordered map with one argument, which is a malformed arity.
    let malformed_id = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::OrderedMap),
        Box::new([int_id]),
    );

    let resolver = MapNominalOriginResolver::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let error = project_type_id_to_canonical_identity(malformed_id, &env, &context)
        .expect_err("malformed map arity must return an error");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains("ordered map") && error.msg.contains("arity"),
        "error should name the map arity: {}",
        error.msg
    );
}

#[test]
fn malformed_generic_instance_arity_returns_compiler_error() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let param_list_id = register_single_param_list(&mut env, &mut string_table);
    let (box_nominal_id, _) =
        register_generic_struct(&mut env, &mut string_table, "Box", param_list_id);

    // Box declares 1 generic parameter but we intern an instance with 0 arguments.
    let malformed_id = env.intern_generic_instance(box_nominal_id, Box::new([]));

    let mut resolver = MapNominalOriginResolver::new();
    resolver.register(box_nominal_id, struct_origin("Box"));
    let registry = ExternalPackageRegistry::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let error = project_type_id_to_canonical_identity(malformed_id, &env, &context)
        .expect_err("malformed generic-instance arity must return an error");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains("malformed generic-instance arity"),
        "error should name the generic-instance arity: {}",
        error.msg
    );
}

/// Verifies that a zero-argument `GenericInstance` built from a non-generic nominal is rejected.
///
/// The previous silent `0` fallback let `Thing[]` project as a legal concrete instance of a
/// nominal that does not actually declare generic parameters. The projection must reject it.
#[test]
fn generic_instance_of_non_generic_base_returns_compiler_error() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let (plain_nominal_id, _) = register_struct(&mut env, &mut string_table, "Thing");

    // Thing declares no generic parameters; a zero-argument instance is not a legal instance.
    let instance_id = env.intern_generic_instance(plain_nominal_id, Box::new([]));

    let mut resolver = MapNominalOriginResolver::new();
    resolver.register(plain_nominal_id, struct_origin("Thing"));
    let registry = ExternalPackageRegistry::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let error = project_type_id_to_canonical_identity(instance_id, &env, &context)
        .expect_err("a generic instance of a non-generic base must return an error");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains("generic parameter list is absent"),
        "error should name the absent generic parameter list: {}",
        error.msg
    );
}

/// Verifies that a `GenericInstance` whose base nominal is unknown to the `TypeEnvironment` is
/// rejected even when the origin resolver claims an origin for it.
#[test]
fn generic_instance_of_unknown_base_returns_compiler_error() {
    let mut env = TypeEnvironment::new();
    // NominalTypeId(999) is never registered in the TypeEnvironment.
    let unknown_base = NominalTypeId(999);
    let instance_id = env.intern_generic_instance(unknown_base, Box::new([]));

    let mut resolver = MapNominalOriginResolver::new();
    // The origin resolver knows an origin for the unknown base, so origin resolution succeeds
    // and the base validation is what catches the missing nominal.
    resolver.register(unknown_base, struct_origin("Phantom"));
    let registry = ExternalPackageRegistry::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let error = project_type_id_to_canonical_identity(instance_id, &env, &context)
        .expect_err("a generic instance of an unknown base must return an error");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error
            .msg
            .contains("neither a registered struct nor a choice"),
        "error should name the unknown nominal base: {}",
        error.msg
    );
}

/// Verifies that a `GenericInstance` whose base declares a generic parameter list that is
/// missing from the `TypeEnvironment` is rejected. The base is registered with a dangling
/// `GenericParameterListId` through the normal registration API, so no production escape hatch
/// is needed to construct this inconsistent state.
#[test]
fn generic_instance_with_missing_parameter_list_returns_compiler_error() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    // Register a struct that claims a parameter list that was never registered.
    let (dangling_nominal_id, _) = register_generic_struct(
        &mut env,
        &mut string_table,
        "Dangling",
        GenericParameterListId(777),
    );

    let instance_id = env.intern_generic_instance(dangling_nominal_id, Box::new([]));

    let mut resolver = MapNominalOriginResolver::new();
    resolver.register(dangling_nominal_id, struct_origin("Dangling"));
    let registry = ExternalPackageRegistry::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let error = project_type_id_to_canonical_identity(instance_id, &env, &context)
        .expect_err("a generic instance whose parameter list is missing must return an error");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains("missing from the TypeEnvironment"),
        "error should name the missing parameter list: {}",
        error.msg
    );
}

// ---------------------------------------------------------------------------
//  Equality and stability across independent construction
// ---------------------------------------------------------------------------

#[test]
fn equal_construction_yields_equal_and_hash_equal_identity() {
    let a = CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int);
    let b = CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int);

    assert_eq!(a, b, "equal construction must yield equal identity");
    let mut set = HashSet::new();
    set.insert(a);
    assert!(
        set.contains(&b),
        "equal identity must hash to the same slot"
    );
}

#[test]
fn distinct_builtins_are_distinct_identities() {
    let int = CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int);
    let string = CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String);
    assert_ne!(int, string, "distinct builtins must be distinct");
}

#[test]
fn canonical_identity_carries_no_local_ids_or_paths() {
    // The Debug output of any canonical identity must not leak donor-local IDs, interned paths,
    // string IDs or external type IDs. This is a structural invariant: the enum and its
    // supporting structs only carry owned stable values.
    let opaque = ExternalOpaqueTypeIdentity::new(
        StablePackageIdentity::binding(crate::builder_surface::PackageOrigin::Core, "@core/io"),
        ExternalSymbolPath::from_components(vec!["input".to_owned(), "Input".to_owned()]),
    );
    let identity = CanonicalTypeIdentity::ExternalOpaque(opaque);
    let debug = format!("{identity:?}");

    assert!(
        !debug.contains("ExternalTypeId") && !debug.contains("ExternalPackageId"),
        "canonical identity must not embed build-local external IDs: {debug}"
    );
    assert!(
        !debug.contains("NominalTypeId") && !debug.contains("TypeId("),
        "canonical identity must not embed donor-local TypeId or NominalTypeId: {debug}"
    );
    assert!(
        !debug.contains("InternedPath") && !debug.contains("StringId"),
        "canonical identity must not embed interned paths or string IDs: {debug}"
    );
}

#[test]
fn recursive_generic_instance_arguments_are_canonical() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let param_list_id = register_single_param_list(&mut env, &mut string_table);
    let (box_nominal_id, _) =
        register_generic_struct(&mut env, &mut string_table, "Box", param_list_id);

    // Box<Option<Int>>
    let int_id = env.builtins().int;
    let option_id = env.intern_option(int_id);
    let nested_instance_id = env.intern_generic_instance(box_nominal_id, Box::new([option_id]));

    let mut resolver = MapNominalOriginResolver::new();
    resolver.register(box_nominal_id, struct_origin("Box"));
    let registry = ExternalPackageRegistry::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let identity = project_type_id_to_canonical_identity(nested_instance_id, &env, &context)
        .expect("nested generic instance projection should succeed");

    let expected = CanonicalTypeIdentity::GenericInstance(GenericInstanceTypeIdentity::new(
        struct_origin("Box"),
        Box::new([CanonicalTypeIdentity::Option(Box::new(
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
        ))]),
    ));
    assert_eq!(
        identity, expected,
        "Box<Option<Int>> must recursively project inner option to canonical identity"
    );
}

// ---------------------------------------------------------------------------
//  Exported generic-parameter identity and open-type projection
// ---------------------------------------------------------------------------

#[test]
fn exported_generic_parameter_identity_is_equal_across_distinct_generic_parameter_ids() {
    // Two environments allocate different GenericParameterId values for the same logical
    // parameter. The stable identity must be equal because it derives from the declaration
    // origin, position and authored name, not from the donor-local GenericParameterId.
    let mut env_a = TypeEnvironment::new();
    let mut env_b = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    // Consume GenericParameterId(0) in env_a with a synthetic parameter so the real list's
    // parameter gets GenericParameterId(1).
    let _ = env_a.register_synthetic_generic_parameter(string_table.intern("dummy"));

    use crate::compiler_frontend::datatypes::generic_parameters::{
        GenericParameter, GenericParameterList, TypeParameterId,
    };

    let list = GenericParameterList {
        parameters: vec![GenericParameter {
            id: TypeParameterId(0),
            name: string_table.intern("T"),
            location: SourceLocation::default(),
            trait_bounds: Vec::new(),
        }],
    };

    let registered_a = env_a.register_generic_parameter_list(&list, &FxHashMap::default());
    let registered_b = env_b.register_generic_parameter_list(&list, &FxHashMap::default());

    let param_id_a = registered_a.canonical_by_local[&TypeParameterId(0)];
    let param_id_b = registered_b.canonical_by_local[&TypeParameterId(0)];

    assert_ne!(
        param_id_a, param_id_b,
        "the two environments must allocate distinct GenericParameterId values"
    );

    let type_id_a = env_a
        .type_id_for_generic_parameter(param_id_a)
        .expect("env_a must have a TypeId for the parameter");
    let type_id_b = env_b
        .type_id_for_generic_parameter(param_id_b)
        .expect("env_b must have a TypeId for the parameter");

    let identity = struct_parameter_identity("Box", 0, "T");

    let mut generic_resolver_a = MapGenericParameterOriginResolver::new();
    generic_resolver_a.register(param_id_a, identity.clone());
    let mut generic_resolver_b = MapGenericParameterOriginResolver::new();
    generic_resolver_b.register(param_id_b, identity.clone());

    let resolver = MapNominalOriginResolver::new();
    let registry = ExternalPackageRegistry::new();

    let context_a = projection_context(&resolver, &generic_resolver_a, &registry);
    let context_b = projection_context(&resolver, &generic_resolver_b, &registry);

    let projected_a = project_type_id_to_canonical_identity(type_id_a, &env_a, &context_a)
        .expect("env_a generic parameter projection should succeed");
    let projected_b = project_type_id_to_canonical_identity(type_id_b, &env_b, &context_b)
        .expect("env_b generic parameter projection should succeed");

    assert_eq!(
        projected_a, projected_b,
        "same owner, position and name must yield equal identity across distinct GenericParameterId allocations"
    );

    let mut set = HashSet::new();
    set.insert(projected_a);
    assert!(
        set.contains(&projected_b),
        "equal identity must hash to the same slot"
    );
}

#[test]
fn exported_generic_parameter_identity_distinguishes_owner_position_and_name() {
    let struct_box = struct_parameter_identity("Box", 0, "T");
    let struct_container = struct_parameter_identity("Container", 0, "T");
    let struct_box_pos1 = struct_parameter_identity("Box", 1, "T");
    let struct_box_name_u = struct_parameter_identity("Box", 0, "U");

    // Different declaration owner.
    assert_ne!(
        struct_box, struct_container,
        "different declaration owners must produce distinct identities"
    );

    // Different parameter position.
    assert_ne!(
        struct_box, struct_box_pos1,
        "different parameter positions must produce distinct identities"
    );

    // Different authored name.
    assert_ne!(
        struct_box, struct_box_name_u,
        "different authored names must produce distinct identities"
    );
}

#[test]
fn exported_generic_parameter_identity_distinguishes_free_function_from_nominal() {
    let nominal_origin = GenericDeclarationOrigin::nominal_type(struct_origin("Box")).unwrap();
    let function_origin = GenericDeclarationOrigin::free_function(free_function_origin("map"))
        .expect("a free function must be a valid generic declaration origin");

    let nominal_identity = ExportedGenericParameterIdentity::new(nominal_origin, 0, "T".to_owned());
    let function_identity =
        ExportedGenericParameterIdentity::new(function_origin, 0, "T".to_owned());

    assert_ne!(
        nominal_identity, function_identity,
        "a free function and a nominal type with the same position and name must be distinct"
    );
}

#[test]
fn transparent_alias_cannot_be_a_generic_declaration_origin() {
    let alias_origin = OriginTypeId::new(
        module_origin("shapes"),
        "Alias".to_owned(),
        OriginTypeCategory::TransparentAlias,
    );

    let error = GenericDeclarationOrigin::nominal_type(alias_origin)
        .expect_err("a transparent alias must not be a valid generic declaration origin");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains("transparent alias"),
        "error should reject transparent alias as a generic declaration owner: {}",
        error.msg
    );
}

#[test]
fn receiver_method_cannot_be_a_generic_declaration_origin() {
    // The receiver type is a normal struct origin, but the function kind is Receiver.
    let receiver_type = struct_origin("Widget");
    let receiver_method = OriginFunctionId::new_receiver(
        module_origin("functions"),
        "process".to_owned(),
        receiver_type,
    );

    let error = GenericDeclarationOrigin::free_function(receiver_method)
        .expect_err("a receiver method must not be a valid generic declaration origin");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains("receiver method"),
        "error should reject the receiver-method function kind, not the receiver type: {}",
        error.msg
    );
}

#[test]
fn projects_option_of_exported_generic_parameter() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    let param_list_id = register_single_param_list(&mut env, &mut string_table);
    let (box_nominal_id, _) =
        register_generic_struct(&mut env, &mut string_table, "Box", param_list_id);

    // Get the GenericParameterId and TypeId for the single parameter "T" before mutating env.
    let (param_id, param_type_id) = {
        let list = env
            .generic_parameters(param_list_id)
            .expect("parameter list must exist");
        let param = &list.parameters[0];
        let type_id = env
            .type_id_for_generic_parameter(param.id)
            .expect("TypeId must exist for the generic parameter");
        (param.id, type_id)
    };

    // Option<T> — a nested constructed open shape around an exported parameter.
    let option_id = env.intern_option(param_type_id);

    let identity = struct_parameter_identity("Box", 0, "T");

    let mut generic_resolver = MapGenericParameterOriginResolver::new();
    generic_resolver.register(param_id, identity.clone());

    let mut resolver = MapNominalOriginResolver::new();
    resolver.register(box_nominal_id, struct_origin("Box"));
    let registry = ExternalPackageRegistry::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let projected = project_type_id_to_canonical_identity(option_id, &env, &context)
        .expect("Option<T> projection should succeed");

    let expected =
        CanonicalTypeIdentity::Option(Box::new(CanonicalTypeIdentity::GenericParameter(identity)));
    assert_eq!(
        projected, expected,
        "Option<T> must recursively project the inner exported generic parameter"
    );
}

#[test]
fn projects_collection_of_exported_generic_parameter() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    let param_list_id = register_single_param_list(&mut env, &mut string_table);
    let (_container_nominal_id, _) =
        register_generic_struct(&mut env, &mut string_table, "Container", param_list_id);

    let (param_id, param_type_id) = {
        let list = env
            .generic_parameters(param_list_id)
            .expect("parameter list must exist");
        let param = &list.parameters[0];
        let type_id = env
            .type_id_for_generic_parameter(param.id)
            .expect("TypeId must exist for the generic parameter");
        (param.id, type_id)
    };

    // {T} — a growable collection around an exported parameter.
    let collection_id = env.intern_collection(param_type_id, None);

    let identity = struct_parameter_identity("Container", 0, "T");

    let mut generic_resolver = MapGenericParameterOriginResolver::new();
    generic_resolver.register(param_id, identity.clone());

    let resolver = MapNominalOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let projected = project_type_id_to_canonical_identity(collection_id, &env, &context)
        .expect("collection of generic parameter projection should succeed");

    let expected = CanonicalTypeIdentity::Collection(CollectionTypeIdentity::new(
        CanonicalTypeIdentity::GenericParameter(identity),
        None,
    ));
    assert_eq!(
        projected, expected,
        "{{T}} must recursively project the inner exported generic parameter"
    );
}

#[test]
fn missing_generic_parameter_resolver_entry_returns_compiler_error() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    let param_list_id = register_single_param_list(&mut env, &mut string_table);
    let (_box_nominal_id, _) =
        register_generic_struct(&mut env, &mut string_table, "Box", param_list_id);

    let param_type_id = {
        let list = env
            .generic_parameters(param_list_id)
            .expect("parameter list must exist");
        let param = &list.parameters[0];
        env.type_id_for_generic_parameter(param.id)
            .expect("TypeId must exist for the generic parameter")
    };

    // No entry in the generic-parameter resolver.
    let resolver = MapNominalOriginResolver::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let error = project_type_id_to_canonical_identity(param_type_id, &env, &context)
        .expect_err("a missing resolver entry must return an error");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains("exported generic-parameter")
            && error.msg.contains("GenericParameterId"),
        "error should name the missing exported generic-parameter identity: {}",
        error.msg
    );
}

#[test]
fn synthetic_generic_parameter_rejected_by_absence_from_resolver() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    // A synthetic parameter (e.g. trait `This`) is not an exported generic declaration parameter.
    let synthetic_type_id = env.register_synthetic_generic_parameter(string_table.intern("This"));

    let resolver = MapNominalOriginResolver::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let error = project_type_id_to_canonical_identity(synthetic_type_id, &env, &context)
        .expect_err("a synthetic parameter absent from the resolver must be rejected");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains("exported generic-parameter"),
        "error should reject the synthetic parameter through the resolver: {}",
        error.msg
    );
}

#[test]
fn generic_parameter_identity_carries_no_local_ids() {
    let identity = struct_parameter_identity("Box", 0, "T");
    let canonical = CanonicalTypeIdentity::GenericParameter(identity);
    let debug = format!("{canonical:?}");

    // Check for actual ID values (e.g. `GenericParameterId(0)`) rather than the struct name
    // `ExportedGenericParameterIdentity`, which naturally contains the substring.
    assert!(
        !debug.contains("GenericParameterId(") && !debug.contains("GenericParameterListId("),
        "canonical generic-parameter identity must not embed donor-local parameter IDs: {debug}"
    );
    assert!(
        !debug.contains("TypeId(") && !debug.contains("StringId("),
        "canonical generic-parameter identity must not embed TypeId or StringId: {debug}"
    );
    assert!(
        !debug.contains("InternedPath"),
        "canonical generic-parameter identity must not embed interned paths: {debug}"
    );
}

// ---------------------------------------------------------------------------
//  Canonical trait identity
// ---------------------------------------------------------------------------

fn trait_origin(name: &str) -> OriginTraitId {
    OriginTraitId::new(module_origin("traits"), name.to_owned())
}

#[test]
fn source_trait_identity_is_equal_for_equal_origin() {
    let identity_a = CanonicalTraitIdentity::Source(trait_origin("DISPLAYABLE"));
    let identity_b = CanonicalTraitIdentity::Source(trait_origin("DISPLAYABLE"));

    assert_eq!(identity_a, identity_b);
    let mut set = HashSet::new();
    set.insert(identity_a.clone());
    assert!(set.contains(&identity_b));
}

#[test]
fn source_trait_identity_distinguishes_different_module_origins() {
    let identity_a = CanonicalTraitIdentity::Source(OriginTraitId::new(
        module_origin("module_a"),
        "MyTrait".to_owned(),
    ));
    let identity_b = CanonicalTraitIdentity::Source(OriginTraitId::new(
        module_origin("module_b"),
        "MyTrait".to_owned(),
    ));

    assert_ne!(identity_a, identity_b);
}

#[test]
fn source_trait_identity_distinguishes_different_defining_names() {
    let identity_a = CanonicalTraitIdentity::Source(trait_origin("TraitA"));
    let identity_b = CanonicalTraitIdentity::Source(trait_origin("TraitB"));

    assert_ne!(identity_a, identity_b);
}

#[test]
fn displayable_core_trait_identity_is_equal_to_itself() {
    let identity_a = CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Displayable);
    let identity_b = CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Displayable);

    assert_eq!(identity_a, identity_b);
    let mut set = HashSet::new();
    set.insert(identity_a);
    assert!(set.contains(&identity_b));
}

#[test]
fn infallible_cast_core_trait_identity_preserves_target_and_fallibility() {
    let identity = CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Castable {
        target: BuiltinCastTarget::Int,
        fallibility: BuiltinCastFallibility::Infallible,
    });

    let debug = format!("{identity:?}");
    assert!(
        debug.contains("Int") && debug.contains("Infallible"),
        "infallible cast core trait identity must carry target and fallibility: {debug}"
    );
}

#[test]
fn fallible_cast_core_trait_identity_preserves_target_and_fallibility() {
    let identity = CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Castable {
        target: BuiltinCastTarget::Int,
        fallibility: BuiltinCastFallibility::Fallible,
    });

    let debug = format!("{identity:?}");
    assert!(
        debug.contains("Int") && debug.contains("Fallible"),
        "fallible cast core trait identity must carry target and fallibility: {debug}"
    );
}

#[test]
fn cast_core_trait_distinguishes_infallible_from_fallible() {
    let infallible = CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Castable {
        target: BuiltinCastTarget::Int,
        fallibility: BuiltinCastFallibility::Infallible,
    });
    let fallible = CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Castable {
        target: BuiltinCastTarget::Int,
        fallibility: BuiltinCastFallibility::Fallible,
    });

    assert_ne!(infallible, fallible);
}

#[test]
fn cast_core_trait_distinguishes_different_targets() {
    let to_int = CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Castable {
        target: BuiltinCastTarget::Int,
        fallibility: BuiltinCastFallibility::Infallible,
    });
    let to_string = CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Castable {
        target: BuiltinCastTarget::String,
        fallibility: BuiltinCastFallibility::Infallible,
    });

    assert_ne!(to_int, to_string);
}

#[test]
fn source_trait_identity_is_distinct_from_displayable_core() {
    let source = CanonicalTraitIdentity::Source(trait_origin("DISPLAYABLE"));
    let core = CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Displayable);

    assert_ne!(source, core);
}

#[test]
fn canonical_trait_identity_carries_no_local_ids_or_paths() {
    let source = CanonicalTraitIdentity::Source(trait_origin("MyTrait"));
    let core = CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Castable {
        target: BuiltinCastTarget::Bool,
        fallibility: BuiltinCastFallibility::Fallible,
    });

    for identity in [source, core] {
        let debug = format!("{identity:?}");
        assert!(
            !debug.contains("TraitId(")
                && !debug.contains("StringId(")
                && !debug.contains("FileId("),
            "canonical trait identity must not embed local IDs: {debug}"
        );
        assert!(
            !debug.contains("InternedPath"),
            "canonical trait identity must not embed interned paths: {debug}"
        );
        assert!(
            !debug.contains("CoreTraitKind"),
            "canonical trait identity must not embed a CoreTraitKind registry handle: {debug}"
        );
    }
}

// ---------------------------------------------------------------------------
//  Anonymous const-record marker projection
// ---------------------------------------------------------------------------

#[test]
fn projects_anonymous_const_record_marker_to_payload_free_identity() {
    let resolver = MapNominalOriginResolver::new();
    let generic_resolver = MapGenericParameterOriginResolver::new();
    let registry = ExternalPackageRegistry::new();
    let env = TypeEnvironment::new();
    let context = projection_context(&resolver, &generic_resolver, &registry);

    let marker = env.anonymous_const_record_type();
    let identity = project_type_id_to_canonical_identity(marker, &env, &context)
        .expect("the compile-time marker is a legal closed type");

    // The identity is the payload-free variant and equal across independent projections.
    assert_eq!(identity, CanonicalTypeIdentity::AnonymousConstRecord);
    let again = project_type_id_to_canonical_identity(marker, &env, &context)
        .expect("projection is deterministic");
    assert_eq!(identity, again);

    // The payload-free variant is a leaf: it visits exactly itself.
    let mut visited = Vec::new();
    identity.visit(&mut |visited_identity| visited.push(visited_identity.clone()));
    assert_eq!(visited, vec![CanonicalTypeIdentity::AnonymousConstRecord]);
}
