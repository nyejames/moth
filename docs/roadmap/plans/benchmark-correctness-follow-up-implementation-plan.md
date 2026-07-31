# Benchmark and build-system closeout plan

## Purpose

Finish the benchmark correctness follow-up, repair the two stale benchmark fixtures, apply the bounded build-system corrections found by the closeout audit, record clean baselines and remove this plan from active work.

Phases 1 through 6 are complete. Their implementation detail belongs in Git history and tests, not in this remaining-work plan.

## Current state

```text
WORK_ID: benchmark-build-closeout
WORK_SOURCE: docs/roadmap/plans/benchmark-correctness-follow-up-implementation-plan.md
AUDIT_BASE_REVISION: c54eafa116e9cba46e01bf65d3d3e6bd7a1bde69
STATUS: active - final closeout
CURRENT_SLICE: Phase 7A - migrate the two stale benchmark fixtures
COMPLETED: benchmark output isolation, repository mutation detection, explicit fingerprint boundaries, split source and measurement identity, identity-safe normal/profile history, checked timing protocol, bounded bench-ci and benchmark documentation
BLOCKERS: module_root_role_mix_frontend and few_modules_many_files_each_frontend still use removed @./ source-import syntax
BUILD_AUDIT: output-root ownership is not yet keyed by build profile, conflicting manifests fail open and directory output settings still accept unsafe empty or absolute roots
NEXT_ACTION: complete Phases 7A through 7F in order, then mark the plan complete and move its roadmap entry out of active work
LATEST_REPORTED_VALIDATION: Clippy, workspace tests, 1812 integration executions and docs checks pass. bench-ci reaches only the two stale benchmark fixtures
```

Keep this block concise. Update it after each accepted slice. Git history is the implementation record.

## Required authorities

Read these before implementation and before the final audit:

- `AGENTS.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/language-overview.md`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `docs/src/docs/codebase/memory-management/overview.mtf`
- `docs/src/docs/progress/@page.moth`
- `benchmarks/README.md`
- `docs/roadmap/plans/canonical-module-compilation-and-scoped-packages-plan.md`
- this plan

The compiler overview owns semantic artefacts and compiler stages. The build-system design owns Stage 0, project orchestration, output policy and manifests. The language overview owns import syntax. The canonical module plan owns canonical artefact retention and module hot-path cleanup.

## Accepted foundation

Keep the completed benchmark system unless this plan names a narrow correction:

- one typed `benchmarks/manifest.toml`
- strict CLI exit status and exact opt-in `MOTH_BENCH status` records
- one shared execution path for preflight and measurement
- isolated file-entry build outputs under `target/benchmark-work/`
- repository-state verification before persistence
- explicit `full_tree` and `partitioned` fingerprint boundaries
- separate source workload and case measurement fingerprints
- protocol-aware normal history and profile drift
- fail-closed timing and counter parsing
- non-recording `bench-ci` as the normal validation gate

Do not restore text case lists, path-derived case identities, source-import fallbacks or a second command-construction path.

## Build-system audit verdict

The build system has a coherent Stage 0 and output-cleanup direction. `output_cleanup.rs` is a valid focused owner. The frontend benchmark API also reuses production path validation, bootstrap and frontend compilation rather than maintaining a second compiler path.

Three closeout corrections belong here:

1. Two benchmark projects still use removed file-relative source imports.
2. Output manifests identify a builder but not the selected build profile. A builder or ownership mismatch enters limited safe mode and can then be overwritten instead of failing before output mutation.
3. Directory output settings can resolve to an absolute path or the project root. The current writer validates individual output paths while emitting them, so a late invalid path can leave earlier files written.

One documentation correction also belongs here. `index.md` still names deleted Stage 0 files such as `reachable_file_discovery.rs` and `import_scanning.rs`.

The following findings are real but already owned by the canonical module plan. Do not duplicate them in this plan:

- retain `CompiledModuleArtifact`, provider interfaces and graph outcomes through `ProjectFrontendCompilation` and `ProjectCompilation`
- remove early flattening back to `Vec<Module>`
- replace repeated provider, materialisation-context and stable-identity linear scans with build-owned indexes
- narrow generated summary convergence instead of rescanning every base module and sidecar
- avoid cloning complete prepared source payloads
- split the oversized `build.rs` and `frontend_orchestration.rs` owners after their final data boundaries settle

The queued config and module work owns removal of legacy `package_folders`, the current flat config storage and structural support-package migration. Existing module-root marker cleanup owns stale internal `#page` or hash-root naming. Do not pull those changes into benchmark closeout.

Complete this plan before resuming a canonical-module slice that edits `build.rs` or `output_cleanup.rs`. If that work resumes first, rebase and stop for an ownership review rather than merging two competing payload shapes.

## Non-negotiable rules

- `@./...` remains invalid for Moth source imports.
- Explicit provider imports with an extension, such as `@./metrics.js`, remain valid where the active provider supports them.
- Do not add a compatibility fallback or globally replace every `@./` spelling.
- Benchmark fixtures provide performance workloads. Canonical language correctness remains owned by focused tests under `tests/cases/` and stage-local Rust tests.
- Output ownership must be checked before creating, deleting or writing output files.
- Directory project output roots must be relative to the project root, outside `entry_root` and distinct between development and release profiles.
- Single-file command output semantics stay unchanged. Benchmark isolation already supplies a safe working directory for file-entry cases.
- Keep one output manifest reader, one owner comparison and one output-file preflight.
- Do not add broad filesystem traits, generic path utilities, compatibility wrappers or parallel manifest formats.
- Baseline recording starts from a clean committed worktree and happens only after every non-recording gate passes.

## Phase 7A - Repair the stale benchmark fixtures

This is a fixture migration, not a compiler change.

Apply these exact substitutions:

```text
benchmarks/module-root-role-mix/api/@api.moth
    import @./detail {detail_tag}
    ->
    import @detail {detail_tag}

benchmarks/module-root-role-mix/lib/toolkit/@kit.moth
    import @./parts {join_parts}
    ->
    import @parts {join_parts}

benchmarks/parallelism/few-modules-many-files-each/site/@page.moth
    import @./copy { site_title }
    ->
    import @copy { site_title }

    import @./panel { render_site_panel }
    ->
    import @panel { render_site_panel }

benchmarks/parallelism/few-modules-many-files-each/site/panel.moth
    import @./stats { stats_label }
    ->
    import @stats { stats_label }

benchmarks/parallelism/few-modules-many-files-each/admin/@page.moth
    import @./copy { admin_title }
    ->
    import @copy { admin_title }

    import @./panel { render_admin_panel }
    ->
    import @panel { render_admin_panel }

benchmarks/parallelism/few-modules-many-files-each/admin/panel.moth
    import @./stats { stats_label }
    ->
    import @stats { stats_label }
```

Then search every benchmark `.moth` and `.mtf` file for `import @./`.

Classify each remaining match:

- extensionless Moth source or content import: migrate it to module-root-relative syntax
- explicit provider file with a registered extension such as `.js`: leave it unchanged
- ambiguous match: stop and report the file, import and active provider contract

Do not add new correctness fixtures for these eight substitutions. Existing import-resolution tests own the language rule. The benchmark gate owns proof that the workloads compile.

### Phase 7A validation

Run:

```bash
just bench-validate
just bench-ci
just bench-frontend-check
just bench-check
```

Acceptance:

- both named frontend cases pass
- every manifest case passes preflight
- no extensionless `import @./` remains under `benchmarks/`
- explicit provider-relative imports remain unchanged
- no benchmark history or summary is recorded
- repository state is unchanged apart from the intended fixture edits

Commit this slice separately:

```text
bench: migrate remaining module-root-relative fixtures
```

## Phase 7B - Enforce output-root and manifest ownership

This slice closes the independent build-system audit finding before new baselines are recorded.

### 7B.1 Validate directory output settings once

Use the current project-config and builder validation boundary. Do not bolt checks onto the benchmark harness.

For directory projects, reject development or release output settings that are:

- empty
- absolute or platform-prefixed
- `.` or contain `..`
- equal to the project root after normalisation
- equal to or inside `entry_root`
- equal to each other after normalisation

Use a structured config diagnostic with the relevant setting location. Do not report these user-controlled values as `CompilerError`.

Keep single-file output behaviour separate. A direct single-file build may still write to the command working directory. File-entry benchmarks remain isolated by `BenchmarkExecutionWorkspace`.

If the queued config migration has already replaced `dev_folder` and `output_folder`, apply the same invariant to the new builder-owned fields. Do not restore the legacy fields.

### 7B.2 Give output manifests a complete owner

Replace builder-only manifest ownership with one small typed value containing:

- stable builder identity
- build profile: development or release

The HTML builder must construct its cleanup policy with the selected profile. Do not infer profile later from an output folder name.

Bump the output manifest from v3 to v4. The v4 header must store builder identity, profile and managed extensions.

Policy:

- matching v4 owner and managed extension set: normal cleanup
- matching v4 owner with changed managed extension set: limited safe mode, preserve old files and write the new v4 manifest after a successful build
- different v4 builder or profile: structured ownership conflict before any output mutation
- v3 manifest: legacy limited safe mode because it has no profile identity, then write v4 after a successful build
- missing, unreadable or unsupported manifests: keep the existing conservative limited-safe behaviour

Do not silently replace a current v4 manifest owned by another builder or profile.

### 7B.3 Preflight the complete output batch before writing

Before `create_dir_all`, stale cleanup or the first artefact write:

- validate every non-`NotBuilt` relative output path
- reject duplicate output destinations in the generic writer
- compute the complete managed-path set once
- load and validate manifest ownership

Only after that preflight succeeds may emission start.

Keep HTML's source-aware duplicate-route and tracked-asset diagnostics. They provide richer source ownership than the generic writer. The generic preflight is the final filesystem contract and must not reconstruct HTML route meaning.

Prefer one named prepared batch such as `PreparedOutputWrite` over parallel vectors or repeated scans. Keep it local to the output subsystem.

### 7B.4 Keep callers on one output plan

Review CLI build and dev-server setup together.

- resolve the selected output root and profile through one build-system helper
- pass one typed ownership value into output writing
- do not let CLI and dev reconstruct manifest identity separately
- preserve `AlwaysWrite` for direct builds and `SkipUnchanged` for dev rebuilds
- preserve the existing separation where `build_project` compiles without writing

Do not create a general build context bag. Use a narrow output plan or write context with only root, project boundary, owner, source location and write mode.

### Phase 7B tests

Add or update focused Rust tests outside production files:

- empty directory output setting is rejected
- absolute and parent-traversing output settings are rejected
- output inside `entry_root` is rejected
- identical development and release roots are rejected
- valid distinct project-relative roots resolve unchanged
- matching v4 owner performs stale cleanup
- development then release against the same v4 root fails before writing
- different builder identity fails before writing
- a v3 manifest enters limited safe mode and upgrades only after success
- managed-extension drift for the same owner preserves old files
- a duplicate or invalid later output path causes zero files to be written
- CLI and dev produce the same owner identity for the same profile

Remove or rewrite tests that assert empty directory output folders fall back to the project root. That behaviour conflicts with the accepted output contract.

### Phase 7B validation

Run:

```bash
cargo fmt
cargo test --workspace --quiet build_orchestration -- --format terse
cargo test --workspace --quiet build_cleanup -- --format terse
cargo test --workspace --quiet dev_server -- --format terse
cargo test --test cli_exit_status --quiet
just bench-validate
just validate
```

Acceptance:

- no output file or manifest changes on an ownership or preflight failure
- the output owner has one typed representation
- v3 support exists only in the manifest reader
- no caller infers ownership from a path spelling
- no new lint allowance or compatibility wrapper exists

Commit this slice separately:

```text
build: enforce output profile ownership
```

## Phase 7C - Correct build-system navigation and audit drift

Update `index.md` to match the current `create_project_modules` module map.

At minimum:

- remove references to deleted `reachable_file_discovery.rs` and `import_scanning.rs`
- describe `source_discovery.rs` as the retained single-file traversal owner
- describe `source_scanning.rs` as the single-file lexical/import extraction owner only where that remains current
- list `provider_store.rs`, `prepared_module.rs`, `module_namespace.rs` and the current canonical directory owners accurately

Review touched source comments for stale claims about the output manifest version, ownership and empty-root fallback.

Do not use this phase to refactor `build.rs`, `frontend_orchestration.rs`, canonical artefact storage, provider indexes, package discovery or config schema. Record any newly discovered overlap in the canonical module or config plan instead of adding another implementation path here.

Review the progress matrix. Update it only if the implemented output support or coverage status changes materially. Do not add a cosmetic row.

Commit this slice separately when it contains more than plan-state updates:

```text
docs: align build-system output ownership references
```

## Phase 7D - Run the complete non-recording gate

Start from a committed worktree. Capture `git status --porcelain` before and after each benchmark command.

Run in this order:

```bash
cargo fmt --check
cargo test --workspace --quiet -- --format terse
cargo run --quiet -- tests --terse
cargo run --quiet -- check docs --terse
just bench-validate
just bench-ci
just bench-frontend-check
just bench-check
just validate
```

Requirements:

- every command passes
- non-recording benchmark commands leave repository state unchanged
- no root-level generated HTML appears
- no output ownership test leaves temporary project artefacts behind
- the two fixture cases pass through the in-process frontend path
- the complete CLI and frontend suites compile cleanly

Do not continue to baseline recording after a flaky or unexplained failure. Re-run only after identifying the cause. Do not weaken a case, remove a gate or change quick selection to hide it.

## Phase 7E - Record clean protocol baselines

Do this only after Phase 7D passes on the exact committed revision to be measured.

Back up local history and the current monthly summary under `/tmp`.

Inspect current local history before deleting anything. Remove only records that are invalid under the completed protocol or were already identified as contaminated by repository-root file output. Print the exact removed records from a one-off `/tmp` script. Do not add permanent migration code and do not guess by date alone.

Record the suites separately:

```bash
just bench
```

Inspect and commit only the intended end-to-end CLI summary update.

Return to a clean worktree, then run:

```bash
just bench-frontend
```

Inspect and commit only the intended frontend summary update.

Do not amend either baseline commit. Do not invent or hand-edit timing values.

Acceptance:

- both records use the current benchmark protocol and history format
- every current case has complete source and measurement identity
- each run records a clean start revision
- no comparison is made against changed workload or measurement identity
- summary blocks and case counts match the current manifest
- raw benchmark and profile data remain local
- the tracked diff contains only the intended summary changes

Suggested commits:

```text
bench: record clean CLI baseline
bench: record clean frontend baseline
```

## Phase 7F - Final audit and close the plan

Perform one repository-aware audit after the baseline commits.

### Benchmark system

- one typed manifest owns every case
- every case passes shared preflight
- preflight, measurement, observation and Samply use one invocation authority
- file-entry builds remain below `target/benchmark-work/`
- repository mutation blocks persistence
- source and measurement identity remain separate
- normal and profile history reject incompatible comparisons
- timing parsing fails closed
- non-recording commands do not mutate the checkout
- no extensionless source import uses `@./`

### Build-system output

- directory output roots are safe, relative, outside `entry_root` and profile-distinct
- manifest v4 stores builder and profile identity
- another owner cannot be overwritten
- every output batch is validated before the first write
- cleanup removes only manifest-owned paths
- CLI and dev share ownership resolution
- user-controlled output mistakes use structured diagnostics
- filesystem and invariant failures stay on the infrastructure lane

### Quality and ownership

- no completed Phase 1 through 6 implementation detail remains in this plan
- no duplicate output owner, manifest reader or path preflight exists
- no compatibility shim preserves v3 as a current producer
- no module-plan finding was reimplemented here
- no broad utility module, trait hierarchy or context bag was added
- touched modules follow file-size, function-size, naming and WHAT/WHY guidance
- tests protect user behaviour or hidden subsystem invariants rather than implementation accidents
- `index.md` names only current files and owners

Run one final:

```bash
just validate
```

Then update:

- this plan state to `complete`
- the canonical module plan's validation note to remove the benchmark-fixture blocker, without advancing its implementation slice
- `docs/roadmap/roadmap.md` to remove this plan from active work and place it under completed work or remove the link if completed plans are intentionally deleted

Do not delete this plan until its final commits and recorded baseline revisions are easy to locate from Git history.

## Stop conditions

Stop and report the exact conflict when:

- an extensionless benchmark import cannot be expressed through the owning module root
- fixing a fixture appears to require restoring file-relative source resolution
- output ownership requires changing compiler or source-language semantics
- the active config plan has already replaced the fields this plan would validate
- the canonical module plan has concurrently changed `ProjectFrontendCompilation`, `ProjectCompilation`, `build.rs` or output ownership
- a second current manifest producer or output writer appears necessary
- output preflight cannot prevent mutation before a user-controlled failure
- a current history record lacks complete identity
- a baseline command changes source, config or benchmark fixtures
- any required gate remains red

Preserve completed work, record the failing command and do not invent a fallback.

## Final agent report

The implementing agent must report:

- commits created and one-line purpose for each
- the exact eight fixture import substitutions
- any remaining `import @./` matches and why each is a valid provider import
- output manifest version and ownership fields
- tests and validation commands that ran
- repository-state evidence around non-recording benchmark commands
- CLI and frontend baseline commits and summary result
- any omitted Samply runtime validation
- every finding transferred to the canonical module or config plan
- confirmation that `just validate` passed
