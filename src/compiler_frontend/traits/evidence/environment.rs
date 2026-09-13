//! Trait evidence data types and lookup environment.
//!
//! WHAT: defines the core data structures for trait conformance evidence (`TraitEvidenceEnvironment`,
//!       `TraitEvidenceDefinition`, etc.) and manages indexing/rebuilding of these structures.
//! WHY: Trait evidence is frontend-only metadata used for conformance validation and static
//!      generic bounds checking. It is not runtime dispatch metadata or backend method-table state.

use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::traits::ids::{TraitEvidenceId, TraitId, TraitRequirementId};
use rustc_hash::FxHashMap;

/// The ownership class of an evidence record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TraitEvidenceKind {
    Canonical,
    Builtin,
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // Kept complete for frontend validation and static bound checks.
pub(crate) struct TraitRequirementEvidence {
    pub(crate) requirement_id: TraitRequirementId,
    pub(crate) method_path: PathId,
}

/// Resolved evidence for one accepted conformance declaration.
///
/// WHAT: records the frontend-selected receiver methods that satisfy one trait
/// conformance declaration.
/// WHY: conformance diagnostics and static generic bounds need stable evidence
/// facts without rescanning source declarations. This is frontend metadata only,
/// not HIR or backend runtime-dispatch state.
#[derive(Clone, Debug)]
#[allow(dead_code)] // Kept complete while generic-bound receiver calls use evidence paths.
pub(crate) struct TraitEvidenceDefinition {
    pub(crate) id: TraitEvidenceId,
    pub(crate) kind: TraitEvidenceKind,
    pub(crate) target_type_id: TypeId,
    pub(crate) trait_id: TraitId,
    pub(crate) source_file: PathId,
    pub(crate) declaration_span: Option<SourceSpan>,
    pub(crate) requirements: Vec<TraitRequirementEvidence>,
}

/// Indexed evidence facts for one module.
#[derive(Clone, Debug, Default)]
pub(crate) struct TraitEvidenceEnvironment {
    evidence: Vec<TraitEvidenceDefinition>,
    canonical_by_target_and_trait: FxHashMap<(TypeId, TraitId), TraitEvidenceId>,
    builtin_by_target_and_trait: FxHashMap<(TypeId, TraitId), TraitEvidenceId>,
}

impl TraitEvidenceEnvironment {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn get(&self, id: TraitEvidenceId) -> Option<&TraitEvidenceDefinition> {
        self.evidence.get(id.0 as usize)
    }

    #[allow(dead_code)] // Some compile modes validate traits without querying canonical evidence.
    pub(crate) fn canonical_for(
        &self,
        target_type_id: TypeId,
        trait_id: TraitId,
    ) -> Option<TraitEvidenceId> {
        self.canonical_by_target_and_trait
            .get(&(target_type_id, trait_id))
            .copied()
    }

    pub(crate) fn builtin_for(
        &self,
        target_type_id: TypeId,
        trait_id: TraitId,
    ) -> Option<TraitEvidenceId> {
        self.builtin_by_target_and_trait
            .get(&(target_type_id, trait_id))
            .copied()
    }

    /// Iterates over all compiler-owned builtin evidence records.
    ///
    /// WHAT: exposes the builtin evidence slice so validation can check for
    ///      incompatible trait pairs without needing to know the internal
    ///      `(TypeId, TraitId)` index shape.
    /// WHY: conflict detection needs to inspect builtin evidence rows for the
    ///      same target type; a simple iterator keeps the lookup local to the
    ///      evidence environment.
    pub(crate) fn builtins(&self) -> impl Iterator<Item = &TraitEvidenceDefinition> {
        self.evidence
            .iter()
            .filter(|definition| definition.kind == TraitEvidenceKind::Builtin)
    }

    /// Iterates over all source-authored canonical evidence records.
    ///
    /// WHAT: exposes the canonical evidence slice so the public-interface draft projection
    ///      can iterate source-owned conformance evidence. Direct module drafts intentionally
    ///      omit builtin evidence.
    /// WHY: the draft retains only evidence semantically owned by the declaring module.
    ///      Builtin evidence is compiler-global. The ownership vocabulary reserves a builtin
    ///      classification for a future compiler-owned handoff, so the projection iterates
    ///      canonical evidence only.
    pub(crate) fn canonical_evidence(&self) -> impl Iterator<Item = &TraitEvidenceDefinition> {
        self.evidence
            .iter()
            .filter(|definition| definition.kind == TraitEvidenceKind::Canonical)
    }

    pub(crate) fn insert_builtin(&mut self, mut definition: TraitEvidenceDefinition) {
        let id = TraitEvidenceId(self.evidence.len() as u32);
        definition.id = id;

        self.index_definition(&definition);
        self.evidence.push(definition);
    }

    pub(crate) fn insert_validated(&mut self, mut definition: TraitEvidenceDefinition) {
        let id = TraitEvidenceId(self.evidence.len() as u32);
        definition.id = id;

        self.index_definition(&definition);
        self.evidence.push(definition);
    }

    fn index_definition(&mut self, definition: &TraitEvidenceDefinition) {
        match definition.kind {
            TraitEvidenceKind::Canonical => {
                self.canonical_by_target_and_trait.insert(
                    (definition.target_type_id, definition.trait_id),
                    definition.id,
                );
            }

            TraitEvidenceKind::Builtin => {
                self.builtin_by_target_and_trait.insert(
                    (definition.target_type_id, definition.trait_id),
                    definition.id,
                );
            }
        }
    }
}
