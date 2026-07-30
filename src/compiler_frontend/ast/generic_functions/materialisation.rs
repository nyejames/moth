//! Immutable declaring-module context for generated generic functions.
//!
//! WHAT: retains the resolved, TIR-free semantic tables required to reparse one validated
//! generic body after a consumer emits a concrete request.
//! WHY: generated sidecars must compile in an independent type environment without reopening
//! source or borrowing the mutable requester or declaring-module environment.

use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::arena::FrontendArenaCapacityEstimate;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration};
use crate::compiler_frontend::ast::module_ast::build_context::AstPhaseContext;
use crate::compiler_frontend::ast::module_ast::emission::AstEmitter;
use crate::compiler_frontend::ast::module_ast::environment::{
    AstModuleEnvironment, AstModuleLookups, DeclarationSemanticTable, TopLevelDeclarationTable,
};
use crate::compiler_frontend::ast::module_ast::finalization::AstFinalizer;
use crate::compiler_frontend::ast::module_ast::scope_context::ReceiverMethodCatalog;
use crate::compiler_frontend::ast::type_resolution::{
    ResolvedFunctionSignature, ResolvedTypeAnnotation,
};
use crate::compiler_frontend::ast::{AstBuildContext, AstBuildResult, AstImportedFunctionContract};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTraitIdentity, CanonicalTypeIdentity,
    ModulePrivateNominalIdentity, ModulePrivateTraitIdentity,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, StructTypeDefinition, TypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::NominalTypeId;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariant;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::import_environment::{
    HeaderImportEnvironment, SourceFunctionTarget,
};
use crate::compiler_frontend::headers::module_symbols::{
    GenericDeclarationMetadata, ModuleSymbols,
};
use crate::compiler_frontend::paths::path_format::PathStringFormatConfig;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallMutationEffect, PublicCallParameterAccess,
    PublicCallParameterSummary, PublicCallReactiveEffect, PublicCallSummary,
    PublicCallTransferEffect, PublicCallTransferEligibility,
};
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, ModulePrivateExecutableCategory, ModulePrivateExecutableIdentity,
    ModuleRootRole, OriginFunctionId, OriginTraitId, OriginTypeCategory, OriginTypeId,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringIdRemap, StringTable};
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::environment::{
    TraitEvidenceKind, TraitRequirementEvidence,
};
use crate::compiler_frontend::traits::evidence::{
    TraitEvidenceDefinition, TraitEvidenceEnvironment,
};
use crate::compiler_frontend::traits::ids::TraitEvidenceId;
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use super::{
    GenericFunctionInstanceKey, GenericFunctionInstantiationRequest, GenericFunctionTemplate,
};
use crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity;

pub(crate) struct MaterialisedGenericAst {
    pub(crate) build_result: AstBuildResult,
    pub(crate) string_table: StringTable,
    pub(crate) instance_path: InternedPath,
}

/// Self-contained immutable semantic context owned by one successful declaring module.
#[derive(Clone)]
pub(crate) struct ModuleMaterialisationContext {
    pub(crate) string_table: StringTable,
    pub(crate) entry_dir: InternedPath,
    pub(crate) type_environment: TypeEnvironment,
    pub(crate) declaration_table: TopLevelDeclarationTable,
    pub(crate) import_environment: HeaderImportEnvironment,
    pub(crate) imported_functions_by_local_path:
        FxHashMap<InternedPath, AstImportedFunctionContract>,
    pub(crate) imported_struct_definitions:
        Vec<crate::compiler_frontend::ast::AstImportedStructDefinition>,
    pub(crate) imported_choice_definitions: Vec<crate::compiler_frontend::ast::AstChoiceDefinition>,
    pub(crate) module_constants: Vec<Declaration>,
    pub(crate) builtin_struct_ast_nodes: Vec<AstNode>,
    pub(crate) resolved_struct_fields_by_path: FxHashMap<InternedPath, Vec<Declaration>>,
    pub(crate) resolved_function_signatures_by_path:
        FxHashMap<InternedPath, ResolvedFunctionSignature>,
    pub(crate) generic_function_templates_by_path: FxHashMap<InternedPath, GenericFunctionTemplate>,
    pub(crate) resolved_type_aliases_by_path: FxHashMap<InternedPath, ResolvedTypeAnnotation>,
    pub(crate) choice_variant_shells_by_path: FxHashMap<InternedPath, Vec<ChoiceVariant>>,
    pub(crate) declaration_semantics: DeclarationSemanticTable,
    pub(crate) generic_declarations_by_path: FxHashMap<InternedPath, GenericDeclarationMetadata>,
    pub(crate) nominal_type_ids_by_path: FxHashMap<InternedPath, TypeId>,
    public_trait_paths: Vec<InternedPath>,
    source_nominals_by_origin: FxHashMap<OriginTypeId, (InternedPath, TypeId)>,
    private_nominals_by_identity: FxHashMap<ModulePrivateNominalIdentity, (InternedPath, TypeId)>,
    pub(crate) receiver_methods: ReceiverMethodCatalog,
    pub(crate) trait_environment: TraitEnvironment,
    pub(crate) trait_evidence_environment: TraitEvidenceEnvironment,
    pub(crate) external_package_registry: Arc<ExternalPackageRegistry>,
    pub(crate) style_directives: StyleDirectiveRegistry,
    pub(crate) build_profile: FrontendBuildProfile,
    pub(crate) project_path_resolver: Option<ProjectPathResolver>,
    pub(crate) path_format_config: PathStringFormatConfig,
    pub(crate) template_const_loop_iteration_limit: usize,
    pub(crate) capacity_estimate: FrontendArenaCapacityEstimate,
}

impl ModuleMaterialisationContext {
    /// Freeze stable targets for every concrete executable visible to generated bodies.
    ///
    /// Public functions retain their interface identity. Concrete local helpers receive a
    /// distinct artefact-scoped identity and are projected as imported contracts when a generic
    /// body is materialised in an independent sidecar.
    pub(crate) fn install_concrete_executable_contracts(
        &mut self,
        module_origin: &crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity,
        public_origins_by_path: &FxHashMap<InternedPath, OriginFunctionId>,
    ) -> Result<Vec<(InternedPath, ModulePrivateExecutableIdentity)>, CompilerError> {
        self.install_private_semantic_identities(module_origin)?;

        let generic_paths = self
            .generic_function_templates_by_path
            .keys()
            .cloned()
            .collect::<Vec<_>>();

        for path in generic_paths {
            let expected_identity = if let Some(origin) = public_origins_by_path.get(&path) {
                GeneratedDeclarationIdentity::Public(origin.clone())
            } else if let Some(existing_identity) = self
                .generic_function_templates_by_path
                .get(&path)
                .and_then(|template| template.declaration_identity.clone())
            {
                existing_identity
            } else {
                let resolved = self
                    .resolved_function_signatures_by_path
                    .get(&path)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Local generic template has no resolved function signature",
                        )
                    })?;
                GeneratedDeclarationIdentity::ModulePrivate(self.private_executable_identity(
                    module_origin,
                    &path,
                    resolved,
                    ModulePrivateExecutableCategory::GenericFunction,
                )?)
            };

            let template = self
                .generic_function_templates_by_path
                .get_mut(&path)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Generic template disappeared while installing declaration identity",
                    )
                })?;
            if let Some(existing_identity) = template.declaration_identity.as_ref()
                && existing_identity != &expected_identity
            {
                return Err(CompilerError::compiler_error(
                    "Generic template declaration identity disagrees with its callable origin",
                ));
            }
            template.declaration_identity = Some(expected_identity);
        }

        let signatures = self
            .resolved_function_signatures_by_path
            .iter()
            .map(|(path, resolved)| (path.clone(), resolved.clone()))
            .collect::<Vec<_>>();
        let mut private_executables = Vec::new();

        for (path, resolved) in signatures {
            if self.imported_functions_by_local_path.contains_key(&path)
                || self.generic_function_templates_by_path.contains_key(&path)
            {
                continue;
            }

            let target = if let Some(origin) = public_origins_by_path.get(&path) {
                SourceFunctionTarget::Imported {
                    origin: origin.clone(),
                    local_path: path.clone(),
                }
            } else {
                let category = if resolved.receiver.is_some() {
                    ModulePrivateExecutableCategory::ReceiverMethod
                } else {
                    ModulePrivateExecutableCategory::FreeFunction
                };
                let identity =
                    self.private_executable_identity(module_origin, &path, &resolved, category)?;
                private_executables.push((path.clone(), identity.clone()));
                SourceFunctionTarget::ModulePrivate {
                    identity,
                    local_path: path.clone(),
                }
            };

            let fallible_carrier_type_id =
                fallible_carrier_for_signature(&resolved.signature, &mut self.type_environment);
            self.imported_functions_by_local_path.insert(
                path,
                AstImportedFunctionContract {
                    target,
                    summary: bootstrap_call_summary_from_signature(&resolved.signature),
                    fallible_carrier_type_id,
                },
            );
        }

        Ok(private_executables)
    }

    fn install_private_semantic_identities(
        &mut self,
        module_origin: &crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity,
    ) -> Result<(), CompilerError> {
        let nominal_types = self
            .nominal_type_ids_by_path
            .iter()
            .map(|(path, type_id)| (path.clone(), *type_id))
            .collect::<Vec<_>>();
        for (path, type_id) in nominal_types {
            if self
                .type_environment
                .canonical_identity_for_type_id(type_id)
                .is_some()
            {
                continue;
            }
            let category = match self.type_environment.get(type_id) {
                Some(TypeDefinition::Struct(_)) => OriginTypeCategory::Struct,
                Some(TypeDefinition::Choice(_)) => OriginTypeCategory::Choice,
                _ => continue,
            };
            let identity = ModulePrivateNominalIdentity::new(
                module_origin.clone(),
                path.to_string(&self.string_table),
                category,
            );
            self.type_environment.register_canonical_identity(
                CanonicalTypeIdentity::ModulePrivateNominal(identity.clone()),
                type_id,
            )?;
            self.private_nominals_by_identity
                .insert(identity, (path, type_id));
        }

        let private_traits = self
            .public_trait_paths
            .iter()
            .map(|path| {
                let trait_id = self.trait_environment.id_for_path(path).ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Public materialisation trait path has no resolved trait definition",
                    )
                })?;
                let defining_name = path.name().ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Public materialisation trait path has no defining name",
                    )
                })?;
                Ok((
                    trait_id,
                    self.string_table.resolve(defining_name).to_owned(),
                ))
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;
        for (trait_id, defining_name) in private_traits {
            if self
                .trait_environment
                .canonical_identity_for_id(trait_id)
                .is_none()
            {
                self.trait_environment.register_canonical_identity(
                    CanonicalTraitIdentity::Source(OriginTraitId::new(
                        module_origin.clone(),
                        defining_name,
                    )),
                    trait_id,
                )?;
            }
        }

        let private_traits = self
            .trait_environment
            .definitions()
            .filter(|definition| {
                self.trait_environment
                    .canonical_identity_for_id(definition.id)
                    .is_none()
            })
            .map(|definition| {
                (
                    definition.id,
                    definition.canonical_path.to_string(&self.string_table),
                )
            })
            .collect::<Vec<_>>();
        for (trait_id, defining_path) in private_traits {
            self.trait_environment.register_canonical_identity(
                CanonicalTraitIdentity::ModulePrivate(ModulePrivateTraitIdentity::new(
                    module_origin.clone(),
                    defining_path,
                )),
                trait_id,
            )?;
        }

        Ok(())
    }

    fn private_executable_identity(
        &self,
        module_origin: &crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity,
        path: &InternedPath,
        resolved: &ResolvedFunctionSignature,
        category: ModulePrivateExecutableCategory,
    ) -> Result<ModulePrivateExecutableIdentity, CompilerError> {
        let receiver_path = match resolved.receiver.as_ref() {
            Some(ReceiverKey::Struct(path) | ReceiverKey::Choice(path)) => {
                Some(path.to_string(&self.string_table))
            }
            Some(ReceiverKey::External(_) | ReceiverKey::BuiltinScalar(_)) => {
                return Err(CompilerError::compiler_error(
                    "Module-private source receiver method has a non-source receiver",
                ));
            }
            None => None,
        };
        let name = path
            .name()
            .map(|name| self.string_table.resolve(name).to_owned())
            .ok_or_else(|| {
                CompilerError::compiler_error("Module-private executable path has no defining name")
            })?;
        let declaring_source = path
            .parent()
            .unwrap_or_else(InternedPath::new)
            .to_string(&self.string_table);

        Ok(ModulePrivateExecutableIdentity::new(
            module_origin.clone(),
            declaring_source,
            category,
            name,
            receiver_path,
        ))
    }

    pub(crate) fn from_environment(
        lookups: &AstModuleLookups,
        type_environment: &TypeEnvironment,
        public_trait_roots: &[crate::compiler_frontend::ast::module_ast::environment::ResolvedPublicTraitRoot],
        entry_dir: InternedPath,
        string_table: &StringTable,
        template_const_loop_iteration_limit: usize,
        capacity_estimate: FrontendArenaCapacityEstimate,
    ) -> Self {
        let source_nominals_by_origin = lookups
            .nominal_type_ids_by_path
            .iter()
            .filter_map(|(path, type_id)| {
                let CanonicalTypeIdentity::SourceNominal(origin) =
                    type_environment.canonical_identity_for_type_id(*type_id)?
                else {
                    return None;
                };
                Some((origin.clone(), (path.clone(), *type_id)))
            })
            .collect();
        let private_nominals_by_identity = lookups
            .nominal_type_ids_by_path
            .iter()
            .filter_map(|(path, type_id)| {
                let CanonicalTypeIdentity::ModulePrivateNominal(identity) =
                    type_environment.canonical_identity_for_type_id(*type_id)?
                else {
                    return None;
                };
                Some((identity.clone(), (path.clone(), *type_id)))
            })
            .collect();
        Self {
            string_table: string_table.clone(),
            entry_dir,
            type_environment: type_environment.clone(),
            declaration_table: (*lookups.declaration_table).clone(),
            import_environment: lookups.import_environment.clone(),
            imported_functions_by_local_path: lookups.imported_functions_by_local_path.clone(),
            imported_struct_definitions: lookups.imported_struct_definitions.clone(),
            imported_choice_definitions: lookups.imported_choice_definitions.clone(),
            module_constants: lookups.module_constants.clone(),
            builtin_struct_ast_nodes: lookups.builtin_struct_ast_nodes.clone(),
            resolved_struct_fields_by_path: (*lookups.resolved_struct_fields_by_path).clone(),
            resolved_function_signatures_by_path: (*lookups.resolved_function_signatures_by_path)
                .clone(),
            generic_function_templates_by_path: lookups.generic_function_templates_by_path.clone(),
            resolved_type_aliases_by_path: (*lookups.resolved_type_aliases_by_path).clone(),
            choice_variant_shells_by_path: (*lookups.choice_variant_shells_by_path).clone(),
            declaration_semantics: (*lookups.declaration_semantics).clone(),
            generic_declarations_by_path: (*lookups.generic_declarations_by_path).clone(),
            nominal_type_ids_by_path: (*lookups.nominal_type_ids_by_path).clone(),
            public_trait_paths: public_trait_roots
                .iter()
                .map(|root| root.canonical_path.clone())
                .collect(),
            source_nominals_by_origin,
            private_nominals_by_identity,
            receiver_methods: (*lookups.receiver_methods).clone(),
            trait_environment: (*lookups.trait_environment).clone(),
            trait_evidence_environment: (*lookups.trait_evidence_environment).clone(),
            external_package_registry: Arc::clone(&lookups.external_package_registry),
            style_directives: lookups.style_directives.clone(),
            build_profile: lookups.build_profile,
            project_path_resolver: lookups.project_path_resolver.clone(),
            path_format_config: lookups.path_format_config.clone(),
            template_const_loop_iteration_limit,
            capacity_estimate,
        }
    }

    pub(crate) fn build_environment(&self) -> AstModuleEnvironment {
        let lookups = AstModuleLookups {
            module_symbols: ModuleSymbols::empty(),
            import_environment: self.import_environment.clone(),
            warnings: Vec::new(),
            declaration_table: Rc::new(self.declaration_table.clone()),
            imported_functions_by_local_path: self.imported_functions_by_local_path.clone(),
            imported_struct_definitions: self.imported_struct_definitions.clone(),
            imported_choice_definitions: self.imported_choice_definitions.clone(),
            module_constants: self.module_constants.clone(),
            rendered_path_usages: Rc::new(RefCell::new(Vec::new())),
            builtin_struct_ast_nodes: self.builtin_struct_ast_nodes.clone(),
            resolved_struct_fields_by_path: Rc::new(self.resolved_struct_fields_by_path.clone()),
            resolved_function_signatures_by_path: Rc::new(
                self.resolved_function_signatures_by_path.clone(),
            ),
            generic_function_templates_by_path: self.generic_function_templates_by_path.clone(),
            resolved_type_aliases_by_path: Rc::new(self.resolved_type_aliases_by_path.clone()),
            choice_variant_shells_by_path: Rc::new(self.choice_variant_shells_by_path.clone()),
            declaration_semantics: Rc::new(self.declaration_semantics.clone()),
            receiver_methods: Rc::new(self.receiver_methods.clone()),
            trait_environment: Rc::new(self.trait_environment.clone()),
            trait_evidence_environment: Rc::new(self.trait_evidence_environment.clone()),
            generic_declarations_by_path: Rc::new(self.generic_declarations_by_path.clone()),
            nominal_type_ids_by_path: Rc::new(self.nominal_type_ids_by_path.clone()),
            external_package_registry: Arc::clone(&self.external_package_registry),
            style_directives: self.style_directives.clone(),
            build_profile: self.build_profile,
            project_path_resolver: self.project_path_resolver.clone(),
            path_format_config: self.path_format_config.clone(),
        };

        AstModuleEnvironment {
            lookups: Rc::new(lookups),
            type_environment: self.type_environment.clone(),
            resolved_public_type_roots: Default::default(),
            resolved_public_trait_roots: Vec::new(),
        }
    }

    pub(crate) fn generic_function_templates(
        &self,
    ) -> &FxHashMap<InternedPath, GenericFunctionTemplate> {
        &self.generic_function_templates_by_path
    }

    pub(crate) fn generic_function_templates_mut(
        &mut self,
    ) -> &mut FxHashMap<InternedPath, GenericFunctionTemplate> {
        &mut self.generic_function_templates_by_path
    }

    pub(crate) fn trait_environment(&self) -> &TraitEnvironment {
        &self.trait_environment
    }

    pub(crate) fn trait_evidence_environment(&self) -> &TraitEvidenceEnvironment {
        &self.trait_evidence_environment
    }

    pub(crate) fn template_for_identity(
        &self,
        identity: &GeneratedDeclarationIdentity,
    ) -> Option<&GenericFunctionTemplate> {
        self.generic_function_templates_by_path
            .values()
            .find(|template| {
                template.declaration_identity.as_ref() == Some(identity)
                    && template.body_tokens.is_some()
            })
    }

    pub(crate) fn materialise_ast(
        &self,
        identity: &GeneratedFunctionIdentity,
        requester_context: &ModuleMaterialisationContext,
        requester_call_location: &crate::compiler_frontend::tokenizer::tokens::SourceLocation,
    ) -> Result<MaterialisedGenericAst, CompilerMessages> {
        let template = self
            .template_for_identity(identity.declaration())
            .ok_or_else(|| {
                CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(
                        "Generated request has no retained declaring-module generic template",
                    ),
                    &self.string_table,
                )
            })?;
        let mut string_table = self.string_table.clone();
        let requester_string_remap = string_table.merge_from(&requester_context.string_table);
        let mut call_location = requester_call_location.clone();
        call_location.remap_string_ids(&requester_string_remap);
        let mut environment = self.build_environment();
        let mut type_arguments = Vec::with_capacity(identity.type_arguments().len());
        for canonical_identity in identity.type_arguments() {
            let type_id = intern_generated_canonical_type(
                canonical_identity,
                &mut environment.type_environment,
                self.external_package_registry.as_ref(),
                requester_context,
                &requester_string_remap,
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, &self.string_table))?;
            type_arguments.push(type_id);
        }
        install_generated_request_evidence(
            identity,
            requester_context,
            &requester_string_remap,
            &mut environment,
            &string_table,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, &self.string_table))?;

        let instance_path = template
            .function_path
            .join_str("__generated_instance", &mut string_table);
        let request = GenericFunctionInstantiationRequest {
            declaration_identity: Some(identity.declaration().clone()),
            evidence: Box::new([]),
            key: GenericFunctionInstanceKey {
                function_path: template.function_path.clone(),
                type_arguments: type_arguments.into_boxed_slice(),
            },
            instance_path: instance_path.clone(),
            call_location,
        };
        let build_context = AstBuildContext {
            external_package_registry: Arc::clone(&self.external_package_registry),
            style_directives: &self.style_directives,
            string_table: &mut string_table,
            entry_dir: self.entry_dir.clone(),
            root_role: ModuleRootRole::Support,
            build_profile: self.build_profile,
            project_path_resolver: self.project_path_resolver.clone(),
            path_format_config: self.path_format_config.clone(),
            template_const_loop_iteration_limit: self.template_const_loop_iteration_limit,
            capacity_estimate: self.capacity_estimate,
        };
        let (phase_context, string_table_ref) = AstPhaseContext::from_build_context(build_context);
        let emitted = AstEmitter::new(&phase_context, &mut environment, 1)
            .emit_generated_request(request, string_table_ref)?;

        let build_result = AstFinalizer::new(&phase_context, environment).finalize(
            emitted,
            &[],
            string_table_ref,
        )?;
        Ok(MaterialisedGenericAst {
            build_result,
            string_table,
            instance_path,
        })
    }
}

fn intern_generated_canonical_type(
    identity: &CanonicalTypeIdentity,
    type_environment: &mut TypeEnvironment,
    external_registry: &ExternalPackageRegistry,
    requester_context: &ModuleMaterialisationContext,
    requester_string_remap: &StringIdRemap,
) -> Result<TypeId, CompilerError> {
    if let Some(type_id) = type_environment.type_id_for_canonical_identity(identity) {
        return Ok(type_id);
    }

    let type_id = match identity {
        CanonicalTypeIdentity::Builtin(builtin) => match builtin {
            CanonicalBuiltinType::Bool => builtin_type_ids::BOOL,
            CanonicalBuiltinType::Int => builtin_type_ids::INT,
            CanonicalBuiltinType::Float => builtin_type_ids::FLOAT,
            CanonicalBuiltinType::Decimal => builtin_type_ids::DECIMAL,
            CanonicalBuiltinType::String => builtin_type_ids::STRING,
            CanonicalBuiltinType::Char => builtin_type_ids::CHAR,
            CanonicalBuiltinType::Range => builtin_type_ids::RANGE,
            CanonicalBuiltinType::None => builtin_type_ids::NONE,
            CanonicalBuiltinType::Error => type_environment
                .type_id_for_canonical_identity(identity)
                .ok_or_else(|| CompilerError::compiler_error(
                    "Generated canonical Error type has no declaring-context builtin declaration",
                ))?,
        },
        CanonicalTypeIdentity::Option(inner) => {
            let inner = intern_generated_canonical_type(
                inner,
                type_environment,
                external_registry,
                requester_context,
                requester_string_remap,
            )?;
            type_environment.intern_option(inner)
        }
        CanonicalTypeIdentity::Collection(collection) => {
            let element = intern_generated_canonical_type(
                collection.element(),
                type_environment,
                external_registry,
                requester_context,
                requester_string_remap,
            )?;
            type_environment.intern_collection(element, collection.fixed_capacity())
        }
        CanonicalTypeIdentity::OrderedMap(map) => {
            let key = intern_generated_canonical_type(
                map.key(),
                type_environment,
                external_registry,
                requester_context,
                requester_string_remap,
            )?;
            let value = intern_generated_canonical_type(
                map.value(),
                type_environment,
                external_registry,
                requester_context,
                requester_string_remap,
            )?;
            type_environment.intern_map(key, value)
        }
        CanonicalTypeIdentity::FallibleCarrier(carrier) => {
            let success = intern_generated_canonical_type(
                carrier.success(),
                type_environment,
                external_registry,
                requester_context,
                requester_string_remap,
            )?;
            let error = intern_generated_canonical_type(
                carrier.error(),
                type_environment,
                external_registry,
                requester_context,
                requester_string_remap,
            )?;
            type_environment.intern_fallible_carrier(success, error)
        }
        CanonicalTypeIdentity::SourceNominal(origin) => intern_requester_source_nominal(
            origin,
            requester_context,
            requester_string_remap,
            type_environment,
        )?,
        CanonicalTypeIdentity::ModulePrivateNominal(identity) => {
            intern_requester_private_nominal(
                identity,
                requester_context,
                requester_string_remap,
                type_environment,
            )?
        }
        CanonicalTypeIdentity::ExternalOpaque(external) => {
            let (external_type_id, _) = external_registry
                .resolve_canonical_package_type_by_path(external.package(), external.symbol_path())
                .ok_or_else(|| CompilerError::compiler_error(
                    "Generated canonical external type is absent from the binding registry",
                ))?;
            type_environment.intern_external(external_type_id)
        }
        CanonicalTypeIdentity::GenericInstance(instance) => {
            let base_identity = CanonicalTypeIdentity::SourceNominal(instance.base().clone());
            let base_type_id = intern_generated_canonical_type(
                &base_identity,
                type_environment,
                external_registry,
                requester_context,
                requester_string_remap,
            )?;
            let nominal_id = match type_environment.get(base_type_id) {
                Some(TypeDefinition::Struct(definition)) => definition.id,
                Some(TypeDefinition::Choice(definition)) => definition.id,
                _ => {
                    return Err(CompilerError::compiler_error(
                        "Generated canonical generic base is not nominal",
                    ));
                }
            };
            let mut arguments = Vec::with_capacity(instance.arguments().len());
            for argument in instance.arguments() {
                arguments.push(intern_generated_canonical_type(
                    argument,
                    type_environment,
                    external_registry,
                    requester_context,
                    requester_string_remap,
                )?);
            }
            type_environment.intern_generic_instance(nominal_id, arguments.into_boxed_slice())
        }
        CanonicalTypeIdentity::GenericParameter(_) => {
            return Err(CompilerError::compiler_error(
                "Generated request retained an unresolved generic parameter",
            ));
        }
    };
    type_environment.register_canonical_identity(identity.clone(), type_id)?;
    Ok(type_id)
}

fn intern_requester_private_nominal(
    identity: &ModulePrivateNominalIdentity,
    requester_context: &ModuleMaterialisationContext,
    requester_string_remap: &StringIdRemap,
    type_environment: &mut TypeEnvironment,
) -> Result<TypeId, CompilerError> {
    let (requester_path, requester_type_id) = requester_context
        .private_nominals_by_identity
        .get(identity)
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Generated request private nominal '{}' is absent from its requester artefact",
                identity.defining_path()
            ))
        })?;
    let mut generated_path = requester_path.clone();
    generated_path.remap_string_ids(requester_string_remap);

    let type_id = match (
        identity.category(),
        requester_context.type_environment.get(*requester_type_id),
    ) {
        (OriginTypeCategory::Struct, Some(TypeDefinition::Struct(definition))) => {
            let (_, type_id) = type_environment.register_nominal_struct(StructTypeDefinition {
                id: NominalTypeId(0),
                path: generated_path,
                fields: Box::new([]),
                generic_parameters: None,
                const_record: definition.const_record,
            });
            type_id
        }
        (OriginTypeCategory::Choice, Some(TypeDefinition::Choice(_))) => {
            let (_, type_id) = type_environment.register_nominal_choice(ChoiceTypeDefinition {
                id: NominalTypeId(0),
                path: generated_path,
                variants: Box::new([]),
                generic_parameters: None,
            });
            type_id
        }
        _ => {
            return Err(CompilerError::compiler_error(format!(
                "Generated request private nominal '{}' has a mismatched requester definition",
                identity.defining_path()
            )));
        }
    };

    Ok(type_id)
}

fn intern_requester_source_nominal(
    origin: &OriginTypeId,
    requester_context: &ModuleMaterialisationContext,
    requester_string_remap: &StringIdRemap,
    type_environment: &mut TypeEnvironment,
) -> Result<TypeId, CompilerError> {
    let (requester_path, requester_type_id) = requester_context
        .source_nominals_by_origin
        .get(origin)
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Generated request source nominal {:?} is absent from the requester context",
                origin
            ))
        })?;
    let mut generated_path = requester_path.clone();
    generated_path.remap_string_ids(requester_string_remap);

    let type_id = match (
        origin.category(),
        requester_context.type_environment.get(*requester_type_id),
    ) {
        (OriginTypeCategory::Struct, Some(TypeDefinition::Struct(definition))) => {
            let (_, type_id) = type_environment.register_nominal_struct(StructTypeDefinition {
                id: NominalTypeId(0),
                path: generated_path,
                fields: Box::new([]),
                generic_parameters: None,
                const_record: definition.const_record,
            });
            type_id
        }
        (OriginTypeCategory::Choice, Some(TypeDefinition::Choice(_))) => {
            let (_, type_id) = type_environment.register_nominal_choice(ChoiceTypeDefinition {
                id: NominalTypeId(0),
                path: generated_path,
                variants: Box::new([]),
                generic_parameters: None,
            });
            type_id
        }
        _ => {
            return Err(CompilerError::compiler_error(format!(
                "Generated request source nominal {:?} has a mismatched requester definition",
                origin
            )));
        }
    };

    Ok(type_id)
}

fn install_generated_request_evidence(
    identity: &GeneratedFunctionIdentity,
    requester_context: &ModuleMaterialisationContext,
    requester_string_remap: &StringIdRemap,
    environment: &mut AstModuleEnvironment,
    string_table: &StringTable,
) -> Result<(), CompilerError> {
    for evidence_identity in identity.evidence() {
        let generated_target_type_id = environment
            .type_environment
            .type_id_for_canonical_identity(evidence_identity.target_type_identity())
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated evidence target type was not interned in the generated environment",
                )
            })?;
        let generated_trait_id = if let Some(trait_id) = environment
            .lookups
            .trait_environment
            .id_for_canonical_identity(evidence_identity.trait_identity())
        {
            trait_id
        } else {
            return Err(CompilerError::compiler_error(format!(
                "Generated evidence trait {:?} is absent from the declaring context",
                evidence_identity.trait_identity()
            )));
        };
        let requester_trait_id = requester_context
            .trait_environment
            .id_for_canonical_identity(evidence_identity.trait_identity())
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated evidence trait is absent from the requester context",
                )
            })?;
        let requester_target_type_id = requester_type_id_for_canonical_identity(
            evidence_identity.target_type_identity(),
            requester_context,
        )?;
        let requester_evidence_id = requester_context
            .trait_evidence_environment
            .canonical_for(requester_target_type_id, requester_trait_id)
            .or_else(|| {
                requester_context
                    .trait_evidence_environment
                    .builtin_for(requester_target_type_id, requester_trait_id)
            })
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated request selected evidence absent from the requester context",
                )
            })?;
        let requester_evidence = requester_context
            .trait_evidence_environment
            .get(requester_evidence_id)
            .ok_or_else(|| {
                CompilerError::compiler_error("Generated requester evidence is missing")
            })?;
        let requester_trait = requester_context
            .trait_environment
            .get(requester_trait_id)
            .ok_or_else(|| CompilerError::compiler_error("Generated requester trait is missing"))?;
        let generated_trait = environment
            .lookups
            .trait_environment
            .get(generated_trait_id)
            .ok_or_else(|| CompilerError::compiler_error("Generated declaring trait is missing"))?;

        let mut requirements = Vec::with_capacity(generated_trait.requirements.len());
        let mut imported_contracts = Vec::with_capacity(generated_trait.requirements.len());
        let executable_requirements = if requester_evidence.kind == TraitEvidenceKind::Canonical {
            generated_trait.requirements.as_slice()
        } else {
            // Compiler-owned builtin cast evidence proves the bound directly. It deliberately
            // has no source receiver-method mapping because `cast` lowers through builtin cast
            // semantics rather than an evidence method call.
            &[]
        };
        for generated_requirement in executable_requirements {
            let requester_requirement = requester_trait
                .requirements
                .iter()
                .find(|requirement| {
                    requester_context.string_table.resolve(requirement.name)
                        == string_table.resolve(generated_requirement.name)
                })
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Generated evidence could not match a canonical trait requirement",
                    )
                })?;
            let requester_mapping = requester_evidence
                .requirements
                .iter()
                .find(|mapping| mapping.requirement_id == requester_requirement.id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Generated evidence has no executable target for a trait requirement",
                    )
                })?;
            let mut method_path = requester_mapping.method_path.clone();
            method_path.remap_string_ids(requester_string_remap);
            requirements.push(TraitRequirementEvidence {
                requirement_id: generated_requirement.id,
                method_path: method_path.clone(),
            });

            let source_contract = requester_context
                .imported_functions_by_local_path
                .get(&requester_mapping.method_path)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Generated evidence method has no frozen executable target",
                    )
                })?;
            let target = match source_contract.target.clone() {
                SourceFunctionTarget::Imported { origin, .. } => SourceFunctionTarget::Imported {
                    origin,
                    local_path: method_path.clone(),
                },
                SourceFunctionTarget::ModulePrivate { identity, .. } => {
                    SourceFunctionTarget::ModulePrivate {
                        identity,
                        local_path: method_path.clone(),
                    }
                }
                SourceFunctionTarget::Local(_) | SourceFunctionTarget::Generated { .. } => {
                    return Err(CompilerError::compiler_error(
                        "Generated evidence method retained an invalid executable target",
                    ));
                }
            };
            imported_contracts.push((
                method_path.clone(),
                AstImportedFunctionContract {
                    target,
                    summary: source_contract.summary.clone(),
                    fallible_carrier_type_id: None,
                },
            ));
        }

        let mut source_file = requester_evidence.source_file.clone();
        source_file.remap_string_ids(requester_string_remap);
        let mut declaration_location = requester_evidence.declaration_location.clone();
        declaration_location.remap_string_ids(requester_string_remap);
        let generated_evidence = TraitEvidenceDefinition {
            id: TraitEvidenceId(0),
            kind: requester_evidence.kind,
            target_type_id: generated_target_type_id,
            trait_id: generated_trait_id,
            source_file,
            declaration_location,
            requirements,
        };
        let lookups = Rc::make_mut(&mut environment.lookups);
        match requester_evidence.kind {
            TraitEvidenceKind::Canonical => Rc::make_mut(&mut lookups.trait_evidence_environment)
                .insert_validated(generated_evidence),
            TraitEvidenceKind::Builtin => Rc::make_mut(&mut lookups.trait_evidence_environment)
                .insert_builtin(generated_evidence),
        }
        for (path, contract) in imported_contracts {
            lookups
                .imported_functions_by_local_path
                .insert(path, contract);
        }
    }

    Ok(())
}

fn fallible_carrier_for_signature(
    signature: &crate::compiler_frontend::ast::statements::functions::FunctionSignature,
    type_environment: &mut TypeEnvironment,
) -> Option<TypeId> {
    let error_type_id = signature
        .returns
        .iter()
        .find(|slot| {
            slot.channel
                == crate::compiler_frontend::ast::statements::functions::ReturnChannel::Error
        })?
        .type_id?;
    let success_types = signature.success_return_type_ids();
    let success_type_id = match success_types.as_slice() {
        [] => builtin_type_ids::NONE,
        [single] => *single,
        many => type_environment.intern_tuple(many.to_vec()),
    };
    Some(type_environment.intern_fallible_carrier(success_type_id, error_type_id))
}

pub(crate) fn bootstrap_call_summary_from_signature(
    signature: &crate::compiler_frontend::ast::statements::functions::FunctionSignature,
) -> PublicCallSummary {
    let parameters = signature
        .parameters
        .iter()
        .map(|parameter| {
            let access = if parameter.value.reactive_source.is_some() {
                PublicCallParameterAccess::Reactive
            } else if parameter.value.value_mode.is_mutable() {
                PublicCallParameterAccess::Mutable
            } else {
                PublicCallParameterAccess::Shared
            };
            PublicCallParameterSummary {
                access,
                mutation: PublicCallMutationEffect::NoWrite,
                transfer_eligibility: if access == PublicCallParameterAccess::Reactive {
                    PublicCallTransferEligibility::Ineligible
                } else {
                    PublicCallTransferEligibility::Eligible
                },
                transfer_effect: if access == PublicCallParameterAccess::Reactive {
                    PublicCallTransferEffect::NeverConsumes
                } else {
                    PublicCallTransferEffect::MayConsume
                },
                reactive_effect: PublicCallReactiveEffect::None,
            }
        })
        .collect();
    let mut alias_parameters = signature
        .success_returns()
        .into_iter()
        .filter_map(|returned| returned.alias_candidates())
        .flatten()
        .copied()
        .collect::<Vec<_>>();
    alias_parameters.sort_unstable();
    alias_parameters.dedup();

    PublicCallSummary {
        parameters,
        return_alias: if alias_parameters.is_empty() {
            FunctionReturnAliasSummary::Fresh
        } else {
            FunctionReturnAliasSummary::AliasParams(alias_parameters)
        },
    }
}

fn requester_type_id_for_canonical_identity(
    identity: &CanonicalTypeIdentity,
    requester_context: &ModuleMaterialisationContext,
) -> Result<TypeId, CompilerError> {
    if let Some(type_id) = requester_context
        .type_environment
        .type_id_for_canonical_identity(identity)
    {
        return Ok(type_id);
    }
    let CanonicalTypeIdentity::SourceNominal(origin) = identity else {
        return match identity {
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Bool) => {
                Ok(builtin_type_ids::BOOL)
            }
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int) => Ok(builtin_type_ids::INT),
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Float) => {
                Ok(builtin_type_ids::FLOAT)
            }
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Decimal) => {
                Ok(builtin_type_ids::DECIMAL)
            }
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String) => {
                Ok(builtin_type_ids::STRING)
            }
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Char) => {
                Ok(builtin_type_ids::CHAR)
            }
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Range) => {
                Ok(builtin_type_ids::RANGE)
            }
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::None) => {
                Ok(builtin_type_ids::NONE)
            }
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error)
            | CanonicalTypeIdentity::ModulePrivateNominal(_)
            | CanonicalTypeIdentity::ExternalOpaque(_)
            | CanonicalTypeIdentity::Collection(_)
            | CanonicalTypeIdentity::OrderedMap(_)
            | CanonicalTypeIdentity::Option(_)
            | CanonicalTypeIdentity::FallibleCarrier(_)
            | CanonicalTypeIdentity::GenericInstance(_)
            | CanonicalTypeIdentity::GenericParameter(_) => Err(CompilerError::compiler_error(
                "Generated evidence target has no requester-local canonical type handle",
            )),
            CanonicalTypeIdentity::SourceNominal(_) => unreachable!(),
        };
    };
    requester_context
        .source_nominals_by_origin
        .get(origin)
        .map(|(_, type_id)| *type_id)
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "Generated source evidence target is absent from the requester context",
            )
        })
}
