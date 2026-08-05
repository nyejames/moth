# Canonical module compilation and scoped packages - Phase 5 closeout plan

## Purpose

Finish the canonical module and scoped-package cutover, then hand a syntax-independent dependency substrate to the dedicated dependency-clause and path-syntax plan.

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

Phase 5 does not implement the new dependency grammar, builtin `Path`, resource identity or asset publication.

## Current state

```text
ACTIVE_PLAN: docs/roadmap/plans/canonical-module-compilation-and-scoped-packages-plan.md
WORK_ID: R5-closeout
WORK_SOURCE: parent reviews through checkpoint 909d41660b198db5f39e7d822282a107e69be118
IMPLEMENTED_CHECKPOINT: R5C1C checkpoint (complete, awaiting Gate B re-review)
REPOSITORY_STATE: clean at the R5C1C checkpoint commit
STATUS: paused - Gate B re-review required
CURRENT_SLICE: R5C1C - finalize boundaries and validate dense identity mapping (complete)
ACCEPTED_FROM_CHECKPOINT:
- R5C3C provider agreement and recursive interface closure
- R5C4A exhaustive canonical token traversal for correctness
- R5C5B boundary-scoped generated ownership and caller-scoped lookup
- R5C4B exact remapping, exact template-row identity and transactional publication
COMPLETED_R5C1C:
- CompiledGraphBoundary::finish sorts and proves every boundary before publication
- compile_module_waves and single-file compilation return only finished boundaries
- CompletedSourcePackageRegistry::publish validates the finished package boundary before mutation
- dense outcome lanes replace hash sets; successful slots prove interface origin equals graph node origin and reference exactly one artefact row with no orphaned rows
- CompiledSourcePackage::validate proves root range, package identity, normal root role, final outcome and interface agreement
- generated publication proves one in-range generated root and exact summary agreement
- ProjectCompilation::from_successful_boundaries uses one require_all_successful conversion
- project/package materialisation collision check uses direct indexes without per-row owner strings
PENDING_GATE_B:
- R5C1B completion through R5C1C
REQUIRED_RELOADS: AGENTS.md, this plan, compiler-design-overview.md, build-system-design.md, compiled_boundary.rs, module_artifact_store.rs, generated_worklist.rs, compilation.rs, build.rs and boundary tests
VALIDATION_STATE: full just validate green at R5C1C (workspace tests 4083, integration 1818/1818, cross-target Clippy, docs, bench-ci)
BLOCKERS: none
NEXT_WORKER_ORDER: Gate B re-review -> R5C6A -> Gate C review -> R5C6B -> R5C7 -> R5C8 -> R5C9 -> Phase 5 exit review -> mandatory handoff
STOP_REASON: plan requires stopping for Gate B re-review after R5C1C
NEXT_RESUME_ACTION: submit the R5C1C checkpoint for Gate B re-review; after acceptance implement R5C6A
FOLLOW_UP_CHAIN:
1. dependency-clauses-and-path-syntax-plan.md
2. tir-corrections-and-simplification-plan.md
3. path-values-and-resource-linking-plan.md
```

Keep this block current and concise. Git history is the durable implementation record.

## Required authorities

Read before implementation and each review gate:

- `AGENTS.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/language-overview.md`
- memory-management, style-guide, testing and validation authorities under `docs/src/docs/codebase/`
- `docs/src/docs/progress/@page.moth`

The architecture overviews own semantic boundaries. The language overview owns current syntax. This plan owns the remaining Phase 5 sequence and handoff.

## Accepted baseline

Keep this work. Do not rebuild it under new names.

- Project and source-package `CompiledGraphBoundary` values retain graph identity, artefacts, generated lanes and diagnosed or blocked outcomes.
- Boundary-local dense IDs remain separate from stable semantic identities.
- `CompletedSourcePackageRegistry` owns package records, prefix lookup and direct dependency adjacency.
- Retained provider references carry non-optional file-local shell identity.
- Authored and implicit provider inputs are explicit states.
- One operation-local `ProviderInterfaceId` selects one immutable interface and binding view.
- Provider declarations, evidence and summaries use agreement insertion.
- Recursive interface closure uses exact `RecordRef` values and one declaration/evidence queue.
- Final interfaces remain deterministic vectors with construction-only indexes.
- Frozen generic bodies reuse the canonical `TokenKind` vocabulary and exhaustive string-ID traversal.
- Generated declaration lookup resolves an exact artefact and template row.
- Generated summaries and sidecars live in one `CompletedGeneratedFunction` row.
- Generated sessions reuse only their own boundary store and local transaction.
- Equal generated identities may coexist in unrelated project or package boundaries.
- Entry assembly resolves generated functions relative to the calling boundary.
- Generated symbol names remain globally collision-free without making identity globally owned.

Gate A accepted the R5C5B checkpoint. Do not reopen boundary-local generated ownership unless a concrete invariant failure is found.

## Locked implementation decisions

### Generated functions are boundary owned

Concrete generic functions live in sidecars owned by the consuming project or package compilation.

- `GeneratedFunctionIdentity` is unique only inside one boundary.
- Equal identities may exist in independent package boundaries.
- Another package's sidecar never suppresses a local request.
- Generated lookup is scoped by the caller's boundary.
- Local HIR may retain `GeneratedFunctionIdentity` while assembly supplies the boundary.
- "One materialisation per identity" means once per owning boundary.
- Duplicate identity inside one boundary is an invariant failure.
- Package completion order must not change ownership or generated symbol names.

Do not introduce a hybrid global store whose row is owned by whichever package compiles first.

### Graph outcomes are total

`CompiledGraphBoundary::validate_invariants` is the single completion proof for one frontend graph result.

```text
Successful  -> no diagnosed or blocked record
Diagnosed   -> exactly one diagnosed record
Blocked     -> exactly one blocked record
Unavailable -> CompilerError
```

Also reject:

- duplicate diagnosed records
- duplicate blocked records
- diagnosed and blocked overlap
- out-of-range module IDs
- invalid successful artefact references
- graph and slot-count disagreement

`ProjectFrontendCompilation::new` validates the project and every source-package boundary. Success-only assembly adds only the stricter all-successful requirement.

### Publication is transactional

Any multi-row publication preflights before mutation:

1. validate row-local identity agreement
2. reject duplicates inside the input
3. reject duplicates against retained state
4. resolve required dependencies
5. reserve capacity
6. append all rows and indexes

This applies to generated deltas, module materialisation contexts and package publication.

### Exact rows verify identity

An indexed materialisation row must verify:

```rust
artefact.declaration_identity == *input.identity.declaration()
```

A stale but in-range row is `CompilerError`.

### Token remapping is exhaustive, in place and fallible

Keep one canonical exhaustive walker equivalent to:

```rust
pub fn try_remap_string_ids<E>(
    &mut self,
    map: &mut impl FnMut(StringId) -> Result<StringId, E>,
) -> Result<(), E>
```

Ordinary remapping mutates existing payloads. Frozen capture and materialisation clone each token once, then remap the clone. Token locations and path fields use the same fallible contract. Invalid frozen indexes return `CompilerError`.

### Provider facts are imported once

Each consumer/provider pair projects one closed provider interface. Stable declarations, evidence and summaries are stored once. Aliases and namespace members retain stable references.

Agreement insertion borrows the candidate and clones only when the key is vacant. Equal duplicate records are normal for recursively closed facades.

### Dependency syntax remains replaceable

Current import-oriented Rust names are temporary syntax-owner vocabulary.

Phase 5 preserves:

- exact file-local retained shell identities
- retained typed structural provider references from the single preparation pass
- exact shell-to-provider joins
- immutable interfaces
- direct graph edges and package dependency IDs
- no source or path rediscovery after identities exist

Current grouped imports may still retain more than one shell identity because the parser expands selected items. The immediate follow-up plan owns the coherent path-table migration and consolidates one authored clause under one `DependencyShellId`.

Graph, package, provider, closure and binding consumers must not depend on:

- `TokenKind::Import`
- the authored keyword
- raw source spelling
- a second token scan

Do not partially rename `ImportShellId`, `FileImport` or import-oriented modules during this closeout.

### Work limits

Inside one project or package boundary:

- read, tokenize and prepare each selected source once
- resolve each retained shell once
- compile each physical module once
- close and project each provider interface once per consumer/provider pair
- materialise each generated identity once per boundary
- emit one diagnostic set per diagnosed module or request

Directory projects must not use the synthetic single-file scanner. Synthetic mode may retain its isolated shared-parser traversal.

Moth is pre-release. Replace APIs directly and delete old owners. Do not add compatibility wrappers, feature flags or production adapters kept only for tests.

## Slice and review discipline

Each slice names:

- owner
- inputs and outputs
- deleted code
- focused tests
- non-goals
- full validation gate

Standard code gate:

```bash
cargo fmt --all
just validate
```

Stop when:

- a second durable representation appears necessary
- identity would depend on display data
- a full-table clone remains in a hot loop
- more than two unlisted stage boundaries change
- a slice exceeds roughly 12 production files or 600 net production lines, excluding mechanical moves
- a user-facing failure would need `CompilerError`
- the same invariant needs a second correction pass

Review gates:

- **Gate A:** accepted at R5C5B
- **Gate B:** after R5C1B and R5C4B
- **Gate C:** after R5C6A
- **Gate D:** after R5C9

Reviews are read-only. Resolve every required finding before continuing.

## Remaining Phase 5 work

### R5C1B - make graph-outcome validation total

**Goal:** establish one complete frontend graph outcome gate.

**Owners:** `compiled_boundary.rs`, `module_artifact_store.rs`, `compilation.rs`, `build.rs` and boundary tests.

**Changes:**

- validate the complete slot/lane bijection
- reject duplicate and overlapping diagnosed or blocked entries
- reject every final `Unavailable` slot
- validate successful artefact references while walking slots
- validate every source-package boundary in `ProjectFrontendCompilation::new`
- remove redundant downstream completion checks
- change unfinished-slot regressions to expect frontend-boundary rejection
- move `ProjectCompilation::from_test_modules(Vec<Module>)` and synthetic builders into test support
- remove the old flat-module test-construction shape from production `build.rs`

**Tests:**

- both mismatch directions for diagnosed and blocked states
- duplicate and overlap cases
- unavailable project and package slots
- missing successful artefact rows
- valid mixed outcomes retained for `check`

**Non-goals:** scheduler, diagnostic cascade and `CompiledModuleRef` redesign.

### R5C4B - exact remapping, lookup and publication

**Goal:** retain checkpoint correctness while removing avoidable allocation and partial mutation.

**Owners:** `tokenizer/tokens.rs`, frozen generic syntax and materialisation, `generated_worklist.rs`, `module_artifact_store.rs` and `compiled_boundary.rs`.

**Changes:**

- replace reconstructive token mapping with one exhaustive in-place fallible walker
- route frozen token, source-location and path remapping through it
- verify declaration identity at the exact template row
- preflight complete generated deltas before append
- validate `record.identity == record.sidecar.identity` at boundary publication
- preflight module and package materialisation rows before mutation
- resolve package dependencies before publishing the package row
- retain deterministic materialisation rows in contiguous order plus one lookup map
- replace symmetric project/package duplicate scans with one deterministic registration pass
- make agreement insertion borrow and clone only on vacant entries
- assign generated names inside a boundary from stable generated identity order
- reuse one shared empty generated-name map rather than allocating per lookup

**Tests:**

- all token payloads remap in place and path vectors keep their allocation
- invalid frozen token or location indexes return `CompilerError`
- stale in-range template rows fail identity validation
- late generated or package duplicates leave owners unchanged
- sidecar/record identity disagreement leaves the store unchanged
- deterministic materialisation iteration survives reversed insertion order
- occupied agreement insertion avoids cloning where instrumentation can prove it
- generated names remain stable under sidecar publication reordering

**Delete:**

- reconstructive ordinary remapping
- panic-prone frozen remap indexing
- incremental multi-row publication
- nondeterministic iteration described as deterministic
- repeated empty generated-name allocations

Run the standard gate after R5C1B and R5C4B, then stop for Gate B.

### R5C6A - instrument and build the read-only convergence graph

**Goal:** measure current work and prove node ownership before scheduling changes.

**Precondition:** Gate B accepted.

Instrument:

- base-module and sidecar borrow passes
- complete summary-map clones
- summary changes
- dirty nodes and SCCs
- SCC sizes and maximum iterations
- stable nodes revisited after convergence

Build one read-only deterministic graph over boundary-local source, private and generated functions. Stable cross-module source summaries and provider summaries are fixed leaves. Another package's generated sidecar is never a node or leaf in the current boundary.

Requirements:

- derive edges from stable HIR targets and retained link facts
- preserve current borrow execution and results
- make boundary ownership explicit in node identity
- expose focused test inspection without retaining a second graph authority
- compare predicted dirty sets with current rechecks

Tests cover:

- equal generated identities in two boundaries
- provider leaves
- independent SCCs
- reversed discovery order
- unchanged diagnostics and summaries

Run the standard gate and stop for Gate C.

### R5C6B - dependency-driven call-summary convergence

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

If function-granular scheduling requires a broad borrow-checker rewrite, stop and create a dedicated plan. A module-level fallback is acceptable only when counters prove unrelated stable sidecars are not rechecked.

Tests cover reverse-reachable dirtying, recursive convergence, independent boundaries, oscillation failure and parity with the baseline.

### R5C7 - remove prepared-source payload cloning

**Goal:** preserve one read and tokenization without complete payload copies.

- keep `PreparedSourceStore` indexed by `SourceId`
- move canonical payloads into their single preparation owner
- share one immutable source allocation only for a real second consumer
- do not clone `FileTokens` before header preparation
- rebind source identity without copying tokens
- prevent diagnosed preparation from being consumed twice
- retain typed path syntax facts so the dependency/path plan can replace grammar without rescanning
- do not narrow storage around import-only facts
- do not implement `Path`, resource identity or asset publication here

Synthetic single-file traversal may retain its isolated cache.

### R5C8 - simplify ownership and preserve the dependency boundary

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
- package indexing and ordering in one owner
- provider input construction in one owner
- graph and provider consumers accept typed retained references only
- no directory owner imports tokenizer-specific dependency grammar or branches on `TokenKind::Import`
- generated materialisation and convergence stay outside base semantic orchestration
- replace long compile-wave argument lists with one narrow immutable context
- split generic materialisation only at real ownership boundaries
- consolidate generated symbol-name maps into one owner when their lifecycle matches
- return borrowed views instead of rebuilding vectors when simple
- keep import-oriented filenames temporary for the follow-up migration
- remove stale comments, forwarding modules, compatibility derefs and production test adapters
- keep tests with their owner
- do not add stage traits or dynamic dispatch

Do not split files mechanically before ownership is clear.

### R5C9 - deletion audit and Phase 5 validation

Audit:

**Source and graph**

- no directory fallback scanner or `ProjectPathResolver` source fallback
- synthetic traversal stays isolated with one read and tokenization
- no source/token reparse after retained dependency facts exist
- no graph/provider consumer depends on `TokenKind::Import`
- no suffix or path matching for provider joins

**Provider and interface**

- no donor header, AST or HIR copying
- no first-provider-wins record
- no optional retained shell identity
- no provider or closure hot-path linear scan
- no complete payload clone per alias or namespace member
- no occupied agreement-path clone

**Generated and boundaries**

- no cross-boundary request suppression or global identity-only owner map
- equal generated identities may coexist across boundaries
- no consumer-local generic materialisation or context scan by identity
- no parallel summary and sidecar stores
- no partial generated or materialisation publication
- no `Unavailable` retained slot
- every project and package boundary passes the full outcome bijection

**Token and ownership**

- no mirrored token vocabulary
- no ordinary remap that rebuilds path vectors
- no infallible frozen remap indexing
- no full declaration-table clone in hot loops
- no complete prepared-source clone per handoff
- no flat `Vec<Module>` production test constructor
- no compatibility API

**Handoff**

- dependency grammar can be replaced without graph, package, provider or interface changes
- current import-specific names are not public or persistent identity
- retained path/dependency facts can move to one file-owned path table without another source pass
- no resource behaviour is implemented or reconstructed from rendered strings in this plan

Final gate:

```bash
cargo fmt --all
cargo run --quiet -- tests --audit
just validate
just bench-ci
```

Counters must prove:

- one source read, tokenization and preparation
- one module compilation
- one interface publication
- one provider projection per consumer/provider pair
- one generated materialisation per identity per boundary
- no unrelated SCC recheck

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
- full validation and Gates B through D are clean

After acceptance, compress the detailed R5C section to one accepted-baseline paragraph before archiving this plan.

## Mandatory post-Phase-5 handoff

Stop after Gate D. Do not continue path values, resource linking, asset unions or incremental resource work from this plan.

The implementation chain is:

```text
canonical module Phase 5 Gate D
-> dependency clauses and path syntax
-> TIR corrections and simplification
-> Path values and resource linking
```

Phase 5 hands the dependency-clause plan:

- exact retained file-local shell identities
- one preparation pass
- typed structural provider references
- exact shell/provider joins
- immutable interfaces
- boundary-local graph and package identities
- strict facade and support visibility
- no dependency rediscovery after identities exist

The dependency-clause plan owns:

- one file-owned path syntax table
- one `DependencyShellId` per authored clause
- removal of the `import` keyword
- direct top-level dependency clauses
- coherent dependency terminology

The later resource plan owns:

- builtin compile-time `Path`
- stable resource identity and source registry
- TIR and HIR resource anchors
- public Path values
- exact entry resource unions
- provider resources and builder placement

## Validation policy

Focused tests are iteration evidence, not the acceptance gate. Run the architecture audit whenever a slice changes discovery, dependency shells, provider binding, interface closure, identity, borrow summaries, graph scheduling or generated sidecars.

## Deferred beyond Phase 5

Immediate follow-ups:

- dependency-clause grammar and path syntax
- TIR corrections and simplification
- Path values and resource linking

Later:

- complete package assemblies over final resource facts
- backend handoff, fingerprints and incremental semantic reuse
- persistent artefact serialization and caches
- package registries, remote fetching, versions and lockfiles
- direct normal-sibling dependencies
- cross-entry browser chunking
- Wasm physical partition and Component Model integration
- cross-build generated-instance caches
- measured module-wave parallelism unless benchmarks justify it
