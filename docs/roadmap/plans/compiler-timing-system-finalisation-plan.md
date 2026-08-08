# Compiler timing system finalisation plan

## Status

- **Plan state:** authoritative replacement plan, Phase 3 implementation complete and checkpoint committed
- **Committed correction baseline:** `3abc75f6f`
- **Intended repository path:** `docs/roadmap/plans/compiler-timing-system-finalisation-plan.md`
- **Roadmap status:** do not add this plan to `docs/roadmap/roadmap.md` unless the coordinator requests it separately
- **Primary invariant:** a compiler built without `timers` performs no timer-system runtime work
- **Timing compatibility stance:** this plan deliberately establishes timing schema v1. Timing data recorded before v1 is legacy and non-comparable
- **Historical-plan policy:** the two earlier timing-plan files were deliberately removed in
  `77a45e790708f76f8b96267991a0370a3ecfc9c8`; Git history retains them. This plan is the
  single current timing authority and must not claim that deleted files remain marked in place.

The earlier plans established the concise report, zero-cost feature boundary, project/package attribution, initial session ownership and several frontend/backend observations. This plan owns the final architecture, naming, measurement boundaries and implementation path.

The implementation is early alpha. Prefer correct long-term stage ownership, a narrow API and low self-interference over compatibility with provisional metric names or historical measurements.

---

## Active context capsule

Refresh this block after every accepted slice and before compaction.

```text
ACTIVE_PLAN:
- `docs/roadmap/plans/compiler-timing-system-finalisation-plan.md`

PROGRESS_RECORD:
- WORK_ID: `compiler-timing-finalisation`
- WORK_SOURCE: this plan and the 2026-08-08 Phase 0/1 correction review
- BASE_REVISION: `3abc75f6f`
- STATUS: active
- CURRENT_SCOPE: Phase 3 checkpoint complete and ready for Phase 4
- COMPLETED: Phase 0/1 correction checkpoint committed as `3abc75f6f`; Phase 2 committed as `a9b970bb6` with immutable runtime configuration, explicit session channels, fallible raw-session ownership, atomic inactive fast paths and focused lifecycle regressions while retaining raw event snapshots. Metric-only raw sessions skip facade attribution-context expressions. The final audit's stale multi-record output gap is covered by `multi_record_outcome_rejects_stale_contexts_before_bench_emission`. The retained Phase 3 candidate has been fully diff-inspected, its AST timing-family visibility path corrected, provisional production names migrated to schema-v1 identities, obsolete live timers removed, summary command applicability corrected, and focused regression coverage added. The stale counter expectation in `chunked_file_preparation_skips_identity_payload_remap` was corrected after confirming the same expectation failed at the accepted Phase 2 baseline. The existing forced output-plan failure regression now also asserts `build.output.total`.
- VALIDATION: Phase 3 passes `cargo fmt --all --check`; the five-feature check matrix (`cargo check --no-default-features`, `timers`, `detailed_timers`, `timers,benchmark_counters` and `benchmark_counters`); `cargo test --features timers,benchmark_counters --lib -- --format terse` (4,185 tests); `cargo test --features detailed_timers --lib -- --format terse` (4,176 tests); the timing-focused suite (81 tests); the focused frontend suite (38 tests); the forced output-plan regression; `just timers-erasure-check`; and the complete `just validate` gate. The final gate passed native, Linux-x64 and Windows-x64 Clippy, workspace tests (4,174, 17 and 601 passing test groups), 1,818 integration cases, docs checking, all 60 benchmark preflights and timer erasure.
- AUDITS: Phase 0/1 final auditor found one low-severity `§ 12` cross-reference; corrected to `§ 10`. Phase 2 interim audit found two low-severity gaps: restore one-lock multi-recording and add runtime-gated macro regressions. Both are corrected. The Phase 3 interim auditor route was attempted twice and the configured `final_auditor` route was attempted twice. Every attempt terminated before handoff with HTTP 429 from the only eligible Ollama provider, with no worktree changes. The coordinator completed a local final audit covering registry identity, stale names, stage ownership, attribution, exact-once AST totals, backend/output applicability, failure-path guards, tests, erasure and documentation accuracy. No code findings remain; the independent route limitation is recorded explicitly.
- BLOCKERS: No code blockers remain after the AST timing-family visibility correction. The independent auditor could not produce a handoff because the only eligible provider repeatedly returned HTTP 429; the Codex provider remained disabled by configured routing. Phase 3 is accepted with that audit limitation recorded.
- NOTES: Phase 0 and Phase 1 were committed together in `77a45e790708f76f8b96267991a0370a3ecfc9c8`; their correction review became the accepted checkpoint `3abc75f6f`. Phase 2 is its own accepted checkpoint. The candidate was explicitly retained after complete diff inspection, validated locally and accepted as the Phase 3 checkpoint.

CURRENT_SLICE:
- Phase: Phase 3 semantic call-site migration
- Goal: migrate production timing call sites to v1 names and semantic endpoints while retaining raw event storage
- Non-goals: typed facade, dense collector and typed summary-policy work

LAST_GOOD_COMMIT:
- `a9b970bb6`

CURRENT_WORKTREE_STATE:
- Confirmed clean on `main` at `a9b970bb6` before the attempted Phase 3 work
- Phase 0 and Phase 1 were committed together
- Phase 0/1 correction checkpoint committed separately
- Phase 2 deliberately retains the raw event vector and passed its final audit cycle and serial full validation gate
- The retained, compiled and fully validated Phase 3 candidate across command, build-system, frontend, AST, backend and summary code plus focused tests is committed. The worktree is clean. The independent audit route limitation is recorded above.
- No typed facade or dense collector work has started

BLOCKERS / RISKS:
- The Phase 3 candidate was retained explicitly after reconciliation and has passed the local audit and full validation gate
- raw metric call sites intentionally remain until corrected Phase 3; no compatibility mapper may be introduced before then
- preserve raw event snapshots until Phase 5; do not introduce a parallel collector
- inactive channels must avoid timing clocks and collector locks

NEXT_ACTION:
- Begin Phase 4 typed-facade and collector work from the accepted Phase 3 checkpoint.
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
    pub(crate) parent: Option<TimingParent>,
    pub(crate) accounting: TimingAccountingRole,
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

pub(crate) enum TimingParent {
    Metric(TimingMetric),
    SummaryGroup(TimingSummaryGroup),
}

pub(crate) enum TimingSummaryGroup {
    PublicInterface,
    BorrowValidation,
    GeneratedFunctions,
}

pub(crate) enum TimingAccountingRole {
    CommandTotal,
    Pipeline(TimingPipelineStage),
    Evidence,
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
| `backend.js.lower_entry` | entry-module HIR to JS lowering |
| `backend.js.lower_linked` | linked-module HIR to JS lowering |
| `backend.html.render` | HTML document rendering |
| `backend.wasm.total` | complete HTML-Wasm route build |
| `backend.wasm.lower` | Wasm lowering only |
| `backend.wasm.artifacts` | Wasm artifact and bootstrap assembly |
| `backend.assets.plan` | tracked/runtime asset planning |
| `backend.assets.emit` | tracked/runtime asset emission |
| `output.write.total` | complete output write orchestration |

`build.backend.total` is the sole generic selected-backend pipeline span.
There is no `backend.html.total`: HTML, JS, Wasm and asset metrics are
evidence nested below the generic pipeline span. Backend and output metrics
apply to `Build` and `Dev`, never `Check`.

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

`TimingParent::Metric` identifies a real measured containing span.
`TimingParent::SummaryGroup` identifies only a typed human aggregate of
disjoint accumulated work. It never pretends that an unmeasured aggregate row
was a parent duration.

`TimingAccountingRole` records command-accounting ownership in the schema:
the command total, each unique top-level pipeline segment, or non-accounted
evidence. Stage 0 directory and single-file spans are nested evidence under
`build.frontend.total`; `output.write.total` is nested under
`build.output.total`; boundary inventory and compile remain accumulated Stage
0 attribution evidence and never command-accounting children.

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

Names may be adjusted once during corrected Phase 4 for consistency. There must be one final shape.

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
- directory inventory and compile are nested evidence under `build.frontend.total`
- single-file timing is the alternative nested frontend path under `build.frontend.total`
- boundary inventory and compile are accumulated Stage 0 attributed work, never command-accounting children

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
- both leaves are `Accumulated` under `TimingSummaryGroup::PublicInterface`
- do not record a duplicate aggregate span with the same values

### Borrow and generated work

Keep distinct:

- initial direct borrow analysis
- direct summary convergence
- generated materialisation
- generated borrow rechecks

The human report may group them, but raw schema keeps their owners separate.

### Backend

- `build.backend.total` is the one selected-backend command-pipeline span
- `backend.html.total` does not exist because it would duplicate the generic span
- `backend.wasm.total` means complete Wasm route build and is displayed as `Wasm build`
- Wasm lowering alone uses `backend.wasm.lower`
- tracked assets group plan and emission or label emission precisely
- JS entry and linked lowering remain distinct raw metrics
- HTML, JS, Wasm and asset evidence is nested under `build.backend.total`
- output write evidence is nested under `build.output.total`
- backend and output evidence applies to build and dev, never check
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

# Phase 0 - Reconcile the migration and install this authority

## Recorded deviation

Phase 0 and Phase 1 were committed together in
`77a45e790708f76f8b96267991a0370a3ecfc9c8`, so the intended review gate did
not occur between them. Do not rewrite history. This correction review acts
as the missed gate, then later phases return to one accepted checkpoint per
phase.

## Accepted retained foundation

- declarative dense metric enum and descriptor table
- `TIMING_SCHEMA_VERSION = 1` with no legacy aliases
- separate config, module and generated AST identities
- dedicated `command.rs` and `counter_summary.rs`
- exact-once named guards and direct-expression disabled macro arms
- explicit facade re-exports and the enabled-only schema module
- unified instrumentation test lock and all pre-v1 timing-data reset policy

## Reconciliation record

- [x] Confirm the worktree was clean at
  `77a45e790708f76f8b96267991a0370a3ecfc9c8` before corrections.
- [x] Preserve the committed implementation rather than reverting it.
- [x] Record that the historic timer-plan files were deleted and remain
  available through Git history.
- [x] Remove the accidental roadmap entry because no separate coordinator
  approval made this plan roadmap-active.
- [x] Refresh the active context capsule with the actual baseline and risks.
- [x] Accept this correction review after the focused validation and final
  audit; its one documentation finding was corrected.

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
- [x] every basic command-accounting span has a unique typed pipeline role
- [x] every attributed metric permits the supplied context kind
- [x] every command total is unique
- [x] every metric parent is typed and every virtual group is a typed summary group
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

- [x] At the original `77a45e790708f76f8b96267991a0370a3ecfc9c8`
  checkpoint: schema unit tests (12 tests, `cargo test --features timers --lib timing` -> 69 passed).
- [x] five-feature check matrix
- [x] no-feature build proves the schema module is absent
- [x] `cargo fmt --all --check`
- [x] `just validate`

## Checkpoint

- [x] The original checkpoint was committed with Phase 0 in
  `77a45e790708f76f8b96267991a0370a3ecfc9c8`.
- [x] Complete the correction review before Phase 2.

## Phase 0/1 correction checkpoint

- [x] Replace string parents with `TimingParent::Metric` and
  `TimingParent::SummaryGroup`.
- [x] Make public-interface leaves accumulated
  `TimingSummaryGroup::PublicInterface` evidence.
- [x] Record typed command-accounting roles and reject duplicate Basic
  pipeline spans.
- [x] Remove duplicate `backend.html.total` and attach HTML, JS, Wasm and
  asset evidence to `build.backend.total`.
- [x] Make backend and output metrics `BuildOrDev`; check never records them.
- [x] Define Stage 0/frontend, output/build-output and boundary/Stage-0
  relationships.
- [x] Replace leaking check/config guards with endpoint-timed expressions or
  blocks and test their completion order.
- [x] Converge failed output-plan construction through the build-command total
  and test it.
- [x] Reconcile roadmap, historical-plan policy and the active context.
- [x] Reorder the remaining phases to remove the typed-schema/collector gap.
- [x] Run the correction validation matrix and audits.
- [x] Create correction checkpoint `3abc75f6f` before starting Phase 2.

## Phase 1 closeout - final v1 metric table

The registry implemented in `src/timing/enabled/schema.rs` is the account of
record for timing schema v1. Level: `Basic` (concise report) or `Detailed`
(verbose/bench only). Relation: `WallSpan`, `Accumulated` or `NestedEvidence`.
Parent values are typed `Metric(...)` spans or `Group(...)` human aggregates.
Accounting is `CommandTotal`, `Pipeline(...)` or non-accounted `Evidence`.

| Stable name | Level | Relation | Attribution | Scope | Parent | Accounting | Owner |
|---|---|---|---|---|---|---|---|
| `command.build.total` | Basic | WallSpan | None | BuildOnly | — | CommandTotal | Command |
| `command.check.total` | Basic | WallSpan | None | CheckOnly | — | CommandTotal | Command |
| `command.dev.build_write` | Basic | WallSpan | None | DevOnly | — | CommandTotal | Command |
| `command.dev.cycle` | Detailed | WallSpan | None | DevOnly | — | Evidence | Command |
| `build.bootstrap.total` | Basic | WallSpan | None | Universal | — | Pipeline(Bootstrap) | BuildSystem |
| `build.frontend.total` | Basic | WallSpan | None | Universal | — | Pipeline(Frontend) | BuildSystem |
| `build.backend.total` | Basic | WallSpan | None | BuildOrDev | — | Pipeline(Backend) | BuildSystem |
| `build.output.total` | Basic | WallSpan | None | BuildOrDev | — | Pipeline(Output) | BuildSystem |
| `stage0.directory.inventory` | Basic | NestedEvidence | None | Universal | Metric(`build.frontend.total`) | Evidence | Stage0 |
| `stage0.directory.compile` | Basic | NestedEvidence | None | Universal | Metric(`build.frontend.total`) | Evidence | Stage0 |
| `stage0.single_file.total` | Basic | NestedEvidence | None | Universal | Metric(`build.frontend.total`) | Evidence | Stage0 |
| `boundary.inventory` | Basic | Accumulated | Boundary | Universal | — | Evidence | Stage0 |
| `boundary.compile` | Basic | Accumulated | Boundary | Universal | — | Evidence | Stage0 |
| `frontend.prepare` | Basic | Accumulated | Module | Universal | — | Evidence | Frontend |
| `frontend.bind_headers` | Basic | Accumulated | Module | Universal | — | Evidence | Frontend |
| `frontend.order_declarations` | Basic | Accumulated | Module | Universal | — | Evidence | Frontend |
| `frontend.ast.total` | Basic | Accumulated | Module | Universal | — | Evidence | Frontend |
| `frontend.ast.environment` | Basic | NestedEvidence | Module | Universal | Metric(`frontend.ast.total`) | Evidence | Frontend |
| `frontend.ast.emit` | Basic | NestedEvidence | Module | Universal | Metric(`frontend.ast.total`) | Evidence | Frontend |
| `frontend.ast.finalise` | Basic | NestedEvidence | Module | Universal | Metric(`frontend.ast.total`) | Evidence | Frontend |
| `frontend.public_interface.project` | Basic | Accumulated | Module | Universal | Group(PublicInterface) | Evidence | Frontend |
| `frontend.hir` | Basic | Accumulated | Module | Universal | — | Evidence | Frontend |
| `frontend.borrow.initial` | Basic | Accumulated | Module | Universal | Group(BorrowValidation) | Evidence | Frontend |
| `frontend.borrow.converge` | Basic | Accumulated | Module | Universal | Group(BorrowValidation) | Evidence | Frontend |
| `frontend.generated.materialise` | Basic | Accumulated | Module | Universal | Group(GeneratedFunctions) | Evidence | Frontend |
| `frontend.generated.borrow_recheck` | Basic | Accumulated | Module | Universal | Group(GeneratedFunctions) | Evidence | Frontend |
| `frontend.public_interface.finalise` | Basic | Accumulated | Module | Universal | Group(PublicInterface) | Evidence | Frontend |
| `frontend.module.semantic_total` | Basic | Accumulated | Module | Universal | — | Evidence | Frontend |
| `config.ast.total` | Detailed | NestedEvidence | None | Universal | Metric(`build.bootstrap.total`) | Evidence | BuildSystem |
| `config.ast.environment` | Detailed | NestedEvidence | None | Universal | Metric(`config.ast.total`) | Evidence | BuildSystem |
| `config.ast.emit` | Detailed | NestedEvidence | None | Universal | Metric(`config.ast.total`) | Evidence | BuildSystem |
| `config.ast.finalise` | Detailed | NestedEvidence | None | Universal | Metric(`config.ast.total`) | Evidence | BuildSystem |
| `frontend.generated.ast.total` | Detailed | Accumulated | Module | Universal | Group(GeneratedFunctions) | Evidence | Frontend |
| `frontend.generated.ast.environment` | Detailed | NestedEvidence | Module | Universal | Metric(`frontend.generated.ast.total`) | Evidence | Frontend |
| `frontend.generated.ast.emit` | Detailed | NestedEvidence | Module | Universal | Metric(`frontend.generated.ast.total`) | Evidence | Frontend |
| `frontend.generated.ast.finalise` | Detailed | NestedEvidence | Module | Universal | Metric(`frontend.generated.ast.total`) | Evidence | Frontend |
| `backend.js.lower_entry` | Basic | NestedEvidence | None | BuildOrDev | Metric(`build.backend.total`) | Evidence | Backend |
| `backend.js.lower_linked` | Basic | NestedEvidence | None | BuildOrDev | Metric(`build.backend.total`) | Evidence | Backend |
| `backend.html.render` | Basic | NestedEvidence | None | BuildOrDev | Metric(`build.backend.total`) | Evidence | Backend |
| `backend.wasm.total` | Basic | NestedEvidence | None | BuildOrDev | Metric(`build.backend.total`) | Evidence | Backend |
| `backend.wasm.lower` | Detailed | NestedEvidence | None | BuildOrDev | Metric(`backend.wasm.total`) | Evidence | Backend |
| `backend.wasm.artifacts` | Detailed | NestedEvidence | None | BuildOrDev | Metric(`backend.wasm.total`) | Evidence | Backend |
| `backend.assets.plan` | Basic | NestedEvidence | None | BuildOrDev | Metric(`build.backend.total`) | Evidence | Backend |
| `backend.assets.emit` | Basic | NestedEvidence | None | BuildOrDev | Metric(`build.backend.total`) | Evidence | Backend |
| `output.write.total` | Basic | NestedEvidence | None | BuildOrDev | Metric(`build.output.total`) | Evidence | BuildSystem |

The correction checkpoint intentionally removes `backend.html.total`. It also
records the stage/output parent and accounting rules in the descriptor table,
not in summary string lists.

### Deliberate compatibility reset

Timing data recorded before schema v1 is legacy and non-comparable. The schema
carries no numeric migration and no aliases for provisional names. Benchmark
reports must label a schema mismatch as non-comparable (see § 10).

---

# Corrected remaining phase order

The pre-correction sequence put dense collection ahead of the typed facade.
That would require a string-to-v1 mapper, dual collectors or a silent drop
path. None is acceptable. The following sequence is now authoritative; the
historical details after it remain only as implementation notes.

## Phase 2 - Runtime and session channels, retaining event storage

Keep the current raw event storage temporarily. Implement active channel
selection, fallible raw-session ownership, lock-free mode reads and inactive
clock avoidance without changing the recording identity from raw final-name
strings to a compatibility mapping.

- [x] Move timer mode parsing into `runtime.rs` with pure parsing tests.
- [x] Represent metrics, counters, attribution, detailed output, bench output
  and human summary as explicit session channels.
- [x] Make raw benchmark session start fallible and reject nested starts before
  compiler work.
- [x] Preserve generation-scoped finish/drop cleanup and reject stale context.
- [x] Avoid timer clocks and collector locks when the relevant channel is off.
- [x] Retain the existing event snapshot until Phase 5. Do not add a
  string-to-`TimingMetric` compatibility mapper.
- [x] Test active/inactive channels, nested/raw sessions, stale finishes and
  counter-only combinations.

Checkpoint only after runtime/session tests, the five-feature matrix, erasure
check and the full code-bearing validation gate pass.

## Handoff at the Phase 3 boundary

The accepted implementation baseline is `a9b970bb6`. Phase 2 deliberately
retains the raw string event vector, so Phase 3 must finish semantic migration
before Phase 4 makes recording typed and before Phase 5 replaces storage.

At this pause, the worktree contains an uncommitted candidate migration across
command, build-system, Stage 0, config, frontend, AST, backend, dev and summary
owners. It is not validated or audited and is not accepted progress. The next
coordinator must make its disposition explicit before editing:

1. discard it and start Phase 3 from the clean `a9b970bb6` baseline, or
2. retain it as a candidate, inspect the complete diff, compile it and accept
   only coherent, tested portions through the normal audit checkpoint.

Do not start Phase 4 or Phase 5 while raw names remain, and do not add a
string-to-typed compatibility mapper, a second collector or a dual facade.
After reconciliation, work through the Phase 3 checklist below in ownership
order: command lifecycle, build and Stage 0, config, frontend and AST, then
backend, output and dev. Finish with the focused migration regressions, the
required audit cycle and one Phase 3 checkpoint. The next checkpoint must
refresh this capsule with the exact command results and audit disposition.

## Phase 3 - Semantic call-site migration to v1 names and boundaries

While the current raw event collector still accepts strings, migrate every
production timer call site to its final v1 stable name and semantic endpoint.
This phase makes the later all-at-once typed-facade migration mechanical.

- [x] Commands: one command session finish point, command totals before
  rendering, and partial failure evidence.
- [x] Build/Stage 0: bootstrap, frontend, backend and output owner spans;
  nested directory/single-file evidence and accumulated boundary evidence.
- [x] Config: final `config.ast.*` identities, expression/block endpoints and
  no leaked later-stage duration.
- [x] Frontend: final module, AST, public-interface, HIR, borrow and generated
  identities with explicit module attribution.
- [x] Backend/dev: generic `build.backend.total`, nested HTML/JS/Wasm/assets,
  `BuildOrDev` evidence, nested output write and one dev build/write owner.
- [x] Delete live timers that do not exist in schema v1 rather than preserving
  provisional names.
- [x] Add build/check/dev success and failure, directory/single-file,
  config-heavy, generic-heavy and backend boundary tests.

Checkpoint only after every production name appears in the v1 registry and no
provisional production timing name remains.

## Phase 4 - Typed facade checkpoint

Replace every final raw string at call sites with `TimingMetric` in one
buildable change. Then delete string recording and guard APIs. There is no
dual facade or fallback mapper.

- [ ] Move enabled implementation to the final module layout, including
  `guard.rs`, and remove the old broad `enabled.rs` allowances.
- [ ] Make every expression macro, guard, multi-span and direct-record facade
  accept `TimingMetric`.
- [ ] Keep disabled macro arms as direct production expressions or no
  statements, without evaluating metric/context/command expressions.
- [ ] Move detailed timer prose out of `compiler_dev_logging` and ensure it
  uses the captured stored duration.
- [ ] Delete string recording APIs, raw parent names and old macro surfaces.
- [ ] Test no-feature erasure, value/error pass-through, exactly-once guards,
  multi-metric equality and inactive clock avoidance.

Checkpoint only after no production timer call can provide a raw metric name.

## Phase 5 - Dense aggregate collector

With all recording typed, replace the event vector with dense
`TimingMetric::index()` accumulators. Snapshot order follows
`TimingMetric::ALL`; dynamic boundary/module records receive dense attributed
slots only for allowed metric kinds.

- [ ] Store global and attributed totals atomically without record-path
  allocation, formatting, hashing or a global collector mutex.
- [ ] Stop recording before deterministic snapshot extraction.
- [ ] Build schema-order aggregates with sample counts where useful.
- [ ] Retain only typed metrics in snapshots and expose no raw compatibility
  parser.
- [ ] Test exact parallel additions, schema-order snapshots, attribution
  validation and no-allocation/no-lock inactive behavior.

Checkpoint only after collector/session tests, all feature combinations,
erasure and full validation pass.

## Phase 6 - Typed summary policy and rendering

Build the concise report from typed descriptors and one policy owner. Command
accounting consumes only `TimingAccountingRole::Pipeline` spans. Nested and
accumulated evidence never becomes additive command time.

- [ ] Construct display, parent, threshold and command-accounting policy from
  typed schema identities.
- [ ] Detect duplicate or over-accounted command spans rather than saturating
  `Other`.
- [ ] Render boundaries, frontend rows and slowest modules deterministically
  without absolute paths.
- [ ] Keep report construction pure and terminal styling in `render.rs` only.

## Phase 7 - Benchmark schema reset and erasure hardening

- [ ] Emit exactly one `MOTH_BENCH timing-schema 1` record and final aggregate
  metric lines in schema order.
- [ ] Include the timing schema in CLI and frontend benchmark identities and
  reject mismatches as non-comparable.
- [ ] Reset pre-v1 timing history without numeric migration.
- [ ] Extend source and binary erasure checks for the final typed facade.

## Phase 8 - Documentation, final audit and closeout

- [ ] Update timer, benchmark, feature and validation documentation from the
  final implementation only.
- [ ] Rebuild required documentation and inspect generated routes.
- [ ] Run final zero-cost, architecture and benchmark-schema audits.
- [ ] Record validation, accepted deferrals, final inventory and checkpoints.
- [ ] Leave the worktree clean and stop for coordinator acceptance.

---

# Historical pre-correction phase detail (superseded)

The sections below preserve the prior checklist wording for reference. They do
not define execution order. Follow the corrected phase order above.

## Historical Phase 2 - Rebuild session, runtime and aggregate collection

## Context

The committed session generation work is valuable. The runtime now needs explicit channels, lock-free mode reads and an aggregate-first collector.

## Checklist

### Runtime configuration

- [x] Move timer mode parsing into `runtime.rs`.
- [x] Use `OnceLock` or equivalent for lock-free production reads.
- [x] Add pure parsing functions.
- [x] Replace permanent test mutation with explicit session config or scoped restoration.
- [x] Add active channel bits.
- [x] `begin_metric` avoids `Instant` when the metric is inactive.

### Session lifecycle

- [x] Keep generation-scoped session IDs.
- [x] Keep matching finish/drop cleanup.
- [x] Make raw benchmark start fallible.
- [x] Reject nested raw sessions before compiler work.
- [x] Keep one active process session.
- [x] Store command kind explicitly.
- [x] Store collection channels explicitly.
- [x] Remove unused `TimingCollectionPurpose` because channels own behavior.

### Collector

- [ ] Replace event vectors with dense metric accumulators.
- [ ] Add sample counts only where useful.
- [ ] Use atomic aggregate updates.
- [ ] No global collector mutex on ordinary metric record.
- [ ] Stop recording before snapshot extraction.
- [ ] Build deterministic snapshots in schema order.
- [ ] Flush benchmark lines only after the snapshot exists.

### Counters

- [x] Support counter-only collection.
- [x] Preserve `benchmark_counters` independence.
- [x] Counter summary works with timer Summary, Bench and Silent modes.
- [x] Counter names remain static internally.

### Record outcome

- [x] Return structured outcome where prose needs it.
- [x] A dropped stale context never changes output suppression.
- [x] Invalid attribution is rejected without emitting a line.

## Tests

- [x] nested start cannot replace the outer session
- [x] raw nested start returns an error
- [x] rejected raw work does not enter an outer snapshot
- [x] stale finish cannot drain a new session
- [x] inactive mode captures no clock
- [x] inactive mode takes no collector lock
- [x] Bench mode has no attribution metadata
- [x] Silent plus counter Summary collects counters only
- [ ] schema-order snapshot is deterministic
- [ ] parallel metric additions produce exact totals
- [ ] poisoned lifecycle lock recovery is deliberate

## Audit

- [x] No thread-local state.
- [x] No unsafe code unless separately justified and approved.
- [ ] Record path allocates nothing.
- [ ] Record path formats nothing.
- [x] Lifecycle state has one owner.
- [x] Session generation reaches attribution validation.

## Style review

- [x] Use enums instead of boolean-heavy public APIs.
- [x] Keep atomics internal.
- [x] Keep lifecycle and aggregate storage separate.
- [ ] Remove stale event-log terminology.

## Validation

- [x] collector/session/runtime unit tests
- [x] five-feature check matrix
- [x] timers and detailed-timers test suites
- [x] counters feature combinations
- [x] `just timers-erasure-check`
- [x] `just validate`

## Checkpoint

- [ ] Commit the runtime before call-site migration.

---

## Historical Phase 3 - Finalise the compile-erasing facade and module layout

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

## Historical Phase 4 - Migrate command, build, Stage 0, config and output timing

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

## Historical Phase 5 - Migrate frontend, AST, generated work and borrow timing

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

## Historical Phase 6 - Migrate backend, assets, output children and dev timing

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

## Historical Phase 7 - Rebuild summary policy and renderer over schema v1

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

## Historical Phase 8 - Reset benchmark timing identity and strengthen erasure

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

## Historical Phase 9 - Documentation, final audits and closeout

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
- [x] Historical-plan policy: the two earlier timing-plan files were removed at
  `77a45e790708f76f8b96267991a0370a3ecfc9c8` and remain available through
  Git history; do not claim they are marked in place.
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
- the historical-plan policy accurately records that the older plan files were
  removed and remain available through Git history
- this plan is marked complete
