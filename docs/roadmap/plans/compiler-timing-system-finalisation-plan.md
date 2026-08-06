# Compiler timing system finalisation plan

## Status

- **Plan state:** authoritative replacement plan, not started
- **Committed repository anchor:** `e739d4f1ba418cffbfce74a139ba86414c624465`
- **Intended repository path:** `docs/roadmap/plans/compiler-timing-system-finalisation-plan.md`
- **Roadmap status:** do not add this plan to `docs/roadmap/roadmap.md` unless the coordinator requests it separately
- **Primary invariant:** a compiler built without `timers` performs no timer-system runtime work
- **Timing compatibility stance:** this plan deliberately establishes timing schema v1. Timing data recorded before v1 is legacy and non-comparable
- **Authority:** this plan supersedes:
  - `docs/roadmap/plans/compiler-timing-summary-and-zero-cost-instrumentation-plan.md`
  - `docs/roadmap/plans/compiler-timing-final-review-corrections.md`

The earlier plans established the concise report, zero-cost feature boundary, project/package attribution, initial session ownership and several frontend/backend observations. This plan owns the final architecture, naming, measurement boundaries and implementation path.

The implementation is early alpha. Prefer correct long-term stage ownership, a narrow API and low self-interference over compatibility with provisional metric names or historical measurements.

---

## Active context capsule

Refresh this block after every accepted slice and before compaction.

```text
ACTIVE_PLAN:
- `docs/roadmap/plans/compiler-timing-system-finalisation-plan.md`

CURRENT_SLICE:
- Phase: 0
- Checklist item: reconcile the large uncommitted Phase 4 migration with this replacement design
- Goal: preserve useful work, remove transitional drift and restore one buildable implementation path
- Non-goals: final metric migration, benchmark recording, roadmap insertion

PROGRESS_RECORD:
- WORK_ID: `compiler-timing-finalisation`
- WORK_SOURCE: this plan
- BASE_REVISION: `e739d4f1ba418cffbfce74a139ba86414c624465`
- HEAD_REVISION: `34cf62911` (current worktree dirty)
- STATUS: in progress
- CURRENT_SCOPE: Phase 1 timing schema v1 completed and audited; commit checkpoint, then Phase 2
- PIPELINE: launcher installed at `~/.local/bin`, `validate-config` + `doctor` clean, routes auditor/explorer/final_auditor/involved_worker/simple_worker
- AUTHORITIES: read `AGENTS.md`, `compiler-design-overview.md`, `build-system-design.md`, `style-guide.mtf`, `testing.mtf`, `validation.mtf`, `benchmarks/README.md`
- UNCOMMITTED (Phase 1 slice, pending checkpoint commit); full diff saved to `/tmp/compiler-timing-phase4-in-progress.patch`
  - untracked: `src/timing/enabled/schema.rs`, `src/timing/tests/schema_tests.rs`; `type-stress.html` (unrelated user file, preserve untouched)
  - modified: `src/timing/enabled.rs` (`mod schema;`), `src/timing/tests/mod.rs` (`mod schema_tests;`), this plan (capsule, checklist, closeout table)
  - committed: Phase 0 reconciliation checkpoint `a30effed2`
  - schema inventory verified against `/tmp/inventory-raw.txt` (74 provisional names, extraction diff clean)
  - 46 v1 metrics = 35 Basic + 11 Detailed; stable names match plan §3 exactly
  - schema module absent in a no-`timers` build (erasure check clean)
- NEXT_ACTION: commit the accepted Phase 1 checkpoint, then begin Phase 2 (rebuild session, runtime and aggregate collection)
- AUDITS:
  - 2026-08-07 Phase 1 final_auditor route completed (run 20260807T095857Z-b7159c64): verdict findings -> all resolved; scope: schema.rs/schema_tests.rs, wiring, erasure boundary, session reuse, summary disjointness, plan closeout; found registry correct, plan-coincident, drift-proof, no doc drift except one stale test count now fixed
  - 2026-08-07 Phase 1 interim auditor route completed (run 20260807T095359Z-da99d88e): verdict `findings` (2 low optional, both resolved): reworded the stale `TimingMetricDescriptor.parent` comment; added `registry_size_matches_plan_closeout` test pinning 46/35/11
  - 2026-08-07 Phase 0 read-only auditor route completed (run 20260807T000334Z-04ac5112): verdict "audit pass with required fixes"; findings addressed:
  - removed dead `record_labeled_pipeline_timing` (stale `labeled_pipeline_timer!` backing helper, no callers) from `src/timing/enabled.rs`
  - added optional trailing comma `$(,)?` to the `command_timing_scope!` enabled arm for parity with the disabled arm (src/timing.rs)
  - post-fix: five-feature check matrix, `cargo clippy --features timers`, timing lib tests (57 pass), `just timers-erasure-check` (7935056 bytes), `cargo fmt --all --check`, `git diff --check` all clean
- BLOCKERS: none
- SESSION_NOTES 2026-08-07:
  - read the plan's authoritative design and full Phase 0 checklist after compaction
  - root-fixed the parallel timing flake: unified the two independent test locks
    (`lock_timing_tests` in tests/mod.rs and `lock_counter_test` in
    instrumentation) into one facade-owned `lock_instrumentation_tests` in
    `src/timing.rs`; frontend and timing suites now share one fence
  - `cargo test --features timers --lib timing` passes 5/5 parallel runs (57 tests)
  - full lib suite passes under default, `timers`, `detailed_timers`,
    `benchmark_counters` (4146/4147/4100/4098 passed); `benchmark_counters,timers`
    has one pre-existing failure `chunked_file_preparation_skips_identity_payload_remap`
    that ALSO fails at the clean committed HEAD 34cf62911 (not introduced here)
  - five-feature check matrix passes; `cargo fmt --all --check` passes;
    `git diff --check` passes; `just timers-erasure-check` passes
  - marked `compiler-timing-final-review-corrections.md` superseded and linked to
    this plan (the earlier summary-plan was already removed by the anchor commit)
  - macro surface audited: all seven old names (`pipeline_timer!`,
    `labeled_pipeline_timer!`, `timing_guard!`, `timed_manual_finish!`,
    `timed_manual_finish_attributed!`, `command_timing_start!`) replaced by the
    plan's final surface; no mixed old/new surface remains
  - full diff (23 mods + 3 untracked) re-saved to /tmp/compiler-timing-phase4-in-progress.patch

LAST_GOOD_COMMIT:
- `e739d4f1ba418cffbfce74a139ba86414c624465`

CURRENT_WORKTREE_STATE:
- GitHub can verify only the committed anchor
- User-reported uncommitted work:
  - new facade macros added:
    - `timed_stage!`
    - `timed_stage_attributed!`
    - `timing_scope!`
    - `timing_scope_attributed!`
    - `timing_scope_multi!`
    - `record_timing_duration!`
    - `record_attributed_duration!`
    - `command_timing_scope!`
  - old facade macros deleted
  - production call sites migrated broadly to guards
  - counter summary and command orchestration split into dedicated modules
  - guard `finish()` calls restored in `build.rs`, `check.rs`, `compilation.rs` and `source_discovery.rs`
  - remaining reported call-site work:
    - `project_config/parsing.rs`
    - `project_config.rs`
    - `output/orchestrator.rs`
    - `source_tree_index.rs`
  - validation is incomplete
  - `just timers-erasure-check` was running when work paused
- Confirm branch, exact diff and unrelated changes before editing
- Save the uncommitted diff under `/tmp` before restructuring it

RELEVANT_DOCS_THIS_SLICE:
- `AGENTS.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `benchmarks/README.md`
- the two superseded timer plans for implementation history only

RELEVANT_CODE:
- `src/timing.rs`: compile-erasing facade
- `src/timing/enabled.rs`: current enabled implementation entry point
- `src/timing/enabled/session.rs`: owned session lifecycle
- `src/timing/enabled/collector.rs`: process-global collection
- `src/timing/enabled/mode.rs`: cached output mode
- `src/timing/enabled/attribution.rs`: boundary and module identities
- `src/timing/enabled/summary.rs`: basic report policy and aggregation
- `src/timing/enabled/render.rs`: saying-based renderer
- `src/compiler_frontend/compiler_messages/compiler_dev_logging.rs`: remaining detailed timer ownership
- `src/compiler_frontend/ast/mod.rs`: AST environment, emission and finalisation timing
- `src/compiler_frontend/ast/module_ast/build_context.rs`: timer-only AST context
- `src/compiler_frontend/ast/module_ast/environment/builder.rs`: AST environment boundary
- `src/compiler_frontend/ast/generic_functions/materialisation.rs`: generated AST path
- `src/build_system/create_project_modules/compilation.rs`: Stage 0 and boundary timing
- `src/build_system/create_project_modules/frontend_orchestration.rs`: frontend stage timing
- `src/build_system/project_config.rs`
- `src/build_system/project_config/parsing.rs`
- `src/build_system/output/orchestrator.rs`
- `src/build_system/create_project_modules/source_discovery.rs`
- `src/build_system/create_project_modules/source_tree_index.rs`
- `src/projects/cli.rs`
- `src/projects/check.rs`
- `src/projects/dev_server/build_loop.rs`
- `src/projects/dev_server/server.rs`
- `src/projects/html_project/js_path.rs`
- `src/projects/html_project/wasm/artifacts.rs`
- `src/benchmarking/frontend.rs`
- `xtask/src/timers_erasure_check.rs`
- benchmark parser, fingerprint and report owners under `xtask/src/`

ACCEPTANCE_CRITERIA:
- no-timer builds contain no timer clock reads, types, state, context, labels, environment queries, collector calls or timer-only strings
- timing schema v1 has one typed metric registry and documented semantic boundaries
- every recorded metric is named and owned by the stage whose work it measures
- config AST, module AST and generated AST work use distinct metric identities
- command, boundary, module and nested stage timing cannot be confused by string-prefix inference
- session start, finish, drop and nested-start behaviour is explicit and tested
- disabled and inactive timer modes avoid unnecessary clock reads and locks
- record paths perform no formatting or allocation
- raw benchmark output is deterministic and tagged with timing schema v1
- the basic report has one policy owner for display and command accounting
- detailed timers retain deeper evidence without changing basic metrics
- old benchmark data is never compared as if it used timing schema v1
- the worktree contains one current implementation path with no compatibility wrappers

DECISIONS_ALREADY_MADE:
- decision: zero runtime cost without `timers` is non-negotiable
  - reason: developer instrumentation must not affect ordinary compiler builds
  - source/user/date: Nye, 2026-08-05 and reaffirmed 2026-08-06
- decision: historical timing compatibility is no longer a design constraint
  - reason: early-alpha provisional data is less valuable than correct long-term stage ownership
  - source/user/date: Nye, 2026-08-06
- decision: this plan establishes the first stable timing schema
  - reason: future comparisons need an explicit semantic baseline
  - source/user/date: Nye, 2026-08-06
- decision: basic timers, detailed timers and benchmark tooling remain distinct products
  - reason: the human summary must stay quick to scan and must not become a second benchmark system
  - source/user/date: accepted timer design, 2026-08-05
- decision: attribution remains explicit and thread-scheduling independent
  - reason: future Rayon work must not lose or invent package/module ownership
  - source/user/date: accepted timer design, 2026-08-05
- decision: no thread-local attribution or nested tracing framework
  - reason: timer ownership should remain explicit and narrow
  - source/user/date: accepted correction direction, 2026-08-06
- decision: the plan stays outside the roadmap for now
  - reason: the coordinator chooses the execution pause point
  - source/user/date: Nye, 2026-08-05

BLOCKERS / RISKS:
- the uncommitted Phase 4 diff is large and cannot be reviewed through GitHub
- blanket guard migration may have widened scopes accidentally
- the committed collector still has incomplete nested raw-session ownership
- the committed mode cache and collector still use mutexes on active paths
- counter-summary collection and timer output modes are coupled incorrectly
- old raw metric names and `_ms` suffixes are provisional
- existing benchmark history must be invalidated deliberately
- instrumentation overlaps active build/frontend files, so resume only from the current worktree after re-reading project authorities

VALIDATION_STATE:
- committed Phase 3 checkpoint reports:
  - five-feature Cargo check matrix
  - thousands of no-feature, timer and detailed-timer tests
  - Clippy with warnings denied
  - `just timers-erasure-check`
  - `just validate`
  - docs, failed-AST, single-file and dev smokes
- GitHub exposes no Actions status rows for the checkpoint
- uncommitted Phase 4 validation is incomplete

DOCS_IMPACT:
- add this plan
- mark both older timer plans superseded by this plan
- update `benchmarks/README.md`
- update timer module docs and Cargo feature comments
- update validation documentation if the erasure gate changes
- update `index.md` if timing modules move from `enabled.rs` to `enabled/mod.rs`
- no progress-matrix change
- no roadmap insertion

NEXT_ACTION:
- execute Phase 0 exactly, then stop for checkpoint review
```

---

# Authoritative design

## 1. Product boundaries

### Basic timers

Purpose:

- immediate rough feedback on real Moth projects
- one command or dev rebuild
- major wall spans
- project and source-package boundaries
- coarse frontend/backend ownership
- one slowest module
- no historical judgement

Basic output must remain short enough to scan without searching.

### Detailed timers

Purpose:

- deeper compiler investigation
- config, Stage 0, frontend, AST, TIR, HIR, borrow, backend and output substages
- inline prose where useful
- raw aggregate evidence at command completion

Detailed timers must not redefine basic metrics. They add children and investigation-only metrics.

### Benchmark system

Purpose:

- repeatable non-recording checks
- history and comparison
- workload fingerprints
- timing-schema compatibility
- reports and profiles

The timer system produces measurements. The benchmark system decides whether two runs are comparable.

### Benchmark counters

Purpose:

- high-volume work-shape evidence
- explicit feature-selected instrumentation
- independent output controls

`benchmark_counters` does not weaken the timer zero-cost rule. A plain compiler build with neither developer feature pays for neither system.

---

## 2. Non-negotiable invariants

### Zero-cost feature erasure

Without `timers`:

- no timer implementation module exists
- no timer enum, descriptor, session, guard, collector or renderer exists
- no timer-only field changes a production struct layout
- no timer-only parameter changes a production function ABI
- no timer metric or heading string is linked into the binary
- no `Instant::now()` is executed for timer instrumentation
- no timer macro argument is evaluated
- no no-op timer function is called
- no environment variable is read by the timer system

Disabled expression macros expand to the production expression. Disabled statement macros emit no statement.

### Semantic timing boundaries

A metric boundary is chosen because it matches one owner, not because an old timer happened to start or stop there.

Each recorded metric must answer:

- what work starts the span
- what exact result or owner ends it
- whether errors record partial elapsed work
- whether it is wall time, accumulated work or nested evidence
- whether it can overlap another displayed row
- what attribution context it accepts

### One current path

Moth is pre-release.

- no compatibility wrappers for old timer APIs
- no duplicate old/new metric registries
- no forwarding macros retained for provisional names
- no parser fallback for two timing schemas unless required to read one tracked repository file
- no hidden boundary preservation solely for old local data

### Low self-interference

With `timers` compiled:

- inactive timer modes avoid clock reads
- recording performs no string formatting
- recording performs no heap allocation
- metric lookup uses a dense typed ID
- mode lookup is lock-free after initialisation
- no collector mutex is taken when no collection channel is active
- stable benchmark lines are emitted after measurement, not inside measured stage bodies
- attribution work is only performed when the active session requests it

---

## 3. Timing schema v1

### Version

Add one explicit constant:

```rust
pub(crate) const TIMING_SCHEMA_VERSION: u32 = 1;
```

Timing schema v1 begins at the final checkpoint of this plan.

A future change increments the schema when it changes:

- a stable metric name
- a metric's semantic start or end
- wall/accumulated/nested classification
- parent or accounting ownership
- attribution meaning
- aggregate output semantics

Adding a new independent metric may remain within the same schema when existing metrics keep their meaning. Document that decision in the checkpoint.

### Legacy data

- all timing data before schema v1 is legacy
- no numeric migration
- no attempt to compare old and v1 stage values
- old local raw history may be deleted
- tracked reports may be reset or regenerated where required
- benchmark reports must label a schema mismatch as non-comparable

### Typed metric registry

Replace raw string metric arguments with a dense timer-only enum:

```rust
#[cfg(feature = "timers")]
#[repr(u16)]
pub(crate) enum TimingMetric {
    CommandBuildTotal,
    CommandCheckTotal,
    CommandDevBuildWrite,
    // ...
}
```

Each metric has one descriptor:

```rust
pub(crate) struct TimingMetricDescriptor {
    pub(crate) stable_name: &'static str,
    pub(crate) level: TimingLevel,
    pub(crate) relation: TimingRelation,
    pub(crate) attribution: TimingAttributionKind,
}
```

Suggested supporting enums:

```rust
pub(crate) enum TimingLevel {
    Basic,
    Detailed,
}

pub(crate) enum TimingRelation {
    WallSpan,
    Accumulated,
    NestedEvidence,
}

pub(crate) enum TimingAttributionKind {
    None,
    Boundary,
    Module,
}
```

Use one readable local declarative metric list only if it prevents enum/name/descriptor drift. Do not add a procedural macro.

The registry must provide:

- `TimingMetric::ALL`
- dense index conversion
- stable name lookup
- descriptor lookup
- uniqueness validation
- deterministic schema order

### Naming rules

- lowercase dotted names
- name the semantic owner
- omit unit suffixes such as `_ms`
- use `.total` only for a complete owner span
- use sibling names for disjoint work
- use hierarchy for true parent/child ownership
- do not encode presentation labels into stable names

### Required core schema

The exact descriptor table is completed in Phase 1. It must include at least these semantic owners.

#### Commands

| Stable name | Meaning |
|---|---|
| `command.build.total` | complete build command work through required output write, excluding timer rendering |
| `command.check.total` | complete check command work through diagnostic rendering, preserving the chosen command contract |
| `command.dev.build_write` | one dev executor build and output write |
| `command.dev.cycle` | detailed-only full dev cycle including state and broadcast work |

#### Build orchestration

| Stable name | Meaning |
|---|---|
| `build.bootstrap.total` | complete build bootstrap |
| `build.frontend.total` | complete Stage 0 plus frontend project compilation |
| `build.backend.total` | complete selected backend build |
| `build.output.total` | complete output orchestration |

#### Stage 0

| Stable name | Meaning |
|---|---|
| `stage0.directory.inventory` | directory graph, source and module inventory work |
| `stage0.directory.compile` | package and project module compilation batch |
| `stage0.single_file.total` | complete single-file Stage 0/frontend orchestration |
| `boundary.inventory` | accumulated inventory work attributed to one source package or main project |
| `boundary.compile` | accumulated compile work attributed to one source package or main project |

#### Frontend module work

| Stable name | Meaning |
|---|---|
| `frontend.prepare` | source preparation owned by one module |
| `frontend.bind_headers` | provider-dependent header binding |
| `frontend.order_declarations` | dependency ordering and sorted declaration preparation |
| `frontend.ast.total` | complete module AST construction |
| `frontend.ast.environment` | complete environment construction including final environment assembly |
| `frontend.ast.emit` | `AstEmitter` production of emitted AST state |
| `frontend.ast.finalise` | `AstFinalizer` production of the final AST result |
| `frontend.public_interface.project` | pre-HIR public-interface projection |
| `frontend.hir` | module AST to HIR lowering |
| `frontend.borrow.initial` | initial direct borrow analysis |
| `frontend.generated.materialise` | generated-function materialisation |
| `frontend.borrow.converge` | repeated direct call-summary borrow convergence |
| `frontend.generated.borrow_recheck` | generated sidecar borrow rechecks |
| `frontend.public_interface.finalise` | post-borrow public-interface closure |
| `frontend.module.semantic_total` | complete provider-dependent semantic module compilation |

Config AST and generated AST must not reuse the module AST metrics:

| Stable name | Meaning |
|---|---|
| `config.ast.total` | config AST work |
| `frontend.generated.ast.total` | generated materialisation AST work |

Detailed child metrics may use the same hierarchy, for example:

```text
config.ast.environment
frontend.generated.ast.environment
frontend.generated.ast.emit
frontend.generated.ast.finalise
```

#### Backend and output

| Stable name | Meaning |
|---|---|
| `backend.html.total` | complete HTML backend work |
| `backend.js.lower_entry` | entry-module HIR to JS lowering |
| `backend.js.lower_linked` | linked-module HIR to JS lowering |
| `backend.html.render` | HTML document rendering |
| `backend.wasm.total` | complete HTML-Wasm route build |
| `backend.wasm.lower` | Wasm lowering only |
| `backend.wasm.artifacts` | Wasm artifact and bootstrap assembly |
| `backend.assets.plan` | tracked/runtime asset planning |
| `backend.assets.emit` | tracked/runtime asset emission |
| `output.write.total` | complete output write orchestration |

Config, bootstrap and output microstages remain detailed-only unless evidence shows they belong in the basic report.

### Parent and aggregate rule

Record one duration once.

Do not record the same duration under a parent and child solely to create a human aggregate.

Human rows may sum disjoint raw metrics:

```text
Public interface = project + finalise
Generated functions = materialise + generated borrow recheck
JS lowering = lower_entry + lower_linked
Tracked assets = plan + emit
```

Record a parent total separately only when it measures a wider real span that includes gaps or work not represented by its children.

---

## 4. Enabled module shape

Target layout:

```text
src/timing.rs
src/timing/
  enabled/
    mod.rs
    schema.rs
    runtime.rs
    session.rs
    collector.rs
    attribution.rs
    guard.rs
    command.rs
    counter_summary.rs
    summary.rs
    render.rs
  tests/
    erasure_tests.rs
    facade_tests.rs
    collector_tests.rs
    schema_tests.rs
    summary_tests.rs
```

Responsibilities:

- `src/timing.rs`
  - cfg-selected facade
  - compile-erasing macros
  - explicit re-exports only
- `schema.rs`
  - timing schema version
  - metric enum and descriptors
- `runtime.rs`
  - cached process configuration
  - active channel bits
  - lock-free fast predicates
- `session.rs`
  - one active process session
  - command/raw ownership
  - start, finish and drop lifecycle
- `collector.rs`
  - dense aggregate storage
  - snapshot construction
  - no terminal output
- `attribution.rs`
  - session-scoped boundary/module IDs
  - metadata registration
  - attributed aggregate storage
- `guard.rs`
  - start tokens and finishable guards
- `command.rs`
  - command mode to session configuration
  - final benchmark flush and summary render handoff
- `counter_summary.rs`
  - counter grouping only
- `summary.rs`
  - pure report construction and accounting
- `render.rs`
  - saying-based terminal output only

Delete `src/timing/enabled.rs` after moving its map/orchestration role into `src/timing/enabled/mod.rs`.

Remove broad `allow(dead_code)` attributes. Fix or delete every surfaced dead API.

---

## 5. Runtime and session design

### Process configuration

Parse `MOTH_TIMERS` and `MOTH_COUNTERS` once through a `OnceLock` or equivalent lock-free read path.

Production recording must not lock to read mode.

Tests should inject explicit session configuration or use a scoped override guard that restores state. Do not mutate a global mode permanently.

### Session configuration

Use explicit collection channels:

```rust
pub(crate) struct TimingChannels {
    pub(crate) metrics: bool,
    pub(crate) counters: bool,
    pub(crate) attribution: bool,
    pub(crate) detailed: bool,
    pub(crate) bench_output: bool,
    pub(crate) human_summary: bool,
    pub(crate) human_prose: bool,
}
```

The exact representation may be an internal bitset without adding a dependency.

Required mode matrix:

| Timer mode | Counter mode | Channels |
|---|---|---|
| Summary | Off | metrics + attribution + human summary |
| Verbose | Off | metrics + attribution + detailed + bench output + human prose + human summary |
| Bench | Off | metrics + bench output, no attribution |
| Silent | Off | no timer channels |
| Summary | Summary | metrics + counters + attribution + human summary + counter summary |
| Bench | Summary | metrics + counters + bench output + counter summary |
| Silent | Summary | counters + counter summary |
| Any | Full | counter collection plus full counter output policy |
| Explicit frontend benchmark | caller-owned | metrics, optional counters, no attribution, stdout suppressed |

Bench mode still collects dense metric aggregates because those aggregates are the output. It must not allocate boundary/module metadata.

### Session ownership

- one process-active timing session
- no destructive replacement
- session IDs are monotonically increasing
- finish and drop affect only the matching generation
- stale boundary/module contexts are rejected
- explicit raw benchmark start returns an error when the collector is busy
- a raw benchmark must fail before compilation rather than write into an outer session
- nested command sessions remain unsupported and must be treated as an internal instrumentation error in tests
- no thread-local ownership

### Active fast path

Expose a lock-free active-channel check.

When no relevant channel is active:

- no timer `Instant` is created
- no collector lock is taken
- no formatting occurs
- the production expression still runs once

---

## 6. Aggregate-first collector

The collector stores aggregates, not a raw event log.

### Global metric aggregates

Use dense storage indexed by `TimingMetric`.

A suitable accumulator:

```rust
pub(crate) struct MetricAccumulator {
    total_nanos: AtomicU64,
    samples: AtomicU32,
}
```

Requirements:

- reset at session start while no other session is active
- relaxed atomic additions are sufficient for totals
- clamp `Duration::as_nanos()` to `u64`
- no string hashing
- no BTreeMap on the record path
- no allocation per observation

The final snapshot converts atomics back to `Duration`.

### Attribution aggregates

Boundary and module registries remain dynamic and session-scoped.

Use:

- a lifecycle/registration lock for adding metadata
- read access plus atomic metric slots for attributed recording
- returned opaque IDs only
- no direct construction of module keys outside timing internals and focused tests
- exact duplicate registration may be idempotent
- conflicting duplicate metadata is an error

Each registered boundary/module may own dense attributed metric accumulators. Only metrics whose descriptors allow that attribution may be recorded there.

### Snapshot

A finished snapshot contains:

```rust
pub(crate) struct TimingSnapshot {
    pub(crate) schema_version: u32,
    pub(crate) command: Option<TimingCommandKind>,
    pub(crate) metrics: Box<[TimingMetricAggregate]>,
    pub(crate) boundaries: Vec<TimingBoundarySnapshot>,
    pub(crate) modules: Vec<TimingModuleSnapshot>,
    pub(crate) counters: Vec<CounterAggregate>,
}
```

The snapshot is deterministic in metric-schema order and registration order.

No snapshot retains absolute filesystem paths.

---

## 7. Compile-erasing facade

### Final macro surface

Keep the surface small:

```text
timed_stage!(metric, expression)
timed_stage_attributed!(metric, context, expression)
timing_scope!(binding, metric)
timing_scope_attributed!(binding, metric, context)
timing_scope_multi!(binding, metrics)
finish_timing_scope!(binding)
record_timing_duration!(metric, duration)
record_attributed_duration!(metric, duration, context)
command_timing_scope!(binding, command_kind)
finish_command_timing!(binding, succeeded)
```

Names may be adjusted once during Phase 3 for consistency. There must be one final shape.

### Expression macros

Enabled expansion:

1. ask runtime whether this metric is active
2. capture `Instant` only when active
3. execute the production expression once
4. record the captured duration once
5. return the original value unchanged

Disabled expansion is the production expression.

Use expression timing for:

- one call
- one result-producing block
- work with a clean lexical endpoint

### Scope guards

Use named guards only for:

- multiple early returns
- several `?` paths
- a real lexical span that cannot be wrapped clearly

Guard contract:

- `finish()` records once
- Drop after `finish()` records nothing
- Drop without `finish()` records elapsed work to the error exit
- the successful endpoint is chosen semantically
- no-timer builds have no guard binding
- `finish_timing_scope!` erases completely without `timers`

Do not use guards as the default migration for every old manual timer.

### Multi-metric spans

Use a multi-record duration only when distinct raw metrics intentionally share one real span.

Prefer one recorded leaf plus a derived human aggregate when possible.

### Detailed prose

Timer prose belongs in `timing`, not `compiler_dev_logging`.

`compiler_dev_logging` keeps:

- token dumps
- header dumps
- AST dumps
- HIR dumps
- borrow dumps
- unrelated developer logging

Detailed prose must print the same captured duration stored in the collector.

---

## 8. Semantic boundary rules

### Commands

- command timing starts after command dispatch selects the command
- command timing ends after required command work and user diagnostics/status output according to the documented command contract
- timer rendering and stable benchmark flushing occur after the command duration is captured
- command functions should converge on one finish point
- refactor early returns into an outcome helper when that makes lifecycle ownership clearer

### Stage 0

- inventory measures source discovery, graph/inventory construction and retained preparation owned by that pass
- compile measures package and project module compilation
- boundary inventory and compile are accumulated attributed work
- single-file timing uses its own coherent owner rather than imitating directory internals

### AST

Final v1 definitions:

- `frontend.ast.environment`
  - starts when `AstModuleEnvironmentBuilder::build` begins semantic environment work
  - ends when the complete `AstModuleEnvironment` has been assembled and returned
- `frontend.ast.emit`
  - measures the `AstEmitter` production call
  - excludes later unrelated result inspection and counters
- `frontend.ast.finalise`
  - measures the `AstFinalizer` production call
- `frontend.ast.total`
  - wraps the complete module AST construction owner

Config and generated materialisation use separate metric IDs. Basic module AST children therefore need no contextual filtering trick.

### Public interface

- project and finalisation are separate recorded leaves
- the basic `Public interface` row sums them
- do not record a duplicate aggregate span with the same values

### Borrow and generated work

Keep distinct:

- initial direct borrow analysis
- direct summary convergence
- generated materialisation
- generated borrow rechecks

The human report may group them, but raw schema keeps their owners separate.

### Backend

- `backend.wasm.total` means complete Wasm route build and is displayed as `Wasm build`
- Wasm lowering alone uses `backend.wasm.lower`
- tracked assets group plan and emission or label emission precisely
- JS entry and linked lowering remain distinct raw metrics
- backend parent totals may overlap nested evidence and are never added together in command accounting

### Failure paths

- expression timers record a returned `Err`
- scope timers record to the early exit through Drop
- failed command reports use partial evidence
- no timing failure changes compiler success or diagnostic semantics
- instrumentation invariant failures remain internal and testable

---

## 9. Basic summary model

### Report order

```text
<Command> timings <total>

<Command> pipeline
Compilation boundaries
Frontend work
Backend
Slowest module
```

Empty sections are omitted.

### Pipeline accounting

One policy descriptor owns:

- row label
- metric sources
- command applicability
- relation
- parent
- significance behavior
- command-accounting role

Delete separate hardcoded accounting lists.

`Other`:

- uses non-overlapping command-child rows only
- appears at 1ms or 2% of command total
- is omitted when zero
- never uses saturating subtraction to hide over-accounting
- an accounted total greater than the command total is a tested internal policy error

### Frontend

Show:

- Prepare source files
- Bind headers
- Order declarations
- Semantic frontend / AST
  - Environment, types and constants
  - Bodies and TIR construction
  - Template and constant finalisation
- Public interface
  - Projection
  - Finalisation
- HIR
- Borrow validation
- Generated functions

Child rows use the 1ms and 5% threshold.

### Boundaries

Show source-backed packages and the main project in deterministic order.

Boundary total:

```text
boundary.inventory + boundary.compile
```

Label it accumulated work.

### Slowest module

Use:

```text
frontend.prepare + frontend.module.semantic_total
```

Display:

- logical module identity
- accumulated duration
- source file count
- source size in KiB

Never display an absolute path.

### Rendering

- `saying::say!` only
- headings in blue
- ordinary values in green
- command and boundary totals in yellow
- suffixes in dark white
- recursive label width includes child rows
- singular/plural is correct
- boundary count columns align
- long logical identities are bounded while preserving their unique tail

---

## 10. Benchmark integration

### Output contract

At command finish in Bench or Verbose mode, emit:

```text
MOTH_BENCH timing-schema 1
MOTH_BENCH timing <metric>=<milliseconds>ms
```

Rules:

- exactly one schema line
- at most one line per recorded metric
- metrics ordered by `TimingMetric::ALL`
- values are final aggregate totals
- no output inside measured stage bodies
- command total is required for CLI benchmark cases
- counters remain separate records

### In-process frontend benchmark

Return:

- timing schema version
- total wall duration
- aggregate stage list in schema order
- optional counters

Starting the raw session while another session is active returns a typed tooling error before compilation begins.

### Benchmark protocol reset

- add timing schema to the measurement identity
- bump the benchmark protocol or measurement format as required
- treat all pre-v1 timing data as incompatible
- do not migrate numeric stage values
- remove local old timing history where practical
- regenerate tracked current summaries only when required by repository format
- reports say `timing schema changed`, not a speed delta

### Future rule

After this plan closes:

- changing a metric's meaning increments timing schema
- changing only its human label does not
- adding a new independent metric is documented and reviewed for schema impact
- benchmark comparison requires equal timing schema

---

# Implementation phases

Each phase is a stable checkpoint sized for one coding-agent context. Every phase ends with a focused audit, style review and validation gate.

---

# Phase 0 - Reconcile the uncommitted migration and install this authority

## Context

The current worktree contains a large uncommitted Phase 4 migration that GitHub cannot inspect. Preserve it before deciding what remains useful. Do not continue the mechanical guard-finish pass under the superseded plan.

## Checklist

### Preserve and inspect

- [x] Re-read the required project authorities after compaction.
- [ ] Run:
  - [x] `git status --short`
  - [x] `git branch --show-current`
  - [x] `git log -1 --oneline`
  - [x] `git diff --stat`
  - [x] `git diff --check`
- [ ] Save:
  - [x] `git diff > /tmp/compiler-timing-phase4-in-progress.patch`
  - [x] a changed-file list under `/tmp`
- [x] Record every unrelated change in the active capsule.
- [x] Do not stash, reset or discard the diff without preserving the patch.

### Classify uncommitted work

For each changed file, classify the work as:

- [x] retain unchanged
- [x] retain but adapt to typed metrics
- [x] replace because guard scope is wrong
- [x] revert to the committed anchor and reimplement later
- [x] unrelated and preserve untouched

Expected likely retention:

- [x] dedicated `command.rs`
- [x] dedicated `counter_summary.rs`
- [x] named guard types with exact-once finish semantics
- [x] direct-expression disabled macro arms
- [x] explicit module split work

Expected rework:

- [x] string metric arguments
- [x] blanket guard call-site migration
- [x] explicit `.finish()` placements made only to mimic old boundaries
- [x] old compatibility comments
- [x] old macro names retained through wrappers

### Establish one buildable path

- [x] Add this plan to `docs/roadmap/plans/`.
- [x] Mark both older timer plans `superseded` and link to this plan.
- [x] Update the capsule with the exact worktree.
- [x] Make the current tree compile without introducing duplicate timer APIs.
- [ ] If the broad migration cannot be made coherent in this slice:
  - [ ] restore affected call sites to `e739d4f1...`
  - [ ] retain the saved patch as reference
  - [ ] keep only the clean module/facade pieces
- [x] Do not commit a mixed old/new macro surface.

## Audit

- [x] Confirm one owner for every retained macro and enabled module.
- [x] Confirm no timer data enters semantic artefacts.
- [x] Confirm no uncommitted call-site span is accepted merely because it matches an old boundary.
- [x] Confirm the replacement plan is the only active authority.

## Style review

- [x] No transitional wrappers.
- [x] No broad lint allowances added.
- [x] Module files have concise WHAT/WHY documentation.
- [x] The plan capsule is accurate enough to resume after compaction.

## Validation

- [x] `cargo fmt --all --check`
- [x] minimum five-feature `cargo check` matrix
- [x] focused no-feature erasure tests
- [x] `git diff --check`

## Checkpoint

- [ ] Commit the reconciliation checkpoint.
- [ ] Stop for review before Phase 1.

---

# Phase 1 - Define timing schema v1 and the typed metric registry

## Context

Metric identity must be settled before collector and call-site work. This phase intentionally breaks provisional names and invalidates old timing comparisons.

## Checklist

### Schema module

- [x] Add `schema.rs`.
- [x] Add `TIMING_SCHEMA_VERSION = 1`.
- [x] Add dense `TimingMetric`.
- [x] Add descriptor enums and table.
- [x] Add `TimingMetric::ALL`.
- [x] Add stable-name and index lookup.
- [x] Add command applicability where needed.
- [x] Add basic/detailed level.
- [x] Add relation and attribution policy.

### Final metric inventory

- [x] Replace every provisional string name in the baseline inventory with a v1 metric.
- [x] Use the required core schema in this plan.
- [x] Review every current config, Stage 0, frontend, backend and output metric.
- [x] Remove unit suffixes.
- [x] Split config AST, module AST and generated AST identities.
- [x] Remove duplicate parent/child metrics that record the same duration.
- [x] Document the exact start/end meaning of every basic metric.
- [x] Keep detailed microstages only when they answer a real investigation question.
- [x] Delete meaningless zero-cost microtimers.

### Schema tests

- [x] enum and descriptor counts agree
- [x] stable names are unique
- [x] names follow lowercase dotted syntax
- [x] no `_ms` suffix remains
- [x] every basic metric has a human owner
- [x] every attributed metric permits the supplied context kind
- [x] every command total is unique
- [x] every nested row has a valid parent policy
- [x] schema order is deterministic

### Plan refresh

- [x] Add the final v1 metric table to this plan's closeout record or an appendix inside this plan.
- [x] Record the deliberate compatibility reset.

## Audit

- [x] Compiler-stage owner reviews metric boundaries.
- [x] Build-system owner reviews Stage 0, command and output boundaries.
- [x] Backend owner reviews HTML, JS, Wasm and assets.
- [x] Remove names that expose implementation accidents instead of semantic work.

## Style review

- [x] Registry is data-oriented.
- [x] No broad string-prefix inference.
- [x] No procedural macro.
- [x] One readable local metric list at most.
- [x] No legacy name aliases.

## Validation

- [x] schema unit tests (12 tests, `cargo test --features timers --lib timing` -> 69 passed)
- [x] five-feature check matrix
- [x] no-feature build proves the schema module is absent
- [x] `cargo fmt --all --check`
- [x] `just validate`

## Checkpoint

- [x] Stop for review before Phase 2; commit schema v1 when the reviewer approves.

## Phase 1 closeout - final v1 metric table

The registry implemented in `src/timing/enabled/schema.rs` is the account of
record for timing schema v1. Level: `Basic` (concise report) or `Detailed`
(verbose/bench only). Relation: `WallSpan`, `Accumulated` or `NestedEvidence`.
Parent values reference another metric's stable name or a well-known human
aggregate row key (`frontend.public_interface`, `frontend.borrow`,
`frontend.generated`).

| Stable name | Level | Relation | Attribution | Command scope | Owner |
|---|---|---|---|---|---|
| `command.build.total` | Basic | WallSpan | None | BuildOnly | Command |
| `command.check.total` | Basic | WallSpan | None | CheckOnly | Command |
| `command.dev.build_write` | Basic | WallSpan | None | DevOnly | Command |
| `command.dev.cycle` | Detailed | WallSpan | None | DevOnly | Command |
| `build.bootstrap.total` | Basic | WallSpan | None | Universal | BuildSystem |
| `build.frontend.total` | Basic | WallSpan | None | Universal | BuildSystem |
| `build.backend.total` | Basic | WallSpan | None | BuildOrDev | BuildSystem |
| `build.output.total` | Basic | WallSpan | None | BuildOrDev | BuildSystem |
| `stage0.directory.inventory` | Basic | WallSpan | None | Universal | Stage0 |
| `stage0.directory.compile` | Basic | WallSpan | None | Universal | Stage0 |
| `stage0.single_file.total` | Basic | WallSpan | None | Universal | Stage0 |
| `boundary.inventory` | Basic | Accumulated | Boundary | Universal | BuildSystem |
| `boundary.compile` | Basic | Accumulated | Boundary | Universal | BuildSystem |
| `frontend.prepare` | Basic | Accumulated | Module | Universal | Frontend |
| `frontend.bind_headers` | Basic | Accumulated | Module | Universal | Frontend |
| `frontend.order_declarations` | Basic | Accumulated | Module | Universal | Frontend |
| `frontend.ast.total` | Basic | Accumulated | Module | Universal | Frontend |
| `frontend.ast.environment` | Basic | NestedEvidence | Module | Universal | Frontend |
| `frontend.ast.emit` | Basic | NestedEvidence | Module | Universal | Frontend |
| `frontend.ast.finalise` | Basic | NestedEvidence | Module | Universal | Frontend |
| `frontend.public_interface.project` | Basic | NestedEvidence | Module | Universal | Frontend |
| `frontend.hir` | Basic | Accumulated | Module | Universal | Frontend |
| `frontend.borrow.initial` | Basic | Accumulated | Module | Universal | Frontend |
| `frontend.borrow.converge` | Basic | Accumulated | Module | Universal | Frontend |
| `frontend.generated.materialise` | Basic | Accumulated | Module | Universal | Frontend |
| `frontend.generated.borrow_recheck` | Basic | Accumulated | Module | Universal | Frontend |
| `frontend.public_interface.finalise` | Basic | NestedEvidence | Module | Universal | Frontend |
| `frontend.module.semantic_total` | Basic | Accumulated | Module | Universal | Frontend |
| `config.ast.total` | Detailed | WallSpan | None | Universal | BuildSystem |
| `config.ast.environment` | Detailed | NestedEvidence | None | Universal | BuildSystem |
| `config.ast.emit` | Detailed | NestedEvidence | None | Universal | BuildSystem |
| `config.ast.finalise` | Detailed | NestedEvidence | None | Universal | BuildSystem |
| `frontend.generated.ast.total` | Detailed | Accumulated | Module | Universal | Frontend |
| `frontend.generated.ast.environment` | Detailed | NestedEvidence | Module | Universal | Frontend |
| `frontend.generated.ast.emit` | Detailed | NestedEvidence | Module | Universal | Frontend |
| `frontend.generated.ast.finalise` | Detailed | NestedEvidence | Module | Universal | Frontend |
| `backend.html.total` | Basic | WallSpan | None | BuildOnly | Backend |
| `backend.js.lower_entry` | Basic | NestedEvidence | None | BuildOnly | Backend |
| `backend.js.lower_linked` | Basic | NestedEvidence | None | BuildOnly | Backend |
| `backend.html.render` | Basic | NestedEvidence | None | BuildOnly | Backend |
| `backend.wasm.total` | Basic | WallSpan | None | BuildOnly | Backend |
| `backend.wasm.lower` | Detailed | NestedEvidence | None | BuildOnly | Backend |
| `backend.wasm.artifacts` | Detailed | NestedEvidence | None | BuildOnly | Backend |
| `backend.assets.plan` | Basic | NestedEvidence | None | BuildOnly | Backend |
| `backend.assets.emit` | Basic | NestedEvidence | None | BuildOnly | Backend |
| `output.write.total` | Basic | WallSpan | None | BuildOnly | BuildSystem |

Parent relationships implemented in the registry:

- `frontend.ast.environment`, `frontend.ast.emit`, `frontend.ast.finalise` ->
  `frontend.ast.total`
- `frontend.public_interface.project`, `frontend.public_interface.finalise` ->
  `frontend.public_interface` (aggregate row, no metric)
- `frontend.borrow.initial`, `frontend.borrow.converge` -> `frontend.borrow`
  (aggregate row, no metric)
- `frontend.generated.materialise`, `frontend.generated.borrow_recheck`,
  `frontend.generated.ast.total` -> `frontend.generated` (aggregate row, no metric)
- `frontend.generated.ast.environment`, `frontend.generated.ast.emit`,
  `frontend.generated.ast.finalise` -> `frontend.generated.ast.total`
- `config.ast.environment`, `config.ast.emit`, `config.ast.finalise` ->
  `config.ast.total`
- `backend.js.lower_entry`, `backend.js.lower_linked`, `backend.html.render`,
  `backend.assets.plan`, `backend.assets.emit` -> `backend.html.total`
- `backend.wasm.lower`, `backend.wasm.artifacts` -> `backend.wasm.total`

### Deliberate compatibility reset

Timing data recorded before schema v1 is legacy and non-comparable. The schema
carries no numeric migration and no aliases for provisional names. Benchmark
reports must label a schema mismatch as non-comparable (see § 12).

---

# Phase 2 - Rebuild session, runtime and aggregate collection

## Context

The committed session generation work is valuable. The runtime now needs explicit channels, lock-free mode reads and an aggregate-first collector.

## Checklist

### Runtime configuration

- [ ] Move timer mode parsing into `runtime.rs`.
- [ ] Use `OnceLock` or equivalent for lock-free production reads.
- [ ] Add pure parsing functions.
- [ ] Replace permanent test mutation with explicit session config or scoped restoration.
- [ ] Add active channel bits.
- [ ] `begin_metric` avoids `Instant` when the metric is inactive.

### Session lifecycle

- [ ] Keep generation-scoped session IDs.
- [ ] Keep matching finish/drop cleanup.
- [ ] Make raw benchmark start fallible.
- [ ] Reject nested raw sessions before compiler work.
- [ ] Keep one active process session.
- [ ] Store command kind explicitly.
- [ ] Store collection channels explicitly.
- [ ] Remove unused `TimingCollectionPurpose` if channels fully own behavior.

### Collector

- [ ] Replace event vectors with dense metric accumulators.
- [ ] Add sample counts only where useful.
- [ ] Use atomic aggregate updates.
- [ ] No global collector mutex on ordinary metric record.
- [ ] Stop recording before snapshot extraction.
- [ ] Build deterministic snapshots in schema order.
- [ ] Flush benchmark lines only after the snapshot exists.

### Counters

- [ ] Support counter-only collection.
- [ ] Preserve `benchmark_counters` independence.
- [ ] Counter summary works with timer Summary, Bench and Silent modes.
- [ ] Counter names remain static internally.

### Record outcome

- [ ] Return structured outcome where prose needs it.
- [ ] A dropped stale context never changes output suppression.
- [ ] Invalid attribution is rejected without emitting a line.

## Tests

- [ ] nested start cannot replace the outer session
- [ ] raw nested start returns an error
- [ ] rejected raw work does not enter an outer snapshot
- [ ] stale finish cannot drain a new session
- [ ] inactive mode captures no clock
- [ ] inactive mode takes no collector lock
- [ ] Bench mode has no attribution metadata
- [ ] Silent plus counter Summary collects counters only
- [ ] schema-order snapshot is deterministic
- [ ] parallel metric additions produce exact totals
- [ ] poisoned lifecycle lock recovery is deliberate

## Audit

- [ ] No thread-local state.
- [ ] No unsafe code unless separately justified and approved.
- [ ] Record path allocates nothing.
- [ ] Record path formats nothing.
- [ ] Lifecycle state has one owner.
- [ ] Session generation reaches attribution validation.

## Style review

- [ ] Use enums instead of boolean-heavy public APIs.
- [ ] Keep atomics internal.
- [ ] Keep lifecycle and aggregate storage separate.
- [ ] Remove stale event-log terminology.

## Validation

- [ ] collector/session/runtime unit tests
- [ ] five-feature check matrix
- [ ] timers and detailed-timers test suites
- [ ] counters feature combinations
- [ ] `just timers-erasure-check`
- [ ] `just validate`

## Checkpoint

- [ ] Commit the runtime before call-site migration.

---

# Phase 3 - Finalise the compile-erasing facade and module layout

## Context

This phase consumes the useful uncommitted macro work and produces the one final facade. It does not migrate every compiler call site yet.

## Checklist

### Module layout

- [ ] Move `enabled.rs` to `enabled/mod.rs`.
- [ ] Add `guard.rs`.
- [ ] Keep `command.rs` and `counter_summary.rs` when their ownership is clean.
- [ ] Replace wildcard re-export with explicit exports.
- [ ] Update `index.md` if required.

### Macro facade

- [ ] Implement the final macro surface.
- [ ] Use typed `TimingMetric`.
- [ ] Disabled expression macros return the production expression.
- [ ] Disabled statement macros emit nothing.
- [ ] Metric, context, label, command and finish expressions are not evaluated without `timers`.
- [ ] Enabled expression macros execute production work once.
- [ ] Begin paths skip clocks for inactive metrics.
- [ ] Detailed prose uses the stored duration.
- [ ] Remove old macros and helpers.
- [ ] Move timer-specific macros out of `compiler_dev_logging`.

### Guards

- [ ] `finish()` records once.
- [ ] Drop after finish is a no-op.
- [ ] Early return records once.
- [ ] Add compile-erasing `finish_timing_scope!`.
- [ ] Multi-metric guards capture one duration.
- [ ] No hidden fixed binding names.

### Tests

For every final macro:

- [ ] no-feature arguments are not evaluated
- [ ] inactive timer mode avoids a clock
- [ ] values pass through
- [ ] `Result` values pass through
- [ ] work executes once
- [ ] finish plus Drop records once
- [ ] error-path Drop records once
- [ ] multi-record values are identical
- [ ] detailed prose and snapshot duration agree

## Audit

- [ ] No direct timer `Instant::now()` outside the facade/guard implementation.
- [ ] No direct collector calls outside timing internals.
- [ ] No old macro name remains.
- [ ] No compatibility forwarding macro remains.
- [ ] Production control flow is readable.

## Style review

- [ ] Macros stay small.
- [ ] Ordinary compiler flow is not hidden by clever syntax.
- [ ] Names and argument ordering are consistent.
- [ ] No broad lint allowance.

## Validation

- [ ] no-feature macro suite
- [ ] timer macro suite
- [ ] detailed macro suite
- [ ] five-feature matrix
- [ ] cross-target Clippy
- [ ] `just timers-erasure-check`
- [ ] `just validate`

## Checkpoint

- [ ] Commit the facade before broad call-site migration.

---

# Phase 4 - Migrate command, build, Stage 0, config and output timing

## Context

Choose semantic v1 boundaries. Do not preserve accidental legacy endpoints.

## Checklist

### Commands

- [ ] Convert build to one owned command session.
- [ ] Convert check to one owned command session.
- [ ] Keep user-visible `Done in` clocks separate from timer instrumentation.
- [ ] Capture command totals before timer render/bench flush.
- [ ] Refactor early returns into outcome helpers when useful.
- [ ] Ensure failure paths finish the session once.
- [ ] Use explicit Build and Check kinds.

### Build orchestration

- [ ] Instrument bootstrap, frontend, backend and output owners.
- [ ] Record raw parent totals where they measure genuine wider spans.
- [ ] Avoid duplicate rows that share the same duration accidentally.
- [ ] Hide bootstrap microsteps from basic mode.

### Stage 0

- [ ] Define directory inventory boundary.
- [ ] Define directory compile boundary.
- [ ] Define single-file total.
- [ ] Register boundaries lazily only when attribution is active.
- [ ] Retain returned module keys.
- [ ] Reject conflicting registration metadata.
- [ ] Record boundary inventory and compile work.

### Config

- [ ] Use config-specific metric IDs.
- [ ] Config AST uses `config.ast.*`.
- [ ] Keep config microstages detailed-only.
- [ ] Remove noisy zero-cost timers.

### Output

- [ ] Define `output.write.total`.
- [ ] Keep preflight, cleanup and emission children detailed-only.
- [ ] Ensure parent/child overlap is explicit.

### Tests

- [ ] successful build
- [ ] failed build
- [ ] successful check
- [ ] failed check
- [ ] directory project
- [ ] single-file project
- [ ] config-heavy project
- [ ] boundary ordering
- [ ] boundary duplicate conflict
- [ ] command total excludes timer rendering
- [ ] Bench output flushes at command end

## Audit

- [ ] Stage ownership matches build-system design.
- [ ] No timer context enters retained build artefacts.
- [ ] No output timer changes filesystem behavior.
- [ ] Every basic metric has a documented v1 endpoint.

## Style review

- [ ] Use expression macros for clean spans.
- [ ] Use guards only for real multi-exit scopes.
- [ ] No scattered manual start/finish plumbing.
- [ ] Comments explain non-obvious semantic boundaries only.

## Validation

- [ ] targeted command/build/Stage 0/output tests
- [ ] five-feature matrix
- [ ] build/check smokes in Summary, Bench, Verbose and Silent modes
- [ ] `just bench-check`
- [ ] `just timers-erasure-check`
- [ ] `just validate`

## Checkpoint

- [ ] Commit before frontend migration.

---

# Phase 5 - Migrate frontend, AST, generated work and borrow timing

## Context

This phase establishes the most important long-term compiler-stage breakdown.

## Checklist

### Preparation and semantic orchestration

- [ ] Instrument source preparation per module.
- [ ] Instrument header binding.
- [ ] Instrument declaration ordering.
- [ ] Instrument complete module semantic work.
- [ ] Preserve explicit module attribution.

### AST

- [ ] Module AST uses `frontend.ast.*`.
- [ ] Config AST uses `config.ast.*`.
- [ ] Generated AST uses `frontend.generated.ast.*`.
- [ ] `frontend.ast.environment` includes complete environment assembly.
- [ ] `frontend.ast.emit` measures emitter production.
- [ ] `frontend.ast.finalise` measures finaliser production.
- [ ] Parent module AST total wraps the complete owner.
- [ ] Failed environment, emission and finalisation record partial elapsed work.
- [ ] No contextual filtering is needed to distinguish AST owner classes.

### Public interface and HIR

- [ ] Record projection leaf.
- [ ] Record HIR lowering.
- [ ] Record post-borrow finalisation leaf.
- [ ] Derive the human Public interface row from the two leaves.

### Borrow and generated work

- [ ] Record initial borrow analysis.
- [ ] Record direct convergence work.
- [ ] Record generated materialisation.
- [ ] Record generated borrow rechecks.
- [ ] Keep generated AST children under generated work, not base module AST.
- [ ] Avoid double counting in the human report.

### Detailed TIR evidence

- [ ] Retain useful detailed TIR/folding substages.
- [ ] Remove detailed microtimers that cannot guide an investigation.
- [ ] Keep constant/template finalisation under its real AST finaliser owner.

### Slowest module

- [ ] Compute from module prepare plus semantic total.
- [ ] Tie-break deterministically by registration order.
- [ ] Include diagnosed modules where timing evidence exists.

## Tests

- [ ] config AST never appears under module AST
- [ ] generated AST never appears under module AST
- [ ] module AST children belong to the same module context
- [ ] failed AST stages record partial work
- [ ] public-interface human sum is exact
- [ ] generated work is counted once
- [ ] slowest module is deterministic
- [ ] no absolute paths
- [ ] parallel preparation attribution is stable

## Audit

- [ ] No second AST, TIR, HIR or source walk.
- [ ] Timer context remains cfg-gated.
- [ ] Frontend orchestration stays readable.
- [ ] Metric boundaries match compiler stage ownership.

## Style review

- [ ] No timer abstraction leaks into AST/HIR data.
- [ ] No tuple-heavy timing payloads.
- [ ] Stage comments remain concise.
- [ ] Tests protect ownership, not old implementation accidents.

## Validation

- [ ] frontend and AST unit tests
- [ ] config-heavy smoke
- [ ] generic-heavy smoke
- [ ] failed-AST smoke
- [ ] `just bench-frontend-check`
- [ ] five-feature matrix
- [ ] `just timers-erasure-check`
- [ ] `just validate`

## Checkpoint

- [ ] Commit before backend and dev migration.

---

# Phase 6 - Migrate backend, assets, output children and dev timing

## Context

Backend metrics should expose meaningful build owners without flooding the basic report.

## Checklist

### HTML and JS

- [ ] Record HTML backend total.
- [ ] Record entry JS lowering.
- [ ] Record linked JS lowering.
- [ ] Record HTML render.
- [ ] Keep config/path/glue microsteps detailed-only.

### Wasm

- [ ] Record complete Wasm build.
- [ ] Record Wasm lowering.
- [ ] Record artifact/bootstrap assembly.
- [ ] Human label is `Wasm build`.
- [ ] Do not call the full build `Wasm lowering`.

### Assets

- [ ] Record planning.
- [ ] Record emission.
- [ ] Human Tracked assets row sums both.
- [ ] Avoid zero-duration noise.

### Dev

- [ ] One session per initial build/rebuild.
- [ ] Build/write timing wraps the executor trait call.
- [ ] Full dev cycle remains detailed-only.
- [ ] Watch polling stays outside build/write.
- [ ] State mutation, error-page rendering and broadcast stay outside build/write.
- [ ] Status line prints before the structured report.
- [ ] Every successful and failed cycle finishes once.
- [ ] Live rebuild and queued rebuild paths use the same owner.

## Tests

- [ ] linked JS lowering is separate
- [ ] Wasm total and lower are distinct
- [ ] tracked asset aggregate is exact
- [ ] backend microsteps stay hidden from basic output
- [ ] initial dev cycle
- [ ] watch-triggered rebuild
- [ ] failed rebuild
- [ ] fake executor receives dev total
- [ ] no cross-cycle leakage

## Audit

- [ ] Backend code owns measurements, summary owns presentation.
- [ ] No backend reparses source.
- [ ] Dev session does not include watch polling.
- [ ] No timing state enters output plans.

## Style review

- [ ] Keep backend helpers narrow.
- [ ] Do not introduce a generic tracing abstraction.
- [ ] Use one dev timing path.

## Validation

- [ ] backend artifact tests
- [ ] HTML and HTML-Wasm integration tests
- [ ] dev tests
- [ ] dev initial and live rebuild smoke where environment permits
- [ ] `just bench-check`
- [ ] five-feature matrix
- [ ] `just timers-erasure-check`
- [ ] `just validate`

## Checkpoint

- [ ] Commit before summary finalisation.

---

# Phase 7 - Rebuild summary policy and renderer over schema v1

## Context

The summary should consume typed aggregates and one policy table. It must not infer architecture from strings.

## Checklist

### Report policy

- [ ] Replace raw string policies with `TimingMetric` IDs.
- [ ] One descriptor owns display and command accounting.
- [ ] Add command-specific pipeline titles.
- [ ] Add basic/detailed visibility.
- [ ] Add parent/child relationships.
- [ ] Add threshold behavior.
- [ ] Add command masks.
- [ ] Remove dead emphasis/suffix fields or make them meaningful.

### Accounting

- [ ] Build accounted wall time from policy rows.
- [ ] Detect overlap.
- [ ] Detect accounted > command total.
- [ ] Compute Other without saturation.
- [ ] Test directory and single-file fallbacks.
- [ ] Never add accumulated or nested rows to command accounting.

### Dynamic sections

- [ ] Boundaries in registration order.
- [ ] Frontend module count with singular/plural.
- [ ] Slowest module.
- [ ] KiB size labels.
- [ ] Long identity truncation.
- [ ] Recursive alignment including child labels.
- [ ] Padded boundary columns.

### Rendering

- [ ] Keep pure line helpers.
- [ ] `saying::say!` only.
- [ ] No ANSI literals.
- [ ] Command total remains visually distinct.
- [ ] Basic output omits zero rows.

## Tests

- [ ] complete top-level order
- [ ] command-specific sections
- [ ] policy uniqueness
- [ ] accounting overlap rejection
- [ ] invalid accounted total
- [ ] 1ms and 5% thresholds
- [ ] singular/plural
- [ ] recursive alignment
- [ ] boundary alignment
- [ ] long identity
- [ ] KiB formatting
- [ ] unknown metric cannot appear
- [ ] deterministic output from parallel record order

## Audit

- [ ] Renderer contains no compiler-stage logic.
- [ ] Summary contains no terminal styling.
- [ ] Metric policy has one owner.
- [ ] No BTreeMap keyed by raw metric strings remains.

## Style review

- [ ] Static labels use `&'static str`.
- [ ] Dynamic strings are restricted to project/package/module identities and final rendered lines.
- [ ] No dead model states.
- [ ] No broad lint allowance.

## Validation

- [ ] summary and render tests
- [ ] docs project summary smoke
- [ ] single-file summary smoke
- [ ] failed command summary smoke
- [ ] dev summary smoke
- [ ] Wasm summary smoke
- [ ] `just validate`

## Checkpoint

- [ ] Commit before benchmark reset.

---

# Phase 8 - Reset benchmark timing identity and strengthen erasure

## Context

The benchmark system must understand timing schema v1. Old data is deliberately incompatible.

## Checklist

### Benchmark schema

- [ ] Add timing schema version to CLI benchmark observations.
- [ ] Add timing schema version to frontend benchmark reports.
- [ ] Require exactly one schema record.
- [ ] Emit one aggregate line per metric in schema order.
- [ ] Include schema version in measurement fingerprints.
- [ ] Bump benchmark protocol/format where required.
- [ ] Report schema mismatch as non-comparable.
- [ ] Remove provisional repeated-line assumptions if no longer needed.
- [ ] Reset local timing history.
- [ ] Regenerate tracked current summaries only when required.

### Erasure source audit

- [ ] Reject direct timer clock creation outside timing internals.
- [ ] Maintain a narrow allowlist for normal user-visible and benchmark harness clocks.
- [ ] Reject direct collector calls.
- [ ] Reject runtime `cfg!(feature = "timers")`.
- [ ] Reject disabled macro helper/closure calls.
- [ ] Reject timing-only fields or parameters without cfg.
- [ ] Reject old macro names.
- [ ] Reject old raw metric string literals outside schema tests/history docs.

### Binary audit

- [ ] Build clean no-feature release binary.
- [ ] Derive representative timer markers from schema inventory.
- [ ] Scan environment names, headings and metrics.
- [ ] Keep symbol/LLVM checks supplemental.
- [ ] Do not require exact binary size or assembly equality.

### Benchmark tests

- [ ] schema line parsing
- [ ] missing schema rejection
- [ ] duplicate schema rejection
- [ ] unknown future schema rejection
- [ ] deterministic metric order
- [ ] one aggregate line per metric
- [ ] frontend/CLI schema agreement
- [ ] legacy data marked incompatible

## Audit

- [ ] No numeric history migration.
- [ ] No compatibility shim hides old/new boundary differences.
- [ ] Erasure gate covers the final facade.
- [ ] Counter-only builds do not pull in timer implementation.

## Style review

- [ ] Benchmark parser error messages are precise.
- [ ] Timing schema ownership remains in timing, comparison ownership remains in xtask.
- [ ] Source audit is maintainable and tested.

## Validation

- [ ] xtask tests
- [ ] `just timers-erasure-check`
- [ ] `just bench-check`
- [ ] `just bench-frontend-check`
- [ ] five-feature matrix
- [ ] cross-target Clippy
- [ ] `just validate`

## Checkpoint

- [ ] Commit benchmark/schema reset before documentation closeout.

---

# Phase 9 - Documentation, final audits and closeout

## Context

Make schema v1 and its ownership reloadable. Close both superseded plans and this plan cleanly.

## Checklist

### Documentation

- [ ] Update `benchmarks/README.md`:
  - [ ] product boundaries
  - [ ] mode/channel matrix
  - [ ] timing schema v1
  - [ ] wall, accumulated and nested meaning
  - [ ] command/boundary/module attribution
  - [ ] benchmark incompatibility rules
  - [ ] zero-cost rule
- [ ] Update `Cargo.toml` feature comments.
- [ ] Update timing module docs.
- [ ] Update validation docs for the strengthened erasure gate.
- [ ] Update `index.md` for moved timing modules.
- [ ] Mark the original timer summary plan superseded.
- [ ] Mark the correction plan superseded.
- [ ] Record final schema table and checkpoint in this plan.
- [ ] Do not add to roadmap.
- [ ] Do not update progress matrix.

### Final zero-cost audit

- [ ] no timer clocks without feature
- [ ] no timer types or statics
- [ ] no timer fields or arguments
- [ ] no timer strings
- [ ] no no-op calls
- [ ] no timer environment reads
- [ ] no dead compatibility APIs

### Final architecture audit

- [ ] command ownership
- [ ] Stage 0 ownership
- [ ] frontend/AST/TIR/HIR ownership
- [ ] config/generated AST separation
- [ ] backend ownership
- [ ] dev lifecycle
- [ ] attribution and parallel determinism
- [ ] collector self-interference
- [ ] summary accounting
- [ ] benchmark schema identity

### Final validation

- [x] `cargo fmt --all --check`
- [ ] `cargo check --no-default-features`
- [ ] `cargo check --features timers`
- [ ] `cargo check --features detailed_timers`
- [ ] `cargo check --features timers,benchmark_counters`
- [ ] `cargo check --features benchmark_counters`
- [ ] no-feature tests
- [ ] timer tests
- [ ] detailed-timer tests
- [ ] counter feature tests
- [ ] xtask tests
- [ ] `just timers-erasure-check`
- [ ] `just bench-check`
- [ ] `just bench-frontend-check`
- [ ] `just validate`
- [ ] docs check
- [ ] docs release build
- [ ] build/check success and failure smokes
- [ ] directory and single-file smokes
- [ ] config-heavy smoke
- [ ] generic-heavy smoke
- [ ] JS and Wasm smokes
- [ ] initial dev and watch rebuild smokes

### Closeout

- [ ] Record starting and final commits.
- [ ] Record timing schema version.
- [ ] Record final metric inventory.
- [ ] Record benchmark protocol reset.
- [ ] Record validation commands and results.
- [ ] Record any known unrelated failures.
- [ ] Record deferred work.
- [ ] Leave worktree clean.
- [ ] Stop for coordinator acceptance.

---

# Deliberately deferred

These are not hidden TODOs.

- more than one active process timing session
- nested tracing spans
- thread-local attribution
- persistent timer history in the compiler
- performance pass/fail gates in the basic report
- user-configurable sections, thresholds or colours
- full module tables in basic output
- exact exclusive-time reconstruction
- allocation profiler integration
- flamegraph generation
- remote telemetry
- CI performance dashboards
- interactive drill-down
- lock-free dynamic attribution beyond the v1 aggregate design
- automatic cache-hit timing
- public stable timer API
- migration of pre-v1 timing values
- roadmap insertion
- progress-matrix tracking

The benchmark and profiling systems remain the owners of history, profiles and future performance gates.

---

# Definition of done

The timer system is finalised when:

- a no-timer build is structurally free of timer-system work
- timing schema v1 has one typed metric registry
- provisional raw metric strings and `_ms` names are gone
- config, module and generated AST work have distinct identities
- metric boundaries are semantically owned and documented
- inactive timer modes avoid clocks and collection locks
- active recording performs no formatting or allocation
- the collector stores dense aggregates rather than raw events
- session ownership cannot destroy or contaminate another collection
- raw benchmarks fail cleanly when they cannot own the collector
- boundary/module IDs are generation-scoped and registration-validated
- the macro facade has one compile-erasing API
- guards record exactly once and end at semantic boundaries
- the basic summary has one display/accounting policy
- detailed timers add evidence without redefining basic metrics
- benchmark output includes timing schema v1 and deterministic aggregate lines
- pre-v1 benchmark timing is treated as incompatible
- the erasure gate protects both source structure and the release binary
- documentation names the final owners
- all required validation is green
- both older timer plans are marked superseded
- this plan is marked complete
