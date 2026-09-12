//! Stable capture types shared by generated materialisation lanes.
//!
//! These mirror types intentionally own donor-independent names, identities and spans. A frozen
//! artefact must outlive the declaring environment, so it cannot retain live `StringId`,
//! `InternedPath` or type-table handles.
use super::frozen_syntax::StableBodySyntax;
use super::nominal_blueprints::{
    MaterialisationTypeBlueprint, NominalMaterialisationBlueprint, intern_generated_canonical_type,
};
use super::preparation_freeze::ModuleMaterialisationPreparation;
use super::visibility::StableFileVisibility;
use crate::compiler_frontend::ast::module_ast::environment::builder::import_projection::values::FoldedValueMaterialiser;
use crate::compiler_frontend::ast::module_ast::scope_context::{
    FileValueResolutionServices, Stage0ResolutionFacts,
};
use crate::compiler_frontend::ast::statements::functions::ReturnChannel;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalTraitIdentity, CanonicalTypeIdentity, ExportedGenericParameterIdentity,
    GenericDeclarationOrigin, NominalOriginResolver,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, GeneratedFunctionIdentity, ModulePrivateExecutableIdentity,
    OriginDeclarationId, OriginFunctionId, OriginTypeId, StableModuleOriginIdentity,
};
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{
    PathId, PathIdRemap, PathInternerFork,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
#[derive(Clone)]
pub(super) struct GenericTemplateArtefact {
    pub(super) declaration_identity: GeneratedDeclarationIdentity,
    pub(super) generic_parameter_owner: Option<GenericDeclarationOrigin>,
    pub(super) receiver: Option<StableReceiverKey>,
    pub(super) receiver_nominal_identity: Option<CanonicalTypeIdentity>,
    pub(super) function_path: PathId,
    pub(super) source_file: PathId,
    pub(super) declaration_span: Option<SourceSpan>,
    pub(super) body: StableBodySyntax,
    pub(super) signature: StableFunctionSignature,
    pub(super) generic_parameters: Box<[StableGenericParameter]>,
    pub(super) visibility: StableFileVisibility,
    pub(super) declarations: Box<[StableDeclarationBinding]>,
    pub(super) local_declarations: Box<[PathId]>,
    pub(super) callables: Box<[StableCallableBinding]>,
    pub(super) nominals: Box<[StableNominalBinding]>,
    pub(super) nominal_blueprints:
        FxHashMap<CanonicalTypeIdentity, NominalMaterialisationBlueprint>,
}

impl GenericTemplateArtefact {
    pub(super) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.function_path = remap.get(self.function_path);
        self.source_file = remap.get(self.source_file);
        if let Some(receiver) = &mut self.receiver {
            receiver.remap_path_ids(remap);
        }
        self.body.remap_path_ids(remap);
        self.visibility.remap_path_ids(remap);
        for declaration in &mut self.declarations {
            declaration.remap_path_ids(remap);
        }
        for path in &mut self.local_declarations {
            *path = remap.get(*path);
        }
        for callable in &mut self.callables {
            callable.remap_path_ids(remap);
        }
        for nominal in &mut self.nominals {
            nominal.remap_path_ids(remap);
        }
    }
}

#[derive(Clone)]
pub(super) struct StableGenericParameter {
    pub(super) name: String,
    pub(super) exported_identity: Option<ExportedGenericParameterIdentity>,
    pub(super) bounds: Box<[CanonicalTraitIdentity]>,
}

#[derive(Clone)]
pub(super) struct StableFunctionSignature {
    pub(super) parameters: Box<[StableFunctionParameter]>,
    pub(super) returns: Box<[StableFunctionReturn]>,
}

#[derive(Clone)]
pub(super) struct StableFunctionParameter {
    pub(super) name: String,
    pub(super) value_mode: ValueMode,
    pub(super) reactive: bool,
    pub(super) folded_default: Option<PublicFoldedValue>,
    pub(super) parameter_type: MaterialisationTypeBlueprint,
    pub(super) span: Option<SourceSpan>,
}

#[derive(Clone)]
pub(super) struct StableFunctionReturn {
    pub(super) return_type: MaterialisationTypeBlueprint,
    pub(super) channel: ReturnChannel,
}

#[derive(Clone)]
pub(super) struct StableDeclarationBinding {
    pub(super) local_path: PathId,
    pub(super) origin: OriginDeclarationId,
}

#[derive(Clone)]
pub(super) struct StableCallableBinding {
    pub(super) local_path: PathId,
    pub(super) target: StableFunctionTarget,
    pub(super) signature: StableFunctionSignature,
    pub(super) summary: PublicCallSummary,
}

#[derive(Clone)]
pub(super) struct StableNominalBinding {
    pub(super) local_path: PathId,
    pub(super) identity: CanonicalTypeIdentity,
}
impl StableDeclarationBinding {
    fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.local_path = remap.get(self.local_path);
    }
}

impl StableCallableBinding {
    fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.local_path = remap.get(self.local_path);
    }
}

impl StableNominalBinding {
    fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.local_path = remap.get(self.local_path);
    }
}


#[derive(Clone)]
pub(super) enum StableFunctionTarget {
    Imported(OriginFunctionId),
    Generated(GeneratedFunctionIdentity),
    ModulePrivate(ModulePrivateExecutableIdentity),
}

#[derive(Clone)]
pub(super) enum StableReceiverKey {
    Struct(PathId),
    Choice(PathId),
}

impl StableReceiverKey {
    pub(super) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        match self {
            Self::Struct(path) | Self::Choice(path) => *path = remap.get(*path),
        }
    }

    pub(super) fn capture(
        receiver: &ReceiverKey,
        _string_table: &StringTable,
    ) -> Result<Self, CompilerError> {
        match receiver {
            ReceiverKey::Struct(path) => Ok(Self::Struct(*path)),
            ReceiverKey::Choice(path) => Ok(Self::Choice(*path)),
            ReceiverKey::External(_) | ReceiverKey::BuiltinScalar(_) => {
                Err(CompilerError::compiler_error(
                    "Retained receiver method has a non-source receiver key",
                ))
            }
        }
    }

    pub(super) fn materialise(&self, _string_table: &mut StringTable) -> ReceiverKey {
        match self {
            Self::Struct(path) => ReceiverKey::Struct(*path),
            Self::Choice(path) => ReceiverKey::Choice(*path),
        }
    }
}
pub(super) trait MaterialisationNominalSource {
    const SOURCE_LABEL: &'static str;

    fn nominal_blueprint(
        &self,
        identity: &CanonicalTypeIdentity,
    ) -> Option<&NominalMaterialisationBlueprint>;

    fn nominal_blueprints(
        &self,
    ) -> impl Iterator<Item = (&CanonicalTypeIdentity, &NominalMaterialisationBlueprint)> + '_;
}

impl MaterialisationNominalSource for ModuleMaterialisationPreparation {
    const SOURCE_LABEL: &'static str = "contexts";

    fn nominal_blueprint(
        &self,
        identity: &CanonicalTypeIdentity,
    ) -> Option<&NominalMaterialisationBlueprint> {
        self.nominal_blueprints.get(identity)
    }

    fn nominal_blueprints(
        &self,
    ) -> impl Iterator<Item = (&CanonicalTypeIdentity, &NominalMaterialisationBlueprint)> + '_ {
        self.nominal_blueprints.iter()
    }
}

impl MaterialisationNominalSource for GenericTemplateArtefact {
    const SOURCE_LABEL: &'static str = "artefact";

    fn nominal_blueprint(
        &self,
        identity: &CanonicalTypeIdentity,
    ) -> Option<&NominalMaterialisationBlueprint> {
        self.nominal_blueprints.get(identity)
    }

    fn nominal_blueprints(
        &self,
    ) -> impl Iterator<Item = (&CanonicalTypeIdentity, &NominalMaterialisationBlueprint)> + '_ {
        self.nominal_blueprints.iter()
    }
}

pub(super) struct MaterialisationNominalOriginResolver<'a> {
    pub(super) type_environment: &'a TypeEnvironment,
}

pub(super) struct GeneratedFoldedValueMaterialiser<'a, 'b, N: MaterialisationNominalSource> {
    pub(super) type_environment: &'a mut TypeEnvironment,
    pub(super) external_registry: &'b ExternalPackageRegistry,
    pub(super) nominal_source: &'b N,
    pub(super) template_ir_store:
        Rc<RefCell<crate::compiler_frontend::ast::templates::tir::TemplateIrStore>>,
    pub(super) module_resources: Rc<RefCell<ModuleResourceTable>>,
    pub(super) path_fork: &'a mut PathInternerFork,
}

impl<N: MaterialisationNominalSource> FoldedValueMaterialiser
    for GeneratedFoldedValueMaterialiser<'_, '_, N>
{
    fn intern_resource_origin(
        &mut self,
        origin: &crate::compiler_frontend::paths::resource_identity::StableResourceOriginId,
        span: Option<SourceSpan>,
    ) -> Result<crate::compiler_frontend::paths::module_resources::ResourceId, CompilerError> {
        // WHY: `intern_origin` is idempotent within one sidecar table, so repeated projections
        // during one materialisation reuse one local handle and one row. A handle is valid only in
        // the table that issued it.
        Ok(self
            .module_resources
            .borrow_mut()
            .intern_origin(origin.clone(), span))
    }

    fn intern_canonical_type(
        &mut self,
        identity: &CanonicalTypeIdentity,
        string_table: &mut StringTable,
    ) -> Result<TypeId, CompilerError> {
        intern_generated_canonical_type(
            identity,
            self.type_environment,
            self.external_registry,
            self.nominal_source,
            string_table,
            self.path_fork,
        )
    }

    fn path_fork(&mut self) -> &mut PathInternerFork {
        self.path_fork
    }

    fn type_environment(&self) -> &TypeEnvironment {
        self.type_environment
    }

    fn template_ir_store(
        &self,
    ) -> Rc<RefCell<crate::compiler_frontend::ast::templates::tir::TemplateIrStore>> {
        Rc::clone(&self.template_ir_store)
    }
}
/// Shared services for projecting stable values into one generated AST environment.
///
/// WHAT: keeps the authorities needed by nominal field-default projection together while the
///       generated environment owns its mutable type table.
/// WHY: one service bundle prevents the helper's resource-table handoff from becoming an
///       unreviewable argument list as generated value projection grows.
pub(super) struct GeneratedValueMaterialisationServices<'a> {
    pub(super) external_registry: &'a ExternalPackageRegistry,
    pub(super) template_ir_store:
        &'a Rc<RefCell<crate::compiler_frontend::ast::templates::tir::TemplateIrStore>>,
    pub(super) module_resources: Rc<RefCell<ModuleResourceTable>>,
}

pub(super) fn generated_file_value_resolution_services(
    module_resources: Rc<RefCell<ModuleResourceTable>>,
    module_origin: Option<StableModuleOriginIdentity>,
    stage0_resolution_facts: Arc<Stage0ResolutionFacts>,
) -> Rc<FileValueResolutionServices> {
    let frozen_identity_handle = module_origin
        .as_ref()
        .map(|origin| FrozenIdentityHandle::for_domain(origin.package().clone()))
        .unwrap_or_else(FrozenIdentityHandle::new);
    Rc::new(FileValueResolutionServices {
        stage0_resolution_facts: Some(stage0_resolution_facts),
        module_resources,
        module_origin,
        frozen_identity_handle,
    })
}

impl NominalOriginResolver for MaterialisationNominalOriginResolver<'_> {
    fn resolve_nominal_origin(
        &self,
        nominal_id: NominalTypeId,
    ) -> Result<OriginTypeId, CompilerError> {
        let type_id = self
            .type_environment
            .type_id_for_nominal_id(nominal_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Materialisation folded-value projection has an unknown nominal type",
                )
            })?;
        match self
            .type_environment
            .canonical_identity_for_type_id(type_id)
        {
            Some(CanonicalTypeIdentity::SourceNominal(origin)) => Ok(origin.clone()),
            _ => Err(CompilerError::compiler_error(
                "Materialisation folded-value projection has no source nominal origin",
            )),
        }
    }
}

impl StableFunctionTarget {
    pub(super) fn capture(target: &SourceFunctionTarget) -> Option<Self> {
        match target {
            SourceFunctionTarget::Imported { origin, .. } => Some(Self::Imported(origin.clone())),
            SourceFunctionTarget::Generated { identity, .. } => {
                Some(Self::Generated(identity.clone()))
            }
            SourceFunctionTarget::ModulePrivate { identity, .. } => {
                Some(Self::ModulePrivate(identity.clone()))
            }
            SourceFunctionTarget::Local(_) => None,
        }
    }

    pub(super) fn materialise(&self, local_path: PathId) -> SourceFunctionTarget {
        match self {
            Self::Imported(origin) => SourceFunctionTarget::Imported {
                origin: origin.clone(),
                local_path,
            },
            Self::Generated(identity) => SourceFunctionTarget::Generated {
                identity: identity.clone(),
                local_path,
            },
            Self::ModulePrivate(identity) => SourceFunctionTarget::ModulePrivate {
                identity: identity.clone(),
                local_path,
            },
        }
    }
}
