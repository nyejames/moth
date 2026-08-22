//! Function-signature and call parsing regression tests.
//!
//! WHAT: validates the AST shapes produced for function declarations and call sites.
//! WHY: statement parsing should preserve signature metadata and host/user call dispatch.

use crate::compiler_frontend::ast::ast_nodes::{MatchExhaustiveness, NodeKind};
use crate::compiler_frontend::ast::expressions::expression::{
    ExpressionKind, FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::ast::statements::functions::ReturnChannel;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidCallShapeReason, InvalidFunctionSignatureReason,
    InvalidReceiverCallReason, InvalidReceiverDeclarationReason, InvalidThisUsageReason,
    NameNamespace, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, function_signature_by_name, start_function_body,
};
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_build_result, parse_single_file_ast_diagnostic,
};

fn parse_function_diagnostic_payload(source: &str) -> DiagnosticPayload {
    parse_single_file_ast_diagnostic(source).payload
}

fn assert_type_mismatch_context(source: &str, expected_context: TypeMismatchContext) {
    let payload = parse_function_diagnostic_payload(source);

    assert!(
        matches!(
            payload,
            DiagnosticPayload::TypeMismatch {
                context,
                ..
            } if context == expected_context
        ),
        "{payload:?}"
    );
}

fn assert_invalid_receiver_call(source: &str, expected_reason: InvalidReceiverCallReason) {
    let payload = parse_function_diagnostic_payload(source);

    assert!(
        matches!(
            payload,
            DiagnosticPayload::InvalidReceiverCall {
                reason,
                ..
            } if reason == expected_reason
        ),
        "{payload:?}"
    );
}

// --------------------------
//  Signature parsing
// --------------------------

#[test]
fn function_default_path_token_reports_structured_diagnostic_not_panic() {
    // A path token nested in a template default lives in the detached parameter
    // default buffer; the header must own its path row so AST parsing reports the
    // missing target as a structured diagnostic instead of indexing a missing row.
    let payload = parse_function_diagnostic_payload(
        "label |prefix String = [: [@docs/intro.md] ]| -> String:\n    return prefix\n;\n",
    );
    assert!(
        matches!(
            payload,
            DiagnosticPayload::InvalidCompileTimePath {
                reason: crate::compiler_frontend::compiler_messages::InvalidCompileTimePathReason::MissingTarget,
                ..
            }
        ),
        "expected a structured missing-target diagnostic, got {payload:?}"
    );
}

#[test]
fn parses_function_parameters_and_return_types() {
    let (ast, string_table) =
        parse_single_file_ast("add |left Int, right Int| -> Int:\n    return left + right\n;\n");

    let signature = function_signature_by_name(&ast, &string_table, "add");

    assert_eq!(signature.parameters.len(), 2);
    assert_eq!(
        signature.parameters[0].id.name_str(&string_table),
        Some("left")
    );
    assert_eq!(
        signature.parameters[1].id.name_str(&string_table),
        Some("right")
    );
    assert_eq!(signature.returns.len(), 1);
    assert_eq!(signature.returns[0].value, DataType::Int);
    assert_eq!(signature.returns[0].channel, ReturnChannel::Success);
    assert!(
        signature.returns[0].type_id.is_some(),
        "return type_id should be resolved"
    );
}

#[test]
fn parses_final_error_return_slot_in_function_signature() {
    let (ast, string_table) =
        parse_single_file_ast("compute |x Int| -> Int, Error!:\n    return x\n;\n");

    let signature = function_signature_by_name(&ast, &string_table, "compute");

    assert_eq!(signature.returns.len(), 2);
    assert_eq!(signature.returns[0].value, DataType::Int);
    assert_eq!(signature.returns[0].channel, ReturnChannel::Success);
    assert!(
        signature.returns[0].type_id.is_some(),
        "success return type_id should be resolved"
    );
    assert!(
        matches!(&signature.returns[1].value, DataType::Struct { .. }),
        "builtin Error should resolve to a struct-shaped diagnostic type"
    );
    assert!(
        signature.returns[1].type_id.is_some(),
        "error return type_id should be resolved"
    );
    assert_eq!(signature.returns[1].channel, ReturnChannel::Error);
}

#[test]
fn parses_optional_final_error_return_slot() {
    let (ast, string_table) =
        parse_single_file_ast("compute |x Int| -> Int, String?!:\n    return x\n;\n");

    let signature = function_signature_by_name(&ast, &string_table, "compute");

    assert_eq!(signature.returns.len(), 2);
    assert_eq!(
        signature.returns[1].value,
        DataType::Option(Box::new(DataType::StringSlice))
    );
    assert_eq!(signature.returns[1].channel, ReturnChannel::Error);
    assert!(
        signature.returns[1].type_id.is_some(),
        "optional error return type_id should be resolved"
    );
}

#[test]
fn rejects_non_final_error_return_slot() {
    let payload =
        parse_function_diagnostic_payload("compute |x Int| -> Error!, Int:\n    return 42\n;\n");

    assert_eq!(
        payload,
        DiagnosticPayload::InvalidFunctionSignature {
            reason: InvalidFunctionSignatureReason::ErrorSlotNotLast
        }
    );
}

#[test]
fn rejects_multiple_error_return_slots() {
    let payload = parse_function_diagnostic_payload(
        "compute |x Int| -> Int, String!, Error!:\n    return 42\n;\n",
    );

    assert_eq!(
        payload,
        DiagnosticPayload::InvalidFunctionSignature {
            reason: InvalidFunctionSignatureReason::MultipleErrorReturnSlots
        }
    );
}

#[test]
fn parses_generic_function_declaration_without_emitting_executable_function() {
    let (ast, string_table) = parse_single_file_ast(
        "identity type T |value T| -> T:\n    return value\n;\n\nio.line([: [\"ready\"]])\n",
    );

    let generic_function_emitted = ast.nodes.iter().any(|node| match &node.kind {
        NodeKind::Function(path, ..) => path.name_str(&string_table) == Some("identity"),
        _ => false,
    });

    assert!(
        !generic_function_emitted,
        "generic function declarations should be retained as AST templates until instantiated"
    );
}

#[test]
fn same_file_generic_calls_emit_requests_without_eager_function_bodies() {
    let (build_result, string_table) = parse_single_file_ast_build_result(
        "identity type T |value T| -> T:\n    return value\n;\n\nvalue = identity(1)\n",
    )
    .expect("source should parse into a request-only AST");
    let ast = build_result.ast;

    let generic_template_emitted = ast.nodes.iter().any(|node| match &node.kind {
        NodeKind::Function(path, ..) => path.name_str(&string_table) == Some("identity"),
        _ => false,
    });
    assert!(
        !generic_template_emitted,
        "generic function templates should not emit under their source name"
    );

    let start_body = start_function_body(&ast, &string_table);
    let NodeKind::VariableDeclaration(declaration) = &start_body[0].kind else {
        panic!("expected start body to store the generic call result")
    };
    let ExpressionKind::FunctionCall {
        name,
        result_type_ids,
        ..
    } = &declaration.value.kind
    else {
        panic!("expected variable initializer to call the concrete generic instance")
    };

    assert!(
        !ast.nodes
            .iter()
            .any(|node| matches!(&node.kind, NodeKind::Function(path, ..) if path == name)),
        "generic calls must not materialise concrete bodies inside the requester AST"
    );
    let [request] = build_result.deferred_generic_requests.as_slice() else {
        panic!("one generic call should emit one deferred request")
    };
    assert_eq!(name, &request.instance_path);
    assert_eq!(request.key.type_arguments.as_ref(), result_type_ids);
    assert_eq!(declaration.value.diagnostic_type, DataType::Int);
}

#[test]
fn fallible_generic_calls_remain_request_only() {
    let (build_result, _string_table) = parse_single_file_ast_build_result(
        "raise type E |err E| -> E!:\n    return! err\n;\n\nrecover || -> String:\n    raise(Error(\"boom\")) catch |err|:\n        return err.message\n    ;\n    return \"unreachable\"\n;\n\nvalue = recover()\n",
    )
    .expect("fallible generic call should emit a deferred request");

    let [request] = build_result.deferred_generic_requests.as_slice() else {
        panic!("one fallible generic call should emit one deferred request")
    };
    assert!(
        !build_result.ast.nodes.iter().any(
            |node| matches!(&node.kind, NodeKind::Function(path, ..) if path == &request.instance_path)
        ),
        "fallible generic bodies must be materialised by the generated-function transaction"
    );
}

#[test]
fn static_true_assertion_validates_but_discards_inactive_generic_message_requests() {
    let (build_result, string_table) = parse_single_file_ast_build_result(
        "identity type T |value T| -> T:\n    return value\n;\n\nassert(true, identity(\"inactive\"))\n",
    )
    .expect("inactive generic assertion message should still be frontend-valid");

    assert!(
        build_result.deferred_generic_requests.is_empty(),
        "static-true assertion messages must not publish generic requests"
    );
    let start_body = start_function_body(&build_result.ast, &string_table);
    assert!(
        start_body
            .iter()
            .any(|node| matches!(node.kind, NodeKind::Assert { .. })),
        "the assertion itself remains in the executable AST contract"
    );
}

#[test]
fn static_true_assertion_still_reports_invalid_generic_message() {
    let payload = parse_function_diagnostic_payload(
        "identity type T |value T| -> T:\n    return value\n;\n\nassert(true, identity(1))\n",
    );

    assert!(
        matches!(
            payload,
            DiagnosticPayload::TypeMismatch {
                context: TypeMismatchContext::AssertionArgument,
                ..
            }
        ),
        "inactive assertion messages must still report type errors, got {payload:?}"
    );
}

#[test]
fn static_false_assertion_retains_active_generic_message_requests() {
    let (build_result, _string_table) = parse_single_file_ast_build_result(
        "identity type T |value T| -> T:\n    return value\n;\n\nassert(false, identity(\"active\"))\n",
    )
    .expect("active generic assertion message should be frontend-valid");

    assert_eq!(
        build_result.deferred_generic_requests.len(),
        1,
        "static-false assertion messages remain executable on the failure edge"
    );
}

#[test]
fn dynamic_assertion_retains_generic_message_requests() {
    let (build_result, _string_table) = parse_single_file_ast_build_result(
        "condition || -> Bool:\n    return true\n;\n\nidentity type T |value T| -> T:\n    return value\n;\n\nassert(condition(), identity(\"dynamic\"))\n",
    )
    .expect("dynamic assertion message should remain frontend-valid");

    assert_eq!(
        build_result.deferred_generic_requests.len(),
        1,
        "dynamic assertion messages must retain their required generic request"
    );
}

// --------------------------
//  Call dispatch
// --------------------------

#[test]
fn start_function_distinguishes_user_and_host_calls() {
    let (ast, string_table) = parse_single_file_ast(
        "identity |value Int| -> Int:\n    return value\n;\n\nresult = identity(1)\nio.line([: [result]])\n",
    );

    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(result_decl) = &body[0].kind else {
        panic!("expected variable declaration for function result");
    };
    assert!(matches!(
        result_decl.value.kind,
        ExpressionKind::FunctionCall { .. }
    ));

    assert!(
        body.iter().any(|node| match &node.kind {
            NodeKind::ExpressionStatement(expression) => {
                matches!(expression.kind, ExpressionKind::HostFunctionCall { .. })
            }
            _ => false,
        }),
        "expected io.line([: [...]]) to parse as a host-function expression statement"
    );

    let function_body = function_body_by_name(&ast, &string_table, "identity");
    assert!(
        function_body
            .iter()
            .any(|node| matches!(node.kind, NodeKind::Return(..))),
        "explicit function body should preserve return statements"
    );
}

// --------------------------
//  Collection and struct parameter validation
// --------------------------

#[test]
fn rejects_immutable_collection_place_argument_to_mutable_parameter() {
    let payload = parse_function_diagnostic_payload(
        "touch |items ~{Int}| -> Int:\n    return items.length()\n;\n\nvalues = {1, 2, 3}\ncount = touch(values)\n",
    );

    assert!(
        matches!(
            payload,
            DiagnosticPayload::InvalidCallShape {
                reason: InvalidCallShapeReason::ImmutablePlaceMutableAccessRequired { .. },
                ..
            }
        ),
        "{payload:?}"
    );
}

#[test]
fn rejects_collection_element_type_mismatch_with_mutability_relaxation() {
    assert_type_mismatch_context(
        "sum |items {Int}| -> Int:\n    return items.length()\n;\n\nvalues ~= {true, false}\ncount = sum(values)\n",
        TypeMismatchContext::FunctionArgument,
    );
}

#[test]
fn rejects_struct_constructor_argument_type_with_field_wording() {
    assert_type_mismatch_context(
        "Point = |\n    x Int,\n    y Int,\n|\n\npoint = Point(x = 1, y = \"oops\")\n",
        TypeMismatchContext::ConstructorArgument,
    );
}

// --------------------------
//  Named types in signatures
// --------------------------

#[test]
fn resolves_named_struct_type_in_function_parameters() {
    let (ast, string_table) = parse_single_file_ast(
        "Point = |\n    x Int,\n|\n\nshow |value Point|:\n    io.line([: [value.x]])\n;\n",
    );

    let signature = function_signature_by_name(&ast, &string_table, "show");
    assert!(matches!(
        signature.parameters[0].value.diagnostic_type,
        DataType::Struct {
            const_record: false,
            ..
        }
    ));
}

#[test]
fn resolves_named_struct_type_in_function_returns() {
    let (ast, string_table) = parse_single_file_ast(
        "Point = |\n    x Int,\n|\n\nclone |value Point| -> Point:\n    return value\n;\n",
    );

    let signature = function_signature_by_name(&ast, &string_table, "clone");
    assert!(matches!(
        signature.returns[0].value,
        DataType::Struct {
            const_record: false,
            ..
        }
    ));
}

#[test]
fn rejects_unknown_named_type_in_function_signatures() {
    let payload =
        parse_function_diagnostic_payload("use_missing |value Missing|:\n    return value\n;\n");

    assert!(
        matches!(
            payload,
            DiagnosticPayload::UnknownName {
                namespace: NameNamespace::Type,
                ..
            }
        ),
        "{payload:?}"
    );
}

#[test]
fn rejects_unknown_named_return_type_in_function_signatures() {
    let payload =
        parse_function_diagnostic_payload("clone |value Int| -> Missing:\n    return value\n;\n");

    assert!(
        matches!(
            payload,
            DiagnosticPayload::UnknownName {
                namespace: NameNamespace::Type,
                ..
            }
        ),
        "{payload:?}"
    );
}

// --------------------------
//  Receiver parameter rules
// --------------------------

#[test]
fn rejects_receiver_parameter_when_not_first() {
    let payload = parse_function_diagnostic_payload(
        "Point = |\n    x Int,\n|\n\nreset |value Int, this Point|:\n    return value\n;\n",
    );

    assert!(
        matches!(
            payload,
            DiagnosticPayload::InvalidThisUsage {
                reason: InvalidThisUsageReason::NotFirstParameter { .. }
            }
        ),
        "{payload:?}"
    );
}

#[test]
fn rejects_builtin_scalar_receiver_parameter() {
    let payload =
        parse_function_diagnostic_payload("reset |this Int| -> Int:\n    return this\n;\n");

    assert!(
        matches!(
            payload,
            DiagnosticPayload::InvalidReceiverDeclaration {
                reason: InvalidReceiverDeclarationReason::BuiltinScalarType,
                ..
            }
        ),
        "{payload:?}"
    );
}

#[test]
fn rejects_multiple_this_parameters_in_one_signature() {
    let payload = parse_function_diagnostic_payload(
        "Point = |\n    x Int = 0,\n|\n\nlength |this Point, this Point| -> Int:\n    return this.x\n;\n",
    );

    assert!(
        matches!(
            payload,
            DiagnosticPayload::InvalidThisUsage {
                reason: InvalidThisUsageReason::DuplicateThis { .. }
            }
        ),
        "{payload:?}"
    );
}

// --------------------------
//  Method call rules
// --------------------------

#[test]
fn rejects_field_method_name_collisions() {
    let payload = parse_function_diagnostic_payload(
        "Point = |\n    reset Int = 0,\n|\n\nreset |this Point| -> Int:\n    return this.reset\n;\n",
    );

    assert_eq!(
        payload,
        DiagnosticPayload::InvalidReceiverDeclaration {
            reason: InvalidReceiverDeclarationReason::FieldNameConflict
        }
    );
}

#[test]
fn rejects_free_function_call_syntax_for_receiver_methods() {
    assert_invalid_receiver_call(
        "Point = |\n    x Int = 0,\n|\n\nreset |this Point|:\n    return\n;\n\npoint ~= Point()\nreset(point)\n",
        InvalidReceiverCallReason::CalledAsFreeFunction,
    );
}

#[test]
fn rejects_const_record_method_calls() {
    assert_invalid_receiver_call(
        "Point = |\n    x Int = 0,\n|\n\nlength |this Point| -> Int:\n    return this.x\n;\n\norigin #= Point()\norigin.length()\n",
        InvalidReceiverCallReason::ConstRecordNoRuntimeCalls,
    );
}

#[test]
fn parses_mutable_receiver_methods_with_explicit_receiver_tilde() {
    let (ast, string_table) = parse_single_file_ast(
        "Point = |\n    x Int = 0,\n|\n\nreset |this ~Point|:\n    this.x = 0\n;\n\npoint ~= Point()\n~point.reset()\n",
    );

    let body = start_function_body(&ast, &string_table);
    let NodeKind::ExpressionStatement(call_expr) = &body[1].kind else {
        panic!("expected receiver method statement");
    };
    assert!(
        matches!(call_expr.kind, ExpressionKind::MethodCall { .. }),
        "expected user-defined receiver method call expression"
    );
}

#[test]
fn parses_result_propagation_for_receiver_method_call() {
    parse_single_file_ast(
        "Point = |\n    x Int = 1,\n|\n\nread |this Point| -> Int, Error!:\n    return this.x\n;\n\nforward |point Point| -> Int, Error!:\n    return point.read()!\n;\n",
    );
}

#[test]
fn rejects_mutable_receiver_methods_on_immutable_bindings_without_marker() {
    assert_invalid_receiver_call(
        "Point = |\n    x Int = 0,\n|\n\nreset |this ~Point|:\n    this.x = 0\n;\n\npoint = Point()\npoint.reset()\n",
        InvalidReceiverCallReason::ImmutableReceiverMutableMethod,
    );
}

#[test]
fn rejects_explicit_receiver_tilde_for_shared_receiver_methods() {
    assert_invalid_receiver_call(
        "Point = |\n    x Int = 0,\n|\n\nlength |this Point| -> Int:\n    return this.x\n;\n\npoint ~= Point()\nvalue = ~point.length()\n",
        InvalidReceiverCallReason::UnneededMutableAccessMarker,
    );
}

// --------------------------
//  Fallible handling
// --------------------------

#[test]
fn parses_result_propagation_call_in_expression_position() {
    let (ast, string_table) = parse_single_file_ast(
        "can_error |value String| -> String, Error!:\n    return value\n;\n\nforward |value String| -> String, Error!:\n    return can_error(value)!\n;\n",
    );

    let body = function_body_by_name(&ast, &string_table, "forward");
    let NodeKind::Return(values) = &body[0].kind else {
        panic!("expected return statement in forward()");
    };

    assert_eq!(values.len(), 1);
    assert!(matches!(
        values[0].kind,
        ExpressionKind::HandledFallibleFunctionCall {
            handling: FallibleExpressionHandling::Propagate,
            ..
        }
    ));
}

#[test]
fn rejects_result_propagation_when_error_slot_types_do_not_match() {
    assert_type_mismatch_context(
        "ErrA = |\n    message String,\n|\n\nErrB = |\n    message String,\n|\n\ninner || -> Int, ErrA!:\n    return! ErrA(\"failed\")\n;\n\nouter || -> Int, ErrB!:\n    value = inner()!\n    return value\n;\n",
        TypeMismatchContext::ErrorReturn,
    );
}

#[test]
fn rejects_return_value_type_with_expected_found_and_value_details() {
    assert_type_mismatch_context(
        "render || -> String:\n    return 1\n;\n\nrender()\n",
        TypeMismatchContext::ReturnValue,
    );
}

#[test]
fn return_int_context_reports_targeted_guidance_for_regular_division() {
    assert_type_mismatch_context(
        "render || -> Int:\n    return 5 / 2\n;\n\nrender()\n",
        TypeMismatchContext::ReturnValue,
    );
}

#[test]
fn parses_result_fallback_call_in_expression_position() {
    let (ast, string_table) = parse_single_file_ast(
        "can_error |value String| -> String, Error!:\n    return value\n;\n\nrecover |value String| -> String:\n    return can_error(value) catch:\n        then \"fallback\"\n    ;\n;\n",
    );

    let body = function_body_by_name(&ast, &string_table, "recover");
    let NodeKind::Return(values) = &body[0].kind else {
        panic!("expected return statement in recover()");
    };

    assert_eq!(values.len(), 1);
    let ExpressionKind::ValueBlock { block } = &values[0].kind else {
        panic!("expected catch value block in recover return");
    };
    let ValueBlock::Catch(value_catch) = block.as_ref() else {
        panic!("expected catch value block");
    };
    let ExpressionKind::HandledFallibleFunctionCall { handling, .. } =
        &value_catch.handled_value.kind
    else {
        panic!("expected handled call expression in catch value block");
    };
    assert!(matches!(handling, FallibleExpressionHandling::Recover));
    let FallibleHandling::Handler { body, .. } = &value_catch.handler else {
        panic!("expected catch-block handling");
    };
    assert_eq!(body.len(), 1);
    assert!(matches!(body[0].kind, NodeKind::ThenValue(_)));
}

#[test]
fn parses_inline_choice_predicate_receiver_as_value_match() {
    let (ast, string_table) = parse_single_file_ast(
        "Status :: Ready, Waiting;\n\
         score_for |status Status| -> Int:\n\
             score = if status is Ready then 1 else 0\n\
             return score\n\
         ;\n",
    );

    let body = function_body_by_name(&ast, &string_table, "score_for");
    let NodeKind::VariableDeclaration(score_decl) = &body[0].kind else {
        panic!("expected score declaration in score_for()");
    };
    let ExpressionKind::ValueBlock { block } = &score_decl.value.kind else {
        panic!("expected inline choice predicate to parse as a value block");
    };
    let ValueBlock::Match(value_match) = block.as_ref() else {
        panic!("expected inline choice predicate to lower through value match");
    };

    assert_eq!(value_match.arms.len(), 1);
    assert!(
        matches!(
            value_match.arms[0].pattern,
            MatchPattern::ChoiceVariant { tag: 0, .. }
        ),
        "Ready should resolve as the first Status variant, not as an ordinary value name"
    );
    assert!(
        value_match.default.is_some(),
        "inline else branch should become the value-match default"
    );
    assert_eq!(value_match.exhaustiveness, MatchExhaustiveness::HasDefault);
}

#[test]
fn parses_inline_option_present_capture_receiver_as_value_match() {
    let (ast, string_table) = parse_single_file_ast(
        "display |maybe_name String?| -> String:\n\
             name = if maybe_name is |name| then name else \"guest\"\n\
             return name\n\
         ;\n",
    );

    let body = function_body_by_name(&ast, &string_table, "display");
    let NodeKind::VariableDeclaration(name_decl) = &body[0].kind else {
        panic!("expected name declaration in display()");
    };
    let ExpressionKind::ValueBlock { block } = &name_decl.value.kind else {
        panic!("expected inline option unwrap to parse as a value block");
    };
    let ValueBlock::Match(value_match) = block.as_ref() else {
        panic!("expected inline option unwrap to lower through value match");
    };

    assert_eq!(value_match.arms.len(), 1);
    assert!(
        matches!(
            value_match.arms[0].pattern,
            MatchPattern::OptionPresentCapture { .. }
        ),
        "the canonical option unwrap predicate should bind a present payload"
    );
    assert!(
        value_match.default.is_some(),
        "inline else branch should remain outside the present-capture scope"
    );
    assert_eq!(value_match.exhaustiveness, MatchExhaustiveness::HasDefault);
}

#[test]
fn parses_catch_handler_with_fallback_scope_in_declaration_rhs() {
    let (ast, string_table) = parse_single_file_ast(
        "can_error |value String| -> String, Error!:\n    return value\n;\n\nrecover |value String| -> String:\n    output = can_error(value) catch |err|:\n        io.line([: [err.message]])\n        then \"fallback\"\n    ;\n    return output\n;\n",
    );

    let body = function_body_by_name(&ast, &string_table, "recover");
    let NodeKind::VariableDeclaration(output_decl) = &body[0].kind else {
        panic!("expected declaration statement in recover()");
    };

    let ExpressionKind::ValueBlock { block } = &output_decl.value.kind else {
        panic!("expected catch value block in recover declaration");
    };
    let ValueBlock::Catch(value_catch) = block.as_ref() else {
        panic!("expected catch value block");
    };
    let ExpressionKind::HandledFallibleFunctionCall { handling, .. } =
        &value_catch.handled_value.kind
    else {
        panic!("expected handled call expression in catch value block");
    };
    assert!(matches!(handling, FallibleExpressionHandling::Recover));
    let FallibleHandling::Handler {
        error: Some(_),
        body,
    } = &value_catch.handler
    else {
        panic!("expected catch-handler call handling");
    };

    assert_eq!(body.len(), 2);
}

#[test]
fn parses_standalone_result_propagation_statement() {
    let (ast, string_table) = parse_single_file_ast(
        "can_error || -> Error!:\n    return\n;\n\nrun || -> Error!:\n    can_error()!\n;\n",
    );

    let body = function_body_by_name(&ast, &string_table, "run");
    let NodeKind::ExpressionStatement(expression) = &body[0].kind else {
        panic!("expected standalone handled call to parse as an expression statement");
    };
    assert!(matches!(
        expression.kind,
        ExpressionKind::HandledFallibleFunctionCall {
            handling: FallibleExpressionHandling::Propagate,
            ..
        }
    ));
}

// --------------------------
//  Trailing comma rejections
// --------------------------

#[test]
fn rejects_trailing_comma_in_single_return() {
    let payload = parse_function_diagnostic_payload("compute |x Int| -> Int,:\n    return 42\n;\n");

    assert_eq!(
        payload,
        DiagnosticPayload::InvalidFunctionSignature {
            reason: InvalidFunctionSignatureReason::TrailingCommaInReturns
        }
    );
}

#[test]
fn rejects_trailing_comma_in_error_return() {
    let payload =
        parse_function_diagnostic_payload("compute |x Int| -> Int, Error!,:\n    return 42\n;\n");

    assert_eq!(
        payload,
        DiagnosticPayload::InvalidFunctionSignature {
            reason: InvalidFunctionSignatureReason::TrailingCommaInReturns
        }
    );
}

// --------------------------
//  Valid parameter and return parsing
// --------------------------

#[test]
fn parses_valid_single_return_without_trailing_comma() {
    let (ast, string_table) = parse_single_file_ast("compute |x Int| -> Int:\n    return 42\n;\n");

    let signature = function_signature_by_name(&ast, &string_table, "compute");
    assert_eq!(signature.returns.len(), 1);
    assert_eq!(signature.returns[0].value, DataType::Int);
    assert_eq!(signature.returns[0].channel, ReturnChannel::Success);
    assert!(
        signature.returns[0].type_id.is_some(),
        "return type_id should be resolved"
    );
}

#[test]
fn parses_valid_multiple_returns_without_trailing_comma() {
    let (ast, string_table) =
        parse_single_file_ast("compute |x Int| -> Int, String:\n    return 42, \"result\"\n;\n");

    let signature = function_signature_by_name(&ast, &string_table, "compute");
    assert_eq!(signature.returns.len(), 2);
    assert_eq!(signature.returns[0].value, DataType::Int);
    assert_eq!(signature.returns[0].channel, ReturnChannel::Success);
    assert!(
        signature.returns[0].type_id.is_some(),
        "return type_id should be resolved"
    );
    assert_eq!(signature.returns[1].value, DataType::StringSlice);
    assert_eq!(signature.returns[1].channel, ReturnChannel::Success);
    assert!(
        signature.returns[1].type_id.is_some(),
        "return type_id should be resolved"
    );
}

#[test]
fn parses_valid_single_parameter_without_trailing_comma() {
    let (ast, string_table) = parse_single_file_ast("compute |x Int| -> Int:\n    return x\n;\n");

    let signature = function_signature_by_name(&ast, &string_table, "compute");
    assert_eq!(signature.parameters.len(), 1);
    assert_eq!(
        signature.parameters[0].id.name_str(&string_table),
        Some("x")
    );
}

#[test]
fn parses_valid_multiple_parameters_without_trailing_comma() {
    let (ast, string_table) =
        parse_single_file_ast("compute |x Int, y Int| -> Int:\n    return x + y\n;\n");

    let signature = function_signature_by_name(&ast, &string_table, "compute");
    assert_eq!(signature.parameters.len(), 2);
    assert_eq!(
        signature.parameters[0].id.name_str(&string_table),
        Some("x")
    );
    assert_eq!(
        signature.parameters[1].id.name_str(&string_table),
        Some("y")
    );
}
