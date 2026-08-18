# Moth Benchmarks

## Purpose

The benchmark system provides a rough compiler-development sanity check. It answers whether a change obviously helped, hurt or did nothing measurable.

It doesn't enforce a pass/fail performance threshold and it doesn't replace correctness tests. Public summaries stay terse so the repository can track them without noise.

## Commands

```bash
just bench-ci
just bench-validate
just bench-check
just bench-frontend-check
just bench
just bench-frontend
```

`just bench-ci` runs the bounded development gate used by `just validate`. It preflights every manifest case once, then measures the quick subset with three iterations. The command never writes local history or tracked summaries.

`just bench-validate` preflights every case without measuring it. Use this when you only need to check the benchmark inventory and execution contracts.

`just bench-check` runs the full end-to-end CLI suite without writing local history or tracked summaries.

`just bench-frontend-check` runs the focused in-process frontend suite without writing local history or tracked summaries. Use it when compiler-stage changes are too small to read through subprocess noise.

`just bench` records an end-to-end CLI run, updates local raw history under `benchmarks/local-data/`, and updates the current monthly summary under `benchmarks/summaries/`.

`just bench-frontend` records the focused frontend suite through the same local history and monthly summary flow, but under a separate suite kind.

Every mode preflights its selected cases before measurement. That successful preflight provides the one warmup. Full check and recording modes then run ten measured iterations per case. `bench-ci` preflights all 60 cases before it selects 8 quick CLI cases and 10 quick frontend cases for three measured iterations.

Non-recording commands never append local JSONL history or change tracked summaries.

Recorded runs (`just bench` and `just bench-frontend`) require a clean committed worktree. The command rejects a dirty or uncommitted repository before fingerprint traversal, compiler construction or history access. Read-only commands (`bench-ci`, `bench-validate`, `bench-check` and `bench-frontend-check`) permit a dirty worktree as long as it stays unchanged during the run.

## Manifest And Stable Identity

`benchmarks/manifest.toml` owns the ordered workload and case inventory. It currently declares 33 workloads and 60 cases.

A workload names the source inputs that determine one compilation workload:

```toml
[[workload]]
id = "speed_test"
entry = "benchmarks/speed-test.moth"
fingerprint_mode = "full_tree"
fingerprint_roots = ["benchmarks/speed-test.moth"]
fingerprint_excludes = []
```

`id` gives the workload a stable authored identity. `entry` selects the file or project passed to the runner. `fingerprint_mode` selects how source identity is computed: `full_tree` uses one root equal to the entry, while `partitioned` uses disjoint strict-descendant roots under a directory entry. `fingerprint_roots` list every authored input boundary that affects compilation. `fingerprint_excludes` remove generated output trees inside those roots, such as `dev` and `release`.

Directory-entry workloads declare `fingerprint_mode = "full_tree"` with the entry as the sole root, or `fingerprint_mode = "partitioned"` with disjoint roots under the entry. Schema 3 validates that full-tree roots equal the entry, partitioned roots are strict descendants, and no root or exclude overlaps another.

Each case connects a workload to one typed runner:

```toml
[[case]]
id = "speed_test_build"
workload = "speed_test"
group = "core"
quick = true
expectation = "clean"

[case.runner]
kind = "cli"
command = "build"
args = []
```

Frontend cases use the in-process frontend API:

```toml
[[case]]
id = "type_stress_frontend"
workload = "type_stress"
group = "core"
quick = false
expectation = "clean"

[case.runner]
kind = "frontend"
profile = "dev"
```

Case IDs identify measurements and reports. CLI profiling also selects cases by this ID. They don't derive from paths, commands or compiler names, so moving a fixture doesn't rename its history. Workload IDs let CLI and frontend cases share the same source identity.

## Source and Measurement Identity

The source workload fingerprint covers the source fingerprint format version, manifest schema version, workload ID, entry logical path, entry kind, fingerprint mode, normalised root and exclude sets, and every included file path and byte. It does not hash runner declarations, `group` or `quick`. Changing source bytes invalidates every case attached to that workload.

The case measurement fingerprint covers the measurement fingerprint format version, benchmark protocol version, timing schema version, the source workload fingerprint, workload ID, this case's runner kind, command or frontend profile, authored runner arguments and expectation. It does not hash sibling cases. Changing one case's runner changes only that case's measurement fingerprint.

Comparison output distinguishes four states for each matching case ID:

- **workload changed**: source fingerprint differs — no speed delta is reported.
- **timing schema changed**: source matches but the timing schema differs — no speed delta or stage movement is reported. The report uses this state even when the numeric timing values happen to be equal.
- **measurement changed**: source and timing schema match but measurement fingerprint differs — no speed delta is reported.
- **timing comparable**: both match — speed deltas and stage movement are computed.

Schema 3 accepts only `expectation = "clean"`. A clean case must compile without errors or warnings. Negative diagnostic coverage belongs under `tests/cases/`, not in this manifest.

## Execution Contract

All benchmark modes use the same case executor. The executor validates the runner and observations during preflight and during every measured iteration. One failure aborts the suite before any history write.

CLI cases run the prebuilt timer-enabled compiler. Benchmark subprocesses receive:

```text
MOTH_TIMERS=bench
MOTH_COUNTERS=off
MOTH_BENCH_STATUS=1
```

A completed `check` or `build` emits exactly one machine status record:

```text
MOTH_BENCH status errors=<usize> warnings=<usize>
```

The executor requires exactly one timing schema record and the matching `command.check.total` or `command.build.total` aggregate:

```text
MOTH_BENCH timing-schema 2
MOTH_BENCH timing command.check.total=<ms>ms
```

The compiler emits one final aggregate line for each non-empty metric, in the
canonical order owned by the timing registry. Duplicate schema or status
records, duplicate live timing metrics, unsupported schema versions, malformed
timing records and non-finite values are invalid. The required command total
must exist. Measured iterations must expose the same timing metric set and
schema; missing or additional timing names across iterations fail the run. A
zero process status can't compensate for reported diagnostics, and a clean
status can't compensate for a non-zero process status.

Frontend cases call the production in-process API. The same executor checks typed error and warning facts, a positive finite total duration, the current timing schema version and a non-empty stable stage set.

## Output Isolation And Artifact Cleanup

File-entry CLI cases run from isolated temporary directories under `target/benchmark-work/` so compiler output never writes into the tracked checkout. Directory-entry cases run from the repository root because their output folders are project-owned and excluded from workload fingerprints.

Directory build workloads declare their generated output roots explicitly in the manifest, for example `generated_output_roots = ["dev", "release"]`. Roots are relative to the workload entry, must be strict descendants of it, must be absent when the run starts and must be covered by an explicit fingerprint exclude. They are cleanup authority only; xtask never parses `config.moth` to rediscover output settings.

Explicit finalisation removes only run-owned roots after measurement. Cleanup completes before repository verification and before any history or summary write. A root that already exists, a tracked root, a symlink replacement or a removal failure aborts persistence. `Drop` is best-effort emergency cleanup only and never defines success.

Repository state is captured before each run and verified after measurement. If the repository changed during a run through source edits, commit changes or unexpected file creation, history persistence is blocked. This prevents recording a baseline from a contaminated worktree.

Recorded CLI and frontend runs require an exactly clean committed worktree at start. Read-only modes permit dirty but unchanged worktrees: they compare and present without appending history. Dirty profile runs still write profile artifacts, but they never append comparable profile history.

## Timing And Counter Controls

Normal benchmark commands build the compiler with the concise `timers` feature.
End-to-end CLI benchmarks run subprocesses with `MOTH_TIMERS=bench` and
`MOTH_COUNTERS=off` so stdout contains one timing-schema header and the final
stable aggregate observations without per-event prose or counter floods.
Focused frontend benchmarks run in-process and read the same timing collector
directly.

Feature roles:

- `timers`: enables command, build-system, Stage 0, frontend, backend, and output
  timing collection. Timers-only builds default to a concise human summary.
- `detailed_timers`: implies `timers` and adds detailed AST substage timing evidence
  plus detailed AST substage timings. It does not enable counters by itself.
- `benchmark_counters`: enables high-volume local diagnostic counters. In a
  counter-only build, counter logging can emit direct `MOTH_BENCH counter`
  lines without timer collection; when combined with `timers`, counters also
  enter the shared timing session. Normal benchmark runs leave counter stdout
  off.

Environment controls:

```text
MOTH_TIMERS=summary   # concise human summary
MOTH_TIMERS=bench     # schema header plus final aggregate MOTH_BENCH lines
MOTH_TIMERS=verbose   # detailed substage evidence plus final aggregate timing lines
MOTH_TIMERS=off       # disable ordinary command timing and timing output

MOTH_COUNTERS=off     # default
MOTH_COUNTERS=summary # stable counter lines plus grouped summary
MOTH_COUNTERS=full    # stable counter lines plus full legacy counter dump
```

Counter lines are emitted when the compiler has `benchmark_counters` and
`MOTH_COUNTERS=summary` or `MOTH_COUNTERS=full`. With `timers`, those lines are
backed by the shared timing session; without `timers`, they are direct output
from counter logging call sites and no timer aggregate is collected. In-process
frontend collection uses an explicit caller-owned session configuration rather
than `MOTH_TIMERS=off`. Do not turn counters on for normal before/after
benchmark runs unless the active investigation specifically needs counter
evidence.

### Timer report model

The human `MOTH_TIMERS=summary` report is a short developer scan, not a fourth
benchmark system. It shows one command, one set of compilation boundaries, the
curated frontend and backend sections, and one slowest module. Detailed timers
keep detailed substage evidence; bench mode emits one `MOTH_BENCH timing-schema 2`
header followed by the non-empty final aggregate `MOTH_BENCH timing` lines.
Both benchmark output and the concise report consume the typed schema rather
than inferring architecture from metric strings.

`just bench-report` and the tracked monthly summaries print the timing schema
identity alongside their latest-run evidence. Current records show
`Timing schema: 2`; an obsolete but uniform record remains readable and is
labelled non-comparable, with no speed, stage or counter movement. A record
whose cases carry mixed schemas is explicitly omitted from aggregate report
sections rather than being collapsed into one version. Monthly-summary
rewrites preserve legacy entries without promoting them to a current-schema
claim.

Rows distinguish wall spans from accumulated work:

- command and pipeline rows are wall-clock spans
- boundary rows sum disjoint inventory and compile passes and are labelled
  accumulated work
- frontend rows sum repeated per-module observations and are labelled
  accumulated
- nested AST children are evidence inside the AST parent and are never added
  to top-level accounting

Compilation boundaries name source-backed packages (`@<prefix>`) and the main
project in deterministic registration order. The slowest-module row uses
registered logical module metadata, never absolute filesystem paths, and
defines module work as source preparation plus `frontend.module.semantic_total`.
Optional child rows appear only when the unrounded duration is at least 1ms and
5% of the parent. `Other` appears only when it is at least 1ms or 2% of the
command total.

The zero-cost rule is a hard contract: a compiler built without `timers`
performs no timer-system clock reads, allocations, formatting, environment
lookups, collector operations or context propagation. The erasure gate
(`just timers-erasure-check`) builds a no-timer release binary, scans the
schema-owned metric inventory plus timer-only environment and report markers,
and audits both source roots for direct timer implementation leakage.

The current timing schema is the compatibility boundary. The typed registry owns stable
names, semantic boundaries, wall/accumulated/nested meaning and command
accounting. A change to those meanings requires a schema bump and makes the
old and new observations non-comparable. Data recorded before v1 is legacy;
there is no numeric migration or compatibility alias for provisional names.
Human grouping may combine schema metrics for presentation without changing
their identities. The benchmark protocol and measurement fingerprints carry
the schema version so this reset cannot be hidden as a speed movement.

### Frontend parallelism matrix

For frontend scheduling and parallelism work, run the focused frontend suite with the default
thread count and the fixed Rayon thread counts used by the roadmap plan:

```bash
just bench-frontend-check
RAYON_NUM_THREADS=1 just bench-frontend-check
RAYON_NUM_THREADS=2 just bench-frontend-check
RAYON_NUM_THREADS=4 just bench-frontend-check
just bench-frontend-check
```

The `parallelism` frontend group contains tiny serial-threshold cases, many-file preparation
cases, markdown-heavy source-loading coverage, and multi-module directory projects. Use these with
stage timings and optional counters to tune scheduling policy without changing the suite's normal
warmup/measured iteration model.

The current frontend parallelism cases are:

- `tiny-one-file`, `tiny-two-files`, `tiny-seven-files`, and `tiny-eight-files` for serial,
  byte-threshold, and parallel strategy boundaries.
- `many-tiny-files` and `many-medium-files` for per-file versus chunked file preparation.
- `many-markdown-assets` for Stage 0 missing-source loading.
- `many-modules-one-file-each` and `few-modules-many-files-each` for module inventory and
  per-module frontend scheduling.

Use `just bench-frontend-check` for before/after validation because it does not write local
history or tracked summaries. The unset Rayon environment is the `default` thread identity. A
positive `RAYON_NUM_THREADS` value creates a distinct fixed thread identity and invalid, empty or
zero values are rejected. Reports compare only runs with the exact same identity and label that
identity explicitly.

Use `just bench-frontend` only when you intentionally want a recorded run. Default-thread runs
append raw local data under `benchmarks/local-data/` and may update the concise tracked monthly
summary. Recorded fixed-thread runs stay in local JSONL and never update tracked summaries. Raw
local data, expanded counter tables and profile artefacts stay untracked.

### Profiling commands

```bash
just profile                  # default terse filter across all cases
just profile <filter>         # named filter: terse, normal, deep, raw-index
just profile-case <case-id> [filter]     # profile one manifest case ID
just profile-symbolicated [filter]       # request Samply presymbolication
just profile-case-symbolicated <case-id> [filter] # request presymbolication for one case
just profile-build            # build the profiling binary (target/profiling/moth)
```

Run `just bench-report` first to identify which case and stage are worth profiling.

### Profile drift compatibility

Profile history uses its own protocol version (`PROFILE_PROTOCOL_VERSION`, currently 2) and format version (currently 4). Drift comparison selects a previous run only when system UUID, filter mode, sample rate, and profile protocol version all match. Case-level comparison uses source and measurement fingerprints — not command text — as the comparison authority.

When a case's source fingerprint changed since the previous run, drift reports "workload changed" and the case does not contribute to function, stage, or counter drift. When the timing schema changed, drift reports "timing schema changed" and applies the same exclusion, even if the old and new numeric observations are otherwise equal. When the remaining measurement fingerprint changed (e.g., runner or protocol changed), drift reports "measurement changed" with the same exclusion. Only cases with identical identity contribute to drift aggregates.

Legacy profile history (formats v1 through v3) remains readable but is never selected as a directly comparable previous run because it lacks current protocol version and identity.

Current profile history requires a mandatory measurement identity and a captured revision. Dirty profile runs write artifacts but skip history append, so they never become future drift baselines.

## Measurement Model

CLI wall-clock time is the public rough regression signal. It measures the built `moth` binary as a subprocess, so it includes command startup, project loading, frontend compilation, backend work where relevant, and output handling.

Compiler stage timings are attribution and debugging evidence. They help explain whether obvious movement likely came from command/bootstrap setup, Stage 0 project structure, path resolution, reachable-file discovery, file preparation, dependency sorting, AST, HIR, borrow validation, backend lowering, output writing, or another instrumented stage.

Stage observations are emitted after the completed timing session as one
`MOTH_BENCH timing-schema 2` header followed by stable
`MOTH_BENCH timing <metric>=<ms>ms` aggregate lines when the compiler is built
with `timers` and run with `MOTH_TIMERS=bench` or `MOTH_TIMERS=verbose`. Lines
follow the timing registry's canonical order and are emitted only for metrics
with samples. Human timer prose is developer output only; benchmark parsing
requires the schema header and the aggregate metric set.

Stage 0/bootstrap/path-resolution timings are first-class attribution metrics. A CLI benchmark whose wall time is much larger than the sum of relevant top-level command phases should be treated as an instrumentation gap, not as harmless subprocess noise.

Counter observations are local diagnostic evidence, not public benchmark
results. Stable counter metric names use snake_case or dotted subsystem names
and are emitted as `MOTH_BENCH counter <metric>=<value>` lines only when
counter output is explicitly requested. Counters are stored in local JSONL and
used by local report tooling. Raw counter tables must not be added to tracked
summaries.

The current `frontend.prepare` metric is the schema-owned combined parallel
file-preparation aggregate: per-file tokenization, header parsing, local
string-table work, and deterministic merge/remap into the module table.
Directory projects also record the same metric for incremental Stage 0
discovery: each per-file header preparation and the final retained
header-syntax aggregation are attributed to the owning module and boundary.
Older local records may still contain legacy `file_prepare_ms`,
`tokenize_ms`, or `headers_ms` observations; those schema-less values remain
non-comparable and do not contribute to current reports.

In-process frontend timings call production compiler paths directly and stop at the documented frontend/backend boundary after HIR and borrow validation. They are useful for compiler refactors, but they are still rough development signals rather than precise measurements.

`no measurable change` means no overlapping benchmark case exceeded the deliberately rough comparison threshold.

## Suite Kinds

`end_to_end_cli` is the normal CLI benchmark suite. Its primary metric is subprocess wall-clock time.

`frontend_phases` is the focused in-process frontend suite. Its primary metric is total frontend time, with stage timings used for attribution.

Local history records the suite kind and primary metric so CLI and frontend runs are never compared against each other.

## Case Groups

The typed manifest assigns every case a public summary group. Groups organise reports without changing compiler or runner behaviour.

Groups are public summary labels, not compiler architecture boundaries. The group set is closed and typed in xtask: manifest validation rejects an unknown group value, so a typo cannot silently create a new summary bucket.

- `core`: baseline check/build cases.
- `docs`: documentation project checking.
- `stress`: targeted template, type, fold, pattern, collection, and environment stress fixtures.
- `module`: module/import/dependency graph and import fanout coverage.
- `parallelism`: frontend scheduling threshold, source-loading, and module/file fanout coverage.
- `borrow`: valid borrow and exclusivity coverage.

## Protocol And Format Versions

- Benchmark manifest schema: 3
- Source workload fingerprint version: 3
- Timing observation schema version: 1
- Benchmark protocol version: 4
- Normal JSONL history format: 8
- Profile protocol version: 2
- Profile history format: 4
- Profile run-manifest format: 4

## Summary Interpretation

Monthly summaries show absolute average times for `all` cases and for each group. Group averages provide context without adding long per-case tables.

`Case spread latest` is spread across different benchmark cases. It is not timing uncertainty.

`**-18ms avg** with 5 faster and 0 slower` means an obvious improvement across shared cases.

`no measurable change` means no overlapping benchmark case exceeded the rough per-case threshold.

`mixed` means at least one case improved and at least one case regressed. Inspect local JSONL or rerun before drawing broad conclusions.

`case set changed` means cases were added or removed, so only shared cases are directly comparable.

`workload changed` means a stable case ID still exists but its workload fingerprint differs. The report excludes that case from speed claims and compares any unchanged cases separately.

## Optimization Phase Protocol

For compiler optimisation phases, run both focused frontend and end-to-end suites five independent
times and compare the benchmark-system medians. Keep the suite's normal warmup/measured iteration
model. Repeat the whole recorded command rather than changing per-case iteration counts.

Use `just bench-report` and targeted `just profile-case <case-id>` runs for attribution. Record
only concise conclusions in `benchmarks/frontend-optimization-results.md` and the tracked monthly
summary. Raw benchmark history, raw profiles, and expanded counter tables stay local-only.

## Stage Movement Interpretation

`Stage movement: ast +22ms` suggests the change likely affected AST construction, but the benchmark is still rough. Confirm with frontend benchmarks or targeted profiling if the change matters.

Only the top meaningful stage movers are shown. Full per-case stage data stays local-only.

Stage movement should explain a benchmark result, not replace it. Treat it as a clue for where to investigate.

## Raw Local History

Detailed run data is local-only in `benchmarks/local-data/runs.jsonl`. Do not commit raw local history.

Raw records include per-case means, medians, standard deviations, timing schema
version, stage timings, counters, suite kind, primary metric name, exact thread
identity, system identity and commit metadata when available. Counters include
work-volume counters and implementation-pressure counters.

The tracked Markdown summaries under `benchmarks/summaries/` are the public record. They must stay concise.

## Local Drilldown Reports

`just bench-report` reads local JSONL only. It does not update tracked summaries or append local history.

Use it for compact per-case, stage, counter, ratio, and unattributed wall-time
detail during active optimisation work. The unattributed wall-time section
compares CLI wall time with the sum of the schema-owned, non-nested pipeline
rows: `build.bootstrap.total`, `build.frontend.total`,
`build.backend.total` and `build.output.total` where the selected command
applies them. It flags cases whose visible pipeline evidence no longer
explains the command cost; the `command.check.total` or `command.build.total`
row remains the headline command total, not an additional pipeline component.

## Local Profiling

Use `just bench-report` to choose a case and stage before profiling. Then run
`just profile` or `just profile-case <case-id>` to collect Samply-backed stack
samples alongside detailed timing observations.

### Two-run model

Each profiling case runs twice:

1. **Observation pass** runs without profiling and collects detailed stage timings.
2. **Samply pass** records stack samples into a raw profile.

The observation pass provides reliable stage attribution without profiler
overhead. Counter fields may still appear in older local records or explicit
counter-enabled investigations, but the normal profiling path is timing-first.
The Samply pass provides call-stack evidence.

### Profiling binary

The profiling binary is built to `target/profiling/moth` using
`just profile-build`. It uses release settings with full debug info and
`detailed_timers` for verbose timing evidence. `detailed_timers` no longer
enables high-volume counters by itself. Profile runs prepare symbol directories
for the profiling binary where available. On macOS the xtask path also tries to
materialize `target/profiling/moth.dSYM` with `dsymutil` and reports whether its
UUID matches the binary when `dwarfdump` is available. Do not commit the binary
or `.dSYM` bundle.

`--presymbolicate` remains an explicit profiling option. Use `just profile-symbolicated` or `just profile-case-symbolicated <case-id>` when a normal profile reports raw-address function names. xtask maps that request to the Samply flag supported by the installed CLI (`--presymbolicate` or `--unstable-presymbolicate`) and warns when neither flag is available.

### Filter modes

Filter modes control how much detail appears in summaries:

| Mode | Purpose | Keeps |
|---|---|---|
| `terse` | agent-first default | top 8 Moth-owned functions per case, top 3 cases in root summary |
| `normal` | human + agent investigation | top 20 functions per case, top 8 cases in root summary |
| `deep` | pre-refactor investigation | top 50 functions per case, all profiled cases, caller/callee context |
| `raw-index` | artefact generation only | raw profile and observation logs, no parsed hotspots |

`terse` is the default when no filter is specified.

### Output layout

```text
benchmarks/local-data/
├── profile-runs.jsonl              # derived local history (not raw profiles)
└── profiles/
    └── <run-id>/
        ├── agent-summary.md        # start here
        ├── profile-drift.md        # drift report when comparable history exists
        ├── profile-hotspots.json   # aggregated hotspot metadata
        └── cases/
            └── <case-id>/
                ├── summary.md
                ├── detailed-observations.json
                ├── profile-shape.txt      # written when symbolication fails
                └── profile.json.gz
```

Profile summaries include symbolication health. If most hot function names are raw `0x...` addresses, the summary marks symbolication as failed and function hotspots should not be treated as actionable. A failed-symbolication case also writes `profile-shape.txt`, which records the profile table shape, first function names, libraries, and native-symbol metadata for parser/debug-info investigation. Stage timings, plus any present counters, from the observation pass are still useful in that state.

`profile-hotspots.json`, the root `agent-summary.md`, and each per-case
`summary.md` identify the current timing schema alongside their schema-owned stage
observations. Profile report generation rejects obsolete or mixed-schema case
data; profile history remains the authority for cross-run comparability and
drift exclusion.

### Drift thresholds

When comparable profiling history exists, drift reports flag significant changes:

- **Function drift**: at least 300 samples, at least 1.0% inclusive share, at least 2.0 percentage-point delta, and at least 20ms estimated delta.
- **Stage drift**: at least 5% change and at least 10ms absolute delta.
- **Counter drift**: at least 3% change with a meaningful absolute delta.

Drift is attribution evidence. It does not prove an optimisation or regression.

### Rules

- Do not commit raw profiles, `profile-runs.jsonl`, or anything under `benchmarks/local-data/`.
- Profile evidence is attribution, not proof. Use benchmarks to validate or reject changes.
- Public summary rules under `benchmarks/summaries/` are unchanged by profiling.

## Adding Cases

Add cases through `benchmarks/manifest.toml`:

1. Choose an existing workload when the new runner measures the same source inputs. Add a workload only when the entry or authored input boundary differs.
2. Give the workload and case descriptive lowercase IDs with underscores. Treat both IDs as persistent history keys.
3. List every source or config input under `fingerprint_roots`. Exclude only generated paths inside those roots. Never exclude authored assets.
4. Choose the typed CLI `check` or `build` runner, or the frontend `dev` profile. Keep runner arguments explicit and ordered.
5. Set `expectation = "clean"`. Add the case to the quick subset only when it gives useful bounded coverage for normal development validation.
6. Run `just bench-validate`, then `just bench-ci`. Run the matching full non-recording suite when the case affects performance work.

New fixtures must compile successfully and exercise a distinct compiler or build-system path. Prefer one representative fixture over near-duplicates. If a fixture exposes a compiler bug, fix the compiler and add canonical coverage under `tests/cases/`. Don't weaken, annotate or reshape the benchmark source to hide the failure.

Negative diagnostic cases belong under `tests/cases/`, where their stable codes and source context can be asserted. Project fixtures commit source inputs only. Generated `dev` and `release` directories stay untracked.

Keep the public group list short. Reuse an existing group unless a new group makes summaries clearer.

Adversarial fixtures under `benchmarks/adversarial/` are compiler churn discovery workloads, not
public language examples. They should remain valid successful programs or projects, but they may
combine many surfaces in ways that are intentionally dense so profiling can expose frontend
allocation, lookup, folding, import, and lowering pressure.

## Fixture List

- `speed-test.moth`: broad baseline language and compiler exercise covering constant folding, templates, structs, receivers, collections, and control flow.
- `benchmark-root-single-file.moth`: root-level single-file check case that
  exercises the non-project single-file path.
- `template-stress.moth`: deeply nested template composition, slot usage, `$children` wrappers, and formatter directive stress.
- `type-stress.moth`: type and method-heavy source with structs, choices, aliases, receivers, and constructor patterns.
- `fold-stress.moth`: constant folding coverage with large arithmetic trees, chained dependencies, and const record creation.
- `pattern-stress.moth`: pattern and match coverage including exhaustive choice arms, guards, payload capture, and relational patterns.
- `collection-stress.moth`: collection operations and loop coverage with mutations, range loops, nested iteration, and fallible fallback patterns.
- `environment-stress.moth`: AST environment building, type alias expansion, nominal structs and choices, receiver catalog construction, generic declarations and instantiations, and body validation/type resolution.
- `module-graph/`: small multi-file project with cross-file imports, constants and templates.
- `import-fanout/`: multi-file project with repeated imports, aliases, wrapper declarations and cross-file constants for string-table interning and module-graph resolution.
- `module-root-stress/`: directory project with config parsing, multiple
  reachable module directories, and irrelevant non-Moth trees for Stage 0
  module-root/path-resolution attribution.
- `module-root-role-mix/`: directory project combining many skipped source directories,
  output-producing and API-only module roots and a source-backed package whose cosmetic root name is
  not `@mod.moth`.
- `external-js-imports/`: HTML project with annotated JavaScript imports, runtime helper imports, opaque external types, namespace imports, and external free functions.
- `borrow-stress.moth`: valid mutable/exclusive access and borrow-validation coverage.
- `adversarial/one-module-kitchen-sink.moth`: dense single-module churn across imports, constants,
  aliases, nominal types, choices, traits, generics, templates, collections, maps, receivers, and
  external package calls.
- `adversarial/deep-scope-churn.moth`: nested functions, control blocks, loop scopes, and local
  declaration pressure for scope-frame creation and ancestor lookup.
- `adversarial/template-render-plan-churn.moth`: nested template composition, slots, inserts,
  `$children` wrappers, repeated slot replay, and runtime template rebuilding.
- `adversarial/constant-dag-churn.moth`: large compile-time constant dependency DAGs, arithmetic
  folding, const records, and folded templates.
- `adversarial/expression-rpn-churn.moth`: expression parsing and RPN lowering pressure through
  choice matching, mutable stacks, checked operators, and value recovery.
- `adversarial/generic-trait-churn.moth`: generic structs/functions, trait declarations, explicit
  conformances, bound-provided receiver calls, and concrete instantiations.
- `adversarial/collection-map-borrow-churn.moth`: valid collection/map mutation, fallible
  operations, mutable receiver calls, and borrow-checker side-table pressure.
- `adversarial/import-external-churn/`: HTML project fixture with import fanout, cross-file
  constants/types/helpers, core package calls, and repeated external JavaScript free-function
  usage.

## What Not To Do

- Do not treat small timing changes as precise performance measurements.
- Do not add per-case tables to tracked summaries.
- Do not add raw counter dumps to tracked summaries.
- Do not add expensive counters that require new full-pipeline traversals without a targeted investigation.
- Do not treat counter movement as an optimisation result unless timing moved meaningfully too.
- Do not compare CLI and frontend suite results manually as if they were the same metric.
- Do not commit `benchmarks/local-data/`, generated benchmark outputs, or old benchmark result folders.
- Do not add failing diagnostic cases to benchmark suites.
- Do not weaken a valid benchmark fixture to avoid fixing a compiler regression.
- Do not derive new case IDs from paths or commands.
- Do not compare changed workload fingerprints as speed movement.
- Do not add many fixtures that stress the same path in slightly different ways.
