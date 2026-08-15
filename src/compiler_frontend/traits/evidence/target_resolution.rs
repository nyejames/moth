//! Conformance target and trait reference resolution.
//!
//! WHAT: Resolves the semantic target type (struct, choice, builtin scalar, or external type)
//!       and the trait reference for an explicit `Type must TRAIT` conformance declaration.
//! WHY: Translates syntax-stage name spellings and visibility contexts into compile-time types
//!      and trait definition IDs so matching/validation can proceed.

use super::diagnostics::invalid_conformance;
use super::environment::TraitEvidenceKind;

use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DeferredFeatureReason, InvalidTraitConformanceReason,
};
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::binding_environment::FileVisibility;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::ids::TraitId;
use crate::compiler_frontend::traits::syntax::{
    ConformanceTargetKind, ConformanceTargetSyntax, TraitReferenceSyntax,
};
use rustc_hash::FxHashMap;

/// Boxed diagnostics for the connected conformance-target resolution family.
///
/// Target and trait lookup feed directly into the already boxed evidence
/// validation boundary. Keeping the result family boxed avoids large local
/// `Result` values without changing diagnostic construction or propagation.
type TargetResolutionResult<T> = Result<T, Box<CompilerDiagnostic>>;

#[derive(Clone)]
pub(super) struct ConformanceTarget {
    pub(super) type_id: TypeId,
    pub(super) receiver_key: ReceiverKey,
    pub(super) path: Option<InternedPath>,
    pub(super) is_generic_constructor: bool,
    pub(super) evidence_kind: TraitEvidenceKind,
}

pub(super) struct ResolveConformanceTargetContext<'a> {
    pub(super) conformance_source_file: &'a InternedPath,
    pub(super) visibility: &'a FileVisibility,
    pub(super) nominal_type_ids_by_path: &'a FxHashMap<InternedPath, TypeId>,
    pub(super) struct_source_by_path: &'a FxHashMap<InternedPath, InternedPath>,
    pub(super) choice_source_by_path: &'a FxHashMap<InternedPath, InternedPath>,
    pub(super) type_environment: &'a TypeEnvironment,
    pub(super) string_table: &'a StringTable,
}

pub(super) fn resolve_conformance_target(
    target: &ConformanceTargetSyntax,
    context: ResolveConformanceTargetContext<'_>,
) -> TargetResolutionResult<ConformanceTarget> {
    if target.kind == ConformanceTargetKind::SpecializedGenericInstance {
        return Err(CompilerDiagnostic::deferred_feature_reason(
            DeferredFeatureReason::NamedFeature {
                feature: target.name,
            },
            target.location.clone(),
        )
        .into());
    }

    if is_builtin_scalar_target(target.name, context.string_table) {
        return Err(invalid_conformance(
            target.name,
            None,
            InvalidTraitConformanceReason::BuiltinTarget,
            target.location.clone(),
            Vec::new(),
        )
        .into());
    }

    if let Some(symbol_id) = context
        .visibility
        .visible_external_symbols
        .get(&target.name)
        && let ExternalSymbolId::Type(_) = symbol_id
    {
        return Err(invalid_conformance(
            target.name,
            None,
            InvalidTraitConformanceReason::ExternalOpaqueTarget,
            target.location.clone(),
            Vec::new(),
        )
        .into());
    }

    if context
        .visibility
        .visible_type_alias_names
        .contains_key(&target.name)
    {
        return Err(invalid_conformance(
            target.name,
            None,
            InvalidTraitConformanceReason::AliasTarget,
            target.location.clone(),
            Vec::new(),
        )
        .into());
    }

    let Some(target_path) = context.visibility.visible_source_names.get(&target.name) else {
        return Err(
            CompilerDiagnostic::unknown_type_name(target.name, target.location.clone()).into(),
        );
    };
    let Some(type_id) = context.nominal_type_ids_by_path.get(target_path).copied() else {
        return Err(invalid_conformance(
            target.name,
            None,
            InvalidTraitConformanceReason::NonCanonicalTarget,
            target.location.clone(),
            Vec::new(),
        )
        .into());
    };

    let Some(definition) = context.type_environment.get(type_id) else {
        return Err(invalid_conformance(
            target.name,
            None,
            InvalidTraitConformanceReason::NonCanonicalTarget,
            target.location.clone(),
            Vec::new(),
        )
        .into());
    };

    match definition {
        TypeDefinition::Struct(definition) => {
            let target_is_declared_here = context
                .struct_source_by_path
                .get(&definition.path)
                .is_some_and(|source_file| source_file == context.conformance_source_file);
            if !target_is_declared_here {
                return Err(invalid_conformance(
                    target.name,
                    None,
                    InvalidTraitConformanceReason::NonlocalSourceTarget,
                    target.location.clone(),
                    Vec::new(),
                )
                .into());
            }

            Ok(ConformanceTarget {
                type_id,
                receiver_key: ReceiverKey::Struct(definition.path.clone()),
                path: Some(definition.path.clone()),
                is_generic_constructor: definition.generic_parameters.is_some(),
                evidence_kind: TraitEvidenceKind::Canonical,
            })
        }

        TypeDefinition::Choice(definition) => {
            let target_is_declared_here = context
                .choice_source_by_path
                .get(&definition.path)
                .is_some_and(|source_file| source_file == context.conformance_source_file);
            if !target_is_declared_here {
                return Err(invalid_conformance(
                    target.name,
                    None,
                    InvalidTraitConformanceReason::NonlocalSourceTarget,
                    target.location.clone(),
                    Vec::new(),
                )
                .into());
            }

            Ok(ConformanceTarget {
                type_id,
                receiver_key: ReceiverKey::Choice(definition.path.clone()),
                path: Some(definition.path.clone()),
                is_generic_constructor: definition.generic_parameters.is_some(),
                evidence_kind: TraitEvidenceKind::Canonical,
            })
        }

        _ => Err(invalid_conformance(
            target.name,
            None,
            InvalidTraitConformanceReason::NonCanonicalTarget,
            target.location.clone(),
            Vec::new(),
        )
        .into()),
    }
}

fn is_builtin_scalar_target(name: StringId, string_table: &StringTable) -> bool {
    matches!(
        string_table.resolve(name),
        "Int" | "Float" | "Bool" | "String" | "Char"
    )
}

pub(super) fn resolve_trait_reference(
    trait_ref: &TraitReferenceSyntax,
    visibility: &FileVisibility,
    trait_environment: &TraitEnvironment,
    string_table: &mut StringTable,
) -> TargetResolutionResult<TraitId> {
    if let Some(path) = visibility.visible_trait_names.get(&trait_ref.name)
        && let Some(id) = trait_environment.id_for_path(path)
    {
        return Ok(id);
    }

    if let Some(id) = trait_environment.core_trait_id_for_name(trait_ref.name, string_table) {
        return Ok(id);
    }

    Err(CompilerDiagnostic::unknown_trait_name(trait_ref.name, trait_ref.location.clone()).into())
}
