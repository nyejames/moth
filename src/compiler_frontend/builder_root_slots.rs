//! Compiler-owned builder root-slot contract.
//!
//! WHAT: the statically registered named destinations a builder exposes for direct top-level
//!       const contributions, plus the destination vocabulary carried by the const-fragment
//!       handoff.
//! WHY: the frontend must validate direct root `[$in name: ...]` targets without depending on
//!      a project builder, arbitrary callbacks or builder-specific names. A builder registers
//!      these names once through its surface; the compiler treats the set as read-only input.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

/// One statically known named destination registered by the active builder.
///
/// WHAT: owns the authored destination name so it stays stable across module string-table merges.
/// WHY: fragment metadata outlives the module-local `StringTable`; a donor-local `StringId` would
///      name a different string after publication.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BuilderRootSlotName(Arc<str>);

impl BuilderRootSlotName {
    pub(crate) fn new(name: impl Into<Arc<str>>) -> Self {
        Self(name.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BuilderRootSlotName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Read-only set of named destinations registered by the active builder.
///
/// WHAT: a compact ordered name set. Registration happens while a builder defines its surface;
///       compilation only reads it.
/// WHY: top-level named contributions are a deliberately shallow, builder-owned contract. The
///      ordered set gives deterministic diagnostics without a registry or callback engine.
#[derive(Clone, Debug, Default)]
pub struct BuilderRootSlotSchema {
    names: BTreeSet<BuilderRootSlotName>,
}

impl BuilderRootSlotSchema {
    pub(crate) fn register(&mut self, name: impl Into<Arc<str>>) {
        self.names.insert(BuilderRootSlotName::new(name));
    }

    /// Resolve an authored destination spelling to its registered stable name.
    pub(crate) fn resolve(&self, name: &str) -> Option<BuilderRootSlotName> {
        self.names
            .iter()
            .find(|candidate| candidate.as_str() == name)
            .cloned()
    }

    /// Registered names in deterministic order for authored diagnostics.
    pub(crate) fn names(&self) -> impl Iterator<Item = &str> + '_ {
        self.names.iter().map(BuilderRootSlotName::as_str)
    }
}

/// Destination of one ordered top-level const fragment.
///
/// WHAT: distinguishes the default body lane from a builder-registered named destination.
/// WHY: builders merge body fragments into rendered output, while named fragments must stay in
///      the module artefact for builder interpretation without appearing in the body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConstFragmentDestination {
    /// The ordinary default body destination.
    Body,
    /// One named destination registered by the active builder.
    Named(BuilderRootSlotName),
}

impl ConstFragmentDestination {
    pub(crate) fn is_body(&self) -> bool {
        matches!(self, Self::Body)
    }
}
