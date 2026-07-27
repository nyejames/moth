# Benchmark correctness follow-up implementation plan

## Purpose

Finish the benchmark correctness work by fixing the remaining harness side effect and identity gaps found in the follow-up review.

The target state is:

- benchmark and profiling commands never write generated build outputs into the tracked checkout
- preflight, measured execution, observation profiling and Samply use one resolved CLI invocation
- every benchmark command detects tracked or untracked repository changes caused during its run
- recorded revision metadata describes the source state measured at the start of the run
- source workload identity remains separate from case measurement identity
- profile drift compares only cases with matching source and measurement identities
- fingerprint roots and excludes have explicit, validated boundary semantics
- public benchmark documentation describes the implemented timing protocol accurately
- the tracked benchmark summary starts from a clean post-fix protocol baseline

This plan follows the completed benchmark correctness implementation at `5528c3f891b933c71cf8ced625fcca06ea1a0de8` and the reviewed benchmark regeneration at `e97a9b9ff9f8a4d40aae3f2cb427947b1b955819`.

Recommended repository path:

```text
docs/roadmap/plans/benchmark-correctness-follow-up-plan.md
```

## Current state

```text
ACTIVE_PLAN: docs/roadmap/plans/benchmark-correctness-follow-up-plan.md
STATUS: ready for implementation
CURRENT_SLICE: Phase 0 - reload authorities and confirm the reviewed base
LAST_REVIEWED_COMMIT: e97a9b9ff9f8a4d40aae3f2cb427947b1b955819
BASE_IMPLEMENTATION_COMMIT: 5528c3f891b933c71cf8ced625fcca06ea1a0de8
KNOWN_TRACKED_POLLUTION: speed-test.html
KNOWN_BLOCKING_FINDING: single-file build cases run from the repository root
KNOWN_IDENTITY_GAPS: profile history has no workload fingerprint and manifest boundary validation accepts incomplete roots
RECORDING_POLICY: do not run recording benchmark commands until Phases 1 through 6 pass review
NEXT_ACTION: implement Phase 1A only after confirming current main still contains the reviewed ownership paths
```

Update this block after each accepted phase. Keep it concise. Git history remains the durable implementation record, so do not append command transcripts or worker journals.

## Required authorities

Read these files before implementation and again before the final audit:

- `AGENTS.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/language-overview.md`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `benchmarks/README.md`
- this plan

The compiler overview owns compiler semantics and stage contracts. The build-system design owns output writing and project orchestration. This plan changes repository benchmark tooling only. It must not redefine compiler output semantics or make the benchmark harness another build system.

## Scope

This plan owns:

- benchmark execution working directories
- benchmark CLI invocation resolution
- repository-state capture and mutation checks
- benchmark manifest fingerprint-boundary validation
- source and case measurement fingerprints
- normal benchmark local history and comparisons
- profiling history and drift compatibility
- benchmark documentation corrections
- removal of the generated root `speed-test.html`
- clean baseline regeneration

This plan does not own:

- new compiler language behaviour
- compiler single-file output semantics
- performance pass or fail thresholds
- a general-purpose sandbox framework
- persistent compiler artefacts
- benchmark fixture redesign beyond identity and output safety
- changes to raw Samply profile parsing or hotspot extraction unless identity threading requires them
- CI workflow redesign

If implementation appears to require compiler or build-system semantic changes, stop and report the exact conflict. The harness should adapt to the existing build contract rather than changing that contract for benchmark convenience.

## Accepted foundation

Keep the completed correctness work unless a phase below explicitly replaces a narrow owner:

- typed `benchmarks/manifest.toml` inventory
- strict CLI process exit semantics
- exact opt-in `MOTH_BENCH status` records
- checked timing and counter observation parsing
- shared case execution for preflight and measured runs
- bounded non-recording `bench-ci`
- current benchmark selection and recording policy types
- current local history migration support
- current comparison thresholds and summary shape
- current frontend in-process benchmark API
- current profiling observation and Samply two-pass model

Do not reintroduce path-derived case identity, text case lists or separate preflight command construction.

## Root causes

The remaining problems come from four ownership mistakes.

1. `BenchmarkManifest::cli_invocation` combines authored manifest facts with run-local working-directory policy. This makes single-file build cases inherit the repository root as their output directory.
2. benchmark revision metadata is captured after measurement, so harness-created files can mark a clean run dirty and source edits during a run can enter history silently.
3. `WorkloadFingerprint` mixes source inputs with every runner attached to the workload. One sibling runner change invalidates unrelated case history.
4. profile history matches cases by case ID, command and arguments without source identity. A fixture change can appear as performance drift.

The manifest validator also treats either direction of entry/root ancestry as coverage. A nested source root can pass even when it omits configuration, assets or sibling source trees.

## Non-negotiable design

### No checkout output writes

- Every CLI case with a file entry runs from an isolated case directory under `target/benchmark-work/`.
- The compiler receives an absolute entry path so changing the current directory cannot change source resolution.
- Directory-project cases may keep their project-root output folders because those folders are project-owned, ignored and explicitly excluded from workload fingerprints.
- Do not solve the problem by adding `/speed-test.html` to `.gitignore`.
- Do not add a benchmark-only output flag to the Moth CLI.

### One CLI invocation authority

One owner must combine:

- the manifest case
- its validated workload entry
- the run workspace
- the command
- authored runner arguments

Preflight, measured iterations, the profile observation pass and the Samply pass consume that same resolved invocation. Delete any second path that reconstructs command arguments or working directories.

### Repository state describes measured source

- Capture the Git commit and complete non-ignored source state before compiler construction or benchmark preflight.
- Retain the tracked diff against `HEAD`, the untracked path set and content identity for every untracked file.
- Retain porcelain status only for dirty metadata and readable failure evidence. Porcelain bytes alone cannot detect further edits to a file that started dirty.
- Compare the complete state again after preflight and measurement.
- Run the comparison on success and failure paths.
- Refuse history or summary persistence when the repository changed during measurement.
- Persist the start revision, not a revision captured after measurement.
- A repository that starts dirty may still run. Its tracked diff and untracked file identities must remain unchanged.
- Files created under ignored `target/` or `benchmarks/local-data/` paths do not count as repository changes.

### Source identity and measurement identity stay separate

Use two fingerprints.

```text
source workload fingerprint
    = workload boundary declaration + included logical paths + included bytes

case measurement fingerprint
    = source workload fingerprint + benchmark protocol + this case's runner + expectation
```

Changing one case's runner must not invalidate another case attached to the same workload. Changing source bytes must invalidate every case attached to that workload.

Do not include report-only fields such as `group` or selection-only fields such as `quick` in measurement identity.

### Fingerprint boundaries are explicit

Each workload declares one mode:

```toml
fingerprint_mode = "full_tree"
```

or:

```toml
fingerprint_mode = "partitioned"
```

`full_tree` means the complete entry file or directory forms the authored source boundary, minus explicit generated-output excludes.

`partitioned` means an author deliberately lists disjoint roots under one directory entry. It exists for workloads such as the documentation project where config and source live in separate roots.

No implicit ancestry heuristic may stand in for this declaration.

### No transitional APIs

- Replace the old invocation constructor once the new owner works.
- Rename or replace `workload_fingerprint.rs` when it owns both source and case fingerprints.
- Do not retain old and new current history fields in parallel.
- Legacy history may use explicit read adapters only. Current producers and current-format records use one shape.
- Do not add forwarding wrappers for deleted functions or fields.

### Data-oriented implementation

Prefer:

- small typed enums for entry kind and fingerprint mode
- one run-scoped workspace
- deterministic case directories named by already validated case IDs
- one vector of source fingerprints in workload order
- one vector of measurement identities in case order
- one dense manifest case index retained on each `BenchmarkCase`
- direct indexes through `case_index` and `workload_index`
- explicit structs for repository snapshots and case identity

Avoid:

- trait-based workspace providers
- generic filesystem abstraction layers
- maps keyed by case ID when manifest order or the case's validated ID already provides direct access
- a broad xtask utility module
- per-iteration temporary directories that change warmup behaviour

## Ownership map

The final owners should remain narrow.

### `xtask/src/benchmark_manifest.rs`

Owns:

- authored benchmark inventory
- validated workload entry kind
- fingerprint mode
- logical roots and excludes
- dense case indexes and case to workload relationships
- runner declarations

It must not own temporary working directories or Git state.

### `xtask/src/benchmark_workspace.rs`

New focused module. Owns:

- one run-scoped ignored workspace under `target/benchmark-work/`
- stable per-case working directories
- file-entry output isolation
- resolving one executable CLI invocation from manifest facts

It must not spawn processes, parse output or persist history.

### `xtask/src/benchmark_execution.rs`

Owns:

- executing one resolved case
- validating process, diagnostic and timing facts
- shared preflight and measurement behaviour

It consumes resolved invocations. It does not reconstruct them.

### `xtask/src/process_runner.rs`

Owns process spawning and channel capture only.

### `xtask/src/benchmark_repository.rs`

New focused module. Owns:

- start commit capture
- tracked diff capture against `HEAD`
- untracked path and content identity capture
- porcelain status for dirty metadata and diagnostics
- unchanged-source verification
- producing the `GitRevision` persisted with a run

It must not own benchmark history or summary rendering.

### `xtask/src/benchmark_fingerprint.rs`

Rename or replace `workload_fingerprint.rs`. Owns:

- source workload fingerprints
- case measurement fingerprints
- deterministic file collection and hashing
- one combined fingerprint result in manifest order

It must not own comparisons or history migration.

### `xtask/src/bench_history.rs`

Owns normal benchmark history formats, adapters and persistence. It consumes repository revision and case identity supplied by earlier owners.

### `xtask/src/profile/history.rs` and `xtask/src/profile/drift.rs`

Own profile history format, compatibility selection and drift classification. They consume the same case measurement identities used by normal benchmarks.

## Phase 0 - Reload and confirm the reviewed base

### Goals

- confirm current main still matches the reviewed ownership paths
- avoid reproducing the known output write in the active checkout
- identify any commits after `e97a9b9` that already changed this area

### Steps

- [ ] Read every required authority.
- [ ] Inspect `git log --oneline --decorate -20` and the diff from `e97a9b9` to current `HEAD`.
- [ ] Inspect:
  - [ ] `xtask/src/benchmark_manifest.rs`
  - [ ] `xtask/src/benchmark_execution.rs`
  - [ ] `xtask/src/process_runner.rs`
  - [ ] `xtask/src/bench.rs`
  - [ ] `xtask/src/frontend_bench.rs`
  - [ ] `xtask/src/bench_ci.rs`
  - [ ] `xtask/src/bench_validate.rs`
  - [ ] `xtask/src/workload_fingerprint.rs`
  - [ ] `xtask/src/bench_history.rs`
  - [ ] `xtask/src/bench_types.rs`
  - [ ] `xtask/src/bench_summary.rs`
  - [ ] `xtask/src/profile/mod.rs`
  - [ ] `xtask/src/profile/history.rs`
  - [ ] `xtask/src/profile/drift.rs`
  - [ ] `xtask/src/profile/runner.rs`
  - [ ] `benchmarks/manifest.toml`
  - [ ] `benchmarks/README.md`
- [ ] Search for every construction of `CliBenchmarkInvocation` and `SamplyRunInput`.
- [ ] Search for every call to `get_git_revision`, `git status` and history append functions.
- [ ] Confirm whether `speed-test.html` remains tracked.
- [ ] Do not run `bench-validate`, `bench-ci`, `bench-check`, `bench` or profiling in the active checkout before Phase 1 lands.
- [ ] If reproduction is needed, create a disposable worktree under `/tmp` and run it there.

### Stop conditions

Stop and update the plan before implementation when:

- current main replaced the reviewed invocation or history owners
- another active branch already implements output isolation
- the benchmark harness now delegates output writing through a different path
- the known generated file no longer comes from `speed_test_build`

## Phase 1 - Isolate CLI file-entry outputs

Keep this phase limited to invocation and working-directory ownership. Do not change history formats yet.

### Phase 1A - Retain entry kind in the manifest

- [ ] Add a small typed entry-kind enum such as:

```rust
pub(crate) enum BenchmarkEntryKind {
    File,
    Directory,
}
```

- [ ] Determine entry kind during the existing manifest metadata validation.
- [ ] Retain it in `BenchmarkWorkload` instead of probing `is_file()` or `is_dir()` during every execution.
- [ ] Keep the canonical entry path and repository containment checks unchanged.
- [ ] Add focused manifest tests for file and directory entries.

Do not add an authored `entry_kind` field. The filesystem fact already exists at manifest load time.

### Phase 1B - Add the run workspace

Create `xtask/src/benchmark_workspace.rs` with concise file-level WHAT/WHY documentation.

The workspace should:

- [ ] create `target/benchmark-work/` below the canonical repository root
- [ ] create one unique run directory through `tempfile`
- [ ] create one stable subdirectory per file-entry CLI case
- [ ] reuse that case directory across preflight, warmup, measured iterations, observation and Samply
- [ ] clean the run directory when the owning command exits
- [ ] return contextual infrastructure errors for directory creation failures

A conceptual shape is:

```rust
pub(crate) struct BenchmarkExecutionWorkspace {
    run_root: tempfile::TempDir,
}

impl BenchmarkExecutionWorkspace {
    pub(crate) fn create(repository_root: &Path) -> Result<Self, String>;

    pub(crate) fn resolve_cli_invocation(
        &self,
        manifest: &BenchmarkManifest,
        case: &BenchmarkCase,
    ) -> Result<CliBenchmarkInvocation, String>;
}
```

Exact names may follow the current code, but ownership may not drift.

Resolved invocation rules:

- [ ] convert every workload entry to an absolute path before placing it in CLI arguments
- [ ] use the isolated case directory as `current_directory` for file entries
- [ ] use the repository root as `current_directory` for directory entries
- [ ] append authored runner arguments in their original order
- [ ] reject frontend cases at this boundary
- [ ] reject a missing workload relationship as infrastructure failure

Case IDs already use a restricted safe alphabet. Use the validated ID directly for the case directory. Do not add a second sanitiser with different identity rules.

### Phase 1C - Make execution consume one invocation

- [ ] Give `BenchmarkExecutionContext` access to the run workspace for CLI execution.
- [ ] Add one context method or focused helper that resolves the invocation.
- [ ] Make `execute_cli_case` consume that resolved invocation.
- [ ] Remove `BenchmarkManifest::cli_invocation` when no caller needs it.
- [ ] Keep `process_runner.rs` unchanged apart from signature movement needed to consume the resolved data.
- [ ] Do not let `process_runner.rs` decide working directories.

### Phase 1D - Use the same invocation for profiling

The profile observation pass already delegates execution through `execute_case`. The Samply pass currently reconstructs its command separately.

- [ ] Resolve the CLI invocation once for each selected profile case through the execution context.
- [ ] Pass its exact command, arguments and current directory into `SamplyRunInput`.
- [ ] Delete the separate manifest invocation construction from `profile/mod.rs`.
- [ ] Add a test proving observation and Samply receive identical command inputs.
- [ ] Preserve the current two-pass profile model and profile output layout.

### Phase 1E - Remove tracked pollution

- [ ] Delete the tracked root `speed-test.html`.
- [ ] Do not add a matching ignore rule.
- [ ] Search for other root-level files produced by benchmark cases and remove only confirmed generated outputs.
- [ ] Confirm normal source fixtures remain unchanged.

### Phase 1 tests

Add focused tests under the owning xtask test modules.

Required cases:

- [ ] a file-entry CLI case resolves to an absolute entry argument
- [ ] a file-entry CLI case uses a directory below `target/benchmark-work/`
- [ ] repeated resolution for one case during one run returns the same directory
- [ ] two cases use distinct case directories
- [ ] a directory-entry case keeps the repository root as its current directory
- [ ] authored runner arguments remain ordered after the entry argument
- [ ] frontend cases cannot request a CLI invocation
- [ ] Samply command construction receives the same current directory and arguments as ordinary execution

### Phase 1 validation

Run:

```bash
cargo fmt
cargo test --package xtask benchmark_manifest
cargo test --package xtask benchmark_workspace
cargo test --package xtask benchmark_execution
cargo test --package xtask profile
```

Then capture repository status and run:

```bash
just bench-validate
```

Confirm:

- no root `speed-test.html` appears
- no tracked or untracked non-ignored file changes
- all manifest cases pass preflight

### Phase 1 checkpoint

Pause for review before adding repository mutation detection.

Review:

- invocation construction exists in one place
- file-entry working directories remain stable across iterations
- no compiler output semantic changed
- no duplicate profile invocation path remains
- the new module owns one narrow responsibility

## Phase 2 - Detect repository mutation and fix persistence ordering

This phase turns checkout cleanliness into an enforced benchmark invariant rather than a manual convention.

### Phase 2A - Add benchmark repository snapshots

Create `xtask/src/benchmark_repository.rs`.

Use direct Git commands from the canonical repository root:

```text
git rev-parse --verify HEAD
git diff --binary --full-index --no-ext-diff HEAD --
git ls-files --others --exclude-standard -z
git status --porcelain=v1 -z --untracked-files=all
```

For each untracked path, capture a Git object ID through `git hash-object --no-filters -- <path>` or an equivalent exact content snapshot. Preserve path type and symlink target semantics rather than following a symlink outside the repository.

The owner should retain:

- full commit identity
- complete tracked diff bytes against `HEAD`
- ordered untracked path and content identities
- NUL-delimited porcelain status for dirty metadata and readable evidence
- dirty state derived from the tracked diff or untracked set

A conceptual shape is:

```rust
pub(crate) struct BenchmarkRepositorySnapshot {
    commit: String,
    tracked_diff: Vec<u8>,
    untracked_files: Vec<UntrackedFileSnapshot>,
    porcelain_status: Vec<u8>,
}

impl BenchmarkRepositorySnapshot {
    pub(crate) fn capture(repository_root: &Path) -> Result<Self, String>;

    pub(crate) fn git_revision(&self) -> GitRevision;

    pub(crate) fn verify_unchanged(
        &self,
        repository_root: &Path,
    ) -> Result<(), String>;
}
```

Requirements:

- [ ] capture failures return an explicit infrastructure error
- [ ] comparison uses tracked diff bytes and untracked content identities, not only a dirty boolean or porcelain status
- [ ] a file that starts dirty and changes again is rejected even when its porcelain code stays the same
- [ ] commit changes also fail verification
- [ ] diagnostics list changed porcelain entries in a bounded readable form
- [ ] paths with spaces remain unambiguous
- [ ] ignored `target/` and `benchmarks/local-data/` writes do not appear
- [ ] no panic or `.unwrap()` depends on Git output

Move Git probing out of `bench_history.rs`. History should consume a supplied `GitRevision`.

### Phase 2B - Verify on success and failure paths

Add one small helper that combines an operation result with final repository verification.

Required behaviour:

- operation succeeds, repository unchanged -> return the result
- operation succeeds, repository changed -> return repository mutation error
- operation fails, repository unchanged -> return operation error
- operation fails, repository changed -> return an error containing both failures

Do not hide the original benchmark failure when the repository also changed.

### Phase 2C - Verify before persistence

Restructure orchestration so measurement and persistence no longer happen inside one opaque completion callback.

For normal CLI and frontend recording:

```text
load manifest and resolve repository root
-> capture repository snapshot
-> compute fingerprints
-> build any required compiler binary
-> preflight
-> measure
-> verify repository unchanged
-> present comparison
-> append local history
-> update tracked summary
```

For read-only modes:

```text
capture snapshot
-> run complete command
-> verify repository unchanged
-> return
```

Apply this to:

- [ ] `bench`
- [ ] `bench-check`
- [ ] `bench-frontend`
- [ ] `bench-frontend-check`
- [ ] `bench-ci`
- [ ] `bench-validate`

Persist the start snapshot's full revision in recorded runs. Derive short display or profile-directory prefixes from that stored value without probing Git again.

Do not compare repository state after the summary writer intentionally updates the tracked summary. The invariant protects the measured interval and the boundary before persistence.

### Phase 2D - Put profile persistence after verification

Split profile collection from profile history append.

The inner profile operation may:

- build the profiling compiler
- preflight
- write ignored profile artefacts
- produce a pending current history record

The outer owner must:

- capture repository state before compiler construction
- run the inner operation
- verify repository state on success or failure
- append profile history only after successful verification

Keep the current rule that a failed case leaves no appended history record.

### Phase 2 tests

Use temporary Git repositories for repository-state tests.

Required cases:

- [ ] clean repository remains accepted
- [ ] dirty repository that remains unchanged is accepted and records `dirty = true`
- [ ] a tracked file that starts dirty and changes again is rejected
- [ ] an untracked file that starts present and changes content is rejected
- [ ] tracked file modification is rejected
- [ ] untracked file creation is rejected
- [ ] ignored file creation under `target/` is accepted
- [ ] commit change is rejected
- [ ] operation failure plus repository mutation reports both causes
- [ ] start revision reaches normal history records
- [ ] start revision reaches profile history records
- [ ] persistence callbacks are not reached after verification failure

Do not mock Git through a trait hierarchy. Use narrow pure parsing helpers and temporary repositories where needed.

### Phase 2 validation

Run:

```bash
cargo fmt
cargo test --package xtask benchmark_repository
cargo test --package xtask bench_ci
cargo test --package xtask bench_validate
cargo test --package xtask bench_history
cargo test --package xtask profile
just validate
```

Then capture exact status before and after each command:

```bash
just bench-validate
just bench-ci
just bench-check
just bench-frontend-check
```

All four commands must leave the repository state unchanged.

### Phase 2 checkpoint

Pause for review.

Do not continue when:

- any read-only benchmark command can change repository status without failing
- history append can run before repository verification
- profile history still captures revision after preflight
- the new repository module becomes a generic command runner
- normal and profile paths use different mutation rules

## Phase 3 - Make fingerprint boundaries explicit

This phase changes manifest schema only. It does not yet change persisted history identity.

### Phase 3A - Add schema 2 fingerprint mode

- [ ] Bump `BENCHMARK_MANIFEST_SCHEMA_VERSION` from 1 to 2.
- [ ] Add a typed deserialised `BenchmarkFingerprintMode` with `full_tree` and `partitioned` variants.
- [ ] Require `fingerprint_mode` on every workload.
- [ ] Retain the typed mode in `BenchmarkWorkload`.
- [ ] Reject unknown values through TOML deserialisation or one contextual manifest error.

Do not default missing mode. Schema 2 should make boundary intent explicit.

### Phase 3B - Validate `full_tree`

For a file entry:

- [ ] require exactly one fingerprint root
- [ ] require that root to resolve to the entry file
- [ ] require no excludes

For a directory entry:

- [ ] require exactly one fingerprint root
- [ ] require that root to resolve to the entry directory
- [ ] allow excludes only as strict descendants of that root

### Phase 3C - Validate `partitioned`

- [ ] allow only directory entries
- [ ] require every root to be a strict descendant of the entry directory
- [ ] allow file and directory roots
- [ ] reject duplicate logical or canonical roots
- [ ] reject root pairs where either root contains the other
- [ ] reject roots outside the entry directory
- [ ] require at least one root

Partitioned mode deliberately trusts the authored list as the complete boundary. The explicit mode and exhaustive repository test make that exception visible.

### Phase 3D - Harden excludes

For both modes:

- [ ] reject duplicate excludes
- [ ] require each exclude to be a strict descendant of exactly one directory root
- [ ] reject an exclude equal to a root
- [ ] reject an exclude that contains another declared root
- [ ] reject an exclude that intersects another root boundary
- [ ] preserve support for non-existent generated output directories through nearest-existing-ancestor validation
- [ ] retain repository escape and symlink protections

Delete the current bidirectional `entry_is_covered` heuristic.

### Phase 3E - Migrate the repository manifest

Set:

```toml
fingerprint_mode = "full_tree"
```

for file workloads and directory workloads whose entry directory forms the complete source boundary.

Set:

```toml
fingerprint_mode = "partitioned"
```

for the documentation workload with roots such as:

```toml
fingerprint_roots = ["docs/config.moth", "docs/src"]
```

Do not change workload IDs, case IDs, groups, quick selection or runner declarations in this phase.

### Phase 3 tests

Required focused tests:

- [ ] file full-tree exact root accepted
- [ ] file full-tree extra root rejected
- [ ] file full-tree exclude rejected
- [ ] directory full-tree exact entry root accepted
- [ ] directory full-tree nested-only root rejected
- [ ] partitioned file entry rejected
- [ ] partitioned disjoint roots accepted
- [ ] partitioned root outside entry rejected
- [ ] duplicate roots rejected
- [ ] ancestor and descendant roots rejected
- [ ] exclude equal to root rejected
- [ ] exclude containing another root rejected
- [ ] exclude under no directory root rejected
- [ ] non-existent generated descendant exclude accepted
- [ ] current repository manifest matches its exhaustive expected inventory and modes

### Phase 3 validation

Run:

```bash
cargo fmt
cargo test --package xtask benchmark_manifest
cargo test --package xtask workload_fingerprint
just bench-validate
just validate
```

Repository state must remain unchanged.

## Phase 4 - Split source and measurement identity

This phase replaces the current workload fingerprint model and normal benchmark history shape.

### Phase 4A - Replace the fingerprint owner

Rename `xtask/src/workload_fingerprint.rs` to `xtask/src/benchmark_fingerprint.rs` when it begins owning both levels.

Delete the old module path and update imports in one slice. Do not leave a re-export shim.

Define typed values such as:

```rust
pub(crate) struct SourceWorkloadFingerprint {
    lanes: [u64; 2],
}

pub(crate) struct CaseMeasurementFingerprint {
    lanes: [u64; 2],
}

pub(crate) struct BenchmarkFingerprints {
    pub(crate) workloads: Vec<SourceWorkloadFingerprint>,
    pub(crate) cases: Vec<CaseMeasurementFingerprint>,
}
```

Exact storage may follow the current two-lane implementation. Keep the deterministic non-cryptographic intent documented.

### Phase 4B - Compute source fingerprints once

The source fingerprint must cover:

- source fingerprint format version
- manifest schema version
- workload ID
- entry logical path
- entry kind
- fingerprint mode
- normalised root set
- normalised exclude set
- every included logical file path
- every included file byte

Rules:

- [ ] collect source files once per workload
- [ ] keep deterministic logical path order
- [ ] canonicalise root and exclude ordering so declaration reordering does not change identity
- [ ] reject symlinks or repository escape exactly as the current owner does
- [ ] do not hash runner declarations
- [ ] do not hash `group` or `quick`

### Phase 4C - Compute one measurement fingerprint per case

The case measurement fingerprint must cover:

- measurement fingerprint format version
- `BENCHMARK_PROTOCOL_VERSION`
- the source workload fingerprint
- workload ID
- this case's runner kind
- this case's command or frontend profile
- this case's authored runner arguments
- this case's expectation

Do not hash sibling cases.

Retain one dense `case_index` on every validated `BenchmarkCase` if the current type does not already provide one. Return fingerprints in manifest order. Use `case_index` and `workload_index` to join case and source identity without a path or string lookup.

### Phase 4D - Add one explicit case identity shape

Use one nested current-domain identity rather than several loose optional strings.

Conceptual shape:

```rust
pub struct BenchmarkMeasurementIdentity {
    pub workload_id: String,
    pub source_fingerprint: SourceWorkloadFingerprint,
    pub measurement_fingerprint: CaseMeasurementFingerprint,
}
```

Keep typed wrappers in the current in-memory domain. Convert them to fixed lowercase hexadecimal strings only at the persistence and JSON artefact boundaries.

`BenchmarkCaseResult` may retain `Option<BenchmarkMeasurementIdentity>` only because adapted legacy records lack current identity. Every current producer must provide `Some`.

Profile history should reuse this type or an exact serialisable equivalent rather than defining different identity semantics.

### Phase 4E - Bump normal history and benchmark protocol

- [ ] Increment `BENCHMARK_PROTOCOL_VERSION` because invocation working-directory policy and fingerprint semantics changed.
- [ ] Increment the normal local history format from 6 to 7.
- [ ] Make current v7 records require `BenchmarkMeasurementIdentity` for every case.
- [ ] Add one explicit v6 adapter that reads the old mixed `workload_fingerprint` field as legacy data.
- [ ] Never relabel the old mixed value as a source fingerprint. Adapt it to missing current identity or retain it only inside a legacy-only field that comparisons ignore.
- [ ] Do not present v6 data as directly comparable with the new protocol.
- [ ] Keep older adapters isolated from the current record type.

Do not add optional current-format defaults that allow malformed v7 records to pass.

### Phase 4F - Distinguish identity changes in comparisons

Match by stable case ID first.

Then classify:

- workload ID or source fingerprint differs -> workload changed
- source matches but measurement fingerprint differs -> measurement changed
- both match -> timing comparable

Extend `BenchmarkComparison` with ordered case IDs for workload and measurement changes.

Formatting requirements:

- never compute speed deltas for identity-changed cases
- name `workload changed` for source changes
- name `measurement changed` for runner, expectation or protocol changes
- retain case-set change reporting separately
- when no unchanged measurements remain, report that state instead of `baseline` or a speed claim
- preserve manifest order in named case lists

Update monthly top-block comparison through the same domain path. Do not add a second summary-only identity implementation.

### Phase 4G - Thread fingerprints once

Compute `BenchmarkFingerprints` once per command and pass it through:

- [ ] normal CLI benchmark runs
- [ ] frontend benchmark runs
- [ ] `bench-ci`
- [ ] `bench-validate`
- [ ] normal result construction
- [ ] profile preparation for Phase 5

`bench-validate` should still compute every fingerprint so invalid boundaries and unreadable inputs fail during validation.

### Phase 4 tests

Required fingerprint tests:

- [ ] changing source bytes changes the source fingerprint
- [ ] changing source bytes changes every attached case measurement fingerprint
- [ ] changing one case's runner changes only that case's measurement fingerprint
- [ ] changing a sibling case's runner leaves this case unchanged
- [ ] changing expectation changes measurement identity
- [ ] changing `group` does not change measurement identity
- [ ] changing `quick` does not change measurement identity
- [ ] reordering roots or excludes does not change source identity
- [ ] changing protocol changes measurement identity

Required history and comparison tests:

- [ ] v7 round trip requires complete identity
- [ ] v6 remains readable through the legacy adapter
- [ ] protocol-mismatched runs are not selected as direct baselines
- [ ] same case and source with changed runner reports measurement change
- [ ] same case with changed source reports workload change
- [ ] identity-changed cases do not contribute to average or stage movement
- [ ] quick-subset comparison retains exact identity checks
- [ ] monthly summary output names source and measurement changes correctly

### Phase 4 validation

Run:

```bash
cargo fmt
cargo test --package xtask benchmark_fingerprint
cargo test --package xtask bench_types
cargo test --package xtask bench_history
cargo test --package xtask bench_summary
cargo test --package xtask bench_report
just bench-validate
just bench-ci
just bench-check
just bench-frontend-check
just validate
```

Do not record a new baseline yet.

### Phase 4 checkpoint

Pause for review.

Confirm:

- source file walking happens once per workload
- no runner data remains in source fingerprints
- no sibling case can invalidate another case
- current history has one identity shape
- legacy adapters cannot produce a current-format record silently
- summary formatting never compares identity-changed cases

## Phase 5 - Make profile history and drift identity-safe

Reuse the completed Phase 4 identity. Do not invent a profile-only workload hash.

### Phase 5A - Add profile protocol and format 3

- [ ] Add `PROFILE_PROTOCOL_VERSION` with initial value 1.
- [ ] Increment profile history format from 2 to 3.
- [ ] Store profile protocol version in every current run.
- [ ] Store the start `GitRevision`, not a post-preflight commit string.
- [ ] Store `BenchmarkMeasurementIdentity` on every current profile case.
- [ ] Retain command and arguments as descriptive execution facts, not the comparison authority.

### Phase 5B - Carry identity through profile artefacts

Add workload and measurement identity to:

- [ ] `ProfileCaseManifest`
- [ ] `run-manifest.json`
- [ ] current profile history case records
- [ ] drift case inputs
- [ ] agent-readable root summary data where a workload change must be explained

Do not put absolute source paths into persisted identity.

### Phase 5C - Select compatible profile runs

Run-level compatibility requires:

- same system UUID
- same profile protocol version
- same filter mode
- same sample rate policy

Case-level compatibility requires:

- same case ID
- same source fingerprint
- same measurement fingerprint

Command and argument equality may remain a defensive assertion or diagnostic fact, but fingerprint equality is authoritative.

### Phase 5D - Report identity changes instead of drift

For a matching case ID:

- source differs -> report workload changed
- source matches and measurement differs -> report measurement changed
- both match -> compute stage, counter and hotspot drift

Identity-changed cases must not contribute to:

- wall-time drift
- stage drift
- counter drift
- hotspot movement
- regression or improvement claims

Keep output concise. Name changed case IDs and explain that a new baseline is required.

### Phase 5E - Read legacy profile history safely

- [ ] keep v1 and v2 readable through explicit adapters
- [ ] mark them with no current profile protocol and no current measurement identity
- [ ] never select them as directly comparable v3 runs
- [ ] avoid a parallel legacy/current comparison path

### Phase 5F - Remove silent history failures

Move previous-history and system-identity loading before compiler construction where practical.

Replace:

- `read_profile_runs(...).unwrap_or_default()`
- `load_or_create_system(...).unwrap_or(None)`

with explicit results.

Policy:

- unreadable history or invalid system identity returns a contextual infrastructure error before profiling starts
- an absent prior run remains a normal `None`
- malformed individual legacy lines may follow the existing bounded warning policy when the history reader already supports it
- no error may silently become an empty history set

### Phase 5G - Preserve output isolation through Samply

- [ ] assert profile observation and Samply use the Phase 1 resolved invocation
- [ ] assert `speed_test_build` profiles below `target/benchmark-work/`
- [ ] preserve profile artefacts under `benchmarks/local-data/profiles/`
- [ ] verify repository state before appending profile history

### Phase 5 tests

Required profile history tests:

- [ ] v3 round trip retains profile protocol and case identity
- [ ] v1 and v2 remain readable but not comparable
- [ ] profile protocol mismatch rejects comparison
- [ ] source fingerprint mismatch reports workload changed
- [ ] measurement fingerprint mismatch reports measurement changed
- [ ] identity-changed cases do not enter drift aggregates
- [ ] exact matching identity retains current drift behaviour
- [ ] history read failure propagates
- [ ] system identity failure propagates
- [ ] history append is unreachable after repository mutation

Required profile orchestration tests:

- [ ] run manifest includes case identity
- [ ] observation and Samply use one resolved invocation
- [ ] profile history uses start revision
- [ ] failed profile collection appends no history

### Phase 5 validation

Run:

```bash
cargo fmt
cargo test --package xtask profile
cargo test --package xtask benchmark_fingerprint
cargo test --package xtask benchmark_repository
just bench-validate
just validate
```

When Samply is installed, also run:

```bash
just profile-case speed_test_build raw-index
```

Capture repository state before and after. It must remain unchanged.

When Samply is unavailable, record that exact omission in the final report. The automated invocation and persistence tests remain mandatory.

### Phase 5 checkpoint

Pause for review.

Do not continue when:

- profile drift can compare a case with missing current identity
- profile history uses command text as its primary identity
- profile and normal benchmark fingerprints can disagree for one case
- previous-history errors still become an empty baseline silently
- profile persistence occurs before repository verification

## Phase 6 - Correct documentation and remove stale paths

Documentation changes are explicitly in scope for this plan.

### Benchmark protocol wording

Update `benchmarks/README.md` so it states:

- duplicate `MOTH_BENCH status` records are invalid
- malformed timing records are invalid
- repeated timing metric names inside one iteration are valid and summed
- the required command total must exist
- measured iterations must expose the same timing metric set
- missing or additional timing names across iterations fail the run

Do not describe every repeated timing name as a duplicate protocol failure.

### Manifest documentation

Document:

- schema 2
- `fingerprint_mode`
- full-tree boundaries
- partitioned boundaries
- source workload fingerprints
- case measurement fingerprints
- workload change versus measurement change
- profile drift compatibility
- isolated file-entry build working directories
- worktree mutation refusal before persistence

Keep examples aligned with the checked-in manifest.

### Stale reference audit

Search current documentation and plans for:

```text
benchmarks/cases.txt
benchmarks/frontend-cases.txt
workload_fingerprint
schema 1
duplicate timing records
```

Update files that claim those are current authorities. Historical completion notes may retain old names only when the text clearly describes past state.

Do not edit `docs/release/**` directly. Rebuild generated documentation when a docs-site source changes.

### Code comment audit

Review touched files for:

- stale phase-number comments
- comments claiming profile hotspot extraction is future work
- comments naming the old fingerprint owner
- duplicated WHAT/WHY prose that restates signatures
- `#[allow(dead_code)]` that no longer has a valid reason

Keep comments concise and ownership-focused.

### Phase 6 validation

Run the documentation gate required by the actual changed-file set. Since earlier phases change Rust, the final combined gate remains:

```bash
cargo fmt
just validate
```

When docs-site source changed, also inspect the generated documentation diff after the normal build path. Do not commit unrelated generated changes.

## Phase 7 - Repair local history and record clean baselines

Do this only after Phases 1 through 6 pass review and the worktree starts in the intended committed state.

### Phase 7A - Back up local history

Create backups under `/tmp` for:

- `benchmarks/local-data/runs.jsonl`
- profile history when it exists
- `benchmarks/summaries/2026-07-Summary.md`

Do not commit backups.

### Phase 7B - Remove only contaminated normal CLI records

Identify records by their stored revision, timestamp and suite kind. Remove only the normal CLI runs produced by the reviewed correctness migration and regeneration where the single-file build used the repository root.

Expected revisions include:

```text
5528c3f891b933c71cf8ced625fcca06ea1a0de8
e97a9b9ff9f8a4d40aae3f2cb427947b1b955819
```

Use a one-off script under `/tmp` to parse JSONL and print the exact removed records before writing the cleaned file.

Do not add permanent migration code for this local one-time cleanup.

Remove the corresponding exact run blocks from `benchmarks/summaries/2026-07-Summary.md`. Leave unrelated historical entries intact.

### Phase 7C - Run final non-recording checks

Capture exact repository state before and after:

```bash
just bench-validate
just bench-ci
just bench-check
just bench-frontend-check
```

All must pass and leave repository state unchanged.

### Phase 7D - Record protocol baselines

Record both suite kinds because the shared benchmark protocol and current history format changed.

Run and commit them separately so each measurement starts from a clean repository and records `dirty = false`.

First:

```bash
just bench
```

Inspect the summary diff, then commit only the intended CLI baseline update.

Return to a clean worktree, then run:

```bash
just bench-frontend
```

Inspect the summary diff again, then commit only the intended frontend baseline update.

Do not amend either baseline commit after recording. Amending changes the commit identity stored by the run.

Expected behaviour:

- each suite reports a new baseline under the new protocol
- each recorded run starts clean
- local current-format history contains complete case identities
- the tracked monthly summary top blocks use only the new protocol's runs
- no speed delta is claimed against protocol 1 records
- no root `speed-test.html` appears
- no tracked file changes except the intended monthly summary update

Do not manually invent timing values.

### Phase 7E - Inspect the regenerated summary

Check:

- suite labels and system identity
- baseline wording
- case counts
- group averages
- no comparison against identity-changed records
- no contaminated July 26 or July 27 CLI entry remains
- no duplicate summary block for the same suite and system
- no raw per-case timing table entered the tracked summary

## Review phases

### Review A - Execution safety

Run after Phase 2.

Answer:

- Can any file-entry build write outside `target/benchmark-work/`?
- Do preflight, measurement and Samply use the same invocation?
- Does repository verification run after failures?
- Can persistence run after a source change during measurement?
- Does history use the start revision?
- Did the implementation add any compiler or build-system semantic exception?

Do not continue until every answer is satisfactory.

### Review B - Identity and manifest boundaries

Run after Phase 4.

Answer:

- Does each workload walk its source files once?
- Does one case runner change invalidate only that case?
- Can a nested partial root pass as a full-tree boundary?
- Can excludes suppress another declared root?
- Can malformed current history omit identity?
- Can summary code compare a source or measurement mismatch?

Do not continue until every answer is satisfactory.

### Review C - Profile drift

Run after Phase 5.

Answer:

- Can profile drift compare changed fixture bytes?
- Can profile drift compare a changed runner or benchmark protocol?
- Do v1 and v2 records remain historical only?
- Are history and system identity errors visible?
- Does profile persistence follow repository verification?

Do not continue until every answer is satisfactory.

## Backtrack criteria

Stop and revise the active phase when:

- output isolation requires modifying Moth source semantics
- workspace lifetime differs between warmup and measured iterations
- one phase creates a second invocation or identity path
- manifest validation requires filesystem rescans during execution
- fingerprint splitting causes source files to be read once per case
- history migration weakens current-format validation
- profile identity duplicates normal benchmark hashing logic
- summary rendering grows a second comparison implementation
- a slice introduces broad traits, registries or compatibility wrappers
- targeted tests reveal the current design cannot represent a real repository workload cleanly

Prefer a smaller owner correction over adding a boolean or optional fallback to bypass the conflict.

## Required final audit

Before reporting completion, inspect every touched path and answer each item explicitly.

### Ownership and duplication

- [ ] manifest, workspace, execution, repository state, fingerprints, history and profile drift each have one owner
- [ ] no duplicate CLI invocation constructor remains
- [ ] no duplicate source fingerprint implementation remains
- [ ] profile history reuses normal case identity
- [ ] no obsolete `workload_fingerprint` current field or module remains
- [ ] no compatibility wrapper preserves a deleted current API

### Failure safety

- [ ] user-authored invalid manifest data returns contextual errors
- [ ] Git and filesystem failures return infrastructure errors
- [ ] repository mutation blocks normal and profile persistence
- [ ] operation and mutation errors can both be reported
- [ ] missing identity never falls back to comparison
- [ ] malformed current history cannot pass validation

### Data flow

- [ ] entry kind is discovered once
- [ ] source files are collected once per workload
- [ ] source fingerprints are stored in workload order
- [ ] measurement fingerprints are stored in case order
- [ ] case joins use dense `case_index` and `workload_index`
- [ ] resolved invocations carry absolute entries and explicit working directories
- [ ] persisted identities contain no absolute checkout path

### Tests

- [ ] tests live outside production implementation files
- [ ] unit tests protect subsystem invariants
- [ ] no benchmark fixture is used as substitute correctness coverage
- [ ] current manifest has exhaustive schema and boundary coverage
- [ ] history and profile adapters cover legacy formats without weakening current formats
- [ ] summary tests cover workload and measurement changes
- [ ] worktree tests cover clean, dirty, ignored, tracked and untracked states

### Documentation and comments

- [ ] benchmark README matches implemented protocol
- [ ] current references use the typed manifest
- [ ] no generated docs were edited directly
- [ ] touched files have concise current ownership comments
- [ ] stale phase comments and obsolete owner names are gone

### Validation

Run:

```bash
cargo fmt
just validate
just bench-validate
just bench-ci
just bench-check
just bench-frontend-check
```

When Samply is installed:

```bash
just profile-case speed_test_build raw-index
```

Then record:

```bash
just bench
just bench-frontend
```

State exactly which commands ran and which did not. Do not claim a clean profile runtime gate when Samply was unavailable.

## Completion criteria

The plan is complete only when all of these hold:

- `speed-test.html` is no longer tracked and does not reappear
- every file-entry CLI build runs below `target/benchmark-work/`
- one invocation authority feeds preflight, measurement, observation and Samply
- all read-only benchmark commands leave repository state unchanged
- repository mutation during a run blocks persistence
- current normal history stores complete source and measurement identity
- current profile history stores complete source and measurement identity
- changed source or measurement never produces a speed or drift claim
- schema 2 rejects incomplete full-tree roots and overlapping partitioned boundaries
- repeated timing names are documented as summed within an iteration
- normal CLI and frontend suites have clean new-protocol baselines
- `just validate` passes
- the final audit finds no duplicate, legacy or transitional path

## Suggested commit sequence

Keep commits reviewable and do not combine baseline regeneration with implementation code.

1. `bench: isolate file-entry benchmark outputs`
2. `bench: reject worktree mutation during benchmark runs`
3. `bench: make fingerprint boundaries explicit`
4. `bench: split source and measurement identity`
5. `profile: make drift identity and protocol aware`
6. `docs: correct benchmark identity and timing guidance`
7. `bench: record clean CLI protocol baseline`
8. `bench: record clean frontend protocol baseline`

Each commit must pass its targeted tests. Commits 2, 4 and 5 should also pass `just validate` before the next phase starts. Baseline commits must not be amended after their recorded runs.

## Final agent report

The implementing agent's final response must include:

- commit list and one-line purpose for each commit
- exact files added, renamed, removed and materially changed
- normal benchmark protocol and history format versions
- profile protocol and history format versions
- manifest schema version
- tests and validation commands run
- before and after repository-state evidence for non-recording commands
- whether the Samply runtime check ran
- the newly recorded baseline summary result without claiming performance improvement
- any remaining limitation or omitted validation
