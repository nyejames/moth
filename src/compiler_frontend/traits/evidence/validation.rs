//! Trait conformance validation orchestration.
//!
//! WHAT: Orchestrates validation of all `Type must TRAIT` conformance headers across files,
//!       detecting duplicate declarations, incompatible trait pairs, and checking method
//!       compatibility.
//! WHY: Fuses syntactic headers, resolved traits, visible trait-incompatibility metadata,
//!      method catalogs and dependency-binding rules into a consistent, valid `TraitEvidenceEnvironment`.

use super::diagnostics::{invalid_conformance, previous_declaration_label};
use super::environment::{TraitEvidenceDefinition, TraitEvidenceEnvironment};
use super::requirement_matching::{RequirementValidationContext, validate_requirements};
use super::target_resolution::{
    ConformanceTarget, ResolveConformanceTargetContext, resolve_conformance_target,
    resolve_trait_reference,
};
use crate::compiler_frontend::ast::ReceiverMethodCatalog;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTraitConformanceReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::headers::binding_environment::HeaderBindingEnvironment;
use crate::compiler_frontend::headers::parse_file_headers::{FileRole, Header, HeaderKind};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::ids::{TraitEvidenceId, TraitId};
use rustc_hash::FxHashMap;

/// Boxed-diagnostic result for trait-evidence conformance validation.
///
/// `CompilerDiagnostic` is large enough to trigger `clippy::result_large_err` when
/// returned directly in `Result`. Boxing the error variant keeps the validation
/// family's `Result` enum small; callers unbox at existing plain-diagnostic
/// accumulation boundaries.
type TraitEvidenceValidationResult<T = ()> = Result<T, Box<CompilerDiagnostic>>;

/// Inputs needed to validate evidence after trait definitions and receiver methods exist.
pub(crate) struct ValidateTraitEvidenceInput<'a> {
    pub(crate) sorted_headers: &'a [Header],
    pub(crate) trait_environment: &'a TraitEnvironment,
    pub(crate) receiver_methods: &'a ReceiverMethodCatalog,
    pub(crate) type_environment: &'a TypeEnvironment,
    pub(crate) binding_environment: &'a HeaderBindingEnvironment,
    pub(crate) nominal_type_ids_by_path: &'a FxHashMap<InternedPath, TypeId>,
    pub(crate) struct_source_by_path: &'a FxHashMap<InternedPath, InternedPath>,
    pub(crate) choice_source_by_path: &'a FxHashMap<InternedPath, InternedPath>,
    pub(crate) string_table: &'a mut StringTable,
}

struct PendingConformanceEvidence {
    target: ConformanceTarget,
    target_name: StringId,
    trait_id: TraitId,
    trait_name: StringId,
    source_file: InternedPath,
    declaration_location: SourceLocation,
    trait_location: SourceLocation,
}

struct IncompatibleEvidence {
    trait_name: StringId,
    declaration_location: SourceLocation,
}

/// Validate explicit conformance declarations and store indexed evidence facts.
///
/// WHAT: validates same-file nominal conformance evidence before matching trait requirements
/// against receiver methods in the declaring file. The caller is responsible for registering
/// compiler-owned builtin evidence first so that user attempts to override builtin evidence
/// and user evidence that conflicts with builtin evidence for an incompatible trait are both
/// rejected here.
/// WHY: later dispatch phases need stable evidence indexes and must not rediscover conformance
/// headers or infer structural conformance from arbitrary matching methods.
pub(crate) fn validate_trait_evidence(
    input: ValidateTraitEvidenceInput<'_>,
    evidence_environment: &mut TraitEvidenceEnvironment,
) -> TraitEvidenceValidationResult {
    let mut pending_evidence = Vec::new();
    let mut pending_canonical_locations: FxHashMap<(TypeId, TraitId), SourceLocation> =
        FxHashMap::default();

    for header in input.sorted_headers {
        let HeaderKind::TraitConformance { conformance } = &header.kind else {
            continue;
        };

        if header.file_role == FileRole::ImportedModuleRoot {
            return Err(invalid_conformance(
                conformance.target.name,
                conformance.traits.first().map(|trait_ref| trait_ref.name),
                InvalidTraitConformanceReason::ImportedModuleRoot,
                conformance.location.clone(),
                Vec::new(),
            )
            .into());
        }

        let visibility = input
            .binding_environment
            .visibility_for(&header.source_file)
            .map_err(|_| {
                invalid_conformance(
                    conformance.target.name,
                    conformance.traits.first().map(|trait_ref| trait_ref.name),
                    InvalidTraitConformanceReason::NonCanonicalTarget,
                    conformance.location.clone(),
                    Vec::new(),
                )
            })?;
        let conformance_source_file = header.canonical_source_file(input.string_table);

        let target_context = ResolveConformanceTargetContext {
            conformance_source_file: &conformance_source_file,
            visibility,
            nominal_type_ids_by_path: input.nominal_type_ids_by_path,
            struct_source_by_path: input.struct_source_by_path,
            choice_source_by_path: input.choice_source_by_path,
            type_environment: input.type_environment,
            string_table: input.string_table,
        };
        let target = resolve_conformance_target(&conformance.target, target_context)?;

        for trait_ref in &conformance.traits {
            let trait_id = resolve_trait_reference(
                trait_ref,
                visibility,
                input.trait_environment,
                input.string_table,
            )?;

            if let Some(existing_id) = evidence_environment.builtin_for(target.type_id, trait_id) {
                let previous_location = evidence_environment
                    .get(existing_id)
                    .map(|definition| definition.declaration_location.clone());

                return Err(invalid_conformance(
                    conformance.target.name,
                    Some(trait_ref.name),
                    InvalidTraitConformanceReason::BuiltinEvidenceOverride,
                    trait_ref.location.clone(),
                    previous_declaration_label(previous_location),
                )
                .into());
            }

            let key = (target.type_id, trait_id);
            if let Some(previous_location) = pending_canonical_locations.get(&key) {
                return Err(invalid_conformance(
                    conformance.target.name,
                    Some(trait_ref.name),
                    InvalidTraitConformanceReason::DuplicateCanonicalEvidence,
                    trait_ref.location.clone(),
                    previous_declaration_label(Some(previous_location.clone())),
                )
                .into());
            }

            if let Some(incompatible) = find_incompatible_evidence(
                target.type_id,
                trait_id,
                evidence_environment,
                &pending_evidence,
                input.trait_environment,
            ) {
                let secondary_labels = previous_declaration_label(non_default_location(
                    incompatible.declaration_location,
                ));

                return Err(invalid_conformance(
                    conformance.target.name,
                    Some(trait_ref.name),
                    InvalidTraitConformanceReason::IncompatibleTraitEvidence {
                        incompatible_trait_name: incompatible.trait_name,
                    },
                    trait_ref.location.clone(),
                    secondary_labels,
                )
                .into());
            }

            pending_canonical_locations.insert(key, conformance.location.clone());

            pending_evidence.push(PendingConformanceEvidence {
                target: target.clone(),
                target_name: conformance.target.name,
                trait_id,
                trait_name: trait_ref.name,
                source_file: conformance_source_file.clone(),
                declaration_location: conformance.location.clone(),
                trait_location: trait_ref.location.clone(),
            });
        }
    }

    for pending in pending_evidence {
        let Some(trait_definition) = input.trait_environment.get(pending.trait_id) else {
            return Err(CompilerDiagnostic::unknown_trait_name(
                pending.trait_name,
                pending.trait_location.clone(),
            )
            .into());
        };

        let mut requirement_context = RequirementValidationContext {
            receiver_methods: input.receiver_methods,
            type_environment: input.type_environment,
            target_name: pending.target_name,
            trait_name: pending.trait_name,
            conformance_location: pending.declaration_location.clone(),
            string_table: input.string_table,
        };
        let requirement_methods = validate_requirements(
            trait_definition,
            &pending.target,
            &pending.source_file,
            &mut requirement_context,
        )?;

        let evidence = TraitEvidenceDefinition {
            id: TraitEvidenceId(0),
            kind: pending.target.evidence_kind,
            target_type_id: pending.target.type_id,
            trait_id: pending.trait_id,
            source_file: pending.source_file,
            declaration_location: pending.declaration_location,
            requirements: requirement_methods,
        };

        evidence_environment.insert_validated(evidence);
    }

    Ok(())
}

fn find_incompatible_evidence(
    target_type_id: TypeId,
    trait_id: TraitId,
    evidence_environment: &TraitEvidenceEnvironment,
    pending_evidence: &[PendingConformanceEvidence],
    trait_environment: &TraitEnvironment,
) -> Option<IncompatibleEvidence> {
    for pending in pending_evidence {
        if pending.target.type_id != target_type_id {
            continue;
        }
        if !trait_environment.traits_are_incompatible(pending.trait_id, trait_id) {
            continue;
        }

        return Some(IncompatibleEvidence {
            trait_name: pending.trait_name,
            declaration_location: pending.declaration_location.clone(),
        });
    }

    for definition in evidence_environment.builtins() {
        if definition.target_type_id != target_type_id {
            continue;
        }
        if !trait_environment.traits_are_incompatible(definition.trait_id, trait_id) {
            continue;
        }

        let trait_name = trait_environment
            .get(definition.trait_id)
            .map(|definition| definition.name)?;

        return Some(IncompatibleEvidence {
            trait_name,
            declaration_location: definition.declaration_location.clone(),
        });
    }

    None
}

fn non_default_location(location: SourceLocation) -> Option<SourceLocation> {
    if location == SourceLocation::default() {
        None
    } else {
        Some(location)
    }
}
