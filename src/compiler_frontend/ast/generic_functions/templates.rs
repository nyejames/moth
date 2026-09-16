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
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork, PathTable};
use crate::compiler_frontend::symbols::string_interning::{
    FrozenStringTable, StringTable, StringTableResolver,
};
use crate::compiler_frontend::tokenizer::tokens::FileTokens;
use crate::compiler_frontend::tokenizer::tokens::SourceTokens;
use crate::compiler_frontend::tokenizer::tokens::TokenRange;
use crate::compiler_frontend::tokenizer::tokens::TokenSequenceId;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

/// One generic function body backed by one canonical source owner.
///
/// WHAT: retains only the canonical source owner, a checked contiguous range or segmented
/// sequence and the declaration/donor identities needed by later parsers.
/// WHY: generic syntax can outlive the declaring AST pass, but it must never retain another token
/// vector or a second `SourceTokens` store. Parser consumers derive a bounded adapter on demand.
/// Source bodies share the declaring module's canonical `SourceTokens` allocation directly.
/// Materialised bodies keep the donor `FileTokens` shell (including its remapped compatibility
/// lane) until H5 deletes the remaining adapters.
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
    /// Materialised bodies retain the donor owner and frozen Stage 0/identity context.
    Materialised {
        source_owner: Arc<FileTokens>,
        token_range: TokenRange,
        token_sequence: Option<TokenSequenceId>,
        declaration_path: PathId,
        resolution_facts: Arc<Stage0ResolutionFacts>,
        frozen_identity_handle: FrozenIdentityHandle,
        /// Provider identity tables are present when the donor crossed a package boundary.
        ///
        /// Same-boundary materialisation uses the active path/string domain directly. A published
        /// foreign context supplies these shared tables so transient parser adapters can rebase
        /// donor payloads without mutating the canonical owner.
        source_path_table: Option<Arc<crate::compiler_frontend::symbols::path_interner::PathTable>>,
        source_string_table:
            Option<Arc<crate::compiler_frontend::symbols::string_interning::FrozenStringTable>>,
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
        source_owner: Arc<FileTokens>,
        token_range: TokenRange,
        token_sequence: Option<TokenSequenceId>,
        declaration_path: PathId,
        donor_context: MaterialisedDonorContext,
    ) -> Result<Self, crate::compiler_frontend::compiler_errors::CompilerError> {
        validate_syntax_owner(
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
            Self::Source { source_owner, .. } => source_owner.source(),
            Self::Materialised { source_owner, .. } => source_owner.file_id,
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

    #[cfg(test)]
    pub(crate) fn materialised_owner(&self) -> Option<&Arc<FileTokens>> {
        match self {
            Self::Source { .. } => None,
            Self::Materialised { source_owner, .. } => Some(source_owner),
        }
    }

    /// Derive the bounded parser adapter for this body.
    ///
    /// Source bodies share the canonical owner directly and never rebase payloads or mutate the
    /// path fork. Materialised bodies additionally rebase donor payloads through their retained
    /// provider identity pair. The returned adapter is intentionally short-lived and is never
    /// retained.
    pub(crate) fn parser_stream(
        &self,
        string_table: &mut crate::compiler_frontend::symbols::string_interning::StringTable,
        path_fork: &mut crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    ) -> Result<FileTokens, crate::compiler_frontend::compiler_errors::CompilerError> {
        match self {
            Self::Source {
                source_owner,
                token_range,
                token_sequence,
                declaration_path,
                canonical_os_path,
            } => bounded_adapter_from_canonical(
                source_owner,
                canonical_os_path.clone(),
                *token_range,
                *token_sequence,
                *declaration_path,
            ),
            Self::Materialised {
                source_owner,
                token_range,
                token_sequence,
                declaration_path,
                source_path_table,
                source_string_table,
                ..
            } => match source_string_table.as_deref() {
                Some(source_strings) => remapped_bounded_adapter(
                    source_owner,
                    *token_range,
                    *token_sequence,
                    *declaration_path,
                    RemappedAdapterContext {
                        source_path_table: source_path_table.as_deref(),
                        source_strings,
                        destination_strings: string_table,
                        path_fork: Some(path_fork),
                    },
                ),
                None => bounded_adapter(
                    source_owner,
                    *token_range,
                    *token_sequence,
                    *declaration_path,
                ),
            },
        }
    }

    /// Whether [`parser_stream`](Self::parser_stream) returns a remapped compatibility vector.
    ///
    /// WHAT: true when a donor string table is retained (donor path rebasing optional).
    /// WHY: remapped adapters keep donor canonical provenance whose spans/tags match but whose
    /// payload IDs are stale; consumers must force the compatibility cursor lane to preserve
    /// the rebased token/path payloads.
    pub(crate) fn uses_remapped_adapter(&self) -> bool {
        matches!(
            self,
            Self::Materialised {
                source_string_table: Some(_),
                ..
            }
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
            Self::Source { .. } => None,
            Self::Materialised {
                source_path_table: Some(path_table),
                source_string_table: Some(string_table),
                ..
            } => Some((path_table, string_table)),
            Self::Materialised { .. } => None,
        }
    }
    /// Resolve the exact frozen string domain needed while capturing this body.
    ///
    /// Source and same-domain bodies use the shared declaring owner. Materialised bodies retain
    /// their foreign string owner when present; a path owner without its issuing string owner is
    /// invalid and must not silently fall back to the current domain.
    pub(crate) fn capture_string_table<'a>(
        &'a self,
        donor_strings: &'a FrozenStringTable,
    ) -> Result<&'a FrozenStringTable, crate::compiler_frontend::compiler_errors::CompilerError>
    {
        match self {
            Self::Source { .. } => Ok(donor_strings),
            Self::Materialised {
                source_path_table: Some(_),
                source_string_table: None,
                ..
            } => Err(
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                    "materialised generic body has an incomplete source identity table pair",
                ),
            ),
            Self::Materialised {
                source_string_table: Some(source_strings),
                ..
            } => Ok(source_strings),
            Self::Materialised {
                source_path_table: None,
                source_string_table: None,
                ..
            } => Ok(donor_strings),
        }
    }

    /// Derive a bounded canonical adapter for stable capture without mutating identity tables.
    ///
    /// Capture only validates the retained owner and scans path references. Payload rebasing is
    /// deferred to [`parser_stream`](Self::parser_stream), the requester parser boundary.
    pub(crate) fn parser_stream_for_capture(
        &self,
    ) -> Result<FileTokens, crate::compiler_frontend::compiler_errors::CompilerError> {
        match self {
            Self::Source {
                source_owner,
                token_range,
                token_sequence,
                declaration_path,
                canonical_os_path,
            } => bounded_adapter_from_canonical(
                source_owner,
                canonical_os_path.clone(),
                *token_range,
                *token_sequence,
                *declaration_path,
            ),
            Self::Materialised {
                source_owner,
                token_range,
                token_sequence,
                declaration_path,
                ..
            } => bounded_adapter(
                source_owner,
                *token_range,
                *token_sequence,
                *declaration_path,
            ),
        }
    }
}

pub(crate) fn validate_syntax_owner(
    source_owner: &FileTokens,
    token_range: crate::compiler_frontend::tokenizer::tokens::TokenRange,
    token_sequence: Option<crate::compiler_frontend::tokenizer::tokens::TokenSequenceId>,
    role: &str,
) -> Result<(), crate::compiler_frontend::compiler_errors::CompilerError> {
    if token_range.source() != source_owner.file_id {
        return Err(
            crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(format!(
                "{role} token range has a foreign source identity"
            )),
        );
    }
    let canonical = source_owner.canonical_source_tokens().map_err(|_| {
        crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(format!(
            "{role} token range is outside its canonical source owner: parser adapter stream owns no canonical source-token store"
        ))
    })?;
    canonical.cursor(token_range).map_err(|error| {
        crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(format!(
            "{role} token range is outside its canonical source owner: {error:?}"
        ))
    })?;
    if let Some(sequence) = token_sequence {
        canonical.token_sequence(sequence).map_err(|error| {
            crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(format!(
                "{role} token sequence is outside its canonical source owner: {error:?}"
            ))
        })?;
    }
    Ok(())
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

pub(crate) fn bounded_adapter(
    source_owner: &FileTokens,
    token_range: TokenRange,
    token_sequence: Option<TokenSequenceId>,
    declaration_path: PathId,
) -> Result<FileTokens, crate::compiler_frontend::compiler_errors::CompilerError> {
    if let Some(sequence) = token_sequence {
        FileTokens::new_bounded_sequence_substream(source_owner, sequence, declaration_path)
    } else {
        FileTokens::new_bounded_substream(source_owner, token_range, declaration_path)
    }
}

pub(crate) fn remapped_bounded_adapter<S: StringTableResolver>(
    source_owner: &FileTokens,
    token_range: crate::compiler_frontend::tokenizer::tokens::TokenRange,
    token_sequence: Option<crate::compiler_frontend::tokenizer::tokens::TokenSequenceId>,
    declaration_path: PathId,
    rebase: RemappedAdapterContext<'_, S>,
) -> Result<FileTokens, crate::compiler_frontend::compiler_errors::CompilerError> {
    let RemappedAdapterContext {
        source_path_table,
        source_strings,
        destination_strings,
        path_fork,
    } = rebase;
    let mut adapter = bounded_adapter(source_owner, token_range, token_sequence, declaration_path)?;
    let mut path_remap = None;
    if let Some(source_path_table) = source_path_table {
        let path_fork = path_fork.ok_or_else(|| {
            crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                "generic donor path rebasing requires a destination path fork",
            )
        })?;
        path_remap = Some(
            path_fork
                .remap_table_from_strings(source_path_table, source_strings, destination_strings)
                .ok_or_else(|| {
                    crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                        "generic donor path table could not be rebased into the destination domain",
                    )
                })?,
        );
    }
    let mut destination_path_syntax = adapter.path_syntax_table()?.clone();
    if let Some(path_remap) = path_remap.as_ref() {
        destination_path_syntax.remap_path_ids(path_remap);
    }
    for token in &mut adapter.tokens {
        token.try_remap_string_ids(&mut |id| {
            Ok::<_, crate::compiler_frontend::compiler_errors::CompilerError>(
                destination_strings.intern(source_strings.resolve(id)),
            )
        })?;
    }
    FileTokens::new_remapped_bounded_adapter(
        source_owner,
        token_range,
        token_sequence,
        declaration_path,
        adapter.tokens,
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
    /// ordinary facts; generated templates use the `Materialised` variant, which owns
    /// retained donor file-reference facts for its canonical source range.
    pub(crate) body_tokens: Option<GenericFunctionBody>,
    pub(crate) declaration_span: Option<crate::compiler_frontend::source::SourceSpan>,
}
