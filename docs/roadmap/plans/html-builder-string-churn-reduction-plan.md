# HTML Builder String Churn Reduction Plan

Status: Queued — not active
Scope: narrow HTML builder string/path churn reduction (P2)
Related: `docs/compiler-data-layout-design.md`, `docs/roadmap/plans/compiler-source-token-and-diagnostic-data-layout-plan.md`

## Background

Two P2 sites were flagged for string churn:

- `src/build_system/utils.rs:68` — `string_table.clone()` into `error_string_table` for `CompilerError::new_file_error`
- `src/projects/html_project/html_project_builder.rs:114,121,160-164` — `HashSet<PathBuf>` / `HashMap<PathBuf,PathBuf>` plus `output_path.clone()` per entry, and `artifact_entries.iter().cloned()` (which clones `Module` values containing HIR)

Investigation shows the first site is failure-path only (`OutputRejectionReason` from `validate_relative_output_path`). Success builds never hit it. The dominant success-path churn in `html_project_builder.rs:121` is the `Module` clone, not the `PathBuf` keys. All `StringTable` clones in `src/projects/html_project/**` outside `utils.rs:68` are cold error branches. Wider `PathBuf` and `StringTable` widening is owned by the accepted data-layout migration (`PathId(NonZeroU32)` trie, `Arc<FrozenCompilationContext>`, typed failure lanes), which will delete these clones structurally. A narrow pre-migration interner would be transitional duplication.

The accepted fix is therefore measurement-first, then either defer or a narrow success-path fix only if benchmark evidence justifies it.

## Prerequisites (capabilities, not plan links)

- Deterministic build-lifetime source registration and frozen string/table context — gives `Arc`-shared `StringTable`/path tables across the build graph so error-path clones are not needed to manufacture a mutable table for `new_file_error`.
- Genuine 4-byte path identities (`PathId` trie from `docs/compiler-data-layout-design.md`) — lets output collision maps rekey from `PathBuf` to `PathId` without allocating per-entry `PathBuf` keys.
- Typed diagnostic source facts — lets `new_file_error` accept an interned path + frozen context instead of `&mut StringTable`.

Until those capabilities land, only success-path, benchmark-proven narrow changes are in scope.

## Slice 0 — mandatory measurement and Narrow vs Defer decision (no code yet)

Owner: one agent context.
Goal: collect five-run medians and counters before any code, then decide.

Steps:

- 0A: five-run `cargo bench --bench frontend -- --save-baseline` or `just bench-check` / `just bench-frontend-check` medians for the `import-external-churn` and `docs` fixtures (the fixtures that exercise `html_project_builder.rs:114-164` at scale). Capture `target/test-reports/**` bench lines.
- 0B: five-run `detailed_timers,benchmark_counters` capture of `string_table_full_clones`, `string_table_fork_source_base_copies`, `merge_delta_from_source_entries_scanned`, `module_remap_string_ids_calls`, `output_path_owners` (add a local counter under `benchmark_counters` if needed) — attribute `utils.rs:68` vs `html_project_builder.rs:121` separately.
- 0C: inventory every `StringTable::clone()` in `src/projects/html_project/**` and `src/build_system/**`, marking cold (`map_err`, `CompilerError`) vs success-path, with entry counts. Record `artifact_entries.len()` for the bench fixtures.
- 0D: Narrow vs Defer gate — only proceed to Slice 1 if success-path median wall time improves >5 percent with counters proving the cloned path owners are material in success builds. Otherwise log evidence, close without code, and add a single note to the data-layout evidence file: rekey output maps to `PathId` when frozen path tables land and remove error-path `StringTable::clone` when frozen context replaces message-table clones.

Evidence file: append to `benchmarks/frontend-optimization-results.md` or the crate-local evidence note named in the data-layout plan.

Exit: either Defer (no code) or Narrow (slices 1A-1C ordered below).

## Slice 1A — remove success-path Module clone (only if Slice 0 proves it material)

Scope: one slice, one commit.
Change: replace `artifact_entries.iter().cloned()` at `html_project_builder.rs:121` with a borrow or index walk that avoids cloning `Module` (retain `PathBuf` keys unchanged in this slice). No new traits, no generic helpers, no broad `PathBuf` interner.
Test: `cargo test -p moth --lib`, `cargo run --quiet -- check docs --terse`, five-run bench medians show >5 percent success-path win without regressing docs fixture.
Docs: no progress change (narrow perf).

## Slice 1B — rekey output maps to PathId (only after capability lands)

Scope: one slice, one commit, blocked until the frozen path-identity capability ships.
Change: `HashSet<PathBuf>` / `HashMap<PathBuf,PathBuf>` in `html_project_builder.rs:105,114-164` → `HashSet<PathId>` / `HashMap<PathId, PathId>` (or `PathId`+`StringId` pair) using the 4-byte `PathId` from `docs/compiler-data-layout-design.md`. Remove per-entry `PathBuf` clones and `output_path`/`entry_point` owned copies by borrowing `PathId` keys. Keep `Module` borrow fix from 1A.
Test: same as 1A plus `just source-audit` if new bans added.
Docs: no progress change.

## Slice 1C — success-path StringTable borrow fix (only if Slice 0 proves it material in success builds)

Scope: one slice, one commit.
Change: replace the narrow success-path `StringTable` widening in `src/build_system/utils.rs:68` only if counters prove it fires in success builds. Prefer `Arc<FrozenCompilationContext>` borrow or a `Cow`-style borrow when the frozen capability exists; do not introduce a second error-path string interner. If the capability has shipped, delete the clone outright as part of the data-layout migration and close this slice as subsumed.
Test: same as 1A; assert error path still produces `CompilerDiagnostic` with correct `SourceLocation` and no panic.
Docs: no progress change.

## Out of scope

- Any broad `PathBuf` interner or string-table wrapper introduced before frozen tables — it would be deleted by the data-layout plan.
- Boxing `CompilerDiagnostic` or changing `DiagnosticBag`/`CompilerMessages` layout — not needed for these clones.
- Second schedulers, benchmark-only fixtures as correctness coverage, or cross-crate counter plumbing beyond `benchmark_counters`.

## Validation

- Each code slice ends with `cargo test -p moth --lib` (4392 default, 4504 with `timers`, 4525 with `timers,benchmark_counters`), `cargo run --quiet -- check docs --terse`, and five-run bench medians vs pre-slice baseline.
- Final slice runs `just validate` (per `docs/src/docs/codebase/style-guide/validation.mtf`) and the `AGENTS.md` Slice review before the plan file is deleted in the completion commit.

## Sequencing

Blocked on `Diagnostics and tokens optimised memory layout plan` for 1B/1C path-identity and frozen-context capabilities. Slice 0 is the only slice that may run before those capabilities land. Slices 1A-1C are ordered and each is one agent context.
