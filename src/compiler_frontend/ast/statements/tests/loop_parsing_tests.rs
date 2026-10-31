//! Loop parsing regression tests.
//!
//! WHAT: validates conditional/range/collection loop AST shapes and loop-header diagnostics.
//! WHY: loop lowering depends on parser output staying stable across the new loop header syntax.

use super::*;
use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::RangeEndKind;
use crate::compiler_frontend::ast::const_values::ConstValuePayload;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExprId, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    calls_data::{InvalidRangeOperandView, InvalidValueReceiverView},
    names_declarations::ReservedNameCollisionView,
    types_borrows_targets::TypeMismatchView,
};
use crate::compiler_frontend::compiler_messages::{
    InvalidRangeOperandReason, RangeOperandKind, ReservedNameOwner, TypeMismatchContext,
    ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::datatypes::{TypeId, builtin_type_ids};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId};
use crate::compiler_frontend::tests::ast_fixture_support::function_body_by_name;
use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, ast_error_primary_span, first_ast_error_view, parse_single_file_ast,
    parse_single_file_ast_diagnostic,
};
use crate::compiler_frontend::tokenizer::tokens::{TestSourceTokensBuilder, TokenTag};
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use std::sync::Arc;

#[test]
fn rejects_non_single_loop_sources_and_range_operands() {
    let producers = "pair || -> I32, I32:\nreturn 1, 2\n;\nempty ||:\nreturn\n;\n";
    for (header, receiver, produced_count) in [
        ("pair()", ValueReceiverContext::General, 2),
        ("empty()", ValueReceiverContext::General, 0),
        ("pair() |item|", ValueReceiverContext::General, 2),
        ("empty() |item|", ValueReceiverContext::General, 0),
        ("0 to pair()", ValueReceiverContext::OperatorOperand, 2),
        ("0 to empty()", ValueReceiverContext::OperatorOperand, 0),
        ("pair() to 5", ValueReceiverContext::OperatorOperand, 2),
        ("empty() to 5", ValueReceiverContext::OperatorOperand, 0),
        ("0 to 5 by pair()", ValueReceiverContext::OperatorOperand, 2),
        (
            "0 to 5 by empty()",
            ValueReceiverContext::OperatorOperand,
            0,
        ),
    ] {
        let source = format!("{producers}loop {header}:\n;\n");
        let failure = parse_single_file_ast_diagnostic(&source);
        assert_ast_diagnostic(&failure, "MOTH-RULE-0087", None);
        let diagnostic = first_ast_error_view(&failure);
        let receiver_facts = diagnostic
            .family_view::<InvalidValueReceiverView>()
            .expect("the first error is an invalid value receiver");
        assert_eq!(receiver_facts.receiver, receiver, "{source}");
        assert_eq!(receiver_facts.target_count, 1, "{source}");
        assert_eq!(receiver_facts.produced_count, produced_count, "{source}");
    }
}

#[test]
fn malformed_loop_header_payload_stays_infrastructure_error_while_scanning_for_colon() {
    let source = SourceId::COMPILATION_ROOT;
    let mut builder = TestSourceTokensBuilder::new(source);
    builder
        .push_bool(TokenTag::BOOL_LITERAL, true, LocalSpan::source_start())
        .expect("valid loop-header fixture token should build");
    builder
        .push_static(TokenTag::EOF, LocalSpan::source_start())
        .expect("valid loop-header fixture token should build");
    let mut owner = builder
        .finish()
        .expect("valid loop-header fixture should build a canonical source owner");
    Arc::get_mut(&mut owner)
        .expect("the test owner should remain uniquely owned")
        .corrupt_payload_for_test(0, TokenTag::NUMERIC_LITERAL);
    let range = owner
        .full_range()
        .expect("the canonical test owner should expose a full range");
    let mut token_stream = AstCursor::from_source_tokens(&owner, range)
        .expect("the malformed canonical owner should still construct a cursor");

    let error = super::find_loop_header_colon_index(&mut token_stream)
        .expect_err("a malformed trusted payload must not become MissingColon");

    assert!(matches!(error, ExpressionParseError::Infrastructure(_)));
}

#[test]
fn malformed_loop_header_payload_stays_infrastructure_error_while_checking_empty_window() {
    let source = SourceId::COMPILATION_ROOT;
    let mut builder = TestSourceTokensBuilder::new(source);
    builder
        .push_bool(TokenTag::BOOL_LITERAL, true, LocalSpan::source_start())
        .expect("valid loop-header fixture token should build");
    builder
        .push_static(TokenTag::EOF, LocalSpan::source_start())
        .expect("valid loop-header fixture token should build");
    let mut owner = builder
        .finish()
        .expect("valid loop-header fixture should build a canonical source owner");
    Arc::get_mut(&mut owner)
        .expect("the test owner should remain uniquely owned")
        .corrupt_payload_for_test(0, TokenTag::NUMERIC_LITERAL);
    let range = owner
        .full_range()
        .expect("the canonical test owner should expose a full range");
    let mut window = AstCursor::from_source_tokens(&owner, range)
        .expect("the malformed canonical owner should still construct a cursor");

    let error = super::is_empty_header_window(&mut window)
        .expect_err("a malformed trusted payload must not become EmptyHeader");

    assert!(matches!(error, ExpressionParseError::Infrastructure(_)));
}

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

fn assert_loop_fixture_diagnostic(
    loop_body_source: &str,
    expected_code: &str,
    expected_reason_key: Option<&str>,
) {
    let failure = parse_single_file_ast_diagnostic(&loop_fixture_source(loop_body_source));
    assert_ast_diagnostic(&failure, expected_code, expected_reason_key);
}

fn loop_function_body<'a>(
    ast: &'a Ast,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> &'a [AstNode] {
    function_body_by_name(ast, path_fork, string_table, "loop_test")
}

/// Returns the stored view of a parsed loop-header expression.
fn expression_view(ast: &Ast, id: ExprId) -> ExpressionView<'_> {
    ast.expressions
        .view(id)
        .expect("parsed loop expression should be stored")
}

/// Returns the single value type of a parsed loop-header expression.
fn expression_type_id(ast: &Ast, id: ExprId) -> TypeId {
    expression_view(ast, id)
        .value_type_id()
        .expect("parsed loop expression should produce one value")
}

/// Returns the fixed-scalar value of a parsed loop-header literal, or `None` for other expressions.
fn fixed_scalar_constant(ast: &Ast, id: ExprId) -> Option<FixedScalarValue> {
    let ExpressionOperationView::Constant(constant) = expression_view(ast, id).operation() else {
        return None;
    };
    match ast.const_values.payload(constant)? {
        ConstValuePayload::FixedScalar(value) => Some(*value),
        _ => None,
    }
}

// --------------------------
//  Conditional loops
// --------------------------

#[test]
fn parses_conditional_loop_without_bindings() {
    let (ast, path_fork, string_table) =
        parse_loop_fixture("counter ~= 0\nloop counter < 3:\n    ~counter = counter + 1\n;");

    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::WhileLoop(condition, loop_body) = &body[1].kind else {
        panic!("expected conditional loop in function body");
    };

    assert_eq!(expression_type_id(&ast, *condition), builtin_type_ids::BOOL);
    assert_eq!(loop_body.len(), 1);
    assert!(matches!(loop_body[0].kind, NodeKind::Assignment { .. }));
}

// --------------------------
//  Range loops
// --------------------------

#[test]
fn parses_range_loop_with_pipe_binding() {
    let (ast, path_fork, string_table) =
        parse_loop_fixture("sum ~= 0\nloop 1 to & 5 by 2 |i|:\n    ~sum = sum + i\n;");

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
        bindings.item.as_ref().and_then(|binding| path_fork
            .component(binding.id)
            .map(|id| string_table.resolve(id))),
        Some("i")
    );
    assert!(bindings.index.is_none());
    assert_eq!(range.end_kind, RangeEndKind::Inclusive);
    assert!(matches!(
        fixed_scalar_constant(&ast, range.start),
        Some(value)
            if value.scalar() == FixedScalar::I32 && value.as_i64() == Some(1)
    ));
    assert!(matches!(
        fixed_scalar_constant(&ast, range.end),
        Some(value)
            if value.scalar() == FixedScalar::I32 && value.as_i64() == Some(5)
    ));
    assert!(matches!(
        range.step.and_then(|step| fixed_scalar_constant(&ast, step)),
        Some(value)
            if value.scalar() == FixedScalar::I32 && value.as_i64() == Some(2)
    ));
    assert_eq!(loop_body.len(), 1);
}

#[test]
fn parses_range_loop_with_value_and_index_bindings() {
    let (ast, path_fork, string_table) =
        parse_loop_fixture("sum ~= 0\nloop 0 to 4 |value, index|:\n    ~sum = sum + value\n;");

    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::RangeLoop { bindings, .. } = &body[1].kind else {
        panic!("expected range loop in function body");
    };

    assert_eq!(
        bindings.item.as_ref().and_then(|binding| path_fork
            .component(binding.id)
            .map(|id| string_table.resolve(id))),
        Some("value")
    );
    assert_eq!(
        bindings.index.as_ref().and_then(|binding| path_fork
            .component(binding.id)
            .map(|id| string_table.resolve(id))),
        Some("index")
    );
}

#[test]
fn range_index_binding_has_i32_type() {
    let (ast, path_fork, string_table) =
        parse_loop_fixture("sum ~= 0\nloop 0 to 4 |value, index|:\n    ~sum = sum + value\n;");
    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::RangeLoop { bindings, .. } = &body[1].kind else {
        panic!("expected range loop in function body");
    };

    assert_eq!(
        bindings.index.as_ref().map(|binding| binding.type_id),
        Some(builtin_type_ids::fixed_scalar(FixedScalar::I32))
    );
}

#[test]
fn range_item_type_uses_canonical_fixed_integer_promotion() {
    let (ast, path_fork, string_table) = parse_loop_fixture(
        "first_bound I8 = 1\nend U16 = 5\nstep U16 = 1\n\
         loop first_bound to end by step |value, index|:\n    io.line([: [value]])\n;",
    );
    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::RangeLoop {
        bindings, range, ..
    } = &body[3].kind
    else {
        panic!("expected fixed integer range loop");
    };

    assert_eq!(
        bindings.item.as_ref().map(|binding| binding.type_id),
        Some(builtin_type_ids::fixed_scalar(FixedScalar::I32))
    );
    assert_eq!(
        bindings.index.as_ref().map(|binding| binding.type_id),
        Some(builtin_type_ids::fixed_scalar(FixedScalar::I32))
    );
    // The spec carries the selected iteration domain: the counter type and the domain must
    // agree, because HIR lowers the loop from the spec's domain alone.
    assert_eq!(range.domain, NumericScalar::Fixed(FixedScalar::I32));
    assert_eq!(
        expression_type_id(&ast, range.start),
        builtin_type_ids::fixed_scalar(FixedScalar::I8)
    );
    assert_eq!(
        expression_type_id(&ast, range.end),
        builtin_type_ids::fixed_scalar(FixedScalar::U16)
    );
    assert!(range.step.as_ref().is_some_and(|step| {
        expression_type_id(&ast, *step) == builtin_type_ids::fixed_scalar(FixedScalar::U16)
    }));
}

#[test]
fn range_item_type_promotes_mixed_i32_u32_to_i64() {
    let (ast, path_fork, string_table) = parse_loop_fixture(
        "first_bound I32 = 1\nend U32 = 5\n\
         loop first_bound to end |value|:\n    io.line([: [value]])\n;",
    );
    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::RangeLoop {
        bindings, range, ..
    } = &body[2].kind
    else {
        panic!("expected mixed fixed integer range loop");
    };

    assert_eq!(
        bindings.item.as_ref().map(|binding| binding.type_id),
        Some(builtin_type_ids::fixed_scalar(FixedScalar::I64))
    );
    assert_eq!(range.domain, NumericScalar::Fixed(FixedScalar::I64));
}

#[test]
fn omitted_range_start_zero_uses_promoted_fixed_domain() {
    let (ast, path_fork, string_table) = parse_loop_fixture(
        "end U64 = 18446744073709551615\nstep U64 = 1\n\
         loop to end by step |value, index|:\n    io.line([: [value]])\n;",
    );
    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::RangeLoop {
        bindings, range, ..
    } = &body[2].kind
    else {
        panic!("expected omitted-start fixed integer range loop");
    };

    assert!(matches!(
        fixed_scalar_constant(&ast, range.start),
        Some(value)
            if value.scalar() == FixedScalar::U64 && value.as_u64() == Some(0)
    ));
    assert_eq!(
        bindings.item.as_ref().map(|binding| binding.type_id),
        Some(builtin_type_ids::fixed_scalar(FixedScalar::U64))
    );
    assert_eq!(range.domain, NumericScalar::Fixed(FixedScalar::U64));
    assert_eq!(
        bindings.index.as_ref().map(|binding| binding.type_id),
        Some(builtin_type_ids::fixed_scalar(FixedScalar::I32))
    );
}

#[test]
fn omitted_range_zero_keeps_endpoint_type_before_step_promotion() {
    let (ast, path_fork, string_table) = parse_loop_fixture(
        "end U16 = 5\nstep I8 = 1\n\
         loop to end by step |value|:\n    io.line([: [value]])\n;",
    );
    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::RangeLoop {
        bindings, range, ..
    } = &body[2].kind
    else {
        panic!("expected omitted-start promoted integer range loop");
    };

    assert!(matches!(
        fixed_scalar_constant(&ast, range.start),
        Some(value)
            if value.scalar() == FixedScalar::U16 && value.as_u64() == Some(0)
    ));
    // The `U16` endpoint and `I8` step mix selects the smallest signed type that contains
    // both complete ranges, so the counter domain is `I32` without a minimum-width floor.
    assert_eq!(
        bindings.item.as_ref().map(|binding| binding.type_id),
        Some(builtin_type_ids::fixed_scalar(FixedScalar::I32))
    );
}

#[test]
fn range_item_type_promotes_f16_with_explicit_step_to_f32() {
    let (ast, path_fork, string_table) = parse_loop_fixture(
        "first_bound F16 = 0.5\nend F32 = 1.0\nstep F16 = 0.25\n\
         loop first_bound to end by step |value|:\n    io.line([: [value]])\n;",
    );
    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::RangeLoop {
        bindings, range, ..
    } = &body[3].kind
    else {
        panic!("expected fixed float range loop");
    };

    assert_eq!(
        bindings.item.as_ref().map(|binding| binding.type_id),
        Some(builtin_type_ids::fixed_scalar(FixedScalar::F32))
    );
    assert_eq!(range.domain, NumericScalar::Fixed(FixedScalar::F32));
}

#[test]
fn rejects_byte_range_operand() {
    let failure = parse_single_file_ast_diagnostic(&loop_fixture_source(
        "end Byte = 1\nloop 0 to end |value|:\n    io.line([: [value]])\n;",
    ));
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0059",
        Some("invalid_range_operand.not_numeric"),
    );
    let operand = diagnostic
        .family_view::<InvalidRangeOperandView>()
        .expect("the first error is an invalid range operand");
    assert_eq!(operand.operand, RangeOperandKind::End);
    assert_eq!(operand.reason, InvalidRangeOperandReason::NotNumeric);
}

/// Parse one range header that must fail on domain selection, returning the blamed role and
/// authored source text under the primary span.
fn range_domain_failure(loop_body_source: &str) -> (RangeOperandKind, String) {
    let source = loop_fixture_source(loop_body_source);
    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0059",
        Some("invalid_range_operand.incompatible_domain"),
    );
    let operand_facts = diagnostic
        .family_view::<InvalidRangeOperandView>()
        .expect("the first error is an invalid range operand");
    assert_eq!(
        operand_facts.reason,
        InvalidRangeOperandReason::IncompatibleDomain
    );
    let range = ast_error_primary_span(diagnostic)
        .expect("range operand diagnostics carry the operand span")
        .resolve_with(ExtendedSpanBuilder::new().resolver_for(SourceId::COMPILATION_ROOT));
    (
        operand_facts.operand,
        source[range.start() as usize..range.end() as usize].to_owned(),
    )
}

#[test]
fn range_step_that_breaks_compatible_bounds_is_blamed_on_the_step() {
    // The bounds agree on their own (`U64`/`U64` and `I64`/`I64`); the explicit step's complete
    // range has no common fixed-integer type with them (`U64` with a signed type needs 65 bits),
    // so the step is the blamed operand regardless of the authored signed/unsigned direction.
    let u64_bounds_i64_step = range_domain_failure(
        "lo U64 = 0\nhi U64 = 10\nstep I64 = 1\n\
         loop lo to hi by step |value|:\n    io.line([: [value]])\n;",
    );
    let i64_bounds_u64_step = range_domain_failure(
        "lo I64 = 0\nhi I64 = 10\nstep U64 = 1\n\
         loop lo to hi by step |value|:\n    io.line([: [value]])\n;",
    );

    for (operand, blamed) in [u64_bounds_i64_step, i64_bounds_u64_step] {
        assert_eq!(operand, RangeOperandKind::Step);
        assert_eq!(blamed, "step");
    }
}

#[test]
fn incompatible_range_bounds_are_blamed_on_the_bounds() {
    let unstepped = range_domain_failure(
        "lo U64 = 0\nhi I64 = 10\nloop lo to hi |value|:\n    io.line([: [value]])\n;",
    );
    // The U64 step matches the start, so only the bounds disagree.
    let stepped = range_domain_failure(
        "lo U64 = 0\nhi I64 = 10\nstep U64 = 1\n\
         loop lo to hi by step |value|:\n    io.line([: [value]])\n;",
    );

    for (operand, blamed) in [unstepped, stepped] {
        assert_eq!(operand, RangeOperandKind::End);
        assert_eq!(blamed, "hi");
    }
}

#[test]
fn float_step_selects_wider_precision_for_mixed_float_bounds_in_either_order() {
    // Fixed integers and binary floats require an explicit conversion to mix, so a range
    // header combines operands within one family. A float step therefore selects the wider
    // *float* precision when the bounds already use mixed float precisions, and the selected
    // counter domain does not depend on the bounds' authored order.
    for (start_type, end_type) in [("F16", "F32"), ("F32", "F16")] {
        let (ast, path_fork, string_table) = parse_loop_fixture(&format!(
            "lo {start_type} = 0.0\nhi {end_type} = 3.0\nstep F16 = 0.5\n\
             loop lo to hi by step |value|:\n    io.line([: [value]])\n;"
        ));
        let body = loop_function_body(&ast, &path_fork, &string_table);
        let NodeKind::RangeLoop { bindings, .. } = &body[3].kind else {
            panic!("expected an F32 range loop for {start_type}/{end_type} bounds");
        };
        let item_type = bindings.item.as_ref().map(|binding| binding.type_id);
        assert_eq!(
            item_type,
            Some(builtin_type_ids::fixed_scalar(FixedScalar::F32)),
            "{start_type}/{end_type} bounds must iterate at the wider F32 precision"
        );
    }
}

#[test]
fn rejects_binary_float_step_for_integer_bounds_without_conversion() {
    // The header's operands must share one family, so an integer bound set cannot take a
    // binary-float step without an explicit conversion and the incompatible step is blamed.
    let failure = parse_single_file_ast_diagnostic(&loop_fixture_source(
        "lo I32 = 0\nhi I32 = 3\nstep F32 = 0.5\n\
         loop lo to hi by step |value|:\n    io.line([: [value]])\n;",
    ));
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0059",
        Some("invalid_range_operand.incompatible_domain"),
    );
    let operand = diagnostic
        .family_view::<InvalidRangeOperandView>()
        .expect("the first error is an invalid range operand");
    assert_eq!(operand.operand, RangeOperandKind::Step);
    assert_eq!(
        operand.reason,
        InvalidRangeOperandReason::IncompatibleDomain
    );
}

#[test]
fn rejects_fixed_float_range_without_by() {
    assert_loop_fixture_diagnostic(
        "first_bound F16 = 0.5\nend F32 = 1.0\n\
         loop first_bound to end |value|:\n    io.line([: [value]])\n;",
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.float_range_missing_step"),
    );
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
        bindings.item.as_ref().and_then(|binding| path_fork
            .component(binding.id)
            .map(|id| string_table.resolve(id))),
        Some("item")
    );
    assert!(bindings.index.is_none());
    assert!(matches!(
        expression_view(&ast, *iterable).operation(),
        ExpressionOperationView::Reference(_)
    ));
    assert!(
        ast.type_environment
            .is_collection(expression_type_id(&ast, *iterable))
    );
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
        bindings.item.as_ref().and_then(|binding| path_fork
            .component(binding.id)
            .map(|id| string_table.resolve(id))),
        Some("item")
    );
    assert_eq!(
        bindings.index.as_ref().and_then(|binding| path_fork
            .component(binding.id)
            .map(|id| string_table.resolve(id))),
        Some("index")
    );
}

#[test]
fn collection_index_binding_has_i32_type() {
    let (ast, path_fork, string_table) = parse_loop_fixture(
        "items = {1, 2, 3}\nloop items |item, index|:\n    io.line([: [item]])\n;",
    );
    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::CollectionLoop { bindings, .. } = &body[1].kind else {
        panic!("expected collection loop in function body");
    };

    assert_eq!(
        bindings.index.as_ref().map(|binding| binding.type_id),
        Some(builtin_type_ids::fixed_scalar(FixedScalar::I32))
    );
}

// --------------------------
//  Bare binding rejections
// --------------------------

#[test]
fn rejects_collection_loop_with_bare_single_binding() {
    assert_loop_fixture_diagnostic(
        "items = {1, 2, 3}\nloop items item:\n    io.line([: [item]])\n;",
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.bare_single_binding"),
    );
}

#[test]
fn rejects_collection_loop_with_bare_dual_bindings() {
    assert_loop_fixture_diagnostic(
        "items = {1, 2, 3}\nloop items item, index:\n    io.line([: [item]])\n;",
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.bare_dual_binding"),
    );
}

#[test]
fn rejects_range_loop_with_bare_single_binding() {
    assert_loop_fixture_diagnostic(
        "loop 0 to 10 i:\n    io.line([: [i]])\n;",
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.bare_single_binding"),
    );
}

#[test]
fn rejects_range_loop_with_bare_dual_bindings() {
    assert_loop_fixture_diagnostic(
        "loop 0 to 10 i, index:\n    io.line([: [i]])\n;",
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.bare_dual_binding"),
    );
}

#[test]
fn rejects_string_literal_as_range_binding() {
    assert_loop_fixture_diagnostic(
        "loop 0 to 10 |\"item\"|:\n    io.line([: [\"value\"]])\n;",
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.binding_must_be_symbol"),
    );
}

// --------------------------
//  Loops without bindings
// --------------------------

#[test]
fn parses_collection_loop_without_bindings() {
    let (ast, path_fork, string_table) =
        parse_loop_fixture("count ~= 0\nitems = {1, 2, 3}\nloop items:\n    ~count = count + 1\n;");
    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::CollectionLoop { bindings, .. } = &body[2].kind else {
        panic!("expected collection loop in function body");
    };

    assert!(bindings.item.is_none());
    assert!(bindings.index.is_none());
}

#[test]
fn parses_range_loop_without_bindings() {
    let (ast, path_fork, string_table) =
        parse_loop_fixture("loop 0 to 10:\n    io.line([: [1]])\n;");
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
    assert_loop_fixture_diagnostic(
        "items = {1, 2, 3}\nloop items ||:\n    io.line([: [items]])\n;",
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.empty_binding_list"),
    );
}

#[test]
fn rejects_more_than_two_loop_bindings() {
    assert_loop_fixture_diagnostic(
        "items = {1, 2, 3}\nloop items |a, b, c|:\n    io.line([: [a]])\n;",
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.too_many_bindings"),
    );
}

#[test]
fn rejects_duplicate_loop_binding_names() {
    assert_loop_fixture_diagnostic(
        "items = {1, 2, 3}\nloop items |item, item|:\n    io.line([: [item]])\n;",
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.duplicate_binding_name"),
    );
}

#[test]
fn rejects_loop_binding_shadowing_existing_name() {
    assert_loop_fixture_diagnostic(
        "items = {1, 2, 3}\nitem = 0\nloop items |item|:\n    io.line([: [item]])\n;",
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.binding_already_declared"),
    );
}

#[test]
fn rejects_loop_binding_shadowing_visible_source_declaration() {
    let failure = parse_single_file_ast_diagnostic(
        "source_value || -> I32:\n    return 1\n;\nloop 0 to 2 |source_value|:\n;\n",
    );
    assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.binding_already_declared"),
    );
}

#[test]
fn rejects_keyword_shadow_loop_binding_names() {
    let failure = parse_single_file_ast_diagnostic(&loop_fixture_source(
        "items = {1, 2, 3}\nloop items |_if|:\n    io.line([: [items]])\n;",
    ));
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0039", None);
    let collision = diagnostic
        .family_view::<ReservedNameCollisionView>()
        .expect("the first error is a reserved-name collision");
    assert_eq!(collision.reserved_by, ReservedNameOwner::Keyword);
}

// --------------------------
//  Loop source type checks
// --------------------------

#[test]
fn rejects_collection_loop_on_non_collection_expression() {
    assert_loop_fixture_diagnostic(
        "value = 3\nloop value |item|:\n    io.line([: [item]])\n;",
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.collection_source_not_collection"),
    );
}

#[test]
fn rejects_non_boolean_conditional_loop_condition() {
    let failure = parse_single_file_ast_diagnostic(&loop_fixture_source(
        "loop 1 + 2:\n    io.line([: [1]])\n;",
    ));
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-TYPE-0001", None);
    let mismatch = diagnostic
        .family_view::<TypeMismatchView>()
        .expect("the first error is a type mismatch");
    assert_eq!(mismatch.context, TypeMismatchContext::Condition);
}

// --------------------------
//  Range loop edge cases
// --------------------------

#[test]
fn parses_inclusive_range_loop_with_tight_ampersand() {
    let (ast, path_fork, string_table) =
        parse_loop_fixture("sum ~= 0\nloop 0 to &5 |i|:\n    ~sum = sum + i\n;");

    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::RangeLoop { range, .. } = &body[1].kind else {
        panic!("expected range loop in function body");
    };

    assert_eq!(range.end_kind, RangeEndKind::Inclusive);
}

#[test]
fn parses_omitted_start_exclusive_range_loop() {
    let (ast, path_fork, string_table) =
        parse_loop_fixture("sum ~= 0\nloop to 5 |i|:\n    ~sum = sum + i\n;");

    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::RangeLoop { range, .. } = &body[1].kind else {
        panic!("expected range loop in function body");
    };

    assert!(matches!(
        fixed_scalar_constant(&ast, range.start),
        Some(value)
            if value.scalar() == FixedScalar::I32 && value.as_i64() == Some(0)
    ));
    assert_eq!(range.end_kind, RangeEndKind::Exclusive);
    assert!(matches!(
        fixed_scalar_constant(&ast, range.end),
        Some(value)
            if value.scalar() == FixedScalar::I32 && value.as_i64() == Some(5)
    ));
}

#[test]
fn parses_omitted_start_inclusive_range_loop() {
    let (ast, path_fork, string_table) =
        parse_loop_fixture("sum ~= 0\nloop to & 5 |i|:\n    ~sum = sum + i\n;");

    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::RangeLoop { range, .. } = &body[1].kind else {
        panic!("expected range loop in function body");
    };

    assert!(matches!(
        fixed_scalar_constant(&ast, range.start),
        Some(value)
            if value.scalar() == FixedScalar::I32 && value.as_i64() == Some(0)
    ));
    assert_eq!(range.end_kind, RangeEndKind::Inclusive);
    assert!(matches!(
        fixed_scalar_constant(&ast, range.end),
        Some(value)
            if value.scalar() == FixedScalar::I32 && value.as_i64() == Some(5)
    ));
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
    assert_loop_fixture_diagnostic(
        "loop 0 to 10 by |i|:\n    io.line([: [i]])\n;",
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.missing_range_step"),
    );
}

#[test]
fn rejects_zero_step_literal() {
    assert_loop_fixture_diagnostic(
        "loop 0 to 10 by 0 |i|:\n    io.line([: [i]])\n;",
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.zero_range_step"),
    );
}

#[test]
fn rejects_float_range_without_by() {
    assert_loop_fixture_diagnostic(
        "loop 0.0 to 1.0 |t|:\n    io.line([: [t]])\n;",
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.float_range_missing_step"),
    );
}

// --------------------------
//  Bare binding edge cases
// --------------------------

#[test]
fn rejects_missing_comma_between_bare_dual_bindings() {
    assert_loop_fixture_diagnostic(
        "items = {1, 2, 3}\nloop items item index:\n    io.line([: [item]])\n;",
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.bare_dual_binding"),
    );
}

#[test]
fn rejects_missing_closing_pipe_in_loop_bindings() {
    assert_loop_fixture_diagnostic(
        "items = {1, 2, 3}\nloop items |item:\n    io.line([: [item]])\n;",
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.missing_closing_pipe"),
    );
}

#[test]
fn rejects_range_loop_with_bare_binding_after_complex_step_expression() {
    assert_loop_fixture_diagnostic(
        "limit = 8\nstep = 2\nsum ~= 0\nloop 0 to limit by step i:\n    ~sum = sum + i\n;",
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.bare_single_binding"),
    );
}

#[test]
fn operator_tail_does_not_trigger_bare_loop_binding_diagnostic() {
    let failure = parse_single_file_ast_diagnostic(&loop_fixture_source(
        "a = 1\nb = 2\nloop a + b value:\n    io.line([: [1]])\n;",
    ));
    let diagnostic = first_ast_error_view(&failure);
    assert!(
        !(diagnostic.descriptor().code == "MOTH-SYNTAX-0029"
            && matches!(
                diagnostic.reason_key(),
                Some("invalid_loop_header.bare_single_binding")
                    | Some("invalid_loop_header.bare_dual_binding")
            ))
    );
}

fn assert_missing_range_end_bound(source: &str) {
    assert_loop_fixture_diagnostic(
        source,
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.missing_range_end_bound"),
    );
}
