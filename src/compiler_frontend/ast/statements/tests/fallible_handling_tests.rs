//! Fallible-handling parsing and validation regression tests.
//!
//! WHAT: exercises shared fallible-handling helpers across call/expression paths.
//! WHY: fallible handling spans dense syntax plus control-flow constraints, so focused tests prevent
//! parser and validation drift during refactors.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::{
    FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionOperationView;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidAssignmentTargetReason, InvalidFallibleHandlingReason,
    InvalidReturnShapeReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::tests::ast_fixture_support::function_body_by_name;
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};
use moth_lexical::numeric::fixed_scalar::FixedScalar;

#[test]
fn optional_propagation_call_argument_preserves_unwrapped_receiving_type() {
    let (ast, paths, strings) = parse_single_file_ast(
        "identity |text String| -> String:\n    return text\n;\nreceive |value String?| -> String?:\n    return identity(value?)\n;\n",
    );
    let body = function_body_by_name(&ast, &paths, &strings, "receive");
    let NodeKind::Return(values) = &body[0].kind else {
        panic!("expected optional return");
    };
    assert_eq!(values.len(), 1);
    let returned = ast.expressions.view(values[0]).unwrap();
    let optional_type = returned.value_type_id().unwrap();
    assert_eq!(
        ast.type_environment.option_inner_type(optional_type),
        Some(builtin_type_ids::STRING)
    );
    let call_value = match returned.operation() {
        ExpressionOperationView::Coerce { value, .. } => value,
        _ => returned,
    };
    assert_eq!(call_value.success_types(), &[builtin_type_ids::STRING]);
    let ExpressionOperationView::Call(call) = call_value.operation() else {
        panic!("expected typed source-call consumer");
    };
    assert_eq!(call.arguments().len(), 1);
    let argument = call.arguments().iter().next().unwrap();
    assert_eq!(argument.metadata().parameter_slot, Some(0));
    let propagated = argument.value();
    assert_eq!(propagated.success_types(), &[builtin_type_ids::STRING]);
    assert_eq!(propagated.error_type(), None);
    let ExpressionOperationView::PropagateOption { value } = propagated.operation() else {
        panic!("expected argument's optional exit before its consumer call");
    };
    assert_eq!(value.success_types(), &[optional_type]);
    assert!(matches!(
        value.operation(),
        ExpressionOperationView::Reference(_)
    ));
}

#[test]
fn optional_propagation_context_reasons_retain_authored_operator_span() {
    let cases = [
        (
            "receive |value String| -> String?:\n    return value?\n;\n",
            InvalidFallibleHandlingReason::NotOptionExpression,
        ),
        (
            "receive |value String?| -> String:\n    return value?\n;\n",
            InvalidFallibleHandlingReason::FunctionHasNoOptionalReturn,
        ),
        (
            "find || -> String?:\n    return \"Priya\"\n;\nfind()?\n",
            InvalidFallibleHandlingReason::FunctionHasNoOptionalReturn,
        ),
        (
            "receive |value String?|:\n    io.line(value?)\n;\n",
            InvalidFallibleHandlingReason::FunctionHasNoOptionalReturn,
        ),
        (
            "receive |value String?| -> String?, I32:\n    name = value?\n    return name, 1\n;\n",
            InvalidFallibleHandlingReason::FunctionHasMultipleSuccessReturns { count: 2 },
        ),
        (
            "receive |value String?| -> I32?:\n    name = value?\n    return 1\n;\n",
            InvalidFallibleHandlingReason::OptionPropagationReturnTypeMismatch,
        ),
    ];
    for (source, expected_reason) in cases {
        let diagnostic = parse_single_file_ast_diagnostic(source);
        assert!(matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidFallibleHandling { reason } if reason == expected_reason
        ));
        assert_postfix_span(source, '?', diagnostic.primary_span);
    }
}

#[test]
fn error_propagation_without_error_slot_retains_authored_operator_span() {
    let source = "produce || -> I32, Error!:\n    return! Error(\"failed\")\n;\nreceive || -> I32:\n    return produce()!\n;\n";
    let diagnostic = parse_single_file_ast_diagnostic(source);
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling {
            reason: InvalidFallibleHandlingReason::FunctionHasNoErrorSlot,
        }
    ));
    assert_postfix_span(source, '!', diagnostic.primary_span);
}

#[test]
fn error_propagation_incompatible_entry_retains_error_types_and_operator_span() {
    let source = "produce || -> I32, String!:\n    return! \"failed\"\n;\nvalue = produce()!\n";
    let diagnostic = parse_single_file_ast_diagnostic(source);
    let expected_error = parsed_builtin_error_type();
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling {
            reason: InvalidFallibleHandlingReason::EntryHasIncompatibleError {
                produced_error_type: builtin_type_ids::STRING,
                entry_error_type,
            },
        } if entry_error_type == expected_error
    ));
    assert_postfix_span(source, '!', diagnostic.primary_span);
}

#[test]
fn error_propagation_return_mismatch_retains_error_types_and_operator_span() {
    let source = "produce || -> I32, String!:\n    return! \"failed\"\n;\nreceive || -> I32, Error!:\n    return produce()!\n;\n";
    let diagnostic = parse_single_file_ast_diagnostic(source);
    let expected_error = parsed_builtin_error_type();
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::TypeMismatch {
            expected,
            found: builtin_type_ids::STRING,
            context: TypeMismatchContext::ErrorReturn,
        } if expected == expected_error
    ));
    assert_postfix_span(source, '!', diagnostic.primary_span);
}

fn parsed_builtin_error_type() -> crate::compiler_frontend::datatypes::ids::TypeId {
    use crate::compiler_frontend::canonical_type_identity::{
        CanonicalBuiltinType, CanonicalTypeIdentity,
    };
    let (ast, _, _) = parse_single_file_ast("");
    ast.type_environment
        .type_id_for_canonical_identity(&CanonicalTypeIdentity::Builtin(
            CanonicalBuiltinType::Error,
        ))
        .expect("entry Error contract")
}

fn assert_postfix_span(
    source: &str,
    operator: char,
    actual: Option<crate::compiler_frontend::source::SourceSpan>,
) {
    use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
    let start = source.rfind(operator).expect("authored postfix operator");
    let local = LocalSpan::exact(start as u32, 1, &mut ExtendedSpanBuilder::new()).unwrap();
    assert_eq!(
        actual,
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, local))
    );
}

// --------------------------
//  Catch handler with fallback
// --------------------------

#[test]
fn parses_catch_handler_with_fallback() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "can_error |value String| -> String, Error!:\n    return! Error(\"boom\")\n;\n\nrecover |value String| -> String:\n    output = can_error(value) catch |err|:\n        io.line([: [err.message]])\n        then \"fallback\"\n    ;\n    return output\n;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "recover");
    let NodeKind::VariableDeclaration(output_decl) = &body[0].kind else {
        panic!("expected declaration statement in recover()")
    };

    let ExpressionOperationView::ValueBlock(block) = ast
        .expressions
        .view(
            output_decl
                .value
                .initializer()
                .expect("fixture initializer"),
        )
        .unwrap()
        .operation()
    else {
        panic!("expected catch value block in recover declaration")
    };
    let ValueBlock::Catch(value_catch) = block else {
        panic!("expected catch value block")
    };
    let ExpressionOperationView::Call(call) = ast
        .expressions
        .view(value_catch.handled_value)
        .unwrap()
        .operation()
    else {
        panic!("expected handled call expression in catch value block")
    };
    assert!(matches!(
        call.handling().unwrap().handling,
        FallibleExpressionHandling::Recover
    ));
    assert_eq!(
        ast.expressions
            .view(value_catch.handled_value)
            .unwrap()
            .success_types()
            .len(),
        1
    );
    let FallibleHandling::Handler {
        body: handler_body, ..
    } = &value_catch.handler
    else {
        panic!("expected catch handler handling")
    };

    assert_eq!(handler_body.len(), 2);
    assert!(matches!(handler_body[1].kind, NodeKind::ThenValue(_)));
}

#[test]
fn parses_catch_handler_fallback_that_reads_error_binding() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "can_error |value String| -> String, Error!:\n    return! Error(\"boom\")\n;\n\nrecover |value String| -> String:\n    output = can_error(value) catch |err|:\n        io.line([: [err.code]])\n        then err.message\n    ;\n    return output\n;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "recover");
    let NodeKind::VariableDeclaration(output_decl) = &body[0].kind else {
        panic!("expected declaration statement in recover()")
    };

    let ExpressionOperationView::ValueBlock(block) = ast
        .expressions
        .view(
            output_decl
                .value
                .initializer()
                .expect("fixture initializer"),
        )
        .unwrap()
        .operation()
    else {
        panic!("expected catch value block in recover declaration")
    };
    let ValueBlock::Catch(value_catch) = block else {
        panic!("expected catch value block")
    };
    let ExpressionOperationView::Call(call) = ast
        .expressions
        .view(value_catch.handled_value)
        .unwrap()
        .operation()
    else {
        panic!("expected handled call expression in catch value block")
    };
    assert!(matches!(
        call.handling().unwrap().handling,
        FallibleExpressionHandling::Recover
    ));
    let FallibleHandling::Handler { body, .. } = &value_catch.handler else {
        panic!("expected catch handler handling")
    };

    assert!(matches!(
        body.last().map(|node| &node.kind),
        Some(NodeKind::ThenValue(_))
    ));
}

#[test]
fn preserves_u32_peer_and_fallback_literal_types_in_expression_catch() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "increment |code U32| -> U32:\n    return code + 1\n;\n\
         recover |code U32| -> U32:\n    return code + 1 catch then 0\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "recover");
    let NodeKind::Return(values) = &body[0].kind else {
        panic!("expected recovered return");
    };
    let u32_type_id = builtin_type_ids::fixed_scalar(FixedScalar::U32);
    assert_eq!(
        ast.expressions
            .view(values[0])
            .unwrap()
            .value_type_id()
            .unwrap(),
        u32_type_id
    );
    let ExpressionOperationView::ValueBlock(block) =
        ast.expressions.view(values[0]).unwrap().operation()
    else {
        panic!("expected expression catch");
    };
    let ValueBlock::Catch(catch) = block else {
        panic!("expected catch value block");
    };
    assert_eq!(
        ast.expressions
            .view(catch.handled_value)
            .unwrap()
            .value_type_id()
            .unwrap(),
        u32_type_id
    );
    let FallibleHandling::Handler { body, .. } = &catch.handler else {
        panic!("expected recovery handler");
    };
    let NodeKind::ThenValue(fallback) = &body[0].kind else {
        panic!("expected literal fallback");
    };
    assert_eq!(
        ast.expressions
            .view(fallback.expressions[0])
            .unwrap()
            .value_type_id()
            .unwrap(),
        u32_type_id
    );
}

#[test]
fn parses_inline_catch_fallback_as_value_block() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "can_error || -> I32, Error!:\n    return! Error(\"boom\")\n;\n\nrecover || -> I32:\n    value = can_error() catch then 0\n    return value\n;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "recover");
    let NodeKind::VariableDeclaration(value_decl) = &body[0].kind else {
        panic!("expected declaration statement in recover()")
    };

    let ExpressionOperationView::ValueBlock(block) = ast
        .expressions
        .view(value_decl.value.initializer().expect("fixture initializer"))
        .unwrap()
        .operation()
    else {
        panic!("expected inline catch to parse as a value block")
    };
    let ValueBlock::Catch(value_catch) = block else {
        panic!("expected catch value block")
    };
    let ExpressionOperationView::Call(call) = ast
        .expressions
        .view(value_catch.handled_value)
        .unwrap()
        .operation()
    else {
        panic!("expected handled call expression in catch value block")
    };
    assert!(matches!(
        call.handling().unwrap().handling,
        FallibleExpressionHandling::Recover
    ));
    let FallibleHandling::Handler { body, .. } = &value_catch.handler else {
        panic!("expected catch handler handling")
    };

    assert_eq!(body.len(), 1);
    assert!(matches!(body[0].kind, NodeKind::ThenValue(_)));
}

#[test]
fn parses_inline_catch_fallback_that_reads_error_binding() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "can_error |value String| -> String, Error!:\n    return! Error(\"boom\")\n;\n\nrecover |value String| -> String:\n    output = can_error(value) catch |err| then err.message\n    return output\n;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "recover");
    let NodeKind::VariableDeclaration(output_decl) = &body[0].kind else {
        panic!("expected declaration statement in recover()")
    };

    let ExpressionOperationView::ValueBlock(block) = ast
        .expressions
        .view(
            output_decl
                .value
                .initializer()
                .expect("fixture initializer"),
        )
        .unwrap()
        .operation()
    else {
        panic!("expected inline catch to parse as a value block")
    };
    let ValueBlock::Catch(value_catch) = block else {
        panic!("expected catch value block")
    };
    let ExpressionOperationView::Call(call) = ast
        .expressions
        .view(value_catch.handled_value)
        .unwrap()
        .operation()
    else {
        panic!("expected handled call expression in catch value block")
    };
    assert!(matches!(
        call.handling().unwrap().handling,
        FallibleExpressionHandling::Recover
    ));
    let FallibleHandling::Handler { body, error } = &value_catch.handler else {
        panic!("expected catch handler handling")
    };

    assert!(error.is_some());
    assert_eq!(body.len(), 1);
    assert!(matches!(body[0].kind, NodeKind::ThenValue(_)));
}

// --------------------------
//  Catch handler without fallback (terminating body)
// --------------------------

#[test]
fn parses_catch_handler_without_fallback_when_handler_guarantees_return() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "can_error |value String| -> String, Error!:\n    return! Error(\"boom\")\n;\n\nrecover |value String| -> String:\n    output = can_error(value) catch |err|:\n        return \"recovered\"\n    ;\n    return output\n;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "recover");
    let NodeKind::VariableDeclaration(output_decl) = &body[0].kind else {
        panic!("expected declaration statement in recover()")
    };

    let ExpressionOperationView::ValueBlock(block) = ast
        .expressions
        .view(
            output_decl
                .value
                .initializer()
                .expect("fixture initializer"),
        )
        .unwrap()
        .operation()
    else {
        panic!("expected catch value block in recover declaration")
    };
    let ValueBlock::Catch(value_catch) = block else {
        panic!("expected catch value block")
    };
    let ExpressionOperationView::Call(call) = ast
        .expressions
        .view(value_catch.handled_value)
        .unwrap()
        .operation()
    else {
        panic!("expected handled call expression in catch value block")
    };
    assert!(matches!(
        call.handling().unwrap().handling,
        FallibleExpressionHandling::Recover
    ));
    let FallibleHandling::Handler {
        body: handler_body, ..
    } = &value_catch.handler
    else {
        panic!("expected catch handler handling")
    };

    assert!(matches!(handler_body[0].kind, NodeKind::Return(_)));
}

#[test]
fn parses_catch_handler_without_fallback_when_handler_ends_with_assert_false() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "can_error |value String| -> String, Error!:\n    return! Error(\"boom\")\n;\n\nrecover |value String| -> String:\n    output = can_error(value) catch |err|:\n        io.line([: [err.message]])\n        assert(false, \"unreachable error path\")\n    ;\n    return output\n;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "recover");
    let NodeKind::VariableDeclaration(output_decl) = &body[0].kind else {
        panic!("expected declaration statement in recover()")
    };

    let ExpressionOperationView::ValueBlock(block) = ast
        .expressions
        .view(
            output_decl
                .value
                .initializer()
                .expect("fixture initializer"),
        )
        .unwrap()
        .operation()
    else {
        panic!("expected catch value block in recover declaration")
    };
    let ValueBlock::Catch(value_catch) = block else {
        panic!("expected catch value block")
    };
    let ExpressionOperationView::Call(call) = ast
        .expressions
        .view(value_catch.handled_value)
        .unwrap()
        .operation()
    else {
        panic!("expected handled call expression in catch value block")
    };
    assert!(matches!(
        call.handling().unwrap().handling,
        FallibleExpressionHandling::Recover
    ));
    let FallibleHandling::Handler {
        body: handler_body, ..
    } = &value_catch.handler
    else {
        panic!("expected catch handler handling")
    };

    assert_eq!(handler_body.len(), 2);
    assert!(matches!(handler_body[1].kind, NodeKind::Assert { .. }));
}

#[test]
fn parses_catch_handler_without_fallback_when_handler_ends_with_assert_false_no_binding() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "can_error |value String| -> String, Error!:\n    return! Error(\"boom\")\n;\n\nrecover |value String| -> String:\n    output = can_error(value) catch:\n        assert(false, \"unreachable error path\")\n    ;\n    return output\n;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "recover");
    let NodeKind::VariableDeclaration(output_decl) = &body[0].kind else {
        panic!("expected declaration statement in recover()")
    };

    let ExpressionOperationView::ValueBlock(block) = ast
        .expressions
        .view(
            output_decl
                .value
                .initializer()
                .expect("fixture initializer"),
        )
        .unwrap()
        .operation()
    else {
        panic!("expected catch value block in recover declaration")
    };
    let ValueBlock::Catch(value_catch) = block else {
        panic!("expected catch value block")
    };
    let ExpressionOperationView::Call(call) = ast
        .expressions
        .view(value_catch.handled_value)
        .unwrap()
        .operation()
    else {
        panic!("expected handled call expression in catch value block")
    };
    assert!(matches!(
        call.handling().unwrap().handling,
        FallibleExpressionHandling::Recover
    ));
    let FallibleHandling::Handler {
        body: handler_body, ..
    } = &value_catch.handler
    else {
        panic!("expected catch handler handling")
    };

    assert_eq!(handler_body.len(), 1);
    assert!(matches!(handler_body[0].kind, NodeKind::Assert { .. }));
}

// --------------------------
//  Catch handler fallthrough rejection
// --------------------------

#[test]
fn rejects_catch_handler_without_fallback_when_handler_can_fall_through() {
    assert_invalid_fallible_handling(
        "can_error |value String| -> String, Error!:\n    return! Error(\"boom\")\n;\n\nrecover |value String, route Bool| -> String:\n    return can_error(value) catch |err|:\n        if route:\n            io.line([: [err.message]])\n        else\n            io.line([: [err.code]])\n        ;\n    ;\n;\n",
        InvalidFallibleHandlingReason::CatchHandlerCanFallThrough,
    );
}

#[test]
fn rejects_catch_handler_if_without_else_even_when_then_branch_returns() {
    assert_invalid_fallible_handling(
        "can_error |value String| -> String, Error!:\n    return! Error(\"boom\")\n;\n\nrecover |value String, route Bool| -> String:\n    return can_error(value) catch:\n        if route:\n            return \"fallback\"\n        ;\n    ;\n;\n",
        InvalidFallibleHandlingReason::CatchHandlerCanFallThrough,
    );
}

#[test]
fn accepts_catch_handler_with_mixed_produce_and_return_paths() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "can_error |value String| -> String, Error!:\n    return! Error(\"boom\")\n;\n\nrecover |value String, route Bool| -> String:\n    return can_error(value) catch |err|:\n        if route:\n            then \"fallback\"\n        else\n            return \"returned\"\n        ;\n    ;\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "recover");
    assert!(
        matches!(body[0].kind, NodeKind::Return(_)),
        "mixed produce/terminate catch handlers must parse as a value-producing return"
    );
}

#[test]
fn rejects_assignment_target_read_inside_catch_fallback() {
    assert_invalid_assignment_target(
        "can_error || -> I32, Error!:\n    return 1\n;\n\nrecover || -> I32:\n    value ~= 0\n    value = can_error() catch:\n        then value\n    ;\n    return value\n;\n",
        InvalidAssignmentTargetReason::UnavailableInCatchRecovery,
    );
}

#[test]
fn rejects_assignment_target_read_inside_inline_catch_fallback() {
    assert_invalid_assignment_target(
        "can_error || -> I32, Error!:\n    return 1\n;\n\nrecover || -> I32:\n    value ~= 0\n    value = can_error() catch then value\n    return value\n;\n",
        InvalidAssignmentTargetReason::UnavailableInCatchRecovery,
    );
}

#[test]
fn rejects_assignment_target_mutation_inside_catch_handler() {
    assert_invalid_assignment_target(
        "can_error || -> I32, Error!:\n    return 1\n;\n\nrecover || -> I32:\n    value ~= 0\n    value = can_error() catch |err|:\n        value = 2\n        then 0\n    ;\n    return value\n;\n",
        InvalidAssignmentTargetReason::UnavailableInCatchRecovery,
    );
}

#[test]
fn rejects_multiline_inline_catch_fallback_value() {
    assert_invalid_fallible_handling(
        "can_error || -> I32, Error!:\n    return 1\n;\n\nrecover || -> I32:\n    value = can_error() catch then\n        0\n    return value\n;\n",
        InvalidFallibleHandlingReason::InlineCatchMultiline,
    );
}

#[test]
fn rejects_bound_inline_catch_with_line_break_before_then() {
    assert_invalid_fallible_handling(
        "can_error || -> I32, Error!:\n    return 1\n;\n\nrecover || -> I32:\n    value = can_error() catch |err|\n        then err.code\n    return value\n;\n",
        InvalidFallibleHandlingReason::InlineCatchMultiline,
    );
}

#[test]
fn rejects_unbound_inline_catch_then_at_eof() {
    // A missing inline catch fallback at a real EOF boundary must report the shared
    // missing-value reason at the boundary, not the multiline recovery reason.
    let source = "can_error || -> I32, Error!:\n    return 1\n;\n\nvalue = can_error() catch then";
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling { reason }
            if reason == InvalidFallibleHandlingReason::ThenRequiresValues
    ));
}

#[test]
fn rejects_bound_inline_catch_then_at_eof() {
    // The bound `catch |err| then` form reaches the same boundary routing through its
    // own binding-parse path and must report the same missing-value reason at the EOF
    // boundary.
    let source =
        "can_error || -> I32, Error!:\n    return 1\n;\n\nvalue = can_error() catch |err| then";
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling { reason }
            if reason == InvalidFallibleHandlingReason::ThenRequiresValues
    ));
}

#[test]
fn rejects_string_literal_as_catch_handler_binding() {
    assert_invalid_fallible_handling(
        "can_error || -> I32, Error!:\n    return 1\n;\n\nrecover || -> I32:\n    value = can_error() catch |\"oops\"|:\n        then 0\n    ;\n    return value\n;\n",
        InvalidFallibleHandlingReason::ExpectedCatchHandlerIdentifier,
    );
}

// --------------------------
//  Fallback arity validation
// --------------------------

#[test]
fn rejects_fallback_arity_mismatch_for_multi_value_success_returns() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "pair_error |value String| -> String, I32, Error!:\n    return! Error(\"boom\")\n;\n\nrecover |value String| -> String, I32:\n    first, count = pair_error(value) catch:\n        then \"fallback\", 0, 1\n    ;\n    return first, count\n;\n",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidReturnShape {
            reason: InvalidReturnShapeReason::TooManyReturnValues { .. }
        }
    ));
}

#[test]
fn rejects_fallback_type_mismatch_before_hir_lowering() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "can_error || -> I32, Error!:\n    return 1\n;\n\nrecover || -> I32:\n    return can_error() catch:\n        then \"fallback\"\n    ;\n;\n",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::TypeMismatch {
            context: TypeMismatchContext::General,
            ..
        }
    ));
}

#[test]
fn rejects_fallback_too_few_values_for_multi_success() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "pair_error |value String| -> String, I32, Error!:\n    return! Error(\"boom\")\n;\n\nrecover |value String| -> String, I32:\n    first, count = pair_error(value) catch:\n        then \"fallback\"\n    ;\n    return first, count\n;\n",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidReturnShape {
            reason: InvalidReturnShapeReason::TooFewReturnValues {
                expected_count: 2,
                provided_count: 1
            },
        }
    ));
}

#[test]
fn rejects_fallback_too_many_values_for_single_success() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "can_error || -> I32, Error!:\n    return 1\n;\n\nrecover || -> I32:\n    return can_error() catch:\n        then 0, 1\n    ;\n;\n",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidReturnShape {
            reason: InvalidReturnShapeReason::TooManyReturnValues { expected_count: 1 },
        }
    ));
}

#[test]
fn rejects_fallback_statement_after_then_multiline() {
    // A bare literal after `then` is still not a valid statement, even though
    // ordinary statements after `then` are now parsed as unreachable handler tails.
    let diagnostic = parse_single_file_ast_diagnostic(
        "can_error || -> I32, Error!:\n    return 1\n;\n\nrecover || -> I32:\n    return can_error() catch:\n        then 0\n        1\n    ;\n;\n",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedToken { .. }
    ));
}

#[test]
fn accepts_then_none_for_optional_success() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "find_name || -> String?, Error!:\n    return! Error(\"boom\")\n;\n\nrecover || -> String?:\n    return find_name() catch:\n        then none\n    ;\n;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "recover");
    let NodeKind::Return(return_values) = &body[0].kind else {
        panic!("expected return statement in recover()")
    };

    assert!(matches!(
        ast.expressions.view(return_values[0]).unwrap().operation(),
        ExpressionOperationView::ValueBlock(_)
    ));
}

#[test]
fn accepts_then_string_for_optional_success() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "find_name || -> String?, Error!:\n    return! Error(\"boom\")\n;\n\nrecover || -> String?:\n    return find_name() catch:\n        then \"fallback\"\n    ;\n;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "recover");
    let NodeKind::Return(return_values) = &body[0].kind else {
        panic!("expected return statement in recover()")
    };

    assert!(matches!(
        ast.expressions.view(return_values[0]).unwrap().operation(),
        ExpressionOperationView::ValueBlock(_)
    ));
}

#[test]
fn rejects_empty_catch_on_success_producing_call_without_then() {
    assert_invalid_fallible_handling(
        "can_error || -> I32, Error!:\n    return 1\n;\n\nrecover || -> I32:\n    return can_error() catch:\n    ;\n;\n",
        InvalidFallibleHandlingReason::CatchHandlerCanFallThrough,
    );
}

#[test]
fn accepts_empty_catch_on_zero_success_statement() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "fail || -> Error!:\n    return! Error(\"boom\")\n;\n\nrecover ||:\n    fail() catch:\n    ;\n;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "recover");
    let NodeKind::ExpressionStatement(expression) = &body[0].kind else {
        panic!("expected expression statement in recover()")
    };

    let ExpressionOperationView::ValueBlock(block) =
        ast.expressions.view(*expression).unwrap().operation()
    else {
        panic!("expected zero-success catch statement to parse as a catch value block");
    };
    let ValueBlock::Catch(value_catch) = block else {
        panic!("expected catch value block");
    };
    assert!(
        ast.expressions
            .view(*expression)
            .unwrap()
            .success_types()
            .is_empty()
    );
    assert!(
        ast.expressions
            .view(value_catch.handled_value)
            .unwrap()
            .success_types()
            .is_empty()
    );
    assert_eq!(
        ast.expressions.view(*expression).unwrap().error_type(),
        None
    );
    assert!(matches!(
        ast.expressions.view(value_catch.handled_value).unwrap().operation(),
        ExpressionOperationView::Call(call) if matches!(call.handling().unwrap().handling, FallibleExpressionHandling::Recover)
    ));
}

#[test]
fn accepts_nested_postfix_propagation_in_fallback() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "inner || -> I32, Error!:\n    return 1\n;\n\nouter || -> I32, Error!:\n    return 2\n;\n\nrecover || -> I32, Error!:\n    return outer() catch:\n        then inner()!\n    ;\n;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "recover");
    let NodeKind::Return(return_values) = &body[0].kind else {
        panic!("expected return statement in recover()")
    };

    assert!(matches!(
        ast.expressions.view(return_values[0]).unwrap().operation(),
        ExpressionOperationView::ValueBlock(_)
    ));
}

#[test]
fn accepts_then_inside_nested_catch_branch() {
    parse_single_file_ast(
        "can_error || -> I32, Error!:\n    return 1\n;\n\nrecover |route Bool| -> I32:\n    return can_error() catch:\n        if route:\n            then 0\n        else\n            then 1\n        ;\n    ;\n;\n",
    );
}

#[test]
fn accepts_statement_after_then_as_unreachable_handler_tail() {
    parse_single_file_ast(
        "can_error || -> I32, Error!:\n    return 1\n;\n\nrecover || -> I32:\n    return can_error() catch:\n        then 0\n        io.line([: [\"late\"]])\n    ;\n;\n",
    );
}

// --------------------------
//  Boundary-only catch placement
// --------------------------

#[test]
fn rejects_catch_inside_function_call_argument() {
    assert_invalid_fallible_handling(
        "can_error |value String| -> String, Error!:\n    return value\n;\n\nrender |value String| -> String:\n    return value\n;\n\nrecover || -> String:\n    return render(can_error(\"x\") catch:\n        then \"fallback\"\n    ;)\n;\n",
        InvalidFallibleHandlingReason::CatchOutsideBoundary,
    );
}

#[test]
fn rejects_catch_inside_parenthesized_arithmetic_expression() {
    assert_invalid_fallible_handling(
        "can_error || -> I32, Error!:\n    return 1\n;\n\nrecover || -> I32:\n    value = 1 + (can_error() catch:\n        then 0\n    ;)\n    return value\n;\n",
        InvalidFallibleHandlingReason::CatchOutsideBoundary,
    );
}

#[test]
fn rejects_catch_inside_parenthesized_field_access_base() {
    assert_invalid_fallible_handling(
        "User = |\n    name String,\n|\n\nload_user || -> User, Error!:\n    return User(\"Ana\")\n;\n\nrecover || -> String:\n    default_user = User(\"fallback\")\n    return (load_user() catch:\n        then default_user\n    ;).name\n;\n",
        InvalidFallibleHandlingReason::CatchOutsideBoundary,
    );
}

#[test]
fn rejects_catch_inside_collection_literal_item() {
    assert_invalid_fallible_handling(
        "can_error || -> I32, Error!:\n    return 1\n;\n\nrecover || -> {I32}:\n    return {can_error() catch:\n        then 0\n    ;}\n;\n",
        InvalidFallibleHandlingReason::CatchOutsideBoundary,
    );
}

#[test]
fn rejects_catch_inside_if_condition() {
    assert_invalid_fallible_handling(
        "can_error || -> Bool, Error!:\n    return true\n;\n\nrecover || -> String:\n    if can_error() catch:\n        then false\n    ;:\n        return \"yes\"\n    else\n        return \"no\"\n    ;\n;\n",
        InvalidFallibleHandlingReason::CatchOutsideBoundary,
    );
}

#[test]
fn rejects_catch_inside_loop_condition() {
    assert_invalid_fallible_handling(
        "can_error || -> Bool, Error!:\n    return false\n;\n\nrecover || -> I32:\n    count ~= 0\n    loop can_error() catch:\n        then false\n    ;:\n        count = count + 1\n    ;\n    return count\n;\n",
        InvalidFallibleHandlingReason::CatchOutsideBoundary,
    );
}

#[test]
fn rejects_catch_inside_range_loop_bound() {
    assert_invalid_fallible_handling(
        "can_error || -> I32, Error!:\n    return 3\n;\n\nrecover || -> I32:\n    count ~= 0\n    loop 0 to can_error() catch:\n        then 3\n    ; |value|:\n        count = count + value\n    ;\n    return count\n;\n",
        InvalidFallibleHandlingReason::CatchOutsideBoundary,
    );
}

#[test]
fn rejects_catch_inside_collection_loop_source() {
    assert_invalid_fallible_handling(
        "can_error || -> {I32}, Error!:\n    return {1, 2}\n;\n\nrecover || -> I32:\n    count ~= 0\n    loop can_error() catch:\n        then {0}\n    ; |value|:\n        count = count + value\n    ;\n    return count\n;\n",
        InvalidFallibleHandlingReason::CatchOutsideBoundary,
    );
}

#[test]
fn rejects_catch_inside_template_interpolation() {
    assert_invalid_fallible_handling(
        "can_error || -> String, Error!:\n    return \"ok\"\n;\n\nrecover || -> String:\n    return [:value=[can_error() catch:\n        then \"fallback\"\n    ;]]\n;\n",
        InvalidFallibleHandlingReason::CatchOutsideBoundary,
    );
}

fn assert_invalid_fallible_handling(source: &str, expected_reason: InvalidFallibleHandlingReason) {
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling { reason } if reason == expected_reason
    ));
}

fn assert_invalid_assignment_target(source: &str, expected_reason: InvalidAssignmentTargetReason) {
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidAssignmentTarget { reason, .. } if reason == expected_reason
    ));
}

#[test]
fn preserves_multiple_success_slots_through_catch() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "pair || -> String, I32, Error!:\n    return! Error(\"missing\")\n;\n\nrecover || -> String, I32:\n    return pair() catch:\n        then \"guest\", 0\n    ;\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "recover");
    let NodeKind::Return(values) = &body[0].kind else {
        panic!("expected forwarded catch return");
    };
    let [expression] = values.as_slice() else {
        panic!("expected one producer for both returned slots");
    };
    assert_eq!(
        ast.expressions.view(*expression).unwrap().success_types(),
        &[
            builtin_type_ids::STRING,
            builtin_type_ids::fixed_scalar(FixedScalar::I32)
        ],
    );
    assert_eq!(
        ast.expressions.view(*expression).unwrap().error_type(),
        None
    );
    let ExpressionOperationView::ValueBlock(block) =
        ast.expressions.view(*expression).unwrap().operation()
    else {
        panic!("expected catch value block");
    };
    let ValueBlock::Catch(catch) = block else {
        panic!("expected catch value block");
    };
    assert_eq!(
        ast.expressions
            .view(catch.handled_value)
            .unwrap()
            .success_types(),
        ast.expressions.view(*expression).unwrap().success_types()
    );
}
