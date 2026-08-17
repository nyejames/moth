# Frontend Module Compilation Ownership Cleanup Plan

## Current state

```text
WORK_ID: frontend-module-compilation-ownership-cleanup
WORK_SOURCE: docs/roadmap/plans/frontend-module-compilation-ownership-cleanup-plan.md
BASE_REVISION: d670b1b25dcf6edb534fc9b67b91291f168bac24
STATUS: queued
CURRENT_SCOPE: plan authored from the current main branch ownership audit
COMPLETED: planning audit, target boundary definition and roadmap sequencing
NEXT_ACTION: after test-suite honesty completes, re-anchor Phase 0 against the then-current main branch before changing production code
VALIDATION: plan-only authoring change, implementation validation is defined per phase below
AUDITS: compiler/build ownership, generated-function ownership, module artefact ownership, config and direct Moth-template compiler clients, canonical docs and style-guide boundary rules
BLOCKERS: TIR corrections is active, then test-suite honesty must complete
NOTES: this plan must complete before command timing accounting corrections because that plan references and measures frontend/build boundaries that this plan moves
```

## Purpose

Restore one clear owner for frontend semantic compilation.

The build system should decide what source belongs to a module, prepare the source needed for graph construction, schedule completed providers and publish completed compiler results. It should not implement the compiler's local semantic stage sequence.

The compiler should expose a small opinionated service for one module compilation. That service owns provider binding, local declaration ordering, AST semantics, public-interface projection, HIR lowering and validation, borrow validation and the semantic work required to finish generated functions. The build system supplies already-known provider and generated boundary facts, then receives a complete success, diagnosed result or `CompilerError`.

This plan also closes the other production escape hatches that currently assemble raw frontend stages outside `compiler_frontend`, then hardens the canonical docs, style guide, Rust visibility and architecture tests so the ownership drift cannot quietly return.

The goal is not to hide complexity behind a broad abstraction. The goal is to put existing complexity under the owner that already owns its semantics and leave Stage 0 with a smaller data-oriented scheduling contract.

## Roadmap position

This plan runs after:

1. TIR corrections and simplification
2. test-suite honesty and infrastructure hardening

It runs before:

1. command timing accounting and reporting corrections
2. constant evaluation and static control-flow optimisation
3. the remaining queued roadmap chain

This ordering is deliberate. The command timing plan currently references `src/build_system/create_project_modules/frontend_orchestration.rs` and measures frontend/build boundaries that this plan will relocate. Repairing timing ownership first would create avoidable churn and could make the timing plan encode the wrong architectural boundary.

The test-suite honesty plan stays first because this refactor needs trustworthy failure identity, platform coverage and orchestration tests before moving a large compiler/build seam.

## Planning snapshot and confirmed current shape

The final planning recheck on 2026-08-17 confirmed `main` at:

```text
d670b1b25dcf6edb534fc9b67b91291f168bac24
```

The following shape is confirmed at that revision.

### `compiler_frontend/pipeline.rs` is mostly a stage facade

`CompilerFrontend` currently exposes operations such as:

- source tokenization and per-file preparation
- `sort_headers`
- `headers_to_ast`
- `generate_hir`
- `check_borrows`

It does not own the complete canonical module semantic sequence.

### `build_system/create_project_modules/frontend_orchestration.rs` owns the real module semantic sequence

The build-system file currently performs or directly coordinates:

```text
bind retained provider interfaces
-> order declarations
-> build export and public-origin seed state
-> build AST
-> build the public-interface draft
-> canonicalise and register generated requests
-> build HIR origin mappings
-> lower and validate HIR
-> collect link facts
-> run initial borrow validation
-> materialise generated functions
-> converge generated and base call summaries
-> rerun borrow validation as convergence requires
-> finalise the public interface
-> construct the module semantic result
```

That sequence is compiler semantic work even though Stage 0 is the caller that decides when the module is ready.

### Generated semantic convergence is build-owned

`src/build_system/create_project_modules/generated_summary_convergence.rs` derives a semantic call dependency model from validated HIR, mutates call-summary state and reruns borrow validation for base and generated HIR.

This is compiler analysis. Build-owned work should stop at boundary-wide request aggregation, deduplication, availability, publication, placement and reuse.

### Compiler artefact types are declared in `build_system/build.rs`

The current build module declares compiler-produced values including:

- `ModuleExecutable`
- `ModuleLinkFacts`
- `ModuleCompilerMetadata`
- `ModuleRootActivity`
- `ResolvedConstFragment`
- `ModuleExternalImport`
- `Module`
- `GeneratedFunctionSidecar`
- `CompiledModuleArtifact`
- `ModuleSemanticDraft`

The accepted architecture describes these as compiler result lanes. Leaving the Rust types under `build_system` would force a reverse compiler-to-build dependency if semantic orchestration were moved without first correcting data ownership.

`ProjectCompilation`, entry assembly, output records and builder orchestration remain build-owned.

### `CompilerFrontend::new` depends on project-tool config

`compiler_frontend/pipeline.rs` currently imports `crate::projects::settings::Config` and extracts compiler options from it.

The compiler should receive compiler-owned options. It should not know the project tool's configuration container.

### Project config assembles raw frontend stages

`src/build_system/project_config/parsing.rs` currently performs its own tokenizer, header preparation, interface binding, declaration ordering and AST construction sequence.

Project config is a special compile-time source client and stops at folded AST data, but the build system should call one named compiler service for that sequence rather than owning raw stage assembly.

### Direct Moth-template compilation assembles raw frontend stages

`src/projects/html_project/moth_template/compile.rs` currently performs frontend preparation, header aggregation, binding, ordering and AST folding directly.

This is especially important because `docs/compiler-design-overview.md` already describes direct Moth-template compilation as a narrow compiler service rather than a second compiler pipeline.

### The docs state the intended ownership but leave escape hatches

The canonical architecture already says the compiler owns source preparation, interface binding and local semantic compilation while Stage 0 owns discovery and scheduling. Some later wording still describes compile-wave stage steps from the build-system perspective and says the build system owns generated worklist scheduling without separating boundary scheduling from semantic convergence.

The style guide requires clear stage separation but does not yet state a concrete production dependency rule that prevents build/project code from reassembling AST, HIR and borrow stages.

## Target architecture

The target flow is:

```text
Build system / Stage 0
    discover and own source topology
    select semantic source sets
    ask compiler to prepare selected source exactly once
    consume structural provider references
    finish provider graphs and deterministic waves
    wait for completed provider interfaces
    build one compiler input value
        |
        v
CompilerFrontend::compile_module(...)
    bind provider interfaces
    order local declarations
    run AST semantics
    build public semantic projection state
    lower and validate HIR
    collect compiler-owned link facts
    run borrow validation
    complete generated semantic work
    finalise public semantic interface
    assemble compiler-owned module artefact and generated delta
        |
        v
Build system / Stage 0
    merge deterministic string identity deltas
    publish module artefact and generated delta atomically
    mark diagnosed or blocked graph nodes
    continue deterministic scheduling
```

The build system may know that a compiler module job produces a complete semantic artefact. It must not know how the compiler sequences AST, public-interface projection, HIR and borrow analysis to produce it.

## Locked ownership decisions

### One canonical production module compile entry

Normal module, support module, package-facade and synthetic single-file semantic compilation use one compiler-owned module compilation service after provider-independent preparation is complete.

Do not keep a build-owned semantic path as a compatibility route.

### `pipeline.rs` is the thin opinionated frontend facade

`pipeline.rs`, or the root API it exposes, remains the obvious entry surface for compiler clients. The implementation may live in focused child modules such as `compiler_frontend/module_compilation/`.

Do not move the existing 2,000-line build orchestration file wholesale into `pipeline.rs`.

### Compiler result types are compiler-owned

Types whose meaning is "the compiler produced this semantic module result" move under `compiler_frontend`.

Build-owned project aggregation types stay in `build_system`.

A move must delete the old definition. Do not keep forwarding aliases or duplicate payload structs.

### The compiler receives narrow compiler options

Replace the `projects::settings::Config` dependency in `compiler_frontend/pipeline.rs` with a compiler-owned input such as `FrontendOptions` or a more focused equivalent.

Only values the compiler actually needs cross the boundary.

### Stage 0 keeps provider-independent source scheduling

Stage 0 has a legitimate reason to call compiler source preparation before provider interfaces exist. It needs retained structural provider references to finish the graph.

That exception is narrow:

- compiler code owns tokenization and header preparation semantics
- Stage 0 decides which source candidate to prepare and when
- Stage 0 may consume structural provider references from the returned prepared syntax
- Stage 0 does not bind source symbols, order declarations or enter AST/HIR/borrow stages

### Generated boundary scheduling and generated semantic work are different owners

Build owns:

- boundary-wide generated identity aggregation
- deterministic deduplication against already published sidecars
- completed sidecar storage
- transactional publication
- boundary placement and reuse

Compiler owns:

- canonicalising a concrete request from AST facts
- generated AST/HIR materialisation
- generated HIR validation
- generated borrow analysis
- call-summary installation and semantic convergence
- the local semantic fixed point required to complete a module compilation transaction
- construction of the final generated sidecar delta

Do not make the compiler call back into mutable Stage 0 stores while semantic analysis is running. Stage 0 should supply an immutable value/view of already available generated summaries and provider materialisation contexts.

### Config and direct Moth-template compilation use named compiler services

These are intentionally shorter compiler paths, not permission for project/build modules to compose raw stage functions.

The final production clients should look conceptually like:

```text
build config owner -> compiler.compile_config_source(...)
HTML tooling owner -> compiler.compile_moth_template_source(...)
```

Exact Rust names remain implementation details.

### Raw stage APIs become compiler-internal where practical

Production code outside `compiler_frontend` should not directly call the owners for:

- provider interface binding
- local declaration ordering
- `Ast::new`
- AST-to-HIR lowering
- borrow checking
- public-interface draft construction
- generated semantic convergence

Tests may target stage-local APIs from their owning compiler modules.

Later build/link code may still consume completed HIR, function IDs, link facts and borrow/lifetime result data where the architecture explicitly requires those completed artefact lanes. This plan bans external orchestration of the stages, not legitimate consumption of their final data.

### No semantic or target-policy redesign

This refactor preserves accepted source semantics, diagnostics, deterministic identities, target validation policy and output behavior.

Do not combine it with language changes, lifetime-region implementation, backend lowering redesign or new caching policy.

## Documentation hardening required by this plan

Documentation changes are implementation work in this plan. They are not optional cleanup after the code move.

### `docs/compiler-design-overview.md`

Harden `Compiler input and result boundary`, frontend stage ownership, generated concrete functions and the implementation map.

The final document must state explicitly that:

- Stage 0 schedules a canonical module compile service, it does not invoke semantic stages individually
- compiler-owned module compilation is the only production owner of binding -> ordering -> AST -> HIR -> borrow sequencing
- build/project code must not construct public-interface drafts or rerun compiler analyses
- compiler module artefact lanes are produced and owned by the compiler boundary even when the build system stores them
- generated semantic convergence belongs to the compiler while boundary aggregation, deduplication and publication belong to the build system
- config and direct Moth-template compilation are explicit compiler services with shorter stopping points

### `docs/build-system-design.md`

Harden `Prepared-source orchestration`, `Deterministic scheduling and graph outcomes`, `Generated-function worklist`, project bootstrap and the implementation map.

Replace any wording that makes the compile-wave pseudocode look like Stage 0 is the owner of each semantic stage. The build-side conceptual sequence should be closer to:

```text
ready module + completed providers
-> build compiler input
-> compiler compile-module service
-> Success / Diagnosed / CompilerError
-> deterministic remap and publication
```

The document may still describe the compiler's internal semantic sequence for context, but it must label that sequence as compiler-owned.

Clarify that build-owned generated scheduling means boundary request availability, deduplication, publication and reuse. It does not include HIR mutation, borrow rechecks or semantic summary convergence.

Clarify project config bootstrap as a build-owned client of a named compiler service rather than a second build-owned frontend pipeline.

### `docs/src/docs/codebase/style-guide/style-guide.mtf`

Add a concrete production layering rule near stage separation and refactor moves.

At minimum it must state:

- `compiler_frontend` must not depend on `build_system` or project configuration containers to perform local semantic compilation
- `build_system/create_project_modules` must not assemble AST, HIR, public-interface or borrow stages
- project/build clients that need a shorter compiler path must use a named compiler service, not raw stage functions
- moving a stage owner requires moving its data contracts and deleting the old owner, not adding forwarding wrappers
- exceptions must be documented on the canonical architecture boundary rather than introduced locally

### Educational compiler-design docs

Audit `docs/src/docs/codebase/compiler-design/**` for this boundary.

`module-artefacts-and-reuse/module-artefacts-and-reuse.mtf` is already known to need a generated-sidecar wording correction. Its current statement that the build system "runs a worklist to a fixed point" must distinguish build boundary scheduling from compiler semantic completion.

Update any other page that teaches Stage 0, module compilation, generated functions, config compilation or direct Moth-template compilation.

### Roadmap plan references

Audit queued plans for source paths and ownership assumptions changed by this refactor.

`docs/roadmap/plans/command-timing-accounting-and-reporting-correction-plan.md` already references `src/build_system/create_project_modules/frontend_orchestration.rs` and must be updated before that plan starts.

Update any other active or queued plan that points at moved files or assumes build-owned semantic orchestration.

## Agent execution rules

Each implementation phase below is one bounded review unit intended to fit inside one coding-agent context.

- Do not start the next phase until the current phase gate is complete.
- Keep every phase buildable and testable.
- Update this plan's current-state capsule and checkboxes at the end of every accepted phase.
- Do not preserve an old API through compatibility aliases, forwarding wrappers or parallel payload types.
- Prefer moving ownership and then deleting the old owner in the same phase.
- If Phase 0 finds that active TIR or test-honesty work materially changed these boundaries, update this plan before implementing later phases. Do not force stale file names or stale type shapes onto the new tree.
- A phase may be split into A/B implementation commits when needed for review, but the complete phase gate remains the acceptance boundary.

## Mandatory gate for every phase

Every phase below ends with three explicit checks.

### Ownership and architecture audit

Inspect the changed owner and both adjacent boundaries. Look for:

- duplicated orchestration
- old and new payloads coexisting
- forwarding wrappers or aliases preserving the previous dependency direction
- build code importing compiler stage internals
- compiler code importing build/project containers
- semantic facts reconstructed from an earlier representation
- string-table or identity ownership added only to make a bad boundary compile
- generated HIR or borrow state exposed to Stage 0 mutation

### Style-guide review

Review every changed production file against `style-guide.mtf`, especially:

- one clear responsibility per module
- files under roughly 2,000 lines where practical
- main orchestration functions that read as named steps
- no copied comments that still name the old owner
- no unnecessary cloning introduced by the move
- no broad trait/callback framework hiding ordinary compiler flow
- no compatibility shims
- tests kept outside production files unless a narrow test-only hook genuinely belongs there

### Validation

Run the narrow tests for the changed owner, then at minimum:

```bash
cargo fmt --all -- --check
cargo test --workspace --quiet -- --format terse
cargo run --quiet -- tests --audit
just validate
```

When a phase changes timer-bearing code, also run the repository's current timers feature lane. When a phase changes generated-function behavior, run the generated, generic, public-interface and borrow-checker focused suites before `just validate`.

If the test-suite honesty plan introduces stronger canonical validation commands before this plan starts, Phase 0 must adopt them. The then-current validation authority wins over the static command list above.

---

# Implementation phases

## Phase 0: Re-anchor the repository and lock the ownership contract

### Context

This plan is queued behind active work. The implementation tree may move before this plan starts. The first phase therefore verifies the real repository shape and strengthens the architectural contract before any production owner moves.

This phase should make the intended dependency direction unambiguous enough that later phases can be reviewed against a written target rather than inferred from the refactor itself.

### Checklist

- [ ] Resolve the then-current `main` commit and record it as the implementation start revision in this plan.
- [ ] Compare the current versions of `compiler_frontend/pipeline.rs`, `build_system/create_project_modules/`, `build_system/build.rs`, project config parsing and direct Moth-template compilation against the planning snapshot above.
- [ ] Record every production caller outside `compiler_frontend` that directly invokes or imports raw binding, ordering, AST construction, HIR lowering, public-interface draft or borrow-analysis owners.
- [ ] Record every build-owned type that is semantically part of the compiler module artefact or generated sidecar result.
- [ ] Record the current generated request, summary, sidecar and materialisation-context data flow from requester AST through boundary publication.
- [ ] Record the exact current targeted tests for module preparation, module compilation, generated convergence, public-interface publication, single-file compilation, project config and direct Moth-template compilation.
- [ ] Record the current canonical benchmark/timing command used to detect gross frontend performance regressions after pure ownership moves.
- [ ] Update `docs/compiler-design-overview.md` with the hardened local module compilation service boundary described above.
- [ ] Update `docs/build-system-design.md` so Stage 0 compile waves clearly invoke one compiler semantic service rather than owning the stage sequence.
- [ ] Update generated-function ownership wording in both canonical architecture docs.
- [ ] Update config and direct Moth-template service wording in the canonical docs where it is currently permissive.
- [ ] Add the explicit compiler/build layering rule to `style-guide.mtf`.
- [ ] Update educational compiler-design pages that already describe this ownership boundary, including `module-artefacts-and-reuse.mtf`.
- [ ] Audit queued roadmap plans for paths or assumptions that this refactor will invalidate and record the required edits in this plan before code moves.
- [ ] Update this plan's current-state capsule with confirmed paths, blockers and baseline validation.

### Phase 0 gate

- [ ] Ownership audit confirms the plan matches the current tree and no newly added semantic owner was missed.
- [ ] Style-guide review confirms the new documentation states a concrete enforceable rule rather than only a preference for separation.
- [ ] Documentation checks and full validation pass with no production behavior changes.

Exit state: the current repository is re-anchored and the accepted docs already forbid the architecture this plan is about to remove.

## Phase 1: Move compiler semantic payloads and options to the compiler boundary

### Context

The semantic orchestration cannot move cleanly while the values it produces are declared under `build_system`. Moving the function first would either make `compiler_frontend` depend on `build_system` or require duplicate transition types. Both are worse than the current shape.

This phase fixes data ownership before control-flow ownership.

### Checklist

- [ ] Add a focused compiler-owned module compilation area, expected to be `src/compiler_frontend/module_compilation/` unless Phase 0 finds a better existing owner.
- [ ] Move the compiler-produced module artefact vocabulary out of `build_system/build.rs`.
- [ ] Include `ModuleExecutable`, `ModuleLinkFacts`, `ModuleCompilerMetadata`, `ModuleRootActivity`, resolved const-fragment metadata, module external-import facts, `Module`, `GeneratedFunctionSidecar` and `CompiledModuleArtifact` or their current equivalents.
- [ ] Move `ModuleSemanticDraft` to the compiler boundary or replace it with one compiler-owned named result that carries the same transient remap/publication facts.
- [ ] Keep `ProjectCompilation`, graph boundaries, entry assembly, builders, output records and publication stores build-owned.
- [ ] Replace `CompilerFrontend::new(&Config, ...)` with a compiler-owned options/input value containing only the settings the frontend actually consumes.
- [ ] Remove the `crate::projects::settings::Config` dependency from `compiler_frontend/pipeline.rs`.
- [ ] Split the current build-owned `PreparedModule` if needed so Stage 0-only facts such as implicit Moth-template provider activation stay build-owned while the semantic compilation payload is a compiler-owned input value.
- [ ] Preserve one exact string-table ownership path through preparation, semantic compilation, deterministic merge and publication.
- [ ] Remove loose duplicate active-root inputs where the same fact can be derived from the retained `FileId`, source table and source-origin table.
- [ ] Update call sites directly to the new owner and delete old type definitions in the same phase.
- [ ] Add or update tests for artefact remapping, lane coherence and successful publication using the compiler-owned types.

### Phase 1 gate

- [ ] Ownership audit finds no compiler semantic result type left in `build_system/build.rs` solely because the old orchestration lived there.
- [ ] Style-guide review confirms the new module is split by real data responsibility and does not become a generic dumping ground.
- [ ] Targeted artefact/publication tests and full validation pass.

Exit state: compiler semantic values can move through compiler code without a dependency back into `build_system`.

## Phase 2: Separate generated boundary publication from generated semantic completion

### Context

Generated functions are the hardest seam in the move. Boundary-wide deduplication is genuinely Stage 0 work, but generated HIR materialisation, borrow checking and summary convergence are compiler semantics.

This phase establishes that split before moving the base module pipeline.

It deliberately does not perform the separate cleanup of `ast/generic_functions/materialisation.rs`. That file may need a later simplification plan, but this phase changes only the API and ownership required by this boundary.

### Checklist

- [ ] Define a compiler-owned immutable input/view containing the already published generated identities and summaries a module compile may reuse.
- [ ] Define one compiler-owned generated delta containing newly completed generated identities, exact summaries and sidecars produced by a successful module transaction.
- [ ] Keep the boundary generated store, duplicate prevention, deterministic publication and boundary placement under `build_system/create_project_modules`.
- [ ] Remove semantic HIR and borrow responsibilities from the build-owned generated worklist/store APIs.
- [ ] Move generated request canonicalisation, semantic materialisation coordination and generated-summary convergence under `compiler_frontend`.
- [ ] Move `generated_summary_convergence.rs` out of the build system and delete the old file once the compiler owner is wired.
- [ ] Ensure the compiler, not Stage 0, installs summaries into base/generated HIR and decides when a borrow recheck is required.
- [ ] Replace `SourceProviderMaterialisationSet` access to mutable build stores with an immutable compiler-facing provider materialisation registry/view built from already completed providers.
- [ ] Preserve requester-local materialisation as a compiler-local case rather than making Stage 0 fake a completed provider.
- [ ] Ensure a diagnosed module publishes neither its base artefact nor any generated delta.
- [ ] Ensure a successful module publishes its base artefact and generated delta through the existing atomic boundary transaction.
- [ ] Preserve boundary-local identity rules so equal generated identities in unrelated project/package boundaries do not collide.
- [ ] Preserve recursive generic diagnostics and exact call-summary transition validation.
- [ ] Add focused tests for known-generated reuse, duplicate suppression, recursive requests, cross-package materialisation, generated borrow convergence and transactional publication.

### Phase 2 gate

- [ ] Ownership audit confirms no build-system function mutates generated/base HIR or directly reruns borrow analysis.
- [ ] Style-guide review confirms the compiler generated owner is focused and does not absorb boundary storage/publication policy.
- [ ] Generated, generic, borrow, public-interface and boundary publication tests pass, followed by full validation.

Exit state: Stage 0 owns generated availability and publication while the compiler owns generated semantic completion.

## Phase 3: Move canonical module semantic orchestration into `compiler_frontend`

### Context

With result types and generated semantics already compiler-owned, the main orchestration can move without reverse dependencies or a callback-heavy bridge.

This is the central phase. The build system should become a client that supplies one ready module job and receives one typed outcome.

### Checklist

- [ ] Introduce one compiler-owned `compile_module` service input containing prepared semantic syntax, completed provider interfaces, compiler options, immutable generated/provider materialisation views and required capability inputs.
- [ ] Move `FrontendModuleBuildContext` or replace it with a compiler-owned module compilation context.
- [ ] Move `compile_module_semantic` under the compiler owner.
- [ ] Move the pre-AST export seed and public source origin coordination that exists only to build the compiler public interface into the compiler module compilation flow.
- [ ] Keep public-interface projection implementation in `compiler_frontend/public_interface`, but make the compiler module service its production caller.
- [ ] Keep AST construction in `compiler_frontend/ast`, HIR construction in `compiler_frontend/hir` and borrow analysis in its current compiler owner.
- [ ] Make the module service sequence those owners without duplicating their implementation.
- [ ] Keep successful warnings, diagnostic identity context and string-table state inside the compiler result until the build boundary performs its deterministic merge.
- [ ] Keep `ModuleCompilationOutcome::Success`, `Diagnosed` and `CompilerError` classification at the compiler semantic boundary.
- [ ] Update directory-module compilation so it builds provider inputs, calls the compiler once and only handles success/diagnosed/infrastructure publication state.
- [ ] Update synthetic single-file compilation to use the same canonical compiler semantic service after its Stage 0 preparation path.
- [ ] Remove build-owned helpers for AST construction, HIR lowering, borrow checking, public-interface finalisation and generated semantic completion as their compiler replacements land.
- [ ] Preserve existing timing metric identities during the move. Do not redesign command or metric accounting in this phase.
- [ ] Add focused tests proving directory and single-file clients receive the same semantic outcome classes and that diagnosed providers still block consumers without partial interfaces.

### Phase 3 gate

- [ ] Ownership audit confirms `build_system/create_project_modules` no longer sequences binding -> ordering -> AST -> HIR -> borrow for a module.
- [ ] Style-guide review confirms the new compiler module orchestration reads as a short sequence of named semantic steps and large helpers live in focused child files.
- [ ] Module, public-interface, generated, borrow, single-file and graph-blocking tests pass, followed by timers-feature validation where applicable and full validation.

Exit state: the compiler is the one production owner of canonical local module semantic compilation.

## Phase 4: Reduce build `frontend_orchestration` to Stage 0 preparation ownership and delete the misleading owner

### Context

After Phase 3, the legitimate build-owned work left in `frontend_orchestration.rs` should be provider-independent source preparation scheduling, source discovery interaction, parallel file preparation policy and deterministic string-table merge behavior.

That work should be named for what it does. Keeping a file called `frontend_orchestration.rs` after semantic orchestration moves would preserve the old mental model and invite new code back into it.

### Checklist

- [ ] Inventory the exact code remaining in `frontend_orchestration.rs` after Phase 3.
- [ ] Move Stage 0 source preparation scheduling into a narrowly named owner such as `module_preparation.rs` or `source_preparation.rs`.
- [ ] Keep serial/per-file/chunked Rayon policy build-owned because it is scheduling policy, not language semantics.
- [ ] Keep actual tokenization/header-preparation semantics compiler-owned and called through one provider-independent preparation API.
- [ ] Keep incremental `ModuleSyntaxDiscovery` behavior needed for Stage 0 to consume structural provider references without a second scanner.
- [ ] Preserve deterministic merge/remap order and the single-preparation invariant.
- [ ] Remove semantic compilation context types from the build preparation module.
- [ ] Delete `frontend_orchestration.rs` once no valid owner remains there.
- [ ] Update `create_project_modules/mod.rs` documentation so its module map names Stage 0 discovery, preparation, scheduling, publication and graph outcomes only.
- [ ] Update focused tests for serial/parallel preparation selection, deterministic remapping, exactly-once source preparation and structural provider discovery.

### Phase 4 gate

- [ ] Ownership audit confirms the replacement build module contains no AST, HIR, public-interface draft or borrow semantic orchestration.
- [ ] Style-guide review confirms the preparation file has one responsibility and the old broad name is gone.
- [ ] Source preparation, Stage 0 graph and parallelism tests pass, followed by full validation.

Exit state: the old build-owned frontend orchestration owner no longer exists.

## Phase 5: Move specialised frontend clients behind compiler services and lock down raw stage APIs

### Context

The main module boundary is not fully protected while project config and direct Moth-template compilation still assemble raw stages outside `compiler_frontend`.

These clients intentionally stop earlier than normal module compilation. They should keep their specialised behavior, but the compiler must own the stage sequence they use.

This phase closes those sanctioned escape hatches and then makes the dependency rule mechanically difficult to violate.

### Checklist

- [ ] Create or expose one compiler-owned project-config compilation service for the current single-file compile-time path.
- [ ] Keep project config schema/application policy build-owned while moving tokenizer -> retained headers -> binding -> ordering -> AST orchestration behind the compiler service.
- [ ] Preserve config-specific diagnostics, authored key locations and the folded AST/value boundary without constructing HIR or borrow facts.
- [ ] Create or expose one compiler-owned direct Moth-template service that performs the current prepare -> bind -> order -> AST-fold path and returns the folded `content` result plus warnings.
- [ ] Keep HTML project source collection and project-specific output packaging outside the compiler service.
- [ ] Remove direct raw-stage assembly from `src/build_system/project_config/parsing.rs` and `src/projects/html_project/moth_template/compile.rs`.
- [ ] Inventory any other production raw-stage client outside `compiler_frontend` and either route it through an existing named compiler service or document a canonical architectural exception before keeping it.
- [ ] Narrow visibility of raw stage orchestration entry points to `compiler_frontend` where Rust module visibility permits it.
- [ ] Keep completed artefact data types visible to later link/backend consumers only where their final data is part of the documented handoff.
- [ ] Add an architecture-boundary regression test or equivalent repository check for dependency directions Rust visibility cannot encode.
- [ ] The guard must reject production Stage 0/project code reintroducing calls to raw AST construction, HIR lowering, public-interface draft construction or borrow execution.
- [ ] The guard must reject new `compiler_frontend` dependencies on `build_system` or project config container types for local semantic compilation.
- [ ] Do not make source-text layering tests the only protection. Use narrowed Rust visibility and public API shape wherever possible, with the source-level check covering dependency-direction rules that Rust cannot express.
- [ ] Add focused config-service and direct-template-service tests before removing the old clients.

### Phase 5 gate

- [ ] Ownership audit finds no production client outside `compiler_frontend` assembling a frontend stage pipeline from raw pieces unless an explicit canonical exception exists.
- [ ] Style-guide review confirms the specialised services are narrow and do not become a generic configurable pipeline framework.
- [ ] Config, direct Moth-template, architecture-boundary and full validation suites pass.

Exit state: production compiler clients choose a named compiler service, not a custom stage sequence.

## Phase 6: Prune migration residue and reconcile every documentation/reference owner

### Context

Moving ownership tends to leave old helper names, comments, roadmap paths and payload aliases behind. This phase deliberately treats deletion and documentation reconciliation as implementation work.

Do not use this phase to redesign Module AST public-state projection or split the large generic materialisation implementation. Those remain separate follow-up reviews.

### Checklist

- [ ] Search the repository for `frontend_orchestration`, `generated_summary_convergence`, old semantic context names and old build-owned artefact type paths.
- [ ] Delete stale forwarding functions, aliases, compatibility modules and migration-only adapters.
- [ ] Delete old comments that describe Stage 0 as the owner of AST/HIR/borrow sequencing.
- [ ] Review `compiler_frontend/pipeline.rs` and `compiler_frontend/mod.rs` as the public structural map and remove stage methods that no longer need crate-wide visibility.
- [ ] Review `build_system/create_project_modules/mod.rs` as the Stage 0 structural map and make its exclusions explicit.
- [ ] Update the implementation maps in `docs/compiler-design-overview.md` and `docs/build-system-design.md` to the final source paths.
- [ ] Re-read the Phase 0 canonical boundary wording against the final code and tighten any sentence that still permits external stage assembly.
- [ ] Reconcile all affected `docs/src/docs/codebase/compiler-design/**` pages with the final generated and module compilation ownership.
- [ ] Update `docs/roadmap/plans/command-timing-accounting-and-reporting-correction-plan.md` relevant-code paths and assumptions to the new owners before that plan becomes active.
- [ ] Update every other active or queued roadmap plan found by the repository reference audit.
- [ ] Update source file WHAT/WHY comments after moves rather than retaining historical location explanations.
- [ ] Compare the relevant production LOC and dependency surface against the Phase 0 baseline. Record regressions that came from new wrappers, duplicate state or unnecessary abstraction and remove them before accepting the phase.
- [ ] Do not impose an arbitrary LOC reduction target. The target is simpler ownership and less duplicated orchestration.

### Phase 6 gate

- [ ] Ownership audit confirms only one current API shape remains and repository searches find no stale semantic owner.
- [ ] Style-guide review focuses on moved-file responsibility, file size, comment accuracy, clone/copy regressions and redundant wrappers.
- [ ] Documentation checks, architecture-boundary checks and full validation pass.

Exit state: the repository teaches and implements the same compiler/build boundary with no migration layer left behind.

## Phase 7: Final cross-boundary audit, performance sanity check and roadmap release

### Context

The final phase verifies the whole boundary rather than another local slice. It also protects the next timing plan from starting on a partially migrated tree.

### Checklist

- [ ] Re-run the Phase 0 production raw-stage caller inventory and require zero unapproved external orchestration callers.
- [ ] Re-run the compiler-to-build/project dependency inventory and require no local semantic compiler dependency on build/project containers.
- [ ] Confirm canonical module compilation has one production entry and one success/diagnosed/error contract.
- [ ] Confirm Stage 0 still prepares source exactly once and consumes structural provider references before provider binding.
- [ ] Confirm a diagnosed provider publishes no partial interface and blocks only semantic consumers that require it.
- [ ] Confirm generated sidecars remain immutable boundary records and equal identities in unrelated boundaries remain independent.
- [ ] Confirm build-owned generated code performs aggregation, deduplication, storage and publication only.
- [ ] Confirm compiler-owned generated code performs materialisation, validation, borrow analysis and semantic convergence only.
- [ ] Confirm project config and direct Moth-template compilation use named compiler services.
- [ ] Confirm `frontend_orchestration.rs` and build-owned `generated_summary_convergence.rs` are absent.
- [ ] Confirm compiler artefact lane types live under the compiler owner while `ProjectCompilation`, entries and output policy remain build-owned.
- [ ] Run the current canonical frontend benchmark/timing comparison recorded in Phase 0 and investigate any material regression caused by extra cloning, remapping or repeated walks introduced by the refactor.
- [ ] Do not turn this checkpoint into a general frontend optimisation phase. Correct only regressions caused by the ownership move.
- [ ] Run the repository's complete post-test-honesty validation matrix.
- [ ] Update this plan's capsule to complete with the final validating commit and audit result.
- [ ] Mark this roadmap item complete.
- [ ] Update the roadmap so command timing accounting and reporting corrections becomes the next eligible queued plan.

### Phase 7 gate

- [ ] Broad ownership audit reports no unresolved compiler/build boundary finding.
- [ ] Broad style-guide audit reports no stale owner, compatibility shim, unjustified large moved file or comment-policy regression.
- [ ] Full validation, docs validation, architecture-boundary tests and the recorded frontend performance sanity comparison pass.

Exit state: the next roadmap plan starts from a frontend whose ownership boundary is explicit in code, documentation and tests.

---

# Validation matrix

Phase-specific targeted tests are mandatory. The final acceptance matrix must use the then-current validation commands after the test-suite honesty plan lands.

At planning time the minimum expected matrix is:

```bash
cargo fmt --all -- --check
cargo test --workspace --quiet -- --format terse
cargo test --workspace --quiet --features timers -- --format terse
cargo test --workspace --quiet --all-features -- --format terse
cargo run --quiet -- tests --audit
cargo run --quiet -- tests --terse
just validate
```

Also run the current focused owners for:

- Stage 0 filesystem/source identity
- source preparation and deterministic remapping
- directory module compilation and provider blocking
- single-file frontend compilation
- public-interface projection and closure
- generic materialisation and generated sidecars
- generated summary convergence
- borrow checker call summaries
- project config parsing/validation
- direct Moth-template compilation

The test-suite honesty plan may rename or strengthen these commands. Phase 0 must rebase this matrix instead of preserving obsolete command names.

# Completion criteria

This plan is complete only when all of the following hold:

- one compiler-owned production service owns canonical module semantic compilation
- Stage 0 schedules the service but does not sequence its semantic stages
- build-owned `frontend_orchestration.rs` no longer exists
- build-owned generated summary convergence no longer exists
- build-generated storage/publication code does not mutate HIR or run borrow analysis
- compiler semantic code does not depend on build-system stores or project config containers
- compiler-produced module artefact types live under the compiler owner
- project/build aggregation and output types remain under their existing appropriate owners
- source preparation remains exactly once and provider-independent until Stage 0 has structural graph facts
- directory and single-file module compilation use the same semantic service after preparation
- project config uses one compiler-owned AST-only service
- direct Moth-template compilation uses one compiler-owned AST-only/folded-content service
- raw stage orchestration APIs are compiler-internal where practical
- an architecture check guards dependency-direction rules that Rust visibility cannot fully encode
- canonical architecture docs explicitly forbid build/project semantic stage assembly
- style-guide policy explicitly forbids this ownership drift
- educational compiler docs teach the same generated and module compilation ownership
- queued roadmap plans reference the final paths and boundary
- no forwarding aliases, duplicate payloads or old compatibility entry points remain
- no material performance regression is introduced through extra copying or repeated semantic work
- the complete validation matrix passes

# Non-goals and follow-up boundaries

- Do not redesign Moth language semantics.
- Do not implement lifetime-region and escape validation in this plan.
- Do not redesign target partitioning or backend lowering.
- Do not perform the command timing accounting correction here. Preserve metric meaning while code moves, then let the queued timing plan own accounting semantics.
- Do not turn `CompilerFrontend` into a configurable pass manager.
- Do not introduce a general callback or plugin framework for compiler stages.
- Do not keep old and new pipeline APIs in parallel.
- Do not perform the planned deep simplification/splitting of `src/compiler_frontend/ast/generic_functions/materialisation.rs` beyond changes required to use the corrected boundary.
- Do not perform the separate Module AST environment/finalisation public-state consolidation review. In particular, this plan does not attempt to remove `ResolvedPublicTypeRootTable` or `synchronize_normalized_public_defaults` unless a narrow boundary move makes a small deletion unavoidable.
- Do not use LOC reduction as a substitute for ownership correctness.

The generic materialisation file and Module AST public-state/finalisation model should be reviewed after this ownership cleanup because this plan gives both of them a cleaner surrounding compiler boundary. Their internal simplification should not be mixed into this refactor.

# Expected end state

A new contributor or coding agent should be able to answer the ownership question from the directory structure alone:

```text
build_system/create_project_modules
    discovers
    prepares for graph construction
    schedules
    publishes

compiler_frontend
    prepares source semantics
    compiles one ready module
    completes generated semantic work
    returns sealed compiler artefacts

build_system/build and project builders
    assemble completed artefacts
    link entries
    select targets
    produce output plans
```

There should be no second place where a developer can casually add "the next frontend step" from build or project code. A new semantic stage belongs inside the compiler's module compilation service. Stage 0 only needs to know what inputs make the module ready and what complete outcome came back.
