# Const functions foundation gate report

Companion evidence log for the completed const-functions implementation plan. It records
decisions, commands and final gate results, not a second architecture authority.

STATUS: COMPLETE - implementation, audit and `just validate-full` accepted.

## Phase 0 baseline

```text
BRANCH: const-functions
REVIEWED_HEAD: 625de94843c5330b4af6ff96abc1e98cbcb96dab
WORKTREE: clean
TOOLCHAIN: rustc 1.99.0 (b940084d7 2026-09-28), cargo 1.99.0 (5f94df478 2026-08-27)
FEATURES: default workspace features (none of the compiler instrumentation features enabled)
NODE: v26.13.0
PROFILE: dev for tests, release for cli benchmarks
```

### Confirmed data flow

| Stage | Owner | Result |
| --- | --- | --- |
| Stage 1-2 references | `src/compiler_frontend/headers/parse_file_headers.rs`, `src/compiler_frontend/utilities/token_scan.rs` | Retained tokens, initializer references and shallow binding names before AST. |
| Stage 3 order | `src/compiler_frontend/headers/constant_dependencies.rs`, `src/compiler_frontend/module_dependencies.rs` | One declaration graph and `SortedHeaders`. Const bodies contribute token-scanned edges. |
| Stage 4 environment | `src/compiler_frontend/ast/module_ast/environment/builder.rs` | Import projection, nominals, aliases, constants, ordinary signatures and receiver catalog. |
| Stage 4 const preparation | `src/compiler_frontend/ast/module_ast/environment/type_resolution.rs` | Signature, canonical body parse and `ConstFunctionExecutable` install before later constants fold. |
| Stage 4 required folding | `src/compiler_frontend/ast/expressions/eval_expression/evaluator.rs`, `src/compiler_frontend/ast/const_values/const_execution.rs` | Typed AST expression rewrite and bounded frame execution. No HIR exists yet. |
| Stage 4 finalisation | `src/compiler_frontend/ast/module_ast/finalization/finalizer.rs` | Validates retained const bodies, clears them, freezes expressions and builds `Ast`. |
| Stage 5 HIR | `src/compiler_frontend/module_compilation/stages.rs`, `src/compiler_frontend/hir/hir_builder.rs` | Const bodies are absent; ordinary calls become HIR call statements and `Invoke` terminators. |
| Stage 5+ | `src/compiler_frontend/module_compilation/service.rs` | Link facts, borrow validation, generated convergence, publication. |
| Publication | `src/compiler_frontend/ast/generic_functions/materialisation/preparation_freeze.rs`, `.../const_materialisation.rs` | Stable closure rows; consumer reparses provider closure into consumer-local executables. |

### Existing coverage

- 71 const-function integration cases plus `const_call_runtime_argument_rejected`.
- 12 Stage 3 const/constant ordering tests and six diagnostic-order tests.
- Four scanner reference tests and six inline executor/accounting tests.
- One provider-edit test proving changed output through two fresh compilations, not cache reuse.
- No const-function benchmark workload. Existing constant-chain and DAG workloads exercise constant expressions only.

### Baseline commands

```text
cargo test -q -p moth --lib
6996 passed; 0 failed

cargo run --quiet -p moth-cli --bin moth -- tests --tag const --terse
92/92 correct

cargo run --quiet --package xtask --bin xtask -- bench-frontend-check
suite average 135.96 ms, 1 warmup, 10 measured iterations, default thread count
evidence commit 625de9484, git_dirty false
case set changed against stored history: no comparable unchanged workloads
```

The benchmark result is a clean branch baseline for final reruns. It is not compared with
an older revision because the current workload set has no unchanged comparable case.

### Initial decisions

- Confirmed defect: a deferred const body can be parsed by emission after early preparation
  returned without installing an executable.
- Confirmed defect: catch handlers are compiled during eligibility and recompiled every time
  a selected handler executes.
- Confirmed defect: `charge_new_value_text` rescans reachable graphs for each new root, so a
  shared child can be charged repeatedly.
- Suspected defect: projected mutable aliases are not represented by `direct_alias_root`.
  Reproduce through ordinary Moth semantics before changing the storage model.
- Tested and disproved: Stage 3 const-body local filtering already covers typed locals, loop
  binders, multi-bind targets, map keys, named arguments and renamed payload fields.
- Accepted limitation: no build semantic cache or persistent fingerprint reuse exists at this
  boundary. Phase 5 must test the current in-memory path and document the deferred gate.
- Accepted limitation: no const-function benchmark fixture exists. Add only a bounded
  representative workload if final performance evidence needs one.

## Phase 1 architecture decision

```text
STATUS: ACCEPTED
SEAM: AST-owned candidate rewrite immediately before expression freezing and HIR lowering
EXECUTOR: the existing required-call typed-AST executor
REJECTED: HIR-owned ordinary-body execution
```

### Probe A evidence

A disposable test-only pass classified ordinary source functions whose reachable expressions were
arithmetic, locals, copies, coercions and calls to the same safe set. It prepared their executable
rows with `executable_for_body`, rewrote call expressions through the existing
`rewrite_with_bindings` walk immediately before `AstExpressionBuilder::freeze`, then let ordinary
HIR lowering and validation run. The pass and its temporary feature were removed after measurement.

Probe input, with `pure` exercising a mutable local and a nested eligible call:

```moth
inner |value I32| -> I32:
    return value + 1
;

pure |value I32| -> I32:
    local ~= value + 0
    ~local += inner(1)
    return local * 2
;

result = pure(20)
~[: [result] ]
```

The failing-assertion probe replaced `pure` with a `bad` body containing
`assert(value > 10, "boom")` and called `bad(1)`. The impure probe replaced it with an
`impure` body containing `io.line("side-effect")` and called `impure(4)`.

Historical commands used a temporary `release_probe` Cargo feature forwarded to `moth`:

```text
cargo run -p moth-cli --bin moth -- build <project>
cargo run -p moth-cli --bin moth --features release_probe -- build <project> --release
extract every <script> block from each index.html
run the blocks under a Node harness that stubs document.getElementById and records
insertAdjacentHTML and console events
```

The test-only pipeline probe inspected HIR directly: release `local_call_count` was `0` for the
eligible nested call and debug was `1`; both used the ordinary `lower_hir` path. For the CLI
comparison, the evidence is generated-output evidence: release emitted a fixed `42`/`44` binding
with no private call, while debug emitted the private call. No textual HIR dump was archived. The
module feature and probe code were then removed.

Measured with the HTML backend and Node harness, debug versus release:

| Program | Debug | Release |
| --- | --- | --- |
| `pure` with mutable local and nested eligible call, called as `pure(20)` | rendered ` 44 ` and retained the source call | rendered ` 44 ` with the call replaced before HIR |
| `bad` with failing `assert(1 > 10)` | observed `boom` at runtime | observed `boom` at runtime; the call remained |
| `impure` with `io.line` | emitted `side-effect` and rendered ` 5 ` | emitted `side-effect` and rendered ` 5 `; the call remained |

The same HIR path lowered, validated, borrow-checked and emitted both profiles. No HIR-to-AST
adapter and no second parser or value domain were introduced. This evidence covers successful
value replacement and runtime refusal parity for these examples. It does not prove that every
authored borrow or escape diagnostic survives replacement; that is an explicit Slice 7 gating
condition below.

### Probe B assessment

HIR does contain ordinary function bodies: `HirFunction` names an entry block and parameters, and
`HirModule` owns blocks, statements, terminators and expressions with `LocalId`/`HirValueId` and
explicit result contracts. It does not contain the prepared source-body representation the executor
consumes. `ConstFunctionExecutable` holds `AstNode` instructions, `ExprId`, `PlaceId`, parameter
`PathId`s and expression-owned handlers. HIR calls are split between `HirStatementKind::Call` and
fallible `Invoke` terminators with destination locals.

A HIR-owned attempt therefore needs one of two substantial owners:

- a HIR-to-AST value/place adapter that maps every HIR local, place, expression and result contract
  into the AST stores, reconstructs a `ScopeContext` and TIR bindings, and preserves handler and
  provenance facts; or
- a second HIR interpreter for numeric, cast, collection, assertion and template semantics, which
  duplicates the shared folders and violates architectural rule 2.

Neither API exists. The source inspection estimate is over 1,500 lines of adapter/interpreter code
plus new retained state, compared with a narrow candidate classifier and one expression rewrite at
the existing AST seam. The adapter is a second semantic owner, not a narrow rewrite. Rejected.

### Accepted ownership

- **Eligibility:** one shared const-safety classifier over canonical typed expressions and prepared
  executable rows. Required declarations turn refusal into an error; speculation turns refusal into
  an unchanged call.
- **Candidate preparation:** during Stage 4 body emission while the canonical typed body and scope
  facts are live. Prepare each ordinary body at most once; do not retain a second AST.
- **Execution:** the existing bounded frame executor, `ConstValueStore` and TIR folding owner.
- **Replacement:** a transactional AST expression rewrite at the finalization barrier before
  `AstExpressionBuilder::freeze`. A failed attempt leaves the original `ExprId` and runtime call.
  Debug never invokes this rewrite. Required const calls keep their earlier Stage 4 path and are
  profile-independent.
- **Validation:** AST typing, arity, access-marker, terminality and failure rules run on the
  authored call before replacement. `check_borrows` consumes HIR after replacement, so the candidate
  policy must prove that removal erases no borrow obligation. A release candidate is eligible only
  when every argument is a compile-time constant value (not a runtime place), callee parameters are
  shared value parameters, the body is pure and the result is fresh. Mutable-reference parameters,
  retained-reference results, host effects, external state and any operation that cannot prove those
  facts remain runtime calls. Calls that cannot satisfy the proof are never removed, so borrow
  validation still sees them. HIR validation, borrow validation, reachability, link facts and
  backend emission run on the final AST/HIR; no post-HIR fact is invalidated because HIR has not
  been produced yet.
- **Scope:** candidate rows retain only the execution facts the supported subset needs. V1 release
  candidates reject operations that require a live body-local template scope or host dispatch until
  the seam proves those facts can be retained without a scope graph. This is a policy limit, not a
  second execution path.

### Stop conditions

- If a release candidate needs body-local scope facts not retained by the finalization barrier, do
  not reconstruct a parallel scope system. Either retain the exact narrow fact during emission or
  leave that candidate ineligible.
- If the AST rewrite cannot prove that removal preserves every source failure, effect and borrow
  obligation, do not remove the call. A candidate that cannot satisfy the borrow-erasure proof above
  must stay runtime until a concrete ordering fix exists.
- If classification needs a different operation allowlist from declaration validation, stop and
  repair the shared classifier instead of adding a second policy.

### Phase 1 validation limitation

The probe was removed before commit and its generated project was temporary. The recorded sources,
historical commands, HIR call-count assertions and Node event outputs are historical observations,
not a runnable harness in the current tree. Restoring the probe would be required to repeat them.
No negative borrow/escape differential was run in Phase 1; Slice 7 must add one for every accepted
candidate class before the release rewrite is enabled.

Gate 1 passes only for the seam decision, not for a production release pass. Slice 7 remains paused
until corrective Phases 2-8 close.

## Corrective Phase 2 evidence

Phase 2 tightened the Stage 3 dependency and Stage 4 declaration-preparation boundary without
changing the accepted executor seam.

### Dependency graph

Const-function bodies continue to use the prepared-source token scan, then remove every
body-local binding spelling collected from declarations, multi-bind targets, loop bindings, match
payloads and catch-local declarations. Moth's no-shadowing rule makes that filter complete: a
body-local spelling cannot name a top-level graph participant. Qualified selections retain their
base reference and dot member for canonical import resolution; field and choice member spellings
are not separate graph nodes. Ordinary function bodies and the implicit start remain excluded.

Two regressions cover the boundary:

- `const_function_field_name_no_dependency_success` uses a const function whose parameter field is
  named `x` while a later constant `x` calls that function. A false edge from the field spelling
  would create a cycle and fail compilation; the case builds and renders `x=7`.
- `const_function_core_const_eval_parameter_success` calls trusted `@core/text.length` with a
  parameter value inside a const function. It proves the operation scan does not mistake a
  const-evaluable host operation for a runtime source call.

### Structured declaration refusal

`ConstBodyRejection` now carries the first unsupported statement operation and its authored span.
`compile_body_mode` propagates it through nested lexical scopes, branches, loops and catch-handler
bodies. Local Stage 4 declaration and expression emission convert it into one
`MOTH-RULE-0053` diagnostic with the operation name and the unsupported site as the primary label;
uncalled bodies and handlers the current evaluation would not select are still rejected before any
call. Imported materialisation treats the condition as an internal invariant because provider
publication already rejects an unsupported declaration.

The operation-level classifier is shared at the typed-expression boundary:
`classify_const_expression` names expression forms the bounded executor cannot evaluate in any
context, while call-target purity and binding safety remain policy facts for the release candidate
phase. Declaration preparation applies that classifier together with executable statement
compilation; released speculation can reuse the same two entry points and convert a refusal to
`NotEligible`.

### Validation

- `cargo check -q`: clean.
- `moth tests --tag const --terse`: 94/94 correct.
- `moth tests --terse`: 3355/3355 correct.
- The updated diagnostics assert reason, path, line, column, occurrence count and the rendered
  operation (`match statement`) for both an uncalled body and an unselected catch handler.

### Deferred diagnostic ordering

Expression roots now visit in authored instruction order. Ordering between an unsupported statement
and an unsupported catch handler in an earlier expression is not globally source-sorted: statement
compilation returns before handler expressions are inspected. Closing that gap needs a partial-body
statement walker or a second AST diagnostic pass. Phase 7 owns the competing-site regression and
must decide whether to reuse the statement walker or extract a rejection collector. Current parser
rules already reject ordinary runtime operations in source order; this deferral affects only
post-parse statement forms the bounded executor cannot compile.

## Corrective Phase 3 evidence

### Projected mutable aliases

The executor previously recorded only unprojected alias roots. A mutable binding to a field such as
`alias ~= root.values` was treated as an independent value, so a push through the alias changed the
const result while the runtime result followed the shared field.

`AliasTarget` now retains the canonical root plus the ordered field/index projections. Alias
composition prepends the existing target's projections when an alias is bound through another
alias, assignment and mutation receivers expand the alias before updating the root, and every root
write refreshes alias bindings with the projected value. `copy place` remains an independent value
and is never recorded as an alias.

`const_function_projected_alias_mutation_parity` proves four behaviours in one case: a push through
a projected alias is visible through the alias and the root, and an explicit `copy` of the same
projection remains independent. Actual JSON/html output is `a=22 b=22 c=12 d=12`; the case fails
against the previous executor with `a=1 b=2`.

### Catch bodies prepared once

Declaration validation already compiled each catch handler to prove eligibility. That work was
discarded, and execution cloned and recompiled the `AstNode` body every time a handler was selected.
The prepared `ConstInstruction` body is now retained in the executable, keyed by the catch
expression. Execution stores the active function's prepared maps in the budget, restores them around nested
calls, and clones only `Rc` handles. Bodies are keyed by catch expression and by authored span, so a
catch expression rebuilt during child rewriting still resolves its prepared body. Ordinary
expression rewriting outside a const frame does not require a handler map. A handler body can
therefore be selected repeatedly without reparsing or cloning the authored tree.

### Bounded nested handler execution

Function-call depth already has `MAX_CALL_DEPTH`. Handler execution has its own
`MAX_HANDLER_DEPTH` counter, separate from `expression_depth`, so a chain of selected handlers is
bounded and returns a structured budget diagnostic without changing when inherited recovery is
consumed at the outermost handled expression.

### Validation

- `cargo check -q`: clean.
- `cargo clippy -q -p moth`: clean.
- `moth tests --tag const --terse`: 96/96 correct.
- `moth tests --terse`: 3357/3357 correct.
- `const_function_repeated_handler_success` selects one handler repeatedly with an
  iteration-dependent recovery value, so stale or reordered handler results change the total.
  It also covers a catch around a required-call receiver and a catch under a folded child. The
  span-keyed lookup remains a defensive fallback for a rewritten catch that is selected during
  execution; the current fixtures do not demonstrate that path.

## Corrective Phase 4 evidence

### Incremental allocation charges

`charge_new_value_text` previously walked every reachable child from each new value row. A new
collection that referenced an existing large string therefore charged that string again for every
parent, making the memory budget depend on graph fan-out rather than newly produced work.

The charge now inspects only the payload owned by each newly inserted row: its own text bytes,
structural string pieces, one retained value slot and the owned vector slots of aggregate payloads.
Shared child rows were already charged when their own row was created and are not traversed again.
Charging tracks one contiguous covered index range, closing gaps when an outer evaluation starts
before a nested call advanced the watermark. Every successful `evaluate_expression` boundary
charges rows created by that evaluation, including direct collection reads and caught-error records
that do not pass through the folder.

Collection mutation, removal and projected assignment charge the exact payload and row slots once,
after the replacement payload exists and before its store insertion. Those rows are recorded as
precharged so the expression-boundary scan does not count the same durable allocation again.
Scratch clones used for reads and loop iteration remain ordinary charges and are not marked as
store rows. A handled mutation failure charges the copied collection slots it discards; a
collection-length read charges its scratch clone slots and leaves the new scalar row to the
expression-boundary charge, so the result row is not counted twice.

Focused unit coverage:

- `shared_text_is_charged_once_through_its_owning_row` inserts one six-byte string, a collection
  that references it twice, then charges both rows once. The store ends at six text bytes and four
  retained slots (two rows plus two owned collection elements), not twelve bytes or a
  graph-dependent count. A second collection appended later is charged for its own row and payload
  without recharging the shared string.
- `structural_piece_growth_consumes_the_shared_value_allowance` retains the existing refusal path
  when owned structural pieces exceed the aggregate allowance.

### Template and executor budget ownership

`ExecutionBudget::template_work` is now the single constructor for a bounded template fold's work
meter. It shares the executor's aggregate meter and per-attempt step cell, so template iterations
and interpreter steps consume the same allowance and cannot drift to separate constants.
`TemplateOutputLimits` remains the remaining-attempt allowance derived from the same aggregate
counters. New unit tests assert the shared `Rc` ownership, the exact remaining byte/piece limits,
and the executor's step limits.

### Validation

- `cargo test -q -p moth --lib`: 7002 passed; 0 failed.
- `cargo clippy -q -p moth`: clean.
- `moth tests --tag const --terse`: 96/96 correct.
- `moth tests --terse`: 3357/3357 correct.
- Existing adversarial limit cases still exercise the infinite loop, deep call chain and template
  output ceiling through the same `MOTH-RULE-0053` budget diagnostic.

## Phase 5 publication and reuse evidence

Provider const materialisation is keyed by the published origin and the entry provider path.
Repeated aliases of one provider origin prepare the provider body exactly once, and the installed
executable serves both local paths and both calls.
`repeated_import_aliases_install_one_provider_const_closure` counts preparation at the body-install
site through a test-only probe and asserts the complete folded output `first=21 second=3` with no
runtime fragment push in any reachable start-function block; the earlier independent substring
assertions could not distinguish a wrong second value. `const_function_facade_private_helper_success`
covers the same shape end to end through the source facade and private helper closure.

Materialisation remains necessary rather than eager: the published closure walk installs only rows
reachable from the entry, and the private helper, transitive callee and alias cases prove the
reachable closure is complete. This evidence does not isolate the negative case of an unrelated
provider row whose body the consumer cannot represent; no behavioural test for that exclusion exists
yet. The local `const_function_unsupported_uncalled_body_rejected` case separately records that a
const body is classified even when no call site exists; it is not provider closure evidence.

Provider invalidation is tested across fresh compilations because no persistent semantic cache or
fingerprint reuse exists at this boundary. Three regressions isolate one cause each while keeping
the provider callable signature and unrelated module interfaces fixed:
`const_function_private_body_edit_changes_consumer_folded_content` changes only the exported body
(`3 -> 13`), `const_function_private_helper_body_edit_changes_consumer_folded_content` changes only
a transitive private helper body (`3 -> 4`), and
`const_function_private_constant_edit_changes_consumer_folded_content` changes only a transitive
private constant (`3 -> 11`). Each asserts the complete folded output and zero runtime fragment
pushes across all reachable start-function blocks before and after. Do not describe this as a
persistent cache invalidation facility.

## Phase 6 owner-boundary evidence

`src/compiler_frontend/ast/const_values/const_execution.rs` is now a directory module. `mod.rs`
retains instruction preparation, rewriting and frame execution; `budget.rs` owns the hard limits,
aggregate meter and per-attempt accounting state plus text/value allocation helpers; `tests.rs` is a
child module so the nine original unit tests retain access to private executor internals through
`super::*`. Budget items are `pub(super)` only, so the parent can use them without widening module
visibility. Moving the accounting helpers removed the last duplicated allocation accounting from
the executor body.

A separate duplicate-ownership audit found no second owner for budget state, traversal, constants
or the moved tests. The remaining `too_many_arguments` suppressions mark executor entry points whose
arguments are already distinct owners; no stale suppression or bounded replacement was identified.

Validation:

- `cargo test -q -p moth --lib`: 7002 passed; 0 failed.
- `cargo clippy -q -p moth`: clean.
- `moth tests --tag const --terse`: 96/96 correct.
- `moth tests --terse`: 3357/3357 correct.

## Corrective Phase 7 coverage evidence

`const_function_parameter_cast_success` proves parameter substitution preserves successful `String`
and `I32` cast semantics inside a required call. `const_function_projected_alias_composition_parity`
covers an alias bound through another projected alias for both a shared collection and an explicit
`copy`, with matching const/runtime counts. `const_function_structural_text_refusal_rejected`
exposed a real boundary defect: `fold_to_constant` treated a shared folder `Refused` outcome as an
unsupported expression, so a structural resource reaching a concrete-text Core operation reported
`const_function_execution_unsupported` instead of
`structural_string_requires_final_text`. The executor now propagates the refusal's owned diagnostic;
the new negative case asserts the structural reason at the required call.

`private_const_helper_closure_is_not_an_importable_declaration` proves executable closure retention
does not widen source visibility. `repeated_import_aliases_install_one_provider_const_closure` now
also exports an unrelated provider const body and asserts its preparation count is zero, covering
the previously untested closure-exclusion negative. The materialisation probe records preparation at
the body-installation site.

Deliberate limits:

- Collection read and iteration now charge scratch-vector bytes before cloning. The follow-up adds
  `const_function_collection_read_memory_rejected`, which executes repeated real `length()` reads
  until the configured memory allowance is exhausted.
- Competing unsupported sites remain fail-fast rather than globally source-sorted when a statement
  rejection and an earlier catch-handler rejection exist in one body. Source-facing runtime forms
  are rejected earlier in authored order; this deferral affects only post-parse executor forms and
  needs one combined collector rather than a second diagnostic pass.

Validation:

- `cargo test -q -p moth --lib`: 7003 passed; 0 failed.
- `cargo clippy -q -p moth`: clean.
- `moth tests --tag const --terse`: 99/99 correct.
- `moth tests --terse`: 3360/3360 correct.

## Slice 7 release folding evidence

Release optimisation runs at the finalization barrier after static selection and AST validation and
before expression freezing/HIR lowering. `rewrite_release_calls` prepares each authored ordinary
body with the same instruction compiler and expression classifier as required const calls, then
rewrites only calls whose arguments fold to constants and whose bounded execution succeeds.

Eligibility is deliberately conservative: parameters must be immutable owned values; the ordinary
return contract must have exactly one success slot and no error channel; every expression must use
supported const forms; source calls must be const calls or other candidates; host calls must be
registered pure Core const-eval operations; and calls with handling or error channels are refused.
Assignments and collection mutations must target a body-local binding, so a speculative fold cannot
erase a module-state write. The candidate graph is then closed transitively: a candidate whose
ordinary callee was refused, or whose ordinary-call closure reaches itself, is removed before any
expression is rewritten. A refused candidate never enters the executable map.

Zero- and multiple-success-slot ordinary calls remain runtime calls. The current expression store has
no single replacement expression that can carry an ordered multi-slot result without inventing a new
value-block convention; the conservative refusal preserves every slot and leaves a native result
path available for a later slice that introduces one.

The speculative call path reuses `CompleteCall`. Missing executables, nonconstant arguments, depth
refusal, folder diagnostics, numeric failure, arity mismatch and result-type mismatch all rebuild
the original call with any rewritten constant children instead of aborting. Infrastructure failures
still abort the module. No partial call removal is published. Expression completion and fallback
union the original expression, its completed replacement and every rewritten child, so folding a
nested argument cannot hide a project-context dependency from later link facts. Each rewritten
expression gets a fresh attempt budget while the const-value store keeps the aggregate module
allowance.

Coverage: `release_folds_only_proven_pure_ordinary_calls` compiles the same fixture in Dev and
Release. Release removes the direct pure call, its transitive pure callee and a pure local mutation,
while parameter-dependent, host-effect and mutable-reference calls remain in HIR. `replaced_child_provenance_reaches_the_surviving_parent`
proves a folded child's project-context provenance survives on an intermediate parent.
`release_rejects_recursive_ordinary_candidates` keeps a mutually recursive pair in release while an
independent helper still folds. `release_preserves_authored_borrow_diagnostics` compiles an authored
mutable-access violation in both profiles and asserts identical user diagnostics.
`const_function_release_folding_success` checks release output parity end to end.

The one existing release Wasm expectation moved because a now-folded function body no longer emits
the first recoverable runtime numeric operation. The compiler-owned constant result is
target-independent, so the next remaining runtime numeric operation becomes the unsupported-backend
site. The diagnostic kind and backend refusal are unchanged.

The integration runner now uses the normal 8 MiB main-thread stack for its Rayon workers. The compiler
is recursive and release adds one bounded finalization pass; the worker previously had Rayon's
smaller default while the same project shape succeeded on the CLI main thread. The stack change
keeps parallel execution equivalent to a direct build rather than masking an unbounded new
recursion.

Validation:

- `cargo test -q -p moth --lib`: 7007 passed; 0 failed.
- `cargo clippy -q -p moth`: clean.
- `moth tests --tag const --terse`: 100/100 correct.
- `moth tests --terse`: 3361/3361 correct.

## Final corrective-phase and release-folding evidence

```text
PROFILE: dev tests; release benchmark
TOOLCHAIN: rustc 1.99.0; cargo 1.99.0
BRANCH: const-functions
```

Configured budgets are now project settings (`const_work_limit`, `const_memory_limit_mb`) bounded
by fixed safety maxima. Deterministic work steps and retained value/text allocation bytes charge one
module meter; no outcome depends on wall-clock time. Collection mutation charges the projected
vector allocation before cloning it. Budget refusals remain typed through template folding. Release
rewriting keeps the runtime call, emits one `Note` at the innermost refused call site (capped per
module) and leaves exit status successful; required const calls keep their blocking error, an
actionable work/memory/hard-limit reason and the active const-call labels.

Final validation:

- `cargo test -q -p moth --lib`: 7012 passed; 0 failed.
- `cargo clippy -q -p moth`: clean.
- `just source-audit`: 0 findings.
- `just validate`: passed; docs check clean.
- `moth tests --terse`: 3367/3367 correct.
- `xtask bench-frontend-check`: suite average 133.45 ms, 300.95 ms case spread, 1 warmup, 10
  measured iterations, `git_dirty: true`, checkpoint `848b4d847`, 76 current vs 42 previous
  workloads. The workload set changed, so this records a non-comparable absolute run rather than a
  like-for-like delta; it remains inside the accepted 30% envelope and preserves the top-stage
  shape (single-file frontend ~139 ms, frontend ~125 ms, boundary compile ~119 ms).
- `just validate-full`: passed. Feature lanes, workspace formatting, feature-enabled clippy,
  test-honesty audit (0 hard findings), all 14 scaling series within budget and the no-timer binary
  check passed. Final audit corrections removed nondeterministic time budgets, added aggregate,
  projected-write, structural-piece, collection-read and iteration-scratch allocation charging,
  exact logical short-circuit parity with
  left-failure propagation and provenance retention, const-call diagnostic labels, linear
  release-candidate graph classification and a bounded ordinary-logical discovery path. Correction
  reruns: 7012 unit tests, 3367/3367 integration cases, clippy clean, final benchmark above. `cache-maintain` reported the existing 36 GiB development cache over its soft
  budget; it removed nothing and this is not a gate failure.
