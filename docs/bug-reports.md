# Bug reports

A lightweight queue for unresolved bugs and issues discovered during other work or a Correctness audit. Any agent may record an unrelated, non-blocking concern here and return to its task without starting a formal audit. This includes concrete documentation, test, redundancy and efficiency leads found outside the current scope.

Check for an existing entry before adding one. Name where the problem lives, what was observed or looks suspicious and a useful next investigation. Distinguish confirmed defects from unverified hypotheses. Keep the format flexible and the entry short when evidence is limited. Follow [the audit guide](./roadmap/audit-guide.md) for formal audit coverage and findings. Remove an entry in the commit that resolves it.

## Open

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
