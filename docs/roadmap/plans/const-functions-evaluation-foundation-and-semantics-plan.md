# Const Functions: Shared Evaluation Foundation and Semantics Implementation Plan

> **For agentic workers:** Implement this plan one independently reviewable phase at a time. Follow repository `AGENTS.md`, the canonical code style and testing rules and the phase review gates below. Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` when available. A phase may be committed, integrated and paused before the next one starts.

**Goal:** Make const-function execution smaller, more efficient and easier to extend by sharing direct typed-expression value evaluation, then add match statements, native multi-return/multi-bind and immutable receiver methods.

**Architecture:** Retain Stage 4's compact typed `ExprId` graph, `ConstValueStore`, the existing iterative `FoldContext` and the prepared const instruction/frame machine. Separate computing a value from committing an AST replacement, then remove redundant rewriting for proven-safe expression forms. Keep TIR as the sole template authority and HIR as the first backend-facing IR.

**Tech stack:** Rust, Moth compiler frontend, typed AST, TIR, existing integration runner and frontend benchmark instrumentation.

**Spec and permanent authorities:** `docs/compiler-design-overview.md` (Stage 4, typed expression ownership, result slots and const execution), `docs/docs/functions/function-declarations.mtf`, `docs/docs/functions/returns-and-multiple-values.mtf`, `docs/docs/structs/receiver-methods.mtf`, `docs/docs/constants/constant-folding.mtf`, `docs/docs/loops/collection-loops.mtf`, the canonical pattern references and `docs/docs/design-scope/deferred-and-outside-scope.mtf`. The accepted decisions below supersede the old V1 restrictions only for the features explicitly approved here. Transfer enduring decisions into their canonical owners as implementation lands.

**Status:** Queued
**Current slice:** Phase 0, current-tree discovery and baseline
**Blockers:** None known
**Next action:** Activate on a fresh branch when scheduled. Re-read current authorities and measure the existing implementation before editing it.

## Global constraints

- Respect the current repository `AGENTS.md`, `docs/developer-docs/style-guide/style-guide.mtf`, `testing.mtf` and `validation.mtf`.
- Compiler semantics and const evaluation stay Stage 4-owned. Do not introduce HIR execution, another parser, another type checker or another template engine.
- One authoritative operation/value meaning. Reuse resolved typed operations, numeric/cast/Core evaluators, `ConstValueStore` and TIR.
- Preserve zero, one and multiple ordered success slots without tuples. Preserve a distinct error channel and distinct Never contract.
- Preserve immutable source expression rows during evaluation. Create replacement expression rows only when a compiler consumer needs them.
- Keep required source const calls mandatory in all profiles. Keep ordinary-call release speculation conservative, optional and non-diagnostic on refusal.
- Preserve determinism, evaluation order, source provenance, finite work/memory limits, transactionality and cross-module semantic identities.
- Prefer small direct Rust changes and deletion over wrappers, registries, compatibility layers, caches or speculative generality.
- Preserve benchmark and test ownership. Benchmark fixtures never substitute for correctness tests.
- Do not edit generated documentation or benchmark history directly.
- The requested roadmap placement is immediately after Core Math fast counterparts. This plan may pause after any completed phase and resume against a newer `main`.

## Review focus

Five failure classes must have an explicit regression owner in the phases below.

1. **Required versus speculative calls:** Required const failures remain source diagnostics. Refused release candidates remain the original runtime calls with unchanged checks.
2. **Lazy execution:** Inactive logical operands, match guards, match arms, catches and handlers never execute, even when they contain a failing operation.
3. **Value identity and provenance:** Refused value staging publishes nothing. Successful values preserve exact numeric types, source-config origins and structural resource anchors.
4. **Native multi-results and aliases:** A producer executes once. Ordered success slots remain distinct. Multi-bind reads every incoming value before changing existing destinations.
5. **Limits and nested execution:** Long expressions, nested calls, loops, large values and template output remain bounded and produce the correct source or infrastructure failure lane.

## Accepted design and language scope

### Accepted implementation

1. Shared value-only evaluator API over the existing `FoldContext`.
2. Measured reduction of redundant const-interpreter expression rewriting, with specialised fallback for operations requiring execution state.
3. Match statements using the full existing resolved pattern vocabulary, including guards, captures and defaults.
4. Multiple success results from const functions and multi-bind in their bodies, using Moth's native ordered result slots.
5. Immutable non-generic const receiver methods on eligible same-file nominal struct and choice receivers, with existing receiver visibility and call syntax.
6. Investigation of stepped collection loops. Implement only if the current collection frame can preserve the complete runtime contract without a disproportionate second execution mechanism.
7. Realistic package-style integration and performance coverage for repeated HTML/CSS helper computation.

### Held outside this implementation

- Generic const functions and generic const receiver methods.
- Mutable-reference const parameters and mutable receiver methods.
- Fallible success/error return contracts, Never const functions and zero-success-result const functions.
- Recursive const-function call graphs and recursive declaration dependencies.
- General const propagation through runtime locals, arbitrary const-evaluation caching and whole-program partial evaluation.
- Broader speculative release eligibility, including multi-result ordinary functions.

These items do **not** all have accepted future semantics. Generic const support and extended release eligibility need a dedicated design and performance case. Mutable parameters, error slots, Never, zero results and recursion remain open semantic questions until explicitly accepted. Neither this plan nor a progress-matrix entry promises their eventual inclusion.

### Permanently enforced compile-time boundary

Required const execution cannot depend on runtime values, IO or mutable application/module state, arbitrary untrusted foreign effects, runtime-only templates, macro-like AST transformation, physical target inspection or final builder-dependent URL text. Trusted registered Core constant operations remain permitted under their existing contracts. Structural resource-bearing `String` values remain valid constants without guessing their final characters.

Keep the ordinary language's exclusions, such as tuple values, macros and operator overloading. Do not invent const-only language forms to work around an unsupported operation.

### Scope distinctions

- Match **inside const-function execution** does not introduce general static match specialisation of ordinary runtime code.
- Allowing multi-result **required** const calls does not widen release-only ordinary-call candidacy.
- Immutable const methods do not enable methods on anonymous data-only const records or bypass same-file receiver ownership.
- Const evaluation remains eager for already-known pure subexpressions but never eagerly executes unreachable work.
- A declaration with no calls still receives its normal static eligibility/type checks. Its parameterised body need not execute.

## Current repository map and architectural evidence

Source ownership and likely changes:

- `src/compiler_frontend/ast/expressions/eval_expression/evaluator.rs`: Eagerly folds completed typed operands/operators and resolves required const calls at expression completion. Preserve construction timing and re-entry folding.
- `src/compiler_frontend/ast/expressions/expression_store.rs` and `expression_store_view.rs`: Compact typed rows and borrowed views indexed by `ExprId`. Retain current representation.
- `src/compiler_frontend/ast/expressions/expression_rewrite.rs` and `expression_scratch.rs`: Rebuild changed expression edges, reuse unchanged rows. Delete only redundant execution-time uses.
- `src/compiler_frontend/ast/const_eval/mod.rs`: `FoldContext::fold_walk` directly reads typed expression views, uses local `PathId` lookup and stages values. Separate value result from replacement commit.
- `src/compiler_frontend/ast/const_values/store.rs`: Canonical value store and transactional `stage_values`, including multiple roots. Reuse, preserve failure rollback.
- `src/compiler_frontend/ast/const_values/const_execution/values.rs`: Interpreter `fold_to_constant` currently asks folder for an expression then reads its `ConstValueId`. Consume value-only result.
- `src/compiler_frontend/ast/const_values/const_execution/rewrite.rs`: Required/release call rewriting, nested call execution, lazy logical completion, catch provenance. Retain necessary lazy/effectful paths, eliminate redundant rebuilds.
- `src/compiler_frontend/ast/const_values/const_execution/execute.rs`: Prepared instruction frames, bindings, loops, returns, template adapter. Use direct value evaluation where safe, add statement semantics.
- `src/compiler_frontend/ast/const_values/const_execution/instructions.rs`: Prepares one executable instruction vocabulary. Replace explicit match/multi-bind/stepped-loop refusals as phases land.
- `src/compiler_frontend/ast/const_values/const_execution/eligibility.rs`: Const-body operation eligibility and separately conservative release proof. Add justified method support without broadening release proofs.
- `src/compiler_frontend/ast/const_values/const_execution/places.rs` and `state.rs`: Projections, aliases, mutation, frame and handler state. Reuse local state and capture/update semantics.
- `src/compiler_frontend/ast/const_values/const_execution/budget.rs`: Bounded execution and allocation charges. Preserve or strengthen cost coverage.
- `src/compiler_frontend/ast/statements/value_production/expression_build.rs`: `commit_constant_result_block` for existing multi-result trusted host folds. Reuse for const-function result slots.
- `src/compiler_frontend/ast/statements/match_patterns/`: Resolved match arms, literals, Option/choice captures and guards. Consume typed pattern facts, do not reparse.
- `src/compiler_frontend/ast/field_access/receiver_calls/`: Source receiver resolution, ownership, visibility and call shape. Extend the existing method dispatch path.
- `src/compiler_frontend/ast/templates/tir/`: Authoritative template composition and constant folding. Remain sole template owner.
- `src/compiler_frontend/ast/module_ast/finalization/finalizer.rs`: Release speculation after static selection, before freeze/HIR. Keep existing boundary and conservative refusal.

The current value walk already provides most of the proposed shared foundation. `fold_root` wraps that work with AST metadata handling and `commit_replacement`. Interpreter `evaluate_expression_inner` often rewrites an expression, folds it and extracts a value from the replacement. This repeated representation work is the first removal candidate.

The current instruction compiler explicitly rejects `NodeKind::Match` and `NodeKind::MultiBind` and rejects collection loops with a step. `ConstFunctionExecutable::return_slots` and `execute_body` already use vectors. Single-result restrictions remain at const declaration preparation and required-call rewriting. `commit_constant_result_block` already supports the ordered multi-result replacement shape.

## Phase and pause protocol

Each phase below is a stable stop point. Execute its tasks in order, but choose the smallest coherent independently testable implementation slices within them.

**At phase start:**
1. Read current `AGENTS.md` and routed canonical references. Inspect changed areas on the current branch.
2. Locate current source/test owners with `rg` and inspect new code since this plan was written.
3. Write a short phase design note covering candidate approaches, chosen ownership, invariants, exact tests and removal targets before editing.
4. If current authoritative design contradicts the proposed change, resolve the conflict with the user before implementation. Never silently let old code settle a semantic decision.

**For each implementation task:**
1. Extend an existing primary test or add one distinct failing regression.
2. Verify failure for the intended reason.
3. Implement the smallest complete change.
4. Run affected tests and compiler checks.
5. Remove obsolete helpers, temporary adapters and stale comments.
6. Review correctness, style and simplification before committing.

**At phase close:**
1. Review all changed source and tests against the style guide and code-review guide. Look for duplicates, allocations, stale code and inadequate WHY comments.
2. Verify source diagnostics, budget refusal and cross-module cases relevant to that phase.
3. Run targeted tests for intermediate commits, `just validate` at important phase boundaries and `just validate-full` only for final merge readiness, per `validation.mtf`.
4. Update canonical documentation and the progress matrix only for actual contract/status changes.
5. Update this plan's small status block and record the exact next action. Keep evidence, benchmark details and commits in the phase handover or Git history.
6. Pause, switch branches or integrate the completed phase as ordinary repository policy permits.

**At plan completion:** Migrate durable decisions into canonical documentation, remove the roadmap entry and delete this short-lived plan in the completing commit. Follow `docs/roadmap/roadmap.md`. Do not leave a committed "complete" plan indefinitely.


---

## Phase 0: Confirm the current baseline and establish evidence

**Purpose:** Measure the cost of const execution and confirm the smallest correct refactor before changing expression evaluation. This plan and its roadmap entry already exist when Phase 0 starts.

**Files:** `const_eval/mod.rs`, `const_execution/{execute,rewrite,values,budget}.rs`, `module_ast/finalization/expression_census.rs`, `compiler_frontend/instrumentation/`, the benchmark owners and existing `tests/cases/const_function_*`.

### Task 0.1: Trace and classify actual work

- [ ] Map the execution path for eager ordinary folding, required call rewriting, constant declaration resolution, imported const functions, interpreted statements, TIR folding and release speculation. Annotate when each path creates an `ExprId`, stages a `ConstValueId`, walks a subtree and charges work.
- [ ] Confirm `FoldContext::fold_walk` reads an immutable expression prefix using a lookup callback. Confirm `fold_root` stages values and optionally commits `ExpressionOp::Constant`, including multi-result host handling.
- [ ] Trace `ConstInterpreter::evaluate_expression_inner`, `RewriteWalk::complete`, `fold_to_constant`, `rewrite_expression_children` and the specialised projection/catch/template paths. Identify actual duplicate traversals and allocations, not merely similar names.
- [ ] Inspect existing expression census and counters before adding any new counter. Record which work is append-only and which expression rows remain reachable after finalisation.
- [ ] Document which operations cannot bypass rewriting without violating lazy evaluation, handler ownership, aliasing or call semantics.

### Task 0.2: Record reproducible performance baselines

- [ ] Build representative small cases: scalar arithmetic, repeated pure expressions in a loop, deep nested const calls, composite construction, mutable local collections, catches, templates, imports and structural strings.
- [ ] Add a small, realistic `@core/html`/`@core/css`-style benchmark using a local fixture. Benchmark repeated helper invocation and template/string composition without modifying public package APIs.
- [ ] Capture per-case time, expression rows appended/rebuilt, value-store allocation and work-budget consumption using existing instrumentation. Measure peak memory only if a practical profiler provides defensible evidence.
- [ ] Run the repository's non-recording benchmark checks where applicable. Record the toolchain, command, input and variance. Raw runs remain local.
- [ ] Establish a minimal evidence-backed success criterion: fewer redundant expression rows or traversals, lower time/memory on representative cases, or a demonstrable reduction in owned machinery with no meaningful regression.

### Task 0.3: Verify the baseline tests

Inspect existing owners before adding fixtures:

- `const_function_constant_call_success`
- `const_function_logical_short_circuit_success`
- `const_function_rebuilt_catch_success`
- `const_function_collection_operations_success`
- `const_function_nested_template_call_success`
- `const_function_imported_success`
- `const_function_structural_text_refusal_rejected`
- `const_function_execution_limit_rejected`
- `const_function_memory_limit_rejected`
- `const_function_release_folding_success`

Use the filtered integration runner, for example:

```bash
cargo run --quiet -- tests --case const_function_constant_call_success --backend html
cargo test -p moth --lib const_eval
```

Check that each filter selects real tests. Do not use a passing zero-test run as evidence.

**Phase 0 exit:** A short architecture map, baseline results, existing test inventory and precise Phase 1 API/replacement decision. No speculative new IR or evaluator has been introduced.

**Review:** Confirm the recorded costs belong to const execution rather than ordinary expression construction, TIR or an unrelated frontend stage.

## Phase 1: Extract value-only evaluation from AST replacement

**Purpose:** Make the existing shared fold walker return a constant value without manufacturing an expression row when the consumer only needs a value.

**Files:** `src/compiler_frontend/ast/const_eval/mod.rs`, `const_values/store.rs`, `const_execution/values.rs` and existing `const_eval/tests/` plus relevant integration cases.

### Task 1.1: Fix the result and ownership contract before code changes

- [ ] Split current `fold_root` responsibilities on paper: read root/metadata, stage candidate values, perform `fold_walk`, retain refusal and provenance, select host multi-slot handling, and commit an AST replacement.
- [ ] Choose the minimum internal result type or existing outcome reuse that distinguishes `Value(ConstValueId)`, `NotConstant` and `Refused(reason)`, with infrastructure failures in the existing error lane. These are conceptual variants, not mandatory new names.
- [ ] Define an API that reads the builder immutably, accepts the current `FoldBoundary` and local binding lookup and returns the staged value without touching expression rows. Keep the existing transaction scope and `ConstValueCandidate`.
- [ ] Retain separate treatment of multi-slot trusted host calls when they require `commit_constant_result_block`. Do not force those results into a scalar value type or duplicate `SuccessShape`.
- [ ] Review `Dec` exponent context, occurrence provenance, catch eligibility and the runtime/content boundary before choosing final Rust signatures.

### Task 1.2: Prove the new boundary with focused tests

- [ ] Add a const-eval unit test where value-only evaluation reads a local binding and leaves the original `ExprId` and expression-row count unchanged. The computed value must match ordinary folding.
- [ ] Add refusal/error tests that assert candidate values do not publish a partial reachable result and the original AST remains untouched.
- [ ] Add a test distinguishing a `Runtime` fold refusal for a composite operation from legal `Content` composite evaluation. Runtime folding must not silently become more powerful.
- [ ] Verify structural resource provenance, exact fixed-width numeric/`Dec` results and the original required versus opportunistic failure policy.

### Task 1.3: Implement the smallest extraction

- [ ] Extract the value-computation portion of `fold_root`. Keep one actual `fold_walk` and one semantic implementation for numeric, cast, logical, comparison and trusted Core operations.
- [ ] Make `constant_fold`, `fold_expression_with_constants` and related expression-oriented callers commit replacements only when they need a rewritten `ExprId`.
- [ ] Change `ConstInterpreter::fold_to_constant` to consume the value directly instead of constructing `ExpressionOp::Constant` and reading the new row back.
- [ ] Preserve per-call and aggregate work/memory accounting. Check that removing rewriting does not accidentally avoid previously charged work. Do not charge speculative discovery as required execution unless the existing budget contract calls for it.
- [ ] Remove the superseded result-extraction path and any now-unused metadata/clones. Do not leave compatibility wrappers around old signatures.

### Task 1.4: Validate and inspect the result

- [ ] Run focused unit tests, existing const-function integration cases and release-refusal cases.
- [ ] Compare expression-row growth and elapsed time with Phase 0 for pure repeated expressions and composite values.
- [ ] Audit new borrowing/lifetime constraints. The evaluator reads expression rows while candidate values stage, so no mutable builder borrow should leak into the walk.
- [ ] Review whether `const_eval/mod.rs` should split by real responsibilities. A split is justified only if it removes mixed ownership or materially improves the current code.

**Phase 1 exit:** Value-only const evaluation operates over the existing typed rows and value store. Const execution no longer commits an AST constant merely to recover its value ID. Ordinary folding still commits the correct replacement when required.

**Review:** Check transaction rollback, `Dec` context, provenance and runtime/content boundary parity. Phase 1 can pause here independently.

## Phase 2: Reduce repeated rewriting in const-function execution

**Purpose:** Let the interpreter evaluate pure typed expressions with bindings directly while retaining specialised handling for complex execution semantics.

**Files:** `const_execution/{execute,rewrite,eligibility,places,values,state,budget}.rs`, `const_eval/mod.rs` and expression-rewrite owners only where obsolete calls can be removed.

### Task 2.1: Choose the safest direct path

- [ ] Classify existing expression operations into pure value evaluation and interpreter-owned operations. Start with references, scalar arithmetic, comparisons, casts and supported composites.
- [ ] Preserve specialisation for nested user const calls, mutable collection/place writes, TIR, catch/handler propagation and lazy logical control flow until each can be proven equivalent.
- [ ] Determine whether the current `FoldContext` nonconstant return can trigger the established fallback without emitting a premature semantic error. If not, introduce the smallest operation-level gate. Avoid a costly new whole-tree eligibility scan on every expression.
- [ ] Keep `RewriteWalk` where program-expression rewriting is genuinely required. Do not build another general statement interpreter inside `FoldContext`.

### Task 2.2: Protect lazy and effectful cases before changing dispatch

- [ ] Write a regression with `false and rhs` and `true or rhs` where the inactive operand would fail if executed.
- [ ] Cover nested const calls, selected versus unselected catches, early return from handlers and a projection of a mutable local composite value.
- [ ] Verify the earlier source type/eligibility checks still run where required, independent of whether runtime execution visits a branch.

### Task 2.3: Implement and delete redundant work

- [ ] Insert the value-only path at the narrowest correct point in `evaluate_expression_inner`. Feed it existing frame bindings and retain the original `ExprId`.
- [ ] Prefer direct `ConstValueId` projection using existing `project_constant_value` when a simple projection needs no expression reconstruction.
- [ ] Rebuild expression children only for a case that needs altered semantic edges, call substitution, catch authority or program-level AST replacement.
- [ ] Remove now-dead substitution/rebuild helpers and state once their users have migrated. Retain source-provenance transfer and catch-body identity where replacements remain necessary.
- [ ] Audit evaluation-order and budget differences on every newly direct operation.

### Task 2.4: Decide whether further architectural work pays for itself

- [ ] Compare Phase 0 and Phase 1/2 time, allocations, row counts and peak memory with matched inputs.
- [ ] Check that a fallback after direct evaluation has not introduced a second expensive traversal for common unsupported expressions.
- [ ] If simple direct execution produces the useful win, stop. If a measured hotspot remains, record the exact operation/representation responsible before proposing a compact instruction or additional specialisation.
- [ ] Reject unmeasured global memoisation, generic value caches, a replacement AST/IR, a second expression callback framework and general bytecode at this stage.

**Phase 2 exit:** Direct evaluation covers demonstrably useful pure operations without semantic drift or a competing evaluator. Measured costs and a clear stop decision are recorded.

**Review:** Compare the final responsibilities of `FoldContext`, `ConstInterpreter` and `RewriteWalk`. Remove duplicated decisions and ensure each remaining layer owns a real invariant.


---

## Phase 3: Support match statements in const functions

**Purpose:** Execute Moth's existing typed match semantics rather than a reduced const-only pattern language.

**Files:** `src/compiler_frontend/ast/ast_nodes.rs` (read-only shape), `src/compiler_frontend/ast/statements/match_patterns/` (canonical pattern facts), `const_execution/{instructions,execute,state,eligibility}.rs` and `src/compiler_frontend/hir/hir_statement/{control_flow,match_captures}.rs` (read semantics, no HIR dependency).

### Task 3.1: Discover the exact matching contract

- [ ] Read the canonical match/pattern documentation and inspect `NodeKind::Match { scrutinee, arms, default, exhaustiveness }` and `MatchArm { pattern, guard, body }`.
- [ ] Enumerate the existing resolved `MatchPattern` forms: literal, option absent/value/present capture, relational and choice variant with payload captures. Inspect `PatternValueKind::Runtime` and `Constant`, choice tag identity, typed comparison and capture placement.
- [ ] Trace HIR match lowering solely to establish the existing observable ordering. Do not call HIR helpers from Stage 4 or recompute exhaustiveness from source syntax.
- [ ] Design one prepared match instruction that retains resolved facts and compiles each arm/default body once. Prefer shallow existing AST-owned pattern storage or compact prepared arm entries over cloning whole statements per invocation.

### Task 3.2: Implement arm selection through the existing frames

- [ ] Before implementation, add a selected-arm diagnostic regression: a failing unselected guard/body must not execute, but authored branches must still type-check.
- [ ] Replace the explicit `NodeKind::Match` preparation rejection with a prepared instruction and execute it through `ConstInterpreter`.
- [ ] Evaluate the scrutinee once. Check patterns in authored order, evaluate a guard only after its pattern succeeds and execute the first arm whose pattern and guard both succeed.
- [ ] Publish captures into the selected arm's lexical binding scope before evaluating its guard. Ensure captures cannot leak to the next arm, the default or the enclosing function.
- [ ] Retain ordinary `return`, `break` and `continue` unwinding and the existing work budget. A guard that rejects one arm must not leave a pending handler or captured binding.
- [ ] Consume the AST's proven exhaustiveness/default facts without adding a second exhaustiveness checker.

### Task 3.3: Complete coverage and diagnostics

- [ ] Extend existing match/const tests for literals, relational patterns, exhaustive choices, choice payload capture, Option none/value/present capture, guards, default and nested matches.
- [ ] Cover a runtime-valued pattern operand that becomes known through const local bindings, respecting its authored evaluation position.
- [ ] Cover match inside a range or collection loop, early return, a nested const call, a selected failure and a failure in an inactive arm.
- [ ] Check imported nominal choice identities and structural-text comparison refusal where the current evaluator cannot inspect final URL characters.
- [ ] Preserve typed diagnostic codes/reasons and authored spans. Avoid generic "unsupported const expression" errors where a supported match contains a more precise failure.

**Phase 3 exit:** All supported ordinary statement pattern forms execute through the prepared const instruction machine with correct capture scope and guard order. No new pattern parser, matcher registry or general static match optimisation exists.

**Docs:** Update `docs/docs/functions/function-declarations.mtf` and the Const functions progress-matrix row for actual delivered match support. The existing canonical match contract remains authoritative.

**Review:** Independently inspect guard laziness, capture scope restoration, transitive failures and costs in nested matches. This phase may pause.

## Phase 4: Support native multi-return and multi-bind

**Purpose:** Admit multiple ordered success slots in required const calls and const bodies using the existing result-slot machinery.

**Files:** `const_execution/{mod,instructions,execute,rewrite,places,eligibility}.rs`, `src/compiler_frontend/ast/statements/value_production/expression_build.rs`, `expression_store.rs` and `const_values/store.rs`.

### Task 4.1: Locate every genuine single-slot restriction

- [ ] Inspect `ConstFunctionExecutable::return_slots` and `execute_body() -> Vec<ConstValueId>`.
- [ ] Find `executable_for_const_declaration`'s one-result check, `ConstCallSite`, `complete_call`'s `returns.len() == 1` check and every `success_shape().single()` in the const call path.
- [ ] Separate deliberate single-value *consumers* from the arbitrary V1 restriction on *producers*. Do not widen operands that require exactly one value.
- [ ] Inspect the existing multi-result trusted host fold and `commit_constant_result_block`. Reuse its `ValueBlock::LexicalScope` and ordered `ThenValue` slot mechanism rather than building tuples or a new carrier.
- [ ] Confirm the complete `SuccessShape`, call provenance and source/import publication contracts survive the replacement. Check cross-module ID rules.

### Task 4.2: Implement multi-slot required call replacement

- [ ] Extend const declaration preparation to accept two or more ordinary immutable success slots, retaining existing fallible/Never/zero restrictions.
- [ ] Make the const call-site and result validation represent the full resolved success shape rather than a single `TypeId`.
- [ ] Validate the executed slot count and types against the prepared executable and call shape before committing any replacement.
- [ ] Stage and publish all result values transactionally through existing value-store operations. Commit the ordered result block using `commit_constant_result_block`.
- [ ] Preserve per-slot provenance and the call's authored metadata. Do not merge result values into one synthetic aggregate.
- [ ] Keep scalar-only release eligibility unchanged, including its benign refusal behaviour for non-scalar/multi-result candidates.

### Task 4.3: Implement multi-bind inside const bodies

- [ ] Inspect the existing `NodeKind::MultiBind { targets, value }` and `MultiBindTarget` representation. Confirm declaration versus update targets, aliases and recorded `ValueMode`.
- [ ] Add one prepared multi-bind instruction that retains the already-typed producer and target facts.
- [ ] Evaluate the producer exactly once, obtain every result and preserve their order. Collect all RHS values before any existing target is updated.
- [ ] Use existing `FrameBindings`, place projection and alias-write paths for declarations and assignments. Validate target shape and avoid partially installing new bindings on a failed result producer.
- [ ] Preserve ordinary left-to-right target update semantics where aliases overlap. Do not introduce a new binding store, runtime tuple or mutable-parameter capability.

### Task 4.4: Preserve returns and receivers

- [ ] Inspect `ConstInstructionKind::Return(Vec<ExprId>)` and the ordinary return-production owner for any assumption that one expression equals one result slot.
- [ ] Support returning multiple scalar/composite values, nested multi-result producers and value-producing blocks permitted by current AST receiving rules.
- [ ] Preserve the distinction between a multi-result producer and a single-value operand. Ensure a single-target binding rejects a multi-result call with the existing arity diagnostic.

### Task 4.5: Assert observable parity

Start with `const_multiple_result_slots_rejected` and `adversarial_multi_bind_in_loops`. Replace a rejection fixture only when its formerly invalid behaviour becomes supported. Keep `const_zero_result_function_rejected` as a separate current boundary.

- [ ] Add a multi-result const helper with two and three distinct slots and assert exact output.
- [ ] Cover a nested multi-result const call and cross-file import/publication.
- [ ] Cover new declarations, existing mutable updates, mixed targets, alias-sensitive updates and evaluated-once side effects on local state.
- [ ] Assert incorrect slot count/type diagnostics at receiving sites and no silent first-slot truncation.
- [ ] Check that a released ordinary multi-return candidate remains a runtime call while a required const multi-return executes.

**Phase 4 exit:** Required const functions support multiple ordered ordinary success slots and const bodies support multi-bind. The existing native result block remains the only multi-result replacement representation. Zero-result, fallible, Never and generic const signatures remain outside scope.

**Docs:** Update `function-declarations.mtf`, `returns-and-multiple-values.mtf` only where the const-specific contract warrants clarification and the progress matrix. Leave release eligibility wording accurate.

**Review:** Audit every residual `single()` assumption in affected producers, return forwarding, aliases and cross-module slot validation. This phase may pause.

## Phase 5: Support immutable const receiver methods

**Purpose:** Allow the existing const marker on eligible immutable source receiver methods and evaluate receiver calls through the one prepared executable/frame machine.

**Files:** `src/compiler_frontend/ast/field_access/receiver_calls/{mod,source_methods,shared}.rs`, `receiver_access.rs`, `src/compiler_frontend/ast/expressions/{function_calls,expression_store}.rs`, the function declaration/header and public-interface owners, and `const_execution/{mod,eligibility,rewrite,execute}.rs`.

### Task 5.1: Trace receiver identity before choosing an API

- [ ] Inspect method declaration and call parsing. Confirm how the current `name #|...|` modifier intersects with `this T` receiver declarations, including declaration-time restrictions.
- [ ] Trace `ExpressionOperationView::MethodCall`, resolved callable target, receiver argument position, metadata, method visibility and success shape. Locate the unconditional receiver-call refusal in `classify_const_expression`.
- [ ] Inspect how nominal type publication carries same-file methods to consumers. Identify any missing const-evaluation summary in an existing callable record before introducing a new record.
- [ ] Decide whether the method can be prepared as an ordinary `ConstFunctionExecutable` with its receiver bound as the first parameter. Prefer this shared route.

### Task 5.2: Accept the narrow receiver contract

- [ ] Allow only a shared immutable `this T` receiver on an eligible same-file source-authored nominal struct or choice.
- [ ] Require compile-time-known receiver and arguments. Use normal receiver-call syntax and the existing const marker. Keep ordinary receiver methods unaffected.
- [ ] Reject `this ~T`, generic const methods, foreign/opaque/builtin receiver extension and cross-file receiver ownership.
- [ ] Preserve nominal type identity, method visibility and the data-only const-record restriction. Do not turn anonymous compile-time records into runtime method-capable values.
- [ ] Produce a source diagnostic at the authored declaration or call boundary, as appropriate. A never-called invalid const method must receive the established declaration-time eligibility check.

### Task 5.3: Execute through the existing callable path

- [ ] Extend the prepared callable lookup and required rewrite to recognise a const method's resolved target.
- [ ] Route the receiver value through the existing parameter/binding machinery, preserving receiver-before-arguments evaluation order and provenance.
- [ ] Reuse `ConstInterpreter::execute_body` and existing budgets. Do not build separate method frames, method registries or a method-specific value representation.
- [ ] Reuse Phase 4 result-slot completion for a const method returning multiple values.
- [ ] Confirm imported nominal methods execute from stable semantic interface facts without donor-local `ExprId` or `TypeId` leakage.
- [ ] Keep release ordinary-method eligibility unchanged unless an existing conservative proof already admits it.

### Task 5.4: Validate method semantics

- [ ] Add or extend cases for nominal struct and choice receivers, explicit parameters, nested const calls, composite returns and multiple returns.
- [ ] Cover imported receiver types, method visibility, runtime receiver refusal, mutable receiver rejection, generic rejection and invalid ownership.
- [ ] Cover template/string values returned by a const receiver method without making runtime templates legal.
- [ ] Check receiver and argument evaluation order, failures and source provenance.

**Phase 5 exit:** Eligible immutable const methods use the same prepared executable and value evaluator as free const functions. No parallel dispatch or execution representation exists.

**Docs:** Update `docs/docs/structs/receiver-methods.mtf`, `function-declarations.mtf` and the progress matrix.

**Review:** Audit callable publication, nominal identity, receiver mutability and redundant call-target classification. This phase may pause.


---

## Phase 6: Investigate stepped collection loops

**Purpose:** Close the existing const-only stepped-loop gap if the current collection frame can implement the complete runtime contract without another execution model.

**Files:** `src/compiler_frontend/ast/ast_nodes.rs`, `src/compiler_frontend/ast/const_values/const_execution/{instructions,execute,state,eligibility,budget}.rs`, `src/compiler_frontend/hir/hir_statement/loop_lowering.rs` (reference only), `docs/docs/loops/collection-loops.mtf`.

### Task 6.1: Make the acceptance decision from concrete code

- [ ] Inspect `NodeKind::CollectionLoop { iterable, step, bindings, body }`, the current explicit `step.is_some()` rejection and `ExecFrame::Collection`.
- [ ] Compare the current ordinary loop cursor and collection-length capture to the runtime contract. Check whether the existing frame can retain signed stride, cursor, length and per-iteration bindings without another engine.
- [ ] Prepare a concise estimate of added code and state, failure paths and unavoidable duplication. Prefer implementing in the current frame rather than copying HIR loop lowering.
- [ ] If full parity requires broad new machinery, defer. Document the exact blocker and re-entry criterion without adding partial semantics or leaving an unused scaffolding variant.

### Task 6.2: Implement only if the gate passes

The runtime contract must remain exact:

- Evaluate source once and capture its length.
- Evaluate step once afterward, including for an empty source.
- Default to signed `I32` +1.
- Accept existing signed fixed-width step types only.
- Reject zero before the empty-source shortcut, preserving the runtime `InvalidLoopStep` contract and source diagnostic policy.
- Start at zero for a positive step, at the last source index for a negative step.
- Bind the actual source index, not the ordinal visit count.
- Advance on `continue` and terminate on `break`.
- Stop before overshoot or invalid index construction.
- Preserve finite work limits and safe signed/unsigned index conversions.

- [ ] Extend the existing collection instruction/frame with only the state these rules need.
- [ ] Add positive, negative, omitted, overshoot, empty, dynamic-zero, boundary, nested, `break`/`continue` and index-binding parity cases.
- [ ] Preserve the ordinary runtime policy for collection mutation and captured length.
- [ ] Remove the explicit stepped-loop rejection only when all parity tests pass.

**Phase 6 exit:** Either correct stepped-loop execution with meaningful tests, or an evidence-backed deferral. A partial independent stepping contract is unacceptable.

**Docs:** If implemented, update the Const functions matrix row. If deferred, leave the matrix's implementation gap accurate and record the reason and re-entry condition in this plan while active, then migrate the durable deferred item into the roadmap/design-scope reference on plan retirement.

**Review:** Focus on negative-step overflow, zero-before-empty order, actual index bindings and frame allocation. This phase may pause.

## Phase 7: Package workloads, architecture audit and closure

**Purpose:** Verify that const functions help realistic package authors while leaving the compiler simpler and no less correct.

**Files:** `packages/core/html/` and `packages/core/css/` as workload references, targeted benchmark fixtures, relevant `tests/cases/`, `const_execution/`, `const_eval/`, TIR owner and canonical documentation. Do not modify first-party package APIs without approval from the separate package programme.

### Task 7.1: Exercise real helper shapes

- [ ] Construct representative nested const helpers that assemble safe HTML/CSS fragments, use match for selection, use immutable methods and consume/produce multiple results.
- [ ] Exercise imported package-style helpers, repeat calls, collections, structural resource-bearing strings and nested templates.
- [ ] Assert rendered output and asset behaviour, not only successful compilation.
- [ ] Verify runtime-only template content remains runtime or correctly rejected in required constant contexts.
- [ ] Use the existing `fold_prepared_template` path and exact TIR view identity. Do not introduce a template evaluator or speculative cache.

### Task 7.2: Measure and explain the final architecture

- [ ] Repeat the Phase 0 cases with matched environment/toolchain and inspect the same counters.
- [ ] Record compile time, append/rewrite counts, retained value allocation, peak memory where measured and effects on non-const ordinary source.
- [ ] Attribute unexpected regressions to specific operations. Refactor only when the evidence identifies a real owner or responsibility problem.
- [ ] Do not use benchmark wins as a substitute for semantic parity.

### Task 7.3: Conduct a final simplification and correctness audit

Review all changed modules using `moth-code-review-guide.md`. Specifically search for:

- duplicate arithmetic, cast, pattern or projection evaluation
- unnecessary expression rebuilds, clones and allocations
- overlapping helper APIs or staged compatibility wrappers
- broad state bags, long parameter lists or unexplained lint suppressions
- duplicate const versus release eligibility checks with unclear ownership
- dead enum variants and unsupported branches superseded by new semantics
- fresh values losing source/config/resource provenance
- stale WHAT/WHY comments and inaccurate `mod.rs` architecture maps
- redundant or weak tests, especially acceptance-only tests hiding wrong output
- stage leaks, global registries and reintroduced HIR/TIR authority in the executor

- [ ] Review direct-value execution against the original provenance/handler/budget tests.
- [ ] Review match guards/captures and multi-result aliases for control-flow leaks.
- [ ] Review immutable method publication against cross-module identity rules.
- [ ] Run selected integration cases and Rust tests, then `just validate` at the integrated checkpoint.
- [ ] Run `just validate-full` on the final integrated tree before merge readiness. Report exact commands and results, never unrun gates.
- [ ] Delete temporary/obsolete code and update `index.md` only when significant files or responsibilities changed.

**Phase 7 exit:** The accepted semantics have landed or carry a user-approved explicit deferral, required/release boundaries remain correct, final benchmarks and validation are recorded and permanent design references contain every enduring decision.

## Documentation and status synchronisation

Documentation ownership follows repository `AGENTS.md` and `docs/roadmap/roadmap.md`. A roadmap plan is an implementation work item, not the permanent owner of language semantics.

### This PR, before implementation

- Add this plan as a queued item **immediately after Core Math fast counterparts** in `docs/roadmap/roadmap.md`.
- Do **not** update the current Const functions progress-matrix status. No compiler feature changes in this PR.
- Do **not** change canonical language behaviour by editing docs to imply the proposed features already compile.
- Keep the plan's queued status concise. Phase implementation may be paused independently.

### On completing a semantic phase

Update the canonical unsuffixed reference that owns the feature:

- Match: `docs/docs/functions/function-declarations.mtf` and the canonical pattern reference only if the contract needs a const-specific distinction.
- Multi-results: `function-declarations.mtf`, `returns-and-multiple-values.mtf` and the existing native result-slot architecture reference if a permanent boundary changed.
- Receiver methods: `docs/docs/structs/receiver-methods.mtf` and `function-declarations.mtf`.
- Stepped loops: `docs/docs/loops/collection-loops.mtf` only if a const-specific contract must be described, since the runtime stepping semantics already live there.
- Shared evaluation: `docs/compiler-design-overview.md` Stage 4 only if architecture ownership changes, with implementation details in Rust module documentation.

For each landed semantic feature, update the existing Const functions row in `docs/docs/progress/@entry.moth`. Keep its current `Partial` status until the described accepted coverage justifies another label. A pure refactor does not change implementation status.

### Deferred, open and outside scope

Use `docs/docs/design-scope/deferred-and-outside-scope.mtf` to distinguish:

1. **Accepted deferred implementation:** a feature with an approved exact contract whose compiler support remains incomplete. Include matrix coverage only when the accepted contract warrants it.
2. **Open design:** mutable const parameters, const error channels, const Never, zero-result const functions and recursion until a later design explicitly settles their meaning. Do not promise them in the progress matrix.
3. **Outside const design:** runtime effects, untrusted host execution, runtime-dependent arguments, macro-like AST rewriting and physical target dependence. Do not present these as future implementation work.

Generic const declarations and methods remain deferred **design investigation**, not an implementation commitment. Any future acceptance needs an explicit instantiation, compilation-cost and cross-module contract. Release optimiser eligibility remains a separate future proposal.

If Phase 6 defers stepped loops, record the exact unmet parity rule or architecture cost and an evidence-based condition for revisiting it. Do not write "later" without the reason.

### Plan retirement

Once all accepted work has completed, migrate any unique permanent contracts from this plan into the canonical owners. Retain unresolved design decisions under the proper deferred/open reference, not as stale plan tasks. Remove this plan and its roadmap entry in the same completing commit. Git history preserves the completed work.

## Verification commands and test ownership

Use the exact current `testing.mtf` and `validation.mtf` routes when each phase starts. Typical narrow checks:

```bash
cargo check -p moth
cargo test -p moth --lib const_eval
cargo run --quiet -- tests --case const_function_constant_call_success --backend html
cargo run --quiet -- tests --case const_function_logical_short_circuit_success --backend html
cargo run --quiet -- tests --audit
```

The selected integration `--case` must exist and run. Test broad semantic interactions under `tests/cases/`, private evaluator invariants under `src/compiler_frontend/ast/const_eval/tests/` and local const executor invariants in the owning Rust test module. Do not move tests into production files. Use exact diagnostic codes/reasons and meaningful outputs. Preserve distinct regression coverage when consolidating fixtures.

Important boundaries:

- Feature branch intermediate commits: targeted checks.
- Important phase integration checkpoints: `just validate` and affected feature lanes.
- Final branch merge readiness and any commit to `main`: `just validate-full` evidence for the resulting tree.
- Performance-sensitive phases: non-recording `just bench-check` and carefully matched measurements.

## Completion definition

Const functions execute against one typed AST/value foundation. Value-only computation no longer requires a temporary replacement expression. Necessary call/handler/template execution remains explicit and lazy. Match statements, multiple success slots, multi-bind and eligible immutable receiver methods work through established compiler owners. Stepped collection loops have either landed with full parity or a concrete deferred decision. Release speculation remains conservative, packages demonstrate useful const helpers and the codebase carries less redundant state and rewriting.