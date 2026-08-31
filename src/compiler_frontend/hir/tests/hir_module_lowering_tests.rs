//! HIR module-level lowering regression tests.
//!
//! WHAT: checks how top-level declarations, doc fragments, and templates lower into HIR module
//!       structure.
//! WHY: module lowering defines the global HIR shape that backends traverse; regressions here
//!      affect code generation and symbol emission.

use crate::compiler_frontend::ast::ast_nodes::{Declaration, NodeKind};
use crate::compiler_frontend::ast::const_values::store::{ConstStringPiece, ConstStringValue};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::expression_types::ConstValueKind;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::templates::template::TemplateType;
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff,
};
use crate::compiler_frontend::ast::{Ast, AstDocFragment, AstDocFragmentKind};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::constants::{HirConstField, HirConstValue};
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
use crate::compiler_frontend::hir::functions::{HirFunctionOrigin, HirFunctionOriginLookup};
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
    function_node, make_test_variable, node, test_source_location,
};
use crate::compiler_frontend::tests::hir_fixture_support::raw_template_expression_for_hir_invariant;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use std::path::Path;
use std::{cell::RefCell, rc::Rc};

use crate::compiler_frontend::value_mode::ValueMode;

fn add_test_module_constant(ast: &mut Ast, declaration: Declaration) {
    let type_environment = ast.type_environment.clone();
    ast.const_values
        .insert_test_declaration(declaration, &type_environment);
}

use crate::compiler_frontend::hir::hir_builder::{
    build_ast_with_registered_types, expressions_to_owned_render_node,
    expressions_to_owned_render_node_with_resources, fixture_resource, lower_ast,
    lower_ast_with_metadata, lower_module,
};
use crate::compiler_frontend::tests::type_id_fixture_support::{
    inferred_type_reference_expr, no_value_expr,
};

#[test]
fn registers_declarations_and_resolves_start_function() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let struct_name = super::symbol("MyStruct", &mut string_table);
    let field_name = struct_name.append(string_table.intern("field"));

    let struct_node = node(
        NodeKind::StructDefinition(
            struct_name,
            vec![make_test_variable(
                field_name,
                no_value_expr(
                    builtin_type_ids::INT,
                    test_source_location(1),
                    ValueMode::ImmutableOwned,
                ),
            )],
        ),
        test_source_location(1),
    );

    let start_function = function_node(
        start_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_source_location(2),
    );

    let ast = build_ast_with_registered_types(vec![struct_node, start_function], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    assert_eq!(module.structs.len(), 1);
    assert_eq!(module.functions.len(), 1);
    assert_eq!(
        module
            .side_table
            .function_name_path(
                module
                    .start_function
                    .expect("normal test module should have start"),
            )
            .cloned(),
        Some(start_name)
    );
}

#[test]
fn api_only_root_roles_lower_without_implicit_start() {
    for root_role in [
        ModuleRootRole::Support,
        ModuleRootRole::ProjectPackageFacade,
    ] {
        let mut string_table = StringTable::new();
        let entry_path = super::symbol("api.moth", &mut string_table);
        let declaration_path = super::symbol("exported_value", &mut string_table);
        let declaration = function_node(
            declaration_path,
            FunctionSignature {
                parameters: vec![],
                returns: vec![],
            },
            vec![],
            test_source_location(1),
        );
        let mut ast = build_ast_with_registered_types(vec![declaration], entry_path);
        ast.root_role = root_role;

        let (module, _type_environment) =
            lower_ast(ast, &mut string_table).expect("API-only HIR lowering should succeed");

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
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_source_location(1),
    );

    let mut ast = build_ast_with_registered_types(vec![start_function], entry_path);
    let const_name = super::symbol("SITE_NAME", &mut string_table);
    add_test_module_constant(
        &mut ast,
        make_test_variable(
            const_name,
            Expression::string_slice(
                string_table.intern("Moth"),
                test_source_location(1),
                ValueMode::ImmutableOwned,
            ),
        ),
    );

    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");
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
    let (ast, string_table) = parse_single_file_ast(source);
    let helper = ast
        .const_values
        .iter_module_constant_views()
        .find(|row| row.path.name_str(&string_table) == Some("stored_title"))
        .expect("slot-insert helper should be retained in the AST store");
    let helper_name = helper.path.to_string(&string_table);
    let helper_metadata = helper.metadata;
    assert_eq!(
        helper_metadata.value_kind,
        ConstValueKind::SlotInsertTemplate
    );
    assert!(!helper_metadata.hir_visible);

    let mut string_table = string_table;
    let (module, _) = lower_ast(ast, &mut string_table)
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
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let third_const = super::symbol("third_const", &mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::ExpressionStatement(inferred_type_reference_expr(
                third_const.clone(),
                builtin_type_ids::INT,
                test_source_location(2),
                ValueMode::ImmutableReference,
            )),
            test_source_location(2),
        )],
        test_source_location(1),
    );

    let mut ast = build_ast_with_registered_types(vec![start_function], entry_path);
    add_test_module_constant(
        &mut ast,
        make_test_variable(
            third_const,
            Expression::int(3, test_source_location(1), ValueMode::ImmutableOwned),
        ),
    );

    let (module, _type_environment) = lower_ast(ast, &mut string_table)
        .expect("start function should lower when referencing a module constant");

    let start_fn = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start_fn.entry.0 as usize];

    assert!(
        entry_block.statements.iter().any(|statement| matches!(
            statement.kind,
            HirStatementKind::Expr(ref value)
                if matches!(value.kind, HirExpressionKind::Int(3))
        )),
        "expected constant reference to lower into a usable expression in start body"
    );
}

#[test]
fn rejects_unmaterialized_template_constants_in_hir_module_constant_lowering() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_source_location(1),
    );

    let (template_constant, _template_registry) = raw_template_expression_for_hir_invariant(
        TemplateType::String,
        test_source_location(2),
        ValueMode::ImmutableOwned,
    );

    let mut ast = build_ast_with_registered_types(vec![start_function], entry_path);
    add_test_module_constant(
        &mut ast,
        make_test_variable(
            super::symbol("WRAPPER", &mut string_table),
            template_constant,
        ),
    );

    let error =
        lower_ast(ast, &mut string_table).expect_err("template constants should fail in HIR");
    let (_error_type, message, _location) = error
        .first_infrastructure_error_for_tests()
        .expect("HIR lowering failure should be wrapped for rendering");
    assert!(message.contains(
        "Template constant reached HIR module-constant lowering before AST materialized it.",
    ));
}

#[test]
fn rejects_nested_unmaterialized_template_constants_in_hir_module_constant_lowering() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_source_location(1),
    );

    let (template_constant, _template_registry) = raw_template_expression_for_hir_invariant(
        TemplateType::String,
        test_source_location(2),
        ValueMode::ImmutableOwned,
    );

    let page_const_name = super::symbol("PAGE", &mut string_table);
    let body_field = page_const_name.append(string_table.intern("body"));

    let mut ast = build_ast_with_registered_types(vec![start_function], entry_path);
    add_test_module_constant(
        &mut ast,
        make_test_variable(
            page_const_name,
            Expression::struct_instance(
                super::symbol("Page", &mut string_table),
                vec![make_test_variable(body_field, template_constant)],
                test_source_location(2),
                ValueMode::ImmutableOwned,
                true,
                None,
                builtin_type_ids::NONE,
            ),
        ),
    );

    let error =
        lower_ast(ast, &mut string_table).expect_err("nested template constants should fail");
    let (_error_type, message, _location) = error
        .first_infrastructure_error_for_tests()
        .expect("HIR lowering failure should be wrapped for rendering");
    assert!(message.contains(
        "Template constant reached HIR module-constant lowering before AST materialized it.",
    ));
}

#[test]
fn template_folded_piece_bearing_module_constant_lowers_into_pool_pieces() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_source_location(1),
    );

    // The fold fixture supplies the pieces a wrapper-template finalization would fold, while
    // the row keeps the wrapper footprint the production store writes for such constants.
    let mut resources = ModuleResourceTable::new();
    let logo = fixture_resource_id(&mut resources, "assets/logo.svg");
    let prefix = string_table.intern("docs/");
    let suffix = string_table.intern(".svg");

    let (template_constant, _template_registry) = raw_template_expression_for_hir_invariant(
        TemplateType::String,
        test_source_location(2),
        ValueMode::ImmutableOwned,
    );
    let folded = ConstStringValue::Pieces(vec![
        ConstStringPiece::Text(prefix),
        ConstStringPiece::Resource(logo),
        ConstStringPiece::Text(suffix),
    ]);

    let mut ast = build_ast_with_registered_types(vec![start_function], entry_path);
    let type_environment = ast.type_environment.clone();
    ast.const_values.insert_test_template_fold(
        make_test_variable(
            super::symbol("DOCS_URL", &mut string_table),
            template_constant,
        ),
        folded,
        &type_environment,
    );

    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("folded template constant should lower");
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
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let struct_name = super::symbol("Point", &mut string_table);
    let x_field = struct_name.append(string_table.intern("x"));
    let y_field = struct_name.append(string_table.intern("y"));

    let struct_node = node(
        NodeKind::StructDefinition(
            struct_name,
            vec![
                make_test_variable(
                    x_field.clone(),
                    no_value_expr(
                        builtin_type_ids::INT,
                        test_source_location(1),
                        ValueMode::ImmutableOwned,
                    ),
                ),
                make_test_variable(
                    y_field.clone(),
                    no_value_expr(
                        builtin_type_ids::INT,
                        test_source_location(1),
                        ValueMode::ImmutableOwned,
                    ),
                ),
            ],
        ),
        test_source_location(1),
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_source_location(2),
    );

    let mut ast = build_ast_with_registered_types(vec![struct_node, start_function], entry_path);
    let const_name = super::symbol("POINT", &mut string_table);

    add_test_module_constant(
        &mut ast,
        make_test_variable(
            const_name,
            Expression::struct_instance(
                super::symbol("Point", &mut string_table),
                vec![
                    make_test_variable(
                        x_field,
                        Expression::int(5, test_source_location(2), ValueMode::ImmutableOwned),
                    ),
                    make_test_variable(
                        y_field,
                        Expression::int(99, test_source_location(2), ValueMode::ImmutableOwned),
                    ),
                ],
                test_source_location(2),
                ValueMode::ImmutableOwned,
                true,
                None,
                builtin_type_ids::NONE,
            ),
        ),
    );

    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");
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
            assert!(matches!(fields[0].value, HirConstValue::Int(5)));
            let second_field_name = fields[1]
                .name
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(fields[1].name.as_str());
            assert_eq!(second_field_name, "y");
            assert!(matches!(fields[1].value, HirConstValue::Int(99)));
        }
        other => panic!("expected record constant, got {other:?}"),
    }
}

#[test]
fn extracts_ast_doc_fragments_into_module_metadata() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let first_doc = string_table.intern("First doc");
    let second_doc = string_table.intern("Second doc");

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_source_location(1),
    );

    let mut ast = build_ast_with_registered_types(vec![start_function], entry_path);
    ast.doc_fragments = vec![
        AstDocFragment {
            kind: AstDocFragmentKind::Doc,
            value: first_doc,
            location: test_source_location(4),
        },
        AstDocFragment {
            kind: AstDocFragmentKind::Doc,
            value: second_doc,
            location: test_source_location(7),
        },
    ];

    let lowering =
        lower_ast_with_metadata(ast, &mut string_table).expect("HIR lowering should succeed");
    let doc_fragments = &lowering.metadata.doc_fragments;
    assert_eq!(doc_fragments.len(), 2);
    assert!(matches!(doc_fragments[0].kind, ModuleDocFragmentKind::Doc));
    assert!(matches!(doc_fragments[1].kind, ModuleDocFragmentKind::Doc));
    assert_eq!(doc_fragments[0].rendered_text, "First doc");
    assert_eq!(doc_fragments[1].rendered_text, "Second doc");
    assert_eq!(doc_fragments[0].location.start_pos.line_number, 4);
    assert_eq!(doc_fragments[1].location.start_pos.line_number, 7);
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
        test_source_location(1),
    )
}

fn structural_constant(
    ast: &mut Ast,
    string_table: &mut StringTable,
    name: &str,
    pieces: Vec<ConstStringPiece>,
) {
    let const_name = super::symbol(name, string_table);
    add_test_module_constant(
        ast,
        make_test_variable(
            const_name,
            Expression::structural_string(pieces, test_source_location(1)),
        ),
    );
}

#[test]
fn piece_bearing_module_constant_reaches_hir_const_pool_in_authored_order() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_source_location(1),
    );

    let mut ast = build_ast_with_registered_types(vec![start_function], entry_path);
    let mut resources = ModuleResourceTable::new();
    let logo = fixture_resource_id(&mut resources, "assets/logo.svg");
    let prefix = string_table.intern("docs/");
    let suffix = string_table.intern(".svg");
    structural_constant(
        &mut ast,
        &mut string_table,
        "LOGO",
        vec![
            ConstStringPiece::Text(prefix),
            ConstStringPiece::Resource(logo),
            ConstStringPiece::Text(suffix),
        ],
    );

    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("piece-bearing constant should lower");
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
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_source_location(1),
    );

    let mut ast = build_ast_with_registered_types(vec![start_function], entry_path);
    let prefix = string_table.intern("docs_url = ");
    let suffix = string_table.intern("docs/");
    structural_constant(
        &mut ast,
        &mut string_table,
        "DOCS_URL",
        vec![
            ConstStringPiece::Text(prefix),
            ConstStringPiece::SiteRoot,
            ConstStringPiece::Text(suffix),
        ],
    );

    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("site-root constant should lower");
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
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let logo_const = super::symbol("LOGO", &mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::ExpressionStatement(inferred_type_reference_expr(
                logo_const.clone(),
                builtin_type_ids::STRING,
                test_source_location(2),
                ValueMode::ImmutableReference,
            )),
            test_source_location(2),
        )],
        test_source_location(1),
    );

    let mut ast = build_ast_with_registered_types(vec![start_function], entry_path);
    let mut resources = ModuleResourceTable::new();
    let logo = fixture_resource_id(&mut resources, "assets/logo.svg");
    let prefix = string_table.intern("docs/");
    structural_constant(
        &mut ast,
        &mut string_table,
        "LOGO",
        vec![
            ConstStringPiece::Resource(logo),
            ConstStringPiece::Text(prefix),
        ],
    );

    let (module, _type_environment) = lower_ast(ast, &mut string_table)
        .expect("start body referencing a structural constant should lower");

    // The const-store expression lane (module-constant reference lowering) must preserve
    // pieces, not just the constant pool value.
    let start_fn = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start_fn.entry.0 as usize];

    assert!(
        entry_block.statements.iter().any(|statement| matches!(
            &statement.kind,
            HirStatementKind::Expr(value)
                if matches!(
                    &value.kind,
                    HirExpressionKind::StructuralString { pieces }
                        if pieces.as_slice() == [
                            ConstStringPiece::Resource(logo),
                            ConstStringPiece::Text(prefix),
                        ]
                )
        )),
        "expected the constant reference to lower into a structural string expression \
         with the authored pieces"
    );
}

#[test]
fn remaps_structural_module_constant_piece_text_after_table_merge() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_source_location(1),
    );

    let mut ast = build_ast_with_registered_types(vec![start_function], entry_path);
    let prefix = string_table.intern("docs/");
    structural_constant(
        &mut ast,
        &mut string_table,
        "LOGO",
        vec![ConstStringPiece::Text(prefix), ConstStringPiece::SiteRoot],
    );

    let (mut module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("structural constant should lower");

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

        HirConstValue::Int(_)
        | HirConstValue::Float(_)
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
///       accumulator `Assign`, and handoff regressions (fusing, wrong table) must be reported
///       against that piece list, not against the surrounding `StringAppend` shape.
fn structural_pieces_in_statement(statement: &HirStatement) -> Option<&[ConstStringPiece]> {
    let value = match &statement.kind {
        HirStatementKind::Assign { value, .. } | HirStatementKind::Expr(value) => value,
        _ => return None,
    };

    fn pieces_in(expression: &HirExpression) -> Option<&[ConstStringPiece]> {
        match &expression.kind {
            HirExpressionKind::StructuralString { pieces } => Some(pieces),
            HirExpressionKind::BinOp { left, right, .. } => {
                pieces_in(left).or_else(|| pieces_in(right))
            }
            _ => None,
        }
    }

    pieces_in(value)
}

#[test]
fn runtime_template_handoff_resource_piece_lowers_through_the_module_resource_table() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    // The decoy occupies row 0 of the table, so a handoff origin minted against the wrong table
    // returns an index that resolves to the decoy instead of silently aliasing the real origin.
    let mut resources = ModuleResourceTable::new();
    let _decoy = fixture_resource(&mut resources, "assets/decoy.png");
    let (logo, logo_origin) = fixture_resource(&mut resources, "assets/logo.svg");

    let prefix = string_table.intern("docs/");
    let suffix = string_table.intern(".svg");
    let structural = Expression::structural_string(
        vec![
            ConstStringPiece::Text(prefix),
            ConstStringPiece::Resource(logo),
            ConstStringPiece::Text(suffix),
        ],
        test_source_location(2),
    );
    // WHAT: the fixture mapper is the production handoff materialization mirrored, so the node
    //       crossing into HIR is the exact piece-bearing `Text` payload the handoff carries.
    let text_node =
        expressions_to_owned_render_node_with_resources(&[structural], &string_table, &resources);

    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(text_node),
        location: test_source_location(2),
    };

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::ExpressionStatement(Expression::runtime_template_handoff(
                handoff,
                ValueMode::ImmutableOwned,
            )),
            test_source_location(2),
        )],
        test_source_location(1),
    );

    let ast = build_ast_with_registered_types(vec![start_function], entry_path);
    let table = Rc::new(RefCell::new(resources));

    // Lower through the real module entry point with the issuing table installed, exactly as
    // module compilation does, so `intern_handoff_resource_origin` runs against this table.
    let module = lower_module(
        ast,
        &mut string_table,
        HirFunctionOriginLookup::default(),
        Some(Rc::clone(&table)),
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
        .find_map(structural_pieces_in_statement)
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
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let prefix = string_table.intern("docs_url = ");
    let suffix = string_table.intern("docs/");
    let structural = Expression::structural_string(
        vec![
            ConstStringPiece::Text(prefix),
            ConstStringPiece::SiteRoot,
            ConstStringPiece::Text(suffix),
        ],
        test_source_location(2),
    );
    let text_node = expressions_to_owned_render_node(&[structural], &string_table);

    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(text_node),
        location: test_source_location(2),
    };

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::ExpressionStatement(Expression::runtime_template_handoff(
                handoff,
                ValueMode::ImmutableOwned,
            )),
            test_source_location(2),
        )],
        test_source_location(1),
    );
    let ast = build_ast_with_registered_types(vec![start_function], entry_path);
    let table = Rc::new(RefCell::new(ModuleResourceTable::new()));
    let module = lower_module(
        ast,
        &mut string_table,
        HirFunctionOriginLookup::default(),
        Some(Rc::clone(&table)),
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
        .find_map(structural_pieces_in_statement)
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
