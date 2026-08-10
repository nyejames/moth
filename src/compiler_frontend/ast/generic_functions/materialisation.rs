//! Immutable declaring-module context for generated generic functions.
//!
//! WHAT: retains the resolved, TIR-free semantic tables required to reparse one validated
//! generic body after a consumer emits a concrete request.
//! WHY: generated sidecars must compile in an independent type environment without reopening
//! source or borrowing the mutable requester or declaring-module environment.

use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::arena::FrontendArenaCapacityEstimate;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveSource, ReactiveSourceKind,
};
use crate::compiler_frontend::ast::module_ast::build_context::AstPhaseContext;
use crate::compiler_frontend::ast::module_ast::emission::AstEmitter;
use crate::compiler_frontend::ast::module_ast::environment::builder::import_projection::values::{
    FoldedValueMaterialiser, materialize_public_const_template, materialize_public_folded_value,
};
use crate::compiler_frontend::ast::module_ast::environment::{
    AstEnvironmentInput, AstModuleEnvironment, AstModuleEnvironmentBuilder, AstModuleLookups,
    DeclarationSemanticTable, ResolvedPublicTraitRoot, TopLevelDeclarationTable,
};
use crate::compiler_frontend::ast::module_ast::finalization::AstFinalizer;
use crate::compiler_frontend::ast::module_ast::scope_context::{
    ReceiverMethodCatalog, ReceiverMethodEntry,
};
use crate::compiler_frontend::ast::statements::functions::{
    FunctionSignature, ReturnChannel, ReturnSlot,
};
use crate::compiler_frontend::ast::type_resolution::{
    ResolvedFunctionSignature, ResolvedTypeAnnotation,
};
use crate::compiler_frontend::ast::{AstBuildContext, AstBuildResult, AstImportedFunctionContract};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTraitIdentity, CanonicalTypeIdentity,
    CanonicalTypeProjectionContext, ExportedGenericParameterIdentity, ExternalOpaqueTypeIdentity,
    GenericDeclarationOrigin, ModulePrivateNominalIdentity, ModulePrivateTraitIdentity,
    NominalOriginResolver, project_type_id_to_canonical_identity,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, FieldDefinition,
    StructTypeDefinition, TypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_parameters::{
    GenericParameter, GenericParameterList, TypeParameterId,
};
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, BuiltinTypeKey, FunctionTypeKey, GenericParameterId,
    GenericParameterListId, NominalTypeId, TypeConstructor, TypeId,
};
use crate::compiler_frontend::datatypes::{DataType, ReceiverKey, diagnostic_type_spelling};
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariant;
use crate::compiler_frontend::external_packages::{
    CanonicalBindingSymbolIdentity, ExternalPackageRegistry,
};
use crate::compiler_frontend::folded_value::{
    FoldedValueGenericParameterResolver, PublicConstTemplate, PublicFoldedValue,
    convert_expression_to_folded_value,
};
use crate::compiler_frontend::headers::import_environment::{
    FileVisibility, HeaderImportEnvironment, ImportedFunctionContract, NamespaceRecord,
    NamespaceRecordSource, NamespaceTypeMember, NamespaceValueMember, SourceDeclarationTarget,
    SourceFunctionTarget,
};
use crate::compiler_frontend::headers::module_symbols::{
    GenericDeclarationKind, GenericDeclarationMetadata, ModuleSymbols,
};
use crate::compiler_frontend::paths::path_format::PathStringFormatConfig;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallMutationEffect, PublicCallParameterAccess,
    PublicCallParameterSummary, PublicCallReactiveEffect, PublicCallSummary,
    PublicCallTransferEffect, PublicCallTransferEligibility,
};
use crate::compiler_frontend::public_interface::{
    PublicDeclarationRecord, PublicDeclarationSemantics, PublicEvidenceRecord,
    PublicSemanticInterface,
};
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, ModulePrivateExecutableCategory, ModulePrivateExecutableIdentity,
    ModuleRootRole, OriginDeclarationId, OriginFunctionId, OriginTraitId, OriginTypeCategory,
    OriginTypeId,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, Token, TokenKind};
use crate::compiler_frontend::traits::definitions::{
    ResolvedTraitDefinition, ResolvedTraitParameter, ResolvedTraitRequirement, ResolvedTraitReturn,
    TraitReceiverRequirement, TraitVisibility,
};
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::environment::trait_this_name;
use crate::compiler_frontend::traits::evidence::environment::{
    TraitEvidenceKind, TraitRequirementEvidence,
};
use crate::compiler_frontend::traits::evidence::{
    TraitEvidenceDefinition, TraitEvidenceEnvironment,
};
use crate::compiler_frontend::traits::ids::TraitEvidenceId;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::{FxHashMap, FxHashSet};
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

/// Active build services and requester facts for one published generic materialisation.
///
/// The published context owns only stable declaration artefacts. Build-lifetime registries,
/// source-path folding policy and the requester-local evidence environment enter through this
/// transient input and are never retained by successful module metadata.
pub(crate) struct ModuleMaterialisationInput<'a> {
    pub(crate) identity: &'a GeneratedFunctionIdentity,
    pub(crate) requester_context: &'a ModuleMaterialisationPreparation,
    pub(crate) requester_call_location: &'a SourceLocation,
    pub(crate) external_package_registry: &'a ExternalPackageRegistry,
    pub(crate) style_directives: &'a StyleDirectiveRegistry,
    pub(crate) build_profile: FrontendBuildProfile,
    pub(crate) project_path_resolver: Option<ProjectPathResolver>,
    pub(crate) template_const_loop_iteration_limit: usize,
    /// The requesting module owns generated work in schema-v1 attribution.
    #[cfg(feature = "timers")]
    pub(crate) timing_context: Option<crate::timing::TimingContext>,
}

/// Owned frozen token buffer retained by one generic declaration artefact.
///
/// WHAT: preserves the already-tokenized body as canonical [`TokenKind`] values whose
///       `StringId` payloads index one context-local immutable frozen string pool.
/// WHY: successful metadata must not retain donor `StringId`, `InternedPath`, `FileId`,
/// filesystem paths, or a mutable string table. Freezing remaps donor IDs into the pool once;
/// materialisation merges the pool into the fresh generated-local table once and remaps every
/// token payload through that single pool remap, without running tokenization again and without
/// a second exhaustive token-kind vocabulary.
#[derive(Clone)]
struct StableBodySyntax {
    source_path: Box<[String]>,
    pool: Box<[String]>,
    tokens: Box<[Token]>,
}

/// Incremental frozen string pool builder used while capturing one body syntax.
///
/// Repeated spellings, path components and literals share one pool entry.
#[derive(Default)]
struct FrozenStringPool {
    entries: Vec<String>,
    by_text: FxHashMap<String, u32>,
}

impl FrozenStringPool {
    fn index(&mut self, text: &str) -> StringId {
        if let Some(index) = self.by_text.get(text) {
            return StringId::from_index(*index);
        }

        let index = self.entries.len() as u32;
        let owned = text.to_owned();
        self.entries.push(owned.clone());
        self.by_text.insert(owned, index);
        StringId::from_index(index)
    }

    fn finish(self) -> Box<[String]> {
        self.entries.into_boxed_slice()
    }
}

#[derive(Clone)]
struct StableSourceLocation {
    scope: Box<[String]>,
    start: crate::compiler_frontend::tokenizer::tokens::CharPosition,
    end: crate::compiler_frontend::tokenizer::tokens::CharPosition,
}

impl StableBodySyntax {
    fn capture(tokens: &FileTokens, string_table: &StringTable) -> Self {
        let mut pool = FrozenStringPool::default();
        let frozen_tokens = tokens
            .tokens
            .iter()
            .map(|token| {
                let mut frozen = token.clone();
                // Clone each token once, then remap the clone in place through the frozen pool.
                // The pool never fails to accept a donor spelling, so the walker is infallible.
                frozen
                    .try_remap_string_ids(&mut |id| {
                        Ok::<StringId, std::convert::Infallible>(
                            pool.index(string_table.resolve(id)),
                        )
                    })
                    .expect("frozen string pooling is infallible");
                frozen
            })
            .collect::<Vec<_>>();
        Self {
            source_path: stable_path(&tokens.src_path, string_table),
            pool: pool.finish(),
            tokens: frozen_tokens.into_boxed_slice(),
        }
    }

    fn materialise(&self, string_table: &mut StringTable) -> Result<FileTokens, CompilerError> {
        let source_path = materialise_path(&self.source_path, string_table);
        let remap = self
            .pool
            .iter()
            .map(|text| string_table.intern(text))
            .collect::<Vec<_>>();
        let mut tokens = Vec::with_capacity(self.tokens.len());
        for token in self.tokens.iter() {
            let mut materialised = token.clone();
            materialised.try_remap_string_ids(&mut |id| {
                let index = id.index() as usize;
                remap.get(index).copied().ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "frozen token payload references out-of-range pool entry {index}"
                    ))
                })
            })?;
            tokens.push(materialised);
        }
        Ok(FileTokens::new(source_path, tokens))
    }
}

impl StableSourceLocation {
    fn capture(location: &SourceLocation, string_table: &StringTable) -> Self {
        Self {
            scope: stable_path(&location.scope, string_table),
            start: location.start_pos,
            end: location.end_pos,
        }
    }

    fn materialise(&self, string_table: &mut StringTable) -> SourceLocation {
        SourceLocation::new(
            materialise_path(&self.scope, string_table),
            self.start,
            self.end,
        )
    }
}

fn stable_path(path: &InternedPath, string_table: &StringTable) -> Box<[String]> {
    path.as_components()
        .iter()
        .map(|component| string_table.resolve(*component).to_owned())
        .collect()
}

fn materialise_path(path: &[String], string_table: &mut StringTable) -> InternedPath {
    InternedPath::from_components(
        path.iter()
            .map(|component| string_table.intern(component))
            .collect(),
    )
}

/// Immutable requester-owned definition used to project one nominal into a generated-local
/// type environment.
///
/// The blueprint carries owned names, stable type identities and declaration-local generic
/// parameter slots only. It contains no requester `TypeId`, `NominalTypeId`, `GenericParameterId`,
/// `InternedPath` or `StringId`. Registering every shell before populating members makes mutually
/// referential definitions safe without reopening the requester environment during materialisation.
#[derive(Clone, PartialEq, Eq)]
struct NominalMaterialisationBlueprint {
    generic_parameters: Box<[NominalGenericParameterBlueprint]>,
    definition: NominalMaterialisationDefinition,
}

#[derive(Clone, PartialEq, Eq)]
struct NominalGenericParameterBlueprint {
    name: String,
    exported_identity: Option<ExportedGenericParameterIdentity>,
    bounds: Box<[CanonicalTraitIdentity]>,
}

#[derive(Clone, PartialEq, Eq)]
enum NominalMaterialisationDefinition {
    Struct {
        fields: Box<[NominalFieldBlueprint]>,
        const_record: bool,
    },
    Choice {
        variants: Box<[NominalChoiceVariantBlueprint]>,
    },
}

#[derive(Clone, PartialEq, Eq)]
struct NominalFieldBlueprint {
    name: String,
    field_type: MaterialisationTypeBlueprint,
    folded_default: Option<PublicFoldedValue>,
}

#[derive(Clone, PartialEq, Eq)]
struct NominalChoiceVariantBlueprint {
    name: String,
    tag: usize,
    payload_fields: Box<[NominalFieldBlueprint]>,
}

/// Closed type shape used inside a nominal blueprint.
///
/// Canonical identities cover stable closed leaves. Declaration-local parameters and shapes that
/// contain them remain explicit so private generic nominals never acquire exported parameter
/// identities merely to support generated-local layout.
#[derive(Clone, PartialEq, Eq)]
enum MaterialisationTypeBlueprint {
    Canonical(CanonicalTypeIdentity),
    GenericParameter(usize),
    Collection {
        element: Box<MaterialisationTypeBlueprint>,
        fixed_capacity: Option<usize>,
    },
    OrderedMap {
        key: Box<MaterialisationTypeBlueprint>,
        value: Box<MaterialisationTypeBlueprint>,
    },
    Option(Box<MaterialisationTypeBlueprint>),
    FallibleCarrier {
        success: Box<MaterialisationTypeBlueprint>,
        error: Box<MaterialisationTypeBlueprint>,
    },
    Tuple(Box<[MaterialisationTypeBlueprint]>),
    GenericInstance {
        base: CanonicalTypeIdentity,
        arguments: Box<[MaterialisationTypeBlueprint]>,
    },
}

/// Published generated-function metadata for one successful declaring module.
///
/// The context is deliberately a compact list rather than a donor-module snapshot. It owns one
/// module-wide stable semantic closure and one retained body per template. Modules without
/// retained bodies publish no context.
#[derive(Clone)]
pub(crate) struct ModuleMaterialisationContext {
    declaration_closure: Box<[PublicDeclarationRecord]>,
    evidence: Box<[PublicEvidenceRecord]>,
    semantic_closure: StableSemanticClosure,
    artefacts: Box<[GenericTemplateArtefact]>,
}

/// Module-owned facts needed to reconstruct private declarations referenced by retained bodies.
///
/// Public/imported declarations continue to use the immutable public-interface closure above.
/// Private constants and aliases have no cross-module origin, so they are retained once here as
/// stable values and selected by each artefact's local-path list.
#[derive(Clone, Default)]
struct StableSemanticClosure {
    constants: Box<[StableLocalConstant]>,
    aliases: Box<[StableLocalAlias]>,
    traits: Box<[StablePrivateTrait]>,
    evidence: Box<[StablePrivateEvidence]>,
}

#[derive(Clone)]
struct StableLocalConstant {
    local_path: Box<[String]>,
    type_identity: CanonicalTypeIdentity,
    value: PublicFoldedValue,
}

#[derive(Clone)]
struct StableLocalAlias {
    local_path: Box<[String]>,
    target_type_identity: CanonicalTypeIdentity,
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

#[derive(Clone)]
struct GenericTemplateArtefact {
    declaration_identity: GeneratedDeclarationIdentity,
    generic_parameter_owner: Option<GenericDeclarationOrigin>,
    receiver: Option<StableReceiverKey>,
    receiver_nominal_identity: Option<CanonicalTypeIdentity>,
    function_path: Box<[String]>,
    source_file: Box<[String]>,
    declaration_location: StableSourceLocation,
    body: StableBodySyntax,
    signature: StableFunctionSignature,
    generic_parameters: Box<[StableGenericParameter]>,
    visibility: StableFileVisibility,
    declarations: Box<[StableDeclarationBinding]>,
    local_declarations: Box<[Box<[String]>]>,
    callables: Box<[StableCallableBinding]>,
    nominals: Box<[StableNominalBinding]>,
    nominal_blueprints: FxHashMap<CanonicalTypeIdentity, NominalMaterialisationBlueprint>,
}

#[derive(Clone)]
struct StableGenericParameter {
    name: String,
    exported_identity: Option<ExportedGenericParameterIdentity>,
    bounds: Box<[CanonicalTraitIdentity]>,
}

#[derive(Clone)]
struct StableFunctionSignature {
    parameters: Box<[StableFunctionParameter]>,
    returns: Box<[StableFunctionReturn]>,
}

#[derive(Clone)]
struct StableFunctionParameter {
    name: String,
    value_mode: ValueMode,
    reactive: bool,
    folded_default: Option<PublicFoldedValue>,
    parameter_type: MaterialisationTypeBlueprint,
    location: StableSourceLocation,
}

#[derive(Clone)]
struct StableFunctionReturn {
    return_type: MaterialisationTypeBlueprint,
    channel: ReturnChannel,
}

#[derive(Clone)]
struct StableDeclarationBinding {
    local_path: Box<[String]>,
    origin: OriginDeclarationId,
}

#[derive(Clone)]
struct StableCallableBinding {
    local_path: Box<[String]>,
    target: StableFunctionTarget,
    signature: StableFunctionSignature,
    summary: PublicCallSummary,
}

#[derive(Clone)]
struct StableNominalBinding {
    local_path: Box<[String]>,
    identity: CanonicalTypeIdentity,
}

#[derive(Clone)]
enum StableFunctionTarget {
    Imported(OriginFunctionId),
    Generated(GeneratedFunctionIdentity),
    ModulePrivate(ModulePrivateExecutableIdentity),
}

#[derive(Clone)]
enum StableReceiverKey {
    Struct(Box<[String]>),
    Choice(Box<[String]>),
}

impl StableReceiverKey {
    fn capture(receiver: &ReceiverKey, string_table: &StringTable) -> Result<Self, CompilerError> {
        match receiver {
            ReceiverKey::Struct(path) => Ok(Self::Struct(stable_path(path, string_table))),
            ReceiverKey::Choice(path) => Ok(Self::Choice(stable_path(path, string_table))),
            ReceiverKey::External(_) | ReceiverKey::BuiltinScalar(_) => {
                Err(CompilerError::compiler_error(
                    "Retained receiver method has a non-source receiver key",
                ))
            }
        }
    }

    fn materialise(&self, string_table: &mut StringTable) -> ReceiverKey {
        match self {
            Self::Struct(path) => ReceiverKey::Struct(materialise_path(path, string_table)),
            Self::Choice(path) => ReceiverKey::Choice(materialise_path(path, string_table)),
        }
    }
}

#[derive(Clone, Default)]
struct StableFileVisibility {
    source_names: Box<[StableVisibleDeclaration]>,
    type_alias_names: Box<[StableVisibleDeclaration]>,
    trait_names: Box<[StableVisibleDeclaration]>,
    external_symbols: Box<[StableExternalSymbol]>,
    namespace_records: Box<[StableNamespaceBinding]>,
    receiver_methods: Box<[StableReceiverMethod]>,
}

#[derive(Clone)]
struct StableVisibleDeclaration {
    visible_name: String,
    local_path: Box<[String]>,
    origin: Option<OriginDeclarationId>,
}

#[derive(Clone)]
struct StableExternalSymbol {
    visible_name: String,
    identity: CanonicalBindingSymbolIdentity,
}

#[derive(Clone)]
struct StableNamespaceBinding {
    visible_name: String,
    record: StableNamespaceRecord,
}

#[derive(Clone)]
struct StableNamespaceRecord {
    record_source: StableNamespaceRecordSource,
    value_members: Box<[StableNamespaceValueMember]>,
    type_members: Box<[StableNamespaceTypeMember]>,
    child_namespaces: Box<[StableNamespaceBinding]>,
}

#[derive(Clone)]
enum StableNamespaceRecordSource {
    SourceFile(Box<[String]>),
    ExternalPackage(String),
}

#[derive(Clone)]
enum StableNamespaceValueMember {
    Source(StableVisibleDeclaration),
    External {
        visible_name: String,
        identity: CanonicalBindingSymbolIdentity,
    },
}

#[derive(Clone)]
enum StableNamespaceTypeMember {
    Source(StableVisibleDeclaration),
    External {
        visible_name: String,
        identity: CanonicalBindingSymbolIdentity,
    },
}

#[derive(Clone)]
struct StableReceiverMethod {
    visible_name: String,
    local_path: Box<[String]>,
    target: StableFunctionTarget,
    receiver: StableReceiverKey,
    signature: StableFunctionSignature,
    summary: Option<PublicCallSummary>,
    generic_parameters: Box<[StableGenericParameter]>,
    location: StableSourceLocation,
}

trait MaterialisationNominalSource {
    fn nominal_blueprint(
        &self,
        identity: &CanonicalTypeIdentity,
    ) -> Option<&NominalMaterialisationBlueprint>;
}

impl MaterialisationNominalSource for ModuleMaterialisationPreparation {
    fn nominal_blueprint(
        &self,
        identity: &CanonicalTypeIdentity,
    ) -> Option<&NominalMaterialisationBlueprint> {
        self.nominal_blueprints.get(identity)
    }
}

impl MaterialisationNominalSource for GenericTemplateArtefact {
    fn nominal_blueprint(
        &self,
        identity: &CanonicalTypeIdentity,
    ) -> Option<&NominalMaterialisationBlueprint> {
        self.nominal_blueprints.get(identity)
    }
}

struct MaterialisationNominalOriginResolver<'a> {
    type_environment: &'a TypeEnvironment,
}

struct GeneratedFoldedValueMaterialiser<'a, 'b, N: MaterialisationNominalSource> {
    type_environment: &'a mut TypeEnvironment,
    external_registry: &'b ExternalPackageRegistry,
    nominal_source: &'b N,
    template_ir_store: Rc<RefCell<crate::compiler_frontend::ast::templates::tir::TemplateIrStore>>,
}

impl<N: MaterialisationNominalSource> FoldedValueMaterialiser
    for GeneratedFoldedValueMaterialiser<'_, '_, N>
{
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
        )
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
    fn capture(target: &SourceFunctionTarget) -> Option<Self> {
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

    fn materialise(&self, local_path: InternedPath) -> SourceFunctionTarget {
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

impl MaterialisationTypeBlueprint {
    fn collect_nominal_identities(&self, identities: &mut FxHashSet<CanonicalTypeIdentity>) {
        match self {
            Self::Canonical(identity) => {
                if matches!(
                    identity,
                    CanonicalTypeIdentity::SourceNominal(_)
                        | CanonicalTypeIdentity::ModulePrivateNominal(_)
                ) {
                    identities.insert(identity.clone());
                }
            }
            Self::GenericParameter(_) => {}
            Self::Collection { element, .. } | Self::Option(element) => {
                element.collect_nominal_identities(identities);
            }
            Self::OrderedMap { key, value } => {
                key.collect_nominal_identities(identities);
                value.collect_nominal_identities(identities);
            }
            Self::FallibleCarrier { success, error } => {
                success.collect_nominal_identities(identities);
                error.collect_nominal_identities(identities);
            }
            Self::Tuple(elements) => {
                for element in elements {
                    element.collect_nominal_identities(identities);
                }
            }
            Self::GenericInstance { base, arguments } => {
                identities.insert(base.clone());
                for argument in arguments {
                    argument.collect_nominal_identities(identities);
                }
            }
        }
    }
}

fn stable_body_symbol_names(tokens: &FileTokens, string_table: &StringTable) -> FxHashSet<String> {
    tokens
        .tokens
        .iter()
        .filter_map(|token| match token.kind {
            TokenKind::Symbol(symbol) => Some(string_table.resolve(symbol).to_owned()),
            _ => None,
        })
        .collect()
}

fn collect_namespace_source_paths(
    record: &NamespaceRecord,
    selected: &mut FxHashSet<InternedPath>,
) {
    for member in record.value_members.values() {
        if let NamespaceValueMember::SourceDeclaration(target) = member {
            selected.insert(target.local_path().clone());
        }
    }
    for member in record.type_members.values() {
        if let NamespaceTypeMember::SourceDeclaration(target) = member {
            selected.insert(target.local_path().clone());
        }
    }
    for child in record.child_namespaces.values() {
        collect_namespace_source_paths(child, selected);
    }
}

impl ModuleMaterialisationContext {
    /// Build a test-only context with one artefact per identity and no real body payload.
    ///
    /// WHY: build-system tests need to exercise publication duplicate detection and exact row
    ///      indexing without preparing a full generic module.
    #[cfg(test)]
    pub(crate) fn from_identities_for_test(identities: Vec<GeneratedDeclarationIdentity>) -> Self {
        let artefacts = identities
            .into_iter()
            .map(|declaration_identity| GenericTemplateArtefact {
                declaration_identity,
                generic_parameter_owner: None,
                receiver: None,
                receiver_nominal_identity: None,
                function_path: Box::new([]),
                source_file: Box::new([]),
                declaration_location: StableSourceLocation {
                    scope: Box::new([]),
                    start: crate::compiler_frontend::tokenizer::tokens::CharPosition::default(),
                    end: crate::compiler_frontend::tokenizer::tokens::CharPosition::default(),
                },
                body: StableBodySyntax {
                    source_path: Box::new([]),
                    pool: Box::new([]),
                    tokens: Box::new([]),
                },
                signature: StableFunctionSignature {
                    parameters: Box::new([]),
                    returns: Box::new([]),
                },
                generic_parameters: Box::new([]),
                visibility: StableFileVisibility::default(),
                declarations: Box::new([]),
                local_declarations: Box::new([]),
                callables: Box::new([]),
                nominals: Box::new([]),
                nominal_blueprints: FxHashMap::default(),
            })
            .collect::<Box<[_]>>();
        Self {
            declaration_closure: Box::new([]),
            evidence: Box::new([]),
            semantic_closure: StableSemanticClosure::default(),
            artefacts,
        }
    }

    #[cfg(test)]
    pub(crate) fn contains_template(&self, identity: &GeneratedDeclarationIdentity) -> bool {
        self.artefacts
            .iter()
            .any(|artefact| &artefact.declaration_identity == identity)
    }

    /// Iterate every published template with its exact dense row index.
    ///
    /// WHAT: lets the boundary publication index point at one exact template row instead of
    ///       re-searching all artefacts at materialisation time.
    /// WHY: generated materialisation is request-driven; a direct row index keeps lookup
    ///      proportional to one identity and detects duplicates inside one context.
    pub(crate) fn declaration_rows(
        &self,
    ) -> impl Iterator<Item = (&GeneratedDeclarationIdentity, usize)> + '_ {
        self.artefacts
            .iter()
            .enumerate()
            .map(|(index, artefact)| (&artefact.declaration_identity, index))
    }

    /// Materialise the exact template row selected by the boundary publication index.
    pub(crate) fn materialise_ast_at(
        &self,
        template_index: usize,
        input: ModuleMaterialisationInput<'_>,
    ) -> Result<MaterialisedGenericAst, CompilerMessages> {
        let artefact = self.artefacts.get(template_index).ok_or_else(|| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error(format!(
                    "Published materialisation context has no template row {template_index}"
                )),
                &input.requester_context.string_table,
            )
        })?;
        check_materialisation_row_identity(artefact, input.identity).map_err(|error| {
            CompilerMessages::from_error_ref(error, &input.requester_context.string_table)
        })?;
        artefact.materialise_ast(self, input)
    }
}

/// Verify that one indexed template row belongs to the requested generated identity.
///
/// WHAT: a stale but in-range row must fail as an internal invariant error instead of
///       materialising the wrong generic declaration.
/// WHY: the boundary publication index is exact by contract; identity disagreement means the
///      index or the retained artefact lane is corrupt.
fn check_materialisation_row_identity(
    artefact: &GenericTemplateArtefact,
    identity: &GeneratedFunctionIdentity,
) -> Result<(), CompilerError> {
    if artefact.declaration_identity != *identity.declaration() {
        return Err(CompilerError::compiler_error(format!(
            "Published materialisation row holds declaration identity {:?} but request {:?} selected it",
            artefact.declaration_identity,
            identity.declaration()
        )));
    }
    Ok(())
}

impl GenericTemplateArtefact {
    fn materialise_ast(
        &self,
        context: &ModuleMaterialisationContext,
        input: ModuleMaterialisationInput<'_>,
    ) -> Result<MaterialisedGenericAst, CompilerMessages> {
        let ModuleMaterialisationInput {
            identity,
            requester_context,
            requester_call_location,
            external_package_registry,
            style_directives,
            build_profile,
            project_path_resolver,
            template_const_loop_iteration_limit,
            #[cfg(feature = "timers")]
            timing_context,
        } = input;
        let project_path_resolver = project_path_resolver.ok_or_else(|| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error(
                    "Stable generated materialisation has no active project path resolver",
                ),
                &requester_context.string_table,
            )
        })?;
        let mut string_table = StringTable::new();
        let requester_string_remap = string_table.merge_from(&requester_context.string_table);
        let mut call_location = requester_call_location.clone();
        call_location.remap_string_ids(&requester_string_remap);

        let source_file = materialise_path(&self.source_file, &mut string_table);
        let function_path = materialise_path(&self.function_path, &mut string_table);
        let entry_dir = source_file.parent().unwrap_or_default();
        let build_context = AstBuildContext {
            external_package_registry: Arc::new(external_package_registry.clone()),
            style_directives,
            string_table: &mut string_table,
            entry_dir,
            root_role: ModuleRootRole::Support,
            build_profile,
            project_path_resolver: Some(project_path_resolver),
            path_format_config: PathStringFormatConfig::default(),
            template_const_loop_iteration_limit,
            capacity_estimate: FrontendArenaCapacityEstimate::default(),
            #[cfg(feature = "timers")]
            timing_context,
            #[cfg(feature = "timers")]
            timing_metric_family: crate::compiler_frontend::ast::AstTimingMetricFamily::Generated,
        };
        let (phase_context, string_table_ref) = AstPhaseContext::from_build_context(build_context);
        crate::timing_scope_attributed!(
            timing_guard_generated_ast_total,
            crate::timing::TimingMetric::FrontendGeneratedAstTotal,
            timing_context
        );
        let import_environment = self
            .materialise_import_environment(
                context,
                &source_file,
                external_package_registry,
                string_table_ref,
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table_ref))?;
        let builtin_manifest =
            crate::compiler_frontend::builtins::error_type::register_builtin_error_types(
                string_table_ref,
            );
        let mut module_symbols = ModuleSymbols::empty();
        module_symbols
            .builtin_visible_symbol_paths
            .extend(builtin_manifest.visible_symbol_paths.iter().cloned());
        module_symbols.declarations = builtin_manifest.declarations;
        module_symbols
            .resolved_struct_fields_by_path
            .extend(builtin_manifest.resolved_struct_fields_by_path);
        module_symbols
            .struct_source_by_path
            .extend(builtin_manifest.struct_source_by_path);
        module_symbols
            .builtin_struct_ast_nodes
            .extend(builtin_manifest.ast_struct_nodes);
        let mut environment = AstModuleEnvironmentBuilder::new(&phase_context).build(
            &[],
            AstEnvironmentInput {
                module_symbols,
                import_environment,
            },
            string_table_ref,
        )?;
        self.install_closed_environment(
            context,
            &mut environment,
            external_package_registry,
            &phase_context.template_ir_store,
            string_table_ref,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table_ref))?;

        let mut type_arguments = Vec::with_capacity(identity.type_arguments().len());
        for canonical_identity in identity.type_arguments() {
            type_arguments.push(
                intern_generated_canonical_type(
                    canonical_identity,
                    &mut environment.type_environment,
                    external_package_registry,
                    requester_context,
                    string_table_ref,
                )
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table_ref))?,
            );
        }
        install_generated_request_evidence(
            identity,
            requester_context,
            &requester_string_remap,
            &mut environment,
            string_table_ref,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table_ref))?;

        let instance_path = function_path.join_str("__generated_instance", string_table_ref);
        let request = GenericFunctionInstantiationRequest {
            declaration_identity: Some(identity.declaration().clone()),
            evidence: Box::new([]),
            key: GenericFunctionInstanceKey {
                function_path,
                type_arguments: type_arguments.into_boxed_slice(),
            },
            instance_path: instance_path.clone(),
            call_location,
        };
        let emitted = {
            crate::timing_scope_attributed!(
                timing_guard_generated_ast_emit,
                crate::timing::TimingMetric::FrontendGeneratedAstEmit,
                timing_context
            );
            AstEmitter::new(&phase_context, &mut environment, 1)
                .emit_generated_request(request, string_table_ref)?
        };
        let mut build_result = {
            crate::timing_scope_attributed!(
                timing_guard_generated_ast_finalise,
                crate::timing::TimingMetric::FrontendGeneratedAstFinalise,
                timing_context
            );
            AstFinalizer::new(&phase_context, environment).finalize(
                emitted,
                &[],
                string_table_ref,
            )?
        };
        build_result
            .materialisation_context
            .inherit_nominal_blueprints(requester_context)
            .and_then(|()| {
                build_result
                    .materialisation_context
                    .inherit_artefact_nominal_blueprints(self)
            })
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table_ref))?;
        Ok(MaterialisedGenericAst {
            build_result,
            string_table,
            instance_path,
        })
    }

    fn materialise_import_environment(
        &self,
        context: &ModuleMaterialisationContext,
        source_file: &InternedPath,
        external_package_registry: &ExternalPackageRegistry,
        string_table: &mut StringTable,
    ) -> Result<HeaderImportEnvironment, CompilerError> {
        let mut environment = HeaderImportEnvironment::default();
        environment.file_visibility_by_source.insert(
            source_file.clone(),
            self.visibility
                .materialise(external_package_registry, string_table)?,
        );
        for binding in &self.declarations {
            let local_path = materialise_path(&binding.local_path, string_table);
            environment
                .imported_declarations_by_local_path
                .insert(local_path, binding.origin.clone());
        }
        for record in &context.declaration_closure {
            environment
                .imported_declarations_by_origin
                .insert(record.origin.clone(), record.clone());
        }
        for record in &context.evidence {
            environment
                .imported_evidence_by_identity
                .insert(record.identity.clone(), record.clone());
        }
        for callable in &self.callables {
            let local_path = materialise_path(&callable.local_path, string_table);
            let target = callable.target.materialise(local_path.clone());
            let SourceFunctionTarget::Imported { origin, .. } = &target else {
                // Generated and module-private callables materialise through the generated
                // function lanes; only imported provider callables enter the header contract
                // tables.
                continue;
            };
            environment
                .imported_call_summaries_by_origin
                .insert(origin.clone(), callable.summary.clone());
            environment
                .imported_functions_by_local_path
                .insert(local_path, ImportedFunctionContract { target });
        }
        Ok(environment)
    }

    fn install_closed_environment(
        &self,
        context: &ModuleMaterialisationContext,
        environment: &mut AstModuleEnvironment,
        external_package_registry: &ExternalPackageRegistry,
        template_ir_store: &Rc<
            RefCell<crate::compiler_frontend::ast::templates::tir::TemplateIrStore>,
        >,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerError> {
        for nominal in &self.nominals {
            let type_id = intern_generated_canonical_type(
                &nominal.identity,
                &mut environment.type_environment,
                external_package_registry,
                self,
                string_table,
            )?;
            let local_path = materialise_path(&nominal.local_path, string_table);
            environment
                .type_environment
                .register_nominal_path_alias(local_path.clone(), type_id)?;
            let generated_nominal_path = environment
                .type_environment
                .nominal_path(type_id)
                .cloned()
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Materialised nominal binding has no generated environment path",
                    )
                })?;
            let generic_metadata =
                materialised_generic_nominal_metadata(type_id, &environment.type_environment)?;
            let lookups = Rc::make_mut(&mut environment.lookups);
            Rc::make_mut(&mut lookups.nominal_type_ids_by_path).insert(local_path.clone(), type_id);
            Rc::make_mut(&mut lookups.source_nominal_paths).insert(local_path.clone());
            if let Some(metadata) = generic_metadata {
                let declarations = Rc::make_mut(&mut lookups.generic_declarations_by_path);
                declarations
                    .entry(local_path.clone())
                    .or_insert_with(|| metadata.clone());
                declarations
                    .entry(generated_nominal_path.clone())
                    .or_insert(metadata);
            }
            if !lookups
                .resolved_struct_fields_by_path
                .contains_key(&generated_nominal_path)
                && let Some(blueprint) = self.nominal_blueprints.get(&nominal.identity)
                && let Some(fields) = materialised_struct_fields(
                    type_id,
                    &mut environment.type_environment,
                    blueprint,
                    self,
                    external_package_registry,
                    template_ir_store,
                    string_table,
                )?
            {
                Rc::make_mut(&mut lookups.resolved_struct_fields_by_path)
                    .insert(generated_nominal_path, fields);
            }
            if lookups.declaration_table.get_by_path(&local_path).is_none() {
                append_materialised_declaration(
                    lookups,
                    materialised_nominal_declaration(
                        local_path.clone(),
                        type_id,
                        &environment.type_environment,
                    )?,
                )?;
            }
        }

        // Private traits are part of the declaration environment needed to resolve bounds on
        // nested retained templates. Install them after nominal shells exist, but before any
        // nested signature or bound reconstruction consumes the generated trait table.
        install_private_semantic_closure(
            self,
            context,
            environment,
            external_package_registry,
            template_ir_store,
            string_table,
        )?;

        for callable in &self.callables {
            if self
                .declarations
                .iter()
                .any(|declaration| declaration.local_path == callable.local_path)
            {
                continue;
            }
            let local_path = materialise_path(&callable.local_path, string_table);
            let (signature, function_type_id, fallible_carrier_type_id) =
                callable.signature.materialise(
                    &local_path,
                    &mut StableFunctionMaterialisationContext {
                        generic_parameter_type_ids: &[],
                        nominal_source: self,
                        type_environment: &mut environment.type_environment,
                        external_package_registry,
                        template_ir_store,
                        string_table,
                    },
                )?;
            let declaration = Declaration {
                id: local_path.clone(),
                value: Expression::new(
                    ExpressionKind::NoValue,
                    Default::default(),
                    function_type_id,
                    DataType::Function(Box::new(None), signature.clone()),
                    ValueMode::ImmutableReference,
                ),
            };
            let lookups = Rc::make_mut(&mut environment.lookups);
            append_materialised_declaration(lookups, declaration)?;
            Rc::make_mut(&mut lookups.resolved_function_signatures_by_path).insert(
                local_path.clone(),
                ResolvedFunctionSignature {
                    receiver: None,
                    signature,
                },
            );
            Rc::make_mut(&mut lookups.declaration_semantics)
                .register_materialised_function(local_path.clone());
            lookups.imported_functions_by_local_path.insert(
                local_path.clone(),
                AstImportedFunctionContract {
                    target: callable.target.materialise(local_path),
                    summary: callable.summary.clone(),
                    fallible_carrier_type_id,
                },
            );
        }

        for local_path_components in &self.local_declarations {
            let local_path = materialise_path(local_path_components, string_table);
            if let Some(constant) = context
                .semantic_closure
                .constants
                .iter()
                .find(|constant| constant.local_path.as_ref() == local_path_components.as_ref())
            {
                let type_id = intern_generated_canonical_type(
                    &constant.type_identity,
                    &mut environment.type_environment,
                    external_package_registry,
                    self,
                    string_table,
                )?;
                let mut materialiser = GeneratedFoldedValueMaterialiser {
                    type_environment: &mut environment.type_environment,
                    external_registry: external_package_registry,
                    nominal_source: self,
                    template_ir_store: Rc::clone(template_ir_store),
                };
                let mut value = materialize_public_folded_value(
                    &mut materialiser,
                    &constant.value,
                    type_id,
                    string_table,
                )?;
                value.value_mode = ValueMode::ImmutableReference;
                let declaration = Declaration {
                    id: local_path.clone(),
                    value,
                };
                let lookups = Rc::make_mut(&mut environment.lookups);
                if lookups.declaration_table.get_by_path(&local_path).is_none() {
                    append_materialised_declaration(lookups, declaration.clone())?;
                }
                lookups.module_constants.push(declaration);
                Rc::make_mut(&mut lookups.declaration_semantics)
                    .register_materialised_constant(local_path);
                continue;
            }

            if let Some(alias) = context
                .semantic_closure
                .aliases
                .iter()
                .find(|alias| alias.local_path.as_ref() == local_path_components.as_ref())
            {
                let type_id = intern_generated_canonical_type(
                    &alias.target_type_identity,
                    &mut environment.type_environment,
                    external_package_registry,
                    self,
                    string_table,
                )?;
                let declaration = Declaration {
                    id: local_path.clone(),
                    value: Expression::new(
                        ExpressionKind::NoValue,
                        Default::default(),
                        type_id,
                        diagnostic_type_spelling(type_id, &environment.type_environment),
                        ValueMode::ImmutableReference,
                    ),
                };
                let lookups = Rc::make_mut(&mut environment.lookups);
                if lookups.declaration_table.get_by_path(&local_path).is_none() {
                    append_materialised_declaration(lookups, declaration)?;
                }
                Rc::make_mut(&mut lookups.resolved_type_aliases_by_path).insert(
                    local_path.clone(),
                    ResolvedTypeAnnotation {
                        source_ref:
                            crate::compiler_frontend::datatypes::parsed::ParsedTypeRef::Inferred,
                        diagnostic_type: diagnostic_type_spelling(
                            type_id,
                            &environment.type_environment,
                        ),
                        type_id: Some(type_id),
                    },
                );
                Rc::make_mut(&mut lookups.declaration_semantics)
                    .register_materialised_value(local_path);
            }
        }

        for method in &self.visibility.receiver_methods {
            let method_path = materialise_path(&method.local_path, string_table);
            if context
                .artefacts
                .iter()
                .any(|nested| materialise_path(&nested.function_path, string_table) == method_path)
            {
                continue;
            }
            if !method.generic_parameters.is_empty() {
                // Imported generic receiver methods are reprojected from their imported nominal
                // declaration. A locally-owned generic receiver method must have a retained
                // artefact, so leaving this path unresolved would hide a broken closure.
                if matches!(method.target, StableFunctionTarget::Imported(_)) {
                    continue;
                }
                return Err(CompilerError::compiler_error(
                    "Generic receiver method has no retained materialisation artefact",
                ));
            }
            let (signature, function_type_id, fallible_carrier_type_id) =
                method.signature.materialise(
                    &method_path,
                    &mut StableFunctionMaterialisationContext {
                        generic_parameter_type_ids: &[],
                        nominal_source: self,
                        type_environment: &mut environment.type_environment,
                        external_package_registry,
                        template_ir_store,
                        string_table,
                    },
                )?;
            let receiver = materialised_receiver_key(&signature, &environment.type_environment)
                .or_else(|_| {
                    Ok::<ReceiverKey, CompilerError>(method.receiver.materialise(string_table))
                })?;
            let declaration = Declaration {
                id: method_path.clone(),
                value: Expression::new(
                    ExpressionKind::NoValue,
                    Default::default(),
                    function_type_id,
                    DataType::Function(Box::new(Some(receiver.clone())), signature.clone()),
                    ValueMode::ImmutableReference,
                ),
            };
            let lookups = Rc::make_mut(&mut environment.lookups);
            if lookups
                .declaration_table
                .get_by_path(&method_path)
                .is_none()
            {
                append_materialised_declaration(lookups, declaration)?;
            }
            Rc::make_mut(&mut lookups.resolved_function_signatures_by_path).insert(
                method_path.clone(),
                ResolvedFunctionSignature {
                    receiver: Some(receiver.clone()),
                    signature: signature.clone(),
                },
            );
            register_materialised_receiver_method(
                lookups,
                method_path.clone(),
                receiver,
                signature,
            )?;
            Rc::make_mut(&mut lookups.declaration_semantics)
                .register_materialised_function(method_path.clone());
            lookups.imported_functions_by_local_path.insert(
                method_path.clone(),
                AstImportedFunctionContract {
                    target: method.target.materialise(method_path),
                    summary: method.summary.clone().ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Concrete receiver method has no retained call summary",
                        )
                    })?,
                    fallible_carrier_type_id,
                },
            );
        }

        let selected_paths = self.visibility.materialised_selected_paths(string_table);
        for nested in &context.artefacts {
            let nested_path = materialise_path(&nested.function_path, string_table);
            if nested.declaration_identity != self.declaration_identity
                && !selected_paths.contains(&nested_path)
            {
                continue;
            }
            let (generic_parameter_list_id, generic_parameter_type_ids) = if let Some(
                nominal_origin,
            ) = nested
                .generic_parameter_owner
                .as_ref()
                .and_then(GenericDeclarationOrigin::nominal_type_origin)
            {
                // A generic receiver method shares the enclosing nominal's local generic
                // handles. Registering a second list for the method would assign one stable
                // exported parameter identity to two TypeIds and make its generated sidecar
                // internally inconsistent.
                let nominal_identity = CanonicalTypeIdentity::SourceNominal(nominal_origin.clone());
                let nominal_type_id = intern_generated_canonical_type(
                    &nominal_identity,
                    &mut environment.type_environment,
                    external_package_registry,
                    self,
                    string_table,
                )?;
                let generic_parameter_list_id =
                    match environment.type_environment.get(nominal_type_id) {
                        Some(TypeDefinition::Struct(definition)) => definition.generic_parameters,
                        Some(TypeDefinition::Choice(definition)) => definition.generic_parameters,
                        _ => {
                            return Err(CompilerError::compiler_error(
                                "Generic receiver template owner is not a nominal type",
                            ));
                        }
                    }
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Generic receiver template owner has no generic parameter list",
                        )
                    })?;
                let generic_parameter_type_ids = environment
                    .type_environment
                    .generic_parameters(generic_parameter_list_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Generic receiver template owner has a missing parameter list",
                        )
                    })?
                    .parameters
                    .iter()
                    .map(|parameter| {
                        environment
                            .type_environment
                            .type_id_for_generic_parameter(parameter.id)
                            .ok_or_else(|| {
                                CompilerError::compiler_error(
                                    "Generic receiver template owner parameter has no type handle",
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, CompilerError>>()?;
                if generic_parameter_type_ids.len() != nested.generic_parameters.len() {
                    return Err(CompilerError::compiler_error(
                        "Generic receiver template parameter count disagrees with its nominal owner",
                    ));
                }
                for (parameter, type_id) in nested
                    .generic_parameters
                    .iter()
                    .zip(&generic_parameter_type_ids)
                {
                    if let Some(exported_identity) = parameter.exported_identity.as_ref() {
                        let expected_identity =
                            CanonicalTypeIdentity::GenericParameter(exported_identity.clone());
                        environment
                            .type_environment
                            .register_canonical_identity(expected_identity, *type_id)?;
                    }
                }
                (generic_parameter_list_id, generic_parameter_type_ids)
            } else if nested.receiver.is_some() {
                let receiver_identity = nested.receiver_nominal_identity.as_ref().ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Materialised generic receiver template {:?} has no retained nominal identity",
                        nested_path
                    ))
                })?;
                let receiver_type_id = intern_generated_canonical_type(
                    receiver_identity,
                    &mut environment.type_environment,
                    external_package_registry,
                    self,
                    string_table,
                )?;
                let generic_parameter_list_id =
                    match environment.type_environment.get(receiver_type_id) {
                        Some(TypeDefinition::Struct(definition)) => definition.generic_parameters,
                        Some(TypeDefinition::Choice(definition)) => definition.generic_parameters,
                        _ => {
                            return Err(CompilerError::compiler_error(
                                "Private generic receiver owner is not a nominal type",
                            ));
                        }
                    }
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Private generic receiver owner has no generic parameter list",
                        )
                    })?;
                let generic_parameter_type_ids = environment
                    .type_environment
                    .generic_parameters(generic_parameter_list_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Private generic receiver owner has a missing parameter list",
                        )
                    })?
                    .parameters
                    .iter()
                    .map(|parameter| {
                        environment
                            .type_environment
                            .type_id_for_generic_parameter(parameter.id)
                            .ok_or_else(|| {
                                CompilerError::compiler_error(
                                    "Private generic receiver owner parameter has no type handle",
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, CompilerError>>()?;
                if generic_parameter_type_ids.len() != nested.generic_parameters.len() {
                    return Err(CompilerError::compiler_error(
                        "Private generic receiver template parameter count disagrees with its nominal owner",
                    ));
                }
                self.restore_private_receiver_generic_parameter_bounds(
                    nested,
                    receiver_identity,
                    generic_parameter_list_id,
                    &mut environment.type_environment,
                    &environment.lookups.trait_environment,
                    string_table,
                )?;
                (generic_parameter_list_id, generic_parameter_type_ids)
            } else {
                let parsed_parameters = GenericParameterList {
                    parameters: nested
                        .generic_parameters
                        .iter()
                        .enumerate()
                        .map(|(slot, parameter)| GenericParameter {
                            id: TypeParameterId(slot as u32),
                            name: string_table.intern(&parameter.name),
                            location: Default::default(),
                            trait_bounds: Vec::new(),
                        })
                        .collect(),
                };
                let registration = environment
                    .type_environment
                    .register_generic_parameter_list(&parsed_parameters, &FxHashMap::default());
                let generic_parameter_type_ids = (0..nested.generic_parameters.len())
                    .map(|slot| {
                        let parameter_id = registration
                            .canonical_by_local
                            .get(&TypeParameterId(slot as u32))
                            .copied()
                            .ok_or_else(|| {
                                CompilerError::compiler_error(
                                    "Materialised generic template omitted a parameter slot",
                                )
                            })?;
                        environment
                            .type_environment
                            .type_id_for_generic_parameter(parameter_id)
                            .ok_or_else(|| {
                                CompilerError::compiler_error(
                                    "Materialised generic template parameter has no type handle",
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, CompilerError>>()?;
                let mut resolved_bounds_by_local = FxHashMap::default();
                for (slot, parameter) in nested.generic_parameters.iter().enumerate() {
                    let local_id = TypeParameterId(slot as u32);
                    let parameter_id = registration
                        .canonical_by_local
                        .get(&local_id)
                        .copied()
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "Materialised generic template omitted a stable parameter slot",
                            )
                        })?;
                    let parameter_type_id = environment
                        .type_environment
                        .type_id_for_generic_parameter(parameter_id)
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "Materialised generic template parameter has no type identity",
                            )
                        })?;
                    if let Some(exported_identity) = &parameter.exported_identity {
                        environment.type_environment.register_canonical_identity(
                            CanonicalTypeIdentity::GenericParameter(exported_identity.clone()),
                            parameter_type_id,
                        )?;
                    }
                    let bounds = parameter
                            .bounds
                            .iter()
                            .map(|identity| {
                                environment
                                    .lookups
                                    .trait_environment
                                    .id_for_canonical_identity(identity)
                                    .ok_or_else(|| {
                                        CompilerError::compiler_error(
                                            "Materialised generic bound is absent from its trait closure",
                                        )
                                    })
                            })
                            .collect::<Result<Vec<_>, CompilerError>>()?;
                    resolved_bounds_by_local.insert(local_id, bounds);
                }
                environment
                    .type_environment
                    .update_generic_parameter_bounds(
                        registration.list_id,
                        &resolved_bounds_by_local,
                        &registration.canonical_by_local,
                    );
                (registration.list_id, generic_parameter_type_ids)
            };
            let (signature, function_type_id, _) = nested.signature.materialise(
                &nested_path,
                &mut StableFunctionMaterialisationContext {
                    generic_parameter_type_ids: &generic_parameter_type_ids,
                    nominal_source: nested,
                    type_environment: &mut environment.type_environment,
                    external_package_registry,
                    template_ir_store,
                    string_table,
                },
            )?;
            let receiver = if nested.receiver.is_some() {
                Some(materialised_receiver_key(
                    &signature,
                    &environment.type_environment,
                )?)
            } else {
                None
            };
            let template = GenericFunctionTemplate {
                function_path: nested_path.clone(),
                source_file: materialise_path(&nested.source_file, string_table),
                declaration_identity: Some(nested.declaration_identity.clone()),
                generic_parameter_owner: nested.generic_parameter_owner.clone(),
                generic_parameter_list_id,
                signature: signature.clone(),
                body_tokens: Some(nested.body.materialise(string_table)?),
                declaration_location: nested.declaration_location.materialise(string_table),
            };
            let lookups = Rc::make_mut(&mut environment.lookups);
            lookups
                .generic_function_templates_by_path
                .insert(nested_path.clone(), template);
            Rc::make_mut(&mut lookups.resolved_function_signatures_by_path).insert(
                nested_path.clone(),
                ResolvedFunctionSignature {
                    receiver: receiver.clone(),
                    signature: signature.clone(),
                },
            );
            if let Some(receiver) = receiver.clone() {
                register_materialised_receiver_method(
                    lookups,
                    nested_path.clone(),
                    receiver,
                    signature.clone(),
                )?;
            }
            if lookups
                .declaration_table
                .get_by_path(&nested_path)
                .is_none()
            {
                append_materialised_declaration(
                    lookups,
                    Declaration {
                        id: nested_path.clone(),
                        value: Expression::new(
                            ExpressionKind::NoValue,
                            Default::default(),
                            function_type_id,
                            DataType::Function(Box::new(receiver), signature),
                            ValueMode::ImmutableReference,
                        ),
                    },
                )?;
            }
            Rc::make_mut(&mut lookups.declaration_semantics)
                .register_materialised_function(nested_path);
        }
        Ok(())
    }
}

fn install_private_semantic_closure(
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

fn materialised_receiver_key(
    signature: &FunctionSignature,
    type_environment: &TypeEnvironment,
) -> Result<ReceiverKey, CompilerError> {
    let receiver_type_id = signature
        .parameters
        .first()
        .map(|parameter| parameter.value.type_id)
        .ok_or_else(|| {
            CompilerError::compiler_error("Receiver method has no receiver parameter")
        })?;
    type_environment
        .receiver_key_for_type_id(receiver_type_id)
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "Receiver method parameter does not resolve to a nominal receiver",
            )
        })
}

fn register_materialised_receiver_method(
    lookups: &mut AstModuleLookups,
    function_path: InternedPath,
    receiver: ReceiverKey,
    signature: FunctionSignature,
) -> Result<(), CompilerError> {
    let method_name = function_path.name().ok_or_else(|| {
        CompilerError::compiler_error(
            "Materialised receiver method path has no final method-name component",
        )
    })?;
    let entry = ReceiverMethodEntry {
        function_path: function_path.clone(),
        receiver: receiver.clone(),
        source_file: function_path.parent().unwrap_or_default(),
        receiver_mutable: signature
            .parameters
            .first()
            .is_some_and(|parameter| parameter.value.value_mode.is_mutable()),
        signature,
    };
    let receiver_methods = Rc::make_mut(&mut lookups.receiver_methods);
    receiver_methods
        .by_receiver_and_name
        .entry((receiver, method_name))
        .or_default()
        .push(entry.clone());
    receiver_methods
        .by_method_name
        .entry(method_name)
        .or_default()
        .push(entry.clone());
    if receiver_methods
        .by_function_path
        .insert(function_path, entry)
        .is_some()
    {
        return Err(CompilerError::compiler_error(
            "Materialised receiver method path was registered more than once",
        ));
    }
    Ok(())
}

fn append_materialised_declaration(
    lookups: &mut AstModuleLookups,
    declaration: Declaration,
) -> Result<(), CompilerError> {
    let path = declaration.id.clone();
    Rc::make_mut(&mut lookups.declaration_table)
        .append_for_construction(declaration)
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Materialised declaration path {path:?} was registered more than once",
            ))
        })?;
    Ok(())
}

impl GenericTemplateArtefact {
    /// Restore the declaration-site bounds shared by a private receiver and its nominal owner.
    ///
    /// WHAT: validates ordered names and canonical bound identities, then patches the generated
    ///       nominal parameter handles with consumer-local `TraitId` values.
    /// WHY: nominal reconstruction creates reusable local handles before the generated trait
    ///       table is available. A private receiver template must not inherit an empty bound
    ///       list, because bound dispatch selects both its method and generated evidence identity.
    fn restore_private_receiver_generic_parameter_bounds(
        &self,
        nested: &GenericTemplateArtefact,
        receiver_identity: &CanonicalTypeIdentity,
        generic_parameter_list_id: GenericParameterListId,
        type_environment: &mut TypeEnvironment,
        trait_environment: &TraitEnvironment,
        string_table: &StringTable,
    ) -> Result<(), CompilerError> {
        let nominal_blueprint =
            self.nominal_blueprints
                .get(receiver_identity)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Private generic receiver owner has no retained nominal blueprint",
                    )
                })?;
        let nominal_parameters = &nominal_blueprint.generic_parameters;
        let local_parameters = type_environment
            .generic_parameters(generic_parameter_list_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Private generic receiver owner has no reconstructed parameter list",
                )
            })?
            .parameters
            .clone();
        if nominal_parameters.len() != nested.generic_parameters.len()
            || nominal_parameters.len() != local_parameters.len()
        {
            return Err(CompilerError::compiler_error(
                "Private generic receiver owner and method parameter lists disagree in arity",
            ));
        }

        let mut bounds_by_local = FxHashMap::default();
        let mut canonical_by_local = FxHashMap::default();
        for (slot, ((nominal_parameter, receiver_parameter), local_parameter)) in nominal_parameters
            .iter()
            .zip(&nested.generic_parameters)
            .zip(&local_parameters)
            .enumerate()
        {
            if nominal_parameter.name != receiver_parameter.name
                || string_table.resolve(local_parameter.name) != nominal_parameter.name
                || nominal_parameter.bounds.as_ref() != receiver_parameter.bounds.as_ref()
            {
                return Err(CompilerError::compiler_error(
                    "Private generic receiver owner and method parameter bounds disagree",
                ));
            }

            let bounds = receiver_parameter
                .bounds
                .iter()
                .map(|identity| {
                    trait_environment
                        .id_for_canonical_identity(identity)
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "Private generic receiver bound is absent from the generated trait table",
                            )
                        })
                })
                .collect::<Result<Vec<_>, CompilerError>>()?;
            let local_id = TypeParameterId(slot as u32);
            canonical_by_local.insert(local_id, local_parameter.id);
            bounds_by_local.insert(local_id, bounds);
        }

        type_environment.update_generic_parameter_bounds(
            generic_parameter_list_id,
            &bounds_by_local,
            &canonical_by_local,
        );
        Ok(())
    }
}

fn materialised_nominal_declaration(
    local_path: InternedPath,
    type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> Result<Declaration, CompilerError> {
    let diagnostic_type = match type_environment.get(type_id) {
        Some(TypeDefinition::Struct(definition)) => DataType::Struct {
            nominal_path: local_path.clone(),
            type_id,
            const_record: definition.const_record,
            generic_instance_key: None,
        },
        Some(TypeDefinition::Choice(_)) => DataType::Choices {
            nominal_path: local_path.clone(),
            type_id,
            generic_instance_key: None,
        },
        _ => {
            return Err(CompilerError::compiler_error(
                "Materialised nominal binding has no struct or choice definition",
            ));
        }
    };
    Ok(Declaration {
        id: local_path,
        value: Expression::new(
            ExpressionKind::NoValue,
            Default::default(),
            type_id,
            diagnostic_type,
            ValueMode::ImmutableReference,
        ),
    })
}

fn materialised_generic_nominal_metadata(
    type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> Result<Option<GenericDeclarationMetadata>, CompilerError> {
    let Some(generic_parameter_list_id) =
        type_environment.generic_parameter_list_id_for_type(type_id)
    else {
        return Ok(None);
    };
    let environment_parameters = type_environment
        .generic_parameters(generic_parameter_list_id)
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "Materialised generic nominal has no registered parameter list",
            )
        })?
        .parameters
        .iter()
        .enumerate()
        .map(|(index, parameter)| GenericParameter {
            id: TypeParameterId(index as u32),
            name: parameter.name,
            location: Default::default(),
            trait_bounds: Vec::new(),
        })
        .collect::<Vec<_>>();
    let kind = match type_environment.get(type_id) {
        Some(TypeDefinition::Struct(_)) => GenericDeclarationKind::Struct,
        Some(TypeDefinition::Choice(_)) => GenericDeclarationKind::Choice,
        Some(TypeDefinition::GenericInstance(_)) => {
            return Err(CompilerError::compiler_error(
                "Materialised generic nominal metadata target is an instance",
            ));
        }
        _ => {
            return Err(CompilerError::compiler_error(
                "Materialised generic nominal metadata target is not a struct or choice",
            ));
        }
    };
    Ok(
        (!environment_parameters.is_empty()).then_some(GenericDeclarationMetadata {
            kind,
            parameters: GenericParameterList {
                parameters: environment_parameters,
            },
            declaration_location: Default::default(),
        }),
    )
}

fn materialised_struct_fields(
    type_id: TypeId,
    type_environment: &mut TypeEnvironment,
    blueprint: &NominalMaterialisationBlueprint,
    nominal_source: &impl MaterialisationNominalSource,
    external_package_registry: &ExternalPackageRegistry,
    template_ir_store: &Rc<RefCell<crate::compiler_frontend::ast::templates::tir::TemplateIrStore>>,
    string_table: &mut StringTable,
) -> Result<Option<Vec<Declaration>>, CompilerError> {
    if !matches!(
        type_environment.get(type_id),
        Some(TypeDefinition::Struct(_))
    ) {
        return Ok(None);
    }
    let Some(fields) = type_environment.fields_for(type_id).map(<[_]>::to_vec) else {
        return Ok(None);
    };
    let NominalMaterialisationDefinition::Struct {
        fields: blueprint_fields,
        ..
    } = &blueprint.definition
    else {
        return Err(CompilerError::compiler_error(
            "Materialised struct has a non-struct nominal blueprint",
        ));
    };
    if fields.len() != blueprint_fields.len() {
        return Err(CompilerError::compiler_error(
            "Materialised struct field count disagrees with its stable blueprint",
        ));
    }
    let mut declarations = Vec::with_capacity(fields.len());
    for (field, blueprint_field) in fields.iter().zip(blueprint_fields) {
        if field.name.name_str(string_table) != Some(blueprint_field.name.as_str()) {
            return Err(CompilerError::compiler_error(
                "Materialised struct field name disagrees with its stable blueprint",
            ));
        }
        let mut value = if let Some(default) = blueprint_field.folded_default.as_ref() {
            let mut materialiser = GeneratedFoldedValueMaterialiser {
                type_environment,
                external_registry: external_package_registry,
                nominal_source,
                template_ir_store: Rc::clone(template_ir_store),
            };
            materialize_public_folded_value(
                &mut materialiser,
                default,
                field.type_id,
                string_table,
            )?
        } else {
            Expression::new(
                ExpressionKind::NoValue,
                field.location.clone(),
                field.type_id,
                diagnostic_type_spelling(field.type_id, type_environment),
                ValueMode::ImmutableReference,
            )
        };
        value.location = field.location.clone();
        value.value_mode = ValueMode::ImmutableReference;
        declarations.push(Declaration {
            id: field.name.clone(),
            value,
        });
    }
    Ok(Some(declarations))
}

impl StableFileVisibility {
    fn capture_namespace_record(
        record: &NamespaceRecord,
        external_package_registry: &ExternalPackageRegistry,
        string_table: &StringTable,
    ) -> Result<StableNamespaceRecord, CompilerError> {
        let record_source = match &record.record_source {
            NamespaceRecordSource::SourceFile(path) => {
                StableNamespaceRecordSource::SourceFile(stable_path(path, string_table))
            }
            NamespaceRecordSource::ExternalPackage(package) => {
                StableNamespaceRecordSource::ExternalPackage(
                    string_table.resolve(*package).to_owned(),
                )
            }
        };
        let mut value_members = record
            .value_members
            .iter()
            .map(|(name, member)| {
                let visible_name = string_table.resolve(*name).to_owned();
                let member = match member {
                    NamespaceValueMember::SourceDeclaration(target) => {
                        StableNamespaceValueMember::Source(StableVisibleDeclaration {
                            visible_name: visible_name.clone(),
                            local_path: stable_path(target.local_path(), string_table),
                            origin: match target {
                                SourceDeclarationTarget::Local(_) => None,
                                SourceDeclarationTarget::Imported { origin, .. } => {
                                    Some(origin.clone())
                                }
                            },
                        })
                    }
                    NamespaceValueMember::ExternalSymbol(symbol_id) => {
                        StableNamespaceValueMember::External {
                            visible_name: visible_name.clone(),
                            identity: external_package_registry
                                .canonical_symbol_identity(*symbol_id)
                                .ok_or_else(|| {
                                    CompilerError::compiler_error(
                                        "Materialisation namespace value has no canonical external identity",
                                    )
                                })?,
                        }
                    }
                };
                Ok((visible_name, member))
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;
        value_members.sort_by(|left, right| left.0.cmp(&right.0));

        let mut type_members = record
            .type_members
            .iter()
            .map(|(name, member)| {
                let visible_name = string_table.resolve(*name).to_owned();
                let member = match member {
                    NamespaceTypeMember::SourceDeclaration(target) => {
                        StableNamespaceTypeMember::Source(StableVisibleDeclaration {
                            visible_name: visible_name.clone(),
                            local_path: stable_path(target.local_path(), string_table),
                            origin: match target {
                                SourceDeclarationTarget::Local(_) => None,
                                SourceDeclarationTarget::Imported { origin, .. } => {
                                    Some(origin.clone())
                                }
                            },
                        })
                    }
                    NamespaceTypeMember::ExternalSymbol(symbol_id) => {
                        StableNamespaceTypeMember::External {
                            visible_name: visible_name.clone(),
                            identity: external_package_registry
                                .canonical_symbol_identity(*symbol_id)
                                .ok_or_else(|| {
                                    CompilerError::compiler_error(
                                        "Materialisation namespace type has no canonical external identity",
                                    )
                                })?,
                        }
                    }
                };
                Ok((visible_name, member))
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;
        type_members.sort_by(|left, right| left.0.cmp(&right.0));

        let mut child_namespaces = record
            .child_namespaces
            .iter()
            .map(|(name, child)| {
                Ok(StableNamespaceBinding {
                    visible_name: string_table.resolve(*name).to_owned(),
                    record: StableFileVisibility::capture_namespace_record(
                        child,
                        external_package_registry,
                        string_table,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;
        child_namespaces.sort_by(|left, right| left.visible_name.cmp(&right.visible_name));

        Ok(StableNamespaceRecord {
            record_source,
            value_members: value_members
                .into_iter()
                .map(|(_, member)| member)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            type_members: type_members
                .into_iter()
                .map(|(_, member)| member)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            child_namespaces: child_namespaces.into_boxed_slice(),
        })
    }

    fn materialise_namespace_record(
        record: &StableNamespaceRecord,
        external_package_registry: &ExternalPackageRegistry,
        string_table: &mut StringTable,
    ) -> Result<NamespaceRecord, CompilerError> {
        let record_source = match &record.record_source {
            StableNamespaceRecordSource::SourceFile(path) => {
                NamespaceRecordSource::SourceFile(materialise_path(path, string_table))
            }
            StableNamespaceRecordSource::ExternalPackage(package) => {
                NamespaceRecordSource::ExternalPackage(string_table.intern(package))
            }
        };
        let mut materialised = NamespaceRecord::empty(record_source);
        for member in &record.value_members {
            match member {
                StableNamespaceValueMember::Source(binding) => {
                    let name = string_table.intern(&binding.visible_name);
                    let local_path = materialise_path(&binding.local_path, string_table);
                    let target = match &binding.origin {
                        Some(origin) => SourceDeclarationTarget::Imported {
                            origin: origin.clone(),
                            local_path,
                        },
                        None => SourceDeclarationTarget::Local(local_path),
                    };
                    materialised
                        .value_members
                        .insert(name, NamespaceValueMember::SourceDeclaration(target));
                }
                StableNamespaceValueMember::External {
                    visible_name,
                    identity,
                } => {
                    let symbol_id = external_package_registry
                        .resolve_canonical_symbol(identity)
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "Materialisation namespace value is absent from the active registry",
                            )
                        })?;
                    materialised.value_members.insert(
                        string_table.intern(visible_name),
                        NamespaceValueMember::ExternalSymbol(symbol_id),
                    );
                }
            }
        }
        for member in &record.type_members {
            match member {
                StableNamespaceTypeMember::Source(binding) => {
                    let name = string_table.intern(&binding.visible_name);
                    let local_path = materialise_path(&binding.local_path, string_table);
                    let target = match &binding.origin {
                        Some(origin) => SourceDeclarationTarget::Imported {
                            origin: origin.clone(),
                            local_path,
                        },
                        None => SourceDeclarationTarget::Local(local_path),
                    };
                    materialised
                        .type_members
                        .insert(name, NamespaceTypeMember::SourceDeclaration(target));
                }
                StableNamespaceTypeMember::External {
                    visible_name,
                    identity,
                } => {
                    let symbol_id = external_package_registry
                        .resolve_canonical_symbol(identity)
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "Materialisation namespace type is absent from the active registry",
                            )
                        })?;
                    materialised.type_members.insert(
                        string_table.intern(visible_name),
                        NamespaceTypeMember::ExternalSymbol(symbol_id),
                    );
                }
            }
        }
        for child in &record.child_namespaces {
            let name = string_table.intern(&child.visible_name);
            let child_record = Self::materialise_namespace_record(
                &child.record,
                external_package_registry,
                string_table,
            )?;
            materialised.child_namespaces.insert(name, child_record);
        }
        Ok(materialised)
    }

    fn materialise(
        &self,
        external_package_registry: &ExternalPackageRegistry,
        string_table: &mut StringTable,
    ) -> Result<FileVisibility, CompilerError> {
        let mut visibility = FileVisibility::default();
        let mut materialise_bindings =
            |bindings: &[StableVisibleDeclaration],
             target: &mut FxHashMap<StringId, SourceDeclarationTarget>| {
                for binding in bindings {
                    let name = string_table.intern(&binding.visible_name);
                    let local_path = materialise_path(&binding.local_path, string_table);
                    visibility
                        .visible_declaration_paths
                        .insert(local_path.clone());
                    let declaration_target = match &binding.origin {
                        Some(origin) => SourceDeclarationTarget::Imported {
                            origin: origin.clone(),
                            local_path,
                        },
                        None => SourceDeclarationTarget::Local(local_path),
                    };
                    target.insert(name, declaration_target);
                }
            };
        materialise_bindings(&self.source_names, &mut visibility.visible_source_names);
        materialise_bindings(
            &self.type_alias_names,
            &mut visibility.visible_type_alias_names,
        );
        materialise_bindings(&self.trait_names, &mut visibility.visible_trait_names);
        let error_path =
            crate::compiler_frontend::builtins::error_type::builtin_error_type_path(string_table);
        let error_name =
            string_table.intern(crate::compiler_frontend::builtins::error_type::ERROR_TYPE_NAME);
        visibility
            .visible_declaration_paths
            .insert(error_path.clone());
        visibility
            .visible_source_names
            .insert(error_name, SourceDeclarationTarget::Local(error_path));
        for binding in &self.external_symbols {
            let symbol_id = external_package_registry
                .resolve_canonical_symbol(&binding.identity)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Materialisation external binding is absent from the active registry",
                    )
                })?;
            visibility
                .visible_external_symbols
                .insert(string_table.intern(&binding.visible_name), symbol_id);
        }
        for binding in &self.namespace_records {
            visibility.visible_namespace_records.insert(
                string_table.intern(&binding.visible_name),
                Self::materialise_namespace_record(
                    &binding.record,
                    external_package_registry,
                    string_table,
                )?,
            );
        }
        for method in &self.receiver_methods {
            let local_path = materialise_path(&method.local_path, string_table);
            visibility
                .visible_receiver_methods
                .entry(string_table.intern(&method.visible_name))
                .or_default()
                .push(crate::compiler_frontend::headers::import_environment::ReceiverMethodVisibility {
                    target: method.target.materialise(local_path),
                    location: method.location.materialise(string_table),
                });
        }
        Ok(visibility)
    }

    fn materialised_selected_paths(
        &self,
        string_table: &mut StringTable,
    ) -> FxHashSet<InternedPath> {
        let mut selected = self
            .source_names
            .iter()
            .chain(self.type_alias_names.iter())
            .chain(self.trait_names.iter())
            .map(|binding| materialise_path(&binding.local_path, string_table))
            .collect::<FxHashSet<_>>();
        selected.extend(
            self.receiver_methods
                .iter()
                .map(|method| materialise_path(&method.local_path, string_table)),
        );
        for namespace in &self.namespace_records {
            Self::collect_namespace_source_paths(&namespace.record, &mut selected, string_table);
        }
        selected
    }

    fn collect_namespace_source_paths(
        record: &StableNamespaceRecord,
        selected: &mut FxHashSet<InternedPath>,
        string_table: &mut StringTable,
    ) {
        for member in &record.value_members {
            if let StableNamespaceValueMember::Source(binding) = member {
                selected.insert(materialise_path(&binding.local_path, string_table));
            }
        }
        for member in &record.type_members {
            if let StableNamespaceTypeMember::Source(binding) = member {
                selected.insert(materialise_path(&binding.local_path, string_table));
            }
        }
        for child in &record.child_namespaces {
            Self::collect_namespace_source_paths(&child.record, selected, string_table);
        }
    }
}

struct StableFunctionMaterialisationContext<'a, N: MaterialisationNominalSource> {
    generic_parameter_type_ids: &'a [TypeId],
    nominal_source: &'a N,
    type_environment: &'a mut TypeEnvironment,
    external_package_registry: &'a ExternalPackageRegistry,
    template_ir_store:
        &'a Rc<RefCell<crate::compiler_frontend::ast::templates::tir::TemplateIrStore>>,
    string_table: &'a mut StringTable,
}

impl StableFunctionSignature {
    fn materialise<N: MaterialisationNominalSource>(
        &self,
        function_path: &InternedPath,
        context: &mut StableFunctionMaterialisationContext<'_, N>,
    ) -> Result<(FunctionSignature, TypeId, Option<TypeId>), CompilerError> {
        let mut parameters = Vec::with_capacity(self.parameters.len());
        let mut parameter_type_ids = Vec::with_capacity(self.parameters.len());
        for parameter in &self.parameters {
            let type_id = intern_materialisation_type_blueprint(
                &parameter.parameter_type,
                context.generic_parameter_type_ids,
                context.nominal_source,
                context.type_environment,
                context.external_package_registry,
                context.string_table,
            )?;
            let name = context.string_table.intern(&parameter.name);
            let parameter_path = function_path.append(name);
            let parameter_location = parameter.location.materialise(context.string_table);
            let mut value = if let Some(default) = parameter.folded_default.as_ref() {
                let mut materialiser = GeneratedFoldedValueMaterialiser {
                    type_environment: &mut *context.type_environment,
                    external_registry: context.external_package_registry,
                    nominal_source: context.nominal_source,
                    template_ir_store: Rc::clone(context.template_ir_store),
                };
                materialize_public_folded_value(
                    &mut materialiser,
                    default,
                    type_id,
                    context.string_table,
                )?
            } else {
                Expression::new(
                    ExpressionKind::NoValue,
                    parameter_location.clone(),
                    type_id,
                    diagnostic_type_spelling(type_id, context.type_environment),
                    parameter.value_mode.clone(),
                )
            };
            value.location = parameter_location;
            value.value_mode = parameter.value_mode.clone();
            if parameter.reactive {
                value.reactive_source = Some(ReactiveSource {
                    path: parameter_path.clone(),
                    kind: ReactiveSourceKind::Parameter,
                });
            }
            parameters.push(Declaration {
                id: parameter_path,
                value,
            });
            parameter_type_ids.push(type_id);
        }
        let mut returns = Vec::with_capacity(self.returns.len());
        let mut success_type_ids = Vec::new();
        let mut error_return = None;
        for returned in &self.returns {
            let type_id = intern_materialisation_type_blueprint(
                &returned.return_type,
                context.generic_parameter_type_ids,
                context.nominal_source,
                context.type_environment,
                context.external_package_registry,
                context.string_table,
            )?;
            let diagnostic_type = diagnostic_type_spelling(type_id, context.type_environment);
            returns.push(ReturnSlot {
                value: diagnostic_type,
                type_id: Some(type_id),
                reactive_template: None,
                channel: returned.channel,
            });
            match returned.channel {
                ReturnChannel::Success => success_type_ids.push(type_id),
                ReturnChannel::Error => error_return = Some(type_id),
            }
        }
        let fallible_carrier_type_id = error_return.map(|error_type_id| {
            let success_type_id = match success_type_ids.as_slice() {
                [] => builtin_type_ids::NONE,
                [single] => *single,
                many => context.type_environment.intern_tuple(many.to_vec()),
            };
            context
                .type_environment
                .intern_fallible_carrier(success_type_id, error_type_id)
        });
        let function_type_id = context.type_environment.intern_function(FunctionTypeKey {
            parameters: parameter_type_ids.into_boxed_slice(),
            returns: success_type_ids.into_boxed_slice(),
            error_return,
        });
        Ok((
            FunctionSignature {
                parameters,
                returns,
            },
            function_type_id,
            fallible_carrier_type_id,
        ))
    }
}

/// Self-contained immutable semantic context owned by one successful declaring module.
#[derive(Clone)]
pub(crate) struct ModuleMaterialisationPreparation {
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
    const_templates_by_path: FxHashMap<InternedPath, PublicConstTemplate>,
    pub(crate) builtin_struct_ast_nodes: Vec<AstNode>,
    pub(crate) resolved_struct_fields_by_path: FxHashMap<InternedPath, Vec<Declaration>>,
    pub(crate) resolved_function_signatures_by_path:
        FxHashMap<InternedPath, ResolvedFunctionSignature>,
    pub(crate) generic_function_templates_by_path: FxHashMap<InternedPath, GenericFunctionTemplate>,
    generic_template_paths_by_identity: FxHashMap<GeneratedDeclarationIdentity, InternedPath>,
    pub(crate) resolved_type_aliases_by_path: FxHashMap<InternedPath, ResolvedTypeAnnotation>,
    pub(crate) choice_variant_shells_by_path: FxHashMap<InternedPath, Vec<ChoiceVariant>>,
    pub(crate) declaration_semantics: DeclarationSemanticTable,
    pub(crate) generic_declarations_by_path: FxHashMap<InternedPath, GenericDeclarationMetadata>,
    pub(crate) nominal_type_ids_by_path: FxHashMap<InternedPath, TypeId>,
    source_nominal_paths: FxHashSet<InternedPath>,
    public_trait_paths: Vec<InternedPath>,
    nominal_blueprints: FxHashMap<CanonicalTypeIdentity, NominalMaterialisationBlueprint>,
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

/// Construction-only owner for the declaring-module context.
///
/// WHAT: accumulates stable executable identities, validated template identities and exact local
/// call summaries while the declaring module is still compiling, then freezes the completed
/// context before publication.
/// WHY: phase-local mutation must not leak into provider metadata. The builder can either hand
/// its broad state to another in-flight generated compilation or freeze the successful module's
/// retained bodies into [`ModuleMaterialisationContext`].
pub(crate) struct ModuleMaterialisationPreparationBuilder {
    context: ModuleMaterialisationPreparation,
}

/// Declaring-module facts captured together before AST finalisation releases its local owners.
pub(crate) struct ModuleMaterialisationEnvironmentInput<'a> {
    pub(crate) lookups: &'a AstModuleLookups,
    pub(crate) type_environment: &'a TypeEnvironment,
    pub(crate) public_trait_roots: &'a [ResolvedPublicTraitRoot],
    pub(crate) const_templates_by_path: FxHashMap<InternedPath, PublicConstTemplate>,
    pub(crate) entry_dir: InternedPath,
    pub(crate) string_table: &'a StringTable,
    pub(crate) template_const_loop_iteration_limit: usize,
    pub(crate) capacity_estimate: FrontendArenaCapacityEstimate,
}

impl ModuleMaterialisationPreparationBuilder {
    pub(crate) fn from_environment(input: ModuleMaterialisationEnvironmentInput<'_>) -> Self {
        Self {
            context: ModuleMaterialisationPreparation::from_environment(input),
        }
    }

    pub(crate) fn context(&self) -> &ModuleMaterialisationPreparation {
        &self.context
    }

    pub(crate) fn finish_preparation(
        mut self,
    ) -> Result<ModuleMaterialisationPreparation, CompilerError> {
        self.context
            .rebuild_generic_template_identity_index()
            .map(|()| self.context)
    }

    pub(crate) fn finalize_generic_template_identity_index(&mut self) -> Result<(), CompilerError> {
        self.context.rebuild_generic_template_identity_index()
    }

    pub(crate) fn freeze(
        self,
        public_interface: &PublicSemanticInterface,
    ) -> Result<Option<ModuleMaterialisationContext>, CompilerError> {
        self.context.freeze(public_interface)
    }

    pub(crate) fn install_concrete_executable_contracts(
        &mut self,
        module_origin: &crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity,
        public_origins_by_path: &FxHashMap<InternedPath, OriginFunctionId>,
        public_nominal_origins_by_path: &FxHashMap<InternedPath, OriginTypeId>,
    ) -> Result<Vec<(InternedPath, ModulePrivateExecutableIdentity)>, CompilerError> {
        self.context.install_concrete_executable_contracts(
            module_origin,
            public_origins_by_path,
            public_nominal_origins_by_path,
        )
    }

    pub(crate) fn generic_function_templates_mut(
        &mut self,
    ) -> &mut FxHashMap<InternedPath, GenericFunctionTemplate> {
        &mut self.context.generic_function_templates_by_path
    }

    pub(crate) fn imported_functions_mut(
        &mut self,
    ) -> &mut FxHashMap<InternedPath, AstImportedFunctionContract> {
        &mut self.context.imported_functions_by_local_path
    }

    fn inherit_nominal_blueprints(
        &mut self,
        source: &ModuleMaterialisationPreparation,
    ) -> Result<(), CompilerError> {
        for (identity, blueprint) in &source.nominal_blueprints {
            if let Some(existing) = self.context.nominal_blueprints.get(identity)
                && existing != blueprint
            {
                return Err(CompilerError::compiler_error(format!(
                    "Generated materialisation contexts disagree on nominal blueprint {identity:?}",
                )));
            }
            self.context
                .nominal_blueprints
                .insert(identity.clone(), blueprint.clone());
        }
        Ok(())
    }

    fn inherit_artefact_nominal_blueprints(
        &mut self,
        source: &GenericTemplateArtefact,
    ) -> Result<(), CompilerError> {
        for (identity, blueprint) in &source.nominal_blueprints {
            if let Some(existing) = self.context.nominal_blueprints.get(identity)
                && existing != blueprint
            {
                return Err(CompilerError::compiler_error(format!(
                    "Generated materialisation artefact disagrees on nominal blueprint {identity:?}",
                )));
            }
            self.context
                .nominal_blueprints
                .insert(identity.clone(), blueprint.clone());
        }
        Ok(())
    }
}

impl ModuleMaterialisationPreparation {
    fn freeze(
        mut self,
        public_interface: &PublicSemanticInterface,
    ) -> Result<Option<ModuleMaterialisationContext>, CompilerError> {
        self.rebuild_generic_template_identity_index()?;
        let mut templates = self
            .generic_function_templates_by_path
            .values()
            .filter(|template| template.body_tokens.is_some())
            .collect::<Vec<_>>();
        templates.sort_by(|left, right| left.declaration_identity.cmp(&right.declaration_identity));
        if templates.is_empty() {
            return Ok(None);
        }

        let mut declaration_closure = self
            .import_environment
            .imported_declarations_by_origin
            .values()
            .cloned()
            .collect::<Vec<_>>();
        declaration_closure.extend(public_interface.declarations.iter().cloned());
        declaration_closure.sort_by(|left, right| left.origin.cmp(&right.origin));
        declaration_closure.dedup_by(|left, right| left.origin == right.origin);

        let mut evidence = self
            .import_environment
            .imported_evidence_by_identity
            .values()
            .cloned()
            .collect::<Vec<_>>();
        evidence.extend(public_interface.reusable_evidence.iter().cloned());
        evidence.sort_by(|left, right| left.identity.cmp(&right.identity));
        evidence.dedup_by(|left, right| left.identity == right.identity);

        let semantic_closure = self.stable_semantic_closure()?;

        let artefacts = templates
            .into_iter()
            .map(|template| self.freeze_template(template, public_interface, &semantic_closure))
            .collect::<Result<Box<[_]>, CompilerError>>()?;
        Ok(Some(ModuleMaterialisationContext {
            declaration_closure: declaration_closure.into_boxed_slice(),
            evidence: evidence.into_boxed_slice(),
            semantic_closure,
            artefacts,
        }))
    }

    fn freeze_template(
        &self,
        template: &GenericFunctionTemplate,
        public_interface: &PublicSemanticInterface,
        semantic_closure: &StableSemanticClosure,
    ) -> Result<GenericTemplateArtefact, CompilerError> {
        let declaration_identity = template.declaration_identity.clone().ok_or_else(|| {
            CompilerError::compiler_error(
                "Retained generic template has no stable declaration identity",
            )
        })?;
        let body_tokens = template.body_tokens.as_ref().ok_or_else(|| {
            CompilerError::compiler_error("Retained generic template has no body syntax")
        })?;
        let generic_parameters = self.stable_generic_parameters(template)?;
        let generic_parameter_owner = template.generic_parameter_owner.clone();
        let receiver = self
            .resolved_function_signatures_by_path
            .get(&template.function_path)
            .and_then(|resolved| resolved.receiver.as_ref())
            .map(|receiver| StableReceiverKey::capture(receiver, &self.string_table))
            .transpose()?;
        let receiver_nominal_identity = self.receiver_nominal_identity(&template.function_path)?;
        let parameter_slots = self.generic_parameter_slots(template)?;
        let signature = self.stable_function_signature(&template.signature, &parameter_slots)?;
        let mut referenced_names = stable_body_symbol_names(body_tokens, &self.string_table);
        self.retain_generic_bound_trait_names(
            &template.source_file,
            &generic_parameters,
            &mut referenced_names,
        )?;
        let selected_paths =
            self.selected_visible_paths(&template.source_file, &referenced_names)?;
        let visibility = self.stable_file_visibility(&template.source_file, &referenced_names)?;
        let declarations = self.stable_declaration_bindings(&selected_paths, public_interface)?;
        let local_declarations = self.stable_local_declaration_bindings(&selected_paths);
        let callables = self.stable_callable_bindings(&selected_paths)?;
        let nominals = self.stable_nominal_bindings(&selected_paths);
        let nominal_blueprints =
            self.stable_nominal_blueprints(&selected_paths, &signature, semantic_closure)?;

        Ok(GenericTemplateArtefact {
            declaration_identity,
            generic_parameter_owner,
            receiver,
            receiver_nominal_identity,
            function_path: stable_path(&template.function_path, &self.string_table),
            source_file: stable_path(&template.source_file, &self.string_table),
            declaration_location: StableSourceLocation::capture(
                &template.declaration_location,
                &self.string_table,
            ),
            body: StableBodySyntax::capture(body_tokens, &self.string_table),
            signature,
            generic_parameters,
            visibility,
            declarations,
            local_declarations,
            callables,
            nominals,
            nominal_blueprints,
        })
    }

    fn stable_generic_parameters(
        &self,
        template: &GenericFunctionTemplate,
    ) -> Result<Box<[StableGenericParameter]>, CompilerError> {
        let parameters = self
            .type_environment
            .generic_parameters(template.generic_parameter_list_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Retained generic template references a missing parameter list",
                )
            })?;
        parameters
            .parameters
            .iter()
            .enumerate()
            .map(|(slot, parameter)| {
                let name = self.string_table.resolve(parameter.name).to_owned();
                let exported_identity = match template.declaration_identity.as_ref() {
                    Some(GeneratedDeclarationIdentity::Public(_)) => Some(
                        ExportedGenericParameterIdentity::new(
                            template
                                .generic_parameter_owner
                                .clone()
                                .ok_or_else(|| {
                                    CompilerError::compiler_error(
                                        "Public retained generic template has no explicit generic-parameter owner",
                                    )
                                })?,
                            slot as u32,
                            name.clone(),
                        ),
                    ),
                    Some(GeneratedDeclarationIdentity::ModulePrivate(_)) => None,
                    None => {
                        return Err(CompilerError::compiler_error(
                            "Retained generic template has no declaration identity",
                        ));
                    }
                };
                let bounds = parameter
                    .trait_bounds
                    .iter()
                    .map(|trait_id| {
                        self.trait_environment
                            .canonical_identity_for_id(*trait_id)
                            .cloned()
                            .ok_or_else(|| {
                                CompilerError::compiler_error(
                                    "Retained generic parameter bound has no stable trait identity",
                                )
                            })
                    })
                    .collect::<Result<Box<[_]>, CompilerError>>()?;
                Ok(StableGenericParameter {
                    name,
                    exported_identity,
                    bounds,
                })
            })
            .collect()
    }

    /// Capture local constants and transparent aliases once for the whole declaring module.
    ///
    /// These declarations have no provider origin, so retaining only their visible paths would
    /// leave a generated sidecar with a visibility entry but no declaration fact to resolve.
    fn stable_semantic_closure(&self) -> Result<StableSemanticClosure, CompilerError> {
        let mut constants = Vec::new();
        for declaration in &self.module_constants {
            if self
                .import_environment
                .imported_declarations_by_local_path
                .contains_key(&declaration.id)
            {
                continue;
            }
            let type_identity = self.stable_type_identity(declaration.value.type_id)?;
            let value = self.stable_folded_value_at_path(&declaration.id, &declaration.value)?;
            constants.push(StableLocalConstant {
                local_path: stable_path(&declaration.id, &self.string_table),
                type_identity,
                value,
            });
        }
        constants.sort_by(|left, right| left.local_path.cmp(&right.local_path));

        let mut aliases = self
            .resolved_type_aliases_by_path
            .iter()
            .filter(|(path, _)| {
                !self
                    .import_environment
                    .imported_declarations_by_local_path
                    .contains_key(*path)
            })
            .map(|(path, annotation)| {
                let type_id = annotation.type_id.ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Retained local type alias has no resolved target type",
                    )
                })?;
                Ok(StableLocalAlias {
                    local_path: stable_path(path, &self.string_table),
                    target_type_identity: self.stable_type_identity(type_id)?,
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

    fn stable_local_declaration_bindings(
        &self,
        selected_paths: &FxHashSet<InternedPath>,
    ) -> Box<[Box<[String]>]> {
        let mut paths = selected_paths
            .iter()
            .filter(|path| {
                self.module_constants.iter().any(|declaration| {
                    declaration.id == **path
                        && !self
                            .import_environment
                            .imported_declarations_by_local_path
                            .contains_key(*path)
                }) || (self.resolved_type_aliases_by_path.contains_key(*path)
                    && !self
                        .import_environment
                        .imported_declarations_by_local_path
                        .contains_key(*path))
            })
            .map(|path| stable_path(path, &self.string_table))
            .collect::<Vec<_>>();
        paths.sort();
        paths.into_boxed_slice()
    }

    fn stable_type_identity(
        &self,
        type_id: TypeId,
    ) -> Result<CanonicalTypeIdentity, CompilerError> {
        let nominal_origins = MaterialisationNominalOriginResolver {
            type_environment: &self.type_environment,
        };
        let generic_parameter_origins = FoldedValueGenericParameterResolver;
        let projection_context = CanonicalTypeProjectionContext::new(
            &nominal_origins,
            &generic_parameter_origins,
            &self.external_package_registry,
        );
        project_type_id_to_canonical_identity(type_id, &self.type_environment, &projection_context)
    }

    fn stable_folded_value(
        &self,
        expression: &Expression,
    ) -> Result<PublicFoldedValue, CompilerError> {
        let nominal_origins = MaterialisationNominalOriginResolver {
            type_environment: &self.type_environment,
        };
        let generic_parameter_origins = FoldedValueGenericParameterResolver;
        let projection_context = CanonicalTypeProjectionContext::new(
            &nominal_origins,
            &generic_parameter_origins,
            &self.external_package_registry,
        );
        convert_expression_to_folded_value(
            expression,
            &self.type_environment,
            &self.string_table,
            &projection_context,
        )
        .map_err(|mut error| {
            error.msg = format!(
                "{} (while freezing value at {}:{}:{})",
                error.msg,
                expression.location.scope.to_string(&self.string_table),
                expression.location.start_pos.line_number,
                expression.location.start_pos.char_column,
            );
            error
        })
    }

    fn stable_folded_value_at_path(
        &self,
        path: &InternedPath,
        expression: &Expression,
    ) -> Result<PublicFoldedValue, CompilerError> {
        if matches!(expression.kind, ExpressionKind::Template(_)) {
            return self
                .const_templates_by_path
                .get(path)
                .cloned()
                .map(PublicFoldedValue::ConstTemplate)
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Retained const template at {} has no stable folded template value",
                        path.to_string(&self.string_table)
                    ))
                });
        }
        self.stable_folded_value(expression)
    }

    /// Retains the declaration-file spellings that make non-core generic bounds visible.
    ///
    /// Bound-provided receiver dispatch checks ordinary file visibility even after the bound has
    /// been resolved to a canonical trait identity. Bound names live in the declaration header,
    /// outside the retained body token slice, so body-name filtering alone would silently hide
    /// them in the fresh generated environment.
    fn retain_generic_bound_trait_names(
        &self,
        source_file: &InternedPath,
        parameters: &[StableGenericParameter],
        referenced_names: &mut FxHashSet<String>,
    ) -> Result<(), CompilerError> {
        let visibility = self.import_environment.visibility_for(source_file)?;

        for parameter in parameters {
            for bound in &parameter.bounds {
                if matches!(bound, CanonicalTraitIdentity::Core(_)) {
                    continue;
                }

                let trait_id = self
                    .trait_environment
                    .id_for_canonical_identity(bound)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Retained generic bound has no declaring-module trait definition",
                        )
                    })?;
                let mut retained_visible_name = false;

                for (visible_name, target) in &visibility.visible_trait_names {
                    if self
                        .trait_environment
                        .has_path(trait_id, target.local_path())
                    {
                        referenced_names
                            .insert(self.string_table.resolve(*visible_name).to_owned());
                        retained_visible_name = true;
                    }
                }

                if !retained_visible_name {
                    return Err(CompilerError::compiler_error(
                        "Retained generic bound is not visible in its declaration file",
                    ));
                }
            }
        }

        Ok(())
    }

    fn generic_parameter_slots(
        &self,
        template: &GenericFunctionTemplate,
    ) -> Result<FxHashMap<GenericParameterId, usize>, CompilerError> {
        let parameters = self
            .type_environment
            .generic_parameters(template.generic_parameter_list_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Retained generic template references a missing parameter list",
                )
            })?;
        Ok(parameters
            .parameters
            .iter()
            .enumerate()
            .map(|(slot, parameter)| (parameter.id, slot))
            .collect())
    }

    fn stable_function_signature(
        &self,
        signature: &FunctionSignature,
        parameter_slots: &FxHashMap<GenericParameterId, usize>,
    ) -> Result<StableFunctionSignature, CompilerError> {
        let parameters = signature
            .parameters
            .iter()
            .map(|parameter| {
                let name = parameter.id.name().ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Materialisation function parameter has no defining name",
                    )
                })?;
                Ok(StableFunctionParameter {
                    name: self.string_table.resolve(name).to_owned(),
                    value_mode: parameter.value.value_mode.clone(),
                    reactive: parameter.value.reactive_source.is_some(),
                    folded_default: (!matches!(parameter.value.kind, ExpressionKind::NoValue))
                        .then(|| self.stable_folded_value_at_path(&parameter.id, &parameter.value))
                        .transpose()?,
                    parameter_type: self
                        .materialisation_type_blueprint(parameter.value.type_id, parameter_slots)?,
                    location: StableSourceLocation::capture(
                        &parameter.value.location,
                        &self.string_table,
                    ),
                })
            })
            .collect::<Result<Box<[_]>, CompilerError>>()?;
        let returns = signature
            .returns
            .iter()
            .map(|returned| {
                let type_id = returned.type_id.ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Materialisation function return has no resolved type",
                    )
                })?;
                Ok(StableFunctionReturn {
                    return_type: self.materialisation_type_blueprint(type_id, parameter_slots)?,
                    channel: returned.channel,
                })
            })
            .collect::<Result<Box<[_]>, CompilerError>>()?;
        Ok(StableFunctionSignature {
            parameters,
            returns,
        })
    }

    fn stable_file_visibility(
        &self,
        source_file: &InternedPath,
        referenced_names: &FxHashSet<String>,
    ) -> Result<StableFileVisibility, CompilerError> {
        let visibility = self.import_environment.visibility_for(source_file)?;
        let capture_declarations = |bindings: &FxHashMap<StringId, SourceDeclarationTarget>| {
            bindings
                .iter()
                .filter_map(|(name, target)| {
                    let visible_name = self.string_table.resolve(*name);
                    referenced_names
                        .contains(visible_name)
                        .then(|| StableVisibleDeclaration {
                            visible_name: visible_name.to_owned(),
                            local_path: stable_path(target.local_path(), &self.string_table),
                            origin: match target {
                                SourceDeclarationTarget::Local(_) => None,
                                SourceDeclarationTarget::Imported { origin, .. } => {
                                    Some(origin.clone())
                                }
                            },
                        })
                })
                .collect::<Vec<_>>()
                .into_boxed_slice()
        };
        let mut external_symbols = Vec::new();
        for (name, symbol_id) in &visibility.visible_external_symbols {
            let visible_name = self.string_table.resolve(*name);
            if !referenced_names.contains(visible_name) {
                continue;
            }
            let identity = self
                .external_package_registry
                .canonical_symbol_identity(*symbol_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Materialisation external binding has no canonical identity",
                    )
                })?;
            external_symbols.push(StableExternalSymbol {
                visible_name: visible_name.to_owned(),
                identity,
            });
        }
        let mut receiver_methods = Vec::new();
        for (name, methods) in &visibility.visible_receiver_methods {
            if !referenced_names.contains(self.string_table.resolve(*name)) {
                continue;
            }
            for method in methods {
                let local_path = method.target.local_path();
                let Some(target) = self.stable_target_for_path(&method.target) else {
                    continue;
                };
                let resolved = self
                    .resolved_function_signatures_by_path
                    .get(local_path)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Visible receiver method has no resolved function signature",
                        )
                    })?;
                let receiver = resolved
                    .receiver
                    .as_ref()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Visible receiver method has no receiver signature",
                        )
                    })
                    .and_then(|receiver| {
                        StableReceiverKey::capture(receiver, &self.string_table)
                    })?;
                let generic_template = self.generic_function_templates_by_path.get(local_path);
                let parameter_slots = generic_template
                    .map(|template| self.generic_parameter_slots(template))
                    .transpose()?
                    .unwrap_or_default();
                let generic_parameters = generic_template
                    .map(|template| self.stable_generic_parameters(template))
                    .transpose()?
                    .unwrap_or_default();
                receiver_methods.push(StableReceiverMethod {
                    visible_name: self.string_table.resolve(*name).to_owned(),
                    local_path: stable_path(local_path, &self.string_table),
                    target,
                    receiver,
                    signature: self
                        .stable_function_signature(&resolved.signature, &parameter_slots)?,
                    summary: self
                        .imported_functions_by_local_path
                        .get(local_path)
                        .map(|contract| contract.summary.clone()),
                    generic_parameters,
                    location: StableSourceLocation::capture(&method.location, &self.string_table),
                });
            }
        }
        let mut namespace_records = visibility
            .visible_namespace_records
            .iter()
            .filter(|(name, _)| referenced_names.contains(self.string_table.resolve(**name)))
            .map(|(name, record)| {
                Ok(StableNamespaceBinding {
                    visible_name: self.string_table.resolve(*name).to_owned(),
                    record: StableFileVisibility::capture_namespace_record(
                        record,
                        &self.external_package_registry,
                        &self.string_table,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;
        namespace_records.sort_by(|left, right| left.visible_name.cmp(&right.visible_name));
        Ok(StableFileVisibility {
            source_names: capture_declarations(&visibility.visible_source_names),
            type_alias_names: capture_declarations(&visibility.visible_type_alias_names),
            trait_names: capture_declarations(&visibility.visible_trait_names),
            external_symbols: external_symbols.into_boxed_slice(),
            namespace_records: namespace_records.into_boxed_slice(),
            receiver_methods: receiver_methods.into_boxed_slice(),
        })
    }

    fn selected_visible_paths(
        &self,
        source_file: &InternedPath,
        referenced_names: &FxHashSet<String>,
    ) -> Result<FxHashSet<InternedPath>, CompilerError> {
        let visibility = self.import_environment.visibility_for(source_file)?;
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

    fn stable_target_for_path(
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

    fn stable_declaration_bindings(
        &self,
        selected_paths: &FxHashSet<InternedPath>,
        public_interface: &PublicSemanticInterface,
    ) -> Result<Box<[StableDeclarationBinding]>, CompilerError> {
        let mut bindings = Vec::new();
        for path in selected_paths {
            if let Some(origin) = self
                .import_environment
                .imported_declarations_by_local_path
                .get(path)
                && self
                    .import_environment
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

    fn stable_callable_bindings(
        &self,
        selected_paths: &FxHashSet<InternedPath>,
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
                signature: self
                    .stable_function_signature(&resolved.signature, &FxHashMap::default())?,
                summary: contract.summary.clone(),
            });
        }
        callables.sort_by(|left, right| left.local_path.cmp(&right.local_path));
        Ok(callables.into_boxed_slice())
    }

    fn stable_nominal_blueprints(
        &self,
        selected_paths: &FxHashSet<InternedPath>,
        signature: &StableFunctionSignature,
        semantic_closure: &StableSemanticClosure,
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
                identities.insert(identity.clone());
            }
            if let Some(constant) = self
                .module_constants
                .iter()
                .find(|declaration| &declaration.id == path)
            {
                if let Ok(identity) = self.stable_type_identity(constant.value.type_id) {
                    identities.insert(identity);
                }
                if let Ok(value) = self.stable_folded_value_at_path(&constant.id, &constant.value) {
                    value.visit_type_identities(&mut |identity| {
                        identities.insert(identity.clone());
                    });
                }
            }
            if let Some(alias) = self.resolved_type_aliases_by_path.get(path)
                && let Some(type_id) = alias.type_id
                && let Ok(identity) = self.stable_type_identity(type_id)
            {
                identities.insert(identity);
            }
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
            evidence.target_type_identity.visit(&mut |identity| {
                if matches!(
                    identity,
                    CanonicalTypeIdentity::SourceNominal(_)
                        | CanonicalTypeIdentity::ModulePrivateNominal(_)
                ) {
                    identities.insert(identity.clone());
                }
            });
        }
        let mut blueprints = FxHashMap::default();
        for identity in identities {
            if let Some(blueprint) = self.nominal_blueprints.get(&identity) {
                blueprints.insert(identity, blueprint.clone());
            }
        }
        Ok(blueprints)
    }

    fn stable_nominal_bindings(
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

    fn receiver_nominal_identity(
        &self,
        function_path: &InternedPath,
    ) -> Result<Option<CanonicalTypeIdentity>, CompilerError> {
        let Some(receiver) = self
            .resolved_function_signatures_by_path
            .get(function_path)
            .and_then(|resolved| resolved.receiver.as_ref())
        else {
            return Ok(None);
        };
        let receiver_path = match receiver {
            ReceiverKey::Struct(path) | ReceiverKey::Choice(path) => path,
            ReceiverKey::External(_) | ReceiverKey::BuiltinScalar(_) => {
                return Err(CompilerError::compiler_error(
                    "Retained receiver method has no source nominal identity",
                ));
            }
        };
        let type_id = self
            .nominal_type_ids_by_path
            .get(receiver_path)
            .copied()
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Retained receiver method has no enclosing nominal type handle",
                )
            })?;
        self.type_environment
            .canonical_identity_for_type_id(type_id)
            .cloned()
            .map(Some)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Retained receiver method has no enclosing nominal identity",
                )
            })
    }

    /// Freeze stable targets for every concrete executable visible to generated bodies.
    ///
    /// Public functions retain their interface identity. Concrete local helpers receive a
    /// distinct artefact-scoped identity and are projected as imported contracts when a generic
    /// body is materialised in an independent sidecar.
    fn install_concrete_executable_contracts(
        &mut self,
        module_origin: &crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity,
        public_origins_by_path: &FxHashMap<InternedPath, OriginFunctionId>,
        public_nominal_origins_by_path: &FxHashMap<InternedPath, OriginTypeId>,
    ) -> Result<Vec<(InternedPath, ModulePrivateExecutableIdentity)>, CompilerError> {
        self.install_private_semantic_identities(module_origin, public_nominal_origins_by_path)?;
        self.install_nominal_blueprints()?;

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

            let resolved = self
                .resolved_function_signatures_by_path
                .get(&path)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Generic template has no resolved function signature",
                    )
                })?;
            let existing_owner = self
                .generic_function_templates_by_path
                .get(&path)
                .and_then(|template| template.generic_parameter_owner.clone());
            let generic_parameter_owner = match resolved.receiver.as_ref() {
                Some(ReceiverKey::Struct(receiver_path) | ReceiverKey::Choice(receiver_path)) => {
                    public_nominal_origins_by_path
                        .get(receiver_path)
                        .cloned()
                        .map(GenericDeclarationOrigin::nominal_type)
                        .transpose()?
                        .or(existing_owner)
                }
                Some(ReceiverKey::External(_) | ReceiverKey::BuiltinScalar(_)) => {
                    if public_origins_by_path.contains_key(&path) {
                        return Err(CompilerError::compiler_error(
                            "Public generic receiver template has a non-source receiver owner",
                        ));
                    }
                    existing_owner
                }
                None => public_origins_by_path
                    .get(&path)
                    .map(|origin| GenericDeclarationOrigin::free_function(origin.clone()))
                    .transpose()?
                    .or(existing_owner),
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
            if matches!(&expected_identity, GeneratedDeclarationIdentity::Public(_))
                && generic_parameter_owner.is_none()
            {
                return Err(CompilerError::compiler_error(
                    "Public generic template has no explicit generic-parameter owner",
                ));
            }
            if let (Some(existing), Some(expected)) = (
                template.generic_parameter_owner.as_ref(),
                generic_parameter_owner.as_ref(),
            ) && existing != expected
            {
                return Err(CompilerError::compiler_error(
                    "Generic template generic-parameter owner disagrees with its callable origin",
                ));
            }
            template.declaration_identity = Some(expected_identity);
            template.generic_parameter_owner = generic_parameter_owner;
            let owner_for_identity = template.generic_parameter_owner.clone();
            let generic_parameter_list_id = template.generic_parameter_list_id;
            let receiver = resolved.receiver.clone();
            let is_public = matches!(
                template.declaration_identity,
                Some(GeneratedDeclarationIdentity::Public(_))
            );
            if is_public && let Some(owner) = owner_for_identity {
                self.register_exported_generic_parameter_identities(
                    generic_parameter_list_id,
                    &owner,
                    receiver.as_ref(),
                )?;
            }
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

    fn register_exported_generic_parameter_identities(
        &mut self,
        template_parameter_list_id: GenericParameterListId,
        owner: &GenericDeclarationOrigin,
        receiver: Option<&ReceiverKey>,
    ) -> Result<(), CompilerError> {
        let generic_parameter_list_id = if owner.nominal_type_origin().is_some() {
            let receiver_path = match receiver {
                Some(ReceiverKey::Struct(path) | ReceiverKey::Choice(path)) => path,
                Some(ReceiverKey::External(_) | ReceiverKey::BuiltinScalar(_)) | None => {
                    return Err(CompilerError::compiler_error(
                        "Nominal generic-parameter owner has no source receiver path",
                    ));
                }
            };
            let receiver_type_id = self
                .nominal_type_ids_by_path
                .get(receiver_path)
                .copied()
                .or_else(|| {
                    self.type_environment
                        .nominal_id_for_path(receiver_path)
                        .and_then(|nominal_id| {
                            self.type_environment.type_id_for_nominal_id(nominal_id)
                        })
                })
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Public generic receiver owner has no local nominal type handle",
                    )
                })?;
            match self.type_environment.get(receiver_type_id) {
                Some(TypeDefinition::Struct(definition)) => definition.generic_parameters,
                Some(TypeDefinition::Choice(definition)) => definition.generic_parameters,
                _ => None,
            }
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Public generic receiver owner has no generic parameter list",
                )
            })?
        } else {
            template_parameter_list_id
        };
        let parameters = self
            .type_environment
            .generic_parameters(generic_parameter_list_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Public generic template references a missing parameter list",
                )
            })?
            .parameters
            .iter()
            .enumerate()
            .map(|(position, parameter)| {
                (
                    position as u32,
                    parameter.id,
                    self.string_table.resolve(parameter.name).to_owned(),
                )
            })
            .collect::<Vec<_>>();
        for (position, parameter_id, authored_name) in parameters {
            let type_id = self
                .type_environment
                .type_id_for_generic_parameter(parameter_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Public generic template parameter has no local type handle",
                    )
                })?;
            self.type_environment.register_canonical_identity(
                CanonicalTypeIdentity::GenericParameter(ExportedGenericParameterIdentity::new(
                    owner.clone(),
                    position,
                    authored_name,
                )),
                type_id,
            )?;
        }
        Ok(())
    }

    fn install_private_semantic_identities(
        &mut self,
        module_origin: &crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity,
        public_nominal_origins_by_path: &FxHashMap<InternedPath, OriginTypeId>,
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
            if let Some(origin) = public_nominal_origins_by_path.get(&path) {
                self.type_environment.register_canonical_identity(
                    CanonicalTypeIdentity::SourceNominal(origin.clone()),
                    type_id,
                )?;
                continue;
            }
            if !self.source_nominal_paths.contains(&path) {
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

    /// Freeze every requester-visible nominal definition into owned, stable semantic data.
    ///
    /// Imported and local aliases can add several lookup paths for the same nominal. The
    /// canonical identity is the sole blueprint key, so each definition is captured once in
    /// deterministic identity order.
    fn install_nominal_blueprints(&mut self) -> Result<(), CompilerError> {
        let mut nominal_type_ids = FxHashMap::default();
        for (identity, type_id) in self.type_environment.canonical_type_identities() {
            if matches!(
                identity,
                CanonicalTypeIdentity::SourceNominal(_)
                    | CanonicalTypeIdentity::ModulePrivateNominal(_)
            ) {
                nominal_type_ids.entry(identity.clone()).or_insert(type_id);
            }
        }

        let mut nominal_type_ids = nominal_type_ids.into_iter().collect::<Vec<_>>();
        nominal_type_ids.sort_by(|left, right| left.0.cmp(&right.0));

        let mut blueprints = FxHashMap::default();
        for (identity, type_id) in nominal_type_ids {
            let blueprint = self.nominal_blueprint(&identity, type_id)?;
            blueprints.insert(identity, blueprint);
        }
        self.nominal_blueprints = blueprints;
        Ok(())
    }

    fn nominal_blueprint(
        &self,
        identity: &CanonicalTypeIdentity,
        type_id: TypeId,
    ) -> Result<NominalMaterialisationBlueprint, CompilerError> {
        let generic_parameter_list_id = match self.type_environment.get(type_id) {
            Some(TypeDefinition::Struct(definition)) => definition.generic_parameters,
            Some(TypeDefinition::Choice(definition)) => definition.generic_parameters,
            _ => {
                return Err(CompilerError::compiler_error(
                    "Materialisation nominal blueprint target is not a struct or choice",
                ));
            }
        };

        let (generic_parameters, parameter_slots) = if let Some(generic_parameter_list_id) =
            generic_parameter_list_id
        {
            let generic_parameter_list = self
                .type_environment
                .generic_parameters(generic_parameter_list_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Materialisation nominal references a missing generic parameter list",
                    )
                })?;
            let parameter_slots = generic_parameter_list
                .parameters
                .iter()
                .enumerate()
                .map(|(index, parameter)| (parameter.id, index))
                .collect::<FxHashMap<_, _>>();
            let public_parameters = self.public_nominal_generic_parameters(identity)?;
            if let Some(public_parameters) = public_parameters
                && public_parameters.len() != generic_parameter_list.parameters.len()
            {
                return Err(CompilerError::compiler_error(
                    "Materialisation nominal public generic surface has inconsistent arity",
                ));
            }

            let generic_parameters = generic_parameter_list
                    .parameters
                    .iter()
                    .enumerate()
                    .map(|(index, parameter)| {
                let name = self.string_table.resolve(parameter.name).to_owned();
                if let Some(public_parameters) = public_parameters {
                    let public_parameter = &public_parameters[index];
                    if public_parameter.identity.authored_name() != name {
                        return Err(CompilerError::compiler_error(
                            "Materialisation nominal public generic parameter name disagrees with its local definition",
                        ));
                    }
                    return Ok(NominalGenericParameterBlueprint {
                        name,
                        exported_identity: Some(public_parameter.identity.clone()),
                        bounds: public_parameter.bounds.clone().into_boxed_slice(),
                    });
                }

                let exported_identity = match identity {
                    CanonicalTypeIdentity::SourceNominal(origin) => Some(
                        ExportedGenericParameterIdentity::new(
                            GenericDeclarationOrigin::nominal_type(origin.clone())?,
                            index as u32,
                            name.clone(),
                        ),
                    ),
                    CanonicalTypeIdentity::ModulePrivateNominal(_) => None,
                    _ => {
                        return Err(CompilerError::compiler_error(
                            "Materialisation generic parameter owner is not nominal",
                        ));
                    }
                };
                let bounds = parameter
                    .trait_bounds
                    .iter()
                    .map(|trait_id| {
                        self.trait_environment
                            .canonical_identity_for_id(*trait_id)
                            .cloned()
                            .ok_or_else(|| {
                                CompilerError::compiler_error(
                                    "Materialisation nominal generic bound has no canonical trait identity",
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into_boxed_slice();
                Ok(NominalGenericParameterBlueprint {
                    name,
                    exported_identity,
                    bounds,
                })
                    })
                    .collect::<Result<Box<[_]>, CompilerError>>()?;
            (generic_parameters, parameter_slots)
        } else {
            (
                Vec::<NominalGenericParameterBlueprint>::new().into_boxed_slice(),
                FxHashMap::default(),
            )
        };

        let definition = match self.type_environment.get(type_id) {
            Some(TypeDefinition::Struct(definition)) => NominalMaterialisationDefinition::Struct {
                fields: self.nominal_field_blueprints(
                    &definition.fields,
                    &parameter_slots,
                    self.type_environment.nominal_path(type_id),
                )?,
                const_record: definition.const_record,
            },
            Some(TypeDefinition::Choice(definition)) => NominalMaterialisationDefinition::Choice {
                variants: self.nominal_choice_blueprints(
                    &definition.variants,
                    &parameter_slots,
                    self.type_environment.nominal_path(type_id),
                )?,
            },
            _ => unreachable!("nominal kind was validated before generic blueprint extraction"),
        };

        Ok(NominalMaterialisationBlueprint {
            generic_parameters,
            definition,
        })
    }

    fn public_nominal_generic_parameters(
        &self,
        identity: &CanonicalTypeIdentity,
    ) -> Result<
        Option<&[crate::compiler_frontend::public_interface::PublicGenericParameterSurface]>,
        CompilerError,
    > {
        let CanonicalTypeIdentity::SourceNominal(origin) = identity else {
            return Ok(None);
        };
        let Some(record) = self
            .import_environment
            .imported_declarations_by_origin
            .get(&OriginDeclarationId::Type(origin.clone()))
        else {
            return Ok(None);
        };
        match &record.semantics {
            PublicDeclarationSemantics::Struct(semantics) => {
                Ok(Some(&semantics.generic_parameters))
            }
            PublicDeclarationSemantics::Choice(semantics) => {
                Ok(Some(&semantics.generic_parameters))
            }
            _ => Err(CompilerError::compiler_error(
                "Materialisation nominal origin resolved to a non-nominal public declaration",
            )),
        }
    }

    fn field_declaration(
        &self,
        nominal_path: &InternedPath,
        field_name: StringId,
    ) -> Option<&Declaration> {
        self.resolved_struct_fields_by_path
            .get(nominal_path)?
            .iter()
            .find(|declaration| declaration.id.name() == Some(field_name))
    }

    fn nominal_field_blueprints(
        &self,
        fields: &[FieldDefinition],
        parameter_slots: &FxHashMap<GenericParameterId, usize>,
        nominal_path: Option<&InternedPath>,
    ) -> Result<Box<[NominalFieldBlueprint]>, CompilerError> {
        fields
            .iter()
            .map(|field| {
                let name = field.name.name().ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Materialisation nominal field path has no defining name",
                    )
                })?;
                Ok(NominalFieldBlueprint {
                    name: self.string_table.resolve(name).to_owned(),
                    field_type: self
                        .materialisation_type_blueprint(field.type_id, parameter_slots)?,
                    folded_default: nominal_path
                        .and_then(|path| self.field_declaration(path, name))
                        .filter(|declaration| {
                            !matches!(declaration.value.kind, ExpressionKind::NoValue)
                        })
                        .map(|declaration| {
                            self.stable_folded_value_at_path(&declaration.id, &declaration.value)
                        })
                        .transpose()?,
                })
            })
            .collect::<Result<Box<[_]>, CompilerError>>()
    }

    fn nominal_choice_blueprints(
        &self,
        variants: &[ChoiceVariantDefinition],
        parameter_slots: &FxHashMap<GenericParameterId, usize>,
        nominal_path: Option<&InternedPath>,
    ) -> Result<Box<[NominalChoiceVariantBlueprint]>, CompilerError> {
        variants
            .iter()
            .map(|variant| {
                let payload_fields = match &variant.payload {
                    ChoiceVariantPayloadDefinition::Unit => Box::new([]),
                    ChoiceVariantPayloadDefinition::Record { fields } => {
                        self.nominal_field_blueprints(fields, parameter_slots, nominal_path)?
                    }
                };
                Ok(NominalChoiceVariantBlueprint {
                    name: self.string_table.resolve(variant.name).to_owned(),
                    tag: variant.tag,
                    payload_fields,
                })
            })
            .collect::<Result<Box<[_]>, CompilerError>>()
    }

    fn materialisation_type_blueprint(
        &self,
        type_id: TypeId,
        parameter_slots: &FxHashMap<GenericParameterId, usize>,
    ) -> Result<MaterialisationTypeBlueprint, CompilerError> {
        let definition = self.type_environment.get(type_id).ok_or_else(|| {
            CompilerError::compiler_error(
                "Materialisation nominal member references an unknown local type",
            )
        })?;
        match definition {
            TypeDefinition::GenericParameter(parameter) => parameter_slots
                .get(&parameter.id)
                .copied()
                .map(MaterialisationTypeBlueprint::GenericParameter)
                .or_else(|| {
                    self.type_environment
                        .canonical_identity_for_type_id(type_id)
                        .cloned()
                        .map(MaterialisationTypeBlueprint::Canonical)
                })
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Materialisation nominal member references a foreign unresolved generic parameter",
                    )
                }),
            TypeDefinition::Builtin(builtin) => Ok(MaterialisationTypeBlueprint::Canonical(
                CanonicalTypeIdentity::Builtin(canonical_builtin_type(builtin.key)),
            )),
            TypeDefinition::Struct(_) | TypeDefinition::Choice(_) => {
                self.type_environment
                    .canonical_identity_for_type_id(type_id)
                    .cloned()
                    .map(MaterialisationTypeBlueprint::Canonical)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "Materialisation nominal member TypeId({}) has no canonical identity: {definition:?}",
                            type_id.0,
                        ))
                    })
            }
            TypeDefinition::External(external) => {
                let (package, symbol_path) = self
                    .external_package_registry
                    .resolve_type_package_and_symbol_path(external.type_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Materialisation nominal external member type is absent from the binding registry",
                        )
                    })?;
                Ok(MaterialisationTypeBlueprint::Canonical(
                    CanonicalTypeIdentity::ExternalOpaque(ExternalOpaqueTypeIdentity::new(
                        package,
                        symbol_path.clone(),
                    )),
                ))
            }
            TypeDefinition::Constructed(constructed) => {
                self.constructed_materialisation_type_blueprint(constructed, parameter_slots)
            }
            TypeDefinition::GenericInstance(instance) => {
                let base_type_id = self
                    .type_environment
                    .type_id_for_nominal_id(instance.base)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Materialisation generic instance has no nominal base type",
                        )
                    })?;
                let base = self
                    .type_environment
                    .canonical_identity_for_type_id(base_type_id)
                    .cloned()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Materialisation generic instance base has no canonical identity",
                        )
                    })?;
                let arguments = instance
                    .arguments
                    .iter()
                    .map(|argument| {
                        self.materialisation_type_blueprint(*argument, parameter_slots)
                    })
                    .collect::<Result<Box<[_]>, _>>()?;
                Ok(MaterialisationTypeBlueprint::GenericInstance { base, arguments })
            }
            TypeDefinition::Function(_) => Err(CompilerError::compiler_error(
                "Materialisation nominal member cannot contain a function type",
            )),
        }
    }

    fn constructed_materialisation_type_blueprint(
        &self,
        constructed: &crate::compiler_frontend::datatypes::definitions::ConstructedTypeDefinition,
        parameter_slots: &FxHashMap<GenericParameterId, usize>,
    ) -> Result<MaterialisationTypeBlueprint, CompilerError> {
        let arguments = constructed.arguments.as_ref();
        let project = |type_id| self.materialisation_type_blueprint(type_id, parameter_slots);
        match constructed.constructor {
            TypeConstructor::Builtin(BuiltinTypeConstructor::Collection { fixed_capacity }) => {
                let [element] = arguments else {
                    return Err(materialisation_type_arity_error(
                        "collection",
                        1,
                        arguments.len(),
                    ));
                };
                Ok(MaterialisationTypeBlueprint::Collection {
                    element: Box::new(project(*element)?),
                    fixed_capacity,
                })
            }
            TypeConstructor::Builtin(BuiltinTypeConstructor::OrderedMap) => {
                let [key, value] = arguments else {
                    return Err(materialisation_type_arity_error(
                        "ordered map",
                        2,
                        arguments.len(),
                    ));
                };
                Ok(MaterialisationTypeBlueprint::OrderedMap {
                    key: Box::new(project(*key)?),
                    value: Box::new(project(*value)?),
                })
            }
            TypeConstructor::Builtin(BuiltinTypeConstructor::Option) => {
                let [inner] = arguments else {
                    return Err(materialisation_type_arity_error(
                        "option",
                        1,
                        arguments.len(),
                    ));
                };
                Ok(MaterialisationTypeBlueprint::Option(Box::new(project(
                    *inner,
                )?)))
            }
            TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier) => {
                let [success, error] = arguments else {
                    return Err(materialisation_type_arity_error(
                        "fallible carrier",
                        2,
                        arguments.len(),
                    ));
                };
                Ok(MaterialisationTypeBlueprint::FallibleCarrier {
                    success: Box::new(project(*success)?),
                    error: Box::new(project(*error)?),
                })
            }
            TypeConstructor::Builtin(BuiltinTypeConstructor::Tuple) => {
                Ok(MaterialisationTypeBlueprint::Tuple(
                    arguments
                        .iter()
                        .map(|argument| project(*argument))
                        .collect::<Result<Box<[_]>, _>>()?,
                ))
            }
        }
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
            .unwrap_or_default()
            .to_string(&self.string_table);

        Ok(ModulePrivateExecutableIdentity::new(
            module_origin.clone(),
            declaring_source,
            category,
            name,
            receiver_path,
        ))
    }

    fn from_environment(input: ModuleMaterialisationEnvironmentInput<'_>) -> Self {
        let ModuleMaterialisationEnvironmentInput {
            lookups,
            type_environment,
            public_trait_roots,
            const_templates_by_path,
            entry_dir,
            string_table,
            template_const_loop_iteration_limit,
            capacity_estimate,
        } = input;

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
            const_templates_by_path,
            builtin_struct_ast_nodes: lookups.builtin_struct_ast_nodes.clone(),
            resolved_struct_fields_by_path: (*lookups.resolved_struct_fields_by_path).clone(),
            resolved_function_signatures_by_path: (*lookups.resolved_function_signatures_by_path)
                .clone(),
            generic_function_templates_by_path: lookups.generic_function_templates_by_path.clone(),
            generic_template_paths_by_identity: FxHashMap::default(),
            resolved_type_aliases_by_path: (*lookups.resolved_type_aliases_by_path).clone(),
            choice_variant_shells_by_path: (*lookups.choice_variant_shells_by_path).clone(),
            declaration_semantics: (*lookups.declaration_semantics).clone(),
            generic_declarations_by_path: (*lookups.generic_declarations_by_path).clone(),
            nominal_type_ids_by_path: (*lookups.nominal_type_ids_by_path).clone(),
            source_nominal_paths: (*lookups.source_nominal_paths).clone(),
            public_trait_paths: public_trait_roots
                .iter()
                .map(|root| root.canonical_path.clone())
                .collect(),
            nominal_blueprints: FxHashMap::default(),
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

    pub(crate) fn build_environment(
        &self,
        phase_context: &AstPhaseContext<'_>,
        string_table: &mut StringTable,
    ) -> Result<AstModuleEnvironment, CompilerError> {
        let mut module_constants = self.module_constants.clone();
        for declaration in &mut module_constants {
            let ExpressionKind::Template(_) = &declaration.value.kind else {
                continue;
            };
            let projected = self
                .const_templates_by_path
                .get(&declaration.id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Generated materialisation constant has no stable const-template projection",
                    )
                })?;
            let template = materialize_public_const_template(
                projected,
                &phase_context.template_ir_store,
                string_table,
                declaration.value.location.clone(),
            )?;
            declaration.value.kind = ExpressionKind::Template(Box::new(template));
        }

        let module_constants_by_path = module_constants
            .iter()
            .map(|declaration| (&declaration.id, declaration))
            .collect::<FxHashMap<_, _>>();
        let declarations = self
            .declaration_table
            .iter()
            .map(|declaration| {
                module_constants_by_path
                    .get(&declaration.id)
                    .map(|constant| (*constant).clone())
                    .unwrap_or_else(|| declaration.clone())
            })
            .collect();

        let lookups = AstModuleLookups {
            module_symbols: ModuleSymbols::empty(),
            import_environment: self.import_environment.clone(),
            warnings: Vec::new(),
            declaration_table: Rc::new(TopLevelDeclarationTable::new(declarations)),
            imported_functions_by_local_path: self.imported_functions_by_local_path.clone(),
            imported_struct_definitions: self.imported_struct_definitions.clone(),
            imported_choice_definitions: self.imported_choice_definitions.clone(),
            module_constants,
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
            source_nominal_paths: Rc::new(self.source_nominal_paths.clone()),
            external_package_registry: Arc::clone(&self.external_package_registry),
            style_directives: self.style_directives.clone(),
            build_profile: self.build_profile,
            project_path_resolver: self.project_path_resolver.clone(),
            path_format_config: self.path_format_config.clone(),
        };

        Ok(AstModuleEnvironment {
            lookups: Rc::new(lookups),
            generated_evidence_pairs: Rc::new(FxHashSet::default()),
            type_environment: self.type_environment.clone(),
            resolved_public_type_roots: Default::default(),
            resolved_public_trait_roots: Vec::new(),
        })
    }

    pub(crate) fn generic_function_templates(
        &self,
    ) -> &FxHashMap<InternedPath, GenericFunctionTemplate> {
        &self.generic_function_templates_by_path
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
        let path = self.generic_template_paths_by_identity.get(identity)?;
        let template = self.generic_function_templates_by_path.get(path)?;
        (template.declaration_identity.as_ref() == Some(identity) && template.body_tokens.is_some())
            .then_some(template)
    }

    fn rebuild_generic_template_identity_index(&mut self) -> Result<(), CompilerError> {
        self.generic_template_paths_by_identity =
            Self::generic_template_identity_index(&self.generic_function_templates_by_path)?;
        Ok(())
    }

    fn generic_template_identity_index(
        templates: &FxHashMap<InternedPath, GenericFunctionTemplate>,
    ) -> Result<FxHashMap<GeneratedDeclarationIdentity, InternedPath>, CompilerError> {
        let mut paths_by_identity = FxHashMap::default();
        for (path, template) in templates {
            if template.body_tokens.is_none() {
                continue;
            }
            let Some(identity) = template.declaration_identity.as_ref() else {
                continue;
            };
            if let Some(previous_path) = paths_by_identity.insert(identity.clone(), path.clone()) {
                return Err(CompilerError::compiler_error(format!(
                    "Generic template identity {identity:?} is retained at both {previous_path:?} and {path:?}",
                )));
            }
        }
        Ok(paths_by_identity)
    }

    pub(crate) fn materialise_ast(
        &self,
        identity: &GeneratedFunctionIdentity,
        requester_context: &ModuleMaterialisationPreparation,
        requester_call_location: &crate::compiler_frontend::tokenizer::tokens::SourceLocation,
        project_path_resolver: Option<ProjectPathResolver>,
        #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
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
        let build_context = AstBuildContext {
            external_package_registry: Arc::clone(&self.external_package_registry),
            style_directives: &self.style_directives,
            string_table: &mut string_table,
            entry_dir: self.entry_dir.clone(),
            root_role: ModuleRootRole::Support,
            build_profile: self.build_profile,
            project_path_resolver: self.project_path_resolver.clone().or(project_path_resolver),
            path_format_config: self.path_format_config.clone(),
            template_const_loop_iteration_limit: self.template_const_loop_iteration_limit,
            capacity_estimate: self.capacity_estimate,
            #[cfg(feature = "timers")]
            timing_context,
            #[cfg(feature = "timers")]
            timing_metric_family: crate::compiler_frontend::ast::AstTimingMetricFamily::Generated,
        };
        let (phase_context, string_table_ref) = AstPhaseContext::from_build_context(build_context);
        crate::timing_scope_attributed!(
            timing_guard_generated_ast_total,
            crate::timing::TimingMetric::FrontendGeneratedAstTotal,
            timing_context
        );
        let mut environment = self
            .build_environment(&phase_context, string_table_ref)
            .map_err(|error| CompilerMessages::from_error_ref(error, &self.string_table))?;
        let mut type_arguments = Vec::with_capacity(identity.type_arguments().len());
        for canonical_identity in identity.type_arguments() {
            let type_id = intern_generated_canonical_type(
                canonical_identity,
                &mut environment.type_environment,
                self.external_package_registry.as_ref(),
                requester_context,
                string_table_ref,
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, &self.string_table))?;
            type_arguments.push(type_id);
        }
        install_generated_request_evidence(
            identity,
            requester_context,
            &requester_string_remap,
            &mut environment,
            string_table_ref,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, &self.string_table))?;

        let instance_path = template
            .function_path
            .join_str("__generated_instance", string_table_ref);
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
        let emitted = {
            crate::timing_scope_attributed!(
                timing_guard_generated_ast_emit,
                crate::timing::TimingMetric::FrontendGeneratedAstEmit,
                timing_context
            );
            AstEmitter::new(&phase_context, &mut environment, 1)
                .emit_generated_request(request, string_table_ref)?
        };

        let mut build_result = {
            crate::timing_scope_attributed!(
                timing_guard_generated_ast_finalise,
                crate::timing::TimingMetric::FrontendGeneratedAstFinalise,
                timing_context
            );
            AstFinalizer::new(&phase_context, environment).finalize(
                emitted,
                &[],
                string_table_ref,
            )?
        };
        build_result
            .materialisation_context
            .inherit_nominal_blueprints(self)
            .and_then(|()| {
                build_result
                    .materialisation_context
                    .inherit_nominal_blueprints(requester_context)
            })
            .map_err(|error| CompilerMessages::from_error_ref(error, &self.string_table))?;
        Ok(MaterialisedGenericAst {
            build_result,
            string_table,
            instance_path,
        })
    }
}

fn canonical_builtin_type(key: BuiltinTypeKey) -> CanonicalBuiltinType {
    match key {
        BuiltinTypeKey::Bool => CanonicalBuiltinType::Bool,
        BuiltinTypeKey::Int => CanonicalBuiltinType::Int,
        BuiltinTypeKey::Float => CanonicalBuiltinType::Float,
        BuiltinTypeKey::Decimal => CanonicalBuiltinType::Decimal,
        BuiltinTypeKey::String => CanonicalBuiltinType::String,
        BuiltinTypeKey::Char => CanonicalBuiltinType::Char,
        BuiltinTypeKey::Range => CanonicalBuiltinType::Range,
        BuiltinTypeKey::None => CanonicalBuiltinType::None,
    }
}

fn materialisation_type_arity_error(shape: &str, expected: usize, actual: usize) -> CompilerError {
    CompilerError::compiler_error(format!(
        "Materialisation {shape} type has malformed arity: expected {expected}, found {actual}",
    ))
}

fn intern_generated_canonical_type(
    identity: &CanonicalTypeIdentity,
    type_environment: &mut TypeEnvironment,
    external_registry: &ExternalPackageRegistry,
    nominal_source: &impl MaterialisationNominalSource,
    string_table: &mut StringTable,
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
                nominal_source,
                string_table,
            )?;
            type_environment.intern_option(inner)
        }
        CanonicalTypeIdentity::Collection(collection) => {
            let element = intern_generated_canonical_type(
                collection.element(),
                type_environment,
                external_registry,
                nominal_source,
                string_table,
            )?;
            type_environment.intern_collection(element, collection.fixed_capacity())
        }
        CanonicalTypeIdentity::OrderedMap(map) => {
            let key = intern_generated_canonical_type(
                map.key(),
                type_environment,
                external_registry,
                nominal_source,
                string_table,
            )?;
            let value = intern_generated_canonical_type(
                map.value(),
                type_environment,
                external_registry,
                nominal_source,
                string_table,
            )?;
            type_environment.intern_map(key, value)
        }
        CanonicalTypeIdentity::FallibleCarrier(carrier) => {
            let success = intern_generated_canonical_type(
                carrier.success(),
                type_environment,
                external_registry,
                nominal_source,
                string_table,
            )?;
            let error = intern_generated_canonical_type(
                carrier.error(),
                type_environment,
                external_registry,
                nominal_source,
                string_table,
            )?;
            type_environment.intern_fallible_carrier(success, error)
        }
        CanonicalTypeIdentity::SourceNominal(_)
        | CanonicalTypeIdentity::ModulePrivateNominal(_) => intern_materialisation_nominal(
            identity,
            nominal_source,
            type_environment,
            external_registry,
            string_table,
        )?,
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
                nominal_source,
                string_table,
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
                    nominal_source,
                    string_table,
                )?);
            }
            type_environment.intern_generic_instance(nominal_id, arguments.into_boxed_slice())
        }
        CanonicalTypeIdentity::ModulePrivateGenericInstance(instance) => {
            let base_identity =
                CanonicalTypeIdentity::ModulePrivateNominal(instance.base().clone());
            let base_type_id = intern_generated_canonical_type(
                &base_identity,
                type_environment,
                external_registry,
                nominal_source,
                string_table,
            )?;
            let nominal_id = match type_environment.get(base_type_id) {
                Some(TypeDefinition::Struct(definition)) => definition.id,
                Some(TypeDefinition::Choice(definition)) => definition.id,
                _ => {
                    return Err(CompilerError::compiler_error(
                        "Generated private generic base is not nominal",
                    ));
                }
            };
            let mut arguments = Vec::with_capacity(instance.arguments().len());
            for argument in instance.arguments() {
                arguments.push(intern_generated_canonical_type(
                    argument,
                    type_environment,
                    external_registry,
                    nominal_source,
                    string_table,
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

fn intern_materialisation_type_blueprint(
    blueprint: &MaterialisationTypeBlueprint,
    generic_parameter_type_ids: &[TypeId],
    nominal_source: &impl MaterialisationNominalSource,
    type_environment: &mut TypeEnvironment,
    external_registry: &ExternalPackageRegistry,
    string_table: &mut StringTable,
) -> Result<TypeId, CompilerError> {
    match blueprint {
        MaterialisationTypeBlueprint::Canonical(identity) => intern_generated_canonical_type(
            identity,
            type_environment,
            external_registry,
            nominal_source,
            string_table,
        ),
        MaterialisationTypeBlueprint::GenericParameter(slot) => generic_parameter_type_ids
            .get(*slot)
            .copied()
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Materialisation nominal member references an invalid generic parameter slot",
                )
            }),
        MaterialisationTypeBlueprint::Collection {
            element,
            fixed_capacity,
        } => {
            let element = intern_materialisation_type_blueprint(
                element,
                generic_parameter_type_ids,
                nominal_source,
                type_environment,
                external_registry,
                string_table,
            )?;
            Ok(type_environment.intern_collection(element, *fixed_capacity))
        }
        MaterialisationTypeBlueprint::OrderedMap { key, value } => {
            let key = intern_materialisation_type_blueprint(
                key,
                generic_parameter_type_ids,
                nominal_source,
                type_environment,
                external_registry,
                string_table,
            )?;
            let value = intern_materialisation_type_blueprint(
                value,
                generic_parameter_type_ids,
                nominal_source,
                type_environment,
                external_registry,
                string_table,
            )?;
            Ok(type_environment.intern_map(key, value))
        }
        MaterialisationTypeBlueprint::Option(inner) => {
            let inner = intern_materialisation_type_blueprint(
                inner,
                generic_parameter_type_ids,
                nominal_source,
                type_environment,
                external_registry,
                string_table,
            )?;
            Ok(type_environment.intern_option(inner))
        }
        MaterialisationTypeBlueprint::FallibleCarrier { success, error } => {
            let success = intern_materialisation_type_blueprint(
                success,
                generic_parameter_type_ids,
                nominal_source,
                type_environment,
                external_registry,
                string_table,
            )?;
            let error = intern_materialisation_type_blueprint(
                error,
                generic_parameter_type_ids,
                nominal_source,
                type_environment,
                external_registry,
                string_table,
            )?;
            Ok(type_environment.intern_fallible_carrier(success, error))
        }
        MaterialisationTypeBlueprint::Tuple(elements) => {
            let elements = elements
                .iter()
                .map(|element| {
                    intern_materialisation_type_blueprint(
                        element,
                        generic_parameter_type_ids,
                        nominal_source,
                        type_environment,
                        external_registry,
                        string_table,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(type_environment.intern_tuple(elements))
        }
        MaterialisationTypeBlueprint::GenericInstance { base, arguments } => {
            let base_type_id = intern_generated_canonical_type(
                base,
                type_environment,
                external_registry,
                nominal_source,
                string_table,
            )?;
            let nominal_id = match type_environment.get(base_type_id) {
                Some(TypeDefinition::Struct(definition)) => definition.id,
                Some(TypeDefinition::Choice(definition)) => definition.id,
                _ => {
                    return Err(CompilerError::compiler_error(
                        "Materialisation generic instance base is not nominal",
                    ));
                }
            };
            let expected_arity = type_environment
                .generic_parameter_list_id_for_type(base_type_id)
                .and_then(|list_id| type_environment.generic_parameters(list_id))
                .map(|parameters| parameters.parameters.len())
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Materialisation generic instance base has no generic parameter list",
                    )
                })?;
            if arguments.len() != expected_arity {
                return Err(materialisation_type_arity_error(
                    "generic instance",
                    expected_arity,
                    arguments.len(),
                ));
            }
            let arguments = arguments
                .iter()
                .map(|argument| {
                    intern_materialisation_type_blueprint(
                        argument,
                        generic_parameter_type_ids,
                        nominal_source,
                        type_environment,
                        external_registry,
                        string_table,
                    )
                })
                .collect::<Result<Box<[_]>, _>>()?;
            Ok(type_environment.intern_generic_instance(nominal_id, arguments))
        }
    }
}

fn materialisation_nominal_path(
    identity: &CanonicalTypeIdentity,
    string_table: &mut StringTable,
) -> Result<InternedPath, CompilerError> {
    let mut path = InternedPath::from_single_str("<materialised>", string_table);
    match identity {
        CanonicalTypeIdentity::SourceNominal(origin) => {
            path.push_str("source", string_table);
            append_materialisation_module_origin(&mut path, origin.module_origin(), string_table);
            path.push_str(origin.defining_name(), string_table);
            path.push_str(origin_type_category_name(origin.category()), string_table);
        }
        CanonicalTypeIdentity::ModulePrivateNominal(identity) => {
            path.push_str("private", string_table);
            append_materialisation_module_origin(&mut path, identity.module_origin(), string_table);
            path.push_str(identity.defining_path(), string_table);
            path.push_str(origin_type_category_name(identity.category()), string_table);
        }
        _ => {
            return Err(CompilerError::compiler_error(
                "Materialisation nominal path requested for a non-nominal identity",
            ));
        }
    }
    Ok(path)
}

fn append_materialisation_module_origin(
    path: &mut InternedPath,
    origin: &crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity,
    string_table: &mut StringTable,
) {
    let package_origin = match origin.package().origin() {
        crate::builder_surface::PackageOrigin::Core => "core",
        crate::builder_surface::PackageOrigin::Standard => "standard",
        crate::builder_surface::PackageOrigin::Builder => "builder",
        crate::builder_surface::PackageOrigin::ProjectLocal => "project",
        crate::builder_surface::PackageOrigin::Dependency => "dependency",
    };
    let root_role = match origin.role() {
        ModuleRootRole::Normal => "normal",
        ModuleRootRole::Support => "support",
        ModuleRootRole::ProjectPackageFacade => "facade",
    };
    path.push_str(package_origin, string_table);
    path.push_str(origin.package().name(), string_table);
    path.push_str(root_role, string_table);
    for component in origin.logical_module_path().split('/') {
        if !component.is_empty() {
            path.push_str(component, string_table);
        }
    }
}

fn origin_type_category_name(category: OriginTypeCategory) -> &'static str {
    match category {
        OriginTypeCategory::Struct => "struct",
        OriginTypeCategory::Choice => "choice",
        OriginTypeCategory::TransparentAlias => "alias",
    }
}

fn intern_materialisation_nominal(
    identity: &CanonicalTypeIdentity,
    nominal_source: &impl MaterialisationNominalSource,
    type_environment: &mut TypeEnvironment,
    external_registry: &ExternalPackageRegistry,
    string_table: &mut StringTable,
) -> Result<TypeId, CompilerError> {
    if let Some(type_id) = type_environment.type_id_for_canonical_identity(identity) {
        return Ok(type_id);
    }
    let blueprint = nominal_source
        .nominal_blueprint(identity)
        .cloned()
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Generated request nominal {identity:?} is absent from its requester artefact"
            ))
        })?;

    let parsed_parameters = GenericParameterList {
        parameters: blueprint
            .generic_parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| GenericParameter {
                id: TypeParameterId(index as u32),
                name: string_table.intern(&parameter.name),
                location: Default::default(),
                trait_bounds: Vec::new(),
            })
            .collect(),
    };
    // Bounds remain exact stable facts on the immutable blueprint. Reconstructing a concrete
    // nominal for field/variant substitution does not re-run declaration-site evidence solving.
    let generic_parameter_registration = (!parsed_parameters.parameters.is_empty()).then(|| {
        type_environment.register_generic_parameter_list(&parsed_parameters, &FxHashMap::default())
    });
    let generic_parameter_list_id = generic_parameter_registration
        .as_ref()
        .map(|registration| registration.list_id);
    let generated_path = materialisation_nominal_path(identity, string_table)?;
    let type_id = match &blueprint.definition {
        NominalMaterialisationDefinition::Struct { const_record, .. } => {
            type_environment
                .register_nominal_struct(StructTypeDefinition {
                    id: NominalTypeId(0),
                    path: generated_path,
                    fields: Box::new([]),
                    generic_parameters: generic_parameter_list_id,
                    const_record: *const_record,
                })
                .1
        }
        NominalMaterialisationDefinition::Choice { .. } => {
            type_environment
                .register_nominal_choice(ChoiceTypeDefinition {
                    id: NominalTypeId(0),
                    path: generated_path,
                    variants: Box::new([]),
                    generic_parameters: generic_parameter_list_id,
                })
                .1
        }
    };
    type_environment.register_canonical_identity(identity.clone(), type_id)?;

    let parameter_type_ids = if let Some(registration) = generic_parameter_registration {
        blueprint
            .generic_parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                let parameter_id = registration
                    .canonical_by_local
                    .get(&TypeParameterId(index as u32))
                    .copied()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Generated nominal parameter registration omitted a parameter slot",
                        )
                    })?;
                let parameter_type_id = type_environment
                    .type_id_for_generic_parameter(parameter_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Generated nominal parameter registration omitted its type handle",
                        )
                    })?;
                if let Some(exported_identity) = &parameter.exported_identity {
                    type_environment.register_canonical_identity(
                        CanonicalTypeIdentity::GenericParameter(exported_identity.clone()),
                        parameter_type_id,
                    )?;
                }
                Ok(parameter_type_id)
            })
            .collect::<Result<Vec<_>, CompilerError>>()?
    } else {
        Vec::new()
    };

    match &blueprint.definition {
        NominalMaterialisationDefinition::Struct { fields, .. } => {
            let nominal_path =
                type_environment
                    .nominal_path(type_id)
                    .cloned()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Generated struct nominal shell has no local path",
                        )
                    })?;
            let fields = fields
                .iter()
                .map(|field| {
                    Ok(FieldDefinition {
                        name: nominal_path.join_str(&field.name, string_table),
                        type_id: intern_materialisation_type_blueprint(
                            &field.field_type,
                            &parameter_type_ids,
                            nominal_source,
                            type_environment,
                            external_registry,
                            string_table,
                        )?,
                        location: Default::default(),
                    })
                })
                .collect::<Result<Box<[_]>, CompilerError>>()?;
            type_environment.update_struct_fields(type_id, fields);
        }
        NominalMaterialisationDefinition::Choice { variants } => {
            let nominal_path =
                type_environment
                    .nominal_path(type_id)
                    .cloned()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Generated choice nominal shell has no local path",
                        )
                    })?;
            let variants = variants
                .iter()
                .map(|variant| {
                    let payload = if variant.payload_fields.is_empty() {
                        ChoiceVariantPayloadDefinition::Unit
                    } else {
                        let fields = variant
                            .payload_fields
                            .iter()
                            .map(|field| {
                                Ok(FieldDefinition {
                                    name: nominal_path.join_str(&field.name, string_table),
                                    type_id: intern_materialisation_type_blueprint(
                                        &field.field_type,
                                        &parameter_type_ids,
                                        nominal_source,
                                        type_environment,
                                        external_registry,
                                        string_table,
                                    )?,
                                    location: Default::default(),
                                })
                            })
                            .collect::<Result<Box<[_]>, CompilerError>>()?;
                        ChoiceVariantPayloadDefinition::Record { fields }
                    };
                    Ok(ChoiceVariantDefinition {
                        name: string_table.intern(&variant.name),
                        tag: variant.tag,
                        payload,
                        location: Default::default(),
                    })
                })
                .collect::<Result<Box<[_]>, CompilerError>>()?;
            type_environment.update_choice_variants(type_id, variants);
        }
    }

    Ok(type_id)
}

fn install_generated_request_evidence(
    identity: &GeneratedFunctionIdentity,
    requester_context: &ModuleMaterialisationPreparation,
    requester_string_remap: &StringIdRemap,
    environment: &mut AstModuleEnvironment,
    string_table: &mut StringTable,
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
        let generated_evidence_pairs = Rc::make_mut(&mut environment.generated_evidence_pairs);
        generated_evidence_pairs.insert((generated_target_type_id, generated_trait_id));
        if let Some(TypeDefinition::GenericInstance(instance)) =
            environment.type_environment.get(generated_target_type_id)
            && let Some(base_type_id) = environment
                .type_environment
                .type_id_for_nominal_id(instance.base)
        {
            generated_evidence_pairs.insert((base_type_id, generated_trait_id));
        }
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

            let (source_target, source_summary) = if let Some(source_contract) = requester_context
                .imported_functions_by_local_path
                .get(&requester_mapping.method_path)
            {
                (
                    source_contract.target.clone(),
                    source_contract.summary.clone(),
                )
            } else if let Some(template) = requester_context
                .generic_function_templates_by_path
                .get(&requester_mapping.method_path)
            {
                let target = match template.declaration_identity.as_ref() {
                    Some(GeneratedDeclarationIdentity::Public(origin)) => {
                        SourceFunctionTarget::Imported {
                            origin: origin.clone(),
                            local_path: requester_mapping.method_path.clone(),
                        }
                    }
                    Some(GeneratedDeclarationIdentity::ModulePrivate(identity)) => {
                        SourceFunctionTarget::ModulePrivate {
                            identity: identity.clone(),
                            local_path: requester_mapping.method_path.clone(),
                        }
                    }
                    None => {
                        return Err(CompilerError::compiler_error(
                            "Generated evidence generic method has no frozen executable identity",
                        ));
                    }
                };
                (
                    target,
                    bootstrap_call_summary_from_signature(&template.signature),
                )
            } else {
                return Err(CompilerError::compiler_error(
                    "Generated evidence method has no frozen executable target",
                ));
            };
            let target = match source_target {
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
                    summary: source_summary,
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
    PublicCallSummary {
        parameters,
        return_alias: FunctionReturnAliasSummary::Fresh,
    }
}

fn requester_type_id_for_canonical_identity(
    identity: &CanonicalTypeIdentity,
    requester_context: &ModuleMaterialisationPreparation,
) -> Result<TypeId, CompilerError> {
    if let Some(type_id) = requester_context
        .type_environment
        .type_id_for_canonical_identity(identity)
    {
        return Ok(type_id);
    }
    match identity {
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Bool) => Ok(builtin_type_ids::BOOL),
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int) => Ok(builtin_type_ids::INT),
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Float) => Ok(builtin_type_ids::FLOAT),
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Decimal) => {
            Ok(builtin_type_ids::DECIMAL)
        }
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String) => {
            Ok(builtin_type_ids::STRING)
        }
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Char) => Ok(builtin_type_ids::CHAR),
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Range) => Ok(builtin_type_ids::RANGE),
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::None) => Ok(builtin_type_ids::NONE),
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error)
        | CanonicalTypeIdentity::ModulePrivateNominal(_)
        | CanonicalTypeIdentity::ExternalOpaque(_)
        | CanonicalTypeIdentity::Collection(_)
        | CanonicalTypeIdentity::OrderedMap(_)
        | CanonicalTypeIdentity::Option(_)
        | CanonicalTypeIdentity::FallibleCarrier(_)
        | CanonicalTypeIdentity::GenericInstance(_)
        | CanonicalTypeIdentity::ModulePrivateGenericInstance(_)
        | CanonicalTypeIdentity::GenericParameter(_) => Err(CompilerError::compiler_error(
            "Generated evidence target has no requester-local canonical type handle",
        )),
        CanonicalTypeIdentity::SourceNominal(_) => Err(CompilerError::compiler_error(
            "Generated source evidence target has no requester-local canonical type handle",
        )),
    }
}

#[cfg(test)]
#[path = "tests/frozen_body_tests.rs"]
mod frozen_body_tests;
