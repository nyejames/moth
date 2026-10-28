//! Closed semantic inputs selected for one retained generic materialisation.

use super::artefact_emit::ModuleMaterialisationContext;
use super::nominal_blueprints::{
    MaterialisationTypeBlueprint, NominalMaterialisationBlueprint,
    NominalMaterialisationDefinition, intern_generated_canonical_type,
    intern_materialisation_type_blueprint,
};
use super::preparation_freeze::ModuleMaterialisationCapture;
use super::stable_types::{
    GeneratedValueMaterialisationServices, GenericTemplateArtefact, MaterialisationNominalSource,
    StableCallableBinding, StableDeclarationBinding, StableFunctionSignature, StableFunctionTarget,
    StableNominalBinding, const_store_error_into_prepared_failure,
};
use super::visibility::collect_namespace_source_paths;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::generic_functions::GenericFunctionBody;
use crate::compiler_frontend::ast::module_ast::environment::AstModuleEnvironment;
use crate::compiler_frontend::ast::statements::functions::{ParameterContract, ReturnChannel};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalTraitIdentity, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::{CompilerStage, PreparedFailure};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::public_interface::PublicDeclarationRecord;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, OriginDeclarationId,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{FrozenStringTable, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
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
impl StableSemanticClosure {
    /// A generated environment is only a projection of the declaring module. Never replace its
    /// original facts with recaptures from that projection, but retain genuinely new local facts.
    pub(super) fn inherit_original(&mut self, original: &Self) {
        let mut constants = original.constants.to_vec();
        constants.extend(std::mem::take(&mut self.constants));
        constants.sort_by_key(|row| row.local_path);
        constants.dedup_by_key(|row| row.local_path);
        self.constants = constants.into_boxed_slice();

        let mut aliases = original.aliases.to_vec();
        aliases.extend(std::mem::take(&mut self.aliases));
        aliases.sort_by_key(|row| row.local_path);
        aliases.dedup_by_key(|row| row.local_path);
        self.aliases = aliases.into_boxed_slice();

        let mut traits = original.traits.to_vec();
        traits.extend(std::mem::take(&mut self.traits));
        traits.sort_by(|left, right| left.identity.cmp(&right.identity));
        traits.dedup_by(|left, right| left.identity == right.identity);
        self.traits = traits.into_boxed_slice();

        let mut evidence = original.evidence.to_vec();
        evidence.extend(std::mem::take(&mut self.evidence));
        evidence.sort_by(|left, right| {
            left.target_type_identity
                .cmp(&right.target_type_identity)
                .then_with(|| left.trait_identity.cmp(&right.trait_identity))
        });
        evidence.dedup_by(|left, right| {
            left.target_type_identity == right.target_type_identity
                && left.trait_identity == right.trait_identity
        });
        self.evidence = evidence.into_boxed_slice();
    }

    pub(super) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        for constant in &mut self.constants {
            constant.local_path = remap.get(constant.local_path);
        }
        for alias in &mut self.aliases {
            alias.local_path = remap.get(alias.local_path);
        }
        for trait_definition in &mut self.traits {
            trait_definition.remap_path_ids(remap);
        }
        for evidence in &mut self.evidence {
            evidence.remap_path_ids(remap);
        }
    }

    /// Private evidence rows carry their own copy of the callable they dispatch to, because a
    /// consumer installs them without the provider's preparation. Convergence widens summaries
    /// after those copies are made, so every widening must refresh them too: a stale bootstrap
    /// summary would hide return aliases from the consumer.
    pub(super) fn update_callable_summary(&mut self, path: &PathId, summary: &PublicCallSummary) {
        for callable in self
            .evidence
            .iter_mut()
            .flat_map(|evidence| evidence.requirements.iter_mut())
            .filter_map(|requirement| requirement.callable.as_mut())
            .filter(|callable| &callable.local_path == path)
        {
            callable.summary = summary.clone();
        }
    }
}

#[derive(Clone)]
pub(super) struct StableLocalConstant {
    pub(super) local_path: PathId,
    pub(super) type_identity: CanonicalTypeIdentity,
    pub(super) value: PublicFoldedValue,
    pub(super) span: Option<crate::compiler_frontend::source::SourceSpan>,
}

#[derive(Clone)]
pub(super) struct StableLocalAlias {
    pub(super) local_path: PathId,
    pub(super) target_type_identity: CanonicalTypeIdentity,
    pub(super) declaration_span: Option<crate::compiler_frontend::source::SourceSpan>,
}

#[derive(Clone)]
struct StablePrivateTrait {
    identity: CanonicalTraitIdentity,
    name: String,
    canonical_path: PathId,
    source_file: PathId,
    declaration_span: Option<crate::compiler_frontend::source::SourceSpan>,
    requirements: Box<[StablePrivateTraitRequirement]>,
}

#[derive(Clone)]
struct StablePrivateTraitRequirement {
    name: String,
    receiver_mutable: bool,
    parameters: Box<[StablePrivateTraitParameter]>,
    returns: Box<[StablePrivateTraitReturn]>,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
}

#[derive(Clone)]
struct StablePrivateTraitParameter {
    name: PathId,
    value_mode: ValueMode,
    parameter_type: StableTraitTypeBlueprint,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
    contract: StablePrivateTraitParameterContract,
}

#[derive(Clone)]
enum StablePrivateTraitParameterContract {
    Value,
    Wire,
    Route {
        input: Option<StableTraitTypeBlueprint>,
    },
}

impl StablePrivateTraitParameterContract {
    /// Trait-type blueprint this contract contributes beyond the parameter's own type.
    fn input_blueprint(&self) -> Option<&StableTraitTypeBlueprint> {
        match self {
            StablePrivateTraitParameterContract::Route { input } => input.as_ref(),
            StablePrivateTraitParameterContract::Value
            | StablePrivateTraitParameterContract::Wire => None,
        }
    }
}

impl StablePrivateTraitParameter {
    /// Every trait-type blueprint this requirement parameter touches.
    ///
    /// WHAT: yields the parameter's own blueprint, then any contract-owned blueprint.
    /// WHY: the private-trait sidecar closure must retain a blueprint for every type the frozen
    ///      requirement names, including a Route input.
    fn semantic_type_blueprints(&self) -> impl Iterator<Item = &StableTraitTypeBlueprint> {
        std::iter::once(&self.parameter_type).chain(self.contract.input_blueprint())
    }
}

#[derive(Clone)]
struct StablePrivateTraitReturn {
    return_type: StableTraitTypeBlueprint,
    channel: ReturnChannel,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
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
    source_file: PathId,
    declaration_span: Option<crate::compiler_frontend::source::SourceSpan>,
    requirements: Box<[StablePrivateEvidenceRequirement]>,
}

#[derive(Clone)]
struct StablePrivateEvidenceRequirement {
    requirement_name: String,
    method_path: PathId,
    /// Concrete binding of `method_path`, so each environment that installs this row can call it.
    /// `None` for generic templates, which keep their own lane.
    callable: Option<StableCallableBinding>,
}
impl StablePrivateTrait {
    fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.canonical_path = remap.get(self.canonical_path);
        self.source_file = remap.get(self.source_file);
        for requirement in &mut self.requirements {
            requirement.remap_path_ids(remap);
        }
    }
}

impl StablePrivateTraitRequirement {
    fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        for parameter in &mut self.parameters {
            parameter.remap_path_ids(remap);
        }
    }
}

impl StablePrivateTraitParameter {
    fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.name = remap.get(self.name);
    }
}

impl StablePrivateEvidence {
    fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.source_file = remap.get(self.source_file);
        for requirement in &mut self.requirements {
            requirement.method_path = remap.get(requirement.method_path);
            if let Some(callable) = &mut requirement.callable {
                callable.remap_path_ids(remap);
            }
        }
    }
}

impl StableFunctionSignature {
    fn collect_nominal_identities(&self, identities: &mut FxHashSet<CanonicalTypeIdentity>) {
        for parameter in &self.parameters {
            for blueprint in parameter.semantic_type_blueprints() {
                blueprint.collect_nominal_identities(identities);
            }
            if let Some(default) = &parameter.folded_default {
                default.visit_type_identities(&mut |identity| {
                    collect_reachable_nominal_identities(identity, identities);
                });
            }
        }
        for returned in &self.returns {
            returned.collect_nominal_identities(identities);
        }
        if let Some(error_return) = &self.error_return {
            error_return.collect_nominal_identities(identities);
        }
    }
}
pub(super) fn stable_body_symbol_names(
    body: &GenericFunctionBody,
    string_table: &FrozenStringTable,
) -> Result<FxHashSet<String>, InfrastructureFailure> {
    let (source_owner, token_range, token_sequence) = body.canonical_view();
    let mut cursor = crate::compiler_frontend::ast::generic_functions::templates::body_cursor(
        source_owner,
        token_range,
        token_sequence,
        "generic body symbol closure",
    )?;
    let mut names = FxHashSet::default();
    while let Some(token) = cursor.advance() {
        let is_eof = token.is_eof();
        if token.tag() == TokenTag::SYMBOL
            && let Some(symbol) = token.string_id()
        {
            names.insert(string_table.resolve(symbol).to_owned());
        }
        if is_eof {
            break;
        }
    }
    Ok(names)
}

/// Collect every nominal identity one canonical type reaches, including generic-instance bases.
///
/// WHY: `CanonicalTypeIdentity::visit` yields an instance and its arguments, but reconstructing
/// `Box of I32` inside a generated sidecar also needs the blueprint of `Box` itself. Blueprint
/// type trees already record that base explicitly; canonical identities carry it on the instance.
pub(super) fn collect_reachable_nominal_identities(
    identity: &CanonicalTypeIdentity,
    identities: &mut FxHashSet<CanonicalTypeIdentity>,
) {
    identity.visit(&mut |nested| {
        identities.insert(nested.clone());

        match nested {
            CanonicalTypeIdentity::GenericInstance { base, .. } => {
                identities.insert(CanonicalTypeIdentity::SourceNominal(base.clone()));
            }
            CanonicalTypeIdentity::ModulePrivateGenericInstance { base, .. } => {
                identities.insert(CanonicalTypeIdentity::ModulePrivateNominal(base.clone()));
            }
            _ => {}
        }
    });
}
pub(super) fn install_private_semantic_closure(
    nominal_source: &GenericTemplateArtefact,
    context: &ModuleMaterialisationContext,
    environment: &mut AstModuleEnvironment,
    services: &mut GeneratedValueMaterialisationServices<'_>,
    template_ir_store: &Rc<RefCell<crate::compiler_frontend::ast::templates::tir::TemplateIrStore>>,
    string_table: &mut StringTable,
    path_fork: &mut crate::compiler_frontend::symbols::path_interner::PathInternerFork,
) -> Result<(), PreparedFailure> {
    let external_package_registry = services.external_registry;
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
                        name: parameter.name,
                        value_mode: parameter.value_mode,
                        type_id: intern_stable_trait_type(
                            &parameter.parameter_type,
                            this_type,
                            nominal_source,
                            &mut environment.type_environment,
                            external_package_registry,
                            template_ir_store,
                            string_table,
                            path_fork,
                        )?,
                        span: parameter.span,
                        contract: match &parameter.contract {
                            StablePrivateTraitParameterContract::Value => ParameterContract::Value,
                            StablePrivateTraitParameterContract::Wire => ParameterContract::Wire,
                            StablePrivateTraitParameterContract::Route { input } => {
                                ParameterContract::Route {
                                    input: input
                                        .as_ref()
                                        .map(|input| {
                                            intern_stable_trait_type(
                                                input,
                                                this_type,
                                                nominal_source,
                                                &mut environment.type_environment,
                                                external_package_registry,
                                                template_ir_store,
                                                string_table,
                                                path_fork,
                                            )
                                        })
                                        .transpose()?,
                                }
                            }
                        },
                    })
                })
                .collect::<Result<Vec<_>, PreparedFailure>>()?;
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
                            path_fork,
                        )?,
                        channel: returned.channel,
                        span: returned.span,
                    })
                })
                .collect::<Result<Vec<_>, PreparedFailure>>()?;
            requirements.push(ResolvedTraitRequirement {
                id: requirement_id,
                name: string_table.intern(&stable_requirement.name),
                receiver,
                parameters,
                returns,
                span: stable_requirement.span,
            });
        }
        let canonical_path = stable_trait.canonical_path;
        let source_file = stable_trait.source_file;
        let definition = ResolvedTraitDefinition {
            id: trait_id,
            name: string_table.intern(&stable_trait.name),
            canonical_path,
            source_file,
            this_type,
            requirements,
            declaration_span: stable_trait.declaration_span,
            visibility: TraitVisibility::Source { exported: false },
        };
        let lookups = Rc::get_mut(&mut environment.lookups)
            .expect("AST environment lookups are uniquely owned before materialisation");
        let trait_environment = Rc::make_mut(&mut lookups.trait_environment);
        if trait_environment.insert(definition).is_some() {
            return Err(InfrastructureFailure::internal(
                CompilerStage::GenericMaterialisation,
                "Private materialisation trait path was registered more than once",
            )
            .into());
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
            path_fork,
        )
        .map_err(|error| {
            const_store_error_into_prepared_failure(
                error,
                string_table,
                Some(&environment.type_environment),
            )
        })?;
        let trait_id = environment
            .lookups
            .trait_environment
            .id_for_canonical_identity(&stable_evidence.trait_identity)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::GenericMaterialisation,
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
                InfrastructureFailure::internal(
                    CompilerStage::GenericMaterialisation,
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
                        InfrastructureFailure::internal(
                            CompilerStage::GenericMaterialisation,
                            "Private materialisation evidence requirement is absent from its trait",
                        )
                    })?;
                Ok(TraitRequirementEvidence {
                    requirement_id: trait_requirement.id,
                    method_path: requirement.method_path,
                })
            })
            .collect::<Result<Vec<_>, InfrastructureFailure>>()?;
        // A private evidence row names a concrete method, which this generated environment needs
        // to call through the row. Several rows can name one method, so a path already present
        // is skipped.
        for requirement in &stable_evidence.requirements {
            let Some(callable) = &requirement.callable else {
                continue;
            };
            if environment
                .lookups
                .imported_functions_by_local_path
                .contains_key(&callable.local_path)
            {
                continue;
            }
            callable.install_requester_callable(
                environment,
                services,
                nominal_source,
                string_table,
                path_fork,
            )?;
        }
        let definition = TraitEvidenceDefinition {
            id: TraitEvidenceId(0),
            kind: TraitEvidenceKind::Canonical,
            target_type_id,
            trait_id,
            source_file: stable_evidence.source_file,
            declaration_span: stable_evidence.declaration_span,
            requirements,
        };
        let lookups = Rc::get_mut(&mut environment.lookups)
            .expect("AST environment lookups are uniquely owned before materialisation");
        Rc::make_mut(&mut lookups.trait_evidence_environment).insert_validated(definition);
    }
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "stable trait interning keeps the blueprint, this type, nominal source, mutable type environment, registry, template store, and mutable string/path forks as separate borrows"
)]
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
    path_fork: &mut crate::compiler_frontend::symbols::path_interner::PathInternerFork,
) -> Result<TypeId, PreparedFailure> {
    match blueprint {
        StableTraitTypeBlueprint::This => Ok(this_type),
        StableTraitTypeBlueprint::Type(blueprint) => intern_materialisation_type_blueprint(
            blueprint,
            &[this_type],
            nominal_source,
            type_environment,
            external_package_registry,
            string_table,
            path_fork,
        )
        .map_err(|error| {
            const_store_error_into_prepared_failure(error, string_table, Some(type_environment))
        }),
    }
}
impl ModuleMaterialisationCapture {
    /// Capture the union of local declarations that retained artefacts actually install.
    ///
    /// Folded values inline sibling constant values rather than retaining constant-path edges.
    /// Nominal dependencies still use the existing blueprint closure, while private traits and
    /// evidence retain their whole-module authority.
    pub(super) fn stable_semantic_closure(
        &self,
        const_values: &ConstValueStore,
        retained_local_paths: &FxHashSet<PathId>,
        callable_contracts: &[StableCallableBinding],
        resources: &ModuleResourceTable,
        path_fork: &PathInternerFork,
    ) -> Result<StableSemanticClosure, InfrastructureFailure> {
        let mut constants = Vec::new();
        for row in const_values.iter_module_constant_views() {
            let path = *row.path;
            let metadata = row.metadata;
            if !retained_local_paths.contains(&path)
                || self
                    .binding_environment
                    .imported_declarations_by_local_path
                    .contains_key(&path)
            {
                continue;
            }
            let type_identity = self.stable_type_identity(metadata.type_id)?;
            let value =
                self.stable_folded_value_at_path(const_values, &path, resources, path_fork)?;
            constants.push(StableLocalConstant {
                local_path: path,
                type_identity,
                value,
                span: metadata.span,
            });
        }
        constants.sort_by_key(|row| row.local_path);

        let mut aliases = self
            .resolved_type_aliases_by_path
            .iter()
            .filter(|(path, _)| {
                retained_local_paths.contains(*path)
                    && !self
                        .binding_environment
                        .imported_declarations_by_local_path
                        .contains_key(*path)
            })
            .map(|(path, alias)| {
                let target_type_identity = self.stable_alias_target_identity(path, alias)?;
                Ok(StableLocalAlias {
                    local_path: *path,
                    target_type_identity,
                    declaration_span: alias.declaration_span,
                })
            })
            .collect::<Result<Vec<_>, InfrastructureFailure>>()?;
        aliases.sort_by_key(|row| row.local_path);

        let traits = self.stable_private_traits()?;
        let evidence = self.stable_private_evidence(callable_contracts)?;

        Ok(StableSemanticClosure {
            constants: constants.into_boxed_slice(),
            aliases: aliases.into_boxed_slice(),
            traits,
            evidence,
        })
    }
    fn stable_private_traits(&self) -> Result<Box<[StablePrivateTrait]>, InfrastructureFailure> {
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
                                name: parameter.name,
                                value_mode: parameter.value_mode,
                                parameter_type: self.stable_trait_type(
                                    parameter.type_id,
                                    definition.this_type,
                                )?,
                                span: parameter.span,
                                contract: match parameter.contract {
                                    ParameterContract::Value => {
                                        StablePrivateTraitParameterContract::Value
                                    }
                                    ParameterContract::Wire => {
                                        StablePrivateTraitParameterContract::Wire
                                    }
                                    ParameterContract::Route { input } => {
                                        StablePrivateTraitParameterContract::Route {
                                            input: input
                                                .map(|type_id| {
                                                    self.stable_trait_type(
                                                        type_id,
                                                        definition.this_type,
                                                    )
                                                })
                                                .transpose()?,
                                        }
                                    }
                                },
                            })
                        })
                        .collect::<Result<Box<[_]>, InfrastructureFailure>>()?;
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
                                span: returned.span,
                            })
                        })
                        .collect::<Result<Box<[_]>, InfrastructureFailure>>()?;
                    let receiver_mutable = matches!(
                        requirement.receiver,
                        crate::compiler_frontend::traits::definitions::TraitReceiverRequirement::Mutable { .. }
                    );
                    Ok(StablePrivateTraitRequirement {
                        name: self.string_table.resolve(requirement.name).to_owned(),
                        receiver_mutable,
                        parameters,
                        returns,
                        span: requirement.span,
                    })
                })
                .collect::<Result<Box<[_]>, InfrastructureFailure>>()?;
            traits.push(StablePrivateTrait {
                identity,
                name: self.string_table.resolve(definition.name).to_owned(),
                canonical_path: definition.canonical_path,
                source_file: definition.source_file,
                declaration_span: definition.declaration_span,
                requirements,
            });
        }
        traits.sort_by(|left, right| left.identity.cmp(&right.identity));
        Ok(traits.into_boxed_slice())
    }

    fn stable_private_evidence(
        &self,
        callable_contracts: &[StableCallableBinding],
    ) -> Result<Box<[StablePrivateEvidence]>, InfrastructureFailure> {
        let mut evidence = Vec::new();
        for definition in self.trait_evidence_environment.canonical_evidence() {
            let target_type_identity = self.stable_type_identity(definition.target_type_id)?;
            let trait_identity = self
                .trait_environment
                .canonical_identity_for_id(definition.trait_id)
                .cloned()
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::GenericMaterialisation,
                        "Private materialisation evidence has no canonical trait identity",
                    )
                })?;
            let target_is_private = {
                let mut private = false;
                target_type_identity.visit(&mut |identity| {
                    private |= matches!(
                        identity,
                        CanonicalTypeIdentity::ModulePrivateNominal(_)
                            | CanonicalTypeIdentity::ModulePrivateGenericInstance { .. }
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
                        InfrastructureFailure::internal(
                            CompilerStage::GenericMaterialisation,
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
                            InfrastructureFailure::internal(
                                CompilerStage::GenericMaterialisation,
                                "Private materialisation evidence has no trait requirement",
                            )
                        })?;
                    Ok(StablePrivateEvidenceRequirement {
                        requirement_name: self
                            .string_table
                            .resolve(trait_requirement.name)
                            .to_owned(),
                        method_path: requirement.method_path,
                        // A miss means a generic template, or a method with no contract, so the
                        // row has no callable; `callable_contracts` is sorted by `local_path`.
                        callable: callable_contracts
                            .binary_search_by_key(&requirement.method_path, |binding| {
                                binding.local_path
                            })
                            .ok()
                            .map(|index| callable_contracts[index].clone()),
                    })
                })
                .collect::<Result<Box<[_]>, InfrastructureFailure>>()?;
            evidence.push(StablePrivateEvidence {
                target_type_identity,
                trait_identity,
                source_file: definition.source_file,
                declaration_span: definition.declaration_span,
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
    ) -> Result<StableTraitTypeBlueprint, InfrastructureFailure> {
        if type_id == this_type {
            Ok(StableTraitTypeBlueprint::This)
        } else {
            let this_parameter_id = match self.type_environment.get(this_type) {
                Some(TypeDefinition::GenericParameter(parameter)) => parameter.id,
                _ => {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::GenericMaterialisation,
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
        const_values: &ConstValueStore,
        selected_paths: &FxHashSet<PathId>,
    ) -> Box<[PathId]> {
        let mut paths = selected_paths
            .iter()
            .filter(|path| {
                (const_values.value_for_path(path).is_some()
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
            .copied()
            .collect::<Vec<_>>();
        paths.sort();
        paths.into_boxed_slice()
    }
    pub(super) fn selected_visible_paths(
        &self,
        source_file: &PathId,
        referenced_names: &FxHashSet<String>,
        path_fork: &PathInternerFork,
    ) -> Result<FxHashSet<PathId>, InfrastructureFailure> {
        let visibility = self.binding_environment.visibility_for(source_file)?;
        let mut selected = visibility
            .visible_source_names
            .iter()
            .chain(visibility.visible_type_alias_names.iter())
            .chain(visibility.visible_trait_names.iter())
            .filter(|(name, _)| referenced_names.contains(self.string_table.resolve(**name)))
            .map(|(_, target)| *target.local_path())
            .collect::<FxHashSet<_>>();
        for (name, methods) in &visibility.visible_receiver_methods {
            if referenced_names.contains(self.string_table.resolve(*name)) {
                selected.extend(methods.iter().map(|method| *method.target.local_path()));
            }
        }
        // WHY: a value-carried method reaches the body through its receiver value, so a referenced
        // name selects it whether or not the name is visible in the body's file.
        selected.extend(self.value_carried_paths_for_names(referenced_names));
        for (name, record) in &visibility.visible_namespace_records {
            if referenced_names.contains(self.string_table.resolve(*name)) {
                collect_namespace_source_paths(record, &mut selected);
            }
        }
        // WHY: an imported receiver method is projected under the authored binding path of its
        // nominal (`child(nominal binding, method name)`). A body that reaches the method must
        // therefore also capture that nominal binding; otherwise the restored body projects the
        // nominal at its deterministic path and the captured method path would not resolve.
        let nominal_paths = selected
            .iter()
            .filter(|path| {
                self.resolved_function_signatures_by_path
                    .get(*path)
                    .is_some_and(|resolved| resolved.receiver.is_some())
            })
            .filter_map(|path| path_fork.try_parent(*path))
            .filter(|parent| {
                self.binding_environment
                    .imported_declarations_by_local_path
                    .get(parent)
                    .is_some_and(|origin| matches!(origin, OriginDeclarationId::Type(_)))
            })
            .collect::<Vec<_>>();
        selected.extend(nominal_paths);
        Ok(selected)
    }

    /// Function paths of the value-carried receiver methods that a body's referenced names select.
    ///
    /// WHAT: a referenced name selects each value-carried method with that name, whether or not the
    /// name is visible in the body's file.
    /// WHY: a value-carried method reaches a body through its receiver value, so file visibility
    /// must not decide whether it is captured.
    pub(super) fn value_carried_paths_for_names(
        &self,
        referenced_names: &FxHashSet<String>,
    ) -> Vec<PathId> {
        self.receiver_methods.value_carried_paths_named(|name| {
            referenced_names.contains(self.string_table.resolve(name))
        })
    }

    /// Stable target of a function reached through a visible name or a value-carried method.
    ///
    /// WHAT: a source target that carries its own identity wins; otherwise the path decides.
    /// WHY: a visible-name entry already carries its target, while a value-carried entry is known
    /// only by its path.
    pub(super) fn stable_target_for_path(
        &self,
        target: &SourceFunctionTarget,
    ) -> Option<StableFunctionTarget> {
        StableFunctionTarget::capture(target)
            .or_else(|| self.stable_target_for_local_path(target.local_path()))
    }

    /// Stable target of the function at one canonical path.
    ///
    /// WHAT: a retained executable contract wins; otherwise a generic template resolves through its
    /// declaration identity.
    /// WHY: the contract owns the identity of every concrete executable, including same-module
    /// methods, so it is consulted before the template.
    pub(super) fn stable_target_for_local_path(
        &self,
        local_path: &PathId,
    ) -> Option<StableFunctionTarget> {
        if let Some(stable) = self
            .imported_functions_by_local_path
            .get(local_path)
            .and_then(|contract| StableFunctionTarget::capture(&contract.target))
        {
            return Some(stable);
        }
        self.generic_function_templates_by_path
            .get(local_path)
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
        selected_paths: &FxHashSet<PathId>,
        declarations: &[PublicDeclarationRecord],
    ) -> Box<[StableDeclarationBinding]> {
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
                    local_path: *path,
                    origin: origin.clone(),
                });
                continue;
            }
            let origin = self.public_origin_for_path(path);
            if let Some(origin) = origin
                && declarations.iter().any(|record| record.origin == origin)
            {
                bindings.push(StableDeclarationBinding {
                    local_path: *path,
                    origin,
                });
            }
        }
        bindings.sort_by_key(|binding| binding.local_path);
        bindings.into_boxed_slice()
    }

    fn public_origin_for_path(&self, path: &PathId) -> Option<OriginDeclarationId> {
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
        const_values: &ConstValueStore,
        selected_paths: &FxHashSet<PathId>,
        resources: &ModuleResourceTable,
        path_fork: &PathInternerFork,
    ) -> Result<Box<[StableCallableBinding]>, InfrastructureFailure> {
        let mut callables = Vec::new();
        for path in selected_paths {
            if self.generic_function_templates_by_path.contains_key(path) {
                continue;
            }
            let Some(contract) = self.imported_functions_by_local_path.get(path) else {
                continue;
            };
            let Some(target) = StableFunctionTarget::capture(&contract.target) else {
                continue;
            };
            let Some(resolved) = self.resolved_function_signatures_by_path.get(path) else {
                continue;
            };
            callables.push(StableCallableBinding {
                local_path: *path,
                target,
                signature: self.stable_function_signature(
                    const_values,
                    &resolved.signature,
                    &FxHashMap::default(),
                    resources,
                    path_fork,
                )?,
                summary: contract.summary.clone(),
            });
        }
        callables.sort_by_key(|binding| binding.local_path);
        Ok(callables.into_boxed_slice())
    }
    /// Collect the nominal blueprints one template's generated sidecar can need.
    ///
    /// WHAT: gathers every canonical identity the artefact reaches (retained callable and receiver
    /// signatures and defaults, selected nominals, selected constants and their folded values,
    /// selected alias targets, private traits and retained evidence) and keeps each blueprint.
    /// WHY: this is the single owner of blueprint closure collection. Every canonical identity
    /// source runs through [`collect_reachable_nominal_identities`], because a concrete
    /// `Box of I32` identity also needs the `Box` blueprint and canonical identities carry that
    /// base on the instance rather than as a nested identity.
    pub(super) fn stable_nominal_blueprints<'a>(
        &self,
        const_values: &ConstValueStore,
        selected_paths: &FxHashSet<PathId>,
        signatures: impl IntoIterator<Item = &'a StableFunctionSignature>,
        semantic_closure: &StableSemanticClosure,
        resources: &ModuleResourceTable,
        path_fork: &PathInternerFork,
    ) -> FxHashMap<CanonicalTypeIdentity, NominalMaterialisationBlueprint> {
        let mut identities = FxHashSet::default();
        for signature in signatures {
            signature.collect_nominal_identities(&mut identities);
        }
        for path in selected_paths {
            if let Some(type_id) = self.nominal_type_ids_by_path.get(path)
                && let Some(identity) = self
                    .type_environment
                    .canonical_identity_for_type_id(*type_id)
            {
                collect_reachable_nominal_identities(identity, &mut identities);
            }
            if let Some(value_id) = const_values.value_for_path(path) {
                if let Some(metadata) = const_values.metadata(value_id)
                    && let Ok(identity) = self.stable_type_identity(metadata.type_id)
                {
                    collect_reachable_nominal_identities(&identity, &mut identities);
                }
                if let Ok(value) =
                    self.stable_folded_value_at_path(const_values, path, resources, path_fork)
                {
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
                    for blueprint in parameter.semantic_type_blueprints() {
                        if let StableTraitTypeBlueprint::Type(blueprint) = blueprint {
                            blueprint.collect_nominal_identities(&mut identities);
                        }
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
            // Evidence methods can name nominals that no retained body mentions, so their
            // signatures contribute blueprints the same way the trait requirements do.
            for requirement in &evidence.requirements {
                if let Some(callable) = &requirement.callable {
                    callable
                        .signature
                        .collect_nominal_identities(&mut identities);
                }
            }
        }
        let mut blueprints = FxHashMap::default();
        let mut initial_identities = identities.into_iter().collect::<Vec<_>>();
        initial_identities.sort();
        for identity in initial_identities {
            if let Some(blueprint) = self.nominal_blueprints.get(&identity) {
                blueprints.insert(identity, blueprint.clone());
            }
        }

        // A retained nominal can mention another nominal only in its member blueprint. Keep
        // following those member shapes against the module-wide source map until the closure is
        // stable. Sort each frontier so retention does not depend on FxHashMap/FxHashSet order.
        loop {
            let mut discovered = FxHashSet::default();
            let mut retained_identities = blueprints.keys().cloned().collect::<Vec<_>>();
            retained_identities.sort();
            for identity in retained_identities {
                let blueprint = &blueprints[&identity];
                match &blueprint.definition {
                    NominalMaterialisationDefinition::Struct { fields, .. } => {
                        for field in fields {
                            field.field_type.collect_nominal_identities(&mut discovered);
                            if let Some(default) = &field.folded_default {
                                default.visit_type_identities(&mut |identity| {
                                    collect_reachable_nominal_identities(identity, &mut discovered);
                                });
                            }
                        }
                    }
                    NominalMaterialisationDefinition::Choice { variants } => {
                        for variant in variants {
                            for field in &variant.payload_fields {
                                field.field_type.collect_nominal_identities(&mut discovered);
                                if let Some(default) = &field.folded_default {
                                    default.visit_type_identities(&mut |identity| {
                                        collect_reachable_nominal_identities(
                                            identity,
                                            &mut discovered,
                                        );
                                    });
                                }
                            }
                        }
                    }
                }
            }
            let mut frontier = discovered
                .into_iter()
                .filter(|identity| !blueprints.contains_key(identity))
                .collect::<Vec<_>>();
            frontier.sort();
            if frontier.is_empty() {
                break;
            }
            let mut retained = false;
            for identity in frontier {
                if let Some(blueprint) = self.nominal_blueprints.get(&identity) {
                    blueprints.insert(identity, blueprint.clone());
                    retained = true;
                }
            }
            if !retained {
                break;
            }
        }
        blueprints
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
        path: &PathId,
        semantic_closure: &StableSemanticClosure,
        identities: &mut FxHashSet<CanonicalTypeIdentity>,
    ) {
        if !self.resolved_type_aliases_by_path.contains_key(path) {
            return;
        }

        let components = *path;
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

    /// Capture all existing lookup aliases for the retained canonical nominal closure.
    ///
    /// Visibility selects semantic dependencies, not the complete alias index. Signature-only
    /// imports and inherited canonical paths still belong to the generated publication domain.
    pub(super) fn stable_nominal_bindings(
        &self,
        blueprints: &FxHashMap<CanonicalTypeIdentity, NominalMaterialisationBlueprint>,
    ) -> Box<[StableNominalBinding]> {
        let mut bindings = self
            .type_environment
            .nominal_path_bindings()
            .filter_map(|(path, type_id)| {
                let identity = self
                    .type_environment
                    .canonical_identity_for_type_id(type_id)?;
                blueprints
                    .contains_key(identity)
                    .then(|| StableNominalBinding {
                        local_path: path,
                        identity: identity.clone(),
                    })
            })
            .collect::<Vec<_>>();
        bindings.sort_by_key(|binding| binding.local_path);
        bindings.dedup_by_key(|binding| binding.local_path);
        bindings.into_boxed_slice()
    }
}
