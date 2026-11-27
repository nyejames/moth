//! Phase B constraint-fact convergence over final HIR.
//!
//! WHAT: after private failure lanes have been installed and summaries refreshed, widens the
//!       three post-lane bits (`uses_fast_semantics`, `may_trap`, `may_fail`) on every base and
//!       live generated-sidecar function, then writes one final value back to the base report,
//!       sidecar reports, generated transaction summaries and HIR leaf maps.
//! WHY: trap/fail facts depend on lane delivery: a pre-lane `Trap` and a post-lane recoverable
//!      `Invoke` are different facts. This pass is the only writer of the bits, runs once outside
//!      the escape fixed point and never calls `check_borrows`. Provider-owned leaves are
//!      immutable inputs: this pass reads their published summaries and widens only for nodes it
//!      owns.
//!
//! The worklist is deterministic FIFO over base functions (ascending `FunctionId`), then each
//! completed sidecar in generated-identity order, then its functions in `FunctionId` order.
//! Reverse edges cover every call target in a function, not only reachable ones, because a callee
//! `may_fail` widening can make a previously dead handler reachable. Every node's bits only widen,
//! so the worklist terminates.

use std::collections::VecDeque;

use rustc_hash::FxHashMap;

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalPackageRegistry};
use crate::compiler_frontend::hir::constraint_facts::{
    ConstraintCalleeFacts, ConstraintSummaryBits, analyze_function, function_call_targets,
};
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::module_compilation::generated::convergence::update_generated_summary;
use crate::compiler_frontend::module_compilation::generated::transaction::GeneratedFunctionTransaction;
use crate::compiler_frontend::semantic_identity::{
    GeneratedFunctionIdentity, ModulePrivateExecutableIdentity, OriginFunctionId,
};

/// One function node in the phase B worklist.
#[derive(Clone, Debug)]
struct ConstraintNode {
    site: ConstraintNodeSite,
    function: FunctionId,
}

#[derive(Clone, Debug)]
enum ConstraintNodeSite {
    Base,
    Sidecar(GeneratedFunctionIdentity),
}

/// Stable target-node lookups shared by reverse edges and callee evidence.
#[derive(Debug, Default)]
struct NodeIndex {
    base_functions: FxHashMap<FunctionId, usize>,
    base_public_origins: FxHashMap<OriginFunctionId, usize>,
    base_private_identities: FxHashMap<ModulePrivateExecutableIdentity, usize>,
    sidecar_functions: FxHashMap<GeneratedFunctionIdentity, FxHashMap<FunctionId, usize>>,
    sidecar_private_identities:
        FxHashMap<GeneratedFunctionIdentity, FxHashMap<ModulePrivateExecutableIdentity, usize>>,
    generated_roots: FxHashMap<GeneratedFunctionIdentity, usize>,
}

impl NodeIndex {
    /// Resolve a direct call target to an owned node. `None` means an immutable leaf: a provider
    /// summary or an external contract.
    fn target_node(&self, site: &ConstraintNodeSite, target: &CallTarget) -> Option<usize> {
        match target {
            CallTarget::Local(function) => match site {
                ConstraintNodeSite::Base => self.base_functions.get(function).copied(),
                ConstraintNodeSite::Sidecar(identity) => self
                    .sidecar_functions
                    .get(identity)
                    .and_then(|functions| functions.get(function))
                    .copied(),
            },
            CallTarget::CrossModule(origin) => self.base_public_origins.get(origin).copied(),
            CallTarget::ModulePrivate(identity) => {
                let sidecar_owned = match site {
                    ConstraintNodeSite::Base => None,
                    ConstraintNodeSite::Sidecar(sidecar) => self
                        .sidecar_private_identities
                        .get(sidecar)
                        .and_then(|identities| identities.get(identity))
                        .copied(),
                };
                sidecar_owned.or_else(|| self.base_private_identities.get(identity).copied())
            }
            CallTarget::Generated(identity) => self.generated_roots.get(identity).copied(),
            CallTarget::External(_) => None,
        }
    }
}

/// Run phase B and write the final bits into every owning report and leaf map.
///
/// WHAT: the single post-lane constraint convergence entry point for one base module and its
///       completed generated sidecars.
/// WHY: callers must run this exactly once, after lane installation/pruning and before
///      `retain_live_call_summaries`/`install_exact_concrete_call_summaries`, so every later
///      consumer reads one final summary value. It owns all bit mutation; no borrow check may be
///      rebuilt afterwards.
pub(crate) fn converge_constraint_summaries(
    base_hir: &mut HirModule,
    base_report: &mut BorrowCheckReport,
    transaction: &mut GeneratedFunctionTransaction<'_>,
    external_registry: &ExternalPackageRegistry,
) -> Result<(), InfrastructureFailure> {
    let mut nodes = Vec::new();
    let mut bits = Vec::new();
    let mut index = NodeIndex::default();

    build_base_nodes(base_hir, base_report, &mut nodes, &mut bits, &mut index)?;
    let sidecar_identities = build_sidecar_nodes(transaction, &mut nodes, &mut bits, &mut index)?;

    let callers = build_reverse_edges(base_hir, transaction, &nodes, &index);
    run_worklist(
        base_hir,
        transaction,
        external_registry,
        &nodes,
        &index,
        &callers,
        &mut bits,
    );

    write_back_base_summaries(base_hir, base_report, &nodes, &index, &bits)?;
    write_back_sidecar_summaries(transaction, &sidecar_identities, &index, &bits)?;
    Ok(())
}

fn build_base_nodes(
    base_hir: &HirModule,
    base_report: &BorrowCheckReport,
    nodes: &mut Vec<ConstraintNode>,
    bits: &mut Vec<ConstraintSummaryBits>,
    index: &mut NodeIndex,
) -> Result<(), InfrastructureFailure> {
    let mut function_ids = base_hir
        .functions
        .iter()
        .map(|function| function.id)
        .collect::<Vec<_>>();
    function_ids.sort_unstable_by_key(|function| function.0);
    for function in function_ids {
        let summary = base_report
            .analysis
            .public_call_summaries
            .get(&function)
            .ok_or_else(|| missing_summary("base", function))?;
        let node_id = nodes.len();
        nodes.push(ConstraintNode {
            site: ConstraintNodeSite::Base,
            function,
        });
        bits.push(ConstraintSummaryBits::from_summary(summary));
        index.base_functions.insert(function, node_id);
    }

    for (origin, function) in &base_hir.function_ids_by_origin {
        if let Some(node) = index.base_functions.get(function).copied() {
            index.base_public_origins.insert(origin.clone(), node);
        }
    }
    for (identity, function) in &base_hir.function_ids_by_private_origin {
        if let Some(node) = index.base_functions.get(function).copied() {
            index.base_private_identities.insert(identity.clone(), node);
        }
    }
    Ok(())
}

fn build_sidecar_nodes(
    transaction: &mut GeneratedFunctionTransaction<'_>,
    nodes: &mut Vec<ConstraintNode>,
    bits: &mut Vec<ConstraintSummaryBits>,
    index: &mut NodeIndex,
) -> Result<Vec<GeneratedFunctionIdentity>, InfrastructureFailure> {
    let mut identities = transaction
        .completed_link_facts()
        .map(|(identity, _)| identity.clone())
        .collect::<Vec<_>>();
    identities.sort();
    identities.dedup();

    for identity in &identities {
        let sidecar = transaction.sidecar_mut(identity);
        let hir = &sidecar.module.executable.hir;
        let report = &sidecar.module.executable.borrow_analysis;
        let mut function_ids = hir
            .functions
            .iter()
            .map(|function| function.id)
            .collect::<Vec<_>>();
        function_ids.sort_unstable_by_key(|function| function.0);

        let mut function_nodes = FxHashMap::default();
        for function in function_ids {
            let summary = report
                .analysis
                .public_call_summaries
                .get(&function)
                .ok_or_else(|| missing_summary("generated sidecar", function))?;
            let node_id = nodes.len();
            nodes.push(ConstraintNode {
                site: ConstraintNodeSite::Sidecar(identity.clone()),
                function,
            });
            bits.push(ConstraintSummaryBits::from_summary(summary));
            function_nodes.insert(function, node_id);
        }

        for (private_identity, function) in &hir.function_ids_by_private_origin {
            if let Some(node) = function_nodes.get(function).copied() {
                index
                    .sidecar_private_identities
                    .entry(identity.clone())
                    .or_default()
                    .insert(private_identity.clone(), node);
            }
        }
        for (generated_identity, function) in &hir.function_ids_by_generated {
            if let Some(node) = function_nodes.get(function).copied() {
                index
                    .generated_roots
                    .insert(generated_identity.clone(), node);
            }
        }
        index
            .sidecar_functions
            .insert(identity.clone(), function_nodes);
    }
    Ok(identities)
}

/// Build reverse call edges over every direct call target in every node.
///
/// WHAT: duplicate-free, ascending caller lists.
/// WHY: all-call edges are required because a callee `may_fail` widening can make a previously
///      dead handler reachable and add fast/trap facts in that handler.
fn build_reverse_edges(
    base_hir: &HirModule,
    transaction: &mut GeneratedFunctionTransaction<'_>,
    nodes: &[ConstraintNode],
    index: &NodeIndex,
) -> Vec<Vec<usize>> {
    let mut callers = vec![Vec::new(); nodes.len()];
    for (node_id, node) in nodes.iter().enumerate() {
        let targets = match &node.site {
            ConstraintNodeSite::Base => function_call_targets(base_hir, node.function),
            ConstraintNodeSite::Sidecar(identity) => {
                let sidecar = transaction.sidecar_mut(identity);
                function_call_targets(&sidecar.module.executable.hir, node.function)
            }
        };
        for target in targets {
            if let Some(callee) = index.target_node(&node.site, &target) {
                callers[callee].push(node_id);
            }
        }
    }
    for caller_list in &mut callers {
        caller_list.sort_unstable();
        caller_list.dedup();
    }
    callers
}

fn run_worklist(
    base_hir: &HirModule,
    transaction: &mut GeneratedFunctionTransaction<'_>,
    external_registry: &ExternalPackageRegistry,
    nodes: &[ConstraintNode],
    index: &NodeIndex,
    callers: &[Vec<usize>],
    bits: &mut [ConstraintSummaryBits],
) {
    let mut queue = VecDeque::from_iter(0..nodes.len());
    let mut queued = vec![true; nodes.len()];

    while let Some(node_id) = queue.pop_front() {
        queued[node_id] = false;
        let node = &nodes[node_id];
        let analysis = match &node.site {
            ConstraintNodeSite::Base => analyze_function(base_hir, node.function, |target| {
                resolve_callee(target, &node.site, base_hir, index, bits, external_registry)
            }),
            ConstraintNodeSite::Sidecar(identity) => {
                let sidecar = transaction.sidecar_mut(identity);
                let hir = &sidecar.module.executable.hir;
                analyze_function(hir, node.function, |target| {
                    resolve_callee(target, &node.site, hir, index, bits, external_registry)
                })
            }
        };

        let widened = bits[node_id].union(analysis.bits);
        if widened == bits[node_id] {
            continue;
        }
        bits[node_id] = widened;
        for caller in &callers[node_id] {
            if !queued[*caller] {
                queued[*caller] = true;
                queue.push_back(*caller);
            }
        }
    }
}

fn resolve_callee(
    target: &CallTarget,
    site: &ConstraintNodeSite,
    current_hir: &HirModule,
    index: &NodeIndex,
    bits: &[ConstraintSummaryBits],
    external_registry: &ExternalPackageRegistry,
) -> ConstraintCalleeFacts {
    if let Some(node) = index.target_node(site, target) {
        return ConstraintCalleeFacts::from_bits(bits[node]);
    }

    match target {
        CallTarget::External(function) => external_callee_facts(external_registry, *function),
        CallTarget::CrossModule(origin) => current_hir
            .imported_call_summaries
            .get(origin)
            .map(|summary| {
                ConstraintCalleeFacts::from_bits(ConstraintSummaryBits::from_summary(summary))
            })
            .unwrap_or_else(ConstraintCalleeFacts::unknown_external),
        CallTarget::ModulePrivate(identity) => current_hir
            .module_private_call_summaries
            .get(identity)
            .map(|summary| {
                ConstraintCalleeFacts::from_bits(ConstraintSummaryBits::from_summary(summary))
            })
            .unwrap_or_else(ConstraintCalleeFacts::unknown_external),
        CallTarget::Generated(identity) => current_hir
            .generated_call_summaries
            .get(identity.as_ref())
            .map(|summary| {
                ConstraintCalleeFacts::from_bits(ConstraintSummaryBits::from_summary(summary))
            })
            .unwrap_or_else(ConstraintCalleeFacts::unknown_external),
        CallTarget::Local(_) => {
            // Local, owned targets always resolve through `target_node`; a miss is a broken node
            // model rather than an unknown provider leaf.
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "constraint convergence lost a local call target node"
            );
        }
    }
}

fn external_callee_facts(
    external_registry: &ExternalPackageRegistry,
    function: crate::compiler_frontend::external_packages::ExternalFunctionId,
) -> ConstraintCalleeFacts {
    match external_registry.get_function_by_id(function) {
        Some(definition) => {
            ConstraintCalleeFacts::from_external_contract(definition.numeric_contract)
        }
        // An unregistered callable cannot certify absence, so it is unknown evidence too.
        None => ConstraintCalleeFacts::unknown_external(),
    }
}

/// Write the base module's final bits into its report and owned leaf maps.
fn write_back_base_summaries(
    base_hir: &mut HirModule,
    base_report: &mut BorrowCheckReport,
    nodes: &[ConstraintNode],
    index: &NodeIndex,
    bits: &[ConstraintSummaryBits],
) -> Result<(), InfrastructureFailure> {
    for (node_id, node) in nodes.iter().enumerate() {
        if !matches!(node.site, ConstraintNodeSite::Base) {
            continue;
        }
        let summary = base_report
            .analysis
            .public_call_summaries
            .get_mut(&node.function)
            .ok_or_else(|| missing_summary("base", node.function))?;
        bits[node_id].apply_to(summary);
    }
    sync_leaf_maps(base_hir, &ConstraintNodeSite::Base, index, bits);
    Ok(())
}

/// Write every completed sidecar's final bits into its report, leaf maps and published summary.
fn write_back_sidecar_summaries(
    transaction: &mut GeneratedFunctionTransaction<'_>,
    sidecar_identities: &[GeneratedFunctionIdentity],
    index: &NodeIndex,
    bits: &[ConstraintSummaryBits],
) -> Result<(), InfrastructureFailure> {
    for identity in sidecar_identities {
        let root_bits = index.generated_roots.get(identity).copied();
        {
            let sidecar = transaction.sidecar_mut(identity);
            let report = &mut sidecar.module.executable.borrow_analysis;
            let hir = &mut sidecar.module.executable.hir;
            let site = ConstraintNodeSite::Sidecar(identity.clone());
            let Some(function_nodes) = index.sidecar_functions.get(identity) else {
                continue;
            };
            for (function, node_id) in function_nodes {
                let summary = report
                    .analysis
                    .public_call_summaries
                    .get_mut(function)
                    .ok_or_else(|| missing_summary("generated sidecar", *function))?;
                bits[*node_id].apply_to(summary);
            }
            sync_leaf_maps(hir, &site, index, bits);
        }

        let Some(root_bits) = root_bits.map(|node| bits[node]) else {
            continue;
        };
        let Some(current) = transaction.summary(identity).cloned() else {
            return Err(missing_summary_for_generated(identity));
        };
        let current_bits = ConstraintSummaryBits::from_summary(&current);
        let widened_bits = current_bits.union(root_bits);
        if widened_bits == current_bits {
            continue;
        }
        let mut summary = current;
        widened_bits.apply_to(&mut summary);
        update_generated_summary(transaction, identity, summary)?;
    }
    Ok(())
}

/// Sync the HIR leaf maps that can name an owned node.
///
/// WHAT: active-base imported entries, owned module-private entries and completed generated roots.
/// WHY: verification and retained artefacts must see one final value. Provider leaves stay
///      immutable because their origins/identities resolve to no node.
fn sync_leaf_maps(
    hir: &mut HirModule,
    site: &ConstraintNodeSite,
    index: &NodeIndex,
    bits: &[ConstraintSummaryBits],
) {
    for (origin, summary) in &mut hir.imported_call_summaries {
        if let Some(node) = index.base_public_origins.get(origin).copied() {
            bits[node].apply_to(summary);
        }
    }
    for (identity, summary) in &mut hir.module_private_call_summaries {
        let node = match site {
            ConstraintNodeSite::Base => index.base_private_identities.get(identity).copied(),
            ConstraintNodeSite::Sidecar(sidecar) => index
                .sidecar_private_identities
                .get(sidecar)
                .and_then(|identities| identities.get(identity))
                .copied()
                .or_else(|| index.base_private_identities.get(identity).copied()),
        };
        if let Some(node) = node {
            bits[node].apply_to(summary);
        }
    }
    for (identity, summary) in &mut hir.generated_call_summaries {
        if let Some(node) = index.generated_roots.get(identity).copied() {
            bits[node].apply_to(summary);
        }
    }
}

fn missing_summary(owner: &str, function: FunctionId) -> InfrastructureFailure {
    InfrastructureFailure::internal(
        CompilerStage::ProjectBuildAssembly,
        format!("constraint convergence found no {owner} call summary for {function:?}"),
    )
}

fn missing_summary_for_generated(identity: &GeneratedFunctionIdentity) -> InfrastructureFailure {
    InfrastructureFailure::internal(
        CompilerStage::ProjectBuildAssembly,
        format!("constraint convergence lost the generated summary for {identity:?}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
    use crate::compiler_frontend::datatypes::CallableReturnShape;
    use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
    use crate::compiler_frontend::datatypes::numeric_operators::{
        NumericOperator, NumericSemantics,
    };
    use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
    use crate::compiler_frontend::external_packages::ExternalFunctionId;
    use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
    use crate::compiler_frontend::hir::expression_store::{HirExpressionStore, HirValueRange};
    use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
    use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionReturnContract};
    use crate::compiler_frontend::hir::ids::{BlockId, HirNodeId, LocalId, RegionId};
    use crate::compiler_frontend::hir::numeric::{HirNumericOp, HirNumericOperands};
    use crate::compiler_frontend::hir::places::HirPlace;
    use crate::compiler_frontend::hir::regions::HirRegion;
    use crate::compiler_frontend::hir::statements::{
        HirStatement, HirStatementKind, HirWriteTarget,
    };
    use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
    use crate::compiler_frontend::module_compilation::generated::known::KnownGeneratedFunctions;
    use crate::compiler_frontend::module_compilation::generated::transaction::GeneratedFunctionTransaction;
    use crate::compiler_frontend::public_call_summary::{
        CallAllocationFacts, CallLaneSummary, FunctionReturnAliasSummary, PublicCallSummary,
    };
    use crate::compiler_frontend::semantic_identity::{
        ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
    };
    use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
    use rustc_hash::FxHashMap;

    fn provider_origin() -> OriginFunctionId {
        let module_origin = StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("constraint-convergence-tests"),
            "main".to_owned(),
            ModuleRootRole::Normal,
        );
        OriginFunctionId::new_free(module_origin, "provided".to_owned())
    }

    fn literal_expression(
        store: &mut HirExpressionStore,
        scalar: FixedScalar,
        value: i64,
    ) -> crate::compiler_frontend::hir::ids::HirValueId {
        let literal = FixedScalarValue::signed(scalar, value).expect("test literal fits");
        store
            .append_expression(HirExpression {
                kind: HirExpressionKind::FixedScalar(literal),
                ty: builtin_type_ids::fixed_scalar(scalar),
                value_kind: ValueKind::Const,
                region: RegionId(0),
                span: None,
            })
            .expect("test expression store accepts a literal")
    }

    fn summary_with_bits(
        uses_fast_semantics: bool,
        may_trap: bool,
        may_fail: bool,
    ) -> PublicCallSummary {
        PublicCallSummary {
            parameters: Vec::new(),
            return_alias: FunctionReturnAliasSummary::Known(Box::new(CallAllocationFacts {
                success: CallLaneSummary {
                    reachable: true,
                    ..Default::default()
                },
                error: CallLaneSummary::default(),
            })),
            escapes_builtin_failure: false,
            uses_fast_semantics,
            may_trap,
            may_fail,
        }
    }

    /// Build one function whose only dead handler calls an immutable provider leaf.
    ///
    /// Block 0 proves `10 // divisor` total from a same-block single-definition literal, so the
    /// `Invoke` failure edge to block 2 is dead for liveness. Block 2 still holds a selected call
    /// to a fast/trapping provider: fast use must widen, trap must not.
    fn dead_handler_module() -> (HirModule, OriginFunctionId) {
        let provider = provider_origin();
        let mut module = HirModule::new();
        module.regions = vec![HirRegion::lexical(RegionId(0), None)];
        module.expressions = HirExpressionStore::default();

        let left = literal_expression(&mut module.expressions, FixedScalar::I32, 10);
        let divisor_literal = literal_expression(&mut module.expressions, FixedScalar::I32, 2);
        let divisor_local = LocalId(0);
        let divisor_load = module
            .expressions
            .append_expression(HirExpression {
                kind: HirExpressionKind::Load(HirPlace::local(divisor_local)),
                ty: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                value_kind: ValueKind::Place,
                region: RegionId(0),
                span: None,
            })
            .expect("test expression store accepts a load");

        module.blocks = vec![
            HirBlock {
                id: BlockId(0),
                region: RegionId(0),
                locals: vec![HirLocal {
                    id: divisor_local,
                    ty: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    mutable: false,
                    region: RegionId(0),
                    span: None,
                }],
                statements: vec![HirStatement {
                    id: HirNodeId(0),
                    kind: HirStatementKind::Write {
                        target: HirWriteTarget::DefineLocal(divisor_local),
                        value: divisor_literal,
                    },
                    span: None,
                }],
                terminator: HirTerminator::Invoke {
                    id: HirNodeId(1),
                    span: None,
                    operation: HirFallibleOperation::NumericOp {
                        op: HirNumericOp {
                            operator: NumericOperator::IntegerDivide,
                            domain: NumericScalar::Fixed(FixedScalar::I32),
                            semantics: NumericSemantics::Ordinary,
                        },
                        operands: HirNumericOperands::Binary {
                            left,
                            right: divisor_load,
                        },
                    },
                    success: BlockId(1),
                    success_results: vec![],
                    failure: BlockId(2),
                    error_result: LocalId(1),
                },
            },
            HirBlock {
                id: BlockId(1),
                region: RegionId(0),
                locals: vec![],
                statements: vec![],
                terminator: HirTerminator::Return(HirValueRange::empty()),
            },
            HirBlock {
                id: BlockId(2),
                region: RegionId(0),
                locals: vec![],
                statements: vec![HirStatement {
                    id: HirNodeId(2),
                    kind: HirStatementKind::Call {
                        target: CallTarget::CrossModule(provider.clone()),
                        args: HirValueRange::empty(),
                        results: vec![],
                    },
                    span: None,
                }],
                terminator: HirTerminator::Return(HirValueRange::empty()),
            },
        ];
        module.functions = vec![HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: vec![],
            return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        }];
        module.start_function = Some(FunctionId(0));
        module
            .imported_call_summaries
            .insert(provider.clone(), summary_with_bits(true, true, false));
        (module, provider)
    }

    #[test]
    fn dead_handler_calls_propagate_selected_fast_but_not_trap() {
        let (mut hir, provider) = dead_handler_module();
        let mut report = BorrowCheckReport::default();
        report
            .analysis
            .public_call_summaries
            .insert(FunctionId(0), summary_with_bits(false, false, false));

        let records = Vec::new();
        let by_identity = FxHashMap::default();
        let mut transaction =
            GeneratedFunctionTransaction::new(KnownGeneratedFunctions::new(&records, &by_identity));
        let registry = ExternalPackageRegistry::new();

        converge_constraint_summaries(&mut hir, &mut report, &mut transaction, &registry)
            .expect("phase B should converge a synthetic module");

        let caller = &report.analysis.public_call_summaries[&FunctionId(0)];
        assert!(
            caller.uses_fast_semantics,
            "a call in a liveness-dead handler is still selected fast evidence"
        );
        assert!(
            !caller.may_trap,
            "the dead handler must not contribute a trap"
        );
        assert!(!caller.may_fail);

        let leaf = &hir.imported_call_summaries[&provider];
        assert!(
            leaf.uses_fast_semantics && leaf.may_trap,
            "the provider leaf keeps its published bits"
        );
        assert!(!leaf.may_fail, "phase B must never rewrite a provider leaf");
    }

    #[test]
    fn plain_unknown_external_call_is_fast_and_trapping_but_not_failing() {
        let mut module = HirModule::new();
        module.regions = vec![HirRegion::lexical(RegionId(0), None)];
        module.blocks = vec![HirBlock {
            id: BlockId(0),
            region: RegionId(0),
            locals: vec![],
            statements: vec![HirStatement {
                id: HirNodeId(0),
                kind: HirStatementKind::Call {
                    // A synthetic ID has no registry entry: it is the unknown-external case now
                    // that Core IO and Core Math publish trusted contracts.
                    target: CallTarget::External(ExternalFunctionId::Synthetic(9_999)),
                    args: HirValueRange::empty(),
                    results: vec![],
                },
                span: None,
            }],
            terminator: HirTerminator::Return(HirValueRange::empty()),
        }];
        module.functions = vec![HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: vec![],
            return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        }];
        module.start_function = Some(FunctionId(0));

        let mut report = BorrowCheckReport::default();
        report
            .analysis
            .public_call_summaries
            .insert(FunctionId(0), summary_with_bits(false, false, false));
        let records = Vec::new();
        let by_identity = FxHashMap::default();
        let mut transaction =
            GeneratedFunctionTransaction::new(KnownGeneratedFunctions::new(&records, &by_identity));
        converge_constraint_summaries(
            &mut module,
            &mut report,
            &mut transaction,
            &ExternalPackageRegistry::new(),
        )
        .expect("phase B should classify an unknown external");

        let summary = &report.analysis.public_call_summaries[&FunctionId(0)];
        assert!(
            summary.uses_fast_semantics,
            "an unregistered external is unknown for fast semantics"
        );
        assert!(
            summary.may_trap,
            "an unregistered external is unknown for trap freedom"
        );
        assert!(
            !summary.may_fail,
            "a plain external call has no error channel to propagate"
        );
    }
}
