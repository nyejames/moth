//! Frozen token and path syntax retained by generic-function materialisation.
//!
//! Captured bodies use a compact immutable string pool and canonical path-syntax table. The pool
//! is merged into the generated string table exactly once when a body is materialised.

use super::frozen_file_references::StableResolvedFileReference;
use crate::compiler_frontend::ast::generic_functions::GenericFunctionBody;
use crate::compiler_frontend::ast::module_ast::scope_context::Stage0ResolutionFacts;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::numeric_text::store::{NumericLiteralId, NumericLiteralStore};
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::FrozenIdentityHandle;
use crate::compiler_frontend::symbols::path_interner::{
    PathId, PathIdRemap, PathInternerFork,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token, TokenKind};
use std::sync::Arc;

/// Owned frozen token buffer retained by one generic declaration artefact.
///
/// WHAT: preserves the already-tokenized body as canonical [`Token`] values whose `StringId`
///       payloads index one context-local immutable frozen string pool, plus the exact donor
///       `SourceId` that owns the body's spans and path rows.
/// WHY: successful metadata must not retain donor `StringId`, `PathId`, filesystem paths,
///      or a mutable string table. Freezing remaps donor string IDs into the pool once, while the
///      donor `SourceId` is retained verbatim as the materialised owner. Materialisation merges
///      the pool into the fresh generated-local table once and remaps every token payload through
///      that single pool remap, without running tokenization again.
/// OWNERSHIP: every materialised token stream carries this donor identity via
///      `FileTokens::new_frozen(..., donor_file_id, ...)`, the frozen facts owner and the
///      late-bound `FrozenIdentityHandle`. The final render boundary installs the matching
///      project/package context before resolving labels; donor ranges are never silently
///      remapped onto the requester call-site source, and no magic identity or `None` fallback
///      is fabricated.
#[derive(Clone)]
pub(super) struct StableBodySyntax {
    /// Declaration-qualified stream path, such as `file/generic_function`.
    ///
    /// This path names the token stream's semantic declaration context. The owning source-file
    /// identity lives in `donor_file_id`, because token and path-row locations are file-scoped
    /// rather than declaration-scoped.
    pub(super) declaration_path: PathId,
    /// Exact owning source identity captured from `FileTokens::file_id`.
    ///
    /// Retained verbatim so materialisation can restore a concrete `FileTokens::file_id` and a
    /// matching frozen-facts owner.
    pub(super) donor_file_id: crate::compiler_frontend::source::SourceId,
    /// Required late-bound frozen identity for the donor source domain.
    pub(super) frozen_identity_handle: FrozenIdentityHandle,
    pub(super) pool: Box<[String]>,
    pub(super) tokens: Box<[Token]>,
    /// Persistent numeric side-store rows and compact handles owned by this body.
    pub(super) numeric_literals: NumericLiteralStore,
    pub(super) numeric_literal_ids: Box<[Option<NumericLiteralId>]>,
    /// Canonical table vocabulary retained only for the path rows referenced by this body.
    /// Its StringIds index `pool` until materialisation remaps the whole table in place.
    pub(super) path_syntax: PathSyntaxTable,
    pub(super) resolved_file_references: Box<[StableResolvedFileReference]>,
}

/// Materialised body payload passed to the generic-function AST builder.
pub(super) struct MaterialisedBody {
    pub(super) file_tokens: FileTokens,
    pub(super) resolution_facts: Arc<Stage0ResolutionFacts>,
    pub(super) frozen_identity_handle: FrozenIdentityHandle,
}

impl MaterialisedBody {
    pub(super) fn into_generic_body(self) -> GenericFunctionBody {
        GenericFunctionBody::materialised(
            self.file_tokens,
            self.resolution_facts,
            self.frozen_identity_handle,
        )
    }
}

impl std::fmt::Debug for MaterialisedBody {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MaterialisedBody")
            .finish_non_exhaustive()
    }
}

impl StableBodySyntax {
    pub(super) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.declaration_path = remap.get(self.declaration_path);
        self.path_syntax.remap_path_ids(remap);
    }

    pub(super) fn capture(
        tokens: &FileTokens,
        source_file: PathId,
        path_fork: &PathInternerFork,
        string_table: &StringTable,
        stage0_resolution_facts: Option<&Stage0ResolutionFacts>,
        frozen_identity_handle: FrozenIdentityHandle,
        content_value_at_path: &impl Fn(
            &PathId,
        ) -> Result<
            crate::compiler_frontend::folded_value::PublicFoldedValue,
            CompilerError,
        >,
    ) -> Result<Self, CompilerError> {
        if !path_fork.starts_with(tokens.src_path, source_file) {
            return Err(CompilerError::compiler_error(
                "frozen generic body declaration path is outside its owning source file",
            ));
        }

        let source_path_syntax = tokens.path_syntax_table()?;
        source_path_syntax.validate_file_owned_locations(tokens.file_id)?;
        source_path_syntax.validate_file_tokens(
            &tokens.tokens,
            tokens.file_id,
            "generic body capture",
        )?;
        if let Some(owner) = tokens.numeric_literal_store().owner_source()
            && owner != tokens.file_id
        {
            return Err(CompilerError::compiler_error(
                "generic body numeric store does not match its enclosing source identity",
            ));
        }
        let mut pool = FrozenStringPool::default();
        let mut frozen_tokens = tokens.tokens.clone();
        let (path_syntax, path_syntax_map) =
            source_path_syntax.capture_persistent_generic_subset(&mut frozen_tokens)?;
        let numeric_ids = tokens.numeric_literal_ids.iter().flatten().copied();
        validate_capture_numeric_ids(tokens, numeric_ids.clone())?;
        let (mut numeric_literals, numeric_id_map) = tokens
            .numeric_literal_store()
            .compact_subset(numeric_ids)
            .map_err(|error| {
                CompilerError::compiler_error(format!(
                    "persistent generic numeric literal subset is invalid: {error:?}"
                ))
            })?;
        let mut frozen_numeric_ids = tokens.numeric_literal_ids.clone();
        for id in &mut frozen_numeric_ids {
            if let Some(old_id) = id {
                *id = numeric_id_map.get(old_id).copied();
            }
        }
        validate_compact_numeric_ids(&frozen_tokens, &frozen_numeric_ids, &numeric_literals)?;
        let mut path_syntax_map = path_syntax_map.into_iter().collect::<Vec<_>>();
        path_syntax_map.sort_by_key(|(_, compact_id)| *compact_id);

        let mut resolved_file_references = Vec::with_capacity(path_syntax_map.len());
        for (source_path_id, compact_path_id) in path_syntax_map {
            let facts = stage0_resolution_facts.ok_or_else(|| {
                CompilerError::compiler_error(
                    "persistent generic body has path syntax but no Stage 0 resolution facts",
                )
            })?;
            let resolved = facts
                .lookup(tokens.file_id, source_path_id)?
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "persistent generic body path handle {:?} had no matching Stage 0 resolved-reference row",
                        source_path_id
                    ))
                })?;
            resolved_file_references.push(StableResolvedFileReference::capture(
                compact_path_id,
                resolved,
                &mut |text| pool.index(text),
                content_value_at_path,
            )?);
        }

        // The source stream owns the complete table. A persistent generic retains only the
        // referenced canonical subset, then token and table payloads enter the same frozen pool.
        for token in &mut frozen_tokens {
            token.try_remap_string_ids(&mut |id| {
                Ok::<StringId, CompilerError>(pool.index(string_table.resolve(id)))
            })?;
        }
        numeric_literals
            .try_remap_string_ids(&mut |id| {
                Ok::<StringId, CompilerError>(pool.index(string_table.resolve(id)))
            })
            .map_err(|error| {
                CompilerError::compiler_error(format!(
                    "persistent generic numeric literal remap failed: {error:?}"
                ))
            })?;
        numeric_literals.freeze();
        // Path rows already use the build-wide path identity domain; only token string payloads
        // enter the frozen pool here.

        Ok(Self {
            declaration_path: tokens.src_path,
            donor_file_id: tokens.file_id,
            frozen_identity_handle,
            pool: pool.finish(),
            tokens: frozen_tokens.into_boxed_slice(),
            numeric_literals,
            numeric_literal_ids: frozen_numeric_ids.into_boxed_slice(),
            path_syntax,
            resolved_file_references: resolved_file_references.into_boxed_slice(),
        })
    }

    pub(super) fn materialise(
        &self,
        source_file: PathId,
        path_fork: &PathInternerFork,
        string_table: &mut StringTable,
    ) -> Result<MaterialisedBody, CompilerError> {
        let declaration_path = self.declaration_path;
        if !path_fork.starts_with(declaration_path, source_file) {
            return Err(CompilerError::compiler_error(
                "frozen generic body declaration path is outside its materialised source file",
            ));
        }
        let remap = self
            .pool
            .iter()
            .map(|text| string_table.intern(text))
            .collect::<Vec<_>>();
        let mut tokens = Vec::with_capacity(self.tokens.len());
        for token in self.tokens.iter() {
            let mut materialised = token.clone();
            materialised.try_remap_string_ids(&mut |id| {
                let index = id.index() as usize;
                remap.get(index).copied().ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "frozen token payload references out-of-range pool entry {index}"
                    ))
                })
            })?;
            tokens.push(materialised);
        }
        let mut numeric_literals = self.numeric_literals.clone_for_materialisation();
        numeric_literals
            .try_remap_string_ids(&mut |id| {
                let index = id.index() as usize;
                remap.get(index).copied().ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "frozen numeric literal payload references out-of-range pool entry {index}"
                    ))
                })
            })
            .map_err(|error| {
                CompilerError::compiler_error(format!(
                    "frozen numeric literal materialisation failed: {error:?}"
                ))
            })?;
        numeric_literals.freeze();
        let path_syntax = self.path_syntax.clone();
        path_syntax.validate_file_owned_locations(self.donor_file_id)?;
        path_syntax.validate_file_tokens(&tokens, self.donor_file_id, "frozen generic body")?;
        validate_numeric_ids(&numeric_literals, &self.numeric_literal_ids, tokens.len())?;
        if let Some(owner) = numeric_literals.owner_source()
            && owner != self.donor_file_id
        {
            return Err(CompilerError::compiler_error(
                "frozen numeric literal store does not match its donor source identity",
            ));
        }

        let resolved_file_references = self
            .resolved_file_references
            .iter()
            .map(|reference| reference.materialise(&remap, string_table))
            .collect::<Result<Vec<_>, CompilerError>>()?;
        let resolution_facts = Arc::new(Stage0ResolutionFacts::frozen_generic(
            self.donor_file_id,
            resolved_file_references,
        )?);

        // Retain the captured donor identity as the explicit materialised owner. The final
        // boundary installs its domain-specific frozen context before labels render, so donor
        // ranges stay distinct from the requester call-site source without any rebinding, magic
        // identity or `None` fallback.
        Ok(MaterialisedBody {
            file_tokens: FileTokens::new_frozen_with_numeric_store(
                declaration_path,
                self.donor_file_id,
                None,
                tokens,
                path_syntax,
                numeric_literals,
                self.numeric_literal_ids.to_vec(),
            ),
            resolution_facts,
            frozen_identity_handle: self.frozen_identity_handle.clone(),
        })
    }
}

/// Validate frozen numeric handles without aliasing the donor store.
///
/// The artefact owns a compact copy; handles must align with the materialised tokens and
/// address rows in that copy. Any stale donor handle is retained-state corruption.
fn validate_numeric_ids(
    store: &NumericLiteralStore,
    ids: &[Option<NumericLiteralId>],
    token_len: usize,
) -> Result<(), CompilerError> {
    if ids.len() != token_len {
        return Err(CompilerError::compiler_error(
            "frozen numeric literal handles do not align with the retained token slice",
        ));
    }
    for id in ids.iter().flatten() {
        store.try_get(*id).map_err(|_| {
            CompilerError::compiler_error(
                "frozen numeric literal handle does not address a retained numeric row",
            )
        })?;
    }
    Ok(())
}

/// The donor handles captured for one persistent body must align with its tokens and address
/// rows in the donor store. A non-numeric position must never carry a handle, and a dropped
/// numeric handle would silently detach a literal from its lexical record.
fn validate_capture_numeric_ids(
    tokens: &FileTokens,
    ids: impl Iterator<Item = NumericLiteralId> + Clone,
) -> Result<(), CompilerError> {
    let referenced = ids.clone().count();
    let mut numeric_positions = 0usize;
    for (token, id) in tokens.tokens.iter().zip(tokens.numeric_literal_ids.iter()) {
        let is_numeric = matches!(token.kind, TokenKind::NumericLiteral(_));
        match (is_numeric, id) {
            (true, Some(handle)) => {
                numeric_positions += 1;
                tokens.numeric_literal_store().try_get(*handle).map_err(|_| {
                    CompilerError::compiler_error(
                        "persistent generic numeric handle does not address a donor numeric row",
                    )
                })?;
            }
            (true, None) => {
                return Err(CompilerError::compiler_error(
                    "persistent generic numeric token is missing its side-store handle",
                ));
            }
            (false, Some(_)) => {
                return Err(CompilerError::compiler_error(
                    "persistent generic non-numeric token carries a numeric handle",
                ));
            }
            (false, None) => {}
        }
    }
    if referenced != numeric_positions {
        return Err(CompilerError::compiler_error(
            "persistent generic numeric handle slice does not align with its tokens",
        ));
    }
    Ok(())
}

/// The compact copy must carry exactly the handles referenced by the retained tokens: every
/// numeric position resolves inside the copy and no stale donor handle survives.
fn validate_compact_numeric_ids(
    tokens: &[Token],
    ids: &[Option<NumericLiteralId>],
    store: &NumericLiteralStore,
) -> Result<(), CompilerError> {
    validate_numeric_ids(store, ids, tokens.len())?;
    for (token, id) in tokens.iter().zip(ids.iter()) {
        let is_numeric = matches!(token.kind, TokenKind::NumericLiteral(_));
        match (is_numeric, id) {
            (true, Some(_)) | (false, None) => {}
            (true, None) => {
                return Err(CompilerError::compiler_error(
                    "persistent generic numeric token lost its compact handle",
                ));
            }
            (false, Some(_)) => {
                return Err(CompilerError::compiler_error(
                    "persistent generic non-numeric token carries a compact numeric handle",
                ));
            }
        }
    }
    Ok(())
}

// Path IDs are build-wide identities. The frozen artefact retains them directly rather than
// rendering to text and interning a second logical path tree at materialisation time.


#[derive(Default)]
pub(super) struct FrozenStringPool {
    entries: Vec<String>,
    by_text: rustc_hash::FxHashMap<String, u32>,
}

impl FrozenStringPool {
    fn index(&mut self, text: &str) -> StringId {
        if let Some(index) = self.by_text.get(text) {
            return StringId::from_index(*index);
        }

        let index = self.entries.len() as u32;
        let owned = text.to_owned();
        self.entries.push(owned.clone());
        self.by_text.insert(owned, index);
        StringId::from_index(index)
    }

    fn finish(self) -> Box<[String]> {
        self.entries.into_boxed_slice()
    }
}

