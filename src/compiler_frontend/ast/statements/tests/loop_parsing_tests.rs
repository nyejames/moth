//! Loop parsing regression tests.
//!
//! WHAT: validates conditional/range/collection loop AST shapes and loop-header diagnostics.
//! WHY: loop lowering depends on parser output staying stable across the new loop header syntax.

use super::*;
use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::{LoopBindings, RangeEndKind};
use crate::compiler_frontend::ast::const_values::ConstValuePayload;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExprId, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    calls_data::{InvalidLoopOperandView, InvalidValueReceiverView},
    choices_control_flow::{InvalidLoopHeaderCode, InvalidLoopHeaderView},
    names_declarations::ReservedNameCollisionView,
    types_borrows_targets::TypeMismatchView,
};
use crate::compiler_frontend::compiler_messages::{
    InvalidLoopOperandReason, LoopOperandKind, ReservedNameOwner, TypeMismatchContext,
    ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::datatypes::{TypeId, builtin_type_ids};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::tests::ast_fixture_support::function_body_by_name;
use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, ast_error_label_spans, ast_error_primary_span,
    ast_first_error_type_display, first_ast_error_view, parse_single_file_ast,
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
fn range_counter_uses_bounds_promotion_and_keeps_the_step_independent() {
    let (ast, path_fork, string_table) = parse_loop_fixture(
        "first_bound I8 = 1\nend U16 = 5\nstep I16 = 1\n\
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
    // The spec carries the bounds-selected counter domain. The signed step is independent and
    // never participates in that selection, so it keeps its own `I16` type.
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
        expression_type_id(&ast, *step) == builtin_type_ids::fixed_scalar(FixedScalar::I16)
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
fn omitted_range_start_zero_uses_the_endpoint_domain_with_an_independent_step() {
    let (ast, path_fork, string_table) = parse_loop_fixture(
        "end U64 = 18446744073709551615\nstep I64 = 1\n\
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
    assert!(range.step.as_ref().is_some_and(|step| {
        expression_type_id(&ast, *step) == builtin_type_ids::fixed_scalar(FixedScalar::I64)
    }));
    assert_eq!(
        bindings.index.as_ref().map(|binding| binding.type_id),
        Some(builtin_type_ids::fixed_scalar(FixedScalar::I32))
    );
}

#[test]
fn omitted_range_zero_uses_the_endpoint_domain_not_the_step_domain() {
    let (ast, path_fork, string_table) = parse_loop_fixture(
        "end U16 = 5\nstep I8 = 1\n\
         loop to end by step |value|:\n    io.line([: [value]])\n;",
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
            if value.scalar() == FixedScalar::U16 && value.as_u64() == Some(0)
    ));
    // The omitted start takes the endpoint's `U16` domain. The signed `I8` step never widens the
    // counter into a signed type.
    assert_eq!(
        bindings.item.as_ref().map(|binding| binding.type_id),
        Some(builtin_type_ids::fixed_scalar(FixedScalar::U16))
    );
    assert_eq!(range.domain, NumericScalar::Fixed(FixedScalar::U16));
    assert!(range.step.as_ref().is_some_and(|step| {
        expression_type_id(&ast, *step) == builtin_type_ids::fixed_scalar(FixedScalar::I8)
    }));
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
        Some("invalid_loop_operand.bound_not_numeric"),
    );
    let operand = diagnostic
        .family_view::<InvalidLoopOperandView>()
        .expect("the first error is an invalid range operand");
    assert_eq!(operand.operand, LoopOperandKind::End);
    assert_eq!(operand.reason, InvalidLoopOperandReason::BoundNotNumeric);
}

/// Parse one range header that must fail on domain selection, returning the blamed role and
/// authored source text under the primary span.
fn range_domain_failure(loop_body_source: &str) -> (LoopOperandKind, String) {
    let source = loop_fixture_source(loop_body_source);
    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0059",
        Some("invalid_loop_operand.bound_incompatible_domain"),
    );
    let operand_facts = diagnostic
        .family_view::<InvalidLoopOperandView>()
        .expect("the first error is an invalid range operand");
    assert_eq!(
        operand_facts.reason,
        InvalidLoopOperandReason::BoundIncompatibleDomain
    );
    let range = ast_error_primary_span(diagnostic)
        .expect("range operand diagnostics carry the operand span")
        .resolve_with(ExtendedSpanBuilder::new().resolver_for(SourceId::COMPILATION_ROOT));
    (
        operand_facts.operand,
        source[range.start() as usize..range.end() as usize].to_owned(),
    )
}

/// Parse one range step that must fail against otherwise valid bounds, returning the authored
/// source text under the primary span.
fn range_step_failure(loop_body_source: &str, reason: InvalidLoopOperandReason) -> String {
    let reason_key = match reason {
        InvalidLoopOperandReason::StepNotSignedInteger => {
            "invalid_loop_operand.step_not_signed_integer"
        }
        InvalidLoopOperandReason::StepNotConvertibleToFloatCounter => {
            "invalid_loop_operand.step_not_convertible_to_float_counter"
        }
        other => panic!("not a step reason: {other:?}"),
    };
    let source = loop_fixture_source(loop_body_source);
    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0059", Some(reason_key));
    let operand_facts = diagnostic
        .family_view::<InvalidLoopOperandView>()
        .expect("the first error is an invalid range operand");
    assert_eq!(operand_facts.operand, LoopOperandKind::Step);
    assert_eq!(operand_facts.reason, reason);
    let range = ast_error_primary_span(diagnostic)
        .expect("range operand diagnostics carry the operand span")
        .resolve_with(ExtendedSpanBuilder::new().resolver_for(SourceId::COMPILATION_ROOT));
    source[range.start() as usize..range.end() as usize].to_owned()
}

#[test]
fn signed_steps_keep_their_own_type_independent_of_unsigned_bounds() {
    // A signed step never forces an unsigned counter into a wider signed type. An untyped
    // negative literal step takes the independent `I32` default rather than the counter type.
    for (bounds_type, step_source) in [("U8", "by -1"), ("U64", "by -1")] {
        let (ast, path_fork, string_table) = parse_loop_fixture(&format!(
            "lo {bounds_type} = 0\nhi {bounds_type} = 10\n\
             loop lo to hi {step_source} |value|:\n    io.line([: [value]])\n;"
        ));
        let body = loop_function_body(&ast, &path_fork, &string_table);
        let NodeKind::RangeLoop {
            bindings, range, ..
        } = &body[2].kind
        else {
            panic!("expected a {bounds_type} range loop");
        };

        assert_eq!(
            bindings.item.as_ref().map(|binding| binding.type_id),
            Some(builtin_type_ids::fixed_scalar(match bounds_type {
                "U8" => FixedScalar::U8,
                "U64" => FixedScalar::U64,
                other => panic!("unexpected fixture bounds {other}"),
            }))
        );
        assert_eq!(
            range.domain,
            NumericScalar::Fixed(match bounds_type {
                "U8" => FixedScalar::U8,
                "U64" => FixedScalar::U64,
                other => panic!("unexpected fixture bounds {other}"),
            })
        );
        assert!(range.step.as_ref().is_some_and(|step| {
            expression_type_id(&ast, *step) == builtin_type_ids::fixed_scalar(FixedScalar::I32)
        }));
    }

    // A typed signed step keeps its authored type even when it is wider than the counter.
    let (ast, path_fork, string_table) = parse_loop_fixture(
        "lo U64 = 0\nhi U64 = 10\nstep I64 = 1\n\
         loop lo to hi by step |value|:\n    io.line([: [value]])\n;",
    );
    let body = loop_function_body(&ast, &path_fork, &string_table);
    let NodeKind::RangeLoop {
        bindings, range, ..
    } = &body[3].kind
    else {
        panic!("expected a U64 range loop");
    };
    assert_eq!(
        bindings.item.as_ref().map(|binding| binding.type_id),
        Some(builtin_type_ids::fixed_scalar(FixedScalar::U64))
    );
    assert!(range.step.as_ref().is_some_and(|step| {
        expression_type_id(&ast, *step) == builtin_type_ids::fixed_scalar(FixedScalar::I64)
    }));
}

#[test]
fn rejects_non_signed_integer_steps_for_integer_bounds_at_the_step_span() {
    // Integer counters accept only signed fixed-width integer steps. Unsigned integers, binary
    // floats, `Byte`, `Dec` and non-numeric types are all rejected at the authored step position,
    // and the counter remains the bounds-selected type.
    for (declaration, authored_step) in [
        ("step U32 = 1", "step"),
        ("step F64 = 0.5", "step"),
        ("step Byte = 1", "step"),
        ("step Dec1 = 0.5", "step"),
        ("step = true", "step"),
    ] {
        let blamed = range_step_failure(
            &format!(
                "lo U8 = 0\nhi U8 = 10\n{declaration}\n\
                 loop lo to hi by {authored_step} |value|:\n    io.line([: [value]])\n;"
            ),
            InvalidLoopOperandReason::StepNotSignedInteger,
        );
        assert_eq!(blamed, authored_step, "{declaration}");
    }
}

#[test]
fn incompatible_range_bounds_are_blamed_on_the_bounds() {
    let unstepped = range_domain_failure(
        "lo U64 = 0\nhi I64 = 10\nloop lo to hi |value|:\n    io.line([: [value]])\n;",
    );
    // The counter domain comes from the bounds alone, so the unsigned step never joins it and
    // only the bounds can be blamed.
    let stepped = range_domain_failure(
        "lo U64 = 0\nhi I64 = 10\nstep U64 = 1\n\
         loop lo to hi by step |value|:\n    io.line([: [value]])\n;",
    );

    for (operand, blamed) in [unstepped, stepped] {
        assert_eq!(operand, LoopOperandKind::End);
        assert_eq!(blamed, "hi");
    }
}

#[test]
fn mixed_float_bounds_select_the_wider_precision_for_a_narrower_step() {
    // Fixed integers and binary floats require an explicit conversion to mix, so a range header
    // combines bounds within one family. The bounds alone select the counter precision, so mixed
    // float bounds iterate at the wider precision in either authored order, and the narrower
    // `F16` step then converts safely to that counter precision.
    for (start_type, end_type) in [("F16", "F32"), ("F32", "F16")] {
        let (ast, path_fork, string_table) = parse_loop_fixture(&format!(
            "lo {start_type} = 0.0\nhi {end_type} = 3.0\nstep F16 = 0.5\n\
             loop lo to hi by step |value|:\n    io.line([: [value]])\n;"
        ));
        let body = loop_function_body(&ast, &path_fork, &string_table);
        let NodeKind::RangeLoop {
            bindings, range, ..
        } = &body[3].kind
        else {
            panic!("expected an F32 range loop for {start_type}/{end_type} bounds");
        };
        let item_type = bindings.item.as_ref().map(|binding| binding.type_id);
        assert_eq!(
            item_type,
            Some(builtin_type_ids::fixed_scalar(FixedScalar::F32)),
            "{start_type}/{end_type} bounds must iterate at the wider F32 precision"
        );
        assert_eq!(range.domain, NumericScalar::Fixed(FixedScalar::F32));
        assert!(range.step.as_ref().is_some_and(|step| {
            expression_type_id(&ast, *step) == builtin_type_ids::fixed_scalar(FixedScalar::F32)
        }));
    }
}

#[test]
fn infers_float_step_literals_at_the_counter_precision() {
    // An untyped float step takes the bounds-selected counter precision rather than defaulting to
    // F64 and widening the counter. Integer literal steps on float ranges take the same context.
    for (bounds_type, step_source, expected) in [
        ("F16", "by 0.1", FixedScalar::F16),
        ("F32", "by 0.1", FixedScalar::F32),
        ("F32", "by -0.5", FixedScalar::F32),
        ("F32", "by 2", FixedScalar::F32),
    ] {
        let (ast, path_fork, string_table) = parse_loop_fixture(&format!(
            "lo {bounds_type} = 0.0\nhi {bounds_type} = 1.0\n\
             loop lo to hi {step_source} |value|:\n    io.line([: [value]])\n;"
        ));
        let body = loop_function_body(&ast, &path_fork, &string_table);
        let NodeKind::RangeLoop {
            bindings, range, ..
        } = &body[2].kind
        else {
            panic!("expected a {bounds_type} range loop for {step_source}");
        };

        assert_eq!(
            bindings.item.as_ref().map(|binding| binding.type_id),
            Some(builtin_type_ids::fixed_scalar(expected))
        );
        assert_eq!(range.domain, NumericScalar::Fixed(expected));
        assert!(range.step.as_ref().is_some_and(|step| {
            expression_type_id(&ast, *step) == builtin_type_ids::fixed_scalar(expected)
        }));
    }
}

#[test]
fn deferred_integer_range_step_literal_keeps_its_own_i32_default() {
    // `hi` stays unannotated, so the header retains its end bound and its literal step until the
    // body root completes it. The completed bounds select an integer counter, which supplies no
    // literal destination, so the step keeps the independent `I32` default.
    let (ast, path_fork, string_table) =
        parse_loop_fixture("hi = 10\nloop 0 to hi by -1 |value|:\n    io.line([: [value]])\n;");
    let body = loop_function_body(&ast, &path_fork, &string_table);
    let NodeKind::RangeLoop {
        bindings, range, ..
    } = &body[1].kind
    else {
        panic!("expected a completed range loop after the pending declaration");
    };

    assert_eq!(range.domain, NumericScalar::Fixed(FixedScalar::I32));
    assert_eq!(
        bindings.item.as_ref().map(|binding| binding.type_id),
        Some(builtin_type_ids::fixed_scalar(FixedScalar::I32))
    );
    assert!(range.step.as_ref().is_some_and(|step| {
        expression_type_id(&ast, *step) == builtin_type_ids::fixed_scalar(FixedScalar::I32)
    }));
}

#[test]
fn deferred_float_range_step_literal_takes_the_pinned_counter_precision() {
    // Untyped bounds stay pending until body completion, where the later `F32` witnesses pin them.
    // The completed F32 bounds then supply the retained literal step's destination, so it
    // materialises at F32 instead of defaulting to F64.
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "pin_f32 |value F32| -> F32:\n    return value\n;\n\n\
         loop_test ||:\n    lo = 0.0\n    hi = 1.0\n\
         loop lo to hi by 0.1 |value|:\n    io.line([: [value]])\n;\n\
         lo_pin F32 = pin_f32(lo)\n    hi_pin F32 = pin_f32(hi)\n;\n\nloop_test()\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "loop_test");
    let NodeKind::RangeLoop {
        bindings, range, ..
    } = &body[2].kind
    else {
        panic!("expected a completed range loop after the pending declarations");
    };

    assert_eq!(range.domain, NumericScalar::Fixed(FixedScalar::F32));
    assert_eq!(
        bindings.item.as_ref().map(|binding| binding.type_id),
        Some(builtin_type_ids::fixed_scalar(FixedScalar::F32))
    );
    assert!(range.step.as_ref().is_some_and(|step| {
        expression_type_id(&ast, *step) == builtin_type_ids::fixed_scalar(FixedScalar::F32)
    }));
}

#[test]
fn postponed_float_step_binding_takes_the_counter_precision() {
    // `s` is a pending numeric binding used only as the step. The typed F32 bounds select the
    // counter before the step completes, and the step binding adopts that precision rather than
    // defaulting to F64 and making the step wider than the counter.
    let (ast, path_fork, string_table) = parse_loop_fixture(
        "a F32 = 0.0\nb F32 = 3.0\ns = 0.5\n\
         loop a to b by s |value|:\n    io.line([: [value]])\n;",
    );
    let body = loop_function_body(&ast, &path_fork, &string_table);
    let NodeKind::VariableDeclaration(step_binding) = &body[2].kind else {
        panic!("expected the pending step declaration");
    };
    let NodeKind::RangeLoop { range, .. } = &body[3].kind else {
        panic!("expected a completed range loop after the step declaration");
    };

    assert_eq!(
        step_binding.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::F32)
    );
    assert_eq!(range.domain, NumericScalar::Fixed(FixedScalar::F32));
    let step = range.step.expect("the explicit step stays present");
    assert_eq!(
        expression_type_id(&ast, step),
        builtin_type_ids::fixed_scalar(FixedScalar::F32)
    );
}

#[test]
fn template_range_loop_infers_float_step_at_counter_precision() {
    // A template range header whose bounds reference pending numeric origins routes through TIR
    // intake completion. The later `F32` witnesses pin both bounds, and the retained literal-only
    // step, single literal or literal expression, must then materialise at F32 precision.
    for authored_step in ["0.1", "0.1 + 0.1"] {
        let (ast, path_fork, string_table) = parse_single_file_ast(&format!(
            "pin_f32 |value F32| -> F32:\n    return value\n;\n\n\
             loop_test ||:\n    lo = 0.0\n    hi = 1.0\n\
             text String = [loop lo to hi by {authored_step}:\n    [1]\n]\n\
             lo_pin F32 = pin_f32(lo)\n    hi_pin F32 = pin_f32(hi)\n    io.line([: [text]])\n;\n\nloop_test()\n",
        ));
        let body = function_body_by_name(&ast, &path_fork, &string_table, "loop_test");
        let initializer = body
            .iter()
            .find_map(|node| match &node.kind {
                NodeKind::VariableDeclaration(declaration)
                    if path_fork
                        .component(declaration.id)
                        .map(|component| string_table.resolve(component))
                        == Some("text") =>
                {
                    declaration.value.initializer()
                }
                _ => None,
            })
            .expect("the template declaration initializer");
        let ExpressionOperationView::RuntimeTemplate(handoff) =
            expression_view(&ast, initializer).operation()
        else {
            panic!("expected a runtime template handoff for step `{authored_step}`");
        };
        let OwnedRuntimeTemplateBody::Render(root) = &handoff.body else {
            panic!("expected a render body for step `{authored_step}`");
        };
        let range = find_runtime_range_loop(root).expect("template range loop");
        assert_eq!(range.domain, NumericScalar::Fixed(FixedScalar::F32));
        let step = range.step.expect("the explicit step stays present");
        assert_eq!(
            expression_type_id(&ast, step),
            builtin_type_ids::fixed_scalar(FixedScalar::F32),
            "step `{authored_step}`"
        );
    }
}

fn find_runtime_range_loop(
    node: &OwnedRuntimeTemplateNode,
) -> Option<&crate::compiler_frontend::ast::ast_nodes::RangeLoopSpec> {
    match node {
        OwnedRuntimeTemplateNode::Loop {
            header: TemplateLoopHeader::Range { range, .. },
            ..
        } => Some(range),
        OwnedRuntimeTemplateNode::Sequence { children, .. } => {
            children.iter().find_map(find_runtime_range_loop)
        }
        OwnedRuntimeTemplateNode::Conditional { body, .. } => find_runtime_range_loop(body),
        OwnedRuntimeTemplateNode::ConditionalWrapper { child, wrapper, .. } => {
            find_runtime_range_loop(child).or_else(|| find_runtime_range_loop(wrapper))
        }
        OwnedRuntimeTemplateNode::ChildTemplate { template, .. } => match &template.body {
            OwnedRuntimeTemplateBody::Render(body) => find_runtime_range_loop(body),
            OwnedRuntimeTemplateBody::RuntimeSlotApplication(_) => None,
        },
        _ => None,
    }
}

#[test]
fn rejects_steps_that_cannot_convert_to_the_float_counter() {
    // A typed integer step never becomes a float step, and a wider float would narrow the
    // authored step precision. Both are rejected at the step position.
    for (declaration, authored_step) in [
        ("step I32 = 1", "step"),
        ("step F64 = 0.5", "step"),
        ("step U8 = 1", "step"),
        ("step = true", "step"),
    ] {
        let blamed = range_step_failure(
            &format!(
                "lo F32 = 0.0\nhi F32 = 3.0\n{declaration}\n\
                 loop lo to hi by {authored_step} |value|:\n    io.line([: [value]])\n;"
            ),
            InvalidLoopOperandReason::StepNotConvertibleToFloatCounter,
        );
        assert_eq!(blamed, authored_step, "{declaration}");
    }
}

#[test]
fn rejects_narrower_float_counter_for_wider_step() {
    let blamed = range_step_failure(
        "lo F16 = 0.0\nhi F16 = 3.0\nstep F32 = 0.5\n\
         loop lo to hi by step |value|:\n    io.line([: [value]])\n;",
        InvalidLoopOperandReason::StepNotConvertibleToFloatCounter,
    );
    assert_eq!(blamed, "step");
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
        step: _,
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
//  Collection loop steps
// --------------------------

#[test]
fn parses_collection_loop_steps_in_every_binding_form() {
    for (header, expected_item, expected_index) in [
        ("loop items by 2:", None, None),
        ("loop items by 2 |item|:", Some("item"), None),
        (
            "loop items by 2 |item, index|:",
            Some("item"),
            Some("index"),
        ),
    ] {
        let (ast, path_fork, string_table) = parse_loop_fixture(&format!(
            "items = {{1, 2, 3}}
{header}
    io.line([: [1]])
;"
        ));
        let body = loop_function_body(&ast, &path_fork, &string_table);

        let NodeKind::CollectionLoop {
            bindings,
            iterable,
            step,
            ..
        } = &body[1].kind
        else {
            panic!("expected collection loop for header {header}");
        };

        assert_eq!(
            bindings.item.as_ref().and_then(|binding| path_fork
                .component(binding.id)
                .map(|id| string_table.resolve(id))),
            expected_item,
            "{header}"
        );
        assert_eq!(
            bindings.index.as_ref().and_then(|binding| path_fork
                .component(binding.id)
                .map(|id| string_table.resolve(id))),
            expected_index,
            "{header}"
        );
        let iterable_type = expression_type_id(&ast, *iterable);
        assert!(
            ast.type_environment.is_collection(iterable_type),
            "{header}"
        );
        if let Some(item) = &bindings.item {
            assert_eq!(
                Some(item.type_id),
                ast.type_environment.collection_element_type(iterable_type),
                "the step never changes the item type ({header})"
            );
        }
        let step = step.expect("the explicit step stays present");
        assert_eq!(
            expression_type_id(&ast, step),
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            "an untyped literal step takes the independent I32 default ({header})"
        );
        assert!(matches!(
            fixed_scalar_constant(&ast, step),
            Some(value)
                if value.scalar() == FixedScalar::I32 && value.as_i64() == Some(2)
        ));
    }
}

#[test]
fn collection_steps_keep_their_own_signed_type_independent_of_the_item_type() {
    for (step_type, scalar, authored) in [
        ("I8", FixedScalar::I8, "2"),
        ("I64", FixedScalar::I64, "-2"),
    ] {
        let (ast, path_fork, string_table) = parse_loop_fixture(&format!(
            "items = {{1, 2, 3}}
step {step_type} = {authored}
             loop items by step |item, index|:
    io.line([: [item]])
;"
        ));
        let body = loop_function_body(&ast, &path_fork, &string_table);

        let NodeKind::CollectionLoop {
            bindings,
            iterable,
            step,
            ..
        } = &body[2].kind
        else {
            panic!("expected collection loop for a {step_type} step");
        };

        let iterable_type = expression_type_id(&ast, *iterable);
        assert_eq!(
            bindings.item.as_ref().map(|binding| binding.type_id),
            ast.type_environment.collection_element_type(iterable_type),
            "the {step_type} step never changes the item type"
        );
        assert_eq!(
            bindings.index.as_ref().map(|binding| binding.type_id),
            Some(builtin_type_ids::fixed_scalar(FixedScalar::I32))
        );
        let step = step.expect("the explicit step stays present");
        assert_eq!(
            expression_type_id(&ast, step),
            builtin_type_ids::fixed_scalar(scalar)
        );
    }
}

/// Parse one collection step that must fail on its independent type, returning the authored
/// source text under the primary span.
fn collection_step_failure(loop_body_source: &str) -> String {
    let source = loop_fixture_source(loop_body_source);
    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0059",
        Some("invalid_loop_operand.step_not_signed_integer"),
    );
    let operand_facts = diagnostic
        .family_view::<InvalidLoopOperandView>()
        .expect("the first error is an invalid range operand");
    assert_eq!(operand_facts.operand, LoopOperandKind::Step);
    assert_eq!(
        operand_facts.reason,
        InvalidLoopOperandReason::StepNotSignedInteger
    );
    let range = ast_error_primary_span(diagnostic)
        .expect("collection step diagnostics carry the step span")
        .resolve_with(ExtendedSpanBuilder::new().resolver_for(SourceId::COMPILATION_ROOT));
    source[range.start() as usize..range.end() as usize].to_owned()
}

#[test]
fn rejects_non_signed_collection_steps_at_the_step_span() {
    // Like a range step, a collection step is independent of the item type and accepts only a
    // signed fixed-width integer. Unsigned, float, Byte, Dec and Bool steps are rejected at the
    // authored step position.
    for (declaration, authored_step) in [
        ("step U32 = 1", "step"),
        ("step F64 = 0.5", "step"),
        ("step Byte = 1", "step"),
        ("step Dec1 = 0.5", "step"),
        ("step = true", "step"),
    ] {
        let blamed = collection_step_failure(&format!(
            "items = {{1, 2, 3}}
{declaration}
             loop items by {authored_step} |item|:
    io.line([: [item]])
;"
        ));
        assert_eq!(blamed, authored_step, "{declaration}");
    }
}

#[test]
fn rejects_static_zero_loop_steps_for_both_loop_kinds() {
    // Range and collection steps share one zero-step reason and one wording.
    for (header, body) in [
        ("loop 0 to 10 by 0 |i|:", "io.line([: [i]])"),
        ("loop 0.0 to 1.0 by 0.0 |value|:", "io.line([: [value]])"),
        ("loop 0.0 to 1.0 by -0.0 |value|:", "io.line([: [value]])"),
    ] {
        assert_loop_fixture_diagnostic(
            &format!("{header}\n    {body}\n;"),
            "MOTH-SYNTAX-0029",
            Some("invalid_loop_header.zero_loop_step"),
        );
    }
    for step in ["0", "-0"] {
        assert_loop_fixture_diagnostic(
            &format!(
                "items = {{1, 2, 3}}
loop items by {step} |item|:
    io.line([: [item]])
;"
            ),
            "MOTH-SYNTAX-0029",
            Some("invalid_loop_header.zero_loop_step"),
        );
    }
}

#[test]
fn rejects_repeated_collection_step_markers_at_the_second_by() {
    let source = loop_fixture_source(
        "items = {1, 2, 3}
loop items by 1 by 2 |item|:
    io.line([: [item]])
;",
    );
    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.repeated_loop_step"),
    );
    let range = ast_error_primary_span(diagnostic)
        .expect("the repeated marker diagnostic carries the token span")
        .resolve_with(ExtendedSpanBuilder::new().resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(&source[range.start() as usize..range.end() as usize], "by");
    assert_eq!(
        range.start() as usize,
        source.rfind("by").expect("the fixture has two markers")
    );
}

#[test]
fn rejects_repeated_range_step_markers_at_the_second_by() {
    let source = loop_fixture_source(
        "loop 0 to 10 by 2 by 3 |i|:
    io.line([: [i]])
;",
    );
    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.repeated_loop_step"),
    );
    let range = ast_error_primary_span(diagnostic)
        .expect("the repeated marker diagnostic carries the token span")
        .resolve_with(ExtendedSpanBuilder::new().resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(&source[range.start() as usize..range.end() as usize], "by");
    assert_eq!(
        range.start() as usize,
        source.rfind("by").expect("the fixture has two markers")
    );
}

#[test]
fn rejects_range_step_before_the_end_bound_at_the_step() {
    let source = loop_fixture_source(
        "loop 0 by 2 to 5 |i|:
    io.line([: [i]])
;",
    );
    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.misplaced_loop_step"),
    );
    let range = ast_error_primary_span(diagnostic)
        .expect("the misplaced-step diagnostic carries the marker span")
        .resolve_with(ExtendedSpanBuilder::new().resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(&source[range.start() as usize..range.end() as usize], "by");
}

#[test]
fn rejects_collection_step_after_bindings_at_the_step() {
    let source = loop_fixture_source(
        "items = {1, 2, 3}
loop items |item| by 2:
    io.line([: [item]])
;",
    );
    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.misplaced_loop_step"),
    );
    let range = ast_error_primary_span(diagnostic)
        .expect("the misplaced-step diagnostic carries the marker span")
        .resolve_with(ExtendedSpanBuilder::new().resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(&source[range.start() as usize..range.end() as usize], "by");
}

#[test]
fn rejects_range_step_after_bindings_at_the_step() {
    let source = loop_fixture_source(
        "loop 0 to 5 |i| by 2:
    io.line([: [i]])
;",
    );
    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.misplaced_loop_step"),
    );
    let range = ast_error_primary_span(diagnostic)
        .expect("the misplaced-step diagnostic carries the marker span")
        .resolve_with(ExtendedSpanBuilder::new().resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(&source[range.start() as usize..range.end() as usize], "by");
}

#[test]
fn rejects_bindings_on_a_conditional_loop() {
    let source = loop_fixture_source(
        "flag = true
loop flag |item|:
    io.line([: [item]])
;",
    );
    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.bindings_on_conditional_loop"),
    );
    let span = ast_error_primary_span(diagnostic).expect("the bindings carry the primary span");
    let inline = span
        .local()
        .inline_range()
        .expect("a short binding suffix stays inline");
    let start = source
        .find("|item|")
        .expect("the fixture has a binding suffix") as u32;
    assert_eq!(inline.start(), start);
    assert_eq!(inline.end(), start + "|item|".len() as u32);
}

#[test]
fn rejects_a_pending_non_boolean_condition_with_the_condition_context() {
    let failure = parse_single_file_ast_diagnostic(&loop_fixture_source(
        "value = 1
loop value:
    io.line([: [1]])
;",
    ));
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-TYPE-0001", None);
    let mismatch = diagnostic
        .family_view::<TypeMismatchView>()
        .expect("the first error is a type mismatch");
    assert_eq!(mismatch.context, TypeMismatchContext::Condition);
}

#[test]
fn rejects_missing_collection_step_value() {
    assert_loop_fixture_diagnostic(
        "items = {1, 2, 3}
loop items by:
    io.line([: [1]])
;",
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.missing_collection_step"),
    );
}

#[test]
fn rejects_by_on_conditional_loop() {
    let source = loop_fixture_source(
        "loop true by 2:
    io.line([: [1]])
;",
    );
    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.step_on_conditional_loop"),
    );
    let range = ast_error_primary_span(diagnostic)
        .expect("the conditional-step diagnostic carries the marker span")
        .resolve_with(ExtendedSpanBuilder::new().resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(&source[range.start() as usize..range.end() as usize], "by");
}

#[test]
fn deferred_boolean_loop_sources_with_a_step_keep_the_conditional_reason() {
    // Every source references a pending numeric origin, so the header defers the classification to
    // the body root. The completed type is still `Bool`, so the step diagnostic stays at the `by`
    // marker rather than claiming the source is a non-collection.
    for (prologue, header) in [
        (
            "",
            "count ~= 0
loop count < 2 by 2:
    ~count += 1
;",
        ),
        (
            "ready type A |value A| -> Bool:
    return true
;
",
            "count ~= 0
loop ready(count) by 2:
    ~count += 1
;",
        ),
        (
            "ready type A |value A| -> Bool:
    return true
;
",
            "count ~= 0
loop not ready(count) by 2:
    ~count += 1
;",
        ),
    ] {
        let source = format!("{prologue}{}", loop_fixture_source(header));
        let failure = parse_single_file_ast_diagnostic(&source);
        let diagnostic = assert_ast_diagnostic(
            &failure,
            "MOTH-SYNTAX-0029",
            Some("invalid_loop_header.step_on_conditional_loop"),
        );
        let range = ast_error_primary_span(diagnostic)
            .expect("the conditional-step diagnostic carries the marker span")
            .resolve_with(ExtendedSpanBuilder::new().resolver_for(SourceId::COMPILATION_ROOT));
        assert_eq!(
            &source[range.start() as usize..range.end() as usize],
            "by",
            "{header}"
        );
    }
}

#[test]
fn deferred_collection_step_binding_keeps_the_i32_default() {
    // `step` is unannotated, so the header retains its step until the body root resolves the
    // pending origin. The resolved step keeps its own I32 default and never adopts the item type.
    let (ast, path_fork, string_table) = parse_loop_fixture(
        "items = {1, 2, 3}
step = 2
         loop items by step |item, index|:
    io.line([: [item]])
;",
    );
    let body = loop_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(step_binding) = &body[1].kind else {
        panic!("expected the pending step declaration");
    };
    assert_eq!(
        step_binding.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );

    let NodeKind::CollectionLoop { step, iterable, .. } = &body[2].kind else {
        panic!("expected the completed collection loop after the step declaration");
    };
    assert!(
        ast.type_environment
            .is_collection(expression_type_id(&ast, *iterable))
    );
    let step = step.expect("the explicit step stays present");
    assert_eq!(
        expression_type_id(&ast, step),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
}

#[test]
fn deferred_non_collection_source_with_a_step_names_the_collection_source() {
    // `value` is pending until the body root selects its numeric type, so the header cannot prove
    // the source is not a collection at parse time. The authored `by` still marks a collection
    // loop, so the diagnostic must name the completed source rather than call this a conditional
    // loop.
    let source = loop_fixture_source(
        "value = 1
loop value by 2:
    io.line([: [1]])
;",
    );
    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.collection_source_not_collection"),
    );
    let span = ast_error_primary_span(diagnostic)
        .expect("the completed source carries the primary span")
        .resolve_with(ExtendedSpanBuilder::new().resolver_for(SourceId::COMPILATION_ROOT));
    let header_facts = diagnostic
        .family_view::<InvalidLoopHeaderView>()
        .expect("the loop source rejection keeps its typed reason");
    assert_eq!(
        header_facts.reason,
        InvalidLoopHeaderCode::CollectionSourceNotCollection
    );
    let found_type = header_facts
        .found_type
        .expect("the completed source type is recorded");
    assert_eq!(
        ast_first_error_type_display(&failure, found_type),
        "I32",
        "the diagnostic names the type the body root selected for the source"
    );
    let header_start = source
        .find("loop value by 2:")
        .expect("the fixture has the authored loop header") as u32;
    let source_start = header_start + "loop ".len() as u32;
    assert_eq!(&source[span.start() as usize..span.end() as usize], "value");
    assert_eq!(span.start(), source_start);
}

#[test]
fn deferred_collection_source_with_a_step_completes_without_bindings() {
    // The source is a pending collection binding and the step is pending too, so the bindingless
    // header completes both at the body root instead of rejecting the deferred source.
    let (ast, path_fork, string_table) = parse_loop_fixture(
        "items = {1, 2, 3}
step = 2
loop items by step:
    io.line([: [1]])
;",
    );
    let body = loop_function_body(&ast, &path_fork, &string_table);

    let loop_node = body
        .iter()
        .find(|node| matches!(node.kind, NodeKind::CollectionLoop { .. }))
        .expect("expected the completed collection loop after the pending declarations");
    let NodeKind::CollectionLoop {
        bindings,
        iterable,
        step,
        ..
    } = &loop_node.kind
    else {
        unreachable!("the node was selected as a collection loop");
    };
    assert!(bindings.item.is_none() && bindings.index.is_none());
    assert!(
        ast.type_environment
            .is_collection(expression_type_id(&ast, *iterable))
    );
    let step = step.expect("the explicit step stays present");
    assert_eq!(
        expression_type_id(&ast, step),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
}

/// Find the first runtime collection loop header in a runtime template handoff.
fn find_runtime_collection_loop(
    node: &OwnedRuntimeTemplateNode,
) -> Option<(&LoopBindings, ExprId, Option<ExprId>)> {
    match node {
        OwnedRuntimeTemplateNode::Loop {
            header:
                TemplateLoopHeader::Collection {
                    bindings,
                    iterable,
                    step,
                },
            ..
        } => Some((bindings, *iterable, *step)),
        OwnedRuntimeTemplateNode::Sequence { children, .. } => {
            children.iter().find_map(find_runtime_collection_loop)
        }
        OwnedRuntimeTemplateNode::Conditional { body, .. } => find_runtime_collection_loop(body),
        OwnedRuntimeTemplateNode::ConditionalWrapper { child, wrapper, .. } => {
            find_runtime_collection_loop(child).or_else(|| find_runtime_collection_loop(wrapper))
        }
        OwnedRuntimeTemplateNode::ChildTemplate { template, .. } => match &template.body {
            OwnedRuntimeTemplateBody::Render(body) => find_runtime_collection_loop(body),
            OwnedRuntimeTemplateBody::RuntimeSlotApplication(_) => None,
        },
        _ => None,
    }
}

/// Return one function body's `text` template declaration initializer.
fn runtime_template_initializer(
    ast: &Ast,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> ExprId {
    let body = loop_function_body(ast, path_fork, string_table);
    body.iter()
        .find_map(|node| match &node.kind {
            NodeKind::VariableDeclaration(declaration)
                if path_fork
                    .component(declaration.id)
                    .map(|component| string_table.resolve(component))
                    == Some("text") =>
            {
                declaration.value.initializer()
            }
            _ => None,
        })
        .expect("the template declaration initializer")
}

#[test]
fn template_collection_headers_carry_steps_into_runtime_templates() {
    for (header, body_source, expected_step, has_item, has_index) in [
        ("loop items by 2", "[1]", 2, false, false),
        ("loop items by -1 |item|", "[item]", -1, true, false),
        ("loop items by 2 |item, index|", "[index]", 2, true, true),
    ] {
        let (ast, path_fork, string_table) = parse_loop_fixture(&format!(
            "items = {{1, 2, 3}}
text String = [{header}:
    {body_source}
]
io.line([: [text]])"
        ));
        let initializer = runtime_template_initializer(&ast, &path_fork, &string_table);
        let ExpressionOperationView::RuntimeTemplate(handoff) =
            expression_view(&ast, initializer).operation()
        else {
            panic!("expected a runtime template handoff for `{header}`");
        };
        let OwnedRuntimeTemplateBody::Render(root) = &handoff.body else {
            panic!("expected a render body for `{header}`");
        };
        let (bindings, iterable, step) =
            find_runtime_collection_loop(root).expect("template collection loop");
        assert!(
            ast.type_environment
                .is_collection(expression_type_id(&ast, iterable))
        );
        assert_eq!(bindings.item.is_some(), has_item, "{header}");
        assert_eq!(bindings.index.is_some(), has_index, "{header}");
        let step = step.expect("the template step stays present");
        assert_eq!(
            expression_type_id(&ast, step),
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            "{header}"
        );
        assert!(matches!(
            fixed_scalar_constant(&ast, step),
            Some(value)
                if value.scalar() == FixedScalar::I32 && value.as_i64() == Some(expected_step)
        ));
    }
}

#[test]
fn template_collection_step_pending_binding_completes_into_tir() {
    // The step reaches a pending numeric origin, so the whole header waits for the body root,
    // completes both slots through TIR intake and installs one concrete loop header.
    let (ast, path_fork, string_table) = parse_loop_fixture(
        "items = {1, 2, 3}
step = 2
         text String = [loop items by step |item|:
    [item]
]
io.line([: [text]])",
    );
    let body = loop_function_body(&ast, &path_fork, &string_table);
    let NodeKind::VariableDeclaration(step_binding) = &body[1].kind else {
        panic!("expected the pending step declaration");
    };
    assert_eq!(
        step_binding.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );

    let initializer = runtime_template_initializer(&ast, &path_fork, &string_table);
    let ExpressionOperationView::RuntimeTemplate(handoff) =
        expression_view(&ast, initializer).operation()
    else {
        panic!("expected a runtime template handoff");
    };
    let OwnedRuntimeTemplateBody::Render(root) = &handoff.body else {
        panic!("expected a render body");
    };
    let (bindings, iterable, step) =
        find_runtime_collection_loop(root).expect("template collection loop");
    assert!(bindings.item.is_some());
    assert!(
        ast.type_environment
            .is_collection(expression_type_id(&ast, iterable))
    );
    let step = step.expect("the completed step stays present");
    assert_eq!(
        expression_type_id(&ast, step),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
}

#[test]
fn template_deferred_non_collection_source_with_a_step_is_rejected_at_the_source() {
    // The template completion owner finishes the same retained bindingless source, so it must
    // apply the loop's collection-source contract rather than install an Int source as a loop.
    let source = loop_fixture_source(
        "value = 1
text String = [loop value by 2:
    [1]
]
io.line([: [text]])",
    );
    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.collection_source_not_collection"),
    );
    let span = ast_error_primary_span(diagnostic)
        .expect("the completed source carries the primary span")
        .resolve_with(ExtendedSpanBuilder::new().resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(&source[span.start() as usize..span.end() as usize], "value");
}

#[test]
fn template_deferred_negated_boolean_source_with_a_step_reports_the_marker() {
    // The template completion owner classifies the retained source after it completes, so a
    // negated retained `Bool` keeps the conditional-loop step diagnostic at the marker.
    let source = format!(
        "ready type A |value A| -> Bool:
    return true
;
{}",
        loop_fixture_source(
            "value = 1
text String = [loop not ready(value) by 2:
    [1]
]
io.line([: [text]])"
        )
    );
    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.step_on_conditional_loop"),
    );
    let range = ast_error_primary_span(diagnostic)
        .expect("the conditional-step diagnostic carries the marker span")
        .resolve_with(ExtendedSpanBuilder::new().resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(&source[range.start() as usize..range.end() as usize], "by");
}

#[test]
fn retained_collection_source_diagnostic_precedes_its_step_diagnostic() {
    // The source is completed and checked before the step, so an invalid source is reported first
    // even when the authored step is static-zero or not a signed integer.
    for step in ["0", "1.5"] {
        let source = loop_fixture_source(&format!(
            "value = 1
loop value by {step}:
    io.line([: [1]])
;"
        ));
        let failure = parse_single_file_ast_diagnostic(&source);
        let diagnostic = assert_ast_diagnostic(
            &failure,
            "MOTH-SYNTAX-0029",
            Some("invalid_loop_header.collection_source_not_collection"),
        );
        let span = ast_error_primary_span(diagnostic)
            .expect("the completed source carries the primary span")
            .resolve_with(ExtendedSpanBuilder::new().resolver_for(SourceId::COMPILATION_ROOT));
        assert_eq!(
            &source[span.start() as usize..span.end() as usize],
            "value",
            "by {step}"
        );
    }
}

#[test]
fn retained_collection_source_keeps_its_zero_step_diagnostic() {
    // A pending collection source completes legally, so the static-zero step is still reported at
    // its own authored position.
    let source = loop_fixture_source(
        "items = {1, 2, 3}
loop items by 0:
    io.line([: [1]])
;",
    );
    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.zero_loop_step"),
    );
    let span = ast_error_primary_span(diagnostic)
        .expect("the zero-step diagnostic carries the step span")
        .resolve_with(ExtendedSpanBuilder::new().resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(&source[span.start() as usize..span.end() as usize], "0");
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
fn rejects_reserved_keyword_as_loop_binding() {
    let source =
        loop_fixture_source("items = {1, 2, 3}\nloop items |by|:\n    io.line([: [items]])\n;");
    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0039", None);
    let facts = diagnostic
        .family_view::<ReservedNameCollisionView>()
        .expect("the loop binding must report a reserved name collision");
    assert_eq!(facts.reserved_by, ReservedNameOwner::Keyword);

    let keyword_start = source.find("|by|").expect("the loop binding must occur") as u32 + 1;
    let mut builder = ExtendedSpanBuilder::new();
    let keyword_span = LocalSpan::exact(keyword_start, "by".len() as u32, &mut builder)
        .expect("the keyword span must fit");
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, keyword_span)),
    );
}

#[test]
fn rejects_loop_binding_shadowing_existing_name() {
    let source = loop_fixture_source(
        "items = {1, 2, 3}\nitem = 0\nloop items |item|:\n    io.line([: [item]])\n;",
    );
    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0002", None);
    let duplicate_start = source.find("|item|").expect("the loop binding must occur") as u32 + 1;
    let previous_start = source
        .find("    item = 0")
        .expect("the earlier local declaration must occur") as u32
        + 4;
    let mut builder = ExtendedSpanBuilder::new();
    let duplicate_span = LocalSpan::exact(duplicate_start, "item".len() as u32, &mut builder)
        .expect("the duplicate name span must fit");
    let previous_span = LocalSpan::exact(previous_start, "item".len() as u32, &mut builder)
        .expect("the previous name span must fit");
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, duplicate_span)),
    );
    assert_eq!(
        ast_error_label_spans(diagnostic),
        vec![Some(SourceSpan::new(
            SourceId::COMPILATION_ROOT,
            previous_span
        ))],
    );
}

#[test]
fn rejects_loop_binding_shadowing_visible_source_declaration() {
    let source = "source_value || -> I32:\n    return 1\n;\nloop 0 to 2 |source_value|:\n;\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0002", None);
    let name = "source_value";
    let mut builder = ExtendedSpanBuilder::new();
    let previous_span = LocalSpan::exact(0, name.len() as u32, &mut builder)
        .expect("the declaration name span must fit");
    let duplicate_start = source.rfind(name).expect("the loop binding must occur") as u32;
    let duplicate_span = LocalSpan::exact(duplicate_start, name.len() as u32, &mut builder)
        .expect("the duplicate name span must fit");
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, duplicate_span)),
    );
    assert_eq!(
        ast_error_label_spans(diagnostic),
        vec![Some(SourceSpan::new(
            SourceId::COMPILATION_ROOT,
            previous_span
        ))],
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
fn collection_loop_on_unselected_number_names_no_type() {
    let failure = parse_single_file_ast_diagnostic(&loop_fixture_source(
        "value = 3\nloop value |item|:\n    io.line([: [item]])\n;",
    ));
    assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.collection_source_not_collection"),
    );
    assert_eq!(
        crate::compiler_frontend::tests::parse_support::ast_first_error_rendered_message(&failure),
        "Collection loop source must be a collection. Use a collection after 'loop'. \
         For numeric iteration, use range syntax such as 'loop 0 to 10 |i|:'."
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

fn assert_missing_range_end_bound(loop_body_source: &str) {
    let source = loop_fixture_source(loop_body_source);
    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0029",
        Some("invalid_loop_header.missing_range_end_bound"),
    );
    let range = ast_error_primary_span(diagnostic)
        .expect("the missing-bound diagnostic carries the range marker span")
        .resolve_with(ExtendedSpanBuilder::new().resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(&source[range.start() as usize..range.end() as usize], "to");
}

#[test]
fn rejects_then_inside_a_loop_body_with_loop_specific_reason() {
    let failure = parse_single_file_ast_diagnostic("loop 0 to 3 |i|:\n    then i\n;\n");
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.then_inside_loop_body"),
    );
}

#[test]
fn rejects_then_crossing_an_ordinary_loop_from_a_value_block() {
    let failure = parse_single_file_ast_diagnostic(
        "value = if true:\n    loop true:\n        then 1\n    ;\nelse\n    then 0\n;\n",
    );
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.then_crosses_blocked_construct"),
    );
}
