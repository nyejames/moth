//! Frozen token and path syntax retained by generic-function materialisation.
//!
//! Captured bodies use a compact immutable string pool and canonical path-syntax table. The pool
//! is merged into the generated string table exactly once when a body is materialised.

use super::frozen_file_references::StableResolvedFileReference;
use crate::compiler_frontend::ast::generic_functions::GenericFunctionBody;
use crate::compiler_frontend::ast::module_ast::scope_context::Stage0ResolutionFacts;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::FrozenIdentityHandle;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token};
use std::sync::Arc;

/// Owned frozen token buffer retained by one generic declaration artefact.
///
/// WHAT: preserves the already-tokenized body as canonical [`Token`] values whose `StringId`
///       payloads index one context-local immutable frozen string pool, plus the exact donor
///       `SourceId` that owns the body's spans and path rows.
/// WHY: successful metadata must not retain donor `StringId`, `InternedPath`, filesystem paths,
///      or a mutable string table. Freezing remaps donor string IDs into the pool once, while the
///      donor `SourceId` is retained verbatim as the materialised owner. Materialisation merges
///      the pool into the fresh generated-local table once and remaps every token payload through
///      that single pool remap, without running tokenization again.
/// OWNERSHIP: every materialised token stream carries this donor identity via
///      `FileTokens::new_frozen(..., donor_file_id, ...)` and the frozen facts owner. The donor
///      range is never silently remapped onto the requester call-site source, no magic identity
///      is fabricated, and independent package/cross-module handles stay distinct. A future
///      `FrozenIdentityContext` remap across compilation databases remains the only sanctioned
///      rebinding; until that boundary lands, the donor identity is the explicit owner.
#[derive(Clone)]
pub(super) struct StableBodySyntax {
    /// Declaration-qualified stream path, such as `file/generic_function`.
    ///
    /// This path names the token stream's semantic declaration context. The owning source-file
    /// identity lives in `donor_file_id`, because token and path-row locations are file-scoped
    /// rather than declaration-scoped.
    pub(super) declaration_path: Box<[String]>,
    /// Exact owning source identity captured from `FileTokens::file_id`.
    ///
    /// Retained verbatim so materialisation can restore a concrete `FileTokens::file_id` and a
    /// matching frozen-facts owner.
    pub(super) donor_file_id: crate::compiler_frontend::source::SourceId,
    /// Required late-bound frozen identity for the donor source domain.
    pub(super) frozen_identity_handle: FrozenIdentityHandle,
    pub(super) pool: Box<[String]>,
    pub(super) tokens: Box<[Token]>,
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
    pub(super) fn capture(
        tokens: &FileTokens,
        source_file: &InternedPath,
        string_table: &StringTable,
        stage0_resolution_facts: Option<&Stage0ResolutionFacts>,
        frozen_identity_handle: FrozenIdentityHandle,
        content_value_at_path: &impl Fn(
            &InternedPath,
        ) -> Result<
            crate::compiler_frontend::folded_value::PublicFoldedValue,
            CompilerError,
        >,
    ) -> Result<Self, CompilerError> {
        if !tokens.src_path.starts_with(source_file) {
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

        let mut pool = FrozenStringPool::default();
        let mut frozen_tokens = tokens.tokens.clone();
        let (mut path_syntax, path_syntax_map) =
            source_path_syntax.capture_persistent_generic_subset(&mut frozen_tokens)?;
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
        path_syntax.try_remap_string_ids(&mut |id| {
            Ok::<StringId, CompilerError>(pool.index(string_table.resolve(id)))
        })?;

        Ok(Self {
            declaration_path: stable_path(&tokens.src_path, string_table),
            donor_file_id: tokens.file_id,
            frozen_identity_handle,
            pool: pool.finish(),
            tokens: frozen_tokens.into_boxed_slice(),
            path_syntax,
            resolved_file_references: resolved_file_references.into_boxed_slice(),
        })
    }

    pub(super) fn materialise(
        &self,
        source_file: &InternedPath,
        string_table: &mut StringTable,
    ) -> Result<MaterialisedBody, CompilerError> {
        let declaration_path = materialise_path(&self.declaration_path, string_table);
        if !declaration_path.starts_with(source_file) {
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
        let mut path_syntax = self.path_syntax.clone();
        path_syntax.try_remap_string_ids(&mut |id| pool_remap(id, &remap))?;
        path_syntax.validate_file_owned_locations(self.donor_file_id)?;
        path_syntax.validate_file_tokens(&tokens, self.donor_file_id, "frozen generic body")?;

        let resolved_file_references = self
            .resolved_file_references
            .iter()
            .map(|reference| reference.materialise(&remap, string_table))
            .collect::<Result<Vec<_>, CompilerError>>()?;
        let resolution_facts = Arc::new(Stage0ResolutionFacts::frozen_generic(
            self.donor_file_id,
            resolved_file_references,
        )?);

        // Retain the captured donor identity as the explicit materialised owner. The donor range
        // stays distinct from the requester call-site source; cross-database rebinding waits for
        // the final `FrozenIdentityContext` migration, which is reported as the remaining
        // prerequisite and must never be emulated with a magic identity or `None`.
        Ok(MaterialisedBody {
            file_tokens: FileTokens::new_frozen(
                declaration_path,
                self.donor_file_id,
                None,
                tokens,
                path_syntax,
            ),
            resolution_facts,
            frozen_identity_handle: self.frozen_identity_handle.clone(),
        })
    }
}

pub(super) fn stable_path(path: &InternedPath, string_table: &StringTable) -> Box<[String]> {
    path.as_components()
        .iter()
        .map(|component| string_table.resolve(*component).to_owned())
        .collect()
}

pub(super) fn materialise_path(path: &[String], string_table: &mut StringTable) -> InternedPath {
    InternedPath::from_components(
        path.iter()
            .map(|component| string_table.intern(component))
            .collect(),
    )
}

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

fn pool_remap(id: StringId, remap: &[StringId]) -> Result<StringId, CompilerError> {
    let index = id.index() as usize;
    remap.get(index).copied().ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "frozen generic payload references out-of-range pool entry {index}"
        ))
    })
}
