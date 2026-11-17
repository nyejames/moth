//! Source-constraint verification over final HIR.
//!
//! WHAT: after phase B has converged the three call-summary bits, checks every `$safe_math` and
//!       `$infallible` function boundary and statement region, and builds one typed diagnostic per
//!       failed guarantee.
//! WHY: the guarantees are proved from final lane-delivered HIR and the converged summaries, never
//!       from backend proof tables or AST syntax. A function whose bit is true only through calls
//!       needs a bounded, deterministic witness path; this module owns that walk and the one
//!       projection from facts to diagnostics. Statement regions reuse the post-lane analysis so
//!       both boundary shapes apply the same proof and liveness rules.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::ast::expressions::failure_facts::MAX_CALL_HOPS;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::constraints::invalid_source_constraint;
use crate::compiler_frontend::compiler_messages::diagnostics::reasons::InvalidSourceConstraintReason;
use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalFunctionId, ExternalPackageRegistry,
};
use crate::compiler_frontend::hir::constraint_facts::analysis::{
    ConstraintWitness, ConstraintWitnessKind, FunctionConstraintAnalysis, TrapCauseMask,
    analyze_statement_region, collect_function_call_sites,
};
use crate::compiler_frontend::hir::constraint_facts::{
    ConstraintCalleeFacts, ConstraintSummaryBits, analyze_function,
};
use crate::compiler_frontend::hir::failure_facts::{
    HirConstraintBoundary, HirConstraintEvidence, HirFastUseProvenance, HirFastUseWitness,
    HirSelectedCallSite,
};
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::module_compilation::generated::GeneratedFunctionTransaction;
use crate::compiler_frontend::semantic_identity::{
    GeneratedFunctionIdentity, ModulePrivateExecutableIdentity, OriginFunctionId,
};
use crate::compiler_frontend::source::{
    FrozenIdentityHandle, SourceDatabase, SourceSpan, SpanResolutionSources,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Which module a verification verdict belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConstraintModuleSite {
    Base,
    Sidecar(GeneratedFunctionIdentity),
}

/// One Moth-source function in the verification universe.
struct ConstraintFunctionFacts<'hir> {
    module_index: usize,
    function: FunctionId,
    summary: ConstraintSummaryBits,
    analysis: FunctionConstraintAnalysis,
    /// Projected boundaries and fast evidence; verification only reads them.
    constraint_evidence: Option<&'hir HirConstraintEvidence>,
    calls: Vec<HirSelectedCallSite>,
    name_path: Option<PathId>,
}

impl ConstraintFunctionFacts<'_> {
    fn boundaries(&self) -> &[HirConstraintBoundary] {
        self.constraint_evidence
            .map_or(&[], |evidence| evidence.boundaries.as_slice())
    }

    fn fast_use(&self) -> &[HirFastUseWitness] {
        self.constraint_evidence
            .map_or(&[], |evidence| evidence.fast_use.as_slice())
    }
}

/// Evidence owned by a non-local callable.
struct LeafFacts {
    bits: ConstraintSummaryBits,
    name: String,
    unknown: bool,
    external: bool,
}

impl LeafFacts {
    fn callee_facts(&self) -> ConstraintCalleeFacts {
        if self.unknown {
            ConstraintCalleeFacts::unknown_external()
        } else if self.external {
            ConstraintCalleeFacts::trusted_external(self.bits)
        } else {
            ConstraintCalleeFacts::from_bits(self.bits)
        }
    }
}

/// One module's final HIR, functions, summaries and target-related leaf facts.
struct ConstraintModuleFacts<'hir> {
    site: ConstraintModuleSite,
    /// Final HIR retained by reference; verification never re-runs phase B or clones bodies.
    hir: &'hir HirModule,
    /// Declaring-domain spans for a sidecar; `None` for the base module's live database.
    source_owner: Option<FrozenIdentityHandle>,
    functions: Vec<ConstraintFunctionFacts<'hir>>,
    function_by_id: FxHashMap<FunctionId, usize>,
    public_origins: &'hir FxHashMap<OriginFunctionId, FunctionId>,
    private_origins: &'hir FxHashMap<ModulePrivateExecutableIdentity, FunctionId>,
    imported_leaves: FxHashMap<OriginFunctionId, LeafFacts>,
    private_leaves: FxHashMap<ModulePrivateExecutableIdentity, LeafFacts>,
    generated_leaves: FxHashMap<GeneratedFunctionIdentity, LeafFacts>,
}

/// A final-HIR, final-summary snapshot used by verification.
pub(crate) struct ConstraintUniverse<'hir> {
    modules: Vec<ConstraintModuleFacts<'hir>>,
    /// Every owned generated root across the base and completed sidecars.
    ///
    /// WHAT: one identity to `(module, function)` row per final HIR that owns the body.
    /// WHY: phase B resolves generated calls through its global node index. Verification needs the
    ///      same ownership to follow a witness into a materialised body instead of stopping at a
    ///      published summary leaf. This is diagnostic metadata only and copies no HIR.
    generated_targets: FxHashMap<GeneratedFunctionIdentity, (usize, usize)>,
    base_public_origins: &'hir FxHashMap<OriginFunctionId, FunctionId>,
    base_private_origins: &'hir FxHashMap<ModulePrivateExecutableIdentity, FunctionId>,
    external_leaves: FxHashMap<ExternalFunctionId, LeafFacts>,
}

/// One module's failed verification and its diagnostics.
pub(crate) struct ConstraintFailure {
    pub(crate) site: ConstraintModuleSite,
    pub(crate) diagnostics: DiagnosticBag,
}

/// One callable a call target names.
enum ResolvedTarget<'a> {
    Function { module: usize, index: usize },
    Leaf(&'a LeafFacts),
    Missing { name: String },
}

impl<'a> ResolvedTarget<'a> {
    fn bits(&self, universe: &ConstraintUniverse) -> ConstraintSummaryBits {
        match self {
            Self::Function { module, index } => universe.modules[*module].functions[*index].summary,
            Self::Leaf(leaf) => leaf.bits,
            Self::Missing { .. } => ConstraintSummaryBits::unknown(),
        }
    }

    fn unknown(&self) -> bool {
        match self {
            Self::Function { .. } => false,
            Self::Leaf(leaf) => leaf.unknown,
            Self::Missing { .. } => true,
        }
    }

    fn name(&self) -> Option<&str> {
        match self {
            Self::Function { .. } => None,
            Self::Leaf(leaf) => Some(&leaf.name),
            Self::Missing { name, .. } => Some(name.as_str()),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Guarantee {
    Fast,
    Trap,
    Fail,
}

impl Guarantee {
    /// The traceable guarantee, or `None` for a recoverable failure.
    ///
    /// WHY: a failure witness is always the boundary-local escaping edge or call the analyzer
    ///      selected, so `$infallible` failure evidence never needs a call-chain walk.
    fn trace_guarantee(self) -> Option<TraceGuarantee> {
        match self {
            Self::Fast => Some(TraceGuarantee::Fast),
            Self::Trap => Some(TraceGuarantee::Trap),
            Self::Fail => None,
        }
    }
}

/// The guarantees whose evidence can lie behind a bounded call chain.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TraceGuarantee {
    Fast,
    Trap,
}

impl TraceGuarantee {
    fn unknown_reason(self) -> InvalidSourceConstraintReason {
        match self {
            Self::Fast => InvalidSourceConstraintReason::UnknownFastEvidence,
            Self::Trap => InvalidSourceConstraintReason::UnknownTrapEvidence,
        }
    }

    fn unavailable_reason(self) -> InvalidSourceConstraintReason {
        match self {
            Self::Fast => InvalidSourceConstraintReason::UnavailableCalleeFastEvidence,
            Self::Trap => InvalidSourceConstraintReason::UnavailableCalleeTrapEvidence,
        }
    }

    fn beyond_bound_reason(self) -> InvalidSourceConstraintReason {
        match self {
            Self::Fast => InvalidSourceConstraintReason::FastEvidenceBeyondWitnessBound,
            Self::Trap => InvalidSourceConstraintReason::TrapEvidenceBeyondWitnessBound,
        }
    }

    fn recursive_cycle_reason(self) -> InvalidSourceConstraintReason {
        match self {
            Self::Fast => InvalidSourceConstraintReason::FastEvidenceRecursiveCycle,
            Self::Trap => InvalidSourceConstraintReason::TrapEvidenceRecursiveCycle,
        }
    }

    fn bit(self, bits: ConstraintSummaryBits) -> bool {
        match self {
            Self::Fast => bits.uses_fast_semantics,
            Self::Trap => bits.may_trap,
        }
    }

    fn witness(self, analysis: &FunctionConstraintAnalysis) -> Option<ConstraintWitness> {
        match self {
            Self::Fast => analysis.fast_witness,
            Self::Trap => analysis.trap_witness,
        }
    }
}

enum EvidenceName {
    Owned(String),
    Path(PathId),
}

/// One authored witness span and the verification module whose source domain owns it.
///
/// WHAT: pairs a span with the module whose final HIR produced it.
/// WHY: source identities are only unique inside one identity domain. A witness that crosses a
///      materialised sidecar boundary must render each label through the owning module's frozen
///      source context, not the boundary module's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WitnessSpan {
    module_index: usize,
    span: SourceSpan,
}

struct WitnessOutcome {
    reason: InvalidSourceConstraintReason,
    evidence: Option<WitnessSpan>,
    name: Option<EvidenceName>,
    hops: Vec<Option<WitnessSpan>>,
}

/// One bounded witness search result.
enum TracedWitness {
    /// A complete evidence path, ready to render.
    Evidence(WitnessOutcome),
    /// This edge returned to an already visited function without reaching evidence.
    Cycle(WitnessOutcome),
}

impl TracedWitness {
    fn into_outcome(self) -> WitnessOutcome {
        match self {
            Self::Evidence(outcome) | Self::Cycle(outcome) => outcome,
        }
    }
}

impl<'hir> ConstraintUniverse<'hir> {
    /// Snapshot final functions, summaries, boundary rows and call sites.
    ///
    /// WHAT: one borrowed read-only universe over the base module and every completed sidecar, or
    ///       `None` when no function in either carries a source-constraint boundary.
    /// WHY: statement regions need the final HIR after phase-B write-back, and copying bodies or a
    ///      compact CFG would duplicate the very facts verification must read. Borrowed HIR keeps
    ///      the walk exact without a mutable sidecar borrow. Phase B has already published the
    ///      bits, so a module with no boundary needs no universe and no per-function analysis.
    pub(crate) fn build(
        base_hir: &'hir HirModule,
        base_report: &'hir BorrowCheckReport,
        transaction: &'hir GeneratedFunctionTransaction<'_>,
        external_registry: &'hir ExternalPackageRegistry,
    ) -> Result<Option<Self>, InfrastructureFailure> {
        let mut sidecar_identities = transaction
            .completed_link_facts()
            .map(|(identity, _)| identity.clone())
            .collect::<Vec<_>>();
        sidecar_identities.sort();
        sidecar_identities.dedup();

        let has_boundaries = module_has_constraint_boundaries(base_hir)
            || sidecar_identities.iter().any(|identity| {
                module_has_constraint_boundaries(
                    &transaction.sidecar(identity).module.executable.hir,
                )
            });
        if !has_boundaries {
            return Ok(None);
        }

        let mut modules = vec![collect_module(
            0,
            ConstraintModuleSite::Base,
            base_hir,
            base_report,
            None,
        )?];

        for identity in &sidecar_identities {
            let sidecar = transaction.sidecar(identity);
            modules.push(collect_module(
                modules.len(),
                ConstraintModuleSite::Sidecar(identity.clone()),
                &sidecar.module.executable.hir,
                &sidecar.module.executable.borrow_analysis,
                Some(sidecar.source_owner().clone()),
            )?);
        }

        let base_public_origins = modules[0].public_origins;
        let base_private_origins = modules[0].private_origins;

        let generated_targets = collect_generated_targets(&modules);

        let mut universe = Self {
            modules,
            generated_targets,
            base_public_origins,
            base_private_origins,
            external_leaves: collect_external_leaves(external_registry),
        };

        // Resolve every function against the final bits, then store the analyses. The resolver
        // borrows the universe immutably; analyses are collected separately and assigned after.
        let mut analyses = Vec::with_capacity(universe.modules.len());
        for (module_index, module) in universe.modules.iter().enumerate() {
            let mut module_analyses = Vec::with_capacity(module.functions.len());
            for facts in &module.functions {
                module_analyses.push(analyze_function(module.hir, facts.function, |target| {
                    universe.resolve_callee_facts(module_index, target)
                }));
            }
            analyses.push(module_analyses);
        }

        for (module, module_analyses) in universe.modules.iter_mut().zip(analyses) {
            for (facts, analysis) in module.functions.iter_mut().zip(module_analyses) {
                facts.analysis = analysis;
            }
        }

        Ok(Some(universe))
    }

    /// Verify every function boundary, stopping at the first module with diagnostics.
    ///
    /// WHAT: returns that module's site, frozen source owner and bag so the caller can capture it
    ///       with the module's type environment.
    /// WHY: base diagnostics must precede sidecar diagnostics and the first diagnosed module must
    ///       be the one whose prepared failure is reported. Returning the stored owner keeps the
    ///       span domain that produced the labels identical to the one frozen on the report.
    pub(crate) fn verify(
        &self,
        source_files: &SourceDatabase,
        string_table: &mut StringTable,
        path_fork: &PathInternerFork,
    ) -> Option<ConstraintFailure> {
        for module in &self.modules {
            let frozen_context = module.source_owner.as_ref().and_then(|owner| owner.get());
            let sources = SpanResolutionSources::new(Some(source_files), frozen_context);
            let mut bag = DiagnosticBag::new();
            for facts in &module.functions {
                for boundary in facts.boundaries() {
                    let drafts = match boundary.statement {
                        Some(_) => self.verify_statement_boundary(
                            module,
                            facts,
                            boundary,
                            &sources,
                            string_table,
                            path_fork,
                        ),
                        None => self.verify_whole_function_boundary(
                            facts,
                            boundary,
                            string_table,
                            path_fork,
                        ),
                    };
                    for draft in drafts.into_iter().flatten() {
                        bag.push(draft);
                    }
                }
            }
            if bag.has_errors() {
                return Some(ConstraintFailure {
                    site: module.site.clone(),
                    diagnostics: bag,
                });
            }
        }
        None
    }

    /// Verify one whole-function boundary.
    ///
    /// WHAT: returns at most one draft per guarantee, the safe-math draft before the infallible one.
    /// WHY: each guarantee labels its violation at the directive that declared it, and returning
    ///      drafts keeps the diagnostic sink with the caller that owns it.
    fn verify_whole_function_boundary(
        &self,
        facts: &ConstraintFunctionFacts,
        boundary: &HirConstraintBoundary,
        string_table: &mut StringTable,
        path_fork: &PathInternerFork,
    ) -> [Option<DiagnosticDraft>; 2] {
        let fast =
            if boundary.constraints.safe_math_active() && facts.analysis.bits.uses_fast_semantics {
                self.trace_witness(facts, Guarantee::Fast).map(|outcome| {
                    outcome.into_draft(
                        boundary.directive_spans.safe_math,
                        self,
                        facts.module_index,
                        string_table,
                        path_fork,
                    )
                })
            } else {
                None
            };
        let infallible = if boundary.constraints.infallible_active() {
            let guarantee = if facts.analysis.bits.may_trap {
                Some(Guarantee::Trap)
            } else if facts.analysis.bits.may_fail {
                Some(Guarantee::Fail)
            } else {
                None
            };
            guarantee
                .and_then(|guarantee| self.trace_witness(facts, guarantee))
                .map(|outcome| {
                    outcome.into_draft(
                        boundary.directive_spans.infallible,
                        self,
                        facts.module_index,
                        string_table,
                        path_fork,
                    )
                })
        } else {
            None
        };
        [fast, infallible]
    }

    /// Verify one statement boundary, skipping a boundary whose handler path is dead.
    ///
    /// WHAT: analyses the authored statement region once, then applies the guarantees this run
    ///       declares and returns at most one draft per guarantee.
    /// WHY: the region reuses the converged bits and call summaries, and a draft leaves diagnostic
    ///      emission to the caller instead of threading the bag through the region analysis.
    fn verify_statement_boundary(
        &self,
        module: &ConstraintModuleFacts<'hir>,
        facts: &ConstraintFunctionFacts,
        boundary: &HirConstraintBoundary,
        sources: &SpanResolutionSources<'_>,
        string_table: &mut StringTable,
        path_fork: &PathInternerFork,
    ) -> [Option<DiagnosticDraft>; 2] {
        let Some(statement) = boundary.statement else {
            // Whole-function boundaries are verified by their own path.
            return [None, None];
        };
        let region = analyze_statement_region(
            module.hir,
            facts.function,
            statement.start,
            statement.end,
            sources,
            &|target| self.resolve_callee_facts(facts.module_index, target),
        );
        if !region.has_live_region_site {
            return [None, None];
        }
        // A guarantee already declared by an enclosing boundary is checked there. A statement run
        // only owns the guarantees it authored, so an inherited guarantee keeps the declaring
        // boundary's span and is never reported twice. Whole-function boundaries differ: a
        // reconstructed set has no span yet must still be verified.
        let mut function_safe = false;
        let mut function_infallible = false;
        for function_boundary in facts.boundaries() {
            if function_boundary.statement.is_some() {
                continue;
            }
            function_safe |= function_boundary.constraints.safe_math_active();
            function_infallible |= function_boundary.constraints.infallible_active();
        }

        let fast = if !function_safe && boundary.directive_spans.safe_math.is_some() {
            region
                .fast_witness
                .and_then(|witness| self.trace_region_witness(facts, Guarantee::Fast, witness))
                .map(|outcome| {
                    outcome.into_draft(
                        boundary.directive_spans.safe_math,
                        self,
                        facts.module_index,
                        string_table,
                        path_fork,
                    )
                })
        } else {
            None
        };
        let infallible = if !function_infallible && boundary.directive_spans.infallible.is_some() {
            let violation = if let Some(witness) = region.trap_witness {
                Some((Guarantee::Trap, witness))
            } else {
                region
                    .fail_witness
                    .map(|witness| (Guarantee::Fail, witness))
            };
            violation
                .and_then(|(guarantee, witness)| {
                    self.trace_region_witness(facts, guarantee, witness)
                })
                .map(|outcome| {
                    outcome.into_draft(
                        boundary.directive_spans.infallible,
                        self,
                        facts.module_index,
                        string_table,
                        path_fork,
                    )
                })
        } else {
            None
        };
        [fast, infallible]
    }

    /// Follow one guarantee's selected local witness through at most three call hops.
    ///
    /// A recoverable failure witness is already the escaping edge itself, so `Fail` returns it
    /// directly and never walks the call graph.
    fn trace_witness(
        &self,
        facts: &ConstraintFunctionFacts,
        guarantee: Guarantee,
    ) -> Option<WitnessOutcome> {
        let Some(guarantee) = guarantee.trace_guarantee() else {
            return direct_outcome(facts.analysis.fail_witness?, facts.module_index, &[]);
        };
        let mut visited = FxHashSet::default();
        self.trace_witness_from(facts, guarantee, &mut visited, Vec::new())
            .map(TracedWitness::into_outcome)
    }

    fn trace_witness_from(
        &self,
        facts: &ConstraintFunctionFacts,
        guarantee: TraceGuarantee,
        visited: &mut FxHashSet<(usize, FunctionId)>,
        hops: Vec<Option<WitnessSpan>>,
    ) -> Option<TracedWitness> {
        if !visited.insert((facts.module_index, facts.function)) {
            return Some(TracedWitness::Cycle(WitnessOutcome {
                reason: guarantee.recursive_cycle_reason(),
                evidence: None,
                name: facts
                    .name_path
                    .map(EvidenceName::Path)
                    .or_else(|| Some(EvidenceName::Owned("<recursive function>".to_owned()))),
                hops,
            }));
        }

        // A local fast ledger witness is always direct evidence. Prefer a spaced witness over a
        // generated operation with no authored span so a range guard or write-back is named
        // instead of a generated ordinal with no location.
        if guarantee == TraceGuarantee::Fast
            && let Some(witness) = select_fast_use_witness(facts.fast_use())
        {
            return Some(TracedWitness::Evidence(WitnessOutcome {
                reason: witness.0,
                evidence: witness.1.map(|span| WitnessSpan {
                    module_index: facts.module_index,
                    span,
                }),
                name: None,
                hops,
            }));
        }

        let witness = guarantee.witness(&facts.analysis)?;
        self.trace_witness_kind(facts, guarantee, witness, visited, hops)
    }

    /// Trace one statement-region witness. The region already selected the first local fact, so
    /// only the call-chain continuation is shared with the whole-function path.
    fn trace_region_witness(
        &self,
        facts: &ConstraintFunctionFacts,
        guarantee: Guarantee,
        witness: ConstraintWitness,
    ) -> Option<WitnessOutcome> {
        let Some(guarantee) = guarantee.trace_guarantee() else {
            return direct_outcome(witness, facts.module_index, &[]);
        };
        let mut visited = FxHashSet::default();
        visited.insert((facts.module_index, facts.function));
        self.trace_witness_kind(facts, guarantee, witness, &mut visited, Vec::new())
            .map(TracedWitness::into_outcome)
    }

    fn trace_witness_kind(
        &self,
        facts: &ConstraintFunctionFacts,
        guarantee: TraceGuarantee,
        witness: ConstraintWitness,
        visited: &mut FxHashSet<(usize, FunctionId)>,
        hops: Vec<Option<WitnessSpan>>,
    ) -> Option<TracedWitness> {
        if let Some(outcome) = direct_outcome(witness, facts.module_index, &hops) {
            return Some(TracedWitness::Evidence(outcome));
        }

        if matches!(
            witness.kind,
            ConstraintWitnessKind::FastUnknownExternal | ConstraintWitnessKind::TrapUnknownExternal
        ) {
            let resolved = self.resolve_target_at_span(facts, witness.span, guarantee);
            return Some(TracedWitness::Evidence(self.leaf_outcome(
                resolved,
                guarantee,
                witness.span.map(|span| WitnessSpan {
                    module_index: facts.module_index,
                    span,
                }),
                hops,
            )));
        }

        // The remaining witness kinds name a call. Try the selected site first, then every other
        // call site in this function that carries the guarantee bit, so a recursive cycle on one
        // edge cannot hide a leaf reachable from another. Phase B and this walk use the same
        // resolver, so at least one candidate must carry the bit.
        let mut cycle = None;
        for call in self.ordered_call_candidates(facts, witness.span, guarantee) {
            let resolved = self.resolve_target(facts.module_index, &call.target);
            match resolved {
                ResolvedTarget::Function { module, index } => {
                    let callee_facts = &self.modules[module].functions[index];
                    if hops.len() == MAX_CALL_HOPS {
                        return Some(TracedWitness::Evidence(WitnessOutcome {
                            reason: guarantee.beyond_bound_reason(),
                            evidence: None,
                            name: callee_facts.name_path.map(EvidenceName::Path),
                            hops,
                        }));
                    }
                    let mut callee_hops = hops.clone();
                    callee_hops.push(call.span.map(|span| WitnessSpan {
                        module_index: facts.module_index,
                        span,
                    }));
                    match self.trace_witness_from(callee_facts, guarantee, visited, callee_hops) {
                        Some(TracedWitness::Evidence(outcome)) => {
                            return Some(TracedWitness::Evidence(outcome));
                        }
                        Some(TracedWitness::Cycle(outcome)) => {
                            cycle.get_or_insert(outcome);
                        }
                        None => {}
                    }
                }
                ResolvedTarget::Leaf(_) | ResolvedTarget::Missing { .. } => {
                    return Some(TracedWitness::Evidence(self.leaf_outcome(
                        resolved,
                        guarantee,
                        call.span.map(|span| WitnessSpan {
                            module_index: facts.module_index,
                            span,
                        }),
                        hops,
                    )));
                }
            }
        }

        if let Some(outcome) = cycle {
            return Some(TracedWitness::Cycle(outcome));
        }
        Some(TracedWitness::Evidence(WitnessOutcome {
            reason: guarantee.unknown_reason(),
            evidence: witness.span.map(|span| WitnessSpan {
                module_index: facts.module_index,
                span,
            }),
            name: None,
            hops,
        }))
    }

    /// Call sites to try for one call witness, most specific first.
    ///
    /// WHAT: calls at the selected witness span that carry the guarantee bit, then every other
    ///       call site in the function that carries it.
    /// WHY: the analyzer reports the first local call witness. When that edge closes a recursive
    ///      cycle, a later call site can still reach the evidence that made the bit true.
    fn ordered_call_candidates<'a>(
        &self,
        facts: &'a ConstraintFunctionFacts,
        span: Option<SourceSpan>,
        guarantee: TraceGuarantee,
    ) -> Vec<&'a HirSelectedCallSite> {
        let mut candidates = Vec::new();
        if span.is_some() {
            candidates.extend(
                facts
                    .calls
                    .iter()
                    .filter(|call| call.span == span)
                    .filter(|call| self.call_carries(call, facts.module_index, guarantee)),
            );
        }
        candidates.extend(
            facts
                .calls
                .iter()
                .filter(|call| !(span.is_some() && call.span == span))
                .filter(|call| self.call_carries(call, facts.module_index, guarantee)),
        );
        candidates
    }

    fn call_carries(
        &self,
        call: &HirSelectedCallSite,
        caller_module: usize,
        guarantee: TraceGuarantee,
    ) -> bool {
        guarantee.bit(self.resolve_target(caller_module, &call.target).bits(self))
    }

    fn leaf_outcome(
        &self,
        resolved: ResolvedTarget<'_>,
        guarantee: TraceGuarantee,
        evidence: Option<WitnessSpan>,
        hops: Vec<Option<WitnessSpan>>,
    ) -> WitnessOutcome {
        let reason = if resolved.unknown() {
            guarantee.unknown_reason()
        } else {
            guarantee.unavailable_reason()
        };
        let name = resolved
            .name()
            .map(|name| EvidenceName::Owned(name.to_owned()));
        WitnessOutcome {
            reason,
            evidence,
            name,
            hops,
        }
    }

    fn resolve_target_at_span(
        &self,
        facts: &ConstraintFunctionFacts,
        span: Option<SourceSpan>,
        guarantee: TraceGuarantee,
    ) -> ResolvedTarget<'_> {
        self.callee_at_span(facts, span, guarantee)
            .unwrap_or_else(|| ResolvedTarget::Missing {
                name: "<unknown callable>".to_owned(),
            })
    }

    fn callee_at_span(
        &self,
        facts: &ConstraintFunctionFacts,
        span: Option<SourceSpan>,
        guarantee: TraceGuarantee,
    ) -> Option<ResolvedTarget<'_>> {
        for call in &facts.calls {
            if span.is_some() && call.span != span {
                continue;
            }
            let resolved = self.resolve_target(facts.module_index, &call.target);
            if guarantee.bit(resolved.bits(self)) {
                return Some(resolved);
            }
        }
        None
    }

    fn resolve_callee_facts(
        &self,
        caller_module: usize,
        target: &CallTarget,
    ) -> ConstraintCalleeFacts {
        match self.resolve_target(caller_module, target) {
            ResolvedTarget::Function { module, index } => {
                ConstraintCalleeFacts::from_bits(self.modules[module].functions[index].summary)
            }
            ResolvedTarget::Leaf(leaf) => leaf.callee_facts(),
            ResolvedTarget::Missing { .. } => ConstraintCalleeFacts::unknown_external(),
        }
    }

    fn resolve_target(&self, caller_module: usize, target: &CallTarget) -> ResolvedTarget<'_> {
        match target {
            CallTarget::Local(function) => self.modules[caller_module]
                .function_by_id
                .get(function)
                .map(|index| ResolvedTarget::Function {
                    module: caller_module,
                    index: *index,
                })
                .unwrap_or_else(|| ResolvedTarget::Missing {
                    name: "<local function>".to_owned(),
                }),
            CallTarget::CrossModule(origin) => {
                if let Some(function) = self.base_public_origins.get(origin) {
                    return self.function_target(0, *function);
                }
                if let Some(leaf) = self.modules[caller_module].imported_leaves.get(origin) {
                    return ResolvedTarget::Leaf(leaf);
                }
                if let Some(leaf) = self.modules[0].imported_leaves.get(origin) {
                    return ResolvedTarget::Leaf(leaf);
                }
                ResolvedTarget::Missing {
                    name: origin.defining_name().to_owned(),
                }
            }
            CallTarget::ModulePrivate(identity) => {
                if let Some(function) = self.modules[caller_module]
                    .private_origins
                    .get(identity)
                    .copied()
                {
                    return self.function_target(caller_module, function);
                }
                if let Some(function) = self.base_private_origins.get(identity) {
                    return self.function_target(0, *function);
                }
                if let Some(leaf) = self.modules[caller_module].private_leaves.get(identity) {
                    return ResolvedTarget::Leaf(leaf);
                }
                if let Some(leaf) = self.modules[0].private_leaves.get(identity) {
                    return ResolvedTarget::Leaf(leaf);
                }
                ResolvedTarget::Missing {
                    name: identity.defining_name().to_owned(),
                }
            }
            CallTarget::Generated(identity) => {
                if let Some((module, index)) =
                    self.generated_targets.get(identity.as_ref()).copied()
                {
                    debug_assert!(
                        self.modules[caller_module]
                            .generated_leaves
                            .get(identity.as_ref())
                            .is_none_or(|leaf| {
                                leaf.bits == self.modules[module].functions[index].summary
                            }),
                        "verification and phase B disagree about {identity:?} converged bits"
                    );
                    return ResolvedTarget::Function { module, index };
                }
                if let Some(leaf) = self.modules[caller_module]
                    .generated_leaves
                    .get(identity.as_ref())
                {
                    return ResolvedTarget::Leaf(leaf);
                }
                ResolvedTarget::Missing {
                    name: identity.declaration().defining_name().to_owned(),
                }
            }
            CallTarget::External(function) => match self.external_leaves.get(function) {
                Some(leaf) => ResolvedTarget::Leaf(leaf),
                None => ResolvedTarget::Missing {
                    name: function.name().to_owned(),
                },
            },
        }
    }

    fn function_target(&self, module: usize, function: FunctionId) -> ResolvedTarget<'_> {
        let Some(index) = self.modules[module].function_by_id.get(&function).copied() else {
            return ResolvedTarget::Missing {
                name: "<local function>".to_owned(),
            };
        };
        ResolvedTarget::Function { module, index }
    }
}

impl WitnessOutcome {
    /// Build the diagnostic draft and point every spanned site at its owning source context.
    ///
    /// WHAT: the boundary directive and each label whose module issued it carry that module's
    ///       frozen owner handle.
    /// WHY: the prepared report renders the directive and its witness in the reporting domain,
    ///      but source identities are only unique inside one domain. Attaching each owner here,
    ///      rather than letting a later pass paint every label with the reporting module's owner,
    ///      keeps a materialised body's evidence in its declaring source file and leaves
    ///      base-module labels on the live database.
    fn into_draft(
        self,
        directive_span: Option<SourceSpan>,
        universe: &ConstraintUniverse<'_>,
        boundary_module: usize,
        string_table: &mut StringTable,
        path_fork: &PathInternerFork,
    ) -> DiagnosticDraft {
        let callee = match self.name {
            Some(EvidenceName::Owned(name)) => Some(string_table.get_or_intern(name)),
            Some(EvidenceName::Path(path)) => path_fork.component(path),
            None => None,
        };
        let hop_spans = self
            .hops
            .iter()
            .map(|hop| hop.map(|hop| hop.span))
            .collect::<Vec<_>>();
        let evidence_span = self.evidence.map(|evidence| evidence.span);
        let mut draft = invalid_source_constraint(
            self.reason,
            callee,
            directive_span,
            evidence_span,
            &hop_spans,
        );
        if directive_span.is_some()
            && let Some(owner) = universe.modules[boundary_module].source_owner.as_ref()
        {
            draft.set_primary_frozen_identity_handle_if_missing(owner.clone());
        }
        for (index, hop) in self.hops.iter().enumerate() {
            if let Some(hop) = hop {
                draft = attach_label_owner(draft, index, *hop, |module| {
                    universe.modules[module].source_owner.as_ref()
                });
            }
        }
        if let Some(evidence) = self.evidence {
            draft = attach_label_owner(draft, self.hops.len(), evidence, |module| {
                universe.modules[module].source_owner.as_ref()
            });
        }
        draft
    }
}

/// Point one label at the frozen source context of the module that owns its span.
///
/// WHAT: attaches the owner handle for the label's own module whenever that module has one.
/// WHY: the diagnostic's labels each carry a span from one verification module. The boundary
///      module's handle is the same declaring-domain handle convergence attaches to the primary
///      directive, so attaching it per label here keeps one owner rule instead of relying on a
///      blanket pass. Base-module labels have no handle because their spans resolve against the
///      report's live source database. A sidecar can reach a base-owned span only through a base
///      local function of its own declaring module, whose sources its frozen context covers, so a
///      handle-free base label still resolves in the right domain.
fn attach_label_owner<'a>(
    draft: DiagnosticDraft,
    label_index: usize,
    span: WitnessSpan,
    owner_of: impl Fn(usize) -> Option<&'a FrozenIdentityHandle>,
) -> DiagnosticDraft {
    let Some(owner) = owner_of(span.module_index) else {
        return draft;
    };
    draft.with_label_frozen_identity_handle(label_index, owner.clone())
}

/// Choose the most informative selected fast-use witness for a diagnostic.
///
/// WHAT: the first witness with an authored span, or the first witness when every row is generated.
/// WHY: a fast range loop also generates an ordinal operation with no authored span. The range
///      guard or write-back is the evidence the user can act on, so a spaced row wins.
fn select_fast_use_witness(
    fast_use: &[HirFastUseWitness],
) -> Option<(InvalidSourceConstraintReason, Option<SourceSpan>)> {
    let witness = fast_use
        .iter()
        .find(|witness| witness.span.is_some())
        .or_else(|| fast_use.first())?;
    let reason = match witness.provenance {
        HirFastUseProvenance::AuthoredArithmetic => InvalidSourceConstraintReason::FastOperation,
        HirFastUseProvenance::RangeGuard => InvalidSourceConstraintReason::FastRangeGuard,
        HirFastUseProvenance::StoreConversion => InvalidSourceConstraintReason::FastWriteBack,
    };
    Some((reason, witness.span))
}

/// The direct reason one local witness proves, or `None` when the witness names a call.
fn direct_reason(
    kind: ConstraintWitnessKind,
    trap_causes: TrapCauseMask,
) -> Option<InvalidSourceConstraintReason> {
    use InvalidSourceConstraintReason as Reason;
    Some(match kind {
        ConstraintWitnessKind::FastArithmetic => Reason::FastOperation,
        ConstraintWitnessKind::FastRangeGuard => Reason::FastRangeGuard,
        ConstraintWitnessKind::FastStoreConversion => Reason::FastWriteBack,
        ConstraintWitnessKind::TrapArithmetic => {
            if trap_causes.contains(BuiltinErrorCode::DivideByZero)
                && trap_causes.contains(BuiltinErrorCode::FloatNonFinite)
            {
                Reason::TrapFloatDivision
            } else if trap_causes.contains(BuiltinErrorCode::DivideByZero)
                && trap_causes.contains(BuiltinErrorCode::IntOverflow)
            {
                // Signed `//` carries both causes, so its message must name both.
                Reason::TrapDivision
            } else if trap_causes.contains(BuiltinErrorCode::DivideByZero) {
                Reason::TrapDivideByZero
            } else if trap_causes.contains(BuiltinErrorCode::InvalidExponent) {
                Reason::TrapExponent
            } else if trap_causes.contains(BuiltinErrorCode::FloatNonFinite) {
                Reason::TrapFloatNonFinite
            } else if trap_causes.contains(BuiltinErrorCode::IntOverflow) {
                Reason::TrapOverflow
            } else {
                // The trap-cause tables only emit the four causes handled above, and a witness
                // is recorded only when at least one cause remains.
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "trap arithmetic witness lost its builtin cause"
                )
            }
        }
        ConstraintWitnessKind::TrapRangeGuard => Reason::TrapRangeGuard,
        ConstraintWitnessKind::TrapFastStoreConversion => Reason::TrapWriteBack,
        ConstraintWitnessKind::TrapAssertion => Reason::TrapAssertion,
        ConstraintWitnessKind::TrapFloatValidation => Reason::TrapFloatValidation,
        ConstraintWitnessKind::TrapConversion => Reason::TrapConversion,
        ConstraintWitnessKind::EscapingError => Reason::EscapingError,
        ConstraintWitnessKind::FastCallee
        | ConstraintWitnessKind::FastUnknownExternal
        | ConstraintWitnessKind::TrapCallee
        | ConstraintWitnessKind::TrapUnknownExternal => return None,
    })
}

/// Build a direct witness outcome, or `None` when the witness names a call.
fn direct_outcome(
    witness: ConstraintWitness,
    module_index: usize,
    hops: &[Option<WitnessSpan>],
) -> Option<WitnessOutcome> {
    let reason = direct_reason(witness.kind, witness.trap_causes)?;
    Some(WitnessOutcome {
        reason,
        evidence: witness.span.map(|span| WitnessSpan { module_index, span }),
        name: None,
        hops: hops.to_vec(),
    })
}

/// Whether any function in one module carries a source-constraint boundary.
fn module_has_constraint_boundaries(hir: &HirModule) -> bool {
    hir.function_failure_facts
        .values()
        .any(|facts| !facts.constraint.boundaries.is_empty())
}

/// Build the one generated-identity lookup shared by every diagnostic resolution.
///
/// WHAT: copies each final HIR's `function_ids_by_generated` row once, keyed by identity.
/// WHY: an owned generated body can be reached from the base or from any completed sidecar, so a
///      caller-local map cannot decide ownership. Identities absent here name provider-owned or
///      otherwise unavailable leaves, which stay immutable summary evidence.
fn collect_generated_targets(
    modules: &[ConstraintModuleFacts<'_>],
) -> FxHashMap<GeneratedFunctionIdentity, (usize, usize)> {
    let mut targets = FxHashMap::default();
    for (module_index, module) in modules.iter().enumerate() {
        for (identity, function) in module.hir.function_ids_by_generated.iter() {
            if let Some(index) = module.function_by_id.get(function) {
                targets.insert(identity.clone(), (module_index, *index));
            }
        }
    }
    targets
}

fn collect_module<'hir>(
    module_index: usize,
    site: ConstraintModuleSite,
    hir: &'hir HirModule,
    report: &BorrowCheckReport,
    source_owner: Option<FrozenIdentityHandle>,
) -> Result<ConstraintModuleFacts<'hir>, InfrastructureFailure> {
    let mut function_ids = hir
        .functions
        .iter()
        .map(|function| function.id)
        .collect::<Vec<_>>();
    function_ids.sort_unstable_by_key(|function| function.0);

    let mut functions = Vec::with_capacity(function_ids.len());
    let mut function_by_id = FxHashMap::default();
    for function in function_ids {
        let summary = report
            .analysis
            .public_call_summaries
            .get(&function)
            .map(ConstraintSummaryBits::from_summary)
            .ok_or_else(|| missing_call_summary(function))?;
        let constraint_evidence = hir
            .function_failure_facts
            .get(&function)
            .map(|facts| &facts.constraint);
        let index = functions.len();
        function_by_id.insert(function, index);
        functions.push(ConstraintFunctionFacts {
            module_index,
            function,
            summary,
            analysis: FunctionConstraintAnalysis::default(),
            constraint_evidence,
            calls: collect_function_call_sites(hir, function),
            name_path: hir.side_table.function_name_path(function),
        });
    }

    let imported_leaves = hir
        .imported_call_summaries
        .iter()
        .map(|(origin, summary)| {
            (
                origin.clone(),
                LeafFacts {
                    bits: ConstraintSummaryBits::from_summary(summary),
                    name: origin.defining_name().to_owned(),
                    unknown: false,
                    external: false,
                },
            )
        })
        .collect();
    let private_leaves = hir
        .module_private_call_summaries
        .iter()
        .map(|(identity, summary)| {
            (
                identity.clone(),
                LeafFacts {
                    bits: ConstraintSummaryBits::from_summary(summary),
                    name: identity.defining_name().to_owned(),
                    unknown: false,
                    external: false,
                },
            )
        })
        .collect();
    let generated_leaves = hir
        .generated_call_summaries
        .iter()
        .map(|(identity, summary)| {
            (
                identity.clone(),
                LeafFacts {
                    bits: ConstraintSummaryBits::from_summary(summary),
                    name: identity.declaration().defining_name().to_owned(),
                    unknown: false,
                    external: false,
                },
            )
        })
        .collect();

    Ok(ConstraintModuleFacts {
        site,
        hir,
        source_owner,
        functions,
        function_by_id,
        public_origins: &hir.function_ids_by_origin,
        private_origins: &hir.function_ids_by_private_origin,
        imported_leaves,
        private_leaves,
        generated_leaves,
    })
}

/// A missing summary is an infrastructure failure, never a default seed.
///
/// WHAT: mirrors phase B's missing-summary handling for the verification snapshot.
/// WHY: `ConstraintSummaryBits::default()` means ordinary, total and failure-free, so defaulting
///      a missing summary would turn absent evidence into a proof of absence.
fn missing_call_summary(function: FunctionId) -> InfrastructureFailure {
    InfrastructureFailure::internal(
        CompilerStage::ProjectBuildAssembly,
        format!("constraint verification found no call summary for {function:?}"),
    )
}

fn collect_external_leaves(
    registry: &ExternalPackageRegistry,
) -> FxHashMap<ExternalFunctionId, LeafFacts> {
    registry
        .functions()
        .map(|(id, definition)| {
            let facts = ConstraintCalleeFacts::from_external_contract(definition.numeric_contract);
            let name = match (
                registry.resolve_function_package(id),
                registry.resolve_function_symbol_path(id),
            ) {
                (Some(package), Some(path)) => format!("{package}.{}", path.display_text()),
                _ => definition.name.clone(),
            };
            (
                id,
                LeafFacts {
                    bits: facts.bits,
                    name,
                    unknown: facts.unknown_external,
                    external: true,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_frontend::module_compilation::generated::{
        CompletedGeneratedFunction, GeneratedFunctionTransaction, KnownGeneratedFunctions,
    };
    use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
    use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId};
    use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
    use crate::compiler_frontend::tests::hir_fixture_support::lower_hir;
    use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
    use rustc_hash::FxHashMap;

    /// One span in a fixed test source.
    fn exact_span(start: u32, length: u32) -> SourceSpan {
        let mut extended = ExtendedSpanBuilder::new();
        let local =
            LocalSpan::exact(start, length, &mut extended).expect("test span sits below the limit");
        SourceSpan::new(SourceId::from_index(1), local)
    }

    /// A witness that crosses a sidecar boundary keeps each label in its owning source domain.
    #[test]
    fn a_cross_module_witness_label_names_its_own_source_domain() {
        let boundary_owner =
            FrozenIdentityHandle::for_domain(StablePackageIdentity::project_local("boundary"));
        let helper_owner =
            FrozenIdentityHandle::for_domain(StablePackageIdentity::project_local("helper"));
        let owners = [
            None,
            Some(boundary_owner.clone()),
            Some(helper_owner.clone()),
        ];
        let directive = exact_span(0, 5);
        let boundary_hop = exact_span(10, 5);
        let base_hop = exact_span(20, 5);
        let evidence = exact_span(30, 3);
        let draft = invalid_source_constraint(
            InvalidSourceConstraintReason::FastOperation,
            None,
            Some(directive),
            Some(evidence),
            &[Some(boundary_hop), Some(base_hop)],
        );
        let owner_of = |module: usize| owners[module].as_ref();
        let draft = attach_label_owner(
            draft,
            0,
            WitnessSpan {
                module_index: 1,
                span: boundary_hop,
            },
            owner_of,
        );
        let draft = attach_label_owner(
            draft,
            1,
            WitnessSpan {
                module_index: 0,
                span: base_hop,
            },
            owner_of,
        );
        let draft = attach_label_owner(
            draft,
            2,
            WitnessSpan {
                module_index: 2,
                span: evidence,
            },
            owner_of,
        );

        assert!(
            draft.has_label_at_with_owner(Some(boundary_hop), Some(&boundary_owner)),
            "a boundary-module hop names the boundary module's declaring domain"
        );
        assert!(
            draft.has_label_at_with_owner(Some(base_hop), None),
            "the base module has no frozen handle and keeps live-resolution labels"
        );
        assert!(
            draft.has_label_at_with_owner(Some(evidence), Some(&helper_owner)),
            "the evidence label names the module that owns its source domain"
        );
        assert!(
            !draft.has_label_at_with_owner(Some(evidence), Some(&boundary_owner)),
            "the evidence label must not fall back to the boundary's domain"
        );
    }

    /// Lower one source into a base module and ask for the verification snapshot.
    fn builds_universe(source: &str) -> Result<bool, InfrastructureFailure> {
        let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
        let registry = default_external_package_registry(&mut string_table);
        let hir = lower_hir(ast, &mut string_table, &mut path_fork, &registry);
        let records: Vec<CompletedGeneratedFunction> = Vec::new();
        let by_identity = FxHashMap::default();
        let known = KnownGeneratedFunctions::new(&records, &by_identity);
        let transaction = GeneratedFunctionTransaction::new(known);
        let report = BorrowCheckReport::default();
        ConstraintUniverse::build(&hir, &report, &transaction, registry.as_ref())
            .map(|universe| universe.is_some())
    }

    #[test]
    fn an_unconstrained_module_skips_the_verification_universe() {
        let mut source = String::new();
        for index in 0..32 {
            source.push_str(&format!(
                "plain_{index} |value I32| -> I32:\n    return value\n;\n\n"
            ));
        }
        let built = builds_universe(&source).expect("no boundary means no snapshot");
        assert!(
            !built,
            "a module without boundaries must not pay for per-function analysis"
        );
    }

    #[test]
    fn a_boundary_without_a_call_summary_is_an_infrastructure_failure() {
        let error = builds_universe("$safe_math\nplain || -> I32:\n    return 1\n;\n")
            .expect_err("a boundary requires a call summary");
        assert!(
            format!("{error:?}").contains("found no call summary"),
            "missing summaries must never default to ordinary, total evidence: {error:?}"
        );
    }
}
