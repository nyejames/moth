//! HIR lexical regions.
//!
//! WHAT: region nodes used by HIR locals, blocks, and later lifetime/ownership analysis.
//! WHY: regions give borrow validation and lowering a stable scope tree.

use crate::compiler_frontend::hir::ids::RegionId;

#[derive(Debug, Clone)]
pub struct HirRegion {
    id: RegionId,
    parent: Option<RegionId>,
}

impl HirRegion {
    pub(crate) fn lexical(id: RegionId, parent: Option<RegionId>) -> Self {
        Self { id, parent }
    }

    pub fn id(&self) -> RegionId {
        self.id
    }

    pub fn parent(&self) -> Option<RegionId> {
        self.parent
    }
}
