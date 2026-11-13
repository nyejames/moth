# Core Math fast counterparts implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task by task. Track work with `- [ ]` checkboxes.

**Status:** Queued. Start after constraints and fast arithmetic, then numeric optimisation, have merged and published their permanent contracts.

**Goal:** Add a small, justified set of explicit `fast_*` F64 functions to `@core/math`, preserving ordinary Math APIs while exposing selected trap-only or deliberately cheaper numerical contracts.

**Architecture:** Extend Core Math's existing package-local registration table and the compiler-owned trusted-call metadata established by the constraints work. Reuse the single external F64 value-validation boundary. Keep trivial functions as inline JS expressions and use the existing demand-emitted Core helper infrastructure for any genuinely composite algorithm.

**Tech stack:** Rust, external package registry, HTML-JS backend, Core Math `.mtf` references, Moth integration tests and benchmarks.

**Repository destination:** `docs/roadmap/plans/core-math-fast-counterparts-plan.md`. This is plan **3 of 3**, sequenced after `constraints-and-fast-math-plan.md` and `numeric-optimisation-plan.md`.

## Global constraints

- Follow `AGENTS.md`, `index.md`, the developer style/testing/validation guides, `docs/compiler-design-overview.md` and `docs/build-system-design.md`. Read `docs/docs/packages/core/math/math.mtf` and `docs/roadmap/plans/packages/core-math.md` for the existing Math contract.
- Read the permanent constraints and numeric backend contracts delivered by plan 1. Plan 2's final checked-carrier behaviour is the performance baseline. Do not reinterpret the `$fast_math`, `$safe_math`, `$infallible` or ordinary numeric contracts.
- Keep the existing `@core/math` origin, explicit dependency, F64-only positional signatures, shared parameters, fresh returns and absence of a declared Error slot. Keep all current ordinary function bodies, names and numerical guarantees.
- A `fast_*` function is **explicitly selected by its name**, callable with or without `$fast_math` on the caller. `$fast_math` does not rewrite ordinary `sqrt`, `hypot` or other calls into fast counterparts.
- Registered fast helpers contribute fast-math use to `$safe_math` and their potential semantic traps to `$infallible`, through trusted per-callable metadata, regardless of calling scope, alias, facade or re-export. Do not infer those facts from the `fast_` prefix or package path.
- NaN and infinity must never become Moth F64 values. Fast numerical invalidity is a semantic trap, **never an implicit recoverable Error**, including from inside a function that declares `Error!` or a surrounding `catch`. The builder-selected execution-instance fault policy is the one established by plan 1.
- No F16/F32 overloads, generic Math, new approximations with unapproved bounds, Dec fast mode, separate Math error ABI, new Wasm Math lowerings or custom constant evaluator.

## Review focus

1. **False speedups:** regular Core Math already emits `Math.sqrt`, `Math.log`, `Math.exp` and `Math.pow` inline. The fast name alone cannot justify an identical lowering. Test and measure before extending the public API.
2. **Math's shared finite boundary:** ordinary non-finite results currently reach the shared external F64 validation path, with a recoverable builtin code 304 when the surrounding declared `Error!` permits it. Fast calls must use that same validity predicate but always trap, without a new Math-specific guard.
3. **`fast_hypot` intermediate overflow and underflow:** its explicit sequential arithmetic can trap on large finite inputs or round tiny results to zero while regular `hypot` remains finite. These are intended differences and need executable tests.
4. **One-time argument evaluation:** an inline `fast_hypot` expansion that repeats `#0` or `#1` would repeat side effects. Use a normal runtime helper with one argument evaluation, not a large duplicated inline expression.
5. **Trust and target reachability:** an aliased/re-exported fast call remains fast. A reachable Math call still rejects HTML-Wasm with the existing target diagnostic, while an unreachable call must not force a Wasm requirement or JS helper emission.

## 1. Accepted numerical and API contract

### Candidate selection

The five **candidate** names are `fast_sqrt(x)`, `fast_log(x)`, `fast_exp(x)`, `fast_pow(base, exponent)` and `fast_hypot(x, y)`. The first four retain the regular function's accepted input domains, exact identities, F64 precision/accuracy policy and ordinary arity. Their difference is trap-only invalid-result delivery and, **only if measurements establish it**, cheaper lowering. They must not quietly use a lower-accuracy approximation or a different host primitive.

`fast_hypot(x, y)` has an approved distinct composition, independent of whether the host `Math.hypot` happens to be fast:

1. Evaluate both arguments once, in normal call order, as finite F64 values.
2. Compute `x2 = round_F64(x * x)` and trap if non-finite.
3. Compute `y2 = round_F64(y * y)` and trap if non-finite.
4. Compute `sum = round_F64(x2 + y2)` and trap if non-finite.
5. Compute `result = sqrt(sum)` using the regular accepted square-root precision contract, validate finiteness and return F64.

The sequence is part of the public contract. Each step completes at F64 precision and cannot be reassociated, fused or replaced with the overflow-avoiding ordinary `Math.hypot` algorithm. Underflow to finite zero is valid. `fast_hypot(1e200, 1e200)` traps on the first square, while ordinary `hypot(1e200, 1e200)` produces a finite result. With very small finite inputs, fast composition may produce zero where ordinary hypot retains a nonzero result. `fast_hypot(3.0, 4.0)` is finite and approximately five.

### Shipping rule

Begin by profiling **all five candidates** against the merged ordinary Core Math path. Seal the public subset with Nye before adding registration rows:

- Retain a candidate when it has a measured material advantage in a representative successful-call workload **or** a useful, explicitly different numerical contract. Document which reason applies.
- `fast_hypot` is the principal algorithmically distinct candidate and should be retained if its measured cost and documented overflow/underflow tradeoff are acceptable. Compare it with ordinary hypot rather than claiming it is automatically faster.
- Omit `fast_sqrt`, `fast_log`, `fast_exp` and `fast_pow` if their generated code and end-to-end cost are effectively identical to regular calls after plan 2. Their trap-versus-Error difference alone is not an obligation to double the API.
- Record omitted candidates and the evidence. If the measurements suggest a different public subset or no worthwhile helper, stop at this gate for Nye's decision. Do not invent a wider approximation or silently publish five aliases.

After the subset is sealed, the remaining tasks implement **only** those names. Every selected function keeps the existing regular Math mathematical result contract unless the explicit `fast_hypot` composition above overrides it. There is no global `fast_math` Core Math dispatch table.

### Constraint and failure policy

- Every selected fast helper declares normal `F64` success slots without a typed Error return, matching ordinary Math.
- The *validated* host result must be finite before publication. Ordinary Math keeps its current shared boundary and conditional recoverable code 304. Selected fast helpers use semantic-trap delivery at that same boundary, even in an enclosing builtin `Error!` function.
- `catch` cannot convert a fast helper's trap into Error. A `$safe_math` boundary rejects transitive fast-helper use even when the result happens to be finite or the call is later folded away.
- `$infallible` accepts a fast-helper call only if the existing mandatory semantic proof policy can exclude every possible trap. A syntactically valid F64 signature or optional `NumericProofs` entry is not such proof. Do not add a special transcendental solver.
- Keep the regular `sqrt`, `log`, `exp`, `pow` and `hypot` APIs unchanged. Ordinary calls inside `$fast_math` remain ordinary calls with the regular Error/guard boundary.

### Target contract

V1 executes on the existing HTML-JS Core Math path. The current registry has `wasm: None` for Math and `src/backends/external_package_validation.rs` rejects reachable unsupported calls before lowering (`MOTH-RULE-0058`). Preserve that rule for fast counterparts. Do not introduce a Wasm import stub, a new Wasm Math capability or an alternate result to force fast calls to compile. Inactive/unreachable calls retain the existing reachability policy.

## 2. Existing owners and smallest implementation shape

Reconfirm locators when the plan activates. These paths were checked against main when this plan was written.

- `src/builder_surface/core_packages/math.rs` contains one `MathFunctionSpec { name, js_lowering, parameter_count }` table and one `ExternalFunctionSpec` registration loop. Extend only the rows and bounded trusted-call contract metadata needed for the sealed subset. Do not duplicate the ordinary registration loop or build a function-family hierarchy.
- `src/compiler_frontend/external_packages/definitions.rs` owns `ExternalFunctionSpec`, `ExternalFunctionLowerings` and `ExternalJsLowering::{InlineExpression,RuntimeFunction}`. Reuse plan 1's trusted per-callable fast-use/trap facts. If the F64 boundary lacks a per-callable trap-delivery policy, add **one narrowly named classification in this existing owner**, rather than a generic effects structure.
- `src/compiler_frontend/hir/hir_expression/numeric.rs` and its external-value validation owners already select the shared finite-result recovery/fatal lane. Thread trusted fast delivery through that same owner, not a second finite predicate in Math.
- `src/backends/js/js_calls.rs` already lowers external inline templates and runtime function targets. One occurrence of each argument is mandatory. Keep the sealed unary/binary direct `Math.*` candidates inline when that remains their complete lowering.
- For `fast_hypot`, register `ExternalJsLowering::RuntimeFunction("__moth_math_fast_hypot")`. Put its nontrivial, single-purpose JS body in `src/backends/js/package_bindings/core/math.rs` and use `CoreJsHelper`, `referenced_external_runtime_function` and `emit_referenced_core_helpers` from `src/backends/js/package_bindings/core/mod.rs`. Update the current Core helper inventory so first-party validation sees it. Emit only when reachable.
- Reuse the numeric runtime's existing trap/integrity mechanism established by plan 1 for `fast_hypot`'s step checks. Never call an Error-carrier-producing operation and silently unwrap it as if its failure were recoverable. Do not add a generic Math helper runtime or a second finite-result validation implementation.
- `docs/docs/packages/core/math/math.mtf`, `math-basic.mtf` and `@entry.moth` own Math's user-facing contract and navigation. `docs/roadmap/plans/packages/core-math.md` is the ongoing package programme record, not a duplicate numeric specification. The progress owners are `docs/docs/progress/@entry.moth` and `docs/docs/progress/packages-and-builders/@entry.moth`.

## 3. Execution tasks and gates

### Task 0 - Baseline, evidence and public subset

- [ ] Establish a fresh branch/worktree on the merged first two plans and record the current Core Math table, shared finite boundary, trusted metadata and JS helper emission. Run the required baseline validation.
- [ ] Create a small benchmark corpus for the five candidates, with both plain no-error-slot and builtin `Error!` callers. Measure regular Math success calls, current guard/carrier overhead, JS text size, helper count and frontend cost. Use real runtime arguments, warm-up and repeat runs. Measure exceptional behaviour separately rather than comparing a trap to an Error as a speedup.
- [ ] For `fast_hypot`, benchmark ordinary overflow-avoiding hypot against the documented sequential algorithm on normal finite inputs. Include the cost of every mandatory finite check. Identify the actual algorithmic tradeoff.
- [ ] Record a proposed exported subset and evidence for every inclusion or omission. **Get Nye's approval before creating public API rows.** Do not expand the task when a candidate proves redundant.

**Gate:** A small, named public subset is explicitly accepted. Regular and fast execution costs are measured under the same toolchain and semantic conditions.

### Task 1 - Registry, trust and source-level constraints

- [ ] Add failing registration and frontend tests for each sealed name's F64 signature, arity, fresh success result, lack of Error slot, explicit Core dependency and access through direct/aliased/re-exported imports.
- [ ] Add failing `$safe_math` and `$infallible` tests. Fast helpers contribute fast use independent of caller mode. Potential invalid inputs contribute a semantic trap. Regular Math calls inside a fast scope retain their ordinary contract. Unknown external wrappers remain conservative.
- [ ] Extend the existing `MathFunctionSpec` table and one registration loop, using trusted per-callable facts from plan 1. Validate those facts at the existing provider boundary. Keep regular row metadata unchanged and preserve current public interface/fingerprint propagation.
- [ ] Run the focused registry/frontend tests and review emitted public contract facts and diagnostic witnesses for a private helper changed from ordinary to fast.

**Gate:** Selected fast APIs have one provider-owned identity and correct transitive constraint semantics. No name-prefix dispatch or separate package registration mechanism exists.

### Task 2 - Shared F64 boundary and simple JS lowerings

- [ ] Add failing executable tests for the sealed simple candidates. Test normal values, invalid domains and non-finite output in both no-error-slot and declared builtin `Error!` callers. Verify a regular counterpart still follows the recoverable code-304 path when allowed, while a fast counterpart always traps beyond Moth catch.
- [ ] Route the trusted fast trap policy through the existing external F64 result validation and HIR failure delivery. Preserve first-failure order and exactly-once argument evaluation. Do not add a Math-specific validity predicate or typed Error channel.
- [ ] Keep directly corresponding Math functions on their existing `InlineExpression` strategy. Avoid a wrapper that only calls the same `Math.*` function and adds overhead. Verify generated calls contain no unintended carrier or duplicate guard.
- [ ] Run focused Node observer, diagnostic and emitted-source tests. Confirm supported signatures and absent Wasm lowerings produce the existing target failure, not an internal compiler error.

**Gate:** Same accepted numerical values as regular Math, correct trap-only delivery and no duplicated finite-check logic.

### Task 3 - The explicit `fast_hypot` composition

- [ ] Add failing Node-executed tests for `(3, 4)`, `(1e200, 1e200)`, `(1e-200, 1e-200)`, finite near-overflow operands, signed zeros and a caller whose argument expressions have observable side effects. Assert the defined difference from ordinary `hypot` and the absence of later effects after a trap.
- [ ] Register the approved helper with one `RuntimeFunction` target. Implement the ordered F64-square, F64-square, F64-sum and square-root sequence in a single `CoreJsHelper`. Validate each specified intermediate and result through the shared fast-trap mechanism. Do not compose it from Moth call expressions that add their own failure boundaries.
- [ ] Integrate it with `src/backends/js/package_bindings/core/mod.rs` and existing emitted-helper inventory. Check that unused fast Math functions emit no runtime helper, and that a referenced fast helper emits exactly once across modules.
- [ ] Run paired normal and boundary tests, plus a mutation check changing a square to an addition or replacing the helper with `Math.hypot`. Each mutation must make an identifying fixture fail.

**Gate:** One-time argument evaluation, explicit intermediate rounding/validation and demand-only helper emission all hold. This task is skipped if `fast_hypot` was excluded at Task 0.

### Task 4 - Documentation, performance and deletion review

- [ ] Update the canonical Math reference with **only the shipped subset**, naming its signatures, trap semantics, exact/tolerance-based expectations and `fast_hypot`'s overflow/underflow tradeoff. Keep the Basic reference compact and show one example of explicit fast selection. Record the honest supported-target matrix.
- [ ] Update the living Core Math programme plan, relevant constraint docs, progress matrices and package navigation. Keep ordinary Math documentation intact and do not promise universal speedups, constant-time behaviour, exact transcendental rounding or Wasm support.
- [ ] Re-run the baseline benchmark corpus and report runtime/variance, code size, helper counts and frontend time. Explain every retained helper's value from Task 0 without claiming the fast name itself confers speed.
- [ ] Inspect the final diff for repeated registration loops, duplicate boundary predicates, extra adapter layers, dead carrier wrapping, unnecessary allocations and `#[allow(...)]` suppressions introduced to hide complex code. Delete or consolidate these before merge. Confirm inline trivial helpers remain inline and only `fast_hypot` needs a runtime helper if shipped.
- [ ] Run `cargo run -- tests --tag math`, the relevant external/constraint fixtures, `just validate`, the documentation gate and `just validate-full` on the integrated tree. Follow benchmark clean-worktree rules and record every skipped or blocked check. Finish the Slice audit and verify all package/source reference links.
- [ ] Transfer lasting rules to permanent docs, mark only shipped functions delivered and remove this plan and its sequenced roadmap bullet in the completion commit. Continue tracking the independent Core Math programme in its existing living plan.

**Final gate:** The exported fast Math subset has an observed reason to exist, precise documented semantics, correct constraint/trap behaviour and executable coverage. Ordinary Math and all unsupported-target behaviour remain unchanged.

## 4. Regression fixture owners

Prefer extending existing test families rather than adding a large duplicate suite:

- `tests/cases/core_math_basic_functions` and `core_math_transcendental_functions`: ordinary mathematical values and approximate comparisons.
- `tests/cases/core_math_float_boundary_validation`: shared code-304 recovery versus fast trap, invalid square root, logarithm, power and exponential overflow.
- `tests/cases/core_inline_expression_lowering`: direct generated call shape and no duplicate argument evaluation.
- `tests/cases/core_math_type_errors` and `core_math_arity_error`: reuse existing signature-error coverage for the new spelling.
- Backend Core helper tests and first-party helper inventory: demand emission and actual `fast_hypot` execution, if selected.
- Existing dependency, namespace, facade and public-interface tests: transitive fast identity and re-exported constraints.

Use exact assertions only for contract-fixed identities. Use justified tolerances for transcendental approximations. Test an actually different wrong lowering rather than a fixture that could pass with `Math.max`, regular `Math.hypot` or a swapped registration.
