# Recommended research sequence

## Current handoff

Package 1 is complete. The next agent should implement package 2, **Bounded operational oracle**, from `boracle-bounded-operational-oracle-plan.md` in this directory.

That plan is still `proposed`. Activate it before editing: copy it to `docs/roadmap/plans/boracle-bounded-operational-oracle-plan.md`, add a roadmap entry under Active implementation work, set its status block to `active`, and treat that copied file as the work source. Do not treat this README as the work source.

The package 1 blocker on that plan is cleared: typed provenance overlap evidence, explicit unknown reasons, deterministic reports, and composable experiment selection are in the `boracle` branch. The remaining first action in the oracle plan is to establish the current semantic corpus baseline and specify the smallest executable normalized semantics.

The completed package 1 plan was deleted from `docs/roadmap/plans/` with its roadmap entry. The copy in this directory is historical only.

| Order | Package | Status | Main result |
| ----: | --------------------------------------------- | ---------- | -------------------------------------------------------------------------------------------- |
| 1 | Typed provenance relations | complete | Replaces the overloaded origin relation and establishes composable experiments |
| 2 | Bounded operational oracle | next | Provides an independent executable check against unsound experimental acceptance |
| 3 | Conflict-directed relational refinement | queued | Implements the core coarse-solve then targeted-refinement architecture |
| 4 | Loop generation epochs | queued | Adds iteration-sensitive generations and edge-specific last-use reasoning |
| 5 | Call summaries and deferred exclusive access | queued | Explores per-result, outcome-sensitive, recursive summaries and reserved exclusive arguments |
| 6 | Aggregate copy and builtin storage provenance | queued | Models deep-copy topology, fixed projection precision and compiler-known storage effects |

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

### Deferred from package 1

These are explicit non-goals of the completed package. Do not reopen them while implementing the oracle unless a new required finding depends on them.

* CFG join of empty ∪ nonempty argument origin state still unions to the nonempty set. `merge_state` widening belongs to package 4, loop generation epochs.
* Dynamic-index sibling overlap is unused in current producers. `PrecisionLossReason::DynamicIndex` exists but is not emitted. That belongs to package 6, or to refinement if a producer starts emitting precise dynamic-index origins.
* `ProvenDisjoint` rows against unknown registrations are not rejected at construction. No current emitter produces that shape.
* Production Stage 6 remaining on storage-root overlap is intentional, not unfinished Boracle work.

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

This is the next package. The oracle executes small normalized problems using dynamic generations and capabilities. It deliberately does not reuse Boracle's fixed-point or overlap algorithm.

This gives the research process two useful results:

```text
static accepted + runtime conflict found
    -> likely soundness defect

all bounded executions safe + static rejected
    -> concrete precision opportunity
```

Bounded execution is not a proof for unbounded loops. The plan treats truncated exploration as inconclusive and uses the oracle for counterexamples, regression evidence and reduction.

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

### 4. Loops after acyclic path refinement

Loop epochs add another correlation dimension. Starting them before the acyclic relation and refinement machinery is stable would mix two hard problems.

The loop package starts with a deliberately small domain:

```text
Current
Prior
UnknownMany
```

It also adds edge-specific last-use queries and honest witnesses for infinite no-use continuations.

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

### 6. Aggregate and builtin storage semantics last

This package then uses the established relation, oracle, refinement and call-result vocabulary to model:

* Distinct tuple and fixed-collection indexes
* Deep copies that preserve internal sharing
* Source/result graph independence
* `get`, `set`, `remove` and `clear`
* Preliminary detached-result provenance

It explicitly stops before allocation families, retained-edge cardinality, cleanup frontiers and REC selection.
