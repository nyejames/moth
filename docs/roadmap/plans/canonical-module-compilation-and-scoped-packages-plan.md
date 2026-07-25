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
STATUS: blocked — codex-cli worker order remains primary delegation path; next slice requires codex-cli
CURRENT_SLICE: R2 audit correction 1 — exported generic receiver stable-parameter projection — completed parent-direct
LAST_ACCEPTED_COMMIT: 194cf795e (R2j)
WORKTREE: main at 194cf795e with a committed 12-file correction-1 patch; preserve all worker artefacts unless they interfere with the next slice
REQUIRED_RELOADS: startup files, this plan, the committed correction diff, generic-template metadata extraction, generic-parameter origin registration and frontend orchestration coverage
RELEVANT_CONTEXT_NOW:
- docs: compiler-design-overview.md and locked decision 10 require one validated generic body artefact per stable exported generic callable origin without path/name identity crossing the metadata boundary
- code: the committed correction-1 patch threads generic_function_templates as FxHashMap<InternedPath, GenericFunctionTemplate> through PublicInterfaceDraftBuilder into build_defined_public_type_surface; register_receiver_method_generic_parameter_aliases aliases receiver-method local GenericParameterId values to the receiver nominal's stable ExportedGenericParameterIdentity, verified by authored-name alignment; the dedicated test uses a genuine generic nominal Box<A> and passes a matching GenericFunctionTemplate; full just validate passes
ACCEPTANCE_CRITERIA: — all met by Correction 1
- one exact transient path-to-OriginFunctionId relationship joins public generic free functions and receiver methods to validated body payloads before the metadata boundary ✓
- same-named generic receiver methods on distinct receivers retain distinct artefacts; private templates remain excluded only after exact path joining ✓
- missing, duplicate, path-mismatched or generic/non-generic public joins fail through CompilerError; artefacts sort by full stable origin ✓
- receiver-local aligned generic parameters resolve to nominal-owned stable identities without making the method a generic declaration owner; production orchestration retains the exact receiver artefact ✓
- retained metadata contains no InternedPath identity and no R3 materialization/worklist behavior ✓
VALIDATION_STATE:
- just validate passes: clippy (native/linux/windows), 3757 unit tests, 1793 integration tests, docs build, bench-check
- the failing GenericParameterId(15) probe from the earlier partial state is resolved by the aliasing functions
DOCS_IMPACT: active plan only; progress matrix unchanged because this slice retains internal direct-interface facts for already-supported behavior
BLOCKERS_OR_OPEN_DECISIONS: this slice is complete; codex-cli usage limit no longer blocks Correction 1. Audit cycle 1 retains four later findings: declaration parameter access, finite-float negative zero, origin-to-FunctionId injectivity and receiver-catalog duplication. These remain deferred to the next appropriate slice.
DELEGATION_DECISION: codex-cli for subsequent implementation, correction and audit slices
NEXT_WORKER_ORDER: tbd — next audit slice or R2k/R3 slice as determined by plan owner
STOP_REASON: Correction 1 completed parent-direct; ready for next slice assignment
NEXT_RESUME_ACTION: assign the next audit finding or R2k/R3 slice through the preferred worker path
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

- `SourceTreeIndex` owns physical source inventory, module roots, nearest ownership and canonical source IDs.
- A prepared-source store owns source text, tokenization and retained syntax for each selected source.
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

Change:

- add declared access to public function and receiver parameter slots
- derive it from the resolved source signature before HIR
- keep mutation, transfer, alias and reactive effects in analysis summaries
- make generic template contracts carry declared access without needing a concrete generated function

Delete:

- any code that treats borrow-observed access as the only public signature access authority

Tests:

- shared, mutable and reactive free-function parameters
- shared and mutable receiver access
- generic free functions and aligned generic receiver methods
- signature access remains stable before borrow validation

Stop if the source signature has more than one competing access owner.

### R2C2: replace temporal callable states

Change:

- replace `PublicCallSummaryState`
- model concrete local callables and generic template declarations as distinct semantic categories
- require a complete summary for each non-generic concrete exported function after borrow validation
- leave concrete generated summaries exclusively on generated sidecars

Delete:

- `PendingLocal`
- `PendingGenerated`
- encoder branches that treat temporal states as semantic bytes

Tests:

- a completed direct concrete interface cannot omit a summary
- a generic declaration cannot carry a concrete base summary
- malformed category/summary combinations are unrepresentable or fail at one construction boundary

Stop if generic template effects cannot be distinguished from concrete generated effects. Record the unresolved contract for Review phase 1.

### R2C3: one receiver and callable seed owner

Create one transient callable seed table during AST environment finalization. Each entry carries only the facts required before stable projection:

- exact donor-local declaration path
- stable public origin when public
- stable receiver origin when applicable
- resolved signature reference or index
- generic-template classification
- nominal-owned generic parameter aliases where required

Consumers:

- direct export projection
- declaration record projection
- HIR origin seeding
- generic-template extraction

Delete one of the current duplicate receiver authorities from the public projection path. Evidence projection must consume completed declaration records rather than iterate the receiver catalog.

Tests:

- same-named methods on different receivers
- methods before and after receiver declarations
- generic nominal receiver aliases
- private methods on public receivers
- duplicate path and duplicate origin rejection

Stop if any consumer needs to reconstruct a receiver origin.

### R2C4: move public projection out of `Ast`

Introduce a typed AST result equivalent to:

```rust
pub struct AstBuildResult {
    pub ast: Ast,
    pub direct_interface_input: DirectInterfaceProjectionInput,
    pub generic_template_input: GenericTemplateProjectionInput,
}
```

Rules:

- `Ast` contains only state HIR consumes
- HIR never receives public projection roots, trait/evidence environments or generic template maps
- no `Rc` or `RefCell` crosses into module artefacts
- the side result has a closed field list and one consumer

Delete:

- `Ast::public_interface_projection_input`
- `Ast::generic_function_templates`
- broad take-before-HIR comments and fixtures that preserve this temporary shape

Tests:

- production AST construction returns all three outputs
- HIR test helpers build only executable AST state
- no public projection fact remains reachable from completed HIR input

### R2C5: direct declaration-oriented projection

Split `public_interface_draft.rs` into focused modules. The recommended structure is:

```text
src/compiler_frontend/public_interface/
├── mod.rs
├── model.rs
├── direct_projection.rs
├── receiver_projection.rs
├── trait_projection.rs
├── evidence_projection.rs
├── local_finalization.rs
└── tests/
```

Build one declaration record table directly from stable export/callable seeds. Category helpers project leaf values into that table. Do not first build several complete aggregate vectors and then index and rejoin all of them.

Rules:

- one record per direct origin
- separate export bindings
- receiver methods attached once
- evidence consumes the completed receiver surface
- construction-time maps are dropped before the seed boundary
- no file should become a second orchestration monolith
- production functions should normally stay below the style-guide size targets

Delete aggregate `DefinedPublic*` containers once their projection logic has moved. Leaf types may remain when they have one clear owner.

Tests should cover the final record contract rather than each deleted intermediate getter.

### R2C6: concrete summary join and HIR injectivity

Change:

- validate both directions of the stable origin and local `FunctionId` relationship
- reject two origins mapped to one local function
- reject one origin mapped to two local functions
- retain concrete call summaries in one stable-origin table
- finalize the local interface from that table without mutating declaration variants

Borrow validation remains read-only over HIR. No foreign HIR lookup is introduced.

Tests:

- missing, duplicate and wrong-category origin mappings
- private functions and `start` remain excluded from public summary tables
- concrete free and receiver functions join once
- deterministic ordering does not depend on `FxHashMap` iteration

### R2C7: folded Float decision and encoder rollback

Before code changes, perform a focused semantic review of negative zero across:

- constant equality
- arithmetic and checked failure behaviour
- casts
- external value boundaries
- canonical type/value identity
- formatting

Formatting `-0.0` as `0` does not by itself prove that signed zero is globally unobservable.

Decision rule:

- if the language authority explicitly makes both signs semantically identical, normalize at folded-value construction and document it
- otherwise preserve exact finite IEEE bits and normalize only at the formatting boundary

Do not guess. Stop until the decision is written into `docs/language-overview.md` when necessary.

Remove or demote the current direct-draft byte encoder:

- do not claim incomplete direct seed bytes are the public-interface fingerprint input
- remove encoder-only getters and dead-code allowances
- recreate canonical encoding only after the final `PublicSemanticInterface` and all five fingerprint fact sets exist
- a test-only deterministic ordering helper may remain only when it is small and has no production API

### R2C exit gate

- all four retained audit findings are resolved
- no durable interface type has pending states
- declared access exists before borrow analysis
- one receiver/callable seed owner exists
- public projection is outside executable `Ast`
- direct interface construction is declaration-oriented
- HIR origin mapping is injective
- signed-zero semantics are explicit
- the incomplete encoder no longer defines future fingerprint policy
- `cargo fmt` and `just validate` pass

Run Review phase 1 before proceeding.

## Phase R3: prove the provider consumer contract

Goal: validate the interface against one real consumer before expanding more producer data.

This phase is read-only by default. It produces a precise binding contract. It does not add dormant production scaffolding.

### R3A: static provider-consumer trace

Trace one directly exported:

- non-generic free function
- nominal type
- folded constant

from provider preparation through the current direct interface and into a hypothetical consumer.

Record the exact required operations:

- provider lookup by `ModuleId`
- export binding lookup by public name
- canonical type projection into the consumer `TypeEnvironment`
- folded value import
- declared parameter access import
- stable cross-module function target creation
- concrete call-summary lookup
- file-local visibility and collision insertion

List every current field that is unnecessary and every missing field.

### R3B: bounded disposable spike

Use a disposable branch only when R3A cannot settle the contract.

Limits:

- one provider module
- one consumer module
- one function, one type and one constant
- no traits, receiver methods, re-exports, generics, packages or backend changes
- no production feature flag
- no compatibility adapter
- no accepted dead code
- maximum one review cycle

The spike must prove that the consumer never opens provider headers, AST, HIR or private source.

Do not merge the spike merely because tests pass. Its durable output is the reviewed contract.

### R3C: binding contract decision

Before R4 begins, lock:

- final lookup keys
- canonical-to-local type interning owner
- provider interface storage owner
- imported visibility record shape
- cross-module call-summary resolver inputs
- diagnostic ownership for missing or private symbols
- re-export record requirements
- string ownership/remapping requirements

Update this plan only when the reviewed contract changes implementation order.

Run Review phase 2.

## Phase R4: eliminate duplicated source discovery

Goal: evolve the existing `SourceTreeIndex` and `ProjectModuleGraph` into the complete Stage 0 data path without adding a second index.

### R4A: central source IDs

Evolve `SourceTreeIndex` in place.

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

### R4B: prepare-once source store

Add one build-boundary `PreparedSourceStore` indexed by `SourceId`.

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

### R4E: graph edges and waves

Insert module dependency edges directly by `ModuleId` while resolving structural references. Do not create canonical path-pair facts and remap them later.

After edge construction:

- sort and deduplicate adjacency
- freeze provider and consumer vectors
- compute indegrees in dense arrays
- produce deterministic waves in `ModuleId` order
- retain authored edge locations in a separate sorted side table
- keep project, Core, Builder and dependency package graphs separate

Produce one `ModuleCompilationJob` per selected normal, support or project-facade node. The job contains IDs and immutable store references, not cloned token streams or source text.

### R4F: remove duplicate discovery

Delete production use of:

- per-entry import BFS over filesystem paths
- `ProviderFreeProjectInventory`
- provider-free versus provider-capable replay
- per-entry `ScannedImportSource` caches
- path-pair `LocalStructuralDependencyFact`
- repeated `PreparedSourceInput` ownership per entry

Delete `reachable_file_discovery.rs` and `import_scanning.rs` when no other real owner remains.

The legacy entry-closure semantic compiler may remain only until the canonical cutover phase. Do not introduce a new adapter type for it. It may consume the new prepared store through its existing boundary for at most one integration phase and may not gain features.

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
- local `TypeEnvironment`
- validated base HIR
- generated requests
- compiler metadata
- incomplete per-function link facts
- diagnostic render context

It must not implement provider lookup and must not be stored in `GraphCompilationOutcome::successful`.

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
    Generated(GeneratedFunctionIdentity),
}

pub enum HirCallTarget {
    Source(SourceCallTarget),
    Binding(ExternalFunctionId),
}
```

Borrow transfer matches the target enum through one context struct. Do not add dynamic dispatch.

Tests:

- provider compiled once for two consumers
- canonical type equality through distinct consumer-local `TypeId`s
- declared mutable access imported correctly
- cross-module return alias and transfer facts consumed without foreign HIR
- private symbol rejection is a source diagnostic
- missing provider summary is `CompilerError`

Pause for Review phase 2 findings if the binding contract changes materially.

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
- finalize `LocalPublicInterface`
- join provider re-exports
- construct `PublicSemanticInterface`
- construct the successful module artefact

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
- provenance

Re-exports add bindings and preserve donor origin. The exporting interface remains self-contained for all records its bindings expose. Consumers do not reopen transitive providers.

### R5K: canonical graph scheduler and cutover

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

### R6A: per-function link-fact records

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

### R6B: remove `start`-filtered module finalization

Delete `collect_reachability_from_start` as a module finalization authority.

A normal module artefact retains dormant `start` and its per-function facts. Support and facade roots have no `start`.

Unsupported private code remains semantically compiled. Target validation later checks only supplied reachable roots.

### R6C: entry assemblies

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

### R7A: backend API cutover

Replace flat `BackendBuilder::build_backend(Vec<Module>, ...)` input with `ProjectCompilation`.

Backends receive:

- explicit selected functions
- stable call targets
- paired type environments
- borrow and lifetime facts
- link plans
- import and capability plans
- entry/package plans

They do not scan source, rebuild imports, infer generics or choose roots.

Delete the flat module backend loop.

### R7B: HTML builder migration

Use `EntryAssembly` for:

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
