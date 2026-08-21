# Frontend Module Compilation Ownership Cleanup Plan

## Current state

```text
WORK_ID: frontend-module-compilation-ownership-cleanup
WORK_SOURCE: docs/roadmap/plans/frontend-module-compilation-ownership-cleanup-plan.md
BASE_REVISION: f3b4178118069e857034dc2ba0e9f71864980721
STATUS: complete
CURRENT_SCOPE: all phases complete; `compiler_frontend` is the single production owner of local semantic compilation, and every stage owner is frontend-private rather than crate-visible
COMPLETED: Phase 0 re-anchor and documentation hardening, Phase 1 data-ownership move, Phases 2-3 implemented as one slice, Phase 4 preparation-owner reduction and rename, Phase 5 specialised compiler services and the architecture-boundary guard, two independent fresh-context reviews of Phases 0-5, Phase 6 residue pruning and visibility narrowing, Phase 7 cross-boundary audit and performance comparison
NEXT_ACTION: none; the constant evaluation and static control-flow specialisation plan is the head of the queued chain
VALIDATION: `just validate` exits 0 at every phase gate including Phase 7. Final state: 4403 / 17 / 779 Rust tests, 1851 integration cases, `xtask source-audit` 1198 files 0 findings, `just test-honesty-evidence` 0 hard and 0 integrity findings, `just bench-frontend-check` unchanged against a worktree at the base revision (~97ms frontend average on both)
AUDITS: compiler/build ownership, generated-function ownership, module artefact ownership, config and direct Moth-template compiler clients, canonical docs and style-guide boundary rules
BLOCKERS: none
NOTES: not yet committed. The command timing accounting plan was deleted from the roadmap on 2026-08-18, so its references here are historical; the constant-folding plan's dangling prerequisite on it was corrected in Phase 6. Per the convention this plan records for its own predecessors, this file is removed from `docs/roadmap/plans/` once the work is committed.
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

Both are complete and their plans have been removed from `docs/roadmap/plans/`.

It runs before:

1. runtime assertion messages and call-argument parser consolidation
2. constant evaluation and static control-flow optimisation
3. the remaining queued roadmap chain

The command timing accounting and reporting correction plan referenced throughout the original authoring no longer exists. It was deleted from the roadmap on 2026-08-18, after this plan was authored. Its sequencing constraint is therefore satisfied by absence; every later reference to that plan in this document is historical context, not an outstanding action.

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

# Phase 0 re-anchor findings

Recorded against `f3b4178118069e857034dc2ba0e9f71864980721` on 2026-08-20.

## Planning snapshot verification

Every shape described under `Planning snapshot and confirmed current shape` still holds. No newly added
semantic owner appeared between the planning revision and the implementation start revision. `pipeline.rs`
is 474 lines and still exposes `sort_headers`, `headers_to_ast`, `generate_hir` and `check_borrows` beside
tokenization and per-file preparation, and still constructs `CompilerFrontend` from
`crate::projects::settings::Config`.

`src/build_system/create_project_modules/source_preparation.rs` already exists but owns single-pass source
tokenization and retained dependency-clause preparation for Stage 0 discovery, not module preparation
scheduling. Phase 4 must not collide with that name.

## Production raw-stage callers outside `compiler_frontend`

| Owner | Raw stage entry points used |
|---|---|
| `src/build_system/create_project_modules/frontend_orchestration.rs` | `prepare_header_syntax`, `bind_module_headers`, `CompilerFrontend::sort_headers`, `CompilerFrontend::headers_to_ast`, `CompilerFrontend::generate_hir`, `CompilerFrontend::check_borrows`, `PublicInterfaceDraftBuilder`, `build_direct_export_seed`, `build_public_source_nominal_origin_index`, `build_public_source_trait_origin_index`, `validate_materialisation_context_templates`, `collect_module_function_link_facts` |
| `src/build_system/create_project_modules/generated_summary_convergence.rs` | `CompilerFrontend::check_borrows`, `validate_public_call_summary_transition`, direct mutation of base and generated `HirModule` call summaries |
| `src/build_system/project_config/parsing.rs` | `prepare_file_from_tokens`, `prepare_header_syntax`, `bind_module_headers`, `resolve_module_dependencies`, `Ast::new` |
| `src/projects/html_project/moth_template/compile.rs` | `prepare_header_syntax`, `bind_module_headers`, `CompilerFrontend::sort_headers`, `CompilerFrontend::headers_to_ast` |

Test callers are `src/build_system/tests/frontend_orchestration_tests.rs` and
`src/compiler_tests/frontend_pipeline_tests.rs`. Both are allowed to target stage-local APIs, but
`frontend_pipeline_tests.rs` currently reassembles the whole sequence and should follow the production
owner as it moves.

## Build-owned types that are semantically compiler module results

Declared in `src/build_system/build.rs`: `ResolvedConstFragment`, `ModuleExternalImport`,
`ModuleRootActivity`, `ModuleExecutable`, `ModuleLinkFacts`, `ModuleCompilerMetadata`, `Module`,
`GeneratedFunctionSidecar`, `CompiledModuleArtifact`, `ModuleSemanticDraft`.

Declared in `src/build_system/create_project_modules/frontend_orchestration.rs`:
`ModuleCompilationOutcome`, `FrontendModuleBuildContext`, `SourceProviderMaterialisationSet`.

Declared in `src/build_system/create_project_modules/prepared_module.rs`: `PreparedModule`, which mixes the
compiler semantic input payload with the Stage 0-only `contains_moth_template` activation fact.

Staying build-owned: `ProjectCompilation`, `EntryAssembly`, `LinkedModuleAssembly`, `ProjectEntry`,
`ProjectLinkedModule`, `BuildBootstrap`, `OutputFile`, `FileKind`, `Project`, `BuildResult`,
`ProjectBuilder`, the graph boundaries in `compiled_boundary.rs`, `ModuleArtifactStore` and
`BoundaryGeneratedFunctionStore`.

## Generated data flow at the start revision

```text
AST deferred_generic_requests
-> install_generated_request_contracts (frontend_orchestration.rs)
-> GeneratedRequestFacts -> GeneratedFunctionWorklist::register_requests (build-owned)
-> materialise_generated_request_roots / materialise_generated_request (frontend_orchestration.rs)
-> GeneratedFunctionWorklist::enter / complete with PublicCallSummary + GeneratedFunctionSidecar
-> run_generated_summary_convergence (build-owned, mutates base and generated HIR, reruns borrows)
-> GeneratedFunctionWorklist::finish -> GeneratedFunctionWorklistDelta
-> ModuleSemanticDraft.generated_worklist_delta
-> BoundaryGeneratedFunctionStore::preflight / reserve_commit / commit (atomic with the artefact)
```

`SourceProviderMaterialisationSet` currently borrows the mutable build stores `ModuleArtifactStore` and
`CompletedSourcePackageRegistry` to resolve a declaring module's materialisation context, and falls back to
the requester's own in-progress `ModuleMaterialisationPreparation`. Phase 2 replaces the store access with an
immutable compiler-facing view and keeps requester-local materialisation as a compiler-local case.

## Current targeted tests

- module preparation and file-preparation strategy: `src/build_system/tests/frontend_orchestration_tests.rs`
- module compilation and boundary waves: `src/build_system/tests/compilation_tests.rs`, `src/build_system/tests/compile_project_frontend_tests.rs`, `src/build_system/tests/create_project_modules_tests.rs`
- generated convergence: `src/build_system/tests/generated_summary_convergence_tests.rs`
- generated worklist and publication: `src/build_system/tests/generated_worklist_tests.rs`, `src/build_system/tests/module_artifact_store_tests.rs`
- artefact lanes: `src/build_system/tests/module_lane_tests.rs`
- Stage 0 source identity: `src/build_system/tests/stage0_filesystem_identity_tests.rs`
- single-file and end-to-end frontend: `src/compiler_tests/frontend_pipeline_tests.rs`
- public-interface projection: `src/compiler_frontend/public_interface/tests/`
- generic materialisation: `src/compiler_frontend/ast/generic_functions/tests/`
- direct Moth template: `src/projects/html_project/moth_template/tests.rs`
- project config parsing: `tests/cases/` config coverage plus `src/build_system/tests/build_dependency_tests.rs`
- user-visible behaviour: `tests/cases/` through `cargo run -- tests`

## Baseline validation authority

The test-suite honesty plan landed before this plan started, so the validation authority is the `justfile`
plus `docs/src/docs/codebase/style-guide/validation.mtf`, not the static command list authored with this
plan. The canonical gate is:

```bash
just validate
```

which runs clippy, the feature-lane check, `just source-audit`, `cargo test --workspace`,
`cargo run -- tests --terse`, the docs build, `xtask bench-ci` and the timers-erasure check.

The canonical non-recording frontend performance comparison for this plan is:

```bash
just bench-frontend-check
```

with `just bench-check` as the broader non-recording command.

## Roadmap references to re-check in Phase 6

- `docs/roadmap/plans/compiler-source-token-and-diagnostic-data-layout-plan.md` lines 108-117 name `frontend_orchestration.rs` and `compiler_frontend/pipeline.rs::CompilerFrontend`.
- `docs/roadmap/plans/frontend-arena-semantic-invariant-optimization-plan.md` names `frontend_orchestration.rs` twice and `compiler_frontend/pipeline.rs` once.
- `docs/roadmap/plans/post-tir-template-parser-optimization-plan.md` names `compiler_frontend/pipeline.rs`.
- `docs/roadmap/evidence/test_honesty_inventory.json` records suite paths that move with the tests.
- `docs/roadmap/plans/constant-folding-and-type-system-hot-path-optimization-plan.md` still gates itself on the deleted command timing plan. That staleness predates this refactor; record it rather than silently rewriting another plan's gate.

---

# Implementation phases

## Phase 0: Re-anchor the repository and lock the ownership contract

### Context

This plan is queued behind active work. The implementation tree may move before this plan starts. The first phase therefore verifies the real repository shape and strengthens the architectural contract before any production owner moves.

This phase should make the intended dependency direction unambiguous enough that later phases can be reviewed against a written target rather than inferred from the refactor itself.

### Checklist

- [x] Resolve the then-current `main` commit and record it as the implementation start revision in this plan.
- [x] Compare the current versions of `compiler_frontend/pipeline.rs`, `build_system/create_project_modules/`, `build_system/build.rs`, project config parsing and direct Moth-template compilation against the planning snapshot above.
- [x] Record every production caller outside `compiler_frontend` that directly invokes or imports raw binding, ordering, AST construction, HIR lowering, public-interface draft or borrow-analysis owners.
- [x] Record every build-owned type that is semantically part of the compiler module artefact or generated sidecar result.
- [x] Record the current generated request, summary, sidecar and materialisation-context data flow from requester AST through boundary publication.
- [x] Record the exact current targeted tests for module preparation, module compilation, generated convergence, public-interface publication, single-file compilation, project config and direct Moth-template compilation.
- [x] Record the current canonical benchmark/timing command used to detect gross frontend performance regressions after pure ownership moves.
- [x] Update `docs/compiler-design-overview.md` with the hardened local module compilation service boundary described above.
- [x] Update `docs/build-system-design.md` so Stage 0 compile waves clearly invoke one compiler semantic service rather than owning the stage sequence.
- [x] Update generated-function ownership wording in both canonical architecture docs.
- [x] Update config and direct Moth-template service wording in the canonical docs where it is currently permissive.
- [x] Add the explicit compiler/build layering rule to `style-guide.mtf`.
- [x] Update educational compiler-design pages that already describe this ownership boundary, including `module-artefacts-and-reuse.mtf`.
- [x] Audit queued roadmap plans for paths or assumptions that this refactor will invalidate and record the required edits in this plan before code moves.
- [x] Update this plan's current-state capsule with confirmed paths, blockers and baseline validation.

### Phase 0 gate

- [x] Ownership audit confirms the plan matches the current tree and no newly added semantic owner was missed.
- [x] Style-guide review confirms the new documentation states a concrete enforceable rule rather than only a preference for separation.
- [x] Documentation checks and full validation pass with no production behavior changes.

Exit state: the current repository is re-anchored and the accepted docs already forbid the architecture this plan is about to remove.

Phase 0 result: documentation-only. `docs/compiler-design-overview.md` gained `Canonical module compilation service`
and `Project config compilation service`, hardened generated-function ownership and a new architectural invariant.
`docs/build-system-design.md` gained the scheduling invariant, the compile-wave service call, the generated
boundary/semantic split, the Stage 0 preparation exception and the config-service client wording.
`style-guide.mtf` gained `Production layering and stage ownership`. Educational pages updated:
`module-artefacts-and-reuse.mtf`, `project-graphs-and-modules.mtf`, `templates-and-tir.mtf` and
`starting-a-build.mtf`.

## Phase 1: Move compiler semantic payloads and options to the compiler boundary

### Context

The semantic orchestration cannot move cleanly while the values it produces are declared under `build_system`. Moving the function first would either make `compiler_frontend` depend on `build_system` or require duplicate transition types. Both are worse than the current shape.

This phase fixes data ownership before control-flow ownership.

### Checklist

- [x] Add a focused compiler-owned module compilation area, expected to be `src/compiler_frontend/module_compilation/` unless Phase 0 finds a better existing owner.
- [x] Move the compiler-produced module artefact vocabulary out of `build_system/build.rs`.
- [x] Include `ModuleExecutable`, `ModuleLinkFacts`, `ModuleCompilerMetadata`, `ModuleRootActivity`, resolved const-fragment metadata, module external-import facts, `Module`, `GeneratedFunctionSidecar` and `CompiledModuleArtifact` or their current equivalents.
- [x] Move `ModuleSemanticDraft` to the compiler boundary or replace it with one compiler-owned named result that carries the same transient remap/publication facts.
- [x] Keep `ProjectCompilation`, graph boundaries, entry assembly, builders, output records and publication stores build-owned.
- [x] Replace `CompilerFrontend::new(&Config, ...)` with a compiler-owned options/input value containing only the settings the frontend actually consumes.
- [x] Remove the `crate::projects::settings::Config` dependency from `compiler_frontend/pipeline.rs`.
- [x] Split the current build-owned `PreparedModule` if needed so Stage 0-only facts such as implicit Moth-template provider activation stay build-owned while the semantic compilation payload is a compiler-owned input value.
- [x] Preserve one exact string-table ownership path through preparation, semantic compilation, deterministic merge and publication.
- [x] Remove loose duplicate active-root inputs where the same fact can be derived from the retained `FileId`, source table and source-origin table.
- [x] Update call sites directly to the new owner and delete old type definitions in the same phase.
- [x] Add or update tests for artefact remapping, lane coherence and successful publication using the compiler-owned types.

### Phase 1 gate

- [x] Ownership audit finds no compiler semantic result type left in `build_system/build.rs` solely because the old orchestration lived there.
- [x] Style-guide review confirms the new module is split by real data responsibility and does not become a generic dumping ground.
- [x] Targeted artefact/publication tests and full validation pass.

Exit state: compiler semantic values can move through compiler code without a dependency back into `build_system`.

Phase 1 result: `src/compiler_frontend/module_compilation/` now owns `options.rs` (`FrontendOptions`,
`DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS`), `prepared.rs` (`PreparedModuleInput`), `artefact.rs` (`Module`,
`ModuleExecutable`, `ModuleLinkFacts`, `ModuleCompilerMetadata`, `ModuleRootActivity`,
`ResolvedConstFragment`, `ModuleExternalImport`, `CompiledModuleArtifact`), `generated.rs`
(`GeneratedFunctionSidecar`, `CompletedGeneratedFunction`, `GeneratedFunctionDelta`,
`validate_completed_generated_record`) and `outcome.rs` (`ModuleCompilationOutcome`,
`ModuleSemanticResult`).

`ModuleSemanticDraft` was renamed to `ModuleSemanticResult` and `GeneratedFunctionWorklistDelta` to
`GeneratedFunctionDelta`; both old definitions are deleted. `build_system/build.rs` dropped from 1,259 to
998 lines and now declares only project aggregation, entry assembly, builder and output types.
`create_project_modules/prepared_module.rs` is the Stage 0 record pairing `PreparedModuleInput` with the
build-owned `contains_moth_template` scheduling fact. `Config::frontend_options()` is the one project-side
projection into compiler options, covered by `src/projects/tests/settings_tests.rs`.

The duplicate active-root path was removed in two places: `compile_module_semantic` no longer takes an
`entry_file_path` argument and `ModuleCompilationJob` no longer stores `entry_point`. Both now read
`PreparedModuleInput::entry_file_path()`, asserted in `frontend_orchestration_tests.rs`.

## Phase 2: Separate generated boundary publication from generated semantic completion

### Context

Generated functions are the hardest seam in the move. Boundary-wide deduplication is genuinely Stage 0 work, but generated HIR materialisation, borrow checking and summary convergence are compiler semantics.

This phase establishes that split before moving the base module pipeline.

It deliberately does not perform the separate cleanup of `ast/generic_functions/materialisation.rs`. That file may need a later simplification plan, but this phase changes only the API and ownership required by this boundary.

### Checklist

- [x] Define a compiler-owned immutable input/view containing the already published generated identities and summaries a module compile may reuse.
- [x] Define one compiler-owned generated delta containing newly completed generated identities, exact summaries and sidecars produced by a successful module transaction.
- [x] Keep the boundary generated store, duplicate prevention, deterministic publication and boundary placement under `build_system/create_project_modules`.
- [x] Remove semantic HIR and borrow responsibilities from the build-owned generated worklist/store APIs.
- [x] Move generated request canonicalisation, semantic materialisation coordination and generated-summary convergence under `compiler_frontend`.
- [x] Move `generated_summary_convergence.rs` out of the build system and delete the old file once the compiler owner is wired.
- [x] Ensure the compiler, not Stage 0, installs summaries into base/generated HIR and decides when a borrow recheck is required.
- [x] Replace `SourceProviderMaterialisationSet` access to mutable build stores with an immutable compiler-facing provider materialisation registry/view built from already completed providers.
- [x] Preserve requester-local materialisation as a compiler-local case rather than making Stage 0 fake a completed provider.
- [x] Ensure a diagnosed module publishes neither its base artefact nor any generated delta.
- [x] Ensure a successful module publishes its base artefact and generated delta through the existing atomic boundary transaction.
- [x] Preserve boundary-local identity rules so equal generated identities in unrelated project/package boundaries do not collide.
- [x] Preserve recursive generic diagnostics and exact call-summary transition validation.
- [x] Add focused tests for known-generated reuse, duplicate suppression, recursive requests, cross-package materialisation, generated borrow convergence and transactional publication.
  - Cross-package materialisation had no focused test until the Phase 5 review added
    `generated/tests/provider_materialisations_tests.rs`. Until then it was covered only end to end
    by `tests/cases/generic_receiver_source_package_facade_success/`.

### Phase 2 gate

- [x] Ownership audit confirms no build-system function mutates generated/base HIR or directly reruns borrow analysis.
- [x] Style-guide review confirms the compiler generated owner is focused and does not absorb boundary storage/publication policy.
- [x] Generated, generic, borrow, public-interface and boundary publication tests pass, followed by full validation.

Exit state: Stage 0 owns generated availability and publication while the compiler owns generated semantic completion.

## Phase 3: Move canonical module semantic orchestration into `compiler_frontend`

### Context

With result types and generated semantics already compiler-owned, the main orchestration can move without reverse dependencies or a callback-heavy bridge.

This is the central phase. The build system should become a client that supplies one ready module job and receives one typed outcome.

### Checklist

- [x] Introduce one compiler-owned `compile_module` service input containing prepared semantic syntax, completed provider interfaces, compiler options, immutable generated/provider materialisation views and required capability inputs.
- [x] Move `FrontendModuleBuildContext` or replace it with a compiler-owned module compilation context.
- [x] Move `compile_module_semantic` under the compiler owner.
- [x] Move the pre-AST export seed and public source origin coordination that exists only to build the compiler public interface into the compiler module compilation flow.
- [x] Keep public-interface projection implementation in `compiler_frontend/public_interface`, but make the compiler module service its production caller.
- [x] Keep AST construction in `compiler_frontend/ast`, HIR construction in `compiler_frontend/hir` and borrow analysis in its current compiler owner.
- [x] Make the module service sequence those owners without duplicating their implementation.
- [x] Keep successful warnings, diagnostic identity context and string-table state inside the compiler result until the build boundary performs its deterministic merge.
- [x] Keep `ModuleCompilationOutcome::Success`, `Diagnosed` and `CompilerError` classification at the compiler semantic boundary.
- [x] Update directory-module compilation so it builds provider inputs, calls the compiler once and only handles success/diagnosed/infrastructure publication state.
- [x] Update synthetic single-file compilation to use the same canonical compiler semantic service after its Stage 0 preparation path.
- [x] Remove build-owned helpers for AST construction, HIR lowering, borrow checking, public-interface finalisation and generated semantic completion as their compiler replacements land.
- [x] Preserve existing timing metric identities during the move. Do not redesign command or metric accounting in this phase.
- [x] Add focused tests proving directory and single-file clients receive the same semantic outcome classes and that diagnosed providers still block consumers without partial interfaces.

### Phase 3 gate

- [x] Ownership audit confirms `build_system/create_project_modules` no longer sequences binding -> ordering -> AST -> HIR -> borrow for a module.
- [x] Style-guide review confirms the new compiler module orchestration reads as a short sequence of named semantic steps and large helpers live in focused child files.
- [x] Module, public-interface, generated, borrow, single-file and graph-blocking tests pass, followed by timers-feature validation where applicable and full validation.

Exit state: the compiler is the one production owner of canonical local module semantic compilation.


---

## Phases 2 and 3 were implemented as one slice

Phase 2 moves generated semantic completion into the compiler, and Phase 3 moves the module
compilation service that drives it. Splitting them would have required a transitional compiler-side
context that Phase 3 immediately replaced, which `AGENTS.md` forbids. They were therefore implemented
as one coherent slice with both gates applied at the end.

### What moved

`src/compiler_frontend/module_compilation/` gained:

- `context.rs`: `ModuleCompilationContext`, replacing the deleted `FrontendModuleBuildContext`
- `service.rs`: `compile_module`, the one production owner of binding -> ordering -> AST ->
  public-interface projection -> HIR -> borrow -> generated completion -> interface closure
- `stages.rs`: the warning-preserving `lower_hir` and `check_borrows` wrappers both the service and
  generated materialisation use
- `external_imports.rs`: provider and builder runtime import candidates
- `generated/artefacts.rs`, `generated/known.rs`, `generated/transaction.rs`,
  `generated/requests.rs`, `generated/provider_materialisations.rs`,
  `generated/materialisation.rs`, `generated/convergence.rs`

`build_system/create_project_modules/generated_summary_convergence.rs` is deleted.
`generated_worklist.rs` is renamed `generated_store.rs` and reduced from 521 to 136 lines: it owns
preflight, commit, sidecar storage and the `known_generated()` view, and nothing else.

### Ownership decisions worth recording

- `GeneratedFunctionWorklist` became the compiler-owned `GeneratedFunctionTransaction`. The boundary
  store lends it a `KnownGeneratedFunctions` view built from its own records and identity index, so
  the compiler reads published work without touching a build store.
- `SourceProviderMaterialisationSet` is replaced by `ProviderMaterialisationRegistry`, a
  compiler-owned identity-keyed map the build system populates. `ModuleCompilerMetadata`'s frozen
  materialisation context is now an `Arc`, so registry entries keep resolving as the artefact store
  grows behind them. `seed_completed_package_materialisations` seeds each boundary from completed
  packages; `publish_module_and_generated` extends it as each module publishes.
- `ModuleArtifactStore` keeps its own declaration index for publication provenance and duplicate
  detection. That is build policy with precise artefact/row diagnostics, distinct from the
  compiler's template lookup.
- Test ownership followed the code: `generated/tests/transaction_tests.rs` and
  `generated/tests/convergence_tests.rs` attach to their compiler owners, `generated/tests/fixtures.rs`
  is the shared fixture owner, and `build_system/tests/generated_store_tests.rs` keeps only the
  build-owned publication tests.

### Deferred to later phases

- `service.rs` is 766 lines and `generated/convergence.rs` is 844. Both are under the style guide's
  ~2,000-line guidance, but Phase 6 should review whether `compile_module` reads as named steps.
- Focused tests proving directory and single-file clients receive the same outcome classes are
  Phase 6/7 work; existing suites already cover both paths end to end.

## Self-audit corrections applied before Phase 4

A correctness and style pass over the Phases 0-3 result found no defect in the moved semantics. The
two highest-risk changes were verified rather than assumed:

- Deriving the entry file from `PreparedModuleInput::entry_file_path()` instead of a stored
  `ModuleCompilationJob::entry_point` is exactly behaviour-preserving. `SourceFileTable` keys
  `canonical_to_id` with the same `PathBuf` it stores as `canonical_os_path`, so the
  `path -> FileId -> path` round-trip is identity whenever the lookup succeeds, and preparation
  already fails when it does not.
- Wrapping `materialisation_context` in `Arc` dropped no string-ID remapping.
  `ModuleCompilerMetadata::remap_string_ids` never remapped that field at `f3b41781` either; the
  materialisation context owns self-contained strings and stable identities.

Five corrections were applied:

- `install_exact_concrete_call_summaries` returned a `changed` flag no caller ever read. The return
  type is now `Result<(), CompilerError>` and the dead local is gone, which also removed the
  `let _ =` at one of its two call sites that had made the two sites look different.
- `service.rs` built module external-import candidates inline with the type spelled as a full
  `crate::...` path three times, while `external_imports.rs` already existed as the named owner for
  exactly that data and claimed to cover "one compiled module or generated sidecar". The collection
  moved into `collect_external_import_candidates_for_source_files`, so the file now owns both
  candidate shapes and `compile_module` reads as one named step.
- `ProviderMaterialisationRegistry::publish` documented replacement-on-duplicate as reachable "when
  a project module republishes over a completed package seed". It is not:
  `GeneratedDeclarationIdentity` carries its owning package and module origin, and all three
  producers prove uniqueness first. The comment now states the real invariant.
- `GeneratedFunctionTransaction::complete` cloned the same identity twice.
- Two spellings of the same borrowed-registry clone were unified.

Challenged and left unchanged:

- `#[cfg(test)]` accessors on production stores (`push_completed_for_test`,
  `materialisation_context_for`, `BoundaryGeneratedFunctionStore::publish`) match the existing
  convention in `compiled_boundary.rs`, and their test modules are siblings under
  `build_system/tests` rather than child modules, so they cannot reach private state another way.
- The duplicated request-materialisation loop in `materialisation.rs` predates this plan and
  factoring it out would need seven parameters to express which context and compiler each loop
  drives, which is worse than the repetition.

## Phase 4: Reduce build `frontend_orchestration` to Stage 0 preparation ownership and delete the misleading owner

### Context

After Phase 3, the legitimate build-owned work left in `frontend_orchestration.rs` should be provider-independent source preparation scheduling, source discovery interaction, parallel file preparation policy and deterministic string-table merge behavior.

That work should be named for what it does. Keeping a file called `frontend_orchestration.rs` after semantic orchestration moves would preserve the old mental model and invite new code back into it.

### Checklist

- [x] Inventory the exact code remaining in `frontend_orchestration.rs` after Phase 3.
- [x] Move Stage 0 source preparation scheduling into a narrowly named owner such as `module_preparation.rs` or `source_preparation.rs`.
- [x] Keep serial/per-file/chunked Rayon policy build-owned because it is scheduling policy, not language semantics.
- [x] Keep actual tokenization/header-preparation semantics compiler-owned and called through one provider-independent preparation API.
- [x] Keep incremental `ModuleSyntaxDiscovery` behavior needed for Stage 0 to consume structural provider references without a second scanner.
- [x] Preserve deterministic merge/remap order and the single-preparation invariant.
- [x] Remove semantic compilation context types from the build preparation module.
- [x] Delete `frontend_orchestration.rs` once no valid owner remains there.
- [x] Update `create_project_modules/mod.rs` documentation so its module map names Stage 0 discovery, preparation, scheduling, publication and graph outcomes only.
- [x] Update focused tests for serial/parallel preparation selection, deterministic remapping, exactly-once source preparation and structural provider discovery.

### Phase 4 gate

- [x] Ownership audit confirms the replacement build module contains no AST, HIR, public-interface draft or borrow semantic orchestration.
- [x] Style-guide review confirms the preparation file has one responsibility and the old broad name is gone.
- [x] Source preparation, Stage 0 graph and parallelism tests pass, followed by full validation.

Exit state: the old build-owned frontend orchestration owner no longer exists.

Result: `frontend_orchestration.rs` became `module_preparation.rs` (1173 lines, preparation only) and
`frontend_orchestration_tests.rs` became `module_preparation_tests.rs`. The rename is the whole
change: Phase 3 had already emptied the file of semantic orchestration. The only remaining call-site
mention of a semantic stage is a doc comment naming `bind_module_headers` as the downstream consumer
of retained syntax. Three prose references to the deleted `FrontendModuleBuildContext` and
`compile_module_semantic` owners survived this phase and were corrected during the Phase 5 review. `create_project_modules/mod.rs`
now groups its map under discovery/structure, source preparation, scheduling/publication and
diagnostics, and states that local semantic compilation is not owned there.

`source_preparation.rs` kept its name: it owns single-pass per-file tokenisation, while
`module_preparation.rs` owns the per-module scheduling of that work. Its own file documentation
already used the term "final module preparation" for the consumer, so the two names now match the
vocabulary the code was already using.

`xtask/src/timers_erasure_check.rs` names the preparation file in one test fixture string and was
refreshed with the rename. An earlier note here claimed `just validate` would otherwise have failed;
that was wrong. The fixture path is only matched against literal allowlists and is never resolved
against the filesystem, so the stale string would have kept passing while naming a file that no
longer exists.

Two tests in `module_preparation_tests.rs` deliberately call `compile_module`. They own the
preparation/semantic seam itself — that retained syntax reaches semantic compilation without a
second file preparation, and that a Stage 0 root role reaches interface projection intact — so they
stay, and the module documentation now says so instead of claiming the file contains no semantic
coverage.

## Phase 5: Move specialised frontend clients behind compiler services and lock down raw stage APIs

### Context

The main module boundary is not fully protected while project config and direct Moth-template compilation still assemble raw stages outside `compiler_frontend`.

These clients intentionally stop earlier than normal module compilation. They should keep their specialised behavior, but the compiler must own the stage sequence they use.

This phase closes those sanctioned escape hatches and then makes the dependency rule mechanically difficult to violate.

### Checklist

- [x] Create or expose one compiler-owned project-config compilation service for the current single-file compile-time path.
- [x] Keep project config schema/application policy build-owned while moving tokenizer -> retained headers -> binding -> ordering -> AST orchestration behind the compiler service.
- [x] Preserve config-specific diagnostics, authored key locations and the folded AST/value boundary without constructing HIR or borrow facts.
- [x] Create or expose one compiler-owned direct Moth-template service that performs the current prepare -> bind -> order -> AST-fold path and returns the folded `content` result plus warnings.
- [x] Keep HTML project source collection and project-specific output packaging outside the compiler service.
- [x] Remove direct raw-stage assembly from `src/build_system/project_config/parsing.rs` and `src/projects/html_project/moth_template/compile.rs`.
- [x] Inventory any other production raw-stage client outside `compiler_frontend` and either route it through an existing named compiler service or document a canonical architectural exception before keeping it.
- [x] Narrow visibility of raw stage orchestration entry points to `compiler_frontend` where Rust module visibility permits it.
- [x] Keep completed artefact data types visible to later link/backend consumers only where their final data is part of the documented handoff.
- [x] Add an architecture-boundary regression test or equivalent repository check for dependency directions Rust visibility cannot encode.
- [x] The guard must reject production Stage 0/project code reintroducing calls to raw AST construction, HIR lowering, public-interface draft construction or borrow execution.
- [x] The guard must reject new `compiler_frontend` dependencies on `build_system` or project config container types for local semantic compilation.
- [x] Do not make source-text layering tests the only protection. Use narrowed Rust visibility and public API shape wherever possible, with the source-level check covering dependency-direction rules that Rust cannot express.
- [x] Add focused config-service and direct-template-service tests before removing the old clients.

### Phase 5 gate

- [x] Ownership audit finds no production client outside `compiler_frontend` assembling a frontend stage pipeline from raw pieces unless an explicit canonical exception exists.
- [x] Style-guide review confirms the specialised services are narrow and do not become a generic configurable pipeline framework.
- [x] Config, direct Moth-template, architecture-boundary and full validation suites pass.

Exit state: production compiler clients choose a named compiler service, not a custom stage sequence.

Phase 5 result: `single_source_compilation` is the compiler owner of both short paths.
`config.rs` owns the `config.moth` stage sequence, its dialect surface rules, duplicate-key
reclassification and authored key-name spans; `build_system/project_config.rs` keeps locating,
reading, validating and applying, and `parsing.rs` is deleted. `moth_template.rs` owns the direct
`.mtf` fold; `projects/html_project/moth_template/compile.rs` keeps request normalization, the
project style vocabulary and output packaging. `CompilerFrontend::set_source_files`, `sort_headers`,
`headers_to_ast`, `generate_hir` and `check_borrows` narrowed from `pub` to `pub(crate)`, joined by
`Ast::new`, `hir_builder::lower_module` and the `module_compilation` re-exports during the Phase 5
review. This is a declaration correction, not a reachability change: `mod compiler_frontend;` is
private at the crate root, so `pub` inside it was already capped at crate visibility and build code
could always reach these methods. `xtask/src/architecture_boundary.rs` adds two source-audit rules
covering the direction Rust cannot encode, verified to fire on both pre-Phase-5 escape hatches and
reporting nothing on the current tree.

Phase 5 recorded that "tighter than `pub(crate)` is not expressible while the frontend's own tests
drive single stages". Phase 6 disproved that: the one test outside `compiler_frontend` that drove
semantic stages was a compiler-frontend test living in the wrong tree, and moving it made every
stage owner `pub(in crate::compiler_frontend)` or narrower.


## Independent review of Phases 0-5

Two fresh-context reviews audited the whole diff against `f3b41781` using the `AGENTS.md` Final
audit steps. Both independently confirmed the move is behaviour-preserving: every relocated function
was diffed against its pre-change body with no silent change to stage order, diagnostic sets or
ordering, error-versus-warning routing, string-table merge and remap order, or sort and dedup order.
Their findings were about the guard, comment accuracy and residue. Applied:

- **The guard was bypassable.** `CompilerFrontend::sort_headers` is a one-line wrapper over the
  banned `resolve_module_dependencies`, and was itself one of the pre-Phase-5 escape-hatch entry
  points. Added, along with the pre-AST public-interface projection owners and the generated
  completion owners `style-guide.mtf` names directly ("no build-owned function installs call
  summaries"). A braced `use ...settings::{Config, ...}` also evaded the container rule; it is now
  matched, and the remaining multi-line limit is stated in the guard's own documentation.
- **The guard exempted a production subtree.** `is_test_source` skipped all of
  `src/compiler_tests/`, which also holds `integration_test_runner` — production code shipped
  without `#[cfg(test)]`. The directory clause was removed; the one harness test that drives stages
  directly was already covered by the filename rule, so the exemption protected nothing.
- **`ProviderMaterialisationRegistry::publish` documented a proof that does not run.**
  `ProjectFrontendCompilation::new` is the final boundary handoff, not a publication preflight, so a
  cross-lane collision does reach the replacing insert. The comment now says so, records that the
  result matches the pre-registry lookup order, and states that seeding packages before modules
  publish is load-bearing.
- **Cross-package materialisation had no focused test**, despite a ticked Phase 2 box.
  `generated/tests/provider_materialisations_tests.rs` now pins row resolution and the collision
  order. `PublishedBoundary` moved into the shared generated fixture owner, which also removed the
  compiler convergence tests' two reverse dependencies on the build-owned generated store.
- **Three plan claims were false or overstated** and were corrected in place: the
  `timers_erasure_check.rs` edit could not have failed `just validate`, the Phase 4 ownership grep
  missed three prose references to deleted owners, and the Phase 5 visibility narrowing changed no
  reachability because `mod compiler_frontend;` is private at the crate root.
- **Canonical docs assigned generated request deduplication to the build system** while
  `transaction.rs` does it. Reworded in `compiler-design-overview.md`, `build-system-design.md` and
  `module-artefacts-and-reuse.mtf`: the build system owns the published set, the compiler
  deduplicates against it.
- Residue and shape: stale references to `FrontendModuleBuildContext`, `compile_module_semantic`
  and the removed worklist vocabulary; `FrontendOptions::from_origin`, a single-caller helper whose
  doc contradicted the real projection in `Config::frontend_options`; the dead `#[cfg(test)]`
  accessor `ModuleArtifactStore::materialisation_context_for`, whose one test now asserts the same
  invariant through the production accessor; `Ast::new`, `hir_builder::lower_module` and the
  `module_compilation` re-exports narrowed to `pub(crate)` for consistency with Phase 5; and
  `compiler_frontend/mod.rs`, which had no module map at all, given one that names the three
  production entry points.

Deferred with reasons rather than fixed here:

- `compile_module` is 476 lines around a 395-line closure whose named steps have large unnumbered
  gaps. It is a byte-for-byte structural carry-over, and Phase 6 already owns the review.
- `src/compiler_tests/frontend_pipeline_tests.rs` still hand-assembles the canonical sequence and
  has drifted from it (no public-interface projection, no generated completion). Routing it through
  `compile_module` is a test change Phase 6 should own with the rest of the residue pruning.
- `docs/roadmap/audit-log.md` was not touched. The `tests.support` Redundancy cell is already `P`,
  and moving one helper into an existing shared fixture owner consolidates toward fewer duplicate
  owners rather than invalidating recorded coverage.

`just validate` passes after the applied fixes.

## Phase 6: Prune migration residue and reconcile every documentation/reference owner

### Context

Moving ownership tends to leave old helper names, comments, roadmap paths and payload aliases behind. This phase deliberately treats deletion and documentation reconciliation as implementation work.

Do not use this phase to redesign Module AST public-state projection or split the large generic materialisation implementation. Those remain separate follow-up reviews.

### Checklist

- [x] Search the repository for `frontend_orchestration`, `generated_summary_convergence`, old semantic context names and old build-owned artefact type paths.
- [x] Delete stale forwarding functions, aliases, compatibility modules and migration-only adapters.
- [x] Delete old comments that describe Stage 0 as the owner of AST/HIR/borrow sequencing.
- [x] Review `compiler_frontend/pipeline.rs` and `compiler_frontend/mod.rs` as the public structural map and remove stage methods that no longer need crate-wide visibility.
- [x] Review `build_system/create_project_modules/mod.rs` as the Stage 0 structural map and make its exclusions explicit.
- [x] Update the implementation maps in `docs/compiler-design-overview.md` and `docs/build-system-design.md` to the final source paths.
- [x] Re-read the Phase 0 canonical boundary wording against the final code and tighten any sentence that still permits external stage assembly.
- [x] Reconcile all affected `docs/src/docs/codebase/compiler-design/**` pages with the final generated and module compilation ownership.
- [x] Update `docs/roadmap/plans/command-timing-accounting-and-reporting-correction-plan.md` relevant-code paths and assumptions to the new owners before that plan becomes active.
- [x] Update every other active or queued roadmap plan found by the repository reference audit.
- [x] Update source file WHAT/WHY comments after moves rather than retaining historical location explanations.
- [x] Compare the relevant production LOC and dependency surface against the Phase 0 baseline. Record regressions that came from new wrappers, duplicate state or unnecessary abstraction and remove them before accepting the phase.
- [x] Do not impose an arbitrary LOC reduction target. The target is simpler ownership and less duplicated orchestration.

### Phase 6 gate

- [x] Ownership audit confirms only one current API shape remains and repository searches find no stale semantic owner.
- [x] Style-guide review focuses on moved-file responsibility, file size, comment accuracy, clone/copy regressions and redundant wrappers.
- [x] Documentation checks, architecture-boundary checks and full validation pass.

Exit state: the repository teaches and implements the same compiler/build boundary with no migration layer left behind.

Phase 6 result: the boundary is now carried by Rust visibility, not by prose plus a text guard.

**Residue.** No `src/` file names `frontend_orchestration` or a build-owned
`generated_summary_convergence`; the remaining hits are this plan's own history. Seven comments
still called the generated transaction "the build-owned worklist" — three in the AST emitter, one in
`AstBuildResult`, one in `GenericFunctionInstantiationRequest`, one benchmark-counter note and one
test assertion message. All now name `GeneratedFunctionTransaction`. `compilation.rs` no longer
re-teaches the compiler's stage list from Stage 0; it states the fact Stage 0 owns (provider
readiness) and points at `compile_module`. `check_borrows_with_warnings` in `convergence.rs` was a
byte-for-byte duplicate of `stages::check_borrows` and is deleted.

**Visibility.** Every one of the sixteen stage owners the guard names is now
`pub(in crate::compiler_frontend)` or narrower — `bind_module_headers`,
`resolve_module_dependencies`, `sort_headers`, `AstBuildInput`, `AstBuildContext`, `Ast::new`,
`headers_to_ast`, `hir_builder`, `lower_module`, `generate_hir`, `PublicInterfaceDraftBuilder`, the
three export-projection index builders, `check_borrows`, `install_exact_concrete_call_summaries`,
`materialise_generated_request_roots` and `run_generated_summary_convergence`. `module_compilation`
exposes only `artefact` and `generated` by path (build and project tests construct artefact lanes
and generated fixtures); `context`, `options`, `outcome`, `prepared`, `service`, `stages` and
`external_imports` are private behind the module map, as are every `generated` submodule and both
`single_source_compilation` services. This is a real reachability change, unlike Phase 5's.

Two things made it possible. `src/compiler_tests/frontend_pipeline_tests.rs` was a compiler-frontend
stage test in the wrong tree; it moved to `src/compiler_frontend/tests/`, and its header now says it
is not the canonical sequence and names `compile_module` as the owner of the stages it skips. And
one Stage 0 merge test called `bind_module_headers` to observe string identities that
`prepare_header_syntax` already produced — a provider-dependent stage run for facts that exist one
stage earlier. It now asserts on the prepared syntax.

The guard's own WHY was rewritten to match: it is no longer a substitute for visibility but a
tripwire on the edit that would widen one of these declarations back, which is legal Rust and
otherwise silent. The reverse rule keeps its original justification — nothing in the module tree
stops `compiler_frontend` from importing `crate::build_system`.

**Shape.** `compile_module` was 476 lines wrapping a 395-line IIFE. The closure body moved verbatim
into `run_semantic_stages`, taking one named `SemanticStageInputs` bundle. `compile_module` is now
92 lines reading as setup, one stage run, one classification; the 411-line stage sequence is a
linear numbered sequence that must be read in order and is not split further. Test counts are
unchanged at 4403 / 17 / 779 and 1851 integration cases.

**Documentation.** The compiler implementation map led with "Frontend orchestration:
`pipeline.rs`", which has been false since Phase 3; it now separates production entry points from
stage owners and names the boundary guard. Two shapes had drifted from the code and were corrected:
`ModuleCompilationOutcome::Success` carries `ModuleSemanticResult`, not the stored artefact — the
distinction is the atomic-publication design — and `CompiledModuleArtifact` pairs `Module` with the
interface rather than holding four flat lanes. `ModuleFingerprints` is marked planned, because no
such lane exists. `## Generated-function worklist` became `## Generated-function boundary` in
`build-system-design.md`, with both cross-reference tables updated, and the claim that cross-module
requests "converge through the build-system worklist ... by scheduling further compiler module jobs"
was replaced: `compile_module_waves` iterates dependency waves once, and every request converges
inside the requesting module's own transaction. `compile-time-semantics.mtf` still assigned request
deduplication to the build system, the same error the Phase 5 review fixed in the canonical docs.

**References.** `compiler-source-token-and-diagnostic-data-layout-plan.md`,
`frontend-arena-semantic-invariant-optimization-plan.md` and
`post-tir-template-parser-optimization-plan.md` were repointed at the current owners.
`constant-folding-and-type-system-hot-path-optimization-plan.md` gated itself on a plan file deleted
on 2026-08-18; the timing requirement it carried is now stated as that plan's own Phase 0
prerequisite, so the gate is satisfiable rather than dangling. The command-timing plan checklist
item above is satisfied by that file's absence. `docs/roadmap/evidence/test_honesty_inventory.json`
was not hand-edited: it is generated, so `just test-honesty-evidence` regenerated it, and it now
records the current tree with no deleted owners and the moved test at its new path.

**Size.** Production LOC excluding all test files: 268,490 at `f3b41781` to 269,437, up 947 lines
(+0.35%). 205 of those are `xtask/src/architecture_boundary.rs`, a Phase 5 deliverable that replaced
nothing. The rest is the documentation cost of splitting one 2,726-line file into named owners: the
four replaced owners carried 10.4% comment lines, `module_compilation` carries 16.5% and
`single_source_compilation` 20.6%, which is roughly 215 lines, plus about twenty new file headers
and two module maps. No wrapper, duplicate payload or parallel state survived the audit, and clones
fell from 94 to 87 across the moved owners, including the eleven per-source `string_table.clone()`
calls the old direct template path made. `stages.rs` was kept after review: two eight-line
warning-merging wrappers over four call sites, which is what lets the stage sequence read as named
steps.

## Phase 7: Final cross-boundary audit, performance sanity check and roadmap release

### Context

The final phase verifies the whole boundary rather than another local slice. It also protects the next timing plan from starting on a partially migrated tree.

### Checklist

- [x] Re-run the Phase 0 production raw-stage caller inventory and require zero unapproved external orchestration callers.
- [x] Re-run the compiler-to-build/project dependency inventory and require no local semantic compiler dependency on build/project containers.
- [x] Confirm canonical module compilation has one production entry and one success/diagnosed/error contract.
- [x] Confirm Stage 0 still prepares source exactly once and consumes structural provider references before provider binding.
- [x] Confirm a diagnosed provider publishes no partial interface and blocks only semantic consumers that require it.
- [x] Confirm generated sidecars remain immutable boundary records and equal identities in unrelated boundaries remain independent.
- [x] Confirm build-owned generated code performs aggregation, deduplication, storage and publication only.
- [x] Confirm compiler-owned generated code performs materialisation, validation, borrow analysis and semantic convergence only.
- [x] Confirm project config and direct Moth-template compilation use named compiler services.
- [x] Confirm `frontend_orchestration.rs` and build-owned `generated_summary_convergence.rs` are absent.
- [x] Confirm compiler artefact lane types live under the compiler owner while `ProjectCompilation`, entries and output policy remain build-owned.
- [x] Run the current canonical frontend benchmark/timing comparison recorded in Phase 0 and investigate any material regression caused by extra cloning, remapping or repeated walks introduced by the refactor.
- [x] Do not turn this checkpoint into a general frontend optimisation phase. Correct only regressions caused by the ownership move.
- [x] Run the repository's complete post-test-honesty validation matrix.
- [x] Update this plan's capsule to complete with the final validating commit and audit result.
- [x] Mark this roadmap item complete.
- [x] Update the roadmap so command timing accounting and reporting corrections becomes the next eligible queued plan.

### Phase 7 gate

- [x] Broad ownership audit reports no unresolved compiler/build boundary finding.
- [x] Broad style-guide audit reports no stale owner, compatibility shim, unjustified large moved file or comment-policy regression.
- [x] Full validation, docs validation, architecture-boundary tests and the recorded frontend performance sanity comparison pass.

Exit state: the next roadmap plan starts from a frontend whose ownership boundary is explicit in code, documentation and tests.

Phase 7 result: every inventory the plan opened with now returns zero.

**Raw-stage callers outside `compiler_frontend`.** Two hits across all production source, both
approved. `prepare_header_syntax` in `module_preparation.rs` is the documented Stage 0 exception,
which ends at prepared syntax. `lower_module` in `backends/js/emitter.rs` is `JsEmitter::lower_module`,
an unrelated method the guard's own comment already accounts for. The Phase 0 table listed four
owners assembling twelve stage entry points between them; none remains.

**Compiler dependency on build or project containers.** Zero production hits for
`crate::build_system` or `settings::Config` under `src/compiler_frontend/`.

**One entry, one contract.** `compile_module` has one definition and two logical call sites, both in
`compilation.rs` — the single-file synthetic path and the directory wave path — each a
`#[cfg(feature = "timers")]` pair. It returns `Result<ModuleCompilationOutcome, CompilerError>` and
nothing else classifies a module result.

**Stage 0 invariants.** Exactly-once preparation is pinned by
`serial_chunk_local_preparation_counts_each_selected_source_once` and by the
`file_frontend_prepare_count_for_path_for_test` assertions in `create_project_modules_tests.rs`,
which check the counter end to end for entry and helper files. Structural provider references are
read from the preparation result in `source_discovery.rs` before any provider interface exists.

**Diagnosed providers.** `diagnosed_provider_retains_independent_successful_module`,
`project_consumers_blocked_by_diagnosed_source_package_are_not_infrastructure_errors` and
`directory_graph_retains_independent_diagnostics_without_blocked_consumer_cascades` cover the
no-partial-interface and blocks-only-required-consumers contracts.

**Generated boundaries.** `equal_generated_identities_publish_across_independent_boundaries` and
`independent_packages_publish_equal_generated_identities_in_any_order` pin identity independence
across unrelated boundaries. `generated_store.rs` exposes exactly `known_generated`, `preflight`,
`commit`, `reserve_commit`, `publish`, `sidecars` and `sidecar_at`: lending the published set,
storing and publishing. It has no deduplication entry point, which matches the corrected docs — the
compiler deduplicates against the lent set inside `transaction.rs`. The compiler's `generated/`
owns request canonicalisation, materialisation, validation, borrow analysis and convergence, and
nothing else.

**Files absent.** `frontend_orchestration.rs`, `generated_summary_convergence.rs`,
`generated_worklist.rs` and `project_config/parsing.rs` are all gone.

**Artefact lanes.** All eight lane types — `Module`, `CompiledModuleArtifact`, `ModuleExecutable`,
`ModuleLinkFacts`, `ModuleCompilerMetadata`, `ModuleRootActivity`, `ResolvedConstFragment`,
`ModuleExternalImport` — live in `compiler_frontend/module_compilation/artefact.rs`.
`ProjectCompilation`, `EntryAssembly`, `ProjectEntry` and `OutputFile` stay in `build_system/build.rs`.

**Performance.** Phase 0 recorded the command but no measurement, and recorded benchmark history is
local-data-ignored, so there was no stored figure to diff. `just bench-frontend-check` was therefore
run on a git worktree at `f3b41781` and on the current tree, same machine, same session:

| | baseline `f3b41781` | current |
|---|---|---|
| frontend avg, 31 cases | ~97ms | ~97ms |
| AST const-template parse | ~336ms | ~336ms |
| directory compile | ~119ms | ~117ms |
| frontend stage | ~89ms | ~88ms |

No regression from extra cloning, remapping or repeated walks. This is consistent with the
structural evidence: stage order is unchanged, clone sites fell from 94 to 87 across the moved
owners, and the remaining `string_table.clone()` calls are all on error paths building
`CompilerMessages`.

**Validation.** `just validate` exits 0: clippy all-features, feature-lane check, `xtask
source-audit` (1198 files, 0 findings, both boundary rules active), `cargo test --workspace`
(4403 / 17 / 779, 0 failed), `cargo run -- tests --terse` (1851 cases), the docs build, `xtask
bench-ci` and the timers-erasure check. `just test-honesty-evidence` regenerated the durable
inventory: 0 hard findings, 0 ledger-integrity findings, 0 source-audit findings, 0 feature-lane
findings.

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
