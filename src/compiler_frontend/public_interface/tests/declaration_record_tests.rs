//! Focused hidden-invariant tests for the per-binding declaration-record projection.
//!
//! WHAT: exercises the final [`PublicDeclarationRecord`] semantics produced by
//! [`PublicInterfaceDraftBuilder`] that integration output cannot inspect: ordered stable
//! generic identities and ordered source/core trait bounds on generic free functions,
//! stability of generic identity across donor-local allocation differences, nested
//! option/collection and imported public nominal references projecting to canonical
//! identities, the `CompilerError` totality failures for missing nominal origin, missing
//! signature `TypeId`, category mismatch and unregistered generic parameters, and the
//! receiver-method join invariants: same-named methods on distinct receivers join by exact
//! stable origin, and duplicate receiver seed, path and category mismatches are rejected.
//! WHY: these are projection invariants owned by `compiler_frontend::public_interface`, so
//! they own a focused test beside the module rather than an end-to-end case.

use super::super::{
    DirectExportSeed, PublicDeclarationRecord, PublicInterfaceDraftBuilder,
    PublicInterfaceDraftBuilderInput, build_callable_seed_table,
};
use super::test_support::{path, receiver_entry, register_struct};

use crate::compiler_frontend::ast::AstPublicInterfaceProjectionInput;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ReactiveSource, ReactiveSourceKind,
};
use crate::compiler_frontend::ast::statements::functions::{
    FunctionReturn, FunctionSignature, ReturnChannel, ReturnSlot,
};
use crate::compiler_frontend::ast::{
    ReceiverMethodEntry, ResolvedPublicTypeRoot, ResolvedPublicTypeRootKind,
    ResolvedPublicTypeRootTable,
};
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastTarget,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalCoreTraitIdentity, CanonicalTraitIdentity,
    CanonicalTypeIdentity, ExportedGenericParameterIdentity, GenericDeclarationOrigin,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, FieldDefinition,
    StructTypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_parameters::{
    GenericParameter, GenericParameterList, TypeParameterId,
};
use crate::compiler_frontend::datatypes::ids::{GenericParameterListId, NominalTypeId, TypeId};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::public_call_summary::PublicCallParameterAccess;
use crate::compiler_frontend::public_interface::{
    CallableSeed, CallableSeedKind, PublicDeclarationSemantics, PublicFunctionCategory,
    PublicGenericParameterSurface, PublicStructSemantics,
};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, ModuleRootRole, OriginConstantId, OriginDeclarationId, OriginFunctionId,
    OriginTraitId, OriginTypeCategory, OriginTypeId, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::traits::ids::TraitId;
use crate::compiler_frontend::value_mode::ValueMode;

use rustc_hash::FxHashMap;
use std::rc::Rc;

// ---------------------------------------------------------------------------
//  Fixtures
// ---------------------------------------------------------------------------

fn module_origin(logical_path: &str) -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        logical_path.to_owned(),
        ModuleRootRole::Normal,
    )
}

fn active_module_origin() -> StableModuleOriginIdentity {
    module_origin("shapes")
}

fn struct_origin(name: &str) -> OriginTypeId {
    OriginTypeId::new(
        active_module_origin(),
        name.to_owned(),
        OriginTypeCategory::Struct,
    )
}

fn imported_struct_origin(name: &str) -> OriginTypeId {
    OriginTypeId::new(
        module_origin("imports"),
        name.to_owned(),
        OriginTypeCategory::Struct,
    )
}

fn choice_origin(name: &str) -> OriginTypeId {
    OriginTypeId::new(
        active_module_origin(),
        name.to_owned(),
        OriginTypeCategory::Choice,
    )
}

fn free_function_origin(name: &str) -> OriginFunctionId {
    OriginFunctionId::new_free(module_origin("functions"), name.to_owned())
}

fn constant_origin(name: &str) -> OriginConstantId {
    OriginConstantId::new(module_origin("constants"), name.to_owned())
}

fn trait_origin(name: &str) -> OriginTraitId {
    OriginTraitId::new(module_origin("traits"), name.to_owned())
}

fn location() -> SourceLocation {
    SourceLocation::default()
}

fn empty_fields() -> Box<[FieldDefinition]> {
    Box::new([])
}

fn param_declaration(name: &str, type_id: TypeId, string_table: &mut StringTable) -> Declaration {
    Declaration {
        id: path(name, string_table),
        value: Expression::no_value_with_type_id(
            location(),
            DataType::Inferred,
            type_id,
            ValueMode::default(),
        ),
    }
}

fn mutable_param_declaration(
    name: &str,
    type_id: TypeId,
    string_table: &mut StringTable,
) -> Declaration {
    let mut declaration = param_declaration(name, type_id, string_table);
    declaration.value.value_mode = ValueMode::MutableReference;
    declaration
}

fn reactive_param_declaration(
    name: &str,
    type_id: TypeId,
    string_table: &mut StringTable,
) -> Declaration {
    let mut declaration = param_declaration(name, type_id, string_table);
    declaration.value.reactive_source = Some(ReactiveSource {
        path: declaration.id.clone(),
        kind: ReactiveSourceKind::Parameter,
    });
    declaration
}

fn field_declaration(name: &str, type_id: TypeId, string_table: &mut StringTable) -> Declaration {
    Declaration {
        id: path(name, string_table),
        value: Expression::no_value_with_type_id(
            location(),
            DataType::Inferred,
            type_id,
            ValueMode::ImmutableOwned,
        ),
    }
}

fn field_def(name: &str, type_id: TypeId, string_table: &mut StringTable) -> FieldDefinition {
    FieldDefinition {
        name: path(name, string_table),
        type_id,
        location: location(),
    }
}

fn return_slot(type_id: TypeId, channel: ReturnChannel) -> ReturnSlot {
    ReturnSlot {
        value: FunctionReturn::Value(DataType::Inferred),
        type_id: Some(type_id),
        reactive_template: None,
        channel,
    }
}

fn unresolved_return_slot(channel: ReturnChannel) -> ReturnSlot {
    ReturnSlot {
        value: FunctionReturn::Value(DataType::Inferred),
        type_id: None,
        reactive_template: None,
        channel,
    }
}

fn free_function_signature(
    parameters: Vec<Declaration>,
    return_type_ids: Vec<TypeId>,
) -> FunctionSignature {
    let returns = return_type_ids
        .into_iter()
        .map(|type_id| return_slot(type_id, ReturnChannel::Success))
        .collect();
    FunctionSignature {
        parameters,
        returns,
    }
}

fn function_root(
    name: &str,
    signature: FunctionSignature,
    generic_parameter_list_id: Option<GenericParameterListId>,
    string_table: &mut StringTable,
) -> ResolvedPublicTypeRoot {
    ResolvedPublicTypeRoot {
        path: path(name, string_table),
        kind: ResolvedPublicTypeRootKind::Function {
            signature,
            generic_parameter_list_id,
        },
    }
}

fn struct_root(
    name: &str,
    type_id: TypeId,
    fields: Vec<Declaration>,
    string_table: &mut StringTable,
) -> ResolvedPublicTypeRoot {
    ResolvedPublicTypeRoot {
        path: path(name, string_table),
        kind: ResolvedPublicTypeRootKind::Struct { type_id, fields },
    }
}

fn choice_root(
    name: &str,
    type_id: TypeId,
    string_table: &mut StringTable,
) -> ResolvedPublicTypeRoot {
    ResolvedPublicTypeRoot {
        path: path(name, string_table),
        kind: ResolvedPublicTypeRootKind::Choice { type_id },
    }
}

fn constant_root(
    name: &str,
    type_id: TypeId,
    string_table: &mut StringTable,
) -> ResolvedPublicTypeRoot {
    ResolvedPublicTypeRoot {
        path: path(name, string_table),
        kind: ResolvedPublicTypeRootKind::Constant { type_id },
    }
}

fn export_binding(name: &str, origin: OriginDeclarationId) -> ExportBinding {
    ExportBinding::new(origin.module_origin().clone(), name.to_owned(), origin)
}

fn nominal_origins_map(
    entries: Vec<(&str, OriginTypeId)>,
    string_table: &mut StringTable,
) -> FxHashMap<InternedPath, OriginTypeId> {
    let mut map = FxHashMap::default();
    for (name, origin) in entries {
        map.insert(path(name, string_table), origin);
    }
    map
}

fn register_choice(
    env: &mut TypeEnvironment,
    string_table: &mut StringTable,
    name: &str,
    variants: Box<[ChoiceVariantDefinition]>,
    generic_parameters: Option<GenericParameterListId>,
) -> (NominalTypeId, TypeId) {
    let path = InternedPath::from_single_str(name, string_table);
    env.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path,
        variants,
        generic_parameters,
    })
}

fn register_struct_at_path(
    env: &mut TypeEnvironment,
    path: InternedPath,
    fields: Box<[FieldDefinition]>,
    generic_parameters: Option<GenericParameterListId>,
) -> (NominalTypeId, TypeId) {
    env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path,
        fields,
        generic_parameters,
        const_record: false,
    })
}

fn unit_variant(name: &str, string_table: &mut StringTable) -> ChoiceVariantDefinition {
    ChoiceVariantDefinition {
        name: string_table.intern(name),
        tag: 0,
        payload: ChoiceVariantPayloadDefinition::Unit,
        location: location(),
    }
}

fn record_variant(
    name: &str,
    fields: Box<[FieldDefinition]>,
    string_table: &mut StringTable,
) -> ChoiceVariantDefinition {
    ChoiceVariantDefinition {
        name: string_table.intern(name),
        tag: 0,
        payload: ChoiceVariantPayloadDefinition::Record { fields },
        location: location(),
    }
}

fn register_param_list(
    env: &mut TypeEnvironment,
    string_table: &mut StringTable,
    param_names: &[&str],
) -> GenericParameterListId {
    let parameters = param_names
        .iter()
        .enumerate()
        .map(|(position, name)| GenericParameter {
            id: TypeParameterId(position as u32),
            name: string_table.intern(name),
            location: location(),
            trait_bounds: Vec::new(),
        })
        .collect();
    let list = GenericParameterList { parameters };
    env.register_generic_parameter_list(&list, &FxHashMap::default())
        .list_id
}

fn register_single_param_list(
    env: &mut TypeEnvironment,
    string_table: &mut StringTable,
    param_name: &str,
) -> GenericParameterListId {
    register_param_list(env, string_table, &[param_name])
}

fn register_param_list_with_bounds(
    env: &mut TypeEnvironment,
    string_table: &mut StringTable,
    param_name: &str,
    bound_trait_ids: Vec<TraitId>,
) -> GenericParameterListId {
    let parameters = vec![GenericParameter {
        id: TypeParameterId(0),
        name: string_table.intern(param_name),
        location: location(),
        trait_bounds: Vec::new(),
    }];
    let list = GenericParameterList { parameters };
    let mut bounds_by_local: FxHashMap<TypeParameterId, Vec<TraitId>> = FxHashMap::default();
    bounds_by_local.insert(TypeParameterId(0), bound_trait_ids);
    env.register_generic_parameter_list(&list, &bounds_by_local)
        .list_id
}

fn root_table(
    roots: Vec<ResolvedPublicTypeRoot>,
    receiver_methods: Vec<ReceiverMethodEntry>,
    trait_source_facts: FxHashMap<TraitId, crate::compiler_frontend::ast::ResolvedTraitSourceFact>,
) -> ResolvedPublicTypeRootTable {
    ResolvedPublicTypeRootTable {
        roots,
        receiver_methods,
        trait_source_facts,
    }
}

/// Run the draft builder over the given roots and bindings, returning the projected
/// declaration records. This is the test entry point for the per-binding declaration-record
/// projection: the builder produces one `PublicDeclarationRecord` per stable origin.
struct DraftRefs<'a> {
    nominal_origins: &'a FxHashMap<InternedPath, OriginTypeId>,
    trait_origins: &'a FxHashMap<InternedPath, OriginTraitId>,
    env: &'a TypeEnvironment,
    string_table: &'a StringTable,
}

fn build_draft(
    roots: Vec<ResolvedPublicTypeRoot>,
    receiver_methods: Vec<ReceiverMethodEntry>,
    bindings: Vec<ExportBinding>,
    trait_source_facts: FxHashMap<TraitId, crate::compiler_frontend::ast::ResolvedTraitSourceFact>,
    refs: &DraftRefs<'_>,
) -> Result<Vec<PublicDeclarationRecord>, CompilerError> {
    build_draft_with_constants(
        roots,
        receiver_methods,
        bindings,
        trait_source_facts,
        &[],
        refs,
    )
}

fn build_draft_with_constants(
    roots: Vec<ResolvedPublicTypeRoot>,
    receiver_methods: Vec<ReceiverMethodEntry>,
    bindings: Vec<ExportBinding>,
    trait_source_facts: FxHashMap<TraitId, crate::compiler_frontend::ast::ResolvedTraitSourceFact>,
    module_constants: &[Declaration],
    refs: &DraftRefs<'_>,
) -> Result<Vec<PublicDeclarationRecord>, CompilerError> {
    let table = root_table(roots, receiver_methods, trait_source_facts);
    let export_seed = DirectExportSeed::new(active_module_origin(), bindings, FxHashMap::default());
    let projection_input = AstPublicInterfaceProjectionInput {
        root_table: table,
        trait_roots: Vec::new(),
        trait_environment: Some(Rc::new(TraitEnvironment::new())),
        trait_evidence_environment: Some(Rc::new(TraitEvidenceEnvironment::new())),
        const_templates_by_name: FxHashMap::default(),
    };
    let registry = ExternalPackageRegistry::new();
    PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: refs.nominal_origins,
        public_source_trait_origins: refs.trait_origins,
        type_environment: refs.env,
        external_registry: &registry,
        string_table: refs.string_table,
        generic_function_templates: &FxHashMap::default(),
        module_constants,
    })
    .build()
    .map(|result| result.draft.declarations)
}

fn field_declaration_with_default(
    name: &str,
    type_id: TypeId,
    default: Expression,
    string_table: &mut StringTable,
) -> Declaration {
    let mut value = default;
    value.type_id = type_id;
    Declaration {
        id: path(name, string_table),
        value,
    }
}

fn project_struct_record(
    env: &mut TypeEnvironment,
    string_table: &mut StringTable,
    field_definitions: Box<[FieldDefinition]>,
    retained_fields: Vec<Declaration>,
) -> Result<Vec<PublicDeclarationRecord>, CompilerError> {
    let struct_path = path("Widget", string_table);
    let (_, type_id) = register_struct_at_path(env, struct_path, field_definitions, None);
    let root = struct_root("Widget", type_id, retained_fields, string_table);
    let nominal_origins =
        nominal_origins_map(vec![("Widget", struct_origin("Widget"))], string_table);

    build_draft(
        vec![root],
        Vec::new(),
        vec![export_binding(
            "Widget",
            OriginDeclarationId::Type(struct_origin("Widget")),
        )],
        FxHashMap::default(),
        &DraftRefs {
            nominal_origins: &nominal_origins,
            trait_origins: &FxHashMap::default(),
            env,
            string_table,
        },
    )
}

/// Create a minimal export binding + root for one free function so the builder has a module
/// origin. Returns the binding and the matching free-function root.
fn free_fn_binding_and_root(
    name: &str,
    string_table: &mut StringTable,
) -> (ExportBinding, ResolvedPublicTypeRoot) {
    let root = ResolvedPublicTypeRoot {
        path: path(name, string_table),
        kind: ResolvedPublicTypeRootKind::Function {
            signature: FunctionSignature::default(),
            generic_parameter_list_id: None,
        },
    };
    let binding = export_binding(
        name,
        OriginDeclarationId::Function(OriginFunctionId::new_free(
            module_origin("functions"),
            name.to_owned(),
        )),
    );
    (binding, root)
}

// ---------------------------------------------------------------------------
//  Generic free-function ordered identities and ordered bounds
// ---------------------------------------------------------------------------

#[test]
fn generic_free_function_exposes_ordered_generic_parameter_identities() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let param_list_id = register_param_list(&mut env, &mut string_table, &["Key", "Value"]);

    let key_type_id = env
        .type_id_for_generic_parameter(
            env.generic_parameters(param_list_id).unwrap().parameters[0].id,
        )
        .expect("first generic parameter must have a TypeId");
    let value_type_id = env
        .type_id_for_generic_parameter(
            env.generic_parameters(param_list_id).unwrap().parameters[1].id,
        )
        .expect("second generic parameter must have a TypeId");

    let key_param = param_declaration("key", key_type_id, &mut string_table);
    let value_param = param_declaration("value", value_type_id, &mut string_table);
    let signature = free_function_signature(vec![key_param, value_param], vec![value_type_id]);
    let root = function_root("pair", signature, Some(param_list_id), &mut string_table);

    let binding = export_binding(
        "pair",
        OriginDeclarationId::Function(free_function_origin("pair")),
    );

    let declarations = build_draft(
        vec![root],
        Vec::new(),
        vec![binding],
        FxHashMap::default(),
        &DraftRefs {
            nominal_origins: &FxHashMap::default(),
            trait_origins: &FxHashMap::default(),
            env: &env,
            string_table: &string_table,
        },
    )
    .expect("generic function projection should succeed");

    let function = declaration_function(&declarations, "pair");
    let expected_origin = GenericDeclarationOrigin::free_function(free_function_origin("pair"))
        .expect("free function must be a valid generic declaration owner");
    let expected_first =
        ExportedGenericParameterIdentity::new(expected_origin.clone(), 0, "Key".to_owned());
    let expected_second =
        ExportedGenericParameterIdentity::new(expected_origin.clone(), 1, "Value".to_owned());

    assert_eq!(
        generic_identity_list(function),
        &[&expected_first, &expected_second],
        "the generic free function must expose its parameters in declaration-local order"
    );
}

#[test]
fn generic_choice_exposes_ordered_generic_parameter_identities() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let param_list_id = register_param_list(&mut env, &mut string_table, &["T", "U"]);

    let first_type_id = env
        .type_id_for_generic_parameter(
            env.generic_parameters(param_list_id).unwrap().parameters[0].id,
        )
        .expect("first generic parameter must have a TypeId");
    let second_type_id = env
        .type_id_for_generic_parameter(
            env.generic_parameters(param_list_id).unwrap().parameters[1].id,
        )
        .expect("second generic parameter must have a TypeId");

    let variant_fields = Box::new([
        field_def("first", first_type_id, &mut string_table),
        field_def("second", second_type_id, &mut string_table),
    ]);
    let variant = record_variant("Pair", variant_fields, &mut string_table);
    let (_nominal_id, type_id) = register_choice(
        &mut env,
        &mut string_table,
        "Result",
        Box::new([variant]),
        Some(param_list_id),
    );

    let root = choice_root("Result", type_id, &mut string_table);
    let binding = export_binding("Result", OriginDeclarationId::Type(choice_origin("Result")));
    let nominal_map =
        nominal_origins_map(vec![("Result", choice_origin("Result"))], &mut string_table);

    let declarations = build_draft(
        vec![root],
        Vec::new(),
        vec![binding],
        FxHashMap::default(),
        &DraftRefs {
            nominal_origins: &nominal_map,
            trait_origins: &FxHashMap::default(),
            env: &env,
            string_table: &string_table,
        },
    )
    .expect("generic choice projection should succeed");

    let choice = declaration_choice(&declarations, "Result");
    let expected_origin = GenericDeclarationOrigin::nominal_type(choice_origin("Result"))
        .expect("choice origin must be a valid generic declaration owner");
    let expected_first =
        ExportedGenericParameterIdentity::new(expected_origin.clone(), 0, "T".to_owned());
    let expected_second =
        ExportedGenericParameterIdentity::new(expected_origin.clone(), 1, "U".to_owned());

    assert_eq!(
        choice
            .generic_parameters
            .iter()
            .map(|surface| &surface.identity)
            .collect::<Vec<_>>(),
        &[&expected_first, &expected_second],
        "the generic choice must expose its parameters in declaration-local order"
    );
}

#[test]
fn generic_parameter_identities_are_stable_across_donor_local_allocation() {
    // Two independent TypeEnvironments register the same single-parameter generic list, but one
    // environment first registers a throwaway generic parameter list through the ordinary
    // registration owner so its target parameter is allocated from a higher donor-local counter.
    // The donor-local GenericParameterId allocations must differ, yet the projected
    // ExportedGenericParameterIdentity must be identical because it derives from the stable
    // declaration origin and declaration-local position, not the donor-local id.
    let function_name = "identity";
    let make_draft = |perturb: bool| {
        let mut env = TypeEnvironment::new();
        let mut string_table = StringTable::new();
        if perturb {
            let _ = register_single_param_list(&mut env, &mut string_table, "Perturb");
        }
        let param_list_id = register_single_param_list(&mut env, &mut string_table, "T");
        let target_local_id = env
            .generic_parameters(param_list_id)
            .expect("target generic parameter list must resolve")
            .parameters[0]
            .id;
        let generic_type_id = env
            .type_id_for_generic_parameter(target_local_id)
            .expect("generic parameter must have a TypeId");

        let param = param_declaration("value", generic_type_id, &mut string_table);
        let signature = free_function_signature(vec![param], vec![generic_type_id]);
        let root = function_root(
            function_name,
            signature,
            Some(param_list_id),
            &mut string_table,
        );

        let binding = export_binding(
            function_name,
            OriginDeclarationId::Function(free_function_origin(function_name)),
        );

        let declarations = build_draft(
            vec![root],
            Vec::new(),
            vec![binding],
            FxHashMap::default(),
            &DraftRefs {
                nominal_origins: &FxHashMap::default(),
                trait_origins: &FxHashMap::default(),
                env: &env,
                string_table: &string_table,
            },
        )
        .expect("projection should succeed");
        (declarations, target_local_id)
    };

    let (declarations_a, local_id_a) = make_draft(false);
    let (declarations_b, local_id_b) = make_draft(true);

    assert_ne!(
        local_id_a, local_id_b,
        "the two environments must allocate different donor-local GenericParameterIds so the \
         stability premise is real"
    );
    assert_eq!(
        generic_identity_list(declaration_function(&declarations_a, function_name)),
        generic_identity_list(declaration_function(&declarations_b, function_name)),
        "exported generic parameter identities must be stable across donor-local \
         GenericParameterId allocation"
    );
}

#[test]
fn generic_parameter_with_no_bounds_projects_empty_bound_list() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let param_list_id = register_single_param_list(&mut env, &mut string_table, "T");
    let generic_type_id = env
        .type_id_for_generic_parameter(
            env.generic_parameters(param_list_id).unwrap().parameters[0].id,
        )
        .unwrap();

    let param = param_declaration("value", generic_type_id, &mut string_table);
    let signature = free_function_signature(vec![param], vec![generic_type_id]);
    let root = function_root(
        "identity",
        signature,
        Some(param_list_id),
        &mut string_table,
    );

    let binding = export_binding(
        "identity",
        OriginDeclarationId::Function(free_function_origin("identity")),
    );

    let declarations = build_draft(
        vec![root],
        Vec::new(),
        vec![binding],
        FxHashMap::default(),
        &DraftRefs {
            nominal_origins: &FxHashMap::default(),
            trait_origins: &FxHashMap::default(),
            env: &env,
            string_table: &string_table,
        },
    )
    .expect("parameter with no bounds should project");

    let function = declaration_function(&declarations, "identity");
    assert!(
        generic_parameters(function)[0].bounds.is_empty(),
        "a parameter with no bounds must project an empty bound list"
    );
}

#[test]
fn generic_parameter_with_source_trait_bound_projects_canonical_source_identity() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    let source_trait_id = TraitId(0);
    let param_list_id =
        register_param_list_with_bounds(&mut env, &mut string_table, "T", vec![source_trait_id]);

    let generic_type_id = env
        .type_id_for_generic_parameter(
            env.generic_parameters(param_list_id).unwrap().parameters[0].id,
        )
        .unwrap();

    let param = param_declaration("value", generic_type_id, &mut string_table);
    let signature = free_function_signature(vec![param], vec![generic_type_id]);
    let root = function_root("render", signature, Some(param_list_id), &mut string_table);

    let trait_path = path("RENDERABLE", &mut string_table);
    let mut trait_source_facts = FxHashMap::default();
    trait_source_facts.insert(
        source_trait_id,
        crate::compiler_frontend::ast::ResolvedTraitSourceFact::Source(trait_path.clone()),
    );

    let mut trait_origins = FxHashMap::default();
    trait_origins.insert(trait_path, trait_origin("RENDERABLE"));

    let binding = export_binding(
        "render",
        OriginDeclarationId::Function(free_function_origin("render")),
    );

    let declarations = build_draft(
        vec![root],
        Vec::new(),
        vec![binding],
        trait_source_facts,
        &DraftRefs {
            nominal_origins: &FxHashMap::default(),
            trait_origins: &trait_origins,
            env: &env,
            string_table: &string_table,
        },
    )
    .expect("source trait bound should project");

    let function = declaration_function(&declarations, "render");
    assert_eq!(
        &generic_parameters(function)[0].bounds,
        &[CanonicalTraitIdentity::Source(trait_origin("RENDERABLE"))],
        "a source trait bound must project to its canonical source identity"
    );
}

#[test]
fn generic_parameter_with_displayable_core_bound_projects_canonical_core_identity() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    let displayable_trait_id = TraitId(0);
    let param_list_id = register_param_list_with_bounds(
        &mut env,
        &mut string_table,
        "T",
        vec![displayable_trait_id],
    );

    let generic_type_id = env
        .type_id_for_generic_parameter(
            env.generic_parameters(param_list_id).unwrap().parameters[0].id,
        )
        .unwrap();

    let param = param_declaration("value", generic_type_id, &mut string_table);
    let signature = free_function_signature(vec![param], vec![generic_type_id]);
    let root = function_root("display", signature, Some(param_list_id), &mut string_table);

    let mut trait_source_facts = FxHashMap::default();
    trait_source_facts.insert(
        displayable_trait_id,
        crate::compiler_frontend::ast::ResolvedTraitSourceFact::Core(
            crate::compiler_frontend::traits::environment::CoreTraitKind::Displayable,
        ),
    );

    let binding = export_binding(
        "display",
        OriginDeclarationId::Function(free_function_origin("display")),
    );

    let declarations = build_draft(
        vec![root],
        Vec::new(),
        vec![binding],
        trait_source_facts,
        &DraftRefs {
            nominal_origins: &FxHashMap::default(),
            trait_origins: &FxHashMap::default(),
            env: &env,
            string_table: &string_table,
        },
    )
    .expect("displayable core bound should project");

    let function = declaration_function(&declarations, "display");
    assert_eq!(
        &generic_parameters(function)[0].bounds,
        &[CanonicalTraitIdentity::Core(
            CanonicalCoreTraitIdentity::Displayable
        )],
        "a Displayable core bound must project to its canonical core identity"
    );
}

#[test]
fn multiple_bounds_preserve_declaration_order() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    let source_trait_id = TraitId(0);
    let displayable_trait_id = TraitId(1);
    let cast_trait_id = TraitId(2);

    let param_list_id = register_param_list_with_bounds(
        &mut env,
        &mut string_table,
        "T",
        vec![source_trait_id, displayable_trait_id, cast_trait_id],
    );

    let generic_type_id = env
        .type_id_for_generic_parameter(
            env.generic_parameters(param_list_id).unwrap().parameters[0].id,
        )
        .unwrap();

    let param = param_declaration("value", generic_type_id, &mut string_table);
    let signature = free_function_signature(vec![param], vec![generic_type_id]);
    let root = function_root("multi", signature, Some(param_list_id), &mut string_table);

    let source_trait_path = path("RENDERABLE", &mut string_table);
    let mut trait_source_facts = FxHashMap::default();
    trait_source_facts.insert(
        source_trait_id,
        crate::compiler_frontend::ast::ResolvedTraitSourceFact::Source(source_trait_path.clone()),
    );
    trait_source_facts.insert(
        displayable_trait_id,
        crate::compiler_frontend::ast::ResolvedTraitSourceFact::Core(
            crate::compiler_frontend::traits::environment::CoreTraitKind::Displayable,
        ),
    );
    trait_source_facts.insert(
        cast_trait_id,
        crate::compiler_frontend::ast::ResolvedTraitSourceFact::Core(
            crate::compiler_frontend::traits::environment::CoreTraitKind::Castable {
                target: BuiltinCastTarget::String,
                fallibility: BuiltinCastFallibility::Infallible,
            },
        ),
    );

    let mut trait_origins = FxHashMap::default();
    trait_origins.insert(source_trait_path, trait_origin("RENDERABLE"));

    let binding = export_binding(
        "multi",
        OriginDeclarationId::Function(free_function_origin("multi")),
    );

    let declarations = build_draft(
        vec![root],
        Vec::new(),
        vec![binding],
        trait_source_facts,
        &DraftRefs {
            nominal_origins: &FxHashMap::default(),
            trait_origins: &trait_origins,
            env: &env,
            string_table: &string_table,
        },
    )
    .expect("multiple bounds should project in order");

    let function = declaration_function(&declarations, "multi");
    assert_eq!(
        &generic_parameters(function)[0].bounds,
        &[
            CanonicalTraitIdentity::Source(trait_origin("RENDERABLE")),
            CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Displayable),
            CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Castable {
                target: BuiltinCastTarget::String,
                fallibility: BuiltinCastFallibility::Infallible,
            }),
        ],
        "multiple bounds must be projected in declaration-site order"
    );
}

#[test]
fn missing_trait_source_fact_is_compiler_error() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    let unknown_trait_id = TraitId(99);
    let param_list_id =
        register_param_list_with_bounds(&mut env, &mut string_table, "T", vec![unknown_trait_id]);

    let generic_type_id = env
        .type_id_for_generic_parameter(
            env.generic_parameters(param_list_id).unwrap().parameters[0].id,
        )
        .unwrap();

    let param = param_declaration("value", generic_type_id, &mut string_table);
    let signature = free_function_signature(vec![param], vec![generic_type_id]);
    let root = function_root("missing", signature, Some(param_list_id), &mut string_table);

    let binding = export_binding(
        "missing",
        OriginDeclarationId::Function(free_function_origin("missing")),
    );

    let result = build_draft(
        vec![root],
        Vec::new(),
        vec![binding],
        FxHashMap::default(),
        &DraftRefs {
            nominal_origins: &FxHashMap::default(),
            trait_origins: &FxHashMap::default(),
            env: &env,
            string_table: &string_table,
        },
    );

    assert!(
        result.is_err(),
        "a bound TraitId with no retained trait source fact must be a CompilerError"
    );
}

#[test]
fn duplicate_canonical_generic_bound_identity_is_compiler_error() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    let first_trait_id = TraitId(0);
    let second_trait_id = TraitId(1);
    let param_list_id = register_param_list_with_bounds(
        &mut env,
        &mut string_table,
        "T",
        vec![first_trait_id, second_trait_id],
    );

    let generic_type_id = env
        .type_id_for_generic_parameter(
            env.generic_parameters(param_list_id).unwrap().parameters[0].id,
        )
        .unwrap();
    let signature = free_function_signature(
        vec![param_declaration(
            "value",
            generic_type_id,
            &mut string_table,
        )],
        vec![generic_type_id],
    );
    let root = function_root(
        "duplicate_bound",
        signature,
        Some(param_list_id),
        &mut string_table,
    );

    let trait_path = path("RENDERABLE", &mut string_table);
    let mut trait_source_facts = FxHashMap::default();
    trait_source_facts.insert(
        first_trait_id,
        crate::compiler_frontend::ast::ResolvedTraitSourceFact::Source(trait_path.clone()),
    );
    trait_source_facts.insert(
        second_trait_id,
        crate::compiler_frontend::ast::ResolvedTraitSourceFact::Source(trait_path.clone()),
    );

    let mut trait_origins = FxHashMap::default();
    trait_origins.insert(trait_path, trait_origin("RENDERABLE"));

    let result = build_draft(
        vec![root],
        Vec::new(),
        vec![export_binding(
            "duplicate_bound",
            OriginDeclarationId::Function(free_function_origin("duplicate_bound")),
        )],
        trait_source_facts,
        &DraftRefs {
            nominal_origins: &FxHashMap::default(),
            trait_origins: &trait_origins,
            env: &env,
            string_table: &string_table,
        },
    )
    .expect_err("duplicate canonical generic bounds must be rejected");

    assert!(
        result.msg.contains("same canonical trait identity"),
        "the failure should identify the collapsed canonical bound identity"
    );
}

#[test]
fn source_trait_bound_resolves_to_provider_module_origin_not_active_origin() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    let source_trait_id = TraitId(0);
    let param_list_id =
        register_param_list_with_bounds(&mut env, &mut string_table, "T", vec![source_trait_id]);

    let generic_type_id = env
        .type_id_for_generic_parameter(
            env.generic_parameters(param_list_id).unwrap().parameters[0].id,
        )
        .unwrap();

    let param = param_declaration("value", generic_type_id, &mut string_table);
    let signature = free_function_signature(vec![param], vec![generic_type_id]);
    let root = function_root("render", signature, Some(param_list_id), &mut string_table);

    let trait_path = path("RENDERABLE", &mut string_table);
    let mut trait_source_facts = FxHashMap::default();
    trait_source_facts.insert(
        source_trait_id,
        crate::compiler_frontend::ast::ResolvedTraitSourceFact::Source(trait_path.clone()),
    );

    // The trait is defined by an imported provider module whose origin differs from the
    // active module that owns the generic function. The projection must resolve the bound to
    // the trait's provider module origin, never the active function module origin.
    let provider_origin = module_origin("provider");
    let provider_trait_origin = OriginTraitId::new(provider_origin, "RENDERABLE".to_owned());

    let mut trait_origins = FxHashMap::default();
    trait_origins.insert(trait_path, provider_trait_origin.clone());

    let binding = export_binding(
        "render",
        OriginDeclarationId::Function(free_function_origin("render")),
    );

    let declarations = build_draft(
        vec![root],
        Vec::new(),
        vec![binding],
        trait_source_facts,
        &DraftRefs {
            nominal_origins: &FxHashMap::default(),
            trait_origins: &trait_origins,
            env: &env,
            string_table: &string_table,
        },
    )
    .expect("a source trait bound from a provider module should project");

    let function = declaration_function(&declarations, "render");
    assert_eq!(
        &generic_parameters(function)[0].bounds,
        &[CanonicalTraitIdentity::Source(provider_trait_origin)],
        "a source-bound trait must resolve to its provider module origin, not the active module origin"
    );
}

// ---------------------------------------------------------------------------
//  Declaration-owned free-function parameter access
// ---------------------------------------------------------------------------

/// Build and project one exported non-generic free function with the given single parameter
/// declaration, returning the projected function semantics so the access contract can be
/// asserted on the final declaration record.
fn project_free_function_with_parameter(
    parameter: Declaration,
    env: &TypeEnvironment,
    string_table: &mut StringTable,
) -> crate::compiler_frontend::public_interface::PublicFunctionSemantics {
    let int_id = env.builtins().int;
    let signature = free_function_signature(vec![parameter], vec![int_id]);
    let root = function_root("render", signature, None, string_table);
    let binding = export_binding(
        "render",
        OriginDeclarationId::Function(free_function_origin("render")),
    );
    let declarations = build_draft(
        vec![root],
        Vec::new(),
        vec![binding],
        FxHashMap::default(),
        &DraftRefs {
            nominal_origins: &FxHashMap::default(),
            trait_origins: &FxHashMap::default(),
            env,
            string_table,
        },
    )
    .expect("free function projection should succeed");
    declaration_function(&declarations, "render").clone()
}

#[test]
fn shared_free_function_parameter_projects_shared_access() {
    let env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let int_id = env.builtins().int;

    let function = project_free_function_with_parameter(
        param_declaration("value", int_id, &mut string_table),
        &env,
        &mut string_table,
    );

    assert_eq!(
        function.parameters[0].access,
        PublicCallParameterAccess::Shared,
        "an immutable parameter projects declaration-owned shared access"
    );
    assert!(
        matches!(function.category, PublicFunctionCategory::ConcreteLocal),
        "a non-generic free function remains a concrete-local declaration"
    );
}

#[test]
fn mutable_free_function_parameter_projects_mutable_access() {
    let env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let int_id = env.builtins().int;

    let function = project_free_function_with_parameter(
        mutable_param_declaration("value", int_id, &mut string_table),
        &env,
        &mut string_table,
    );

    assert_eq!(
        function.parameters[0].access,
        PublicCallParameterAccess::Mutable,
        "a mutable parameter projects declaration-owned mutable access"
    );
    assert!(
        matches!(function.category, PublicFunctionCategory::ConcreteLocal),
        "a non-generic free function remains a concrete-local declaration"
    );
}

#[test]
fn reactive_free_function_parameter_projects_reactive_access() {
    let env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let int_id = env.builtins().int;

    let function = project_free_function_with_parameter(
        reactive_param_declaration("source", int_id, &mut string_table),
        &env,
        &mut string_table,
    );

    assert_eq!(
        function.parameters[0].access,
        PublicCallParameterAccess::Reactive,
        "a reactive parameter projects declaration-owned reactive access"
    );
    assert!(
        matches!(function.category, PublicFunctionCategory::ConcreteLocal),
        "a non-generic free function remains a concrete-local declaration"
    );
}

#[test]
fn generic_free_function_retains_mutable_parameter_access_without_concrete_summary() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let param_list_id = register_single_param_list(&mut env, &mut string_table, "T");
    let generic_type_id = env
        .type_id_for_generic_parameter(
            env.generic_parameters(param_list_id).unwrap().parameters[0].id,
        )
        .expect("generic parameter must have a TypeId");

    let parameter = mutable_param_declaration("value", generic_type_id, &mut string_table);
    let signature = free_function_signature(vec![parameter], vec![generic_type_id]);
    let root = function_root(
        "identity",
        signature,
        Some(param_list_id),
        &mut string_table,
    );
    let binding = export_binding(
        "identity",
        OriginDeclarationId::Function(free_function_origin("identity")),
    );
    let declarations = build_draft(
        vec![root],
        Vec::new(),
        vec![binding],
        FxHashMap::default(),
        &DraftRefs {
            nominal_origins: &FxHashMap::default(),
            trait_origins: &FxHashMap::default(),
            env: &env,
            string_table: &string_table,
        },
    )
    .expect("generic function projection should succeed");

    let function = declaration_function(&declarations, "identity");
    assert!(
        matches!(
            function.category,
            PublicFunctionCategory::GenericTemplate(_)
        ),
        "a generic free function remains a generic-template declaration, not a concrete callable"
    );
    assert_eq!(
        function.parameters[0].access,
        PublicCallParameterAccess::Mutable,
        "a generic declaration retains declaration-owned mutable access before a concrete generated summary exists"
    );
}

// ---------------------------------------------------------------------------
//  Nested option/collection and imported public nominal references
// ---------------------------------------------------------------------------

#[test]
fn projects_nested_collection_and_option_types() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let int_id = env.builtins().int;

    let collection_id = env.intern_collection(int_id, None);
    let option_id = env.intern_option(collection_id);

    let param = param_declaration("items", option_id, &mut string_table);
    let signature = free_function_signature(vec![param], vec![collection_id]);
    let root = function_root("collect", signature, None, &mut string_table);

    let binding = export_binding(
        "collect",
        OriginDeclarationId::Function(free_function_origin("collect")),
    );

    let declarations = build_draft(
        vec![root],
        Vec::new(),
        vec![binding],
        FxHashMap::default(),
        &DraftRefs {
            nominal_origins: &FxHashMap::default(),
            trait_origins: &FxHashMap::default(),
            env: &env,
            string_table: &string_table,
        },
    )
    .expect("nested constructed type projection should succeed");

    let function = declaration_function(&declarations, "collect");
    assert_eq!(
        &function.parameters[0].type_identity,
        &CanonicalTypeIdentity::Option(Box::new(CanonicalTypeIdentity::Collection(
            crate::compiler_frontend::canonical_type_identity::CollectionTypeIdentity::new(
                CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
                None,
            )
        ))),
        "nested option(collection(int)) must project recursively"
    );
    assert_eq!(
        &function.returns[0].type_identity,
        &CanonicalTypeIdentity::Collection(
            crate::compiler_frontend::canonical_type_identity::CollectionTypeIdentity::new(
                CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
                None,
            )
        )
    );
}

#[test]
fn projects_imported_public_nominal_reference_to_provider_origin() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    // An imported public struct "Imported" owned by a different module origin.
    let (_imported_nominal_id, imported_type_id) = register_struct(
        &mut env,
        &mut string_table,
        "Imported",
        empty_fields(),
        None,
    );

    // A directly-defined public struct "Widget" with a field of the imported type.
    let fields = Box::new([field_def("value", imported_type_id, &mut string_table)]);
    let (_widget_nominal_id, widget_type_id) =
        register_struct(&mut env, &mut string_table, "Widget", fields, None);

    let root = struct_root(
        "Widget",
        widget_type_id,
        vec![field_declaration(
            "value",
            imported_type_id,
            &mut string_table,
        )],
        &mut string_table,
    );

    let binding = export_binding("Widget", OriginDeclarationId::Type(struct_origin("Widget")));

    // The expanded nominal origin index carries both the active-root nominal (Widget) and the
    // imported project-graph nominal (Imported) with its provider module origin.
    let nominal_map = nominal_origins_map(
        vec![
            ("Widget", struct_origin("Widget")),
            ("Imported", imported_struct_origin("Imported")),
        ],
        &mut string_table,
    );

    let declarations = build_draft(
        vec![root],
        Vec::new(),
        vec![binding],
        FxHashMap::default(),
        &DraftRefs {
            nominal_origins: &nominal_map,
            trait_origins: &FxHashMap::default(),
            env: &env,
            string_table: &string_table,
        },
    )
    .expect("imported nominal projection should succeed");

    let nominal = declaration_struct(&declarations, "Widget");
    assert_eq!(nominal.fields[0].name.as_str(), "value");
    assert_eq!(
        &nominal.fields[0].type_identity,
        &CanonicalTypeIdentity::SourceNominal(imported_struct_origin("Imported")),
        "a directly-defined public field referencing an imported public nominal must project \
         to SourceNominal(provider_origin), not the active module origin"
    );
}

#[test]
fn imported_nominal_required_but_absent_from_index_is_compiler_error() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    let (_imported_nominal_id, imported_type_id) = register_struct(
        &mut env,
        &mut string_table,
        "Imported",
        empty_fields(),
        None,
    );

    let fields = Box::new([field_def("value", imported_type_id, &mut string_table)]);
    let (_widget_nominal_id, widget_type_id) =
        register_struct(&mut env, &mut string_table, "Widget", fields, None);

    let root = struct_root(
        "Widget",
        widget_type_id,
        vec![field_declaration(
            "value",
            imported_type_id,
            &mut string_table,
        )],
        &mut string_table,
    );

    let binding = export_binding("Widget", OriginDeclarationId::Type(struct_origin("Widget")));

    // The index carries only the active-root nominal; "Imported" is absent, so its required
    // nominal reference cannot resolve and must fail with a precise CompilerError rather than a
    // path/display identity fallback.
    let nominal_map =
        nominal_origins_map(vec![("Widget", struct_origin("Widget"))], &mut string_table);

    let result = build_draft(
        vec![root],
        Vec::new(),
        vec![binding],
        FxHashMap::default(),
        &DraftRefs {
            nominal_origins: &nominal_map,
            trait_origins: &FxHashMap::default(),
            env: &env,
            string_table: &string_table,
        },
    );
    assert!(
        result.is_err(),
        "a public field referencing an imported nominal absent from the source-nominal origin \
         index (None owner) must be a CompilerError"
    );
}

// ---------------------------------------------------------------------------
//  CompilerError totality failures
// ---------------------------------------------------------------------------

#[test]
fn missing_nominal_origin_is_compiler_error() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let int_id = env.builtins().int;

    let fields = Box::new([field_def("x", int_id, &mut string_table)]);
    let (_nominal_id, type_id) =
        register_struct(&mut env, &mut string_table, "Point", fields, None);

    let root = struct_root("Point", type_id, Vec::new(), &mut string_table);
    let binding = export_binding("Point", OriginDeclarationId::Type(struct_origin("Point")));

    // Empty nominal map: the struct is not a registered public nominal origin.
    let result = build_draft(
        vec![root],
        Vec::new(),
        vec![binding],
        FxHashMap::default(),
        &DraftRefs {
            nominal_origins: &FxHashMap::default(),
            trait_origins: &FxHashMap::default(),
            env: &env,
            string_table: &string_table,
        },
    );
    assert!(
        result.is_err(),
        "a struct whose nominal path is not in the public nominal-type origin index must fail"
    );
}

#[test]
fn missing_signature_slot_type_id_is_compiler_error() {
    let env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let int_id = env.builtins().int;

    let param = param_declaration("value", int_id, &mut string_table);
    let signature = FunctionSignature {
        parameters: vec![param],
        returns: vec![unresolved_return_slot(ReturnChannel::Success)],
    };
    let root = function_root("unresolved_return", signature, None, &mut string_table);

    let binding = export_binding(
        "unresolved_return",
        OriginDeclarationId::Function(free_function_origin("unresolved_return")),
    );

    let result = build_draft(
        vec![root],
        Vec::new(),
        vec![binding],
        FxHashMap::default(),
        &DraftRefs {
            nominal_origins: &FxHashMap::default(),
            trait_origins: &FxHashMap::default(),
            env: &env,
            string_table: &string_table,
        },
    );
    assert!(
        result.is_err(),
        "a return slot with no TypeId must be a CompilerError, not silently omitted"
    );
}

#[test]
fn category_mismatch_between_root_and_binding_is_compiler_error() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    let (_nominal_id, type_id) =
        register_struct(&mut env, &mut string_table, "Widget", empty_fields(), None);

    let root = struct_root("Widget", type_id, Vec::new(), &mut string_table);

    // The root is a struct but the binding origin says it is a constant.
    let binding = export_binding(
        "Widget",
        OriginDeclarationId::Constant(constant_origin("Widget")),
    );
    let nominal_map =
        nominal_origins_map(vec![("Widget", struct_origin("Widget"))], &mut string_table);

    let result = build_draft(
        vec![root],
        Vec::new(),
        vec![binding],
        FxHashMap::default(),
        &DraftRefs {
            nominal_origins: &nominal_map,
            trait_origins: &FxHashMap::default(),
            env: &env,
            string_table: &string_table,
        },
    );
    assert!(
        result.is_err(),
        "a struct root matched to a constant binding must fail"
    );
}

#[test]
fn unregistered_generic_parameter_in_signature_is_compiler_error() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let param_list_id = register_single_param_list(&mut env, &mut string_table, "T");
    let generic_type_id = env
        .type_id_for_generic_parameter(
            env.generic_parameters(param_list_id).unwrap().parameters[0].id,
        )
        .expect("generic parameter must have a TypeId");

    let param = param_declaration("value", generic_type_id, &mut string_table);
    let signature = free_function_signature(vec![param], vec![generic_type_id]);

    // Create the root WITHOUT the generic_parameter_list_id, so the resolver won't register it.
    let root = function_root("missing_generic", signature, None, &mut string_table);
    let binding = export_binding(
        "missing_generic",
        OriginDeclarationId::Function(free_function_origin("missing_generic")),
    );

    let result = build_draft(
        vec![root],
        Vec::new(),
        vec![binding],
        FxHashMap::default(),
        &DraftRefs {
            nominal_origins: &FxHashMap::default(),
            trait_origins: &FxHashMap::default(),
            env: &env,
            string_table: &string_table,
        },
    );
    assert!(
        result.is_err(),
        "a generic parameter whose owner was not registered must be a CompilerError"
    );
}

#[test]
fn non_trait_binding_without_matching_root_is_compiler_error() {
    let env = TypeEnvironment::new();
    let string_table = StringTable::new();

    // A binding with no matching root in the root table.
    let binding = export_binding(
        "orphan",
        OriginDeclarationId::Function(free_function_origin("orphan")),
    );

    let result = build_draft(
        Vec::new(),
        Vec::new(),
        vec![binding],
        FxHashMap::default(),
        &DraftRefs {
            nominal_origins: &FxHashMap::default(),
            trait_origins: &FxHashMap::default(),
            env: &env,
            string_table: &string_table,
        },
    );
    assert!(
        result.is_err(),
        "a non-trait binding with no matching root must be a CompilerError"
    );
}

#[test]
fn unmatched_extra_root_is_compiler_error() {
    let env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let int_id = env.builtins().int;

    // A root with no matching export binding.
    let root = constant_root("extra", int_id, &mut string_table);

    let result = build_draft(
        vec![root],
        Vec::new(),
        Vec::new(),
        FxHashMap::default(),
        &DraftRefs {
            nominal_origins: &FxHashMap::default(),
            trait_origins: &FxHashMap::default(),
            env: &env,
            string_table: &string_table,
        },
    );
    assert!(
        result.is_err(),
        "a root with no matching export binding must be a CompilerError"
    );
}

// ---------------------------------------------------------------------------
//  Declaration-owned receiver-method parameter access (non-generic)
// ---------------------------------------------------------------------------

/// Build and project one exported non-generic struct with a single receiver method, returning
/// the projected receiver-method semantics so the receiver parameter access contract can be
/// asserted on the final declaration record.
enum ReceiverAccessFixture {
    Shared,
    Mutable,
}

fn project_struct_with_receiver_method(
    method_name: &str,
    access: ReceiverAccessFixture,
    env: &mut TypeEnvironment,
    string_table: &mut StringTable,
) -> crate::compiler_frontend::public_interface::PublicReceiverMethodSemantics {
    let receiver_path = path("Counter", string_table);
    let (_, struct_type_id) =
        register_struct_at_path(env, receiver_path.clone(), empty_fields(), None);
    let root = struct_root("Counter", struct_type_id, Vec::new(), string_table);
    let binding = export_binding(
        "Counter",
        OriginDeclarationId::Type(struct_origin("Counter")),
    );
    let nominal_origins =
        nominal_origins_map(vec![("Counter", struct_origin("Counter"))], string_table);

    let mut receiver_parameter = param_declaration("this", struct_type_id, string_table);
    let receiver_mutable = matches!(access, ReceiverAccessFixture::Mutable);
    if receiver_mutable {
        receiver_parameter.value.value_mode = ValueMode::MutableReference;
    }

    let mut entry = receiver_entry(
        path(method_name, string_table),
        ReceiverKey::Struct(receiver_path),
        FunctionSignature {
            parameters: vec![receiver_parameter],
            returns: Vec::new(),
        },
    );
    entry.receiver_mutable = receiver_mutable;

    let table = root_table(vec![root], vec![entry], FxHashMap::default());
    let export_seed = DirectExportSeed::new(
        active_module_origin(),
        vec![binding],
        nominal_origins.clone(),
    );
    let projection_input = AstPublicInterfaceProjectionInput {
        root_table: table,
        trait_roots: Vec::new(),
        trait_environment: Some(Rc::new(TraitEnvironment::new())),
        trait_evidence_environment: Some(Rc::new(TraitEvidenceEnvironment::new())),
        const_templates_by_name: FxHashMap::default(),
    };
    let registry = ExternalPackageRegistry::new();
    let declarations = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: &nominal_origins,
        public_source_trait_origins: &FxHashMap::default(),
        type_environment: env,
        external_registry: &registry,
        string_table,
        generic_function_templates: &FxHashMap::default(),
        module_constants: &[],
    })
    .build()
    .map(|result| result.draft.declarations)
    .expect("receiver-method projection should succeed");

    let struct_semantics = declaration_struct(&declarations, "Counter");
    assert_eq!(
        struct_semantics.receiver_methods.len(),
        1,
        "one receiver method must attach to the struct declaration record"
    );
    struct_semantics.receiver_methods[0].clone()
}

#[test]
fn shared_receiver_method_projects_shared_access() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    let method = project_struct_with_receiver_method(
        "value",
        ReceiverAccessFixture::Shared,
        &mut env,
        &mut string_table,
    );

    assert_eq!(
        method.parameters[0].access,
        PublicCallParameterAccess::Shared,
        "an immutable receiver projects declaration-owned shared access"
    );
    assert!(
        matches!(
            method.category,
            crate::compiler_frontend::public_interface::PublicReceiverMethodCategory::ConcreteLocal
        ),
        "a non-generic receiver method remains a concrete-local declaration"
    );
}

#[test]
fn mutable_receiver_method_projects_mutable_access() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    let method = project_struct_with_receiver_method(
        "reset",
        ReceiverAccessFixture::Mutable,
        &mut env,
        &mut string_table,
    );

    assert_eq!(
        method.parameters[0].access,
        PublicCallParameterAccess::Mutable,
        "a mutable receiver projects declaration-owned mutable access"
    );
    assert!(
        matches!(
            method.category,
            crate::compiler_frontend::public_interface::PublicReceiverMethodCategory::ConcreteLocal
        ),
        "a non-generic receiver method remains a concrete-local declaration"
    );
}

// ---------------------------------------------------------------------------
//  Receiver-method exact-origin join and mismatch rejection
// ---------------------------------------------------------------------------

fn module_path(module: &str, name: &str, string_table: &mut StringTable) -> InternedPath {
    let mut path = InternedPath::from_single_str(module, string_table);
    path.push_str(name, string_table);
    path
}

#[test]
fn receiver_methods_join_by_exact_origin_not_rendered_name() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let int_id = env.builtins().int;

    // Two same-named nominals "Counter" in different modules, with distinct canonical paths.
    let shapes_path = module_path("shapes", "Counter", &mut string_table);
    let imports_path = module_path("imports", "Counter", &mut string_table);
    let (_shapes_nominal_id, _) =
        register_struct_at_path(&mut env, shapes_path.clone(), empty_fields(), None);
    let (_imports_nominal_id, _) =
        register_struct_at_path(&mut env, imports_path.clone(), empty_fields(), None);

    let shapes_origin = struct_origin("Counter");
    let imports_origin = imported_struct_origin("Counter");

    let nominal_map = FxHashMap::from_iter([
        (shapes_path.clone(), shapes_origin.clone()),
        (imports_path.clone(), imports_origin.clone()),
    ]);

    // A "tick" method on each receiver. Rendered names collide ("Counter::tick"), so only the
    // exact stable origin can join the right entry.
    let make_entry =
        |method_path: InternedPath, receiver_path: InternedPath, string_table: &mut StringTable| {
            let param = param_declaration("delta", int_id, string_table);
            let signature = FunctionSignature {
                parameters: vec![param],
                returns: vec![return_slot(int_id, ReturnChannel::Success)],
            };
            receiver_entry(method_path, ReceiverKey::Struct(receiver_path), signature)
        };
    let tick_shapes_path = module_path("shapes", "tick", &mut string_table);
    let tick_imports_path = module_path("imports", "tick", &mut string_table);
    let entry_shapes = make_entry(tick_shapes_path, shapes_path.clone(), &mut string_table);
    let entry_imports = make_entry(tick_imports_path, imports_path.clone(), &mut string_table);

    // A free-function binding + root provides the module origin for the seed builder. The
    // test exercises receiver-method seeds, not free-function seeds.
    let (helper_binding, helper_root) = free_fn_binding_and_root("helper", &mut string_table);
    let table = root_table(
        vec![helper_root],
        vec![entry_shapes, entry_imports],
        FxHashMap::default(),
    );

    let callable_seeds = build_callable_seed_table(
        std::slice::from_ref(&helper_binding),
        &active_module_origin(),
        &nominal_map,
        &table,
        &FxHashMap::default(),
        &string_table,
    )
    .expect("callable seed table should build");

    let receiver_seeds: Vec<&CallableSeed> = callable_seeds
        .iter()
        .filter(|seed| matches!(seed.kind, CallableSeedKind::ReceiverMethod { .. }))
        .collect();
    assert_eq!(receiver_seeds.len(), 2);
    let by_receiver: FxHashMap<&OriginTypeId, &CallableSeed> = receiver_seeds
        .iter()
        .filter_map(|seed| {
            if let CallableSeedKind::ReceiverMethod {
                receiver_origin, ..
            } = &seed.kind
            {
                Some((receiver_origin, *seed))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        by_receiver
            .get(&shapes_origin)
            .unwrap()
            .origin
            .defining_name(),
        "tick",
        "the shapes receiver method must join its own origin"
    );
    assert_eq!(
        by_receiver
            .get(&imports_origin)
            .unwrap()
            .origin
            .defining_name(),
        "tick",
        "the imports receiver method must join its own origin"
    );
}

#[test]
fn duplicate_receiver_method_entry_is_compiler_error() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let int_id = env.builtins().int;

    let receiver_path = path("Counter", &mut string_table);
    let (_nominal_id, _) =
        register_struct(&mut env, &mut string_table, "Counter", empty_fields(), None);
    let nominal_map = nominal_origins_map(
        vec![("Counter", struct_origin("Counter"))],
        &mut string_table,
    );

    let method_path = path("tick", &mut string_table);
    let param = param_declaration("delta", int_id, &mut string_table);
    let signature = FunctionSignature {
        parameters: vec![param],
        returns: vec![return_slot(int_id, ReturnChannel::Success)],
    };

    // Two entries with the same exact stable receiver origin and method name.
    let entry_a = receiver_entry(
        method_path.clone(),
        ReceiverKey::Struct(receiver_path.clone()),
        signature.clone(),
    );
    let entry_b = receiver_entry(method_path, ReceiverKey::Struct(receiver_path), signature);

    let (helper_binding, helper_root) = free_fn_binding_and_root("helper", &mut string_table);
    let table = root_table(
        vec![helper_root],
        vec![entry_a, entry_b],
        FxHashMap::default(),
    );

    let result = build_callable_seed_table(
        std::slice::from_ref(&helper_binding),
        &active_module_origin(),
        &nominal_map,
        &table,
        &FxHashMap::default(),
        &string_table,
    );
    assert!(
        result.is_err(),
        "two receiver-method entries sharing the exact stable receiver origin and method name \
         must be a CompilerError, not a silent overwrite"
    );
}

#[test]
fn duplicate_exact_seed_path_with_distinct_origins_is_compiler_error() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let int_id = env.builtins().int;

    // Two receivers with distinct origins, but the method entries share the same function_path.
    // Same-named methods on distinct receivers must have distinct declaration paths; a shared
    // path is a duplicate that the seed construction boundary must reject.
    let alpha_path = path("Alpha", &mut string_table);
    let beta_path = path("Beta", &mut string_table);
    let (_alpha_id, _) =
        register_struct_at_path(&mut env, alpha_path.clone(), empty_fields(), None);
    let (_beta_id, _) = register_struct_at_path(&mut env, beta_path.clone(), empty_fields(), None);

    let shared_method_path = path("tick", &mut string_table);
    let param = param_declaration("delta", int_id, &mut string_table);
    let signature = FunctionSignature {
        parameters: vec![param],
        returns: vec![return_slot(int_id, ReturnChannel::Success)],
    };
    let entry_alpha = receiver_entry(
        shared_method_path.clone(),
        ReceiverKey::Struct(alpha_path.clone()),
        signature.clone(),
    );
    let entry_beta = receiver_entry(
        shared_method_path,
        ReceiverKey::Struct(beta_path.clone()),
        signature,
    );

    let nominal_map = FxHashMap::from_iter([
        (alpha_path, struct_origin("Alpha")),
        (beta_path, struct_origin("Beta")),
    ]);

    let (helper_binding, helper_root) = free_fn_binding_and_root("helper", &mut string_table);
    let table = root_table(
        vec![helper_root],
        vec![entry_alpha, entry_beta],
        FxHashMap::default(),
    );

    let result = build_callable_seed_table(
        std::slice::from_ref(&helper_binding),
        &active_module_origin(),
        &nominal_map,
        &table,
        &FxHashMap::default(),
        &string_table,
    );
    assert!(
        result.is_err(),
        "two receiver-method seeds sharing the exact declaration path must be a CompilerError \
         even when their stable origins differ"
    );
    let message = result.unwrap_err().msg.clone();
    assert!(
        message.contains("duplicate public callable declaration path"),
        "expected a duplicate-path diagnostic, got: {message}"
    );
}

#[test]
fn struct_receiver_key_with_choice_origin_is_compiler_error() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let int_id = env.builtins().int;

    let receiver_path = path("Counter", &mut string_table);
    // The nominal is registered as a struct so the TypeEnvironment can resolve its path, but the
    // public nominal origin index names it as a choice: the struct receiver key must disagree.
    let (_nominal_id, _) =
        register_struct(&mut env, &mut string_table, "Counter", empty_fields(), None);
    let nominal_map = nominal_origins_map(
        vec![("Counter", choice_origin("Counter"))],
        &mut string_table,
    );

    let method_path = path("tick", &mut string_table);
    let param = param_declaration("delta", int_id, &mut string_table);
    let signature = FunctionSignature {
        parameters: vec![param],
        returns: vec![return_slot(int_id, ReturnChannel::Success)],
    };
    let entry = receiver_entry(method_path, ReceiverKey::Struct(receiver_path), signature);

    let (helper_binding, helper_root) = free_fn_binding_and_root("helper", &mut string_table);
    let table = root_table(vec![helper_root], vec![entry], FxHashMap::default());

    let result = build_callable_seed_table(
        std::slice::from_ref(&helper_binding),
        &active_module_origin(),
        &nominal_map,
        &table,
        &FxHashMap::default(),
        &string_table,
    );
    assert!(
        result.is_err(),
        "a struct receiver key whose resolved nominal origin is a choice must be a CompilerError \
         rather than a silent coercion"
    );
}

#[test]
fn choice_receiver_key_with_struct_origin_is_compiler_error() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let int_id = env.builtins().int;

    let receiver_path = path("Counter", &mut string_table);
    // The nominal is registered as a choice so the TypeEnvironment can resolve its path, but the
    // public nominal origin index names it as a struct: the choice receiver key must disagree.
    let zero_variant = unit_variant("Zero", &mut string_table);
    let (_nominal_id, _) = register_choice(
        &mut env,
        &mut string_table,
        "Counter",
        Box::new([zero_variant]),
        None,
    );
    let nominal_map = nominal_origins_map(
        vec![("Counter", struct_origin("Counter"))],
        &mut string_table,
    );

    let method_path = path("tick", &mut string_table);
    let param = param_declaration("delta", int_id, &mut string_table);
    let signature = FunctionSignature {
        parameters: vec![param],
        returns: vec![return_slot(int_id, ReturnChannel::Success)],
    };
    let entry = receiver_entry(method_path, ReceiverKey::Choice(receiver_path), signature);

    let (helper_binding, helper_root) = free_fn_binding_and_root("helper", &mut string_table);
    let table = root_table(vec![helper_root], vec![entry], FxHashMap::default());

    let result = build_callable_seed_table(
        std::slice::from_ref(&helper_binding),
        &active_module_origin(),
        &nominal_map,
        &table,
        &FxHashMap::default(),
        &string_table,
    );
    assert!(
        result.is_err(),
        "a choice receiver key whose resolved nominal origin is a struct must be a CompilerError \
         rather than a silent coercion"
    );
}

// ---------------------------------------------------------------------------
//  Declaration-record accessors
// ---------------------------------------------------------------------------

fn declaration_function<'a>(
    declarations: &'a [PublicDeclarationRecord],
    name: &str,
) -> &'a crate::compiler_frontend::public_interface::PublicFunctionSemantics {
    let record = declarations
        .iter()
        .find(|record| {
            matches!(&record.semantics, PublicDeclarationSemantics::Function(_))
                && origin_defining_name(&record.origin) == name
        })
        .unwrap_or_else(|| panic!("no function declaration record named `{name}`"));
    let PublicDeclarationSemantics::Function(semantics) = &record.semantics else {
        panic!("record `{name}` is not a function");
    };
    semantics
}

fn declaration_struct<'a>(
    declarations: &'a [PublicDeclarationRecord],
    name: &str,
) -> &'a PublicStructSemantics {
    let record = declarations
        .iter()
        .find(|record| {
            matches!(&record.semantics, PublicDeclarationSemantics::Struct(_))
                && origin_defining_name(&record.origin) == name
        })
        .unwrap_or_else(|| panic!("no struct declaration record named `{name}`"));
    let PublicDeclarationSemantics::Struct(semantics) = &record.semantics else {
        panic!("record `{name}` is not a struct");
    };
    semantics
}

fn declaration_choice<'a>(
    declarations: &'a [PublicDeclarationRecord],
    name: &str,
) -> &'a crate::compiler_frontend::public_interface::PublicChoiceSemantics {
    let record = declarations
        .iter()
        .find(|record| {
            matches!(&record.semantics, PublicDeclarationSemantics::Choice(_))
                && origin_defining_name(&record.origin) == name
        })
        .unwrap_or_else(|| panic!("no choice declaration record named `{name}`"));
    let PublicDeclarationSemantics::Choice(semantics) = &record.semantics else {
        panic!("record `{name}` is not a choice");
    };
    semantics
}

#[test]
fn struct_record_rejects_field_count_mismatch() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let int_id = env.builtins().int;
    let bool_id = env.builtins().bool;

    let field_definitions = Box::new([
        field_def("x", int_id, &mut string_table),
        field_def("flag", bool_id, &mut string_table),
    ]);
    let retained_fields = vec![field_declaration("x", int_id, &mut string_table)];

    let error = project_struct_record(
        &mut env,
        &mut string_table,
        field_definitions,
        retained_fields,
    )
    .expect_err("a missing retained field declaration must be rejected");

    assert!(error.msg.contains("retained declaration count must match"));
}

#[test]
fn struct_record_rejects_field_name_or_order_mismatch() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let int_id = env.builtins().int;
    let bool_id = env.builtins().bool;

    let field_definitions = Box::new([
        field_def("x", int_id, &mut string_table),
        field_def("flag", bool_id, &mut string_table),
    ]);
    let retained_fields = vec![
        field_declaration("flag", bool_id, &mut string_table),
        field_declaration("x", int_id, &mut string_table),
    ];

    let error = project_struct_record(
        &mut env,
        &mut string_table,
        field_definitions,
        retained_fields,
    )
    .expect_err("retained field order must match the canonical definition");

    assert!(error.msg.contains("field name or order mismatch"));
}

#[test]
fn struct_record_rejects_field_type_id_mismatch() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let int_id = env.builtins().int;
    let string_id = env.builtins().string;

    let field_definitions = Box::new([field_def("x", int_id, &mut string_table)]);
    let default_value = Expression::string_slice(
        string_table.intern("wrong"),
        location(),
        ValueMode::ImmutableOwned,
    );
    let retained_fields = vec![field_declaration_with_default(
        "x",
        string_id,
        default_value,
        &mut string_table,
    )];

    let error = project_struct_record(
        &mut env,
        &mut string_table,
        field_definitions,
        retained_fields,
    )
    .expect_err("a retained field TypeId mismatch must be rejected");

    assert!(error.msg.contains("TypeId mismatch"));
}

#[test]
fn struct_record_rejects_duplicate_canonical_field_name() {
    let mut env = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let int_id = env.builtins().int;
    let bool_id = env.builtins().bool;

    let field_definitions = Box::new([
        field_def("x", int_id, &mut string_table),
        field_def("x", bool_id, &mut string_table),
    ]);
    let retained_fields = vec![
        field_declaration("x", int_id, &mut string_table),
        field_declaration("x", bool_id, &mut string_table),
    ];

    let error = project_struct_record(
        &mut env,
        &mut string_table,
        field_definitions,
        retained_fields,
    )
    .expect_err("duplicate canonical field names must be rejected");

    assert!(error.msg.contains("duplicate field name"));
}

fn origin_defining_name(origin: &OriginDeclarationId) -> &str {
    match origin {
        OriginDeclarationId::Function(function) => function.defining_name(),
        OriginDeclarationId::Type(type_id) => type_id.defining_name(),
        OriginDeclarationId::Constant(constant) => constant.defining_name(),
        OriginDeclarationId::Trait(trait_id) => trait_id.defining_name(),
    }
}

fn generic_identity_list(
    function: &crate::compiler_frontend::public_interface::PublicFunctionSemantics,
) -> Vec<&ExportedGenericParameterIdentity> {
    let PublicFunctionCategory::GenericTemplate(descriptor) = &function.category else {
        panic!("expected a generic-template function category");
    };
    descriptor
        .generic_parameters
        .iter()
        .map(|surface| &surface.identity)
        .collect()
}
fn generic_parameters(
    function: &crate::compiler_frontend::public_interface::PublicFunctionSemantics,
) -> &[PublicGenericParameterSurface] {
    let PublicFunctionCategory::GenericTemplate(descriptor) = &function.category else {
        panic!("expected a generic-template function category");
    };
    &descriptor.generic_parameters
}
