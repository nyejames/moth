//! HIR module container.
//!
//! WHAT: the executable/semantic IR payload produced for one Moth module.
//! WHY: backends consume `HirModule` as the stable frontend output after AST lowering and borrow
//! validation.
//!
//! Type identity lives in the frontend `TypeEnvironment` carried beside the module at the
//! compiled-module boundary. HIR nodes store compact frontend `TypeId`s and do not own a separate
//! semantic type table.
//!
//! Non-HIR compiler metadata — warnings and resolved documentation fragments — is extracted by
//! HIR lowering into `HirLoweringMetadata` and assembled into `ModuleCompilerMetadata` on the
//! build-system module payload. `HirModule` carries only executable/semantic HIR state.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::const_facts::HirConstFacts;
use crate::compiler_frontend::hir::constants::HirModuleConst;
use crate::compiler_frontend::hir::expression_store::HirExpressionStore;
use crate::compiler_frontend::hir::failure_facts::HirFunctionFailureFacts;
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::hir_builder::{CatchHandlerRecord, CatchProtectedCall};
use crate::compiler_frontend::hir::hir_side_table::HirSideTable;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId};
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::structs::HirStruct;
use crate::compiler_frontend::hir::utils::for_each_terminator_target_mut;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::semantic_identity::{
    GeneratedFunctionIdentity, ModulePrivateExecutableIdentity, OriginFunctionId,
};
use crate::compiler_frontend::symbols::path_interner::PathIdRemap;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use rustc_hash::FxHashMap;

// -------------------------
//  Choice Layout Metadata
// -------------------------

/// Lowering-local choice layout entry.
///
/// WHY: HIR expressions and backends reference variants by stable `ChoiceId` and flat
/// variant/field indices. Semantic type identity (variant names, payload types) lives in the
/// frontend `TypeEnvironment`; `frontend_type_id` traces this entry back to the canonical type.
#[derive(Debug, Clone)]
pub struct HirChoice {
    /// The stable `ChoiceId` assigned during HIR lowering.
    /// WHY: preserved so diagnostics and backend validation can trust the choice
    ///      layout entry's own identity while walking HIR choices.
    pub id: crate::compiler_frontend::hir::ids::ChoiceId,

    /// Trace to the canonical frontend `TypeId` in `TypeEnvironment`.
    /// WHY: validation and backend checks use this to confirm HIR layout entries
    ///      still map back to canonical frontend type identity.
    pub frontend_type_id: TypeId,

    pub variants: Vec<HirChoiceVariant>,
}

#[derive(Debug, Clone)]
pub struct HirChoiceVariant {
    /// The interned variant name from the frontend `TypeEnvironment`.
    /// WHY: preserved for completeness and future diagnostic rendering.
    ///      Currently not read outside tests.
    #[allow(dead_code)]
    pub name: StringId,
    pub fields: Vec<HirChoiceField>,
}

#[derive(Debug, Clone)]
pub struct HirChoiceField {
    pub name: StringId,
    pub ty: TypeId,
}

// -------------------------
//  HIR Module Container
// -------------------------

#[derive(Debug, Clone)]
pub struct HirModule {
    pub expressions: HirExpressionStore,
    pub blocks: Vec<HirBlock>,
    pub functions: Vec<HirFunction>,
    pub structs: Vec<HirStruct>,
    pub choices: Vec<HirChoice>,
    pub side_table: HirSideTable,

    /// Compiler-synthesised entry point for normal roots.
    pub start_function: Option<FunctionId>,
    /// Classification for every function in the module.
    ///
    /// WHY: backends/builders need explicit semantic role tagging to keep
    /// entry/runtime-template behavior stable across lowering passes.
    pub function_origins: FxHashMap<FunctionId, HirFunctionOrigin>,

    /// Stable origin-to-local-function lookup for directly exported non-generic functions and
    /// receiver methods that lower to local HIR.
    ///
    /// WHAT: records the explicit stable-origin-to-local-function relationship produced while
    /// lowering, excluding private functions and the implicit start.
    /// WHY: public-interface finalization joins borrow summaries to declaration records through
    /// this side table rather than rendered names, paths or declaration order.
    pub function_ids_by_origin: FxHashMap<OriginFunctionId, FunctionId>,
    pub(crate) function_ids_by_private_origin:
        FxHashMap<ModulePrivateExecutableIdentity, FunctionId>,
    pub(crate) function_ids_by_generated: FxHashMap<GeneratedFunctionIdentity, FunctionId>,
    pub(crate) imported_call_summaries: FxHashMap<OriginFunctionId, PublicCallSummary>,
    pub(crate) module_private_call_summaries:
        FxHashMap<ModulePrivateExecutableIdentity, PublicCallSummary>,
    pub(crate) generated_call_summaries: FxHashMap<GeneratedFunctionIdentity, PublicCallSummary>,

    pub module_constants: Vec<HirModuleConst>,

    /// Region tree
    pub regions: Vec<HirRegion>,

    /// Advisory const facts projected from the AST for future optimization.
    ///
    /// WHAT: records which declarations are compile-time constants, their scope,
    ///       source, value kind, and source location.
    /// WHY: provides metadata for later borrow-checker and lowering optimizations
    ///      without changing HIR semantics today.
    pub const_facts: HirConstFacts,

    /// Direct synthetic compile-time interface provenance for every local function.
    ///
    /// WHAT: one immutable read-only provenance fact per local `FunctionId`, including an explicit
    /// empty fact for functions with no synthetic-interface dependencies. The fact is the sorted,
    /// duplicate-free union of all expression provenance lowered from the function body. It is a
    /// read-only fact and does not alter HIR control flow.
    /// WHY: the per-function link-fact lane described in the compiler design overview needs stable,
    /// deterministic direct provenance. HIR validation rejects missing, extra or out-of-range
    /// coverage as `CompilerError`.
    pub function_provenance: FxHashMap<FunctionId, SyntheticInterfaceProvenance>,

    /// Construction-only provenance grouped by the HIR block that lowered each expression.
    ///
    /// A row keyed with `None` preserves provenance lowered before an active block was selected.
    /// The private failure convergence owner finalizes the retained function facts and then
    /// releases this projection, so it is never part of the published executable.
    pub(crate) function_provenance_by_block:
        FxHashMap<(FunctionId, Option<BlockId>), SyntheticInterfaceProvenance>,

    /// Whether the construction block projection has owned the per-function aggregate facts.
    ///
    /// WHAT: set the first time HIR construction supplied block rows and `function_provenance`
    ///       was rebuilt from them.
    /// WHY: an empty projection is ambiguous. When it is construction-owned, finalization must
    ///      rebuild the aggregate to explicit empty facts for every retained function even after
    ///      catch pruning removed the last block row; a directly constructed module that never
    ///      used the projection keeps facts assigned outside it.
    pub(crate) function_provenance_projection_used: bool,

    /// Direct unhandled failure producers and source boundary for every local function.
    /// Summary convergence consumes this read-only projection rather than AST or CFG rescans.
    pub(crate) function_failure_facts: FxHashMap<FunctionId, HirFunctionFailureFacts>,

    /// Scalar calls lowered inside catch-protected work.
    ///
    /// WHAT: each record binds one `Call` statement id to the catch handler that owned the
    ///       failure edge at lowering time. Records cover infallible callees optimistically;
    ///       the private-lane installer skips those whose callee never escapes.
    /// WHY: the installer runs after summary convergence and cannot re-derive protection from
    ///      the HIR alone, so lowering records the route once and the installer acts on it.
    pub(crate) catch_protected_calls: Vec<CatchProtectedCall>,
    /// Every lowered catch handler, including handlers with no protected calls.
    pub(crate) catch_handlers: Vec<CatchHandlerRecord>,
}

impl HirModule {
    pub fn new() -> Self {
        Self {
            expressions: HirExpressionStore::default(),
            blocks: vec![],
            functions: vec![],
            structs: vec![],
            choices: vec![],
            side_table: HirSideTable::default(),
            start_function: None,
            function_origins: FxHashMap::default(),
            function_ids_by_origin: FxHashMap::default(),
            function_ids_by_private_origin: FxHashMap::default(),
            function_ids_by_generated: FxHashMap::default(),
            imported_call_summaries: FxHashMap::default(),
            module_private_call_summaries: FxHashMap::default(),
            generated_call_summaries: FxHashMap::default(),
            module_constants: vec![],
            regions: vec![],
            const_facts: HirConstFacts::default(),
            function_provenance: FxHashMap::default(),
            function_provenance_by_block: FxHashMap::default(),
            function_provenance_projection_used: false,
            function_failure_facts: FxHashMap::default(),
            catch_protected_calls: vec![],
            catch_handlers: vec![],
        }
    }

    pub(crate) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.side_table.remap_path_ids(remap);
        self.const_facts.remap_path_ids(remap);
    }

    /// Rebuild function provenance from retained HIR construction blocks.
    ///
    /// WHAT: recomputes one fact per function as the union of the surviving block rows and reports
    ///       whether the aggregate changed.
    /// WHY: block pruning can remove the last row of a projection that still owns the aggregate,
    ///      so a construction-produced empty projection must settle to explicit empty facts for
    ///      every retained function instead of leaving the pre-pruning aggregate in place. Directly
    ///      constructed modules that never used the projection keep their own facts.
    pub(crate) fn refresh_function_provenance_from_blocks(
        &mut self,
    ) -> Result<bool, CompilerError> {
        if self.function_provenance_by_block.is_empty() && !self.function_provenance_projection_used
        {
            return Ok(false);
        }
        self.function_provenance_projection_used = true;
        let mut provenance_by_function = self
            .functions
            .iter()
            .map(|function| (function.id, SyntheticInterfaceProvenance::empty()))
            .collect::<FxHashMap<_, _>>();
        for ((function_id, _), provenance) in &self.function_provenance_by_block {
            let Some(function_provenance) = provenance_by_function.get_mut(function_id) else {
                return Err(CompilerError::compiler_error(format!(
                    "HIR block provenance references unknown function {function_id:?}"
                )));
            };
            function_provenance.merge(provenance);
        }
        let changed = self.function_provenance != provenance_by_function;
        self.function_provenance = provenance_by_function;
        Ok(changed)
    }

    /// Rebuild final function provenance from retained HIR construction blocks.
    ///
    /// WHAT: rebuilds the aggregate, then replaces the construction-only projection with a fresh
    ///       empty map.
    /// WHY: construction is complete and the projection has no later reader, so dropping the map
    ///      releases its backing allocation instead of retaining capacity for rows that can never
    ///      be added again.
    pub(crate) fn finalize_function_provenance_after_rewrites(
        &mut self,
    ) -> Result<bool, CompilerError> {
        let changed = self.refresh_function_provenance_from_blocks()?;
        self.function_provenance_by_block = FxHashMap::default();
        Ok(changed)
    }

    /// Renumber blocks so `blocks[index].id == BlockId(index)`.
    ///
    /// HIR construction removes unused scaffolding blocks after other blocks already exist,
    /// which leaves gaps. The builder calls this once construction is done, before validation.
    pub(crate) fn compact_block_ids(&mut self) {
        if self
            .blocks
            .iter()
            .enumerate()
            .all(|(index, block)| block.id == BlockId(index as u32))
        {
            return;
        }

        let remap = self
            .blocks
            .iter()
            .enumerate()
            .map(|(index, block)| (block.id, BlockId(index as u32)))
            .collect::<FxHashMap<_, _>>();
        for block in &mut self.blocks {
            block.id = remap[&block.id];
            // Removed blocks have no incoming edges. Any unknown target is left for validation.
            for_each_terminator_target_mut(&mut block.terminator, |target| {
                *target = remap.get(target).copied().unwrap_or(*target);
            });
        }
        for function in &mut self.functions {
            function.entry = remap
                .get(&function.entry)
                .copied()
                .unwrap_or(function.entry);
        }
        for record in &mut self.catch_protected_calls {
            record.handler.block = remap
                .get(&record.handler.block)
                .copied()
                .unwrap_or(record.handler.block);
        }
        for record in &mut self.catch_handlers {
            record.handler.block = remap
                .get(&record.handler.block)
                .copied()
                .unwrap_or(record.handler.block);
        }
        self.function_provenance_by_block = std::mem::take(&mut self.function_provenance_by_block)
            .into_iter()
            .filter_map(|((function_id, block_id), provenance)| {
                let block_id = match block_id {
                    Some(block_id) => remap.get(&block_id).copied().map(Some)?,
                    None => None,
                };
                Some(((function_id, block_id), provenance))
            })
            .collect();
        self.side_table.remap_block_ids(&remap);
    }

    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.expressions.remap_string_ids(remap);

        for choice in &mut self.choices {
            for variant in &mut choice.variants {
                variant.name = remap.get(variant.name);
                for field in &mut variant.fields {
                    field.name = remap.get(field.name);
                }
            }
        }

        // Structural string constants carry interned `Text` handles alongside the
        // executable lane, so they re-bind on the same merge.
        for constant in &mut self.module_constants {
            constant.value.remap_string_ids(remap);
        }
        self.side_table.remap_string_ids(remap);
        self.const_facts.remap_string_ids(remap);
    }

    pub(crate) fn require_start_function(&self, owner: &str) -> Result<FunctionId, CompilerError> {
        self.start_function.ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "{owner} requires a normal module with an implicit start function"
            ))
        })
    }
}
