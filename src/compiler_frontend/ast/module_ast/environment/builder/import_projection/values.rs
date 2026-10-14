//! Imported aliases, constants, defaults, and folded values.

use super::*;
use crate::compiler_frontend::ast::ast_nodes::DeclarationValue;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};

use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstValueField, ConstValueMetadata,
};
use crate::compiler_frontend::ast::const_values::{
    ConstValueId, ConstValuePayload, ConstValueStore, ConstValueStoreError,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    ConstRecordState, ConstValueKind,
};
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceVariantPayloadDefinition, FieldDefinition,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, OwnedFoldedStringPiece, PublicConstTemplateKind, PublicFoldedField,
    PublicFoldedValue,
};
use crate::compiler_frontend::paths::module_resources::ResourceId;
use crate::compiler_frontend::paths::resource_identity::StableResourceOriginId;
use crate::compiler_frontend::project_globals::PROJECT_GLOBALS_DEPENDENCY_NAME;
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, OriginDeclarationId};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::value_mode::ValueMode;
/// Services needed to invert one stable folded value into a fresh AST environment.
///
/// WHAT: keeps canonical-type interning, path interning and the active type environment explicit
/// while allowing imported declarations and generated materialisation to share one folded-value
/// inverse projection.
/// WHY: a second recursive value converter would make defaults and constants diverge between
/// provider dependencies and generated sidecars.
pub(crate) trait FoldedValueMaterialiser {
    fn intern_resource_origin(
        &mut self,
        origin: &StableResourceOriginId,
        span: Option<SourceSpan>,
    ) -> Result<ResourceId, CompilerError>;
    fn intern_canonical_type(
        &mut self,
        identity: &crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity,
        string_table: &mut StringTable,
    ) -> Result<TypeId, CompilerError>;
    fn path_fork(&mut self) -> &mut PathInternerFork;

    fn type_environment(&self) -> &TypeEnvironment;
}

impl<'context, 'services> FoldedValueMaterialiser
    for AstModuleEnvironmentBuilder<'context, 'services>
{
    fn intern_canonical_type(
        &mut self,
        identity: &crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity,
        _string_table: &mut StringTable,
    ) -> Result<TypeId, CompilerError> {
        self.intern_imported_canonical_type(identity)
    }

    fn intern_resource_origin(
        &mut self,
        origin: &StableResourceOriginId,
        span: Option<SourceSpan>,
    ) -> Result<ResourceId, CompilerError> {
        let services = self.context.file_value_resolution.as_ref().ok_or_else(|| {
            CompilerError::compiler_error(
                "Imported structural string projection requires module file-value resolution services",
            )
        })?;
        Ok(services
            .module_resources
            .borrow_mut()
            .intern_origin(origin.clone(), span))
    }
    fn path_fork(&mut self) -> &mut PathInternerFork {
        self.path_fork
    }

    fn type_environment(&self) -> &TypeEnvironment {
        &self.type_environment
    }
}

impl<'context, 'services> AstModuleEnvironmentBuilder<'context, 'services> {
    pub(in crate::compiler_frontend::ast::module_ast::environment::builder) fn project_imported_alias_declarations(
        &mut self,
    ) -> Result<(), CompilerError> {
        let imported = self
            .binding_environment
            .imported_declarations_by_local_path
            .clone();

        for (local_path, origin) in imported {
            let alias_target = {
                let Some(record) = self
                    .binding_environment
                    .imported_declarations_by_origin
                    .get(&origin)
                else {
                    continue;
                };
                let PublicDeclarationSemantics::TransparentAlias(alias) = &record.semantics else {
                    continue;
                };
                alias.target_type_identity.clone()
            };
            let type_id = self.intern_imported_canonical_type(&alias_target)?;
            Rc::make_mut(&mut self.resolved_type_aliases_by_path).insert(
                local_path,
                ResolvedTypeAlias {
                    diagnostic_type: diagnostic_type_spelling(type_id, &self.type_environment),
                    target_type_id: type_id,
                    declaration_span: None,
                },
            );
        }

        Ok(())
    }

    pub(in crate::compiler_frontend::ast::module_ast::environment::builder) fn project_imported_constant_declarations(
        &mut self,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerError> {
        let imported = self
            .binding_environment
            .imported_declarations_by_local_path
            .clone();
        for (local_path, origin) in imported {
            let (constant, declaration_provenance) = {
                let Some(record) = self
                    .binding_environment
                    .imported_declarations_by_origin
                    .get(&origin)
                else {
                    continue;
                };
                let PublicDeclarationSemantics::Constant(constant) = &record.semantics else {
                    continue;
                };
                (
                    constant.clone(),
                    record.synthetic_interface_provenance.clone(),
                )
            };
            let declaration = self.project_imported_constant(
                local_path,
                &constant,
                &declaration_provenance,
                &origin,
                string_table,
            )?;
            append_projected_constant(
                &mut self.declaration_table,
                &mut self.resolved_module_constants,
                declaration,
                self.path_fork,
            )?;
        }
        Ok(())
    }
    fn project_imported_constant(
        &mut self,
        local_path: PathId,
        constant: &PublicConstantSemantics,
        declaration_provenance: &SyntheticInterfaceProvenance,
        origin: &OriginDeclarationId,
        string_table: &mut StringTable,
    ) -> Result<Declaration, CompilerError> {
        let type_id = self.intern_imported_canonical_type(&constant.type_identity)?;
        let mut synthetic_interface_provenance = declaration_provenance.clone();
        if is_project_globals_origin(origin) {
            let project_member = SyntheticInterfaceMemberIdentity::new(
                SyntheticInterfaceClass::ProjectContext,
                PROJECT_GLOBALS_DEPENDENCY_NAME,
                origin.defining_name(),
            );
            synthetic_interface_provenance = synthetic_interface_provenance
                .union(&SyntheticInterfaceProvenance::single(project_member));
        }
        // Imported constants are explicit compile-time values: a named record root is a nominal
        // const record, and declaration provenance attaches to that root node only.
        let root = ImportedRootMetadata {
            const_record: true,
            synthetic_interface_provenance,
        };
        let id = self
            .with_owned_const_values(|builder, const_values| {
                import_folded_value(
                    builder,
                    &constant.folded_value,
                    type_id,
                    const_values,
                    string_table,
                    None,
                    root,
                )
            })
            .map_err(const_store_error_into_compiler_error)?;
        self.context
            .const_values
            .bind_module_constant(local_path, id)
            .map_err(const_store_error_into_compiler_error)?;

        Ok(Declaration {
            id: local_path,
            type_id,
            diagnostic_type: diagnostic_type_spelling(type_id, &self.type_environment),
            value_mode: ValueMode::ImmutableReference,
            value: DeclarationValue::Default(id),
            binding_span: None,
            config_qualifier: None,
        })
    }

    /// Runs one projection with the constant store moved out of the builder.
    ///
    /// WHY: projection needs `&mut ConstValueStore` alongside `&mut self` as the materialiser. The
    /// store is taken out for the call and restored afterwards, so the two borrows never overlap.
    fn with_owned_const_values<T>(
        &mut self,
        project: impl FnOnce(&mut Self, &mut ConstValueStore) -> T,
    ) -> T {
        let mut const_values = std::mem::take(&mut self.context.const_values);
        let result = project(self, &mut const_values);
        self.context.const_values = const_values;
        result
    }

    pub(in crate::compiler_frontend::ast::module_ast::environment) fn project_imported_folded_value(
        &mut self,
        folded: &PublicFoldedValue,
        expected_type_id: TypeId,
        string_table: &mut StringTable,
    ) -> Result<ConstValueId, CompilerError> {
        // Public folded values intentionally omit declaration spans, so imported projections
        // have no authored range to carry into a consumer-local resource table.
        self.with_owned_const_values(|builder, const_values| {
            materialize_public_folded_value(
                builder,
                folded,
                expected_type_id,
                const_values,
                string_table,
                None,
            )
        })
    }
}

fn is_project_globals_origin(origin: &OriginDeclarationId) -> bool {
    let module_origin = origin.module_origin();
    module_origin.role() == ModuleRootRole::ProjectPackageFacade
        && module_origin.logical_module_path() == "project-globals"
}

fn append_projected_constant(
    declaration_table: &mut Rc<TopLevelDeclarationTable>,
    resolved_constants: &mut Rc<ResolvedConstantSet>,
    declaration: Declaration,
    path_fork: &PathInternerFork,
) -> Result<DeclarationId, CompilerError> {
    let declaration_id = Rc::make_mut(declaration_table)
        .append_for_construction(declaration, path_fork)
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "Imported constant declaration path was registered more than once",
            )
        })?;
    Rc::make_mut(resolved_constants).insert(declaration_id);
    Ok(declaration_id)
}

#[cfg(test)]
#[path = "../../../../tests/imported_constant_projection_tests.rs"]
mod tests;
/// Rebuild one owned folded string in the active module-local string and resource tables.
///
/// This is the string-specific inverse projection shared by imported declarations and generated
/// materialisation. Its callback keeps resource interning explicit without coupling this leaf
/// projection to the full recursive [`FoldedValueMaterialiser`] service bundle.
pub(in crate::compiler_frontend::ast) fn materialize_owned_folded_string(
    value: &OwnedFoldedString,
    string_table: &mut StringTable,
    mut intern_resource_origin: impl FnMut(&StableResourceOriginId) -> Result<ResourceId, CompilerError>,
) -> Result<ConstStringValue, CompilerError> {
    match value {
        OwnedFoldedString::Text(text) => Ok(ConstStringValue::Text(string_table.intern(text))),
        OwnedFoldedString::Pieces(pieces) => {
            let mut projected = Vec::with_capacity(pieces.len());
            for piece in pieces {
                projected.push(match piece {
                    OwnedFoldedStringPiece::Text(text) => {
                        ConstStringPiece::Text(string_table.intern(text))
                    }
                    OwnedFoldedStringPiece::Resource(origin) => {
                        ConstStringPiece::Resource(intern_resource_origin(origin)?)
                    }
                    OwnedFoldedStringPiece::SiteRoot => ConstStringPiece::SiteRoot,
                });
            }
            Ok(ConstStringValue::Pieces(projected))
        }
    }
}

/// Rebuild one stable folded value as interned nodes in the consumer's constant store.
///
/// WHAT: walks the public value against consumer-local canonical types and inserts each node once.
/// WHY: constant content lives only in `ConstValueStore`; projection creates no expression nodes.
pub(crate) fn materialize_public_folded_value<M: FoldedValueMaterialiser>(
    materialiser: &mut M,
    folded: &PublicFoldedValue,
    expected_type_id: TypeId,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    span: Option<SourceSpan>,
) -> Result<ConstValueId, CompilerError> {
    import_folded_value(
        materialiser,
        folded,
        expected_type_id,
        const_values,
        string_table,
        span,
        ImportedRootMetadata::default(),
    )
    .map_err(const_store_error_into_compiler_error)
}

/// Declaration-level facts applied only to the outermost node of one imported projection.
///
/// WHAT: carries declaration provenance and whether a named record is a nominal const record.
/// WHY: nested folded children never inherit declaration facts, so only the root node stores them.
#[derive(Default)]
struct ImportedRootMetadata {
    const_record: bool,
    synthetic_interface_provenance: SyntheticInterfaceProvenance,
}

/// Converts a constant-store failure raised while projecting an imported value.
///
/// WHY: `Infrastructure` already carries the compiler error that produced it. A `Diagnostic` means the
/// store rejected a projected node; imported folded input is validated upstream, so it is reported as
/// an invariant break rather than a user-facing diagnostic.
fn const_store_error_into_compiler_error(error: ConstValueStoreError) -> CompilerError {
    match error {
        ConstValueStoreError::Infrastructure(error) => *error,
        ConstValueStoreError::Diagnostic(diagnostic) => CompilerError::compiler_error(format!(
            "Imported folded value violated a constant store invariant: {diagnostic:?}"
        )),
    }
}

fn import_folded_value<M: FoldedValueMaterialiser>(
    materialiser: &mut M,
    folded: &PublicFoldedValue,
    expected_type_id: TypeId,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    span: Option<SourceSpan>,
    root: ImportedRootMetadata,
) -> Result<ConstValueId, ConstValueStoreError> {
    let node = |value_kind: ConstValueKind| ConstValueMetadata {
        type_id: expected_type_id,
        value_mode: ValueMode::ImmutableReference,
        span,
        const_record_state: ConstRecordState::RuntimeValue,
        contains_regular_division: false,
        synthetic_interface_provenance: root.synthetic_interface_provenance.clone(),
        value_kind,
        hir_visible: true,
    };

    match folded {
        PublicFoldedValue::Number(value) => {
            if materialiser
                .type_environment()
                .number_scale(expected_type_id)
                != Some(value.scale())
            {
                return Err(CompilerError::compiler_error(format!(
                    "Imported folded Dec value at scale {} disagrees with its consumer-local canonical type",
                    value.scale()
                ))
                .into());
            }
            const_values.insert_value(
                node(ConstValueKind::Literal),
                ConstValuePayload::Number(value.clone()),
            )
        }
        PublicFoldedValue::FixedScalar(value) => const_values.insert_value(
            node(ConstValueKind::Literal),
            ConstValuePayload::FixedScalar(*value),
        ),
        PublicFoldedValue::Bool(value) => const_values.insert_value(
            node(ConstValueKind::Literal),
            ConstValuePayload::Bool(*value),
        ),
        PublicFoldedValue::Char(value) => const_values.insert_value(
            node(ConstValueKind::Literal),
            ConstValuePayload::Char(*value),
        ),
        PublicFoldedValue::String(value) => {
            let value = materialize_owned_folded_string(value, string_table, |origin| {
                materialiser.intern_resource_origin(origin, span)
            })?;
            const_values.insert_value(
                node(ConstValueKind::Literal),
                ConstValuePayload::String(value),
            )
        }
        PublicFoldedValue::ConstTemplate { template, folded } => {
            let (value_kind, hir_visible) = match template.kind {
                PublicConstTemplateKind::Wrapper => (ConstValueKind::TemplateWrapper, true),
                PublicConstTemplateKind::SlotInsert(_) => {
                    (ConstValueKind::SlotInsertTemplate, false)
                }
            };
            let folded = folded
                .as_ref()
                .map(|value| {
                    materialize_owned_folded_string(value, string_table, |origin| {
                        materialiser.intern_resource_origin(origin, span)
                    })
                })
                .transpose()?;
            let mut metadata = node(value_kind);
            metadata.hir_visible = hir_visible;
            const_values.insert_value(
                metadata,
                ConstValuePayload::Template {
                    template: template.clone(),
                    folded,
                },
            )
        }
        PublicFoldedValue::Collection(values) => {
            let element_type_id = materialiser
                .type_environment()
                .collection_shape(expected_type_id)
                .map(|shape| shape.element_type)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported folded collection value has a non-collection canonical type",
                    )
                })?;
            let mut items = Vec::with_capacity(values.len());
            for value in values {
                items.push(import_folded_value(
                    materialiser,
                    value,
                    element_type_id,
                    const_values,
                    string_table,
                    span,
                    ImportedRootMetadata::default(),
                )?);
            }
            const_values.insert_value(
                node(ConstValueKind::Composite),
                ConstValuePayload::Collection(items),
            )
        }
        PublicFoldedValue::Record(fields) => {
            // Anonymous const records keep their field facts on the folded values: the
            // compile-time marker type has no field definitions, so import projects the folded
            // fields directly and marks the node as a compile-time member group.
            if expected_type_id
                == materialiser
                    .type_environment()
                    .anonymous_const_record_type()
            {
                let mut projected = Vec::with_capacity(fields.len());
                for field in fields {
                    let field_type_id =
                        materialiser.intern_canonical_type(&field.type_identity, string_table)?;
                    let field_path = materialiser
                        .path_fork()
                        .try_intern_child(PathId::ROOT, string_table.intern(&field.name))
                        .ok_or_else(|| {
                            CompilerError::compiler_error("Imported field path table exhausted")
                        })?;
                    let value = import_folded_value(
                        materialiser,
                        &field.value,
                        field_type_id,
                        const_values,
                        string_table,
                        span,
                        ImportedRootMetadata::default(),
                    )?;
                    projected.push(ConstValueField {
                        name: field_path,
                        value,
                    });
                }
                let mut metadata = node(ConstValueKind::Composite);
                metadata.const_record_state = ConstRecordState::ConstRecord;
                metadata.hir_visible = false;
                const_values.insert_value(metadata, ConstValuePayload::Record(projected))
            } else {
                let definitions = materialiser
                    .type_environment()
                    .fields_for(expected_type_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Imported folded record value has a canonical type without fields",
                        )
                    })?
                    .to_vec();
                let projected = import_named_fields(
                    materialiser,
                    fields,
                    &definitions,
                    const_values,
                    string_table,
                    span,
                )?;
                let mut metadata = node(ConstValueKind::Composite);
                if root.const_record {
                    metadata.const_record_state = ConstRecordState::ConstRecord;
                    metadata.hir_visible = false;
                }
                const_values.insert_value(metadata, ConstValuePayload::Record(projected))
            }
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
                .type_environment()
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
            let field_definitions = match &variant.payload {
                ChoiceVariantPayloadDefinition::Unit => &[][..],
                ChoiceVariantPayloadDefinition::Record { fields } => fields.as_ref(),
            };
            let nominal_path = materialiser
                .type_environment()
                .nominal_path(choice_type_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported folded choice type has no consumer-local nominal path",
                    )
                })?
                .to_owned();
            let projected = import_named_fields(
                materialiser,
                fields,
                field_definitions,
                const_values,
                string_table,
                span,
            )?;
            const_values.insert_value(
                node(ConstValueKind::Composite),
                ConstValuePayload::Choice {
                    nominal_path,
                    tag: variant.tag,
                    fields: projected,
                },
            )
        }
        PublicFoldedValue::Range { start, end } => {
            // A range has no shared endpoint identity: preserve each folded scalar's exact type
            // instead of applying the unconstrained literal default.
            let start_type_id = folded_range_endpoint_type(start)?;
            let end_type_id = folded_range_endpoint_type(end)?;
            let start_id = import_folded_value(
                materialiser,
                start,
                start_type_id,
                const_values,
                string_table,
                span,
                ImportedRootMetadata::default(),
            )?;
            let end_id = import_folded_value(
                materialiser,
                end,
                end_type_id,
                const_values,
                string_table,
                span,
                ImportedRootMetadata::default(),
            )?;
            const_values.insert_value(
                node(ConstValueKind::Composite),
                ConstValuePayload::Range {
                    start: start_id,
                    end: end_id,
                },
            )
        }
        PublicFoldedValue::OptionSome(value) => {
            let inner_type_id = materialiser
                .type_environment()
                .option_inner_type(expected_type_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported folded option value has a non-option canonical type",
                    )
                })?;
            let inner = import_folded_value(
                materialiser,
                value,
                inner_type_id,
                const_values,
                string_table,
                span,
                ImportedRootMetadata::default(),
            )?;
            const_values.insert_value(
                node(ConstValueKind::Composite),
                ConstValuePayload::OptionSome(inner),
            )
        }
        PublicFoldedValue::OptionNone => {
            const_values.insert_value(node(ConstValueKind::Literal), ConstValuePayload::OptionNone)
        }
    }
}

fn folded_range_endpoint_type(
    endpoint: &PublicFoldedValue,
) -> Result<TypeId, ConstValueStoreError> {
    match endpoint {
        PublicFoldedValue::FixedScalar(value) => Ok(builtin_type_ids::fixed_scalar(value.scalar())),
        _ => Err(CompilerError::compiler_error(
            "Imported folded range endpoints must be fixed scalar values",
        )
        .into()),
    }
}

fn import_named_fields<M: FoldedValueMaterialiser>(
    materialiser: &mut M,
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
            .path_fork()
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
        let value = import_folded_value(
            materialiser,
            &field.value,
            definition.type_id,
            const_values,
            string_table,
            span,
            ImportedRootMetadata::default(),
        )?;
        projected.push(ConstValueField {
            name: definition.name,
            value,
        });
    }
    Ok(projected)
}
