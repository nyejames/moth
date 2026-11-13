# Numeric optimisation implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task by task. Track each task using its `- [ ]` checkboxes.

**Status:** Queued. Begin after the constraints and fast arithmetic plan has merged and its permanent semantic contracts are published.

**Goal:** Reduce checked-numeric runtime overhead and emitted JS noise by eliminating provably unnecessary success carriers, error branches and predicates, while preserving exact Moth results and all observable failure behaviour.

**Architecture:** Reuse the frontend's sparse `NumericProofs` side table and existing mandatory numeric failure classification. Perform optional simplification within backend lowering and helper emission, paired to immutable HIR. Add bounded analysis only when correctness and measured value justify it.

**Tech stack:** Rust, existing numeric HIR, HTML-JS/HTML-Wasm, Node observer/integration tests and Moth benchmark harness.

**Repository destination:** Add `docs/roadmap/plans/numeric-optimisation-plan.md`. This is plan **2 of 3**, sequenced after constraints/fast arithmetic and before a separately designed Core Math fast-counterparts item. It can be reviewed on its own after plan 1 publishes the final numeric backend contract.

## Global constraints

- Read `AGENTS.md`, `index.md`, the canonical style/testing/validation guides, `docs/compiler-design-overview.md`, `docs/build-system-design.md` and the numeric backend contract published by plan 1. Work from the final merged implementation, not hard-coded assumptions about today's structure.
- **No language change.** Checked arithmetic stays checked, fast operators keep their defined wrapping/trap semantics and Dec stays ordinary even in fast scopes. This plan does not introduce new directives, accepted arithmetic pairs, Core Math functions, compiler-facing guarantees or public ABI changes.
- **Source acceptance is independent of optimisation.** `numeric_operation_cannot_fail`, declared constraints, author-written catch eligibility, `Error!` signatures and public summaries continue to mean the same thing with the optimiser on or off.
- A missing proof means use the existing checked path. Optional proofs never justify ignoring a semantic trap, an observable failure, a side effect, a numeric rounding boundary or a target capability restriction.
- Do not introduce a general optimiser, effect system, new CFG IR, second interval solver or whole-program specialisation framework. Reuse or delete existing scaffolding whenever a narrower owner suffices.
- Use a failing regression first, then the smallest implementation, focused verification, Slice review and a coherent commit. Benchmarks support decisions, not a commitment to a speedup without evidence.

## Review focus and acceptance hazards

1. A semantic `Error!` or authored `catch` boundary may still be necessary even when its numeric producer is proven total. Preserve the handler's source validation, declared result ABI, evaluation order and every reachable side effect.
2. A proof must match the **exact final HIR executable and numeric semantics**. Optional integer intervals are not valid for fast-mode numeric ops, binary floats or Dec merely because they share an operator spelling.
3. A raw JS scalar bypass must preserve integer signedness, bounds, canonical integer zero, exact I64/U64 BigInt values and F16/F32 rounding. Removing a wrapper must not change Moth's value representation.
4. A trap or Error from a nested operation must still occur before later effects, including in a discarded expression, handler or compound store. Reordering or eager calculation is a correctness failure.
5. Measure generated helper count, allocations and runtime with ordinary, proven-safe and failure-heavy programs. Any proposed new analysis complexity must earn its cost.

## 1. Scope and non-goals

Deliver three closely related improvements where they demonstrably help:

- **Scalar successes:** emit scalar numeric values directly when a mandatory semantic fact or verified optional proof establishes that an operation cannot fail. Remove corresponding temporary JS `{ tag: "ok", value }` carriers, unwraps and dead failure branches in the **backend's generated code**. Preserve real result carriers across ABI boundaries that require them.
- **Total operators:** give intrinsically total operations a scalar path. Today `numeric_failure_codes` already classifies binary-float negation and non-dividing, non-power Dec operations as having no recoverable failure, yet their JS helper family may allocate success carriers. Reuse this single classification owner, and retain current Dec coefficients and half-even rounding.
- **Demand-driven checks and helpers:** extend the existing narrow integer proof table or helper usage classification only when profiling identifies a repeated redundant predicate or wrapper. Remove helpers that are provably unused after lowering.

Out of scope: converting checked arithmetic into wrapping arithmetic, removing safety checks without proof, implementing fast Dec semantics, adding F64 `fast_*` Core Math functions, new public constraint effects, broad loop-induction or interprocedural range solving, source-level HIR rewrites for performance alone and expanding Wasm's supported type/foreign surface.

### Exact optimisation rules

- HIR `NumericFailureMode::{Trap,Infallible}` describes the statement's failure delivery. `HirTerminator::Invoke` owns recoverable error edges. Plan 1 adds an arithmetic semantic mode. These remain independent. An infallible **fact** permits scalar lowering, not a new source mode.
- Retain an Invoke's observable success/error protocol unless the backend locally proves the numeric error successor unreachable and can simplify it without invalidating source semantics, live handler work or the result-slot dominance rules. Keep a genuine external/result ABI carrier if its caller expects one.
- `NumericProofs` may prove a checked operation or integer narrowing safe. A proof can eliminate a check and its helper for that exact operation, but not an upstream operand's failure, an earlier call's effects or a separate write-back conversion.
- The table is optional and sparse. If a case is unproved, retain all required ordinary checks. Do not infer that `NumericFailureMode::Infallible` means every later implicit cast or range update is infallible.
- Floating point has a distinct rule: finite unary negation is total, but float addition, subtraction, multiplication, division, remainder, power and precision conversions may need per-operation validation. Never eliminate those validations based only on a finite-input fact.
- Dec add/subtract/negate are exact, multiplication still requires its documented half-even result rounding and division/remainder/power retain their actual error cases. Total Dec operations can return scalar coefficients without an artificial Result envelope. Do not change round-at-each-language-operation behaviour or treat Dec as fixed-width wrapping.
- Fast numeric operations already follow plan 1's scalar/trap path. Avoid building another fast path here. Restrict proof-table predicates to the ordinary checked form unless a later separately approved proof has a demonstrated benefit and fully specified identity.

## 2. Current owners to reuse

The following files were verified on Moth's main branch when the plan was written. Reconfirm after plan 1 merges.

- `src/compiler_frontend/datatypes/numeric_operators.rs`: `numeric_failure_codes` is the **single semantic fallibility classifier**. `numeric_operation_cannot_fail` proves bounded all-input integer cases for mandatory semantic use. Do not duplicate either classification in backend-local match trees.
- `src/compiler_frontend/analysis/numeric_proofs/mod.rs`: one optional `NumericProofs` with `safe_operations` and `safe_narrowings`, keyed by `HirNodeId` and paired with exact validated HIR. Its current closed-interval, block-local last-written-local algorithm has no CFG predecessor propagation. Extend only for a named measured case and keep the invalidation rules.
- `src/compiler_frontend/hir/numeric.rs`, `hir/hir_expression/numeric.rs`, `hir/private_failure_lane.rs`: current HIR numeric operation and Invoke/failure ownership. Do not mutate frontend HIR to force a backend optimisation.
- `src/backends/js/js_statement.rs`: `lower_numeric_operation`, `proven_integer_operation_expression` and `emit_numeric_carrier_assignment` already select scalar proven-safe expressions versus `{tag,value}` helper results. Extend or consolidate this path rather than creating a new pass.
- `src/backends/js/emitter.rs` and `runtime/mod.rs`: already calculate `NumericRuntimeHelperUsage` from reachable HIR. `runtime/numeric.rs` owns Number/BigInt/Dec/float helper generation and currently emits `__moth_numeric_trap` when numeric families are used. Refine demand by actual emitted callers without creating parallel reachability machinery.
- `src/backends/js/numeric_carrier.rs`: canonical carrier width and representation. Keep I8-U32 Number and I64/U64 BigInt distinctions. Reuse existing width/float/Dec helpers rather than encoding types again in emitted strings.
- `src/backends/wasm/hir_to_lir/stmt.rs` and `emit/checked_integer.rs`: already consume `NumericProofs` and `IntegerCheckMode::{Runtime,ProvenSafe}`. Look for real redundant LIR instructions. `emit/checked_float.rs` owns float checks. Do not introduce a competing Wasm proof table.
- Tests: `src/backends/js/tests/numeric_proofs.rs`, `src/compiler_frontend/analysis/numeric_proofs/tests/` and `tests/cases/` numeric exact-width/debug/release fixture families. Use existing test utilities and observer output.

## 3. Execution tasks and merge gates

### Task 0 - Baseline, fixtures and cost model

- [ ] Activate against merged plan 1 and record its revision, compiler flags, supported JS/Wasm matrix and the permanent numeric contract. Start an isolated worktree or branch.
- [ ] Run `just validate` and the documented benchmark checks. Select a small, representative corpus: checked/proven-safe mixed-width integer chains, I64/U64 BigInt, simple Dec arithmetic, finite-float negation, real recoverable arithmetic failures, catch-heavy bodies and a no-numeric baseline. Record raw baseline runtime, generated JS size/helper count, allocations where practical and frontend time.
- [ ] Trace generated code for each case through the present proof decision, JS helper selection and Wasm check mode. List actual wasted carriers or predicates, with a before sample. Prioritise measured hotspots instead of adding generic abstractions for hypothetical cases.

**Gate:** At least one reproducible unnecessary carrier/check is observed with exact semantics and a test owner. If benchmarks show no meaningful runtime or code-size benefit, document that and narrow the following tasks.

### Task 1 - Scalar-total operations

- [ ] Add failing emitter/Node tests for finite float unary minus (including `+0.0` to `-0.0`), Dec add/subtract/negation/multiplication, simple mixed numeric chains and their runtime parameter variants. Verify values, scale, half-even rounding and representation, not only generated text.
- [ ] Route known-total operations to scalar results using the existing `numeric_failure_codes` contract or a minimal frontend-owned totality fact. Avoid changing its classification or adding redundant backend semantic tables. Preserve single evaluation and normal error paths for Dec division/remainder/power.
- [ ] Convert total helper implementations to scalar outputs where safe, or bypass the helper when simpler. Reuse the same Dec arithmetic/rounding kernel so the carrier-free and fallible paths cannot diverge. Delete obsolete ok-carrier wrapping and unwrap sites only after no callers remain.
- [ ] Add negative tests showing a dividing or exponentiating Dec operation still yields ordinary recoverable Error under `$fast_math`. Ensure no Dec fast helper is generated.

**Gate:** Total numeric paths emit scalar success without artificial recoverable carriers, while Dec and float values match the previous runtime exactly.

### Task 2 - Proven-safe checked operations and dead edges

- [ ] Add paired proof/no-proof tests for D15 mixed `I8 + U8 -> I16`, equal `U8 + U8` that may overflow, signed MIN/MAX, U32 low-word products, I64/U64, integer narrowings and cached-local invalidation after writes/calls. Prove the selected expression actually has or lacks a `NumericProofs` entry. Compare analysed and empty tables in existing internal test utilities, without adding a user-facing optimiser switch.
- [ ] Tighten `lower_numeric_operation` and `proven_integer_operation_expression` where proof-backed emission still creates a carrier, unwrap or range predicate. Preserve `-0` normalisation on Number integer carriers, exact BigInt ranges, division/remainder edge behaviour and operand evaluation order.
- [ ] Where a proven numeric Invoke has a dead error edge, simplify it **only inside backend lowering** when success slots and handler reachability are sound. Verify catch handlers are validated before elimination and preserve non-numeric operations in the same block. Keep declared `Error!` ABI unchanged.
- [ ] Reconcile JS usage discovery with the emission decision so no unused checked-family helper or `__moth_numeric_trap` is emitted when every reachable operation is scalar. Prefer reusing one operation classification rather than two handwritten tests that can disagree.
- [ ] For Wasm, inspect current `IntegerCheckMode` lowering, remove duplicated proven-safe checks only where still emitted and preserve present target capability gates. A proof must not incidentally enable recoverable numeric/result channels on an unsupported Wasm surface.

**Gate:** Identical observable success and failure with analysed or empty optional proof tables. No live handler, call or runtime trap has been optimised away.

### Task 3 - Optional bounded proof improvement, if justified

- [ ] Profile the coverage of the current block-local interval cache. Identify a common exact pattern with missed safe checks and a measurable generated-code/runtime cost. If none exists, mark this task intentionally skipped rather than expanding analysis for its own sake.
- [ ] For an approved pattern, first add proof and counterexample tests covering assignments, calls, branches, joins, recursion, loops, aliases, unknown values and `i128` endpoint overflow as applicable. Uncertain paths must return no fact.
- [ ] Extend the single `NumericProofs` owner and reuse its `HirNodeId` identity, conservative interval arithmetic and invalidation rules. Avoid adding persistent state per expression or a whole-function fixpoint. Document the exact supported fact and why it is sound.
- [ ] Measure analysis time, allocations, generated code and runtime benefit. Revert the extension if its implementation cost and complexity outweigh the demonstrated gain.

**Gate:** Optional analysis remains bounded, backend-only and cheap on programs where it proves nothing.

### Task 4 - Regression, simplification and merge

- [ ] Execute paired debug/release JS observer tests and supported Wasm parity tests, with runtime values that cannot all fold. Include actual arithmetic failure, a caught Error, an escaping Error, a fast integer wrap, fast division trap, an uncalled invalid branch, Dec under fast mode and a compound store failing after RHS effects.
- [ ] Re-run the exact baseline benchmark corpus with the same inputs and toolchain. Report median/runtime variation, generated JS bytes, helper counts, frontend cost and any effect on build time. Never classify a source-semantic change as a performance improvement.
- [ ] Perform a deletion-focused review: consolidate duplicated operator classification, remove orphan helper emitters and redundant `{tag,value}` wrappers, avoid duplicate scans in `emitter.rs`/`js_statement.rs`, and trim Rust functions made overly complex by new branches. Retain one clear numeric fast/ordinary/totality decision path.
- [ ] Run focused tests after each slice, `just validate` at integration boundaries and `just validate-full` on the final tree. Follow the documented benchmarking hygiene (clean benchmark worktree where required), documentation checks and Slice review. Record blocked validations explicitly.
- [ ] Update only permanent developer documentation that describes backend optimisation/proof policy and the progress matrix. No source-level numeric contract changes are authorised. On completion, remove this roadmap plan and bullet while leaving the separate future Core Math counterpart item queued.

**Final gate:** Checked code has fewer **measured** redundant carriers/checks without any changed result, Error, trap, catch, type, ABI or unsupported-target behaviour. Optional analyses neither alter constraint acceptance nor become a second semantic authority.

## 4. Final reviewer checklist

- Did we delete at least as much redundant generated/runtime machinery as we added where the old path became obsolete?
- Is operator fallibility still defined once in `numeric_operators.rs` and typed carrier representation once in the backend's numeric owner?
- Are semantic validity, failure delivery, proof evidence and emitted representation visibly separate in Rust APIs?
- Does the same HIR pair with its own proofs for main modules and generated sidecars without remapping or invalid caches?
- Can a contributor explain every new test's unique boundary role? Equivalent widths should share test cases instead of accumulating fixture copies.
- Are possible traps, actual Errors, live handlers, F16 rounding and Dec recovery protected by executable tests, not just generated-source string assertions?

The next roadmap item evaluates `fast_*` Core Math F64 counterparts as a separate API and package-lowering decision. This plan neither registers nor assumes them.
