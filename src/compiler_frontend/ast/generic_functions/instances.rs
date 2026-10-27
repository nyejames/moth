//! Generic function instance identity.
//!
//! WHAT: defines the canonical key and record shape for concrete generic function and receiver
//! instances.
//! WHY: call inference and emission deduplicate instances by source function path and canonical
//! `TypeId` arguments, not by rendered names or local dependency aliases.

use crate::compiler_frontend::ast::generic_bounds::BoundEvidenceSelection;
use crate::compiler_frontend::compiler_messages::CapacityResource;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::semantic_identity::GeneratedDeclarationIdentity;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct GenericFunctionInstanceKey {
    pub(crate) function_path: PathId,
    pub(crate) type_arguments: Box<[TypeId]>,
}

#[derive(Debug, Clone)]
pub(crate) struct GenericFunctionInstance {
    pub(crate) instance_path: PathId,
    pub(crate) key: GenericFunctionInstanceKey,
}

/// Concrete generic demand retained by the ordered AST request ledger.
///
/// WHAT: owns the declaration, selected arguments and evidence needed by the shared
/// provider materialisation route before HIR failure-summary convergence.
/// WHY: executable calls and inert assertion witnesses need the same concrete semantic
/// identity, while only executable reachability publishes a generated sidecar.
#[derive(Debug, Clone)]
pub(crate) struct GenericFunctionInstantiationRequest {
    /// Imported public contracts already carry their origin. Local requests receive either a
    /// public origin or an artefact-scoped private identity before stable-request
    /// canonicalisation.
    pub(crate) declaration_identity: Option<GeneratedDeclarationIdentity>,
    /// Ordered requester-local evidence selections, canonicalized when the stable request is
    /// installed.
    pub(crate) evidence: Box<[BoundEvidenceSelection]>,
    pub(crate) key: GenericFunctionInstanceKey,
    pub(crate) instance_path: PathId,
    pub(crate) call_span: Option<SourceSpan>,
}

impl GenericFunctionInstantiationRequest {
    pub(crate) fn generated(
        declaration_identity: &GeneratedDeclarationIdentity,
        function_path: PathId,
        type_arguments: Box<[TypeId]>,
        path_fork: &mut PathInternerFork,
        string_table: &mut StringTable,
        call_span: Option<SourceSpan>,
    ) -> Result<Self, DiagnosticDraft> {
        let instance_component = string_table.intern("__generated_instance");
        let instance_path = path_fork
            .try_intern_child(function_path, instance_component)
            .ok_or_else(|| {
                token_lexical::compiler_capacity_exceeded(CapacityResource::PathNodes, call_span)
            })?;
        Ok(Self {
            declaration_identity: Some(declaration_identity.clone()),
            evidence: Box::new([]),
            key: GenericFunctionInstanceKey {
                function_path,
                type_arguments,
            },
            instance_path,
            call_span,
        })
    }
}

/// Half-open slice of provisional generic requests emitted while parsing one branch body.
///
/// WHAT: records the shared request sink positions before and after a fully validated branch.
/// WHY: static Bool specialisation happens after parsing and constant finalisation, so it needs
/// to discard inactive branch work without rediscovering generic calls from the AST.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct GenericRequestRange {
    pub(crate) start: usize,
    pub(crate) end: usize,
}

impl GenericRequestRange {
    pub(crate) fn new(start: usize, end: usize) -> Self {
        debug_assert!(start <= end, "generic request range must be ordered");
        Self { start, end }
    }

    #[cfg(feature = "benchmark_counters")]
    pub(crate) fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }
}

/// Provisional generic-request ownership for the two authored bodies of an `if`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct IfGenericRequestRanges {
    pub(crate) then_branch: GenericRequestRange,
    pub(crate) else_branch: GenericRequestRange,
}

/// One position in the body-local ordered generic-request ledger.
///
/// WHAT: numeric-dependent calls reserve a pending slot and complete it only after exact types
///       and bound evidence validate. Known-true assertions retain validation-only candidates in
///       the same slots, while unrelated inactive work is marked `Dropped`.
/// WHY: slots are never removed or reused, so checkpoints, static-if ranges and pending tickets
///      keep one meaning for the whole body. An inactive continuation can only mark its own
///      tombstone and therefore cannot publish into a later call's slot.
// Concrete states keep the owned request inline: boxing would add one allocation per request.
// Pending states and `Dropped` carry no payload.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub(crate) enum GenericRequestSlot {
    Pending,
    AssertionValidationPending,
    Concrete(GenericFunctionInstantiationRequest),
    AssertionValidation(GenericFunctionInstantiationRequest),
    Dropped,
}

/// Stable index of a pending slot in the ordered generic-request ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GenericRequestTicket(usize);

impl GenericRequestTicket {
    pub(crate) fn new(index: usize) -> Self {
        Self(index)
    }

    pub(crate) fn index(self) -> usize {
        self.0
    }
}
