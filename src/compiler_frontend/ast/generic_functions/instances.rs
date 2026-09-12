//! Generic function instance identity.
//!
//! WHAT: defines the canonical key and record shape for concrete generic function and receiver
//! instances.
//! WHY: call inference and emission deduplicate instances by source function path and canonical
//! `TypeId` arguments, not by rendered names or local dependency aliases.

use crate::compiler_frontend::compiler_errors::CompilerError;
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

/// Request emitted by call parsing and consumed by AST emission.
///
/// WHAT: records that one concrete generic function instance must be materialized as a
/// normal AST function before HIR lowering.
/// WHY: expression parsing can infer the concrete call target, but only the emitter owns the
/// module-level AST node list where the specialized function body belongs.
#[derive(Debug, Clone)]
pub(crate) struct GenericFunctionInstantiationRequest {
    /// Imported public contracts already carry their origin. Local requests receive either a
    /// public origin or an artefact-scoped private identity before stable-request
    /// canonicalisation.
    pub(crate) declaration_identity: Option<GeneratedDeclarationIdentity>,
    /// Ordered local evidence selections, canonicalized when the stable request is installed.
    pub(crate) evidence: Box<[crate::compiler_frontend::traits::ids::TraitEvidenceId]>,
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
    ) -> Result<Self, CompilerError> {
        let instance_component = string_table.intern("__generated_instance");
        let instance_path = path_fork
            .try_intern_child(function_path, instance_component)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "path table exhausted while creating generated generic function instance",
                )
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
