//! Structured and dispatcher-based JavaScript control-flow lowering tests.

use super::support::*;
use crate::compiler_frontend::datatypes::ids::{BuiltinTypeConstructor, TypeConstructor};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{
    HirExpressionKind, HirVariantCarrier, HirVariantField, ValueKind,
};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;

// CFG lowering tests [cfg]
// ---------------------------------------------------------------------------

/// Verifies that a simple acyclic if-then-else lowers to structured JS without a dispatcher. [cfg]
#[test]
fn emits_structured_if_without_dispatcher() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let assign_then = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: int_expression(1, 2, types.int, RegionId(0)),
        },
        2,
    );

    let assign_else = statement(
        2,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: int_expression(2, 3, types.int, RegionId(0)),
        },
        3,
    );

    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![local(0, types.int, RegionId(0))],
            statements: vec![],
            terminator: HirTerminator::If {
                condition: bool_expression(3, true, types.boolean, RegionId(0)),
                then_block: BlockId(1),
                else_block: BlockId(2),
            },
        },
        HirBlock {
            id: BlockId(1),
            region: RegionId(0),
            locals: vec![],
            statements: vec![assign_then],
            terminator: HirTerminator::Jump {
                target: BlockId(3),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(2),
            region: RegionId(0),
            locals: vec![],
            statements: vec![assign_else],
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
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[(LocalId(0), "x")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(output.source.contains("if (true)"));
    assert!(!output.source.contains("switch (__bb"));
}

/// Verifies that a synthetic wildcard merge arm remains a post-match continuation. [cfg]
#[test]
fn emits_structured_match_without_inlining_synthetic_merge_arm() {
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
                    value: int_expression(1, 1, types.int, RegionId(0)),
                },
                1,
            )],
            terminator: HirTerminator::Match {
                scrutinee: expression(
                    2,
                    HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                    types.int,
                    RegionId(0),
                    ValueKind::RValue,
                ),
                arms: vec![
                    HirMatchArm {
                        pattern: HirPattern::Literal(int_expression(3, 0, types.int, RegionId(0))),
                        guard: None,
                        body: BlockId(1),
                    },
                    HirMatchArm {
                        pattern: HirPattern::Literal(int_expression(4, 1, types.int, RegionId(0))),
                        guard: None,
                        body: BlockId(2),
                    },
                    HirMatchArm {
                        pattern: HirPattern::Wildcard,
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
            terminator: HirTerminator::Return(unit_expression(5, types.unit, RegionId(0))),
        },
    ];

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[(LocalId(0), "x")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(output.source.contains("const __match_value_"));
    assert!(!output.source.contains("else if (true)"));
    assert!(!output.source.contains("switch (__bb"));
}

/// Verifies that literal matches lower to structured if-chains when CFG is acyclic. [cfg]
#[test]
fn literal_match_uses_structured_lowering_when_cfg_is_acyclic() {
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
                    value: int_expression(1, 2, types.int, RegionId(0)),
                },
                1,
            )],
            terminator: HirTerminator::Match {
                scrutinee: expression(
                    2,
                    HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                    types.int,
                    RegionId(0),
                    ValueKind::RValue,
                ),
                arms: vec![
                    HirMatchArm {
                        pattern: HirPattern::Literal(int_expression(3, 1, types.int, RegionId(0))),
                        guard: None,
                        body: BlockId(1),
                    },
                    HirMatchArm {
                        pattern: HirPattern::Literal(int_expression(4, 2, types.int, RegionId(0))),
                        guard: None,
                        body: BlockId(2),
                    },
                    HirMatchArm {
                        pattern: HirPattern::Wildcard,
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
            terminator: HirTerminator::Return(unit_expression(5, types.unit, RegionId(0))),
        },
    ];

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[(LocalId(0), "subject")],
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
        output.source.contains("const __match_value_"),
        "structured lowering should stage the scrutinee once in a temp"
    );
    assert!(
        output.source.contains("=== 1"),
        "literal arm comparison must emit strict equality for first arm"
    );
    assert!(
        output.source.contains("=== 2"),
        "literal arm comparison must emit strict equality for second arm"
    );
    assert!(
        !output.source.contains("switch (__bb"),
        "acyclic literal matches should avoid the dispatcher"
    );
}

/// Verifies that OptionPresent match patterns lower to a some-tag check without
/// payload comparison. [cfg]
#[test]
fn option_present_match_checks_some_tag_without_payload_comparison() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let value_name = string_table.intern("value");

    let option_value = expression(
        1,
        HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Option,
            variant_index: 1,
            fields: vec![HirVariantField {
                name: Some(value_name),
                value: int_expression(2, 10, types.int, RegionId(0)),
            }],
        },
        types.option_int,
        RegionId(0),
        ValueKind::RValue,
    );

    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![local(0, types.option_int, RegionId(0))],
            statements: vec![statement(
                1,
                HirStatementKind::Assign {
                    target: HirPlace::Local(LocalId(0)),
                    value: option_value,
                },
                1,
            )],
            terminator: HirTerminator::Match {
                scrutinee: expression(
                    3,
                    HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                    types.option_int,
                    RegionId(0),
                    ValueKind::RValue,
                ),
                arms: vec![
                    HirMatchArm {
                        pattern: HirPattern::OptionPresent,
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
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[(LocalId(0), "maybe")],
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
        output.source.contains(".tag === \"some\""),
        "OptionPresent should lower to a some-tag condition"
    );
    assert!(
        !output.source.contains(".value ==="),
        "OptionPresent should not compare the present payload"
    );
}

/// Verifies that literal matches lower through dispatcher fallback in cyclic CFGs. [cfg]
#[test]
fn literal_match_uses_dispatcher_when_cfg_contains_cycle() {
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
                    value: int_expression(1, 0, types.int, RegionId(0)),
                },
                1,
            )],
            terminator: HirTerminator::Jump {
                target: BlockId(1),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(1),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Match {
                scrutinee: expression(
                    2,
                    HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                    types.int,
                    RegionId(0),
                    ValueKind::RValue,
                ),
                arms: vec![
                    HirMatchArm {
                        pattern: HirPattern::Literal(int_expression(3, 0, types.int, RegionId(0))),
                        guard: None,
                        body: BlockId(2),
                    },
                    HirMatchArm {
                        pattern: HirPattern::Literal(int_expression(4, 1, types.int, RegionId(0))),
                        guard: None,
                        body: BlockId(3),
                    },
                ],
            },
        },
        HirBlock {
            id: BlockId(2),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Jump {
                target: BlockId(1),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(3),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Return(unit_expression(5, types.unit, RegionId(0))),
        },
    ];

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[(LocalId(0), "subject")],
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
        output.source.contains("while (true)"),
        "cyclic CFG must select dispatcher lowering"
    );
    assert!(
        output.source.contains("switch (__bb"),
        "dispatcher lowering must emit block switch"
    );
    assert!(
        output.source.contains("const __match_"),
        "dispatcher match lowering should stage scrutinee in a temp"
    );
    assert!(
        output.source.contains("=== 0") && output.source.contains("=== 1"),
        "dispatcher match lowering should preserve literal strict-equality checks"
    );
}

/// Verifies structured literal-match arms converging on one continuation lower jump args stably. [cfg]
#[test]
fn structured_match_merge_convergence_lowers_jump_arguments() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let assign_arm0 = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(1)),
            value: int_expression(1, 10, types.int, RegionId(0)),
        },
        1,
    );
    let assign_arm1 = statement(
        2,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(2)),
            value: int_expression(2, 20, types.int, RegionId(0)),
        },
        2,
    );
    let assign_default = statement(
        3,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(3)),
            value: int_expression(3, 30, types.int, RegionId(0)),
        },
        3,
    );

    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![local(0, types.int, RegionId(0))],
            statements: vec![statement(
                4,
                HirStatementKind::Assign {
                    target: HirPlace::Local(LocalId(0)),
                    value: int_expression(4, 1, types.int, RegionId(0)),
                },
                4,
            )],
            terminator: HirTerminator::Match {
                scrutinee: expression(
                    5,
                    HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                    types.int,
                    RegionId(0),
                    ValueKind::RValue,
                ),
                arms: vec![
                    HirMatchArm {
                        pattern: HirPattern::Literal(int_expression(6, 0, types.int, RegionId(0))),
                        guard: None,
                        body: BlockId(1),
                    },
                    HirMatchArm {
                        pattern: HirPattern::Literal(int_expression(7, 1, types.int, RegionId(0))),
                        guard: None,
                        body: BlockId(2),
                    },
                    HirMatchArm {
                        pattern: HirPattern::Wildcard,
                        guard: None,
                        body: BlockId(3),
                    },
                ],
            },
        },
        HirBlock {
            id: BlockId(1),
            region: RegionId(0),
            locals: vec![local(1, types.int, RegionId(0))],
            statements: vec![assign_arm0],
            terminator: HirTerminator::Jump {
                target: BlockId(4),
                args: vec![LocalId(1)],
            },
        },
        HirBlock {
            id: BlockId(2),
            region: RegionId(0),
            locals: vec![local(2, types.int, RegionId(0))],
            statements: vec![assign_arm1],
            terminator: HirTerminator::Jump {
                target: BlockId(4),
                args: vec![LocalId(2)],
            },
        },
        HirBlock {
            id: BlockId(3),
            region: RegionId(0),
            locals: vec![local(3, types.int, RegionId(0))],
            statements: vec![assign_default],
            terminator: HirTerminator::Jump {
                target: BlockId(4),
                args: vec![LocalId(3)],
            },
        },
        HirBlock {
            id: BlockId(4),
            region: RegionId(0),
            locals: vec![local(4, types.int, RegionId(0))],
            statements: vec![],
            terminator: HirTerminator::Return(unit_expression(8, types.unit, RegionId(0))),
        },
    ];

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[
            (LocalId(0), "subject"),
            (LocalId(1), "arm0_value"),
            (LocalId(2), "arm1_value"),
            (LocalId(3), "default_value"),
            (LocalId(4), "merged"),
        ],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");
    let merged_name = expected_dev_local_name("merged", 4);

    assert!(
        !output.source.contains("switch (__bb"),
        "acyclic converging match should stay on structured lowering"
    );
    assert_eq!(
        output.source.matches("const __jump_arg_").count(),
        3,
        "all converging match-arm edges should stage one captured jump argument"
    );
    assert_eq!(
        output
            .source
            .matches(&format!("__moth_assign_value({merged_name}, __jump_arg_"))
            .count(),
        3,
        "all converging match-arm edges should assign merge locals"
    );
}

/// Verifies dispatcher fallback preserves merge convergence jump-arg lowering for match arms. [cfg]
#[test]
fn dispatcher_match_merge_convergence_lowers_jump_arguments() {
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
                    value: int_expression(1, 0, types.int, RegionId(0)),
                },
                1,
            )],
            terminator: HirTerminator::Jump {
                target: BlockId(1),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(1),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Match {
                scrutinee: expression(
                    2,
                    HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                    types.int,
                    RegionId(0),
                    ValueKind::RValue,
                ),
                arms: vec![
                    HirMatchArm {
                        pattern: HirPattern::Literal(int_expression(3, 0, types.int, RegionId(0))),
                        guard: None,
                        body: BlockId(2),
                    },
                    HirMatchArm {
                        pattern: HirPattern::Literal(int_expression(4, 1, types.int, RegionId(0))),
                        guard: None,
                        body: BlockId(3),
                    },
                    HirMatchArm {
                        pattern: HirPattern::Wildcard,
                        guard: None,
                        body: BlockId(4),
                    },
                ],
            },
        },
        HirBlock {
            id: BlockId(2),
            region: RegionId(0),
            locals: vec![local(1, types.int, RegionId(0))],
            statements: vec![statement(
                5,
                HirStatementKind::Assign {
                    target: HirPlace::Local(LocalId(1)),
                    value: int_expression(5, 10, types.int, RegionId(0)),
                },
                5,
            )],
            terminator: HirTerminator::Jump {
                target: BlockId(5),
                args: vec![LocalId(1)],
            },
        },
        HirBlock {
            id: BlockId(3),
            region: RegionId(0),
            locals: vec![local(2, types.int, RegionId(0))],
            statements: vec![statement(
                6,
                HirStatementKind::Assign {
                    target: HirPlace::Local(LocalId(2)),
                    value: int_expression(6, 20, types.int, RegionId(0)),
                },
                6,
            )],
            terminator: HirTerminator::Jump {
                target: BlockId(5),
                args: vec![LocalId(2)],
            },
        },
        HirBlock {
            id: BlockId(4),
            region: RegionId(0),
            locals: vec![local(3, types.int, RegionId(0))],
            statements: vec![statement(
                7,
                HirStatementKind::Assign {
                    target: HirPlace::Local(LocalId(3)),
                    value: int_expression(7, 30, types.int, RegionId(0)),
                },
                7,
            )],
            terminator: HirTerminator::Jump {
                target: BlockId(5),
                args: vec![LocalId(3)],
            },
        },
        HirBlock {
            id: BlockId(5),
            region: RegionId(0),
            locals: vec![local(4, types.int, RegionId(0))],
            statements: vec![],
            terminator: HirTerminator::Jump {
                target: BlockId(1),
                args: vec![],
            },
        },
    ];

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[
            (LocalId(0), "subject"),
            (LocalId(1), "arm0_value"),
            (LocalId(2), "arm1_value"),
            (LocalId(3), "default_value"),
            (LocalId(4), "merged"),
        ],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");
    let merged_name = expected_dev_local_name("merged", 4);

    assert!(
        output.source.contains("switch (__bb"),
        "cycle should force dispatcher lowering for converging match"
    );
    assert!(
        output.source.matches("const __jump_arg_").count() >= 3,
        "dispatcher should capture each converging match-arm jump argument"
    );
    assert!(
        output
            .source
            .matches(&format!("__moth_assign_value({merged_name}, __jump_arg_"))
            .count()
            >= 3,
        "dispatcher should assign merge locals for converging match-arm edges"
    );
}

/// Verifies guarded match-arm conditions emit as literal-check && guard-check conjunctions. [cfg]
#[test]
fn match_guard_condition_emits_pattern_and_guard_conjunction() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![
                local(0, types.int, RegionId(0)),
                local(1, types.boolean, RegionId(0)),
            ],
            statements: vec![
                statement(
                    1,
                    HirStatementKind::Assign {
                        target: HirPlace::Local(LocalId(0)),
                        value: int_expression(1, 1, types.int, RegionId(0)),
                    },
                    1,
                ),
                statement(
                    2,
                    HirStatementKind::Assign {
                        target: HirPlace::Local(LocalId(1)),
                        value: bool_expression(2, true, types.boolean, RegionId(0)),
                    },
                    2,
                ),
            ],
            terminator: HirTerminator::Match {
                scrutinee: expression(
                    3,
                    HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                    types.int,
                    RegionId(0),
                    ValueKind::RValue,
                ),
                arms: vec![
                    HirMatchArm {
                        pattern: HirPattern::Literal(int_expression(4, 1, types.int, RegionId(0))),
                        guard: Some(expression(
                            5,
                            HirExpressionKind::Load(HirPlace::Local(LocalId(1))),
                            types.boolean,
                            RegionId(0),
                            ValueKind::Place,
                        )),
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
                target: BlockId(2),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(2),
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
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[(LocalId(0), "subject"), (LocalId(1), "guard_flag")],
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
        output.source.contains("if (((__match_value_") && output.source.contains("=== 1)) && ("),
        "guarded match arm should emit conjunction between pattern and guard"
    );
    assert!(
        !output.source.contains("switch (__bb"),
        "acyclic guarded match should remain structured"
    );
}

/// Verifies malformed non-exhaustive dispatcher match emits stable runtime fallback. [cfg]
#[test]
fn dispatcher_match_without_selected_arm_emits_no_arm_selected_fallback() {
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
                    value: int_expression(1, 0, types.int, RegionId(0)),
                },
                1,
            )],
            terminator: HirTerminator::Jump {
                target: BlockId(1),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(1),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Match {
                scrutinee: expression(
                    2,
                    HirExpressionKind::Load(HirPlace::Local(LocalId(0))),
                    types.int,
                    RegionId(0),
                    ValueKind::RValue,
                ),
                arms: vec![HirMatchArm {
                    pattern: HirPattern::Literal(int_expression(3, 0, types.int, RegionId(0))),
                    guard: Some(bool_expression(4, false, types.boolean, RegionId(0))),
                    body: BlockId(2),
                }],
            },
        },
        HirBlock {
            id: BlockId(2),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Jump {
                target: BlockId(1),
                args: vec![],
            },
        },
    ];

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[(LocalId(0), "subject")],
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
        output.source.contains("switch (__bb"),
        "cycle should force dispatcher path for malformed match fallback assertion"
    );
    assert!(
        output
            .source
            .contains("throw new Error(\"No match arm selected\");"),
        "dispatcher match lowering must emit stable no-arm-selected fallback"
    );
}

/// Verifies that a CFG cycle falls back to a switch-based block dispatcher. [cfg]
#[test]
fn falls_back_to_dispatcher_for_cfg_cycle() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let loop_assign = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: int_expression(1, 1, types.int, RegionId(0)),
        },
        2,
    );

    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![local(0, types.int, RegionId(0))],
            statements: vec![],
            terminator: HirTerminator::Jump {
                target: BlockId(1),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(1),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::If {
                condition: bool_expression(2, true, types.boolean, RegionId(0)),
                then_block: BlockId(2),
                else_block: BlockId(3),
            },
        },
        HirBlock {
            id: BlockId(2),
            region: RegionId(0),
            locals: vec![],
            statements: vec![loop_assign],
            terminator: HirTerminator::Jump {
                target: BlockId(1),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(3),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Return(unit_expression(3, types.unit, RegionId(0))),
        },
    ];

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[(LocalId(0), "counter")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(output.source.contains("switch (__bb"));
}

/// Verifies that break and continue terminators emit the expected block-number assignments. [cfg]
#[test]
fn lowers_break_and_continue_terminators_with_dispatcher() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Jump {
                target: BlockId(1),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(1),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::If {
                condition: bool_expression(1, true, types.boolean, RegionId(0)),
                then_block: BlockId(2),
                else_block: BlockId(4),
            },
        },
        HirBlock {
            id: BlockId(2),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Continue { target: BlockId(3) },
        },
        HirBlock {
            id: BlockId(3),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Break { target: BlockId(4) },
        },
        HirBlock {
            id: BlockId(4),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Return(unit_expression(2, types.unit, RegionId(0))),
        },
    ];

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(&mut string_table, "main", blocks, function, &[]);

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(output.source.contains("switch (__bb"));
    assert!(output.source.contains("= 3;"));
    assert!(output.source.contains("= 4;"));
}

/// Verifies that a direct jump captures source values and assigns them into target block params. [cfg]
#[test]
fn jump_args_lower_block_to_block_value_transfer() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let assign_source = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: int_expression(1, 7, types.int, RegionId(0)),
        },
        1,
    );

    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![local(0, types.int, RegionId(0))],
            statements: vec![assign_source],
            terminator: HirTerminator::Jump {
                target: BlockId(1),
                args: vec![LocalId(0)],
            },
        },
        HirBlock {
            id: BlockId(1),
            region: RegionId(0),
            locals: vec![local(1, types.int, RegionId(0))],
            statements: vec![],
            terminator: HirTerminator::Return(unit_expression(2, types.unit, RegionId(0))),
        },
    ];

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[(LocalId(0), "source"), (LocalId(1), "param")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");
    let source_name = expected_dev_local_name("source", 0);
    let parameter_name = expected_dev_local_name("param", 1);

    assert_eq!(
        output.source.matches("const __jump_arg_").count(),
        1,
        "single-edge jump argument transfer should emit one capture temp"
    );
    assert!(
        output
            .source
            .contains(&format!("const __jump_arg_0 = __moth_read({source_name});")),
        "jump arguments should capture source values with __moth_read before assignment"
    );
    assert!(
        output.source.contains(&format!(
            "__moth_assign_value({parameter_name}, __jump_arg_0);"
        )),
        "jump arguments should assign into the first target local by position"
    );
    assert!(
        !output.source.contains("switch (__bb"),
        "acyclic jump-only CFG should stay on structured lowering"
    );
}

/// Verifies that structured if-branch merges lower block arguments for both incoming edges. [cfg]
#[test]
fn structured_branch_merge_lowers_jump_arguments() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let assign_then = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: int_expression(1, 10, types.int, RegionId(0)),
        },
        1,
    );
    let assign_else = statement(
        2,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(1)),
            value: int_expression(2, 20, types.int, RegionId(0)),
        },
        2,
    );

    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::If {
                condition: bool_expression(3, true, types.boolean, RegionId(0)),
                then_block: BlockId(1),
                else_block: BlockId(2),
            },
        },
        HirBlock {
            id: BlockId(1),
            region: RegionId(0),
            locals: vec![local(0, types.int, RegionId(0))],
            statements: vec![assign_then],
            terminator: HirTerminator::Jump {
                target: BlockId(3),
                args: vec![LocalId(0)],
            },
        },
        HirBlock {
            id: BlockId(2),
            region: RegionId(0),
            locals: vec![local(1, types.int, RegionId(0))],
            statements: vec![assign_else],
            terminator: HirTerminator::Jump {
                target: BlockId(3),
                args: vec![LocalId(1)],
            },
        },
        HirBlock {
            id: BlockId(3),
            region: RegionId(0),
            locals: vec![local(2, types.int, RegionId(0))],
            statements: vec![],
            terminator: HirTerminator::Return(unit_expression(4, types.unit, RegionId(0))),
        },
    ];

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[
            (LocalId(0), "then_value"),
            (LocalId(1), "else_value"),
            (LocalId(2), "merged"),
        ],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");
    let merged_name = expected_dev_local_name("merged", 2);

    assert!(output.source.contains("if (true)"));
    assert!(
        !output.source.contains("switch (__bb"),
        "acyclic branch merge with jump args should remain structured"
    );
    assert_eq!(
        output.source.matches("const __jump_arg_").count(),
        2,
        "both branch edges should stage one captured jump argument value"
    );
    assert_eq!(
        output
            .source
            .matches(&format!("__moth_assign_value({merged_name}, __jump_arg_"))
            .count(),
        2,
        "each branch edge should assign the merge parameter local"
    );
}

/// Verifies that loop back-edges carry jump arguments through the dispatcher path. [cfg]
#[test]
fn dispatcher_loop_back_edge_lowers_jump_arguments() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let assign_entry = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: int_expression(1, 1, types.int, RegionId(0)),
        },
        1,
    );
    let assign_back_edge = statement(
        2,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(2)),
            value: int_expression(2, 2, types.int, RegionId(0)),
        },
        2,
    );

    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![local(0, types.int, RegionId(0))],
            statements: vec![assign_entry],
            terminator: HirTerminator::Jump {
                target: BlockId(1),
                args: vec![LocalId(0)],
            },
        },
        HirBlock {
            id: BlockId(1),
            region: RegionId(0),
            locals: vec![local(1, types.int, RegionId(0))],
            statements: vec![],
            terminator: HirTerminator::If {
                condition: bool_expression(3, true, types.boolean, RegionId(0)),
                then_block: BlockId(2),
                else_block: BlockId(3),
            },
        },
        HirBlock {
            id: BlockId(2),
            region: RegionId(0),
            locals: vec![local(2, types.int, RegionId(0))],
            statements: vec![assign_back_edge],
            terminator: HirTerminator::Jump {
                target: BlockId(1),
                args: vec![LocalId(2)],
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
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[
            (LocalId(0), "entry_value"),
            (LocalId(1), "loop_value"),
            (LocalId(2), "back_edge_value"),
        ],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");
    let loop_value_name = expected_dev_local_name("loop_value", 1);

    assert!(
        output.source.contains("switch (__bb"),
        "CFG cycle with back-edge should lower through dispatcher"
    );
    assert!(
        output.source.matches("const __jump_arg_").count() >= 2,
        "entry edge and loop back-edge should each stage captured jump arguments"
    );
    assert!(
        output
            .source
            .matches(&format!(
                "__moth_assign_value({loop_value_name}, __jump_arg_"
            ))
            .count()
            >= 2,
        "dispatcher jump edges should assign carried loop values into block parameters"
    );
}

/// Verifies that jump-arg assignment writes through alias-only target block params. [cfg] [alias]
#[test]
fn jump_args_write_through_alias_only_target_local() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let assign_source = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: int_expression(1, 42, types.int, RegionId(0)),
        },
        1,
    );

    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![local(0, types.int, RegionId(0))],
            statements: vec![assign_source],
            terminator: HirTerminator::Jump {
                target: BlockId(1),
                args: vec![LocalId(0)],
            },
        },
        HirBlock {
            id: BlockId(1),
            region: RegionId(0),
            locals: vec![local(1, types.int, RegionId(0))],
            statements: vec![],
            terminator: HirTerminator::Return(unit_expression(2, types.unit, RegionId(0))),
        },
    ];

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[(LocalId(0), "source"), (LocalId(1), "alias_param")],
    );

    let mut report = BorrowCheckReport::default();
    report.analysis.block_entry_states.insert(
        BlockId(1),
        BorrowStateSnapshot {
            locals: vec![LocalBorrowSnapshot {
                local: LocalId(1),
                mode: LocalMode::ALIAS,
                alias_roots: vec![],
            }],
        },
    );

    let output = lower_hir_to_js(
        &module,
        &report,
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");
    let destination_name = expected_dev_local_name("alias_param", 1);

    assert!(
        output
            .source
            .contains(&format!("__moth_write({destination_name}, __jump_arg_0);")),
        "alias-only jump-arg destinations must use __moth_write at block entry"
    );
    assert!(
        !output.source.contains(&format!(
            "__moth_assign_value({destination_name}, __jump_arg_0);"
        )),
        "alias-only jump-arg destinations must not use __moth_assign_value"
    );
}

// ---------------------------------------------------------------------------

// Dispatcher / structured lowering regression tests [cfg]
// ---------------------------------------------------------------------------

/// Verifies that a fallible-returning function with a cyclic CFG wraps the dispatcher
/// in a try/catch, not just a structured body. [cfg] [result]
#[test]
fn dispatcher_with_fallible_return_wraps_dispatcher_in_try_catch() {
    let mut string_table = StringTable::new();
    let (mut type_environment, types) = build_type_environment();

    let fallible_return_type = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier),
        Box::new([types.string, types.string]),
    );

    let loop_assign = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: string_expression(1, "loop_body", types.string, RegionId(0)),
        },
        2,
    );

    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![local(0, types.string, RegionId(0))],
            statements: vec![],
            terminator: HirTerminator::Jump {
                target: BlockId(1),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(1),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::If {
                condition: bool_expression(2, true, types.boolean, RegionId(0)),
                then_block: BlockId(2),
                else_block: BlockId(3),
            },
        },
        HirBlock {
            id: BlockId(2),
            region: RegionId(0),
            locals: vec![],
            statements: vec![loop_assign],
            terminator: HirTerminator::Jump {
                target: BlockId(1),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(3),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Return(expression(
                3,
                HirExpressionKind::VariantConstruct {
                    carrier: HirVariantCarrier::Fallible,
                    variant_index: 0,
                    fields: vec![HirVariantField {
                        name: Some(string_table.intern("value")),
                        value: string_expression(4, "done", types.string, RegionId(0)),
                    }],
                },
                fallible_return_type,
                RegionId(0),
                ValueKind::RValue,
            )),
        },
    ];

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: fallible_return_type,
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[(LocalId(0), "label")],
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
        output.source.contains("switch (__bb"),
        "cyclic CFG must use dispatcher"
    );

    let try_pos = output
        .source
        .find("try {")
        .expect("Fallible function must emit try/catch wrapper");
    let while_pos = output
        .source
        .find("while (true)")
        .expect("dispatcher must emit while (true)");
    assert!(
        try_pos < while_pos,
        "try/catch must wrap the dispatcher, not the other way around"
    );

    assert!(
        output.source.contains("} catch (__moth_err) {"),
        "Fallible function must emit catch block for propagation sentinel"
    );
    assert!(
        output
            .source
            .contains("return { tag: \"err\", value: __moth_err.value };"),
        "catch block must re-wrap propagated errors into a fallible carrier"
    );
}

/// Verifies that many independent acyclic if-else blocks in one function stay structured
/// and do not accidentally fall back to the dispatcher. [cfg]
#[test]
fn multiple_acyclic_if_blocks_stay_structured() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    // Four sequential if-else blocks, each with simple branches.
    // Block 0: if -> 1 else 2
    // Block 1: assign "a", jump 3
    // Block 2: assign "b", jump 3
    // Block 3: if -> 4 else 5
    // Block 4: assign "c", jump 6
    // Block 5: assign "d", jump 6
    // Block 6: if -> 7 else 8
    // Block 7: assign "e", jump 9
    // Block 8: assign "f", jump 9
    // Block 9: if -> 10 else 11
    // Block 10: assign "g", jump 12
    // Block 11: assign "h", jump 12
    // Block 12: return

    let assign_a = statement(
        1,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: string_expression(1, "a", types.string, RegionId(0)),
        },
        1,
    );
    let assign_b = statement(
        2,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: string_expression(2, "b", types.string, RegionId(0)),
        },
        1,
    );
    let assign_c = statement(
        3,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: string_expression(3, "c", types.string, RegionId(0)),
        },
        1,
    );
    let assign_d = statement(
        4,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: string_expression(4, "d", types.string, RegionId(0)),
        },
        1,
    );
    let assign_e = statement(
        5,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: string_expression(5, "e", types.string, RegionId(0)),
        },
        1,
    );
    let assign_f = statement(
        6,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: string_expression(6, "f", types.string, RegionId(0)),
        },
        1,
    );
    let assign_g = statement(
        7,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: string_expression(7, "g", types.string, RegionId(0)),
        },
        1,
    );
    let assign_h = statement(
        8,
        HirStatementKind::Assign {
            target: HirPlace::Local(LocalId(0)),
            value: string_expression(8, "h", types.string, RegionId(0)),
        },
        1,
    );

    let blocks = vec![
        HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![local(0, types.string, RegionId(0))],
            statements: vec![],
            terminator: HirTerminator::If {
                condition: bool_expression(9, true, types.boolean, RegionId(0)),
                then_block: BlockId(1),
                else_block: BlockId(2),
            },
        },
        HirBlock {
            id: BlockId(1),
            region: RegionId(0),
            locals: vec![],
            statements: vec![assign_a],
            terminator: HirTerminator::Jump {
                target: BlockId(3),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(2),
            region: RegionId(0),
            locals: vec![],
            statements: vec![assign_b],
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
            terminator: HirTerminator::If {
                condition: bool_expression(10, true, types.boolean, RegionId(0)),
                then_block: BlockId(4),
                else_block: BlockId(5),
            },
        },
        HirBlock {
            id: BlockId(4),
            region: RegionId(0),
            locals: vec![],
            statements: vec![assign_c],
            terminator: HirTerminator::Jump {
                target: BlockId(6),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(5),
            region: RegionId(0),
            locals: vec![],
            statements: vec![assign_d],
            terminator: HirTerminator::Jump {
                target: BlockId(6),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(6),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::If {
                condition: bool_expression(11, true, types.boolean, RegionId(0)),
                then_block: BlockId(7),
                else_block: BlockId(8),
            },
        },
        HirBlock {
            id: BlockId(7),
            region: RegionId(0),
            locals: vec![],
            statements: vec![assign_e],
            terminator: HirTerminator::Jump {
                target: BlockId(9),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(8),
            region: RegionId(0),
            locals: vec![],
            statements: vec![assign_f],
            terminator: HirTerminator::Jump {
                target: BlockId(9),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(9),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::If {
                condition: bool_expression(12, true, types.boolean, RegionId(0)),
                then_block: BlockId(10),
                else_block: BlockId(11),
            },
        },
        HirBlock {
            id: BlockId(10),
            region: RegionId(0),
            locals: vec![],
            statements: vec![assign_g],
            terminator: HirTerminator::Jump {
                target: BlockId(12),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(11),
            region: RegionId(0),
            locals: vec![],
            statements: vec![assign_h],
            terminator: HirTerminator::Jump {
                target: BlockId(12),
                args: vec![],
            },
        },
        HirBlock {
            id: BlockId(12),
            region: RegionId(0),
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Return(unit_expression(13, types.unit, RegionId(0))),
        },
    ];

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        "main",
        blocks,
        function,
        &[(LocalId(0), "result")],
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
        !output.source.contains("switch (__bb"),
        "acyclic CFG with many simple if-else blocks must stay structured"
    );
    assert!(
        !output.source.contains("while (true)"),
        "acyclic CFG must not use dispatcher"
    );

    let if_count = output.source.matches("if (").count();
    assert!(
        if_count >= 4,
        "expected at least 4 structured if statements, found {if_count}"
    );
}
