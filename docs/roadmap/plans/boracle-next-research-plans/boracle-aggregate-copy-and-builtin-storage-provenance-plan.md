# Boracle aggregate copy and builtin storage provenance plan

Status: proposed

Current slice: not started

Blockers: typed provenance relations and the bounded operational oracle must be complete; per-result call provenance should be available before storage operations return detached values

Next action: activate with an inventory of aggregate projection kinds, builtin collection and map effects and current copy-graph assumptions

Repository path:

```text
docs/roadmap/plans/boracle-aggregate-copy-and-builtin-storage-provenance-plan.md
```

Canonical authorities:

- `docs/compiler-design-overview.md`
- `docs/src/developer-docs/memory-management/overview.mtf`
- `docs/src/developer-docs/memory-management/access-and-aliasing/access-and-aliasing.mtf`
- `docs/src/developer-docs/memory-management/borrow-validation/borrow-validation.mtf`
- `docs/src/developer-docs/memory-management/boracle/boracle-reference-solver.mtf`
- `docs/src/developer-docs/memory-management/retained-edge-counting/overview.mtf`
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`

## Purpose

Strengthen Boracle's aggregate, copy and builtin storage provenance without turning borrow validation into lifetime topology.

The current model already distinguishes a fresh outer aggregate from aliased child origins. It also treats `copy` as an independent result. The next useful step is to preserve more internal graph structure and to model compiler-known builtin storage operations directly.

This package investigates:

- exact fixed-field and fixed-index disjointness
- deep copy graph correspondence
- repeated internal aliases
- collection and map storage domains
- `get`, `set`, `remove` and `clear` borrow effects
- detached result provenance at the borrow boundary

The later lifetime system still owns retained edges, cardinality, cleanup frontiers, region epochs, groups and REC.

## Prerequisites

The active repository must already provide:

- typed provenance relationships
- positive disjointness
- bounded operational graph copying
- per-result preliminary provenance for calls that return values
- exact call argument and result events
- conservative collection and map place overlap
- explicit borrow/lifetime ownership boundaries

## Locked decisions

1. Borrow-place precision is independent from allocation-family splitting.
2. A fresh aggregate outer origin does not make existing child values fresh.
3. Deep copy preserves internal alias topology while making source and result graphs independent.
4. Copyability remains an AST and type-checking decision.
5. Copy destination ownership and copied-cycle placement remain lifetime-analysis decisions.
6. Dynamic collection indexes and map entries remain conservative in the first slice.
7. Distinct fixed struct fields are disjoint.
8. Distinct tuple fields and fixed collection indexes may be proven disjoint only after their semantic categories are explicit.
9. Builtin storage effects are compiler-known normalized facts. Boracle must not infer them from method names or rendered call labels.
10. `get` creates a temporary shared alias. It is not a retained edge or REC count.
11. `set`, structural mutation and `clear` require exclusive receiver access.
12. `remove` may produce preliminary detached-result provenance, but lifetime ownership is decided later.
13. No uniqueness scans, alias registries or runtime borrow tracking are added.
14. This plan is deleted with its roadmap entry in the completion commit.

## Target architecture

Refine projection categories where current HIR and types can prove the distinction:

```rust
pub(crate) enum ProjectionElem {
    Field(u32),
    TupleIndex(u32),
    FixedCollectionIndex(u32),
    DynamicIndex,
    CollectionElement,
    MapEntry,
    VariantPayload(u32),
}
```

Exact variants may differ. Do not split categories that HIR cannot identify reliably.

Copy graph concepts:

```rust
pub(crate) struct CopyGraphId(u32);

pub(crate) struct CopyCorrespondence {
    pub(crate) graph: CopyGraphId,
    pub(crate) source: ValueOriginId,
    pub(crate) result: ValueOriginId,
}
```

Storage effects:

```rust
pub(crate) enum StorageEffect {
    ObserveStored {
        receiver: PlaceId,
        result: PlaceId,
        domain: StorageDomain,
    },
    InsertStored {
        receiver: PlaceId,
        value: PlaceId,
        domain: StorageDomain,
    },
    ReplaceStored {
        receiver: PlaceId,
        old_result: Option<PlaceId>,
        value: PlaceId,
        domain: StorageDomain,
    },
    DetachStored {
        receiver: PlaceId,
        result: PlaceId,
        domain: StorageDomain,
    },
    KillDomain {
        receiver: PlaceId,
        domain: StorageDomain,
    },
}
```

These are borrow/provenance effects only.

## In scope

- projection-category audit
- fixed aggregate disjointness
- deep copy graph correspondence
- repeated child aliases
- nested aggregate provenance
- compiler-known storage effects
- temporary storage-domain loans
- preliminary detached result provenance
- source and normalized tests
- operational-oracle comparison
- canonical docs and promotion review

## Out of scope

- dynamic index arithmetic
- map key uniqueness proofs
- retained-edge counts
- cleanup frontiers
- REC selection
- region ownership
- explicit group placement
- backend collection layout
- user-defined storage effects
- custom collection protocols
- source annotations

## Phase 0: inventory projection and storage semantics

### Summary and reasoning

The normalized vocabulary should follow semantic categories already known by HIR, not guess from syntax.

### Work

- [ ] Re-anchor the branch and record the working baseline in untracked notes.
- [ ] Inventory every producer and consumer of `ProjectionElem`.
- [ ] Map HIR shapes for:
  - [ ] struct field
  - [ ] tuple field
  - [ ] choice payload
  - [ ] fixed collection index
  - [ ] growable collection index
  - [ ] dynamic index
  - [ ] map entry
- [ ] Inventory current aggregate child events.
- [ ] Inventory copy origin construction and every place that assumes one fresh outer origin is sufficient.
- [ ] Inventory builtin collection and map effect metadata.
- [ ] Identify operations currently lowered as generic calls.
- [ ] Classify each operation's borrow effect and later lifetime effect separately.
- [ ] Add reduced regressions for:
  - [ ] repeated child alias
  - [ ] nested aggregate
  - [ ] copy of repeated child
  - [ ] fixed sibling access
  - [ ] `get` then mutation
  - [ ] `remove` result
  - [ ] `clear` with temporary alias
- [ ] Record current conservative results.

### Phase gate

- [ ] Audit the inventory against access, lifetime and REC authorities.
- [ ] Review that no physical layout assumption entered the normalized model.
- [ ] Run `just boracle`.
- [ ] Commit the inventory and corpus.

## Phase 1: make fixed projection categories explicit

### Summary and reasoning

Different fixed fields and indexes can be disjoint without proving anything about physical allocation families.

### Work

- [ ] Split projection categories only where semantic type information makes the distinction stable.
- [ ] Update place interning and validation.
- [ ] Update structural overlap:
  - [ ] different struct fields are disjoint
  - [ ] different tuple fields are disjoint
  - [ ] different fixed collection indexes are disjoint
  - [ ] base versus child overlaps
  - [ ] dynamic indexes remain conservative
  - [ ] growable structural mutation overlaps every element
  - [ ] map entries remain conservative
- [ ] Retain the reason in `PlaceOverlap` evidence.
- [ ] Update Boracle relation evidence.
- [ ] Keep lifetime-family facts unchanged.
- [ ] Add HIR extraction tests so categories cannot be assigned from source spelling alone.

### Required tests

- [ ] two struct fields
- [ ] nested fields
- [ ] two tuple indexes
- [ ] base and tuple index
- [ ] two fixed collection indexes
- [ ] fixed versus dynamic index
- [ ] growable element versus push
- [ ] map entry versus set
- [ ] variant payload categories

### Phase gate

- [ ] Audit each disjointness rule against operation shape.
- [ ] Review HIR and problem ownership.
- [ ] Run focused place tests.
- [ ] Run `just boracle`.
- [ ] Run `just validate`.
- [ ] Commit projection precision.

## Phase 2: preserve deep copy graph correspondence

### Summary and reasoning

A copy must preserve internal sharing.

Example:

```text
source.left  -> child A
source.right -> child A

copy.left    -> copied child B
copy.right   -> copied child B

A and B are disjoint
```

Creating two unrelated copied children would change source semantics.

### Work

- [ ] Add a copy-graph identity for one copy operation.
- [ ] Build a source-to-result origin mapping.
- [ ] Reuse one copied result origin for repeated source child identity.
- [ ] Preserve nested aggregate shape.
- [ ] Record positive source/result disjointness.
- [ ] Keep cycles as graph facts only. Do not decide group placement.
- [ ] Add copy graph traces and dumps.
- [ ] Update the operational oracle to use the same semantic contract through its independent runtime implementation.
- [ ] Add validation for duplicate or inconsistent correspondence.
- [ ] Keep scalar and opaque copy cases simple.

### Required tests

- [ ] repeated child alias preserved
- [ ] two independent children remain independent
- [ ] nested repeated child
- [ ] projection from copied graph
- [ ] copy in branch join
- [ ] copy in loop
- [ ] copied cycle represented but not lifetime-legalised
- [ ] deterministic graph IDs

### Phase gate

- [ ] Audit graph correspondence and independence.
- [ ] Audit that copyability and lifetime placement remain outside Boracle.
- [ ] Compare every graph case with the operational oracle.
- [ ] Run `just boracle`.
- [ ] Commit deep copy provenance.

## Phase 3: add compiler-known storage effects

### Summary and reasoning

Builtin storage operations should not be approximated only through generic parameter access and call-result alias unions.

### Work

- [ ] Add normalized compiler-known storage-effect events or call metadata.
- [ ] Emit them from the HIR problem builder using semantic builtin operation identity.
- [ ] Model:
  - [ ] `get` as a temporary shared alias into the receiver storage domain
  - [ ] `set` as exclusive receiver access plus stored-value replacement
  - [ ] `remove` as exclusive receiver access plus detached preliminary result
  - [ ] `clear` as exclusive receiver access plus whole-domain borrow kill
  - [ ] collection structural mutation as exclusive receiver access
- [ ] Preserve ordinary access for keys, values and index expressions.
- [ ] Keep `get` loans temporary and uncounted.
- [ ] Keep storage-domain provenance conservative where the exact entry is dynamic.
- [ ] Add exact result event boundaries.
- [ ] Do not add retained-edge cardinality or cleanup-frontier facts.
- [ ] Validate that generic call summaries cannot contradict builtin effects.

### Required tests

- [ ] `get` then receiver mutation conflicts while result is live
- [ ] final `get` use permits later mutation
- [ ] `set` replaces stored provenance
- [ ] `remove` result no longer aliases the receiver domain under accepted preliminary rules
- [ ] `clear` kills storage-domain borrow capabilities after the event
- [ ] index expression read before mutable receiver access
- [ ] map and collection domains remain distinct
- [ ] no REC row is created

### Phase gate

- [ ] Audit every builtin against canonical language semantics.
- [ ] Audit borrow effects versus lifetime effects.
- [ ] Review problem-builder ownership and no name-based inference.
- [ ] Run source and normalized storage tests.
- [ ] Run `just boracle`.
- [ ] Run `just validate`.
- [ ] Commit storage effects.

## Phase 4: refine detached result and domain witnesses

### Summary and reasoning

Detached results are important to future lifetime summaries, but Boracle should publish only preliminary access/provenance evidence.

### Work

- [ ] Add a preliminary detached-result relationship:
  - [ ] result came from storage domain
  - [ ] result is no longer observed through the receiver entry after detach
  - [ ] complete ownership remains unknown to Boracle
- [ ] Keep result provenance separate per result slot.
- [ ] Add conflict witnesses that identify:
  - [ ] storage receiver
  - [ ] domain
  - [ ] operation
  - [ ] result holder
  - [ ] keeping use
- [ ] Add precision-loss reasons for dynamic entries and unknown domain contents.
- [ ] Add generated properties:
  - [ ] replacing `get` with `copy get` cannot add receiver alias conflicts
  - [ ] `remove` cannot leave the same temporary storage loan live
  - [ ] `clear` cannot kill an unrelated local alias
  - [ ] fixed aggregate disjointness survives branch splitting
- [ ] Compare bounded cases with the oracle.

### Phase gate

- [ ] Audit detached result wording against lifetime ownership.
- [ ] Review witness completeness and path compatibility.
- [ ] Run `just boracle`.
- [ ] Commit detached-result evidence.

## Phase 5: promotion review, documentation and closeout

### Summary and reasoning

Finish with explicit decisions about fixed-place precision, copy graph semantics and builtin effects.

### Work

- [ ] Produce a promotion report for:
  - [ ] tuple and fixed-index disjointness
  - [ ] copy graph correspondence
  - [ ] storage-effect vocabulary
  - [ ] detached preliminary results
- [ ] Promote rules already implied by canonical source semantics.
- [ ] Keep uncertain storage-domain rules as named experiments.
- [ ] Update Boracle, access-and-aliasing and borrow-validation docs.
- [ ] Update the progress matrix only for current compiler behaviour.
- [ ] Do not document REC or lifetime ownership as Boracle output.
- [ ] Run final scoped audits:
  - [ ] fixed projection soundness
  - [ ] copy graph topology preservation
  - [ ] storage operation semantics
  - [ ] borrow/lifetime/REC boundary
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

- fixed semantic projections carry explicit categories and sound overlap rules
- deep copy preserves internal sharing and source/result independence
- builtin storage operations emit compiler-known borrow effects
- temporary `get` aliases never become REC obligations
- detached results remain preliminary borrow facts
- no lifetime owner or physical strategy is selected by Boracle
- promoted rules and experiments are documented accurately
- the plan and roadmap entry are removed in the completion commit
