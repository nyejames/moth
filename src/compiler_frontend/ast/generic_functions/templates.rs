//! Generic function template records.
//!
//! WHAT: stores the original generic function body plus its resolved signature.
//! WHY: concrete instance emission reparses the body under inferred type substitutions while
//! keeping the original source locations for diagnostics.

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::module_ast::scope_context::Stage0ResolutionFacts;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::canonical_type_identity::GenericDeclarationOrigin;
use crate::compiler_frontend::datatypes::ids::GenericParameterListId;
use crate::compiler_frontend::semantic_identity::GeneratedDeclarationIdentity;
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceId};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathTable};
use crate::compiler_frontend::symbols::string_interning::FrozenStringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceTokens;
use crate::compiler_frontend::tokenizer::tokens::TokenCursor;
use crate::compiler_frontend::tokenizer::tokens::TokenPayloadOrigin;
use crate::compiler_frontend::tokenizer::tokens::TokenRange;
use crate::compiler_frontend::tokenizer::tokens::TokenSequenceId;
use crate::compiler_frontend::tokenizer::tokens::TokenSequenceView;
use std::fmt;
use std::sync::Arc;

/// One generic function body backed by one canonical source owner.
///
/// WHAT: retains the canonical `SourceTokens` owner, a checked contiguous range or segmented
/// sequence, and the declaration/donor identities later parsers need.
/// WHY: generic syntax can outlive the declaring AST pass, but it must never retain a second
/// token representation. Every parse borrows the retained owner in place and carries the donor
/// identity tables separately, so payloads translate by spelling at consumption.
#[derive(Clone)]
pub(crate) enum GenericFunctionBody {
    /// Source templates use the declaring module's canonical source owner.
    Source {
        source_owner: Arc<SourceTokens>,
        token_range: TokenRange,
        token_sequence: Option<TokenSequenceId>,
        declaration_path: PathId,
    },
    /// Materialised bodies share the donor's canonical owner plus its issuing identity tables.
    ///
    /// The retained pair names the tables that issued the payload IDs. Every parse borrows the
    /// donor range and carries these tables as `TokenPayloadOrigin`; typed payload readers
    /// translate only values consumed by the requester.
    /// A path table without its issuing string table is rejected at construction.
    Materialised {
        source_owner: Arc<SourceTokens>,
        token_range: TokenRange,
        token_sequence: Option<TokenSequenceId>,
        declaration_path: PathId,
        resolution_facts: Arc<Stage0ResolutionFacts>,
        frozen_identity_handle: FrozenIdentityHandle,
        source_path_table: Option<Arc<PathTable>>,
        source_string_table: Option<Arc<FrozenStringTable>>,
    },
}

/// Frozen Stage 0 and identity data retained by a materialised generic body.
///
/// The body owner and checked token bounds remain direct constructor inputs; these donor facts
/// travel together so materialisation callers cannot accidentally mix identity domains.
pub(crate) struct MaterialisedDonorContext {
    pub(crate) resolution_facts: Arc<Stage0ResolutionFacts>,
    pub(crate) frozen_identity_handle: FrozenIdentityHandle,
    pub(crate) source_path_table: Option<Arc<PathTable>>,
    pub(crate) source_string_table: Option<Arc<FrozenStringTable>>,
}

impl MaterialisedDonorContext {
    /// A donor path table is interpretable only through the string table that issued its roots.
    ///
    /// This is the single owner of that rule: every other donor-pair consumer reads a pair that
    /// already passed through here.
    fn validate_identity_pair(
        &self,
    ) -> Result<(), crate::compiler_frontend::compiler_errors::CompilerError> {
        if self.source_path_table.is_some() && self.source_string_table.is_none() {
            return Err(
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                    "materialised generic body has an incomplete source identity table pair",
                ),
            );
        }
        Ok(())
    }
}

/// The canonical owner one generic body parses from.
///
/// WHAT: borrows the body's retained owner with its checked bounds plus the optional donor
/// identity that issued its payload IDs.
/// WHY: every parse uses the retained owner in place; requester payloads translate by spelling
/// at consumption through the cursor's typed readers instead of a copied token window.
pub(crate) struct BodyParseOwner<'a> {
    pub(crate) owner: &'a Arc<SourceTokens>,
    pub(crate) token_range: TokenRange,
    pub(crate) token_sequence: Option<TokenSequenceId>,
    pub(crate) payload_origin: Option<TokenPayloadOrigin<'a>>,
}

impl BodyParseOwner<'_> {
    /// Borrow this body's parser cursor and the source identity its spans belong to.
    pub(crate) fn cursor(
        &self,
    ) -> Result<(AstCursor<'_>, SourceId), crate::compiler_frontend::compiler_errors::CompilerError>
    {
        let cursor = match self.token_sequence {
            Some(sequence) => AstCursor::from_source_sequence(self.owner, sequence)?,
            None => AstCursor::from_source_tokens(self.owner, self.token_range)?,
        };
        let cursor = match self.payload_origin {
            Some(origin) => cursor.with_payload_origin(origin),
            None => cursor,
        };
        Ok((cursor, self.owner.source()))
    }
}

impl GenericFunctionBody {
    pub(crate) fn source(
        source_owner: Arc<SourceTokens>,
        token_range: TokenRange,
        token_sequence: Option<TokenSequenceId>,
        declaration_path: PathId,
    ) -> Result<Self, crate::compiler_frontend::compiler_errors::CompilerError> {
        validate_source_owner(
            &source_owner,
            token_range,
            token_sequence,
            "source generic body",
        )?;
        Ok(Self::Source {
            source_owner,
            token_range,
            token_sequence,
            declaration_path,
        })
    }

    pub(crate) fn materialised(
        source_owner: Arc<SourceTokens>,
        token_range: TokenRange,
        token_sequence: Option<TokenSequenceId>,
        declaration_path: PathId,
        donor_context: MaterialisedDonorContext,
    ) -> Result<Self, crate::compiler_frontend::compiler_errors::CompilerError> {
        donor_context.validate_identity_pair()?;
        validate_source_owner(
            &source_owner,
            token_range,
            token_sequence,
            "materialised generic body",
        )?;
        Ok(Self::Materialised {
            source_owner,
            token_range,
            token_sequence,
            declaration_path,
            resolution_facts: donor_context.resolution_facts,
            frozen_identity_handle: donor_context.frozen_identity_handle,
            source_path_table: donor_context.source_path_table,
            source_string_table: donor_context.source_string_table,
        })
    }

    /// Canonical source identity that owns every retained token of this body.
    pub(crate) fn donor_source_id(&self) -> crate::compiler_frontend::source::SourceId {
        match self {
            Self::Source { source_owner, .. } | Self::Materialised { source_owner, .. } => {
                source_owner.source()
            }
        }
    }

    /// Canonical owner and checked bounds of the retained body.
    pub(crate) fn canonical_view(
        &self,
    ) -> (&Arc<SourceTokens>, TokenRange, Option<TokenSequenceId>) {
        match self {
            Self::Source {
                source_owner,
                token_range,
                token_sequence,
                ..
            }
            | Self::Materialised {
                source_owner,
                token_range,
                token_sequence,
                ..
            } => (source_owner, *token_range, *token_sequence),
        }
    }
    /// Choose the canonical owner this body parses from.
    ///
    /// Source bodies and materialised bodies both borrow their retained canonical owner and
    /// checked range/sequence. Materialised bodies additionally carry the frozen donor identity
    /// through `TokenPayloadOrigin`; payloads are translated only when a parser consumes them.
    pub(crate) fn parse_owner(
        &self,
    ) -> Result<BodyParseOwner<'_>, crate::compiler_frontend::compiler_errors::CompilerError> {
        let (source_owner, token_range, token_sequence) = self.canonical_view();
        let payload_origin = match self {
            Self::Materialised {
                source_string_table: Some(source_strings),
                ..
            } => Some(TokenPayloadOrigin {
                strings: source_strings.as_ref(),
            }),
            Self::Source { .. } | Self::Materialised { .. } => None,
        };
        Ok(BodyParseOwner {
            owner: source_owner,
            token_range,
            token_sequence,
            payload_origin,
        })
    }

    pub(crate) fn token_range(&self) -> TokenRange {
        match self {
            Self::Source { token_range, .. } | Self::Materialised { token_range, .. } => {
                *token_range
            }
        }
    }
    pub(crate) fn token_sequence(&self) -> Option<TokenSequenceId> {
        match self {
            Self::Source { token_sequence, .. } | Self::Materialised { token_sequence, .. } => {
                *token_sequence
            }
        }
    }

    pub(crate) fn declaration_path(&self) -> PathId {
        match self {
            Self::Source {
                declaration_path, ..
            }
            | Self::Materialised {
                declaration_path, ..
            } => *declaration_path,
        }
    }

    pub(crate) fn resolution_facts(&self) -> Option<&Arc<Stage0ResolutionFacts>> {
        match self {
            Self::Source { .. } => None,
            Self::Materialised {
                resolution_facts, ..
            } => Some(resolution_facts),
        }
    }

    pub(crate) fn frozen_identity_handle(&self) -> Option<&FrozenIdentityHandle> {
        match self {
            Self::Source { .. } => None,
            Self::Materialised {
                frozen_identity_handle,
                ..
            } => Some(frozen_identity_handle),
        }
    }

    pub(crate) fn source_identity_tables(
        &self,
    ) -> Option<(
        &Arc<crate::compiler_frontend::symbols::path_interner::PathTable>,
        &Arc<crate::compiler_frontend::symbols::string_interning::FrozenStringTable>,
    )> {
        match self {
            Self::Materialised {
                source_path_table: Some(path_table),
                source_string_table: Some(string_table),
                ..
            } => Some((path_table, string_table)),
            Self::Source { .. } | Self::Materialised { .. } => None,
        }
    }

    /// Resolve the exact frozen string domain needed while capturing this body.
    ///
    /// Source bodies use the shared declaring owner; a materialised body uses its retained donor
    /// strings when it has them. The constructor already rejected a retained path table without
    /// its issuing string owner, so no incomplete pair reaches here.
    pub(crate) fn capture_string_table<'a>(
        &'a self,
        donor_strings: &'a FrozenStringTable,
    ) -> &'a FrozenStringTable {
        match self {
            Self::Source { .. } => donor_strings,
            Self::Materialised {
                source_string_table,
                ..
            } => source_string_table.as_deref().unwrap_or(donor_strings),
        }
    }
}

pub(crate) fn validate_source_owner(
    source_owner: &SourceTokens,
    token_range: TokenRange,
    token_sequence: Option<TokenSequenceId>,
    role: &str,
) -> Result<(), crate::compiler_frontend::compiler_errors::CompilerError> {
    if token_range.source() != source_owner.source() {
        return Err(
            crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(format!(
                "{role} token range has a foreign source identity"
            )),
        );
    }
    source_owner.cursor(token_range).map_err(|error| {
        crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(format!(
            "{role} token range is outside its canonical source owner: {error:?}"
        ))
    })?;
    if let Some(sequence) = token_sequence {
        source_owner.token_sequence(sequence).map_err(|error| {
            crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(format!(
                "{role} token sequence is outside its canonical source owner: {error:?}"
            ))
        })?;
    }
    Ok(())
}

/// Walk one retained body's canonical tokens in place.
///
/// Range-only bodies walk their checked range; segmented bodies walk their registered sequence.
/// Callers break on the first `Eof` token: the cursor deliberately stalls there rather than
/// advancing past the end of its window.
pub(crate) fn body_cursor<'tokens>(
    source_owner: &'tokens SourceTokens,
    token_range: TokenRange,
    token_sequence: Option<TokenSequenceId>,
    role: &str,
) -> Result<TokenCursor<'tokens>, crate::compiler_frontend::compiler_errors::CompilerError> {
    match token_sequence {
        Some(sequence) => source_owner
            .token_sequence(sequence)
            .and_then(TokenSequenceView::cursor)
            .map_err(|error| {
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(format!(
                    "{role} token sequence is outside its canonical source owner: {error:?}"
                ))
            }),
        None => source_owner.cursor(token_range).map_err(|error| {
            crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(format!(
                "{role} token range is outside its canonical source owner: {error:?}"
            ))
        }),
    }
}
impl fmt::Debug for GenericFunctionBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct(match self {
            Self::Source { .. } => "Source",
            Self::Materialised { .. } => "Materialised",
        });
        debug
            .field("source", &self.donor_source_id())
            .field("range", &self.token_range())
            .field("sequence", &self.token_sequence())
            .field("declaration_path", &self.declaration_path());
        debug.finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct GenericFunctionTemplate {
    pub(crate) function_path: PathId,
    pub(crate) source_file: PathId,
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
    /// ordinary facts; generated templates use the `Materialised` variant, which owns retained
    /// donor file-reference facts for its canonical source range.
    pub(crate) body_tokens: Option<GenericFunctionBody>,
    pub(crate) declaration_span: Option<crate::compiler_frontend::source::SourceSpan>,
}
