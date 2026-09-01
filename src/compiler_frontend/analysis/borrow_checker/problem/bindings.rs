//! Function-local binding rows used by normalized places and scope events.

use crate::compiler_frontend::hir::ids::{LocalId, RegionId};

use super::events::EventSource;
use super::ids::BindingId;

/// One binding visible to a normalized function problem.
///
/// The HIR local and region are optional because hand-authored problems do not have a source
/// owner. When present they are function-local inspection links, never cross-module semantic
/// identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Binding {
    pub(crate) id: BindingId,
    pub(crate) hir_local: Option<LocalId>,
    pub(crate) region: Option<RegionId>,
    pub(crate) mutable: bool,
    pub(crate) compiler_temporary: bool,
    pub(crate) source: EventSource,
}

impl Binding {
    pub(crate) const fn new(
        id: BindingId,
        hir_local: Option<LocalId>,
        region: Option<RegionId>,
        mutable: bool,
        compiler_temporary: bool,
        source: EventSource,
    ) -> Self {
        Self {
            id,
            hir_local,
            region,
            mutable,
            compiler_temporary,
            source,
        }
    }

    pub(crate) const fn synthetic(id: BindingId) -> Self {
        Self::new(id, None, None, false, false, EventSource::none())
    }
}
