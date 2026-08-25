# Retained Edge Counting design and implementation plan

Status: accepted final design; the multi-edge obligation algebra, direct-edge resolution and target/profile physical-planning consistency closure are complete. REC implementation remains deferred.

Repository path:

```text
docs/roadmap/plans/retained-edge-counting-design-and-implementation-plan.md
```

Canonical implementation authority:

```text
docs/src/developer-docs/memory-management/retained-edge-counting/retained-edge-counting.mtf
```

Companion to:

- the parent plan `docs/roadmap/plans/final-memory-management-redesign-and-implementation-plan.md`
- `docs/src/developer-docs/memory-management/overview.mtf`
- `docs/src/developer-docs/memory-management/ownership-and-drops/`
- `docs/src/developer-docs/memory-management/lifetime-regions-and-escape-validation/`
- `docs/src/developer-docs/memory-management/declared-memory-groups/`
- `docs/src/developer-docs/memory-management/runtime-and-backend-lowering/`

The canonical REC page now exists. REC implementation remains deferred and follows the phases in this plan.

This plan owns the detailed compiler and runtime contract for Retained Edge Counting, abbreviated REC. The main memory-management plan should explain when REC exists and link here. It should not duplicate the implementation details in this document.

## Purpose

Moth statically validates one legal lifetime topology for every accepted allocation. Full-memory-control release backends lower that topology without a tracing collector. Last-use analysis, affine cleanup responsibility and inferred regions handle most values without runtime alias counting.

REC covers the remaining narrow case:

> A runtime-dependent number of persistent retained edges can point to one allocation family, those edges can disappear independently and the narrowest statically proven region would otherwise retain substantial storage materially beyond its useful lifetime.

REC is not general reference counting. It does not count ordinary aliases. It does not establish lifetime legality. It does not permit cycles. It is a selective physical representation chosen only after the compiler has exhausted static and region-based answers.

## Final decision summary

The following decisions are locked for the final design.

1. REC is a generic allocation-family strategy, not a source type and not a collection type feature.
2. Builtin collections are the primary source and first implementation boundary for REC because they are Moth's opinionated dynamic-storage substrate.
3. User-defined collection abstractions compose builtin collection operations. The compiler infers and exports equivalent retained-edge summaries without source annotations.
4. Local aliases, parameters, temporary projections, ordinary calls and `get()` borrows are never counted.
5. Persistent edges with statically known cardinality remain under affine and region analysis when possible.
6. Dynamic retained multiplicity makes an allocation eligible for REC. It does not force REC.
7. Final cleanup frontiers, inferred region epochs, explicit groups, field-sensitive splitting and statically cheap retention may elide REC.
8. Explicit-group-owned allocations never use REC. Cycles require explicit groups.
9. REC strategy is selected statically. Runtime adaptive switching between REC and region ownership is rejected.
10. Profiling-guided REC strategy selection is out of scope for the initial design and implementation.
11. Full-control backends use a two-bit tagged allocation-family handle for heap values that participate in the memory ABI.
12. Bit 0 carries affine cleanup responsibility: owned or borrowed.
13. Bit 1 identifies the physical retention representation: REC counted or uncounted.
14. REC-managed allocation families carry one inline target-word-sized counter. Other allocation families carry no REC counter.
15. The counter is non-atomic under the accepted channel and task model.
16. Counted destruction is iterative through a deletion worklist rather than recursive through the native or Wasm stack.
17. REC can cross Moth function and package boundaries, but only retention-sensitive operations need to inspect the REC bit.
18. Public semantic interfaces describe aliasing, retention, detached stored results, cardinality, whole-domain kills and outcome-sensitive cleanup effects. They never expose REC as source semantics.
19. Ordinary foreign boundaries remain closed and value-only. REC does not create a cross-language shared-reference protocol.
20. The first REC implementation includes structured developer reporting for every considered allocation, including exact elision reasons.

## Relationship to the final memory model

The final Moth memory system has six cooperating parts.

### Borrow and last-use analysis

This proves:

- current shared and exclusive access legality
- path-sensitive alias liveness
- the last potential use of local aliases
- safe affine cleanup-responsibility transfer points
- whether a call receives an owned or borrowed handle

### Lifetime topology

This proves:

- one semantic lifetime owner for every allocation family
- retained-edge outlives legality
- legal escapes
- compiler-generated non-lexical lifetime intervals
- retained-edge liveness and final cleanup frontiers
- legal builder lifecycle roots

### Affine Ownership ABI

This carries the runtime path that may perform individual cleanup when control flow prevents complete static specialization.

Cleanup responsibility is affine. It may move or be discharged. It never duplicates.

### Explicit groups

A declared group is one hard lifetime and cleanup domain.

- group-owned values are not individually released early
- group-owned values do not use REC
- cycles are legal only inside one explicit group
- group exit performs bulk reclamation

### Retained Edge Counting

REC counts only runtime-dependent persistent retained edges that survive the static and region elision process.

### Physical memory planning

The compiler-owned memory planner chooses stack placement, static drops, individual heap allocations, inferred arenas, explicit-group arenas and REC layouts for a topology already proven legal. It runs after build-owned target partition and target-contract validation, once per candidate physical variant, and the backend only realises the resulting `ValidatedMemoryPlan`. The backend never chooses a memory strategy.

REC is therefore a precision mechanism inside a statically safe collector-free system. It is not the correctness fallback.

## Terminology

### Allocation family

The semantic allocation root that must be kept alive as one unit under the current field-splitting result. A projection may point inside an allocation family while cleanup still targets the family root.

### Retained edge

A persistent relationship where one aggregate or runtime domain stores a reference that can outlive the operation that created it.

A temporary local alias or call borrow is not a retained edge.

### Retention domain

A place or aggregate family that can store one or more retained edges, such as a collection, map, struct field set, choice payload or future recursive aggregate.

### Dynamic retained multiplicity

Runtime control flow determines how many persistent retained edges to an allocation family exist. Static analysis cannot resolve the exact cardinality.

### Final cleanup frontier

A CFG point after which static analysis proves that a retention domain cannot contain any surviving retained edge into the relevant allocation family or region.

A collection may continue to live after its retained-edge frontier.

### REC candidate

An allocation family with dynamic retained multiplicity that may benefit from counted early reclamation.

### REC-selected allocation

A candidate for which the final memory planner chooses the REC physical strategy after all elision passes.

### Affine root obligation

The one optional counted obligation represented by a runtime handle carrying affine cleanup responsibility.

### Persistent-edge obligation

One counted obligation represented by one runtime persistent retained edge.

## Scope

### In scope

- backend-neutral dynamic-retention facts
- retained-edge cardinality and liveness
- cleanup-frontier inference
- REC strategy selection and elision
- two-bit full-control handle ABI
- inline REC counter layout
- count transitions
- builtin collection integration
- compositional user-function and package summaries
- detached stored results
- transitive aggregate retention
- iterative destruction
- mixed region and REC cleanup
- developer tracing and validation
- release-backend verification

### Out of scope

- source-visible RC or REC types
- retain and release syntax
- weak references
- cycle detection or cycle collection
- atomic or cross-task shared REC
- runtime adaptive REC-to-region switching
- runtime region-pressure strategy changes
- REC profiling infrastructure
- profile-guided strategy selection
- user-facing `config.moth` REC thresholds
- uniqueness scans on collection removal
- alias registries
- `group self`
- a `checked` block or expensive-analysis source mode
- foreign code retaining ordinary Moth references
- unsafe pointers or allocator APIs

## Core invariants

These invariants must hold in every implementation.

1. Lifetime topology is validated before REC strategy selection.
2. REC never makes an otherwise illegal retained edge legal.
3. Every REC allocation retains its statically proven fallback lifetime region.
4. REC only permits earlier individual reclamation than that region frontier.
5. Cleanup responsibility remains affine even when REC is active.
6. There is at most one affine root obligation for an allocation family.
7. Each counted persistent retained edge contributes exactly one obligation.
8. Ordinary local aliases contribute no obligation.
9. Temporary `get()` aliases contribute no obligation.
10. Borrow validation prevents the final counted obligation from disappearing while an uncounted temporary alias remains usable.
11. REC graphs are acyclic. Cycles require explicit groups.
12. Explicit-group-owned allocations carry no REC counter.
13. REC state never changes an allocation's source type or public semantic identity.
14. A strategy selected as uncounted is never upgraded to REC at runtime.
15. A strategy selected as REC never abandons counting at runtime.
16. Count zero means no affine root obligation and no counted persistent-edge obligation remain.
17. Count zero triggers deterministic destruction exactly once.
18. Counter underflow is a compiler or runtime invariant failure and must trap rather than cause undefined behavior.
19. Full-control release output may not silently fall back to tracing GC.
20. GC-backed debug or GC-native backends preserve the same accepted programs and observable source behavior.

## Why REC exists

Consider one heavy allocation stored under a runtime-dependent number of keys.

```moth
blob = load_blob()!

loop aliases |name|:
    ~index.set(name, blob)!
;

loop eviction_keys |key|:
    ~index.remove(key) catch:
    ;
;
```

Runtime execution may create this graph:

```text
index["a"] ----+
index["b"] ----+
index["c"] ----+----> blob
index["d"] ----+
```

Last-use analysis can resolve local aliases. Retained-edge analysis can prove a later `clear()` frontier. Neither can generally know which runtime `remove(key)` destroys the final retained edge.

Without REC, the safe answer is to keep `blob` until the narrowest proven region frontier. That remains memory-safe and collector-free, but a page-lifetime or session-lifetime region may retain large dead allocations for too long.

REC supplies only the missing runtime fact: how many unresolved persistent retained-edge obligations remain.

## REC is not tied to loops, recursion or collections

Loops and recursion are common producers of runtime-many retention, but they are not the semantic trigger.

This loop needs no REC:

```moth
loop items |item|:
    use(item)
;
```

Its aliases are temporary and statically bounded.

Repeated events can create REC pressure without a visible loop:

```moth
add_alias |index ~{String = Blob}, key String, blob Blob| -> Error!:
    ~index.set(key, blob)!
;
```

Ordinary fields can also create runtime-dependent retained multiplicity:

```text
state.primary -> blob
state.backup  -> blob
```

Transitive aggregates can create it indirectly:

```text
holders[n] -> Holder -> blob
```

The architecture is generic over retained edges. The first implementation remains collection-first because builtin collections are the dominant and most constrained mutation boundary.

## Compiler pipeline

REC planning occurs after source legality and complete topology validation.

```text
validated HIR
    |
    v
borrow and last-use analysis
    |
    +-- local alias liveness
    +-- affine transfer facts
    +-- access legality
    |
    v
local retained-edge analysis
    |
    +-- allocation families
    +-- result provenance
    +-- retained parameters and receiver fields
    +-- edge creation and destruction
    +-- cardinality constraints
    +-- cleanup-frontier candidates
    |
    v
exported lifetime and retention summaries
    |
    v
project and package summary instantiation
    |
    v
complete backend-neutral lifetime-topology validation
    |
    v
non-lexical interval, frontier and epoch completion
    |
    v
backend-neutral memory requirements
    |
    +-- REC candidacy and cardinality facts
    +-- no selected REC representation
    |
    v
target-affinity analysis and partition        (build-owned)
    |
    v
target-contract validation                    (build-owned roots)
    |
    v
candidate physical variant scope
    |
    v
per-variant field-sensitive family/layout refinement
    |
    v
revalidate affected refined family-edge facts
    |
    v
target/profile-aware memory planning          (compiler-owned)
    |
    +-- affine static cleanup
    +-- inferred region
    +-- explicit group
    +-- REC
    |
    v
ValidatedMemoryPlan (one per physical variant)
    |
    v
backend lowering and collector-free verification
```

REC candidacy, cardinality and edge-effect summaries are backend-neutral and precede target partition. **Selected** REC representation does not: it is chosen inside target/profile-aware memory planning, after target partition and target-contract validation, and is therefore scoped to one physical variant. The same source function may be REC-capable in one variant and GC-native in another without any change to its public semantic interface.

The memory planner is compiler-owned. The build system owns target partition, physical-variant orchestration, the build profile and target/backend capability metadata. The backend lowerer realises the finished plan and never chooses a memory strategy.

REC facts must not be inserted into HIR as source semantics. They belong in immutable side tables, exported summaries and the final memory plan.

## Backend-neutral analysis vocabulary

Exact Rust names may change. The following conceptual distinctions may not collapse.

```rust
pub enum RetentionCardinality {
    None,
    One,
    Fixed(u32),
    RuntimeMany,
}

pub enum ResultProvenance {
    Fresh,
    AliasParameter(u32),
    ProjectionOfParameter(u32),
    DetachedStoredValue(u32),
    AliasResult(u32),
    Independent,
}

pub enum RetentionKill {
    None,
    OneEdge,
    WholeDomain,
}

pub enum MemoryStrategy {
    Affine,
    InferredRegion,
    ExplicitGroup,
    Rec,
}
```

`Fixed` does not automatically require REC. A statically bounded edge set can often remain affine or region-owned.

`RuntimeMany` means REC-eligible, not REC-selected.

## Public function and package summaries

Public semantic summaries remain source-level and backend-neutral.

They may need to express:

- parameter access mode
- optional affine transfer eligibility
- returned alias or projection relationships
- detached stored results
- result-to-result aliasing
- parameters or receiver domains retained after return
- retention cardinality, including runtime-many creation
- persistent-retention effects, including exit-specific success and error paths
- complete retained-edge domain destruction
- outlives constraints
- external boundary classification

They must not express:

- `REC<T>`
- counter layout
- pointer-tag encoding
- selected physical strategy
- backend helper names

A package consumer instantiates these summaries into its project topology. Strategy selection occurs after linking against concrete lifecycles and call roots, and after build-owned target partition and target-contract validation. REC strategy is selected independently per physical variant, so public semantic interfaces stay REC-free and identical across variants.

## Builtin collection retention vocabulary

Builtin fixed and growable collections are the trusted dynamic-storage substrate.

Their memory effects are compiler-known. Each stored value contributes a complete retained-edge summary. A scalar such as `Int` may contribute zero obligations, a direct heap-backed value may contribute one, and an inline aggregate that physically stores several handles may contribute several direct obligations. A summary may also describe nested retention so the compiler knows what a value structurally contains.

**Nested or transitive retention summaries are analysis descriptions. REC obligations count actual direct persistent edges between the final allocation families and retention domains. Reachability through a separately allocated child is never counted again.**

Given `collection -> Holder family -> Blob family`: if `Holder` is separately allocated, the direct graph is `collection -> Holder` and `Holder -> Blob`, and inserting a `Holder` adds no `collection -> Blob` obligation. If layout refinement places `Holder` inline in collection storage and that representation holds a `Blob` handle, the direct graph contains `collection storage -> Blob`, which does contribute an obligation.

Final counted obligations are therefore resolved after applicable field splitting and physical layout refinement, per physical variant. A storage operation adds or removes the complete set of direct obligations contributed by the stored value.

The table describes the successful semantic path. Fallible operations publish their effects only at their semantic commit point.

| Operation | Retained-edge effect |
|---|---|
| `get` | creates a statically bounded temporary alias. It adds no persistent obligation |
| `push` | adds the inserted element's retained-edge obligations |
| `set` | removes the replaced element's obligations and adds the new element's obligations |
| `remove` | removes the stored element's obligations and returns the existing value as a detached stored result |
| `clear` | removes the obligations contributed by every stored element |
| collection destruction | removes all element obligations and destroys the backing-storage domain |
| growth or reallocation | replaces backing storage while preserving logical element summaries |

The compiler does not recognize user methods by name.

For the initial direct-handle implementation, an element commonly contributes zero or one direct obligation. The semantic vocabulary remains general so aggregate and nested retention do not require a later redesign.

### Outcome-sensitive builtin commits

A builtin collection mutation commits its retained-edge effects atomically on the successful path. A failed operation preserves the original storage topology and every cleanup obligation. HIR represents these outcomes as explicit control-flow paths, so public summaries must preserve the distinction:

```text
success:
    remove old obligations
    add new obligations

error:
    retain the original obligations
```

Invalid-index `remove` destroys no obligation. A failed fixed-capacity `push` adds none. A failed `set` leaves the old element intact. A failed map insertion retains neither the incoming key nor the incoming value. Count changes happen after the operation reaches its semantic commit point.

A fallible operation receives inferred affine responsibility only when last-use analysis proves transfer safe across every relevant outcome. If a failure path still uses the incoming value, the operation receives a borrow. A failed operation commits no retained edge, so no ownership-return protocol exists or is required. This is ordinary all-path transfer proof.

### Map retention vocabulary

Maps retain both stored keys and stored values. Replacing an existing key keeps the stored key and changes only the stored value. The lookup key for an existing-key `set`, `get`, `contains` or `remove` call remains a temporary borrowed alias unless the operation inserts it as a new stored key.

| Operation | Retained-edge effect |
|---|---|
| `get` or `contains` | creates a temporary lookup alias and adds no persistent obligation |
| new-key `set` | adds the stored key's obligations and the new value's obligations |
| existing-key `set` | keeps the existing stored key, removes the old value's obligations and adds the new value's obligations. The incoming lookup key is not retained |
| `remove` | removes the stored key's and stored value's obligations. The value becomes a detached stored result, but the stored key does not |
| `clear` | removes all stored key and value obligations |
| map destruction | removes all stored key and value obligations and destroys the backing-storage domain |

Consider a map whose key and value point to the same allocation family:

```moth
text = [: large value]
values ~{String = String} = {}

~values.set(text, text)! -- final use of text
removed = ~values.remove("large value")!
```

```text
fresh text root:
    affine root = 1
    persistent edges = 0
    count = 1

after final-use new-key set:
    affine root = 0
    stored key edge = 1
    stored value edge = 1
    count = 2

after remove:
    stored key edge disappears
    stored value edge becomes returned affine root
    count = 1
```

The new-key insertion creates two direct persistent obligations to the same family, one from the stored key and one from the stored value. Because this was the final use of `text`, its affine root reclassifies into exactly one of the two edges; the second edge is a new obligation, so the count rises to `2`.

Removal drops the stored key obligation and reclassifies the stored value obligation into the returned result's affine root when the caller receives affine responsibility. The lookup string passed to `remove` is a temporary borrowed alias and does not add another obligation. If the mutation fails, neither stored obligation changes.

A user method gets a strong summary by composing builtin effects.

```moth
clear |this ~UserIndex|:
    ~this.values.clear()
    this.count = 0
;
```

If `values` is the only field retaining indexed elements, the compiler may infer that `UserIndex.clear` kills the whole element-retention domain.

If a user implementation clears slots through a handwritten loop, the function remains valid. It receives the strong summary only when analysis proves every relevant edge is killed on every path.

## Detached stored results

`remove` returns a detached stored result. It is not a fresh result and not an ordinary alias return.

Before container detachment:

```text
collection -> stored value
```

After container detachment:

```text
collection -X-> stored value
result       -> stored value
```

The existing allocation survives. The collection no longer retains it.

The summary vocabulary must preserve this distinction so the caller can:

- receive affine cleanup responsibility
- keep a borrowed alias under another owner
- reclassify the detached value's persistent obligations into the returned result

This applies to maps, collections, stacks, queues, deques, caches and other container detachment operations. Container detachment does not mean group extraction or adoption, which would move a group-owned allocation family outside its group. It also does not mean interior projection detachment, which would separate a field from its containing allocation family. Group extraction and adoption remain forbidden in V1. Interior projection detachment remains deferred until field-sensitive splitting has established separate ownership.

The restriction on extraction means moving a group-owned allocation out of its group or retroactively detaching an interior projection from its allocation family. It does not prohibit builtin collection or map `remove`, which kills a container-retained edge and returns the already-stored value under ordinary lifetime rules.

## REC elision and strategy selection

Strategy selection must be deterministic for one compile and backend configuration.

The planner uses the following order.

### 1. No dynamic retained multiplicity

Use affine cleanup or an inferred region. Do not emit REC.

### 2. Static fixed cardinality with a known cleanup path

Use affine or region handling where path-sensitive analysis can resolve the obligations. Do not emit REC merely because more than one edge may exist.

### 3. Explicit-group ownership

Use the hard group lifetime. Do not emit REC for group-owned allocations.

### 4. Complete final cleanup frontier

If all possible runtime-many retained edges disappear at one statically proven frontier, create an inferred region or region epoch ending there.

```moth
populate(~index)
use(index)
~index.clear()
```

The collection may continue living. If no other live alias or retention domain can retain a target allocation family, `clear()` can become that family's final cleanup frontier. The collection itself may continue to live. Group-owned storage still remains until group exit, even though `clear()` kills the group's collection edges logically.

### 5. Field-sensitive family splitting

Split an expensive parent allocation family when a small retained field can legally become independent, then rerun strategy selection for the new families.

Field-sensitive splitting is required in the final architecture but may land after the first REC implementation.

### 6. Statically cheap bounded retention

A compiler-owned heuristic may choose the fallback region when all of these are bounded enough:

- retained-family size
- allocation-site repetition
- maximum retained-edge cardinality where known
- fallback lifecycle breadth
- distance to a cleanup frontier

The initial implementation should keep this heuristic simple and conservative. It must not introduce a user-facing `config.moth` contract.

### 7. REC

Select REC when runtime-many persistent edges disappear independently and region retention is expected to be materially broader than useful lifetime.

## Cleanup frontiers and region epochs

Retained-edge liveness is part of lifetime inference.

A final cleanup frontier for family `A` requires proof that:

1. every possible retained edge into `A` from the relevant domains is gone on that path
2. no live local or projection alias survives the point
3. no future operation can recreate an edge because no capable source alias survives
4. no external or builder lifecycle retains `A`
5. every aliasing aggregate that remains live is proven unable to retain `A` after the point

A long-lived collection may create several inferred epochs.

```text
R_index_epoch_0
    population 0
    -> first clear

R_index_epoch_1
    population 1
    -> second clear
```

The collection object and its backing capacity may outlive each retained-value epoch.

## Two-bit tagged handle ABI

Full-memory-control backends use a two-bit tagged allocation-family handle for heap values that participate in ownership-aware lowering.

Logical bit assignment:

```text
bit 0: OWNED
bit 1: REC
```

Tag table:

| Bits | Meaning |
|---|---|
| `00` | uncounted borrowed handle |
| `01` | uncounted affine-owned handle |
| `10` | REC-managed borrowed handle or counted persistent edge |
| `11` | REC-managed affine-owned handle |

Bit 1 identifies the target allocation family's physical REC layout. It does not prove that this particular handle contributed one count. A temporary borrowed `get()` result may carry `10` while adding no obligation. The operation that stores or removes a persistent edge determines count changes.

Conceptual constants:

```rust
const OWNED_TAG: usize = 0b01;
const REC_TAG: usize = 0b10;
const TAG_MASK: usize = 0b11;
```

A backend masks both bits before recovering the allocation-family base.

```text
family_base = handle & !TAG_MASK
```

### ABI invariants

- Taggable family handles are aligned to at least four bytes.
- Bit 0 is per-handle cleanup responsibility.
- Bit 1 describes the target allocation family's physical REC representation.
- A counted persistent storage edge is canonically stored as `10`.
- An affine REC root is carried as `11`.
- A borrowed call from an owned REC root passes `10` while the caller retains `11`.
- An affine transfer moves `11` without changing the counter.
- Scalars and target-native immediate values do not use this tagged handle ABI.
- GC-native backends may erase the physical tags while preserving semantic analysis.

### Projections

The tags belong to the allocation-family handle, not an arbitrary interior address.

A projection must therefore either:

- carry the family base handle alongside its offset or projection data
- use a canonical handle that can recover the family base
- use another backend representation that preserves the same two logical bits and family identity

Masking an arbitrary interior pointer is not sufficient.

Field-sensitive splitting may later assign the projected field its own family handle.

### No runtime strategy upgrade

An uncounted allocation is never upgraded to REC at runtime.

The memory planner selects the layout before code generation. Hidden destination allocation and mixed allocator helpers may choose the planned destination representation at allocation time.

## Inline counter representation

Only REC-managed allocation families carry a counter.

Conceptual layout:

```text
+-----------------------+
| REC count word        |
+-----------------------+
| allocation-family data|
+-----------------------+
```

The exact offset and any integration with an existing backend header are backend details.

Requirements:

- no global REC side table
- no universal counter on affine, region or group allocations
- one target-word-sized unsigned count
- `u32` on Wasm32
- `u64` on ordinary 64-bit native targets
- non-atomic operations under the accepted concurrency model

The maximum count is bounded by addressable storage because every counted edge requires runtime storage. Development builds still assert overflow and underflow invariants.

Counter underflow must trap in every profile rather than permit double free or undefined behavior.

## Counter invariant

For an REC-managed allocation family:

```text
REC count =
    number of live counted persistent-edge obligations
    + one optional affine-root obligation
```

There can never be more than one affine-root obligation for one allocation family.

### Per-family transition algebra

Every retention-sensitive semantic commit is evaluated independently for each target allocation family `F`. This equation is normative:

```text
delta_count(F) =
    created_persistent_edges(F)
    - removed_persistent_edges(F)
    + affine_root_after(F)
    - affine_root_before(F)
```

```text
affine_root_before(F) in {0, 1}
affine_root_after(F)  in {0, 1}
```

- `created_persistent_edges(F)`: direct persistent edges into `F` stored by this commit
- `removed_persistent_edges(F)`: direct persistent edges into `F` removed by this commit
- `affine_root_before(F)`: `1` when an owned affine root for `F` exists on the incoming path
- `affine_root_after(F)`: `1` when an owned affine root for `F` exists on the outgoing path

A single affine root can reclassify into at most one new edge, and at most one removed edge can reclassify into a returned affine root.

```text
one-edge final-use insertion:
    before: 0 edges, 1 root
    after:  1 edge,  0 roots
    delta = +1 - 1 = 0

two-edge final-use insertion:
    before: 0 edges, 1 root
    after:  2 edges, 0 roots
    delta = +2 - 1 = +1

two-edge removal returning one affine root:
    before: 2 edges, 0 roots
    after:  0 edges, 1 root
    delta = -2 + 1 = -1
```

### Atomic per-family commits

A semantic storage mutation commits its complete obligation delta atomically for each target family. Do not model an overwrite as decrement, possible destroy, increment when the operation replaces one obligation with another obligation to the same family:

```text
same target family F:

old edge removed  -1
new edge created  +1
---------------------
net transition     0
```

Destruction is tested only after the complete semantic commit delta for `F` is known. There is no independent transient-zero ownership problem.

The counter does not equal the number of all source aliases.

It excludes:

- local aliases
- parameters
- temporary projections
- ordinary call borrows
- `get()` results
- statically known region-only edges
- explicit-group-owned edges

## Initialization

A fresh REC allocation returned with affine cleanup responsibility starts with:

```text
count = 1
handle tag = 11
```

A fresh REC allocation constructed directly into one counted persistent edge may start with:

```text
count = 1
stored handle tag = 10
```

The initial count reflects the initial liveness obligation. A temporary root need not be created and removed when hidden destination allocation constructs directly into storage.

## Common direct-handle count transition table

This table is a shorthand for the per-family equation above, for the common direct-handle case. `N` is the number of direct persistent edges into the target family that the operation creates or removes; in the common case `N` is `1`.

| Operation | Counter effect | Handle effect |
|---|---:|---|
| ordinary local alias | `0` | borrowed alias, no new obligation |
| borrowed function call | `0` | callee receives owned bit clear |
| affine root transfer | `0` | `11` moves to new path |
| persistent insertion while affine root remains | `+N` created edges | stored edge obligations use `10`, root stays `11` |
| final-use insertion | `+(N - 1)`, because one root reclassifies into at most one edge | one root becomes a stored `10` edge, further edges are new obligations |
| `get()` | `0` | returns temporary `10` borrow |
| persistent removal with no returned root | `-N` | obligations disappear |
| detachment producing an affine root | `1 - N`, because at most one removed edge reclassifies into the returned root | one removed obligation becomes an `11` result, further removed obligations disappear |
| detachment while an affine root already remains | `-N` | result is borrowed `10` |
| affine root discharge | `-1` | owned path discharged; family destroyed only if the count reaches zero |
| overwrite | compute one atomic per-family before/after delta | old summary disappears, new summary appears |
| whole-domain clear | one release per surviving external counted-edge obligation unless region elision applies | domain obligations disappear |
| count reaches zero | destroy exactly once | no usable handles remain |

The one-edge fast path stays cheap and remains the common case:

```text
N = 1 final-use insertion -> 0
N = 1 extraction to root  -> 0
```

These `0` results are scoped to `N = 1`. There is no unconditional general rule that final-use insertion or extraction to an affine root leaves the count unchanged.

### Discharge is not destruction

Keep four terms distinct: **transfer** moves affine cleanup responsibility, **discharge** satisfies the current obligation, **destroy** physically destroys one allocation family and **bulk reclaim** reclaims a region or group.

```text
discharge owned REC root
    -> remove one affine-root obligation
    -> decrement count by one

if count > 0:
    family remains alive

if count == 0:
    destroy family
```

Dropping an owned REC root therefore does not necessarily destroy or free the family.

## Why temporary aliases do not race count zero

Borrow validation guarantees that a persistent edge or affine root cannot be destroyed while a temporary borrow depending on it remains usable.

Examples:

- a map cannot mutate while a live `get()` result aliases one of its values
- an affine root is not released before the last local borrow
- a function cannot transfer ownership and then use an alias on the same path

REC relies on these existing proofs. It does not count temporary aliases as a second safety mechanism.

## Core backend operations

The backend needs four conceptual operations.

```text
retain_persistent(handle)
release_persistent(handle)
release_if_owned(handle)
extract_persistent(handle)
```

Their final implementation should be specialized by static representation state.

### `retain_persistent`

- uncounted target: no REC operation
- REC borrowed insertion: increment
- REC final-use insertion: reclassify the affine root into one edge, then apply `+(N - 1)` for any further direct edges the same commit creates into that family

### `release_persistent`

- uncounted borrowed edge: no individual cleanup
- uncounted affine-owned child: deterministic child cleanup
- REC edge: decrement and enqueue destruction if zero

### `release_if_owned`

- borrowed handle: no operation
- owned uncounted handle: deterministic drop or region-specific action
- owned REC handle: decrement affine root obligation and enqueue destruction if zero

### `extract_persistent`

- detach the existing edge from storage
- transfer or reclassify cleanup responsibility to the result when legal
- avoid a decrement plus increment when one obligation changes category

These are compiler/backend concepts, not source builtins.

## Function boundaries

REC-managed values may cross arbitrary Moth function and Moth package boundaries.

Most functions do not need to inspect REC state.

### Read-only and temporary-borrow functions

A function that only reads a value:

- receives an ordinary borrowed handle
- performs no count change
- does not branch on the REC bit

### Affine transfer

A final-use call moves the owned bit with the handle.

- no count increment
- no count decrement
- no new function variant

### Retention-sensitive functions

A function needs REC representation knowledge only where it:

- creates a persistent retained edge
- destroys a persistent retained edge
- extracts and reclassifies an edge
- discharges affine cleanup responsibility

The REC bit travels with the allocation-family handle and is inspected locally at those operations.

REC representation is physical-variant state. One source function may be lowered GC-native in one target/profile variant and with the two-bit REC-capable ABI in another, with no difference in source semantics or in its public semantic interface. Local tag tests stay limited to retention-sensitive operations inside a full-control variant.

### Avoid whole-function specialization

Do not generate default families such as:

```text
remember_rec
remember_region
remember_affine
```

Use one function body. At each retention-sensitive operation, lower one of:

- no operation when statically uncounted
- direct REC arithmetic when statically counted
- one small REC-bit test when genuinely mixed

Representation propagation should classify each operation as:

```rust
pub enum RecRepresentationState {
    Never,
    Always,
    Mixed,
}
```

`Never` removes the branch.

`Always` emits direct count operations.

`Mixed` emits a local tag test around the memory operation only.

### Returns and multiple aliases

Return tags preserve affine responsibility.

- a returned affine root carries `11`
- a returned borrow carries `10`
- result-to-result alias summaries ensure at most one returned handle carries the affine root for one family
- detached stored results may reclassify returned obligations without count traffic

## Hidden destination allocation

Fresh-result allocation may be directed into:

- an inferred region
- an explicit group
- an REC layout
- an ordinary affine allocation

A reusable function may therefore receive a hidden allocation plan or destination handle at the allocation operation. The body need not be duplicated.

The selected destination is not a source lifetime parameter and does not enter `TypeId`.

## Moth package boundaries

Moth source-package boundaries use semantic summaries.

A package function may say conceptually:

- retains parameter `P`
- may create runtime-many retention for `P`
- kills the complete receiver retention domain
- extracts a result from the receiver
- returns an alias or projection

The consuming link plan selects REC or another strategy for concrete allocation families.

Base package artifacts remain immutable. Physical lowering may insert local mixed-strategy tag tests where required.

## Foreign boundaries

REC does not weaken closed foreign-boundary rules.

Ordinary JavaScript, WIT and native bindings must not retain arbitrary Moth references.

Permitted boundary shapes remain:

- independent value conversion
- Moth-owned opaque handles passed by value under a closed contract
- foreign-owned resource handles
- a separately designed trusted runtime participant, only if accepted in the future

A foreign side does not implicitly increment or decrement Moth REC.

## Destruction plans

REC zero-count destruction uses compiler-generated concrete destruction plans. It does not use runtime reflection.

For each concrete allocation family, the compiler knows its outgoing retained edges after field splitting and strategy selection.

A destruction plan may contain:

- uncounted owned child drop
- REC child decrement
- borrowed child no-op
- region-owned child no-op until region release
- backing-storage cleanup
- final allocation free

Generic functions are already materialized to concrete types before backend handoff, so destruction plans can remain concrete.

## Iterative destruction worklist

Do not recursively destroy REC cascades on the native or Wasm call stack.

Use an iterative worklist:

```text
decrement obligation
if count becomes zero:
    enqueue family

while queue is not empty:
    family = pop

    process outgoing retained edges
    enqueue children that reach zero
    free family
```

The runtime may reuse dead object header space for worklist links where the backend layout permits it. The plan does not require that encoding.

The worklist must preserve deterministic cleanup semantics without exposing destruction order to source programs.

## Mixed region and REC cleanup

A region may contain internal objects plus outgoing edges to REC-managed allocations owned elsewhere.

When the region ends:

- internal region-to-region edges need no individual processing
- group-owned internal edges need no individual processing
- outgoing REC edges must be decremented
- the region then bulk-reclaims its internal storage

Conceptual plan:

```text
destroy region R:
    process outgoing REC boundary edges
    bulk release R
```

A `clear()` frontier may therefore bulk-release one local region while still decrementing external REC targets retained by that collection.

## Explicit groups

Explicit groups have hard semantics.

- group-owned allocations carry no REC counter
- group-owned children do not receive individual affine cleanup responsibility
- last-use analysis still validates accesses but does not shorten group-owned physical lifetime
- group exit performs bulk cleanup
- cycles are allowed only inside one group

Group ownership affects the **target** allocation, not every edge whose source happens to be inside a group.

```text
group-owned target
    -> no REC counter

edge inside group -> group-owned target
    -> count-free

edge inside group -> external REC target
    -> ordinary persistent REC obligation on external target
```

At `clear()` or group exit the ordering is:

```text
process outgoing obligations to external REC families
-> bulk reclaim group-owned storage
```

The external target is destroyed only if its own count reaches zero. Group count-free semantics never mean that every external allocation referenced from a group is count-free.

The initial implementation does not attempt to coalesce every group-to-external alias into one group-domain count.

## Field-sensitive allocation splitting

Field-sensitive splitting is part of the final memory architecture.

It runs before final REC selection where implemented.

Example:

```text
Document family
    title: 32 bytes
    body and assets: 100 MB
```

If an index retains only `title`, keeping the whole family alive is unacceptable.

Splitting may create:

```text
Title family
Body family
Assets family
```

REC and region strategy are then chosen independently for each family.

REC cannot compensate for an overly broad unsplit family. It would only count retention of that broad family more precisely.

## Physical region and drop coalescing

Semantic lifetime inference should remain maximally precise.

A later physical planner may coalesce nearby implicit lifetimes into one synthetic arena or grouped drop when the retained-byte cost is small and the cleanup savings are worthwhile.

The first heuristic may require:

- straight-line control flow
- no intervening function call
- no effectful or potentially expensive operation between candidate drops
- one common proven enclosing region

This is separate from REC strategy and separate from semantic region widening.

## Static cost policy

The initial REC implementation should prioritize hard proofs over speculative tuning.

Accepted hard elisions:

- no dynamic retained multiplicity
- affine transfer
- complete cleanup frontier
- explicit-group ownership
- field-sensitive split that removes the expensive family
- short and statically bounded fallback region

A small bounded-retention heuristic may be added only when:

- the retained family size is statically known or conservatively bounded
- edge cardinality is fixed or bounded
- allocation-site repetition is known or conservatively bounded
- the fallback lifecycle is short

No runtime strategy switching is allowed.

No `config.moth` tuning is accepted initially.

## Concurrency and channels

REC is non-atomic by design.

The future channel system must preserve this assumption.

- channel send transfers affine responsibility
- arbitrary shared Moth reference graphs do not cross task boundaries
- queued values belong to a channel or task lifecycle
- failed send preserves or returns responsibility to the sender
- group-owned values do not cross independently
- cycles remain group-only

If future concurrency requires atomic REC, that is a new memory-model design decision and not an implementation detail.

## Rejected alternatives

### General reference counting

Rejected because it counts ordinary locals, parameters and temporary borrows that Moth already resolves statically.

### Universal REC header

Rejected because affine, region and group allocations must not pay REC memory overhead.

### Runtime adaptive REC

Rejected because it adds strategy state, hot-path branches and potential physical variants while giving up exact counts for a heuristic gain.

### Runtime region pressure switching

Rejected for the same reason. Region pressure may be useful as future profiling terminology, but it does not change strategy at runtime.

### Uniqueness scans on removal

Rejected because scans can turn expected constant-time removals into linear or quadratic behavior and require closed alias-domain reasoning.

### Alias registries

Rejected because maintaining a dynamic alias set is more metadata and pointer traffic than a counter.

### `group self`

Rejected because group extraction, shared values and cross-collection retention would require region migration, adoption or implicit copying.

### Source-visible RC

Rejected because REC is a backend strategy, not a source ownership model.

### Foreign REC participation

Rejected because it would require a shared cross-language allocator, lifetime and decrement protocol.

### Whole-function strategy variants by default

Rejected due to binary-size growth. Mixed strategy is localized to retention-sensitive operations.

## Developer observability

The initial implementation must make REC decisions visible to compiler developers.

### Compile-time feature

Add a development-only Cargo feature or cfg named conceptually:

```text
rec_debug
```

It compiles structured REC decision records and stronger count invariants.

### Internal report flag

Add an internal compiler flag named conceptually:

```text
--show-rec
```

It emits one structured report for every considered allocation site, whether REC is selected or elided.

The final spelling should follow existing compiler debug-flag conventions, but both compile-time gating and per-invocation output control are required.

### Structured decision record

Conceptual shape:

```rust
pub struct RecDecisionRecord {
    pub allocation_site: AllocationSiteId,
    pub family: AllocationFamilyId,
    pub dynamic_multiplicity: bool,
    pub fallback_region: RegionId,
    pub cleanup_frontier: Option<CleanupFrontierId>,
    pub strategy: MemoryStrategy,
    pub reasons: Vec<RecDecisionReason>,
}
```

Reason enum:

```rust
pub enum RecDecisionReason {
    NotHeapManaged,
    NoDynamicMultiplicity,
    AffineTransfer,
    FixedRetainedSet,
    ExplicitGroup,
    FinalCleanupFrontier,
    ShortFallbackRegion,
    SmallBoundedRetention,
    FieldSplit,
    RecRequired,
}
```

Tests assert the structured reason. Rendered prose is not the authority.

### Report examples

REC selected:

```text
allocation: Blob at cache.moth:42
multiplicity: runtime-many
fallback owner: PageRegion
cleanup frontier: none
strategy: REC
reason: RecRequired
```

REC elided:

```text
allocation: ParsedPost at index.moth:18
multiplicity: runtime-many
fallback owner: FunctionRegion
cleanup frontier: index.clear at index.moth:61
strategy: InferredRegion
reason: FinalCleanupFrontier
```

## Initial implementation boundaries

The first implementation should be deliberately narrow.

### Required first slice

- direct retained edges in builtin growable collections
- direct retained edges in builtin fixed collections
- direct retained edges in builtin maps
- `push`, `set`, `remove`, `clear` and destruction
- ordinary local aliases remain static
- `get()` remains count-free
- exact two-bit handle lowering
- non-atomic inline counter
- iterative zero-count destruction
- cleanup-frontier REC elision
- explicit-group REC elision
- structured decision reporting

### Required compositional slice

- user-defined functions wrapping builtin collection operations
- public retention and detached stored-result summaries
- Moth package propagation
- mixed counted and uncounted function paths
- local tag tests only at retention-sensitive operations

### Deferred extensions that must remain compatible

- transitive retention through user aggregate fields
- ordinary fixed-field dynamic multiplicity
- recursive acyclic aggregates
- field-sensitive allocation splitting
- more sophisticated static cost models
- profile-guided strategy selection
- group-to-external domain coalescing

These are deferred implementation stages, not permission to design a collection-only semantic model.

# Implementation plan

Each phase ends with an audit before the next phase starts. Do not implement a parallel memory model or preserve GC fallback as the release answer.

## Phase 0: Lock documentation authority and migration boundaries

### Summary and reasoning

This phase locks REC into the replacement final model rather than treating it as an optional patch to the superseded collector-fallback architecture. The canonical memory authorities now make static topology proof mandatory, keep cycles group-only, and place REC after topology validation as one physical strategy. Compiler implementation remains deferred.

### Tasks

- [x] Add this plan at the proposed roadmap path.
- [x] Add the REC canonical docs directory and route it from the memory overview.
- [x] Add a concise REC section to the main final memory-management plan.
- [x] State that full-control release backends cannot fall back to tracing GC.
- [x] State that REC is one physical strategy after mandatory topology validation.
- [ ] Confirm every full-control handle path uses the two-bit logical handle contract.
- [x] Mark REC implementation as incomplete in the progress matrix.
- [x] Mark current GC fallback implementation as migration debt rather than accepted final behavior.
- [x] Record profiling, adaptive REC and config tuning as out of scope.

### Audit and validation

- [x] No document claims REC is source-visible.
- [x] No document claims dynamic multiplicity automatically requires REC.
- [x] No document permits cycles under REC.
- [x] No final-design document says release may silently retain tracing GC.
- [x] Documentation links have one clear authority chain.

## Phase 1: Add retained-edge effect vocabulary

### Summary and reasoning

REC planning requires precise semantic effects before any counter can be emitted. This phase extends immutable analysis facts and public summaries without changing runtime behavior.

### Tasks

- [ ] Define allocation-family identity after current aggregate-family analysis.
- [ ] Define retained-edge creation, destruction and whole-domain kill facts as summaries contributed by stored values rather than one-edge operation shorthands.
- [ ] Define retention cardinality, including `RuntimeMany`.
- [ ] Add `DetachedStoredValue` provenance for container-detached results.
- [ ] Add result-to-result family alias relationships where missing.
- [ ] Add retained receiver and parameter relationships.
- [ ] Preserve successful and error-path retention effects as separate exits.
- [ ] Add cleanup-frontier candidate facts.
- [ ] Extend public interface fingerprints to cover the new summaries.
- [ ] Extend generated-function sidecars with the same summary vocabulary.
- [ ] Keep donor-local region and family IDs out of exported interfaces.

### Audit and validation

- [ ] Every new fact has one semantic owner.
- [ ] Borrow validation does not decide REC strategy.
- [ ] HIR is not rewritten to encode REC.
- [ ] Public summaries remain backend-neutral.
- [ ] Private body changes that do not alter summaries do not invalidate semantic consumers.

## Phase 2: Implement retained-edge liveness and cleanup frontiers

### Summary and reasoning

The compiler must prove when runtime-many aliases disappear together before it considers counting. This phase provides the main REC-elision mechanism.

### Tasks

- [ ] Track retained-edge liveness separately from aggregate binding lifetime.
- [ ] Recognize builtin `clear` as a whole-domain kill.
- [ ] Recognize aggregate destruction as a whole-domain kill.
- [ ] Recognize whole-value replacement when old contents are definitely discarded.
- [ ] Support path-sensitive frontiers across branches.
- [ ] Support loop and repeated-population region epochs.
- [ ] Reject a frontier when a future surviving alias can recreate retention.
- [ ] Export complete-domain kill summaries through user wrappers.
- [ ] Add topology fixtures where collections survive beyond their retained-value regions.

### Audit and validation

- [ ] A collection can remain live after an element-region frontier.
- [ ] `get()` borrows block destructive frontiers until their last use.
- [ ] Individual `remove` does not masquerade as whole-domain cleanup.
- [ ] Frontiers are CFG facts, not method-name guesses.
- [ ] Every frontier is safe on every represented path.

## Phase 3: Add memory-strategy planning and REC decision reporting

### Summary and reasoning

Eligibility and physical selection must remain separate. This phase chooses strategies without changing pointer layout yet.

### Tasks

- [ ] Add `Affine`, `InferredRegion`, `ExplicitGroup` and `Rec` strategy facts.
- [ ] Run strategy selection only after complete topology validation.
- [ ] Implement the deterministic elision order in this plan.
- [ ] Add `RecDecisionReason` as structured data.
- [ ] Add `rec_debug` compile-time gating.
- [ ] Add the internal `--show-rec` report path.
- [ ] Report every considered allocation site, including elided sites.
- [ ] Add tests for each decision reason.
- [ ] Add a hard assertion that group-owned families never select REC.

### Audit and validation

- [ ] Runtime-many facts do not force REC.
- [ ] Cleanup-frontier examples choose regions.
- [ ] Long-lived independent-removal examples choose REC.
- [ ] Decision output is deterministic.
- [ ] Tests assert enums and IDs rather than prose.

## Phase 4: Scaffold the two-bit allocation-family handle ABI

### Summary and reasoning

The runtime needs one compact representation that carries affine responsibility and counted state without function-family duplication.

### Tasks

- [ ] Reserve bit 0 for `OWNED` and bit 1 for `REC` on full-control heap family handles.
- [ ] Enforce four-byte alignment for taggable family handles.
- [ ] Centralize tag masking and construction helpers.
- [ ] Ensure ordinary dereference masks both bits.
- [ ] Define projection representation that retains family-base recovery.
- [ ] Preserve tags across Moth function and package calls.
- [ ] Preserve tags across returns and multiple-return alias summaries.
- [ ] Keep scalar and immediate ABIs unchanged.
- [ ] Allow GC-native backends to erase physical tags after semantic validation.
- [ ] Add debug assertions for invalid tag combinations and family recovery.

### Audit and validation

- [ ] No backend interprets raw tagged values without centralized masking.
- [ ] Projections cannot lose family identity.
- [ ] At most one returned alias carries the owned bit for one family.
- [ ] No source type or `TypeId` changes due to tags.
- [ ] Legacy single-tag assumptions are removed rather than layered underneath.

## Phase 5: Implement REC allocation layout and counter primitives

### Summary and reasoning

This phase adds exact REC mechanics while keeping all non-REC layouts free of count overhead.

### Tasks

- [ ] Add inline count storage only for `Rec` families.
- [ ] Use a target-word-sized unsigned count.
- [ ] Add fresh affine-root initialization.
- [ ] Add direct persistent-edge initialization for hidden destination allocation.
- [ ] Implement non-atomic increment and decrement.
- [ ] Implement zero-count enqueue.
- [ ] Trap on underflow in every profile.
- [ ] Assert overflow in REC development builds.
- [ ] Keep affine, region and group layouts count-free.
- [ ] Add allocation-layout verification to backend handoff.

### Audit and validation

- [ ] Uncounted objects have no REC word.
- [ ] Count starts with the correct initial obligation.
- [ ] Affine transfer does not touch the counter.
- [ ] Borrowed calls do not touch the counter.
- [ ] Count zero cannot occur with a legal live temporary alias.

## Phase 6: Integrate direct builtin collection edges

### Summary and reasoning

Builtin collections are the first practical REC boundary and the trusted effect substrate for user collections.

### Tasks

- [ ] Implement counted `push` insertion.
- [ ] Implement final-use insertion reclassification: one root becomes one edge, and the commit applies `+(N - 1)` for any further direct edges into the same family.
- [ ] Implement counted `set` replacement.
- [ ] Implement counted `remove` edge destruction.
- [ ] Implement detached-stored-result reclassification into an affine root.
- [ ] Keep `get()` count-free.
- [ ] Implement `clear()` count processing when region elision does not apply.
- [ ] Implement collection destruction.
- [ ] Cover fixed collections, growable collections and maps.
- [ ] Apply one complete retained-edge summary per scalar, direct, aggregate or transitive element value.
- [ ] Cover new-key map insertion, existing-key replacement, equal-content lookup keys and key/value aliases.
- [ ] Drop the stored key obligation and detach the stored value obligation on map removal.
- [ ] Commit all successful-path effects atomically and preserve topology on every error path.
- [ ] Preserve collection error and trap semantics.

### Audit and validation

- [ ] Unique insertion stays affine when REC is unnecessary.
- [ ] Duplicate runtime aliases count exactly once per persistent edge.
- [ ] `get()` emits no count traffic.
- [ ] Detached stored results avoid decrement-plus-increment when reclassification is legal.
- [ ] `clear()` is skipped entirely for count-free frontier regions where legal.

## Phase 7: Add iterative destruction plans

### Summary and reasoning

REC cascades must not recurse through the machine stack. Concrete destruction plans also preserve field and strategy precision.

### Tasks

- [ ] Generate concrete outgoing-edge destruction plans.
- [ ] Add iterative zero-count worklist processing.
- [ ] Process REC children through decrements.
- [ ] Process uncounted owned children through deterministic drop.
- [ ] Skip borrowed and region-owned children where appropriate.
- [ ] Free each family exactly once.
- [ ] Add long-chain fixtures that would overflow recursive destruction.
- [ ] Add diamond-DAG fixtures with shared REC children.
- [ ] Add invariant traps for double enqueue or double free.

### Audit and validation

- [ ] Long chains use bounded call-stack space.
- [ ] DAG children free only at final obligation removal.
- [ ] No cycle fixture is accepted outside an explicit group.
- [ ] Destruction order remains source-unobservable.

## Phase 8: Compose through user functions and Moth packages

### Summary and reasoning

User collection packages must receive builtin-quality memory behavior without source annotations or privileged method names.

### Tasks

- [ ] Infer user `clear` summaries from complete builtin-domain kills.
- [ ] Infer user detached stored-result summaries from builtin `remove`.
- [ ] Infer runtime-many retention through wrappers.
- [ ] Export summaries in immutable public interfaces.
- [ ] Instantiate summaries across package graphs.
- [ ] Add `Never`, `Always` and `Mixed` representation propagation for retention-sensitive operations.
- [ ] Remove local REC branches when all reachable values are uncounted.
- [ ] Emit direct REC operations when all reachable values are counted.
- [ ] Emit one local tag test only for genuinely mixed operations.
- [ ] Avoid whole-function REC and non-REC duplication by default.

### Audit and validation

- [ ] Renaming a user method does not change inferred memory effects.
- [ ] Package consumers never inspect provider HIR.
- [ ] Summary changes invalidate the public-interface fingerprint.
- [ ] Mixed paths branch only around retention operations.
- [ ] Binary growth remains bounded in representative generic and package fixtures.

## Phase 9: Integrate regions, explicit groups and mixed cleanup

### Summary and reasoning

REC must remain subordinate to region proof. This phase removes unnecessary count traffic at bulk boundaries and handles outgoing REC edges safely.

### Tasks

- [ ] Hard-disable REC for group-owned families.
- [ ] Preserve REC for externally owned targets retained from inside a group where needed.
- [ ] Generate outgoing REC boundary-edge plans for region cleanup.
- [ ] Bulk-release internal region edges without individual decrements.
- [ ] End inferred region epochs at final cleanup frontiers.
- [ ] Verify whole-domain `clear` chooses region cleanup when all targets are internal.
- [ ] Verify mixed `clear` decrements external REC targets before bulk release.
- [ ] Keep explicit groups free of individual early drop.

### Audit and validation

- [ ] Group-owned cycles remain count-free.
- [ ] Cross-region cycles remain invalid.
- [ ] Region cleanup never loses an outgoing external obligation.
- [ ] Complete internal domains perform no pointless REC traffic.
- [ ] Group semantics remain visible and predictable to the programmer.

## Phase 10: Preserve generic aggregate and field-splitting extension points

### Summary and reasoning

The initial implementation is collection-first, but the semantic mechanism must support transitive aggregate retention and future field splitting without redesign.

### Tasks

- [ ] Represent retained edges through ordinary struct fields.
- [ ] Propagate transitive retention summaries through values stored in collections.
- [ ] Reserve REC planning for choice payloads and recursive acyclic aggregates.
- [ ] Keep cycles group-only.
- [ ] Define the handoff from field-sensitive family splitting into strategy planning.
- [ ] Rerun cardinality and REC selection after a split.
- [ ] Add deferred fixtures for tiny projections retaining heavy parent families.
- [ ] Do not block initial REC completion on full field-sensitive splitting.

### Audit and validation

- [ ] No collection-specific semantic shortcut prevents future aggregate REC.
- [ ] A collection of `Holder` values can eventually account for `Holder -> Blob` retention.
- [ ] Splitting changes family identity before tag and counter layout selection.
- [ ] REC never pretends to solve an overly broad unsplit family.

## Phase 11: Backend release verification and parity

### Summary and reasoning

The feature is not complete until capable release backends prove collector-free output and GC-backed profiles preserve identical source semantics.

### Tasks

- [ ] Add backend capability metadata for collector-free release.
- [ ] Verify every reachable heap family has one physical strategy.
- [ ] Verify all REC families have valid count and destruction plans.
- [ ] Verify all region families have complete cleanup plans.
- [ ] Verify all group families have hard bulk exits.
- [ ] Reject missing strategy data as `CompilerError`.
- [ ] Verify no tracing collector dependency remains in capable release output.
- [ ] Keep debug and GC-native backends semantically equivalent.
- [ ] Add cross-backend acceptance and observable-behavior fixtures.

### Audit and validation

- [ ] Backend lowering never redecides topology legality.
- [ ] Release never silently falls back to tracing GC.
- [ ] Debug GC does not accept programs rejected by topology validation.
- [ ] REC and region strategies remain source-unobservable.

## Phase 12: Documentation and progress-matrix migration

### Summary and reasoning

The documentation migration and consistency pass now use the accepted collector-free model, value-shaped retained-edge effects, detached stored-result terminology, exit-specific public effects and caller-local concrete frontiers. Implementation status remains explicit: REC and the surrounding topology planner are accepted design, not implemented compiler support.

### Tasks

- [x] Update `docs/src/developer-docs/memory-management/overview.mtf` with the six-part memory model and concise REC role.
- [x] Update `ownership-and-drops` to the Affine Ownership ABI and two-bit handle extension.
- [x] Update lifetime-region docs with retained-edge liveness, cleanup frontiers, region epochs and REC eligibility.
- [x] Update declared-group docs with hard count-free group ownership and cycle policy.
- [x] Update runtime/backend docs with the strategy planner, two-bit tags, REC layout and collector-free release invariant.
- [x] Add the canonical REC technical page.
- [x] Update language collection and map docs with value-shaped retained-edge summaries, key/value effects, successful commits and unchanged error paths.
- [x] Add the public Automatic cleanup and retained edges page pair and route it from the public Memory page.
- [x] Distinguish detached stored results from group extraction/adoption and interior projection detachment.
- [x] Keep public summary effects exit-specific and caller-localise concrete cleanup frontiers.
- [x] Update `docs/compiler-design-overview.md` public summaries, analysis boundaries and backend handoff.
- [x] Update `docs/build-system-design.md` physical-strategy plans, capability metadata and release verification.
- [x] Update the README memory goal from optional GC avoidance to the accepted collector-free release direction.
- [x] Update the language cheatsheet with only a concise user-facing statement. Do not expose REC mechanics as source semantics.
- [x] Update `docs/src/docs/progress/@page.moth` with separate rows for topology, retained-edge liveness, Affine Ownership ABI, REC planning, REC backend lowering, explicit groups and collector-free release verification.
- [x] Mark old compiler paths that still assume GC fallback as incomplete migration work.
- [x] Link this plan from the main final memory-management plan.

### Audit and validation

- [x] No final authority calls GC the semantic correctness baseline.
- [x] No final authority describes only one pointer tag bit for full-control lowering.
- [x] REC detail lives here and in the canonical REC page, not duplicated across every memory document.
- [x] User-facing docs explain behavior without exposing compiler-only counter machinery.
- [x] Progress status distinguishes accepted design from implemented support.

## Phase 13: Final validation and closeout

### Summary and reasoning

Close REC only after semantic, ABI, runtime, package and documentation gates pass together.

### Tasks

- [ ] Run the repository validation command.
- [ ] Run native and Wasm-specific compiler tests.
- [ ] Run direct collection REC fixtures.
- [ ] Run scalar-zero, direct-one, aggregate-several and transitive-retention fixtures.
- [ ] Run map key/value, replacement, equal-content lookup and detached-value fixtures.
- [ ] Run successful-commit and failed-mutation fixtures with exit-sensitive summaries.
- [ ] Run user-wrapper and package-summary fixtures.
- [ ] Run explicit-group count-elision fixtures.
- [ ] Run cleanup-frontier region-elision fixtures.
- [ ] Run mixed region-to-external-REC cleanup fixtures.
- [ ] Run long DAG destruction fixtures.
- [ ] Run binary-size comparison for mixed function boundaries.
- [ ] Run compile-time comparison with `rec_debug` disabled.
- [ ] Confirm the REC report shows every considered and selected allocation deterministically.
- [ ] Record implementation status in the progress matrix.

### Audit and validation

- [ ] No universal counter remains.
- [ ] No ordinary local alias changes a counter.
- [ ] No `get()` changes a counter.
- [ ] No group-owned allocation carries REC.
- [ ] No cycle depends on REC.
- [ ] No full-control release path depends on tracing GC.
- [ ] No whole-function strategy explosion was introduced by default.
- [ ] All stale collector-fallback and single-tag architecture wording is removed or clearly marked as current implementation debt.

# Required test matrix

These fixtures define the semantic contract before backend implementation lands. They must exercise
value-shaped obligations, map key/value ownership and exit-sensitive commits rather than encode a
one-direct-handle shortcut.

| Case | Expected strategy or result |
|---|---|
| local aliases only | affine or region, no REC |
| borrowed function calls | no count traffic |
| final-use call transfer | owned bit moves, no count traffic |
| one unique collection insertion | affine collection ownership, no REC where provable |
| runtime branch chooses one destination | affine path selection, no REC |
| duplicate runtime keys followed by `clear` | inferred region epoch, REC elided |
| duplicate runtime keys with individual long-lived eviction | REC |
| `get` followed by use | no count change |
| mutation while `get` alias is live | borrow diagnostic |
| one-direct-edge final-use insertion into REC collection | root-to-edge reclassification, no count change |
| scalar collection element | zero retained-edge obligations |
| direct heap collection element | one direct retained-edge obligation |
| aggregate collection element | several direct retained-edge obligations when its representation stores several handles |
| new-key map insertion | stored key and stored value obligations both appear |
| existing-key map replacement | stored key remains, old value obligations disappear and new value obligations appear |
| equal-content replacement lookup key | lookup key remains borrowed and is not retained |
| map key and value alias one family | two obligations point to one allocation family |
| map removal with aliased key and value | key obligation decrements and value obligation detaches into the result |
| failed collection or map mutation | original storage topology and obligations remain unchanged |
| one-direct-edge remove returning affine result | one detached obligation reclassifies into the result, with no count change; a multi-edge detachment is `1 - N` |
| remove while another affine root exists | edge decrement, borrowed result |
| overwrite REC child | old decrement and new retain/reclassification |
| explicit group with arbitrary aliases | no REC, bulk cleanup |
| explicit group cycle | valid and count-free |
| implicit cycle | diagnostic |
| region with outgoing REC edges | decrement boundary edges then bulk release |
| user collection wrapper around builtin map | same inferred summaries as builtin composition |
| mixed REC and region call sites | one body, local tag test only at memory operation |
| package boundary | semantic summary propagation, no source REC type |
| one-edge final-use insertion | count unchanged |
| two-edge final-use insertion into same family | count `+1` |
| two-edge detachment to one affine root | count `-1` |
| same-family overwrite with net-zero obligation delta | count unchanged |
| no transient destruction during same-family overwrite | family never destroyed mid-commit |
| separately allocated `Holder -> Blob` | no duplicated collection-to-`Blob` obligation |
| inline `Holder` with embedded `Blob` handle | direct collection-domain `Blob` obligation |
| group-owned target | no REC |
| group-to-external REC target | decrement on `clear` or group exit, before bulk reclamation |
| failed fallible mutation with later source use | borrow, no transfer |
| failed fallible mutation after all-path final use | no committed edge, affine obligation discharged safely |
| same source function in two physical variants | GC-native in one and REC-capable in the other, with no semantic interface change |
| foreign boundary | no retained Moth reference permitted |
| long acyclic REC chain | iterative destruction with bounded stack |
| diamond DAG | shared child freed at final obligation |
| projection into unsplit family | count targets family base |
| field split | strategy reruns for new families |

# Completion criteria

REC is complete only when all of these are true.

1. The compiler can identify runtime-many persistent retained edges.
2. The compiler can elide REC through cleanup frontiers, regions and explicit groups.
3. REC-selected families use the two-bit handle ABI and selective inline counters.
4. Count transitions preserve the affine cleanup invariant.
5. Builtin collections implement exact retain, release, clear and detached-stored-result behavior.
6. User functions and packages inherit those effects compositionally.
7. Mixed function boundaries do not require default whole-function duplication.
8. Zero-count destruction is iterative and cycle-free.
9. Full-control release backends verify no tracing collector remains.
10. Developer reports explain every REC selection and elision deterministically.
11. The canonical docs and progress matrix describe the accepted design and actual implementation status without contradiction.

# Handoff to the main memory-management plan

The main plan should carry only this concise contract:

> Moth uses REC only for allocation families with runtime-dependent persistent retained-edge multiplicity that cannot be reclaimed precisely enough through affine last-use analysis, inferred cleanup-frontier regions, field-sensitive splitting or explicit groups. REC counts persistent retained edges, not ordinary aliases. Full-control heap handles carry two logical tag bits: affine owned/borrowed responsibility and REC counted/uncounted representation. Explicit-group-owned values are count-free, cycles remain group-only and capable release backends use REC as one collector-free lowering strategy. Detailed analysis, ABI, counter and elision rules live in the REC companion plan.

The main plan must then link to this document rather than restating its phases and implementation details.
