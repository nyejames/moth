# Benchmark and output-system final correction plan

## Purpose

Correct the remaining output ownership, dev-server and filesystem-safety defects found after the benchmark closeout review. Keep the accepted benchmark harness intact, remove duplicated output policy and record trustworthy baselines only after the final code is committed and validated.

This plan intentionally stays off the roadmap until the current language documentation migration reaches a safe pause. It must be completed before another canonical-module slice changes `src/build_system/build.rs` or the output subsystem.

## Current state

```text
WORK_ID: benchmark-output-final-corrections
WORK_SOURCE: docs/roadmap/plans/benchmark-correctness-follow-up-implementation-plan.md
BASE_REVISION: f15fbbfab7b0d3b4c18cb82ecaa14f311ffb3225
REVIEW_BASE: c162458b3f22c48e5b277e3c777909e1a59cdbce
IMPLEMENTATION_UNDER_REVIEW: fc81a727b74f04ec524cc2ac3834026ea23ae1d7 through ed14f185f534ee9da904166bbdc294070b4111ac, including the explicit foreign-manifest-owner correction and final benchmark records
STATUS: complete, implementation and benchmark closeout accepted, independent final audit clean, intentionally not linked from the roadmap
CURRENT_SLICE: Output lifecycle ownership, canonical root identity, Windows-safe portable identity, fail-closed hard-link inspection, byte-safe manifest recovery, explicit-directory retention, non-regular stale-node retention, repository-anchored file-entry profiling artifacts, Stage 0 exclusion for lexical, physical, contained-symlink and descendant-alias output paths, and physical-root diagnostics complete; profile manifests serialize valid optional summary references; explicit unknown v4 builder/profile owners fail closed with no mutation; final evidence is committed
ACCEPTED: Phase 0 baseline; one selected BuildProfile and OutputOwner per build; validated directory plans consumed by CLI, check, frontend benchmarks and dev; fail-closed manifest ownership; scaffold manifests removed; prepared output preflight and stale-path retention; restored config/output/dev-server/scaffold coverage; stale skipped-collision coverage and comments removed; canonical output-root alias rejection; authored diagnostics; Windows portable-component policy including superscript DOS-device names; fail-closed Windows hard-link inspection; non-regular stale-node retention; validated Stage 0 exclusion for separator-normalized output roots and contained symlink aliases; repository-anchored file-entry profiling output with regression coverage; final independent audit clean after the symlink-alias correction; final CLI and frontend records committed from corrected revisions
OPEN_CORRECTIONS: none
DEPENDENCY: rebase and review before implementation if canonical-module or config work changes build.rs, project config output fields, dev build orchestration or output manifests
NEXT_ACTION: none
VALIDATION: cargo fmt --all -- --check; cargo test --workspace --quiet build_cleanup -- --format terse (40/40) pass; just validate (native/Linux/Windows clippy, 3919 workspace tests, 538 xtask tests, 2 ignored doctests, 1822 integration cases, docs clean, benchmark sanity clean); just bench-validate (58/58); just bench-ci (58/58 preflight, 8 CLI quick, 10 frontend quick); just bench-check (28/28); just bench-frontend-check (30/30); history inspection found 12 complete clean protocol-2 records; final raw-index profile JSON/gzip/path checks pass
AUDITS: pass 1 findings corrected; pass 2 found canonical bootstrap containment and canonical-alias gaps; verification found dangling-alias resolution, blank-metadata contract and quadratic-preflight gaps; second verification found active-chain quadratic traversal and obsolete parity test; final focused audit found reserved manifest, canonical identity, lossless serialization and stale-comment gaps; follow-up verification found reserved-manifest descendant and stale-cleanup gaps; canonical alias-to-manifest, portable lossless identity, hard-link, owner-recovery and all-kind destination findings corrected and covered; final independent audit clean after validated Stage 0 output exclusion correction; final closeout audit found contained symlink-alias Stage 0 exclusion gap; correction implemented and fresh final baselines recorded; latest independent audit found canonical output-root containment and invalid/raw-index profile-manifest references; both corrections implemented with focused and full validation, valid regenerated profile evidence and final baselines; final audit found low-severity physical-root diagnostic wording gap; wording and integration expectation corrected, final focused/full validation and final benchmark records pass; latest final audit found high-severity explicit unknown v4 owner recovery gap; foreign-owner classification and unknown-builder/profile no-mutation regressions implemented and committed in 14da8191f; final manifest-owner CLI/frontend records committed in bea79d655 and ed14f185f; independent final audit 20260802T084016Z-4c1f509b returned audit_clean with no findings
BLOCKERS: none
```

Keep this capsule concise. Update it only at accepted checkpoints. Git history remains the implementation record.

## Required authorities

Read these before implementation and before each review checkpoint:

- `AGENTS.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/src/docs/codebase/language/overview.mtf` and its relevant canonical references
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `docs/src/docs/codebase/memory-management/overview.mtf`
- `docs/src/docs/progress/@page.moth`
- `benchmarks/README.md`
- `docs/roadmap/plans/canonical-module-compilation-and-scoped-packages-plan.md`
- `docs/roadmap/plans/project-config-and-recursive-schemas-plan.md`
- this plan

The build-system design owns command profile selection, output roots, writer policy, manifests and stale cleanup. The compiler overview owns compiler stages and diagnostic boundaries. The style guide owns module shape, comments and readable data flow. The testing guide owns test placement and primary contract ownership.

## Accepted foundation

Do not rewrite these completed systems:

- `benchmarks/manifest.toml` is the only benchmark case authority
- CLI failures return non-zero status
- benchmark status and timing records fail closed
- preflight and measurement use the shared benchmark executor
- single-file benchmark builds run below `target/benchmark-work/`
- repository mutation blocks normal and profile persistence
- source workload identity and case measurement identity stay separate
- normal history and profile drift compare only compatible identities
- `bench-ci` preflights the full inventory and measures a bounded quick set
- the eight extensionless benchmark imports migrated from `@./name` to module-root-relative `@name`
- explicit provider imports such as `@./metrics.js` remain valid

Do not restore text case lists, path-derived identities, file-relative Moth imports, permissive output parsing or another benchmark command-construction path.

## Scope boundary

This plan owns:

- selected build-profile identity for output policy
- output-root validation and resolved output plans
- output manifest ownership and recovery states
- output batch preflight and emission inputs
- stale manifest cleanup retention
- CLI and dev reuse of the same output plan
- scaffold interaction with manifests
- focused diagnostics, tests and baseline regeneration

This plan does not own:

- canonical module artefact retention or graph outcome redesign
- provider indexes, materialisation worklists or generated summary convergence
- recursive project config schemas or migration to `html.dev_output` and `html.release_output`
- removal of transitional `package_folders`
- final builder-selection syntax
- transactional rollback for arbitrary mid-write filesystem failures
- source-language or import semantics

Transfer findings to their owning plan rather than implementing a second path here.

## Confirmed defects

### Manifest ownership is advisory instead of authoritative

A known v4 builder or profile mismatch currently becomes limited safe mode. The writer emits new files, warns and replaces the manifest with the active owner. This contradicts the accepted rule that another builder or profile causes a structured conflict before output mutation.

### The scaffold writes the wrong release owner

`manifest_template()` always emits `profile: dev`, while the scaffolder writes the same text to both `dev/.moth_manifest` and `release/.moth_manifest`. Once ownership becomes fail-closed, a new project's first release build would reject its own release directory.

### CLI and dev do not consume one complete output plan

The current `OutputPlan` carries only an output root and optional project directory. Its comment claims profile ownership that the type does not contain. The dev writer reconstructs its project boundary from `entry_file.parent()` and continues using an output directory chosen before the latest build. A config change can therefore leave dev writing and watching the old root.

### Output preflight is only lexical

The writer checks relative syntax and exact duplicate paths, then joins destinations again during emission. It does not reject symlink escapes, file/child path conflicts or deterministic case-only collisions. Failed stale deletion is warned about but omitted from the next manifest, so the build system loses ownership of a file it failed to remove.

### Test ownership and comments drifted

`skipped_directory_collision_ignored` no longer contains the skipped directory collision named by its primary contract. Several tests encode fail-open owner mismatch, phase-number banners remain and config/bootstrap comments still describe removed import behaviour. Some new config diagnostics have only implementation-shaped Rust coverage.

### Recorded baselines cannot prove the final code revision

The implementation, fixture changes, plan completion and two baseline updates were squashed into one commit. The July summary also retains runs already identified as contaminated. New baselines must be recorded from a clean committed correction revision and committed separately without amendment.

## Required ownership model

The final code must have one owner for each fact.

### Build profile

Define one build-system profile enum for command policy, conceptually:

```rust
pub enum BuildProfile {
    Dev,
    Release,
}
```

Derive it once through one shared `from_flags` helper. Output policy and the HTML builder consume that value. Keep the full flag slice only for unrelated feature flags. Convert to any frontend-specific profile type at one explicit compiler boundary. The output subsystem must not depend on `FrontendBuildProfile`.

### Builder identity and output owner

The selected artefact builder supplies one stable closed identity. The current implementation may keep a small enum because HTML is the only production artefact builder.

```rust
pub struct OutputOwner {
    pub builder: BuilderKind,
    pub profile: BuildProfile,
}
```

`OutputOwner` is the only builder/profile pair. Do not duplicate its fields inside `CleanupPolicy`, `OutputPlan`, manifest state and caller-local tuples.

Preferred boundary:

- `BackendBuilder` exposes its stable `BuilderKind`
- command policy supplies `BuildProfile`
- `ValidatedOutputPlan` stores the resulting `OutputOwner`
- `CleanupPolicy` stores only deletion scope such as managed extensions
- the manifest stores the same `OutputOwner`

Audit `BuilderKind::Generic`. It must not remain as a production fallback that lets unrelated future builders share one manifest identity. Remove it when it is test-only or replace it with an explicit test-only identity. Do not introduce a builder registry in this plan.

### Validated output settings and plan

Validate and normalise directory output settings once during bootstrap. Keep the result outside the transitional flat `Config` storage so the queued recursive-config plan can replace that storage cleanly.

Conceptual shape:

```rust
pub struct ValidatedDirectoryOutputSettings {
    pub dev: ValidatedOutputFolder,
    pub release: ValidatedOutputFolder,
}

pub struct ValidatedOutputFolder {
    pub relative_path: PathBuf,
    pub resolved_path: PathBuf,
    pub location: SourceLocation,
}

pub struct ValidatedOutputPlan {
    pub output_root: PathBuf,
    pub project_root: PathBuf,
    pub entry_root: PathBuf,
    pub owner: OutputOwner,
    pub setting_location: SourceLocation,
}
```

Exact names may vary. Preserve these facts and do not add a broad build context bag.

`BuildBootstrap` should carry the validated directory settings. `BuildResult` should carry the selected directory output plan when the entry is a directory project. Single-file command output remains a separate explicit plan using the command working directory and the source-file boundary.

### Prepared output batch

The writer owns one preflight result with every final destination already resolved.

```rust
pub struct PreparedOutputWrite {
    pub files: Vec<PreparedOutputFile>,
    pub managed_paths: HashSet<PathBuf>,
    pub cleanup: PreparedOutputCleanup,
}
```

Store output indexes or borrowed records plus final destinations. Emission must not rejoin or reinterpret paths after preflight.

## Module layout

Create a focused output subsystem instead of expanding `build.rs` and `output_cleanup.rs` further:

```text
src/build_system/output/
├── mod.rs
├── policy.rs
├── manifest.rs
└── writer.rs
```

Responsibilities:

- `mod.rs`: subsystem map and narrow exports
- `policy.rs`: `BuildProfile`, `OutputOwner`, output-folder classification and resolved plans
- `manifest.rs`: manifest parsing, owner comparison, stale cleanup and persistence
- `writer.rs`: output-batch preflight and emission

Keep shared destination containment helpers private to this subsystem. Do not move them to `utils.rs`.

Move existing output code directly and update call sites in the same slice. Do not leave compatibility re-exports, deprecated aliases or duplicate old modules. `build.rs` keeps build orchestration and the builder/backend handoff. It must not remain the implementation owner for output writing.

If current file shape makes a directory module materially worse, stop and propose a smaller flat layout with the same ownership boundaries. Do not keep adding unrelated functions to `build.rs` by default.

## Phase 0 - Refresh and baseline

Before editing code:

1. Record revision, branch and `git status --porcelain`.
2. Reload the required authorities and current implementations.
3. Inventory every use of:
   - `CleanupPolicy`
   - `OutputOwner`
   - `BuilderKind`
   - `FrontendBuildProfile` in build/output code
   - `OutputPlan`
   - `resolve_project_output_root`
   - `resolve_directory_output_plan`
   - `WriteOptions`
   - `prepare_output_cleanup`
   - `read_build_manifest`
   - `manifest_template`
   - `skipped_directory_collision_ignored`
4. Confirm whether any commit after `c162458b3f22c48e5b277e3c777909e1a59cdbce` changed these owners.
5. Run the current targeted output tests and `just validate` once. Record failures without changing expectations.

Stop when canonical-module or config work has already changed the same data boundaries. Rebase this plan against that work before implementation.

### Phase 0 checkpoint

Confirm:

- the accepted benchmark harness remains green
- the defect tests still encode fail-open ownership
- scaffolded dev and release manifests are identical
- no current production builder needs an extensible runtime builder registry
- the planned module split does not overlap an active canonical-module slice

## Phase 1 - Centralise profile, owner and output-plan policy

### 1A - Add the build-system profile owner

- Add `BuildProfile` under the output policy owner.
- Add one `BuildProfile::from_flags` or equivalent helper.
- Replace repeated `Flag::Release` profile selection in output resolution and HTML builder setup.
- Keep unrelated flag checks local.
- Add one explicit conversion only where the compiler frontend requires its own profile type.
- Remove `FrontendBuildProfile` imports from output policy, manifests and cleanup tests.

### 1B - Make `OutputOwner` the actual owner

- Make the selected builder expose its stable `BuilderKind` through the existing builder abstraction.
- Construct one `OutputOwner` from builder identity and selected profile.
- Store that owner in the validated output plan.
- Remove duplicated `builder_kind` and `build_profile` fields from `CleanupPolicy`.
- Remove constructors or accessors that reconstruct an owner from duplicated state.
- Keep managed extension ownership in `CleanupPolicy`.

Do not add string-backed dynamic identities or a registry. A closed enum matches the current builder surface.

### 1C - Validate output folders through one classifier

Implement one pure output-folder classifier used by config diagnostics and plan construction.

Reject:

- empty paths
- absolute paths
- root or platform prefix components
- `.` components
- `..` components
- a resolved path equal to the project root
- a resolved path equal to or below an explicitly configured non-root `entry_root`
- development and release paths that normalise to the same result

Do not silently accept Windows drive-relative prefixes.

The current implementation still permits transitional empty or `.` entry roots in projects that have not migrated to the final config design. Do not implement the broader strict-entry-root migration here. When that transitional form is active, validate the output against the project root and keep the existing source-index exclusion policy. Record the limitation for the recursive-config plan.

Replace the incorrectly named `EqualsProjectRoot` use when the path equals `entry_root`. Either use the existing inside-entry-root diagnostic for equality or add an exact entry-root reason. Remove any reason variant the classifier can no longer produce.

### 1D - Produce validated settings once

- Make project config/bootstrap return `ValidatedDirectoryOutputSettings` after aggregating user diagnostics.
- Carry it in `BuildBootstrap` rather than adding another optional field to transitional `Config`.
- Select one `ValidatedOutputPlan` from those settings and the command profile.
- Carry the selected plan in successful directory `BuildResult` data.
- Keep single-file output planning explicit and separate.
- Delete `resolve_project_output_root` and the old incomplete `OutputPlan` once all callers migrate.

### 1E - Put CLI and dev on the same plan

CLI build:

- consume the `ValidatedOutputPlan` returned by the build
- stop reconstructing output root, project root or owner

Dev:

- use the same plan constructor during the initial bootstrap needed to start the server after a diagnosed build
- make `ProjectBuildExecutor` resolve the authoritative plan from each successful build result
- remove the stale `output_dir` argument from `DevBuildExecutor::build_and_write`
- return the plan used for the successful write with the build result
- update `BuildState.output_dir` and the watch scope when config changes the output root
- stop deriving the project boundary from `entry_file.parent()`
- retain the previous known plan when a rebuild fails before a new config can be accepted

The initial dev bootstrap may remain because the server needs a location even when semantic compilation fails. It must reuse the same config and output-policy owners, not a separate parser or resolver.

### Phase 1 tests

Add focused tests for:

- one profile selection helper
- absolute, parent, current, prefix and empty output paths
- output equal to or inside an explicit entry root
- output equal to project root
- distinct normalised dev/release roots
- exact diagnostic reason and source location for each public diagnostic family
- a real dev execution changing `dev_folder` and then writing and watching the new root
- CLI and dev plans built through the production helper with equal owner and root facts
- single-file planning remaining independent of directory output rules

Delete the existing test that calls `resolve_directory_output_plan` twice and labels the result as CLI/dev parity.

### Phase 1 validation

Run:

```bash
cargo fmt
cargo test --workspace --quiet project_config -- --format terse
cargo test --workspace --quiet build_orchestration -- --format terse
cargo test --workspace --quiet dev_server -- --format terse
cargo test --test cli_exit_status --quiet
just bench-validate
just validate
```

### Phase 1 review checkpoint

Stop for review. Confirm:

- profile selection exists once
- owner identity exists once
- output settings are classified once during bootstrap
- CLI and dev consume the same plan type
- no caller reconstructs project root or owner from path spelling
- output code no longer imports a frontend profile type
- no compatibility wrapper preserves the old output resolver

## Phase 2 - Make manifest ownership fail closed and remove scaffold manifests

### 2A - Separate parsing, recovery and ownership conflict

The manifest reader should parse filesystem state without deciding that every mismatch is recoverable.

Use explicit states equivalent to:

```rust
pub enum ManifestReadResult {
    Uninitialised,
    Recoverable {
        reason: ManifestRecoveryReason,
    },
    Valid(BuildManifest),
}

pub struct BuildManifest {
    pub owner: OutputOwner,
    pub managed_extensions: BTreeSet<String>,
    pub paths: Vec<PathBuf>,
}
```

Recovery states may include:

- missing manifest in a non-empty existing output root
- unreadable manifest
- unsupported or legacy version
- invalid metadata
- same-owner managed-extension drift

A missing manifest in an absent or empty output root is `Uninitialised` and should not print a warning.

A known v4 owner mismatch is not recoverable. `prepare_output_cleanup` must return a structured diagnostic before directory creation, output writes, stale deletion or manifest replacement.

### 2B - Add one structured ownership diagnostic

Use the normal user-facing diagnostic lane. Carry structured facts for:

- output root
- existing builder identity
- existing profile
- active builder identity
- active profile
- active output setting location or the single-file entry fallback

Do not format this as an internal compiler error. Do not make the diagnostic payload depend on build-system enum types if that creates a compiler-to-build-system dependency. Intern stable rendered owner names at the boundary when needed.

### 2C - Remove manifests from `moth new`

Delete:

- `manifest_template()`
- `DEV_MANIFEST`
- `RELEASE_MANIFEST`
- scaffold conflict ownership for those files
- force-overwrite handling for generated manifests
- tests that require dev and release manifests to match

Keep output directories only when current scaffold UX still needs them. The first successful build owns manifest creation.

Add tests proving:

- a fresh scaffold contains no manifest
- the first dev build writes a dev-owned manifest
- the first release build writes a release-owned manifest
- the two builds succeed independently on default roots

### 2D - Enforce no-mutation conflicts

For both builder and profile mismatch:

- create an existing output file and manifest
- capture their bytes
- attempt the conflicting build
- assert a structured diagnostic
- assert output and manifest bytes are unchanged
- assert no new output directory or file appears

Do not keep tests that expect owner mismatch to succeed in limited safe mode.

### Phase 2 validation

Run:

```bash
cargo fmt
cargo test --workspace --quiet build_cleanup -- --format terse
cargo test --workspace --quiet new_html_project -- --format terse
cargo test --workspace --quiet build_orchestration -- --format terse
cargo run --quiet -- tests --terse
just bench-validate
just validate
```

### Phase 2 review checkpoint

Stop for review. Confirm:

- another owner cannot be overwritten
- first builds do not warn about a missing manifest
- scaffold code knows nothing about manifest format
- v3 exists only as a reader-side recovery input
- owner comparison is implemented once
- conflict tests prove zero mutation

## Phase 3 - Complete output batch preflight and stale ownership retention

### 3A - Prepare final destinations once

Move output emission from `build.rs` into the output writer owner.

Preflight receives:

- the validated output plan
- project output records
- cleanup policy
- write mode

It produces final destinations once. Emission consumes those prepared destinations without joining paths again.

### 3B - Reject the complete conflict set before writing

Preflight must reject:

- empty or non-normal relative paths
- exact duplicate destinations
- a file destination that is an ancestor of another output
- a child output whose ancestor is another non-directory output
- file and directory records claiming the same destination
- deterministic ASCII case-only collisions
- an existing symlink or symlinked ancestor that resolves outside the validated output root

An explicit directory record may contain child outputs. Sort normalised destination keys once and check adjacent and ancestor relationships rather than adding an avoidable quadratic scan.

Reuse one output-subsystem containment helper for writer preflight and stale cleanup. Do not duplicate canonicalisation logic.

### 3C - Keep generic and HTML conflict owners separate

The HTML builder retains source-aware diagnostics for duplicate routes and tracked asset conflicts. The generic writer enforces final filesystem invariants only.

Do not reconstruct route, module or tracked-asset semantics in the writer.

### 3D - Preserve ownership after failed stale deletion

Return a cleanup report that distinguishes:

- removed stale paths
- safe inside-root paths that remain because deletion failed
- invalid or escaping manifest entries that were ignored

When deletion of a safe existing stale path fails, retain it in the next manifest so a later build can retry. Do not lose ownership after printing a warning.

Invalid or escaping manifest entries must never be deleted or copied into a new current manifest.

If manifest persistence fails, return the infrastructure failure. Do not claim successful cleanup.

### Phase 3 tests

Add focused writer tests for:

- symlink escape rejection
- file/child prefix conflict with zero writes
- explicit directory plus child success
- file/directory same-path rejection
- ASCII case-only collision rejection
- an invalid later path producing zero writes
- a manifest owner conflict producing zero writes
- failed stale deletion remaining in the next manifest
- invalid manifest paths never being re-emitted
- emission consuming prepared destinations rather than rebuilding them

Keep one pure reader test for each manifest classification and one writer-boundary test for behaviour. Merge redundant tests that repeat the same recovery outcome.

### Phase 3 validation

Run:

```bash
cargo fmt
cargo test --workspace --quiet output -- --format terse
cargo test --workspace --quiet build_cleanup -- --format terse
cargo test --workspace --quiet build_orchestration -- --format terse
just bench-validate
just bench-ci
just validate
```

### Phase 3 review checkpoint

Stop for review. Confirm:

- every logical and ownership failure occurs before the first write
- destination paths are resolved once
- writer and cleanup share containment code
- no HTML semantics leaked into the generic writer
- failed stale cleanup remains owned
- the output module split reduced `build.rs` rather than adding wrappers

## Phase 4 - Restore test ownership and remove style drift

### 4A - Remove the hollow primary integration case

Delete `skipped_directory_collision_ignored` and its manifest entry unless a current accepted behaviour still needs that exact contract.

Do not keep a primary case named for a configured skipped-directory collision when its fixture contains only a homepage.

If a real current contract exists, replace the fixture with an accepted scenario and strong artefact assertions. Stop and document the authority before doing this. Do not re-allow output directories inside an explicit source entry root to preserve the old case.

### 4B - Add canonical config integration coverage

Add focused integration cases for the public config behaviour:

1. invalid output-folder shape, with one representative path and exact stable reason
2. output equal to or inside explicit `entry_root`
3. development and release output roots not distinct
4. manifest owner conflict when a build is run against an already-owned root, when the integration harness can express this without test-only hooks

Use exact reason keys, source locations and backend intent. Keep pure path variants in Rust unit tests rather than multiplying integration fixtures.

Each non-smoke case needs one clear contract and role under `tests/cases/manifest.toml`.

### 4C - Prune implementation-shaped and duplicate tests

Remove or merge:

- the tautological CLI/dev plan test
- reader and writer tests that assert the same classification without a distinct boundary
- completed `Phase 7B` banners
- comments that say v3 while constructing v4 data
- tests that inspect dead owner accessors
- assertions that only check `is_err()` when a stable reason or no-mutation fact is the real contract

Keep test-only utilities with the test module that owns them. Do not create a broad shared filesystem test framework.

### 4D - Clean code and comment drift

Review every touched file against the style guide.

Required corrections include:

- remove `OutputOwner` constructors or accessors that are no longer the authority
- remove absolute-path fallback branches made unreachable by validation
- remove unused diagnostic payload fields or render them meaningfully
- correct comments that claim config may import Core or Builder packages
- remove comments that restate signatures
- keep WHAT/WHY comments for owner comparison, recovery policy, symlink containment and stale retention
- move inline imports to the module header
- use descriptive names and clear intermediate values
- keep functions focused and split long mixed-responsibility functions
- avoid new lint allowances

Do not refactor unrelated canonical module, config schema or backend code while touching imports.

### 4E - Align documentation without roadmap churn

Update only current authorities and navigation affected by the implementation:

- `docs/build-system-design.md` when exact manifest recovery or output-plan wording needs clarification
- `index.md` when the output module map changes
- `benchmarks/README.md` only when baseline or non-recording command behaviour changes
- the canonical module plan's validation note to remove the now-fixed benchmark fixture blocker, without advancing its current slice
- this plan capsule to `implementation complete, review pending`

Do not add this plan to the roadmap. Do not edit generated docs directly.

### Phase 4 validation

Run:

```bash
cargo fmt
cargo test --workspace --quiet -- --format terse
cargo run --quiet -- tests --terse
cargo run --quiet -- check docs --terse
just bench-validate
just bench-ci
just bench-check
just bench-frontend-check
just validate
```

### Phase 4 review checkpoint

Stop for an independent read-only audit. Do not mark the plan complete yet.

The review must confirm:

- one primary test owner per public behaviour
- no hollow or renamed contract remains
- no duplicate profile, owner, root or manifest logic exists
- output modules follow the style guide
- comments describe current ownership rather than completed phases
- no queued config or canonical-module work was pulled into this plan

## Phase 5 - Clean history and record trustworthy baselines

Do this only after the Phase 4 audit accepts the code and the correction commits are on the branch.

### 5A - Commit implementation before measuring

The complete correction implementation must be committed before any recording command runs.

Do not squash baseline commits into implementation commits. Do not amend a commit after a baseline records its revision.

### 5B - Back up and inspect history

Back up under `/tmp`:

- `benchmarks/local-data/runs.jsonl`
- profile history when present
- `benchmarks/summaries/2026-07-Summary.md`

Use a one-off script under `/tmp` to parse local JSONL and print every candidate record before removal.

Remove only records proven invalid by revision, protocol, suite and timestamp. Expected candidates include the reviewed July 26, July 27 and July 31 runs, but do not delete by date alone when local identity proves otherwise.

Remove matching tracked summary blocks. Leave older historical records intact when they are clearly labelled and not selected as a current baseline.

### 5C - Run the complete non-recording gate

Capture `git status --porcelain` before and after each benchmark command:

```bash
just bench-validate
just bench-ci
just bench-check
just bench-frontend-check
```

Requirements:

- every command passes
- repository state remains unchanged
- no root-level generated output appears
- scaffold tests leave no project artefacts
- all current history identities are complete

Then run:

```bash
just validate
```

### 5D - Record CLI baseline

From the clean committed correction revision:

```bash
just bench
```

Inspect the local record and tracked summary. Commit only the intended CLI summary change:

```text
bench: record corrected CLI baseline
```

Do not amend this commit.

### 5E - Record frontend baseline

Return to a clean worktree, then run:

```bash
just bench-frontend
```

Inspect the local record and tracked summary. Commit only the intended frontend summary change:

```text
bench: record corrected frontend baseline
```

Do not amend this commit.

### Baseline acceptance

- both runs start clean
- recorded revisions already contain the final implementation
- every case has complete source and measurement identity
- changed workload or measurement identity produces no speed claim
- summary case counts match the manifest
- no contaminated run remains selected in the top summary
- raw history remains local
- timing values are generated, never hand-edited

## Phase 6 - Final closeout audit

Run one final repository-aware audit after both baseline commits.

### Output ownership

- one build-system profile selector
- one stable builder identity
- one `OutputOwner`
- one validated output plan
- one manifest parser and owner comparison
- owner mismatch fails before mutation
- scaffold contains no manifest-format knowledge
- CLI and dev use the same plan
- dev updates output and watch roots after accepted config changes

### Filesystem safety

- complete destination preflight occurs before writing
- symlink escapes are rejected
- exact, prefix, file/directory and case-only conflicts are rejected
- emission uses prepared destinations
- stale cleanup deletes only current-owner paths
- failed safe deletions remain tracked
- invalid manifest entries never broaden deletion

### Benchmark integrity

- all cases preflight successfully
- non-recording commands do not mutate the repository
- file-entry builds remain isolated
- current normal and profile history require complete identity
- clean CLI and frontend baselines exist on committed revisions

### Quality

- output code has one clear module owner
- `build.rs` is smaller and keeps orchestration only
- no compatibility shim preserves old output APIs
- no output-specific helper moved to broad utilities
- no dead diagnostic fields, stale comments or phase banners remain
- tests follow primary contract ownership and live outside production files
- no queued config or canonical-module work was duplicated

Run:

```bash
cargo fmt --check
just validate
just bench-validate
just bench-ci
just bench-check
just bench-frontend-check
```

When Samply is installed, also run one file-entry build profile case and confirm the repository stays unchanged:

```bash
just profile-case speed_test_build raw-index
```

When Samply is unavailable, state that exact omission. Automated profile invocation and persistence tests remain mandatory.

After independent review accepts every item, update this plan capsule to `complete`. Leave it off the roadmap unless the user explicitly asks for roadmap placement.

## Stop conditions

Stop and report the exact conflict when:

- canonical-module work has changed `build.rs` or output subsystem boundaries since the review base
- recursive-config work has replaced flat output fields or config bootstrap results
- owner conflict handling appears to require a source-language exception
- CLI and dev cannot share one plan without preserving two policy implementations
- manifest recovery would overwrite a known current-format foreign owner
- symlink safety requires following an untrusted path outside the output root
- output preflight still permits a logical failure after the first write
- a user-facing config error cannot retain a structured diagnostic and useful location
- a test can pass only by weakening the accepted output contract
- a baseline command mutates source, config or benchmark fixtures
- any required validation gate remains red

Preserve accepted work, record the failing command and do not invent a permissive fallback.

## Suggested commit sequence

Keep implementation and measurement changes reviewable:

1. `build: centralise output profile and plans`
2. `build: reject output manifest owner conflicts`
3. `build: preflight resolved output destinations`
4. `tests: restore output-system contract coverage`
5. `docs: align output ownership and closeout state`
6. `bench: record corrected CLI baseline`
7. `bench: record corrected frontend baseline`

Each implementation commit must pass its focused tests. Commits 2, 3 and 5 must pass `just validate` before proceeding. Baseline commits must remain separate and must not be amended.

## Final agent report

The implementing agent must report:

- commit list and one-line purpose for each
- files added, moved, removed and materially changed
- final output module ownership map
- manifest version and all read classifications
- final `OutputOwner` and `ValidatedOutputPlan` facts
- removed duplicated or dead APIs
- integration contracts added or removed
- exact validation commands and results
- before/after repository state for non-recording benchmark commands
- whether the Samply runtime check ran
- local history records removed and the identity used to select them
- CLI and frontend baseline commits and summary values without claiming an optimisation
- findings transferred to canonical-module or recursive-config plans
- any remaining limitation or omitted validation
