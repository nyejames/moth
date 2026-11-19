# Bug reports

A lightweight queue for unresolved bugs and issues discovered during other work or a Correctness audit. Any agent may record an unrelated, non-blocking concern here and return to its task without starting a formal audit. This includes concrete documentation, test, redundancy and efficiency leads found outside the current scope.

Check for an existing entry before adding one. Name where the problem lives, what was observed or looks suspicious and a useful next investigation. Distinguish confirmed defects from unverified hypotheses. Keep the format flexible and the entry short when evidence is limited. Follow [the audit guide](./roadmap/audit-guide.md) for formal audit coverage and findings. Remove an entry in the commit that resolves it.

## Open

### Damaged package set classification is path-only

- Found: Core/Builder package namespace and config filename work, 2026-11.
- Where: `damaged_package_set_report` in `src/builder_surface/definition.rs`, used by Stage 0 package discovery, source tree indexing, resource inputs and package reserved-root validation.
- Observed: the infrastructure lane is chosen by whether a path sits below the canonical `BUILTIN_PACKAGES_ROOT`. A ProjectLocal or Dependency package registered inside the compiler's `packages/` directory gets a damaged-installation failure instead of an authored diagnostic. An official package root that is a symlink to a directory outside `packages/` gets the authored diagnostic instead. Both need a modified installation.
- Investigate: whether lane selection should also require Core or compiler-shipped Builder registration identity, kept in one owner for all callers.

### Repeated work in single-file namespace validation

- Found: Core/Builder package namespace and config filename final review, 2026-11.
- Where: `ModuleNamespaceSet::for_single_file` in `src/build_system/create_project_modules/module_namespace.rs` and `single_file_namespace_inventory` / `eligible_child_directories` in `src/build_system/create_project_modules/source_tree_index.rs`.
- Observed: a selected `@entry.moth` or `@mod.moth` builds a `ProjectModuleGraph` and project namespaces only to validate reserved-root claims, then discards them. The registered-prefix collision probe for an ordinary selected file can list the same parent directories again for each registered source package prefix. No user-visible cost has been measured.
- Investigate only if measurements show a material cost. Bounded single-file mode uses the graph and namespaces to validate reserved-root claims, then discards them. Later synthetic traversal resolves dependencies from inventory keys and filesystem facts, so there are no later project namespaces to reuse. For ordinary files, the prefix probe already shares the root listing. Measure repeated listings below that root before grouping prefixes by shared parents.

### Mutable alias declarations accept an immutable source

- Found: mutable `~` summary fix review, 2026-11. Design conflict between canonical docs and implementation.
- Where: definition path `check_mutable_access` with `require_root_mutable: false` in `src/compiler_frontend/analysis/borrow_checker/transfer/access.rs`. The docs are `docs/developer-docs/memory-management/access-and-aliasing/access-and-aliasing.mtf` (source of a mutable alias "must permit mutation") and `docs/docs/memory/copy-and-exclusive-access.mtf` (alias created "when `source` is an eligible mutable place").
- Observed: `alias ~= data` and `n ~I32 = divisor` compile when `data` or `divisor` is immutable. Writes and `~` arguments through the alias are rejected later. Committed success cases depend on this: `branch_reborrow_after_last_use`, `consistent_borrow_outcomes`, `consistent_ownership_outcomes` and `path_dependent_reborrow`. `live_shared_alias_blocks_mutable_rebinding` expects the conflict reason rather than `immutable_place`. Related: `~total = n`, where `n` is the immutable loop ordinal, is rejected with `MOTH-BORROW-0007` on both base and HEAD. Check that this rejection matches whichever rule is chosen.
- Investigate: decide whether declaration requires a mutable source, as the docs say, or whether the docs should describe deferred rejection at first mutation. Then align the cases.
