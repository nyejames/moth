//! Immutable per-function semantic facts, projected once while HIR is built.
//!
//! Summary convergence reads direct unhandled producers and resolved call targets here, without
//! retaining expression trees or changing the existing runtime HIR failure delivery. Failure
//! contributors come from the pre-fold typed AST facts with authored-unreachable tails removed;
//! release erasure neither adds, removes nor reorders one. Constraint evidence comes from the
//! authored boundaries plus the selected HIR, including the call sites and fast-use witnesses of
//! skipped catch handlers and statements after a terminator. Failure facts are classified before
//! release folding and projected onto HIR call targets during construction; constraint evidence is
//! captured then, while the typed AST bodies still exist, so verification can read this record
//! after they are gone.

use crate::compiler_frontend::ast::AstConstraintStatementSpan;
use crate::compiler_frontend::ast::expressions::failure_facts::FailureWitnessSite;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::style_directives::{ConstraintDirectiveSpans, SourceConstraintSet};
use crate::compiler_frontend::traits::evidence::environment::InfallibleCastEvidenceContract;

#[derive(Debug, Clone)]
pub(crate) struct HirFunctionFailureFacts {
    pub(crate) span: Option<SourceSpan>,
    pub(crate) boundary: HirBuiltinFailureBoundary,
    /// Source evidence promises no escaping failure, independently of ordinary private inference.
    pub(crate) infallible_cast_evidence: Option<InfallibleCastEvidenceContract>,
    pub(crate) contributors: Vec<HirBuiltinFailureContributor>,
    /// Deferred assertion-message checks are not escaping function producers.
    pub(crate) assertion_message_calls: Vec<HirBuiltinFailureContributor>,
    /// Catch eligibility and compatibility wait for private-call summaries to converge.
    pub(crate) deferred_custom_catches: Vec<HirDeferredCustomCatchCheck>,
    /// Source constraint boundaries and selected fast-use evidence, projected while HIR is built
    /// and before catch-handler pruning.
    pub(crate) constraint: HirConstraintEvidence,
}

/// Constraint evidence for one HIR function, after inactive static-`if` pruning.
///
/// WHAT: the function's projected constraint boundaries, selected fast-use witnesses and every
///       direct call target in selected code, recorded before catch-handler pruning.
/// WHY: verification runs after directive syntax and AST bodies are gone, so both facts must be
///      projected once into the function that owns them. Fast semantics are selected-code
///      evidence: a fast callee called only from a handler that later becomes unrouted still
///      counts. The final CFG cannot recover those pruned calls, so the target list is frozen
///      here while the handler blocks still exist.
#[derive(Debug, Clone, Default)]
pub(crate) struct HirConstraintEvidence {
    /// Function and statement boundaries that survived inactive static-`if` pruning.
    pub(crate) boundaries: Vec<HirConstraintBoundary>,
    /// Fast-use witnesses, deduplicated and in deterministic order.
    pub(crate) fast_use: Vec<HirFastUseWitness>,
    /// Every direct call site in selected code before catch-handler pruning.
    ///
    /// WHAT: one row per distinct `(target, authored span)` occurrence, in structural order.
    /// WHY: region verification must see a second call to the same callee inside an annotated
    ///      statement even when an earlier call to that callee is outside it. Phase B derives its
    ///      target-only edge view from this list at the consumer.
    pub(crate) selected_call_sites: Vec<HirSelectedCallSite>,
    /// Selected call sites erased by release folding whose enclosing expression HIR lowered.
    ///
    /// WHAT: the release-erased calls with the failure containment debug lowering preserved.
    /// WHY: `selected_call_sites` intentionally carries fast-use evidence without regard to
    ///      liveness, because fast selection is not a liveness property. A callee's trap or
    ///      escaping-failure hazard is liveness-conditioned, though, so `$infallible` may only
    ///      treat the erased call as executable when its surrounding expression was lowered.
    ///      Statically pruned catch handlers are excluded from release speculation, so they
    ///      produce no erased rows.
    pub(crate) live_erased_call_sites: Vec<HirLiveErasedCallSite>,
}

/// One direct call site recorded before catch handlers are pruned.
///
/// WHAT: the stable call target and its authored statement span when one exists.
/// WHY: fast-use propagation and reverse call edges must include calls that no longer appear in
///      the final CFG. The site, not just the target, is the unit: two calls to one callee at
///      different spans are independently selected evidence. Call targets are identity-based and
///      are not rewritten by lane installation or catch pruning, so the recorded value stays
///      exact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HirSelectedCallSite {
    pub(crate) target: CallTarget,
    pub(crate) span: Option<SourceSpan>,
}

/// One live release-erased call, including the failure containment debug lowering preserved.
///
/// WHAT: the resolved target, authored span and whether an enclosing catch owns its recoverable
///       failure.
/// WHY: an erased call retains its trap hazard, but only an escaping call widens `may_fail`.
///      Reusing [`HirSelectedCallSite`] would force every selected-site producer to carry a
///      containment bit it cannot know. The span is required because every projected row carries
///      its authored call site; release folding skips authored-dead statements and statically
///      pruned catch handlers, so a recorded row always belongs to source HIR lowered.
#[derive(Clone, Debug)]
pub(crate) struct HirLiveErasedCallSite {
    pub(crate) target: CallTarget,
    pub(crate) span: SourceSpan,
    pub(crate) failure_caught: bool,
}

/// The one accumulator for [`HirSelectedCallSite`] rows.
///
/// WHAT: insertion-ordered sites with duplicate `(target, span)` rows dropped.
/// WHY: HIR projection and the skipped-handler walk both add selected calls to one per-function
///      fact. A shared accumulator keeps their ordering and deduplication rule identical, and the
///      hash set replaces a linear membership scan per recorded call.
#[derive(Debug, Default)]
pub(crate) struct HirSelectedCallSiteLedger {
    sites: Vec<HirSelectedCallSite>,
    seen: rustc_hash::FxHashSet<(CallTarget, Option<SourceSpan>)>,
}

impl HirSelectedCallSiteLedger {
    pub(crate) fn push(&mut self, target: &CallTarget, span: Option<SourceSpan>) {
        if self.seen.insert((target.clone(), span)) {
            self.sites.push(HirSelectedCallSite {
                target: target.clone(),
                span,
            });
        }
    }

    pub(crate) fn into_sites(self) -> Vec<HirSelectedCallSite> {
        self.sites
    }
}

/// Evidence from selected AST code that HIR lowering deliberately omits.
///
/// WHAT: the selected call sites and fast-use witnesses of skipped catch handlers and statements
///       after a terminator.
/// WHY: those statements have no HIR blocks, but they are still selected source: their calls
///      select a callee's fast semantics and a `$fast_math` loop selects a fast range guard.
///      One per-function accumulator keeps both evidence classes on the skip path, and projection
///      applies the same inactive-static-branch filter as the authored evidence ledger.
///
/// The sites stay in recorded order. Projection merges them into a
/// [`HirSelectedCallSiteLedger`] beside the live sites, which keeps the first occurrence of any
/// row both sets share.
#[derive(Debug, Default)]
pub(crate) struct HirSkippedSourceEvidence {
    call_sites: Vec<HirSelectedCallSite>,
    fast_use: Vec<HirFastUseWitness>,
}

impl HirSkippedSourceEvidence {
    pub(crate) fn record_call(&mut self, target: CallTarget, span: Option<SourceSpan>) {
        self.call_sites.push(HirSelectedCallSite { target, span });
    }

    pub(crate) fn record_fast_use(
        &mut self,
        provenance: HirFastUseProvenance,
        span: Option<SourceSpan>,
    ) {
        self.fast_use.push(HirFastUseWitness { provenance, span });
    }

    pub(crate) fn call_sites(&self) -> std::slice::Iter<'_, HirSelectedCallSite> {
        self.call_sites.iter()
    }

    pub(crate) fn fast_use(&self) -> &[HirFastUseWitness] {
        &self.fast_use
    }
}

/// One lexical constraint boundary projected onto a HIR function.
///
/// WHAT: the statement range (`None` means the whole function), the active constraint set and the
///       authored directive spans of the guarantees this boundary declares itself.
/// WHY: statement verification selects HIR operations by authored span within this range; inherited
///      constraints intentionally carry the same set as the enclosing boundary while leaving the
///      inherited guarantee's span empty, so only its declaring boundary labels a violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HirConstraintBoundary {
    pub(crate) statement: Option<AstConstraintStatementSpan>,
    pub(crate) constraints: SourceConstraintSet,
    pub(crate) directive_spans: ConstraintDirectiveSpans,
}

/// Provenance of one selected fast-use witness.
///
/// WHAT: authored fast arithmetic (whether or not it survived into HIR), a generated fast range
///       guard, or a fast compound-assignment write-back conversion.
/// WHY: diagnostics distinguish a range guard and write-back from ordinary fast arithmetic, while
///      an authored operation recorded before folding must deduplicate with the same operation
///      retained as HIR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum HirFastUseProvenance {
    /// Authored fast arithmetic: an AST ledger row or a fast HIR numeric operation.
    AuthoredArithmetic,
    /// A generated fast range-loop guard (`LoopStepFailure` under `$fast_math`).
    RangeGuard,
    /// A fast compound-assignment write-back conversion, including the total integer wrap that
    /// carries `Infallible` delivery.
    StoreConversion,
}

/// One sparse selected fast-use witness.
///
/// WHAT: the evidence class and the authored operator/statement span when one exists.
/// WHY: a folded operation has no HIR node, so the span is the only join back to source; generated
///      operations may have no authored span but still prove the function selected fast semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct HirFastUseWitness {
    pub(crate) provenance: HirFastUseProvenance,
    pub(crate) span: Option<SourceSpan>,
}

#[derive(Debug, Clone)]
pub(crate) struct HirDeferredCustomCatchCheck {
    pub(crate) catch_span: Option<SourceSpan>,
    pub(crate) error_type_id: TypeId,
    pub(crate) typed_producer_span: Option<SourceSpan>,
    pub(crate) candidates: Vec<HirBuiltinFailureContributor>,
    pub(crate) eligibility_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HirBuiltinFailureBoundary {
    InferPrivate,
    BuiltinErrorSlot,
    CustomErrorSlot(TypeId),
    ExportedNoSlot,
    /// A declared `-> !` body. It has no error channel, so implicit built-in failure that can
    /// escape it is a closed-boundary diagnostic rather than an inferred private lane.
    Never,
}

#[derive(Debug, Clone)]
pub(crate) struct HirBuiltinFailureContributor {
    pub(crate) source: HirBuiltinFailureSource,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) codes: Vec<BuiltinErrorCode>,
}

#[allow(
    clippy::large_enum_variant,
    reason = "contributors are matched by reference during convergence; boxing would allocate per call contributor"
)]
#[derive(Debug, Clone)]
pub(crate) enum HirBuiltinFailureSource {
    NumericOperation,
    AuthoredCastConversion,
    /// Checked compound-assignment write-back with its canonical target type.
    ///
    /// WHAT: mirrors the AST write-back contributor through projection so the
    ///       witness names the write-back target instead of earlier handled work.
    /// WHY: convergence matches contributors by reference; the target travels
    ///      in the source rather than a parallel lookup.
    CompoundWriteBack {
        target: TypeId,
        arithmetic_span: Option<SourceSpan>,
    },
    Call(CallTarget),
}

impl HirBuiltinFailureContributor {
    pub(crate) fn witness_site(&self) -> FailureWitnessSite {
        match self.source {
            HirBuiltinFailureSource::NumericOperation
            | HirBuiltinFailureSource::AuthoredCastConversion => {
                FailureWitnessSite::Arithmetic(self.span)
            }
            HirBuiltinFailureSource::CompoundWriteBack {
                arithmetic_span, ..
            } => FailureWitnessSite::WriteBack(arithmetic_span),
            HirBuiltinFailureSource::Call(_) => FailureWitnessSite::Call,
        }
    }
}
