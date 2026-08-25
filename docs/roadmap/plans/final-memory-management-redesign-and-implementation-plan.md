# Final Memory Management Redesign and Implementation Plan

**Status:** documentation migration complete through the final memory model consistency closure, compiler implementation deferred
**Repository:** `nyejames/moth`  
**Baseline reviewed:** `main` at `34afc996b746bfe93281dad115c40083a9106ac8`  
**Activation branch:** `main`  
**Activation HEAD:** `357dbab3190f1d7160488479ac7cf246d569fded` (squash-merged upstream as `03168082de813b9ce060a9cceb1667f8ea8e1fa1`)  
**Primary companion work:** Retained Edge Counting, whose canonical authority is `docs/src/developer-docs/memory-management/retained-edge-counting/retained-edge-counting.mtf`  
**Required final code gate:** `just validate`  
**Required documentation-only gate:** `moth build docs --release` or `cargo run --quiet -- build docs --release`  

This plan is the parent roadmap for Moth's final memory-management model. It replaces the old GC-fallback direction with mandatory static lifetime topology and a hard collector-free release guarantee for capable backends. The initial Phase 1 migration reopened for one focused consistency pass after review; a second audit corrected REC effects, public retained-edge teaching, summary ownership and implementation-debt wording; and a final consistency closure encoded the multi-edge REC obligation algebra, direct-edge resolution and target-aware physical planning order across the canonical memory pages.

Milestone A is closed after the final multi-edge REC and target-aware physical-planning consistency closure.

The Retained Edge Counting plan owns the detailed REC analysis, ABI, counter and lowering contract. This plan owns the complete source semantics, analysis boundaries, inferred regions, cleanup frontiers, explicit groups, field-sensitive allocation splitting, physical memory planning, backend/profile parity, channel prerequisites and repository-wide documentation migration.

The former `docs/roadmap/plans/grouped-memory-design.md` plan is superseded as the umbrella memory roadmap. Its still-valid explicit-group implementation detail is folded into Phases 7 and 8 and into the canonical declared-group pages, and the old plan is deleted. Do not reintroduce a second overlapping implementation authority.

## Current state

```text
STATUS: documentation slice complete. Milestone A is closed after the final multi-edge REC and
  target-aware physical-planning consistency closure. The canonical memory documentation, REC
  authority, public retained-edge pages, compiler and build authorities, teaching pages, roadmap
  and progress matrix now describe the accepted collector-free model, the per-family REC
  obligation algebra and the target-aware physical planning order. Compiler implementation
  remains deferred and no phase is active.
CURRENT_SCOPE: none active. Milestone A (Phases 0 and 1) is closed; Phases 2 through 18 are
  deferred implementation work awaiting explicit activation.
NEXT_ACTION: Phase 0, the reopened Phase 1 consistency passes and the final memory model
  consistency closure are complete. Do not begin Phase 2 until the borrow and last-use
  implementation slice is explicitly activated on the roadmap.
BLOCKERS: none for the documentation slice; implementation phases remain gated on its completion.
```

---

## 1. Final model summary

Moth is reference-semantic by default, copy-explicit and move-inferred. Memory safety comes from mandatory borrow validation and compiler-inferred lifetime topology rather than source-visible ownership, references or lifetime annotations. Every allocation family has one statically proven lifetime owner, and every retained edge must point to storage that lives for the same or a longer lifetime.

The compiler combines path-sensitive alias tracking, last-use analysis and affine cleanup transfer with compiler-generated non-lexical lifetime intervals. Retained-edge liveness and final cleanup frontiers let inferred regions end before the collections or objects that once held their aliases. Explicit groups provide hard count-free bulk lifetimes and are the only source mechanism for reference cycles. REC covers only unresolved runtime-many persistent retained edges.

These semantics are identical on every backend. Debug builds and GC-native backends may use a garbage-collected representation. Release builds on backends that advertise full memory control must lower every accepted topology without a tracing collector.

---

## 2. Locked design decisions

These decisions are final for this plan.

### 2.1 Source semantics and legality

1. Existing values use shared read-only reference semantics by default.
2. `copy` is the only source operation that creates an independent copy of an existing runtime graph.
3. Mutation requires explicit exclusive access.
4. Moth has no source reference types, lifetime parameters, move syntax, source RC, source REC, weak references or finalizers.
5. Borrow validation and lifetime-topology validation run under every backend and build profile.
6. Missing mandatory topology proof is a source diagnostic.
7. GC cannot legalize an invalid or unproven topology.
8. A backend or profile may not change which source programs are valid.

### 2.2 Collector-free release guarantee

1. There is no source-visible or project-visible no-GC mode.
2. Backends declare whether they support collector-free release lowering.
3. A capable full-control release backend must not silently fall back to a tracing or reachability collector.
4. A missing physical memory strategy after successful topology validation is `CompilerError`.
5. Debug and development profiles may deliberately use GC for simpler lowering, faster compilation and instrumentation.
6. GC-native backends may use their host collector.
7. Debug, GC-native and collector-free paths preserve identical observable Moth behavior.
8. The guarantee is specifically no tracing or reachability collector. REC is a distinct selective retained-edge strategy and not the semantic correctness fallback.

### 2.3 Borrow and last-use analysis

1. Last-use analysis is central to memory precision.
2. Alias activity is path-sensitive and non-lexical.
3. Branches are analyzed independently.
4. Loops use fixed-point future-use reasoning.
5. A transfer is legal only when every relevant path proves that the transferred value has no later source use.
6. When optional transfer is not proven, the operation remains a borrow and the program remains valid.
7. Last-use facts may move affine cleanup responsibility through calls, returns, aggregate insertion, container detachment and control-flow paths.
8. Loop forms may expose a final-iteration fact only when finality is known without changing source evaluation order.
9. Borrow validation does not assign semantic lifetime owners and does not choose a physical memory strategy.

### 2.4 Semantic lifetime topology

1. Every runtime allocation family has exactly one semantic lifetime owner.
2. A retained edge from container `C` to value `V` is legal only when `V` belongs to the same region or a region that outlives `C`.
3. Implicit lifetime regions are compiler-generated non-lexical CFG intervals, not merely lexical scopes.
4. The compiler starts with the narrowest valid candidate owner.
5. Widening may follow only the nearest existing ancestor on one ordered lifetime chain.
6. The compiler must not promote laterally across independently ending sibling lifetimes.
7. The compiler must not invent a page, application or process owner merely to avoid a diagnostic.
8. Shared sibling persistence requires a real common lifecycle owner, an explicit group or an independent copy.
9. Projections remain rooted in their allocation family until field-sensitive splitting proves an independent family.
10. Cross-region cycles are invalid.

### 2.5 Result provenance and hidden destinations

Static result provenance remains separate from runtime cleanup responsibility.

The final summary vocabulary must distinguish at least:

- fresh result root
- alias of one or more parameters
- projection of a parameter
- detached stored result
- alias of another result
- independent result graph
- retained-parameter and outlives constraints
- retention cardinality
- persistent-edge creation and destruction effects
- whole-domain kill effects
- exit-specific retained-edge effects and frontier-enabling effects

A fresh result root may be allocated directly into a caller-selected hidden destination when every retained edge is legal for that destination.

The hidden destination:

- is not source syntax
- is not a source lifetime parameter
- does not enter `TypeId`
- may select affine, inferred-region, explicit-group or REC layout
- may be ignored physically by a GC backend

A final successful public or generated summary must not leave topology-relevant result provenance unknown.

### 2.6 Retained-edge liveness and cleanup frontiers

1. Lifetime inference tracks retained-edge liveness separately from aggregate binding lifetime.
2. A final cleanup frontier is a CFG point after which every relevant retained edge into an allocation family or inferred region is proven gone.
3. A collection or aggregate may continue to live after its retained-value region ends.
4. Frontiers may be path-sensitive and may have different exits on different branches.
5. Repeated population and whole-domain cleanup may form compiler-generated region epochs.
6. Initial whole-domain kill primitives are:
   - collection or map `clear`
   - aggregate destruction
   - whole-value replacement when old contents are definitely discarded
   - explicit group exit
   - builder lifecycle teardown
7. Individual `remove`, `set` or partial cleanup does not establish uniqueness by itself.
8. Uniqueness scans, alias registries and adaptive owner recovery are rejected from the baseline design.
9. Retained-edge liveness is part of lifetime topology, not a separate runtime ownership system.

### 2.7 Affine Ownership ABI

1. Cleanup responsibility is affine.
2. Cleanup responsibility may move or be discharged. It never duplicates.
2a. **Transfer**, **discharge**, **destroy** and **bulk reclaim** are four different things and are never used as synonyms. Transfer moves the obligation to another path. Discharge satisfies it. Destroy physically destroys one allocation family. Bulk reclaim reclaims a region or group.
2b. Discharging an affine root destroys the family only when the plan makes that family individually destructible and nothing else keeps it alive. For an REC family, discharge removes one affine-root obligation and decrements the count; destruction happens only if the count reaches zero. For a region- or group-owned family, discharge never destroys anything individually.
3. Semantic lifetime ownership remains static even when cleanup responsibility moves at runtime.
4. Runtime owned or borrowed state answers who may perform individual cleanup on the current path.
5. Static result provenance answers where storage came from and which lifetime constraints apply.
6. Region-owned and group-owned allocation-family mode is static memory-plan metadata.
7. Simple `AlwaysConsumes` and `NeverConsumes` cases may be specialized.
8. Mixed call paths use one function body with local tag handling around retention-sensitive or cleanup-sensitive operations.
9. The current direct path from borrow-checker advisory drop sites to Wasm `DropIfOwned` is scaffolding. Final lowering must consume a validated memory plan instead.

### 2.8 Two-bit full-control handle contract

The detailed contract lives in the REC companion plan.

The parent model locks only these facts:

```text
bit 0: affine cleanup responsibility
bit 1: REC representation
```

```text
00: uncounted borrowed
01: uncounted affine-owned
10: REC-managed borrowed or counted persistent edge
11: REC-managed affine owner
```

The tags belong to an allocation-family handle. A projection must retain or recover the family base. Scalars and target-native immediate values do not use this handle ABI.

### 2.9 Explicit groups and cycles

1. A declared group is one hard semantic lifetime and one bulk cleanup domain.
2. Direct source-created reference cycles are legal only inside one declared group.
3. Implicit lifetime inference never invents a cyclic region.
4. Group-owned allocations never use REC.
5. Group-owned allocations never transfer individual cleanup responsibility.
6. Last-use analysis still validates access and alias legality inside groups.
7. Last-use analysis does not shorten the physical lifetime of group-owned storage.
8. Group exit performs bulk cleanup.
9. An overly broad declared group is a visible programmer choice.
10. Field-sensitive splitting may improve layout inside a group but must not shorten group-owned lifetime.
11. Group-owned values do not cross channel boundaries as individually transferable values.
12. Group ownership is a property of the **target** allocation, not of every edge whose source lives inside a group. A group-owned target carries no counter; an edge from group storage to an externally owned REC family is an ordinary counted persistent edge into that external family.
13. At `clear()` or group exit, outgoing REC boundary obligations are released **before** group storage is bulk reclaimed. The external target is destroyed only if its own count reaches zero.

### 2.10 Retained Edge Counting

REC is accepted as a narrow physical precision strategy.

The parent plan owns when REC participates in the final architecture. The companion plan owns exact analysis and lowering.

REC applies only when:

- one allocation family has runtime-dependent persistent retained-edge multiplicity
- those edges disappear independently
- static affine cleanup cannot identify the final obligation
- no complete cleanup frontier gives a precise enough region
- explicit-group ownership does not apply
- field-sensitive splitting and cheap bounded retention do not remove the problem

REC does not count:

- local aliases
- parameters
- temporary projections
- ordinary call borrows
- `get()` results
- affine transfers
- explicit-group-owned edges

REC obligations are per target allocation family. For a family `F`, the count is the number of live counted persistent-edge obligations into `F` plus at most one affine-root obligation for `F`. Every retention-sensitive commit is evaluated independently per family as:

```text
delta_count(F) =
    created_persistent_edges(F)
    - removed_persistent_edges(F)
    + affine_root_after(F)
    - affine_root_before(F)
```

One affine root can reclassify into at most one new persistent edge, and at most one removed edge can reclassify into a returned affine root. One storage operation may therefore create several direct edges into one family and still change the count. A same-family overwrite commits one net delta atomically; there is no transient-zero destruction step.

Counts follow the **direct** post-refinement family-edge graph, never transitive reachability. An allocation reachable only through a separately allocated child is never counted again by the container.

The exact transition table, tag encoding and lowering contract live in the REC companion plan.

REC never establishes legality and never permits cycles.

### 2.11 Builtin collections as the trusted dynamic-storage substrate

Builtin fixed collections, growable collections and maps form the trusted dynamic-storage vocabulary.

Their compiler-known effects are:

| Operation | Memory effect |
|---|---|
| `get` | creates a temporary shared alias and adds no persistent obligation |
| `push` | adds the inserted element's retained-edge obligations |
| `set` | removes the replaced element's obligations and adds the new element's obligations |
| `remove` | removes the stored element's obligations and returns a detached stored result |
| `clear` | removes the obligations contributed by every stored element |
| collection destruction | removes all element obligations and destroys the backing-storage domain |
| growth or reallocation | changes backing storage without changing logical element summaries |

Fixed and growable collections must gain a compiler-owned `clear()` operation in the accepted final surface.

The table describes the successful path. A failed builtin mutation preserves the original storage topology and cleanup obligations, and public summaries preserve separate success and error effects. A stored scalar may contribute zero obligations, a direct heap value one, and an inline aggregate that physically stores several handles several direct obligations. A summary may also describe nested retention, but obligations count direct edges between final allocation families: an allocation reachable only through a separately allocated child is never counted again by the container.

User-defined collections and storage abstractions compose these builtin effects. Their semantic summaries are inferred. The compiler does not grant effects based on method names and source annotations are not added.

Keeping builtin destructive operations narrow is an intentional language-design requirement. Future collection APIs must preserve analyzable whole-domain and single-edge effects rather than adding broad mutable entry protocols that hide retention changes.

### 2.12 Field-sensitive allocation splitting

1. Field-sensitive allocation-family splitting is required final architecture.
2. It may land after the first collector-free release implementation.
3. Mandatory source legality never depends on splitting. The unsplit topology must already be valid.
4. Splitting is a physical refinement, so it runs after build-owned target partition and target-contract validation, once a candidate physical variant exists, and before final memory-strategy selection for that variant.
5. A split rebuilds the affected direct family-edge graph and revalidates the affected outlives, SCC and family-base invariants.
6. A refinement that cannot be proven falls back to the unsplit legal family and conservative retention. It never produces a source diagnostic.
7. It may separate a small long-lived field from a large short-lived parent family.
8. It must preserve source alias, mutation and copy semantics.
9. It does not add partial-move syntax or observable allocation identity.
10. Strategy selection reruns for every resulting family, per physical variant.
11. REC cannot substitute for splitting an unacceptably broad family.

### 2.13 Physical memory planning and coalescing

1. Semantic lifetime inference remains maximally precise.
2. Physical memory planning is target-aware and profile-aware. It runs per physical variant, after target partition and target validation, and produces one `ValidatedMemoryPlan` per variant.
3. The memory planner remains compiler-owned. The build system owns target partition, physical-variant orchestration, the build profile and target/backend capability metadata, and invokes the planner per candidate variant.
4. The backend only realises the plan. It never selects a physical memory strategy.
5. Physical memory planning may deliberately retain values slightly longer to reduce allocation and cleanup overhead.
3. Physical coalescing is not semantic region widening.
4. The first coalescing heuristic is deliberately narrow:
   - straight-line control flow
   - one common proven enclosing region
   - no intervening function call
   - no intervening effectful or potentially expensive operation
5. Compatible implicit intervals may become one synthetic arena or grouped drop.
6. Explicit groups remain separate and keep their hard bulk lifetime.
7. More advanced heuristics require benchmark evidence.

### 2.14 Channels and future async

1. Channels remain deferred until the memory model is implemented and hardened.
2. Channel send is a mandatory affine cleanup-responsibility transfer boundary for ordinary heap graphs.
3. A successful send moves responsibility to the channel or receiver lifecycle.
4. A failed send preserves or returns responsibility to the sender.
5. Queued values belong to a statically bounded channel or task lifecycle.
6. Arbitrary shared Moth reference graphs do not cross task boundaries.
7. Group-owned values do not cross independently and require an independent copy or fresh message.
8. REC remains non-atomic under the accepted task model.
9. A future requirement for atomic REC is a new memory-model design decision.
10. The async and channel design must fit this memory model. The memory model must not be weakened to fit async.

### 2.15 External boundaries and observable cleanup

1. Ordinary foreign code cannot retain references into ordinary Moth storage.
2. WIT V1 remains value-only.
3. Restricted host bindings remain non-retaining for ordinary Moth values.
4. REC does not create a foreign retain and release protocol.
5. Observable external resources use explicit close or teardown operations.
6. Object cleanup timing is not source-observable.
7. This permits debug GC and release static cleanup to preserve identical behavior.

---

## 3. The six cooperating memory mechanisms

| Mechanism | Owns | Does not own |
|---|---|---|
| Borrow and last-use analysis | access conflicts, alias activity, final potential uses, optional affine transfer | lifetime owner assignment, REC selection, physical drops |
| Lifetime topology | allocation families, retained edges, outlives constraints, escapes, non-lexical intervals, cleanup frontiers, lifecycle roots | runtime ownership tags, allocator layout, target lowering |
| Affine Ownership ABI | the runtime path that may perform individual cleanup | lifetime legality, result provenance, alias counting |
| Explicit groups | hard source-declared bulk lifetimes, count-free cycles, group exits | individual early cleanup, REC, dynamic sharing outside the group |
| Retained Edge Counting | unresolved runtime-many persistent retained-edge obligations | ordinary aliases, legality, cycles, source types |
| Physical memory planning | stack placement, affine heap cleanup, inferred arenas, explicit-group arenas, REC layouts, drop coalescing | source validity, borrow rules, lifetime topology |

The mechanisms are ordered. A later mechanism consumes validated facts from earlier owners and must not reconstruct them.

---

## 4. Collector-free correctness argument

For every accepted allocation family `A`:

1. `A` has exactly one statically proven semantic lifetime owner `R`.
2. Every retained edge into `A` is legal under the outlives relation.
3. Borrow validation prevents conflicting access and use after affine transfer.
4. Last-use analysis may move or discharge individual cleanup responsibility earlier.
5. A complete cleanup frontier may end `R` before the retaining aggregate itself dies.
6. Runtime-many independent persistent edges may use REC for earlier cleanup.
7. Explicit-group-owned values remain until their one bulk exit.
8. If no earlier strategy applies, `A` remains alive until `R` ends.
9. When `R` ends, no surviving legal value can retain an edge into `R`.

Therefore a full-control backend can reclaim every remaining allocation in `R` without tracing runtime reachability.

The proof guarantees collector-free correctness. Memory quality depends on interval precision, cleanup frontiers, affine transfer, REC selection, field splitting and physical region planning.

---

## 5. Compiler and build-system ownership

The final pipeline is:

```text
validated HIR
-> borrow and last-use analysis
-> local allocation-family and retained-edge constraints
-> exported lifetime and retention summaries
-> project/package summary instantiation
-> complete backend-neutral lifetime-topology validation
-> non-lexical interval, frontier and epoch completion
-> backend-neutral memory requirements
-> target-affinity analysis and partition
-> target-contract validation
-> per-physical-variant family/layout refinement
-> revalidate affected refined family-edge facts
-> target/profile-aware compiler-owned memory planning
-> ValidatedMemoryPlan
-> backend lowering
-> collector-free artefact verification where required
```

Everything through `backend-neutral memory requirements` is target-independent and shared. Everything after it is scoped to one target/profile physical variant.

`check` runs through creation and validation of the `ValidatedMemoryPlan` and stops before backend lowering and output emission.

### AST

AST owns:

- source access syntax
- `copy`
- group and `into` syntax
- group scope and destination rules
- obvious freshness and placement diagnostics
- no lifetime or REC type identity

### HIR

HIR owns:

- explicit places and CFG
- calls, returns and aggregate operations
- group declarations and exits when implemented
- retained source operations in backend-neutral form

HIR does not own exact lifetime regions, REC strategy or physical allocation.

### Borrow validation

Borrow validation owns:

- shared and exclusive conflict checks
- root and alias activity
- path-sensitive future use
- optional affine transfer eligibility
- last-use facts
- temporary alias safety
- reactive invalidation facts

Borrow validation writes side tables and does not rewrite HIR.

### Lifetime and retained-edge analysis

A new backend-neutral analysis owner after borrow validation owns:

- allocation-family identity
- complete result provenance
- persistent retained-edge creation and destruction
- retention domains
- retention cardinality
- detached stored-result classification
- whole-domain kill effects
- outcome-sensitive success and error effects
- retained-edge liveness
- cleanup-frontier candidates
- frontier-enabling effects for public summaries. Concrete cleanup frontiers remain caller and link-level facts
- local escape and outlives constraints
- local SCC and cycle facts
- exported lifetime and retention summaries

### Project and link topology

Build and link planning owns:

- instantiating summaries over reachable calls
- builder lifecycle roots
- complete topology validation
- cross-module and cross-package outlives relationships
- final source diagnostics for unprovable topology
- one validated topology for target planning

### Memory-strategy planning

The compiler-owned memory planner is invoked once per candidate physical variant, after build-owned target partition and target-contract validation. It consumes validated topology, the backend-neutral memory requirements, the selected target, the build profile and backend memory capability metadata, and owns:

- `Affine`
- `InferredRegion`
- `ExplicitGroup`
- `Rec`
- hidden destination plans
- allocation-family layout requirements
- cleanup plans
- physical coalescing candidates
- developer decision records

Strategy planning runs only after topology is valid **and** after target partition and target validation have established the physical variant. It produces one `ValidatedMemoryPlan` per variant, and never affects source legality.

### Backends

Backends own:

- concrete handle representation
- allocation and region runtime
- affine `drop_if_owned`
- region and group release
- REC operations from the companion plan
- stack or arena placement
- collector-free artifact verification

Backends do not reconsider legality and do not select a memory strategy. They realise the `ValidatedMemoryPlan` for the variant they are lowering.

---

## 6. Plan boundaries

### This parent plan owns

- source memory semantics
- borrow and last-use requirements
- allocation families
- result provenance
- retained-edge liveness
- cleanup frontiers
- compiler-generated lifetime intervals and epochs
- lifetime topology and diagnostics
- explicit groups and cycles
- field-sensitive splitting
- Affine Ownership ABI semantics
- memory-strategy planning
- physical coalescing
- builder lifecycles
- channel prerequisites
- backend/profile parity
- collector-free release capability
- repository-wide documentation and status migration

### The REC companion plan owns

- dynamic retained multiplicity analysis details
- REC eligibility and elision details
- the exact two-bit handle ABI
- inline counter layout
- counter transitions
- detached stored-result reclassification
- function and package boundary lowering
- builtin collection REC integration
- iterative destruction
- REC developer reporting
- REC-specific tests and closeout

The parent plan links to the companion plan. It must not duplicate the companion's implementation phases.

---

## 7. Current repository state and migration debt

Baseline observations at `34afc996b746bfe93281dad115c40083a9106ac8`:

| Area | Current state | Required migration |
|---|---|---|
| Borrow checker | Implemented root tracking, path-sensitive future use, optional transfer facts and advisory drop sites | Preserve and harden last-use analysis, enrich facts and stop treating advisory drop sites as final lowering authority |
| Public call summaries | `Fresh`, `AliasParams` and `Unknown` plus access and transfer effects | Add full result provenance, retained parameters, detached stored results, outlives, domain kills and final-summary completeness |
| Lifetime topology | Accepted design only, no implementation owner | Add local constraint analysis and project/link topology validation |
| Retained-edge liveness | Not implemented | Add domains, edge creation and kills, cleanup frontiers and epochs |
| Declared groups | Accepted syntax and semantics only | Add AST, HIR, topology and backend bulk cleanup |
| Wasm ownership lowering | Borrow advisory sites directly emit `DropIfOwned` under transitional garbage-collected scaffolding | Replace with memory-plan-driven affine, region, group and REC lowering |
| Wasm LIR | Has `DropIfOwned` and a reserved retain instruction | Extend around the final memory plan and remove obsolete single-tag assumptions |
| Wasm runtime memory | Only basic linear-memory page and heap-base planning | Add allocator, region, group, REC and destruction-plan runtime support |
| REC | Accepted companion plan, no implementation | Execute only after parent topology and strategy prerequisites |
| Collections | Maps have `clear`, fixed and growable collections expose five operations without `clear` | Add compiler-owned `clear` and trusted retention effects |
| Progress matrix | Tracks deferred lifetime and group implementation separately from the accepted design | Keep accepted design and current implementation status separate |
| Roadmap | Still contains superseded collector-first sequencing | Replace with the collector-free plan and correct sequencing |
| Async draft | Says channel send may move or pass and leaves memory transfer open | Lock mandatory affine transfer and memory prerequisites while keeping async deferred |
| Canonical docs | Repeatedly permit release GC fallback and describe single-tag ownership only | Rewrite to the accepted final model |
| README and cheatsheet | Describe static topology proof and collector-free release direction | Explain selective REC without exposing it as source semantics |

Current scaffolding must be replaced in place. Do not add compatibility adapters, parallel memory plans or a second backend ownership path.

**Documentation migration status.** The documentation rows of this table are complete as of Milestone A. The canonical memory pages, compiler and build authorities, design-scope pages, collection references, cheatsheet, async draft, `README.md`, `AGENTS.md`, the roadmap and the progress matrix now describe the accepted collector-free model. Every implementation row remains open, and the progress matrix records each one separately.

---

## 8. Delivery milestones

### Milestone A: accepted design migration — complete

The canonical docs, roadmap and progress matrix describe the final model while clearly marking implementation as deferred or partial. Review reopened Phase 1 for a focused consistency closure pass, and the pass now completes the milestone. No compiler or backend behaviour changed in this milestone.

### Milestone B: collector-free correctness baseline

A full-control backend can lower accepted acyclic programs without tracing GC through affine cleanup and conservative inferred regions. REC and field-sensitive splitting are not required for correctness.

### Milestone C: explicit-group and lifecycle baseline

Declared groups, group-only cycles and builder lifecycle regions are implemented.

### Milestone D: precision baseline

Cleanup frontiers, region epochs, physical coalescing and the REC companion implementation reduce conservative retention.

### Milestone E: final optimization architecture

Field-sensitive splitting, mature decision reporting and representative real-code benchmarking complete the intended final model.

Progress-matrix rows must advance independently. Do not mark the whole model complete because one milestone lands.

---

# Implementation plan

Each phase is a stable implementation slice. Every phase ends with an architecture, style, test and validation review before the next phase starts.

## Phase 0: Establish authority and repository baseline

### Summary and reasoning

The repository currently has a completed historical memory documentation plan, an outdated grouped-memory roadmap and the new REC companion plan. The first slice must create one unambiguous plan graph.

### Tasks

- [x] Add this plan at the proposed repository path.
- [x] Record the activation branch and exact activation HEAD.
- [x] Confirm the REC companion plan remains the sole detailed REC owner.
- [x] Mark `grouped-memory-design.md` as superseded.
- [x] Move any still-useful group implementation tasks into this plan.
- [x] Delete the old grouped plan after every live link is migrated. Every live link now points at this plan; no historical stub was retained.
- [x] Add this plan to `docs/roadmap/roadmap.md`. The documentation slice ran under active work; every implementation phase stays under deferred design until explicitly activated.
- [x] Remove the roadmap statement that ownership optimisation is deferred until after the superseded collector-first model.
- [x] Add links between this plan and the REC companion plan.
- [x] Inventory current source, tests, docs and generated output before the first implementation slice.
- [x] Preserve unrelated work.

### Audit and validation

- [x] One parent memory roadmap exists.
- [x] One detailed REC roadmap exists.
- [x] Canonical docs remain semantic authorities.
- [x] No roadmap plan silently overrides a canonical page.
- [x] No implementation work starts in this phase.
- [x] Run the documentation-only release-build gate.

---

## Phase 1: Migrate canonical documentation to the final model

### Summary and reasoning

Accepted design must be authoritative before implementation starts. This phase deliberately documents the end state while the progress matrix records current gaps. Review reopened Phase 1 for two consistency closure passes, which now complete the migration without changing compiler behaviour.

### Memory authority tasks

- [x] Rewrite `docs/src/developer-docs/memory-management/overview.mtf` around the six cooperating mechanisms.
- [x] Replace the two-proof-layer GC fallback wording with mandatory topology plus collector-free strategy selection.
- [x] Add retained-edge liveness, cleanup frontiers, inferred intervals, region epochs and REC's narrow role.
- [x] Update the memory task-reading guide with the REC canonical page and the correct route for collection retention work.
- [x] Rewrite `docs/src/developer-docs/memory-management/@page.moth` with the concise final summary and detailed-page routing.
- [x] Update `access-and-aliasing` with last-use centrality, allocation-family meaning and the static provenance versus runtime responsibility distinction.
- [x] Update `borrow-validation` with the final last-use contract and explicit handoff to retained-edge analysis.
- [x] Update `lifetime-regions-and-escape-validation` with non-lexical intervals, retained-edge liveness, cleanup frontiers, epochs, group-only cycles and REC eligibility.
- [x] Update `declared-memory-groups` with hard count-free lifetimes, no individual early cleanup and the only-source-cycle rule.
- [x] Rename the ownership page title to `Affine ownership and drops`. The existing `ownership-and-drops/` path is kept; no file move was needed.
- [x] Update `ownership-and-drops` with affine cleanup responsibility, memory-plan ownership and the two-bit logical extension.
- [x] Update `runtime-and-backend-lowering` with the collector-free release invariant and all physical strategies.
- [x] Restrict copied cyclic graphs to one explicit declared group across access, copy and lifetime authorities.
- [x] Make physical coalescing distinct from semantic lifetime widening.
- [x] Add the canonical REC documentation directory and page required by the companion plan.
- [x] Route the REC page from the memory index.

### Compiler and build authority tasks

- [x] Update `docs/compiler-design-overview.md` with the retained-edge analysis owner, final summary vocabulary, memory-strategy planner and backend handoff.
- [x] Update `docs/build-system-design.md` with lifecycle-root instantiation, memory strategy plans, backend capability metadata and collector-free verification.
- [x] Update public-interface and fingerprint descriptions with retention, detached stored-result effects, cardinality, whole-domain kills, outcome-sensitive effects and frontier-enabling effects.
- [x] Keep donor-local family and region IDs out of interfaces.

### Language and design-scope tasks

- [x] Update `docs/src/docs/design-scope/design-principles.mtf` so memory safety is no longer described as a GC baseline.
- [x] Update the excluded-language inventory with no source REC, no implicit cyclic regions and no backend-specific legality.
- [x] Update fixed and growable collection references with `clear()` and compiler-known retained-edge effects. `clear` is documented as an accepted sixth operation with implementation deferred: only maps expose it today.
- [x] Update map and collection docs with the trusted dynamic-storage role.
- [x] State that future collection APIs must preserve narrow analyzable destruction effects.
- [x] Generalise collection and map effects to value-shaped retained-edge summaries, including key/value replacement and detached stored results.
- [x] Document atomic successful commits and unchanged error paths for fallible builtin mutations.
- [x] Keep concrete cleanup frontiers caller-local while exporting exit-specific and frontier-enabling effects.
- [x] Add the public Automatic cleanup and retained edges page pair and route it between lifetime and group teaching.
- [x] Remove educational diagram placeholders from the memory and borrow pages and qualify current Wasm and historical GC terminology.
- [x] Update the language cheatsheet with a concise user-facing collector-free release statement.
- [x] Do not expose REC counters, tags or regions as source types in the cheatsheet.
- [x] Update the async draft with mandatory send transfer, channel-owned queued values, group restrictions and non-atomic REC prerequisites.
- [x] Keep async status explicitly deferred.
- [x] Update `README.md` from optional GC avoidance to the accepted collector-free release direction.
- [x] Update `AGENTS.md` core memory contracts and routing only where the new REC page or release invariant needs explicit mention.

### Roadmap and historical-plan tasks

- [x] Update `docs/roadmap/roadmap.md` with this parent plan and the REC companion.
- [x] Remove the superseded collector-first direction as the accepted model.
- [x] Confirm `final-memory-management-documentation-consistency-cleanup-plan.md` no longer exists in the repository and record that no historical annotation is required.
- [x] Remove or supersede stale collector-elision wording in other roadmap plans, and add a historical banner to `docs/wasm-notes/future-wasm-components-report.md`, which described the old model as collector-first.

### Progress-matrix tasks

Create or update separate rows for:

- [x] borrow validation and local last-use analysis
- [x] result provenance and allocation-family summaries
- [x] lifetime-region and escape validation
- [x] retained-edge liveness and cleanup frontiers
- [x] compiler-generated intervals and region epochs
- [x] declared groups and group-only cycles
- [x] Affine Ownership ABI
- [x] memory-strategy planning
- [x] REC analysis and selection
- [x] REC backend lowering
- [x] Physical coalescing
- [x] Builder lifecycle regions
- [x] Channels
- [x] field-sensitive allocation splitting
- [x] inferred-region and group backend lowering
- [x] collector-free release verification
- [x] debug or GC-native representation parity

Use these initial status rules:

- current borrow validation and local last-use analysis: `Supported` or `Partial` with exact limitations
- current Wasm ownership scaffolding: `Experimental`
- lifetime topology, frontiers, groups, REC, field splitting and collector-free verification: `Deferred`
- current GC fallback paths: implementation migration debt, not accepted final behavior

### Audit and validation

- [x] No final authority calls GC the semantic correctness baseline.
- [x] No final authority says capable release builds may silently fall back to tracing GC.
- [x] No user-facing page exposes REC as a source memory type.
- [x] No page permits source-created cycles outside explicit groups.
- [x] No page permits individual early cleanup of group-owned storage.
- [x] All links form one clear authority chain.
- [x] Run the documentation-only release-build gate and inspect every changed route.

---

## Phase 2: Harden borrow and last-use analysis

### Summary and reasoning

Last-use precision is the primary early-reclamation mechanism. This phase strengthens the existing checker without assigning lifetime topology or physical memory.

### Tasks

- [ ] Confirm path-dependent optional transfer always falls back to borrowing rather than rejecting valid source.
- [ ] Preserve branch-sensitive last use and fixed-point loop reasoning.
- [ ] Add explicit last-use facts for aggregate insertion, field storage, returns and container-detachment sites.
- [ ] Add result-to-result last-use handling for multiple aliased returns.
- [ ] Track projection use through the containing allocation family.
- [ ] Expose final-iteration facts for collection and finite range loops when finality is knowable without changing evaluation order.
- [ ] Keep conditional loops conservative when finality is only known after body execution.
- [ ] Replace backend-facing advisory drop sites with backend-neutral last-use and affine-transfer candidates.
- [ ] Keep temporary `get()` aliases count-free and ensure they block mutation or final cleanup until their last use.
- [ ] Extend structured borrow reports with last-use and transfer decisions useful to later analyses.
- [ ] Delete any path that treats lack of optional transfer as a source error.
- [ ] Do not rewrite HIR.

### Test requirements

- [ ] final-use call in straight-line code
- [ ] branch-specific final owner
- [ ] loop-carried alias
- [ ] collection loop final iteration
- [ ] empty collection loop exit cleanup
- [ ] conditional loop with unknown final iteration
- [ ] error return and `break` paths
- [ ] multiple returned aliases
- [ ] projection family transfer
- [ ] live `get()` alias blocking cleanup

### Audit and validation

- [ ] Borrow validation owns access and last use only.
- [ ] No lifetime region or REC strategy enters borrow state.
- [ ] Every optional transfer has a no-later-use proof.
- [ ] Every imprecise case remains a borrow.
- [ ] Tests live under the borrow-checker owner or canonical integration suite.
- [ ] Run `just validate`.

---

## Phase 3: Introduce complete semantic lifetime and retention summaries

### Summary and reasoning

Cross-function and cross-package topology cannot be inferred from the current `Fresh`, `AliasParams` and `Unknown` summary alone.

### Tasks

- [ ] Replace the limited return alias vocabulary with the final result provenance categories.
- [ ] Add allocation-family identity for local facts.
- [ ] Add retained-parameter and retained-receiver facts.
- [ ] Add outlives constraints.
- [ ] Add detached stored results and distinguish them from group extraction and interior projection detachment.
- [ ] Add result-to-result family aliasing.
- [ ] Add complete retention-domain kill facts.
- [ ] Add static retention cardinality required by the REC companion.
- [ ] Add external-boundary classification to the same semantic contract.
- [ ] Extend generated-function sidecars with identical summary vocabulary.
- [ ] Extend public-interface fingerprints.
- [ ] Permit `Unknown` only as a transient fixed-point state.
- [ ] Reject a final public or generated summary that remains topology-relevant `Unknown`.
- [ ] Keep local IDs out of exported interfaces.
- [ ] Validate summary widening through one explicit finite order.

### Audit and validation

- [ ] One summary owner exists.
- [ ] Consumers never reopen provider HIR.
- [ ] Summary changes invalidate semantic consumers.
- [ ] Private body changes that preserve summaries do not.
- [ ] No physical REC or region strategy enters the public semantic interface.
- [ ] Run `just validate`.

---

## Phase 4: Implement local allocation-family and retained-edge analysis

### Summary and reasoning

This phase creates the backend-neutral constraint system that follows borrow validation.

### Tasks

- [ ] Add a dedicated lifetime and retained-edge analysis module under the compiler analysis owner.
- [ ] Identify fresh allocation roots and allocation families.
- [ ] Keep projections attached to their base family.
- [ ] Record persistent retained-edge creation and destruction.
- [ ] Distinguish temporary aliases from persistent retained edges.
- [ ] Record retention domains for structs, choices, collections, maps and reactive state.
- [ ] Produce local escape and outlives constraints.
- [ ] Produce local SCC and cycle facts.
- [ ] Produce cleanup-frontier candidates without deciding final intervals.
- [ ] Produce exported summaries.
- [ ] Use immutable side tables keyed by HIR identity.
- [ ] Keep user-facing local topology diagnostics structured.
- [ ] Treat malformed HIR or inconsistent summaries as `CompilerError`.

### Audit and validation

- [ ] HIR remains unchanged.
- [ ] Borrow facts are consumed, not reconstructed.
- [ ] Source syntax is not re-read.
- [ ] Physical strategy is not selected.
- [ ] Allocation-family identity is distinct from `TypeId`.
- [ ] Run `just validate`.

---

## Phase 5: Implement project and link lifetime-topology validation

### Summary and reasoning

Local modules cannot prove cross-module calls, builder lifecycles or package roots alone.

### Tasks

- [ ] Instantiate local summaries over the exact reachable call graph.
- [ ] Add builder page, mount, request, render and frame lifecycle roots as explicit inputs.
- [ ] Assign one semantic lifetime owner to every reachable allocation family.
- [ ] Solve retained-edge outlives constraints.
- [ ] Widen only to the nearest existing ancestor on one ordered owner chain.
- [ ] Reject lateral promotion across independent siblings.
- [ ] Reject unowned escapes.
- [ ] Reject retained references into shorter-lived storage.
- [ ] Reject implicit source reference cycles.
- [ ] Distinguish invalid topology from topology not proven by conservative analysis.
- [ ] Produce ranked remedies:
  1. allocate directly into the destination
  2. use one common explicit group
  3. use `copy`
  4. shorten the retained edge
  5. repair external metadata
- [ ] Store the validated topology in project or link planning.
- [ ] Make topology validation mandatory for `check`, debug and release.

### Audit and validation

- [ ] Backend choice cannot affect validity.
- [ ] GC paths do not bypass the solver.
- [ ] No arbitrary page or process owner is invented.
- [ ] Cross-package summaries use stable identities.
- [ ] Diagnostics identify both retention source and failed lifetime relationship.
- [ ] Run `just validate`.

---

## Phase 6: Implement non-lexical intervals, retained-edge liveness and cleanup frontiers

### Summary and reasoning

A safe topology can still retain too much memory. This phase creates precise compiler-generated lifetimes before REC is considered.

### Collection surface tasks

- [ ] Add compiler-owned `clear()` to fixed collections.
- [ ] Add compiler-owned `clear()` to growable collections.
- [ ] Keep map `clear()` as a whole-domain kill.
- [ ] Define compiler-known effects for `get`, `push`, `set`, `remove`, `clear`, destruction and growth.
- [ ] Infer equivalent summaries through user wrappers.
- [ ] Do not recognize user methods by name.
- [ ] Keep future builtin destructive APIs narrow and analyzable.

### Analysis tasks

- [ ] Track retained-edge liveness separately from aggregate binding lifetime.
- [ ] Complete final cleanup frontiers only when every relevant edge is gone on that path.
- [ ] Prove that no surviving source alias can recreate a killed edge.
- [ ] Keep live local and projection aliases in the frontier proof.
- [ ] Support branch-specific frontiers.
- [ ] Support aggregate destruction and whole-value replacement.
- [ ] Support repeated population and cleanup as region epochs.
- [ ] Keep individual `remove` and partial cleanup as one-edge effects.
- [ ] Reject uniqueness scans and alias registries.
- [ ] Permit a collection object and backing capacity to outlive its retained-value region.
- [ ] Emit structured interval and frontier debug reports.

### Audit and validation

- [ ] `clear()` can end an element region while the collection remains alive.
- [ ] A live `get()` alias delays the frontier.
- [ ] Frontiers are based on effects, not method names.
- [ ] Every frontier is valid on every represented path.
- [ ] Region epochs do not change collection semantics.
- [ ] Run `just validate`.

---

## Phase 7: Implement declared-group frontend and HIR contracts

### Summary and reasoning

Groups are the explicit source escape hatch for deliberate shared lifetimes and cycles.

### Tasks

- [ ] Implement `group name:` parsing.
- [ ] Implement declaration-site `into group` placement.
- [ ] Enforce current and ancestor placement only.
- [ ] Enforce destination-scope visibility and collision rules.
- [ ] Reject conditional or repeatable ancestor declarations under the accepted V1 rule.
- [ ] Require fresh result roots, independent graphs or legal same-group transfer.
- [ ] Keep group identity outside `TypeId`, signatures and generics.
- [ ] Add HIR group identity, placement and exit metadata.
- [ ] Record every fallthrough, return, error, break and recovery exit.
- [ ] Reject group values in constants, config, fields, signatures and exports.
- [ ] Keep source-created cycles invalid until all members are placed into one explicit group.

HIR group metadata has this conceptual shape. Group identity must not enter `TypeId`.

```rust
pub struct HirMemoryGroup {
    pub id: MemoryGroupId,
    pub name: StringId,
    pub owner_region: RegionId,
    pub parent_group: Option<MemoryGroupId>,
    pub source_location: SourceLocation,
}

pub struct HirPlacement {
    pub group: MemoryGroupId,
    pub source_location: SourceLocation,
}
```

Conditional production uses one declaration in the destination scope whose initializer is a
value-producing `if`, match or `catch`. Loop production mutates a destination-owned aggregate
rather than repeatedly declaring an ancestor-owned name.

### Group and topology diagnostic coverage

Lifetime and group diagnostics are part of the memory model. They must distinguish topology proven
invalid, topology not proven legal by conservative analysis, invalid group syntax or placement,
non-copyable graph contents, unsupported external boundary profiles, and missing or inconsistent
compiler-owned metadata. User-facing failures use stable codes and structured reason payloads.
Internal impossible or inconsistent metadata uses `CompilerError`.

- [ ] invalid group name or position
- [ ] non-fresh placement
- [ ] alias result placement
- [ ] return or projection escape
- [ ] store escape
- [ ] nested escape
- [ ] cross-region cycle
- [ ] live alias at exit
- [ ] reactive escape
- [ ] external retention
- [ ] missing common owner

Diagnostics must present this remedy order:

1. allocate directly into the required destination region
2. place observers under one common group
3. create independent storage with `copy`
4. shorten the alias or retained edge
5. repair package-owned external lifetime metadata

There is no backend-specific escape from semantic lifetime diagnostics.

### Audit and validation

- [ ] AST owns syntax and obvious placement diagnostics.
- [ ] HIR owns structure, not final topology.
- [ ] No source lifetime parameter is introduced.
- [ ] No hidden widening of declared groups exists.
- [ ] Every listed group diagnostic has coverage and a stable code.
- [ ] Run `just validate`.

---

## Phase 8: Implement group topology and bulk backend cleanup

### Summary and reasoning

The group implementation must realize the hard count-free contract without reusing ordinary individual ownership rules.

### Tasks

- [ ] Integrate groups into the lifetime-topology solver.
- [ ] Permit same-group retained edges and cycles.
- [ ] Permit child-to-parent retained edges.
- [ ] Reject parent-to-child, sibling and cross-group illegal edges.
- [ ] Reject every group escape.
- [ ] Mark group-owned families as count-free and individually non-releasable.
- [ ] Ignore early-drop optimization for group-owned storage.
- [ ] Generate one bulk cleanup plan per group exit.
- [ ] Lower groups to no-op physical grouping on JS while preserving legality.
- [ ] Add full-control arena, segmented arena or grouped-allocation lowering.
- [ ] Preserve explicit cleanup for outgoing edges to externally owned REC families as defined by the companion plan.
- [ ] Ensure group-owned values cannot cross channels independently.

### Audit and validation

- [ ] Group-owned cycles are valid and count-free.
- [ ] Implicit cycles remain diagnostics.
- [ ] No group child carries individual cleanup responsibility.
- [ ] Group exit covers every CFG exit.
- [ ] Run `just validate`.

---

## Phase 9: Add the memory-strategy planner and Affine Ownership ABI

### Summary and reasoning

The backend must receive one complete plan instead of translating borrow-checker advisory sites directly.

Explicit prerequisites, in order, before this phase's planner may run:

```text
complete shared topology
target partition
target validation
physical variant scope
```

Only then does target/profile memory planning run, producing one `ValidatedMemoryPlan` per physical variant.

### Tasks

- [ ] Define backend-neutral memory requirements as the last target-independent handoff, carrying no selected physical strategy.
- [ ] Define a final memory plan keyed by allocation family and reachable function, scoped to one target/profile physical variant.
- [ ] Add `Affine`, `InferredRegion`, `ExplicitGroup` and `Rec` strategy categories.
- [ ] Run planning only after complete topology validation, target partition and target-contract validation.
- [ ] Consume the selected target, build profile and backend memory capability metadata.
- [ ] Carry hidden destination plans.
- [ ] Carry cleanup and destruction plans.
- [ ] Carry allocation-family layout requirements.
- [ ] Carry representation state for retention-sensitive operations.
- [ ] Define affine owned and borrowed runtime semantics.
- [ ] Preserve static result provenance separately.
- [ ] Preserve region-owned and group-owned mode as static metadata.
- [ ] Adopt the two-bit logical full-control handle contract from the REC companion.
- [ ] Preserve allocation-family base identity through projections.
- [ ] Remove the direct Wasm path that consumes borrow advisory drop sites as final authority.
- [ ] Delete obsolete single-tag assumptions.
- [ ] Avoid default whole-function strategy variants.
- [ ] Add structured memory-strategy decision reporting.

### Audit and validation

- [ ] Cleanup responsibility never duplicates.
- [ ] A backend receives no unresolved topology fact.
- [ ] One function body handles mixed strategies through local operations only.
- [ ] Source types remain unchanged.
- [ ] Run `just validate`.

---

## Phase 10: Implement affine and inferred-region full-control lowering

### Summary and reasoning

This phase establishes collector-free correctness before REC and field splitting are required for precision.

### Tasks

- [ ] Add full-control allocation helpers for affine heap values.
- [ ] Add inferred-region allocation and bulk release.
- [ ] Add region exit plans for normal, return, error and loop exits.
- [ ] Lower final-use transfer through the Affine Ownership ABI.
- [ ] Lower `drop_if_owned` from the final memory plan.
- [ ] Add hidden destination allocation for fresh result roots.
- [ ] Handle allocation-family responsibility through projections.
- [ ] Add deterministic aggregate overwrite, remove, clear and destruction behavior.
- [ ] Add concrete destruction plans without runtime reflection.
- [ ] Expand the Wasm runtime beyond static heap-base planning.
- [ ] Keep debug or GC-native lowering able to erase physical ownership work after semantic validation.
- [ ] Add developer reports for allocation strategy, region owner and release frontier.

### Audit and validation

- [ ] Acyclic accepted programs lower without tracing GC.
- [ ] Conservative cases fall back to a statically bounded inferred region.
- [ ] No missing optional transfer causes a source diagnostic.
- [ ] Backend lowering does not reconstruct source meaning.
- [ ] Run `just validate`.

---

## Phase 11: Add physical drop and region coalescing

### Summary and reasoning

This phase reduces allocation and cleanup overhead without weakening precise semantic intervals.

### Tasks

- [ ] Keep semantic intervals unchanged.
- [ ] Add one physical coalescing pass after strategy selection.
- [ ] Start with straight-line candidates only.
- [ ] Require one common proven enclosing region.
- [ ] Reject candidates with intervening calls.
- [ ] Reject candidates with intervening effectful or expensive operations.
- [ ] Support grouped drop lists.
- [ ] Support synthetic arenas where layout permits.
- [ ] Record retained-byte and cleanup-operation estimates.
- [ ] Add deterministic debug reporting.
- [ ] Do not apply implicit coalescing rules to shorten or reinterpret explicit groups.

### Audit and validation

- [ ] Physical lifetime coalescing is not stored as semantic topology.
- [ ] Source behavior is unchanged.
- [ ] The initial heuristic is boring and reviewable.
- [ ] Benchmark evidence reports both saved cleanup work and extra retained bytes.
- [ ] Run `just validate`.

---

## Phase 12: Execute the REC companion plan

### Summary and reasoning

REC is a precision layer after the parent topology and memory-plan substrate exists.

### Tasks

- [ ] Rebase the REC companion plan's baseline to the activation HEAD without changing its accepted design.
- [ ] Confirm parent Phases 2 through 10 satisfy its prerequisites.
- [ ] Execute `docs/roadmap/plans/retained-edge-counting-design-and-implementation-plan.md`.
- [ ] Keep all exact counter, ABI and iterative-destruction mechanics in the companion plan.
- [ ] Keep uniqueness scans, alias registries, adaptive switching and source REC rejected.
- [ ] Integrate REC decisions into the parent memory plan.
- [ ] Integrate region and group cleanup with outgoing REC obligations.
- [ ] Preserve non-atomic REC under the accepted channel model.
- [ ] Update parent progress rows when REC phases land.

### Audit and validation

- [ ] REC remains subordinate to topology.
- [ ] Ordinary locals, parameters, calls and `get()` remain count-free.
- [ ] Groups remain count-free.
- [ ] Cycles remain group-only.
- [ ] No full-control release path uses tracing GC.
- [ ] Use the companion plan's required validation and closeout gates.

---

## Phase 13: Implement field-sensitive allocation-family splitting

### Summary and reasoning

Collector-free correctness does not require splitting, but the final performance architecture does.

### Tasks

- [ ] Identify candidate aggregate families where one retained field keeps substantial unrelated storage alive.
- [ ] Require proof that split fields have independent layout and cleanup.
- [ ] Preserve alias and mutation behavior.
- [ ] Preserve explicit-copy graph behavior.
- [ ] Reject splitting when a projection or invariant requires one family.
- [ ] Require that the unsplit topology is already legal. Splitting must never be what makes source legal.
- [ ] Execute splitting per physical variant, after target partition and target validation.
- [ ] Rebuild the affected direct family-edge graph after each accepted split.
- [ ] Revalidate the affected outlives, SCC and family-base invariants after each accepted split.
- [ ] Fall back to the unsplit family and conservative retention when the optimisation cannot be proven, without emitting a source diagnostic.
- [ ] Rerun physical strategy and REC selection after a successful split.
- [ ] Keep all fields inside an explicit group until group exit even when physically split.
- [ ] Add developer reporting for accepted and rejected splits.

### Audit and validation

- [ ] No source-visible partial move exists.
- [ ] No broad parent is retained solely because splitting was skipped silently in a required case.
- [ ] REC targets the post-split family.
- [ ] Group lifetime remains hard.
- [ ] Run `just validate`.

---

## Phase 14: Integrate builder lifecycles and reactivity

### Summary and reasoning

Web workloads need page, mount, request and render lifetimes to avoid broad page-wide retention.

### Tasks

- [ ] Define builder-owned lifecycle root metadata.
- [ ] Instantiate page, mount, request, frame and render-generation roots in link planning.
- [ ] Attach reactive sources and mounted fragments to explicit lifecycle roots.
- [ ] Treat subscriptions as retained-edge facts, not active borrow lifetimes.
- [ ] End mount-owned regions on unmount.
- [ ] End render-generation regions after the generation is no longer observable.
- [ ] Preserve outgoing REC obligations at lifecycle teardown.
- [ ] Keep builder lifecycles unable to change source legality.
- [ ] Integrate the final topology with HTML JavaScript and Wasm partitioning.

### Audit and validation

- [ ] No reactive state is freed while observable.
- [ ] No hidden closure-like retention graph is introduced.
- [ ] JavaScript GC and collector-free Wasm accept the same source.
- [ ] Run `just validate`.

---

## Phase 15: Harden the deferred channel design against the final memory model

### Summary and reasoning

Channel semantics must be constrained before any async implementation starts.

### Tasks

- [ ] Update `docs/src/docs/async/@page.moth` to make send a mandatory affine transfer.
- [ ] Define the successful-send responsibility commit point.
- [ ] Define failed-send responsibility preservation.
- [ ] Define queue and receiver lifecycle ownership.
- [ ] Forbid arbitrary shared Moth reference graphs across tasks.
- [ ] Forbid independent transfer of group-owned values.
- [ ] Require `copy` or fresh message construction when the sender retains its graph.
- [ ] Preserve non-atomic REC by forbidding cross-task shared REC families.
- [ ] Define cancellation and channel destruction as deterministic cleanup obligations before implementation.
- [ ] Keep syntax, scheduling and buffering details deferred.
- [ ] Mark channels blocked on the memory-model implementation in the roadmap and progress matrix.

### Audit and validation

- [ ] The channel design introduces no exception to lifetime topology.
- [ ] Send responsibility never duplicates.
- [ ] No task can outlive its structured lifecycle root.
- [ ] This phase remains documentation-only unless a separate channel implementation plan is approved.
- [ ] Run the documentation-only release-build gate.

---

## Phase 16: Add collector-free release capability and artifact verification

### Summary and reasoning

The final guarantee is real only when a backend proves that every reachable family has a complete non-tracing strategy.

Verification operates on each complete target/profile `ValidatedMemoryPlan` and the artefact emitted from it, not on a single project-global plan.

### Tasks

- [ ] Add backend capability metadata for collector-free release.
- [ ] Require one complete memory strategy for every reachable heap family.
- [ ] Verify every affine family has a cleanup path.
- [ ] Verify every inferred region has complete exits.
- [ ] Verify every explicit group has complete bulk exits.
- [ ] Verify every REC family has valid counter and destruction plans.
- [ ] Verify every projection can recover its family base.
- [ ] Verify all builder lifecycle roots have teardown plans.
- [ ] Verify no tracing collector import, runtime object or fallback helper remains in capable release output.
- [ ] Treat missing strategy or verification data as `CompilerError`.
- [ ] Keep debug and GC-native lowering available.
- [ ] Add cross-profile and cross-backend acceptance parity tests.
- [ ] Add artifact inspection fixtures for the no-tracing guarantee.

### Audit and validation

- [ ] Capable release builds cannot silently fall back to tracing GC.
- [ ] Debug GC cannot accept topology rejected in release.
- [ ] Target partitioning does not change memory legality.
- [ ] Generated artifacts contain no obsolete GC fallback path.
- [ ] Run `just validate`.

---

## Phase 17: Final documentation and progress closeout

### Summary and reasoning

Phase 1 records accepted design. This phase records what actually landed and removes migration markers only when implementation proves them obsolete.

### Tasks

- [ ] Review every canonical memory page against the implemented architecture.
- [ ] Review compiler and build-system handoffs.
- [ ] Update the progress matrix row by row.
- [ ] Remove GC-fallback migration-debt notes only when the corresponding paths are deleted.
- [ ] Update current Wasm and JavaScript target coverage.
- [ ] Update collection operation support.
- [ ] Update channel prerequisites without claiming channel implementation.
- [ ] Update README and cheatsheet status wording.
- [ ] Update roadmap activation and completion state.
- [ ] Rebuild `docs/release/**` through the normal docs build.
- [ ] Inspect every changed generated route.
- [ ] Search for stale terms:
  - GC baseline
  - superseded collector-first correctness
  - release may fall back to a tracing collector
  - single-tag-only handle
  - implicit cycle
  - individual group drop
  - collector elision as optional on capable release backends

### Audit and validation

- [ ] Accepted design and current implementation are clearly separated.
- [ ] Historical plans are marked as historical.
- [ ] No generated file was edited manually.
- [ ] Run the documentation-only release-build gate for a docs-only slice or `just validate` for a mixed slice.

---

## Phase 18: Performance validation and final audit

### Summary and reasoning

Correctness comes first. Performance evidence determines whether later precision work is justified.

### Correctness matrix

- [ ] local aliases only
- [ ] branch-selected affine owner
- [ ] collection-loop final iteration
- [ ] conditional loop with unknown final iteration
- [ ] multiple aliased returns
- [ ] escaping projection
- [ ] hidden result destination
- [ ] retained edge into longer-lived storage
- [ ] illegal shorter-lived retained edge
- [ ] sibling-lifecycle diagnostic
- [ ] cleanup frontier at `clear()`
- [ ] collection survives beyond its retained-value region
- [ ] repeated region epochs
- [ ] user collection wrapper with inferred effects
- [ ] explicit group bulk cleanup
- [ ] explicit group cycle
- [ ] implicit cycle diagnostic
- [ ] group-owned no-early-drop behavior
- [ ] REC runtime-many independent removal
- [ ] region with outgoing REC edges
- [ ] field-sensitive split
- [ ] page and mount lifecycle teardown
- [ ] debug GC and release parity
- [ ] capable release artifact without tracing runtime

### Performance evidence

Measure at least:

- peak live bytes
- total allocated bytes
- allocation count
- individual drop count
- region reset count
- REC increment and decrement count
- REC-selected allocation count
- cleanup-frontier elision count
- field-split count
- compile time
- output size
- runtime throughput

Representative workloads must include:

- allocation-heavy loops
- collection population followed by `clear`
- long-lived cache with independent eviction
- large parent with one retained small field
- explicit-group cyclic graph
- reactive mount and unmount
- mixed package calls
- mixed counted and uncounted collection operations

### Final gates

- [ ] Run `cargo fmt` when Rust changed.
- [ ] Run targeted tests during iteration.
- [ ] Run `just validate` for the completed code-bearing implementation.
- [ ] Run full non-recording benchmark checks when gathering performance evidence.
- [ ] Do not commit benchmark history unless intentionally updating it.
- [ ] Run the documentation release build.
- [ ] Perform the AGENTS final audit.
- [ ] Confirm no compatibility wrapper, duplicate memory plan or stale fallback path remains.

---

## 9. Progress-matrix target shape

The final progress matrix should make these distinctions explicit.

| Surface | Initial status after design migration | Completion condition |
|---|---|---|
| Borrow validation and local last use | Supported or Partial | path-sensitive transfer and final-use facts complete |
| Result provenance and retention summaries | Partial | complete stable summaries across modules and packages |
| Lifetime topology and escape validation | Deferred | mandatory local and link proof implemented |
| Retained-edge liveness and cleanup frontiers | Deferred | whole-domain effects, frontiers and epochs implemented |
| Declared groups | Deferred | source, HIR, topology and backend support implemented |
| Group-only cycles | Deferred | explicit cycle construction and bulk cleanup implemented |
| Affine Ownership ABI | Experimental | memory-plan-driven full-control lowering implemented |
| Inferred-region lowering | Deferred | region allocation and complete exits implemented |
| Physical coalescing | Deferred | simple verified coalescing pass implemented |
| REC analysis and selection | Deferred | companion analysis and decision reporting implemented |
| REC backend lowering | Deferred | tags, counters, collections and iterative destruction implemented |
| Field-sensitive splitting | Deferred | split analysis and strategy rerun implemented |
| Builder lifecycle regions | Deferred | page, mount, request and render roots implemented |
| Collector-free release verification | Deferred | capable backend proves no tracing runtime remains |
| Debug or GC-native representation | Supported where target supports it | same source legality and behavior as collector-free paths |
| Channels | Deferred | final memory prerequisites and separate implementation plan accepted |

Do not collapse these rows into one generic "memory management" status.

---

## 10. Documentation migration matrix

| File or area | Required change |
|---|---|
| `docs/src/developer-docs/memory-management/overview.mtf` | six-part final model, collector-free invariant, REC boundary |
| `docs/src/developer-docs/memory-management/@page.moth` | concise introduction and REC route |
| `access-and-aliasing/**` | last-use centrality, allocation families, field-splitting extension |
| `borrow-validation/**` | affine transfer facts and handoff to retained-edge analysis |
| `lifetime-regions-and-escape-validation/**` | intervals, liveness, frontiers, epochs, group-only cycles |
| `declared-memory-groups/**` | count-free hard lifetimes, no early cleanup, cycles |
| `ownership-and-drops/**` | Affine Ownership ABI and memory-plan-driven cleanup |
| `runtime-and-backend-lowering/**` | all strategies and no tracing fallback for capable release |
| new `retained-edge-counting/**` | canonical REC technical authority, routed from the memory index |
| `docs/compiler-design-overview.md` | analysis owner, summaries, artifacts and backend handoff |
| `docs/build-system-design.md` | lifecycle instantiation, strategy plans and capability verification |
| `docs/src/docs/collections/**` | trusted builtin effects and collection `clear()` |
| `docs/src/docs/design-scope/**` | remove GC baseline and preserve exclusions |
| `docs/src/docs/async/@page.moth` | mandatory send transfer and memory prerequisites |
| `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf` | concise user-facing final model only |
| `docs/src/docs/progress/@page.moth` | separate accepted-design and implementation rows |
| `docs/roadmap/roadmap.md` | parent and companion plan sequencing, no superseded collector-first wording |
| `docs/roadmap/plans/grouped-memory-design.md` | superseded and deleted; its group implementation detail now lives in Phases 7 and 8, and every live link points at this plan |
| historical memory cleanup plan | confirm the file is absent and record that no historical annotation is required |
| `README.md` | collector-free capable release goal |
| `AGENTS.md` | update memory route and invariant wording where needed |
| generated `docs/release/**` | regenerate only through docs build |

---

## 11. Rejected and deferred alternatives

Rejected from the baseline model:

- source-visible RC or REC
- source lifetime annotations
- source move syntax
- source no-GC mode
- tracing-GC fallback in capable release builds
- uniqueness scans on collection removal
- alias registries
- runtime adaptive REC and region switching
- implicit cyclic regions
- cycle collection
- individual early cleanup for group-owned storage
- target-specific source legality
- unknown ordinary foreign retention
- whole-function REC and non-REC variants by default

Accepted but implementation-deferred:

- field-sensitive allocation splitting
- more advanced physical coalescing heuristics
- profile-guided strategy selection
- group-local allocator reuse
- richer aggregate REC beyond the collection-first slice
- channel and task implementation
- any future atomic REC model
- reserved-byte or preallocation syntax
- safe adoption of an ungrouped uniquely owned value into a group
- safe group extraction or movement between declared groups
- expression-site placement
- group-local graph construction and publication
- direct source construction of reference cycles inside one group
- builder lifecycle region metadata for reactivity

Final-use interior projection detachment from an allocation family is not accepted
design. Before it could become accepted architecture, a separate design must define partially moved
aggregate semantics, the parent representation after detachment, invalidation of existing aliases
and projections, control-flow joins, destruction of remaining fields, reactive and external
observers, aggregate invariants, and parity across GC, region, REC and collector-free backends.
Until then, projections remain rooted in their containing allocation family, and a proven final use
transfers the entire allocation family rather than detaching one child.

Deferred work must remain compatible with the locked source semantics and collector-free guarantee.

---

## 12. Completion criteria

The final memory-management redesign is complete only when all of these are true.

1. Every accepted reachable allocation family has one validated semantic lifetime owner.
2. Every retained edge has a proven outlives relationship.
3. Borrow and last-use analysis provides path-sensitive affine-transfer facts.
4. Public and generated summaries carry complete result, retention, detached stored-result and outlives facts.
5. Retained-edge liveness can end regions at final cleanup frontiers.
6. Fixed collections, growable collections and maps expose the trusted retention vocabulary.
7. Explicit groups provide hard count-free bulk lifetimes and the only source cycle mechanism.
8. The Affine Ownership ABI is memory-plan-driven and cleanup responsibility never duplicates.
9. REC is integrated only as defined by its companion plan.
10. Field-sensitive splitting fits before final strategy selection and is implemented before final optimization closure.
11. Builder lifecycles participate in ordinary topology.
12. Capable release backends verify that no tracing collector remains.
13. Debug and GC-native backends preserve identical source legality and observable behavior.
14. The progress matrix reports every surface honestly.
15. All stale collector-fallback and single-tag architecture wording is removed or marked historical.
16. `just validate`, the required docs build and the final architecture audit pass.

---

## 13. Start handoff

When this plan is activated:

1. Re-read `AGENTS.md`.
2. Confirm the activation HEAD.
3. Read every canonical memory authority in full.
4. Read the compiler and build-system architecture authorities in full.
5. Read the REC companion plan in full.
6. Read the collection references and async design draft.
7. Complete Phase 0 before changing implementation.
8. Complete the Phase 1 documentation migration before treating code behavior as the accepted design.
9. Keep each later phase on one owner and one implementation path.
10. Stop and update this plan only when a real contradiction appears. Do not silently create a second memory model.
