//! Module-local folded values.
//!
//! WHAT: owns the compact value graph and module-constant rows produced for one AST module after
//! constant evaluation. The graph is the only folded-value representation retained across the
//! AST finalisation boundary. Public projection and HIR use its borrowed postorder visitor.
//! WHY: stable scalar and aggregate facts belong in one indexed store so each consumer reads
//! the same resolved values without reconstructing declaration trees.
//!
//! The store is module-local. Its IDs and rows must never enter a cross-module
//! interface. Public projection converts them to [`PublicFoldedValue`] before publication.

use crate::compiler_frontend::ast::expressions::expression_store::{
    CoercionOperation, ConstructorFieldView, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    ConstRecordState, ConstValueKind,
};
use crate::compiler_frontend::ast::module_ast::environment::declaration_table::{
    ResolvedConstantSet, TopLevelDeclarationTable,
};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::compiler_errors::{CompilerError, InfrastructureErrors};
use crate::compiler_frontend::compiler_messages::{AstCapacityResource, CompilerDiagnostic};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::number::NumberValue;
use crate::compiler_frontend::folded_value::PublicConstTemplate;
use crate::compiler_frontend::paths::module_resources::ResourceId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::FixedScalarValue;
use rustc_hash::FxHashMap;

#[cfg(test)]
#[path = "tests/store_tests.rs"]
mod tests;

fn checked_const_value_index(
    prefix: usize,
    offset: usize,
    span: Option<SourceSpan>,
) -> Result<u32, CompilerDiagnostic> {
    prefix
        .checked_add(offset)
        .and_then(|index| u32::try_from(index).ok())
        .ok_or_else(|| {
            CompilerDiagnostic::compiler_capacity_exceeded(AstCapacityResource::ConstValues, span)
        })
}

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
/// joins a module constant by its exact defining `PathId`, so the path is the row key.
#[derive(Clone, Debug)]
struct ConstValueRow {
    path: PathId,
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
    Diagnostic(CompilerDiagnostic),
    Infrastructure(Box<InfrastructureErrors>),
}

impl From<CompilerDiagnostic> for ConstValueStoreError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        Self::Diagnostic(diagnostic)
    }
}

impl From<CompilerError> for ConstValueStoreError {
    fn from(error: CompilerError) -> Self {
        Self::Infrastructure(error.into())
    }
}

impl From<TemplateError> for ConstValueStoreError {
    fn from(error: TemplateError) -> Self {
        match error {
            TemplateError::Diagnostic(diagnostic) => Self::Diagnostic(diagnostic),
            TemplateError::Infrastructure(error) => Self::Infrastructure(error),
        }
    }
}

/// Canonical content metadata used by owned projection and HIR.
///
/// Occurrence-specific failure and access facts remain on expression rows.
#[derive(Clone, Debug)]
pub(crate) struct ConstValueMetadata {
    pub(crate) type_id: TypeId,
    pub(crate) value_mode: ValueMode,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) const_record_state: ConstRecordState,
    pub(crate) contains_regular_division: bool,
    pub(crate) synthetic_interface_provenance: SyntheticInterfaceProvenance,
    pub(crate) value_kind: ConstValueKind,
    pub(crate) hir_visible: bool,
}

impl ConstValueMetadata {
    pub(crate) fn resolved(
        type_id: TypeId,
        span: Option<SourceSpan>,
        value_mode: ValueMode,
    ) -> Self {
        Self {
            type_id,
            span,
            value_mode,
            const_record_state: ConstRecordState::RuntimeValue,
            contains_regular_division: false,
            synthetic_interface_provenance: SyntheticInterfaceProvenance::empty(),
            value_kind: ConstValueKind::Literal,
            hir_visible: true,
        }
    }

    /// Metadata for one use of a shared constant. Returns `None` when the use adds no provenance
    /// members, so the stored value is reused unchanged.
    ///
    /// WHY: one stored constant serves every use, but each use's synthetic provenance must still
    /// reach the value it resolves to, as a folded result did in the baseline.
    pub(crate) fn with_occurrence_provenance(
        &self,
        occurrence: &SyntheticInterfaceProvenance,
    ) -> Option<Self> {
        if occurrence.is_subset_of(&self.synthetic_interface_provenance) {
            return None;
        }
        let mut metadata = self.clone();
        metadata.synthetic_interface_provenance =
            self.synthetic_interface_provenance.union(occurrence);
        Some(metadata)
    }
}

/// A named field in a folded record or choice payload.
///
/// WHAT: keeps authored field order and the folded value id.
///
/// Public folded values remain location-free; exact source spans are retained on the value
/// metadata where they are available.
#[derive(Clone, Debug)]
pub(crate) struct ConstValueField {
    pub(crate) name: PathId,
    pub(crate) value: ConstValueId,
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
    /// One exact arbitrary-precision decimal value, retaining its canonical scale.
    Number(NumberValue),
    /// One materialised fixed-width scalar or `Byte` value with exact-bit identity.
    FixedScalar(FixedScalarValue),
    Bool(bool),
    Char(char),
    String(ConstStringValue),
    Collection(Vec<ConstValueId>),
    Record(Vec<ConstValueField>),
    Choice {
        nominal_path: PathId,
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
    /// Borrowed exact decimal leaf; visitors clone only when ownership crosses their boundary.
    Number(&'a NumberValue),
    FixedScalar(FixedScalarValue),
    Bool(bool),
    Char(char),
    String(&'a ConstStringValue),
    Collection(Vec<T>),
    Record(Vec<ConstValueFieldVisit<'a, T>>),
    Choice {
        nominal_path: &'a PathId,
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
    pub(crate) path: &'a PathId,
    pub(crate) id: ConstValueId,
    pub(crate) metadata: &'a ConstValueMetadata,
}

/// One folded field visited with the semantic type of its value node.
///
/// WHAT: pairs the authored field name with the value's store metadata `TypeId` so public
/// projection can project each field's canonical identity without a second lookup.
pub(crate) struct ConstValueFieldVisit<'a, T> {
    pub(crate) name: &'a PathId,
    pub(crate) type_id: TypeId,
    pub(crate) value: T,
}

/// The one module-local folded-value authority.
#[derive(Clone, Debug, Default)]
pub(crate) struct ConstValueStore {
    values: Vec<ConstValue>,
    rows: Vec<ConstValueRow>,
    values_by_path: FxHashMap<PathId, ConstValueId>,
}

/// Producer-local content scratch. Its payload vocabulary is the ordinary constant graph.
///
/// Temporary IDs address this borrowed prefix plus staged suffix only inside `stage_values`.
/// No staged payload is installed in expressions, defaults or module bindings.
pub(crate) struct ConstValueCandidate<'a> {
    prefix: &'a ConstValueStore,
    values: Vec<ConstValue>,
}

impl ConstValueCandidate<'_> {
    pub(crate) fn value(&self, id: ConstValueId) -> Option<&ConstValue> {
        if id.index() < self.prefix.values.len() {
            self.prefix.value(id)
        } else {
            self.values.get(id.index() - self.prefix.values.len())
        }
    }

    pub(crate) fn metadata(&self, id: ConstValueId) -> Option<&ConstValueMetadata> {
        self.value(id).map(|value| &value.metadata)
    }

    pub(crate) fn payload(&self, id: ConstValueId) -> Option<&ConstValuePayload> {
        self.value(id).map(|value| &value.payload)
    }

    pub(crate) fn insert_value(
        &mut self,
        metadata: ConstValueMetadata,
        payload: ConstValuePayload,
    ) -> Result<ConstValueId, ConstValueStoreError> {
        validate_payload_children(&payload, |id| self.value(id).is_some())?;
        let index =
            checked_const_value_index(self.prefix.values.len(), self.values.len(), metadata.span)?;
        let id = ConstValueId(index);
        self.values.push(ConstValue { metadata, payload });
        Ok(id)
    }

    /// Attach one use's occurrence provenance to a staged shared constant. See
    /// [`ConstValueStore::with_occurrence_provenance`].
    pub(crate) fn with_occurrence_provenance(
        &mut self,
        expression: ExpressionView<'_>,
        id: ConstValueId,
    ) -> Result<ConstValueId, ConstValueStoreError> {
        let Some(stored) = self.value(id) else {
            return Err(
                CompilerError::compiler_error("Shared constant lacks a stored value.").into(),
            );
        };
        let Some((metadata, payload)) = occurrence_copy(stored, expression) else {
            return Ok(id);
        };
        self.insert_value(metadata, payload)
    }
}

/// Stage the copy one use of a shared constant needs, or `None` when the use adds no provenance.
///
/// WHAT: the copy carries the use's provenance and the shared payload. The stored value itself is
/// never mutated.
/// WHY: one stored constant serves every use, but each use's synthetic provenance must still reach
/// the value it resolves to, as a folded result did in the baseline.
fn occurrence_copy(
    stored: &ConstValue,
    expression: ExpressionView<'_>,
) -> Option<(ConstValueMetadata, ConstValuePayload)> {
    let facts = expression.occurrence_facts()?;
    let metadata = stored
        .metadata
        .with_occurrence_provenance(&facts.provenance)?;
    Some((metadata, stored.payload.clone()))
}

fn visit_payload_children(payload: &ConstValuePayload, mut visitor: impl FnMut(ConstValueId)) {
    match payload {
        ConstValuePayload::Collection(values) => {
            for &value in values {
                visitor(value);
            }
        }
        ConstValuePayload::Record(fields) | ConstValuePayload::Choice { fields, .. } => {
            for field in fields {
                visitor(field.value);
            }
        }
        ConstValuePayload::Range { start, end } => {
            visitor(*start);
            visitor(*end);
        }
        ConstValuePayload::Coerced(value) | ConstValuePayload::OptionSome(value) => visitor(*value),
        _ => {}
    }
}

fn validate_payload_children(
    payload: &ConstValuePayload,
    exists: impl Fn(ConstValueId) -> bool,
) -> Result<(), CompilerError> {
    let mut missing = false;
    visit_payload_children(payload, |id| missing |= !exists(id));
    if missing {
        return Err(CompilerError::compiler_error(
            "Constant child is outside its content owner.",
        ));
    }
    if let ConstValuePayload::Record(fields) | ConstValuePayload::Choice { fields, .. } = payload {
        let mut names = rustc_hash::FxHashSet::default();
        for field in fields {
            if !names.insert(field.name) {
                return Err(CompilerError::compiler_error(
                    "Duplicate folded record field.",
                ));
            }
        }
    }
    Ok(())
}

fn remap_payload_children(
    payload: &mut ConstValuePayload,
    mut remap: impl FnMut(&mut ConstValueId),
) {
    match payload {
        ConstValuePayload::Collection(values) => {
            for value in values {
                remap(value);
            }
        }
        ConstValuePayload::Record(fields) | ConstValuePayload::Choice { fields, .. } => {
            for field in fields {
                remap(&mut field.value);
            }
        }
        ConstValuePayload::Range { start, end } => {
            remap(start);
            remap(end);
        }
        ConstValuePayload::Coerced(value) | ConstValuePayload::OptionSome(value) => remap(value),
        _ => {}
    }
}

/// Depth-dependent work for the postorder value visitor.
///
/// Finishes borrow their immutable node rather than copying payload or type facts. Field
/// boundaries defer validation until earlier siblings have completed their callbacks.
enum FoldFrame<'a> {
    Visit(ConstValueId),
    Field(ConstValueId),
    Finish {
        value: &'a ConstValue,
        result_start: usize,
    },
}

/// The missing-id error every folded-value lookup queue reports.
fn fold_value_missing_error<E>(id: ConstValueId) -> E
where
    E: From<CompilerError>,
{
    E::from(CompilerError::compiler_error(format!(
        "ConstValueStore value id {:?} is outside the module value graph.",
        id
    )))
}

fn fold_stack_error<E: From<CompilerError>>() -> E {
    CompilerError::compiler_error("Folded-value traversal lost a child result.").into()
}

/// Work item for the iterative constant ingestion walk.
enum IngestFrame<'a> {
    /// Ingest one expression node. Leaves complete at once. Composites queue `Finish` first.
    Visit(ExpressionView<'a>),
    /// Build one composite row once its children are complete on the result stack.
    Finish {
        expression: ExpressionView<'a>,
        pending: IngestPending,
    },
}

/// Node data captured before its children run and consumed once they are complete.
enum IngestPending {
    Collection,
    Record,
    Choice { nominal_path: PathId, tag: u32 },
    Range,
    Coerce { target: TypeId, inner_type: TypeId },
}

/// Push a finish frame, then the children so that the first child is visited first.
fn queue_children<'a>(
    work: &mut Vec<IngestFrame<'a>>,
    finish: IngestFrame<'a>,
    children: impl Iterator<Item = ExpressionView<'a>>,
) {
    let start = work.len();
    work.push(finish);
    work.extend(children.map(IngestFrame::Visit));
    work[start + 1..].reverse();
}

fn ingest_invariant(message: &'static str) -> ConstValueStoreError {
    CompilerError::compiler_error(message).into()
}

/// Index of the first of the `count` completed child values at the top of `results`.
fn child_start(results: &[ConstValueId], count: usize) -> Result<usize, ConstValueStoreError> {
    results
        .len()
        .checked_sub(count)
        .ok_or_else(|| ingest_invariant("Constant ingestion lost a child value."))
}

fn pop_child(results: &mut Vec<ConstValueId>) -> Result<ConstValueId, ConstValueStoreError> {
    results
        .pop()
        .ok_or_else(|| ingest_invariant("Constant ingestion lost a child value."))
}

/// Build named fields from the top `fields.len()` completed child values, in source order.
fn take_fields<'a>(
    results: &mut Vec<ConstValueId>,
    fields: impl ExactSizeIterator<Item = ConstructorFieldView<'a>>,
) -> Result<Vec<ConstValueField>, ConstValueStoreError> {
    let start = child_start(results, fields.len())?;
    Ok(results
        .drain(start..)
        .zip(fields)
        .map(|(value, field)| ConstValueField {
            name: field.name(),
            value,
        })
        .collect())
}

impl ConstValueStore {
    /// Stage ordinary content and move only the selected root's reachable suffix on acceptance.
    pub(crate) fn stage_values<E>(
        &mut self,
        produce: impl FnOnce(&mut ConstValueCandidate<'_>) -> Result<Option<ConstValueId>, E>,
    ) -> Result<Option<ConstValueId>, E>
    where
        E: From<ConstValueStoreError>,
    {
        let prefix_len = self.values.len();
        let (root, values) = {
            let mut candidate = ConstValueCandidate {
                prefix: self,
                values: Vec::new(),
            };
            let Some(root) = produce(&mut candidate)? else {
                return Ok(None);
            };
            if candidate.value(root).is_none() {
                return Err(E::from(
                    CompilerError::compiler_error(
                        "Selected constant root is outside its candidate.",
                    )
                    .into(),
                ));
            }
            (root, candidate.values)
        };
        if root.index() < prefix_len {
            return Ok(Some(root));
        }

        // Every staged edge is backwards: append validates against the already issued prefix.
        // Reachability does not depend on visit order, so an explicit stack marks the same rows.
        let mut live = vec![false; values.len()];
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            if id.index() < prefix_len {
                continue;
            }
            let index = id.index() - prefix_len;
            if live[index] {
                continue;
            }
            live[index] = true;
            visit_payload_children(&values[index].payload, |child| {
                if child.index() >= prefix_len {
                    pending.push(child);
                }
            });
        }
        let live_count = live.iter().filter(|&&live| live).count();
        checked_const_value_index(
            prefix_len,
            live_count,
            values[root.index() - prefix_len].metadata.span,
        )
        .map_err(|error| E::from(ConstValueStoreError::from(error)))?;
        self.values.reserve(live_count);
        let mut remapped = vec![None; values.len()];
        for (index, mut value) in values.into_iter().enumerate() {
            if !live[index] {
                continue;
            }
            remap_payload_children(&mut value.payload, |child| {
                if child.index() >= prefix_len {
                    *child = remapped[child.index() - prefix_len]
                        .expect("validated backwards edge has a live promoted child");
                }
            });
            let id = ConstValueId(self.values.len() as u32);
            self.values.push(value);
            remapped[index] = Some(id);
        }
        Ok(Some(
            remapped[root.index() - prefix_len].expect("selected staged root is marked reachable"),
        ))
    }

    /// Bind every resolved module constant to its header-bound value and record its row.
    ///
    /// WHY: header resolution already folded each module constant into this graph, so
    /// finalization only assigns declaration-order rows. A missing binding is an invariant
    /// violation, never a reason to fold the initializer again.
    pub(crate) fn populate_from_declaration_table(
        &mut self,
        declaration_table: &TopLevelDeclarationTable,
        resolved_module_constants: &ResolvedConstantSet,
    ) -> Result<(), ConstValueStoreError> {
        for declaration_id in resolved_module_constants.iter() {
            let declaration = declaration_table.get_by_id(declaration_id).ok_or_else(|| {
                CompilerError::compiler_error("Resolved constant has no declaration row.")
            })?;
            let value = self.value_for_path(&declaration.id).ok_or_else(|| {
                CompilerError::compiler_error("Resolved constant has no header-bound value.")
            })?;
            self.bind_module_constant(declaration.id, value)?;
        }
        Ok(())
    }

    pub(crate) fn bind_module_constant(
        &mut self,
        path: PathId,
        value: ConstValueId,
    ) -> Result<(), ConstValueStoreError> {
        self.bind_body_local(path, value)?;
        if let Some(row) = self.rows.iter().find(|row| row.path == path) {
            if row.value != value {
                return Err(
                    CompilerError::compiler_error("Constant binding changed content.").into(),
                );
            }
        } else {
            self.rows.push(ConstValueRow { path, value });
        }
        Ok(())
    }

    pub(crate) fn bind_body_local(
        &mut self,
        path: PathId,
        value: ConstValueId,
    ) -> Result<(), ConstValueStoreError> {
        self.require_value(value)?;
        if let Some(previous) = self.values_by_path.get(&path) {
            if *previous != value {
                return Err(
                    CompilerError::compiler_error("Duplicate constant path binding.").into(),
                );
            }
        } else {
            self.values_by_path.insert(path, value);
        }
        Ok(())
    }

    fn require_value(&self, value: ConstValueId) -> Result<(), CompilerError> {
        if self.value(value).is_none() {
            return Err(CompilerError::compiler_error(
                "Constant child is outside its content owner.",
            ));
        }
        Ok(())
    }

    /// Insert resolved content. Children must already belong to this content graph.
    pub(crate) fn insert_value(
        &mut self,
        metadata: ConstValueMetadata,
        payload: ConstValuePayload,
    ) -> Result<ConstValueId, ConstValueStoreError> {
        validate_payload_children(&payload, |id| self.value(id).is_some())?;
        let index = checked_const_value_index(self.values.len(), 0, metadata.span)?;
        let id = ConstValueId(index);
        self.values.push(ConstValue { metadata, payload });
        Ok(id)
    }

    pub(crate) fn insert_resolved(
        &mut self,
        type_id: TypeId,
        payload: ConstValuePayload,
        span: Option<SourceSpan>,
    ) -> Result<ConstValueId, ConstValueStoreError> {
        let value_kind = match &payload {
            ConstValuePayload::Collection(_)
            | ConstValuePayload::Record(_)
            | ConstValuePayload::Choice { .. }
            | ConstValuePayload::Range { .. }
            | ConstValuePayload::Coerced(_)
            | ConstValuePayload::OptionSome(_) => ConstValueKind::Composite,
            _ => ConstValueKind::Literal,
        };
        self.insert_value(
            ConstValueMetadata {
                type_id,
                value_mode: ValueMode::ImmutableOwned,
                span,
                const_record_state: ConstRecordState::RuntimeValue,
                contains_regular_division: false,
                synthetic_interface_provenance: SyntheticInterfaceProvenance::default(),
                value_kind,
                hir_visible: true,
            },
            payload,
        )
    }

    /// Attach one use's occurrence provenance to a shared constant. Stages a copy only when the
    /// occurrence adds members, so the stored value itself is never mutated.
    fn with_occurrence_provenance(
        &mut self,
        expression: ExpressionView<'_>,
        id: ConstValueId,
    ) -> Result<ConstValueId, ConstValueStoreError> {
        let Some(stored) = self.value(id) else {
            return Err(
                CompilerError::compiler_error("Shared constant lacks a stored value.").into(),
            );
        };
        let Some((metadata, payload)) = occurrence_copy(stored, expression) else {
            return Ok(id);
        };
        self.insert_value(metadata, payload)
    }

    /// Finalisation reads bound edges directly. Ordinary constant occurrences reuse content.
    ///
    /// WHY: composite nodes queue their children on an explicit stack, so ingestion depth is
    /// bounded by the heap rather than the thread stack. Children complete in source order, so
    /// rows are inserted in child-before-parent order.
    pub(crate) fn ingest_view(
        &mut self,
        expression: ExpressionView<'_>,
        type_environment: &TypeEnvironment,
        template_builder: &mut impl FnMut(&Template) -> Result<ConstTemplateValue, ConstValueStoreError>,
    ) -> Result<ConstValueId, ConstValueStoreError> {
        let mut work = Vec::new();
        if let Some(id) =
            self.ingest_visit(expression, type_environment, template_builder, &mut work)?
        {
            return Ok(id);
        }
        let mut results = Vec::new();
        while let Some(frame) = work.pop() {
            let id = match frame {
                IngestFrame::Visit(expression) => match self.ingest_visit(
                    expression,
                    type_environment,
                    template_builder,
                    &mut work,
                )? {
                    Some(id) => id,
                    None => continue,
                },
                IngestFrame::Finish {
                    expression,
                    pending,
                } => self.ingest_finish(expression, pending, type_environment, &mut results)?,
            };
            results.push(id);
        }
        match results.as_slice() {
            [root] => Ok(*root),
            _ => Err(ingest_invariant(
                "Constant ingestion did not produce one root value.",
            )),
        }
    }

    /// Ingest one expression node. Leaves and templates complete here and return their id.
    /// Composite nodes queue a `Finish` frame ahead of their children and return `None`.
    fn ingest_visit<'a>(
        &mut self,
        expression: ExpressionView<'a>,
        type_environment: &TypeEnvironment,
        template_builder: &mut impl FnMut(&Template) -> Result<ConstTemplateValue, ConstValueStoreError>,
        work: &mut Vec<IngestFrame<'a>>,
    ) -> Result<Option<ConstValueId>, ConstValueStoreError> {
        let type_id = expression.value_type_id()?;
        match expression.operation() {
            ExpressionOperationView::Constant(id) => {
                self.require_value(id)?;
                if self.metadata(id).map(|metadata| metadata.type_id) != Some(type_id) {
                    return Err(CompilerError::compiler_error(
                        "Constant occurrence has a foreign type.",
                    )
                    .into());
                }
                self.with_occurrence_provenance(expression, id).map(Some)
            }
            ExpressionOperationView::Reference(path) => {
                let Some(id) = self.value_for_path(&path) else {
                    return Err(CompilerError::compiler_error(
                        "Constant reference has no folded binding.",
                    )
                    .into());
                };
                self.with_occurrence_provenance(expression, id).map(Some)
            }
            ExpressionOperationView::Collection(items) => {
                queue_children(
                    work,
                    IngestFrame::Finish {
                        expression,
                        pending: IngestPending::Collection,
                    },
                    items.iter(),
                );
                Ok(None)
            }
            ExpressionOperationView::Struct { fields } => {
                queue_children(
                    work,
                    IngestFrame::Finish {
                        expression,
                        pending: IngestPending::Record,
                    },
                    fields.iter().map(|field| field.value()),
                );
                Ok(None)
            }
            ExpressionOperationView::Choice { tag, fields } => {
                let nominal_path = *type_environment.nominal_path(type_id).ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Folded choice has no canonical nominal identity.",
                    )
                })?;
                queue_children(
                    work,
                    IngestFrame::Finish {
                        expression,
                        pending: IngestPending::Choice { nominal_path, tag },
                    },
                    fields.iter().map(|field| field.value()),
                );
                Ok(None)
            }
            ExpressionOperationView::Range { start, end, .. } => {
                queue_children(
                    work,
                    IngestFrame::Finish {
                        expression,
                        pending: IngestPending::Range,
                    },
                    [start, end].into_iter(),
                );
                Ok(None)
            }
            ExpressionOperationView::Coerce {
                value,
                operation: CoercionOperation::ToType(target),
            } => {
                let inner_type = value.value_type_id()?;
                queue_children(
                    work,
                    IngestFrame::Finish {
                        expression,
                        pending: IngestPending::Coerce { target, inner_type },
                    },
                    [value].into_iter(),
                );
                Ok(None)
            }
            ExpressionOperationView::Template(template) => {
                let result = template_builder(template)?;
                let (payload, kind, visible, provenance) = match result {
                    ConstTemplateValue::Folded { value, provenance } => (
                        ConstValuePayload::String(value),
                        ConstValueKind::RenderableTemplate,
                        true,
                        provenance,
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
                        provenance,
                    ),
                };
                let mut metadata = Self::metadata_from_view(expression, kind)?;
                metadata.hir_visible = visible;
                metadata.synthetic_interface_provenance =
                    metadata.synthetic_interface_provenance.union(&provenance);
                self.insert_value(metadata, payload).map(Some)
            }
            _ => Err(CompilerError::compiler_error(
                "Unfolded operation reached constant ingestion.",
            )
            .into()),
        }
    }

    /// Build one composite row once its children are complete on top of `results`.
    fn ingest_finish(
        &mut self,
        expression: ExpressionView<'_>,
        pending: IngestPending,
        type_environment: &TypeEnvironment,
        results: &mut Vec<ConstValueId>,
    ) -> Result<ConstValueId, ConstValueStoreError> {
        let payload = match pending {
            IngestPending::Collection => {
                let ExpressionOperationView::Collection(items) = expression.operation() else {
                    return Err(ingest_invariant("Collection row lost its expression."));
                };
                let start = child_start(results, items.len())?;
                ConstValuePayload::Collection(results.drain(start..).collect())
            }
            IngestPending::Record => {
                let ExpressionOperationView::Struct { fields } = expression.operation() else {
                    return Err(ingest_invariant("Record row lost its expression."));
                };
                ConstValuePayload::Record(take_fields(results, fields.iter())?)
            }
            IngestPending::Choice { nominal_path, tag } => {
                let ExpressionOperationView::Choice { fields, .. } = expression.operation() else {
                    return Err(ingest_invariant("Choice row lost its expression."));
                };
                ConstValuePayload::Choice {
                    nominal_path,
                    tag: tag as usize,
                    fields: take_fields(results, fields.iter())?,
                }
            }
            IngestPending::Range => {
                let end = pop_child(results)?;
                let start = pop_child(results)?;
                ConstValuePayload::Range { start, end }
            }
            IngestPending::Coerce { target, inner_type } => {
                let value = pop_child(results)?;
                if type_environment.option_inner_type(target) == Some(inner_type) {
                    ConstValuePayload::OptionSome(value)
                } else {
                    ConstValuePayload::Coerced(value)
                }
            }
        };
        self.insert_value(
            Self::metadata_from_view(expression, ConstValueKind::Composite)?,
            payload,
        )
    }

    pub(crate) fn metadata_from_view(
        expression: ExpressionView<'_>,
        value_kind: ConstValueKind,
    ) -> Result<ConstValueMetadata, CompilerError> {
        Ok(ConstValueMetadata {
            type_id: expression.value_type_id()?,
            value_mode: expression.value_mode(),
            span: expression.span(),
            const_record_state: expression.classification().const_record_state,
            contains_regular_division: expression.classification().contains_regular_division,
            synthetic_interface_provenance: expression
                .occurrence_facts()
                .map(|facts| facts.provenance.clone())
                .unwrap_or_default(),
            value_kind,
            hir_visible: true,
        })
    }

    pub(crate) fn value(&self, id: ConstValueId) -> Option<&ConstValue> {
        self.values.get(id.index())
    }

    pub(crate) fn metadata(&self, id: ConstValueId) -> Option<&ConstValueMetadata> {
        self.value(id).map(|value| &value.metadata)
    }

    pub(crate) fn payload(&self, id: ConstValueId) -> Option<&ConstValuePayload> {
        self.value(id).map(|value| &value.payload)
    }

    pub(crate) fn value_for_path(&self, path: &PathId) -> Option<ConstValueId> {
        self.values_by_path.get(path).copied()
    }

    /// Every path binding in the store, including body-local const records.
    pub(crate) fn path_value_bindings(&self) -> impl Iterator<Item = (&PathId, ConstValueId)> {
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

    pub(crate) fn module_constant_paths(&self) -> impl Iterator<Item = &PathId> {
        self.rows.iter().map(|row| &row.path)
    }

    pub(crate) fn field_value(
        &self,
        id: ConstValueId,
        field: StringId,
        path_fork: &PathInternerFork,
    ) -> Option<ConstValueId> {
        let fields = match self.payload(id)? {
            ConstValuePayload::Record(fields) | ConstValuePayload::Choice { fields, .. } => fields,
            _ => return None,
        };
        fields
            .iter()
            .find(|entry| path_fork.component(entry.name) == Some(field))
            .map(|entry| entry.value)
    }

    /// The text-only string accessor.
    ///
    /// Piece-bearing strings have no final text until the builder resolves each piece's URL
    /// context, so the accessor reports none instead of flattening structure.
    #[cfg(test)]
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

    /// Map one value graph through the common postorder visitor.
    ///
    /// Depth-dependent work stays on the heap. Each edge produces its own `T`, including
    /// repeated children, and callbacks and field validation retain authored depth-first order.
    /// Leaf roots reach the callback before either scratch vector allocates.
    pub(crate) fn fold_value<T, E>(
        &self,
        id: ConstValueId,
        visitor: &mut impl FnMut(&ConstValueMetadata, ConstValueVisit<'_, T>) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<CompilerError>,
    {
        let mut work = Vec::new();
        let mut results: Vec<T> = Vec::new();
        let mut next = Some(FoldFrame::Visit(id));

        while let Some(frame) = next {
            let (value, visit) = match frame {
                FoldFrame::Finish {
                    value,
                    result_start,
                } => {
                    let visit = match &value.payload {
                        ConstValuePayload::Collection(items) => {
                            if results.len().checked_sub(result_start) != Some(items.len()) {
                                return Err(fold_stack_error::<E>());
                            }
                            ConstValueVisit::Collection(results.drain(result_start..).collect())
                        }
                        ConstValuePayload::Record(fields) => ConstValueVisit::Record(
                            self.drain_fold_fields::<T, E>(fields, &mut results, result_start)?,
                        ),
                        ConstValuePayload::Choice {
                            nominal_path,
                            tag,
                            fields,
                        } => ConstValueVisit::Choice {
                            nominal_path,
                            tag: *tag,
                            fields: self.drain_fold_fields::<T, E>(
                                fields,
                                &mut results,
                                result_start,
                            )?,
                        },
                        ConstValuePayload::Range { .. } => {
                            if results.len().checked_sub(result_start) != Some(2) {
                                return Err(fold_stack_error::<E>());
                            }
                            let end = results.pop().ok_or_else(fold_stack_error::<E>)?;
                            let start = results.pop().ok_or_else(fold_stack_error::<E>)?;
                            ConstValueVisit::Range { start, end }
                        }
                        ConstValuePayload::Coerced(_) => {
                            if results.len().checked_sub(result_start) != Some(1) {
                                return Err(fold_stack_error::<E>());
                            }
                            ConstValueVisit::Coerced(
                                results.pop().ok_or_else(fold_stack_error::<E>)?,
                            )
                        }
                        ConstValuePayload::OptionSome(_) => {
                            if results.len().checked_sub(result_start) != Some(1) {
                                return Err(fold_stack_error::<E>());
                            }
                            ConstValueVisit::OptionSome(
                                results.pop().ok_or_else(fold_stack_error::<E>)?,
                            )
                        }
                        ConstValuePayload::Number(_)
                        | ConstValuePayload::FixedScalar(_)
                        | ConstValuePayload::Bool(_)
                        | ConstValuePayload::Char(_)
                        | ConstValuePayload::String(_)
                        | ConstValuePayload::OptionNone
                        | ConstValuePayload::Template { .. } => {
                            return Err(fold_stack_error::<E>());
                        }
                    };
                    (value, visit)
                }
                FoldFrame::Visit(id) | FoldFrame::Field(id) => {
                    let value = self.value(id).ok_or_else(|| match frame {
                        FoldFrame::Field(_) => E::from(CompilerError::compiler_error(format!(
                            "ConstValueStore field value id {:?} is outside the module value graph.",
                            id
                        ))),
                        _ => fold_value_missing_error::<E>(id),
                    })?;
                    let result_start = results.len();
                    let visit = match &value.payload {
                        ConstValuePayload::Number(number) => ConstValueVisit::Number(number),
                        ConstValuePayload::FixedScalar(scalar) => {
                            ConstValueVisit::FixedScalar(*scalar)
                        }
                        ConstValuePayload::Bool(boolean) => ConstValueVisit::Bool(*boolean),
                        ConstValuePayload::Char(character) => ConstValueVisit::Char(*character),
                        ConstValuePayload::String(string) => ConstValueVisit::String(string),
                        ConstValuePayload::OptionNone => ConstValueVisit::OptionNone,
                        ConstValuePayload::Template { template, folded } => {
                            ConstValueVisit::Template {
                                template,
                                folded: folded.as_ref(),
                            }
                        }
                        ConstValuePayload::Collection(items) => {
                            work.push(FoldFrame::Finish {
                                value,
                                result_start,
                            });
                            work.extend(items.iter().rev().copied().map(FoldFrame::Visit));
                            next = work.pop();
                            continue;
                        }
                        ConstValuePayload::Record(fields)
                        | ConstValuePayload::Choice { fields, .. } => {
                            work.push(FoldFrame::Finish {
                                value,
                                result_start,
                            });
                            work.extend(
                                fields
                                    .iter()
                                    .rev()
                                    .map(|field| FoldFrame::Field(field.value)),
                            );
                            next = work.pop();
                            continue;
                        }
                        ConstValuePayload::Range { start, end } => {
                            work.push(FoldFrame::Finish {
                                value,
                                result_start,
                            });
                            work.push(FoldFrame::Visit(*end));
                            work.push(FoldFrame::Visit(*start));
                            next = work.pop();
                            continue;
                        }
                        ConstValuePayload::Coerced(child)
                        | ConstValuePayload::OptionSome(child) => {
                            work.push(FoldFrame::Finish {
                                value,
                                result_start,
                            });
                            work.push(FoldFrame::Visit(*child));
                            next = work.pop();
                            continue;
                        }
                    };
                    (value, visit)
                }
            };

            let mapped = visitor(&value.metadata, visit)?;
            if work.is_empty() {
                return Ok(mapped);
            }
            results.push(mapped);
            next = work.pop();
        }
        Err(fold_stack_error::<E>())
    }

    /// Drain directly into the field vector required by the callback, without an intermediate
    /// child vector. Each field's immutable metadata was validated when its `Field` frame ran.
    fn drain_fold_fields<'a, T, E>(
        &self,
        fields: &'a [ConstValueField],
        results: &mut Vec<T>,
        result_start: usize,
    ) -> Result<Vec<ConstValueFieldVisit<'a, T>>, E>
    where
        E: From<CompilerError>,
    {
        if results.len().checked_sub(result_start) != Some(fields.len()) {
            return Err(fold_stack_error::<E>());
        }
        Ok(results
            .drain(result_start..)
            .zip(fields)
            .map(|(value, field)| ConstValueFieldVisit {
                name: &field.name,
                type_id: self.values[field.value.index()].metadata.type_id,
                value,
            })
            .collect())
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
