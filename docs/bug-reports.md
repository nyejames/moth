# Bug reports

A lightweight queue for unresolved bugs and issues discovered during other work or a Correctness audit. Any agent may record an unrelated, non-blocking concern here and return to its task without starting a formal audit. This includes concrete documentation, test, redundancy and efficiency leads found outside the current scope.

Check for an existing entry before adding one. Name where the problem lives, what was observed or looks suspicious and a useful next investigation. Distinguish confirmed defects from unverified hypotheses. Keep the format flexible and the entry short when evidence is limited. Follow [the audit guide](./roadmap/audit-guide.md) for formal audit coverage and findings. Remove an entry in the commit that resolves it.

## Open

### Dev server snapshot can expose files outside its output root through symlinks

- Found: Static-file Correctness audit, 2026-11. Confirmed by code trace at `72330758a`. Runtime reproduction was unavailable because Cargo is not installed.
- Where: `capture_served_artifacts` in `src/projects/dev_server/build_loop.rs`, consumed by `resolve_request_with_file_exists` in `src/projects/dev_server/static_files.rs` and `prepare_static_response` in `src/projects/dev_server/http.rs`.
- Observed: An existing unowned `_dev/external` symlink to a readable directory outside `_dev` does not invalidate an otherwise successful output write when it is not a planned destination. Snapshot capture follows it and stores the external files under output-relative keys. `GET /external/secret.txt` then serves those bytes. Individual output destinations are canonically checked, and HTML entry validation rejects external symlinks, but snapshot capture has no matching containment check. Exposure is more serious when the dev server binds to a non-loopback interface.
- Investigate: Constrain snapshot capture to canonical targets inside the output root, including file and directory symlinks. Preserve legitimate contained symlinks and the immutable-generation contract. Add an HTTP regression with an unrelated external symlink present before a successful build.

### Authored HTML marker can suppress dev-client injection

- Found: Static-file Correctness audit, 2026-11. Confirmed by code trace at `72330758a`. Runtime reproduction was unavailable because Cargo is not installed.
- Where: `inject_dev_client` in `src/projects/dev_server/static_files.rs`, called by `prepare_static_response` in `src/projects/dev_server/http.rs`.
- Observed: A supported Moth root contribution `[$in head:<!-- moth-dev-client -->]` renders the exact `DEV_CLIENT_MARKER` into generated HTML. `inject_dev_client` treats that authored comment as proof that injection already happened and returns the page unchanged. The response therefore lacks the EventSource subscription and startup-error hooks required for development reporting. Production HTTP injects independently into pristine snapshot bytes for each GET, so the idempotence shortcut does not protect a necessary production state.
- Investigate: Ensure the normal HTTP path installs the client on each pristine generated HTML response regardless of authored marker text, before bootstrap scripts. Add a generated-HTML/HTTP regression and retain an idempotence check only if another production caller needs it.

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

### Executable-body `$export` operand paths reach structural discovery

- Found: namespaced public export paths Phase 1 audit, 2026-11. Confirmed by code trace; pre-dates the route operand.
- Where: `attach_structural_file_facts` in `src/compiler_frontend/headers/file_parser.rs` excludes route path rows only for `$export` runs attached to top-level declarations and dependency clauses. Statement-level runs are captured later by `parse_statement_source_directive_run` in `src/compiler_frontend/style_directives/source.rs`.
- Observed: inside a function body or nested entry block, `$export @docs.mtf` or `$export @missing name` leaves the `PATH` token in the file-reference and qualified-reference scans. Stage 0 can queue the content file or add a provider edge before the AST rejects the directive as `WrongTarget`, so a missing provider may be reported instead of the placement error. Extensionless routes create no physical reference.
- Investigate: identify directive-owned path rows before structural classification, keyed by `PathSyntaxId`, or reject `$export` runs at statement positions during header preparation. Add a body-placement case whose operand resembles a provider reference.

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

### JS projected call arguments still retarget when a later argument rebinds the base

- Found: projected-alias backend audit, 2026-11. Confirmed by Node probe on the current branch; pre-dates it.
- Where: `src/compiler_frontend/hir/hir_expression/calls.rs` keeps an earlier projected argument as an unevaluated place load while a later argument's nested call is flushed into the prelude before the enclosing call. The JS computed-place helpers now capture their base storage when constructed (`src/backends/js/runtime/places.rs`), so the retarget comes from that construction happening after the hoisted later argument ran.
- Observed: `read_count(holder.items, replace(~holder))` returns the replacement's length: the `replace(~holder)` call is lowered into a prelude temp before `__moth_field(holder, "items")` is constructed, so the earlier argument observes the rebound holder. Mutable projected alias declarations and collection-loop iterable temporaries are fixed by base capture; only hoisted call arguments still read the post-mutation base.
- Investigate: keep projected argument construction at its source evaluation position, or capture the projected storage before emitting prelude statements for later arguments. Add a Node-executed case.

### Rooted in-memory template sources may not reach disk resources on Windows

- Found: template path simplification review, 2026-11. Unverified hypothesis from code reading; no Windows host was available.
- Where: `src/projects/html_project/moth_template/input.rs` resource containment for in-memory `Sources` with rooted display paths.
- Suspected: `portable_path_text` strips the `\\?\` verbatim prefix while `fs::canonicalize` returns it, so the containment check may reject a sibling resource such as `[@logo.svg]` that the same request accepts on Unix. `rooted_in_memory_document_with_sibling_resource_uses_normalized_url_context` is Unix-only for this reason.
- Investigate: run that case on Windows. If it fails, compare both sides of the containment check in one canonical spelling.

### Projected alias write targets keep the holder cell as a conservative identity

- Found: borrow-checker projected-alias corrections, 2026-11. Two parts with different origins: the fresh-holder rejection (`holder ~= Pair(...)`) predates this branch, while the value-backed rejection (`holder ~= same_pair(original)`) is a precision regression this branch accepts in exchange for soundness. The baseline accepted it only because its shared-access scan ignored projected write targets, the same gap that made scalar write-through unsound.
- Where: `mutable_alias_write_targets_from_expression` in `src/compiler_frontend/analysis/borrow_checker/transfer/access.rs`. A projected mutable alias targets the union of the holder's designated cells (the pre-correction identity) and the containing allocation roots observed at binding time. A freshly built allocation has no older root: its identity is encoded as the holder cell or as empty provenance, and that identity cannot be transported precisely through joins, whole-binding alias chains, calls or scope exits.
- Observed: `holder ~= Pair({1}, {0}); writer ~= holder.first; ~holder = Pair({2}, {3}); read = holder.first.length(); return read + writer.length()` is rejected with MOTH-BORROW-0003 even though `writer` observes the old allocation and the read observes the replacement. The value-backed form `holder ~= same_pair(original)` is rejected the same way when the rebound holder is read before the final writer read (`mutable_projected_alias_rebound_holder_while_writer_live_rejected`). Allocation-root-only targets accepted unsafe programs wherever the fresh alternative was lost: a join (`mutable_projected_alias_join_fresh_alternative_rejected`), a whole-binding alias transport (`mutable_projected_alias_join_alias_chain_rejected`), a loop back edge (`mutable_projected_alias_loop_back_edge_rejected`) and a scope exit (`mutable_projected_alias_scope_exit_conflict_rejected`). Those rejection cases are now pinned in debug and release. A projection that is only read after the rebind, with no live mutable alias, still passes (`projected_alias_survives_source_rebinding_accepted`).
- Investigate: represent a freshly built allocation as an explicit transported root, for example a generation carried with the projected referent, so targets can stay allocation-rooted through joins, alias chains, calls and scope exits. Then drop the holder-cell union and restore the accepted F1 shape.

### Check-only nested `.mtf` content may skip authored provider occurrences

- Found: namespaced public export paths Phase 2 audit, 2026-11. Unverified hypothesis from code trace; pre-dates routed exports.
- Where: the nested-content loop in `discover_modules_serial_provider_capable` in `src/build_system/create_project_modules/module_inventory.rs` (after `syntax.prepare_source`, around the `structural_file_references` walk).
- Observed: the loop resolves file references for nested content sources but does not visit authored provider occurrences, so `moth check` may never create the transient check-only provider binding that a provider-qualified `.mtf` template head needs. Build mode binds the same head through canonical provider edges.
- Investigate: add a check-only case whose nested `.mtf` file uses a provider-qualified template head from another module, compare with build mode, and if it fails, walk authored provider occurrences in the same loop.

### One nominal type imported under two local aliases reports duplicate receiver methods

- Found: namespaced public export paths Phase 3, 2026-11. Confirmed by reproduction; pre-dates routed exports.
- Where: provider binding deduplication by local path in `src/compiler_frontend/headers/binding_environment/builder.rs` (around line 988) and the defensive receiver guard in `src/compiler_frontend/ast/module_ast/environment/function_signatures.rs` (around line 539), which compares local-path targets instead of method origins.
- Observed: a support package exports `Button` with receiver methods; two child facades each re-export it; an entry importing both aliases fails with MOTH-RULE-0041 (`invalid_receiver_declaration.duplicate_visible_method`). Also reproduced when one facade publishes the same struct under `$export @one` and `$export @two` and a consumer selects both. Each import alone compiles.
- Investigate: deduplicate visible receiver methods by origin declaration identity rather than local path, then add a case importing one type through two aliases.

### Registered Builder package const functions have no published provider body

- Found: namespaced public export paths Phase 3, 2026-11. Confirmed by reproduction at baseline `b0465cd9d` and on the branch; independent of public export routes.
- Where: provider-body publication for registered source packages, surfaced during project entry AST construction. The `compile_registered_package_project` helper in `src/build_system/tests/compile_project_frontend_tests/package_materialisation_tests.rs` reproduces it.
- Observed: a registered package `builder/test/rootpkg` whose `@package.moth` exports `decorate #|label String| -> String` and an entry calling it via `@builder/test/rootpkg decorate as f` fails with `Internal AstConstruction: Imported const function OriginFunctionId { ... package: Builder("builder/test/rootpkg") ... defining_name: "decorate" ... } has no published provider body`. The same function in a project-local `helper/@package.moth` builds, and no-argument package functions work.
- Investigate: compare how project-local support facades and registered package facades publish const-function bodies to the consumer's provider-body registry; an internal error is reachable from valid source, so fix the publication lane and add a registered-package case calling a parameterised const function.

### Package-route collisions omit the package-side authored site

- Found: namespaced public export paths final review, 2026-11. Confirmed limitation by code trace.
- Where: route collision reporting in `src/build_system/create_project_modules/module_namespace.rs` when a registered package's `$export @route` collides with a project-local provider (or the reverse for a package consumer).
- Observed: MOTH-IMPORT-0006 reports only the site owned by the reporting boundary. Project route failures become an unfrozen premerge report with one source owner, so the package `$export` operand's `SourceId` cannot be attached without reinterpreting it in the project source database. The route spelling is kept in the diagnostic path fact.
- Investigate: carry a frozen label handle for the foreign site and retain the package `SourceDatabase` as a report domain, then add the package operand as a related label and assert both sites.
