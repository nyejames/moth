//! Unit tests for generic function diagnostic helpers.
//!
//! WHAT: asserts that `with_generic_instantiation_context` rebuilds diagnostics correctly.
//! WHY: the helper is the single point of truth for call-site-primary generic instantiation
//! diagnostics and should be tested in isolation.

use crate::compiler_frontend::ast::generic_functions::diagnostics::{
    GenericInstantiationDiagnosticContext, conflicting_generic_function_argument,
    with_generic_instantiation_context,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticLabel, DiagnosticLabelMessage,
    DiagnosticLabelStyle, DiagnosticPayload, GenericSubstitutionDiagnostic, RuleDiagnosticKind,
    TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::generic_bindings::BindingConflict;
use crate::compiler_frontend::datatypes::ids::GenericParameterId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::StringTable;

fn make_span(source: usize, start: u32, length: u32) -> SourceSpan {
    let mut span_builder = ExtendedSpanBuilder::new();
    SourceSpan::new(
        SourceId::from_index(source),
        LocalSpan::exact(start, length, &mut span_builder).unwrap(),
    )
}

fn instantiation_context(
    call_span: SourceSpan,
    declaration_span: SourceSpan,
) -> GenericInstantiationDiagnosticContext {
    GenericInstantiationDiagnosticContext {
        call_span: Some(call_span),
        declaration_span: Some(declaration_span),
        substitutions: Vec::new(),
    }
}

#[test]
fn with_generic_instantiation_context_changes_primary_span_to_call_site() {
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
        builtin_type_ids::STRING,
        TypeMismatchContext::FunctionArgument,
        Some(body_span),
    );

    let transformed =
        with_generic_instantiation_context(diagnostic, instantiation_context(call_span, body_span));

    assert_eq!(transformed.primary_span, Some(call_span));
    assert!(transformed.labels.iter().any(|label| {
        label.style == DiagnosticLabelStyle::Secondary
            && label.span == Some(body_span)
            && label.message == Some(DiagnosticLabelMessage::GenericInstantiationBodySite)
    }));
}

#[test]
fn with_generic_instantiation_context_retains_exact_extended_call_span() {
    let body_span = make_span(1, 10, 1);
    let mut span_builder = ExtendedSpanBuilder::new();
    let call_span = SourceSpan::new(
        SourceId::from_index(4),
        LocalSpan::exact(17, 4096, &mut span_builder).unwrap(),
    );
    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
        builtin_type_ids::STRING,
        TypeMismatchContext::FunctionArgument,
        Some(body_span),
    );

    let transformed =
        with_generic_instantiation_context(diagnostic, instantiation_context(call_span, body_span));

    assert_eq!(span_builder.len(), 1);
    assert_eq!(transformed.primary_span, Some(call_span));
    assert!(
        transformed
            .labels
            .iter()
            .all(|label| label.span != Some(call_span))
    );
}

#[test]
fn with_generic_instantiation_context_preserves_existing_secondary_labels() {
    let mut string_table = StringTable::new();
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let extra_span = make_span(3, 30, 1);
    let diagnostic = CompilerDiagnostic::new(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        Some(body_span),
        DiagnosticPayload::UnknownName {
            name: string_table.intern("x"),
            namespace: crate::compiler_frontend::compiler_messages::NameNamespace::Value,
        },
    )
    .with_labels(vec![DiagnosticLabel::secondary(
        Some(extra_span),
        Some(DiagnosticLabelMessage::PreviousDeclaration),
    )]);

    let transformed =
        with_generic_instantiation_context(diagnostic, instantiation_context(call_span, body_span));

    assert!(transformed.labels.iter().any(|label| {
        label.style == DiagnosticLabelStyle::Secondary
            && label.span == Some(extra_span)
            && label.message == Some(DiagnosticLabelMessage::PreviousDeclaration)
    }));
}

#[test]
fn with_generic_instantiation_context_avoids_duplicate_body_secondary_label() {
    let mut string_table = StringTable::new();
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let diagnostic = CompilerDiagnostic::new(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        Some(body_span),
        DiagnosticPayload::UnknownName {
            name: string_table.intern("x"),
            namespace: crate::compiler_frontend::compiler_messages::NameNamespace::Value,
        },
    )
    .with_labels(vec![DiagnosticLabel::secondary(
        Some(body_span),
        Some(DiagnosticLabelMessage::PreviousDeclaration),
    )]);

    let transformed =
        with_generic_instantiation_context(diagnostic, instantiation_context(call_span, body_span));

    assert_eq!(
        transformed
            .labels
            .iter()
            .filter(|label| label.span == Some(body_span))
            .count(),
        1
    );
}

#[test]
fn with_generic_instantiation_context_adds_declaration_span_label() {
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let declaration_span = make_span(3, 30, 1);
    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
        builtin_type_ids::STRING,
        TypeMismatchContext::FunctionArgument,
        Some(body_span),
    );

    let transformed = with_generic_instantiation_context(
        diagnostic,
        instantiation_context(call_span, declaration_span),
    );

    assert!(transformed.labels.iter().any(|label| {
        label.style == DiagnosticLabelStyle::Secondary
            && label.span == Some(declaration_span)
            && label.message == Some(DiagnosticLabelMessage::GenericInstantiationDeclarationSite)
    }));
}

#[test]
fn with_generic_instantiation_context_adds_structured_substitution_label() {
    let mut string_table = StringTable::new();
    let body_span = make_span(1, 10, 1);
    let call_span = make_span(2, 20, 1);
    let declaration_span = make_span(3, 30, 1);
    let parameter_name = string_table.intern("T");
    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
        builtin_type_ids::STRING,
        TypeMismatchContext::FunctionArgument,
        Some(body_span),
    );

    let transformed = with_generic_instantiation_context(
        diagnostic,
        GenericInstantiationDiagnosticContext {
            call_span: Some(call_span),
            declaration_span: Some(declaration_span),
            substitutions: vec![GenericSubstitutionDiagnostic {
                parameter_name,
                concrete_type_id: builtin_type_ids::STRING,
            }],
        },
    );

    assert!(transformed.labels.iter().any(|label| {
        label.span == Some(declaration_span)
            && matches!(
                &label.message,
                Some(DiagnosticLabelMessage::GenericInstantiationSubstitutions {
                    substitutions
                }) if substitutions
                    == &vec![GenericSubstitutionDiagnostic {
                        parameter_name,
                        concrete_type_id: builtin_type_ids::STRING,
                    }]
            )
    }));
}

#[test]
fn conflicting_generic_function_argument_keeps_current_evidence_primary_span() {
    let mut string_table = StringTable::new();
    let current_span = make_span(5, 12, 1);
    let previous_span = make_span(5, 1, 1);
    let function_name = string_table.intern("same");
    let parameter_name = string_table.intern("T");

    let diagnostic = conflicting_generic_function_argument(
        Some(function_name),
        BindingConflict {
            parameter_id: GenericParameterId(0),
            existing_type_id: builtin_type_ids::INT,
            replacement_type_id: builtin_type_ids::STRING,
        },
        parameter_name,
        Some(current_span),
        Some(previous_span),
    );

    assert_eq!(diagnostic.primary_span, Some(current_span));
    assert!(diagnostic.labels.iter().any(|label| {
        label.style == DiagnosticLabelStyle::Secondary
            && label.span == Some(previous_span)
            && label.message == Some(DiagnosticLabelMessage::GenericInferencePreviousEvidence)
    }));
}

#[test]
fn conflicting_generic_function_argument_retains_evidence_spans() {
    let mut string_table = StringTable::new();
    let current_span = make_span(5, 2, 4096);
    let previous_span = make_span(5, 1, 4);
    let function_name = string_table.intern("same");
    let parameter_name = string_table.intern("T");

    let diagnostic = conflicting_generic_function_argument(
        Some(function_name),
        BindingConflict {
            parameter_id: GenericParameterId(0),
            existing_type_id: builtin_type_ids::INT,
            replacement_type_id: builtin_type_ids::STRING,
        },
        parameter_name,
        Some(current_span),
        Some(previous_span),
    );

    assert_eq!(diagnostic.primary_span, Some(current_span));
    assert_eq!(diagnostic.labels.len(), 1);
    assert_eq!(diagnostic.labels[0].span, Some(previous_span));
}
