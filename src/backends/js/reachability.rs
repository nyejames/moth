//! HIR CFG reachability adapter for JS function lowering.
//!
//! WHAT: collects the set of HIR blocks reachable from a given entry block,
//! using the JS emitter's block lookup.
//! WHY: JS function emission needs a deterministic, sorted list of reachable
//! blocks to emit in order.
//!
//! This module must not own symbol lookup, source text emission, or identifier
//! generation. Those responsibilities belong to their focused owners.

use crate::backends::js::JsEmitter;
use crate::compiler_frontend::compiler_messages::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::hir::ids::BlockId;
use crate::compiler_frontend::hir::utils::terminator_targets;

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn collect_reachable_blocks(
        &self,
        entry_block: BlockId,
    ) -> Result<Vec<BlockId>, InfrastructureFailure> {
        let mut order = crate::compiler_frontend::hir::utils::collect_reachable_blocks(
            entry_block,
            |block_id| {
                let block = self.block_by_id(block_id)?;
                Ok::<_, InfrastructureFailure>(terminator_targets(&block.terminator))
            },
        )?;
        order.sort_by_key(|block_id| block_id.0);
        Ok(order)
    }
}
