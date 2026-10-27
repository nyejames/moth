//! Generic function diagnostics owned by AST call/template handling.
//!
//! WHAT: provides focused constructors and helpers for generic function inference, concrete
//! instantiation context, and unsupported generic function value use.
//! WHY: call parsing and instance emission should report structured generic facts without
//! knowing diagnostic rendering details.

use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api::{
    conflicting_generic_inference, invalid_generic_instantiation,
};
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelDraftMessage;
use crate::compiler_frontend::compiler_messages::{
    GenericInferenceSubject, GenericSubstitutionDiagnostic, InvalidGenericInstantiationReason,
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
) -> DiagnosticDraft {
    invalid_generic_instantiation(
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
) -> DiagnosticDraft {
    conflicting_generic_inference(
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
) -> DiagnosticDraft {
    invalid_generic_instantiation(
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
) -> DiagnosticDraft {
    invalid_generic_instantiation(
        function_name,
        InvalidGenericInstantiationReason::RecursiveFunctionInstantiation,
        span,
    )
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
    mut diagnostic: DiagnosticDraft,
    context: GenericInstantiationDiagnosticContext,
) -> DiagnosticDraft {
    let GenericInstantiationDiagnosticContext {
        call_span,
        declaration_span,
        frozen_identity_handle,
        call_site_frozen_identity_handle,
        substitutions,
    } = context;

    let body_span = diagnostic.primary_span();
    if let Some(handle) = frozen_identity_handle.as_ref() {
        diagnostic.attach_frozen_identity_handle_if_missing(handle.clone());
    }
    let body_frozen_identity_handle = diagnostic.primary_frozen_identity_handle().cloned();
    let has_call_span = call_span.is_some();
    if let Some(call_span) = call_span {
        diagnostic.set_primary_span(Some(call_span));
        diagnostic.replace_primary_frozen_identity_handle(call_site_frozen_identity_handle);
    } else {
        diagnostic.set_primary_span(body_span);
        diagnostic.replace_primary_frozen_identity_handle(body_frozen_identity_handle.clone());
    }

    let mut insertion_index = 0;
    let add_body_label = has_call_span
        && body_span.is_some_and(|span| {
            !diagnostic.has_label_at_with_owner(Some(span), body_frozen_identity_handle.as_ref())
        });
    let add_declaration_label = declaration_span.is_some_and(|span| {
        !diagnostic.has_label_at_with_owner(Some(span), frozen_identity_handle.as_ref())
    });

    if add_body_label {
        let span = body_span.expect("a body label requires the original primary span");
        diagnostic = diagnostic.with_label_at(
            insertion_index,
            Some(span),
            Some(LabelDraftMessage::GenericInstantiationBodySite),
        );
        if let Some(handle) = body_frozen_identity_handle {
            diagnostic = diagnostic.with_label_frozen_identity_handle(insertion_index, handle);
        }
        insertion_index += 1;
    }

    if add_declaration_label {
        let span = declaration_span.expect("a declaration label requires its span");
        diagnostic = diagnostic.with_label_at(
            insertion_index,
            Some(span),
            Some(LabelDraftMessage::GenericInstantiationDeclarationSite),
        );
        if let Some(handle) = frozen_identity_handle.as_ref() {
            diagnostic =
                diagnostic.with_label_frozen_identity_handle(insertion_index, handle.clone());
        }
        insertion_index += 1;
    }

    if !substitutions.is_empty() {
        diagnostic = diagnostic.with_label_at(
            insertion_index,
            declaration_span,
            Some(LabelDraftMessage::GenericInstantiationSubstitutions { substitutions }),
        );
        if let Some(handle) = frozen_identity_handle {
            diagnostic = diagnostic.with_label_frozen_identity_handle(insertion_index, handle);
        }
    }

    diagnostic
}
