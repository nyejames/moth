# Moth benchmark correctness and performance sanity suite implementation plan

## Repository baseline

- Repository: `nyejames/moth`
- Defect-discovery commit: `06f6425f3c8161e7ee464afb1a3d47edb50725bc`
- Reviewed `main` head: `021a2e680a93669d968cd0d6b731dee1a6cab25c`
- Difference after the defect-discovery commit: README-only. The benchmark, CLI and compiler code reviewed by this plan is unchanged.

This plan fixes the benchmark fixtures and compiler regressions exposed by the new validation, then replaces the benchmark harness paths that allowed failed compilations to be timed and recorded as successful results.

Do not update benchmark history, tracked summaries or performance baselines until every correctness phase in this plan is complete and all benchmark cases pass preflight.

---

## Agent operating rules

### Read these files before editing

Read the current repository versions, not remembered or copied versions:

- `README.md`
- `docs/language-overview.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `benchmarks/README.md`
- `justfile`
- `Cargo.toml`
- `xtask/Cargo.toml`

For each compiler failure, also read the nearest implementation modules and existing tests named in that phase before changing code.

### Hard implementation constraints

- Preserve the language semantics in the authority documents.
- Treat optional inferred transfer as an optimisation. Missing or path-dependent transfer proof must fall back to borrowing and must not reject otherwise valid source.
- Keep borrow validation side-table based. Do not mutate HIR to solve these failures.
- Keep user-facing failures as structured `CompilerDiagnostic` values.
- Keep internal and tooling failures as `CompilerError` or typed xtask errors.
- Do not parse normal rendered diagnostic prose as a machine protocol.
- Do not special-case benchmark paths, benchmark case IDs or compiler-generated local names such as `__hir_tmp_*`.
- Do not retain compatibility wrappers for `cases.txt`, `frontend-cases.txt` or the current sanitised path-derived case names.
- Do not add lint allowances, ignore failing tests or weaken assertions.
- Do not add a new dependency when the repository already has a suitable direct dependency. Adding direct `serde` and `toml` dependencies to `xtask` is allowed because the workspace already uses them and the new manifest/history formats require them.
- Use modern idiomatic Rust, explicit enums for meaningful states and data-oriented vectors/maps rather than trait-heavy orchestration.
- Keep benchmark cases deterministic and serial unless an existing suite explicitly tests frontend parallel scheduling.

### Mandatory stop protocol

Stop work immediately when any of these conditions occurs:

1. The checked-out commit differs materially from the baseline above in a file this plan changes.
2. A current authority document contradicts a required semantic change in this plan.
3. A minimal regression case does not reproduce the reported failure.
4. A reported compiler failure disappears after an earlier fix. Do not implement the later speculative fix.
5. A proposed fix would require changing source syntax, public language semantics or a stable diagnostic contract not authorised by the current docs.
6. An existing integration contract already has a primary owner and the correct role for a new case is unclear.
7. A workload's exact source/input inventory cannot be represented without guessing.
8. A benchmark record or tracked summary cannot be identified with certainty during cleanup.
9. The agent cannot run a clean recorded benchmark on the same machine that owns the tracked summary. Never fabricate timing values.
10. `just validate` exposes an unrelated existing failure that cannot be shown to result from this work.

When stopping, provide only:

- the phase and substep
- the command that was run
- the exact observed diagnostic or code evidence
- the authority or implementation conflict
- the smallest decision required from the user
- the last completed checkpoint

Do not continue with a guessed interpretation.

---

## Current failures that must be resolved

The supplied validation contains twelve failed entries, representing six underlying workloads:

1. `benchmarks/speed-test.moth`
   - `MOTH-SYNTAX-0031`
   - negative integer match arm `-1 =>` is being interpreted as binary subtraction requiring whitespace after `-`

2. `benchmarks/fold-stress.moth`
   - `MOTH-TYPE-0001`
   - a chained const expression containing `Int / Int` is incorrectly expected to remain `Int` even though `/` produces `Float`

3. `benchmarks/pattern-stress.moth`
   - `MOTH-RULE-0034`
   - `nested_match()` reads top-level runtime bindings `outer` and `inner` that are not in function scope

4. `benchmarks/adversarial/expression-rpn-churn.moth`
   - `MOTH-BORROW-0008`
   - optional transfer state differs across control-flow paths for `op_name`

5. `benchmarks/module-graph/components/listing.moth`
   - `MOTH-BORROW-0007`
   - compiler-generated temporary is reported as aliasing the collection parameter

6. `benchmarks/adversarial/import-external-churn/#page.moth`
   - `MOTH-BORROW-0007`
   - compiler-generated temporary is reported as aliasing the collection parameter

The CLI and frontend duplicates of the same workload are not separate compiler bugs.

---

## Target end state

At completion:

- failed `moth check` and `moth build` commands return a non-zero process status
- warnings-only commands remain successful
- benchmark subprocesses receive one stable machine status record and fail closed when it is missing or malformed
- every benchmark mode preflights its selected cases before measurement
- `bench-validate` is a thin caller of the shared preflight path, not a second compiler invocation implementation
- `bench`, `bench-check`, frontend benchmark modes and profiling use the same case execution contract
- benchmark cases come from one typed TOML manifest with authored stable IDs
- case identity no longer changes when a path, extension or compiler name changes
- workload changes are fingerprinted and excluded from direct before/after speed comparisons
- failed, warning-producing or malformed runs cannot update local history or tracked summaries
- `just validate` runs all benchmark-tool tests and one bounded quick benchmark gate
- full CLI and frontend suites remain available for deliberate performance work
- the contaminated July 25 records are removed and a clean baseline is recorded only after all validation passes

---

# Phase 0: Baseline and evidence preservation

## 0.1 Confirm repository state

Run:

```bash
git rev-parse HEAD
git status --short
git diff --stat 06f6425f3c8161e7ee464afb1a3d47edb50725bc..HEAD
```

Expected:

- HEAD is `021a2e680a93669d968cd0d6b731dee1a6cab25c` or a descendant whose relevant files have not changed
- only the known README-only commit follows `06f6425f...`
- the working tree is clean before edits

Stop if relevant files changed after this review.

## 0.2 Capture the failing baseline

Run the existing commands without recording history:

```bash
cargo test --workspace --quiet -- --format terse
cargo run --package xtask --bin xtask -- bench-validate
```

Then run the current full validation only to capture its failure shape:

```bash
just validate
```

Do not run `just bench` or `just bench-frontend`.

Save the complete benchmark failure list in the work log. The expected count is twelve entries matching the six workloads above. If the set differs, stop and report the new set before editing.

## 0.3 Check whether xtask tests are currently included

Run:

```bash
cargo test --quiet -- --list | rg 'xtask|bench_|case_parser|process_runner'
cargo test --workspace --quiet -- --list | rg 'bench_|case_parser|process_runner'
```

Record the difference. This establishes whether the root-package `cargo test` command omits `xtask` tests on the active Cargo configuration.

Checkpoint: no source edits yet.

---

# Phase 1: Correct CLI process status and add a stable benchmark status record

This phase fixes the root contract. Benchmark tooling must be able to trust process status rather than compensating for a CLI that reports compilation failure as success.

## 1.1 Add one internal command-status type

Create:

```text
src/projects/command_status.rs
```

Add a small internal enum:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandStatus {
    Success,
    Failure,
}
```

Implement conversion to `std::process::ExitCode` in this module. Use exit code `0` for success and `1` for failure.

Also add one benchmark-only diagnostic summary emitter in the same module or another narrowly named projects module if the file would otherwise mix unrelated concerns.

The exact machine line is:

```text
MOTH_BENCH status errors=<usize> warnings=<usize>
```

Emission rules:

- emit only when `MOTH_BENCH_STATUS=1`
- emit exactly once for every completed `check` or `build` command path that has diagnostic counts
- do not change ordinary user output when the environment variable is absent
- do not include prose, duration, colour or unstable wording in this line
- do not treat output-write or infrastructure failure as a compiler diagnostic count. Process status remains the authority for those failures

Expose only the minimum helper needed by `check` and `build`.

Add `pub(crate) mod command_status;` to the projects module declaration.

## 1.2 Make `check` return a status

Modify:

```text
src/projects/check.rs
```

Change `run_check` to return `CommandStatus`.

Required behaviour:

- compute `error_count` and `warning_count` before consuming messages for rendering
- preserve current diagnostic rendering and timer output
- emit the stable benchmark status line after ordinary output when enabled
- return `Failure` when `error_count > 0`
- return `Success` for zero errors, including warnings-only results
- invalid paths represented through compiler messages return `Failure`

Do not make warning-only source fail the normal CLI.

Keep `execute_check` as the compiler-facing operation used by unit tests. Do not expose rendered strings as the status API.

## 1.3 Make every CLI command return through one exit path

Modify:

```text
src/projects/cli.rs
src/main.rs
```

Required shape:

```rust
pub fn start_cli() -> std::process::ExitCode
```

and:

```rust
fn main() -> std::process::ExitCode {
    start_cli()
}
```

Remove direct `process::exit` calls from command branches. Map every branch to `CommandStatus`, then convert once at the top-level return.

Required status mapping:

- no arguments, help and version: success
- invalid standalone flags, unknown commands and argument parsing errors: failure
- successful project creation: success
- explicit user cancellation already identified by the existing `Cancelled project creation.` branch: success
- other project creation errors: failure
- successful build with or without warnings: success
- compiler/build diagnostics: failure
- output resolution or output writing failure: failure
- successful check with or without warnings: success
- check diagnostics: failure
- successful dev-server startup/termination: preserve current success semantics
- dev-server error: failure
- correct integration test summary: success
- incorrect integration test summary or runner infrastructure error: failure

For `build`:

- capture compiler message counts before rendering an error
- capture successful warning count before rendering warnings
- emit one `MOTH_BENCH status` line on compiler success or diagnosed compiler failure when enabled
- return failure for `build_project` errors and output-writing errors

Do not alter normal build artefact behaviour.

## 1.4 Add real subprocess regression tests

Add a root-package Rust integration test:

```text
tests/cli_exit_status.rs
```

Use only `std::process::Command`, `tempfile` and `env!("CARGO_BIN_EXE_moth")`. Do not invoke nested `cargo run` from the test.

Required tests:

1. valid single-file `check` exits 0
2. invalid-syntax `check` exits non-zero
3. warning-only `check` exits 0
4. valid project `build` exits 0
5. invalid-syntax project `build` exits non-zero
6. unknown command exits non-zero
7. unknown check/build flag exits non-zero
8. `--version` exits 0

Use temporary project directories. Do not depend on repository benchmark fixtures for these CLI contract tests.

The warning-only test may reuse the small warning pattern already used by `src/projects/tests/check_tests.rs`. Assert process status only unless output shape is the specific contract under test.

## 1.5 Add machine-status tests

Add focused unit tests beside the new status emitter and xtask parser added later. At this phase, test compiler-side emission through a pure formatting helper rather than global stdout capture.

Required assertions:

- exact clean line
- exact warning count line
- exact error count line
- no emission decision when the environment variable is absent
- only exact value `1` enables emission

## 1.6 Validate phase 1

Run:

```bash
cargo fmt
cargo test --workspace --quiet -- --format terse
cargo test --quiet --test cli_exit_status
```

Then manually verify:

```bash
cargo run --quiet -- check <invalid-temp-file> --terse
echo $?
```

Use the platform-equivalent exit-code command on Windows.

Expected: non-zero.

Do not continue until these tests pass.

Suggested checkpoint commit:

```text
cli: return failure status for diagnosed commands
```

---

# Phase 2: Fix the benchmark-exposed source and compiler regressions

Use the existing `bench-validate` command during this phase. Do not redesign the benchmark harness until the semantic failures are fixed and covered by canonical tests.

## 2.1 Correct the invalid pattern-stress fixture

Modify:

```text
benchmarks/pattern-stress.moth
```

Required source correction:

```moth
nested_match |outer Int, inner Int| -> String:
    ...existing match body unchanged...
;
```

Remove the top-level runtime bindings:

```moth
outer = 10
inner = 5
```

Change the call to:

```moth
nested_match(10, 5)
```

Do not turn the values into constants. Passing runtime parameters preserves the intended nested pattern workload without relying on invalid function capture.

Run:

```bash
cargo run --quiet -- check benchmarks/pattern-stress.moth --terse
```

Expected: clean compilation after the change.

This is the only currently known benchmark-source correction. Treat the remaining failures as compiler regressions unless a minimal reduction proves otherwise.

## 2.2 Fix negative integer literals at match-arm starts

Affected fixture:

```text
benchmarks/speed-test.moth
```

Relevant implementation area:

```text
src/compiler_frontend/tokenizer/lexer.rs
src/compiler_frontend/tokenizer/tests/lexer_tests.rs
src/compiler_frontend/ast/pattern or match parsing modules discovered from the current code
```

Authority:

- `-1` is a signed integer literal in prefix position
- binary symbolic operators require surrounding whitespace
- a line-initial match pattern is a prefix position, not continuation of the preceding arm expression

Before changing implementation:

1. Create the smallest source reduction containing two match arms where a preceding arm can end an expression and the next arm begins `-1 =>`.
2. Confirm it reproduces `MOTH-SYNTAX-0031`.
3. Add a canonical integration case under `tests/cases/`.
4. Inspect `tests/cases/manifest.toml` for the existing pattern-matching contract family. Reuse that contract with the correct `boundary` or `adversarial` role when a primary already exists.

The integration case must execute or render a result for input `-1`, not only parse dead source.

Add a lexer-focused unit test proving that the `-1` pattern token is a negative integer literal after a previous arm and newline.

Implementation requirements:

- fix lexical/parser context at the real logical-line or pattern boundary
- preserve rejection of `a-1`
- preserve rejection of spaced unary negation `- value`
- preserve binary subtraction spacing diagnostics
- do not special-case the literal text `-1`
- do not rewrite the benchmark arm to `< 0`, `0 - 1` or another workaround

Stop if the only apparent fix would broadly reset expression context on every newline and multiline expression semantics are unclear.

Validate:

```bash
cargo test --workspace --quiet -- --format terse
cargo run --quiet -- tests --case <new-case-id>
cargo run --quiet -- check benchmarks/speed-test.moth --terse
cargo run --quiet -- build benchmarks/speed-test.moth
```

## 2.3 Fix chained const division type inference

Affected fixture:

```text
benchmarks/fold-stress.moth
```

Authority:

- `Int / Int -> Float`
- mixed `Int` and `Float` arithmetic produces `Float`
- `#=` infers the natural folded expression type
- there is no implicit `Float -> Int` coercion

Existing narrow unit evidence already proves the low-level fold helper can produce a `Float`. The missing coverage is the complete authored-expression typing and const-folding path.

Before changing implementation:

1. Reduce the failure to one inferred const containing `(100 / 4) * (200 / 5)`.
2. Add a typed use that requires the inferred constant to be `Float`.
3. Confirm the reduction fails with the same Int/Float mismatch.
4. Add a canonical integration case. Reuse an existing numeric or constant-folding contract family where appropriate.

Implementation investigation must trace:

- RPN/expression natural-type construction
- operator result-type propagation after `/`
- expected-type propagation into later multiplication
- const declaration inference
- folded expression replacement and retained type identity

Implementation requirements:

- the whole expression must naturally resolve to `Float`
- compile-time and runtime operator typing must agree
- do not change `/` to `//`
- do not add an explicit `Float` annotation solely to hide the bug
- do not insert `cast`
- do not weaken type mismatch diagnostics

Validate:

```bash
cargo run --quiet -- tests --case <new-case-id>
cargo run --quiet -- check benchmarks/fold-stress.moth --terse
cargo test --workspace --quiet -- --format terse
```

## 2.4 Separate optional transfer from source-legality state

Affected fixture:

```text
benchmarks/adversarial/expression-rpn-churn.moth
```

Primary implementation files to inspect:

```text
src/compiler_frontend/analysis/borrow_checker/state.rs
src/compiler_frontend/analysis/borrow_checker/engine.rs
src/compiler_frontend/analysis/borrow_checker/transfer/access/move_decision.rs
src/compiler_frontend/analysis/borrow_checker/transfer/access/statement.rs
src/compiler_frontend/analysis/borrow_checker/transfer/access.rs
src/compiler_frontend/analysis/borrow_checker/diagnostics.rs
src/compiler_frontend/analysis/borrow_checker/tests/
src/compiler_frontend/public_call_summary.rs
```

Current code facts that must be reviewed:

- `FutureUseKind::May` is described as path-dependent usage
- `MoveDecision` contains `Inconsistent`
- call transfer may already fall back to borrowing for `Inconsistent`
- `BorrowChecker::check_inconsistent_move_join` rejects a join where one incoming state is `UNINIT` and another is not
- optional move paths currently invalidate roots in the mandatory borrow state

Authority:

- inferred transfer is optional
- when proof is unavailable on every relevant path, the operation remains a borrow
- failure to prove transfer must not reject valid source
- borrow validation may emit optional transfer/drop facts but must not rewrite HIR

Required design correction:

1. Mandatory `BorrowState` must represent source-visible initialization and alias/access legality.
2. Optional destruction-responsibility transfer must be represented as advisory side-table information, not by making the source root unavailable on one path.
3. A path-dependent optional transfer candidate must conservatively remain borrowed.
4. A control-flow join must not diagnose only because an optional transfer was selected on one predecessor and not another.
5. Genuine source-visible uninitialized access and mandatory consuming semantics, if any exist, must remain diagnosed through a distinct path.

Do not implement this as only deleting the diagnostic call. First identify every place that calls `invalidate_root` for optional transfer and every consumer of the resulting state.

Required investigation sequence:

1. Search all emit sites and tests for `MOTH-BORROW-0008`, `invalid_access_after_possible_ownership_transfer`, `check_inconsistent_move_join`, `MoveDecision::Inconsistent` and optional transfer facts.
2. Classify each occurrence as:
   - optional optimisation
   - mandatory source-visible consumption
   - stale test for the old global-consistency rule
3. Add a minimal integration case reproducing the `op_name` control-flow shape.
4. Add focused state/transfer tests proving `FutureUseKind::May` and path-dependent calls fall back to borrow.
5. Change the analysis so optional transfer records an advisory fact without invalidating mandatory source state.
6. Remove or repurpose `check_inconsistent_move_join` only after no valid mandatory state depends on it.
7. Update or delete old expected-failure cases only when their sole contract is the obsolete optional-transfer rejection. Do not bulk-delete all `MOTH-BORROW-0008` coverage.
8. Preserve real use-after-uninitialized and alias-conflict diagnostics.

The minimal positive integration case must prove that:

- one path reaches a final-use candidate
- another path retains or bypasses the value
- the function remains valid because the compiler can borrow on all paths
- a later actual conflicting mutable access still fails in a separate negative case

Stop if any operation currently modelled by `invalidate_root` is claimed to be mandatory consumption but the authority documents do not define it.

Validate:

```bash
cargo test --workspace --quiet -- --format terse
cargo run --quiet -- tests --case <optional-transfer-positive-case>
cargo run --quiet -- tests --case <retained-real-conflict-case>
cargo run --quiet -- check benchmarks/adversarial/expression-rpn-churn.moth --terse
```

## 2.5 Re-run the two generated-temporary alias failures before changing HIR

After phase 2.4, run:

```bash
cargo run --quiet -- check benchmarks/module-graph --terse
cargo run --quiet -- build benchmarks/module-graph
cargo run --quiet -- check benchmarks/adversarial/import-external-churn --terse
cargo run --quiet -- build benchmarks/adversarial/import-external-churn
```

If both failures are gone, do not make an additional HIR or borrow-checker change. Add regression coverage for the formerly failing collection-loop shapes only where current integration coverage is insufficient.

If either `MOTH-BORROW-0007` failure remains, continue with phase 2.6.

## 2.6 Conditional residual fix for compiler-generated temporary lifetimes

Only perform this phase when phase 2.5 still reproduces a failure.

Relevant source shapes:

- immutable collection parameter or binding
- collection loop item alias
- mutation of a separate output accumulator
- source or cross-module function call inside template construction

Required reductions:

1. local single-file collection loop appending to an independent mutable string
2. cross-module loop over an imported nominal element type

Add canonical integration cases only after each reduction reproduces the same diagnostic family.

Investigation requirements:

- inspect generated HIR and side-table local origins for the exact reduction
- identify what `__hir_tmp_*` represents
- identify its region and last use
- determine whether the false conflict is created during HIR linearisation, jump-argument transfer, visibility kills, alias-root propagation or conflict checking
- compare the source location associated with the hidden local and the reported user location

Fix requirements:

- correct the owning stage's lifetime or alias scope
- end compiler-introduced temporary activity at the earliest semantically valid boundary
- preserve aliases that genuinely escape into a result
- preserve cross-module call summaries
- never match on hidden-local names
- never exempt all compiler-owned temporaries from conflict checking
- never suppress `MOTH-BORROW-0007` globally

Add a stage-local unit test when the bug is an HIR/borrow invariant and an integration case for each genuinely distinct local and cross-module boundary.

Validate the two benchmark projects and every retained negative alias test.

## 2.7 Complete semantic checkpoint

Run:

```bash
cargo fmt
cargo test --workspace --quiet -- --format terse
cargo run --quiet -- tests
cargo run --package xtask --bin xtask -- bench-validate
```

Expected: every existing CLI and frontend benchmark case compiles cleanly.

Do not proceed to benchmark harness replacement until this checkpoint is green.

Suggested checkpoint commit:

```text
compiler: fix benchmark-exposed frontend regressions
```

---

# Phase 3: Replace path-derived text case lists with one typed manifest

## 3.1 Add direct xtask dependencies

Modify:

```text
xtask/Cargo.toml
```

Add direct dependencies matching the workspace versions:

```toml
serde = { version = "1.0.228", features = ["derive"] }
toml = "1.1.2"
```

Keep `serde_json`.

Do not add a shell-word parser or another manifest library.

## 3.2 Introduce `benchmarks/manifest.toml`

Create one manifest owning all CLI and frontend cases.

Use this exact top-level shape:

```toml
schema = 1

[[workload]]
id = "speed_test"
entry = "benchmarks/speed-test.moth"
fingerprint_roots = ["benchmarks/speed-test.moth"]
fingerprint_excludes = []

[[case]]
id = "speed_test_check"
workload = "speed_test"
group = "core"
quick = false
expectation = "clean"

[case.runner]
kind = "cli"
command = "check"
args = []
```

Frontend form:

```toml
[[case]]
id = "type_stress_frontend"
workload = "type_stress"
group = "stress"
quick = true
expectation = "clean"

[case.runner]
kind = "frontend"
profile = "dev"
```

Manifest rules:

- `schema` is required and must equal `1`
- every workload and case ID is authored, stable and unique
- IDs use lowercase ASCII letters, digits and `_`
- IDs may not begin or end with `_`
- `group`, `quick` and `expectation` are required on every case
- schema 1 accepts only `expectation = "clean"`
- CLI runner commands are typed and limited to `check` and `build`
- frontend profile is typed and currently limited to `dev`
- unknown fields are rejected through `#[serde(deny_unknown_fields)]`
- cases preserve manifest order
- each case references one existing workload
- duplicate runner invocation plus workload combinations are rejected
- every workload has at least one case
- every workload entry path is repository-relative and exists
- absolute paths, `.` components, `..` components and platform prefixes are rejected
- canonicalised paths must remain within the repository root
- the entry must be covered by at least one fingerprint root
- fingerprint excludes must stay within a declared root
- missing excludes are allowed only for generated paths that do not exist yet

Use typed Rust enums. Do not retain `command: String` as the execution authority.

## 3.3 Implement manifest loading

Replace:

```text
xtask/src/case_parser.rs
```

with a focused module such as:

```text
xtask/src/benchmark_manifest.rs
```

Required types:

```rust
BenchmarkManifest
BenchmarkWorkload
BenchmarkCase
BenchmarkRunner
CliBenchmarkCommand
FrontendBenchmarkProfile
BenchmarkExpectation
```

Recommended runtime shape:

```rust
pub struct BenchmarkCase {
    pub id: String,
    pub workload_id: String,
    pub group_name: String,
    pub quick: bool,
    pub expectation: BenchmarkExpectation,
    pub runner: BenchmarkRunner,
}
```

Resolve workload references once during loading. Keep the immutable manifest data available to all benchmark modes.

Return typed errors carrying the manifest path and relevant ID. Render them at the xtask boundary.

Delete after migration:

```text
benchmarks/cases.txt
benchmarks/frontend-cases.txt
xtask/src/case_parser.rs
```

Do not keep a fallback parser.

## 3.4 Define exact stable case inventory

Migrate the existing cases to these IDs. Preserve current groups unless explicitly noted.

### CLI cases

Core:

- `root_single_file_check` -> check `benchmark-root-single-file.moth`, quick
- `speed_test_check` -> check `benchmarks/speed-test.moth`
- `speed_test_build` -> build `benchmarks/speed-test.moth`, quick

Docs:

- `docs_check` -> check `docs`, quick

Stress:

- `template_stress_check`
- `type_stress_check`
- `fold_stress_check`
- `pattern_stress_check`
- `collection_stress_check`
- `environment_stress_check`
- `one_module_kitchen_sink_check`, quick
- `deep_scope_churn_check`
- `template_render_plan_churn_check`
- `constant_dag_churn_check`
- `expression_rpn_churn_check`
- `generic_trait_churn_check`
- `collection_map_borrow_churn_check`

Module:

- `module_graph_check`
- `module_graph_build`, quick
- `import_fanout_check`
- `import_fanout_build`
- `external_js_imports_check`
- `external_js_imports_build`, quick
- `module_root_stress_check`
- `module_root_stress_build`
- `import_external_churn_check`
- `import_external_churn_build`, quick

Borrow:

- `borrow_stress_check`, quick

### Frontend cases

Core/docs:

- `type_stress_frontend`
- `docs_frontend`, quick

Stress:

- `template_stress_frontend`, quick
- `fold_stress_frontend`, quick
- `pattern_stress_frontend`
- `collection_stress_frontend`
- `environment_stress_frontend`
- `one_module_kitchen_sink_frontend`
- `deep_scope_churn_frontend`
- `template_render_plan_churn_frontend`
- `constant_dag_churn_frontend`
- `expression_rpn_churn_frontend`, quick
- `generic_trait_churn_frontend`
- `collection_map_borrow_churn_frontend`, quick

Module:

- `module_graph_frontend`, quick
- `import_fanout_frontend`
- `module_root_stress_frontend`
- `external_js_imports_frontend`
- `import_external_churn_frontend`, quick
- `module_root_role_mix_frontend`

Parallelism:

- `tiny_one_file_frontend`
- `tiny_two_files_frontend`
- `tiny_seven_files_frontend`
- `tiny_eight_files_frontend`
- `many_tiny_files_frontend`
- `many_medium_files_frontend`, quick
- `many_markdown_assets_frontend`
- `many_modules_one_file_each_frontend`, quick
- `few_modules_many_files_each_frontend`

Borrow:

- `borrow_stress_frontend`, quick

The three added frontend cases intentionally cover template, fold and pattern workloads that were absent from the current frontend list.

If one of these cases cannot run through the frontend API after its compiler fix, stop and report the exact API limitation. Do not silently omit it.

## 3.5 Define workload fingerprint roots

Use one workload record for each unique entry path. CLI check/build and frontend cases for the same source share a workload.

For file entries, use the file itself as the only fingerprint root.

For directory entries:

- inspect the workload's current config and source layout
- include config and every source/input directory that affects compilation
- explicitly exclude generated output roots such as `dev` and `release`
- do not include `target`, `.git` or `benchmarks/local-data`
- do not exclude authored assets used by the build

For `docs`, use explicit authored roots such as `docs/config.moth`, `docs/src` and other authored input directories. Do not include `docs/release`.

Stop if a directory workload's input boundary cannot be stated precisely from current config and source ownership.

## 3.6 Add manifest tests

Create tests beside `benchmark_manifest.rs`.

Required tests:

- valid manifest loads in source order
- duplicate workload ID fails
- duplicate case ID fails
- unknown workload reference fails
- duplicate invocation fails
- invalid ID spelling fails
- unknown runner kind fails
- unknown CLI command fails
- unknown frontend profile fails
- unknown field fails
- missing required field fails
- absolute, parent and current-directory paths fail
- entry outside repository fails after canonicalisation
- empty workload and empty case inventories fail
- uncovered entry path fails
- invalid exclude path fails

Use `tempfile::tempdir`. Avoid fixed names under the global system temp directory.

## 3.7 Validate phase 3

At this checkpoint, adapt existing benchmark modes only enough to load the new manifest while preserving behaviour. Do not yet add duplicate execution logic.

Run:

```bash
cargo fmt
cargo test --workspace --quiet -- --format terse
cargo run --package xtask --bin xtask -- bench-validate
```

Expected manifest counts after adding the three frontend cases:

- 28 CLI cases
- 30 frontend cases
- 58 total cases

Suggested checkpoint commit:

```text
bench: replace text case lists with typed manifest
```

---

# Phase 4: Add deterministic workload fingerprints and protocol identity

## 4.1 Add a stable fingerprint implementation

Create a focused xtask module such as:

```text
xtask/src/workload_fingerprint.rs
```

Do not use `DefaultHasher` because its stability is not a persistence contract.

Implement a small explicitly versioned non-cryptographic fingerprint builder. A two-lane FNV-1a 64-bit implementation producing 128 bits is sufficient. Document that it is for deterministic change detection, not security.

Hash length-prefixed fields to avoid concatenation ambiguity.

Include:

- manifest schema version
- benchmark protocol version
- runner kind
- CLI command or frontend profile
- ordered runner args
- workload entry path
- ordered fingerprint root paths
- ordered exclude paths
- every included file's repository-relative path
- every included file's bytes

Directory traversal rules:

- canonicalise repository root once
- canonicalise each root
- reject roots outside the repository
- walk recursively
- sort entries by normalised repository-relative path before hashing
- ignore timestamps, permissions and directory enumeration order
- apply exact path-prefix excludes
- reject unreadable files
- reject an empty resulting file set
- reject symlinks escaping the repository
- if in-repository symlinks are supported, hash the logical path plus resolved file bytes and add a test. Otherwise reject all symlinks with a clear manifest error

Compute the fingerprint once per workload before preflight. Reuse it for every case sharing the workload and throughout the run.

## 4.2 Introduce an explicit benchmark protocol version

Add:

```rust
pub const BENCHMARK_PROTOCOL_VERSION: u32 = 1;
```

Place it in the benchmark domain module that owns persisted measurement semantics.

The version covers:

- one mandatory preflight serving as warmup
- exact status-record validation
- required timer validation
- iteration counts
- case identity and workload fingerprint semantics

Increment it in future only when measurement methodology changes enough that direct comparison is invalid.

## 4.3 Add fingerprint tests

Required tests:

- same files in different directory enumeration order produce the same fingerprint
- changing file bytes changes the fingerprint
- renaming a file changes the fingerprint
- changing runner command/profile/args changes the fingerprint
- excluded output changes do not change the fingerprint
- adding an included file changes the fingerprint
- missing root fails
- empty included file set fails
- path escape and symlink escape fail

---

# Phase 5: Replace duplicate validation paths with one shared case executor

## 5.1 Add typed process status

Modify:

```text
xtask/src/process_runner.rs
xtask/src/process_runner/tests.rs
```

Replace the loose `success: bool` authority with a typed status record that retains:

- `success`
- `code: Option<i32>`

Keep wall-clock duration, stdout and stderr.

Every Moth benchmark subprocess must receive:

```text
MOTH_TIMERS=bench
MOTH_COUNTERS=off
MOTH_BENCH_STATUS=1
```

Use the exact compiler binary supplied by the caller. Never invoke `cargo run` per case.

Update process-runner tests to use `tempfile::tempdir` rather than shared fixed temp filenames. Retain Unix and Windows coverage.

## 5.2 Add strict status-line parsing

Create a pure xtask parser for:

```text
MOTH_BENCH status errors=<usize> warnings=<usize>
```

Rules:

- zero matching lines is an error for a successful CLI benchmark run
- more than one matching line is an error
- any line beginning `MOTH_BENCH status` but not matching the exact grammar is an error
- unknown fields, negative values, overflow and trailing prose are errors
- ANSI stripping is unnecessary because the stable line must not contain ANSI output

Required tests:

- clean record
- warnings record
- errors record
- missing record
- duplicate record
- malformed prefix record
- unknown field
- overflow
- unrelated output before and after a valid record

## 5.3 Extend the frontend benchmark report with warning facts

Modify:

```text
src/benchmarking/frontend.rs
src/benchmarking/tests.rs
xtask/src/frontend_bench.rs
xtask/src/frontend_bench/tests.rs
```

On successful frontend compilation, collect frontend warnings from the compiled modules instead of replacing them with an empty message set.

Add to `FrontendBenchmarkReport`:

```rust
pub warning_count: usize,
pub warning_codes: Vec<String>,
```

Required behaviour:

- compiler errors still return `FrontendBenchmarkError`
- warnings return a successful report with count/codes
- clean benchmark expectations reject a non-zero warning count in the xtask executor
- normal frontend compiler semantics are unchanged

Add a warning-only frontend benchmark API test.

## 5.4 Introduce the shared executor

Create:

```text
xtask/src/benchmark_execution.rs
```

This module owns one execution of one manifest case. It does not own statistics, history or summary rendering.

Required types:

```rust
BenchmarkExecutionContext
BenchmarkCaseExecution
BenchmarkCaseFailure
BenchmarkFailureKind
BenchmarkDiagnosticStatus
```

`BenchmarkFailureKind` must distinguish at least:

- process spawn failure
- non-zero process status
- missing/malformed machine status
- clean expectation violated by errors
- clean expectation violated by warnings
- frontend compilation failure
- invalid/non-positive total duration
- malformed or missing required timer observations
- workload/manifest infrastructure failure

For CLI cases:

1. build args from the typed runner, workload entry and manifest args
2. invoke the supplied compiler binary with the declared `check` or `build` command
3. parse the stable status line
4. fail on non-zero process status
5. fail if a successful process reports non-zero errors
6. fail if a clean case reports warnings
7. parse timer observations through the checked parser in phase 6
8. return typed execution data

For frontend cases:

1. call the existing in-process frontend benchmark API with the declared profile
2. fail on compiler error
3. fail on warnings for a clean case
4. validate positive finite total time
5. validate stage observations
6. return the same typed execution shape

Failure rendering must include:

- stable case ID
- workload ID
- runner/command
- entry path
- exit code when available
- diagnostic status when available
- terse compiler diagnostic lines or a bounded stdout/stderr excerpt

Do not concatenate stdout and stderr without a separator.

## 5.5 Implement shared aggregate preflight

Add:

```rust
pub fn preflight_cases(
    context: &BenchmarkExecutionContext,
    cases: &[BenchmarkCase],
) -> Result<Vec<BenchmarkCaseExecution>, Vec<BenchmarkCaseFailure>>
```

Rules:

- execute each selected case exactly once
- continue after ordinary case failure so all broken cases are reported together
- preserve manifest order in output and failures
- do not measure or record any suite if preflight has one or more failures
- the successful preflight execution serves as the suite's one warmup
- do not write history or summaries

## 5.6 Replace `bench_validate.rs`

Delete the current implementation that:

- invokes `cargo run` once per CLI case
- always runs `check`
- parses ordinary terse output

Either delete `xtask/src/bench_validate.rs` completely and dispatch to the shared preflight module, or retain it only as a thin orchestration wrapper with no process or compiler logic.

`bench-validate` must:

- load the manifest once
- build the release compiler with timers once
- preflight all 58 cases
- print all failures in manifest order
- return non-zero on any failure

## 5.7 Use the shared executor in every mode

Refactor:

```text
xtask/src/bench.rs
xtask/src/frontend_bench.rs
xtask/src/profile/observations.rs
xtask/src/profile/mod.rs
```

Requirements:

- full CLI modes preflight all selected CLI cases, then measure them
- frontend modes preflight all selected frontend cases, then measure them
- profile mode preflights the selected CLI case using the exact profiling binary before observation/Samply work
- warmup and observation helpers call the shared executor or a shared lower-level execution primitive
- measured iterations revalidate process status, diagnostic status, warnings and timer protocol every time
- any measured-iteration failure aborts the complete suite and writes no run record
- profile mode writes no history record when preflight, observation or Samply fails
- frontend cases cannot be selected for Samply CLI profiling. Return a clear typed error

Do not duplicate case validation in the orchestrators.

## 5.8 Executor tests

Add mock executable tests proving:

- a declared `build` case invokes `build`, not `check`
- a declared `check` case invokes `check`
- entry path and args are passed in deterministic order
- exit 0 plus clean status succeeds
- exit 0 plus warning status fails a clean case
- exit 0 plus error status fails closed
- exit 0 with missing status fails
- exit 0 with malformed/duplicate status fails
- non-zero exit fails even when the status line says clean
- preflight aggregates multiple failures in manifest order
- a failed preflight prevents measurement callback invocation
- a failed measured iteration prevents history callback invocation

Prefer pure argument-construction tests plus small mock executable tests. Do not introduce shell parsing into production code.

Suggested checkpoint commit:

```text
bench: unify case preflight and execution
```

---

# Phase 6: Make timing observations fail closed

## 6.1 Replace permissive observation parsing

Modify:

```text
xtask/src/bench_observations.rs
xtask/src/bench_observations/tests.rs
```

Add a checked parser returning `Result<BenchmarkCaseObservations, BenchmarkObservationError>`.

Requirements:

- a line beginning `MOTH_BENCH timing` but failing the exact metric grammar is an error
- metric names must be non-empty
- values must parse, be finite and be non-negative
- stable metrics take precedence over legacy prose only for old-history parsing, not live execution
- live execution requires stable lines and must not accept only legacy human prose
- repeated metric names within one iteration may be summed because module-level stages can repeat
- a successful CLI `check` execution requires `command.check.total`
- a successful CLI `build` execution requires `command.build.total`
- timer-enabled frontend reports require at least one stage and finite non-negative stage values

If a documented top-level frontend metric already exists, require it. If no stable top-level metric exists, do not guess a name. The returned `total_ms` plus non-empty stage set is the minimum required frontend contract.

## 6.2 Enforce iteration consistency

Before averaging observations for one case:

- reduce duplicate metric names within each iteration
- compare the complete timing metric-name set across all measured iterations
- reject missing or additional timing metrics in a later iteration
- validate counters when present, but do not require counters in normal runs
- require total duration to be finite and greater than zero

Change `average_observations` to return `Result` or require a prior validated collection type that makes inconsistent sets impossible.

Do not silently average a metric across only the iterations where it appeared.

## 6.3 Add observation tests

Required tests:

- exact stable check/build metrics parse
- malformed stable timing line fails
- non-finite and negative values fail
- required total metric missing fails
- repeated per-module metric names sum within one iteration
- stable metric-name set mismatch across iterations fails
- consistent sets average correctly
- legacy prose remains readable only through old-history compatibility, not live execution

---

# Phase 7: Rework benchmark domain identity, comparison and history

## 7.1 Rename path-derived identity fields

Modify benchmark domain types so the authority is `case_id`, not `case_name` generated from command/path.

Thread through:

```text
xtask/src/bench_types.rs
xtask/src/bench.rs
xtask/src/frontend_bench.rs
xtask/src/bench_history.rs
xtask/src/bench_report.rs
xtask/src/bench_summary.rs
xtask/src/profile/
```

Each `BenchmarkCaseResult` must retain:

- `case_id`
- `workload_id`
- `workload_fingerprint`
- group
- runner identity
- command/profile and args for local reporting
- timing statistics
- observations

Artifact directory names may use the already validated safe case ID directly. Remove sanitisation helpers.

## 7.2 Replace manual JSON with serde-backed JSONL

`xtask` already directly depends on `serde_json`. After adding `serde`, remove the handwritten JSON serializer/parser from `bench_history.rs`.

Set:

```rust
const FORMAT_VERSION: u32 = 6;
```

Use serde-derived current records. For compatibility:

- parse each line to `serde_json::Value`
- inspect `format_version`
- adapt v1 through v5 through small explicit legacy structs or migration functions
- deserialize v6 directly
- keep current behaviour of skipping malformed or unknown-future lines with a warning
- old records receive protocol version `0`
- old cases receive no workload fingerprint
- old path-derived names remain historical data only and never become new stable IDs

Current v6 run records must add:

- `benchmark_protocol_version`
- `git_dirty`
- case workload ID
- case workload fingerprint

Replace `get_commit_hash()` with one helper that returns:

```rust
GitRevision {
    commit: Option<String>,
    dirty: Option<bool>,
}
```

Use `git rev-parse --short HEAD` and `git status --porcelain`. Do not fail benchmarking only because git metadata is unavailable.

## 7.3 Filter previous runs by protocol

Update `find_latest_matching_run` to require:

- system UUID
- suite kind
- thread identity
- current benchmark protocol version

Old protocol-0 records must not be direct baselines for protocol-1 measurements.

## 7.4 Make comparison workload-aware

Comparison matching sequence:

1. match by stable case ID
2. when IDs match, compare workload fingerprints
3. only equal fingerprints enter timing delta classification
4. differing fingerprints are reported as workload changes
5. added/removed IDs remain case-set changes

Add fields such as:

- `workload_changed_case_count`
- ordered workload-changed case IDs

Formatting requirements:

- never present a speed delta for a changed workload
- when some cases are comparable, report the delta only over comparable cases
- distinguish `case set changed` from `workload changed`
- preserve faster/slower/mixed classification for unchanged comparable cases
- keep thresholds unchanged in this work

For quick non-recording runs, filter the previous full-run case list to the current quick IDs before comparison. Intentional quick selection must not be reported as a removed case set.

## 7.5 History and comparison tests

Required tests:

- v6 roundtrip includes protocol, dirty state and fingerprints
- v1 through v5 compatibility remains readable
- old records do not match protocol 1
- stable ID plus equal fingerprint compares
- stable ID plus changed fingerprint is excluded and reported
- renamed path with unchanged stable ID and unchanged declared workload fingerprint remains comparable
- changed runner args change fingerprint and prevent comparison
- quick subset comparison filters the previous set correctly
- no invalid run is appended when preflight or measurement fails
- unknown future history version is skipped

Suggested checkpoint commit:

```text
bench: make history stable and workload aware
```

---

# Phase 8: Add the bounded `bench-ci` development gate

## 8.1 Add a new xtask mode

Modify:

```text
xtask/src/mode.rs
xtask/src/mode/tests.rs
xtask/src/main.rs
```

Add:

```text
bench-ci
```

`bench-ci` behaviour:

1. load the manifest once
2. build the release timer-enabled compiler once
3. compute workload fingerprints once
4. preflight all 58 cases once
5. stop and report all failures if any preflight fails
6. measure only cases with `quick = true`
7. run three measured iterations per quick case
8. write no local history or tracked summaries
9. print separate CLI and frontend result sections
10. compare quick cases against matching current-protocol local full baselines when available
11. print absolute metrics when no matching baseline exists

The preflight execution is the one warmup. Do not add another warmup loop.

Full modes keep ten measured iterations.

## 8.2 Refactor options around mandatory preflight

Current options expose a freely chosen warmup count. Replace or constrain this so normal modes cannot bypass preflight.

Recommended shape:

```rust
pub struct BenchmarkRunPolicy {
    pub measured_iterations: NonZeroUsize,
    pub selection: BenchmarkSelection,
    pub recording: BenchmarkRecording,
}
```

Where:

```rust
BenchmarkSelection::Full
BenchmarkSelection::Quick
BenchmarkRecording::Record
BenchmarkRecording::ReadOnly
```

Preflight is mandatory and outside the configurable iteration count.

Persist `warmup_runs = 1` for protocol 1 history, or rename the persisted field in v6 to `preflight_runs` and explicitly migrate older records. Do not leave a misleading configurable warmup field.

## 8.3 Update `justfile`

Add:

```make
bench-ci:
    cargo run --package xtask --bin xtask -- bench-ci
```

Replace the three benchmark commands currently inside `validate` with one:

```make
@echo "benchmark sanity"
cargo run --package xtask --bin xtask -- bench-ci
```

Keep standalone recipes:

- `bench-check`
- `bench-frontend-check`
- `bench-validate`
- `bench`
- `bench-frontend`
- profile commands

## 8.4 Ensure the validation gate includes xtask

Change unit tests in `validate` to:

```bash
cargo test --workspace --quiet -- --format terse
```

Update native Clippy to include the complete workspace:

```bash
cargo +1.95.0 clippy --workspace --all-targets --all-features -- -D warnings
```

Attempt the same `--workspace` coverage for Linux and Windows cross-target Clippy.

If xtask is genuinely host-only and cross-target compilation fails for a justified platform reason, stop and report the exact dependency/module failure. Do not silently exclude xtask. At minimum, native workspace Clippy and workspace tests are mandatory.

## 8.5 Validate phase 8

Run:

```bash
cargo fmt
cargo test --workspace --quiet -- --format terse
cargo run --package xtask --bin xtask -- bench-validate
cargo run --package xtask --bin xtask -- bench-ci
```

Confirm:

- `bench-validate` executes 58 preflights and no measurements
- `bench-ci` executes 58 preflights, then only quick measurements
- no history or summary files change
- a deliberately broken temporary benchmark source causes preflight failure before any timing summary

Restore the temporary source immediately and verify the working tree.

---

# Phase 9: Documentation and validation-authority updates

## 9.1 Update benchmark documentation

Modify:

```text
benchmarks/README.md
```

Document:

- `benchmarks/manifest.toml`
- stable case/workload IDs
- CLI and frontend runner forms
- clean expectation
- quick flag
- fingerprint roots/excludes
- preflight as mandatory warmup
- `bench-ci` versus full check/record modes
- machine status and required timer protocol
- workload-changed comparison semantics
- profile selection by stable case ID
- how to add a case
- prohibition on fixing compiler bugs by weakening benchmark source
- non-recording commands never change history

Remove documentation for `cases.txt` and `frontend-cases.txt`.

## 9.2 Update validation documentation

Modify:

```text
docs/src/docs/codebase/style-guide/validation.mtf
```

Update the executable gate to match the `justfile` exactly:

- workspace Rust tests
- one `bench-ci` sanity gate
- explain that `bench-ci` validates every benchmark case once, then measures the quick subset without recording
- retain full `bench-check` and frontend check as targeted performance commands

Do not claim that `just validate` runs the full ten-iteration suites after this change.

## 9.3 Review progress and architecture docs

Review but do not automatically edit:

```text
docs/src/docs/progress/#page.moth
docs/compiler-design-overview.md
docs/build-system-design.md
```

Expected result: no progress-matrix or core architecture status change is needed. The implementation is being corrected to match existing command and optional-transfer contracts.

Edit only if the current text contains a now-false executable command description. Do not add benchmark implementation detail to the compiler architecture authority.

---

# Phase 10: Remove contaminated benchmark history and record a clean baseline

Perform this phase only after phases 1 through 9 pass all non-recording validation. Before editing either local history or the tracked summary, confirm that the agent is running on the same machine that owns the tracked system summary and can immediately run both recording commands. If that cannot be confirmed, stop with phase 10 untouched and ask the user to perform the recording handoff.

## 10.1 Back up local history

If present:

```bash
cp benchmarks/local-data/runs.jsonl benchmarks/local-data/runs.jsonl.pre-benchmark-fix
```

The backup remains local and untracked.

## 10.2 Remove invalid local records

Inspect local JSONL records around July 25, 2026.

Remove only records corresponding to the invalid early-exit and post-validator runs represented by the tracked entries:

- July 25 at 07:54
- July 25 at 11:00

Match by timestamp, suite, system and commit where available. Do not delete by line number alone.

Stop if exact records cannot be identified.

## 10.3 Remove invalid tracked summary blocks

Modify:

```text
benchmarks/summaries/2026-07-Summary.md
```

Remove the two blocks:

```text
# End-to-end CLI / macOS Apple Silicon (6D851D): July 25th - 07:54
```

and:

```text
# End-to-end CLI / macOS Apple Silicon (6D851D): July 25th - 11:00
```

Do not manually invent replacement `Latest`, `Case spread latest` or change-summary values.

## 10.4 Run clean non-recording suites first

Run:

```bash
just bench-validate
just bench-check
just bench-frontend-check
```

Confirm all cases pass and no tracked summary changes occur.

## 10.5 Record new baselines intentionally

On the same machine that owns the tracked system summary, run:

```bash
just bench
just bench-frontend
```

Because protocol and stable IDs changed, the first new runs should be treated as protocol-1 baselines rather than compared to contaminated or protocol-0 data.

Let the summary generator update the header and append entries. Inspect every generated summary change.

If the recording commands cannot be run on the correct machine, restore the phase-10 history edits from the backup or leave them uncommitted, then stop. Do not leave a tracked summary whose header still describes a removed invalid run, and do not fabricate or partially reconstruct summary values.

Suggested checkpoint commit:

```text
bench: replace invalid rename-era baseline
```

---

# Phase 11: Final validation and audit

## 11.1 Run formatting and complete gates

Run exactly:

```bash
cargo fmt
git diff --check
cargo test --workspace --quiet -- --format terse
cargo run --quiet -- tests --audit
cargo run --quiet -- tests
cargo run --package xtask --bin xtask -- bench-validate
cargo run --package xtask --bin xtask -- bench-ci
just validate
just bench-check
just bench-frontend-check
```

The final two full benchmark checks are deliberate confirmation beyond the bounded validation gate. They must remain non-recording.

## 11.2 Audit removed legacy paths

Run:

```bash
test ! -e benchmarks/cases.txt
test ! -e benchmarks/frontend-cases.txt
rg 'case_parser|parse_cases|sanitize_case_name' xtask benchmarks justfile
rg 'cargo run.*check.*--terse' xtask/src
rg 'errors=' xtask/src/bench_validate.rs xtask/src 2>/dev/null
rg 'Beanstalk|BEANSTALK|BST_|\.moth' xtask benchmarks justfile
```

Expected:

- no legacy case parser/list references
- no per-case `cargo run` validation path
- no rendered prose parsing for compilation success
- no stale Beanstalk benchmark/tooling identities

Review any matches rather than deleting blindly. Historical benchmark records may legitimately contain old path-derived names and are not source authority.

## 11.3 Audit required behavioural invariants

Manually verify:

- invalid check/build source exits non-zero
- warnings-only source exits zero
- machine status is absent in normal CLI output
- machine status is exactly one line under benchmark environment
- build cases execute build
- frontend cases use the in-process API
- all benchmark modes use shared preflight
- failed preflight prevents measurement
- failed measurement prevents history writes
- quick runs do not record
- case IDs are stable authored identifiers
- workload changes do not produce speed claims
- optional transfer disagreement falls back to borrow
- real alias conflicts remain rejected
- no benchmark fixture was weakened to hide a compiler bug

## 11.4 Final handoff report

The implementing agent must provide:

- commits/checkpoints created
- exact files changed
- exact benchmark fixture correction
- each compiler regression and its canonical test case
- old tests removed or changed, with semantic justification
- manifest case/workload counts
- quick CLI/frontend case counts
- benchmark protocol and history format versions
- invalid history records removed
- every command run and its result
- any command not run and why
- confirmation that recording commands were or were not run
- remaining uncertainty, if any

Do not claim completion unless `just validate`, `bench-validate`, `bench-ci`, full `bench-check` and full `bench-frontend-check` all pass.

---

# Forbidden shortcuts

The following changes are explicitly prohibited:

- changing `-1 =>` to another pattern to avoid the tokenizer bug
- changing `/` to `//` in `fold-stress.moth`
- adding a `Float` annotation or cast solely to hide the inferred-type bug
- adding `copy op_name`, a dummy later use or duplicated branches to avoid optional-transfer analysis
- adding an arbitrary later use of `cards` or `items` to change liveness
- suppressing `MOTH-BORROW-0007` or `MOTH-BORROW-0008`
- exempting all hidden locals from borrow checking
- matching on names beginning `__hir_tmp_`
- treating exit code 0 as success when the machine status is missing
- treating malformed status/timer lines as zero errors
- always compiling CLI cases with `check`
- spawning `cargo run` once per case
- retaining both text lists and the TOML manifest
- using path-derived sanitised names as persistent IDs
- comparing changed workload fingerprints as performance regressions/improvements
- writing history before a whole selected suite succeeds
- lowering iteration counts for the full recorded suites
- changing benchmark thresholds during this work
- deleting tests merely because optional-transfer semantics changed
- recording new performance data before correctness is green

---

# Recommended checkpoint sequence

1. `cli: return failure status for diagnosed commands`
2. `compiler: fix benchmark-exposed frontend regressions`
3. `bench: replace text case lists with typed manifest`
4. `bench: unify case preflight and execution`
5. `bench: make history stable and workload aware`
6. `bench: add bounded workspace validation gate`
7. `docs: document benchmark manifest and validation flow`
8. `bench: replace invalid rename-era baseline`

Each checkpoint must pass its targeted tests before continuing. Do not squash checkpoints until the complete plan has passed final validation and the diff has been reviewed.
