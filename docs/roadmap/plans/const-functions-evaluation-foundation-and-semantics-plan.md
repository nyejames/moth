# Const Functions: Profile-Independent Semantics and Shared Evaluation Foundation Implementation Plan

> **For agentic workers:** Implement this plan one independently reviewable phase at a time. Follow repository `AGENTS.md`, the canonical code style and testing rules and the phase review gates below. Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` when available. A phase may be committed, integrated and paused before the next one starts.

**Goal:** Enforce profile-independent semantic compilation and validated HIR, remove optional early whole-function speculation, assess folding machinery for justified cost and necessity, then simplify and extend required const-function execution.

**Architecture:** Stage 4 owns required const execution, TIR, deterministic static selection and only semantics-preserving eager folding. Identical semantic inputs produce equivalent validated HIR and semantic summaries in debug and release; profile-dependent work belongs downstream. Remove ordinary-call release speculation rather than relocating it. Retain the compact typed `ExprId` graph, immutable `ConstValueStore` and prepared interpreter. Live const execution owns bounded storage identities and projection-aware mutation generations, separate from folded value publication. It uses dynamic access validation, not runtime HIR borrow/lifetime analysis.

**Tech stack:** Rust, Moth compiler frontend, typed AST, TIR, existing integration runner and frontend benchmark instrumentation.

**Spec and permanent authorities:** `docs/compiler-design-overview.md` (Stage 4, typed expression ownership, result slots and const execution), `docs/docs/functions/function-declarations.mtf`, `docs/docs/functions/returns-and-multiple-values.mtf`, `docs/docs/structs/receiver-methods.mtf`, `docs/docs/constants/constant-folding.mtf`, `docs/docs/directives/constraints.mtf`, `docs/docs/loops/collection-loops.mtf`, `docs/developer-docs/memory-management/access-and-aliasing/access-and-aliasing.mtf`, the canonical pattern references and `docs/docs/design-scope/deferred-and-outside-scope.mtf`. The accepted decisions below supersede the old V1 restrictions only for the features explicitly approved here. Transfer enduring decisions into their canonical owners as implementation lands.

**Status:** Foundation Phases A-C and Phases 0-4 complete. Independent checkpoint corrections verified. The owner has approved dynamic generational access validation instead of runtime borrow analysis. Implementation remains pending.
**Current slice:** Corrective checkpoint C1-C5: replace live alias storage and implement generational access validation.
**Blockers:** The current value-only frame bindings and alias refresh map cannot preserve every required reference relationship. Close the storage, projection and publication gates below before Phase 5.
**Next action:** Start C1. Read the updated permanent authorities and trace the interpreter. Do not implement validation-only HIR or resume Phase 5 prematurely.

## Global constraints

- Respect the current repository `AGENTS.md`, `docs/developer-docs/style-guide/style-guide.mtf`, `testing.mtf` and `validation.mtf`.
- Stage 4 owns source typing, required constant evaluation, TIR, static selection and const dynamic access validation. HIR owns runtime executable control flow and downstream constraint, borrow and lifetime verification. Do not introduce validation-only HIR for const declarations, HIR const execution, another parser, another type checker or another template engine.
- Preserve real reference semantics, explicit copies, mutation permissions and place identity inside const execution. Multiple overlapping mutable references are permitted only in const execution. A successful mutation invalidates existing overlapping immutable references. Later use is a non-recoverable source diagnostic. The permanent access-and-aliasing reference owns the semantic contract.
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
11. Projection-aware generational access validation and bounded interpreter-owned live storage, with no const HIR, static borrow or lifetime analysis or const-only public borrow-summary hierarchy.

### Held outside this implementation

- Generic const functions and generic const receiver methods.
- Mutable-reference const parameters and mutable receiver methods.
- Fallible success/error return contracts, Never const functions and zero-success-result const functions.
- Recursive const-function call graphs and recursive declaration dependencies.
- General const propagation through runtime locals, arbitrary const-evaluation caching and whole-program partial evaluation.
- Any optional ordinary-function folding, including scalar calls. This is a low-priority future post-semantic optimisation subject to evidence that backend optimisers are insufficient.

These items do **not** all have accepted future semantics. Generic const support and future post-semantic whole-function optimisation each need a dedicated design and performance case. Mutable parameters, error slots, Never, zero results and recursion remain open semantic questions until explicitly accepted. Neither this plan nor a progress-matrix entry promises their eventual inclusion.

### Permanently enforced compile-time boundary

Required const execution cannot depend on runtime values, IO, async/channels, Wiring, mutable application/module state, arbitrary untrusted foreign effects, runtime-only templates, macro-like AST transformation, physical target inspection or final builder-dependent URL text. Local mutation is permitted. No live mutable interpreter storage or reference handle may escape the evaluation. Trusted registered Core constant operations remain permitted under their existing contracts. Structural resource-bearing `String` values remain valid constants without guessing their final characters.

Keep the ordinary language's exclusions, such as tuple values, macros and operator overloading. Do not invent const-only language forms to work around an unsupported operation.

### Scope distinctions

- Match **inside const-function execution** does not introduce general static match specialisation of ordinary runtime code.
- Allowing multi-result **required** const calls does not introduce optional ordinary-function evaluation.
- Immutable const methods do not enable methods on anonymous data-only const records or bypass same-file receiver ownership.
- Required constant evaluation remains eager where language semantics need values. Retained pure eager folding must be deterministic, justified and preserve selected-operation evidence; never evaluate unreachable work.
- Static branch selection uses only required const values and established deterministic eager expression folding, never optional whole-function evaluation.
- Explicit test compilation is a semantic input. Debug/release are optimisation and instrumentation profiles, not source semantics.
- A declaration with no calls still receives its normal static eligibility/type checks. Its parameterised body need not execute. Dynamic access failures are checked only on executed paths, including when a previously unused function or branch first executes. This is an accepted source contract, not an incomplete static proof.

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

- [x] Read current authorities and run the R1 inventory above (F1-F9, opposite-direction acceptance and transitive fast-use controls). Record exact verdicts and diagnostic labels before modifying semantic code. A previously missing or already repaired case must be reported explicitly; do not invent an expected failure.
- [x] Trace every `FrontendBuildProfile`, `BuildProfile` and backend/builder-sensitive path that can influence AST/HIR construction, generated instantiation, static selection, diagnostics, constraint summaries, public interfaces or reuse fingerprints. Distinguish explicit semantic inputs, prohibited profile choices, observation-only measurements and post-verification physical decisions.
- [x] Map current eager operator folding, mandatory const calls, optional ordinary-call candidate evaluation, TIR, static selection and HIR proof/failure contributors, noting every `ExprId` rewrite, `ConstValueId` creation and required consumer.
- [x] Audit each early folding subsystem against a retain/simplify/remove decision: mandatory semantic consumers, code/state complexity, frontend time and memory in both profiles, and equivalent later backend optimisation. Measure representative workloads. Preserve currently specified static selection and source diagnostics as a hard constraint: dropping an eager fold that changes those facts is a language change requiring separate approval, never an unreviewed performance refactor.
- [x] Inspect the real project compilation fixtures in `src/build_system/tests/compile_project_frontend_tests/mod.rs`, which accept `BuildProfile` and expose each `CompiledModuleArtifact.module.executable.hir`. Start with `HirDisplayContext::render_module` and identify any missing semantic side-table/public-summary fields; use test-only comparisons, not a second HIR representation or production serializer.
- [x] Recheck the roadmap plans affected by the profile and explicit test-mode contract against changes since this PR's initial survey. Its documentation-only review already covered the then-current queue; avoid repeating a blanket audit without new code evidence.

### Phase A findings

**R1 verdicts (debug / release, HTML backend):** F1 `trap_assertion` (annotation + assertion labels) / accepted. F2 `escaping_error` (annotation only) / accepted. F3 `trap_assertion` / accepted. F4 `escaping_error` (annotation + division labels) / accepted. F5 `trap_division` (annotation + division labels) / accepted. F6 `trap_division` / accepted. F7 `trap_assertion` / accepted. F8 `trap_assertion` in both. F9 accepted in both. All five opposite-direction controls and the transitive fast-use summary control are accepted in both profiles. No case was already repaired.

**Profile map:** the finalizer's `FrontendBuildProfile::Release` branch calling `rewrite_release_calls` is the only semantic profile decision. Every other `FrontendBuildProfile` use forwards it (build `create_project_modules`, `pipeline`, `module_compilation`, AST build/scope contexts, environment lookups, constant resolution, emission and generic materialisation) or pins `Dev` for config/direct-template services and tests. `cfg(debug_assertions)` validation and `benchmark_counters` are observation-only. `BuildProfile` in `build.rs`, the HTML JS path and Wasm artefacts is post-verification physical output. No reuse fingerprint reads a profile.

**Folding decisions:** retain required const calls, TIR, explicit-constant operands, static Bool selection and deterministic operator folding: each has a mandatory Stage 4 consumer. Remove release speculation: `rewrite_release_calls`, `release_call_targets`, `RewriteMode::Release`, auxiliary scoping (`fold_auxiliary`), handled-subject/catch skipping, `revert_refused_call`, `release_can_preserve_runtime`, `merge_release_candidate_provenance`, `unexecuted_expression_roots`, `eligibility.rs` release preparation, purity and call-graph proofs, `ExecutionState.release_erased_calls`, `AstReleaseErasedCallSite` and its HIR projection, the `ReleaseBudgetScope` Note and `release_speculation_scope_tests.rs`. Without release folding the post-finalisation store equals the pre-fold store, so `pre_fold_bool_conditions`, `PreFoldConditionRoots` and the HIR lookup collapse into direct `folded_bool_condition` reads. `pre_fold_failure_facts` stays: finalisation computes it for generic boundary validation and HIR projects it, so only its release-specific name and comments change. Matched release-compiler timings (median of 15) and peak RSS on seven benchmark workloads differ by no more than measurement noise between profiles (for example `speed-test` 22.2 / 22.1 ms, `fold-stress` 13.4 / 14.1 ms). Speculation has no measurable compile-time cost to recover, and its runtime benefit (constant-argument call elimination) is ordinary backend optimiser work.

**Parity test design:** one temporary project compiled twice through `compile_project_frontend` with `BuildProfile::Dev` and `Release`, comparing each module's `HirDisplayContext::render_module` output plus explicit failure-summary, constraint and public-interface facts. Roadmap recheck found no plan changes since the survey beyond the already-updated test-mode wording.

**Exit:** Profile-dependence map, reproduced regressions, folding-system decisions with measurements, exact deletion candidates and one reviewable HIR parity test design. No second evaluator.

## Foundation Phase B: Delete optional ordinary-call speculation

**Purpose:** Fix the confirmed semantic boundary violation by removing optional runtime-function evaluation before HIR, not by migrating it to a new HIR transaction pass.

**Files:** `ast/module_ast/finalization/finalizer.rs`, `ast/const_values/const_execution/{mod,rewrite,eligibility,execute,budget}.rs`, `ast/const_eval/mod.rs`, `ast/mod.rs`, HIR constraints/failure owners, public-interface and sidecar publication plus `tests/cases/`.

- [x] Turn F1-F9 and the acceptance controls above into debug/release integration tests with exact reasons, source labels and runtime results where appropriate. Add a HIR-level assertion that a formerly folded ordinary call remains a call in both profiles while required const functions still fold at Stage 4.
- [x] Remove the finalizer's `rewrite_release_calls` and `FrontendBuildProfile::Release` branch. Delete `eligibility.rs`'s `prepare_release_executable`, purity/dependency graph proofs, retained candidate types and `rewrite.rs`'s release-only target discovery, provenance, `RewriteMode::Release`, auxiliary speculation, `revert_refused_call`, `release_can_preserve_runtime` and whole-call dead-root exclusion. Remove `ExecutionState.release_erased_calls` and `ReleaseBudgetScope` diagnostic/renderer/tests that only exist for those notes. Retain `RewriteMode::RequiredExpression` and `InterpretedBody` while both have distinct consumers. Preserve `ConstValueStore::stage_values` rollback, required const budgets, general `FoldRequirement` decisions and runtime failure handling; do not delete those merely because they also supported release speculation. No dormant feature switch or late optimiser.
- [x] Trace `AstReleaseErasedCallSite` through `ast/mod.rs` and `hir/hir_builder/metadata.rs`; remove its HIR live-erased-call projection. Audit `pre_fold_bool_conditions` and `PreFoldConditionRoots` in the finalizer, AST control-flow exits, HIR statement/control-flow lowering and expression freeze: delete the map and special freeze roots if direct finalised AST reads now supply the same facts. Audit `pre_fold_failure_facts` and the fast-operation witness ledger **separately**: retain contributors, authored-dead catch/assertion checks and selected fast-use facts still needed without release folding. Do not remove validation-only evidence simply because its name contains 'pre-fold'.
- [x] Remove `FrontendBuildProfile` and its now-unneeded field/parameter propagation through `create_project_modules`, `module_compilation`, `pipeline`, AST scope/build contexts and generic materialisation, including config/direct-template services and tests. Keep build-owned `BuildProfile` as a physical/build-output choice. A remaining frontend profile dependency must have a named, verified observation-only purpose; never retain the enum as a no-op forwarder. Fix every other confirmed profile-dependent semantic divergence found in Phase A before proceeding.
- [x] Update `const_execution/mod.rs`, the `RewriteWalk` variants/steps and their comments to describe only required/interpreted execution. Remove no-longer-used `FxHashMap` arguments and empty release-provenance maps rather than forwarding them. Verify ordinary call effects/CFG, imported/generic summaries, interface fingerprints, inferred failures and backend output. No release budget Note survives; required const budgets still diagnose.
- [x] Re-run affected integration, Rust, generated sidecar, constraints and parity cases; review deletion opportunities beyond immediate R1 repair.

### Phase B outcome

Release speculation, its eligibility proofs, erased-call evidence, the release budget Note (`MOTH-RULE-0090`), the budget `aggregate` scope flag and both pre-fold tables are deleted. HIR reads folded `Bool` conditions from the shared constant store; `pre_fold_failure_facts` survives as `Ast::function_failure_facts`. The frontend takes no profile input at all: `FrontendBuildProfile` is gone and `compile_project_frontend` and its siblings dropped their forwarded `BuildProfile`, which now reaches only `BackendBuilder::build_backend` and output policy. The R1 cases are `source_constraints_ordinary_call_evidence_{rejected,accepted}[_release]`; the HIR assertion is `ordinary_constant_argument_call_stays_a_hir_call_while_const_call_folds` (the frontend has no profile, so one lowering covers both builds). Two release HTML-Wasm cases now report the same first unsupported operation as debug. Consequence for Phase C: Dev-versus-Release compilation through `compile_project_frontend` would compare identical calls, so the parity gate must sit at the build level.

**Exit:** Optional ordinary-function folding is absent, semantic profile branches are gone and authored evidence is verified identically in both builds. Resolve R1 only after all reproduced divergences pass.

## Foundation Phase C: Permanent parity verification and simplified stage contract

**Purpose:** Prevent new profile-sensitive semantic behaviour before continuing const-function features.

**Files:** HIR tests and snapshot/structural validation, integration harness, semantic interface and fingerprint tests, validation configuration and canonical documentation.

- [x] Add an unfeature-gated Rust parity test under `src/build_system/tests/compile_project_frontend_tests/`. Use **one** temporary project root and identical builder semantic capability registration, then independently compile it with `BuildProfile::Dev` and `Release` using the normal `compile_project_frontend` service. The existing `compile_fixture_result` creates a fresh directory each call, so don't invoke it twice and accidentally compare different source identities. Compare every base module and generated sidecar by stable origin. Start with `HirDisplayContext::render_module` for CFG, operation kinds, numeric modes and results, and assert omitted failure/constraint/provenance and public-interface facts explicitly. The renderer's `with_type_environment` and `with_side_table` helpers are currently guarded by `#[cfg(feature = "show_hir")]`; when using them in default unit tests, relax **only those test-facing cfg gates** to `#[cfg(any(test, feature = "show_hir"))]` and provide the published artefact's paired `TypeEnvironment` and identity context. If rendering cannot consume the published identity tables without a second owner, implement the narrow comparison over HIR fields in this test module instead. No production serializer or second IR. If incidental IDs vary, establish a test-only bijective mapping that preserves references, not regex deletion of names, IDs or operations.
- [x] Pair structural comparison with acceptance, exact diagnostic labels and runtime output for the R1 inventory, numeric constraints, static selection, dead validation, required const/TIR, imported calls and trusted foreign summaries. Compare semantic public interfaces/fingerprints explicitly: equal rendered HIR by itself does not cover those side tables.
- [x] Include an explicit test-compilation semantic-input control when the test-mode implementation exists; until then, record the gate for the later testing plan without inventing test mode now. Debug/release within any mode must agree.
- [x] Recheck every `FrontendBuildProfile` use and profile-sensitive semantic branch in module compilation, generated materialisation and AST/HIR. Every surviving profile use must be demonstrated as downstream physical choice or genuinely observation-only; no no-op forwarding. Correct any unrelated confirmed semantic divergence before closing the foundation.
- [x] Refresh benchmark/allocations/expression-row baselines without release speculation. Record the concrete complexity removals and retain/simplify/remove decisions for remaining eager folding. Update permanent documentation to describe *delivered* invariants only.
- [x] Keep the parity comparison in the default `cargo test -p moth --lib`/normal validation lane, not under `show_hir`, benchmark flags or an opt-in matrix. Mutation-check the test comparator with one deliberate opcode/CFG difference. Run selected constraint/HIR cases and appropriate phase gates per `validation.mtf`; only claim final merge readiness after the full gate passes.

### Phase C outcome

**Parity gate (deviation from the item text, recorded as pushback):** after Phase B the frontend takes no profile, so compiling twice through `compile_project_frontend` would compare identical calls. `profile_parity_tests.rs` instead builds one project root through `build_project` without and with `Flag::Release`, the only place a profile exists before backend lowering. A capturing `BackendBuilder` wraps `HtmlProjectBuilder` and records pretty `Debug` of every module and generated sidecar handed to the backend (HIR with all side tables, type environment, borrow report, numeric proofs, resources, link facts, external imports and entry metadata with folded const fragments), the project facade interfaces, the text of every interned string and the successful-build warnings. Snapshots are byte-identical between profiles without any ID mapping, so no renderer cfg gate was relaxed. The one exception is interned `__scope_N` text, drawn from the process-wide `CONTROL_FLOW_SCOPE_COUNTER` and so different between any two builds in one process; the snapshot numbers those components by first appearance, a bijection the item permits (recorded in `docs/bug-reports.md`). A rejected fixture compares root-relative rendered diagnostics (code, reason, severity, title, locations, labels, message and guidance) and pins the `trap_assertion` and `escaping_error` `MOTH-RULE-0091` records. Coverage guards assert at least three lowered modules, a facade interface and a const fragment. Mutation check: changing `value * 3` to `value + 3` in the release build reported `operator: Multiply` versus `operator: Add` at the first differing line; changing a divisor operator reported the span change.

**Coverage pairing:** the accepted fixture covers required const calls, static `if` selection, authored-dead assertion validation, `$infallible` evidence beside ordinary calls, a match, imported module calls, trusted `@core/text` and `@core/random` summaries and a generated generic sidecar. Debug/release integration pairs remain the acceptance, label and runtime-output authority for the R1 inventory, numeric constraints and template folding.

**Test mode:** no test-compilation mode exists yet. The later testing plan must add a test-mode semantic-input control to this gate; debug and release within each mode must agree.

**Profile recheck:** no `FrontendBuildProfile` remains. `BuildProfile` lives only in `build_system/{build.rs,build_profile.rs,output/{manifest,policy}.rs}` (re-exported from `build_system/mod.rs`), `builder/{compiled_project,mod,project}.rs` and `projects/html_project/{compile_input,html_project_builder}.rs`, whose `HtmlCompileInput::build_profile` field `js_path.rs` and `wasm/artifacts.rs` read. The `moth::builder` API only turns it into `Flag::Release` for `build_project`; every other use is downstream of the frontend call: backend build, JS path, Wasm artefacts and output folders.

**Measurements** (release binaries, median of 15 builds in ms, baseline `a350bec08f` versus Phase B; RSS in MiB): fold-stress dev 13.5/12.1, release 11.7/12.0; speed-test 20.6/20.4, 21.0/20.8; template-stress 12.2/12.0, 11.9/12.4; type-stress 13.0/12.8, 14.2/12.8; collection-stress 20.7/20.4, 20.8/20.6; pattern-stress 13.6/14.0, 13.6/13.4; module-graph 11.0/11.4, 10.9/11.5. All differences are within run-to-run noise (about 5%); peak RSS moved by under 1 MiB on every workload and the binary shrank by 97 KiB. Phase B removed about 4,500 lines, the `FrontendBuildProfile` chain and both pre-fold tables. Remaining eager folding keeps the Phase A retain decisions: required const calls, TIR, explicit-constant operands, static Bool selection and deterministic operator folding each have a mandatory Stage 4 consumer.

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

- [x] Trace `FoldContext::fold_walk`, `fold_root`, `ConstInterpreter::evaluate_expression_inner`, `RewriteWalk`, `fold_to_constant` and `rewrite_expression_children` against the current implementation. Count duplicate traversals, appended expression rows, staging and clones. Identify which lazy, handler, projection and TIR operations still need specialised rewriting.
- [x] Reuse Phase A/C benchmark data, then measure representative scalar, repeated nested call, collection, catch, `Dec`, imported helper and structural-template workloads with existing counters. Add only attribution-critical benchmark instrumentation. Use `just bench-check` where useful; keep raw measurements local.
- [x] Verify existing `const_function_constant_call_success`, `const_function_logical_short_circuit_success`, `const_function_rebuilt_catch_success`, `const_function_collection_operations_success`, `const_function_nested_template_call_success`, `const_function_imported_success`, `const_function_structural_text_refusal_rejected` and the work/memory-limit cases. Ensure each selector actually runs.

**Exit:** Measured redundant-work owners, retained semantic special cases and exact API/test targets for Phases 1-2. No new evaluator or global cache.

### Phase 0 outcome

**Trace.** One required call `f(1)` whose body is `return g(x) + 1` (`g` returns `x * 2`) runs three `RewriteWalk`s (program site, `f` body, `g` body), four `fold_walk`s (each argument, then each body result over the rewritten tree), four `stage_values` transactions and five collection/handler probes. It appends about nine expression rows, of which one survives. Each `fold_to_constant` commits a `Constant` row only to read its `ConstValueId` back. Each interpreted expression is walked twice: `evaluate_expression_inner` rewrites it, then `fold_to_constant` folds the rewritten tree. Rebuilt parents and their provenance rows die once that fold settles.

**Measurements.** Release CLI build of the main module, median of 15 runs. Rows are `ast_census_expression_rows` from a `benchmark_counters` build. A literal-only control with the same 600 declarations takes 12.3 ms and produces 1,850 rows.

| Workload | Call sites | Time (ms) | Expression rows |
| --- | --- | --- | --- |
| scalar arithmetic | 600 | 16.4 | 4,871 |
| nested depth 8 | 300 | 18.0 | 10,290 |
| collection loop | 300 | 16.1 | 5,768 |
| catch on `get` | 300 | 14.2 | 3,550 |
| `Dec2` arithmetic | 300 | 13.3 | 2,161 |
| imported helper | 600 | 14.7 | 3,663 |
| template result | 300 | 13.3 | 2,752 |

A scalar call costs about 7 µs and five dead rows. Each nested call level adds about four rows. The absolute cost is small. No existing benchmark workload declares a const function, so these workloads stay local and no counter was added: the row census already attributes the dead rows.

**Redundant-work owners, by cost.**
1. `fold_to_constant` commit-and-read-back, through `fold_root` and `commit_replacement`. This is the Phase 1 target.
2. The rewrite-then-fold double walk in `evaluate_expression_inner`. This is the Phase 2 target.
3. Per-node `Vec` allocation and `root_substitution`/`const_call_site` probes in `RewriteWalk::enter`.
4. Repeated `evaluate_collection_read` and handler-table probes per expression.
5. Provenance clones and the extra provenance row in `inherit_replaced_provenance`.
6. In templates: per-template sorting of bound paths, two rows per frame binding and a `FrameBindings` rebuild per site.

**Retained special cases.** These keep rewriting:
- Required-call substitution in program expressions.
- Catch-handler identity keyed by `ExprId`, and early handler return.
- Lazy `and`/`or`: `FoldContext::complete` requires both operands, so routing these through `fold_walk` would execute the inactive operand.
- TIR template completion.
- Collection mutation and bounds recovery.

**Phase 1 targets.**
- Extract the scalar staging block of `fold_root` into a value-only fold over `&AstExpressionBuilder` that returns value, not-constant or refusal.
- `fold_root` keeps the multi-slot host branch and the replacement commit.
- `fold_expression_with_constants` becomes that value-only fold. Its callers are `ConstInterpreter::fold_to_constant`, `ConstValueResolver::resolve_expression`, and the unit tests in `type_resolution_tests.rs` and `direct_projection_tests.rs`. All of them want a value.
- Tests: the value-only fold leaves the row count and the original `ExprId` untouched, a refusal publishes no reachable value, and the `Runtime` boundary still refuses composites that `Content` builds.

**Phase 2 targets.**
- Try a value-only fold of the original expression with frame bindings at the top of `evaluate_expression_inner`.
- Rewrite only when that fold reports not-constant and the tree contains const calls, catches, lazy logic, templates or collection reads.
- Keep `RewriteWalk` for program-site substitution.
- Regressions: `const_function_logical_short_circuit_success`, `const_function_rebuilt_catch_success`, `const_function_collection_operations_success` and `const_function_nested_template_call_success`.

## Phase 1: Extract value-only evaluation from AST replacement

**Purpose:** Make the existing shared fold walker return a constant value without manufacturing an expression row when the consumer only needs a value.

**Files:** `src/compiler_frontend/ast/const_eval/mod.rs`, `const_values/store.rs`, `const_execution/values.rs` and existing `const_eval/tests/` plus relevant integration cases.

### Task 1.1: Fix the result and ownership contract before code changes

- [x] Split current `fold_root` responsibilities on paper: read root/metadata, stage candidate values, perform `fold_walk`, retain refusal and provenance, select host multi-slot handling, and commit an AST replacement.
- [x] Choose the minimum internal result type or existing outcome reuse that distinguishes `Value(ConstValueId)`, `NotConstant` and `Refused(reason)`, with infrastructure failures in the existing error lane. These are conceptual variants, not mandatory new names.
- [x] Define an API that reads the builder immutably, accepts the current `FoldBoundary` and local binding lookup and returns the staged value without touching expression rows. Keep the existing transaction scope and `ConstValueCandidate`.
- [x] Retain separate treatment of multi-slot trusted host calls when they require `commit_constant_result_block`. Do not force those results into a scalar value type or duplicate `SuccessShape`.
- [x] Review `Dec` exponent context, occurrence provenance, catch eligibility and the runtime/content boundary before choosing final Rust signatures.

### Task 1.2: Prove the new boundary with focused tests

- [x] Add a const-eval unit test where value-only evaluation reads a local binding and leaves the original `ExprId` and expression-row count unchanged. The computed value must match ordinary folding.
- [x] Add refusal/error tests that assert candidate values do not publish a partial reachable result and the original AST remains untouched.
- [x] Add a test distinguishing a `Runtime` fold refusal for a composite operation from legal `Content` composite evaluation. Runtime folding must not silently become more powerful.
- [x] Verify structural resource provenance, exact fixed-width numeric/`Dec` results and the original required versus opportunistic failure policy.

### Task 1.3: Implement the smallest extraction

- [x] Extract the value-computation portion of `fold_root`. Keep one actual `fold_walk` and one semantic implementation for numeric, cast, logical, comparison and trusted Core operations.
- [x] Make `constant_fold`, `fold_expression_with_constants` and related expression-oriented callers commit replacements only when they need a rewritten `ExprId`.
- [x] Change `ConstInterpreter::fold_to_constant` to consume the value directly instead of constructing `ExpressionOp::Constant` and reading the new row back.
- [x] Preserve per-call and aggregate work/memory accounting. Check that removing rewriting does not accidentally avoid previously charged work. Release speculative discovery and notes were removed in Foundation Phase B.
- [x] Remove the superseded result-extraction path and any now-unused metadata/clones. Do not leave compatibility wrappers around old signatures.

### Task 1.4: Validate and inspect the result

- [x] Run focused unit tests, required const-function integration cases and ordinary-call profile-parity cases.
- [x] Compare expression-row growth and elapsed time with Phase 0 for pure repeated expressions and composite values.
- [x] Audit new borrowing/lifetime constraints. The evaluator reads expression rows while candidate values stage, so no mutable builder borrow should leak into the walk.
- [x] Review whether `const_eval/mod.rs` should split by real responsibilities. A split is justified only if it removes mixed ownership or materially improves the current code.

**Phase 1 exit:** Value-only const evaluation operates over the existing typed rows and value store. Const execution no longer commits an AST constant merely to recover its value ID. Ordinary folding still commits the correct replacement when required.

**Review:** Check transaction rollback, `Dec` context, provenance and runtime/content boundary parity. Phase 1 can pause here independently.

### Phase 1 outcome

**Code changes.**
- `fold_root` now delegates the staged walk to `fold_value`. `fold_value` borrows the builder immutably and returns `ConstantValueFold::{Value, NotConstant, Refused}`.
- `fold_root` keeps the multi-slot host branch and the replacement commit.
- Occurrence facts are cloned only after a successful fold.
- `fold_expression_with_constants` was replaced by the value-only `fold_value_with_constants`, with no wrapper. Every caller wanted a value: `fold_to_constant`, `ConstValueResolver::resolve_expression` and test fixtures. The resolver's `Constant` read-back arm was unreachable and is gone.
- Budget accounting is unchanged. The dropped row was never charged: `budget.rs` counts value-store values and bytes, not builder rows.

**Callers left as they are.** Signature defaults, TIR conditions, loop operands and output folding still commit rows. They either need the rewritten `ExprId` or read through wrapper rows when the fold returns not-constant. None of them is on the const-execution path that Phase 0 measured.

**Measurements against Phase 0.** Same method as Phase 0.

| Workload | Rows before | Rows after | Time before (ms) | Time after (ms) |
| --- | --- | --- | --- | --- |
| scalar | 4,871 | 3,071 | 16.0 | 15.1 |
| nested depth 8 | 10,290 | 5,790 | 17.8 | 15.4 |
| collection | 5,768 | 3,968 | 15.3 | 13.8 |
| catch | 3,550 | 2,769 | 13.8 | 12.8 |
| `Dec2` | 2,161 | 1,561 | 13.2 | 11.9 |
| imported | 3,663 | 3,063 | 15.5 | 13.9 |
| template | 2,752 | 2,752 | 12.4 | 12.4 |

The literal-only control is unchanged. Template const calls go through TIR site rewriting, which Phase 2 owns.

**`const_eval/mod.rs` split.** No split. The extraction separates value computation from row commit inside one owner, and the file has no mixed ownership that a split would remove.

## Phase 2: Reduce repeated rewriting in const-function execution

**Purpose:** Let the interpreter evaluate pure typed expressions with bindings directly while retaining specialised handling for complex execution semantics.

**Files:** `const_execution/{execute,rewrite,eligibility,places,values,state,budget}.rs`, `const_eval/mod.rs` and expression-rewrite owners only where obsolete calls can be removed.

### Task 2.1: Choose the safest direct path

- [x] Classify existing expression operations into pure value evaluation and interpreter-owned operations. Start with references, scalar arithmetic, comparisons, casts and supported composites.
- [x] Preserve specialisation for nested user const calls, mutable collection/place writes, TIR, catch/handler propagation and lazy logical control flow until each can be proven equivalent.
- [x] Determine whether the current `FoldContext` nonconstant return can trigger the established fallback without emitting a premature semantic error. If not, introduce the smallest operation-level gate. Avoid a costly new whole-tree eligibility scan on every expression.
- [x] Keep `RewriteWalk` where program-expression rewriting is genuinely required. Do not build another general statement interpreter inside `FoldContext`.

### Task 2.2: Protect lazy and effectful cases before changing dispatch

- [x] Write a regression with `false and rhs` and `true or rhs` where the inactive operand would fail if executed.
- [x] Cover nested const calls, selected versus unselected catches, early return from handlers and a projection of a mutable local composite value.
- [x] Verify the earlier source type/eligibility checks still run where required, independent of whether runtime execution visits a branch.

### Task 2.3: Implement and delete redundant work

- [x] Insert the value-only path at the narrowest correct point in `evaluate_expression_inner`. Feed it existing frame bindings and retain the original `ExprId`.
- [x] Prefer direct `ConstValueId` projection using existing `project_constant_value` when a simple projection needs no expression reconstruction.
- [x] Rebuild expression children only for a case that needs altered semantic edges, call substitution, catch authority or program-level AST replacement.
- [x] Remove now-dead substitution/rebuild helpers and state once their users have migrated. Retain source-provenance transfer and catch-body identity where replacements remain necessary.
- [x] Audit evaluation-order and budget differences on every newly direct operation.

### Task 2.4: Decide whether further architectural work pays for itself

- [x] Compare Phase 0 and Phase 1/2 time, allocations, row counts and peak memory with matched inputs.
- [x] Check that a fallback after direct evaluation has not introduced a second expensive traversal for common unsupported expressions.
- [x] If simple direct execution produces the useful win, stop. If a measured hotspot remains, record the exact operation/representation responsible before proposing a compact instruction or additional specialisation.
- [x] Reject unmeasured global memoisation, generic value caches, a replacement AST/IR, a second expression callback framework and general bytecode at this stage.

**Phase 2 exit:** Direct evaluation covers demonstrably useful pure operations without semantic drift or a competing evaluator. Measured costs and a clear stop decision are recorded.

**Review:** Compare the final responsibilities of `FoldContext`, `ConstInterpreter` and `RewriteWalk`. Remove duplicated decisions and ensure each remaining layer owns a real invariant.

### Phase 2 outcome

This records the original implementation. The [corrective checkpoint](#corrective-checkpoint-before-phase-5)
removed this speculative probe after tracing work performed before its budget charge, duplicated
fallback evaluation and text interning outside the value-store transaction.

**Design.**
- `evaluate_expression_inner` first calls `ConstInterpreter::evaluate_directly`. It runs right after the existing collection-read probe and before handler setup and `RewriteWalk`.
- `evaluate_directly` folds the original `ExprId` through `fold_interpreted_value` with the frame bindings. `fold_interpreted_value` is the Phase 1 value fold under a new private `FoldBoundary::Interpreted`.
- That boundary is required content. `LogicalBinary` and user const calls settle as not constant immediately. The alternative, eager evaluation, would build values, text and provenance for an operand the interpreter may skip, and would walk call arguments for a call that can never fold here.
- Any not-constant result, refusal or source diagnostic falls back to the unchanged rewrite path. That path owns evaluation order, so it decides which failure is reported.
- No whole-tree scan was added.
- No rewrite helper became dead: calls, catches with handler bodies, collection reads, `copy` places, templates, `#` constant roots and short-circuit logic still take the fallback.
- `copy` reads keep their existing post-rewrite `project_constant_value` projection. Their index expressions need interpreter evaluation, and the measured workloads do not contain them.
- Source type and eligibility checks run at body preparation, before execution, so they are unaffected.

**Equivalence.**
- The probe settles only constants, bound references, numeric, comparison and `not` operations, trusted Text host calls, composites and foldable casts, including a catch or `!` wrapper around one. For those trees the old rewrite changed nothing but wrapper rows, and its final fold ran the same walk.
- Handler state matters only on failure, and every failure falls back.
- Staged values roll back on fallback, and the probe charges nothing.

**Budget differences, accepted.**
- A direct value costs one step, charged at the whole expression's span. The old path also charged one step for its final fold, plus a separate fold attempt on the inner value of a catch or `!` wrapper.
- A const body binding `cast t catch then 0` ten times in a loop now passes with a work limit of 69 instead of 79. Pure expressions charge the same.
- Work-limit diagnostics on direct expressions now point at the whole expression instead of the last leaf the rewrite walk entered.
- New value text is still charged once, by `evaluate_expression`.

**Tests.**
- The unit test `interpreted_probe_leaves_an_inactive_operand_to_the_interpreter` pins the logical gate. Removing the gate fails it.
- `const_function_logical_short_circuit_success` now covers failing pure operands, both at the root and under `not`.
- `const_function_parameter_cast_success` covers a caught cast in a const body.
- Existing cases cover nested calls, selected and unselected catches, early handler return (`const_function_mutation_handler_return_success`) and mutable composite projection.
- A caught *failing* cast is rejected in every compile-time context, before and after this phase. It is recorded in `docs/bug-reports.md`.

**Measurements against Phase 1.** Same method as Phase 0. The heavy workloads are local: 600 calls of a 20-statement arithmetic body, and a body with 10 `if flag and acc > k` statements.

| Workload | Phase 1 (ms) | Phase 2 (ms) |
| --- | --- | --- |
| scalar | 15.1 | 14.5 |
| scalar, heavy body | 33.2 | 22.2 |
| logic, heavy body | 22.8 | 20.9 |
| nested depth 8 | 15.3 | 15.4 |
| other Phase 0 workloads | unchanged within noise | unchanged within noise |

Row counts are unchanged, because pure expressions never appended rows. Call-dominated workloads do not regress: the probe settles a user call at its root without visiting its arguments.

**Stop decision.** Stop here. Short-circuit logic and calls are the remaining interpreter-owned costs. Making `FoldContext` lazy would give short-circuit evaluation a second owner. Call execution is genuine work. No compact instruction form or cache is justified.

**Layer responsibilities.**
- `FoldContext`: the one semantic implementation of value operations.
- `ConstInterpreter`: statements, handlers, collection effects and evaluation order.
- `RewriteWalk`: program-site call substitution and the ordered fallback for interpreter-owned expressions.


---

## Phase 3: Support match statements in const functions

**Purpose:** Execute Moth's existing typed match semantics rather than a reduced const-only pattern language.

**Files:** `src/compiler_frontend/ast/ast_nodes.rs` (read-only shape), `src/compiler_frontend/ast/statements/match_patterns/` (canonical pattern facts), `const_execution/{instructions,execute,state,eligibility}.rs` and `src/compiler_frontend/hir/hir_statement/{control_flow,match_captures}.rs` (read semantics, no HIR dependency).

### Task 3.1: Discover the exact matching contract

- [x] Read the canonical match/pattern documentation and inspect `NodeKind::Match { scrutinee, arms, default, exhaustiveness }` and `MatchArm { pattern, guard, body }`.
- [x] Enumerate the existing resolved `MatchPattern` forms: literal, option absent/value/present capture, relational and choice variant with payload captures. Inspect `PatternValueKind::Runtime` and `Constant`, choice tag identity, typed comparison and capture placement. These are internal typed-AST states, not a new authored expression-pattern grammar. Ordinary source pattern syntax stays unchanged.
- [x] Trace HIR match lowering solely to establish the existing observable ordering. Do not call HIR helpers from Stage 4 or recompute exhaustiveness from source syntax.
- [x] Design one prepared match instruction that retains resolved facts and compiles each arm/default body once. Prefer shallow existing AST-owned pattern storage or compact prepared arm entries over cloning whole statements per invocation.

### Task 3.2: Implement arm selection through the existing frames

- [x] Before implementation, add a selected-arm diagnostic regression: a failing unselected guard/body must not execute, but authored branches must still type-check.
- [x] Replace the explicit `NodeKind::Match` preparation rejection with a prepared instruction and execute it through `ConstInterpreter`.
- [x] Evaluate the scrutinee once. Check patterns in authored order, evaluate a guard only after its pattern succeeds and execute the first arm whose pattern and guard both succeed.
- [x] Publish captures into the selected arm's lexical binding scope before evaluating its guard. Ensure captures cannot leak to the next arm, the default or the enclosing function.
- [x] Retain ordinary `return`, `break` and `continue` unwinding and the existing work budget. A guard that rejects one arm must not leave a pending handler or captured binding.
- [x] Consume the AST's proven exhaustiveness/default facts without adding a second exhaustiveness checker.

### Task 3.3: Complete coverage and diagnostics

- [x] Extend existing match/const tests for literals, relational patterns, exhaustive choices, choice payload capture, Option none/value/present capture, guards, default and nested matches.
- [x] Cover valid authored literal patterns (including contextually typed numeric literals) whose typed-AST `PatternValueKind::Runtime` may become constant only after normal resolution. Do not admit local-variable or arbitrary expression patterns through const evaluation.
- [x] Cover match inside a range or collection loop, early return, a nested const call, a selected failure and a failure in an inactive arm.
- [x] Check imported nominal choice identities and structural-text comparison refusal where the current evaluator cannot inspect final URL characters.
- [x] Preserve typed diagnostic codes/reasons and authored spans. Avoid generic "unsupported const expression" errors where a supported match contains a more precise failure.

**Phase 3 exit:** All supported ordinary statement pattern forms execute through the prepared const instruction machine with correct capture scope and guard order. No new pattern parser, matcher registry or general static match optimisation exists.

**Docs:** Update `docs/docs/functions/function-declarations.mtf` and the Const functions progress-matrix row for actual delivered match support. The existing canonical match contract remains authoritative.

**Review:** Independently inspect guard laziness, capture scope restoration, transitive failures and costs in nested matches. This phase may pause.

### Phase 3 outcome

**Design.**
- `ConstInstructionKind::Match` holds the scrutinee, the prepared arms and an optional default. Each `ConstMatchArm` holds the AST's resolved `MatchPattern` (cloned once at preparation), the guard and its body, compiled once.
- `default` is `None` exactly when the AST proved the match exhaustive (`MatchExhaustiveness::ExhaustiveChoice`, for choices and for options). No second exhaustiveness check exists. Selecting no arm without a default is an infrastructure failure.
- `matching.rs::select_match_arm` evaluates the scrutinee once. It tests patterns in authored order and binds a matching pattern's captures before its guard runs. The first accepting arm's body goes back to the frame machine, which pushes it as an ordinary `Program` frame, so `return`, `break`, `continue` and handler `then` unwind as before.
- The guard uses `evaluate_bool`. The pending-handler early exits mirror `if` and `assert`. Authored source cannot place a `catch` in a scrutinee or guard, so those exits are defensive.
- Captures are not removed after a guard rejects. Capture paths are interned per match, but an arm's guard and body name only that arm's own captures, which its pattern has just rebound, and no code outside the match can name them. The audit found no observable leak.

**Pattern tests.**
- `Coerced` wrappers are transparent.
- Options test `OptionNone` or `OptionSome`.
- Choices compare the variant `tag` and capture payload fields positionally by `field_index`, as HIR does.
- Literal, relational and option-value patterns call `const_eval::fold_pattern_comparison`, which sits beside `fold_numeric_comparison`. It orders same-type payloads:
  - integers through `i128`, including `U64` above `i64::MAX`;
  - binary floats, where NaN satisfies no pattern;
  - `Byte`;
  - `Dec` at one scale;
  - `Char` by scalar value;
  - `Bool`.
- Text compares for equality through `concrete_payload_text`. A resource-bearing string reports `structural_string_requires_final_text` at the pattern.
- A payload pair that cannot be compared reports the unsupported `match pattern` operation.
- `PatternValueKind::Runtime` values fold without bindings. In every source form probed they never reached a const body, including negative, relational, option, `Dec2`/`F64`/`Byte` and body-typed numeric subjects. Pattern completion folds authored literals first, so this path is defensive.

**Tests.**
- `const_function_match_selected_arm_success` was written first; before implementation it failed with MOTH-RULE-0053 at the match.
- `const_function_match_patterns_success` covers:
  - literal, text, `Char`, relational, `Dec2`, `F64` and `Byte` patterns;
  - exhaustive and guarded choices with `as` captures;
  - option none, value, relational and capture patterns;
  - nested matches and matches in range and collection loops;
  - early return, a nested const call, and a match in a catch handler using `then`.
- `const_function_match_selected_failure_rejected` and `const_function_match_structural_text_rejected` pin codes, reasons and spans.
- `const_function_match_inactive_arm_type_rejected` pins type-checking of an arm that is never selected.
- `const_function_imported_success` adds owner-module and importing-module matches over an imported choice.
- The two rejection cases that used a match now use a stepped collection loop, which is still unsupported.
- The audit compared const and runtime results for these forms and found no difference attributable to this phase.

**Cost.** Over 8,008 loop iterations in `tmp/phase3`, a nested relational match took 16.9 ms and the equivalent nested `if`/`else` chain took 22.6 ms (median of 20 release builds). A match tests one payload per arm and evaluates no condition expression.

**Existing bugs found and recorded in `docs/bug-reports.md`:**
- JS relational `Char` patterns order UTF-16 code units.
- A nested match ending without `else` fails to parse in a non-final arm.
- A value-producing match in a const body gets a vague diagnostic.
- A choice arm repeated after a guarded arm warns as unreachable.
- A collection literal argument in a `#` initializer causes a compiler bug.

None of these is a regression from this phase.

## Phase 4: Support native multi-return and multi-bind

**Purpose:** Admit multiple ordered success slots in required const calls and const bodies using the existing result-slot machinery.

**Files:** `const_execution/{mod,instructions,execute,rewrite,places,eligibility}.rs`, `src/compiler_frontend/ast/statements/value_production/expression_build.rs`, `expression_store.rs` and `const_values/store.rs`.

### Task 4.1: Locate every genuine single-slot restriction

- [x] Inspect `ConstFunctionExecutable::return_slots` and `execute_body() -> Vec<ConstValueId>`.
- [x] Find `executable_for_const_declaration`'s one-result check, `ConstCallSite`, `complete_call`'s `returns.len() == 1` check and every `success_shape().single()` in the const call path.
- [x] Separate deliberate single-value *consumers* from the arbitrary V1 restriction on *producers*. Do not widen operands that require exactly one value.
- [x] Inspect the existing multi-result trusted host fold and `commit_constant_result_block`. Reuse its `ValueBlock::LexicalScope` and ordered `ThenValue` slot mechanism rather than building tuples or a new carrier.
- [x] Confirm the complete `SuccessShape`, call provenance and source/import publication contracts survive the replacement. Check cross-module ID rules.

### Task 4.2: Implement multi-slot required call replacement

- [x] Extend const declaration preparation to accept two or more ordinary immutable success slots, retaining existing fallible/Never/zero restrictions.
- [x] Make the const call-site and result validation represent the full resolved success shape rather than a single `TypeId`.
- [x] Validate the executed slot count and types against the prepared executable and call shape before committing any replacement.
- [x] Stage and publish all result values transactionally through existing value-store operations. Commit the ordered result block using `commit_constant_result_block`.
- [x] Preserve per-slot provenance and the call's authored metadata. Do not merge result values into one synthetic aggregate.
- [x] Keep ordinary multi-result functions as ordinary runtime calls in every profile; release-only eligibility no longer exists.

### Task 4.3: Evaluate multi-slot producers at receiving sites

- [x] Inspect `ConstInterpreter::evaluate_expression(..) -> ConstValueId`, `evaluate_expression_inner`, `execute_return`/`evaluate_return_values` and `values.rs::fold_to_constant`. Record where one `ExprId` is wrongly assumed to yield one slot. In particular, `return pair()` contains one producer expression but can supply multiple ordered slots.
- [x] First add failing regressions for `return pair()`, forwarding a nested multi-result call and a multi-bind consuming a single multi-result producer. Cover valid value-producing `if`, `match` and `catch` blocks with multiple result slots where ordinary AST receiving rules permit them.
- [x] Introduce the smallest receiving-site operation that evaluates **one producer once** and extracts all values according to its resolved `SuccessShape`. Reuse the current prepared frames, value store and evaluation/handler behaviour. A narrowly scoped `evaluate_result_slots(..) -> Vec<ConstValueId>`-style helper is acceptable if it removes duplicate logic, but avoid a second generic evaluator.
- [x] Keep `evaluate_expression` and scalar operands single-valued. Route return forwarding and multi-bind through multi-slot receiving without creating tuples or flattening one slot into many accidental expressions. Preserve lazy branch selection, catch/handler behaviour and the existing effect/work budget.
- [x] Validate slot shape, ordering and types before target mutation or AST publication. Keep the existing `commit_constant_result_block` as the only AST multi-slot replacement path. Examine alias-sensitive updates and transactional failure for receiving sites separately.

### Task 4.4: Implement multi-bind inside const bodies

- [x] Inspect the existing `NodeKind::MultiBind { targets, value }` and `MultiBindTarget` representation. Confirm declaration versus update targets, aliases and recorded `ValueMode`.
- [x] Add one prepared multi-bind instruction that retains the already-typed producer and target facts.
- [x] Use Task 4.3's multi-slot receiving operation to evaluate the producer exactly once and preserve ordered results. Collect all RHS values before any existing target is updated.
- [x] Use existing `FrameBindings`, place projection and alias-write paths for declarations and assignments. Validate target shape and avoid partially installing new bindings on a failed result producer.
- [x] Preserve ordinary left-to-right target update semantics where aliases overlap. Do not introduce a new binding store, runtime tuple or mutable-parameter capability.

### Task 4.5: Preserve returns and receivers

- [x] Inspect `ConstInstructionKind::Return(Vec<ExprId>)` and the ordinary return-production owner for any assumption that one expression equals one result slot. Route each expression through Task 4.3's receiving-site operation and append its validated success slots in order, especially for `return pair()`.
- [x] Support returning multiple scalar/composite values, nested multi-result producers and value-producing blocks permitted by current AST receiving rules.
- [x] Preserve the distinction between a multi-result producer and a single-value operand. Ensure a single-target binding rejects a multi-result call with the existing arity diagnostic.

### Task 4.6: Assert observable parity

Start with `const_multiple_result_slots_rejected` and `adversarial_multi_bind_in_loops`. Replace a rejection fixture only when its formerly invalid behaviour becomes supported. Keep `const_zero_result_function_rejected` as a separate current boundary.

- [x] Add a multi-result const helper with two and three distinct slots and assert exact output.
- [x] Cover a nested multi-result const call, `return pair()` forwarding, a multi-result producer used in multi-bind and cross-file import/publication. Check permitted multi-result `if`/`match`/`catch` receiving sites.
- [x] Cover new declarations, existing mutable updates, mixed targets, alias-sensitive updates and evaluated-once effects on local state. Confirm a failing producer or invalid shape never partially commits bindings or published values.
- [x] Assert incorrect slot count/type diagnostics at receiving sites and no silent first-slot truncation.
- [x] Verify ordinary multi-return calls remain runtime calls in both profiles while required const multi-return calls execute during Stage 4.

**Phase 4 exit:** Required const functions support multiple ordered ordinary success slots, expression-to-result forwarding and const-body multi-bind. Multi-valued producers evaluate once at receiving sites, while scalar consumers remain single-valued. The existing native result block remains the only AST multi-result replacement representation. Zero-result, fallible, Never and generic const signatures remain outside scope.

**Docs:** Update `function-declarations.mtf`, `returns-and-multiple-values.mtf` only where the const-specific contract warrants clarification and the progress matrix. Confirm required const semantics remain accurate and ordinary-call release eligibility has no remaining documentation or implementation owner.

**Review:** Audit every residual `single()` assumption in affected producers, return forwarding, aliases and cross-module slot validation. This phase may pause.

### Phase 4 outcome

**Finding.** Value-producing `if`, match and lexical-scope blocks were unsupported in const bodies even with one slot; only `catch` executed. Multi-slot receiving needed value-block execution in general. Value blocks occur only as the root of a declaration, assignment, multi-bind or return, so the receiver is known when the body is compiled.

**Design.**
- Declarations accept one or more success slots; zero slots, fallible and Never contracts stay rejected. `labels::MULTIPLE_SUCCESS_RESULTS` is gone.
- `complete_call` validates the slot count and every slot type against the call's success types. `values::call_result_inheriting_provenance` replaces one slot with a `Constant` and several with `commit_constant_result_block`, the host folder's replacement. The occurrence keeps the union of call, argument and evaluated provenance, as the one-slot path did.
- The body compiler (`BodyCompiler`) compiles a value block at a receiving site to `ValueBlock { body, receiver }`: the selected-branch statements (`If`, `Match` or the flattened scope) plus a `ValueReceiver`. Branch `then` becomes `Then(values)`. Execution pushes `ExecFrame::ValueBlock(receiver)` under the body; `Then` reads every slot, unwinds to that frame and applies the receiver. `return`, `break` and `continue` unwind past it like any frame, and fallthrough is an infrastructure failure. A non-block multi-bind producer compiles to the one-statement block `[Then([value])]`, so every multi-bind receives through one path. `catch` stays an expression.
- `evaluate_value_slots` serves `return`, `then` and multi-bind. A `SuccessShape::Multiple` value goes to `evaluate_result_block`, which rewrites the producer once and reads the committed rows back. A successful catch unwraps to its handled block, and a recovering catch takes every handler slot. A discarded multi-result call in a statement also takes this path. `evaluate_expression` stays scalar.
- Multi-bind declarations insert and clear aliases. Assignments go through `write_through`, which is split out of `execute_assign` and follows mutable aliases to their referent, left to right after every slot is read.
- The recovery marker became `inherited_handler_values: Vec<ConstValueId>`. `execute_frames` now gives each instruction stream its own recovery context: a statement depth, no inherited handler and no recovery. Each callee and handler body consumes its own recovery at its statement depth. This fixed two older leaks, both found in review: a callee's catch recovery escaping into its caller (`recover_inner(9) + 1` gave 50, now 51), and a handler statement's recovery corrupting the next handler statement (gave 5, now 6). Total expression depth still bounds native recursion across calls.

**Tests.**
- `const_function_multi_return_success`, written first, failed with MOTH-RULE-0053 "with multiple success results". It covers two- and three-slot helpers, `return forward(..)` forwarding, declaration and mixed multi-bind, value `if`/match returning and binding several slots, multi-bind in loops, single-slot value blocks, a branch `return`, `break`/`continue` leaving a block, and top-level runtime multi-bind from a required call.
- `const_function_multi_bind_staging_success` covers read-before-write swaps, alias write-through, a value block inside a handler with `return`, multi-slot catch success and recovery, including inside another handler, static selection, a composite slot, cross-call recovery isolation and a discarded multi-result call.
- `const_function_multi_return_arity_rejected` pins the existing MOTH-RULE-0087 receiver diagnostic. `const_function_imported_success` adds an imported multi-result const used both in a const body and at top level.
- `const_multiple_result_slots_rejected` was removed because its behaviour is now supported. Ordinary multi-return calls stay `Invoke` calls in debug and release.

**Measurements** (release, 15 runs). `tmp/phase3` match and if workloads are unchanged: 15.8 ms vs 16.7 ms, and 21.9 ms vs 21.7 ms. A 20,000-iteration loop making one two-slot call costs 62 ms, against 59 ms for two one-slot calls. The extra cost is the result-block rows and frames per call. Like one-slot calls, user const calls still take the ordered rewrite and commit AST rows on each execution. Those rows are not charged to the const budget. QA measured peak memory with `moth check` on 20,000 iterations: 48.5 MB for a two-slot multi-bind, against 18.5 MB for a one-slot binding. A two-slot call commits four rows (scope, `then` and two constants), where a one-slot call commits one. Phase 7's workload pass owns the decision on this pre-existing unbudgeted row growth: charge the committed rows, or carry statement-root slots out of the rewrite without rows.

**Limits.** `break` and `continue` cannot leave a `catch` handler body, which runs on its own frame stack; this limit predates the phase. `then pair()` is rejected by the AST, so multi-result producers inside value blocks go through a branch multi-bind.

**Existing bugs found and recorded in `docs/bug-reports.md`:**
- A runtime multi-bind through a mutable alias is rejected with an internal temporary name.
- Historical finding: const bodies skip runtime borrow rules. The design now deliberately excludes runtime borrow analysis and instead requires interpreter-owned mutation permissions and dynamic generation checks.

## Corrective checkpoint before Phase 5

Preserve the profile-independent frontend and the single required const evaluator. The owner has approved a different access contract for exclusively compile-time functions. The previous validation-only HIR proposal is superseded, not deferred work. The independently verified result, recovery, folding and parity corrections below remain required.

Permanent semantic authority: `docs/developer-docs/memory-management/access-and-aliasing/access-and-aliasing.mtf` > `Const execution: generational access validation`. Compiler ownership: `docs/compiler-design-overview.md` > `Required const execution and deterministic eager folding`. Read both before choosing a representation. The checkpoint does not change runtime memory semantics.

### Access design gate (resolved)

The Phase 4 binary accepted writes through immutable parameter aliases, overlapping shared aliases, returned aliases and multi-return aliases that ordinary HIR rejected. The following remains invalid:

```moth
probe #|input I32| -> I32:
    slot ~= input
    ~slot = slot + 1
    return slot
;
```

Acquiring mutable access from a read-only place is invalid regardless of generation. Reject statically evident misuse through existing access/type owners. Dynamic permission checks enforce the same law where actual referent identity is needed.

Multiple overlapping mutation-capable aliases are deliberately legal in const execution. A new immutable reference records the current generation of its resolved storage. A successful overlapping mutation invalidates that reference. A stale reference may remain stored or go out of scope without error, but reading, copying, projecting, passing, returning or otherwise using it is invalid. Acquiring an alias from a stale reference cannot refresh it. A fresh reference acquired through a valid source after a write is valid. Only actual execution checks generations, so an unused const declaration or unselected path can contain a dynamic failure. This is comparable to parameter-dependent checked arithmetic or an assertion, not a relaxation of static typing or eligibility.

The interpreter executes sequentially, with no event loop, concurrency, async, IO, external mutable state, arbitrary host execution, static lifetime topology, REC or runtime ownership planning. It retains bounded storage for one required evaluation, including nested calls, and releases that state at its owning boundary. Keep mutable parameters, mutable receivers, generic const declarations, recursive call graphs, zero success and error-slot/Never contracts outside this slice. Preserve existing unsupported declared-region and cyclic graph forms. Do not introduce a future mutable-exclusivity flag or capability-generation table.

### C1. Audit live storage, projection and publication contracts

- [ ] Read the routed authorities, executor, shared fold and TIR owners. Trace `state.rs::FrameBindings` and `ExecutionState::aliases`, `execute.rs::{execute_body,execute_bind,receive_values,write_through}`, `places.rs::{direct_alias_target,expand_alias_target,canonical_alias_root,write_binding_value}`, `values.rs::{project_constant_value,assign_constant_projections,fold_to_constant}`, `rewrite.rs::complete_call`, captures, loop bindings and imported body materialisation. Record actual supported shapes rather than assuming all ordinary operations are supported.
- [ ] Establish a small reference/slot contract for direct aliases, fresh binding cells, allocation identity, scalar places, aggregate children, returned projections, detached stored values, assignments and rebinds. Compare ordinary legal cases against canonical source semantics and runtime output. A result has a fresh binding even if it shares an allocation. A mutable alias writes through. Rebinding an independent slot does not mutate its old allocation. No-shadowing simplifies lookup, but a declaration path is not a dynamic cell/allocation identity. Calls, loop iterations and scope re-entry issue distinct instances.
- [ ] Distinguish a read-only slot, the permission of a reference to storage and a writable fresh slot holding a value from a shared result. Rebinding that slot must not grant mutation of an immutable referent or launder stale access. Derive the exact permission rules from the ordinary source contract, including stored children. If ordinary rules are incomplete or contradictory, present a concrete example and ask the owner before inventing a permission upgrade or implicit copy.
- [ ] Resolve projection overlap through actual storage identity. Test ancestor/descendant, independent siblings, nested paths, two paths to one stored child, replaced parents, collection reads and structural edits. A tree of authored field names is insufficient for a graph with shared children. Choose a bounded representation that detects overlap through retained references without scanning active aliases. Structural collection edits may conservatively invalidate collection-origin projections. Pin the distinction between detached values and interior projections.
- [ ] Establish the publication boundary. Nested calls and value-block/multi-bind receivers carry live references, not independent folded snapshots. Outer required evaluation freezes only validated finite data through the existing constant owner. Test whether existing folded projection/materialisation preserves result-to-result and internal sharing when materialised into runtime bindings. The owner has excluded live interpreter references escaping to runtime, but has not approved erasing observable allocation relationships across that boundary. If existing folded data cannot represent a required relationship, present a minimal reproducer and ask the owner to choose between explicit snapshot-boundary semantics and a compatible owned projection. Do not silently duplicate allocations, generalise public constants to cyclic graphs or add an ABI.
- [ ] Audit all read paths, particularly expression substitution, fold callbacks, template interpolation/captures, collection/pattern reads, named argument order, lazy logic, handlers and default values. An immutable cached ConstValueId or copied binding map must not bypass a stale-reference check or read pre-mutation state. Identify any ordinary HIR operand-order bugs separately rather than importing them.
- [ ] Review the prepared instruction vocabulary and the permissive default in `eligibility.rs::classify_const_expression`. Keep one static declaration eligibility owner, including never-called declarations and unselected authored paths. Make operation coverage exhaustive or otherwise prove new unsupported expression variants cannot silently slip through. No second type checker.
- [ ] Decide the exact Rust layout and accounting from these traces. Prefer a bounded dense arena with typed attempt-local IDs and sparse projection metadata over per-reference objects, a reference registry, collector or general object system. Reuse the existing immutable typed value vocabulary and evaluator for numeric, cast, comparison, Text and TIR operations. If the storage model would require a substantial second value language, pause for evidence-based alternatives. Record C1 findings and resolve any outstanding observable contract before C2.

### C2. Replace alias refresh with one live storage owner

- [ ] Write red tests for multiple mutable aliases, immutable-parameter mutation rejection, scope/call/iteration freshness, projected write-through, rebind, nested/imported calls, return and multi-result aliases and copies. Assert exact values and diagnostics. Use ordinary-function controls where contracts agree and explicit contrasts where they differ.
- [ ] Make frame bindings resolve through one live storage/handle owner. Separate binding-instance identity from allocation identity and projected location. Share storage across nested frames in one evaluation. References may outlive the lexical binding that named them. Do not use donor-local indices or static declaration paths as storage roots.
- [ ] Preserve relationships through projections, aggregate insertion, arguments, match and loop captures, returns, value blocks, multi-bind and collection reads. Explicit deep copies preserve internal topology but share no mutable storage with the source. Elide storage identity only when provably unobservable, not merely because payload IDs match.
- [ ] Route read, reference acquisition, derivation, write commit and copy through the live owner. Shared fold operations consume resolved values at the authored evaluation point. Do not reconstruct whole expression graphs or freeze nested results to transport them. Preserve native ordered slots, frame outcomes and lazy handlers.
- [ ] Delete `ExecutionState::aliases`, alias refresh scans, root-chain chasing, duplicate snapshots and superseded helpers once their consumers move. Keep legitimate direct value folding and immutable constant projection. No compatibility fallback, validation-only HIR or public borrow summaries.
- [ ] Retain storage under the bounded owning evaluation. No inferred regions, affine cleanup, REC, alias release counts or tracing collector. Charge actual storage, retained values, edges and work. Do not silently support cyclic graphs or broaden declared-region support. If admitted constructs can form a cycle, reject the unsupported graph deterministically before committing or publishing it. Bound graph checking and cleanup.

### C3. Implement projection-aware mutation generations

- [ ] Use a checked attempt-local mutation clock or equivalent generation facts. Immutable references record their acquisition witness for resolved storage and relevant projection/containing-edge identities. Derivation preserves validity and cannot refresh a stale reference. Mutable handles remain valid across overlapping writes when their target still exists and permits mutation. Target existence is separate from access invalidation.
- [ ] Update generations only at a successful semantic mutation commit after argument/RHS evaluation, bounds, type/access and capacity checks. Failed operations commit no invalidation. Mutating then restoring equal content still invalidates old readers. Distinguish slot rebind from allocation mutation.
- [ ] Implement same-place, ancestor/descendant, independent sibling and shared-child overlap. A write through one path invalidates overlapping immutable references reached through another. Deriving from an already stale aggregate is invalid even if the intended field would otherwise be disjoint. Sparse direct-write/subtree-write generations are a candidate for structural locations, not a complete shared-graph algorithm. Handle parent replacement and element identity explicitly. No raw-path-only equality or blanket family invalidation for provably independent struct fields.
- [ ] Check every actual use, including copying, passing, returning, storing/forwarding, deriving, builtin and TIR observation and pattern captures. Dropping a stale unused handle needs no validation. Reject invalid targets even for mutable handles. Centralise acquisition, use and mutation commit so later mutable invalidation could be added without replacing storage. Do not implement that stricter policy now.
- [ ] Diagnose stale use through a typed non-catchable source diagnostic at the invalid use, with acquisition and invalidating-write labels where available and the bounded const call chain. Keep donor span identity. Distinguish access failure, unsupported operation, budget refusal and infrastructure/compiler invariants.
- [ ] Test no-later-use acceptance, stale read/derive/copy/pass/return/store rejection, fresh reads, multiple writers, disjoint sibling reads, ancestor/descendant/whole replacement, shared children, collection edit/read rules, failed mutation with recovery, equal-value writes, selected/unselected paths and never-called bodies. Static eligibility/type errors in unselected code remain errors. Add focused generation-overflow/capacity and identity tests. Charge graph traversal, access metadata, copy, freeze and TIR against existing deterministic limits before work/growth. No new configuration or parallel budget.

### C4. Integrate required evaluation and close publication

- [ ] Run the same access owner in type-level evaluation, initialisers, defaults, const fragments, TIR, imported/package const calls and any short compiler service that actually executes source const bodies. Config currently excludes source function declarations and imports. Keep it that way, without HIR. Direct template services invoking provider const functions use the same access checks.
- [ ] Implement the C1-approved outer publication contract. Freeze validated finite data into the existing `ConstValueStore`/owned public projection, preserving types, structural resource strings, configuration/project provenance, slot order and const-record restrictions. Never publish storage IDs, generations or borrowed graphs. Release temporary state at the owning boundary. A failed required evaluation publishes no partial semantic success.
- [ ] Keep imported callable materialisation on stable origins and retained syntax/context. No provider HIR or const borrow summaries. Check private helper chains and donor diagnostic sites. Ensure provider semantic changes invalidate consumer folded results through existing interface/materialisation compatibility facts, not another fingerprint domain.
- [ ] Extend the real build-service parity fixtures with valid and invalid const access, imports/packages, structural results and type-level evaluation. Verify identical debug/release diagnostics, output and runtime handoff. No const function, target requirement or link edge reaches runtime. Preserve the fail-closed runtime HIR guard. Runtime borrow rules still reject conflicting mutable aliases.
- [ ] Retain the ordinary multi-bind false-conflict report as independent runtime work. The new const policy neither fixes nor justifies that bug. Fix separately in its ordinary HIR/borrow owner with approval, preserving simultaneous RHS evaluation and the distinction between fresh slots and fresh allocations.

### C5. Checkpoint verification and simplification

- [ ] Repeat the existing 20,000-iteration scalar and multi-result workloads against the independent checkpoint. Add alias/projection-heavy shared-child, write and copy workloads. Record expression census, time and RSS and bounded live-storage/edge/projection accounting if needed. Attribute growth to real work, not alias holders or unnecessary frozen intermediates. Keep the ordinary no-const control. Do not invent a performance gate from one noisy run.
- [ ] Review code size, duplicated representation, unmetered work, stage boundaries and dead helpers/tests. No const HIR, static solver, event loop, speculative evaluator, allocator framework, GC, reference-count registry or future-exclusivity scaffolding. Reuse the current frame, value operation and TIR owners.
- [ ] Update progress for implemented support, retain canonical design and retire the const-access bug report only when replacement reproductions pass. Keep independent reports/audit records accurate. Run affected Rust and integration tests, relevant feature lanes and `just validate` at the checkpoint. Record exact evidence, perform the Slice review and commit independently. `just validate-full` remains the final merge-readiness gate.
- [ ] Resume Phase 5 only after C1-C5, all observable contract decisions and integrated checkpoint validation are complete. The owner-approved generational model is final for this slice. Raise only specific unresolved observable contracts, not the superseded question of whether to use runtime borrow checking.

### Independent corrections

- Prepared calls check every stored slot against the resolved call and executable contracts before
  publication. Interpreted receivers consume the ordered values directly; ordinary multi-bind
  uses the receiving operation without a synthetic frame. Nested/local and imported forwarding
  fixtures check exact rendered values.
- Recovery and nonlocal transfer share one frame outcome. A catch is active for its protected
  evaluation, not looked up again under each rebuilt expression ID. Handlers preserve a distinct
  zero-slot recovery; `break` and `continue` use ordinary loop unwind. Regressions cover nested
  and multi-slot recovery, returns, loops, projected receivers and hard memory refusals.
- Pattern comparisons use the same numeric and text operations as binary comparisons. JS
  relational `Char` operators and patterns compare Unicode scalar values. Known-invalid
  conversions remain diagnostics; a conversion that fails on a bound value uses the prepared
  handler. Concrete aggregate parameters complete literal arguments at `#` initializers.
- The normal build-service parity tests compare canonical package interfaces and generated
  sidecars, with exact synthetic scope names in sequential and concurrent builds.

### Result ownership measurement

Matched release CLI builds with `benchmark_counters`, before and after the result
and loop-binding changes. Each uses the recorded 20,000-iteration workload. Rows
are the main module's pre-freeze expression census, wall time is the median of
15 `moth check` runs, and RSS is the maximum macOS resident size across them.

| Workload | Phase 4 rows | Checkpoint rows | Phase 4 wall | Checkpoint wall | Phase 4 peak RSS | Checkpoint peak RSS |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Two single results per iteration | 60,032 | 31 | 64.66 ms | 63.03 ms | 29,622,272 B | 26,148,864 B |
| One two-slot result per iteration | 80,030 | 29 | 67.76 ms | 56.09 ms | 38,649,856 B | 26,181,632 B |

Direct result delivery alone removed 40,000 and 60,000 expression rows respectively.
The 20,000 common residual rows were range-iteration bindings, now delivered as
already-typed values. Collection reads and iteration hold the immutable collection ID
instead of copying its items. Fresh values remain subject to the existing allocation
ceiling. Rebuilt expression child slots and TIR binding rows consume the existing
retained-slot allowance; neither has a separate budget.

### Review and verification

The Slice review checked the remaining result and handler boundaries for duplicate
representations, avoidable copies, shared semantic rules and stale comments. New cases
assert exact rendered results or diagnostic identities and source sites, not acceptance alone.
Targeted checks passed: 341 const Rust tests, 131 const integration checks and both backends
of the ordinary optional cast recovery case. `just validate` passed, including 3,856
integration checks and the docs build. `just validate-full` remains a merge-readiness gate.

These are historical checkpoint results. They verify the independent corrections, not the newly accepted generational access model. C1-C5 must supply new verification.

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
- [ ] Evaluate the separately stored receiver once, before explicit arguments, then bind it as shared parameter zero through the C1-C5 live storage/access owner. Preserve actual identity, generation witness, permissions, provenance and normal method resolution without a synthetic second evaluation.
- [ ] Reuse `ConstInterpreter::execute_body` and existing budgets. Do not build separate method frames, method registries or a method-specific value representation.
- [ ] Reuse Phase 4 result-slot completion for a const method returning multiple values.
- [ ] Confirm imported nominal methods execute from stable semantic interface facts without donor-local `ExprId` or `TypeId` leakage.
- [ ] Keep ordinary receiver methods as runtime calls in every profile; explicitly marked const methods remain required compile-time calls.

### Task 5.4: Validate method semantics

- [ ] Add or extend cases for nominal struct and choice receivers, explicit parameters, nested const calls, composite returns and multiple returns.
- [ ] Cover imported receiver types, method visibility, runtime receiver refusal, mutable receiver rejection, generic rejection and invalid ownership. Add positive/negative regressions for every declaration, resolution, eligibility and call-rewrite gate, including ordinary runtime methods on data-only const records.
- [ ] Cover template/string values returned by a const receiver method without making runtime templates legal.
- [ ] Check receiver and argument evaluation order, failures and source provenance. Cover projected receivers, stale receiver access, returned aliases, imported methods and copies through the same dynamic checks.

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
- [ ] Preserve the ordinary evaluation and captured-length contract and accepted const dynamic projection/access rules. Do not let a structural edit silently retarget a live projection or bypass validation.
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
- [ ] Review match guards/captures and multi-result aliases for control-flow leaks, dynamic binding identity and preserved generation witnesses. Check that no live interpreter storage escapes.
- [ ] Review immutable method publication against cross-module identity rules.
- [ ] Run selected integration cases and Rust tests, then `just validate` at the integrated checkpoint.
- [ ] Run `just validate-full` on the final integrated tree before merge readiness. Report exact commands and results, never unrun gates.
- [ ] Delete temporary/obsolete code and update `index.md` only when significant files or responsibilities changed.

**Phase 7 exit:** The accepted semantics have landed or carry a user-approved explicit deferral, required-const versus ordinary-runtime call boundaries remain correct, final benchmarks and validation are recorded and permanent design references contain every enduring decision.

## Documentation and status synchronisation

Documentation ownership follows repository `AGENTS.md` and `docs/roadmap/roadmap.md`. A roadmap plan is an implementation work item, not the permanent owner of language semantics.

### This PR and foundation implementation

This PR consolidated and activated the plan at the top of the roadmap, retired
the separate release-folding plan and recorded optional post-HIR folding as
deferred in the progress matrix. Foundation Phases A-C deleted the release-only
AST behaviour, replaced the transitional wording with delivered facts, removed
the release budget and eligibility advice and closed R1 after the preserved
probes and permanent parity gate passed. Update `index.md` only if a file or
ownership boundary actually changes.

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

Const functions execute against one typed AST/value foundation. The interpreter owns bounded live storage and projection-aware generational access validation, preserving ordinary reference and copy semantics with deliberately relaxed mutable alias exclusivity. Runtime static borrow and lifetime validation remain mandatory for executable bodies. Value-only computation no longer requires a temporary replacement expression. Necessary call/handler/template execution remains explicit and lazy. Match statements, multiple success slots, multi-bind and eligible immutable receiver methods work through established compiler owners. Stepped collection loops have either landed with full parity or a concrete deferred decision. Optional whole-function speculation remains removed, profile-independent HIR is guarded by tests, packages demonstrate useful const helpers and the codebase carries less redundant state and rewriting.