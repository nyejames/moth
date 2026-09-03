//! Compiler-owned synthetic compile-time interface provenance vocabulary.
//!
//! WHAT: owns the stable, deterministic semantic identity for synthetic compile-time interface
//! members and the provenance value that records one function's direct synthetic-interface
//! dependencies. Empty provenance means portable (no project-context dependency). Non-empty
//! provenance carries a sorted, duplicate-free, member-granular dependency set.
//! WHY: the compiler design overview requires stable member-granular synthetic-interface
//! identities and provenance. AST values, AST-to-HIR function facts, HIR link facts and the
//! project-global facade consume them without leaking process-local IDs, source locations,
//! interned names or unordered iteration.
//!
//! ## Ownership boundary
//!
//! This module owns the provenance vocabulary and its deterministic set operations only. It does
//! not own propagation policy, AST traversal, HIR lowering or link-fact propagation. Those remain
//! with AST expression/value handling, AST-to-HIR function-fact accumulation, HIR link-fact
//! reachability and project-global facade projection; this module supplies their shared identity.

/// Classification of a synthetic compile-time interface's scope.
///
/// WHAT: records whether the interface is project-context scoped or builder-owned. The
/// classification is semantic, not inferred from the authored `@project` spelling.
/// WHY: the representation must name both current project-context interfaces and future
/// builder-owned synthetic interfaces without adding a second provenance vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum SyntheticInterfaceClass {
    /// Project-global membership. Non-portable: the dependency set depends on project context.
    ProjectContext,
    /// Builder-owned synthetic interface. Reserved for future builder-owned producers.
    #[allow(
        dead_code,
        reason = "The builder-owned producer lands in a later builder interface slice."
    )]
    Builder,
}

/// Stable, self-contained identity for one member of a synthetic compile-time interface.
///
/// WHAT: carries the interface class, interface name and member name as owned strings. It stores
/// no `StringId`, `InternedPath`, local compiler IDs, source locations, absolute paths or
/// rendered display names. Iteration order is deterministic because the identity is totally
/// ordered.
/// WHY: downstream link-fact propagation and cross-build provenance comparison need a stable
/// identity that survives across processes, checkouts and module compilation boundaries.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct SyntheticInterfaceMemberIdentity {
    class: SyntheticInterfaceClass,
    interface: String,
    member: String,
}

impl SyntheticInterfaceMemberIdentity {
    /// Construct a synthetic interface member identity.
    ///
    /// The names are owned stable strings, not interned IDs or rendered display names.
    pub(crate) fn new(
        class: SyntheticInterfaceClass,
        interface: impl Into<String>,
        member: impl Into<String>,
    ) -> Self {
        Self {
            class,
            interface: interface.into(),
            member: member.into(),
        }
    }

    /// The semantic scope classification of this member.
    #[cfg(test)]
    pub(crate) fn class(&self) -> SyntheticInterfaceClass {
        self.class
    }

    /// The stable synthetic interface name.
    #[cfg(test)]
    pub(crate) fn interface(&self) -> &str {
        &self.interface
    }

    /// The stable member name within the interface.
    #[cfg(test)]
    pub(crate) fn member(&self) -> &str {
        &self.member
    }
}

/// Semantic-provenance value recording one function's direct synthetic-interface dependencies.
///
/// WHAT: an empty value means portable (no synthetic-interface dependency). A non-empty value
/// carries a sorted, duplicate-free set of member-granular dependencies. The set is canonical
/// after construction; `merge` and `union` preserve the canonical sorted, duplicate-free order.
/// WHY: per-function link facts need a stable, deterministic provenance value that does not
/// depend on iteration order, source location or process-local identity. AST-to-HIR lowering
/// carries this value into HIR link-fact collection, while the project-global facade retains it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct SyntheticInterfaceProvenance {
    members: Vec<SyntheticInterfaceMemberIdentity>,
}

impl SyntheticInterfaceProvenance {
    /// Empty portable provenance: no synthetic-interface dependencies.
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    /// Construct provenance from a set of member identities.
    ///
    /// Compiler-internal: sorts and deduplicates the members so the value is canonical regardless
    /// of caller insertion order.
    pub(crate) fn from_members(members: Vec<SyntheticInterfaceMemberIdentity>) -> Self {
        let mut sorted = members;
        sorted.sort();
        sorted.dedup();
        Self { members: sorted }
    }

    /// Provenance carrying a single direct synthetic-interface dependency.
    pub(crate) fn single(member: SyntheticInterfaceMemberIdentity) -> Self {
        Self::from_members(vec![member])
    }

    /// Whether this provenance is empty (portable).
    pub(crate) fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
    /// Whether this provenance contains a member from the requested interface class.
    ///
    /// Consumers use this narrow read-only query to enforce class-specific boundary policy
    /// without exposing the canonical member storage or depending on iteration order.
    pub(crate) fn contains_class(&self, class: SyntheticInterfaceClass) -> bool {
        self.members.iter().any(|member| member.class == class)
    }

    /// The sorted, duplicate-free member dependencies.
    ///
    /// Tests inspect this canonical slice to verify deterministic unions and AST-to-HIR
    /// association; production consumers use set operations and class queries without exposing
    /// member storage.
    #[cfg(test)]
    pub(crate) fn members(&self) -> &[SyntheticInterfaceMemberIdentity] {
        &self.members
    }

    /// Produce a new provenance value that is the sorted, duplicate-free union of `self` and
    /// `other`.
    pub(crate) fn union(&self, other: &Self) -> Self {
        let mut combined = Vec::with_capacity(self.members.len() + other.members.len());
        combined.extend(self.members.iter().cloned());
        combined.extend(other.members.iter().cloned());
        Self::from_members(combined)
    }

    /// Produce the canonical union of a sequence of provenance values.
    pub(crate) fn union_all<'a>(
        provenances: impl IntoIterator<Item = &'a SyntheticInterfaceProvenance>,
    ) -> Self {
        let mut combined = Self::empty();
        for provenance in provenances {
            combined.merge(provenance);
        }
        combined
    }

    /// Merge `other` into `self` in place, keeping the canonical sorted, duplicate-free order.
    pub(crate) fn merge(&mut self, other: &Self) {
        if other.is_empty() {
            return;
        }
        let merged = std::mem::take(self).union(other);
        *self = merged;
    }
}
