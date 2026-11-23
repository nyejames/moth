# Const Functions: Profile-Independent Semantics and Shared Evaluation Foundation Implementation Plan

> **For agentic workers:** Implement this plan one independently reviewable phase at a time. Follow repository `AGENTS.md`, the canonical code style and testing rules and the phase review gates below. Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` when available. A phase may be committed, integrated and paused before the next one starts.

**Goal:** Enforce profile-independent semantic compilation and validated HIR, remove optional early whole-function speculation, assess folding machinery for justified cost and necessity, then simplify and extend required const-function execution.

**Architecture:** Stage 4 owns required const execution, TIR, deterministic static selection and only semantics-preserving eager folding. Identical semantic inputs produce equivalent validated HIR and semantic summaries in debug and release; profile-dependent work belongs downstream. Remove ordinary-call release speculation rather than relocating it. Retain the compact typed `ExprId` graph, `ConstValueStore` and prepared interpreter, separating value computation from AST replacement where useful.

**Tech stack:** Rust, Moth compiler frontend, typed AST, TIR, existing integration runner and frontend benchmark instrumentation.

**Spec and permanent authorities:** `docs/compiler-design-overview.md` (Stage 4, typed expression ownership, result slots and const execution), `docs/docs/functions/function-declarations.mtf`, `docs/docs/functions/returns-and-multiple-values.mtf`, `docs/docs/structs/receiver-methods.mtf`, `docs/docs/constants/constant-folding.mtf`, `docs/docs/directives/constraints.mtf`, `docs/docs/loops/collection-loops.mtf`, the canonical pattern references and `docs/docs/design-scope/deferred-and-outside-scope.mtf`. The accepted decisions below supersede the old V1 restrictions only for the features explicitly approved here. Transfer enduring decisions into their canonical owners as implementation lands.

**Status:** Active - implementation ready to begin on this PR branch after review
**Current slice:** Foundation Phase A, profile-sensitive semantic path inventory and baseline
**Blockers:** None known
**Next action:** Execute Foundation Phase A on this branch after review. The three foundation gates must complete before the const-language feature phases or later roadmap work.

## Global constraints

- Respect the current repository `AGENTS.md`, `docs/developer-docs/style-guide/style-guide.mtf`, `testing.mtf` and `validation.mtf`.
- Stage 4 owns source typing, required constant evaluation, TIR and static selection. HIR owns executable control flow and downstream constraint, borrow and lifetime verification. Do not introduce HIR const execution, another parser, another type checker or another template engine.
- One authoritative operation/value meaning. Reuse resolved typed operations, numeric/cast/Core evaluators, `ConstValueStore` and TIR.
- Preserve zero, one and multiple ordered success slots without tuples. Preserve a distinct error channel and distinct Never contract.
- Preserve immutable source expression rows during evaluation. Create replacement expression rows only when a compiler consumer needs them.
- Preserve mandatory required const calls in all profiles. Remove release-only ordinary-call speculation, candidate evaluation and candidate notes/budgets; no replacement late optimiser in this plan.
- Preserve determinism, evaluation order, source provenance, finite work/memory limits, transactionality and cross-module semantic identities.
- Prefer small direct Rust changes and deletion over wrappers, registries, compatibility layers, caches or speculative generality.
- Preserve benchmark and test ownership. Benchmark fixtures never substitute for correctness tests.
- Do not edit generated documentation or benchmark history directly.
- This is the first sequenced roadmap plan. Complete Foundation Phases A-C before other queued compiler work; later feature phases may pause at validated phase boundaries.

## Review focus

Five failure classes must be covered by the phases below.

1. **Profile drift and lost evidence:** Debug/release agree on HIR, diagnostics, constraint summaries, guard CFG, checked argument work, native result order and R1 acceptance/rejection.
2. **Selected semantics:** Static selection is profile-independent; eager folding retains selected fast operations and deferred source-validation obligations even when executable work disappears.
3. **Evaluation order and laziness:** Short-circuit operands, catches, handlers, match guards and calls execute in source order only when selected; runtime calls cannot be erased before verification.
4. **Values and aliases:** Required const values retain exact types, config/resource provenance and atomic staging; multi-return and projected writes do not truncate, duplicate or partially commit work.
5. **Limits and cross-boundaries:** Work/memory budgets, imported functions, generated sidecars and backend execution preserve the same source verdict and allowed output.

## Accepted design and language scope

### Accepted implementation

1. A single profile-independent AST/HIR semantic pipeline, a permanent canonical debug/release HIR-equivalence test and an audit of all profile-dependent semantic paths.
2. Remove early optional ordinary-function speculation and all uniquely associated proof/work/notes machinery, preserving unrelated validation evidence.
3. Independently assess each eager/folded operation for mandatory semantic consumers, compile-time and code complexity cost and potential downstream backend optimisation; retain, simplify or remove based on evidence.
4. Shared value-only evaluator API over the existing `FoldContext`.
5. Measured reduction of redundant const-interpreter expression rewriting, with specialised fallback for operations requiring execution state.
6. Match statements using the full existing resolved pattern vocabulary, including guards, captures and defaults.
7. Multiple success results from const functions and multi-bind in their bodies, using Moth's native ordered result slots.
8. Immutable non-generic const receiver methods on eligible same-file nominal struct and choice receivers, with existing receiver visibility and call syntax.
9. Investigation of stepped collection loops. Implement only if the current collection frame can preserve the complete runtime contract without a disproportionate second execution mechanism.
10. Realistic package-style integration and performance coverage for repeated HTML/CSS helper computation.

### Held outside this implementation

- Generic const functions and generic const receiver methods.
- Mutable-reference const parameters and mutable receiver methods.
- Fallible success/error return contracts, Never const functions and zero-success-result const functions.
- Recursive const-function call graphs and recursive declaration dependencies.
- General const propagation through runtime locals, arbitrary const-evaluation caching and whole-program partial evaluation.
- Any optional ordinary-function folding, including scalar calls. This is a low-priority future post-semantic optimisation subject to evidence that backend optimisers are insufficient.

These items do **not** all have accepted future semantics. Generic const support and future post-semantic whole-function optimisation each need a dedicated design and performance case. Mutable parameters, error slots, Never, zero results and recursion remain open semantic questions until explicitly accepted. Neither this plan nor a progress-matrix entry promises their eventual inclusion.

### Permanently enforced compile-time boundary

Required const execution cannot depend on runtime values, IO or mutable application/module state, arbitrary untrusted foreign effects, runtime-only templates, macro-like AST transformation, physical target inspection or final builder-dependent URL text. Trusted registered Core constant operations remain permitted under their existing contracts. Structural resource-bearing `String` values remain valid constants without guessing their final characters.

Keep the ordinary language's exclusions, such as tuple values, macros and operator overloading. Do not invent const-only language forms to work around an unsupported operation.

### Scope distinctions

- Match **inside const-function execution** does not introduce general static match specialisation of ordinary runtime code.
- Allowing multi-result **required** const calls does not introduce optional ordinary-function evaluation.
- Immutable const methods do not enable methods on anonymous data-only const records or bypass same-file receiver ownership.
- Required constant evaluation remains eager where language semantics need values. Retained pure eager folding must be deterministic, justified and preserve selected-operation evidence; never evaluate unreachable work.
- Static branch selection uses only required const values and established deterministic eager expression folding, never optional whole-function evaluation.
- Explicit test compilation is a semantic input. Debug/release are optimisation and instrumentation profiles, not source semantics.
- A declaration with no calls still receives its normal static eligibility/type checks. Its parameterised body need not execute.

## Current repository map and architectural evidence

- **Profile propagation:** `src/build_system/create_project_modules/{mod,compilation}.rs`, `src/compiler_frontend/{mod,pipeline}.rs`, `module_compilation/{context,service}.rs`, `ast/module_ast/{build_context,scope_context,emission,environment,finalization}/` and `ast/generic_functions/materialisation/` currently carry `FrontendBuildProfile`. Only the release-call finalizer needs a semantic profile decision. Remove the propagation chain and its test-only defaults without altering build-owned `BuildProfile` for physical variants.
- **Premature rewrite:** `ast/module_ast/finalization/finalizer.rs::rewrite_release_calls` calls `ast/const_values/const_execution/rewrite.rs`. `eligibility.rs` prepares transitive ordinary candidates; `rewrite.rs` shares `RewriteWalk` and `ConstInterpreter` with required const calls. Release-specific state also appears in `const_execution/{state,mod,budget}.rs`, AST/HIR proof ledgers and diagnostic `ReleaseBudgetScope` rendering.
- **Shared evaluator:** `ast/expressions/eval_expression/evaluator.rs` owns eager typed-expression completion. `ast/const_eval/mod.rs::fold_root` stages values via `FoldContext::fold_walk` then commits `ExpressionOp::Constant` through the existing builder. `const_execution/values.rs::fold_to_constant` requests that replacement to read back a `ConstValueId`. Reuse `ast/const_values/store.rs::stage_values` and the current expression store, rewrite and scratch owners.
- **Required interpreter:** `ast/const_values/const_execution/{instructions,execute,rewrite,eligibility,places,state,budget}.rs` owns one prepared instruction/frame machine, value bindings, lazy handler execution and finite budgets. Keep `RewriteMode::RequiredExpression` versus `InterpretedBody` only if their different source/exec obligations remain necessary after deleting `Release`.
- **Evidence:** `ast/statements/control_flow_exits.rs`, `ast/expressions/failure_classification/`, `ast/mod.rs`, `hir/hir_builder/metadata.rs`, `hir/constraint_facts/` and `hir/constraint_verification.rs` own selected fast use, failure contributors, authored-dead validation and proof inputs. They are not all disposable release-fold compensation.
- **Tests:** `src/build_system/tests/compile_project_frontend_tests/mod.rs` can compile the same project under `BuildProfile::Dev`/`Release` and inspect `CompiledModuleArtifact.module.executable.hir` plus interfaces; `hir/hir_display.rs::HirDisplayContext::render_module` already renders the CFG and operations in tests. `src/compiler_frontend/tests/frontend_pipeline_tests.rs` owns stage-local AST/HIR invariants. Prefer these existing owners to a new serializer or test-only compiler entry point.
- **Later extensions:** `ast/statements/value_production/expression_build.rs::commit_constant_result_block` owns multi-slot replacement; `ast/statements/match_patterns/` owns patterns; `ast/field_access/receiver_calls/` and `ast/module_ast/environment/function_signatures.rs` own methods; TIR remains under `ast/templates/tir/`.

The current value walk already provides most of the proposed shared foundation. `fold_root` wraps that work with AST metadata handling and `commit_replacement`. Interpreter `evaluate_expression_inner` often rewrites an expression, folds it and extracts a value from the replacement. This repeated representation work is the first removal candidate.

The current instruction compiler explicitly rejects `NodeKind::Match` and `NodeKind::MultiBind` and rejects collection loops with a step. `ConstFunctionExecutable::return_slots` and `execute_body` already use vectors. Single-result restrictions remain at const declaration preparation and required-call rewriting. `commit_constant_result_block` already supports the ordered multi-result replacement shape.

## Semantic boundary for implementation

The authoritative invariant is `docs/compiler-design-overview.md > Architectural invariants`.
For the same source, dependencies, resolved configuration, selected compilation/test mode
and builder-provided semantic capabilities, debug and release must produce equivalent
validated semantic HIR, source diagnostics, public summaries and semantic fingerprints.
Different builder-provided capabilities or entries are different *semantic inputs*.
Build-owned target validation may still reject unsupported operations for a target,
without changing the compiled source meaning.

Stage 4 keeps required const execution, TIR, static selection and justified deterministic
pure expression folding. `$safe_math` observes selected fast operations after an
early fold; `$infallible` governs reachable runtime semantic failure, while invalid
required compile-time arithmetic still diagnoses. Optional whole-function execution
must never participate in static selection. Debug instrumentation before verification
must be observation-only; test compilation is an explicit semantic mode.
A future post-verification optimiser is deferred and must not change validated
source semantics or public summaries.

Remove optional release-only ordinary-call evaluation rather than building a second
HIR optimiser. Do not require every eligible expression to fold eagerly: measure
mandatory users, compilation/allocations, maintenance overhead and backend
alternatives before retaining, simplifying or deleting each mechanism.

## R1 regression inventory (preserved from the retired plan)

The original release-folding plan has been deleted. These are its concrete
regression cases and verdicts, not a request to rediscover the probes.
Each case uses the normal frontend/integration test grammar, a debug/release
pair and exact diagnostic code, reason and source labels. Helpers are
ordinary functions returning fixed values: `divisor() -> I32` returns
`2`, `numerator() -> I32` returns `8`, `identity(value I32) -> I32`
returns its argument, and `flag() -> Bool` returns `true`.

- **F1 - folded local:** Under `$infallible`, a `probe |value I32| -> I32`
  stores `d I32 = divisor()`, then evaluates `value // d` with a catch handler
  containing `assert(false)`. Reject `MOTH-RULE-0091` / `trap_assertion` with
  annotation and assertion labels.
- **F2 - direct operand:** `$infallible probe |value I32| -> I32` returns
  `value // divisor()`. Reject `escaping_error` with an annotation label
  and no secondary label.
- **F3 - local proof invalidation:** F1 instead declares `d I32 = 2`, then
  `n I32 = divisor()` and evaluates `n // d` under the failing catch.
  Reject `trap_assertion` with annotation and assertion labels.
- **F4 - inferred-lane `Invoke`:** `$infallible probe |value I32| -> I32, Error!`
  declares `d I32 = 2` and returns `numerator() // d`. Reject
  `escaping_error`, labelling the annotation and division.
- **F5 - multi-return ordering:** `$fast_math $infallible probe |value I32|
  -> I32, I32` declares `d I32 = 2` and returns
  `numerator(), value // d`. Reject `trap_division` with annotation and
  division labels.
- **F6 - wholesale folded argument:** `$fast_math $infallible probe ||
  -> I32` returns `identity(8 // divisor())`. Reject `trap_division`
  with annotation and division labels even when the outer call could fold.
- **F7 - cast-wrapped folded value:** F1 instead declares `d I32 = 2`
  followed by `n I64 = cast divisor()`, then handles `value // d` with
  `assert(false)`. Reject `trap_assertion` with both labels.
- **F8 - statement boundary:** A normal `probe |value I32| -> I32`
  declares `d I32 = 2` and attaches statement-level `$infallible`
  to the handled `numerator() // d` expression whose handler asserts
  false. Existing reject/reject control: `trap_assertion` with the
  statement annotation and assertion labels.
- **F9 - match guard:** The complete reproducer below must be accepted
  in both profiles with `x == 2`, without `MOTH-RULE-0088` /
  `side_effecting_prelude`.

For F9, use the ordinary statement-match spelling (not a `match` keyword):

```moth
flag || -> Bool:
    return true
;
identity_bool |value Bool| -> Bool:
    return value
;
x ~= 1
if x is:
    1 if identity_bool(flag() and flag()) => ~x = 2
    else => ~x = 3
;
~[: guard=[x]]
```

Keep the guard's authored match CFG rather than introducing a synthetic prelude.
The reverted proof-marker experiment caused F9's false rejection; F9 is
an acceptance guard, not a currently failing case. F8 was already rejected
in both profiles. F1-F7 are the confirmed debug/release divergences.

Preserve opposite-direction controls, so a repair cannot simply reject
every call: literal divisor `value // 2`, a pure wrapper around that
expression, a call after a proved division in statement/multi-return order,
a literal definition after a call and an already handled division followed
by a call must remain accepted where the current source semantics permit.
Keep `source_constraint_infallible_auxiliary_fold_stays_runtime_rejected`
as a source diagnostic control; after removing release speculation it no
longer asserts rollback of a candidate that cannot exist. Retain
`source_constraints_folded_call_fast_rejected` for transitive fast-use and
add a positive public-summary test that observes the transitive fast-use
fact without an enclosing `$safe_math` rejection.

Do not carry these cases as an implementation-only list: assert actual
source diagnostics, selected witnesses, final HIR where relevant and
runtime output for accepted cases. No span-equality marker or second HIR
lowering pass is needed.

## Foundation Phase A: Baseline, profile audit and folding-system assessment

**Purpose:** Establish evidence and exact ownership before modifying semantic code. This phase starts immediately and precedes every other queued compiler roadmap plan.

**Files:** `src/compiler_frontend/{ast,hir,analysis}/`, module compiler services and build-profile handoff, `tests/cases/source_constraint_*`, existing parity tests, frontend benchmark owners and `docs/bug-reports.md`.

- [ ] Read current authorities and run the R1 inventory above (F1-F9, opposite-direction acceptance and transitive fast-use controls). Record exact verdicts and diagnostic labels before modifying semantic code. A previously missing or already repaired case must be reported explicitly; do not invent an expected failure.
- [ ] Trace every `FrontendBuildProfile`, `BuildProfile` and backend/builder-sensitive path that can influence AST/HIR construction, generated instantiation, static selection, diagnostics, constraint summaries, public interfaces or reuse fingerprints. Distinguish explicit semantic inputs, prohibited profile choices, observation-only measurements and post-verification physical decisions.
- [ ] Map current eager operator folding, mandatory const calls, optional ordinary-call candidate evaluation, TIR, static selection and HIR proof/failure contributors, noting every `ExprId` rewrite, `ConstValueId` creation and required consumer.
- [ ] Audit each early folding subsystem against a retain/simplify/remove decision: mandatory semantic consumers, code/state complexity, frontend time and memory in both profiles, and equivalent later backend optimisation. Measure representative workloads. Preserve currently specified static selection and source diagnostics as a hard constraint: dropping an eager fold that changes those facts is a language change requiring separate approval, never an unreviewed performance refactor.
- [ ] Inspect the real project compilation fixtures in `src/build_system/tests/compile_project_frontend_tests/mod.rs`, which accept `BuildProfile` and expose each `CompiledModuleArtifact.module.executable.hir`. Start with `HirDisplayContext::render_module` and identify any missing semantic side-table/public-summary fields; use test-only comparisons, not a second HIR representation or production serializer.
- [ ] Recheck the roadmap plans affected by the profile and explicit test-mode contract against changes since this PR's initial survey. Its documentation-only review already covered the then-current queue; avoid repeating a blanket audit without new code evidence.

**Exit:** Profile-dependence map, reproduced regressions, folding-system decisions with measurements, exact deletion candidates and one reviewable HIR parity test design. No second evaluator.

## Foundation Phase B: Delete optional ordinary-call speculation

**Purpose:** Fix the confirmed semantic boundary violation by removing optional runtime-function evaluation before HIR, not by migrating it to a new HIR transaction pass.

**Files:** `ast/module_ast/finalization/finalizer.rs`, `ast/const_values/const_execution/{mod,rewrite,eligibility,execute,budget}.rs`, `ast/const_eval/mod.rs`, `ast/mod.rs`, HIR constraints/failure owners, public-interface and sidecar publication plus `tests/cases/`.

- [ ] Turn F1-F9 and the acceptance controls above into debug/release integration tests with exact reasons, source labels and runtime results where appropriate. Add a HIR-level assertion that a formerly folded ordinary call remains a call in both profiles while required const functions still fold at Stage 4.
- [ ] Remove the finalizer's `rewrite_release_calls` and `FrontendBuildProfile::Release` branch. Delete `eligibility.rs`'s `prepare_release_executable`, purity/dependency graph proofs, retained candidate types and `rewrite.rs`'s release-only target discovery, provenance, `RewriteMode::Release`, auxiliary speculation, `revert_refused_call`, `release_can_preserve_runtime` and whole-call dead-root exclusion. Remove `ExecutionState.release_erased_calls` and `ReleaseBudgetScope` diagnostic/renderer/tests that only exist for those notes. Retain `RewriteMode::RequiredExpression` and `InterpretedBody` while both have distinct consumers. Preserve `ConstValueStore::stage_values` rollback, required const budgets, general `FoldRequirement` decisions and runtime failure handling; do not delete those merely because they also supported release speculation. No dormant feature switch or late optimiser.
- [ ] Trace `AstReleaseErasedCallSite` through `ast/mod.rs` and `hir/hir_builder/metadata.rs`; remove its HIR live-erased-call projection. Audit `pre_fold_bool_conditions` and `PreFoldConditionRoots` in the finalizer, AST control-flow exits, HIR statement/control-flow lowering and expression freeze: delete the map and special freeze roots if direct finalised AST reads now supply the same facts. Audit `pre_fold_failure_facts` and the fast-operation witness ledger **separately**: retain contributors, authored-dead catch/assertion checks and selected fast-use facts still needed without release folding. Do not remove validation-only evidence simply because its name contains 'pre-fold'.
- [ ] Remove `FrontendBuildProfile` and its now-unneeded field/parameter propagation through `create_project_modules`, `module_compilation`, `pipeline`, AST scope/build contexts and generic materialisation, including config/direct-template services and tests. Keep build-owned `BuildProfile` as a physical/build-output choice. A remaining frontend profile dependency must have a named, verified observation-only purpose; never retain the enum as a no-op forwarder. Fix every other confirmed profile-dependent semantic divergence found in Phase A before proceeding.
- [ ] Update `const_execution/mod.rs`, the `RewriteWalk` variants/steps and their comments to describe only required/interpreted execution. Remove no-longer-used `FxHashMap` arguments and empty release-provenance maps rather than forwarding them. Verify ordinary call effects/CFG, imported/generic summaries, interface fingerprints, inferred failures and backend output. No release budget Note survives; required const budgets still diagnose.
- [ ] Re-run affected integration, Rust, generated sidecar, constraints and parity cases; review deletion opportunities beyond immediate R1 repair.

**Exit:** Optional ordinary-function folding is absent, semantic profile branches are gone and authored evidence is verified identically in both builds. Resolve R1 only after all reproduced divergences pass.

## Foundation Phase C: Permanent parity verification and simplified stage contract

**Purpose:** Prevent new profile-sensitive semantic behaviour before continuing const-function features.

**Files:** HIR tests and snapshot/structural validation, integration harness, semantic interface and fingerprint tests, validation configuration and canonical documentation.

- [ ] Add an unfeature-gated Rust parity test under `src/build_system/tests/compile_project_frontend_tests/`. Use **one** temporary project root and identical builder semantic capability registration, then independently compile it with `BuildProfile::Dev` and `Release` using the normal `compile_project_frontend` service. The existing `compile_fixture_result` creates a fresh directory each call, so don't invoke it twice and accidentally compare different source identities. Compare every base module and generated sidecar by stable origin. Start with `HirDisplayContext::render_module` for CFG, operation kinds, numeric modes and results, and assert omitted failure/constraint/provenance and public-interface facts explicitly. The renderer's `with_type_environment` and `with_side_table` helpers are currently guarded by `#[cfg(feature = "show_hir")]`; when using them in default unit tests, relax **only those test-facing cfg gates** to `#[cfg(any(test, feature = "show_hir"))]` and provide the published artefact's paired `TypeEnvironment` and identity context. If rendering cannot consume the published identity tables without a second owner, implement the narrow comparison over HIR fields in this test module instead. No production serializer or second IR. If incidental IDs vary, establish a test-only bijective mapping that preserves references, not regex deletion of names, IDs or operations.
- [ ] Pair structural comparison with acceptance, exact diagnostic labels and runtime output for the R1 inventory, numeric constraints, static selection, dead validation, required const/TIR, imported calls and trusted foreign summaries. Compare semantic public interfaces/fingerprints explicitly: equal rendered HIR by itself does not cover those side tables.
- [ ] Include an explicit test-compilation semantic-input control when the test-mode implementation exists; until then, record the gate for the later testing plan without inventing test mode now. Debug/release within any mode must agree.
- [ ] Recheck every `FrontendBuildProfile` use and profile-sensitive semantic branch in module compilation, generated materialisation and AST/HIR. Every surviving profile use must be demonstrated as downstream physical choice or genuinely observation-only; no no-op forwarding. Correct any unrelated confirmed semantic divergence before closing the foundation.
- [ ] Refresh benchmark/allocations/expression-row baselines without release speculation. Record the concrete complexity removals and retain/simplify/remove decisions for remaining eager folding. Update permanent documentation to describe *delivered* invariants only.
- [ ] Keep the parity comparison in the default `cargo test -p moth --lib`/normal validation lane, not under `show_hir`, benchmark flags or an opt-in matrix. Mutation-check the test comparator with one deliberate opcode/CFG difference. Run selected constraint/HIR cases and appropriate phase gates per `validation.mtf`; only claim final merge readiness after the full gate passes.

**Exit:** Canonical semantic HIR, source acceptance, diagnostics, constraint guarantees and interface summaries are profile-independent and guarded by automated regression coverage. Do not start the const feature phases before this gate completes.

---

## Phase and pause protocol

The foundation gates A-C are mandatory and sequential. The subsequent feature phases retain their independent pause and review points. Execute tasks in order, with the smallest coherent testable slices.

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
1. Review all changed source and tests against `AGENTS.md`, the routed style guide and the simplification checklist in Task 7.3. Look for duplicates, allocations, stale code and missing WHY comments.
2. Verify source diagnostics, required-const budget exhaustion and cross-module cases relevant to that phase; release-only candidate budget refusal disappears in Phase B.
3. Run targeted tests for intermediate commits, `just validate` at important phase boundaries and `just validate-full` only for final merge readiness, per `validation.mtf`.
4. Update canonical documentation and the progress matrix only for actual contract/status changes.
5. Update this plan's small status block and record the exact next action. Keep evidence, benchmark details and commits in the phase handover or Git history.
6. Pause, switch branches or integrate the completed phase as ordinary repository policy permits.

**At plan completion:** Migrate durable decisions into canonical documentation, remove the roadmap entry and delete this short-lived plan in the completing commit. Follow `docs/roadmap/roadmap.md`. Do not leave a committed "complete" plan indefinitely.


---

## Phase 0: Reassess the shared evaluator after profile cleanup

**Purpose:** Focus measurements on remaining required const execution after the foundation gates. Phase A already owns the whole-compiler baseline and Phase C measures after release speculation is removed.

**Files:** `const_eval/mod.rs`, `const_execution/{execute,rewrite,values,budget}.rs`, `module_ast/finalization/expression_census.rs`, `compiler_frontend/instrumentation/`, the benchmark owners and existing `tests/cases/const_function_*`.

### Task 0.1: Identify and benchmark redundant const work

- [ ] Trace `FoldContext::fold_walk`, `fold_root`, `ConstInterpreter::evaluate_expression_inner`, `RewriteWalk`, `fold_to_constant` and `rewrite_expression_children` against the current implementation. Count duplicate traversals, appended expression rows, staging and clones. Identify which lazy, handler, projection and TIR operations still need specialised rewriting.
- [ ] Reuse Phase A/C benchmark data, then measure representative scalar, repeated nested call, collection, catch, `Dec`, imported helper and structural-template workloads with existing counters. Add only attribution-critical benchmark instrumentation. Use `just bench-check` where useful; keep raw measurements local.
- [ ] Verify existing `const_function_constant_call_success`, `const_function_logical_short_circuit_success`, `const_function_rebuilt_catch_success`, `const_function_collection_operations_success`, `const_function_nested_template_call_success`, `const_function_imported_success`, `const_function_structural_text_refusal_rejected` and the work/memory-limit cases. Ensure each selector actually runs.

**Exit:** Measured redundant-work owners, retained semantic special cases and exact API/test targets for Phases 1-2. No new evaluator or global cache.

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
- [ ] Preserve per-call and aggregate work/memory accounting. Check that removing rewriting does not accidentally avoid previously charged work. Release speculative discovery and notes were removed in Foundation Phase B.
- [ ] Remove the superseded result-extraction path and any now-unused metadata/clones. Do not leave compatibility wrappers around old signatures.

### Task 1.4: Validate and inspect the result

- [ ] Run focused unit tests, required const-function integration cases and ordinary-call profile-parity cases.
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
- [ ] Enumerate the existing resolved `MatchPattern` forms: literal, option absent/value/present capture, relational and choice variant with payload captures. Inspect `PatternValueKind::Runtime` and `Constant`, choice tag identity, typed comparison and capture placement. These are internal typed-AST states, not a new authored expression-pattern grammar. Ordinary source pattern syntax stays unchanged.
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
- [ ] Cover valid authored literal patterns (including contextually typed numeric literals) whose typed-AST `PatternValueKind::Runtime` may become constant only after normal resolution. Do not admit local-variable or arbitrary expression patterns through const evaluation.
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
- [ ] Keep ordinary multi-result functions as ordinary runtime calls in every profile; release-only eligibility no longer exists.

### Task 4.3: Evaluate multi-slot producers at receiving sites

- [ ] Inspect `ConstInterpreter::evaluate_expression(..) -> ConstValueId`, `evaluate_expression_inner`, `execute_return`/`evaluate_return_values` and `values.rs::fold_to_constant`. Record where one `ExprId` is wrongly assumed to yield one slot. In particular, `return pair()` contains one producer expression but can supply multiple ordered slots.
- [ ] First add failing regressions for `return pair()`, forwarding a nested multi-result call and a multi-bind consuming a single multi-result producer. Cover valid value-producing `if`, `match` and `catch` blocks with multiple result slots where ordinary AST receiving rules permit them.
- [ ] Introduce the smallest receiving-site operation that evaluates **one producer once** and extracts all values according to its resolved `SuccessShape`. Reuse the current prepared frames, value store and evaluation/handler behaviour. A narrowly scoped `evaluate_result_slots(..) -> Vec<ConstValueId>`-style helper is acceptable if it removes duplicate logic, but avoid a second generic evaluator.
- [ ] Keep `evaluate_expression` and scalar operands single-valued. Route return forwarding and multi-bind through multi-slot receiving without creating tuples or flattening one slot into many accidental expressions. Preserve lazy branch selection, catch/handler behaviour and the existing effect/work budget.
- [ ] Validate slot shape, ordering and types before target mutation or AST publication. Keep the existing `commit_constant_result_block` as the only AST multi-slot replacement path. Examine alias-sensitive updates and transactional failure for receiving sites separately.

### Task 4.4: Implement multi-bind inside const bodies

- [ ] Inspect the existing `NodeKind::MultiBind { targets, value }` and `MultiBindTarget` representation. Confirm declaration versus update targets, aliases and recorded `ValueMode`.
- [ ] Add one prepared multi-bind instruction that retains the already-typed producer and target facts.
- [ ] Use Task 4.3's multi-slot receiving operation to evaluate the producer exactly once and preserve ordered results. Collect all RHS values before any existing target is updated.
- [ ] Use existing `FrameBindings`, place projection and alias-write paths for declarations and assignments. Validate target shape and avoid partially installing new bindings on a failed result producer.
- [ ] Preserve ordinary left-to-right target update semantics where aliases overlap. Do not introduce a new binding store, runtime tuple or mutable-parameter capability.

### Task 4.5: Preserve returns and receivers

- [ ] Inspect `ConstInstructionKind::Return(Vec<ExprId>)` and the ordinary return-production owner for any assumption that one expression equals one result slot. Route each expression through Task 4.3's receiving-site operation and append its validated success slots in order, especially for `return pair()`.
- [ ] Support returning multiple scalar/composite values, nested multi-result producers and value-producing blocks permitted by current AST receiving rules.
- [ ] Preserve the distinction between a multi-result producer and a single-value operand. Ensure a single-target binding rejects a multi-result call with the existing arity diagnostic.

### Task 4.6: Assert observable parity

Start with `const_multiple_result_slots_rejected` and `adversarial_multi_bind_in_loops`. Replace a rejection fixture only when its formerly invalid behaviour becomes supported. Keep `const_zero_result_function_rejected` as a separate current boundary.

- [ ] Add a multi-result const helper with two and three distinct slots and assert exact output.
- [ ] Cover a nested multi-result const call, `return pair()` forwarding, a multi-result producer used in multi-bind and cross-file import/publication. Check permitted multi-result `if`/`match`/`catch` receiving sites.
- [ ] Cover new declarations, existing mutable updates, mixed targets, alias-sensitive updates and evaluated-once effects on local state. Confirm a failing producer or invalid shape never partially commits bindings or published values.
- [ ] Assert incorrect slot count/type diagnostics at receiving sites and no silent first-slot truncation.
- [ ] Verify ordinary multi-return calls remain runtime calls in both profiles while required const multi-return calls execute during Stage 4.

**Phase 4 exit:** Required const functions support multiple ordered ordinary success slots, expression-to-result forwarding and const-body multi-bind. Multi-valued producers evaluate once at receiving sites, while scalar consumers remain single-valued. The existing native result block remains the only AST multi-result replacement representation. Zero-result, fallible, Never and generic const signatures remain outside scope.

**Docs:** Update `function-declarations.mtf`, `returns-and-multiple-values.mtf` only where the const-specific contract warrants clarification and the progress matrix. Confirm required const semantics remain accurate and ordinary-call release eligibility has no remaining documentation or implementation owner.

**Review:** Audit every residual `single()` assumption in affected producers, return forwarding, aliases and cross-module slot validation. This phase may pause.

## Phase 5: Support immutable const receiver methods

**Purpose:** Allow the existing const marker on eligible immutable source receiver methods and evaluate receiver calls through the one prepared executable/frame machine.

**Files:** `src/compiler_frontend/ast/module_ast/environment/function_signatures.rs`, `src/compiler_frontend/ast/field_access/receiver_calls/{mod,source_methods,shared}.rs`, `receiver_access.rs`, `src/compiler_frontend/ast/expressions/{function_calls,expression_store}.rs`, the function declaration/header and public-interface owners, and `const_execution/{mod,eligibility,rewrite,execute}.rs`.

### Task 5.1: Trace receiver identity before choosing an API

- [ ] Inspect method declaration and call parsing. Confirm how the current `name #|...|` modifier intersects with `this T` receiver declarations, including declaration-time restrictions.
- [ ] Trace `ExpressionOperationView::MethodCall`, resolved callable target, receiver argument position, metadata, method visibility and success shape. The receiver is stored separately from other arguments. Locate the unconditional `MethodCall` refusal in `const_execution/eligibility.rs::classify_const_expression` and the `Call`-only handling in `const_execution/rewrite.rs::const_call_site`.
- [ ] Inspect the independent gates in `module_ast/environment/function_signatures.rs` (all const receiver declarations rejected) and `field_access/receiver_calls/source_methods.rs` (calls in const bodies rejected, plus a distinct runtime-method-on-const-record refusal). Determine precisely which gate should permit eligible const methods, without weakening the data-only const-record restriction.
- [ ] Inspect how nominal type publication carries same-file methods to consumers. Identify any missing const-evaluation summary in an existing callable record before introducing a new record.
- [ ] Decide whether the method can be prepared as an ordinary `ConstFunctionExecutable` with its receiver bound as the first parameter. Prefer this shared route.

### Task 5.2: Accept the narrow receiver contract

- [ ] Allow only a shared immutable `this T` receiver on an eligible same-file source-authored nominal struct or choice.
- [ ] Require compile-time-known receiver and arguments. Use normal receiver-call syntax and the existing const marker. Keep ordinary receiver methods unaffected.
- [ ] Reject `this ~T`, generic const methods, foreign/opaque/builtin receiver extension and cross-file receiver ownership.
- [ ] Preserve nominal type identity, method visibility and the data-only const-record restriction. Eligible compile-time methods on supported nominal constant receivers must remain distinguishable from ordinary runtime methods called on data-only const-record values. Keep those runtime method calls rejected, including for nominal values where the established rule applies. Do not turn anonymous compile-time records into runtime method-capable values. If existing identity/eligibility facts cannot express this distinction, stop for an explicit design decision rather than broadly removing the guard.
- [ ] Produce a source diagnostic at the authored declaration or call boundary, as appropriate. A never-called invalid const method must receive the established declaration-time eligibility check.

### Task 5.3: Execute through the existing callable path

- [ ] Extend `const_execution/rewrite.rs::const_call_site` beyond `ExpressionOperationView::Call` to recognise the eligible `MethodCall` target, and update `eligibility.rs` without admitting arbitrary receiver calls. Align the declaration and source-method resolution gates identified in Task 5.1.
- [ ] Evaluate the separately stored receiver once, before explicit arguments, then bind it as parameter zero through the existing parameter/binding machinery. Preserve provenance and normal method resolution without constructing a synthetic second receiver evaluation.
- [ ] Reuse `ConstInterpreter::execute_body` and existing budgets. Do not build separate method frames, method registries or a method-specific value representation.
- [ ] Reuse Phase 4 result-slot completion for a const method returning multiple values.
- [ ] Confirm imported nominal methods execute from stable semantic interface facts without donor-local `ExprId` or `TypeId` leakage.
- [ ] Keep ordinary receiver methods as runtime calls in every profile; explicitly marked const methods remain required compile-time calls.

### Task 5.4: Validate method semantics

- [ ] Add or extend cases for nominal struct and choice receivers, explicit parameters, nested const calls, composite returns and multiple returns.
- [ ] Cover imported receiver types, method visibility, runtime receiver refusal, mutable receiver rejection, generic rejection and invalid ownership. Add positive/negative regressions for every declaration, resolution, eligibility and call-rewrite gate, including ordinary runtime methods on data-only const records.
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

Review changed modules under repository `AGENTS.md`, its routed style guide and the checklist below. The previously attached `moth-code-review-guide.md` is not a tracked repo file and must not be a required checkout path. Search for:

- duplicate arithmetic, cast, pattern or projection evaluation
- unnecessary expression rebuilds, clones and allocations
- overlapping helper APIs or staged compatibility wrappers
- broad state bags, long parameter lists or unexplained lint suppressions
- remaining unused speculative eligibility or profile-sensitive semantic decisions
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

**Phase 7 exit:** The accepted semantics have landed or carry a user-approved explicit deferral, required-const versus ordinary-runtime call boundaries remain correct, final benchmarks and validation are recorded and permanent design references contain every enduring decision.

## Documentation and status synchronisation

Documentation ownership follows repository `AGENTS.md` and `docs/roadmap/roadmap.md`. A roadmap plan is an implementation work item, not the permanent owner of language semantics.

### This PR and foundation implementation

This PR already consolidated and activated the plan at the top of the roadmap,
retired the separate release-folding plan and recorded optional post-HIR folding
as deferred in the progress matrix. The canonical references now describe the
**accepted target design**, while the progress matrix and open R1 bug explicitly
identify the current release-only AST behaviour pending deletion.

During Foundation Phases A-C, replace transitional implementation-drift
wording with delivered facts, remove old release budget/eligibility advice and
update progress status only when implementation changes. Close R1 only after
the preserved probes and permanent parity gate pass. Update `index.md` only
if a file or ownership boundary actually changes.

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

Generic const declarations and methods remain deferred **design investigation**, not an implementation commitment. Any future acceptance needs an explicit instantiation, compilation-cost and cross-module contract. Optional ordinary-function optimisation remains a deferred, low-priority proposal, with no active candidate eligibility or implementation.

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

Const functions execute against one typed AST/value foundation. Value-only computation no longer requires a temporary replacement expression. Necessary call/handler/template execution remains explicit and lazy. Match statements, multiple success slots, multi-bind and eligible immutable receiver methods work through established compiler owners. Stepped collection loops have either landed with full parity or a concrete deferred decision. Optional whole-function speculation remains removed, profile-independent HIR is guarded by tests, packages demonstrate useful const helpers and the codebase carries less redundant state and rewriting.