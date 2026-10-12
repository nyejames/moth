# Production borrow checker and Boracle development

This is the living implementation companion for Moth's production borrow checker and the operational follow-ups uncovered by Boracle. It owns current implementation status, accepted production implementation choices, observed defects, performance evidence and the ordered queue of next work. Keep it after individual fixes land. Update it whenever new Boracle evidence changes an accepted production decision or identifies a production issue.

The accepted source language and memory semantics remain authoritative in `docs/src/developer-docs/memory-management/`, particularly `borrow-validation/borrow-validation.mtf`. `docs/compiler-design-overview.md` owns compiler stages and the solver-independent Stage 6 handoff. The permanent Boracle reference and operational-oracle documents own their respective semantics and limitations. This plan owns implementation strategy and live status, not a competing source-language contract or a second reference solver.

## Current handoff - 2026-10-12

```text
STATUS: ongoing living record and new parallel development branch established
BRANCH: borrow-checker-and-boracle-development
AUDITED_MAIN: 36156b40849eecad12e69b8d4e62388672a1ecff
CURRENT_SLICE: baseline inventory and recorded performance investigations
BLOCKERS: new borrow precision and Boracle research activation await the
          merged typed-expression/native-result checkpoint and a fresh audit
NEXT_ACTION: retain reproducible borrow benchmarks, verify the post-cutover
             baseline, then scope the measured production performance fixes
```

The new branch starts at the audited `main` checkpoint. It is a parallel work area, not a claim that the typed-semantic-expressions programme has finished. Sync it with `main` after that programme's required capabilities have merged, rerun source/Boracle validation and refresh affected paths and measurements before activating migration or new research. The typed-expression programme currently reports an accepted Phase 3M closeout but paused Phase 4, with native result channels still pending. Dense HIR delivery alone is not the research activation gate.

Recording and investigating defects can continue while those gates are closed. A confirmed bug or isolated performance correction may receive an independently scoped patch if it preserves existing source semantics, the canonical module transaction, summaries and validated HIR. Research packages 3-6 are inactive proposals, not accepted production implementation orders. There is no general switch to Boracle in normal builds.

## Verified implementation inventory

This inventory comes from the code at `AUDITED_MAIN`, not just the progress matrix. It describes the current alpha implementation and does not claim that every required final Stage 6 or lifetime fact is already published.

| Area | Confirmed implementation | Important boundary or gap |
| --- | --- | --- |
| Production entry | `borrow_checker/mod.rs::check_borrows` creates `BorrowChecker`. Normal module compilation and generated sidecars use this alpha checker. | `problem` and `last_use` are compiled only for `boracle`/tests and `boracle` itself is behind the Cargo feature. There is no default reference-solver fallback. |
| Driver and convergence | `engine.rs::run` establishes local public call summaries, analyses every function, converges parameter mutation effects and publishes a `BorrowCheckReport`. Per-function reachable blocks use a forward worklist fixed point. | This still analyses complete module functions. It does not select only changed functions after private-failure rewrites. |
| State and overlap | `state.rs` tracks `UNINIT`, `SLOT` and `ALIAS` modes plus value-root and direct-alias sets per local. Each `RootSet` is a heap-backed `Vec<u64>` sized to the local count. | Production overlap is based on local storage roots, with conservative projections and joins. It does not consume Boracle's typed `OriginRelations`. |
| Local future use | `metadata.rs` builds visible-local masks, may/must future-use and future-assignment facts. Transfer checks ordered accesses and narrows branch-local visibility. | This is alpha local-use reasoning, not a complete origin/loan/event-sensitive production solver or loop-generation proof. |
| Mutations and calls | `transfer/access.rs` checks immutable/uninitialised updates, reads, mutable access, aliases, map and collection operations. `transfer/call_semantics.rs` resolves local, imported, generated, private and external call contracts. | Unknown provenance and dynamic storage overlap remain conservative. |
| Return summaries | `metadata.rs` stabilises local return-alias summaries, `return_alias.rs` classifies result provenance, and recursion cycles are classified `Unknown`. Public return alias is one coarse `Fresh`, `AliasParams` or `Unknown` result classification. | Multiple returned values can be unioned. Fine-grained per-result, projection, detached storage and complete retention/outlives effects are not yet delivered. |
| Borrow facts | `types.rs` contains per-statement, terminator and value facts, local function summaries, optional `Borrow`/`Transfer` decisions and advisory drop-site lists. Test/debug-only state snapshots are separately gated. | Advisory drop sites are not validated memory-plan cleanup or runtime ownership semantics. Missing optional transfer proof falls back to borrowing. |
| Generated convergence | `module_compilation/generated/convergence.rs` rechecks generated sidecars when their installed summaries change, and `refresh_private_failure_lanes` reruns `check_borrows` on the rewritten base `HirModule` when installation changes it. | The base refresh currently reruns the entire module. `Unchanged` already avoids the refresh. Preserve that fast path. |
| Boracle | Feature-gated source-to-`BorrowProblem` extraction, independent reference provenance/loans/last-use, named experiments, bounded operational oracle, differential comparisons, deterministic generator and reducer are present. | Reference research and oracle execution do not determine default acceptance, lifetime topology or physical memory strategies. |

The current worklist uses `join_in_place` for an existing successor state, reusing root-set allocations. This existing optimisation must not be described as missing. The remaining expensive copies are the block-input clone, the new successor-input clone and any repeated retained state costs. The implementation also performs a complete output-state comparison before propagating.

The alpha liveness decisions use point-specific CFG future-use facts and reachable write-through assignments, not an arbitrary cross-block traversal order. When every path redefines an alias before another read it can stop blocking mutation, including across a loop back-edge. Advisory exit candidates are recorded for reachable returns, breaks and region-exit edges. An infinite continuation does not invent a cleanup exit.

### Verified coverage and gaps

The production test owners include `borrow_checker_fact_tests.rs`, `borrow_checker_call_summary_tests.rs`, `borrow_checker_loop_tests.rs`, `borrow_checker_update_tests.rs`, `borrow_checker_scope_tests.rs`, `borrow_checker_jump_tests.rs`, `borrow_checker_drop_site_tests.rs` and `state_tests.rs`. Their checked cases cover same-statement access order, shared/mutable conflicts, rebindings and scope kills, loop back edges, path-dependent optional transfer, aggregate/map storage, fallible flow, fresh/aliased results and rejected uninitialised or immutable updates. The production `state_tests.rs` exercise in-place joins and visibility narrowing directly.

Boracle has its own tested normalized problem, reference solver, operational oracle, differential and reduction lanes. `boracle/tests/differential.rs` deliberately pins a known shared-alias disagreement instead of concealing it. These sources were inspected, but no fresh benchmark, complete test suite or post-cutover Boracle replay was executed for this documentation-only checkpoint. Historical validation below must not be presented as validation of later code.

Production borrow access checks are implemented and mandatory. The full accepted lifetime-region and escape solver, retained-edge/frontier proofs, REC and validated physical memory plans are separate undelivered analyses. Stage 6 must supply stronger solver-independent facts before those consumers can rely on them. Existing alpha drop-site emission is not evidence of a complete collector-free ownership implementation.

## Accepted production engineering decisions

These constraints are already established by compiler/memory authorities and existing code, not speculative Boracle promotions:

- Source access checking runs on all selected backends. A GC-backed output does not weaken source legality.
- Borrow validation consumes validated HIR and publishes immutable side facts. It does not rewrite HIR, repair malformed HIR, choose allocation owners or emit physical cleanup.
- Distinguish authored binding definitions from updates, preserve exact access/evaluation order and retain success/error edge availability. A static expression ID or separate result slot is not a fresh runtime generation proof.
- Distinguish slot storage, aliases and value origins. Preserve independent `copy` results and old-alias behaviour after a fresh rebind. Missing call or projection provenance stays unknown rather than becoming independent by default.
- Path-dependent or unproven **optional** transfer remains a borrow. It is not a source error. No optimisation is allowed to introduce mandatory move semantics implicitly.
- Exported summaries use stable semantic identities. Source/private/generated summaries converge inside the compiler-owned module transaction, independently of build scheduling. Diagnostics remain deterministic.
- Keep the reference solver inspectable and independent of the optimised production algorithm. The bounded oracle can demonstrate a concrete counterexample. Bounded safe executions alone cannot prove unbounded source acceptance.
- Production changes require a matching correctness/precision test, deterministic negative coverage and measurements when motivated by performance. Do not copy reference-solver maps/sets into production merely because they are convenient for Boracle.

**Implementation decisions still open:** compact root/state representation, clone elimination or ownership reuse, selective failure-lane invalidation, the exact iterative SCC implementation, production conflict-directed refinement and richer call/loop/aggregate proof domains. These are candidates or research directions until their evidence and invariants are reviewed. Existing code and accepted semantics are not approval of any one proposed data structure.

## Permanent performance evidence - 2026-10-12

The following is the retained content of the temporary `borrow-checker-followup-evidence.md`. It was recorded during the exact-numeric implementation review on `expression-refactor`, corrections at `2e000a073` before the squash onto `main`. Code was re-inspected on `AUDITED_MAIN`: the major mechanisms below remain present, while current line numbers may differ after future merges. Numeric/AST work is not blamed for these costs. Equivalent pre-numeric runs exhibit the same issue.

### E1. Dominant state size, copying and repeated checking

For `L` function locals, `BorrowState` contains `L` `LocalState` entries. Each has `value_roots` and `direct_alias_roots` as independent `RootSet { words: Vec<u64>, bit_len }` buffers of `ceil(L / 64)` words. A state therefore reserves roughly `2 * L * ceil(L / 64)` words, plus `2 * L` separate nonempty heap allocations. This is quadratic in local count, independent of the number of actual alias edges. The initial review described this as `O(L^2)` bits plus many allocations.

The worklist in `engine.rs` clones a complete input state to produce each block output. For each reachable successor it clones the block output again, applies jump-argument transfer, narrows visibility and joins or installs the successor input. Existing `join_in_place` avoids an additional whole-state allocation on a join, but does not eliminate these clones. Output-state equality still compares the complete state. `kill_invisible` scans local root sets at visibility-narrowing edges. With many basic blocks and locals this produces substantial copying, allocator traffic and state-comparison work.

`refresh_private_failure_lanes` invokes `check_borrows` for the whole base HIR when private failure-lane installation changed the HIR. Before `2e000a073` this refresh was not attributed to the borrow convergence metric. It now appears in `frontend.borrow.converge` for base work and `frontend.generated.borrow_recheck` for generated sidecars. These names measure distinct sources of work. Retain `command.check.total` when comparing the instrumentation boundary.

`metadata.rs::recursive_return_alias_functions` constructs local call edges, then invokes recursive `reaches_function` independently from each function with a fresh visited set. This repeated traversal takes up to `O(F * (F + E))` graph work and has call-depth proportional to a long dependency chain. It also conservatively marks recursive return provenance `Unknown`.

### E2. Profile evidence

The 1 ms-interval macOS `sample` trace on `43c72330f` recorded a generic-failure-chain `moth check` run with **1,349 main-thread samples**. Its exact workload size was not retained (the original note guessed larger than 512 from the 1.35 s sample, explicitly inference, not measurement). Of those samples, **1,338** fell beneath `run_generated_summary_convergence` and **1,295 (96% of all samples)** beneath `refresh_private_failure_lanes -> check_borrows -> BorrowChecker::run`. Within function analysis, **602** samples were at the per-block clone site, **299 + 263** at the successor path around its clone and **56** at function-layout building.

Self-time included **481** samples in `_xzm_xzone_malloc_freelist_outlined`, **432** in `_platform_memmove`, **82** in `kill_invisible`, **63** in `_xzm_xzone_malloc_tiny`, **43** in `build_function_layout` and **34** each in `check_shared_access` and two malloc routes. The review attributed approximately **1,039 / 1,349 (77%)** samples to allocation and copying. `reaches_function` reached **947 nested frames**. Recorded physical footprint and peak were both **7.4 GB**. These are one sampled run's attribution, not a guaranteed distribution across projects.

### E3. Matched checks

Medians of five interleaved runs on one Apple Silicon macOS host. Release builds with `--features detailed_timers` compared `43c72330f` (before timing correction) with `2e000a073` (after). Units are **milliseconds**. The `frontend.borrow.converge` jump is predominantly the newly visible private-failure refresh, **not** evidence of a performance regression. Total check time is the correct before/after comparison.

| Workload | Size | Borrow converge before | Borrow converge after | Check total before | Check total after | AST total after |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Sibling loops, one collection (`r4-512.moth`) | 512 | 0 | 2268.71 | 4912.23 | 4380.14 | 3.22 |
| `narrow-chains` | 512 | 0 | 1064.07 | 1106.93 | 1086.95 | 4.39 |
| `generic-failure-chain` | 32 | 0.22 | 1.56 | 5.67 | 5.26 | 0.61 |
| `generic-failure-chain` | 128 | 1.07 | 19.39 | 32.07 | 26.51 | 1.48 |
| `generic-failure-chain` | 512 | 8.34 | 446.48 | 569.65 | 475.25 | 4.79 |
| `alias-receiver-chain` | 256 | 0 | 0.58 | 5.96 | 5.47 | 1.48 |
| `alias-receiver-chain` | 1024 | 0 | 3.91 | 29.97 | 17.29 | 4.64 |
| `alias-receiver-chain` | 4096 | 0 | 46.36 | 324.21 | 126.61 | 20.82 |
| `alias-receiver-chain` | 8192 | 0 | 180.73 | 1218.14 | 432.43 | 42.17 |
| `aggregate-forwarding` | 512 | 26.18 | 53.44 | 520.43 | 522.88 | 16.10 |
| `origin-fanout` | 512 | 0 | 11.74 | 29.18 | 29.41 | 2.39 |
| `copy-chain` | 512 | 0 | 0.74 | 5.72 | 5.15 | 1.02 |
| `speed-test.moth` | - | 0 | 7.61 | 22.15 | 22.59 | 6.74 |
| `collection-stress.moth` | - | 0 | 33.58 | 51.61 | 51.45 | 1.68 |

The visible converge metric grows **1.56 -> 19.39 -> 446.48 ms** for successive 4x generic-failure-chain sizes (approximately 12x then 23x), while the AST metric grows much more slowly. `narrow-chains-512` spends about **1064 of 1087 ms** in borrow convergence and only **4.39 ms** in AST. The 512 sibling-loop check spends **2269 ms** in the newly attributed refresh out of **4380 ms** total, with additional first-pass borrow cost not extracted into the temporary timing JSON.

The pre-numeric checkpoint `9a416ab67` took **5213 ms** on equivalent `Int` source with 512 sibling loops, recorded in `benchmarks/frontend-optimization-results.md` under *Exact numeric types: review follow-up (2026-10-12)*. A 1024-sibling-loop check was observed around 52 seconds but its raw log was not retained. **Do not cite that observation as a verified benchmark without a rerun.**

### E4. Reproduction and evidence preservation

`just validate-full` passed all 13 recorded scaling series on the corrected review tree, but those series measured `frontend.ast.total`. They provide **no borrow-cost regression gate**. The review recorded fitted AST exponents `n^0.98` for numeric alias-receiver-chain and `n^0.87` for numeric generic-failure-chain against an AST budget of `n^1.25`. These numbers do not constrain borrowing.

Reproducible checked-in sources include `benchmarks/numeric-scaling/generate.py` and its `*-{32,128,512}.moth` cases, with larger alias-receiver-chain sizes. Regenerate with:

```sh
python3 -B benchmarks/numeric-scaling/generate.py
cargo build --release --features detailed_timers
# Run moth check on each workload five times and compare medians.
# Capture frontend.borrow.initial, frontend.borrow.converge,
# frontend.generated.borrow_recheck, frontend.ast.total and command.check.total.
```

For a macOS sampling run, build with `CARGO_PROFILE_RELEASE_DEBUG=true` and use `sample <pid> 5 -file out.txt` on the active `moth check` process.

Historical raw artefacts were **local-only**, not committed and may disappear. Their identifying paths are retained so later work knows what must be reconstructed:

| Original path | Recorded purpose |
| --- | --- |
| `tmp/exact-numerics/review-followup/measure.json` | Before/after five-run medians |
| `tmp/exact-numerics/review-followup/gfc.sample.txt` | Symbolised generic-failure-chain sample |
| `tmp/exact-numerics/review-followup/validate-full.txt` | Passing full validation and AST scaling fits |
| `tmp/exact-numerics/review-followup/r4-512.moth` | 512 sibling collection loops in one function with an I32 helper |
| `tmp/exact-numerics/review-followup/alias-8192.moth` | Large alias receiver chain |
| `tmp/exact-numerics/review-followup/r5-*.moth` | Nested Pair chains, borrow-neutral AST control |
| `benchmarks/frontend-optimization-results.md` | Committed performance write-up and pre-numeric comparison |

Preserve or regenerate the sibling-loop and large-chain reproducers before changing state representation. Record exact compiler/checkpoint, hardware, build flags, input fingerprints, peak memory and sample counts. Compare both borrow passes and whole-command time, and use a no-borrow control. A post-typed-expression baseline is required before accepting claimed performance wins.

## Open work - production

A confirmed problem accepts the **need to investigate and fix** it, not the first proposed algorithm. A candidate becomes an accepted implementation only after scoped tests, semantic review and representative measurements. The ordering here is local to this companion and does not override the main roadmap's unrelated sequencing.

| ID | Status | Priority | Next production work and acceptance gate |
| --- | --- | --- | --- |
| `BC-01` | Confirmed, queued | High | Add a borrow-specific scaling and memory baseline with dedicated initial/convergence measurements and an end-to-end check guard. Reproduce E2/E3 on the branch after the typed-expression merge. Commit durable workload generation, not multi-GB raw traces. |
| `BC-02` | Confirmed, implementation open | High | Eliminate dominant per-block/per-edge dense `BorrowState` copying. Compare contiguous buffers, sparse roots, interned/shared representations and carefully proven in-place transfers. Measure allocations, bytes, memory and wall time. Preserve same-place, mixed-slot/alias and visibility-kill correctness. Existing `join_in_place` is already delivered. |
| `BC-03` | Confirmed, implementation open | High | Avoid unnecessary whole-module `check_borrows` in changed private-failure-lane refreshes. First prove affected-function and caller-summary invalidation, side-table replacement and deterministic report publication. Preserve the existing `Unchanged` fast path and ensure generated sidecar convergence is not bypassed. |
| `BC-04` | Confirmed, implementation open | High | Replace repeated recursive `reaches_function` searches with a single deterministic iterative SCC/recursion classification. Include long acyclic chains, self recursion and mutually recursive SCCs. Do not independently promote recursive result precision. |
| `BC-05` | Investigate, unmeasured | Medium | Measure repeated function cloning in `finalize_public_call_summary_effects_to_fixed_point` (`self.module.functions[function_index].clone()` each iteration) and full-state equality cost. Prefer borrowing/reuse before extra caches. Avoid adding a parallel call-graph owner without evidence. |
| `BC-06` | Known capability gap, gated | Later | Design the fast production checker against Boracle's validated provenance/loan/event model, including precise conflicts, loops, per-result summaries and aggregate storage only as those research rules are accepted. First common path should be cheap. Deeper conflict refinement should occur only on candidate conflicts. Do not port Boracle's slow storage. |
| `BC-07` | Known capability gap, gated | Later | Complete solver-independent future/last-use and affine-transfer facts required by lifetime analysis, including loop final-iteration and detached stored-result cases. Missing optional transfer never becomes a rejection. Lifetime topology, retained-edge proofs and REC retain their own downstream ownership. |
| `BC-08` | Evidence needed | Later | Investigate broad borrow-fact compaction only after measuring fact-table cost independently of lattice cloning. Keep immutable facts outside HIR and do not merge unrelated parser/statement layouts into this effort. |
| `BC-09` | Untriaged | Later | Reassess use of folded private-constant HIR metadata in borrow analysis and temporary-local reduction after the typed-expression cutover. It was a deferred compiler follow-up, not evidence of a current bug or an accepted optimiser. |
| `BC-10` | Candidate only | Later | Investigate cheap production fast paths when no aliases, live loans, exclusive access, optional-transfer candidates, recursive call-summary SCCs or structural projections exist, or when a function is scalar-only or has an acyclic CFG. These are previously recorded Boracle observations, not implemented production skips. Require sound guards and evidence before promoting any. |

### Candidate approaches for BC-02 and BC-03

The historical review proposed: (1) put all root words in one contiguous state buffer, (2) reuse a block or single-successor state in place when its lifetime allows, (3) use sparse/interned roots when sets are mostly empty, (4) recheck only functions affected by private-failure rewrite and (5) classify recursion through one iterative Tarjan/Kosaraju-style SCC traversal.

None of these five approaches was prototyped or measured in the recorded investigation. Contiguity reduces allocator churn but may leave quadratic bytes and copying. Sparse storage can trade allocation savings for worse unions. A changed function's callers may need new summaries. No implementation approach should be locked before measuring these trade-offs and proving the cross-stage invariants.

## Open work - Boracle and the production handoff

Keep reference-solver findings and downstream production relevance separate. Boracle's accepted research results belong in its permanent reference/oracle authorities. This tracker owns the decision to investigate, promote or reject their **production** implications and the practical tooling/issues that must be resolved.

| ID | Status | Issue and next evidence |
| --- | --- | --- |
| `BR-01` | Confirmed Boracle reference gap. Production impact unverified | The bounded oracle reports `StaticAcceptedRuntimeConflict` with `SoundnessFailure` when a shared-alias holder performs an exclusive write. The reference solver unconditionally excludes a holder's own loan use and misses a terminal write-through definition without a following provenance writer. This is pinned in `boracle/tests/differential.rs`. Give reference correction its own semantics/test audit. Check whether production has any analogous acceptance defect before describing it as a production bug. Canonical Boracle reference docs retain the exact rule locations. |
| `BR-02` | Historical replay failures. Revalidate after cutover | A historical 1068-source real replay at `99ab43de2` recorded two `BorrowProblem` extraction failures (before oracle execution). `tests/cases/result_match_guard_propagation_order/input/@page.moth` produced unsorted branch targets (`BlockId(12)` then `BlockId(7)`), violating strictly increasing unique terminator targets. `tests/cases/runtime_if_reactive_metadata_preserved/input/@page.moth` produced unknown `LocalId(1)` in retired reactive-if metadata. Re-run the current corpus after the typed-expression/Reactivity V1 retirement. Fix only surviving producer bugs, not obsolete V1 semantics. |
| `BR-03` | Confirmed developer-tooling gap | `boracle/reducer.rs::reduce_problem` and `render_fixture_skeleton` exist and have tests, but `src/projects/cli.rs` has no reduction command and `boracle/service.rs` uses `OracleBounds::default()` for differential dumps. Expose reproducible bounds and class-preserving reduction through a bounded developer CLI change if still useful. No production checker or public solver switch. |
| `BR-04` | Research inactive | Typed provenance relations and the bounded oracle are completed reference capabilities. Conflict-directed refinement, loop epochs/edge last-use, call summaries/deferred exclusive access and aggregate-copy/builtin-storage provenance remain proposed packages 3-6, requiring refreshed evidence and explicit activation after merged native-result capabilities. Research acceptance alone does not promote a production solver rule. |
| `BR-05` | Deliberately deferred | The reference's `dead-exclusive-loan` experiment remains opt-in and not canonical acceptance. `MultiHolderLoan`, `RepeatedProjectionChild`, dynamic-index sibling precision and empty-union CFG joins have bounded or deferred treatment in the reference research handoff. Reopen only with an actual reduced case and named research owner. |
| `BR-06` | Historical oracle inconclusive. Revalidate | The old progress matrix recorded checked numeric reassignment with `OracleInconclusive` / `UndecidableOverlap` on the existing result-carrier projection and no completed executions. It supplied no bounded safety evidence. Infallible Boolean reassignment had recorded real-source mutation replay coverage, which should be retained as a control. Revisit after native result-channel migration, with a reduced source replay and separate proof of any improvement. |

For the older replay failures, the historical reproducer was `cargo run --features boracle -- boracle <source> --dump problem`. Its result was tied to an earlier source corpus. Do not claim those exact failures persist on a later branch without rerunning it. Resolve the reference acceptance gap before using that case as supporting evidence for a production precision change. The `BR-01` counterexample is a **known reference-model issue** and must remain visible to the differential campaign until resolved. A bounded oracle is not a replacement for a proof of universal correctness.

The proposed `$checked:` deeper proof-budget surface remains a separate design/research question. It is not implementation-ready and must not alter the ordinary source safety rules. Production compilation retains no developer-visible reference/experimental mode switch.

## Working and closeout rules

1. **Record evidence first.** Give each new issue a stable `BC-` or `BR-` ID, affected owner, checkpoint, reduced reproducer and observed behaviour. Distinguish confirmed bugs, implementation gaps, design proposals and performance hypotheses.
2. **Accept a specific fix separately.** Before changing the algorithm, state preserved semantics, new invariants, correctness cases and the cheapest measured representation that solves the problem. An unchecked speed claim is not an accepted decision.
3. **Validate differences.** Run focused production tests, source fixtures, existing module/generated summary gates and applicable `just boracle` / `just boracle-campaign` gates. Preserve independent oracle comparisons, invariant failures and deterministic witnesses. Record the exact branch revision and commands actually run. Do not inherit a historical pass.
4. **Measure performance on representative shapes.** Include normal source, worst-case chain/loops, large locals, generated/private failure convergence and a borrow-neutral control. Capture both borrow stages and end-to-end cost. Avoid timing comparisons across different metric definitions.
5. **Record the outcome and trim.** On landing, replace an open item with a short accepted decision, actual validation/measurement outcome and commit/PR link. Remove superseded alternatives, copied plans and stale duplicate status. Preserve durable regression inputs and meaningful negative tests, not running logs.
6. **Respect owners.** Canonical memory/compiler docs define semantics and stage boundaries. Boracle docs define reference and oracle behaviour. This companion owns live production progress, implementation choices, follow-ups and the handoff from research. The main roadmap owns only the combined parallel entry and prerequisites. The progress matrix links here instead of repeating this inventory.

The document is intentionally permanent. Finishing an individual patch does not complete or delete the tracker. Keep only the current small queue and accepted decisions after closeout. Git records detailed historical implementation steps.