//! HIR blocks and locals.
//!
//! WHAT: explicit control-flow blocks plus locals declared inside those blocks.
//! WHY: borrow checking, backend lowering, and diagnostics all operate over block/local IDs.

use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::ids::{BlockId, LocalId, RegionId};
use crate::compiler_frontend::hir::statements::HirStatement;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::SourceSpan;
#[derive(Debug, Clone)]
pub struct HirBlock {
    pub id: BlockId,
    pub region: RegionId,

    /// All locals declared within this block.
    pub locals: Vec<HirLocal>,

    pub statements: Vec<HirStatement>,
    pub terminator: HirTerminator,
}

#[derive(Debug, Clone)]
pub struct HirLocal {
    pub id: LocalId,
    pub ty: TypeId,
    pub mutable: bool,
    pub region: RegionId,
    /// Exact authored declaration/binding span; compiler-generated locals are span-free.
    pub span: Option<SourceSpan>,
}
