//! Collection literal parsing regression tests.
//!
//! WHAT: validates collection item parsing and unterminated-literal diagnostics.
//! WHY: collections use expression recursion and can silently drift without focused coverage.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, FallibleExpressionHandling,
};
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::builtins::CollectionBuiltinOp;
use crate::compiler_frontend::builtins::maps::MapBuiltinOp;
use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, DiagnosticPayload, InvalidAssignmentTargetReason,
    InvalidBuiltinCallReason, InvalidFallibleHandlingReason, InvalidFieldAccessReason,
    InvalidMapLiteralReason, InvalidReceiverCallReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, start_function_body,
};
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};

fn runtime_collection_builtin_op(expression: &Expression) -> CollectionBuiltinOp {
    let ExpressionKind::CollectionBuiltinCall { op, .. } = &expression.kind else {
        panic!("expected collection builtin call expression");
    };

    *op
}

fn handled_collection_builtin_op(expression: &Expression) -> CollectionBuiltinOp {
    let handled_expression = match &expression.kind {
        ExpressionKind::HandledFallibleExpression { value, .. } => value.as_ref(),
        ExpressionKind::ValueBlock { block } => {
            let ValueBlock::Catch(value_catch) = block.as_ref() else {
                panic!("expected catch value block");
            };
            let ExpressionKind::HandledFallibleExpression { value, .. } =
                &value_catch.handled_value.kind
            else {
                panic!("expected handled fallible expression inside catch value block");
            };
            value.as_ref()
        }
        _ => panic!("expected handled fallible expression"),
    };

    runtime_collection_builtin_op(handled_expression)
}

// --------------------------
//  Collection literals
// --------------------------

#[test]
fn parses_collection_literal_items() {
    let (ast, string_table) = parse_single_file_ast("values ~= {1, 2, 3}\n");
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(values_decl) = &body[0].kind else {
        panic!("expected collection declaration");
    };

    let ExpressionKind::Collection(items) = &values_decl.value.kind else {
        panic!("expected collection expression");
    };

    assert_eq!(items.len(), 3);
    assert!(matches!(items[0].kind, ExpressionKind::Int(1)));
    assert!(matches!(items[1].kind, ExpressionKind::Int(2)));
    assert!(matches!(items[2].kind, ExpressionKind::Int(3)));
}

#[test]
fn parses_multiline_inferred_collection_literal_with_constructor_items() {
    let source = "Entry = |\n    value Int,\n|\n\nrender || -> String:\n    entries = {\n        Entry(1),\n        Entry(2),\n    }\n    return [entries.length()]\n;\n";
    let (ast, string_table) = parse_single_file_ast(source);
    let body = function_body_by_name(&ast, &string_table, "render");

    let NodeKind::VariableDeclaration(entries_decl) = &body[0].kind else {
        panic!("expected collection declaration");
    };

    let ExpressionKind::Collection(items) = &entries_decl.value.kind else {
        panic!("expected collection expression");
    };

    assert_eq!(items.len(), 2);
    assert_eq!(
        entries_decl
            .value
            .diagnostic_type
            .display_with_table(&string_table),
        "{Entry}"
    );
}

#[test]
fn infers_collection_element_type_from_non_empty_literal() {
    let (ast, string_table) = parse_single_file_ast("values ~= {1, 2, 3}\n");
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(values_decl) = &body[0].kind else {
        panic!("expected values declaration");
    };

    assert_eq!(
        values_decl
            .value
            .diagnostic_type
            .display_with_table(&string_table),
        "{Int}"
    );
}

#[test]
fn parses_empty_collection_with_explicit_element_type() {
    let (ast, string_table) =
        parse_single_file_ast("Reading = |\n    value Float,\n|\n\nreadings ~{Reading} = {}\n");
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(readings_decl) = &body[0].kind else {
        panic!("expected readings declaration");
    };

    assert_eq!(
        readings_decl
            .value
            .diagnostic_type
            .display_with_table(&string_table),
        "{Reading}"
    );
}

#[test]
fn rejects_mixed_item_types_in_inferred_collection() {
    let diagnostic = parse_single_file_ast_diagnostic("values ~= {1, \"bad\"}\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::TypeMismatch {
            context: TypeMismatchContext::CollectionElement,
            ..
        }
    ));
}

#[test]
fn rejects_item_that_does_not_match_explicit_collection_type() {
    let diagnostic = parse_single_file_ast_diagnostic("values ~{Int} = {1, \"bad\"}\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::TypeMismatch {
            context: TypeMismatchContext::CollectionElement,
            ..
        }
    ));
}

#[test]
fn parses_push_after_explicit_empty_collection() {
    let (ast, string_table) = parse_single_file_ast(
        "values ~{Int} = {}\n~values.push(1) catch:
;\n",
    );
    let body = start_function_body(&ast, &string_table);

    let NodeKind::ExpressionStatement(push_expr) = &body[1].kind else {
        panic!("expected push statement");
    };

    assert_eq!(
        handled_collection_builtin_op(push_expr),
        CollectionBuiltinOp::Push
    );
}

#[test]
fn rejects_empty_collection_even_when_later_push_would_reveal_type() {
    let diagnostic = parse_single_file_ast_diagnostic("values ~= {}\n~values.push(1)\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::EmptyCollectionTypeAmbiguity
    ));
}

#[test]
fn rejects_missing_collection_item_after_comma() {
    let diagnostic = parse_single_file_ast_diagnostic("values ~= {1, , 2}\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::MissingCollectionItem
    ));
}

// --------------------------
//  Fallible handling on collections
// --------------------------

#[test]
fn parses_collection_get_with_fallback_handler_and_propagation() {
    let (ast, string_table) = parse_single_file_ast(
        "read_or_default |values {Int}, idx Int| -> Int:\n    return values.get(idx) catch:\n        then 0\n    ;\n;\n\nread_with_handler |values {Int}, idx Int| -> Int:\n    return values.get(idx) catch |err|:\n        io.line([: [err.message]])\n        then 0\n    ;\n;\n\nforward_read |values {Int}, idx Int| -> Int, Error!:\n    return values.get(idx)!\n;\n",
    );

    let fallback_body = function_body_by_name(&ast, &string_table, "read_or_default");
    let NodeKind::Return(values) = &fallback_body[0].kind else {
        panic!("expected return statement in fallback function");
    };
    assert!(matches!(values[0].kind, ExpressionKind::ValueBlock { .. }));

    let handler_body = function_body_by_name(&ast, &string_table, "read_with_handler");
    let NodeKind::Return(values) = &handler_body[0].kind else {
        panic!("expected return statement in handler function");
    };
    assert!(matches!(values[0].kind, ExpressionKind::ValueBlock { .. }));

    let propagation_body = function_body_by_name(&ast, &string_table, "forward_read");
    let NodeKind::Return(values) = &propagation_body[0].kind else {
        panic!("expected return statement in propagation function");
    };
    let ExpressionKind::HandledFallibleExpression { handling, .. } = &values[0].kind else {
        panic!("expected handled fallible expression for propagation");
    };
    assert!(matches!(handling, FallibleExpressionHandling::Propagate));
}

// --------------------------
//  Collection mutators and accessors
// --------------------------

#[test]
fn parses_collection_mutators_and_length_calls() {
    let (ast, string_table) = parse_single_file_ast(
        "values ~= {1, 2, 3}\n~values.set(1, 9) catch:\n;\n~values.push(4) catch:
;\nremoved = ~values.remove(2) catch:\n    then 0\n;\nsize = values.length()\n",
    );
    let body = start_function_body(&ast, &string_table);

    let NodeKind::ExpressionStatement(set_expr) = &body[1].kind else {
        panic!("expected set(...) statement");
    };
    assert_eq!(
        handled_collection_builtin_op(set_expr),
        CollectionBuiltinOp::Set
    );

    let NodeKind::ExpressionStatement(push_expr) = &body[2].kind else {
        panic!("expected push(...) statement");
    };
    assert_eq!(
        handled_collection_builtin_op(push_expr),
        CollectionBuiltinOp::Push
    );

    let NodeKind::VariableDeclaration(removed_decl) = &body[3].kind else {
        panic!("expected removed declaration");
    };
    assert_eq!(
        handled_collection_builtin_op(&removed_decl.value),
        CollectionBuiltinOp::Remove
    );

    let NodeKind::VariableDeclaration(size_decl) = &body[4].kind else {
        panic!("expected length declaration");
    };
    assert_eq!(size_decl.value.type_id, builtin_type_ids::INT);
    assert_eq!(
        runtime_collection_builtin_op(&size_decl.value),
        CollectionBuiltinOp::Length
    );
}

#[test]
fn parses_collection_mutators_with_explicit_receiver_tilde_prefix() {
    let (ast, string_table) = parse_single_file_ast(
        "values ~= {1, 2, 3}\n~values.push(4) catch:
;\n~values.set(1, 9) catch:\n;\nremoved = ~values.remove(2) catch:\n    then 0\n;\nsize = values.length()\n",
    );
    let body = start_function_body(&ast, &string_table);

    let NodeKind::ExpressionStatement(push_expr) = &body[1].kind else {
        panic!("expected push(...) statement");
    };
    assert_eq!(
        handled_collection_builtin_op(push_expr),
        CollectionBuiltinOp::Push
    );

    let NodeKind::ExpressionStatement(set_expr) = &body[2].kind else {
        panic!("expected set(...) statement");
    };
    assert_eq!(
        handled_collection_builtin_op(set_expr),
        CollectionBuiltinOp::Set
    );

    let NodeKind::VariableDeclaration(removed_decl) = &body[3].kind else {
        panic!("expected removed declaration");
    };
    assert_eq!(
        handled_collection_builtin_op(&removed_decl.value),
        CollectionBuiltinOp::Remove
    );
}

#[test]
fn rejects_unhandled_collection_push_result() {
    let diagnostic = parse_single_file_ast_diagnostic("values ~= {1, 2, 3}\n~values.push(4)\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidBuiltinCall {
            reason: InvalidBuiltinCallReason::UnhandledFallibleCall,
            ..
        }
    ));
}

#[test]
fn accepts_collection_push_postfix_propagation() {
    parse_single_file_ast(
        "append || -> Error!:
    values ~= {1, 2, 3}
    ~values.push(4)!
;
",
    );
}

#[test]
fn rejects_collection_push_fallback_value() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "values ~= {1, 2, 3}
~values.push(4) catch:
    then 0
;
",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling {
            reason: InvalidFallibleHandlingReason::FallbackValuesForErrorOnlyResult,
            ..
        }
    ));
}

#[test]
fn rejects_mutating_collection_method_without_explicit_receiver_tilde() {
    let diagnostic = parse_single_file_ast_diagnostic("values ~= {1, 2, 3}\nvalues.push(4)\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidReceiverCall {
            reason: InvalidReceiverCallReason::MutableReceiverMissingMarker,
            ..
        }
    ));
}

#[test]
fn rejects_explicit_receiver_tilde_for_collection_get_and_length() {
    let get_diagnostic = parse_single_file_ast_diagnostic(
        "values ~= {1, 2, 3}\nvalue = ~values.get(0) catch:\n    then 0\n;\n",
    );
    assert!(matches!(
        get_diagnostic.payload,
        DiagnosticPayload::InvalidReceiverCall {
            reason: InvalidReceiverCallReason::UnneededMutableAccessMarker,
            ..
        }
    ));

    let length_diagnostic =
        parse_single_file_ast_diagnostic("values ~= {1, 2, 3}\nvalue = ~values.length()\n");
    assert!(matches!(
        length_diagnostic.payload,
        DiagnosticPayload::InvalidReceiverCall {
            reason: InvalidReceiverCallReason::UnneededMutableAccessMarker,
            ..
        }
    ));
}

#[test]
fn parses_multiline_zero_argument_collection_and_map_builtins() {
    let source = "values = {1, 2}\ncount = values.length(\n)\nscores ~{String = Int} = {\"Ada\" = 10}\n~scores.clear(\n)\n";

    parse_single_file_ast(source);
}

#[test]
fn preserves_zero_argument_builtin_extra_argument_diagnostic() {
    let diagnostic = parse_single_file_ast_diagnostic("values = {1, 2}\nvalues.length(1)\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidBuiltinCall {
            reason: InvalidBuiltinCallReason::TakesNoArguments,
            ..
        }
    ));
}

#[test]
fn rejects_pull_method_as_unknown_collection_member() {
    let diagnostic = parse_single_file_ast_diagnostic("values ~= {1, 2, 3}\nvalues.pull(1)\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFieldAccess {
            reason: InvalidFieldAccessReason::UnknownMember,
            ..
        }
    ));
}

#[test]
fn rejects_get_index_assignment_as_removed_syntax() {
    let diagnostic = parse_single_file_ast_diagnostic("values = {1, 2, 3}\nvalues.get(0) = 9\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidAssignmentTarget {
            reason: InvalidAssignmentTargetReason::CollectionGetTargetNotWritable,
            ..
        }
    ));
}

#[test]
fn rejects_unhandled_collection_get_result() {
    let diagnostic =
        parse_single_file_ast_diagnostic("values ~= {1, 2, 3}\nvalue = values.get(0)\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidBuiltinCall {
            reason: InvalidBuiltinCallReason::UnhandledFallibleCall,
            ..
        }
    ));
}

#[test]
fn rejects_unhandled_collection_set_result() {
    let diagnostic = parse_single_file_ast_diagnostic("values ~= {1, 2, 3}\n~values.set(0, 9)\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidBuiltinCall {
            reason: InvalidBuiltinCallReason::UnhandledFallibleCall,
            ..
        }
    ));
}

#[test]
fn rejects_collection_set_fallback_value() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "values ~= {1, 2, 3}\n~values.set(0, 9) catch:\n    then 0\n;\n",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling {
            reason: InvalidFallibleHandlingReason::FallbackValuesForErrorOnlyResult,
            ..
        }
    ));
}

#[test]
fn rejects_discarded_collection_remove_success() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "drop_removed || -> Error!:\n    values ~= {1, 2, 3}\n    ~values.remove(0)!\n;\n",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling {
            reason: InvalidFallibleHandlingReason::SuccessValueDiscarded,
            ..
        }
    ));
}

// --------------------------
//  Removed builtin error helpers
// --------------------------

#[test]
fn rejects_removed_builtin_error_helper_methods() {
    let diagnostic =
        parse_single_file_ast_diagnostic("err = Error(\"bad\")\nvalue = err.bubble()\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFieldAccess {
            reason: InvalidFieldAccessReason::UnknownMember,
            ..
        }
    ));
}

// --------------------------
//  Fixed collection literals
// --------------------------

#[test]
fn fixed_collection_literal_within_capacity_is_accepted() {
    let (ast, string_table) = parse_single_file_ast("items {2 Int} = {1, 2}\n");
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(decl) = &body[0].kind else {
        panic!("expected declaration");
    };

    assert_eq!(
        decl.value.diagnostic_type.display_with_table(&string_table),
        "{2 Int}"
    );
}

// --------------------------
//  Map literals
// --------------------------

#[test]
fn parses_inferred_map_literal() {
    let (ast, string_table) = parse_single_file_ast("scores ~= {\"Ada\" = 10, \"Grace\" = 12}\n");
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(scores_decl) = &body[0].kind else {
        panic!("expected scores declaration");
    };

    let ExpressionKind::MapLiteral(entries) = &scores_decl.value.kind else {
        panic!("expected map literal expression");
    };

    assert_eq!(entries.len(), 2);
    assert!(
        ast.type_environment.is_map_type(scores_decl.value.type_id),
        "expected map type id"
    );
    assert_eq!(
        scores_decl
            .value
            .diagnostic_type
            .display_with_table(&string_table),
        "{String = Int}"
    );
}

#[test]
fn parses_explicit_empty_map_literal() {
    let (ast, string_table) = parse_single_file_ast("scores {String = Int} = {}\n");
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(scores_decl) = &body[0].kind else {
        panic!("expected scores declaration");
    };

    let ExpressionKind::MapLiteral(entries) = &scores_decl.value.kind else {
        panic!("expected map literal expression");
    };

    assert!(entries.is_empty());
    assert_eq!(
        scores_decl
            .value
            .diagnostic_type
            .display_with_table(&string_table),
        "{String = Int}"
    );
}

#[test]
fn parses_map_literal_with_runtime_key_expression() {
    let (ast, string_table) = parse_single_file_ast(
        "get_key || -> String:\n    return \"Ada\"\n;\n\nscores ~= {get_key() = 10}\n",
    );
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(scores_decl) = &body[0].kind else {
        panic!("expected scores declaration");
    };

    let ExpressionKind::MapLiteral(entries) = &scores_decl.value.kind else {
        panic!("expected map literal expression");
    };

    assert_eq!(entries.len(), 1);
    assert!(
        matches!(entries[0].key.kind, ExpressionKind::FunctionCall { .. }),
        "expected runtime key expression"
    );
}

#[test]
fn parses_map_literal_with_bare_identifier_key_as_variable() {
    let (ast, string_table) = parse_single_file_ast("key = \"Ada\"\nscores ~= {key = 10}\n");
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(scores_decl) = &body[1].kind else {
        panic!("expected scores declaration, got: {:?}", body[1].kind);
    };

    let ExpressionKind::MapLiteral(entries) = &scores_decl.value.kind else {
        panic!(
            "expected map literal expression, got: {:?}",
            scores_decl.value.kind
        );
    };

    assert_eq!(entries.len(), 1);
    assert!(
        matches!(entries[0].key.kind, ExpressionKind::Reference { .. }),
        "expected bare identifier key to be parsed as variable reference"
    );
}

#[test]
fn parses_map_literal_with_contextual_none_value() {
    let (ast, string_table) =
        parse_single_file_ast("scores ~{String = Int?} = {\"Ada\" = none, \"Grace\" = 12}\n");
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(scores_decl) = &body[0].kind else {
        panic!("expected scores declaration");
    };

    let ExpressionKind::MapLiteral(entries) = &scores_decl.value.kind else {
        panic!("expected map literal expression");
    };

    assert_eq!(entries.len(), 2);
    assert!(
        matches!(entries[0].value.kind, ExpressionKind::OptionNone),
        "expected none in value position with option context"
    );
}

#[test]
fn parses_map_literal_with_string_key_coercion() {
    let (ast, string_table) = parse_single_file_ast("scores ~{String = Int} = {\"Ada\" = 10}\n");
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(scores_decl) = &body[0].kind else {
        panic!("expected scores declaration");
    };

    let ExpressionKind::MapLiteral(entries) = &scores_decl.value.kind else {
        panic!("expected map literal expression");
    };

    assert_eq!(entries.len(), 1);
    assert!(
        matches!(entries[0].key.kind, ExpressionKind::StringSlice(_)),
        "expected string literal key"
    );
}

#[test]
fn parses_nested_map_literal_value() {
    let (ast, string_table) =
        parse_single_file_ast("scores ~{String = {String = Int}} = {\"group\" = {\"Ada\" = 10}}\n");
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(scores_decl) = &body[0].kind else {
        panic!("expected scores declaration");
    };

    let ExpressionKind::MapLiteral(entries) = &scores_decl.value.kind else {
        panic!("expected map literal expression");
    };

    assert_eq!(entries.len(), 1);
    assert!(
        matches!(entries[0].value.kind, ExpressionKind::MapLiteral(_)),
        "expected nested map literal value"
    );
}

#[test]
fn parses_map_type_alias_literal() {
    let (ast, string_table) =
        parse_single_file_ast("Scores as {String = Int}\nscores Scores = {\"Ada\" = 10}\n");
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(scores_decl) = &body[0].kind else {
        panic!("expected scores declaration");
    };

    let ExpressionKind::MapLiteral(entries) = &scores_decl.value.kind else {
        panic!("expected map literal expression");
    };

    assert_eq!(entries.len(), 1);
    assert_eq!(
        scores_decl
            .value
            .diagnostic_type
            .display_with_table(&string_table),
        "{String = Int}"
    );
}

#[test]
fn rejects_collection_first_mixed_collection_map_entries() {
    let diagnostic = parse_single_file_ast_diagnostic("scores ~= {\"a\", \"b\" = 2}\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidMapLiteral {
            reason: InvalidMapLiteralReason::MixedCollectionMapEntries,
            ..
        }
    ));
}

#[test]
fn rejects_map_entry_in_explicit_collection_context() {
    let diagnostic = parse_single_file_ast_diagnostic("values {String} = {\"a\" = 1}\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidMapLiteral {
            reason: InvalidMapLiteralReason::MixedCollectionMapEntries,
            ..
        }
    ));
}

#[test]
fn rejects_double_equal_inside_collection_as_common_syntax_mistake() {
    let diagnostic =
        parse_single_file_ast_diagnostic("left = true\nright = false\nvalues ~= {left == right}\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::CommonSyntaxMistake {
            reason: CommonSyntaxMistakeReason::EqualityOperator
        }
    ));
}

#[test]
fn rejects_unknown_bare_identifier_key() {
    let diagnostic =
        parse_single_file_ast_diagnostic("scores ~{String = Int} = {unknown_key = 10}\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnknownName { .. }
    ));
}

// --------------------------
//  Map builtin helpers
// --------------------------

fn runtime_map_builtin_op(expression: &Expression) -> MapBuiltinOp {
    let ExpressionKind::MapBuiltinCall { op, .. } = &expression.kind else {
        panic!(
            "expected map builtin call expression, got {:?}",
            expression.kind
        );
    };

    *op
}

fn handled_map_builtin_op(expression: &Expression) -> MapBuiltinOp {
    let handled_expression = match &expression.kind {
        ExpressionKind::HandledFallibleExpression { value, .. } => value.as_ref(),
        ExpressionKind::ValueBlock { block } => {
            let ValueBlock::Catch(value_catch) = block.as_ref() else {
                panic!("expected catch value block");
            };
            let ExpressionKind::HandledFallibleExpression { value, .. } =
                &value_catch.handled_value.kind
            else {
                panic!("expected handled fallible expression inside catch value block");
            };
            value.as_ref()
        }
        _ => panic!("expected handled fallible expression"),
    };

    runtime_map_builtin_op(handled_expression)
}

// --------------------------
//  Map builtin parsing
// --------------------------

#[test]
fn parses_map_get_with_catch_handler() {
    let (ast, string_table) = parse_single_file_ast(
        "scores ~{String = Int} = {\"Ada\" = 10}\nvalue = scores.get(\"Ada\") catch:\n    then 0\n;\n",
    );
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(value_decl) = &body[1].kind else {
        panic!("expected value declaration");
    };

    assert_eq!(handled_map_builtin_op(&value_decl.value), MapBuiltinOp::Get);
}

#[test]
fn parses_map_get_with_propagation() {
    let (ast, string_table) = parse_single_file_ast(
        "get_value |scores {String = Int}| -> Int, Error!:\n    return scores.get(\"Ada\")!\n;\n",
    );
    let body = function_body_by_name(&ast, &string_table, "get_value");

    let NodeKind::Return(values) = &body[0].kind else {
        panic!("expected return statement");
    };

    assert_eq!(handled_map_builtin_op(&values[0]), MapBuiltinOp::Get);
}

#[test]
fn parses_map_contains() {
    let (ast, string_table) = parse_single_file_ast(
        "scores ~{String = Int} = {\"Ada\" = 10}\nfound = scores.contains(\"Ada\")\n",
    );
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(found_decl) = &body[1].kind else {
        panic!("expected found declaration");
    };

    assert_eq!(
        runtime_map_builtin_op(&found_decl.value),
        MapBuiltinOp::Contains
    );
}

#[test]
fn parses_map_set_with_mutable_receiver() {
    let (ast, string_table) = parse_single_file_ast(
        "scores ~{String = Int} = {\"Ada\" = 10}\n~scores.set(\"Linus\", 7) catch:\n;\n",
    );
    let body = start_function_body(&ast, &string_table);

    let NodeKind::ExpressionStatement(expr) = &body[1].kind else {
        panic!("expected expression statement");
    };

    assert_eq!(handled_map_builtin_op(expr), MapBuiltinOp::Set);
}

#[test]
fn parses_map_remove_with_mutable_receiver() {
    let (ast, string_table) = parse_single_file_ast(
        "scores ~{String = Int} = {\"Ada\" = 10}\nremoved = ~scores.remove(\"Ada\") catch:\n    then 0\n;\n",
    );
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(removed_decl) = &body[1].kind else {
        panic!("expected removed declaration");
    };

    assert_eq!(
        handled_map_builtin_op(&removed_decl.value),
        MapBuiltinOp::Remove
    );
}

#[test]
fn parses_map_clear_with_mutable_receiver() {
    let (ast, string_table) =
        parse_single_file_ast("scores ~{String = Int} = {\"Ada\" = 10}\n~scores.clear()\n");
    let body = start_function_body(&ast, &string_table);

    let NodeKind::ExpressionStatement(expr) = &body[1].kind else {
        panic!("expected expression statement");
    };

    assert_eq!(runtime_map_builtin_op(expr), MapBuiltinOp::Clear);
}

#[test]
fn parses_map_length_as_property() {
    let (ast, string_table) =
        parse_single_file_ast("scores ~{String = Int} = {\"Ada\" = 10}\ncount = scores.length\n");
    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(count_decl) = &body[1].kind else {
        panic!("expected count declaration");
    };

    assert_eq!(
        runtime_map_builtin_op(&count_decl.value),
        MapBuiltinOp::Length
    );
}

#[test]
fn rejects_map_get_index_assignment() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "scores ~{String = Int} = {\"Ada\" = 10}\nscores.get(\"Ada\") = 5\n",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidAssignmentTarget {
            reason: InvalidAssignmentTargetReason::MapGetTargetNotWritable,
            ..
        }
    ));
}

#[test]
fn rejects_map_mutation_on_immutable_binding() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "scores {String = Int} = {\"Ada\" = 10}\n~scores.set(\"Linus\", 7)!\n",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidReceiverCall {
            reason: InvalidReceiverCallReason::MutableMarkerOnImmutableReceiver,
            ..
        }
    ));
}

#[test]
fn rejects_map_contains_with_fallible_suffix() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "scores ~{String = Int} = {\"Ada\" = 10}\nvalue = scores.contains(\"Ada\")!\n",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling {
            reason: InvalidFallibleHandlingReason::BangOnNonFallible,
            ..
        }
    ));
}

#[test]
fn rejects_map_clear_with_fallible_suffix() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "scores ~{String = Int} = {\"Ada\" = 10}\n~scores.clear()!\n",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling {
            reason: InvalidFallibleHandlingReason::BangOnNonFallible,
            ..
        }
    ));
}

#[test]
fn rejects_map_length_with_fallible_suffix() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "scores ~{String = Int} = {\"Ada\" = 10}\ncount = scores.length!\n",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling {
            reason: InvalidFallibleHandlingReason::BangOnNonFallible,
            ..
        }
    ));
}

#[test]
fn rejects_map_builtin_as_free_function() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "scores ~{String = Int} = {\"Ada\" = 10}\nvalue = get(scores, \"Ada\")!\n",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnknownName { .. }
    ));
}

#[test]
fn map_builtin_wins_before_visible_value_name() {
    // A visible free function named `get` must not capture `scores.get(...)`.
    // Map member syntax is compiler-owned once the receiver has a map type.
    let (ast, string_table) = parse_single_file_ast(
        "get |key String| -> Int:\n    return 0\n;\n\nget_value |scores {String = Int}| -> Int, Error!:\n    return scores.get(\"Ada\")!\n;\n",
    );
    let body = function_body_by_name(&ast, &string_table, "get_value");

    let NodeKind::Return(values) = &body[0].kind else {
        panic!("expected return statement");
    };

    assert_eq!(handled_map_builtin_op(&values[0]), MapBuiltinOp::Get);
}
