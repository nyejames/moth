# Boracle call-summary and deferred exclusive-access research plan

Status: proposed

Current slice: not started

Blockers: typed provenance relations, conflict-directed refinement and the bounded operational oracle must be complete

Next action: activate with an inventory of current local, generated, cross-module and external call-summary producers and every HIR multiple-result representation

Repository path:

```text
docs/roadmap/plans/boracle-call-summary-and-deferred-exclusive-access-plan.md
```

Canonical authorities:

- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/src/developer-docs/memory-management/overview.mtf`
- `docs/src/developer-docs/memory-management/borrow-validation/borrow-validation.mtf`
- `docs/src/developer-docs/memory-management/boracle/boracle-reference-solver.mtf`
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`

## Purpose

Investigate a richer borrow-level call boundary for Boracle.

Moth calls are statically resolved. Generated generics are concrete before borrow validation. There are no trait objects, general function values, closures or unsafe pointers. Boracle should exploit those restrictions rather than reducing every call to one parameter-access list and one unioned return-alias fact.

This package explores:

- per-result provenance
- projection results
- result-to-result relationships
- success and error outcome effects
- recursive summary fixed points
- deferred activation of exclusive call arguments

The work remains borrow-level. Complete retention, detached stored results, outlives constraints and public lifetime summaries belong to lifetime analysis.

## Prerequisites

The active repository must already provide:

- typed origin relationships and uncertainty
- composable experiments
- conflict-directed path refinement
- bounded operational checking
- exact call-argument and call-effect events
- compiler-owned local and generated semantic convergence
- stable call targets and provider summaries
- normal build-system isolation from semantic stage sequencing

## Locked decisions

1. Call summaries are compiler-owned semantic facts.
2. The build system may schedule modules and provide immutable published summaries. It must not solve call summaries or rerun borrow analysis.
3. Borrow-level result provenance remains preliminary. Lifetime analysis owns complete exported result and retention summaries.
4. Donor-local HIR, origin, region and allocation-family IDs never cross module interfaces.
5. The first richer summary is Boracle-owned and experimental. It does not immediately change `PublicSemanticInterface`.
6. Multiple results are represented per result slot. One function-wide alias union is not accepted as the final model.
7. Outcome-sensitive effects use explicit HIR exits. They do not invent exception semantics.
8. Deferred exclusive access activates at the call-effect boundary, not at an inferred first mutation inside the callee.
9. Reservation is allowed only when argument evaluation order and call metadata are known.
10. Unknown or external boundaries remain conservative unless their closed metadata explicitly supports the rule.
11. Optional transfer remains a separate later decision.
12. Recursive solving uses a small monotone finite lattice and deterministic SCC convergence.
13. No arbitrary Boolean or arithmetic predicate solver is added.
14. This plan is deleted with its roadmap entry in the completion commit.

## Target summary vocabulary

Exact names may change after the HIR inventory.

```rust
pub(crate) struct BorrowCallSummary {
    pub(crate) parameters: Box<[ParameterBorrowEffect]>,
    pub(crate) results: Box<[ResultBorrowProvenance]>,
    pub(crate) outcomes: Box<[OutcomeBorrowEffect]>,
}

pub(crate) struct ParameterBorrowEffect {
    pub(crate) access: AccessKind,
    pub(crate) may_mutate: bool,
    pub(crate) activation: AccessActivation,
}

pub(crate) enum AccessActivation {
    Immediate,
    AtCallEffect,
}

pub(crate) enum ResultSource {
    Fresh,
    Parameter {
        index: u32,
        projection: Option<ProjectionPath>,
    },
    OtherResult {
        index: u32,
        projection: Option<ProjectionPath>,
    },
    Unknown(UnknownSummaryReason),
}
```

Outcome keys should use semantic exits already present in HIR:

```rust
pub(crate) enum BorrowOutcome {
    Return,
    Success,
    Error,
    RuntimeFailure,
}
```

Do not add option or choice predicates until current HIR proves they need separate summary outcomes.

## In scope

- current summary-owner inventory
- per-result preliminary provenance
- projection and result-to-result relations
- outcome-sensitive borrow effects
- local and generated recursive SCC reference solving
- deferred exclusive call activation
- same-call overlap rules
- source and normalized fixtures
- operational-oracle comparison
- future production-summary recommendations

## Out of scope

- complete lifetime summaries
- retention cardinality
- detached stored-result lifetime ownership
- public ABI migration before review
- dynamic dispatch
- arbitrary value guards
- callee-body inlining into callers
- source annotations
- mandatory move parameters
- backend lowering
- build-system summary solving

## Phase 0: inventory current call and result representation

### Summary and reasoning

Do not design a new result table until the current HIR and summary lanes are mapped end to end.

### Work

- [ ] Re-anchor the branch and record the working baseline in untracked notes.
- [ ] Inventory call-summary producers for:
  - [ ] local source functions
  - [ ] generated functions
  - [ ] cross-module public calls
  - [ ] module-private calls
  - [ ] binding-backed external calls
  - [ ] WIT or closed host boundaries
- [ ] Inventory call-summary consumers in:
  - [ ] current Alpha checker
  - [ ] `BorrowProblem` builder
  - [ ] Boracle origin solver
  - [ ] Boracle loan solver
  - [ ] public-interface projection
  - [ ] generated summary convergence
- [ ] Map how HIR represents:
  - [ ] one result
  - [ ] multiple returns
  - [ ] multi-bind
  - [ ] fallible success and error values
  - [ ] compiler temporaries
- [ ] Identify every function-wide alias union.
- [ ] Classify current unknown summaries by reason.
- [ ] Add reduced fixtures for current over-approximation.
- [ ] Confirm compiler-owned convergence stays inside module compilation.
- [ ] Record the minimum Boracle-only vocabulary required before any public interface change.

### Phase gate

- [ ] Audit the map against compiler and build-system ownership docs.
- [ ] Review every proposed ID for stable versus donor-local scope.
- [ ] Run `just boracle`.
- [ ] Commit the inventory and regression corpus.

## Phase 1: add per-result borrow provenance

### Summary and reasoning

Separate result slots before adding recursion or outcome sensitivity.

### Work

- [ ] Add a Boracle-owned result-slot identity.
- [ ] Evolve normalized call effects to expose every semantic result slot needed by borrow analysis.
- [ ] Preserve exact result-write event order.
- [ ] Add result provenance for:
  - [ ] fresh
  - [ ] alias parameter
  - [ ] alias parameter projection
  - [ ] alias another result
  - [ ] unknown
- [ ] Add positive result-to-result disjointness where supplied.
- [ ] Keep complete retained relationships out of this vocabulary.
- [ ] Update origin propagation, provenance loans and reports.
- [ ] Ensure one result's alias source does not contaminate another result.
- [ ] Add validation for invalid result cycles and missing result slots.
- [ ] Keep public interfaces unchanged in this phase.

### Required tests

- [ ] result 0 fresh, result 1 aliases parameter 0
- [ ] two independent fresh results
- [ ] result 1 aliases result 0
- [ ] result aliases a fixed field projection
- [ ] unknown one result does not make every result unknown
- [ ] result-write ordering remains exact
- [ ] malformed result relationships fail as `CompilerError`

### Phase gate

- [ ] Audit borrow-level versus lifetime-level result facts.
- [ ] Review normalized event changes and test ownership.
- [ ] Run focused call-result tests.
- [ ] Run `just boracle`.
- [ ] Run `just validate`.
- [ ] Commit per-result provenance.

## Phase 2: add outcome-sensitive borrow effects

### Summary and reasoning

Fallible control flow already has separate success and error edges. Borrow summaries should not union effects that cannot occur on the same outcome.

### Work

- [ ] Add outcome-keyed preliminary borrow effects.
- [ ] Support current HIR exits:
  - [ ] ordinary return
  - [ ] success
  - [ ] error
  - [ ] unrecoverable failure where relevant
- [ ] Attach result provenance and parameter mutation to the outcome that performs them.
- [ ] Instantiate outcome effects on the caller's exact continuation edge.
- [ ] Ensure a success-only alias does not keep a loan live on the error edge.
- [ ] Ensure an error-only mutation does not contaminate the success edge.
- [ ] Add edge-sensitive last-use observations for call outcomes.
- [ ] Keep lifetime retention effects separate.
- [ ] Reject incomplete outcome tables as internal errors when the callee contract promised them.
- [ ] Keep unknown outcomes conservative.

### Required tests

- [ ] success aliases parameter, error returns fresh
- [ ] success mutates, error does not
- [ ] error mutates, success does not
- [ ] catch path drops success-only alias
- [ ] early return from error path
- [ ] outcome-specific final-use candidate
- [ ] unknown outcome preserves conservative result

### Phase gate

- [ ] Audit every outcome against HIR CFG edges.
- [ ] Review that no effect is inferred from source text.
- [ ] Compare bounded call traces with the oracle.
- [ ] Run `just boracle`.
- [ ] Commit outcome-sensitive effects.

## Phase 3: solve recursive call-summary SCCs

### Summary and reasoning

Static calls make recursive summary convergence practical. Use a small finite lattice before exposing any new cross-module contract.

### Work

- [ ] Build a compiler-owned call graph for the functions in the Boracle source solve.
- [ ] Include generated functions only through the existing compiler-owned generated convergence boundary.
- [ ] Decompose the graph into deterministic SCCs.
- [ ] Define a monotone lattice:
  - [ ] possible parameter accesses grow
  - [ ] possible mutations grow
  - [ ] possible result alias sources grow
  - [ ] possible outcomes grow
  - [ ] unknown reason is retained
- [ ] Start recursive SCCs from the least sound summary that cannot under-approximate effects.
- [ ] Iterate to a fixed point.
- [ ] Retain derivation and widening reasons.
- [ ] Do not expose donor-local IDs across module boundaries.
- [ ] Do not ask Stage 0 to reschedule a module.
- [ ] Add mutual recursion and generated recursion cases.
- [ ] Keep this as a Boracle reference experiment until architecture review accepts the summary surface.

### Phase gate

- [ ] Audit monotonicity and termination.
- [ ] Audit compiler/build ownership.
- [ ] Review unknown fallback behaviour.
- [ ] Run recursive summary tests.
- [ ] Run `just boracle`.
- [ ] Run `just validate`.
- [ ] Commit SCC solving.

## Phase 4: investigate deferred exclusive call activation

### Summary and reasoning

Current call argument loans can make an exclusive first argument block later shared argument evaluation. Because calls are static and argument order is explicit, Moth can investigate a broader reservation rule.

Proposed experiment:

```text
exclusive argument evaluation
    -> reserve exclusive capability

later argument evaluation
    -> compatible shared reads may proceed
    -> overlapping mutation or another exclusive reservation remains invalid

call effect starts
    -> activate exclusive capability
```

### Work

- [ ] Add `deferred-exclusive-call` as a named experiment.
- [ ] Add reserved and active capability phases.
- [ ] Reserve at the argument event.
- [ ] Activate at the call-effect event.
- [ ] End reservation without activation when later argument evaluation exits through failure.
- [ ] Allow only shared reads that preserve the reserved value.
- [ ] Reject:
  - [ ] replacement of the reserved place
  - [ ] overlapping exclusive access
  - [ ] another incompatible reservation
  - [ ] unknown external argument effect
- [ ] Keep mutable alias declarations outside calls under ordinary immediate rules.
- [ ] Do not infer activation from callee internals.
- [ ] Add source cases for method-like builtin calls where current syntax supports them.
- [ ] Compare every acceptance delta with the operational oracle.

### Required tests

- [ ] exclusive receiver plus later shared length read
- [ ] exclusive first argument plus later shared read
- [ ] two mutable arguments
- [ ] shared then mutable argument
- [ ] disjoint field arguments
- [ ] later argument failure before call effect
- [ ] copy as later argument
- [ ] unknown external boundary
- [ ] exact activation witness

### Phase gate

- [ ] Audit reservation compatibility rules.
- [ ] Audit failure paths that never activate.
- [ ] Review experiment isolation.
- [ ] Run `just boracle`.
- [ ] Commit deferred activation experiment.

## Phase 5: public-boundary review, documentation and closeout

### Summary and reasoning

Finish with a clear decision about which borrow summary facts belong in the future public semantic interface.

### Work

- [ ] Produce a summary-shape review covering:
  - [ ] per-result provenance
  - [ ] projection results
  - [ ] result-to-result relations
  - [ ] outcome-sensitive borrow effects
  - [ ] recursion convergence
  - [ ] deferred activation
- [ ] Compare the borrow-level facts with the accepted complete lifetime-summary contract.
- [ ] Decide which facts should later cross modules through stable semantic identities.
- [ ] Do not migrate public interfaces unless the architecture review explicitly accepts the shape.
- [ ] Promote accepted reference rules and keep unsettled rules as experiments.
- [ ] Update Boracle and borrow-validation docs.
- [ ] Review the progress matrix without advertising Boracle experiments as Alpha support.
- [ ] Run final scoped audits:
  - [ ] call evaluation and same-call conflicts
  - [ ] recursive summary soundness
  - [ ] compiler/build ownership
  - [ ] public identity and interface leakage
  - [ ] test honesty
  - [ ] documentation contradictions
- [ ] Resolve findings and run a fresh final audit.
- [ ] Remove this plan and roadmap entry in the completion commit.

### Final validation

- [ ] `cargo fmt --all`
- [ ] `just boracle`
- [ ] `just validate`
- [ ] `cargo run --quiet -- build docs --release`
- [ ] `git diff --check`

## Completion criteria

This package is complete only when:

- Boracle can model result slots independently
- outcome-specific borrow effects stay on their exact CFG edges
- recursive local and generated summaries converge deterministically
- deferred exclusive activation is explicit and experiment-only until accepted
- the build system never owns summary solving
- borrow facts remain separate from complete lifetime summaries
- every acceptance delta has operational evidence
- public-interface migration has an explicit reviewed decision
- the plan and roadmap entry are removed in the completion commit
