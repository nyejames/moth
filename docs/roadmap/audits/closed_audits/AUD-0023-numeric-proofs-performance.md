# AUD-0023: Numeric proof side-table performance

- State: `complete`
- Kind: `Performance`
- Area: `analysis.numeric_proofs` - the bounded-integer proof side table, its production invocations and publication, and its backend proof queries
- Coverage: `partial`
- Reviewed: `2026-11`
- Baseline: Branch `audit-guide-improvements` at `e49a166e8`. Production code is identical to `main` at `49714f1b8`. `cargo test -q -p moth --lib numeric_proof` passed (60 tests). Measurements used the `profiling` build produced by `just profile-case` (`target/profiling/moth`, optimised with debug info), rustc 1.99.0, Apple M1 Pro with 10 cores, default Rayon threads and a warm filesystem cache. `just profile-case numeric_grouped_arithmetic_512_check` built the binary and sampled an approximately 8 ms run. Samply symbolication reported `failed_raw_addresses`, and the recipe then exited because this audit's own report edits changed tracked files mid-run. That profile was not used as evidence.

## Question

Does the proof analysis or its JS and Wasm consumption add measurable compiler time? Does any stage repeat proof work that could be removed while making the code simpler?

## What was inspected

Code paths: the same surfaces as [AUD-0022](./AUD-0022-numeric-proofs-redundancy.md#what-was-inspected). They are `analysis/numeric_proofs/mod.rs` in full; its invocations in `module_compilation/service.rs` and the `generated/convergence.rs` publication loop; the materialisation placeholder; and the proof queries in `backends/js/{reachability.rs, js_statement.rs, emitter.rs, js_function.rs}` and `backends/wasm/hir_to_lir/stmt.rs`. `JsNumericOperationLowering::classify` and `numeric_failure_codes` were read to establish per-query cost.

Workloads, generated into `./tmp/aud0023/` with the repository generator `benchmarks/numeric-scaling/generate.py` (`fixture("narrow-chains", n)`):

- `narrow-chains-{512,1024,2048,4096}`: three functions of `n` parameter-driven `I8`, `U8` and `F16` additions. The integer chains are checked statements that the analysis walks with its single-local cache. This is the existing `numeric_narrow_chains` benchmark shape, scaled past its committed 512 size to clear the sampling floor.

Measurements:

| Command | Runs | Total samples (1 ms interval) | Samples in `numeric_proofs` | Samples in `lower_hir_to_js` |
|---|---|---|---|---|
| `moth check narrow-chains-2048.moth` | 4 | 1594, 1909, 1621, 1855 | 1, 1, 1, 1 | not run |
| `moth build narrow-chains-2048.moth` (HTML-JS) | 4 | 2141, 2058, 1725, 1652 | 0, 1, 0, 1 | 9, 14, 7, 6 |
| `moth check narrow-chains-4096.moth` | 1 | 18045 | 0 | not run |

Samples came from macOS `sample <pid> 30 1` attached right after launch, covering the whole run. Nested frames of one sample were counted once. Wall time for `check` grew 0.18 s, 0.53 s and 1.98 s at sizes 512, 1024 and 2048. A 5 s sample at size 4096 attributed that superlinear growth to `BorrowChecker::analyze_function` and `build_function_layout`. The [production borrow checker plan](../../plans/production-borrow-checker.md) already records that cost for `narrow-chains`. It is outside this area and was not re-filed.

The proof analysis took at most one sample per run, which is at most about 0.06% of samples. Total HTML-JS lowering, including every proof query, took at most 0.7%. Both figures are within sampling noise.

## Authorities read

`AGENTS.md`; `docs/developer-docs/style-guide/style-guide.mtf`; `docs/roadmap/audit-guide.md`; `docs/roadmap/audit-kinds/README.md`; `docs/roadmap/audit-kinds/performance.md`; `docs/roadmap/audits/README.md`; `justfile` benchmark and profile recipes; `benchmarks/manifest.toml` numeric-scaling cases; `docs/compiler-design-overview.md` opening, `Architectural invariants` and `Stage 5 > Numeric ownership`; proof passages of `backend-lowering.mtf` and `numeric-backend-contract.mtf`.

## Existing findings and active plans checked

No audit-log row, open finding or bug entry covered this area. The production borrow checker plan owns the measured borrow-convergence scaling on `narrow-chains`. The numeric optimisation plan was retired in `49714f1b8`.

## Findings

None. No measured cost was attributable to the area. No candidate met the structural-efficiency bar of removing work while simplifying code.

## Checked and clean

- **Analysis complexity.** `analyse_numeric_proofs` makes one pass over blocks and statements, plus one terminator-target pass in `count_incoming_edges`. It does no fixpoint, no per-statement allocation and no string or path work. `expression_interval` recurses only through nested `Cast` expressions. Sampling confirms negligible cost at 6,144 checked operations.
- **Analysis in `check`.** `service.rs` computes the table unconditionally, including for `moth check`, where no backend consumes it. Measured cost is at most about 0.06%. Making it lazy or backend-requested would add state and conflict with the overview's rule that each executable computes its table before publication. Rejected.
- **Discarded generated sidecars.** The convergence publication loop computes tables for every generated node before `retain_reachable_sidecars` drops unreachable ones. Moving the call into the later reachable-only loop would save that work but would not simplify the code. No workload showed sidecar proof cost. Rejected as unmeasured and non-simplifying.
- **JS repeated liveness queries.** `invoke_failure_edge_is_live` runs on each visit from the live closure, cycle detection, branch inspection and helper demand. Each call is a match, a static `numeric_failure_codes` slice and at most one `FxHashSet` lookup, with no allocation. The live closure is computed once per function. Total JS lowering stays within 0.7% of samples. Caching would add state for no measured gain.
- **JS narrowing re-classification.** This is filed as a Redundancy finding in AUD-0022-F02. It has no measurable time cost here.
- **Retained memory.** `NumericProofs` keeps two sparse `FxHashSet<HirNodeId>` sets per executable, holding only proven node ids, with no strings or layouts. No probe was run. The size is bounded by the count of proven operations.
- **Wasm query.** There is one `FxHashSet` lookup per checked integer statement, only after the `Infallible` short-circuit.

## Limitations

- Only the `narrow-chains` shape was measured. No workload exercised many generated sidecars, `IntegerDivide` or `Remainder` proofs, or seeded `Invoke` chains under `Error!` lanes.
- HTML-Wasm lowering was not measured, and Wasm LIR emission below `hir_to_lir/stmt.rs` was not inspected.
- No allocation counters or memory probes were run. Retained-memory claims are structural only.
- `just bench-frontend-check`, `just bench-scaling` and `just bench-check` were not run. The committed `numeric_narrow_chains_512` cases sit below a useful sampling floor for this stage.
