//! Typed identifiers for TIR store entries.
//!
//! WHAT: Newtyped `u32` IDs that index into the `TemplateIrStore` side vectors.
//! Each ID type is bound to one store collection so mixing indices across
//! collections is a compile error.
//!
//! WHY: Raw `usize` indices are easy to confuse. Newtypes prevent accidental
//! cross-collection index misuse and keep the store API self-documenting.
//!
//! ## Ownership contract
//!
//! IDs are AST-local. They are not exposed to HIR, backends, or the public API.
//! IDs remain valid only within the `TemplateIrStore` that created them.

use std::fmt;

// -------------------------
//  Template IR ID
// -------------------------

/// Stable index for a top-level template in `TemplateIrStore::templates`.
///
/// WHAT: identifies one `TemplateIr` entry by position in the store's template vector.
/// WHY: TIR uses IDs instead of borrowed references so the store can own all data
/// contiguously and hand out cheap `Copy` handles to later phases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TemplateIrId(u32);

impl TemplateIrId {
    /// Creates a new ID from a raw index.
    ///
    /// Panics if the index exceeds `u32::MAX`. This is an internal invariant —
    /// no realistic template count will approach this bound.
    pub(crate) fn new(index: usize) -> Self {
        Self(
            u32::try_from(index)
                .expect("template IR index exceeds u32::MAX; this is a compiler bug"),
        )
    }

    /// Returns the raw index for store lookups.
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for TemplateIrId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TemplateIrId({})", self.0)
    }
}

// -------------------------
//  Template IR Node ID
// -------------------------

/// Stable index for a node in `TemplateIrStore::nodes`.
///
/// WHAT: identifies one `TemplateIrNode` entry by position in the store's node vector.
/// WHY: nodes form a tree via child IDs; having a dedicated type keeps tree
/// navigation distinct from template-level references.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TemplateIrNodeId(u32);

impl TemplateIrNodeId {
    /// Creates a new ID from a raw index.
    pub(crate) fn new(index: usize) -> Self {
        Self(
            u32::try_from(index)
                .expect("template IR node index exceeds u32::MAX; this is a compiler bug"),
        )
    }

    /// Returns the raw index for store lookups.
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for TemplateIrNodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TemplateIrNodeId({})", self.0)
    }
}

// -------------------------
//  Template Wrapper Set ID
// -------------------------

/// Stable index for a wrapper set in `TemplateIrStore::wrapper_sets`.
///
/// WHAT: identifies a reusable set of `$children(..)` wrapper templates.
/// WHY: keeping wrapper ownership behind an ID avoids storing wrapper vectors
/// directly on `TemplateIr` and gives later phases a stable handle for
/// deduplicating identical wrapper combinations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TemplateWrapperSetId(u32);

impl TemplateWrapperSetId {
    /// Creates a new ID from a raw index.
    pub(crate) fn new(index: usize) -> Self {
        Self(
            u32::try_from(index)
                .expect("template wrapper set index exceeds u32::MAX; this is a compiler bug"),
        )
    }

    /// Returns the raw index for store lookups.
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for TemplateWrapperSetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TemplateWrapperSetId({})", self.0)
    }
}

// -------------------------
//  Template Slot Plan ID
// -------------------------

/// Stable index for a slot plan in `TemplateIrStore::slot_plans`.
///
/// WHAT: identifies one AST-prepared slot-routing plan carried by TIR.
/// WHY: runtime slot sites need a typed handle to the plan that owns their
/// `RuntimeSlotSiteId`, otherwise a bare site ID is ambiguous across templates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TemplateSlotPlanId(u32);

impl TemplateSlotPlanId {
    /// Creates a new ID from a raw index.
    pub(crate) fn new(index: usize) -> Self {
        Self(
            u32::try_from(index)
                .expect("template slot plan index exceeds u32::MAX; this is a compiler bug"),
        )
    }

    /// Returns the raw index for store lookups.
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for TemplateSlotPlanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TemplateSlotPlanId({})", self.0)
    }
}

// -------------------------
//  Slot Occurrence ID
// -------------------------

/// Document-order occurrence identifier for a `Slot` node.
///
/// WHAT: a per-store counter assigns one `SlotOccurrenceId` to each `Slot`
/// node in the order it is emitted during TIR construction.
/// WHY: overlay and slot-resolution phases need a stable key that identifies
/// which slot occurrence a contribution targets, without relying on traversal
/// side effects or node-vector positions that shift during composition.
///
/// IDs are preserved across derived roots because TIR nodes are store-owned
/// and shared by node ID: a derived root that reuses an existing node keeps
/// the occurrence ID already embedded in that node without re-allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SlotOccurrenceId(u32);

impl SlotOccurrenceId {
    /// Creates a new ID from a raw counter value.
    pub(crate) fn new(index: usize) -> Self {
        Self(
            u32::try_from(index)
                .expect("slot occurrence index exceeds u32::MAX; this is a compiler bug"),
        )
    }

    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for SlotOccurrenceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SlotOccurrenceId({})", self.0)
    }
}

// -------------------------
//  Child-Template Occurrence ID
// -------------------------

/// Document-order occurrence identifier for a `ChildTemplate` node.
///
/// WHAT: a per-store counter assigns one `ChildTemplateOccurrenceId` to each
/// `ChildTemplate` node in the order it is emitted during TIR construction.
/// WHY: wrapper overlays need a stable key that identifies which child-template
/// occurrence a wrapper context applies to, without relying on traversal order
/// or node-vector positions.
///
/// IDs are preserved across derived roots because TIR nodes are store-owned
/// and shared by node ID: a derived root that reuses an existing node keeps
/// the occurrence ID already embedded in that node without re-allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ChildTemplateOccurrenceId(u32);

impl ChildTemplateOccurrenceId {
    /// Creates a new ID from a raw counter value.
    pub(crate) fn new(index: usize) -> Self {
        Self(
            u32::try_from(index)
                .expect("child-template occurrence index exceeds u32::MAX; this is a compiler bug"),
        )
    }
}

impl fmt::Display for ChildTemplateOccurrenceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ChildTemplateOccurrenceId({})", self.0)
    }
}

// -------------------------
//  Expression Site ID
// -------------------------

/// Document-order site identifier for a `DynamicExpression` node.
///
/// WHAT: a per-store counter assigns one `ExpressionSiteId` to each
/// `DynamicExpression` node in the order it is emitted during TIR construction.
/// WHY: expression overlays need a stable key that identifies which expression
/// splice site an effective expression applies to, without relying on traversal
/// order or node-vector positions. Branch-selector and loop-header expression
/// sites receive their own `ExpressionSiteId`s from the same document-order
/// counter so all expression-bearing TIR sites share one overlay key space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ExpressionSiteId(u32);

impl ExpressionSiteId {
    /// Creates a new ID from a raw counter value.
    pub(crate) fn new(index: usize) -> Self {
        Self(
            u32::try_from(index)
                .expect("expression site index exceeds u32::MAX; this is a compiler bug"),
        )
    }

    /// Returns the raw module-local counter value for store validation.
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for ExpressionSiteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ExpressionSiteId({})", self.0)
    }
}
