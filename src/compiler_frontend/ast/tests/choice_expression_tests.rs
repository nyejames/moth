//! Choice expression parsing tests.
//!
//! WHAT: validates `Choice::Variant` expression resolution and diagnostics.
//! WHY: alpha choices are unit-variant-only and must fail fast for unknown/deferred forms.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::compiler_messages::{DiagnosticPayload, InvalidChoiceVariantReason};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, start_function_body,
};
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};

#[test]
fn resolves_choice_variant_expressions_with_choice_types() {
    let (ast, path_fork, string_table) = parse_single_file_ast("Status :: Ready, Busy;\n\
     echo_status |status Status| -> Status:\n\
         return status\n\
     ;\n\
     make_status || -> Status:\n\
         selected = Status::Busy\n\
         return echo_status(selected)\n\
     ;\n\
     current Status = Status::Ready\n\
     next = make_status()\n");

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

    let (nominal_path, tag) = match &current_declaration.value.kind {
        ExpressionKind::ChoiceConstruct {
            nominal_path, tag, ..
        } => (nominal_path, *tag),
        other => panic!("expected ChoiceConstruct, got {other:?}"),
    };
    assert_eq!(tag, 0, "expected Status::Ready to have tag 0");
    assert_eq!(
        path_fork
            .component(*nominal_path)
            .map(|id| string_table.resolve(id)),
        Some("Status"),
        "expected nominal path to be Status"
    );
    assert!(
        matches!(
            &current_declaration.value.diagnostic_type,
            DataType::Choices {
                nominal_path,
                ..
            } if path_fork
                .component(*nominal_path)
                .map(|id| string_table.resolve(id))
                == Some("Status")
        ),
        "choice literal should keep declaration-backed choice identity"
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

    let (nominal_path, tag) = match &selected_declaration.value.kind {
        ExpressionKind::ChoiceConstruct {
            nominal_path, tag, ..
        } => (nominal_path, *tag),
        other => panic!("expected ChoiceConstruct, got {other:?}"),
    };
    assert_eq!(tag, 1, "expected Status::Busy to have tag 1");
    assert_eq!(
        path_fork
            .component(*nominal_path)
            .map(|id| string_table.resolve(id)),
        Some("Status"),
        "expected nominal path to be Status"
    );
    assert!(
        matches!(
            &selected_declaration.value.diagnostic_type,
            DataType::Choices {
                nominal_path,
                ..
            } if path_fork
                .component(*nominal_path)
                .map(|id| string_table.resolve(id))
                == Some("Status")
        ),
        "choice literal should preserve declaration-backed choice type"
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
fn reports_missing_variant_name_after_choice_separator() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "Status :: Ready, Busy;\n\
         value = Status::\n",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedToken { .. }
    ));
}
