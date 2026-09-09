//! Generic function diagnostics owned by AST call/template handling.
//!
//! WHAT: provides focused constructors and helpers for generic function inference, concrete
//! instantiation context, and unsupported generic function value use.
//! WHY: call parsing and instance emission should report structured generic facts without
//! knowing diagnostic rendering details.

use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticLabel, DiagnosticLabelMessage, GenericInferenceSubject,
    GenericSubstitutionDiagnostic, InvalidGenericInstantiationReason,
};
use crate::compiler_frontend::datatypes::generic_bindings::BindingConflict;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;

/// Carries the source spans and substitution facts needed to rewrite a
/// concrete-body diagnostic into a call-site-primary generic instantiation diagnostic.
///
/// WHAT: bundles the call-site span, generic declaration span, and substitution
/// payload consumed by `with_generic_instantiation_context`.
/// WHY: emitter code can build this once and avoid duplicating the diagnostic
/// rewrite logic at each call site.
#[derive(Clone, Debug)]
pub(crate) struct GenericInstantiationDiagnosticContext {
    pub(crate) call_span: Option<SourceSpan>,
    pub(crate) declaration_span: Option<SourceSpan>,
    pub(crate) substitutions: Vec<GenericSubstitutionDiagnostic>,
}

pub(crate) fn cannot_infer_generic_function_arguments(
    function_name: Option<StringId>,
    missing_parameters: Vec<StringId>,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_generic_instantiation(
        function_name,
        InvalidGenericInstantiationReason::CannotInferFunctionArguments { missing_parameters },
        span,
    )
}

pub(crate) fn conflicting_generic_function_argument(
    function_name: Option<StringId>,
    conflict: BindingConflict,
    parameter_name: StringId,
    current_evidence_span: Option<SourceSpan>,
    previous_evidence_span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::conflicting_generic_inference(
        function_name,
        GenericInferenceSubject::Function,
        conflict,
        parameter_name,
        current_evidence_span,
        previous_evidence_span,
    )
}

pub(crate) fn missing_generic_function_trait_evidence(
    function_name: Option<StringId>,
    parameter_name: StringId,
    trait_name: StringId,
    concrete_type_id: TypeId,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_generic_instantiation(
        function_name,
        InvalidGenericInstantiationReason::MissingTraitEvidence {
            parameter_name,
            trait_name,
            concrete_type_id,
        },
        span,
    )
}

pub(crate) fn recursive_generic_function_instantiation(
    function_name: Option<StringId>,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_generic_instantiation(
        function_name,
        InvalidGenericInstantiationReason::RecursiveFunctionInstantiation,
        span,
    )
}

pub(crate) fn with_generic_primary_span(
    mut diagnostic: CompilerDiagnostic,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    if let Some(span) = span {
        diagnostic.primary_span = Some(span);
    }
    diagnostic
}

/// Rebuild a concrete-body diagnostic so the generic call site is primary.
///
/// WHAT: stores the original diagnostic primary location as the generic body location,
/// makes the call location the diagnostic primary location, and rebuilds labels so the
/// body, declaration, and substitution sites remain secondary.
/// WHY: the call selected the concrete type arguments, so the call site should be primary
/// and the generic body span should be secondary.
pub(crate) fn with_generic_instantiation_context(
    mut diagnostic: CompilerDiagnostic,
    context: GenericInstantiationDiagnosticContext,
) -> CompilerDiagnostic {
    let GenericInstantiationDiagnosticContext {
        call_span,
        declaration_span,
        substitutions,
    } = context;

    let body_span = diagnostic.primary_span;
    diagnostic.primary_span = call_span;
    let mut new_labels = Vec::with_capacity(diagnostic.labels.len() + 3);

    if let Some(span) = body_span
        && !diagnostic
            .labels
            .iter()
            .any(|label| label.span == Some(span))
    {
        new_labels.push(DiagnosticLabel::secondary(
            Some(span),
            Some(DiagnosticLabelMessage::GenericInstantiationBodySite),
        ));
    }

    if let Some(span) = declaration_span
        && !diagnostic
            .labels
            .iter()
            .any(|label| label.span == Some(span))
    {
        new_labels.push(DiagnosticLabel::secondary(
            Some(span),
            Some(DiagnosticLabelMessage::GenericInstantiationDeclarationSite),
        ));
    }

    if !substitutions.is_empty() {
        new_labels.push(DiagnosticLabel::secondary(
            declaration_span,
            Some(DiagnosticLabelMessage::GenericInstantiationSubstitutions { substitutions }),
        ));
    }

    new_labels.extend(diagnostic.labels);
    diagnostic.labels = new_labels;
    diagnostic
}
