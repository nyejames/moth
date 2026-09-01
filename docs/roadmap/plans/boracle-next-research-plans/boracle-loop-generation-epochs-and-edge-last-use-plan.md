# Boracle loop generation epochs and edge-sensitive last-use plan

Status: proposed, inactive

Current slice: not started

Prerequisites: typed provenance evidence and the bounded operational oracle are complete; conflict-directed refinement should be available for acyclic slices; no current implementation owner is active

Next action: if approved, create a current owning plan from this note, re-anchor it against the active repository, then collect a reduced loop corpus that proves where one origin per definition site loses required generation distinctions

Not a reproduction: looping sources under `tests/cases/` once failed to replay under the bounded operational oracle, reporting an access that exercised a capability after its end. That was an oracle defect and not a missing generation distinction. The oracle re-exercised capability rows from earlier iterations because an interval did not record why it closed, and those sources replay now that it does. This plan still needs a reduced loop corpus that proves where one origin per definition site loses a required generation distinction, and that corpus does not exist yet.

Repository path:

```text
docs/roadmap/plans/boracle-next-research-plans/boracle-loop-generation-epochs-and-edge-last-use-plan.md
```

Canonical authorities:

- `docs/compiler-design-overview.md`
- `docs/src/developer-docs/memory-management/overview.mtf`
- `docs/src/developer-docs/memory-management/borrow-validation/borrow-validation.mtf`
- `docs/src/developer-docs/memory-management/boracle/boracle-reference-solver.mtf`
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`

## Purpose

Investigate and implement an explicit Boracle model for values and capabilities created by repeated executions of one static loop site.

The current reference origin solver assigns one `ValueOriginId` to one normalized fresh, copy, projection or call-result event. A loop may execute that event many times. Using one static origin for every iteration can lose the fact that:

```text
current iteration's fresh generation
    is distinct from
the prior iteration's generation from the same site
```

The same issue applies to loans issued by one alias site on several iterations.

This package also adds edge-sensitive last-use queries. A value can have a future use on a back edge and no future use on an exit edge. Point-only results cannot express that distinction precisely.

## Prerequisites

The repository must already provide:

- exact normalized CFG event order
- typed provenance overlap evidence
- explicit precision-loss reasons
- bounded operational loop replay
- named experiments
- modular last-use analysis
- conflict-directed acyclic refinement
- deterministic source and hand-authored loop cases

## Locked decisions

1. One static definition site is not claimed to be one runtime value.
2. The first experiment uses a small abstract generation domain. It does not retain unbounded iteration histories.
3. Fresh-per-iteration disjointness needs an explicit proof that the site executes at most once per loop iteration path.
4. Exit-edge last use does not retroactively make an earlier pre-branch operation a final use.
5. Optional transfer remains invisible and conservative.
6. Loan instances need generation classes when one static issue site executes several times.
7. Infinite no-use continuations matter to `MustBeUsed`.
8. A bounded oracle trace can find a counterexample but cannot prove unbounded soundness.
9. Loop facts remain borrow and last-use facts. They do not create lifetime regions or physical region epochs.
10. The later lifetime analysis may consume loop facts, but Boracle does not assign lifetime owners.
11. No loop unrolling is added to source or HIR.
12. Do not solve arbitrary numeric induction variables or dynamic index arithmetic.
13. Reference mode remains conservative until each loop rule is promoted.
14. This plan is deleted with its roadmap entry in the completion commit.

## Target architecture

Useful concepts:

```rust
pub(crate) struct LoopId(u32);
pub(crate) struct OriginSiteId(u32);
pub(crate) struct LoanSiteId(u32);

pub(crate) enum GenerationClass {
    OutsideLoop,
    CurrentIteration(LoopId),
    PriorIteration(LoopId),
    EarlierIterations(LoopId),
    UnknownMany(LoopId),
}
```

The exact representation may differ. The semantic facts must support:

```text
FreshEachIteration(site, loop)
Disjoint(Current(site, loop), Prior(site, loop))
BackEdgeCarries(place, site, loop)
```

Edge-sensitive query:

```rust
pub(crate) enum LastUseLocation {
    AtPoint(PointId),
    AfterEvent {
        event: EventId,
        point: PointId,
    },
    OnEdge(CfgEdgeId),
}
```

Witness additions:

```rust
pub(crate) enum NoUseContinuation {
    Exit(BlockId),
    ClosedCycle(LoopId),
}
```

## In scope

- stable CFG edge identity where required
- loop SCC and back-edge classification
- one-site generation classes
- current and prior iteration disjointness
- loop-carried place state
- loop loan-instance classes
- edge-sensitive last-use
- no-use cycle witnesses
- exit-edge final-use candidates
- bounded operational comparison
- generated loop properties
- experiment promotion review

## Out of scope

- physical lifetime region epochs
- retained-edge cleanup frontiers
- REC planning
- arbitrary recurrence solving
- loop unrolling
- general induction-variable analysis
- dynamic collection index disjointness
- async suspension
- production optimization
- changing source syntax

## Phase 0: build the adversarial loop corpus

### Summary and reasoning

Do not design the generation lattice from one toy loop. Start by proving which distinctions matter.

### Work

- [ ] Re-anchor the branch and record the working baseline in untracked notes.
- [ ] Add reduced cases for:
  - [ ] zero iterations
  - [ ] one iteration
  - [ ] several iterations
  - [ ] fresh replacement of a loop-carried value
  - [ ] alias to the previous generation
  - [ ] projection replacement
  - [ ] copy in a loop
  - [ ] exclusive alias issued each iteration
  - [ ] alias killed before `continue`
  - [ ] alias carried across `continue`
  - [ ] `break` before and after use
  - [ ] fallible success and error exits
  - [ ] nested loop
  - [ ] closed infinite no-use cycle
  - [ ] closed cycle with a use
  - [ ] call result created at one loop site
  - [ ] aggregate child retained from a prior iteration
- [ ] Record current origin, loan and last-use results.
- [ ] Replay bounded versions through the operational oracle.
- [ ] Identify cases where one origin per site:
  - [ ] creates a false overlap
  - [ ] hides a real overlap
  - [ ] changes last-use classification
  - [ ] only affects later lifetime analysis
- [ ] Separate borrow problems from lifetime-topology problems.
- [ ] Write the minimum required generation distinctions into the Boracle authority as an experiment contract.

### Phase gate

- [ ] Audit each case against source semantics and evaluation order.
- [ ] Review that no lifetime-region decision leaked into the corpus.
- [ ] Run `just boracle`.
- [ ] Build docs when the contract changed.
- [ ] Commit the corpus and classification.

## Phase 1: add stable edge identity and edge last-use queries

### Summary and reasoning

Loop exit and back-edge continuations must be queryable directly. Edge identity should be added only where shared consumers need it.

### Work

- [ ] Inventory every current `CfgEdge` consumer.
- [ ] Add `CfgEdgeId` if deterministic edge identity cannot be recovered safely from existing rows.
- [ ] Keep edge IDs dense and validated.
- [ ] Extend `BorrowProblem::validate` for edge ownership and target validity.
- [ ] Add `LastUseLocation::OnEdge`.
- [ ] Define an edge query as starting after the predecessor terminator and before the successor's first event.
- [ ] Preserve exact call-event boundaries.
- [ ] Add no-use continuation witnesses:
  - [ ] reachable exits
  - [ ] closed no-use SCCs
- [ ] Correct `MustBeUsed` so an infinite no-use continuation prevents a must result.
- [ ] Keep place, origin and loan subjects supported.
- [ ] Add deterministic edge and witness dumps.

### Required tests

- [ ] back edge may use and exit edge has no use
- [ ] both edges use
- [ ] neither edge uses
- [ ] early return edge
- [ ] fallible success and error edges
- [ ] closed no-use cycle prevents `MustBeUsed`
- [ ] closed cycle with unavoidable use yields `MustBeUsed`
- [ ] edge renumbering remains deterministic

### Phase gate

- [ ] Audit temporal meaning at terminators and successor entry.
- [ ] Review CFG identity changes against `BorrowProblem` ownership.
- [ ] Run last-use tests.
- [ ] Run `just boracle`.
- [ ] Run `just validate`.
- [ ] Commit edge-sensitive last use.

## Phase 2: derive loop sites and abstract generations

### Summary and reasoning

Introduce a solver-owned loop model without making `ValueOriginId` or `BorrowProblem` claim runtime multiplicity.

### Work

- [ ] Derive loop SCCs in deterministic order.
- [ ] Identify back edges and loop entries.
- [ ] Identify origin definition sites inside each loop.
- [ ] Prove whether a site can execute more than once on one logical iteration path.
- [ ] Add solver-owned `OriginSiteId` and generation classes.
- [ ] Start with:
  - [ ] `CurrentIteration`
  - [ ] `PriorIteration`
  - [ ] `UnknownMany`
- [ ] Add `OutsideLoop` only where it simplifies joins.
- [ ] At a back edge:
  - [ ] move current generation facts to prior
  - [ ] widen older prior facts to unknown-many when required
- [ ] Keep positive disjointness between current and prior for proven fresh-per-iteration sites.
- [ ] Do not prove disjointness for alias, unknown call result or reused aggregate child sites.
- [ ] Record every widening reason.
- [ ] Keep the experiment separate from reference mode.

### Required tests

- [ ] current and prior fresh generations are disjoint
- [ ] current alias and prior source may overlap
- [ ] copy result remains independent from source on each iteration
- [ ] unknown call result stays conservative
- [ ] nested loop sites receive distinct loop identities
- [ ] site that can execute twice per iteration is not given unsafe current/prior disjointness

### Phase gate

- [ ] Audit fresh-per-iteration proof preconditions.
- [ ] Compare bounded cases with the operational oracle.
- [ ] Review generation code for explicit state and no hidden lifetime ownership.
- [ ] Run `just boracle`.
- [ ] Commit generation classes.

## Phase 3: model loop loan instances and conflict checking

### Summary and reasoning

Origins alone are insufficient. A static loan issue site can create separate runtime capabilities on different iterations.

### Work

- [ ] Add solver-owned `LoanSiteId`.
- [ ] Classify loan instances as current, prior or unknown-many.
- [ ] Carry holder uses and kills per generation class.
- [ ] Ensure a kill in the current iteration does not kill a prior instance unless the holder relationship proves it.
- [ ] Ensure a rebind that replaces a loop-carried holder kills only the relevant instance.
- [ ] Extend conflict evidence with loop and generation class.
- [ ] Integrate generation-aware overlap into the loop experiment.
- [ ] Keep the coarse reference conflict when generation proof is unavailable.
- [ ] Add path-compatible witnesses across back edges.

### Required tests

- [ ] one alias instance per iteration
- [ ] prior shared loan versus current exclusive access
- [ ] current loan killed before back edge
- [ ] prior loan retained through aggregate child
- [ ] dead exclusive alias each iteration
- [ ] branch-local loop loan
- [ ] no witness combines current issue with prior keeping use incorrectly

### Phase gate

- [ ] Audit loan-instance kill semantics.
- [ ] Audit witness generation compatibility.
- [ ] Compare bounded executions with the oracle.
- [ ] Run `just boracle`.
- [ ] Run `just validate`.
- [ ] Commit loop loan instances.

## Phase 4: expose final-iteration and exit-edge facts safely

### Summary and reasoning

The useful outcome is not a magical final-iteration operator. It is precise evidence at control-flow locations that already execute only on exit paths.

### Work

- [ ] Add queries for:
  - [ ] origin final use on a loop exit edge
  - [ ] loan final use on a loop exit edge
  - [ ] place final use in an exit-only block
- [ ] Do not mark an event before a loop condition as final based only on a later exit choice.
- [ ] Do not change source evaluation or move operations between blocks.
- [ ] Record when a transfer candidate is blocked by:
  - [ ] back-edge use
  - [ ] prior-generation alias
  - [ ] reactive observation
  - [ ] unknown call effect
  - [ ] retained aggregate relationship
- [ ] Keep transfer as a later consumer decision.
- [ ] Add report output for edge facts and witnesses.
- [ ] Add source cases where an exit-only call can receive a final-use candidate.

### Phase gate

- [ ] Audit every final-use candidate against exact execution order.
- [ ] Review that last use remains separate from complete transfer and lifetime topology.
- [ ] Run last-use and Boracle tests.
- [ ] Run `just boracle`.
- [ ] Commit exit-edge facts.

## Phase 5: generated loop properties, promotion review and closeout

### Summary and reasoning

Loop rules need stronger evidence than acyclic rules because bounded execution cannot prove arbitrary iteration counts.

### Work

- [ ] Add deterministic generated loop problems.
- [ ] Add metamorphic properties:
  - [ ] loop peeling preserves reference legality
  - [ ] one unrolled iteration agrees with the current/prior abstraction
  - [ ] adding an exit edge cannot create a must-use result
  - [ ] deleting a back-edge use cannot extend a loan
  - [ ] fresh site renumbering changes no result
  - [ ] nested loop renumbering changes no result
- [ ] Compare bounds 0, 1, 2 and 3 where the oracle completes.
- [ ] Write a soundness argument for each promoted generation rule.
- [ ] Decide which parts become reference semantics:
  - [ ] edge-sensitive last use
  - [ ] no-use cycle witnesses
  - [ ] current/prior fresh disjointness
  - [ ] loan generation classes
- [ ] Keep unresolved rules as named experiments.
- [ ] Update canonical docs and progress matrix only to match actual authority and production status.
- [ ] Run final scoped audits:
  - [ ] loop soundness
  - [ ] infinite continuation handling
  - [ ] edge temporal meaning
  - [ ] loan-instance compatibility
  - [ ] test honesty
  - [ ] architecture and documentation
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

- loop edges have stable queryable identity
- last-use results distinguish back-edge and exit-edge continuations
- infinite no-use cycles are represented honestly
- fresh loop sites can distinguish current and prior generations under explicit preconditions
- loop loan instances cannot be combined across incompatible iterations
- final-use facts never change source evaluation order
- promoted rules have a written soundness argument and adversarial corpus
- unresolved rules remain named experiments
- the plan and roadmap entry are removed in the completion commit
