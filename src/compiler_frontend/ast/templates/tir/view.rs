//! Central AST-local read API over the TIR store.
//!
//! WHAT: `TirView` is the single borrowed read surface that all future template
//! consumers use to inspect a structural root plus its overlay context inside a
//! `TemplateIrStore`. It pairs a module-local root `TemplateIrId`, a
//! `TemplateTirPhase`, and a `TemplateViewContext` so consumers never reach
//! into raw stores or combine overlay maps ad hoc.
//!
//! WHY: the final TIR architecture requires one production read API. Without a
//! central view, each consumer would re-implement store traversal, overlay
//! resolution, and phase checking, creating duplicated logic and stage-boundary
//! leaks. `TirView` keeps raw store traversal internal to the
//! view/store/builder/transform modules and exposes only the narrow facts
//! that composition, formatting, folding, and finalization need.
//!
//! ## Phase semantics
//!
//! `TemplateTirPhase` tracks how far a structural root has progressed through
//! the TIR pipeline:
//!
//! ```text
//! Parsed -> Composed -> Formatted -> Finalized
//! ```
//!
//! Consumers that need a particular minimum phase (e.g. folding requires at
//! least `Composed`) use [`TirView::with_minimum_phase`] so the check is
//! centralized and the error is a structured `CompilerError` rather than a
//! silent downgrade.
//!
//! ## Overlay resolution
//!
//! The view carries one `TemplateViewContext` produced by the shared
//! composition path. The overlay-dimension entry accessors
//! ([`TirView::expression_overlay`], [`TirView::slot_resolution_overlay`],
//! [`TirView::wrapper_context_overlay`]) resolve which overlays are in play.
//! Occurrence-keyed lookups ([`TirView::effective_expression_for_site`],
//! [`TirView::effective_slot_resolution`], and
//! [`TirView::effective_wrapper_context`]) resolve an effective value for a
//! specific site or occurrence by reading the current view context. When no
//! overlay entry covers the requested key, the caller falls back to the
//! structural node.
//!
//! ## Ownership contract
//!
//! `TirView` is AST-local and borrowed: it holds `&'a TemplateIrStore` and
//! lives only as long as the store. It is not exposed to HIR, backends, or
//! the public API.

use std::fmt;

use crate::compiler_frontend::compiler_errors::CompilerError;

use super::ids::ChildTemplateOccurrenceId;
use super::ids::{ExpressionSiteId, SlotOccurrenceId, TemplateIrId, TemplateIrNodeId};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::template::Template;

use super::node::{TemplateIr, TemplateIrNode, TirSlotPlaceholder};

use super::overlays::{
    TemplateViewContext, TirExpressionOverlay, TirSlotResolution, TirSlotResolutionOverlay,
    TirWrapperContext, TirWrapperContextOverlay,
};
use super::store::TemplateIrStore;

// -------------------------
//  TIR Phase
// -------------------------

/// Pipeline phase of a structural root inside a `TirView`.
///
/// WHAT: tracks the progression from raw parser output through composition,
/// formatting, and finalization. The variant declaration order matches the
/// semantic ordering, so derived `PartialOrd`/`Ord` comparisons reflect the
/// pipeline sequence.
///
/// WHY: consumers need to reject roots that have not yet reached the phase they
/// require (e.g. folding needs `Composed`, HIR handoff needs `Finalized`).
/// Centralizing the phase on the view lets one constructor enforce the minimum
/// instead of scattering ad hoc checks across every consumer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum TemplateTirPhase {
    /// Raw parser output; the tree has been emitted but not yet composed.
    Parsed,

    /// Child-template contributions and slot routing have been composed into the tree.
    Composed,

    /// Style formatters (e.g. `$md`) have been applied to the composed tree.
    Formatted,

    /// The tree is finalized and ready for HIR handoff.
    Finalized,
}

impl TemplateTirPhase {
    /// Returns whether this phase reaches `minimum`.
    pub(crate) fn is_at_least(self, minimum: TemplateTirPhase) -> bool {
        self >= minimum
    }
}

pub(super) fn structural_transition_context(
    parent_context: TemplateViewContext,
    reference_phase: TemplateTirPhase,
    referenced_context: TemplateViewContext,
) -> TemplateViewContext {
    TemplateViewContext {
        expression_overlay: parent_context.expression_overlay,
        slot_resolution: reference_phase
            .is_at_least(TemplateTirPhase::Composed)
            .then_some(referenced_context.slot_resolution)
            .flatten(),
        wrapper_context: reference_phase
            .is_at_least(TemplateTirPhase::Composed)
            .then_some(referenced_context.wrapper_context)
            .flatten(),
    }
}

impl fmt::Display for TemplateTirPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TemplateTirPhase::Parsed => write!(f, "Parsed"),
            TemplateTirPhase::Composed => write!(f, "Composed"),
            TemplateTirPhase::Formatted => write!(f, "Formatted"),
            TemplateTirPhase::Finalized => write!(f, "Finalized"),
        }
    }
}

// -------------------------
//  Finalized TirView Resolution
// -------------------------

/// Resolves the required finalized store-backed `TirView` for a `Template`.
///
/// WHAT: the single authority used by AST-to-HIR handoff, final type-boundary
///       validation and debug TypeId validation. It requires the template's
///       `tir_reference` to be at least `Finalized`, to resolve its root and
///       validate the optional payload IDs carried by its view context against
///       the exact module store through `TirView`. Every missing authority condition is an explicit
///       internal `CompilerError`. No caller may bypass that authority with raw
///       module-store reconstruction.
/// WHY: after normalization every template that reaches the AST-to-HIR boundary
///      owns a Finalized store-backed identity. A missing phase, root or overlay
///      is a compiler bug, not permission to reconstruct
///      template meaning from raw stores. Centralizing the required resolution
///      keeps the authority boundary in one place and removes duplicate local
///      reconstruction helpers from AST finalization.
pub(crate) fn finalized_tir_view_for_template<'a>(
    template: &Template,
    store: &'a TemplateIrStore,
) -> Result<TirView<'a>, CompilerError> {
    let reference = &template.tir_reference;

    if !reference.phase.is_at_least(TemplateTirPhase::Finalized) {
        return Err(CompilerError::compiler_error(format!(
            "finalized_tir_view_for_template: template TIR reference is at phase {:?}, final AST boundary consumers require Finalized",
            reference.phase
        )));
    }
    TirView::with_minimum_phase(
        store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Finalized,
        reference.context,
    )
}

// -------------------------
//  TirView
// -------------------------

/// Borrowed read view over a structural root owned by the store plus a
/// value-carried view context.
///
/// WHAT: pairs an immutable borrow of `TemplateIrStore` with a module-local
///       root `TemplateIrId`, a pipeline `TemplateTirPhase`, and a
///       `TemplateViewContext`. All read access goes through narrow methods
///       that validate root and overlay IDs and return `CompilerError` on failure.
///
/// WHY: this is the single production read API for template consumers. It
///      keeps raw store traversal internal and centralizes phase and overlay
///      validation so consumers do not re-implement those checks.
///
/// ## Construction
///
/// Use [`TirView::new`] for a basic view that validates the root template and
/// view context exist. Use [`TirView::with_minimum_phase`] when the consumer
/// additionally requires the root to have reached a particular pipeline phase.
/// Use the named transition methods to construct views over referenced roots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TirViewIdentity {
    pub(crate) root: TemplateIrId,
    pub(crate) phase: TemplateTirPhase,
    pub(crate) context: TemplateViewContext,
}

#[derive(Clone)]
pub(crate) struct TirView<'a> {
    store: &'a TemplateIrStore,
    identity: TirViewIdentity,
}

impl<'a> fmt::Debug for TirView<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TirView")
            .field("identity", &self.identity)
            .finish()
    }
}

impl<'a> TirView<'a> {
    // -------------------------
    //  Constructors
    // -------------------------

    /// Creates a view over `root` at `phase` with the given view context.
    ///
    /// WHAT: validates that `root` resolves to a template in the store and
    ///       that each optional overlay ID carried by `context` resolves to a
    ///       payload in the store.
    /// WHY: every consumer should go through a constructor so invalid store
    ///      IDs produce a structured `CompilerError` instead of a silent
    ///      placeholder or a later lookup panic.
    pub(crate) fn new(
        store: &'a TemplateIrStore,
        root: TemplateIrId,
        phase: TemplateTirPhase,
        context: TemplateViewContext,
    ) -> Result<TirView<'a>, CompilerError> {
        if store.get_template(root).is_none() {
            return Err(CompilerError::compiler_error(format!(
                "TirView::new: root template {} does not exist in the store",
                root
            )));
        }

        validate_context(store, context, "TirView::new")?;

        Ok(TirView {
            store,
            identity: TirViewIdentity {
                root,
                phase,
                context,
            },
        })
    }

    /// Creates a view and validates that `phase` satisfies `minimum_phase`.
    ///
    /// WHAT: performs the same root and view context validation as [`TirView::new`],
    ///       then additionally rejects views whose `phase` has not yet reached
    ///       `minimum_phase`.
    /// WHY: consumers such as folding (`Composed`) or HIR handoff (`Finalized`)
    ///      need to fail early with a structured error when a root is not ready
    ///      for their stage, rather than silently reading incomplete data.
    pub(crate) fn with_minimum_phase(
        store: &'a TemplateIrStore,
        root: TemplateIrId,
        phase: TemplateTirPhase,
        minimum_phase: TemplateTirPhase,
        context: TemplateViewContext,
    ) -> Result<TirView<'a>, CompilerError> {
        if !phase.is_at_least(minimum_phase) {
            return Err(CompilerError::compiler_error(format!(
                "TirView::with_minimum_phase: root {} at phase {} does not satisfy minimum phase {}",
                root, phase, minimum_phase
            )));
        }

        Self::new(store, root, phase, context)
    }

    /// Enters a structural child while retaining the current complete expression overlay.
    ///
    /// Parsed references cannot yet authorize their slot or wrapper dimensions. Composed and
    /// later references carry those dimensions, while the current root overlay remains the
    /// expression authority for the complete structural subtree.
    pub(crate) fn structural_child(
        &self,
        reference: super::refs::TemplateTirChildReference,
    ) -> Result<TirView<'a>, CompilerError> {
        self.structural_transition(
            reference.root,
            reference.phase,
            reference.context,
            "structural_child",
        )
    }

    /// Enters a wrapper through the same structural transition as a child template.
    pub(crate) fn wrapper(
        &self,
        reference: super::refs::TemplateWrapperReference,
    ) -> Result<TirView<'a>, CompilerError> {
        self.structural_transition(
            reference.root,
            reference.phase,
            reference.context,
            "wrapper",
        )
    }

    /// Enters a resolved slot source while retaining the current exact view context.
    pub(crate) fn resolved_slot_source(
        &self,
        root: TemplateIrId,
    ) -> Result<TirView<'a>, CompilerError> {
        self.transition(root, self.phase(), self.context(), "resolved_slot_source")
    }

    /// Enters an `InsertContribution` helper as a structural root.
    pub(crate) fn structural_helper(
        &self,
        root: TemplateIrId,
    ) -> Result<TirView<'a>, CompilerError> {
        self.transition(root, self.phase(), self.context(), "structural_helper")
    }

    /// Enters an independently owned nested template value.
    ///
    /// Nested AST template values use their durable reference in full. They do not inherit the
    /// containing structural root's expression overlay.
    pub(crate) fn nested_template_value(
        &self,
        reference: super::refs::TemplateTirReference,
    ) -> Result<TirView<'a>, CompilerError> {
        self.transition(
            reference.root,
            reference.phase,
            reference.context,
            "nested_template_value",
        )
    }

    fn structural_transition(
        &self,
        root: TemplateIrId,
        phase: TemplateTirPhase,
        referenced_context: TemplateViewContext,
        owner: &str,
    ) -> Result<TirView<'a>, CompilerError> {
        let context = structural_transition_context(self.context(), phase, referenced_context);
        self.transition(root, phase, context, owner)
    }

    fn transition(
        &self,
        root: TemplateIrId,
        phase: TemplateTirPhase,
        context: TemplateViewContext,
        owner: &str,
    ) -> Result<TirView<'a>, CompilerError> {
        if self.store.get_template(root).is_none() {
            return Err(CompilerError::compiler_error(format!(
                "TirView::{owner}: missing root_template {root}; it does not exist in the store"
            )));
        }

        validate_context(self.store, context, &format!("TirView::{owner}"))?;

        Ok(TirView {
            store: self.store,
            identity: TirViewIdentity {
                root,
                phase,
                context,
            },
        })
    }

    // -------------------------
    //  Narrow read accessors
    // -------------------------

    /// Returns the structural root ID.
    pub(crate) fn root_ref(&self) -> TemplateIrId {
        self.identity.root
    }

    /// Returns the view phase.
    pub(crate) fn phase(&self) -> TemplateTirPhase {
        self.identity.phase
    }

    /// Returns the value-carried overlay context.
    pub(crate) fn context(&self) -> TemplateViewContext {
        self.identity.context
    }

    /// Returns the exact identity for effective reads and cache keys.
    pub(crate) fn identity(&self) -> TirViewIdentity {
        self.identity
    }

    /// Borrows the store that owns this view.
    pub(crate) fn store(&self) -> &'a TemplateIrStore {
        self.store
    }

    /// Looks up a slot placeholder by occurrence.
    pub(crate) fn slot_placeholder(
        &self,
        occurrence: SlotOccurrenceId,
    ) -> Option<&'a TirSlotPlaceholder> {
        self.store.slot_placeholder(occurrence)
    }

    /// Resolves the root template or reports broken store authority.
    pub(crate) fn root_template(&self) -> Result<&'a TemplateIr, CompilerError> {
        self.store.get_template(self.identity.root).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "TirView::root_template: root {} was valid at construction but is now missing; this is a compiler bug",
                self.identity.root
            ))
        })
    }

    /// Returns an immutable borrow of the effective node at `node_ref`.
    ///
    /// WHAT: looks up a module-local node through the store. The
    ///       "effective" node is the structural node as stored; per-site
    ///       expression overrides and per-occurrence slot resolutions are
    ///       resolved through the occurrence-keyed lookup methods rather than
    ///       by replacing the structural node itself.
    /// WHY: consumers traverse the tree by following child `TemplateIrNodeId`
    ///      values stored on node payloads.  Routing those lookups through the
    ///      view keeps raw store traversal internal and lets later phases insert
    ///      overlay resolution without changing call sites.
    pub(crate) fn effective_node(
        &self,
        node_ref: TemplateIrNodeId,
    ) -> Result<&'a TemplateIrNode, CompilerError> {
        self.store.get_node(node_ref).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "TirView::effective_node: node {} does not exist in the store",
                node_ref
            ))
        })
    }

    // -------------------------
    //  Overlay-dimension entry accessors
    // -------------------------
    //
    // These accessors resolve the value context fields into the concrete per-dimension
    // overlay entry stored on the store. Returning `None` means "this overlay
    // dimension has no entry for this view's value context." A context that names a
    // missing overlay entry is an internal store invariant error.
    // Occurrence-keyed lookups on top of these entries are provided by the
    // methods in the "Occurrence-keyed overlay lookups" section below.

    /// Resolves the expression overlay named by this view.
    pub(crate) fn expression_overlay(
        &self,
    ) -> Result<Option<&'a TirExpressionOverlay>, CompilerError> {
        let Some(overlay_id) = self.identity.context.expression_overlay else {
            return Ok(None);
        };

        let overlay = self.store.expression_overlay(overlay_id).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "TirView::expression_overlay: overlay {} does not exist in the store",
                overlay_id
            ))
        })?;

        Ok(Some(overlay))
    }

    /// Resolves the slot-resolution overlay named by this view.
    pub(crate) fn slot_resolution_overlay(
        &self,
    ) -> Result<Option<&'a TirSlotResolutionOverlay>, CompilerError> {
        let Some(overlay_id) = self.identity.context.slot_resolution else {
            return Ok(None);
        };

        let overlay = self
            .store
            .slot_resolution_overlay(overlay_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "TirView::slot_resolution_overlay: overlay {} does not exist in the store",
                    overlay_id
                ))
            })?;

        Ok(Some(overlay))
    }

    /// Resolves the wrapper-context overlay named by this view.
    pub(crate) fn wrapper_context_overlay(
        &self,
    ) -> Result<Option<&'a TirWrapperContextOverlay>, CompilerError> {
        let Some(overlay_id) = self.identity.context.wrapper_context else {
            return Ok(None);
        };

        let overlay = self
            .store
            .wrapper_context_overlay(overlay_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "TirView::wrapper_context_overlay: overlay {} does not exist in the store",
                    overlay_id
                ))
            })?;

        Ok(Some(overlay))
    }
    /// Returns the override expression for an `ExpressionSiteId`, if the view
    /// context provides one.
    ///
    /// WHAT: resolves the expression overlay entry for this view's view context,
    ///       then looks up the override expression for `site_id` within that
    ///       entry. Returns `Ok(None)` when no expression overlay exists or the
    ///       overlay has no entry for this site.
    /// WHY: consumers that need the effective expression for a dynamic-expression
    ///      splice, branch selector, or loop-header expression site read it
    ///      through the view so overlay resolution stays centralized. When no
    ///      override exists, the caller falls back to the structural expression
    ///      stored on the node.
    pub(crate) fn effective_expression_for_site(
        &self,
        site_id: ExpressionSiteId,
    ) -> Result<Option<&'a Expression>, CompilerError> {
        let Some(overlay) = self.expression_overlay()? else {
            return Ok(None);
        };

        Ok(overlay.expression_for_site(site_id))
    }

    /// Returns the effective slot resolution for a `SlotOccurrenceId`, if the
    /// view context provides one.
    ///
    /// WHAT: resolves the slot-resolution overlay entry for this view's value
    ///       context, then looks up the resolution for `occurrence_id` within that
    ///       entry. Returns `Ok(None)` when no slot-resolution overlay exists or
    ///       the overlay has no entry for this occurrence.
    /// WHY: consumers that need the effective slot content for a slot occurrence
    ///      read it through the view so overlay resolution stays centralized.
    ///      When no resolution exists, the caller falls back to structural slot
    ///      routing.
    pub(crate) fn effective_slot_resolution(
        &self,
        occurrence_id: SlotOccurrenceId,
    ) -> Result<Option<&'a TirSlotResolution>, CompilerError> {
        let Some(overlay) = self.slot_resolution_overlay()? else {
            return Ok(None);
        };

        Ok(overlay.resolution_for_occurrence(occurrence_id))
    }

    /// Returns the effective wrapper context for a child-template occurrence.
    ///
    /// WHAT: resolves the wrapper-context overlay entry for this view's value
    ///       context, then looks up the context for `occurrence_id` within that
    ///       entry. Returns `Ok(None)` when no wrapper-context overlay exists or
    ///       the overlay has no entry for this child occurrence.
    /// WHY: view-native folding uses this to apply inherited `$children(..)`
    ///      wrappers around a child-template emission without mutating the
    ///      shared structural root.
    pub(crate) fn effective_wrapper_context(
        &self,
        occurrence_id: ChildTemplateOccurrenceId,
    ) -> Result<Option<&'a TirWrapperContext>, CompilerError> {
        let Some(overlay) = self.wrapper_context_overlay()? else {
            return Ok(None);
        };

        Ok(overlay.context_for_occurrence(occurrence_id))
    }
}

pub(crate) fn validate_context(
    store: &TemplateIrStore,
    context: TemplateViewContext,
    owner: &str,
) -> Result<(), CompilerError> {
    if let Some(id) = context.expression_overlay
        && store.expression_overlay(id).is_none()
    {
        return Err(CompilerError::compiler_error(format!(
            "{owner}: expression overlay {id} does not exist in the store"
        )));
    }

    if let Some(id) = context.slot_resolution
        && store.slot_resolution_overlay(id).is_none()
    {
        return Err(CompilerError::compiler_error(format!(
            "{owner}: slot resolution overlay {id} does not exist in the store"
        )));
    }

    if let Some(id) = context.wrapper_context
        && store.wrapper_context_overlay(id).is_none()
    {
        return Err(CompilerError::compiler_error(format!(
            "{owner}: wrapper context overlay {id} does not exist in the store"
        )));
    }

    Ok(())
}
