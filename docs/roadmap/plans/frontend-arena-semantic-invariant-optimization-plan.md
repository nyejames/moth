# Frontend Arena + Semantic Invariant Optimisation Plan

## Purpose

This plan defines a cautious, evidence-based optimisation programme for the Moth compiler frontend. The focus is to reduce allocation, cloning, remapping, and repeated semantic work in hot frontend paths while preserving compiler correctness, diagnostics, and the current stage ownership model.

The first implementation target is **typed `Vec` arenas with stable ID handles**, seeded by capacity heuristics gathered during tokenization/header preparation. The first structural refactor target is **scope-frame arenas**, because Moth’s no-shadowing rule gives a direct opportunity to replace cloned scope contexts with parent-linked frames.

This plan is deliberately staged. Each phase must produce benchmark/profile evidence and must be willing to backtrack if an expected optimisation does not improve the targeted measurements.

---

## Final Agreed Interview Decisions

- [x] Use typed `Vec` arenas with stable ID handles first.
- [x] Defer `bumpalo`, dropless bump allocation, and lifetime-heavy arena references until profiling justifies them.
- [x] Implement scope-frame arenas before expression/template arenas.
- [x] Start with an instrumentation, benchmark, and profiling baseline phase before optimisation refactors.
- [x] Require before/after profiling and benchmark evidence for every optimisation phase.
- [x] Use repeatable performance gates with a rollback/scrutiny rule.
- [x] Commit adversarial high-churn Moth benchmark fixtures under `./benchmarks`.
- [x] Include deterministic fixture generation only where useful, with committed generated fixtures as the canonical benchmark inputs.
- [x] Include an early external package registry clone-reduction phase.
- [x] Allow modest initial over-allocation for capacity heuristics.
- [x] Store optimisation reports under `benchmarks/`, not roadmap.
- [x] Treat capacity heuristics as tunable policy, never correctness logic.
- [x] Add a roadmap/progress matrix item for **Frontend Arena + Semantic Invariant Optimisation**.
- [x] Mark deeper allocator work and broader arena migrations as deliberately deferred until profiling justifies them.
- [x] Replace the existing scope-context cloning path directly; do not add compatibility wrappers.
- [x] Produce `TokenStats`, `HeaderStats`, and `FrontendArenaCapacityEstimate` unconditionally, but keep verbose reporting behind existing instrumentation/benchmark output.
- [x] Use five repeated benchmark invocations and compare medians. The existing benchmark runner’s internal warmup/measured iteration model should remain unless a later phase explicitly justifies changing it.
- [x] Use Samply for targeted profiles, not every fixture.
- [x] Require semantic/output equivalence checks for every phase.
- [x] Optimisation must never change graph identity, fingerprints or diagnostic order.
- [x] Keep arena-specific types, capacity heuristics, counters, and optimisation helpers out of core pipeline files when they grow beyond small orchestration glue.

---

## Current Repo Shape After Interview

### Frontend orchestration

Current frontend module compilation is coordinated in:

```text
src/build_system/create_project_modules/frontend_orchestration.rs
```

The current module pipeline is:

```text
attach source files
prepare module files
sort headers
build AST
resolve const fragments
lower HIR
check borrows
collect reachability
finalize Module
```

Important current implementation facts:

- File preparation is already fused as tokenization + header parsing and runs per file before module aggregation.
- Stage timings are already wrapped around file preparation, dependency sorting, AST, HIR, and borrow checking.
- `CompilerFrontend::new(...)` currently receives cloned `style_directives`, cloned `external_packages`, and cloned `project_path_resolver`.
- The returned `Module` currently stores a cloned `external_package_registry`.
- `record_module_input_counters(...)` already records module count, source file count, and source byte count.
- `record_header_counters(...)` already records header count, dependency-clause count, and top-level declaration count.

### Profiling profile

Current `Cargo.toml` has:

```toml
[profile.profiling]
inherits = "release"
debug = "line-tables-only"
strip = false
lto = "thin"
codegen-units = 1
panic = "abort"
```

This is suitable for Samply profiling while retaining useful symbols.

### Instrumentation

Current frontend instrumentation lives under:

```text
src/compiler_frontend/instrumentation/
```

The existing `FrontendCounter` surface already includes counters for:

- module/file/source/token/header/dependency/declaration volume;
- dependency sorting volume;
- AST construction and compile-time evaluation volume;
- HIR and borrow validation volume;
- type-environment cache/query pressure;
- string-table full clones and merge pressure;
- module string-ID remap calls.

This is the right owner for new local-only optimisation counters. Do not scatter ad hoc counters through unrelated modules.

### Header/token preparation

Current header-stage data contracts live in:

```text
src/compiler_frontend/headers/types.rs
src/compiler_frontend/headers/parse_file_headers.rs
```

`FileFrontendPrepareOutput` already stores:

- `token_count`;
- `file_dependency_clauses`;
- `headers`;
- `top_level_const_fragments`;
- `const_template_count`;
- `runtime_fragment_count`;
- warnings.

This is the best early place to add `TokenStats` and aggregate `HeaderStats` without adding extra full-pipeline traversals.

### Benchmarks

Current benchmark documentation lives in:

```text
benchmarks/README.md
```

Existing commands:

```bash
just bench-check
just bench-frontend-check
just bench
just bench-frontend
just bench-report
just profile-build
```

Current benchmark case files:

```text
benchmarks/cases.txt
benchmarks/frontend-cases.txt
```

Current benchmark fixture groups include `core`, `docs`, `stress`, `module`, and `borrow`. The existing `stress` group already covers template, type, fold, pattern, collection, and environment stress. The new adversarial fixtures should extend this group unless a new group clearly improves summary readability.

Raw local history and raw profiler output must remain uncommitted.

---

## Non-Negotiable Constraints

- [ ] Optimisations must be semantics-neutral unless a phase explicitly says otherwise.
- [ ] Capacity estimates must never affect diagnostics, ordering, lowering, type identity, or emitted artifacts.
- [ ] If an estimate is too small, the arena grows normally.
- [ ] If an estimate is too large, the only acceptable effect is bounded memory overhead.
- [ ] Keep `StringTable` fork/merge/remap semantics intact.
- [ ] Keep typed diagnostics on `CompilerDiagnostic` and infrastructure failures on `CompilerError`.
- [ ] Run `just validate` before finishing every non-trivial implementation phase.
- [ ] Perform manual stage-boundary review for frontend boundary cleanup.
- [ ] Do not add compatibility wrappers for obsolete APIs.
- [ ] Do not commit raw Samply profiles or `benchmarks/local-data/`.
- [ ] Do not add failing diagnostic cases as benchmarks.

---

## Semantic Invariants To Exploit

These invariants should be documented in the optimisation report and used as review checks when evaluating new arena/scope/lookup code.

### No visible shadowing

Moth forbids visible redeclaration while a name is still in scope.

Optimisation use:

- [ ] Replace cloned scope-context maps with parent-linked `ScopeFrameId`s.
- [ ] Use redeclaration checks against ancestor frames instead of maintaining shadow stacks.
- [ ] Avoid “nearest binding wins” candidate resolution machinery.
- [ ] Store local declarations in frame-local maps and ranges.

### Header parsing owns top-level discovery

Header parsing discovers top-level declarations and creates declaration shells. AST consumes sorted headers and must not rediscover top-level declarations from raw tokens.

Optimisation use:

- [ ] Use header counts as declaration arena capacity seeds.
- [ ] Avoid extra AST scans for top-level discovery.
- [ ] Keep declaration-shell parsing shared, not duplicated.

### Dependency sorting is authoritative

AST receives declarations in dependency order.

Optimisation use:

- [ ] Avoid AST fixpoint passes for constants, aliases, structs, choices, and signatures.
- [ ] Add new top-level dependency edges to header/dependency sorting instead of compensating in AST.
- [ ] Treat missing sorted-order assumptions as bugs or missing header edges.

### Header syntax preparation plus interface binding

Header syntax preparation builds declaration shells, dependency shells and local ordering hints. Interface binding later resolves retained dependency shells against completed provider interfaces to produce final visibility.

Optimisation use:

- [ ] Scope frames store body-local declarations only.
- [ ] Immutable file and module visibility tables are shared by reference or ID.
- [ ] Avoid copying binding visibility into every child context.
- [ ] Immutable provider interfaces are shared rather than cloned per consumer.
- [ ] Build-owned mutable provider state is separate from reusable builder capability definitions.

### Normal-root dormant activity

A normal root owns dormant top-level runtime work and page fragments. Support roots and the project package facade are API-only with no implicit start.

Optimisation use:

- [ ] Allocate start-specific structures only when the module root role has dormant activity.
- [ ] Use root runtime and const fragment counts from file preparation instead of rescanning all files later.
- [ ] Entry-local module assumptions are replaced by canonical module artefacts compiled once per project or package boundary.

### Generics resolve before HIR

HIR never carries unresolved generic executable types or unsolved generic calls.

Optimisation use:

- [ ] Keep generic templates in AST-only storage.
- [ ] Drop generic template scratch before/at HIR handoff when no longer needed.
- [ ] Avoid broad constraint graph machinery; inference is local to immediate call evidence and immediate expected result context.

### Traits are static metadata

Traits are frontend compile-time metadata, not runtime values or trait objects.

Optimisation use:

- [ ] Use compact evidence maps keyed by stable IDs.
- [ ] Avoid vtables, erased dispatch metadata, and runtime trait object lowering.
- [ ] Resolve bound-provided receiver calls before HIR.

### External packages expose free functions only

External packages expose opaque types, constants, and free functions. They do not expose receiver methods.

Optimisation use:

- [ ] Avoid external receiver-method candidate catalogs.
- [ ] Resolve external calls to stable IDs early.
- [ ] Share immutable external package metadata instead of cloning it per module where practical.

### Canonical `TypeId` semantic identity

Semantic type decisions use `TypeId` in the module `TypeEnvironment`. `DataType` is parse-only or diagnostic-only after semantic resolution.

Optimisation use:

- [ ] Arena nodes store `TypeId`s and compact IDs, not cloned semantic type trees.
- [ ] Query borrowed field/variant views instead of cloning member lists.
- [ ] Keep rendered type names at diagnostic render boundaries.

### No closures or general function values

General closures, anonymous function values, generic function values, and higher-order polymorphism are outside current language scope.

Optimisation use:

- [ ] No closure-capture environment objects.
- [ ] Function-local scope data does not escape as runtime function values.
- [ ] Arena lifetimes can stay stage/module scoped without capture promotion.

### No macro expansion language

General macros are outside current language scope.

Optimisation use:

- [ ] No macro expansion arena.
- [ ] No hygiene context.
- [ ] No repeated parse/expand/fold loop.

### Borrow validation is side-table based

Borrow validation reads HIR and produces side-table facts. It does not mutate HIR.

Optimisation use:

- [ ] Borrow facts can become dense side-table arenas keyed by HIR IDs later.
- [ ] HIR nodes should not grow borrow-state fields.
- [ ] Snapshot-heavy diagnostics can be investigated separately from semantic borrow facts.

---

## Evidence and Rollback Protocol

### Required benchmark protocol

At every phase boundary:

- [ ] Run five independent benchmark invocations and compare medians.
- [ ] Preserve the existing benchmark runner’s internal measured-iteration model unless a dedicated benchmark-system phase changes it.
- [ ] Run focused frontend benchmarks for compiler-stage refactors.
- [ ] Run end-to-end CLI benchmarks before merging substantial changes.
- [ ] Run the docs project build/check path.
- [ ] Run targeted adversarial fixtures relevant to the phase.
- [ ] Record summarized results in `benchmarks/frontend-optimization-results.md`.
- [ ] Do not commit raw local history or raw profiler files.

Suggested command shape:

```bash
# Validate semantics.
just validate

# Fast local checks while iterating.
just bench-frontend-check
just bench-check

# Phase-boundary recorded benchmark runs.
just bench-frontend
just bench

# Local drilldown.
just bench-report
```

For five independent invocations:

```bash
for i in 1 2 3 4 5; do
    just bench-frontend
    just bench
done

just bench-report
```

### Required Samply profiling protocol

Use Samply at baseline and after major refactors, not for every fixture.

```bash
just profile-build
samply record ./target/profiling/moth build docs --release
```

If `just profile-build` does not build with `detailed_timers`, use:

```bash
cargo build --profile profiling --features detailed_timers
samply record ./target/profiling/moth build docs --release
```

Targeted fixture examples:

```bash
samply record ./target/profiling/moth check benchmarks/template-stress.moth
samply record ./target/profiling/moth check benchmarks/environment-stress.moth
samply record ./target/profiling/moth check benchmarks/adversarial/one-module-kitchen-sink.moth
```

### Regression threshold

- [ ] Treat a median regression greater than 5% in `docs --release`, the focused frontend suite, or a targeted adversarial fixture as a blocker unless the phase has a documented, accepted tradeoff.
- [ ] If a change was expected to improve performance but does not move the relevant timing/counter/profile, scrutinize, revise, or revert it.
- [ ] Do not treat counter movement as a win unless timing also moves meaningfully or Samply proves the hotspot moved away.
- [ ] If results are mixed, inspect per-case local data and run targeted profiles before drawing conclusions.

---

## Organisation and Modularity Rules For This Refactor

Arena and optimisation support code must not bloat core pipeline files.

### Keep pipeline files focused

Files such as:

```text
src/build_system/create_project_modules/frontend_orchestration.rs
src/compiler_frontend/pipeline.rs
src/compiler_frontend/ast/mod.rs
```

should remain orchestration files. They may construct or pass a context, but they should not grow large capacity formulas, arena internals, or benchmark-specific helper logic.

### Recommended new module layout

Use a dedicated frontend optimisation/arena area for reusable support:

```text
src/compiler_frontend/arena/
    mod.rs
    ids.rs
    typed_arena.rs
    capacity.rs
    token_stats.rs
    header_stats.rs
    actuals.rs
```

Use AST-local modules for AST-owned arena structures:

```text
src/compiler_frontend/ast/module_ast/scope_context/
    mod.rs
    context.rs
    scope_arena.rs
    scope_frame.rs
    lookup.rs
```

If expression/template arenas are later implemented, prefer focused owners:

```text
src/compiler_frontend/ast/expressions/
    arena.rs
    scratch.rs

src/compiler_frontend/ast/templates/
    arena.rs
    render_plan_arena.rs
```

If benchmark generators are needed, prefer one clear tooling owner:

```text
benchmarks/generators/
    README.md
    generate_adversarial.rs or generate_adversarial.py
```

or extend `xtask` if that already owns repo tooling in the current implementation.

### File organisation checklist

- [ ] Add file-level docs to every new arena/stats/capacity module.
- [ ] Keep capacity formulas centralized in `capacity.rs`.
- [ ] Keep actual allocation/capacity reporting centralized in `actuals.rs` or instrumentation modules.
- [ ] Keep IDs in `ids.rs` or the module that owns the storage.
- [ ] Avoid long parameter lists by passing named context structs.
- [ ] Avoid generic helper abstractions that hide stage ownership.
- [ ] Delete old wrappers once new APIs are wired.
- [ ] Keep test-only helpers in test modules, not production files.

---

## Phase 0 — Baseline, Current-State Audit, and Report File

### Summary

This phase creates the evidence baseline and report structure before changing optimisation-sensitive code. It should capture the current compiler shape, current benchmark behaviour, and current Samply hotspot profile. No major optimisation refactor belongs in this phase.

Phase 0 completed on 2026-06-18. The baseline report is in
`benchmarks/frontend-optimization-results.md`. Existing benchmark fixtures were repaired to match
current language rules before the final benchmark/validation pass. Samply profile files were
produced under ignored local data, but contained zero samples even after a repeated docs profile,
so Phase 1 should use the benchmark stage/counter baseline unless a later slice needs
function-level attribution.

### Implementation steps

- [x] Create `benchmarks/frontend-optimization-results.md`.
- [x] Add a baseline section with:
  - [x] date;
  - [x] commit SHA;
  - [x] machine/OS/CPU notes;
  - [x] Rust toolchain;
  - [x] command list;
  - [x] benchmark suite versions/case list;
  - [x] raw profile filenames stored locally only.
- [x] Run semantic validation:
  - [x] `just validate`.
- [x] Run benchmark checks:
  - [x] `just bench-frontend-check`;
  - [x] `just bench-check`.
- [x] Run five recorded benchmark invocations:
  - [x] `just bench-frontend` five times;
  - [x] `just bench` five times;
  - [x] summarize medians with `just bench-report`.
- [x] Build a profiling binary:
  - [x] `just profile-build`, or `cargo build --profile profiling --features detailed_timers` if needed.
- [x] Record baseline Samply profiles:
  - [x] `samply record ./target/profiling/moth build docs --release`;
  - [x] one targeted profile for existing `template-stress.moth`;
  - [x] one targeted profile for existing `environment-stress.moth`.
- [x] Summarize top findings in `benchmarks/frontend-optimization-results.md`:
  - [x] top stage timings;
  - [x] top counters;
  - [x] top sampled functions from Samply;
  - [x] obvious clone/allocation/drop hotspots;
  - [x] no raw profile data.
- [x] Record the agreed semantic invariants and the intended optimisation use of each invariant.

### Audit / style guide review / validation

- [x] Confirm no compiler semantics changed.
- [x] Confirm no raw local data or profiles were committed.
- [x] Confirm benchmark report stays concise and does not duplicate raw counter tables.
- [x] Confirm `just validate` passes.

### Backtrack criteria

- [ ] If the benchmark report workflow is too noisy or hard to reproduce, simplify it before proceeding.
- [x] If baseline profiles are too short to interpret, profile larger targeted cases or repeat workloads before optimisation phases begin.

---

## Phase 1 — TokenStats, HeaderStats, and Capacity Estimate Framework

### Summary

This phase adds cheap, unconditional statistics that piggyback on work already performed during tokenization and header aggregation. It also introduces the central capacity-estimate model used by typed arenas. This phase should not alter compiler semantics.

Phase 1 completed on 2026-06-18. It added frontend-local token/header statistics, centralized
capacity-estimate policy, and initial detailed-timer counters for scope-frame estimates. Actual
scope-frame and scope-arena-capacity counters remain zero until Phase 4 creates real arena
storage. The user requested a pause before starting Phase 2 to add more comprehensive profiling
and benchmarking tooling.

### Implementation steps

- [x] Add `TokenStats` in a dedicated module, preferably:

```text
src/compiler_frontend/arena/token_stats.rs
```

or another clearly named frontend stats module if the final owner is different.

- [x] Track cheap token counts while tokenization already creates tokens:
  - [x] total tokens;
  - [x] symbols;
  - [x] literals;
  - [x] operators;
  - [x] template starts/body markers;
  - [x] style directives;
  - [x] dependencies;
  - [x] hashes;
  - [x] `if`;
  - [x] `loop`;
  - [x] `catch`;
  - [x] `then`;
  - [x] returns;
  - [x] casts;
  - [x] mutable markers;
  - [x] map/collection delimiters.

- [x] Add `TokenStats` to `FileFrontendPrepareOutput`.
- [x] Remap handling must remain unaffected; stats contain counts only and need no string-ID remap.
- [x] Add `HeaderStats` in a dedicated module, preferably:

```text
src/compiler_frontend/arena/header_stats.rs
```

- [x] Compute `HeaderStats` during `parse_headers(...)` aggregation or adjacent header-counter recording:
  - [x] functions;
  - [x] constants;
  - [x] structs;
  - [x] choices;
  - [x] type aliases;
  - [x] traits;
  - [x] conformances;
  - [x] trait incompatibilities;
  - [x] const templates;
  - [x] start functions;
  - [x] dependencies;
  - [x] generic parameters;
  - [x] signature members;
  - [x] choice variants;
  - [x] dependency edges.

- [x] Add `FrontendArenaCapacityEstimate` in:

```text
src/compiler_frontend/arena/capacity.rs
```

- [x] Seed initial estimates using:
  - [x] source file count;
  - [x] source byte count;
  - [x] token stats;
  - [x] header stats;
  - [x] existing fragment counts.

- [x] Keep formulas conservative with modest over-allocation and hard caps.
- [x] Add a short comment above every non-obvious formula explaining what it estimates and why.
- [x] Add initial estimate fields for:
  - [x] scope frames;
  - [x] declarations;
  - [x] expressions;
  - [x] expression items;
  - [x] statements;
  - [x] templates;
  - [x] template atoms;
  - [x] render pieces;
  - [x] HIR blocks/statements/expressions;
  - [x] borrow facts.

- [x] Only wire the fields needed by early phases. Future fields remain unused until their arena
      phases consume them.
- [x] Add counters for estimate-vs-actual reporting behind `detailed_timers`:
  - [x] estimated scope frames;
  - [x] actual scope frames;
  - [x] scope arena capacity;
  - [x] estimate saturation / capped estimates;
  - [ ] future fields as they become used. Deferred to the arena phase that consumes each field.

### Tests

- [x] Add unit tests for `TokenStats` classification if tokenization APIs make this cheap.
- [x] Add unit tests for `HeaderStats` from simple synthetic headers.
- [x] Add unit tests for `FrontendArenaCapacityEstimate` caps and monotonic behaviour.
- [x] Add regression tests ensuring stats do not affect diagnostics or output. Existing
      integration/golden validation is the regression owner; adding a duplicate fixture would not
      prove a distinct behavior because stats are policy-only.

### Audit / style guide review / validation

- [x] Confirm stats collection does not add a new full source/token/header traversal beyond work already being done.
- [x] Confirm stats/capacity files are separate from pipeline orchestration.
- [x] Confirm `FrontendArenaCapacityEstimate` is policy-only.
- [x] Run `just validate`.
- [x] Run `just bench-frontend-check` and compare to baseline.

### Backtrack criteria

- [ ] If stats collection produces measurable overhead before any arena uses it, simplify the stats set or move costly metrics behind `detailed_timers`.
- [ ] If formulas become noisy in orchestration files, move them back into `capacity.rs`.

---

## Phase 2 — Adversarial Benchmark Fixtures and Generator Support

### Summary

This phase expands benchmark coverage before the major refactors. The goal is to expose unexpected compiler weak spots and produce repeatable high-churn workloads. Fixtures should be valid Moth programs, not diagnostic failures.

Phase 2 completed on 2026-06-18. It added hand-authored adversarial benchmark fixtures under
`benchmarks/adversarial/`, wired them into the focused frontend and end-to-end benchmark suites,
documented the fixture purpose in `benchmarks/README.md`, and recorded validation/profile evidence
in `benchmarks/frontend-optimization-results.md`. No generator was added because the initial
fixtures are clearer as static source inputs.

### Implementation steps

- [x] Add a `benchmarks/adversarial/` directory.
- [x] Add static committed fixtures first:
  - [x] `one-module-kitchen-sink.moth` — many constructs in one module to stress combined AST/environment/type/template paths.
  - [x] `deep-scope-churn.moth` — nested functions/control/value blocks/templates to stress scope frame creation and lookup.
  - [x] `template-render-plan-churn.moth` — nested templates, slots, `$children`, `$md`, runtime template control flow.
  - [x] `constant-dag-churn.moth` — many constants, cross-constant references, folded templates, arithmetic trees.
  - [x] `expression-rpn-churn.moth` — large expressions, casts, operators, value-producing blocks at valid receiving sites.
  - [x] `generic-trait-churn.moth` — generic functions/types, trait bounds, conformances, concrete instantiations.
  - [x] `collection-map-borrow-churn.moth` — collections, maps, mutable access, fallible operations, borrow-valid aliases.
  - [x] `import-external-churn/` — project fixture with import fanout and repeated external package usage.

- [x] Keep fixtures representative; do not add many near-duplicates.
- [x] Add fixtures to `benchmarks/cases.txt` and `benchmarks/frontend-cases.txt` under existing `stress` or `module` groups unless a new group is clearly justified.
- [x] Generation was considered but was not useful for this static fixture set; no generator was added.
- [x] Update `benchmarks/README.md` fixture list with the new adversarial fixtures.
- [x] Add notes explaining that adversarial fixtures are for compiler churn discovery, not public language examples.

### Audit / style guide review / validation

- [x] Ensure all fixtures are valid successful programs/projects.
- [x] Ensure no generated output folders are committed.
- [x] Ensure benchmark groups remain readable.
- [x] Run `just bench-frontend-check`.
- [x] Run `just bench-check`.
- [x] Run targeted Samply profile for `one-module-kitchen-sink.moth` after it is added.
- [x] Run `just validate` before committing the accepted slice.

### Backtrack criteria

- [ ] Remove or merge fixtures that stress the same compiler path without adding insight.
- [ ] If one fixture is too large/noisy for routine checks, keep it as targeted profiling-only and document that status.

---

## Phase 3 — External Package Registry Clone Reduction

### Summary

The first Samply review showed external package metadata cloning as a surprising hotspot. This phase reduces clone pressure before the deeper AST scope rewrite. It is a targeted quick-win phase and should stay independent from arena work where possible.

Phase 3 completed on 2026-06-18. It added detailed-timer counters for external package clone
pressure, captured before/after counter data on dependency-heavy fixtures, and replaced deep registry
clones through the frontend/AST/module/backend handoff with a shared immutable
`Arc<ExternalPackageRegistry>`. Remaining definition/path/ABI clones are registration-time
ownership or owned builder-runtime metadata at true module handoff boundaries.

### Implementation steps

- [x] Audit clone sites for:
  - [x] `ExternalPackageRegistry`;
  - [x] `ExternalPackage`;
  - [x] `ExternalFunctionDef`;
  - [x] `ExternalSymbolPath`;
  - [x] ABI parameter lists;
  - [x] builder runtime package metadata.

- [x] Add or verify counters:
  - [x] `ExternalPackageRegistryCloneCount`;
  - [x] `ExternalPackageDefinitionCloneCount`;
  - [x] `ExternalFunctionDefinitionCloneCount`;
  - [x] `ExternalSymbolPathCloneCount`;
  - [x] `ExternalAbiParameterCloneCount`.

- [x] Review whether `CompilerFrontend::new(...)` can borrow or share immutable external package metadata instead of cloning it.
- [x] Prefer `Arc`/shared immutable storage only where ownership needs to survive across module/backend boundaries.
- [x] Avoid lifetime-heavy borrow threading if it makes the pipeline harder to reason about; use a small shared registry wrapper if needed.
- [x] Preserve backend-facing `Module` semantics:
  - [x] backend validators still have access to reachable external function metadata;
  - [x] diagnostics can still resolve external symbol/type names;
  - [x] module external imports remain reachable-filtered as before.

- [x] If final `Module` still needs an effective registry, make it cheap to clone by sharing internal immutable tables.
- [x] Remove obsolete clone-heavy APIs after replacement.
- [x] Do not add compatibility wrappers.

### Tests

- [x] Run existing external-package dependency tests.
- [x] Add targeted tests only if ownership/API changes create new invariants. Existing targeted
      coverage was sufficient because semantics did not change.
- [x] Ensure external JS import fixtures still pass.

### Audit / style guide review / validation

- [x] Confirm immutable external package metadata is not accidentally made mutable through shared ownership.
- [x] Confirm no backend rediscovery or duplicate metadata path was introduced.
- [x] Confirm code remains organised under `external_packages/` and pipeline files only pass the new shared handle/context.
- [x] Run `just validate`.
- [x] Run `just bench-frontend-check` and targeted external/import benchmarks.
- [x] Record before/after clone counters and Samply findings.

### Backtrack criteria

- [ ] If shared ownership complicates APIs but clone counters/timings do not improve, revert or narrow the change.
- [ ] If a clone remains necessary at a true ownership boundary, document it rather than contorting APIs.

---

## Phase 4 — ScopeFrame Arena Refactor

### Summary

This is the first major arena refactor. Replace scope-context cloning with a typed `Vec` arena of parent-linked scope frames. This directly exploits Moth’s no-shadowing invariant and should reduce cloned maps/vectors during AST environment, expression, template, and body parsing.

Phase 4 completed on 2026-06-18. It replaced the flat cloned local-declaration state with
`ScopeArena`, stable `ScopeFrameId`s, parent-linked frames, and arena-local local declarations
stored behind cheap handles so parser APIs do not expose `RefCell` guards. Body-local functions
still start from fresh root frames, preserving the no-closure/no-capture language invariant.
Capacity preallocation from `FrontendArenaCapacityEstimate` is intentionally left to Phase 5:
Phase 4 observations show current estimates undercount scope frames on scope-heavy fixtures, so
the formulas should be tuned before they seed arena capacity.

### Target design

```rust
pub struct ScopeArena {
    frames: Vec<ScopeFrame>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScopeFrameId(u32);

pub struct ScopeFrame {
    pub parent: Option<ScopeFrameId>,
    pub kind: ScopeFrameKind,
    pub local_names: FxHashMap<StringId, DeclarationId>,
    pub local_declarations: Range<DeclarationId>,
    pub expected_result: Option<ExpectedResultId>,
    pub flags: ScopeFrameFlags,
}

pub struct ScopeContext {
    pub shared: ScopeSharedId,
    pub frame: ScopeFrameId,
}
```

Exact field names should follow the current codebase’s naming and existing types. The design principle is stable: frame IDs and parent links replace cloned visible state.

### Implementation steps

- [x] Move/split scope code so arena-specific pieces are separate:

```text
src/compiler_frontend/ast/module_ast/scope_context/
    mod.rs
    context.rs
    scope_arena.rs
    scope_frame.rs
    lookup.rs
```

Use the current module shape if it differs, but keep arena internals out of high-level AST orchestration files.

- [ ] Add `ScopeArena::with_capacity(estimate.scope_frames)`. Deferred to Phase 5 tuning after
      actual-vs-estimate observations showed the initial estimates undercount scope-heavy cases.
- [x] Add `ScopeFrameId` with safe index conversion helpers.
- [x] Add root/module/file/function/template/block frame constructors.
- [x] Replace child context clone constructors with frame allocation:
  - [x] child expression context;
  - [x] child template context;
  - [x] child function/body context;
  - [x] child constant context;
  - [x] block/value-producing context.

- [x] Replace visible-local cloned maps with parent-chain lookup.
- [x] Implement redeclaration checks using no-shadowing:
  - [x] check current frame;
  - [x] check ancestor frames until file/module boundary as required by current visibility rules;
  - [x] check shared header-built visibility/dependency/builtin records.

- [x] Ensure binding visibility remains header-owned and shared.
- [x] Ensure diagnostics keep correct `SourceLocation` and source labels.
- [x] Add actual arena counters:
  - [x] frames allocated;
  - [x] max frame depth;
  - [x] local declarations inserted;
  - [x] lookup ancestor steps;
  - [x] redeclaration ancestor checks;
  - [x] old scope-context clone counter, if still present during migration, should trend to zero and then be removed.

- [x] Delete obsolete scope cloning helpers once callers are migrated.
- [x] Do not keep old and new scope APIs in parallel.

### Tests

- [x] Add focused scope-frame unit tests outside production files:
  - [x] parent lookup;
  - [x] no-shadowing redeclaration across frames;
  - [x] same-frame redeclaration;
  - [x] visibility boundary behaviour;
  - [x] expected-result propagation where applicable.

- [x] Run integration tests covering:
  - [x] local declarations;
  - [x] nested `if`/loop/catch/value-producing blocks;
  - [x] templates capturing values;
  - [x] same-file receiver methods;
  - [x] dependencies and aliases;
  - [x] duplicate declaration diagnostics.

### Audit / style guide review / validation

- [x] Confirm no user-visible shadowing rule changed.
- [x] Confirm AST does not rebuild binding visibility.
- [x] Confirm scope arena internals are not embedded in pipeline orchestration files.
- [x] Confirm stage-local diagnostics still use `CompilerDiagnostic`.
- [x] Confirm no compatibility wrappers remain.
- [x] Run `just validate`.
- [x] Run five repeated frontend benchmark invocations and compare medians.
- [x] Run targeted Samply profiles for:
  - [x] `environment-stress.moth`;
  - [x] `deep-scope-churn.moth`;
  - [x] `one-module-kitchen-sink.moth`.

### Backtrack criteria

- [ ] If scope-frame lookup increases runtime due to long parent walks, add cached nearest-name indexes or flatten only at safe boundaries, then re-profile.
- [ ] If diagnostics regress, stop and fix diagnostics before further optimisation.
- [ ] If performance regresses more than 5% without a clear offsetting win, revise or revert.

---

## Phase 5 — Capacity Heuristic Tuning For Scope Arenas

### Summary

After scope frames are arena-backed, tune capacity estimates using actual data. This phase should tighten formulas based on `docs`, existing benchmarks, and adversarial fixtures.

Phase 5 completed on 2026-06-18. It tuned the scope-frame estimate formula, added
estimate/actual ratio counters, added `ScopeArena::with_capacity`, and threaded the module-level
estimate into AST emission. Production seeding uses an AST-owned
`ScopeFrameCapacityBudget` that spends the module scope-frame estimate once across known root
function, start, generic-template-validation, and const-template parse contexts. Dynamic generic
instances and direct AST helper callers remain unseeded and grow normally. The final Phase 5
evidence pass found no scope-frame under-estimates across docs, template, scope, kitchen-sink, and
dependency/module fixtures; capacity/actual ratios ranged from about `2.9x` to `3.8x`. Five recorded
frontend and end-to-end benchmark invocations showed no measurable regression.

### Implementation steps

- [x] Record estimate-vs-actual data for scope frames across:
  - [x] docs build;
  - [x] `environment-stress.moth`;
  - [x] `template-stress.moth`;
  - [x] `deep-scope-churn.moth`;
  - [x] `one-module-kitchen-sink.moth`;
  - [x] dependency/module fixtures.

- [x] Add reported ratios behind detailed timers:
  - [x] estimated / actual;
  - [x] final capacity / actual;
  - [x] capped estimate count.

- [x] Tune formulas in `capacity.rs` only.
- [x] Keep modest over-allocation acceptable.
- [x] Add comments explaining the semantic categories that influenced formulas.
- [x] Do not add fixture-specific special cases unless they represent a real semantic category.
- [x] Add `ScopeArena::with_capacity` for frame Vec storage.
- [x] Choose a narrow AST-owned distribution policy for module-level scope estimates before wiring
      `estimate.scope_frames` into production `ScopeArena` construction.
- [x] Wire production scope arena seeding after the per-root policy is chosen.

### Audit / style guide review / validation

- [x] Confirm formulas remain centralized.
- [x] Confirm capacity estimates do not affect semantics.
- [x] Run `just validate`.
- [x] Run five repeated benchmark invocations.
- [x] Update `benchmarks/frontend-optimization-results.md` with estimate-vs-actual summaries.

### Backtrack criteria

- [ ] If tuned formulas overfit adversarial fixtures and hurt real docs/benchmark performance, prefer simpler formulas.
- [ ] If capacity tuning has no measurable effect, keep only the simple low-risk formulas.

---

## Phase 6 — Expression Scratch / Expression Item Arena Pilot

### Summary

This phase is gated by evidence. Implement only if profiles/counters still show expression RPN/order/fold allocation pressure after scope and external package clone work.

Start with scratch-buffer reuse and typed `Vec` arenas. Do not convert the entire expression AST to borrowed arena references.

Gate checked on 2026-06-18 with an Ollama worker plus parent-side `profile-case` reruns after
nested Samply failed inside the worker. The `expression-rpn-churn` profile showed a small
`~5ms` AST case and no dedicated expression allocation/clone pressure counter. Phase 6 is deferred
until a future report shows expression-specific pressure.

### Entry criteria

- [ ] Samply or counters show meaningful expression allocation/clone pressure. Phase 5 evidence
      and the Phase 6 gate profile did not establish this for a broad expression arena pilot.
- [ ] `expression-rpn-churn.moth` or real docs/build profiles show expression paths as top movers.
      Phase 5 and gate reports point first at file preparation and docs AST emission instead.
- [x] Phase 4 and 5 are complete and stable.

### Implementation steps

- [ ] Add expression scratch storage in a focused file:

```text
src/compiler_frontend/ast/expressions/scratch.rs
```

- [ ] Add `ExpressionScratch` with reusable vectors:
  - [ ] RPN items;
  - [ ] operator stack;
  - [ ] fold stack;
  - [ ] temporary argument lists where appropriate.

- [ ] Seed capacities from `FrontendArenaCapacityEstimate`.
- [ ] Thread scratch through expression parser/evaluator context structs, not long parameter lists.
- [ ] Avoid global mutable scratch.
- [ ] Add counters:
  - [ ] expression scratch clears;
  - [ ] expression item pushes;
  - [ ] expression item realloc/growth events if practical;
  - [ ] expression clone count where practical.

- [ ] If scratch reuse is insufficient, consider `ExpressionArena` with `ExpressionId` as a second step in this phase.
- [ ] Keep `ExpressionId` storage internal to AST until HIR lowering is updated.

### Audit / style guide review / validation

- [ ] Confirm expression diagnostics and source locations are unchanged.
- [ ] Confirm constant folding output is unchanged.
- [ ] Confirm no expression scratch leaks across files/modules.
- [ ] Run `just validate`.
- [ ] Run targeted expression benchmark/profile.

### Backtrack criteria

- [ ] If scratch threading makes parser APIs harder to reason about without measurable improvement, revert to simpler local vectors.
- [ ] If expression arena conversion causes broad churn, stop after scratch-buffer reuse and defer full conversion.

---

## Phase 7 — Template / Render-Plan Arena Pilot

### Summary

This phase is gated by evidence. Templates are central to Moth, so template arena changes must be cautious and heavily validated. The goal is to reduce cloning of template atoms, render pieces, and composed/finalized templates.

Gate checked on 2026-06-18 with targeted template and docs profiles. The
`template-render-plan-churn` fixture remained a small `~7ms` AST case. The docs profile showed
meaningful AST emit/finalize time and large template counts, but the stack samples were still
unsymbolicated and the counters did not isolate template/render-plan clone pressure. Phase 7 is
deferred as a broad arena migration until narrower docs/template attribution justifies it.

The final TIR work subsequently established one module store, exact views, one preparation owner,
one fold entry and neutral owned runtime handoff. The template/text/parser/formatter/fold portion of
this phase is therefore superseded by
`docs/roadmap/plans/post-tir-template-parser-optimization-plan.md`. The evidence below remains useful,
but future template work must begin from that final architecture rather than the pre-TIR arena
sketch.

### Entry criteria

- [ ] Samply or counters show template/render-plan clone/allocation pressure after earlier phases.
      Gate profiles did not isolate this pressure.
- [ ] `template-stress.moth`, `template-render-plan-churn.moth`, or docs profiles show template paths
      as top movers. Docs shows AST emit/finalize pressure, but not yet template/render-plan
      clone pressure specifically.
- [ ] Existing template semantics and goldens are stable.

### Handoff

- [ ] Use the dedicated post-TIR plan for fresh profiles, current owner maps, source/text storage,
      formatter allocation, cache keys, invalidation and fold scheduling.
- [ ] Keep broad AST/expression/HIR arena work in this plan only when profiling identifies those
      owners independently of template semantics.
- [ ] Do not add the pre-TIR `arena.rs`, `render_plan_arena.rs` or parallel template state model as a
      compatibility layer.
- [ ] Any later low-risk allocation reduction must preserve the accepted exact-view preparation,
      folding and neutral handoff boundaries and pass the dedicated plan's validation/evidence gate.

---

## Phase 8 — HIR Dense Storage / Borrow Fact Compaction Investigation

### Summary

This is deliberately later. The original profile did not show HIR, borrow checking, or JS lowering as the primary bottleneck. Do not start here unless earlier phases shift the bottleneck or targeted fixtures expose HIR/borrow pressure.

Gate posture after the 2026-06-18 Phase 6/7 evidence pass: HIR and borrow timings were small in
the expression, template, and docs profiles. Phase 8 remains deferred.

### Entry criteria

- [ ] Benchmarks show HIR or borrow validation as a meaningful remaining stage mover. Gate profiles
      did not show this.
- [ ] Samply identifies dense ID hash maps, borrow snapshots, or fact storage as hot.
- [x] AST/scope/template/external clone work is complete or intentionally deferred.

### Investigation steps

- [ ] Audit HIR ID allocation and ID-to-index maps.
- [ ] Replace dense compiler-owned ID hash maps with `Vec<Option<T>>` / dense maps only where IDs are actually dense and stable.
- [ ] Investigate borrow fact side tables keyed by HIR IDs.
- [ ] Investigate snapshot storage size and diagnostic-only snapshot detail.
- [ ] Add counters for dense map hits/misses and borrow fact storage volume.

### Audit / style guide review / validation

- [ ] Confirm HIR validation still catches transformation invariants.
- [ ] Confirm borrow facts remain side tables and do not mutate HIR.
- [ ] Confirm GC-only backend semantics are unchanged.
- [ ] Run `just validate`.
- [ ] Run borrow stress benchmarks and profiles.

### Backtrack criteria

- [ ] If HIR/borrow changes do not improve stage timing or memory pressure, defer them.
- [ ] If fact compaction hurts diagnostic quality, preserve diagnostic fidelity and defer compaction.

---

## Phase 9 — Roadmap, Progress Matrix, and Documentation Updates

### Summary

This phase records the optimisation programme in project planning docs and documents what is intentionally deferred. Do this once the initial implementation direction is stable, and update it again after major phase outcomes.

### Roadmap/progress updates

- [x] Add a roadmap/progress matrix item named:

```text
Frontend Arena + Semantic Invariant Optimisation
```

- [x] Status suggestion:
  - [x] `Planned` before implementation starts;
  - [x] `In Progress` once baseline/instrumentation and first arena work begins;
  - [x] `Partial` or equivalent when scope arenas and capacity heuristics land;
  - [x] keep deeper arena migrations separate/deferred unless implemented.

- [x] Describe scope:
  - [x] typed `Vec` arenas with stable IDs;
  - [x] capacity heuristics from token/header stats;
  - [x] scope-frame arena refactor;
  - [x] clone reduction in external package metadata;
  - [x] benchmark/adversarial fixture expansion;
  - [x] evidence-based rollback rules.

### Deliberately deferred items

Add these as deferred until profiling justifies them:

- [x] `bumpalo`/dropless bump allocation.
- [x] Full AST node arena conversion.
- [x] Full expression arena conversion beyond scratch-buffer reuse.
- [x] Full template/render-plan arena migration.
- [x] HIR arena conversion.
- [x] Borrow fact compaction and snapshot reduction.
- [x] Source-backed package HIR caching remains with canonical module/build-system work.
- [x] Incremental module/template cache prerequisites move to `post-tir-template-parser-optimization-plan.md`; broader incremental compilation remains build-system owned.
- [x] Whole-project persistent semantic cache remains canonical/build-system deferred work.

### Documentation updates

- [x] Update `docs/compiler-design-overview.md` only after implementation changes are real.
- [x] Add a short section explaining:
  - [x] frontend arenas are stage/module-owned implementation details;
  - [x] capacity heuristics are policy-only;
  - [x] `StringTable` remains the path/string identity system;
  - [x] AST/HIR stage ownership remains unchanged.

- [x] Update `benchmarks/README.md`:
  - [x] list new adversarial fixtures;
  - [x] explain the optimisation campaign’s five-invocation median protocol;
  - [x] reaffirm raw local data/profiles are not committed.

- [x] Keep `benchmarks/frontend-optimization-results.md` current:
  - [x] baseline;
  - [x] per-phase result summaries;
  - [x] regressions and reversions;
  - [x] heuristic tuning notes;
  - [x] final decisions.

### Audit / style guide review / validation

- [x] Confirm docs do not promise deferred work as implemented.
- [x] Confirm progress matrix wording is concise and editor-friendly.
- [x] Confirm no stale design notes conflict with the new arena direction.
- [x] Run doc/build validation if the docs site is affected.
- [x] Run `just validate`.

---

## Suggested Phase Order For Coding Agents

Use these as independent implementation chunks. Do not merge phases unless the change is trivial.

1. [x] Phase 0: Baseline and report file.
2. [x] Phase 1: Stats and capacity estimate framework.
3. [x] Phase 2: Adversarial benchmark fixtures.
4. [x] Phase 3: External package registry clone reduction.
5. [x] Phase 4: Scope-frame arena refactor.
6. [x] Phase 5: Scope arena capacity tuning.
7. [x] Phase 6: Expression scratch/arena pilot, deferred until evidence supports it.
8. [x] Phase 7: Template/render-plan arena pilot, deferred until evidence supports it.
9. [x] Phase 8: HIR/borrow dense storage investigation, deferred until evidence supports it.
10. [x] Phase 9: Roadmap/progress/docs finalization.

---

## Final Completion Criteria

The initial optimisation programme is complete when:

- [x] `TokenStats`, `HeaderStats`, and `FrontendArenaCapacityEstimate` exist and are used by the first arena implementation.
- [x] Scope context cloning has been replaced by parent-linked scope frames in a typed `Vec` arena.
- [x] External package registry clone pressure has been reduced or documented as unavoidable at true ownership boundaries.
- [x] Benchmark/adversarial fixtures exist and are documented.
- [x] `benchmarks/frontend-optimization-results.md` records baseline and final phase summaries.
- [x] Roadmap/progress matrix explicitly tracks the optimisation work and deferred deeper arena features.
- [x] `just validate` passes.
- [x] Five-run median benchmarks show no unjustified regressions.
- [x] Samply profiles show the targeted clone/allocation hotspot was reduced or moved.
- [x] Core pipeline files remain readable and are not bloated with arena/capacity helper internals.
