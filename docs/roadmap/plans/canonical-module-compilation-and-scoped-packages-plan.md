# Canonical module compilation and scoped packages recovery implementation plan

## Purpose

Continue the canonical-module work from the last accepted implementation checkpoint while correcting the phase order that caused Phase 7 to accumulate producer-side interface fragments without a real provider consumer.

The target remains unchanged:

- one canonical semantic compilation per physical module inside one project or package boundary
- immutable module artefacts with complete public semantic interfaces
- source-provider binding against completed interfaces rather than donor headers or copied bodies
- generated functions in project- or package-owned sidecars
- explicit graph outcomes, entry assemblies, package assemblies and link plans
- strict scoped support packages and module-root-relative imports

This document replaces the previous incremental phase sequence at the same path. The accepted implementation through `4a0cd4e01` is retained. The replacement changes what happens next and when incomplete migration owners are deleted.

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

Do not append worktree-specific notes, complete validation histories or worker transcripts to this plan. Keep this status block current and concise. Git history is the validation history.

## Required authority documents

- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/language-overview.md`
- `docs/src/docs/codebase/memory-management/borrow-validation/overview.bd`
- `docs/src/docs/codebase/style-guide/style-guide.bd`
- `docs/src/docs/codebase/style-guide/testing.bd`
- `docs/src/docs/codebase/style-guide/validation.bd`
- `docs/src/docs/progress/#page.bst`
- downstream config, entry-config and HTML-Wasm plans

The two architecture overviews remain authoritative. This plan fixes implementation order and fills missing ownership details. It does not reopen accepted TIR, language, memory or backend-neutrality decisions.

## Architecture checkpoint conclusion

The accepted Phase 7 work is coherent and should not be reverted:

- stable package, module and exported declaration origins
- canonical closed and open type identities
- stable exported generic parameter identities
- canonical generic trait-bound identities
- total direct-export and direct public-type projection
- provider-independent prepared syntax
- deterministic graph identities and wave calculations
- explicit executable, link and compiler-metadata lanes
- diagnosed-versus-infrastructure module outcomes

The churn is caused by phase order rather than repeated correctness failures.

The current implementation keeps adding donor-independent facts to a transient `CompiledModuleResult`, while the production build still:

- compiles per-entry reachable source closures
- filters graph waves down to normal entry jobs
- omits support roots and the project facade from semantic compilation
- reconstructs source public surfaces from headers and `ProjectPathResolver`
- carries only local `FunctionId` and binding-backed external call targets in HIR
- performs entry-`start` reachability during module compilation
- drops the accepted `DefinedPublic*` facts at the legacy `Vec<Module>` handoff
- materialises generic instances into the requesting module AST
- retains configured `package_folders`, default `/lib`, `@./` and entry-root fallback paths

That is the drift. More independent interface components would deepen it.

## Locked recovery decisions

These decisions resolve the Phase 7 ambiguities and bind the replacement plan.

### 1. Aggregate the interface producer now

No new independent `DefinedPublic*` field may be added to `CompiledModuleResult`.

The accepted export-origin and type-surface projectors, plus the corrected trait-requirement projector, become private constituents of one `PublicInterfaceDraft` construction path. The draft is the sole pre-HIR public-semantic handoff.

Exact Rust names may change. The single aggregate ownership boundary may not.

### 2. Keep a draft/final split

Public facts are produced at two semantic times:

- AST owns declaration semantics, canonical public types, folded values, trait contracts, evidence, receiver surfaces, generic-template descriptors and direct provenance.
- HIR and borrow validation own executable effects, return aliasing, mutation/consumption summaries, reactive effects and per-function executable provenance.

Therefore:

```text
AST semantic result
-> PublicInterfaceDraft
-> HIR and borrow summaries
-> provider re-export join
-> PublicSemanticInterface
```

`PublicSemanticInterface` is complete or absent. A diagnosed module exposes neither a partial interface nor a draft.

### 3. Make the final interface declaration-centric

The final interface must not remain a set of parallel arrays joined by public name at every consumer.

Conceptually:

```rust
pub struct PublicSemanticInterface {
    pub module_origin: StableModuleOriginIdentity,
    pub export_bindings: Vec<ExportBinding>,
    pub declarations: Vec<PublicDeclarationEntry>,
    pub reusable_evidence: Vec<PublicEvidenceEntry>,
}

pub struct PublicDeclarationEntry {
    pub origin: OriginDeclarationId,
    pub semantics: PublicDeclarationSemantics,
    pub provenance: SemanticProvenance,
}

pub enum PublicDeclarationSemantics {
    Function(PublicFunctionInterface),
    Struct(PublicStructInterface),
    Choice(PublicChoiceInterface),
    TransparentAlias(PublicAliasInterface),
    Constant(PublicConstantInterface),
    Trait(PublicTraitInterface),
}
```

The interface may use deterministic vectors plus indexes rather than the exact containers above. The invariants are:

- one semantic record per origin
- zero or more export bindings may name the same origin
- re-exports preserve the donor origin
- receiver methods remain attached to receiver surfaces
- all consumer-visible facts are owned and donor-independent
- backend planning facts are excluded

### 4. Treat the existing `DefinedPublic*` values as internal projection components

`DefinedPublicExportOrigins`, `DefinedPublicTypeSurface` and the unaccepted trait surface are not long-term build-boundary payloads.

Keep their proven projection logic where useful, but either:

- make them private builder steps inside `PublicInterfaceDraftBuilder`, or
- merge their value types into declaration-centric draft records.

They must not remain separate fields that every later phase has to rejoin.

### 5. Salvage Phase 7c2e, but do not commit it as-is

The uncommitted Phase 7c2e work is review input only.

Keep only the parts that fit the aggregate draft:

- stable trait requirement identities and owned requirement signatures
- canonical `This` representation
- deterministic requirement order
- total trait-origin joins
- focused invariant coverage

Correct the parent-review gap before reuse:

- every requirement receiver `this_type` must equal the owning `ResolvedTraitDefinition::this_type`
- direct parameter or return occurrences of the same local `This` type become a canonical trait-self placeholder
- no unrelated local `TypeId` may be classified as trait self
- mutable versus immutable receiver access is stored separately from the self type

Do not retain a broadly renamed AST handoff merely because it can hold more future fields. Replace it with one narrowly owned interface-projection input/result.

### 6. Provider interfaces are the next structural consumer

After the aggregate direct draft is complete enough to represent every current public declaration category, the next structural milestone is completed source-provider interfaces and canonical per-node scheduling.

Do not continue adding producer-side facts while provider binding remains header/path based.

### 7. Source re-exports are finalized from provider interfaces

Direct declaration facts are built by the declaring module. Re-exporting modules consume completed provider interfaces, add their own `ExportBinding` values and retain the original declaration origins.

A consumer never opens provider headers, AST or HIR to reconstruct a re-export.

The exporting interface owns an immutable canonical copy or self-contained view of every semantic record required by its bindings so package facades can later be serialized without reopening dependencies.

### 8. Trait requirement self types use an explicit canonical placeholder

Trait requirements do not project their local synthetic `This` `TypeId` through ordinary canonical type identity.

Use a dedicated requirement type vocabulary equivalent to:

```rust
pub enum CanonicalTraitRequirementType {
    SelfType,
    Concrete(CanonicalTypeIdentity),
}
```

Composed `This` forms remain rejected by the language. This keeps the canonical requirement surface small and prevents a synthetic local generic parameter from masquerading as an exported generic parameter.

### 9. Reusable evidence identity is target-plus-trait

A canonical conformance is uniquely identified inside a compilation boundary by:

```text
canonical target type identity + canonical trait identity
```

A stable evidence record also maps each stable trait requirement identity to the stable implementing receiver-function origin where source evidence is used.

Source locations, `TraitEvidenceId`, `TraitId`, `TypeId`, `InternedPath` and declaration order are not evidence identity.

Builtin evidence uses the same target-plus-trait semantic key with a builtin ownership classification.

### 10. Generic template descriptors and bodies are separate artefact facts

The public interface exposes the generic semantic contract required for inference:

- stable declaration origin
- stable generic parameters and bounds
- canonical parameter and return types
- required evidence shape
- call/access/effect contract

The declaring module artefact retains the validated template body and its immutable compilation context, keyed by the stable declaration origin. Raw body tokens and donor-local semantic tables are compiler metadata for materialisation, not public semantic identity.

Consumers emit stable requests. They do not copy or mutate the provider template.

### 11. Generated requests that affect borrow transfer must be resolved before the requesting module becomes a successful artefact

The build system owns a compilation-boundary worklist and deduplicates by:

```text
stable generic declaration origin
+ canonical concrete type identities
+ required evidence identities
```

A module may reach validated HIR with pending generated requests, but it is not finalized as a successful `CompiledModuleArtifact` until the worklist has produced the call summaries required by its borrow validation.

The accepted flow is therefore:

```text
module AST and validated base HIR
-> enqueue generated requests
-> materialise/deduplicate generated sidecars to a fixed point
-> borrow-validate generated functions
-> produce generated local lifetime constraints and summaries
-> borrow-validate the requesting base module using resolved generated summaries
-> produce base-module local lifetime constraints and summaries
-> finalize module interface, link facts and artefact
```

Complete lifetime topology is instantiated later by project/link planning over reachable functions and builder lifecycle roots. This plan preserves the accepted artefact lanes and summary boundaries without claiming that deferred lifetime-region analysis has been implemented.

The worklist may process requests incrementally between graph waves. It remains build-owned and global to the project or package boundary.

### 12. HIR call targets become explicit source target classes

Replace the current two-way `UserFunction`/`ExternalFunction` call target with explicit classes equivalent to:

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

Private helper calls from a generated sidecar back into its declaring module use an artefact-local declaring-module reference. They do not create consumer-visible public declaration identities.

Borrow validation resolves:

- local calls from local HIR summaries
- cross-module calls from completed provider interface summaries
- generated calls from generated sidecar summaries
- binding calls from immutable binding package metadata

It never opens foreign HIR as local control flow.

### 13. Link facts are recorded per function before entry reachability

Remove `collect_reachability_from_start` from module finalization.

Each source or generated function records backend-neutral facts independently. Entry and package planning later compute exact reachable unions from explicit roots.

A module artefact does not filter external imports, helpers, assets or target-gated features through its dormant `start` during compilation.

### 14. Project-context provenance uses a general synthetic-interface dependency vocabulary

This plan establishes provenance plumbing even though the downstream config plan supplies the full `@project` surface.

Public facts and functions record stable dependencies on synthetic compile-time interface members. An empty dependency set is portable. A non-empty project-global dependency set is project-context provenance.

The same representation can support builder-owned synthetic interfaces without pretending they are `@project`.

Package-facade validation walks public facts and reachable source/generated call edges. It rejects prohibited project-context dependence without reparsing source.

### 15. Import semantics cut over before canonical compilation is accepted

The production canonical module path may not continue to depend on:

- importing-file-relative `@./`
- entry-root fallback
- path probing through another module's private files
- configured `package_folders`
- default `/lib`
- public-surface fallback by walking filesystem parents

Split source import namespace resolution from general compile-time path-literal resolution. Source imports resolve from the owning module root against a Stage 0 namespace and explicit provider contracts.

### 16. No long-lived dual production architecture

Internal implementation slices may prepare the cutover, but a milestone is not accepted while both old and new production paths remain callable.

Do not add compatibility wrappers, feature flags, fallback adapters or parallel payloads to preserve entry-closure compilation.

### 17. Worker validation is provisional

A worker-reported check, Clippy run or unit-test pass is not an accepted checkpoint.

Every code-bearing checkpoint requires:

- parent review of ownership, invariants and deletion scope
- focused validation
- `cargo fmt`
- full `just validate`
- a clean source diff inspection

## Target artefact boundary

Conceptual final shape:

```rust
pub struct CompiledModuleArtifact {
    pub interface: PublicSemanticInterface,
    pub executable: ModuleExecutable,
    pub link_facts: ModuleLinkFacts,
    pub metadata: ModuleCompilerMetadata,
    pub fingerprints: ModuleFingerprints,
}
```

### `PublicSemanticInterface`

Contains only semantic facts visible to a source consumer:

- stable declaration origins and export bindings
- canonical type shapes
- function parameter names, access modes, defaults and canonical return channels
- function mutation and optional-transfer eligibility/effect categories
- return-alias and projection-alias summaries
- retained-parameter and outlives summaries
- external-boundary classifications
- reactive summaries
- folded exported constants and const-template values
- struct fields and folded defaults
- choice variants and payload fields
- transparent alias targets
- generic signatures, parameters, bounds and required evidence
- trait requirements and incompatibility facts that are part of the public contract
- reusable conformance evidence
- receiver surfaces and visible methods
- semantic provenance

### `ModuleExecutable`

Contains module-local executable state only:

- one local `TypeEnvironment`
- validated module-local HIR
- borrow facts
- local lifetime-region and escape facts

The accepted artefact lane reserves these facts conceptually. The current canonical-module milestone must preserve the lane and stable handoff without claiming the deferred lifetime analysis has landed.

Normal modules may have an optional dormant `start`. Support roots and facades have none.

### `ModuleLinkFacts`

Contains per-function backend-neutral planning facts:

- local, cross-module and generated source calls
- binding-backed calls
- helper and capability requirements
- reactive features
- numeric, cast, map and target-gated operations
- runtime paths and asset usages
- generated request references
- executable provenance
- a stable origin-to-local-function lookup for exported callable roots

It does not carry a complete `ExternalPackageRegistry`.

### `ModuleCompilerMetadata`

Contains non-HIR compiler and builder metadata:

- dormant root activity
- folded top-level fragment values and insertion indexes
- resolved entry-local metadata
- documentation and API-index facts
- rendered path usages
- validated generic-template body artefacts and materialisation context
- structured warnings

### `ModuleFingerprints`

Contains exactly the five accepted base fingerprints:

- public interface
- implementation
- dormant root activity
- runtime dependency
- documentation

Fingerprint encoding has one deterministic owner. It does not hash process-local IDs, absolute paths, source locations or unordered map iteration.

## Current implementation disposition

| Current owner | Decision | Replacement or final owner |
|---|---|---|
| `semantic_identity.rs` | Keep | Stable semantic identity vocabulary |
| `canonical_type_identity.rs` | Keep and narrow dead-code allowances | Canonical cross-module type projection |
| `DefinedPublicExportOrigins` | Keep projection logic; internalize | `PublicInterfaceDraftBuilder` |
| `DefinedPublicTypeSurface` | Keep projection logic; internalize | declaration-centric draft records |
| uncommitted `DefinedPublicTraitSurface` | Salvage after invariant fix | declaration-centric trait draft records |
| `ResolvedPublicTypeRootTable` / proposed generalized AST table | Replace | one narrow interface-projection input/result outside executable AST |
| separate `CompiledModuleResult.defined_public_*` fields | Delete in R1 | one `PublicInterfaceDraft` field, then final artefact |
| `Module` three-lane payload | Retain lane contents temporarily | `CompiledModuleArtifact` |
| `ModuleLinkFacts.external_package_registry` | Delete | build-boundary binding registry plus per-function link facts |
| `ProjectModuleGraph` identity, ancestry, edges and waves | Keep and make production-complete | canonical project graph and scheduler |
| dead-code support visibility query | Wire or delete | graph-aware source namespace resolution |
| `ModuleEntryCompileWaves` | Delete at graph cutover | canonical node jobs for every selected role |
| `DiscoveredModule` and per-entry input closures | Delete at graph cutover | `ModuleCompilationJob` over one `SemanticSourceSet` |
| reachable-file BFS seeded per entry | Replace | one project/package source scan plus per-module semantic source classification |
| `ProjectPathResolver` import fallback behavior | Split and delete | Stage 0 source namespace resolver; separate path-literal resolver |
| project-local source package discovery and `package_folders` | Delete | structural `+*.bst` support packages and project facade |
| AST emitter generic instance materialisation | Delete | build-owned generated sidecar worklist |
| `CallTarget::UserFunction` for foreign source calls | Replace | explicit local/cross-module/generated source targets |
| module-local `start` reachability filtering | Delete | entry/package link planning |
| flat `BackendBuilder::build_backend(Vec<Module>, ...)` | Delete after assembly cutover | `ProjectCompilation` |
| `FileKind::NotBuilt` | Delete if no longer required | tooling outcome without fake output artefact |

## Milestone policy

The recovery is organized by architectural milestones rather than dozens of accepted micro-components.

A milestone may contain several implementation commits. It is accepted only when its exit gate is met and obsolete production owners named by that milestone are deleted.

## Milestone R0: Freeze the accepted baseline and preserve unaccepted work

Status: completed by this architecture checkpoint and replacement plan.

Actions:

- Treat `4a0cd4e01` as the last accepted compiler implementation checkpoint.
- Preserve the uncommitted Phase 7c2e diff as a patch or named stash outside the accepted branch.
- Do not merge or commit 7c2e wholesale.
- Record the salvage/deletion decisions from this plan in the active status block.
- Replace the previous plan contents without modifying accepted implementation code.

Exit gate:

- replacement plan reviewed against the architecture authorities
- accepted and unaccepted work clearly separated
- no claim that worker-only validation accepted 7c2e

## Milestone R1: Consolidate the public-interface producer boundary

Status: completed by the accepted R1 aggregate-draft checkpoint.

Goal: one pre-HIR aggregate instead of another parallel interface component.

Implementation:

1. Introduce one `PublicInterfaceDraftBuilder` and one owned `PublicInterfaceDraft`.
2. Move the accepted direct export-origin and canonical type-surface outputs behind that builder.
3. Rework the useful Phase 7c2e trait-requirement projection into the same builder.
4. Validate trait receiver `this_type` against the owning trait and emit explicit canonical `SelfType` facts.
5. Replace the AST-owned public-root field family with one narrow projection input/result. Prefer an `AstBuildResult` or semantic side result over widening executable `Ast`.
6. Replace all separate `CompiledModuleResult.defined_public_*` fields with one draft field.
7. Remove test-only getters and dead-code allowances that existed only because each component lacked a production consumer.
8. Keep the draft private to compiler/build orchestration. Do not expose it to backends.

Required direct draft coverage:

- free functions
- structs and choices
- transparent aliases
- constants
- receiver methods attached to direct public receivers
- generic parameters and bounds
- traits and requirement signatures
- direct export bindings and origins

Tests:

- table-driven projection tests for all declaration categories
- trait self-type positive and mismatch invariant tests
- direct/imported/alias-target origin stability cases retained from accepted coverage
- one orchestration test proving the module result carries exactly one aggregate draft
- no duplicate getter-level tests for fields already covered by a declaration projection case

Deletion gate:

- no separate `defined_public_export_origins`, `defined_public_type_surface` or trait-surface field on `CompiledModuleResult`
- no generalized transient AST bag with open-ended future fields

Validation:

- focused interface projection and orchestration tests
- `cargo fmt`
- `just validate`

## Milestone R2: Complete direct semantic facts and artefact finalization inputs

R2a checkpoint: completed by the declaration-centric direct-record implementation. The draft now
owns module identity, separate export bindings and exactly one closed semantic record per unique
direct declaration origin. Transient `DefinedPublic*` aggregates are consumed before the boundary,
and receiver methods attach to their nominal records through total deterministic joins.

R2e checkpoint: direct source-canonical evidence now uses canonical target-plus-trait identity and
maps authored-order stable trait requirements to the exact stable receiver origins already attached
to declaration records. Private targets and traits remain excluded, builtin evidence remains
compiler-global and no donor-local evidence, trait, requirement, type or path identity crosses the
draft boundary.

R2i checkpoint: borrow validation and the direct declaration draft now share one frontend-owned
call-summary vocabulary. Non-generic exported free functions and receiver methods join exactly once
through stable function origins and local HIR IDs after borrow validation. Exported generic
templates carry an explicit pending-generated state until R3 sidecars produce concrete summaries.

R2j checkpoint: one frontend-owned encoder now produces deterministic, unambiguous canonical
bytes for every semantic fact retained by the completed direct draft. It sorts semantic sets by
stable identity, preserves authored-order contracts, rejects incomplete or wrong-owner states and
chooses neither a digest nor a persistent cache format ahead of R7.

Goal: make one declaring module able to produce every direct semantic fact required by a future provider interface.

Implementation:

1. Convert draft records to the declaration-centric shape.
2. Add folded exported constant values, const-template values and const-record field values using owned backend-neutral value types.
3. Add function and field defaults where they are part of the callable/constructor contract.
4. Add complete struct/choice constructor semantics and receiver surfaces.
5. Add stable trait requirements, public incompatibility facts and reusable evidence.
6. Introduce target-plus-trait evidence identity and stable requirement-to-method mappings.
7. Retain validated generic template descriptors in the draft and template body artefacts in compiler metadata.
8. Produce direct function provenance dependencies from AST/HIR facts.
9. Extend borrow analysis to retain public-call summaries keyed by local `FunctionId`:
   - parameter access mode
   - mutation effect
   - optional transfer eligibility and effect category
   - return alias summary
   - relevant reactive effect
10. Finalize direct declaration records after borrow validation by joining stable function origins to local summaries exactly once.
11. Add deterministic canonical encoders for later fingerprinting, but do not introduce persistent serialization.

The result of R2 is still an internal direct interface draft because source re-exports require completed provider interfaces. Do not call it a complete `PublicSemanticInterface` yet.

Tests:

- folded constant and default values survive without AST/TIR IDs
- evidence identity is stable across local `TypeId`/`TraitId` allocation changes
- requirement mappings preserve authored order and stable implementing function origins
- public function summaries are complete and reject missing local summary joins
- private declarations do not leak into direct interface records
- project-context provenance plumbing has empty and synthetic non-empty unit cases

Deletion gate:

- no public semantic fact is reconstructed from HIR display names or rendered type names
- no trait/evidence public fact retains donor-local IDs
- no generic body token stream is treated as public semantic identity

Validation:

- focused AST/HIR/borrow/interface tests
- canonical integration cases where existing user-visible behavior is involved
- `cargo fmt`
- `just validate`

## Milestone R3: Move generated functions to build-owned sidecars

Goal: remove consumer-local generic body emission before canonical provider compilation depends on it.

Implementation:

1. Replace `GenericFunctionInstanceKey` path-plus-local-`TypeId` identity with stable declaration origin, canonical concrete type identities and required evidence identities.
2. Change AST generic calls to emit stable requests and explicit generated call targets.
3. Extract directly-defined validated generic templates into the declaring module's template store.
4. Split module semantic compilation so a module can return validated base HIR plus pending generated requests before final borrow/artefact finalization.
5. Add one project/package-boundary generated worklist with deterministic deduplication.
6. Materialize sidecars using the declaring module's immutable template body/context and a generated-local type environment.
7. Allow generated bodies to enqueue more requests until a fixed point.
8. Borrow-validate each generated function and publish its call summary before final borrow validation of requesters that need it.
9. Associate each sidecar with its declaring module while keeping ownership in the consuming compilation boundary.
10. Support private declaring-module helper calls without making private helpers source-visible identities.
11. Delete AST-emitter materialisation and base-module mutation.

Every generated sidecar carries generated-local type context, HIR, borrow facts, local lifetime facts and summaries, link facts and fingerprints.

Tests:

- two entries requesting the same concrete instance produce one sidecar
- nested generic requests reach a deterministic fixed point
- concrete type/evidence identity, not local IDs or aliases, controls deduplication
- a generated body can call a private helper in its declaring module
- a diagnosed request blocks only dependent roots
- recursive request diagnostics remain source-attributed
- base module HIR and interface fingerprints do not change when another consumer requests a new instance

Deletion gate:

- `AstEmitter` no longer materializes concrete generic functions
- base module AST/HIR is never extended by a consumer request
- no path-plus-local-`TypeId` generated identity remains in production

Validation:

- focused generic/worklist/HIR/borrow tests
- existing generic integration contracts
- `cargo fmt`
- `just validate`

## Milestone R4: Canonical graph and provider-interface cutover

Goal: replace entry closures, fallback import resolution and header-based foreign surfaces with one canonical project/package compilation path.

This is the main cutover milestone. Its internal slices are not accepted as separate production architectures.

### R4a: Build the canonical namespace and semantic source sets

- Scan each selected project or package source boundary once.
- Tokenize/scan each `.bst` source candidate once and retain structural provider references.
- Build `SemanticSourceSet` per module from its root and same-owner reachable sources/assets.
- Stop traversal at child module and support package boundaries.
- Build check-only orphan units as owned `.bst` minus canonical semantic `.bst`.
- Build a graph-aware import namespace from module ownership, direct children, visible support packages, registered packages and provider contracts.
- Resolve ordinary source imports from the owning module root, not the importing file.
- Reject `@./`, parent traversal and entry-root fallback.
- Split source import resolution from general compile-time path-literal resolution.
- Make provider-backed explicit-extension imports module-root-relative unless their provider contract declares another explicit owner.
- Remove configured `package_folders`, default `/lib` and project-local source-package scanning.

### R4b: Compile every selected graph role

- Create one `ModuleCompilationJob` per selected normal, support or project-facade node.
- Compile source-backed Core and Builder packages as separate package graphs before consumers.
- Treat registered package facades as API-only semantic roots supplied by package metadata, independent of cosmetic root filename.
- Include support private descendants before their facade.
- Include project-facade dependencies before the facade.
- Normal modules compile dormant root work whether or not an entry later activates it.
- API-only roots have no implicit or sentinel `start`.

### R4c: Bind completed provider interfaces

Replace the current binding signature with an input equivalent to:

```rust
pub struct InterfaceBindingInput<'a> {
    pub prepared: PreparedHeaderSyntax,
    pub module_namespace: &'a ResolvedModuleNamespace,
    pub source_providers: &'a CompletedSourceProviderInterfaces,
    pub binding_packages: &'a BindingInterfaceRegistry,
    pub synthetic_interfaces: &'a SyntheticCompileTimeInterfaceRegistry,
}
```

Binding must:

- resolve imported stable declaration origins
- project canonical provider types into the consumer `TypeEnvironment`
- import folded values without re-folding
- import trait requirements, evidence and receiver surfaces
- retain final file-local visibility and collision results
- resolve public re-exports from provider interfaces
- produce explicit cross-module source call targets
- never inspect provider AST/HIR/private headers

### R4d: Finalize interfaces, effects and graph outcomes

- Join direct drafts, provider re-exports and local borrow/effect summaries into complete `PublicSemanticInterface` values.
- Construct `CompiledModuleArtifact` with all four lanes and five fingerprints.
- Merge successful wave string-table deltas in canonical `ModuleId` order before later consumers use remapped payloads.
- Build `GraphCompilationOutcome { successful, diagnosed, blocked }`.
- A diagnosed provider exposes no interface.
- Mark consumers blocked without semantically compiling them.
- Continue independent branches.
- Abort the boundary only on `CompilerError`.
- Emit shared diagnostics once.

### R4e: Delete the old production path

Delete in the same milestone:

- `DiscoveredModule`
- `ModuleEntryCompileWaves`
- per-entry reachable source closures
- provider-body copying into consumers
- module-root public-surface fallback by filesystem walk
- `package_folders` and `/lib` project package discovery
- source `@./` and entry-root import fallback
- old source import binding from combined donor headers
- compatibility adapters introduced during the cutover

Tests:

- one shared provider compiled once for multiple entries
- one shared provider failure emits one diagnostic set
- blocked consumers emit no secondary name/type cascades
- independent branches continue
- imported function/type/constant/trait/evidence/receiver/generic behavior uses provider interfaces
- module-root-relative nested-file imports
- no private path bypass through child modules or support packages
- strict support scope visibility and overlap diagnostics
- separate `@html` Builder package graph
- API-only support/facade roots have no start
- deterministic results under varied Rayon completion order

Acceptance gate:

- one canonical node scheduler is the only directory-project production path
- source-provider binding consumes completed interfaces
- no entry closure or fallback import path remains
- full `just validate` passes

## Milestone R5: Per-function link facts, entry assemblies and package assemblies

Goal: move runtime reachability and activation out of module compilation.

Implementation:

1. Record per-function link facts during HIR construction/finalization for base and generated functions.
2. Remove module-finalization reachability from `start` and remove reachability-filtered external import lists.
3. Remove the complete external package registry from every module artefact.
4. Build exact cross-artefact source/generated call resolution through the compiled graph.
5. Build `EntryAssembly` values that activate only one normal module's dormant root work, fragments and entry settings.
6. Build `ProjectPackageAssembly` over the compiled project facade and reachable descendant surfaces.
7. Propagate provenance through local, cross-module and generated call edges.
8. Reject package exports whose public facts or reachable implementation depend on prohibited project context.
9. Provide explicit target-validation roots from each entry or package assembly.
10. Build success-only `ProjectCompilation` only when every selected requirement succeeded.

Tests:

- importing a normal module never activates its root work
- one canonical normal module can produce multiple assemblies without recompilation
- exact reachable binding/runtime assets differ correctly by entry
- unreachable private target-unsupported code does not fail `check`/build validation
- project facade never bypasses `export:`
- direct and transitive project-context exposure is rejected
- unreachable private project-context implementation remains allowed where the authority permits it

Deletion gate:

- no module-wide start reachability is a linking authority
- entry/package assembly never invokes compiler semantic stages
- no backend derives call or package identity from rendered names

Validation:

- focused link/assembly/provenance tests
- integration output exact-once and imported-root suppression contracts
- `cargo fmt`
- `just validate`

## Milestone R6: Migrate HTML, backend and output consumers to `ProjectCompilation`

Goal: make backends consume explicit assemblies and link plans without rediscovering project structure.

Implementation:

- replace `BackendBuilder::build_backend(Vec<Module>, ...)` with `ProjectCompilation`
- migrate HTML route generation to `EntryAssembly`
- preserve fragment interleaving and root `start` activation exactly once
- compute target partitions and validation from per-function reachable unions
- generate external JavaScript runtime/glue from reachable binding calls
- plan tracked assets from reachable link facts and compiler metadata
- preserve output ownership, manifests, stale cleanup and path validation
- remove `FileKind::NotBuilt` if tooling no longer needs it
- keep physical Wasm partition/layout work in the downstream Wasm plan

Tests:

- current HTML-JS output parity
- homepage and route policy parity
- duplicate route/output diagnostics
- external runtime asset/glue deduplication by reachable entry use
- tracked asset conflicts and relative output behavior
- no backend source scan or import reconstruction

Deletion gate:

- no flat module loop in the HTML builder
- no backend accepts a partial graph outcome
- no backend writes final project outputs directly

Validation:

- backend artifact and runtime integration contracts
- `cargo fmt`
- `just validate`

## Milestone R7: Complete `check`, dev reuse and fingerprint invalidation

Goal: finish command policy and in-memory reuse on the canonical artefact model.

Implementation:

### `check`

- compile every discovered project module in the selected boundary
- compile check-only orphan units without adding them to canonical artefacts
- retain successful independent artefacts from `GraphCompilationOutcome`
- validate actual linkable roots without backend lowering or output writing
- report diagnosed modules once and blocked modules without cascades

### `dev`

- retain successful immutable module/package/generated artefacts
- invalidate changed modules from source/config fingerprints
- recompile semantic consumers only when public-interface fingerprints change
- relink/regenerate entries for implementation, root, runtime, generated, entry-setting or relevant config changes
- keep diagnostic and output ordering deterministic across rebuilds

### Fingerprints

- finalize one canonical fingerprint encoder for all five module fingerprints
- include access/effect summaries in public-interface fingerprints, including optional-transfer summaries, alias/projection summaries, retained-parameter relationships, outlives constraints and external-boundary classifications
- exclude docs-only facts from semantic/implementation fingerprints
- include generated request-set changes in implementation/worklist invalidation
- preserve package capability compatibility facts without implementing persistent caches

Tests:

- private body edit relinks without semantic consumer recompilation
- public effect edit recompiles consumers
- root-only edit relinks its entries
- docs-only edit avoids executable invalidation
- failed branch does not discard reusable independent artefacts
- orphan unit diagnostics do not alter module public interfaces or backend roots

Deletion gate:

- `projects/check.rs` has no all-or-error `Vec<Module>` path
- dev has no compile-everything path when artefacts are reusable
- no duplicate invalidation policy exists in builders/backends

Validation:

- focused reuse/invalidation tests
- dev/check integration tests
- `cargo fmt`
- `just validate`

## Milestone R8: Repository migration, documentation and final deletion audit

Goal: leave one documented architecture with no legacy migration owners.

Implementation:

- update all fixtures and source imports to module-root-relative syntax
- remove `@./`, `package_folders` and `/lib` examples
- update `moth new` scaffolding
- update language, project-structure, packages, imports and compiler educational pages
- add complete normal/support/facade project trees and legal/illegal topology examples
- update the progress matrix only for behavior that is now implemented
- rebuild generated documentation
- prune superseded implementation-shaped unit tests and dead-code allowances
- remove stale plan comments naming future consumers that now exist

Final deletion audit:

- no `DiscoveredModule`
- no entry-closure compilation
- no flat `Vec<Module>` backend handoff
- no configured project-local source-package scanning
- no default `/lib`
- no source `@./`
- no entry-root import fallback
- no donor header/body copying into consumers
- no donor-local cross-module type/evidence transport
- no consumer-local generic materialisation
- no borrow path opening foreign HIR
- no module-level start reachability used as link authority
- no API-only sentinel start
- no compatibility wrappers around removed paths

Validation:

- documentation release build for documentation-only final slices
- otherwise `cargo fmt` and `just validate`
- manual architecture audit from `validation.bd`

## Required end-to-end contracts

The canonical integration suite remains the primary owner of user-visible behavior. Add focused Rust tests only for hidden graph, identity, interface, scheduling and fingerprint facts.

Required canonical contracts:

- shared module compiled once across entries
- shared failure diagnosed once
- independent graph branches continue
- blocked consumers have no cascades
- module-root-relative imports from nested files
- strict child-module facade enforcement
- strict scoped support visibility
- support-scope overlap diagnostics
- project facade assembly
- separate Builder/Core/dependency package graphs
- stable identities under source-file moves and declaration reordering
- re-export alias changes binding identity without changing declaration origin
- canonical type/evidence identity independent of local allocation
- generated sidecar reuse and fixed point
- cross-module borrow effects and return aliasing
- exact per-entry runtime/link unions
- no source/provider fallback to `@./`
- no API-only start
- check-only orphans excluded from artefacts
- deterministic diagnostics and output under parallel scheduling
- direct and transitive project-context package-export rejection
- consuming-project inputs do not satisfy dependency build-input contracts

## Test discipline

- Prefer one realistic multi-module integration case over many getter-shaped unit tests.
- Use table-driven unit tests for canonical declaration variants and total join failures.
- Do not add public/test-only accessors solely to inspect a temporary component.
- Remove superseded tests when an old API or owner is deleted.
- Preserve one primary contract owner per behavior.
- Run `cargo run --quiet -- tests --audit` after fixture metadata changes.

## Validation requirements

Every code-bearing milestone requires:

```bash
cargo fmt
just validate
```

Also perform the manual architecture audit required by `validation.bd` whenever the milestone changes stage ownership, HIR, diagnostics, types, provider binding, graph scheduling or backend handoff.

A focused command is iteration evidence only. It is not a milestone acceptance gate.

Documentation-only slices use the documentation release-build gate from `validation.bd` and do not claim full compiler validation.

## Final architecture acceptance

Before marking this plan complete, verify:

- each physical module is semantically compiled once per project/package boundary
- every source consumer binds completed immutable provider interfaces
- every successful module artefact has a complete public interface, executable lane, per-function link facts, metadata and five fingerprints
- diagnosed modules expose no partial interface
- generated functions live only in sidecars
- borrow validation uses local, provider, generated or binding summaries without foreign HIR inspection
- entry and package assembly never trigger semantic compilation
- support and project-facade roots are API-only
- source imports are module-root-relative and topology-checked
- project, Builder, Core and dependency source graphs remain separate
- backends receive success-only explicit project/link plans
- source, tests, docs, progress matrix and roadmap agree
- the HTML-Wasm plan can proceed without redesigning frontend module identity, provider binding or linking

## Deliberately deferred work

- persistent module/package/generated artefact serialization
- on-disk cache layout, eviction and migration
- dependency declaration syntax and local path dependencies
- package registries, remote fetching, versions and lockfiles
- precompiled dependency caches
- direct normal-sibling imports
- cross-entry browser chunking
- physical Wasm module partition and Component Model integration
- cross-build generated-instance caches
