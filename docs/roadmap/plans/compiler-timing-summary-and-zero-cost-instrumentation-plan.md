# Compiler timing summary and zero-cost instrumentation plan

## Status

- **Plan state:** active — Phase 4 checkpoint complete
- **Repository anchor:** `a036b8b26f3e1643c90081a9dd948db8ca289d84` (accepted Phase 0-3 checkpoint; the Phase 3 correction checkpoint commit follows it)
- **Intended repository path:** `docs/roadmap/plans/compiler-timing-summary-and-zero-cost-instrumentation-plan.md`
- **Roadmap status:** deliberately not linked from `docs/roadmap/roadmap.md` yet
- **Scheduling:** the coordinator is executing the plan; this correction checkpoint pauses for user review before Phase 4
- **Primary invariant:** a compiler built without `timers` must perform no timer-system runtime work

This plan improves the human `timers` report without turning it into another benchmark system. It preserves stable benchmark observations, keeps `detailed_timers` verbose and makes the basic report useful for quickly spotting churn in real Moth projects.

---

## Active context capsule

Refresh this block after every accepted slice and before context compaction.

```text
ACTIVE_PLAN:
- `docs/roadmap/plans/compiler-timing-summary-and-zero-cost-instrumentation-plan.md`

CURRENT_SLICE:
- Phase: 4 boundary registration and instrumentation complete
- Checklist item: boundary/inventory/compile metrics, module registration,
  semantic-total recording, slowest-module basis, attribution context
  propagation and Phase 4 test coverage
- Goal: Phase 5 frontend timing gaps
- Non-goals: summary redesign, roadmap insertion, benchmark protocol changes

LAST_GOOD_COMMIT:
- `c1603fd06` (Phase 4 boundary attribution checkpoint; supersedes
  `171895d96`)

CURRENT_WORKTREE_STATE:
- Uncommitted Phase 4 work pending the checkpoint commit. No unrelated user changes.
- Branch: `main`.
- Dedicated worker worktrees: none.

AUDITOR_ROUTE_LIMITATION:
- The delegated `auditor` route is unusable: every launch attempt ended in a
  launcher `contract_violation` because the child model referenced
  out-of-workspace paths. Phase 0, 2 and 3 audit checklist items were
  satisfied by coordinator-led manual audits with recorded `rg`/script
  evidence, not by the formal route. This is a limitation, not an accepted
  formal audit; Phase 8 final audits need a working route or an explicit
  coordinator-approved substitute.

PHASE_3_CORRECTION_RECORD:
- 1. `timed_frontend_stage!` now expands to the production expression
  directly in both feature arms; no closure wrapper survives without
  `timers`. Every call site passes a direct expression, not a closure.
- 2. `timed_frontend_substep` became `timed_frontend_substep!`, a
  direct-expression macro gated on `detailed_timers`; the function wrapper
  was deleted.
- 3. Basic rows render aggregate duration only. `sample_count` and
  `max_label` were removed from `TimingSummaryRow`; attribution renders only
  in the dedicated slowest-module section and through explicit row suffixes.
- 4. `TimingSummaryRow.label` is now `Cow<'static, str>`; the model owns
  `TimingBoundarySummary` and `TimingSlowestModuleSummary` for Phase 4.
- 5. Headings are command-specific (`Build timings 384.63ms` / `Check
  timings 356.11ms`) with the total in the heading; the duplicate
  `Command total` pipeline row was removed.
- 6. The frontend title no longer infers module count from `frontend.ast`;
  Phase 4 will use registered module metadata.
- 7. Erasure tests cover both frontend macros, discarded success
  expressions, headings, totals, noise suppression, dynamic labels and
  renderer layout.
- 8. The source erasure audit rejects `$stage()` / `$substep()` closure
  expansions and function-wrapper forms of the frontend substep.

PHASE_4_RECORD:
- 1. New stable metrics recorded: `build.boundary.inventory`,
  `build.boundary.compile` and `frontend.module.semantic_total`.
- 2. `compile_directory_frontend` registers every source-backed package
  boundary (`@<prefix>`) in deterministic prefix order and the main project
  boundary (`project.name`) after them; binding-backed packages never enter
  the registry because registration walks `source_package_boundaries()`.
- 3. Single-file compilation registers one synthetic main-project boundary
  and one entry-root module with the portable logical identity.
- 4. `compile_module_waves` registers every module keyed by boundary plus the
  dense graph `ModuleId`; logical identities come from the retained
  `StableModuleOriginIdentity` portable path, never absolute filesystem
  paths. Source file counts and byte counts come from prepared facts.
- 5. Boundary totals are summed inventory + compile per boundary and labelled
  accumulated work; `stage0.directory.module_inventory` and
  `stage0.directory.module_compile_batch` are unchanged.
- 6. Compact `TimingModuleContext` values are passed explicitly through
  package/project compilation into `prepare_module` and
  `compile_module_semantic`; no thread-local state exists. Context fields and
  arguments disappear from no-timer builds.
- 7. Slowest-module work is preparation attributed to the module plus
  `frontend.module.semantic_total`, computed from registered module metadata
  in deterministic order.
- 8. Erasure gate now rejects the new metric markers in no-timer binaries.
  `timed_manual_finish_labeled!` was replaced by
  `timed_manual_finish_attributed!`; `timed_frontend_stage!` carries an
  explicit context argument. Both stay direct-expression erasing macros.
- 9. Test coverage: summary boundary rows, registration order, dense-key
  isolation, shuffled-event stability, preparation-plus-semantic slowest
  module, logical identity, binding-package exclusion, erasure of the new
  macro arguments, collector registration, and a real directory compile that
  registers `@<package>` and the project separately. Benchmarking tests now
  share the collector test lock whenever `timers` is enabled.
- 10. Collector sentinel: `NO_TIMING_BOUNDARY` prevents a compile that
  started before a collection scope from polluting the first active scope.

PHASE_0_INVENTORY (unchanged record):
- `pipeline_timer!` 0 call sites (5 doc mentions), `labeled_pipeline_timer!`
  0 call sites (3 doc mentions); `start_pipeline_timing` 57,
  `record_started_pipeline_timing*` 27, `PipelineTimingGuard::new` 16,
  `start_command_timing` 4, `print_command_timing_summary` 7,
  `start_benchmark_collection` 12, `stop_and_collect_benchmark_observations`
  13, `module_timing_label` 5.
- Every timing metric name is a static string literal.
- Consumers: `xtask/src/bench_observations.rs` requires
  `command.build.total` or `command.check.total`; `xtask/src/bench_types.rs`
  owns the display-label map; `xtask/src/bench_report.rs` owns ratio
  definitions.
- In-process collection: `src/benchmarking/frontend.rs` starts a suppressing
  collection; nesting is prevented by lifecycle.

PHASE_1_SUMMARY:
- `src/timing.rs` is a small facade: conditional `mod enabled` plus the
  erasing macro set (`pipeline_timer!`, `labeled_pipeline_timer!`,
  `timing_guard!`, `timed_manual_finish!`, `timed_manual_finish_attributed!`,
  `timed_frontend_stage!`, `timed_frontend_substep!`, `command_timing_start!`,
  `command_timing_finish!`, `counter_observation!`).
- `src/timing/enabled/` owns collector, modes, observation types, summary
  and renderer; counter-only builds keep `CounterOutputMode` and counter
  stdout on the facade.
- Timing observations store `Duration` with static metric names; no-timer
  builds define no snapshot, mode, collector or guard types.
- Tests: no-feature erasure tests and enabled-collector tests under
  `src/timing/tests/`.

PHASE_2_SUMMARY:
- Every manual start/record pair migrated to erasing macros; all
  `PipelineTimingGuard` constructors use `timing_guard!`; command collection
  uses `command_timing_start!` / `command_timing_finish!`; local
  `log_*_timing` helpers deleted.
- `module_label` parameters are `#[cfg(feature = "timers")]`-gated;
  timer-only `Instant::now()` reads are
  `#[cfg(feature = "detailed_timers")]`-gated.
- Hard gate `just timers-erasure-check` builds the no-timer release binary
  in an isolated target dir, scans marker strings and audits sources; wired
  into `just validate`. `nm` on the current binary shows no timer symbols.

PHASE_3_SUMMARY:
- `src/timing/enabled/summary.rs` owns the typed model and one
  `BASIC_METRIC_POLICY` table; `src/timing/enabled/render.rs` owns coloured
  layout with pure line-text helpers.
- Sections: pipeline, frontend (accumulated), backend children, compilation
  boundaries (empty until Phase 4) and slowest module (empty until Phase 4).
- Unknown raw metrics stay in snapshots and bench output but never appear in
  basic output.
- Validation: feature matrix green, no-default/timers/detailed_timers lib
  suites green, `just validate` green, erasure gate green, and
  summary/bench/failing-check smoke captures recorded under `/tmp`.

RELEVANT_CODE:
- `src/timing.rs`, `src/timing/enabled.rs`,
  `src/timing/enabled/{mode,collector,summary,render}.rs`
- `src/projects/cli.rs`, `src/projects/check.rs`,
  `src/projects/dev_server/build_loop.rs`
- `src/build_system/build.rs`,
  `src/build_system/create_project_modules/compilation.rs`
- `src/build_system/create_project_modules/frontend_orchestration.rs`
- `src/compiler_frontend/ast/mod.rs`,
  `src/compiler_frontend/ast/module_ast/environment/builder.rs`,
  `src/compiler_frontend/ast/module_ast/finalization/finalizer.rs`
- `src/projects/html_project/html_project_builder.rs`,
  `src/projects/html_project/js_path.rs`,
  `src/build_system/output/orchestrator.rs`
- `src/benchmarking/frontend.rs`, `xtask/src/bench_*`,
  `xtask/src/timers_erasure_check.rs`, `justfile`

ACCEPTANCE_CRITERIA:
- Builds without `timers` execute no timer-system clock reads, calls,
  allocations, formatting, label construction, environment lookups,
  collector operations or context propagation.
- Timer-only implementation types, statics, renderer code and selected
  timer marker strings are absent from a no-timer release binary.
- Existing stable `MOTH_BENCH timing` names and measurement boundaries
  remain unchanged.
- Basic timer output is curated, grouped, coloured and deterministic.
- The command total is prominent in the heading.
- Source-backed packages and the main project are shown as separate
  compilation boundaries (Phase 4).
- Frontend source preparation, AST/TIR work, public-interface work, HIR,
  borrow validation and generated-function work are readable (Phase 5).
- Detailed timers and benchmark output retain full raw evidence.
- Dev builds print the same summary only when timer output requests it
  (Phase 7).
- Every phase passes its audit, style review and validation gate before the
  next phase starts.

DECISIONS_ALREADY_MADE:
- decision: stable benchmark metric compatibility is mandatory
  - reason: human presentation changes must not corrupt historical benchmark interpretation
  - source/user/date: Nye, 2026-08-05
- decision: zero runtime cost without `timers` is a hard rule
  - reason: normal compiler builds must not pay for developer instrumentation
  - source/user/date: Nye, 2026-08-05
- decision: basic output uses a fixed curated hierarchy and significance filtering
  - reason: the report is for fast scanning, not raw observation dumping
  - source/user/date: Nye, 2026-08-05
- decision: dev initial builds and rebuilds use the same structured report when requested
  - reason: real-project rebuilds are a primary use case for rough churn detection
  - source/user/date: Nye, 2026-08-05
- decision: add the plan under `docs/roadmap/plans/` but do not add it to the roadmap yet
  - reason: the coordinator will choose a flexible natural pause point
  - source/user/date: Nye, 2026-08-05

BLOCKERS / RISKS:
- `compilation.rs`, `frontend_orchestration.rs` and `build.rs` overlap with
  active canonical-module work; re-anchor before edits.
- Package inventory and package compilation happen in separate passes, so
  boundary totals are accumulated work.
- Nested stage timings can double-count; the summary model keeps wall,
  accumulated and nested evidence distinct.
- Future Rayon module parallelism must not make attribution or output
  ordering nondeterministic.
- Release stripping and LTO limit symbol inspection; binary marker absence
  and source-level erasure are the portable hard gates.

VALIDATION_STATE:
- last command: Phase 4 checkpoint validation (in progress before `just validate`)
- result: five `cargo check` feature combos green; 4096 no-feature lib tests,
  4113 `timers` lib tests, 4113 `detailed_timers` lib tests, 601 xtask tests
  green; `just timers-erasure-check` clean (no-timer release binary has none
  of the new metric markers); `just bench-frontend-check` green; docs-build
  summary smoke shows `@html` and the main project as separate boundaries
- known unrelated failure: `cargo test --features timers,benchmark_counters`
  fails only `chunked_file_preparation_skips_identity_payload_remap`
  (expects 1 identity remap, records 2); reproduced at the pristine anchor
  `2877d3012` and unrelated to this plan. With that one case excluded the
  counters suite is green (4119 passed).

DOCS_IMPACT:
- progress matrix needed: no
- other docs stale: `benchmarks/README.md`, timer module docs and possibly
  validation docs after the command set changes (Phase 8)
- authorized docs updates: this plan, benchmark/timer developer
  documentation and validation command documentation
- explicitly unauthorized in this plan: adding this plan to
  `docs/roadmap/roadmap.md`

NEXT_ACTION:
- record the Phase 4 checkpoint commit after `just validate`
- then run Phase 5 frontend timing gaps
```


---

## Accepted design

### Product boundaries

The repository keeps three separate tools:

1. **Basic timers**
   - A short human report for one real command or dev rebuild
   - Curated stage names
   - Project/package attribution
   - Rough wall and accumulated timings
   - No history, thresholds against previous runs or regression claims

2. **Detailed timers**
   - Verbose inline developer prose
   - Fine-grained AST, TIR, config, backend and output observations
   - Full raw inspection when an investigation needs it

3. **Benchmark observations**
   - Stable machine-readable `MOTH_BENCH timing` records
   - In-process observation snapshots
   - History, comparisons and report tooling owned by the benchmark system

Basic timers must not grow into a fourth profiling or benchmarking framework.

### Hard zero-cost rule

When `timers` is not selected, timer instrumentation must be removed by conditional compilation rather than left for optimisation.

The disabled build must add none of the following:

- timer-specific `Instant::now()` reads
- timer guard construction or drop work
- no-op timer function calls
- collector mutex access
- event, boundary or module metadata allocation
- metric or label formatting
- logical-path rendering for timing labels
- `MOTH_TIMERS` environment access
- timing-only fields in ephemeral compiler or build structs
- timing-only arguments in emitted function ABIs
- summary construction
- `saying::say!` calls from the timer system
- selected timer-only strings in the final release binary

Existing clocks that are part of normal user-visible behaviour, such as `Done in ...` or the dev-server status line, are baseline command behaviour. They may remain. The hard rule forbids additional work introduced solely by the timer system.

### Benchmark compatibility rule

- Keep every existing stable metric name.
- Keep every existing metric's measurement boundary.
- New observations use new metric names.
- Human grouping may combine several raw metrics without changing them.
- Do not silently broaden an old metric. For example, linked-module JS lowering must not be added to the existing entry-only `backend.js.lower_hir`.
- A benchmark protocol bump is outside this plan unless the coordinator explicitly approves one after a discovered incompatibility.

### Basic summary contract

The final shape should be close to:

```text
Build timings                                      384.63ms

Build pipeline
  Bootstrap                                          11.20ms
  Discover and prepare graph                         94.03ms
  Compile packages and project                      241.74ms
  Backend                                              7.27ms
  Write output                                        28.19ms
  Other                                                 2.90ms

Compilation boundaries · accumulated work
  @html                              1 module           5.13ms
  moth_docs                         69 modules         336.40ms

Frontend work · 70 modules · accumulated
  Prepare source files                                 82.61ms
  Bind headers                                         19.37ms
  Order declarations                                    3.75ms
  Semantic frontend / AST                             184.56ms
    Environment, types and constants                    52.30ms
    Bodies and TIR construction                         83.11ms
    Template and constant finalization                  49.15ms
  Public interface                                      12.40ms
  HIR                                                    3.79ms
  Borrow validation                                      1.64ms
  Generated functions                                    2.08ms

Backend
  JS lowering                                            2.02ms
  HTML rendering                                         1.30ms

Slowest module
  @docs/progress                         16.70ms · 1 file · 43.9KB
```

Exact values are illustrative.

Formatting rules:

- Use architecture order, never alphabetical metric order.
- Use blank lines between major sections.
- Indent children under their owner.
- Use logical module identities rather than absolute filesystem paths.
- Show the command total prominently.
- Use `saying::say!` directly. Do not embed raw ANSI codes.
- Use `Blue` or `Bold Blue` for headings.
- Use `Green` for ordinary timing values.
- Use `Yellow` for command, package and project totals.
- Use `Dark White` or reset text for suffixes and attribution.
- Colour must not imply a performance judgement. It distinguishes row roles only.
- Omit rows that would render as `0.00ms`.
- Always show available major top-level rows.
- Show optional child rows only when the unrounded duration is at least `1ms` and at least `5%` of its parent.
- Show `Other` only when it is at least `1ms` or `2%` of command total.
- Never render a negative `Other` value.
- Label repeated or parallel stage totals as accumulated work.
- Do not show percentages for accumulated work because parallel samples may exceed wall time.
- Basic mode shows one slowest module, not a full module table.

### Attribution model

Timing metadata is command-local developer data. It must not enter:

- `CompiledModuleArtifact`
- `PublicSemanticInterface`
- `ModuleExecutable`
- `ModuleLinkFacts`
- `ModuleCompilerMetadata`
- HIR
- TIR handoff data
- fingerprints
- persistent cache keys
- output plans

Use compact command-local identities only when `timers` is enabled:

```rust
#[cfg(feature = "timers")]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TimingBoundaryId(u32);

#[cfg(feature = "timers")]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TimingModuleKey {
    boundary: TimingBoundaryId,
    module_index: u32,
}
```

The exact names may change. The ownership rules may not.

Boundary metadata records:

- boundary kind: source package or main project
- display name such as `@html` or `moth_docs`
- deterministic sort key
- module count

Module metadata records:

- boundary identity
- dense module index inside that boundary
- stable logical module identity
- source file count
- source byte count

Do not use thread-local attribution. Work may run on Rayon workers. Pass or derive compact IDs explicitly through `#[cfg(feature = "timers")]` fields and parameters. Register boundaries and modules in deterministic graph order rather than worker completion order.

### Wall time versus accumulated work

The report must distinguish these concepts:

- **Command and pipeline rows** are wall-clock spans.
- **Boundary rows** are accumulated boundary work. Package inventory and package compilation occur in separate passes, so their human total is the sum of those disjoint spans.
- **Frontend rows** aggregate repeated per-module or per-file observations and are accumulated work.
- **Nested AST rows** are evidence inside the AST parent and must not be counted again when computing top-level command accounting.
- **Slowest module** uses a consistent module-work definition, not the largest single file sample.

---

## Current repository shape at the anchor

At `ce9371bf69f254455fb16d2d0f91df829a593717`:

- `Cargo.toml` defines:
  - `timers = []`
  - `detailed_timers = ["timers"]`
  - `benchmark_counters = []`
- `src/timing.rs` owns the collector, output modes, timing aggregation, counter summary, command collection, RAII guard, no-op stubs and exported macros.
- The basic summary aggregates raw names in a `BTreeMap`, sorts them alphabetically and formats each line into an unstyled `String`.
- The command summary then sends each complete string to `saying::say!`, preventing independent styling of labels, ordinary timings and totals.
- Disabled timer APIs currently use zero-sized tokens, zero-sized guards and no-op functions.
- Macro-wrapped expressions already erase timing work when `timers` is off.
- Manual start/record call sites and guard constructors still compile through disabled stubs rather than disappearing at the call site.
- `module_timing_label(...)` currently constructs a `String` before frontend compilation even in builds without `timers`.
- Build and check commands start and drain command timing collections.
- Dev builds show a useful one-line total but do not run the structured command collection.
- Source-backed packages and the main project already compile through distinct build-system calls, but one batch timer hides the boundary split.
- `frontend.file_prepare` exists on the normal preparation path.
- Incremental directory discovery prepares sources through `ModuleSyntaxDiscovery::prepare_source` without the same aggregate observation.
- `frontend.ast`, `frontend.hir` and borrow metrics are collected per module.
- `ast_build_environment_ms`, `ast_emit_nodes_ms` and `ast_finalize_ms` are detailed-timer-only observations.
- Public-interface projection, generated-function work and some generated borrow work have no coherent basic aggregate.
- HTML backend and output orchestration record many useful raw microstages that should remain hidden in basic mode.
- `backend.js.lower_hir` wraps the entry module only. Linked modules are lowered outside that guard.
- Benchmark parsing requires stable `MOTH_BENCH timing` lines and the matching `command.build.total` or `command.check.total`.
- Repeated metric names are valid and summed by benchmark tooling.

Phase 0 must verify every point because execution may start from a later commit.

---

## Phase 0 verification record (executed at `2877d3012`, 2026-08-05)

Every "Current repository shape" claim above was verified against the actual tree. Two RELEVANT_CODE files
differ from the anchor (`src/build_system/build.rs`, `src/build_system/create_project_modules/compilation.rs`)
through the unrelated `CompiledGraphBoundary::finish` refactor; no timing surface changed.

### Stable timing metric inventory

Command wall spans:

```text
command.build.total, command.build.output_write
command.check.total, command.check.bootstrap, command.check.builder_construction,
command.check.compile_project_frontend, command.check.message_rendering, command.check.path_validation
```

Build-system wall spans:

```text
build_project.total, build_project.bootstrap, build_project.compile_project_frontend,
build_project.backend, build_project.path_validation
bootstrap.total, bootstrap.config_init, bootstrap.symbol_preseed, bootstrap.frontend_surface,
bootstrap.style_directives, bootstrap.load_project_config, bootstrap.backend_config_validate
config.load_total, config.file_exists_check, config.parse_project_config_file,
config.parse.total, config.parse.canonicalize, config.parse.prepare_files_total, config.parse.headers,
config.parse.dependency_sort, config.parse.ast
stage0.single_file.total, stage0.single_file.entry_canonicalize, stage0.single_file.path_resolver,
stage0.single_file.reachable_files, stage0.single_file.string_table_fork, stage0.single_file.compile_module,
stage0.single_file.merge_delta
stage0.directory.total, stage0.directory.path_resolver, stage0.directory.module_inventory,
stage0.directory.module_compile_batch
stage0.reachable_discovery.total, stage0.reachable_discovery.source_load, stage0.reachable_discovery.import_scan,
stage0.reachable_discovery.import_resolve
stage0.source_tree_index.discovery
```

The xtask label map also names `stage0.directory.result_sort`, `stage0.directory.failure_aggregation`,
`stage0.directory.success_merge`, `config.parse.path_resolver`, `config.parse.source_set`,
`stage0.module_root_discovery.total` and `bootstrap.backend_libraries`, but the compiler never records
those names. They are label-map-only and must not appear in the stable inventory.

Repeated frontend accumulated work:

```text
frontend.file_prepare, frontend.header_bind, frontend.dependency_sort, frontend.ast, frontend.hir,
frontend.borrow, frontend.borrow.exact_generated, frontend.module.total
```

Backend and output evidence:

```text
backend.html.total, backend.html.site_config, backend.html.document_config,
backend.html.entry_path_plan, backend.html.module_compile_total, backend.html.external_runtime_assets,
backend.html.external_runtime_glue, backend.html.tracked_assets_plan, backend.html.tracked_assets_emit
backend.js.lower_hir, backend.js.generate_module_glue, backend.js.render_html_document
backend.wasm.total, backend.wasm.lower_wasm, backend.wasm.bootstrap_js, backend.wasm.artifact_assembly
output.write_total, output.preflight, output.prepare_cleanup, output.create_root,
output.emit_files_total, output.emit_file, output.finalize_cleanup
```

Detailed-only evidence (currently recorded only under `detailed_timers`):

```text
ast_build_environment_ms, ast_emit_nodes_ms, ast_finalize_ms,
ast_function_body_parse_ms, ast_start_body_parse_ms, ast_const_template_fold_ms,
ast_const_template_parse_ms, file_prepare_strategy_selection_ms, file_prepare_result_production_ms,
file_prepare_result_sort_ms, file_prepare_string_table_delta_merge_ms, file_prepare_payload_remap_ms,
file_prepare_header_syntax_preparation_ms
```

Benchmark-required names: every name above is a candidate stable record, but the parser hard-requires
`command.build.total` (build) or `command.check.total` (check). Repeated names are summed by xtask.

### Compatibility facts

- Every metric name is a static literal. Storage can move to `&'static str` without an interner.
- `module_timing_label` builds a `String` unconditionally at `frontend_orchestration.rs:2648`, including no-timer
  builds.
- `timed_frontend_stage` and the local `log_*_timing` delegates still call no-op stub functions when `timers` is
  off. `timed_frontend_substep` and the exported expression macros already erase.
- `frontend.file_prepare` exists only on the normal preparation path (`frontend_orchestration.rs:432`); the
  incremental discovery path through `ModuleSyntaxDiscovery::prepare_source` records no aggregate.
- `backend.js.lower_hir` wraps only the entry module (`js_path.rs:124`); linked-module lowering sits outside it.
- Dev builds never start the structured command collection; `build_loop.rs` uses only its baseline one-line
  duration clock.
- In-process frontend benchmarks start a suppressing collection that would discard an active command collection
  if one existed; the two are not nested in any current process path.
- `MOTH_BENCH status` is a separate benchmark-status contract and must not be rejected by the erasure audit.

### Test coverage gap

No tests assert macro erasure, no-timer binary markers, summary rendering, bench-line stability or collector
lifecycle. These arrive with Phases 1 to 3.

---

## Implementation principles

- Prefer one enabled implementation module over scattered `#[cfg]` branches.
- Keep disabled expansion at macro call sites, not no-op runtime functions.
- Use explicit descriptive names and small context structs.
- Keep the timer facade readable. Split collector, summary and rendering responsibilities before the file becomes another large mixed module.
- Avoid a generic tracing framework. Model only the timing facts this report needs.
- Keep stable machine metric names separate from human labels.
- Keep summary policy in a static descriptor table or equivalent typed owner.
- Unknown raw metrics stay available to detailed and benchmark modes but do not appear automatically in basic output.
- Preserve deterministic output independent of event insertion order.
- Do not hide instrumentation gaps with guessed or derived numbers.
- Do not change compiler stage ownership to make timing easier.

---

# Phase 0 - Re-anchor, inventory and freeze the compatibility surface

## Context and goal

This plan is intentionally scheduled later at a natural pause. The first slice must reload the repository rather than trusting the anchor. It freezes the old metric contract before any refactor and records the exact baseline output and feature behaviour.

## Checklist

### Repository and worktree

- [x] Read `AGENTS.md` and the current codebase style, testing and validation guides.
- [x] Run `git status --short`, `git branch --show-current` and `git log -1 --oneline`.
- [x] Record all unrelated changes in the active context capsule.
- [x] Preserve the reported user edit in `docs/src/styles/+package.moth` if it still exists.
- [x] Compare `ce9371bf69f254455fb16d2d0f91df829a593717..HEAD` for every file in `RELEVANT_CODE`.
- [x] Update the current-state section and code map when paths or ownership changed.
- [x] Stop for coordinator review only if current code invalidates an accepted design decision, not for routine file movement.

### Timer and metric inventory

- [x] Inventory every `pipeline_timer!`, labeled timer, manual start/record call, guard and timer logging macro.
- [x] Inventory every stable timing name.
- [x] Classify each existing name as:
  - [x] command wall span
  - [x] build-system wall span
  - [x] repeated frontend accumulated work
  - [x] nested child evidence
  - [x] detailed-only evidence
  - [x] benchmark-required metric
- [x] Find every metric-name reference in `xtask`, tests, docs and benchmark reports.
- [x] Record existing measurement boundaries for metrics that this plan touches.
- [x] Confirm whether every metric name is a static literal. If not, document the exceptions before changing storage to `&'static str`.
- [x] Confirm every in-process benchmark collection entrypoint and whether it can nest with command collection.

### Baseline captures

- [x] Run the current feature matrix:
  - [x] `cargo check --no-default-features`
  - [x] `cargo check --features timers`
  - [x] `cargo check --features detailed_timers`
  - [x] `cargo check --features timers,benchmark_counters`
  - [x] `cargo check --features benchmark_counters`
- [x] Capture one docs build with `MOTH_TIMERS=summary`.
- [x] Capture one docs build with `MOTH_TIMERS=bench`.
- [x] Capture one detailed timer run.
- [x] Capture one successful and one failing check/build path if existing fixtures make this cheap.
- [x] Store captures under an ignored temporary directory, not tracked documentation.
- [x] Run the current `just validate` before implementation and record the result.

### Plan refresh

- [x] Replace the capsule's anchor, branch, worktree and validation state with current facts.
- [x] Add any newly discovered compatibility risks.
- [x] Confirm the plan remains absent from `docs/roadmap/roadmap.md`.

## Phase audit

- [x] A read-only auditor checks that the metric inventory covers every call site and benchmark consumer.
- [x] The auditor checks that no existing metric is marked safe to redefine.
- [x] The coordinator accepts the refreshed capsule before implementation starts.

## Style-guide review

- [x] The inventory uses current stage names and ownership terminology.
- [x] No speculative refactor is mixed into the preflight slice.
- [x] Temporary captures and scripts are not committed.

## Validation gate

- [x] `cargo fmt --all --check`
- [x] Current feature matrix green
- [x] Current `just validate` green or unrelated failures recorded precisely
- [x] Worktree diff contains only the plan refresh if a commit is made

## Phase checkpoint

- [x] Record the accepted commit as `LAST_GOOD_COMMIT`.
- [x] Set `NEXT_ACTION` to Phase 1.

---

# Phase 1 - Build the compile-erasing timer facade

## Context and goal

The zero-cost rule must be enforced before adding any new observations. This phase separates the enabled implementation from the all-build macro facade. It does not yet redesign human output.

The disabled compiler should contain macro expansions that execute only the wrapped production expression. Timer implementation types must not exist in that build.

## Target module shape

A suitable shape is:

```text
src/timing.rs
src/timing/enabled.rs
src/timing/enabled/collector.rs
src/timing/enabled/mode.rs
```

Later phases may add:

```text
src/timing/enabled/summary.rs
src/timing/enabled/render.rs
```

`src/timing.rs` should remain a small facade containing module documentation, conditional re-exports and compile-erasing macros.

## Checklist

### Enabled-only implementation

- [x] Move collector state, output modes, timing observation types, command collection and guards behind `#[cfg(feature = "timers")]`.
- [x] Ensure `TimerOutputMode`, timing snapshots, collector mutexes and timer guards are not defined in a no-timer build.
- [x] Keep counter collection behaviour compatible with the current feature contract.
- [x] Do not make `benchmark_counters` imply `timers` unless the refreshed inventory proves that change is required and the coordinator approves it.
- [x] Store timing durations as `Duration` internally where practical.
- [x] Require static metric names when every call site is static. Otherwise intern dynamic names once per collection rather than allocating per sample.
- [x] Keep stable bench-line rendering in the enabled implementation.

### Compile-erasing macros

- [x] Add a small, consistent macro set for:
  - [x] expression timing
  - [x] expression timing with attribution
  - [x] scope guard timing
  - [x] manual start/finish only where expression or guard timing cannot model control flow
  - [x] command collection start/finish
- [x] Disabled expression-timer expansion must be exactly the wrapped expression.
- [x] Disabled guard, command and manual timer expansions must emit no runtime statement.
- [x] Metric, label, boundary and module expressions must not be evaluated in disabled expansions.
- [x] Do not use `cfg!(feature = "timers")`.
- [x] Avoid adding clever macro syntax that obscures production control flow.

Example contract:

```rust
#[cfg(feature = "timers")]
macro_rules! timed_stage {
    ($metric:expr, $expression:expr) => {{
        let timing_start = std::time::Instant::now();
        let timing_result = $expression;
        $crate::timing::record_stage($metric, timing_start.elapsed());
        timing_result
    }};
}

#[cfg(not(feature = "timers"))]
macro_rules! timed_stage {
    ($metric:expr, $expression:expr) => {{
        $expression
    }};
}
```

The exact names may change.

### Erasure tests

- [x] Add no-feature tests proving that:
  - [x] metric expressions are not evaluated
  - [x] label expressions are not evaluated
  - [x] boundary expressions are not evaluated
  - [x] module expressions are not evaluated
  - [x] wrapped production expressions execute exactly once
  - [x] return values and errors pass through unchanged
- [x] Add timer-enabled tests proving the same wrapped expression still executes exactly once.
- [x] Add a source audit rejecting `cfg!(feature = "timers")` in the timer subsystem.
- [x] Add a source audit that identifies direct timer implementation calls outside the facade.

## Phase audit

- [x] A read-only auditor reviews only the facade and enabled module split.
- [x] The auditor verifies that disabled macros discard every instrumentation argument.
- [x] The auditor verifies that no semantic artefact type gained timing state.

## Style-guide review

- [x] File-level docs explain WHAT the facade owns and WHY disabled macros erase work.
- [x] Collector, mode and macro responsibilities are separate.
- [x] No compatibility wrapper remains solely to preserve an internal no-op API.
- [x] Names are descriptive and the macro set is minimal.

## Validation gate

- [x] `cargo fmt --all --check`
- [x] `cargo test --no-default-features` for erasure tests
- [x] `cargo test --features timers` for enabled macro tests
- [x] Full feature matrix from Phase 0
- [x] `just validate`
- [x] Existing benchmark metric inventory unchanged

## Phase checkpoint

- [x] Refresh the capsule.
- [x] Record the checkpoint commit.
- [x] Do not start Phase 2 until the zero-cost facade design is accepted.

---

# Phase 2 - Migrate existing call sites and prove zero runtime cost

## Context and goal

The enabled module alone is insufficient while production code still calls zero-sized stubs, creates timing labels or carries timing-only values. This phase migrates every current call site to compile-erasing macros and adds the hard binary audit.

No summary redesign starts until this phase is green.

## Checklist

### Call-site migration

- [x] Replace manual `start_pipeline_timing` and `record_started_pipeline_timing` pairs with erasing macros.
- [x] Replace no-op `PipelineTimingGuard` constructors with an erasing scope-guard macro.
- [x] Remove local `log_*_timing` no-op functions from:
  - [x] build orchestration
  - [x] check
  - [x] CLI build
  - [x] output orchestration
  - [x] Stage 0 compilation
- [x] Replace `timed_frontend_stage` with an enabled helper called through a disabled-erasing macro, or paired implementations whose disabled call site evaluates no timing argument.
- [x] Keep detailed human prose gated behind `detailed_timers`.
- [x] Remove disabled timing stubs once no call site requires them.
- [x] Feature-gate timing-only imports, especially `Instant`.
- [x] Keep baseline command-duration clocks that serve normal user output.

### Remove unconditional label and context work

- [x] Make `module_timing_label(...)` timer-only.
- [x] Ensure path display and `String` allocation for timing labels happen only in an enabled macro expansion or enabled block.
- [x] Find every `format!`, `.display()`, `.to_string()` and allocation reachable only from timing.
- [x] Gate every timing-only local, field and function parameter with `#[cfg(feature = "timers")]`.
- [x] Confirm production struct sizes do not include timing IDs when timers are off.
- [x] Confirm function signatures do not carry timing-only arguments when timers are off.

### Hard erasure command

- [x] Add an xtask or repository script named by a `just` recipe such as `just timers-erasure-check`.
- [x] Build a no-feature release compiler in an isolated target directory:
  - [x] `cargo build --release --no-default-features --bin moth`
- [x] Scan the produced binary bytes for timer-only markers including:
  - [x] `MOTH_TIMERS`
  - [x] `MOTH_BENCH timing`
  - [x] `Timing summary:`
  - [x] `Build timings`
  - [x] `Compilation boundaries`
  - [x] at least one new internal timer-only metric marker once later phases add it
- [x] Do not reject `MOTH_BENCH status`, which is a separate benchmark-status contract.
- [x] Run `nm`, `llvm-nm` or the platform equivalent when available and meaningful.
- [x] Treat symbol inspection as supplemental because release stripping may remove symbols.
- [x] Make source-level erasure tests and binary marker absence the portable hard gates.
- [x] Add the erasure command to CI.
- [x] Add it to `just validate` unless the current validation authority requires a separate hard-gate command. Document whichever path is chosen.

### Baseline comparison

- [x] Confirm no-timer command output is byte-for-byte unchanged for representative build and check fixtures.
- [x] Confirm no-timer dev status output is unchanged.
- [x] Confirm timer-enabled bench lines still match the Phase 0 inventory.

## Phase audit

- [x] A fresh read-only auditor searches the full repository for timer calls, timer types, labels and metric strings.
- [x] Every remaining occurrence is classified as:
  - [x] enabled implementation
  - [x] macro invocation erased when disabled
  - [x] baseline user-visible duration unrelated to instrumentation
  - [x] deliberate benchmark-counter code
- [x] The auditor verifies that no timer data entered semantic or persistent artefacts.

## Style-guide review

- [x] Removed no-op wrappers are not replaced with parallel compatibility APIs.
- [x] Production functions remain readable after macro migration.
- [x] Conditional fields and arguments are narrowly scoped.
- [x] Comments explain the zero-cost invariant without repeating macro syntax.

## Validation gate

- [x] `cargo fmt --all --check`
- [x] `cargo check --no-default-features`
- [x] `cargo test --no-default-features`
- [x] Full feature matrix
- [x] `just timers-erasure-check`
- [x] `just validate`
- [x] Benchmark stable-name inventory unchanged

## Phase checkpoint

- [x] Update the capsule with the first proven zero-cost checkpoint.
- [x] Record exact binary markers used by the hard gate.
- [x] Do not continue if any timer-only marker or runtime call remains unexplained.

---

# Phase 3 - Introduce the structured basic-summary model and coloured renderer

## Context and goal

The current basic report is noisy because presentation is derived from raw metric names. This phase introduces a typed human-summary model over the existing observations. It changes presentation only. New package and frontend observations arrive later.

## Checklist

### Structured model

- [x] Add enabled-only types equivalent to:
  - [x] `TimingSummaryReport`
  - [x] `TimingSummarySection`
  - [x] `TimingSummaryRow`
  - [x] `TimingEmphasis`
  - [x] `TimingMeasurementKind`
- [x] Keep wall spans, accumulated work and nested evidence distinct.
- [x] Keep stable metric names separate from human labels.
- [x] Add a static descriptor table or typed mapping for metrics shown in basic mode.
- [x] Unknown metrics must remain in raw snapshots and bench output but stay hidden from basic output.
- [x] Define architecture order explicitly.
- [x] Make report construction deterministic for shuffled input events.

### Summary policy

- [x] Add top-level sections for:
  - [x] command/build pipeline
  - [x] compilation boundaries placeholder
  - [x] frontend work
  - [x] backend
  - [x] slowest module placeholder
- [x] Use existing top-level metrics without redefining them.
- [x] Select one canonical display source where duplicate totals overlap.
- [x] Implement optional-child filtering at `1ms` and `5%`.
- [x] Implement zero-row suppression.
- [x] Implement bounded `Other` computation using wall-clock children only.
- [x] Never subtract accumulated or nested observations from command total.
- [x] Omit empty sections.

### Renderer

- [x] Render rows directly with `saying::say!`.
- [x] Style headings, ordinary values, totals and suffixes separately.
- [x] Add blank lines and indentation through structured rows rather than embedded newline-heavy strings.
- [x] Align timing values without breaking on long labels.
- [x] Keep output useful when colour is disabled by the terminal.
- [x] Do not manually emit ANSI escapes.

### Command outcome

- [x] Let the enabled command-summary finish path know whether the command succeeded or failed.
- [x] Render a partial report after diagnostics on failure.
- [x] Use a title such as `Build timings · failed after ...` without changing stable command metrics.
- [x] Keep benchmark-only mode free of the human report.

### Tests

- [x] Test architecture-defined section order.
- [x] Test optional-child thresholds at exact boundaries.
- [x] Test `Other` inclusion and omission.
- [x] Test no negative `Other`.
- [x] Test zero suppression before rounding.
- [x] Test unknown metrics remain hidden.
- [x] Test raw snapshot preservation.
- [x] Test emphasis/style classification.
- [x] Test deterministic rows from differently ordered events.
- [x] Do not snapshot real timing values.

## Phase audit

- [x] A read-only auditor checks for double-counting and incorrect wall/accumulated classification.
- [x] The auditor confirms no raw metric disappeared from bench mode.
- [x] The auditor confirms summary policy is owned in one place.

## Style-guide review

- [x] Summary construction, policy and terminal rendering are separate modules.
- [x] The renderer contains no compiler-stage logic.
- [x] The policy table is readable and uses full human labels.
- [x] Tests target pure structured data before terminal output.

## Validation gate

- [x] `cargo fmt --all --check`
- [x] Targeted summary and renderer tests
- [x] Full feature matrix
- [x] `just timers-erasure-check`
- [x] `just validate`
- [x] `MOTH_TIMERS=bench` output unchanged
- [x] `MOTH_TIMERS=summary` output matches the new structure

## Phase checkpoint

- [x] Refresh the capsule.
- [x] Attach one successful and one failing summary capture to the handoff.
- [x] Record the checkpoint commit.

---

# Phase 3 correction checkpoint (reviewer findings)

## Findings and resolutions

1. **Blocker, disabled frontend timers executed closure wrappers.** The
   no-timer expansion of `timed_frontend_stage!` called `$stage()`, and
   `timed_frontend_substep` remained a generic `FnOnce` function in
   `frontend_orchestration.rs`. Both now expand directly to the production
   expression: `timed_frontend_stage!` and `timed_frontend_substep!` take a
   direct expression, and every call site passes a block or expression
   instead of a closure.
2. **Basic report reproduced sample-count and max-label noise.** The renderer
   appended `across N samples` and the slowest sample's absolute path label
   to repeated rows. `TimingSummaryRow` no longer carries `sample_count` or
   `max_label`; rows render aggregate duration only. Attribution renders only
   in the dedicated slowest-module section and through explicit row suffixes.
3. **The model could not represent Phase 4.** Row labels are now
   `Cow<'static, str>`, and the report owns `TimingBoundarySummary` and
   `TimingSlowestModuleSummary` placeholders, rendered as empty sections
   until Phase 4 populates them. The frontend title no longer infers module
   count from `frontend.ast`.
4. **Command total presentation.** Headings are command-specific
   (`Build timings 384.63ms` / `Check timings 356.11ms`) with the total in
   the heading; the duplicate `Command total` pipeline row was removed. Raw
   `command.build.total` / `command.check.total` observations are unchanged.
5. **Reloadable plan state.** The status, capsule, commit anchor and Phase 0
   to 3 checklists are refreshed. The unavailable auditor route is recorded
   as a limitation, not an accepted formal audit.
6. **Test coverage.** New tests cover no-timer erasure of both frontend
   macros, discarded `command_timing_finish!` success expressions, build
   versus check headings, total-in-heading, sample-count/max-label
   suppression, dynamic boundary/module labels, explicit row suffixes and
   renderer layout.

## Erasure audit hardening

`just timers-erasure-check` now rejects `$stage()` / `$substep()` closure
expansions and function-wrapper forms of `timed_frontend_substep` in source,
in addition to the existing binary marker scan and direct-call audit.

## Auditor route limitation

Every delegated `auditor` launch during Phases 0, 2 and 3 ended in a launcher
`contract_violation` because the child model referenced out-of-workspace
paths such as `/tmp/`, `/dev/null` or `/target/`. The audit checklist items
for those phases were completed by the coordinator with manual `rg`/script
evidence recorded in the plan. This is a limitation, not a formal auditor
acceptance; Phase 8 must use a working route or an explicit substitute.

## Validation record

- Five `cargo check` feature combos green.
- `cargo test --no-default-features --lib`, `--features timers --lib` and
  `--features detailed_timers --lib` green.
- `cargo test --features timers,benchmark_counters --lib` green except the
  known pre-existing `chunked_file_preparation_skips_identity_payload_remap`
  failure (reproduced at the pristine anchor).
- `cargo test --package xtask` green, including the new erasure-audit tests.
- `just timers-erasure-check` green; `nm` shows no timer symbols in the
  no-timer release binary.
- `just validate` green: cross-target Clippy, 4103 workspace lib tests,
  601 xtask tests, 17 integration-runner tests, 1818/1818 integration
  executions, docs check, `bench-ci` preflight and the erasure gate.
- Smoke captures under `/tmp`: `timers-summary-smoke.txt` (docs build),
  `timers-bench-smoke.txt` (738 stable lines, no human report) and
  `timers-summary-failed.txt` (failing check heading).

## Phase checkpoint

- [ ] Record the correction checkpoint commit.
- [ ] Pause for user review before Phase 4.

---

# Phase 4 - Add project/package boundaries and logical module attribution

## Context and goal

The build system already compiles source-backed packages and the main project as separate graph boundaries. The timer report must expose that architecture without putting timing data into compiled artefacts.

Package inventory and package compilation are disjoint spans. The human boundary total is accumulated work, not one contiguous wall span.

## New stable metrics

Add new names rather than changing existing ones:

```text
build.boundary.inventory
build.boundary.compile
frontend.module.semantic_total
```

Repeated observations are expected and benchmark tooling already sums repeated names.

## Checklist

### Boundary registration

- [x] Register every source-backed package boundary in deterministic package order.
- [x] Register the main project boundary using `project.name`.
- [x] Use display names such as `@html` for source packages.
- [x] Record boundary kind, deterministic order and module count.
- [x] Keep external binding packages out of this section because they are not source compilation boundaries.
- [x] Make the model extensible to future dependency package graphs without implementing package management.

### Boundary instrumentation

- [x] Record `build.boundary.inventory` around each package inventory operation.
- [x] Record the same metric around main-project inventory.
- [x] Record `build.boundary.compile` around each package `compile_module_waves` call.
- [x] Record the same metric around main-project compilation.
- [x] Keep `stage0.directory.module_inventory` and `stage0.directory.module_compile_batch` unchanged for compatibility.
- [x] Build the human boundary total by summing inventory and compile observations for that boundary.
- [x] Label the section `accumulated work`.

### Explicit context propagation

- [x] Add `#[cfg(feature = "timers")]` boundary fields only to ephemeral build contexts that need attribution.
- [x] Pass compact IDs explicitly through package/project compilation.
- [x] Do not use thread-local boundary state.
- [x] Verify future Rayon worker movement cannot lose context.
- [x] Verify fields and arguments disappear from no-timer builds.

### Module registration

- [x] Key modules by boundary plus dense module ID.
- [x] Derive display identity from `StableModuleOriginIdentity` or graph-owned logical identity.
- [x] Do not reconstruct module identity from absolute paths.
- [x] Register or update source file count and source bytes when preparation facts are available.
- [x] Keep deterministic metadata independent of worker completion order.
- [x] Add a single-file synthetic project/module mapping.

### Consistent slowest-module basis

- [x] Record `frontend.module.semantic_total` around `compile_module_semantic` in every compilation mode.
- [x] Do not change `frontend.module.total`.
- [x] Define module work as:
  - [x] source preparation attributed to the module
  - [x] plus `frontend.module.semantic_total`
- [x] Use this definition for the slowest-module row.
- [x] Show logical identity, source file count and source size.
- [x] Hide absolute filesystem paths in basic mode.
- [x] Keep absolute paths available to detailed output when useful.

### Tests

- [x] Separate `@html` and main-project totals.
- [x] Several source packages sort deterministically.
- [x] Same dense module ID in two boundaries does not collide.
- [x] Shuffled event insertion does not change output.
- [x] Module labels contain no checkout-specific prefix.
- [x] No package row is emitted for binding-backed packages.
- [x] Boundary totals are labelled accumulated rather than wall time.

## Phase audit

- [x] A read-only auditor traces every timing ID through Stage 0 and frontend orchestration.
- [x] The auditor verifies timing state remains command-local and cfg-gated.
- [x] The auditor checks package/project ordering against the graph's deterministic order.

## Style-guide review

- [x] Boundary context is a small typed struct, not loose booleans or strings.
- [x] Existing build-system ownership remains unchanged.
- [x] Comments explain why totals are accumulated across separate passes.
- [x] No timing concern leaks into `CompiledGraphBoundary` or `ProjectCompilation`.

## Validation gate

- [x] `cargo fmt --all --check`
- [x] Targeted Stage 0 and timing attribution tests
- [x] Full feature matrix
- [x] `just timers-erasure-check`
- [x] `just validate`
- [x] `just bench-frontend-check`
- [x] Basic docs-build summary shows `@html` and the main project separately

## Phase checkpoint

- [x] Refresh the capsule.
- [x] Record boundary output from the docs project.
- [x] Record the checkpoint commit (`c1603fd06`).

### Docs-project boundary smoke (2026-08-05, `MOTH_TIMERS=summary build docs`)

```text
Compilation boundaries · accumulated work
@html         1 module   16.77ms
html_project  69 modules 1396.62ms

Slowest module
html_project/docs/packages/builder/canvas  89.26ms · 3 files · 26.0KB
```

### Phase 4 audit record

- [x] `just validate` green (cross-target Clippy, workspace tests, 1818/1818
  integration executions, docs check, bench-ci, timers-erasure-check).
- [x] Manual audit with `rg` evidence: every boundary id originates in
  `compile_directory_frontend` or `compile_single_file_frontend`, flows into
  `compile_module_waves` and `DirectoryModuleCompileContext`, and reaches
  frontend stages only through `TimingModuleAttribution`/`TimingModuleContext`
  parameters. No thread-local state exists.
- [x] Old label-only recording API removed: `timed_manual_finish_labeled!` and
  `record_started_pipeline_timing_with_label` have zero remaining call sites.
- [x] New metric markers absent from the no-timer release binary
  (timers-erasure-check).
- [x] Package/project ordering verified against `source_package_boundaries()`
  prefix order plus project-after-packages registration, pinned by unit and
  build-system tests.
- [x] Known unrelated failure remains: `chunked_file_preparation_skips_identity_payload_remap`
  under `timers,benchmark_counters` (reproduced at pristine anchor).

---

# Phase 5 - Fill the critical frontend timing gaps

## Context and goal

The basic report currently over-exposes config and backend microsteps while hiding source preparation, AST/TIR subphases, public-interface work and generated-function work. This phase adds coarse observations at existing ownership boundaries.

It must not create a second pass over AST, HIR or TIR. Time work where it already happens.

## Source preparation

- [ ] Keep the existing `frontend.file_prepare` measurement boundary on the normal preparation path.
- [ ] Add the same stable metric to incremental directory discovery.
- [ ] Attribute every incremental source-preparation observation to its owning module and boundary.
- [ ] Include final retained-header-syntax preparation in the module's preparation aggregate.
- [ ] Do not add another source scan.
- [ ] Update benchmark documentation if the exact directory-path coverage was previously undocumented.
- [ ] Keep the existing metric name because this extends availability to a missing path, not the old path's boundary.

## AST/TIR aggregate promotion

- [ ] Record these existing metrics whenever `timers` is enabled:
  - [ ] `ast_build_environment_ms`
  - [ ] `ast_emit_nodes_ms`
  - [ ] `ast_finalize_ms`
- [ ] Keep their current measurement boundaries.
- [ ] Keep existing inline prose gated by `detailed_timers`.
- [ ] Prevent double recording when `detailed_timers` is enabled.
- [ ] Use these human labels:
  - [ ] `Environment, types and constants`
  - [ ] `Bodies and TIR construction`
  - [ ] `Template and constant finalization`
- [ ] Keep fine-grained environment, folding and normalization observations detailed-only.

## Public-interface work

- [ ] Add a new stable `frontend.public_interface` metric.
- [ ] Record repeated non-overlapping observations for:
  - [ ] draft and canonical surface construction
  - [ ] post-borrow finalization and interface closure
- [ ] Attribute each observation to module and boundary.
- [ ] Do not time unrelated AST or HIR work inside this metric.
- [ ] Let the human summary sum repeated observations.

## Borrow validation

- [ ] Keep `frontend.borrow` unchanged.
- [ ] Keep `frontend.borrow.exact_generated` unchanged.
- [ ] Add a new metric for generated-sidecar borrow rechecks if they are currently unobserved, such as `frontend.borrow.generated`.
- [ ] Define the human `Borrow validation` row as the sum of direct borrow-check calls only.
- [ ] Do not add an outer borrow span that overlaps generated-function materialization.

## Generated functions

- [ ] Add a new stable `frontend.generated_functions` aggregate around generated-function materialization work not already represented by base AST/HIR rows.
- [ ] Decide and document whether generated borrow calls are nested children or excluded from this aggregate.
- [ ] Avoid showing overlapping generated and borrow rows as additive siblings.
- [ ] Attribute generated work to the requesting module and consuming boundary.
- [ ] Keep generated sidecars immutable and free of timing data.

## Summary integration

- [ ] Populate the curated frontend section in architecture order.
- [ ] Show AST children only when they pass the significance threshold.
- [ ] Show generated functions only when non-zero.
- [ ] Mark the section accumulated.
- [ ] Ensure frontend rows are explanatory evidence and are not subtracted from command wall time.
- [ ] Use `frontend.module.semantic_total` plus preparation for slowest-module calculation rather than summing nested stage rows.

## Tests

- [ ] Directory builds now contain `frontend.file_prepare`.
- [ ] Single-file and directory module-work definitions agree.
- [ ] AST aggregate metrics appear with `timers` but inline prose remains detailed-only.
- [ ] `detailed_timers` does not double-record AST aggregates.
- [ ] Public-interface repeated samples sum correctly.
- [ ] Generated borrow work is classified once.
- [ ] Slowest module includes preparation and semantic compilation.
- [ ] Basic summary contains TIR and constant-finalization labels.

## Phase audit

- [ ] A frontend-focused read-only auditor checks every new timing scope against actual stage ownership.
- [ ] The auditor checks that no source, AST, HIR or TIR walk was added for timing.
- [ ] The auditor checks for overlapping sibling metrics and double recording.

## Style-guide review

- [ ] Timing wrappers do not obscure the semantic pipeline.
- [ ] New helpers remain stage-local.
- [ ] Comments describe non-obvious measurement boundaries and consumers.
- [ ] Detailed-only microtimers remain out of basic policy.

## Validation gate

- [ ] `cargo fmt --all --check`
- [ ] Targeted frontend orchestration, AST and timing tests
- [ ] Full feature matrix
- [ ] `just timers-erasure-check`
- [ ] `just validate`
- [ ] `just bench-frontend-check`
- [ ] Existing stable metric boundaries verified against the Phase 0 inventory

## Phase checkpoint

- [ ] Refresh the capsule.
- [ ] Save a docs-build summary demonstrating the frontend section.
- [ ] Record the checkpoint commit.

---

# Phase 6 - Curate backend and output reporting without losing raw detail

## Context and goal

The HTML backend, config parser and output writer have useful microtimers, but basic mode should show only major work and significant children. Most of this phase is policy. The only known measurement gap is linked-module JS lowering.

## Checklist

### Preserve raw detail

- [ ] Keep existing HTML backend, config and output metric names and boundaries.
- [ ] Keep all raw observations available in bench and detailed modes.
- [ ] Do not delete microtimers merely because basic mode hides them.

### Basic backend policy

- [ ] Show the build-system backend total as the major row.
- [ ] Add optional significant children for:
  - [ ] JS lowering
  - [ ] HTML document rendering
  - [ ] Wasm lowering when active
  - [ ] runtime/tracked assets only when significant
- [ ] Hide site config, document config, path planning and zero-cost glue/asset rows by default.
- [ ] Avoid showing both `build_project.backend` and `backend.html.total` as additive siblings.

### Linked JS lowering

- [ ] Preserve `backend.js.lower_hir` as entry-module lowering only.
- [ ] Add `backend.js.lower_linked_hir` around linked-module lowering.
- [ ] Attribute repeated linked observations to the active entry or boundary when useful.
- [ ] Aggregate both raw metrics under the human label `JS lowering`.
- [ ] Update benchmark references and tests for the new metric.
- [ ] Do not rename or broaden the old metric.

### Output policy

- [ ] Show `Write output` as one major row.
- [ ] Keep preflight, cleanup, root creation, emission and finalization hidden in basic mode.
- [ ] Allow significant output children only if a real project demonstrates that the extra row improves diagnosis.
- [ ] Do not add filesystem counters to basic timers.

### Tests

- [ ] Linked-module lowering is observed separately.
- [ ] Existing entry-module metric remains unchanged.
- [ ] Basic output contains no HTML config microstep flood.
- [ ] Detailed and bench output retain every raw metric.
- [ ] Wasm and JS modes produce deterministic section order.

## Phase audit

- [ ] A backend-focused auditor checks the exact old and new JS lowering boundaries.
- [ ] The auditor verifies that hidden raw metrics still reach benchmark snapshots.
- [ ] The auditor checks that summary rows do not double-count backend totals.

## Style-guide review

- [ ] Human grouping stays in timing policy rather than backend code.
- [ ] Backend code adds only the missing measurement scope.
- [ ] No generic backend tracing abstraction is introduced.

## Validation gate

- [ ] `cargo fmt --all --check`
- [ ] Targeted HTML JS, linked module, Wasm and output tests
- [ ] Full feature matrix
- [ ] `just timers-erasure-check`
- [ ] `just validate`
- [ ] `just bench-check`
- [ ] `just bench-frontend-check`

## Phase checkpoint

- [ ] Refresh the capsule.
- [ ] Capture JS and Wasm basic summaries.
- [ ] Record the checkpoint commit.

---

# Phase 7 - Integrate build, check and dev command presentation

## Context and goal

Build and check already own command collection. Dev needs one collection per build cycle. The structured report must appear after the existing command or dev status output, never before diagnostics.

The dev headline total is build-and-write work only. Watcher polling, state updates and reload broadcasting remain outside it.

## Build and check

- [ ] Start command collection through an erasing macro.
- [ ] Finish it through an erasing macro after normal diagnostics or success output.
- [ ] Pass success/failure state to the enabled renderer.
- [ ] Keep `command.build.total` unchanged.
- [ ] Keep `command.check.total` unchanged, including its existing message-rendering boundary.
- [ ] Do not standardize old command totals by silently redefining them.
- [ ] Show only stages applicable to the command.
- [ ] Check must not show backend or output sections when those stages did not run.

## Dev initial build and rebuilds

- [ ] Start a fresh command timing collection for every initial build and rebuild.
- [ ] Ensure the previous cycle is always drained on success or failure.
- [ ] Wrap `DevBuildExecutor::build_and_write` with the new stable metric:
  - [ ] `command.dev.build_and_write`
- [ ] Exclude watch polling, state mutation, error-page rendering and SSE broadcasting from that metric.
- [ ] Keep the existing one-line dev status and its existing whole-cycle duration.
- [ ] Print the structured report after the dev status line.
- [ ] Print successful warnings and diagnostics in the existing order.
- [ ] Print a partial structured report for failed builds.
- [ ] Verify initial and watch-triggered builds use the same path.
- [ ] If a detailed full-cycle metric is useful, add `command.dev.cycle` only under detailed timers and keep it out of basic totals.
- [ ] Do not print any new output in a binary built without `timers`.

## Collection safety

- [ ] Define behaviour for an attempted nested collection.
- [ ] Prefer a structured internal error or test failure over silently replacing an active collection.
- [ ] Keep in-process benchmark suppression working.
- [ ] Ensure repeated dev builds do not retain observations, strings or metadata from earlier cycles.
- [ ] Recover safely if diagnostics or build failure exits early.

## Tests

- [ ] Build success and failure ordering.
- [ ] Check success and failure ordering.
- [ ] Dev initial build summary.
- [ ] Dev rebuild summary.
- [ ] Dev failed rebuild partial summary.
- [ ] One collection per cycle with no cross-cycle leakage.
- [ ] No timer output when `MOTH_TIMERS=off`.
- [ ] Bench mode emits machine lines without the human report.
- [ ] Summary mode emits the human report without stable bench-line noise.
- [ ] Verbose mode retains detailed prose and ends with the curated report.
- [ ] No-feature command output remains unchanged.

## Phase audit

- [ ] A command-orchestration auditor checks all early returns and failure paths.
- [ ] The auditor verifies dev build-and-write boundaries exclude state and broadcast work.
- [ ] The auditor checks collector lifecycle and suppression.

## Style-guide review

- [ ] Command orchestration remains readable.
- [ ] Timer lifecycle helpers do not duplicate build or dev architecture.
- [ ] Error handling uses existing command and compiler error lanes.
- [ ] No user-input panic is introduced.

## Validation gate

- [ ] `cargo fmt --all --check`
- [ ] Targeted CLI, check and dev-server tests
- [ ] Full feature matrix
- [ ] `just timers-erasure-check`
- [ ] `just validate`
- [ ] Manual initial dev build and one rebuild smoke test with summary mode

## Phase checkpoint

- [ ] Refresh the capsule.
- [ ] Capture build, check and dev examples.
- [ ] Record the checkpoint commit.

---

# Phase 8 - Documentation, benchmark compatibility and final closeout

## Context and goal

The final phase makes the behaviour reloadable for future maintainers, proves compatibility and closes the plan without changing roadmap order or the language progress matrix.

## Documentation

- [ ] Update `benchmarks/README.md` with:
  - [ ] basic, detailed and bench product boundaries
  - [ ] output mode matrix
  - [ ] wall time versus accumulated work
  - [ ] boundary and slowest-module attribution
  - [ ] child significance thresholds
  - [ ] zero-cost compile-time-erasure rule
  - [ ] stable metric compatibility rule
- [ ] Update `Cargo.toml` feature comments.
- [ ] Update `src/timing.rs` and enabled-module docs.
- [ ] Update `docs/src/docs/codebase/style-guide/validation.mtf` if `just validate` gains the erasure gate.
- [ ] Update `AGENTS.md` only if it enumerates validation commands that changed.
- [ ] Keep `docs/compiler-design-overview.md` unchanged unless implementation accidentally created an architectural instrumentation boundary that must be documented. Prefer fixing the leak.
- [ ] Keep `docs/build-system-design.md` unchanged for the same reason.
- [ ] Mark this plan complete and replace the capsule with the final state.

## Roadmap and progress matrix

- [ ] Do **not** add this plan to `docs/roadmap/roadmap.md`.
- [ ] Do **not** reorder the queued implementation chain.
- [ ] Do **not** add a progress-matrix row. Timers are compiler developer tooling, not source-language or backend feature support.
- [ ] Leave future tracing, allocation profiling, CI dashboards and broad benchmark tooling under the existing deferred benchmarking/profiling owner.
- [ ] Record in this plan that roadmap insertion remains a separate coordinator action.

## Compatibility audit

- [ ] Compare the final existing-metric inventory against Phase 0.
- [ ] Confirm every old name still exists where its old path executes.
- [ ] Confirm every old measurement boundary is unchanged.
- [ ] Confirm new metrics use new names.
- [ ] Confirm no benchmark protocol version was bumped.
- [ ] If an old boundary changed unintentionally, restore it. Do not hide the change with a protocol bump.
- [ ] Run benchmark parser and report tests.
- [ ] Run read-only CLI and frontend benchmark suites.
- [ ] Inspect local report output for new metrics and no malformed records.
- [ ] Confirm measured iterations expose stable metric sets per case.

## Final zero-cost proof

- [ ] Run `just timers-erasure-check` from a clean target directory.
- [ ] Run no-feature unit and integration tests.
- [ ] Confirm no timer-only release-binary markers.
- [ ] Confirm no direct no-op timer calls remain.
- [ ] Confirm no timer-only field exists without a cfg gate.
- [ ] Confirm no timing label is constructed outside enabled code.
- [ ] Confirm no timer data enters semantic artefacts, fingerprints or serialization.
- [ ] Confirm no-feature CLI and dev output remain unchanged.

## Final audits

- [ ] Fresh architecture auditor:
  - [ ] stage ownership
  - [ ] boundary attribution
  - [ ] no semantic leakage
  - [ ] deterministic parallel behaviour
- [ ] Fresh performance/erasure auditor:
  - [ ] disabled macro expansion
  - [ ] no-feature binary markers
  - [ ] no runtime timer work
- [ ] Fresh benchmark auditor:
  - [ ] names and boundaries
  - [ ] parser/report compatibility
  - [ ] no protocol drift
- [ ] Final style-guide review:
  - [ ] files remain modular
  - [ ] comments are useful rather than noisy
  - [ ] no compatibility shims
  - [ ] tests are in appropriate test modules
  - [ ] output code uses `saying::say!`

## Final validation

- [ ] `cargo fmt --all --check`
- [ ] Full feature matrix
- [ ] `cargo test --no-default-features`
- [ ] `cargo test --features timers`
- [ ] `cargo test --features detailed_timers`
- [ ] `just timers-erasure-check`
- [ ] `just validate`
- [ ] `just bench-check`
- [ ] `just bench-frontend-check`
- [ ] docs project check
- [ ] docs project development build
- [ ] docs project release build
- [ ] manual dev rebuild smoke test

## Closeout

- [ ] Record starting and final commits.
- [ ] Record every new metric name.
- [ ] Record confirmation that old metric boundaries did not change.
- [ ] Record final sample output.
- [ ] Record validation commands and results.
- [ ] Record all deliberately deferred items.
- [ ] Leave the worktree clean except for explicitly preserved user changes.
- [ ] Stop for coordinator acceptance.

---

## Stable metric plan

### Existing metrics that must not be redefined

This is a minimum list. Phase 0 owns the complete list.

```text
command.build.total
command.check.total
build_project.total
build_project.bootstrap
build_project.compile_project_frontend
build_project.backend
stage0.directory.total
stage0.directory.module_inventory
stage0.directory.module_compile_batch
frontend.file_prepare
frontend.header_bind
frontend.dependency_sort
frontend.ast
frontend.hir
frontend.borrow
frontend.borrow.exact_generated
frontend.module.total
backend.html.total
backend.html.module_compile_total
backend.js.lower_hir
backend.js.generate_module_glue
backend.js.render_html_document
output.write_total
```

### Planned new metrics

Names may be corrected during Phase 0 for repository conventions, but an accepted new name must remain stable after implementation.

```text
build.boundary.inventory
build.boundary.compile
frontend.module.semantic_total
frontend.public_interface
frontend.borrow.generated
frontend.generated_functions
backend.js.lower_linked_hir
command.dev.build_and_write
```

`command.dev.cycle` is optional detailed-only evidence.

### Existing detailed metrics promoted to basic collection

```text
ast_build_environment_ms
ast_emit_nodes_ms
ast_finalize_ms
```

Promotion means they are recorded whenever `timers` is enabled. Their old scope must remain unchanged and detailed builds must not record them twice.

---

## Required summary classification

| Human row | Measurement kind | Raw source |
|---|---|---|
| Command total | Wall time | Existing command total or `command.dev.build_and_write` |
| Bootstrap | Wall time | Existing build/check bootstrap span |
| Discover and prepare graph | Wall time | Existing Stage 0 inventory span |
| Compile packages and project | Wall time | Existing module compile batch |
| Boundary total | Accumulated work | `build.boundary.inventory` + `build.boundary.compile` |
| Prepare source files | Accumulated work | `frontend.file_prepare` |
| Bind headers | Accumulated work | `frontend.header_bind` |
| Order declarations | Accumulated work | `frontend.dependency_sort` |
| Semantic frontend / AST | Accumulated parent | `frontend.ast` |
| AST child rows | Nested evidence | promoted AST aggregate metrics |
| Public interface | Accumulated work | `frontend.public_interface` |
| HIR | Accumulated work | `frontend.hir` |
| Borrow validation | Accumulated work | direct borrow metrics |
| Generated functions | Accumulated or nested, fixed by Phase 5 | `frontend.generated_functions` |
| Backend | Wall time | `build_project.backend` |
| JS lowering | Accumulated child | entry + linked JS lowering metrics |
| HTML rendering | Accumulated child | `backend.js.render_html_document` |
| Write output | Wall time | command/output total selected by policy |
| Slowest module | Accumulated module work | preparation + `frontend.module.semantic_total` |

The summary builder must not infer these categories from dotted-name prefixes.

---

## Validation command matrix

| Purpose | Command |
|---|---|
| Default no-feature compile | `cargo check --no-default-features` |
| Basic timers compile | `cargo check --features timers` |
| Detailed timers compile | `cargo check --features detailed_timers` |
| Timers and counters compile | `cargo check --features timers,benchmark_counters` |
| Counter-only compatibility | `cargo check --features benchmark_counters` |
| Disabled macro tests | `cargo test --no-default-features` |
| Enabled timer tests | `cargo test --features timers` |
| Detailed-timer tests | `cargo test --features detailed_timers` |
| Hard erasure audit | `just timers-erasure-check` |
| Repository gate | `just validate` |
| CLI benchmark compatibility | `just bench-check` |
| Frontend benchmark compatibility | `just bench-frontend-check` |
| Basic human smoke | `MOTH_TIMERS=summary cargo run --features timers -- build docs` |
| Stable machine smoke | `MOTH_TIMERS=bench cargo run --features timers -- build docs` |
| Verbose smoke | `MOTH_TIMERS=verbose cargo run --features detailed_timers -- build docs` |

Adjust invocation syntax to current repository commands during Phase 0.

---

## Deliberately deferred and outside this plan

These are not hidden TODOs.

- Historical comparison in the basic timer report
- Pass/fail performance thresholds
- Persistent timer history
- Public performance dashboards
- CI regression gates beyond the zero-cost erasure invariant
- Flamegraphs, tracing spans and causal span trees
- Allocation profiler integration
- User-configurable summary sections, colours or thresholds
- Full per-module tables in basic mode
- Interactive drill-down
- Exact exclusive-time reconstruction across parallel tasks
- Automatic incremental cache-hit reporting
- Source-backed package HIR caching
- JS minification and tree shaking
- Ownership, drop or ABI specialization metrics
- Exact binary-size equality or assembly equality between unrelated builds
- Package manager or dependency graph implementation
- Roadmap insertion for this plan
- Progress-matrix tracking for developer timers

The existing benchmark and profiling roadmap remains the owner for broader performance tooling. This plan must not duplicate it.

---

## Risks and required mitigations

### Metric compatibility drift

**Risk:** a convenient refactor broadens an existing metric.

**Mitigation:** Phase 0 freezes boundaries. Every new scope gets a new name. Final audit compares the inventory.

### Double-counted frontend rows

**Risk:** AST children, generated borrow work and outer semantic spans appear as additive siblings.

**Mitigation:** typed measurement kinds and explicit parent relationships. Slowest-module work uses preparation plus one semantic total.

### Misleading package totals

**Risk:** package inventory starts long before package compilation, so a long-lived guard includes unrelated work.

**Mitigation:** record separate inventory and compile spans. Sum them as accumulated boundary work.

### Parallel nondeterminism

**Risk:** worker completion order changes IDs or output order.

**Mitigation:** key modules by boundary and dense graph ID. Sort by deterministic metadata. Never use registration or event insertion order as display authority.

### Hidden no-feature overhead

**Risk:** a no-op function, zero-sized guard, label formatter or cfg-gated field survives in production code.

**Mitigation:** erasing macros, no-feature tests, repository-wide source audit and release-binary marker scan.

### Collector lifecycle in dev

**Risk:** one rebuild overwrites or retains another rebuild's collection.

**Mitigation:** one explicit start/drain per cycle, nested-collection checks and cross-cycle tests.

### Symbol audit portability

**Risk:** stripped release binaries expose no useful symbols.

**Mitigation:** source-level erasure and binary string markers are hard gates. Symbol tools are supplemental.

---

## Definition of done

This plan is complete only when:

- no-timer builds have proven zero timer-system runtime cost
- the release-binary marker audit is a hard repository gate
- basic output is short, structured, coloured and deterministic
- command total is obvious
- source packages and main project have separate accumulated totals
- frontend source preparation, AST/TIR, public interface, HIR, borrow and generated work are readable
- slowest module uses logical identity and consistent work
- HTML/config/output microstages no longer flood basic output
- linked-module JS lowering is measured under a new metric
- build, check, initial dev and rebuild paths share the report when requested
- detailed and machine output retain full evidence
- old stable metric names and boundaries remain intact
- benchmark protocol is unchanged
- docs are current
- roadmap and progress matrix remain intentionally unchanged
- every phase has an accepted checkpoint, audit, style review and validation record
