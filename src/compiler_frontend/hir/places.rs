//! HIR memory places.
//!
//! WHAT: identifies a local root and its ordered field/index projections.
//! WHY: expression storage owns index values and one flat projection range avoids recursive
//!      place graphs in statements and expression rows.

use crate::compiler_frontend::hir::expression_store::{
    HirConstructionFailure, HirExpressionStore, HirProjection, HirProjectionRange,
};
use crate::compiler_frontend::hir::ids::{FieldId, HirValueId, LocalId};
use crate::compiler_frontend::source::SourceSpan;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HirPlace {
    pub root: LocalId,
    pub projections: HirProjectionRange,
}

impl HirPlace {
    pub const fn local(root: LocalId) -> Self {
        Self {
            root,
            projections: HirProjectionRange::empty(),
        }
    }

    pub(crate) fn with_field(
        self,
        field: FieldId,
        store: &mut HirExpressionStore,
        span: Option<SourceSpan>,
    ) -> Result<Self, HirConstructionFailure> {
        let projections =
            store.extend_projections(self.projections, HirProjection::Field(field), span)?;
        Ok(Self {
            root: self.root,
            projections,
        })
    }

    pub(crate) fn with_index(
        self,
        index: HirValueId,
        store: &mut HirExpressionStore,
        span: Option<SourceSpan>,
    ) -> Result<Self, HirConstructionFailure> {
        let projections =
            store.extend_projections(self.projections, HirProjection::Index(index), span)?;
        Ok(Self {
            root: self.root,
            projections,
        })
    }
}
