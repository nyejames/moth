//! Unit tests for generic function diagnostic helpers.
//!
//! WHAT: asserts that `with_generic_instantiation_context` rebuilds diagnostics correctly.
//! WHY: the helper is the single point of truth for call-site-primary generic instantiation
//! diagnostics and should be tested in isolation.

use crate::compiler_frontend::ast::generic_functions::diagnostics::{
    GenericInstantiationDiagnosticContext, conflicting_generic_function_argument,
    with_generic_instantiation_context,
};
use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticLabel, DiagnosticLabelMessage,
    DiagnosticLabelStyle, DiagnosticPayload, GenericSubstitutionDiagnostic, RuleDiagnosticKind,
    TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::generic_bindings::BindingConflict;
use crate::compiler_frontend::datatypes::ids::GenericParameterId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

fn make_location(
    path: &str,
    line: i32,
    col: i32,
    string_table: &mut StringTable,
) -> SourceLocation {
    let interned_path = InternedPath::from_single_str(path, string_table);
    SourceLocation::new(
        interned_path,
        CharPosition {
            line_number: line,
            char_column: col,
        },
        CharPosition {
            line_number: line,
            char_column: col + 1,
        },
    )
}

fn instantiation_context(
    call_location: SourceLocation,
    declaration_location: SourceLocation,
) -> GenericInstantiationDiagnosticContext {
    GenericInstantiationDiagnosticContext {
        call_location,
        declaration_location,
        substitutions: Vec::new(),
    }
}

#[test]
fn with_generic_instantiation_context_changes_primary_location_to_call_site() {
    let mut string_table = StringTable::new();
    let body_location = make_location("body.moth", 10, 5, &mut string_table);
    let call_location = make_location("call.moth", 20, 8, &mut string_table);

    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
        builtin_type_ids::STRING,
        TypeMismatchContext::FunctionArgument,
        body_location.clone(),
    );

    let transformed = with_generic_instantiation_context(
        diagnostic,
        instantiation_context(call_location.clone(), body_location),
    );

    assert_eq!(transformed.primary_location, call_location);
}

#[test]
fn with_generic_instantiation_context_first_label_is_primary_call_site() {
    let mut string_table = StringTable::new();
    let body_location = make_location("body.moth", 10, 5, &mut string_table);
    let call_location = make_location("call.moth", 20, 8, &mut string_table);

    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
        builtin_type_ids::STRING,
        TypeMismatchContext::FunctionArgument,
        body_location.clone(),
    );

    let transformed = with_generic_instantiation_context(
        diagnostic,
        instantiation_context(call_location.clone(), body_location),
    );

    let first_label = transformed
        .labels
        .first()
        .expect("expected at least one label");
    assert_eq!(first_label.location, call_location);
    assert_eq!(first_label.style, DiagnosticLabelStyle::Primary);
    assert_eq!(
        first_label.message,
        Some(DiagnosticLabelMessage::GenericInstantiationCallSite)
    );
}

#[test]
fn with_generic_instantiation_context_adds_secondary_body_site_label() {
    let mut string_table = StringTable::new();
    let body_location = make_location("body.moth", 10, 5, &mut string_table);
    let call_location = make_location("call.moth", 20, 8, &mut string_table);

    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
        builtin_type_ids::STRING,
        TypeMismatchContext::FunctionArgument,
        body_location.clone(),
    );

    let transformed = with_generic_instantiation_context(
        diagnostic,
        instantiation_context(call_location, body_location.clone()),
    );

    let body_label = transformed
        .labels
        .iter()
        .find(|label| {
            label.style == DiagnosticLabelStyle::Secondary
                && label.location == body_location
                && label.message == Some(DiagnosticLabelMessage::GenericInstantiationBodySite)
        })
        .expect("expected a secondary body-site label");

    assert_eq!(body_label.location, body_location);
}

#[test]
fn with_generic_instantiation_context_preserves_existing_secondary_labels() {
    let mut string_table = StringTable::new();
    let body_location = make_location("body.moth", 10, 5, &mut string_table);
    let call_location = make_location("call.moth", 20, 8, &mut string_table);
    let extra_location = make_location("extra.moth", 15, 3, &mut string_table);

    let diagnostic = CompilerDiagnostic::new(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        body_location.clone(),
        DiagnosticPayload::UnknownName {
            name: string_table.intern("x"),
            namespace: crate::compiler_frontend::compiler_messages::NameNamespace::Value,
        },
    )
    .with_labels(vec![DiagnosticLabel::secondary(
        extra_location.clone(),
        Some(DiagnosticLabelMessage::PreviousDeclaration),
    )]);

    let transformed = with_generic_instantiation_context(
        diagnostic,
        instantiation_context(call_location, body_location),
    );

    let extra_label = transformed
        .labels
        .iter()
        .find(|label| {
            label.style == DiagnosticLabelStyle::Secondary
                && label.location == extra_location
                && label.message == Some(DiagnosticLabelMessage::PreviousDeclaration)
        })
        .expect("expected the existing secondary label to be preserved");

    assert_eq!(extra_label.location, extra_location);
}

#[test]
fn with_generic_instantiation_context_avoids_duplicate_body_secondary_label() {
    let mut string_table = StringTable::new();
    let body_location = make_location("body.moth", 10, 5, &mut string_table);
    let call_location = make_location("call.moth", 20, 8, &mut string_table);

    // Create a diagnostic where the body location is already a secondary label.
    let diagnostic = CompilerDiagnostic::new(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        body_location.clone(),
        DiagnosticPayload::UnknownName {
            name: string_table.intern("x"),
            namespace: crate::compiler_frontend::compiler_messages::NameNamespace::Value,
        },
    )
    .with_labels(vec![DiagnosticLabel::secondary(
        body_location.clone(),
        Some(DiagnosticLabelMessage::PreviousDeclaration),
    )]);

    let transformed = with_generic_instantiation_context(
        diagnostic,
        instantiation_context(call_location, body_location.clone()),
    );

    let body_secondary_count = transformed
        .labels
        .iter()
        .filter(|label| {
            label.style == DiagnosticLabelStyle::Secondary && label.location == body_location
        })
        .count();

    assert_eq!(
        body_secondary_count, 1,
        "body location should appear as secondary only once"
    );
}

#[test]
fn with_generic_instantiation_context_drops_original_primary_label() {
    let mut string_table = StringTable::new();
    let body_location = make_location("body.moth", 10, 5, &mut string_table);
    let call_location = make_location("call.moth", 20, 8, &mut string_table);

    // Create a diagnostic that already has a primary label at the body location.
    let diagnostic = CompilerDiagnostic::new(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        body_location.clone(),
        DiagnosticPayload::UnknownName {
            name: string_table.intern("x"),
            namespace: crate::compiler_frontend::compiler_messages::NameNamespace::Value,
        },
    )
    .with_labels(vec![DiagnosticLabel {
        location: body_location.clone(),
        style: DiagnosticLabelStyle::Primary,
        message: Some(DiagnosticLabelMessage::PreviousDeclaration),
    }]);

    let transformed = with_generic_instantiation_context(
        diagnostic,
        instantiation_context(call_location.clone(), body_location.clone()),
    );

    // No primary label should remain at the old body location.
    assert!(
        !transformed.labels.iter().any(|label| {
            label.style == DiagnosticLabelStyle::Primary && label.location == body_location
        }),
        "original primary label at body location should be removed"
    );

    // The first label should be the new primary at call_location.
    let first_label = transformed.labels.first().unwrap();
    assert_eq!(first_label.location, call_location);
    assert_eq!(first_label.style, DiagnosticLabelStyle::Primary);
}

#[test]
fn with_generic_instantiation_context_adds_declaration_site_label() {
    let mut string_table = StringTable::new();
    let body_location = make_location("body.moth", 10, 5, &mut string_table);
    let call_location = make_location("call.moth", 20, 8, &mut string_table);
    let declaration_location = make_location("declaration.moth", 2, 1, &mut string_table);

    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
        builtin_type_ids::STRING,
        TypeMismatchContext::FunctionArgument,
        body_location,
    );

    let transformed = with_generic_instantiation_context(
        diagnostic,
        instantiation_context(call_location, declaration_location.clone()),
    );

    let declaration_label = transformed
        .labels
        .iter()
        .find(|label| {
            label.style == DiagnosticLabelStyle::Secondary
                && label.location == declaration_location
                && label.message
                    == Some(DiagnosticLabelMessage::GenericInstantiationDeclarationSite)
        })
        .expect("expected a secondary declaration-site label");

    assert_eq!(declaration_label.location, declaration_location);
}

#[test]
fn with_generic_instantiation_context_adds_structured_substitution_label() {
    let mut string_table = StringTable::new();
    let body_location = make_location("body.moth", 10, 5, &mut string_table);
    let call_location = make_location("call.moth", 20, 8, &mut string_table);
    let declaration_location = make_location("declaration.moth", 2, 1, &mut string_table);
    let parameter_name = string_table.intern("T");

    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
        builtin_type_ids::STRING,
        TypeMismatchContext::FunctionArgument,
        body_location,
    );

    let transformed = with_generic_instantiation_context(
        diagnostic,
        GenericInstantiationDiagnosticContext {
            call_location,
            declaration_location: declaration_location.clone(),
            substitutions: vec![GenericSubstitutionDiagnostic {
                parameter_name,
                concrete_type_id: builtin_type_ids::STRING,
            }],
        },
    );

    let substitution_label = transformed
        .labels
        .iter()
        .find(|label| {
            label.style == DiagnosticLabelStyle::Secondary
                && label.location == declaration_location
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
        })
        .expect("expected a structured substitution label");

    assert_eq!(substitution_label.location, declaration_location);
}

#[test]
fn conflicting_generic_function_argument_keeps_current_evidence_primary_label() {
    let mut string_table = StringTable::new();
    let current_location = make_location("call.moth", 12, 4, &mut string_table);
    let previous_location = make_location("call.moth", 12, 1, &mut string_table);
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
        current_location.clone(),
        Some(previous_location.clone()),
    );

    assert_eq!(diagnostic.primary_location, current_location);
    assert!(
        diagnostic.labels.iter().any(|label| {
            label.style == DiagnosticLabelStyle::Primary && label.location == current_location
        }),
        "expected current evidence to remain represented by a primary label"
    );
    assert!(
        diagnostic.labels.iter().any(|label| {
            label.style == DiagnosticLabelStyle::Secondary
                && label.location == previous_location
                && label.message == Some(DiagnosticLabelMessage::GenericInferencePreviousEvidence)
        }),
        "expected previous evidence to be represented by a secondary label"
    );
}
