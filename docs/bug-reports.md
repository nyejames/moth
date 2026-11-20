# Bug reports

A lightweight queue for unresolved bugs and issues discovered during other work or a Correctness audit. Any agent may record an unrelated, non-blocking concern here and return to its task without starting a formal audit. This includes concrete documentation, test, redundancy and efficiency leads found outside the current scope.

Check for an existing entry before adding one. Name where the problem lives, what was observed or looks suspicious and a useful next investigation. Distinguish confirmed defects from unverified hypotheses. Keep the format flexible and the entry short when evidence is limited. Follow [the audit guide](./roadmap/audit-guide.md) for formal audit coverage and findings. Remove an entry in the commit that resolves it.

## Open

### `$infallible` release-only rejection from an erased fallback call

- Found: release/debug divergence audit, 2026-11. Confirmed defect; pre-dates this branch, reproduced by probe on the current branch.
- Where: release-erased call projection into `HirConstraintEvidence::live_erased_call_sites` in `src/compiler_frontend/hir/hir_builder/metadata.rs` from `Ast::release_erased_call_sites`, consumed by the `$infallible` escaping-failure analysis.
- Observed: a probe defines `trap_if_zero |value I32| -> I32: return 1 // value` and a `$infallible` boundary containing `discarded I32 = value // 2 catch then trap_if_zero(1)`. Debug check accepts with no warnings. Release build rejects MOTH-RULE-0091 at the `$infallible` directive: "`$infallible` forbids escaping recoverable errors, but this boundary can return an `Error`". The divisor literal 2 cannot fail, so the fallback cannot run; release folding still erases the fallback call and HIR treats that erased row as live evidence.
- Investigate: release-erased rows are inserted as live unconditionally. Keep a liveness link to the catch handler a folded call came from, or stop release speculation from folding fallback source that HIR cannot route. Add matching debug/release cases.

### `$infallible` accepts a handler trap erased by release folding

- Found: release/debug divergence audit, 2026-11. Confirmed defect; pre-dates this branch, reproduced by probe on the current branch.
- Where: HIR catch lowering in `src/compiler_frontend/hir/hir_expression.rs` and the skipped-handler walk in `src/compiler_frontend/hir/hir_expression/constraint_evidence.rs`. The contract is stated in `docs/docs/directives/constraints.mtf`: "Semantic reachability is checked after static selection, but before optional optimisation erases evidence."
- Observed: a probe defines `divisor || -> I32: return 2` and a `$infallible` boundary containing `discarded I32 = value // divisor() catch:` with `assert(false)` in the handler. Debug check rejects MOTH-RULE-0091 at the `$infallible` directive: "`$infallible` forbids reachable traps, but this boundary's assertion may fail." Release build accepts and emits the project. Release folding replaces `divisor()` with 2, so the lowered protected division no longer routes an error edge to the handler and its assertion never reaches trap analysis; the skipped-handler evidence walk records only calls and fast uses.
- Investigate: preserve trap and assertion evidence for a handler pruned after folding, as the pre-fold failure contributors and pre-fold `Bool` conditions already are. Add matching debug/release cases.

### Repeated work in single-file namespace validation

- Found: Core/Builder package namespace and config filename final review, 2026-11.
- Where: `ModuleNamespaceSet::for_single_file` in `src/build_system/create_project_modules/module_namespace.rs` and `single_file_namespace_inventory` / `eligible_child_directories` in `src/build_system/create_project_modules/source_tree_index.rs`.
- Observed: a selected `@entry.moth` or `@mod.moth` builds a `ProjectModuleGraph` and project namespaces only to validate reserved-root claims, then discards them. The registered-prefix collision probe for an ordinary selected file can list the same parent directories again for each registered source package prefix. No user-visible cost has been measured.
- Investigate only if measurements show a material cost. Bounded single-file mode uses the graph and namespaces to validate reserved-root claims, then discards them. Later synthetic traversal resolves dependencies from inventory keys and filesystem facts, so there are no later project namespaces to reuse. For ordinary files, the prefix probe already shares the root listing. Measure repeated listings below that root before grouping prefixes by shared parents.

### Deferred catch eligibility skipped in inactive static-`if` branches

- Found: failure-fact retention review follow-up, 2026-11. Confirmed defect; pre-dates this branch.
- Where: `StaticIfCandidate::publish` in `src/compiler_frontend/ast/module_ast/finalization/static_if_specialization.rs` rewrites the AST before `AstBuiltinFailureSummaries::compute` in `src/compiler_frontend/ast/module_ast/finalization/finalizer.rs`, so a discarded branch never reaches failure classification.
- Observed: a catch whose eligibility needs the deferred private-call check is rejected when live but accepted in an inactive static-`if` branch. Example: `flaky(1, 2) catch then 0` where an infallible generic `flaky` returns its divisor, placed inside `if false:`. Debug and release both accept it. Syntax, type and immediate catch-eligibility checks still run while parsing, so only the deferred private-call check is lost. The same ordering exists on `main` (read from code, not re-executed there), so this is not a `bug-fixes` regression.
- Investigate: `docs/docs/branching/statement-if.mtf` requires both bodies to be frontend-validated before a known `Bool` selects one, and `tests/cases/catch_and_inactive_branch_source_validation_rejected` already pins syntax and type validation in discarded branches. Preserve the discarded branch's deferred catch checks as validation-only facts, or compute failure summaries before static selection. Add matching debug/release rejection cases.

### JS lazy projection references still retarget on some paths

- Found: projected-alias backend audit, 2026-11. Confirmed by Node probe on the current branch; pre-dates it.
- Where: `src/backends/js/runtime/places.rs` computed `__moth_field`/`__moth_index` records, reached from the range/collection loop iterable temporary in `src/compiler_frontend/hir/hir_statement/loop_lowering.rs` (~1596), mutable assignments and mutable alias declarations, and call arguments evaluated before a later argument rebinds the base.
- Observed: with `s ~= Pair({{1}, {2, 3}})`, `loop s.items |item, idx|:` whose body rebinds `s` to `Pair({{7, 8, 9}, {10, 11, 12}})` prints item lengths `1, 3` instead of `1, 2`: the generated iterable temporary follows the rebound field. `pack(s.items, replace(~s))` also reads the projection after `replace` rebinds `s`. Immutable projected alias definitions already snapshot their storage.
- Investigate: capture the iterable value once at loop entry without losing its borrow provenance, and materialise projected call arguments at evaluation time. Add Node-executed cases.

### Rooted in-memory template sources may not reach disk resources on Windows

- Found: template path simplification review, 2026-11. Unverified hypothesis from code reading; no Windows host was available.
- Where: `src/projects/html_project/moth_template/input.rs` resource containment for in-memory `Sources` with rooted display paths.
- Suspected: `portable_path_text` strips the `\\?\` verbatim prefix while `fs::canonicalize` returns it, so the containment check may reject a sibling resource such as `[@logo.svg]` that the same request accepts on Unix. `rooted_in_memory_document_with_sibling_resource_uses_normalized_url_context` is Unix-only for this reason.
- Investigate: run that case on Windows. If it fails, compare both sides of the containment check in one canonical spelling.
