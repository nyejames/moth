//! Generic function instance identity.
//!
//! WHAT: defines the canonical key and record shape for concrete generic free-function
//! instances.
//! WHY: call inference and emission deduplicate instances by source function path and canonical
//! `TypeId` arguments, not by rendered names or local import aliases.

use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::semantic_identity::GeneratedDeclarationIdentity;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct GenericFunctionInstanceKey {
    pub(crate) function_path: InternedPath,
    pub(crate) type_arguments: Box<[TypeId]>,
}

#[derive(Debug, Clone)]
pub(crate) struct GenericFunctionInstance {
    pub(crate) instance_path: InternedPath,
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
    /// public origin or an artefact-scoped private identity before worklist canonicalisation.
    pub(crate) declaration_identity: Option<GeneratedDeclarationIdentity>,
    /// Ordered local evidence selections, canonicalized when the stable request is installed.
    pub(crate) evidence: Box<[crate::compiler_frontend::traits::ids::TraitEvidenceId]>,
    pub(crate) key: GenericFunctionInstanceKey,
    pub(crate) instance_path: InternedPath,
    pub(crate) call_location: SourceLocation,
}
