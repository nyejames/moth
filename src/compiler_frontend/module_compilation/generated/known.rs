//! Immutable view of generated functions already published in one compilation boundary.
//!
//! WHAT: identity membership and exact call summaries for every sidecar the boundary has already
//!       committed.
//! WHY:  a module compile must reuse published generated work without reaching into the build
//!       system's mutable store. The build boundary lends this view for the duration of one module
//!       transaction; the compiler reads it and never writes through it.

use crate::compiler_frontend::module_compilation::generated::artefacts::{
    CompletedGeneratedFunction, GeneratedFunctionId,
};
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity;

use rustc_hash::FxHashMap;

/// Borrowed lookup over one boundary's committed generated records.
#[derive(Clone, Copy)]
pub(crate) struct KnownGeneratedFunctions<'a> {
    records: &'a [CompletedGeneratedFunction],
    by_identity: &'a FxHashMap<GeneratedFunctionIdentity, GeneratedFunctionId>,
}

impl<'a> KnownGeneratedFunctions<'a> {
    pub(crate) fn new(
        records: &'a [CompletedGeneratedFunction],
        by_identity: &'a FxHashMap<GeneratedFunctionIdentity, GeneratedFunctionId>,
    ) -> Self {
        Self {
            records,
            by_identity,
        }
    }

    /// Whether this boundary already published the given generated function.
    pub(crate) fn contains(&self, identity: &GeneratedFunctionIdentity) -> bool {
        self.by_identity.contains_key(identity)
    }

    /// The exact borrow summary of an already published generated function.
    pub(crate) fn summary(
        &self,
        identity: &GeneratedFunctionIdentity,
    ) -> Option<&'a PublicCallSummary> {
        self.by_identity
            .get(identity)
            .and_then(|id| self.records.get(id.index()))
            .map(|record| &record.summary)
    }
}
