# Test-suite honesty exposed failures

These Windows-only failures were observed by GitHub Actions run
[`33814563746`](https://github.com/nyejames/moth/actions/runs/33814563746) at `c0d69377b`.
Windows remains non-blocking under `validation.mtf`, but each failure stays visible here until a
`windows-latest` run validates its correction. Ledger entries do not make failing tests acceptable.

## EF-0003 — Windows Clippy compiles a Unix-only symlink helper

- Test or case: `compiler_tests::test_fs::assert_symlink`
- Command: `just ci-gate-clippy`
- Environment: `windows-latest`, x86_64-pc-windows-msvc, all standard Clippy features
- Intended contract: Unix-only symlink assertion support is compiled only where its callers exist.
- Previously masked condition: Windows is a non-blocking CI lane.
- Observed result: `-D warnings` rejects `assert_symlink` as dead code.
- Classification: test-support defect
- Correction owner: `src/compiler_tests/test_fs.rs`
- Status: correction candidate applied; pending `windows-latest` validation
- Validating commit: pending

## EF-0004 — Windows logical paths leak native spelling

- Tests or cases: `rejects_canonical_fixture_without_expectation_before_execution`,
  `distinct_origins_claiming_one_direct_output_path_are_diagnosed`,
  `reserved_javascript_glue_output_rejects_resource_planning`, and the two Stage 0
  `NotADirectory` classification tests
- Commands: `just ci-gate-unit-tests`; `just ci-gate-feature-matrix`
- Environment: `windows-latest`, x86_64-pc-windows-msvc, default and standard feature lanes
- Intended contract: logical identities and rendered fixture/resource paths use portable `/`
  spelling; a child below a regular-file ancestor is `TargetNotRegular` on every platform.
- Previously masked condition: native separators, extended-length prefixes, and Windows
  `ERROR_PATH_NOT_FOUND` differ from Unix path behavior.
- Observed result: five Moth unit failures and five repeated feature-lane failures; xtask also had
  five Windows batch/CRLF fixture failures.
- Classification: harness and test-support defects
- Correction owners: integration fixture rendering, HTML diagnostic path identity, shared file
  reference resolution, and xtask benchmark test fixtures
- Status: correction candidates applied; pending `windows-latest` validation
- Validating commit: pending

## EF-0005 — Windows output-root containment rejects valid project-local roots

- Test or case: 1,921 HTML integration executions
- Command: `just ci-gate-integration`
- Environment: `windows-latest`, x86_64-pc-windows-msvc, `MOTH_TEST_THREADS=2`
- Intended contract: project-local `dev_output` and `release_output` directories pass strict
  canonical containment checks while symlink escapes remain rejected.
- Previously masked condition: canonical output roots and project roots used different Windows
  extended-length path spellings.
- Observed result: valid output roots were rejected with `MOTH-CONFIG-0001`; 28/1949 executions
  were correct.
- Classification: build-system defect
- Correction owner: `src/build_system/output/policy.rs`
- Status: correction candidate applied; pending `windows-latest` validation
- Validating commit: pending

## EF-0006 — Windows benchmark preflight overflows before identifying its case

- Test or case: unknown benchmark manifest case; the old runner emitted no per-case identity
- Command: `just ci-gate-benchmarks`
- Environment: `windows-latest`, x86_64-pc-windows-msvc, release compiler, 82-case preflight
- Intended contract: every benchmark case compiles within supported stack bounds, and a failing
  preflight identifies the responsible case before aborting.
- Previously masked condition: Windows' main-thread stack exposes deeper recursion than Linux and
  macOS; preflight logged only the aggregate case count.
- Observed result: `STATUS_STACK_OVERFLOW` about 13 seconds into preflight.
- Classification: compiler or benchmark-workload defect, pending case localization
- Correction owner: benchmark execution diagnostics first; compiler owner after the case is known
- Status: open; per-case identity and flush added, awaiting `windows-latest` reproduction
- Validating commit: pending
