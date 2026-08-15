# Command Timing Accounting and Reporting Correction Plan

> **Repository path:**
> `docs/roadmap/plans/command-timing-accounting-and-reporting-correction-plan.md`
>
> **Implementation branch:**
> `agent/timing-accounting-correction-plan`
>
> **Status:**
> Ready for implementation.
>
> **Planning snapshot:**
> `main` at `c77dfa0f3f5decd98ce64682d65f8977973cfb06`.

## Purpose

Make every reported command duration answer one unambiguous question, remove duplicated and
self-perturbing detailed timers and leave a reliable timing baseline for subsequent constant-folding
optimisation.

The implementation must produce:

- one clock owner for each user-visible command duration
- one explicit execution-to-presentation boundary
- exact agreement between a success-line duration and its structured command total
- command totals that exclude terminal, benchmark and timer rendering
- schema-versioned compatibility after the command-boundary correction
- unique, attributed constant-sensitive detailed metrics
- concise reports that distinguish wall-clock accounting from overlapping accumulated evidence
- no timer-system work in builds without `timers`, beyond the command stopwatch already required for
  user-facing duration output

This plan changes measurement semantics, not compiler or build semantics. Old and new command totals
must be treated as non-comparable.

---

## Active context capsule

ACTIVE_PLAN:
- `docs/roadmap/plans/command-timing-accounting-and-reporting-correction-plan.md`

CURRENT_SLICE:
- Phase: 0
- Goal: freeze current output and timer lifecycle before changing accounting
- Non-goals: no frontend optimisation and no benchmark speed claims

LAST_GOOD_COMMIT:
- `c77dfa0f3f5decd98ce64682d65f8977973cfb06`

RELEVANT_DOCS:
- `AGENTS.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `benchmarks/README.md`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`

RELEVANT_CODE:
- `src/projects/cli.rs`
- `src/projects/check.rs`
- `src/projects/dev_server/build_loop.rs`
- `src/projects/command_status.rs`
- `src/timing.rs`
- `src/timing/enabled/schema.rs`
- `src/timing/enabled/session.rs`
- `src/timing/enabled/runtime.rs`
- `src/timing/enabled/summary.rs`
- `src/timing/enabled/render.rs`
- `src/timing/tests/`
- `src/compiler_frontend/ast/module_ast/environment/builder.rs`
- `src/compiler_frontend/ast/module_ast/environment/type_resolution.rs`
- `src/compiler_frontend/ast/module_ast/emission/emitter.rs`
- `src/compiler_frontend/ast/module_ast/finalization/finalizer.rs`
- `src/build_system/create_project_modules/frontend_orchestration.rs`
- `xtask/src/benchmark_run.rs`
- `xtask/src/benchmark_observation/`

NEXT_ACTION:
- execute Phase 0 and checkpoint the baseline tests before changing command timing ownership

---

## Current defects

### Two command clocks answer different questions

`build` captures its success duration before printing the success message, then finishes
`command.build.total` after success and warning rendering. `check` similarly returns an execution
duration from `execute_check`, while the structured command guard includes message rendering.

The two values differ by terminal work plus the small offset between independent start points. The
human line and structured total must not share a label while using different boundaries.

### Dev reports a third boundary

`command.dev.build_write` measures the executor's build and output write. The human `Dev build done
in` stopwatch currently wraps the complete cycle, including state mutation and reload broadcast.
`command.dev.cycle` already exists for that broader detailed boundary. The status line should use the
build/write duration or be labelled as a cycle duration, not silently mix both.

### Detailed probes duplicate and perturb parents

Constant resolution starts nested detailed timers with the same label, so verbose output appears to
report two independent constant costs. Legacy `timer_log!` calls print synchronously while parent
spans are active, making enclosing detailed numbers include terminal I/O. Per-file `Files Prepared`
lines also flood verbose output while duplicating the typed `frontend.prepare` metric.

### The report does not state its accounting rule prominently

The pipeline is disjoint wall-clock accounting. Boundary, frontend, module and nested AST rows are
overlapping attribution views. Their section titles contain `accumulated`, but the report still
invites users to add all visible rows together.

---

## Locked decisions

### Command-total boundary

For `build`, `check` and the dev build/write total:

```text
start after timing-session configuration
-> execute all required command work
-> construct and classify the command outcome
-> capture one duration and record the command-total metric
-> render human diagnostics, success text, benchmark status and timing output
```

The command total includes diagnostic construction and outcome classification. It excludes all
presentation, including:

- success or failure text
- warning and diagnostic cards
- terse summaries
- `MOTH_BENCH status` output
- timing-schema and timing-record output
- human timing and counter summaries

`build` includes output planning and required filesystem writes. `check` ends after the frontend
outcome and renderable messages are complete. `command.dev.build_write` ends after the executor has
completed build and output writing. `command.dev.cycle` remains a distinct detailed span for state,
error-page and broadcast work.

### One stopwatch, one captured duration

Each command owns one ordinary `Instant` because user-facing command durations exist even without the
`timers` feature. The structured collector records that already-captured duration. It must not run a
second command-total guard.

Add one small facade operation that:

1. reads the command stopwatch once
2. records the supplied command-total metric when `timers` is enabled
3. returns the same `Duration` in every build

Do not retain the old command-total guard as a compatibility wrapper. The timing session remains the
collector and renderer owner, not a second clock owner.

### Work first, presentation second

Command orchestration returns or retains a typed presentation payload before any terminal call. The
execution-to-presentation barrier captures the duration once. Rendering must not be interleaved with
command work on success or failure paths.

### Timing schema v2

Changing the semantic end point of command totals requires `TIMING_SCHEMA_VERSION = 2`. Perform one
coordinated schema bump after every changed metric and boundary in this plan is final. Existing schema
v1 records remain readable but non-comparable. Do not numerically migrate or alias old totals.

### Constant-sensitive detailed metrics

The follow-up optimisation plan needs module-attributed evidence. Add typed detailed nested metrics
for the current owners of:

- constant-header semantic resolution
- const-template parsing
- const-template folding
- module-constant finalisation

Use the existing timing schema, module attribution and dense collector. These metrics are evidence,
never pipeline accounting. Remove the equivalent duplicate manual prose timers rather than retaining
two systems.

### Detailed prose is buffered

Legacy detailed probes that remain after the typed-metric migration record observations into the
active timing session and render after the command total has ended. They must not print at their call
sites. Preserve deterministic observation order independently of worker completion.

Remove per-file preparation prose entirely. `frontend.prepare`, source/file counters and slowest
module attribution already own that evidence. Do not replace the flood with another per-file format.

### Report wording

The concise report must say, once and near the pipeline section, that only pipeline rows account for
the command total and later sections are overlapping attribution views. Keep the current wall,
accumulated and nested schema classifications as the authority.

The build success line must say `output file` or `output files`. It must not imply that emitted files
are source files or modules.

---

## Non-goals

- no compiler-stage optimisation
- no attempt to make accumulated module work equal wall time
- no requirement that nested child metrics exhaust their parent
- no removal of the bounded `Other` row
- no new profiling framework
- no benchmark-history comparison across schema versions
- no change to source diagnostics, emitted artefacts or output ordering
- no timer or counter work when the corresponding feature is disabled

---

## Phase 0 - Baseline and ownership audit

- [ ] Record the current `build`, `check` and dev timing lifecycle for success, diagnosed failure and
      infrastructure failure paths.
- [ ] Add focused tests that demonstrate the current build/check human and structured duration
      boundaries differ. Keep these tests narrow enough to update into final invariants in Phase 2.
- [ ] Inventory every command-total guard and every caller of `finish_command_timing!`.
- [ ] Inventory manual detailed labels in the four constant-sensitive paths and locate every
      per-file `Files Prepared` emission.
- [ ] Confirm the benchmark parser, fingerprints, summaries and profile history all carry
      `TIMING_SCHEMA_VERSION` rather than a copied constant.
- [ ] Record unrelated validation failures before implementation begins.

Checkpoint: baseline tests and inventory only. Do not change schema meaning in this commit.

## Phase 1 - One command-duration authority

### Shared facade

- [ ] Add the single finish operation described above to the existing timing facade.
- [ ] In a timer build, record the captured duration through the normal dense collector without a
      second clock read.
- [ ] In a no-timer build, return the captured duration and erase collector arguments and calls.
- [ ] Rename session-finalisation helpers where needed so `finish command session` cannot be
      confused with `capture command duration`.
- [ ] Delete obsolete command-total guard setup and finish calls after every command migrates.

The helper must accept only a command-total metric. Reject a pipeline/evidence metric in tests or by a
narrow type/API so the operation cannot become a generic manual-duration escape hatch.

### Outcome boundary

- [ ] Introduce or refine typed command outcome structs only where needed to separate execution from
      rendering.
- [ ] Keep semantic result data and presentation data together enough to avoid rebuilding messages,
      warning lists or status counts after the timer ends.
- [ ] Do not add a general command framework or trait hierarchy. Three explicit command paths with
      one shared timing primitive are clearer.

Checkpoint: facade and focused unit tests. No schema bump until command callers are migrated.

## Phase 2 - Migrate build, check and dev

### Build

- [ ] Start the one command stopwatch immediately after timing-session configuration.
- [ ] Execute bootstrap, frontend, backend, output planning/write and outcome classification before
      rendering.
- [ ] Capture and record `command.build.total` once.
- [ ] Pass that exact duration to the success line.
- [ ] Render warnings, errors, timer output and benchmark status only after capture.
- [ ] Change the success count wording to `output file(s)`.
- [ ] Cover output-plan failure and output-write failure without moving rendering back inside the
      measured boundary.

### Check

- [ ] Make `run_check` own the single stopwatch.
- [ ] Remove the independent duration field/start point from `execute_check`.
- [ ] Complete message construction and diagnostic counts before capture.
- [ ] Record `command.check.total` from the exact duration used by normal and terse success text.
- [ ] Render diagnostics and summaries after capture on every path.

### Dev

- [ ] Capture `command.dev.build_write` around exactly the executor build/write operation.
- [ ] Carry the captured duration through `BuildOutcome` and `BuildCycleReport`.
- [ ] Use it in `Dev build #... done in ...`.
- [ ] Remove the redundant outer stopwatch used only for that status line.
- [ ] Keep `command.dev.cycle` as detailed evidence around state update, error-page construction and
      broadcast.
- [ ] Render warning cards and the timing summary after the cycle/session snapshot is complete.

### Required invariants

- [ ] Inject a fake or scripted clock at focused test boundaries. Do not use sleep-based assertions.
- [ ] Add an artificial renderer delay in tests and prove it changes neither the captured duration
      nor command total.
- [ ] Assert the human duration and structured command total are the same `Duration`, not merely
      close after formatting.
- [ ] Assert command totals still contain exactly one sample.

Checkpoint: all command paths migrated and obsolete command-total guards deleted.

## Phase 3 - Schema v2 and benchmark compatibility

- [ ] Set `TIMING_SCHEMA_VERSION` to 2.
- [ ] Update command-total descriptor comments to name the execution-to-presentation boundary.
- [ ] Add the four constant-sensitive detailed metrics with module attribution and correct AST
      parents.
- [ ] Update schema order, names, labels and tests from the one declarative registry.
- [ ] Update benchmark protocol/fingerprint expectations without duplicating schema identity.
- [ ] Prove schema v1 history is readable but reported as non-comparable.
- [ ] Prove mixed-schema aggregate reports are omitted or separated under existing policy.
- [ ] Refresh erasure inventories and schema-owned metric-name tests.

Do not claim a speed regression or improvement from the schema-reset run.

Checkpoint: one atomic schema-v2 compatibility commit.

## Phase 4 - Detailed timer cleanup

- [ ] Replace constant-resolution, const-template parse/fold and module-constant-finalisation manual
      timers with the new typed detailed metrics.
- [ ] Delete the nested duplicate `AST/environment/constants resolved in` timer.
- [ ] Keep one broader `nominal members and constants` aggregate only if it remains useful and has a
      distinct label and boundary.
- [ ] Remove every per-file `Files Prepared` detailed print.
- [ ] Change remaining `timer_log!` handling to record buffered session observations.
- [ ] Render buffered detailed observations after the command total and in deterministic logical
      order.
- [ ] Ensure summary and bench modes do not pay to format detailed prose.
- [ ] Ensure verbose mode does not contaminate measured parent spans with terminal writes.
- [ ] Audit detailed labels for duplicate wording that denotes different boundaries.

Prefer deleting redundant probes to preserving every historical line. Typed stage metrics and
benchmark counters are the durable evidence owners.

Checkpoint: detailed output is concise, non-duplicated and deferred.

## Phase 5 - Human report clarity

- [ ] Add one concise accounting note near the pipeline section:

```text
Only pipeline rows account for the command total. Remaining sections are overlapping attribution.
```

- [ ] Keep `Compilation boundaries` and `Frontend work` explicitly marked as accumulated.
- [ ] Keep AST children visually nested and never include them in top-level `Other` calculation.
- [ ] Retain `Other = command total - disjoint pipeline spans` and its current significance rule.
- [ ] Add renderer tests for build, check and dev wording.
- [ ] Update `benchmarks/README.md` only where the command-total and schema compatibility contract
      changed.

Checkpoint: report and documentation tests.

## Phase 6 - Final validation and closeout

Run, at minimum:

```bash
cargo test --lib timing
cargo test --lib projects::tests
cargo test --lib projects::dev_server
cargo test --lib benchmarking
just timers-erasure-check
just bench-ci
just validate
```

Also run one manual normal and one verbose command for each supported command kind. Verify:

- [ ] success duration equals the command-total observation
- [ ] renderer work does not enter the total
- [ ] pipeline plus `Other` equals the command total at full precision
- [ ] accumulated sections are not presented as additive
- [ ] constant-sensitive metrics have module attribution
- [ ] no duplicate constant timer line remains
- [ ] no per-file preparation flood remains
- [ ] no timer-only symbol or environment lookup survives a no-timer build
- [ ] schema-v1 data is not compared numerically with schema v2

Record the final commit and validation state in this plan. Do not update the progress matrix because
this work changes compiler instrumentation, not supported language behaviour.

---

## Simplification and deletion audit

The completed implementation should remove, not layer over:

- the separate structured command-total guards
- the second build/check duration start points
- the dev status-line outer stopwatch
- the old `finish_command_timing!` name if it conflates duration capture with session finalisation
- duplicate constant-resolution prose timers
- per-file preparation prose output
- direct synchronous printing from detailed timer call sites
- any copied timing-schema constant outside the schema owner

Reuse:

- the existing timing session and dense metric registry
- the existing wall/accumulated/nested descriptor model
- the existing benchmark schema and measurement fingerprints
- existing module attribution keys
- the existing `Other` accounting logic
- existing timer erasure checks

Do not add a second event collector, report model or benchmark parser.

## Completion criteria

- [ ] Every command has one execution stopwatch and one capture point.
- [ ] Human and structured durations that describe the same command are exactly identical.
- [ ] Presentation is excluded on success and failure paths.
- [ ] Dev build/write and full-cycle boundaries are explicit and separately named.
- [ ] Timing schema v2 is the sole current compatibility identity.
- [ ] Constant-sensitive detailed evidence is typed, nested and module-attributed.
- [ ] Detailed output is deferred, deterministic and non-duplicated.
- [ ] The concise report states its accounting rule.
- [ ] Old redundant timer paths are deleted without compatibility wrappers.
- [ ] No compiler semantics, diagnostic order or emitted artefacts changed.
- [ ] Full validation passes and the branch is ready for implementation review.
