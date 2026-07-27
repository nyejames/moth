# Canonical module compilation and scoped packages implementation plan

## Purpose

Continue the canonical-module work from the current accepted checkpoint while correcting the R2 ownership and sequencing problems before implementation resumes.

The target remains:

- one canonical semantic compilation per physical module inside one project or package boundary
- one deterministic Stage 0 source inventory with no repeated filesystem discovery
- source preparation, tokenization and header syntax produced at most once per selected source
- immutable completed provider interfaces rather than copied donor headers, AST or HIR
- generated concrete functions in build-owned sidecars
- explicit graph outcomes, entry assemblies, package assemblies and link plans
- strict scoped support packages and module-root-relative imports
- small data-oriented owners with dense IDs, contiguous records and no abstraction hierarchy

This file is a drop-in replacement for the previous plan at the same path. Accepted semantic identity, graph, direct-interface and R2 leaf-value work is retained. The remaining implementation order is replaced.

## Current state

```text
ACTIVE_PLAN: docs/roadmap/plans/canonical-module-compilation-and-scoped-packages-plan.md
STATUS: active
CURRENT_SLICE: R5B-R5K/R5P3 — inverse canonical type/declaration/value projection and imported fallible carriers complete; paused before recursive provider-interface closure
LAST_ACCEPTED_COMMIT: main squash checkpoint for canonical module integration through inverse projection (this commit; hash intentionally omitted to avoid self-reference)
WORKTREE: `main`; clean after the requested squash checkpoint, while the larger R5 atomic production-acceptance boundary remains incomplete
REQUIRED_RELOADS: startup files, this plan, R5B-R5C/R5P3, directory wave scheduling, module semantic drafts, public-interface owners and current diff
RELEVANT_CONTEXT_NOW:
- user decision: Moth is pre-alpha and compatibility is not required; obsolete APIs, project shapes and import fallbacks may hard break and no shim may survive
- R2C/Review phase 1 and R3/Review phase 2 are accepted; the provider-consumer contract fixes source calls on stable `OriginFunctionId`
- R4 accepted work owns indexed source/package boundaries, production namespaces, ID-only project semantic sets, frozen adjacency and the project prepare-once store; package/check-only jobs remain deferred to their live scheduler consumer
- R5A owns the internal `ModuleSemanticDraft`; generic requests still use the legacy eager path until R5F and no placeholder state was added
- R5P1 replaced the backend `Vec<Module>` API outright with success-only `ProjectCompilation`; build-owned `EntryAssembly` records select live root activity and HTML consumes an owner-bound entry-module view
- R5P2 records narrow direct runtime/link facts per base HIR block and function; build-owned entry planning derives one coherent `HirBackendSelection` plus exact runtime-feature and external-import unions
- HTML validation, JS symbol/body emission and selected Wasm lowering consume that build-owned selection; backend `start`/export reachability rediscovery and the obsolete policy variants are deleted
- selected JS/Wasm validate exact function membership, CFG assignment and local call closure before lowering; selected Wasm consumes validated per-function blocks and rejects unselected exports
- three Codex CLI R5P2b audits were resolved; the final pass found no production defect and its optional collector/symbol duplication plus JS symbol-boundary test feedback was accepted
- R5P3 sequencing inspection proved stable call production and borrow consumption require exact provider `OriginFunctionId` and `PublicCallSummary` facts before backend handoff; current wave scheduling retains results but publishes no interface to later waves
- the temporary `ModuleEntryCompileWaves` inventory filters support/facade provider roles and compiles donor closures, while active-root preparation has no API-only support/facade role and no inverse canonical-interface binder exists
- a Codex CLI boundary audit rejected the current R5K0/R5B/R5C ordering: role-aware root preparation is one safe replacement slice, but provider binding, graph outcomes, non-generic re-export closure, stable HIR/borrow targets, R5P3 JS handoff and donor deletion require one later acceptance boundary
- the user selected preservation of current cross-module generic support; the donor-deletion acceptance boundary therefore expands through R5D-R5I generated sidecars rather than adding temporary rejection diagnostics
- R5R now threads graph-owned root role through preparation/AST/HIR, represents `start` explicitly as optional and guards normal-only backend boundaries; support/facade semantic tests produce no `start`
- the R5R audit correctly found that the legacy scheduler cannot make API-only compilation a production behaviour: it filters support/facade jobs and donor-compiles support roots as imported files, so R5R is a prerequisite seam whose production acceptance belongs to the all-role R5K cutover
- directory compilation jobs now retain graph-assigned `ModuleId` separately from portable stable origin and restore post-Rayon result, string-table merge and diagnostic order by `ModuleId`; the Codex audit's stale-comment and post-task regression findings were accepted and corrected
- a bounded provider-store prototype was rejected before acceptance because it was dormant and wiring its successful slot to `ModuleSemanticDraft` would publish an incomplete interface; R5B must land with completed artefact storage and its first real next-wave consumer
- Spark exploration confirmed the smallest non-transitional bundle spans all-role job inventory, completed artefact publication, next-wave interface consumption and removal of the current public-interface drop; generic metadata remains unchanged until the same atomic train replaces discard-before-remap with the complete R5E-R5I owner
- atomic cutover in progress: module discovery now seeds all graph nodes and schedules normal, support and facade roles in graph waves; semantic input assembly excludes cross-module project donor sources and the obsolete donor-ordering test now protects their absence; focused `cargo check`, semantic-set tests, all 126 Stage 0 tests and all 24 directory frontend tests passed at the recorded checkpoints, but interface binding/publication and generic preservation remain and this work is intentionally unaccepted
- the graph scheduler retains resolved provider bindings, publishes completed artefacts after ModuleId-ordered wave merges, blocks consumers of diagnosed providers, aborts later waves on infrastructure failure and passes immutable interfaces into later-wave binding
- direct concrete imported functions now carry stable `CallTarget::CrossModule` identities through HIR, consume provider `PublicCallSummary` facts in borrow validation, close entry reachability over provider functions and use a build-owned stable JavaScript symbol plan; linked module bundles are scope-isolated so runtime/private symbols cannot collide, and Wasm rejects linked cross-module calls through structured backend validation
- consumer environments now retain stable imported declaration records and inverse-project every reachable direct canonical semantic category: builtins including `Error`, source and external nominals, options, collections, maps, fallible carriers, generic instances and parameters, aliases, structs, choices, constants, folded values/defaults and concrete callable contracts
- `TypeEnvironment` now owns a durable bidirectional canonical interner; imported nominal registration preserves exact package/module/role origin, reuses canonical `TypeId` values and projects only the semantic closure reachable from direct imported declarations, so unrelated exported open generics cannot poison concrete imports
- `TypeEnvironment` is the one imported-struct field owner consumed by AST and HIR, imported choices are handed explicitly to HIR, and direct imported fallible calls lower through their projected carrier and stable cross-module target for propagation and catch recovery
- the JavaScript field ABI now uses collision-free UTF-8 hex names shared with builtin `Error` runtime glue; absent field metadata is an internal compiler error rather than a local-ID fallback
- inverse projection was split into focused canonical, nominal, value and callable owners under `ast/module_ast/environment/builder/import_projection`; recursive re-export closure, receiver/trait/evidence projection and generated generic sidecars remain incomplete
ACCEPTANCE_CRITERIA:
- compile every selected normal, support and facade module exactly once through graph jobs
- bind consumers only from completed immutable provider interfaces and stable identities
- preserve cross-module generic support through generated sidecars before deleting donor compilation
- make API-only activity rejection and no-`start` artefacts reachable through the production scheduler
- delete donor-copy, entry-closure and fallback paths in the same accepted boundary; add no compatibility path
VALIDATION_STATE:
- inverse-projection audit: the first Codex CLI read-only review reported lossy synthetic canonical identity, eager provider-wide nominal projection, non-injective Unicode field ABI/missing-metadata fallback and duplicated imported struct ownership; every finding was accepted and corrected
- inverse-projection follow-up audits: duplicate AST-to-HIR struct fields, weak direct invariant tests and a source-spellable imported nominal namespace were accepted and corrected; the internal namespace is now structurally unauthorable and the regression test covers the former local/imported collision
- inverse-projection focused gates: `cargo check --workspace`, `cargo test --workspace --quiet`, exact origin-path/reverse-identity tests, exact Unicode field-ABI and missing-metadata tests, four fallible-wrapper tests and the strengthened `module_facade_explicit_export_surface_success` HTML case passed; the fixture covers imported construction/field reads, success and failure catch paths, builtin `Error.message`, and unrelated exported open generic nominals
- inverse-projection `just validate`: native/Linux/Windows Clippy, 3,813 workspace, 17 CLI and 500 package tests passed; integration reached 1,802 executions with 1,740 correct and 62 expected incomplete-boundary failures concentrated in recursive provider interfaces, receiver/evidence closure and generic sidecars; docs and benchmark did not run because integration failed
- R5P2b focused checks: cargo check plus reachability, validation, JS, Wasm, HTML, build-owner and reactivity suites passed
- R5P2b audits: three read-only Codex CLI passes; every correctness finding and final optional hardening item was accepted and corrected, and the final pass found no production defect
- R5P2b `just validate`: native/Linux/Windows Clippy, 3,803 workspace, 17 CLI, 500 package tests and all 1,802 integration executions passed; docs stopped at known MOTH-IMPORT-0005 in `docs/src/docs/#page.moth`; benchmark did not run
- blocker and preservation-sequence checkpoint docs gates: `moth` was unavailable; `cargo run --quiet -- build docs --release` reached the known MOTH-IMPORT-0005 missing `styles/docs/navbar` failure in `docs/src/docs/#page.moth`
- R5R Codex CLI audit: required scheduler-reachability finding accepted into the atomic cutover; call-specific diagnostic-location polish accepted and corrected; no other production defect found
- R5R `just validate`: native/Linux/Windows Clippy, 3,807 workspace, 17 CLI, 500 package tests and all 1,802 integration executions passed; docs stopped at the same known MOTH-IMPORT-0005; benchmark did not run
- ModuleId-ordering `just validate`: native/Linux/Windows Clippy, 3,808 workspace, 17 CLI, 500 package tests and all 1,802 integration executions passed; docs stopped at the same known MOTH-IMPORT-0005; benchmark did not run
DOCS_IMPACT: progress support unchanged; downstream `html_project_backend_wasm_final_implementation_plan.md` still describes the removed flat handoff and reachable-from-start policy and needs a separately authorised refresh
BLOCKERS_OR_OPEN_DECISIONS: no design blocker. Remaining implementation is required before R5 production acceptance: recursive re-export/provider-interface closure, receiver/trait/evidence projection, validated interface invariants and generated sidecars preserving cross-module generics. Donor fallback remains forbidden and exact Wasm cross-module success remains deferred
DELEGATION_DECISION: parent-direct implementation with Codex CLI audit after each accepted slice, per the user-approved workflow reversal
NEXT_WORKER_ORDER: none; Codex CLI is review-only
STOP_REASON: user requested a pause after the inverse-projection slice is committed and recorded, then requested the entire feature branch be squash-merged into `main` and removed
NEXT_RESUME_ACTION: implement recursive provider-interface re-export closure and interface-invariant validation, then receiver/trait/evidence projection and generated-sidecar preservation before donor-path deletion
```

Do not change this block as part of the plan replacement. Later implementers update its values only after parent acceptance of a slice. Do not append worker transcripts, complete validation logs or worktree journals. Git history is the durable implementation history.

## Required authorities

Read these before every implementation or review phase:

- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/language-overview.md`
- `docs/src/docs/codebase/memory-management/overview.mtf`
- `docs/src/docs/codebase/memory-management/borrow-validation/overview.mtf`
- `docs/src/docs/codebase/memory-management/lifetime-regions-and-escape-validation/overview.mtf`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `docs/src/docs/progress/#page.moth`
- downstream config, entry-config and HTML-Wasm plans

The compiler and build-system overviews remain authoritative. This plan owns implementation order, migration boundaries and deletion gates. It must not create a competing language, memory or backend design.

## Accepted foundation

Keep the accepted work unless a correction slice below explicitly replaces its owner:

- stable package, module, declaration, trait, evidence and exported generic identities
- canonical cross-module type identities
- provider-independent retained header syntax and structural provider references
- one Stage 0 `SourceTreeIndex` traversal with module identities and owned source classification
- the canonical `ProjectModuleGraph` identity, ancestry and dependency-wave model
- separate successful, diagnosed and infrastructure failure classes
- explicit executable, link-fact and compiler-metadata lanes
- the aggregate declaration-centric direct-interface result
- folded public values and defaults
- trait requirements, incompatibilities and reusable evidence
- generic declaration descriptors
- direct synthetic-interface provenance vocabulary
- concrete local call-summary vocabulary and borrow-side summary production

Do not recreate these facts under new parallel names.

The following accepted implementations are migration scaffolding rather than final ownership:

- the current phase-mutable `PublicInterfaceDraft`
- the current broad AST public-projection handoff
- duplicate receiver catalog and receiver surface paths
- the incomplete validated generic-template body store
- the direct-draft canonical byte encoder
- per-entry reachable source closures and `ModuleEntryCompileWaves`
- path-based source import fallback and donor-header binding
- the flat `Vec<Module>` backend handoff

They may be refactored or deleted only through the bounded slices below.

## Course correction

R1 corrected the module-result shape but did not fully consolidate production. R2 then added many producer-side facts before a real provider consumer existed.

The resulting churn has five causes:

1. Direct declarations are still assembled from several complete projection aggregates and rejoined by path, name or origin.
2. `PublicInterfaceDraft` represents pre-HIR facts, post-borrow facts and future generated facts through mutable pending states.
3. `Ast` carries transient public projection and generic-template stores that HIR never consumes.
4. The generic-template artefact is intentionally incomplete, so sidecar requirements keep changing its boundary.
5. Stage 0 already owns one source-tree traversal, but later per-entry discovery still rescans import graphs, builds duplicate caches and compiles entry closures rather than canonical module jobs.

The replacement order is:

```text
close R2 ownership gaps
-> prove the provider consumer contract
-> remove duplicated source discovery
-> implement provider interfaces and generated sidecars as one integration train
-> cut production over to canonical module jobs
-> add per-function link planning and assemblies
-> migrate backends, check, dev and fingerprints
-> delete all legacy paths
```

Do not add more producer schema before the next listed consumer or deletion gate exists.

## Non-negotiable architecture

### One owner per fact

- Each project or package compilation boundary has one `SourceTreeIndex` that owns its physical
  source inventory, module roots, nearest ownership and canonical source IDs.
- Each boundary has one prepared-source store that owns source text, tokenization and retained
  syntax for its selected sources. Raw `SourceId`, `ModuleId` and prepared syntax never cross
  boundaries; immutable interfaces carry stable cross-boundary identities.
- `ProjectModuleGraph` owns module identities, structural edges, support visibility and compile order.
- The compiler owns direct declaration semantics and local executable summaries.
- Completed module artefacts own immutable provider interfaces.
- The generated worklist owns concrete generic request deduplication and sidecars.
- Link planning owns reachability, entry activation and package assembly.
- Backends consume explicit plans and never rediscover source meaning.

A later stage must not reconstruct a fact from source, rendered names, filesystem probing or foreign IR when an earlier owner already produced it.

### Data-oriented storage

Prefer:

- dense build-local IDs such as `SourceId`, `ModuleId` and `GeneratedRequestId`
- contiguous `Vec` storage in deterministic ID order
- small enums for source kind, module role, graph outcome and call target
- construction-time hash maps for lookup only
- sorted adjacency vectors after graph construction
- explicit indexes into immutable stores
- one context struct at stage boundaries

Avoid:

- trait-object provider hierarchies
- nested object graphs with repeated identity fields
- maps of maps where one dense table and side index is sufficient
- `Arc` or `Rc` per semantic leaf
- generic registries that obscure ownership
- open-ended context bags
- parallel representations of the same declaration, receiver or source file

Exact Rust names may change. These ownership and storage rules may not.

### Interface phases are distinct types

Use three semantic phases, not one object with pending variants:

```rust
pub struct DirectInterfaceSeed {
    pub module_origin: StableModuleOriginIdentity,
    pub direct_export_bindings: Vec<ExportBinding>,
    pub declarations: Vec<DirectDeclarationRecord>,
    pub reusable_evidence: Vec<PublicEvidenceRecord>,
}

pub struct LocalPublicInterface {
    pub direct: DirectInterfaceSeed,
    pub concrete_call_summaries: Vec<PublicFunctionSummaryRecord>,
    pub concrete_lifetime_summaries: Vec<PublicLifetimeSummaryRecord>,
    pub provenance: Vec<PublicProvenanceRecord>,
}

pub struct PublicSemanticInterface {
    pub module_origin: StableModuleOriginIdentity,
    pub export_bindings: Vec<ExportBinding>,
    pub declarations: Vec<PublicDeclarationRecord>,
    pub reusable_evidence: Vec<PublicEvidenceRecord>,
    pub concrete_call_summaries: Vec<PublicFunctionSummaryRecord>,
    pub concrete_lifetime_summaries: Vec<PublicLifetimeSummaryRecord>,
    pub provenance: Vec<PublicProvenanceRecord>,
}
```

These are conceptual shapes. Current unimplemented lifetime analysis must not be faked with placeholder facts. The module architecture must provide a narrow slot and stable join point for real lifetime summaries when that owner exists.

Rules:

- `DirectInterfaceSeed` is complete for direct declaration facts or construction fails.
- `LocalPublicInterface` is complete for directly defined concrete functions or absent.
- `PublicSemanticInterface` is complete after provider re-export joining or absent.
- No durable interface type contains `PendingLocal`, `PendingGenerated` or another temporal state.
- Generic templates are declaration contracts, not unfinished concrete functions.
- Concrete generated summaries live on generated sidecars.
- Declared parameter access is part of the signature.
- Mutation, optional transfer, aliasing, reactivity and lifetime facts are analysis summaries.
- Receiver methods remain attached to their receiver records.
- Re-export bindings preserve donor origin identity.

### Module result phases are distinct

A module may temporarily produce an internal semantic draft while generated requests are unresolved:

```text
prepared module
-> bound headers
-> AST build result
-> validated base HIR
-> generated request fixed point
-> generated summaries
-> base borrow and local lifetime analysis
-> local interface finalization
-> provider re-export join
-> successful CompiledModuleArtifact
```

The internal draft is not a provider interface and cannot enter the successful graph result.

### No duplicate work

Inside one project or package compilation boundary:

- each directory is visited once by source-tree indexing
- each selected source file is read once
- each `.moth` source is tokenized once
- each selected source has header syntax prepared once
- each import shell is namespace-resolved once
- each physical module is semantically compiled once
- each generated identity is materialised once
- each provider diagnosis is produced once
- each final fingerprint is encoded once by its final owner

Add or retain counters that can prove these counts in integration tests. Timing data and benchmark output do not prove absence of duplicated work.

## Slice discipline and churn controls

Every implementation slice must define:

- one owning subsystem
- exact inputs and outputs
- the production consumer introduced or changed
- files and legacy owners expected to be deleted
- focused tests
- the final validation gate
- explicit non-goals
- stop conditions

A slice is not accepted when it only adds dormant future data, test-only accessors or `allow(dead_code)` for a later consumer.

### Default churn tripwires

Pause implementation and perform a read-only review when any of these occurs:

- the slice must change more than two stage boundaries not named in its scope
- a second long-lived representation of the same fact appears necessary
- the old and new production path cannot both be removed by the stated deletion gate
- a new abstraction exists only to bridge old and new APIs
- the implementation needs a test-only production accessor
- a future-consumer `allow(dead_code)` would be required
- the same ownership invariant needs a second correction pass
- the slice grows beyond roughly 12 production files or 600 net production lines, excluding mechanical moves, without a parent-approved split
- source loading, tokenization, preparation or module compilation counts increase
- a provider or generated-function identity cannot be defined without rendered names, source positions or donor-local IDs
- user-visible failures would need `CompilerError`
- a required validation gate is unreliable or fails for an unrelated infrastructure reason

The implementer must stop cleanly, preserve the work, update the current-state block with the exact unresolved question and request plan-owner review. Do not improvise a compatibility layer.

### Commit and acceptance policy

- Focused commands are iteration evidence.
- Each parent-accepted code slice runs `cargo fmt` and full `just validate`.
- A review phase is read-only. Corrections become separate bounded slices.
- Intermediate commits on an integration branch are allowed.
- Do not merge an integration phase to the accepted baseline while it leaves a second callable production architecture.
- Documentation-only plan edits use the documentation release-build gate.
- Review the progress matrix only when current support or rejection behaviour changes.

## Preserved review phases

The existing parent-review model remains. These reviews are mandatory pauses, not optional summaries.

### Review phase 1: R2 boundary audit

Scope:

- the four retained R2 audit findings
- direct-interface stage ownership
- receiver and callable identity ownership
- generic template contract versus concrete generated summary
- encoder and folded-float semantics
- module/file organisation

No R3 work starts until every accepted finding is fixed or explicitly moved to a named later owner.

### Review phase 2: provider consumer contract

Scope:

- one provider and one consumer
- imported function, type and folded constant
- canonical-to-local type projection
- declared parameter access
- cross-module call summary lookup
- no provider AST, HIR or private header access

This review happens before generated sidecar implementation. A bounded disposable spike is allowed only when static inspection cannot settle the contract.

### Review phase 3: discovery and graph audit

Scope:

- one source-tree traversal
- lazy prepare-once source storage
- module-root-relative namespace resolution
- semantic source sets
- support visibility
- graph edge and wave construction
- deletion of parallel import scanning and path fallback

### Review phase 4: generated sidecar audit

Scope:

- complete immutable template artefacts
- request identity and deduplication
- private declaring-module helper targets
- fixed-point scheduling
- generated HIR, borrow facts and real lifetime facts where available
- requester finalization
- deletion of consumer-local materialisation

### Review phase 5: canonical production cutover

Scope:

- every selected module role
- completed provider registry
- graph outcomes
- blocked consumers
- string-table merge order
- deletion of entry closures, donor copying and fallback resolution
- one production compiler path

### Review phase 6: link, backend and reuse audit

Scope:

- per-function link facts
- entry and package assemblies
- project-context provenance
- target and lifetime-topology roots
- `ProjectCompilation`
- backend handoff
- five fingerprints
- `check` and dev reuse
- final deletion audit

Each review reports findings only. Parent-approved correction slices must complete before the next implementation phase.

## Current owner disposition

Keep and evolve:

- `source_tree_index.rs` as the only directory-project filesystem inventory owner
- `module_identity.rs` as the dense module identity owner
- `project_module_graph.rs` as the structural graph and scheduling owner
- `prepared_source.rs` as the source-kind-safe prepared value concept
- `semantic_identity.rs` and `canonical_type_identity.rs`
- stable folded-value, evidence, provenance and call-summary leaf types
- `ModuleCompilationOutcome` and diagnosed versus infrastructure separation

Refactor:

- `public_interface_draft.rs` into a small module tree with separate model, projection and finalization owners
- `AstPublicInterfaceProjectionInput` into an AST side result
- receiver method projection into one callable seed authority
- local call-summary finalization into stable tables keyed by origin
- validated generic-template metadata into a complete materialisation artefact
- `ProjectModuleGraph` edge storage into sorted dense adjacency when construction is complete
- `PreparedSourceInput` storage to use `SourceId` slots rather than per-entry owned copies

Delete at the named phase:

- `PublicCallSummaryState`
- duplicate receiver catalogs/surfaces used only for public projection
- direct-draft canonical bytes and encoder-only accessors
- `DiscoveredModule`
- `ModuleEntryCompileWaves`
- per-entry reachable source caches
- provider-free versus provider-capable duplicate discovery paths
- path-pair structural dependency facts
- source import fallback through `ProjectPathResolver`
- configured `package_folders` and default `/lib`
- donor header/body copying into consumers
- consumer-local generic materialisation
- module-level `start` reachability as link authority
- complete external package registries on every module
- flat `Vec<Module>` backend input
- compatibility wrappers introduced during migration

## Phase R2C: close the direct-interface boundary

Goal: finish the current R2 audit by fixing ownership and phase modelling. Do not add another public semantic feature.

### R2C1: declaration-owned parameter access

Completed in `715614974`. Resolved source signatures now own Shared, Mutable and Reactive
parameter access before HIR for free functions, receiver methods and generic templates. Borrow
analysis retains mutation, transfer, alias and reactive effects as separate summaries. Focused
final-record tests cover every access family.

### R2C2: replace temporal callable states

Completed in `6669754ad`. Concrete local callables and generic template declarations now use
distinct semantic categories. `PublicCallSummaryState`, `PendingLocal`, `PendingGenerated` and
their encoder branches are deleted. Concrete exported functions require a post-borrow summary,
while generated concrete summaries remain sidecar-owned.

### R2C3: one receiver and callable seed owner

Completed in `429eb156b`. One transient post-AST `CallableSeed` table owns exact callable and
receiver identity for direct projection, declaration records, HIR origin seeding and validated
generic-template extraction. Evidence consumes completed receiver surfaces rather than the donor
catalog, and no consumer reconstructs a receiver origin.

### R2C4: move public projection out of `Ast`

Completed in `9e300579d`. `AstBuildResult` separates executable `Ast`, public-interface projection
input and validated generic-template input. HIR receives executable AST state only. The old
`Ast`-resident projection/template stores and their take-before-HIR scaffolding are deleted.

### R2C5: direct declaration-oriented projection

Completed in `4bc0c6e61` and `b42225f76`. The former monolith now lives under
`src/compiler_frontend/public_interface/` with focused model, export/type/receiver/trait/evidence,
direct-projection and local-finalization owners. Direct projection builds one declaration record
per stable origin with separate export bindings, receiver attachment once and evidence over the
completed receiver surface. The aggregate `DefinedPublic*` owners are deleted.

### R2C6: concrete summary join and HIR injectivity

Completed in `7446eb178`. HIR validates both directions of the stable-origin/`FunctionId`
relationship. Local finalization joins concrete summaries once into an origin-sorted table,
rejects missing, extra, duplicate, wrong-category and shape/effect drift, and excludes private
functions and `start`. Borrow validation remains read-only over local HIR.

### R2C7: folded Float decision and encoder rollback

Completed in `d814f0c94`. The semantic review found no language rule that equates the two IEEE zero
signs globally, so `FiniteFloat` preserves exact finite bits and exact-bit equality while the
common Float-to-String boundary alone renders `-0.0` as `0`. The incomplete direct-draft encoder,
encoder-only accessors/tests and stale dead-code scaffolding are deleted. Final fingerprint
encoding remains owned by R7E.

### R2C exit gate

Complete. Review phase 1 ran two whole-phase audit/correction cycles. Corrections restored exact
boundary coverage and truthful fixtures (`74394c0a7`), split public-interface tests by production
owner (`0f3225407`) and aligned R5D-R5G comments plus the frontend owner index (`e6da36cb3`). Both
audits found every production exit-gate invariant satisfied. The final code-bearing checkpoint
passed `cargo fmt --all`, cross-target Clippy, 3,751 Rust library tests, 17 CLI tests, 500 package
tests, 1,801 integration cases, docs and all 58 benchmark preflights with 8/8 CLI and 10/10
frontend quick measurements.

## Phase R3: prove the provider consumer contract

Complete. Static inspection traced a directly exported non-generic free function, nominal type and
folded constant through current production and the future consumer boundary. It found the current
provider draft and concrete summaries complete for the vertical subset, but the legacy handoffs
drop them, donor-path imports still make provider declarations local and HIR has no cross-module
source target. No spike or production scaffolding was needed.

### Accepted provider-consumer contract

R5B-R5C implement the exact vertical consumer trace:

1. Stage 0 resolves structural providers to successful immutable artefact slots by build-local
   `ModuleId`; that ID never enters stable semantic identity or HIR.
2. Interface binding validates the export's stable exporting-module origin, looks up the public
   `str` and records the closed source-or-binding target. Local spelling and location stay
   consumer-owned; no donor path, IR or local handle crosses the stable boundary.
3. AST recursively interns `CanonicalTypeIdentity -> TypeId`, retains the reverse identity and
   imports nominal shapes, function signatures and owned folded values without donor syntax.
   Repeated projection returns the same consumer-local `TypeId`.
4. Source functions bind exact `OriginFunctionId`; AST owns declaration access for typing and HIR
   retains `SourceCallTarget::CrossModule` without aliases, `ModuleId` or donor `FunctionId`.
5. Borrow validation receives one explicit resolver view. The cross-module arm returns the
   complete validated `PublicCallSummary`, including access, by `OriginFunctionId`; generated and
   binding arms remain with their named later owners.
6. `VisibleNameRegistry` remains the collision owner and compares stable imported targets.
   `FileVisibility` stores local-name-to-imported-target records; only consumer-local resolved
   handles accompany them where AST/HIR need one.

R5C consumes the current draft's module origin, source export bindings and function/struct/constant
declaration records, including canonical signatures, declared access, nominal field shape, folded
constant value and complete origin-keyed call summaries. R5B-R5C add provider slots, immutable
lookup views, imported target records, canonical-to-local interning with reverse origin, stable HIR
targets and the cross-module resolver arm.

R5C deliberately excludes generics and sidecars (R5D-R5I), aliases, choices, defaults, receivers,
traits, evidence, namespace members, binding-backed re-exports and final closure (R5J), real
lifetime facts until their memory-analysis owner exists, link planning and fingerprints (R6-R7).
Direct project provenance travels through R5A and R5I; R5J joins re-export closure and R6D validates
direct and transitive package eligibility.

R3B was skipped because static inspection settled every question without a disposable spike.

### Locked binding decisions

Conceptual closed export-target identity:

```rust
pub enum StableExportTargetIdentity {
    Source(OriginDeclarationId),
    Binding(CanonicalBindingSymbolIdentity),
}

pub struct CanonicalBindingSymbolIdentity {
    pub package: StablePackageIdentity,
    pub symbol_path: Vec<String>,
    pub category: BindingDeclarationCategory,
}
```

`BindingDeclarationCategory` is the closed semantic function/type/constant category, not runtime
helper or lowering metadata. Exact Rust names may change; the stable-vs-build-local boundary may
not. Evolve the existing `StablePackageIdentity` owner to cover the binding package's origin and
exact canonical package path; do not create a parallel package-identity registry.

- Provider store lookup uses build-local `ModuleId`. Public-name lookup uses the authored or
  re-exported `str` and returns a closed stable export target. A source target carries
  `OriginDeclarationId`; a binding-backed target carries an owned canonical binding package
  identity, structured symbol path and declaration category and never an `ExternalSymbolId`.
  Source declaration lookup uses `OriginDeclarationId`. Concrete summary lookup and cross-module
  HIR calls use `OriginFunctionId`. Canonical type interning keys the complete
  `CanonicalTypeIdentity`.
- The build system owns provider slots and selects the immutable interfaces supplied to a module
  job. Compiler interface binding owns public-name resolution, privacy and file visibility. AST
  owns canonical-to-local type and folded-value projection. HIR owns stable call targets. Borrow
  validation consumes summaries through an explicit resolver view.
- A bound imported target stores its closed stable source-or-binding identity, while its local
  spelling and source location remain consumer-owned. Binding-backed targets resolve to any
  consumer-local external handle through the supplied binding-package interface. The stable
  binding does not store a donor path or donor-local/build-local handle.
- Missing or private public names use the existing structured import diagnostic at the consumer
  import site. A diagnosed provider blocks the consumer before semantic compilation. An export
  binding without its declaration, a concrete callable without its summary or an inconsistent
  successful provider slot is `CompilerError`.
- Provider interface strings and folded values remain self-contained owned data. Binding interns
  only consumer-local spellings and projected member names into the consumer string table.
  Provider `StringId`s and source locations never require consumer remapping.
- Re-export bindings preserve the donor's closed stable export target: source declarations retain
  `OriginDeclarationId`, while binding-backed targets retain their canonical binding package and
  symbol identity. The binding owns the exporting module and public alias.
- The exporting `PublicSemanticInterface` carries the recursive fixed-point closure of every
  interface-owned semantic fact reachable from its export bindings: declarations and nested
  canonical facts, concrete call summaries, real lifetime/outlives summaries when their owner
  exists, generic contracts, defaults and folded values, receiver surfaces, traits, reusable
  evidence, external-boundary classifications and project-context provenance. Closure records
  need no public export binding of their own. `ModuleLinkFacts`, call edges, helpers, assets and
  backend/lowering metadata never enter the closure. A consumer never opens a transitive source
  provider. Any missing closure record in a successful interface is `CompilerError`.
- R5C may add derived interface indexes and compact per-module imported-binding tables. It must not
  clone a complete interface, copy provider declarations into local declaration tables or add a
  compatibility adapter around donor-header binding.

Review phase 2 ran two whole-phase audit/correction cycles. Cycle 1 retained summary-level access,
made re-export closure recursive and completed R5C coverage (`c375d66f1`). Cycle 2 added stable
binding-backed export targets, complete semantic closure/provenance, corrected R5I/R5J and R5F
sequencing and added hidden invariant coverage (`577252018`). R3 changed only this plan; both
documentation gates built 67 files with no generated diff. The compiler overview's conceptual
`ExportBinding.exporting_module: ModuleId` remains separately reported documentation drift against
its stable-identity rules and production `StableModuleOriginIdentity`.

## Phase R4: eliminate duplicated source discovery

Goal: evolve the existing `SourceTreeIndex` and `ProjectModuleGraph` into the complete Stage 0 data path without adding a second index.

### R4A: central source IDs

Evolve `SourceTreeIndex` in place.

R4A1 is complete at `600d3d963`. `SourceTreeIndex` now owns one contiguous
portable-identity-sorted `SourceRecord` table addressed by dense `SourceId`; duplicate logical
identities fail before ID assignment. Owned and unrooted collections store IDs only. The retained
index sits beside `ProjectModuleGraph`, whose nodes no longer clone source records or ownership
sets. Focused tests cover dense IDs, exact-once membership, owned/unrooted state, cross-checkout and
creation-order determinism, facade ownership and source-origin projection.

Conceptual storage:

```rust
pub struct SourceTreeIndex {
    pub sources: Vec<SourceRecord>,
    pub module_identities: ModuleIdentityTable,
    pub owned_source_ids: Vec<Vec<SourceId>>,
    pub unrooted_source_ids: Vec<SourceId>,
    pub stats: SourceTreeDiscoveryStats,
}
```

Each source record owns:

- dense `SourceId`
- canonical physical path for IO
- portable logical identity
- `SourceFileKind`
- owning `ModuleId` or explicit unrooted state

Rules:

- do not introduce `ProjectSourceIndex`
- assign IDs in deterministic logical path order
- absolute paths never become semantic identity
- each source record exists once
- owned sets store IDs rather than duplicate source records

Keep the current one-traversal root and collision logic.

#### R4A2: indexed explicit-provider files

Complete the accepted project-index contract for explicit provider-owned files before prepared
storage begins. Reuse the existing external-provider extension registry during the same project
traversal, add a closed record classification that distinguishes compiler semantic sources from
explicit provider inputs, and retain nearest-module ownership plus portable logical identity.

For the migrated provider-import path, resolve the authored provider reference through the central
index to `SourceId`, enforce the existing module boundary through indexed ownership, and pass the
record's canonical path to the provider as its IO handle. Remove the provider-only canonical path
probe and boundary reconstruction for that path. Do not index source-backed package-private files
in the project boundary, add a second registry or enter prepared-source storage.

R4A2 is implemented pending its acceptance commit. The project index now carries one closed
semantic/provider classification plus derived logical-path and canonical-path lookup maps into the
same record table. Directory provider imports resolve and validate an indexed target before the
provider runs. The obsolete project-local `package_folders` provider success path was deleted
rather than retained through a filesystem fallback. Single-file synthetic compilation keeps its
separate filesystem resolution because it has no directory-project source index.

### Corrected R4/R5 integration order

R4B cannot precede canonical consumers while the legacy entry-closure compiler owns the live
lexical scan and injects complete source-backed package roots into project modules. A second
prepare-once path duplicates source work; an unconsumed store is dormant; feeding a project store
into donor closures requires the forbidden cross-boundary adapter. Pre-alpha compatibility does
not constrain the correction: obsolete source-import forms and fallbacks must hard break when
their indexed replacement lands. The remaining migration therefore proceeds as one integration
train:

1. R4A2 and R4P1 establish the current project/provider and package-boundary indexes.
2. Before source storage, land R4C1 as a production replacement inside the current traversal:
   retain the separate package indexes, build boundary-aware module namespaces, resolve the
   already-scanned structural references through boundary-local IDs and insert direct project
   graph edges by `ModuleId`. Delete directory source-import filesystem probing, public-surface
   fallback and path-pair dependency facts in the same slice. The existing scanner remains the
   sole lexical owner, so source-read and tokenization counts do not increase.
3. Complete R4C-R4E around the production ID-valued reference output: semantic source sets,
   package graphs, frozen adjacency and compilation jobs become current owners rather than
   parallel discovery structures. Raw IDs never cross a project or package boundary; completed
   immutable interfaces are the only cross-boundary values.
4. Implement R5A-R5J as replacements of the current semantic owners on the integration branch.
   Introduce each prepared boundary store only with its first canonical consumer and never run it
   beside a second lexical/header-preparation owner in one build.
5. Execute the final R4B/R4F and R5K scheduler switch as one atomic production cutover: the stores
   become the sole source-preparation owners and the legacy scanner, BFS, donor closures and path
   resolver source-import path are deleted. Then run Review phases 3, 4 and 5 at their recorded
   gates.

Do not accept a baseline state in which the prepared-store path and donor-closure semantic path are
both callable. Do not bridge them with cross-store aggregation, path fallback, an auxiliary
unindexed store or another source identity.

### R4P1: source-package boundary indexes

Build one `SourceTreeIndex` per selected source-backed Core, Builder, project-local legacy or future
dependency package boundary, stored in deterministic import-prefix order. Refactor the existing
index traversal around an explicit boundary descriptor so project and package indexing share one
implementation while retaining distinct stable package identities, source IDs, module IDs and
ownership tables.

The package index becomes the filesystem owner for root discovery and sibling source/folder
collision checks. Derive the current resolver's narrow package-root lookup view from indexed facts
and delete the separate root-file and package-tree collision scans. Preserve the structured
missing-root, multiple-root, unreadable-root and collision diagnostics. Do not build a prepared
store, package graph, cross-boundary aggregate index or compatibility source identity in this
slice.

R4P1 is complete at `be0d9ce85`. `SourceTreeIndex` now takes a typed project or
package boundary descriptor and remains the only traversal implementation. Package indexes use
stable origin-plus-prefix package identities, boundary-local dense IDs and deterministic prefix
order. Their validated root projection has no missing/pending state. The separate root validation
and recursive collision modules were deleted, and focused tests cover boundary-local identities,
provider-file ownership, hash/support root rules and preserved structured diagnostics.

### R4B: prepare-once source store

Add one `PreparedSourceStore` per project or package compilation boundary, indexed by that
boundary's `SourceId`.

The first R4B1 implementation attempt proved that the legacy directory traversal unconditionally
injects source-backed package roots into every project entry closure while the project
`SourceTreeIndex` correctly indexes only the project boundary. The build-system authority requires
Core, Builder and dependency source packages to compile as separate graphs with their own source
indexes. Do not resolve this by adding package-private files to the project graph, creating an
auxiliary unindexed prepared-source path or retrying R4B before the production namespace and graph
consumers land.

Each slot has a small explicit state:

```text
Unprepared
Prepared
Diagnosed
```

Preparation is lazy and deterministic:

- build prepares roots first in `ModuleId` order
- retained structural references enqueue additional `SourceId`s
- `check` eventually prepares owned orphan `.moth` sources
- each slot may transition once
- `.moth` text is read once, tokenized once and header-prepared once
- `.mtf` and `.md` use their one source-kind adapter path
- completed syntax is retained for later binding and AST use

Do not retain a separate lexical import scanner when header syntax preparation already produces structural provider references.

Provider-backed discovery may remain serial while it mutates provider caches. It still uses the same source slot and never repeats tokenization.

#### R4B1: project-store graph-edge vertical

Land the first bounded store slice only where it has a current production consumer. Add the
project-boundary store and use retained structural references to insert direct `ModuleId` graph
edges, replacing `LocalStructuralDependencyFact` collection and path-pair merging. Keep the legacy
entry-closure semantic compiler isolated from the store until R4F/R5K. Do not create package stores
yet: R4B2 adds them with the package-graph consumer so no dormant cache or compatibility adapter
exists between boundaries.

The first implementation attempt was rejected and fully removed. It inserted graph edges from a
second lexical import scan, dropped the store immediately, retained future-only fields under
dead-code allowances, left `.mtf`/`.md` unprepared and doubled whole-build source reads while the
legacy entry-closure scanner remained isolated. The sequencing audit moved R4C1 and the directly
consumed part of R4E ahead of this phase. Do not attempt R4B1 again until those production consumers
are accepted.

### R4C: module namespaces without filesystem probing

Build one `ResolvedModuleNamespace` per module from:

- owned source IDs
- direct child module IDs
- visible support package IDs
- registered Core, Builder and dependency packages
- explicit provider-owned files
- synthetic compile-time interfaces

Namespace entries use explicit tagged records. No precedence or ordered fallback exists.

Source import resolution:

- starts from the owning module root
- resolves through the namespace and owned logical paths
- stops at child module and support boundaries
- rejects `@./`, parent components and private path bypass
- never calls `read_dir`, `exists` or fallback-candidate probing
- remains separate from compile-time path-literal resolution

Wire the existing support visibility query or replace it in the same slice. Do not leave it dead.

#### R4C1: production namespace and direct-edge vertical

Retain `SourcePackageBoundaryIndexes` in the directory-project setup beside the project index; do
not flatten them or allow one boundary's raw IDs to address another. Build the first complete
`ResolvedModuleNamespace` set from indexed project modules, indexed source-package modules, support
visibility, provider-owned records and compile-time package registrations.

The live reachable traversal must consume this namespace using the structural references already
held by `ScannedImportSource`. Resolve each compiler-semantic source target to an explicit
boundary-tagged source/module result. The target record's canonical path may then be used only to
read or enqueue the source through the current compiler path; it is not semantic identity. For a
project-local cross-module result, insert the provider-before-consumer edge directly by `ModuleId`
and retain the authored location in the graph side table. Delete `LocalStructuralDependencyFact`
and its path-to-ID merge.

For directory compiler-semantic imports, delete the migrated filesystem candidate probing and
public-surface fallback. Bare module/package surfaces must be explicit namespace entries. Reject
`@./`, parent traversal and private child/support bypass with structured diagnostics. Single-file
synthetic compilation and compile-time path literals remain separate owners and are not broadened
by this slice.

Do not add a `PreparedSourceStore`, re-tokenize a source, scan imports a second time, construct an
interim path namespace or aggregate boundary IDs. Counter tests must prove whole-build source reads
and tokenization do not increase from the accepted R4P1 baseline.

### R4D: semantic source sets

Build each `SemanticSourceSet` by traversing retained structural references over `SourceId`s.

Classification:

- same-owner source reference adds a source ID
- cross-module source reference adds a module graph edge and does not add provider source to the consumer set
- binding/provider reference adds the appropriate package/provider edge
- child module or support boundary exposes only its interface
- unsupported or missing source kinds produce structured diagnostics
- a source cannot belong to two semantic module sets

For `check`:

```text
check-only source IDs
= owned .moth source IDs
- canonical semantic source IDs
```

Check-only units reuse the same prepared source and provider namespace. They never enter canonical artefacts or backend roots.

#### R4D1: project semantic-set production vertical

Land the project-boundary set only where the live donor traversal can consume it without a second
scanner or dormant owner. Each current normal entry module gets one deterministic set of its
same-owner compiler-semantic `SourceId`s. Current `PreparedSourceInput` assembly must read that set
as the authority for project semantic membership while the retained scan cache remains the IO and
token handle. Cross-module roots, source-package facades and provider/binding inputs stay explicit
interface/external inputs for the donor compiler and never enter the consumer's semantic set.

This is a bounded migration seam, not compatibility resolution: obsolete import spellings remain
rejected, and path handles carry no semantic identity. Do not create package sets, check-only sets
or a prepared store in this slice. Counter and ownership tests must prove deterministic membership,
no cross-module implementation leakage and unchanged reads/tokenization.

R4D1 is complete at `8501120f3`. The live traversal now builds one ID-only set from retained
namespace resolutions, and current input assembly projects its ordered project members from the
central index before appending explicitly separate donor interface and package inputs. Focused
coverage protects deterministic membership, boundary exclusion and unchanged source work.

#### R4D2: package and check-only completion

Add package-boundary semantic sets with the first package job/store that consumes them, keeping
each package's raw IDs inside its own boundary. Add project and package check-only subtraction with
the first check-only unit consumer. Do not stage either as stored or dropped future-only data.

### R4E: graph edges, waves and jobs

Insert module dependency edges directly by `ModuleId` while resolving structural references. Do not create canonical path-pair facts and remap them later.

After edge construction:

- sort and deduplicate adjacency
- freeze provider and consumer vectors
- compute indegrees in dense arrays
- produce deterministic waves in `ModuleId` order
- retain authored edge locations in a separate sorted side table
- keep project, Core, Builder and dependency package graphs separate

Produce one `ModuleCompilationJob` per selected normal, support or project-facade node. The job contains IDs and immutable store references, not cloned token streams or source text.

#### R4E1: frozen project adjacency production vertical

Replace the current long-lived construction `BTreeSet` adjacency with an explicit completion
transition. Edge insertion retains construction-time deduplication; completion converts provider
and consumer adjacency to sorted `Vec<ModuleId>` storage and keeps authored edge locations in the
separate deterministic side table. The current production compile-wave path must consume only the
frozen adjacency. Mutation after completion or scheduling before completion is an internal
`CompilerError`.

Do not create package graphs, prepared stores or `ModuleCompilationJob` in this slice. They would
have no current production consumer. Preserve the existing temporary normal-entry wave payload
until the atomic scheduler cutover, but do not add an adapter or another wave representation.

R4E1 is complete at `7975981fc`. One graph lifecycle enum now owns both adjacency directions:
construction uses sorted sets for idempotent insertion, completion consumes them into sorted dense
vectors and production wave scheduling accepts only the frozen state. The first authored edge
location remains a separate side-table fact. Focused tests cover frozen ordering, duplicate edges,
cycles, invalid lifecycle use and the production completion boundary. No package graph, store or
job was introduced.

#### R4E2: package graphs and compilation jobs

Add separate frozen Core, Builder and dependency package graphs with the first canonical package
consumers. Produce `ModuleCompilationJob` only when the corresponding prepared stores and
canonical scheduler consume it. Complete R4D2 package/check-only sets in those same consumer-backed
slices so no raw boundary ID or future-only job is stored and dropped.

### R4F: remove duplicate discovery

Delete production use of the following atomically with the R5K canonical scheduler cutover:

- per-entry import BFS over filesystem paths
- `ProviderFreeProjectInventory`
- provider-free versus provider-capable replay
- per-entry `ScannedImportSource` caches
- path-pair `LocalStructuralDependencyFact`
- repeated `PreparedSourceInput` ownership per entry

Delete `reachable_file_discovery.rs` and `import_scanning.rs` when no other real owner remains.

The legacy entry-closure semantic compiler remains isolated until the atomic cutover. It must not
consume the new prepared stores, gain features or acquire an adapter to the canonical path.

Remove `package_folders` and default `/lib` discovery with their production owner. Update or delete tests that assert the obsolete config surface.

### R4 tests and counters

Add focused counters and integration assertions for:

- one source-tree traversal
- one source read
- one tokenization
- one header preparation
- one import-shell resolution
- deterministic source and module IDs
- no filesystem calls during source import resolution
- semantic sets stop at module boundaries
- support visibility and overlap diagnostics
- check-only orphan exclusion
- separate package graphs

Use counters or explicit hooks owned by existing instrumentation. Do not add timing-sensitive tests.

### R4 exit gate

This gate is reached only by the atomic R4F/R5K production cutover, not by an intermediate R4
commit on the integration branch.

- `SourceTreeIndex` is the only directory source inventory
- every selected source has one prepare-once slot
- source imports resolve from frozen namespace data
- semantic source sets and graph edges use IDs
- duplicate scanning paths are deleted
- no new legacy adapter was introduced
- `cargo fmt` and `just validate` pass

Run Review phase 3.

## Phase R5: canonical provider and generated-sidecar integration train

Goal: implement provider binding and generated sidecars together so neither is designed around the legacy entry-closure compiler.

Work may use bounded commits on an integration branch. Do not accept the phase onto the baseline until R5K deletes the old production path.

### R5A: internal module semantic draft

Introduce one internal, non-provider result for a module whose base HIR is validated but generated requests are unresolved.

It may contain:

- `ModuleId` and stable origin
- direct interface seed
- direct project-context provenance seeds produced by their existing semantic owners
- local `TypeEnvironment`
- validated base HIR
- generated requests
- compiler metadata
- incomplete per-function link facts
- diagnostic render context

It must not implement provider lookup and must not be stored in `GraphCompilationOutcome::successful`.

### R5R: explicit module-root preparation roles

Replace entry-path-derived active-root semantics with the graph-owned `ModuleRootRole` before the
provider/generated cutover. This is an independently reviewable prerequisite seam, but its
support/facade behaviour is not production-reachable until R5K schedules those roles directly.

- normal active roots retain dormant runtime work, fragments and compiler-synthesised `start`
- support and project-facade roots are API-only, diagnose root runtime activity and fragments and
  never synthesize `start`
- ordinary non-root sources and imported normal roots retain their existing declaration-only and
  dormant-root behaviour
- the final preparation API carries one explicit root-preparation policy; delete the old boolean
  inference path instead of wrapping it

Focused tests cover all root roles, invalid API-root activity and exact absence of API-root HIR.
This slice introduces no provider store, scheduler job, imported-interface table or dormant future
data. Production acceptance remains part of the R5B-R5K/R5P3 boundary: the legacy entry-closure
scheduler is not extended to imitate API-only jobs.

## Phase R5P: live link and backend prerequisites

Goal: land the final-owner link and backend contracts needed for provider interfaces to replace
donor-root compilation. R5P1-R5P2 are complete. R5P3 is part of the expanded atomic
provider/generated-sidecar acceptance boundary because its stable call producer and borrow
consumer require completed provider identities and summaries. These slices may not introduce a
donor fallback, process-global registry or compatibility API.

### R5P1: success-only backend and entry boundary

Replace `BackendBuilder::build_backend(Vec<Module>, ...)` with one `ProjectCompilation` input.
The initial live shape owns the successful compiled modules and explicit `EntryAssembly` records
selected from their root activity. Do not add unused graph, generated-sidecar, package-assembly or
lifetime fields before their production consumers exist.

The build system constructs entries after all current frontend work succeeds. HTML consumes the
entry records for route compilation, runtime planning and asset planning instead of filtering the
flat module vector itself. Single-file and directory builds use the same boundary. Delete the old
backend signature and update every test backend directly.

Tests protect:

- explicit entry selection excludes modules with no HTML artefact activity
- every selected entry refers to exactly one module
- the HTML builder receives no partial or diagnosed frontend result
- existing HTML output and diagnostics remain unchanged

### R5P2: per-function facts and build-owned reachability

Replace start-filtered module finalisation with deterministic per-function link-fact records for
the facts the current HIR and binding owners can already produce. Build-owned entry planning walks
those records from the selected `start` root and supplies exact reachable functions, binding calls,
runtime features, paths and assets to HTML validation and lowering.

Delete backend calls to `collect_reachability_from_start` and remove module-wide external-import
unions as linking authority. Do not add generated, module-private or cross-module variants until
their stable identities and live producers land.

R5P2a is complete at `6328df774`. HIR now retains deterministic direct records for every base
function, module finalisation records complete candidates without selecting `start` and build-owned
entry assembly derives exact reachable external-import unions. HTML runtime emission and glue
consume those entry unions. R5P2b completes the slice by carrying reachable functions and runtime
features through the same assembly into HTML validation and JS/Wasm selection, then deletes their
backend-owned `start` rediscovery.

### R5P3: stable cross-module backend handoff

Extend the live call-target and entry-link inputs with stable `OriginFunctionId` cross-module
targets. JavaScript resolves selected cross-module calls through the project compilation's module
and function indexes. The current structured Wasm cross-module rejection remains the explicit
contract until a separately accepted Wasm linking design lands.

This slice must have a live producer and consumer together. It may not resolve through copied
donor HIR, rendered names, a process-global registry or a fallback to donor-local `FunctionId`.
R5P3 lands in the same acceptance boundary as provider binding, generated sidecars and donor-path
deletion so no stable cross-module call can enter a backend that cannot resolve it.

### R5B: completed provider store

Create one completed provider store indexed by package graph and `ModuleId`.

A slot is one of:

```text
Unavailable
Successful(CompiledModuleArtifactId)
Diagnosed
Blocked
```

The successful slot points into immutable artefact storage. Do not clone a full interface into every consumer.

Consumer jobs receive:

- their resolved namespace
- the IDs of completed required providers
- immutable access to provider interfaces
- binding package interfaces
- synthetic compile-time interfaces

A diagnosed provider exposes no interface.

R5B lands inside the expanded atomic cutover. At each dependency-wave boundary, successful final
`PublicSemanticInterface` values publish before the next wave compiles. Diagnosed and blocked graph
outcomes land in the same boundary; incomplete `LocalPublicInterface` values never become provider
slots.

### R5C: non-generic vertical provider path

Implement the R3 reviewed contract for:

- imported free functions
- imported nominal types
- imported folded constants
- stable cross-module source calls
- provider call-summary lookup

Use explicit call targets:

```rust
pub enum SourceCallTarget {
    Local(FunctionId),
    CrossModule(OriginFunctionId),
}

pub enum HirCallTarget {
    Source(SourceCallTarget),
    Binding(ExternalFunctionId),
}
```

Borrow transfer matches the target enum through one context struct. Do not add dynamic dispatch.

Tests:

- provider compiled once for two consumers
- one primary provider-consumer integration case covering a named free-function call, nominal
  construction and field access and folded-constant use without donor syntax
- canonical type equality through distinct consumer-local `TypeId`s and complete nominal member
  projection
- repeated projection of one canonical identity returns the same consumer-local `TypeId`, and the
  reverse origin lookup returns that exact canonical identity
- declared shared, mutable and reactive access imported correctly for AST typing and resolved from
  the complete summary for borrow effects
- exact `OriginFunctionId` retained by cross-module HIR calls
- cross-module return alias and transfer facts consumed without foreign HIR
- same-origin visible-name coexistence and different-origin collision diagnostics under one local
  spelling
- private symbol rejection is a source diagnostic
- an export binding whose exporting-module identity differs from its successful artefact is
  `CompilerError`
- missing provider declaration, missing provider summary and other inconsistent successful
  provider slots are `CompilerError`

Pause for Review phase 2 findings if the binding contract changes materially.

R5C lands inside the expanded atomic cutover with R5P3 and R5D-R5I. Its complete inverse binder,
stable HIR targets and borrow-summary resolver become production only when the generated worklist,
final provider surfaces, project backend index and donor deletion are complete.

### R5D: generic materialisation design checkpoint

No implementation begins until the complete immutable generic artefact is defined.

It must answer:

- how validated body syntax is retained without AST or TIR
- how generic parameter ownership is represented
- how provider visibility and imported identities are retained
- how canonical types become generated-local types
- how required evidence is supplied
- how source locations and strings remain self-contained or remappable
- how private declaring-module helper calls are identified
- how nested generated requests are emitted
- how project-context provenance is retained
- how dependency artefacts remain immutable

Define a module-private executable identity distinct from public `OriginFunctionId`. It may change when private implementation identity changes. It must not use donor-local `FunctionId` outside its artefact.

Stop if the artefact is still described as "body now, context later".

R5D is accepted as the following complete contract. This checkpoint intentionally lands before
R5A-R5C implementation so their internal draft and provider boundaries cannot preserve the
current body-only store or consumer-local materialisation by accident.

The contract checkpoint is committed at `9d1639745`.

#### Complete artefact ownership

One successful declaring module owns an immutable `ModuleMaterialisationContext`. It is compiler
metadata beside the base executable and public interface, not part of either lane. It contains one
deterministic `GenericTemplateArtefact` per exported generic callable and the minimal immutable
private semantic tables those templates can reference. The current
`ValidatedGenericTemplateStore`, its `allow(dead_code)` accessors and every discard-before-remap
call are replaced, not wrapped.

Each template artefact contains:

- the public `OriginFunctionId` of the generic declaration
- ordered `ExportedGenericParameterIdentity` slots and canonical trait bounds from the public
  declaration contract
- the already-tokenized, body-only retained syntax produced by the declaring module, with original
  portable source locations and no AST or TIR
- a canonical signature using `CanonicalTypeIdentity`, declared access and owned folded defaults
- one closed binding environment for the declaration file, expressed as owned local spellings to
  stable source, binding, builtin, namespace, receiver and module-private targets
- direct project-context provenance and the declaring artefact compatibility identity

The retained syntax may be parsed again only by generic materialisation. Tokenization, declaration
shell parsing, interface binding and visibility construction are never repeated. A materialiser
does not reopen source, donor AST, donor HIR or a mutable donor `TypeEnvironment`.

#### Stable and private identities

Generic parameter ownership uses the existing ordered `ExportedGenericParameterIdentity`; no
`GenericParameterId`, `GenericParameterListId` or donor `TypeId` crosses the artefact boundary.
Request evidence uses ordered `CanonicalEvidenceIdentity` values selected at the call site. Core
evidence remains compiler-owned. Source evidence resolves through immutable public or
module-private evidence records that include the exact requirement-to-executable target mapping.

Private callable targets use a closed `ModulePrivateExecutableIdentity` consisting conceptually of
the declaring stable module origin, portable declaring-source identity, declaration category,
owned declaration name and optional private receiver identity. It carries no source position,
`InternedPath`, `StringId` or donor `FunctionId`. Moving or changing private implementation may
change this identity. Every generated sidecar records the exact declaring implementation
compatibility fingerprint, so a private identity is never resolved against another artefact.
Private nominal and trait facts use the same artefact-scoped principle and cannot enter a public
interface or request identity.

#### Generated-local compilation

The materialiser creates a fresh generated-local `TypeEnvironment`, seeds compiler builtins and
interns canonical concrete types through one canonical-to-local table. It maps ordered exported
generic parameter identities to the request's ordered canonical concrete arguments. Private
declaring-module type blueprints are projected from the immutable materialisation context into the
same generated-local environment. The context never borrows or mutates the requester or declaring
module's local environment.

The closed binding environment supplies stable source and binding targets. Public provider facts
come from immutable completed interfaces. Private helper and evidence targets resolve only through
the exact declaring module artefact. Local spellings exist for parsing and diagnostics only and
never define request, call or type identity.

#### Strings, diagnostics, provenance and fixed point

Before publication, the declaring module's template syntax, binding spellings and source locations
are remapped into the build-lifetime diagnostic identity context in deterministic module order.
No stale worker-local `StringId`, absolute path or donor-local source handle survives. Persistent
artefacts later store portable logical source identities plus self-contained strings or a
remappable table.

A request key contains exactly the generic declaration origin, ordered canonical concrete type
identities and ordered canonical evidence identities. Call location, requester and queue order are
diagnostic/worklist context, not identity. Materialisation emits concrete validated HIR, borrow
facts, real lifetime facts when that owner exists, summaries, link facts, provenance and zero or
more stable nested requests. The build-owned worklist inserts nested requests into the same
deterministic deduplication map until a fixed point. Base and dependency artefacts remain immutable.

A diagnosed request publishes no partial sidecar. Missing bindings, private targets, remaps,
canonical type projections or evidence in an otherwise successful artefact are `CompilerError`.
Source-dependent inference, evidence or materialised-body failures remain structured diagnostics
with the request call site primary and the generic declaration as supporting context.

### R5E: complete generic template artefacts

Replace the current incomplete validated-template store with one deterministic store keyed by public generic declaration origin.

Rules:

- one complete artefact per exported generic callable
- no `InternedPath` identity beyond extraction
- no `GenericParameterId`, `TypeId`, `StringId` or local evidence ID crosses without a defined remap/context owner
- no raw `Ast`, TIR store or mutable environment
- private helper targets use module-private executable identities
- values are `Send`
- the artefact is consumed by the worklist before backend handoff

Delete the old body-only store and its explicit discard-before-remap path.

### R5F: stable generated requests

Define request identity from:

```text
generic declaration origin
+ ordered canonical concrete type identities
+ ordered required evidence identities
```

AST call inference emits the stable request and a generated call target. It does not materialise the function.

This slice adds `SourceCallTarget::Generated(GeneratedFunctionIdentity)` to the local and
cross-module source-target enum introduced by R5C. Do not add the variant or its resolver arm
before the stable generated identity exists here.

Rules:

- aliases do not change identity
- local `TypeId` allocation does not change identity
- request order does not change identity
- source location is diagnostic context, not identity
- duplicate requests retain deterministic primary diagnostic context
- base AST and HIR remain immutable after emission

### R5G: deterministic worklist

The build system owns one worklist per project or package compilation boundary.

Use:

- dense `GeneratedRequestId`
- a vector of request records
- one construction-time deduplication map
- deterministic queue order by stable request identity
- explicit requester sets
- explicit dependency edges between generated requests

A generated body may enqueue more requests. Continue to a fixed point.

Outcomes:

```text
Successful sidecar
Diagnosed request
Blocked requester
Infrastructure failure
```

A diagnosed request exposes no partial sidecar.

### R5H: generated compilation

For each accepted request:

- build generated-local type context
- materialise concrete typed body
- lower and validate HIR
- borrow-validate
- produce real local lifetime facts and summaries when the memory-analysis owner exists
- produce concrete call summaries
- produce per-function link facts
- retain implementation, runtime and compatibility fingerprint inputs

Do not borrow or mutate the requesting module's `TypeEnvironment`.

Generated calls to private provider helpers use the reviewed module-private target, not public interface identity and not foreign local `FunctionId`.

### R5I: finalize requester modules

After the generated fixed point:

- resolve every generated target
- make generated call summaries available to base-module borrow validation
- run base borrow validation
- run real local lifetime analysis when available
- finalize `LocalPublicInterface`, including direct provenance and real lifetime summaries when
  their owners exist
- return one internal post-generation module result to R5J; it is not a successful provider
  artefact and cannot enter the provider store

A module with an unresolved or diagnosed required generated request is blocked or diagnosed according to the owning error. It is never successful with pending state.

### R5J: complete provider surfaces

Extend binding from the vertical subset to:

- aliases
- defaults
- choices
- receiver surfaces
- traits and requirements
- incompatibilities
- reusable evidence
- generic template contracts
- public re-exports
- re-export and project-context provenance closure

Re-exports add bindings and preserve the donor's closed stable export target. Source targets carry
`OriginDeclarationId`. Binding-backed targets carry owned canonical binding package identity,
structured symbol path and declaration category, resolve through the supplied binding-package
interfaces and never retain build-local `ExternalSymbolId` values.

The exporting interface computes the recursive fixed-point closure of every
`PublicSemanticInterface`-owned fact reachable from its export bindings: declarations and nested
canonical facts, concrete call summaries, real lifetime/outlives summaries when their owner
exists, generic contracts, defaults and folded values, receiver surfaces, traits, reusable
evidence, external-boundary classifications and project-context provenance. It retains needed
closure records even when the facade exposes no public binding for their names. It excludes
`ModuleLinkFacts`, source and binding call edges, helpers, assets and backend/lowering metadata.
Interface validation rejects a missing closure record as `CompilerError`. Consumers do not reopen
transitive source providers.

After closing and validating this surface, R5J performs the sole provider re-export join and
constructs the final `PublicSemanticInterface` and successful module artefact. R5I must not create
an incomplete final interface or artefact.

Add a provider -> facade -> consumer integration case where a re-exported callable reaches nested
donor nominal shapes that the facade doesn't separately bind. The consumer must succeed from the
facade interface alone.

Retain binding-backed re-export coverage, including existing source-package external function and
constant re-exports, through this canonical path. Add direct and transitive project-context
provenance rejection across re-export closure.

### R5K: canonical graph scheduler and cutover

R5B-R5K and R5P3 form one production acceptance boundary. Internal integration commits may divide
the implementation for review, but no intermediate commit may expose a second callable provider
architecture, publish incomplete interfaces or delete donor compilation before generated sidecars
preserve current cross-module generic support.

Compile every selected graph role:

- normal modules
- scoped support modules
- project package facade
- source-backed Core packages
- source-backed Builder packages
- dependency package graphs

For each wave:

1. ensure required provider slots succeeded
2. mark blocked consumers without semantic compilation
3. compile independent ready jobs in parallel
4. complete generated requests needed by those jobs
5. finalize successful artefacts
6. merge string-table deltas in `ModuleId` order
7. publish interfaces for the next wave
8. retain one diagnostic set per diagnosed module

Build:

```rust
pub struct GraphCompilationOutcome {
    pub successful: Vec<CompiledModuleArtifact>,
    pub diagnosed: Vec<ModuleDiagnostics>,
    pub blocked: Vec<BlockedModule>,
}
```

Cut production over in the same accepted phase.

Delete:

- `DiscoveredModule`
- `ModuleEntryCompileWaves`
- entry-closure semantic compilation
- donor header/body copying
- public-surface fallback by filesystem walk
- old source binding from combined headers
- consumer-local generic materialisation
- source `@./` and entry-root fallback
- production source-import use of `ProjectPathResolver`
- compatibility code added during the integration train

The cutover is complete only when non-generic and generic cross-module calls both resolve through
stable identities. Temporary rejection of currently supported cross-module generics is not part of
the accepted migration.

No feature flag or legacy fallback remains.

### R5 tests

Required end-to-end cases:

- one provider compiled once for multiple entries
- one provider failure diagnosed once
- blocked consumers emit no secondary name/type cascades
- independent graph branches continue
- imported functions, types, constants, defaults, aliases, traits, evidence, receivers and re-exports use interfaces
- cross-module borrow and return-alias effects
- same generated request deduplicated across entries
- nested generated fixed point
- generated private helper call
- generated diagnosis blocks only requesters
- base module artefacts remain unchanged when another consumer requests a sidecar
- support and facade roots have no `start`
- deterministic results under varied Rayon completion order
- no source read, tokenization, preparation or module compile count exceeds one

### R5 exit gate

- completed provider interfaces are the only source-module binding input
- generated functions live only in sidecars
- one canonical scheduler is the only directory-project production compiler
- all legacy entry-closure, donor-copy and fallback paths are deleted
- no successful artefact contains pending state
- `cargo fmt` and `just validate` pass

Run Review phases 4 and 5 before proceeding.

## Phase R6: per-function link facts and assemblies

Goal: move runtime reachability, root activation and package selection entirely after semantic compilation.

### R6A: complete per-function link-fact records

R5P2 establishes the live base-function owner before provider cutover. Complete that owner after
R5 by adding the generated and module-private identities that do not exist before the generated
sidecar train:

Record for every base and generated function:

- local, cross-module, module-private and generated source calls
- binding-backed calls
- helper and capability families
- reactive features
- numeric, cast, map and target-gated operations
- runtime paths and assets
- generated request references
- project-context provenance

Store facts in deterministic function identity order. Module-wide unions may be derived caches only.

Delete the complete `ExternalPackageRegistry` from each module artefact. Link facts refer to stable binding identities.

### R6B: verify removal of `start`-filtered module finalization

R5P2 removes the production owner. After the canonical scheduler cutover, verify no replacement
path filters module finalisation by `start` and delete any scheduler-era residue:

Delete `collect_reachability_from_start` as a module finalization authority.

A normal module artefact retains dormant `start` and its per-function facts. Support and facade roots have no `start`.

Unsupported private code remains semantically compiled. Target validation later checks only supplied reachable roots.

### R6C: complete entry assemblies

R5P1 establishes build-owned entry selection and R5P2 adds base-function reachable unions.
Complete the same records with generated sidecars, canonical module identities and final provider
link facts. Do not create a second assembly type.

Build `EntryAssembly` from already successful artefacts.

It selects:

- one normal module's dormant `start`
- root runtime work
- compile-time and runtime fragments
- entry-local settings
- exact reachable source/generated/binding functions
- runtime and asset union

Imported modules never activate their root work.

Assembly cannot invoke tokenization, binding, AST, HIR, borrow or lifetime analysis.

### R6D: project package assembly

Build `ProjectPackageAssembly` over:

- the compiled project facade
- selected descendant public interfaces
- reachable generated sidecars
- permitted runtime requirements

It never bypasses `export:`.

Propagate project-context provenance through direct facts and reachable source/generated call edges. Reject direct or transitive prohibited project context.

### R6E: lifetime and target roots

Link planning supplies:

- reachable function roots
- builder lifecycle roots
- generated sidecar roots
- external package exports

Use these to instantiate complete lifetime topology through the memory-analysis owner before target planning. Do not implement a second lifetime solver in the build system.

### R6F: success-only project compilation

Construct:

```rust
pub struct ProjectCompilation {
    pub structure: ProjectModuleGraph,
    pub project_globals: ProjectGlobalsInterface,
    pub modules: Vec<CompiledModuleArtifact>,
    pub generated: Vec<GeneratedFunctionSidecar>,
    pub entries: Vec<EntryAssembly>,
    pub package_facade: Option<ProjectPackageAssembly>,
}
```

Only complete required successes enter this payload. A project builder never receives diagnosed or blocked required work.

### R6 exit gate

- per-function facts are the linking authority
- module compilation performs no entry reachability filtering
- assemblies perform no semantic compilation
- project-context validation uses stable provenance
- complete lifetime topology is supplied by the correct owner
- `cargo fmt` and `just validate` pass

Run the link portion of Review phase 6.

## Phase R7: backend handoff, commands and fingerprints

### R7A: complete the backend API cutover

R5P1 replaces the flat `BackendBuilder::build_backend(Vec<Module>, ...)` input with
`ProjectCompilation`. Complete that same payload after R6 with:

Backends receive:

- explicit selected functions
- stable call targets
- paired type environments
- borrow and lifetime facts
- link plans
- import and capability plans
- entry/package plans

They do not scan source, rebuild imports, infer generics or choose roots.

Delete any remaining backend loop that treats all compiled modules as entries.

### R7B: complete the HTML builder migration

R5P1 makes HTML consume `EntryAssembly` for entry selection and R5P2 moves base reachability into
the build-owned plan. Complete the same path for:

- route generation
- exact-once `start` activation
- fragment interleaving
- external JavaScript glue
- tracked assets
- JavaScript/Wasm partition inputs

Keep physical Wasm partition and Component Model work in its downstream plan.

Tests:

- HTML-JS output parity
- route and homepage parity
- exact-once root and fragment output
- external runtime and asset deduplication
- duplicate route/output diagnostics
- no backend source discovery

### R7C: `check`

`check` consumes graph outcomes:

- compile every selected project module
- compile check-only orphan units
- retain successful independent artefacts
- report diagnosed modules once
- report blocked modules without cascades
- run real link and target validation roots
- stop before backend lowering and output writing

Delete the all-or-error `Vec<Module>` check path.

### R7D: dev reuse

Retain immutable in-memory:

- source preparation slots
- successful module artefacts
- package artefacts
- generated sidecars
- graph and namespace data

Invalidation:

- source change reparses only changed source slots
- module rebuild occurs when its semantic source or imported interface changes
- semantic consumers rebuild only when provider public-interface fingerprint changes
- entries relink for implementation, root, runtime, generated, entry-setting or relevant config changes
- diagnostic and output ordering remains deterministic

Do not create a second dev compiler architecture.

### R7E: final fingerprint encoders

Only now implement canonical encoders for the five accepted fingerprints:

- public interface
- implementation
- dormant root activity
- runtime dependency
- documentation

Rules:

- encode final facts, not direct seeds or pending states
- one domain tag and version per fingerprint family
- stable identities and owned values only
- no source locations, warnings, absolute paths or process-local IDs
- sort semantic sets by stable identity
- preserve authored order where order is semantic
- generated request set affects implementation/worklist invalidation, not runtime-dependency content
- choose digest and persistent format separately

Delete any remaining R2 direct-draft encoding API.

### R7F: output ownership

Backends and project builders return output records. The build system owns:

- output-root validation
- conflict diagnostics
- manifests
- skip-unchanged writes
- stale cleanup

Remove `FileKind::NotBuilt` if no tooling owner remains.

### R7 exit gate

- all project builders consume `ProjectCompilation`
- `check` and dev use the canonical graph
- five final fingerprint owners exist
- no duplicate invalidation policy exists
- no backend writes final project files directly
- `cargo fmt` and `just validate` pass

Complete Review phase 6.

## Phase R8: repository migration and final deletion audit

Update:

- module-root-relative import fixtures
- support and project-facade examples
- `moth new` scaffolding
- package and project-structure docs
- compiler educational docs
- progress matrix only where support changed
- generated documentation through the release build

Prune:

- obsolete implementation-shaped unit tests
- dead-code allowances for future consumers that now exist
- stale comments naming old phases or owners
- benchmark cases that do not validate successful compilation
- old package-folder fixtures
- any remaining Beanstalk, `.bst`, `.bd`, `bean` or `BST-*` references in this plan's affected surfaces

Final deletion audit:

- no `DiscoveredModule`
- no `ModuleEntryCompileWaves`
- no entry-closure compilation
- no parallel import scanner
- no configured `package_folders`
- no default `/lib`
- no source `@./`
- no entry-root import fallback
- no donor header/body copying
- no donor-local cross-module type/evidence transport
- no consumer-local generic materialisation
- no foreign HIR borrow lookup
- no module-level `start` reachability authority
- no API-only sentinel `start`
- no flat `Vec<Module>` backend handoff
- no incomplete direct-draft encoder
- no compatibility wrappers around removed paths

## Required end-to-end contracts

The canonical integration suite owns user-visible behaviour. Focused Rust tests own hidden identity, graph, summary and scheduling invariants.

Required contracts:

- shared module compiled once across entries
- shared failure diagnosed once
- independent graph branches continue
- blocked consumers have no cascades
- module-root-relative imports from nested files
- strict child-module facade enforcement
- strict scoped support visibility
- support-scope overlap diagnostics
- project facade assembly
- separate Builder, Core and dependency package graphs
- stable identities under ordinary source-file moves and declaration reordering
- re-export alias changes binding identity without changing declaration origin
- canonical type and evidence identity independent of local allocation
- generated sidecar reuse and fixed point
- cross-module borrow, alias and transfer summaries
- exact per-entry runtime and asset unions
- no source/provider fallback to `@./`
- no API-only `start`
- check-only orphans excluded from artefacts
- deterministic diagnostics and output under parallel completion
- direct and transitive project-context package-export rejection
- consuming-project inputs do not satisfy dependency build-input contracts
- one source read, tokenization and preparation per selected source
- one semantic compilation per physical module
- one materialisation per generated identity

## Test discipline

- Prefer realistic multi-module integration cases over getter-shaped unit tests.
- Use unit tests for pure identity, graph, projection, encoding and impossible-state invariants.
- Keep one primary contract owner per behaviour.
- Do not add public or test-only production accessors.
- Remove tests for deleted APIs rather than preserving wrappers.
- Use exact instrumentation counts for duplicate-work contracts.
- Run `cargo run --quiet -- tests --audit` after fixture metadata changes.
- Benchmark checks are sanity gates. They do not replace correctness assertions that compilation completed.

## Validation

Every parent-accepted code slice requires:

```bash
cargo fmt
just validate
```

Also perform the manual architecture audit from `validation.mtf` whenever a slice changes:

- stage ownership
- source discovery
- provider binding
- AST or HIR boundaries
- semantic identities
- borrow or lifetime summary handoff
- graph scheduling
- backend handoff

A focused command is iteration evidence only.

Documentation-only final slices use:

```bash
moth build docs --release
```

Do not claim full compiler validation for a documentation-only gate.

## Final acceptance

Before marking this plan complete, verify:

- one physical module is compiled once per project or package boundary
- one filesystem inventory and one prepare-once source store exist
- every source consumer binds completed immutable provider interfaces
- every successful module artefact has a complete interface, executable lane, per-function link facts, metadata and five fingerprints
- diagnosed modules expose no partial interface
- generated functions live only in sidecars
- borrow validation uses local, provider, generated or binding summaries without foreign HIR
- lifetime topology is validated through the memory-analysis owner
- entry and package assembly never trigger semantic compilation
- support and project-facade roots are API-only
- source imports are module-root-relative and topology-checked
- project, Builder, Core and dependency source graphs remain separate
- backends receive success-only explicit project and link plans
- source, tests, docs, progress matrix and roadmap agree
- the HTML-Wasm plan can proceed without redesigning module identity, provider binding or linking

## Deliberately deferred work

- persistent module, package and generated artefact serialization
- on-disk cache layout, eviction and migration
- dependency declaration syntax and local path dependencies
- package registries, remote fetching, versions and lockfiles
- precompiled dependency caches
- direct normal-sibling imports
- cross-entry browser chunking
- physical Wasm module partition and Component Model integration
- cross-build generated-instance caches
