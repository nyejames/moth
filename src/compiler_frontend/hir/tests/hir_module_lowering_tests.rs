//! HIR module-level lowering regression tests.
//!
//! WHAT: checks how top-level declarations, doc fragments, and templates lower into HIR module
//!       structure.
//! WHY: module lowering defines the global HIR shape that backends traverse; regressions here
//!      affect code generation and symbol emission.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::templates::template::TemplateType;
use crate::compiler_frontend::ast::{AstDocFragment, AstDocFragmentKind};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::constants::HirConstValue;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::module_metadata::ModuleDocFragmentKind;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_node, make_test_variable, node, test_location,
};
use crate::compiler_frontend::tests::hir_fixture_support::raw_template_expression_for_hir_invariant;

use crate::compiler_frontend::value_mode::ValueMode;

use crate::compiler_frontend::hir::hir_builder::{build_ast, lower_ast, lower_ast_with_metadata};
use crate::compiler_frontend::tests::type_id_fixture_support::{no_value_expr, reference_expr};

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
                    test_location(1),
                    ValueMode::ImmutableOwned,
                ),
            )],
        ),
        test_location(1),
    );

    let start_function = function_node(
        start_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_location(2),
    );

    let ast = build_ast(vec![struct_node, start_function], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    assert_eq!(module.structs.len(), 1);
    assert_eq!(module.functions.len(), 1);
    assert_eq!(
        module
            .side_table
            .function_name_path(module.start_function)
            .cloned(),
        Some(start_name)
    );
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
        test_location(1),
    );

    let mut ast = build_ast(vec![start_function], entry_path);
    let const_name = super::symbol("SITE_NAME", &mut string_table);
    ast.module_constants.push(make_test_variable(
        const_name,
        Expression::string_slice(
            string_table.intern("Moth"),
            test_location(1),
            ValueMode::ImmutableOwned,
        ),
    ));

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
            NodeKind::ExpressionStatement(reference_expr(
                third_const.clone(),
                builtin_type_ids::INT,
                test_location(2),
                ValueMode::ImmutableReference,
            )),
            test_location(2),
        )],
        test_location(1),
    );

    let mut ast = build_ast(vec![start_function], entry_path);
    ast.module_constants.push(make_test_variable(
        third_const,
        Expression::int(3, test_location(1), ValueMode::ImmutableOwned),
    ));

    let (module, _type_environment) = lower_ast(ast, &mut string_table)
        .expect("start function should lower when referencing a module constant");

    let start_fn = &module.functions[module.start_function.0 as usize];
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
        test_location(1),
    );

    let (template_constant, _template_registry) = raw_template_expression_for_hir_invariant(
        TemplateType::String,
        test_location(2),
        ValueMode::ImmutableOwned,
    );

    let mut ast = build_ast(vec![start_function], entry_path);
    ast.module_constants.push(make_test_variable(
        super::symbol("WRAPPER", &mut string_table),
        template_constant,
    ));

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
        test_location(1),
    );

    let (template_constant, _template_registry) = raw_template_expression_for_hir_invariant(
        TemplateType::String,
        test_location(2),
        ValueMode::ImmutableOwned,
    );

    let page_const_name = super::symbol("PAGE", &mut string_table);
    let body_field = page_const_name.append(string_table.intern("body"));

    let mut ast = build_ast(vec![start_function], entry_path);
    ast.module_constants.push(make_test_variable(
        page_const_name,
        Expression::struct_instance(
            super::symbol("Page", &mut string_table),
            vec![make_test_variable(body_field, template_constant)],
            test_location(2),
            ValueMode::ImmutableOwned,
            true,
            None,
            builtin_type_ids::NONE,
        ),
    ));

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
                        test_location(1),
                        ValueMode::ImmutableOwned,
                    ),
                ),
                make_test_variable(
                    y_field.clone(),
                    no_value_expr(
                        builtin_type_ids::INT,
                        test_location(1),
                        ValueMode::ImmutableOwned,
                    ),
                ),
            ],
        ),
        test_location(1),
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_location(2),
    );

    let mut ast = build_ast(vec![struct_node, start_function], entry_path);
    let const_name = super::symbol("POINT", &mut string_table);

    ast.module_constants.push(make_test_variable(
        const_name,
        Expression::struct_instance(
            super::symbol("Point", &mut string_table),
            vec![
                make_test_variable(
                    x_field,
                    Expression::int(5, test_location(2), ValueMode::ImmutableOwned),
                ),
                make_test_variable(
                    y_field,
                    Expression::int(99, test_location(2), ValueMode::ImmutableOwned),
                ),
            ],
            test_location(2),
            ValueMode::ImmutableOwned,
            true,
            None,
            builtin_type_ids::NONE,
        ),
    ));

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
        test_location(1),
    );

    let mut ast = build_ast(vec![start_function], entry_path);
    ast.doc_fragments = vec![
        AstDocFragment {
            kind: AstDocFragmentKind::Doc,
            value: first_doc,
            location: test_location(4),
        },
        AstDocFragment {
            kind: AstDocFragmentKind::Doc,
            value: second_doc,
            location: test_location(7),
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
