//! Choice expression parsing tests.
//!
//! WHAT: validates `Choice::Variant` expression resolution and diagnostics.
//! WHY: alpha choices are unit-variant-only and must fail fast for unknown/deferred forms.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionOperationView;
use crate::compiler_frontend::compiler_messages::{
    ChoiceVariantSeparatorGap, CommonSyntaxMistakeReason, DiagnosticPayload,
    InvalidChoiceVariantReason, NameNamespace,
};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, start_function_body,
};
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};

#[test]
fn resolves_choice_variant_expressions_with_choice_types() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        "Status :: Ready, Busy;\n\
     echo_status |status Status| -> Status:\n\
         return status\n\
     ;\n\
     make_status || -> Status:\n\
         selected = Status::Busy\n\
         return echo_status(selected)\n\
     ;\n\
     current Status = Status::Ready\n\
     next = make_status()\n",
    );

    let start_body = start_function_body(&ast, &path_fork, &string_table);
    let current_declaration = start_body
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::VariableDeclaration(declaration)
                if path_fork
                    .component(declaration.id)
                    .map(|id| string_table.resolve(id))
                    == Some("current") =>
            {
                Some(declaration)
            }
            _ => None,
        })
        .expect("expected 'current' declaration in start function");

    let choice = ast
        .expressions
        .view(
            current_declaration
                .value
                .initializer()
                .expect("fixture initializer"),
        )
        .expect("fixture initializer belongs to AST");
    let ExpressionOperationView::Choice { tag, .. } = choice.operation() else {
        panic!("expected Choice operation");
    };
    let current_type_id = current_declaration.type_id;
    let Some(TypeDefinition::Choice(definition)) = ast.type_environment.get(current_type_id) else {
        panic!("expected the current declaration to have a registered choice type");
    };
    assert_eq!(tag, 0, "expected Status::Ready to have tag 0");
    assert_eq!(
        path_fork
            .component(definition.path)
            .map(|id| string_table.resolve(id)),
        Some("Status"),
        "expected nominal path to be Status"
    );

    let make_status_body = function_body_by_name(&ast, &path_fork, &string_table, "make_status");
    let selected_declaration = make_status_body
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::VariableDeclaration(declaration)
                if path_fork
                    .component(declaration.id)
                    .map(|id| string_table.resolve(id))
                    == Some("selected") =>
            {
                Some(declaration)
            }
            _ => None,
        })
        .expect("expected 'selected' declaration in make_status");

    let selected = ast
        .expressions
        .view(
            selected_declaration
                .value
                .initializer()
                .expect("fixture initializer"),
        )
        .expect("fixture initializer belongs to AST");
    let ExpressionOperationView::Choice { tag, .. } = selected.operation() else {
        panic!("expected Choice operation");
    };
    let Some(TypeDefinition::Choice(definition)) =
        ast.type_environment.get(selected_declaration.type_id)
    else {
        panic!("expected the selected declaration to have a registered choice type");
    };
    assert_eq!(tag, 1, "expected Status::Busy to have tag 1");
    assert_eq!(
        path_fork
            .component(definition.path)
            .map(|id| string_table.resolve(id)),
        Some("Status"),
        "expected nominal path to be Status"
    );
    assert_eq!(
        selected_declaration.type_id, current_type_id,
        "choice literals should share the declaration-backed semantic type"
    );
}

#[test]
fn reports_unknown_choice_variant_with_targeted_diagnostic() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "Status :: Ready, Busy;\n\
         value = Status::Unknown\n",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidChoiceVariant {
            reason: InvalidChoiceVariantReason::UnknownVariant,
            ..
        }
    ));
}

#[test]
fn reports_spacing_after_choice_separator_without_variant_text() {
    assert_choice_separator_gap(
        "Status :: Ready, Busy;\nvalue = Status::\n",
        ChoiceVariantSeparatorGap::After,
    );
}

#[test]
fn rejects_line_break_or_comment_before_choice_separator() {
    for reference in ["Status\n::Ready", "Status -- note\n::Ready"] {
        let source = format!("Status :: Ready;\nvalue = {reference}\n");
        let diagnostic = parse_single_file_ast_diagnostic(&source);

        assert_eq!(diagnostic.identity().code, "MOTH-RULE-0037");
        assert!(matches!(
            diagnostic.payload,
            DiagnosticPayload::NamespaceMisuse {
                expected: NameNamespace::Value,
                found: NameNamespace::Type,
                ..
            }
        ));
    }
}

fn assert_choice_separator_gap(source: &str, expected_gap: ChoiceVariantSeparatorGap) {
    let diagnostic = parse_single_file_ast_diagnostic(source);
    assert_eq!(diagnostic.identity().code, "MOTH-SYNTAX-0031");
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::CommonSyntaxMistake {
                reason: CommonSyntaxMistakeReason::InvalidChoiceVariantSpacing { gap },
            } if *gap == expected_gap
        ),
        "{:?}",
        diagnostic.payload
    );
}

#[test]
fn rejects_choice_separator_gaps_in_expressions() {
    for (constructor, expected_gap) in [
        ("Status ::Ready", ChoiceVariantSeparatorGap::Before),
        ("Status:: Ready", ChoiceVariantSeparatorGap::After),
        ("Status :: Ready", ChoiceVariantSeparatorGap::Both),
        ("Status::\nReady", ChoiceVariantSeparatorGap::After),
        (
            "Status:: -- comment\nReady",
            ChoiceVariantSeparatorGap::After,
        ),
    ] {
        let source = format!("Status :: Ready;\nvalue = {constructor}\n");
        assert_choice_separator_gap(&source, expected_gap);
    }
}

#[test]
fn rejects_newline_after_choice_separator_in_function_body() {
    assert_choice_separator_gap(
        "Status :: Ready;\n\
         make || -> Status:\n\
             return Status::\n\
                 Ready\n\
         ;\n",
        ChoiceVariantSeparatorGap::After,
    );
}
