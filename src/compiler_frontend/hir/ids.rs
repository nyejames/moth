//! Stable HIR ID newtypes.
//!
//! WHAT: dense IDs used to index HIR modules, blocks, locals, expressions, and choices.
//! WHY: HIR facts and side tables refer to semantic objects by ID rather than by AST paths.

macro_rules! define_hir_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(pub u32);
    };
}

define_hir_id!(HirNodeId);
define_hir_id!(HirValueId);
define_hir_id!(BlockId);
define_hir_id!(LocalId);
define_hir_id!(StructId);
define_hir_id!(FieldId);
define_hir_id!(FunctionId);
define_hir_id!(RegionId);
define_hir_id!(ChoiceId);
