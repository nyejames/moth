# Memory management documentation corrections plan

## Purpose

Correct and consolidate Moth's permanent memory-management documentation before replacing the current collector-free memory implementation plans.

The accepted model is already coherent at the architectural level. This plan does not redesign it. It removes wording that could lead to a wrong implementation, makes ownership boundaries explicit and gives the later implementation plan one precise set of permanent authorities to consume.

The completed documentation must make these ideas unmistakable:

- borrow and future-use analysis proves access legality and reusable liveness facts
- lifetime analysis proves one legal topology and owns retained-edge legality, escapes, frontiers and cycles
- the compiler-owned memory planner turns one already legal topology into one target/profile physical plan
- one `ValidatedMemoryPlan` belongs to one physical-variant scope
- backends encode that plan and never reconstruct ownership, count transitions or source legality
- REC tracks unresolved runtime obligations rather than ordinary aliases
- a selected REC counter contains persistent retained-edge obligations plus at most one affine-root obligation
- last-use facts can reclassify obligations and remove count traffic without changing source semantics
- a real retained-edge cycle still requires one explicit group
- path-sensitive edge death and committed-state reasoning may prevent false cycle findings, but do not collect a real cycle

This plan ends with corrected permanent authorities, corrected teaching material and a clean handoff for the later replacement of the collector-free memory implementation roadmap.

## Current state

```text
STATUS: queued - ready for activation
CURRENT_SLICE: Phase 0 - refresh the correction inventory against the active worktree
BLOCKERS: none
NEXT_ACTION: activate this plan in a dedicated documentation worktree and execute Phase 0
```

Keep this block small. Record activation revisions, checkpoints and validation results in Git history and working notes.

## Starting assumptions

This plan starts from these accepted contracts:

- Moth is reference-semantic by default, copy-explicit and move-inferred
- borrow validation and lifetime-topology validation are mandatory for every backend and profile
- missing optional transfer proof falls back to borrowing without rejecting legal source
- missing topology proof is a source diagnostic
- every allocation family has exactly one semantic lifetime owner
- every retained edge satisfies `R_value >= R_container` or remains inside one owner region
- inferred regions are non-lexical CFG intervals and widen only along one ordered owner chain
- explicit groups are hard count-free lifetime domains and the only source mechanism for direct retained-edge cycles
- a group-owned target never uses REC, while an edge from group storage to an external REC target remains a counted obligation on that external target
- field-sensitive family splitting refines an already legal topology per physical variant
- `BackendNeutralMemoryRequirements` is the last target-independent memory artefact
- target partition and target-contract validation precede target/profile physical planning
- the compiler-owned planner produces one `ValidatedMemoryPlan` per physical-variant scope
- `check` creates and validates the memory plan and stops before lowering
- a capable full-control release backend must not fall back to tracing or reachability collection
- current Wasm `DropIfOwned` lowering from advisory borrow-checker sites is scaffolding debt

The separate Boracle work may improve the executable reference model and the future production borrow checker. This documentation work does not depend on Boracle becoming the production solver. It depends only on the stable fact contract defined below.

## Required authorities

Read the current versions of:

- `AGENTS.md`
- `docs/roadmap/roadmap.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/src/docs/codebase/memory-management/overview.mtf`
- every detailed and overview file under `docs/src/docs/codebase/memory-management/`
- `docs/src/docs/codebase/compiler-design/memory-management-and-gc/memory-management-and-gc.mtf`
- `docs/src/docs/codebase/compiler-design/borrow-validation-and-drops/borrow-validation-and-drops.mtf`
- `docs/src/docs/codebase/compiler-design/backend-lowering/backend-lowering.mtf`
- `docs/src/docs/memory/automatic-cleanup-and-retained-edges.mtf`
- `docs/src/docs/memory/declared-memory-groups.mtf`
- `docs/src/docs/cheatsheet/moth-language-cheatsheet.md`
- `docs/src/docs/progress/@page.moth`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `index.md`

Use the compiler and build-system task-reading guides. Read adjacent producer and consumer handoffs when an edit changes stage ownership wording.

## Migration inputs, not authorities

Read these files to preserve accepted decisions and identify stale wording:

- `docs/roadmap/plans/final-memory-management-redesign-and-implementation-plan.md`
- `docs/roadmap/plans/retained-edge-counting-design-and-implementation-plan.md`

They are temporary implementation work items. They do not override the permanent memory, compiler or build-system authorities.

Do not rewrite or delete either implementation plan in this documentation slice. Their replacement is the next planning task. This slice may remove links to them from permanent authorities and may correct roadmap wording that incorrectly calls a plan a semantic authority.

## Scope boundaries

### In scope

- permanent memory-model wording
- permanent compiler and build-system handoff wording
- REC obligation algebra and performance contract
- ownership of planned count transitions
- cycle coexistence and committed-state terminology
- `ValidatedMemoryPlan` conceptual contents and validation invariants
- stable borrow-analysis output contract
- current alpha checker wording clearly labelled as current implementation
- teaching and public documentation derived from the permanent authorities
- progress and roadmap link hygiene
- generated documentation rebuild

### Out of scope

- Rust implementation changes
- deleting or replacing the existing memory implementation plans
- changing Moth source syntax or accepted source legality
- implementing Boracle or selecting a production borrow-checker algorithm
- implementing lifetime analysis, groups, regions, REC or the memory planner
- renaming current Rust variants such as `DropIfOwned`
- adding an HIR cleanup instruction
- deleting current HIR or LIR scaffolding
- profile-guided memory strategy selection
- runtime adaptive switching between REC and region ownership
- atomic REC or cross-task shared REC
- benchmarking implementation
- changing progress-matrix status or coverage values
- editing generated files under `docs/release/**` by hand

## Locked correction contract

The following wording and ownership decisions are fixed for this documentation work.

### 1. Two questions stay separate

Permanent documentation must keep these questions separate:

```text
Is the topology legal?
    access and alias legality
    one lifetime owner
    retained-edge outlives
    escapes
    cycles
    external boundaries

How is one legal topology represented?
    stack or inline
    affine cleanup
    inferred region
    explicit group
    REC
    host GC where permitted
```

No physical strategy may make an illegal topology legal. No planning imprecision may become a source diagnostic. A missing strategy after successful topology validation is `CompilerError`.

### 2. REC obligation invariant

For one REC-selected allocation family `F`, the permanent invariant is:

```text
REC count(F) =
    live counted persistent-edge obligations(F)
    + optional affine-root obligation(F)
```

The affine-root term is either zero or one.

Permanent docs must distinguish the reason REC is selected from the complete physical counter invariant:

- REC is selected because unresolved runtime-many persistent retained edges disappear independently
- a selected counter also carries at most one affine-root obligation
- ordinary aliases, parameters, temporary projections and `get()` results never contribute obligations
- an affine transfer moves an existing root obligation and causes no count change by itself
- final-use storage may reclassify the affine root into one persistent edge with zero net count change
- detachment may reclassify one persistent edge into an affine result root with zero net count change
- one affine root can replace at most one newly created direct persistent edge

Remove or replace every unqualified sentence that says REC physically counts persistent retained edges only.

### 3. Planned obligation transitions

Retained-edge analysis owns semantic facts:

- which direct persistent edges an operation creates or removes
- cardinality
- outcome-specific commit effects
- whole-domain kills
- detached stored-result provenance
- frontier-enabling facts

The memory planner owns physical decisions:

- whether the target family is REC-selected
- the post-refinement direct family graph
- the concrete obligation transition at each semantic commit
- root-to-edge and edge-to-root reclassification
- transition fusion by target family
- cleanup and destruction plans

Backend REC lowering owns encoding only:

- tag extraction and masking
- counter load, checked update and zero test where the plan requires them
- the planned iterative destruction worklist
- target-specific instruction and runtime-helper selection

Permanent docs must not say that backend REC lowering independently decides count transitions.

Use this conceptual transition shape where a concrete example helps:

```rust
pub struct PlannedObligationTransition {
    pub family: PhysicalAllocationFamilyId,
    pub removed_persistent_edges: u32,
    pub created_persistent_edges: u32,
    pub affine_root_before: bool,
    pub affine_root_after: bool,
}
```

Exact Rust names remain open. The ownership boundary does not.

For one family:

```text
delta_count =
    created_persistent_edges
    - removed_persistent_edges
    + affine_root_after
    - affine_root_before
```

The planner normalises the complete semantic commit before lowering. The backend must never emit a temporary decrement-to-zero followed by an increment for a same-family replacement whose net transition is zero.

### 4. Cleanup terminology

Use these terms consistently:

- **transfer** moves affine cleanup responsibility to another path
- **discharge** satisfies one affine cleanup obligation
- **destroy** physically destroys one allocation family
- **bulk reclaim** ends a region or group

Use `discharge_if_owned` as the conceptual operation in permanent documentation. Exact Rust spelling remains open.

Use `DropIfOwned` only when naming the current Wasm scaffolding instruction or its migration debt. Do not use `drop_if_owned`, `release_if_owned` and `discharge_if_owned` as three competing permanent names.

A discharge does not always destroy:

```text
uncounted individually releasable family:
    discharge may destroy immediately

REC family:
    discharge removes the affine-root obligation
    destroy only when the total count reaches zero

inferred-region family:
    the individual handle does not own region reclamation

explicit-group family:
    no individual discharge destroys group storage
    group exit bulk-reclaims it
```

### 5. Committed retained-edge states and cycles

Cycle validation reasons about direct retained edges that may coexist in one reachable committed program state.

Permanent lifetime documentation must define:

- a semantic commit point for a retention-sensitive mutation
- old edges removed and new edges added atomically on the successful committed path
- failure paths preserving the old topology
- path-sensitive or epoch-sensitive facts that may prove two edge sets cannot coexist
- conservative may-coexist treatment when the analysis cannot disprove coexistence

Examples that must be covered:

```text
branch A:
    A -> B

branch B:
    B -> A
```

A path-insensitive union must not automatically be presented as proof that a runtime cycle exists. The analysis may preserve branch separation. If coexistence cannot be disproved, the topology is not proven legal and receives the normal topology diagnostic.

```text
old committed state:
    A -> B

new committed state:
    B -> A
```

The validator reasons about committed states rather than a temporary union of pre-commit and post-commit edges.

Last-use and future-use facts may prove that:

- an edge is dead
- no source capable of recreating the edge survives
- two edge sets occur in disjoint paths or epochs
- an earlier epoch ends before another topology starts

They do not reclaim a real retained-edge SCC. Every real self-cycle or multi-family SCC still requires one explicit declared group.

Field-sensitive family refinement must rebuild the affected direct graph and revalidate affected outlives, family-base and SCC facts.

### 6. Stable borrow-analysis output contract

Permanent borrow documentation must describe stable semantic outputs independently from the current alpha algorithm.

The stable contract includes:

- normalised semantic places and overlap facts
- value origins and preliminary provenance
- shared and exclusive loan or access liveness
- path-sensitive future-use and last-use classifications
- optional affine-transfer candidates
- proof that a persistent edge or affine root cannot disappear while a dependent temporary borrow remains usable
- proof that no capable source alias survives a candidate cleanup frontier
- preliminary return-root alias and projection evidence
- reactive invalidation and observability facts
- resolved external access-boundary classifications

The current alpha lattice and algorithm may still be documented, but only under an explicit current-implementation heading. These details are not permanent language architecture:

- `uninitialized`, `slot`, `alias` and `slot + alias`
- the current LocalId-centred root approximation
- current per-block may/must future-use precomputation
- current forward fixed-point transfer implementation
- current advisory drop candidates

Boracle remains a permanent reference solver and experiment facility. The memory model does not depend on Boracle running during normal compilation or on the future production solver using Boracle's exact storage and algorithm.

### 7. Last use feeds several systems

Permanent overview and teaching material must show that one future-use substrate has several consumers without collapsing their ownership:

| Fact | Consumer |
|---|---|
| a shared or exclusive access may still be used | borrow conflict validation |
| no later source use exists on every relevant path | optional affine transfer |
| no capable source alias survives | cleanup-frontier proof |
| a final-use persistent store commits | affine root to persistent edge reclassification |
| a stored value is detached into an owned result | persistent edge to affine root reclassification |
| a final loop iteration is statically known | final-iteration responsibility transfer |
| no observer or recreating source survives an epoch | inferred-region epoch completion |
| edge sets are mutually exclusive across paths or epochs | avoid a false cycle coexistence result |

Borrow validation supplies facts. Lifetime analysis and memory planning own the later decisions.

### 8. `ValidatedMemoryPlan` conceptual contract

One plan belongs to one target/profile physical-variant scope. It is not:

- one plan per source module
- one project-global physical plan
- part of the canonical semantic module artefact
- a backend-owned strategy choice

The permanent conceptual contents are:

```text
ValidatedMemoryPlan
    physical variant identity and capability inputs
    post-refinement allocation-family graph
    one selected physical family plan for every reachable family
    region plans and complete exits
    explicit-group plans and complete exits
    planned affine-responsibility transitions
    planned retained-edge obligation transitions
    hidden-destination plans
    cleanup plans
    destruction plans
    REC layout and counter decisions
    physical coalescing decisions
    normalised memory-plan identity inputs
```

The family strategy vocabulary must cover every accepted outcome:

```text
Stack
Inline
Affine
InferredRegion
ExplicitGroup
REC
HostGC
```

Exact Rust enum decomposition remains open. For example, stack or inline placement may be separate from heap cleanup strategy. No accepted outcome may be omitted from the conceptual contract.

Before publication, plan validation proves:

1. Every reachable post-refinement family has exactly one selected physical plan.
2. Every plan family belongs to this physical-variant scope.
3. Every retained-edge transition refers to valid direct post-refinement families.
4. Every family preserves its one validated semantic lifetime owner.
5. Every affine obligation reaches a planned discharge or safe transfer on every relevant path.
6. Every inferred region has complete exits.
7. Every explicit group has complete bulk exits.
8. Group-owned families are never individually releasable and never REC-selected.
9. Every REC family is acyclic and has a valid counter layout.
10. Every REC family keeps its validated fallback region.
11. Every destruction plan processes outgoing obligations exactly once.
12. Every projection representation preserves or recovers the allocation-family base.
13. Every hidden destination satisfies its validated destination and retained-edge constraints.
14. Every physical coalescing decision preserves the original semantic topology.
15. `HostGC` is rejected for a capable full-control release variant.
16. Normalised plan identity is deterministic for identical inputs.

Physical coalescing occurs before final plan validation and fingerprinting. It may retain storage slightly longer, but it never widens semantic topology, changes outlives, changes source legality or changes diagnostics.

### 9. One final affine authority

Borrow and lifetime stages produce:

- last-use facts
- optional transfer candidates
- ownership constraints
- candidate cleanup sites

`ValidatedMemoryPlan` owns the final physical affine decisions.

Backend handoff lists may still include borrow and lifetime facts as validated context. They must not list a separate final `validated affine cleanup decisions` artefact beside the plan as a competing authority.

### 10. Group boundaries

Public and technical group documentation must state:

```text
group-owned target:
    count-free

edge from group storage to a target owned by the same group:
    count-free internal group edge

edge from group storage to an external REC family:
    counted persistent obligation on the external target

at group exit:
    release outgoing REC obligations
    -> bulk reclaim group storage
```

Group count-free semantics are a property of the target owner, not every edge source located inside a group.

### 11. Authority and plan links

Permanent authorities, teaching pages and the progress matrix must not depend on short-lived implementation-plan filenames.

Allowed links to active plan files:

- `docs/roadmap/roadmap.md`
- a plan's own status or history where needed

Permanent docs should link:

- canonical memory authorities
- `docs/roadmap/roadmap.md` for sequencing
- `docs/src/docs/progress/@page.moth` for current support

The roadmap must not call either current memory plan a semantic authority. Until their replacement lands, describe them as temporary implementation work items pending consolidation.

## Exact source edit matrix

The implementation must follow this matrix. If a heading moved, update the plan's working notes and edit the current owner rather than creating a duplicate section.

### Permanent memory authorities

| File | Required headings or area | Required change |
|---|---|---|
| `docs/src/docs/codebase/memory-management/overview.mtf` | `The six cooperating mechanisms` | State that REC is selected for unresolved persistent-edge multiplicity and that a selected counter also contains at most one affine-root obligation. |
| same | `Rules every contributor must know` | Expand the last-use rule with the multi-consumer table or a direct link to one nearby table. Keep later decision ownership with lifetime analysis and planning. |
| same | `Collector-free correctness argument` | Replace shorthand that implies only persistent edges can contribute to a selected count. Include obligation reclassification as a precision mechanism. |
| same | `Compiler and backend layers` | State that the planner creates concrete per-family obligation transitions and the backend only encodes them. |
| same | `Hard invariants` | Replace `REC counts persistent retained edges only` with the complete obligation invariant and ordinary-alias exclusion. |
| `docs/src/docs/codebase/memory-management/borrow-validation/overview.mtf` | `Contract` | Replace advisory-drop-centred output wording with the stable place, origin, loan, future-use, transfer and preliminary provenance fact contract. Label advisory drops as current scaffolding only. |
| `docs/src/docs/codebase/memory-management/borrow-validation/borrow-validation.mtf` | `Design contract` and `The last-use contract` | Define the stable downstream fact contract and the several consumers of future-use facts. |
| same | `Analysis model`, `Future-use and optional transfer safety`, `Control flow and joins` | Move algorithm-specific alpha details under a clear `Current alpha implementation` heading. Do not present its lattice as accepted permanent architecture. |
| same | `Side-table outputs` | Separate stable required facts from current advisory drop candidates. |
| same | `Handoff to lifetime validation` | Add capable-source death, temporary-borrow safety and edge-coexistence inputs. |
| same | `Conservative precision and extension points` | State that stronger production solvers and Boracle may improve precision without changing the stable handoff or source semantics. |
| `docs/src/docs/codebase/memory-management/lifetime-regions-and-escape-validation/overview.mtf` | `Contract` | Add committed retained-edge states and may-coexist cycle facts to the output contract. |
| `docs/src/docs/codebase/memory-management/lifetime-regions-and-escape-validation/lifetime-regions-and-escape-validation.mtf` | `Terminology` | Define semantic commit point, committed retained-edge state and may-coexist edge relation. |
| same | `Retained-edge liveness and cleanup frontiers` | Explain how future-use facts prove edge death and loss of recreation capability. |
| same | new subsection adjacent to retained-edge liveness | Specify atomic successful commit effects, unchanged failure topology and path or epoch separation. |
| same | `Cycles and strongly connected graphs` | Define SCC validation over edges that may coexist in one reachable committed state. Explain conservative unproven coexistence and reaffirm group-only real cycles. |
| same | `Splitting refines an already legal topology` | State that direct graph and may-coexist SCC facts are rebuilt and revalidated after a split. |
| `docs/src/docs/codebase/memory-management/ownership-and-drops/overview.mtf` | `Contract` | Make final affine decisions plan-owned and use discharge terminology. |
| `docs/src/docs/codebase/memory-management/ownership-and-drops/ownership-and-drops.mtf` | `Discharging responsibility is not necessarily destruction` | Preserve the four-term distinction and make it the terminology authority. |
| same | `Unified ownership ABI` | Explain that responsibility and representation are independent dimensions and local specialisation may remove known tag checks without source-visible overloads. |
| same | `Conditional destruction` | Rename the conceptual operation to `discharge_if_owned`. Explain that current `DropIfOwned` is only an implementation spelling where referenced elsewhere. |
| same | `Static specialisation` | Permit local constant folding of responsibility and REC state without requiring whole-function duplication. |
| same | `Common mistakes` | Add reconstructing transitions in the backend and treating discharge as destruction. |
| `docs/src/docs/codebase/memory-management/retained-edge-counting/overview.mtf` | `Contract` | Give the planner ownership of concrete normalised transitions and destruction decisions. Limit lowering to encoding. |
| same | ownership chain diagram | Change the final chain to `memory planner -> selected layout and planned transitions -> backend REC encoding`. |
| same | `Invariant` | Use the complete counter equation and the ordinary-alias exclusion. |
| same | `Read next` | Remove the direct REC implementation-plan link and link the roadmap instead. |
| `docs/src/docs/codebase/memory-management/retained-edge-counting/retained-edge-counting.mtf` | `Design contract` and relationship sections | Separate semantic edge facts, physical plan decisions and backend encoding. |
| same | `Counter representation and invariant` | Keep the equation and state why selection is about persistent edges while the physical count also carries one root. |
| same | count-transition sections | Make semantic-commit fusion and per-family normalisation mandatory planner work. Include same-family replacement, final-use insertion, detached results and hidden destinations. |
| same | boundaries and specialisation sections | State that only retention-sensitive and cleanup-sensitive operations inspect bit 1 and known states may remove local tests. |
| same | new `Performance contract` section | Record which operations have no count traffic, which operations may update a counter, counter layout costs, non-atomic policy, iterative destruction and the permitted specialisations. Separate hard guarantees from benchmarking choices. |
| same | related reading | Replace direct plan links with permanent authorities and the roadmap. |
| `docs/src/docs/codebase/memory-management/declared-memory-groups/overview.mtf` | `Contract` and `Read next` | Keep the external REC edge rule and remove the direct parent-plan link. |
| `docs/src/docs/codebase/memory-management/declared-memory-groups/declared-memory-groups.mtf` | group boundary and teardown sections | Verify outgoing external REC obligations are released before bulk reclaim. Remove direct implementation-plan links. |
| `docs/src/docs/codebase/memory-management/runtime-and-backend-lowering/overview.mtf` | `Contract` | Remove separate final affine-decision authority. Add planned obligation transitions, hidden destinations, region/group placement and coalescing to the plan summary. |
| `docs/src/docs/codebase/memory-management/runtime-and-backend-lowering/runtime-and-backend-lowering.mtf` | `Backend-neutral semantics` and `Planning order` | State that final physical ownership and count transitions come only from the plan. |
| same | new `ValidatedMemoryPlan contract` section after planning order | Add the conceptual contents, seven outcomes, validation invariants, variant scope and fingerprint order from this plan. |
| same | `Wasm lowering` | Replace generic retain language with plan-driven persistent-edge transitions. |
| same | `Drop and retain behaviour` | Rename to `Planned discharge and retained-edge transitions`. Remove generic ARC-like `retain when` wording. |
| same | `Fact consumption` | Remove a separate final affine-decision input. Keep borrow and lifetime facts as validation context only. |
| same | `Backend specialisation` | Add local tag-test elimination, base-pointer mask hoisting as a permitted encoding optimisation and transition fusion as plan-owned. |

### Compiler and build-system authorities

| File | Required headings or area | Required change |
|---|---|---|
| `docs/compiler-design-overview.md` | `Frontend stages > Stage 6: borrow validation` | Describe the stable borrow handoff without locking the permanent architecture to the alpha checker lattice. |
| same | `Lifetime-region and escape validation > Retained-edge analysis` | Add committed-state and may-coexist cycle facts. |
| same | `Lifetime-region and escape validation > Backend-neutral memory requirements` | Keep candidates and constraints target-independent. Explicitly exclude concrete obligation transitions. |
| same | `Lifetime-region and escape validation > Memory-strategy planning` | Add the complete plan contents, transition ownership, coalescing-before-final-validation order and plan validation requirement. |
| same | `Lifetime-region and escape validation > Backend handoff` | Remove separate final affine-decision authority. Remove direct implementation-plan links. |
| same | `Backend-facing compiler handoff` | List one `ValidatedMemoryPlan` as the final physical authority. Borrow and lifetime facts remain context and assertions only. |
| `docs/build-system-design.md` | `Fixed bootstrap order` and `HTML project builder > Mixed-target planning and validation` | Preserve the shared/per-variant seam and add explicit plan validation before lowering. |
| same | `HTML project builder > Physical variants` | Add normalised obligation transitions and hidden-destination plans to the memory-plan fingerprint. State that coalescing is finalised before fingerprinting. |
| same | `HTML project builder > Link planning and lifetime topology` | Keep topology and backend-neutral requirements shared. Exclude concrete physical transitions. |
| same | `HTML project builder > Memory-strategy plans` | Add the complete plan contents, seven outcomes and validation invariants. |
| same | `HTML project builder > Runtime and memory` | State that Wasm lowering consumes planned transitions rather than deriving generic retain or release operations. |

### Teaching and public documentation

| File | Required headings or area | Required change |
|---|---|---|
| `docs/src/docs/codebase/compiler-design/memory-management-and-gc/memory-management-and-gc.mtf` | `The proof pipeline` | Add concrete transition planning to the per-variant side of the seam. |
| same | after `When transfer proof is missing` | Add a compact `One fact, several consumers` table. |
| same | `Backend representations` | Correct the REC physical invariant and explain zero-traffic reclassification. |
| `docs/src/docs/codebase/compiler-design/borrow-validation-and-drops/borrow-validation-and-drops.mtf` | `Data-flow facts`, `Future use and inferred transfer`, `Handoff to lifetime validation`, `From facts to lowering` | Separate stable facts from current alpha implementation and show how later stages consume them. |
| same | `Roadmap and current status` | Keep current advisory `DropIfOwned` debt but avoid presenting advisory drops as final input. |
| `docs/src/docs/codebase/compiler-design/backend-lowering/backend-lowering.mtf` | `Lowerer inputs` and `External bindings and ownership facts` | State that planned obligation transitions and discharge operations arrive settled. Lowerers do not infer them from borrow facts. |
| `docs/src/docs/memory/automatic-cleanup-and-retained-edges.mtf` | `Last use and stored obligations work together` | Keep the existing teaching example and add edge-to-root detachment and same-family zero-net commit wording. |
| same | `What REC doesn't count` | Replace `ordinary affine transfers` with precise wording that transfer moves or reclassifies one existing obligation without adding another. |
| same | source visibility section | State that only retention-sensitive commits on REC-selected families update counts. |
| `docs/src/docs/memory/declared-memory-groups.mtf` | `Bulk cleanup and cycles` or `Nested groups and retained edges` | Add the external REC target rule and group-exit release order. |
| same | `Read next` | Remove the direct implementation-plan link and link the roadmap. |

### Status, routing and generated documentation

| File | Required area | Required change |
|---|---|---|
| `docs/src/docs/progress/@page.moth` | memory rows | Keep all status and coverage values unchanged. Remove direct memory-plan file links. Keep current Wasm `DropIfOwned` scaffolding debt explicit. State that `ValidatedMemoryPlan` has no implementation owner yet where useful. |
| `docs/roadmap/roadmap.md` | `Collector-free memory implementation` | Stop calling a plan a semantic authority. State that canonical design lives in permanent memory docs and that the two current plans are temporary implementation work items awaiting one replacement plan. |
| same | plan list | While this plan is active or queued, link it under a separate documentation-work heading so it does not distort the hard implementation chain. Remove the link in the completion commit when this plan is deleted. |
| `docs/src/docs/cheatsheet/moth-language-cheatsheet.md` | reference semantics and groups | Audit only. Keep it concise. Change it only if it contradicts the corrected authorities. Do not add counter equations or backend-plan detail. |
| `AGENTS.md` | memory core contracts | Audit only. Change it only if the corrected ownership boundaries are not already represented. |
| `index.md` | navigation | No update is expected because no implementation file or owner moves. Update only if a permanent source documentation path changes. |
| every relevant `@page.moth` under the memory and compiler-design documentation trees | page introductions and summaries | Audit for duplicated stale wording. Update only summaries that repeat a corrected invariant. Keep detailed contracts in topic-named files. |
| `docs/release/**` | generated output | Never edit by hand. Regenerate through the documentation release build after source changes. |

## Required REC performance wording

The detailed REC authority must distinguish required performance properties from later measured policy.

### Required properties

- Unselected affine, region and group families carry no REC counter.
- REC-selected families carry one inline target-word-sized non-atomic counter under the accepted task model.
- Local aliases, parameters, projections, read-only calls and `get()` borrows cause no counter traffic.
- Final-use root-to-edge and detached-result edge-to-root reclassification may have zero net counter traffic.
- Fresh hidden-destination construction may start directly with one persistent obligation.
- Same-family remove/add commits are fused before lowering.
- Only retention-sensitive and cleanup-sensitive operations inspect REC representation.
- Counted destruction uses an iterative worklist and never recursive target-stack destruction.
- Count zero destroys one family exactly once.
- Underflow traps as an invariant failure.
- Counter overflow cannot wrap. The authority must either state the address-space bound or require a checked trap.
- REC graphs are acyclic because real cycles require groups.

### Measured or implementation-dependent choices

These remain for the later implementation plan and benchmarks:

- exact small-object REC selection threshold
- exact counter-header packing
- exact base-mask hoisting strategy
- exact local tag-test specialisation threshold
- exact deletion worklist inline capacity
- whole-function cloning or monomorphisation policy beyond the accepted default of local handling
- target-specific instruction sequences
- profile-guided strategy selection

The detailed page may name likely costs such as target-header cache-line writes, small-object overhead and destruction bursts. It must not lock an unmeasured heuristic into source or config semantics.

## Implementation phases

### Phase 0: Refresh the inventory

- Record the active branch and revision in working notes.
- Re-read every required authority from this worktree.
- Search all non-generated documentation for the stale phrases and direct plan links listed under `Search gates`.
- Confirm the current Boracle docs still keep lifetime topology and REC outside the reference solver's authority.
- Confirm no implementation or progress status changed since this plan was written.
- Produce a working checklist from the exact source edit matrix.

Stop if a canonical authority now contains a conflicting accepted decision. Resolve that design conflict with the user before editing derived pages.

### Phase 1: Correct detailed memory authorities

Edit the topic-named detailed memory files first:

1. borrow validation
2. lifetime regions and escape validation
3. affine ownership and drops
4. retained-edge counting
5. declared memory groups
6. runtime and backend lowering

Do not update overview or teaching pages until the detailed owners agree.

Phase exit gate:

- one owner exists for each term and decision
- the full REC counter invariant is correct
- cycle coexistence semantics are explicit
- concrete transitions belong to the plan
- the backend is encoding-only
- the stable borrow handoff is solver-independent

### Phase 2: Correct compact memory overviews

Update:

- `docs/src/docs/codebase/memory-management/overview.mtf`
- every affected leaf `overview.mtf`

Keep overviews compact. They should route to the detailed authority rather than duplicate every example.

Phase exit gate:

- no overview contradicts its detailed leaf
- no overview links a short-lived memory implementation plan
- hard invariants use the complete obligation wording

### Phase 3: Correct compiler and build-system authorities

Update the exact headings listed in the edit matrix.

Preserve the fixed pipeline:

```text
validated HIR
-> borrow and last-use facts
-> local family and retained-edge constraints
-> exported summaries
-> project/package instantiation
-> complete shared topology
-> intervals, frontiers and epochs
-> BackendNeutralMemoryRequirements
-> target partition
-> target validation
-> per-variant refinement
-> affected topology revalidation
-> target/profile memory planning
-> ValidatedMemoryPlan validation
-> plan fingerprint and final variant key
-> backend lowering
-> collector-free verification where required
```

Phase exit gate:

- `BackendNeutralMemoryRequirements` contains no physical selection or concrete transitions
- `ValidatedMemoryPlan` is the only final physical authority
- one plan belongs to one physical-variant scope
- coalescing precedes final validation and fingerprinting
- backend handoff has no competing final affine artefact

### Phase 4: Correct teaching and public pages

Update the compiler-design teaching pages and public memory pages from the corrected permanent authorities.

Use accessible wording. Keep source-language pages focused on what authors need to understand. Do not expose internal type names or every validation invariant in the public memory guide.

Phase exit gate:

- the last-use and obligation-reclassification advantage is explained at both compiler and public levels
- public group docs explain external REC targets
- no teaching page implies general ARC retain/release behaviour
- no teaching page treats Boracle or the alpha checker algorithm as the permanent implementation

### Phase 5: Correct status and roadmap references

- Update the progress matrix wording without changing status or coverage values.
- Update the roadmap's collector-free section to identify permanent authorities correctly.
- Remove direct memory implementation-plan links from permanent and public docs.
- Do not edit, delete or replace the two current memory implementation plans.

Phase exit gate:

- the roadmap is the only permanent source that links active memory implementation plans
- permanent docs link canonical authorities and the roadmap
- current implementation status remains honest

### Phase 6: Rebuild generated documentation

Run the documentation release build through the current release compiler or the Cargo equivalent.

Inspect generated changes for:

- updated memory pages
- updated compiler-design teaching pages
- updated progress page
- no unrelated generated churn
- no missing route or broken link

Never patch `docs/release/**` directly.

### Phase 7: Final consistency and Slice review

Re-read in this order:

1. detailed memory leaves
2. memory overview
3. compiler design
4. build-system design
5. compiler teaching pages
6. public memory pages
7. progress matrix
8. roadmap

Then perform the `AGENTS.md` Slice review.

Delete this plan and remove its roadmap entry in the same completion commit. Do not leave a completed plan in the tree.

## Search gates

Run these searches against source documentation, excluding generated release output unless the command is checking generated parity.

```bash
rg -n \
    "REC counts persistent retained edges only|ordinary affine transfers|Retain when a runtime representation" \
    docs \
    -g '!docs/release/**'
```

Expected result after correction: no stale unqualified wording.

```bash
rg -n \
    "validated affine cleanup decisions" \
    docs \
    -g '!docs/release/**'
```

Expected result after correction: no backend handoff treats this as a final authority separate from `ValidatedMemoryPlan`. Historical migration text may name the old shape only when clearly labelled.

```bash
rg -n \
    "final-memory-management-redesign-and-implementation-plan.md|retained-edge-counting-design-and-implementation-plan.md" \
    docs/src docs/compiler-design-overview.md docs/build-system-design.md
```

Expected result after correction: no permanent or public authority links either short-lived plan file.

```bash
rg -n \
    "DropIfOwned|drop_if_owned|release_if_owned|discharge_if_owned" \
    docs \
    -g '!docs/release/**'
```

Review every result:

- `discharge_if_owned` is the conceptual permanent operation
- `DropIfOwned` appears only in current implementation debt or progress wording
- `drop_if_owned` and `release_if_owned` do not remain as competing permanent names

```bash
rg -n \
    "BackendNeutralMemoryRequirements|ValidatedMemoryPlan" \
    docs/compiler-design-overview.md \
    docs/build-system-design.md \
    docs/src/docs/codebase/memory-management \
    docs/src/docs/codebase/compiler-design
```

Review every result for the shared/per-variant seam and single final authority.

```bash
rg -n \
    "parent authority|sole detailed owner|implementation sequencing" \
    docs/roadmap/roadmap.md \
    docs/src/docs \
    docs/compiler-design-overview.md \
    docs/build-system-design.md
```

Expected result: no temporary implementation plan is described as permanent semantic authority.

## Required validation

This is a documentation-only plan. The required final gate is:

```bash
moth build docs --release
```

Use the Cargo equivalent when an up-to-date release compiler is unavailable:

```bash
cargo run --quiet -- build docs --release
```

A useful preflight is:

```bash
moth check docs --terse
```

or:

```bash
cargo run --quiet -- check docs --terse
```

Do not claim either command ran unless it completed in the active worktree.

## Required final audit

Before completion, verify all of the following:

- The counter equation includes persistent obligations and at most one affine root.
- Ordinary aliases and temporary borrows are never counted.
- Last-use analysis remains required when REC is selected.
- Root-to-edge, edge-to-root and same-family net-zero transitions are explicit.
- Concrete transitions are planned before lowering.
- The backend cannot produce a transient zero for one atomic same-family replacement.
- Transfer, discharge, destroy and bulk reclaim are distinct terms.
- Real retained-edge SCCs remain group-only.
- Mutually exclusive or sequential edge states are not automatically merged into a false runtime cycle.
- Conservative failure to disprove coexistence still reports topology not proven.
- Group-owned targets are count-free while outgoing edges to external REC targets remain counted.
- The stable borrow handoff is independent from the alpha checker algorithm and Boracle execution.
- `BackendNeutralMemoryRequirements` remains the final target-independent memory artefact.
- `ValidatedMemoryPlan` is scoped to one physical variant and covers all accepted strategy outcomes.
- Plan validation invariants are listed in permanent architecture documentation.
- Physical coalescing precedes final plan validation and fingerprinting.
- Backend handoff contains no competing final affine authority.
- Current Wasm `DropIfOwned` remains clearly labelled scaffolding debt.
- Progress statuses and coverage values did not change.
- No permanent authority links a short-lived memory implementation-plan filename.
- Generated docs were rebuilt rather than edited manually.
- This plan and its roadmap entry are removed in the completion commit.

## Handoff to the replacement implementation plan

The completion summary for this documentation work must give the next planning task a concise handoff containing:

- the final permanent authority paths
- the final shared/per-variant pipeline
- the final stable borrow-analysis input and output contract
- the final `BackendNeutralMemoryRequirements` boundary
- the final conceptual `ValidatedMemoryPlan` contents and validation rules
- the final obligation-transition ownership split
- the final cycle and group rules
- the current implementation gaps that remain after documentation-only work
- confirmation that the two old memory implementation plans were deliberately left for replacement

The later replacement implementation plan must consume those permanent authorities, replace the two overlapping memory plans with one implementation sequence and update the roadmap in the same change.