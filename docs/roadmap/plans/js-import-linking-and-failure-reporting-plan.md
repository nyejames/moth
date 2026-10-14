# HTML JavaScript re-export linking and failure reporting implementation plan

> For agentic workers: implement the tasks in order using the repository's agent workflow. Use `superpowers:executing-plans` or `superpowers:subagent-driven-development` when available. Preserve the evidence gates and final Slice review. This document authorises compiler corrections, not website workarounds or resumption of other paused plans.

**Goal:** compile legal JavaScript function re-exports without losing runtime assets, distinguish compiler defects from operational failures and expose adjacent dependency bugs through meaningful regression tests.

**Architecture:** complete ordinary modules' existing runtime-import candidate inventory from final compiler-owned external-call facts. Retain exact entry reachability and existing provider/resource emission. Derive error presentation from authoritative error types, preserving separate causes when the transitional infrastructure carrier aggregates failures.

**Technology:** safe Rust, existing module/link/provider records, Moth project fixtures, the native-ES-module Node harness and current Cargo/just validation.

**Specification:** the user's accepted scope is recorded below. Permanent compiler, build, language and data-layout authorities govern implementation. This replaces the earlier plan based on `36156b40849eecad12e69b8d4e62388672a1ecff`.

## Status and inspection baseline

```text
STATUS: active on link-fixes-and-test-cleanup
CURRENT_SLICE: executable red regression established, runtime-import handoff next
BLOCKERS: supplied website archive unavailable in inspected input locations
NEXT_ACTION: complete final-call-demanded runtime candidates and checked HTML glue inputs
```

The reviewed main includes the typed-expression/native-result Phase 4 checkpoint. Its plan explicitly pauses before Phase 5. The affected source still collects ordinary-module provider candidates by source-file ownership and still loses later error types during infrastructure aggregation. The original missing-asset path remains present. Establish an executed failing baseline before changing it. [S1] [S2] [S5] [S6]

Read the activation checkout rather than trusting GitHub search snippets. During this review, code search still returned the old `36156b4` revision while explicit file fetches at `0096b78` returned the synced source. Record the actual implementation base and refresh affected owners if main advances. A changed file or a passing minimal probe requires explaining the difference, not forcing the old diagnosis onto new code.

### Corrections made by this refresh

| Earlier ambiguity | Required resolution |
|---|---|
| Generic references to finalized call facts | Read the final `HirModuleLinkFacts` after convergence. Cover statement `Call` and terminator `Invoke`. `direct_call_targets()` currently omits external calls. |
| Broad import/export case list | Keep the minimal regression, then combine related interactions into a cohesive runtime scenario. Use focused negatives and invariant tests where they protect a different failure. |
| Optional runtime execution | The current Node harness supports one inline ES module and its relative JS graph. Require execution for the no-DOM regression. Import-map cases retain a separate artifact contract. |
| Unspecified duplicate compatibility | Compare runtime facts, not authored spans. Preserve provenance separately. Reject incompatible definitions instead of choosing the first or last. |
| Conditional investigation of mixed failures | `set_infrastructure_error` already concatenates messages and loses the incoming type. Make preservation of separate causes a required part of reporting. |
| Broad cross-package success implication | Preserve the same-project fix's scope. Probe registry-boundary identity explicitly and report independent defects without claiming they were repaired. |
| Future language syntax leaking into fixtures | Use delivered config, export and generic syntax. Keep the queued directive, generics/JS-WAT and failure-layout programmes inactive. |

## Global constraints

- Fix the compiler while preserving the website's authored Moth and JavaScript. Add no Moth wrapper, relocated import, special filename case or visibility workaround.
- Keep one producing owner for call facts, provider metadata, resource placement and failure classification. Add no second import graph, backend source scan or AST/TIR traversal.
- Preserve source/module visibility, dormant root behaviour, canonical binding identity, numerical semantics and native success/error channels.
- Published module artifacts stay immutable. Build code selects complete facts and does not mutate HIR, install summaries or rerun semantic stages.
- Preserve graph/input membership independently from output liveness. An unused provider may still be discovered and validated without emitting its runtime asset.
- Keep `ExternalPackageId` and `ExternalFunctionId` paired with their issuing registry. Never join independently issued numeric IDs in one unqualified map.
- Preserve `MOTH-INFRA-0001`, the five-field terse record and existing command/status semantics for the current transitional carrier.
- User mistakes remain typed diagnostics. Expected IO, provider, process and host failures are operational. Proven Moth invariants identify compiler/build-system defects.
- Keep the final three-lane design intact. This slice does not implement the full `InfrastructureFailure`/`compiler_bug!` migration, a new panic hook or new host isolation.
- Use current tests and helpers before adding new ones. Keep fixtures under their owning test directories and temporary probes under ignored `./tmp`.
- Land one validated main commit without force-pushing, overwriting concurrent work or resuming expression-refactor. Repository changes are implementation deliverables, not part of this read-only plan refresh.

### Required reading at activation

Read `AGENTS.md`, `docs/src/developer-docs/style-guide/style-guide.mtf` and the language cheatsheet. Read the affected testing/validation sections before selecting fixtures or commands. Use `index.md` to locate moved owners. Also read:

- `docs/compiler-design-overview.md`, including module outcomes, binding identities, final link facts, native results, assertion validation and backend handoff
- `docs/build-system-design.md`, including boundaries, entry assembly, external JavaScript, resource placement and output ownership
- `docs/compiler-data-layout-design.md` > `Failure architecture` and `Tooling host isolation`, expanding to the full authority for storage/context changes
- The canonical package dependency, public re-export, external binding and function/error references selected by `docs/src/developer-docs/language/overview.mtf`
- Both progress matrices, the current roadmap and the status/ownership sections of the typed-expression and diagnostic-layout plans
- `docs/roadmap/plans/generics-core-bindings-and-file-backed-js-wat-plan.md` for its queued boundary only

Permanent authorities own semantics. Plans record delivery and approved task decisions. Resolve any explicit authority conflict before relying on it and report stale status separately. This plan authorises narrowly necessary documentation corrections, status/index maintenance and the implementation findings report, not a general documentation rewrite. [S1] [S3] [S4]

## Review focus

| Risk | Defence and owner |
|---|---|
| A legal re-export resolves a callable but loses its runtime asset | Task 1's minimal executable regression and Task 2's complete candidate join |
| A new call-fact query misses fallible calls or retains validation-only work | Task 2's `Call`/`Invoke` coverage and Task 3's assertion/static-inactivity cases |
| Aliases or repeated imports cause duplicated, conflicting or misrouted assets | Task 2's semantic metadata comparison and Task 3's topology/output assertions |
| A banner mislabels operational failures or hides a later compiler defect | Task 4's typed classification, mixed-cause storage and complete renderer tests |
| Another registry or test-harness limitation produces a false pass | Task 5's boundary probes and explicit separation of compiler, harness and unsupported-capability outcomes |

## 1. Establish an executable baseline

**Owners:** the uploaded website, `tests/cases/`, `src/build_system/tests/build_dependency_tests.rs` and `src/compiler_tests/integration_test_runner/assertions/`.

- [x] Record the checkout revision, dirty state, Rust toolchain, Node version, command and build profile. Read the repository's pinned toolchain files. Keep baseline logs under `./tmp`.
- [ ] Build a disposable copy of the supplied website with unchanged source. Its archive has `config.moth` at the repository root, `src/@page.moth` and `src/components/@components.moth`. The page calls the directly re-exported `animate_canvas` from `src/components/background-animation.js`.
- [x] Reproduce with the small project below. Record whether failure is the expected missing runtime asset, an earlier independent diagnostic or a harness problem.
- [x] Before patching, add a permanent regression whose post-fix expectation is successful execution. Demonstrate its failure on the baseline. Preserve the smallest source shape even when adding a richer companion scenario.

### Minimal project, using delivered syntax

Place these files under a new integration case's `input/` directory. Check for an existing primary contract before assigning the proposed ID `html_js_reexport_linking`.

`config.moth`:

```moth
project #= (
    name = "js_reexport_probe",
    entry_root = "src",
)
html #= ()
```

`src/@page.moth`:

```moth
@components animate_canvas, observed

animate_canvas("canvas")
[:[observed()]]
```

`src/components/@components.moth`:

```moth
export:
    @background-animation.js animate_canvas, observed
;
```

`src/components/background-animation.js`:

```javascript
let lastCanvasId = "";
let invocationCount = 0;

/** @moth.sig animate_canvas |canvas_id String| -> */
export function animate_canvas(canvas_id) {
    lastCanvasId = canvas_id;
    invocationCount += 1;
}

/** @moth.sig observed || -> String */
export function observed() {
    return `${lastCanvasId}:${invocationCount}`;
}
```

`expect.toml`:

```toml
entry = "."

[backends.html]
mode = "success"
warnings = "forbid"
rendered_output_exact = "canvas:1"
```

After checking the primary-owner inventory, register the proposed case in `tests/cases/manifest.toml`:

```toml
[[case]]
id = "html_js_reexport_linking"
path = "html_js_reexport_linking"
tags = ["integration", "imports", "external-js"]
contract = "build.external_js.reexport_runtime_linking"
role = "primary"
```

Reuse an existing primary contract when it already owns this behaviour. Any intentional existing warning needs an exact warning contract, not a blanket ignore.

This source deliberately has no direct provider clause in the page, no callable Moth body in the facade, no DOM dependency and no `@moth/runtime` import. The observer is also re-exported so it cannot accidentally supply the missing page-owned candidate. Its output checks both the argument and exactly-once invocation. The currently delivered config is verified by the existing build test. Do not replace it with future `$project`/`$page` forms during this repair. [S7] [S8]

### Facts to inspect before fixing

| Boundary | Expected baseline observation |
|---|---|
| JS provider | The annotated function, owning package and `Some(RuntimeAssetIdentity)` are registered. |
| Components artifact | Its source-owned candidates contain the JS package. |
| Page artifact | Final executable call facts contain the external calls, but its source-only candidates can omit that package. |
| Entry assembly | Reachability contains the external function. A facade with only re-exports has no Moth function edge requiring it in `linked_modules`. |
| Glue input | An emitted external function has no matching entry runtime-asset path. |

Confirm the first lost fact with a focused test or temporary instrumentation. Remove instrumentation before landing. The failed glue lookup is a metadata failure, not proof that the JS file is absent. The source-inspected chain is `service.rs` -> entry assembly in `build.rs` -> `js_path.rs` -> `runtime_glue/source.rs`. [S2] [S5] [S9]

**Exit gate:** an executed red regression for the observed failure, with the actual base revision recorded. A baseline that already passes requires adjusting the remaining work to the proven gaps rather than manufacturing a regression.

## 2. Complete the existing runtime-import handoff

### Ownership and interfaces

| File | Change |
|---|---|
| `src/compiler_frontend/hir/reachability.rs` | Add a narrow borrowed accessor over retained executable external-call facts. |
| `src/compiler_frontend/module_compilation/external_imports.rs` | Join source-owned and final-call-demanded package metadata through one canonical lookup/merge owner. |
| `src/compiler_frontend/module_compilation/service.rs` | Invoke that collector after final generated-summary convergence and before module publication. |
| `src/compiler_frontend/module_compilation/generated/materialisation.rs` | Reuse the strengthened package lookup and preserve generated-root selection. |
| `src/builder_surface/external_import_providers/resolution_table.rs` | Change only when needed to borrow canonical records without redundant cloning or expose a checked lookup. |
| `src/projects/html_project/external_js/runtime_glue/` | Validate the selected HTML glue inputs and report missing facts precisely. |
| `src/build_system/build.rs` | Preserve exact entry selection. Modify only a necessary checked handoff, not executable graph topology. |

The following is a **proposed new API**, not an existing method:

```rust
pub(crate) fn direct_external_calls(
    &self,
) -> impl Iterator<Item = &ReachableExternalCall>;
```

Add it to `HirModuleLinkFacts`. Flatten the existing block records' `reachable_external_calls` in retained deterministic order. It borrows facts and creates no new persistent side table. The current `direct_call_targets()` returns only Moth local/cross-module/private/generated targets and is unsuitable for this job. Leave that method's convergence contract unchanged. [S10]

### 2.1 Select packages from complete executable facts

- [ ] Add an accessor test through real lowered code showing both `HirStatementKind::Call` and `HirTerminator::Invoke` contribute external call facts. The current collector already funnels both through `collect_call_target`. Reuse those facts rather than creating another HIR walk. [S10]
- [ ] Use `function_link_facts` only after `run_generated_summary_convergence` returns. Keep the existing freeze/publication order. Preserve the Boracle prefix's early return rather than demanding production runtime metadata from an analysis-only result.
- [ ] Resolve each external function through the module's effective registry, validating its definition and owning package. Deduplicate requested packages in that registry domain. An unknown finalized callable is an internal error, not an absent optional dependency.
- [ ] Extend the source-file-derived candidate set with packages demanded by final external calls. Keep directly owned source candidates and builder runtime metadata as conservative availability facts. This preserves facade/export-only and existing tooling consumers without making them live outputs.
- [ ] Evolve/rename `collect_external_import_candidates_for_source_files` to reflect this broader responsibility and update its caller. Reuse `collect_external_import_candidates_for_packages` as the canonical projection path. Remove an obsolete forwarding helper if the refactor makes it unnecessary.
- [ ] Return checked failure from the shared collector where metadata conflicts. Publish into the existing `ModuleLinkFacts.external_import_candidates`. Keep one record per package within that module's registry domain.

Process each module's retained call facts once. Gather package identities before cloning output records. The existing source-based and package-based collectors are the starting owners, not two independently maintained policies. [S11] Avoid a full reachability result for every possible root, per-call provider rescans or cloning complete `ResolvedExternalImport` values just to discard diagnostics and exported-symbol lists.

An absent runtime record is not universally an error. Inline/Core/runtime-helper lowerings can legitimately have no provider asset. Keep generic collection backend-neutral. HTML's selected `ExternalModuleExport` consumer owns the requirement for a JS asset and the distinction from an ordinary no-asset helper.

### 2.2 Define metadata compatibility precisely

The same package may be encountered through direct ownership, final call facts and shared builder metadata. For that **same registry-owned package identity**, compare:

- asset presence, stable resource origin, asset kind and already-resolved canonical byte-source path
- required runtime module names and their imported-name sets, with comparison independent of incidental ordering/duplicate names

An authored `source_span` is provenance, not runtime identity. Different import locations for the same package are compatible. Preserve the canonical provider record's provenance and retain both useful locations when reporting an actual conflict. Do not choose a span using unordered-map iteration or compare source IDs across independent domains.

Conflicting runtime facts fail explicitly. In particular, `Some(asset)` versus `None`, different origins/byte sources or different required runtime contracts cannot be hidden by `dedup_by_key`, `or_insert` or provider-first fallback. Compare already retained facts without new filesystem reads, content hashing or source parsing. Apply this rule at the one merge owner rather than implementing equality checks in every renderer.

- [ ] Test compatible repeated references with different authored spans.
- [ ] Inject a conflicting provider/builder record and assert an internal error naming the package and conflicting fact.
- [ ] Retain a no-asset Core helper control case so strictness does not reject legitimate inline/runtime implementations.

### 2.3 Preserve entry liveness and generated convergence

Keep the existing `collect_reachable_external_package_ids` and entry filtering over final executable reachability. Supplying a candidate must not activate a facade's dormant root, add a Moth function to `linked_modules` or emit every candidate. [S5]

Generated sidecars already use package-derived metadata. Inspect their final publication in `module_compilation/generated/convergence/` as well as initial materialisation. Preserve complete candidates if convergence changes call form. Any newly demanded package must be resolved before publication, while removed validation-only work must not become an entry runtime dependency. Reuse the same lookup policy rather than adding a generated-only fallback. [S12]

The synced AST service retains `assertion_validation_generic_requests` separately from executable demand. A true assertion's message can require semantic validation without requiring its external asset at runtime. This is a mandatory liveness regression, not an invitation to alter assertion validation.

### 2.4 Check the selected HTML handoff

Validate selected external exports in the existing `generate_module_glue`/referenced-export preparation path before source emission. Reuse that work rather than adding a second graph walk. Keep the final source generator's defensive lookup.

- [ ] A referenced emitted function has a definition and applicable JS lowering in the registry used to interpret it. The current `collect_referenced_exports` silently skips missing definitions/lowerings. Replace that behaviour for broken closed inputs with an explicit internal error. Valid non-module JS lowerings still need no wrapper. [S13]
- [ ] Every selected `ExternalModuleExport` resolves to compatible entry metadata with a JS runtime asset. Preserve normal typed target rejection before this boundary for a genuinely unsupported source call.
- [ ] Error details identify the failing stage, entry or calling module where available, Moth function/JS export, readable package path and build-local IDs. Resolve these from existing records. Add only the narrow context the failure actually needs.
- [ ] Distinguish missing candidate, candidate with no asset and wrong asset kind where those facts are available. State that validated runtime metadata is inconsistent. Keep developer detail after the shared compiler-bug introduction from Task 4.
- [ ] Inject unknown-function, missing-asset and wrong-kind inputs at the owning boundary. Assert the internal failure instead of allowing a missing wrapper or browser import error.

Registry identity must remain correct through the whole join, including the merged `referenced_external_functions` in `js_path.rs`. This task fixes the same-project missing-candidate path. It does not authorise an unqualified build-global registry merge. Task 5 owns evidence for any existing cross-package weakness.

**Exit gate:** the Task 1 regression executes successfully, selected assets/glue are present and existing unused/constant-only controls remain absent. No website or visibility workaround is part of the patch.

## 3. Add topology and liveness regression coverage

Use the revised testing standards' admission rule: identify the credible failure, the catching assertion and the existing gap before adding each test. The rows below describe required coverage across the suite, not one new fixture per row. Preserve the small real-bug regression and extend one cohesive no-DOM binding scenario for related interactions. [S7]

| Contract | Required test observation |
|---|---|
| Direct import baseline | Existing direct-call build continues to emit exactly one provider asset and correct glue. |
| Pure one-hop and two-hop Moth re-exports | Calls execute without intermediate callable Moth bodies or activation of dormant provider roots. |
| Alias and source-namespace access | Different Moth public/local names call the annotated JS export. Renaming a binding changes neither asset identity nor result. |
| Several legal paths to one external symbol | One raw JS module instance per page and one wrapper per demanded external function in that page's glue. Check exact invocation/state results. |
| Infallible and fallible calls | Execute both statement-call and `Invoke` paths, including successful return and caught JS `{ ok: false, error: ... }` result. Check exactly-once effects and error code/message. |
| Mixed constant/function JS module | Constants retain exact values/types. A surviving function call emits the asset once. A separate constant-only selection emits no asset, wrapper or runtime helper. |
| Unused or inactive external work | An unused private wrapper, a statically removed branch and a true assertion message introduce no selected runtime asset. Keep their normal frontend validation. |
| Generated execution and validation-only generation | A concrete generated function's executed external call works. A validation-only generic assertion message contributes no runtime module or wrapper. |
| Distinct files with the same basename | Correct functions/results and distinct logical outputs, not last-wins package/path lookup. |
| Multiple entries | One shared build-level provider asset, valid relative glue imports for each page and only that page's demanded wrappers. Check per-page runtime behaviour in separate executions where the current harness can select that page. Record any route-selection harness gap separately from the mandatory per-entry artifact checks. |
| Invalid dependency/annotation/export selection | Exact existing typed diagnostic code and source location, never an infrastructure banner. Use focused negative cases. |

Use a fallible JS fixture returning the supported result object directly to avoid an import map in the runtime scenario. Preserve the current JS ABI adapter. Native HIR results do not authorise changing foreign return conventions, adding tuple-like source types or duplicating error packing in this fix. A caught JS error is program execution, not a compiler infrastructure failure. [S9]

### Artifact and emission assertions

Reuse `BuiltOutputs` for existing build tests and `BuiltArtifactIndex` for runner assertions. Add any new cross-stage Rust harness under `src/compiler_tests/` when no smaller owner suffices. Keep data-only invariant tests beside their production module. Avoid relocating unrelated old tests.

For the minimal fixture, the declared provider output is `_moth/js/components/background-animation.js`. Its glue lives under `_moth/js/glue/`, so the import resolves to `../components/background-animation.js`. Verify the page imports the actual emitted glue file rather than reconstructing its hash. Dynamic numeric IDs and absolute-checkout-derived glue names are not golden contracts. [S14]

Check planned output records, deferred resources and materialised bytes at the appropriate owner. A correct path in a string is insufficient if no corresponding output is written. Preserve deferred reads and the writer's conflict preflight. Reuse existing tests that prove those boundaries instead of duplicating the writer suite.

Distinguish **asset granularity** from **wrapper granularity**. A live function emits its whole authored JS file, including unused exports and its registered runtime imports. This repair does not tree-shake that file. An unused function requires no wrapper, but cannot make its containing live asset disappear.

The Node harness stages a validated relative JS graph and runs one inline module. It rejects import maps, mixed classic/module pages and multiple module scripts. Keep the main scenario within that supported shape. Extend the existing runtime-import-map artifact test for a facade re-export, while explicitly reporting that the normal harness does not execute that map. A harness `TypeError`, staging failure or missing Node is not a successful runtime assertion or evidence that Moth should reject the source. [S7] [S15]

**Exit gate:** required contracts have identified primary owners, actual execution evidence where supported and no unnecessary fixture-per-spelling matrix. Generated numeric suffixes, incidental helper counts and whole-page text remain non-contractual. Exact asset and demanded-wrapper counts remain required where they prove deduplication.

## 4. Report compiler defects and operational failures truthfully

### 4.1 Classify causes before choosing wording

The current `CompilerError` remains the rare outer carrier beside typed diagnostics. Refine its existing `ErrorType` only where inspection proves a mixed category. Failure classification describes the cause, not just the subsystem in which it surfaced. Derive a small presentation class exhaustively from that authoritative type. Add no stored `is_compiler_bug` flag and no message-text heuristics. [S6]

| Existing cause | Required classification |
|---|---|
| `Compiler` | Internal Moth compiler/build-system defect, after correcting any misclassified operational producer found in this task |
| `HirTransformation` | Internal invariant failure |
| `Backend(LirTransformation)` / `Backend(WasmGeneration)` | Internal lowering/emission invariant, with ordinary capability rejection kept in typed diagnostics |
| `File` | Operational filesystem/path/read/write failure |
| `DevServer` | Operational host/bind/watch failure, unless an internal defect was incorrectly relabelled on the way out |
| `Config` | Inventory actual producers. Authored configuration errors belong to typed diagnostics and config IO belongs to operational failure. Remove this variant if it has no production owner. |

- [ ] Inventory constructors and reclassification wrappers with repository search, recording path/function, actual cause and final class. Prioritise build, provider, output, frontend load, backend invocation and dev-server boundaries. Search results from an old branch are not an inventory.
- [ ] Correct proven misclassifications before their new banner ships. A source path that does not exist, permission denial, tool spawn failure, port bind failure or provider invocation failure is not proof of a Moth bug. A validated missing link record or invalid compiler-generated Wasm is an invariant failure.
- [ ] Preserve the original class when adding stage/host context. Audit `.with_error_type(...)` and broad adapter helpers in `src/projects/dev_server/server.rs` and `build_loop.rs` so an internal compiler failure cannot become merely a dev-server operation failure.
- [ ] Keep this bounded to the affected outer carrier and its classification boundary. Report unrelated producer cleanup separately. An unresolved classification on a changed path is a blocker, not permission to guess or launch the full layout migration.

### 4.2 Preserve mixed failures before rendering them

The current `CompilerMessages::set_infrastructure_error` concatenates messages, keeps the first `error_type` and merges unrelated path/span/metadata fields. With different cause classes, a single introduction would misdescribe part of that message. This is a source-confirmed defect in the aggregation operation even before finding a real-world double-failure fixture. [S6]

Use one cold aggregate beside the diagnostic stream. The preferred concrete change is to keep the existing outer optional box but let it hold a crate-private `InfrastructureErrors` with `primary: CompilerError` and `additional: Vec<CompilerError>`. The vector is empty for the normal single-failure case. It stores actual causes, not pre-rendered strings or another diagnostic lane.

- [ ] Keep each cause's message, type, optional host path, source provenance and metadata intact. Preserve production order with the first cause as primary. Append another aggregate's causes in order rather than nesting aggregates.
- [ ] Update construction, append/freeze transport, render access and tests together. A primary-cause accessor remains useful where a consumer explicitly needs the first cause. Renderers must iterate the whole aggregate.
- [ ] Keep `CompilerMessages`' current compact-size invariant and existing infrastructure-event count/status policy. Several related causes still represent one aborted compilation, not additional user diagnostics.
- [ ] Keep the matching message/report context through existing transport. Never splice one cause's span into another cause's host path or invent a source frame from an unpaired compact ID.
- [ ] Test compiler->operational, operational->compiler and same-class sequences, both through the setter and a real aggregate append path. The first cause and every additional cause must keep their own facts and classification.

This small cold collection fixes information loss. It is not the final failure architecture. The eventual failure-lane migration still owns separate operational results, self-contained compiler-bug reports and worker unwinding. Do not introduce those mechanisms here.

### 4.3 One presentation owner, three consumers

Put shared classification and introduction text under `compiler_messages/render/`, for example a small `infrastructure.rs`, and reuse it from terminal display, terse output and dev-server HTML. Keep presentation separate from `CompilerError.msg`. Move existing generic guidance there only if doing so removes the current renderer-to-terminal-helper coupling, updating all callers and deleting the old forwarding path.

Use the following compiler-defect introduction:

```text
COMPILER BUG: Moth encountered an internal compiler or build-system error.
This is a Moth toolchain defect, not a source-code error. Please report it at:
https://github.com/nyejames/moth/issues

Details:
<original failure message>
```

Use the following operational introduction:

```text
OPERATIONAL FAILURE: Moth could not complete a filesystem, host or tooling operation.

Details:
<original failure message>
```

Keep actionable paths, OS/process details and stage/guidance text. A failed third-party tool is operational until evidence establishes a Moth invariant failure. Include the compiler version/revision only if an existing build-version owner supplies it. Rendering must perform no network lookup or new repository scan.

| Consumer | Required contract |
|---|---|
| Terminal `display_messages.rs` | Introduction precedes original detail. Preserve specific stage/guidance. Use a neutral filesystem heading instead of `Missing File or Directory` for every `File` error. Render additional causes as separately labelled sections. |
| `render/dev_server.rs` | Keep the infrastructure kind and `data-diagnostic-code="MOTH-INFRA-0001"`. Put the correct introduction before details and escape all error-derived text. Use a fixed clickable report URL for compiler defects. Render additional causes within the one failure card. |
| `render/terse.rs` | Keep one `E|MOTH-INFRA-0001|path||message` record and exactly five fields. Prefix each cause's part of the sanitized message with its class. The path field belongs to the primary cause, while additional paths remain in their labelled message sections. No extra records, delimiters or physical newlines. |

For a single failure, emit one introduction. Include the fixed report URL in a terse record containing a compiler defect, once per record. For several causes, identify each cause once so an operational primary cannot hide a later compiler defect or make it look user-fixable. Normal typed diagnostic/warning ordering and `benchmark_diagnostic_counts` remain unchanged. Language runtime `Error` values and `RenderHarnessError` retain their separate owners.

### 4.4 Reporting tests

Extend the existing `display_messages_tests.rs`, `diagnostic_model_tests.rs` and command-status tests where they own the assertions. Add a small local table for exhaustive type classification rather than many duplicate fixtures.

- [ ] Every internal category renders the compiler-defect introduction and report URL through terminal, terse and dev-server paths.
- [ ] Missing/read/write/path errors and host operation failures render as operational, preserving the cause without compiler-blame text.
- [ ] Normal malformed import, config and type diagnostics keep their codes/source labels and have neither infrastructure introduction.
- [ ] Mixed-cause aggregation preserves both categories, messages, paths and guidance in order. Inspect final rendered output, not merely the classification helper.
- [ ] Hostile details containing `<`, `&`, quotes, pipes and line breaks remain safe HTML and one valid terse record. Preserve meaningful developer text after sanitisation.
- [ ] Infrastructure errors still cause failure status and suppress the benchmark diagnostic-count record. Error/warning ordering and the raw stored messages are unchanged by rendering.
- [ ] Use portable injected IO errors for permission-denied coverage rather than assuming `chmod` fails under a privileged test runner. Exercise at least one real failing filesystem/host boundary where an existing harness permits it. Add no production fault-injection switch.

**Exit gate:** the same internal metadata failure is clearly labelled a compiler bug in all three views, genuine operational failures are not blamed on the compiler and mixed failures retain their individual provenance.

## 5. Run a bounded adjacent-defect campaign

This is a required implementation investigation, not a claim that the following suspected failures have already been reproduced. Run the named probes and record every result. Use compact table-driven fixtures where useful and retain only tests that protect a distinct contract.

| Probe | First owners to inspect and required evidence |
|---|---|
| Registry-boundary collision | `build.rs`'s entry import union, `js_path.rs`'s merged external-function set and `runtime_glue/exports.rs`'s use of the entry registry. Establish whether project/source-package functions actually share or remap registry IDs. Use two different external implementations with distinguishable results. |
| Cross-package metadata | Follow a legal source-package facade to an annotated JS callable. Prove which registry and provider table own it. Equal IDs from separately constructed registries must never be treated as equal symbols merely because their integers match. |
| Support packages and deep facades | Exercise a legal scoped support facade and a two-hop normal-child re-export. A direct normal-sibling import remains invalid and is a negative control, not a requested topology extension. |
| Asset-only rebuild | Change JS implementation bytes without changing annotations and rebuild. Verify new bytes reach the output. Measure reuse only if an actual reuse path exists, rather than asserting final-design incrementality is implemented. |
| Signature/removal rebuild | Change a signature, remove an export and remove the last reachable caller in separate builds. Verify normal diagnostics, no stale glue and normal manifest cleanup of the last asset. |
| Native results and assertion demand | Exercise an external call inside a concrete generated function, an active assertion message and an inert validation-only generic message. Inspect final published demand after convergence. |
| HTML-Wasm | Determine current support from actual target validation. Demand successful JS-affinity routing only where delivered. Otherwise require the appropriate typed unsupported-target diagnostic, not missing-asset infrastructure. Do not implement mixed partitioning here. |
| Runtime imports and resources | Probe a live JS asset requiring `@moth/runtime`, ordinary `.js` resource use versus provider use and output conflicts. Preserve distinct resource origins where the contracts require them. |
| Determinism | Repeat identical builds and vary legal discovery/publication order. Compare semantic resolution, asset paths, wrapper ordering and errors. Do not require glue hashes based on absolute entry paths to match across relocated checkouts. |

Fix blockers of the required same-project or reporting contracts and small defects caused by the patch. For an independent larger issue, preserve a minimal reproducer and an actionable next-wave recommendation. Never hide a new defect by widening visibility, emitting all assets or accepting a generic failure.

Write one durable implementation report, for example `docs/roadmap/evidence/js-import-linking-and-failure-reporting-review.md`, unless the repository already provides an owning findings file. This is implementation evidence, not a new structured audit run or authority to mark audit coverage current.

Each row records: case, actual command, revision/profile, fixture, result, expected contract, first failing owner, evidence and disposition. Results distinguish pass, confirmed compiler defect, suspected risk, accepted-but-undelivered capability, outside-design case, harness/environment failure and not exercised. A confirmed new defect needs reproduction steps and an observed failure. Record remaining checks explicitly instead of an empty success report.

## 6. Validate, document and land

- [ ] Run focused tests after each coherent task. Keep the first red result and the corresponding green result. Ensure every filter selects the intended nonzero test set.
- [ ] Rebuild the unchanged website in dev and release with the patched compiler. Verify `background-animation.js`, the actual glue import and the HTML import chain. A browser check of the animation is useful additional evidence, not a replacement for deterministic automated execution.
- [ ] If another website bug blocks the full build, record it separately. Keep the compiler regression green and make no claim that the whole site passes until it does. The website remains unchanged.
- [ ] Review the real provider/table path, output deduplication, final generated call facts, classification wrappers and aggregate transport. Check no new AST dependency, HIR mutation or eager asset read was introduced.
- [ ] Run `cargo fmt --all` for Rust changes and the targeted Cargo/integration commands below. Run affected feature lanes when their owners changed, including the opt-in Boracle check if this patch touches its prefix or feature-gated compilation path.
- [ ] Update narrowly stale comments, authorised permanent contracts and delivered-status entries. Update `index.md` only for an added/moved owner or a fundamental responsibility change. Rebuild generated documentation through the compiler when required.
- [ ] Run `just validate-full` on the final integrated tree before its main commit. Record all failures and their classification. A previous branch's CI result is not evidence for the new tree. Never suppress a required gate or claim unexecuted platform coverage.
- [ ] Produce one cohesive main commit with explicit changed paths, baseline/final revisions, test evidence and the Task 5 report. Use the repository's allowed landing workflow without rewriting main history. Leave all other paused programmes paused.

### Focused commands

Existing test and runner names at the review baseline:

```bash
cargo test -p moth --lib build_html_project_local_js_import_emits_generated_glue
cargo test -p moth --lib build_dependency_tests
cargo test -p moth --lib provider_sources_tests
cargo test -p moth --lib runtime_glue_tests
cargo test -p moth --lib display_messages_tests
cargo test -p moth --lib diagnostic_model_tests
cargo test -p moth --lib command_status_tests
cargo run --quiet -- tests --list --tag integration
```

After adding the proposed canonical case ID, run:

```bash
cargo run --quiet -- tests --case html_js_reexport_linking --backend html --terse
```

Replace that ID only when the admission review deliberately extends an existing primary owner and record the chosen ID. Use additional exact case IDs for admitted scenarios/negatives. Follow current `validation.mtf` for checkpoint, docs, feature and full gates rather than running every broad command after each small edit.

### Acceptance checklist

- [ ] A permanent red-to-green regression proves pure Moth re-exports retain callable JS runtime assets.
- [ ] Both infallible calls and native fallible `Invoke` calls execute correctly through the existing foreign ABI.
- [ ] Runtime metadata stays complete after convergence, while constants, inactive code and validation-only work cause no extra runtime output.
- [ ] Compatible repeated references deduplicate without rejecting different source spans. Incompatible runtime facts fail explicitly.
- [ ] Assets, per-page wrappers and actual relative imports have exact output/runtime evidence at their owning granularity.
- [ ] Compiler bugs, operational failures and typed source errors remain distinct in terminal, terse and dev-server presentation.
- [ ] Aggregating failures preserves every cause's type and provenance without changing the terse/status/count contract.
- [ ] The unchanged website is rebuilt or an independently reproduced remaining blocker is explicitly reported.
- [ ] Every adjacent probe has an honest result and independent larger defects have a next-wave handoff.
- [ ] The final tree has the required validation evidence for landing on main, with no claimed unrun gates.

## Final Slice review

End implementation with the repository Slice review and the supplied `moth-code-review-guide.md`. Inspect the final diff rather than relying on the implementation summary.

Check that the change has one candidate join, one runtime-metadata compatibility rule and one presentation policy. Remove obsolete helpers, duplicate projections, unnecessary clones, stale comments and replaced fixtures only when their distinct contracts remain protected. Confirm no unrelated expression, generic, ABI, panic, host-isolation or package-topology work entered the commit.

Review every new test against its named failure and catching assertion. Distinguish code-inspected risks from executed defects. Confirm the main landing tree matches the validated tree. Record the changed files, successful checks, blocked checks and exact remaining findings. If this plan is placed in the repository roadmap, follow its completion rule by deleting the finished plan and entry in the completion commit rather than committing a permanently completed plan.

## Source inspection receipt

All repository references below are pinned to the reviewed main. These support the plan refresh, not claims that implementation or tests have run.

[S1]: https://github.com/nyejames/moth/blob/0096b781b45e347dfd545ccf504a654a8e3767bf/docs/roadmap/plans/typed-semantic-expressions-plan.md
[S2]: https://github.com/nyejames/moth/blob/0096b781b45e347dfd545ccf504a654a8e3767bf/src/compiler_frontend/module_compilation/service.rs
[S3]: https://github.com/nyejames/moth/blob/0096b781b45e347dfd545ccf504a654a8e3767bf/AGENTS.md
[S4]: https://github.com/nyejames/moth/blob/0096b781b45e347dfd545ccf504a654a8e3767bf/docs/roadmap/plans/generics-core-bindings-and-file-backed-js-wat-plan.md
[S5]: https://github.com/nyejames/moth/blob/0096b781b45e347dfd545ccf504a654a8e3767bf/src/build_system/build.rs
[S6]: https://github.com/nyejames/moth/blob/0096b781b45e347dfd545ccf504a654a8e3767bf/src/compiler_frontend/compiler_messages/compiler_errors.rs
[S7]: https://github.com/nyejames/moth/blob/0096b781b45e347dfd545ccf504a654a8e3767bf/docs/src/developer-docs/style-guide/testing.mtf
[S8]: https://github.com/nyejames/moth/blob/0096b781b45e347dfd545ccf504a654a8e3767bf/src/build_system/tests/build_dependency_tests.rs
[S9]: https://github.com/nyejames/moth/blob/0096b781b45e347dfd545ccf504a654a8e3767bf/src/projects/html_project/external_js/runtime_glue/source.rs
[S10]: https://github.com/nyejames/moth/blob/0096b781b45e347dfd545ccf504a654a8e3767bf/src/compiler_frontend/hir/reachability.rs
[S11]: https://github.com/nyejames/moth/blob/0096b781b45e347dfd545ccf504a654a8e3767bf/src/compiler_frontend/module_compilation/external_imports.rs
[S12]: https://github.com/nyejames/moth/blob/0096b781b45e347dfd545ccf504a654a8e3767bf/src/compiler_frontend/module_compilation/generated/materialisation.rs
[S13]: https://github.com/nyejames/moth/blob/0096b781b45e347dfd545ccf504a654a8e3767bf/src/projects/html_project/external_js/runtime_glue/exports.rs
[S14]: https://github.com/nyejames/moth/blob/0096b781b45e347dfd545ccf504a654a8e3767bf/src/projects/html_project/external_js/runtime_assets.rs
[S15]: https://github.com/nyejames/moth/blob/0096b781b45e347dfd545ccf504a654a8e3767bf/src/compiler_tests/integration_test_runner/assertions/rendered_output.rs
