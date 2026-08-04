//! Imported provider trait, generic-bound and reusable-evidence projection.
//!
//! WHAT: reconstructs consumer-local `TraitId`, generic-bound and `TraitEvidenceId` handles from
//! stable completed provider-interface records.
//! WHY: AST trait checks operate on dense local environments. This join lets them consume
//! immutable provider semantics without donor headers, donor type IDs or structural conformance
//! reconstruction.

use super::*;
use crate::compiler_frontend::canonical_type_identity::CanonicalTraitIdentity;
use crate::compiler_frontend::headers::import_environment::SourceFunctionTarget;
use crate::compiler_frontend::public_interface::{
    PublicTraitReceiverAccess, TraitSurfaceTypeIdentity,
};
use crate::compiler_frontend::semantic_identity::{OriginFunctionId, OriginTraitId};
use crate::compiler_frontend::traits::definitions::{
    ResolvedTraitDefinition, ResolvedTraitParameter, ResolvedTraitRequirement, ResolvedTraitReturn,
    TraitReceiverRequirement, TraitVisibility,
};
use crate::compiler_frontend::traits::evidence::environment::{
    TraitEvidenceDefinition, TraitEvidenceKind, TraitRequirementEvidence,
};
use crate::compiler_frontend::traits::ids::{TraitEvidenceId, TraitRequirementId};

impl<'context, 'services> AstModuleEnvironmentBuilder<'context, 'services> {
    /// Projects every stable source trait in the retained provider closure before local trait
    /// headers resolve references to those imports.
    pub(in crate::compiler_frontend::ast) fn project_imported_trait_declarations(
        &mut self,
        trait_environment: &mut TraitEnvironment,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerError> {
        let mut imported_traits = self
            .import_environment
            .imported_declarations_by_origin
            .iter()
            .filter_map(|(origin, record)| {
                let OriginDeclarationId::Trait(trait_origin) = origin else {
                    return None;
                };
                let PublicDeclarationSemantics::Trait(semantics) = &record.semantics else {
                    return None;
                };

                Some((trait_origin.clone(), semantics.clone()))
            })
            .collect::<Vec<_>>();
        imported_traits.sort_by(|left, right| left.0.cmp(&right.0));

        for (trait_origin, semantics) in &imported_traits {
            let trait_path = imported_trait_path(trait_origin, string_table);
            let trait_name = string_table.intern(trait_origin.defining_name());
            let this_name = string_table.intern("This");
            let this_type = self
                .type_environment
                .register_synthetic_generic_parameter(this_name);
            let mut next_requirement_id = trait_environment.next_requirement_id();
            let mut requirements = Vec::with_capacity(semantics.requirements.len());

            for requirement in &semantics.requirements {
                requirements.push(self.project_imported_trait_requirement(
                    &trait_path,
                    this_type,
                    next_requirement_id,
                    requirement,
                    string_table,
                )?);
                next_requirement_id.0 += 1;
            }

            let trait_id = trait_environment.next_trait_id();
            let definition = ResolvedTraitDefinition {
                id: trait_id,
                name: trait_name,
                canonical_path: trait_path,
                source_file: InternedPath::new(),
                this_type,
                requirements,
                declaration_location: Default::default(),
                visibility: TraitVisibility::Source { exported: true },
            };
            if trait_environment.insert(definition).is_some() {
                return Err(CompilerError::compiler_error(
                    "Two imported trait origins produced the same consumer-local trait path.",
                ));
            }
            trait_environment.register_canonical_identity(
                CanonicalTraitIdentity::Source(trait_origin.clone()),
                trait_id,
            )?;
        }

        // All traits have dense IDs before incompatibility edges are joined.
        for (trait_origin, semantics) in &imported_traits {
            let trait_identity = CanonicalTraitIdentity::Source(trait_origin.clone());
            let trait_id = trait_environment
                .id_for_canonical_identity(&trait_identity)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported trait projection omitted its canonical local identity.",
                    )
                })?;

            for incompatible_identity in &semantics.incompatibilities {
                let incompatible_id = trait_environment
                    .id_for_canonical_identity(incompatible_identity)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Imported trait incompatibility names a trait absent from the provider closure.",
                        )
                    })?;
                trait_environment.record_incompatible_traits(trait_id, incompatible_id);
            }
        }

        // Direct grouped imports and aliases are alternate paths to the same imported trait.
        let mut visible_paths = self
            .import_environment
            .imported_declarations_by_local_path
            .iter()
            .filter_map(|(path, origin)| {
                let OriginDeclarationId::Trait(origin) = origin else {
                    return None;
                };
                Some((path.clone(), origin.clone()))
            })
            .collect::<Vec<_>>();
        visible_paths.sort_by_key(|(path, _)| path.to_string(string_table));

        for (path, origin) in visible_paths {
            let identity = CanonicalTraitIdentity::Source(origin);
            let trait_id = trait_environment
                .id_for_canonical_identity(&identity)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "A visible imported trait has no projected canonical definition.",
                    )
                })?;
            trait_environment.register_path(path, trait_id)?;
        }

        Ok(())
    }

    fn project_imported_trait_requirement(
        &mut self,
        trait_path: &InternedPath,
        this_type: TypeId,
        requirement_id: TraitRequirementId,
        requirement: &crate::compiler_frontend::public_interface::PublicTraitRequirementSurface,
        string_table: &mut StringTable,
    ) -> Result<ResolvedTraitRequirement, CompilerError> {
        let requirement_name = string_table.intern(&requirement.name);
        let requirement_path = trait_path.append(requirement_name);
        let receiver = match requirement.receiver_access {
            PublicTraitReceiverAccess::Immutable => {
                TraitReceiverRequirement::Immutable { this_type }
            }
            PublicTraitReceiverAccess::Mutable => TraitReceiverRequirement::Mutable { this_type },
        };
        let mut parameters = Vec::with_capacity(requirement.parameters.len());

        for (index, parameter) in requirement.parameters.iter().enumerate() {
            let name = parameter
                .name
                .as_deref()
                .map(|name| string_table.intern(name))
                .unwrap_or_else(|| string_table.intern(&format!("parameter_{index}")));
            let type_id =
                self.project_imported_trait_surface_type(&parameter.type_identity, this_type)?;
            parameters.push(ResolvedTraitParameter {
                name: requirement_path.append(name),
                value_mode: parameter.value_mode.clone(),
                type_id,
                location: Default::default(),
            });
        }

        let mut returns = Vec::with_capacity(requirement.returns.len());
        for returned in &requirement.returns {
            let type_id =
                self.project_imported_trait_surface_type(&returned.type_identity, this_type)?;
            returns.push(ResolvedTraitReturn {
                type_id,
                channel: returned.channel,
                location: Default::default(),
            });
        }

        Ok(ResolvedTraitRequirement {
            id: requirement_id,
            name: requirement_name,
            name_location: Default::default(),
            receiver,
            parameters,
            returns,
            location: Default::default(),
        })
    }

    fn project_imported_trait_surface_type(
        &mut self,
        identity: &TraitSurfaceTypeIdentity,
        this_type: TypeId,
    ) -> Result<TypeId, CompilerError> {
        match identity {
            TraitSurfaceTypeIdentity::SelfType => Ok(this_type),
            TraitSurfaceTypeIdentity::Concrete(identity) => {
                self.intern_imported_canonical_type(identity)
            }
        }
    }

    /// Patches imported nominal generic lists once imported and core trait IDs are available.
    pub(in crate::compiler_frontend::ast) fn resolve_imported_generic_parameter_bounds(
        &mut self,
        trait_environment: &TraitEnvironment,
    ) -> Result<(), CompilerError> {
        let registrations = self.imported_generic_parameter_registrations.clone();

        for registration in registrations {
            let mut bounds_by_local = FxHashMap::default();

            for (index, parameter) in registration.surfaces.iter().enumerate() {
                let mut bounds = Vec::with_capacity(parameter.bounds.len());
                let mut seen = FxHashSet::default();
                for bound in &parameter.bounds {
                    let trait_id = trait_environment
                        .id_for_canonical_identity(bound)
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "An imported generic bound names a trait absent from the provider closure.",
                            )
                        })?;
                    if !seen.insert(trait_id) {
                        return Err(CompilerError::compiler_error(
                            "An imported generic parameter contains a duplicate canonical trait bound.",
                        ));
                    }
                    bounds.push(trait_id);
                }
                bounds_by_local.insert(TypeParameterId(index as u32), bounds);
            }

            self.type_environment.update_generic_parameter_bounds(
                registration.list_id,
                &bounds_by_local,
                &registration.canonical_by_local,
            );
        }

        Ok(())
    }

    /// Projects provider reusable evidence after receiver methods have their local call paths.
    pub(in crate::compiler_frontend::ast) fn project_imported_trait_evidence(
        &mut self,
        trait_environment: &TraitEnvironment,
        evidence_environment: &mut TraitEvidenceEnvironment,
        string_table: &StringTable,
    ) -> Result<(), CompilerError> {
        // Evidence is already keyed by canonical identity and agreement-checked when provider
        // semantics are imported; project it in deterministic identity order.
        let mut unique_evidence = self
            .import_environment
            .imported_evidence_by_identity
            .values()
            .cloned()
            .collect::<Vec<_>>();
        unique_evidence.sort_by(|left, right| left.identity.cmp(&right.identity));

        for evidence in unique_evidence {
            let target_type_id =
                self.intern_imported_canonical_type(evidence.identity.target_type_identity())?;
            let trait_id = trait_environment
                .id_for_canonical_identity(evidence.identity.trait_identity())
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported reusable evidence names a trait absent from the provider closure.",
                    )
                })?;
            let trait_definition = trait_environment.get(trait_id).ok_or_else(|| {
                CompilerError::compiler_error(
                    "Imported reusable evidence resolved to a missing local trait definition.",
                )
            })?;
            let mut requirements = Vec::with_capacity(evidence.requirement_mappings.len());

            for mapping in &evidence.requirement_mappings {
                if mapping.requirement_identity.trait_identity()
                    != evidence.identity.trait_identity()
                {
                    return Err(CompilerError::compiler_error(
                        "Imported reusable evidence maps a requirement owned by another trait.",
                    ));
                }
                let requirement = trait_definition
                    .requirements
                    .iter()
                    .find(|requirement| {
                        string_table.resolve(requirement.name)
                            == mapping.requirement_identity.requirement_name()
                    })
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Imported reusable evidence names a missing trait requirement.",
                        )
                    })?;
                let method_path = self
                    .imported_method_path(&mapping.method_origin, string_table)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Imported reusable evidence has no visible projected receiver method target.",
                        )
                    })?;
                requirements.push(TraitRequirementEvidence {
                    requirement_id: requirement.id,
                    method_path,
                });
            }

            if requirements.len() != trait_definition.requirements.len() {
                return Err(CompilerError::compiler_error(
                    "Imported reusable evidence does not map every trait requirement.",
                ));
            }

            evidence_environment.insert_validated(TraitEvidenceDefinition {
                id: TraitEvidenceId(0),
                kind: TraitEvidenceKind::Canonical,
                target_type_id,
                trait_id,
                source_file: InternedPath::new(),
                declaration_location: Default::default(),
                requirements,
            });
        }

        Ok(())
    }

    fn imported_method_path(
        &self,
        method_origin: &OriginFunctionId,
        string_table: &StringTable,
    ) -> Option<InternedPath> {
        let mut paths = self
            .projected_imported_functions_by_local_path
            .iter()
            .filter_map(|(path, contract)| match &contract.target {
                SourceFunctionTarget::Imported { origin, .. } if origin == method_origin => {
                    Some(path.clone())
                }
                SourceFunctionTarget::Local(_)
                | SourceFunctionTarget::Imported { .. }
                | SourceFunctionTarget::Generated { .. }
                | SourceFunctionTarget::ModulePrivate { .. } => None,
            })
            .collect::<Vec<_>>();
        paths.sort_by_key(|path| path.to_string(string_table));
        paths.into_iter().next()
    }
}

fn imported_trait_path(origin: &OriginTraitId, string_table: &mut StringTable) -> InternedPath {
    let mut path = InternedPath::from_single_str("<imported-trait>", string_table);
    let package_origin = match origin.module_origin().package().origin() {
        crate::builder_surface::PackageOrigin::Core => "core",
        crate::builder_surface::PackageOrigin::Standard => "standard",
        crate::builder_surface::PackageOrigin::Builder => "builder",
        crate::builder_surface::PackageOrigin::ProjectLocal => "project",
        crate::builder_surface::PackageOrigin::Dependency => "dependency",
    };
    let root_role = match origin.module_origin().role() {
        crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal => "normal",
        crate::compiler_frontend::semantic_identity::ModuleRootRole::Support => "support",
        crate::compiler_frontend::semantic_identity::ModuleRootRole::ProjectPackageFacade => {
            "facade"
        }
    };
    path.push_str(package_origin, string_table);
    path.push_str(origin.module_origin().package().name(), string_table);
    path.push_str(root_role, string_table);
    for component in origin.module_origin().logical_module_path().split('/') {
        if !component.is_empty() {
            path.push_str(component, string_table);
        }
    }
    path.push_str(origin.defining_name(), string_table);
    path
}
