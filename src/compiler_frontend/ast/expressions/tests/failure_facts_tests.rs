//! Invariants for expression failure side data and recovered contributor boundaries.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstValuePayload;
use crate::compiler_frontend::ast::expressions::assertion_message_effects::{
    assert_message_escape_diagnostic, classify_assertion_message_effect,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    ArgumentListView, AstExpressionBuilder, ExpressionOp, ExpressionOperationView, ExpressionStore,
    ExpressionView, ResolvedCallableTarget,
};
use crate::compiler_frontend::ast::expressions::failure_classification::{
    AstBuiltinFailureSummaries, EnclosingExitEffect, pending_expression_failure_facts,
    pending_function_failure_facts,
};
use crate::compiler_frontend::ast::expressions::failure_facts::{
    ExpressionFailureFacts, FailureDisposition, FailureWitnessSite, ImplicitFailureSource,
    WitnessCandidates,
};
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::{
    InvalidFallibleHandlingCode, InvalidFallibleHandlingView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::{BuiltinFailureOriginKind, DiagnosticBag};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, function_node_by_name,
};
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use std::cell::Cell;

fn returned_value<'a>(body: &[AstNode], owner: &'a ExpressionStore) -> ExpressionView<'a> {
    let id = body
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Return(values) => match values.as_slice() {
                [value] => Some(*value),
                _ => None,
            },
            _ => None,
        })
        .expect("the fixture returns exactly one value");
    owner
        .view(id)
        .expect("the return belongs to the fixture AST")
}

fn call_arguments(expression: ExpressionView<'_>) -> ArgumentListView<'_> {
    match expression.operation() {
        ExpressionOperationView::Call(call) => call.arguments(),
        _ => panic!("the fixture contains an ordinary function call"),
    }
}

fn argument(expression: ExpressionView<'_>, index: usize) -> ExpressionView<'_> {
    call_arguments(expression)
        .iter()
        .nth(index)
        .expect("the fixture has the requested argument")
        .value()
}

fn failure_facts(expression: ExpressionView<'_>) -> &ExpressionFailureFacts {
    static EMPTY: std::sync::LazyLock<ExpressionFailureFacts> =
        std::sync::LazyLock::new(ExpressionFailureFacts::default);
    expression.failure_facts().unwrap_or(&EMPTY)
}

#[test]
fn classifier_rejects_a_foreign_owner_even_when_row_ids_match() {
    use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
    use crate::compiler_frontend::value_mode::ValueMode;

    let mut constants = ConstValueStore::default();
    let value = constants
        .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
        .expect("the fixture constant is resolved");
    let mut first = AstExpressionBuilder::new().unwrap();
    let mut second = AstExpressionBuilder::new().unwrap();
    let first_root = first
        .constant(
            value,
            builtin_type_ids::BOOL,
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("the first fixture row is complete");
    let second_root = second
        .constant(
            value,
            builtin_type_ids::BOOL,
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("the second fixture row is complete");
    assert_eq!(
        first_root.id(),
        second_root.id(),
        "IDs are local, not globally unique"
    );
    assert!(
        pending_expression_failure_facts(
            first.view_owned(first_root).unwrap(),
            &TemplateIrStore::new(),
            &second
        )
        .is_err()
    );
    assert!(
        pending_expression_failure_facts(
            first.view_owned(first_root).unwrap(),
            &TemplateIrStore::new(),
            &first
        )
        .unwrap()
        .summary
        .first_implicit
        .is_none()
    );
}

#[test]
fn arithmetic_codes_exclude_boundary_and_format_invariants() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "power |value I32, exponent I32| -> I32:\n    return value ^ exponent\n;\n\
         remainder |value I32, divisor I32| -> I32:\n    return value % divisor\n;\n\
         divide |value F64, divisor F64| -> F64:\n    return value / divisor\n;\n",
    );
    let facts = pending_function_failure_facts(
        function_body_by_name(&ast, &path_fork, &string_table, "power"),
        &TemplateIrStore::new(),
        &ast.expressions,
    )
    .expect("resolved power facts");
    assert_eq!(facts.body.implicit.len(), 1);
    assert_eq!(
        facts.body.implicit[0].codes,
        &[
            BuiltinErrorCode::IntOverflow,
            BuiltinErrorCode::InvalidExponent
        ],
    );

    let remainder = pending_function_failure_facts(
        function_body_by_name(&ast, &path_fork, &string_table, "remainder"),
        &TemplateIrStore::new(),
        &ast.expressions,
    )
    .expect("resolved remainder facts");
    assert_eq!(remainder.body.implicit.len(), 1);
    assert_eq!(
        remainder.body.implicit[0].codes,
        &[BuiltinErrorCode::DivideByZero]
    );

    let float = pending_function_failure_facts(
        function_body_by_name(&ast, &path_fork, &string_table, "divide"),
        &TemplateIrStore::new(),
        &ast.expressions,
    )
    .expect("resolved float facts");
    assert_eq!(float.body.implicit.len(), 1);
    assert_eq!(
        float.body.implicit[0].codes,
        &[
            BuiltinErrorCode::DivideByZero,
            BuiltinErrorCode::FloatNonFinite
        ],
    );
}

#[test]
fn partial_arithmetic_fold_retains_only_the_surviving_operation_origin() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("check |value I32| -> I32:\n    return (1 + 2) + value\n;\n");
    let body = function_body_by_name(&ast, &path_fork, &string_table, "check");
    let expression = returned_value(body, &ast.expressions);
    let ExpressionOperationView::NumericBinary {
        left,
        right,
        facts: operation,
    } = expression.operation()
    else {
        panic!("the named operand keeps the outer addition runtime-dependent");
    };
    assert_eq!(failure_facts(expression).implicit.len(), 1);
    assert!(failure_facts(expression).checked_numeric_operation);
    assert!(failure_facts(expression).has_authored_safety_catch_eligibility());
    assert!(
        matches!(left.operation(), ExpressionOperationView::Constant(_)),
        "the constant addition was folded away"
    );
    assert!(matches!(
        right.operation(),
        ExpressionOperationView::Reference(_)
    ));
    let contributor = operation
        .failure
        .expect("the surviving addition owns its failure");
    assert_eq!(contributor.source, ImplicitFailureSource::NumericOperation);
    assert_eq!(contributor.codes, &[BuiltinErrorCode::IntOverflow]);

    let projected = pending_function_failure_facts(body, &TemplateIrStore::new(), &ast.expressions)
        .expect("the partly folded body has valid failure facts");
    assert_eq!(projected.body.implicit.len(), 1);
    assert_eq!(projected.body.implicit[0].span, contributor.span);
    assert_eq!(projected.body.implicit[0].source, contributor.source);
    assert_eq!(projected.body.implicit[0].codes, contributor.codes);
}

#[test]
fn call_keeps_argument_failure_without_changing_success_type() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "format_count |count I32| -> String:\n    return \"count\"\n;\n\
         check |value I32| -> String:\n    return format_count(value + 1)\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "check");
    let expression = returned_value(body, &ast.expressions);
    let argument = argument(expression, 0);
    let numeric_span = failure_facts(argument)
        .summary
        .first_numeric
        .expect("the argument contains checked arithmetic")
        .span;

    assert_eq!(
        expression.value_type_id().unwrap(),
        builtin_type_ids::STRING
    );
    assert_eq!(
        failure_facts(expression)
            .summary
            .first_numeric
            .expect("the call retains the argument's failure classification")
            .span,
        numeric_span,
    );
    assert_eq!(
        classify_assertion_message_effect(expression, &TemplateIrStore::new(), &ast.expressions)
            .expect("valid AST"),
        Some(EnclosingExitEffect::InferredFailure(numeric_span)),
    );
}

#[test]
fn nested_calls_retain_one_local_witness_per_producer_in_postorder() {
    const PRODUCER_COUNT: usize = 16;
    let mut source = String::new();
    for index in 0..PRODUCER_COUNT {
        source.push_str(&format!(
            "producer_{index} |value I32| -> I32:\n    return value + 1\n;\n",
        ));
    }
    source.push_str("check || -> I32:\n    return ");
    for index in (0..PRODUCER_COUNT).rev() {
        source.push_str(&format!("producer_{index}("));
    }
    source.push('1');
    for _ in 0..PRODUCER_COUNT {
        source.push(')');
    }
    source.push_str("\n;\n");

    let (ast, path_fork, string_table) = parse_single_file_ast(&source);
    let body = function_body_by_name(&ast, &path_fork, &string_table, "check");
    let expression = returned_value(body, &ast.expressions);
    let mut current = expression;
    let mut stored_witnesses = 0;
    let mut producer_sites = Vec::new();
    while let ExpressionOperationView::Call(call) = current.operation() {
        let ResolvedCallableTarget::Function(name) = call.metadata().target else {
            panic!("the fixture contains source calls");
        };
        let local_facts = failure_facts(current);
        assert_eq!(local_facts.implicit.len(), 1);
        let witness = &local_facts.implicit[0];
        assert_eq!(witness.span, current.span());
        assert_eq!(witness.source, ImplicitFailureSource::PrivateCall(name));
        assert!(witness.codes.is_empty());
        assert!(current.span().is_some());
        assert!(!producer_sites.iter().any(|(span, source)| {
            *span == current.span() || *source == ImplicitFailureSource::PrivateCall(name)
        }));

        stored_witnesses += local_facts.implicit.len();
        producer_sites.push((current.span(), ImplicitFailureSource::PrivateCall(name)));
        assert_eq!(call.arguments().len(), 1);
        current = argument(current, 0);
    }
    stored_witnesses += failure_facts(current).implicit.len();
    assert_eq!(producer_sites.len(), PRODUCER_COUNT);
    assert_eq!(stored_witnesses, PRODUCER_COUNT);
    producer_sites.reverse();

    let pending = pending_function_failure_facts(body, &TemplateIrStore::new(), &ast.expressions)
        .expect("valid AST");
    let compact =
        pending_expression_failure_facts(expression, &TemplateIrStore::new(), &ast.expressions)
            .expect("valid AST");
    drop(ast);
    let collected_sites: Vec<_> = pending
        .body
        .implicit
        .iter()
        .map(|witness| (witness.span, witness.source))
        .collect();
    assert_eq!(collected_sites, producer_sites);
    assert_eq!(
        pending
            .body
            .summary
            .summary
            .first_private_call
            .expect("the compact summary retains the first producer")
            .span,
        producer_sites[0].0,
    );
    assert!(pending.assertion_message_calls.is_empty());

    assert!(compact.implicit.is_empty());
    assert_eq!(
        compact
            .summary
            .first_private_call
            .expect("the chain has a producer")
            .span,
        producer_sites[0].0,
    );
}

#[test]
fn inactive_assertion_keeps_private_call_facts_after_ast_release() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "message type T |value T| -> T:\n    return value\n;\n\
         check |value String|:\n    assert(true, message(value))\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "check");
    let [assertion] = body else {
        panic!("the fixture contains one assertion");
    };
    let NodeKind::Assert { message, .. } = assertion.kind else {
        panic!("the fixture retains the validated assertion");
    };
    let message = ast.expressions.view(message).unwrap();
    let ExpressionOperationView::Constant(value) = message.operation() else {
        panic!("a known-true assertion discards executable message work");
    };
    assert!(matches!(
        ast.const_values.payload(value),
        Some(ConstValuePayload::OptionNone)
    ));
    let pending = pending_function_failure_facts(body, &TemplateIrStore::new(), &ast.expressions)
        .expect("the inactive message retains validated failure facts");
    let summaries = AstBuiltinFailureSummaries::compute(
        &[&ast.nodes],
        &TemplateIrStore::new(),
        &ast.expressions,
    )
    .expect("source facts retain the pending concrete message call");
    drop(ast);

    assert!(pending.body.implicit.is_empty());
    let [contributor] = pending.assertion_message_calls.as_slice() else {
        panic!("the inert message keeps exactly its original private-call witness");
    };
    assert!(matches!(
        contributor.source,
        ImplicitFailureSource::PrivateCall(_)
    ));
    assert!(contributor.span.is_some());
    assert!(contributor.codes.is_empty());
    assert!(summaries.is_active(contributor));
    let witness = summaries.witness(contributor);
    assert_eq!(witness.call_spans, vec![contributor.span]);
    assert!(witness.origin_span.is_none());
}

#[test]
fn deferred_custom_catch_keeps_typed_locations_and_candidates_after_ast_release() {
    let (pending, summaries, expected, private_span, error_path, type_environment) = {
        let (ast, path_fork, string_table) = parse_single_file_ast(
            "Failure = | message String |\n\
             operand type T |value T| -> T:\n    return value\n;\n\
             load |value I32| -> I32, Failure!:\n    return value\n;\n\
             recover |value I32| -> I32:\n    return load(operand(value)) catch then 0\n;\n",
        );
        let body = function_body_by_name(&ast, &path_fork, &string_table, "recover");
        let expression = returned_value(body, &ast.expressions);
        let ExpressionOperationView::ValueBlock(ValueBlock::Catch(catch)) = expression.operation()
        else {
            panic!("the fixture retains a typed custom-error catch");
        };
        let protected = expression.bind(catch.handled_value).unwrap();
        let facts = failure_facts(protected);
        let expected = facts
            .deferred_custom_catch
            .expect("private operands retain their custom-error compatibility check");
        let private_span = facts.summary.first_private_call.unwrap().span;
        let error_path = *ast
            .type_environment
            .nominal_path(expected.error_type_id)
            .unwrap();
        let pending =
            pending_function_failure_facts(body, &TemplateIrStore::new(), &ast.expressions)
                .expect("custom-error catch facts project before expression release");
        let summaries = AstBuiltinFailureSummaries::compute(
            &[&ast.nodes],
            &TemplateIrStore::new(),
            &ast.expressions,
        )
        .expect("source facts retain the pending concrete operand call");

        // Move the required type authority out while the AST and expression owner end here.
        (
            pending,
            summaries,
            expected,
            private_span,
            error_path,
            ast.type_environment,
        )
    };
    assert!(pending.body.implicit.is_empty());
    let [check] = pending.body.deferred_custom_catches.as_slice() else {
        panic!("the retained projection has exactly one custom-error compatibility check");
    };
    assert!(!check.eligibility_only);
    assert_eq!(check.error_type_id, expected.error_type_id);
    assert_eq!(
        type_environment.nominal_path(check.error_type_id),
        Some(&error_path)
    );
    assert_eq!(check.catch_span, expected.catch_span);
    assert_eq!(check.typed_producer_span, expected.typed_producer_span);
    assert!(check.catch_span.is_some());
    assert!(check.typed_producer_span.is_some());
    assert_ne!(check.catch_span, check.typed_producer_span);
    let [candidate] = check.candidates.as_slice() else {
        panic!("only the protected private operand needs deferred classification");
    };
    assert_eq!(candidate.span, private_span);
    assert!(candidate.span.is_some());
    assert_ne!(candidate.span, check.typed_producer_span);
    assert!(matches!(
        candidate.source,
        ImplicitFailureSource::PrivateCall(_)
    ));
    assert!(summaries.is_active(candidate));
    let witness = summaries.witness(candidate);
    assert_eq!(witness.call_spans, vec![candidate.span]);
    assert!(witness.origin_span.is_none());
}

#[test]
fn catch_handler_failure_goes_outward_without_reentering_protected_work() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "protected |value I32| -> I32:\n    return value + 1\n;\n\
         check |value I32| -> I32:\n\
             return protected(value + 1) catch:\n        then value // value\n    ;\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "check");
    let expression = returned_value(body, &ast.expressions);
    let ExpressionOperationView::ValueBlock(block) = expression.operation() else {
        panic!("the fixture returns a catch value block");
    };
    let ValueBlock::Catch(catch) = block else {
        panic!("the fixture contains a catch");
    };
    let handled_value = expression.bind(catch.handled_value).unwrap();
    let protected_span = handled_value.span();
    let protected_numeric_span = failure_facts(handled_value)
        .summary
        .first_numeric
        .expect("the protected argument contains checked arithmetic")
        .span;
    assert!(
        failure_facts(handled_value)
            .summary
            .first_private_call
            .is_some()
    );

    let pending = pending_function_failure_facts(body, &TemplateIrStore::new(), &ast.expressions)
        .expect("valid AST");
    assert_eq!(pending.body.implicit.len(), 1);
    let handler_witness = &pending.body.implicit[0];
    assert_eq!(
        handler_witness.source,
        ImplicitFailureSource::NumericOperation
    );
    assert_eq!(
        handler_witness.codes,
        &[
            BuiltinErrorCode::DivideByZero,
            BuiltinErrorCode::IntOverflow
        ],
    );
    assert!(handler_witness.span.is_some());
    assert_ne!(handler_witness.span, protected_span);
    assert_ne!(handler_witness.span, protected_numeric_span);

    let protected =
        pending_expression_failure_facts(handled_value, &TemplateIrStore::new(), &ast.expressions)
            .expect("valid AST");
    assert!(protected.summary.first_implicit.is_none());
    let outward =
        pending_expression_failure_facts(expression, &TemplateIrStore::new(), &ast.expressions)
            .expect("valid AST");
    assert_eq!(
        outward
            .summary
            .first_numeric
            .expect("handler failure escapes")
            .span,
        handler_witness.span,
    );
    assert_eq!(
        classify_assertion_message_effect(expression, &TemplateIrStore::new(), &ast.expressions)
            .expect("valid AST"),
        Some(EnclosingExitEffect::InferredFailure(handler_witness.span)),
    );
}

#[test]
fn first_numeric_and_typed_diagnostics_keep_descendant_producer_spans() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "typed_first |value I32| -> I32, Error!:\n    return value\n;\n\
         typed_second |value I32| -> I32, Error!:\n    return value\n;\n\
         check |value I32| -> I32:\n\
             return typed_second(typed_first(value + 1)) + typed_second(value * 2) catch then 0\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "check");
    let expression = returned_value(body, &ast.expressions);
    let ExpressionOperationView::ValueBlock(block) = expression.operation() else {
        panic!("the fixture returns a catch value block");
    };
    let ValueBlock::Catch(catch) = block else {
        panic!("the fixture contains a catch");
    };
    let protected = expression.bind(catch.handled_value).unwrap();
    let ExpressionOperationView::NumericBinary {
        left: first_outer_call,
        right: second_call,
        ..
    } = protected.operation()
    else {
        panic!("the protected expression adds two typed call results");
    };
    let first_inner_call = argument(first_outer_call, 0);
    let first_typed = failure_facts(first_inner_call)
        .typed_error
        .expect("the inner call produces builtin Error");
    let first_numeric = failure_facts(argument(first_inner_call, 0))
        .summary
        .first_numeric
        .expect("the inner argument contains checked arithmetic");
    let later_numeric = failure_facts(argument(second_call, 0))
        .summary
        .first_numeric
        .expect("the later argument also contains checked arithmetic");
    assert!(first_typed.span.is_some());
    assert!(first_numeric.span.is_some());
    assert_ne!(first_typed.span, first_outer_call.span());
    assert_ne!(first_numeric.span, later_numeric.span);
    assert_eq!(
        failure_facts(protected).summary.first_typed,
        Some(first_typed)
    );
    assert_eq!(
        failure_facts(protected).summary.first_numeric,
        Some(first_numeric),
    );
    assert!(failure_facts(protected).summary.conflicting_typed.is_none());

    // The diagnostic owner reads the retained summary, not an owning expression clone.
    // Project that summary into a pending occurrence in a fresh construction domain.
    let mut builder = AstExpressionBuilder::new().unwrap();
    let ExpressionOperationView::Call(call) = first_outer_call.operation() else {
        panic!("the first producer is a call");
    };
    let ResolvedCallableTarget::Function(path) = call.metadata().target else {
        panic!("the producer is a source function");
    };
    let pending = builder
        .append_expression(
            ExpressionOp::Reference(path),
            protected.span(),
            protected.value_type_id().unwrap(),
            protected.value_mode(),
        )
        .unwrap();
    let mut pending_facts = failure_facts(protected).clone();
    pending_facts.disposition = FailureDisposition::Pending;
    let pending = builder.with_failure_facts(pending, pending_facts).unwrap();
    drop(ast);
    let pending = builder.view_owned(pending).unwrap();
    let typed_diagnostic = pending
        .unhandled_typed_error_diagnostic()
        .expect("the pending expression has an unhandled typed producer");
    let typed_compact = compact_bag_for_test(DiagnosticBag::from(typed_diagnostic), None);
    let typed_diagnostic = typed_compact.first().expect("the diagnostic is retained");
    assert!(matches!(
        typed_diagnostic.primary_site(),
        SiteRef::Local(span) if Some(span) == first_typed.span
    ));
    let typed_facts = typed_diagnostic
        .family_view::<InvalidFallibleHandlingView>()
        .expect("the diagnostic is invalid fallible handling");
    assert_eq!(
        typed_facts.reason,
        InvalidFallibleHandlingCode::UnhandledErrorReturn
    );
    let numeric_diagnostic =
        assert_message_escape_diagnostic(pending, &TemplateIrStore::new(), &builder)
            .expect("valid AST")
            .expect("checked numeric work can escape message evaluation");
    let numeric_compact = compact_bag_for_test(DiagnosticBag::from(numeric_diagnostic), None);
    let numeric_diagnostic = numeric_compact.first().expect("the diagnostic is retained");
    assert!(matches!(
        numeric_diagnostic.primary_site(),
        SiteRef::Local(span) if Some(span) == first_numeric.span
    ));
    let numeric_facts = numeric_diagnostic
        .family_view::<InvalidFallibleHandlingView>()
        .expect("the diagnostic is invalid fallible handling");
    assert_eq!(
        numeric_facts.reason,
        InvalidFallibleHandlingCode::AssertionMessageCannotEscape
    );
}

#[test]
fn failure_witness_prefers_only_the_same_compound_updates_writeback_through_private_hops() {
    // The earlier statement must stay genuinely fallible so the first-origin branch keeps its
    // intent: `U8 + I8` is discharged through its `I16` common domain, so the only failure edge
    // left inside the compound update is the `I16`-to-`U8` write-back, while `U32 + U32` is a
    // genuinely fallible operation.
    for earlier in ["", "    earlier = wide + one\n"] {
        let (ast, path_fork, string_table) = parse_single_file_ast(&format!(
            "risky |amount I8| -> I8, Error!:\n    return amount\n;\n\
             bump |wide U32, one U32, amount I8| -> U8:\n\
             {earlier}    total ~U8 = 250\n\
                 ~total += risky(amount) catch then amount\n    return total\n;\n\
             forward |wide U32, one U32, amount I8| -> U8:\n    return bump(wide, one, amount)\n;\n"
        ));
        let summaries = AstBuiltinFailureSummaries::compute(
            &[&ast.nodes],
            &TemplateIrStore::new(),
            &ast.expressions,
        )
        .expect("fixture summaries converge");
        let bump = function_body_by_name(&ast, &path_fork, &string_table, "bump");
        let facts = pending_function_failure_facts(bump, &TemplateIrStore::new(), &ast.expressions)
            .unwrap();
        let expected = summaries.first_active(&facts.body.implicit).unwrap();
        let direct = summaries.witness(expected);
        if earlier.is_empty() {
            assert_eq!(
                direct.origin,
                BuiltinFailureOriginKind::CompoundWriteBack {
                    target: builtin_type_ids::fixed_scalar(
                        moth_lexical::numeric::fixed_scalar::FixedScalar::U8
                    ),
                }
            );
        } else {
            assert_eq!(direct.origin, BuiltinFailureOriginKind::Operation);
            assert_eq!(direct.origin_span, facts.body.implicit[0].span);
        }
        let forward = function_body_by_name(&ast, &path_fork, &string_table, "forward");
        let facts =
            pending_function_failure_facts(forward, &TemplateIrStore::new(), &ast.expressions)
                .unwrap();
        let witness = summaries.witness(summaries.first_active(&facts.body.implicit).unwrap());
        assert_eq!(witness.origin, direct.origin);
        assert_eq!(witness.origin_span, direct.origin_span);
        assert_eq!(witness.call_spans.len(), 1);
    }
}

#[test]
fn recursive_witness_skips_back_edge_before_later_numeric_origin() {
    // WHAT: the recursive call precedes the overflow producer in source order in both the
    //       self-recursive and the mutually recursive fixtures.
    // WHY: the witness must skip only the back-edge and retain the available origin instead
    //      of stopping with call-only hops and no origin.
    for source in [
        "helper |value I32, ready Bool| -> I32:\n\
             if ready:\n        return helper(value, ready)\n    ;\n    return value * 2\n;\n\
         inner |value I32, ready Bool| -> I32:\n    return helper(value, ready)\n;\n\
         middle |value I32, ready Bool| -> I32:\n    return inner(value, ready)\n;\n\
         outer |value I32, ready Bool| -> I32:\n    return middle(value, ready)\n;\n\
         boundary |value I32, ready Bool| -> I32:\n    return outer(value, ready)\n;\n",
        "first |value I32, ready Bool| -> I32:\n\
             if ready:\n        return second(value, ready)\n    ;\n    return value * 2\n;\n\
         second |value I32, ready Bool| -> I32:\n    return first(value, ready)\n;\n\
         inner |value I32, ready Bool| -> I32:\n    return first(value, ready)\n;\n\
         middle |value I32, ready Bool| -> I32:\n    return inner(value, ready)\n;\n\
         outer |value I32, ready Bool| -> I32:\n    return middle(value, ready)\n;\n\
         boundary |value I32, ready Bool| -> I32:\n    return outer(value, ready)\n;\n",
    ] {
        let (ast, path_fork, string_table) = parse_single_file_ast(source);
        let summaries = AstBuiltinFailureSummaries::compute(
            &[&ast.nodes],
            &TemplateIrStore::new(),
            &ast.expressions,
        )
        .expect("fixture summaries converge");
        let boundary = function_body_by_name(&ast, &path_fork, &string_table, "boundary");
        let facts =
            pending_function_failure_facts(boundary, &TemplateIrStore::new(), &ast.expressions)
                .unwrap();
        let helper = if source.starts_with("first") {
            "first"
        } else {
            "helper"
        };
        let helper_body = function_body_by_name(&ast, &path_fork, &string_table, helper);
        let helper_facts =
            pending_function_failure_facts(helper_body, &TemplateIrStore::new(), &ast.expressions)
                .unwrap();
        let numeric_span = helper_facts
            .body
            .implicit
            .iter()
            .find(|contributor| {
                matches!(contributor.source, ImplicitFailureSource::NumericOperation)
            })
            .expect("the helper retains its numeric producer")
            .span;
        let recursive_span = helper_facts
            .body
            .implicit
            .iter()
            .find(|contributor| matches!(contributor.source, ImplicitFailureSource::PrivateCall(_)))
            .expect("the recursive branch precedes the producer")
            .span;
        let visible_call_spans: Vec<_> = ["boundary", "outer", "middle"]
            .into_iter()
            .map(|name| {
                let body = function_body_by_name(&ast, &path_fork, &string_table, name);
                let pending =
                    pending_function_failure_facts(body, &TemplateIrStore::new(), &ast.expressions)
                        .unwrap();
                assert_eq!(pending.body.implicit.len(), 1);
                pending.body.implicit[0].span
            })
            .collect();
        assert!(matches!(
            helper_facts.body.implicit[0].source,
            ImplicitFailureSource::PrivateCall(_)
        ));
        // Witness selection consumes retained facts after their expression owner has ended.
        drop(ast);
        let entry = summaries
            .first_active(&facts.body.implicit)
            .expect("the boundary escapes through its helper");
        let witness = summaries.witness(entry);
        assert_eq!(witness.origin, BuiltinFailureOriginKind::Operation);

        // The origin is the actual multiplication in the helper, never the skipped call.
        assert_eq!(witness.origin_span, numeric_span);
        assert!(!witness.call_spans.contains(&recursive_span));
        assert_eq!(witness.call_spans, visible_call_spans);
        assert_eq!(witness.elided_call_hops, 1);

        // Repeated runs over the same summaries produce the same structured witness.
        assert_eq!(summaries.witness(entry), witness);
    }
}

#[test]
fn origin_free_recursion_stays_free_of_recoverable_numeric_failure() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "recurse |value I32| -> I32:\n    return recurse(value)\n;\n\
         boundary |value I32| -> I32:\n    return recurse(value)\n;\n",
    );
    let summaries = AstBuiltinFailureSummaries::compute(
        &[&ast.nodes],
        &TemplateIrStore::new(),
        &ast.expressions,
    )
    .expect("fixture summaries converge");
    let boundary = function_body_by_name(&ast, &path_fork, &string_table, "boundary");
    let facts = pending_function_failure_facts(boundary, &TemplateIrStore::new(), &ast.expressions)
        .unwrap();
    assert!(
        summaries.first_active(&facts.body.implicit).is_none(),
        "skipping the back-edge must not invent an origin for pure recursion"
    );
}

#[test]
fn unavailable_call_before_numeric_origin_stops_at_the_visible_leaf() {
    // WHAT: an active dormant generic callee (no retained body facts) precedes the numeric
    //       producer in source order.
    // WHY: the leaf is terminal, not skippable: trying the later producer would invent a
    //      connected origin across the unavailable hop already recorded in call_spans.
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "deferred type U |value U| -> U:\n    return value\n;\n\
         helper |value I32| -> I32:\n    return deferred(value) + value * 2\n;\n\
         boundary |value I32| -> I32:\n    return helper(value)\n;\n",
    );
    let summaries = AstBuiltinFailureSummaries::compute(
        &[&ast.nodes],
        &TemplateIrStore::new(),
        &ast.expressions,
    )
    .expect("fixture summaries converge");
    let helper = function_body_by_name(&ast, &path_fork, &string_table, "helper");
    let facts =
        pending_function_failure_facts(helper, &TemplateIrStore::new(), &ast.expressions).unwrap();
    let entry = summaries
        .first_active(&facts.body.implicit)
        .expect("the helper escapes through the unavailable callee");
    let witness = summaries.witness(entry);
    let unavailable_span = facts
        .body
        .implicit
        .iter()
        .find(|contributor| matches!(contributor.source, ImplicitFailureSource::PrivateCall(_)))
        .expect("the unavailable call precedes the producer")
        .span;
    assert_eq!(witness.call_spans, vec![unavailable_span]);
    assert!(witness.origin_span.is_none());
}

#[test]
fn recursive_back_edge_before_compound_update_keeps_writeback_origin() {
    // WHAT: the recursive call precedes the compound update in the proven write-back
    //       fixture; the handled RHS leaves the write-back as the policy winner, but in
    //       raw order the call and the arithmetic both precede it.
    // WHY: skipping the back-edge must reapply the write-back policy instead of walking
    //      the remainder in raw order (which would report the arithmetic as Operation).
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "risky |amount I8| -> I8, Error!:\n    return amount\n;\n\
         helper |wide U32, one U32, amount I8, ready Bool| -> U8:\n\
             if ready:\n        return helper(wide, one, amount, ready)\n    ;\n\
             total ~U8 = 250\n    ~total += risky(amount) catch then amount\n    return total\n;\n\
         boundary |wide U32, one U32, amount I8, ready Bool| -> U8:\n\
             return helper(wide, one, amount, ready)\n;\n",
    );
    let summaries = AstBuiltinFailureSummaries::compute(
        &[&ast.nodes],
        &TemplateIrStore::new(),
        &ast.expressions,
    )
    .expect("fixture summaries converge");
    let boundary = function_body_by_name(&ast, &path_fork, &string_table, "boundary");
    let facts = pending_function_failure_facts(boundary, &TemplateIrStore::new(), &ast.expressions)
        .unwrap();
    let entry = summaries
        .first_active(&facts.body.implicit)
        .expect("the boundary escapes through its helper");
    let witness = summaries.witness(entry);
    let helper = function_body_by_name(&ast, &path_fork, &string_table, "helper");
    let helper_facts =
        pending_function_failure_facts(helper, &TemplateIrStore::new(), &ast.expressions).unwrap();
    let writeback_span = helper_facts
        .body
        .implicit
        .iter()
        .find(|contributor| {
            matches!(
                contributor.source,
                ImplicitFailureSource::CompoundWriteBack { .. }
            )
        })
        .expect("the compound update retains its write-back")
        .span;
    assert_eq!(witness.origin_span, writeback_span);
    assert!(matches!(
        witness.origin,
        BuiltinFailureOriginKind::CompoundWriteBack { .. }
    ));
}

#[test]
fn mutually_recursive_component_with_late_origin_expands_each_body_once() {
    // WHAT: every body calls every other body before body 0's only numeric producer, so all
    //       bodies are active and the origin is reachable only after the whole component.
    // WHY: path-only backtracking re-entered exhausted bodies through every simple path (over
    //      a hundred thousand expansions at nine bodies). The bounded search enters each body
    //      once and still reports the same origin with only the entry hop visible.
    const BODY_COUNT: usize = 12;
    let mut source = String::new();
    for body in 0..BODY_COUNT {
        source.push_str(&format!("body_{body} |value I32, ready Bool| -> I32:\n"));
        for callee in (0..BODY_COUNT).filter(|callee| *callee != body) {
            source.push_str(&format!(
                "    if ready:\n        return body_{callee}(value, ready)\n    ;\n"
            ));
        }
        let result = if body == 0 { "value * 2" } else { "value" };
        source.push_str(&format!("    return {result}\n;\n"));
    }
    source
        .push_str("boundary |value I32, ready Bool| -> I32:\n    return body_0(value, ready)\n;\n");

    let (ast, path_fork, string_table) = parse_single_file_ast(&source);
    let summaries = AstBuiltinFailureSummaries::compute(
        &[&ast.nodes],
        &TemplateIrStore::new(),
        &ast.expressions,
    )
    .expect("fixture summaries converge");
    let boundary = function_body_by_name(&ast, &path_fork, &string_table, "boundary");
    let boundary_facts =
        pending_function_failure_facts(boundary, &TemplateIrStore::new(), &ast.expressions)
            .unwrap();
    let entry = summaries
        .first_active(&boundary_facts.body.implicit)
        .expect("the boundary escapes through the component");

    let (witness, expanded_bodies) = summaries.witness_with_expanded_bodies(entry);
    assert_eq!(expanded_bodies, BODY_COUNT);

    let origin_body = function_body_by_name(&ast, &path_fork, &string_table, "body_0");
    let origin_facts =
        pending_function_failure_facts(origin_body, &TemplateIrStore::new(), &ast.expressions)
            .unwrap();
    let numeric_span = origin_facts
        .body
        .implicit
        .iter()
        .find(|contributor| matches!(contributor.source, ImplicitFailureSource::NumericOperation))
        .expect("body 0 retains its late numeric producer")
        .span;
    assert_eq!(witness.origin, BuiltinFailureOriginKind::Operation);
    assert_eq!(witness.origin_span, numeric_span);
    assert_eq!(witness.call_spans, vec![entry.span]);
    assert_eq!(witness.elided_call_hops, 0);
    assert_eq!(summaries.witness(entry), witness);
}

fn cursor_span(index: u32) -> SourceSpan {
    let mut builder = ExtendedSpanBuilder::new();
    let local = LocalSpan::exact(index * 10, 3, &mut builder).unwrap();
    SourceSpan::new(SourceId::from_index(1), local)
}

#[test]
fn witness_candidates_yield_calls_then_one_terminal_origin_on_demand() {
    // WHAT: a long body whose first candidate already ends the search.
    // WHY: the walkers try one candidate at a time, so selecting the first must not classify or
    //      order the rest. Arithmetic looks ahead once for its own compound write-back.
    const CALL_COUNT: usize = 10_000;
    let mut sites = vec![FailureWitnessSite::Call; CALL_COUNT];
    let activity_checks = Cell::new(0);
    let site_reads = Cell::new(0);
    let is_active = |_: &FailureWitnessSite| {
        activity_checks.set(activity_checks.get() + 1);
        Ok::<_, std::convert::Infallible>(true)
    };
    let site = |site: &FailureWitnessSite| {
        site_reads.set(site_reads.get() + 1);
        *site
    };

    let mut candidates = WitnessCandidates::new(&sites);
    let first = candidates.next_candidate(is_active, site).unwrap();
    assert!(std::ptr::eq(first.unwrap(), &sites[0]));
    assert_eq!((activity_checks.get(), site_reads.get()), (1, 1));

    // Arithmetic scans its remainder once, linearly, and yields its write-back first.
    sites[0] = FailureWitnessSite::Arithmetic(Some(cursor_span(0)));
    sites[CALL_COUNT - 1] = FailureWitnessSite::WriteBack(Some(cursor_span(0)));
    activity_checks.set(0);
    site_reads.set(0);
    let mut candidates = WitnessCandidates::new(&sites);
    let first = candidates.next_candidate(is_active, site).unwrap();
    assert!(std::ptr::eq(first.unwrap(), &sites[CALL_COUNT - 1]));
    assert_eq!(activity_checks.get(), 2);
    assert_eq!(site_reads.get(), CALL_COUNT);
    // The selected write-back is terminal, so the cursor is exhausted.
    assert!(
        candidates
            .next_candidate(is_active, site)
            .unwrap()
            .is_none()
    );

    // Calls are skippable and stay in order until the first terminal origin ends the body.
    sites[2] = FailureWitnessSite::Arithmetic(Some(cursor_span(2)));
    sites[0] = FailureWitnessSite::Call;
    activity_checks.set(0);
    let mut candidates = WitnessCandidates::new(&sites);
    for expected in sites.iter().take(3) {
        let yielded = candidates.next_candidate(is_active, site).unwrap();
        assert!(std::ptr::eq(yielded.unwrap(), expected));
    }
    assert!(
        candidates
            .next_candidate(is_active, site)
            .unwrap()
            .is_none()
    );
    assert_eq!(activity_checks.get(), 3);
}

#[test]
fn u32_arithmetic_records_stable_integer_failure_codes() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "add |value U32, other U32| -> U32:\n    return value + other\n;\n\
         divide |value U32, divisor U32| -> U32:\n    return value // divisor\n;\n\
         power |value U32, exponent U32| -> U32:\n    return value ^ exponent\n;\n",
    );
    let add = pending_function_failure_facts(
        function_body_by_name(&ast, &path_fork, &string_table, "add"),
        &TemplateIrStore::new(),
        &ast.expressions,
    )
    .expect("resolved U32 add facts");
    assert_eq!(add.body.implicit.len(), 1);
    assert_eq!(add.body.implicit[0].codes, &[BuiltinErrorCode::IntOverflow]);
    assert_eq!(
        add.body.summary.summary.first_numeric, add.body.summary.summary.first_implicit,
        "the U32 add witness is the first implicit witness"
    );

    let floor_div = pending_function_failure_facts(
        function_body_by_name(&ast, &path_fork, &string_table, "divide"),
        &TemplateIrStore::new(),
        &ast.expressions,
    )
    .expect("resolved U32 division facts");
    assert_eq!(floor_div.body.implicit.len(), 1);
    assert_eq!(
        floor_div.body.implicit[0].codes,
        &[
            BuiltinErrorCode::DivideByZero,
            BuiltinErrorCode::IntOverflow
        ],
    );

    let power = pending_function_failure_facts(
        function_body_by_name(&ast, &path_fork, &string_table, "power"),
        &TemplateIrStore::new(),
        &ast.expressions,
    )
    .expect("resolved U32 power facts");
    assert_eq!(power.body.implicit.len(), 1);
    assert_eq!(
        power.body.implicit[0].codes,
        &[
            BuiltinErrorCode::IntOverflow,
            BuiltinErrorCode::InvalidExponent
        ],
    );
}

#[test]
fn u32_i32_comparison_records_no_failure_witness() {
    // Exact U32/I32 comparisons convert nothing, so they carry no
    // failure edge even though both sides are checked integer domains.
    let source = "compare |value U32, limit I32| -> Bool:\n    return value > limit\n;\n";
    let (ast, path_fork, string_table) = parse_single_file_ast(source);
    let body = function_body_by_name(&ast, &path_fork, &string_table, "compare");
    let expression = returned_value(body, &ast.expressions);
    assert!(failure_facts(expression).implicit.is_empty());
    assert!(failure_facts(expression).summary.first_numeric.is_none());
    assert!(!failure_facts(expression).checked_numeric_operation);
}

#[cfg(feature = "benchmark_counters")]
#[test]
fn generic_failure_chain_classification_visits_are_linear_in_authored_bodies() {
    use crate::compiler_frontend::instrumentation::{
        AstCounter, lock_counter_test, reset_ast_counters, test_read_ast_counter,
    };
    let _guard = lock_counter_test();
    let mut measurements = Vec::new();
    for count in [8, 32, 128] {
        for unrelated_size in [1, 64] {
            for reverse in [false, true] {
                let mut source =
                    String::from("identity type T |value T| -> T:\n    return value\n;\n");
                for position in 0..count {
                    let index = if reverse {
                        count - position - 1
                    } else {
                        position
                    };
                    let result = if index + 1 == count {
                        String::from("value + 1")
                    } else {
                        format!("step_{}(value)", index + 1)
                    };
                    source.push_str(&format!(
                        "step_{index} |value I32| -> I32:\n    return {result}\n;\n"
                    ));
                }
                source.push_str("unrelated |value I32| -> I32:\n");
                for index in 0..unrelated_size {
                    source.push_str(&format!("    local_{index} = value\n"));
                }
                source.push_str("    return value\n;\n");
                reset_ast_counters();
                let (ast, path_fork, string_table) = parse_single_file_ast(&source);
                let production_bodies =
                    test_read_ast_counter(AstCounter::FailureSummaryBodiesClassified);
                let production_nodes =
                    test_read_ast_counter(AstCounter::FailureSummaryBodyNodesClassified);
                measurements.push((
                    count,
                    unrelated_size,
                    reverse,
                    production_bodies,
                    production_nodes,
                ));
                let summaries = AstBuiltinFailureSummaries::compute(
                    &[&ast.nodes],
                    &TemplateIrStore::new(),
                    &ast.expressions,
                )
                .expect("chain summaries converge");
                let boundary = function_body_by_name(&ast, &path_fork, &string_table, "step_0");
                let pending = pending_function_failure_facts(
                    boundary,
                    &TemplateIrStore::new(),
                    &ast.expressions,
                )
                .expect("valid chain body");
                assert!(summaries.first_active(&pending.body.implicit).is_some());
            }
        }
    }
    eprintln!("failure convergence (chain, unrelated, reverse, bodies, nodes): {measurements:?}");
    for (count, unrelated_size, reverse, bodies, nodes) in measurements {
        assert!(
            bodies <= count + 3,
            "count={count}, reverse={reverse}: {bodies} bodies"
        );
        assert!(
            nodes <= count + unrelated_size + 3,
            "count={count}, unrelated={unrelated_size}, reverse={reverse}: {nodes} nodes"
        );
    }
}

#[test]
fn summary_convergence_keeps_nested_handler_origins_outside_protected_scopes() {
    let source = "identity type T |value T| -> T:\n    return value\n;\n\
        producer |value I32| -> I32:\n    return value + 1\n;\n\
        check |value I32| -> I32:\n\
            return producer(value) catch:\n\
                nested = producer(value) catch:\n            then value // value\n        ;\n\
                then nested\n    ;\n;\n";
    let (ast, path_fork, string_table) = parse_single_file_ast(source);
    let summaries = AstBuiltinFailureSummaries::compute(
        &[&ast.nodes],
        &TemplateIrStore::new(),
        &ast.expressions,
    )
    .expect("nested handler summaries converge");
    let NodeKind::Function(path, ..) =
        function_node_by_name(&ast, &path_fork, &string_table, "check").kind
    else {
        unreachable!("the named fixture is a function");
    };
    let facts = summaries
        .function_facts(path)
        .expect("check has closed facts");
    assert_eq!(facts.body.implicit.len(), 1);
    let contributor = summaries
        .first_active(&facts.body.implicit)
        .expect("the nested handler division escapes");
    assert_eq!(contributor.source, ImplicitFailureSource::NumericOperation);
    assert_eq!(
        contributor.codes,
        &[
            BuiltinErrorCode::DivideByZero,
            BuiltinErrorCode::IntOverflow
        ]
    );
    assert!(summaries.witness(contributor).call_spans.is_empty());
}

#[test]
fn infallible_protected_work_retains_validation_without_executable_handler_failure() {
    let source = "identity type T |value T| -> T:\n    return value\n;\n\
        producer |value I8| -> I8:\n    return value + 1\n;\n\
        check |unsigned U8, signed I8| -> I16:\n\
            return unsigned + signed catch:\n\
                fallback I16 = cast producer(127)\n        then fallback\n    ;\n;\n";
    let (ast, path_fork, string_table) = parse_single_file_ast(source);
    let body = function_body_by_name(&ast, &path_fork, &string_table, "check");
    let expression = returned_value(body, &ast.expressions);
    assert!(
        matches!(
            expression.operation(),
            ExpressionOperationView::ValueBlock(_)
        ),
        "validated recovery remains present despite its provisional empty failure summary"
    );
    let pending = pending_function_failure_facts(body, &TemplateIrStore::new(), &ast.expressions)
        .expect("the non-executable handler remains valid");
    assert!(pending.body.implicit.is_empty());
    let summaries = AstBuiltinFailureSummaries::compute(
        &[&ast.nodes],
        &TemplateIrStore::new(),
        &ast.expressions,
    )
    .expect("infallible protected work converges");
    let NodeKind::Function(path, ..) =
        function_node_by_name(&ast, &path_fork, &string_table, "check").kind
    else {
        unreachable!("the named fixture is a function");
    };
    let closed = summaries
        .function_facts(path)
        .expect("check has closed facts");
    assert!(closed.body.implicit.is_empty());
}

#[cfg(feature = "benchmark_counters")]
#[test]
fn catch_dependencies_batch_activation_before_reclassifying_the_consumer() {
    use crate::compiler_frontend::instrumentation::{
        AstCounter, lock_counter_test, reset_ast_counters, test_read_ast_counter,
    };
    let _guard = lock_counter_test();
    for count in [8, 32, 128] {
        let mut source = String::from("identity type T |value T| -> T:\n    return value\n;\n");
        source.push_str(
            "check |value I32| -> Bool:\n\
                return step_0(value) is step_1(value) catch:\n\
                    then value // value > 0\n    ;\n;\n",
        );
        for index in 0..count {
            let result = if index + 1 == count {
                String::from("value + 1")
            } else {
                format!("step_{}(value)", index + 1)
            };
            source.push_str(&format!(
                "step_{index} |value I32| -> I32:\n    return {result}\n;\n"
            ));
        }
        let (ast, path_fork, string_table) = parse_single_file_ast(&source);
        reset_ast_counters();
        let summaries = AstBuiltinFailureSummaries::compute(
            &[&ast.nodes],
            &TemplateIrStore::new(),
            &ast.expressions,
        )
        .expect("dependent catch summaries converge");
        let visits = test_read_ast_counter(AstCounter::FailureSummaryBodiesClassified);
        assert!(
            visits <= ast.nodes.len() + 1,
            "the two protected dependencies share one batched reclassification: {visits}"
        );
        let NodeKind::Function(path, ..) =
            function_node_by_name(&ast, &path_fork, &string_table, "check").kind
        else {
            unreachable!("the named fixture is a function");
        };
        let facts = summaries
            .function_facts(path)
            .expect("check has closed facts");
        let contributor = summaries
            .first_active(&facts.body.implicit)
            .expect("the handler becomes executable after the chain activates");
        assert_eq!(contributor.source, ImplicitFailureSource::NumericOperation);
        eprintln!(
            "batched catch chain={count}: bodies={visits}, retained={}",
            ast.nodes.len()
        );
    }
}

#[test]
fn propagated_map_remove_consumes_own_typed_error_summary() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "remove_value |scores ~{String = String}| -> String, Error!:\n    return ~scores.remove(\"key\")!\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "remove_value");
    let expression = returned_value(body, &ast.expressions);
    assert_eq!(expression.error_type(), None);
    assert!(failure_facts(expression).summary.first_typed.is_none());
    assert!(failure_facts(expression).postfix_exit_span.is_some());
    let ExpressionOperationView::HandleFallible { value, .. } = expression.operation() else {
        panic!("expected explicitly propagated builtin");
    };
    assert!(matches!(
        value.operation(),
        ExpressionOperationView::MapBuiltin { .. }
    ));
    assert!(value.error_type().is_some());
    assert!(failure_facts(value).typed_error.is_some());
}

#[test]
fn classifier_walks_a_4096_deep_runtime_arithmetic_chain_on_a_1_mib_stack() {
    use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
    use crate::compiler_frontend::ast::expressions::eval_expression::result_type::resolved_operation;
    use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
    use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
    use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
    use crate::compiler_frontend::symbols::path_interner::PathId;
    use crate::compiler_frontend::value_mode::ValueMode;
    use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

    // WHY: every row is one nesting level of the completed tree, so a recursive walk must
    // overflow a 1 MiB stack at this depth in debug builds.
    const DEPTH: usize = 4096;

    let worker = std::thread::Builder::new()
        .name("deep-arithmetic-classification".to_owned())
        .stack_size(1024 * 1024)
        .spawn(|| {
            let u64_type = builtin_type_ids::fixed_scalar(FixedScalar::U64);
            let type_environment = TypeEnvironment::new();
            let Ok(addition) = resolved_operation(
                Operator::Add,
                None,
                u64_type,
                u64_type,
                Some(u64_type),
                &type_environment,
            ) else {
                panic!("the U64 addition resolves");
            };
            let mut constants = ConstValueStore::default();
            let mut builder = AstExpressionBuilder::new().unwrap();
            let mut root = builder
                .reference_with_type_id(
                    PathId::ROOT,
                    u64_type,
                    None,
                    ValueMode::ImmutableOwned,
                    ConstRecordState::RuntimeValue,
                )
                .expect("the runtime leaf is complete");
            for _ in 0..DEPTH {
                let one = builder
                    .fixed_scalar(
                        &mut constants,
                        FixedScalarValue::unsigned(FixedScalar::U64, 1).expect("U64 one fits"),
                        None,
                        ValueMode::ImmutableOwned,
                    )
                    .expect("the literal operand is complete");
                root = builder
                    .resolved_operation(root, Some(one), addition, ValueMode::ImmutableOwned)
                    .expect("the addition row is complete");
            }
            let classified = pending_expression_failure_facts(
                builder
                    .view_owned(root)
                    .expect("the root belongs to the builder"),
                &TemplateIrStore::new(),
                &builder,
            )
            .expect("the deep chain classifies");
            assert!(classified.summary.first_implicit.is_some());
        })
        .expect("the classification thread starts");
    worker
        .join()
        .expect("the classification thread completes without overflowing its stack");
}
