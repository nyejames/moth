//! Function-signature and call parsing regression tests.
//!
//! WHAT: validates the AST shapes produced for function declarations and call sites.
//! WHY: statement parsing should preserve signature metadata and host/user call dispatch.

use crate::compiler_frontend::ast::ast_nodes::{
    MatchExhaustiveness, MultiBindTargetKind, NodeKind,
};
use crate::compiler_frontend::ast::const_values::store::{
    ConstValueId, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExpressionOperationView, ResolvedCallableTarget,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::ast::statements::functions::ReturnChannel;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    calls_data::InvalidValueReceiverView, types_borrows_targets::TypeMismatchView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::DurableLabelMessage;
use crate::compiler_frontend::compiler_messages::{TypeMismatchContext, ValueReceiverContext};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, function_signature_by_name, start_function_body,
};
use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, ast_error_label_spans, ast_error_primary_span,
    ast_first_error_rendered_message, first_ast_error_view, parse_single_file_ast,
    parse_single_file_ast_build_result, parse_single_file_ast_diagnostic,
};
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

fn assert_optional_string_default(
    constants: &ConstValueStore,
    default: ConstValueId,
    optional_type: crate::compiler_frontend::datatypes::ids::TypeId,
    string_type: crate::compiler_frontend::datatypes::ids::TypeId,
) {
    assert_eq!(
        constants
            .metadata(default)
            .expect("default metadata")
            .type_id,
        optional_type
    );
    let Some(ConstValuePayload::OptionSome(inner)) = constants.payload(default) else {
        panic!("present String default must retain its optional wrapping");
    };
    assert_eq!(
        constants.metadata(*inner).expect("inner metadata").type_id,
        string_type
    );
    assert!(matches!(
        constants.payload(*inner),
        Some(ConstValuePayload::String(_))
    ));
}

fn assert_function_diagnostic(source: &str, code: &str, reason_key: Option<&str>) {
    let failure = parse_single_file_ast_diagnostic(source);
    assert_ast_diagnostic(&failure, code, reason_key);
}

fn assert_type_mismatch_context(source: &str, expected_context: TypeMismatchContext) {
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-TYPE-0001", None);
    let facts = diagnostic
        .family_view::<TypeMismatchView>()
        .expect("the diagnostic is a type mismatch");
    assert_eq!(facts.context, expected_context);
}

fn assert_invalid_receiver_call(source: &str, expected_reason_key: &'static str) {
    assert_function_diagnostic(source, "MOTH-RULE-0047", Some(expected_reason_key));
}

// --------------------------
//  Signature parsing
// --------------------------

#[test]
fn generic_declarations_in_executable_bodies_require_module_top_level() {
    for declaration in [
        "nested type A |value A|:\n        return\n    ;",
        "Nested type A = |value A|",
        "Nested type A :: Item |value A|;",
        "Nested type A as A",
    ] {
        let source = format!("outer ||:\n    {declaration}\n;\n");
        let failure = parse_single_file_ast_diagnostic(&source);
        assert_ast_diagnostic(
            &failure,
            "MOTH-RULE-0043",
            Some("invalid_declaration.declaration_requires_module_top_level"),
        );
    }
}

#[test]
fn function_default_path_token_reports_structured_diagnostic_not_panic() {
    // A path token nested in a template default lives in the detached parameter
    // default buffer; the header must own its path row so AST parsing reports the
    // missing target as a structured diagnostic instead of indexing a missing row.
    assert_function_diagnostic(
        "label |prefix String = [: [@docs/intro.md] ]| -> String:\n    return prefix\n;\n",
        "MOTH-RULE-0063",
        Some("invalid_compile_time_path.missing_target"),
    );
}

#[test]
fn parameters_cannot_shadow_bound_external_symbols_or_namespaces() {
    for (source, name) in [
        ("read |io I32|:\n;\n", "io"),
        ("read |io ~I32|:\n;\n", "io"),
        ("read type T |io T|:\n;\n", "io"),
        ("Point = |value I32|\nread |this Point, io I32|:\n;\n", "io"),
        (
            "@core/io line as host_line\nread |host_line I32|:\n;\n",
            "host_line",
        ),
        ("@core/io as output\nread |output I32|:\n;\n", "output"),
    ] {
        let failure = parse_single_file_ast_diagnostic(source);
        let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0038", None);
        let start = source.rfind(name).expect("the parameter name must occur") as u32;
        let mut builder = ExtendedSpanBuilder::new();
        let span = LocalSpan::exact(start, name.len() as u32, &mut builder)
            .expect("the name span must fit");
        assert_eq!(
            ast_error_primary_span(diagnostic),
            Some(SourceSpan::new(SourceId::COMPILATION_ROOT, span)),
            "{source}",
        );
        assert!(ast_error_label_spans(diagnostic).is_empty(), "{source}");
    }
}

#[test]
fn parameters_cannot_shadow_visible_source_declarations() {
    for (source, name) in [
        ("read |read I32|:\n;\n", "read"),
        ("top_value #= 1\nread |top_value I32|:\n;\n", "top_value"),
        ("available ||:\n;\nread |available I32|:\n;\n", "available"),
        (
            "top_value #= 1\nPoint = |value I32|\nread |this Point, top_value I32|:\n;\n",
            "top_value",
        ),
    ] {
        let failure = parse_single_file_ast_diagnostic(source);
        let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0038", None);
        let start = source.rfind(name).expect("the parameter name must occur") as u32;
        let mut builder = ExtendedSpanBuilder::new();
        let span = LocalSpan::exact(start, name.len() as u32, &mut builder)
            .expect("the name span must fit");
        assert_eq!(
            ast_error_primary_span(diagnostic),
            Some(SourceSpan::new(SourceId::COMPILATION_ROOT, span)),
            "{source}",
        );
    }
}

#[test]
fn ordinary_mutable_parameters_keep_single_and_multi_bind_updates() {
    let (ast, paths, strings) = parse_single_file_ast(
        "pair || -> I32, I32:\n    return 1, 2\n;\n\
         rebind |first ~I32, second ~I32| -> I32, I32:\n\
             ~first = 3\n\
             ~first, ~second = pair()\n\
             return first, second\n;\n",
    );
    let body = function_body_by_name(&ast, &paths, &strings, "rebind");
    assert!(matches!(body[0].kind, NodeKind::Assignment { .. }));
    let NodeKind::MultiBind { targets, .. } = &body[1].kind else {
        panic!("the existing parameter updates must remain a multi-bind");
    };
    assert_eq!(targets.len(), 2);
    for (target, name) in targets.iter().zip(["first", "second"]) {
        assert!(matches!(target.kind, MultiBindTargetKind::Assignment));
        assert_eq!(target.value_mode, ValueMode::MutableOwned);
        assert_eq!(
            target.type_id,
            builtin_type_ids::fixed_scalar(FixedScalar::I32)
        );
        assert_eq!(
            strings.resolve(paths.component(target.id).expect("the target has a name")),
            name,
        );
    }
    assert!(matches!(body[2].kind, NodeKind::Return(_)));
    assert_eq!(body.len(), 3);
}

#[test]
fn multi_bind_prefix_selects_update_independently_of_declaration() {
    let (ast, paths, strings) = parse_single_file_ast(
        "pair || -> I32, I32:\n    return 1, 2\n;\n\
         rebind |first ~I32| -> I32, I32:\n\
             ~first I32, second ~I32 = pair()\n\
             return first, second\n;\n",
    );
    let body = function_body_by_name(&ast, &paths, &strings, "rebind");
    let NodeKind::MultiBind { targets, .. } = &body[0].kind else {
        panic!("expected mixed multi-bind");
    };
    assert!(matches!(targets[0].kind, MultiBindTargetKind::Assignment));
    assert!(matches!(targets[1].kind, MultiBindTargetKind::Declaration));
    assert_eq!(targets[1].value_mode, ValueMode::MutableOwned);
}

#[test]
fn multi_bind_prefix_diagnostics_identify_the_authored_name() {
    for (targets, parameter, reason, name) in [
        (
            "first, second",
            "first ~I32",
            "invalid_multi_bind.existing_target_needs_prefix",
            "first",
        ),
        (
            "first, second",
            "first I32",
            "invalid_multi_bind.existing_target_immutable",
            "first",
        ),
        (
            "~missing, second",
            "first ~I32",
            "invalid_multi_bind.unknown_prefixed_target",
            "missing",
        ),
    ] {
        let source = format!(
            "pair || -> I32, I32:\n    return 1, 2\n;\nrebind |{parameter}|:\n    {targets} = pair()\n;\n"
        );
        let failure = parse_single_file_ast_diagnostic(&source);
        let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0045", Some(reason));
        let start = source.find(targets).expect("target list is present")
            + targets.find(name).expect("target name is present");
        let mut builder = ExtendedSpanBuilder::new();
        let span = LocalSpan::exact(start as u32, name.len() as u32, &mut builder)
            .expect("target name span fits");
        assert_eq!(
            ast_error_primary_span(diagnostic),
            Some(SourceSpan::new(SourceId::COMPILATION_ROOT, span)),
            "{source}",
        );
    }
}

#[test]
fn multi_bind_immutable_update_precedes_producer_errors_and_labels_declaration() {
    for (declaration, prefix) in [
        ("rebind |first I32|:\n", "~"),
        ("rebind |first I32|:\n", ""),
        ("rebind ||:\n    first = 0\n", ""),
        ("rebind ||:\n    first = 0\n", "~"),
    ] {
        let source = format!("{declaration}    {prefix}first, second = missing()\n;\n");
        let failure = parse_single_file_ast_diagnostic(&source);
        let diagnostic = assert_ast_diagnostic(
            &failure,
            "MOTH-RULE-0045",
            Some("invalid_multi_bind.existing_target_immutable"),
        );
        assert_eq!(
            ast_first_error_rendered_message(&failure),
            "Existing multi-bind target 'first' is immutable. Declare it mutable to update it.",
        );
        let mut spans = ExtendedSpanBuilder::new();
        let declaration =
            LocalSpan::exact(source.find("first").unwrap() as u32, 5, &mut spans).unwrap();
        let target =
            LocalSpan::exact(source.rfind("first").unwrap() as u32, 5, &mut spans).unwrap();
        assert_eq!(
            ast_error_primary_span(diagnostic),
            Some(SourceSpan::new(SourceId::COMPILATION_ROOT, target)),
        );
        assert_eq!(
            ast_error_label_spans(diagnostic),
            vec![Some(SourceSpan::new(
                SourceId::COMPILATION_ROOT,
                declaration
            ))],
        );
    }
}

#[test]
fn multi_bind_pending_mutability_and_malformed_declaration_precedence() {
    for (declaration, target, reason) in [
        (
            "first ~= 0",
            "first",
            "invalid_multi_bind.existing_target_needs_prefix",
        ),
        (
            "first = 0",
            "first ~I32",
            "invalid_multi_bind.existing_target_mutable_marker",
        ),
        (
            "first I32 = 0",
            "first ~I32",
            "invalid_multi_bind.existing_target_mutable_marker",
        ),
    ] {
        let source =
            format!("rebind ||:\n    {declaration}\n    {target}, second = missing()\n;\n");
        let failure = parse_single_file_ast_diagnostic(&source);
        assert_ast_diagnostic(&failure, "MOTH-RULE-0045", Some(reason));
    }
}

#[test]
fn parses_function_parameters_and_return_types() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("add |left I32, right I32| -> I32:\n    return left + right\n;\n");

    let signature = function_signature_by_name(&ast, &path_fork, &string_table, "add");

    assert_eq!(signature.parameters.len(), 2);
    assert_eq!(
        path_fork
            .component(signature.parameters[0].id)
            .map(|id| string_table.resolve(id)),
        Some("left")
    );
    assert_eq!(
        path_fork
            .component(signature.parameters[1].id)
            .map(|id| string_table.resolve(id)),
        Some("right")
    );
    assert_eq!(signature.returns.len(), 1);
    assert_eq!(
        signature.returns[0].type_id,
        Some(builtin_type_ids::fixed_scalar(FixedScalar::I32))
    );
    assert_eq!(signature.returns[0].channel, ReturnChannel::Success);
}

#[test]
fn parses_final_error_return_slot_in_function_signature() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("compute |x I32| -> I32, Error!:\n    return x\n;\n");

    let signature = function_signature_by_name(&ast, &path_fork, &string_table, "compute");

    assert_eq!(signature.returns.len(), 2);
    assert_eq!(
        signature.returns[0].type_id,
        Some(builtin_type_ids::fixed_scalar(FixedScalar::I32))
    );
    assert_eq!(signature.returns[0].channel, ReturnChannel::Success);
    assert!(
        matches!(
            ast.type_environment.get(
                signature.returns[1]
                    .type_id
                    .expect("Error return type should be resolved")
            ),
            Some(TypeDefinition::Struct(_))
        ),
        "builtin Error should resolve to a struct type"
    );
    assert_eq!(signature.returns[1].channel, ReturnChannel::Error);
}

#[test]
fn parses_optional_final_error_return_slot() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("compute |x I32| -> I32, String?!:\n    return x\n;\n");

    let signature = function_signature_by_name(&ast, &path_fork, &string_table, "compute");

    assert_eq!(signature.returns.len(), 2);
    assert_eq!(
        ast.type_environment.option_inner_type(
            signature.returns[1]
                .type_id
                .expect("optional error return should be resolved")
        ),
        Some(ast.type_environment.builtins().string)
    );
    assert_eq!(signature.returns[1].channel, ReturnChannel::Error);
}

#[test]
fn rejects_non_final_error_return_slot() {
    assert_function_diagnostic(
        "compute |x I32| -> Error!, I32:\n    return 42\n;\n",
        "MOTH-RULE-0062",
        Some("invalid_function_signature.error_slot_not_last"),
    );
}

#[test]
fn rejects_multiple_error_return_slots() {
    assert_function_diagnostic(
        "compute |x I32| -> I32, String!, Error!:\n    return 42\n;\n",
        "MOTH-RULE-0062",
        Some("invalid_function_signature.multiple_error_return_slots"),
    );
}

#[test]
fn parses_generic_function_declaration_without_emitting_executable_function() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "identity type T |value T| -> T:\n    return value\n;\n\nio.line([: [\"ready\"]])\n",
    );

    let generic_function_emitted = ast.nodes.iter().any(|node| match &node.kind {
        NodeKind::Function(path, ..) => {
            path_fork
                .component(*path)
                .map(|id| string_table.resolve(id))
                == Some("identity")
        }
        _ => false,
    });

    assert!(
        !generic_function_emitted,
        "generic function declarations should be retained as AST templates until instantiated"
    );
}

#[test]
fn same_file_generic_calls_emit_requests_without_eager_function_bodies() {
    let source = "identity type T |value T| -> T:\n    return value\n;\n\nvalue = identity(1)\n";
    let (build_result, path_fork, string_table) = parse_single_file_ast_build_result(source)
        .expect("source should parse into a request-only AST");
    let ast = build_result.ast;

    let generic_template_emitted = ast.nodes.iter().any(|node| match &node.kind {
        NodeKind::Function(path, ..) => {
            path_fork
                .component(*path)
                .map(|id| string_table.resolve(id))
                == Some("identity")
        }
        _ => false,
    });
    assert!(
        !generic_template_emitted,
        "generic function templates should not emit under their source name"
    );

    let start_body = start_function_body(&ast, &path_fork, &string_table);
    let NodeKind::VariableDeclaration(declaration) = &start_body[0].kind else {
        panic!("expected start body to store the generic call result")
    };
    let ExpressionOperationView::Call(call) = ast
        .expressions
        .view(
            declaration
                .value
                .initializer()
                .expect("fixture initializer"),
        )
        .expect("bound initializer")
        .operation()
    else {
        panic!("expected variable initializer to call the concrete generic instance")
    };
    let ResolvedCallableTarget::Function(name) = call.metadata().target else {
        panic!("expected a user function target");
    };

    assert!(
        !ast.nodes
            .iter()
            .any(|node| matches!(&node.kind, NodeKind::Function(path, ..) if *path == name)),
        "generic calls must not materialise concrete bodies inside the requester AST"
    );
    let [request] = build_result.deferred_generic_requests.as_slice() else {
        panic!("one generic call should emit one deferred request")
    };
    assert_eq!(name, request.instance_path);
    assert_eq!(
        request.key.type_arguments.as_ref(),
        ast.expressions
            .view(
                declaration
                    .value
                    .initializer()
                    .expect("fixture initializer")
            )
            .expect("bound initializer")
            .success_types(),
    );
    let call_start = source
        .find("identity(1)")
        .expect("the generic call should be present") as u32;
    let mut span_builder = ExtendedSpanBuilder::new();
    let call_span = LocalSpan::exact(call_start, "identity".len() as u32, &mut span_builder)
        .expect("the generic callee should fit an inline span");
    assert_eq!(
        request.call_span,
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, call_span))
    );
    assert_eq!(
        declaration.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
}

#[test]
fn generic_receiver_diagnostic_retains_exact_multibyte_member_span() {
    let source = "prefix String = \"é\"\n\
DISPLAY_TEXT must:\n\
    display |This| -> String\n\
;\n\
Plain = |\n\
    text String,\n\
|\n\
display |this Plain| -> String:\n\
    return this.text\n\
;\n\
Box type A = |\n\
    value A,\n\
|\n\
show type A is DISPLAY_TEXT |this Box of A| -> String:\n\
    return this.value.display()\n\
;\n\
box = Box(Plain(\"no evidence\"))\n\
value = box.show()\n";

    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0057",
        Some("invalid_generic_instantiation.missing_trait_evidence"),
    );
    let member_start = source
        .find("box.show()")
        .expect("the generic receiver call should be present") as u32
        + "box.".len() as u32;
    let mut span_builder = ExtendedSpanBuilder::new();
    let member_span = LocalSpan::exact(member_start, "show".len() as u32, &mut span_builder)
        .expect("the generic receiver member should fit an inline span");

    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, member_span))
    );
}

#[test]
fn nonconstant_function_reference_in_parameter_default_keeps_member_label() {
    let source = "runtime || -> Bool:\n\
    return true\n\
;\n\
consume |enabled Bool = runtime()|:\n\
    return false\n\
;\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0053",
        Some("compile_time_evaluation_error.non_constant_reference_in_constant"),
    );

    let member_start = source
        .find("enabled")
        .expect("the function parameter should be present") as u32;
    let mut span_builder = ExtendedSpanBuilder::new();
    let member_span = SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::exact(member_start, "enabled".len() as u32, &mut span_builder)
            .expect("the parameter name should fit an inline span"),
    );
    let labels = diagnostic.durable_labels().collect::<Vec<_>>();
    assert_eq!(labels.len(), 1);
    assert!(matches!(labels[0].message, DurableLabelMessage::None));
    assert!(matches!(
        labels[0].site,
        SiteRef::Local(span) if span == member_span
    ));
}

#[test]
fn fallible_generic_calls_remain_request_only() {
    let (build_result, _path_fork, _string_table) = parse_single_file_ast_build_result(
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
fn static_true_assertion_retains_validation_only_generic_message_request() {
    let (build_result, path_fork, string_table) = parse_single_file_ast_build_result(
        "identity type T |value T| -> T:\n    return value\n;\n\nassert(true, identity(\"inactive\"))\n",
    )
    .expect("inactive generic assertion message should still be frontend-valid");

    assert!(
        build_result.deferred_generic_requests.is_empty(),
        "static-true assertion messages must not publish executable generic requests"
    );
    let [request] = build_result
        .assertion_validation_generic_requests
        .as_slice()
    else {
        panic!("the surviving concrete assertion must retain its generic validation request");
    };
    assert_eq!(
        request.key.type_arguments.as_ref(),
        &[build_result.ast.type_environment.builtins().string],
    );
    assert!(
        !build_result.ast.nodes.iter().any(
            |node| matches!(&node.kind, NodeKind::Function(path, ..) if *path == request.instance_path)
        ),
        "validation-only candidates must not eagerly materialise executable bodies",
    );
    let start_body = start_function_body(&build_result.ast, &path_fork, &string_table);
    assert!(
        start_body
            .iter()
            .any(|node| matches!(node.kind, NodeKind::Assert { .. })),
        "the assertion itself remains in the executable AST contract"
    );
}

#[test]
fn static_true_assertion_completes_pending_validation_ticket_without_dropping_sibling() {
    for condition in ["true", "1 + 2 is 3"] {
        let source = format!(
            "\
describe type T |value T| -> String:
    return \"inactive\"
;
identity type T |value T| -> T:
    return value
;

value = 1
assert({condition}, describe(value))
active = identity(value)
"
        );
        let (build_result, path_fork, string_table) = parse_single_file_ast_build_result(&source)
            .expect("retained assertion and sibling generic calls should both complete");

        let [validation] = build_result
            .assertion_validation_generic_requests
            .as_slice()
        else {
            panic!("condition {condition}: the message must complete its own validation ticket");
        };
        let [active] = build_result.deferred_generic_requests.as_slice() else {
            panic!("condition {condition}: the sibling's executable request must survive");
        };
        assert_eq!(
            path_fork
                .component(validation.key.function_path)
                .map(|component| string_table.resolve(component)),
            Some("describe"),
            "condition {condition}",
        );
        assert_eq!(
            path_fork
                .component(active.key.function_path)
                .map(|component| string_table.resolve(component)),
            Some("identity"),
            "condition {condition}",
        );
        let int = builtin_type_ids::fixed_scalar(FixedScalar::I32);
        assert_eq!(validation.key.type_arguments.as_ref(), &[int]);
        assert_eq!(active.key.type_arguments.as_ref(), &[int]);
    }
}

#[test]
fn numeric_true_condition_reclassifies_original_concrete_message_request_only() {
    let source = "\
identity type T |value T| -> T:
    return value
;
check ||:
    assert(1 + 2 is 3, identity(\"message\"))
;

active = identity(4)
";
    let (build_result, _path_fork, _string_table) = parse_single_file_ast_build_result(source)
        .expect("the numeric condition should finish true after the message request was recorded");

    let [validation] = build_result
        .assertion_validation_generic_requests
        .as_slice()
    else {
        panic!("the original concrete message request must become validation-only");
    };
    let [active] = build_result.deferred_generic_requests.as_slice() else {
        panic!("the later active call must keep its executable request");
    };
    assert_eq!(
        validation.key.type_arguments.as_ref(),
        &[build_result.ast.type_environment.builtins().string],
    );
    assert_eq!(
        active.key.type_arguments.as_ref(),
        &[builtin_type_ids::fixed_scalar(FixedScalar::I32)],
    );
}

#[test]
fn executable_generic_request_takes_precedence_over_pending_assertion_validation() {
    let source = "\
describe type T |value T| -> String:
    return \"message\"
;

value = 1
assert(true, describe(value))
active = describe(value)
";
    let (build_result, _path_fork, _string_table) = parse_single_file_ast_build_result(source)
        .expect("duplicate validation and executable calls should both complete");

    assert_eq!(build_result.deferred_generic_requests.len(), 1);
    assert!(
        build_result
            .assertion_validation_generic_requests
            .is_empty(),
        "an executable request already supplies the canonical validation dependency",
    );
}

#[test]
fn inactive_static_if_drops_known_true_assertion_validation_requests() {
    let source = "\
describe type T |value T| -> String:
    return \"inactive\"
;

value = 1
if false:
    assert(true, describe(value))
;
";
    let (build_result, _path_fork, _string_table) = parse_single_file_ast_build_result(source)
        .expect("inactive assertions should still finish numeric and generic validation");

    assert!(build_result.deferred_generic_requests.is_empty());
    assert!(
        build_result
            .assertion_validation_generic_requests
            .is_empty(),
        "an assertion removed with its static branch has no surviving validation witness",
    );
}

#[test]
fn abstract_generic_body_does_not_publish_assertion_validation_requests() {
    let source = "\
describe type T |value T| -> String:
    return \"inactive\"
;
unused type T |value T|:
    assert(true, describe(value))
;
";
    let (build_result, _path_fork, _string_table) = parse_single_file_ast_build_result(source)
        .expect("abstract generic bodies should validate without activating requests");

    assert!(build_result.deferred_generic_requests.is_empty());
    assert!(
        build_result
            .assertion_validation_generic_requests
            .is_empty(),
        "abstract template validation is not a surviving concrete assertion witness",
    );
}

#[test]
fn static_true_assertion_still_reports_invalid_generic_message() {
    assert_type_mismatch_context(
        "identity type T |value T| -> T:\n    return value\n;\n\nassert(true, identity(1))\n",
        TypeMismatchContext::AssertionArgument,
    );
}
#[test]
fn static_false_assertion_retains_active_generic_message_requests() {
    let (build_result, _path_fork, _string_table) = parse_single_file_ast_build_result(
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
    let (build_result, _path_fork, _string_table) = parse_single_file_ast_build_result(
        "condition || -> Bool:\n    return true\n;\n\nidentity type T |value T| -> T:\n    return value\n;\n\nassert(condition(), identity(\"dynamic\"))\n",
    )
    .expect("dynamic assertion message should remain frontend-valid");

    assert_eq!(
        build_result.deferred_generic_requests.len(),
        1,
        "dynamic assertion messages must retain their required generic request"
    );
}

#[test]
fn static_if_discards_inactive_generic_requests() {
    let (build_result, _path_fork, _string_table) = parse_single_file_ast_build_result(
        "enabled #= true\nidentity type T |value T| -> T:\n    return value\n;\n\nactive ~= 0\ninactive ~= \"\"\nif enabled:\n    ~active = 1\nelse\n    ~inactive = identity(\"inactive\")\n;\n",
    )
    .expect("both static-if branches should remain frontend-valid");

    assert!(
        build_result.deferred_generic_requests.is_empty(),
        "the inactive branch must not publish a generated-function request"
    );
}

#[test]
fn static_if_keeps_active_duplicate_after_inactive_first_request() {
    let (build_result, _path_fork, _string_table) = parse_single_file_ast_build_result(
        "enabled #= false\nidentity type T |value T| -> T:\n    return value\n;\n\nvalue ~= 0\nif enabled:\n    ~value = identity(1)\nelse\n    ~value = identity(2)\n;\n",
    )
    .expect("both duplicate generic calls should remain frontend-valid");

    assert_eq!(
        build_result.deferred_generic_requests.len(),
        1,
        "deduplication must run after inactive request ranges are removed"
    );
}

#[test]
fn generic_template_terminality_uses_static_module_constant_selection() {
    let (build_result, _path_fork, _string_table) = parse_single_file_ast_build_result(
        "enabled #= true\nchoose type T |value T| -> T:\n    if enabled:\n        return value\n    ;\n;\n\nselected = choose(7)\n",
    )
    .expect("generic template terminality should consume its specialised validated body");

    assert_eq!(build_result.deferred_generic_requests.len(), 1);
}

// --------------------------
//  Call dispatch
// --------------------------

#[test]
fn start_function_distinguishes_user_and_host_calls() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "identity |value I32| -> I32:\n    return value\n;\n\nresult = identity(1)\nio.line([: [result]])\n",
    );

    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(result_decl) = &body[0].kind else {
        panic!("expected variable declaration for function result");
    };
    assert!(matches!(
        ast.expressions.view(result_decl.value.initializer().expect("fixture initializer"))
            .expect("bound initializer").operation(),
        ExpressionOperationView::Call(call) if matches!(call.metadata().target, ResolvedCallableTarget::Function(_))
    ));

    assert!(
        body.iter().any(|node| match &node.kind {
            NodeKind::ExpressionStatement(expression) => {
                matches!(ast.expressions.view(*expression).expect("bound expression").operation(),
                    ExpressionOperationView::Call(call) if matches!(call.metadata().target, ResolvedCallableTarget::Host(_)))
            }
            _ => false,
        }),
        "expected io.line([: [...]]) to parse as a host-function expression statement"
    );

    let function_body = function_body_by_name(&ast, &path_fork, &string_table, "identity");
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
    assert_function_diagnostic(
        "touch |items ~{I32}| -> I32:\n    return items.length()\n;\n\nvalues = {1, 2, 3}\ncount = touch(values)\n",
        "MOTH-RULE-0054",
        Some("invalid_call_shape.immutable_place_mutable_access_required"),
    );
}

#[test]
fn rejects_collection_element_type_mismatch_with_mutability_relaxation() {
    assert_type_mismatch_context(
        "sum |items {I32}| -> I32:\n    return items.length()\n;\n\nvalues ~= {true, false}\ncount = sum(values)\n",
        TypeMismatchContext::FunctionArgument,
    );
}

#[test]
fn rejects_struct_constructor_argument_type_with_field_wording() {
    assert_type_mismatch_context(
        "Point = |\n    x I32,\n    y I32,\n|\n\npoint = Point(x = 1, y = \"oops\")\n",
        TypeMismatchContext::ConstructorArgument,
    );
}
// --------------------------
//  Named types in signatures
// --------------------------

#[test]
fn resolves_named_struct_type_in_function_parameters() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "Point = |\n    x I32,\n|\n\nshow |value Point|:\n    io.line([: [value.x]])\n;\n",
    );

    let signature = function_signature_by_name(&ast, &path_fork, &string_table, "show");
    assert!(matches!(
        ast.type_environment.get(signature.parameters[0].type_id),
        Some(TypeDefinition::Struct(definition)) if !definition.const_record
    ));
}

#[test]
fn resolves_named_struct_type_in_function_returns() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "Point = |\n    x I32,\n|\n\nclone |value Point| -> Point:\n    return value\n;\n",
    );

    let signature = function_signature_by_name(&ast, &path_fork, &string_table, "clone");
    assert_eq!(
        signature.returns[0].type_id,
        Some(signature.parameters[0].type_id),
        "the named return should resolve to the parameter's semantic type"
    );
    assert!(matches!(
        ast.type_environment.get(signature.parameters[0].type_id),
        Some(TypeDefinition::Struct(definition)) if !definition.const_record
    ));
}

#[test]
fn rejects_unknown_named_type_in_function_signatures() {
    assert_function_diagnostic(
        "use_missing |value Missing|:\n    return value\n;\n",
        "MOTH-RULE-0035",
        None,
    );
}

#[test]
fn rejects_unknown_named_return_type_in_function_signatures() {
    assert_function_diagnostic(
        "clone |value I32| -> Missing:\n    return value\n;\n",
        "MOTH-RULE-0035",
        None,
    );
}

// --------------------------
//  Receiver parameter rules
// --------------------------

#[test]
fn rejects_receiver_parameter_when_not_first() {
    assert_function_diagnostic(
        "Point = |\n    x I32,\n|\n\nreset |value I32, this Point|:\n    return value\n;\n",
        "MOTH-RULE-0040",
        Some("invalid_this_usage.not_first_parameter"),
    );
}

#[test]
fn rejects_builtin_scalar_receiver_parameter() {
    assert_function_diagnostic(
        "reset |this I32| -> I32:\n    return this\n;\n",
        "MOTH-RULE-0041",
        Some("invalid_receiver_declaration.builtin_scalar_type"),
    );
}

#[test]
fn rejects_multiple_this_parameters_in_one_signature() {
    assert_function_diagnostic(
        "Point = |\n    x I32 = 0,\n|\n\nlength |this Point, this Point| -> I32:\n    return this.x\n;\n",
        "MOTH-RULE-0040",
        Some("invalid_this_usage.duplicate_this"),
    );
}

// --------------------------
//  Method call rules
// --------------------------

#[test]
fn rejects_field_method_name_collisions() {
    assert_function_diagnostic(
        "Point = |\n    reset I32 = 0,\n|\n\nreset |this Point| -> I32:\n    return this.reset\n;\n",
        "MOTH-RULE-0041",
        Some("invalid_receiver_declaration.field_name_conflict"),
    );
}
#[test]
fn rejects_free_function_call_syntax_for_receiver_methods() {
    assert_invalid_receiver_call(
        "Point = |\n    x I32 = 0,\n|\n\nreset |this Point|:\n    return\n;\n\npoint ~= Point()\nreset(point)\n",
        "invalid_receiver_call.called_as_free_function",
    );
}

#[test]
fn rejects_const_record_method_calls() {
    assert_invalid_receiver_call(
        "Point = |\n    x I32 = 0,\n|\n\nlength |this Point| -> I32:\n    return this.x\n;\n\norigin #= Point()\norigin.length()\n",
        "invalid_receiver_call.const_record_no_runtime_calls",
    );
}

#[test]
fn parses_mutable_receiver_methods_with_explicit_receiver_tilde() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "Point = |\n    x I32 = 0,\n|\n\nreset |this ~Point|:\n    ~this.x = 0\n;\n\npoint ~= Point()\n~point.reset()\n",
    );

    let body = start_function_body(&ast, &path_fork, &string_table);
    let NodeKind::ExpressionStatement(call_expr) = &body[1].kind else {
        panic!("expected receiver method statement");
    };
    assert!(
        matches!(
            ast.expressions
                .view(*call_expr)
                .expect("bound call")
                .operation(),
            ExpressionOperationView::MethodCall { .. }
        ),
        "expected user-defined receiver method call expression"
    );
}

#[test]
fn mutable_receiver_can_write_fields_but_cannot_replace_this() {
    let source = "Point = |\n    x I32 = 0,\n|\nreset |this ~Point|:\n    ~this = Point()\n;\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0040",
        Some("invalid_this_usage.reassignment"),
    );
    let mut spans = ExtendedSpanBuilder::new();
    let span = LocalSpan::exact(source.rfind("this").unwrap() as u32, 4, &mut spans).unwrap();
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, span))
    );
}

#[test]
fn prefixed_write_keeps_value_if_receiving_context() {
    let (ast, paths, strings) = parse_single_file_ast(
        "choose |flag Bool| -> I32:\n    value ~I32 = 0\n    ~value = if flag then 1 else 2\n    return value\n;\n",
    );
    let body = function_body_by_name(&ast, &paths, &strings, "choose");
    assert!(matches!(&body[1].kind, NodeKind::Assignment { .. }));
}

#[test]
fn prefixed_write_preserves_catch_target_unavailability() {
    assert_function_diagnostic(
        "can_error || -> I32, Error!:\n    return 1\n;\nrecover || -> I32:\n    value ~= 0\n    ~value = can_error() catch:\n        ~value = 2\n        then 0\n    ;\n    return value\n;\n",
        "MOTH-RULE-0044",
        Some("invalid_assignment_target.unavailable_in_catch_recovery"),
    );
}

#[test]
fn parses_result_propagation_for_receiver_method_call() {
    parse_single_file_ast(
        "Point = |\n    x I32 = 1,\n|\n\nread |this Point| -> I32, Error!:\n    return this.x\n;\n\nforward |point Point| -> I32, Error!:\n    return point.read()!\n;\n",
    );
}

#[test]
fn rejects_mutable_receiver_methods_on_immutable_bindings_without_marker() {
    assert_invalid_receiver_call(
        "Point = |\n    x I32 = 0,\n|\n\nreset |this ~Point|:\n    ~this.x = 0\n;\n\npoint = Point()\npoint.reset()\n",
        "invalid_receiver_call.immutable_receiver_mutable_method",
    );
}

#[test]
fn rejects_explicit_receiver_tilde_for_shared_receiver_methods() {
    assert_invalid_receiver_call(
        "Point = |\n    x I32 = 0,\n|\n\nlength |this Point| -> I32:\n    return this.x\n;\n\npoint ~= Point()\nvalue = ~point.length()\n",
        "invalid_receiver_call.unneeded_mutable_access_marker",
    );
}

// --------------------------
//  Fallible handling
// --------------------------

#[test]
fn parses_result_propagation_call_in_expression_position() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "can_error |value String| -> String, Error!:\n    return value\n;\n\nforward |value String| -> String, Error!:\n    return can_error(value)!\n;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "forward");
    let NodeKind::Return(values) = &body[0].kind else {
        panic!("expected return statement in forward()");
    };

    assert_eq!(values.len(), 1);
    assert!(matches!(
        ast.expressions.view(values[0]).expect("bound return").operation(),
        ExpressionOperationView::Call(call) if matches!(
            call.handling().map(|record| &record.handling),
            Some(FallibleExpressionHandling::Propagate)
        )
    ));
}

#[test]
fn rejects_result_propagation_when_error_slot_types_do_not_match() {
    assert_type_mismatch_context(
        "ErrA = |\n    message String,\n|\n\nErrB = |\n    message String,\n|\n\ninner || -> I32, ErrA!:\n    return! ErrA(\"failed\")\n;\n\nouter || -> I32, ErrB!:\n    value = inner()!\n    return value\n;\n",
        TypeMismatchContext::ErrorReturn,
    );
}

#[test]
fn rejects_return_value_type_mismatch() {
    assert_type_mismatch_context(
        "render || -> String:\n    return 1\n;\n\nrender()\n",
        TypeMismatchContext::ReturnValue,
    );
}

#[test]
fn regular_division_return_type_mismatch_is_reported() {
    assert_type_mismatch_context(
        "render || -> I32:\n    return 5 / 2\n;\n\nrender()\n",
        TypeMismatchContext::ReturnValue,
    );
}
#[test]
fn parses_result_fallback_call_in_expression_position() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "can_error |value String| -> String, Error!:\n    return value\n;\n\nrecover |value String| -> String:\n    return can_error(value) catch:\n        then \"fallback\"\n    ;\n;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "recover");
    let NodeKind::Return(values) = &body[0].kind else {
        panic!("expected return statement in recover()");
    };

    assert_eq!(values.len(), 1);
    let ExpressionOperationView::ValueBlock(block) = ast
        .expressions
        .view(values[0])
        .expect("bound return")
        .operation()
    else {
        panic!("expected catch value block in recover return");
    };
    let ValueBlock::Catch(value_catch) = block else {
        panic!("expected catch value block");
    };
    let ExpressionOperationView::Call(call) = ast
        .expressions
        .view(value_catch.handled_value)
        .expect("bound handled value")
        .operation()
    else {
        panic!("expected handled call expression in catch value block");
    };
    assert!(matches!(
        call.handling().map(|record| &record.handling),
        Some(FallibleExpressionHandling::Recover)
    ));
    let FallibleHandling::Handler { body, .. } = &value_catch.handler else {
        panic!("expected catch-block handling");
    };
    assert_eq!(body.len(), 1);
    assert!(matches!(body[0].kind, NodeKind::ThenValue(_)));
}

#[test]
fn parses_inline_choice_predicate_receiver_as_value_match() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "Status :: Ready, Waiting;\n\
     score_for |status Status| -> I32:\n\
         score = if status is Ready then 1 else 0\n\
         return score\n\
     ;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "score_for");
    let NodeKind::VariableDeclaration(score_decl) = &body[0].kind else {
        panic!("expected score declaration in score_for()");
    };
    let ExpressionOperationView::ValueBlock(block) = ast
        .expressions
        .view(score_decl.value.initializer().expect("fixture initializer"))
        .expect("bound initializer")
        .operation()
    else {
        panic!("expected inline choice predicate to parse as a value block");
    };
    let ValueBlock::Match(value_match) = block else {
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
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "display |maybe_name String?| -> String:\n\
         name = if maybe_name is |name| then name else \"guest\"\n\
         return name\n\
     ;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "display");
    let NodeKind::VariableDeclaration(name_decl) = &body[0].kind else {
        panic!("expected name declaration in display()");
    };
    let ExpressionOperationView::ValueBlock(block) = ast
        .expressions
        .view(name_decl.value.initializer().expect("fixture initializer"))
        .expect("bound initializer")
        .operation()
    else {
        panic!("expected inline option unwrap to parse as a value block");
    };
    let ValueBlock::Match(value_match) = block else {
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
fn rejects_empty_inline_option_present_capture() {
    assert_function_diagnostic(
        "display |maybe_name String?| -> String:\n\
         name = if maybe_name is || then name else \"guest\"\n\
         return name\n\
     ;\n",
        "MOTH-RULE-0049",
        Some("invalid_match_pattern.empty_optional_present_capture"),
    );
}

#[test]
fn rejects_literal_inline_option_present_capture_binding() {
    assert_function_diagnostic(
        "display |maybe_name String?| -> String:\n\
         name = if maybe_name is |\"text\"| then name else \"guest\"\n\
         return name\n\
     ;\n",
        "MOTH-RULE-0049",
        Some("invalid_match_pattern.expected_binding_in_optional_present_capture"),
    );
}
#[test]
fn newline_between_is_and_option_capture_is_not_committed_at_inline_receiver() {
    let failure = parse_single_file_ast_diagnostic(
        "display |maybe_name String?| -> String:\n\
             name = if maybe_name is\n|name| then name else \"guest\"\n\
             return name\n\
         ;\n",
    );
    let diagnostic = first_ast_error_view(&failure);
    assert_ne!(
        diagnostic.descriptor().code,
        "MOTH-RULE-0049",
        "newline-separated receiver pipes must not commit option capture",
    );
}

#[test]
fn parses_block_option_present_capture_receiver_as_value_match() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "display |maybe_name String?| -> String:\n\
         name = if maybe_name is |name|:\n\
             then name\n\
         else\n\
             then \"guest\"\n\
         ;\n\
         return name\n\
     ;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "display");
    let NodeKind::VariableDeclaration(name_decl) = &body[0].kind else {
        panic!("expected name declaration in display()");
    };
    let ExpressionOperationView::ValueBlock(block) = ast
        .expressions
        .view(name_decl.value.initializer().expect("fixture initializer"))
        .expect("bound initializer")
        .operation()
    else {
        panic!("expected block option unwrap to parse as a value block");
    };
    let ValueBlock::Match(value_match) = block else {
        panic!("expected block option unwrap to lower through value match");
    };

    assert_eq!(value_match.arms.len(), 1);
    assert!(
        value_match.arms[0].guard.is_none(),
        "single-predicate block match must not invent a guard"
    );
    assert!(
        matches!(
            value_match.arms[0].pattern,
            MatchPattern::OptionPresentCapture { .. }
        ),
        "the block option unwrap predicate should bind a present payload"
    );
    assert!(
        value_match.default.is_some(),
        "block else branch should remain outside the present-capture scope"
    );
    assert_eq!(value_match.exhaustiveness, MatchExhaustiveness::HasDefault);
}

#[test]
fn newline_between_is_and_option_capture_is_not_committed_at_block_receiver() {
    let failure = parse_single_file_ast_diagnostic(
        "display |maybe_name String?| -> String:\n\
             name = if maybe_name is\n|name|:\n\
                 then name\n\
             else\n\
                 then \"guest\"\n\
             ;\n\
             return name\n\
         ;\n",
    );
    let diagnostic = first_ast_error_view(&failure);
    assert_ne!(
        diagnostic.descriptor().code,
        "MOTH-RULE-0049",
        "newline-separated receiver pipes must not commit option capture",
    );
}

#[test]
fn parses_block_choice_predicate_receiver_as_value_match() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "Status :: Ready, Waiting;\n\
     score_for |status Status| -> I32:\n\
         score = if status is Ready:\n\
             then 1\n\
         else\n\
             then 0\n\
         ;\n\
         return score\n\
     ;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "score_for");
    let NodeKind::VariableDeclaration(score_decl) = &body[0].kind else {
        panic!("expected score declaration in score_for()");
    };
    let ExpressionOperationView::ValueBlock(block) = ast
        .expressions
        .view(score_decl.value.initializer().expect("fixture initializer"))
        .expect("bound initializer")
        .operation()
    else {
        panic!("expected block choice predicate to parse as a value block");
    };
    let ValueBlock::Match(value_match) = block else {
        panic!("expected block choice predicate to lower through value match");
    };

    assert_eq!(value_match.arms.len(), 1);
    assert!(
        value_match.arms[0].guard.is_none(),
        "single-predicate block match must not invent a guard"
    );
    assert!(
        matches!(
            value_match.arms[0].pattern,
            MatchPattern::ChoiceVariant { tag: 0, .. }
        ),
        "Ready should resolve as the first Status variant, not as an ordinary value name"
    );
    assert!(
        value_match.default.is_some(),
        "block else branch should become the value-match default"
    );
    assert_eq!(value_match.exhaustiveness, MatchExhaustiveness::HasDefault);
}

#[test]
fn parses_inferred_multi_bind_inline_option_capture_as_value_match() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "display |maybe_name String?| -> String, I32:\n\
         label, count = if maybe_name is |name| then name, 1 else \"guest\", 0\n\
         return label, count\n\
     ;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "display");
    let NodeKind::MultiBind { value, .. } = &body[0].kind else {
        panic!("expected inferred multi-bind statement");
    };
    let ExpressionOperationView::ValueBlock(block) = ast
        .expressions
        .view(*value)
        .expect("bound value")
        .operation()
    else {
        panic!("expected inferred multi-bind option unwrap to parse as a value block");
    };
    let ValueBlock::Match(value_match) = block else {
        panic!("expected inferred inline option unwrap to lower through value match");
    };

    assert_eq!(value_match.arms.len(), 1);
    assert!(value_match.arms[0].guard.is_none());
    assert!(
        matches!(
            value_match.arms[0].pattern,
            MatchPattern::OptionPresentCapture { .. }
        ),
        "partially inferred inline option capture should bind a present payload"
    );
    assert!(value_match.default.is_some());
    assert_eq!(value_match.exhaustiveness, MatchExhaustiveness::HasDefault);
}

#[test]
fn parses_inferred_multi_bind_block_choice_payload_as_value_match() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "Response :: Complete, Failed | message String |;\n\
     label_for |response Response| -> String, I32:\n\
         text, code = if response is Failed(message):\n\
             then message, 1\n\
         else\n\
             then \"complete\", 0\n\
         ;\n\
         return text, code\n\
     ;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "label_for");
    let NodeKind::MultiBind { value, .. } = &body[0].kind else {
        panic!("expected inferred multi-bind statement");
    };
    let ExpressionOperationView::ValueBlock(block) = ast
        .expressions
        .view(*value)
        .expect("bound value")
        .operation()
    else {
        panic!("expected inferred block choice payload to parse as a value block");
    };
    let ValueBlock::Match(value_match) = block else {
        panic!("expected inferred block choice payload to lower through value match");
    };

    assert_eq!(value_match.arms.len(), 1);
    assert!(value_match.arms[0].guard.is_none());
    assert!(
        matches!(
            value_match.arms[0].pattern,
            MatchPattern::ChoiceVariant { tag: 1, .. }
        ),
        "Failed should resolve as a choice payload arm, not as an ordinary value name"
    );
    assert!(value_match.default.is_some());
    assert_eq!(value_match.exhaustiveness, MatchExhaustiveness::HasDefault);
}

#[test]
fn parses_option_equality_receiver_as_bool_value_if() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "compare |left String?, right String?| -> String:\n\
         result = if left is right then \"yes\" else \"no\"\n\
         return result\n\
     ;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "compare");
    let NodeKind::VariableDeclaration(result_decl) = &body[0].kind else {
        panic!("expected result declaration in compare()");
    };
    let ExpressionOperationView::ValueBlock(block) = ast
        .expressions
        .view(
            result_decl
                .value
                .initializer()
                .expect("fixture initializer"),
        )
        .expect("bound initializer")
        .operation()
    else {
        panic!("expected option equality to parse as a value block");
    };
    let ValueBlock::If(_) = block else {
        panic!("non-pattern option equality must fall back to Bool value-if, not a match");
    };
}

#[test]
fn parses_catch_handler_with_fallback_scope_in_declaration_rhs() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "can_error |value String| -> String, Error!:\n    return value\n;\n\nrecover |value String| -> String:\n    output = can_error(value) catch |err|:\n        io.line([: [err.message]])\n        then \"fallback\"\n    ;\n    return output\n;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "recover");
    let NodeKind::VariableDeclaration(output_decl) = &body[0].kind else {
        panic!("expected declaration statement in recover()");
    };

    let ExpressionOperationView::ValueBlock(block) = ast
        .expressions
        .view(
            output_decl
                .value
                .initializer()
                .expect("fixture initializer"),
        )
        .expect("bound initializer")
        .operation()
    else {
        panic!("expected catch value block in recover declaration");
    };
    let ValueBlock::Catch(value_catch) = block else {
        panic!("expected catch value block");
    };
    let ExpressionOperationView::Call(call) = ast
        .expressions
        .view(value_catch.handled_value)
        .expect("bound handled value")
        .operation()
    else {
        panic!("expected handled call expression in catch value block");
    };
    assert!(matches!(
        call.handling().map(|record| &record.handling),
        Some(FallibleExpressionHandling::Recover)
    ));
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
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "can_error || -> Error!:\n    return\n;\n\nrun || -> Error!:\n    can_error()!\n;\n",
    );

    let body = function_body_by_name(&ast, &path_fork, &string_table, "run");
    let NodeKind::ExpressionStatement(expression) = &body[0].kind else {
        panic!("expected standalone handled call to parse as an expression statement");
    };
    assert!(matches!(
        ast.expressions.view(*expression).expect("bound expression").operation(),
        ExpressionOperationView::Call(call) if matches!(
            call.handling().map(|record| &record.handling),
            Some(FallibleExpressionHandling::Propagate)
        )
    ));
}

// --------------------------
//  Trailing comma rejections
// --------------------------

#[test]
fn rejects_trailing_comma_in_single_return() {
    assert_function_diagnostic(
        "compute |x I32| -> I32,:\n    return 42\n;\n",
        "MOTH-RULE-0062",
        Some("invalid_function_signature.trailing_comma_in_returns"),
    );
}

#[test]
fn rejects_trailing_comma_in_error_return() {
    assert_function_diagnostic(
        "compute |x I32| -> I32, Error!,:\n    return 42\n;\n",
        "MOTH-RULE-0062",
        Some("invalid_function_signature.trailing_comma_in_returns"),
    );
}

// --------------------------
//  Valid parameter and return parsing
// --------------------------

#[test]
fn parses_valid_single_return_without_trailing_comma() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("compute |x I32| -> I32:\n    return 42\n;\n");

    let signature = function_signature_by_name(&ast, &path_fork, &string_table, "compute");
    assert_eq!(signature.returns.len(), 1);
    assert_eq!(
        signature.returns[0].type_id,
        Some(builtin_type_ids::fixed_scalar(FixedScalar::I32))
    );
    assert_eq!(signature.returns[0].channel, ReturnChannel::Success);
}

#[test]
fn parses_valid_multiple_returns_without_trailing_comma() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("compute |x I32| -> I32, String:\n    return 42, \"result\"\n;\n");

    let signature = function_signature_by_name(&ast, &path_fork, &string_table, "compute");
    assert_eq!(signature.returns.len(), 2);
    assert_eq!(
        signature.returns[0].type_id,
        Some(builtin_type_ids::fixed_scalar(FixedScalar::I32))
    );
    assert_eq!(signature.returns[0].channel, ReturnChannel::Success);
    assert_eq!(
        signature.returns[1].type_id,
        Some(ast.type_environment.builtins().string)
    );
    assert_eq!(signature.returns[1].channel, ReturnChannel::Success);
}

#[test]
fn parses_valid_single_parameter_without_trailing_comma() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("compute |x I32| -> I32:\n    return x\n;\n");

    let signature = function_signature_by_name(&ast, &path_fork, &string_table, "compute");
    assert_eq!(signature.parameters.len(), 1);
    assert_eq!(
        path_fork
            .component(signature.parameters[0].id)
            .map(|id| string_table.resolve(id)),
        Some("x")
    );
}

#[test]
fn parses_valid_multiple_parameters_without_trailing_comma() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("compute |x I32, y I32| -> I32:\n    return x + y\n;\n");

    let signature = function_signature_by_name(&ast, &path_fork, &string_table, "compute");
    assert_eq!(signature.parameters.len(), 2);
    assert_eq!(
        path_fork
            .component(signature.parameters[0].id)
            .map(|id| string_table.resolve(id)),
        Some("x")
    );
    assert_eq!(
        path_fork
            .component(signature.parameters[1].id)
            .map(|id| string_table.resolve(id)),
        Some("y")
    );
}

#[test]
fn optional_present_default_preserves_function_parameter_type() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "greet |name String? = \"guest\", required String?| -> String:\n    return \"ok\"\n;\n",
    );
    let signature = function_signature_by_name(&ast, &path_fork, &string_table, "greet");
    let defaulted = &signature.parameters[0];
    let required = &signature.parameters[1];
    assert!(required.default.is_none());
    let string = ast.type_environment.builtins().string;

    assert_eq!(
        ast.type_environment.option_inner_type(defaulted.type_id),
        Some(string),
        "a present default must keep the declared String? parameter identity"
    );
    assert_eq!(
        defaulted.type_id, required.type_id,
        "declared spelling must preserve the same semantic type through a present default"
    );
    assert_optional_string_default(
        &ast.const_values,
        defaulted.default.expect("present parameter default"),
        defaulted.type_id,
        string,
    );
}

#[test]
fn optional_present_default_preserves_constant_function_parameter_type() {
    let source = "FALLBACK_TEXT #= \"guest\"\n\
greet |name String? = FALLBACK_TEXT, required String?| -> String:\n\
    return \"ok\"\n\
;\n";
    let (ast, path_fork, string_table) = parse_single_file_ast(source);
    let signature = function_signature_by_name(&ast, &path_fork, &string_table, "greet");
    let defaulted = &signature.parameters[0];
    let required = &signature.parameters[1];
    assert!(required.default.is_none());
    let string = ast.type_environment.builtins().string;

    assert_eq!(
        ast.type_environment.option_inner_type(defaulted.type_id),
        Some(string),
        "a constant-backed present default must keep the declared String? parameter identity"
    );
    assert_eq!(
        defaulted.type_id, required.type_id,
        "the declared String? type must be unchanged by a constant-backed default"
    );
    assert_optional_string_default(
        &ast.const_values,
        defaulted.default.expect("present parameter default"),
        defaulted.type_id,
        string,
    );
}

#[test]
fn optional_present_default_preserves_receiver_method_parameter_type() {
    let source = "FALLBACK_TEXT #= \"guest\"\n\
Holder = |\n\
    id I32 = 0,\n\
|\n\
greet |this Holder, name String? = FALLBACK_TEXT, required String?| -> String:\n\
    return \"ok\"\n\
;\n";
    let (ast, path_fork, string_table) = parse_single_file_ast(source);
    let signature = function_signature_by_name(&ast, &path_fork, &string_table, "greet");
    let defaulted = &signature.parameters[1];
    let required = &signature.parameters[2];
    assert!(required.default.is_none());
    let string = ast.type_environment.builtins().string;

    assert_eq!(
        ast.type_environment.option_inner_type(defaulted.type_id),
        Some(string),
        "a receiver method's present default must keep its declared String? identity"
    );
    assert_eq!(
        defaulted.type_id, required.type_id,
        "the receiver method must retain the declared String? semantic type"
    );
}

#[test]
fn optional_present_default_does_not_supply_generic_function_type_argument() {
    let source = "FALLBACK_TEXT #= \"guest\"\n\
choose type T |value T, label String? = FALLBACK_TEXT, required String?| -> T:\n\
    return value\n\
;\n\
result = choose(7, required = none)\n";
    let (mut build_result, path_fork, string_table) =
        parse_single_file_ast_build_result(source).expect("generic default should resolve");
    let int = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let string = build_result.ast.type_environment.builtins().string;

    {
        let templates = build_result
            .materialisation_context
            .as_mut()
            .expect("module build should retain its generic templates")
            .generic_function_templates_mut();
        let template = templates
            .values()
            .find(|template| {
                path_fork
                    .component(template.function_path)
                    .map(|id| string_table.resolve(id))
                    == Some("choose")
            })
            .expect("the generic function template should be retained");
        let defaulted = &template.signature.parameters[1];
        let required = &template.signature.parameters[2];
        assert!(required.default.is_none());

        assert_eq!(
            build_result
                .ast
                .type_environment
                .option_inner_type(defaulted.type_id),
            Some(string),
            "the generic function's independent default must remain String?"
        );
        assert_eq!(
            defaulted.type_id, required.type_id,
            "the generic function must retain the declared String? semantic type"
        );
        assert_optional_string_default(
            &build_result.ast.const_values,
            defaulted.default.expect("present parameter default"),
            defaulted.type_id,
            string,
        );
    }

    let [request] = build_result.deferred_generic_requests.as_slice() else {
        panic!("the ordinary argument should produce one generic function request");
    };
    assert_eq!(
        request.key.type_arguments.as_ref(),
        &[int],
        "the supplied ordinary value, not the independent optional default, must infer T"
    );
}

#[test]
fn rejects_implicit_integer_default_for_float_parameter() {
    // Fixed integers and binary floats require an explicit conversion, so a typed I32
    // constant is not accepted at an F64 receiving boundary without `cast`.
    assert_type_mismatch_context(
        "LARGE #I32 = 16_777_217\n\
as_float |value F64 = LARGE, required F64| -> F64:\n\
    return value\n\
;\n",
        TypeMismatchContext::Declaration,
    );
}
#[test]
fn mutable_f64_operator_default_keeps_parameter_access_mode() {
    let source = "\
adjust |value ~F64 = 1 + 1|:\n\
    ~value = value + 1.0\n\
;\n\
\n\
caller |input ~F64|:\n\
    adjust(~input)\n\
;\n";
    let (ast, path_fork, string_table) = parse_single_file_ast(source);
    let signature = function_signature_by_name(&ast, &path_fork, &string_table, "adjust");
    let parameter = &signature.parameters[0];

    assert_eq!(
        parameter.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
        "the folded literal default must carry the declared F64 identity"
    );
    assert_eq!(
        parameter.value_mode,
        ValueMode::MutableOwned,
        "receiving coercion must not replace the authored mutable parameter mode"
    );
    let payload = ast
        .const_values
        .payload(parameter.default.expect("present parameter default"));
    assert!(
        matches!(payload, Some(ConstValuePayload::FixedScalar(value))
            if value.scalar() == FixedScalar::F64 && value.as_f64() == Some(2.0)),
        "the operator default should materialise as F64 two, got {payload:?}",
    );
}

#[test]
fn forwards_multiple_return_slots_without_a_tuple_value() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "pair || -> I32, String:\n    return 1, \"one\"\n;\n\
         forward || -> I32, String:\n    return pair()\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "forward");
    let NodeKind::Return(values) = &body[0].kind else {
        panic!("expected forwarding return");
    };
    assert_eq!(values.len(), 1, "the producer must execute once");
    assert_eq!(
        ast.expressions
            .view(values[0])
            .expect("bound return")
            .success_types(),
        &[
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            builtin_type_ids::STRING
        ],
    );
}

#[test]
fn single_return_rejects_zero_and_multiple_success_slots() {
    for (producer, produced_count) in [
        ("produce ||:\n    return\n;\n", 0),
        ("produce || -> I32, String:\n    return 1, \"one\"\n;\n", 2),
    ] {
        let source = format!("{producer}\nforward || -> I32:\n    return produce()\n;\n");
        let failure = parse_single_file_ast_diagnostic(&source);
        let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0087", None);
        let facts = diagnostic
            .family_view::<InvalidValueReceiverView>()
            .expect("the diagnostic is an invalid value receiver");
        assert_eq!(facts.receiver, ValueReceiverContext::Return, "{source}");
        assert_eq!(facts.target_count, 1, "{source}");
        assert_eq!(facts.produced_count, produced_count, "{source}");
    }
}

#[test]
fn receiver_methods_with_non_single_results_followed_by_assignment_report_receiver_arity() {
    for (method_name, method, produced_count) in [
        ("empty", "empty |this Point|:\n    return\n;\n", 0),
        (
            "pair",
            "pair |this Point| -> I32, I32:\n    return 1, 2\n;\n",
            2,
        ),
    ] {
        let source = format!(
            "Point = |\n    value I32 = 0,\n|\n\n{method}\n\
             caller |point Point|:\n    point.{method_name}() = 0\n;\n",
        );
        let failure = parse_single_file_ast_diagnostic(&source);
        let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0087", None);
        let facts = diagnostic
            .family_view::<InvalidValueReceiverView>()
            .expect("the diagnostic is an invalid value receiver");
        assert_eq!(
            facts.receiver,
            ValueReceiverContext::ReceiverBase,
            "{source}"
        );
        assert_eq!(facts.target_count, 1, "{source}");
        assert_eq!(facts.produced_count, produced_count, "{source}");
    }
}

#[test]
fn forwards_retained_numeric_call_results_per_slot() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "pair |number I64| -> I64, String:\n    return number, \"one\"\n;\n\
         forward || -> I64, String:\n    number = 1\n    return pair(number)\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "forward");
    let NodeKind::Return(values) = &body[1].kind else {
        panic!("expected resumed forwarding return");
    };
    assert_eq!(values.len(), 1);
    assert_eq!(
        ast.expressions
            .view(values[0])
            .expect("bound return")
            .success_types(),
        &[
            builtin_type_ids::fixed_scalar(FixedScalar::I64),
            builtin_type_ids::STRING
        ],
    );
}

#[test]
fn forwarded_return_validates_compatible_optional_slots() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "pair || -> I32, String:\n    return 1, \"one\"\n;\n\
         forward || -> I32?, String?:\n    return pair()\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "forward");
    let NodeKind::Return(values) = &body[0].kind else {
        panic!("expected compatible forwarding return");
    };
    assert_eq!(values.len(), 1);
    assert_eq!(
        ast.expressions
            .view(values[0])
            .expect("bound return")
            .success_types(),
        &[
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            builtin_type_ids::STRING
        ],
        "the producer keeps its natural slots at an optional receiving boundary",
    );
}
