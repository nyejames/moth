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
use super::visibility::StableFileVisibility;
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstValueField, ConstValueId, ConstValueMetadata,
    ConstValuePayload, ConstValueStore, ConstValueStoreError,
};
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::expressions::expression_types::{
    ConstRecordState, ConstValueKind,
};
use crate::compiler_frontend::ast::module_ast::environment::builder::import_projection::values::FoldedValueMaterialiser;
use crate::compiler_frontend::ast::module_ast::scope_context::{
    FileValueResolutionServices, Stage0ResolutionFacts,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalTraitIdentity, CanonicalTypeIdentity, ExportedGenericParameterIdentity,
    GenericDeclarationOrigin, NominalOriginResolver,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, InfrastructureErrors};
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceVariantPayloadDefinition, FieldDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, OwnedFoldedStringPiece, PublicConstTemplate, PublicConstTemplateKind,
    PublicFoldedField, PublicFoldedValue,
};
use crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, GeneratedFunctionIdentity, ModulePrivateExecutableIdentity,
    OriginDeclarationId, OriginFunctionId, OriginTypeId, StableModuleOriginIdentity,
};
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceSpan};
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
    pub(super) returns: Box<[MaterialisationTypeBlueprint]>,
    pub(super) error_return: Option<MaterialisationTypeBlueprint>,
}

#[derive(Clone)]
pub(super) struct StableFunctionParameter {
    pub(super) name: String,
    pub(super) value_mode: ValueMode,
    pub(super) folded_default: Option<PublicFoldedValue>,
    pub(super) parameter_type: MaterialisationTypeBlueprint,
    pub(super) span: Option<SourceSpan>,
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

pub(super) struct MaterialisationNominalOriginResolver<'a> {
    pub(super) type_environment: &'a TypeEnvironment,
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

/// Project one detached folded value into owned constant content in the active generated store.
///
/// WHAT: converts every stable variant into the same ordinary `ConstValueStore` payload the
///       declaring phase owns, then stages children before their aggregate.
/// WHY: generated bodies must not reconstruct legacy expression trees for defaults; constant
///      defaults and executable occurrences retain separate owners.
pub(super) fn import_canonical_folded_value(
    materialiser: &mut GeneratedFoldedValueMaterialiser<'_, '_, impl MaterialisationNominalSource>,
    folded: &PublicFoldedValue,
    expected_type_id: TypeId,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    span: Option<SourceSpan>,
) -> Result<ConstValueId, ConstValueStoreError> {
    match folded {
        PublicFoldedValue::Number(value) => {
            if materialiser.type_environment.number_scale(expected_type_id) != Some(value.scale()) {
                return Err(CompilerError::compiler_error(format!(
                    "Imported folded Dec value at scale {} disagrees with its consumer-local canonical type",
                    value.scale()
                ))
                .into());
            }
            insert_literal(
                const_values,
                expected_type_id,
                span,
                ConstValuePayload::Number(value.clone()),
            )
        }
        PublicFoldedValue::FixedScalar(value) => insert_literal(
            const_values,
            expected_type_id,
            span,
            ConstValuePayload::FixedScalar(*value),
        ),
        PublicFoldedValue::Bool(value) => insert_literal(
            const_values,
            expected_type_id,
            span,
            ConstValuePayload::Bool(*value),
        ),
        PublicFoldedValue::Char(value) => insert_literal(
            const_values,
            expected_type_id,
            span,
            ConstValuePayload::Char(*value),
        ),
        PublicFoldedValue::String(value) => {
            let value = import_folded_string(materialiser, value, string_table, span)?;
            insert_literal(
                const_values,
                expected_type_id,
                span,
                ConstValuePayload::String(value),
            )
        }
        PublicFoldedValue::ConstTemplate { template, folded } => {
            let folded = match folded {
                Some(value) => Some(import_folded_string(
                    materialiser,
                    value,
                    string_table,
                    span,
                )?),
                None => None,
            };
            import_const_template(template, folded, expected_type_id, const_values, span)
        }
        PublicFoldedValue::Collection(values) => {
            let element_type_id = materialiser
                .type_environment
                .collection_shape(expected_type_id)
                .map(|shape| shape.element_type)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported folded collection value has a non-collection canonical type",
                    )
                })?;
            let mut items = Vec::with_capacity(values.len());
            for value in values {
                items.push(import_canonical_folded_value(
                    materialiser,
                    value,
                    element_type_id,
                    const_values,
                    string_table,
                    span,
                )?);
            }
            const_values.insert_value(
                ConstValueMetadata {
                    type_id: expected_type_id,
                    value_mode: ValueMode::ImmutableReference,
                    span,
                    const_record_state: crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,
                    contains_regular_division: false,
                    synthetic_interface_provenance: Default::default(),
                    value_kind: ConstValueKind::Composite,
                    hir_visible: true,
                },
                ConstValuePayload::Collection(items),
            )
        }
        PublicFoldedValue::Record(fields) => {
            if expected_type_id == materialiser.type_environment.anonymous_const_record_type() {
                return import_anonymous_record(
                    materialiser,
                    fields,
                    expected_type_id,
                    const_values,
                    string_table,
                    span,
                );
            }
            let definitions = materialiser
                .type_environment
                .fields_for(expected_type_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported folded record value has a canonical type without fields",
                    )
                })?
                .to_vec();
            import_record_fields(
                materialiser,
                fields,
                &definitions,
                expected_type_id,
                false,
                const_values,
                string_table,
                span,
            )
        }
        PublicFoldedValue::Choice {
            type_identity,
            variant_name,
            fields,
        } => {
            let choice_type_id = materialiser.intern_canonical_type(type_identity, string_table)?;
            if choice_type_id != expected_type_id {
                return Err(CompilerError::compiler_error(
                    "Imported folded choice value disagrees with its declared canonical type",
                )
                .into());
            }
            let variant = materialiser
                .type_environment
                .variants_for(choice_type_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported folded choice value has a non-choice canonical type",
                    )
                })?
                .iter()
                .find(|variant| string_table.resolve(variant.name) == variant_name)
                .cloned()
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Imported folded choice value names unknown variant '{variant_name}'"
                    ))
                })?;
            let definitions = match &variant.payload {
                ChoiceVariantPayloadDefinition::Unit => Vec::new(),
                ChoiceVariantPayloadDefinition::Record { fields } => fields.to_vec(),
            };
            let nominal_path = materialiser
                .type_environment
                .nominal_path(choice_type_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported folded choice type has no consumer-local nominal path",
                    )
                })?
                .to_owned();
            let fields = import_named_fields(
                materialiser,
                fields,
                &definitions,
                const_values,
                string_table,
                span,
            )?;
            const_values.insert_value(
                ConstValueMetadata {
                    type_id: expected_type_id,
                    value_mode: ValueMode::ImmutableReference,
                    span,
                    const_record_state: crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,
                    contains_regular_division: false,
                    synthetic_interface_provenance: Default::default(),
                    value_kind: ConstValueKind::Composite,
                    hir_visible: true,
                },
                ConstValuePayload::Choice { nominal_path, tag: variant.tag, fields },
            )
        }
        PublicFoldedValue::Range { start, end } => {
            let start_id = import_canonical_folded_value(
                materialiser,
                start,
                start_type_for_folded_range(
                    start,
                    expected_type_id,
                    materialiser.type_environment,
                )?,
                const_values,
                string_table,
                span,
            )?;
            let end_id = import_canonical_folded_value(
                materialiser,
                end,
                end_type_for_folded_range(end, expected_type_id, materialiser.type_environment)?,
                const_values,
                string_table,
                span,
            )?;
            const_values.insert_value(
                ConstValueMetadata {
                    type_id: expected_type_id,
                    value_mode: ValueMode::ImmutableReference,
                    span,
                    const_record_state: crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,
                    contains_regular_division: false,
                    synthetic_interface_provenance: Default::default(),
                    value_kind: ConstValueKind::Composite,
                    hir_visible: true,
                },
                ConstValuePayload::Range { start: start_id, end: end_id },
            )
        }
        PublicFoldedValue::OptionSome(value) => {
            let inner_type_id = materialiser
                .type_environment
                .option_inner_type(expected_type_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported folded option value has a non-option canonical type",
                    )
                })?;
            let inner = import_canonical_folded_value(
                materialiser,
                value,
                inner_type_id,
                const_values,
                string_table,
                span,
            )?;
            const_values.insert_value(
                ConstValueMetadata {
                    type_id: expected_type_id,
                    value_mode: ValueMode::ImmutableReference,
                    span,
                    const_record_state: crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,
                    contains_regular_division: false,
                    synthetic_interface_provenance: Default::default(),
                    value_kind: ConstValueKind::Composite,
                    hir_visible: true,
                },
                ConstValuePayload::OptionSome(inner),
            )
        }
        PublicFoldedValue::OptionNone => {
            let inner_type_id = materialiser
                .type_environment
                .option_inner_type(expected_type_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported folded option value has a non-option canonical type",
                    )
                })?;
            let inner_type = materialiser
                .type_environment
                .get(inner_type_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported folded option value has an unknown inner canonical type",
                    )
                })?;
            let _ = inner_type;
            const_values.insert_value(
                ConstValueMetadata {
                    type_id: expected_type_id,
                    value_mode: ValueMode::ImmutableReference,
                    span,
                    const_record_state: crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,
                    contains_regular_division: false,
                    synthetic_interface_provenance: Default::default(),
                    value_kind: ConstValueKind::Literal,
                    hir_visible: true,
                },
                ConstValuePayload::OptionNone,
            )
        }
    }
}

/// Converts a constant-store failure raised while projecting stable folded values.
///
/// WHY: `Infrastructure` already carries every compiler error that produced it. A `Diagnostic` means the
/// store rejected a projected node; stable folded input is validated upstream, so it is reported as an
/// invariant break rather than a user-facing diagnostic.
pub(super) fn const_store_error_into_compiler_error(
    error: ConstValueStoreError,
) -> Box<InfrastructureErrors> {
    match error {
        ConstValueStoreError::Infrastructure(errors) => errors,
        ConstValueStoreError::Diagnostic(diagnostic) => CompilerError::compiler_error(format!(
            "Imported folded value violated a constant store invariant: {diagnostic:?}"
        ))
        .into(),
    }
}
fn insert_literal(
    const_values: &mut ConstValueStore,
    type_id: TypeId,
    span: Option<SourceSpan>,
    payload: ConstValuePayload,
) -> Result<ConstValueId, ConstValueStoreError> {
    const_values.insert_value(
        ConstValueMetadata {
            type_id,
            value_mode: ValueMode::ImmutableReference,
            span,
            const_record_state: ConstRecordState::RuntimeValue,
            contains_regular_division: false,
            synthetic_interface_provenance: Default::default(),
            value_kind: ConstValueKind::Literal,
            hir_visible: true,
        },
        payload,
    )
}
fn import_folded_string(
    materialiser: &mut GeneratedFoldedValueMaterialiser<'_, '_, impl MaterialisationNominalSource>,
    value: &OwnedFoldedString,
    string_table: &mut StringTable,
    span: Option<SourceSpan>,
) -> Result<ConstStringValue, ConstValueStoreError> {
    match value {
        OwnedFoldedString::Text(text) => Ok(ConstStringValue::Text(string_table.intern(text))),
        OwnedFoldedString::Pieces(pieces) => {
            let mut projected = Vec::with_capacity(pieces.len());
            for piece in pieces {
                projected.push(match piece {
                    OwnedFoldedStringPiece::Text(text) => {
                        ConstStringPiece::Text(string_table.intern(text))
                    }
                    OwnedFoldedStringPiece::Resource(origin) => ConstStringPiece::Resource(
                        materialiser
                            .module_resources
                            .borrow_mut()
                            .intern_origin(origin.clone(), span),
                    ),
                    OwnedFoldedStringPiece::SiteRoot => ConstStringPiece::SiteRoot,
                });
            }
            Ok(ConstStringValue::Pieces(projected))
        }
    }
}
fn import_const_template(
    template: &PublicConstTemplate,
    folded: Option<ConstStringValue>,
    expected_type_id: TypeId,
    const_values: &mut ConstValueStore,
    span: Option<SourceSpan>,
) -> Result<ConstValueId, ConstValueStoreError> {
    let (kind, hir_visible) = match template.kind {
        PublicConstTemplateKind::Wrapper => (ConstValueKind::TemplateWrapper, true),
        PublicConstTemplateKind::SlotInsert(_) => (ConstValueKind::SlotInsertTemplate, false),
    };
    const_values.insert_value(
        ConstValueMetadata {
            type_id: expected_type_id,
            value_mode: ValueMode::ImmutableReference,
            span,
            const_record_state: ConstRecordState::RuntimeValue,
            contains_regular_division: false,
            synthetic_interface_provenance: Default::default(),
            value_kind: kind,
            hir_visible,
        },
        ConstValuePayload::Template {
            template: template.clone(),
            folded,
        },
    )
}
fn import_anonymous_record(
    materialiser: &mut GeneratedFoldedValueMaterialiser<'_, '_, impl MaterialisationNominalSource>,
    fields: &[PublicFoldedField],
    expected_type_id: TypeId,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    span: Option<SourceSpan>,
) -> Result<ConstValueId, ConstValueStoreError> {
    let mut projected = Vec::with_capacity(fields.len());
    for field in fields {
        let field_type_id =
            materialiser.intern_canonical_type(&field.type_identity, string_table)?;
        let field_path = materialiser
            .path_fork
            .try_intern_child(PathId::ROOT, string_table.intern(&field.name))
            .ok_or_else(|| CompilerError::compiler_error("Imported field path table exhausted"))?;
        let value = import_canonical_folded_value(
            materialiser,
            &field.value,
            field_type_id,
            const_values,
            string_table,
            span,
        )?;
        projected.push(ConstValueField {
            name: field_path,
            value,
        });
    }
    let mut metadata =
        ConstValueMetadata::resolved(expected_type_id, span, ValueMode::ImmutableReference);
    metadata.const_record_state = ConstRecordState::ConstRecord;
    metadata.value_kind = ConstValueKind::Composite;
    metadata.hir_visible = false;
    const_values.insert_value(metadata, ConstValuePayload::Record(projected))
}
#[allow(
    clippy::too_many_arguments,
    reason = "record import keeps the materialiser, field lists, type, const flag and mutable state separate"
)]
fn import_record_fields(
    materialiser: &mut GeneratedFoldedValueMaterialiser<'_, '_, impl MaterialisationNominalSource>,
    fields: &[PublicFoldedField],
    definitions: &[FieldDefinition],
    expected_type_id: TypeId,
    const_record: bool,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    span: Option<SourceSpan>,
) -> Result<ConstValueId, ConstValueStoreError> {
    let fields = import_named_fields(
        materialiser,
        fields,
        definitions,
        const_values,
        string_table,
        span,
    )?;
    let mut metadata =
        ConstValueMetadata::resolved(expected_type_id, span, ValueMode::ImmutableReference);
    if const_record {
        metadata.const_record_state = ConstRecordState::ConstRecord;
        metadata.hir_visible = false;
    }
    metadata.value_kind = ConstValueKind::Composite;
    const_values.insert_value(metadata, ConstValuePayload::Record(fields))
}
fn import_named_fields(
    materialiser: &mut GeneratedFoldedValueMaterialiser<'_, '_, impl MaterialisationNominalSource>,
    fields: &[PublicFoldedField],
    definitions: &[FieldDefinition],
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    span: Option<SourceSpan>,
) -> Result<Vec<ConstValueField>, ConstValueStoreError> {
    if fields.len() != definitions.len() {
        return Err(CompilerError::compiler_error(
            "Imported folded aggregate value has a field count inconsistent with its canonical type",
        )
        .into());
    }
    let mut projected = Vec::with_capacity(fields.len());
    for (field, definition) in fields.iter().zip(definitions) {
        let definition_name = materialiser
            .path_fork
            .component(definition.name)
            .map(|name| string_table.resolve(name))
            .ok_or_else(|| CompilerError::compiler_error("Imported field path has no name"))?;
        if definition_name != field.name {
            return Err(CompilerError::compiler_error(format!(
                "Imported folded aggregate field '{}' does not match canonical field '{definition_name}'",
                field.name
            ))
            .into());
        }
        let value = import_canonical_folded_value(
            materialiser,
            &field.value,
            definition.type_id,
            const_values,
            string_table,
            span,
        )?;
        projected.push(ConstValueField {
            name: definition.name,
            value,
        });
    }
    Ok(projected)
}
fn start_type_for_folded_range(
    start: &PublicFoldedValue,
    expected_type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> Result<TypeId, ConstValueStoreError> {
    match start {
        PublicFoldedValue::FixedScalar(value) => Ok(
            crate::compiler_frontend::datatypes::ids::builtin_type_ids::fixed_scalar(
                value.scalar(),
            ),
        ),
        _ => {
            let _ = (expected_type_id, type_environment);
            Err(CompilerError::compiler_error(
                "Imported folded range endpoints must be fixed scalar values",
            )
            .into())
        }
    }
}
fn end_type_for_folded_range(
    end: &PublicFoldedValue,
    expected_type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> Result<TypeId, ConstValueStoreError> {
    start_type_for_folded_range(end, expected_type_id, type_environment)
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
