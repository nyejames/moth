//! Module-local folded values.
//!
//! WHAT: owns the compact value graph and module-constant rows produced for one AST module after
//! constant evaluation.  The graph is the only folded-value representation retained across the
//! AST finalization boundary; public projection and HIR use its borrowed postorder visitor.
//! WHY: a finalized module constant was previously retained as a declaration, recursively
//! normalized into another declaration tree, then recursively interpreted once by public
//! projection and again by HIR.  Stable scalar and aggregate facts belong in one indexed store.
//!
//! The store is module-local.  Its IDs and rows must never enter a cross-module
//! interface; public projection converts them to [`PublicFoldedValue`] before publication.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveSource, ReactiveTemplateMetadata,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    ConstRecordState, ConstValueKind,
};
use crate::compiler_frontend::ast::module_ast::environment::declaration_table::{
    ResolvedConstantSet, TopLevelDeclarationTable,
};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::folded_value::PublicConstTemplate;
use crate::compiler_frontend::paths::module_resources::ResourceId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;

#[cfg(test)]
#[path = "tests/store_tests.rs"]
mod tests;

#[cfg(test)]
mod test_support;

/// Dense identity for one node in a module's folded-value graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ConstValueId(u32);

impl ConstValueId {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// Compact authored module-constant row.
///
/// WHAT: pairs one module constant's defining path with the folded value the store owns for it,
/// in declaration-table order.  The declaration table remains the source declaration owner.
/// WHY: every consumer - config extraction, public projection, HIR, generated materialisation -
/// joins a module constant by its exact defining `InternedPath`, so the path is the row key.
#[derive(Clone, Debug)]
struct ConstValueRow {
    path: InternedPath,
    value: ConstValueId,
}

/// A template result supplied by the AST finalization owner.
///
/// WHAT: keeps exact TIR classification and folding in finalization while the store owns the
/// resulting neutral value.  The callback is invoked with the original template reference, so
/// callers cannot classify a template from a reconstructed or flattened shape.
pub(crate) enum ConstTemplateValue {
    /// A renderable template's fully folded string, plain text or structural pieces.
    ///
    /// WHAT: the same [`ConstStringValue`] an ordinary folded string stores, so a
    /// piece-bearing template fold stays structural exactly like a file-value constant.
    Folded {
        value: ConstStringValue,
        provenance: SyntheticInterfaceProvenance,
    },
    Public {
        template: PublicConstTemplate,
        kind: ConstValueKind,
        hir_visible: bool,
        /// The template's folded string when finalization produced one; structural pieces
        /// stay intact until a consumer can represent them.
        folded: Option<ConstStringValue>,
        provenance: SyntheticInterfaceProvenance,
    },
}

/// User-facing or infrastructure failure while constructing the module store.
#[derive(Debug)]
pub(crate) enum ConstValueStoreError {
    Diagnostic(Box<CompilerDiagnostic>),
    Infrastructure(Box<CompilerError>),
}

impl From<CompilerDiagnostic> for ConstValueStoreError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        Self::Diagnostic(Box::new(diagnostic))
    }
}

impl From<CompilerError> for ConstValueStoreError {
    fn from(error: CompilerError) -> Self {
        Self::Infrastructure(Box::new(error))
    }
}

/// Metadata preserved for one folded value node.
///
/// WHAT: retains the semantic type, source context and value facts that were previously carried
/// by every cloned `Expression`.  `diagnostic_type` and access metadata are kept for the short
/// config/template services and for temporary advisory resolution; semantic consumers use
/// `type_id`.
#[derive(Clone, Debug)]
pub(crate) struct ConstValueMetadata {
    pub(crate) type_id: TypeId,
    pub(crate) diagnostic_type: DataType,
    pub(crate) value_mode: ValueMode,
    pub(crate) location: SourceLocation,
    pub(crate) reactive_source: Option<ReactiveSource>,
    pub(crate) reactive_template: Option<ReactiveTemplateMetadata>,
    pub(crate) const_record_state: ConstRecordState,
    pub(crate) contains_regular_division: bool,
    pub(crate) synthetic_interface_provenance: SyntheticInterfaceProvenance,
    pub(crate) value_kind: ConstValueKind,
    pub(crate) hir_visible: bool,
}

/// A named field in a folded record or choice payload.
///
/// WHAT: keeps authored field order, the folded value id, and the field's declaration
/// location. `location` preserves declaration provenance for diagnostics after folding;
/// it is not remapped because the store is consumed before the module-wide string remap.
#[derive(Clone, Debug)]
pub(crate) struct ConstValueField {
    pub(crate) name: InternedPath,
    pub(crate) value: ConstValueId,
    /// Field initializer location for diagnostic projection after folding. Public folded
    /// values remain location-free. Store tests read this; production diagnostics still
    /// project from the value-node location.
    #[allow(dead_code)]
    pub(crate) location: SourceLocation,
}

/// A folded `String` value: plain text or an ordered sequence of structural pieces.
///
/// WHAT: the value graph's representation of every folded `String`. Plain-text values keep the
/// compact fast path and never allocate a piece vector - authored slices and folds that emit
/// only text land there - while folds whose output carries structural pieces produce the
/// `Pieces` form, where resource origins and the site root stay structural instead of being
/// rendered to URL text at fold time.
/// WHY: a resource-bearing `String` is an ordinary `String` at the language level, so its
/// structure must survive folding until the builder resolves each piece's URL context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConstStringValue {
    Text(StringId),
    Pieces(Vec<ConstStringPiece>),
}

/// One structural piece of a folded `String`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConstStringPiece {
    /// A literal text run inside a piece-bearing value.
    ///
    /// WHAT: the compact interned form an ordinary folded string keeps for plain runs.
    /// WHY: each run's position among the structural pieces around it must survive fold,
    /// projection and handoff until the builder resolves every piece's URL context.
    Text(StringId),

    /// One resource origin interned in the module resource table.
    Resource(ResourceId),

    /// The site root, rendered with the consuming artefact's project-origin policy.
    SiteRoot,
}

/// Payload variants stored in the module-local value graph.
#[derive(Clone, Debug)]
pub(crate) enum ConstValuePayload {
    Int(i32),
    Float(f64),
    Bool(bool),
    Char(char),
    String(ConstStringValue),
    Collection(Vec<ConstValueId>),
    Record(Vec<ConstValueField>),
    Choice {
        nominal_path: InternedPath,
        tag: usize,
        fields: Vec<ConstValueField>,
    },
    Range {
        start: ConstValueId,
        end: ConstValueId,
    },
    Coerced(ConstValueId),
    OptionSome(ConstValueId),
    OptionNone,
    Template {
        template: PublicConstTemplate,
        folded: Option<ConstStringValue>,
    },
}

/// One value node and its preserved source/semantic metadata.
#[derive(Clone, Debug)]
pub(crate) struct ConstValue {
    pub(crate) metadata: ConstValueMetadata,
    pub(crate) payload: ConstValuePayload,
}

/// Borrowed postorder shape passed to public and HIR consumers.
///
/// The store owns recursion.  Consumers only map this already traversed shape to their own
/// boundary vocabulary, so they cannot independently walk and reinterpret AST expressions.
pub(crate) enum ConstValueVisit<'a, T> {
    Int(i32),
    Float(f64),
    Bool(bool),
    Char(char),
    String(&'a ConstStringValue),
    Collection(Vec<T>),
    Record(Vec<ConstValueFieldVisit<'a, T>>),
    Choice {
        nominal_path: &'a InternedPath,
        tag: usize,
        fields: Vec<ConstValueFieldVisit<'a, T>>,
    },
    Range {
        start: T,
        end: T,
    },
    Coerced(T),
    OptionSome(T),
    OptionNone,
    Template {
        template: &'a PublicConstTemplate,
        folded: Option<&'a ConstStringValue>,
    },
}

/// One module-constant row, borrowed together with the metadata it is guaranteed to have.
pub(crate) struct ConstValueRowView<'a> {
    pub(crate) path: &'a InternedPath,
    pub(crate) id: ConstValueId,
    pub(crate) metadata: &'a ConstValueMetadata,
}

/// One folded field visited with the semantic type of its value node.
///
/// WHAT: pairs the authored field name with the value's store metadata `TypeId` so public
/// projection can project each field's canonical identity without a second lookup.
pub(crate) struct ConstValueFieldVisit<'a, T> {
    pub(crate) name: &'a InternedPath,
    pub(crate) type_id: TypeId,
    pub(crate) value: T,
}

/// The one module-local folded-value authority.
#[derive(Clone, Debug, Default)]
pub(crate) struct ConstValueStore {
    values: Vec<ConstValue>,
    rows: Vec<ConstValueRow>,
    values_by_path: FxHashMap<InternedPath, ConstValueId>,
}

impl ConstValueStore {
    /// Build the store in declaration-table order.
    ///
    /// `template_builder` remains owned by AST finalization.  It receives the exact original
    /// template and its defining path (only for a root module constant) and returns the result
    /// of the exact TIR view/fold operation.  The store never reconstructs template identity.
    pub(crate) fn from_declaration_table(
        declaration_table: &TopLevelDeclarationTable,
        resolved_module_constants: &ResolvedConstantSet,
        type_environment: &TypeEnvironment,
        template_builder: &mut impl FnMut(
            Option<&InternedPath>,
            &crate::compiler_frontend::ast::templates::template::Template,
        ) -> Result<ConstTemplateValue, ConstValueStoreError>,
    ) -> Result<Self, ConstValueStoreError> {
        let mut store = Self::default();
        let mut pending: Vec<_> = resolved_module_constants.iter().collect();

        while !pending.is_empty() {
            let mut deferred = Vec::new();
            let mut progress = false;

            for declaration_id in pending {
                let declaration = declaration_table.get_by_id(declaration_id).ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Resolved module-constant ID had no declaration-table row.",
                    )
                })?;

                if Self::waits_for_const_record_target(&declaration.value, &store) {
                    deferred.push(declaration_id);
                    continue;
                }

                let value = store.insert_expression(
                    &declaration.value,
                    Some(&declaration.id),
                    type_environment,
                    template_builder,
                )?;
                store.rows.push(ConstValueRow {
                    path: declaration.id.clone(),
                    value,
                });
                if store
                    .values_by_path
                    .insert(declaration.id.clone(), value)
                    .is_some()
                {
                    return Err(CompilerError::compiler_error(
                        "ConstValueStore received duplicate module-constant declaration paths.",
                    )
                    .into());
                }
                progress = true;
            }

            if !deferred.is_empty() && !progress {
                return Err(CompilerError::compiler_error(
                    "module constant reference reached ConstValueStore before its target folded",
                )
                .into());
            }

            pending = deferred;
        }

        Ok(store)
    }

    /// Bind a body-local const record by its qualified declaration path.
    ///
    /// WHAT: folds the record into the shared value graph without creating a module-constant
    /// row. HIR looks the value up by path; public projection never sees it.
    pub(crate) fn insert_body_local_binding(
        &mut self,
        declaration: &Declaration,
        type_environment: &TypeEnvironment,
        template_builder: &mut impl FnMut(
            Option<&InternedPath>,
            &Template,
        ) -> Result<ConstTemplateValue, ConstValueStoreError>,
    ) -> Result<(), ConstValueStoreError> {
        if Self::waits_for_const_record_target(&declaration.value, self) {
            return Err(CompilerError::compiler_error(format!(
                "body-local const record {:?} reached ConstValueStore before its target folded",
                declaration.id
            ))
            .into());
        }

        let value = self.insert_expression(
            &declaration.value,
            Some(&declaration.id),
            type_environment,
            template_builder,
        )?;
        if self
            .values_by_path
            .insert(declaration.id.clone(), value)
            .is_some()
        {
            return Err(CompilerError::compiler_error(
                "ConstValueStore received duplicate body-local const-record paths.",
            )
            .into());
        }

        Ok(())
    }

    fn waits_for_const_record_target(expression: &Expression, store: &Self) -> bool {
        match &expression.kind {
            ExpressionKind::Reference(path) => {
                expression.is_const_record_value() && !store.values_by_path.contains_key(path)
            }
            ExpressionKind::AnonymousConstRecord { fields }
            | ExpressionKind::StructInstance(fields) => fields
                .iter()
                .any(|field| Self::waits_for_const_record_target(&field.value, store)),
            ExpressionKind::Collection(items) => items
                .iter()
                .any(|item| Self::waits_for_const_record_target(item, store)),
            _ => false,
        }
    }

    fn insert_record_fields(
        &mut self,

        fields: &[Declaration],
        type_environment: &TypeEnvironment,
        template_builder: &mut impl FnMut(
            Option<&InternedPath>,
            &crate::compiler_frontend::ast::templates::template::Template,
        ) -> Result<ConstTemplateValue, ConstValueStoreError>,
    ) -> Result<Vec<ConstValueField>, ConstValueStoreError> {
        let mut field_index_by_name: FxHashMap<StringId, usize> = FxHashMap::default();
        let mut stored_fields = Vec::with_capacity(fields.len());

        for field in fields {
            let value =
                self.insert_expression(&field.value, None, type_environment, template_builder)?;
            let name = field.id.name().ok_or_else(|| {
                CompilerError::compiler_error(
                    "ConstValueStore record field declaration has no interned field name.",
                )
            })?;
            if field_index_by_name
                .insert(name, stored_fields.len())
                .is_some()
            {
                return Err(CompilerError::compiler_error(
                    "ConstValueStore received a record with duplicate field names.",
                )
                .into());
            }

            stored_fields.push(ConstValueField {
                name: field.id.clone(),
                value,
                location: field.value.location.clone(),
            });
        }

        Ok(stored_fields)
    }

    fn insert_expression(
        &mut self,
        expression: &Expression,
        defining_path: Option<&InternedPath>,
        type_environment: &TypeEnvironment,
        template_builder: &mut impl FnMut(
            Option<&InternedPath>,
            &Template,
        ) -> Result<ConstTemplateValue, ConstValueStoreError>,
    ) -> Result<ConstValueId, ConstValueStoreError> {
        if let ExpressionKind::Reference(path) = &expression.kind
            && expression.is_const_record_value()
        {
            let target = self.values_by_path.get(path).copied().ok_or_else(|| {
                ConstValueStoreError::from(CompilerError::compiler_error(format!(
                    "module constant reference {:?} reached ConstValueStore before its target folded",
                    path
                )))
            })?;
            // Aliases share the target root. Distinct `ConstValueId`s would clone the field
            // vector without alias-local metadata; path identity lives on `values_by_path`.
            if self.values.get(target.index()).is_none() {
                return Err(ConstValueStoreError::from(CompilerError::compiler_error(
                    "ConstValueStore alias target was not allocated in the value graph.",
                )));
            }
            return Ok(target);
        }

        let (payload, value_kind, hir_visible, provenance) = match &expression.kind {
            ExpressionKind::Int(value) => (
                ConstValuePayload::Int(*value),
                ConstValueKind::Literal,
                true,
                None,
            ),
            ExpressionKind::Float(value) => (
                ConstValuePayload::Float(*value),
                ConstValueKind::Literal,
                true,
                None,
            ),
            ExpressionKind::Bool(value) => (
                ConstValuePayload::Bool(*value),
                ConstValueKind::Literal,
                true,
                None,
            ),
            ExpressionKind::Char(value) => (
                ConstValuePayload::Char(*value),
                ConstValueKind::Literal,
                true,
                None,
            ),
            ExpressionKind::StringSlice(value) => (
                ConstValuePayload::String(ConstStringValue::Text(*value)),
                ConstValueKind::Literal,
                true,
                None,
            ),
            ExpressionKind::StructuralString { pieces } => (
                ConstValuePayload::String(ConstStringValue::Pieces(pieces.clone())),
                ConstValueKind::Literal,
                true,
                None,
            ),
            ExpressionKind::Collection(items) => {
                let values = items
                    .iter()
                    .map(|item| {
                        self.insert_expression(item, None, type_environment, template_builder)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                (
                    ConstValuePayload::Collection(values),
                    ConstValueKind::Composite,
                    true,
                    None,
                )
            }
            ExpressionKind::StructInstance(fields) => {
                let stored_fields =
                    self.insert_record_fields(fields, type_environment, template_builder)?;
                (
                    ConstValuePayload::Record(stored_fields),
                    ConstValueKind::Composite,
                    true,
                    None,
                )
            }
            ExpressionKind::AnonymousConstRecord { fields } => {
                let stored_fields =
                    self.insert_record_fields(fields, type_environment, template_builder)?;
                (
                    ConstValuePayload::Record(stored_fields),
                    ConstValueKind::Composite,
                    false,
                    None,
                )
            }
            ExpressionKind::ChoiceConstruct {
                nominal_path,
                tag,
                fields,
            } => {
                let fields = fields
                    .iter()
                    .map(|field| {
                        Ok(ConstValueField {
                            name: field.id.clone(),
                            value: self.insert_expression(
                                &field.value,
                                None,
                                type_environment,
                                template_builder,
                            )?,
                            location: field.value.location.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>, ConstValueStoreError>>()?;
                (
                    ConstValuePayload::Choice {
                        nominal_path: nominal_path.clone(),
                        tag: *tag,
                        fields,
                    },
                    ConstValueKind::Composite,
                    true,
                    None,
                )
            }
            ExpressionKind::Range(start, end) => {
                let start =
                    self.insert_expression(start, None, type_environment, template_builder)?;
                let end = self.insert_expression(end, None, type_environment, template_builder)?;
                (
                    ConstValuePayload::Range { start, end },
                    ConstValueKind::Composite,
                    true,
                    None,
                )
            }
            ExpressionKind::Coerced { value, to_type } => {
                let value =
                    self.insert_expression(value, None, type_environment, template_builder)?;
                let inner_type = self
                    .metadata(value)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "ConstValueStore coercion child was not allocated in the value graph.",
                        )
                    })?
                    .type_id;
                let payload = if type_environment.option_inner_type(*to_type) == Some(inner_type) {
                    ConstValuePayload::OptionSome(value)
                } else {
                    ConstValuePayload::Coerced(value)
                };
                (payload, ConstValueKind::Composite, true, None)
            }
            ExpressionKind::OptionNone => (
                ConstValuePayload::OptionNone,
                ConstValueKind::Literal,
                true,
                None,
            ),
            ExpressionKind::Template(template) => {
                let result = template_builder(defining_path, template)?;
                match result {
                    ConstTemplateValue::Folded { value, provenance } => (
                        ConstValuePayload::String(value),
                        ConstValueKind::RenderableTemplate,
                        true,
                        Some(provenance),
                    ),
                    ConstTemplateValue::Public {
                        template,
                        kind,
                        hir_visible,
                        folded,
                        provenance,
                    } => (
                        ConstValuePayload::Template { template, folded },
                        kind,
                        hir_visible,
                        Some(provenance),
                    ),
                }
            }
            kind => {
                return Err(CompilerError::compiler_error(format!(
                    "module constant {:?} reached ConstValueStore without a folded value",
                    kind
                ))
                .into());
            }
        };

        let mut metadata = ConstValueMetadata {
            type_id: expression.type_id,
            diagnostic_type: expression.diagnostic_type.clone(),
            value_mode: expression.value_mode.clone(),
            location: expression.location.clone(),
            reactive_source: expression.reactive_source.clone(),
            reactive_template: expression.reactive_template.clone(),
            const_record_state: expression.const_record_state,
            contains_regular_division: expression.contains_regular_division,
            synthetic_interface_provenance: expression.synthetic_interface_provenance.clone(),
            value_kind,
            hir_visible,
        };
        if let Some(provenance) = provenance {
            metadata.synthetic_interface_provenance =
                metadata.synthetic_interface_provenance.union(&provenance);
        }

        let value = ConstValue { metadata, payload };
        let id = ConstValueId(self.values.len() as u32);
        self.values.push(value);
        Ok(id)
    }

    pub(crate) fn value(&self, id: ConstValueId) -> Option<&ConstValue> {
        self.values.get(id.index())
    }

    pub(crate) fn metadata(&self, id: ConstValueId) -> Option<&ConstValueMetadata> {
        self.value(id).map(|value| &value.metadata)
    }

    /// The semantic type of one field's folded value node.
    ///
    /// Field values are pushed into the graph before the field that references them, so a
    /// miss is a construction invariant violation rather than a lookup miss.
    fn field_value_type_id(&self, id: ConstValueId) -> Result<TypeId, CompilerError> {
        self.value(id)
            .map(|value| value.metadata.type_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "ConstValueStore field value id {:?} is outside the module value graph.",
                    id
                ))
            })
    }

    pub(crate) fn payload(&self, id: ConstValueId) -> Option<&ConstValuePayload> {
        self.value(id).map(|value| &value.payload)
    }

    pub(crate) fn value_for_path(&self, path: &InternedPath) -> Option<ConstValueId> {
        self.values_by_path.get(path).copied()
    }

    /// Every path binding in the store, including body-local const records.
    pub(crate) fn path_value_bindings(
        &self,
    ) -> impl Iterator<Item = (&InternedPath, ConstValueId)> {
        self.values_by_path.iter().map(|(path, id)| (path, *id))
    }

    /// Iterate module-constant rows as complete borrowed views.
    ///
    /// WHAT: yields each row's path, id and metadata together.
    /// WHY: a row's id was minted by this store and its value node was pushed before the row,
    /// so the metadata always exists. Handing out a bare id forces every consumer to look the
    /// metadata up again and invent a meaning for a miss that cannot happen - callers variously
    /// skipped the row, reported it as a user config error, or read it as "not HIR visible".
    /// Yielding the view removes the lookup and those branches with it.
    pub(crate) fn iter_module_constant_views(&self) -> impl Iterator<Item = ConstValueRowView<'_>> {
        self.rows.iter().map(|row| ConstValueRowView {
            path: &row.path,
            id: row.value,
            metadata: &self.values[row.value.index()].metadata,
        })
    }

    pub(crate) fn module_constant_paths(&self) -> impl Iterator<Item = &InternedPath> {
        self.rows.iter().map(|row| &row.path)
    }

    pub(crate) fn field_value(&self, id: ConstValueId, field: StringId) -> Option<ConstValueId> {
        let fields = match self.payload(id)? {
            ConstValuePayload::Record(fields) | ConstValuePayload::Choice { fields, .. } => fields,
            _ => return None,
        };
        fields
            .iter()
            .find(|entry| entry.name.name() == Some(field))
            .map(|entry| entry.value)
    }

    /// The text-only string accessor.
    ///
    /// Piece-bearing strings have no final text until the builder resolves each piece's URL
    /// context, so the accessor reports none instead of flattening structure.
    pub(crate) fn string_value(&self, id: ConstValueId) -> Option<StringId> {
        match self.payload(id)? {
            ConstValuePayload::String(ConstStringValue::Text(value)) => Some(*value),
            ConstValuePayload::Template {
                folded: Some(ConstStringValue::Text(value)),
                ..
            } => Some(*value),
            ConstValuePayload::Coerced(value) => self.string_value(*value),
            _ => None,
        }
    }

    /// Map one value tree through the common recursive visitor.
    pub(crate) fn fold_value<T>(
        &self,
        id: ConstValueId,
        visitor: &mut impl FnMut(
            &ConstValueMetadata,
            ConstValueVisit<'_, T>,
        ) -> Result<T, CompilerError>,
    ) -> Result<T, CompilerError> {
        let value = self.value(id).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "ConstValueStore value id {:?} is outside the module value graph.",
                id
            ))
        })?;

        match &value.payload {
            ConstValuePayload::Int(scalar) => {
                visitor(&value.metadata, ConstValueVisit::Int(*scalar))
            }
            ConstValuePayload::Float(scalar) => {
                visitor(&value.metadata, ConstValueVisit::Float(*scalar))
            }
            ConstValuePayload::Bool(scalar) => {
                visitor(&value.metadata, ConstValueVisit::Bool(*scalar))
            }
            ConstValuePayload::Char(scalar) => {
                visitor(&value.metadata, ConstValueVisit::Char(*scalar))
            }
            ConstValuePayload::String(string) => {
                visitor(&value.metadata, ConstValueVisit::String(string))
            }
            ConstValuePayload::Collection(items) => {
                let mapped = items
                    .iter()
                    .copied()
                    .map(|child| self.fold_value(child, visitor))
                    .collect::<Result<Vec<_>, _>>()?;
                visitor(&value.metadata, ConstValueVisit::Collection(mapped))
            }
            ConstValuePayload::Record(fields) => {
                let mapped = fields
                    .iter()
                    .map(|field| {
                        Ok(ConstValueFieldVisit {
                            name: &field.name,
                            type_id: self.field_value_type_id(field.value)?,
                            value: self.fold_value(field.value, visitor)?,
                        })
                    })
                    .collect::<Result<Vec<_>, CompilerError>>()?;
                visitor(&value.metadata, ConstValueVisit::Record(mapped))
            }
            ConstValuePayload::Choice {
                nominal_path,
                tag,
                fields,
            } => {
                let mapped = fields
                    .iter()
                    .map(|field| {
                        Ok(ConstValueFieldVisit {
                            name: &field.name,
                            type_id: self.field_value_type_id(field.value)?,
                            value: self.fold_value(field.value, visitor)?,
                        })
                    })
                    .collect::<Result<Vec<_>, CompilerError>>()?;
                visitor(
                    &value.metadata,
                    ConstValueVisit::Choice {
                        nominal_path,
                        tag: *tag,
                        fields: mapped,
                    },
                )
            }
            ConstValuePayload::Range { start, end } => {
                let mapped_start = self.fold_value(*start, visitor)?;
                let mapped_end = self.fold_value(*end, visitor)?;
                visitor(
                    &value.metadata,
                    ConstValueVisit::Range {
                        start: mapped_start,
                        end: mapped_end,
                    },
                )
            }
            ConstValuePayload::Coerced(child) => {
                let mapped = self.fold_value(*child, visitor)?;
                visitor(&value.metadata, ConstValueVisit::Coerced(mapped))
            }
            ConstValuePayload::OptionSome(child) => {
                let mapped = self.fold_value(*child, visitor)?;
                visitor(&value.metadata, ConstValueVisit::OptionSome(mapped))
            }
            ConstValuePayload::OptionNone => visitor(&value.metadata, ConstValueVisit::OptionNone),
            ConstValuePayload::Template { template, folded } => visitor(
                &value.metadata,
                ConstValueVisit::Template {
                    template,
                    folded: folded.as_ref(),
                },
            ),
        }
    }

    /// Return a temporary expression for advisory body-local resolution.
    ///
    /// This is deliberately not used as the module constant representation.  It exists only so
    /// the separate `AstConstFacts` resolver can substitute an authored module constant into a
    /// body-local RPN expression without making the fact collector a second folded-value owner.
    pub(crate) fn expression_for_resolution(
        &self,
        id: ConstValueId,
    ) -> Result<Expression, CompilerError> {
        self.expression_for_store_value(id, &mut |_, _| {
            Err(CompilerError::compiler_error(
                "A wrapper or slot-insert template cannot be materialized for advisory expression resolution.",
            ))
        })
    }

    /// Rebuild one temporary expression tree for a generated environment boundary.
    ///
    /// The caller supplies the only TIR-dependent step: converting an owned public template
    /// projection into a fresh generated-module template. All scalar and aggregate recursion
    /// remains owned by this store visitor.
    pub(crate) fn expression_for_materialisation(
        &self,
        id: ConstValueId,
        template_builder: &mut impl FnMut(
            &PublicConstTemplate,
            &ConstValueMetadata,
        ) -> Result<ExpressionKind, CompilerError>,
    ) -> Result<Expression, CompilerError> {
        self.expression_for_store_value(id, template_builder)
    }

    fn expression_for_store_value(
        &self,
        id: ConstValueId,
        template_builder: &mut impl FnMut(
            &PublicConstTemplate,
            &ConstValueMetadata,
        ) -> Result<ExpressionKind, CompilerError>,
    ) -> Result<Expression, CompilerError> {
        let value = self.value(id).ok_or_else(|| {
            CompilerError::compiler_error(
                "ConstValueStore advisory expression lookup missed a value.",
            )
        })?;
        let kind = match &value.payload {
            ConstValuePayload::Int(value) => ExpressionKind::Int(*value),
            ConstValuePayload::Float(value) => ExpressionKind::Float(*value),
            ConstValuePayload::Bool(value) => ExpressionKind::Bool(*value),
            ConstValuePayload::Char(value) => ExpressionKind::Char(*value),
            ConstValuePayload::String(string) => match string {
                ConstStringValue::Text(value) => ExpressionKind::StringSlice(*value),
                ConstStringValue::Pieces(pieces) => ExpressionKind::StructuralString {
                    pieces: pieces.clone(),
                },
            },
            ConstValuePayload::Collection(items) => ExpressionKind::Collection(
                items
                    .iter()
                    .map(|item| self.expression_for_store_value(*item, template_builder))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            ConstValuePayload::Record(fields) => {
                let projected = fields
                    .iter()
                    .map(|field| {
                        Ok(Declaration {
                            id: field.name.clone(),
                            value: self
                                .expression_for_store_value(field.value, template_builder)?,
                        })
                    })
                    .collect::<Result<Vec<_>, CompilerError>>()?;
                if value.metadata.hir_visible {
                    ExpressionKind::StructInstance(projected)
                } else {
                    ExpressionKind::AnonymousConstRecord { fields: projected }
                }
            }
            ConstValuePayload::Choice {
                nominal_path,
                tag,
                fields,
            } => ExpressionKind::ChoiceConstruct {
                nominal_path: nominal_path.clone(),
                tag: *tag,
                fields: fields
                    .iter()
                    .map(|field| {
                        Ok(Declaration {
                            id: field.name.clone(),
                            value: self
                                .expression_for_store_value(field.value, template_builder)?,
                        })
                    })
                    .collect::<Result<Vec<_>, CompilerError>>()?,
            },
            ConstValuePayload::Range { start, end } => ExpressionKind::Range(
                Box::new(self.expression_for_store_value(*start, template_builder)?),
                Box::new(self.expression_for_store_value(*end, template_builder)?),
            ),
            ConstValuePayload::Coerced(child) => ExpressionKind::Coerced {
                value: Box::new(self.expression_for_store_value(*child, template_builder)?),
                to_type: value.metadata.type_id,
            },
            ConstValuePayload::OptionSome(child) => ExpressionKind::Coerced {
                value: Box::new(self.expression_for_store_value(*child, template_builder)?),
                to_type: value.metadata.type_id,
            },
            ConstValuePayload::OptionNone => ExpressionKind::OptionNone,
            ConstValuePayload::Template { template, .. } => {
                template_builder(template, &value.metadata)?
            }
        };

        let mut expression = Expression::new(
            kind,
            value.metadata.location.clone(),
            value.metadata.type_id,
            value.metadata.diagnostic_type.clone(),
            value.metadata.value_mode.clone(),
        );
        expression.reactive_source = value.metadata.reactive_source.clone();
        expression.reactive_template = value.metadata.reactive_template.clone();
        expression.const_record_state = value.metadata.const_record_state;
        expression.contains_regular_division = value.metadata.contains_regular_division;
        expression.synthetic_interface_provenance =
            value.metadata.synthetic_interface_provenance.clone();
        Ok(expression)
    }

    /// Validate every stored node's semantic type against the final module environment.
    pub(crate) fn validate_type_ids(
        &self,
        type_environment: &TypeEnvironment,
    ) -> Result<(), CompilerError> {
        for value in &self.values {
            if type_environment.get(value.metadata.type_id).is_none() {
                return Err(CompilerError::compiler_error(format!(
                    "ConstValueStore contains unresolved TypeId({}).",
                    value.metadata.type_id.0
                )));
            }
        }
        Ok(())
    }
}
