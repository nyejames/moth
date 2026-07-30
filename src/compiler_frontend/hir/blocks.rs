//! HIR blocks and locals.
//!
//! WHAT: explicit control-flow blocks plus locals declared inside those blocks.
//! WHY: borrow checking, backend lowering, and diagnostics all operate over block/local IDs.

use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::ids::{BlockId, LocalId, RegionId};
use crate::compiler_frontend::hir::statements::HirStatement;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::string_interning::StringIdRemap;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

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
    pub source_info: Option<SourceLocation>,
}

impl HirBlock {
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        for local in &mut self.locals {
            if let Some(source_info) = &mut local.source_info {
                source_info.remap_string_ids(remap);
            }
        }

        for statement in &mut self.statements {
            statement.remap_string_ids(remap);
        }

        self.terminator.remap_string_ids(remap);
    }
}
