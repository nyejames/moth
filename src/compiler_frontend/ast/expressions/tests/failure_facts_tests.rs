//! Invariants for expression failure side data and recovered contributor boundaries.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::assertion_message_effects::{
    assert_message_escape_diagnostic, classify_assertion_message_effect,
};
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::failure_classification::{
    AstBuiltinFailureSummaries, EnclosingExitEffect, pending_expression_failure_facts,
    pending_function_failure_facts,
};
use crate::compiler_frontend::ast::expressions::failure_facts::{
    ExpressionFailureFacts, FailureDisposition, ImplicitFailureSource,
};
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::compiler_messages::{
    BuiltinFailureOriginKind, DiagnosticPayload, InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::tests::ast_fixture_support::function_body_by_name;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;

fn returned_value(body: &[AstNode]) -> &Expression {
    body.iter()
        .find_map(|node| match &node.kind {
            NodeKind::Return(values) => match values.as_slice() {
                [value] => Some(value),
                _ => None,
            },
            _ => None,
        })
        .expect("the fixture returns exactly one value")
}

fn call_arguments(expression: &Expression) -> &[CallArgument] {
    match &expression.kind {
        ExpressionKind::FunctionCall { args, .. }
        | ExpressionKind::HandledFallibleFunctionCall { args, .. } => args,
        _ => panic!("the fixture contains an ordinary function call"),
    }
}

#[test]
fn arithmetic_codes_exclude_boundary_and_format_invariants() {
    let mut facts = ExpressionFailureFacts::default();
    facts.record_numeric_operation(
        NumericOperator::Power,
        NumericScalar::Int,
        Some(NumericScalar::Int),
        NumericScalar::Int,
        None,
    );
    assert_eq!(
        facts.implicit[0].codes,
        &[
            BuiltinErrorCode::IntOverflow,
            BuiltinErrorCode::InvalidExponent
        ],
    );

    let mut remainder = ExpressionFailureFacts::default();
    remainder.record_numeric_operation(
        NumericOperator::Remainder,
        NumericScalar::Int,
        Some(NumericScalar::Int),
        NumericScalar::Int,
        None,
    );
    assert_eq!(
        remainder.implicit[0].codes,
        &[BuiltinErrorCode::DivideByZero]
    );

    let mut float = ExpressionFailureFacts::default();
    float.record_numeric_operation(
        NumericOperator::Divide,
        NumericScalar::Float,
        Some(NumericScalar::Float),
        NumericScalar::Float,
        None,
    );
    assert_eq!(
        float.implicit[0].codes,
        &[
            BuiltinErrorCode::DivideByZero,
            BuiltinErrorCode::FloatNonFinite
        ],
    );
}

#[test]
fn call_keeps_argument_failure_without_changing_success_type() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "format_count |count Int| -> String:\n    return \"count\"\n;\n\
         check |value Int| -> String:\n    return format_count(value + 1)\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "check");
    let expression = returned_value(body);
    let argument = &call_arguments(expression)[0].value;
    let numeric_span = argument
        .failure_facts
        .summary
        .first_numeric
        .expect("the argument contains checked arithmetic")
        .span;

    assert_eq!(expression.type_id, builtin_type_ids::STRING);
    assert_eq!(
        expression
            .failure_facts
            .summary
            .first_numeric
            .expect("the call retains the argument's failure classification")
            .span,
        numeric_span,
    );
    assert_eq!(
        classify_assertion_message_effect(expression, &TemplateIrStore::new()).expect("valid AST"),
        Some(EnclosingExitEffect::InferredFailure(numeric_span)),
    );
}

#[test]
fn nested_calls_retain_one_local_witness_per_producer_in_postorder() {
    const PRODUCER_COUNT: usize = 16;
    let mut source = String::new();
    for index in 0..PRODUCER_COUNT {
        source.push_str(&format!(
            "producer_{index} |value Int| -> Int:\n    return value + 1\n;\n",
        ));
    }
    source.push_str("check || -> Int:\n    return ");
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
    let expression = returned_value(body);
    let mut current = expression;
    let mut stored_witnesses = 0;
    let mut producer_sites = Vec::new();
    while let ExpressionKind::FunctionCall { name, args, .. } = &current.kind {
        assert_eq!(current.failure_facts.implicit.len(), 1);
        let witness = &current.failure_facts.implicit[0];
        assert_eq!(witness.span, current.span);
        assert_eq!(witness.source, ImplicitFailureSource::PrivateCall(*name));
        assert!(witness.codes.is_empty());
        assert!(current.span.is_some());
        assert!(!producer_sites.iter().any(|(span, source)| {
            *span == current.span || *source == ImplicitFailureSource::PrivateCall(*name)
        }));

        stored_witnesses += current.failure_facts.implicit.len();
        producer_sites.push((current.span, ImplicitFailureSource::PrivateCall(*name)));
        assert_eq!(args.len(), 1);
        current = &args[0].value;
    }
    stored_witnesses += current.failure_facts.implicit.len();
    assert_eq!(producer_sites.len(), PRODUCER_COUNT);
    assert_eq!(stored_witnesses, PRODUCER_COUNT);
    producer_sites.reverse();

    let pending = pending_function_failure_facts(body, &TemplateIrStore::new()).expect("valid AST");
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

    let compact =
        pending_expression_failure_facts(expression, &TemplateIrStore::new()).expect("valid AST");
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
fn catch_handler_failure_goes_outward_without_reentering_protected_work() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "protected |value Int| -> Int:\n    return value + 1\n;\n\
         check |value Int| -> Int:\n\
             return protected(value + 1) catch:\n        then value // value\n    ;\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "check");
    let expression = returned_value(body);
    let ExpressionKind::ValueBlock { block } = &expression.kind else {
        panic!("the fixture returns a catch value block");
    };
    let ValueBlock::Catch(catch) = block.as_ref() else {
        panic!("the fixture contains a catch");
    };
    let protected_span = catch.handled_value.span;
    let protected_numeric_span = catch
        .handled_value
        .failure_facts
        .summary
        .first_numeric
        .expect("the protected argument contains checked arithmetic")
        .span;
    assert!(
        catch
            .handled_value
            .failure_facts
            .summary
            .first_private_call
            .is_some()
    );

    let pending = pending_function_failure_facts(body, &TemplateIrStore::new()).expect("valid AST");
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

    let protected = pending_expression_failure_facts(&catch.handled_value, &TemplateIrStore::new())
        .expect("valid AST");
    assert!(protected.summary.first_implicit.is_none());
    let outward =
        pending_expression_failure_facts(expression, &TemplateIrStore::new()).expect("valid AST");
    assert_eq!(
        outward
            .summary
            .first_numeric
            .expect("handler failure escapes")
            .span,
        handler_witness.span,
    );
    assert_eq!(
        classify_assertion_message_effect(expression, &TemplateIrStore::new()).expect("valid AST"),
        Some(EnclosingExitEffect::InferredFailure(handler_witness.span)),
    );
}

#[test]
fn first_numeric_and_typed_diagnostics_keep_descendant_producer_spans() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "typed_first |value Int| -> Int, Error!:\n    return value\n;\n\
         typed_second |value Int| -> Int, Error!:\n    return value\n;\n\
         check |value Int| -> Int:\n\
             return typed_second(typed_first(value + 1)) + typed_second(value * 2) catch then 0\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "check");
    let ExpressionKind::ValueBlock { block } = &returned_value(body).kind else {
        panic!("the fixture returns a catch value block");
    };
    let ValueBlock::Catch(catch) = block.as_ref() else {
        panic!("the fixture contains a catch");
    };
    let protected = &catch.handled_value;
    let ExpressionKind::Runtime(rpn) = &protected.kind else {
        panic!("the protected expression adds two typed call results");
    };
    let calls: Vec<_> = rpn
        .items
        .iter()
        .filter_map(|item| match item {
            ExpressionRpnItem::Operand(value) => Some(value),
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 2);
    let first_outer_call = calls[0];
    let first_inner_call = &call_arguments(first_outer_call)[0].value;
    let first_typed = first_inner_call
        .failure_facts
        .typed_error
        .expect("the inner call produces builtin Error");
    let first_numeric = call_arguments(first_inner_call)[0]
        .value
        .failure_facts
        .summary
        .first_numeric
        .expect("the inner argument contains checked arithmetic");
    let later_numeric = call_arguments(calls[1])[0]
        .value
        .failure_facts
        .summary
        .first_numeric
        .expect("the later argument also contains checked arithmetic");
    assert!(first_typed.span.is_some());
    assert!(first_numeric.span.is_some());
    assert_ne!(first_typed.span, first_outer_call.span);
    assert_ne!(first_numeric.span, later_numeric.span);
    assert_eq!(
        protected.failure_facts.summary.first_typed,
        Some(first_typed)
    );
    assert_eq!(
        protected.failure_facts.summary.first_numeric,
        Some(first_numeric),
    );
    assert!(protected.failure_facts.summary.conflicting_typed.is_none());

    // Inspect the same typed work before the receiving catch discharges it. Numeric and typed
    // diagnostics use different first witnesses, neither the enclosing call nor the catch span.
    let mut pending = protected.as_ref().clone();
    pending.failure_facts.disposition = FailureDisposition::Pending;
    let typed_diagnostic = pending
        .unhandled_typed_error_diagnostic()
        .expect("the pending expression has an unhandled typed producer");
    assert_eq!(typed_diagnostic.primary_span, first_typed.span);
    assert!(matches!(
        typed_diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling {
            reason: InvalidFallibleHandlingReason::UnhandledErrorReturn,
        }
    ));
    let numeric_diagnostic = assert_message_escape_diagnostic(&pending, &TemplateIrStore::new())
        .expect("valid AST")
        .expect("checked numeric work can escape message evaluation");
    assert_eq!(numeric_diagnostic.primary_span, first_numeric.span);
    assert!(matches!(
        numeric_diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling {
            reason: InvalidFallibleHandlingReason::AssertionMessageCannotEscape,
        }
    ));
}

#[test]
fn failure_witness_prefers_only_the_same_compound_updates_writeback_through_private_hops() {
    // The earlier statement must stay genuinely fallible so the first-origin branch keeps its
    // intent: U8 + U8 is discharged (promoted to U32), so the fixture uses U32 + U32 here.
    for earlier in ["", "    earlier = wide + one\n"] {
        let (ast, path_fork, string_table) = parse_single_file_ast(&format!(
            "risky |amount U8| -> U8, Error!:\n    return amount\n;\n\
             bump |wide U32, one U32, amount U8| -> U8:\n\
             {earlier}    total ~U8 = 250\n\
                 total += risky(amount) catch then amount\n    return total\n;\n\
             forward |wide U32, one U32, amount U8| -> U8:\n    return bump(wide, one, amount)\n;\n"
        ));
        let summaries = AstBuiltinFailureSummaries::compute(&[&ast.nodes], &TemplateIrStore::new())
            .expect("fixture summaries converge");
        let bump = function_body_by_name(&ast, &path_fork, &string_table, "bump");
        let facts = pending_function_failure_facts(bump, &TemplateIrStore::new()).unwrap();
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
        let facts = pending_function_failure_facts(forward, &TemplateIrStore::new()).unwrap();
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
    //      of stopping with call-only hops and empty cause codes.
    for source in [
        "helper |value Int, ready Bool| -> Int:\n\
             if ready:\n        return helper(value, ready)\n    ;\n    return value * 2\n;\n\
         boundary |value Int, ready Bool| -> Int:\n    return helper(value, ready)\n;\n",
        "first |value Int, ready Bool| -> Int:\n\
             if ready:\n        return second(value, ready)\n    ;\n    return value * 2\n;\n\
         second |value Int, ready Bool| -> Int:\n    return first(value, ready)\n;\n\
         boundary |value Int, ready Bool| -> Int:\n    return first(value, ready)\n;\n",
    ] {
        let (ast, path_fork, string_table) = parse_single_file_ast(source);
        let summaries = AstBuiltinFailureSummaries::compute(&[&ast.nodes], &TemplateIrStore::new())
            .expect("fixture summaries converge");
        let boundary = function_body_by_name(&ast, &path_fork, &string_table, "boundary");
        let facts = pending_function_failure_facts(boundary, &TemplateIrStore::new()).unwrap();
        let entry = summaries
            .first_active(&facts.body.implicit)
            .expect("the boundary escapes through its helper");
        let witness = summaries.witness(entry);
        assert_eq!(witness.codes, vec![BuiltinErrorCode::IntOverflow]);
        assert_eq!(witness.origin, BuiltinFailureOriginKind::Operation);

        // The origin is the actual multiplication in the helper, never the skipped call.
        let helper = if source.starts_with("first") {
            "first"
        } else {
            "helper"
        };
        let helper_body = function_body_by_name(&ast, &path_fork, &string_table, helper);
        let helper_facts =
            pending_function_failure_facts(helper_body, &TemplateIrStore::new()).unwrap();
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
        assert_eq!(witness.origin_span, numeric_span);
        assert!(!witness.call_spans.contains(&recursive_span));
        assert_eq!(witness.elided_call_hops, 0);

        // Repeated runs over the same summaries produce the same structured witness.
        assert_eq!(summaries.witness(entry), witness);
    }
}

#[test]
fn origin_free_recursion_stays_free_of_recoverable_numeric_failure() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "recurse |value Int| -> Int:\n    return recurse(value)\n;\n\
         boundary |value Int| -> Int:\n    return recurse(value)\n;\n",
    );
    let summaries = AstBuiltinFailureSummaries::compute(&[&ast.nodes], &TemplateIrStore::new())
        .expect("fixture summaries converge");
    let boundary = function_body_by_name(&ast, &path_fork, &string_table, "boundary");
    let facts = pending_function_failure_facts(boundary, &TemplateIrStore::new()).unwrap();
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
         helper |value Int| -> Int:\n    return deferred(value) + value * 2\n;\n\
         boundary |value Int| -> Int:\n    return helper(value)\n;\n",
    );
    let summaries = AstBuiltinFailureSummaries::compute(&[&ast.nodes], &TemplateIrStore::new())
        .expect("fixture summaries converge");
    let helper = function_body_by_name(&ast, &path_fork, &string_table, "helper");
    let facts = pending_function_failure_facts(helper, &TemplateIrStore::new()).unwrap();
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
    assert_eq!(witness.codes, Vec::<BuiltinErrorCode>::new());
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
        "risky |amount U8| -> U8, Error!:\n    return amount\n;\n\
         helper |wide U32, one U32, amount U8, ready Bool| -> U8:\n\
             if ready:\n        return helper(wide, one, amount, ready)\n    ;\n\
             total ~U8 = 250\n    total += risky(amount) catch then amount\n    return total\n;\n\
         boundary |wide U32, one U32, amount U8, ready Bool| -> U8:\n\
             return helper(wide, one, amount, ready)\n;\n",
    );
    let summaries = AstBuiltinFailureSummaries::compute(&[&ast.nodes], &TemplateIrStore::new())
        .expect("fixture summaries converge");
    let boundary = function_body_by_name(&ast, &path_fork, &string_table, "boundary");
    let facts = pending_function_failure_facts(boundary, &TemplateIrStore::new()).unwrap();
    let entry = summaries
        .first_active(&facts.body.implicit)
        .expect("the boundary escapes through its helper");
    let witness = summaries.witness(entry);
    let helper = function_body_by_name(&ast, &path_fork, &string_table, "helper");
    let helper_facts = pending_function_failure_facts(helper, &TemplateIrStore::new()).unwrap();
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
