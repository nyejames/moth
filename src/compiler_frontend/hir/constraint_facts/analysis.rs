//! Post-lane function constraint analysis shared by phase B convergence and verification.
//!
//! WHAT: one deterministic pass over final HIR computes a function's selected fast semantics,
//!       reachable traps and reachable recoverable escapes, and returns the first local witness
//!       for each bit.
//! WHY: `escapes_builtin_failure` routes private failure lanes and must never double as the
//!      post-lane proof bit. `analyze_function` therefore re-walks the rewritten HIR with the D16
//!      proof policy and liveness-conditioned `Invoke` failure edges. Phase B calls it to widen
//!      summaries; the later verification pass calls the same code for region acceptance.
//!
//! ## `RuntimeFailure` classification
//!
//! Every producer sets `HirTerminator::RuntimeFailure::kind` before the terminator reaches
//! analysis. The kind is the only input to this table, so message text and surrounding block
//! shape never decide trap status:
//!
//! | `RuntimeFailureKind` | Meaning | Counts as `may_trap`? |
//! | --- | --- | --- |
//! | `Conversion` | Unrecovered implicit cast or compound write-back | Yes, `TrapConversion` |
//! | `RangeGuard` | The unreachable continuation of a `LoopStepFailure` guard | No, the guard statement owns the trap |
//! | `NoMatchFallback` | The defensive stop behind an exhaustive match | No, no source match reaches it |
//!
//! `AssertFailure` is always counted because an assertion is a source-authored unrecoverable
//! stop. Environmental exhaustion, termination and implementation faults do not appear as these
//! terminators and are outside the contract.

use rustc_hash::FxHashSet;
use std::collections::VecDeque;

use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalNumericContract};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::constraint_facts::proof::{self, BlockLiteralFacts};
use crate::compiler_frontend::hir::expression_store::HirExpressionStore;
use crate::compiler_frontend::hir::failure_facts::{
    HirFastUseProvenance, HirSelectedCallSite, HirSelectedCallSiteLedger,
};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::{
    HirFallibleOperation, HirTerminator, RuntimeFailureKind,
};
use crate::compiler_frontend::hir::utils::{collect_structural_blocks, for_each_terminator_target};
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::source::{SourceSpan, SpanResolutionSources};

/// The three post-lane summary bits one function owns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ConstraintSummaryBits {
    pub(crate) uses_fast_semantics: bool,
    pub(crate) may_trap: bool,
    pub(crate) may_fail: bool,
}

impl ConstraintSummaryBits {
    pub(crate) fn from_summary(summary: &PublicCallSummary) -> Self {
        Self {
            uses_fast_semantics: summary.uses_fast_semantics,
            may_trap: summary.may_trap,
            may_fail: summary.may_fail,
        }
    }

    /// Every hazard bit set: the evidence for a callee whose absence of hazards cannot be certified.
    pub(crate) const fn unknown() -> Self {
        Self {
            uses_fast_semantics: true,
            may_trap: true,
            may_fail: true,
        }
    }

    /// Monotone union used by phase B; a true bit never returns to false.
    pub(crate) fn union(self, other: Self) -> Self {
        Self {
            uses_fast_semantics: self.uses_fast_semantics || other.uses_fast_semantics,
            may_trap: self.may_trap || other.may_trap,
            may_fail: self.may_fail || other.may_fail,
        }
    }

    /// Write these bits into one existing summary, preserving every other field.
    pub(crate) fn apply_to(self, summary: &mut PublicCallSummary) {
        summary.uses_fast_semantics |= self.uses_fast_semantics;
        summary.may_trap |= self.may_trap;
        summary.may_fail |= self.may_fail;
    }
}

/// Callee evidence returned by the caller's target resolver.
///
/// WHAT: the callee's three bits plus whether the evidence is an untrusted external contract.
/// WHY: an unknown external must be both hazard evidence and a distinct diagnostic leaf, while a
///      local summary's bit is derived evidence. The analyzer never owns target lookup: phase B
///      reads report/leaf maps and the verifier reads the same retained summaries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ConstraintCalleeFacts {
    pub(crate) bits: ConstraintSummaryBits,
    pub(crate) unknown_external: bool,
    /// Whether the callee is an external binding rather than a Moth-source function.
    ///
    /// A plain infallible-shaped external call has no error channel, so its contract's
    /// `may_fail` is meaningful only when the call is invoked fallibly. A local plain call has
    /// no such exclusion: if its summary widened after lowering, the recoverable failure is
    /// still real function-level evidence.
    pub(crate) external_contract: bool,
}

impl ConstraintCalleeFacts {
    pub(crate) fn from_bits(bits: ConstraintSummaryBits) -> Self {
        Self {
            bits,
            unknown_external: false,
            external_contract: false,
        }
    }

    /// A trusted external contract: its flags are used verbatim, with no local exclusion.
    pub(crate) fn trusted_external(bits: ConstraintSummaryBits) -> Self {
        Self {
            bits,
            unknown_external: false,
            external_contract: true,
        }
    }

    /// An untrusted external cannot certify absence, so every open question is evidence.
    pub(crate) fn unknown_external() -> Self {
        Self {
            bits: ConstraintSummaryBits::unknown(),
            unknown_external: true,
            external_contract: true,
        }
    }

    /// Project a registered external contract. A trusted contract is used verbatim, while an
    /// untrusted one is unknown evidence, exactly like a callable with no registration.
    pub(crate) fn from_external_contract(contract: ExternalNumericContract) -> Self {
        match contract {
            ExternalNumericContract::Trusted(trusted) => {
                Self::trusted_external(ConstraintSummaryBits {
                    uses_fast_semantics: trusted.uses_fast_semantics,
                    may_trap: trusted.may_trap,
                    may_fail: trusted.may_fail,
                })
            }
            ExternalNumericContract::Unknown => Self::unknown_external(),
        }
    }
}

/// The evidence class of one local constraint witness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConstraintWitnessKind {
    FastArithmetic,
    FastRangeGuard,
    FastStoreConversion,
    FastCallee,
    FastUnknownExternal,
    TrapArithmetic,
    TrapRangeGuard,
    TrapFastStoreConversion,
    TrapConversion,
    TrapAssertion,
    TrapFloatValidation,
    TrapCallee,
    TrapUnknownExternal,
    EscapingError,
}

/// The compact set of numeric trap causes remaining at one arithmetic witness.
///
/// WHAT: one bit per canonical `BuiltinErrorCode` in the numeric cause range.
/// WHY: verification distinguishes overflow, division, exponent and non-finite wording without
///      retaining an allocation or the whole proof-policy result in the immutable witness record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TrapCauseMask(u32);

impl TrapCauseMask {
    const FIRST_CODE: u16 = 300;
    const LAST_CODE: u16 = 307;

    fn insert(&mut self, code: BuiltinErrorCode) {
        let value = code as u16;
        if (Self::FIRST_CODE..=Self::LAST_CODE).contains(&value) {
            self.0 |= 1 << (value - Self::FIRST_CODE);
        }
    }

    pub(crate) fn contains(self, code: BuiltinErrorCode) -> bool {
        let value = code as u16;
        (Self::FIRST_CODE..=Self::LAST_CODE).contains(&value)
            && self.0 & (1 << (value - Self::FIRST_CODE)) != 0
    }

    fn from_remaining(
        expressions: &HirExpressionStore,
        literals: &BlockLiteralFacts,
        op: HirNumericOp,
        operands: &HirNumericOperands,
    ) -> Self {
        let mut mask = Self::default();
        for cause in proof::remaining_trap_causes(expressions, literals, op, operands) {
            mask.insert(cause);
        }
        mask
    }
}

/// The first local witness for one bit.
///
/// A witness is local to the analyzed function: it names the direct operation or terminator, not
/// a transitive callee origin. Verification follows call summaries separately when the bit is
/// true only through a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConstraintWitness {
    pub(crate) kind: ConstraintWitnessKind,
    pub(crate) span: Option<SourceSpan>,
    /// Numeric causes still possible at an arithmetic trap witness; empty for every other kind.
    pub(crate) trap_causes: TrapCauseMask,
}

impl ConstraintWitness {
    fn new(kind: ConstraintWitnessKind, span: Option<SourceSpan>) -> Self {
        Self {
            kind,
            span,
            trap_causes: TrapCauseMask::default(),
        }
    }
}

/// One function's post-lane constraint facts and first local witnesses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FunctionConstraintAnalysis {
    pub(crate) bits: ConstraintSummaryBits,
    pub(crate) fast_witness: Option<ConstraintWitness>,
    pub(crate) trap_witness: Option<ConstraintWitness>,
    pub(crate) fail_witness: Option<ConstraintWitness>,
}

impl FunctionConstraintAnalysis {
    /// Bits known from the projected fast-use ledger before any call is resolved.
    fn with_local_fast_evidence(hir: &HirModule, function: FunctionId) -> Self {
        let mut analysis = Self::default();
        let Some(facts) = hir.function_failure_facts.get(&function) else {
            return analysis;
        };
        let Some(witness) = facts.constraint.fast_use.first().copied() else {
            return analysis;
        };
        analysis.bits.uses_fast_semantics = true;
        analysis.fast_witness = Some(ConstraintWitness::new(
            fast_use_witness_kind(witness.provenance),
            witness.span,
        ));
        analysis
    }
}

fn fast_use_witness_kind(provenance: HirFastUseProvenance) -> ConstraintWitnessKind {
    match provenance {
        HirFastUseProvenance::AuthoredArithmetic => ConstraintWitnessKind::FastArithmetic,
        HirFastUseProvenance::RangeGuard => ConstraintWitnessKind::FastRangeGuard,
        HirFastUseProvenance::StoreConversion => ConstraintWitnessKind::FastStoreConversion,
    }
}

fn fast_call_witness_kind(unknown_external: bool) -> ConstraintWitnessKind {
    if unknown_external {
        ConstraintWitnessKind::FastUnknownExternal
    } else {
        ConstraintWitnessKind::FastCallee
    }
}

/// Widen fast use from one resolved callee and keep the first local witness.
fn note_fast_callee(
    analysis: &mut FunctionConstraintAnalysis,
    callee: ConstraintCalleeFacts,
    span: Option<SourceSpan>,
) {
    if !callee.bits.uses_fast_semantics {
        return;
    }
    analysis.bits.uses_fast_semantics = true;
    if analysis.fast_witness.is_none() {
        analysis.fast_witness = Some(ConstraintWitness::new(
            if callee.unknown_external {
                ConstraintWitnessKind::FastUnknownExternal
            } else {
                ConstraintWitnessKind::FastCallee
            },
            span,
        ));
    }
}

/// Analyze one final-HIR function against the caller's current callee evidence.
///
/// WHAT: `uses_fast_semantics` from the projected ledger plus every call target (regardless of
///       liveness); `may_trap`/`may_fail` from a liveness-conditioned CFG walk.
/// WHY: phase B calls this repeatedly as callee bits widen, and verification calls it once with
///      the final summaries. It never mutates HIR or reads the backend proof table. Callers pass
///      post-lane HIR, whose block ids `compact_block_ids` has made dense.
pub(crate) fn analyze_function(
    hir: &HirModule,
    function_id: FunctionId,
    resolve_callee: impl Fn(&CallTarget) -> ConstraintCalleeFacts,
) -> FunctionConstraintAnalysis {
    let Some(function) = hir
        .functions
        .iter()
        .find(|function| function.id == function_id)
    else {
        crate::compiler_bug!(
            CompilerStage::HirConstruction,
            "constraint analysis received unknown HIR function {function_id:?}"
        );
    };

    let Some(entry_block) = hir.block(function.entry) else {
        crate::compiler_bug!(
            CompilerStage::HirConstruction,
            "constraint analysis function {function_id:?} has no entry block"
        );
    };

    let mut analysis = FunctionConstraintAnalysis::with_local_fast_evidence(hir, function_id);
    if let Some(facts) = hir.function_failure_facts.get(&function_id) {
        // Calls recorded at projection time include handlers that convergence pruned. Fast use
        // is selected-code evidence, so those calls remain part of the function's summary.
        for selected in &facts.constraint.selected_call_sites {
            note_fast_callee(
                &mut analysis,
                resolve_callee(&selected.target),
                selected.span,
            );
        }
    }
    let structural_blocks = collect_structural_blocks(hir, entry_block.id);

    // The final CFG adds calls lowered after projection (generated/lane materialisation) and
    // keeps the walk authoritative for HIR that did not come from a projected AST function.
    // A direct local witness remains preferred for diagnostics, but every call site must widen
    // the bit.
    for block_id in &structural_blocks {
        let Some(block) = hir.block(*block_id) else {
            continue;
        };
        visit_call_targets(hir, block, |target, span| {
            note_fast_callee(&mut analysis, resolve_callee(target), span);
        });
    }

    walk_live_blocks(
        hir,
        entry_block.id,
        &mut analysis,
        &resolve_callee,
        &mut WalkScope::WholeFunction,
    );
    // Release folding removed these calls from the CFG. Their live rows are the exact executable
    // hazard evidence the debug build's call statement would have provided.
    if let Some(facts) = hir.function_failure_facts.get(&function_id) {
        for selected in &facts.constraint.live_erased_call_sites {
            // A release candidate always carries an infallible-shaped contract, so an inherited
            // `may_fail` is implicit failure propagation. HIR models that as a generated failure
            // edge with no authored span, and a debug build therefore reports the escaped error
            // without an evidence span. Only the trap hazard keeps the authored call span.
            note_live_call_hazards(
                &mut analysis,
                resolve_callee(&selected.target),
                selected.span,
                None,
                selected.failure_caught,
            );
        }
    }

    analysis
}

/// One statement boundary's local constraint evidence.
///
/// WHAT: the first fast-use witness, reachable trap witness and live escaping-failure witness
///       whose authored span lies inside the annotated statement, plus whether any live HIR site
///       belongs to that region.
/// WHY: a statement boundary must be verified from the same proof and liveness rules as the
///      function walk, but fast evidence is liveness-independent and includes pruned handlers.
///      The boolean distinguishes a dead boundary (skip it) from a live boundary with no
///      offending evidence (accept it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RegionConstraintAnalysis {
    pub(crate) fast_witness: Option<ConstraintWitness>,
    pub(crate) trap_witness: Option<ConstraintWitness>,
    pub(crate) fail_witness: Option<ConstraintWitness>,
    pub(crate) has_live_region_site: bool,
}

/// Verify one authored statement region inside a final HIR function.
///
/// WHAT: selects statements, terminators and Invoke sites whose authored span falls inside
///       `start..=end` in the same source, scans selected fast evidence without regard to
///       liveness, then walks only live CFG edges to collect in-region traps and escaping
///       failures.
/// WHY: statement constraints are lexical, so evidence outside the annotated statement must not
///      reject them; a failure produced in the region is contained only when its live failure
///      successor is also in the region, while a fast use in a selected dead handler still
///      counts because fast selection is not a liveness property.
pub(crate) fn analyze_statement_region(
    hir: &HirModule,
    function_id: FunctionId,
    start: SourceSpan,
    end: SourceSpan,
    sources: &SpanResolutionSources<'_>,
    resolve_callee: &impl Fn(&CallTarget) -> ConstraintCalleeFacts,
) -> RegionConstraintAnalysis {
    let Some(function) = hir
        .functions
        .iter()
        .find(|function| function.id == function_id)
    else {
        return RegionConstraintAnalysis::default();
    };
    let Some(entry) = hir.block(function.entry) else {
        return RegionConstraintAnalysis::default();
    };

    let structural = collect_structural_blocks(hir, entry.id);
    let fast_witness = select_region_fast_witness(
        hir,
        function_id,
        &structural,
        start,
        end,
        sources,
        resolve_callee,
    );

    // The live walk owns trap, escaping-failure and termination classification for both
    // boundaries. The scope contributes only site attribution and failure containment.
    let mut scope = WalkScope::Region {
        sources,
        start,
        end,
        saw_site: false,
    };
    let mut analysis = FunctionConstraintAnalysis::default();
    walk_live_blocks(hir, entry.id, &mut analysis, resolve_callee, &mut scope);
    // A folded call that stood in this region had the same executable hazards as the runtime call
    // the debug build retains, so it attributes to the region by its authored span.
    if let Some(facts) = hir.function_failure_facts.get(&function_id) {
        for selected in &facts.constraint.live_erased_call_sites {
            let Some(span) = selected.span else {
                continue;
            };
            if !sources.span_within(span, start, end) {
                continue;
            }
            note_live_call_hazards(
                &mut analysis,
                resolve_callee(&selected.target),
                Some(span),
                Some(span),
                selected.failure_caught,
            );
        }
    }

    RegionConstraintAnalysis {
        fast_witness,
        trap_witness: analysis.trap_witness,
        fail_witness: analysis.fail_witness,
        has_live_region_site: scope.saw_site(),
    }
}

fn attributed_site_span(
    authored_span: Option<SourceSpan>,
    fallback_span: Option<SourceSpan>,
    sources: &SpanResolutionSources<'_>,
    start: SourceSpan,
    end: SourceSpan,
) -> Option<SourceSpan> {
    match authored_span {
        Some(span) => sources.span_within(span, start, end).then_some(span),
        None => fallback_span.filter(|span| sources.span_within(*span, start, end)),
    }
}

/// The authored span an Invoke owns, falling back to its block for generated calls.
fn attributed_terminator_span(
    hir: &HirModule,
    block: &HirBlock,
    sources: &SpanResolutionSources<'_>,
    start: SourceSpan,
    end: SourceSpan,
) -> Option<SourceSpan> {
    let authored_span = match &block.terminator {
        HirTerminator::Invoke { span, .. } => *span,
        _ => None,
    };
    attributed_site_span(
        authored_span,
        hir.side_table.block_span(block.id),
        sources,
        start,
        end,
    )
}

fn block_has_region_site(
    hir: &HirModule,
    block: &HirBlock,
    sources: &SpanResolutionSources<'_>,
    start: SourceSpan,
    end: SourceSpan,
) -> bool {
    let fallback_span = hir.side_table.block_span(block.id);
    block.statements.iter().any(|statement| {
        attributed_site_span(statement.span, fallback_span, sources, start, end).is_some()
    }) || attributed_terminator_span(hir, block, sources, start, end).is_some()
}

/// Select the first fast-use witness inside one statement region.
///
/// WHAT: the selected-code ledger first, then retained HIR for operations and calls lowered after
///       projection.
/// WHY: fast selection is not a liveness property, so a folded operation or a call in a pruned
///       handler still rejects `$safe_math`; the retained-HIR pass attributes generated sites
///       through their block span without scanning the whole module per query.
fn select_region_fast_witness(
    hir: &HirModule,
    function_id: FunctionId,
    structural: &[BlockId],
    start: SourceSpan,
    end: SourceSpan,
    sources: &SpanResolutionSources<'_>,
    resolve_callee: &impl Fn(&CallTarget) -> ConstraintCalleeFacts,
) -> Option<ConstraintWitness> {
    ledger_fast_use_witness(hir, function_id, start, end, sources)
        .or_else(|| retained_fast_operation_witness(hir, structural, start, end, sources))
        .or_else(|| ledger_fast_call_witness(hir, function_id, start, end, sources, resolve_callee))
        .or_else(|| {
            retained_fast_call_witness(hir, structural, start, end, sources, resolve_callee)
        })
}

fn ledger_fast_use_witness(
    hir: &HirModule,
    function_id: FunctionId,
    start: SourceSpan,
    end: SourceSpan,
    sources: &SpanResolutionSources<'_>,
) -> Option<ConstraintWitness> {
    let facts = hir.function_failure_facts.get(&function_id)?;
    for witness in &facts.constraint.fast_use {
        let Some(span) = witness.span else {
            continue;
        };
        if sources.span_within(span, start, end) {
            return Some(ConstraintWitness::new(
                fast_use_witness_kind(witness.provenance),
                Some(span),
            ));
        }
    }
    None
}

fn retained_fast_operation_witness(
    hir: &HirModule,
    structural: &[BlockId],
    start: SourceSpan,
    end: SourceSpan,
    sources: &SpanResolutionSources<'_>,
) -> Option<ConstraintWitness> {
    for block_id in structural {
        let Some(block) = hir.block(*block_id) else {
            continue;
        };
        let fallback_span = hir.side_table.block_span(block.id);
        for statement in &block.statements {
            let Some(span) =
                attributed_site_span(statement.span, fallback_span, sources, start, end)
            else {
                continue;
            };
            let provenance = match &statement.kind {
                HirStatementKind::NumericOp { op, .. } if op.semantics.is_fast() => {
                    HirFastUseProvenance::AuthoredArithmetic
                }
                HirStatementKind::LoopStepFailure { semantics, .. } if semantics.is_fast() => {
                    HirFastUseProvenance::RangeGuard
                }
                HirStatementKind::FastStoreConversion { .. } => {
                    HirFastUseProvenance::StoreConversion
                }
                _ => continue,
            };
            return Some(ConstraintWitness::new(
                fast_use_witness_kind(provenance),
                Some(span),
            ));
        }
        if let HirTerminator::Invoke {
            operation: HirFallibleOperation::NumericOp { op, .. },
            ..
        } = &block.terminator
            && op.semantics.is_fast()
            && let Some(span) = attributed_terminator_span(hir, block, sources, start, end)
        {
            return Some(ConstraintWitness::new(
                ConstraintWitnessKind::FastArithmetic,
                Some(span),
            ));
        }
    }
    None
}

fn ledger_fast_call_witness(
    hir: &HirModule,
    function_id: FunctionId,
    start: SourceSpan,
    end: SourceSpan,
    sources: &SpanResolutionSources<'_>,
    resolve_callee: &impl Fn(&CallTarget) -> ConstraintCalleeFacts,
) -> Option<ConstraintWitness> {
    let facts = hir.function_failure_facts.get(&function_id)?;
    for selected in &facts.constraint.selected_call_sites {
        let Some(span) = selected.span else {
            continue;
        };
        if !sources.span_within(span, start, end) {
            continue;
        }
        let callee = resolve_callee(&selected.target);
        if callee.bits.uses_fast_semantics {
            return Some(ConstraintWitness::new(
                fast_call_witness_kind(callee.unknown_external),
                Some(span),
            ));
        }
    }
    None
}

fn retained_fast_call_witness(
    hir: &HirModule,
    structural: &[BlockId],
    start: SourceSpan,
    end: SourceSpan,
    sources: &SpanResolutionSources<'_>,
    resolve_callee: &impl Fn(&CallTarget) -> ConstraintCalleeFacts,
) -> Option<ConstraintWitness> {
    for block_id in structural {
        let Some(block) = hir.block(*block_id) else {
            continue;
        };
        let fallback_span = hir.side_table.block_span(block.id);
        for statement in &block.statements {
            let HirStatementKind::Call { target, .. } = &statement.kind else {
                continue;
            };
            let Some(span) =
                attributed_site_span(statement.span, fallback_span, sources, start, end)
            else {
                continue;
            };
            let callee = resolve_callee(target);
            if callee.bits.uses_fast_semantics {
                return Some(ConstraintWitness::new(
                    fast_call_witness_kind(callee.unknown_external),
                    Some(span),
                ));
            }
        }
        let target = match &block.terminator {
            HirTerminator::Invoke {
                operation: HirFallibleOperation::Call { target, .. },
                ..
            }
            | HirTerminator::NeverCall { target, .. } => Some(target),
            _ => None,
        };
        if let Some(target) = target
            && let Some(span) = attributed_terminator_span(hir, block, sources, start, end)
        {
            let callee = resolve_callee(target);
            if callee.bits.uses_fast_semantics {
                return Some(ConstraintWitness::new(
                    fast_call_witness_kind(callee.unknown_external),
                    Some(span),
                ));
            }
        }
    }
    None
}

/// Whether a live failure edge stays lexically inside the annotated statement.
///
/// WHAT: follows only spanless generated jumps to their first authored destination.
/// WHY: private failure lanes and range-loop lowering create adapters and continuation blocks
///      without authored spans. Treating a spanless adapter that jumps to an in-region handler
///      as outside the region would report a failure the statement already caught. A spanless
///      block with any other terminator is a genuine escape or trap site. The visited set bounds
///      the walk by block count.
pub(crate) fn failure_destination_in_region(
    hir: &HirModule,
    failure: BlockId,
    sources: &SpanResolutionSources<'_>,
    start: SourceSpan,
    end: SourceSpan,
) -> bool {
    let mut visited = FxHashSet::default();
    let mut current = failure;
    loop {
        if !visited.insert(current) {
            return false;
        }
        let Some(block) = hir.block(current) else {
            return false;
        };
        if block_has_region_site(hir, block, sources, start, end) {
            return true;
        }
        // A block with an authored location outside the region is an escaping destination.
        if hir.side_table.block_span(block.id).is_some()
            || block
                .statements
                .iter()
                .any(|statement| statement.span.is_some())
        {
            return false;
        }
        match &block.terminator {
            HirTerminator::Jump { target, .. } => current = *target,
            _ => return false,
        }
    }
}

/// The authored-site window one live-CFG walk reports evidence for.
///
/// WHAT: the whole function or one annotated statement region.
/// WHY: function summaries and statement boundaries must apply identical proof, liveness and
///      termination-classification rules; only site attribution and failure containment differ.
enum WalkScope<'sources, 'database> {
    WholeFunction,
    Region {
        sources: &'sources SpanResolutionSources<'database>,
        start: SourceSpan,
        end: SourceSpan,
        saw_site: bool,
    },
}

/// What one live-walk site contributes.
#[derive(Clone, Copy)]
enum SiteAttribution {
    /// The walk owns the site and reports this witness span.
    Owned(Option<SourceSpan>),
    /// The site lies outside the walked scope.
    Skipped,
}

impl<'sources, 'database> WalkScope<'sources, 'database> {
    fn statement_attribution(
        &mut self,
        statement: &HirStatement,
        fallback_span: Option<SourceSpan>,
    ) -> SiteAttribution {
        self.attribute(statement.span, fallback_span)
    }

    fn terminator_attribution(&mut self, hir: &HirModule, block: &HirBlock) -> SiteAttribution {
        let authored_span = match &block.terminator {
            HirTerminator::Invoke { span, .. } => *span,
            _ => hir.side_table.terminator_span(block.id).copied(),
        };
        self.attribute(authored_span, hir.side_table.block_span(block.id))
    }

    fn attribute(
        &mut self,
        authored_span: Option<SourceSpan>,
        fallback_span: Option<SourceSpan>,
    ) -> SiteAttribution {
        match self {
            Self::WholeFunction => SiteAttribution::Owned(authored_span),
            Self::Region {
                sources,
                start,
                end,
                saw_site,
            } => {
                let Some(span) =
                    attributed_site_span(authored_span, fallback_span, sources, *start, *end)
                else {
                    return SiteAttribution::Skipped;
                };
                *saw_site = true;
                SiteAttribution::Owned(Some(span))
            }
        }
    }

    /// Whole-function boundaries accept a live failure edge by construction; a statement region
    /// contains it only when the destination stays inside the authored range.
    fn failure_stays_in_scope(&self, hir: &HirModule, failure: BlockId) -> bool {
        match self {
            Self::WholeFunction => true,
            Self::Region {
                sources,
                start,
                end,
                ..
            } => failure_destination_in_region(hir, failure, sources, *start, *end),
        }
    }

    fn saw_site(&self) -> bool {
        matches!(self, Self::Region { saw_site, .. } if *saw_site)
    }
}

/// Every direct call site in one function, including pruned catch-handler sites.
///
/// WHAT: deterministic, duplicate-free union of final-CFG call sites and sites projected into
///       constraint evidence before convergence pruned handler subgraphs.
/// WHY: phase B builds reverse edges from the targets and verification matches a witness to its
///      site by authored span. One traversal keeps those consumers on the same selected set.
pub(crate) fn collect_function_call_sites(
    hir: &HirModule,
    function_id: FunctionId,
) -> Vec<HirSelectedCallSite> {
    let Some(function) = hir
        .functions
        .iter()
        .find(|function| function.id == function_id)
    else {
        return Vec::new();
    };
    let Some(entry) = hir.block(function.entry) else {
        return Vec::new();
    };

    let mut sites = HirSelectedCallSiteLedger::default();
    for block_id in collect_structural_blocks(hir, entry.id) {
        let Some(block) = hir.block(block_id) else {
            continue;
        };
        visit_call_targets(hir, block, |target, span| sites.push(target, span));
    }
    if let Some(facts) = hir.function_failure_facts.get(&function_id) {
        for selected in &facts.constraint.selected_call_sites {
            sites.push(&selected.target, selected.span);
        }
    }
    sites.into_sites()
}

/// Every call target in one function, including targets inside pruned catch handlers.
///
/// WHAT: the target-only view of [`collect_function_call_sites`].
/// WHY: phase B builds reverse edges over all calls, not only live ones: a callee `may_fail`
///      widening can make a previously dead handler reachable and add facts there. A pruned
///      handler contributes no final-CFG call, so its recorded target supplies the edge.
pub(crate) fn function_call_targets(hir: &HirModule, function_id: FunctionId) -> Vec<CallTarget> {
    let mut targets = Vec::new();
    let mut seen = FxHashSet::default();
    for site in collect_function_call_sites(hir, function_id) {
        if seen.insert(site.target.clone()) {
            targets.push(site.target);
        }
    }
    targets
}

fn visit_call_targets(
    hir: &HirModule,
    block: &HirBlock,
    mut visit: impl FnMut(&CallTarget, Option<SourceSpan>),
) {
    for statement in &block.statements {
        if let HirStatementKind::Call { target, .. } = &statement.kind {
            visit(target, statement.span);
        }
    }
    match &block.terminator {
        HirTerminator::Invoke {
            span,
            operation: HirFallibleOperation::Call { target, .. },
            ..
        } => visit(target, *span),
        HirTerminator::NeverCall { target, .. } => {
            visit(target, hir.side_table.terminator_span(block.id).copied());
        }
        _ => {}
    }
}

/// Walk the liveness-conditioned CFG from the function entry or inside one statement region.
///
/// WHAT: scans each reachable block once for trap sites, escaping `ReturnError` terminators and
///       live successors. The success edge of an `Invoke` is always followed; its failure edge is
///       followed only when the operation can actually fail under the proof policy and callee bits.
///       `scope` decides which sites contribute witnesses and whether a failure edge leaves the
///       boundary.
/// WHY: a dead catch handler must not contribute traps or escapes, while a callee widening can
///       make its edge live again during phase B. Whole-function and statement verification share
///       this body so proof, delivery and termination classification cannot drift between them.
fn walk_live_blocks(
    hir: &HirModule,
    entry: BlockId,
    analysis: &mut FunctionConstraintAnalysis,
    resolve_callee: &impl Fn(&CallTarget) -> ConstraintCalleeFacts,
    scope: &mut WalkScope<'_, '_>,
) {
    let mut visited = FxHashSet::default();
    let mut queue = VecDeque::from([entry]);
    let mut literals = BlockLiteralFacts::new();
    while let Some(block_id) = queue.pop_front() {
        if !visited.insert(block_id) {
            continue;
        }
        let Some(block) = hir.block(block_id) else {
            continue;
        };

        literals.reset();
        let fallback_span = hir.side_table.block_span(block.id);
        for statement in &block.statements {
            if let SiteAttribution::Owned(witness_span) =
                scope.statement_attribution(statement, fallback_span)
            {
                scan_statement_for_traps(
                    hir,
                    statement,
                    witness_span,
                    &literals,
                    resolve_callee,
                    analysis,
                );
            }
            literals.observe_statement(&hir.expressions, statement);
        }

        let terminator_site = scope.terminator_attribution(hir, block);
        match &block.terminator {
            HirTerminator::Invoke {
                operation,
                success,
                failure,
                ..
            } => {
                // A callee's own trap is a property of executing the call, independent of the
                // recoverable failure edge that delivers its error.
                if let SiteAttribution::Owned(span) = terminator_site
                    && let HirFallibleOperation::Call { target, .. } = operation
                {
                    let callee = resolve_callee(target);
                    if callee.bits.may_trap {
                        record_trap_witness(
                            analysis,
                            call_trap_witness_kind(callee.unknown_external),
                            span,
                        );
                    }
                }
                let failure_is_live =
                    invoke_failure_is_live(hir, operation, &literals, resolve_callee);
                queue.push_back(*success);
                if failure_is_live {
                    if let SiteAttribution::Owned(Some(span)) = terminator_site
                        && !scope.failure_stays_in_scope(hir, *failure)
                        && analysis.fail_witness.is_none()
                    {
                        analysis.fail_witness = Some(ConstraintWitness::new(
                            ConstraintWitnessKind::EscapingError,
                            Some(span),
                        ));
                    }
                    queue.push_back(*failure);
                }
            }
            HirTerminator::NeverCall { target, .. } => {
                if let SiteAttribution::Owned(span) = terminator_site {
                    let callee = resolve_callee(target);
                    if callee.bits.may_trap {
                        record_trap_witness(
                            analysis,
                            call_trap_witness_kind(callee.unknown_external),
                            span,
                        );
                    }
                }
            }
            HirTerminator::RuntimeFailure { kind, .. } => {
                if let SiteAttribution::Owned(span) = terminator_site
                    && *kind == RuntimeFailureKind::Conversion
                {
                    record_trap_witness(analysis, ConstraintWitnessKind::TrapConversion, span);
                }
            }
            HirTerminator::AssertFailure { .. } => {
                if let SiteAttribution::Owned(span) = terminator_site {
                    record_trap_witness(analysis, ConstraintWitnessKind::TrapAssertion, span);
                }
            }
            HirTerminator::ReturnError(_) => {
                if let SiteAttribution::Owned(span) = terminator_site {
                    analysis.bits.may_fail = true;
                    if analysis.fail_witness.is_none() {
                        analysis.fail_witness = Some(ConstraintWitness::new(
                            ConstraintWitnessKind::EscapingError,
                            span,
                        ));
                    }
                }
            }
            _ => {}
        }

        if !matches!(block.terminator, HirTerminator::Invoke { .. }) {
            for_each_terminator_target(&block.terminator, |target| queue.push_back(target));
        }
    }
}

/// Scan one block statement for trap and escaping-failure evidence.
///
/// WHAT: classifies numeric proof, generated guards and conversions, float validation, and direct
///       call hazards. It records the first witness at `witness_span` so a region can attribute a
///       generated site through its block.
fn scan_statement_for_traps(
    hir: &HirModule,
    statement: &HirStatement,
    witness_span: Option<SourceSpan>,
    literals: &BlockLiteralFacts,
    resolve_callee: &impl Fn(&CallTarget) -> ConstraintCalleeFacts,
    analysis: &mut FunctionConstraintAnalysis,
) {
    let expressions = &hir.expressions;
    match &statement.kind {
        HirStatementKind::NumericOp {
            op,
            failure_mode,
            operands,
            ..
        } => {
            if !proof::failure_mode_is_total(*failure_mode)
                && proof::operation_has_remaining_trap_causes(expressions, literals, *op, operands)
            {
                let trap_causes = if analysis.trap_witness.is_none() {
                    TrapCauseMask::from_remaining(expressions, literals, *op, operands)
                } else {
                    TrapCauseMask::default()
                };
                record_trap_witness_with_causes(
                    analysis,
                    ConstraintWitnessKind::TrapArithmetic,
                    witness_span,
                    trap_causes,
                );
            }
        }
        HirStatementKind::LoopStepFailure { .. } => {
            // A statement loop-step guard always fails: ordinary delivery traps and fast delivery
            // is fatal, and a recoverable guard is an `Invoke` that this walk follows instead.
            record_trap_witness(
                analysis,
                ConstraintWitnessKind::TrapRangeGuard,
                witness_span,
            );
        }
        HirStatementKind::FastStoreConversion { conversion, .. } => {
            if conversion.required_failure_mode() != Some(NumericFailureMode::Infallible) {
                record_trap_witness(
                    analysis,
                    ConstraintWitnessKind::TrapFastStoreConversion,
                    witness_span,
                );
            }
        }
        HirStatementKind::FormatFloat { failure_mode, .. }
        | HirStatementKind::ValidateFloat { failure_mode, .. } => {
            if !proof::failure_mode_is_total(*failure_mode) {
                record_trap_witness(
                    analysis,
                    ConstraintWitnessKind::TrapFloatValidation,
                    witness_span,
                );
            }
        }
        HirStatementKind::Call { target, .. } => {
            note_live_call_hazards(
                analysis,
                resolve_callee(target),
                witness_span,
                witness_span,
                false,
            );
        }
        _ => {}
    }
}

/// Record the liveness-conditioned hazards of one direct call.
///
/// WHAT: a trap witness for a callee that may trap and an escaping-failure witness for a plain
///       Moth-source callee whose summary widened to `may_fail`.
/// WHY: retained HIR call statements and release-folded calls both present an executable callee
///      hazard. One helper keeps their analysis identical while each owner supplies its own
///      authored span and liveness proof.
fn note_live_call_hazards(
    analysis: &mut FunctionConstraintAnalysis,
    callee: ConstraintCalleeFacts,
    trap_span: Option<SourceSpan>,
    fail_span: Option<SourceSpan>,
    failure_caught: bool,
) {
    if callee.bits.may_trap {
        record_trap_witness(
            analysis,
            call_trap_witness_kind(callee.unknown_external),
            trap_span,
        );
    }
    // A plain external call is infallible-shaped, so its contract cannot contribute an error
    // channel. A plain Moth-source call whose callee widened to `may_fail` has a real escape
    // path, even though lowering saw a then-infallible contract.
    if callee.bits.may_fail && !callee.external_contract && !failure_caught {
        analysis.bits.may_fail = true;
        if analysis.fail_witness.is_none() {
            analysis.fail_witness = Some(ConstraintWitness::new(
                ConstraintWitnessKind::EscapingError,
                fail_span,
            ));
        }
    }
}

fn call_trap_witness_kind(unknown_external: bool) -> ConstraintWitnessKind {
    if unknown_external {
        ConstraintWitnessKind::TrapUnknownExternal
    } else {
        ConstraintWitnessKind::TrapCallee
    }
}

fn invoke_failure_is_live(
    hir: &HirModule,
    operation: &HirFallibleOperation,
    literals: &BlockLiteralFacts,
    resolve_callee: &impl Fn(&CallTarget) -> ConstraintCalleeFacts,
) -> bool {
    match operation {
        HirFallibleOperation::Call { target, .. } => resolve_callee(target).bits.may_fail,
        HirFallibleOperation::NumericOp { op, operands } => {
            proof::operation_has_remaining_trap_causes(&hir.expressions, literals, *op, operands)
        }
        HirFallibleOperation::CastOp { .. }
        | HirFallibleOperation::MapOp { .. }
        | HirFallibleOperation::LoopStepFailure { .. }
        | HirFallibleOperation::FormatFloat { .. }
        | HirFallibleOperation::ValidateFloat { .. } => true,
    }
}

fn record_trap_witness(
    analysis: &mut FunctionConstraintAnalysis,
    kind: ConstraintWitnessKind,
    span: Option<SourceSpan>,
) {
    record_trap_witness_with_causes(analysis, kind, span, TrapCauseMask::default());
}

fn record_trap_witness_with_causes(
    analysis: &mut FunctionConstraintAnalysis,
    kind: ConstraintWitnessKind,
    span: Option<SourceSpan>,
    trap_causes: TrapCauseMask,
) {
    analysis.bits.may_trap = true;
    if analysis.trap_witness.is_none() {
        let mut witness = ConstraintWitness::new(kind, span);
        witness.trap_causes = trap_causes;
        analysis.trap_witness = Some(witness);
    }
}
