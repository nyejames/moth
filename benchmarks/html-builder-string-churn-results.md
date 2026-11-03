# HTML Builder String Churn Results

Status: Complete - Phase 2 accepted; Phases 3 and 4 skipped as not material
Origin: Retired HTML builder string churn reduction plan, completed 2026-11-02. Git history holds
the original phase plan.
Scope: HTML project builder assembly, JS-backed HTML output and narrowly justified
output-path/resource URL work.

This file keeps compact, reproducible evidence. Raw logs and profiler artefacts stay in ignored
local directories. It does not replace the benchmark suite's own history or summaries.

## Environment

These facts identify every comparison block so a run cannot be compared against a different
toolchain or host by accident.

| Fact | Baseline |
|---|---|
| Source commit | `ad04332207e73d3406b8a3d9eddc14cfc1b4d97e` |
| Evidence commit | `fe6ca5318aac33fa4c21189a5bf10972884d4ebe` (docs-only child of the source commit) |
| Dependency lockfile | `Cargo.lock` sha256 `cf1bb1cda77e3ff013a343724fc2abd962b85f4e0a06dfb62bb31ff1cb5b664a` |
| `rustc --version --verbose` | `rustc 1.99.0 (b940084d7 2026-09-28)`, host `aarch64-apple-darwin`, LLVM 23.1.1 |
| `cargo --version` | `cargo 1.99.0 (5f94df478 2026-08-27)` |
| Host | macOS 27.0.1, Darwin 27.0.0, arm64, 10 CPUs, 16 GiB |
| Compiler profile | `target/release/moth` built with `--features timers` for isolated docs trials, benchmark-managed profiles for suite runs |
| Docs trial binary | sha256 `ac97dfb20fe464e87cc1a566dffb9002a842eea31454e24cd126537ad9c596a7` |
| `RAYON_NUM_THREADS` | unset (host-selected default) |
| Output-state mode | Isolated docs trials start with an absent `release` root; suite runs use their own isolation |

The evidence commit adds only this results file and the plan and roadmap activation text. Source,
toolchain and lockfile are identical to the source commit, so the suite fingerprints remain
comparable.

Baseline validation: `just validate` passed before measurement while the activation edits were
uncommitted on the source commit. That run's 197.51 s wall time was observed in-session and no log
was retained. The retained validation log for the final tree is under `tmp/final/`.

## Case Inventory And Backend Coverage

`docs_check` and `docs_frontend` are frontend controls. Neither runs the HTML builder, so neither
can attribute builder or JS output movement.

The six manifest CLI build cases that open the HTML builder are:

| Case | Entry | Builder path | Role |
|---|---|---|---|
| `speed_test_build` | `benchmarks/speed-test.moth` | HTML JS-only, single file | Inventory context |
| `module_graph_build` | `benchmarks/module-graph` | HTML JS-only | Inventory context |
| `import_fanout_build` | `benchmarks/import-fanout` | HTML JS-only | Inventory context |
| `import_external_churn_build` | `benchmarks/adversarial/import-external-churn` | HTML JS-only | Focused comparison |
| `module_root_stress_build` | `benchmarks/module-root-stress` | HTML JS-only | Focused comparison |
| `external_js_imports_build` | `benchmarks/external-js-imports` | HTML JS-only with external JS glue | Focused comparison |

The plan selects the last three for the focused comparison. The other three still open the HTML
builder and their baseline stage totals are recorded below so the inventory is not implied to be
complete.

The isolated docs build is a supplementary measurement, not a manifest-managed case.

## Isolated Docs Build Procedure

The production docs project must not write into the tracked checkout. It also must not touch
pre-existing `docs/dev` or `docs/release` output.

```bash
WORK=tmp/docs-build-isolated

rm -rf "$WORK"
mkdir -p "$WORK/src"
rsync -a --exclude='.DS_Store' docs/src/ "$WORK/src/"
cp docs/config.moth "$WORK/config.moth"

# One warmup, then reset the run-owned output root before each measured trial.
./target/release/moth build "$WORK" --release

for trial in 1 2 3 4 5 6; do
    rm -rf "$WORK/release"
    /usr/bin/time -p ./target/release/moth build "$WORK" --release
done
```

The copied input set is `docs/config.moth` plus the complete `docs/src` tree. Physical references
live under `docs/src` (`fonts/`, `styles/`), not `docs/assets`, so the documented site build does
not need the repository-level `docs/assets` tree. A clean copied build was verified to produce
88 output files with no diagnostics before measurement.

Each trial starts from an absent `release` root, so no trial mixes cold writes with
skip-unchanged warm writes. The same copied input tree is reused for before/after comparison. The
prebuilt `target/release/moth` binary is used so Cargo compilation is outside the measurement.

Six trials were recorded. The final docs timings come from `0fc4a37fc`, the Phase 2 checkpoint.
The later seam correction `cfcc7233b` was checked for byte-identical isolated docs output only; it
did not produce new timing evidence.

## Existing Timing Coverage

`backend.js.lower_entry`, `backend.js.lower_linked` and `backend.html.render` are nested evidence
under `build.backend.total`. `backend.html.render` wraps the document-shell render only. The
builder assembly before the shell, glue generation, bundle assembly, inline-script escaping,
fragment rendering, resource planning, output-path reservation and deferred-resource resolution
have no dedicated metric. `build.output.total` covers output planning and filesystem writes after
the backend returns.

The recorded runs observed `backend.js.lower_entry` and `backend.html.render` in six CLI cases
and `backend.js.lower_linked` in none. The three focused fixtures have zero linked views. The docs
build has 83 linked views, but its isolated runs emit the human timing summary rather than
per-metric `MOTH_BENCH` lines. A schema-defined metric with no observations is not evidence about
the linked-lowering path.

No persistent metric or counter is added. Profiling must attribute work inside the untimed
builder span, or the phase must state that the samples are too small to resolve it.

## Baseline (Phase 1)

All evidence below was collected on commit `fe6ca5318aac33fa4c21189a5bf10972884d4ebe` with the
worktree recorded clean by the harness. Raw logs are under `tmp/baseline/`.

```bash
just bench-check
just bench-frontend-check
```

| Suite | Cases | Iterations | Suite average |
|---|---|---|---|
| `end_to_end_cli` | 74 | 10 | 22.645 ms |
| `frontend_phases` | 76 | 10 | 134.567 ms |

Builder-relevant CLI cases, with nested stages in milliseconds:

| Case | Mean | Median | Std dev | Build total | Backend | Output | JS entry | HTML render | Frontend |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `speed_test_build` | 18.532 | 18.386 | 0.419 | 13.809 | 1.580 | 0.440 | 1.266 | 0.007 | 10.411 |
| `module_graph_build` | 11.356 | 11.410 | 0.471 | 6.344 | 0.354 | 0.436 | 0.192 | 0.011 | 3.450 |
| `import_fanout_build` | 13.106 | 12.525 | 1.795 | 7.707 | 0.409 | 0.501 | 0.208 | 0.015 | 4.368 |
| `import_external_churn_build` | 14.807 | 14.571 | 0.672 | 9.299 | 0.743 | 0.720 | 0.477 | 0.016 | 5.588 |
| `module_root_stress_build` | 10.916 | 10.909 | 0.361 | 5.657 | 0.237 | 0.385 | 0.105 | 0.005 | 2.991 |
| `external_js_imports_build` | 10.753 | 10.709 | 0.281 | 5.370 | 0.260 | 0.746 | 0.090 | 0.006 | 2.345 |
| `docs_check` | 160.114 | 158.816 | 2.259 | 154.647 check | n/a | n/a | n/a | n/a | 151.320 |
| `docs_frontend` | 998.598 | 978.895 | 41.135 | n/a | n/a | n/a | n/a | n/a | 987.775 |

`docs_check` and `docs_frontend` remain frontend controls. The six build cases open the HTML
builder, but its share of wall time is small. Process startup dominates the CLI case wall time.

Fingerprints and protocol identities from the `MOTH_BENCH_EVIDENCE` lines:

| Fact | Baseline | Final |
|---|---|---|
| Benchmark protocol | 4 | 4 |
| Timing observation schema | 2 | 2 |
| Thread identity | `default` | `default` |
| Suite averages | 22.645 ms CLI, 134.567 ms frontend | 21.098 ms CLI, 132.776 ms frontend |

Per-case source and measurement fingerprints were identical between baseline and final for every
cited case:

| Case | Workload | Source fingerprint | Measurement fingerprint |
|---|---|---|---|
| `import_external_churn_build` | `import_external_churn` | `47df8422779a` | `10478f58806c` |
| `module_root_stress_build` | `module_root_stress` | `8e130420acf7` | `c584936aa03e` |
| `external_js_imports_build` | `external_js_imports` | `2c8b9b008dd0` | `4cd622f8dc78` |
| `docs_check` | `docs` | `ef747df8639a` | `9ea6da949bc2` |
| `docs_frontend` | `docs` | `ef747df8639a` | `9e823ec7d2f5` |
| `speed_test_build` | `speed_test` | `a7510c9d2010` | `e0e84e323de1` |
| `module_graph_build` | `module_graph` | `8637f73dd03d` | `415a79ede00b` |
| `import_fanout_build` | `import_fanout` | `3f138d4afbf8` | `41ee11fb96bb` |

The final suite run was recorded with `git_dirty=true` because the source change was uncommitted.
The workload fingerprints above are the comparison authority and did not change. The isolated docs
build is outside the manifest evidence and has no suite fingerprint.

Isolated docs build, six measured trials after one warmup. Each trial started with an absent
`release` root. The compiler's own build duration excludes process startup.

| Trial | Built (ms) | Wall (s) |
|---|---:|---:|
| 1 | 190.324 | 0.19 |
| 2 | 190.290 | 0.19 |
| 3 | 187.628 | 0.19 |
| 4 | 188.414 | 0.19 |
| 5 | 189.523 | 0.19 |
| 6 | 187.999 | 0.19 |

Wall values come from `/usr/bin/time -p`, which has 10 ms resolution here. The compiler's `Built`
millisecond value is the comparison signal.

Median built duration 188.968 ms, range 187.628 to 190.324 ms. The build reports 88 emitted
outputs, made up of 83 HTML pages, 4 JS files and one `woff2` file. The hashed output set contains
89 files because it also includes the generated `.moth_manifest`. The same convention applies to
the fixture counts below: the manifest is present on disk but excluded from the builder's emitted
output count.

### Observed Builder Shape

Counts came from a temporary diagnostic probe gated by `MOTH_CHURN_PROBE`. The probe was removed
before implementation and no persistent counter was added. The raw logs are
`tmp/baseline/probe-docs.err`, `tmp/baseline/probe-import-external-churn.err`,
`tmp/baseline/probe-module-root-stress.err` and `tmp/baseline/probe-external-js-imports.err`.
The command shape was:

```bash
MOTH_CHURN_PROBE=1 ./target/release/moth build <entry> [--release]
```

| Build | Entries | Modules | Linked views | Emitted outputs | Hashed files | JS source bytes | `</` in JS | Import preamble |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| Docs | 83 | 87 | 83 | 88 | 89 | 16,966 per page | 0 | present, 80 to 90 bytes |
| `import_external_churn` | 1 | 3 | 0 | 3 | 4 | 57,310 | 15 | 137 bytes |
| `module_root_stress` | 1 | 3 | 0 | 1 | 2 | 13,755 | 6 | absent |
| `external_js_imports` | 1 | 3 | 0 | 5 | 6 | 15,238 | 4 | 175 bytes |

Docs planned 3 resource records carrying 166 metadata uses, about two metadata uses per page.
Every `lowering_map` call, 166 in total, saw zero reachable executable resource uses. The three
focused benchmark cases planned 1, 0 and 2 resource records respectively, all with zero executable
uses.

### Attribution Limitation

`just profile-case import_external_churn_build deep` and `just profile-case module_root_stress_build deep`
produced 50 raw-address hotspots each. Symbolication failed, including a
`just profile-case-symbolicated` retry. The dSYM UUID matched the profiling binary. Both workloads
run in roughly 15 ms or less with a backend under 1 ms, so the samples are too short to attribute
individual builder operations.

No profile function attribution is claimed. The measured stage totals and the temporary counts
above are the attribution evidence. The untimed builder span remains a real instrumentation gap,
but a persistent probe is not justified for a sub-millisecond backend stage that the default
profile cannot resolve.

## Allocation Inventory And Decision

| Site | Classification | Evidence and action |
|---|---|---|
| `js_module.source.clone()` when no import preamble exists | Avoidable full-string copy | `module_root_stress` takes this branch with 13,755 bytes. Borrow and reuse the original source without cloning it. |
| `format!("{preamble}{}", js_module.source)` | Avoidable concatenated bundle | Docs takes it on all 83 pages, `import_external_churn` and `external_js_imports` also take it. Emit the preamble and bundle into the final script buffer instead. |
| `escape_inline_script` returning `js.replace("</", "<\\/")` | Avoidable full intermediate string | Docs has no `</` match but still copies about 17 KiB per page. The match cases exist in all three build fixtures. Escaping belongs in the destination buffer. |
| `artifact_entries.iter().cloned()` | Avoidable borrowed-view clone | Docs clones 83 entry views and their `Arc` handles. Building the owned runtime plan before the loop allows `into_iter()`. |
| `assemble_isolated_module_sources` | Required independent ownership | It creates the one bundle scope for linked modules. Not changed. |
| `import_map_html.clone()` in `render_html_document` | Avoidable but small | Import-map HTML is short and bounded by entry imports. Not material. |
| Per-use `context.clone()` and `output_path.clone()` in the resource planner | Real but small | 166 uses over 3 records in docs. No measurable pressure and simplifying ownership would add lifetime machinery. Not material. |
| Linear `record.uses.contains` deduplication | Potentially quadratic in uses | 166 uses total and 3 records. No evidence of cost. Not material. |
| `HashSet<PathBuf>` / `HashMap<PathBuf, ...>` output bookkeeping | Small maps | 88 keys in the largest measured build. No hashing, cloning or traversal pressure. Not material. |
| Resource URL rendering | No executable uses | `lowering_map` saw zero reachable uses in every measured case. Metadata rendering repeats about two URLs per docs page. Not material. |

Decision:

- Phase 2: **approved candidate**. The three JS ownership/copy sites and the entry-view clone are
  confirmed avoidable and the replacement keeps the same output bytes.
- Phase 3: **not material**. Skip the resource URL reduction.
- Phase 4: **not material**. Skip the output-path bookkeeping consolidation.

## Phase 2 Change And Comparison

Changed implementation:

- `src/projects/html_project/js_path.rs` passes the generated module import preamble and the
  lowered JS source separately to the document renderer. The no-preamble branch borrows and reuses
  the original source without cloning it. Both are written through one escaping helper that appends
  `<\/` for each `</` directly into the final script buffer, including a closing sequence split
  across the preamble/bundle seam.
- `src/projects/html_project/html_project_builder.rs` builds the owned external runtime plan
  before the entry loop and consumes the entry views with `into_iter()` instead of cloning them.
- `src/projects/html_project/tests/js_path_tests.rs` covers absent and present preambles, exact
  script body bytes, one and multiple closing sequences, unrelated `<` and `/` characters,
  closing sequences split across the seam and preamble ordering.

Removed transient traffic, bounded by the observed source sizes:

| Build | Removed copies |
|---|---|
| Docs | About 1.41 MB of concatenated preamble-plus-bundle buffers, about 1.41 MB of escaping intermediates and 83 cloned entry views |
| `import_external_churn` | About 57.4 KB concatenated buffer and 57.3 KB escaping intermediate |
| `module_root_stress` | 13.8 KB source clone and 13.8 KB escaping intermediate |
| `external_js_imports` | About 15.4 KB concatenated buffer and 15.2 KB escaping intermediate |

Peak memory was not measured. The removed buffers are bounded by the JS bundle size and did not
change output bytes.

### Output Equivalence

Every emitted file was hashed before and after the change.

| Build | Result |
|---|---|
| Isolated docs release build | 89 hashed files, including `.moth_manifest`, identical SHA-256 set |
| `import_external_churn` dev build | 4 hashed files, including `.moth_manifest`, identical SHA-256 set |
| `module_root_stress` dev build | 2 hashed files, including `.moth_manifest`, identical SHA-256 set |
| `external_js_imports` dev build | 6 hashed files, including `.moth_manifest`, identical SHA-256 set |

The ordinary JS builds above succeeded. A separate `module-root-stress --html-wasm` build failed
with the fixture's pre-existing projected-place-read rejection, which is unrelated to this change.
The supported Wasm smoke case `cast_int_to_float_runtime` passed `1/1` on the `html_wasm` backend.
The retained log records the command and result in `tmp/final/wasm-integration-smoke.log`.

### Timing Comparison

Isolated docs build, six measured trials:

| Build | Built median | Range | Wall median |
|---|---:|---:|---:|
| Baseline | 188.968 ms | 187.628 to 190.324 ms | 0.19 s |
| Final | 188.593 ms | 186.339 to 191.908 ms | 0.19 s |

CLI cases, ten measured iterations per run:

| Case | Baseline mean / median / sd | Final mean / median / sd | Baseline backend | Final backend |
|---|---:|---:|---:|---:|
| `import_external_churn_build` | 14.807 / 14.571 / 0.672 | 13.200 / 13.136 / 0.278 | 0.743 | 0.664 |
| `module_root_stress_build` | 10.916 / 10.909 / 0.361 | 11.016 / 9.717 / 3.280 | 0.237 | 0.238 |
| `external_js_imports_build` | 10.753 / 10.709 / 0.281 | 9.470 / 9.425 / 0.159 | 0.260 | 0.240 |

`module_root_stress_build` has a final mean above its median with a 3.28 ms standard deviation,
which is an outlier rather than a consistent regression. Its median improved by 1.19 ms while
its backend total stayed flat.

The suite average moved from 22.645 ms to 21.098 ms and the frontend control from 134.567 ms to
132.776 ms. Those movements are process and scheduling noise. The backend deltas are at most
0.08 ms and do not establish a wall-time improvement. The change is accepted on the removed
ownership and copying plus byte-identical output, not on speed.

### Validation

- `cargo test -p moth --lib`: 6475 passed. Log: `tmp/final/unit-tests.log`.
- `cargo run -p moth-cli --bin moth -- tests --terse`: 3091/3091 correct. Log:
  `tmp/final/integration-tests.log`.
- `cast_int_to_float_runtime` on the `html_wasm` backend: 1/1 correct. Log:
  `tmp/final/wasm-integration-smoke.log`.
- `cargo fmt --check -p moth`: clean.
- `just bench-check` and `just bench-frontend-check`: completed and emitted evidence under
  `tmp/final/`.

## Candidate Decisions

- Phase 3 resource URL work: **skipped**. Every measured build had zero reachable executable
  resource uses in `lowering_map`. Docs rendered 166 metadata uses over 3 records, about two per
  page. That is repetition, but no evidence that it is costly.
- Phase 4 output-path bookkeeping: **skipped**. The largest measured build had 88 output paths.
  No direct hashing or traversal measurement exists because the profiles were unusable, so the
  decision rests on the small N and the absence of a plausible pressure source, not on a measured
  absence of cost.
- Phase 2 JS ownership and entry-view cleanup: **implemented** with byte-identical output.

## Final Verification

Final code tree: Phase 2 source changes plus the post-review seam correction and retained evidence,
on top of the source and evidence commits above. The recorded isolated docs timings were produced
from `0fc4a37fc` (the Phase 2 checkpoint) with a `--features timers` release compiler. The seam
correction `cfcc7233b` was validated by the full gates and a byte-identical output comparison, not
by replacing those timings.

- `just validate`: passed. Log: `tmp/final/validate.log`. It covers formatting, clippy, source
  audit, first-party dependency audit, workspace unit tests, the 3091-case integration suite,
  the docs check and cache maintenance.
- `just validate-full`: passed. Log: `tmp/final/validate-full.log`. It adds the workspace format
  check, feature clippy, the feature test matrix, the test honesty audit, the bounded performance
  gates and the timer-erasure check.
- `just bench-validate`: all 152 benchmark cases passed shared preflight. Log:
  `tmp/final/bench-validate.log`.
- `just bench-check` and `just bench-frontend-check`: final evidence under `tmp/final/`, with
  matching source and measurement fingerprints for every cited case.
- Isolated docs build: final median 188.593 ms, range 186.339 to 191.908 ms. All 89 hashed output
  files are identical to the baseline.
- HTML-Wasm smoke: `cast_int_to_float_runtime` passed 1/1. Log:
  `tmp/final/wasm-integration-smoke.log`.

Post-review seam correction: the module script renderer now escapes a `</` split across the
preamble/bundle boundary without joining the slices. `just validate` and `just validate-full` were
rerun on the corrected tree; logs are `tmp/final/seam-validate.log` and
`tmp/final/seam-validate-full.log`. The focused seam regression and concatenated-contract tests are
in `js_path_tests.rs`. The ordinary `external_js_imports` dev build and the isolated docs build
remain byte-identical to the recorded baseline outputs. No benchmark baseline was regenerated; the
correction leaves the common emitted path unchanged.

Final audit outcome:

- Code lane: clean. No correctness, integration, ordering, escaping or scope regression found.
- Measurement lane: initial findings corrected. The builder inventory, identities, probe logs,
  skipped-phase limits, output-count convention, spread reporting and unit corrections were
  re-verified.

Remaining limitations:

- The untimed builder assembly span has no stable attribution metric. Profiling failed
  symbolication and the selected workloads are too short, so the accepted change rests on
  ownership and byte-equivalence evidence rather than a measured wall-time win.
- Phases 3 and 4 were skipped as not material on the measured builds. No cache, path-identity or
  URL-policy change was made.
- Peak memory was not measured.

## Local Artefacts

Raw evidence stays under ignored paths. Record paths only.
