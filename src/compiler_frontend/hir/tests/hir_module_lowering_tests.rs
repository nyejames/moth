//! HIR module-level lowering regression tests.
//!
//! WHAT: checks how top-level declarations, doc fragments, and templates lower into HIR module
//!       structure.
//! WHY: module lowering defines the global HIR shape that backends traverse; regressions here
//!      affect code generation and symbol emission.

use crate::compiler_frontend::ast::ast_nodes::{Declaration, DeclarationValue, NodeKind};
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstTemplateValue, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ConstructorField, ExprId,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstValueKind;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::templates::template::TemplateType;
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff,
};
use crate::compiler_frontend::ast::{AstDocFragment, AstDocFragmentKind};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{PublicConstTemplate, PublicConstTemplateKind};
use crate::compiler_frontend::hir::constants::{HirConstField, HirConstValue};
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::functions::{HirFunctionOrigin, HirFunctionOriginLookup};
use crate::compiler_frontend::hir::ids::HirValueId;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::module_metadata::ModuleDocFragmentKind;
use crate::compiler_frontend::paths::module_resources::{ModuleResourceTable, ResourceId};
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_node, make_test_variable, node, runtime_template_handoff_expr,
};
use crate::compiler_frontend::tests::hir_fixture_support::raw_template_expression_for_hir_invariant;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use std::path::Path;
use std::{cell::RefCell, rc::Rc};

use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

/// One exact `I32` literal expression.
///
/// WHY: these fixtures pin the exact `I32` width explicitly at the scalar carrier.
fn i32_literal(
    builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    value: i64,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
    value_mode: ValueMode,
) -> ExprId {
    builder
        .fixed_scalar(
            const_values,
            FixedScalarValue::signed(FixedScalar::I32, value).expect("test literal fits I32"),
            span,
            value_mode,
        )
        .expect("fixture literal fits store")
        .id()
}

fn structural_string_expr(
    builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    pieces: Vec<ConstStringPiece>,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> ExprId {
    let value = const_values
        .insert_resolved(
            crate::compiler_frontend::datatypes::ids::builtin_type_ids::STRING,
            ConstValuePayload::String(ConstStringValue::Pieces(pieces)),
            None,
        )
        .expect("fixture structural string fits store");
    builder
        .constant(
            value,
            crate::compiler_frontend::datatypes::ids::builtin_type_ids::STRING,
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture structural string fits store")
        .id()
}

fn fold_test_module_constant(
    const_values: &mut ConstValueStore,
    expressions: &AstExpressionBuilder,
    declaration: &Declaration,
    type_environment: &TypeEnvironment,
) {
    let initializer = declaration
        .value
        .initializer()
        .expect("fixture module constant has an initializer");
    let view = expressions
        .view(initializer)
        .expect("fixture constant belongs to builder");
    let mut template_builder =
        |_: &crate::compiler_frontend::ast::templates::template::Template| -> Result<
            ConstTemplateValue,
            crate::compiler_frontend::ast::const_values::store::ConstValueStoreError,
        > {
            Ok(ConstTemplateValue::Public {
                template: PublicConstTemplate {
                    kind: PublicConstTemplateKind::Wrapper,
                    pieces: Vec::new(),
                    conditional_child_wrappers: Vec::new(),
                },
                kind: ConstValueKind::TemplateWrapper,
                hir_visible: true,
                folded: None,
                provenance:
                    crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance::empty(),
            })
        };
    let value = const_values
        .ingest_view(view, type_environment, &mut template_builder)
        .expect("fixture constant folds into the const store");
    const_values
        .bind_module_constant(declaration.id, value)
        .expect("fixture constant binds once");
}

fn fold_test_template_constant(
    const_values: &mut ConstValueStore,
    expressions: &AstExpressionBuilder,
    declaration: &Declaration,
    folded: ConstStringValue,
    type_environment: &TypeEnvironment,
) {
    let initializer = declaration
        .value
        .initializer()
        .expect("fixture module constant has an initializer");
    let view = expressions
        .view(initializer)
        .expect("fixture constant belongs to builder");
    let mut template_builder =
        |_: &crate::compiler_frontend::ast::templates::template::Template| -> Result<
            ConstTemplateValue,
            crate::compiler_frontend::ast::const_values::store::ConstValueStoreError,
        > {
            Ok(ConstTemplateValue::Public {
                template: PublicConstTemplate {
                    kind: PublicConstTemplateKind::Wrapper,
                    pieces: Vec::new(),
                    conditional_child_wrappers: Vec::new(),
                },
                kind: ConstValueKind::TemplateWrapper,
                hir_visible: true,
                folded: Some(folded.clone()),
                provenance:
                    crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance::empty(),
            })
        };
    let value = const_values
        .ingest_view(view, type_environment, &mut template_builder)
        .expect("fixture template constant folds into the const store");
    const_values
        .bind_module_constant(declaration.id, value)
        .expect("fixture constant binds once");
}

/// One struct field declaration without a default value.
///
/// WHY: `NodeKind::StructDefinition` carries declarations; these fixtures pin only the field type.
fn struct_field_declaration(
    name: crate::compiler_frontend::symbols::path_interner::PathId,
    type_id: crate::compiler_frontend::datatypes::TypeId,
    binding_span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> Declaration {
    Declaration {
        id: name,
        type_id,
        diagnostic_type: DataType::Inferred,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Absent,
        binding_span,
        config_qualifier: None,
    }
}

use crate::compiler_frontend::hir::hir_builder::{
    build_ast_with_registered_types, expressions_to_owned_render_node,
    expressions_to_owned_render_node_with_resources, fixture_resource, lower_ast,
    lower_ast_with_metadata, lower_module,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::tests::ast_fixture_support::reference_expr_with_type_id;

#[test]
fn registers_declarations_and_resolves_start_function() {
    let builder = AstExpressionBuilder::new().unwrap();
    let constants = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let struct_name = super::symbol("MyStruct", &mut path_fork, &mut string_table);
    let field_name = path_fork
        .try_intern_child(struct_name, string_table.intern("field"))
        .expect("test path fits");

    let struct_node = node(
        NodeKind::StructDefinition(
            struct_name,
            vec![struct_field_declaration(
                field_name,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                None,
            )],
        ),
        None,
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![struct_node, start_function],
        entry_path,
        builder,
        constants,
        type_environment,
    );

    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("HIR lowering should succeed");

    assert_eq!(module.structs.len(), 1);
    assert_eq!(module.structs[0].fields.len(), 1);
    assert_eq!(
        module.structs[0].fields[0].ty,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    assert_eq!(module.functions.len(), 1);
    assert_eq!(
        module.side_table.function_name_path(
            module
                .start_function
                .expect("normal test module should have start"),
        ),
        Some(start_name),
    );
}

#[test]
fn api_only_root_roles_lower_without_implicit_start() {
    let mut path_fork = super::PathInternerFork::empty();
    for root_role in [
        ModuleRootRole::Support,
        ModuleRootRole::ProjectPackageFacade,
    ] {
        let mut string_table = StringTable::new();
        let entry_path = super::symbol("api.moth", &mut path_fork, &mut string_table);
        let declaration_path = super::symbol("exported_value", &mut path_fork, &mut string_table);
        let declaration = function_node(
            declaration_path,
            FunctionSignature {
                parameters: vec![],
                returns: vec![],
            },
            vec![],
            None,
        );
        let mut ast = build_ast_with_registered_types(
            vec![declaration],
            entry_path,
            AstExpressionBuilder::new().unwrap(),
            ConstValueStore::default(),
            TypeEnvironment::new(),
        );
        ast.root_role = root_role;

        let (module, _type_environment) = lower_ast(
            ast,
            &mut string_table,
            &mut path_fork,
            &super::ExternalPackageRegistry::new(),
        )
        .expect("API-only HIR lowering should succeed");

        assert_eq!(module.start_function, None);
        assert_eq!(module.functions.len(), 1);
        assert!(
            module
                .function_origins
                .values()
                .all(|origin| !matches!(origin, HirFunctionOrigin::EntryStart)),
            "API-only roots must not contain an EntryStart origin"
        );
    }
}

#[test]
fn lowers_module_constants_into_hir_const_pool() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        None,
    );

    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();
    let const_name = super::symbol("SITE_NAME", &mut path_fork, &mut string_table);
    let value = builder
        .string_slice(
            &mut constants,
            string_table.intern("Moth"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("string constant should build")
        .id();
    let declaration = make_test_variable(const_name, value, &builder);
    fold_test_module_constant(&mut constants, &builder, &declaration, &type_environment);

    let ast = build_ast_with_registered_types(
        vec![start_function],
        entry_path,
        builder,
        constants,
        type_environment,
    );

    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("HIR lowering should succeed");
    assert_eq!(module.module_constants.len(), 1);

    let constant = &module.module_constants[0];
    assert_eq!(constant.name, "SITE_NAME");
    assert!(matches!(
        constant.value,
        HirConstValue::String(ref value) if value == "Moth"
    ));
}

#[test]
fn excludes_real_slot_insert_helpers_from_hir_but_keeps_wrapper_constants_visible() {
    let source = r#"
layout #= [:<h1>[$slot("title")]</h1><p>[$slot]</p>]
stored_title #= [$insert("title"): Stored title]
rendered #= [layout: [stored_title] Body]
"#;
    let (ast, mut parsed_path_fork, mut string_table) = parse_single_file_ast(source);
    let stored_title_path = parsed_path_fork
        .try_intern_portable_path("stored_title", &mut string_table)
        .expect("helper component should fit the parsed fork");
    let helper = ast
        .const_values
        .iter_module_constant_views()
        .find(|row| parsed_path_fork.ends_with(*row.path, stored_title_path))
        .expect("slot-insert helper should be retained in the AST store");
    let helper_name =
        parsed_path_fork.render_portable(*helper.path, &string_table, &mut Vec::new());
    let helper_metadata = helper.metadata;
    assert_eq!(
        helper_metadata.value_kind,
        ConstValueKind::SlotInsertTemplate
    );
    assert!(!helper_metadata.hir_visible);

    let (module, _) = lower_ast(
        ast,
        &mut string_table,
        &mut parsed_path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("real slot-insert helper should be excluded before HIR lowering");
    assert!(
        module
            .module_constants
            .iter()
            .all(|constant| constant.name != helper_name
                && !constant.name.ends_with("/stored_title")),
        "helper-only constants must not enter the HIR constant pool"
    );
    assert!(
        module
            .module_constants
            .iter()
            .any(|constant| constant.name.ends_with("/layout") || constant.name == "layout"),
        "wrapper constants must remain visible to HIR"
    );
}

#[test]
fn start_function_can_reference_module_constant() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let third_const = super::symbol("third_const", &mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();

    let reference = reference_expr_with_type_id(
        &mut builder,
        third_const,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        None,
        ValueMode::ImmutableReference,
    );
    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::ExpressionStatement(reference), None)],
        None,
    );

    let value = i32_literal(
        &mut builder,
        &mut constants,
        3,
        None,
        ValueMode::ImmutableOwned,
    );
    let declaration = make_test_variable(third_const, value, &builder);
    fold_test_module_constant(&mut constants, &builder, &declaration, &type_environment);

    let ast = build_ast_with_registered_types(
        vec![start_function],
        entry_path,
        builder,
        constants,
        type_environment,
    );

    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("start function should lower when referencing a module constant");

    let start_fn = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start_fn.entry.0 as usize];

    assert!(
        entry_block.statements.iter().any(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Expr(value)
                    if matches!(
                        &module.expressions.expression(*value).kind,
                        HirExpressionKind::FixedScalar(literal)
                            if literal.as_i64() == Some(3)
                    )
            )
        }),
        "expected constant reference to lower into a usable expression in start body"
    );
}

#[test]
fn rejects_unmaterialized_template_constants_in_hir_module_constant_lowering() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        None,
    );

    let (template_constant, _template_registry) = raw_template_expression_for_hir_invariant(
        &mut builder,
        TemplateType::String,
        None,
        ValueMode::ImmutableOwned,
    );

    let wrapper_name = super::symbol("WRAPPER", &mut path_fork, &mut string_table);
    let declaration = make_test_variable(wrapper_name, template_constant, &builder);
    fold_test_module_constant(&mut constants, &builder, &declaration, &type_environment);

    let ast = build_ast_with_registered_types(
        vec![start_function],
        entry_path,
        builder,
        constants,
        type_environment,
    );

    let error = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect_err("template constants should fail in HIR");
    let error = error
        .infrastructure_error()
        .expect("HIR lowering failure should be wrapped for rendering");
    assert!(error.msg.contains(
        "Template constant reached HIR module-constant lowering before AST materialized it.",
    ));
}

#[test]
fn rejects_nested_unmaterialized_template_constants_in_hir_module_constant_lowering() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        None,
    );

    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();
    let (template_constant, _template_registry) = raw_template_expression_for_hir_invariant(
        &mut builder,
        TemplateType::String,
        None,
        ValueMode::ImmutableOwned,
    );

    let page_const_name = super::symbol("PAGE", &mut path_fork, &mut string_table);
    let body_field = path_fork
        .try_intern_child(page_const_name, string_table.intern("body"))
        .expect("test path fits");
    let page_value = builder
        .struct_instance(
            vec![ConstructorField {
                name: body_field,
                slot: 0,
                value: builder.own(template_constant),
            }],
            builtin_type_ids::NONE,
            None,
            ValueMode::ImmutableOwned,
            true,
        )
        .expect("struct constant should build")
        .id();
    let declaration = make_test_variable(page_const_name, page_value, &builder);
    fold_test_module_constant(&mut constants, &builder, &declaration, &type_environment);

    let ast = build_ast_with_registered_types(
        vec![start_function],
        entry_path,
        builder,
        constants,
        type_environment,
    );

    let error = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect_err("nested template constants should fail");
    let error = error
        .infrastructure_error()
        .expect("HIR lowering failure should be wrapped for rendering");
    assert!(error.msg.contains(
        "Template constant reached HIR module-constant lowering before AST materialized it.",
    ));
}

#[test]
fn template_folded_piece_bearing_module_constant_lowers_into_pool_pieces() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        None,
    );

    // The fold fixture supplies the pieces a wrapper-template finalization would fold, while
    // the row keeps the wrapper footprint the production store writes for such constants.
    let mut resources = ModuleResourceTable::new();
    let logo = fixture_resource_id(&mut resources, "assets/logo.svg");
    let prefix = string_table.intern("docs/");
    let suffix = string_table.intern(".svg");

    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();
    let (template_constant, _template_registry) = raw_template_expression_for_hir_invariant(
        &mut builder,
        TemplateType::String,
        None,
        ValueMode::ImmutableOwned,
    );
    let folded = ConstStringValue::Pieces(vec![
        ConstStringPiece::Text(prefix),
        ConstStringPiece::Resource(logo),
        ConstStringPiece::Text(suffix),
    ]);

    let declaration = make_test_variable(
        super::symbol("DOCS_URL", &mut path_fork, &mut string_table),
        template_constant,
        &builder,
    );
    fold_test_template_constant(
        &mut constants,
        &builder,
        &declaration,
        folded,
        &type_environment,
    );

    let ast = build_ast_with_registered_types(
        vec![start_function],
        entry_path,
        builder,
        constants,
        type_environment,
    );

    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("folded template constant should lower");
    assert_eq!(module.module_constants.len(), 1);

    // The template-fold arm stores the folded structural value exactly like the plain string
    // arm, so a folded wrapper constant reaches the pool with its authored piece sequence.
    let pieces = match &module.module_constants[0].value {
        HirConstValue::StructuralString { pieces } => pieces,
        other => panic!("expected a structural string constant, got {other:?}"),
    };
    match pieces.as_slice() {
        [
            ConstStringPiece::Text(before),
            ConstStringPiece::Resource(stored),
            ConstStringPiece::Text(after),
        ] => {
            assert_eq!(string_table.resolve(*before), "docs/");
            assert_eq!(string_table.resolve(*after), ".svg");
            assert_eq!(*stored, logo);
        }
        other => panic!("expected [Text, Resource, Text] pieces in authored order, got {other:?}"),
    }
}

#[test]
fn lowers_struct_module_constant_into_record_with_ordered_fields() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let struct_name = super::symbol("Point", &mut path_fork, &mut string_table);
    let x_field = path_fork
        .try_intern_child(struct_name, string_table.intern("x"))
        .expect("test path fits");
    let y_field = path_fork
        .try_intern_child(struct_name, string_table.intern("y"))
        .expect("test path fits");

    let struct_node = node(
        NodeKind::StructDefinition(
            struct_name,
            vec![
                struct_field_declaration(
                    x_field,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    None,
                ),
                struct_field_declaration(
                    y_field,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    None,
                ),
            ],
        ),
        None,
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        None,
    );

    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();
    let x_value = i32_literal(
        &mut builder,
        &mut constants,
        5,
        None,
        ValueMode::ImmutableOwned,
    );
    let y_value = i32_literal(
        &mut builder,
        &mut constants,
        99,
        None,
        ValueMode::ImmutableOwned,
    );
    let point_value = builder
        .struct_instance(
            vec![
                ConstructorField {
                    name: x_field,
                    slot: 0,
                    value: builder.own(x_value),
                },
                ConstructorField {
                    name: y_field,
                    slot: 1,
                    value: builder.own(y_value),
                },
            ],
            builtin_type_ids::NONE,
            None,
            ValueMode::ImmutableOwned,
            true,
        )
        .expect("struct constant should build")
        .id();
    let const_name = super::symbol("POINT", &mut path_fork, &mut string_table);
    let declaration = make_test_variable(const_name, point_value, &builder);
    fold_test_module_constant(&mut constants, &builder, &declaration, &type_environment);

    let ast = build_ast_with_registered_types(
        vec![struct_node, start_function],
        entry_path,
        builder,
        constants,
        type_environment,
    );

    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("HIR lowering should succeed");
    assert_eq!(module.module_constants.len(), 1);

    let constant = &module.module_constants[0];
    match &constant.value {
        HirConstValue::Record(fields) => {
            assert_eq!(fields.len(), 2);
            let first_field_name = fields[0]
                .name
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(fields[0].name.as_str());
            assert_eq!(first_field_name, "x");
            assert!(matches!(
                fields[0].value,
                HirConstValue::FixedScalar(value) if value.as_i64() == Some(5)
            ));
            let second_field_name = fields[1]
                .name
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(fields[1].name.as_str());
            assert_eq!(second_field_name, "y");
            assert!(matches!(
                fields[1].value,
                HirConstValue::FixedScalar(value) if value.as_i64() == Some(99)
            ));
        }
        other => panic!("expected record constant, got {other:?}"),
    }
}

#[test]
fn lowers_fixed_scalar_module_constant_with_exact_bits_and_type() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        None,
    );
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();

    let u64_max =
        FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).expect("U64::MAX fits U64");
    let const_name = super::symbol("BIG", &mut path_fork, &mut string_table);
    let value = builder
        .fixed_scalar(&mut constants, u64_max, None, ValueMode::ImmutableOwned)
        .expect("fixed scalar constant should build")
        .id();
    let declaration = make_test_variable(const_name, value, &builder);
    fold_test_module_constant(&mut constants, &builder, &declaration, &type_environment);

    let ast = build_ast_with_registered_types(
        vec![start_function],
        entry_path,
        builder,
        constants,
        type_environment,
    );

    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("HIR lowering should succeed");
    assert_eq!(module.module_constants.len(), 1);
    let constant = &module.module_constants[0];
    assert_eq!(
        constant.ty,
        builtin_type_ids::fixed_scalar(FixedScalar::U64)
    );
    assert!(matches!(
        constant.value,
        HirConstValue::FixedScalar(value) if value == u64_max
    ));
}

#[test]
fn extracts_ast_doc_fragments_into_module_metadata() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let first_doc = string_table.intern("First doc");
    let second_doc = string_table.intern("Second doc");

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        None,
    );

    let mut ast = build_ast_with_registered_types(
        vec![start_function],
        entry_path,
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    ast.doc_fragments = vec![
        AstDocFragment {
            kind: AstDocFragmentKind::Doc,
            value: first_doc,
            span: None,
        },
        AstDocFragment {
            kind: AstDocFragmentKind::Doc,
            value: second_doc,
            span: None,
        },
    ];

    let lowering = lower_ast_with_metadata(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("HIR lowering should succeed");
    let doc_fragments = &lowering.metadata.doc_fragments;
    assert_eq!(doc_fragments.len(), 2);
    assert!(matches!(doc_fragments[0].kind, ModuleDocFragmentKind::Doc));
    assert!(matches!(doc_fragments[1].kind, ModuleDocFragmentKind::Doc));
    assert_eq!(doc_fragments[0].rendered_text, "First doc");
    assert_eq!(doc_fragments[1].rendered_text, "Second doc");
    assert_eq!(doc_fragments[0].span, None);
    assert_eq!(doc_fragments[1].span, None);
}

/// Mint one real resource handle through the issuing module resource table.
///
/// WHAT: follows the production interning path (`intern_origin`) the piece vocabulary expects.
/// WHY: pieces carry dense module-local handles, so a fixture resource must come from the same
/// table that would resolve it in a real module.
fn fixture_resource_id(resources: &mut ModuleResourceTable, relative: &str) -> ResourceId {
    let module = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("site"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let logical_path = PortableResourcePath::from_relative_logical_path(Path::new(relative))
        .expect("relative resource path should be portable");

    resources.intern_origin(
        StableResourceOriginId::module_owned(module, logical_path),
        None,
    )
}

fn structural_constant(
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    name: &str,
    pieces: Vec<ConstStringPiece>,
) {
    let const_name = super::symbol(name, path_fork, string_table);
    let value = structural_string_expr(builder, constants, pieces, None);
    let declaration = make_test_variable(const_name, value, builder);
    fold_test_module_constant(constants, builder, &declaration, type_environment);
}

#[test]
fn piece_bearing_module_constant_reaches_hir_const_pool_in_authored_order() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        None,
    );

    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();
    let mut resources = ModuleResourceTable::new();
    let logo = fixture_resource_id(&mut resources, "assets/logo.svg");
    let prefix = string_table.intern("docs/");
    let suffix = string_table.intern(".svg");
    structural_constant(
        &mut builder,
        &mut constants,
        &type_environment,
        &mut string_table,
        &mut path_fork,
        "LOGO",
        vec![
            ConstStringPiece::Text(prefix),
            ConstStringPiece::Resource(logo),
            ConstStringPiece::Text(suffix),
        ],
    );
    let ast = build_ast_with_registered_types(
        vec![start_function],
        entry_path,
        builder,
        constants,
        type_environment,
    );

    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("piece-bearing constant should lower");
    assert_eq!(module.module_constants.len(), 1);

    let pieces = match &module.module_constants[0].value {
        HirConstValue::StructuralString { pieces } => pieces,
        other => panic!("expected a structural string constant, got {other:?}"),
    };
    // Authored order and the text-coalescing boundary around the resource piece both survive:
    // the runs beside the resource are separate interned pieces, never fused through it.
    match pieces.as_slice() {
        [
            ConstStringPiece::Text(before),
            ConstStringPiece::Resource(stored),
            ConstStringPiece::Text(after),
        ] => {
            assert!(*before == prefix);
            assert!(*after == suffix);
            assert!(*stored == logo);
        }
        other => panic!("expected [Text, Resource, Text] pieces in authored order, got {other:?}"),
    }
}

#[test]
fn site_root_module_constant_survives_hir_lowering() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        None,
    );

    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();
    let prefix = string_table.intern("docs_url = ");
    let suffix = string_table.intern("docs/");
    structural_constant(
        &mut builder,
        &mut constants,
        &type_environment,
        &mut string_table,
        &mut path_fork,
        "DOCS_URL",
        vec![
            ConstStringPiece::Text(prefix),
            ConstStringPiece::SiteRoot,
            ConstStringPiece::Text(suffix),
        ],
    );
    let ast = build_ast_with_registered_types(
        vec![start_function],
        entry_path,
        builder,
        constants,
        type_environment,
    );

    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("site-root constant should lower");
    assert_eq!(module.module_constants.len(), 1);

    let pieces = match &module.module_constants[0].value {
        HirConstValue::StructuralString { pieces } => pieces,
        other => panic!("expected a structural string constant, got {other:?}"),
    };
    // The site-root mark carries no resource identity, and the text runs beside it stay
    // separate pieces rather than merging across it.
    match pieces.as_slice() {
        [
            ConstStringPiece::Text(before),
            ConstStringPiece::SiteRoot,
            ConstStringPiece::Text(after),
        ] => {
            assert!(*before == prefix);
            assert!(*after == suffix);
        }
        other => panic!("expected [Text, SiteRoot, Text] pieces in authored order, got {other:?}"),
    }
}

#[test]
fn structural_module_constant_reference_lowers_into_structural_expression() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let logo_const = super::symbol("LOGO", &mut path_fork, &mut string_table);

    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();

    let reference = reference_expr_with_type_id(
        &mut builder,
        logo_const,
        builtin_type_ids::STRING,
        None,
        ValueMode::ImmutableReference,
    );
    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::ExpressionStatement(reference), None)],
        None,
    );

    let mut resources = ModuleResourceTable::new();
    let logo = fixture_resource_id(&mut resources, "assets/logo.svg");
    let prefix = string_table.intern("docs/");
    structural_constant(
        &mut builder,
        &mut constants,
        &type_environment,
        &mut string_table,
        &mut path_fork,
        "LOGO",
        vec![
            ConstStringPiece::Resource(logo),
            ConstStringPiece::Text(prefix),
        ],
    );
    let ast = build_ast_with_registered_types(
        vec![start_function],
        entry_path,
        builder,
        constants,
        type_environment,
    );

    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("start body referencing a structural constant should lower");

    // The const-store expression lane (module-constant reference lowering) must preserve
    // pieces, not just the constant pool value.
    let start_fn = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start_fn.entry.0 as usize];

    assert!(
        entry_block.statements.iter().any(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Expr(value)
                    if matches!(
                        &module.expressions.expression(*value).kind,
                        HirExpressionKind::StructuralString { pieces }
                            if module.expressions.string_pieces(*pieces) == [
                                ConstStringPiece::Resource(logo),
                                ConstStringPiece::Text(prefix),
                            ]
                    )
            )
        }),
        "expected the constant reference to lower into a structural string expression \
     with the authored pieces"
    );
}

#[test]
fn remaps_structural_module_constant_piece_text_after_table_merge() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        None,
    );

    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();
    let prefix = string_table.intern("docs/");
    structural_constant(
        &mut builder,
        &mut constants,
        &type_environment,
        &mut string_table,
        &mut path_fork,
        "LOGO",
        vec![ConstStringPiece::Text(prefix), ConstStringPiece::SiteRoot],
    );
    let ast = build_ast_with_registered_types(
        vec![start_function],
        entry_path,
        builder,
        constants,
        type_environment,
    );

    let (mut module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("structural constant should lower");

    // Merge the module-local table into a fresh global table whose IDs differ, as module
    // compilation does before executable output.
    let mut target_table = StringTable::new();
    target_table.intern("existing");
    let remap = target_table.merge_from(&string_table);
    assert!(
        !remap.is_identity(),
        "fixture merge must move the piece handles"
    );

    module.remap_string_ids(&remap);

    let pieces = match &module.module_constants[0].value {
        HirConstValue::StructuralString { pieces } => pieces,
        other => panic!("expected a structural string constant, got {other:?}"),
    };
    match pieces.as_slice() {
        [ConstStringPiece::Text(text), ConstStringPiece::SiteRoot] => {
            assert!(
                *text != prefix,
                "piece text handle should re-bind to the merged table"
            );
            assert_eq!(target_table.resolve(*text), "docs/");
            assert_eq!(string_table.resolve(prefix), "docs/");
        }
        other => panic!("expected [Text, SiteRoot] pieces after remap, got {other:?}"),
    }
}

/// Merges the fixture source table into a fresh table whose ID space cannot align with it.
///
/// WHAT: interning a decoy row first guarantees the merge shifts every handle, so both an
/// identity merge and a stale un-remapped handle resolve to visibly wrong text here.
/// WHY: only a non-identity merge exercises `remap_string_ids`, and the decoy keeps a missed
/// remap from landing on the authored string by index coincidence.
fn non_identity_merge_remap(source: &StringTable) -> (StringTable, StringIdRemap) {
    let mut target_table = StringTable::new();
    target_table.intern("decoy table row");

    let remap = target_table.merge_from(source);
    assert!(
        !remap.is_identity(),
        "fixture merge must move the piece handles"
    );

    (target_table, remap)
}

/// Finds the piece lists holding every structural string nested inside one constant value.
///
/// WHAT: one independent traversal over the same container set `remap_string_ids` recurses
/// into, used only to locate remapped payload lists for the nested remap assertions.
/// WHY: assertions must check the remapped payload wherever it sits, and each test's list
/// count pins this locator so a missed container cannot pass vacuously.
fn nested_structural_piece_lists<'value>(
    value: &'value HirConstValue,
    piece_lists: &mut Vec<&'value [ConstStringPiece]>,
) {
    match value {
        HirConstValue::StructuralString { pieces } => piece_lists.push(pieces),

        HirConstValue::Record(fields) | HirConstValue::Choice { fields, .. } => {
            for field in fields {
                nested_structural_piece_lists(&field.value, piece_lists);
            }
        }

        HirConstValue::Collection(values) => {
            for value in values {
                nested_structural_piece_lists(value, piece_lists);
            }
        }

        HirConstValue::Range(start, end) => {
            nested_structural_piece_lists(start, piece_lists);
            nested_structural_piece_lists(end, piece_lists);
        }

        HirConstValue::OptionSome(inner) => nested_structural_piece_lists(inner, piece_lists),

        HirConstValue::FixedScalar(_)
        | HirConstValue::Number(_)
        | HirConstValue::Bool(_)
        | HirConstValue::Char(_)
        | HirConstValue::String(_)
        | HirConstValue::OptionNone => {}
    }
}

/// Asserts one piece list re-bound to the merged table and still spells its authored text.
///
/// WHAT: resolves every remapped text piece against the merged table and compares the spelled
/// result, then checks that the handle itself moved.
/// WHY: handle inequality alone cannot catch a remap that binds to the wrong string, which is
/// the failure the nested remap tests hunt.
fn assert_remapped_piece_text(
    pieces: &[ConstStringPiece],
    originals: &[StringId],
    source_table: &StringTable,
    target_table: &StringTable,
) {
    assert_eq!(
        pieces.len(),
        originals.len(),
        "expected one text piece per authored run, got {pieces:?}"
    );

    for (piece, original) in pieces.iter().zip(originals) {
        let ConstStringPiece::Text(remapped) = piece else {
            panic!("expected a text piece, got {piece:?}");
        };

        let authored = source_table.resolve(*original);
        assert_eq!(
            target_table.resolve(*remapped),
            authored,
            "remapped piece must resolve to its authored text in the merged table"
        );
        assert_ne!(
            remapped, original,
            "piece handle should re-bind to the merged table"
        );
    }
}

#[test]
fn remaps_structural_pieces_inside_record_fields_after_table_merge() {
    let mut string_table = StringTable::new();
    let before = string_table.intern("docs/");
    let after = string_table.intern("page.html");

    let mut value = HirConstValue::Record(vec![HirConstField {
        name: "path".to_string(),
        value: HirConstValue::StructuralString {
            pieces: vec![
                ConstStringPiece::Text(before),
                ConstStringPiece::Text(after),
            ],
        },
    }]);

    let (target_table, remap) = non_identity_merge_remap(&string_table);
    value.remap_string_ids(&remap);

    let mut piece_lists = Vec::new();
    nested_structural_piece_lists(&value, &mut piece_lists);
    assert_eq!(
        piece_lists.len(),
        1,
        "the record field holds one piece list"
    );
    assert_remapped_piece_text(
        piece_lists[0],
        &[before, after],
        &string_table,
        &target_table,
    );
}

#[test]
fn remaps_structural_pieces_inside_choice_fields_after_table_merge() {
    let mut string_table = StringTable::new();
    let host = string_table.intern("https://");
    let page = string_table.intern("example.moth");

    let mut value = HirConstValue::Choice {
        tag: 0,
        fields: vec![HirConstField {
            name: "url".to_string(),
            value: HirConstValue::StructuralString {
                pieces: vec![ConstStringPiece::Text(host), ConstStringPiece::Text(page)],
            },
        }],
    };

    let (target_table, remap) = non_identity_merge_remap(&string_table);
    value.remap_string_ids(&remap);

    let mut piece_lists = Vec::new();
    nested_structural_piece_lists(&value, &mut piece_lists);
    assert_eq!(
        piece_lists.len(),
        1,
        "the choice field holds one piece list"
    );
    assert_remapped_piece_text(piece_lists[0], &[host, page], &string_table, &target_table);
}

#[test]
fn remaps_structural_pieces_inside_collection_elements_after_table_merge() {
    let mut string_table = StringTable::new();
    let before = string_table.intern("before/");
    let after = string_table.intern("after");

    let mut value = HirConstValue::Collection(vec![HirConstValue::StructuralString {
        pieces: vec![
            ConstStringPiece::Text(before),
            ConstStringPiece::Text(after),
        ],
    }]);

    let (target_table, remap) = non_identity_merge_remap(&string_table);
    value.remap_string_ids(&remap);

    let mut piece_lists = Vec::new();
    nested_structural_piece_lists(&value, &mut piece_lists);
    assert_eq!(
        piece_lists.len(),
        1,
        "the collection element holds one piece list"
    );
    assert_remapped_piece_text(
        piece_lists[0],
        &[before, after],
        &string_table,
        &target_table,
    );
}

#[test]
fn remaps_structural_pieces_inside_range_bounds_after_table_merge() {
    let mut string_table = StringTable::new();
    let start_text = string_table.intern("start/");
    let end_text = string_table.intern("end");

    let mut value = HirConstValue::Range(
        Box::new(HirConstValue::StructuralString {
            pieces: vec![ConstStringPiece::Text(start_text)],
        }),
        Box::new(HirConstValue::StructuralString {
            pieces: vec![ConstStringPiece::Text(end_text)],
        }),
    );

    let (target_table, remap) = non_identity_merge_remap(&string_table);
    value.remap_string_ids(&remap);

    let mut piece_lists = Vec::new();
    nested_structural_piece_lists(&value, &mut piece_lists);
    assert_eq!(piece_lists.len(), 2, "both bounds hold their piece lists");
    assert_remapped_piece_text(piece_lists[0], &[start_text], &string_table, &target_table);
    assert_remapped_piece_text(piece_lists[1], &[end_text], &string_table, &target_table);
}

#[test]
fn remaps_structural_pieces_inside_option_some_after_table_merge() {
    let mut string_table = StringTable::new();
    let before = string_table.intern("wrap/");
    let after = string_table.intern("inner");

    let mut value = HirConstValue::OptionSome(Box::new(HirConstValue::StructuralString {
        pieces: vec![
            ConstStringPiece::Text(before),
            ConstStringPiece::Text(after),
        ],
    }));

    let (target_table, remap) = non_identity_merge_remap(&string_table);
    value.remap_string_ids(&remap);

    let mut piece_lists = Vec::new();
    nested_structural_piece_lists(&value, &mut piece_lists);
    assert_eq!(
        piece_lists.len(),
        1,
        "the Some payload holds one piece list"
    );
    assert_remapped_piece_text(
        piece_lists[0],
        &[before, after],
        &string_table,
        &target_table,
    );
}

#[test]
fn remaps_structural_pieces_nested_two_container_levels_deep_after_table_merge() {
    let mut string_table = StringTable::new();
    let leaf_text = string_table.intern("leaf/");
    let tail_text = string_table.intern("tail");

    // Collection element -> record field -> structural string: a walker that stops one
    // container short leaves this piece list un-remapped and the text assertion fails.
    let mut value = HirConstValue::Collection(vec![HirConstValue::Record(vec![HirConstField {
        name: "path".to_string(),
        value: HirConstValue::StructuralString {
            pieces: vec![
                ConstStringPiece::Text(leaf_text),
                ConstStringPiece::Text(tail_text),
            ],
        },
    }])]);

    let (target_table, remap) = non_identity_merge_remap(&string_table);
    value.remap_string_ids(&remap);

    let mut piece_lists = Vec::new();
    nested_structural_piece_lists(&value, &mut piece_lists);
    assert_eq!(
        piece_lists.len(),
        1,
        "the two-level nest holds one piece list"
    );
    assert_remapped_piece_text(
        piece_lists[0],
        &[leaf_text, tail_text],
        &string_table,
        &target_table,
    );
}

/// Finds the first structural-string piece list inside one lowered statement.
///
/// WHAT: walks the only value shapes the runtime-template accumulator append emits, so the
///       handoff lane can assert on the structural chunk without pinning the accumulator
///       plumbing around it.
/// WHY: the linear template path appends the owned piece payload as the chunk operand of the
///       accumulator update, and handoff regressions (fusing, wrong table) must be reported
///       against that piece list, not against the surrounding `StringAppend` shape.
fn structural_pieces_in_statement<'a>(
    module: &'a crate::compiler_frontend::hir::module::HirModule,
    statement: &HirStatement,
) -> Option<&'a [ConstStringPiece]> {
    let value = match &statement.kind {
        HirStatementKind::Write { value, .. } | HirStatementKind::Expr(value) => *value,
        _ => return None,
    };

    fn pieces_in(
        module: &crate::compiler_frontend::hir::module::HirModule,
        value: HirValueId,
    ) -> Option<&[ConstStringPiece]> {
        match &module.expressions.expression(value).kind {
            HirExpressionKind::StructuralString { pieces } => {
                Some(module.expressions.string_pieces(*pieces))
            }
            HirExpressionKind::BinOp { left, right, .. } => {
                pieces_in(module, *left).or_else(|| pieces_in(module, *right))
            }
            _ => None,
        }
    }

    pieces_in(module, value)
}

#[test]
fn runtime_template_handoff_resource_piece_lowers_through_the_module_resource_table() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    // The decoy occupies row 0 of the table, so a handoff origin minted against the wrong table
    // returns an index that resolves to the decoy instead of silently aliasing the real origin.
    let mut resources = ModuleResourceTable::new();
    let _decoy = fixture_resource(&mut resources, "assets/decoy.png");
    let (logo, logo_origin) = fixture_resource(&mut resources, "assets/logo.svg");

    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let prefix = string_table.intern("docs/");
    let suffix = string_table.intern(".svg");
    let structural = structural_string_expr(
        &mut builder,
        &mut constants,
        vec![
            ConstStringPiece::Text(prefix),
            ConstStringPiece::Resource(logo),
            ConstStringPiece::Text(suffix),
        ],
        None,
    );
    // WHAT: the fixture mapper is the production handoff materialization mirrored, so the node
    //       crossing into HIR is the exact piece-bearing `Text` payload the handoff carries.
    let text_node = expressions_to_owned_render_node_with_resources(
        &builder,
        &constants,
        &[structural],
        &string_table,
        &resources,
    );

    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(text_node),
    };
    let handoff_expr =
        runtime_template_handoff_expr(&mut builder, handoff, None, ValueMode::ImmutableOwned);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::ExpressionStatement(handoff_expr), None)],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_function],
        entry_path,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let table = Rc::new(RefCell::new(resources));

    // Lower through the real module entry point with the issuing table installed, exactly as
    // module compilation does, so `intern_handoff_resource_origin` runs against this table.
    let module = lower_module(
        ast,
        &mut string_table,
        &mut path_fork,
        HirFunctionOriginLookup::default(),
        Some(Rc::clone(&table)),
        Default::default(),
        &ExternalPackageRegistry::new(),
    )
    .expect("piece-bearing template handoff should lower through the module resource table")
    .hir_module;

    let start_function_id = module
        .start_function
        .expect("normal test module should have start");
    let entry_block =
        &module.blocks[module.functions[start_function_id.0 as usize].entry.0 as usize];

    let pieces = entry_block
        .statements
        .iter()
        .find_map(|statement| structural_pieces_in_statement(&module, statement))
        .expect("template handoff should append one structural string chunk");

    match pieces {
        [
            ConstStringPiece::Text(before),
            ConstStringPiece::Resource(resource_piece),
            ConstStringPiece::Text(after),
        ] => {
            // Authored order survived: three pieces, with the runs beside the anchor separate
            // and the text re-bound through the module's own string table.
            assert_eq!(string_table.resolve(*before), "docs/");
            assert_eq!(string_table.resolve(*after), ".svg");

            // The piece handle must resolve through the installed table back to the very origin
            // the fixture minted: a wrong-table or wrong-asset regression fails exactly here.
            let stored_origin = table
                .borrow()
                .try_origin(*resource_piece)
                .expect("handoff resource piece must resolve through the module resource table")
                .origin
                .clone();
            assert_eq!(stored_origin, logo_origin);
        }
        other => panic!("expected [Text, Resource, Text] pieces in authored order, got {other:?}"),
    }
}

#[test]
fn runtime_template_handoff_site_root_piece_lowers_through_the_module_resource_table() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let prefix = string_table.intern("docs_url = ");
    let suffix = string_table.intern("docs/");
    let structural = structural_string_expr(
        &mut builder,
        &mut constants,
        vec![
            ConstStringPiece::Text(prefix),
            ConstStringPiece::SiteRoot,
            ConstStringPiece::Text(suffix),
        ],
        None,
    );
    let text_node =
        expressions_to_owned_render_node(&builder, &constants, &[structural], &string_table);

    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(text_node),
    };
    let handoff_expr =
        runtime_template_handoff_expr(&mut builder, handoff, None, ValueMode::ImmutableOwned);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::ExpressionStatement(handoff_expr), None)],
        None,
    );
    let ast = build_ast_with_registered_types(
        vec![start_function],
        entry_path,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let table = Rc::new(RefCell::new(ModuleResourceTable::new()));
    let module = lower_module(
        ast,
        &mut string_table,
        &mut path_fork,
        HirFunctionOriginLookup::default(),
        Some(Rc::clone(&table)),
        Default::default(),
        &ExternalPackageRegistry::new(),
    )
    .expect("site-root template handoff should lower through the module resource table")
    .hir_module;

    let start_function_id = module
        .start_function
        .expect("normal test module should have start");
    let entry_block =
        &module.blocks[module.functions[start_function_id.0 as usize].entry.0 as usize];

    let pieces = entry_block
        .statements
        .iter()
        .find_map(|statement| structural_pieces_in_statement(&module, statement))
        .expect("template handoff should append one structural string chunk");

    match pieces {
        [
            ConstStringPiece::Text(before),
            ConstStringPiece::SiteRoot,
            ConstStringPiece::Text(after),
        ] => {
            assert_eq!(string_table.resolve(*before), "docs_url = ");
            assert_eq!(string_table.resolve(*after), "docs/");
        }
        other => panic!("expected [Text, SiteRoot, Text] pieces in authored order, got {other:?}"),
    }
}
