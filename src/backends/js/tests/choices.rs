//! Choice construction and match lowering tests for JavaScript output.

use super::support::*;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern, HirRelationalPatternOp};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;

// Choice lowering contract tests [choice]
// ---------------------------------------------------------------------------

/// Verifies that choice variant construction emits a plain integer literal.
#[test]
fn choice_variant_construction_emits_integer_literal() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.choice_unit, RegionId(0))],
        statements: vec![statement(
            1,
            HirStatementKind::Assign {
                target: HirPlace::Local(LocalId(0)),
                value: int_expression(1, 2, types.choice_unit, RegionId(0)),
            },
            1,
        )],
        terminator: HirTerminator::Return(unit_expression(2, types.unit, RegionId(0))),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
        return_aliases: vec![],
    };

    let module = build_module(
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "status")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(
        output
            .source
            .contains("__moth_assign_value(moth_status_l0, 2)"),
        "choice variant must lower to a plain integer literal inside an assignment"
    );
}

/// Verifies that choice match lowers to structured if with === against integer tags.
#[test]
fn choice_match_lowers_to_structured_if_with_literal_equals() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![local(0, types.choice_unit, RegionId(0))],
            statements: vec![statement(
                1,
                HirStatementKind::Assign {
                    target: HirPlace::Local(LocalId(0)),
                    value: int_expression(1, 0, types.choice_unit, RegionId(0)),
                },
                1,
            )],
            terminator: HirTerminator::Match {
                scrutinee: expression(
                    2,
                    HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                    types.choice_unit,
                    RegionId(0),
                    ValueKind::RValue,
                ),
                arms: vec![
                    HirMatchArm {
                        pattern: HirPattern::Literal(int_expression(
                            3,
                            0,
                            types.choice_unit,
                            RegionId(0),
                        )),
                        guard: None,
                        body: BlockId(1),
                    },
                    HirMatchArm {
                        pattern: HirPattern::Literal(int_expression(
                            4,
                            1,
                            types.choice_unit,
                            RegionId(0),
                        )),
                        guard: None,
                        body: BlockId(2),
                    },
                    HirMatchArm {
                        pattern: HirPattern::Literal(int_expression(
                            5,
                            2,
                            types.choice_unit,
                            RegionId(0),
                        )),
                        guard: None,
                        body: BlockId(3),
                    },
                ],
            },
        },
        HirBlock {
            id: BlockId(1),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Jump {
                target: BlockId(4),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(2),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Jump {
                target: BlockId(4),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(3),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Jump {
                target: BlockId(4),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(4),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Return(unit_expression(6, types.unit, RegionId(0))),
        },
    ];

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
        return_aliases: vec![],
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[(LocalId(0), "status")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(
        output.source.contains("if ("),
        "choice match must emit structured if"
    );
    assert!(
        output.source.contains("=== 0"),
        "choice match arm must compare with === 0"
    );
    assert!(
        output.source.contains("=== 1"),
        "choice match arm must compare with === 1"
    );
    assert!(
        !output.source.contains("while (true)"),
        "acyclic choice match must not use dispatcher"
    );
}

/// Verifies that a wildcard arm in a choice match emits a catch-all else block.
#[test]
fn choice_match_with_wildcard_arm_emits_true_condition() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![local(0, types.choice_unit, RegionId(0))],
            statements: vec![statement(
                1,
                HirStatementKind::Assign {
                    target: HirPlace::Local(LocalId(0)),
                    value: int_expression(1, 1, types.choice_unit, RegionId(0)),
                },
                1,
            )],
            terminator: HirTerminator::Match {
                scrutinee: expression(
                    2,
                    HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                    types.choice_unit,
                    RegionId(0),
                    ValueKind::RValue,
                ),
                arms: vec![
                    HirMatchArm {
                        pattern: HirPattern::Literal(int_expression(
                            3,
                            0,
                            types.choice_unit,
                            RegionId(0),
                        )),
                        guard: None,
                        body: BlockId(1),
                    },
                    HirMatchArm {
                        pattern: HirPattern::Wildcard,
                        guard: None,
                        body: BlockId(2),
                    },
                ],
            },
        },
        HirBlock {
            id: BlockId(1),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Jump {
                target: BlockId(3),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(2),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Jump {
                target: BlockId(3),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(3),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Return(unit_expression(4, types.unit, RegionId(0))),
        },
    ];

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
        return_aliases: vec![],
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[(LocalId(0), "status")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(
        output.source.contains("if ("),
        "choice match must emit structured if"
    );
    assert!(
        output.source.contains("else if (true)"),
        "wildcard arm must emit a catch-all 'else if (true)' condition"
    );
    assert!(
        !output.source.contains("while (true)"),
        "acyclic choice match must not use dispatcher"
    );
}

/// Verifies that relational match patterns emit correct JS comparison operators.
#[test]
fn relational_match_patterns_emit_correct_js_operators() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![local(0, types.int, RegionId(0))],
            statements: vec![statement(
                1,
                HirStatementKind::Assign {
                    target: HirPlace::Local(LocalId(0)),
                    value: int_expression(1, 5, types.int, RegionId(0)),
                },
                1,
            )],
            terminator: HirTerminator::Match {
                scrutinee: expression(
                    2,
                    HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                    types.int,
                    RegionId(0),
                    ValueKind::Place,
                ),
                arms: vec![
                    HirMatchArm {
                        pattern: HirPattern::Relational {
                            op: HirRelationalPatternOp::LessThan,
                            value: int_expression(3, 10, types.int, RegionId(0)),
                        },
                        guard: None,
                        body: BlockId(1),
                    },
                    HirMatchArm {
                        pattern: HirPattern::Relational {
                            op: HirRelationalPatternOp::LessThanOrEqual,
                            value: int_expression(4, 20, types.int, RegionId(0)),
                        },
                        guard: None,
                        body: BlockId(2),
                    },
                    HirMatchArm {
                        pattern: HirPattern::Relational {
                            op: HirRelationalPatternOp::GreaterThan,
                            value: int_expression(5, 30, types.int, RegionId(0)),
                        },
                        guard: None,
                        body: BlockId(3),
                    },
                    HirMatchArm {
                        pattern: HirPattern::Relational {
                            op: HirRelationalPatternOp::GreaterThanOrEqual,
                            value: int_expression(6, 40, types.int, RegionId(0)),
                        },
                        guard: None,
                        body: BlockId(4),
                    },
                ],
            },
        },
        HirBlock {
            id: BlockId(1),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Jump {
                target: BlockId(5),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(2),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Jump {
                target: BlockId(5),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(3),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Jump {
                target: BlockId(5),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(4),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Jump {
                target: BlockId(5),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(5),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Return(unit_expression(7, types.unit, RegionId(0))),
        },
    ];

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
        return_aliases: vec![],
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[(LocalId(0), "value")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(
        output.source.contains(" < 10"),
        "LessThan should emit '<' operator, got: {}",
        output.source
    );
    assert!(
        output.source.contains(" <= 20"),
        "LessThanOrEqual should emit '<=' operator, got: {}",
        output.source
    );
    assert!(
        output.source.contains(" > 30"),
        "GreaterThan should emit '>' operator, got: {}",
        output.source
    );
    assert!(
        output.source.contains(" >= 40"),
        "GreaterThanOrEqual should emit '>=' operator, got: {}",
        output.source
    );
}

// ---------------------------------------------------------------------------
