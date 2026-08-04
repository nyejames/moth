# Canonical module compilation and scoped packages implementation plan

## Purpose

Finish the canonical module cutover from the reviewed Phase 5 checkpoint, correct the remaining provider-index and interface-closure problems, then close the avoidable performance and ownership gaps before link, reuse and backend work continues.

The target remains:

- one canonical semantic compilation per physical module inside one project or package boundary
- one deterministic source inventory and one preparation pass per selected source
- immutable completed provider interfaces rather than donor headers, AST or HIR
- stable cross-module identities and generated concrete functions in build-owned sidecars
- explicit graph outcomes, artefacts, entry assemblies, package assemblies and link plans
- strict scoped support packages and module-root-relative imports
- small data-oriented owners using dense IDs, contiguous records and operation-scoped indexes
- no compatibility path, duplicated semantic owner or speculative duplicate compiler work

The architecture is accepted. The reviewed checkpoint needs bounded corrections, not another redesign.

## Current state

```text
ACTIVE_PLAN: docs/roadmap/plans/canonical-module-compilation-and-scoped-packages-plan.md
WORK_ID: R5-closeout
WORK_SOURCE: parent in-depth review of the implemented R5C1-R5C3 checkpoint
BASE_REVISION: 85199889194ba6b3a378d9d25a4bd72f74a57ed4 (implementation checkpoint reviewed; later main commits are unrelated docs/benchmark work)
STATUS: active - R5C3B checkpointed; R5C4 next
CURRENT_SLICE: R5C4 - compact immutable generic materialisation metadata
LAST_ACCEPTED_COMMIT: 6750c9a57238203ba83b2a31d74d6a19ecf36d70 (R5C1); R5C2/R5C3 remain implemented on main but their acceptance is reopened by this review
WORKTREE: committed main; coordinator-owned edits begin after this record
REQUIRED_RELOADS: AGENTS.md, this plan, compiler-design-overview.md, build-system-design.md, compilation.rs, compiled_boundary.rs, module_artifact_store.rs, source_discovery.rs, headers/file_imports.rs, public_interface/import_bindings.rs, public_interface/interface_view.rs, public_interface/interface_closure.rs and headers/import_environment/
RELEVANT_CONTEXT_NOW:
- R5C1 retained project/package graph boundaries, dense ModuleId-to-artefact mappings, generated lanes and diagnosed/blocked outcomes
- R5C2 replaced path/suffix provider joins with ImportShellId lookup, but retained invalid optional shell states and repeated package dependency/index work
- R5C3 added InterfaceView and indexed closure work, but header binding still performs repeated full-interface projection and closure still has evidence agreement, exact-interface identity and duplicate-index gaps
- no donor compilation, path fallback or consumer-local generic materialisation has returned
ACCEPTANCE_CRITERIA:
- retained import shells have non-optional identities and invalid provider-import state combinations are unrepresentable
- one exact build-local provider interface ID is used for caches and shell bindings
- equal module origins with differing interface contents fail deterministically
- completed source packages and per-consumer package dependencies are indexed once
- package ordering uses one deterministic dense dependency graph rather than repeated whole-set scans
VALIDATION_STATE: R5C2A+R5C3A+R5C3B pass full just validate (ci-clippy native/linux/windows, workspace tests 4019+17+581+595, integration 1817/1817, docs check, bench-ci 60 preflight cases) and the R5C3B correction re-run (closure tests 8/8, clippy clean, integration 1817/1817)
DOCS_IMPACT: active plan only unless implementation exposes architecture-doc drift
BLOCKERS_OR_OPEN_DECISIONS: none for R5C2A; stop if exact provider identity cannot be introduced without another durable semantic identity
AUDIT_STATE: full provider/closure audit reported one low finding (temporary vectors in enqueue_type/enqueue_folded_value instead of direct callback enqueue); corrected and validated (closure tests, clippy, integration). InterfaceView deleted; closure uses ClosureRecordView; provider tables keyed by ProviderInterfaceId.
DELEGATION_DECISION: coordinator-owned implementation for the R5C2A correction train; separate read-only auditors at the named review gates
NEXT_WORKER_ORDER: R5C2A -> audit -> R5C3A -> R5C3B -> audit -> R5C4 -> R5C5 -> R5C6 -> R5C7 -> R5C8 -> R5C9 -> R6 -> R7 -> R8
STOP_REASON: none active; user requested continuation through plan completion
NEXT_RESUME_ACTION: implement R5C4 (compact immutable generic materialisation metadata, frozen token buffer over canonical TokenKind vocabulary)
```

Keep this block current and concise. Git history is the durable implementation history. Do not append worker transcripts or complete validation logs.

## Required authorities

Read these before implementation and before every review phase:

- `AGENTS.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/language-overview.md`
- `docs/src/docs/codebase/memory-management/overview.mtf`
- `docs/src/docs/codebase/memory-management/borrow-validation/overview.mtf`
- `docs/src/docs/codebase/memory-management/lifetime-regions-and-escape-validation/overview.mtf`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `docs/src/docs/progress/@page.moth`
- downstream config, entry-config and HTML-Wasm plans

The compiler and build-system overviews own the architecture. This plan owns implementation order, migration boundaries and deletion gates.

## Accepted architecture and implemented baseline

Keep the following work. Do not rebuild it under new names:

- stable package, module, source, declaration, type, trait, evidence, private executable and generated identities
- one `SourceTreeIndex` traversal implementation for project and source-package boundaries
- boundary-local dense `SourceId` and `ModuleId` values with stable portable identities kept separate
- indexed module-root-relative namespaces and direct graph edges
- prepare-once source storage and header-owned structural provider discovery
- canonical module jobs in deterministic dependency waves
- explicit normal, support and project-facade root roles
- successful, diagnosed, blocked and infrastructure outcomes inside graph compilation
- retained `CompiledGraphBoundary`, `CompiledSourcePackage` and `ModuleArtifactStore` ownership
- immutable completed provider interfaces and recursive re-export closure
- inverse canonical type, value, receiver, trait and evidence projection
- stable local, cross-module, module-private, generated and binding call targets
- read-only borrow validation over local HIR plus completed call summaries
- complete generic materialisation contexts, stable request identities and generated sidecars
- build-owned per-function reachability and entry assembly
- separate executable, interface, link-fact and compiler-metadata lanes
- retained import-shell identities replacing path and suffix matching
- vector-backed final interfaces with transient lookup indexes

The implementation through `851998891` is the reviewed input. R5C1 remains accepted. R5C2 and R5C3 keep their direction and most of their code, but the correction slices below must replace their incomplete identity, projection and closure details before R5C4 starts.

## Locked implementation decisions

### Exact identities at each boundary

- Stable module and declaration origins remain semantic identity.
- Dense `ModuleId`, `SourceId`, package IDs, provider interface IDs and import-shell IDs remain build-local handles.
- A stable module origin is not an operation-local provider cache key. Distinct interface values claiming one origin must agree or fail.
- Retained `ImportShellId` contains a real `FileId` and ordinal. It is never partially stamped.
- Raw pre-header scanning and retained header syntax use distinct provider-reference types. Do not keep `Option<ImportShellId>` on the retained type.
- Paths remain IO and diagnostic data. They do not rediscover providers or semantic declarations after IDs exist.

### Valid provider-input states

Use an explicit provider import kind equivalent to:

```rust
pub enum ProviderImportKind<'a> {
    Authored { shell_id: ImportShellId },
    ImplicitTemplate { package_prefix: &'a str },
}
```

Exact names may change. Invalid combinations of optional shell, optional prefix and boolean flags must become unrepresentable.

One operation-local provider table assigns dense `ProviderInterfaceId` values after validating:

- exact repeated references collapse to one ID
- equal module origins with equal interface contents collapse to one ID
- equal module origins with different contents fail through `CompilerError`
- one authored shell maps to exactly one provider ID
- project and source-package identity spaces cannot cross-address

`ProviderInterfaceId` never enters persistent or public semantic identity.

### Stable final vectors and narrow transient indexes

Final artefacts remain deterministic vector-backed data. Query-heavy construction stages may use transient `FxHashMap` indexes.

Prefer narrow indexes owned by one operation:

- provider binding view: public name, binding export, declaration origin and concrete summary origin
- closure index: declaration origin, summary origin, evidence identity and evidence trigger origins
- package registry: package prefix and package dependency adjacency
- generated registry: generic declaration identity and completed summary identity

Do not keep one broad index type merely because several operations query the same final interface differently. Do not rebuild a narrow index inside a loop.

### Import semantic facts once

A completed provider interface is already recursively closed. Header binding imports its stable semantic closure once per unique provider used by the module, not once per import shell.

The stage-local import environment should store:

```text
origin -> one imported declaration record
canonical evidence identity -> one evidence record
function origin -> one concrete call summary
local path -> stable declaration/function origin
```

Aliases, namespace members and receiver paths reference these stable tables. They do not clone the declaration or summary payload again.

### One closure queue

Recursive interface closure uses one deterministic queue of explicit work items:

```rust
pub enum ClosureWorkItem {
    Declaration(OriginDeclarationId),
    Evidence(CanonicalEvidenceIdentity),
}
```

Separate queued and completed sets prevent duplicate queue entries. Processing either item may enqueue more declaration or evidence items. Do not retain nested fixed-point loops when one queue expresses the dependency relation directly.

### One validation owner

- Publication validates a completed interface once.
- Provider registration validates equal-origin agreement once.
- Header binding validates local alias and visibility rules, not provider publication again for every shell.
- Evidence equality includes ownership and every requirement mapping.
- Internal dense lookup failures propagate as `CompilerError`. Do not erase them with `.ok()`, convert them to absence or panic later.

### No duplicate work

Inside one project or package boundary:

- visit each directory once
- read each selected source once
- tokenize each `.moth` source once
- prepare each retained header once
- resolve each import shell once
- compile each physical module once
- close each provider interface once
- project each unique provider closure into a consumer module once
- materialise each generated identity once
- emit one diagnostic set per diagnosed module or generated request

Use counters for these contracts. Timings alone are not proof.

### Single-file mode remains explicit

Directory-project compilation must not use the old reachable-file scanner or path fallback.

Synthetic single-file compilation may retain a narrow transitive source traversal when it:

- uses the shared tokenizer and import-clause parser
- reads and tokenizes each source once
- never creates project graph identities it does not own
- remains isolated from directory-project scheduling

The final deletion audit must distinguish this legitimate synthetic mode from removed directory-project discovery.

### No compatibility scaffolding

Moth is pre-release. Replace APIs directly and delete old owners. Do not add forwarding wrappers, fallback branches, duplicate payloads, feature flags or test-only production adapters.

## Slice discipline and stop conditions

Every implementation slice must state its owner, inputs, outputs, deleted code, focused tests, non-goals and full validation gate.

Stop and request review when:

- more than two unlisted stage boundaries must change
- a second durable representation of one fact appears necessary
- an identity requires rendered names, source positions or donor-local IDs
- a wrapper exists only to bridge old and new APIs
- a full-table clone remains inside a loop over imports, declarations, modules, packages or sidecars
- the same invariant needs a second correction pass
- the slice exceeds roughly 12 production files or 600 net production lines, excluding mechanical moves
- a user-facing failure would need `CompilerError`
- exact counters show increased source, provider, module or generated work
- focused tests cannot isolate the intended owner

Preserve the work and record the exact unresolved question. Do not improvise a fallback.

## Preserved review phases

The six parent review phases remain mandatory.

### Review phase 1: direct interface boundary

Complete and accepted. Reopen only if direct declaration or local summary ownership changes.

### Review phase 2: provider consumer contract

Reopen narrowly through R5C2A-R5C3B. Verify exact provider identity, one-time provider projection, stable consumer-local aliases and no foreign AST or HIR access.

### Review phase 3: discovery and graph

Run after R5C2A, R5C7 and R5C9. Verify one source inventory, one preparation owner, indexed package/provider dependencies, separate package boundaries and deterministic graph jobs.

### Review phase 4: generated sidecars

Run after R5C4-R5C6. Verify compact immutable template artefacts, one generated identity lookup, deterministic request deduplication, bounded summary convergence and no consumer-local materialisation.

### Review phase 5: canonical production cutover

Run after all R5C slices. Verify one production compiler path, complete provider publication, durable graph outcomes, no donor or fallback path and no discarded semantic artefact lane.

### Review phase 6: link, backend and reuse

Run during R6 and R7. Verify complete link facts, assemblies, provenance, lifetime roots, backend handoff, fingerprints, `check`, dev reuse and the final deletion audit.

Reviews are read-only. Corrections become separate bounded slices.

## Phase R5C: Phase 5 correction and closeout

### R5C0: reviewed baseline

Status: complete.

- keep the semantic architecture through `851998891`
- keep R5C1 as accepted
- reopen R5C2 and R5C3 only for the corrections below
- do not begin R5C4 or R6 while the correction train remains open

### R5C1A: boundary invariant and test-support cleanup

Goal: remove the small retained-boundary debt without changing its architecture.

Changes:

- add one `CompiledGraphBoundary` invariant validator that checks graph node count, slot count, diagnosed/blocked lanes and final slot states agree
- expose one success-only consuming boundary instead of separately checking outcome vectors and module slots in several callers
- propagate `ModuleArtifactStore::artifact` errors during construction instead of using `.ok().flatten()`
- keep later infallible lookup only after the constructor has validated every retained reference
- move `ProjectCompilation::from_test_modules`, synthetic interface builders and graph test constructors into test support
- remove the test-only flat `Vec<Module>` production constructor
- replace one-shot `compilation_module_views` allocation with direct iteration only when it reduces code without adding custom iterator machinery

Do not redesign `CompiledModuleRef` or entry assembly in this slice.

Tests:

- inconsistent slot and outcome lanes fail at the boundary validator
- overlapping package-local `ModuleId` values remain isolated
- invalid dense references return `CompilerError`
- production code contains no test-only flat-module adapter

This cleanup may accompany R5C2A only when the combined slice remains within the churn limits.

### R5C2A: exact provider and package indexes

Goal: complete R5C2 by making provider states explicit and indexing package work once.

#### Provider reference phases

Split the current optional-shell provider reference into two typed phases:

```text
ScannedProviderReference
- path
- location
- grouped shape

RetainedProviderReference
- path
- location
- grouped shape
- ImportShellId
```

Raw single-file scanning produces the scanned form. Header preparation stamps the retained form. Directory graph edges and provider bindings accept only the retained form.

`ImportShellId` uses a non-optional `FileId` and ordinal. Synthetic tests obtain a real test `FileId` from test support.

#### Provider interface table

Build one dense provider table for each module binding operation:

```text
ProviderInterfaceId -> &PublicSemanticInterface
ImportShellId -> ProviderInterfaceId
implicit template provider -> ProviderInterfaceId
```

Construction is fallible and validates duplicate shell, duplicate provider and equal-origin interface agreement. Re-export caches and header binding use `ProviderInterfaceId`, not module origin or raw pointer identity.

Do not re-run complete provider publication validation per shell. When external binding resolution needs boundary-specific validation, perform it once per provider ID and registry context.

#### Package registry and ordering

Replace repeated package-prefix map construction with one incrementally maintained registry:

```rust
pub struct CompletedSourcePackageRegistry {
    packages: Vec<CompiledSourcePackage>,
    by_prefix: FxHashMap<String, PackageBoundaryId>,
}
```

Build package dependency data once:

- prefix -> `PackageBoundaryId`
- package -> provider package IDs
- package -> consumer package IDs
- consumer module -> package dependency IDs

Use a deterministic dense indegree/topological schedule. Reuse the existing dense Kahn scheduling pattern when a small shared helper removes duplicate code without merging package and module identity types.

Readiness checks walk only the current module's package dependencies. They never filter the full import vector.

Delete:

- `Option<ImportShellId>` on retained provider references
- optional shell/prefix/boolean provider input combinations
- silent duplicate shell overwrite in `SourceProviderImportSet`
- `FxHashMap<StableModuleOriginIdentity, InterfaceView>` cache identity
- repeated completed-package index construction
- repeated whole-vector package dependency filtering
- ad hoc package-order loops that rebuild dependency sets each pass

Tests and counters:

- duplicate or unstamped shells fail
- one shell cannot target both a source module and source package
- two distinct interfaces claiming one origin agree or fail in either input order
- exact repeated provider interfaces receive one provider ID
- package prefix indexing occurs once
- package readiness visits only direct dependencies
- deterministic package order survives reversed discovery order
- project and package local IDs never cross-address

Validation:

```bash
cargo fmt --all
just validate
```

Stop for Review phases 2 and 3 before R5C3A.

### R5C3A: one-time provider projection in header binding

Goal: complete the provider consumer cutover so each unique provider interface is indexed and projected once per consumer module.

#### Narrow binding view

Replace the broad shared `InterfaceView` use in header binding with a provider binding view containing only:

- borrowed public-name keys to export binding indexes
- borrowed public-name keys to binding-export indexes
- declaration origin to declaration index
- function origin to summary index
- export diagnostic provenance by public name

Avoid allocating duplicate owned `String` keys when the interface already owns stable strings.

#### Provider semantic tables

When the first shell references a provider ID, import its closed semantics once into the module-wide header environment:

```text
imported_declarations_by_origin
imported_evidence_by_identity
imported_call_summaries_by_origin
```

For every later grouped, namespace, receiver or implicit-template import from that provider:

- resolve the selected public binding through the cached view
- store local path or local name -> stable origin/target
- reuse the one semantic record and summary by origin
- do not clone the complete provider declarations or evidence again

Replace duplicated payload maps where practical:

```text
local declaration path -> OriginDeclarationId
local function path -> SourceFunctionTarget or OriginFunctionId
```

Do not store another full declaration or call summary for every alias and namespace member.

Evidence insertion is keyed by `CanonicalEvidenceIdentity`. Equal records deduplicate. Different records with one identity fail before AST projection.

All source-provider import forms must use this path:

- grouped imports
- namespace imports
- receiver methods
- traits and evidence
- implicit `.mtf` constant scope
- binding-backed re-exports reached through a source provider

Delete or demote linear `PublicSemanticInterface` query methods after the final call sites migrate. Occasional final-vector lookup may use binary search over the already sorted vectors. Query-heavy code must use the transient view.

Tests and counters:

- ten shells from one provider build one binding view and project one closure
- grouped and namespace imports from one provider do not duplicate evidence or summaries
- two aliases of one declaration retain one semantic record
- receiver methods reuse summary-by-origin storage
- implicit template scope reuses the provider cache
- missing or differing provider records fail deterministically
- provider binding performs no filesystem query and no full-interface scan per shell

Validation:

```bash
cargo fmt --all
just validate
```

Stop for a focused provider consumer review before R5C3B.

### R5C3B: simplify and harden recursive interface closure

Goal: replace overlapping index layers and first-provider-wins behaviour with one explicit closure index and queue.

#### Closure index

Build one `ClosureIndex` in one pass over each unique provider and the direct interface:

```text
declaration origin -> RecordRef list
function origin -> RecordRef list
evidence identity -> RecordRef list
declaration origin -> evidence identity list
```

`RecordRef` identifies the exact source interface and vector index. Closure record access uses that index directly. Do not re-query a per-interface map after resolving a `RecordRef`.

Provider deduplication uses `ProviderInterfaceId`. Equal-origin disagreement must already have failed in R5C2A, though closure retains defensive full-record agreement checks.

#### Agreement rules

Every publisher of one key must agree:

- declaration origin -> complete declaration record
- function origin -> complete call summary
- evidence identity -> ownership and complete ordered requirement mappings

No key uses first-provider wins.

#### Work queue

Use one `VecDeque<ClosureWorkItem>` with queued and selected sets. Enqueue declarations and evidence only once.

Dependency walking enqueues directly through callbacks. Avoid temporary vectors for nested type identities where direct visiting is clear.

Deduplicate declaration origins produced from one evidence target before indexing triggers.

#### Materialisation

Consume direct record vectors in one linear pass and move selected records. Clone each selected provider record once through its exact `RecordRef`.

Remove:

- provider scans during evidence materialisation
- `.position(...)` searches after an indexed lookup
- broad per-interface indexes unused by closure
- `swap_remove` position bookkeeping when consuming the direct vector is simpler
- cloned top-level direct export vectors when ownership can move through closure

Final vectors remain sorted by stable semantic identity and contain no durable maps.

Tests and counters:

- differing evidence mappings fail in both provider orders
- equal repeated evidence materialises once
- deep repeated canonical type references queue one declaration once
- each input declaration, summary and evidence record is indexed once
- each selected record is materialised once
- unrelated provider facts remain absent
- reversed provider order produces identical final vectors
- two distinct interfaces with one origin cannot share the wrong binding view

Validation:

```bash
cargo fmt --all
just validate
```

Run the full reopened Review phase 2 and a focused interface audit. Do not begin R5C4 until all required findings are resolved.

### R5C4: compact immutable generic materialisation metadata

Goal: retain one self-contained generic template representation without a second tokenizer vocabulary or repeated token strings.

Replace mirrored `StableTokenKind` and `StablePlainTokenKind` with one compact remappable frozen token buffer over the canonical `TokenKind` vocabulary.

Invariants:

- one context-local immutable frozen string pool
- donor token IDs remap into the pool once when freezing
- the pool merges into the generated-local table once when materialising
- repeated symbols, path components and literals share one frozen string entry
- adding a tokenizer token variant does not require editing a second exhaustive token enum
- no donor `StringId`, `InternedPath`, `FileId`, absolute path or mutable string table crosses the artefact boundary
- source identity remains portable and self-contained

Store module-wide declaration, nominal, trait, evidence and callable closure tables once. Template records reference those tables by dense indexes and remain keyed by stable generic declaration identity.

Do not retain AST, TIR, a mutable type environment or a persistent-serialization format.

Tests:

- every token payload round-trips
- repeated spellings occupy one frozen string entry
- two templates share module closure tables
- contexts remain `Send`
- no donor-local identity crosses

Stop for Review phase 4 before R5C5.

### R5C5: direct generated registries and delta-only sessions

Goal: make generated lookup and module sessions proportional to new requests.

Add boundary-owned indexes:

```text
GeneratedDeclarationIdentity -> MaterialisationContextId
GeneratedFunctionIdentity -> completed summary/sidecar
```

Requirements:

- publish contexts transactionally with successful module artefacts
- validate duplicate declaration identities at publication
- project and package contexts share one borrowed lookup view without building a new `Vec` per module
- generated sessions borrow immutable completed summaries and own only their new delta
- request records own their diagnostic location and display facts
- materialisation never searches a parallel request list by identity
- requester and dependency vectors remain deterministic and deduplicated
- bulk-build generated declaration tables rather than cloning the full table per callable or nested template

Tests:

- one declaration identity resolves to one context
- duplicate contexts fail before materialisation
- two modules requesting one identity produce one sidecar
- a session allocates only new records
- nested diagnostics use the exact request record

### R5C6: dependency-driven call-summary convergence

Goal: remove unconditional whole-boundary borrow rechecking while preserving exact summaries.

Instrument first:

- base-module borrow passes
- sidecar borrow passes
- summary changes
- dirty dependency components
- maximum fixed-point iterations

Build one deterministic dependency graph over local, module-private and generated call targets. Imported completed summaries are fixed leaves.

Requirements:

- derive edges from stable HIR call targets and link facts
- schedule only nodes whose callee summary changed
- process strongly connected components in deterministic identity order
- prove updates are monotone or diagnose internal oscillation
- avoid cloning complete private/generated summary maps into every sidecar
- do not recheck an independent sidecar after its component stabilises
- run final base borrow validation with exact summaries required by that module

Do not redesign source borrow rules.

When function-granular scheduling requires a broad borrow-checker rewrite, stop and create a dedicated convergence plan. A temporary module-level fallback is acceptable only when isolated behind one owner, measured with exact counters and unable to recheck stable unrelated sidecars.

### R5C7: remove complete prepared-source payload cloning

Goal: preserve one read and tokenization without copying complete source and token buffers at each handoff.

Requirements:

- `PreparedSourceStore` remains indexed by `SourceId`
- move a canonical source payload into its single module preparation owner
- share one source-level immutable allocation only when check-only or tooling reuse creates a real second consumer
- do not clone `FileTokens` before header preparation
- rebind source identity without copying the token vector
- source text follows the same move/share policy
- diagnosed preparation cannot be consumed twice

Do not add per-token or semantic-leaf reference counting.

Synthetic single-file traversal may retain its isolated source cache under the locked single-file rules.

### R5C8: split ownership and remove noisy orchestration

Goal: make the finished pipeline inspectable without changing its data flow.

Split by real ownership after the correction slices settle. Suggested boundaries:

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

Exact names may change.

Requirements:

- one obvious module compilation coordinator
- package indexing and package ordering in one owner
- provider-input construction in one owner
- generated materialisation and summary convergence outside base semantic orchestration
- replace long `compile_module_waves` arguments with one narrow immutable context
- split generic materialisation into context model, frozen syntax, environment installation and execution only when each has a distinct owner
- move provider-interface projection out of the general import-environment builder when it simplifies that builder
- group the three generated/source/private name maps into one `LinkSymbolNames` owner when they have the same lifecycle
- return borrowed entry views or iterators instead of rebuilding owner-independent vectors when that reduces allocations without custom iterator machinery
- remove stale comments, forwarding modules, compatibility derefs and test-only production helpers
- keep tests with their production owner and move rather than duplicate them
- do not introduce stage traits or dynamic dispatch

Do not split files mechanically before ownership is clear.

### R5C9: deletion audit and Phase 5 validation

Run the production deletion audit:

- no donor header or body copying
- no directory-project source fallback through `ProjectPathResolver`
- no suffix or path matching for provider interface joins
- no directory-project reachable-file or import scanner
- synthetic single-file traversal remains isolated and uses one read/tokenization
- no `DiscoveredModule` or `ModuleEntryCompileWaves`
- no consumer-local generic materialisation
- no body-only template store
- no mirrored token-kind vocabulary
- no module-only artefact flattening
- no package metadata mutation for entry suppression
- no full declaration-table clone inside a callable, template or import loop
- no per-module clone of every completed generated summary
- no provider/materialisation context scan by declaration identity
- no linear query API in provider or closure hot paths
- no optional retained shell identity
- no first-provider-wins semantic record
- no compatibility API for removed paths

Required validation:

```bash
cargo fmt --all
cargo run --quiet -- tests --audit
just validate
just bench-ci
```

Use counters to prove:

- one source read, tokenization and header preparation
- one semantic module compilation
- one interface publication
- one provider view and provider semantic projection per consumer/provider pair
- one generated materialisation per stable identity
- no unrelated sidecar borrow recheck
- package dependency visits are proportional to direct dependencies

Run Review phases 3, 4 and 5. Resolve every required finding before Phase 5 is accepted.

### Phase 5 exit gate

Phase 5 is complete only when:

- every selected project and source-package module compiles once through canonical graph jobs
- consumers bind only completed immutable provider interfaces
- recursive source and binding re-exports are closed and validated
- generated concrete functions live only in sidecars
- successful artefacts and graph identity survive into `ProjectCompilation`
- provider identity, binding and closure perform no path rediscovery or repeated full-interface projection
- package scheduling and readiness use one indexed dependency model
- prepared source, generated lookup and summary convergence avoid repeated payload clones and unrelated work
- one production directory-project compiler path remains
- full validation, docs and benchmark gates pass
- Review phases 3, 4 and 5 report no unresolved required finding

After acceptance, replace the detailed R5C section with one short accepted-baseline paragraph.

## Optional Phase R5O: measured module-wave parallelism

This phase is optional and requires benchmark evidence after R5C.

Do not regain parallelism by materialising one generated identity in several workers.

When serial ready-wave execution is a measured regression, use a two-stage schedule:

```text
parallel base semantic/request discovery
-> deterministic global request reservation
-> generated fixed point
-> parallel requester finalisation where dependencies permit
-> ModuleId-ordered publication
```

Keep one materialisation per identity and deterministic diagnostics.

## Phase R6: complete link facts and assemblies

### R6A: complete per-function link facts

Retain local, cross-module, module-private, generated and binding calls plus capabilities, reactive features, target-gated operations, paths, assets, generated requests and project-context provenance in deterministic function identity order.

Delete complete `ExternalPackageRegistry` values from module artefacts once stable binding identities cover backend planning.

### R6B: complete entry assemblies

Extend the existing `EntryAssembly`. It selects one normal root's dormant work, fragments, settings, exact reachable functions and runtime/asset union from completed artefacts only.

Imported normal modules never activate root work. Support and facade roots never become entries.

### R6C: package assembly and provenance

Build `ProjectPackageAssembly` over the compiled facade, selected descendant interfaces, generated sidecars and permitted runtime requirements.

Never bypass `export:` or mutate base artefacts. Propagate project-context provenance through public facts and reachable calls.

### R6D: lifetime and target roots

Supply explicit entry, package, generated and builder lifecycle roots to the memory-analysis owner. The build system does not implement another lifetime solver.

## Phase R7: backend handoff, commands and reuse

- backends consume `ProjectCompilation` and explicit link plans
- the HTML builder consumes `EntryAssembly` without source discovery
- `check` retains successful independent artefacts and check-only units beside diagnostics
- dev reuses prepared source slots, artefacts, generated sidecars, graphs and namespaces
- semantic consumers rebuild only when public-interface fingerprints change
- implement the five final fingerprint encoders only over final facts
- builders return output records while the build system owns output validation, manifests and cleanup

## Phase R8: documentation and final repository audit

Update language, project-structure, compiler education, scaffolding, downstream plans and the progress matrix only for implemented behaviour.

Verify:

- source, tests, docs and roadmap agree
- no stale module, import or package shape remains
- generated docs build in release mode
- benchmark cases prove successful compilation rather than early exit

## Required end-to-end contracts

The integration suite owns user-visible behaviour. Focused Rust tests own hidden identity, scheduling and impossible-state invariants.

Required contracts:

- shared modules compile once and diagnose once
- independent graph branches continue while blocked consumers do not cascade
- module-root-relative imports and strict facade/support visibility
- separate project, Core, Builder and dependency package boundaries
- stable identities independent of local allocation and completion order
- re-export aliases change bindings without changing declaration origin
- provider to facade to consumer closure without reopening transitive providers
- imported receivers, traits, evidence, defaults and folded values use interfaces
- cross-module borrow, transfer and return-alias summaries
- generated sidecar deduplication and nested fixed point
- exact entry runtime and asset unions
- API-only roots have no `start`
- check-only units never enter canonical artefacts or backend roots
- deterministic diagnostics and output
- direct and transitive project-context package rejection
- one source read, tokenization, preparation and module compilation
- one provider projection per consumer/provider pair
- one generated materialisation per stable identity

## Validation policy

Every parent-accepted code slice requires:

```bash
cargo fmt --all
just validate
```

Focused tests are iteration evidence, not the acceptance gate.

Run the manual architecture audit from `validation.mtf` whenever a slice changes source discovery, provider binding, interface closure, AST/HIR ownership, identities, borrow/lifetime summaries, graph scheduling, generated sidecars or backend handoff.

Documentation-only slices use:

```bash
moth build docs --release
```

## Deliberately deferred work

- persistent module, package and generated artefact serialisation
- on-disk cache layout, eviction and migration
- dependency declaration syntax and local path dependencies
- registries, remote fetching, versions and lockfiles
- precompiled dependency caches
- direct normal-sibling imports
- cross-entry browser chunking
- physical Wasm module partition and Component Model integration
- cross-build generated-instance caches
