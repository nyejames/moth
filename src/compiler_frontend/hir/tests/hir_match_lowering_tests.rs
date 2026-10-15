//! HIR match lowering regression tests.
//!
//! WHAT: checks how `match` expressions lower into HIR switch blocks with pattern arms, guards,
//!       and exhaustiveness checks.
//! WHY: match lowering generates complex multi-way branching; regressions here produce wrong
//!      control flow or missing pattern coverage silently.

use crate::compiler_frontend::ast::ast_nodes::{MatchExhaustiveness, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::call_argument::{CallAccessMode, CallArgument};
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchArm, MatchPattern, PatternValue, RelationalPatternOp,
};
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::declaration_syntax::choice::{ChoiceVariant, ChoiceVariantPayload};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::expressions::{
    HirExpressionKind, HirVariantCarrier, OPTION_SOME_VARIANT_INDEX, ValueKind,
};
use crate::compiler_frontend::hir::ids::ChoiceId;
use crate::compiler_frontend::hir::patterns::HirPattern;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::{LocalSpan, SourceId, SourceSpan};
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    fresh_success_returns, function_node, make_test_variable, node, param_with_type_id,
    reference_expr_with_type_id,
};

use crate::compiler_frontend::tests::type_id_fixture_support::choice_type_id;
use crate::compiler_frontend::value_mode::ValueMode;

use crate::compiler_frontend::hir::hir_builder::{
    HirTestChoiceDefinition, assert_no_placeholder_terminators, build_ast_with_choices,
    build_ast_with_registered_types, lower_ast, validate_module_for_tests,
};

/// One exact `I32` value for pattern and body fixtures.
fn i32_value(value: i64) -> FixedScalarValue {
    FixedScalarValue::signed(FixedScalar::I32, value).expect("I32 literal in range")
}

/// One exact `I32` literal expression for pattern and body fixtures.
fn int_expr(
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: i64,
    span: Option<SourceSpan>,
) -> ExprId {
    builder
        .fixed_scalar(constants, i32_value(value), span, ValueMode::ImmutableOwned)
        .expect("I32 literal expression should construct")
        .id()
}

/// One exact fixed-scalar literal pattern value for match arm fixtures.
fn fixed_scalar_pattern(
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: FixedScalarValue,
    span: Option<SourceSpan>,
) -> PatternValue {
    let id = builder
        .fixed_scalar(constants, value, span, ValueMode::ImmutableOwned)
        .expect("fixed scalar pattern literal should construct")
        .id();
    PatternValue::from_expression(builder.view(id).expect("fixed scalar pattern view"))
        .expect("fixed scalar literal should be a valid pattern value")
}

/// One exact `I32` literal pattern value for match arm fixtures.
fn int_pattern(
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: i64,
    span: Option<SourceSpan>,
) -> PatternValue {
    fixed_scalar_pattern(builder, constants, i32_value(value), span)
}

#[test]
fn non_unit_function_with_terminal_match_default_does_not_report_fallthrough() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let chooser = super::symbol("choose_match", &mut path_fork, &mut string_table);
    let x = super::symbol("x", &mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();

    let chooser_fn = function_node(
        chooser,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                x,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                false,
                None,
            )],
            returns: fresh_success_returns(vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)]),
        },
        vec![node(
            NodeKind::Match {
                scrutinee: reference_expr_with_type_id(
                    &mut builder,
                    x,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    None,
                    ValueMode::ImmutableReference,
                ),
                arms: vec![MatchArm {
                    pattern: MatchPattern::Literal(int_pattern(
                        &mut builder,
                        &mut constants,
                        1,
                        None,
                    )),
                    guard: None,
                    body: vec![node(
                        NodeKind::Return(vec![int_expr(&mut builder, &mut constants, 1, None)]),
                        None,
                    )],
                }],
                default: Some(vec![node(
                    NodeKind::Return(vec![int_expr(&mut builder, &mut constants, 2, None)]),
                    None,
                )]),
                exhaustiveness: MatchExhaustiveness::HasDefault,
            },
            None,
        )],
        None,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn, chooser_fn],
        entry_path,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
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
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let label_fn_name = super::symbol("label_status", &mut path_fork, &mut string_table);
    let status_path = super::symbol("Status", &mut path_fork, &mut string_table);
    let status_local = super::symbol("status", &mut path_fork, &mut string_table);
    let ready_name = string_table.intern("Ready");
    let busy_name = string_table.intern("Busy");
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();

    let choice_variants = vec![
        ChoiceVariant {
            id: ready_name,
            payload: ChoiceVariantPayload::Unit,
            span: None,
        },
        ChoiceVariant {
            id: busy_name,
            payload: ChoiceVariantPayload::Unit,
            span: None,
        },
    ];

    let status_type_id = choice_type_id(&mut type_environment, status_path, &choice_variants);

    let label_fn = function_node(
        label_fn_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                status_local,
                status_type_id,
                false,
                None,
            )],
            returns: fresh_success_returns(vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)]),
        },
        vec![node(
            NodeKind::Match {
                scrutinee: reference_expr_with_type_id(
                    &mut builder,
                    status_local,
                    status_type_id,
                    None,
                    ValueMode::ImmutableReference,
                ),
                arms: vec![
                    MatchArm {
                        pattern: MatchPattern::ChoiceVariant {
                            nominal_path: status_path,
                            tag: 0,
                            captures: vec![],
                            span: None,
                        },
                        guard: None,
                        body: vec![node(
                            NodeKind::Return(vec![int_expr(&mut builder, &mut constants, 1, None)]),
                            None,
                        )],
                    },
                    MatchArm {
                        pattern: MatchPattern::ChoiceVariant {
                            nominal_path: status_path,
                            tag: 1,
                            captures: vec![],
                            span: None,
                        },
                        guard: None,
                        body: vec![node(
                            NodeKind::Return(vec![int_expr(&mut builder, &mut constants, 2, None)]),
                            None,
                        )],
                    },
                ],
                default: None,
                exhaustiveness: MatchExhaustiveness::ExhaustiveChoice,
            },
            None,
        )],
        None,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        None,
    );

    let ast = build_ast_with_choices(
        vec![start_fn, label_fn],
        entry_path,
        vec![HirTestChoiceDefinition {
            nominal_path: status_path,
            variants: choice_variants,
        }],
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
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let x = super::symbol("x", &mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();

    let match_node = node(
        NodeKind::Match {
            scrutinee: reference_expr_with_type_id(
                &mut builder,
                x,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                None,
                ValueMode::ImmutableReference,
            ),
            arms: vec![
                MatchArm {
                    pattern: MatchPattern::Literal(int_pattern(
                        &mut builder,
                        &mut constants,
                        1,
                        None,
                    )),
                    guard: None,
                    body: vec![node(
                        NodeKind::ExpressionStatement(int_expr(
                            &mut builder,
                            &mut constants,
                            9,
                            None,
                        )),
                        None,
                    )],
                },
                MatchArm {
                    pattern: MatchPattern::Literal(int_pattern(
                        &mut builder,
                        &mut constants,
                        2,
                        None,
                    )),
                    guard: None,
                    body: vec![node(
                        NodeKind::ExpressionStatement(int_expr(
                            &mut builder,
                            &mut constants,
                            8,
                            None,
                        )),
                        None,
                    )],
                },
            ],
            default: Some(vec![node(
                NodeKind::ExpressionStatement(int_expr(&mut builder, &mut constants, 0, None)),
                None,
            )]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        None,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                x,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                false,
                None,
            )],
            returns: vec![],
        },
        vec![match_node],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("HIR lowering should succeed");

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
fn lowers_and_validates_fixed_scalar_match_pattern() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let value_name = super::symbol("value", &mut path_fork, &mut string_table);
    let u8_type = builtin_type_ids::fixed_scalar(FixedScalar::U8);
    let pattern_value = FixedScalarValue::unsigned(FixedScalar::U8, 200).expect("200 fits in U8");
    let relational_pattern_value =
        FixedScalarValue::unsigned(FixedScalar::U8, 100).expect("100 fits in U8");
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();

    let match_node = node(
        NodeKind::Match {
            scrutinee: reference_expr_with_type_id(
                &mut builder,
                value_name,
                u8_type,
                None,
                ValueMode::ImmutableReference,
            ),
            arms: vec![
                MatchArm {
                    pattern: MatchPattern::Literal(fixed_scalar_pattern(
                        &mut builder,
                        &mut constants,
                        pattern_value,
                        None,
                    )),
                    guard: None,
                    body: vec![node(NodeKind::Return(vec![]), None)],
                },
                MatchArm {
                    pattern: MatchPattern::Relational {
                        op: RelationalPatternOp::LessThan,
                        value: fixed_scalar_pattern(
                            &mut builder,
                            &mut constants,
                            relational_pattern_value,
                            None,
                        ),
                        span: None,
                    },
                    guard: None,
                    body: vec![node(NodeKind::Return(vec![]), None)],
                },
            ],
            default: Some(vec![node(NodeKind::Return(vec![]), None)]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        None,
    );
    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(value_name, u8_type, false, None)],
            returns: vec![],
        },
        vec![match_node],
        None,
    );
    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let (module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("HIR lowering should succeed");

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
    let HirPattern::Literal(value) = &arms[0].pattern else {
        panic!("fixed scalar literal should lower to HirPattern::Literal");
    };

    let value = module.expressions.expression(*value);
    assert_eq!(value.ty, u8_type);
    assert!(matches!(
        &value.kind,
        HirExpressionKind::FixedScalar(lowered) if lowered == &pattern_value
    ));
    assert_eq!(value.value_kind, ValueKind::Const);

    let HirPattern::Relational {
        value: relational_value,
        ..
    } = &arms[1].pattern
    else {
        panic!("fixed scalar relational pattern should lower to HirPattern::Relational");
    };
    let relational_value = module.expressions.expression(*relational_value);
    assert_eq!(relational_value.ty, u8_type);
    assert!(matches!(
        &relational_value.kind,
        HirExpressionKind::FixedScalar(lowered) if lowered == &relational_pattern_value
    ));
    assert_eq!(relational_value.value_kind, ValueKind::Const);
    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("HIR validation should accept a fixed scalar literal pattern");
}

#[test]
fn lowers_and_validates_fixed_scalar_option_literal_pattern() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let maybe_name = super::symbol("maybe", &mut path_fork, &mut string_table);
    let u8_type = builtin_type_ids::fixed_scalar(FixedScalar::U8);
    let mut type_environment = TypeEnvironment::new();
    let option_u8_type = type_environment.intern_option(u8_type);
    let pattern_value = FixedScalarValue::unsigned(FixedScalar::U8, 200).expect("200 fits in U8");
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();

    let match_node = node(
        NodeKind::Match {
            scrutinee: reference_expr_with_type_id(
                &mut builder,
                maybe_name,
                option_u8_type,
                None,
                ValueMode::ImmutableReference,
            ),
            arms: vec![MatchArm {
                pattern: MatchPattern::OptionValue {
                    value: fixed_scalar_pattern(&mut builder, &mut constants, pattern_value, None),
                    span: None,
                },
                guard: None,
                body: vec![node(NodeKind::Return(vec![]), None)],
            }],
            default: Some(vec![node(NodeKind::Return(vec![]), None)]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        None,
    );
    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(maybe_name, option_u8_type, false, None)],
            returns: vec![],
        },
        vec![match_node],
        None,
    );
    let mut ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        builder,
        constants,
        type_environment,
    );
    assert_eq!(
        ast.type_environment.intern_option(u8_type),
        option_u8_type,
        "the AST fixture should use the registered U8? type"
    );

    let (module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("HIR lowering should succeed");
    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];
    let arms = match &entry_block.terminator {
        HirTerminator::Match { arms, .. } => arms,
        _ => panic!("expected match terminator"),
    };
    let HirPattern::OptionValue { value } = &arms[0].pattern else {
        panic!("fixed scalar optional literal should lower to HirPattern::OptionValue");
    };
    let value = module.expressions.expression(*value);
    assert_eq!(value.ty, u8_type);
    assert!(matches!(
        &value.kind,
        HirExpressionKind::FixedScalar(lowered) if lowered == &pattern_value
    ));
    assert_eq!(value.value_kind, ValueKind::Const);
    validate_module_for_tests(
        &module,
        &string_table,
        &type_environment,
        &ExternalPackageRegistry::new(),
    )
    .expect("HIR validation should accept a fixed scalar optional literal pattern");
}

#[test]
fn lowers_match_with_guarded_arm_into_hir_guard_expression() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let x = super::symbol("x", &mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();

    let match_node = node(
        NodeKind::Match {
            scrutinee: reference_expr_with_type_id(
                &mut builder,
                x,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                None,
                ValueMode::ImmutableReference,
            ),
            arms: vec![MatchArm {
                pattern: MatchPattern::Literal(int_pattern(&mut builder, &mut constants, 1, None)),
                guard: Some(
                    builder
                        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
                        .expect("guard bool fits expression store")
                        .id(),
                ),
                body: vec![node(
                    NodeKind::ExpressionStatement(int_expr(&mut builder, &mut constants, 9, None)),
                    None,
                )],
            }],
            default: Some(vec![node(
                NodeKind::ExpressionStatement(int_expr(&mut builder, &mut constants, 8, None)),
                None,
            )]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        None,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                x,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                false,
                None,
            )],
            returns: vec![],
        },
        vec![match_node],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("HIR lowering should succeed");
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
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let x = super::symbol("x", &mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let true_arg = builder
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .expect("guard argument fits expression store")
        .id();
    let io_line = builder
        .host_function_call_with_typed_arguments(
            crate::compiler_frontend::external_packages::ExternalFunctionId::IoLine,
            vec![CallArgument::positional(
                builder.own(true_arg),
                CallAccessMode::Shared,
                None,
            )],
            vec![builtin_type_ids::NONE],
            false,
            None,
        )
        .expect("guard host call fits expression store")
        .id();
    let guarded_arm = MatchArm {
        pattern: MatchPattern::Literal(int_pattern(&mut builder, &mut constants, 1, None)),
        guard: Some(io_line),
        body: vec![],
    };

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                x,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                false,
                None,
            )],
            returns: vec![],
        },
        vec![node(
            NodeKind::Match {
                scrutinee: reference_expr_with_type_id(
                    &mut builder,
                    x,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    None,
                    ValueMode::ImmutableReference,
                ),
                arms: vec![guarded_arm],
                default: Some(vec![]),
                exhaustiveness: MatchExhaustiveness::HasDefault,
            },
            None,
        )],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let err = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect_err("guard expressions with preludes should fail HIR lowering");

    let error = err
        .primary_infrastructure_error()
        .expect("HIR lowering failure should be wrapped for rendering");
    assert_eq!(&error.error_type, &ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("Match arm guard lowering produced side-effect statements"),
        "unexpected error message: {}",
        error.msg,
    );
}

#[test]
fn match_rejects_non_literal_pattern_expressions() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let x = super::symbol("x", &mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let constants = ConstValueStore::default();
    let non_literal_pattern = reference_expr_with_type_id(
        &mut builder,
        x,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        None,
        ValueMode::ImmutableReference,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                x,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                false,
                None,
            )],
            returns: vec![],
        },
        vec![node(
            NodeKind::Match {
                scrutinee: reference_expr_with_type_id(
                    &mut builder,
                    x,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    None,
                    ValueMode::ImmutableReference,
                ),
                arms: vec![MatchArm {
                    pattern: MatchPattern::Literal(
                        PatternValue::from_expression(
                            builder
                                .view(non_literal_pattern)
                                .expect("reference pattern view"),
                        )
                        .expect("reference pattern should be a valid pattern value"),
                    ),
                    guard: None,
                    body: vec![],
                }],
                default: Some(vec![]),
                exhaustiveness: MatchExhaustiveness::HasDefault,
            },
            None,
        )],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let err = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect_err("non-literal match pattern should fail HIR lowering");
    let error = err
        .primary_infrastructure_error()
        .expect("HIR lowering failure should be wrapped for rendering");
    assert_eq!(&error.error_type, &ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("Match arm patterns must be compile-time literals"),
        "unexpected error message: {}",
        error.msg,
    );
}

#[test]
fn break_outside_loop_reports_hir_transformation_error() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Break, None)],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let err = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect_err("break outside loop should fail");
    let error = err
        .primary_infrastructure_error()
        .expect("HIR lowering failure should be wrapped for rendering");
    assert_eq!(&error.error_type, &ErrorType::HirTransformation);
    assert!(error.msg.contains("active loop context"));
}

#[test]
fn continue_outside_loop_reports_hir_transformation_error() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Continue, None)],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let err = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect_err("continue outside loop should fail");
    let error = err
        .primary_infrastructure_error()
        .expect("HIR lowering failure should be wrapped for rendering");
    assert_eq!(&error.error_type, &ErrorType::HirTransformation);
    assert!(error.msg.contains("active loop context"));
}

#[test]
fn top_level_return_reports_hir_transformation_error() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        None,
    );

    let top_level_return = node(NodeKind::Return(vec![]), None);

    let ast = build_ast_with_registered_types(
        vec![start_fn, top_level_return],
        entry_path,
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let err = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect_err("top-level return should fail");

    let error = err
        .primary_infrastructure_error()
        .expect("HIR lowering failure should be wrapped for rendering");
    assert_eq!(&error.error_type, &ErrorType::HirTransformation);
    assert!(error.msg.contains("Top-level return"));
}

#[test]
fn unit_implicit_return_lowers_to_return_terminator() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("unit fallthrough should succeed");
    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];
    assert!(matches!(entry_block.terminator, HirTerminator::Return(_)));
}

#[test]
fn side_table_maps_statement_and_terminator_spans() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let x = super::symbol("x", &mut path_fork, &mut string_table);

    let decl_span = SourceSpan::new(SourceId::from_index(1), LocalSpan::source_start());
    let ret_span = SourceSpan::new(SourceId::from_index(2), LocalSpan::source_start());
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let one = int_expr(&mut builder, &mut constants, 1, Some(decl_span));

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(x, one, &builder)),
                Some(decl_span),
            ),
            node(NodeKind::Return(vec![]), Some(ret_span)),
        ],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("HIR lowering should succeed");

    let decl_mappings = module.side_table.hir_locations_for_ast(decl_span);
    assert!(!decl_mappings.is_empty());

    let ret_mappings = module.side_table.hir_locations_for_ast(ret_span);
    assert!(!ret_mappings.is_empty());
}

#[test]
fn lowers_relational_pattern_to_hir_relational() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let x = super::symbol("x", &mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let nine = int_expr(&mut builder, &mut constants, 9, None);
    let eight = int_expr(&mut builder, &mut constants, 8, None);

    let match_node = node(
        NodeKind::Match {
            scrutinee: reference_expr_with_type_id(
                &mut builder,
                x,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                None,
                ValueMode::ImmutableReference,
            ),
            arms: vec![MatchArm {
                pattern: MatchPattern::Relational {
                    op: RelationalPatternOp::LessThan,
                    value: int_pattern(&mut builder, &mut constants, 10, None),
                    span: None,
                },
                guard: None,
                body: vec![node(NodeKind::ExpressionStatement(nine), None)],
            }],
            default: Some(vec![node(NodeKind::ExpressionStatement(eight), None)]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        None,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                x,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                false,
                None,
            )],
            returns: vec![],
        },
        vec![match_node],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("HIR lowering should succeed");

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
        let value = module.expressions.expression(*value);
        assert!(
            matches!(
                &value.kind,
                HirExpressionKind::FixedScalar(lowered) if *lowered == i32_value(10)
            ),
            "relational RHS should be a const I32 literal"
        );
        assert_eq!(value.value_kind, ValueKind::Const);
    }
}

#[test]
fn lowers_guarded_relational_pattern_preserving_guard_separation() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let x = super::symbol("x", &mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let guard = builder
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .expect("guard literal fits expression store")
        .id();
    let nine = int_expr(&mut builder, &mut constants, 9, None);
    let eight = int_expr(&mut builder, &mut constants, 8, None);

    let match_node = node(
        NodeKind::Match {
            scrutinee: reference_expr_with_type_id(
                &mut builder,
                x,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                None,
                ValueMode::ImmutableReference,
            ),
            arms: vec![MatchArm {
                pattern: MatchPattern::Relational {
                    op: RelationalPatternOp::LessThan,
                    value: int_pattern(&mut builder, &mut constants, 10, None),
                    span: None,
                },
                guard: Some(guard),
                body: vec![node(NodeKind::ExpressionStatement(nine), None)],
            }],
            default: Some(vec![node(NodeKind::ExpressionStatement(eight), None)]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        None,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                x,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                false,
                None,
            )],
            returns: vec![],
        },
        vec![match_node],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("HIR lowering should succeed");

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
/// WHY: choice match arms must not become `HirPattern::Literal(HirExpressionKind::FixedScalar)`
/// after the Choice Hardening refactor.
#[test]
fn lowers_choice_match_arms_to_hir_choice_variant_patterns() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let status_path = super::symbol("Status", &mut path_fork, &mut string_table);
    let ready_name = string_table.intern("Ready");
    let busy_name = string_table.intern("Busy");
    let status_local = super::symbol("status", &mut path_fork, &mut string_table);

    let choice_variants = vec![
        ChoiceVariant {
            id: ready_name,
            payload: ChoiceVariantPayload::Unit,
            span: None,
        },
        ChoiceVariant {
            id: busy_name,
            payload: ChoiceVariantPayload::Unit,
            span: None,
        },
    ];

    let mut type_environment = TypeEnvironment::new();
    let status_type_id = choice_type_id(&mut type_environment, status_path, &choice_variants);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let one = int_expr(&mut builder, &mut constants, 1, None);
    let two = int_expr(&mut builder, &mut constants, 2, None);

    let match_node = node(
        NodeKind::Match {
            scrutinee: reference_expr_with_type_id(
                &mut builder,
                status_local,
                status_type_id,
                None,
                ValueMode::ImmutableOwned,
            ),
            arms: vec![
                MatchArm {
                    pattern: MatchPattern::ChoiceVariant {
                        nominal_path: status_path,
                        tag: 0,
                        captures: vec![],
                        span: None,
                    },
                    guard: None,
                    body: vec![node(NodeKind::ExpressionStatement(one), None)],
                },
                MatchArm {
                    pattern: MatchPattern::ChoiceVariant {
                        nominal_path: status_path,
                        tag: 1,
                        captures: vec![],
                        span: None,
                    },
                    guard: None,
                    body: vec![node(NodeKind::ExpressionStatement(two), None)],
                },
            ],
            default: None,
            exhaustiveness: MatchExhaustiveness::ExhaustiveChoice,
        },
        None,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                status_local,
                status_type_id,
                false,
                None,
            )],
            returns: vec![],
        },
        vec![match_node],
        None,
    );

    let ast = build_ast_with_choices(
        vec![start_fn],
        entry_path,
        vec![HirTestChoiceDefinition {
            nominal_path: status_path,
            variants: choice_variants,
        }],
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
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let maybe_name = super::symbol("maybe", &mut path_fork, &mut string_table);
    let capture_name = string_table.intern("value");
    let capture_path = super::symbol("value", &mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();
    let option_i32_type_id =
        type_environment.intern_option(builtin_type_ids::fixed_scalar(FixedScalar::I32));

    let match_node = node(
        NodeKind::Match {
            scrutinee: reference_expr_with_type_id(
                &mut builder,
                maybe_name,
                option_i32_type_id,
                None,
                ValueMode::ImmutableReference,
            ),
            arms: vec![MatchArm {
                pattern: MatchPattern::OptionPresentCapture {
                    name: capture_name,
                    binding_path: capture_path,
                    inner_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    span: None,
                    binding_span: None,
                },
                guard: None,
                body: vec![node(
                    NodeKind::ExpressionStatement(reference_expr_with_type_id(
                        &mut builder,
                        capture_path,
                        builtin_type_ids::fixed_scalar(FixedScalar::I32),
                        None,
                        ValueMode::ImmutableReference,
                    )),
                    None,
                )],
            }],
            default: Some(vec![node(
                NodeKind::ExpressionStatement(int_expr(&mut builder, &mut constants, 0, None)),
                None,
            )]),
            exhaustiveness: MatchExhaustiveness::HasDefault,
        },
        None,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                maybe_name,
                option_i32_type_id,
                false,
                None,
            )],
            returns: vec![],
        },
        vec![match_node],
        None,
    );

    let mut ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        builder,
        constants,
        type_environment,
    );
    let registered_option_type_id = ast
        .type_environment
        .intern_option(builtin_type_ids::fixed_scalar(FixedScalar::I32));
    assert_eq!(
        registered_option_type_id, option_i32_type_id,
        "test AST should use the same canonical option TypeId as HIR lowering"
    );

    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("HIR lowering should succeed");
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
        .find_map(|statement| {
            let value = super::write_value(&module, statement)?;
            if !matches!(
                &statement.kind,
                HirStatementKind::Write {
                    target: super::HirWriteTarget::DefineLocal(_),
                    ..
                }
            ) {
                return None;
            }
            match &value.kind {
                HirExpressionKind::VariantPayloadGet {
                    carrier,
                    variant_index,
                    field_index,
                    ..
                } => Some((carrier.clone(), *variant_index, *field_index)),
                _ => None,
            }
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
