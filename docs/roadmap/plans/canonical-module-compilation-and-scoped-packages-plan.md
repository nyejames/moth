# Canonical module compilation and scoped packages - Phase 5 closeout plan

## Purpose

Finish the canonical module and scoped-package cutover from the reviewed Phase 5 checkpoint, correct the remaining boundary-ownership and invariant defects, then hand a syntax-independent dependency substrate to the follow-up path, dependency-clause and resource-linking plan.

Phase 5 targets:

- one semantic compilation per physical module inside one project or package boundary
- one deterministic source inventory and preparation pass per selected source
- immutable completed provider interfaces rather than donor headers, AST or HIR
- stable cross-module identities and generated sidecars owned by the consuming boundary
- complete retained graph outcomes and success-only linkable project payloads
- strict scoped support packages and module-root-relative dependency resolution
- dense build-local IDs, contiguous records and narrow operation-scoped indexes
- no compatibility path, duplicate semantic owner or speculative duplicate work
- no graph or provider consumer coupled to the current `import` keyword

Phase 5 does not implement the new path or resource language surface. It completes the substrate that the follow-up plan will replace at the syntax boundary.

## Current state

```text
ACTIVE_PLAN: docs/roadmap/plans/canonical-module-compilation-and-scoped-packages-plan.md
WORK_ID: R5-closeout
WORK_SOURCE: parent review of checkpoint a3f8a00aff344934cba83e3602c03e577f5d1b46 and plan rebase ba4bc925
IMPLEMENTED_CHECKPOINT: R5C5B checkpoint (pending Gate A review)
REPOSITORY_STATE: one coordinator checkpoint commit for R5C5B; parallel benchmark and highlighter corrections committed separately
STATUS: paused - Gate A review required
CURRENT_SLICE: R5C5B generated boundary ownership (complete, awaiting review)
ACCEPTED_FROM_CHECKPOINT:
- R5C3C provider agreement and recursive interface closure
- R5C4A exhaustive canonical token traversal for correctness
- R5C5B boundary-scoped generated ownership, per-boundary lookup and stable package-identity symbol assignment
REQUIRED_RELOADS: AGENTS.md, this plan, compiler-design-overview.md, build-system-design.md, generated_worklist.rs, compilation.rs, compiled_boundary.rs, module_artifact_store.rs, frontend_orchestration.rs, build.rs
VALIDATION_STATE: full just validate green at the R5C5B checkpoint (workspace tests, integration 1817/1817, clippy, docs, bench-ci)
BLOCKERS: none
NEXT_WORKER_ORDER: Gate A review -> R5C1B/R5C4B -> Gate B review -> R5C6A -> Gate C review -> R5C6B -> R5C7 -> R5C8 -> R5C9 -> Phase 5 exit review -> mandatory handoff
STOP_REASON: plan requires stopping for Gate A after R5C5B
NEXT_RESUME_ACTION: accept Gate A, then implement R5C1B and R5C4B
FOLLOW_UP_BOUNDARY: path values, dependency clauses and resource linking
```

Keep this block current and concise. Git history is the durable implementation history.

## Required authorities

Read before implementation and every review gate:

- `AGENTS.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/language-overview.md`
- memory-management, style-guide, testing and validation authorities under `docs/src/docs/codebase/`
- `docs/src/docs/progress/@page.moth`

The architecture overviews own semantic boundaries. The language overview owns current syntax. This plan owns the remaining Phase 5 sequence and handoff.

## Accepted checkpoint baseline

Keep this work. Do not rebuild it under new names.

- Separate project and source-package `CompiledGraphBoundary` values retain graph identity, artefacts, generated lanes and diagnosed/blocked outcomes.
- Boundary-local dense IDs remain separate from stable semantic identities.
- `CompletedSourcePackageRegistry` owns contiguous package records, prefix lookup and direct dependency adjacency.
- Retained authored provider references carry non-optional file-local shell identity.
- Authored and implicit provider inputs are explicit states.
- One operation-local `ProviderInterfaceId` selects one immutable interface and narrow binding view.
- Provider declarations, evidence and summaries use agreement insertion.
- Recursive interface closure uses exact `RecordRef` values and one declaration/evidence queue.
- Final interfaces remain deterministic vectors with construction-only indexes.
- Frozen generic bodies reuse the canonical `TokenKind` vocabulary and one exhaustive string-ID traversal.
- Generated declaration lookup resolves to an exact artefact and template row.
- Summaries and sidecars live in one `CompletedGeneratedFunction` row.
- Generated sessions own only their local delta.

R5C3C is accepted. R5C4A is accepted for correctness. R5C4B must remove hot-path allocation and make every frozen remap path fallible.

The checkpoint is not a valid Phase 5 boundary because:

- unrelated package stores suppress local generated requests
- generated executable lookup assumes global identity uniqueness
- graph validation checks records to slots but not slots to records
- source-package boundaries are not all validated at frontend publication
- exact template lookup does not verify the requested declaration identity
- generated and materialisation publication can partially mutate before failure
- deterministic iteration and test-support cleanup remain incomplete

## Locked implementation decisions

### Generated functions are boundary owned

Concrete generic functions live in sidecars owned by the consuming project or package compilation.

- A `GeneratedFunctionIdentity` is unique only inside one boundary.
- Equal identities may exist in independent package boundaries.
- Another package's sidecar never suppresses a local request.
- Generated executable lookup is scoped by the caller's `CompiledModuleRef` or equivalent boundary identity.
- Local HIR may retain only `GeneratedFunctionIdentity` while the caller boundary remains explicit through assembly.
- "One materialisation per identity" means once per owning boundary.
- Duplicate identity inside one boundary remains an invariant failure.
- Package compilation order must not change ownership.

Do not implement a hybrid global store whose row is owned by whichever package compiles first. A true project-global store would require a separate architecture change.

### Graph outcomes are total

`CompiledGraphBoundary::validate_invariants` is the single completion proof for one graph frontend result.

```text
Successful -> no diagnosed or blocked record
Diagnosed  -> exactly one diagnosed record
Blocked    -> exactly one blocked record
Unavailable -> CompilerError
```

Also reject duplicate lane entries, diagnosed/blocked overlap, out-of-range IDs, invalid successful artefact references and graph/slot-count disagreement.

`ProjectFrontendCompilation::new` validates the project and every source-package boundary. Later linkable assembly adds only the stricter all-successful gate.

### Publication is transactional

Any multi-row publication preflights before mutation:

1. validate row-local identity agreement
2. reject duplicates inside the input
3. reject duplicates against retained state
4. resolve required dependencies
5. reserve capacity
6. append every row and index entry

This applies to generated deltas, module materialisation contexts and package publication.

### Exact rows verify identity

Before materialising an indexed template row:

```rust
artefact.declaration_identity == *input.identity.declaration()
```

must hold. A stale but in-range row is `CompilerError`.

### Token remapping is exhaustive, in place and fallible

Keep one canonical exhaustive walker:

```rust
pub fn try_remap_string_ids<E>(
    &mut self,
    map: &mut impl FnMut(StringId) -> Result<StringId, E>,
) -> Result<(), E>
```

Ordinary remapping mutates existing payloads and keeps path vectors allocated once. Frozen capture and materialisation clone each token once, then remap the clone. Token locations and path fields use the same fallible contract. Invalid frozen indexes return `CompilerError`.

### Provider facts are imported once

Each consumer/provider pair projects one closed provider interface. Stable declarations, evidence and summaries are stored once, while aliases and namespace members retain only stable references.

Agreement helpers accept borrowed candidates and clone only for vacant entries. Equal duplicate records are normal for recursively closed facades.

### Dependency syntax production is replaceable

Current import-specific Rust names are transitional syntax-owner vocabulary.

Phase 5 preserves:

- one file-local dense shell identity per top-level source or provider dependency clause
- retained typed structural references produced during the single preparation pass
- exact shell-to-provider joins
- immutable interfaces
- direct graph edges and package dependency IDs
- no path or source rediscovery after IDs exist

Directory graph, provider, package, closure and binding owners must not depend on `TokenKind::Import`, the authored keyword, raw source spelling or a second token scan.

Do not partially rename `ImportShellId`, `FileImport` or import-oriented modules during R5 closeout. The follow-up plan owns one coherent grammar and terminology migration.

### Work limits

Inside one project or package boundary:

- read, tokenize and prepare each selected source once
- resolve each dependency shell once
- compile each physical module once
- close and project each provider interface once per consumer/provider pair
- materialise each generated identity once per boundary
- emit one diagnostic set per diagnosed module or request

Directory projects must not use the synthetic single-file scanner. Synthetic mode may retain its isolated shared-parser traversal.

Moth is pre-release. Replace APIs directly and delete old owners. Do not add compatibility wrappers, feature flags or production adapters kept only for tests.

## Slice and review discipline

Each slice must name its owner, inputs, outputs, deleted code, focused tests and non-goals. Standard code gate:

```bash
cargo fmt --all
just validate
```

Stop when a second durable representation appears necessary, identity would depend on display data, a full-table clone remains in a hot loop, more than two unlisted stage boundaries change or the slice exceeds roughly 12 production files or 600 net production lines.

Review gates:

- **Gate A after R5C5B:** generated ownership and caller-scoped lookup
- **Gate B after R5C1B/R5C4B:** total graph outcomes, exact rows, in-place remapping and transactional publication
- **Gate C after R5C6A:** boundary-scoped convergence node model with no behavioural changes
- **Gate D after R5C9:** Phase 5 exit and syntax-independent handoff

Reviews are read-only. Resolve every required finding before continuing past its gate.

## Remaining Phase 5 work

### R5C5B: restore generated boundary ownership

**Goal:** remove cross-boundary generated ownership and define valid R5C6 node identity.

**Owners:**

- `create_project_modules/generated_worklist.rs`
- `create_project_modules/compilation.rs`
- `create_project_modules/compiled_boundary.rs`
- `build_system/build.rs`
- connected build-system test support

**Changes:**

- remove `CompletedGeneratedFunctionView` from local request deduplication
- stop flattening every completed package store into one imported generated view
- let a session reuse only its own `BoundaryGeneratedFunctionStore` and transactional delta
- do not let imported package summaries claim ownership of a local request
- allow equal generated identities across separate project/package boundaries
- scope generated owner indexes by boundary and identity
- resolve generated HIR targets relative to the calling boundary
- preserve package-local sidecar self-containment through `ProjectCompilation`
- update counters and comments from global uniqueness to per-boundary uniqueness

A conceptual key may be:

```rust
pub struct BoundaryGeneratedTarget {
    pub boundary: CompiledBoundaryRef,
    pub identity: GeneratedFunctionIdentity,
}
```

Exact names may differ. Build-local boundary refs do not enter public semantic identity.

**Tests:**

- two independent packages instantiate the same exported generic and each publishes one sidecar
- reversing package order produces identical boundary contents
- a package cannot resolve an unrelated package's sidecar
- project and package boundaries may contain equal identities
- duplicate completion inside one boundary fails
- caller-scoped lookup selects the local sidecar when equal identities exist elsewhere
- each package remains coherent when the other is removed

**Delete:** cross-package request suppression, global identity-only owner maps and duplicate rejection across unrelated boundaries.

**Non-goals:** global generated storage, identity redesign, convergence scheduling and persistent caches.

Run the standard gate and stop for Gate A.

### R5C1B: make graph-outcome validation total

**Goal:** establish one complete graph frontend outcome gate.

**Owners:** `compiled_boundary.rs`, `module_artifact_store.rs`, `compilation.rs`, `build.rs` and boundary tests.

**Changes:**

- validate the complete slot/lane bijection
- reject duplicate and overlapping diagnosed or blocked entries
- reject every final `Unavailable` slot
- validate successful artefact references while walking slots
- validate every source-package boundary in `ProjectFrontendCompilation::new`
- remove redundant downstream completion checks
- change the unfinished-slot regression to expect frontend-boundary rejection
- move `ProjectCompilation::from_test_modules(Vec<Module>)` and synthetic builders into test support
- remove the old flat-module construction shape from production `build.rs`

**Tests:** both mismatch directions for diagnosed/blocked states, duplicate/overlap cases, unavailable project/package slots, missing successful artefacts and a valid mixed outcome retained for `check`.

**Non-goals:** scheduler, diagnostic cascade or `CompiledModuleRef` redesign.

### R5C4B: exact remapping, lookup and publication

**Goal:** retain checkpoint correctness while removing avoidable allocation and partial mutation.

**Owners:** `tokenizer/tokens.rs`, frozen generic syntax/materialisation, `generated_worklist.rs`, `module_artifact_store.rs` and `compiled_boundary.rs`.

**Changes:**

- replace reconstructive token mapping with one exhaustive in-place fallible walker
- route frozen token and source-location remapping through it
- verify declaration identity at the exact template row
- preflight complete generated deltas before append
- validate `record.identity == record.sidecar.identity` again at the boundary store
- preflight module and package materialisation rows before mutation
- resolve package dependencies before publishing the package row
- retain deterministic materialisation rows in contiguous order plus one narrow lookup map
- replace symmetric project/package duplicate scans with one deterministic registration pass
- make agreement insertion borrow and clone only on vacant entries

**Tests:**

- all token payloads remap in place and path vectors are not rebuilt
- invalid frozen token or location indexes return `CompilerError`
- a stale in-range template row fails identity validation
- late generated or package duplicates leave owners unchanged
- sidecar/record identity disagreement leaves the store unchanged
- deterministic materialisation iteration survives reversed insertion order
- occupied agreement paths avoid cloning where instrumentation can prove it

**Delete:** reconstructive ordinary remapping, panic-prone remap indexing, incremental publication and nondeterministic iteration presented as deterministic.

Run the standard gate after R5C1B and R5C4B, then stop for Gate B.

### R5C6A: instrument and build the read-only convergence graph

**Goal:** measure current work and prove node ownership before scheduling changes.

**Precondition:** Gates A and B accepted.

Instrument:

- base-module and sidecar borrow passes
- complete summary-map clones
- summary changes
- dirty nodes and SCCs
- SCC sizes and maximum iterations
- stable nodes revisited after convergence

Build one read-only deterministic graph over boundary-local source, private and generated functions. Stable cross-module source summaries and provider summaries are fixed leaves. Another package's generated sidecar is never a node or leaf for the current boundary.

Requirements:

- derive edges from stable HIR targets and retained link facts
- preserve current borrow execution and results
- make boundary ownership visible in node identity
- expose focused test inspection without retaining a second graph authority
- compare predicted dirty sets with current rechecks

Tests cover equal generated identities in two boundaries, provider leaves, independent SCCs, reversed discovery order and unchanged diagnostics/summaries.

Run the standard gate and stop for Gate C.

### R5C6B: dependency-driven call-summary convergence

**Goal:** remove unconditional whole-boundary borrow rechecking.

Requirements:

- schedule only nodes whose local input or callee summary changed
- process SCCs in deterministic boundary-scoped order
- keep provider and cross-module source summaries as fixed leaves
- prove monotone summary updates or diagnose oscillation
- avoid cloning complete summary maps into every sidecar
- do not recheck independent stable SCCs
- run final base borrow validation with exact required summaries
- preserve deterministic diagnostics

If function-granular scheduling requires a broad borrow-checker rewrite, stop and create a dedicated plan. A temporary module-level schedule is acceptable only when exact counters prove unrelated stable sidecars are not rechecked.

Tests cover reverse-reachable dirtying, recursive convergence, independent boundaries, oscillation failure and result parity with the baseline.

### R5C7: remove prepared-source payload cloning

**Goal:** preserve one read and tokenization without complete payload copies.

- keep `PreparedSourceStore` indexed by `SourceId`
- move canonical payloads into their single preparation owner
- share one immutable source allocation only for a real second consumer
- do not clone `FileTokens` before header preparation
- rebind source identity without copying tokens
- prevent diagnosed preparation from being consumed twice
- retain typed path syntax facts so the follow-up can add resource facts during this same pass
- do not narrow storage around import-only facts or require later rescanning
- do not implement `Path`, resource identity or asset publication here

Synthetic single-file traversal may retain its isolated cache.

### R5C8: simplify ownership and preserve the dependency boundary

**Goal:** make the finished pipeline inspectable without changing semantic dataflow.

Suggested ownership:

```text
create_project_modules/
├── compiled_boundary.rs
├── module_artifact_store.rs
├── package_registry.rs
├── provider_inputs.rs
├── graph_compile.rs
└── frontend/
    ├── mod.rs
    ├── preparation.rs
    ├── semantic_compile.rs
    ├── generated_materialisation.rs
    └── generated_summary_fixpoint.rs
```

Requirements:

- one obvious module coordinator
- package indexing/order in one owner
- provider input construction in one owner
- graph/provider consumers accept typed retained dependency references only
- no directory owner imports tokenizer-specific dependency grammar or branches on `TokenKind::Import`
- generated materialisation/convergence stay outside base semantic orchestration
- replace long compile-wave argument lists with one narrow immutable context
- split generic materialisation only at real ownership boundaries
- group link-symbol name maps when lifecycles match
- return borrowed views instead of rebuilding vectors where simple
- keep import-oriented filenames provisional
- remove stale comments, forwarding modules, compatibility derefs and production test adapters
- keep tests with their owner
- do not add stage traits or dynamic dispatch

### R5C9: deletion audit and Phase 5 validation

Audit:

**Source and graph**

- no directory fallback scanner or `ProjectPathResolver` source fallback
- synthetic traversal stays isolated with one read/tokenization
- no source/token reparse after retained dependency syntax exists
- no graph/provider consumer depends on `TokenKind::Import`
- no suffix/path matching for provider joins

**Provider and interface**

- no donor header, AST or HIR copying
- no first-provider-wins record
- no optional retained shell identity
- no provider/closure hot-path linear scan
- no complete payload clone per alias or namespace member
- no occupied agreement-path clone

**Generated and boundaries**

- no cross-boundary request suppression or global identity-only owner map
- equal generated identities may coexist across boundaries
- no consumer-local materialisation or context scan by identity
- no parallel summary/sidecar stores
- no partial generated/materialisation publication
- no `Unavailable` retained slot
- every project/package boundary passes the full outcome bijection

**Token and ownership**

- no mirrored token vocabulary
- no ordinary remap that rebuilds path vectors
- no infallible frozen remap indexing
- no full declaration-table clone in hot loops
- no complete prepared-source clone per handoff
- no flat `Vec<Module>` production test constructor
- no compatibility API

**Handoff**

- dependency syntax can be replaced without graph/package/provider/interface changes
- import-specific names are not public or persistent identity
- no asset behaviour is reconstructed from rendered strings
- `RenderedPathUsage` is not promoted into final resource authority
- prepared syntax can gain resource facts without another source pass

Final gate:

```bash
cargo fmt --all
cargo run --quiet -- tests --audit
just validate
just bench-ci
```

Counters must prove one source read/tokenization/preparation, one module compilation, one interface publication, one provider projection per pair, one generated materialisation per identity per boundary and no unrelated SCC recheck.

Run Gate D.

## Phase 5 exit gate

Phase 5 is complete only when:

- every selected project and package module compiles once through canonical graph jobs
- every retained boundary has a complete validated outcome
- consumers bind only completed immutable interfaces
- recursive source and binding re-exports are closed and validated
- generated sidecars are owned by their consuming boundary
- equal generated identities coexist across boundaries without cross-addressing
- generated deduplication and convergence are boundary scoped
- graph identity and successful artefacts survive into `ProjectCompilation`
- provider binding performs no path rediscovery or repeated full-interface projection
- package readiness uses one indexed dependency model
- prepared source, generated lookup and convergence avoid repeated payload clones
- generated and materialisation publication are transactional
- token remapping is exhaustive, in place and fallible
- one directory-project compiler path remains
- dependency consumers are independent of the current `import` keyword
- source preparation can later produce resource facts without rescanning
- full validation and all review gates are clean

After acceptance, compress the detailed R5C section to one accepted-baseline paragraph before archiving this plan.

## Mandatory post-Phase-5 handoff

Stop after Phase 5 acceptance. Do not continue the former R6 path, asset, entry-union or reuse work from this plan.

The immediate follow-up plan owns:

- removal of the `import` keyword and top-level extensionless dependency clauses
- grouped bindings, namespace aliases and provider-backed file clauses
- explicit-extension compile-time `Path` values
- opaque resources as path values only
- resource visibility through ordinary exported `Path` values
- stable resource origins and dense build-local resource IDs
- structural resource anchors through TIR, folding and public interfaces
- per-function/root resource facts and exact entry unions
- builder URL rendering, placement, emission and invalidation
- `.mtf` resource paths without import declarations
- `$literal` and order-independent template body syntax
- coherent compiler terminology, diagnostics, docs and example migration

Phase 5 hands it:

- dense dependency shell identity
- one preparation pass
- typed structural provider references
- exact shell/provider joins
- immutable interfaces
- boundary-local graph/package identities
- strict facade/support visibility
- no dependency rediscovery after IDs exist

Later link, backend and reuse plans consume the final resource facts. They must not reconstruct resources from flat strings.

## Validation policy

Focused tests are iteration evidence, not the acceptance gate. Run the architecture audit whenever a slice changes discovery, dependency shells, provider binding, interface closure, semantic identity, borrow summaries, graph scheduling or generated sidecars.

This plan-only revision does not claim that checkpoint code was revalidated.

## Deferred beyond Phase 5

Immediate follow-up:

- dependency-clause grammar migration
- `Path` and resource identity
- template resource anchors
- asset publication and `$literal`

Later:

- complete link and package assemblies over final resource facts
- backend handoff, fingerprints and incremental reuse
- persistent artefact serialisation and caches
- package registries, remote fetching, versions and lockfiles
- direct normal-sibling dependencies
- cross-entry browser chunking
- Wasm physical partition and Component Model integration
- cross-build generated-instance caches
- measured module-wave parallelism unless benchmarks justify it
