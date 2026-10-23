//! Collection literal parsing regression tests.
//!
//! WHAT: validates collection item parsing and unterminated-literal diagnostics.
//! WHY: collections use expression recursion and can silently drift without focused coverage.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::expressions::expression::FallibleExpressionHandling;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::expressions::failure_classification::pending_expression_failure_facts;
use crate::compiler_frontend::ast::expressions::failure_facts::FailureDisposition;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::builtins::CollectionBuiltinOp;
use crate::compiler_frontend::builtins::maps::MapBuiltinOp;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    calls_data::{InvalidBuiltinCallView, InvalidValueReceiverView},
    types_borrows_targets::TypeMismatchView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, InvalidBuiltinCallReason, TypeMismatchContext, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::display::display_type_with_resolver;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::path_interner::PathInternerBuilder;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, start_function_body,
};
use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, parse_single_file_ast, parse_single_file_ast_diagnostic,
};
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

/// One exact `I32` scalar value for collection-item fixtures.
///
/// WHY: an unconstrained whole literal defaults to `I32`, so completed collection items carry
///      the exact `I32` identity.
fn i32_value(value: i64) -> FixedScalarValue {
    FixedScalarValue::signed(FixedScalar::I32, value).expect("test literal fits I32")
}

#[test]
fn rejects_non_single_collection_and_map_entries() {
    let producers = "pair || -> I32, I32:\nreturn 1, 2\n;\nempty ||:\nreturn\n;\n";
    for (entry, receiver, produced_count) in [
        (
            "values = {pair()}\n",
            ValueReceiverContext::CollectionElement,
            2,
        ),
        (
            "values = {empty()}\n",
            ValueReceiverContext::CollectionElement,
            0,
        ),
        ("values = {pair() = 1}\n", ValueReceiverContext::MapKey, 2),
        ("values = {empty() = 1}\n", ValueReceiverContext::MapKey, 0),
        ("values = {1 = pair()}\n", ValueReceiverContext::MapValue, 2),
        (
            "values = {1 = empty()}\n",
            ValueReceiverContext::MapValue,
            0,
        ),
    ] {
        let source = format!("{producers}{entry}");
        let failure = parse_single_file_ast_diagnostic(&source);
        let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0087", None);
        let facts = diagnostic
            .family_view::<InvalidValueReceiverView>()
            .expect("the diagnostic is an invalid value receiver");
        assert_eq!(facts.receiver, receiver, "{source}");
        assert_eq!(facts.target_count, 1, "{source}");
        assert_eq!(facts.produced_count, produced_count, "{source}");
    }
}

fn expression_view(ast: &Ast, expression: ExprId) -> ExpressionView<'_> {
    ast.expressions
        .view(expression)
        .expect("fixture expression view")
}

fn constant_payload(ast: &Ast, expression: ExprId) -> Option<&ConstValuePayload> {
    let view = expression_view(ast, expression);
    let ExpressionOperationView::Constant(constant_id) = view.operation() else {
        return None;
    };
    ast.const_values.payload(constant_id)
}

fn fixed_scalar_constant(ast: &Ast, expression: ExprId) -> Option<FixedScalarValue> {
    match constant_payload(ast, expression)? {
        ConstValuePayload::FixedScalar(value) => Some(*value),
        _ => None,
    }
}

fn collection_items(ast: &Ast, expression: ExprId) -> Vec<ExprId> {
    let view = expression_view(ast, expression);
    let ExpressionOperationView::Collection(items) = view.operation() else {
        panic!("expected collection expression");
    };
    items.iter().map(|item| item.id()).collect()
}

fn map_entries(ast: &Ast, expression: ExprId) -> Vec<(ExprId, ExprId)> {
    let view = expression_view(ast, expression);
    let ExpressionOperationView::Map(entries) = view.operation() else {
        panic!("expected map literal expression");
    };
    entries
        .iter()
        .map(|entry| (entry.key().id(), entry.value().id()))
        .collect()
}

/// Returns the runtime call under a `catch` or propagation handler.
fn handled_runtime_call(ast: &Ast, expression: ExprId) -> ExprId {
    let view = expression_view(ast, expression);
    match view.operation() {
        ExpressionOperationView::HandleFallible { value, .. } => value.id(),
        ExpressionOperationView::ValueBlock(block) => {
            let ValueBlock::Catch(value_catch) = block else {
                panic!("expected catch value block");
            };
            let inner = expression_view(ast, value_catch.handled_value);
            let ExpressionOperationView::HandleFallible { value, .. } = inner.operation() else {
                panic!("expected handled fallible expression inside catch value block");
            };
            value.id()
        }
        _ => panic!("expected handled fallible expression"),
    }
}

fn runtime_collection_builtin_op(ast: &Ast, expression: ExprId) -> CollectionBuiltinOp {
    let view = expression_view(ast, expression);
    let ExpressionOperationView::CollectionBuiltin { facts, .. } = view.operation() else {
        panic!("expected collection builtin call expression");
    };
    facts.operation
}

fn handled_collection_builtin_op(ast: &Ast, expression: ExprId) -> CollectionBuiltinOp {
    runtime_collection_builtin_op(ast, handled_runtime_call(ast, expression))
}

// --------------------------
//  Collection literals
// --------------------------

#[test]
fn parses_collection_literal_items() {
    let (ast, path_fork, string_table) = parse_single_file_ast("values ~= {1, 2, 3}\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(values_decl) = &body[0].kind else {
        panic!("expected collection declaration");
    };

    let items = collection_items(
        &ast,
        values_decl
            .value
            .initializer()
            .expect("fixture initializer"),
    );

    assert_eq!(items.len(), 3);
    assert!(matches!(fixed_scalar_constant(&ast, items[0]), Some(value) if value == i32_value(1)));
    assert!(matches!(fixed_scalar_constant(&ast, items[1]), Some(value) if value == i32_value(2)));
    assert!(matches!(fixed_scalar_constant(&ast, items[2]), Some(value) if value == i32_value(3)));
}

#[test]
fn parses_multiline_inferred_collection_literal_with_constructor_items() {
    let source = "Entry = |\n    value I32,\n|\n\nrender || -> String:\n    entries = {\n        Entry(1),\n        Entry(2),\n    }\n    return [entries.length()]\n;\n";
    let (ast, path_fork, string_table) = parse_single_file_ast(source);
    let body = function_body_by_name(&ast, &path_fork, &string_table, "render");

    let NodeKind::VariableDeclaration(entries_decl) = &body[0].kind else {
        panic!("expected collection declaration");
    };

    let items = collection_items(
        &ast,
        entries_decl
            .value
            .initializer()
            .expect("fixture initializer"),
    );

    assert_eq!(items.len(), 2);
    assert_eq!(
        display_type_with_resolver(
            entries_decl.type_id,
            &ast.type_environment,
            &string_table,
            &path_fork.snapshot_table(),
        ),
        "{Entry}"
    );
}

#[test]
fn infers_collection_element_type_from_non_empty_literal() {
    let (ast, path_fork, string_table) = parse_single_file_ast("values ~= {1, 2, 3}\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(values_decl) = &body[0].kind else {
        panic!("expected values declaration");
    };

    assert_eq!(
        display_type_with_resolver(
            values_decl.type_id,
            &ast.type_environment,
            &string_table,
            &PathInternerBuilder::new().freeze(),
        ),
        "{I32}"
    );
}

#[test]
fn parses_empty_collection_with_explicit_element_type() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("Reading = |\n    value F64,\n|\n\nreadings ~{Reading} = {}\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(readings_decl) = &body[0].kind else {
        panic!("expected readings declaration");
    };

    assert_eq!(
        display_type_with_resolver(
            readings_decl.type_id,
            &ast.type_environment,
            &string_table,
            &path_fork.snapshot_table(),
        ),
        "{Reading}"
    );
}

#[test]
fn rejects_mixed_item_types_in_inferred_collection() {
    let failure = parse_single_file_ast_diagnostic("values ~= {1, \"bad\"}\n");
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-TYPE-0001", None);
    let facts = diagnostic
        .family_view::<TypeMismatchView>()
        .expect("the diagnostic is a type mismatch");
    assert_eq!(facts.context, TypeMismatchContext::CollectionElement);
}

#[test]
fn rejects_item_that_does_not_match_explicit_collection_type() {
    let failure = parse_single_file_ast_diagnostic("values ~{I32} = {1, \"bad\"}\n");
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-TYPE-0001", None);
    let facts = diagnostic
        .family_view::<TypeMismatchView>()
        .expect("the diagnostic is a type mismatch");
    assert_eq!(facts.context, TypeMismatchContext::CollectionElement);
}

#[test]
fn parses_growable_push_after_explicit_empty_collection() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("values ~{I32} = {}\n~values.push(1)\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::ExpressionStatement(push_expr) = &body[1].kind else {
        panic!("expected push statement");
    };

    assert_eq!(
        runtime_collection_builtin_op(&ast, *push_expr),
        CollectionBuiltinOp::PushGrowable
    );
}

#[test]
fn rejects_empty_collection_even_when_later_push_would_reveal_type() {
    let diagnostic = parse_single_file_ast_diagnostic("values ~= {}\n~values.push(1)\n");

    assert_ast_diagnostic(&diagnostic, "MOTH-TYPE-0002", None);
}

#[test]
fn rejects_missing_collection_item_after_comma() {
    let diagnostic = parse_single_file_ast_diagnostic("values ~= {1, , 2}\n");

    assert_ast_diagnostic(&diagnostic, "MOTH-SYNTAX-0027", None);
}

// --------------------------
//  Fallible handling on collections
// --------------------------

#[test]
fn parses_collection_get_with_fallback_handler_and_propagation() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "read_or_default |values {I32}, idx I32| -> I32:\n    return values.get(idx) catch:\n        then 0\n    ;\n;\n\nread_with_handler |values {I32}, idx I32| -> I32:\n    return values.get(idx) catch |err|:\n        io.line([: [err.message]])\n        then 0\n    ;\n;\n\nforward_read |values {I32}, idx I32| -> I32, Error!:\n    return values.get(idx)!\n;\n",
    );

    let fallback_body = function_body_by_name(&ast, &path_fork, &string_table, "read_or_default");
    let NodeKind::Return(values) = &fallback_body[0].kind else {
        panic!("expected return statement in fallback function");
    };
    assert!(matches!(
        expression_view(&ast, values[0]).operation(),
        ExpressionOperationView::ValueBlock(_)
    ));

    let handler_body = function_body_by_name(&ast, &path_fork, &string_table, "read_with_handler");
    let NodeKind::Return(values) = &handler_body[0].kind else {
        panic!("expected return statement in handler function");
    };
    assert!(matches!(
        expression_view(&ast, values[0]).operation(),
        ExpressionOperationView::ValueBlock(_)
    ));

    let propagation_body = function_body_by_name(&ast, &path_fork, &string_table, "forward_read");
    let NodeKind::Return(values) = &propagation_body[0].kind else {
        panic!("expected return statement in propagation function");
    };
    let view = expression_view(&ast, values[0]);
    let ExpressionOperationView::HandleFallible { handling, .. } = view.operation() else {
        panic!("expected handled fallible expression for propagation");
    };
    assert!(matches!(
        handling.handling,
        FallibleExpressionHandling::Propagate
    ));
}

// --------------------------
//  Collection mutators and accessors
// --------------------------

#[test]
fn parses_collection_mutators_and_length_calls() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "values ~= {1, 2, 3}\n~values.set(1, 9) catch:\n;\n~values.push(4)\nremoved = ~values.remove(2) catch:\n    then 0\n;\nsize = values.length()\n",
    );
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::ExpressionStatement(set_expr) = &body[1].kind else {
        panic!("expected set(...) statement");
    };
    assert_eq!(
        handled_collection_builtin_op(&ast, *set_expr),
        CollectionBuiltinOp::Set
    );

    let NodeKind::ExpressionStatement(push_expr) = &body[2].kind else {
        panic!("expected push(...) statement");
    };
    assert_eq!(
        runtime_collection_builtin_op(&ast, *push_expr),
        CollectionBuiltinOp::PushGrowable
    );

    let NodeKind::VariableDeclaration(removed_decl) = &body[3].kind else {
        panic!("expected removed declaration");
    };
    assert_eq!(
        handled_collection_builtin_op(
            &ast,
            removed_decl
                .value
                .initializer()
                .expect("fixture initializer"),
        ),
        CollectionBuiltinOp::Remove
    );

    let NodeKind::VariableDeclaration(size_decl) = &body[4].kind else {
        panic!("expected length declaration");
    };
    assert_eq!(
        size_decl.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
    assert_eq!(
        runtime_collection_builtin_op(
            &ast,
            size_decl.value.initializer().expect("fixture initializer")
        ),
        CollectionBuiltinOp::Length
    );
}

#[test]
fn parses_collection_mutators_with_explicit_receiver_tilde_prefix() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "values ~= {1, 2, 3}\n~values.push(4)\n~values.set(1, 9) catch:\n;\nremoved = ~values.remove(2) catch:\n    then 0\n;\nsize = values.length()\n",
    );
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::ExpressionStatement(push_expr) = &body[1].kind else {
        panic!("expected push(...) statement");
    };
    assert_eq!(
        runtime_collection_builtin_op(&ast, *push_expr),
        CollectionBuiltinOp::PushGrowable
    );

    let NodeKind::ExpressionStatement(set_expr) = &body[2].kind else {
        panic!("expected set(...) statement");
    };
    assert_eq!(
        handled_collection_builtin_op(&ast, *set_expr),
        CollectionBuiltinOp::Set
    );

    let NodeKind::VariableDeclaration(removed_decl) = &body[3].kind else {
        panic!("expected removed declaration");
    };
    assert_eq!(
        handled_collection_builtin_op(
            &ast,
            removed_decl
                .value
                .initializer()
                .expect("fixture initializer"),
        ),
        CollectionBuiltinOp::Remove
    );
}

#[test]
fn accepts_unhandled_growable_collection_push() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("values ~= {1, 2, 3}\n~values.push(4)\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::ExpressionStatement(push_expr) = &body[1].kind else {
        panic!("expected push statement");
    };

    let view = expression_view(&ast, *push_expr);
    let ExpressionOperationView::CollectionBuiltin { facts, .. } = view.operation() else {
        panic!("expected collection builtin call");
    };

    assert_eq!(facts.operation, CollectionBuiltinOp::PushGrowable);
    assert!(view.success_types().is_empty());
}

#[test]
fn rejects_unhandled_fixed_collection_push_result() {
    let diagnostic = parse_single_file_ast_diagnostic("items ~{2 I32} = {1, 2}\n~items.push(3)\n");

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0046",
        Some("invalid_builtin_call.unhandled_fallible_call"),
    );
}

#[test]
fn rejects_catch_handler_on_growable_collection_push() {
    let diagnostic =
        parse_single_file_ast_diagnostic("values ~= {1, 2, 3}\n~values.push(4) catch:\n;\n");

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.catch_on_non_fallible"),
    );
}

#[test]
fn rejects_propagation_on_growable_collection_push() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "append || -> Error!:
    values ~= {1, 2, 3}
    ~values.push(4)!
;
",
    );

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.bang_on_non_fallible"),
    );
}

#[test]
fn parses_fixed_collection_push_with_fallback_handler() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("items ~{2 I32} = {1, 2}\n~items.push(3) catch:\n;\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::ExpressionStatement(push_expr) = &body[1].kind else {
        panic!("expected push statement");
    };

    assert_eq!(
        handled_collection_builtin_op(&ast, *push_expr),
        CollectionBuiltinOp::PushFixed
    );
}

#[test]
fn parses_fixed_collection_push_postfix_propagation() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "append || -> Error!:
        items ~{2 I32} = {1, 2}
        ~items.push(3)!
    ;
    ",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "append");

    let NodeKind::ExpressionStatement(push_expr) = &body[1].kind else {
        panic!("expected push statement");
    };

    assert_eq!(
        handled_collection_builtin_op(&ast, *push_expr),
        CollectionBuiltinOp::PushFixed
    );
}

#[test]
fn rejects_fixed_collection_push_fallback_value() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "items ~{2 I32} = {1, 2}
~items.push(3) catch:
    then 0
;
",
    );

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.fallback_values_for_error_only_result"),
    );
}

#[test]
fn rejects_mutating_collection_method_without_explicit_receiver_tilde() {
    let diagnostic = parse_single_file_ast_diagnostic("values ~= {1, 2, 3}\nvalues.push(4)\n");

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0047",
        Some("invalid_receiver_call.mutable_receiver_missing_marker"),
    );

    let fixed_diagnostic =
        parse_single_file_ast_diagnostic("items ~{2 I32} = {1, 2}\nitems.push(3)\n");

    assert_ast_diagnostic(
        &fixed_diagnostic,
        "MOTH-RULE-0047",
        Some("invalid_receiver_call.mutable_receiver_missing_marker"),
    );
}

#[test]
fn rejects_explicit_receiver_tilde_for_collection_get_and_length() {
    let get_diagnostic = parse_single_file_ast_diagnostic(
        "values ~= {1, 2, 3}\nvalue = ~values.get(0) catch:\n    then 0\n;\n",
    );
    assert_ast_diagnostic(
        &get_diagnostic,
        "MOTH-RULE-0047",
        Some("invalid_receiver_call.unneeded_mutable_access_marker"),
    );

    let length_diagnostic =
        parse_single_file_ast_diagnostic("values ~= {1, 2, 3}\nvalue = ~values.length()\n");
    assert_ast_diagnostic(
        &length_diagnostic,
        "MOTH-RULE-0047",
        Some("invalid_receiver_call.unneeded_mutable_access_marker"),
    );
}

#[test]
fn parses_multiline_zero_argument_collection_and_map_builtins() {
    let source = "values = {1, 2}\ncount = values.length(\n)\nscores ~{String = I32} = {\"Ada\" = 10}\n~scores.clear(\n)\n";

    parse_single_file_ast(source);
}

#[test]
fn preserves_zero_argument_builtin_extra_argument_diagnostic() {
    let diagnostic = parse_single_file_ast_diagnostic("values = {1, 2}\nvalues.length(1)\n");

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0046",
        Some("invalid_builtin_call.takes_no_arguments"),
    );
}

#[test]
fn rejects_pull_method_as_unknown_collection_member() {
    let diagnostic = parse_single_file_ast_diagnostic("values ~= {1, 2, 3}\nvalues.pull(1)\n");

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0048",
        Some("invalid_field_access.unknown_member"),
    );
}

#[test]
fn rejects_get_index_assignment_as_removed_syntax() {
    let diagnostic = parse_single_file_ast_diagnostic("values = {1, 2, 3}\nvalues.get(0) = 9\n");

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0044",
        Some("invalid_assignment_target.collection_get_target_not_writable"),
    );
}

#[test]
fn rejects_unhandled_collection_get_result() {
    let diagnostic =
        parse_single_file_ast_diagnostic("values ~= {1, 2, 3}\nvalue = values.get(0)\n");

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0046",
        Some("invalid_builtin_call.unhandled_fallible_call"),
    );
}

#[test]
fn preserves_nested_builtin_diagnostic_origin_under_whole_expression_catch() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "read |values {I32}, index I32| -> I32:\n    return values.get(index) + 1 catch then 0\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "read");
    let NodeKind::Return(values) = &body[0].kind else {
        panic!("expected return");
    };
    let whole = expression_view(&ast, values[0]);
    let ExpressionOperationView::ValueBlock(block) = whole.operation() else {
        panic!("expected whole-expression catch");
    };
    let ValueBlock::Catch(catch) = block else {
        panic!("expected catch value block");
    };

    assert!(whole.unhandled_typed_error_diagnostic().is_none());

    let protected = expression_view(&ast, catch.handled_value);
    let ExpressionOperationView::NumericBinary { left, .. } = protected.operation() else {
        panic!("expected protected addition");
    };
    let mut constants = ConstValueStore::default();
    let mut builder = AstExpressionBuilder::new().unwrap();
    let stand_in = builder
        .fixed_scalar(
            &mut constants,
            i32_value(0),
            protected.span(),
            protected.value_mode(),
        )
        .expect("fixture pending stand-in");
    let mut pending_facts = protected.failure_facts().cloned().unwrap_or_default();
    pending_facts.disposition = FailureDisposition::Pending;
    let pending = builder
        .with_failure_facts(stand_in, pending_facts)
        .expect("fixture pending failure facts");
    let pending = builder.view_owned(pending).expect("fixture pending view");
    let facts = pending_expression_failure_facts(pending, &TemplateIrStore::new(), &builder)
        .expect("pending failure facts for projected addition");
    assert_eq!(facts.disposition, FailureDisposition::Pending);
    let producer = facts
        .summary
        .first_typed
        .expect("unhandled nested get needs a typed error producer");
    let builtin_name = producer
        .builtin_name
        .expect("unhandled nested get names its builtin");
    assert_eq!(string_table.resolve(builtin_name), "get");
    assert_eq!(producer.span.or(protected.span()), left.span());
    let diagnostic = pending
        .unhandled_typed_error_diagnostic()
        .expect("unhandled nested get needs a builtin diagnostic");
    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = compact
        .first()
        .expect("the draft compacts to a diagnostic view");
    let facts = diagnostic
        .family_view::<InvalidBuiltinCallView>()
        .expect("unhandled nested get retains the invalid-builtin-call facts");
    assert_eq!(
        facts.reason,
        InvalidBuiltinCallReason::UnhandledFallibleCall
    );
    let diagnostic_builtin = facts
        .builtin_name
        .expect("unhandled nested get names its builtin");
    assert_eq!(string_table.resolve(diagnostic_builtin), "get");
    assert!(matches!(
        diagnostic.primary_site(),
        SiteRef::Local(span) if Some(span) == left.span()
    ));
}

#[test]
fn rejects_unhandled_collection_set_result() {
    let diagnostic = parse_single_file_ast_diagnostic("values ~= {1, 2, 3}\n~values.set(0, 9)\n");

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0046",
        Some("invalid_builtin_call.unhandled_fallible_call"),
    );
}

#[test]
fn rejects_collection_set_fallback_value() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "values ~= {1, 2, 3}\n~values.set(0, 9) catch:\n    then 0\n;\n",
    );

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.fallback_values_for_error_only_result"),
    );
}

#[test]
fn rejects_discarded_collection_remove_success() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "drop_removed || -> Error!:\n    values ~= {1, 2, 3}\n    ~values.remove(0)!\n;\n",
    );

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.success_value_discarded"),
    );
}

// --------------------------
//  Removed builtin error helpers
// --------------------------

#[test]
fn rejects_removed_builtin_error_helper_methods() {
    let diagnostic =
        parse_single_file_ast_diagnostic("err = Error(\"bad\")\nvalue = err.bubble()\n");

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0048",
        Some("invalid_field_access.unknown_member"),
    );
}

// --------------------------
//  Fixed collection literals
// --------------------------

#[test]
fn fixed_collection_literal_within_capacity_is_accepted() {
    let (ast, path_fork, string_table) = parse_single_file_ast("items {2 I32} = {1, 2}\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(decl) = &body[0].kind else {
        panic!("expected declaration");
    };

    assert_eq!(
        display_type_with_resolver(
            decl.type_id,
            &ast.type_environment,
            &string_table,
            &PathInternerBuilder::new().freeze(),
        ),
        "{2 I32}"
    );
}

// --------------------------
//  Map literals
// --------------------------

#[test]
fn parses_inferred_map_literal() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("scores ~= {\"Ada\" = 10, \"Grace\" = 12}\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(scores_decl) = &body[0].kind else {
        panic!("expected scores declaration");
    };

    let entries = map_entries(
        &ast,
        scores_decl
            .value
            .initializer()
            .expect("fixture initializer"),
    );

    assert_eq!(entries.len(), 2);
    assert!(
        ast.type_environment.is_map_type(scores_decl.type_id),
        "expected map type id"
    );
    assert_eq!(
        display_type_with_resolver(
            scores_decl.type_id,
            &ast.type_environment,
            &string_table,
            &PathInternerBuilder::new().freeze(),
        ),
        "{String = I32}"
    );
}

#[test]
fn parses_explicit_empty_map_literal() {
    let (ast, path_fork, string_table) = parse_single_file_ast("scores {String = I32} = {}\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(scores_decl) = &body[0].kind else {
        panic!("expected scores declaration");
    };

    let entries = map_entries(
        &ast,
        scores_decl
            .value
            .initializer()
            .expect("fixture initializer"),
    );

    assert!(entries.is_empty());
    assert_eq!(
        display_type_with_resolver(
            scores_decl.type_id,
            &ast.type_environment,
            &string_table,
            &PathInternerBuilder::new().freeze(),
        ),
        "{String = I32}"
    );
}

#[test]
fn parses_map_literal_with_runtime_key_expression() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "get_key || -> String:\n    return \"Ada\"\n;\n\nscores ~= {get_key() = 10}\n",
    );
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(scores_decl) = &body[0].kind else {
        panic!("expected scores declaration");
    };

    let entries = map_entries(
        &ast,
        scores_decl
            .value
            .initializer()
            .expect("fixture initializer"),
    );

    assert_eq!(entries.len(), 1);
    assert!(
        matches!(
            expression_view(&ast, entries[0].0).operation(),
            ExpressionOperationView::Call(_)
        ),
        "expected runtime key expression"
    );
}

#[test]
fn parses_map_literal_with_bare_identifier_key_as_variable() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("key = \"Ada\"\nscores ~= {key = 10}\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(scores_decl) = &body[1].kind else {
        panic!("expected scores declaration, got: {:?}", body[1].kind);
    };

    let entries = map_entries(
        &ast,
        scores_decl
            .value
            .initializer()
            .expect("fixture initializer"),
    );

    assert_eq!(entries.len(), 1);
    assert!(
        matches!(
            expression_view(&ast, entries[0].0).operation(),
            ExpressionOperationView::Reference(_)
        ),
        "expected bare identifier key to be parsed as variable reference"
    );
}

#[test]
fn parses_map_literal_with_contextual_none_value() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("scores ~{String = I32?} = {\"Ada\" = none, \"Grace\" = 12}\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(scores_decl) = &body[0].kind else {
        panic!("expected scores declaration");
    };

    let entries = map_entries(
        &ast,
        scores_decl
            .value
            .initializer()
            .expect("fixture initializer"),
    );

    assert_eq!(entries.len(), 2);
    assert!(
        matches!(
            constant_payload(&ast, entries[0].1),
            Some(ConstValuePayload::OptionNone)
        ),
        "expected none in value position with option context"
    );
}

#[test]
fn parses_map_literal_with_string_key_coercion() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("scores ~{String = I32} = {\"Ada\" = 10}\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(scores_decl) = &body[0].kind else {
        panic!("expected scores declaration");
    };

    let entries = map_entries(
        &ast,
        scores_decl
            .value
            .initializer()
            .expect("fixture initializer"),
    );

    assert_eq!(entries.len(), 1);
    assert!(
        matches!(
            constant_payload(&ast, entries[0].0),
            Some(ConstValuePayload::String(_))
        ),
        "expected string literal key"
    );
}

#[test]
fn parses_nested_map_literal_value() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("scores ~{String = {String = I32}} = {\"group\" = {\"Ada\" = 10}}\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(scores_decl) = &body[0].kind else {
        panic!("expected scores declaration");
    };

    let entries = map_entries(
        &ast,
        scores_decl
            .value
            .initializer()
            .expect("fixture initializer"),
    );

    assert_eq!(entries.len(), 1);
    assert!(
        matches!(
            expression_view(&ast, entries[0].1).operation(),
            ExpressionOperationView::Map(_)
        ),
        "expected nested map literal value"
    );
}

#[test]
fn parses_map_type_alias_literal() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("Scores as {String = I32}\nscores Scores = {\"Ada\" = 10}\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(scores_decl) = &body[0].kind else {
        panic!("expected scores declaration");
    };

    let entries = map_entries(
        &ast,
        scores_decl
            .value
            .initializer()
            .expect("fixture initializer"),
    );

    assert_eq!(entries.len(), 1);
    assert_eq!(
        display_type_with_resolver(
            scores_decl.type_id,
            &ast.type_environment,
            &string_table,
            &PathInternerBuilder::new().freeze(),
        ),
        "{String = I32}"
    );
}

#[test]
fn rejects_collection_first_mixed_collection_map_entries() {
    let diagnostic = parse_single_file_ast_diagnostic("scores ~= {\"a\", \"b\" = 2}\n");

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-SYNTAX-0033",
        Some("invalid_map_literal.mixed_collection_map_entries"),
    );
}

#[test]
fn rejects_map_entry_in_explicit_collection_context() {
    let diagnostic = parse_single_file_ast_diagnostic("values {String} = {\"a\" = 1}\n");

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-SYNTAX-0033",
        Some("invalid_map_literal.mixed_collection_map_entries"),
    );
}

#[test]
fn rejects_double_equal_inside_collection_as_common_syntax_mistake() {
    let diagnostic =
        parse_single_file_ast_diagnostic("left = true\nright = false\nvalues ~= {left == right}\n");

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-SYNTAX-0031",
        Some("common_syntax_mistake.equality_operator"),
    );
}

#[test]
fn rejects_unknown_bare_identifier_key() {
    let diagnostic =
        parse_single_file_ast_diagnostic("scores ~{String = I32} = {unknown_key = 10}\n");

    assert_ast_diagnostic(&diagnostic, "MOTH-RULE-0034", None);
}

// --------------------------
//  Map builtin helpers
// --------------------------

fn runtime_map_builtin_op(ast: &Ast, expression: ExprId) -> MapBuiltinOp {
    let view = expression_view(ast, expression);
    let ExpressionOperationView::MapBuiltin { facts, .. } = view.operation() else {
        panic!("expected map builtin call expression");
    };
    facts.operation
}

fn handled_map_builtin_op(ast: &Ast, expression: ExprId) -> MapBuiltinOp {
    runtime_map_builtin_op(ast, handled_runtime_call(ast, expression))
}

// --------------------------
//  Map builtin parsing
// --------------------------

#[test]
fn parses_map_get_with_catch_handler() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "scores ~{String = I32} = {\"Ada\" = 10}\nvalue = scores.get(\"Ada\") catch:\n    then 0\n;\n",
    );
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(value_decl) = &body[1].kind else {
        panic!("expected value declaration");
    };

    assert_eq!(
        handled_map_builtin_op(
            &ast,
            value_decl.value.initializer().expect("fixture initializer")
        ),
        MapBuiltinOp::Get
    );
}

#[test]
fn parses_map_get_with_propagation() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "get_value |scores {String = I32}| -> I32, Error!:\n    return scores.get(\"Ada\")!\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "get_value");

    let NodeKind::Return(values) = &body[0].kind else {
        panic!("expected return statement");
    };

    assert_eq!(handled_map_builtin_op(&ast, values[0]), MapBuiltinOp::Get);
}

#[test]
fn parses_map_contains() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "scores ~{String = I32} = {\"Ada\" = 10}\nfound = scores.contains(\"Ada\")\n",
    );
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(found_decl) = &body[1].kind else {
        panic!("expected found declaration");
    };

    assert_eq!(
        runtime_map_builtin_op(
            &ast,
            found_decl.value.initializer().expect("fixture initializer")
        ),
        MapBuiltinOp::Contains
    );
}

#[test]
fn parses_map_set_with_mutable_receiver() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "scores ~{String = I32} = {\"Ada\" = 10}\n~scores.set(\"Linus\", 7) catch:\n;\n",
    );
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::ExpressionStatement(expr) = &body[1].kind else {
        panic!("expected expression statement");
    };

    assert_eq!(handled_map_builtin_op(&ast, *expr), MapBuiltinOp::Set);
}

#[test]
fn parses_map_remove_with_mutable_receiver() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "scores ~{String = I32} = {\"Ada\" = 10}\nremoved = ~scores.remove(\"Ada\") catch:\n    then 0\n;\n",
    );
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(removed_decl) = &body[1].kind else {
        panic!("expected removed declaration");
    };

    assert_eq!(
        handled_map_builtin_op(
            &ast,
            removed_decl
                .value
                .initializer()
                .expect("fixture initializer"),
        ),
        MapBuiltinOp::Remove
    );
}

#[test]
fn parses_map_clear_with_mutable_receiver() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("scores ~{String = I32} = {\"Ada\" = 10}\n~scores.clear()\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::ExpressionStatement(expr) = &body[1].kind else {
        panic!("expected expression statement");
    };

    assert_eq!(runtime_map_builtin_op(&ast, *expr), MapBuiltinOp::Clear);
}

#[test]
fn parses_map_length_as_property() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("scores ~{String = I32} = {\"Ada\" = 10}\ncount = scores.length\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::VariableDeclaration(count_decl) = &body[1].kind else {
        panic!("expected count declaration");
    };

    assert_eq!(
        runtime_map_builtin_op(
            &ast,
            count_decl.value.initializer().expect("fixture initializer")
        ),
        MapBuiltinOp::Length
    );
}

#[test]
fn rejects_map_get_index_assignment() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "scores ~{String = I32} = {\"Ada\" = 10}\nscores.get(\"Ada\") = 5\n",
    );

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0044",
        Some("invalid_assignment_target.map_get_target_not_writable"),
    );
}

#[test]
fn rejects_map_mutation_on_immutable_binding() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "scores {String = I32} = {\"Ada\" = 10}\n~scores.set(\"Linus\", 7)!\n",
    );

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0047",
        Some("invalid_receiver_call.mutable_marker_on_immutable_receiver"),
    );
}

#[test]
fn rejects_map_contains_with_fallible_suffix() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "scores ~{String = I32} = {\"Ada\" = 10}\nvalue = scores.contains(\"Ada\")!\n",
    );

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.bang_on_non_fallible"),
    );
}

#[test]
fn rejects_map_clear_with_fallible_suffix() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "scores ~{String = I32} = {\"Ada\" = 10}\n~scores.clear()!\n",
    );

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.bang_on_non_fallible"),
    );
}

#[test]
fn rejects_map_length_with_fallible_suffix() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "scores ~{String = I32} = {\"Ada\" = 10}\ncount = scores.length!\n",
    );

    assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.bang_on_non_fallible"),
    );
}

#[test]
fn rejects_map_builtin_as_free_function() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "scores ~{String = I32} = {\"Ada\" = 10}\nvalue = get(scores, \"Ada\")!\n",
    );

    assert_ast_diagnostic(&diagnostic, "MOTH-RULE-0034", None);
}

#[test]
fn map_builtin_wins_before_visible_value_name() {
    // A visible free function named `get` must not capture `scores.get(...)`.
    // Map member syntax is compiler-owned once the receiver has a map type.
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "get |key String| -> I32:\n    return 0\n;\n\nget_value |scores {String = I32}| -> I32, Error!:\n    return scores.get(\"Ada\")!\n;\n",
    );
    let body = function_body_by_name(&ast, &path_fork, &string_table, "get_value");

    let NodeKind::Return(values) = &body[0].kind else {
        panic!("expected return statement");
    };

    assert_eq!(handled_map_builtin_op(&ast, values[0]), MapBuiltinOp::Get);
}
