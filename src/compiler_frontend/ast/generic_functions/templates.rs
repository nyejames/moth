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
use crate::compiler_frontend::symbols::path_interner::PathId;

use crate::compiler_frontend::tokenizer::tokens::FileTokens;
use std::fmt;
use std::sync::Arc;

/// One generic function body backed by one canonical source owner.
///
/// WHAT: retains only the canonical `FileTokens` owner, a checked contiguous range or segmented
/// sequence and the declaration/donor identities needed by later parsers.
/// WHY: generic syntax can outlive the declaring AST pass, but it must never retain another token
/// vector or a second `SourceTokens` store. Parser consumers derive a bounded adapter on demand.
#[derive(Clone)]
pub(crate) enum GenericFunctionBody {
    /// Source templates use the declaring module's canonical prepared source owner.
    Source {
        source_owner: Arc<FileTokens>,
        token_range: crate::compiler_frontend::tokenizer::tokens::TokenRange,
        token_sequence: Option<crate::compiler_frontend::tokenizer::tokens::TokenSequenceId>,
        declaration_path: PathId,
    },
    /// Materialised bodies retain the donor owner and frozen Stage 0/identity context.
    Materialised {
        source_owner: Arc<FileTokens>,
        token_range: crate::compiler_frontend::tokenizer::tokens::TokenRange,
        token_sequence: Option<crate::compiler_frontend::tokenizer::tokens::TokenSequenceId>,
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

impl GenericFunctionBody {
    pub(crate) fn source(
        source_owner: Arc<FileTokens>,
        token_range: crate::compiler_frontend::tokenizer::tokens::TokenRange,
        token_sequence: Option<crate::compiler_frontend::tokenizer::tokens::TokenSequenceId>,
        declaration_path: PathId,
    ) -> Result<Self, crate::compiler_frontend::compiler_errors::CompilerError> {
        validate_syntax_owner(
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
        source_owner: Arc<FileTokens>,
        token_range: crate::compiler_frontend::tokenizer::tokens::TokenRange,
        token_sequence: Option<crate::compiler_frontend::tokenizer::tokens::TokenSequenceId>,
        declaration_path: PathId,
        resolution_facts: Arc<Stage0ResolutionFacts>,
        frozen_identity_handle: FrozenIdentityHandle,
        source_path_table: Option<Arc<crate::compiler_frontend::symbols::path_interner::PathTable>>,
        source_string_table:
            Option<Arc<crate::compiler_frontend::symbols::string_interning::FrozenStringTable>>,
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
            resolution_facts,
            frozen_identity_handle,
            source_path_table,
            source_string_table,
        })
    }

    pub(crate) fn source_owner(&self) -> &Arc<FileTokens> {
        match self {
            Self::Source { source_owner, .. }
            | Self::Materialised { source_owner, .. } => source_owner,
        }
    }

    pub(crate) fn token_range(
        &self,
    ) -> crate::compiler_frontend::tokenizer::tokens::TokenRange {
        match self {
            Self::Source { token_range, .. }
            | Self::Materialised { token_range, .. } => *token_range,
        }
    }

    pub(crate) fn token_sequence(
        &self,
    ) -> Option<crate::compiler_frontend::tokenizer::tokens::TokenSequenceId> {
        match self {
            Self::Source { token_sequence, .. }
            | Self::Materialised { token_sequence, .. } => *token_sequence,
        }
    }

    pub(crate) fn declaration_path(&self) -> PathId {
        match self {
            Self::Source { declaration_path, .. }
            | Self::Materialised { declaration_path, .. } => *declaration_path,
        }
    }

    /// Derive the bounded parser adapter for this body.
    ///
    /// Materialised bodies additionally rebase donor payloads through their retained provider
    /// identity pair. The returned adapter is intentionally short-lived and is never retained.
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
            } => bounded_adapter(
                source_owner,
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
                    source_path_table.as_deref(),
                    source_strings,
                    string_table,
                    Some(path_fork),
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
    /// Derive a bounded adapter for stable capture without mutating the active path fork.
    ///
    /// Capture only needs donor path rows and payload spelling. Full path rebasing happens at the
    /// generated parser boundary, after the provider identity tables are available.
    pub(crate) fn parser_stream_for_capture(
        &self,
        string_table: &mut crate::compiler_frontend::symbols::string_interning::StringTable,
    ) -> Result<FileTokens, crate::compiler_frontend::compiler_errors::CompilerError> {
        match self {
            Self::Source {
                source_owner,
                token_range,
                token_sequence,
                declaration_path,
            } => bounded_adapter(
                source_owner,
                *token_range,
                *token_sequence,
                *declaration_path,
            ),
            Self::Materialised {
                source_owner,
                token_range,
                token_sequence,
                declaration_path,
                source_string_table,
                ..
            } => match source_string_table.as_deref() {
                Some(source_strings) => remapped_bounded_adapter(
                    source_owner,
                    *token_range,
                    *token_sequence,
                    *declaration_path,
                    None,
                    source_strings,
                    string_table,
                    None,
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
}

pub(crate) fn validate_syntax_owner(
    source_owner: &FileTokens,
    token_range: crate::compiler_frontend::tokenizer::tokens::TokenRange,
    token_sequence: Option<crate::compiler_frontend::tokenizer::tokens::TokenSequenceId>,
    role: &str,
) -> Result<(), crate::compiler_frontend::compiler_errors::CompilerError> {
    if token_range.source() != source_owner.file_id {
        return Err(crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
            format!("{role} token range has a foreign source identity"),
        ));
    }
    let canonical = source_owner.source_tokens()?;
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

pub(crate) fn bounded_adapter(
    source_owner: &FileTokens,
    token_range: crate::compiler_frontend::tokenizer::tokens::TokenRange,
    token_sequence: Option<crate::compiler_frontend::tokenizer::tokens::TokenSequenceId>,
    declaration_path: PathId,
) -> Result<FileTokens, crate::compiler_frontend::compiler_errors::CompilerError> {
    if let Some(sequence) = token_sequence {
        FileTokens::new_bounded_sequence_substream(source_owner, sequence, declaration_path)
    } else {
        FileTokens::new_bounded_substream(source_owner, token_range, declaration_path)
    }
}


pub(crate) fn remapped_bounded_adapter(
    source_owner: &FileTokens,
    token_range: crate::compiler_frontend::tokenizer::tokens::TokenRange,
    token_sequence: Option<crate::compiler_frontend::tokenizer::tokens::TokenSequenceId>,
    declaration_path: PathId,
    source_path_table: Option<&crate::compiler_frontend::symbols::path_interner::PathTable>,
    source_strings: &impl crate::compiler_frontend::symbols::string_interning::StringTableResolver,
    destination_strings: &mut crate::compiler_frontend::symbols::string_interning::StringTable,
    path_fork: Option<&mut crate::compiler_frontend::symbols::path_interner::PathInternerFork>,
) -> Result<FileTokens, crate::compiler_frontend::compiler_errors::CompilerError> {
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
    Ok(FileTokens::new_remapped_adapter(
        declaration_path,
        adapter.file_id,
        adapter.canonical_os_path.clone(),
        adapter.tokens,
        destination_path_syntax,
    ))
}
impl fmt::Debug for GenericFunctionBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct(match self {
            Self::Source { .. } => "Source",
            Self::Materialised { .. } => "Materialised",
        });
        debug
            .field("source", &self.source_owner().file_id)
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
