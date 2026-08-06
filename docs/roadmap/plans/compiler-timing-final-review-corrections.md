# Compiler timing final-review correction plan

## Status

* **Plan state:** active - Phase 0 baseline in progress
* **Repository anchor:** `68d9633bf351aff413a64437a367a945b8be6d78`
* **Suggested path:** `docs/roadmap/plans/compiler-timing-final-review-corrections.md`
* **Roadmap status:** do not add this follow-up to the roadmap
* **Primary invariant:** builds without `timers` have zero timer-system runtime cost
* **Compatibility invariant:** preserve every existing stable metric name and successful-path measurement boundary

## Goal

Harden the completed timing system after final review:

1. Make collection sessions impossible to corrupt through nesting or stale attribution
2. Reduce timer-enabled self-interference
3. Correct AST and command ownership
4. Strengthen compile-time erasure against future regressions
5. Simplify the timing API and summary policy
6. Correct misleading labels and remaining layout issues

This remains a crude real-project timing report. Do not turn it into tracing, profiling or benchmark-history infrastructure.

## Non-goals

* No language semantics changes
* No compiler architecture changes made solely for timers
* No changes to semantic artefacts, HIR, TIR, fingerprints or output plans
* No thread-local attribution
* No existing stable metric rename
* No silent change to an existing successful measurement boundary
* No benchmark protocol bump unless an unavoidable incompatibility is found and escalated
* No roadmap insertion
* No flamegraph, allocation-profiler or persistent-history work

---

## Active context capsule

```text
ACTIVE_PLAN:
- `docs/roadmap/plans/compiler-timing-final-review-corrections.md`

CURRENT_SLICE:
- Phase: 0 (complete - baseline frozen and validated)
- Checklist item: all Phase 0 items accepted
- Goal: begin Phase 1 session ownership after this checkpoint

LAST_GOOD_COMMIT:
- `68d9633bf351aff413a64437a367a945b8be6d78` (correction baseline anchor)
- `3233df2d4` (user `docs regen` commit on top of the anchor)

CURRENT_WORKTREE_STATE:
- Clean; branch `main`; HEAD `3233df2d4`, one commit ahead of `origin/main`
- Preserve the user `docs regen` commit; no unrelated changes present

RELEVANT_CODE:
- `src/timing.rs`
- `src/timing/enabled.rs`
- `src/timing/enabled/collector.rs`
- `src/timing/enabled/mode.rs`
- `src/timing/enabled/attribution.rs`
- `src/timing/enabled/summary.rs`
- `src/timing/enabled/render.rs`
- `src/compiler_frontend/compiler_messages/compiler_dev_logging.rs`
- `src/compiler_frontend/ast/mod.rs`
- `src/compiler_frontend/ast/module_ast/environment/builder.rs`
- `src/compiler_frontend/ast/generic_functions/materialisation.rs`
- `src/build_system/project_config/parsing.rs`
- `src/build_system/create_project_modules/compilation.rs`
- `src/build_system/create_project_modules/module_inventory.rs`
- `src/build_system/create_project_modules/frontend_orchestration.rs`
- `src/projects/cli.rs`
- `src/projects/check.rs`
- `src/projects/dev_server/build_loop.rs`
- `src/projects/dev_server/server.rs`
- `src/projects/html_project/js_path.rs`
- `src/projects/html_project/wasm/artifacts.rs`
- `xtask/src/timers_erasure_check.rs`
- `benchmarks/README.md`

ACCEPTANCE_CRITERIA:
- Nested collection attempts never replace or drain an outer session
- Command kind is explicit, never inferred from metric presence
- Bench and Silent command modes don't build unused timing snapshots
- Timer mode is not read from the environment for every observation
- AST children contain module AST work only
- Detailed prose and raw observations use the same elapsed Duration
- No stale or invalid context can attach to another command
- Existing MOTH_BENCH names and successful boundaries remain unchanged
- No-timer builds contain no timer-system clocks, state, context, labels or metric strings
- Broad timing dead-code allowances are removed
- Summary accounting has one owner
```

---

# Phase 0 - Refresh and freeze the correction baseline

## Context

The previous plan is complete and heavily validated. This phase records the exact final state so the correction work cannot accidentally redefine metrics while refactoring infrastructure.

## Checklist

* [x] Read `AGENTS.md` and the current style, testing and validation guides.
* [x] Confirm `HEAD`, branch and worktree state.
* [x] Compare `68d9633bf351aff413a64437a367a945b8be6d78..HEAD`.
* [x] Record every existing timing metric and exact measurement scope.
* [x] Record every command start and finish path.
* [x] Record every direct `start_pipeline_timing` and timer-specific `Instant::now`.
* [x] Record every timing-only field and parameter.
* [x] Capture:

  * [x] docs summary
  * [x] docs bench output
  * [x] docs verbose output
  * [x] single-file build summary
  * [x] single-file check summary
  * [x] failed build summary
  * [x] initial dev summary
  * [x] generic-heavy project summary
  * [x] config-heavy project summary
* [x] Record the raw metric-name set for each capture.
* [x] Confirm the plan remains outside the roadmap.

## Audit

* [x] Verify the baseline inventory against compiler call sites and xtask consumers.
* [x] Mark metrics that share a start token.
* [x] Mark metrics that are nested, accumulated or command-accounting wall spans.
* [x] Identify every old metric whose successful boundary must remain byte-for-byte conceptually identical.

## Validation

* [x] Five-feature `cargo check` matrix
* [x] `just timers-erasure-check`
* [x] `just bench-check`
* [x] `just bench-frontend-check`
* [x] `just validate`

Stop and checkpoint the refreshed context before implementation.

---

# Phase 1 - Replace global collection ownership with explicit sessions

## Context

The current collector can silently replace an active production scope. Dense boundary and module IDs also carry no collection generation. This phase fixes lifecycle correctness before optimizing or simplifying call sites.

## Design

Add timer-enabled-only identities equivalent to:

```rust
struct TimingSessionId(u64);

struct TimingSession {
    id: TimingSessionId,
    command: Option<TimingCommandKind>,
    purpose: TimingCollectionPurpose,
    finished: bool,
}

enum TimingCollectionPurpose {
    HumanSummary,
    RawBenchmark,
}

enum TimingContext {
    Boundary(TimingBoundaryId),
    Module(TimingModuleKey),
}
```

Boundary and module IDs must contain or be validated against the owning session generation.

## Checklist

* [x] Replace `start_collection` and detached `stop_and_collect` with an owned session API.
* [x] Starting while another session is active must:

  * [x] preserve the active session
  * [x] return a rejected or inactive inner session
  * [x] never overwrite observations
  * [x] never panic while holding the mutex
* [x] Finishing must drain only the matching session ID.
* [x] Dropping an unfinished session must clean up only its matching active scope.
* [x] A stale session must not drain a newer scope.
* [x] Replace the sentinel boundary with session-scoped inactive state, or include the session generation in the sentinel/key validation.
* [x] Replace two independent context options with a typed `TimingContext`.
* [x] Make boundary and module key fields private.
* [x] Validate boundary existence before module registration.
* [x] Reject duplicate module registration within one boundary.
* [x] Derive boundary module count from registered module records, or validate the retained counter against them.
* [x] Recover deliberately from mutex poisoning rather than silently returning empty data.
* [x] Add explicit `Build`, `Check` and `Dev` command kind to command sessions.
* [x] Remove command-kind inference from raw metric scans.
* [x] Keep every new session type, field and argument under `#[cfg(feature = "timers")]`.

## Tests

* [x] Nested start preserves all outer events.
* [x] Nested finish cannot drain the outer session.
* [x] Mismatched finish cannot drain another session.
* [x] Dropped unfinished session leaves no active state.
* [x] Context from session A is rejected during session B.
* [x] Duplicate boundary/module registrations fail deterministically.
* [x] Parallel observations from one valid session remain deterministic.
* [x] No test relies on filtering unrelated collector pollution.
* [x] Timing tests use a timing-owned test lock, not the frontend counter lock.

## Audit and style review

* [x] No timer session identity enters semantic or persistent compiler data.
* [x] No thread-local attribution was introduced.
* [x] Collector state has one clear owner.
* [x] No panic occurs while the collector lock is held.
* [x] No production path silently ignores a lifecycle mismatch.

## Validation

* [x] Targeted collector and command tests
* [x] Full feature matrix
* [x] `just timers-erasure-check`
* [x] `just validate`

Checkpoint before Phase 2.

---

# Phase 2 - Cache output policy and remove unused collection work

## Context

The current implementation reads environment mode and touches the global mutex multiple times per observation. Bench and Silent commands collect snapshots that no consumer reads.

## Checklist

### Mode ownership

* [x] Parse `MOTH_TIMERS` once per process or command session.
* [x] Parse `MOTH_COUNTERS` once per process or command session.
* [x] Keep parsing as a pure function for tests.
* [x] Do not mutate or repeatedly query environment variables during stage recording.
* [x] Preserve all existing default-mode behavior.

### Collection purposes

* [x] `Summary`:

  * [x] collect timing events
  * [x] collect boundary/module metadata
  * [x] print no stable timing lines
* [x] `Bench`:

  * [x] emit stable timing lines
  * [x] do not build a command snapshot
  * [x] do not allocate boundary/module display metadata
* [x] `Verbose`:

  * [x] collect for the final summary
  * [x] emit stable lines and detailed prose
* [x] `Silent`:

  * [x] do not start a command snapshot
  * [x] print nothing
* [x] Explicit in-process benchmark sessions:

  * [x] always collect raw metrics
  * [x] suppress stdout
  * [x] skip boundary/module metadata unless a caller explicitly requests attribution
* [x] Preserve counter-summary collection when `MOTH_COUNTERS=summary` requires it.

### Fast recording path

* [x] Avoid locking when no snapshot sink is active.
* [x] Acquire at most one collector lock per recorded event.
* [x] Drop the lock before terminal output.
* [x] Avoid a second lock solely to read output suppression.
* [x] Make boundary-name construction lazy so Bench, Silent and raw-only sessions don't evaluate `format!` or clone project names.
* [x] Keep stable bench line ordering unchanged unless separately approved.

### Counter cleanup

* [x] Store counter names as `&'static str`.
* [x] Convert to `String` only at the public frontend benchmark boundary.
* [x] Remove the unused counter label field.
* [x] Preserve counter-only feature behavior.

## Tests

* [x] Bench command records no snapshot events.
* [x] Silent command records no snapshot events.
* [x] Summary and Verbose collect the expected events.
* [x] Lazy boundary display expressions aren't evaluated in Bench or Silent mode.
* [x] Explicit raw benchmark collection works regardless of command output mode.
* [x] Output suppression requires no second collector lock.
* [x] Stable bench lines remain unchanged.

## Audit and style review

* [x] The fast path is readable and doesn't become a generic tracing system.
* [x] No new dependency is added solely for event storage.
* [x] Future Rayon workers can record safely.
* [x] No-timer builds remain unaffected because the entire implementation is feature-gated.

## Validation

* [x] Targeted mode and collector tests
* [x] Bench-line parser tests
* [x] `just bench-check`
* [x] `just bench-frontend-check`
* [x] `just timers-erasure-check`
* [x] `just validate`

Checkpoint before Phase 3.

---

# Phase 3 - Correct timing ownership and elapsed-value consistency

## Context

AST children currently include config and generated materialisation observations that aren't inside the displayed `frontend.ast` parent. Some scopes also call `elapsed()` more than once or fail to record child evidence on errors.

## Checklist

### AST timing ownership

* [x] Add a timer-only AST owner/context to `AstBuildContext` or `AstPhaseContext`.
* [x] Base module AST construction carries its `TimingModuleContext`.
* [x] Config AST construction carries a distinct non-module owner or no module context.
* [x] Generated materialisation carries a distinct generated owner or no module context.
* [x] Keep existing raw names:

  * [x] `ast_build_environment_ms`
  * [x] `ast_emit_nodes_ms`
  * [x] `ast_finalize_ms`
* [x] Keep their successful measurement boundaries unchanged.
* [x] Preserve config and generated observations in raw benchmark/detailed output.
* [x] Make the basic AST children aggregate only module-frontend observations.
* [x] Do not add config/generated observations beneath `frontend.ast`.
* [x] Add a defensive invariant: a nested AST child set that cannot be related to the module parent must be omitted or diagnosed in tests, never silently presented as nested evidence.

### Failure-safe AST timing

* [x] Replace finish-only AST timing with result-aware expression timing or finishable guards.
* [x] Record failed environment, emission and finalization work.
* [x] Preserve the successful stop point of each existing metric.
* [x] Remove keep-alive `let _ = start` workarounds.

### One elapsed value per scope

* [x] Change `timed_frontend_stage!` to compute one `Duration`.
* [x] Pass that same value to:

  * [x] the raw observation
  * [x] stable bench output
  * [x] detailed prose
* [x] Do the same for AST and detailed substep helpers.
* [x] Add a multi-record duration helper for intentionally shared boundaries.
* [x] In single-file compilation, record `build.boundary.compile` and `stage0.single_file.compile_module` from the same captured `Duration`.
* [x] Search for every other start token recorded more than once and correct it.

### Dev command ownership

* [x] Move `command.dev.build_and_write` from `ProjectBuildExecutor` to the orchestration around `DevBuildExecutor::build_and_write`.
* [x] Remove the concrete implementation guard.
* [x] Ensure fake and alternate executors receive the same metric.
* [x] Store `Dev` explicitly in the session.
* [x] Decide the failed-title wording when failure occurs after build-and-write, such as missing entry-page validation:

  * [x] prefer `Dev timings · failed` plus the build-and-write total
  * [x] do not imply the displayed total covers post-build failure handling

### Optional public-interface clarification

* [x] Keep `frontend.public_interface` unchanged.
* [x] Consider new nested metrics:

  * [x] `frontend.public_interface.projection`
  * [x] `frontend.public_interface.finalization`
* [x] Show them only as significant children of the existing human row.
* [x] Do not redefine the existing aggregate.

## Tests

* [x] Expensive config AST work never enters frontend AST children.
* [x] Generated materialisation environment work never enters frontend AST children.
* [x] Raw old AST metrics still contain their historical config/generated observations.
* [x] Failed AST stages produce child timing evidence.
* [x] Detailed prose duration equals the stored observation duration.
* [x] Shared single-file metrics receive exactly the same duration.
* [x] Every `DevBuildExecutor` implementation receives one dev total.
* [x] A post-build dev failure is titled accurately.

## Audit and style review

* [x] No existing raw metric was renamed.
* [x] No successful raw boundary moved.
* [x] New owner data is timer-only and command-local.
* [x] Generated AST work remains classified under generated functions in the human report.
* [x] No second AST walk was introduced.

## Validation

* [x] Config-heavy fixture
* [x] Generic-heavy fixture
* [x] Failed AST fixtures
* [x] Real and fake dev executors
* [x] Full feature matrix
* [x] `just bench-check`
* [x] `just bench-frontend-check`
* [x] `just timers-erasure-check`
* [x] `just validate`

Checkpoint before Phase 4.

---

# Phase 4 - Consolidate the compile-erasing macro surface

## Context

The hard zero-cost rule is easier to maintain when timer clocks can only originate inside feature-selected macros or guards. Direct manual starts spread the proof across many call sites.

## Target facade

Prefer a small set equivalent to:

```text
timed_stage!(metric, expression)
timed_stage_attributed!(metric, context, expression)
timing_scope!(binding, metric)
timing_scope_attributed!(binding, metric, context)
record_timing_duration!(metric, duration)
record_attributed_duration!(metric, duration, context)
command_timing_scope!(binding, command_kind)
```

Disabled expression arms must be the production expression itself. Disabled scope and record arms emit no statement and evaluate no instrumentation argument.

## Checklist

* [ ] Replace ordinary manual start/finish pairs with expression macros.
* [ ] Use named RAII guards only for scopes with many early returns.
* [ ] Require callers to name scope guards. Do not inject one fixed hidden binding.
* [ ] Replace all compatible duplicate start-token recording with one captured `Duration`.
* [ ] Remove production direct calls to `start_pipeline_timing`.
* [ ] Keep the start function internal to macro expansion.
* [ ] Confirm `pipeline_timer!` and `labeled_pipeline_timer!` have no production consumers.
* [ ] Delete unused macros and their helper functions.
* [ ] Move timer-specific detailed-output predicates into `timing`, rather than having timing macros depend on compiler frontend logging.
* [ ] Move `timed_ast_stage!` into the timing facade.
* [ ] Keep token, AST dump, HIR dump and unrelated developer logging in `compiler_dev_logging`.
* [ ] Replace `pub(crate) use enabled::*` with explicit re-exports.
* [ ] Split counter summary and command-session code out of the large `enabled.rs`.

## Erasure tests

For every remaining macro:

* [ ] metric expression isn't evaluated without the feature
* [ ] context expression isn't evaluated
* [ ] label/prose expression isn't evaluated
* [ ] wrapped production expression executes once
* [ ] `Ok` and `Err` pass through unchanged
* [ ] guards introduce no field, token or function call
* [ ] command kind/session expressions disappear
* [ ] no timer-only type must exist for disabled expansion to compile

## Audit and style review

* [ ] No direct timer clock start remains outside the facade.
* [ ] No runtime `cfg!` controls timing.
* [ ] Macro names and argument order are consistent.
* [ ] Production control flow remains readable.
* [ ] No compatibility wrapper is retained without a consumer.

## Validation

* [ ] No-feature and timer macro suites
* [ ] Cross-target Clippy
* [ ] Full feature matrix
* [ ] `just timers-erasure-check`
* [ ] `just validate`

Checkpoint before Phase 5.

---

# Phase 5 - Strengthen the zero-cost proof

## Context

The current marker scan is useful but manual. Structural checks should make an unguarded timer clock or context propagation difficult to introduce.

## Checklist

### Source audit

* [ ] Reject direct `start_pipeline_timing` calls outside the timing facade.
* [ ] Reject timer-owned `Instant::now` calls outside cfg-selected timer macros and guards.
* [ ] Maintain a narrow allowlist for baseline user-visible clocks:

  * [ ] build `Done in`
  * [ ] check `Done in`
  * [ ] dev status duration
  * [ ] benchmark harness clocks
* [ ] Reject timing-only struct fields without `#[cfg(feature = "timers")]`.
* [ ] Reject timing-only function parameters without that cfg.
* [ ] Reject direct calls into collector or enabled modules.
* [ ] Reject runtime `cfg!(feature = "timers")`.
* [ ] Reject disabled macro arms that call closures or helpers.

### Binary audit

* [ ] Keep the clean no-feature release build.
* [ ] Centralize timer-only marker ownership.
* [ ] Ensure every newly introduced stable metric is covered automatically or by a required inventory test.
* [ ] Scan for:

  * [ ] environment-variable names
  * [ ] human headings
  * [ ] collector/session type markers where retained
  * [ ] representative metrics from every subsystem
* [ ] Keep symbol or LLVM checks supplemental.
* [ ] Do not require exact assembly or binary-size equality.

### Build matrix

* [ ] `--no-default-features`
* [ ] `timers`
* [ ] `detailed_timers`
* [ ] `timers,benchmark_counters`
* [ ] `benchmark_counters`
* [ ] no-timer release binary
* [ ] cross-target Clippy

## Acceptance

A no-timer build contains:

* no timer clock reads
* no timer environment lookup
* no collector or session state
* no timer context field or argument
* no boundary/module label allocation
* no renderer
* no timer metric strings
* no no-op timer call

Checkpoint before Phase 6.

---

# Phase 6 - Simplify summary policy and correct presentation

## Context

Display policy, accounting and special grouped rows currently have separate owners. Consolidating them prevents future `Other` drift and removes dead model fields.

## Checklist

### One policy owner

* [ ] Define each human row from one descriptor containing:

  * [ ] display label
  * [ ] raw source metric or metrics
  * [ ] command visibility
  * [ ] section
  * [ ] wall, accumulated or nested relationship
  * [ ] whether it contributes to command accounting
  * [ ] optional parent
* [ ] Build `Other` from rows marked as non-overlapping command children.
* [ ] Delete the separate hardcoded accounting list.
* [ ] Validate policy definitions at test time:

  * [ ] no duplicate raw source without explicit aggregation
  * [ ] no overlapping command-accounting rows
  * [ ] nested rows have a parent
  * [ ] command total never appears as its own pipeline row
  * [ ] unknown metrics remain hidden
* [ ] Make `TimingMeasurementKind` enforce policy behavior or remove it.

### Remove dead model surface

* [ ] Remove the broad `allow(dead_code)`.
* [ ] Fix every resulting warning directly.
* [ ] Remove unused row emphasis states.
* [ ] Remove unused suffix support.
* [ ] Use `&'static str` for static policy row labels.
* [ ] Keep owned `String` only for dynamic boundary/module identities.
* [ ] Use `TimingBoundaryKind` for meaningful policy or remove it.
* [ ] Replace the `TimingMetricSummary` wrapper with `Duration` if no extra state remains.

### Correct labels

* [ ] Rename `Wasm lowering` to `Wasm build` when sourced from `backend.wasm.total`.
* [ ] Either:

  * [ ] rename the existing tracked row to `Tracked asset emission`
  * [ ] or aggregate disjoint planning and emission metrics as `Tracked assets`
* [ ] Keep JS entry and linked lowering aggregated as `JS lowering`.
* [ ] Clarify that Public interface is accumulated projection and finalization work.

### Layout

* [ ] Handle `1 module` and `N modules`.
* [ ] Compute row width recursively across nested children.
* [ ] Pad the boundary module-count column.
* [ ] Label binary source sizes as `KiB`, or divide by 1000 for `KB`.
* [ ] Bound very long logical identities without losing their unique tail.
* [ ] Use `Pipeline`, `Build pipeline`, `Check pipeline` or `Dev pipeline` consistently.
* [ ] Detect accounted wall time greater than command total:

  * [ ] never print a fabricated zero `Other`
  * [ ] suppress `Other` and expose an internal test/debug invariant failure

## Tests

* [ ] Complete top-level order
* [ ] Recursive AST child alignment
* [ ] Boundary column alignment
* [ ] singular/plural text
* [ ] KiB formatting
* [ ] long logical identity
* [ ] Wasm and tracked-asset labels
* [ ] invalid wall accounting
* [ ] policy duplicate/overlap validation
* [ ] all existing threshold boundaries

## Audit and style review

* [ ] Summary policy remains a static, data-oriented model.
* [ ] No generic tracing abstraction is introduced.
* [ ] Renderer contains formatting only.
* [ ] Summary construction contains no terminal styling.

## Validation

* [ ] Summary unit tests
* [ ] JS and Wasm HTML integration tests
* [ ] docs summary smoke
* [ ] single-file summary smoke
* [ ] dev summary smoke
* [ ] `just validate`

Checkpoint before closeout.

---

# Phase 7 - Compatibility, documentation and final closeout

## Compatibility audit

* [ ] Diff the final stable metric-name set against Phase 0 and the correction baseline.
* [ ] Verify every existing successful measurement boundary.
* [ ] Verify old `MOTH_BENCH timing` line formatting.
* [ ] Verify command totals remain:

  * [ ] `command.build.total`
  * [ ] `command.check.total`
  * [ ] `command.dev.build_and_write`
* [ ] Verify entry JS lowering remains `backend.js.lower_hir`.
* [ ] Verify linked lowering remains separate.
* [ ] Verify old AST raw metrics remain available.
* [ ] Record every new metric introduced by this correction.
* [ ] Do not bump the protocol unless the audit finds an unavoidable incompatibility.

## Documentation

* [ ] Update `benchmarks/README.md` with session and mode behavior.
* [ ] Explain that Bench and Silent command modes don't build human snapshots.
* [ ] Document AST timing ownership and filtering.
* [ ] Record that directory `frontend.file_prepare` coverage began with the completed timer-plan checkpoint, so old records may omit it.
* [ ] Update the original plan closeout to reference this correction plan.
* [ ] Update validation documentation if the erasure audit gains new checks.
* [ ] Do not add either plan to the roadmap.
* [ ] Do not update the language progress matrix.

## Final validation

* [ ] `cargo fmt --all --check`
* [ ] Five-feature `cargo check` matrix
* [ ] No-feature tests
* [ ] Timers tests
* [ ] Detailed-timers tests
* [ ] Timers plus counters tests
* [ ] Xtask tests
* [ ] Clean-target `just timers-erasure-check`
* [ ] `just bench-check`
* [ ] `just bench-frontend-check`
* [ ] `just validate`
* [ ] docs check
* [ ] docs release build
* [ ] successful and failed build/check smokes
* [ ] single-file smoke
* [ ] config-heavy smoke
* [ ] generic-heavy smoke
* [ ] initial dev smoke
* [ ] watch-triggered rebuild smoke where the environment permits it

## Final audits

* [ ] Zero-cost erasure audit
* [ ] Collector lifecycle and concurrency audit
* [ ] Metric compatibility audit
* [ ] AST ownership audit
* [ ] Summary/accounting audit
* [ ] Style-guide review
* [ ] Clean worktree review

## Definition of done

* No-timer builds remain structurally zero cost
* Nested sessions cannot destroy another report
* Bench and Silent modes avoid unused collection work
* Each observation performs no unnecessary environment query or collector lock
* AST children are true module-AST children
* Command kind and session ownership are explicit
* Shared timing boundaries use one captured duration
* Failed stages retain useful partial timing evidence
* No stale context can cross command sessions
* Summary accounting has one owner
* Dead timing APIs and fields are removed
* Human labels describe the measured work accurately
* Existing stable raw metrics remain compatible
* The correction plan is marked complete and the worktree is clean

---

The system does **not** need a redesign or rollback. The package/project attribution, direct-expression erasure, curated report shape and dev integration are all good foundations. The follow-up should focus on collector ownership, measurement purity and deleting machinery that the finished design no longer uses.

---

## Phase 0 baseline record (2026-08-06)

Repository state:

- HEAD `3233df2d4` (user `docs regen`), branch `main`, worktree clean, one
  commit ahead of `origin/main`; `68d9633bf..HEAD` changes only generated
  `docs/release/**` HTML (user commit, preserved).
- Plan remains unlinked from `docs/roadmap/roadmap.md`.

Stable raw metric inventory (names and successful measurement scopes frozen
for the correction):

- Command totals: `command.build.total` (whole build command),
  `command.build.output_write`, `command.check.total`,
  `command.check.message_rendering`, `command.check.path_validation`,
  `command.check.builder_construction`, `command.check.bootstrap`,
  `command.check.compile_project_frontend`, `command.dev.build_and_write`
  (inside `ProjectBuildExecutor::build_and_write`), `command.dev.cycle`
  (detailed-only whole dev cycle).
- Pipeline: `build_project.bootstrap`, `build_project.backend`,
  `output.write_total`, `stage0.directory.module_inventory`,
  `stage0.directory.module_compile_batch`, `stage0.single_file.total`
  (single-file fallback), plus detailed-only `stage0.*` and `output.*`
  micro-metrics.
- Frontend: `frontend.file_prepare`, `frontend.header_bind`,
  `frontend.dependency_sort`, `frontend.ast`,
  `ast_build_environment_ms`, `ast_emit_nodes_ms`, `ast_finalize_ms`,
  `frontend.hir`, `frontend.public_interface`, `frontend.borrow`,
  `frontend.borrow.exact_generated`, `frontend.borrow.generated`,
  `frontend.generated_functions`, `frontend.module.semantic_total`,
  `frontend.module.total`.
- Boundaries: `build.boundary.inventory`, `build.boundary.compile`
  (attributed per source package and main project).
- Backend: `backend.js.lower_hir` (entry), `backend.js.lower_linked_hir`
  (linked modules), `backend.js.generate_module_glue`,
  `backend.js.render_html_document`, `backend.wasm.total`,
  `backend.wasm.lower_wasm`, `backend.wasm.artifact_assembly`,
  `backend.wasm.bootstrap_js`, `backend.html.total`,
  `backend.html.site_config`, `backend.html.document_config`,
  `backend.html.entry_path_plan`, `backend.html.module_compile_total`,
  `backend.html.external_runtime_assets`, `backend.html.external_runtime_glue`,
  `backend.html.tracked_assets_plan`, `backend.html.tracked_assets_emit`.

Command start/finish paths:

- Build: `src/projects/cli.rs` `run_build_command` uses
  `command_timing_start!` / `command_timing_finish!` and manual
  `command.build.total` start/finish.
- Check: `src/projects/check.rs` `run_check` uses the same command macros plus
  manual per-stage starts.
- Dev: `src/projects/dev_server/build_loop.rs` `run_single_build_cycle` starts
  one command scope per cycle; the server renders the drained snapshot after
  its status line.

Direct timer starts and clocks (baseline):

- `start_pipeline_timing()` call sites: `source_discovery.rs`,
  `source_tree_index.rs`, `create_project_modules/compilation.rs`,
  `project_config/parsing.rs`, `project_config.rs`, `output/orchestrator.rs`,
  `projects/cli.rs`, `projects/check.rs`, `projects/dev_server/build_loop.rs`,
  plus `timing.rs` macro expansions.
- Detailed-timer `Instant::now` substep starts: AST environment builder,
  type resolution, emission, finalization (`module_ast/**`).
- Normal user-visible clocks (baseline allowlist): CLI `Done in`
  (`cli.rs`, `check.rs`), dev status duration (`build_loop.rs`),
  benchmark harness (`benchmarking/frontend.rs`), integration runner
  (`compiler_tests/.../runner.rs`).

Timing-only fields/parameters: `TimingModuleContext` (two independent
`Option` fields), `TimingObservation.boundary/module`, boundary and module
record tables, `BenchmarkObservationMetric.label`, `TimingMetricSummary`,
summary model rows and policy table, renderer, counter groups. All live under
`#[cfg(feature = "timers")]` except the `timing.rs` facade no-op surface.

Smoke captures (baseline, `detailed_timers` build with explicit `MOTH_TIMERS`):

- `/tmp/timers-correction-summary-smoke.txt` - docs build summary: total
  1645.73ms; boundaries `@html 1 module` / `html_project 69 modules`;
  `Frontend work · 70 modules`.
- `/tmp/timers-correction-bench-smoke.txt` - 1999 stable `MOTH_BENCH timing`
  lines, no human report; bench mode still builds and drains a snapshot.
- `/tmp/timers-correction-verbose-smoke.txt` - 6193 lines, inline prose plus
  the full summary.
- `/tmp/timers-correction-single-build-smoke.txt` - single-file build:
  pipeline fallback `Compile frontend` renders after `Write output`;
  `Frontend work · 1 modules` (singular bug); synthetic boundary row has an
  empty label.
- `/tmp/timers-correction-single-check-smoke.txt` - single-file check summary.
- `/tmp/timers-correction-failed-smoke.txt` - failing single-file check:
  `Check timings · failed after`; AST row has no child evidence because the
  finish-style AST aggregates are skipped on error.
- `/tmp/timers-correction-dev-smoke.txt` - initial dev build: `Dev timings
  1636.30ms` printed after the status line.
- `/tmp/timers-correction-generic-heavy-smoke.txt` - generic-heavy temp
  project; `Generated functions 5.12ms` while `ast_build_environment_ms`
  also includes generated materialisation (baseline for the AST-ownership
  finding).
- `/tmp/timers-correction-config-heavy-smoke.txt` - config-heavy temp project
  with the full known config-key set; config parsing runs through `Ast::new`
  and contributes to `ast_*` aggregates (baseline for the AST-ownership
  finding).

Baseline issues confirmed by the captures (all match the review findings):

1. Nested production collections silently replace the active collection.
2. AST children are not strict children of `frontend.ast` (config and
   generated work contribute).
3. Bench and Silent modes still build and discard command snapshots; every
   observation locks the collector and re-reads `MOTH_TIMERS`.
4. Command kind is inferred from metric presence in the renderer.
5. Shared start tokens call `elapsed()` twice (frontend stage prose) and
   single-file records two metrics from one start in sequence.
6. Failed AST stages omit child evidence.
7. `Frontend work · 1 modules` and other layout/label issues (boundary column
   alignment, KB vs KiB, pipeline fallback order, long identities,
   `Build pipeline` used for Check/Dev).
8. Broad `allow(dead_code)` on the enabled module hides unused APIs.
9. Erasure gate marker list is manual; source audit does not reject
   unguarded timer clocks.

Phase 0 validation: five-feature `cargo check` matrix, `just
timers-erasure-check`, `just bench-check`, `just bench-frontend-check` and
`just validate` run before the Phase 1 checkpoint (baseline unchanged from the
completed plan; see Phase 8 record of the original plan for the last full
gate).

---

## Phase 1 checkpoint record (2026-08-06)

Implemented and validated:

- Owned `TimingSession` API: `start_benchmark_collection` and new
  `start_command_session` return session tokens; `finish()` drains only the
  matching active scope; `Drop` abandons an unfinished session; nested starts
  return rejected inactive tokens that preserve the outer session.
- Session-scoped ids: `TimingBoundaryId` and `TimingModuleKey` carry the
  session generation; `TimingContext` is a typed
  `Boundary | Module` enum replacing the two-option context struct.
- Collector validates context session generation, boundary existence,
  duplicate module registration (ignored idempotently) and derives boundary
  module counts from registered records at finish time; mutex poisoning is
  recovered via `into_inner` with no panic while holding the lock.
- Command kind is explicit: `command_timing_start!(session, kind)` binds a
  session and `render_command_timing_summary(snapshot, command, succeeded)`
  never scans metrics to infer Build/Check/Dev.
- Timing tests now use the timing-owned `lock_timing_tests` guard instead of
  the frontend counter-test lock, and add coverage for nested starts,
  mismatched finish, dropped sessions, stale context rejection and duplicate
  registration.
- `compiler_dev_logging` re-export and in-process benchmark callers updated
  to the owned-session API.

Validation (Phase 1 gate):

- Five-feature `cargo check` matrix green.
- `cargo test --features timers --lib`: 4138 passed.
- `cargo test --features detailed_timers --lib`: 4139 passed.
- `cargo test --no-default-features --lib`: 4096 passed.
- `cargo test --features benchmark_counters --lib`: 4098 passed.
- `cargo test --features timers,benchmark_counters --lib`: 4144 passed plus
  the known unrelated `chunked_file_preparation_skips_identity_payload_remap`
  failure (reproduced at the baseline anchor).
- `cargo test --package xtask`: 601 passed.
- `just timers-erasure-check`: clean.
- `just validate`: green (cross-target Clippy, workspace tests, integration
  executions, docs check, bench-ci, erasure gate).
- Smokes: docs build/check summaries, single-file build/check, failed check
  and initial dev summary all render with the explicit command kinds.

Audit (coordinator-led): the delegated `auditor` route again ended in a
launcher `contract_violation` (child referenced an out-of-workspace command
path), matching the limitation recorded in the original plan. Coordinator
audit with `rg` evidence confirms: no session types outside `src/timing/**`,
no thread-local attribution, one collector owner, no panic under the collector
lock, no metric-based command inference, and no-timer erasure green.

---

## Phase 2 checkpoint record (2026-08-06)

Implemented and validated:

- `MOTH_TIMERS` and `MOTH_COUNTERS` are parsed once per process into
  mutex-protected caches with pure `from_env` parsers and test overrides;
  stage recording never re-reads the environment.
- Command sessions start only in Summary/Verbose modes; Bench and Silent
  modes return rejected sessions and never build a snapshot or allocate
  boundary/module metadata.
- Recording takes one collector lock per event and returns the suppression
  flag from that same lock; stable bench lines and verbose prose print after
  the lock is dropped. `record_pipeline_timing`, attributed variants,
  `log_benchmark_timing`, `log_benchmark_counter` and the frontend macros all
  use the single-lock path.
- Boundary display names are lazy (`impl FnOnce() -> String`) and are not
  evaluated when no session stores them.
- Explicit raw benchmark sessions (`start_raw_benchmark_collection`) record
  every metric while skipping boundary/module record tables; the frontend
  benchmark harness uses this path.
- Counter metric names are `&'static str`; the unused `label` field is
  removed; owned `String` conversion happens only at the public frontend
  benchmark boundary.
- New tests: bench/silent command sessions rejected, summary session
  collects, raw benchmark skips metadata, lazy boundary names, static counter
  names, counter-mode test override.

Validation (Phase 2 gate):

- Five-feature `cargo check` matrix green.
- `cargo test --features timers --lib`: 4143 passed.
- `cargo test --features detailed_timers --lib`: 4144 passed.
- `cargo test --no-default-features --lib`: 4096 passed.
- `cargo test --features benchmark_counters --lib`: 4098 passed.
- `cargo test --features timers,benchmark_counters --lib`: 4150 passed plus
  the known unrelated `chunked_file_preparation_skips_identity_payload_remap`
  failure.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `just bench-check` and `just bench-frontend-check`: green.
- `just timers-erasure-check`: clean.
- `just validate`: green.
- Smokes: `MOTH_TIMERS=bench` still emits 1999 stable lines with no human
  report; `MOTH_TIMERS=summary` unchanged; `off`/`silent` print no timing
  output.

Audit (coordinator-led): delegated `auditor` route remains unusable
(`contract_violation`); coordinator audit with `rg` evidence confirms one
collector lock per recorded event, no environment reads in the record path,
lazy boundary names, and no-timer erasure green.

---

## Phase 3 checkpoint record (2026-08-06)

Implemented and validated:

- AST timing ownership: `AstBuildContext`/`AstPhaseContext` carry a
  cfg-gated `timing_context`; module AST construction threads the module
  context through `headers_to_ast`, while config parsing and generated
  materialisation pass `None`. The basic summary's AST children aggregate
  only module-attributed `ast_*` observations; config/generated work stays
  in raw and detailed output.
- Failure-safe AST timing: `AstStageTimingGuard` records
  `ast_build_environment_ms`, `ast_emit_nodes_ms` and `ast_finalize_ms` on
  every exit, including early-return error paths; the keep-alive
  `let _ = start` workarounds are removed.
- One elapsed value per scope: `timed_frontend_stage!` captures one
  `Duration` and reuses it for the raw observation, stable bench line and
  verbose prose; `record_pipeline_timing_multi` records shared-boundary
  metrics (single-file `build.boundary.compile` +
  `stage0.single_file.compile_module`) from one captured duration.
- Dev command ownership: `command.dev.build_and_write` moved from
  `ProjectBuildExecutor` into `build_once` around the
  `DevBuildExecutor::build_and_write` trait call, so every executor
  implementation receives the metric; failed titles now read
  `Dev timings · failed` (and the same for Build/Check).
- Public interface clarification: `frontend.public_interface.projection`
  and `frontend.public_interface.finalization` are recorded from the same
  durations as the aggregate and shown as significant children.
- New tests: AST guard records on drop, multi-record shared duration,
  config/generated AST observations never enter module AST children.

Validation (Phase 3 gate):

- Five-feature `cargo check` matrix green.
- `cargo test --features timers --lib`: 4146 passed.
- `cargo test --features detailed_timers --lib`: 4147 passed.
- `cargo test --no-default-features --lib`: 4096 passed.
- `cargo test --features benchmark_counters --lib`: 4098 passed.
- `cargo test --features timers,benchmark_counters --lib`: 4152 passed plus
  the known unrelated `chunked_file_preparation_skips_identity_payload_remap`
  failure.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `just timers-erasure-check`: clean.
- `just validate`: green.
- Smokes: docs summary shows module-only AST children and Public interface
  Projection/Finalization children; failed AST check records
  `ast_build_environment_ms`/`ast_emit_nodes_ms` raw evidence; single-file
  shared metrics carry identical durations; dev summary renders
  `Dev timings`.

Audit (coordinator-led): delegated `auditor` route remains unusable
(`contract_violation`); coordinator audit with `rg` evidence confirms AST
context flows only through cfg-gated fields, no raw metric renamed, no
successful boundary moved, and no second AST walk introduced.
