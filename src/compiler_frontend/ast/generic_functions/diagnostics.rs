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
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::StringId;

/// Carries the source spans, donor identity and substitution facts needed to rewrite a
/// concrete-body diagnostic into a call-site-primary generic instantiation diagnostic.
///
/// WHAT: bundles the call-site span, generic declaration span, donor identity and substitution
///       payload consumed by `with_generic_instantiation_context`.
/// WHY: emitter code can build this once and avoid duplicating the diagnostic rewrite logic at
///      each call site.
#[derive(Clone, Debug)]
pub(crate) struct GenericInstantiationDiagnosticContext {
    pub(crate) call_span: Option<SourceSpan>,
    pub(crate) declaration_span: Option<SourceSpan>,
    pub(crate) frozen_identity_handle: Option<FrozenIdentityHandle>,
    pub(crate) call_site_frozen_identity_handle: Option<FrozenIdentityHandle>,
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

/// Rebuild a concrete-body diagnostic around optional generic instantiation provenance.
///
/// WHAT: when a call location exists, stores the original diagnostic primary location as the
///       generic body location, makes the call location primary, and keeps declaration and
///       substitution sites as secondary labels. Without a call location, preserves the original
///       body primary instead of downgrading the only available source position.
/// WHY: the call selected the concrete type arguments when authored call provenance is available,
///       while generated or synthetic requests still need the concrete body's source location.
pub(crate) fn with_generic_instantiation_context(
    mut diagnostic: CompilerDiagnostic,
    context: GenericInstantiationDiagnosticContext,
) -> CompilerDiagnostic {
    let GenericInstantiationDiagnosticContext {
        call_span,
        declaration_span,
        frozen_identity_handle,
        call_site_frozen_identity_handle,
        substitutions,
    } = context;

    if let Some(frozen_identity_handle) = frozen_identity_handle.as_ref() {
        diagnostic.attach_frozen_identity_handle_if_missing(frozen_identity_handle.clone());
    }

    let body_span = diagnostic.primary_span;
    let body_frozen_identity_handle = diagnostic.primary_frozen_identity_handle.take();
    let has_call_span = call_span.is_some();
    if let Some(call_span) = call_span {
        diagnostic.primary_span = Some(call_span);
        diagnostic.primary_frozen_identity_handle = call_site_frozen_identity_handle;
    } else {
        diagnostic.primary_span = body_span;
        diagnostic.primary_frozen_identity_handle = body_frozen_identity_handle.clone();
    }
    let mut new_labels = Vec::with_capacity(diagnostic.labels.len() + 3);

    if has_call_span
        && let Some(span) = body_span
        && !diagnostic.labels.iter().any(|label| {
            label.span == Some(span)
                && label_owner_domain_matches(
                    label.frozen_identity_handle.as_ref(),
                    body_frozen_identity_handle
                        .as_ref()
                        .or(frozen_identity_handle.as_ref()),
                )
        })
    {
        let body_frozen_identity_handle =
            body_frozen_identity_handle.or_else(|| frozen_identity_handle.clone());
        let label = match body_frozen_identity_handle {
            Some(frozen_identity_handle) => DiagnosticLabel::secondary_with_frozen_identity(
                Some(span),
                Some(DiagnosticLabelMessage::GenericInstantiationBodySite),
                frozen_identity_handle,
            ),
            None => DiagnosticLabel::secondary(
                Some(span),
                Some(DiagnosticLabelMessage::GenericInstantiationBodySite),
            ),
        };
        new_labels.push(label);
    }

    if let Some(span) = declaration_span
        && !diagnostic.labels.iter().any(|label| {
            label.span == Some(span)
                && label_owner_domain_matches(
                    label.frozen_identity_handle.as_ref(),
                    frozen_identity_handle.as_ref(),
                )
        })
    {
        let label = match frozen_identity_handle.as_ref() {
            Some(frozen_identity_handle) => DiagnosticLabel::secondary_with_frozen_identity(
                Some(span),
                Some(DiagnosticLabelMessage::GenericInstantiationDeclarationSite),
                frozen_identity_handle.clone(),
            ),
            None => DiagnosticLabel::secondary(
                Some(span),
                Some(DiagnosticLabelMessage::GenericInstantiationDeclarationSite),
            ),
        };
        new_labels.push(label);
    }

    if !substitutions.is_empty() {
        let label = match frozen_identity_handle.as_ref() {
            Some(frozen_identity_handle) => DiagnosticLabel::secondary_with_frozen_identity(
                declaration_span,
                Some(DiagnosticLabelMessage::GenericInstantiationSubstitutions { substitutions }),
                frozen_identity_handle.clone(),
            ),
            None => DiagnosticLabel::secondary(
                declaration_span,
                Some(DiagnosticLabelMessage::GenericInstantiationSubstitutions { substitutions }),
            ),
        };
        new_labels.push(label);
    }
    new_labels.extend(diagnostic.labels);
    diagnostic.labels = new_labels;
    diagnostic
}

fn label_owner_domain_matches(
    label_owner: Option<&FrozenIdentityHandle>,
    expected_owner: Option<&FrozenIdentityHandle>,
) -> bool {
    match (label_owner, expected_owner) {
        (None, None) => true,
        (None, Some(_)) | (Some(_), None) => false,
        (Some(label_owner), Some(expected_owner)) => {
            match (label_owner.domain(), expected_owner.domain()) {
                (Some(label_domain), Some(expected_domain)) => label_domain == expected_domain,
                _ => label_owner == expected_owner,
            }
        }
    }
}
