//! HIR display regression tests.
//!
//! WHAT: pins debug-display rendering for HIR-only constructs.
//! WHY: display output is used while auditing lowering and borrow behavior, so embedded message
//! text must remain unambiguous when it contains quotes or control characters.

use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::expression_store::HirExpressionStore;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_display::{HirDisplayContext, HirDisplayOptions};
use crate::compiler_frontend::hir::ids::{BlockId, HirNodeId, LocalId, RegionId};
use crate::compiler_frontend::hir::numeric::NumericFailureMode;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{
    HirAssertionMessageEvaluation, HirJumpArgument, HirTerminator,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn assertion_failure_message_display_escapes_debug_text() {
    let string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let display = HirDisplayContext::new(&string_table, &path_fork);

    let mut expressions = HirExpressionStore::default();
    let message = expressions
        .append_expression(HirExpression {
            kind: HirExpressionKind::StringLiteral("quoted \"message\"\nnext".to_owned()),
            ty: TypeId(0),
            value_kind: ValueKind::Const,
            region: RegionId(0),
            span: None,
        })
        .expect("string expression should append");
    let rendered = display.render_terminator(
        &HirTerminator::AssertFailure {
            message,
            message_evaluation: HirAssertionMessageEvaluation::Folded,
        },
        &expressions,
    );

    assert_eq!(
        rendered,
        "assert_failure [v0] \"quoted \\\"message\\\"\\nnext\" : t0 [Folded]"
    );
}

#[test]
fn runtime_failure_message_display_escapes_debug_text() {
    let string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let display = HirDisplayContext::new(&string_table, &path_fork);
    let rendered = display.render_terminator(
        &HirTerminator::RuntimeFailure {
            message: "quoted \"message\"\nnext".to_owned(),
            cause: None,
        },
        &HirExpressionStore::default(),
    );

    assert_eq!(
        rendered,
        "runtime_failure \"quoted \\\"message\\\"\\nnext\""
    );
}

fn float_expression(value: f64) -> HirExpression {
    HirExpression {
        kind: HirExpressionKind::Float(value),
        ty: TypeId(0),
        value_kind: ValueKind::RValue,
        region: RegionId(0),
        span: None,
    }
}

fn float_statement(kind: HirStatementKind) -> HirStatement {
    HirStatement {
        id: HirNodeId(0),
        kind,
        span: None,
    }
}

fn terse_display_context<'a>(
    string_table: &'a StringTable,
    path_fork: &'a PathInternerFork,
) -> HirDisplayContext<'a> {
    HirDisplayContext::new(string_table, path_fork).with_options(HirDisplayOptions {
        include_ids: false,
        include_types: false,
        include_value_kinds: false,
        include_regions: false,
        multiline_match_arms: false,
    })
}

#[test]
fn hir_display_renders_format_float_trap() {
    let string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let display = terse_display_context(&string_table, &path_fork);

    let mut expressions = HirExpressionStore::default();
    let source = expressions
        .append_expression(float_expression(1.5))
        .expect("float should append");
    let rendered = display.render_statement(
        &float_statement(HirStatementKind::FormatFloat {
            source,
            failure_mode: NumericFailureMode::Trap,
            result: HirLocalDestination::Define(LocalId(9000)),
        }),
        &expressions,
    );

    assert_eq!(rendered, "define l9000 = format_float_trap(1.5)");
}

#[test]
fn hir_display_renders_format_float_return_error() {
    let string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let display = terse_display_context(&string_table, &path_fork);

    let mut expressions = HirExpressionStore::default();
    let source = expressions
        .append_expression(float_expression(-0.25))
        .expect("float should append");
    let rendered = display.render_statement(
        &float_statement(HirStatementKind::FormatFloat {
            source,
            failure_mode: NumericFailureMode::ReturnError,
            result: HirLocalDestination::Define(LocalId(9001)),
        }),
        &expressions,
    );

    assert_eq!(rendered, "define l9001 = format_float_err(-0.25)");
}

#[test]
fn hir_display_renders_validate_float_trap() {
    let string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let display = terse_display_context(&string_table, &path_fork);

    let mut expressions = HirExpressionStore::default();
    let source = expressions
        .append_expression(float_expression(2.5))
        .expect("float should append");
    let rendered = display.render_statement(
        &float_statement(HirStatementKind::ValidateFloat {
            source,
            failure_mode: NumericFailureMode::Trap,
            result: HirLocalDestination::Define(LocalId(9002)),
        }),
        &expressions,
    );

    assert_eq!(rendered, "define l9002 = validate_float_trap(2.5)");
}

#[test]
fn hir_display_renders_validate_float_return_error() {
    let string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let display = terse_display_context(&string_table, &path_fork);

    let mut expressions = HirExpressionStore::default();
    let source = expressions
        .append_expression(float_expression(0.0))
        .expect("float should append");
    let rendered = display.render_statement(
        &float_statement(HirStatementKind::ValidateFloat {
            source,
            failure_mode: NumericFailureMode::ReturnError,
            result: HirLocalDestination::Define(LocalId(9003)),
        }),
        &expressions,
    );

    assert_eq!(rendered, "define l9003 = validate_float_err(0)");
}

#[test]
fn hir_display_marks_local_definitions_and_place_updates() {
    let string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let display = terse_display_context(&string_table, &path_fork);
    let mut expressions = HirExpressionStore::default();
    let value = expressions
        .append_expression(float_expression(1.5))
        .expect("float should append");

    let definition = display.render_statement(
        &float_statement(HirStatementKind::Write {
            target: HirWriteTarget::DefineLocal(LocalId(9010)),
            value,
        }),
        &expressions,
    );
    let update = display.render_statement(
        &float_statement(HirStatementKind::Write {
            target: HirWriteTarget::AssignPlace(HirPlace::local(LocalId(9011))),
            value,
        }),
        &expressions,
    );

    assert_eq!(definition, "define l9010 = 1.5");
    assert_eq!(update, "update l9011 = 1.5");
}

#[test]
fn hir_display_shows_cfg_edge_value_and_definition_locals() {
    let string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let display = terse_display_context(&string_table, &path_fork);

    let rendered = display.render_terminator(
        &HirTerminator::Jump {
            target: BlockId(7),
            args: vec![HirJumpArgument {
                source: LocalId(9012),
                destination: LocalId(9013),
            }],
        },
        &HirExpressionStore::default(),
    );

    assert_eq!(rendered, "jump bb7(l9012 -> define l9013)");
}
