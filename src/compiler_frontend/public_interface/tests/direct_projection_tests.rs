//! Focused hidden-invariant tests for declaration-centric draft-builder orchestration,
//! receiver attachment/classification and folded default retention.
//!
//! WHAT: exercises the invariants of [`PublicInterfaceDraftBuilder`] that integration output
//! cannot inspect: the declaration-centric draft covering every semantics category, receiver
//! methods attached to their owning struct record, generic-receiver classification from an
//! exact template path with Hir-origin seed exclusion, empty-surface survival, free-function
//! seed and missing-signature rejection in the receiver-method projection, and folded
//! parameter, field and choice-payload default retention in authored order.
//! WHY: these are orchestration and projection invariants owned by
//! `compiler_frontend::public_interface::direct_projection` and
//! `compiler_frontend::public_interface::receiver_projection`, so they own a focused test
//! beside the module rather than an end-to-end case.

use super::super::{
    CallableSeed, CallableSeedKind, DirectExportSeed, PublicDeclarationSemantics,
    PublicInterfaceDraftBuilder, PublicInterfaceDraftBuilderInput, PublicReceiverMethodCategory,
    receiver_method_semantics_from_seed,
};
use super::test_support::{
    choice_origin, constant_origin, empty_fields, free_function_origin, module_origin,
    nominal_origins_map, path, receiver_entry, register_struct, struct_origin, struct_root,
    this_type, trait_binding, trait_origin, trait_origins_map, trait_root,
};

use crate::compiler_frontend::analysis::borrow_checker::BorrowAnalysis;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::generic_functions::GenericFunctionTemplate;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::{
    AstPublicInterfaceProjectionInput, ReceiverMethodCatalog, ResolvedPublicTypeRoot,
    ResolvedPublicTypeRootKind, ResolvedPublicTypeRootTable,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::datatype::DataType;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, FieldDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_parameters::{
    GenericParameter, GenericParameterList as ParsedGenericParameterList, TypeParameterId,
};
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::public_call_summary::PublicCallParameterAccess;
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, OriginDeclarationId, OriginFunctionId, OriginTypeCategory, OriginTypeId,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation};
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::value_mode::ValueMode;

use rustc_hash::FxHashMap;

fn alias_origin(name: &str) -> OriginTypeId {
    OriginTypeId::new(
        module_origin(),
        name.to_owned(),
        OriginTypeCategory::TransparentAlias,
    )
}

fn empty_variant_box() -> Box<[ChoiceVariantDefinition]> {
    Box::new([])
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
        variants: empty_variant_box(),
        generic_parameters: None,
    })
}

fn function_root(
    name: &str,
    signature: FunctionSignature,
    string_table: &mut StringTable,
) -> ResolvedPublicTypeRoot {
    ResolvedPublicTypeRoot {
        path: path(name, string_table),
        kind: ResolvedPublicTypeRootKind::Function {
            signature,
            generic_parameter_list_id: None,
        },
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

fn alias_root(
    name: &str,
    target_type_id: TypeId,
    string_table: &mut StringTable,
) -> ResolvedPublicTypeRoot {
    ResolvedPublicTypeRoot {
        path: path(name, string_table),
        kind: ResolvedPublicTypeRootKind::TransparentAlias { target_type_id },
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

fn empty_signature() -> FunctionSignature {
    FunctionSignature {
        parameters: vec![],
        returns: vec![],
    }
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

fn field_declaration_no_default(
    name: &str,
    type_id: TypeId,
    string_table: &mut StringTable,
) -> Declaration {
    Declaration {
        id: path(name, string_table),
        value: Expression::no_value_with_type_id(
            SourceLocation::default(),
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
        location: SourceLocation::default(),
    }
}
#[test]
fn builder_produces_declaration_centric_draft_covering_every_category() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let int_id = env.builtins().int;

    // Register a struct and a choice so the type-surface projection can resolve them.
    let (_, struct_type_id) =
        register_struct(&mut env, &mut string_table, "Counter", empty_fields(), None);
    let (_, choice_type_id) = register_choice(&mut env, &mut string_table, "Status");

    // Build roots for every non-trait category.
    let function_root = function_root("render", empty_signature(), &mut string_table);
    let struct_root = struct_root("Counter", struct_type_id, vec![], &mut string_table);
    let choice_root = choice_root("Status", choice_type_id, &mut string_table);
    let alias_root = alias_root("IntAlias", int_id, &mut string_table);
    let constant_root = constant_root("MaxSize", int_id, &mut string_table);

    let root_table = ResolvedPublicTypeRootTable {
        roots: vec![
            function_root,
            struct_root,
            choice_root,
            alias_root,
            constant_root,
        ],
        receiver_methods: vec![],
        trait_source_facts: FxHashMap::default(),
    };

    // Build the trait root.
    let this_id = this_type(&mut env, &mut string_table);
    let trait_root = trait_root("Shape", this_id, vec![], &mut string_table);

    // Build export bindings for all six categories, in deterministic sorted order by name.
    let bindings = vec![
        ExportBinding::new(
            module_origin(),
            "Counter".to_owned(),
            OriginDeclarationId::Type(struct_origin("Counter")),
        ),
        ExportBinding::new(
            module_origin(),
            "IntAlias".to_owned(),
            OriginDeclarationId::Type(alias_origin("IntAlias")),
        ),
        ExportBinding::new(
            module_origin(),
            "MaxSize".to_owned(),
            OriginDeclarationId::Constant(constant_origin("MaxSize")),
        ),
        ExportBinding::new(
            module_origin(),
            "Status".to_owned(),
            OriginDeclarationId::Type(choice_origin("Status")),
        ),
        ExportBinding::new(
            module_origin(),
            "render".to_owned(),
            OriginDeclarationId::Function(free_function_origin("render")),
        ),
        trait_binding("Shape"),
    ];

    let nominal_origins = nominal_origins_map(
        vec![
            ("Counter", struct_origin("Counter")),
            ("Status", choice_origin("Status")),
        ],
        &mut string_table,
    );
    let trait_origins =
        trait_origins_map(vec![("Shape", trait_origin("Shape"))], &mut string_table);

    let export_seed = DirectExportSeed::new(module_origin(), bindings, nominal_origins.clone());

    let projection_input = AstPublicInterfaceProjectionInput {
        root_table,
        trait_roots: vec![trait_root],
        trait_environment: Some(std::rc::Rc::new(TraitEnvironment::new())),
        trait_evidence_environment: Some(std::rc::Rc::new(TraitEvidenceEnvironment::new())),
        const_templates_by_name: FxHashMap::default(),
    };

    let max_size_constant = Declaration {
        id: InternedPath::from_single_str("MaxSize", &mut string_table),
        value: Expression::int(256, SourceLocation::default(), ValueMode::ImmutableOwned),
    };
    let module_constants = vec![max_size_constant];

    let registry = ExternalPackageRegistry::new();
    let draft = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: &nominal_origins,
        public_source_trait_origins: &trait_origins,
        type_environment: &env,
        external_registry: &registry,
        string_table: &string_table,
        generic_function_templates: &FxHashMap::default(),
        module_constants: &module_constants,
    })
    .build()
    .expect("declaration-centric draft builds for all categories")
    .draft;

    // The draft owns its module origin.
    assert_eq!(draft.module_origin, module_origin());

    // The draft carries exactly six export bindings and six declaration records.
    assert_eq!(draft.export_bindings.len(), 6);
    assert_eq!(draft.declarations.len(), 6);

    // Every semantics category is present as a distinct variant. Collect them by origin name.
    let categories: Vec<&str> = draft
        .declarations
        .iter()
        .map(|record| match &record.semantics {
            PublicDeclarationSemantics::Function(_) => "function",
            PublicDeclarationSemantics::Struct(_) => "struct",
            PublicDeclarationSemantics::Choice(_) => "choice",
            PublicDeclarationSemantics::TransparentAlias(_) => "alias",
            PublicDeclarationSemantics::Constant(_) => "constant",
            PublicDeclarationSemantics::Trait(_) => "trait",
        })
        .collect();
    assert!(categories.contains(&"function"));
    assert!(categories.contains(&"struct"));
    assert!(categories.contains(&"choice"));
    assert!(categories.contains(&"alias"));
    assert!(categories.contains(&"constant"));
    assert!(categories.contains(&"trait"));

    // The constant record carries the canonical builtin int type.
    let constant_record = draft
        .declarations
        .iter()
        .find(|record| matches!(record.semantics, PublicDeclarationSemantics::Constant(_)))
        .expect("constant record exists");
    if let PublicDeclarationSemantics::Constant(semantics) = &constant_record.semantics {
        assert_eq!(
            semantics.type_identity,
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int)
        );
    }

    // The alias record carries the canonical builtin int target.
    let alias_record = draft
        .declarations
        .iter()
        .find(|record| {
            matches!(
                record.semantics,
                PublicDeclarationSemantics::TransparentAlias(_)
            )
        })
        .expect("alias record exists");
    if let PublicDeclarationSemantics::TransparentAlias(semantics) = &alias_record.semantics {
        assert_eq!(
            semantics.target_type_identity,
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int)
        );
    }

    // The trait record carries zero requirements and the correct origin.
    let trait_record = draft
        .declarations
        .iter()
        .find(|record| matches!(record.semantics, PublicDeclarationSemantics::Trait(_)))
        .expect("trait record exists");
    if let PublicDeclarationSemantics::Trait(semantics) = &trait_record.semantics {
        assert!(semantics.requirements.is_empty());
    }
    assert_eq!(
        trait_record.origin,
        OriginDeclarationId::Trait(trait_origin("Shape"))
    );
}

#[test]
fn builder_attaches_receiver_methods_to_struct_record() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    let (_, struct_type_id) =
        register_struct(&mut env, &mut string_table, "Counter", empty_fields(), None);

    let receiver_path = path("Counter", &mut string_table);
    let method_fn_path = path("render", &mut string_table);
    let signature = FunctionSignature {
        parameters: vec![],
        returns: vec![],
    };
    let entry = receiver_entry(
        method_fn_path.clone(),
        ReceiverKey::Struct(receiver_path),
        signature,
    );

    let root = struct_root("Counter", struct_type_id, vec![], &mut string_table);
    let root_table = ResolvedPublicTypeRootTable {
        roots: vec![root],
        receiver_methods: vec![entry.clone()],
        trait_source_facts: FxHashMap::default(),
    };

    let binding = ExportBinding::new(
        module_origin(),
        "Counter".to_owned(),
        OriginDeclarationId::Type(struct_origin("Counter")),
    );

    let method_origin = OriginFunctionId::new_receiver(
        module_origin(),
        "render".to_owned(),
        struct_origin("Counter"),
    );

    let nominal_origins = nominal_origins_map(
        vec![("Counter", struct_origin("Counter"))],
        &mut string_table,
    );

    let export_seed =
        DirectExportSeed::new(module_origin(), vec![binding], nominal_origins.clone());

    let mut catalog = ReceiverMethodCatalog::default();
    catalog.by_function_path.insert(method_fn_path, entry);

    let projection_input = AstPublicInterfaceProjectionInput {
        root_table,
        trait_roots: vec![],
        trait_environment: Some(std::rc::Rc::new(TraitEnvironment::new())),
        trait_evidence_environment: Some(std::rc::Rc::new(TraitEvidenceEnvironment::new())),
        const_templates_by_name: FxHashMap::default(),
    };

    let registry = ExternalPackageRegistry::new();
    let draft = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: &nominal_origins,
        public_source_trait_origins: &FxHashMap::default(),
        type_environment: &env,
        external_registry: &registry,
        string_table: &string_table,
        generic_function_templates: &FxHashMap::default(),
        module_constants: &[],
    })
    .build()
    .expect("draft with receiver method builds")
    .draft;

    assert_eq!(draft.declarations.len(), 1);
    let record = &draft.declarations[0];
    assert!(matches!(
        record.semantics,
        PublicDeclarationSemantics::Struct(_)
    ));
    if let PublicDeclarationSemantics::Struct(semantics) = &record.semantics {
        assert_eq!(semantics.receiver_methods.len(), 1);
        assert_eq!(semantics.receiver_methods[0].method_origin, method_origin);
    }
}

#[test]
fn builder_classifies_generic_receiver_from_exact_template_path_and_excludes_hir_origin_seed() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    // Register a generic parameter list with one authored parameter "A".
    let a_name = string_table.intern("A");
    let parsed_params = ParsedGenericParameterList {
        parameters: vec![GenericParameter {
            id: TypeParameterId(0),
            name: a_name,
            location: SourceLocation::default(),
            trait_bounds: vec![],
        }],
    };
    let registered_list =
        env.register_generic_parameter_list(&parsed_params, &FxHashMap::default());
    let list_id = registered_list.list_id;

    // Register a generic struct Box<A> whose generic parameter list matches the method
    // template below.
    let (_, struct_type_id) = register_struct(
        &mut env,
        &mut string_table,
        "Box",
        empty_fields(),
        Some(list_id),
    );

    let receiver_path = path("Box", &mut string_table);
    let method_fn_path = path("render", &mut string_table);
    let receiver = Declaration {
        id: path("this", &mut string_table),
        value: Expression::no_value_with_type_id(
            SourceLocation::default(),
            DataType::Inferred,
            struct_type_id,
            ValueMode::MutableReference,
        ),
    };
    let method_signature = FunctionSignature {
        parameters: vec![receiver],
        returns: vec![],
    };
    let mut entry = receiver_entry(
        method_fn_path.clone(),
        ReceiverKey::Struct(receiver_path),
        method_signature.clone(),
    );
    entry.receiver_mutable = true;

    let root_table = ResolvedPublicTypeRootTable {
        roots: vec![struct_root(
            "Box",
            struct_type_id,
            vec![],
            &mut string_table,
        )],
        receiver_methods: vec![entry.clone()],
        trait_source_facts: FxHashMap::default(),
    };

    let binding = ExportBinding::new(
        module_origin(),
        "Box".to_owned(),
        OriginDeclarationId::Type(struct_origin("Box")),
    );
    let method_origin =
        OriginFunctionId::new_receiver(module_origin(), "render".to_owned(), struct_origin("Box"));
    let nominal_origins =
        nominal_origins_map(vec![("Box", struct_origin("Box"))], &mut string_table);

    let export_seed =
        DirectExportSeed::new(module_origin(), vec![binding], nominal_origins.clone());

    let mut receiver_catalog = ReceiverMethodCatalog::default();
    receiver_catalog
        .by_function_path
        .insert(method_fn_path.clone(), entry);
    let projection_input = AstPublicInterfaceProjectionInput {
        root_table,
        trait_roots: vec![],
        trait_environment: Some(std::rc::Rc::new(TraitEnvironment::new())),
        trait_evidence_environment: Some(std::rc::Rc::new(TraitEvidenceEnvironment::new())),
        const_templates_by_name: FxHashMap::default(),
    };

    // Build a generic function template for the receiver method, using the same generic
    // parameter list as the receiver nominal so the aliasing step sees matching parameters.
    let template = GenericFunctionTemplate {
        function_path: method_fn_path.clone(),
        source_file: InternedPath::new(),
        declaration_identity: None,
        generic_parameter_owner: None,
        generic_parameter_list_id: list_id,
        signature: method_signature,
        body_tokens: Some(FileTokens::new(method_fn_path.clone(), vec![])),
        declaration_location: SourceLocation::default(),
    };
    let template_map: FxHashMap<InternedPath, GenericFunctionTemplate> =
        [(method_fn_path.clone(), template)].into_iter().collect();

    let registry = ExternalPackageRegistry::new();
    let build_result = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: &nominal_origins,
        public_source_trait_origins: &FxHashMap::default(),
        type_environment: &env,
        external_registry: &registry,
        string_table: &string_table,
        generic_function_templates: &template_map,
        module_constants: &[],
    })
    .build()
    .expect("generic receiver path should build");

    assert!(
        build_result.function_origin_seeds.is_empty(),
        "a generic receiver template must not seed a local HIR FunctionId"
    );
    assert_eq!(build_result.callable_seeds.len(), 1);
    assert_eq!(
        build_result.callable_seeds[0].path, method_fn_path,
        "the generic callable seed retains the exact donor declaration path"
    );
    assert_eq!(
        build_result.callable_seeds[0].origin, method_origin,
        "the generic callable seed retains the exact stable receiver origin"
    );
    assert!(build_result.callable_seeds[0].generic_template);

    let draft = build_result
        .draft
        .finalize_after_borrow_validation(&BorrowAnalysis::default(), &HirModule::new())
        .expect("generic receiver category should survive direct-draft finalization");
    let PublicDeclarationSemantics::Struct(receiver) = &draft.draft.declarations[0].semantics
    else {
        panic!("expected a struct declaration record");
    };
    let method = &receiver.receiver_methods[0];
    assert!(matches!(
        method.category,
        PublicReceiverMethodCategory::GenericTemplate
    ));
    assert_eq!(method.method_origin, method_origin);
    assert_eq!(method.parameters.len(), 1);
    assert_eq!(method.parameters[0].name.as_deref(), Some("this"));
    assert_eq!(
        method.parameters[0].access,
        PublicCallParameterAccess::Mutable,
        "an aligned generic receiver retains declared access without a base concrete summary"
    );
    assert!(draft.concrete_call_summaries.is_empty());
}

#[test]
fn module_origin_survives_empty_public_surface() {
    let string_table = StringTable::new();
    let env = TypeEnvironment::new();

    let export_seed = DirectExportSeed::new(module_origin(), vec![], FxHashMap::default());

    let projection_input = AstPublicInterfaceProjectionInput {
        root_table: ResolvedPublicTypeRootTable::default(),
        trait_roots: vec![],
        trait_environment: Some(std::rc::Rc::new(TraitEnvironment::new())),
        trait_evidence_environment: Some(std::rc::Rc::new(TraitEvidenceEnvironment::new())),
        const_templates_by_name: FxHashMap::default(),
    };

    let registry = ExternalPackageRegistry::new();
    let draft = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: &FxHashMap::default(),
        public_source_trait_origins: &FxHashMap::default(),
        type_environment: &env,
        external_registry: &registry,
        string_table: &string_table,
        generic_function_templates: &FxHashMap::default(),
        module_constants: &[],
    })
    .build()
    .expect("empty-surface draft builds")
    .draft;

    assert_eq!(draft.module_origin, module_origin());
    assert!(draft.export_bindings.is_empty());
    assert!(draft.declarations.is_empty());
}
// ---------------------------------------------------------------------------
//  Receiver-seed rejection
// ---------------------------------------------------------------------------

#[test]
fn receiver_method_semantics_from_seed_rejects_free_function_seed_without_panic() {
    let mut string_table = StringTable::new();
    let seed = CallableSeed {
        path: InternedPath::from_single_str("free_fn", &mut string_table),
        origin: free_function_origin("free_fn"),
        generic_template: false,
        kind: CallableSeedKind::FreeFunction,
    };
    let signatures = FxHashMap::default();
    let result = receiver_method_semantics_from_seed(&seed, &signatures);
    assert!(
        result.is_err(),
        "a free-function seed must not produce receiver-method semantics"
    );
}

#[test]
fn receiver_method_semantics_from_seed_rejects_missing_signature_without_panic() {
    let mut string_table = StringTable::new();
    let origin = struct_origin("Counter");
    let seed = CallableSeed {
        path: InternedPath::from_single_str("tick", &mut string_table),
        origin: OriginFunctionId::new_receiver(module_origin(), "tick".to_owned(), origin),
        generic_template: false,
        kind: CallableSeedKind::ReceiverMethod {
            receiver_origin: struct_origin("Counter"),
            method_index: 99,
        },
    };
    let signatures = FxHashMap::default();
    let result = receiver_method_semantics_from_seed(&seed, &signatures);
    assert!(
        result.is_err(),
        "a receiver-method seed with no projected signature must be a CompilerError"
    );
}

// ---------------------------------------------------------------------------
//  Default retention tests (R2c)
// ---------------------------------------------------------------------------

#[test]
fn free_function_retains_folded_parameter_defaults_in_authored_order() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();
    let int_id = env.builtins().int;
    let string_id = env.builtins().string;

    // Every default expression carries its declared TypeId so the projection reads the
    // correct canonical type identity from the expression, not from a global builtin
    // constant that may differ from the environment.
    let parameters = vec![
        field_declaration_with_default(
            "prefix",
            string_id,
            Expression::string_slice(
                string_table.intern("default-prefix"),
                SourceLocation::default(),
                ValueMode::ImmutableOwned,
            ),
            &mut string_table,
        ),
        field_declaration_with_default(
            "count",
            int_id,
            Expression::int(42, SourceLocation::default(), ValueMode::ImmutableOwned),
            &mut string_table,
        ),
        field_declaration_no_default("subject", string_id, &mut string_table),
    ];

    let signature = FunctionSignature {
        parameters,
        returns: vec![],
    };

    let root = function_root("render", signature, &mut string_table);
    let root_table = ResolvedPublicTypeRootTable {
        roots: vec![root],
        receiver_methods: vec![],
        trait_source_facts: FxHashMap::default(),
    };

    let binding = ExportBinding::new(
        module_origin(),
        "render".to_owned(),
        OriginDeclarationId::Function(free_function_origin("render")),
    );

    let export_seed = DirectExportSeed::new(module_origin(), vec![binding], FxHashMap::default());

    let projection_input = AstPublicInterfaceProjectionInput {
        root_table,
        trait_roots: vec![],
        trait_environment: Some(std::rc::Rc::new(TraitEnvironment::new())),
        trait_evidence_environment: Some(std::rc::Rc::new(TraitEvidenceEnvironment::new())),
        const_templates_by_name: FxHashMap::default(),
    };

    let registry = ExternalPackageRegistry::new();
    let draft = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: &FxHashMap::default(),
        public_source_trait_origins: &FxHashMap::default(),
        type_environment: &env,
        external_registry: &registry,
        string_table: &string_table,
        generic_function_templates: &FxHashMap::default(),
        module_constants: &[],
    })
    .build()
    .expect("draft with function defaults should build")
    .draft;

    assert_eq!(draft.declarations.len(), 1);
    let record = &draft.declarations[0];
    let PublicDeclarationSemantics::Function(semantics) = &record.semantics else {
        panic!("expected a function record");
    };

    assert_eq!(semantics.parameters.len(), 3);

    assert_eq!(semantics.parameters[0].name.as_deref(), Some("prefix"));
    assert_eq!(
        &semantics.parameters[0].folded_default,
        &Some(PublicFoldedValue::String("default-prefix".to_owned()))
    );

    assert_eq!(semantics.parameters[1].name.as_deref(), Some("count"));
    assert_eq!(
        &semantics.parameters[1].folded_default,
        &Some(PublicFoldedValue::Int(42))
    );

    assert_eq!(semantics.parameters[2].name.as_deref(), Some("subject"));
    assert_eq!(&semantics.parameters[2].folded_default, &None);
}

#[test]
fn struct_retains_folded_field_defaults_in_authored_order() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let int_id = env.builtins().int;
    let bool_id = env.builtins().bool;
    let string_id = env.builtins().string;

    let field_defs = Box::new([
        field_def("x", int_id, &mut string_table),
        field_def("flag", bool_id, &mut string_table),
        field_def("label", string_id, &mut string_table),
    ]);

    let (_, struct_type_id) =
        register_struct(&mut env, &mut string_table, "Point", field_defs, None);

    let fields = vec![
        field_declaration_with_default(
            "x",
            int_id,
            Expression::int(10, SourceLocation::default(), ValueMode::ImmutableOwned),
            &mut string_table,
        ),
        field_declaration_with_default(
            "flag",
            bool_id,
            Expression::bool(true, SourceLocation::default(), ValueMode::ImmutableOwned),
            &mut string_table,
        ),
        field_declaration_no_default("label", string_id, &mut string_table),
    ];

    let root = struct_root("Point", struct_type_id, fields, &mut string_table);
    let root_table = ResolvedPublicTypeRootTable {
        roots: vec![root],
        receiver_methods: vec![],
        trait_source_facts: FxHashMap::default(),
    };

    let binding = ExportBinding::new(
        module_origin(),
        "Point".to_owned(),
        OriginDeclarationId::Type(struct_origin("Point")),
    );

    let nominal_origins =
        nominal_origins_map(vec![("Point", struct_origin("Point"))], &mut string_table);

    let export_seed =
        DirectExportSeed::new(module_origin(), vec![binding], nominal_origins.clone());

    let projection_input = AstPublicInterfaceProjectionInput {
        root_table,
        trait_roots: vec![],
        trait_environment: Some(std::rc::Rc::new(TraitEnvironment::new())),
        trait_evidence_environment: Some(std::rc::Rc::new(TraitEvidenceEnvironment::new())),
        const_templates_by_name: FxHashMap::default(),
    };

    let registry = ExternalPackageRegistry::new();
    let draft = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: &nominal_origins,
        public_source_trait_origins: &FxHashMap::default(),
        type_environment: &env,
        external_registry: &registry,
        string_table: &string_table,
        generic_function_templates: &FxHashMap::default(),
        module_constants: &[],
    })
    .build()
    .expect("draft with struct field defaults should build")
    .draft;

    assert_eq!(draft.declarations.len(), 1);
    let record = &draft.declarations[0];
    let PublicDeclarationSemantics::Struct(semantics) = &record.semantics else {
        panic!("expected a struct record");
    };

    assert_eq!(semantics.fields.len(), 3);

    assert_eq!(semantics.fields[0].name.as_str(), "x");
    assert_eq!(
        &semantics.fields[0].folded_default,
        &Some(PublicFoldedValue::Int(10))
    );

    assert_eq!(semantics.fields[1].name.as_str(), "flag");
    assert_eq!(
        &semantics.fields[1].folded_default,
        &Some(PublicFoldedValue::Bool(true))
    );

    assert_eq!(semantics.fields[2].name.as_str(), "label");
    assert_eq!(&semantics.fields[2].folded_default, &None);
}

#[test]
fn choice_payload_fields_remain_default_free() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let int_id = env.builtins().int;

    let variant = ChoiceVariantDefinition {
        name: string_table.intern("Some"),
        tag: 0,
        payload: ChoiceVariantPayloadDefinition::Record {
            fields: Box::new([FieldDefinition {
                name: path("value", &mut string_table),
                type_id: int_id,
                location: SourceLocation::default(),
            }]),
        },
        location: SourceLocation::default(),
    };

    let choice_path = path("Option", &mut string_table);
    env.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path: choice_path,
        variants: Box::new([variant]),
        generic_parameters: None,
    });

    let choice_type_id = env
        .type_id_for_nominal_id(NominalTypeId(0))
        .expect("choice must have a TypeId");

    let root = choice_root("Option", choice_type_id, &mut string_table);
    let root_table = ResolvedPublicTypeRootTable {
        roots: vec![root],
        receiver_methods: vec![],
        trait_source_facts: FxHashMap::default(),
    };

    let binding = ExportBinding::new(
        module_origin(),
        "Option".to_owned(),
        OriginDeclarationId::Type(choice_origin("Option")),
    );

    let nominal_origins =
        nominal_origins_map(vec![("Option", choice_origin("Option"))], &mut string_table);

    let export_seed =
        DirectExportSeed::new(module_origin(), vec![binding], nominal_origins.clone());

    let projection_input = AstPublicInterfaceProjectionInput {
        root_table,
        trait_roots: vec![],
        trait_environment: Some(std::rc::Rc::new(TraitEnvironment::new())),
        trait_evidence_environment: Some(std::rc::Rc::new(TraitEvidenceEnvironment::new())),
        const_templates_by_name: FxHashMap::default(),
    };

    let registry = ExternalPackageRegistry::new();
    let draft = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: &nominal_origins,
        public_source_trait_origins: &FxHashMap::default(),
        type_environment: &env,
        external_registry: &registry,
        string_table: &string_table,
        generic_function_templates: &FxHashMap::default(),
        module_constants: &[],
    })
    .build()
    .expect("draft with choice should build")
    .draft;

    let record = &draft.declarations[0];
    let PublicDeclarationSemantics::Choice(semantics) = &record.semantics else {
        panic!("expected a choice record");
    };

    assert_eq!(semantics.variants.len(), 1);
    let variant = &semantics.variants[0];
    assert_eq!(variant.payload_fields.len(), 1);
    assert_eq!(variant.payload_fields[0].name.as_str(), "value");
    assert_eq!(&variant.payload_fields[0].folded_default, &None);
}

#[test]
fn receiver_method_retains_folded_parameter_defaults() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let string_id = env.builtins().string;

    let (_, struct_type_id) =
        register_struct(&mut env, &mut string_table, "Counter", empty_fields(), None);

    let receiver_path = path("Counter", &mut string_table);
    let method_fn_path = path("render", &mut string_table);

    // The first parameter is the `this` receiver: it carries the struct TypeId and has no
    // default. The second parameter is an ordinary parameter with a folded default.
    let signature = FunctionSignature {
        parameters: vec![
            field_declaration_no_default("this", struct_type_id, &mut string_table),
            field_declaration_with_default(
                "label",
                string_id,
                Expression::string_slice(
                    string_table.intern("fallback"),
                    SourceLocation::default(),
                    ValueMode::ImmutableOwned,
                ),
                &mut string_table,
            ),
        ],
        returns: vec![],
    };

    let entry = receiver_entry(
        method_fn_path.clone(),
        ReceiverKey::Struct(receiver_path.clone()),
        signature,
    );

    let root = struct_root("Counter", struct_type_id, vec![], &mut string_table);
    let root_table = ResolvedPublicTypeRootTable {
        roots: vec![root],
        receiver_methods: vec![entry.clone()],
        trait_source_facts: FxHashMap::default(),
    };

    let binding = ExportBinding::new(
        module_origin(),
        "Counter".to_owned(),
        OriginDeclarationId::Type(struct_origin("Counter")),
    );

    let nominal_origins = nominal_origins_map(
        vec![("Counter", struct_origin("Counter"))],
        &mut string_table,
    );

    let export_seed =
        DirectExportSeed::new(module_origin(), vec![binding], nominal_origins.clone());

    let mut receiver_catalog = ReceiverMethodCatalog::default();
    receiver_catalog
        .by_function_path
        .insert(method_fn_path, entry);

    let projection_input = AstPublicInterfaceProjectionInput {
        root_table,
        trait_roots: vec![],
        trait_environment: Some(std::rc::Rc::new(TraitEnvironment::new())),
        trait_evidence_environment: Some(std::rc::Rc::new(TraitEvidenceEnvironment::new())),
        const_templates_by_name: FxHashMap::default(),
    };

    let registry = ExternalPackageRegistry::new();
    let draft = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: &nominal_origins,
        public_source_trait_origins: &FxHashMap::default(),
        type_environment: &env,
        external_registry: &registry,
        string_table: &string_table,
        generic_function_templates: &FxHashMap::default(),
        module_constants: &[],
    })
    .build()
    .expect("draft with receiver method defaults should build")
    .draft;

    let record = &draft.declarations[0];
    let PublicDeclarationSemantics::Struct(semantics) = &record.semantics else {
        panic!("expected a struct record");
    };

    assert_eq!(semantics.receiver_methods.len(), 1);
    let method = &semantics.receiver_methods[0];

    // The receiver slot remains default-free and authored order is preserved.
    assert_eq!(method.parameters.len(), 2);

    assert_eq!(method.parameters[0].name.as_deref(), Some("this"));
    assert_eq!(&method.parameters[0].folded_default, &None);

    assert_eq!(method.parameters[1].name.as_deref(), Some("label"));
    assert_eq!(
        &method.parameters[1].folded_default,
        &Some(PublicFoldedValue::String("fallback".to_owned()))
    );
}
