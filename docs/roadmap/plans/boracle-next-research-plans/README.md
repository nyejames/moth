# Recommended research sequence

## Current handoff

Package 1 is complete. Package 2, **Bounded operational oracle**, is complete. This directory retains
their completed handoff and inactive proposal notes for packages 3 through 6. It is not an active
work source. Do not activate a proposal directly from here; create a current owning plan and
re-anchor it against the active repository first.

The package 1 blocker was cleared before package 2 began: typed provenance overlap evidence,
explicit unknown reasons, deterministic reports and composable experiment selection were in the
`boracle` branch. Package 2 then added the bounded operational oracle and its generated
differential campaign.

The roadmap currently has no owning implementation plan for packages 3 through 6. The retained
proposal notes are design material only, not current implementation work.

`checked-proof-budget-integration.md` records a future open proposal for opt-in deeper analysis and
the architecture each package should preserve. It is design awareness only, not an implementation
requirement for checked source syntax.

The completed package plans are not retained in this directory. Their durable results live in the
result tables below and the permanent authorities they link; detailed implementation history remains
recoverable from Git.

| Order | Package | Status | Main result |
| ----: | --------------------------------------------- | ------------------- | -------------------------------------------------------------------------------------------- |
| 1 | Typed provenance relations | complete | Replaces the overloaded origin relation and establishes composable experiments |
| 2 | Bounded operational oracle | complete | Provides an independent executable check against unsound experimental acceptance |
| 3 | [Conflict-directed relational refinement](./boracle-conflict-directed-relational-refinement-plan.md) | proposed, inactive | Implements the core coarse-solve then targeted-refinement architecture |
| 4 | [Loop generation epochs](./boracle-loop-generation-epochs-and-edge-last-use-plan.md) | proposed, inactive | Adds iteration-sensitive generations and edge-specific last-use reasoning |
| 5 | [Call summaries and deferred exclusive access](./boracle-call-summary-and-deferred-exclusive-access-plan.md) | proposed, inactive | Explores per-result, outcome-sensitive, recursive summaries and reserved exclusive arguments |
| 6 | [Aggregate copy and builtin storage provenance](./boracle-aggregate-copy-and-builtin-storage-provenance-plan.md) | proposed, inactive | Models deep-copy topology, fixed projection precision and compiler-known storage effects |

### Package 1 result

`OriginRelations` is the only origin-overlap owner. Loan conflicts consume `OriginOverlapDecision`. Empty `AliasParams` is malformed input. Missing argument, copy, projection, and aggregate provenance cannot become a fresh generation. Rule selection is `boracle-reference-v1` plus a sorted experiment set; `dead-exclusive-loan` remains the only named experiment. Canonical Boracle docs describe that model. Production Stage 6 still uses storage-root overlap and does not consume `OriginRelations`.

Checkpoints on `boracle`:

| Phase | Commit |
| --- | --- |
| 0 Activate | `7bfcd10eb` activate typed provenance relations and pin empty AliasParams fallbacks |
| 1 Vocabulary | `e65948094` add typed provenance relation vocabulary |
| 2 Origin overlap | `72276e937` migrate origin overlap to typed relations |
| 3 Unknown provenance | `b626dfda1` harden unknown provenance and invariant failures |
| 4 Experiments | `529127426` make experiment selection composable |
| 5 Docs and closeout | `01294b2c5` document typed relations and retire the plan |

### Package 2 result

The oracle executes a normalized `BorrowProblem` directly rather than consulting the static solver. It carries a typed runtime state of dynamic generations and capabilities, enumerates branches and bounded loops deterministically, and classifies each problem against the static answer. Static acceptance plus an enumerated runtime conflict is a soundness failure and fails the lane loudly; every truncated exploration is typed `Inconclusive` with a named reason, never a safe result. A deterministic generator and a class-preserving reducer supply the campaign, and `just boracle` runs the default bounded corpus while `just boracle-campaign` runs the measured stress lane.

`docs/src/developer-docs/memory-management/boracle/boracle-operational-oracle.mtf` is the permanent operational authority. It states the oracle's exact limits: the shapes it refuses, the reasons it reports and the places where it is deliberately stricter or more approximate than the reference.

Checkpoints on `boracle`:

| Phase | Commit |
| --- | --- |
| 0 Semantics | `2f46958c5` specify the bounded oracle operational semantics |
| 1 Straight line | `ce2688cfe` execute straight-line oracle problems |
| 2 Control flow | `b11cbf15c` enumerate branches and bounded loops |
| 3 Differential | `c0887ef86` classify static and operational disagreements |
| 3 Source replay | `99ab43de2` replay real compiled sources through the oracle |
| 4 Generate and reduce | `dc082a42e` generate, reduce and prove oracle problems |
| 5 Role transitions | `42ecaf8c2` retire holders when an alias state is replaced |
| 5 Destination roles | `ba19b59d9` install destination roles from established state |
| 5 Provenance | `51aaf9ca4` issue the provenance capabilities the reference derives |
| 5 Call boundary | `697336b77` end a call argument capability at its own effect |
| 5 Shape refusals | `7525595a4` refuse shapes the runtime graph cannot represent |

### Deferred from package 2

These are explicit non-goals of the completed package. Do not reopen them while implementing package 3 unless a new required finding depends on them.

* Per-holder capability retirement. A `Loan` row naming several distinct holders is refused as `MultiHolderLoan` because the static solver applies a row's uses and kills capability-wide, so there is no reference semantics to mirror. No producer emits one.
* Storage domains holding several distinct nodes. A repeated projection resolving to two distinct nodes is refused as `RepeatedProjectionChild`. The reference unions the repeated origins into one projected slot and the runtime graph holds one node per position, so modelling the union belongs to package 6.
* Loop generation widening. The oracle enumerates a bounded number of iterations and truncates; iteration-sensitive generations belong to package 4.
* Call summaries. `CallEffect` is opaque and a call argument capability ends at its own effect. Callee internals belong to package 5.
* Production Stage 6 does not consume the oracle. It remains a developer-facing laboratory behind the `boracle` feature.
* Correcting the reference solver's shared-alias acceptance. The oracle found a real soundness gap and it is recorded under `### Reference gaps the oracle has found` in `boracle-reference-solver.mtf`. Fixing it changes which programs the compiler accepts, so it needs its own plan rather than a change inside an oracle package.

### Deferred from package 1

These are explicit non-goals of the completed package. Do not reopen them while implementing the oracle unless a new required finding depends on them.

* CFG join of empty union nonempty argument origin state still unions to the nonempty set. `merge_state` widening belongs to package 4, loop generation epochs.
* Dynamic-index sibling overlap is unused in current producers. `PrecisionLossReason::DynamicIndex` exists but is not emitted. That belongs to package 6, or to refinement if a producer starts emitting precise dynamic-index origins.
* `ProvenDisjoint` rows against unknown registrations are not rejected at construction. No current emitter produces that shape.
* Production Stage 6 remaining on storage-root overlap is intentional, not unfinished Boracle work.

### Future checked proof-budget proposal

The open proposal is a lexical proof-budget scope, currently called a checked block, that may let the compiler spend substantially more deterministic analysis effort on difficult borrow proofs.

The proposal is not accepted syntax and does not belong in the progress matrix. It must preserve:

```text
same safety rules
same runtime meaning
ordinary fast analysis first
deeper conflict-directed proof only when needed
```

The remaining packages should record whether a capability is suitable for:

```text
coarse analysis
normal conflict refinement
potential future deep refinement
```

They must not add checked syntax, wall-clock timeouts or a public solver switch. The shared integration requirements live in `checked-proof-budget-integration.md`.

The plans preserve the accepted compiler boundary: borrow validation consumes validated HIR, writes facts without rewriting HIR and does not decide lifetime topology, retained edges or physical memory strategy.

They also preserve the build-system boundary. Module scheduling and publication remain build-owned while normalized problem construction, reference solving, summary convergence and generated semantic work remain compiler-owned.

## Why this order

### 1. Typed provenance before more permissive experiments

Every later package needs a precise vocabulary for:

```text
same generation
may alias
must be disjoint
contains projected child
copied from
unknown because ...
```

That vocabulary now exists as `OriginRelations`. Later packages must use it rather than adding exceptions around a relatedness set.

### 2. An independent oracle before ambitious acceptance changes

Package 2 implemented the independent oracle. It executes small normalized problems using dynamic
generations and capabilities. It deliberately does not reuse Boracle's fixed-point or overlap
algorithm.

This gives the research process two useful results:

```text
static accepted + runtime conflict found
    -> likely soundness defect

all bounded executions safe + static rejected
    -> concrete precision opportunity
```

Bounded execution is not a proof for unbounded loops. The plan treats truncated exploration as inconclusive and uses the oracle for counterexamples, regression evidence and reduction.

The oracle may validate future deep-refinement acceptance deltas. It can never become the acceptance proof for a checked scope.

### 3. Conflict-directed refinement as the central production direction

This is the most important ambitious package.

The intended model is:

```text
cheap may-analysis
    |
    +-- no candidate conflict -> accept
    |
    v
candidate conflict slice
    |
    v
pairwise must-alias / must-disjoint refinement
    |
    v
bounded compatible state alternatives
    |
    +-- conflict disproved on every alternative -> accept
    |
    v
confirmed conflict with one path-compatible witness
```

This can retain a fast common path in the future production checker while using stronger proof only where conservative merging creates a candidate error.

It is also the main foundation for a possible checked tier. Refinement should record the effort needed to discharge a conflict and keep room for separate normal and deep deterministic state limits.

### 4. Loops after acyclic path refinement

Loop epochs add another correlation dimension. Starting them before the acyclic relation and refinement machinery is stable would mix two hard problems.

The loop package starts with a deliberately small domain:

```text
Current
Prior
UnknownMany
```

It also adds edge-specific last-use queries and honest witnesses for infinite no-use continuations.

A future checked tier may permit delayed widening or richer loop invariants, but bounded unrolling alone never proves unbounded safety.

### 5. Interprocedural work after local proof machinery

Per-result provenance, outcome-sensitive effects and recursive SCC solving all depend on a stable local relation model.

Deferred exclusive activation is kept as a named experiment:

```text
argument evaluation
    -> ReservedExclusive

call effect starts
    -> ActiveExclusive
```

It does not inspect callee internals or change ordinary mutable alias rules.

A future checked caller may authorise more context-sensitive summary specialisation. Separate compilation and stable semantic summaries remain mandatory.

### 6. Aggregate and builtin storage semantics last

This package then uses the established relation, oracle, refinement and call-result vocabulary to model:

* Distinct tuple and fixed-collection indexes
* Deep copies that preserve internal sharing
* Source/result graph independence
* `get`, `set`, `remove` and `clear`
* Preliminary detached-result provenance

It explicitly stops before allocation families, retained-edge cardinality, cleanup frontiers and REC selection.

Dynamic indexes remain conservative in the first package. A later deep-analysis experiment may investigate narrow index inequality facts without making general index solving part of ordinary borrow validation.
