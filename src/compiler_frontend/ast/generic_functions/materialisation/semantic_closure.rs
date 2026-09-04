//! Closed semantic inputs selected for one retained generic materialisation.

use super::frozen_syntax::{StableSourceLocation, materialise_path, stable_path};
use super::nominal_blueprints::{
    MaterialisationTypeBlueprint, NominalMaterialisationBlueprint, intern_generated_canonical_type,
    intern_materialisation_type_blueprint,
};
use super::{
    GenericTemplateArtefact, MaterialisationNominalSource, ModuleMaterialisationContext,
    ModuleMaterialisationPreparation, StableCallableBinding, StableDeclarationBinding,
    StableFunctionSignature, StableFunctionTarget, StableNominalBinding,
    collect_namespace_source_paths,
};
use crate::compiler_frontend::ast::module_ast::environment::AstModuleEnvironment;
use crate::compiler_frontend::ast::statements::functions::ReturnChannel;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalTraitIdentity, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, OriginDeclarationId,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::traits::definitions::{
    ResolvedTraitDefinition, ResolvedTraitParameter, ResolvedTraitRequirement, ResolvedTraitReturn,
    TraitReceiverRequirement, TraitVisibility,
};
use crate::compiler_frontend::traits::environment::trait_this_name;
use crate::compiler_frontend::traits::evidence::TraitEvidenceDefinition;
use crate::compiler_frontend::traits::evidence::environment::{
    TraitEvidenceKind, TraitRequirementEvidence,
};
use crate::compiler_frontend::traits::ids::TraitEvidenceId;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::rc::Rc;
/// Module-owned facts needed to reconstruct private declarations referenced by retained bodies.
///
/// Public/imported declarations continue to use the published declaration closure retained by
/// [`ModuleMaterialisationContext`]. Private constants and aliases have no cross-module origin,
/// so they are retained once here as stable values and selected by each artefact's local-path list.
#[derive(Clone, Default)]
pub(super) struct StableSemanticClosure {
    pub(super) constants: Box<[StableLocalConstant]>,
    pub(super) aliases: Box<[StableLocalAlias]>,
    traits: Box<[StablePrivateTrait]>,
    evidence: Box<[StablePrivateEvidence]>,
}

#[derive(Clone)]
pub(super) struct StableLocalConstant {
    pub(super) local_path: Box<[String]>,
    pub(super) type_identity: CanonicalTypeIdentity,
    pub(super) value: PublicFoldedValue,
    pub(super) location: StableSourceLocation,
}

#[derive(Clone)]
pub(super) struct StableLocalAlias {
    pub(super) local_path: Box<[String]>,
    pub(super) target_type_identity: CanonicalTypeIdentity,
    pub(super) declaration_location: StableSourceLocation,
}

#[derive(Clone)]
struct StablePrivateTrait {
    identity: CanonicalTraitIdentity,
    name: String,
    canonical_path: Box<[String]>,
    source_file: Box<[String]>,
    declaration_location: StableSourceLocation,
    requirements: Box<[StablePrivateTraitRequirement]>,
}

#[derive(Clone)]
struct StablePrivateTraitRequirement {
    name: String,
    receiver_mutable: bool,
    parameters: Box<[StablePrivateTraitParameter]>,
    returns: Box<[StablePrivateTraitReturn]>,
    location: StableSourceLocation,
}

#[derive(Clone)]
struct StablePrivateTraitParameter {
    name: Box<[String]>,
    value_mode: ValueMode,
    parameter_type: StableTraitTypeBlueprint,
    location: StableSourceLocation,
}

#[derive(Clone)]
struct StablePrivateTraitReturn {
    return_type: StableTraitTypeBlueprint,
    channel: ReturnChannel,
    location: StableSourceLocation,
}

#[derive(Clone)]
enum StableTraitTypeBlueprint {
    This,
    Type(Box<MaterialisationTypeBlueprint>),
}

#[derive(Clone)]
struct StablePrivateEvidence {
    target_type_identity: CanonicalTypeIdentity,
    trait_identity: CanonicalTraitIdentity,
    source_file: Box<[String]>,
    declaration_location: StableSourceLocation,
    requirements: Box<[StablePrivateEvidenceRequirement]>,
}

#[derive(Clone)]
struct StablePrivateEvidenceRequirement {
    requirement_name: String,
    method_path: Box<[String]>,
}
impl StableFunctionSignature {
    fn collect_nominal_identities(&self, identities: &mut FxHashSet<CanonicalTypeIdentity>) {
        for parameter in &self.parameters {
            parameter
                .parameter_type
                .collect_nominal_identities(identities);
        }
        for returned in &self.returns {
            returned.return_type.collect_nominal_identities(identities);
        }
    }
}
pub(super) fn stable_body_symbol_names(
    tokens: &FileTokens,
    string_table: &StringTable,
) -> FxHashSet<String> {
    tokens
        .tokens
        .iter()
        .filter_map(|token| match token.kind {
            TokenKind::Symbol(symbol) => Some(string_table.resolve(symbol).to_owned()),
            _ => None,
        })
        .collect()
}

/// Collect every nominal identity one canonical type reaches, including generic-instance bases.
///
/// WHY: `CanonicalTypeIdentity::visit` yields an instance and its arguments, but reconstructing
/// `Box of Int` inside a generated sidecar also needs the blueprint of `Box` itself. Blueprint
/// type trees already record that base explicitly; canonical identities carry it on the instance.
fn collect_reachable_nominal_identities(
    identity: &CanonicalTypeIdentity,
    identities: &mut FxHashSet<CanonicalTypeIdentity>,
) {
    identity.visit(&mut |nested| {
        identities.insert(nested.clone());

        match nested {
            CanonicalTypeIdentity::GenericInstance(instance) => {
                identities.insert(CanonicalTypeIdentity::SourceNominal(
                    instance.base().clone(),
                ));
            }
            CanonicalTypeIdentity::ModulePrivateGenericInstance(instance) => {
                identities.insert(CanonicalTypeIdentity::ModulePrivateNominal(
                    instance.base().clone(),
                ));
            }
            _ => {}
        }
    });
}
pub(super) fn install_private_semantic_closure(
    nominal_source: &GenericTemplateArtefact,
    context: &ModuleMaterialisationContext,
    environment: &mut AstModuleEnvironment,
    external_package_registry: &ExternalPackageRegistry,
    template_ir_store: &Rc<RefCell<crate::compiler_frontend::ast::templates::tir::TemplateIrStore>>,
    string_table: &mut StringTable,
) -> Result<(), CompilerError> {
    for stable_trait in &context.semantic_closure.traits {
        if environment
            .lookups
            .trait_environment
            .id_for_canonical_identity(&stable_trait.identity)
            .is_some()
        {
            continue;
        }
        let this_type = environment
            .type_environment
            .register_synthetic_generic_parameter(trait_this_name(string_table));
        let trait_id = environment.lookups.trait_environment.next_trait_id();
        let mut requirements = Vec::with_capacity(stable_trait.requirements.len());
        for stable_requirement in &stable_trait.requirements {
            let requirement_id = environment.lookups.trait_environment.next_requirement_id();
            let receiver = if stable_requirement.receiver_mutable {
                TraitReceiverRequirement::Mutable { this_type }
            } else {
                TraitReceiverRequirement::Immutable { this_type }
            };
            let parameters = stable_requirement
                .parameters
                .iter()
                .map(|parameter| {
                    Ok(ResolvedTraitParameter {
                        name: materialise_path(&parameter.name, string_table),
                        value_mode: parameter.value_mode.clone(),
                        type_id: intern_stable_trait_type(
                            &parameter.parameter_type,
                            this_type,
                            nominal_source,
                            &mut environment.type_environment,
                            external_package_registry,
                            template_ir_store,
                            string_table,
                        )?,
                        location: parameter.location.materialise(string_table),
                    })
                })
                .collect::<Result<Vec<_>, CompilerError>>()?;
            let returns = stable_requirement
                .returns
                .iter()
                .map(|returned| {
                    Ok(ResolvedTraitReturn {
                        type_id: intern_stable_trait_type(
                            &returned.return_type,
                            this_type,
                            nominal_source,
                            &mut environment.type_environment,
                            external_package_registry,
                            template_ir_store,
                            string_table,
                        )?,
                        channel: returned.channel,
                        location: returned.location.materialise(string_table),
                    })
                })
                .collect::<Result<Vec<_>, CompilerError>>()?;
            let location = stable_requirement.location.materialise(string_table);
            requirements.push(ResolvedTraitRequirement {
                id: requirement_id,
                name: string_table.intern(&stable_requirement.name),
                name_location: location.clone(),
                receiver,
                parameters,
                returns,
                location,
            });
        }
        let canonical_path = materialise_path(&stable_trait.canonical_path, string_table);
        let source_file = materialise_path(&stable_trait.source_file, string_table);
        let definition = ResolvedTraitDefinition {
            id: trait_id,
            name: string_table.intern(&stable_trait.name),
            canonical_path: canonical_path.clone(),
            source_file,
            this_type,
            requirements,
            declaration_location: stable_trait.declaration_location.materialise(string_table),
            visibility: TraitVisibility::Source { exported: false },
        };
        let lookups = Rc::make_mut(&mut environment.lookups);
        let trait_environment = Rc::make_mut(&mut lookups.trait_environment);
        if trait_environment.insert(definition).is_some() {
            return Err(CompilerError::compiler_error(
                "Private materialisation trait path was registered more than once",
            ));
        }
        trait_environment.register_path(canonical_path, trait_id)?;
        trait_environment.register_canonical_identity(stable_trait.identity.clone(), trait_id)?;
    }

    for stable_evidence in &context.semantic_closure.evidence {
        let target_type_id = intern_generated_canonical_type(
            &stable_evidence.target_type_identity,
            &mut environment.type_environment,
            external_package_registry,
            nominal_source,
            string_table,
        )?;
        let trait_id = environment
            .lookups
            .trait_environment
            .id_for_canonical_identity(&stable_evidence.trait_identity)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Private materialisation evidence trait was not reconstructed",
                )
            })?;
        if environment
            .lookups
            .trait_evidence_environment
            .canonical_for(target_type_id, trait_id)
            .is_some()
        {
            continue;
        }
        let trait_definition = environment
            .lookups
            .trait_environment
            .get(trait_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Private materialisation evidence has no reconstructed trait definition",
                )
            })?;
        let requirements = stable_evidence
            .requirements
            .iter()
            .map(|requirement| {
                let trait_requirement = trait_definition
                    .requirements
                    .iter()
                    .find(|candidate| {
                        string_table.resolve(candidate.name) == requirement.requirement_name
                    })
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Private materialisation evidence requirement is absent from its trait",
                        )
                    })?;
                Ok(TraitRequirementEvidence {
                    requirement_id: trait_requirement.id,
                    method_path: materialise_path(&requirement.method_path, string_table),
                })
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;
        let definition = TraitEvidenceDefinition {
            id: TraitEvidenceId(0),
            kind: TraitEvidenceKind::Canonical,
            target_type_id,
            trait_id,
            source_file: materialise_path(&stable_evidence.source_file, string_table),
            declaration_location: stable_evidence
                .declaration_location
                .materialise(string_table),
            requirements,
        };
        let lookups = Rc::make_mut(&mut environment.lookups);
        Rc::make_mut(&mut lookups.trait_evidence_environment).insert_validated(definition);
    }
    Ok(())
}

fn intern_stable_trait_type(
    blueprint: &StableTraitTypeBlueprint,
    this_type: TypeId,
    nominal_source: &impl MaterialisationNominalSource,
    type_environment: &mut TypeEnvironment,
    external_package_registry: &ExternalPackageRegistry,
    _template_ir_store: &Rc<
        RefCell<crate::compiler_frontend::ast::templates::tir::TemplateIrStore>,
    >,
    string_table: &mut StringTable,
) -> Result<TypeId, CompilerError> {
    match blueprint {
        StableTraitTypeBlueprint::This => Ok(this_type),
        StableTraitTypeBlueprint::Type(blueprint) => intern_materialisation_type_blueprint(
            blueprint,
            &[this_type],
            nominal_source,
            type_environment,
            external_package_registry,
            string_table,
        ),
    }
}
impl ModuleMaterialisationPreparation {
    /// Capture local constants and transparent aliases once for the whole declaring module.
    ///
    /// These declarations have no provider origin, so retaining only their visible paths would
    /// leave a generated sidecar with a visibility entry but no declaration fact to resolve.
    pub(super) fn stable_semantic_closure(
        &self,
        resources: &ModuleResourceTable,
    ) -> Result<StableSemanticClosure, CompilerError> {
        let mut constants = Vec::new();
        for row in self.const_values.iter_module_constant_views() {
            let (path, metadata) = (row.path, row.metadata);
            if self
                .binding_environment
                .imported_declarations_by_local_path
                .contains_key(path)
            {
                continue;
            }
            let type_identity = self.stable_type_identity(metadata.type_id)?;
            let value = self.stable_folded_value_at_path(path, resources)?;
            constants.push(StableLocalConstant {
                local_path: stable_path(path, &self.string_table),
                type_identity,
                value,
                location: StableSourceLocation::capture(&metadata.location, &self.string_table),
            });
        }
        constants.sort_by(|left, right| left.local_path.cmp(&right.local_path));

        let mut aliases = self
            .resolved_type_aliases_by_path
            .iter()
            .filter(|(path, _)| {
                !self
                    .binding_environment
                    .imported_declarations_by_local_path
                    .contains_key(*path)
            })
            .map(|(path, alias)| {
                let target_type_identity = self.stable_alias_target_identity(path, alias)?;
                Ok(StableLocalAlias {
                    local_path: stable_path(path, &self.string_table),
                    target_type_identity,
                    declaration_location: StableSourceLocation::capture(
                        &alias.declaration_location,
                        &self.string_table,
                    ),
                })
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;
        aliases.sort_by(|left, right| left.local_path.cmp(&right.local_path));

        let traits = self.stable_private_traits()?;
        let evidence = self.stable_private_evidence()?;

        Ok(StableSemanticClosure {
            constants: constants.into_boxed_slice(),
            aliases: aliases.into_boxed_slice(),
            traits,
            evidence,
        })
    }
    fn stable_private_traits(&self) -> Result<Box<[StablePrivateTrait]>, CompilerError> {
        let mut traits = Vec::new();
        for definition in self.trait_environment.definitions() {
            let Some(identity) = self
                .trait_environment
                .canonical_identity_for_id(definition.id)
                .cloned()
            else {
                continue;
            };
            if !matches!(identity, CanonicalTraitIdentity::ModulePrivate(_)) {
                continue;
            }
            let requirements = definition
                .requirements
                .iter()
                .map(|requirement| {
                    let parameters = requirement
                        .parameters
                        .iter()
                        .map(|parameter| {
                            Ok(StablePrivateTraitParameter {
                                name: stable_path(&parameter.name, &self.string_table),
                                value_mode: parameter.value_mode.clone(),
                                parameter_type: self.stable_trait_type(
                                    parameter.type_id,
                                    definition.this_type,
                                )?,
                                location: StableSourceLocation::capture(
                                    &parameter.location,
                                    &self.string_table,
                                ),
                            })
                        })
                        .collect::<Result<Box<[_]>, CompilerError>>()?;
                    let returns = requirement
                        .returns
                        .iter()
                        .map(|returned| {
                            Ok(StablePrivateTraitReturn {
                                return_type: self.stable_trait_type(
                                    returned.type_id,
                                    definition.this_type,
                                )?,
                                channel: returned.channel,
                                location: StableSourceLocation::capture(
                                    &returned.location,
                                    &self.string_table,
                                ),
                            })
                        })
                        .collect::<Result<Box<[_]>, CompilerError>>()?;
                    let receiver_mutable = matches!(
                        requirement.receiver,
                        crate::compiler_frontend::traits::definitions::TraitReceiverRequirement::Mutable { .. }
                    );
                    Ok(StablePrivateTraitRequirement {
                        name: self.string_table.resolve(requirement.name).to_owned(),
                        receiver_mutable,
                        parameters,
                        returns,
                        location: StableSourceLocation::capture(
                            &requirement.location,
                            &self.string_table,
                        ),
                    })
                })
                .collect::<Result<Box<[_]>, CompilerError>>()?;
            traits.push(StablePrivateTrait {
                identity,
                name: self.string_table.resolve(definition.name).to_owned(),
                canonical_path: stable_path(&definition.canonical_path, &self.string_table),
                source_file: stable_path(&definition.source_file, &self.string_table),
                declaration_location: StableSourceLocation::capture(
                    &definition.declaration_location,
                    &self.string_table,
                ),
                requirements,
            });
        }
        traits.sort_by(|left, right| left.identity.cmp(&right.identity));
        Ok(traits.into_boxed_slice())
    }

    fn stable_private_evidence(&self) -> Result<Box<[StablePrivateEvidence]>, CompilerError> {
        let mut evidence = Vec::new();
        for definition in self.trait_evidence_environment.canonical_evidence() {
            let target_type_identity = self.stable_type_identity(definition.target_type_id)?;
            let trait_identity = self
                .trait_environment
                .canonical_identity_for_id(definition.trait_id)
                .cloned()
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Private materialisation evidence has no canonical trait identity",
                    )
                })?;
            let target_is_private = {
                let mut private = false;
                target_type_identity.visit(&mut |identity| {
                    private |= matches!(
                        identity,
                        CanonicalTypeIdentity::ModulePrivateNominal(_)
                            | CanonicalTypeIdentity::ModulePrivateGenericInstance(_)
                    );
                });
                private
            };
            if !target_is_private
                && !matches!(trait_identity, CanonicalTraitIdentity::ModulePrivate(_))
            {
                continue;
            }
            let trait_definition =
                self.trait_environment
                    .get(definition.trait_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Private materialisation evidence has no trait definition",
                        )
                    })?;
            let requirements = definition
                .requirements
                .iter()
                .map(|requirement| {
                    let trait_requirement = trait_definition
                        .requirements
                        .iter()
                        .find(|candidate| candidate.id == requirement.requirement_id)
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "Private materialisation evidence has no trait requirement",
                            )
                        })?;
                    Ok(StablePrivateEvidenceRequirement {
                        requirement_name: self
                            .string_table
                            .resolve(trait_requirement.name)
                            .to_owned(),
                        method_path: stable_path(&requirement.method_path, &self.string_table),
                    })
                })
                .collect::<Result<Box<[_]>, CompilerError>>()?;
            evidence.push(StablePrivateEvidence {
                target_type_identity,
                trait_identity,
                source_file: stable_path(&definition.source_file, &self.string_table),
                declaration_location: StableSourceLocation::capture(
                    &definition.declaration_location,
                    &self.string_table,
                ),
                requirements,
            });
        }
        evidence.sort_by(|left, right| {
            left.target_type_identity
                .cmp(&right.target_type_identity)
                .then_with(|| left.trait_identity.cmp(&right.trait_identity))
        });
        Ok(evidence.into_boxed_slice())
    }

    fn stable_trait_type(
        &self,
        type_id: TypeId,
        this_type: TypeId,
    ) -> Result<StableTraitTypeBlueprint, CompilerError> {
        if type_id == this_type {
            Ok(StableTraitTypeBlueprint::This)
        } else {
            let this_parameter_id = match self.type_environment.get(this_type) {
                Some(TypeDefinition::GenericParameter(parameter)) => parameter.id,
                _ => {
                    return Err(CompilerError::compiler_error(
                        "Private trait receiver is not represented by a generic parameter",
                    ));
                }
            };
            let parameter_slots = FxHashMap::from_iter([(this_parameter_id, 0)]);
            Ok(StableTraitTypeBlueprint::Type(Box::new(
                self.materialisation_type_blueprint(type_id, &parameter_slots)?,
            )))
        }
    }

    pub(super) fn stable_local_declaration_bindings(
        &self,
        selected_paths: &FxHashSet<InternedPath>,
    ) -> Box<[Box<[String]>]> {
        let mut paths = selected_paths
            .iter()
            .filter(|path| {
                (self.const_values.value_for_path(path).is_some()
                    && !self
                        .binding_environment
                        .imported_declarations_by_local_path
                        .contains_key(*path))
                    || (self.resolved_type_aliases_by_path.contains_key(*path)
                        && !self
                            .binding_environment
                            .imported_declarations_by_local_path
                            .contains_key(*path))
            })
            .map(|path| stable_path(path, &self.string_table))
            .collect::<Vec<_>>();
        paths.sort();
        paths.into_boxed_slice()
    }
    pub(super) fn selected_visible_paths(
        &self,
        source_file: &InternedPath,
        referenced_names: &FxHashSet<String>,
    ) -> Result<FxHashSet<InternedPath>, CompilerError> {
        let visibility = self.binding_environment.visibility_for(source_file)?;
        let mut selected = visibility
            .visible_source_names
            .iter()
            .chain(visibility.visible_type_alias_names.iter())
            .chain(visibility.visible_trait_names.iter())
            .filter(|(name, _)| referenced_names.contains(self.string_table.resolve(**name)))
            .map(|(_, target)| target.local_path().clone())
            .collect::<FxHashSet<_>>();
        for (name, methods) in &visibility.visible_receiver_methods {
            if referenced_names.contains(self.string_table.resolve(*name)) {
                selected.extend(
                    methods
                        .iter()
                        .map(|method| method.target.local_path().clone()),
                );
            }
        }
        for (name, record) in &visibility.visible_namespace_records {
            if referenced_names.contains(self.string_table.resolve(*name)) {
                collect_namespace_source_paths(record, &mut selected);
            }
        }
        Ok(selected)
    }

    pub(super) fn stable_target_for_path(
        &self,
        target: &SourceFunctionTarget,
    ) -> Option<StableFunctionTarget> {
        if let Some(stable) = StableFunctionTarget::capture(target) {
            return Some(stable);
        }
        if let Some(contract) = self
            .imported_functions_by_local_path
            .get(target.local_path())
            && let Some(stable) = StableFunctionTarget::capture(&contract.target)
        {
            return Some(stable);
        }
        self.generic_function_templates_by_path
            .get(target.local_path())
            .and_then(|template| match template.declaration_identity.as_ref()? {
                GeneratedDeclarationIdentity::Public(origin) => {
                    Some(StableFunctionTarget::Imported(origin.clone()))
                }
                GeneratedDeclarationIdentity::ModulePrivate(identity) => {
                    Some(StableFunctionTarget::ModulePrivate(identity.clone()))
                }
            })
    }

    pub(super) fn stable_declaration_bindings(
        &self,
        selected_paths: &FxHashSet<InternedPath>,
        public_interface: &PublicSemanticInterface,
    ) -> Result<Box<[StableDeclarationBinding]>, CompilerError> {
        let mut bindings = Vec::new();
        for path in selected_paths {
            if let Some(origin) = self
                .binding_environment
                .imported_declarations_by_local_path
                .get(path)
                && self
                    .binding_environment
                    .imported_declarations_by_origin
                    .contains_key(origin)
            {
                bindings.push(StableDeclarationBinding {
                    local_path: stable_path(path, &self.string_table),
                    origin: origin.clone(),
                });
                continue;
            }
            let origin = self.public_origin_for_path(path);
            if let Some(origin) = origin
                && public_interface.declaration(&origin).is_some()
            {
                bindings.push(StableDeclarationBinding {
                    local_path: stable_path(path, &self.string_table),
                    origin,
                });
            }
        }
        bindings.sort_by(|left, right| left.local_path.cmp(&right.local_path));
        Ok(bindings.into_boxed_slice())
    }

    fn public_origin_for_path(&self, path: &InternedPath) -> Option<OriginDeclarationId> {
        if let Some(template) = self.generic_function_templates_by_path.get(path)
            && let Some(GeneratedDeclarationIdentity::Public(origin)) =
                template.declaration_identity.as_ref()
        {
            return Some(OriginDeclarationId::Function(origin.clone()));
        }
        if let Some(contract) = self.imported_functions_by_local_path.get(path)
            && let SourceFunctionTarget::Imported { origin, .. } = &contract.target
        {
            return Some(OriginDeclarationId::Function(origin.clone()));
        }
        if let Some(type_id) = self.nominal_type_ids_by_path.get(path)
            && let Some(CanonicalTypeIdentity::SourceNominal(origin)) = self
                .type_environment
                .canonical_identity_for_type_id(*type_id)
        {
            return Some(OriginDeclarationId::Type(origin.clone()));
        }
        if let Some(trait_id) = self.trait_environment.id_for_path(path)
            && let Some(CanonicalTraitIdentity::Source(origin)) =
                self.trait_environment.canonical_identity_for_id(trait_id)
        {
            return Some(OriginDeclarationId::Trait(origin.clone()));
        }
        None
    }

    pub(super) fn stable_callable_bindings(
        &self,
        selected_paths: &FxHashSet<InternedPath>,
        resources: &ModuleResourceTable,
    ) -> Result<Box<[StableCallableBinding]>, CompilerError> {
        let mut callables = Vec::new();
        for path in selected_paths {
            let Some(contract) = self.imported_functions_by_local_path.get(path) else {
                continue;
            };
            let Some(target) = StableFunctionTarget::capture(&contract.target) else {
                continue;
            };
            let Some(resolved) = self.resolved_function_signatures_by_path.get(path) else {
                continue;
            };
            if resolved.receiver.is_some() {
                continue;
            }
            callables.push(StableCallableBinding {
                local_path: stable_path(path, &self.string_table),
                target,
                signature: self.stable_function_signature(
                    &resolved.signature,
                    &FxHashMap::default(),
                    resources,
                )?,
                summary: contract.summary.clone(),
            });
        }
        callables.sort_by(|left, right| left.local_path.cmp(&right.local_path));
        Ok(callables.into_boxed_slice())
    }
    /// Collect the nominal blueprints one template's generated sidecar can need.
    ///
    /// WHAT: gathers every canonical identity the artefact reaches (signature, selected nominals,
    /// selected constants and their folded values, selected alias targets, private traits and
    /// retained evidence) and keeps the blueprint of each one that has one.
    /// WHY: this is the single owner of blueprint closure collection. Every canonical identity
    /// source runs through [`collect_reachable_nominal_identities`], because a concrete
    /// `Box of Int` identity also needs the `Box` blueprint and canonical identities carry that
    /// base on the instance rather than as a nested identity.
    pub(super) fn stable_nominal_blueprints(
        &self,
        selected_paths: &FxHashSet<InternedPath>,
        signature: &StableFunctionSignature,
        semantic_closure: &StableSemanticClosure,
        resources: &ModuleResourceTable,
    ) -> Result<FxHashMap<CanonicalTypeIdentity, NominalMaterialisationBlueprint>, CompilerError>
    {
        let mut identities = FxHashSet::default();
        signature.collect_nominal_identities(&mut identities);
        for path in selected_paths {
            if let Some(type_id) = self.nominal_type_ids_by_path.get(path)
                && let Some(identity) = self
                    .type_environment
                    .canonical_identity_for_type_id(*type_id)
            {
                collect_reachable_nominal_identities(identity, &mut identities);
            }
            if let Some(value_id) = self.const_values.value_for_path(path) {
                if let Some(metadata) = self.const_values.metadata(value_id)
                    && let Ok(identity) = self.stable_type_identity(metadata.type_id)
                {
                    collect_reachable_nominal_identities(&identity, &mut identities);
                }
                if let Ok(value) = self.stable_folded_value_at_path(path, resources) {
                    value.visit_type_identities(&mut |identity| {
                        collect_reachable_nominal_identities(identity, &mut identities);
                    });
                }
            }
            self.collect_selected_alias_identities(path, semantic_closure, &mut identities);
        }
        for trait_definition in &semantic_closure.traits {
            for requirement in &trait_definition.requirements {
                for parameter in &requirement.parameters {
                    if let StableTraitTypeBlueprint::Type(blueprint) = &parameter.parameter_type {
                        blueprint.collect_nominal_identities(&mut identities);
                    }
                }
                for returned in &requirement.returns {
                    if let StableTraitTypeBlueprint::Type(blueprint) = &returned.return_type {
                        blueprint.collect_nominal_identities(&mut identities);
                    }
                }
            }
        }
        for evidence in &semantic_closure.evidence {
            collect_reachable_nominal_identities(&evidence.target_type_identity, &mut identities);
        }
        let mut blueprints = FxHashMap::default();
        for identity in identities {
            if let Some(blueprint) = self.nominal_blueprints.get(&identity) {
                blueprints.insert(identity, blueprint.clone());
            }
        }
        Ok(blueprints)
    }

    /// Add the nominal identities one selected alias target reaches.
    ///
    /// WHY: alias rows are installed per artefact from the module-wide closure, so a template
    /// only needs blueprints for the aliases its own local-declaration list installs. Scanning
    /// every module alias for every template retained unrelated blueprints in each sidecar. The
    /// closure projected each target once and is sorted by local path, so the row is looked up
    /// rather than re-projected.
    fn collect_selected_alias_identities(
        &self,
        path: &InternedPath,
        semantic_closure: &StableSemanticClosure,
        identities: &mut FxHashSet<CanonicalTypeIdentity>,
    ) {
        if !self.resolved_type_aliases_by_path.contains_key(path) {
            return;
        }

        let components = stable_path(path, &self.string_table);
        if let Ok(index) = semantic_closure
            .aliases
            .binary_search_by(|row| row.local_path.cmp(&components))
        {
            collect_reachable_nominal_identities(
                &semantic_closure.aliases[index].target_type_identity,
                identities,
            );
        }
    }

    pub(super) fn stable_nominal_bindings(
        &self,
        selected_paths: &FxHashSet<InternedPath>,
    ) -> Box<[StableNominalBinding]> {
        let mut bindings = selected_paths
            .iter()
            .filter_map(|path| {
                let type_id = self.nominal_type_ids_by_path.get(path)?;
                let identity = self
                    .type_environment
                    .canonical_identity_for_type_id(*type_id)?;
                self.nominal_blueprints
                    .contains_key(identity)
                    .then(|| StableNominalBinding {
                        local_path: stable_path(path, &self.string_table),
                        identity: identity.clone(),
                    })
            })
            .collect::<Vec<_>>();
        bindings.sort_by(|left, right| left.local_path.cmp(&right.local_path));
        bindings.into_boxed_slice()
    }
}
