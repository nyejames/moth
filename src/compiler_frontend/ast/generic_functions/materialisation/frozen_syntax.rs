//! Frozen generic body ownership and donor file-reference capture.
//!
//! Persistent generic syntax keeps the canonical declaring-source owner and checked body view.
//! Stage 0 file-reference facts cross the source-preparation lifetime independently. Generic
//! parsers consume checked ranges and sequences from the retained canonical owner.

use super::super::{GenericFunctionBody, MaterialisedDonorContext};
use super::frozen_file_references::StableResolvedFileReference;
use crate::compiler_frontend::ast::module_ast::scope_context::Stage0ResolutionFacts;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::headers::binding_environment::ResolvedQualifiedReference;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceId};
use crate::compiler_frontend::symbols::path_interner::{
    PathId, PathIdRemap, PathInternerFork, PathTable,
};
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
            strings: Arc::new(table.freeze_snapshot()),
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
/// `SourceTokens` allocation published with its prepared source. Source bodies share one
/// declaring-domain frozen string owner created once per `freeze()` or request capture; donor
/// bodies retain the exact identity tables that issued their payloads. File-reference rows remain
/// stable semantic facts for generated value resolution.
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
    /// Qualified-reference targets authored in this body, keyed by authored path row in source
    /// order.
    pub(super) qualified_references: Box<[(PathSyntaxId, ResolvedQualifiedReference)]>,
}
impl Clone for StableBodySyntax {
    fn clone(&self) -> Self {
        let source_owner = self.source_owner.clone();
        #[cfg(feature = "data_layout_memory_probe")]
        crate::compiler_frontend::instrumentation::record_generic_source_tokens(
            &source_owner.source_tokens,
        );
        Self {
            declaration_path: self.declaration_path,
            donor_file_id: self.donor_file_id,
            frozen_identity_handle: self.frozen_identity_handle.clone(),
            source_owner,
            token_range: self.token_range,
            token_sequence: self.token_sequence,
            source_path_table: self.source_path_table.clone(),
            source_string_table: self.source_string_table.clone(),
            resolved_file_references: self.resolved_file_references.clone(),
            qualified_references: self.qualified_references.clone(),
        }
    }
}

impl Drop for StableBodySyntax {
    fn drop(&mut self) {
        #[cfg(feature = "data_layout_memory_probe")]
        crate::compiler_frontend::instrumentation::release_generic_source_tokens(
            &self.source_owner.source_tokens,
        );
    }
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
    /// Qualified-reference targets keyed by retained donor source and authored path row.
    ///
    /// WHAT: one shared map for this body, cloned into generated value services and reinstalled
    ///       into the materialised binding environment.
    /// WHY: generated service forks must not copy the map per nested parse.
    pub(super) qualified_reference_targets:
        Arc<crate::compiler_frontend::headers::binding_environment::QualifiedReferenceTargets>,
}

impl MaterialisedBody {
    #[cfg(test)]
    pub(super) fn donor_file_id(&self) -> SourceId {
        self.source_owner.source_tokens.source()
    }

    pub(super) fn into_generic_body(self) -> Result<GenericFunctionBody, InfrastructureFailure> {
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

/// Metadata captured beside one generic body's syntax.
///
/// WHAT: qualified-reference targets, Stage 0 facts and the donor identity handle needed to
///       interpret the retained token owner.
/// WHY: they travel together through every capture call and keep the syntax-owning arguments
///      separate from the metadata that is not derived from the token range itself.
pub(super) struct StableBodyCaptureFacts<'a> {
    pub(super) qualified_references: &'a [(PathSyntaxId, ResolvedQualifiedReference)],
    pub(super) stage0_resolution_facts: Option<&'a Stage0ResolutionFacts>,
    pub(super) frozen_identity_handle: FrozenIdentityHandle,
}

impl StableBodySyntax {
    pub(super) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.declaration_path = remap.get(self.declaration_path);
        for (_, reference) in &mut self.qualified_references {
            reference.target.remap_path_ids(remap);
        }
    }

    pub(super) fn capture(
        body: &GenericFunctionBody,
        source_file: PathId,
        path_fork: &PathInternerFork,
        donor_identity: Option<&SharedDonorIdentity>,
        facts: StableBodyCaptureFacts<'_>,
        content_value_at_path: &impl Fn(
            &PathId,
        ) -> Result<
            crate::compiler_frontend::folded_value::PublicFoldedValue,
            InfrastructureFailure,
        >,
    ) -> Result<Self, InfrastructureFailure> {
        let StableBodyCaptureFacts {
            qualified_references,
            stage0_resolution_facts,
            frozen_identity_handle,
        } = facts;
        if !path_fork.starts_with(body.declaration_path(), source_file) {
            return Err(InfrastructureFailure::internal(
                CompilerStage::GenericMaterialisation,
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
                    InfrastructureFailure::internal(
                        CompilerStage::GenericMaterialisation,
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
                    // Qualified occurrences do not carry a file-reference row. Their resolved
                    // target is retained separately and consumed by expression lowering.
                    if qualified_references
                        .iter()
                        .any(|(path_syntax, _)| *path_syntax == source_path_id)
                    {
                        continue;
                    }
                    let facts = stage0_resolution_facts.ok_or_else(|| {
                        InfrastructureFailure::internal(CompilerStage::GenericMaterialisation,
                            "persistent generic body has path syntax but no Stage 0 resolution facts",
                        )
                    })?;
                    let resolved = facts
                        .lookup(donor_file_id, source_path_id)?
                        .ok_or_else(|| {
                            InfrastructureFailure::internal(CompilerStage::GenericMaterialisation, format!(
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

        #[cfg(feature = "data_layout_memory_probe")]
        {
            crate::compiler_frontend::instrumentation::record_generic_source_tokens(
                &source_owner.source_tokens,
            );
            crate::compiler_frontend::instrumentation::record_donor_identity_tables(
                source_path_table.as_deref(),
                source_string_table.as_deref(),
            );
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
            qualified_references: qualified_references.to_vec().into_boxed_slice(),
        })
    }

    pub(super) fn materialise(
        &self,
        source_file: PathId,
        path_fork: &mut PathInternerFork,
    ) -> Result<MaterialisedBody, InfrastructureFailure> {
        // Validate the retained canonical owner before handing its range and sequence to a parser
        // consumer.
        crate::compiler_frontend::ast::generic_functions::templates::validate_source_owner(
            &self.source_owner.source_tokens,
            self.token_range,
            self.token_sequence,
            "frozen generic body materialisation",
        )?;
        if !path_fork.starts_with(self.declaration_path, source_file) {
            return Err(InfrastructureFailure::internal(
                CompilerStage::GenericMaterialisation,
                "frozen generic body declaration path is outside its materialised source file",
            ));
        }

        let resolved_file_references = self
            .resolved_file_references
            .iter()
            .map(StableResolvedFileReference::materialise)
            .collect::<Result<Vec<_>, InfrastructureFailure>>()?;
        let qualified_reference_targets = Arc::new(
            self.qualified_references
                .iter()
                .map(|(path_syntax, resolved)| {
                    ((self.donor_file_id, *path_syntax), resolved.clone())
                })
                .collect(),
        );
        let resolution_facts = Arc::new(Stage0ResolutionFacts::frozen_generic(
            self.donor_file_id,
            resolved_file_references,
        )?);
        // The parser borrows the canonical donor range and installs the retained strings as
        // TokenPayloadOrigin, translating only payloads it consumes into requester tables. Those
        // strings are always the domain that issued the token payloads: a source body retains its
        // declaring module's snapshot, which can differ from the boundary-wide table installed on
        // the published context once earlier packages have merged into the shared boundary table.
        let source_path_table = self.source_path_table.clone();
        let source_string_table = self.source_string_table.clone();
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
            qualified_reference_targets,
        })
    }
}
