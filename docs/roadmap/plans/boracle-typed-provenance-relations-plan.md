# Boracle typed provenance relations and experiment foundation plan

Status: active

Current slice: Phase 4 experiment selection

Blockers: none

Next action: replace the single experiment selector with a typed rule selection

Repository path:

```text
docs/roadmap/plans/boracle-typed-provenance-relations-plan.md
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

Replace Boracle's current untyped origin-relatedness set with an explicit provenance-relation and overlap-evidence model.

The current solver correctly separates bindings, value origins, places and loans. It also now preserves alias-backed and slot-backed alternatives at mixed CFG joins. The remaining weak point is that several different relationships still flow through one undirected `related_origins` relation:

- aliases or joined origin alternatives
- projection ancestry
- aggregate containment
- call-result aliasing
- overlap uncertainty
- facts reconstructed from traces

That relation works as a conservative baseline, but it cannot explain why two origins overlap, cannot carry positive disjointness and needs special handling for mixed traces to avoid inventing relationships between independent generations.

This package creates the semantic foundation needed by later conflict refinement, loop-generation work, call-summary research and storage-domain modelling.

## Prerequisites

The active branch must already provide:

- immutable validated `BorrowProblem` inputs
- parameter-aware origin propagation
- alias-derived and provenance-derived loans
- exact event boundaries
- mixed alias/slot alternatives
- origin-aware conflict checking
- modular last-use queries
- deterministic Boracle reports and named experimental mode
- the complete `just boracle` validation lane

Name these capabilities in roadmap ordering. Do not link this plan from another plan.

## Locked decisions

1. `ValueOriginId` remains a borrow/provenance identity. It does not become `AllocationFamilyId`.
2. Borrow validation continues to stop before lifetime topology, retained edges, groups, REC and physical memory planning.
3. `BorrowProblem` remains immutable and validated before publication.
4. Reference mode must preserve its current accept/reject results unless a separately reviewed semantic correction is required.
5. Experimental rules remain explicit and cannot silently redefine reference behaviour.
6. Missing internal provenance is either a validated top-like fact or `CompilerError`. It is never silently interpreted as fresh.
7. Proven disjointness is positive evidence. It is not represented only by the absence of an alias edge.
8. Directional relationships such as projection and containment must not be flattened into undirected equivalence.
9. Boracle remains readable and intentionally slow. Do not add Datalog, SMT, external graph libraries or packed production data structures.
10. Normal compilation must not construct Boracle problems or relation reports.
11. The build system must not solve provenance, select experiments or rerun borrow analysis.
12. API changes replace the old shape directly. Do not add compatibility wrappers around `related_origins`.
13. Test modules should be split by semantic owner where the existing single file becomes harder to review.
14. This plan is deleted with its roadmap entry in the commit that completes it. Durable decisions move into canonical docs.

## Target architecture

Add a focused Boracle-owned relation layer. Exact Rust names may change during implementation, but the concepts must stay separate.

```rust
pub(crate) enum OriginRelationKind {
    Projection {
        projection: ProjectionElem,
    },
    AggregateChild {
        projection: ProjectionElem,
    },
    CopyCorrespondence {
        copy_graph: CopyGraphId,
    },
    MayAlias {
        reason: PrecisionLossReason,
    },
    ProvenDisjoint {
        reason: DisjointReason,
    },
}

pub(crate) struct OriginRelation {
    pub(crate) left: ValueOriginId,
    pub(crate) right: ValueOriginId,
    pub(crate) kind: OriginRelationKind,
    pub(crate) evidence: OriginRelationEvidence,
}

pub(crate) enum OriginOverlapDecision {
    Overlap(OriginOverlapEvidence),
    Disjoint(OriginDisjointEvidence),
    Unknown(OriginUnknownEvidence),
}
```

Aliases that preserve one exact value generation should continue to carry the same `ValueOriginId`. Do not manufacture an alias-equivalence graph when identity already states the fact.

Useful reasons include:

```rust
pub(crate) enum PrecisionLossReason {
    UnknownCallResult,
    MissingLocalSummary,
    DynamicIndex,
    ConservativeStorageDomain,
    PathJoin,
    MixedBindingMode,
    LoopGenerationWidening,
    ExternalOpaqueValue,
}
```

```rust
pub(crate) enum DisjointReason {
    DifferentFreshGenerations,
    ExplicitCopy,
    DistinctFixedFields,
    DistinctFixedIndices,
    ExperimentProof,
}
```

The relation layer should answer one explicit query:

```text
may these two origin sets observe one source-semantic generation?
```

The answer carries evidence. Callers must not reconstruct the decision by walking traces or matching `OriginKind`.

## In scope

- typed provenance relationships
- typed overlap and disjointness evidence
- explicit unknown and precision-loss reasons
- deterministic relation dumps
- replacing `related_origins`
- hardening call-result and missing-origin fallbacks
- a composable experiment-selection value
- test-module cleanup needed by the new semantic families
- canonical Boracle documentation
- roadmap and progress-matrix review
- source and fixture regressions

## Out of scope

- path-disjunctive conflict refinement
- loop iteration epochs
- recursive call-summary inference
- complete public lifetime summaries
- retained-edge cardinality
- REC selection
- dynamic index arithmetic
- production bitsets or performance work
- changing Alpha borrow-checker authority
- changing source syntax

## Phase 0: re-anchor and relation inventory

### Summary and reasoning

Start from the current repository, not from the review snapshot. The mixed-provenance correction may have moved after this plan was written. Before changing semantics, record exactly which current facts are produced, consumed and rendered.

### Work

- [x] Rebase or merge current `main` according to the branch policy.
- [x] Record the activation commit and clean baseline in untracked working notes.
- [x] Run the complete existing Boracle lane before edits.
- [x] Inventory every producer and consumer of:
  - [x] `related_origins`
  - [x] `origins_overlap`
  - [x] `OriginKind`
  - [x] `CallResultProvenance`
  - [x] top-like unknown origins
  - [x] mixed origin traces
  - [x] copy independence
  - [x] projection and aggregate child relationships
  - [x] `BoracleExperiment`
  - [x] rule-set identity in reports and CLI output
- [x] Classify each current relationship as identity, directional derivation, possible overlap, proven disjointness or uncertainty.
- [x] Identify every fallback that creates a result origin when input provenance is empty.
- [x] Add reduced regression tests for any fallback whose safety depends on an undocumented invariant.
- [x] Confirm `BorrowProblem::validate` either enforces those invariants or leaves them as explicit top-like facts.
- [x] Update the small status block with the active slice, blockers and next action only.

### Phase gate

- [x] Audit the inventory against `problem/`, `last_use/`, `boracle/`, source service and direct docs.
- [x] Review names, comments and module ownership against the style guide.
- [x] Run `just boracle`.
- [x] Run `git diff --check`.
- [x] Commit the phase as one inventory and regression checkpoint.

## Phase 1: add the typed relation vocabulary

### Summary and reasoning

Create one narrow owner for provenance relationships before migrating conflict logic. The new vocabulary should make the current rules explicit without changing reference legality.

### Work

- [x] Add `src/compiler_frontend/analysis/borrow_checker/boracle/relations.rs`.
- [x] Give the file a WHAT/WHY owner comment and explicit exclusions.
- [x] Define typed directional and symmetric relationship rows.
- [x] Define overlap, disjoint and unknown decisions with typed evidence.
- [x] Define `PrecisionLossReason` and `DisjointReason`.
- [x] Keep relation construction deterministic.
- [x] Add validation for:
  - [x] unknown origin IDs
  - [x] invalid self-relations where the kind forbids them
  - [x] invalid projection evidence
  - [x] copy correspondence that aliases source and result
  - [x] contradictory proven-disjoint and forced-overlap facts
- [x] Add a stable debug dump.
- [x] Re-export only the stage-local surface needed by reports and later experiments.
- [x] Do not migrate the solver yet. Build focused hand-authored relation tests first.
- [x] Move Boracle unit tests into `boracle/tests/` if the new cases would make the existing test file harder to navigate.
- [x] Split tests by semantic family rather than by bug report.

### Required tests

- [x] same origin is overlapping by identity
- [x] fresh origins are disjoint
- [x] copy source and result are disjoint
- [x] projection relationship is directional
- [x] siblings do not become related through their parent
- [x] aggregate containment does not imply sibling overlap
- [x] unknown call result returns unknown overlap evidence
- [x] deterministic relation ordering and dumps
- [x] malformed relation rows fail as `CompilerError`

### Phase gate

- [x] Audit the new vocabulary for overlap with `OriginKind`, `PlaceOverlap` and lifetime-topology concepts.
- [x] Review the new file and test split against the style guide.
- [x] Run focused relation tests.
- [x] Run `just boracle`.
- [x] Run `git diff --check`.
- [x] Commit the typed vocabulary without changing reference results.

## Phase 2: migrate origin overlap and delete `related_origins`

### Summary and reasoning

Make the relation layer the only owner of origin overlap. This removes the current untyped graph and its mixed-trace exception from conflict logic.

### Work

- [x] Build relation rows from normalized origins and solved event state.
- [x] Preserve exact identity by reusing `ValueOriginId`.
- [x] Emit projection relationships from the actual projection source and path.
- [x] Emit aggregate child relationships without creating sibling edges.
- [x] Emit explicit copy disjointness.
- [x] Emit top-like uncertainty for unknown call results and opaque external values.
- [x] Represent mixed-state precision loss without relating every union member.
- [x] Replace `OriginSolution::origins_overlap` with one relation-owned overlap query.
- [x] Update loan conflict checking to consume `OriginOverlapDecision`.
- [x] Extend `ConflictWitness` with typed overlap evidence rather than a bare boolean.
- [x] Update reports and dumps.
- [x] Delete:
  - [x] `related_origins`
  - [x] `add_related_edge`
  - [x] mixed-trace skip logic that existed only to protect the untyped relation
  - [x] any duplicate overlap helper left in `loans.rs`
- [x] Keep structural `PlaceOverlap` separate. A conflict should retain both structural-place and origin evidence.
- [x] Snapshot reference accept/reject results before and after migration.
- [x] Any changed result must be classified before the phase closes.

### Required tests

- [x] all existing Boracle semantic tests retain their expected result
- [x] mixed alias/slot writes do not relate preserved alias and new slot generations
- [x] old aliases remain disjoint from fresh rebindings
- [x] copy independence survives branch joins
- [x] field siblings remain disjoint
- [x] base and child still overlap where required
- [x] unknown provenance stays conservative
- [x] witnesses state the exact relation reason

### Phase gate

- [x] Run a semantic-delta audit between the pre-migration and post-migration corpus.
- [x] Audit every deleted helper and update stale comments or names.
- [x] Review the complete origin-to-loan handoff against the compiler and memory authorities.
- [x] Run `cargo fmt --all`.
- [x] Run `just boracle`.
- [x] Run `git diff --check`.
- [x] Commit the migration and deletion as one coherent phase.

## Phase 3: harden unknown provenance and invariant failures

### Summary and reasoning

Unknown semantic data must be conservative. Missing internal data must be an internal error. This phase removes fallbacks that can make missing alias input look fresh or independent.

### Work

- [x] Audit every `OriginSet::is_empty()` branch that changes provenance meaning.
- [x] Replace the empty `AliasParams` result fallback with:
  - [x] `CompilerError` when normalized input promised a present argument origin
  - [x] explicit unknown/top provenance when the boundary is intentionally opaque
- [x] Add a typed reason to every unknown call result.
- [x] Distinguish:
  - [x] summary says unknown
  - [x] summary is unavailable
  - [x] external boundary is opaque
  - [x] normalized input is malformed
  - [x] loop or join widening lost precision
- [x] Strengthen `BorrowProblem::validate` where the invariant belongs at publication.
- [x] Keep solver checks for impossible states that can only be known after origin flow.
- [x] Ensure `CompilerError` messages identify the event, call, place and missing fact.
- [x] Do not add user-facing borrow diagnostics for internal invariant failure.
- [x] Add adversarial fixtures that deliberately construct malformed normalized problems.
- [x] Add real-source cases for conservative unknown local, generated, cross-module and external call summaries where supported by the source service.

### Phase gate

- [x] Audit every unknown and empty-origin branch in `problem/`, `origins.rs`, `loans.rs` and reports.
- [x] Review error lane ownership against the style guide.
- [x] Run focused malformed-problem tests.
- [x] Run `just boracle`.
- [x] Run `just validate`.
- [x] Commit the hardening slice.

## Phase 4: make experiment selection composable

### Summary and reasoning

Later research packages need named experiments that can run alone and in reviewed combinations. The current one-choice enum is too restrictive.

### Work

- [ ] Replace the single experiment selector with a typed rule selection:
  - [ ] one explicit reference rule-set version
  - [ ] a sorted set of named experiments
- [ ] Preserve `reference` as the default with an empty experiment set.
- [ ] Preserve `dead-exclusive-loan` as an explicit experiment.
- [ ] Reject incompatible experiment combinations.
- [ ] Record the full rule selection in every report and dump.
- [ ] Make CLI parsing repeatable or accept one deterministic comma-free repeated option shape.
- [ ] Keep the command internal and unstable.
- [ ] Add experiment metadata:
  - [ ] stable name
  - [ ] whether it may change legality
  - [ ] prerequisite experiment capabilities
  - [ ] reference promotion status
- [ ] Add `--dump relations` and `--dump precision` only if the report data justifies both.
- [ ] Do not expose experiments through normal `check`, build, config or source syntax.
- [ ] Update source-service and CLI tests.
- [ ] Delete the old single-enum path rather than retaining adapters.

### Phase gate

- [ ] Audit normal compilation and CLI help to prove Boracle remains feature-gated and internal.
- [ ] Review experiment naming and report determinism.
- [ ] Run `just feature-lane-check`.
- [ ] Run `just boracle`.
- [ ] Run `git diff --check`.
- [ ] Commit the experiment foundation.

## Phase 5: canonical documentation, final audits and closeout

### Summary and reasoning

Finish with durable relation semantics and remove the temporary plan. Experimental rule names may be documented as developer research tools, but user-facing language docs must not claim unsupported production behaviour.

### Work

- [ ] Update `boracle-reference-solver.mtf` with:
  - [ ] typed relation meanings
  - [ ] positive disjointness
  - [ ] unknown and precision-loss reasons
  - [ ] experiment-set rules
  - [ ] conflict evidence structure
- [ ] Update borrow-validation educational docs only where they currently describe the replaced relation model.
- [ ] Review the progress matrix:
  - [ ] do not mark experiment-only work as Alpha support
  - [ ] update only current implementation facts
- [ ] Update the roadmap ordering and current status.
- [ ] Build release docs and inspect generated output.
- [ ] Run a semantic-soundness audit focused on overlap and disjointness.
- [ ] Run an architecture audit focused on Stage 6, lifetime-topology separation and build-system isolation.
- [ ] Run a test-honesty audit focused on primary owners, duplicate fixtures and witness assertions.
- [ ] Run a documentation contradiction audit across compiler design, memory docs, Boracle docs and progress matrix.
- [ ] Resolve every required finding.
- [ ] Run a fresh final audit after corrections.
- [ ] Remove this plan and its roadmap entry in the completion commit.

### Final validation

- [ ] `cargo fmt --all`
- [ ] `just boracle`
- [ ] `just validate`
- [ ] `cargo run --quiet -- build docs --release`
- [ ] `git diff --check`
- [ ] verify the working tree contains no generated docs drift, stale plan reference or compatibility shim

## Completion criteria

This package is complete only when:

- `related_origins` no longer exists
- one typed relation owner answers origin overlap
- every overlap, disjoint or unknown decision carries evidence
- copy independence is positive data
- directional projection and containment do not create sibling overlap
- missing internal provenance cannot become fresh
- reference-mode results are either unchanged or explicitly approved
- experiment selection is composable and deterministic
- normal compilation remains isolated from Boracle
- canonical docs describe the final relation model
- the plan and roadmap entry are removed in the completion commit
