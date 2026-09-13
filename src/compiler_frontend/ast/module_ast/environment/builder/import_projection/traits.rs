//! Imported provider trait, generic-bound and reusable-evidence projection.
//!
//! WHAT: reconstructs consumer-local `TraitId`, generic-bound and `TraitEvidenceId` handles from
//! stable completed provider-interface records.
//! WHY: AST trait checks operate on dense local environments. This join lets them consume
//! immutable provider semantics without donor headers, donor type IDs or structural conformance
//! reconstruction.

use super::*;
use crate::compiler_frontend::canonical_type_identity::CanonicalTraitIdentity;
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
    /// headers resolve references to those dependency bindings.
    pub(in crate::compiler_frontend::ast) fn project_imported_trait_declarations(
        &mut self,
        trait_environment: &mut TraitEnvironment,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerError> {
        let mut imported_traits = self
            .binding_environment
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
            let trait_path = imported_trait_path(trait_origin, string_table, self.path_fork);
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
                source_file: PathId::ROOT,
                this_type,
                requirements,
                declaration_span: None,
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

        // Direct dependency bindings and aliases are alternate paths to the same imported trait.
        let mut visible_paths = self
            .binding_environment
            .imported_declarations_by_local_path
            .iter()
            .filter_map(|(path, origin)| {
                let OriginDeclarationId::Trait(origin) = origin else {
                    return None;
                };
                Some((path.clone(), origin.clone()))
            })
            .collect::<Vec<_>>();
        visible_paths.sort_by_key(|(path, _)| {
            self.path_fork.render_portable(*path, string_table, &mut Vec::new())
        });

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
        trait_path: &PathId,
        this_type: TypeId,
        requirement_id: TraitRequirementId,
        requirement: &crate::compiler_frontend::public_interface::PublicTraitRequirementSurface,
        string_table: &mut StringTable,
    ) -> Result<ResolvedTraitRequirement, CompilerError> {
        let requirement_name = string_table.intern(&requirement.name);
        let requirement_path = self
            .path_fork
            .try_intern_child(*trait_path, requirement_name)
            .expect("imported trait requirement path table exhausted");
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
                name: self
                    .path_fork
                    .try_intern_child(requirement_path, name)
                    .expect("imported trait parameter path table exhausted"),
                value_mode: parameter.value_mode.clone(),
                type_id,
                span: None,
            });
        }

        let mut returns = Vec::with_capacity(requirement.returns.len());
        for returned in &requirement.returns {
            let type_id =
                self.project_imported_trait_surface_type(&returned.type_identity, this_type)?;
            returns.push(ResolvedTraitReturn {
                type_id,
                channel: returned.channel,
                span: None,
            });
        }

        Ok(ResolvedTraitRequirement {
            id: requirement_id,
            name: requirement_name,
            receiver,
            parameters,
            returns,
            span: None,
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
    pub(in crate::compiler_frontend::ast) fn resolve_dependencyed_generic_parameter_bounds(
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
            .binding_environment
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
                    .imported_method_path(&mapping.method_origin)
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
                source_file: PathId::ROOT,
                declaration_span: None,
                requirements,
            });
        }

        Ok(())
    }

    fn imported_method_path(&self, method_origin: &OriginFunctionId) -> Option<PathId> {
        self.imported_receiver_method_paths_by_origin
            .get(method_origin)
            .cloned()
    }
}

fn imported_trait_path(
    origin: &OriginTraitId,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> PathId {
    let package_origin = match origin.module_origin().package().origin() {
        crate::builder_surface::PackageOrigin::Core => "core",
        crate::builder_surface::PackageOrigin::Builder => "builder",
        crate::builder_surface::PackageOrigin::ProjectLocal => "project",
        crate::builder_surface::PackageOrigin::Dependency => "dependency",
    };
    let root_role = match origin.module_origin().role() {
        crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal => "normal",
        crate::compiler_frontend::semantic_identity::ModuleRootRole::Support => "support",
        crate::compiler_frontend::semantic_identity::ModuleRootRole::ProjectPackageFacade => "facade",
    };
    let mut path = PathId::ROOT;
    for component in std::iter::once("<imported-trait>")
        .chain(std::iter::once(package_origin))
        .chain(std::iter::once(origin.module_origin().package().name()))
        .chain(std::iter::once(root_role))
        .chain(origin.module_origin().logical_module_path().split('/'))
        .chain(std::iter::once(origin.defining_name()))
    {
        if !component.is_empty() {
            path = path_fork
                .try_intern_child(path, string_table.intern(component))
                .expect("imported trait path table exhausted");
        }
    }
    path
}
