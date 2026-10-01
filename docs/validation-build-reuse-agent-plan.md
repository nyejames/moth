# Validation build reuse: agent verification plan

## Goal and authority

Test the validation changes on `validation-build-reuse`, based on
`numeric-type-expanding` at `987dcc3591f480c3577348eae4df6f5ef6664b49`.
Produce a report Nye can return for review. Measure improvement rather than
assuming the new configuration is faster or the cache has stopped growing.

Read `AGENTS.md`, the style and testing guides, and
`docs/src/developer-docs/style-guide/validation.mtf`. The validation guide owns
command policy. This plan owns the experiment and evidence, not another gate.
Perform the Slice review before handing back the result.

This PR was prepared without a Rust toolchain. Its Rust tests, Clippy, rustfmt,
Moth docs generation and full gate still require execution. Static file checks
are not evidence that the code builds. Keep the PR draft until these checks pass
and the measurements support accepting the defaults.

## Implemented scope

- One unpinned stable toolchain, including Clippy and rustfmt, remains selected by
  `rust-toolchain.toml`. CI uses rustup without a setup action disabling incremental builds.
- Dev and inherited test builds use debug information level zero and incremental
  compilation. Dependency optimisation and release/profiling settings are unchanged.
- `.cargo/config.toml` supplies four Cargo jobs, four Rust test threads and four
  Rayon threads. Explicit caller overrides remain available.
- Both workspace packages inherit the same deny-warnings policy. Ordinary
  `just clippy` and `just fmt` invoke the corresponding bare Cargo command.
  Broader package/target/feature coverage has explicit names and remains in CI/full validation.
- `validate` is routine correctness. `validate-perf` contains the unchanged
  non-recording benchmark and scaling lanes. `validate-full` adds all standard
  feature lanes, honesty audit, broad Clippy/formatting and timer erasure.
- Successful validation prunes only old Cargo timing HTML reports and reports
  a soft target budget. It preserves incremental state and build artifacts.
  Package-scoped dev/test eviction is an explicit, separate Cargo clean recipe.

The new full gate includes more work than the previous local gate. Report scope
reduction separately from compilation speedup. No release LTO change, benchmark
fixture reduction, iteration reduction or complexity-budget relaxation is included.

## Supplied baseline, not measurements from this PR

These numbers were supplied by Nye. They are contextual evidence, not a controlled
comparison with the candidate. The probes used rustc 1.98.1 on a 10-core M1 Pro
with the machine otherwise in normal use.

| Scenario | Reported wall time |
|---|---:|
| Previous `just validate`, cached, no Compiling lines | 109.8 s |
| Previous cold-ish validation after a toolchain update | 552.6 s |
| Reported sum of separately measured warm stages | 84.6 s |
| Fresh-target debug compiler build | 40.1 s |
| Fresh-target fat-LTO release compiler build | 163.1 s |
| Fresh-target broad-feature isolated Clippy | 37.8 s |

| Previous warm stage | Seconds |
|---|---:|
| ci-clippy-native | 0.27 |
| feature-lane-check | 3.9 |
| source-audit | 4.8 |
| first-party-deps | 1.5 |
| workspace tests, reported 6457 tests | 10.0 |
| integration, reported 2177 cases | 22.8 |
| docs check | 3.2 |
| bench-ci, 82 preflight cases and 18 quick cases times 3 | 14.9 |
| scaling, 11 cases and 3 series | 12.0 |
| erasure scan, cached 9.3 MB binary | 11.4 |

The attribution of about 340 seconds to two release builds was arithmetic from
probes and an observed run, not a measured per-stage breakdown. The difference
between the warm total and separately measured stages is unexplained. Do not
label it Cargo overhead without measuring it. Rounded stage values need not
sum exactly to the supplied total.

The earlier disk snapshot was about 51 GiB total: 38 GiB debug/deps, 10 GiB
incremental, 1.8 GiB isolated Clippy and about 546 MiB in the two release trees.
Nye has observed bounded memory with incremental compilation and all three
concurrency controls at four. Verify that result for the changed whole pipeline.

## Safety and experiment setup

1. Record the base and candidate SHAs, clean/dirty status, OS, architecture, CPU,
   RAM, available disk space, rustup active toolchain, rustc, Cargo, Clippy,
   rustfmt, just and the selected Node executable/version. Confirm Node supports
   native `Math.f16round` before interpreting integration failures.
2. Record only relevant environment overrides: `RUSTUP_TOOLCHAIN`, `RUSTC`,
   `RUSTC_WRAPPER`, `RUSTC_WORKSPACE_WRAPPER`, `RUSTFLAGS`,
   `CARGO_ENCODED_RUSTFLAGS`, `CARGO_BUILD_JOBS`, `CARGO_BUILD_TARGET`,
   `CARGO_TARGET_DIR`, `CARGO_BUILD_BUILD_DIR`, `CARGO_INCREMENTAL`, dev/test
   profile overrides, `RUST_TEST_THREADS`, `RAYON_NUM_THREADS` and
   `MOTH_CACHE_BUDGET_GIB`. Never dump credentials or the complete environment.
3. Use the same installed toolchain for baseline and candidate measurements.
   Update before both campaigns, not between them. Keep background workloads
   comparable and stop other Cargo users, including editor checks, during runs
   and maintenance. A four-job setting does not mean rustc uses only four cores.
4. Use separate worktrees with their normal repository-local target directories.
   Existing benchmark/profiling helpers assume those paths. Do not use a shared
   or relocated target for the full campaign. Cache-only custom-target testing
   is separately permitted below.
5. Store raw evidence under `tmp/validation-build-reuse/`, outside `target`.
   Nye has authorised a one-time `cargo clean` before the new configuration.
   Confirm the selected worktree and target before that clean. Never clean a
   different worktree or discard someone else's changes.
6. After the initial clean, retain the target throughout the warm/edit/growth
   campaign. Keep Cargo.lock, toolchain and environment unchanged. Record every
   experiment that changes these inputs rather than mixing its data silently.

## Phase 1: correctness and command consistency

- [ ] Run `cargo fmt --all --check`. Apply actual rustfmt output to touched Rust
  files and inspect it. Investigate unrelated drift rather than committing a
  tree-wide formatting rewrite.
- [ ] Run `cargo test -p xtask cache::tests -- --nocapture`. Check test discovery,
  exit status and all retention, overflow, link and idempotence assertions.
- [ ] Run `just --summary` and inspect `just --dry-run clippy`,
  `just --dry-run fmt`, `just --dry-run fmt-check` and
  `just --dry-run clippy-features`. There must be no hidden toolchain override,
  debug/incremental override or isolated Clippy target in these commands.
- [ ] Compare `cargo clippy` with `just clippy`, and `cargo fmt --check` with
  `just fmt-check`, on the same tree. Verify equal selected code and verdicts.
  Broader `clippy-features` and workspace formatting have intentionally different
  scope. They must still use the same toolchain and rules.
- [ ] Verify effective build jobs and child-process thread settings through Cargo
  diagnostics or a disposable probe. Test caller overrides of one without
  changing the committed defaults. Avoid process-global environment mutation
  inside concurrent Rust tests.
- [ ] Run `just validate` at the default four-thread settings. Retain exact suite
  counts and failures. Run the Rust suite again at one thread as a fallback check.
  New or exposed concurrency failures require a root-cause fix, not a blanket
  serialisation or ignored test.
- [ ] Run `just validate-full`. This includes the package-scoped default-feature
  lane, all standard lanes, the normal and broad Clippy checks, honesty audit,
  benchmark preflight, scaling and exact no-timer release scan. Preserve every
  existing failure contract. A missing feature lane is a failure.
- [ ] Run the documentation release build through the current compiler. Inspect
  changed routes and generated output. Update generated docs only through this
  build, and commit justified generated changes separately from measurement logs.
- [ ] Check Linux/macOS/Windows CI commands and the repository-toolchain install
  steps. Ordinary workflow triggers are unchanged: pushes to main or manual
  dispatch, not PR creation. Do not run the deployment workflow on this branch
  merely to get gate data. Use available non-deploying CI or an explicitly agreed
  workflow change. Mark unavailable platforms untested.

Treat every correction as a separate reviewed change. Repeat affected checks and
record the final measured candidate SHA. Never report timings from a different
revision as evidence for the final patch.

## Phase 2: cold, warm and after-edit measurements

Record wall time and exit code for every command, including failed attempts.
Keep raw stdout/stderr. On macOS, `/usr/bin/time -l` can provide process resource
statistics, but its reported peak is not a sum of concurrently resident child
processes. Record system memory pressure and swap separately. On Linux, record
`/usr/bin/time -v` and the monitoring method used. Do not compare unlike RSS metrics.

A suitable macOS capture, from the worktree root, is:

```bash
mkdir -p tmp/validation-build-reuse
/usr/bin/time -l just validate >tmp/validation-build-reuse/validate.log 2>&1
```

Capture the exit code immediately, before running another command. Use unique
log names for each sample. Keep resource monitoring external to benchmark code.

- [ ] Measure one clean routine validation. Do not let Phase 1's prerequisite
  builds masquerade as a cold run. Either capture the first run before those
  checks or label the later run warm/partially warm.
- [ ] Measure three unchanged routine validations. Report individual values and
  median, not only the fastest run. The full cache-maintenance cost belongs in
  the total. A warm run with rebuilds is not a steady-state sample.
- [ ] Record `validate-perf` and the remaining full-gate families separately.
  Distinguish building each release compiler from executing benchmarks or
  scanning the binary. No changed benchmark methodology is part of this PR.
- [ ] Use `cargo test --workspace --no-run --timings` to isolate compilation from
  execution. For freshness evidence, capture an otherwise equivalent
  `--message-format=json` invocation and inspect compiler-artifact `fresh` fields.
  Run diagnostic probes separately from the main wall-time samples.
- [ ] Make one representative, semantics-preserving Rust edit in the compiler
  and another in xtask, in separate experiments. Save the exact patches. Measure
  the first validation after each edit and the unchanged repeat. Restore only
  those patches and record the restoration rebuild too. Never add empty commits
  and call that an incremental compilation experiment.
- [ ] On any unexplained rebuild, rerun the affected command with
  `CARGO_LOG=cargo::core::compiler::fingerprint=info`. Capture which unit became
  dirty and why. Required feature variants and compiler changes are distinct
  from unexpected invalidation or artifact deletion.
- [ ] Compare equivalent base/candidate commands with matched settings where
  practical. Report removal of release work from the routine path separately
  from faster builds. Compare new full validation only with a baseline that
  executes its additional feature-matrix/honesty/formatting work too.

The production release profile remains fat LTO with one codegen unit. Any later
thin-LTO/codegen-unit experiment needs separate build-plus-runtime evidence and
must not silently rewrite historical benchmark interpretation. The warm erasure
scan is also a separate optimisation opportunity, not an implemented claim here.

## Phase 3: maintenance safety and cache stability

- [ ] Snapshot target disk usage and important artifact hashes/mtimes before
  maintenance, run `just cache-maintain`, and snapshot again. Build artifacts,
  incremental files, benchmark history and current test reports must survive.
- [ ] Run validation, maintenance and unchanged validation. The final run must
  have no rebuild caused by maintenance. Repeat maintenance and check idempotence.
- [ ] In a disposable fixture, check retention with more than ten old reports,
  recent reports, future timestamps, the latest alias, unknown filenames,
  directories resembling reports and links. Only recognised old regular timing
  reports outside the retained newest ten may disappear. Exercise Windows
  junction/reparse-point handling on Windows, not only Unix symlinks.
- [ ] Test a target path containing spaces and a cache-only `CARGO_TARGET_DIR`
  override in a disposable worktree. Confirm metadata determines maintenance's
  selected target. Never use this override to run existing benchmark builders.
- [ ] Test invalid budgets such as zero, text and an overflowing integer. The
  command must fail without deleting anything. Test exceeding a small valid
  soft budget with a disposable sparse file. It must warn without evicting
  artifacts. Logical byte counts can exceed allocated disk space for sparse
  files and hard links. Keep both measurements labelled.
- [ ] Run `cache-evict-preview` and explicit `cache-evict` only on a disposable
  populated target with no active Cargo users. Verify that package-scoped dev
  cleanup leaves unrelated dependencies and release artifacts, and document the
  expected next-build cost. Do not include this eviction in routine stability runs.
- [ ] Confirm a deliberately failing gate returns failure and does not proceed
  to maintenance. Do not run `release` or `ship` to test this: they have external
  or recording side effects. Inspect their full-gate dependencies instead.
- [ ] Run ten unchanged routine validations, retaining caches, then five edit
  and restore cycles. Record disk usage after every run: total, debug/deps,
  debug/incremental, release and any isolated trees. Record compiler freshness
  and maintenance duration alongside size. A single small target is not evidence
  that long-term growth is bounded.
- [ ] Explain any monotonic growth: new supported feature variants, toolchain
  changes, accumulating stale artifacts, reports or generated outputs. A soft
  warning is not a hard cap. Do not claim this implementation prunes all stale
  artifacts or prevents every possible source of target growth.

## Required report and return package

Create `tmp/validation-build-reuse/report.md` and `measurements.csv`, with raw
logs and copies of current machine-readable reports beside them. Keep this data
untracked unless Nye explicitly asks to publish it. Link the report in the PR
and hand back its contents or a downloadable copy.

The report must contain:

1. **Revisions and environment.** Exact base/final candidate, hardware, tools,
   runtime, overrides, background workload and monitoring method.
2. **Correctness and coverage.** Every required command, exit status, suite count,
   tested platform, pending platform and any fixes made. State whether full
   validation and documentation generation actually passed.
3. **Timing evidence.** Every cold, warm and after-edit sample, median, compile
   versus execution evidence, and comparable baseline scope. Explain rather
   than hide omitted work or failed samples.
4. **Reuse and cache evidence.** Dirty-unit reasons, cleanup survival checks,
   per-run allocated and logical sizes, incremental growth, and explicit
   eviction behaviour. Include the validation-cleanup-validation result.
5. **Memory and consistency.** Peak/pressure/swap observations, four-thread and
   one-thread outcomes, and direct-Cargo versus just toolchain/scope parity.
6. **Findings and decision.** Regressions with reproduction commands, unresolved
   risks, what improved with evidence, what remains unproven and whether to
   accept the defaults. Separate future LTO/scanner/cache work from required fixes.

Use CSV columns:
`revision,scenario,run,command,exit_code,wall_seconds,compile_seconds,compile_evidence,target_allocated_bytes,target_logical_bytes,incremental_bytes,memory_method,peak_memory_bytes,swap_change_bytes,notes`.
Leave unavailable metrics empty with an explanation. Never fill unknown timings
with arithmetic estimates without labelling them as estimates in the report.
