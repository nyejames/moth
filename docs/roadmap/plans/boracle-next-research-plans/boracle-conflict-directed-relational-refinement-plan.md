# Boracle conflict-directed relational refinement plan

Status: proposed

Current slice: not started

Blockers: typed provenance evidence, composable experiments and the bounded operational oracle must be complete

Next action: activate after those capabilities are available, collect the smallest known false conflicts caused by path correlation loss and freeze reference-mode snapshots

Repository path:

```text
docs/roadmap/plans/boracle-conflict-directed-relational-refinement-plan.md
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

Add an experimental second-stage refinement that runs only for candidate Boracle conflicts and proves whether the conflicting facts can coexist on one execution path.

The current coarse reference solve correctly unions possible origins and live loans at CFG joins. This is sound but loses correlations.

Example:

```text
then:
    x = left
    y = right
else:
    x = right
    y = left

mutate x
inspect y
```

If `left` and `right` are independent, `x` and `y` are independent on every path. Independent per-place origin unions still produce:

```text
x -> {left, right}
y -> {left, right}
```

A coarse solver can then report overlap that no execution contains.

The same problem appears when one path treats a mutable binding as an alias view and another treats it as a replaceable slot. The current mixed-mode implementation preserves both mode-specific origin sets, but it does not retain complete correlations between several places, loans and predecessor paths.

This package keeps the coarse solve as the fast and readable baseline, then refines only the slice needed to confirm or discharge one conflict.

## Prerequisites

The repository must already provide:

- typed origin overlap and disjointness evidence
- explicit precision-loss reasons
- composable named experiments
- bounded operational checking and replay
- deterministic conflict witnesses
- exact event boundaries
- mixed alias/slot origin alternatives
- modular last-use analysis

## Locked decisions

1. Reference mode keeps the current coarse conservative result until a refined rule is separately promoted.
2. The first implementation is a named `relational-refinement` experiment.
3. Refinement may discharge a conflict only when it proves that no compatible execution state contains the access and conflicting live capability together.
4. Failure to refine preserves the reference rejection.
5. Refinement never changes source evaluation order.
6. Refinement never changes optional transfer into a mandatory operation.
7. The first slice supports acyclic conflict slices. Cyclic slices remain conservative and are handed to loop-generation work.
8. The refined witness must be path-compatible:
   - loan issue
   - holder derivation
   - access
   - origin relationship
   - keeping use
   must belong to one compatible state alternative.
9. Do not retain complete path states for every successful function by default.
10. Do not use SMT or arbitrary predicate solving.
11. Use explicit state alternatives and CFG constraints.
12. The production checker is not implemented here. Boracle records which refinement was needed.
13. This plan is deleted with its roadmap entry in the completion commit.

## Target architecture

```text
coarse Boracle solve
    -> candidate conflict
    -> conflict relevance slice
    -> relational state exploration
    -> confirmed conflict or discharged conflict
    -> path-compatible witness or acceptance delta
```

Suggested module shape:

```text
src/compiler_frontend/analysis/borrow_checker/boracle/refinement/
|-- mod.rs
|-- candidate.rs
|-- slice.rs
|-- state.rs
|-- explore.rs
|-- relations.rs
|-- witness.rs
`-- tests/
```

Conceptual state:

```rust
struct RefinementState {
    cursor: EventCursor,
    control: BTreeSet<ControlChoice>,
    places: BTreeMap<PlaceId, RefinedPlaceState>,
    live_capabilities: BTreeSet<RefinedLoanInstance>,
    relations: RefinedRelations,
}

enum RefinedPlaceState {
    Slot {
        origins: BTreeSet<ValueOriginId>,
    },
    Alias {
        origins: BTreeSet<ValueOriginId>,
        access: AccessKind,
    },
}
```

Useful pairwise facts:

```text
MayAlias(a, b)
MustAlias(a, b)
MustBeDisjoint(a, b)
```

At a join:

```text
MayAlias       = union
MustAlias      = intersection
MustDisjoint   = intersection
```

Pairwise facts are the first refinement. Full state alternatives are used only when pairwise facts cannot preserve the required correlation.

## In scope

- candidate-conflict extraction
- backward relevance slicing
- acyclic path alternatives
- pairwise alias/disjoint relations
- mixed binding path correlation
- path-compatible witness construction
- named experimental acceptance deltas
- operational-oracle comparison
- generated and metamorphic cases
- production-handoff observations

## Out of scope

- loop iteration epochs
- arbitrary value predicates
- full symbolic execution of every function
- recursive call-summary solving
- lifetime topology
- retained-edge or REC decisions
- production optimization
- changing normal Alpha checker authority
- automatic promotion to reference semantics

## Phase 0: freeze the conflict corpus and refinement contract

### Summary and reasoning

Start from concrete conservative conflicts. A refinement engine without a classified corpus will grow into generic symbolic execution.

### Work

- [ ] Re-anchor the branch and record the working baseline in untracked notes.
- [ ] Collect reduced cases for:
  - [ ] swapped independent origins
  - [ ] branch-specific alias pairs
  - [ ] mixed alias/slot destination
  - [ ] branch-local loan issue
  - [ ] early return
  - [ ] fallible success-only alias
  - [ ] fallible error-only mutation
  - [ ] nested branch correlation
  - [ ] path-compatible and path-incompatible keeping uses
- [ ] Record current reference result and overlap evidence for each case.
- [ ] Run every case through the bounded operational oracle.
- [ ] Classify:
  - [ ] confirmed reference conflict
  - [ ] bounded precision opportunity
  - [ ] unsettled semantic question
  - [ ] malformed input
- [ ] Define the discharge contract in the Boracle authority.
- [ ] Define explicit reasons for refusing refinement:
  - [ ] cyclic slice
  - [ ] unknown call effect
  - [ ] dynamic storage domain
  - [ ] state limit
  - [ ] missing relation evidence
  - [ ] unsupported event
- [ ] Keep reference snapshots stable.

### Phase gate

- [ ] Audit each corpus case against canonical semantics.
- [ ] Review that no bounded oracle result is treated as unbounded proof.
- [ ] Run `just boracle`.
- [ ] Build docs when the contract changed.
- [ ] Commit the corpus and contract.

## Phase 1: build candidate-conflict and relevance slices

### Summary and reasoning

Refinement should inspect only facts that can affect one reported conflict.

### Work

- [ ] Add `CandidateConflict` built from the typed `ConflictWitness`.
- [ ] Include:
  - [ ] access event and use
  - [ ] access place and origins
  - [ ] conflicting loan
  - [ ] loan issue and holders
  - [ ] keeping use
  - [ ] structural overlap evidence
  - [ ] origin overlap evidence
  - [ ] precision losses
- [ ] Build a backward slice containing only:
  - [ ] events that define relevant places
  - [ ] events that create or kill the relevant loan
  - [ ] CFG edges connecting issue, access and use
  - [ ] relevant call and aggregate effects
  - [ ] branch terminators controlling those paths
- [ ] Validate that the slice preserves all relevant predecessor paths.
- [ ] Return a typed refusal when the slice contains an unsupported cycle.
- [ ] Add deterministic slice dumps.
- [ ] Add tests proving unrelated events do not enter the slice.
- [ ] Add tests proving a required kill or predecessor edge cannot be omitted.

### Phase gate

- [ ] Audit slice completeness against full CFG reachability.
- [ ] Review module boundaries and comments.
- [ ] Run focused slice tests.
- [ ] Run `just boracle`.
- [ ] Commit conflict slicing.

## Phase 2: add pairwise relational facts

### Summary and reasoning

Many false conflicts can be discharged without retaining full path states. Start with cheap, explicit pairwise relations.

### Work

- [ ] Compute `MayAlias`, `MustAlias` and `MustBeDisjoint` over the conflict slice.
- [ ] Seed positive disjointness from:
  - [ ] distinct fresh generations
  - [ ] explicit copies
  - [ ] accepted fixed-field facts
- [ ] Seed must-alias from exact origin identity.
- [ ] Carry branch-local facts through the slice.
- [ ] Join must facts by intersection.
- [ ] Discharge only when the access origin and loan origin are must-disjoint under every compatible predecessor.
- [ ] Keep an explicit proof chain for the discharge.
- [ ] Record why pairwise refinement was sufficient or insufficient.
- [ ] Compare each discharge with the operational oracle.
- [ ] Add a report section for acceptance deltas.

### Required tests

- [ ] swapped independent origins are discharged
- [ ] one path aliases and one path is disjoint remains conflicting
- [ ] copy on both paths stays disjoint
- [ ] distinct fields stay disjoint
- [ ] unknown call result blocks discharge
- [ ] branch splitting preserves the refined result
- [ ] predecessor renumbering preserves the refined result

### Phase gate

- [ ] Audit every discharge proof for universal path coverage.
- [ ] Review no-result and unknown handling.
- [ ] Run focused relational tests and oracle comparison.
- [ ] Run `just boracle`.
- [ ] Commit pairwise refinement.

## Phase 3: add bounded state alternatives

### Summary and reasoning

Pairwise facts cannot represent every correlation. Add explicit alternatives only inside the already reduced conflict slice.

### Work

- [ ] Add one `RefinementState` per compatible predecessor alternative.
- [ ] Preserve:
  - [ ] place storage role
  - [ ] exact origin set for the alternative
  - [ ] live capability instances
  - [ ] branch choices
- [ ] Model mixed alias/slot writes as two path alternatives rather than one union.
- [ ] Carry kills and scope exits per alternative.
- [ ] Add state subsumption:
  - [ ] remove an alternative only when another state is at least as conservative for every relevant fact
- [ ] Add a deterministic state limit.
- [ ] Refuse refinement rather than discharge when the limit is reached.
- [ ] Confirm a conflict only when one alternative contains the complete conflict.
- [ ] Discharge when every complete alternative is conflict-free.
- [ ] Build a path-compatible witness from the confirming alternative.
- [ ] Build an acceptance proof from all discharged alternatives.
- [ ] Do not handle cyclic slices yet.

### Required tests

- [ ] alias on one path and slot on another
- [ ] fresh replacement on slot path with write-through on alias path
- [ ] branch-local loan that cannot reach the access
- [ ] keeping use reachable only through an incompatible predecessor
- [ ] nested branches
- [ ] early return
- [ ] state-limit refusal
- [ ] witness facts all share one alternative

### Phase gate

- [ ] Audit state subsumption for unsound removal.
- [ ] Audit every confirmed witness for path compatibility.
- [ ] Compare all acceptance deltas with the operational oracle.
- [ ] Review state code for explicit readable control flow.
- [ ] Run `just boracle`.
- [ ] Run `just validate`.
- [ ] Commit bounded alternatives.

## Phase 4: integrate the named experiment and generated properties

### Summary and reasoning

The refinement remains an experiment until its acceptance deltas are durable, explainable and adversarially checked.

### Work

- [ ] Add `relational-refinement` to the experiment registry.
- [ ] Run coarse solve first.
- [ ] Invoke refinement only for candidate conflicts.
- [ ] Keep reference conflicts unchanged in reference mode.
- [ ] Record per conflict:
  - [ ] not attempted
  - [ ] refused with reason
  - [ ] confirmed
  - [ ] discharged by pairwise facts
  - [ ] discharged by alternatives
- [ ] Add deterministic generated problems with branch correlations.
- [ ] Add metamorphic properties:
  - [ ] equivalent branch splitting preserves the refined result
  - [ ] adding an unreachable origin alternative changes nothing
  - [ ] replacing one alias path with copy cannot make a discharged conflict return
  - [ ] adding one compatible aliasing path cannot make a confirmed conflict disappear
  - [ ] binding renumbering changes no semantic result
- [ ] Require every experiment acceptance delta to pass the bounded oracle.
- [ ] Keep large stress runs outside the default lane until measured.

### Phase gate

- [ ] Audit experiment isolation and report identity.
- [ ] Review generated cases for honest semantic ownership.
- [ ] Run `just boracle`.
- [ ] Measure refinement state counts and lane runtime.
- [ ] Commit experiment integration.

## Phase 5: promotion review, production handoff and closeout

### Summary and reasoning

The package should finish with a decision, not an indefinitely ambiguous experiment.

### Work

- [ ] Produce a promotion report containing:
  - [ ] accepted cases
  - [ ] rejected cases
  - [ ] refusal reasons
  - [ ] oracle results
  - [ ] maximum state counts
  - [ ] witness quality
  - [ ] remaining soundness questions
- [ ] Decide separately for:
  - [ ] pairwise must-disjoint refinement
  - [ ] mixed alias/slot state alternatives
  - [ ] general acyclic state alternatives
- [ ] Promote only rules accepted by canonical language and memory design.
- [ ] Keep unaccepted rules as named experiments with explicit status.
- [ ] Record the likely production architecture:
  - [ ] coarse may-analysis
  - [ ] conflict slice
  - [ ] targeted refinement
  - [ ] compact witness identity
- [ ] Do not implement production bitsets in this package.
- [ ] Update Boracle docs, borrow-validation docs and progress matrix according to actual authority.
- [ ] Run final scoped audits:
  - [ ] semantic soundness
  - [ ] path compatibility
  - [ ] experiment/reference isolation
  - [ ] test honesty
  - [ ] architecture and ownership
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

- candidate conflicts can be sliced deterministically
- pairwise relations preserve must-disjoint facts across joins
- bounded state alternatives preserve mixed path correlations
- no conflict is discharged without a universal path-compatible proof
- every confirmed witness comes from one compatible alternative
- every experimental acceptance delta passes the operational oracle
- reference mode remains unchanged until explicit promotion
- promotion status is recorded in canonical docs
- the plan and roadmap entry are removed in the completion commit
