//! Immutable normalized input for one Boracle borrow-analysis problem.
//!
//! WHAT: owns the typed problem vocabulary, atomic construction boundary and deterministic dump.
//! WHY: the reference solver must inspect explicit CFG/events/places/origins without reparsing HIR
//!      or recovering meaning from binding names.
//!
//! This module is shared infrastructure, not a normal compiler pass. HIR extraction and solving
//! are explicit consumers, and normal compilation does not construct a problem.

// The alpha checker deliberately does not construct this shared vocabulary yet. Keeping the
// allowance local documents that these rows are a published future-consumer seam, not dead code
// that should be removed from the normal compiler path.
#![allow(dead_code)]

mod bindings;
mod builder;
mod control_flow;
mod events;
mod ids;
mod origins;
mod places;
mod validation;

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;

#[allow(unused_imports)]
pub(crate) use bindings::Binding;
#[allow(unused_imports)]
pub(crate) use builder::from_hir;
pub(crate) use control_flow::{CfgBlock, CfgEdge, ControlFlow, ProgramPoint};
#[allow(unused_imports)]
pub(crate) use events::{
    AccessKind, AggregateField, Call, CallArgument, CallEffect, CallResult, Event, EventKind,
    EventSource, KillReason, Loan, RebindValue, TerminatorEventKind, Use, UseKind,
};
#[allow(unused_imports)]
pub(crate) use ids::{
    BindingId, BlockId, CallId, EventId, LoanId, PlaceId, PointId, UseId, ValueOriginId,
};
#[allow(unused_imports)]
pub(crate) use origins::{CallResultProvenance, CallResultUnknownReason, OriginKind, ValueOrigin};
#[allow(unused_imports)]
pub(crate) use places::{Place, PlaceOverlap, ProjectionElem};

/// Mutable assembly data consumed by the atomic [`BorrowProblem::new`] boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BorrowProblemParts {
    pub(crate) bindings: Vec<Binding>,
    pub(crate) points: Vec<ProgramPoint>,
    pub(crate) blocks: Vec<CfgBlock>,
    pub(crate) edges: Vec<CfgEdge>,
    pub(crate) entry: BlockId,
    pub(crate) exits: Vec<BlockId>,
    pub(crate) places: Vec<Place>,
    pub(crate) origins: Vec<ValueOrigin>,
    pub(crate) loans: Vec<Loan>,
    pub(crate) uses: Vec<Use>,
    pub(crate) calls: Vec<Call>,
    pub(crate) events: Vec<Event>,
}

impl Default for BorrowProblemParts {
    fn default() -> Self {
        Self {
            bindings: Vec::new(),
            points: Vec::new(),
            blocks: Vec::new(),
            edges: Vec::new(),
            entry: BlockId::new(0),
            exits: Vec::new(),
            places: Vec::new(),
            origins: Vec::new(),
            loans: Vec::new(),
            uses: Vec::new(),
            calls: Vec::new(),
            events: Vec::new(),
        }
    }
}

/// One complete, immutable normalized borrow-analysis input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BorrowProblem {
    bindings: Box<[Binding]>,
    control_flow: ControlFlow,
    points: Box<[ProgramPoint]>,
    places: Box<[Place]>,
    origins: Box<[ValueOrigin]>,
    loans: Box<[Loan]>,
    uses: Box<[Use]>,
    calls: Box<[Call]>,
    events: Box<[Event]>,
}

impl BorrowProblem {
    /// Validate every cross-reference before publishing the immutable problem.
    pub(crate) fn new(
        parts: BorrowProblemParts,
    ) -> Result<Self, crate::compiler_frontend::compiler_errors::CompilerError> {
        let problem = Self {
            bindings: parts.bindings.into_boxed_slice(),
            control_flow: ControlFlow::new(parts.blocks, parts.edges, parts.entry, parts.exits),
            points: parts.points.into_boxed_slice(),
            places: parts.places.into_boxed_slice(),
            origins: parts.origins.into_boxed_slice(),
            loans: parts.loans.into_boxed_slice(),
            uses: parts.uses.into_boxed_slice(),
            calls: parts.calls.into_boxed_slice(),
            events: parts.events.into_boxed_slice(),
        };
        problem.validate()?;
        Ok(problem)
    }

    /// Re-run the atomic invariant check at a consumer boundary.
    pub(crate) fn validate(
        &self,
    ) -> Result<(), crate::compiler_frontend::compiler_errors::CompilerError> {
        validation::validate(self)
    }

    /// Produce a stable human-readable representation for dumps and snapshot tests.
    pub(crate) fn debug_dump(&self) -> String {
        format!("{self:#?}")
    }

    pub(crate) fn control_flow(&self) -> &ControlFlow {
        &self.control_flow
    }

    pub(crate) fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    pub(crate) fn points(&self) -> &[ProgramPoint] {
        &self.points
    }

    pub(crate) fn places(&self) -> &[Place] {
        &self.places
    }

    pub(crate) fn origins(&self) -> &[ValueOrigin] {
        &self.origins
    }

    pub(crate) fn loans(&self) -> &[Loan] {
        &self.loans
    }

    pub(crate) fn uses(&self) -> &[Use] {
        &self.uses
    }

    pub(crate) fn calls(&self) -> &[Call] {
        &self.calls
    }

    pub(crate) fn events(&self) -> &[Event] {
        &self.events
    }
}
