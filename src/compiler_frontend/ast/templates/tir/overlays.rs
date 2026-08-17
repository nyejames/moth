//! TIR overlay payloads, compact dimension IDs, and value-carried view context.
//!
//! WHAT: stores immutable occurrence-keyed overlay payloads and the exact three
//! optional overlay dimensions carried by each TIR view.
//!
//! WHY: contextual template state belongs to the reference or view that uses it.
//! Keeping the dimensions as values avoids indirect context storage while preserving
//! typed store lookup and last-context precedence at narrow composition sites.
//!
//! ## Payload coverage
//!
//! Expression and wrapper-context payloads are production surfaces. The
//! slot-resolution dimension remains part of the canonical exact-view contract
//! even though no current production path allocates it.
//! Wrapper-context overlays carry inherited `$children(..)` wrapper sets and
//! `$fresh` suppression state for individual child-template occurrences. They
//! are read by exact-view consumers so finalization can apply inherited wrappers
//! without mutating shared child-template nodes.

use std::fmt;
use std::num::NonZeroU32;

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::templates::tir::ids::{
    ChildTemplateOccurrenceId, ExpressionSiteId, SlotOccurrenceId, TemplateIrId,
    TemplateWrapperSetId,
};
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};

// -------------------------
//  Overlay dimension IDs
// -------------------------

/// A compact module-local overlay payload index.
///
/// The stored value is the zero-based vector index plus one. This keeps every
/// `Option<...Id>` one word without reserving a zero sentinel as a valid ID.
macro_rules! compact_overlay_id {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub(crate) struct $name(NonZeroU32);

        impl $name {
            #[allow(
                dead_code,
                reason = "the exact-view contract retains dimensions without requiring a current production allocator"
            )]
            pub(crate) fn new(index: usize) -> Self {
                let index = u32::try_from(index).expect(concat!(
                    $label,
                    " index exceeds u32::MAX; internal compiler invariant violated"
                ));
                let encoded = index.checked_add(1).expect(concat!(
                    $label,
                    " index-plus-one overflowed; internal compiler invariant violated"
                ));
                Self(NonZeroU32::new(encoded).expect(concat!(
                    $label,
                    " ID cannot be zero; internal compiler invariant violated"
                )))
            }

            pub(crate) fn index(self) -> usize {
                self.0.get() as usize - 1
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self.0.get() - 1)
            }
        }
    };
}

// Store index for expression overrides.
compact_overlay_id!(TirExpressionOverlayId, "expression overlay");

// Store index for slot-resolution overrides.
compact_overlay_id!(TirSlotResolutionOverlayId, "slot resolution overlay");

// Store index for inherited-wrapper context.
compact_overlay_id!(TirWrapperContextOverlayId, "wrapper context overlay");

// -------------------------
//  Slot resolution value type
// -------------------------

/// Effective content state for one slot occurrence.
///
/// WHAT: distinguishes slots that are resolved to one or more contribution
///       templates from slots that are known missing.
/// WHY: missing slots render as empty output, while unresolved slots remain
///      structural work for later phases. Keeping those states explicit avoids
///      treating "no contribution templates" as an ambiguous magic value.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TirSlotResolutionKind {
    /// The slot occurrence resolves to these contribution template refs.
    ///
    /// Repeated slot occurrences can carry equivalent source lists so replay is
    /// represented by data, not by consuming the routed contribution.
    Resolved { sources: Vec<TemplateIrId> },

    /// The slot was routed and intentionally receives no content.
    Missing,
}

/// One slot-resolution overlay entry for a slot occurrence.
///
/// WHAT: records the slot key and effective content state for one `Slot`
///       occurrence when a slot-resolution overlay applies.
/// WHY: slot resolution needs a typed value so overlay entries and `TirView`
///      lookups return a clear, testable result. The key makes default,
///      named, and positional routing explicit at the overlay boundary, while
///      the content state supports missing slots and repeated-slot replay.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TirSlotResolution {
    /// Slot key that this occurrence declared.
    pub(crate) key: SlotKey,
    /// Effective routed content for the occurrence.
    pub(crate) kind: TirSlotResolutionKind,
}

impl TirSlotResolution {
    /// Creates a resolved slot entry.
    #[allow(
        dead_code,
        reason = "the exact-view contract retains slot-resolution payloads without a current production allocator"
    )]
    pub(crate) fn resolved(key: SlotKey, sources: Vec<TemplateIrId>) -> Self {
        Self {
            key,
            kind: TirSlotResolutionKind::Resolved { sources },
        }
    }

    /// Creates a missing slot entry.
    #[allow(
        dead_code,
        reason = "the exact-view contract retains slot-resolution payloads without a current production allocator"
    )]
    pub(crate) fn missing(key: SlotKey) -> Self {
        Self {
            key,
            kind: TirSlotResolutionKind::Missing,
        }
    }

    /// Returns contribution sources, or an empty slice for a missing slot.
    pub(crate) fn sources(&self) -> &[TemplateIrId] {
        match &self.kind {
            TirSlotResolutionKind::Resolved { sources } => sources,
            TirSlotResolutionKind::Missing => &[],
        }
    }
}

// -------------------------
//  Overlay payloads
// -------------------------

// These payload structs give the overlay dimension IDs real store storage
// with final-system-oriented names. Each payload carries occurrence-keyed
// entries so `TirView` can resolve contextual template state centrally instead
// of making consumers combine ad hoc maps.

/// Store-owned expression override payload.
///
/// WHAT: carries expression overrides keyed by `ExpressionSiteId`. Each entry
///       replaces the structural expression at one dynamic-expression splice
///       site (or branch-selector / loop-header site) when the overlay applies.
/// WHY: expression overlays are one of the three overlay dimensions; storing
///      overrides as a keyed list lets `TirView` resolve effective expressions
///      for a site without traversing the structural tree.
///
/// Entries are stored as a flat list rather than a hash map because
/// `Expression` does not implement `Hash` or `Eq`, and realistic overlays
/// contain few entries. Linear lookup is sufficient for the overlay sizes
/// expected in template composition.
#[derive(Clone, Debug, Default)]
pub(crate) struct TirExpressionOverlay {
    /// Expression overrides, keyed by document-order `ExpressionSiteId`.
    pub(crate) overrides: Vec<(ExpressionSiteId, Box<Expression>)>,
}

impl TirExpressionOverlay {
    /// Looks up the override expression for a site, if one exists in this overlay.
    ///
    /// WHAT: linear scan of the override list. Returns the boxed expression
    ///       reference when the site has an override, or `None` when it does not.
    /// WHY: `TirView` calls this to resolve effective expressions for a site;
    ///      keeping the lookup on the payload centralizes the scan logic.
    pub(crate) fn expression_for_site(&self, site_id: ExpressionSiteId) -> Option<&Expression> {
        increment_ast_counter(AstCounter::TirOverlayLookups);
        self.overrides
            .iter()
            .find(|(id, _)| *id == site_id)
            .map(|(_, expression)| expression.as_ref())
    }
}

/// Store-owned slot resolution payload.
///
/// WHAT: carries slot resolutions keyed by `SlotOccurrenceId`. Each entry
///       describes what contribution fills one slot occurrence when the
///       overlay applies.
/// WHY: slot resolution is one of the three overlay dimensions; storing
///      resolutions as a keyed list lets `TirView` resolve effective slot
///      content for an occurrence without traversing the structural tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TirSlotResolutionOverlay {
    /// Slot resolutions, keyed by document-order `SlotOccurrenceId`.
    pub(crate) resolutions: Vec<(SlotOccurrenceId, TirSlotResolution)>,
}

impl TirSlotResolutionOverlay {
    /// Looks up a resolution by slot occurrence.
    pub(crate) fn resolution_for_occurrence(
        &self,
        occurrence_id: SlotOccurrenceId,
    ) -> Option<&TirSlotResolution> {
        increment_ast_counter(AstCounter::TirOverlayLookups);
        self.resolutions
            .iter()
            .find(|(id, _)| *id == occurrence_id)
            .map(|(_, resolution)| resolution)
    }
}

// -------------------------
//  Wrapper application mode
// -------------------------

/// Controls when inherited wrappers apply to a child-template occurrence.
///
/// WHAT: distinguishes ordinary direct children (wrappers always apply) from
///       control-flow children (wrappers apply only if the child structurally
///       emitted output).
/// WHY: the real semantic rule is not "guarded by this condition expression" —
///      it is "apply wrappers only if this child structurally emitted output."
///      That rule matters for false `if` branches, no-else branches, zero-
///      iteration loops, and break/continue behavior.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TirWrapperApplicationMode {
    /// Wrappers always apply around this child occurrence.
    #[default]
    Always,

    /// Wrappers apply only when the child occurrence structurally emits output.
    /// Used for control-flow children where skipped branches or zero-iteration
    /// loops must not receive wrappers.
    IfChildEmits,
}

/// Wrapper context applied at one child-template occurrence.
///
/// WHAT: records the inherited direct-child wrapper set, `$fresh` suppression
///       state, and wrapper application mode that apply at a child-template
///       boundary.
/// WHY: direct-child wrapper application is contextual. Storing it by child
///      occurrence lets `TirView` wrapper resolution apply context without
///      mutating shared child-template nodes.
///
/// Design constraint: the conditional output guard must use an explicit
/// `TirWrapperApplicationMode` enum (`Always` / `IfChildEmits`), not loose
/// expression-site guard vectors. The real semantic rule is not "guarded by
/// this condition expression" — it is "apply wrappers only if this child
/// structurally emitted output." That rule matters for false `if` branches,
/// no-else branches, zero-iteration loops, and break/continue behavior.
///
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TirWrapperContext {
    /// Wrapper set inherited by this child occurrence, when one applies.
    pub(crate) inherited_wrapper_set: Option<TemplateWrapperSetId>,
    /// True when `$fresh` suppresses the immediate parent wrapper context.
    pub(crate) skip_parent_child_wrappers: bool,
    /// When wrappers apply to this child occurrence.
    pub(crate) application_mode: TirWrapperApplicationMode,
}

/// Store-owned wrapper context overlay payload.
///
/// WHAT: carries wrapper context keyed by `ChildTemplateOccurrenceId`.
/// WHY: wrapper context is one of the three overlay dimensions; storing
///      occurrence-keyed entries lets `TirView` resolve inherited wrappers and
///      `$fresh` suppression at child-template boundaries without ad hoc maps.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TirWrapperContextOverlay {
    /// Wrapper contexts keyed by document-order child-template occurrence ID.
    pub(crate) contexts: Vec<(ChildTemplateOccurrenceId, TirWrapperContext)>,
}

impl TirWrapperContextOverlay {
    /// Looks up inherited-wrapper context by child occurrence.
    pub(crate) fn context_for_occurrence(
        &self,
        occurrence_id: ChildTemplateOccurrenceId,
    ) -> Option<&TirWrapperContext> {
        increment_ast_counter(AstCounter::TirOverlayLookups);
        self.contexts
            .iter()
            .find(|(id, _)| *id == occurrence_id)
            .map(|(_, context)| context)
    }
}

// -------------------------
//  Value-carried view context
// -------------------------

/// Exact contextual identity carried by a TIR reference or view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct TemplateViewContext {
    pub(crate) expression_overlay: Option<TirExpressionOverlayId>,
    pub(crate) slot_resolution: Option<TirSlotResolutionOverlayId>,
    pub(crate) wrapper_context: Option<TirWrapperContextOverlayId>,
}

impl TemplateViewContext {
    /// Merges one newer context over this context, preserving last-context
    /// precedence independently for each overlay dimension.
    pub(crate) fn merge(self, newer: Self) -> Self {
        Self {
            expression_overlay: newer.expression_overlay.or(self.expression_overlay),
            slot_resolution: newer.slot_resolution.or(self.slot_resolution),
            wrapper_context: newer.wrapper_context.or(self.wrapper_context),
        }
    }
}
