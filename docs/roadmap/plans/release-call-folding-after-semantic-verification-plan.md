# Release call folding after semantic verification implementation plan

> **For agentic workers:** Implement one independently reviewable phase at a time. Read `AGENTS.md`, the routed compiler authorities and the repository review and validation rules before editing. This is a queued implementation plan, not permission for unrelated optimisation work.

**Status:** Queued.
**Current slice:** Activation pending.
**Blockers:** None. The prerequisite helper rejection is delivered.
**Next action:** Reproduce the recorded probes on a fresh branch, then confirm the owners and sequencing below against that baseline.

**Goal:** Make optional release call folding consume authored semantics rather than define them. HIR construction lowers the specialised authored AST, and the guard-shape decision reads the typed AST expression during lowering. Borrow validation reads the authored HIR before lane installation, and lane installation refreshes the facts it changes. Phase B constraint convergence, constraint verification and public summaries read authored HIR after lane installation. Optional release folding applies to HIR only after those checks finish. Debug and release then agree on source acceptance, diagnostics, public summaries and reuse facts.

**Architecture:** Keep required const execution and release candidate evaluation in Stage 4, where typed source bodies and `ConstValueStore` are the only available representation. Replace the AST rewrite with an ordered release fold plan. HIR lowering consumes the authored AST and the plan together. A new late HIR transaction replaces the lowered call statements in place after verification. It preserves reachable control flow, statement and local identity, evaluation order and every hard-wired CFG decision. A proven-success inferred-lane `Invoke` is replaced with a constant definition and a jump to its success block. Backends consume the folded HIR. Debug never applies the plan.

**Tech stack:** Rust frontend, typed AST expression store, `ConstValueStore`, HIR builder and validation, constraint call-summary convergence, borrow validation, public interface publication, numeric proofs, generated concrete sidecars and the `tests/cases` integration runner.

**Specification:** `docs/compiler-design-overview.md` owns the stage contracts. `docs/docs/directives/constraints.mtf` already owns the user rule that semantic reachability is checked after static selection but before optional optimisation erases evidence. This plan makes the implementation and the overview match that rule. The affected canonical constraints reference must not be weakened.

## Accepted defect and required outcome

Release finalisation currently rewrites pure constant ordinary calls in the AST before HIR exists. `BlockLiteralFacts` and the constraint walkers in `src/compiler_frontend/hir/constraint_facts/` then verify the folded HIR instead of authored semantics. The result is a confirmed debug/release correctness divergence under `$infallible`.

The required outcome has three parts.

1. `$infallible` and `$safe_math` verification read authored HIR for both profiles. A folded call can no longer supply or erase proof evidence.
2. Optional release folding still happens, including inferred-failure private calls that the evaluator proves safe. It becomes a late HIR rewrite after semantic verification and summary publication rather than an AST finalisation rewrite.
3. The compensation paths that exist only because folding runs before HIR are retired, not extended. These are the release-erased call ledger, the executable pre-fold failure contributor projection and any proof-only HIR marker plumbing. `Ast::pre_fold_failure_facts` also carries deferred catch checks and inert assertion-message calls that HIR cannot reconstruct. Retain those validation obligations and their diagnostic order.

No project-wide folding disable and no source-position heuristic is acceptable. Recorded-case refusal by the late pass is acceptable only as a conservative performance loss. It must leave the authored call and cannot fail compilation.

## Architecture

### Target pipeline

```text
Stage 4 finalisation
  static selection, required const folding, validation, constraint attachments,
  fast-use witnesses, release candidate evaluation, release fold plan
  -> authored specialised AST plus fold plan

Stage 5 construction
  authored AST + fold plan -> authored HIR with attached fold transactions
  -> HIR validation
  -> borrow validation
  -> private failure lane installation and catch pruning
  -> phase B constraint summary convergence
  -> constraint verification
  -> public interface close and authored implementation facts

Late release optimisation
  apply fold transactions in place to HIR
  -> structural HIR revalidation
  -> exact link facts and reachability from final HIR
  -> final numeric proof table, expressions freeze, publication

Link time
  target and backend-feature validation consume final link facts and reachability

Backends
  consume folded HIR
```

Construction-time link facts remain transient inputs to summary retention and sidecar selection. The published link facts are recomputed from the final folded HIR. The exact function that owns the late pass can be named during Phase 0. The ordering is not negotiable. Private lane installation and catch pruning finish before phase B. Constraint convergence and verification must complete first. Borrow summaries and public interfaces must complete before any fold can remove a call target. The expression store freeze and numeric-proof computation must happen after the last HIR mutation.

### What moves and what stays

- Required const calls keep folding in Stage 4. They must produce constants before HIR because source typing, constant initialisers, template composition and required-const diagnostics depend on the value. `src/compiler_frontend/ast/const_values/const_execution/mod.rs` documents that boundary.
- Release candidate preparation, purity proof, bounded evaluation and refusal policy stay in `src/compiler_frontend/ast/const_values/const_execution/`. The typed AST and `ConstValueStore` remain the only evaluator input. Do not add a second HIR const executor, a parallel lowering pipeline or a retained incomplete environment.
- The mutation moves. `rewrite_release_calls` must stop replacing AST expressions during `src/compiler_frontend/ast/module_ast/finalization/finalizer.rs`.
- HIR construction remains the first backend-facing semantic producer. It lowers the authored AST, including every call and every checked argument operation that a later fold might erase.
- A new late HIR pass applies the plan. It owns no source parsing, type inference, constant evaluation or user-diagnostic construction.
- Debug builds receive an empty plan and lower exactly as today.

### Authored semantic facts and executable link facts

Authored HIR owns source-semantic facts. Public call summaries, public interfaces, constraint guarantees and reusable interface fingerprints are computed before optional folding and are not rebuilt afterwards. They describe what the source means, including calls that release later erases.

Link facts are executable facts. `src/build_system/build.rs` uses `link_facts.functions` to select reachable functions across modules, and `src/projects/html_project/html_project_builder.rs` passes the resulting reachability to target and backend-feature validation. They must describe the final folded HIR. An erased callee must not stay selected, and its target requirements must not reach validation. Recompute link facts and reachability after the late fold and keep target and backend-feature validation at their existing link-stage boundary.

Each committed fold must merge the replacement's transitive synthetic-interface provenance into `hir.function_provenance` for the owning function before link facts are rebuilt. `src/compiler_frontend/hir/reachability.rs:528` copies that map. Lane installation has already released the construction-only block projection, so a replacement expression row alone would lose a project-context dependency.

### What crosses stages

The fold plan is ordered and immutable after finalisation. Each record describes one authored call occurrence, not one lowered use, and carries enough information for HIR to attach and later replace every executable use without re-reading source.

- the owning concrete callable identity
- the authored expression occurrence ID, remapped through AST freeze
- the exact scalar replacement value and canonical type
- the value mode and synthetic-interface provenance needed by the replacement
- the erased callee target, retained only for plan validation and tests
- the transaction membership that covers the whole call expression and its folded argument tree

Do not key records by source span equality. Two authored literals can share a span shape, generic instances can reuse spans and template subtree copies retain expression identity. The plan key is the remapped authored expression ID plus the owning callable for attribution. A callable plus an unremapped expression ID is not enough.

### Fold identity across freeze and template copies

The finalizer builds the plan before the AST expression store freezes. Freeze compacts rows and rewrites every root, place and edge through `FreezeRemap` in `src/compiler_frontend/ast/expressions/expression_freeze.rs`. A pre-freeze `ExprId` is stale after that pass. Extend the freeze roots with the plan's expression keys, or remap the plan explicitly through the same `FreezeRemap`, before HIR lowering reads it.

The plan key identifies an authored occurrence. HIR lowering mints one transaction per executable lowered use. Template subtree copies can lower the same expression ID at several sites in one callable, so one record can produce several transactions, each with its own statement range, destination locals and replacement. A use that cannot be represented is refused without refusing the other uses. If template normalisation mints a new expression payload for a site, the site-to-expression mapping must carry the fold decision to that payload. HIR never assumes one transaction per expression ID or one expression ID per use.

### Fold transaction contract

1. Only a whole call expression whose complete argument tree has a compile-time constant value may fold. The existing candidate evaluator already requires constant arguments and rejects handled subjects and catch handler bodies.
2. A successful outer call may absorb argument calls that also folded. A refused outer call must roll back every speculative argument rewrite and ledger row. The branch's `revert_refused_call` behaviour in `rewrite.rs` already does this and must be preserved.
3. For an ordinary call statement, the transaction replaces the statement in place with a constant local definition. For a match guard, it replaces the guard expression with the constant while keeping the chosen CFG. For an inferred-lane `Invoke`, it inserts the constant definition and replaces the terminator with a jump to the `Invoke`'s success block. It does not change any reachable control-flow edge, delete or reorder reachable statements, change reachable local IDs or create a failure edge. A reachable block may be renumbered by the compaction that follows removal of an unreachable lane block.
4. Ordinary `Call` statements, inferred-lane `Invoke`s with a matching evaluator-proven-success record and folded match-guard calls are foldable. An explicit fallible `Invoke` (a declared `Error!` call or a source call lowered to an explicit fallible `Invoke`), a catch-routed or otherwise handled call, a Never call and a multiple-result call are refused.
5. A match guard transaction may fold only while retaining the pre-fold guard-lowering decision. `guard_needs_cfg` was chosen from the typed AST guard expression in `src/compiler_frontend/hir/hir_statement/control_flow.rs`. The late pass must not re-run `expression_needs_current_block_lowering` or move the guard prelude to the inline path. If a guard shape cannot keep the chosen CFG, refuse that fold.
6. A record with no executable lowered use, or whose lowered shape is unsupported before mutation, is a normal conservative refusal. The authored call stays, no diagnostic is produced and other records still apply.
7. After a transaction is accepted, a structural violation is a broken invariant. Use `InfrastructureFailure` for an internal mismatch the compiler cannot prove impossible, and `compiler_bug!` only for a state the preflight proves impossible. Never emit a user diagnostic for either.

### Inferred-lane calls

An ordinary call to a private helper with inferred failure is still a release candidate. The evaluator may prove the specific call succeeds, as it already does for `double(2)` in the parity suite. Private lane installation then turns the call statement into an `Invoke` with an unhandled failure edge. The release parity counter includes both `Call` statements and `Invoke` terminators, so a rule that refuses every `Invoke` cannot pass it. The fold plan records the evaluator-proven success and the inferred-lane provenance. The late pass may fold that `Invoke` only when all of the following hold:

- the `Invoke` carries the originating call statement ID and span
- the operation target and argument tree match the plan record
- the failure successor is the lane-generated unhandled failure block, not a catch handler
- the success results match the transaction's expected success-result locals

Lane installation preserves the call statement ID and span. The transaction records the `Invoke`'s success-result locals for the commit. A lane rewrite that changes them without updating the transaction is a normal refusal.

The transaction inserts the constant local definition and replaces the `Invoke` with a jump to its success block. The failure block becomes unreachable and is removed with the same block-removal, side-table rekey and `compact_block_ids` discipline as `prune_unrouted_catch_handlers`. No reachable edge changes.

Authored failure evidence is not lost. Constraint convergence and verification, borrow analysis, `function_failure_facts`, `PublicCallSummary` and the interface facts have already consumed the authored call. The late pass must not edit those facts, and the callee's own failure summary remains on the callee. An explicit fallible `Invoke`, a catch-routed or otherwise handled call, a Never call and a multiple-result call remain refused.

### Preflight refusals and broken invariants

A normal conservative refusal leaves the authored call intact and applies no replacement. It is not an error. It covers a record with no executable lowered use, including an expression removed by static selection, an authored-dead tail, a non-executable template context, a caller that is not lowered or a use in a discarded branch. It also covers a lowered shape the plan does not fold, including an explicit fallible `Invoke`, a Never call, a multiple-result call, a catch-routed or otherwise handled call, an inferred-lane `Invoke` without a matching evaluator-proven-success record, a call whose lowering would need a new prelude, a destination type that differs from the record before mutation or a record that normalisation did not carry to a copied payload. Refusal never skips an argument check, applies a partial transaction or falls back to AST rewriting.

A broken invariant is an internal failure. It covers a transaction that overlaps another transaction, claims a statement or destination twice, loses a destination local, introduces a failure edge or has a replacement type that disagrees with its own recorded result type. Use `InfrastructureFailure` when the compiler cannot prove the state impossible. Reserve `compiler_bug!` for a state the plan's own construction and preflight prove impossible, such as a record whose stored replacement type and stored result type already disagree before HIR exists. Neither case becomes a user diagnostic.

### Rejected approaches

Record these so a later implementation does not rediscover them as shortcuts.

- **Span-equality guards.** Source spans do not identify authored occurrences. Different literals can share a span shape, generic instances can reuse spans and template subtree copies retain expression identity. A span check cannot tell a folded replacement from an authored literal.
- **A proof-only erased-call HIR marker.** It cannot preserve the wholesale argument fold (F6) without more provenance, and it turned the valid guard case (F9) into a rejection by adding a prelude to the inline guard path. It also leaks into borrow publication and numeric proof unless every pass learns a marker-specific skip arm, which turns one local workaround into scattered consumer exceptions.
- **A project-wide fold disable.** The project lead rejected it. It removes release optimisation for every program to correct one proof boundary.
- **Pre-fold HIR duplicated only for checks.** Building a second authored HIR beside the folded HIR doubles release work and creates a parallel semantic authority. It is a migration fallback at best, not the target design.

### Required compiler design overview update

The plan is not complete until `docs/compiler-design-overview.md` describes the delivered design. Update these owners in the completion phase.

- `Frontend stages > Stage 4: AST semantics > Const-function execution and speculative source folding`: replace the sentence that HIR receives the final folded AST with the authored HIR and late fold contract. Required const execution stays in Stage 4. Release ordinary-call folding becomes candidate evaluation plus a fold plan. The AST is not rewritten.
- Check `Architectural invariants` for consistency with the delivered design. The folding sentence is in Stage 4, not in the invariants list. Keep HIR as the first backend-facing semantic IR.
- `Frontend stages > Stage 5: HIR and validation > Lowering boundary`: describe the fold plan input, the authored HIR, the typed-AST guard-shape decision, the in-place transaction and the late pass ordering.
- `Frontend stages > Stage 5: HIR and validation > HIR validation`: require structural revalidation of the folded HIR, state that the final numeric proof table is computed from it and keep backend feature validation at its link-time boundary consuming final link facts.
- `Frontend stages > Stage 5: HIR and validation > Numeric ownership`: separate authored-HIR constraint proof from final-HIR numeric check elision.
- `Per-function link facts`: state that the published facts describe the final folded HIR, that each committed fold merges replacement provenance into the owning function before the facts are rebuilt and that target and backend-feature validation consume them at link time.
- `Public semantic interfaces` and `Fingerprints and reuse facts`: state that constraint summaries, interface facts and reuse fingerprints describe authored source semantics and are not rebuilt after optional folding.
- Reconcile the overview with `docs/docs/directives/constraints.mtf`. The constraints reference is already correct. Do not rewrite its user contract to match the old implementation.

The implementation also updates the module documentation in `src/compiler_frontend/ast/const_values/const_execution/mod.rs`, `src/compiler_frontend/ast/mod.rs` and the new HIR pass. Stale comments that call folding an AST finalisation rewrite are defects.

## Survey: current dependencies on folding order

This survey is a first pass from the current tree. Confirm each owner at activation. The finding that drives the architecture is that constraint summaries and public interfaces are produced after borrow convergence and must never be rebuilt after folding.

| Consumer | Current dependency | Why the ordering matters | Required placement |
|---|---|---|---|
| Constraint convergence and verification | Authored HIR after lane installation, through `src/compiler_frontend/hir/constraint_facts/analysis.rs`, `proof.rs` and `src/compiler_frontend/hir/constraint_verification.rs`. The current release compensation projects `selected_call_sites` for fast use and `live_erased_call_sites` for live trap and failure hazards into `Ast::pre_fold_failure_facts`. | The compensation retains the callee's fast, trap and failure summary bits, but it does not retain the folded call's argument evidence or the proof state around the call. `$infallible` misses a defining call, a direct operand call, a later invalidation, an `Invoke` operand, a multi-return order, a wholesale fold that erases a checked argument and a cast-wrapped value. `src/compiler_frontend/module_compilation/generated/convergence.rs` states phase B is the only writer of the three summary bits and no later pass may rebuild a summary. | Verify authored HIR after phase B. Fold strictly after verification and retire the ledger. |
| Borrow validation and public call summaries | HIR statement walk in `src/compiler_frontend/analysis/borrow_checker/transfer/access/statement.rs`, summary construction in `metadata.rs` and `src/compiler_frontend/public_call_summary.rs` | Statement facts exist per HIR statement. `PublicCallSummary` carries `uses_fast_semantics`, `may_trap` and `may_fail`. Folding before borrow removes call effects and constraint evidence. A later borrow pass reseeds the constraint bits false, so folding cannot be followed by a summary recomputation that recovers them. | Complete borrow validation and summary convergence before folding. Fold must preserve statement identity or refresh every statement-keyed side table. |
| Match-guard and CFG legality | Typed AST guard expression in `guard_needs_cfg` at `src/compiler_frontend/hir/hir_statement/control_flow.rs:533`, consumed during HIR lowering | `guard_needs_cfg` is chosen from the typed AST expression before the guard is lowered. A release fold can make the guard constant, so the debug and release HIR shapes differ. A folded constant guard has no prelude, so the marker-free tree accepts in both profiles. The reverted proof-only marker added a prelude to the inline path and turned the valid guard into `MOTH-RULE-0088`. | Decide guard shape once from the typed AST during HIR lowering. The late pass must not re-run `expression_needs_current_block_lowering` or add a prelude. Keep the guard case as an accept control in both profiles. |
| Numeric proofs | Final HIR in `src/compiler_frontend/analysis/numeric_proofs/mod.rs` | The proof table is operation-keyed by `HirNodeId`, including checked statement operations and `Invoke` terminators, and must pair with the final HIR. The reverted marker proved that an unknown statement clears the interval cache. Folding can legitimately provide a literal divisor for check elision. | Recompute after folding from the final validated HIR. Never pair a pre-fold table with post-fold operation nodes. |
| Const-required calls and argument checks | Stage 4 typed AST in `src/compiler_frontend/ast/const_values/const_execution/` and `src/compiler_frontend/ast/expressions/function_calls.rs` | Required calls and their constant-argument checks establish source types and compile-time values before HIR. They are not optional optimisation. | Leave unchanged in Stage 4. Move only ordinary release speculation. |
| Reachability and link facts | HIR after construction in `src/compiler_frontend/hir/reachability.rs` and `collect_module_function_link_facts`, consumed by `src/build_system/build.rs` and `src/projects/html_project/html_project_builder.rs` | A fold erases a call target. Published link facts drive cross-module function selection and the reachability passed to target and backend-feature validation. If they describe authored HIR, an erased callee stays selected and its target requirements can still affect the build. `reachability.rs` copies `hir.function_provenance`, not replacement expression rows, and lane installation already released the construction-only block projection. | Recompute exact link facts and reachability from the final folded HIR after the late fold. Merge every replacement's provenance into the owning function first. Keep target and backend-feature validation at their existing link-stage boundary. Keep authored summaries and interface/reuse inputs separate. |
| Warnings and diagnostics | AST finalisation in `src/compiler_frontend/ast/module_ast/finalization/finalizer.rs` | Release fold budget notes are appended during finalisation today. Moving the fold changes warning order and can introduce release-only warnings. No production unused-import or unused-binding warning exists today, and `src/compiler_frontend/compiler_messages/diagnostics/families/imports.rs` keeps its unused-import draft for tests only. | Keep warning emission deterministic at the new point. Fold notes stay release-only and ordered. Do not add a new user warning from the late pass. |
| Public summaries and interface fingerprints | `PublicCallSummary`, `src/compiler_frontend/public_interface/local_finalization.rs`, the build-system `interface_fingerprint_content` tests and the fingerprint domains in the overview | Interface close joins borrow summaries and HIR. Folding before close can change interface or implementation facts. The public interface fingerprint has a concrete test owner. The implementation and runtime-dependency fingerprint owners need confirmation at activation. | Close summaries and interfaces before folding. Prove debug/release public interface equality and confirm the other fingerprint owners before claiming their invariance. |
| Release-erased call ledger | `Ast::release_erased_call_sites` in `src/compiler_frontend/ast/mod.rs`, produced by `rewrite.rs` | The ledger exists only to make a folded call visible to post-fold constraint analysis. Once analysis runs before folding, it duplicates authored HIR evidence and invites marker semantics into borrow, numeric proof and backend passes. | Retire it. The fold plan is its replacement and never enters semantic analysis. |
| Pre-fold failure obligations and condition facts | `Ast::pre_fold_failure_facts`, `Ast::pre_fold_bool_conditions` and the projections in `src/compiler_frontend/hir/hir_builder/metadata.rs` | The executable implicit contributor projection and `pre_fold_bool_conditions` exist because folding changed the AST before HIR. `pre_fold_failure_facts` also carries `deferred_custom_catches` and `assertion_message_calls` from authored-dead tails that HIR never lowers. `builtin_failure_diagnostic` consumes both. The named `implicit_failure_custom_mixed_dead_tail_rejected` fixture covers a deferred custom catch and contains no assertion. | Retire the release-fold compensation only. Keep the deferred catch checks, inert assertion-message calls, their authored order and their HIR projection. Keep `implicit_failure_custom_mixed_dead_tail_rejected` and add or retain `implicit_failure_assertion_message_rejects_inferred_call_in_unreachable_code` in `src/compiler_frontend/tests/frontend_pipeline_tests.rs` for the inert message projection. `inactive_assertion_message_obligations_report_the_first_authored_message` covers their authored order. Retire `pre_fold_bool_conditions` only after authored HIR reconstructs every selected condition. |
| Inactive static branches and fast-use evidence | `Ast::constraint_pruned_regions`, `Ast::fast_operation_witnesses` and `src/compiler_frontend/ast/module_ast/finalization/static_if_specialization.rs` | Both bodies are resolved before selection while `$safe_math` must count fast use in an inactive selected handler. HIR never contains the discarded body. | Keep the ledgers and pruning ranges. They are static-selection facts, not release-folding compensations. |
| Template folding | `src/compiler_frontend/ast/templates/template_folding.rs` and the template adapter in `src/compiler_frontend/ast/const_values/const_execution/rewrite.rs` | TIR is AST-local. Release candidates reject template and runtime-template operations, so the late pass must not move template semantics into HIR or reconstruct a template from a folded scalar. | Leave required-const and template folding in Stage 4. Fold scalar call transactions only. |
| HIR validation and backend feature validation | `src/compiler_frontend/hir/validation/blocks.rs`, `dominance.rs` and `src/backends/backend_feature_validation.rs` | The late pass mutates validated HIR. Backends assume validated control flow and exact node/key agreement. `BackendFeatureValidationInput` needs the selected target and the reachable union that `src/projects/html_project/html_project_builder.rs:606` supplies after entry linking. | Revalidate the folded HIR structurally before publication. Leave backend feature validation at its existing link-time boundary and feed it the final link facts. |
| Generated concrete functions | `src/compiler_frontend/module_compilation/generated/convergence.rs` | Sidecars own their HIR, borrow summary, verification, proofs and freeze. Folding only the base module makes generic instances disagree between profiles. | Apply the same late pass to each reachable sidecar after its verification, before its proof table and freeze. |
| Private failure lanes and catch pruning | `refresh_private_failure_lanes` in `src/compiler_frontend/module_compilation/generated/convergence.rs`, which runs before phase B, and `src/compiler_frontend/hir/validation/blocks.rs` | Folded calls influence which failure edges exist only when folding runs too early. Authored HIR keeps the authored failure graph for lane installation. | Install lanes and prune handlers before phase B and constraint verification. Fold after them, then revalidate the in-place rewrite. |

## Phases

### Phase 0: Reproduce, inventory and fix the boundary

1. Reproduce every recorded case in the mandatory inventory below on a scratch project under `tmp/`. Record the debug and release verdicts, typed reasons and labelled spans. The red evidence is temporary. Do not commit a test that is expected to fail.
2. Confirm the pipeline order in `src/compiler_frontend/module_compilation/service.rs` and `src/compiler_frontend/module_compilation/generated/convergence.rs`, especially private failure lane installation, constraint convergence, verification, public interface close, expression freeze, numeric-proof publication and the link-time target and backend-feature validation boundary.
3. Inspect the current release candidate evaluator and rewrite transaction in `src/compiler_frontend/ast/const_values/const_execution/rewrite.rs`. Confirm which expression identities and values are available before the AST is consumed by HIR.
4. Choose the fold-plan key and the HIR attachment point. Record how the key is remapped through AST freeze, how one key produces a transaction per executable use and why the identity is not a span, rendered name or storage position.
5. Confirm the survey table against the current tree and record any owner that has moved. Resolve any conflict with the required ordering in this plan before writing code.

**Gate:** Every recorded failure reproduces its debug/release split, the guard case accepts in both profiles and the pipeline order is written down with current owners. No production behaviour has changed yet.

### Phase 1: Move the fold boundary

1. Split release candidate evaluation from mutation in `src/compiler_frontend/ast/const_values/const_execution/`. Build and return one ordered fold plan. Keep purity, budget, refusal, rollback and required-const behaviour unchanged.
2. Stop calling the AST replacement from `src/compiler_frontend/ast/module_ast/finalization/finalizer.rs`. The specialised AST reaches HIR authored. Static selection, required const folding, constraint attachments and fast-use witnesses keep their current order.
3. Carry the plan across the AST-to-HIR boundary. Remap its expression keys through the AST freeze. HIR construction lowers the authored AST and attaches a transaction to each matching executable lowered use, including template-copy uses of a shared expression. The plan must not retain parser state, TIR authority or a second expression store.
4. Add the late HIR pass. Apply it in release only, after constraint verification, borrow summary convergence and public interface close, and before the expression freeze and numeric-proof computation. Move the freeze and proof publication points where the current order prevents that. Keep inferred-lane calls that the evaluator proves safe foldable through the rule in the inferred-lane section.
5. Replace ordinary call statements in place with constant local definitions. Fold an inferred-lane `Invoke` by inserting the constant definition and replacing it with a jump to its success block. After all transactions commit, remove the unreachable lane failure blocks with the lane installer's block-removal and rekey discipline. Preserve reachable block and statement order, local identity, reachable terminators and the pre-fold match-guard CFG decision. On commit, merge the replacement's transitive synthetic-interface provenance into the owning function's `hir.function_provenance`. The construction-only block projection is already gone after lane installation, so do not rely on it. Refuse before mutation any transaction whose lowered shape cannot keep these invariants.
6. Revalidate the folded HIR structurally. Preflight a transaction for failure edges and destination types before accepting it. For an inferred-lane `Invoke`, prove that the failure successor is the lane-generated unhandled block and that the success results match the plan. A post-acceptance violation follows the broken-invariant lane rather than becoming a user diagnostic.
7. Add the mandatory cases and accept controls at this phase, once the boundary makes them pass in both profiles. Do not leave a marker, a proof-only HIR event or an expected-failing case in the tree.
8. Add a direct test that release still folds a candidate after the move. Assert the folded call is absent or replaced in the final HIR while debug retains it. A silently disabled optimisation must fail the phase. Keep the call-count parity in `src/build_system/tests/compile_project_frontend_tests/release_speculation_scope_tests.rs`, especially `release_speculation_leaves_unrelated_runtime_expressions_unchanged`, where `double(2)` must fold through its inferred failure lane and `release_budget_note_names_only_an_attempted_candidate_call`. Keep `release_folds_only_proven_pure_ordinary_calls` and the branch, transitive and short-circuit provenance tests in `entry_api_stability_tests.rs`.

**Gate:** Every mandatory case has identical debug and release acceptance and diagnostics. Authored-literal, ordering and guard accept controls pass in both profiles. Release folding is demonstrably still active. Debug HIR is unchanged apart from the new plan plumbing.

### Phase 2: Refresh dependent analyses and retire compensations

1. Recompute `NumericProofs` from the final folded HIR for the base module and every reachable generated sidecar. Confirm no proof fact refers to a removed or renumbered operation node.
2. Keep public call summaries, public interfaces and interface/reuse fingerprint inputs on authored HIR. Retained call summaries can describe an erased call after folding. Treat them as conservative publication facts, or prune them only where the owning consumer proves exactness is required and the public interface does not change. Add a debug/release equality test for public interfaces using `interface_fingerprint_content` in `src/build_system/tests/compile_project_frontend_tests/entry_api_stability_tests.rs`. Locate the implementation and runtime-dependency fingerprint owners at activation and prove their invariance too. If a test shows a fingerprint changes only because of folding, fix the fingerprint input rather than accepting profile drift.
3. Recompute exact link facts and reachability from the final folded HIR before publication. `src/build_system/build.rs` uses them to select reachable functions across modules. Confirm every committed fold's replacement provenance has been merged into `hir.function_provenance` for the owning function before `reachability.rs` copies it. Confirm an erased callee is not selected and its target requirements do not reach validation. Keep target and backend-feature validation at their existing link-stage boundary in `src/projects/html_project/html_project_builder.rs`, consuming the final link facts. Add a case whose folded call to a differently targeted or unsupported callee would fail validation if the authored edge were still published. Add a package-context case where the folded private call is the only route to the dependency.
4. Audit statement-keyed borrow and lifetime consumers. Either preserve statement and local identity through the in-place transaction or explicitly prune and rekey every affected fact. Follow the lane installer's rekey discipline when an unreachable failure block is removed. Dangling facts and facts that describe erased calls are both defects. A conservative superset is acceptable only where the owning consumer documents it.
5. Keep the existing transitive fast-use rejection case `source_constraints_folded_call_fast_rejected` and its release pair. It proves rejection, not publication, because a rejected module publishes no interface. Add an accepted companion where a non-`$safe_math` exported caller's only fast use is a folded call and assert that its published `PublicCallSummary.uses_fast_semantics` is true. Extend `private_helper_constraint_changes_invalidate_the_provider_interface_fingerprint` in `src/build_system/tests/compile_project_frontend_tests/entry_api_stability_tests.rs` with a folded-call variant. Add a downstream `$safe_math` caller to prove the published bit rejects. Confirm `may_trap` and `may_fail` have equivalent publication coverage through the same private-helper constraint tests.
6. Retire the pre-fold executable implicit-contributor projection once authored HIR supplies failure contributors. Retire `Ast::pre_fold_bool_conditions` once authored HIR reconstructs every selected condition. Delete `Ast::release_erased_call_sites` with every projection, marker arm or test that exists only for it. Keep the deferred catch checks and inert assertion-message calls carried by `pre_fold_failure_facts`, their authored order, their HIR projection and their `builtin_failure_diagnostic` consumer. Keep `implicit_failure_custom_mixed_dead_tail_rejected` for the deferred catch and keep `implicit_failure_assertion_message_rejects_inferred_call_in_unreachable_code` for the inert message projection, both green in debug and release. Keep `constraint_pruned_regions` and `fast_operation_witnesses`. Confirm that no borrow, numeric-proof, backend, display or feature-validation pass reads a proof-only folded-call event.
7. Keep the existing refused-call rollback. Keep the auxiliary-fold control so a refused outer call cannot retain an argument fold.
8. Audit release fold budget notes and warning order. Keep them deterministic, release-only and free of new user-visible warnings. If an unused-import or unused-binding warning is added later, it must be computed before optional folding.
9. Cover generated concrete functions. A generic instance that folds a call must match its debug instance on acceptance, summaries and diagnostics and must publish proofs and exact link facts from its folded HIR.

**Gate:** The release-fold compensation and marker path are gone, while deferred catch and assertion-message validation obligations remain intact and ordered. Numeric proofs pair with final HIR. Public summaries, interfaces and fingerprint inputs are profile-independent. Published link facts describe only executable final-HIR edges. Target and backend-feature validation consume those facts at their link-time boundary. Sidecars match the base module. All recorded controls pass.

### Phase 3: Document, validate and retire

1. Update `docs/compiler-design-overview.md` as specified above. Reconcile the overview with `docs/docs/directives/constraints.mtf` instead of weakening the constraints reference.
2. Update module documentation and stale comments in the AST const-execution owner, the AST publication type and the new HIR pass. Search for statements that folding happens before HIR or that HIR receives the folded AST.
3. Re-run the mandatory inventory in both profiles, the authored-literal, ordering, guard, rollback, transitive rejection, publication and generated-sidecar controls and the function-selection, target-validation and backend-feature checks over exact post-fold link facts.
4. Run the focused frontend unit tests, the affected `moth tests` tags, the documentation checks and the appropriate validation gates from `docs/developer-docs/style-guide/validation.mtf`. Use `just validate` at the branch boundary and `just validate-full` before merging into `main`.
5. Perform the repository Slice review. Remove redundant checks, stale fixtures, dead plan plumbing and duplicate coverage. Check diagnostic spans, deterministic order and the absence of a release-only warning.
6. In the same commit that completes the work, delete this plan and remove its roadmap bullet. Do not commit a completed plan as outstanding work.

**Gate:** The overview and constraints reference agree, all inventory and control cases are green in both profiles, full validation evidence exists for the final tree, the Slice review has no unresolved finding and the plan is retired in the completion commit.

## Mandatory regression inventory

Add these as debug/release case pairs. F1 to F7 are the recorded divergences and must become reject/reject with the debug reason and labelled spans. F8 is an existing reject/reject control. F9 is an accept control. The source names are suggestions, not a second contract.

The snippets share these helper declarations unless a snippet repeats them.

```text
divisor || -> I32:
    return 2
;

numerator || -> I32:
    return 8
;

identity |value I32| -> I32:
    return value
;

flag || -> Bool:
    return true
;
```

### F1: local defined from a folded call

```text
divisor || -> I32:
    return 2
;

$infallible
probe |value I32| -> I32:
    d I32 = divisor()
    discarded I32 = value // d catch:
        assert(false)
    ;
    return discarded
;

~[: [probe(8)]]
```

Reject in both profiles with `MOTH-RULE-0091` and reason `trap_assertion`, labelling `$infallible` and `assert(false)`. Release currently accepts.

### F2: direct folded operand

```text
$infallible
probe |value I32| -> I32:
    return value // divisor()
;
```

Reject in both profiles with reason `escaping_error`, labelling `$infallible` with no secondary span. Release currently accepts.

### F3: earlier local invalidated by a later folded call

```text
$infallible
probe |value I32| -> I32:
    d I32 = 2
    n I32 = divisor()
    discarded I32 = n // d catch:
        assert(false)
    ;
    return discarded
;
```

Reject in both profiles with reason `trap_assertion`, labelling `$infallible` and `assert(false)`. Release currently accepts.

### F4: folded call inside an `Invoke` terminator

```text
$infallible
probe |value I32| -> I32, Error!:
    d I32 = 2
    return numerator() // d
;
```

Reject in both profiles with reason `escaping_error`, labelling `$infallible` and the division. Release currently accepts.

### F5: multi-value return order

```text
$fast_math $infallible
probe |value I32| -> I32, I32:
    d I32 = 2
    return numerator(), value // d
;
```

Reject in both profiles with reason `trap_division`, labelling `$infallible` and the division. Release currently accepts.

### F6: wholesale fold erasing an argument hazard

```text
identity |value I32| -> I32:
    return value
;

divisor || -> I32:
    return 2
;

$fast_math $infallible
probe || -> I32:
    return identity(8 // divisor())
;
```

Reject in both profiles with reason `trap_division`, labelling `$infallible` and the division witness. Release currently accepts.

### F7: cast-wrapped folded value

```text
$infallible
probe |value I32| -> I32:
    d I32 = 2
    n I64 = cast divisor()
    discarded I32 = value // d catch:
        assert(false)
    ;
    return discarded
;
```

Reject in both profiles with reason `trap_assertion`, labelling `$infallible` and `assert(false)`. Release currently accepts.

### F8: existing statement-boundary reject control

```text
probe |value I32| -> I32:
    d I32 = 2
    $infallible
    discarded I32 = numerator() // d catch:
        assert(false)
    ;
    return discarded
;
```

Existing reject/reject control. Both profiles reject today with the same `trap_assertion` witness, labelling the statement `$infallible` and `assert(false)`. This covers the statement boundary independently of the function boundary and is not a current divergence. Keep it to prevent a regression.

### F9: guard CFG accept control

```text
flag || -> Bool:
    return true
;

identity |value Bool| -> Bool:
    return value
;

x ~= 1
if x is:
    1 if identity(flag() and flag()) => ~x = 2
    else => ~x = 3
;
~[: guard=[x]]
```

Accept in both profiles on the marker-free tree. The reverted proof-only marker caused a `MOTH-RULE-0088` rejection with reason `side_effecting_prelude` by adding a prelude to the inline guard path. This case is a regression control against any future post-fold guard annotation, not a current divergence.

### Authored and ordering accept controls

Add or retain these as debug/release accept pairs.

- A direct authored literal divisor `return value // 2` accepts.
- A literal through a pure wrapper `return keep(value // 2)` accepts.
- An authored literal local divisor followed by a handled division with `assert(false)` accepts.
- A call after the proved division accepts in statement order (`source_constraint_infallible_erased_call_after_terminator_division_accepted`).
- A call after the proved division accepts in multi-return order (`source_constraint_infallible_erased_call_after_multi_return_division_accepted`).
- A literal definition after a folded call accepts (`source_constraint_infallible_literal_after_folded_call_accepted`).
- A handled division followed by a folded call accepts (`source_constraint_infallible_folded_call_after_proof_accepted`).

### Rollback reject control

Keep `source_constraint_infallible_auxiliary_fold_stays_runtime_rejected` and its release pair. The source is `return keep(value // make_names().length())`, where `make_names` is a const function returning a collection. Both profiles reject with `unknown_trap_evidence` because the refused outer call discards the speculative argument fold. This is the end-to-end control for `revert_refused_call`.

## Risks and mitigations

- **Summary understatement from an early fold.** The strongest failure mode is moving the fold before phase B or before public interface close. Mitigation: encode the ordering in one publication owner, add a debug/release public-summary equality test, keep the transitive fast-use rejection case and add an accepted companion that inspects the published summary.
- **Dangling statement-keyed facts.** Borrow, lifetime, numeric proof and catch facts can key on statement, operation or block IDs. Link facts key on functions and blocks. Mitigation: in-place replacement that preserves IDs for normal folds, the lane installer's rekey discipline when an unreachable lane block is removed, plus an audit of every keyed side table and a post-fold revalidation.
- **Lost validation-only failure obligations.** Deleting `pre_fold_failure_facts` wholesale would drop deferred custom-catch checks and inert assertion-message calls from authored-dead tails. Mitigation: retain those fields, their authored order and the `builtin_failure_diagnostic` consumer. Keep `implicit_failure_custom_mixed_dead_tail_rejected` for the deferred catch and keep `implicit_failure_assertion_message_rejects_inferred_call_in_unreachable_code` for the inert message projection, both in debug and release.
- **Match-guard regression.** A post-fold prelude or re-lowering can reject a guard that debug accepts. Mitigation: the guard decision is made once from the typed AST expression during HIR lowering, and the late pass may not add a prelude or inspect folded expression shape to choose a path.
- **Partial or absorbed argument folds.** A whole-call fold can legitimately erase its folded argument tree. A refused outer call cannot keep any argument fold. Mitigation: transaction-level commit and rollback, plus the auxiliary-fold control.
- **Inferred-lane fold erases authored failure evidence.** A lane `Invoke` carries the call site's inferred failure edge. Folding it must not rewrite the pre-fold constraint, borrow or summary facts, and must not fold an explicit fallible or handled call. Mitigation: fold only an evaluator-proven-success unhandled lane with a matching statement ID, target, argument tree and success results. Keep the pre-fold facts read-only and refuse catch-routed calls.
- **Profile drift in fingerprints and selection.** Semantic summaries, interfaces and reuse inputs must describe authored source while link facts must describe executable final HIR. Mixing them either publishes erased calls or changes interface facts between profiles. Mitigation: separate the two lanes, add interface equality tests and add function-selection, target-validation and backend-feature tests over exact post-fold link facts.
- **Optimisation silently disappearing.** A late representation that refuses every transaction would restore debug/release agreement by disabling release folding. Mitigation: a direct HIR test proves a candidate still folds, the `double(2)` parity case proves an inferred-failure call folds through its lane and a parity check compares the folded call-site set with candidate selection.
- **Warning order and release-only noise.** Budget notes move with the fold. Mitigation: one deterministic emission point and no new warning from the pass.
- **Store freeze and capacity.** HIR expression storage stops growing at its final structural producer. Mitigation: fold before the freeze, or pre-materialise replacement constants during construction. Do not retain dead replacement nodes after publication.
- **Lost project-context provenance.** A folded call can carry synthetic-interface provenance from its arguments or its callee body. Carrying it on the replacement is not enough. Private lane installation releases the construction-only block projection, and `src/compiler_frontend/hir/reachability.rs:528` copies `hir.function_provenance` into published link facts. If an erased private call was the only route to a project-context dependency, the package check at `src/build_system/build.rs:1264` loses it. Mitigation: carry the replacement provenance in the fold plan, merge it into `hir.function_provenance` for the owning function when the transaction commits and rebuild link facts only after the merge. Keep the existing branch, transitive and short-circuit provenance tests and add a package-context case where the folded private call is the only route to the dependency.
- **Generated sidecars.** Generic instances can fold independently. Mitigation: run the same pass and proof refresh for every reachable sidecar after its verification.
- **Marker creep.** A proof-only HIR event or span check is tempting because it is local. Mitigation: the plan forbids it. Evidence belongs in authored HIR before the fold, not in a post-fold annotation.

## Validation and completion

- Add the inventory as debug/release integration cases under `tests/cases/`. Use one input with profile-specific expectation files where the existing runner supports it. Keep both profiles visible for every mandatory case.
- Add focused HIR tests for the fold plan attachment, freeze remapping, template-copy use expansion, in-place replacement, inferred-lane `Invoke` replacement, unreachable lane block removal, guard CFG preservation, conservative refusal, broken-invariant classification and final numeric-proof pairing.
- Add a generated-sidecar case. Keep `source_constraints_folded_call_fast_rejected` and its release pair, then add the accepted publication companion that inspects `PublicCallSummary::uses_fast_semantics`.
- Add a debug/release public-interface and fingerprint equality test. Add function-selection, target-validation, backend-feature and package-context provenance tests over exact post-fold link facts. Do not accept a new fingerprint domain for folding.
- Keep `implicit_failure_custom_mixed_dead_tail_rejected` for the deferred catch and keep `implicit_failure_assertion_message_rejects_inferred_call_in_unreachable_code` for the inert assertion-message projection, both in debug and release.
- Run the affected frontend unit tests, the `constraints` and `release` case tags, the generated-function cases and the documentation gate. Follow `docs/developer-docs/style-guide/validation.mtf` for branch and merge checkpoints.
- Complete the Slice review. Then delete this plan and its roadmap bullet in the same commit that completes the work.

Success means the recorded failures reject identically in debug and release, authored literals and valid ordering still accept, the guard case accepts in both profiles, release still folds proven calls including inferred-lane candidates, public summaries and interface facts are profile-independent, published link facts describe only executable final-HIR edges, target and backend-feature validation consume them at their link-time boundary and the overview describes the delivered boundary.

## Out of scope

- New const-function features, new release candidate shapes, broader constant propagation or whole-program partial evaluation.
- A project-wide folding disable, a source-position heuristic, a span-equality guard or a proof-only HIR marker.
- Moving required const execution or template/TIR folding into HIR.
- Recomputing public interfaces, cross-module constraint summaries or reusable interface fingerprints from folded HIR.
- Backend-specific folding, check elision policy changes or unrelated optimisations.
