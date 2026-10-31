//! Choice expression parsing tests.
//!
//! WHAT: validates `Choice::Variant` expression resolution and diagnostics.
//! WHY: alpha choices are unit-variant-only and must fail fast for unknown/deferred forms.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionOperationView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    names_declarations::TypeUsedAsValueView, token_lexical::CommonSyntaxMistakeView,
};
use crate::compiler_frontend::compiler_messages::{
    ChoiceVariantSeparatorGap, NameNamespace, TypeUsedAsValueReason,
};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, start_function_body,
};
use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, parse_single_file_ast, parse_single_file_ast_diagnostic,
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
    let failure = parse_single_file_ast_diagnostic(
        "Status :: Ready, Busy;\n\
         value = Status::Unknown\n",
    );
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0029",
        Some("invalid_choice_variant.unknown_variant"),
    );
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
        let failure = parse_single_file_ast_diagnostic(&source);
        let diagnostic = assert_ast_diagnostic(
            &failure,
            "MOTH-RULE-0037",
            Some("type_used_as_value.bare_type_name"),
        );
        let facts = diagnostic
            .family_view::<TypeUsedAsValueView>()
            .expect("line-separated qualifier retains its namespace facts");
        assert_eq!(facts.expected, NameNamespace::Value);
        assert_eq!(facts.found, NameNamespace::Type);
    }
}

#[test]
fn generic_choice_application_in_value_position_is_a_type_annotation() {
    let source = r#"
Maybe type T ::
    Some | value T |,
    Empty,
;

value = Maybe of I32
"#;
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0037",
        Some("type_used_as_value.generic_choice_application"),
    );
    let facts = diagnostic
        .family_view::<TypeUsedAsValueView>()
        .expect("generic choice application retains its namespace facts");
    assert_eq!(facts.expected, NameNamespace::Value);
    assert_eq!(facts.found, NameNamespace::Type);
    assert_eq!(
        facts.reason,
        TypeUsedAsValueReason::GenericChoiceApplication
    );
}

fn assert_choice_separator_gap(source: &str, expected_gap: ChoiceVariantSeparatorGap) {
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0031",
        Some("common_syntax_mistake.invalid_choice_variant_spacing"),
    );
    let facts = diagnostic
        .family_view::<CommonSyntaxMistakeView>()
        .expect("choice separator spacing retains its typed gap");
    assert_eq!(facts.choice_gap, Some(expected_gap));
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
