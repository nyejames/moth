# JavaScript import linking and failure reporting evidence

## Activation

Implementation started on `link-fixes-and-test-cleanup` at `aa072a27a32b7233b1e3dcfdd9e4f42b0578bafc` with a clean staged, unstaged and untracked worktree. The user selected this branch, overriding the plan's proposed main landing. Other paused programmes remain paused.

Rust is the repository pin `1.99.0` (`b940084d7`, aarch64-apple-darwin), Cargo `1.99.0`, Clippy `0.1.99` and rustfmt `1.10.0-stable`. Activation selected the installed nvm Node `24.21.0`. The adjacent campaign records a later environment mismatch below and explicitly selects that executable on every authoritative command. The baseline uses Cargo's dev profile and HTML development output.

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

After initial input discovery, the user supplied `~/projects/websites/personal/nyejames.com`. Rebuilds used a disposable source copy under ignored `tmp/js-linking-website`; the original site remains unchanged. Website baseline and final rebuild evidence use this supplied tree, not a reconstructed site or compiler workaround. The smoke copy and isolated baseline compiler/target were removed after proof, with raw receipts retained.

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

## Typed failure transport and reporting checkpoint

The candidate after `160782190` keeps one cold ordered `InfrastructureErrors` box beside typed diagnostics. Every cause retains its type, raw message, host path, span and metadata. Explicit cause ranges keep live or frozen source owners paired through append and finalisation. Independent frozen identity domains retain their own strings. The common message vessel remains at most 128 bytes and counts the aggregate as one infrastructure event.

### Producer inventory and classification

The inventory inspected constructors and every production `.with_error_type(...)` group on this branch, prioritising these boundaries:

| Path and producer | Actual cause | Final class and disposition |
|---|---|---|
| `build_system/path_validation.rs::check_if_valid_path`, `create_project_modules/source_loading.rs::source_read_error` | Missing paths and source read failures | `File`, operational, preserved |
| `create_project_modules/resource_inputs.rs`, `build_system/project_config.rs`, `output/{writer,manifest}.rs` and `build_system/utils.rs` filesystem producers | Resource/config reads, canonicalisation, output inspection and writes | `File`, operational, preserved |
| `create_project_modules/discovery_provider_imports.rs::resolve_provider_prefix_to_canonical_path` | Provider-prefix canonicalisation | `File`, operational, preserved |
| Same module's `invoke_provider_and_record_resolution` | Provider's returned failure | Preserve the provider's typed carrier and class |
| `html_project/external_js/js_import_provider.rs::resolve_external_import` | JS read failure versus authored JS errors | `File` for IO, typed import diagnostics for source errors |
| JS package registration, checked runtime-glue exports and module/link assembly | Inconsistent validated compiler or builder metadata | `Compiler`, compiler defect |
| HIR validation/transformation and `backends/error_types.rs::lir_transformation_error` | Generated HIR/LIR invariants | `HirTransformation` or `Backend(LirTransformation)`, compiler defect |
| `backends/wasm/{backend,emit/**}.rs` | Wasm emission and generated-binary invariants | `Backend(WasmGeneration)`, compiler defect; capability rejection stays typed |
| `dev_server/server.rs::validate_dev_entry_path` | Missing paths and failed canonicalisation | Preserve `File` rather than overwrite it with `DevServer` |
| `dev_server/build_loop.rs::ProjectBuildExecutor::build` | Non-UTF-8 host entry path | Path-bearing `File`, operational |
| `dev_server/server.rs::resolve_dev_runtime_paths`, `build_loop.rs::build_once` | Successful bootstrap/build lacks required output settings or entry page | `Compiler`, builder invariant, with warnings retained |
| `dev_server/server.rs` bind/current-directory/entry-kind producers | Host or invocation failure | `DevServer`, operational |
| `dev_server/watch.rs` and Node `node_harness.rs` | Watch and render-harness process failures | Existing warning/IO or `RenderHarnessError` owners, unchanged |

`Config` had no production owner and was removed. Authored config mistakes remain typed diagnostics and config IO remains `File`. No compiler-owned process-spawn/status producer required a new tooling variant. Remaining backend subtype precision in `emit/vec_helpers.rs` and `hir_to_lir/terminator.rs` already yields compiler-defect classification and stays outside this task.

### Transport, render and runtime evidence

The final render tail now returns its original message vessel even when move-only freezing fails. It appends the freeze cause rather than replacing prior causes or warnings. Source snapshots move through the shared freeze owner, and retention probes count owners reachable from the returned report, including live owners after failure. Connected header, premerge, template, expression, import and materialisation adapters move the complete cold aggregate. A small AST-owned named signature result replaces two four-field tuple results without adding copies or allocations.

| Check | Revision/profile | Observed result |
|---|---|---|
| Message/render tests, compiled-tail tests and dev-server tests | `160782190` plus Phase 4 candidate, default | 155, 15 and 91 passed respectively |
| Actual checked-glue metadata failure; header and mixed-premerge regressions; TIR tests | Same candidate, default | 1, 1, 1 and 6 passed; each metadata fault exercises all three renderers |
| Source loading, providers and build orchestration | Same candidate, default | 5, 20 and 53 passed |
| Memory-probe final-tail regression | Same candidate, `data_layout_memory_probe` | 1 passed; failed freezing preserves both causes, warning and exact live source owner |
| Template, package-materialisation and import-projection suites after corrections | Same candidate, default | 32, 29 and 7 passed |
| `just boracle` | Same candidate | Clippy passed; 46 borrow, 5 last-use and 260 Boracle tests passed |
| `just validate` | Final Phase 4 code candidate, workspace-unified features | Passed: main unit lane 6126, compiler integration 2786/2786, docs clean, source/dependency audits zero findings |
| Missing-input CLI, terminal and terse, with `MOTH_BENCH_STATUS=1` | Phase 4 executable, dev | Both exit 1, report operational failure and stage, emit no benchmark status record; terse keeps one five-field infrastructure record |
| Unsupported-JS CLI comparison | Same executable, dev | Exit 1, typed `MOTH-IMPORT-0022` at `@page.moth:1:1`, no infrastructure banner, normal benchmark record |
| Managed Chromium, checked metadata error page | Production renderer capture served statically at port 9877 | One infrastructure card, compiler-bug introduction, original registry detail and fixed clickable report URL |
| Managed Chromium, live `moth dev` at port 9876 | Disposable minimal project | Invalid UTF-8 source triggers one operational card with file/path/stage details and no report link; restoring source recovers `canvas:1`, no browser errors |

The static metadata capture proves presentation, not live recovery. The separate operational probe exercises real failed rebuild and recovery. Browser wait helpers twice timed out across navigation despite subsequent visible DOM observations; those tool failures were reported and were not counted as verification. Fresh observation and screenshots supplied the stated UI evidence. Tabs, services, temporary Rust capture changes, temporary module visibility and smoke sources were removed.

Five fresh audit lanes reviewed cold ownership, premerge/discovery transport, AST adapters, presentation/status and producer/dev-server classification. One required finding migrated an existing infrastructure-only content failure test from a fabricated diagnostic-zero source lookup to its explicit cause owner. A fresh focused verifier accepted that correction and the named signature result. Validation also caught two Boracle-only scalar adapters, mechanical Clippy issues and a direct test clock; each was corrected without suppression. The final missing-entry regression uses the production build cycle once, preserving a warning and asserting the published failure classification.

Infrastructure renderers still show host paths, messages and metadata rather than source-span excerpts. Paired source provenance remains available in the cold carrier. Acquisition-race rollback received source review but no concurrent race execution; external strong-owner failure and weak aliases have deterministic coverage. These limits do not claim the paused final failure-lane or host-isolation migration.

## Bounded adjacent-defect campaign

The CLI campaign used the `8f15579ec08d6659e4e4360e74b55e3d9de5aad6` debug compiler, SHA-256 `2b3d53107b8d505086e6e7670a092059610d25e504ef644218844ee34451c628`, with development and release project output where stated. Rust probes used that checkpoint plus the campaign candidate. Every authoritative campaign command selected Node `24.21.0` through `PATH=/Users/aneirinjames/.nvm/versions/node/v24.21.0/bin:$PATH`.

| Case | Actual command | Revision/profile and fixture | Observed result | Expected contract | First failing owner | Evidence and disposition |
|---|---|---|---|---|---|---|
| Registry-boundary overlap | `cargo test -p moth --lib build_dependency_tests -- --nocapture` with the two temporary probes; `cargo test -p moth --lib canonical_binding_identity_resolves_across_independent_registries` after its extension | Campaign candidate, test/dev, registered project/campaign/marker fixture and existing independent-registry test | Both providers execute as `project=project:17\|package=package:29\|marker:43`. The independent control resolves one canonical function from ID 702 to 703 and rejects a different canonical function that reuses raw ID 702 | Preserve canonical providers and never join unrelated registry-local IDs | None in shared-registry link assembly | One `Arc` registry enters both graphs at `compilation.rs:1371-1407`. Entry unions use issuing registries, linked lowering merges external IDs and final glue uses the entry registry. This path shares rather than remaps IDs. Both controls pass |
| Registered source-package metadata | Same Cargo command, then `cargo test -p moth --lib build_dependency_tests` without temporary includes and the focused registered-package filter after its extension | Campaign candidate, test/dev, nested package facade and direct unrooted-provider topology | Original nested candidate panics at `module_namespace.rs:523`, length 1/index 1. Isolated `aa072a27` reproduces it. Corrected nested and in-bounds flat lookups, actual writer bytes and Node execution pass | Resolve providers against their owning package namespace and issuing registry, including when local indexes happen to be in bounds in both graphs | `DirectoryDependencyResolution::resolve_provider_target` dropped boundary/prefix | Red receipts `artifact://238` and `artifact://248`, green mixed probe `artifact://252`, permanent 27-test suite and focused six-build matrix. Confirmed pre-existing compiler defect, fixed without flattening the original fixture |
| Scoped support, deep facade and invalid namespace | `cargo test -p moth --lib build_dependency_tests`; `cargo run --quiet -- tests --case module_facade_transitive_reexport_closure_success --backend html --terse`; same runner for `module_boundary_cross_module_facade_allowed` | Campaign candidate, test/dev, existing scoped support/two-page fixture, two-hop child facade and new normal-sibling control | Scoped support keeps page wrappers isolated. Both selected HTML integration cases pass 1/1. Reversed creation orders both report only `MOTH-IMPORT-0005` for a child importing its normal sibling | Legal facades stay usable and direct normal-sibling lookup stays invalid | None | Artifact and typed negative controls pass, with integration receipt `artifact://260`. This does not extend topology |
| Asset-only rebuild | `node tmp/js-linking-rebuild-campaign.mjs` | `8f15579ec`, debug/dev, `tmp/js-linking-rebuild-campaign-AavEQC/fixture` | Annotation and Moth source stay unchanged. New emitted bytes execute as `rebuild-byte-edit-9823`, replacing `rebuild-baseline-4107` | Propagate provider bytes on rebuild | None | `results.json` stages 1-2 retain source/output hashes and exact native Node output. Pass for freshness, no cache or semantic-reuse claim |
| Signature, export removal and last caller | Same Node command | Same compiler/profile/fixture | Signature edit gives `MOTH-TYPE-0001`. Removed selected export gives `MOTH-IMPORT-0005`. Restores execute successfully. Removing both callers deletes provider and glue from disk and manifest while retaining `index.html` and `rebuild-entry` | Typed source failures and normal stale cleanup | None | Stages 3-7 retain commands, diagnostics, output snapshots and the v4 HTML/dev manifest containing only `index.html`. Pass |
| Concrete native results and assertion demand | `cargo test -p moth --lib final_generated_failure_lanes_keep_external_candidates_without_validation_only_sidecars`; `node tmp/js-linking-demand-target-campaign.mjs` | Campaign test candidate and `8f15579ec` debug compiler, dev/release fixtures under `tmp/js-linking-demand-target-campaign-1792029318281` | Final source/generated facts retain Call/Invoke providers and active message sidecar, prune inert validation-only message. Native success prints `live`, `result=7`, `messageCalls=0`. False assertion exits 1 with `JS_DEMAND_ACTIVE_MESSAGE_FATAL_6F28` | Publish final executable demand, evaluate active message only on failure, preserve fatal assertions | None | Existing convergence test and exact native ES-module graph execution pass. The fatal result is an uncaught assertion Error, not a returned entry Error or import failure |
| HTML-Wasm | `target/debug/moth build <campaign>/wasm-reexport-generic --html-wasm` and with `--release` | `8f15579ec`, dev/release, generic call through a pure source facade | Both exit 1 with exactly `MOTH-RULE-0058`, source label on `return label()`, no missing-asset infrastructure | Reject unsupported reachable external functions through target validation | Target validation correctly rejects the call | Same demand campaign receipts. Pass for typed rejection. Mixed JS/Wasm affinity routing remains accepted but undelivered |
| Runtime imports, ordinary JS resources and conflicts | `node tmp/js-linking-runtime-resource-campaign.mjs`, real Chromium at `/dev/` and `/release/`, existing dedup/conflict/preflight Cargo filters | `8f15579ec`, debug compiler, dev/release under `tmp/js-linking-runtime-resource-campaign-fCBGBY` | One mapped runtime, provider and glue execute. Visible text `runtime=17 ordinary=./drawing/provider.js`, href `./drawing/provider.js`, no JS errors. Distinct-origin output collision gives only `MOTH-CONFIG-0001`, no outputs | Preserve resource origin versus byte source, exact runtime imports and preflight | None | `records.json`, `browser-results.json` and screenshots. All three script fetches return 200 in both profiles. Existing Rust controls pass for deduplication, reserved glue collision and unhashed file-ancestor/symlink conflicts. CLI output absence alone proves no byte-read claim |
| Determinism | `cargo test -p moth --lib build_dependency_tests`; focused `build_html_registered_source_package_preserves_provider_identity_and_order` after the matrix extension | Campaign candidate, test/dev, nested and flat roots, each with `[0, 1, 0]` forward/reversed/forward creation and registration | All six builds match runtime, provider destinations, import order and wrapper order. Negative sibling diagnostics match across creation orders | Repeat identical roots and vary legal input order without comparing absolute-path glue hashes | None | Both topologies compare nonempty two-import/two-wrapper semantic sequences and exact writer bytes. Pass for input-order independence, not arbitrary publication schedules |

Before the final matrix extension, targeted validation passed `cargo clippy -p moth --all-targets`, 27 build-dependency tests, the extended one-test independent-registry control, three provider-discovery controls and the one-test generated-convergence owner. Both selected HTML integration filters returned 1/1. Formatting used `cargo fmt --all`.

The resource/preflight commands each selected and passed one existing test:

```sh
cargo test -p moth --lib distinct_origins_colliding_at_provider_path_report_both_spans
cargo test -p moth --lib reserved_javascript_glue_output_rejects_resource_planning
cargo test -p moth --lib shared_origin_across_entries_emits_one_planned_record
cargo test -p moth --lib plan_dedupes_js_assets_across_modules
cargo test -p moth --lib plan_dedupes_runtime_specifiers_across_modules
cargo test -p moth --lib deferred_resource_file_ancestor_conflict_stays_unhashed
cargo test -p moth --lib deferred_resource_symlink_destination_conflict_stays_unhashed
```

Additional namespace controls selected two `direct_selection_resolves_` tests and one `direct_child_private_path_bypass_is_rejected` test, all passing. These receipts cover macOS/arm64. They claim no Linux or Windows execution.

Pinned `just validate` passed before the final test-only in-bounds extension: 6128 main workspace-unit tests, all other workspace suites, 2786/2786 compiler integration results, clean docs and zero source/dependency/feature-audit findings. After that extension, the focused registered-package filter passed all six build/runtime rows and `cargo clippy -p moth --all-targets` passed. The corrected compiler also rebuilt the unchanged supplied website in dev and release, four outputs each.

Three fresh phase lanes reviewed registered-package/canonical identity, generated demand/target evidence and the complete campaign/reproducer. All returned clean with no required corrections. The registered-package lane identified an optional in-bounds namespace coverage gap, so the same scenario gained the flat topology without replacing its nested row. The demand receipts contain ten command records, five per profile. They don't independently exercise an external `{ ok: false }` return or returned-entry Error; prior topology/failure coverage remains unchanged.

A fresh focused verifier accepted the added nested/flat matrix and confirmed that it catches wrong-namespace selection even when the local module ID stays in bounds. No required phase finding remains. The three temporary CLI campaign scripts were removed after their evidence review. Saved receipts and output snapshots remain under ignored `tmp`, and the independent collision retains its documented source and local reproducer.

### Namespace correction and retained limits

`DirectoryDependencyResolution` already owns the source index, boundary, package prefix and path fork. Provider lookup now borrows that context and selects the matching project or package namespace. Its source row and namespace use the same boundary-local module ID. The obsolete global path-search fallback and helper were deleted. Ordinary dependency lookup, private boundaries, diagnostics and output policies remain unchanged.

The permanent build test retains the nested package topology and adds an isolated flat-provider row for silent in-bounds misrouting. Both retain same-basename distinguishable providers, the real output writer and existing Node harness. A temporary minimal nested probe also executed `package=package:29` after correction. Temporary Rust includes were removed from both compiler trees.

The initial resumed shell selected Node `26.10.0`. The resource script rejected it before creating a fixture. The demand script and first six rebuild stages executed on 26, but those receipts do not count as pinned validation. Explicit Node 24 reruns supplied the results above. The rebuild script initially rejected the cleanup page's two classic inline scripts after its artifact/manifest assertions had passed. Concatenating their exact authored order into one classic context corrected that smoke harness limit without changing compiler input or output. Native module graphs remain unrewritten and install no DOM mocks or import-map substitutes.

Chromium made one unrelated `/favicon.ico` request that returned 404 in the development check. Provider, glue and runtime requests all succeeded. The report claims no network-wide zero-error result.

### Independent provider-identity collision

Two different physical providers at the same boundary-relative `binding.js` path still collide. Both the activation compiler and corrected candidate report:

```text
External package '@html-js/binding.js' is already registered.
```

The current mixed probe run selected 29 tests: 28 passed and only this exact collision failed. The isolated activation run selected one test and failed with the same compiler cause. The failure occurs before backend execution, so neither two emitted providers nor the expected runtime result has been delivered for this input.

Minimal fixture, with `project/` and `package/` as sibling directories:

```moth
-- project/config.moth
project #= (
    name = "package_collision_campaign",
    entry_root = "src",
)
html #= ()

-- project/src/@page.moth
@binding.js provider_value as project_value
@campaign package_value
[:project=[project_value()]|package=[package_value()]]

-- package/@mod.moth
export:
    @binding.js provider_value as package_value
;
```

```js
// project/src/binding.js
/** @moth.sig provider_value || -> String */
export function projectValue() { return "project:17"; }

// package/binding.js
/** @moth.sig provider_value || -> String */
export function packageValue() { return "package:29"; }
```

Register `package/` as Builder source package `campaign` on the production HTML surface, then call `build_project` on `project/`. The following temporary test belongs at the end of `src/build_system/tests/build_dependency_tests.rs`, which already imports the build APIs and helpers. Set `MOTH_JS_LINKING_REPRO_ROOT` to the fixture's parent directory. It expects success rather than pinning the defect:

```rust
struct CollisionCampaignBuilder {
    package_root: PathBuf,
}

impl BackendBuilder for CollisionCampaignBuilder {
    fn frontend_surface(&self) -> BuilderSurface {
        let mut surface = HtmlProjectBuilder::new().frontend_surface();
        surface.source_packages.register_filesystem_root(
            "campaign",
            self.package_root.clone(),
            PackageOrigin::Builder,
        );
        surface
    }

    fn frontend_style_directives(&self) -> Vec<StyleDirectiveSpec> {
        HtmlProjectBuilder::new().frontend_style_directives()
    }

    fn validate_project_config(
        &self,
        config: &Config,
        strings: &mut StringTable,
    ) -> Result<(), ProjectConfigError> {
        HtmlProjectBuilder::new().validate_project_config(config, strings)
    }

    fn build_backend(
        &self,
        compilation: ProjectCompilation,
        config: &Config,
        profile: BuildProfile,
        flags: &[Flag],
        strings: &mut StringTable,
    ) -> Result<Project, CompilerMessages> {
        HtmlProjectBuilder::new().build_backend(compilation, config, profile, flags, strings)
    }
}

#[test]
fn package_campaign_exact_logical_path_collision() {
    let root = PathBuf::from(
        std::env::var_os("MOTH_JS_LINKING_REPRO_ROOT").expect("set the fixture parent"),
    );
    let builder = ProjectBuilder::new(Box::new(CollisionCampaignBuilder {
        package_root: root.join("package"),
    }));
    let mut built = build_project(
        &builder,
        root.join("project").to_str().expect("fixture path is UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    )
    .expect("distinct source boundaries must retain distinct providers");
    let output_root = root.join("output");
    write_project_outputs(
        &built.project,
        &always_write_options(output_root.clone(), None),
    )
    .expect("the central writer must retain both provider assets");
    let mut actual = built
        .project
        .deferred_resources
        .iter()
        .map(|asset| fs::read(output_root.join(&asset.relative_output_path)).unwrap())
        .collect::<Vec<_>>();
    actual.sort();
    let mut expected = vec![
        fs::read(root.join("project/src/binding.js")).unwrap(),
        fs::read(root.join("package/binding.js")).unwrap(),
    ];
    expected.sort();
    assert_eq!(actual, expected, "both exact byte sources must survive");
    let rendered =
        crate::compiler_tests::integration_test_runner::assertions::execute_html_harness_for_test(
            &mut built,
        )
        .expect("the emitted module graph must execute");
    assert_eq!(rendered.runtime_error_message(), None);
    assert_eq!(rendered.combined_output(), "project=project:17|package=package:29");
}
```

Actual commands used the preserved fully qualified temporary probe, `tmp/js-linking-package-campaign-collision/probe.rs`. Current-tree execution used the mixed Cargo command above. Baseline execution temporarily included that absolute probe path in the isolated `aa072a27` compiler and ran:

```sh
env PATH=/Users/aneirinjames/.nvm/versions/node/v24.21.0/bin:$PATH \
  MOTH_JS_LINKING_REPRO_ROOT=/Users/aneirinjames/projects/beanstalk/moth/tmp/js-linking-package-campaign-collision \
  cargo test --manifest-path tmp/js-linking-baseline-compiler/Cargo.toml \
  --target-dir /Users/aneirinjames/projects/beanstalk/moth/tmp/js-linking-baseline-target \
  -p moth --lib package_campaign_exact_logical_path_collision -- --nocapture
```

Raw receipts: `artifact://252` for the corrected candidate and `artifact://255` for activation. The permanent fixture above preserves the collision independently of ignored local artifacts.

**Disposition: confirmed pre-existing compiler defect, deferred to a source-package provider-identity slice.** `discovery_provider_imports.rs` constructs `ExternalImportRequest` with a boundary-relative logical path but no stable source-package identity. `js_import_provider.rs` derives the same ProjectLocal binding key from both paths, and `ExternalPackageRegistry::register_package` rejects the second. Runtime assets also derive the same provider origin/output destination. Skipping duplicate registration would neither preserve both implementations nor fix output identity.

The next slice must carry the canonical stable source-package identity, already constructed in `source_package_discovery.rs`, through the provider request, binding identity and provider resource origin/output placement. Reuse the existing injective package-prefix owner. Keep project-local output policy, alias independence and distinct byte sources intact. Validate same logical paths across project/package and two packages, canonical ID resolution, exact runtime values, output conflicts and deterministic order. This crosses the provider request and stable resource-identity contracts rather than the final-call candidate repair, so the bounded campaign preserves it instead of inventing an absolute-path prefix, flattening source topology or emitting all assets.

## Documentation reconciliation checkpoint

The candidate after `930b9c103` preserves the current cold-carrier and shared presentation contracts under the data-layout authority's existing migration section. It keeps the final failure-lane and host-isolation migration undelivered. The compiler authority points to that owner, the build authority records boundary-paired provider lookup and the index locates the aggregate/freeze and shared presentation owners. Both progress matrices now describe delivered coverage and retain the separate same-path provider-identity gap.

| Command or observation | Scope | Result |
|---|---|---|
| Explicit Node 24 `just boracle` | Final production code, opt-in feature | Formatting and Clippy passed; 46 borrow-problem, 5 last-use and 260 Boracle tests passed |
| Explicit Node 24 `cargo run --quiet -- build docs --release` | Current compiler and authored documentation | 81 output files built successfully |
| Managed Chromium on ports 9879, 9880 and 9882 | Both changed progress routes, ten tracked refreshes and two newly generated routes | Intended rows, titles and owner text rendered; screenshots captured; browser error entries empty |
| Browser requests for regenerated local owner links | Lappet and Wasm-optimisation navigation | Both HTTP 200 |
| Original/copy `diff -r` on website `src` and `cmp` on config | Supplied unchanged website | Still byte-identical |

The release build refreshed ten tracked HTML files, including eight previously stale routes and previously committed borrow-status content. It also materialised the missing Lappet and Wasm-optimisation HTML routes from existing committed sources. These outputs came from the compiler, not manual HTML edits. Their refresh publishes existing ownership text without changing those sources or activating paused work. Complete generated-content inspection and all-twelve-route browser inspection accepted that backfill. Local receipts remain at `tmp/js-linking-docs-{browser,all-routes,new-routes}-receipt.json` and `tmp/js-linking-docs-local-link-checks.json`. Tabs and temporary servers were closed.

The independent contract/status audit and expanded generated-backfill review returned clean. The expanded review initially raised two literal tracker references in the borrow-status cells at `docs/src/docs/progress/@page.moth:324` and `:332`. Baseline inspection confirmed the same plain-template source already existed at `930b9c103`, so the auditor withdrew the required finding. Those two pre-existing source-presentation defects remain a separate documentation follow-up rather than an unrelated source edit in this task. This receipt does not claim that every existing documentation link works.

Relevant structured audit rows were already stale or did not cover these owners, so the audit log remains unchanged. No new structured audit coverage was recorded.

## Final integrated validation and review

The final `just validate-full` run explicitly prepended `/Users/aneirinjames/.nvm/versions/node/v24.21.0/bin` to `PATH` and passed on macOS arm64 with the pinned Rust toolchain. The candidate used `70af91f6b` plus plan retirement and the roadmap-entry removal. Subsequent evidence-record updates do not change compiler sources, tests, build configuration, Moth documentation sources or generated outputs.

| Final gate | Observed result |
|---|---|
| Formatting and Clippy | Routine and workspace formatting passed, as did ordinary and all-target standard-feature Clippy |
| Workspace tests | 6,128 main unit tests passed, together with the CLI, library, xtask and documentation suites |
| Compiler integration and docs | 2,786/2,786 correct, then docs check with no errors or warnings |
| Standard feature matrix | All 10 executing lanes passed |
| Source, dependency and feature audits | No findings |
| Test honesty | No hard, open-hard or integrity findings. Existing dispositioned review entries and 208 suite advisories remain |
| Non-recording performance checks | All 148 benchmark cases passed shared preflight. The gate measured eight CLI and ten frontend quick cases, then passed all 14 scaling budgets |
| Timer erasure | The no-timer release binary passed the marker check |

The quick benchmark output reports changed workloads for seven of eight CLI and eight of ten frontend comparisons, so it does not establish a matched pre/post performance result. Neither budgets nor benchmark history changed. The full receipt remains in ignored `tmp/js-linking-final-validate-full.log`. The separate opt-in Boracle gate and real website, CLI, dev-server and documentation browser checks are recorded above. No Linux, Windows or remote CI result is claimed.

Six fresh read-only final lanes returned `pass`: final runtime facts and generated publication, Stage 0 namespace and HTML handoff, cold cause ownership and freezing, connected frontend transport, failure classification and presentation, and work-source completion with evidence and documentation. None reported a required correction or blocking test gap. Their static reviews consumed the executed receipts rather than rerunning validation.

The accepted implementation checkpoints are `4c19a00c8`, `1dae29ce2`, `160782190`, `8f15579ec`, `930b9c103` and `70af91f6b`. The closeout retires the finished plan and its roadmap entry while retaining permanent contracts and the larger provider-identity handoff in this report. The fresh cross-scope integration Slice review returned `pass` with no required finding after tracing the combined implementation, acceptance evidence and retirement diff. No required review finding or validation failure remains.
