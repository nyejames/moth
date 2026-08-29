# Boracle bounded operational oracle and reducer plan

Status: proposed

Current slice: not started

Blockers: typed provenance overlap evidence and explicit unknown reasons must be available so static and operational disagreements can be classified without guessing

Next action: activate after the relation foundation is complete, establish the current semantic corpus baseline and specify the smallest executable normalized semantics

Repository path:

```text
docs/roadmap/plans/boracle-bounded-operational-oracle-plan.md
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

Build a small deterministic operational oracle for bounded `BorrowProblem` executions.

Boracle is a static reference solver. Its generated tests currently compare deterministic static results and useful metamorphic transformations. Those tests can still share the same mistaken abstraction as the solver.

The oracle takes the opposite route:

```text
BorrowProblem
    -> explicit dynamic execution states
    -> branch and bounded-loop enumeration
    -> runtime generation and capability checks
    -> complete trace or bounded inconclusive result
```

The oracle is not a second source compiler and not the production checker. It is a deliberately different executable semantics used to find:

- likely soundness defects when static acceptance permits an enumerated invalid execution
- precision opportunities when static rejection conflicts with every complete bounded execution being legal
- malformed normalized inputs whose static and dynamic meanings diverge
- minimal counterexamples for later experiments

## Prerequisites

The active repository must provide:

- validated immutable normalized problems
- exact event order
- typed provenance overlap evidence
- explicit unknown reasons
- deterministic Boracle reports
- generated problem infrastructure
- named experiments separated from reference mode
- the opt-in `just boracle` lane

## Locked decisions

1. The oracle consumes `BorrowProblem`. It does not parse source or walk arbitrary HIR.
2. The oracle uses a substantially different algorithm from the static solver.
3. The oracle must not call origin-flow, loan-liveness or static conflict-decision helpers to decide runtime legality.
4. Bounded execution is evidence and a counterexample finder. It is not proof for unbounded loops.
5. A truncated state space returns `Inconclusive`. It must never be reported as safe.
6. Dynamic value generations are concrete per execution event.
7. `copy` creates a new dynamic graph while preserving internal alias topology.
8. Runtime aliases and capabilities are modelled directly rather than reconstructed from static loan rows.
9. Calls initially execute normalized call effects only. They do not open callee HIR.
10. The oracle remains feature-gated with Boracle.
11. No external fuzzing, solver or graph dependency is added.
12. The default bounded oracle must remain deterministic and suitable for `just boracle`.
13. Larger stress exploration stays outside the ordinary gate unless measured runtime proves it remains bounded.
14. Every disagreement prints a replayable normalized problem and execution trace.
15. This plan is deleted with its roadmap entry in the completion commit.

## Target architecture

Add a separate Boracle submodule:

```text
src/compiler_frontend/analysis/borrow_checker/boracle/oracle/
|-- mod.rs
|-- state.rs
|-- execute.rs
|-- calls.rs
|-- traces.rs
|-- generator.rs
|-- reducer.rs
`-- tests/
```

Conceptual runtime state:

```rust
struct OracleState {
    event: EventCursor,
    places: BTreeMap<PlaceId, RuntimePlaceState>,
    aggregates: BTreeMap<DynamicOriginId, RuntimeAggregate>,
    capabilities: BTreeMap<RuntimeCapabilityId, RuntimeCapability>,
    path: Box<[ExecutedEdge]>,
}

enum RuntimePlaceState {
    Unavailable,
    Slot {
        current: DynamicOriginId,
    },
    Alias {
        target: DynamicOriginId,
        access: AccessKind,
    },
}

struct RuntimeCapability {
    kind: AccessKind,
    origin: DynamicOriginId,
    holders: BTreeSet<PlaceId>,
    usable: bool,
}
```

Conceptual outcome:

```rust
enum OracleOutcome {
    CompleteSafe {
        executions: u64,
    },
    RuntimeConflict {
        trace: OracleTrace,
    },
    Inconclusive {
        reason: OracleLimitReason,
        explored: u64,
    },
}
```

## In scope

- direct execution of normalized events
- branch enumeration
- bounded loop enumeration
- exact dynamic origin generations
- shared and exclusive capability checks
- copy graph reconstruction
- aggregate child relationships
- exact event and CFG traces
- deterministic generated problems
- static-oracle disagreement classification
- deterministic reduction
- source-service smoke cases that lower to replayable problems
- bounded integration into `just boracle`

## Out of scope

- proving unbounded loop soundness
- executing backend code
- opening callee HIR
- lifetime topology
- retained edges and REC
- allocator or drop simulation
- user-facing diagnostics
- random nondeterminism
- external fuzzing frameworks
- production performance data structures
- replacing reference mode

## Phase 0: define the executable normalized semantics

### Summary and reasoning

Do not start with a generator. First write down one explicit operational meaning for every normalized event the oracle will execute.

### Work

- [ ] Re-anchor the active branch and record the working baseline in untracked notes.
- [ ] Inventory every current `EventKind`, `TerminatorEventKind`, place projection and call effect.
- [ ] Classify each event as:
  - [ ] state mutation
  - [ ] observation
  - [ ] capability issue
  - [ ] capability use
  - [ ] capability kill
  - [ ] control-flow choice
  - [ ] metadata-only event
- [ ] Specify evaluation order for access and semantic write events.
- [ ] Specify runtime meaning for alias-only, slot-backed and mixed static states without reusing the static mode lattice.
- [ ] Specify how an execution chooses one concrete predecessor state at a join.
- [ ] Specify how `copy` maps each reachable source node to exactly one result node.
- [ ] Specify aggregate outer identity and stored child identity.
- [ ] Specify call-argument reservation and call-effect completion under the current reference semantics.
- [ ] Specify scope exit and unreachable control flow.
- [ ] List unsupported normalized shapes. Unsupported shapes must return typed `Inconclusive` or `CompilerError`, never implicit success.
- [ ] Add the operational contract to the Boracle developer authority before implementation if it resolves a previously ambiguous accepted rule.

### Phase gate

- [ ] Audit the operational contract against `BorrowProblem` validation and current source extraction.
- [ ] Audit the boundary against lifetime topology and backend semantics.
- [ ] Review terminology and failure lanes against the style guide.
- [ ] Run `just boracle`.
- [ ] Build docs when the authority changed.
- [ ] Commit the contract and any invariant tests.

## Phase 1: implement straight-line exact execution

### Summary and reasoning

Start with acyclic single-path problems. This isolates runtime generation, alias and capability semantics before path enumeration.

### Work

- [ ] Add the oracle module and typed state.
- [ ] Execute:
  - [ ] `Fresh`
  - [ ] shared alias
  - [ ] exclusive alias
  - [ ] `Copy`
  - [ ] `Projection`
  - [ ] `Rebind`
  - [ ] `Aggregate`
  - [ ] `Access`
  - [ ] `LoanIssue`
  - [ ] `LoanKill`
  - [ ] `ScopeExit`
  - [ ] return and failure terminators
- [ ] Give every dynamic generation an execution-local identity.
- [ ] Check access legality directly against runtime capabilities.
- [ ] Keep one complete event trace.
- [ ] Do not derive runtime capabilities from static `LoanFact` rows.
- [ ] Return `CompilerError` for impossible normalized cross-references.
- [ ] Return typed `Inconclusive` for intentionally unsupported event semantics.
- [ ] Add deterministic debug output.

### Required tests

- [ ] many shared aliases
- [ ] shared then exclusive conflict
- [ ] exclusive then shared conflict
- [ ] write through mutable alias
- [ ] fresh slot replacement
- [ ] copy independence
- [ ] repeated child alias inside one aggregate
- [ ] projection access
- [ ] scope exit
- [ ] call argument interval under current semantics
- [ ] exact trace determinism

### Phase gate

- [ ] Audit the executor for accidental calls into static solver logic.
- [ ] Review state names, comments and file ownership.
- [ ] Run focused oracle tests.
- [ ] Run `just boracle`.
- [ ] Run `git diff --check`.
- [ ] Commit straight-line execution.

## Phase 2: enumerate branches and bounded loops

### Summary and reasoning

Path enumeration must choose concrete control flow rather than union predecessor facts. This is the main independence from the static solver.

### Work

- [ ] Add an explicit event cursor over CFG blocks and event indexes.
- [ ] Enumerate branch targets in deterministic ID order.
- [ ] Preserve one independent runtime state per path.
- [ ] Add state-space limits:
  - [ ] maximum executions
  - [ ] maximum steps
  - [ ] maximum visits per back edge
  - [ ] maximum live runtime generations
- [ ] Return `Inconclusive` when any limit truncates a relevant path.
- [ ] Treat recoverable success and error edges as distinct executions.
- [ ] Treat `break`, `continue`, runtime failure and assertion failure as explicit exits.
- [ ] Detect closed cycles that make no progress toward an exit.
- [ ] Record loop visit counts in traces.
- [ ] Do not merge runtime states merely to reduce cost.
- [ ] Add a small deterministic default bound suitable for the normal gate.

### Required tests

- [ ] path-separated shared use and mutation
- [ ] branch join with swapped independent values
- [ ] early return
- [ ] success-only alias
- [ ] error-only mutation
- [ ] zero, one and several loop iterations
- [ ] infinite cycle becomes inconclusive
- [ ] limit changes do not change a complete result
- [ ] deterministic path order

### Phase gate

- [ ] Audit every truncation path for false-safe outcomes.
- [ ] Review control-flow code for readability and explicit state.
- [ ] Run focused branch and loop oracle tests.
- [ ] Run `just boracle`.
- [ ] Commit bounded control-flow execution.

## Phase 3: compare static and operational results

### Summary and reasoning

A useful oracle reports classified disagreements rather than a boolean mismatch.

### Work

- [ ] Define disagreement classes:
  - [ ] static accepted and runtime conflict found
  - [ ] static rejected and every complete bounded execution is safe
  - [ ] static and oracle agree
  - [ ] oracle inconclusive
  - [ ] malformed problem
  - [ ] experiment-only accepted difference
- [ ] Treat static acceptance plus runtime conflict as a required high-severity failure.
- [ ] Treat static rejection plus complete bounded safety as a precision candidate, not automatic proof.
- [ ] Include:
  - [ ] rule set
  - [ ] experiment set
  - [ ] normalized problem
  - [ ] static witness
  - [ ] runtime trace
  - [ ] bounds
  - [ ] disagreement classification
- [ ] Compare reference mode first.
- [ ] Compare each legality-changing experiment separately.
- [ ] Reject a report that omits rule-set identity.
- [ ] Add source-service cases whose normalized problem can be replayed by the oracle.

### Phase gate

- [ ] Audit disagreement severity and terminology.
- [ ] Review that experiment acceptance never becomes reference acceptance automatically.
- [ ] Run all static-oracle comparison tests.
- [ ] Run `just boracle`.
- [ ] Commit the differential layer.

## Phase 4: deterministic generation and reduction

### Summary and reasoning

Generated disagreements need automatic reduction into durable semantic cases. The reducer must preserve the disagreement, not only produce another well-formed problem.

### Work

- [ ] Extend the existing deterministic problem generator rather than adding a second unrelated generator.
- [ ] Generate bounded combinations of:
  - [ ] block shapes
  - [ ] branches
  - [ ] back edges
  - [ ] fresh origins
  - [ ] aliases
  - [ ] copies
  - [ ] projections
  - [ ] aggregates
  - [ ] calls
  - [ ] kills and scope exits
- [ ] Retain the seed and full normalized input.
- [ ] Add a reducer that attempts, in deterministic order:
  - [ ] remove unreachable blocks
  - [ ] remove events
  - [ ] remove uses and loans
  - [ ] remove edges
  - [ ] simplify projections
  - [ ] reduce origins
  - [ ] reduce bindings
  - [ ] replace calls with simpler effects
  - [ ] lower loop bounds
- [ ] Validate every candidate before execution.
- [ ] Keep a candidate only when it preserves the same disagreement class.
- [ ] Print a hand-authored fixture skeleton for a minimal result.
- [ ] Add every confirmed semantic discovery to the durable corpus.
- [ ] Keep large campaigns outside the default lane until measured.

### Required properties

- [ ] replacing an alias with copy cannot create a runtime alias conflict
- [ ] fresh rebinding separates the new dynamic generation
- [ ] adding an unreachable use changes no complete execution
- [ ] deleting a final use cannot extend runtime capability usability
- [ ] branch renumbering preserves complete outcomes
- [ ] repeated execution with one seed is byte-for-byte deterministic
- [ ] reduction preserves disagreement class

### Phase gate

- [ ] Audit generator validity and reducer honesty.
- [ ] Review duplicate coverage against the testing guide.
- [ ] Measure default lane runtime.
- [ ] Run `just boracle`.
- [ ] Run `just validate`.
- [ ] Commit generator and reducer work.

## Phase 5: validation integration, documentation and closeout

### Summary and reasoning

The bounded oracle becomes permanent Boracle infrastructure only when its limits, guarantees and failure modes are explicit.

### Work

- [ ] Add the bounded default oracle corpus to `just boracle`.
- [ ] Add a separate stress command only if measured runtime proves it cannot belong in the default lane.
- [ ] Ensure all reports land under the existing test-report policy or remain direct test failure output.
- [ ] Do not write partial reports directly to final paths.
- [ ] Update Boracle docs with:
  - [ ] oracle purpose
  - [ ] exact versus static algorithm separation
  - [ ] bounded guarantees
  - [ ] inconclusive meaning
  - [ ] disagreement workflow
  - [ ] reduction workflow
- [ ] Review the progress matrix. Do not advertise the oracle as user-facing borrow support.
- [ ] Run final scoped audits:
  - [ ] runtime-semantics audit
  - [ ] static-oracle independence audit
  - [ ] bound and truncation audit
  - [ ] generator and reducer honesty audit
  - [ ] docs and roadmap audit
- [ ] Resolve required findings and run a fresh final review.
- [ ] Remove this plan and its roadmap entry in the completion commit.

### Final validation

- [ ] `cargo fmt --all`
- [ ] `just boracle`
- [ ] `just validate`
- [ ] `cargo run --quiet -- build docs --release`
- [ ] `git diff --check`
- [ ] inspect default Boracle lane runtime and deterministic output

## Completion criteria

This package is complete only when:

- the oracle executes normalized problems without reusing static legality logic
- dynamic generations and capabilities are explicit
- branches and bounded loops are enumerated deterministically
- truncated exploration is always inconclusive
- static acceptance plus an enumerated runtime conflict fails loudly
- precision disagreements carry replayable traces
- generated failures can be reduced while preserving their class
- the default bounded corpus runs in `just boracle`
- canonical docs state the oracle's exact limits
- the plan and roadmap entry are removed in the completion commit
