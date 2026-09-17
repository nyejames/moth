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
use crate::compiler_frontend::tokenizer::tokens::TokenRange;
use crate::compiler_frontend::tokenizer::tokens::TokenSequenceId;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

/// One generic function body backed by one canonical source owner.
///
/// WHAT: retains only the canonical source owner, a checked contiguous range or segmented
/// sequence and the declaration/donor identities needed by later parsers.
/// WHY: generic syntax can outlive the declaring AST pass, but it must never retain an additional
/// canonical token vector or a second `SourceTokens` store. Parser consumers derive bounded
/// adapters on demand.
/// Canonical materialised bodies share the donor `SourceTokens` allocation directly with no
/// `FileTokens` shell, plus the donor string domain used only for transient parser rebasing.
/// Only path-crossing foreign materialised bodies (donor path table present) retain the donor
/// `FileTokens` shell (including its remapped compatibility lane) plus the donor path/string
/// pair. String-only foreign donors canonicalize to the `SourceTokens` owner plus donor strings.
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
    /// Same-domain materialised bodies share the donor canonical owner directly.
    ///
    /// The donor identity pair travels along so parser consumers can transiently rebase
    /// canonical payload IDs by spelling when the requester table differs. No `FileTokens`
    /// shell is retained on this lane; the remapped adapter is derived per parse.
    MaterialisedCanonical {
        source_owner: Arc<SourceTokens>,
        canonical_os_path: Option<PathBuf>,
        token_range: TokenRange,
        token_sequence: Option<TokenSequenceId>,
        declaration_path: PathId,
        resolution_facts: Arc<Stage0ResolutionFacts>,
        frozen_identity_handle: FrozenIdentityHandle,
        /// Donor path domain that issued the retained token payloads, when rebasing is needed.
        ///
        /// `Some` without the issuing string table is rejected at construction.
        source_path_table: Option<Arc<PathTable>>,
        /// Donor strings that issued the retained token payloads, when rebasing is needed.
        ///
        /// Same-boundary materialisation keeps the declaring domain's shared frozen owner.
        /// `None` keeps the direct canonical adapter without rebasing.
        source_string_table: Option<Arc<FrozenStringTable>>,
    },
    /// Path-crossing foreign materialised bodies retain the donor owner and frozen
    /// Stage 0/identity context. This lane requires a donor path table: string-only foreign
    /// donors canonicalize through `materialised` onto `MaterialisedCanonical`.
    MaterialisedForeign {
        source_owner: Arc<FileTokens>,
        token_range: TokenRange,
        token_sequence: Option<TokenSequenceId>,
        declaration_path: PathId,
        resolution_facts: Arc<Stage0ResolutionFacts>,
        frozen_identity_handle: FrozenIdentityHandle,
        /// Provider identity tables for donors that crossed a package boundary.
        ///
        /// A published foreign context supplies these shared tables so transient parser adapters
        /// can rebase donor payloads without mutating the canonical owner. The path table is
        /// always present on this lane; a path table without its issuing string table is
        /// rejected at construction.
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

    pub(crate) fn materialised_canonical(
        source_owner: Arc<SourceTokens>,
        canonical_os_path: Option<PathBuf>,
        token_range: TokenRange,
        token_sequence: Option<TokenSequenceId>,
        declaration_path: PathId,
        donor_context: MaterialisedDonorContext,
    ) -> Result<Self, crate::compiler_frontend::compiler_errors::CompilerError> {
        if donor_context.source_path_table.is_some() && donor_context.source_string_table.is_none()
        {
            return Err(
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                    "materialised generic body has an incomplete source identity table pair",
                ),
            );
        }
        validate_source_owner(
            &source_owner,
            token_range,
            token_sequence,
            "materialised generic body",
        )?;
        Ok(Self::MaterialisedCanonical {
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

    /// Build a path-crossing foreign materialised body, canonicalizing string-only donors.
    ///
    /// A `(None, Some(_))` donor carries only a donor string table, so its payloads are
    /// canonical `SourceTokens` IDs in a different string domain. Canonicalize by sharing the
    /// donor canonical owner plus the donor strings; only a donor path table keeps the
    /// `FileTokens` compatibility shell on `MaterialisedForeign`.
    pub(crate) fn materialised(
        source_owner: Arc<FileTokens>,
        token_range: TokenRange,
        token_sequence: Option<TokenSequenceId>,
        declaration_path: PathId,
        donor_context: MaterialisedDonorContext,
    ) -> Result<Self, crate::compiler_frontend::compiler_errors::CompilerError> {
        if donor_context.source_path_table.is_some() && donor_context.source_string_table.is_none()
        {
            return Err(
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                    "materialised generic body has an incomplete source identity table pair",
                ),
            );
        }
        if donor_context.source_path_table.is_none() {
            let canonical_owner = source_owner.canonical_source_tokens_arc().map_err(|_| {
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                    "materialised generic body has no canonical donor owner",
                )
            })?;
            let canonical_os_path = source_owner.canonical_os_path.clone();
            validate_source_owner(
                &canonical_owner,
                token_range,
                token_sequence,
                "materialised generic body",
            )?;
            return Ok(Self::MaterialisedCanonical {
                source_owner: canonical_owner,
                canonical_os_path,
                token_range,
                token_sequence,
                declaration_path,
                resolution_facts: donor_context.resolution_facts,
                frozen_identity_handle: donor_context.frozen_identity_handle,
                source_path_table: donor_context.source_path_table,
                source_string_table: donor_context.source_string_table,
            });
        }
        validate_syntax_owner(
            &source_owner,
            token_range,
            token_sequence,
            "materialised generic body",
        )?;
        Ok(Self::MaterialisedForeign {
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
            Self::Source { source_owner, .. }
            | Self::MaterialisedCanonical { source_owner, .. } => source_owner.source(),
            Self::MaterialisedForeign { source_owner, .. } => source_owner.file_id,
        }
    }
    pub(crate) fn token_range(&self) -> TokenRange {
        match self {
            Self::Source { token_range, .. }
            | Self::MaterialisedCanonical { token_range, .. }
            | Self::MaterialisedForeign { token_range, .. } => *token_range,
        }
    }

    pub(crate) fn token_sequence(&self) -> Option<TokenSequenceId> {
        match self {
            Self::Source { token_sequence, .. }
            | Self::MaterialisedCanonical { token_sequence, .. }
            | Self::MaterialisedForeign { token_sequence, .. } => *token_sequence,
        }
    }

    pub(crate) fn declaration_path(&self) -> PathId {
        match self {
            Self::Source {
                declaration_path, ..
            }
            | Self::MaterialisedCanonical {
                declaration_path, ..
            }
            | Self::MaterialisedForeign {
                declaration_path, ..
            } => *declaration_path,
        }
    }

    #[cfg(test)]
    pub(crate) fn materialised_owner(&self) -> Option<&Arc<FileTokens>> {
        match self {
            Self::Source { .. } | Self::MaterialisedCanonical { .. } => None,
            Self::MaterialisedForeign { source_owner, .. } => Some(source_owner),
        }
    }

    #[cfg(test)]
    pub(crate) fn materialised_canonical_owner(&self) -> Option<&Arc<SourceTokens>> {
        match self {
            Self::Source { .. } | Self::MaterialisedForeign { .. } => None,
            Self::MaterialisedCanonical { source_owner, .. } => Some(source_owner),
        }
    }
    /// Build the parser cursor for this body.
    ///
    /// Source and unrebased canonical materialised bodies borrow their canonical owner directly.
    /// A canonical body with retained donor strings and every path-crossing foreign body derive
    /// a transient remapped compatibility adapter instead: donor payloads were issued in a
    /// different string-identity domain than the requester's, so spelling-based rebasing must
    /// run before the parser reads them. The adapter is never retained.
    pub(crate) fn parser_cursor<'a>(
        &'a self,
        string_table: &mut StringTable,
        path_fork: &mut PathInternerFork,
    ) -> Result<(AstCursor<'a>, SourceId), crate::compiler_frontend::compiler_errors::CompilerError>
    {
        match self {
            Self::Source {
                source_owner,
                token_range,
                token_sequence,
                canonical_os_path,
                ..
            }
            | Self::MaterialisedCanonical {
                source_owner,
                token_range,
                token_sequence,
                canonical_os_path,
                source_string_table: None,
                ..
            } => {
                let cursor = match token_sequence {
                    Some(sequence) => AstCursor::from_source_sequence(
                        source_owner,
                        canonical_os_path.clone(),
                        *sequence,
                    )?,
                    None => AstCursor::from_source_tokens(
                        source_owner,
                        canonical_os_path.clone(),
                        *token_range,
                    )?,
                };
                Ok((cursor, source_owner.source()))
            }
            Self::MaterialisedCanonical { .. } | Self::MaterialisedForeign { .. } => {
                let source_id = self.donor_source_id();
                let token_stream = self.parser_stream(string_table, path_fork)?;
                Ok((
                    AstCursor::from_owned_file_tokens_compatibility(token_stream),
                    source_id,
                ))
            }
        }
    }

    /// Derive the bounded parser adapter for this body.
    ///
    /// Source and unrebased canonical bodies share the canonical owner directly and never
    /// rebase payloads or mutate the path fork. A canonical body with donor strings derives a
    /// transient remapped adapter from a canonical shell; path-crossing foreign bodies rebase
    /// through their retained provider pair. The returned adapter is intentionally short-lived
    /// and never retained.
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
            Self::MaterialisedCanonical {
                source_owner,
                token_range,
                token_sequence,
                declaration_path,
                canonical_os_path,
                source_path_table,
                source_string_table,
                ..
            } => {
                if source_path_table.is_some() && source_string_table.is_none() {
                    return Err(
                        crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                            "materialised generic body has an incomplete source identity table pair",
                        ),
                    );
                }
                match source_string_table.as_deref() {
                    // Without paths, canonical tokens still need string rebasing, but their range
                    // and path facts stay requester-native. Keep `path_fork` out of the operation.
                    Some(source_strings) if source_path_table.is_none() => {
                        let unrebased = bounded_adapter_from_canonical(
                            source_owner,
                            canonical_os_path.clone(),
                            *token_range,
                            *token_sequence,
                            *declaration_path,
                        )?;
                        let destination_path_syntax = unrebased.path_syntax_table()?.clone();
                        let mut tokens = unrebased.tokens.clone();
                        for token in &mut tokens {
                            token.try_remap_string_ids(&mut |id| {
                                Ok::<_, crate::compiler_frontend::compiler_errors::CompilerError>(
                                    string_table.intern(source_strings.resolve(id)),
                                )
                            })?;
                        }
                        FileTokens::new_remapped_bounded_adapter(
                            &unrebased,
                            *token_range,
                            *token_sequence,
                            *declaration_path,
                            tokens,
                            destination_path_syntax,
                        )
                    }
                    Some(source_strings) => {
                        let unrebased = bounded_adapter_from_canonical(
                            source_owner,
                            canonical_os_path.clone(),
                            *token_range,
                            *token_sequence,
                            *declaration_path,
                        )?;
                        remapped_bounded_adapter(
                            &unrebased,
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
                    None => bounded_adapter_from_canonical(
                        source_owner,
                        canonical_os_path.clone(),
                        *token_range,
                        *token_sequence,
                        *declaration_path,
                    ),
                }
            }
            Self::MaterialisedForeign {
                source_owner,
                token_range,
                token_sequence,
                declaration_path,
                source_path_table,
                source_string_table,
                ..
            } => {
                if source_path_table.is_some() && source_string_table.is_none() {
                    return Err(
                        crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                            "materialised generic body has an incomplete source identity table pair",
                        ),
                    );
                }
                match source_string_table.as_deref() {
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
                }
            }
        }
    }

    pub(crate) fn resolution_facts(&self) -> Option<&Arc<Stage0ResolutionFacts>> {
        match self {
            Self::Source { .. } => None,
            Self::MaterialisedCanonical {
                resolution_facts, ..
            }
            | Self::MaterialisedForeign {
                resolution_facts, ..
            } => Some(resolution_facts),
        }
    }

    pub(crate) fn frozen_identity_handle(&self) -> Option<&FrozenIdentityHandle> {
        match self {
            Self::Source { .. } => None,
            Self::MaterialisedCanonical {
                frozen_identity_handle,
                ..
            }
            | Self::MaterialisedForeign {
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
            Self::MaterialisedCanonical {
                source_path_table: Some(path_table),
                source_string_table: Some(string_table),
                ..
            }
            | Self::MaterialisedForeign {
                source_path_table: Some(path_table),
                source_string_table: Some(string_table),
                ..
            } => Some((path_table, string_table)),
            Self::MaterialisedCanonical { .. } | Self::MaterialisedForeign { .. } => None,
        }
    }
    /// Resolve the exact frozen string domain needed while capturing this body.
    ///
    /// Source bodies use the shared declaring owner. Canonical materialised bodies keep their
    /// retained donor strings when present; a path owner without its issuing string owner is
    /// invalid and must not silently fall back to the current domain. Path-crossing foreign
    /// materialised bodies retain their foreign string owner when present.
    pub(crate) fn capture_string_table<'a>(
        &'a self,
        donor_strings: &'a FrozenStringTable,
    ) -> Result<&'a FrozenStringTable, crate::compiler_frontend::compiler_errors::CompilerError>
    {
        match self {
            Self::Source { .. } => Ok(donor_strings),
            Self::MaterialisedCanonical {
                source_path_table: Some(_),
                source_string_table: None,
                ..
            } => Err(
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                    "materialised generic body has an incomplete source identity table pair",
                ),
            ),
            Self::MaterialisedCanonical {
                source_string_table: Some(source_strings),
                ..
            } => Ok(source_strings),
            Self::MaterialisedCanonical { .. } => Ok(donor_strings),
            Self::MaterialisedForeign {
                source_path_table: Some(_),
                source_string_table: None,
                ..
            } => Err(
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                    "materialised generic body has an incomplete source identity table pair",
                ),
            ),
            Self::MaterialisedForeign {
                source_string_table: Some(source_strings),
                ..
            } => Ok(source_strings),
            Self::MaterialisedForeign {
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
    /// String-only foreign donors scan through their canonical owner; only path-crossing
    /// foreign donors scan through the retained `FileTokens` shell.
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
            }
            | Self::MaterialisedCanonical {
                source_owner,
                token_range,
                token_sequence,
                declaration_path,
                canonical_os_path,
                ..
            } => bounded_adapter_from_canonical(
                source_owner,
                canonical_os_path.clone(),
                *token_range,
                *token_sequence,
                *declaration_path,
            ),
            Self::MaterialisedForeign {
                source_owner,
                token_range,
                token_sequence,
                declaration_path,
                source_path_table: None,
                ..
            } => {
                let canonical_owner = source_owner.canonical_source_tokens_arc().map_err(|_| {
                    crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                        "frozen generic body capture needs a canonical donor owner",
                    )
                })?;
                bounded_adapter_from_canonical(
                    &canonical_owner,
                    source_owner.canonical_os_path.clone(),
                    *token_range,
                    *token_sequence,
                    *declaration_path,
                )
            },
            Self::MaterialisedForeign {
                source_owner,
                token_range,
                token_sequence,
                declaration_path,
                source_path_table,
                source_string_table,
                ..
            } => {
                if source_path_table.is_some() && source_string_table.is_none() {
                    return Err(
                        crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                            "materialised generic body has an incomplete source identity table pair",
                        ),
                    );
                }
                bounded_adapter(
                    source_owner,
                    *token_range,
                    *token_sequence,
                    *declaration_path,
                )
            }
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
            Self::MaterialisedCanonical { .. } => "MaterialisedCanonical",
            Self::MaterialisedForeign { .. } => "MaterialisedForeign",
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
    /// ordinary facts; generated templates use the `MaterialisedCanonical`/`MaterialisedForeign`
    /// variants, which own retained donor file-reference facts for their canonical source range.
    pub(crate) body_tokens: Option<GenericFunctionBody>,
    pub(crate) declaration_span: Option<crate::compiler_frontend::source::SourceSpan>,
}
