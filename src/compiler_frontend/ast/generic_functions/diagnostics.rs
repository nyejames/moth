//! Generic function diagnostics owned by AST call/template handling.
//!
//! WHAT: provides focused constructors and helpers for generic function inference, concrete
//! instantiation context, and unsupported generic function value use.
//! WHY: call parsing and instance emission should report structured generic facts without
//! knowing diagnostic rendering details.

use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticLabel, DiagnosticLabelMessage, DiagnosticLabelStyle,
    GenericInferenceSubject, GenericSubstitutionDiagnostic, InvalidGenericInstantiationReason,
};
use crate::compiler_frontend::datatypes::generic_bindings::BindingConflict;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

/// Carries the source locations and substitution facts needed to rewrite a
/// concrete-body diagnostic into a call-site-primary generic instantiation diagnostic.
///
/// WHAT: bundles the call-site span, generic declaration span, and substitution
/// payload consumed by `with_generic_instantiation_context`.
/// WHY: emitter code can build this once and avoid duplicating the diagnostic
/// rewrite logic at each call site.
#[derive(Clone, Debug)]
pub(crate) struct GenericInstantiationDiagnosticContext {
    pub(crate) call_location: SourceLocation,
    pub(crate) call_span: Option<SourceSpan>,
    pub(crate) declaration_location: SourceLocation,
    pub(crate) substitutions: Vec<GenericSubstitutionDiagnostic>,
}

pub(crate) fn cannot_infer_generic_function_arguments(
    function_name: Option<StringId>,
    missing_parameters: Vec<StringId>,
    location: SourceLocation,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    with_generic_primary_span(
        CompilerDiagnostic::invalid_generic_instantiation(
            function_name,
            InvalidGenericInstantiationReason::CannotInferFunctionArguments { missing_parameters },
            location,
        ),
        span,
    )
}

pub(crate) fn conflicting_generic_function_argument(
    function_name: Option<StringId>,
    conflict: BindingConflict,
    parameter_name: StringId,
    current_evidence_location: SourceLocation,
    previous_evidence_location: Option<SourceLocation>,
    current_evidence_span: Option<SourceSpan>,
    previous_evidence_span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::conflicting_generic_inference(
        function_name,
        GenericInferenceSubject::Function,
        conflict,
        parameter_name,
        current_evidence_location,
        previous_evidence_location,
        current_evidence_span,
        previous_evidence_span,
    )
}

pub(crate) fn missing_generic_function_trait_evidence(
    function_name: Option<StringId>,
    parameter_name: StringId,
    trait_name: StringId,
    concrete_type_id: TypeId,
    location: SourceLocation,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    with_generic_primary_span(
        CompilerDiagnostic::invalid_generic_instantiation(
            function_name,
            InvalidGenericInstantiationReason::MissingTraitEvidence {
                parameter_name,
                trait_name,
                concrete_type_id,
            },
            location,
        ),
        span,
    )
}

pub(crate) fn recursive_generic_function_instantiation(
    function_name: Option<StringId>,
    location: SourceLocation,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    with_generic_primary_span(
        CompilerDiagnostic::invalid_generic_instantiation(
            function_name,
            InvalidGenericInstantiationReason::RecursiveFunctionInstantiation,
            location,
        ),
        span,
    )
}

pub(crate) fn with_generic_primary_span(
    mut diagnostic: CompilerDiagnostic,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    if let Some(span) = span {
        diagnostic.primary_span = Some(span);
        if let Some(label) = diagnostic
            .labels
            .iter_mut()
            .find(|label| label.style == DiagnosticLabelStyle::Primary)
        {
            label.span = Some(span);
        }
    }
    diagnostic
}

/// Rebuild a concrete-body diagnostic so the generic call site is primary.
///
/// WHAT: stores the original diagnostic primary location as the generic body location,
/// makes the call location the diagnostic primary location, and rebuilds labels so the
/// first label is a primary call-site label with `GenericInstantiationCallSite`.
/// WHY: the call selected the concrete type arguments, so the call site should be primary
/// and the generic body span should be secondary.
pub(crate) fn with_generic_instantiation_context(
    mut diagnostic: CompilerDiagnostic,
    context: GenericInstantiationDiagnosticContext,
) -> CompilerDiagnostic {
    let body_location = diagnostic.primary_location.clone();
    let GenericInstantiationDiagnosticContext {
        call_location,
        call_span,
        declaration_location,
        substitutions,
    } = context;

    // The call site selected the concrete type arguments, so it becomes primary.
    diagnostic.primary_location = call_location.clone();
    let body_span = diagnostic.primary_span;
    diagnostic.primary_span = call_span;

    let mut new_labels = Vec::with_capacity(diagnostic.labels.len() + 4);

    // Primary call-site label.
    new_labels.push(DiagnosticLabel {
        location: call_location,
        span: call_span,
        style: DiagnosticLabelStyle::Primary,
        message: Some(DiagnosticLabelMessage::GenericInstantiationCallSite),
    });

    // Avoid duplicate body-site secondary if that body location is already present.
    let body_already_secondary = diagnostic.labels.iter().any(|label| {
        label.style == DiagnosticLabelStyle::Secondary && label.location == body_location
    });

    if !body_already_secondary {
        let mut body_label = DiagnosticLabel::secondary(
            body_location,
            Some(DiagnosticLabelMessage::GenericInstantiationBodySite),
        );
        body_label.span = body_span;
        new_labels.push(body_label);
    } else if let Some(body_span) = body_span
        && let Some(body_label) = diagnostic.labels.iter_mut().find(|label| {
            label.style == DiagnosticLabelStyle::Secondary && label.location == body_location
        })
        && body_label.span.is_none()
    {
        body_label.span = Some(body_span);
    }

    let declaration_label_already_present = diagnostic
        .labels
        .iter()
        .any(|label| label.location == declaration_location);

    if !declaration_label_already_present {
        new_labels.push(DiagnosticLabel::secondary(
            declaration_location.clone(),
            Some(DiagnosticLabelMessage::GenericInstantiationDeclarationSite),
        ));
    }

    if !substitutions.is_empty() {
        new_labels.push(DiagnosticLabel::secondary(
            declaration_location,
            Some(DiagnosticLabelMessage::GenericInstantiationSubstitutions { substitutions }),
        ));
    }

    // Preserve existing non-primary labels from the original diagnostic.
    for label in diagnostic.labels {
        if label.style != DiagnosticLabelStyle::Primary {
            new_labels.push(label);
        }
    }

    diagnostic.labels = new_labels;
    diagnostic
}
