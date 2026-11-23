//! Metadata passes for HIR module construction.
//!
//! WHAT: fills non-CFG module metadata after declarations and constants have
//! been prepared.
//! WHY: function origins are executable HIR metadata consumed by builders and later validation.
//! Resolved documentation fragments are non-HIR compiler metadata extracted into the lowering
//! metadata result boundary, not stored on `HirModule`.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::{AstFunctionIdentity, NodeKind};
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionView;
use crate::compiler_frontend::ast::expressions::failure_facts::{
    ImplicitFailureContributor, ImplicitFailureSource,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::failure_facts::{
    HirBuiltinFailureBoundary, HirBuiltinFailureContributor, HirBuiltinFailureSource,
    HirConstraintBoundary, HirConstraintEvidence, HirDeferredCustomCatchCheck,
    HirFastUseProvenance, HirFastUseWitness, HirFunctionFailureFacts, HirLiveErasedCallSite,
    HirSelectedCallSite, HirSelectedCallSiteLedger,
};
use crate::compiler_frontend::hir::functions::{HirFunctionOrigin, HirStableFunctionOrigin};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId};
use crate::compiler_frontend::hir::numeric::NumericFailureMode;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::hir::utils::{collect_reachable_blocks, for_each_terminator_target};
use crate::compiler_frontend::module_metadata::ModuleDocFragment;
use crate::compiler_frontend::source::{SourceSpan, SpanResolutionSources};
use rustc_hash::{FxHashMap, FxHashSet};
use std::convert::Infallible;

impl<'a> HirBuilder<'a> {
    /// Record this expression's synthetic-interface provenance in its owning HIR block.
    ///
    /// WHAT: retains a transient block projection so post-convergence catch pruning can remove
    ///       inactive handler provenance before link facts are published.
    /// WHY: expression lowering already visits every provenance-bearing AST value; this avoids
    ///      introducing another AST walk or storing provenance on every HIR expression.
    pub(crate) fn accumulate_function_provenance(&mut self, expression: ExpressionView<'_>) {
        let Some(occurrence) = expression.occurrence_facts() else {
            return;
        };
        if occurrence.provenance.is_empty() {
            return;
        }
        let Some(function_id) = self.current_function else {
            return;
        };
        self.module
            .function_provenance_by_block
            .entry((function_id, self.current_block))
            .or_default()
            .merge(&occurrence.provenance);
    }

    pub(super) fn assign_function_origins(&mut self) -> Result<(), InfrastructureFailure> {
        // WHAT: classify every lowered function and retain stable origins for direct public
        // functions and receiver methods.
        // WHY: downstream public-interface finalization needs an explicit semantic join while
        // backend-facing origin tags remain unchanged for private functions and entry start.
        self.module.function_origins.clear();
        self.module.function_ids_by_origin.clear();
        self.module.function_ids_by_private_origin.clear();

        for function in &self.module.functions {
            self.module
                .function_origins
                .insert(function.id, HirFunctionOrigin::Normal);

            if Some(function.id) == self.module.start_function {
                continue;
            }

            let function_path = self
                .side_table
                .function_name_path(function.id)
                .ok_or_else(|| {
                    InfrastructureFailure::internal(CompilerStage::HirConstruction, format!(
                        "HIR function-origin lowering is missing the declaration path for local function {:?}",
                        function.id
                    ))
                })?;

            let Some(origin) = self
                .function_origin_lookup
                .consume_origin_for(&function_path)
            else {
                continue;
            };

            match origin {
                HirStableFunctionOrigin::Public(origin) => {
                    let facts = self
                        .module
                        .function_failure_facts
                        .get_mut(&function.id)
                        .ok_or_else(|| {
                            InfrastructureFailure::internal(
                                CompilerStage::HirConstruction,
                                "HIR function-origin lowering is missing projected failure facts",
                            )
                        })?;
                    if facts.boundary == HirBuiltinFailureBoundary::InferPrivate {
                        facts.boundary = HirBuiltinFailureBoundary::ExportedNoSlot;
                    }
                    if self.module.function_ids_by_origin.contains_key(&origin) {
                        return Err(InfrastructureFailure::internal(
                            CompilerStage::HirConstruction,
                            format!(
                                "HIR function-origin lowering received duplicate stable origin {:?}",
                                origin
                            ),
                        ));
                    }
                    self.module
                        .function_ids_by_origin
                        .insert(origin, function.id);
                }
                HirStableFunctionOrigin::ModulePrivate(origin) => {
                    if self
                        .module
                        .function_ids_by_private_origin
                        .contains_key(&origin)
                    {
                        return Err(InfrastructureFailure::internal(
                            CompilerStage::HirConstruction,
                            format!(
                                "HIR function-origin lowering received duplicate private origin {:?}",
                                origin
                            ),
                        ));
                    }
                    self.module
                        .function_ids_by_private_origin
                        .insert(origin, function.id);
                }
            }
        }

        // Reject any concrete origin seed that no lowered function consumed. An unmatched seed
        // means a public callable declaration did not lower to local HIR, which is an internal
        // invariant failure rather than a silent deferral to public-interface finalization.
        self.function_origin_lookup.validate_all_seeds_consumed()?;

        if let Some(start_function) = self.module.start_function {
            self.module
                .function_origins
                .insert(start_function, HirFunctionOrigin::EntryStart);
        }

        Ok(())
    }

    /// Project pre-fold failure facts and constraint evidence onto HIR functions.
    ///
    /// WHAT: reads the authored-order contributors, deferred catches and assertion-message calls
    ///       finalization retained on the AST, then freezes the same function's constraint evidence.
    /// WHY: release folding runs before HIR, so reclassifying the post-fold AST would let erasure
    ///      add, remove or reorder a failure fact. The AST classifier is the single owner of that
    ///      decision; this pass only resolves its facts onto HIR call targets.
    pub(super) fn project_function_failure_facts(
        &mut self,
        ast: &Ast,
        span_sources: SpanResolutionSources<'_>,
    ) -> Result<(), HirConstructionFailure> {
        let mut constraint_evidence = self.project_constraint_evidence(ast, &span_sources)?;
        let builtin_error_type =
            self.type_environment
                .type_id_for_canonical_identity(&CanonicalTypeIdentity::Builtin(
                    CanonicalBuiltinType::Error,
                ));

        for node in &ast.nodes {
            let signature = match &node.kind {
                NodeKind::Function(_, signature, _) | NodeKind::EntryStart(signature, _) => {
                    signature
                }
                _ => continue,
            };
            let Some(identity) = node.function_identity() else {
                continue;
            };
            let function_id = match identity {
                AstFunctionIdentity::Authored(path) => self.resolve_function_id_or_error(&path)?,
                AstFunctionIdentity::Entry => self.module.start_function.ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::HirConstruction,
                        "completed AST has an entry body but HIR allocated no entry function",
                    )
                })?,
            };
            let boundary = if signature.return_contract.is_never() {
                // A `-> !` body has no error channel. Implicit built-in failure that can escape
                // the body is a closed-boundary diagnostic, not a private lane.
                HirBuiltinFailureBoundary::Never
            } else {
                match signature
                    .return_contract
                    .ordinary()
                    .and_then(|returns| returns.error())
                {
                    Some(error_type) if Some(error_type) == builtin_error_type => {
                        HirBuiltinFailureBoundary::BuiltinErrorSlot
                    }
                    Some(error_type) => HirBuiltinFailureBoundary::CustomErrorSlot(error_type),
                    None => HirBuiltinFailureBoundary::InferPrivate,
                }
            };
            let pending = ast.pre_fold_failure_facts.get(&identity).ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::HirConstruction,
                    "completed function has no pre-fold failure facts",
                )
            })?;
            let constraint = constraint_evidence.remove(&function_id).unwrap_or_default();
            let contributors = self.project_failure_contributors(&pending.body.implicit)?;
            // Inert messages still carry private-call obligations. The module service installs
            // canonical generated contracts for these validation-only roots before AST release;
            // resolving their targets here creates no executable call or link dependency.
            let assertion_message_calls =
                self.project_failure_contributors(&pending.assertion_message_calls)?;
            let mut deferred_custom_catches =
                Vec::with_capacity(pending.body.deferred_custom_catches.len());
            for check in &pending.body.deferred_custom_catches {
                deferred_custom_catches.push(HirDeferredCustomCatchCheck {
                    catch_span: check.catch_span,
                    error_type_id: check.error_type_id,
                    typed_producer_span: check.typed_producer_span,
                    eligibility_only: check.eligibility_only,
                    candidates: self.project_failure_contributors(&check.candidates)?,
                });
            }
            self.module.function_failure_facts.insert(
                function_id,
                HirFunctionFailureFacts {
                    span: node.span,
                    boundary,
                    infallible_cast_evidence: match identity {
                        AstFunctionIdentity::Authored(path) => {
                            ast.infallible_cast_evidence.get(&path).copied()
                        }
                        AstFunctionIdentity::Entry => None,
                    },
                    contributors,
                    assertion_message_calls,
                    deferred_custom_catches,
                    constraint,
                },
            );
        }

        Ok(())
    }

    /// Projects source constraint boundaries and selected fast-use evidence onto HIR functions.
    ///
    /// WHAT: maps AST callable identities to lowered `FunctionId`s, drops evidence inside inactive
    ///       static-`if` regions of the same callable, and merges the AST fast-operation ledger
    ///       with fast operations present in retained HIR.
    /// WHY: verification runs after AST bodies and directive syntax are gone. Dormant generic
    ///       template rows have no HIR function and are dropped here, which makes them inert; a
    ///       materialised instance is an ordinary function in its sidecar and projects normally.
    ///       Deduplication is by `(provenance, span)`, so one authored operation recorded before
    ///       folding and retained as HIR appears once, while a folded-away row survives.
    fn project_constraint_evidence(
        &self,
        ast: &Ast,
        span_sources: &SpanResolutionSources<'_>,
    ) -> Result<FxHashMap<FunctionId, HirConstraintEvidence>, HirConstructionFailure> {
        let mut evidence = self
            .module
            .functions
            .iter()
            .map(|function| (function.id, HirConstraintEvidence::default()))
            .collect::<FxHashMap<_, _>>();

        self.project_authored_constraint_evidence(ast, span_sources, &mut evidence);
        self.project_selected_hir_evidence(ast, span_sources, &mut evidence)?;

        for function_evidence in evidence.values_mut() {
            function_evidence.fast_use.sort_unstable();
            function_evidence.fast_use.dedup();
        }

        Ok(evidence)
    }

    /// Projects boundaries and the authored fast-operation ledger, excluding inactive branches.
    fn project_authored_constraint_evidence(
        &self,
        ast: &Ast,
        span_sources: &SpanResolutionSources<'_>,
        evidence: &mut FxHashMap<FunctionId, HirConstraintEvidence>,
    ) {
        let function_id = |identity: AstFunctionIdentity| match identity {
            AstFunctionIdentity::Entry => self.module.start_function,
            AstFunctionIdentity::Authored(path) => self.functions_by_name.get(&path).copied(),
        };
        let is_pruned = |identity: AstFunctionIdentity, scope, span: Option<SourceSpan>| {
            ast.constraint_pruned_regions.iter().any(|region| {
                region.function == identity
                    && (region.scope.is_some_and(|region_scope| {
                        self.path_fork.starts_with(scope, region_scope)
                    }) || span.is_some_and(|span| {
                        span_sources.span_within(span, region.start, region.end)
                    }))
            })
        };

        for boundary in &ast.constraint_boundaries {
            let Some(function_id) = function_id(boundary.function) else {
                continue;
            };
            if let Some(statement) = boundary.statement
                && (is_pruned(boundary.function, boundary.scope, Some(statement.start))
                    || is_pruned(boundary.function, boundary.scope, Some(statement.end)))
            {
                continue;
            }
            evidence
                .entry(function_id)
                .or_default()
                .boundaries
                .push(HirConstraintBoundary {
                    statement: boundary.statement,
                    constraints: boundary.constraints,
                    directive_spans: boundary.directive_spans,
                });
        }

        for witness in &ast.fast_operation_witnesses {
            let Some(function_id) = function_id(witness.function) else {
                continue;
            };
            if is_pruned(witness.function, witness.scope, witness.span) {
                continue;
            }
            evidence
                .entry(function_id)
                .or_default()
                .fast_use
                .push(HirFastUseWitness {
                    provenance: HirFastUseProvenance::AuthoredArithmetic,
                    span: witness.span,
                });
        }
    }

    /// Projects fast operations and direct call targets from retained HIR.
    fn project_selected_hir_evidence(
        &self,
        ast: &Ast,
        span_sources: &SpanResolutionSources<'_>,
        evidence: &mut FxHashMap<FunctionId, HirConstraintEvidence>,
    ) -> Result<(), HirConstructionFailure> {
        for function in &self.module.functions {
            let mut fast_use = Vec::new();
            let mut live_selected_call_sites = HirSelectedCallSiteLedger::default();
            let mut live_erased_call_sites: Vec<HirLiveErasedCallSite> = Vec::new();
            let function_blocks = self.selected_function_blocks(function.id, function.entry);
            for block_id in &function_blocks {
                let Some(block) = self
                    .block_index_by_id
                    .get(block_id)
                    .and_then(|index| self.module.blocks.get(*index))
                else {
                    continue;
                };
                self.record_block_constraint_evidence(
                    block,
                    &mut fast_use,
                    &mut live_selected_call_sites,
                );
            }
            // Skipped and release-erased call evidence is filtered by the same callable identity
            // and discarded static-`if` spans as the authored evidence ledger.
            let identity = if self.module.start_function == Some(function.id) {
                Some(AstFunctionIdentity::Entry)
            } else {
                self.functions_by_name.iter().find_map(|(path, id)| {
                    (*id == function.id).then_some(AstFunctionIdentity::Authored(*path))
                })
            };
            let is_pruned = |span: Option<SourceSpan>| {
                span.is_some_and(|span| {
                    ast.constraint_pruned_regions.iter().any(|region| {
                        identity.is_none_or(|identity| region.function == identity)
                            && span_sources.span_within(span, region.start, region.end)
                    })
                })
            };
            let skipped_source = self.skipped_source_evidence.get(&function.id);
            let mut skipped_call_sites = Vec::new();
            if let Some(skipped) = skipped_source {
                // The omitted statements still have authored source. Drop evidence that lives only
                // in an inactive static-`if` branch, using the same pruned regions and span rule as
                // the authored evidence ledger.
                // One skipped catch handler can name hundreds of calls; reserve once instead of
                // regrowing the site list one row at a time.
                skipped_call_sites.reserve(skipped.call_sites().len());
                for site in skipped.call_sites() {
                    if is_pruned(site.span) {
                        continue;
                    }
                    skipped_call_sites.push(HirSelectedCallSite {
                        target: site.target.clone(),
                        span: site.span,
                    });
                }
                for witness in skipped.fast_use() {
                    if !is_pruned(witness.span) {
                        fast_use.push(*witness);
                    }
                }
            }
            // Release folding erased these calls before HIR existed, but they were selected source.
            // Adding the row here makes callee summary bits and witness hops identical to a debug
            // build that keeps the runtime call. Resolution uses the ordinary call-target mapping
            // rather than a second AST-path translation. The rows also feed liveness-conditioned
            // trap and escaping-failure analysis.
            for site in &ast.release_erased_call_sites {
                if identity != Some(site.function) || is_pruned(Some(site.span)) {
                    continue;
                }
                let target = self.resolve_call_target_or_error(&site.target)?;
                // Release speculation never folds a catch handler body or a handled subject, so
                // every recorded row is ordinary executable source that HIR lowered. No row needs
                // per-handler containment: a call that can fail here really does escape.
                insert_live_erased_site_in_source_order(
                    &mut live_erased_call_sites,
                    HirLiveErasedCallSite {
                        target,
                        span: site.span,
                    },
                    span_sources,
                );
            }
            let mut live_sites = live_selected_call_sites.into_sites();
            for site in &live_erased_call_sites {
                insert_selected_site_in_source_order(
                    &mut live_sites,
                    HirSelectedCallSite {
                        target: site.target.clone(),
                        span: Some(site.span),
                    },
                    span_sources,
                );
            }
            let mut selected_call_sites = HirSelectedCallSiteLedger::default();
            for site in live_sites.into_iter().chain(skipped_call_sites) {
                selected_call_sites.push(&site.target, site.span);
            }
            let function_evidence = evidence.entry(function.id).or_default();
            function_evidence.fast_use.extend(fast_use);
            function_evidence.selected_call_sites = selected_call_sites.into_sites();
            function_evidence.live_erased_call_sites = live_erased_call_sites;
        }
        Ok(())
    }

    /// Block ids of one function's selected code, including unrouted catch-handler subgraphs.
    ///
    /// WHAT: the entry-reachable CFG plus every handler root and successor not yet linked to it.
    /// WHY: convergence may prune an unrouted handler later, so its fast operations and call
    ///      targets must be recorded now while the blocks still exist.
    fn selected_function_blocks(&self, function: FunctionId, entry: BlockId) -> Vec<BlockId> {
        let reachable_from = |root: BlockId| -> Vec<BlockId> {
            let Ok(order) = collect_reachable_blocks::<Infallible>(root, |block_id, visit| {
                if let Some(block) = self
                    .block_index_by_id
                    .get(&block_id)
                    .and_then(|index| self.module.blocks.get(*index))
                {
                    for_each_terminator_target(&block.terminator, visit);
                }
                Ok(())
            });
            order
        };
        let mut blocks = reachable_from(entry);
        let mut seen_blocks = blocks.iter().copied().collect::<FxHashSet<_>>();
        for record in &self.catch_handlers {
            if record.owner != function || seen_blocks.contains(&record.handler.block) {
                continue;
            }
            let handler_blocks = reachable_from(record.handler.block);
            for block_id in handler_blocks {
                if seen_blocks.insert(block_id) {
                    blocks.push(block_id);
                }
            }
        }
        blocks
    }

    /// Records the fast-use and direct-call evidence one retained block owns.
    fn record_block_constraint_evidence(
        &self,
        block: &HirBlock,
        fast_use: &mut Vec<HirFastUseWitness>,
        selected_call_sites: &mut HirSelectedCallSiteLedger,
    ) {
        for statement in &block.statements {
            match &statement.kind {
                HirStatementKind::Call { target, .. } => {
                    selected_call_sites.push(target, statement.span);
                }
                HirStatementKind::NumericOp { op, .. } if op.semantics.is_fast() => {
                    fast_use.push(HirFastUseWitness {
                        provenance: HirFastUseProvenance::AuthoredArithmetic,
                        span: statement.span,
                    });
                }
                // A generated `$fast_math` range ordinal keeps ordinary checked semantics (so it
                // never wraps) but delivers its overflow fatally. Fast authored operations hit
                // the arm above, so a fatal operation here is the generated ordinal; it must still
                // count as fast range evidence or `$safe_math` accepts a trapping loop.
                HirStatementKind::NumericOp {
                    failure_mode: NumericFailureMode::Fatal,
                    ..
                } => {
                    fast_use.push(HirFastUseWitness {
                        provenance: HirFastUseProvenance::RangeGuard,
                        span: statement.span,
                    });
                }
                HirStatementKind::LoopStepFailure { semantics, .. } if semantics.is_fast() => {
                    fast_use.push(HirFastUseWitness {
                        provenance: HirFastUseProvenance::RangeGuard,
                        span: statement.span,
                    });
                }
                // Every fast write-back is selected fast semantics, including the total integer
                // wrap that carries `Infallible` delivery.
                HirStatementKind::FastStoreConversion { .. } => {
                    fast_use.push(HirFastUseWitness {
                        provenance: HirFastUseProvenance::StoreConversion,
                        span: statement.span,
                    });
                }
                _ => {}
            }
        }
        if let HirTerminator::Invoke {
            span,
            operation: HirFallibleOperation::NumericOp { op, .. },
            ..
        } = &block.terminator
            && op.semantics.is_fast()
        {
            fast_use.push(HirFastUseWitness {
                provenance: HirFastUseProvenance::AuthoredArithmetic,
                span: *span,
            });
        }
        match &block.terminator {
            HirTerminator::Invoke {
                span,
                operation: HirFallibleOperation::Call { target, .. },
                ..
            } => selected_call_sites.push(target, *span),
            HirTerminator::NeverCall { target, .. } => selected_call_sites.push(
                target,
                self.module.side_table.terminator_span(block.id).copied(),
            ),
            _ => {}
        }
    }

    fn project_failure_contributors(
        &self,
        pending: &[ImplicitFailureContributor],
    ) -> Result<Vec<HirBuiltinFailureContributor>, HirConstructionFailure> {
        let mut contributors = Vec::with_capacity(pending.len());
        for contributor in pending {
            let source = match contributor.source {
                ImplicitFailureSource::NumericOperation => {
                    HirBuiltinFailureSource::NumericOperation
                }
                ImplicitFailureSource::AuthoredCastConversion => {
                    HirBuiltinFailureSource::AuthoredCastConversion
                }
                ImplicitFailureSource::CompoundWriteBack {
                    target,
                    arithmetic_span,
                } => HirBuiltinFailureSource::CompoundWriteBack {
                    target,
                    arithmetic_span,
                },
                ImplicitFailureSource::PrivateCall(path) => {
                    HirBuiltinFailureSource::Call(self.resolve_call_target_or_error(&path)?)
                }
            };
            contributors.push(HirBuiltinFailureContributor {
                source,
                span: contributor.span,
                codes: contributor.codes.to_vec(),
            });
        }
        Ok(contributors)
    }

    pub(super) fn resolve_doc_fragments(&mut self, ast: &Ast) {
        self.extracted_metadata.doc_fragments.clear();

        for fragment in &ast.doc_fragments {
            self.extracted_metadata
                .doc_fragments
                .push(ModuleDocFragment {
                    target: fragment.target,
                    rendered_text: self.string_table.resolve(fragment.value).to_owned(),
                    span: fragment.span,
                });
        }
    }
}

/// Insert one erased call into an evaluation-ordered selected-site list.
///
/// WHAT: places the row before the first retained site the erased call is evaluated before.
/// WHY: debug lowering emits argument calls before their outer call statement, so containment
///      outranks source start. Disjoint calls follow authored source order, which is the order
///      retained HIR statements already use.
fn insert_selected_site_in_source_order(
    sites: &mut Vec<HirSelectedCallSite>,
    site: HirSelectedCallSite,
    sources: &SpanResolutionSources<'_>,
) {
    let Some(site_span) = site.span else {
        sites.push(site);
        return;
    };
    let position = sites.iter().position(|existing| {
        existing
            .span
            .is_some_and(|existing| site_precedes(site_span, existing, sources))
    });
    match position {
        Some(index) => sites.insert(index, site),
        None => sites.push(site),
    }
}

/// Insert one live erased call into an evaluation-ordered selected-site list.
fn insert_live_erased_site_in_source_order(
    sites: &mut Vec<HirLiveErasedCallSite>,
    site: HirLiveErasedCallSite,
    sources: &SpanResolutionSources<'_>,
) {
    let site_span = site.span;
    let position = sites
        .iter()
        .position(|existing| site_precedes(site_span, existing.span, sources));
    match position {
        Some(index) => sites.insert(index, site),
        None => sites.push(site),
    }
}

fn site_precedes(left: SourceSpan, right: SourceSpan, sources: &SpanResolutionSources<'_>) -> bool {
    if left == right {
        return false;
    }
    if sources.span_within(left, right, right) {
        return true;
    }
    if sources.span_within(right, left, left) {
        return false;
    }
    left < right
}
