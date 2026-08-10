# Canonical module compilation and scoped packages - Phase 5 closeout plan

## Purpose

Finish the canonical module and scoped-package cutover, then hand one syntax-independent dependency substrate to the dedicated dependency-clause and path-syntax plan.

Phase 5 targets:

- one semantic compilation per physical module inside one project or package boundary
- one deterministic source inventory and one preparation pass per consumed source
- immutable completed provider interfaces rather than donor headers, AST or HIR
- stable cross-module identities and generated sidecars owned by the consuming boundary
- complete retained graph outcomes and success-only linkable project payloads
- strict scoped support packages and module-root-relative dependency resolution
- dense build-local IDs, contiguous records and narrow operation-scoped indexes
- no compatibility path, duplicate semantic owner or speculative duplicate work
- no graph or provider consumer coupled to the current `import` keyword

Phase 5 does not implement the new dependency grammar, builtin `Path`, resource identity, resource linking or asset publication.

## Current state

```text
ACTIVE_PLAN: docs/roadmap/plans/canonical-module-compilation-and-scoped-packages-plan.md
WORK_ID: R5-closeout
WORK_SOURCE: continued Phase 5 closeout after Gate B acceptance and the revised plan
BASE_REVISION: 276bc4cb2 (clean revised-plan baseline before R5C6A)
IMPLEMENTED_CHECKPOINT: 4cbf93829
RECONCILED_HEAD: 4cbf93829
STATUS: active - R5C10C complete; second independent review pending
CURRENT_SLICE: R5C10C - canonical package discovery, requester template indexing and generated evidence visibility (complete)
ACCEPTED_CHECKPOINTS:
- R5C3C provider agreement and recursive interface closure
- R5C4A exhaustive canonical token traversal
- R5C5B boundary-scoped generated ownership and caller-scoped lookup
- R5C4B exact in-place remapping, exact template-row identity and transactional publication
- R5C1B/R5C1C total retained-boundary completion and identity validation
REQUIRED_RELOADS: AGENTS.md, this plan, compiler-design-overview.md, build-system-design.md, frontend_orchestration.rs, generated_worklist.rs, public_call_summary.rs, borrow_checker/metadata.rs, prepared_source.rs, module_inventory.rs, source_tree_index.rs and the focused generated/source tests
VALIDATION_STATE:
- fd16cf7b7 passed full just validate: 4083 workspace tests, 1818/1818 integration executions, cross-target Clippy, docs and bench-ci
- later timing-system code at 5d2918faf passed its own full just validate and timer erasure gates
- 63ea2d6 is a plan-only pause checkpoint validated with the docs release build and diff checks
- 766bb827c1 passed full just validate: 4210 workspace unit tests, 1818/1818 integration executions, cross-target Clippy, docs, benchmark sanity and timer erasure
- a4e3d50ab passed full just validate: 4211 workspace unit tests, 1818/1818 integration executions, cross-target Clippy, docs, 60 benchmark preflights plus quick benchmark sanity and timer erasure
- Gate C1 automation remained intentionally deferred under the explicit user instruction; R5C6B now replaces broad convergence with the accepted monotone dirty queue
- User-provided Gate C2 review identified active-base public-call classification, coarse base invalidation, repeated materialisation-context scans and missing production queue coverage
- a9460bf94 passed full just validate: 4218 workspace tests, 1818/1818 integration executions, cross-target Clippy, docs, 60 benchmark preflights plus quick benchmark sanity and timer erasure
- R5C6C focused coverage passed: 12 convergence tests and the timer/counter active-base public pipeline regression
- 7cb61bcb3 passed full just validate: 4218 workspace tests, 1818/1818 integration executions, cross-target Clippy, docs, 60 benchmark preflights plus quick benchmark sanity and timer erasure
- R5C7A evidence found one owning module queue per selected compiler-semantic `SourceId`; existing Stage 0 coverage covers duplicate same-module paths, cross-module edges, support roots, package boundaries, unrooted sources and reversed discovery order
- R5C7B deleted the project-wide prepared-source store, moved direct owned inputs through the module queue, removed `PreparedSourceInput::Clone`, and narrowed Moth inputs to tokens plus byte length; the synthetic single-file cache remains isolated
- R5C8 initial cleanup grouped the immutable boundary inputs to `compile_module_waves` in one typed context without changing graph, provider or package ownership
- User audit accepted R5C6A/R5C6B/R5C6C and identified the remaining R5C7C blocker; the source-payload correction and R5C8 consolidation are now complete
- Interim implementation-coordinator auditor `20260809T190234Z-f3a404aa` identified four bounded corrections; all were resolved and revalidated
- `156942cc3` passed full `just validate`: 4218 workspace tests, 1818/1818 integration executions, cross-target Clippy, docs, 60 benchmark preflights plus quick benchmark sanity and timer erasure
- User review accepted `156942cc3` and identified only three non-blocking R5C9 cleanup checks: diagnostic-owner coupling, singleton per-file preparation allocation and a stale module-map description
- R5C9 audit inventory passed `cargo run --quiet -- tests --audit`: 1668 integration cases and 1818 backend executions
- R5C9 focused tests passed: frontend orchestration 22, convergence 12 and frontend pipeline 40
- R5C9 working tree passed full `just validate`: 4218 workspace tests, 1818/1818 integration executions, cross-target Clippy, docs, all 60 benchmark preflights plus quick benchmark sanity and timer erasure
- Interim auditor `20260809T204134Z-4d73f013` found one stale `TraversalSourceStorage` ownership comment; the required correction is complete
- Post-correction `just validate` and `cargo run --quiet -- tests --audit` passed with the same full counts and clean timer-erasure/source audit
- Verification auditor `20260809T205404Z-6575bf06` found one test-owner correction; the warning/context regression now lives under compiler-messages tests and the obsolete frontend-orchestration ownership text is removed
- Final post-correction `just validate` passed: 4218 workspace tests, 1818/1818 integration executions, cross-target Clippy, docs, all 60 benchmark preflights plus quick benchmark sanity and timer erasure
- User Gate D review rejected `6cddcc38c`: exported generic receiver methods are not retained/imported end to end; publication preflight/commit APIs remain fallible after mutation; generic materialisation closure/evidence is duplicated per template; and closeout ownership/test placement remains incomplete
- R5C10 interim audit `20260810T004421Z-0a8e7c39` found one package-lane allocation after mutation, missing ownership docs on the two moved test owners and one imported generic-bound negative test gap; all required corrections were resolved
- R5C10 focused validation passed: workspace check, combined publication and package-registry tests, and eight generic receiver/evidence integration cases
- R5C10 `cargo run --quiet -- tests --audit` passed: 1675 integration cases and 1825 backend executions
- R5C10 `just validate` passed: 4224 workspace tests, 1825/1825 integration executions, cross-target Clippy, docs, all 60 benchmark preflights plus quick benchmark sanity and timer erasure
- R5C10 correction focused validation passed: cargo check, declaration-table tests, 15 generic-function unit tests, 81 receiver-method integration cases, the existing private-generic facade case and the new private receiver regression
- R5C10 post-audit correction checks passed: cargo fmt --all -- --check, declaration-table tests and git diff --check
- R5C10 correction serialized full gate passed with `RUST_TEST_THREADS=1 just validate`: 4225 workspace tests, 1826/1826 integration executions, cross-target Clippy, docs, all 60 benchmark preflights plus quick benchmark sanity and timer erasure
- R5C10 correction `cargo run --quiet -- tests --audit` passed: 1676 integration cases and 1826 backend executions
- R5C10 correction `just bench-ci` passed: all 60 benchmark preflights and the selected CLI/frontend benchmark cases
- R5C10B passed the normal default `just validate`: 4225 workspace tests, 1826/1826 integration executions, cross-target Clippy, docs, all 60 benchmark preflights plus quick benchmark sanity and timer erasure; the shared instrumentation lock resolved the prior parallel benchmark-timing race
- R5C10B `cargo run --quiet -- tests --audit` passed: 1676 integration cases and 1826 backend executions
- R5C10B focused private receiver evidence case passed HTML compilation and rendering
- R5C10B regular auditor `20260810T112902Z-2655eca7` returned `audit_clean`; the read-only child could not rerun Cargo gates, so the coordinator retained the recorded validation evidence
- R5C10B documentation release build passed and regenerated the progress page output from its committed source
- R5C10C structural package correction now passes the full integration inventory: 1826/1826 cases, including structural support-facade re-exports, API-only support-root rejection, ignored legacy package-folder configuration and scoped support visibility
- R5C10C exact gate passed: `cargo fmt --all`, `cargo run --quiet -- tests --audit` (1676 cases, 1826 backend executions), `just validate` (4222 workspace tests, 17 auxiliary tests, 641 xtask tests, 1826/1826 integration cases, docs, benchmark sanity and timers-erasure), and standalone `just bench-ci` (60/60 benchmark preflights)
AUDITS: interim auditor `20260810T093610Z-d64f9dcf` found one stale DeclarationId comment; corrected; pass-two auditor `20260810T094147Z-9376c565` found no required correction; interim auditor `20260810T105710Z-703236c2` found no required correction; regular auditor `20260810T112902Z-2655eca7` found no required correction
BLOCKERS: second independent review, final auditor and Gate D remain pending; strict entry_root remains deferred to the queued Project Config plan
NEXT_WORKER_ORDER: reconcile the second independent review -> final auditor -> Gate D -> compress/archive canonical Phase 5 -> dependency-clauses-and-path-syntax-plan.md
STOP_REASON: R5C10C is validated and checkpoint-ready; paused for the second independent review before the final-audit boundary
NEXT_RESUME_ACTION: inspect the second independent review against R5C10C; resolve only valid findings, then run the final auditor and Gate D
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

The compiler and build-system overviews own semantic and orchestration boundaries. The language overview owns current syntax. This plan owns only the remaining Phase 5 sequence and its handoff.

## Accepted Phase 5 baseline

Keep this work. Do not rebuild it under new names.

- Project and source-package `CompiledGraphBoundary` values retain graph identity, artefacts, generated lanes and diagnosed or blocked outcomes.
- `CompiledGraphBoundary::finish` proves the dense slot, outcome, graph-node and artefact-row relationships before a boundary becomes a provider.
- Boundary-local dense IDs remain separate from stable semantic identities.
- `CompletedSourcePackageRegistry` owns package rows, prefix lookup, dependency adjacency and direct materialisation lookup.
- Retained provider references carry non-optional file-local shell identity.
- Authored and implicit provider inputs are explicit valid states.
- One operation-local `ProviderInterfaceId` selects one immutable interface and one binding view.
- Provider declarations, evidence and summaries use agreement insertion.
- Recursive interface closure uses exact `RecordRef` values and one declaration/evidence queue.
- Final interfaces remain deterministic vectors with construction-only indexes.
- Frozen generic bodies reuse the canonical `TokenKind` vocabulary and exhaustive in-place string-ID traversal.
- Generated declaration lookup resolves an exact artefact and template row.
- Generated summaries and sidecars live in one `CompletedGeneratedFunction` row.
- Generated sessions reuse only their own boundary store and local transaction.
- Equal generated identities may coexist in unrelated project or package boundaries.
- Entry assembly resolves generated functions relative to the calling boundary.
- Generated symbol names remain globally collision-free without making generated identity globally owned.
- Timing and benchmark instrumentation now use one typed schema and erasing facade. Remaining Phase 5 instrumentation must extend those owners rather than create another timing path.

Gates A and B are accepted. Reopen them only for a concrete invariant failure.

## Locked implementation decisions

### One owner per semantic or scheduling fact

- `SourceTreeIndex` owns physical source inventory, `SourceId`, source ownership and portable source identity.
- `ProjectModuleGraph` owns module topology and provider-before-consumer scheduling.
- Completed provider interfaces own cross-module semantic facts.
- Validated HIR call targets own executable call dependency facts.
- `BoundaryGeneratedFunctionStore` owns completed generated summaries and sidecars for one boundary.
- Entry and package assembly own reachability and root activation.

A later stage must not reconstruct one of these facts from paths, rendered names, source text, another IR or a parallel graph.

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

### Retained graph outcomes are total

Every boundary finishes before publication:

```text
Successful  -> one matching graph node and one artefact row
Diagnosed   -> exactly one diagnosed record
Blocked     -> exactly one blocked record
Unavailable -> CompilerError
```

Successful artefact interface origin must equal the graph-node origin. No artefact row may be missing, shared or orphaned. Success-only project assembly adds only the stricter all-successful condition.

### Publication is transactional

Any multi-row publication preflights before mutation:

1. validate row-local identity agreement
2. reject duplicates inside the input
3. reject duplicates against retained state
4. resolve required dependencies
5. reserve capacity
6. append all rows and indexes

This applies to generated deltas, module materialisation contexts and package publication.

### Provider facts are imported once

Each consumer/provider pair projects one closed provider interface. Stable declarations, evidence and summaries are stored once. Aliases and namespace members retain stable references.

Agreement insertion borrows the candidate and clones only when the key is vacant. Equal duplicate records are normal for recursively closed facades.

### Convergence uses validated HIR as its topology authority

The convergence scheduler consumes actual lowered call targets. It does not use source requests, path names or a second persistent graph.

The analysis unit is deliberately coarse:

```rust
pub enum ConvergenceNode {
    BaseModule,
    Generated(GeneratedFunctionIdentity),
}
```

- All local and module-private functions stay inside `BaseModule` because the current borrow checker analyses a complete `HirModule`.
- Each generated sidecar is one node.
- A local call within one node creates no cross-node edge.
- `ModulePrivate` from a sidecar targets `BaseModule`.
- `Generated` targets the owning boundary's generated node.
- Provider/cross-boundary source and binding-backed summaries are fixed leaves, not nodes. A
  generated sidecar's `CrossModule` target may name a public executable in the active compilation
  boundary and then depends on `BaseModule`.
- Another package's generated sidecar is neither a node nor a leaf in the current boundary.

Per-function HIR and link facts may derive these edges. They do not create a second function-granular borrow scheduler.

### Call-summary convergence must be monotone

R5C6 must formalise and validate the existing finite summary order before changing scheduling.

Invariant fields must remain equal:

- parameter count and declared access
- transfer eligibility
- transfer effect

Widening fields are:

- mutation: `NoWrite <= Writes`
- reactive effects: retained subscription and invalidation bits may only be added
- return alias:
  - `Fresh` is the least conservative value
  - `AliasParams(A) <= AliasParams(B)` only when `A` is a subset of `B`
  - any `Fresh` or alias value may widen to `Unknown`
  - `Unknown` is the top value

`AliasParams` remains sorted and unique. A recomputation that narrows, changes invariant fields or moves between incomparable alias sets is `CompilerError` and stops this plan. Do not hide non-monotonic behaviour by silently joining it to a less precise result.

### Source payloads are not a cache by default

Canonical source ownership already assigns one source to one module. Same-module traversal deduplicates by `SourceId`, while cross-module dependencies create graph edges rather than adding provider source to a consumer source set.

R5C7 must prove the number of real preparation consumers per canonical `SourceId`. When the production count is one, delete the payload cache and move the source payload once. Shared ownership is allowed only when a real second consumer exists.

Do not retain a clone-heavy cache for hypothetical future reuse.

### Token remapping remains exhaustive, in place and fallible

Ordinary remapping mutates existing payloads. Frozen capture and materialisation clone each token once, then remap the clone. Token locations and path fields use the same fallible contract. Invalid frozen indexes return `CompilerError`.

### Dependency syntax remains replaceable

Current import-oriented Rust names are temporary syntax-owner vocabulary.

Phase 5 preserves:

- exact file-local retained shell identities
- retained typed structural provider references from the single preparation pass
- exact shell-to-provider joins
- immutable interfaces
- direct graph edges and package dependency IDs
- no source or path rediscovery after identities exist

Current grouped imports may still retain more than one shell identity because the parser expands selected items. The immediate follow-up plan owns the coherent path-table migration and one `DependencyShellId` per authored clause.

Graph, package, provider, closure and binding consumers must not depend on:

- `TokenKind::Import`
- the authored keyword
- raw source spelling
- a second token scan

Do not partially rename `ImportShellId`, `FileImport` or import-oriented modules during Phase 5.

### Instrumentation uses existing owners

- Use current typed `TimingMetric` spans for elapsed-time evidence.
- Add structural work counts through `FrontendCounter` under `benchmark_counters`.
- Use private test statistics for graph and queue invariants.
- Do not add raw timing names, direct clock reads or a second collector.
- Do not change timing schema version unless an existing metric's semantic boundary changes.

### No compatibility scaffolding

Moth is pre-release. Replace APIs directly and delete old owners. Do not add compatibility wrappers, forwarding shims, feature flags, fallback branches or production adapters kept only for tests.

## Slice and review discipline

Every slice names its owner, inputs, outputs, deletions, tests, non-goals and final gate.

Standard code gate:

```bash
cargo fmt --all
just validate
```

Focused tests and timings are iteration evidence, not acceptance gates.

Stop and request parent review when:

- a second durable representation of one fact appears necessary
- identity would depend on display data or donor-local IDs
- HIR call topology and a retained request topology disagree
- a summary transition is non-monotone
- a canonical source has more than one real preparation consumer
- a full-table clone remains inside a hot loop after the owning slice
- more than two unlisted stage boundaries must change
- a slice exceeds roughly 12 production files or 600 net production lines, excluding mechanical moves
- a user-facing failure would need `CompilerError`
- the same invariant needs another correction pass

Review gates:

- **Gate A:** accepted at R5C5B
- **Gate B:** accepted at R5C1C
- **Gate C1:** after R5C6A, before convergence behaviour changes
- **Gate C2:** after R5C6B, before source-payload ownership changes
- **Gate D:** after R5C9 deletion audit, R5C10/R5C10C corrections and the final auditor

Reviews are read-only. Corrections land as separate bounded slices.

## Remaining Phase 5 work

### R5C6A - instrument and prove the convergence model

**Goal:** measure the existing fixed point, formalise monotonicity and build one read-only dependency model without changing borrow results.

**Owners:** `frontend_orchestration.rs`, `generated_worklist.rs`, `public_call_summary.rs`, borrow-checker metadata, frontend counters and focused generated tests.

#### R5C6A1 - baseline work evidence

Add benchmark-only counters for:

- initial base-module borrow passes
- convergence base-module borrow passes
- generated sidecar borrow passes
- complete generated-summary map builds
- generated-summary map clones into sidecars
- private-summary map rebuilds
- summary comparisons and changes
- stable sidecars rechecked without an input change
- maximum current convergence iterations

Use the existing timing metrics for initial borrow, convergence borrow and generated rechecks. Do not add a timing metric for every counter.

Record a focused baseline from fixtures containing:

- no generated functions
- independent generated functions
- a generated-to-generated chain
- a generated-to-module-private call
- a base-to-generated-to-base cycle
- equal generated identities in two separate package boundaries

Counters are attribution evidence only. Existing diagnostics, summaries and output must remain byte-for-byte or structurally identical under their current owners.

#### R5C6A2 - monotonicity contract

Add one narrow summary-transition validator under the shared call-summary owner.

It must:

- validate invariant fields
- validate the mutation and reactive partial orders
- validate sorted unique alias parameter sets
- validate the return-alias partial order
- distinguish `Unchanged` from `Widened`
- reject narrowing or incomparable transitions through `CompilerError`

Do not introduce a generic lattice framework.

Add focused unit tests for every allowed and rejected transition.

#### R5C6A3 - transient dependency model

After all local sidecars have materialised, build one construction-only dense model:

```rust
pub struct ConvergenceModel {
    nodes: Vec<ConvergenceNodeRecord>,
    callers: Vec<Vec<ConvergenceNodeId>>,
}
```

Requirements:

- node zero is `BaseModule`
- generated nodes are assigned in stable `GeneratedFunctionIdentity` order
- caller lists are sorted and deduplicated
- edges come only from validated HIR call targets
- local calls inside one analysis unit are ignored
- provider and cross-boundary summaries are validated fixed leaves; active-base public
  `CrossModule` targets are base dependencies
- the model is dropped after one module compilation
- no hash-map iteration order affects IDs, counters or diagnostics

Compare the model's predicted dirty callers with the nodes currently rechecked by the broad loop. Do not use the model to skip work in R5C6A.

`GeneratedRequestRecord.requesters` and `dependencies` currently describe materialisation history rather than final HIR calls. Search all production consumers. When they have no independent diagnostic or scheduling owner, delete them in this slice instead of retaining a second topology. Recursive materialisation detection remains state-based.

**Tests:**

- base/generated edge classification for every HIR call target class
- deterministic node IDs and caller order under reversed materialisation order
- equal identities in separate boundaries remain isolated
- worklist construction topology is absent or demonstrably non-authoritative
- predicted dirty sets match hand-constructed fixtures
- all existing borrow summaries and diagnostics remain unchanged

**Non-goals:** skipping borrow passes, partial borrow reports, function-granular scheduling or source-payload changes.

**Gate:** run the standard code gate and stop for Gate C1.

### R5C6B - replace broad convergence with a monotone dirty queue

**Goal:** remove unconditional base/sidecar rechecking and complete-summary cloning while preserving exact final reports.

**Precondition:** Gate C1 accepts the monotonicity contract and read-only model.

**Owners:** a focused generated-summary convergence owner, `frontend_orchestration.rs`, `generated_worklist.rs` and borrow-check integration.

#### Initial state

- Keep the existing initial base borrow analysis.
- Materialise every requested sidecar exactly once.
- Build the accepted convergence model.
- Seed every node once in deterministic node order. This preserves a conservative parity baseline without repeated global passes.

#### Dirty scheduling

Use one dense `VecDeque<ConvergenceNodeId>` plus a queued bitset.

For one node:

1. install only the direct callee summaries required by that node
2. run the existing complete-HIR borrow analysis for that node
3. compare its produced summary facts with the retained facts
4. reject non-monotone transitions
5. publish widened facts
6. enqueue only direct callers when at least one published fact changed

`BaseModule` may produce several local or module-private summary changes but remains one analysis unit. A generated node produces one completed generated summary.

The queue terminates when empty. Do not retain an arbitrary iteration multiplier after every transition has a finite validated widening order.

#### Data-flow cleanup

Delete or replace these broad paths:

- `GeneratedFunctionWorklist::completed_summaries` full-map construction
- cloning the complete generated summary map into every sidecar
- rebuilding the complete private summary map every pass
- `recheck_generated_borrows` as an all-sidecar loop
- the `(functions + requests) * 4` convergence limit
- warning-vector clones where a borrowed warning slice is sufficient

Each node retains or reconstructs only the direct summary entries its HIR calls require. Do not place references into persistent HIR or artefacts.

#### Correctness

- imported source and binding summaries remain fixed
- another package's generated summary never enters the boundary model
- final base and sidecar `BorrowCheckReport` values are retained on their existing module owners
- user diagnostics remain deterministic
- no source program becomes accepted or rejected because of scheduling order

**Tests and counters:**

- independent sidecars stop after their own initial stable pass
- a changed generated summary rechecks only reverse-reachable callers
- generated-to-base and base-to-generated cycles converge
- recursive alias summaries widen to the same final results as the baseline
- a forced narrowing or oscillating test transfer fails through `CompilerError`
- reversing generated publication order preserves final summaries, diagnostics and output
- complete summary-map clone counters fall to zero
- unchanged nodes are not reanalysed

If this requires partial `BorrowChecker` execution, function-report merging or mutation of validated HIR, stop and create a dedicated incremental-borrow plan. Do not expand R5C6B.

**Gate:** run the standard code gate and stop for Gate C2.

### R5C6C - exact active-base summary dependencies

**Goal:** finish the boundary between the coarse `BaseModule` borrow-analysis unit and the exact stable summaries it publishes before changing source-payload ownership.

**Completed:**

- classify `CrossModule(origin)` against the active base HIR: active public origins depend on `BaseModule`; provider and cross-boundary origins remain immutable fixed leaves
- retain active-base public and private callees in the construction-only convergence model
- resolve direct base summaries from the current base HIR identity maps and current `BorrowCheckReport`, with materialisation preparation used only for bootstrap and final contract installation
- publish base changes as exact widened public/private identities and requeue only generated callers whose direct inputs intersect those identities
- preserve the dense queue, queued bitset, stable generated identity order, adjacent duplicate rejection and provider-private fixed-leaf boundary
- add model, orchestration, narrowing-transition and production pipeline coverage, including counter assertions for generated rechecks and zero legacy complete-map/recheck paths

The correction does not introduce function-granular borrow scheduling, SCC machinery, another durable dependency graph or source-payload changes. `BaseModule` remains one complete-HIR borrow-analysis unit and each generated sidecar remains one convergence node.

**Gate:** `just validate` passed; stop for Gate C2 re-review. R5C7A has not started.

### R5C7A - prove source-payload ownership

**Goal:** establish whether the prepared-source cache has any real multi-consumer requirement before changing storage.

**Owners:** `module_inventory.rs`, `prepared_source.rs`, `source_tree_index.rs` and focused Stage 0 tests.

Prove for directory-project and source-package compilation:

- every compiler-semantic `SourceId` has one `SourceOwnership::Owned(ModuleId)` or is explicitly unrooted
- one module queue consumes each selected owned `SourceId` at most once
- same-module duplicate dependencies collapse through the queued set
- cross-module dependencies add graph edges and never enqueue provider source
- check-only and tooling paths do not currently consume the same canonical payload twice

Use a construction-only dense consumption table or benchmark counters when runtime evidence is useful. Do not retain source payloads merely to collect the proof.

**Stop condition:** when a real second consumer exists, stop and document its owner, lifetime and exact payload need before choosing shared storage.

**Tests:** repeated same-module paths, cross-module imports, support roots, package boundaries, unrooted sources and reversed source discovery order.

### R5C7B - delete or narrow prepared-source storage

**Goal:** move each canonical payload once and remove clone-heavy migration storage.

**Default path when R5C7A proves one consumer:**

- delete `PreparedSourceStore`, `PreparedSourceSlot` and `PreparedSourceEntry`
- load and tokenize one `SourceId` directly into an owned `PreparedSourceInput`
- consume or drop that input after retained header syntax is produced
- remove `Clone` from `PreparedSourceInput`
- remove test-only `input_files` clones from module jobs
- keep `SourceTreeIndex` as the identity, path and ownership owner

Narrow `PreparedSourceInput` where current consumers allow:

```rust
Moth {
    source_path: PathBuf,
    source_byte_len: usize,
    tokens: Box<FileTokens>,
}
```

Do not retain the complete Moth source string after tokenization when no diagnostic or semantic consumer needs it. Template and Markdown variants retain their source text because their preparation path consumes it.

**Fallback only for a proven second consumer:** use one source-level immutable shared payload. Do not clone complete strings or token buffers and do not add per-token reference counting. This fallback requires parent review before implementation.

Retain the typed structural path facts produced by header preparation so the dependency-clause plan can replace grammar without another source scan.

**Tests and counters:** one read, one tokenization, one header preparation and no complete token/source clone per consumed source. Synthetic single-file traversal remains isolated and follows the same one-read contract.

Run the standard code gate before R5C8.

### R5C8 - deletion-first ownership consolidation

**Goal:** make the completed pipeline readable after R5C6 and R5C7 without changing semantic data flow.

**Current progress:** R5C7C now moves each retained Moth token stream by value through header
preparation, validates source ownership before I/O and removes test-only module payload retention.
R5C8 keeps request registration and generated materialisation in `frontend_orchestration.rs`, while
the transient HIR topology, dirty queue and exact summary publication live in
`generated_summary_convergence.rs`; the boundary context also precomputes implicit template package
IDs. No semantic graph, provider or package ownership changed.

First delete stale helpers, fields, comments and test adapters revealed by the previous slices. Split files only around surviving owners.

Likely owners, when still justified:

```text
create_project_modules/
├── compiled_boundary.rs
├── module_artifact_store.rs
├── package_registry.rs
├── graph_compile.rs
└── frontend/
    ├── preparation.rs
    ├── semantic_compile.rs
    ├── generated_materialisation.rs
    └── generated_summary_convergence.rs
```

This tree is illustrative, not a requirement to create every file.

Requirements:

- one obvious directory-project compilation coordinator
- package indexing, package ordering and package publication in one owner
- generated materialisation separate from generated summary convergence
- one narrow immutable context instead of the long `compile_module_waves` argument list
- provider and graph consumers accept typed retained references only
- no directory owner imports tokenizer-specific dependency grammar or branches on `TokenKind::Import`
- no stage traits or dynamic dispatch
- no forwarding module or wrapper left after a move
- tests move with the owner and are not duplicated
- import-oriented filenames remain until the immediate follow-up migration
- comments describe the current serial module-wave policy rather than claiming semantic wave parallelism

Prefer direct functions and dense data over registries or generic orchestration abstractions. Do not mechanically split files to match the diagram.

Run the standard code gate.

### R5C9 - deletion audit and Phase 5 validation

Perform the `AGENTS.md` Final audit in order, then run this plan-specific audit.

#### Source and graph

- no directory fallback scanner or `ProjectPathResolver` source fallback
- synthetic traversal stays isolated with one read and tokenization
- no source or token reparse after retained dependency facts exist
- no graph or provider consumer depends on `TokenKind::Import`
- no suffix or path matching for provider joins
- no clone-backed `PreparedSourceStore` or equivalent speculative payload cache
- no test-only full source/token clone retained in module jobs

#### Provider and interface

- no donor header, AST or HIR copying
- no first-provider-wins record
- no optional retained shell identity
- no provider or closure hot-path linear scan
- no complete payload clone per alias or namespace member
- no occupied agreement-path clone

#### Generated and boundaries

- no cross-boundary request suppression or global identity-only owner map
- equal generated identities may coexist across boundaries
- no consumer-local generic materialisation or context scan by identity
- no parallel summary and sidecar stores
- no partial generated or materialisation publication
- no `Unavailable` retained slot
- every project and package boundary passes the full outcome bijection
- no retained materialisation-history graph competing with HIR convergence topology
- no complete generated-summary map clone
- no complete private-summary map rebuild per iteration
- no broad all-sidecar recheck
- no arbitrary convergence multiplier

#### Token and ownership

- no mirrored token vocabulary
- no ordinary remap that rebuilds path vectors
- no infallible frozen remap indexing
- no full declaration-table clone in hot loops
- no flat `Vec<Module>` production test constructor
- no compatibility API

#### Handoff

- dependency grammar can be replaced without graph, package, provider or interface changes
- current import-specific names are not public or persistent identity
- retained path/dependency facts can move to one file-owned path table without another source pass
- no resource behaviour is implemented or reconstructed from rendered strings in this plan

#### Final gate

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
- zero complete summary-map clones in convergence
- no borrow reanalysis without a changed direct input after the initial seed pass

Use this checkpoint for the Gate D review. Resolve any required findings through the bounded
R5C10/R5C10C correction lane before the final auditor and Gate D acceptance.

### R5C10 / Gate D corrections

R5C10 is the bounded correction lane for Gate D review findings after the R5C9 deletion audit.
It must preserve the accepted source-index, immutable-interface, transactional-publication and
boundary-owned generated-sidecar architecture.

Its acceptance list is:

- exported generic receivers survive immutable interface projection and import
- private generic receivers needed by generated bodies use module-private nominal identity
- generic receiver bounds and evidence survive generated-local reconstruction
- declaration and evidence materialisation closure remains shared at module scope
- no complete declaration record is duplicated per template binding
- module, generated and package publication remain transactional
- generated declaration lookup uses construction-only indexed append
- normal `just validate` is green

The sequence is:

```text
R5C9 deletion audit
-> Gate D review findings
-> R5C10 corrections
-> final review/auditor
-> Gate D acceptance
```

### R5C10C / Gate D ownership cleanup

The first independent final audit identified two Gate D blockers and one bounded hardening item.
R5C10C keeps the accepted Phase 5 architecture while completing the ownership boundary:

- canonical Stage 0 no longer derives project-local packages from `Config.package_folders`; structural `+*.moth` support roots and independently registered Builder/Core source packages remain active
- project compilation supplies the resolver with the indexed normal-and-support root table, so support-facade membership, re-exports and public exports use the same Stage 0 facts as module scheduling
- requester-local generic declarations use a construction-only exact identity-to-path index with duplicate retained-body rejection; the path-keyed template map remains the body owner
- selected generated evidence targets use a transient validation set rather than fabricated source-visible bindings
- shared-directory compatibility keeps the retained project facade out of the normal/support resolver root table, so a facade cannot replace the normal entry root
- the transitional `package_folders` parser/storage and strict empty `entry_root` compatibility remain owned by the queued Project Config plan

R5C10C is implementation-complete and validated. The second independent review remains the next
pause boundary; final auditor and Gate D must still inspect the correction checkpoint before any
follow-up plan starts.

The bounded correction sequence is:

```text
first independent Gate D review
-> R5C10C ownership corrections
-> exact validation gate
-> second independent review
-> final auditor and Gate D
```

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
- source payloads move once or have one proven shared owner
- convergence performs no unrelated rechecks or complete summary-map clones
- generated and materialisation publication are transactional
- token remapping is exhaustive, in place and fallible
- one directory-project compiler path remains
- dependency consumers are independent of the current `import` keyword
- full validation and Gates C1 through D are clean

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

Focused tests are iteration evidence, not the acceptance gate. Run the manual architecture audit from `validation.mtf` whenever a slice changes discovery, dependency shells, provider binding, interface closure, identity, borrow summaries, graph scheduling or generated sidecars.

Documentation-only plan changes use:

```bash
moth build docs --release
```

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
