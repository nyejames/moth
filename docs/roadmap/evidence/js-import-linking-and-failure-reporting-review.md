# JavaScript import linking and failure reporting evidence

## Activation

Implementation started on `link-fixes-and-test-cleanup` at `aa072a27a32b7233b1e3dcfdd9e4f42b0578bafc` with a clean staged, unstaged and untracked worktree. The user selected this branch, overriding the plan's proposed main landing. Other paused programmes remain paused.

Rust is the repository pin `1.99.0` (`b940084d7`, aarch64-apple-darwin), Cargo `1.99.0`, Clippy `0.1.99` and rustfmt `1.10.0-stable`. Checks use the pinned Node `24.21.0` from the installed nvm version. The baseline uses Cargo's dev profile and HTML development output.

## Baseline regression

`html_js_reexport_linking` owns `build.external_js.reexport_runtime_linking`. Existing direct-provider, source-facade and unused-asset cases do not protect execution through a pure Moth facade with no directly owned provider clause in the caller.

| Case | Command | Revision/profile | Fixture | Observed result | Expected contract | First failing boundary | Disposition |
|---|---|---|---|---|---|---|---|
| Minimal re-export | `cargo run --quiet -- tests --case html_js_reexport_linking --backend html --terse` | `aa072a27`, dev | `tests/cases/html_js_reexport_linking` | 0/1 correct: successful-build expectation failed | Execute `canvas:1` | HTML glue runtime-asset lookup | Confirmed compiler defect; retain exact minimal fixture |
| Failure detail | `cargo run --quiet -- build tests/cases/html_js_reexport_linking/input` | `aa072a27`, dev | Same fixture | `HTML JS glue could not find a runtime asset for external package ExternalPackageId(8).` | Matching provider metadata for retained external call | `runtime_glue/source.rs` | Raw baseline logs retained under ignored `tmp/js-import-linking-red*.log` |
| First lost fact | `cargo test -p moth --lib reexported_js_final_calls_retain_provider_runtime_candidate -- --nocapture` | `4c19a00c8` plus probe, test/dev | `provider_sources_tests.rs` | 1 selected test failed only at caller-candidate assertion, after facade asset, both final external calls and registry/owner checks passed | Caller retains the demanded provider candidate | `module_compilation/external_imports.rs` source-only inventory | Confirms metadata loss before entry assembly; log `tmp/js-import-linking-red-facts.log` |
| Supplied website | No build possible | Activation | Supplied archive unavailable | No matching archive found in workspace, `tmp`, project parents or inspected home archive matches | Unchanged website dev/release rebuild | Input availability | Environment/input limitation; no website build claimed |

The attempted `build ... --terse` command was rejected because only `check` and `tests` support that flag. The valid build command above supplied the actual failure detail.

The initial first-fact probe assumed an export-only normal module had no implicit `start`, so its selector failed before the intended assertion. Selecting artifacts by their entry paths corrected that test setup. The corrected probe produced the metadata failure recorded above. No production collector change preceded it.

## Runtime-import handoff checkpoint

The compiler now joins final external-call demand with source-owned and builder availability through one checked package projection. Generated sidecars refresh their selected root's candidates after convergence and validation-only pruning. Entry liveness and foreign ABI owners remain unchanged.

| Command | Exercised scope | Result |
|---|---|---|
| `cargo test -p moth --lib build_dependency_tests` | Direct import, unused providers, constants, deferred assets and glue | 22 passed |
| `cargo test -p moth --lib provider_sources_tests` | Source scoping, candidate loss and registry/provider controls | 20 passed |
| `cargo test -p moth --lib direct_external_calls_retains_lowered_call_and_invoke_sites_in_order` | Real lowered statement Call and terminator Invoke facts | 1 passed |
| `cargo test -p moth --lib final_generated_failure_lanes_keep_external_candidates_without_validation_only_sidecars` | Final generated failure lanes, provider availability and validation-only removal | 1 passed |
| `cargo test -p moth --lib compiler_frontend::module_compilation::external_imports::tests` | Compatible spans/import sets, conflicting runtime facts and Core no-asset calls | 3 passed |
| `cargo test -p moth --lib external_js::runtime_glue::tests` | Checked selected inputs, deterministic errors, relative imports and unchanged foreign wrappers | 29 passed |
| `cargo test -p moth --lib provider_registry_tests` | Provider registry and retained source-prefix/scoped lookup contracts after obsolete test removal | 10 passed |
| `cargo run --quiet -- tests --case html_js_reexport_linking --backend html --terse` | Native ES-module execution | 1/1 correct, exact `canvas:1` |
| `cargo run --quiet -- build tests/cases/html_js_reexport_linking/input` and the same command with `--release` | Materialised dev and release outputs | Both built 3 outputs |
| `just boracle` | Formatting, Boracle-feature Clippy and opt-in source/solver/oracle lanes | Passed, 46 borrow-problem, 5 last-use and 260 Boracle tests |

The initial `runtime_glue_tests` and `external_imports_tests` filters selected zero tests because their file names differ from their registered module paths. The exact module-path runs above replace that empty evidence. Rust formatting used `cargo fmt --all`.

Both materialised profiles contain exactly one `_moth/js/components/background-animation.js` and one generated glue module. The glue imports `../components/background-animation.js` and declares two demanded wrappers. Each HTML document imports the actual emitted glue file. The provider bytes match the authored no-DOM implementation. Generated IDs and glue hashes are observations, not golden contracts.

The independent frontend and HTML phase audits found no production correction. The frontend audit requested a precise candidate-projection description in the permanent contract and identified an additional final-refresh regression. Pointer-identity and duplicate-query-row tests tied to the retired collector were removed rather than repinned. Canonical projection tests own package deduplication.

The plan's generated convergence directory reference drifted: the current owner is `module_compilation/generated/convergence.rs`.

## Supplied website input

After initial input discovery, the user supplied `~/projects/websites/personal/nyejames.com`. A disposable source copy lives under ignored `tmp/js-linking-website`; the original site remains unchanged. Website baseline and final rebuild evidence use this supplied tree, not a reconstructed site or compiler workaround.

The isolated activation compiler was built from `git archive aa072a27` under `tmp/js-linking-baseline-compiler` with its own target directory. Both the minimal probe and supplied site failed with the original missing runtime asset for `ExternalPackageId(8)`. An initial shared-target attempt reused a binary that passed the minimal integrity probe, so it was discarded as baseline evidence. The isolated build's red integrity probe establishes the compiler used for the website baseline.

| Website check | Command or observation | Result |
|---|---|---|
| Activation compiler | `cargo build --manifest-path tmp/js-linking-baseline-compiler/Cargo.toml --target-dir /Users/aneirinjames/projects/beanstalk/moth/tmp/js-linking-baseline-target --bin moth` then `tmp/js-linking-baseline-target/debug/moth build tmp/js-linking-website` | Missing runtime asset, matching the minimal baseline defect |
| Patched development build | `cargo run --quiet -- build tmp/js-linking-website` | 4 outputs built |
| Patched release build | `cargo run --quiet -- build tmp/js-linking-website --release` | 4 outputs built |
| Source preservation | `diff -r` on original/copy `src` and `cmp` on config | Identical before and after builds |
| Provider preservation | `cmp` authored animation against each emitted profile's provider bytes | Identical |
| Import chain | Inspect actual emitted HTML, glue and provider paths in both profiles | HTML imports its emitted glue; glue imports `../components/background-animation.js`; one demanded animation wrapper |
| Browser execution | Served disposable release output at `http://127.0.0.1:9874/`, opened managed Chromium, inspected screenshot and sampled canvas across animation frames | Title `Welcome | Nye James`, visible rendered page and waves, nonzero canvas pixels, changing frame hash, no browser errors |

Raw website logs remain in ignored `tmp/js-linking-website-baseline.log`, `tmp/js-linking-baseline-integrity.log` and `tmp/js-linking-website-green.log`. The browser tab and temporary server were closed after the smoke check.

## Final generated publication regression

The generated service fixture now includes a late-declared private callee whose protected call starts without a failure CFG successor. Convergence infers its checked-addition failure and installs an `Invoke`, making a handler-only `recovery.js` provider newly reachable. Final retained facts and published candidates require both live and recovery providers, while excluding the validation-only provider.

`cargo test -p moth --lib final_generated_failure_lanes_keep_external_candidates_without_validation_only_sidecars` passed with one selected test. Temporarily discarding the final projected candidate list instead of publishing it made the same valid fixture fail: expected two assets, found one. Publication was restored and the targeted test passed again. `cargo test -p moth --lib compiler_frontend::module_compilation::generated::convergence::tests` then passed all 36 tests. `cargo fmt --all -- --check` passed.

The initial whole-block-removal mutation failed compilation because warnings are denied; it supplies no runtime regression evidence. The valid publication mutation and final full convergence-suite outputs are retained under ignored `tmp/js-import-linking-refresh-{mutation,suite}.log`.

A source-only recovery-handler removal case was investigated but not added: pending catch routes are already absent from initial rooted reachability, so that shape would pass even without the final refresh. The newly reachable handler case above exercises the actual missing-publication boundary without fabricated stale state.

Fresh focused verification accepted the final-publication regression and qualified compiler authority as clean. The independent frontend, HTML and final-publication review lanes leave no required phase finding. The generated test does not expose an intermediate candidate snapshot or claim cross-registry correctness; construction-path inspection and the failed publication mutation establish its admission.
