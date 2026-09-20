//! Frozen generic body ownership and donor file-reference capture.
//!
//! Persistent generic syntax keeps the canonical declaring-source owner and checked body view.
//! Compatibility parser adapters are materialised only for the current operation, while stable
//! Stage 0 file-reference facts cross the source-preparation lifetime independently.

use super::super::{GenericFunctionBody, MaterialisedDonorContext};
use super::frozen_file_references::StableResolvedFileReference;
use crate::compiler_frontend::ast::module_ast::scope_context::Stage0ResolutionFacts;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceId};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork, PathTable};
use crate::compiler_frontend::symbols::string_interning::{FrozenStringTable, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{SourceTokens, TokenRange, TokenSequenceId};
use std::sync::Arc;
/// Shared donor string identity for one declaring source/module domain.
///
/// WHAT: owns one frozen snapshot of the declaring domain's string table behind a shared
///       owner. Every source-body capture in the domain clones this `Arc` instead of cloning
///       and freezing the live table per body.
/// WHY: retained token payloads interpret `StringId`s through the exact table that issued
///      them. Freezing once per domain preserves that domain while keeping retained storage
///      proportional to the domain, not to template count. The shared owner is drop-safe:
///      retained bodies keep the allocation alive after the live preparation tables drop.
///      A body that retained its own donor identity pair keeps that pair and never uses this
///      owner.
#[derive(Clone, Debug)]
pub(crate) struct SharedDonorIdentity {
    strings: Arc<FrozenStringTable>,
}

impl SharedDonorIdentity {
    pub(crate) fn freeze(table: &StringTable) -> Self {
        Self {
            strings: Arc::new(table.clone().freeze()),
        }
    }

    /// Borrow the shared frozen strings issued by the declaring domain.
    pub(crate) fn strings(&self) -> &Arc<FrozenStringTable> {
        &self.strings
    }
}
/// Durable generic body syntax.
///
/// The owner/range pair is the only retained syntax payload: every lane shares the canonical
/// `SourceTokens` allocation published with its prepared source. No parser adapter or copied
/// token vector is retained. Source bodies share one declaring-domain frozen string owner
/// created once per `freeze()` or request capture; donor bodies retain the exact identity tables
/// that issued their payloads. File-reference rows remain stable semantic facts for generated
/// value resolution.
#[derive(Clone)]
pub(super) struct StableBodySyntax {
    pub(super) declaration_path: PathId,
    pub(super) donor_file_id: SourceId,
    pub(super) frozen_identity_handle: FrozenIdentityHandle,
    pub(super) source_owner: StableBodyOwner,
    pub(super) token_range: TokenRange,
    pub(super) token_sequence: Option<TokenSequenceId>,
    pub(super) source_path_table: Option<Arc<PathTable>>,
    /// Shared declaring-domain donor strings for source bodies; the exact retained donor
    /// strings for a donor body. `(Some, None)` is rejected at the materialisation boundary.
    pub(super) source_string_table: Option<Arc<FrozenStringTable>>,
    pub(super) resolved_file_references: Box<[StableResolvedFileReference]>,
}

/// Canonical owner retained by durable generic body syntax.
///
/// Every retained body shares its declaring module's immutable `SourceTokens` allocation.
#[derive(Clone, Debug)]
pub(super) struct StableBodyOwner {
    pub(super) source_tokens: Arc<SourceTokens>,
}
impl StableBodySyntax {
    #[cfg(test)]
    pub(super) fn canonical_donor_file_id(&self) -> SourceId {
        self.donor_file_id
    }
}

/// Materialised body payload passed to generated AST construction.
pub(super) struct MaterialisedBody {
    pub(super) source_owner: StableBodyOwner,
    pub(super) token_range: TokenRange,
    pub(super) token_sequence: Option<TokenSequenceId>,
    pub(super) declaration_path: PathId,
    pub(super) resolution_facts: Arc<Stage0ResolutionFacts>,
    pub(super) frozen_identity_handle: FrozenIdentityHandle,
    pub(super) source_path_table: Option<Arc<PathTable>>,
    pub(super) source_string_table: Option<Arc<FrozenStringTable>>,
}

impl MaterialisedBody {
    #[cfg(test)]
    pub(super) fn donor_file_id(&self) -> SourceId {
        self.source_owner.source_tokens.source()
    }

    pub(super) fn into_generic_body(self) -> Result<GenericFunctionBody, CompilerError> {
        GenericFunctionBody::materialised(
            self.source_owner.source_tokens,
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
            .field("donor_file_id", &self.source_owner.source_tokens.source())
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
        donor_identity: Option<&SharedDonorIdentity>,
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
        let (canonical_owner, token_range, token_sequence) = body.canonical_view();
        let source_owner = StableBodyOwner {
            source_tokens: Arc::clone(canonical_owner),
        };
        // Source and same-domain bodies share the declaring preparation's cached donor owner.
        // The cache freezes once per declaring preparation; every body in the domain clones the
        // same `Arc` instead of cloning and freezing the live table per body. Only a body that
        // retained its own donor tables keeps that exact pair. A retained string table without a
        // path table is canonical metadata; the reverse is rejected by the body constructor.
        let declaring_domain_strings = || {
            donor_identity
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "frozen generic source body has no declaring-domain donor identity",
                    )
                })
                .map(|identity| Arc::clone(identity.strings()))
        };
        let (source_path_table, source_string_table) = match body {
            GenericFunctionBody::Source { .. } => (None, Some(declaring_domain_strings()?)),
            GenericFunctionBody::Materialised {
                source_path_table,
                source_string_table,
                ..
            } => (
                source_path_table.clone(),
                match source_string_table {
                    Some(donor_strings) => Some(Arc::clone(donor_strings)),
                    None => Some(declaring_domain_strings()?),
                },
            ),
        };
        let path_syntax = source_owner.source_tokens.path_syntax_table()?;
        path_syntax.validate_structure()?;
        let mut cursor = crate::compiler_frontend::ast::generic_functions::templates::body_cursor(
            &source_owner.source_tokens,
            token_range,
            token_sequence,
            "generic body capture",
        )?;
        let mut seen_paths = rustc_hash::FxHashSet::default();
        let mut resolved_file_references = Vec::new();
        while let Some(token) = cursor.advance() {
            let is_eof = token.is_eof();
            if let Some(source_path_id) = token.path_syntax_id() {
                path_syntax.try_path_for_token(source_path_id, token.source_span())?;
                if seen_paths.insert(source_path_id) {
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
            }
            if is_eof {
                break;
            }
        }

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
        crate::compiler_frontend::ast::generic_functions::templates::validate_source_owner(
            &self.source_owner.source_tokens,
            self.token_range,
            self.token_sequence,
            "frozen generic body materialisation",
        )?;
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
        // A retained path/string pair or string-only donor is carried as independent identity
        // metadata. The parser borrows the canonical donor range and installs these tables as
        // TokenPayloadOrigin, translating only payloads it consumes into requester tables.
        // Without a retained path table, the caller's installed identity pair supplies path
        // provenance for same-domain materialisation.
        let (source_path_table, source_string_table) = match &self.source_path_table {
            Some(path_table) => (
                Some(Arc::clone(path_table)),
                self.source_string_table.clone(),
            ),
            None => identity_tables
                .map(|(path_table, source_strings)| {
                    (
                        Some(Arc::clone(path_table)),
                        Some(Arc::clone(source_strings)),
                    )
                })
                .unwrap_or_else(|| (None, self.source_string_table.clone())),
        };
        let source_owner = self.source_owner.clone();
        Ok(MaterialisedBody {
            source_owner,
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
