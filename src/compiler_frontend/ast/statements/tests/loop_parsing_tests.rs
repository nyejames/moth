//! Loop parsing regression tests.
//!
//! WHAT: validates conditional/range/collection loop AST shapes and loop-header diagnostics.
//! WHY: loop lowering depends on parser output staying stable across the new loop header syntax.

use super::*;
use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::RangeEndKind;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidLoopHeaderReason, ReservedNameOwner, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::tests::ast_fixture_support::function_body_by_name;
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};

fn loop_fixture_source(loop_body_source: &str) -> String {
    let indented_body = loop_body_source
        .lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n");

    format!("loop_test ||:\n{indented_body}\n;\n\nloop_test()\n")
}

fn parse_loop_fixture(loop_body_source: &str) -> (Ast, PathInternerFork, StringTable) {
    parse_single_file_ast(&loop_fixture_source(loop_body_source))
}

fn parse_loop_fixture_diagnostic(loop_body_source: &str) -> DiagnosticPayload {
    parse_single_file_ast_diagnostic(&loop_fixture_source(loop_body_source)).payload
}

fn loop_function_body<'a>(
    ast: &'a Ast,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> &'a [AstNode] {
    function_body_by_name(ast, path_fork, string_table, "loop_test")
}

// --------------------------
//  Conditional loops
// --------------------------

#[test]
fn parses_conditional_loop_without_bindings() {
    let (ast, path_fork, string_table) =
        parse_loop_fixture("counter ~= 0\nloop counter < 3:\n    counter = counter + 1\n;");

    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::WhileLoop(condition, loop_body) = &body[1].kind else {
        panic!("expected conditional loop in function body");
    };

    assert!(matches!(condition.diagnostic_type, DataType::Bool));
    assert_eq!(loop_body.len(), 1);
    assert!(matches!(loop_body[0].kind, NodeKind::Assignment { .. }));
}

// --------------------------
//  Range loops
// --------------------------

#[test]
fn parses_range_loop_with_pipe_binding() {
    let (ast, path_fork, string_table) =
        parse_loop_fixture("sum ~= 0\nloop 1 to & 5 by 2 |i|:\n    sum = sum + i\n;");

    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::RangeLoop {
        bindings,
        range,
        body: loop_body,
    } = &body[1].kind
    else {
        panic!("expected range loop in function body");
    };

    assert_eq!(
        bindings
            .item
            .as_ref()
            .and_then(|binding| path_fork.component(binding.id).map(|id| string_table.resolve(id))),
        Some("i")
    );
    assert!(bindings.index.is_none());
    assert_eq!(range.end_kind, RangeEndKind::Inclusive);
    assert!(matches!(range.start.kind, ExpressionKind::Int(1)));
    assert!(matches!(range.end.kind, ExpressionKind::Int(5)));
    assert!(matches!(
        range.step.as_ref().map(|expr| &expr.kind),
        Some(ExpressionKind::Int(2))
    ));
    assert_eq!(loop_body.len(), 1);
}

#[test]
fn parses_range_loop_with_value_and_index_bindings() {
    let (ast, path_fork, string_table) =
        parse_loop_fixture("sum ~= 0\nloop 0 to 4 |value, index|:\n    sum = sum + value\n;");

    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::RangeLoop { bindings, .. } = &body[1].kind else {
        panic!("expected range loop in function body");
    };

    assert_eq!(
        bindings
            .item
            .as_ref()
            .and_then(|binding| path_fork.component(binding.id).map(|id| string_table.resolve(id))),
        Some("value")
    );
    assert_eq!(
        bindings
            .index
            .as_ref()
            .and_then(|binding| path_fork.component(binding.id).map(|id| string_table.resolve(id))),
        Some("index")
    );
}

#[test]
fn range_index_binding_has_int_type() {
    let (ast, path_fork, string_table) =
        parse_loop_fixture("sum ~= 0\nloop 0 to 4 |value, index|:\n    sum = sum + value\n;");
    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::RangeLoop { bindings, .. } = &body[1].kind else {
        panic!("expected range loop in function body");
    };

    assert!(matches!(
        bindings
            .index
            .as_ref()
            .map(|binding| &binding.value.diagnostic_type),
        Some(DataType::Int)
    ));
}

// --------------------------
//  Collection loops
// --------------------------

#[test]
fn parses_collection_loop_with_pipe_item_binding() {
    let (ast, path_fork, string_table) =
        parse_loop_fixture("items = {1, 2, 3}\nloop items |item|:\n    io.line([: [item]])\n;");

    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::CollectionLoop {
        bindings,
        iterable,
        body: loop_body,
    } = &body[1].kind
    else {
        panic!("expected collection loop in function body");
    };

    assert_eq!(
        bindings
            .item
            .as_ref()
            .and_then(|binding| path_fork.component(binding.id).map(|id| string_table.resolve(id))),
        Some("item")
    );
    assert!(bindings.index.is_none());
    assert!(matches!(iterable.kind, ExpressionKind::Reference(_)));
    assert!(iterable.diagnostic_type.is_collection());
    assert_eq!(loop_body.len(), 1);
}

#[test]
fn parses_collection_loop_with_item_and_index_pipe_bindings() {
    let (ast, path_fork, string_table) = parse_loop_fixture(
        "items = {1, 2, 3}\nloop items |item, index|:\n    io.line([: [item]])\n;",
    );

    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::CollectionLoop { bindings, .. } = &body[1].kind else {
        panic!("expected collection loop in function body");
    };

    assert_eq!(
        bindings
            .item
            .as_ref()
            .and_then(|binding| path_fork.component(binding.id).map(|id| string_table.resolve(id))),
        Some("item")
    );
    assert_eq!(
        bindings
            .index
            .as_ref()
            .and_then(|binding| path_fork.component(binding.id).map(|id| string_table.resolve(id))),
        Some("index")
    );
}

#[test]
fn collection_index_binding_has_int_type() {
    let (ast, path_fork, string_table) = parse_loop_fixture(
        "items = {1, 2, 3}\nloop items |item, index|:\n    io.line([: [item]])\n;",
    );
    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::CollectionLoop { bindings, .. } = &body[1].kind else {
        panic!("expected collection loop in function body");
    };

    assert!(matches!(
        bindings
            .index
            .as_ref()
            .map(|binding| &binding.value.diagnostic_type),
        Some(DataType::Int)
    ));
}

// --------------------------
//  Legacy syntax rejections
// --------------------------

#[test]
fn rejects_old_in_loop_syntax_with_migration_error() {
    let payload = parse_loop_fixture_diagnostic("loop i in 0 to 3:\n    io.line([: [i]])\n;");

    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidLoopHeader {
            reason: InvalidLoopHeaderReason::RemovedInSyntax,
        }
    ));
}

// --------------------------
//  Bare binding rejections
// --------------------------

#[test]
fn rejects_collection_loop_with_bare_single_binding() {
    let payload = parse_loop_fixture_diagnostic(
        "items = {1, 2, 3}\nloop items item:\n    io.line([: [item]])\n;",
    );

    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidLoopHeader {
            reason: InvalidLoopHeaderReason::BareSingleBinding,
        }
    ));
}

#[test]
fn rejects_collection_loop_with_bare_dual_bindings() {
    let payload = parse_loop_fixture_diagnostic(
        "items = {1, 2, 3}\nloop items item, index:\n    io.line([: [item]])\n;",
    );

    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidLoopHeader {
            reason: InvalidLoopHeaderReason::BareDualBinding,
        }
    ));
}

#[test]
fn rejects_range_loop_with_bare_single_binding() {
    let payload = parse_loop_fixture_diagnostic("loop 0 to 10 i:\n    io.line([: [i]])\n;");

    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidLoopHeader {
            reason: InvalidLoopHeaderReason::BareSingleBinding,
        }
    ));
}

#[test]
fn rejects_range_loop_with_bare_dual_bindings() {
    let payload = parse_loop_fixture_diagnostic("loop 0 to 10 i, index:\n    io.line([: [i]])\n;");

    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidLoopHeader {
            reason: InvalidLoopHeaderReason::BareDualBinding,
        }
    ));
}

// --------------------------
//  Loops without bindings
// --------------------------

#[test]
fn parses_collection_loop_without_bindings() {
    let (ast, path_fork, string_table) =
        parse_loop_fixture("count ~= 0\nitems = {1, 2, 3}\nloop items:\n    count = count + 1\n;");
    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::CollectionLoop { bindings, .. } = &body[2].kind else {
        panic!("expected collection loop in function body");
    };

    assert!(bindings.item.is_none());
    assert!(bindings.index.is_none());
}

#[test]
fn parses_range_loop_without_bindings() {
    let (ast, path_fork, string_table) = parse_loop_fixture("loop 0 to 10:\n    io.line([: [1]])\n;");
    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::RangeLoop { bindings, .. } = &body[0].kind else {
        panic!("expected range loop in function body");
    };

    assert!(bindings.item.is_none());
    assert!(bindings.index.is_none());
}

// --------------------------
//  Binding list validation
// --------------------------

#[test]
fn rejects_empty_loop_binding_list() {
    let payload = parse_loop_fixture_diagnostic(
        "items = {1, 2, 3}\nloop items ||:\n    io.line([: [items]])\n;",
    );

    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidLoopHeader {
            reason: InvalidLoopHeaderReason::EmptyBindingList,
        }
    ));
}

#[test]
fn rejects_more_than_two_loop_bindings() {
    let payload = parse_loop_fixture_diagnostic(
        "items = {1, 2, 3}\nloop items |a, b, c|:\n    io.line([: [a]])\n;",
    );

    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidLoopHeader {
            reason: InvalidLoopHeaderReason::TooManyBindings,
        }
    ));
}

#[test]
fn rejects_duplicate_loop_binding_names() {
    let payload = parse_loop_fixture_diagnostic(
        "items = {1, 2, 3}\nloop items |item, item|:\n    io.line([: [item]])\n;",
    );

    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidLoopHeader {
            reason: InvalidLoopHeaderReason::DuplicateBindingName,
        }
    ));
}

#[test]
fn rejects_loop_binding_shadowing_existing_name() {
    let payload = parse_loop_fixture_diagnostic(
        "items = {1, 2, 3}\nitem = 0\nloop items |item|:\n    io.line([: [item]])\n;",
    );

    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidLoopHeader {
            reason: InvalidLoopHeaderReason::BindingAlreadyDeclared,
        }
    ));
}

#[test]
fn rejects_keyword_shadow_loop_binding_names() {
    let payload = parse_loop_fixture_diagnostic(
        "items = {1, 2, 3}\nloop items |_if|:\n    io.line([: [items]])\n;",
    );

    assert!(matches!(
        payload,
        DiagnosticPayload::ReservedNameCollision {
            reserved_by: ReservedNameOwner::Keyword,
            ..
        }
    ));
}

// --------------------------
//  Loop source type checks
// --------------------------

#[test]
fn rejects_collection_loop_on_non_collection_expression() {
    let payload =
        parse_loop_fixture_diagnostic("value = 3\nloop value |item|:\n    io.line([: [item]])\n;");

    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidLoopHeader {
            reason: InvalidLoopHeaderReason::CollectionSourceNotCollection { .. },
        }
    ));
}

#[test]
fn rejects_non_boolean_conditional_loop_condition() {
    let payload = parse_loop_fixture_diagnostic("loop 1 + 2:\n    io.line([: [1]])\n;");

    assert!(matches!(
        payload,
        DiagnosticPayload::TypeMismatch {
            context: TypeMismatchContext::Condition,
            ..
        }
    ));
}

// --------------------------
//  Range loop edge cases
// --------------------------

#[test]
fn parses_inclusive_range_loop_with_tight_ampersand() {
    let (ast, path_fork, string_table) =
        parse_loop_fixture("sum ~= 0\nloop 0 to &5 |i|:\n    sum = sum + i\n;");

    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::RangeLoop { range, .. } = &body[1].kind else {
        panic!("expected range loop in function body");
    };

    assert_eq!(range.end_kind, RangeEndKind::Inclusive);
}

#[test]
fn parses_omitted_start_exclusive_range_loop() {
    let (ast, path_fork, string_table) = parse_loop_fixture("sum ~= 0\nloop to 5 |i|:\n    sum = sum + i\n;");

    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::RangeLoop { range, .. } = &body[1].kind else {
        panic!("expected range loop in function body");
    };

    assert!(matches!(range.start.kind, ExpressionKind::Int(0)));
    assert_eq!(range.end_kind, RangeEndKind::Exclusive);
    assert!(matches!(range.end.kind, ExpressionKind::Int(5)));
}

#[test]
fn parses_omitted_start_inclusive_range_loop() {
    let (ast, path_fork, string_table) =
        parse_loop_fixture("sum ~= 0\nloop to & 5 |i|:\n    sum = sum + i\n;");

    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::RangeLoop { range, .. } = &body[1].kind else {
        panic!("expected range loop in function body");
    };

    assert!(matches!(range.start.kind, ExpressionKind::Int(0)));
    assert_eq!(range.end_kind, RangeEndKind::Inclusive);
    assert!(matches!(range.end.kind, ExpressionKind::Int(5)));
}

#[test]
fn rejects_range_loop_missing_end_bound() {
    assert_missing_range_end_bound("loop 0 to |i|:\n    io.line([: [i]])\n;");
}

#[test]
fn rejects_omitted_start_range_loop_missing_end_bound() {
    assert_missing_range_end_bound("loop to:\n    io.line([: [1]])\n;");
}

#[test]
fn rejects_omitted_start_range_loop_ampersand_without_end_bound() {
    assert_missing_range_end_bound("loop to &:\n    io.line([: [1]])\n;");
}

#[test]
fn rejects_explicit_start_range_loop_ampersand_without_end_bound() {
    assert_missing_range_end_bound("loop 0 to &:\n    io.line([: [1]])\n;");
}

#[test]
fn rejects_range_loop_by_without_step() {
    let payload = parse_loop_fixture_diagnostic("loop 0 to 10 by |i|:\n    io.line([: [i]])\n;");

    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidLoopHeader {
            reason: InvalidLoopHeaderReason::MissingRangeStep,
        }
    ));
}

#[test]
fn rejects_zero_step_literal() {
    let payload = parse_loop_fixture_diagnostic("loop 0 to 10 by 0 |i|:\n    io.line([: [i]])\n;");

    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidLoopHeader {
            reason: InvalidLoopHeaderReason::ZeroRangeStep,
        }
    ));
}

#[test]
fn rejects_float_range_without_by() {
    let payload = parse_loop_fixture_diagnostic("loop 0.0 to 1.0 |t|:\n    io.line([: [t]])\n;");

    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidLoopHeader {
            reason: InvalidLoopHeaderReason::FloatRangeMissingStep,
        }
    ));
}

// --------------------------
//  Bare binding edge cases
// --------------------------

#[test]
fn rejects_missing_comma_between_bare_dual_bindings() {
    let payload = parse_loop_fixture_diagnostic(
        "items = {1, 2, 3}\nloop items item index:\n    io.line([: [item]])\n;",
    );

    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidLoopHeader {
            reason: InvalidLoopHeaderReason::BareDualBinding,
        }
    ));
}

#[test]
fn rejects_missing_closing_pipe_in_loop_bindings() {
    let payload = parse_loop_fixture_diagnostic(
        "items = {1, 2, 3}\nloop items |item:\n    io.line([: [item]])\n;",
    );

    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidLoopHeader {
            reason: InvalidLoopHeaderReason::MissingClosingPipe,
        }
    ));
}

#[test]
fn rejects_range_loop_with_bare_binding_after_complex_step_expression() {
    let payload = parse_loop_fixture_diagnostic(
        "limit = 8\nstep = 2\nsum ~= 0\nloop 0 to limit by step i:\n    sum = sum + i\n;",
    );

    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidLoopHeader {
            reason: InvalidLoopHeaderReason::BareSingleBinding,
        }
    ));
}

#[test]
fn operator_tail_does_not_trigger_bare_loop_binding_diagnostic() {
    let payload =
        parse_loop_fixture_diagnostic("a = 1\nb = 2\nloop a + b value:\n    io.line([: [1]])\n;");

    assert!(!matches!(
        payload,
        DiagnosticPayload::InvalidLoopHeader {
            reason: InvalidLoopHeaderReason::BareSingleBinding
                | InvalidLoopHeaderReason::BareDualBinding,
        }
    ));
}

fn assert_missing_range_end_bound(source: &str) {
    let payload = parse_loop_fixture_diagnostic(source);

    assert!(matches!(
        payload,
        DiagnosticPayload::InvalidLoopHeader {
            reason: InvalidLoopHeaderReason::MissingRangeEndBound,
        }
    ));
}
