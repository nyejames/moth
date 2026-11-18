# AUD-0022: Numeric proof side-table redundancy

- State: `complete`
- Kind: `Redundancy`
- Area: `analysis.numeric_proofs` - the bounded-integer proof side table, its production invocations and publication, and its backend proof queries
- Coverage: `partial`
- Reviewed: `2026-11`
- Baseline: Branch `audit-guide-improvements` at `e49a166e8`. Production code is identical to `main` at `49714f1b8` (numeric optimisation), since the branch changes only audit documentation. `cargo test -q -p moth --lib numeric_proof` passed (60 tests). No broader gate ran. The numeric optimisation plan was retired in `49714f1b8`, so no active plan owns this area.

## What was inspected

Read in full and audited:

- `src/compiler_frontend/analysis/numeric_proofs/mod.rs`: `NumericProofs`, `analyse_numeric_proofs`, `count_incoming_edges` and every `BlockAnalyser` method.
- Production invocations: `module_compilation/service.rs` (base executable, around line 1094), `module_compilation/generated/convergence.rs` (sidecar publication loop, lines 704-770) and the `NumericProofs::default()` placeholder in `module_compilation/generated/materialisation.rs` (around line 468).
- Publication: the `ModuleExecutable::numeric_proofs` field in `module_compilation/artefact.rs`.
- Threading into backends: `projects/html_project/compile_input.rs`, `html_project_builder.rs` (around line 706), `js_path.rs` (lines 150-205) and `wasm/artifacts.rs` (lines 110-155), plus `backends/wasm/backend.rs`, `hir_to_lir/module.rs` and `hir_to_lir/context.rs`.
- JS consumers: `backends/js/reachability.rs` in full; `classify_numeric_operation`, `lower_numeric_operation`, `proven_narrowing_expression`, `integer_narrowing_is_proven` and `prepare_invoke` in `js_statement.rs`; `functions_to_emit`, `emitted_functions_use_numeric_helpers`, `collect_numeric_operation_helpers` and `collect_used_cast_policies` in `emitter.rs`; `has_cfg_cycle`, the declaration-block use and Invoke branch inspection in `js_function.rs`.
- Wasm consumer: `lower_checked_integer_operation` in `backends/wasm/hir_to_lir/stmt.rs`.

Context read but not audited: `numeric_operation_cannot_fail` and `checked_result_range` in `datatypes/numeric_operators.rs`, `NumericScalar::{is_integer, integer_range}`, `FixedScalar::{signed_range, unsigned_max}` in `moth-lexical`, `JsNumericConversion::{classify, proven_safe_integer_narrowing}` and `JsNumericOperationLowering::classify` / `JsProvenIntegerLowering::classify` in `backends/js/numeric_carrier.rs`, and `HirModule::{compact_block_ids, block}`.

## Authorities read

`AGENTS.md`; `docs/developer-docs/style-guide/style-guide.mtf`; `docs/roadmap/audit-guide.md`; `docs/roadmap/audit-kinds/README.md`; `docs/roadmap/audit-kinds/redundancy.md`; `docs/roadmap/audits/README.md`; `docs/compiler-design-overview.md` opening, `Architectural invariants` and `Stage 5 > Numeric ownership`; the proof passages of `docs/developer-docs/compiler-design/backend-lowering/backend-lowering.mtf` and `docs/developer-docs/numeric-backend-contract/numeric-backend-contract.mtf`; the numeric rows of `docs/docs/progress/@entry.moth`.

## Existing findings and active plans checked

The audit log has no row for this area. `docs/roadmap/open-audit-findings.md` had no candidates. `docs/bug-reports.md` has no numeric-proof entry. `docs/roadmap/plans/core-math-fast-counterparts-plan.md` mentions `NumericProofs` only to say it is not mandatory proof, and is queued. No plan owns a change to this area.

## Findings

### AUD-0022-F01: Interval operator arithmetic is implemented twice

- State: `candidate`
- Kind: `Redundancy`

#### Evidence

`BlockAnalyser::prove_operation` in `analysis/numeric_proofs/mod.rs` (lines 324-344 and 390-403) computes closed `i128` result intervals for `Add`, `Subtract`, `Multiply` and `Negate` with checked endpoint arithmetic, then tests containment in the result domain (line 408).

`checked_result_range` in `datatypes/numeric_operators.rs` (lines 383-411) implements the same `Add`, `Subtract` and `Multiply` endpoint arithmetic with the same checked operations, the same four-corner product loop and the same `None` on unrepresentable intermediates. `numeric_operation_cannot_fail` (lines 330-375) repeats the `Negate` endpoint swap and the same containment test. The two copies differ only in their inputs: complete canonical operand ranges for the predicate, analysis-derived intervals for the side table.

#### Counter-explanation tested

The overview keeps semantic discharge separate from check elision (`Stage 5 > Numeric ownership`), and the predicate's doc comment says check elision "never feeds this predicate". That separation concerns *inputs and consequences*: discharge must not depend on optional proof facts. It does not require two implementations of interval arithmetic. Sharing a pure interval-to-interval leaf keeps the predicate independent of the side table.

Independent duplication can be deliberate oracle code. Neither copy is a test oracle. Both are production owners that must agree on the same mathematics. A sign or overflow fix applied to one would silently leave the other wrong.

This finding would be wrong if the two copies needed different rounding, overflow or endpoint rules. Both use exact checked `i128` arithmetic and decline proof on overflow.

#### Violated contract or cost

One semantic rule, the conservative result interval of a fixed-width integer operator, has two owners. `AGENTS.md` and the style guide require one owner per fact and rule. The cost is coordinated edits and drift risk between mandatory semantic discharge and optional backend check elision.

#### Root owner

`datatypes/numeric_operators.rs`, which already owns operator promotion, failure codes and the discharge predicate.

#### Suggested correction

Non-authorising. Move the operator interval arithmetic into one `numeric_operators` function over closed intervals, covering at least `Add`, `Subtract`, `Multiply` and `Negate`, and call it from both `numeric_operation_cannot_fail` and `prove_operation`. `IntegerDivide` and `Remainder` may stay in the analysis, since the predicate deliberately never discharges them. Delete the duplicated arms from `prove_operation`.

#### Fix scope and preserved invariants

Semantic discharge must still read only canonical operand types. The side table must stay optional and never feed discharge. Overflow of an `i128` intermediate must still decline proof. No change to accepted programs, folded values or emitted checks.

#### Required validation

`cargo test -p moth --lib numeric_proof`, the `numeric_operators_tests` module, and the JS and Wasm numeric proof lowering tests. Integration cases tagged for numeric proofs on both backends.

#### Linked findings

None.

### AUD-0022-F02: JS proven narrowing re-classifies its conversion and recovers it through `expect`

- State: `candidate`
- Kind: `Redundancy`

#### Evidence

`JsEmitter::integer_narrowing_is_proven` (`backends/js/js_statement.rs`, lines 397-416) classifies the cast with `JsNumericConversion::classify`, checks for `CheckedIntegerToInteger` and queries `integer_narrowing_is_safe`. It returns only `bool`.

Its lowering caller, `proven_narrowing_expression` (lines 369-391), calls it and then repeats the work: it re-destructures the policy, calls `JsNumericConversion::classify` again with `.expect`, computes the source carrier with `.expect`, and calls `proven_safe_integer_narrowing` (`numeric_carrier.rs`, lines 477-498). That function re-checks `CheckedIntegerToInteger` with a `let else` and returns `Option`, which is unwrapped with a third `.expect`. In total there are three variant checks and two classifications on one path. The three panics exist only because the first check's result was thrown away.

The other callers, `invoke_failure_edge_is_live` (`reachability.rs`, line 42) and `collect_used_cast_policies` (`emitter.rs`, line 507), need only whether the proof applies.

#### Counter-explanation tested

A boolean predicate is the simplest shape for the two liveness and demand callers. Returning the proven conversion as an `Option` costs those callers only `.is_some()`. The backend-local `CheckedIntegerToInteger` gate itself is legitimate: each backend consumes only the proof classes it supports. This finding concerns repeating that gate, not its existence.

This finding would be wrong if the second classification could produce a different result from the first. Both calls take the same `Copy` inputs into a pure function.

#### Violated contract or cost

The style guide requires invariant-backed `expect` to have a locally evident reason. Here the reason lives in another function. The classification is repeated on every proven narrowing. A change to the supported-narrowing rule must be made in both `integer_narrowing_is_proven` and `proven_safe_integer_narrowing`, or the `expect`s become reachable panics.

#### Root owner

`backends/js/js_statement.rs` (`integer_narrowing_is_proven`), with `JsNumericConversion::proven_safe_integer_narrowing` in `numeric_carrier.rs`.

#### Suggested correction

Non-authorising. Have one JS query return the proven raw conversion, for example `Option<JsNumericConversion>` built from a single classification and the table lookup. Liveness and demand callers test `.is_some()`, and lowering renders the returned value. Delete the re-classification and the three `expect`s.

#### Fix scope and preserved invariants

Reachability, branch inspection, helper demand and lowering must keep reaching the same decision through the same query. Emitted JS must be byte-identical.

#### Required validation

`backends/js/tests/numeric_proofs.rs`, `numeric_statements.rs` and the narrowing integration cases on HTML-JS.

#### Linked findings

None.

### AUD-0022-F03: Integer-domain guards repeat `NumericScalar::integer_range`

- State: `candidate`
- Kind: `Redundancy`

#### Evidence

`NumericScalar::integer_range` returns `Some` exactly for signed and unsigned integer classes. `Byte`, binary floats and `Dec` return `None`. `NumericScalar::is_integer` is true for exactly the same classes. In `analysis/numeric_proofs/mod.rs`:

- `prove_operation` (lines 319-322) returns early on `op.domain.integer_range()?`, then tests `!op.domain.is_integer()`. The second branch is unreachable.
- `canonical_interval` (lines 491-495) tests `is_integer` before returning `integer_range`, which already yields `None` for every rejected class.
- `cast_narrowing_is_provable` (lines 434-439) tests `target.is_integer()` before `target.integer_range()` with a `let else`. The `source.is_integer()` test is needed, because a `Byte` literal source still has an interval.

#### Counter-explanation tested

The extra guards could protect against `integer_range` later reporting a `Byte` range. If that changed, every other range consumer, including `numeric_operation_cannot_fail`, which tests only `integer_range`, would need review anyway. A second predicate in the analysis splits the "bounded integer domain" rule across two calls rather than enforcing it.

#### Violated contract or cost

This is small dead control flow and a second spelling of one domain rule. It is low impact. It is recorded because the module's soundness comments rely on exactly which domains have intervals.

#### Root owner

`analysis/numeric_proofs/mod.rs`.

#### Suggested correction

Non-authorising. Delete the unreachable `is_integer` test in `prove_operation` and the redundant `is_integer` tests on `canonical_interval` and on the narrowing target. Keep the source test.

#### Fix scope and preserved invariants

`Byte`, binary floats and `Dec` must still never prove.

#### Required validation

`cargo test -p moth --lib numeric_proof`.

#### Linked findings

AUD-0022-F01 touches the same function. One fix may take both.

## Checked and clean

- **Two fact sets.** `safe_operations` and `safe_narrowings` are both `FxHashSet<HirNodeId>` and could merge into one set, because node ids are unique. Each query site already knows its operation kind, so merging saves one field and one method but loses the named distinction at every caller. Kept.
- **Production invocation.** The base executable computes the table once in `service.rs`. Sidecars get a documented `NumericProofs::default()` placeholder in `materialisation.rs`, and the convergence publication loop overwrites it once from final HIR. That gives one owner, not a parallel path.
- **Backend threading.** `numeric_proofs` travels beside `hir`, `type_environment` and `path_table` as separate borrowed fields of `HtmlModuleCompileInput` and the lowering entry points. This matches how every other `ModuleExecutable` field is threaded and creates no second representation.
- **JS shared classification.** Statement lowering, helper demand, Invoke liveness, cycle detection and branch inspection all reach `classify_numeric_operation` or `invoke_failure_edge_is_live`. The repeated calls are one owner applied at several consumers, as `reachability.rs` documents. The live block closure is computed once per function in `functions_to_emit`. The separate unpruned declaration closure has a documented storage reason.
- **`Infallible` or proven.** JS (`is_discharged() || integer_operation_is_safe`) and Wasm (`failure_mode == Infallible || integer_operation_is_safe`) each apply the overview's rule that lowerers treat `Infallible` as proven. Folding `Infallible` statements into the table would make mandatory totality depend on an optional table, which the overview forbids. Kept as a one-line rule per backend.
- **Wasm query.** Wasm consumes only `integer_operation_is_safe` and never narrowing facts. This is the documented "each target consumes only the proof classes its lowerer supports".
- **CFG edges.** `count_incoming_edges` uses the shared `for_each_terminator_target` instead of a local edge list.
- **Seeds.** The `seeds` map plus `current_block` index is the minimal state for the single-edge seed rule.
- **Explicit `LoopStepFailure` arm.** It behaves like the wildcard arm but carries a distinct reason comment. Too small to file.

## Limitations

- The JS numeric carrier (`numeric_carrier.rs`) and runtime kernels were read only where they gate proof classes. The carrier's other conversion and lowering tables were not audited for redundancy.
- Wasm LIR emission of `IntegerCheckMode::ProvenSafe` below `hir_to_lir/stmt.rs` was not inspected.
- Test-only code under `numeric_proofs/tests/`, `backends/js/tests/` and `backends/wasm/tests/` was not audited.
- An incidental lead outside this area, a JS block lookup map that duplicates `HirModule::block`, is recorded in `docs/bug-reports.md`. It is not covered by this audit.
