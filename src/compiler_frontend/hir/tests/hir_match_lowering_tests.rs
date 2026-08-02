//! HIR match lowering regression tests.
//!
//! WHAT: checks how `match` expressions lower into HIR switch blocks with pattern arms, guards,
//!       and exhaustiveness checks.
//! WHY: match lowering generates complex multi-way branching; regressions here produce wrong
//!      control flow or missing pattern coverage silently.

use crate::compiler_frontend::ast::ast_nodes::{MatchExhaustiveness, NodeKind};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchArm, MatchPattern, RelationalPatternOp,
};
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::declaration_syntax::choice::{ChoiceVariant, ChoiceVariantPayload};
use crate::compiler_frontend::hir::expressions::{
    HirExpressionKind, HirVariantCarrier, OPTION_SOME_VARIANT_INDEX, ValueKind,
};
use crate::compiler_frontend::hir::ids::ChoiceId;
use crate::compiler_frontend::hir::patterns::HirPattern;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_node, make_test_variable, node, test_location,
};

use crate::compiler_frontend::tests::type_id_fixture_support::{
    choice_type_id, fresh_success_returns, param_with_type_id, reference_expr,
};
use crate::compiler_frontend::value_mode::ValueMode;

use crate::compiler_frontend::hir::hir_builder::{
    HirTestChoiceDefinition, assert_no_placeholder_terminators, build_ast, build_ast_with_choices,
    lower_ast,
};

#[test]
fn non_unit_function_with_terminal_match_default_does_not_report_fallthrough() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let chooser = super::symbol("choose_match", &mut string_table);
    let x = super::symbol("x", &mut string_table);

    let chooser_fn = function_node(
        chooser,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                x.clone(),
                builtin_type_ids::INT,
                false,
                test_location(10),
            )],
            returns: fresh_success_returns(vec![builtin_type_ids::INT]),
        },
        vec![node(
            NodeKind::Match {
                scrutinee: reference_expr(
                    x,
                    builtin_type_ids::INT,
                    test_location(11),
                    ValueMode::ImmutableReference,
                ),
                arms: vec![MatchArm {
                    pattern: MatchPattern::Literal(Expression::int(
                        1,
                        test_location(11),
                        ValueMode::ImmutableOwned,
                    )),
                    guard: None,
                    body: vec![node(
                        NodeKind::Return(vec![Expression::int(
                            1,
                            test_location(11),
                            ValueMode::ImmutableOwned,
                        )]),
                        test_location(11),
                    )],
                }],
                default: Some(vec![node(
                    NodeKind::Return(vec![Expression::int(
                        2,
                        test_location(12),
                        ValueMode::ImmutableOwned,
                    )]),
                    test_location(12),
                )]),
                exhaustiveness: MatchExhaustiveness::HasDefault,
            },
            test_location(11),
        )],
        test_location(10),
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_location(1),
    );

    let ast = build_ast(vec![start_fn, chooser_fn], entry_path);
    let (module, _type_environment) = lower_ast(ast, &mut string_table)
        .expect("all-terminal match arms should not trigger fallthrough");

    let chooser_block = &module.blocks[module.functions[1].entry.0 as usize];
    assert!(matches!(
        chooser_block.terminator,
        HirTerminator::Match { .. }
    ));
    assert_no_placeholder_terminators(&module);
}

#[test]
fn non_unit_function_with_exhaustive_choice_match_returns_on_all_arms() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let label_fn_name = super::symbol("label_status", &mut string_table);
    let status_path = InternedPath::from_single_str("Status", &mut string_table);
    let status_local = super::symbol("status", &mut string_table);
    let ready_name = string_table.intern("Ready");
    let busy_name = string_table.intern("Busy");

    let choice_variants = vec![
        ChoiceVariant {
            id: ready_name,
            payload: ChoiceVariantPayload::Unit,
            location: test_location(20),
        },
        ChoiceVariant {
            id: busy_name,
            payload: ChoiceVariantPayload::Unit,
            location: test_location(20),
        },
    ];

    let status_type_id = choice_type_id(status_path.clone(), &choice_variants);

    let label_fn = function_node(
        label_fn_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                status_local.clone(),
                status_type_id,
                false,
                test_location(20),
            )],
            returns: fresh_success_returns(vec![builtin_type_ids::INT]),
        },
        vec![node(
            NodeKind::Match {
                scrutinee: reference_expr(
                    status_local,
                    status_type_id,
                    test_location(21),
                    ValueMode::ImmutableReference,
                ),
                arms: vec![
                    MatchArm {
                        pattern: MatchPattern::ChoiceVariant {
                            nominal_path: status_path.clone(),
                            tag: 0,
                            captures: vec![],
                            location: test_location(22),
                        },
                        guard: None,
                        body: vec![node(
                            NodeKind::Return(vec![Expression::int(
                                1,
                                test_location(22),
                                ValueMode::ImmutableOwned,
                            )]),
                            test_location(22),
                        )],
                    },
                    MatchArm {
                        pattern: MatchPattern::ChoiceVariant {
                            nominal_path: status_path.clone(),
                            tag: 1,
                            captures: vec![],
                            location: test_location(23),
                        },
                        guard: None,
                        body: vec![node(
                            NodeKind::Return(vec![Expression::int(
                                2,
                                test_location(23),
                                ValueMode::ImmutableOwned,
                            )]),
                            test_location(23),
                        )],
                    },
                ],
                default: None,
                exhaustiveness: MatchExhaustiveness::ExhaustiveChoice,
            },
            test_location(21),
        )],
        test_location(20),
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_location(1),
    );

    let ast = build_ast_with_choices(
        vec![start_fn, label_fn],
        entry_path,
        vec![HirTestChoiceDefinition {
            nominal_path: status_path,
            variants: choice_variants,
        }],
    );
    let (module, _type_environment) = lower_ast(ast, &mut string_table)
        .expect("exhaustive choice match with all-returning arms should lower");

    let label_entry = &module.blocks[module.functions[1].entry.0 as usize];
    let arms = match &label_entry.terminator {
        HirTerminator::Match { arms, .. } => arms,
        other => panic!("expected match terminator, got {other:?}"),
    };
    assert_eq!(arms.len(), 2);
    assert!(
        arms.iter()
            .all(|arm| !matches!(arm.pattern, HirPattern::Wildcard)),
        "exhaustive choice match should not include a wildcard fallback arm"
    );
    assert_no_placeholder_terminators(&module);
}

#[test]
fn lowers_match_with_literal_arms_and_explicit_default_wildcard() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let x = super::symbol("x", &mut string_table);

    let match_node = node(
        NodeKind::Match {
            scrutinee: reference_expr(
                x.clone(),
                builtin_type_ids::INT,
                test_location(3),
                ValueMode::ImmutableReference,
            ),
            arms: vec![
                MatchArm {
                    pattern: MatchPattern::Literal(Expression::int(
                        1,
                        test_location(3),
                        ValueMode::ImmutableOwned,
                    )),
                    guard: None,
                    body: vec![node(
                        NodeKind::ExpressionStatement(Expression::int(
                            9,
                            test_location(3),
                            ValueMode::ImmutableOwned,
                        )),
                        test_location(3),
                    )],
                },
                MatchArm {
                    pattern: MatchPattern::Literal(Expression::int(
                        2,
                        test_location(3),
                        ValueMode::ImmutableOwned,
                    )),
                    guard: None,
                    body: vec![node(
                        NodeKind::ExpressionStatement(Expression::int(
                            8,
                            test_location(3),
                            ValueMode::ImmutableOwned,
                        )),
                        test_location(3),
                    )],
                },
            ],
            default: Some(vec![node(
                NodeKind::ExpressionStatement(Expression::int(
                    0,
                    test_location(3),
                    ValueMode::ImmutableOwned,
                )),
                test_location(3),
            )]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        test_location(3),
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                x,
                builtin_type_ids::INT,
                false,
                test_location(2),
            )],
            returns: vec![],
        },
        vec![match_node],
        test_location(2),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];

    let arms = match &entry_block.terminator {
        HirTerminator::Match { arms, .. } => arms,
        _ => panic!("expected match terminator"),
    };

    assert_eq!(arms.len(), 3);
    assert!(matches!(arms[0].pattern, HirPattern::Literal(_)));
    assert!(matches!(arms[1].pattern, HirPattern::Literal(_)));
    assert!(matches!(arms[2].pattern, HirPattern::Wildcard));
}

#[test]
fn lowers_match_with_guarded_arm_into_hir_guard_expression() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let x = super::symbol("x", &mut string_table);

    let match_node = node(
        NodeKind::Match {
            scrutinee: reference_expr(
                x.clone(),
                builtin_type_ids::INT,
                test_location(3),
                ValueMode::ImmutableReference,
            ),
            arms: vec![MatchArm {
                pattern: MatchPattern::Literal(Expression::int(
                    1,
                    test_location(3),
                    ValueMode::ImmutableOwned,
                )),
                guard: Some(Expression::bool(
                    true,
                    test_location(3),
                    ValueMode::ImmutableOwned,
                )),
                body: vec![node(
                    NodeKind::ExpressionStatement(Expression::int(
                        9,
                        test_location(3),
                        ValueMode::ImmutableOwned,
                    )),
                    test_location(3),
                )],
            }],
            default: Some(vec![node(
                NodeKind::ExpressionStatement(Expression::int(
                    8,
                    test_location(4),
                    ValueMode::ImmutableOwned,
                )),
                test_location(4),
            )]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        test_location(3),
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                x,
                builtin_type_ids::INT,
                false,
                test_location(2),
            )],
            returns: vec![],
        },
        vec![match_node],
        test_location(2),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");
    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];

    let arms = match &entry_block.terminator {
        HirTerminator::Match { arms, .. } => arms,
        _ => panic!("expected match terminator"),
    };

    assert!(
        arms[0].guard.is_some(),
        "first arm should preserve the lowered guard expression"
    );
    assert!(
        arms[1].guard.is_none(),
        "default wildcard arm should not carry a guard"
    );
}

#[test]
fn match_guard_rejects_lowering_when_guard_emits_prelude_statements() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let x = super::symbol("x", &mut string_table);
    let guarded_arm = MatchArm {
        pattern: MatchPattern::Literal(Expression::int(
            1,
            test_location(3),
            ValueMode::ImmutableOwned,
        )),
        guard: Some(Expression::host_function_call(
            crate::compiler_frontend::external_packages::ExternalFunctionId::IoLine,
            vec![Expression::bool(
                true,
                test_location(3),
                ValueMode::ImmutableOwned,
            )],
            vec![builtin_type_ids::NONE],
            test_location(3),
        )),
        body: vec![],
    };

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                x.clone(),
                builtin_type_ids::INT,
                false,
                test_location(2),
            )],
            returns: vec![],
        },
        vec![node(
            NodeKind::Match {
                scrutinee: reference_expr(
                    x,
                    builtin_type_ids::INT,
                    test_location(3),
                    ValueMode::ImmutableReference,
                ),
                arms: vec![guarded_arm],
                default: Some(vec![]),
                exhaustiveness: MatchExhaustiveness::HasDefault,
            },
            test_location(3),
        )],
        test_location(2),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let err = lower_ast(ast, &mut string_table)
        .expect_err("guard expressions with preludes should fail HIR lowering");

    let (error_type, message, _location) = err
        .first_infrastructure_error_for_tests()
        .expect("HIR lowering failure should be wrapped for rendering");
    assert_eq!(error_type, &ErrorType::HirTransformation);
    assert!(
        message.contains("Match arm guard lowering produced side-effect statements"),
        "unexpected error message: {message}",
    );
}

#[test]
fn match_rejects_non_literal_pattern_expressions() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let x = super::symbol("x", &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                x.clone(),
                builtin_type_ids::INT,
                false,
                test_location(2),
            )],
            returns: vec![],
        },
        vec![node(
            NodeKind::Match {
                scrutinee: reference_expr(
                    x.clone(),
                    builtin_type_ids::INT,
                    test_location(3),
                    ValueMode::ImmutableReference,
                ),
                arms: vec![MatchArm {
                    pattern: MatchPattern::Literal(reference_expr(
                        x,
                        builtin_type_ids::INT,
                        test_location(3),
                        ValueMode::ImmutableReference,
                    )),
                    guard: None,
                    body: vec![],
                }],
                default: Some(vec![]),
                exhaustiveness: MatchExhaustiveness::HasDefault,
            },
            test_location(3),
        )],
        test_location(2),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let err = lower_ast(ast, &mut string_table)
        .expect_err("non-literal match pattern should fail HIR lowering");

    let (error_type, message, _location) = err
        .first_infrastructure_error_for_tests()
        .expect("HIR lowering failure should be wrapped for rendering");
    assert_eq!(error_type, &ErrorType::HirTransformation);
    assert!(
        message.contains("Match arm patterns must be compile-time literals"),
        "unexpected error message: {message}",
    );
}

#[test]
fn break_outside_loop_reports_hir_transformation_error() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Break, test_location(2))],
        test_location(1),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let err = lower_ast(ast, &mut string_table).expect_err("break outside loop should fail");
    let (error_type, message, _location) = err
        .first_infrastructure_error_for_tests()
        .expect("HIR lowering failure should be wrapped for rendering");
    assert_eq!(error_type, &ErrorType::HirTransformation);
    assert!(message.contains("active loop context"));
}

#[test]
fn continue_outside_loop_reports_hir_transformation_error() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Continue, test_location(2))],
        test_location(1),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let err = lower_ast(ast, &mut string_table).expect_err("continue outside loop should fail");
    let (error_type, message, _location) = err
        .first_infrastructure_error_for_tests()
        .expect("HIR lowering failure should be wrapped for rendering");
    assert_eq!(error_type, &ErrorType::HirTransformation);
    assert!(message.contains("active loop context"));
}

#[test]
fn top_level_return_reports_hir_transformation_error() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_location(1),
    );

    let top_level_return = node(NodeKind::Return(vec![]), test_location(2));

    let ast = build_ast(vec![start_fn, top_level_return], entry_path);
    let err = lower_ast(ast, &mut string_table).expect_err("top-level return should fail");

    let (error_type, message, _location) = err
        .first_infrastructure_error_for_tests()
        .expect("HIR lowering failure should be wrapped for rendering");
    assert_eq!(error_type, &ErrorType::HirTransformation);
    assert!(message.contains("Top-level return"));
}

#[test]
fn unit_implicit_return_lowers_to_return_terminator() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_location(1),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("unit fallthrough should succeed");
    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];
    assert!(matches!(entry_block.terminator, HirTerminator::Return(_)));
}

#[test]
fn side_table_maps_statement_and_terminator_locations() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let x = super::symbol("x", &mut string_table);

    let decl_loc = test_location(4);
    let ret_loc = test_location(5);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x,
                    Expression::int(1, decl_loc.clone(), ValueMode::ImmutableOwned),
                )),
                decl_loc.clone(),
            ),
            node(NodeKind::Return(vec![]), ret_loc.clone()),
        ],
        test_location(3),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    let decl_mappings = module.side_table.hir_locations_for_ast(&decl_loc);
    assert!(!decl_mappings.is_empty());

    let ret_mappings = module.side_table.hir_locations_for_ast(&ret_loc);
    assert!(!ret_mappings.is_empty());
}

#[test]
fn lowers_relational_pattern_to_hir_relational() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let x = super::symbol("x", &mut string_table);

    let match_node = node(
        NodeKind::Match {
            scrutinee: reference_expr(
                x.clone(),
                builtin_type_ids::INT,
                test_location(3),
                ValueMode::ImmutableReference,
            ),
            arms: vec![MatchArm {
                pattern: MatchPattern::Relational {
                    op: RelationalPatternOp::LessThan,
                    value: Expression::int(10, test_location(3), ValueMode::ImmutableOwned),
                    location: test_location(3),
                },
                guard: None,
                body: vec![node(
                    NodeKind::ExpressionStatement(Expression::int(
                        9,
                        test_location(3),
                        ValueMode::ImmutableOwned,
                    )),
                    test_location(3),
                )],
            }],
            default: Some(vec![node(
                NodeKind::ExpressionStatement(Expression::int(
                    8,
                    test_location(4),
                    ValueMode::ImmutableOwned,
                )),
                test_location(4),
            )]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        test_location(3),
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                x,
                builtin_type_ids::INT,
                false,
                test_location(2),
            )],
            returns: vec![],
        },
        vec![match_node],
        test_location(2),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];

    let arms = match &entry_block.terminator {
        HirTerminator::Match { arms, .. } => arms,
        _ => panic!("expected match terminator"),
    };

    assert_eq!(arms.len(), 2);
    assert!(
        matches!(
            arms[0].pattern,
            HirPattern::Relational {
                op: crate::compiler_frontend::hir::patterns::HirRelationalPatternOp::LessThan,
                ..
            }
        ),
        "first arm should lower to HirPattern::Relational"
    );

    if let HirPattern::Relational { value, .. } = &arms[0].pattern {
        assert!(
            matches!(value.kind, HirExpressionKind::Int(10)),
            "relational RHS should be a const int literal"
        );
        assert_eq!(value.value_kind, ValueKind::Const);
    }
}

#[test]
fn lowers_guarded_relational_pattern_preserving_guard_separation() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let x = super::symbol("x", &mut string_table);

    let match_node = node(
        NodeKind::Match {
            scrutinee: reference_expr(
                x.clone(),
                builtin_type_ids::INT,
                test_location(3),
                ValueMode::ImmutableReference,
            ),
            arms: vec![MatchArm {
                pattern: MatchPattern::Relational {
                    op: RelationalPatternOp::LessThan,
                    value: Expression::int(10, test_location(3), ValueMode::ImmutableOwned),
                    location: test_location(3),
                },
                guard: Some(Expression::bool(
                    true,
                    test_location(3),
                    ValueMode::ImmutableOwned,
                )),
                body: vec![node(
                    NodeKind::ExpressionStatement(Expression::int(
                        9,
                        test_location(3),
                        ValueMode::ImmutableOwned,
                    )),
                    test_location(3),
                )],
            }],
            default: Some(vec![node(
                NodeKind::ExpressionStatement(Expression::int(
                    8,
                    test_location(4),
                    ValueMode::ImmutableOwned,
                )),
                test_location(4),
            )]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        test_location(3),
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                x,
                builtin_type_ids::INT,
                false,
                test_location(2),
            )],
            returns: vec![],
        },
        vec![match_node],
        test_location(2),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];

    let arms = match &entry_block.terminator {
        HirTerminator::Match { arms, .. } => arms,
        _ => panic!("expected match terminator"),
    };

    assert_eq!(arms.len(), 2);
    assert!(
        matches!(arms[0].pattern, HirPattern::Relational { .. }),
        "pattern should be relational"
    );
    assert!(
        arms[0].guard.is_some(),
        "guard should remain separate from relational pattern"
    );
}

/// Verifies that `MatchPattern::ChoiceVariant` lowers to `HirPattern::ChoiceVariant`
/// with correct tag indices and a shared `ChoiceId`.
///
/// WHY: choice match arms must not become `HirPattern::Literal(HirExpressionKind::Int)`
/// after the Choice Hardening refactor.
#[test]
fn lowers_choice_match_arms_to_hir_choice_variant_patterns() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let status_path = InternedPath::from_single_str("Status", &mut string_table);
    let ready_name = string_table.intern("Ready");
    let busy_name = string_table.intern("Busy");
    let status_local = super::symbol("status", &mut string_table);

    let choice_variants = vec![
        ChoiceVariant {
            id: ready_name,
            payload: ChoiceVariantPayload::Unit,
            location: test_location(2),
        },
        ChoiceVariant {
            id: busy_name,
            payload: ChoiceVariantPayload::Unit,
            location: test_location(2),
        },
    ];

    let status_type_id = choice_type_id(status_path.clone(), &choice_variants);

    let match_node = node(
        NodeKind::Match {
            scrutinee: reference_expr(
                status_local.clone(),
                status_type_id,
                test_location(3),
                ValueMode::ImmutableOwned,
            ),
            arms: vec![
                MatchArm {
                    pattern: MatchPattern::ChoiceVariant {
                        nominal_path: status_path.clone(),
                        tag: 0,
                        captures: vec![],
                        location: test_location(4),
                    },
                    guard: None,
                    body: vec![node(
                        NodeKind::ExpressionStatement(Expression::int(
                            1,
                            test_location(4),
                            ValueMode::ImmutableOwned,
                        )),
                        test_location(4),
                    )],
                },
                MatchArm {
                    pattern: MatchPattern::ChoiceVariant {
                        nominal_path: status_path.clone(),
                        tag: 1,
                        captures: vec![],
                        location: test_location(5),
                    },
                    guard: None,
                    body: vec![node(
                        NodeKind::ExpressionStatement(Expression::int(
                            2,
                            test_location(5),
                            ValueMode::ImmutableOwned,
                        )),
                        test_location(5),
                    )],
                },
            ],
            default: None,
            exhaustiveness: MatchExhaustiveness::ExhaustiveChoice,
        },
        test_location(3),
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                status_local,
                status_type_id,
                false,
                test_location(2),
            )],
            returns: vec![],
        },
        vec![match_node],
        test_location(2),
    );

    let ast = build_ast_with_choices(
        vec![start_fn],
        entry_path,
        vec![HirTestChoiceDefinition {
            nominal_path: status_path,
            variants: choice_variants,
        }],
    );
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];

    let arms = match &entry_block.terminator {
        HirTerminator::Match { arms, .. } => arms,
        _ => panic!("expected match terminator"),
    };

    assert_eq!(arms.len(), 2);

    let (choice_id_0, tag_0) = match &arms[0].pattern {
        HirPattern::ChoiceVariant {
            choice_id,
            variant_index,
        } => (*choice_id, *variant_index),
        other => panic!("expected ChoiceVariant pattern, got {other:?}"),
    };
    assert_eq!(tag_0, 0, "first arm should match tag 0 (Ready)");
    assert_eq!(choice_id_0, ChoiceId(0));

    let (choice_id_1, tag_1) = match &arms[1].pattern {
        HirPattern::ChoiceVariant {
            choice_id,
            variant_index,
        } => (*choice_id, *variant_index),
        other => panic!("expected ChoiceVariant pattern, got {other:?}"),
    };
    assert_eq!(tag_1, 1, "second arm should match tag 1 (Busy)");
    assert_eq!(
        choice_id_1,
        ChoiceId(0),
        "both arms should share the same ChoiceId"
    );

    assert!(
        arms.iter()
            .all(|arm| !matches!(arm.pattern, HirPattern::Wildcard)),
        "exhaustive choice matches should not synthesize wildcard fallback arms"
    );
}

#[test]
fn lowers_option_present_capture_to_payload_assignment() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let maybe_name = super::symbol("maybe", &mut string_table);
    let capture_name = string_table.intern("value");
    let capture_path = InternedPath::from_single_str("value", &mut string_table);
    let mut type_environment = TypeEnvironment::new();
    let option_int_type_id = type_environment.intern_option(builtin_type_ids::INT);

    let match_node = node(
        NodeKind::Match {
            scrutinee: reference_expr(
                maybe_name.clone(),
                option_int_type_id,
                test_location(2),
                ValueMode::ImmutableReference,
            ),
            arms: vec![MatchArm {
                pattern: MatchPattern::OptionPresentCapture {
                    name: capture_name,
                    binding_path: capture_path.clone(),
                    inner_type_id: builtin_type_ids::INT,
                    location: test_location(3),
                    binding_location: test_location(3),
                },
                guard: None,
                body: vec![node(
                    NodeKind::ExpressionStatement(reference_expr(
                        capture_path,
                        builtin_type_ids::INT,
                        test_location(3),
                        ValueMode::ImmutableReference,
                    )),
                    test_location(3),
                )],
            }],
            default: Some(vec![node(
                NodeKind::ExpressionStatement(Expression::int(
                    0,
                    test_location(4),
                    ValueMode::ImmutableOwned,
                )),
                test_location(4),
            )]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        test_location(2),
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                maybe_name,
                option_int_type_id,
                false,
                test_location(1),
            )],
            returns: vec![],
        },
        vec![match_node],
        test_location(1),
    );

    let mut ast = build_ast(vec![start_fn], entry_path);
    let registered_option_type_id = ast.type_environment.intern_option(builtin_type_ids::INT);
    assert_eq!(
        registered_option_type_id, option_int_type_id,
        "test AST should use the same canonical option TypeId as HIR lowering"
    );

    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");
    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];

    let arms = match &entry_block.terminator {
        HirTerminator::Match { arms, .. } => arms,
        _ => panic!("expected match terminator"),
    };

    assert_eq!(
        arms.len(),
        2,
        "should have option-present arm + default wildcard arm"
    );
    assert!(
        matches!(arms[0].pattern, HirPattern::OptionPresent),
        "present capture should lower to the option-present HIR pattern"
    );
    assert!(
        matches!(arms[1].pattern, HirPattern::Wildcard),
        "default arm should remain a wildcard"
    );

    let capture_block = &module.blocks[arms[0].body.0 as usize];
    let payload_assignment = capture_block
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Assign {
                target: HirPlace::Local(_),
                value,
            } => match &value.kind {
                HirExpressionKind::VariantPayloadGet {
                    carrier,
                    variant_index,
                    field_index,
                    ..
                } => Some((carrier.clone(), *variant_index, *field_index)),
                _ => None,
            },
            _ => None,
        })
        .expect("option-present arm should assign the some payload to the capture local");

    assert!(
        matches!(payload_assignment.0, HirVariantCarrier::Option),
        "capture assignment should extract from the option carrier"
    );
    assert_eq!(
        payload_assignment.1, OPTION_SOME_VARIANT_INDEX,
        "capture assignment should extract from the option some variant"
    );
    assert_eq!(
        payload_assignment.2, 0,
        "capture assignment should extract the some payload field"
    );
}
