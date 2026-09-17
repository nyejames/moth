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
use crate::compiler_frontend::tokenizer::tokens::{
    FileTokens, SourceTokens, TokenKind, TokenRange, TokenSequenceId,
};
use std::path::PathBuf;
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
///      Path-crossing foreign materialised bodies keep their own donor pair and never use this
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
/// The owner/range pair is the only retained syntax payload. Source-origin and same-domain
/// materialised syntax share the canonical `SourceTokens` owner from prepared-source
/// publication; only path-crossing foreign materialised syntax keeps the donor `FileTokens`
/// shell (including its remapped compatibility lane). No additional parser adapter or copied
/// token vector is retained. Source bodies additionally share one declaring-domain frozen
/// string owner created once per `freeze()` or request capture; path-crossing foreign bodies
/// retain their exact donor path/string pair while string-only foreign donors canonicalize to
/// `SourceTokens` plus donor strings. File-reference rows remain stable semantic facts for
/// generated value resolution.
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
    /// strings for canonicalized string-only and path-crossing foreign bodies. `(Some, None)`
    /// is rejected at the materialisation boundary.
    pub(super) source_string_table: Option<Arc<FrozenStringTable>>,
    pub(super) resolved_file_references: Box<[StableResolvedFileReference]>,
}
/// Canonical owner retained by durable generic body syntax.
///
/// Source bodies share the declaring module's immutable `SourceTokens` allocation plus the
/// filesystem identity that `SourceTokens` itself does not store. Same-domain materialised
/// bodies share the donor canonical owner directly, as do canonicalized string-only foreign
/// donors. Only path-crossing foreign materialised bodies retain the donor `FileTokens` shell
/// so rebased compatibility payloads survive.
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
/// Canonical-vs-foreign owner carried by a materialised generic body.
#[derive(Clone, Debug)]
pub(super) enum MaterialisedBodyOwner {
    Canonical {
        source_tokens: Arc<SourceTokens>,
        canonical_os_path: Option<PathBuf>,
    },
    Foreign {
        source_owner: Arc<FileTokens>,
    },
}
impl StableBodySyntax {
    #[cfg(test)]
    pub(super) fn canonical_donor_file_id(&self) -> SourceId {
        self.donor_file_id
    }
}

/// Materialised body payload passed to generated AST construction.
pub(super) struct MaterialisedBody {
    pub(super) source_owner: MaterialisedBodyOwner,
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
        match &self.source_owner {
            MaterialisedBodyOwner::Canonical { source_tokens, .. } => source_tokens.source(),
            MaterialisedBodyOwner::Foreign { source_owner } => source_owner.file_id,
        }
    }

    #[cfg(test)]
    pub(super) fn canonical_owner(&self) -> Option<&Arc<SourceTokens>> {
        match &self.source_owner {
            MaterialisedBodyOwner::Canonical { source_tokens, .. } => Some(source_tokens),
            MaterialisedBodyOwner::Foreign { .. } => None,
        }
    }

    #[cfg(test)]
    pub(super) fn foreign_owner(&self) -> Option<&Arc<FileTokens>> {
        match &self.source_owner {
            MaterialisedBodyOwner::Canonical { .. } => None,
            MaterialisedBodyOwner::Foreign { source_owner } => Some(source_owner),
        }
    }

    pub(super) fn into_generic_body(self) -> Result<GenericFunctionBody, CompilerError> {
        match self.source_owner {
            MaterialisedBodyOwner::Canonical {
                source_tokens,
                canonical_os_path,
            } => GenericFunctionBody::materialised_canonical(
                source_tokens,
                canonical_os_path,
                self.token_range,
                self.token_sequence,
                self.declaration_path,
                MaterialisedDonorContext {
                    resolution_facts: self.resolution_facts,
                    frozen_identity_handle: self.frozen_identity_handle,
                    source_path_table: self.source_path_table,
                    source_string_table: self.source_string_table,
                },
            ),
            MaterialisedBodyOwner::Foreign { source_owner } => GenericFunctionBody::materialised(
                source_owner,
                self.token_range,
                self.token_sequence,
                self.declaration_path,
                MaterialisedDonorContext {
                    resolution_facts: self.resolution_facts,
                    frozen_identity_handle: self.frozen_identity_handle,
                    source_path_table: self.source_path_table,
                    source_string_table: self.source_string_table,
                },
            ),
        }
    }
}

impl std::fmt::Debug for MaterialisedBody {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let donor_file_id = match &self.source_owner {
            MaterialisedBodyOwner::Canonical { source_tokens, .. } => source_tokens.source(),
            MaterialisedBodyOwner::Foreign { source_owner } => source_owner.file_id,
        };
        formatter
            .debug_struct("MaterialisedBody")
            .field("donor_file_id", &donor_file_id)
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
        let source_owner = match body {
            GenericFunctionBody::Source {
                source_owner,
                canonical_os_path,
                ..
            }
            | GenericFunctionBody::MaterialisedCanonical {
                source_owner,
                canonical_os_path,
                ..
            } => StableBodyOwner::Source {
                source_tokens: Arc::clone(source_owner),
                canonical_os_path: canonical_os_path.clone(),
            },
            GenericFunctionBody::MaterialisedForeign {
                source_owner,
                source_path_table: None,
                ..
            } => StableBodyOwner::Source {
                source_tokens: source_owner.canonical_source_tokens_arc().map_err(|_| {
                    CompilerError::compiler_error(
                        "frozen generic body capture needs a canonical donor owner",
                    )
                })?,
                canonical_os_path: source_owner.canonical_os_path.clone(),
            },
            GenericFunctionBody::MaterialisedForeign { source_owner, .. } => {
                StableBodyOwner::Materialised {
                    source_owner: Arc::clone(source_owner),
                }
            }
        };
        // Source and same-domain bodies share the declaring preparation's cached donor owner.
        // The cache freezes once per declaring preparation; every body in the domain clones
        // the same `Arc` instead of cloning and freezing the live table per body.
        // Only path-crossing foreign bodies keep their exact retained donor pair. A string-only
        // foreign donor canonicalizes through its `SourceTokens` owner plus donor strings, so a
        // retained string table without a path table is canonical metadata. A path table
        // without its issuing string table is rejected.
        let (source_path_table, source_string_table) = match body {
            GenericFunctionBody::Source { .. } => {
                let donor_identity = donor_identity.ok_or_else(|| {
                    CompilerError::compiler_error(
                        "frozen generic source body has no declaring-domain donor identity",
                    )
                })?;
                (None, Some(Arc::clone(donor_identity.strings())))
            }
            GenericFunctionBody::MaterialisedCanonical {
                source_path_table,
                source_string_table,
                ..
            } => match (source_path_table, source_string_table) {
                (Some(path_table), Some(source_strings)) => (
                    Some(Arc::clone(path_table)),
                    Some(Arc::clone(source_strings)),
                ),
                (None, Some(source_strings)) => (None, Some(Arc::clone(source_strings))),
                (None, None) => {
                    let donor_identity = donor_identity.ok_or_else(|| {
                        CompilerError::compiler_error(
                            "frozen generic source body has no declaring-domain donor identity",
                        )
                    })?;
                    (None, Some(Arc::clone(donor_identity.strings())))
                }
                (Some(_), None) => {
                    return Err(CompilerError::compiler_error(
                        "frozen generic body has an incomplete source identity table pair",
                    ));
                }
            },
            GenericFunctionBody::MaterialisedForeign {
                source_path_table,
                source_string_table,
                ..
            } => match (source_path_table, source_string_table) {
                (Some(path_table), Some(source_strings)) => (
                    Some(Arc::clone(path_table)),
                    Some(Arc::clone(source_strings)),
                ),
                (None, Some(source_strings)) => (None, Some(Arc::clone(source_strings))),
                (None, None) => {
                    let donor_identity = donor_identity.ok_or_else(|| {
                        CompilerError::compiler_error(
                            "frozen generic source body has no declaring-domain donor identity",
                        )
                    })?;
                    (None, Some(Arc::clone(donor_identity.strings())))
                }
                (Some(_), None) => {
                    return Err(CompilerError::compiler_error(
                        "frozen generic body has an incomplete source identity table pair",
                    ));
                }
            },
        };
        let token_stream = body.parser_stream_for_capture()?;
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
        // Source-origin syntax validates directly against `SourceTokens`; foreign
        // Materialised-origin syntax keeps the `FileTokens`-shell validator so rebased
        // compatibility payloads stay valid.
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
        let source_owner_is_canonical =
            matches!(&self.source_owner, StableBodyOwner::Source { .. });
        let (source_path_table, source_string_table) =
            match (&self.source_path_table, &self.source_string_table) {
                (Some(path_table), Some(source_strings)) => (
                    Some(Arc::clone(path_table)),
                    Some(Arc::clone(source_strings)),
                ),
                (None, Some(source_strings)) if source_owner_is_canonical => identity_tables
                    .map(|(path_table, source_strings)| {
                        (
                            Some(Arc::clone(path_table)),
                            Some(Arc::clone(source_strings)),
                        )
                    })
                    .unwrap_or((None, Some(Arc::clone(source_strings)))),
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
        // Durable canonical syntax always shares the canonical owner directly and never retains a
        // `FileTokens` shell. A retained path/string pair or string-only donor is checked donor
        // identity metadata only; parser consumers derive a transient remapped adapter from it
        // when the requester table differs. Only path-crossing foreign donors keep the donor's
        // remapped compatibility shell.
        let source_owner = match &self.source_owner {
            StableBodyOwner::Source {
                source_tokens,
                canonical_os_path,
            } => MaterialisedBodyOwner::Canonical {
                source_tokens: Arc::clone(source_tokens),
                canonical_os_path: canonical_os_path.clone(),
            },
            StableBodyOwner::Materialised { source_owner } if source_path_table.is_some() => {
                MaterialisedBodyOwner::Foreign {
                    source_owner: Arc::clone(source_owner),
                }
            }
            StableBodyOwner::Materialised { source_owner } => {
                let canonical_owner = source_owner.canonical_source_tokens_arc().map_err(|_| {
                    CompilerError::compiler_error(
                        "frozen generic body materialisation needs a canonical donor owner",
                    )
                })?;
                MaterialisedBodyOwner::Canonical {
                    source_tokens: canonical_owner,
                    canonical_os_path: source_owner.canonical_os_path.clone(),
                }
            }
        };
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
