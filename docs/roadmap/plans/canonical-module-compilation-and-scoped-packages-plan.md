# Canonical module compilation and scoped packages implementation plan

## Purpose

Finish the canonical module cutover from the Phase 5 review checkpoint, remove the remaining avoidable work and leave a compact foundation for linking, reuse and backend work.

The target remains:

- one canonical semantic compilation per physical module inside one project or package boundary
- one deterministic source inventory and one preparation pass per selected source
- immutable completed provider interfaces rather than donor headers, AST or HIR
- stable cross-module identities and generated concrete functions in build-owned sidecars
- explicit graph outcomes, artefacts, entry assemblies, package assemblies and link plans
- strict scoped support packages and module-root-relative imports
- small data-oriented owners using dense IDs, contiguous records and transient indexes
- no compatibility path, duplicated semantic owner or speculative duplicate compiler work

The architecture is accepted. Phase 5 now needs bounded closeout work, not another redesign.

## Current state

```text
ACTIVE_PLAN: docs/roadmap/plans/canonical-module-compilation-and-scoped-packages-plan.md
WORK_ID: R5-closeout
WORK_SOURCE: parent Phase 5 architecture, quality and performance review
BASE_REVISION: 58432008fb5a1f7bb117dd226ccac773c25ed8c2 (current main HEAD; reconciled from the original 9af62e5a review base because the plan branch no longer exists and main advanced with the accepted language migration)
STATUS: active — R5C1 implemented and validated; awaiting interim audit and checkpoint before R5C2
CURRENT_SLICE: R5C1 — retain canonical artefacts and graph outcomes through the frontend handoff
LAST_ACCEPTED_COMMIT: 58432008f is the current main HEAD; R5C1 changes are uncommitted pending audit
WORKTREE: implemented directly on main per user direction; plan branch plan/canonical-module-phase5-closeout does not exist
REQUIRED_RELOADS: startup files, this plan, compiler-design-overview.md, build-system-design.md, compilation.rs, frontend_orchestration.rs, generated_worklist.rs, provider_store.rs, build.rs, public_interface/ and generic materialisation owners
RELEVANT_CONTEXT_NOW:
- R2C and provider-consumer Review phases 1 and 2 are complete
- Stage 0 owns indexed project and source-package boundaries, canonical module jobs, frozen graph waves and prepare-once source storage
- completed provider interfaces close recursive declaration/type/trait/evidence/call-summary facts and validate before publication
- cross-module source calls use stable function origins and borrow validation consumes completed provider summaries without foreign HIR
- generic requests use stable identities, one transactional boundary worklist and independently lowered generated sidecars
- normal, support and facade roots compile through graph-owned roles; API-only roots have no start
- old reachable-file/import scanners, donor closures and legacy module-entry wave types are deleted
- project and source-package graphs pass the current integration suite, but successful artefacts are flattened too early and several hot paths still rebuild indexes, clone large data or linearly search stable facts
- R5C1 target: compilation.rs flattens successful CompiledModuleArtifact into Vec<Module> and clears source-package root_activity to suppress entries
ACCEPTANCE_CRITERIA:
- preserve every accepted Phase 5 semantic contract
- retain completed artefacts, interfaces and graph outcome identity through ProjectCompilation
- remove repeated path matching, provider scans, materialisation-context scans and quadratic declaration rebuilding
- bound generated-summary convergence with dependency-driven work rather than unconditional full rescans
- move or share prepared source payloads without cloning complete token streams
- split mixed-responsibility orchestration modules without adding wrappers or parallel APIs
- complete full validation, docs and benchmark gates with zero known migration-fixture failures
VALIDATION_STATE:
- baseline main HEAD passes the recorded validation gate before R5C edits; full suite not yet rerun for R5C
DOCS_IMPACT: compiler and build-system architecture remain unchanged
BLOCKERS_OR_OPEN_DECISIONS: none for R5C1
AUDIT_STATE: no R5C audit yet
DELEGATION_DECISION: bounded implementation slices with a read-only auditor after each slice that changes a stage boundary; final_auditor before Phase 5 acceptance
NEXT_WORKER_ORDER: R5C1 only
STOP_REASON: none
NEXT_RESUME_ACTION: implement R5C1, run focused validation and stop for review before R5C2
```

Keep this block concise. Update it after parent acceptance of each slice. Git history remains the durable implementation record, so do not append worker transcripts or complete validation logs.

## Required authorities

Read these before implementation and before every review phase:

- `AGENTS.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/src/docs/codebase/language/overview.mtf` and its relevant canonical references
- `docs/src/docs/codebase/memory-management/overview.mtf`
- `docs/src/docs/codebase/memory-management/borrow-validation/overview.mtf`
- `docs/src/docs/codebase/memory-management/lifetime-regions-and-escape-validation/overview.mtf`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `docs/src/docs/progress/@page.moth`
- downstream config, entry-config and HTML-Wasm plans

The compiler and build-system overviews own the architecture. This plan owns implementation order, migration boundaries and deletion gates.

## Accepted foundation

Keep the following work. Do not rebuild it under new names:

- stable package, module, source, declaration, type, trait, evidence, private executable and generated identities
- one `SourceTreeIndex` traversal implementation for project and source-package boundaries
- boundary-local dense `SourceId` and `ModuleId` values with stable portable identities kept separate
- indexed module-root-relative namespaces and direct graph edges
- prepare-once source storage and header-owned structural provider discovery
- canonical module jobs in deterministic dependency waves
- explicit normal, support and project-facade root roles
- distinct successful, diagnosed, blocked and infrastructure outcomes inside graph compilation
- immutable completed provider interfaces and recursive re-export closure
- inverse canonical type, value, receiver, trait and evidence projection
- stable local, cross-module, module-private, generated and binding call targets
- read-only borrow validation over local HIR plus completed call summaries
- complete generic materialisation contexts, stable request identities and generated sidecars
- build-owned per-function reachability and entry assembly
- separate executable, interface, link-fact and compiler-metadata lanes

## Review verdict

Phase 5 has reached the intended architecture. No fundamental module-system or provider-interface redesign is required.

The remaining problems are implementation-boundary and hot-path problems:

1. successful `CompiledModuleArtifact` values are flattened back to `Module`, dropping interfaces and graph identity before later assembly, reuse and fingerprint owners consume them
2. provider imports are matched by repeated path-component and suffix comparisons across linear edge and package scans
3. interface closure repeatedly scans providers and linear interface vectors, then clones and revisits all evidence candidates
4. generated declaration contexts and completed summaries are repeatedly collected or cloned for each module session
5. materialisation retains a second mirrored token-kind vocabulary and repeated owned token strings instead of one compact remappable token buffer
6. generated environment installation repeatedly clones the complete declaration table while adding callables and templates
7. generated/private call-summary convergence reruns whole base modules and all sidecars until stable rather than scheduling only affected dependency components
8. prepared source projection clones source text and complete token streams despite one canonical semantic owner
9. `frontend_orchestration.rs` mixes preparation policy, semantic compilation, materialisation, sidecar scheduling and summary convergence in one oversized file
10. the former docs and benchmark fixture blockers are resolved; remaining closeout work is the
    bounded deletion audit and architecture-focused validation

These are closeout findings. They do not justify restoring donor compilation, path fallback or eager generic materialisation.

## Non-negotiable implementation rules

### One owner and one pass

Within one project or package compilation boundary:

- visit each directory once through the source index
- read each selected source once
- tokenize each `.moth` source once
- prepare each source's header syntax once
- resolve each retained import shell once
- compile each physical module once
- close and validate each provider interface once
- materialise each generated identity once
- emit one diagnostic set per diagnosed module or generated request
- encode each final fingerprint once by its final owner

Instrumentation must prove these counts. Timings alone are not evidence.

### Stable final data, transient fast indexes

Final artefacts should prefer deterministic contiguous vectors and dense IDs. Construction and query-heavy phases may build transient `FxHashMap` or sorted lookup indexes once.

Do not keep a final hash map solely because repeated linear searches were convenient. Do not repeatedly rebuild the same transient index.

### No path-based semantic joins

A path may be an IO handle or diagnostic spelling. It must not be used to rediscover a provider, declaration, callable, generated template or module when Stage 0 or the compiler already assigned an identity.

Build-local import-shell IDs are allowed inside one compilation boundary. They must not become stable semantic identities or persistent artefact keys.

### No duplicate generated work

Do not regain module-wave parallelism by materialising the same generated identity speculatively in multiple workers. Parallelism is acceptable only when request reservation or a two-stage schedule guarantees one materialisation per stable identity.

### No compatibility scaffolding

Moth is pre-release. Replace APIs directly and delete old owners. Do not add forwarding wrappers, fallback branches, duplicated payloads or feature flags.

### Keep modules readable

- one responsibility per module
- files should normally remain below roughly 2,000 lines
- functions should normally remain below roughly 200 lines
- use stage-local context structs instead of long parameter lists
- do not add test-only production accessors or future-consumer dead code
- split after ownership is clear, not by moving arbitrary line ranges

## Slice discipline and stop conditions

Each implementation slice must name:

- one owner
- exact input and output changes
- the live consumer
- deleted code
- focused tests
- full validation required for acceptance
- explicit non-goals

Stop and request review when any of these occurs:

- more than two unlisted stage boundaries need to change
- a second long-lived representation of one fact appears necessary
- an identity would require rendered names, source positions or donor-local IDs
- a new wrapper exists only to bridge old and new APIs
- an optimisation would perform duplicate semantic or generated work
- a full-table clone remains on a loop over declarations, imports, modules or sidecars
- the slice exceeds roughly 12 production files or 600 net production lines, excluding mechanical moves
- an existing invariant needs a second correction pass
- a user-facing failure would need `CompilerError`
- focused tests cannot isolate the intended owner

Preserve the work and record the exact unresolved question. Do not improvise a fallback.

## Preserved review phases

The six parent review phases remain mandatory.

### Review phase 1: direct interface boundary

Complete and accepted. Reopen only if a closeout slice changes direct declaration or summary ownership.

### Review phase 2: provider consumer contract

Complete and accepted. Reopen only if provider binding stops using stable identities or completed interfaces.

### Review phase 3: discovery and graph

Run again after R5C2, R5C7 and R5C9. Verify one source inventory, one preparation owner, indexed import resolution, separate package boundaries and deterministic graph jobs.

### Review phase 4: generated sidecars

Run again after R5C4 through R5C6. Verify compact immutable template artefacts, one generated identity lookup, deterministic request deduplication, bounded summary convergence and no consumer-local materialisation.

### Review phase 5: canonical production cutover

Run after all R5C slices. Verify one production compiler path, complete provider publication, durable graph outcomes, no donor or fallback path and no discarded semantic artefact lane.

### Review phase 6: link, backend and reuse

Run during R6 and R7. It covers complete link facts, assemblies, provenance, lifetime roots, backend handoff, fingerprints, `check`, dev reuse and the final deletion audit.

Reviews are read-only. Corrections become separate bounded slices.

## Phase R5C: Phase 5 closeout

### R5C0: freeze the reviewed boundary

Status: complete through this parent review and plan replacement.

- use `9af62e5a9475d529c6d1012d73b977d5cc0fe42c` as the Phase 5 review base
- keep the current semantic architecture
- treat the recorded passing integration and workspace gates as evidence, not as a substitute for the final rerun
- do not start R6 work while R5C remains open

### R5C1: retain canonical artefacts and graph outcomes

Goal: stop dropping semantic interfaces and boundary identity after graph compilation.

Change the frontend handoff so it retains:

- project graph identity
- project `CompiledModuleArtifact` values
- source-package graph identities and artefacts
- generated sidecars
- diagnosed and blocked outcomes where tooling needs them
- explicit project entry candidates without mutating package module metadata

Requirements:

- `ProjectFrontendCompilation` must not flatten successful artefacts into `Vec<Module>`
- source-package modules must remain immutable; do not clear `root_activity` to prevent entry selection
- entry selection must use boundary and root-role facts, not metadata mutation
- `ProjectCompilation` must own or borrow completed artefacts and expose module views to builders
- retain a dense boundary-local module-to-artefact index
- `build` and `dev` construct a success-only `ProjectCompilation`
- `check` may retain successful independent artefacts beside diagnosed and blocked records
- no fingerprint implementation is added in this slice

Delete:

- module-only flattening from provider stores
- package-root metadata mutation used to suppress entries
- compatibility `Deref` or module-vector APIs that hide the new owner

Tests:

- provider interfaces remain available after frontend compilation
- package modules cannot become project entries
- package metadata is unchanged by project assembly
- diagnosed and blocked branches retain independent successful artefacts
- artefact ordering is deterministic by boundary and `ModuleId`

Stop if builders require direct ownership of raw `Vec<Module>`. Replace that API rather than wrapping it.

### R5C2: index provider imports once

Goal: replace repeated path matching and nested scans with one build-local import-shell join.

Introduce a build-local import-shell identity equivalent to:

```rust
pub struct ImportShellId {
    pub source: FileId,
    pub ordinal: u32,
}
```

Exact names may change. The identity exists only inside prepared syntax and Stage 0 scheduling.

Requirements:

- header preparation assigns one ID to each retained import shell
- each structural provider reference carries or resolves back to that ID
- project and package graph edges retain the exact importing shell ID
- provider bindings and source-package bindings are indexed by consumer `ModuleId` and import-shell ID
- completed source packages are indexed once by import prefix
- `SourceProviderImportSet` performs direct lookup rather than path-component comparison
- remove owned `Vec<String>` importer/imported path copies from provider binding records
- remove suffix matching between authored and normalized paths
- same-module imports remain local compiler bindings and never enter the provider-interface map

Construction maps may be dropped after the module's bound inputs are built.

Tests:

- grouped and bare imports resolve the exact authored shell
- two imports with the same suffix in different files cannot cross-bind
- aliases and re-exports retain exact shell identity
- package and project boundary IDs never cross-address each other
- import binding performs no filesystem query

Delete:

- `provider_binding_matches_import`
- per-import scans over all provider edges
- per-import scans over all source-package imports
- repeated package-prefix lookup loops

### R5C3: index interface closure and binding views

Goal: preserve compact vector-backed interfaces while avoiding repeated linear closure searches.

Add a transient indexed view for one `PublicSemanticInterface`:

- public name to source or binding export
- declaration origin to declaration index
- function origin to concrete summary index
- evidence identity to evidence index
- source type and trait origin indexes where closure requires them

Requirements:

- build each view at most once per closure or binding operation
- validate duplicate keys while constructing the view
- interface closure uses a declaration/evidence work queue over indexes
- do not scan every provider for each selected declaration or summary
- do not clone all evidence candidates on each fixed-point iteration
- borrow direct records during closure and move final records once
- sort final vectors by their semantic order before publication
- final `PublicSemanticInterface` stays deterministic and contains no durable lookup map

Tests:

- deep re-export closure visits each declaration, summary and evidence record once
- providers that expose the same origin with different contents fail deterministically
- hidden nominal and trait closure still succeeds
- unrelated provider declarations and evidence are not copied
- output ordering is independent of provider import order

### R5C4: compact immutable generic materialisation metadata

Goal: keep one self-contained template representation without duplicating the tokenizer vocabulary or repeated strings.

Replace the mirrored `StableTokenKind` and `StablePlainTokenKind` representation with one compact remappable token buffer.

Preferred shape:

```rust
pub struct FrozenTokenBuffer {
    pub strings: FrozenStringPool,
    pub source_path: FrozenPath,
    pub tokens: Box<[Token]>,
}
```

The exact types may differ. The invariants are:

- reuse the canonical `TokenKind` vocabulary
- own one context-local immutable string pool
- remap donor token IDs into that pool once when freezing
- merge/remap the pool into the generated-local table once when materialising
- do not allocate an owned `String` for every repeated symbol, path component or literal token
- adding a tokenizer token variant must not require updating a second exhaustive token enum
- source identities remain portable and self-contained

Also audit the materialisation context for duplicated per-template closure data.

- module-wide declaration, nominal, trait, evidence and callable tables are stored once
- template artefacts reference dense indexes into shared immutable tables
- one template record remains keyed by stable generated declaration identity
- no AST, TIR store, mutable type environment or donor string table is retained

Tests:

- every token payload round-trips through freeze and materialisation
- repeated spellings occupy one frozen string entry
- two templates from one module share closure tables
- no donor `StringId`, `InternedPath`, `FileId` or absolute path crosses the artefact boundary
- frozen contexts remain `Send`

Stop if this requires a general persistent serialisation format. This is an in-memory immutable snapshot only.

### R5C5: index generated contexts and remove session-wide clones

Goal: make generated lookup and module sessions proportional to new requests, not all completed work.

Add one boundary-owned index:

```text
GeneratedDeclarationIdentity -> MaterialisationContextId
```

Requirements:

- publish context entries transactionally with successful module artefacts
- validate duplicate declaration identities at publication
- generated context lookup is direct and does not scan all provider artefacts or templates
- project and completed package contexts share one explicit lookup view without constructing a new `Vec` per module
- `GeneratedFunctionWorklist` borrows an immutable completed-summary view and owns only its local delta
- do not clone the complete boundary summary map for each module session
- request records retain their own diagnostic call location and declaration display data
- materialisation does not search a parallel request vector by identity
- requester and dependency vectors remain deterministic and deduplicated

Tests:

- one declaration identity resolves to exactly one context
- duplicate contexts fail before materialisation
- two modules requesting one generated identity produce one sidecar
- a module session allocates only records for new requests
- nested request diagnostics use the exact request record without a linear search

### R5C6: make call-summary convergence dependency-driven

Goal: remove unconditional whole-boundary borrow rechecking while preserving exact cross-call summaries.

First add instrumentation for:

- base-module borrow passes
- generated sidecar borrow passes
- summary changes
- dirty dependency components
- maximum fixed-point iterations

Then build one deterministic call-summary dependency graph over:

- local base functions
- module-private call targets
- generated sidecars
- imported completed summaries as fixed leaves

Requirements:

- derive dependencies from stable HIR call targets and retained link facts
- schedule only functions or sidecar modules whose callee summary changed
- process strongly connected components in deterministic identity order
- prove the summary update operation is monotone or reject an oscillating internal state explicitly
- remove the arbitrary whole-module-plus-request convergence multiplier
- avoid cloning complete private/generated summary maps into every sidecar
- run final base borrow validation only with exact summaries required by that module
- independent sidecars must not be rechecked when another component changes

Do not redesign the source borrow rules in this slice.

Stop and create a dedicated generated-summary convergence plan when function-granular scheduling requires a broad borrow-checker rewrite. In that case Phase 5 may retain a measured module-level fallback only when:

- the fallback is isolated behind one owner
- exact counters prove bounded work
- no sidecar is unconditionally rechecked after its dependency component is stable
- the follow-up plan is queued before R6 performance work

Tests:

- acyclic generated calls settle in dependency order
- recursive generated calls settle by SCC or produce the existing recursion diagnostic
- an unrelated sidecar is analysed once
- a changed private helper dirties only dependent generated/base summaries
- output and diagnostics remain deterministic

### R5C7: remove complete prepared-source payload cloning

Goal: preserve one read and one tokenization without copying complete source and token buffers at each handoff.

Requirements:

- `PreparedSourceStore` remains indexed by `SourceId`
- a canonical source payload is moved exactly once into its module preparation owner, or shared through one source-level immutable allocation only when a real second consumer exists
- `PreparedSourceInput::Moth` must not clone `FileTokens` before header preparation
- frontend preparation consumes or borrows the retained token buffer and rebinds identity without copying its token vector
- source text follows the same move/share policy
- tests use source IDs and instrumentation rather than retaining cloned production payloads
- future check-only units reuse the same prepared payload without retokenizing

Prefer moving payloads because each source has one canonical module owner. Use `Arc` only at the source payload boundary when check-only or tooling reuse makes shared ownership real. Do not add per-token or per-semantic-leaf reference counting.

Tests:

- one read, tokenization and header preparation per selected source
- shared source payload pointer or move count is exact
- check-only reuse does not retokenize
- diagnosed preparation cannot be consumed twice

### R5C8: split orchestration by responsibility

Goal: make the implemented pipeline inspectable without changing its data flow.

Split `frontend_orchestration.rs` into focused owners such as:

```text
create_project_modules/frontend/
├── mod.rs
├── preparation.rs
├── semantic_compile.rs
├── generated_materialisation.rs
├── generated_summary_fixpoint.rs
└── outcomes.rs
```

Exact names may change.

Requirements:

- keep one obvious module compilation coordinator
- move file-preparation strategy and chunking together
- move generated materialisation and generated-summary convergence out of base semantic orchestration
- replace `compile_module_waves`' long argument list with one narrow context
- split generic materialisation into context model, frozen syntax, environment installation and execution when the current file still mixes them
- bulk-build generated declaration tables once rather than cloning the full table per callable or nested template
- pre-index selected declaration paths instead of repeated `iter().any` searches
- remove stale comments that describe eager generics, discarded interfaces or legacy paths
- remove compatibility `Deref`, forwarding helpers and test-only production fields when the new owner makes them unnecessary
- do not introduce traits or dynamic dispatch for stage coordination

Deletion and moves must be completed in one slice. Do not leave forwarding modules.

Tests should stay with their production owner. Move tests rather than duplicating them.

### R5C9: final fixture migration, deletion audit and validation

Goal: close Phase 5 with one green production path and no known validation tail.

Confirm the fixture migrations required by current semantics remain in place:

- the docs `styles/docs/navbar` import uses the module-root-relative namespace
- removed benchmark `@./` source imports use their canonical module-root-relative or provider-owned form
- expected diagnostics reflect intentional fallback removal

Run the deletion audit:

- no donor header or body copying
- no cross-module source fallback through `ProjectPathResolver`
- no suffix/path matching for provider interface joins
- no old reachable-file or import scanner
- no `DiscoveredModule` or `ModuleEntryCompileWaves`
- no consumer-local generic materialisation
- no body-only template store
- no mirrored token-kind vocabulary
- no module-only artefact flattening
- no package metadata mutation for entry suppression
- no full declaration-table clone inside a callable/template loop
- no per-module clone of every completed generated summary
- no provider/materialisation context scan by declaration identity
- no compatibility API for removed paths

Required validation:

```bash
cargo fmt --all
cargo run --quiet -- tests --audit
just validate
just bench-ci
```

Also run focused counters proving:

- one source read
- one tokenization
- one header preparation
- one semantic module compilation
- one interface publication
- one generated materialisation per stable identity
- no unrelated sidecar borrow recheck

Run Review phases 3, 4 and 5. Resolve every required finding before Phase 5 is accepted.

### Phase 5 exit gate

Phase 5 is complete only when:

- every selected project and source-package module compiles once through canonical graph jobs
- consumers bind only completed immutable provider interfaces
- recursive source and binding re-exports are closed and validated
- generated concrete functions live only in sidecars
- cross-module generics preserve current behaviour without donor compilation
- successful artefacts and graph identity survive into `ProjectCompilation`
- prepared source, provider binding, interface closure and generated lookup avoid repeated scans or complete payload clones
- generated summary convergence is dependency-driven or has an explicitly bounded reviewed fallback
- one production compiler path remains
- all full validation, docs and benchmark gates pass
- Review phases 3, 4 and 5 report no unresolved required finding

After acceptance, replace the detailed R5C section with one short accepted-baseline paragraph. Do not let completed implementation history accumulate again.

## Optional Phase R5O: measured module-wave parallelism

This phase is optional and must be justified by benchmark evidence after R5C.

The current serial ready-wave execution avoids duplicate generated materialisation. Do not parallelise it by allowing workers to perform the same stable request independently.

Measure:

- ready-wave width
- module semantic compile time
- generated request count and overlap
- string-table merge time
- worklist commit time

When module-wave serialisation is a material regression, design a two-stage schedule:

```text
parallel base semantic/request discovery
-> deterministic global request reservation and deduplication
-> generated fixed point
-> parallel requester finalisation where dependencies permit
-> ModuleId-ordered publication
```

This phase must retain one materialisation per generated identity and deterministic diagnostics. It is not required merely because parallelism is theoretically available.

## Phase R6: complete link facts and assemblies

Keep R6 bounded to final link ownership.

### R6A: complete per-function link facts

Add the remaining stable generated and module-private facts to the existing per-function owner:

- local, cross-module, module-private, generated and binding calls
- helper and capability families
- reactive features
- numeric, cast, map and target-gated operations
- runtime paths and assets
- generated request references
- project-context provenance

Store facts in deterministic function identity order. Delete complete `ExternalPackageRegistry` values from module artefacts once stable binding identities cover backend planning.

### R6B: complete entry assemblies

Extend the existing `EntryAssembly` rather than adding a second type.

It selects from completed artefacts only:

- one normal module's dormant `start`
- compile-time and runtime fragments
- entry-local settings
- exact reachable base and generated functions
- binding/runtime/asset unions

Imported normal modules never activate root work. Support and facade roots never become entries.

### R6C: project package assembly and provenance

Build `ProjectPackageAssembly` over the compiled facade, selected descendant interfaces, generated sidecars and permitted runtime requirements.

- never bypass `export:`
- propagate project-context provenance through public facts and reachable source/generated calls
- reject prohibited direct or transitive project context
- do not mutate base artefacts

### R6D: lifetime and target roots

Supply explicit entry, package, generated and builder lifecycle roots to the memory-analysis owner. The build system must not implement a second lifetime solver.

### R6 exit gate

- per-function facts are the linking authority
- assembly performs no semantic compilation
- package assembly preserves facade visibility
- complete lifetime topology is supplied by the correct owner
- full validation passes

## Phase R7: backend handoff, commands and reuse

### R7A: final backend handoff

Backends consume `ProjectCompilation` and explicit link plans. They do not scan source, rebuild imports, infer generics or choose roots.

### R7B: HTML builder migration

Preserve route, fragment, JavaScript glue, asset and mixed-target behaviour through `EntryAssembly`. Keep physical Wasm partition work in its downstream plan.

### R7C: `check` graph outcomes

`check` compiles all selected modules plus check-only orphan units, retains successful independent artefacts and reports diagnosed/blocked work without pretending the project is linkable.

### R7D: dev reuse

Reuse prepared source slots, successful artefacts, package artefacts, generated sidecars, graphs and namespaces. Rebuild semantic consumers only when a provider public-interface fingerprint changes.

### R7E: final fingerprints

Implement canonical encoders only for the final five fact sets:

- public interface
- implementation
- dormant root activity
- runtime dependency
- documentation

Encode final facts, never pending or construction-only state.

### R7F: output ownership

Builders return output records. The build system owns path validation, conflicts, manifests, skip-unchanged writes and stale cleanup.

## Phase R8: documentation and final repository audit

Update:

- language and project-structure docs
- module-root-relative import examples
- support and facade examples
- `moth new` scaffolding
- compiler educational pages
- downstream HTML-Wasm plan
- progress matrix only for implemented behaviour

Final repository audit:

- source, tests, docs and roadmap agree
- no legacy import or package shape remains
- no stale Phase 5 comment or plan instruction remains
- generated documentation builds in release mode
- benchmark cases prove successful compilation rather than early exit

## Required end-to-end contracts

The integration suite owns user-visible behaviour. Focused Rust tests own hidden identity, scheduling and impossible-state invariants.

Required contracts:

- shared module compiled once across entries
- shared failure diagnosed once
- independent graph branches continue
- blocked consumers emit no cascades
- module-root-relative imports from nested files
- strict child facade and support visibility
- separate project, Core, Builder and dependency package graphs
- stable identities independent of local allocation and thread completion
- re-export aliases change bindings without changing declaration origin
- provider -> facade -> consumer closure without reopening transitive providers
- imported receivers, traits, evidence, defaults and folded values use interfaces
- cross-module borrow, transfer and return-alias summaries
- generated sidecar deduplication and nested fixed point
- generated private helper and evidence calls
- exact entry runtime and asset unions
- API-only roots have no `start`
- check-only units never enter canonical artefacts or backend roots
- deterministic diagnostics and output
- direct and transitive project-context package rejection
- one source read, tokenization, preparation and module compilation
- one generated materialisation per stable identity

## Validation policy

Every parent-accepted code slice requires:

```bash
cargo fmt --all
just validate
```

Use focused tests while iterating. They are not the acceptance gate.

Run the manual architecture audit from `validation.mtf` whenever a slice changes:

- source discovery or preparation
- interface binding or closure
- AST/HIR stage ownership
- semantic identities
- borrow or lifetime summary handoff
- graph scheduling
- generated sidecars
- backend handoff

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
