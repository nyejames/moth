# HTML Builder String Churn Reduction Plan

Status: Queued — not active
Scope: narrow HTML builder string/path churn reduction (P2)
Related: `docs/compiler-data-layout-design.md`

## Background

Two P2 sites were flagged for string churn:

- `src/build_system/utils.rs` `file_error_with_rejection_reason` — builds `InfrastructureFailure` with no string-table clone; Slice 1C is subsumed.
- `src/projects/html_project/html_project_builder.rs:184-190,333-334,757` — `HashSet<PathBuf>` / `HashMap<PathBuf,PathBuf>` output maps plus `output_path.clone()` per entry, and `artifact_entries.iter().cloned()` (which clones `Module` values containing HIR).

Investigation shows the first site was failure-path only (`OutputRejectionReason` from `validate_relative_output_path`); success builds never hit it. The dominant success-path churn in `html_project_builder.rs:190` is the `Module` clone, not the `PathBuf` keys. The remaining `StringTable` clones in `src/projects/html_project/**` are cold error branches. The path-identity and frozen-table capabilities are delivered, so this plan reuses them and adds no second interner.

The accepted fix is therefore measurement-first, then either defer or a narrow success-path fix only if benchmark evidence justifies it.

## Prerequisites (delivered capabilities)

- Frozen table snapshots: `FrozenStringTable` (`src/compiler_frontend/symbols/string_interning.rs`) shares immutable string bytes with the live table through `Arc<str>`.
- 4-byte path identities: `PathId(NonZeroU32)` over the parent-linked path trie (`src/compiler_frontend/symbols/path_interner/`) lets output collision maps rekey from `PathBuf` to `PathId` without per-entry `PathBuf` keys.
- Typed failure lane: the `InfrastructureFailure::filesystem` constructor takes a stage, a path and a message, with no `StringTable` argument.

Only success-path, benchmark-proven narrow changes are in scope.

## Slice 0 — mandatory measurement and Narrow vs Defer decision (no code yet)

Owner: one agent context.
Goal: collect five-run medians and counters before any code, then decide.

Steps:

- 0A: five-run `just bench-check` / `just bench-frontend-check` medians for the `import-external-churn` and `docs` fixtures (the HTML project fixtures that run through `html_project_builder.rs`).
- 0B: five-run `detailed_timers,benchmark_counters` capture of `string_table_full_clones` (`FrontendCounter::StringTableFullClones`) and the other `string_table_*` counters in `src/compiler_frontend/instrumentation/frontend_counters.rs`. Attributing clones between the `utils.rs` file-error path and the `html_project_builder.rs` artifact walk may need new counters under `benchmark_counters`; add them only if Slice 0 needs them.
- 0C: inventory every `StringTable::clone()` in `src/projects/html_project/**` and `src/build_system/**`, marking cold (`map_err`, error-path constructors such as `InfrastructureFailure::filesystem`) vs success-path, with entry counts. Record `artifact_entries.len()` for the bench fixtures.
- 0D: Narrow vs Defer gate — only proceed to Slice 1 if success-path median wall time improves >5 percent with counters proving the cloned path owners are material in success builds. Otherwise log evidence, close without code, and add a single note to `benchmarks/frontend-optimization-results.md` that output maps rekey to `PathId` and error paths build `InfrastructureFailure` without a string-table clone.

Evidence file: append to `benchmarks/frontend-optimization-results.md`.

Exit: either Defer (no code) or Narrow (slices 1A-1B ordered below).

## Slice 1A — remove success-path Module clone (only if Slice 0 proves it material)

Scope: one slice, one commit.
Change: replace `artifact_entries.iter().cloned()` at `html_project_builder.rs:190` with a borrow or index walk that avoids cloning `Module` (retain `PathBuf` keys unchanged in this slice). No new traits, no generic helpers, no broad `PathBuf` interner.
Test: `cargo test -p moth --lib`, `cargo run --quiet -p moth-cli --bin moth -- check docs --terse`, five-run bench medians show >5 percent success-path win without regressing docs fixture.
Docs: no progress change (narrow perf).

## Slice 1B — rekey output maps to PathId (only if Slice 0 proves it material)

Scope: one slice, one commit.
Change: `HashSet<PathBuf>` / `HashMap<PathBuf,PathBuf>` in `html_project_builder.rs:184-185,333-334,757` → `HashSet<PathId>` / `HashMap<PathId, PathId>` (or `PathId`+`StringId` pair) using the 4-byte `PathId` from `docs/compiler-data-layout-design.md`. Remove per-entry `PathBuf` clones and `output_path`/`entry_point` owned copies by borrowing `PathId` keys. Keep `Module` borrow fix from 1A.
Test: same as 1A plus `just source-audit` if new bans added.
Docs: no progress change.

## Slice 1C — success-path StringTable borrow fix (subsumed)

Subsumed: `file_error_with_rejection_reason` in `src/build_system/utils.rs` builds `InfrastructureFailure` with no `StringTable` clone, so this slice has no remaining code.

## Out of scope

- Any broad `PathBuf` interner or string-table wrapper beyond the delivered `PathId` and `FrozenStringTable` capabilities.
- Boxing `DiagnosticDraft` or changing `DiagnosticBag`/`InfrastructureErrors` layout — not needed for these clones.
- Second schedulers, benchmark-only fixtures as correctness coverage, or cross-crate counter plumbing beyond `benchmark_counters`.

## Validation

- Each code slice ends with `cargo test -p moth --lib`, `cargo run --quiet -p moth-cli --bin moth -- check docs --terse`, and five-run bench medians vs pre-slice baseline.
- Final slice runs `just validate` (per `docs/src/developer-docs/style-guide/validation.mtf`) and the `AGENTS.md` Slice review before the plan file is deleted in the completion commit.

## Sequencing

Blocked only on Slice 0 five-run benchmark evidence. The path-identity and frozen-table capabilities it depends on are delivered. Slices 1A-1B are ordered and each is one agent context.
