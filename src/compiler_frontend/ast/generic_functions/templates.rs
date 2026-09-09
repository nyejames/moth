//! Generic function template records.
//!
//! WHAT: stores the original generic function body plus its resolved signature.
//! WHY: concrete instance emission reparses the body under inferred type substitutions while
//! keeping the original source locations for diagnostics.

use crate::compiler_frontend::ast::module_ast::scope_context::Stage0ResolutionFacts;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::canonical_type_identity::GenericDeclarationOrigin;
use crate::compiler_frontend::datatypes::ids::GenericParameterListId;
use crate::compiler_frontend::semantic_identity::GeneratedDeclarationIdentity;
use crate::compiler_frontend::source::FrozenIdentityHandle;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation};
use std::fmt;
use std::sync::Arc;

/// One generic function body and the facts for every path handle in that body.
///
/// WHAT: keeps generated body tokens paired with the Stage 0 rows compacted for that exact
///       token table, plus the concrete owning `SourceId` carried by both.
/// WHY: persistent generic path handles restart per body, so a generated parser must never look
///      them up in another body's facts or in the declaring module's table. Materialised bodies
///      retain their donor identity (`FileTokens::file_id` equals the frozen facts owner);
///      donor ranges stay distinct from requester call-site sources until a final
///      `FrozenIdentityContext` remap.
#[derive(Clone)]
pub(crate) enum GenericFunctionBody {
    /// Source templates still use the active module's ordinary Stage 0 services.
    Source(FileTokens),
    /// The materialised token stream and frozen Stage 0 facts retain one concrete donor owner.
    /// Emission threads that owner through `declaring_file_id` and the late-bound identity handle.
    Materialised {
        tokens: FileTokens,
        resolution_facts: Arc<Stage0ResolutionFacts>,
        frozen_identity_handle: FrozenIdentityHandle,
    },
}

impl GenericFunctionBody {
    pub(crate) fn source(tokens: FileTokens) -> Self {
        Self::Source(tokens)
    }

    pub(crate) fn materialised(
        tokens: FileTokens,
        resolution_facts: Arc<Stage0ResolutionFacts>,
        frozen_identity_handle: FrozenIdentityHandle,
    ) -> Self {
        Self::Materialised {
            tokens,
            resolution_facts,
            frozen_identity_handle,
        }
    }

    pub(crate) fn tokens(&self) -> &FileTokens {
        match self {
            Self::Source(tokens) | Self::Materialised { tokens, .. } => tokens,
        }
    }

    pub(crate) fn resolution_facts(&self) -> Option<&Arc<Stage0ResolutionFacts>> {
        match self {
            Self::Source(_) => None,
            Self::Materialised {
                resolution_facts, ..
            } => Some(resolution_facts),
        }
    }

    pub(crate) fn frozen_identity_handle(&self) -> Option<&FrozenIdentityHandle> {
        match self {
            Self::Source(_) => None,
            Self::Materialised {
                frozen_identity_handle,
                ..
            } => Some(frozen_identity_handle),
        }
    }
}

impl fmt::Debug for GenericFunctionBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(tokens) => formatter.debug_tuple("Source").field(tokens).finish(),
            Self::Materialised { tokens, .. } => formatter
                .debug_struct("Materialised")
                .field("tokens", tokens)
                .finish_non_exhaustive(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct GenericFunctionTemplate {
    pub(crate) function_path: InternedPath,
    pub(crate) source_file: InternedPath,
    /// Imported contracts already know their stable declaration origin. Local templates receive
    /// it from the public/private identity join after AST construction.
    pub(crate) declaration_identity: Option<GeneratedDeclarationIdentity>,
    /// Stable owner of the generic parameter list when the template is public.
    ///
    /// Generic free functions use their own function origin. Generic receiver methods use the
    /// enclosing nominal type origin: receiver methods travel with that nominal surface and do
    /// not become independent generic declaration owners. Private templates keep this absent;
    /// their module-local `GenericParameterListId` remains the artefact-local substitution owner.
    pub(crate) generic_parameter_owner: Option<GenericDeclarationOrigin>,
    pub(crate) generic_parameter_list_id: GenericParameterListId,
    pub(crate) signature: FunctionSignature,
    /// Only the declaring module retains body syntax. Source templates use the active module's
    /// ordinary facts; generated templates use the `Materialised` variant, which owns the
    /// compact facts for its tokens.
    pub(crate) body_tokens: Option<GenericFunctionBody>,
    pub(crate) declaration_location: SourceLocation,
}
