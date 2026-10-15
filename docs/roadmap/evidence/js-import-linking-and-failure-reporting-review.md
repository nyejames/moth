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

## Topology, entry liveness and typed source-location checkpoint

The unchanged minimal primary remains separate from one cohesive `html_js_binding_runtime_topology` boundary scenario. Its exact Node result observes module-instance state, alias/namespace calls, fallible success and caught error code/message, full-width U32 consumption, executed generic demand and distinguishable same-basename providers. Dormant providers remain semantically validated without runtime assets.

| Required contract | Primary evidence owner | Observation |
|---|---|---|
| Direct import | Existing `build_dependency_tests` direct-import primary | One selected provider and correct relative glue |
| Pure one/two-hop facades | Minimal primary and cohesive companion | Actual execution without intermediate callable Moth bodies |
| Alias/namespace and repeated legal paths | Cohesive companion | Exact shared state and exactly-once effects |
| Call/Invoke, success and caught failure | Cohesive companion; Phase 2 lowered facts test | Exact successful/caught values, error code 73 and message |
| Mixed constants/live functions; constant-only absence | Companion and existing `external_const_u32_{mixed,static}_emission` controls | Full-width U32 survives; constant-only emits no provider/glue |
| Private/inactive work | Companion and existing canvas-unused control | Private, false-branch and true-assertion providers absent |
| Executed/validation-only generation | Companion and Phase 2 generated service regression | Runtime executed once; final service excludes validation-only sidecar/assets |
| Distinct same-basename providers | Companion and build artifact test | Distinguishable runtime values and distinct logical resources |
| Multiple entries | Shared-support build artifact test and disposable browser smoke | One provider; two page-specific wrapper selections and valid relative imports; `/` renders `root`, `/about/` renders `about` |
| Invalid selection/annotation | Existing unsupported-JS, canvas export, privacy-bypass and missing-symbol negatives | Exact typed codes and authored import locations |
| Runtime import map | Extended existing fallible-JS runtime-helper artifact test | Pure facade, actual runtime module/provider and exact parsed mapping |

The first new unsupported-JS location assertion failed before the narrow producer fix: `MOTH-IMPORT-0022` was present but path and line were `<no source position>`. `JsExternalImportProvider` now preserves the request's authored Moth import span when converting parser diagnostics. The payload still names the JS module; no JS-byte-range/Moth-source splice, new source registration, error-code change or infrastructure relabelling was introduced.

| Validation | Revision/profile | Result |
|---|---|---|
| `cargo run --quiet -- tests --case html_js_binding_runtime_topology --backend html --terse` | `1dae29ce2` plus Phase 3 candidate, dev | 1/1 exact runtime result |
| Existing minimal, constant-only/mixed, canvas-unused, canvas-export and missing-symbol commands | Same candidate, HTML/dev | Each 1/1; privacy-bypass with both backends 2/2 |
| Unsupported-JS and generic-signature focused commands after span fix | Same candidate, HTML/dev | Each 1/1 |
| `cargo run --quiet -- check tests/cases/unsupported_external_js_import_rejected/input --terse` | Same candidate, dev | Expected exit 1; `E\|MOTH-IMPORT-0022\|@page.moth\|1:1\|...`, one source error, zero warnings, no infrastructure banner |
| `cargo test -p moth --lib provider_sources_tests` | Same candidate, test/dev | 20 passed |
| `cargo test -p moth --lib build_dependency_tests` | Same candidate, test/dev | 25 passed, including actual minimal fixture materialisation |
| `cargo fmt --all -- --check` | Same candidate | Passed |
| `target/debug/moth build tmp/js-linking-multiple-pages` and managed Chromium at port 9875 | Before span-only fix, same Phase 3 topology candidate/dev | 5 outputs; both routes render distinct intended text, no browser errors; tab/server closed |

The normal Node harness still selects only `index.html` and rejects import maps. Both-page artifact checks are permanent; separate browser route execution supplies additional runtime evidence without expanding that harness. The import-map row claims artifact verification only, not Node or browser execution. Existing writer tests retain deferred-read/preflight/stale-output ownership; the minimal handoff test additionally compares actual materialised provider, glue and HTML bytes.

Location-red output is retained at ignored `tmp/js-import-linking-location-red.log`. The producer handoff accidentally removed an existing `StringTable` import; parent diff inspection restored it before compilation.

Fresh runtime/emission and typed-provenance auditors accepted Phase 3 as clean, with no required correction. Runtime review traced assertions into their consuming owners and reused existing dormant-provider root controls; diagnostic review traced source identity through provider failure, premerge transport and synthetic rebind. The current routed test-admission authority is `docs/src/developer-docs/style-guide/testing.mtf`, not an absent `docs/testing-standards.md`. Disposable multi-page smoke sources and outputs were removed after review; its permanent build test retains the reproducible source shape.
