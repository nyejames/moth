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
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork, PathTable};
use crate::compiler_frontend::symbols::string_interning::{
    FrozenStringTable, StringTable, StringTableResolver,
};
use crate::compiler_frontend::tokenizer::tokens::FileTokens;
use crate::compiler_frontend::tokenizer::tokens::SourceTokens;
use crate::compiler_frontend::tokenizer::tokens::TokenCursor;
use crate::compiler_frontend::tokenizer::tokens::TokenRange;
use crate::compiler_frontend::tokenizer::tokens::TokenSequenceId;
use crate::compiler_frontend::tokenizer::tokens::TokenSequenceView;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

/// One generic function body backed by one canonical source owner.
///
/// WHAT: retains the canonical `SourceTokens` owner, a checked contiguous range or segmented
/// sequence, and the declaration/donor identities later parsers need.
/// WHY: generic syntax can outlive the declaring AST pass, but it must never retain a second
/// token representation. Parser consumers derive bounded adapters on demand, and a materialised
/// body carries the identity tables that issued its retained payload IDs so the requester
/// boundary rebases by spelling instead of trusting numeric identity.
#[derive(Clone)]
pub(crate) enum GenericFunctionBody {
    /// Source templates use the declaring module's canonical source owner.
    Source {
        source_owner: Arc<SourceTokens>,
        token_range: TokenRange,
        token_sequence: Option<TokenSequenceId>,
        declaration_path: PathId,
        canonical_os_path: Option<PathBuf>,
    },
    /// Materialised bodies share the donor's canonical owner plus its issuing identity tables.
    ///
    /// The retained pair names the tables that issued the payload IDs, not a proven domain
    /// difference: capture always stamps a frozen string owner, so every body produced by
    /// materialisation carries one and rebases by spelling at the requester boundary.
    /// `source_path_table` is additionally `Some` when path roots must be rebased too, and a
    /// path table without its issuing string table is rejected at construction. The rebased
    /// parser adapter is derived per parse and never retained.
    Materialised {
        source_owner: Arc<SourceTokens>,
        canonical_os_path: Option<PathBuf>,
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

/// Borrowed identity data used while rebuilding a remapped parser adapter.
///
/// The destination tables remain borrowed mutably for the adapter's lifetime, matching the
/// original helper's path/string remap behavior without retaining either table.
pub(crate) struct RemappedAdapterContext<'a, S: StringTableResolver> {
    pub(crate) source_path_table: Option<&'a PathTable>,
    pub(crate) source_strings: &'a S,
    pub(crate) destination_strings: &'a mut StringTable,
    pub(crate) path_fork: Option<&'a mut PathInternerFork>,
}

impl GenericFunctionBody {
    pub(crate) fn source(
        source_owner: Arc<SourceTokens>,
        token_range: TokenRange,
        token_sequence: Option<TokenSequenceId>,
        declaration_path: PathId,
        canonical_os_path: Option<PathBuf>,
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
            canonical_os_path,
        })
    }

    pub(crate) fn materialised(
        source_owner: Arc<SourceTokens>,
        canonical_os_path: Option<PathBuf>,
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
            canonical_os_path,
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

    /// Filesystem identity of the canonical owner, which `SourceTokens` itself does not store.
    pub(crate) fn canonical_os_path(&self) -> Option<&PathBuf> {
        match self {
            Self::Source {
                canonical_os_path, ..
            }
            | Self::Materialised {
                canonical_os_path, ..
            } => canonical_os_path.as_ref(),
        }
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

    /// Build the parser cursor for this body.
    ///
    /// Source bodies borrow the canonical owner directly. A materialised body with a retained
    /// string owner interprets its payload IDs through that table, so spelling-based rebasing
    /// must run first; that lane derives a transient compatibility adapter which is never
    /// retained. Capture always retains a string owner, so every materialised body reaching a
    /// parser takes the rebasing lane.
    pub(crate) fn parser_cursor<'a>(
        &'a self,
        string_table: &mut StringTable,
        path_fork: &mut PathInternerFork,
    ) -> Result<(AstCursor<'a>, SourceId), crate::compiler_frontend::compiler_errors::CompilerError>
    {
        if let Self::Materialised {
            source_string_table: Some(_),
            ..
        } = self
        {
            let source_id = self.donor_source_id();
            let token_stream = self.parser_stream(string_table, path_fork)?;
            return Ok((
                AstCursor::from_owned_file_tokens_compatibility(token_stream),
                source_id,
            ));
        }
        let (source_owner, token_range, token_sequence) = self.canonical_view();
        let canonical_os_path = self.canonical_os_path().cloned();
        let cursor = match token_sequence {
            Some(sequence) => {
                AstCursor::from_source_sequence(source_owner, canonical_os_path, sequence)?
            }
            None => AstCursor::from_source_tokens(source_owner, canonical_os_path, token_range)?,
        };
        Ok((cursor, source_owner.source()))
    }

    /// Derive the bounded parser adapter for this body.
    ///
    /// Every lane starts from the same canonical bounded adapter. A body with retained donor
    /// strings then rebases payload spellings into the requester's string domain, and a retained
    /// donor path table additionally rebases its path rows through the destination fork. The
    /// returned adapter is intentionally short-lived and never retained.
    pub(crate) fn parser_stream(
        &self,
        string_table: &mut crate::compiler_frontend::symbols::string_interning::StringTable,
        path_fork: &mut crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    ) -> Result<FileTokens, crate::compiler_frontend::compiler_errors::CompilerError> {
        let unrebased = self.canonical_adapter()?;
        let Self::Materialised {
            token_range,
            token_sequence,
            declaration_path,
            source_path_table,
            source_string_table,
            ..
        } = self
        else {
            return Ok(unrebased);
        };
        let Some(source_strings) = source_string_table.as_deref() else {
            return Ok(unrebased);
        };
        remapped_bounded_adapter(
            unrebased,
            *token_range,
            *token_sequence,
            *declaration_path,
            RemappedAdapterContext {
                source_path_table: source_path_table.as_deref(),
                source_strings,
                destination_strings: string_table,
                path_fork: Some(path_fork),
            },
        )
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

    /// Derive the bounded canonical adapter for this body without rebasing any payload.
    ///
    /// Capture only validates the retained owner and scans its payload handles, so payload
    /// rebasing is deferred to [`parser_stream`](Self::parser_stream), the requester boundary.
    pub(crate) fn canonical_adapter(
        &self,
    ) -> Result<FileTokens, crate::compiler_frontend::compiler_errors::CompilerError> {
        let (source_owner, token_range, token_sequence) = self.canonical_view();
        bounded_adapter_from_canonical(
            source_owner,
            self.canonical_os_path().cloned(),
            token_range,
            token_sequence,
            self.declaration_path(),
        )
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

pub(crate) fn bounded_adapter_from_canonical(
    source_owner: &Arc<SourceTokens>,
    canonical_os_path: Option<PathBuf>,
    token_range: TokenRange,
    token_sequence: Option<TokenSequenceId>,
    declaration_path: PathId,
) -> Result<FileTokens, crate::compiler_frontend::compiler_errors::CompilerError> {
    if let Some(sequence) = token_sequence {
        FileTokens::new_bounded_sequence_substream_from_canonical(
            Arc::clone(source_owner),
            canonical_os_path,
            sequence,
            declaration_path,
        )
    } else {
        FileTokens::new_bounded_substream_from_canonical(
            Arc::clone(source_owner),
            canonical_os_path,
            token_range,
            declaration_path,
        )
    }
}

/// Rebase one already-bounded canonical adapter's payload IDs into the requester's domain.
///
/// The adapter arrives bounded to `token_range`/`token_sequence`, so its compatibility tokens are
/// remapped in place: the rebase costs no second token vector. Canonical spans and tags stay
/// owned by the retained `SourceTokens`, which `new_remapped_bounded_adapter` re-validates.
pub(crate) fn remapped_bounded_adapter<S: StringTableResolver>(
    mut unrebased: FileTokens,
    token_range: TokenRange,
    token_sequence: Option<TokenSequenceId>,
    declaration_path: PathId,
    rebase: RemappedAdapterContext<'_, S>,
) -> Result<FileTokens, crate::compiler_frontend::compiler_errors::CompilerError> {
    let RemappedAdapterContext {
        source_path_table,
        source_strings,
        destination_strings,
        path_fork,
    } = rebase;
    let mut destination_path_syntax = unrebased.path_syntax_table()?.clone();
    if let Some(source_path_table) = source_path_table {
        let path_fork = path_fork.ok_or_else(|| {
            crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                "generic donor path rebasing requires a destination path fork",
            )
        })?;
        let path_remap = path_fork
            .remap_table_from_strings(source_path_table, source_strings, destination_strings)
            .ok_or_else(|| {
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                    "generic donor path table could not be rebased into the destination domain",
                )
            })?;
        destination_path_syntax.remap_path_ids(&path_remap);
    }
    let mut tokens = std::mem::take(&mut unrebased.tokens);
    for token in &mut tokens {
        token.try_remap_string_ids(&mut |id| {
            Ok::<_, crate::compiler_frontend::compiler_errors::CompilerError>(
                destination_strings.intern(source_strings.resolve(id)),
            )
        })?;
    }
    FileTokens::new_remapped_bounded_adapter(
        &unrebased,
        token_range,
        token_sequence,
        declaration_path,
        tokens,
        destination_path_syntax,
    )
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
