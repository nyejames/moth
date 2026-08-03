# Benchmark system final hardening and consolidation plan

## Purpose

Finish the benchmark system with three correctness hardenings and one bounded consolidation pass:

1. Recorded CLI and frontend benchmark runs require a clean committed worktree.
2. Profile identities are captured against one repository snapshot and are mandatory in current-format artifacts and history.
3. Generated benchmark outputs use an explicit, fallible lifecycle that completes before repository verification or persistence.
4. Shared CLI/frontend suite logic is consolidated so measurement, comparison, history and summary behaviour have one implementation.

Preserve the accepted typed manifest, shared case executor, workload fingerprints, fail-closed timing/status protocol, bounded `bench-ci`, output isolation and production build/output architecture.

This is an xtask hardening plan. It does not redesign compiler semantics, Stage 0, module topology or production output ownership.

## Current state

```text
WORK_ID: benchmark-system-final-hardening
WORK_SOURCE: docs/roadmap/plans/benchmark-system-final-hardening-plan.md
BASE_REVISION: 6750c9a57238203ba83b2a31d74d6a19ecf36d70
BRANCH: codex/benchmark-system-final-hardening (worktree /Users/aneirinjames/projects/beanstalk/moth-benchmark-final-hardening)
STATUS: active
ACCEPTED: 32 workloads, 58 typed cases, shared preflight/execution, clean-only benchmark expectations, protocol-aware normal/profile history, isolated file-entry builds, bounded bench-ci, production output-plan integration
COMPLETED: Phase 1 clean committed recording; Phase 2 centralised run preparation and identity; Phase 3 profile hardening (current identity and revision non-optional, serde writers for run-manifest/detailed-observations/hotspots/history with no `{}` fallbacks, finite-value rejection before writing, history format 4 and run-manifest format 4, PROFILE_PROTOCOL_VERSION 2, explicit legacy v1-v3 adapters via StoredProfileHistoryRecord, dirty-profile artifacts-without-history policy with exact message, workflow moved to profile/run.rs with structural mod.rs, json.rs deleted)
OPEN_CORRECTIONS: infallible Drop-only directory cleanup, duplicate CLI/frontend orchestration, stringly benchmark groups, stale documentation
NEXT_ACTION: implement Phase 4 (explicit fallible benchmark output finalisation), then run the Phase 4 audit and checkpoint commit
VALIDATION: Phase 0 baseline green; Phase 1 green (551 xtask tests, bench-validate, full just validate); Phase 2 green (558 xtask tests, bench-validate, bench-ci, full just validate); Phase 3 green: fmt check, 564 xtask tests (256 profile, 15 repository), bench-validate 58/58, just profile-build, full just validate (clippy native/linux/windows, 3945 workspace + 564 xtask tests, 1816 integration cases, docs check, bench-ci)
AUDITS: Phase 1 and Phase 2 reviewed by Coordinator against plan checklists with no open findings (launcher auditor route cannot return JSON handoffs in this environment); Phase 3 reviewed by Coordinator against plan Phase 3 checklist with no open findings
NOTES: Samply raw-index profile validation omitted with exact environmental evidence: `samply record` fails for any process here with `Encountered an error during profiling: Unknown(1100)` (verified on /bin/echo and the profile case); profile-build succeeds and the workflow fails closed with artifacts left for diagnosis and no history append
```

Keep this capsule concise as work advances. Git history is the implementation record.

## Required authorities

Read before implementation and again before final review:

- `AGENTS.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `benchmarks/README.md`
- `benchmarks/manifest.toml`
- `docs/roadmap/plans/benchmark-correctness-follow-up-implementation-plan.md`

Important ownership rules:

- each benchmark fact has one owner
- later phases do not reconstruct identities, commands or output roots
- build-system output policy remains production-owned
- benchmark tooling may consume production APIs but must not copy config parsing or output semantics
- tests stay outside production implementation files
- current APIs replace old APIs directly without compatibility wrappers

## Accepted foundation

Do not rewrite these systems:

- `benchmarks/manifest.toml` is the sole workload and case inventory
- authored case and workload IDs remain stable
- CLI `check` and `build` exit non-zero on diagnosed failure
- `MOTH_BENCH status` and timing records fail closed
- every benchmark mode uses the shared case executor
- one preflight succeeds before measured iterations
- file-entry CLI cases run below `target/benchmark-work/`
- source and measurement fingerprints remain separate
- changed workloads and changed measurements remain incomparable
- normal history uses current serde-backed JSONL records and explicit legacy adapters
- `bench-ci` preflights all cases and measures only the quick subset
- benchmark fixtures remain successful programs, not diagnostic fixtures
- production build outputs remain owned by the build-system output subsystem

## Scope

This plan owns:

- recording eligibility
- repository snapshot and fingerprint ordering
- current profile identity and serialization
- benchmark-generated output cleanup
- shared benchmark suite orchestration
- typed benchmark groups
- benchmark/profile history selection
- benchmark documentation and post-hardening baselines

This plan does not own:

- source-language behaviour
- compiler pipeline semantics
- canonical module compilation
- project config schema redesign
- production output manifest semantics
- dev-server output policy
- general filesystem transaction support
- package manager or dependency work

If another plan changes the same xtask or build-system boundaries before implementation, rebase and review rather than preserving parallel APIs.

## Non-negotiable behaviour

### Recorded normal benchmarks

`just bench` and `just bench-frontend` require:

- a valid `HEAD`
- no tracked diff
- no untracked non-ignored files
- no repository change during setup, measurement, cleanup or pre-persistence verification

Fail before fingerprinting or compiler construction when the worktree starts dirty.

### Read-only benchmarks

These may run from a dirty worktree when it stays byte-for-byte unchanged:

- `just bench-check`
- `just bench-frontend-check`
- `just bench-ci`
- `just bench-validate`
- `just bench-report`

Read-only commands never write normal history or tracked summaries.

### Profiling

A dirty worktree may produce local profile artifacts for active investigation.

A dirty profile run:

- records its dirty state in local run artifacts
- may compare against the latest clean comparable profile record
- must not append a current history record used by later drift comparisons
- prints a clear message that history persistence was skipped

A clean profile run may append current-format history after cleanup and repository verification.

### Generated outputs

- file-entry outputs remain isolated below `target/benchmark-work/`
- directory build outputs have explicit manifest-declared cleanup authority
- cleanup is fallible and completes before repository verification or persistence
- pre-existing output roots are never silently deleted
- check and frontend cases do not register build-output cleanup
- `Drop` is emergency cleanup only, never the success authority

### Consolidation

- do not add a generic trait framework
- do not add a second command resolver, fingerprint builder, comparison engine or persistence path
- do not create broad utility modules
- exact shared operations move to one owner
- production line count across the duplicated orchestration files must decrease
- if a consolidation adds more production code than it removes, stop and justify the design before proceeding

## Target module shape

Use the existing xtask modules where they remain good owners. Add at most these focused owners:

```text
xtask/src/
├── benchmark_run.rs          # manifest -> snapshot -> fingerprints and recording eligibility
├── benchmark_suite.rs        # shared case measurement, presentation and normal persistence flow
└── profile/
    ├── mod.rs                # structural map and re-exports only
    └── run.rs                # profile orchestration
```

Exact names may vary.

Do not create one file per tiny helper. Prefer one cohesive shared suite module over a framework of traits and adapters.

Expected end state:

- `bench.rs`: thin CLI-suite entry wrapper
- `frontend_bench.rs`: thin frontend-suite entry wrapper plus the compiler-facing frontend adapter
- `bench_ci.rs`: bounded selection/orchestration using shared measurement and presentation
- `bench_validate.rs`: thin full-preflight command
- `profile/mod.rs`: module map
- `profile/run.rs`: profile workflow
- `profile/history.rs`: current serde format plus explicit legacy readers
- `benchmark_workspace.rs`: explicit output lifecycle

Aim for `bench.rs` and `frontend_bench.rs` to contain only suite-specific setup and adaptation, not duplicated statistics, history or summary logic.

## Phase 0 - Refresh, inventory and baseline

Before editing:

1. Record revision, branch and `git status --porcelain=v1`.
2. Confirm the current plan is complete and off the active roadmap.
3. Inventory all uses of:
   - `BenchmarkRepositorySnapshot`
   - `BenchmarkRunPolicy`
   - `run_preflighted_suite`
   - `run_benchmark_cases`
   - `run_frontend_cases`
   - `build_case_result`
   - `build_frontend_case_result`
   - `present_benchmark_run`
   - `present_frontend_run`
   - `find_latest_matching_run`
   - `update_monthly_summary`
   - `build_case_identity`
   - `ProfileCaseManifest`
   - `ProfileHistoryRecord`
   - `BenchmarkExecutionWorkspace`
   - `register_directory_artifacts`
   - `COMPILER_OUTPUT_DIRS`
   - string-valued benchmark groups
4. Record line counts for:
   - `xtask/src/bench.rs`
   - `xtask/src/frontend_bench.rs`
   - `xtask/src/bench_ci.rs`
   - `xtask/src/bench_validate.rs`
   - `xtask/src/profile/mod.rs`
   - `xtask/src/profile/history.rs`
   - `xtask/src/profile/artifacts.rs`
5. Run:

```bash
cargo fmt --all -- --check
cargo test --package xtask --quiet -- --format terse
just bench-validate
just bench-ci
just validate
```

Record exact results.

### Phase 0 stop conditions

Stop when:

- current main no longer matches the reviewed architecture
- any accepted benchmark command is red
- another branch has changed benchmark history or output lifecycle
- the local history format differs from the reviewed current format
- a required legacy profile format cannot be identified from code and tests

Do not edit expectations to create a green baseline.

## Phase 1 - Enforce clean committed recording

### 1A - Add one repository eligibility authority

Extend `BenchmarkRepositorySnapshot` with narrow queries equivalent to:

```rust
pub(crate) fn is_clean_committed(&self) -> bool;
pub(crate) fn require_clean_committed(&self) -> Result<(), BenchmarkRepositoryError>;
```

A clean committed state requires:

- captured `HEAD`
- empty tracked diff
- no untracked non-ignored files

Keep unchanged-dirty verification for read-only commands.

Do not add a second Git-status implementation.

### 1B - Reject dirty normal recording before expensive work

For `bench` and `bench-frontend`, use this order:

```text
load manifest
-> capture repository snapshot
-> require clean committed state
-> compute fingerprints
-> build compiler where needed
-> preflight
-> measure
```

The rejection must happen before:

- fingerprint traversal
- release compiler construction
- system identity creation
- local history reads or writes
- tracked summary reads or writes

Read-only modes skip only the clean-start requirement. They retain final unchanged verification.

### 1C - Filter comparison and presentation inputs

Current normal history may contain dirty records from earlier tooling. Exclude records whose state is not exactly clean and committed from:

- `find_latest_matching_run`
- monthly summary initial/latest selection
- `bench-report` latest-run selection
- any shared previous-run loader introduced later

Defense in depth:

- current normal history append rejects a dirty record
- tracked summary update rejects a dirty run even if a caller is wrong
- fixed-thread recording keeps its existing local-only summary policy

Do not delete old local records in this phase. Make them non-comparable.

### 1D - Tests

Add focused tests proving:

- clean committed recording eligibility passes
- tracked dirty recording eligibility fails
- untracked dirty recording eligibility fails
- read-only unchanged-dirty verification still passes
- a dirty record is ignored by latest-comparable selection
- a dirty record cannot enter monthly summary selection
- a dirty record cannot become the latest `bench-report` baseline
- `bench` and `bench-frontend` reject before invoking compiler construction

Use function boundaries or small injected callbacks only where an existing orchestration seam already supports them. Do not add a general mock framework.

### Phase 1 validation

```bash
cargo fmt --all -- --check
cargo test --package xtask --quiet benchmark_repository -- --format terse
cargo test --package xtask --quiet bench_history -- --format terse
cargo test --package xtask --quiet bench_summary -- --format terse
cargo test --package xtask --quiet bench_report -- --format terse
just bench-validate
just validate
```

### Phase 1 review checkpoint

Stop for review. Confirm:

- recording eligibility exists once
- read-only dirty behaviour remains unchanged
- every normal comparison path ignores dirty history
- no compiler build starts before recording eligibility passes
- no new Git command wrapper exists

Suggested commit:

```text
bench: require clean committed recording
```

## Phase 2 - Centralise run preparation and measurement identity

### 2A - Add one prepared-run owner

Create one small owner that performs this exact sequence:

```text
load typed manifest
-> capture repository snapshot
-> compute workload and case fingerprints
```

Conceptual shape:

```rust
pub(crate) struct PreparedBenchmarkRun {
    pub(crate) manifest: BenchmarkManifest,
    pub(crate) snapshot: BenchmarkRepositorySnapshot,
    pub(crate) fingerprints: BenchmarkFingerprints,
}
```

It may expose:

```rust
pub(crate) fn load() -> Result<Self, String>;
pub(crate) fn require_recording_eligible(&self) -> Result<(), String>;
```

All benchmark, validation and profile modes use it.

Do not put compiler construction, case selection, system identity or persistence into this type.

### 2B - Make case identity one checked lookup

Move measurement identity construction to the fingerprint owner or another single narrow helper:

```rust
pub(crate) fn identity_for(
    &self,
    manifest: &BenchmarkManifest,
    case: &BenchmarkCase,
) -> Result<BenchmarkMeasurementIdentity, BenchmarkIdentityError>;
```

It must fail when:

- the workload relationship is invalid
- the workload fingerprint is missing
- the case fingerprint is missing

Replace identity reconstruction in:

- CLI result construction
- frontend result construction
- profile run manifests
- profile drift inputs
- profile history records

Pass `&BenchmarkCase` directly. Remove repeated linear searches through `selected_cases` by case ID.

### 2C - Correct profile setup ordering

Profile orchestration becomes:

```text
load manifest and selected case IDs
-> capture snapshot
-> compute fingerprints
-> inspect Samply
-> build profiling compiler
-> preflight
-> collect
```

The prepared-run helper may load the full manifest before profile selection, or profile selection may happen immediately after manifest load. The snapshot must precede fingerprint traversal.

### 2D - Tests

Add tests for:

- complete current identity construction
- missing workload index fails
- missing source fingerprint fails
- missing measurement fingerprint fails
- CLI, frontend and profile paths receive identical identity for the same case
- prepared-run final verification detects a source mutation after preparation

The preparation function should remain short enough that ordering is obvious by inspection. Do not add test-only lifecycle hooks solely to assert call order.

### Phase 2 validation

```bash
cargo fmt --all -- --check
cargo test --package xtask --quiet benchmark_fingerprint -- --format terse
cargo test --package xtask --quiet benchmark_run -- --format terse
cargo test --package xtask --quiet profile -- --format terse
just bench-validate
just bench-ci
just validate
```

### Phase 2 review checkpoint

Confirm:

- every mode captures one snapshot before fingerprinting
- every current case identity comes from one checked helper
- no selected-case linear search remains for identity
- no optional current identity remains outside legacy adapters

Suggested commit:

```text
bench: centralise run preparation and identity
```

## Phase 3 - Harden and simplify profile artifacts and history

### 3A - Make current profile identity mandatory

For current formats:

- `ProfileCaseManifest.identity` is `BenchmarkMeasurementIdentity`
- `HistoryCaseRecord.identity` is `BenchmarkMeasurementIdentity`
- current profile history has a captured `GitRevision`
- clean persisted profile history requires a known commit and `dirty == Some(false)`

Optional identity and missing revision exist only in explicit legacy input structs or enum variants.

### 3B - Use serde for current machine-readable profile data

Convert current writers to serde-backed structs:

- `run-manifest.json`
- `detailed-observations.json`
- `hotspots.json`
- `profile-runs.jsonl`

Every serializer returns `Result`.

Delete fallbacks such as:

```rust
unwrap_or_else(|_| "{}".to_string())
```

Reject non-finite or otherwise unserialisable data before writing.

Delete `profile/json.rs` when no current writer needs manual string escaping.

Do not create a generic serialization framework. Derive `Serialize` and `Deserialize` on the current records and use explicit conversion helpers.

### 3C - Preserve legacy history without weakening the current domain

Bump:

- profile history format to 4
- profile run-manifest format to 4
- `PROFILE_PROTOCOL_VERSION` to 2

Keep older supported formats readable through explicit legacy structs and adapters.

Preferred shape:

```rust
enum StoredProfileHistoryRecord {
    Current(ProfileHistoryRecord),
    Legacy(LegacyProfileHistoryRecord),
}
```

or another equally explicit form.

Rules:

- only current clean protocol-2 records are comparable
- legacy records may be listed or counted but never become direct drift baselines
- do not guess an undocumented legacy field
- malformed current records fail their line with a warning
- current null identity is invalid

### 3D - Dirty profile policy

After profile artifacts are written and workspace cleanup succeeds:

1. verify the repository is unchanged
2. when the start snapshot was clean, append current history
3. when the start snapshot was dirty, skip history append and print:

```text
Profile artifacts were written, but comparable history was not recorded because the worktree was dirty at run start.
```

Dirty runs may still compare against the latest clean compatible previous record.

`find_comparable_previous` must reject dirty, unknown-revision and legacy records.

### 3E - Reduce profile orchestration size

Move the main workflow from `profile/mod.rs` to `profile/run.rs`.

`profile/mod.rs` becomes the module map and narrow re-export surface.

While moving:

- remove numbered phase comments
- remove repeated identity construction
- replace manual JSON assembly with typed writers
- keep Markdown generation in its existing owner
- do not reorganise hotspot parsing or Samply internals without a direct need

### Phase 3 tests

Add tests for:

- current run manifest requires identity
- current history requires identity
- current history rejects dirty revision
- dirty profile writes artifacts but no history
- clean profile appends history
- previous-profile selection uses only clean current protocol records
- legacy records parse but are incomparable
- JSON escaping, optional paths and Unicode round-trip through serde
- non-finite data fails serialization
- raw-index manifests contain no invalid summary reference
- profile `mod.rs` contains no workflow implementation

### Phase 3 validation

```bash
cargo fmt --all -- --check
cargo test --package xtask --quiet profile -- --format terse
cargo test --package xtask --quiet benchmark_repository -- --format terse
just profile-build
just bench-validate
just validate
```

Run one raw-index profile when Samply is installed:

```bash
just profile-case speed_test_build raw-index
```

When Samply is unavailable, record that exact omission.

### Phase 3 review checkpoint

Confirm:

- current profile identities are non-optional
- snapshot precedes fingerprinting
- dirty runs cannot become future profile baselines
- current JSON uses serde
- manual JSON fallback code is gone
- profile production line count decreased

Suggested commit:

```text
profile: require clean typed history
```

## Phase 4 - Add explicit, fallible benchmark output finalisation

### 4A - Give directory build workloads cleanup authority

Bump benchmark manifest schema to 3.

Add an explicit field to directory workloads that have CLI `build` cases:

```toml
generated_output_roots = ["dev", "release"]
```

These roots are relative to the workload entry, not the repository.

Validation rules:

- file workloads declare no generated roots
- a directory workload with a CLI build case declares at least one root
- roots use the manifest's portable relative path rules
- roots are strict descendants of the workload entry
- roots cannot overlap, duplicate or differ only by deterministic ASCII case
- roots cannot be symlinks when already present
- each root is also covered by an explicit fingerprint exclude
- roots are cleanup authority only, not source or config semantics

Do not parse `config.moth` again in xtask. Do not reuse `fingerprint_excludes` as deletion authority.

### 4B - Reject pre-existing generated roots

Before the first execution of a directory `build` case:

- resolve its declared roots
- require each root to be absent
- reject tracked or pre-existing untracked roots
- register the roots with the run workspace

This avoids deleting user data and keeps repeated preflight/measured iterations inside a controlled run-owned output tree.

`check` and frontend cases register nothing.

### 4C - Replace Drop success with `finish()`

Add:

```rust
pub(crate) fn finish(&mut self) -> Result<(), BenchmarkWorkspaceError>;
```

`finish()`:

- removes only registered run-owned roots
- verifies every root remains within its workload entry
- rejects symlink replacement or path escape
- reports removal failure
- verifies the roots are absent afterwards
- is idempotent

Keep `Drop` as best-effort emergency cleanup. It may log in tests or debug builds but cannot define success.

### 4D - Detect undeclared build output

After successful directory build execution, and again at finalisation, ensure no new `.moth_manifest` exists under the workload entry outside a declared generated root.

Do one bounded scan per affected workload at finalisation, not once per measured iteration.

This catches config/output-root drift without adding a second config parser.

### 4E - Put cleanup before verification and persistence

Every suite and profile operation follows:

```text
preflight/measurement operation completes
-> workspace.finish()
-> repository snapshot verification
-> presentation and persistence
```

When operation and cleanup both fail, report both.

When cleanup fails:

- no normal history append
- no tracked summary update
- no profile history append
- local profile artifacts may remain for diagnosis, but report the incomplete cleanup

Remove `register_directory_artifacts` and `COMPILER_OUTPUT_DIRS`.

### Phase 4 tests

Add tests for:

- declared custom output root cleanup
- existing output root rejected before execution
- check case registers no root
- frontend case registers no root
- build case registers declared roots
- undeclared `.moth_manifest` fails finalisation
- symlink-replaced root fails finalisation
- removal failure blocks persistence
- tracked roots are never deleted
- `finish()` is idempotent
- `Drop` is not required for a successful test
- operation failure plus cleanup failure reports both

### Phase 4 validation

```bash
cargo fmt --all -- --check
cargo test --package xtask --quiet benchmark_manifest -- --format terse
cargo test --package xtask --quiet benchmark_workspace -- --format terse
cargo test --package xtask --quiet benchmark_execution -- --format terse
just bench-validate
just bench-ci
just bench-check
just bench-frontend-check
just validate
```

### Phase 4 review checkpoint

Confirm:

- cleanup authority is explicit
- config parsing was not duplicated
- no hard-coded `dev` or `release` list remains in workspace code
- cleanup succeeds before verification and persistence
- pre-existing roots are never deleted
- no Drop-only success path remains

Suggested commit:

```text
bench: finalise generated outputs explicitly
```

## Phase 5 - Consolidate duplicated suite logic and typed groups

Do this after correctness phases pass so the consolidation preserves known behaviour.

### 5A - Extract exact shared measurement

CLI and frontend suites already use the same `execute_case` result.

Create one shared function for:

- measured iteration loops
- duration collection
- observation averaging
- mean, median and standard deviation
- checked measurement identity lookup
- `BenchmarkCaseResult` construction

Delete:

- duplicate CLI/frontend measurement loops
- duplicate CLI/frontend case-result builders

Keep frontend compiler adaptation in `frontend_bench.rs`.

### 5B - Extract exact shared presentation and persistence

Create one typed presentation path parameterised by `BenchmarkSuiteKind`.

It owns:

- group and suite stats
- local system identity mode
- clean previous-run lookup
- quick-subset comparison
- result lines
- stage movement rendering
- current `BenchmarkRun` construction
- normal history append
- tracked monthly summary update

Keep suite-specific labels and primary metrics on `BenchmarkSuiteKind`.

Delete duplicate:

- frontend and CLI previous-run loaders
- presentation structs
- comparison construction
- persistence functions
- debug consistency checks

Run legacy result migration only when it is still required.

Audit `bench_migration.rs`:

- if `benchmarks/results/` and `benchmarks/old-benchmarks/` are obsolete and no current tests require automatic migration, delete the module and call
- otherwise move migration to one explicit shared recording boundary
- do not silently mutate legacy directories on every CLI recording while frontend recording skips it

### 5C - Simplify suite entry points

`bench.rs` should own only CLI-specific setup:

- prepared run
- clean eligibility for record mode
- release compiler construction
- CLI case selection
- CLI execution context
- call shared suite pipeline

`frontend_bench.rs` should own only:

- prepared run
- clean eligibility for record mode
- frontend case selection
- frontend execution context
- public compiler frontend adapter
- call shared suite pipeline

`bench_ci.rs` should use:

- the shared full preflight
- the shared measurement helper
- the shared read-only presentation helper

`bench_validate.rs` remains a thin full-preflight command.

Remove `run_preflighted_suite` when the new explicit operation -> finish -> verify -> persist sequence supersedes it.

### 5D - Add a typed benchmark group

Replace arbitrary group strings with:

```rust
pub(crate) enum BenchmarkGroup {
    Core,
    Docs,
    Stress,
    Module,
    Parallelism,
    Borrow,
}
```

The enum owns:

- manifest spelling
- persistence spelling
- display label
- stable sort order

Unknown group values fail manifest validation.

Keep JSON/TOML spelling stable so normal history format need not change solely for this refactor. Legacy records with unknown groups remain legacy-only and non-comparable.

### 5E - Line-count and complexity gate

Compare Phase 0 line counts.

Acceptance:

- combined production lines in the duplicated suite/profile orchestration files decrease
- `bench.rs` and `frontend_bench.rs` are thin wrappers
- `profile/mod.rs` is structural only
- no new generic trait or callback framework exists
- no current API forwards to an old API
- no duplicate case-result builder remains
- no duplicate previous-run loader remains
- no string group sort table remains

If the shared suite module grows beyond one coherent responsibility, split presentation/persistence from measurement. Do not split into many one-function files.

### Phase 5 tests

Add tests for:

- shared measurement produces expected statistics
- CLI and frontend equivalent synthetic executions produce the same result shape
- quick selection remains read-only
- recording still requires full selection
- failure before completion writes no history or summary
- clean previous-run selection is shared across both suite kinds
- every accepted group parses and sorts correctly
- unknown group fails manifest loading
- group persistence spelling round-trips

### Phase 5 validation

```bash
cargo fmt --all -- --check
cargo test --package xtask --quiet benchmark_suite -- --format terse
cargo test --package xtask --quiet bench_ci -- --format terse
cargo test --package xtask --quiet frontend_bench -- --format terse
cargo test --package xtask --quiet benchmark_manifest -- --format terse
just bench-validate
just bench-ci
just bench-check
just bench-frontend-check
just validate
```

### Phase 5 review checkpoint

Run an independent read-only audit.

Confirm:

- one setup order
- one identity lookup
- one measurement loop
- one result builder
- one comparison/presentation path
- one normal persistence path
- one typed group authority
- explicit workspace finalisation
- less production code than Phase 0
- no framework or compatibility bloat

Suggested commit:

```text
bench: consolidate suite orchestration
```

## Phase 6 - Documentation, protocol reset and final baselines

### 6A - Protocol and schema versions

Because manifest schema and workload identity semantics changed:

- benchmark manifest schema becomes 3
- source fingerprint version increments
- `BENCHMARK_PROTOCOL_VERSION` becomes 3
- profile history format becomes 4
- profile run-manifest format becomes 4
- `PROFILE_PROTOCOL_VERSION` becomes 2

Do not bump normal JSONL format unless its serialized shape changes.

### 6B - Documentation

Update `benchmarks/README.md` to state:

- recorded CLI and frontend runs require a clean committed worktree
- read-only modes permit dirty but unchanged worktrees
- dirty profile runs produce artifacts but no comparable history
- schema 3 generated output roots are explicit cleanup authority
- cleanup completes before repository verification
- current case groups are typed and closed
- profile current identity is mandatory
- current protocol versions

Correct stale comments:

- schema-1 wording in `benchmark_manifest.rs`
- numbered phase comments in profile files
- Drop-based cleanup promises
- any claim that dirty unchanged runs can become public baselines

Condense the completed benchmark correction plan to:

- accepted architecture
- final evidence
- this hardening plan's completion commit range

Remove present-tense defect descriptions and executable old phase instructions.

### 6C - Full non-recording validation

From a committed worktree:

```bash
cargo fmt --all -- --check
cargo test --workspace --quiet -- --format terse
cargo run --quiet -- tests --terse
cargo run --quiet -- check docs --terse
just bench-validate
just bench-ci
just bench-check
just bench-frontend-check
just validate
```

Capture `git status --porcelain=v1` before and after each benchmark command.

Manually verify dirty read-only behaviour:

1. create one temporary non-ignored untracked sentinel
2. run `just bench-check`
3. confirm it succeeds when the sentinel remains unchanged
4. run `just bench`
5. confirm it rejects before compiler construction
6. remove the sentinel

Do not commit the sentinel.

### 6D - Profile validation

When Samply is available:

```bash
just profile-case speed_test_build raw-index
```

Verify:

- artifacts are under `benchmarks/local-data/profiles/`
- run manifest is valid JSON
- case identity is complete
- repository remains unchanged
- raw-index contains no summary reference

Also run one dirty profile smoke:

- create a temporary dirty sentinel
- run the same raw-index case
- confirm artifacts are written
- confirm history append is skipped
- remove the sentinel

### 6E - Clean old protocol summary evidence

Back up:

- `benchmarks/local-data/runs.jsonl`
- `benchmarks/local-data/profile-runs.jsonl`
- `benchmarks/summaries/2026-08-Summary.md`

Do not delete raw local records solely by date.

Because protocol 3 is a new comparison boundary:

- old protocol-2 records remain readable local history
- they are never selected as protocol-3 baselines
- remove superseded protocol-2 run entries from the tracked August summary when they no longer provide useful public evidence
- retain only concise historical context when deliberately wanted

Use a one-off script under `/tmp` to identify records and print every candidate before modifying local history.

### 6F - Record clean baselines

Commit the implementation and documentation before measuring.

Then:

```bash
just bench
```

Inspect:

- 28/28 cases
- protocol 3
- clean revision
- complete identities
- no repository mutation

Commit only the CLI summary:

```text
bench: record hardened CLI baseline
```

Return to a clean worktree:

```bash
just bench-frontend
```

Inspect:

- 30/30 cases
- protocol 3
- clean revision
- complete identities

Commit only the frontend summary:

```text
bench: record hardened frontend baseline
```

Do not amend either baseline commit.

## Phase 7 - Final closeout audit

Run a final independent audit after both baseline commits.

### Correctness

- recorded normal runs require clean committed state
- read-only dirty runs remain allowed when unchanged
- profile snapshot precedes fingerprinting
- current profile identity is mandatory
- dirty profile history is not persisted
- cleanup succeeds before verification and persistence
- undeclared or pre-existing output roots are never deleted
- all 58 cases preflight successfully

### Consolidation

- one prepared-run owner
- one measurement identity helper
- one case measurement loop
- one case result builder
- one normal suite presentation/persistence path
- one typed group authority
- no duplicate CLI/frontend comparison logic
- no manual current profile JSON
- no obsolete migration or compatibility wrapper
- production line count decreased from Phase 0

### Style

- test modules live outside production files
- module docs describe current ownership only
- `mod.rs` files are structural maps
- no inline long-path imports
- no broad context bag or generic trait framework
- functions have one job and clear intermediate values
- no user-data panic path was added
- no lint allowance was added

### Final commands

```bash
cargo fmt --all -- --check
just validate
just bench-validate
just bench-ci
just bench-check
just bench-frontend-check
```

When Samply is available:

```bash
just profile-case speed_test_build raw-index
```

Mark the plan complete only after the audit is clean.

## Stop conditions

Stop and report the exact conflict when:

- clean recording cannot be enforced before expensive work
- a dirty current record is still eligible for comparison or summary publication
- profile identity must remain optional outside explicit legacy adapters
- generated output cleanup requires duplicating project config parsing
- a cleanup root cannot be proven run-owned and inside the workload entry
- cleanup can still fail after persistence
- consolidation introduces a trait framework or increases production line count
- a legacy format is unclear and would require guessing
- a benchmark fixture fails and the owning compiler/build contract is uncertain
- any required gate remains red
- a baseline command starts from a dirty worktree or mutates the repository

Preserve accepted work and report the failing command, files and ownership conflict. Do not add a permissive fallback.

## Suggested commit sequence

1. `bench: require clean committed recording`
2. `bench: centralise run preparation and identity`
3. `profile: require clean typed history`
4. `bench: finalise generated outputs explicitly`
5. `bench: consolidate suite orchestration`
6. `docs: align benchmark hardening`
7. `bench: record hardened CLI baseline`
8. `bench: record hardened frontend baseline`

Each implementation commit runs its focused tests. Commits 3, 4, 5 and 6 must pass `just validate`.

## Final agent report

Report:

- commits and one-line purpose
- before/after production line counts
- final module ownership map
- clean recording enforcement point
- dirty read-only and dirty profile behaviour
- current benchmark/profile protocol and format versions
- generated output root schema and safety rules
- deleted duplicate functions and modules
- legacy adapters retained
- exact test and validation results
- Samply validation or exact omission
- repository state before and after non-recording commands
- CLI and frontend baseline commits and averages without claiming an optimisation
- every remaining limitation or deferred follow-up
