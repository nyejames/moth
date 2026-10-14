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
use crate::compiler_frontend::ast::ast_nodes::DeclarationValue;

use crate::compiler_frontend::analysis::borrow_checker::BorrowAnalysis;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::store::{ConstValueId, ConstValueStore};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, OwnedExpr,
};
use crate::compiler_frontend::ast::generic_functions::{
    GenericFunctionBody, GenericFunctionTemplate,
};
use crate::compiler_frontend::ast::statements::functions::{FunctionParameter, FunctionSignature};
use crate::compiler_frontend::ast::{
    AstPublicInterfaceProjectionInput, ReceiverMethodCatalog, ResolvedPublicTypeRoot,
    ResolvedPublicTypeRootKind, ResolvedPublicTypeRootTable,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::datatype::DataType;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, FieldDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_parameters::TypeParameterId;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, PublicFoldedValue};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::public_call_summary::PublicCallParameterAccess;
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, OriginDeclarationId, OriginFunctionId, OriginTypeCategory, OriginTypeId,
};
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::tokenizer::tokens::TestSourceTokensBuilder;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

use rustc_hash::FxHashMap;

/// One materialised `I32` fixture value; panics when the fixture literal is out of range.
fn i32_value(value: i64) -> FixedScalarValue {
    FixedScalarValue::signed(FixedScalar::I32, value).expect("I32 literal in range")
}

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
    path_fork: &mut PathInternerFork,
) -> (NominalTypeId, TypeId) {
    let path = path_fork
        .try_intern_portable_path(name, string_table)
        .expect("test path fits");
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
    path_fork: &mut PathInternerFork,
) -> ResolvedPublicTypeRoot {
    ResolvedPublicTypeRoot {
        path: path(name, string_table, path_fork),
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
    path_fork: &mut PathInternerFork,
) -> ResolvedPublicTypeRoot {
    ResolvedPublicTypeRoot {
        path: path(name, string_table, path_fork),
        kind: ResolvedPublicTypeRootKind::Choice { type_id },
    }
}

fn alias_root(
    name: &str,
    target_type_id: TypeId,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> ResolvedPublicTypeRoot {
    ResolvedPublicTypeRoot {
        path: path(name, string_table, path_fork),
        kind: ResolvedPublicTypeRootKind::TransparentAlias { target_type_id },
    }
}

fn constant_root(
    name: &str,
    type_id: TypeId,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> ResolvedPublicTypeRoot {
    ResolvedPublicTypeRoot {
        path: path(name, string_table, path_fork),
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
    default: ConstValueId,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Declaration {
    Declaration {
        id: path(name, string_table, path_fork),
        type_id,
        diagnostic_type: DataType::Inferred,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Default(default),
        binding_span: None,
        config_qualifier: None,
    }
}

fn field_declaration_no_default(
    name: &str,
    type_id: TypeId,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Declaration {
    Declaration {
        id: path(name, string_table, path_fork),
        type_id,
        diagnostic_type: DataType::Inferred,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Absent,
        binding_span: None,
        config_qualifier: None,
    }
}

/// Fixture-owned expression/content pair: projection views bind folded defaults here directly.
struct FixtureConstants {
    builder: AstExpressionBuilder,
    constants: ConstValueStore,
}

impl FixtureConstants {
    fn literal(
        &mut self,
        build: impl FnOnce(&mut AstExpressionBuilder, &mut ConstValueStore) -> OwnedExpr,
    ) -> ConstValueId {
        let root = build(&mut self.builder, &mut self.constants);
        let view = self
            .builder
            .view_owned(root)
            .expect("fixture default belongs to builder");
        let type_id = view.value_type_id().expect("fixture default has one type");
        if !matches!(
            view.operation(),
            crate::compiler_frontend::ast::expressions::expression_store::ExpressionOperationView::Constant(_)
        ) {
            let folded = crate::compiler_frontend::ast::const_eval::fold_expression_with_constants(
                root.id(),
                &mut self.builder,
                &mut self.constants,
                &mut crate::compiler_frontend::symbols::string_interning::StringTable::new(),
                &crate::compiler_frontend::datatypes::environment::TypeEnvironment::new(),
                |_| None,
            )
            .expect("fixture default folds as content");
            let crate::compiler_frontend::ast::expressions::expression_store::ExpressionOperationView::Constant(id) =
                self.builder.view(folded).expect("folded default belongs to builder").operation()
            else {
                panic!("fixture default fold must produce a bound constant occurrence");
            };
            return id;
        }
        let crate::compiler_frontend::ast::expressions::expression_store::ExpressionOperationView::Constant(id) =
            view.operation()
        else {
            unreachable!("operation matched above");
        };
        let _ = type_id;
        id
    }
}

fn field_def(
    name: &str,
    type_id: TypeId,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> FieldDefinition {
    FieldDefinition {
        name: path(name, string_table, path_fork),
        type_id,
        span: None,
    }
}
#[test]
fn builder_produces_declaration_centric_draft_covering_every_category() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut env = TypeEnvironment::new();
    let int_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    // Register a struct and a choice so the type-surface projection can resolve them.
    let (_, struct_type_id) = register_struct(
        &mut env,
        &mut string_table,
        "Counter",
        empty_fields(),
        None,
        &mut path_fork,
    );
    let (_, choice_type_id) =
        register_choice(&mut env, &mut string_table, "Status", &mut path_fork);

    // Build roots for every non-trait category.
    let function_root = function_root(
        "render",
        empty_signature(),
        &mut string_table,
        &mut path_fork,
    );
    let struct_root = struct_root(
        "Counter",
        struct_type_id,
        vec![],
        &mut string_table,
        &mut path_fork,
    );
    let choice_root = choice_root("Status", choice_type_id, &mut string_table, &mut path_fork);
    let alias_root = alias_root("I32Alias", int_id, &mut string_table, &mut path_fork);
    let constant_root = constant_root("MaxSize", int_id, &mut string_table, &mut path_fork);

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
    let trait_root = trait_root("Shape", this_id, vec![], &mut string_table, &mut path_fork);

    // Build export bindings for all six categories, in deterministic sorted order by name.
    let bindings = vec![
        ExportBinding::new(
            module_origin(),
            "Counter".to_owned(),
            OriginDeclarationId::Type(struct_origin("Counter")),
        ),
        ExportBinding::new(
            module_origin(),
            "I32Alias".to_owned(),
            OriginDeclarationId::Type(alias_origin("I32Alias")),
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
        &mut path_fork,
    );
    let trait_origins = trait_origins_map(
        vec![("Shape", trait_origin("Shape"))],
        &mut string_table,
        &mut path_fork,
    );

    let export_seed = DirectExportSeed::new(module_origin(), bindings, nominal_origins.clone());

    let projection_input = AstPublicInterfaceProjectionInput {
        root_table,
        trait_roots: vec![trait_root],
        trait_environment: Some(std::rc::Rc::new(TraitEnvironment::new())),
        trait_evidence_environment: Some(std::rc::Rc::new(TraitEvidenceEnvironment::new())),
    };

    let mut const_values = ConstValueStore::default();
    let max_size_id = const_values
        .insert_resolved(
            int_id,
            crate::compiler_frontend::ast::const_values::store::ConstValuePayload::FixedScalar(
                i32_value(256),
            ),
            None,
        )
        .expect("test module constant should be representable in the value store");
    const_values
        .bind_module_constant(
            path_fork
                .try_intern_portable_path("MaxSize", &mut string_table)
                .expect("test path fits"),
            max_size_id,
        )
        .expect("test module constant binds");

    let registry = ExternalPackageRegistry::new();
    let draft = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        path_fork: &path_fork,
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: &nominal_origins,
        public_source_trait_origins: &trait_origins,
        type_environment: &env,
        external_registry: &registry,
        string_table: &string_table,
        generic_function_templates: &FxHashMap::default(),
        const_values: &const_values,
        module_resources: None,
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

    // The constant record carries the canonical fixed-scalar identity.
    let constant_record = draft
        .declarations
        .iter()
        .find(|record| matches!(record.semantics, PublicDeclarationSemantics::Constant(_)))
        .expect("constant record exists");
    if let PublicDeclarationSemantics::Constant(semantics) = &constant_record.semantics {
        assert_eq!(
            semantics.type_identity,
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::FixedScalar(FixedScalar::I32))
        );
    }

    // The alias record carries the canonical fixed-scalar target.
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
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::FixedScalar(FixedScalar::I32))
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
    let mut path_fork = PathInternerFork::empty();
    let mut env = TypeEnvironment::new();

    let (_, struct_type_id) = register_struct(
        &mut env,
        &mut string_table,
        "Counter",
        empty_fields(),
        None,
        &mut path_fork,
    );

    let receiver_path = path("Counter", &mut string_table, &mut path_fork);
    let method_fn_path = path("render", &mut string_table, &mut path_fork);
    let signature = FunctionSignature {
        parameters: vec![],
        returns: vec![],
    };
    let entry = receiver_entry(
        method_fn_path,
        ReceiverKey::Struct(receiver_path),
        signature,
    );

    let root = struct_root(
        "Counter",
        struct_type_id,
        vec![],
        &mut string_table,
        &mut path_fork,
    );
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
        &mut path_fork,
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
    };

    let registry = ExternalPackageRegistry::new();
    let draft = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        path_fork: &path_fork,
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: &nominal_origins,
        public_source_trait_origins: &FxHashMap::default(),
        type_environment: &env,
        external_registry: &registry,
        string_table: &string_table,
        generic_function_templates: &FxHashMap::default(),
        const_values: &ConstValueStore::default(),
        module_resources: None,
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
    let mut path_fork = PathInternerFork::empty();
    let mut env = TypeEnvironment::new();

    // Register a generic parameter list with one authored parameter "A".
    let a_name = string_table.intern("A");
    let registered_list = env.register_generic_parameter_list(
        [(TypeParameterId(0), a_name)].into_iter(),
        &FxHashMap::default(),
    );
    let list_id = registered_list.list_id;

    // Register a generic struct Box<A> whose generic parameter list matches the method
    // template below.
    let (_, struct_type_id) = register_struct(
        &mut env,
        &mut string_table,
        "Box",
        empty_fields(),
        Some(list_id),
        &mut path_fork,
    );

    let receiver_path = path("Box", &mut string_table, &mut path_fork);
    let method_fn_path = path("render", &mut string_table, &mut path_fork);
    let receiver = FunctionParameter {
        id: path("this", &mut string_table, &mut path_fork),
        type_id: struct_type_id,
        diagnostic_type: DataType::Inferred,
        value_mode: ValueMode::MutableReference,
        default: None,
        binding_span: None,
    };
    let method_signature = FunctionSignature {
        parameters: vec![receiver],
        returns: vec![],
    };
    let mut entry = receiver_entry(
        method_fn_path,
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
            &mut path_fork,
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
    let nominal_origins = nominal_origins_map(
        vec![("Box", struct_origin("Box"))],
        &mut string_table,
        &mut path_fork,
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
    };

    // Build a generic function template for the receiver method, using the same generic
    // parameter list as the receiver nominal so the aliasing step sees matching parameters.
    let template = GenericFunctionTemplate {
        function_path: method_fn_path,
        source_file: PathId::ROOT,
        declaration_identity: None,
        generic_parameter_owner: None,
        generic_parameter_list_id: list_id,
        signature: FunctionSignature::default(),
        body_tokens: Some({
            let body = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT)
                .finish()
                .expect("canonical empty source fixture should finish");
            let range = body
                .full_range()
                .expect("empty canonical source should have a checked full range");
            GenericFunctionBody::source(body, range, None, method_fn_path)
                .expect("empty generic body should retain its checked range")
        }),
        declaration_span: None,
    };
    let template_map: FxHashMap<PathId, GenericFunctionTemplate> =
        [(method_fn_path, template)].into_iter().collect();

    let registry = ExternalPackageRegistry::new();
    let build_result = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        path_fork: &path_fork,
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: &nominal_origins,
        public_source_trait_origins: &FxHashMap::default(),
        type_environment: &env,
        external_registry: &registry,
        string_table: &string_table,
        generic_function_templates: &template_map,
        const_values: &ConstValueStore::default(),
        module_resources: None,
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
    let path_fork = PathInternerFork::empty();
    let string_table = StringTable::new();
    let env = TypeEnvironment::new();
    let export_seed = DirectExportSeed::new(module_origin(), vec![], FxHashMap::default());

    let projection_input = AstPublicInterfaceProjectionInput {
        root_table: ResolvedPublicTypeRootTable::default(),
        trait_roots: vec![],
        trait_environment: Some(std::rc::Rc::new(TraitEnvironment::new())),
        trait_evidence_environment: Some(std::rc::Rc::new(TraitEvidenceEnvironment::new())),
    };

    let registry = ExternalPackageRegistry::new();
    let draft = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        path_fork: &path_fork,
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: &FxHashMap::default(),
        public_source_trait_origins: &FxHashMap::default(),
        type_environment: &env,
        external_registry: &registry,
        string_table: &string_table,
        generic_function_templates: &FxHashMap::default(),
        const_values: &ConstValueStore::default(),
        module_resources: None,
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
    let mut path_fork = PathInternerFork::empty();
    let seed = CallableSeed {
        path: path_fork
            .try_intern_portable_path("free_fn", &mut string_table)
            .expect("test path fits"),
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
    let mut path_fork = PathInternerFork::empty();
    let origin = struct_origin("Counter");
    let seed = CallableSeed {
        path: path_fork
            .try_intern_portable_path("tick", &mut string_table)
            .expect("test path fits"),
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
    let mut path_fork = PathInternerFork::empty();
    let env = TypeEnvironment::new();
    let int_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let string_id = env.builtins().string;
    let default_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "project",
        "default-prefix",
    );

    // Every default expression carries its declared TypeId so the projection reads the
    // correct canonical type identity from the expression, not from a global builtin
    // constant that may differ from the environment.
    let mut fixtures = FixtureConstants {
        builder: AstExpressionBuilder::new().unwrap(),
        constants: ConstValueStore::default(),
    };
    let prefix_text = string_table.intern("default-prefix");
    let prefix_id = fixtures
        .builder
        .string_slice(
            &mut fixtures.constants,
            prefix_text,
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture default fits builder");
    let prefix_id = fixtures
        .builder
        .with_synthetic_interface_provenance(
            prefix_id,
            SyntheticInterfaceProvenance::single(default_member.clone()),
        )
        .expect("fixture provenance fits builder");
    let prefix_default = fixtures.literal(|_, _| prefix_id);
    // Literals already produce bound constant occurrences; re-resolve through the helper
    // so authored provenance stays on the folded content the projection reads.
    let prefix_default = {
        let metadata = fixtures
            .constants
            .metadata(prefix_default)
            .expect("fixture default has content")
            .clone();
        let mut metadata = metadata;
        metadata.synthetic_interface_provenance =
            SyntheticInterfaceProvenance::single(default_member.clone());
        let payload = fixtures
            .constants
            .payload(prefix_default)
            .expect("fixture default has content")
            .clone();
        fixtures
            .constants
            .insert_value(metadata, payload)
            .expect("fixture default refits store")
    };
    let count_id = fixtures
        .builder
        .fixed_scalar(
            &mut fixtures.constants,
            i32_value(42),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture default fits builder");
    let count_default = fixtures.literal(|_, _| count_id);
    let parameters = vec![
        field_declaration_with_default(
            "prefix",
            string_id,
            prefix_default,
            &mut string_table,
            &mut path_fork,
        ),
        field_declaration_with_default(
            "count",
            int_id,
            count_default,
            &mut string_table,
            &mut path_fork,
        ),
        field_declaration_no_default("subject", string_id, &mut string_table, &mut path_fork),
    ];

    let signature = FunctionSignature {
        parameters: parameters
            .into_iter()
            .map(|declaration| {
                crate::compiler_frontend::ast::statements::functions::FunctionParameter {
                    id: declaration.id,
                    type_id: declaration.type_id,
                    diagnostic_type: declaration.diagnostic_type,
                    value_mode: declaration.value_mode,
                    default: declaration.value.default(),
                    binding_span: declaration.binding_span,
                }
            })
            .collect(),
        returns: vec![],
    };

    let root = function_root("render", signature, &mut string_table, &mut path_fork);
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
    };

    let registry = ExternalPackageRegistry::new();
    let draft = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        path_fork: &path_fork,
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: &FxHashMap::default(),
        public_source_trait_origins: &FxHashMap::default(),
        type_environment: &env,
        external_registry: &registry,
        string_table: &string_table,
        generic_function_templates: &FxHashMap::default(),
        const_values: &fixtures.constants,
        module_resources: None,
    })
    .build()
    .expect("draft with function defaults should build")
    .draft;

    assert_eq!(draft.declarations.len(), 1);
    let record = &draft.declarations[0];
    let PublicDeclarationSemantics::Function(semantics) = &record.semantics else {
        panic!("expected a function record");
    };
    assert_eq!(
        record.synthetic_interface_provenance.members(),
        &[default_member],
        "default provenance belongs to the owning declaration record",
    );

    assert_eq!(semantics.parameters.len(), 3);

    assert_eq!(semantics.parameters[0].name.as_deref(), Some("prefix"));
    assert_eq!(
        &semantics.parameters[0].folded_default,
        &Some(PublicFoldedValue::String(OwnedFoldedString::Text(
            "default-prefix".to_owned(),
        )))
    );

    assert_eq!(semantics.parameters[1].name.as_deref(), Some("count"));
    assert_eq!(
        &semantics.parameters[1].folded_default,
        &Some(PublicFoldedValue::FixedScalar(i32_value(42)))
    );

    assert_eq!(semantics.parameters[2].name.as_deref(), Some("subject"));
    assert_eq!(&semantics.parameters[2].folded_default, &None);
}

#[test]
fn struct_retains_folded_field_defaults_in_authored_order() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut env = TypeEnvironment::new();
    let int_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let bool_id = env.builtins().bool;
    let string_id = env.builtins().string;

    let field_defs = Box::new([
        field_def("x", int_id, &mut string_table, &mut path_fork),
        field_def("flag", bool_id, &mut string_table, &mut path_fork),
        field_def("label", string_id, &mut string_table, &mut path_fork),
    ]);

    let (_, struct_type_id) = register_struct(
        &mut env,
        &mut string_table,
        "Point",
        field_defs,
        None,
        &mut path_fork,
    );

    let mut fixtures = FixtureConstants {
        builder: AstExpressionBuilder::new().unwrap(),
        constants: ConstValueStore::default(),
    };
    let x_id = fixtures
        .builder
        .fixed_scalar(
            &mut fixtures.constants,
            i32_value(10),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture default fits builder");
    let x_default = fixtures.literal(|_, _| x_id);
    let flag_id = fixtures
        .builder
        .bool(
            &mut fixtures.constants,
            true,
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture default fits builder");
    let flag_default = fixtures.literal(|_, _| flag_id);
    let fields = vec![
        field_declaration_with_default("x", int_id, x_default, &mut string_table, &mut path_fork),
        field_declaration_with_default(
            "flag",
            bool_id,
            flag_default,
            &mut string_table,
            &mut path_fork,
        ),
        field_declaration_no_default("label", string_id, &mut string_table, &mut path_fork),
    ];

    let root = struct_root(
        "Point",
        struct_type_id,
        fields,
        &mut string_table,
        &mut path_fork,
    );
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

    let nominal_origins = nominal_origins_map(
        vec![("Point", struct_origin("Point"))],
        &mut string_table,
        &mut path_fork,
    );

    let export_seed =
        DirectExportSeed::new(module_origin(), vec![binding], nominal_origins.clone());

    let projection_input = AstPublicInterfaceProjectionInput {
        root_table,
        trait_roots: vec![],
        trait_environment: Some(std::rc::Rc::new(TraitEnvironment::new())),
        trait_evidence_environment: Some(std::rc::Rc::new(TraitEvidenceEnvironment::new())),
    };

    let registry = ExternalPackageRegistry::new();
    let draft = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        path_fork: &path_fork,
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: &nominal_origins,
        public_source_trait_origins: &FxHashMap::default(),
        type_environment: &env,
        external_registry: &registry,
        string_table: &string_table,
        generic_function_templates: &FxHashMap::default(),
        const_values: &fixtures.constants,
        module_resources: None,
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
        &Some(PublicFoldedValue::FixedScalar(i32_value(10)))
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
    let mut path_fork = PathInternerFork::empty();
    let mut env = TypeEnvironment::new();
    let int_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    let variant = ChoiceVariantDefinition {
        name: string_table.intern("Some"),
        tag: 0,
        payload: ChoiceVariantPayloadDefinition::Record {
            fields: Box::new([FieldDefinition {
                name: path("value", &mut string_table, &mut path_fork),
                type_id: int_id,
                span: None,
            }]),
        },
        span: None,
    };

    let choice_path = path("Option", &mut string_table, &mut path_fork);
    env.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path: choice_path,
        variants: Box::new([variant]),
        generic_parameters: None,
    });

    let choice_type_id = env
        .type_id_for_nominal_id(NominalTypeId(0))
        .expect("choice must have a TypeId");

    let root = choice_root("Option", choice_type_id, &mut string_table, &mut path_fork);
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

    let nominal_origins = nominal_origins_map(
        vec![("Option", choice_origin("Option"))],
        &mut string_table,
        &mut path_fork,
    );

    let export_seed =
        DirectExportSeed::new(module_origin(), vec![binding], nominal_origins.clone());

    let projection_input = AstPublicInterfaceProjectionInput {
        root_table,
        trait_roots: vec![],
        trait_environment: Some(std::rc::Rc::new(TraitEnvironment::new())),
        trait_evidence_environment: Some(std::rc::Rc::new(TraitEvidenceEnvironment::new())),
    };

    let registry = ExternalPackageRegistry::new();
    let draft = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        path_fork: &path_fork,
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: &nominal_origins,
        public_source_trait_origins: &FxHashMap::default(),
        type_environment: &env,
        external_registry: &registry,
        string_table: &string_table,
        generic_function_templates: &FxHashMap::default(),
        const_values: &ConstValueStore::default(),
        module_resources: None,
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
    let mut path_fork = PathInternerFork::empty();
    let mut env = TypeEnvironment::new();
    let string_id = env.builtins().string;

    let (_, struct_type_id) = register_struct(
        &mut env,
        &mut string_table,
        "Counter",
        empty_fields(),
        None,
        &mut path_fork,
    );

    let receiver_path = path("Counter", &mut string_table, &mut path_fork);
    let method_fn_path = path("render", &mut string_table, &mut path_fork);

    // The first parameter is the `this` receiver: it carries the struct TypeId and has no
    // default. The second parameter is an ordinary parameter with a folded default.
    let mut fixtures = FixtureConstants {
        builder: AstExpressionBuilder::new().unwrap(),
        constants: ConstValueStore::default(),
    };
    let fallback_text = string_table.intern("fallback");
    let fallback_id = fixtures
        .builder
        .string_slice(
            &mut fixtures.constants,
            fallback_text,
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture default fits builder");
    let fallback_default = fixtures.literal(|_, _| fallback_id);
    let signature = FunctionSignature {
        parameters: vec![
            FunctionParameter {
                id: path("this", &mut string_table, &mut path_fork),
                type_id: struct_type_id,
                diagnostic_type: DataType::Inferred,
                value_mode: ValueMode::ImmutableOwned,
                default: None,
                binding_span: None,
            },
            FunctionParameter {
                id: path("label", &mut string_table, &mut path_fork),
                type_id: string_id,
                diagnostic_type: DataType::Inferred,
                value_mode: ValueMode::ImmutableOwned,
                default: Some(fallback_default),
                binding_span: None,
            },
        ],
        returns: vec![],
    };

    let entry = receiver_entry(
        method_fn_path,
        ReceiverKey::Struct(receiver_path),
        signature,
    );

    let root = struct_root(
        "Counter",
        struct_type_id,
        vec![],
        &mut string_table,
        &mut path_fork,
    );
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
        &mut path_fork,
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
    };

    let registry = ExternalPackageRegistry::new();
    let draft = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        path_fork: &path_fork,
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: &nominal_origins,
        public_source_trait_origins: &FxHashMap::default(),
        type_environment: &env,
        external_registry: &registry,
        string_table: &string_table,
        generic_function_templates: &FxHashMap::default(),
        const_values: &fixtures.constants,
        module_resources: None,
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
        &Some(PublicFoldedValue::String(OwnedFoldedString::Text(
            "fallback".to_owned(),
        )))
    );
}
