# Compiler data-layout regression investigation and Phase 1 hardening

> Repository path: `docs/roadmap/plans/compiler-data-layout-regression-investigation-plan.md`
>
> Branch: `diagnostic-data-layout-changes`
>
> Status: Phase 0 complete. Exact reconstruction of the 19:18 `runs.jsonl` record is blocked because `bench-check` is read-only. Fresh matched predecessor/branch measurements are next. Runtime attribution, patches and validation have not started.
>
> Review snapshot and implementation baseline: `6919619a32a0939510ffcaac23997e71c716fbe9`. Compiler at HEAD equals audit-cleanup `59f95fed`. `main` and this branch shared that commit at activation; this investigation stays branch-local.
>
> Sequencing: Complete this investigation before resuming Phase 2 of the compiler source, token and diagnostic data-layout migration on this branch.

## Purpose

Explain the reported slowdown, repair demonstrated performance regressions and remove bounded Phase 1 cleanup debt without starting a competing representation redesign.

The investigation should establish which revision introduced each material regression, which work accounts for it and whether the final patch recovers performance against the pre-regression compiler. A faster result caused by lost warnings, skipped validation or incomplete compilation is a failure.

The original [source, token and diagnostic data-layout plan](./compiler-source-token-and-diagnostic-data-layout-plan.md) continues to own the migration sequence. The [data-layout architecture](../../compiler-data-layout-design.md) owns representation and lifetime contracts. This plan owns the intervening investigation and fixes, not a second implementation of either document.

The user requested this plan exclusively for `diagnostic-data-layout-changes`. Keep the investigation plan and any temporary roadmap scheduling note branch-local. Do not add it to `main`, create a PR or publish benchmark history as part of activation. Implementation commits and durable evidence can be integrated later through the user's normal workflow.

## Active context

Refresh this block after each accepted phase and before context compaction.

```text
ACTIVE_WORK: data-layout regression investigation and Phase 1 hardening
BRANCH: diagnostic-data-layout-changes
CURRENT_PHASE: 1 - Reproduce and isolate the regression interval
BASE_REVISION: 6919619a32a0939510ffcaac23997e71c716fbe9
REVIEW_REVISION: 6919619a32a0939510ffcaac23997e71c716fbe9
REPORTED_RUN_REVISION: not persisted; inferred HEAD/59f95fed from presentation-time UTC only; dirty tree possible
KNOWN_GOOD_COMPARISON: 232f207f last recorded CLI (harness-equipped Phase 0 baseline); also b6f81fe pre-layout main
PROVEN_REGRESSION_CAUSES: none yet
PATCHES_ACCEPTED: none
VALIDATION_RUNS: none for this investigation (Phase 0 is provenance only)
NEXT_ACTION: build matched historical and current candidates in separate target dirs; screen the checkpoint ladder
RESUME_GATE: final acceptance below, then the original plan's Phase 2
```

Phase 0 recorded the complete first-parent inventory, build-lane recipes, and DLR-01 through DLR-08 dispositions. Untracked notebook: `tmp/dlr-phase0/`. Exact 19:18 per-case samples were never written. Do not treat later controlled measurements as a reproduction of that missing record.

## Authority and scope

Read `AGENTS.md`, the current style guide and the language cheatsheet first. Read the following authorities in full because this work crosses representation, compiler and build boundaries:

- `docs/compiler-data-layout-design.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/roadmap/plans/compiler-source-token-and-diagnostic-data-layout-plan.md`

Read the testing and validation guides before selecting coverage and gates. Use the canonical unsuffixed language references for any affected lexical or diagnostic behaviour. Read the memory overview and its routed leaves when changing borrow-diagnostic provenance or analysis handoffs. The supplied `moth-code-review-guide.md` supplies the redundancy review checklist. Its old navigation paths do not override current `AGENTS.md` routing.

This plan authorises investigation, regression fixes, focused tests and the documentation corrections listed below. It also authorises separate detached historical worktrees for controlled comparisons. Historical worktrees supply comparison code and workloads, not the active branch's design authority.

### Preserve these contracts

1. Source snapshots have one owner. Source excerpts come from the compiled snapshot, not a later filesystem read. Registration, loading, preparation and freezing remain distinct lifecycle steps.
2. `SourceId` is context-local. A matching number in another package is not the same source. Inventory-backed IDs are final before tokenization. Traversal-only discovery normalises provisional identities at its existing finalisation boundary.
3. `LocalSpan` remains an exact four-byte range encoding. `SourceSpan` remains eight bytes. Preserve their `Option` niches, checked capacity failures and the compilation root's exact empty range. Keep source-owned overflow tables paired with every retained span.
4. Compiler stages own semantic work. Stage 0 schedules the canonical module service and publishes its outcome. Fixes must not move parsing, binding, generated convergence or borrow analysis into build orchestration.
5. Remap mutable facts before publication. Already-frozen diagnostics retain their original string, source and type owners. Foreign primary and secondary sites retain explicit ownership independently of a diagnostic's default context.
6. Preserve plain user diagnostics and the current typed outer `CompilerError` lane. The final `DiagnosticDraft`, `DiagnosticRecord`, type-display stores and infrastructure/bug split remain later-phase work.
7. Clean terminal results may skip diagnostic freezing only after the last diagnostic-capable consumer has finished. Successful warnings, generated warnings and late backend/output failures remain observable.
8. Parallel execution preserves source identities, diagnostic order and semantic output. Keep local mutable builders and canonical merges rather than introducing a globally locked interner.

### Work that stays with the original migration

| Original owner | Work that remains there |
| --- | --- |
| Phase 2 | Complete semantic/path migration to `PathId`, worker path deltas and removal of `InternedPath` bridges |
| Phase 3 | Fixed-width token shapes, source-owned token stores, cursors, `TokenRange`, segmented start bodies and removal of retained token-vector copies |
| Phase 4 | Diagnostic schema, draft/durable split, cold mixed-domain ownership, compact type-display snapshots and final report conversion |
| Phase 5 and later owners | Final infrastructure/compiler-bug separation and tooling recovery, according to the original sequence |

Pull forward a later-phase item only when a demonstrated cause requires it and the smallest coherent change removes an existing path. Record the exact original item, prerequisites, acceptance evidence and remaining work in the original plan. A speculative token-store rewrite, new diagnostic framework or broad scope-arena redesign is outside this investigation.

## Starting evidence and corrections

### Reported observation

The user supplied this result from benchmark host `6D851D`:

```text
macOS Apple Silicon, September 11 - 19:18
+16 ms average
0 faster, 16 slower
39 of 40 cases comparable
docs_check excluded because its workload changed
Stage movement:
  check total       +615 ms
  module semantics  +418 ms
  frontend          +416 ms
```

This is an actionable regression signal. Its exact Git revision, comparison run, build configuration and per-case distribution still need recovery from local history. The remaining comparable cases were not all classified slower. Cases below the benchmark's rough threshold may still have moved.

### Correct the earlier attribution before investigating

**The external-import path round trip predates Phase 1.** The predecessor service already called `collect_source_logical_paths_from_table`, rendered its per-module source paths to `Vec<String>` and passed them to the same external-import collector. Phase 1 changed the representation and moved that collection earlier in the service. Investigate the differential cost, cardinality and lifetime. Do not attribute the whole operation to newly introduced work. [E1]

**The three stage deltas are not an accounting equation.** `calculate_stage_movement` sums comparable per-case metric deltas. Module semantics is accumulated module evidence, while frontend and command totals are wall spans. Metrics can be nested and their contributing case sets can differ, including check-only totals versus check/build frontend totals. The near equality of 418 and 416 does not prove that all extra wall time is semantic work. Subtracting 416 from 615 does not identify a 199 ms package or finalisation cost. [E2]

**The public comparison uses means, not the migration's acceptance medians.** The executor calculates both, but `match_cases` compares `mean_ms` and stage observations are averaged. Recover the per-case median data and measure repeated runs for the explicit 5% gate. A rounded suite average cannot establish that gate. [E2]

**A green standard validation run is not timing acceptance.** The benchmark README explicitly describes ordinary timing comparisons as advisory. `bench-ci` preflights the standard inventory, then times only a quick subset with three iterations. The original migration independently requires five invocations and blocks median regressions above 5% unless the user accepts a documented tradeoff. [E3]

**Historical closeout evidence is bounded.** The September 10 record reported -1 ms for quick CLI cases, -4 ms for quick frontend cases and -5 ms for the dedicated data-layout pair. Later review corrections followed that initial closeout. The memory probe reported roughly 18% lower peak allocation proxies on its three historical workloads, but its instrumented timings are not comparable to normal CLI timings. Neither record proves that the final merged full suite was regression-free. [E4]

Future layout gains remain plausible. They are not evidence that this slowdown is an intended cost or will repair itself.

## Review findings and investigation targets

Each finding distinguishes an observed code shape from an unproven runtime cause. Record a disposition for every item: fixed, disproved, retained with a reason or assigned to a specific original-plan item. Recheck the activation tree before editing because the review snapshot may have advanced.

### DLR-01 - Establish measurement provenance and an explicit acceptance gate

**Status:** Phase 0: evidence gap confirmed. The 19:18 print was a read-only `bench-check` against `232f207f`. The measured revision is inferred from presentation-time UTC, not persisted. Compiler causation remains unproven.

**Where:** `xtask/src/benchmark_suite.rs`, `xtask/src/bench_types.rs`, `xtask/src/benchmark_execution.rs`, `src/timing/enabled/schema.rs`, `benchmarks/README.md` and local benchmark history. [E2, E3]

The normal history retains commit/dirty state, suite, machine identity and thread setting. The reviewed `BenchmarkRun` does not carry a complete rustc, Cargo, feature, `RUSTFLAGS` and binary-build recipe. Equal workload/measurement fingerprints therefore do not establish identical compiler construction. The public mean comparison and advisory exit policy are also different from the migration's median acceptance contract.

**Action:** Recover the reported and comparison records, preserve both and record all missing build/environment facts in the investigation evidence. Rebuild historical and current candidates with the same selected toolchain and matched recipes. Keep normal benchmark semantics unchanged unless a separately justified tooling fix is needed.

Use existing per-case median fields. When read-only output cannot expose enough data, add a narrow opt-in evidence export to the existing measurement owner. Export its already-collected samples and observations outside the measured operation into an untracked location. Preserve the default history, cleanup and summary behaviour. A second runner, comparison engine or permanent parallel timing system is not justified.

**Acceptance:** The reported movement is reproduced or its discrepancy is explained. Every performance verdict names exact revisions, workload identity, build lane and repeated measurements. `just validate` is never the sole timing evidence.

### DLR-02 - Remove throwaway frozen-identity handles from root-scope construction

**Status:** Confirmed new allocation path relative to the predecessor. Its contribution to the slowdown is unmeasured.

**Where:** `source/frozen_identity.rs::FrozenIdentityHandle::new`, `ast/module_ast/scope_context.rs::ScopeContext::new`, `scope_context/builders.rs::{with_file_value_resolution,with_frozen_identity_handle}` and root-scope callers in environment construction, constant resolution and emission. All paths are under `src/compiler_frontend/`. [E5]

`ScopeContext::new` allocates an `Arc<OnceLock<Arc<FrozenIdentityContext>>>` through a fresh handle. Service setup then replaces that handle with the real owner's handle on production paths. The pre-Phase-1 constructor had no such field. Ordinary child scopes share `ScopeShared`, so this is not one allocation per child scope.

The constructor also builds default style directives and empty shared services which later setters may replace. Those defaults already existed in the predecessor. Treat them as adjacent cleanup, not newly discovered Phase 1 regressions.

**Action:** Census root construction and handle replacement separately from the broad `ScopeContextsCreated` counter. Measure constructor work on the actual regressed workload. Pass the genuine identity owner into the existing root-construction boundary where callers already have it. Keep genuinely independent source/config/template boundaries responsible for creating their own handle.

Preserve the generated-body donor override. Avoid a global placeholder handle, an optional identity that silently falls back to the requester or a new service-locator framework. Investigate owned-registry copying through `Rc::make_mut` only at measured call sites. Sharing a large default package merely to avoid this one allocation can reintroduce older ownership problems.

**Acceptance:** The create-then-replace path is removed where confirmed, independent identity domains remain separate and ordinary/generated diagnostics render correctly. Report the allocation reduction and the measured timing result separately. A small neutral cleanup is acceptable without claiming it explains +16 ms.

### DLR-03 - Reduce external-import lookup churn without blaming pre-existing work

**Status:** Confirmed existing allocation-heavy lookup. Differential regression is a hypothesis.

**Where:** `module_compilation/service.rs::run_semantic_stages`, `module_compilation/external_imports.rs`, `source/database.rs::legacy_logical_path` and `src/builder_surface/external_import_providers/resolution_table.rs`. [E1, E6]

The current service renders candidate paths before semantic stages begin. The predecessor rendered the per-module table near the final external-import collection. The resolution table still constructs owned `(String, String)` lookup keys, including clones of source paths and prefixes during collection.

The current candidate IDs are module-owned candidates, not automatically the entire boundary database. Preserve that scope, including the distinction between owned candidates, semantic sources and check-only units.

**Action:** Measure path count, components/bytes rendered, empty-table calls, lookup-key allocations and their containing stage. Compare these against the predecessor on the same case. Check whether early construction now performs work for diagnosed or shortened service paths that previously exited before it.

Prefer avoiding unused collection and allocating keys, using borrowed lookup through the existing index or collecting at the actual consumer. Select the smallest measured improvement. Replacing every string key with a raw `SourceId` is unsafe across independent package databases. Any compact-ID conversion must use a proven common identity domain and follow Phase 2 ownership.

**Acceptance:** Provider scoping and deterministic import order are unchanged. Source paths are never rediscovered from output or filesystem scans. New lookup storage does not duplicate an index just to hide avoidable allocation. Retain or defer this target when measurement shows it is immaterial.

### DLR-04 - Preserve existing frozen rows when freezing a mixed message set

**Status:** Confirmed destructive assignment in a supported aggregation API shape. An executable regression and production reachability still need checking. This is an ownership/correctness target, not an established timing cause.

**Where:** `src/compiler_frontend/compiler_messages/compiler_errors.rs::{append_messages_preserving_context,freeze_source_contexts,diagnostic_render_context}`. [E7]

Appending messages explicitly preserves already-frozen rows and remaps only mutable facts. Later, `freeze_source_contexts` builds rows from transitional source contexts and assigns them to `self.render_frozen_contexts`. That assignment replaces any rows already present when transitional contexts are nonempty. It risks dropping the context which still owns an earlier diagnostic's string IDs and type names.

**Action:** Add a focused test which first creates a frozen message set, appends a mutable message set and then freezes the aggregate through the real APIs. Use colliding numeric IDs with different source text and string spellings so a wrong owner cannot pass accidentally. Exercise both append orders, a secondary foreign site and a type-bearing diagnostic. Confirm the earlier frozen row remains authoritative and the mutable row uses its newly frozen owner.

Trace production callers before describing user impact. Preserve existing rows and their first-match precedence when repairing conversion. Do not remap frozen payloads through the aggregate or flatten their contexts. If this composition is genuinely forbidden, enforce that contract at its owner and reconcile the aggregation API and tests rather than silently dropping rows.

**Acceptance:** A failing-before/passing-after ownership regression, or a demonstrated enforced exclusion with no silent loss. Independent review checks range offsets, frozen type ownership, overlapping fallback precedence and repeated freezing.

### DLR-05 - Move warnings at consuming boundaries and measure freeze work separately

**Status:** Confirmed avoidable warning copies in a consuming path. Additional freeze work is a measurement target.

**Where:** `src/build_system/create_project_modules/compiled_boundary.rs::ProjectFrontendCompilation::into_render_messages_with_optional_retention`, `CompiledGraphBoundary::successful_module_views` and the frozen-context installation helpers. [E8]

The final render conversion consumes `ProjectFrontendCompilation` but gathers base and generated warnings with `.iter().cloned()`. The old warning owners then die with the consumed graph. That copies diagnostic payloads and label storage that could move.

When any diagnostic remains, this path also constructs frozen contexts for available source-package databases before final retained-context accounting. Contexts created temporarily are not the same as contexts retained by the returned report. The existing clean-result branch already skips freezing, so failure-path freeze work must not be blamed for a clean benchmark without evidence.

**Action:** Move warnings from consumed owners in the existing deterministic order. Use a narrow consuming/draining operation at the artefact/store owner rather than exposing general mutable access to immutable published artefacts. Keep borrowed build paths separate when they still need their artefacts.

Measure diagnostic-heavy domain installation and source/package freezing. Optimise unused freezes or repeated diagnostic scans only after proving which contexts are needed by normal ranges, foreign primary sites, donor-only secondary handles and retained materialisation state. Keep one shared merged string allocation and exact package ownership.

**Acceptance:** No warning cloning in the confirmed consuming path, with unchanged order and counts across base modules, packages, generated sidecars and transient units. Clean-result retention remains empty where currently promised. Warning/diagnostic retention and elapsed time are measured separately from normal success throughput.

### DLR-06 - Review repeated whole-boundary validation before removing checks

**Status:** Confirmed repeated validation calls. Runtime significance and safe consolidation are unproven.

**Where:** `CompiledGraphBoundary::{finish,validate_invariants}`, `CompiledSourcePackage::validate`, `CompletedSourcePackageRegistry::preflight` and `ProjectFrontendCompilation::new_with_transient_messages` in `compiled_boundary.rs`. [E9]

`finish` validates the graph, package publication validates it again and final project construction revisits completed project/package boundaries. The full check allocates dense lane and artefact-owner tracking vectors. The repeated calls are real, but they occur at boundary handoffs rather than proving a semantic-stage hotspot.

**Action:** Count calls and visited rows on large module/package cases. Establish which callers can receive unfinished or mutable boundaries. Consolidate only when one completion proof remains enforced and later callers cannot invalidate it. Keep package-root, dependency-edge and cross-boundary identity checks that prove different facts.

Avoid a large typestate hierarchy or replacing real checks with `debug_assert!` just to improve a benchmark. Keep the current validation when the ownership change would cost more complexity than the measured benefit.

**Acceptance:** Each removed traversal has an explicit earlier proof and a protected handoff. Malformed internal graph tests still fail before publication. Record a reasoned leave-local decision when appropriate.

### DLR-07 - Prune small representation leftovers and test-only production APIs

**Status:** Confirmed cleanup candidates. Caller reachability must be refreshed before deletion.

**Where:** `src/compiler_frontend/tokenizer/tokens.rs` and `src/compiler_frontend/source/{database.rs,span.rs}`. [E10]

`Token::new` and `Token::with_span` construct the same two fields. Source code retains a suppressed `unique_record_for_logical_path` compatibility lookup and span resolver conveniences described as test-only. These deserve a caller audit instead of more permanent migration scaffolding.

**Action:** Keep one token constructor. Remove unused compatibility lookups when no active caller needs them. Put fixture-only construction and convenience queries under the existing test owner, or delete tests which only pin removed helpers. Preserve production span resolution and the tests of the actual codec, source lifecycle and hard layout contracts.

Audit raw `LocalSpan`/`SourceSpan` equality where a consumer requires byte-range equality. Append-only overflow encoding can represent identical ranges with distinct row indexes, so encoded-handle equality is not automatically a geometric comparison. This is a targeted usage check, not a request to add global span deduplication or change the encoding.

Keep mutable and frozen source owners distinct. Similar accessor bodies do not justify a broad abstraction which obscures their different lifetimes. Keep `FileTokens`, retained vectors and necessary `InternedPath` bridges with their recorded later-phase owner unless a measured cause requires a coherent pull-forward.

**Acceptance:** No orphaned callers, unjustified suppressions or replacement compatibility wrappers. Tests guard observable semantics or live invariants rather than the deleted API shape. Do not claim wall-time improvement from deletion alone.

### DLR-08 - Correct active documentation and prevent a misleading restart

**Status:** Confirmed stale current-state wording and evidence inconsistencies to reconcile against the activation tree.

**Where:** Original layout plan, `benchmarks/README.md`, affected source comments and `benchmarks/frontend-optimization-results.md`. [E3, E4, E11]

The original plan's standing Phase 1 text includes wording about returning the same source `Arc`, an authored line counter in `TokenStream::next` and representations described as deferred despite their Phase 1 implementation. The actual builder finishes to an owned database and the tokenizer now advances byte offsets. Distinguish intended final sketches from erroneous current-state descriptions before editing.

The benchmark README also carries fixed inventory counts that disagree with later recorded inventories, and some historical evidence describes median retention while the public timing comparison uses means.

**Action:** Correct active status and ownership descriptions. Recompute current counts from `benchmarks/manifest.toml`, or avoid duplicating volatile counts in prose. Add a dated evidence correction instead of rewriting historical measurements into results that were never obtained. Preserve the named Phase 4 R10 prerequisites and the original migration's later checkpoints.

**Acceptance:** An arriving agent can identify the real baseline, the confirmed causes, work still deferred and the exact Phase 2 resume point. No documentation claims a full-suite performance pass from quick preflight, memory proxies or unexecuted commands.

## Execution phases

### Phase 0 - Establish baseline, history and complete review coverage

**Goal:** Make the reported comparison reproducible before changing its subject.

- [x] Confirm the active branch and capture `git status`, HEAD, toolchain and benchmark configuration. Preserve user changes. Commit only authorised plan/setup changes before recording modes which require a clean tree.
- [x] Read the current authorities and actual benchmark build recipes. Identify the subprocess compiler, in-process runner and profiler build lanes separately.
- [x] Locate the user's 19:18 run and the exact previous record it compared against in `benchmarks/local-data/runs.jsonl`. Preserve both outside any path a new run overwrites. Resolve the timestamp's timezone from the record rather than inferring the measured revision from wall-clock ordering.
- [x] Extract per-case means, medians, spread, source/measurement fingerprints, timing schema, Git revision and thread setting. List the 16 classified slower cases and any other material median movements. Keep `docs_check` out of the original unchanged-workload calculation.
- [x] Review the complete local change inventory from the predecessor through the measured revision and branch baseline. Include code, tests, Cargo files, instrumentation and benchmark recipe changes. A truncated remote compare is not a complete inventory.
- [x] Record coverage and refreshed dispositions for DLR-01 through DLR-08. Review the recent cleanup implementation rather than repeating findings its patches already removed.

Suggested inspection commands, run from the active worktree:

```bash
git status --short
git branch --show-current
git rev-parse HEAD
git log --first-parent --format='%H %cI %s' -20
rustc -vV
cargo -V
just --show bench-check
just --show bench-frontend-check
just --show profile-build
```

Capture relevant Cargo configuration and environment values explicitly. Avoid dumping unrelated environment variables or credentials into evidence.

#### Historical comparison checkpoints

| Checkpoint | Revision | Purpose |
| --- | --- | --- |
| Pre-layout main | `b6f81fe581fa8c92b6a0e34fa099cff46b683572` | Normal CLI/frontend predecessor |
| Harness-equipped predecessor | `2843b9d6b55013b0cfbc45d45682de1a9aaf3efa` | Available on `token-and-diagnostic-data-layout-changes` (`88a60f3c`). Not an ancestor of HEAD. Harness-only vs `b6f81fe`. |
| Phase 1 squash | `674a3d6bbf55a240ccc9f6e8990f49cb249e07ba` | Isolate the representation landing |
| Packages squash | `82347e4226bd1d367382b2339eb075a07d679c6f` | Isolate package changes on top |
| Before cleanup squash | `0b1e811e43172ec0eba0d23f4cc3b5bf235fe70c` | Separate intervening main changes |
| Audit cleanup squash | `59f95fed520c46a08b299eba352fc133e07d74fb` | Isolate later consolidation. Current-tree compiler equals this commit. |
| Reviewed snapshot | `6919619a32a0939510ffcaac23997e71c716fbe9` | HEAD. Roadmap-only vs cleanup. Same compiler as `59f95fed`. |
| Reported run | Not in `runs.jsonl` | `bench-check` is read-only and does not enforce or persist cleanliness. Header `September 11th - 19:18` is UTC at presentation. Compared against `232f207f`. 16 slower case IDs unrecoverable. Measured revision is inferred, not established. |

Verify ancestry before choosing intervals. Squash messages contain historical subcommit hashes which are not necessarily ancestors in the current main history. Preserve available original refs for a finer bisect. If a historical object is unavailable, record the gap and narrow using the available commits rather than inventing a reconstruction.

**Gate:** Recorded. Exact 19:18 reconstruction is blocked. Continue with fresh controlled predecessor/branch measurements. Do not label those as a reproduction of the missing record.

#### Phase 0 results

- Toolchain: rustc/cargo 1.97.1, `aarch64-apple-darwin`, no `RUSTFLAGS`. Host `6D851D`.
- CLI lane: `cargo build --release --features timers` → subprocess `target/release/moth`. Frontend lane: `cargo run --package xtask --bin xtask -- bench-frontend-check` (default Cargo profile, `moth` with `features = ["timers"]` via `xtask/Cargo.toml`); manifest `profile = "dev"` is the Moth compilation profile, not rustc. Profiler: `RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile profiling --features detailed_timers --bin moth`. Profiling uses thin LTO; release uses fat LTO.
- `runs.jsonl` preserved at `tmp/dlr-phase0/runs.jsonl`. Last CLI record: `2026-09-04T12:59` UTC, commit `232f207f`, mean 32.14 ms, 40 cases, schema 2, 1 warmup / 10 iterations, default threads. Per-case extract: `tmp/dlr-phase0/last-cli-run-35-extract.json`.
- Timestamps are UTC (`BenchmarkTimestamp::now`). Public comparison uses means. Slower threshold: max(2 ms, 3% of previous mean, 2× combined stddev).
- Coverage ledger: `tmp/dlr-phase0/coverage.md`. DLR-02 through DLR-07 shapes survive cleanup. DLR-04 is a correctness target on `build.rs` / dev-server freeze, not the compiled-boundary render tail. DLR-07 and DLR-08 are cleanup/docs. No proven timing cause.
- Cargo.lock change across Phase 1 is `unicode-width` only. rust-toolchain unchanged. Squash-embedded historical hashes are not ancestors of HEAD.

### Phase 1 - Reproduce and isolate the regression interval

**Goal:** Distinguish changed code from changed measurement conditions and identify the earliest bad checkpoint.

- [ ] Build historical and current candidates in separate target directories using the same chosen toolchain and matched feature/profile settings. Record each lockfile and any dependency change. Build once before measuring, with no competing builds.
- [ ] Run the same binary against itself in interleaved batches to establish the current noise/control behaviour. Keep power mode, thermal conditions, competing processes and thread settings stable and record meaningful disturbances.
- [ ] Screen the checkpoint ladder on stable regressed cases. Confirm an apparent transition with balanced, interleaved A/B batches, alternating which revision runs first. Compare only identical source closures and runner contracts.
- [ ] Run full non-recording CLI and frontend checks at the important endpoints, not just quick cases. Run the dedicated warning/diagnostic suite where its harness exists. Save the evidence before any baseline is refreshed.
- [ ] Use five independent invocations for material before/after acceptance. Each full benchmark invocation uses the existing warmup/preflight and ten measured iterations. Compare the median of per-invocation case medians, preserving the public mean result separately.
- [ ] Establish a fixed identical docs workload for both compilers. Include required source packages and other compiler inputs. Keep an absolute current-docs measurement too, but never compare different docs source trees as an optimisation result.
- [ ] Repeat relevant cases at one worker and normal parallelism when scheduling or contention could account for the movement. Keep those as separate measurement identities.

Use the existing suite commands:

```bash
just bench-check
just bench-frontend-check
just bench-data-layout-check
just bench-scaling
```

A historical checkout lacking the diagnostic harness cannot run that suite. Use the verified harness-equipped predecessor for it. Avoid copying today's entire harness or compiler sources into old revisions and calling the result historical.

`MOTH_COUNTERS=off` is not proof that counter instrumentation was compiled out. Verify Cargo features. Keep allocation probes and detailed counters out of normal throughput comparisons. Workspace tests also do not prove the no-feature configuration because `xtask` enables `timers` through its dependency.

Record compiler command time separately from process elapsed time where both are available. Preserve each lane's actual meaning. The `profiling` profile uses different LTO settings from `release` in the reviewed tree, and the in-process runner is not automatically the same Rust build as the CLI binary.

**Gate:** Identify a reproducible regression interval and affected workloads, or explain the original discrepancy with controls and provenance. Keep ambiguous cases open. An unrelated current-tree speedup does not establish the cause of an old report.

### Phase 2 - Attribute costs and reproduce ownership findings

**Goal:** Replace hypotheses with measured mechanisms before proposing larger changes.

- [ ] Produce per-case stage tables with absolute before/after times and each metric's relation. Compare matching case subsets. Inspect semantic children, Stage 0, bootstrap, message construction and destruction rather than summing nested parents.
- [ ] Select a substantial regressed case, a different regressed workload family and an unchanged control. Use the existing `just profile-case <case-id>` path for supported cases. Confirm that profiles have samples and useful symbols before using them as evidence.
- [ ] Follow the actual hotspot. Evaluate DLR-02 and DLR-03, but do not privilege them over a stronger profile result. Inspect success-path error construction, larger result/stack copies, source/path remapping and generated materialisation only where the measured call tree points.
- [ ] Add bounded counters or coarse timers at existing owners when profiles cannot separate the cause. Record call count and work per call, such as entries copied, bytes rendered or rows visited. Call counts alone cannot detect an expensive inner traversal.
- [ ] Verify probe overhead against the same binary without that probe. Avoid per-token clocks, formatting or output. Remove temporary probes before final throughput validation.
- [ ] Reproduce DLR-04 through real aggregation APIs. Review the production paths that combine frozen and mutable messages. Fix a confirmed ownership regression independently of whether it is a performance cause.
- [ ] Review DLR-05 and DLR-06 on warned/diagnosed and module/package-heavy workloads. Separate work performed during conversion from storage retained after the report is returned.

For preparation, count physical reads, lexes and header preparation per source and role. The original plan records repeated canonical/check-only preparation as later Phase 3E work. A before/after increase is a regression candidate. An unchanged pre-existing repetition is recorded debt, not proof that Phase 1 introduced it.

For generic work, preserve existing prefix/delta sharing. Compare string entries scanned, type/declaration copies and generated requests on generic-heavy cases. Review the shared materialisation pipeline introduced by the cleanup squash rather than assuming the old duplicate pipelines still exist.

**Gate:** Every proposed performance patch has a causal hypothesis tied to measured work, an affected case and a control. Every proposed correctness patch has a reproducer or a precise enforced contract failure. Record disproved hypotheses instead of leaving them as queued optimisations.

### Phase 3 - Patch confirmed regressions in bounded slices

**Goal:** Recover throughput through the existing owners while preserving Phase 1 contracts.

Implement one causal mechanism per slice. A slice may span several files when they are one ownership change. Keep unrelated cleanup separate so the performance result remains attributable.

For each slice:

1. Record the cause, current owner, proposed removal or data-flow change and relationship to the original migration.
2. Add or strengthen the narrow correctness/invariant test that protects the behaviour. Use existing integration ownership for user-visible semantics.
3. Implement the smallest coherent patch and delete the superseded path. Preserve diagnostics, generated work and source ownership through every early exit.
4. Run focused tests and an interleaved comparison against the unpatched bad revision. Recheck against the established good revision so a partial recovery is visible.
5. Run formatting, the required benchmark check and `just validate`. Review the complete slice for ownership, duplication, dead paths, diagnostics and test quality before committing it.

Prefer passing an existing owner, moving an owned payload, borrowing an existing lookup or removing redundant work. Add an index only when repeated search is measured and one owner can maintain it without redundant storage. Preserve the current representation when a clever replacement adds complexity without a repeatable gain.

Review failures in the success path as well as success in the failure path. A smaller diagnostic struct does not itself prove improved calling conventions or lower stack traffic, and a shorter compile does not prove the compiler performed the same work.

**Gate:** Each accepted performance claim has repeatable evidence and semantic equivalence. A correctness repair may remain even when it has no speed benefit, but its timing is reported honestly. No unresolved greater-than-5% acceptance regression is silently classified as expected migration cost.

### Phase 4 - Complete bounded cleanup and documentation reconciliation

**Goal:** Leave a smaller, clearer starting point for Phase 2 without expanding this plan into Phase 2.

- [ ] Complete the verified create-then-replace and consuming-warning cleanup from DLR-02 and DLR-05 where it remains after performance work.
- [ ] Resolve the mixed-context ownership finding from DLR-04 before resuming representation changes.
- [ ] Consolidate the token constructor and remove genuinely unused compatibility/test-only APIs from DLR-07. Preserve needed tests under their actual subsystem owner.
- [ ] Give DLR-03 and DLR-06 measured fix/retain/defer decisions. Avoid forcing a path-index migration or validated-state framework when their costs are immaterial.
- [ ] Correct DLR-08's current-state wording and evidence caveats. Name future ownership without resurrecting deleted APIs to match stale comments.
- [ ] Review changed modules from their entry points. Split only when responsibilities are genuinely mixed, deepen the current subsystem rather than adding broad utility modules and retain concise comments which explain non-local ownership.
- [ ] Recheck the complete changed-file inventory and adjacent callers. Mark uninspected or intentionally deferred areas explicitly. Remove experimental code and refresh affected test ownership.

Use one primary regression test owner per behaviour. Remove redundant tests that only exercise deleted constructors or private storage arrangements. Keep hard layout assertions beside the types they constrain. Do not add tests requiring the future eight-byte token or 32-byte diagnostic types before their owning slices introduce them.

**Gate:** Every finding has a supported disposition. Cleanup commits preserve behaviour and have their own required validation. All original-plan pull-forwards, if any, are reconciled without duplicate implementations or double-counted completion.

### Phase 5 - Final acceptance, durable evidence and resume

**Goal:** Establish that the final branch is safe to continue, not merely faster than an intermediate bad patch.

- [ ] Run the final validation matrix below on an unchanged candidate. No tracked edits or competing builds during measurement or the benchmark stages of validation.
- [ ] Repeat full CLI, focused frontend, diagnostic-layout and scaling evidence against the controlled baseline. Use five independent invocations for the material comparisons and the original 5% median policy.
- [ ] Validate frozen ownership and measure clean/warned/diagnosed retention through the existing memory probe separately from uninstrumented throughput. Compare matched workloads and sampling boundaries.
- [ ] Obtain an independent review of attribution and measurement hygiene, then a separate review of ownership, diagnostics, tests and unnecessary complexity. Resolve findings and rerun affected gates.
- [ ] Publish one dated investigation section in `benchmarks/frontend-optimization-results.md`. Keep raw samples, profiles and allocator logs untracked. Correct earlier inference with a dated note rather than invented historical results.
- [ ] Update the original layout plan's context capsule with the accepted fix revision, exact evidence, deferred items and Phase 2 restart instructions. Preserve its later checkpoints and the separate diagnostics-improvement pause.
- [ ] Remove this temporary plan and its branch-only roadmap entry in the same commit that completes it, following repository plan lifecycle rules. Ensure the investigation plan is not introduced on `main` when fixes are later integrated.
- [ ] Finish with the repository Slice review. Record what ran, what did not run and any remaining blocker before authorising the Phase 2 continuation.

## Validation matrix

Run existing tests first and add coverage only for a missing contract or a reproduced defect. Exact filters are selected from the activation tree, not guessed from this document.

| Area | Required evidence |
| --- | --- |
| Source lifecycle | Canonical and discovery registration, selected loading, one retained snapshot, builder installation, clean completion and diagnosed/IO-failure exits |
| Span fidelity | Inline and extended exact ranges, UTF-8, CRLF and bare CR, zero-width EOF, root range, long spans and typed capacity failures |
| Identity ownership | Project/package numeric-ID collisions, donor primary and secondary sites, nonidentity string remaps, type-name ownership and mixed frozen/mutable aggregation |
| Warning transport | Config, AST, base module, package, generated sidecar and transient warnings survive once in deterministic order, including late failures |
| Semantic equivalence | Normal and generated functions, templates/content sources, external imports, public interfaces, HIR/borrow checks and emitted artefacts retain their existing contracts |
| Parallelism | Relevant serial/default/fixed-thread runs agree on semantic output and normalised diagnostics, with stable final source identity where the contract requires it |
| Benchmark correctness | Invalid outcomes and changed inputs remain non-comparable or failures, raw means/medians retain their meanings and optional evidence export leaves default persistence unchanged |
| Memory | Clean-result ownership is released when permitted, warnings/errors retain required contexts, temporary construction cost is separate from live-report and post-drop storage |

Use existing bounded capacity test facilities rather than allocating billions of IDs. Add a large-source or long-line performance fixture only when the current inventory cannot expose a demonstrated cost. Benchmark fixtures supply performance evidence, not correctness coverage.

The normal code gate is:

```bash
cargo fmt --all -- --check
git diff --check
just validate
```

At final closeout also run the curated feature matrix. `just feature-lane-check` checks the lane inventory, not execution of every lane. Run `just test-feature-matrix` and relevant Boracle coverage when shared semantic-service or scope changes affect its shorter compiler path. Follow current CI policy for supported host/target checks and report unavailable platform verification rather than treating cross-compilation as a native run.

Requested documentation changes use the documentation release-build gate. Rebuild generated documentation through the compiler, never edit `docs/release/**` manually. Keep documentation generation outside measurement windows. Update the progress matrix only when support, rejection behaviour or coverage changes under its own rules, not for a pure internal refactor.

## Performance acceptance and evidence format

A green command exit, a reduced struct size and a lower allocator peak answer different questions. Keep all three results separate.

For every important case, retain:

```text
case and complete workload identity
baseline revision and patch revision
compiler binary/build recipe and Rust/Cargo versions
suite, runner, profile, features, flags and thread setting
five before/after per-invocation medians and observed spread
median of those medians, absolute delta and percentage delta
public mean comparison, labelled separately
matching stage/operation measurements and relevant work counters
semantic outcome, diagnostic counts and applicable artefact checks
disposition and any remaining uncertainty
```

Compare the same case set on both sides. Require a supported decision for each material regressed case, not just a favourable suite average. Apply the architecture's greater-than-5% median blocker to docs, focused frontend and targeted workloads. Increase repetitions when the result straddles the threshold or the control is unstable. Do not turn measurement uncertainty into a pass.

A residual regression requires explicit user acceptance with its workload, magnitude and tradeoff. Recording it in the plan is not acceptance. Refreshing history, excluding an unchanged slow fixture or raising a complexity budget to make the result green is not a fix.

For memory, report exact owner counts separately from process-global allocation proxies. Keep snapshot bytes, overflow rows, diagnostic counts and context counts distinct from peak/live/post-drop byte measurements. Use the same probe configuration and lifecycle sampling points on both sides. Historical approximately 18% reductions are context, not a substitute for current matched evidence.

The final evidence section should state the proven causes, rejected hypotheses, patches, comparison table, cleanup dispositions, executed gates and remaining later-phase work. Keep the investigation notebook and raw data out of the tracked report.

## Review coverage and agent handoff

The remote review supplies concrete starting evidence in these areas. The implementing agent completes the local caller and diff audit before marking coverage closed.

| Area inspected | Starting result | Local follow-through |
| --- | --- | --- |
| Source database and compact spans | One move-owned source lifecycle, checked encoding and later-phase bridges | Audit all changed loading/finalisation exits and equality consumers |
| Token and retained syntax shape | Compact spans are present, wide token/vector ownership still exists | Verify constructor duplication and distinguish Phase 3 debt from new copies |
| Semantic-service entry | Candidate path reconstruction changed shape and position | Compare full before/after stage bodies and actual path cardinality |
| Root scope and service setup | New replaceable identity-handle allocation | Census every ordinary, generated, config and direct-template constructor |
| Diagnostic aggregation and freezing | Mixed-row replacement candidate and consuming warning copies | Reproduce composition, trace production callers and verify all owner lifetimes |
| Project/package publication | Full invariant proofs run at several handoffs | Establish which checks are distinct and which inputs are already immutable |
| Benchmark protocol and records | Means versus medians, advisory gate and incomplete build provenance | Recover original records and verify actual build/execution recipes |
| Recent cleanup and future plan | Some old findings have already been consolidated, current wording has drift | Review new shared paths and preserve the original phase boundaries |

Use bounded subagents where the harness supports them. Good read-only work items are benchmark provenance, source/diagnostic ownership and scope/materialisation caller review. Give each one exact revisions, paths, questions and required evidence. Keep one writer per worktree. Reviewers should not benchmark concurrently or mutate the worktree under measurement.

The task is complete only when measured regressions have been resolved or explicitly accepted, ownership defects have been addressed, cleanup has a documented disposition and the original migration has a truthful restart point.

## Source evidence index

All paths below are repository-root-relative. References describe the inspected source, not tests run during preparation of this plan. Unless a predecessor is named, the review revision is `6919619a32a0939510ffcaac23997e71c716fbe9`.

- **E1 - Path reconstruction before and after.** `src/compiler_frontend/module_compilation/service.rs`, current `run_semantic_stages` and predecessor `collect_source_logical_paths_from_table` at `b6f81fe581fa8c92b6a0e34fa099cff46b683572`, around lines 1019 onward. `src/compiler_frontend/module_compilation/external_imports.rs` supplies the shared consumer.
- **E2 - Timing and statistical meaning.** `xtask/src/benchmark_suite.rs::{measure_one_case,build_case_result}`, `xtask/src/bench_types.rs::{match_cases,calculate_median,calculate_stage_movement,BenchmarkRun}` and `src/timing/enabled/schema.rs`. The semantic timer's call boundary is in `src/build_system/create_project_modules/compilation/canonical.rs::compile_prepared`.
- **E3 - Benchmark and validation policy.** `benchmarks/README.md`, `justfile`, `Cargo.toml` and `docs/compiler-data-layout-design.md` under `Performance evidence and acceptance`. The reviewed profiles differ in optimisation/LTO and instrumentation choices.
- **E4 - Historical results and their limits.** `benchmarks/frontend-optimization-results.md` under `Data Layout Migration - Phase 1 Closeout (2026-09-10)`, `Phase 1 Review-Correction Retention Probe` and the recovered predecessor data-layout evidence. The tracked September summary and original local JSONL records are separate evidence sources.
- **E5 - Scope handle allocation.** `src/compiler_frontend/source/frozen_identity.rs::FrozenIdentityHandle`, `src/compiler_frontend/ast/module_ast/scope_context.rs::ScopeContext::new`, `scope_context/builders.rs`, `environment/builder.rs`, `environment/constant_resolution.rs` and `emission/emitter.rs`. Compare the root constructor with `b6f81fe581fa8c92b6a0e34fa099cff46b683572`.
- **E6 - Lookup allocations.** `src/builder_surface/external_import_providers/resolution_table.rs::{get,collect_unique_resolved_imports_for_source_files}`, together with the source path interner and E1's consumer. Source/prefix tuple keys are constructed as owned strings in the reviewed implementation.
- **E7 - Mixed context preservation.** `src/compiler_frontend/compiler_messages/compiler_errors.rs::freeze_source_contexts`, especially assignment to `render_frozen_contexts` around lines 672 onward, and `append_messages_preserving_context` around lines 755 onward. Read the neighbouring frozen-aware diagnostic/type remap functions and first-match render lookup.
- **E8 - Consuming warning transport and freeze.** `src/build_system/create_project_modules/compiled_boundary.rs::into_render_messages_with_optional_retention`, around lines 1103 onward, and `successful_module_views`. The existing clean-result early return and feature-gated retention accounting are load-bearing.
- **E9 - Repeated boundary proof.** `src/build_system/create_project_modules/compiled_boundary.rs::{CompiledGraphBoundary,CompiledSourcePackage,CompletedSourcePackageRegistry,ProjectFrontendCompilation}`. Read `finish`, `validate_invariants`, `validate`, `preflight` and `new_with_transient_messages` together.
- **E10 - Small leftovers.** `src/compiler_frontend/tokenizer/tokens.rs::Token`, `src/compiler_frontend/source/database.rs::unique_record_for_logical_path` and its helper, and `src/compiler_frontend/source/span.rs` test-only resolver conveniences. Refresh all caller searches locally before deletion.
- **E11 - Current versus planned representation.** `docs/roadmap/plans/compiler-source-token-and-diagnostic-data-layout-plan.md`, its standing Phase 1 contracts, deferred-representation wording, R10 prerequisites and Phase 2/3 work items. Check descriptions against `SourceDatabaseBuilder::finish` and `TokenStream::next` rather than against older milestone prose.
