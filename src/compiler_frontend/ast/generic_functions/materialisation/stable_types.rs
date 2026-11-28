//! Stable capture types shared by generated materialisation lanes.
//!
//! These mirror types intentionally own donor-independent names, identities and spans. A frozen
//! artefact must outlive the declaring environment, so it cannot retain live `StringId`,
//! `PathId` or type-table handles.
use super::frozen_syntax::StableBodySyntax;
use super::nominal_blueprints::{
    MaterialisationTypeBlueprint, NominalMaterialisationBlueprint, intern_generated_canonical_type,
};
use super::preparation_freeze::{ModuleMaterialisationCapture, ModuleMaterialisationPreparation};
use super::semantic_closure::{StableLocalAlias, StableLocalConstant};
use super::visibility::StableFileVisibility;
use crate::compiler_frontend::ast::const_values::store::{
    ConstValueId, ConstValueStore, ConstValueStoreError,
};
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::module_ast::environment::builder::import_projection::values::{
    FoldedValueMaterialiser, materialize_folded_value,
};
use crate::compiler_frontend::ast::module_ast::scope_context::{
    FileValueResolutionServices, Stage0ResolutionFacts,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalTraitIdentity, CanonicalTypeIdentity, ExportedGenericParameterIdentity,
    GenericDeclarationOrigin, NominalOriginResolver,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CapacityResource;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::compiler_messages::{CompilerStage, DiagnosticBag, PreparedFailure};
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
use crate::compiler_frontend::style_directives::{ConstraintDirectiveSpans, SourceConstraintSet};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
#[derive(Clone)]
pub(crate) struct GenericTemplateArtefact {
    pub(super) declaration_identity: GeneratedDeclarationIdentity,
    pub(super) generic_parameter_owner: Option<GenericDeclarationOrigin>,
    pub(super) receiver: Option<StableReceiverKey>,
    pub(super) function_path: PathId,
    pub(super) source_file: PathId,
    /// Authored provider declaration span, retained for consuming instantiation diagnostics.
    ///
    /// WHY: the materialised requester environment installs a `GenericFunctionTemplate` per
    ///      selected row, including the root row, carrying this span. The emitter then labels
    ///      `GenericInstantiationDeclarationSite` from it, so a failing instance points at the
    ///      provider declaration.
    pub(super) declaration_span: Option<SourceSpan>,
    /// Lexical source constraints authored on the provider's generic declaration.
    ///
    /// WHAT: the effective `$fast_math`/`$safe_math`/`$infallible` set on the function head.
    /// WHY: a consumer materialises this body through the artefact; the frozen template must
    ///      build every instance body under the declaration's arithmetic mode, not the caller's.
    pub(super) source_constraints: SourceConstraintSet,
    /// Authored guarantee directive spans on the provider's generic declaration head.
    ///
    /// WHAT: the `$safe_math` and `$infallible` primary label spans a materialised instance's
    ///       constraint diagnostics use.
    /// WHY: the instance body is materialised in another module; only the artefact carries the
    ///      provider's guarantee directive locations into that compilation.
    pub(super) constraint_directive_spans: ConstraintDirectiveSpans,
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

/// One retained const function body published for cross-module execution.
///
/// WHAT: a provider-local const function's retained source syntax plus the stable signature,
///       visibility and declaration closure its body needs when a consumer materialises it.
/// WHY: const calls fold before HIR exists, so a consumer must parse the provider body through its
///      own expression store. Carrying only source syntax and stable semantic facts keeps provider
///      `ExprId`, `ConstValueId` and local HIR out of publication.
#[derive(Clone)]
pub(crate) struct ConstFunctionArtefact {
    /// Public origin for an exported entry; `None` for a private transitive helper.
    pub(crate) origin: Option<OriginFunctionId>,
    /// Provider-local declaration path, rebased to the consumer domain on materialisation.
    pub(crate) function_path: PathId,
    pub(super) body: StableBodySyntax,
    pub(super) signature: StableFunctionSignature,
    pub(super) visibility: StableFileVisibility,
    pub(super) declarations: Box<[StableDeclarationBinding]>,
    pub(super) constants: Box<[StableLocalConstant]>,
    pub(super) aliases: Box<[StableLocalAlias]>,
    /// Provider-local const function bodies this body can call.
    pub(super) closure: Box<[PathId]>,
    pub(super) callables: Box<[StableCallableBinding]>,
    pub(super) nominals: Box<[StableNominalBinding]>,
    pub(super) nominal_blueprints:
        FxHashMap<CanonicalTypeIdentity, NominalMaterialisationBlueprint>,
}

impl ConstFunctionArtefact {
    pub(super) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.function_path = remap.get(self.function_path);
        self.body.remap_path_ids(remap);
        self.visibility.remap_path_ids(remap);
        for declaration in &mut self.declarations {
            declaration.remap_path_ids(remap);
        }
        for constant in &mut self.constants {
            constant.local_path = remap.get(constant.local_path);
        }
        for alias in &mut self.aliases {
            alias.local_path = remap.get(alias.local_path);
        }
        for path in &mut self.closure {
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

/// Canonical callable input for generic inference, including imported templates without bodies.
#[derive(Clone)]
pub(super) struct StableGenericCallableContract {
    pub(super) declaration_identity: GeneratedDeclarationIdentity,
    pub(super) generic_parameter_owner: Option<GenericDeclarationOrigin>,
    pub(super) receiver: Option<StableReceiverKey>,
    pub(super) function_path: PathId,
    pub(super) source_file: PathId,
    pub(super) declaration_span: Option<SourceSpan>,
    pub(super) signature: StableFunctionSignature,
    pub(super) generic_parameters: Box<[StableGenericParameter]>,
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
    pub(super) return_contract: StableFunctionReturnContract,
}

/// Frozen whole return contract of one materialised function signature.
///
/// WHAT: either the ordinary success/error blueprints or a declared Never contract.
/// WHY: a generated concrete function must retain the template's exact contract; flattening
///      Never into an empty ordinary shape would let a non-returning instance be called as a
///      value after materialisation.
#[derive(Clone)]
pub(super) enum StableFunctionReturnContract {
    Returns(StableReturns),
    Never,
}

impl StableFunctionReturnContract {
    pub(super) fn ordinary(&self) -> Option<&StableReturns> {
        match self {
            Self::Returns(returns) => Some(returns),
            Self::Never => None,
        }
    }
}

/// Frozen ordinary success and error blueprints inside a whole return contract.
///
/// WHAT: the ordered success blueprints and the independent error blueprint. Stored locally
/// because the slot type is a materialisation blueprint, not a shared `TypeId`.
#[derive(Clone)]
pub(super) struct StableReturns {
    pub(super) success: Box<[MaterialisationTypeBlueprint]>,
    pub(super) error: Option<MaterialisationTypeBlueprint>,
}

#[derive(Clone)]
pub(super) enum StableParameterContract {
    Value,
    Wire,
    Route {
        input: Option<MaterialisationTypeBlueprint>,
    },
}

#[derive(Clone)]
pub(super) struct StableFunctionParameter {
    pub(super) name: String,
    pub(super) value_mode: ValueMode,
    pub(super) folded_default: Option<PublicFoldedValue>,
    pub(super) parameter_type: MaterialisationTypeBlueprint,
    pub(super) contract: StableParameterContract,
    pub(super) span: Option<SourceSpan>,
}

impl StableParameterContract {
    /// Blueprint this contract contributes beyond the parameter's own type.
    ///
    /// WHAT: returns the blueprint for a `Route` input, or `None` for every other contract.
    /// WHY: one owner keeps the sidecar's nominal closure from skipping the Route input.
    pub(super) fn input_blueprint(&self) -> Option<&MaterialisationTypeBlueprint> {
        match self {
            StableParameterContract::Route { input } => input.as_ref(),
            StableParameterContract::Value | StableParameterContract::Wire => None,
        }
    }
}

impl StableFunctionParameter {
    /// Every blueprint this parameter's contract touches.
    ///
    /// WHAT: yields the parameter's own blueprint, then any contract-owned blueprint.
    /// WHY: the sidecar nominal closure must retain a blueprint for every type the accepted
    ///      signature names, including a Route input.
    pub(super) fn semantic_type_blueprints(
        &self,
    ) -> impl Iterator<Item = &MaterialisationTypeBlueprint> {
        std::iter::once(&self.parameter_type).chain(self.contract.input_blueprint())
    }
}

#[derive(Clone)]
pub(super) struct StableDeclarationBinding {
    pub(super) local_path: PathId,
    pub(super) origin: OriginDeclarationId,
}

#[derive(Clone)]
pub(crate) struct StableCallableBinding {
    pub(crate) local_path: PathId,
    pub(crate) target: StableFunctionTarget,
    pub(super) signature: StableFunctionSignature,
    pub(crate) summary: PublicCallSummary,
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
    pub(super) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.local_path = remap.get(self.local_path);
    }
}

impl StableNominalBinding {
    fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.local_path = remap.get(self.local_path);
    }
}

#[derive(Clone)]
pub(crate) enum StableFunctionTarget {
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
    ) -> Result<Self, InfrastructureFailure> {
        match receiver {
            ReceiverKey::Struct(path) => Ok(Self::Struct(*path)),
            ReceiverKey::Choice(path) => Ok(Self::Choice(*path)),
            ReceiverKey::External(_) | ReceiverKey::BuiltinScalar(_) => {
                Err(InfrastructureFailure::internal(
                    CompilerStage::GenericMaterialisation,
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

impl MaterialisationNominalSource for ModuleMaterialisationCapture {
    const SOURCE_LABEL: &'static str = "capture";

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

impl MaterialisationNominalSource for ConstFunctionArtefact {
    const SOURCE_LABEL: &'static str = "const artefact";

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

/// Borrowed composition of two nominal blueprint authorities.
///
/// The declaring source is preferred so authored provenance remains authoritative; requester
/// blueprints fill only identities not retained by that source. The composition owns neither map,
/// which keeps source lifetime and publication ownership unchanged.
impl<P, R> MaterialisationNominalSource for (&P, &R)
where
    P: MaterialisationNominalSource,
    R: MaterialisationNominalSource,
{
    const SOURCE_LABEL: &'static str = "declaring and requester blueprint sources";

    fn nominal_blueprint(
        &self,
        identity: &CanonicalTypeIdentity,
    ) -> Option<&NominalMaterialisationBlueprint> {
        self.0
            .nominal_blueprint(identity)
            .or_else(|| self.1.nominal_blueprint(identity))
    }

    fn nominal_blueprints(
        &self,
    ) -> impl Iterator<Item = (&CanonicalTypeIdentity, &NominalMaterialisationBlueprint)> + '_ {
        let primary = self.0;
        primary.nominal_blueprints().chain(
            self.1
                .nominal_blueprints()
                .filter(move |(identity, _)| primary.nominal_blueprint(identity).is_none()),
        )
    }
}

pub(super) struct GeneratedFoldedValueMaterialiser<'a, 'b, N: MaterialisationNominalSource> {
    pub(super) type_environment: &'a mut TypeEnvironment,
    pub(super) external_registry: &'b ExternalPackageRegistry,
    pub(super) nominal_source: &'b N,
    pub(super) module_resources: Rc<RefCell<ModuleResourceTable>>,
    pub(super) path_fork: &'a mut PathInternerFork,
}

impl<N: MaterialisationNominalSource> FoldedValueMaterialiser
    for GeneratedFoldedValueMaterialiser<'_, '_, N>
{
    const IMPORT_STAGE: CompilerStage = CompilerStage::GenericMaterialisation;

    fn field_path_exhausted(span: Option<SourceSpan>) -> ConstValueStoreError {
        token_lexical::compiler_capacity_exceeded(CapacityResource::PathNodes, span).into()
    }

    fn intern_resource_origin(
        &mut self,
        origin: &crate::compiler_frontend::paths::resource_identity::StableResourceOriginId,
        span: Option<SourceSpan>,
    ) -> Result<crate::compiler_frontend::paths::module_resources::ResourceId, InfrastructureFailure>
    {
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
    ) -> Result<TypeId, ConstValueStoreError> {
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
    pub(super) expression_builder: &'a mut AstExpressionBuilder,
    pub(super) const_values: &'a mut ConstValueStore,
}

/// Rebuild a generated default or provider-private constant using the ordinary folded decoder.
pub(super) fn import_canonical_folded_value(
    materialiser: &mut GeneratedFoldedValueMaterialiser<'_, '_, impl MaterialisationNominalSource>,
    folded: &PublicFoldedValue,
    expected_type_id: TypeId,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    span: Option<SourceSpan>,
) -> Result<ConstValueId, ConstValueStoreError> {
    materialize_folded_value(
        materialiser,
        folded,
        expected_type_id,
        const_values,
        string_table,
        span,
    )
}

pub(super) fn const_store_error_into_prepared_failure(
    error: ConstValueStoreError,
    string_table: &StringTable,
    type_environment: Option<&TypeEnvironment>,
) -> PreparedFailure {
    match error {
        ConstValueStoreError::Infrastructure(errors) => PreparedFailure::Infrastructure(errors),
        ConstValueStoreError::Diagnostic(diagnostic) => PreparedFailure::capture_diagnosed(
            DiagnosticBag::from(diagnostic),
            string_table.clone(),
            type_environment,
            None,
        ),
    }
}

pub(super) fn generated_file_value_resolution_services(
    module_resources: Rc<RefCell<ModuleResourceTable>>,
    module_origin: Option<StableModuleOriginIdentity>,
    stage0_resolution_facts: Arc<Stage0ResolutionFacts>,
    qualified_reference_targets: Arc<
        crate::compiler_frontend::headers::binding_environment::QualifiedReferenceTargets,
    >,
) -> Rc<FileValueResolutionServices> {
    let frozen_identity_handle = module_origin
        .as_ref()
        .map(|origin| FrozenIdentityHandle::for_domain(origin.package().clone()))
        .unwrap_or_else(FrozenIdentityHandle::new);
    Rc::new(FileValueResolutionServices {
        stage0_resolution_facts: Some(stage0_resolution_facts),
        module_resources,
        qualified_reference_targets,
        module_origin,
        public_source_trait_origins: Arc::default(),
        frozen_identity_handle,
    })
}

impl NominalOriginResolver for TypeEnvironment {
    fn resolve_nominal_origin(
        &self,
        nominal_id: NominalTypeId,
    ) -> Result<OriginTypeId, InfrastructureFailure> {
        let type_id = self.type_id_for_nominal_id(nominal_id).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::GenericMaterialisation,
                "Materialisation folded-value projection has an unknown nominal type",
            )
        })?;
        match self.canonical_identity_for_type_id(type_id) {
            Some(CanonicalTypeIdentity::SourceNominal(origin)) => Ok(origin.clone()),
            _ => Err(InfrastructureFailure::internal(
                CompilerStage::GenericMaterialisation,
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
