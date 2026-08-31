//! TIR-native compile-time template folding.
//!
//! WHAT: folds a `TemplateIr` tree directly into an interned string emission
//!
//! WHY: folding works directly on the authoritative TIR representation, keeping
//! the fold stage decoupled from intermediate content surfaces.
//!
//! The reducer owns exact-view entry points, node dispatch and output assembly.
//! Branch/loop semantics and virtual wrapper insertion live in sibling modules.

use crate::compiler_frontend::ast::const_values::store::{ConstStringPiece, ConstStringValue};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::TemplateType;
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopControlKind;
use crate::compiler_frontend::ast::templates::template_folding::{
    TemplateEmission, TemplateFoldResult, TirFoldContext, resolve_fold_bindings_in_expression,
};
use crate::compiler_frontend::ast::templates::tir::ids::{
    ExpressionSiteId, SlotOccurrenceId, TemplateIrId, TemplateIrNodeId, TemplateWrapperSetId,
};
use crate::compiler_frontend::ast::templates::tir::node::TemplateIrNodeKind;
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TirSlotResolutionKind, TirWrapperApplicationMode,
};
use crate::compiler_frontend::ast::templates::tir::preparation::{
    TemplateHelperKind, TemplatePreparation, TemplatePreparationOutcome,
};
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirChildReference;
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirReference;
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::view::{TemplateTirPhase, TirView};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::instrumentation::{
    AstCounter, add_ast_counter, increment_ast_counter,
};
use crate::compiler_frontend::paths::module_resources::ResourceId;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::type_coercion::string::{
    FoldedStringPiece, fold_expression_kind_to_string,
};

use super::control_flow::{fold_tir_branch_chain_with_insertion, fold_tir_loop};
use super::estimate::{
    record_tir_fold_output_estimate_miss, record_tir_fold_output_intern,
    reserve_tir_fold_output_buffer,
};
use super::wrappers::{
    append_template_result_to_buffer, apply_wrapper_context_overlay_to_child_emission,
    fold_conditional_child_wrappers_around_emission,
};
/// Rejects `$insert(...)` helper templates at the exact fold boundary where
/// they would otherwise render as ordinary string content.
///
/// WHAT: every effective template source enters one of the fold-owned template
/// entry points before its root is walked, including slot-resolution sources,
/// wrapper-context wrappers and child-template references.
/// WHY: checking the selected template entry avoids scratch materialization,
/// stale-source reads and repeated whole-descendant prepasses. Raw
/// consumed `InsertContribution` nodes that aren't reachable from the effective
/// fold path remain correctly ignored.
pub(super) fn reject_slot_insert_template(kind: &TemplateType) -> Result<(), TemplateError> {
    if matches!(kind, TemplateType::SlotInsert(_)) {
        return Err(CompilerError::compiler_error(
            "Invalid template content reached string folding: unresolved slot insertions cannot be rendered directly.",
        )
        .into());
    }

    Ok(())
}

/// Borrowed exact-view input shared by recursive fold walkers.
///
/// WHAT: couples overlay reads with the exact view for the store currently
///       being traversed.
/// WHY: recursive node, control-flow, and wrapper folds must preserve the
///      complete view identity without expanding `TirFoldContext`.
pub(super) struct FoldTraversalInput<'view, 'store> {
    pub(super) view: &'view TirView<'store>,
    pub(super) projection_enabled: bool,
    pub(super) projection_allowed_slot_insert_root: Option<TemplateIrId>,
}

impl<'view, 'store> FoldTraversalInput<'view, 'store> {
    pub(super) fn with_view<'next>(
        &self,
        view: &'next TirView<'store>,
    ) -> FoldTraversalInput<'next, 'store>
    where
        'view: 'next,
    {
        FoldTraversalInput {
            view,
            projection_enabled: self.projection_enabled,
            projection_allowed_slot_insert_root: self.projection_allowed_slot_insert_root,
        }
    }

    /// Resolves one expression site through the active exact view.
    ///
    /// WHAT: delegates expression lookup to the shared module-local view.
    /// WHY: structural transitions retain the complete expression authority
    ///       carried by the current view while changing other dimensions.
    pub(super) fn effective_expression_for_site(
        &self,
        site_id: ExpressionSiteId,
    ) -> Result<Option<&'store Expression>, TemplateError> {
        Ok(self.view.effective_expression_for_site(site_id)?)
    }
}
/// AST-local result used to project a folded const-template value into a public interface.
///
/// The reducer returns structured text, resource and site-root pieces and slot pieces before the
/// donor-local TIR store is dropped. The pre-coalescing piece list deliberately differs from the
/// final [`ConstStringValue`] emission because slots remain template composition markers.
pub(crate) struct FoldedConstTemplatePattern {
    pub(crate) pieces: Vec<FoldedConstTemplatePiece>,
    pub(crate) emission: TemplateEmission,
    pub(crate) provenance: SyntheticInterfaceProvenance,
}

/// One pre-coalesced piece in a const-template projection.
///
/// WHAT: preserves authored text runs, module-local structural anchors and unresolved slot
/// occurrences before projection groups non-slot runs into the shared owned string vocabulary.
/// WHY: `Resource` and `SiteRoot` are hard boundaries for text coalescing, while `Slot` is a
/// separate composition boundary that must remain visible to public const-template projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FoldedConstTemplatePiece {
    Text(String),
    Resource(ResourceId),
    SiteRoot,
    Slot(SlotOccurrenceId),
}

/// Mutable output state shared by recursive TIR fold walkers.
///
/// WHAT: keeps rendered text, structural emission pieces, output presence and semantic provenance
/// together while a fold descends through nodes, branches and wrapper subtrees.
/// WHY: these values describe one fold result and must travel through the same recursive call
/// chains without expanding each helper's parameter list. The text buffer remains the fast path
/// until the first structural anchor requires a [`ConstStringValue::Pieces`] result.
pub(super) struct FoldOutputState {
    pub(super) output_buffer: String,
    pub(super) emitted_output: bool,
    pub(super) provenance: SyntheticInterfaceProvenance,
    pub(super) projection_pieces: Option<Vec<FoldedConstTemplatePiece>>,
    structural_pieces: Option<Vec<ConstStringPiece>>,
}

impl FoldOutputState {
    pub(super) fn new(output_buffer: String) -> Self {
        Self {
            output_buffer,
            emitted_output: false,
            provenance: SyntheticInterfaceProvenance::empty(),
            projection_pieces: None,
            structural_pieces: None,
        }
    }

    pub(super) fn with_capacity(estimated_bytes: usize) -> Self {
        Self::new(reserve_tir_fold_output_buffer(estimated_bytes))
    }

    pub(super) fn with_provenance(
        output_buffer: String,
        provenance: SyntheticInterfaceProvenance,
    ) -> Self {
        Self {
            output_buffer,
            emitted_output: false,
            provenance,
            projection_pieces: None,
            structural_pieces: None,
        }
    }

    pub(super) fn enable_projection(&mut self) {
        self.projection_pieces = Some(Vec::new());
    }

    /// Append text to the pre-coalesced projection and the current output run.
    ///
    /// WHAT: extends the trailing text run when possible and keeps text in the output buffer until
    /// a structural anchor forces piece materialization.
    /// WHY: text may coalesce only within one run. `Resource`, `SiteRoot` and `Slot` callers add a
    /// boundary to the projection before later text reaches this method.
    pub(super) fn append_text(&mut self, text: &str) {
        self.output_buffer.push_str(text);

        let Some(pieces) = &mut self.projection_pieces else {
            return;
        };

        if let Some(FoldedConstTemplatePiece::Text(previous)) = pieces.last_mut() {
            previous.push_str(text);
        } else if !text.is_empty() {
            pieces.push(FoldedConstTemplatePiece::Text(text.to_owned()));
        }
    }

    /// Append one module-local structural string piece to fold output and projection output.
    ///
    /// WHAT: flushes the current text run before retaining a resource or site-root anchor, then
    /// records the same boundary in the optional const-template projection.
    /// WHY: the fold result must preserve authored order and cannot render an unresolved anchor as
    /// text. The module-local `ResourceId` remains valid only until public projection or handoff.
    pub(super) fn append_structural_piece(
        &mut self,
        piece: &ConstStringPiece,
        string_table: &mut crate::compiler_frontend::symbols::string_interning::StringTable,
    ) {
        self.start_structural_output(string_table);
        self.flush_structural_text(string_table);

        if let Some(structural_pieces) = &mut self.structural_pieces {
            structural_pieces.push(piece.clone());
        }

        if let Some(projection_pieces) = &mut self.projection_pieces {
            projection_pieces.push(match piece {
                ConstStringPiece::Text(text) => {
                    FoldedConstTemplatePiece::Text(string_table.resolve(*text).to_owned())
                }
                ConstStringPiece::Resource(resource) => {
                    FoldedConstTemplatePiece::Resource(*resource)
                }
                ConstStringPiece::SiteRoot => FoldedConstTemplatePiece::SiteRoot,
            });
        }
    }

    /// Append an already folded value without duplicating its projection markers.
    ///
    /// WHAT: contributes only the emission value to the output state. Callers append a child
    /// result's separate projection list when projection mode is active.
    /// WHY: child projections include slots that cannot be represented by [`ConstStringValue`], so
    /// combining both representations here would duplicate text or reorder slot boundaries.
    pub(super) fn append_emission_value(
        &mut self,
        value: &ConstStringValue,
        string_table: &mut crate::compiler_frontend::symbols::string_interning::StringTable,
    ) {
        match value {
            ConstStringValue::Text(text) => {
                self.output_buffer.push_str(string_table.resolve(*text));
            }
            ConstStringValue::Pieces(pieces) => {
                for piece in pieces {
                    match piece {
                        ConstStringPiece::Text(text) => {
                            self.output_buffer.push_str(string_table.resolve(*text));
                        }
                        ConstStringPiece::Resource(_) | ConstStringPiece::SiteRoot => {
                            self.start_structural_output(string_table);
                            self.flush_structural_text(string_table);
                            if let Some(structural_pieces) = &mut self.structural_pieces {
                                structural_pieces.push(piece.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    pub(super) fn append_pieces(
        &mut self,
        pieces_to_append: Option<&[FoldedConstTemplatePiece]>,
    ) -> Result<(), TemplateError> {
        if self.projection_pieces.is_none() {
            return Ok(());
        }
        let pieces_to_append = pieces_to_append.ok_or_else(|| {
            CompilerError::compiler_error(
                "TIR const-template projection lost structured pieces while appending folded output.",
            )
        })?;

        for piece in pieces_to_append {
            match piece {
                FoldedConstTemplatePiece::Text(text) => self.append_projection_text(text),
                FoldedConstTemplatePiece::Resource(resource) => {
                    if let Some(pieces) = &mut self.projection_pieces {
                        pieces.push(FoldedConstTemplatePiece::Resource(*resource));
                    }
                }
                FoldedConstTemplatePiece::SiteRoot => {
                    if let Some(pieces) = &mut self.projection_pieces {
                        pieces.push(FoldedConstTemplatePiece::SiteRoot);
                    }
                }
                FoldedConstTemplatePiece::Slot(occurrence) => {
                    if let Some(pieces) = &mut self.projection_pieces {
                        pieces.push(FoldedConstTemplatePiece::Slot(*occurrence));
                    }
                }
            }
        }

        Ok(())
    }

    fn append_projection_text(&mut self, text: &str) {
        let Some(pieces) = &mut self.projection_pieces else {
            return;
        };

        if let Some(FoldedConstTemplatePiece::Text(previous)) = pieces.last_mut() {
            previous.push_str(text);
        } else if !text.is_empty() {
            pieces.push(FoldedConstTemplatePiece::Text(text.to_owned()));
        }
    }

    pub(super) fn append_slot(&mut self, occurrence: SlotOccurrenceId) {
        if let Some(pieces) = &mut self.projection_pieces {
            pieces.push(FoldedConstTemplatePiece::Slot(occurrence));
        }
    }

    fn start_structural_output(
        &mut self,
        string_table: &mut crate::compiler_frontend::symbols::string_interning::StringTable,
    ) {
        if self.structural_pieces.is_none() {
            let mut pieces = Vec::new();
            if !self.output_buffer.is_empty() {
                pieces.push(ConstStringPiece::Text(
                    string_table.intern(&self.output_buffer),
                ));
                self.output_buffer.clear();
            }
            self.structural_pieces = Some(pieces);
        }
    }

    fn flush_structural_text(
        &mut self,
        string_table: &mut crate::compiler_frontend::symbols::string_interning::StringTable,
    ) {
        let Some(structural_pieces) = &mut self.structural_pieces else {
            return;
        };
        if !self.output_buffer.is_empty() {
            structural_pieces.push(ConstStringPiece::Text(
                string_table.intern(&self.output_buffer),
            ));
            self.output_buffer.clear();
        }
    }

    pub(super) fn into_const_string_value(
        mut self,
        string_table: &mut crate::compiler_frontend::symbols::string_interning::StringTable,
    ) -> ConstStringValue {
        let Some(mut structural_pieces) = self.structural_pieces.take() else {
            return ConstStringValue::Text(string_table.intern(&self.output_buffer));
        };

        self.flush_structural_text(string_table);
        ConstStringValue::Pieces(std::mem::take(&mut structural_pieces))
    }
}

/// Optional virtual output inserted while one shared reducer walks a TIR tree.
///
/// Normal folding has no insertion. Wrapper folding injects the already-folded child at one slot
/// key, while aggregate-wrapper folding replaces one `AggregateOutput` marker. Each insertion
/// borrows the same module-local [`ConstStringValue`] shape emitted by the reducer.
#[derive(Clone, Copy)]
pub(super) enum FoldInsertion<'a> {
    None,
    Slot {
        key: &'a SlotKey,
        output: &'a ConstStringValue,
        projection: Option<&'a [FoldedConstTemplatePiece]>,
    },
    Aggregate {
        output: &'a ConstStringValue,
        projection: Option<&'a [FoldedConstTemplatePiece]>,
    },
}
impl FoldInsertion<'_> {
    pub(super) fn is_aggregate(self) -> bool {
        matches!(self, Self::Aggregate { .. })
    }
}

// -------------------------
//  Public entry point
// -------------------------

/// Folds one prepared, exact TIR view into its owned emission and provenance.
///
/// WHAT: consumes the completed preparation proof and enters the reducer without
///      reclassifying or re-walking the template for authority.
/// WHY: preparation is the sole semantic classifier. The identity check must
///      happen before reduction so a stale proof can never authorize output.
pub(crate) fn fold_prepared_template(
    prepared: &TemplatePreparation,
    view: TirView<'_>,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<TemplateFoldResult, TemplateError> {
    if prepared.identity != view.identity()
        || !matches!(prepared.outcome, TemplatePreparationOutcome::Foldable)
    {
        return Err(CompilerError::compiler_error(
            "TIR fold preparation root/phase/context identity does not match the supplied view.",
        )
        .into());
    }

    if !view.phase().is_at_least(TemplateTirPhase::Composed) {
        return Err(CompilerError::compiler_error(format!(
            "fold_prepared_template: root {} at phase {} has not reached Composed",
            view.root_ref(),
            view.phase()
        ))
        .into());
    }

    fold_exact_view(&view, fold_context)
}

/// Folds a prepared const-template value while preserving unresolved slot positions.
///
/// This is a projection mode of the canonical reducer, not a second template interpreter. All
/// expression, control-flow, formatting, child-view and wrapper behavior stays in the ordinary
/// fold path while unresolved slots are retained as structured pieces.
pub(crate) fn fold_prepared_const_template_pattern(
    prepared: TemplatePreparation,
    view: TirView<'_>,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<FoldedConstTemplatePattern, TemplateError> {
    if prepared.identity != view.identity() {
        return Err(CompilerError::compiler_error(
            "TIR const-template projection preparation identity does not match the supplied view.",
        )
        .into());
    }

    match prepared.outcome {
        TemplatePreparationOutcome::Foldable
        | TemplatePreparationOutcome::Helper(TemplateHelperKind::SlotInsert) => {}
        TemplatePreparationOutcome::Helper(TemplateHelperKind::LoopControl) => {
            return Err(CompilerError::compiler_error(
                "TIR const-template projection cannot publish a loop-control helper.",
            )
            .into());
        }
        TemplatePreparationOutcome::Runtime(_) => {
            return Err(CompilerError::compiler_error(
                "TIR const-template projection received a runtime preparation.",
            )
            .into());
        }
    }

    if !view.phase().is_at_least(TemplateTirPhase::Composed) {
        return Err(CompilerError::compiler_error(format!(
            "fold_prepared_const_template_pattern: root {} at phase {} has not reached Composed",
            view.root_ref(),
            view.phase()
        ))
        .into());
    }

    let result = fold_exact_view_with_projection(&view, fold_context, true, Some(view.root_ref()))?;
    let pieces = result.projection_pieces.ok_or_else(|| {
        CompilerError::compiler_error(
            "TIR const-template projection completed without structured output pieces.",
        )
    })?;

    match result.emission {
        TemplateEmission::NoOutput | TemplateEmission::Output(_) => {
            Ok(FoldedConstTemplatePattern {
                pieces,
                emission: result.emission,
                provenance: result.provenance,
            })
        }
        TemplateEmission::Break(_) | TemplateEmission::Continue(_) => {
            Err(CompilerError::compiler_error(
                "TIR const-template projection produced an unconsumed loop-control signal.",
            )
            .into())
        }
    }
}

/// Folds one exact Composed-or-later view, consulting the phase-local cache.
///
/// WHAT: validates the view's structural and overlay authority before looking
///       up its precise cache key, then reduces and caches the exact result
///       when no loop bindings are active.
/// WHY: the root and repeated structural child/source folds share one cache
///      owner without preparing or classifying recursively. Parsed structural
///      children and virtual injected-wrapper folds intentionally bypass this
///      helper because their reduction semantics are different.
fn fold_exact_view(
    view: &TirView<'_>,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<TemplateFoldResult, TemplateError> {
    fold_exact_view_with_projection(view, fold_context, false, None)
}

fn fold_exact_view_with_projection(
    view: &TirView<'_>,
    fold_context: &mut TirFoldContext<'_>,
    projection_enabled: bool,
    projection_allowed_slot_insert_root: Option<TemplateIrId>,
) -> Result<TemplateFoldResult, TemplateError> {
    // Attribute one prepared view fold per store-backed view, across
    // finalization, doc-fragment, and HIR-handoff callers.
    increment_ast_counter(AstCounter::TirViewFoldsAttempted);

    let has_expression_overlay = view.context().expression_overlay.is_some();
    let has_slot_overlay = view.context().slot_resolution.is_some();
    let has_wrapper_context = view.context().wrapper_context.is_some();

    // Attribute the overlay shape so callers can rank which overlay combinations
    // drive the view-native fold path.
    match (has_expression_overlay, has_slot_overlay) {
        (false, false) => increment_ast_counter(AstCounter::TirViewFoldOverlayEmpty),
        (true, false) => increment_ast_counter(AstCounter::TirViewFoldOverlayExpressionOnly),
        (false, true) => increment_ast_counter(AstCounter::TirViewFoldOverlaySlotOnly),
        (true, true) => increment_ast_counter(AstCounter::TirViewFoldOverlayExpressionAndSlot),
    }
    if has_wrapper_context {
        increment_ast_counter(AstCounter::TirViewFoldWrapperContextPresent);
    }

    // View-native fold: pass the exact view to the reducer so it reads
    // effective expressions and slot resolutions without cloning the store.
    let fold_input = FoldTraversalInput {
        view,
        projection_enabled,
        projection_allowed_slot_insert_root,
    };
    let result = fold_tir_template_with_view(fold_context, &fold_input)?;

    Ok(result)
}

/// Folds a TIR template through one required exact `TirView`.
///
/// WHAT: the fold walker reads structural nodes from `store` but consults `view`
///       for effective expressions (dynamic-expression sites, branch selectors,
///       loop headers) and slot resolutions.
/// WHY: view-native overlay reads let folding apply expression, slot, and
///      wrapper-context overrides without mutating or cloning the store.
fn fold_tir_template_with_view(
    fold_context: &mut TirFoldContext<'_>,
    fold_input: &FoldTraversalInput<'_, '_>,
) -> Result<TemplateFoldResult, TemplateError> {
    let view = fold_input.view;
    let store = view.store();
    let template_id = view.root_ref();
    add_ast_counter(AstCounter::TirFoldTemplatesFolded, 1);

    let template = store
        .get_template(template_id)
        .ok_or_else(|| missing_template_diagnostic(template_id))?;
    if fold_input.projection_allowed_slot_insert_root != Some(template_id) {
        reject_slot_insert_template(&template.kind)?;
    }

    if template.runtime_slot_plan.is_some() {
        return Err(CompilerError::compiler_error(
            "TIR fold: a runtime slot plan reached the fold reducer without a foldable preparation proof.",
        )
        .into());
    }

    let estimated_bytes = template.summary.estimated_output_bytes;
    let mut output_state = FoldOutputState::with_capacity(estimated_bytes);
    if fold_input.projection_enabled {
        output_state.enable_projection();
    }

    let signal = fold_tir_node_into_buffer(
        template.root,
        &mut output_state,
        fold_context,
        fold_input,
        FoldInsertion::None,
    )?;

    let emission = build_emission_from_buffer(output_state, estimated_bytes, signal, fold_context)?;

    let wrapper_references = match template.conditional_child_wrapper_set {
        Some(wrapper_set_id) => store
            .get_wrapper_set(wrapper_set_id)
            .ok_or_else(|| missing_wrapper_set_diagnostic(wrapper_set_id))?
            .wrappers
            .as_slice(),
        None => &[],
    };

    let TemplateFoldResult {
        emission,
        provenance,
        projection_pieces,
    } = emission;
    fold_conditional_child_wrappers_around_emission(
        wrapper_references,
        emission,
        provenance,
        projection_pieces,
        TirWrapperApplicationMode::IfChildEmits,
        fold_context,
        fold_input,
    )
}

// -------------------------
//  Node folding
// -------------------------

/// Folds a single TIR node into an independent emission.
///
/// WHAT: creates a fresh output buffer for the node and returns the full
/// `TemplateEmission`. This is the right shape for branch bodies and loop
/// bodies, which may produce break/continue signals.
pub(super) fn fold_tir_node(
    node_id: TemplateIrNodeId,
    fold_context: &mut TirFoldContext<'_>,
    fold_input: &FoldTraversalInput<'_, '_>,
    insertion: FoldInsertion<'_>,
) -> Result<TemplateFoldResult, TemplateError> {
    let mut output_state = FoldOutputState::new(String::new());
    if fold_input.projection_enabled {
        output_state.enable_projection();
    }

    let signal = fold_tir_node_into_buffer(
        node_id,
        &mut output_state,
        fold_context,
        fold_input,
        insertion,
    )?;

    build_emission_from_buffer(output_state, 0, signal, fold_context)
}

/// Folds a single TIR node, appending any output to the caller's buffer.
///
/// WHAT: dispatches on node kind and appends output directly. Returns an
/// optional loop-control signal when the node (or a nested node) produced one.
pub(super) fn fold_tir_node_into_buffer(
    node_id: TemplateIrNodeId,
    output_state: &mut FoldOutputState,
    fold_context: &mut TirFoldContext<'_>,
    fold_input: &FoldTraversalInput<'_, '_>,
    insertion: FoldInsertion<'_>,
) -> Result<Option<TemplateLoopControlKind>, TemplateError> {
    let store = fold_input.view.store();
    add_ast_counter(AstCounter::TirFoldNodesVisited, 1);

    let node = store
        .get_node(node_id)
        .ok_or_else(|| missing_node_diagnostic(node_id))?;

    match &node.kind {
        TemplateIrNodeKind::Sequence { children } => fold_tir_sequence(
            children,
            output_state,
            fold_context,
            fold_input,
            insertion,
        ),

        TemplateIrNodeKind::Text { text, .. } => {
            let text = fold_context.string_table.resolve(*text);
            output_state.append_text(text);
            output_state.emitted_output = true;
            Ok(None)
        }

        TemplateIrNodeKind::DynamicExpression { expression, site_id, .. } => {
            let effective_expression = fold_input.effective_expression_for_site(*site_id)?;
            let expression_to_fold = effective_expression.unwrap_or(expression);
            fold_tir_dynamic_expression(
                expression_to_fold,
                output_state,
                fold_context,
                &node.location,
                fold_input,
            )
        }

        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
            ..
        } => {
            let occurrence_context = fold_input
                .view
                .effective_wrapper_context(*occurrence_id)?
                .cloned();
            let emission = match insertion {
                FoldInsertion::None => fold_child_template_reference(
                    reference,
                    fold_context,
                    fold_input,
                )?,
                FoldInsertion::Slot { .. } | FoldInsertion::Aggregate { .. } => {
                    fold_child_template_with_insertion(
                        reference,
                        insertion,
                        fold_context,
                        fold_input,
                    )?
                }
            };
            output_state.provenance.merge(&emission.provenance);

            let wrapped_emission = if insertion.is_aggregate() {
                emission
            } else {
                apply_wrapper_context_overlay_to_child_emission(
                    emission,
                    fold_context,
                    fold_input,
                    occurrence_context.as_ref(),
                )?
            };
            output_state.provenance.merge(&wrapped_emission.provenance);

            append_template_result_to_buffer(wrapped_emission, output_state, fold_context)
        }

        TemplateIrNodeKind::Slot { placeholder } => {
            if let FoldInsertion::Slot {
                key,
                output,
                projection,
            } = insertion
                && key == &placeholder.key
            {
                if output_state.projection_pieces.is_some() {
                    output_state.append_pieces(projection)?;
                }
                output_state.append_emission_value(output, fold_context.string_table);
                output_state.emitted_output = true;
                return Ok(None);
            }

            if insertion.is_aggregate() {
                return Err(CompilerError::compiler_error(
                    "TIR fold: malformed aggregate wrapper subtree contains a slot.",
                )
                .into());
            }

            if let Some(resolution) = fold_input
                .view
                .effective_slot_resolution(placeholder.occurrence_id)?
                && let TirSlotResolutionKind::Resolved { sources } = &resolution.kind
            {
                for source in sources {
                    let emission = fold_resolved_slot_source(
                        *source,
                        fold_context,
                        fold_input,
                    )?;
                    output_state.provenance.merge(&emission.provenance);
                    append_template_result_to_buffer(emission, output_state, fold_context)?;
                }
                return Ok(None);
            }

            if fold_input.projection_enabled {
                output_state.append_slot(placeholder.occurrence_id);
                output_state.emitted_output = true;
            }
            Ok(None)
        }

        TemplateIrNodeKind::InsertContribution { .. } => Err(CompilerError::compiler_error(
            "Insert contribution reached TIR folding without being consumed by slot composition.",
        )
        .into()),

        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            fold_tir_branch_chain_with_insertion(
                branches,
            *fallback,
            output_state,
            fold_context,
            fold_input,
            insertion,
            )
        }

        TemplateIrNodeKind::Loop {
            header,
            header_sites,
            body,
            aggregate_wrapper,
        } => {
            if insertion.is_aggregate() {
                return Err(CompilerError::compiler_error(
                    "TIR fold: malformed aggregate wrapper subtree contains a loop.",
                )
                .into());
            }
            fold_tir_loop(
                header,
                *header_sites,
                *body,
                *aggregate_wrapper,
                output_state,
                fold_context,
                fold_input,
                &node.location,
                insertion,
            )
        }

        TemplateIrNodeKind::AggregateOutput => match insertion {
            FoldInsertion::Aggregate { output, projection } => {
                output_state.append_emission_value(output, fold_context.string_table);
                output_state.append_pieces(projection)?;
                output_state.emitted_output = true;
                Ok(None)
            }
            FoldInsertion::None | FoldInsertion::Slot { .. } => Err(
                CompilerError::compiler_error(
                    "TIR fold: AggregateOutput marker reached a fold site outside a loop aggregate wrapper.",
                )
                .into(),
            ),
        },

        TemplateIrNodeKind::LoopControl { kind } => {
            if insertion.is_aggregate() {
                return Err(CompilerError::compiler_error(
                    "TIR fold: loop-control signal reached aggregate wrapper folding.",
                )
                .into());
            }
            Ok(Some(*kind))
        }

        TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => Err(
            CompilerError::compiler_error(
                "TIR fold: runtime slot nodes cannot enter the constant-fold reducer.",
            )
            .into(),
        ),
    }
}

/// Folds a sequence node by folding each child in authored order.
fn fold_tir_sequence(
    children: &[TemplateIrNodeId],
    output_state: &mut FoldOutputState,
    fold_context: &mut TirFoldContext<'_>,
    fold_input: &FoldTraversalInput<'_, '_>,
    insertion: FoldInsertion<'_>,
) -> Result<Option<TemplateLoopControlKind>, TemplateError> {
    for &child_id in children {
        let signal =
            fold_tir_node_into_buffer(child_id, output_state, fold_context, fold_input, insertion)?;

        if signal.is_some() {
            return Ok(signal);
        }
    }

    Ok(None)
}

/// Folds a dynamic expression node after resolving fold bindings.
fn fold_tir_dynamic_expression(
    expression: &Expression,
    output_state: &mut FoldOutputState,
    fold_context: &mut TirFoldContext<'_>,
    location: &SourceLocation,
    fold_input: &FoldTraversalInput<'_, '_>,
) -> Result<Option<TemplateLoopControlKind>, TemplateError> {
    let store = fold_input.view.store();
    let resolved = resolve_fold_bindings_in_expression(expression, fold_context)?;
    let expression_ref: &Expression = match &resolved {
        crate::compiler_frontend::ast::templates::template_folding::FoldResolvedExpression::Borrowed(
            expr,
        ) => expr,
        crate::compiler_frontend::ast::templates::template_folding::FoldResolvedExpression::Owned(
            expr,
        ) => expr,
    };

    // This is the exact selected dynamic-expression payload. Binding
    // substitution has already happened, so its metadata includes any
    // resolved loop/branch binding provenance without a second AST walk.
    output_state
        .provenance
        .merge(&expression_ref.synthetic_interface_provenance);

    if matches!(
        expression_ref.kind,
        ExpressionKind::RuntimeSlotApplicationHandoff(_)
    ) {
        // Runtime slot applications are helper-owned payloads. They contribute
        // no compile-time text when a surrounding const fold selects this path.
        return Ok(None);
    }

    if let Some(template) = nested_template_value(expression_ref) {
        let template_kind = nested_template_kind(template, store)?;

        // Comments are compile-time metadata and intentionally contribute no
        // rendered output. Slot inserts remain composition helpers and must be
        // rejected if they reach this final nested-value fold boundary.
        if matches!(template_kind, TemplateType::Comment(_)) {
            return Ok(None);
        }
        reject_slot_insert_template(template_kind)?;

        let nested_result = fold_template_reference(
            FoldTemplateReference::Nested(&template.tir_reference),
            fold_context,
            fold_input,
        )?;
        output_state.provenance.merge(&nested_result.provenance);
        return append_template_result_to_buffer(nested_result, output_state, fold_context);
    }

    if let Some(pieces) = structural_string_pieces(&expression_ref.kind) {
        for piece in pieces {
            match piece {
                ConstStringPiece::Text(text) => {
                    output_state.append_text(fold_context.string_table.resolve(*text));
                }
                ConstStringPiece::Resource(_) | ConstStringPiece::SiteRoot => {
                    output_state.append_structural_piece(piece, fold_context.string_table);
                }
            }
        }
        output_state.emitted_output = true;
        return Ok(None);
    }

    match fold_expression_kind_to_string(&expression_ref.kind, fold_context.string_table) {
        Some(FoldedStringPiece::Text(text)) => {
            output_state.append_text(&text);
            output_state.emitted_output = true;
            Ok(None)
        }

        Some(FoldedStringPiece::Char(ch)) => {
            let text = ch.to_string();
            output_state.append_text(&text);
            output_state.emitted_output = true;
            Ok(None)
        }

        None => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::NonFoldableConstTemplate,
            location.to_owned(),
        )
        .into()),
    }
}
/// Returns structural string pieces through contextual coercion wrappers.
///
/// WHAT: exposes the module-local piece list that the reducer must append without asking the
/// scalar string-coercion helper to render unresolved anchors.
/// WHY: structural strings and scalar values share the language-level `String` type, but only the
/// TIR reducer owns ordered template output and can preserve anchor boundaries.
fn structural_string_pieces(kind: &ExpressionKind) -> Option<&[ConstStringPiece]> {
    match kind {
        ExpressionKind::StructuralString { pieces } => Some(pieces),
        ExpressionKind::Coerced { value, .. } => structural_string_pieces(&value.kind),
        _ => None,
    }
}

/// Reads a nested AST template's kind from its authoritative TIR entry.
fn nested_template_kind<'a>(
    template: &Template,
    store: &'a TemplateIrStore,
) -> Result<&'a TemplateType, TemplateError> {
    store
        .get_template(template.tir_reference.root)
        .map(|template_ir| &template_ir.kind)
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "TIR fold: nested template kind for {} was not found in the module store.",
                template.tir_reference.root
            ))
            .into()
        })
}

/// Finds a nested template wrapped by contextual coercion nodes.
fn nested_template_value(expression: &Expression) -> Option<&Template> {
    match &expression.kind {
        ExpressionKind::Template(template) => Some(template),
        ExpressionKind::Coerced { value, .. } => nested_template_value(value),
        _ => None,
    }
}

/// Folds a module-local child-template reference against the module store.
///
/// WHAT: uses the precise `root`/`phase`/`context` identity stored on the
///       `ChildTemplate` node to enter the named structural transition and fold
///       through the resulting exact view. Parsed references retain only the
///       parent expression authority; Composed references additionally carry
///       their slot and wrapper dimensions.
/// WHY: child-template nodes carry enough identity for precise view-based
///      folding. Reading the root from the active store keeps cache and
///      overlay identity intact.
fn fold_child_template_reference(
    reference: &TemplateTirChildReference,
    fold_context: &mut TirFoldContext<'_>,
    fold_input: &FoldTraversalInput<'_, '_>,
) -> Result<TemplateFoldResult, TemplateError> {
    fold_template_reference(
        FoldTemplateReference::Structural(reference),
        fold_context,
        fold_input,
    )
}

/// Folds a child-template root while carrying one virtual insertion mode.
///
/// Ordinary child references use the exact-view fold path above. Wrapper and
/// aggregate reducers instead keep their insertion active through the child
/// root so slots or aggregate markers nested below the reference are handled
/// by the same reducer.
fn fold_child_template_with_insertion(
    reference: &TemplateTirChildReference,
    insertion: FoldInsertion<'_>,
    fold_context: &mut TirFoldContext<'_>,
    fold_input: &FoldTraversalInput<'_, '_>,
) -> Result<TemplateFoldResult, TemplateError> {
    let store = fold_input.view.store();
    let child_template = store
        .get_template(reference.root)
        .cloned()
        .ok_or_else(|| missing_template_diagnostic(reference.root))?;
    reject_slot_insert_template(&child_template.kind)?;

    if child_template.runtime_slot_plan.is_some() {
        return Err(CompilerError::compiler_error(
            "TIR insertion fold: a runtime child slot plan reached the fold reducer without a foldable preparation proof.",
        )
        .into());
    }

    let child_view = fold_input.view.structural_child(*reference)?;
    let child_fold_input = fold_input.with_view(&child_view);
    fold_tir_node(
        child_template.root,
        fold_context,
        &child_fold_input,
        insertion,
    )
}

enum FoldTemplateReference<'reference> {
    Structural(&'reference TemplateTirChildReference),
    Nested(&'reference TemplateTirReference),
}

/// Resolves one effective template reference through the module store, then
/// enters the canonical template fold path.
///
/// Structural child and nested AST references use their named `TirView`
/// transitions. Every Composed-or-later exact child view uses the shared cache;
/// Parsed structural children use the direct reducer because their referenced
/// overlay dimensions are not active yet.
fn fold_template_reference(
    reference: FoldTemplateReference<'_>,
    fold_context: &mut TirFoldContext<'_>,
    fold_input: &FoldTraversalInput<'_, '_>,
) -> Result<TemplateFoldResult, TemplateError> {
    let child_view = {
        let parent_view = fold_input.view;

        match reference {
            FoldTemplateReference::Structural(reference) => {
                parent_view.structural_child(*reference)?
            }
            FoldTemplateReference::Nested(reference) => {
                if !reference.phase.is_at_least(TemplateTirPhase::Composed) {
                    return Err(CompilerError::compiler_error(format!(
                        "TIR fold: nested template {} at phase {} has not reached Composed.",
                        reference.root, reference.phase
                    ))
                    .into());
                }

                parent_view.nested_template_value(*reference)?
            }
        }
    };

    let child_fold_input = fold_input.with_view(&child_view);
    if child_view.phase().is_at_least(TemplateTirPhase::Composed) {
        fold_exact_view_with_projection(
            &child_view,
            fold_context,
            fold_input.projection_enabled,
            fold_input.projection_allowed_slot_insert_root,
        )
    } else {
        fold_tir_template_with_view(fold_context, &child_fold_input)
    }
}

fn fold_resolved_slot_source(
    source: TemplateIrId,
    fold_context: &mut TirFoldContext<'_>,
    fold_input: &FoldTraversalInput<'_, '_>,
) -> Result<TemplateFoldResult, TemplateError> {
    let parent_view = fold_input.view;
    let source_view = parent_view.resolved_slot_source(source)?;
    let source_fold_input = fold_input.with_view(&source_view);
    if source_view.phase().is_at_least(TemplateTirPhase::Composed) {
        fold_exact_view_with_projection(
            &source_view,
            fold_context,
            fold_input.projection_enabled,
            fold_input.projection_allowed_slot_insert_root,
        )
    } else {
        fold_tir_template_with_view(fold_context, &source_fold_input)
    }
}

/// Builds a `TemplateEmission` from a filled output buffer.
pub(super) fn build_emission_from_buffer(
    mut output_state: FoldOutputState,
    estimated_bytes: usize,
    signal: Option<TemplateLoopControlKind>,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<TemplateFoldResult, TemplateError> {
    if signal.is_some() && !output_state.emitted_output {
        return Ok(TemplateFoldResult::with_projection(
            match signal {
                Some(TemplateLoopControlKind::Break) => TemplateEmission::Break(None),
                Some(TemplateLoopControlKind::Continue) => TemplateEmission::Continue(None),
                None => unreachable!(),
            },
            output_state.provenance,
            output_state.projection_pieces,
        ));
    }

    if !output_state.emitted_output {
        return Ok(TemplateFoldResult::with_projection(
            TemplateEmission::NoOutput,
            output_state.provenance,
            output_state.projection_pieces,
        ));
    }

    let actual_len = output_state.output_buffer.len();
    record_tir_fold_output_estimate_miss(actual_len, estimated_bytes);
    let provenance = std::mem::replace(
        &mut output_state.provenance,
        SyntheticInterfaceProvenance::empty(),
    );
    let projection_pieces = output_state.projection_pieces.take();
    let output = output_state.into_const_string_value(fold_context.string_table);
    record_tir_fold_output_intern(actual_len);

    Ok(TemplateFoldResult::with_projection(
        match signal {
            None => TemplateEmission::Output(output),
            Some(TemplateLoopControlKind::Break) => TemplateEmission::Break(Some(output)),
            Some(TemplateLoopControlKind::Continue) => TemplateEmission::Continue(Some(output)),
        },
        provenance,
        projection_pieces,
    ))
}

// -------------------------
//  Internal diagnostics
// -------------------------

fn missing_template_diagnostic(template_id: TemplateIrId) -> CompilerError {
    CompilerError::compiler_error(format!(
        "TIR fold referenced template {} that is not present in the store.",
        template_id
    ))
}

pub(super) fn missing_node_diagnostic(node_id: TemplateIrNodeId) -> CompilerError {
    CompilerError::compiler_error(format!(
        "TIR fold referenced node {} that is not present in the store.",
        node_id
    ))
}

fn missing_wrapper_set_diagnostic(wrapper_set_id: TemplateWrapperSetId) -> CompilerError {
    CompilerError::compiler_error(format!(
        "TIR fold referenced wrapper set {} that is not present in the store.",
        wrapper_set_id
    ))
}
