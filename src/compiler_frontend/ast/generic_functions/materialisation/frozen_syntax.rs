//! Frozen generic body ownership and donor file-reference capture.
//!
//! Persistent generic syntax keeps the canonical declaring-source owner and checked body view.
//! Compatibility parser adapters are materialised only for the current operation, while stable
//! Stage 0 file-reference facts cross the source-preparation lifetime independently.

use super::frozen_file_references::StableResolvedFileReference;
use crate::compiler_frontend::ast::generic_functions::templates::{
    GenericFunctionBody, MaterialisedDonorContext,
};
use crate::compiler_frontend::ast::module_ast::scope_context::Stage0ResolutionFacts;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceId};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork, PathTable};
use crate::compiler_frontend::symbols::string_interning::{FrozenStringTable, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{
    FileTokens, SourceTokens, TokenKind, TokenRange, TokenSequenceId,
};
use std::path::PathBuf;
use std::sync::Arc;

/// Durable generic body syntax.
///
/// The owner/range pair is the only retained syntax payload. Source-origin syntax shares the
/// canonical `SourceTokens` owner from prepared-source publication; Materialised-origin syntax
/// keeps the donor `FileTokens` shell (including its remapped compatibility lane) until H5.
/// Neither lane retains a parser adapter or copied token vector. File-reference rows remain
/// stable semantic facts for generated value resolution.
#[derive(Clone)]
pub(super) struct StableBodySyntax {
    pub(super) declaration_path: PathId,
    pub(super) donor_file_id: SourceId,
    pub(super) frozen_identity_handle: FrozenIdentityHandle,
    pub(super) source_owner: StableBodyOwner,
    pub(super) token_range: TokenRange,
    pub(super) token_sequence: Option<TokenSequenceId>,
    pub(super) source_path_table: Option<Arc<PathTable>>,
    pub(super) source_string_table: Option<Arc<FrozenStringTable>>,
    pub(super) resolved_file_references: Box<[StableResolvedFileReference]>,
}

/// Canonical owner retained by durable generic body syntax.
///
/// Source bodies share the declaring module's immutable `SourceTokens` allocation plus the
/// filesystem identity that `SourceTokens` itself does not store. Materialised bodies retain the
/// donor `FileTokens` shell so rebased compatibility payloads survive until H5.
#[derive(Clone, Debug)]
pub(super) enum StableBodyOwner {
    Source {
        source_tokens: Arc<SourceTokens>,
        canonical_os_path: Option<PathBuf>,
    },
    Materialised {
        source_owner: Arc<FileTokens>,
    },
}
impl StableBodySyntax {
    fn materialised_owner(&self) -> Result<Arc<FileTokens>, CompilerError> {
        match &self.source_owner {
            StableBodyOwner::Source {
                source_tokens,
                canonical_os_path,
            } => FileTokens::canonical_shell_from_canonical(
                Arc::clone(source_tokens),
                canonical_os_path.clone(),
                self.declaration_path,
                self.token_range,
                self.token_sequence,
            )
            .map(Arc::new),
            StableBodyOwner::Materialised { source_owner } => Ok(Arc::clone(source_owner)),
        }
    }
    #[cfg(test)]
    pub(super) fn canonical_donor_file_id(&self) -> SourceId {
        self.donor_file_id
    }
}

/// Materialised body payload passed to generated AST construction.
pub(super) struct MaterialisedBody {
    pub(super) source_owner: Arc<FileTokens>,
    pub(super) token_range: TokenRange,
    pub(super) token_sequence: Option<TokenSequenceId>,
    pub(super) declaration_path: PathId,
    pub(super) resolution_facts: Arc<Stage0ResolutionFacts>,
    pub(super) frozen_identity_handle: FrozenIdentityHandle,
    pub(super) source_path_table: Option<Arc<PathTable>>,
    pub(super) source_string_table: Option<Arc<FrozenStringTable>>,
}

impl MaterialisedBody {
    pub(super) fn into_generic_body(self) -> Result<GenericFunctionBody, CompilerError> {
        GenericFunctionBody::materialised(
            self.source_owner,
            self.token_range,
            self.token_sequence,
            self.declaration_path,
            MaterialisedDonorContext {
                resolution_facts: self.resolution_facts,
                frozen_identity_handle: self.frozen_identity_handle,
                source_path_table: self.source_path_table,
                source_string_table: self.source_string_table,
            },
        )
    }
}

impl std::fmt::Debug for MaterialisedBody {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MaterialisedBody")
            .field("donor_file_id", &self.source_owner.file_id)
            .field("token_range", &self.token_range)
            .field("token_sequence", &self.token_sequence)
            .finish_non_exhaustive()
    }
}

impl StableBodySyntax {
    pub(super) fn remap_path_ids(
        &mut self,
        remap: &crate::compiler_frontend::symbols::path_interner::PathIdRemap,
    ) {
        self.declaration_path = remap.get(self.declaration_path);
    }

    pub(super) fn capture(
        body: &GenericFunctionBody,
        source_file: PathId,
        path_fork: &PathInternerFork,
        string_table: &mut StringTable,
        stage0_resolution_facts: Option<&Stage0ResolutionFacts>,
        frozen_identity_handle: FrozenIdentityHandle,
        content_value_at_path: &impl Fn(
            &PathId,
        ) -> Result<
            crate::compiler_frontend::folded_value::PublicFoldedValue,
            CompilerError,
        >,
    ) -> Result<Self, CompilerError> {
        if !path_fork.starts_with(body.declaration_path(), source_file) {
            return Err(CompilerError::compiler_error(
                "frozen generic body declaration path is outside its owning source file",
            ));
        }
        let donor_file_id = body.donor_source_id();
        let source_owner = match body {
            GenericFunctionBody::Source {
                source_owner,
                canonical_os_path,
                ..
            } => StableBodyOwner::Source {
                source_tokens: Arc::clone(source_owner),
                canonical_os_path: canonical_os_path.clone(),
            },
            GenericFunctionBody::Materialised { source_owner, .. } => {
                StableBodyOwner::Materialised {
                    source_owner: Arc::clone(source_owner),
                }
            }
        };
        let token_stream = body.parser_stream_for_capture(string_table)?;
        let path_syntax = token_stream.path_syntax_table()?;
        path_syntax.validate_file_tokens(
            &token_stream.tokens,
            donor_file_id,
            "generic body capture",
        )?;
        let mut seen_paths = rustc_hash::FxHashSet::default();
        let mut resolved_file_references = Vec::new();
        for token in &token_stream.tokens {
            let TokenKind::Path(source_path_id) = token.kind else {
                continue;
            };
            if !seen_paths.insert(source_path_id) {
                continue;
            }
            let facts = stage0_resolution_facts.ok_or_else(|| {
                CompilerError::compiler_error(
                    "persistent generic body has path syntax but no Stage 0 resolution facts",
                )
            })?;
            let resolved = facts
                .lookup(donor_file_id, source_path_id)?
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "persistent generic body path handle {:?} had no matching Stage 0 resolved-reference row",
                        source_path_id
                    ))
                })?;
            resolved_file_references.push(StableResolvedFileReference::capture(
                source_path_id,
                resolved,
                content_value_at_path,
            )?);
        }

        let (source_path_table, source_string_table) =
            if let Some((source_path_table, source_string_table)) = body.source_identity_tables() {
                (
                    Some(Arc::clone(source_path_table)),
                    Some(Arc::clone(source_string_table)),
                )
            } else {
                (None, Some(Arc::new(string_table.clone().freeze())))
            };
        Ok(Self {
            declaration_path: body.declaration_path(),
            donor_file_id,
            frozen_identity_handle,
            source_owner,
            token_range: body.token_range(),
            token_sequence: body.token_sequence(),
            source_path_table,
            source_string_table,
            resolved_file_references: resolved_file_references.into_boxed_slice(),
        })
    }

    pub(super) fn materialise(
        &self,
        source_file: PathId,
        path_fork: &mut PathInternerFork,
        _string_table: &mut StringTable,
        identity_tables: Option<(&Arc<PathTable>, &Arc<FrozenStringTable>)>,
    ) -> Result<MaterialisedBody, CompilerError> {
        // Validate the retained canonical owner before carrying its range/sequence to a parser
        // consumer; parser adapters themselves are intentionally deferred until that boundary.
        // Source-origin syntax validates directly against `SourceTokens`; Materialised-origin
        // syntax keeps the `FileTokens`-shell validator so rebased compatibility payloads stay
        // valid until H5.
        match &self.source_owner {
            StableBodyOwner::Source { source_tokens, .. } => {
                crate::compiler_frontend::ast::generic_functions::templates::validate_source_owner(
                    source_tokens,
                    self.token_range,
                    self.token_sequence,
                    "frozen generic body materialisation",
                )?;
            }
            StableBodyOwner::Materialised { source_owner } => {
                crate::compiler_frontend::ast::generic_functions::templates::validate_syntax_owner(
                    source_owner,
                    self.token_range,
                    self.token_sequence,
                    "frozen generic body materialisation",
                )?;
            }
        }
        if !path_fork.starts_with(self.declaration_path, source_file) {
            return Err(CompilerError::compiler_error(
                "frozen generic body declaration path is outside its materialised source file",
            ));
        }

        let resolved_file_references = self
            .resolved_file_references
            .iter()
            .map(StableResolvedFileReference::materialise)
            .collect::<Result<Vec<_>, CompilerError>>()?;
        let resolution_facts = Arc::new(Stage0ResolutionFacts::frozen_generic(
            self.donor_file_id,
            resolved_file_references,
        )?);
        let (source_path_table, source_string_table) =
            match (&self.source_path_table, &self.source_string_table) {
                (Some(path_table), Some(source_strings)) => (
                    Some(Arc::clone(path_table)),
                    Some(Arc::clone(source_strings)),
                ),
                (None, Some(source_strings)) => (None, Some(Arc::clone(source_strings))),
                (None, None) => identity_tables
                    .map(|(path_table, source_strings)| {
                        (
                            Some(Arc::clone(path_table)),
                            Some(Arc::clone(source_strings)),
                        )
                    })
                    .unwrap_or((None, None)),
                (Some(_), None) => {
                    return Err(CompilerError::compiler_error(
                        "frozen generic body has an incomplete source identity table pair",
                    ));
                }
            };
        let materialised_owner = self.materialised_owner()?;
        Ok(MaterialisedBody {
            source_owner: materialised_owner,
            token_range: self.token_range,
            token_sequence: self.token_sequence,
            declaration_path: self.declaration_path,
            resolution_facts,
            frozen_identity_handle: self.frozen_identity_handle.clone(),
            source_path_table,
            source_string_table,
        })
    }
}
