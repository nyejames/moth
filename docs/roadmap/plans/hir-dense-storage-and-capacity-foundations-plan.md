# HIR Dense Storage and Capacity Foundations Implementation Plan

## Purpose and authority

Replace HIR's remaining recursive expression and place ownership with module-owned dense storage addressed by compact typed IDs and ranges.

This is primarily an architecture and debt-prevention plan. It establishes the representation that later HIR work should grow on rather than waiting for expression allocation to become a major performance bottleneck and then paying for a larger migration. Performance improvement is welcome but is not required. A repeatable compile-time regression is not acceptable.

The plan also completes the existing frontend capacity-estimation direction for HIR. Cheap statistics gathered during parallel source preparation and existing header aggregation seed growable HIR stores; completed HIR freezes into fixed-size dense ownership before its stable downstream handoff.

The durable architectural rule belongs in `docs/compiler-design-overview.md`. This plan owns the migration and sequencing only. Once implementation is complete, retire the plan and keep the general storage rule and any HIR-specific invariants in the canonical architecture documentation.

No source-language semantics change.

## Current-state capsule

```text
STATUS: queued, design approved
CURRENT_SLICE: Phase 0 - refresh HIR owners, baseline performance and freeze boundary
ROADMAP_POSITION: after unified numeric semantics and before Wiring V1
ACTIVATION: consume delivered MON v1 and the numeric checkpoint; the package programme remains active in parallel and its future expansion is not a prerequisite
BLOCKERS: earlier serial numeric work must be merged; Wiring V1 and native result slots must not start first
NEXT_ACTION: activate from current main, inventory every HIR expression/place owner and consumer, record the performance baseline and freeze the exact store contract before changing representation
```

Do not pin this queued plan to a speculative baseline commit. Record the active revision, worktree and benchmark state in working notes when implementation starts.

## Roadmap sequencing

The intended serial order is:

```text
MON syntax / MON Rust tooling
    -> unified numeric types and semantics
    -> HIR dense storage and capacity foundations
    -> Wiring V1
    -> native result slots and Core constant evaluation
    -> later dependent compiler work
```

The living Core and Builder programme remains active in parallel. It does not need to finish all future package expansion before this plan activates. Its later work already depends on Wiring and native result slots; treating that future work as a prerequisite here would create a cycle.

This plan intentionally lands before Wiring and native result slots. Both later plans should build directly on the dense HIR representation rather than first extending the recursive representation and then being migrated.

This decision supersedes the existing roadmap statement that all HIR compaction must wait for profiling evidence. **HIR expression/place dense storage is now planned architecture work.** Whole-AST flattening, expression-scratch redesign outside HIR and broader borrow-fact compaction remain profiling-gated.

## Required reading

Re-read the current owners at activation rather than treating paths or Rust shapes below as frozen:

* `AGENTS.md`
* `docs/compiler-design-overview.md`, especially the architectural invariants, numeric ownership and Stage 5 HIR sections
* `docs/src/developer-docs/style-guide/style-guide.mtf`
* `docs/src/developer-docs/style-guide/testing.mtf`
* `docs/src/developer-docs/style-guide/validation.mtf`
* `docs/roadmap/roadmap.md`
* `benchmarks/frontend-optimization-results.md`
* `src/compiler_frontend/arena/`
* `src/compiler_frontend/hir/`
* HIR validation and display/rewriting owners
* borrow checking and Boracle HIR consumers
* JavaScript and Wasm HIR lowering
* generated-function/materialisation consumers
* queued Wiring and native-result-slot work boundaries before their implementation begins

`docs/compiler-data-layout-design.md` remains the authority for its source, token and diagnostic domains. Do not broaden that document into a second general HIR architecture authority.

### Numeric checkpoint input

The activation tree already carries fixed I*/U*/F* types, Byte, Number scales,
profile-selected Int/Float and U32 Error.code. Preserve their canonical type
identity, constant payloads, numeric domain/operator/failure metadata and explicit
conversion boundaries. Update store-aware visitors and ID-keyed numeric side
tables together. A storage migration must not restore IntAdd/FloatAdd-only
classifiers, eager i32/f64 literal payload assumptions or backend-specific
interpretation of canonical numeric types.

Source semantic width, physical scalar layout and Wasm carrier are distinct.
Compacting a HIR record does not narrow its literal value or choose a new numeric
profile. Preserve U64 extremes, F16/F32 precision, signed zero and exact Number
coefficients/scales. Arbitrary-precision value payload storage follows its value
owner rather than being mistaken for a recursive child-expression graph.

## Locked design decisions

| Area                         | Decision                                                                                                                                                                                                                                      |
| ---------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Scope                        | Flatten HIR expression ownership and `HirPlace`, including variable-sized payloads directly owned by those structures. Do not turn this into a whole-HIR storage rewrite.                                                                     |
| Expression identity          | `HirValueId` becomes the dense handle for the module's expression store. The expression record does not redundantly store the ID when it can be derived from its index.                                                                       |
| Fixed-arity relationships    | Store child `HirValueId`s inline, such as unary, binary, range and projection operands.                                                                                                                                                       |
| Variable-arity relationships | Store compact typed `{ start, len }` ranges into module-owned dense side stores.                                                                                                                                                              |
| Places                       | Replace recursive `HirPlace` boxes with a root local plus an ordered projection range. Indexed projections reference expressions by `HirValueId`.                                                                                             |
| Construction                 | HIR lowering appends to typed `Vec` stores through one builder/store owner. IDs and ranges remain stable if a `Vec` reallocates.                                                                                                              |
| Final ownership              | Completed published HIR uses fixed-size dense owners such as `Box<[T]>`. Phase 0 may move the precise structural-freeze boundary to fit current generated/remap flows, but published HIR must not retain unnecessary spare growable capacity. |
| Capacity policy              | Extend the existing `FrontendArenaCapacityEstimate`; do not create another statistics or allocation-policy subsystem.                                                                                                                         |
| Statistics source            | Keep cheap per-source facts in the parallel tokenization/preparation path. Existing header facts may refine module estimates after aggregation. Do not add a separate counting traversal solely to size HIR.                                  |
| Exact counts                 | Use an exact count when an existing operation naturally knows it. Do not duplicate lowering logic to predict exact final HIR sizes.                                                                                                           |
| Correctness                  | Capacity estimates are policy only. Under-estimation grows normally; over-estimation costs bounded spare capacity during construction.                                                                                                        |
| Performance                  | Neutral is acceptable. Repeatable regression is blocking and must be investigated or the responsible representation change revised.                                                                                                           |
| AST                          | No whole-AST flattening plan is created here. AST keeps its current ownership until future profiling justifies a separately designed migration.                                                                                               |
| Persistence                  | Persistent HIR caches, mmap-compatible records, relative pointers, pointer relocation and cross-process representation are outside scope. The dense pointer-free shape should make that work easier later without designing for it now.       |
| Allocation framework         | Do not introduce a generic bump allocator, custom allocator or lifetime-heavy arena framework. Typed `Vec` builders plus compact IDs/ranges are the default.                                                                                  |
| Unsafe                       | This plan requires no new unsafe memory representation or pointer manipulation.                                                                                                                                                               |

## General compiler storage rule

Publish one concise invariant in `docs/compiler-design-overview.md`:

> Long-lived, high-volume graph-like compiler IR should default to stage-owned dense storage addressed by typed compact IDs and typed ranges. Fixed-arity edges store IDs inline; variable-arity edges use ranges into typed side stores. Growable `Vec` storage is appropriate during construction and should freeze to fixed dense ownership at a stable handoff where later structural growth is not required. Recursive owning `Box`/`Vec` graph trees require a specific semantic or measured reason.

AST is explicitly not implied to require migration by this rule. Parser-owned syntax trees may keep a representation suited to parsing and semantic construction until evidence supports changing them.

After adding this invariant, audit permanent design documentation for repeated generic explanations of this same build/store/freeze pattern. Replace repetition with a short reference to the invariant where this does not hide a stage-specific contract, representation size, ownership exception or semantic requirement.

## Accepted HIR storage model

Exact Rust names may change during Phase 0. The ownership model may not.

A representative construction/final form is:

```rust
struct HirStoreBuilder {
    expressions: Vec<HirExpression>,
    value_refs: Vec<HirValueId>,
    struct_fields: Vec<HirStructFieldValue>,
    variant_fields: Vec<HirVariantFieldValue>,
    map_entries: Vec<HirMapEntry>,
    structural_string_pieces: Vec<ConstStringPiece>,
    place_projections: Vec<HirPlaceProjection>,
}

struct HirStore {
    expressions: Box<[HirExpression]>,
    value_refs: Box<[HirValueId]>,
    struct_fields: Box<[HirStructFieldValue]>,
    variant_fields: Box<[HirVariantFieldValue]>,
    map_entries: Box<[HirMapEntry]>,
    structural_string_pieces: Box<[ConstStringPiece]>,
    place_projections: Box<[HirPlaceProjection]>,
}
```

Not every side store above must retain those exact names or be physically separate if Phase 0 shows a simpler typed arrangement. Do not merge unrelated element types merely to reduce the number of vectors.

### Expression identity

Conceptually:

```rust
#[repr(transparent)]
struct HirValueId(u32);
```

addresses:

```text
expressions[HirValueId]
```

through checked store accessors.

A durable expression record therefore needs no independent `id: HirValueId` field unless Phase 0 discovers a concrete invariant that cannot be represented by the store index.

Every checked conversion from a host `usize` into a compact ID or range component must reject overflow rather than truncate.

### Fixed child edges

For example:

```rust
BinOp {
    left: HirValueId,
    op: HirBinOp,
    right: HirValueId,
}

Range {
    start: HirValueId,
    end: HirValueId,
}

Cast {
    source: HirValueId,
    policy: BuiltinCastPolicyId,
}
```

These records own no child expression allocation.

### Variable child edges

Use dense ranges for collections such as:

* call arguments
* collection elements
* tuple elements while real tuple values remain part of HIR
* other homogeneous expression lists

Conceptually:

```rust
#[repr(C)]
#[derive(Clone, Copy)]
struct HirValueRange {
    start: u32,
    len: u32,
}
```

Structured variable payloads use their own typed rows and ranges where appropriate:

```rust
struct HirStructFieldValue {
    field: FieldId,
    value: HirValueId,
}

struct HirMapEntry {
    key: HirValueId,
    value: HirValueId,
}
```

The same rule applies to variant fields and other expression-owned variable records.

Do not retain a `Vec` inside each expression after moving its data into the store.

### Flat places

Replace the current recursive conceptual shape:

```text
Local
Field(Box<Place>)
Index(Box<Place>, Box<Expression>)
```

with a value equivalent to:

```rust
struct HirPlace {
    root: LocalId,
    projections: HirPlaceProjectionRange,
}

enum HirPlaceProjection {
    Field(FieldId),
    Index(HirValueId),
}
```

Thus:

```text
items[index].name
```

is represented as one root plus ordered projections:

```text
Local(items)
Index(index_value_id)
Field(name_field_id)
```

`HirPlace` is a compact path value, not another separately identified graph node unless Phase 0 proves that place identity is independently required.

### Complete expression-ownership cutover

Phase 0 must find every durable HIR field that currently owns:

* `HirExpression`
* `Box<HirExpression>`
* `Vec<HirExpression>`
* another record containing these recursively

This includes expression kinds, statements, terminators, patterns, places and any other HIR carrier found in the active tree.

After the cutover, durable HIR records reference expressions through `HirValueId` or typed ranges. Do not leave one hidden recursive ownership lane behind because it is uncommon.

HIR-local transformations that currently recursively clone/rebuild an expression tree must become store-aware. Before structural freeze they may allocate replacement records and return new IDs or use narrow builder-owned mutation where semantics require it. Downstream analyses continue to read validated HIR rather than rewriting it.

## Capacity and allocation policy

The existing `src/compiler_frontend/arena/` owner remains authoritative for frontend capacity policy.

Current estimates already include HIR block, statement and expression pressure. Extend that bundle only for real dense stores introduced by this plan, such as expression references or place projections.

### Gathering

Preserve the existing cheap path:

```text
parallel source tokenization/preparation
    -> per-source TokenStats
    -> deterministic module aggregation
    -> existing HeaderStats and known fragment counts
    -> FrontendArenaCapacityEstimate
    -> HirStoreBuilder::with_capacity(...)
```

Token classification is gathered while the most parallel part of the frontend is already walking the source. This plan must not add a second token traversal.

After dependency sorting and module consolidation, use already-produced header facts where they improve estimates. Do not create an expensive post-sort analysis whose sole purpose is allocation prediction.

### Estimation

Prefer simple monotonic formulas based on existing facts:

* token volume and relevant token classes
* declaration/header counts
* signature and variant counts
* known fragment/template counts
* exact local lengths already available to the lowering operation

Keep hard caps and saturating arithmetic.

The heuristic does not need to predict final HIR exactly. Its job is to avoid obvious repeated growth cheaply.

### Instrumentation

Add detailed counters for the new stores sufficient to compare:

* estimated count
* actual element count
* initial reserved capacity
* growth beyond the initial reservation
* final retained element count/capacity where meaningful

Do not add counters merely because a store exists. Each counter must answer estimate quality, growth pressure or regression attribution.

Use the evidence to tune estimates after the storage cutover. Do not tune formulas in advance against hypothetical HIR.

## Performance posture

This migration is justified by architectural consistency and future debt reduction, but that is not permission to make compilation slower.

Required posture:

* capture the current HIR/frontend baseline before the migration
* use existing adversarial expression, collection/map, borrow and representative project cases before inventing new fixtures
* add a new fixture only if existing cases cannot exercise a required storage shape
* run non-recording benchmark checks after each material cutover
* use five independent benchmark invocations at final performance acceptance, following the existing benchmark protocol
* do not record a slower result as a new baseline to make a regression disappear
* investigate any repeatable movement outside normal noise
* treat a repeatable regression above 5% on a representative affected case as automatically material; smaller consistent regressions still require explanation
* prefer revising a representation or accessor over accepting a measurable slowdown for aesthetic consistency

No performance gain is required for completion.

## Non-goals

This plan does not:

* flatten the whole AST
* introduce an AST arena programme
* flatten every outer HIR collection merely because it is a `Vec`
* require `HirBlock.locals`, `HirBlock.statements`, function tables, choice tables or other unrelated outer stores to change unless the expression/place cutover mechanically requires a narrow adjustment
* redesign result-slot semantics
* implement Wiring
* change borrow-checker legality or Boracle precision
* compact general borrow facts
* redesign TIR or const evaluation
* redesign numeric typing, cast evidence, rounding or the numeric profile
* add incremental compilation or a persistent HIR cache
* define a serialization format
* introduce pointer relocation, relative native pointers or mmap-backed live Rust values
* introduce a general allocator framework
* perform speculative SoA/bit-packing work on HIR records without separate measurement
* change source syntax, diagnostics or observable backend behaviour

## Implementation phases

Every implementation phase ends with the mandatory gate. A phase is a coherent checkpoint, not permission to preserve a second legacy representation for convenience.

### Phase 0 - activation, owner inventory and baseline

* [ ] Activate from current `main` after unified numeric semantics and before Wiring V1 begins. Keep the package programme's parallel work separate.
* [ ] Record the active revision, worktree state, validation baseline and non-recording benchmark baseline.
* [ ] Inventory every durable HIR owner of expressions and recursive places, including expressions, statements, terminators, patterns, remapping, validation, display, rewrites, generated functions, borrow/Boracle consumers and both backends.
* [ ] Include delivered numeric/Byte/Number constant payloads, conversion/failure statements, range proofs and U32 Error.code in the producer/consumer inventory.
* [ ] Search for every `HirExpression`, `Box<HirExpression>`, `Vec<HirExpression>` and recursive `HirPlace` storage site. Classify temporary lowering locals separately from durable IR ownership.
* [ ] Reconfirm how generated-function publication and current ID remapping affect the structural freeze point.
* [ ] Freeze the exact store and typed-range map against the active tree. Preserve the decisions above while allowing names and the number of side stores to simplify.
* [ ] Add only the instrumentation required to establish actual expression/list/projection counts and allocation-growth pressure.
* [ ] Publish the concise general dense-IR invariant in `docs/compiler-design-overview.md`, without prematurely documenting the migration as complete.

Exit: every producer, consumer, variable payload and freeze boundary is known; baseline evidence exists; no planned structure depends on an obsolete pre-numeric or post-result-slot assumption.

Mandatory closeout: architecture/design review, focused instrumentation review, normal validation and non-recording benchmark check.

### Phase 1 - atomic dense expression and place cutover

* [ ] Introduce the module-owned HIR store/builder and checked typed range accessors.
* [ ] Make `HirValueId` the dense expression-store handle and remove redundant per-expression identity where the index owns it.
* [ ] Replace every fixed child expression with `HirValueId`.
* [ ] Replace every expression-owned variable list with an appropriate dense side-store range.
* [ ] Flatten `HirPlace` into a root local plus ordered field/index projections, with index expressions referenced by ID.
* [ ] Migrate statements, terminators, patterns and all other durable expression carriers in the same representation cutover.
* [ ] Preserve evaluation order, source spans, type IDs, regions, value kinds and exact existing HIR semantics.
* [ ] Preserve numeric failure/conversion ordering and update ID-keyed proof side tables without changing the numeric profile or recomputing source meaning.
* [ ] Convert recursive expression rewrites and visitors to store-aware traversal. Do not retain a compatibility tree or reconstruct temporary trees for downstream consumers.
* [ ] Migrate HIR validation, display/remapping, public/compiler handoffs, borrow checking, Boracle inputs and JavaScript/Wasm lowering to the store API.
* [ ] Seed the new builder from the existing capacity estimate where a suitable estimate already exists. Let missing side-store estimates grow normally until Phase 2 tunes them.
* [ ] Delete replaced recursive ownership helpers and obsolete tests rather than preserving forwarding wrappers.
* [ ] Add a narrow architecture audit for reliably detectable forbidden recursive HIR ownership forms so later work does not silently restore them.

Exit: production durable HIR contains no recursively owned expression tree and no recursive `HirPlace`. All supported compiler consumers operate directly on IDs/ranges from one store representation.

Mandatory closeout: full validation, affected backend execution, borrow/Boracle gates, architecture audit and non-recording frontend/end-to-end benchmark checks. Treat this as one representation cutover even if implementation uses bounded internal work items and multiple audits.

### Phase 2 - structural freeze and capacity tuning

* [ ] Establish the final builder-to-fixed-store handoff at the Phase 0 freeze boundary.
* [ ] Convert completed growable HIR arrays to fixed dense ownership such as `Box<[T]>`; retain no structural growth API past that boundary.
* [ ] Preserve legitimate in-place remapping before final publication where current compiler identity merging requires it. Structural freeze means topology/counts are fixed, not that every scalar word can never be rewritten during the owning handoff.
* [ ] Extend `FrontendArenaCapacityEstimate` only for side stores whose growth evidence justifies a seed.
* [ ] Gather any new token-level facts during the existing parallel preparation pass and any header-level facts from existing aggregation. Add no sizing-only source or HIR pre-pass.
* [ ] Use exact local sizes where they are already naturally known.
* [ ] Compare estimates, actual counts, reserved capacity and growth. Tune formulas conservatively and keep existing hard caps.
* [ ] Confirm freeze/shrink behaviour does not introduce a measurable compile-time regression or excessive transient memory peak.
* [ ] Remove temporary instrumentation that no longer answers an ongoing capacity-policy question; retain useful benchmark counters.

Exit: HIR builds through pre-sized growable stores, publishes fixed dense storage and has evidence-backed capacity policy with no correctness dependency on estimate quality.

Mandatory closeout: capacity/retained-memory audit, focused HIR tests, full validation and the benchmark protocol below.

### Phase 3 - hardening, documentation and future-plan handoff

* [ ] Run the final five-invocation frontend and end-to-end benchmark evidence and investigate every repeatable regression.
* [ ] Re-audit production HIR for recursive expression/place ownership, redundant expression IDs and accidental per-record variable allocations.
* [ ] Update the Stage 5 HIR documentation to describe only HIR-specific storage/freeze facts and reference the general dense-IR invariant for the shared pattern.
* [ ] Audit permanent compiler design documentation for repeated descriptions of the same dense-ID/range/build-freeze rule and compress them where clarity is preserved.
* [ ] Update the queued Wiring plan so its HIR work consumes the delivered store representation rather than restoring old expression/place shapes.
* [ ] Update the native-result-slot/Core const-eval plan so its HIR call/result examples and Phase 2 migration assume `HirValueId`/range storage. Preserve its approved result semantics unchanged.
* [ ] Replace the roadmap's old profiling-only HIR-compaction note with the delivered architecture status while keeping whole-AST and broader borrow-fact compaction profiling-gated.
* [ ] Update benchmark evidence, progress documentation and implementation navigation where the delivered representation changes them.
* [ ] Run the existing numeric parity corpus across all profiles, including large fixed integers, F16/F32 rounding, Byte, Number scale boundaries and U32 error codes. Keep Number's Wasm target restriction intact.
* [ ] Run final correctness, architecture and style reviews.
* [ ] Remove this ordinary implementation plan and its roadmap entry in the completion commit after durable rationale has moved into canonical documentation.

Exit: dense HIR expression/place storage is the sole architecture, future queued HIR work assumes it, permanent documentation states the general rule once, and compilation performance is neutral or better.

## Mandatory phase gate

Use the current validation owners from the active tree rather than copying stale command lists blindly. At minimum, affected phases must cover:

* `just validate`
* the standard feature matrix when representation changes cross feature-configured HIR code
* focused HIR lowering/validation tests
* borrow-checker tests and `just boracle` when its HIR inputs change
* JavaScript execution/lowering coverage
* supported Wasm HIR/LIR coverage
* the architecture source audit
* `just bench-frontend-check`
* `just bench-check`

Final performance acceptance additionally uses five independent invocations of the existing frontend and end-to-end benchmark protocol and records the actual revision/configuration with the evidence.

An unavailable required lane is unrun, not passing.

## Acceptance criteria

The plan is complete only when all of these are true:

* `HirValueId` addresses one canonical dense expression store.
* Expression records do not retain an ID that is redundant with their store position.
* No durable HIR type owns child expressions through `Box<HirExpression>`, `Vec<HirExpression>` or equivalent recursive expression values.
* `HirPlace` has no recursive owning place tree.
* Fixed-arity expression edges use compact IDs inline.
* Variable-arity expression data uses typed ranges into module-owned dense side stores.
* HIR construction uses growable typed stores and completed published HIR uses fixed dense ownership.
* Existing token/header-derived capacity policy seeds the new storage without a sizing-only compiler traversal.
* Incorrect capacity guesses cannot change compiler semantics.
* HIR validation, borrow checking, Boracle and current backends consume the new representation directly.
* Existing source semantics, evaluation order, diagnostics and emitted behaviour are preserved.
* Fixed/profile-selected numerics, Byte, Number and U32 Error.code retain their type identity, payload and failure/rounding boundaries through the representation change.
* No repeatable accepted compile-time regression remains.
* AST representation is unchanged except for mechanical HIR-lowering API adaptation.
* The general dense-IR rule exists once in canonical compiler architecture documentation and repeated generic explanations have been compressed where safe.
* Wiring and native result-slot work can start from the delivered representation without another expression/place storage migration.
