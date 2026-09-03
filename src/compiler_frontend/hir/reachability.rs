//! Backend-neutral HIR reachability analysis.
//!
//! WHAT: records direct CFG and call facts per HIR function, then builds exact unions from roots
//! selected by build-owned link planning.
//! WHY: later phases need one retained view of executable runtime facts without re-scanning HIR,
//! dependency syntax, or inventing target-specific reachability rules.
//!
//! This is intentionally a syntactic HIR analysis. It does not fold constants, eliminate dead
//! branches, inspect borrow facts, or perform backend lowering.
use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType, SourceLocation};
use crate::compiler_frontend::external_packages::{CallTarget, ExternalFunctionId};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, HirMapOp};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::hir_side_table::HirLocation;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::HirNumericOperands;
use crate::compiler_frontend::hir::reactivity::ReactiveTemplateId;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::{HirAssertionMessageEvaluation, HirTerminator};
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;

use crate::compiler_frontend::paths::module_resources::ResourceId;
use crate::compiler_frontend::semantic_identity::{
    GeneratedFunctionIdentity, ModulePrivateExecutableIdentity, OriginFunctionId,
};
use crate::compiler_frontend::symbols::string_interning::StringIdRemap;

use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

/// Reachable HIR surface from the selected root functions.
///
/// WHY: later phases need both the user-code slice and the external package calls that are
/// actually reachable, but ownership of artifact planning stays outside HIR.
#[derive(Clone, Debug, Default)]
pub(crate) struct HirReachability {
    pub(crate) reachable_cross_module_functions: FxHashSet<OriginFunctionId>,
    pub(crate) reachable_module_private_functions: FxHashSet<ModulePrivateExecutableIdentity>,
    pub(crate) reachable_generated_functions: FxHashSet<GeneratedFunctionIdentity>,
    pub(crate) reachable_external_functions: FxHashSet<ExternalFunctionId>,
    pub(crate) reachable_external_calls: Vec<ReachableExternalCall>,
    pub(crate) reachable_map_uses: Vec<ReachableMapUse>,
    pub(crate) reachable_resource_uses: Vec<ReachableResourceUse>,
    pub(crate) reachable_site_root_uses: Vec<ReachableSiteRootUse>,
    pub(crate) reachable_reactive_templates: Vec<ReachableReactiveTemplateUse>,
    pub(crate) reachable_reactive_sinks: Vec<ReachableReactiveSinkUse>,
    pub(crate) reachable_runtime_casts: Vec<ReachableRuntimeCastUse>,
    pub(crate) reachable_numeric_ops: Vec<ReachableNumericOpUse>,
    pub(crate) reachable_float_statements: Vec<ReachableFloatStatementUse>,
    pub(crate) reachable_assertion_messages: Vec<ReachableAssertionMessageUse>,
    reachable_function_provenance: SyntheticInterfaceProvenance,
    backend_selection: HirBackendSelection,
}

/// Deterministic direct link facts for one base HIR function.
///
/// In addition to the entry block, each row carries the complete direct synthetic-interface
/// provenance retained for its local function.
#[derive(Clone, Debug)]
pub(crate) struct HirFunctionLinkFacts {
    pub(crate) function_id: FunctionId,
    entry_block: BlockId,
    pub(crate) synthetic_interface_provenance: SyntheticInterfaceProvenance,
}

/// Deterministic direct link facts for one reachable block inside a base HIR function.
#[derive(Clone, Debug)]
struct HirBlockLinkFacts {
    block_id: BlockId,
    function_id: FunctionId,
    successor_blocks: Vec<BlockId>,
    direct_facts: HirBlockRuntimeFacts,
}

/// Module-local per-function linking authority produced once after HIR validation.
#[derive(Clone, Debug, Default)]
pub(crate) struct HirModuleLinkFacts {
    functions: Vec<HirFunctionLinkFacts>,
    blocks: Vec<HirBlockLinkFacts>,
}

/// Closed function/block selection derived by build-owned link planning.
///
/// WHY: selected backend modes consume one coherent value, so callers cannot independently alter
/// function and block sets or detach them from the retained per-function facts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct HirBackendSelection {
    functions: FxHashSet<FunctionId>,
    blocks: FxHashSet<BlockId>,
    blocks_by_function: Vec<HirSelectedFunctionBlocks>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HirSelectedFunctionBlocks {
    function_id: FunctionId,
    blocks: Vec<BlockId>,
}

#[derive(Clone, Debug, Default)]
struct HirBlockRuntimeFacts {
    direct_user_calls: Vec<FunctionId>,
    direct_cross_module_calls: Vec<OriginFunctionId>,
    direct_module_private_calls: Vec<ModulePrivateExecutableIdentity>,
    direct_generated_calls: Vec<GeneratedFunctionIdentity>,
    reachable_external_functions: FxHashSet<ExternalFunctionId>,
    reachable_external_calls: Vec<ReachableExternalCall>,
    reachable_map_uses: Vec<ReachableMapUse>,
    reachable_resource_uses: Vec<ReachableResourceUse>,
    reachable_site_root_uses: Vec<ReachableSiteRootUse>,
    reachable_reactive_templates: Vec<ReachableReactiveTemplateUse>,
    reachable_reactive_sinks: Vec<ReachableReactiveSinkUse>,
    reachable_runtime_casts: Vec<ReachableRuntimeCastUse>,
    reachable_numeric_ops: Vec<ReachableNumericOpUse>,
    reachable_float_statements: Vec<ReachableFloatStatementUse>,
    reachable_assertion_messages: Vec<ReachableAssertionMessageUse>,
}

impl HirModuleLinkFacts {
    fn facts_for(&self, function_id: FunctionId) -> Option<&HirFunctionLinkFacts> {
        self.functions
            .binary_search_by_key(&function_id.0, |facts| facts.function_id.0)
            .ok()
            .map(|index| &self.functions[index])
    }

    fn facts_for_block(&self, block_id: BlockId) -> Option<&HirBlockLinkFacts> {
        self.blocks
            .binary_search_by_key(&block_id.0, |facts| facts.block_id.0)
            .ok()
            .map(|index| &self.blocks[index])
    }

    /// Return the validated HIR call targets with their owning function identities.
    ///
    /// WHAT: exposes the existing per-function link-fact call extraction to transient build
    ///       observations that need topology without rescanning HIR or source.
    /// WHY: convergence derives its read-only dependency model from the same validated call
    ///      targets that already own backend reachability facts. The returned vector is a
    ///      construction input, not a second retained call graph.
    pub(crate) fn direct_call_targets(&self) -> Vec<(FunctionId, CallTarget)> {
        let mut targets = Vec::new();
        for block in &self.blocks {
            targets.extend(
                block
                    .direct_facts
                    .direct_user_calls
                    .iter()
                    .copied()
                    .map(|function_id| (block.function_id, CallTarget::Local(function_id))),
            );
            targets.extend(
                block
                    .direct_facts
                    .direct_cross_module_calls
                    .iter()
                    .cloned()
                    .map(|origin| (block.function_id, CallTarget::CrossModule(origin))),
            );
            targets.extend(
                block
                    .direct_facts
                    .direct_module_private_calls
                    .iter()
                    .cloned()
                    .map(|identity| (block.function_id, CallTarget::ModulePrivate(identity))),
            );
            targets.extend(
                block
                    .direct_facts
                    .direct_generated_calls
                    .iter()
                    .cloned()
                    .map(|identity| (block.function_id, CallTarget::Generated(identity))),
            );
        }
        targets
    }

    /// Remap source locations retained for later target diagnostics.
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        for block in &mut self.blocks {
            block.direct_facts.remap_string_ids(remap);
        }
    }
}

impl HirBackendSelection {
    pub(crate) fn contains_function(&self, function_id: FunctionId) -> bool {
        self.functions.contains(&function_id)
    }

    pub(crate) fn blocks(&self) -> &FxHashSet<BlockId> {
        &self.blocks
    }

    pub(crate) fn functions(&self) -> &FxHashSet<FunctionId> {
        &self.functions
    }

    pub(crate) fn function_count(&self) -> usize {
        self.functions().len()
    }

    pub(crate) fn blocks_for_function(&self, function_id: FunctionId) -> Option<&[BlockId]> {
        self.blocks_by_function
            .iter()
            .find(|selection| selection.function_id == function_id)
            .map(|selection| selection.blocks.as_slice())
    }

    /// Validate that this closed selection belongs to the supplied validated HIR module.
    ///
    /// WHY: physical variants may reuse dense IDs, so backends must reject a selection detached
    /// from its owning module before lowering or host-import planning begins.
    pub(crate) fn validate_for_hir(&self, hir: &HirModule) -> Result<(), CompilerError> {
        let index = HirReachabilityIndex::new(hir)?;
        let selected_functions = hir
            .functions
            .iter()
            .filter(|function| self.contains_function(function.id))
            .collect::<Vec<_>>();
        if selected_functions.len() != self.function_count() {
            return Err(hir_reachability_error(
                "Backend selection contains an unknown HIR function",
            ));
        }

        let mut assigned_blocks = FxHashSet::default();
        for function in selected_functions {
            let Some(function_blocks) = self.blocks_for_function(function.id) else {
                return Err(hir_reachability_error(format!(
                    "Backend selection has no block assignment for {:?}",
                    function.id
                )));
            };
            let mut actual_blocks = crate::compiler_frontend::hir::utils::collect_reachable_blocks(
                function.entry,
                |block_id| {
                    let block = index.block_by_id.get(&block_id).copied().ok_or_else(|| {
                        hir_reachability_error(format!(
                            "Backend selection could not resolve HIR block {block_id:?}"
                        ))
                    })?;
                    Ok::<_, CompilerError>(
                        crate::compiler_frontend::hir::utils::terminator_targets(&block.terminator),
                    )
                },
            )?;
            actual_blocks.sort_by_key(|block_id| block_id.0);
            if actual_blocks != function_blocks {
                return Err(hir_reachability_error(format!(
                    "Backend block selection does not match the CFG for function {:?}",
                    function.id
                )));
            }

            for block_id in function_blocks {
                if !assigned_blocks.insert(*block_id) {
                    return Err(hir_reachability_error(format!(
                        "Backend selection assigns HIR block {block_id:?} more than once"
                    )));
                }

                let block = index.block_by_id[block_id];
                for statement in &block.statements {
                    if let HirStatementKind::Call {
                        target: CallTarget::Local(callee),
                        ..
                    } = &statement.kind
                        && !self.contains_function(*callee)
                    {
                        return Err(hir_reachability_error(format!(
                            "Backend selection omits callee {callee:?} reached from function {:?}",
                            function.id
                        )));
                    }
                }
            }
        }

        if assigned_blocks != self.blocks {
            return Err(hir_reachability_error(
                "Backend selection contains an unassigned HIR block",
            ));
        }

        Ok(())
    }
}

/// A reachable map construction or use at the HIR statement or expression that produces it.
///
/// WHY: backend unsupported-feature validation needs to know which map literals and map
///      operations are reachable from entry so it can emit structured diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReachableMapUse {
    pub(crate) kind: ReachableMapUseKind,
    pub(crate) location: SourceLocation,
}
/// One reachable structural resource anchor and its executable owner.
///
/// WHAT: keeps the module-local `ResourceId`, authored source location and owning HIR function
///       together for one resource use.
/// WHY: resource liveness is selected from executable owners, while the resource table retains
///      only the stable origin behind each dense handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReachableResourceUse {
    pub(crate) resource_id: ResourceId,
    pub(crate) owner: FunctionId,
    pub(crate) location: SourceLocation,
}

/// One reachable structural site-root anchor and its executable owner.
///
/// WHAT: keeps the authored source location and owning HIR function for one site-root use without
///       inventing a resource identity.
/// WHY: site-root output is liveness-bearing but has no `ResourceId` to enter the resource union.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReachableSiteRootUse {
    pub(crate) owner: FunctionId,
    pub(crate) location: SourceLocation,
}

/// A reachable assertion failure message and its HIR evaluation fact.
///
/// WHAT: retains the source location and static/runtime classification selected by HIR lowering.
/// WHY: target validation must reject only reachable runtime message construction without
///      reconstructing assertion semantics from the terminator expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReachableAssertionMessageUse {
    pub(crate) evaluation: HirAssertionMessageEvaluation,
    pub(crate) location: SourceLocation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReachableMapUseKind {
    Literal,
    Operation(HirMapOp),
}

/// A reachable external call at the HIR statement that invokes it.
///
/// WHY: backend validation needs the stable function ID for support checks and the exact
/// statement location for user-facing unsupported-backend diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReachableExternalCall {
    pub(crate) function_id: ExternalFunctionId,
    pub(crate) statement_id: HirNodeId,
    pub(crate) location: SourceLocation,
}

/// A reachable reactive template-backed value.
///
/// WHY: unsupported-backend validation needs to reject reachable reactive runtime features even
/// when they are produced inside helper functions rather than directly pushed into the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReachableReactiveTemplateUse {
    pub(crate) template_id: ReactiveTemplateId,
    pub(crate) location: SourceLocation,
}

/// A reachable sink that consumes a reactive template-backed value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReachableReactiveSinkUse {
    pub(crate) kind: ReachableReactiveSinkKind,
    pub(crate) template_id: ReactiveTemplateId,
    pub(crate) location: SourceLocation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReachableReactiveSinkKind {
    RuntimeFragment,
    ExternalCallArgument {
        function_id: ExternalFunctionId,
        argument_index: usize,
    },
}

/// A reachable compiler-owned builtin runtime cast expression or statement.
///
/// WHY: some backends (currently HTML-Wasm) cannot lower runtime casts yet. Recording the cast
///      site in reachability lets backend feature validation report the first reachable unsupported
///      cast without re-scanning HIR expressions locally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReachableRuntimeCastUse {
    pub(crate) location: SourceLocation,
}

/// A reachable compiler-owned checked numeric operation.
///
/// WHY: backends that do not yet implement checked numeric semantics must reject the reachable HIR
///      operation before lowering instead of failing with a backend-internal error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReachableNumericOpUse {
    pub(crate) location: SourceLocation,
}

/// A reachable compiler-owned Float formatting or validation statement.
///
/// WHY: backends that do not yet implement Moth Float formatting or external-Float boundary
///      validation must reject the reachable HIR operation before lowering instead of failing with a
///      backend-internal error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReachableFloatStatementUse {
    pub(crate) kind: ReachableFloatStatementKind,
    pub(crate) location: SourceLocation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReachableFloatStatementKind {
    FormatFloat,
    ValidateFloat,
}

struct HirReachabilityIndex<'hir> {
    hir: &'hir HirModule,
    function_by_id: FxHashMap<FunctionId, &'hir HirFunction>,
    block_by_id: FxHashMap<BlockId, &'hir HirBlock>,
}

struct HirReachabilityContext<'index, 'hir> {
    index: &'index HirReachabilityIndex<'hir>,
    owner_function: FunctionId,
    block_worklist: VecDeque<BlockId>,
    visited_blocks: FxHashSet<BlockId>,
    seen_user_calls: FxHashSet<FunctionId>,
    direct_facts: HirBlockRuntimeFacts,
    block_facts: Vec<HirBlockLinkFacts>,
}

/// Record direct CFG and call facts for every function without selecting an entry root.
pub(crate) fn collect_module_function_link_facts(
    hir: &HirModule,
) -> Result<HirModuleLinkFacts, CompilerError> {
    let index = HirReachabilityIndex::new(hir)?;
    let mut function_ids = hir
        .functions
        .iter()
        .map(|function| function.id)
        .collect::<Vec<_>>();
    function_ids.sort_by_key(|function_id| function_id.0);

    let mut functions = Vec::with_capacity(function_ids.len());
    let mut blocks = Vec::new();
    for function_id in function_ids {
        let function = index
            .function_by_id
            .get(&function_id)
            .copied()
            .ok_or_else(|| {
                hir_reachability_error(format!(
                    "Unknown HIR function id {function_id:?} reached HIR reachability analysis"
                ))
            })?;
        let synthetic_interface_provenance = hir
            .function_provenance
            .get(&function_id)
            .cloned()
            .ok_or_else(|| {
                hir_reachability_error(format!(
                    "HIR function {function_id:?} is missing a synthetic-interface provenance fact"
                ))
            })?;
        let context = HirReachabilityContext::new(&index, function_id, function.entry);
        let function_block_facts = context.collect()?;
        functions.push(HirFunctionLinkFacts {
            function_id,
            entry_block: function.entry,
            synthetic_interface_provenance,
        });
        blocks.extend(function_block_facts);
    }

    blocks.sort_by_key(|facts| facts.block_id.0);

    Ok(HirModuleLinkFacts { functions, blocks })
}

/// Build an exact reachable union from retained per-function facts.
pub(crate) fn collect_reachability_from_function_link_facts(
    function_facts: &HirModuleLinkFacts,
    root_functions: &[FunctionId],
) -> Result<HirReachability, CompilerError> {
    let mut function_worklist = VecDeque::from(root_functions.to_vec());
    let mut block_worklist = VecDeque::new();
    let mut visited_functions = FxHashSet::default();
    let mut visited_function_order = Vec::new();
    let mut visited_blocks = FxHashSet::default();
    let mut blocks_by_function = FxHashMap::<FunctionId, Vec<BlockId>>::default();
    let mut reachability = HirReachability::default();

    while !function_worklist.is_empty() || !block_worklist.is_empty() {
        while let Some(function_id) = function_worklist.pop_front() {
            if !visited_functions.insert(function_id) {
                continue;
            }

            let Some(function) = function_facts.facts_for(function_id) else {
                return Err(hir_reachability_error(format!(
                    "Function link facts are missing HIR function id {function_id:?}"
                )));
            };
            visited_function_order.push(function_id);
            reachability
                .reachable_function_provenance
                .merge(&function.synthetic_interface_provenance);
            block_worklist.push_back(function.entry_block);
        }
        while let Some(block_id) = block_worklist.pop_front() {
            if !visited_blocks.insert(block_id) {
                continue;
            }

            let Some(block) = function_facts.facts_for_block(block_id) else {
                return Err(hir_reachability_error(format!(
                    "Block link facts are missing HIR block id {block_id:?}"
                )));
            };
            if !visited_functions.contains(&block.function_id) {
                return Err(hir_reachability_error(format!(
                    "Block link facts for {block_id:?} were reached before owner {:?}",
                    block.function_id
                )));
            }

            blocks_by_function
                .entry(block.function_id)
                .or_default()
                .push(block_id);
            reachability.merge(&block.direct_facts);

            for called_function in &block.direct_facts.direct_user_calls {
                if !visited_functions.contains(called_function) {
                    function_worklist.push_back(*called_function);
                }
            }
            for successor in &block.successor_blocks {
                if !visited_blocks.contains(successor) {
                    block_worklist.push_back(*successor);
                }
            }
        }
    }

    reachability.backend_selection = HirBackendSelection {
        functions: visited_functions,
        blocks: visited_blocks,
        blocks_by_function: visited_function_order
            .into_iter()
            .map(|function_id| {
                let mut blocks = blocks_by_function.remove(&function_id).unwrap_or_default();
                blocks.sort_by_key(|block_id| block_id.0);
                HirSelectedFunctionBlocks {
                    function_id,
                    blocks,
                }
            })
            .collect(),
    };

    Ok(reachability)
}

impl HirReachability {
    pub(crate) fn backend_selection(&self) -> &HirBackendSelection {
        &self.backend_selection
    }
    /// Provenance union for every local function reached from the selected roots.
    ///
    /// The value is retained separately from block runtime facts so boundary policy can inspect
    /// all reached functions, including helpers with no reachable runtime statements.
    pub(crate) fn reachable_function_provenance(&self) -> &SyntheticInterfaceProvenance {
        &self.reachable_function_provenance
    }

    fn merge(&mut self, direct: &HirBlockRuntimeFacts) {
        self.reachable_cross_module_functions
            .extend(direct.direct_cross_module_calls.iter().cloned());
        self.reachable_module_private_functions
            .extend(direct.direct_module_private_calls.iter().cloned());
        self.reachable_generated_functions
            .extend(direct.direct_generated_calls.iter().cloned());
        self.reachable_external_functions
            .extend(direct.reachable_external_functions.iter().copied());
        self.reachable_external_calls
            .extend(direct.reachable_external_calls.iter().cloned());
        self.reachable_map_uses
            .extend(direct.reachable_map_uses.iter().cloned());
        self.reachable_resource_uses
            .extend(direct.reachable_resource_uses.iter().cloned());
        self.reachable_site_root_uses
            .extend(direct.reachable_site_root_uses.iter().cloned());
        self.reachable_reactive_templates
            .extend(direct.reachable_reactive_templates.iter().cloned());
        self.reachable_reactive_sinks
            .extend(direct.reachable_reactive_sinks.iter().cloned());
        self.reachable_runtime_casts
            .extend(direct.reachable_runtime_casts.iter().cloned());
        self.reachable_numeric_ops
            .extend(direct.reachable_numeric_ops.iter().cloned());
        self.reachable_float_statements
            .extend(direct.reachable_float_statements.iter().cloned());
        self.reachable_assertion_messages
            .extend(direct.reachable_assertion_messages.iter().cloned());
    }
}

impl HirBlockRuntimeFacts {
    fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        for call in &mut self.reachable_external_calls {
            call.location.remap_string_ids(remap);
        }
        for map_use in &mut self.reachable_map_uses {
            map_use.location.remap_string_ids(remap);
        }
        for resource_use in &mut self.reachable_resource_uses {
            resource_use.location.remap_string_ids(remap);
        }
        for site_root_use in &mut self.reachable_site_root_uses {
            site_root_use.location.remap_string_ids(remap);
        }
        for template in &mut self.reachable_reactive_templates {
            template.location.remap_string_ids(remap);
        }
        for sink in &mut self.reachable_reactive_sinks {
            sink.location.remap_string_ids(remap);
        }
        for runtime_cast in &mut self.reachable_runtime_casts {
            runtime_cast.location.remap_string_ids(remap);
        }
        for numeric_op in &mut self.reachable_numeric_ops {
            numeric_op.location.remap_string_ids(remap);
        }
        for float_statement in &mut self.reachable_float_statements {
            float_statement.location.remap_string_ids(remap);
        }
        for assertion_message in &mut self.reachable_assertion_messages {
            assertion_message.location.remap_string_ids(remap);
        }
    }
}

impl<'hir> HirReachabilityIndex<'hir> {
    fn new(hir: &'hir HirModule) -> Result<Self, CompilerError> {
        Ok(Self {
            hir,
            function_by_id: build_function_map(hir)?,
            block_by_id: build_block_map(hir)?,
        })
    }
}

impl<'index, 'hir> HirReachabilityContext<'index, 'hir> {
    fn new(
        index: &'index HirReachabilityIndex<'hir>,
        owner_function: FunctionId,
        entry_block: BlockId,
    ) -> Self {
        let mut block_worklist = VecDeque::new();
        block_worklist.push_back(entry_block);

        Self {
            index,
            owner_function,
            block_worklist,
            visited_blocks: FxHashSet::default(),
            seen_user_calls: FxHashSet::default(),
            direct_facts: HirBlockRuntimeFacts::default(),
            block_facts: Vec::new(),
        }
    }

    fn collect(mut self) -> Result<Vec<HirBlockLinkFacts>, CompilerError> {
        while let Some(block_id) = self.block_worklist.pop_front() {
            self.visit_block(block_id)?;
        }

        Ok(self.block_facts)
    }

    fn visit_block(&mut self, block_id: BlockId) -> Result<(), CompilerError> {
        if !self.visited_blocks.insert(block_id) {
            return Ok(());
        }

        let Some(block) = self.index.block_by_id.get(&block_id).copied() else {
            return Err(hir_reachability_error(format!(
                "Unknown HIR block id {block_id:?} reached HIR reachability analysis"
            )));
        };

        self.visit_block_statements(block);
        self.collect_runtime_feature_uses_from_terminator(block);
        let successor_blocks = terminator_successors(&block.terminator)?;
        let direct_facts = std::mem::take(&mut self.direct_facts);
        self.block_facts.push(HirBlockLinkFacts {
            block_id,
            function_id: self.owner_function,
            successor_blocks: successor_blocks.clone(),
            direct_facts,
        });

        for successor in successor_blocks {
            self.enqueue_block(successor);
        }

        Ok(())
    }

    fn visit_block_statements(&mut self, block: &HirBlock) {
        // HIR lowering flattens calls into statements; expression trees intentionally do not
        // carry call targets. Keep the reachability boundary here unless HIR gains a call
        // expression variant in a later design.
        for statement in &block.statements {
            self.collect_runtime_feature_uses_from_statement(statement);

            let HirStatementKind::Call { target, .. } = &statement.kind else {
                continue;
            };

            match target {
                CallTarget::Local(function_id) => {
                    if self.seen_user_calls.insert(*function_id) {
                        self.direct_facts.direct_user_calls.push(*function_id);
                    }
                }
                CallTarget::CrossModule(origin) => {
                    if !self.direct_facts.direct_cross_module_calls.contains(origin) {
                        self.direct_facts
                            .direct_cross_module_calls
                            .push(origin.clone());
                    }
                }
                CallTarget::ModulePrivate(identity) => {
                    if !self
                        .direct_facts
                        .direct_module_private_calls
                        .contains(identity)
                    {
                        self.direct_facts
                            .direct_module_private_calls
                            .push(identity.clone());
                    }
                }
                CallTarget::Generated(identity) => {
                    if !self.direct_facts.direct_generated_calls.contains(identity) {
                        self.direct_facts
                            .direct_generated_calls
                            .push(identity.clone());
                    }
                }
                CallTarget::External(function_id) => {
                    self.direct_facts
                        .reachable_external_functions
                        .insert(*function_id);
                    self.direct_facts
                        .reachable_external_calls
                        .push(ReachableExternalCall {
                            function_id: *function_id,
                            statement_id: statement.id,
                            location: statement.location.clone(),
                        });
                }
            }
        }
    }

    fn collect_runtime_feature_uses_from_statement(&mut self, statement: &HirStatement) {
        match &statement.kind {
            // Expressions and calls: recurse into sub-expressions only.
            HirStatementKind::Assign { value, .. } | HirStatementKind::Expr(value) => {
                self.collect_runtime_feature_uses_from_expression(value, &statement.location);
            }

            HirStatementKind::Call { target, args, .. } => {
                for (argument_index, arg) in args.iter().enumerate() {
                    if let CallTarget::External(function_id) = target {
                        self.collect_reactive_sink_from_expression(
                            ReachableReactiveSinkKind::ExternalCallArgument {
                                function_id: *function_id,
                                argument_index,
                            },
                            arg,
                            &statement.location,
                        );
                    }
                    self.collect_runtime_feature_uses_from_expression(arg, &statement.location);
                }
            }

            HirStatementKind::PushRuntimeFragment { value, .. } => {
                self.collect_reactive_sink_from_expression(
                    ReachableReactiveSinkKind::RuntimeFragment,
                    value,
                    &statement.location,
                );
                self.collect_runtime_feature_uses_from_expression(value, &statement.location);
            }

            // Map operations: record the use, then recurse into receiver and args.
            HirStatementKind::MapOp {
                op, receiver, args, ..
            } => {
                self.direct_facts.reachable_map_uses.push(ReachableMapUse {
                    kind: ReachableMapUseKind::Operation(*op),
                    location: statement.location.clone(),
                });
                self.collect_runtime_feature_uses_from_expression(receiver, &statement.location);
                for arg in args {
                    self.collect_runtime_feature_uses_from_expression(arg, &statement.location);
                }
            }

            HirStatementKind::Drop(_) => {}

            HirStatementKind::NumericOp { operands, .. } => {
                self.direct_facts
                    .reachable_numeric_ops
                    .push(ReachableNumericOpUse {
                        location: statement.location.clone(),
                    });

                match operands {
                    HirNumericOperands::Unary { operand } => {
                        self.collect_runtime_feature_uses_from_expression(
                            operand,
                            &statement.location,
                        );
                    }
                    HirNumericOperands::Binary { left, right } => {
                        self.collect_runtime_feature_uses_from_expression(
                            left,
                            &statement.location,
                        );
                        self.collect_runtime_feature_uses_from_expression(
                            right,
                            &statement.location,
                        );
                    }
                }
            }

            HirStatementKind::CastOp { source, .. } => {
                self.direct_facts
                    .reachable_runtime_casts
                    .push(ReachableRuntimeCastUse {
                        location: statement.location.clone(),
                    });
                self.collect_runtime_feature_uses_from_expression(source, &statement.location);
            }

            HirStatementKind::FormatFloat { source, .. } => {
                self.direct_facts
                    .reachable_float_statements
                    .push(ReachableFloatStatementUse {
                        kind: ReachableFloatStatementKind::FormatFloat,
                        location: statement.location.clone(),
                    });
                self.collect_runtime_feature_uses_from_expression(source, &statement.location);
            }

            HirStatementKind::ValidateFloat { source, .. } => {
                self.direct_facts
                    .reachable_float_statements
                    .push(ReachableFloatStatementUse {
                        kind: ReachableFloatStatementKind::ValidateFloat,
                        location: statement.location.clone(),
                    });
                self.collect_runtime_feature_uses_from_expression(source, &statement.location);
            }
        }
    }

    fn collect_runtime_feature_uses_from_terminator(&mut self, block: &HirBlock) {
        let fallback_location = self
            .index
            .hir
            .side_table
            .hir_source_location_for_hir(HirLocation::Terminator(block.id))
            .cloned()
            .unwrap_or_default();

        match &block.terminator {
            // Terminators that carry a sub-expression to inspect.
            HirTerminator::If { condition, .. } => {
                self.collect_runtime_feature_uses_from_expression(condition, &fallback_location);
            }

            HirTerminator::FallibleBranch { result, .. } => {
                self.collect_runtime_feature_uses_from_expression(result, &fallback_location);
            }

            HirTerminator::Match { scrutinee, .. } => {
                self.collect_runtime_feature_uses_from_expression(scrutinee, &fallback_location);
            }

            // Terminators that return a value.
            HirTerminator::Return(value)
            | HirTerminator::ReturnSuccess(value)
            | HirTerminator::ReturnError(value) => {
                self.collect_runtime_feature_uses_from_expression(value, &fallback_location);
            }

            // Terminators with no sub-expressions to inspect.
            HirTerminator::Jump { .. }
            | HirTerminator::Break { .. }
            | HirTerminator::Continue { .. }
            | HirTerminator::RuntimeFailure { .. }
            | HirTerminator::Uninitialized => {}

            HirTerminator::AssertFailure {
                message,
                message_evaluation,
            } => {
                let location = self
                    .index
                    .hir
                    .side_table
                    .value_source_location(message.id)
                    .cloned()
                    .unwrap_or(fallback_location.clone());
                self.direct_facts
                    .reachable_assertion_messages
                    .push(ReachableAssertionMessageUse {
                        evaluation: *message_evaluation,
                        location,
                    });
                self.collect_runtime_feature_uses_from_expression(message, &fallback_location);
            }
        }
    }

    fn collect_runtime_feature_uses_from_expression(
        &mut self,
        expression: &HirExpression,
        fallback_location: &SourceLocation,
    ) {
        let expression_location = self
            .index
            .hir
            .side_table
            .value_source_location(expression.id)
            .unwrap_or(fallback_location)
            .clone();

        // Only templates with actual runtime subscriptions are unsupported reactive runtime
        // features. Plain runtime templates with variable interpolations are snapshots, not live
        // reactive values, and are rejected by other backend-specific checks if needed.
        if let Some(template) = self
            .index
            .hir
            .side_table
            .reactive_template_for_value(expression.id)
            && !template.dependencies.is_empty()
        {
            self.direct_facts
                .reachable_reactive_templates
                .push(ReachableReactiveTemplateUse {
                    template_id: template.id,
                    location: expression_location.clone(),
                });
        }

        match &expression.kind {
            // Map literals.
            HirExpressionKind::MapLiteral(entries) => {
                self.direct_facts.reachable_map_uses.push(ReachableMapUse {
                    kind: ReachableMapUseKind::Literal,
                    location: expression_location.clone(),
                });
                for entry in entries {
                    self.collect_runtime_feature_uses_from_expression(
                        &entry.key,
                        &expression_location,
                    );
                    self.collect_runtime_feature_uses_from_expression(
                        &entry.value,
                        &expression_location,
                    );
                }
            }

            // Composite expressions: recurse into sub-expressions.
            HirExpressionKind::BinOp { left, right, .. } => {
                self.collect_runtime_feature_uses_from_expression(left, &expression_location);
                self.collect_runtime_feature_uses_from_expression(right, &expression_location);
            }

            HirExpressionKind::Cast {
                source: operand, ..
            } => {
                self.direct_facts
                    .reachable_runtime_casts
                    .push(ReachableRuntimeCastUse {
                        location: expression_location.clone(),
                    });
                self.collect_runtime_feature_uses_from_expression(operand, &expression_location);
            }

            HirExpressionKind::UnaryOp { operand, .. }
            | HirExpressionKind::FallibleUnwrapSuccess { result: operand }
            | HirExpressionKind::FallibleUnwrapError { result: operand }
            | HirExpressionKind::VariantPayloadGet {
                source: operand, ..
            } => {
                self.collect_runtime_feature_uses_from_expression(operand, &expression_location);
            }

            HirExpressionKind::StructConstruct { fields, .. } => {
                for (_, value) in fields {
                    self.collect_runtime_feature_uses_from_expression(value, &expression_location);
                }
            }

            HirExpressionKind::Collection(elements)
            | HirExpressionKind::TupleConstruct { elements } => {
                for element in elements {
                    self.collect_runtime_feature_uses_from_expression(
                        element,
                        &expression_location,
                    );
                }
            }

            HirExpressionKind::Range { start, end } => {
                self.collect_runtime_feature_uses_from_expression(start, &expression_location);
                self.collect_runtime_feature_uses_from_expression(end, &expression_location);
            }

            HirExpressionKind::TupleGet { tuple, .. } => {
                self.collect_runtime_feature_uses_from_expression(tuple, &expression_location);
            }

            HirExpressionKind::VariantConstruct { fields, .. } => {
                for field in fields {
                    self.collect_runtime_feature_uses_from_expression(
                        &field.value,
                        &expression_location,
                    );
                }
            }
            HirExpressionKind::StructuralString { pieces } => {
                for piece in pieces {
                    match piece {
                        ConstStringPiece::Text(_) => {}
                        ConstStringPiece::Resource(resource_id) => {
                            self.direct_facts
                                .reachable_resource_uses
                                .push(ReachableResourceUse {
                                    resource_id: *resource_id,
                                    owner: self.owner_function,
                                    location: expression_location.clone(),
                                });
                        }
                        ConstStringPiece::SiteRoot => {
                            self.direct_facts
                                .reachable_site_root_uses
                                .push(ReachableSiteRootUse {
                                    owner: self.owner_function,
                                    location: expression_location.clone(),
                                });
                        }
                    }
                }
            }

            // Leaf values: nothing to record.
            HirExpressionKind::Int(_)
            | HirExpressionKind::Float(_)
            | HirExpressionKind::Bool(_)
            | HirExpressionKind::Char(_)
            | HirExpressionKind::StringLiteral(_)
            | HirExpressionKind::Load(_)
            | HirExpressionKind::Copy(_) => {}
        }
    }

    fn collect_reactive_sink_from_expression(
        &mut self,
        kind: ReachableReactiveSinkKind,
        expression: &HirExpression,
        fallback_location: &SourceLocation,
    ) {
        let Some(template) = self
            .index
            .hir
            .side_table
            .reactive_template_for_value(expression.id)
            .filter(|template| template.has_runtime_reactive_dependency())
        else {
            return;
        };

        let location = self
            .index
            .hir
            .side_table
            .value_source_location(expression.id)
            .unwrap_or(fallback_location)
            .clone();

        self.direct_facts
            .reachable_reactive_sinks
            .push(ReachableReactiveSinkUse {
                kind,
                template_id: template.id,
                location,
            });
    }

    fn enqueue_block(&mut self, block_id: BlockId) {
        if !self.visited_blocks.contains(&block_id) {
            self.block_worklist.push_back(block_id);
        }
    }
}

fn terminator_successors(terminator: &HirTerminator) -> Result<Vec<BlockId>, CompilerError> {
    let successors = match terminator {
        HirTerminator::Jump { target, .. } => vec![*target],
        HirTerminator::If {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        HirTerminator::FallibleBranch {
            success_block,
            error_block,
            ..
        } => vec![*success_block, *error_block],
        HirTerminator::Match { arms, .. } => arms.iter().map(|arm| arm.body).collect(),
        HirTerminator::Break { target } | HirTerminator::Continue { target } => vec![*target],
        HirTerminator::Return(_)
        | HirTerminator::ReturnSuccess(_)
        | HirTerminator::ReturnError(_)
        | HirTerminator::RuntimeFailure { .. }
        | HirTerminator::AssertFailure { .. } => Vec::new(),
        HirTerminator::Uninitialized => {
            return Err(hir_reachability_error(
                "Uninitialized HIR terminator reached HIR reachability analysis",
            ));
        }
    };

    Ok(successors)
}

fn build_function_map(
    hir: &HirModule,
) -> Result<FxHashMap<FunctionId, &HirFunction>, CompilerError> {
    let mut function_by_id = FxHashMap::default();

    for function in &hir.functions {
        if function_by_id.insert(function.id, function).is_some() {
            return Err(hir_reachability_error(format!(
                "Duplicate HIR function id {:?} reached HIR reachability analysis",
                function.id
            )));
        }
    }

    Ok(function_by_id)
}

fn build_block_map(hir: &HirModule) -> Result<FxHashMap<BlockId, &HirBlock>, CompilerError> {
    let mut block_by_id = FxHashMap::default();

    for block in &hir.blocks {
        if block_by_id.insert(block.id, block).is_some() {
            return Err(hir_reachability_error(format!(
                "Duplicate HIR block id {:?} reached HIR reachability analysis",
                block.id
            )));
        }
    }

    Ok(block_by_id)
}

fn hir_reachability_error(message: impl Into<String>) -> CompilerError {
    CompilerError::new(
        message,
        SourceLocation::default(),
        ErrorType::HirTransformation,
    )
}
