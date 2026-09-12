//! Shared receiver-access validation for postfix calls.
//!
//! WHAT: validates whether a receiver call needs `~`, a mutable place, or no mutable marker.
//! WHY: collection builtins, map builtins and user receiver methods share one access policy
//! but need caller-specific diagnostic wording. One classifier distinguishes non-place,
//! immutable-place and mutable-place receivers so each source state gets distinct guidance.
//!
//! Validation results carry plain `CompilerDiagnostic` values on the diagnosed
//! lane. Callers that already hold `ExpressionParseError::Diagnostic` preserve
//! that value through the direct conversion.

use super::ReceiverAccessMode;
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::place_access::{
    ReceiverSourceState, classify_receiver_source_state,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidReceiverCallReason, ReceiverCallKind,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringId;
/// Which receiver-call surface owns a receiver-access diagnostic, plus the method name.
///
/// WHAT: carries the method name and maps the access context to the `ReceiverCallKind` payload
///       fact the renderer uses to name the receiver kind.
/// WHY: source methods, collection builtins and map builtins share one classifier; the kind
///      only selects the rendered noun, so it stays out of the reason enum.
pub(super) enum ReceiverAccessDiagnostic {
    CollectionBuiltin { method_name: StringId },
    MapBuiltin { method_name: StringId },
    ReceiverMethod { method_name: StringId },
}

impl ReceiverAccessDiagnostic {
    fn method_name(&self) -> StringId {
        match self {
            ReceiverAccessDiagnostic::CollectionBuiltin { method_name }
            | ReceiverAccessDiagnostic::MapBuiltin { method_name }
            | ReceiverAccessDiagnostic::ReceiverMethod { method_name } => *method_name,
        }
    }

    fn receiver_kind(&self) -> ReceiverCallKind {
        match self {
            ReceiverAccessDiagnostic::CollectionBuiltin { .. } => {
                ReceiverCallKind::CollectionBuiltin
            }
            ReceiverAccessDiagnostic::MapBuiltin { .. } => ReceiverCallKind::MapBuiltin,
            ReceiverAccessDiagnostic::ReceiverMethod { .. } => ReceiverCallKind::SourceMethod,
        }
    }
}

pub(super) struct ReceiverAccessRequirement {
    pub requires_mutable: bool,
    pub diagnostic: ReceiverAccessDiagnostic,
}

type ReceiverAccessResult = Result<(), CompilerDiagnostic>;

// --------------------------
//  Validation entry point
// --------------------------

pub(super) fn validate_receiver_access(
    receiver_node: &AstNode,
    path_fork: &PathInternerFork,
    access_mode: ReceiverAccessMode,
    method_boundary_span: Option<SourceSpan>,
    authored_marker_span: Option<SourceSpan>,
    access_requirement: ReceiverAccessRequirement,
) -> ReceiverAccessResult {
    // A call that does not require mutable access rejects an authored `~` at the marker, since
    // the marker is the source the author must remove.
    if !access_requirement.requires_mutable {
        if access_mode == ReceiverAccessMode::Mutable {
            return reject_unneeded_mutable_access_marker(
                &access_requirement.diagnostic,
                authored_marker_span,
                method_boundary_span,
            );
        }
        return Ok(());
    }

    let source_state = classify_receiver_source_state(receiver_node, path_fork);

    match (access_mode, source_state) {
        // An existing mutable place needs the explicit `~` marker. The method boundary is the
        // call site the author must prefix; the authored marker is absent here. The binding name
        // lets the renderer show a concrete `~name.method(...)` example when it is known.
        (ReceiverAccessMode::Shared, ReceiverSourceState::MutablePlace { binding_name }) => {
            reject_mutable_receiver_missing_marker(
                &access_requirement.diagnostic,
                binding_name,
                method_boundary_span,
            )
        }
        // An immutable existing place cannot be repaired by adding `~`: the binding itself must
        // be declared mutable. No marker was authored, so point at the method boundary.
        (ReceiverAccessMode::Shared, ReceiverSourceState::ImmutablePlace { binding_name }) => {
            reject_immutable_receiver_mutable_method(
                &access_requirement.diagnostic,
                binding_name,
                method_boundary_span,
            )
        }
        // A temporary or non-place receiver cannot be mutated through. No marker was authored,
        // so point at the method boundary.
        (ReceiverAccessMode::Shared, ReceiverSourceState::Temporary) => {
            reject_non_place_receiver_mutable_method(
                &access_requirement.diagnostic,
                method_boundary_span,
            )
        }
        // An existing mutable place with an authored `~` satisfies the call.
        (ReceiverAccessMode::Mutable, ReceiverSourceState::MutablePlace { .. }) => Ok(()),
        // `~` authored on an immutable place: the marker is the source the author must change,
        // and the binding must be declared mutable before the marker is valid.
        (ReceiverAccessMode::Mutable, ReceiverSourceState::ImmutablePlace { binding_name }) => {
            reject_mutable_marker_on_immutable_receiver(
                &access_requirement.diagnostic,
                binding_name,
                authored_marker_span,
                method_boundary_span,
            )
        }
        // `~` authored on a temporary or non-place value: the marker is invalid because `~`
        // accepts only an existing mutable place.
        (ReceiverAccessMode::Mutable, ReceiverSourceState::Temporary) => {
            reject_mutable_marker_on_non_place_receiver(
                &access_requirement.diagnostic,
                authored_marker_span,
                method_boundary_span,
            )
        }
    }
}

// --------------------------
//  Rejection helpers
// --------------------------

fn reject_mutable_receiver_missing_marker(
    access_diagnostic: &ReceiverAccessDiagnostic,
    binding_name: Option<StringId>,
    method_boundary_span: Option<SourceSpan>,
) -> ReceiverAccessResult {
    reject(
        InvalidReceiverCallReason::MutableReceiverMissingMarker,
        access_diagnostic,
        binding_name,
        method_boundary_span,
    )
}

fn reject_immutable_receiver_mutable_method(
    access_diagnostic: &ReceiverAccessDiagnostic,
    binding_name: Option<StringId>,
    method_boundary_span: Option<SourceSpan>,
) -> ReceiverAccessResult {
    reject(
        InvalidReceiverCallReason::ImmutableReceiverMutableMethod,
        access_diagnostic,
        binding_name,
        method_boundary_span,
    )
}

fn reject_non_place_receiver_mutable_method(
    access_diagnostic: &ReceiverAccessDiagnostic,
    method_boundary_span: Option<SourceSpan>,
) -> ReceiverAccessResult {
    reject(
        InvalidReceiverCallReason::NonPlaceReceiverMutableMethod,
        access_diagnostic,
        None,
        method_boundary_span,
    )
}

fn reject_mutable_marker_on_immutable_receiver(
    access_diagnostic: &ReceiverAccessDiagnostic,
    binding_name: Option<StringId>,
    authored_marker_span: Option<SourceSpan>,
    method_boundary_span: Option<SourceSpan>,
) -> ReceiverAccessResult {
    reject(
        InvalidReceiverCallReason::MutableMarkerOnImmutableReceiver,
        access_diagnostic,
        binding_name,
        authored_marker_span.or(method_boundary_span),
    )
}

fn reject_mutable_marker_on_non_place_receiver(
    access_diagnostic: &ReceiverAccessDiagnostic,
    authored_marker_span: Option<SourceSpan>,
    method_boundary_span: Option<SourceSpan>,
) -> ReceiverAccessResult {
    reject(
        InvalidReceiverCallReason::MutableMarkerOnNonPlaceReceiver,
        access_diagnostic,
        None,
        authored_marker_span.or(method_boundary_span),
    )
}

fn reject_unneeded_mutable_access_marker(
    access_diagnostic: &ReceiverAccessDiagnostic,
    authored_marker_span: Option<SourceSpan>,
    method_boundary_span: Option<SourceSpan>,
) -> ReceiverAccessResult {
    reject(
        InvalidReceiverCallReason::UnneededMutableAccessMarker,
        access_diagnostic,
        None,
        authored_marker_span.or(method_boundary_span),
    )
}

/// Builds the shared receiver-access diagnostic from the reason, access context and span.
///
/// WHAT: threads the method name, receiver kind and optional simple receiver binding name into
///       the structured payload, and never repurposes the type field as a value name.
/// WHY: every receiver-access rejection shares one payload shape, so the renderer can name the
///      receiver kind and binding from facts instead of guessing from a type label.
fn reject(
    reason: InvalidReceiverCallReason,
    access_diagnostic: &ReceiverAccessDiagnostic,
    receiver_binding_name: Option<StringId>,
    span: Option<SourceSpan>,
) -> ReceiverAccessResult {
    Err(CompilerDiagnostic::invalid_receiver_call(
        reason,
        None,
        Some(access_diagnostic.method_name()),
        Some(access_diagnostic.receiver_kind()),
        receiver_binding_name,
        span,
    ))
}
