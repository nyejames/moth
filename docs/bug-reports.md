# Bug reports

A lightweight queue for unresolved bugs and issues discovered during other work or a Correctness audit. Any agent may record an unrelated, non-blocking concern here and return to its task without starting a formal audit. This includes concrete documentation, test, redundancy and efficiency leads found outside the current scope.

Check for an existing entry before adding one. Name where the problem lives, what was observed or looks suspicious and a useful next investigation. Distinguish confirmed defects from unverified hypotheses. Keep the format flexible and the entry short when evidence is limited. Follow [the audit guide](./roadmap/audit-guide.md) for formal audit coverage and findings. Remove an entry in the commit that resolves it.

## Open

### A symlink alias into a `@config.moth` directory bypasses its exclusion

- Found: reserved config directory fix, 2026-11. Reproduced with throwaway Unix symlink tests, 2026-11.
- Where: `SourceTreeSkipPolicy::canonical_directory_is_skipped` in `src/build_system/create_project_modules/source_tree_index.rs` and `authored_directory_exclusion` in `file_reference_resolution.rs`. The dependency-clause message is `direct_special_file_import_message` in `src/compiler_frontend/compiler_messages/render/payload.rs`, reached through `dependency_path_references_config_file` in `module_namespace.rs`, which matches any path component.
- Observed: discovery skips a `@config.moth` directory only by its listed name, and the resolvers reject it only by authored components. With `src/alias -> src/@config.moth`, discovery indexes `@config.moth/notes.mtf` under the alias, and `alias/logo.png` resolves as a resource. Hidden `_` directories have canonical-ancestry checks in both places for this case. Canonical docs (`docs/build-system-design.md`, `project-layout.mtf`, `module-roots.mtf`) define `@config.moth` only as a reserved filename that is inert when nested. No doc makes a config-named directory an excluded subtree. The directory skip came from `37102dfc4` without a doc change. Separately, dependency clauses that enter a `@config.moth` directory are rejected, but the message says the import names `@config.moth` itself.
- Investigate: decide the rule. Either document a config-named directory as an excluded subtree and extend the hidden-ancestry predicate to return the applicable reason in discovery and both resolvers, or delete the undocumented directory skip and the config branch of `authored_directory_exclusion`. Then reword the message for a non-final component. The user-visible reason `ReservedConfigDirectory` (stable key `invalid_compile_time_path.reserved_config_directory`, added in `1c387b7ec`) follows whichever rule is chosen. Document it with the subtree rule, or delete it with the skip.

### Damaged package set classification is path-only

- Found: Core/Builder package namespace and config filename work, 2026-11.
- Where: `damaged_package_set_report` in `src/builder_surface/definition.rs`, used by Stage 0 package discovery, source tree indexing, resource inputs and package reserved-root validation.
- Observed: the infrastructure lane is chosen by whether a path sits below the canonical `BUILTIN_PACKAGES_ROOT`. A ProjectLocal or Dependency package registered inside the compiler's `packages/` directory gets a damaged-installation failure instead of an authored diagnostic. An official package root that is a symlink to a directory outside `packages/` gets the authored diagnostic instead. Both need a modified installation.
- Investigate: whether lane selection should also require Core or compiler-shipped Builder registration identity, kept in one owner for all callers.

### Repeated work in single-file namespace validation

- Found: Core/Builder package namespace and config filename final review, 2026-11.
- Where: `ModuleNamespaceSet::for_single_file` in `src/build_system/create_project_modules/module_namespace.rs` and `single_file_namespace_inventory` / `eligible_child_directories` in `src/build_system/create_project_modules/source_tree_index.rs`.
- Observed: a selected `@entry.moth` or `@mod.moth` builds a `ProjectModuleGraph` and project namespaces only to validate reserved-root claims, then discards them. The registered-prefix collision probe for an ordinary selected file can list the same parent directories again for each registered source package prefix. Both are bounded and small today.
- Investigate: reuse the namespaces later stages build for bounded selections, and group prefixes by shared parent components in the probe. Measure before changing.

### Scaffold `cd` instruction is not shell-quoted

- Found: portable path display hardening inventory, 2026-11.
- Where: `render_summary` in `crates/moth-cli/src/scaffold/mod.rs` (`format!("  cd {}", report.project_path.display())`) and `crates/moth-cli/src/scaffold/tests/summary_tests.rs`.
- Observed: the printed next-step command interpolates the native display spelling with no quoting. Spaces, quotes and shell metacharacters break or alter the pasted command. Control characters pass through to the terminal and a non-UTF-8 directory prints a lossy path that cannot address the directory.
- Investigate: a CLI-owned shell quoting operation with a stated POSIX and Windows contract, control-character handling and no runnable command for an unrepresentable path. Portable path display is the wrong operation here.

### HTML `origin` has no URL encoding contract

- Found: portable path display hardening dev-server URL review, 2026-11.
- Where: `validate_origin`, `prefix_origin`, `strip_origin_prefix` and `origin_root_url` in `src/projects/routing.rs`. The dev server strips the origin from the raw request path in `src/projects/dev_server/http.rs` before percent-decoding the site-local path.
- Observed: validation accepts spaces, Unicode and `%` in the `origin` setting. Emitted URLs use the authored spelling verbatim, so a browser requests `/caf%C3%A9/...` for origin `/café` and the dev server's literal prefix match returns 404. It is undefined whether an authored `%20` is already encoded text or a literal percent.
- Investigate: choose one rule, either restrict origin segments to URL-safe characters or treat the authored origin as decoded text and encode it on emission. Apply the same rule to the dev server prefix match and add loopback coverage.

### Receiver mutation through a `catch`-joined slot changes an immutable parameter's element

- Found: mutable `~` summary fix soundness review, 2026-11. Reproduced by probe on `bug-fixes` HEAD (`tmp/probes-mt/m12_nested_get_receiver_push`, not a durable fixture). Silent miscompile.
- Where: alias-root tracking through `catch` result joins in `src/compiler_frontend/analysis/borrow_checker/transfer/`; root cause not traced.
- Observed: the program below compiles on both profiles and prints `m12 len=2`. The immutable `inner` changes through the immutable `rows` parameter. The alias roots of `rows.get(0)` appear to be lost at the `catch` join, so `~slot.push(4)` sees no immutable root.

  ```moth
  mutate_param |rows {{I32}}|:
      slot ~{I32} = {0}
      ~slot = rows.get(0) catch:
          then {7}
      ;
      ~slot.push(4)
  ;
  inner {I32} = {1}
  data {{I32}} = {inner}
  mutate_param(data)
  io.line([: m12 len=[inner.length()]])
  ```

- Investigate: commit the program as a rejected case expecting `MOTH-BORROW-0007`, then find where the `catch` join drops the success value's roots.

### Mutable scalar slot holding an immutable value's provenance is rejected by `~`

- Found: mutable `~` summary fix review, 2026-11. Confirmed conservative over-rejection on HEAD.
- Where: `roots_for_place` / `effective_roots` in `src/compiler_frontend/analysis/borrow_checker/transfer/access.rs`, consumed by `check_call_mutable_borrow` and `check_call_may_consume`.
- Observed: with immutable `original = 5`, mutable `slot ~= 0`, `~slot = identity(original)` and then `set_two(~slot)` (which only does `~value = 2`) is rejected with `MOTH-BORROW-0007` on `original`. A definite slot storing a produced value reports only the value's provenance roots, so the check targets `original` rather than the slot's own cell.
- Investigate: an attempted waiver for definite slots was unsound for aggregates. Collections, maps and structs share their allocation, so a callee could mutate the immutable original in place. Any relaxation must be limited to types with no in-place mutation surface and must keep the aggregate shapes rejected (see the regression cases added with the `~` summary fix).

### Mutable alias declarations accept an immutable source

- Found: mutable `~` summary fix review, 2026-11. Design conflict between canonical docs and implementation.
- Where: definition path `check_mutable_access` with `require_root_mutable: false` in `src/compiler_frontend/analysis/borrow_checker/transfer/access.rs`. The docs are `docs/developer-docs/memory-management/access-and-aliasing/access-and-aliasing.mtf` (source of a mutable alias "must permit mutation") and `docs/docs/memory/copy-and-exclusive-access.mtf` (alias created "when `source` is an eligible mutable place").
- Observed: `alias ~= data` and `n ~I32 = divisor` compile when `data` or `divisor` is immutable. Writes and `~` arguments through the alias are rejected later. Committed success cases depend on this: `branch_reborrow_after_last_use`, `consistent_borrow_outcomes`, `consistent_ownership_outcomes` and `path_dependent_reborrow`. `live_shared_alias_blocks_mutable_rebinding` expects the conflict reason rather than `immutable_place`. Related: `~total = n`, where `n` is the immutable loop ordinal, is rejected with `MOTH-BORROW-0007` on both base and HEAD. Check that this rejection matches whichever rule is chosen.
- Investigate: decide whether declaration requires a mutable source, as the docs say, or whether the docs should describe deferred rejection at first mutation. Then align the cases.
