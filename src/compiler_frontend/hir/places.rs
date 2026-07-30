//! HIR memory places.
//!
//! WHAT: canonical memory projections such as locals, fields, and indexed elements.
//! WHY: assignments, loads, copies, and borrow checking need one shared place representation.

use crate::compiler_frontend::hir::expressions::HirExpression;
use crate::compiler_frontend::hir::ids::{FieldId, LocalId};
use crate::compiler_frontend::symbols::string_interning::StringIdRemap;

#[derive(Debug, Clone)]
pub enum HirPlace {
    Local(LocalId),

    Field {
        base: Box<HirPlace>,
        field: FieldId,
    },

    Index {
        base: Box<HirPlace>,
        index: Box<HirExpression>,
    },
}

impl HirPlace {
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        match self {
            Self::Local(_) => {}
            Self::Field { base, .. } => base.remap_string_ids(remap),
            Self::Index { base, index } => {
                base.remap_string_ids(remap);
                index.remap_string_ids(remap);
            }
        }
    }
}
