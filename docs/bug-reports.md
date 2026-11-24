# Bug reports

A lightweight queue for unresolved bugs and issues discovered during other work or a Correctness audit. Any agent may record an unrelated, non-blocking concern here and return to its task without starting a formal audit. This includes concrete documentation, test, redundancy and efficiency leads found outside the current scope.

Check for an existing entry before adding one. Name where the problem lives, what was observed or looks suspicious and a useful next investigation. Distinguish confirmed defects from unverified hypotheses. Keep the format flexible and the entry short when evidence is limited. Follow [the audit guide](./roadmap/audit-guide.md) for formal audit coverage and findings. Remove an entry in the commit that resolves it.

## Open

### Function-local constant inserts are rejected before composition

- Found: runtime template helper escape fix review, 2026-11. Confirmed by a source probe. Reported as pre-existing.
- Where: final-value validation of function-body constant bindings.
- Observed: inside an ordinary function, `insert #= [$in title:Priya]` is rejected with `MOTH-SYNTAX-0022` at `$in`, even when the only use composes it into `[card:[insert]]` and `card` defines `[$slot title]`. The equivalent stored insert at module scope is accepted for composition.
- Investigate: preserve function-local constant insert helpers until their receiving template consumes them, while still rejecting helpers that escape as runtime values.

### Stored inserts read through namespaces or constant records do not compose

- Found: runtime template helper escape fix verification and final integration audit, 2026-11. Confirmed by source probes. The integration audit reproduced the namespace forms on base `84cc4c945` and checkpoint `df60b63ac`.
- Where: template heads route a bare identifier helper, including a directly imported one, through `declaration_reference_expression` and `rebuild_const_template_reference`. A qualified namespace leaf such as `h.insert` stays a plain reference, and `resolve_field_member` in `src/compiler_frontend/ast/field_access/field_member.rs` returns a const-record field read without rebuilding the helper as a template.
- Observed: `[card: [h.insert] body]` in a runtime function or a module binding fails with `MOTH-INFRA-0001` at HIR lowering. In a `~[...]` fragment it failed with `MOTH-INFRA-0001` on base and now reports a misleading `MOTH-SYNTAX-0022` "cannot escape as a standalone runtime value", because statement-root templates keep the enclosing `Module` context kind. With `pattern #= (slot_value = [$in style: red])`, the constant `stored #= [card: [pattern.slot_value] body]` is rejected with `MOTH-SYNTAX-0022` as not fully foldable, and `[card: [pattern.slot_value] [n]]` in a runtime function fails with `MOTH-INFRA-0001`. `template-slots.mtf` says imported wrappers and constant-record projections preserve slot routing. Runtime value reads of these spellings outside templates are rejected with `MOTH-SYNTAX-0022`.
- Investigate: rebuild a stored helper read through a namespace leaf or record projection as a template in composition positions, as identifier heads already do, then add success coverage for constant, runtime-template and `~[...]` composition.

### Stored insert reads are gated by context kind rather than position

- Found: runtime template helper escape fix final review verification, 2026-11. Confirmed by source probes on base `84cc4c945` and the candidate.
- Where: `ContextKind::admits_structural_helpers` in `src/compiler_frontend/ast/module_ast/scope_context.rs`, used by `reject_runtime_slot_insert_helper` in `src/compiler_frontend/ast/field_access/parse_chain.rs`. Template heads and call arguments inside them inherit the template parsing context, while `parse_statement_template` in `src/compiler_frontend/ast/statements/body_dispatch.rs` keeps the enclosing kind.
- Observed: with `local #= [$in style: [: red]]` and `show |text String| -> String`, the expression template `x = [: [show(local)]]` fails with `MOTH-INFRA-0001` "non-renderable template constant", because the call argument is admitted as if it were a composition position. The statement root `~[: [show(local)]]` correctly rejects the read with `MOTH-SYNTAX-0022` (`MOTH-RULE-0052` on base). Parsing statement roots in the template context would fix the `~[...]` namespace diagnostic above but turn this correct rejection into `MOTH-INFRA-0001` and change catch eligibility in `~[...]` heads.
- Investigate: admit stored helpers only in direct template head or body value positions, through a composition flag that call-argument and operand parsing do not inherit, then parse statement roots in the template context with a rejection fixture for catch in `~[...]` heads.

### Constant collections of stored inserts reach HIR

- Found: runtime template helper escape fix final integration audit, 2026-11. Confirmed by a source probe on the candidate and reported identical on checkpoint `df60b63ac`.
- Where: runtime reads of constant collection elements, outside the stored-helper checks in `src/compiler_frontend/ast/field_access/`.
- Observed: `items #= {[$in style: red], [$in style: blue]}` with `loop items |item|: ~out = item` in a runtime function fails with `MOTH-INFRA-0001` "Template constant reached HIR constant expression lowering before AST materialized it".
- Investigate: classify a collection whose elements are stored `$in` helpers when it is read as a runtime value and report `MOTH-SYNTAX-0022` at the read.

### Borrow diagnostic names an internal HIR temporary

- Found: runtime template helper escape fix final integration audit, 2026-11. Confirmed by a source probe.
- Where: borrow diagnostics that render a compiler-introduced HIR temporary as a user name.
- Observed: `items #= {[:red], [:blue]}` with `out ~= ""` and `loop items |item|: ~out = item` in a function reports `MOTH-BORROW-0003` "Cannot mutably access `out` while shared alias `__hir_tmp_0` is still needed later".
- Investigate: whether the loop over a constant collection should borrow at all here, and render compiler temporaries by their authored source expression rather than their internal name.

### Dev server snapshot containment checks and reads symlink targets by name

- Found: dev-server symlink containment fix review, 2026-11. Confirmed by code trace; requires a concurrent writer inside the output root, so stable trees are contained.
- Where: `capture_served_artifacts` in `src/projects/dev_server/build_loop.rs` canonicalises a symlink target, checks it is inside the output root, then reopens the target by path.
- Observed: if capture pauses after verifying a contained alias, the alias is retargeted to another contained file and its former target is replaced by an out-of-root symlink, the reopen publishes the external bytes under the alias's HTTP key. The directory branch also checks and reopens by name. An unowned hard link to an external file is served in both the old and new code; path containment cannot distinguish it.
- Investigate: anchor traversal at the verified output-root handle and open entries beneath it (openat with no-follow semantics, or a capability-style filesystem crate), then read the same handles. This needs platform-specific APIs or a new dependency, so decide the approach before implementing. Decide separately whether hard links to external files are inside the boundary.

### Control-flow scope names come from a process-wide counter

- Found: const foundation Phase C parity test, 2026-11. Confirmed by building one project twice in one process; not observed in user-visible output.
- Where: `ScopeContext::new_child_control_flow` in `src/compiler_frontend/ast/module_ast/scope_context.rs` names each control-flow scope `__scope_{N}` from the static `CONTROL_FLOW_SCOPE_COUNTER`.
- Observed: the interned scope-path text depends on how many scopes earlier compilations in the process created, and on thread interleaving when modules compile in parallel. Two builds of the same project in one process intern different `__scope_N` text, visible in diagnostic report `Debug` output. The parity test in `profile_parity_tests.rs` therefore numbers these components by first appearance before comparing interned text.
- Investigate: whether any fingerprint, cache key or rendered name can read this text. If so, derive the component from a module-local counter or the scope arena frame id so identical input yields identical paths.

### JS relational Char patterns compare UTF-16 code units

- Found: const evaluator Phase 3 audit, 2026-11. Reproduced with the baseline binary.
- Where: JS match lowering for relational `Char` patterns in `src/backends/js/js_statement.rs`.
- Observed: `> '\u{FFFF}'` matches `'😀'` in a const function, which compares Unicode scalar values, but not at runtime. The JS output compares the strings with `>`, which orders UTF-16 code units, so astral characters sort below some BMP characters. `strings-and-characters-basic.mtf` says `Char` follows scalar values.
- Investigate: compare code points (`codePointAt(0)`) in the emitted relational pattern test, and check `Char` comparison operators for the same issue.

### Nested match ending without `else` fails to parse in a non-final arm

- Found: const evaluator Phase 3 audit, 2026-11. Reproduced with the baseline binary.
- Where: match arm body parsing in `src/compiler_frontend/ast/statements/branching.rs`.
- Observed: an arm `0 =>` whose body is an option match ending in `|x| => return x`, followed by `else => return 3`, reports MOTH-SYNTAX-0028. The same nested match parses when it ends with `else =>` or sits in the last arm.
- Investigate: how the inner match's closing `;` and the outer arm boundary are recognised when the inner match's final arm is not `else`.

### Value-producing match in a const body reports a vague diagnostic

- Found: const evaluator Phase 3 audit, 2026-11. Same output before and after Phase 3.
- Where: const-body eligibility in `src/compiler_frontend/ast/const_values/const_execution/eligibility.rs`.
- Observed: a value-producing match in a const body reports "content reference" at a capture or "value expression", not the value-producing match as the unsupported form.
- Investigate: name the value-producing match block itself as the unsupported form until const execution supports value-producing matches.

### Choice arm repeated after a guarded arm warns as unreachable

- Found: const evaluator Phase 3 audit, 2026-11. Reproduced with the baseline binary.
- Where: duplicate-variant tracking in `src/compiler_frontend/ast/statements/match_exhaustiveness.rs`.
- Observed: `Circle(radius) if radius > 5 => ...` followed by `Circle(radius) => ...` reports MOTH-RULE-0022 "Unreachable match arm", but the second arm runs when the guard rejects.
- Investigate: whether a guarded arm should mark its variant as covered at all, and align the warning with the exhaustiveness rule that guarded choice arms require `else =>`.

### Collection literal argument in a `#` constant initializer is a compiler bug

- Found: const evaluator Phase 3 tests, 2026-11. Reproduced with the Phase 0 baseline binary, so this is not a match regression.
- Where: the numeric-fragment completion guard for compile-time declaration initializers, raised during AST construction.
- Observed: `early #I32 = first({3, 12})`, where `first #|values {I32}| -> I32` is a const function, reports MOTH-INFRA-0001 "a numeric-dependent expression fragment reached a compile-time or non-runtime declaration initializer". The same call as `early = first({3, 12})` builds.
- Investigate: whether the collection literal's contextually typed numeric elements should complete against the parameter type before the required call folds, as scalar literal arguments already do.

### A caught failing cast is rejected in compile-time evaluation

- Found: const evaluator Phase 2 audit, 2026-11. Confirmed with the debug CLI before and after Phase 2, so this is not a Phase 2 regression.
- Where: `FoldContext::complete_cast` / `cast_complete` in `src/compiler_frontend/ast/const_eval/mod.rs` under a required boundary.
- Observed: `value #I32 = cast "x" catch then 0` and a const function returning `cast value catch then 0` called with `"x"` both report MOTH-RULE-0083 ("This builtin cast failed while evaluating a compile-time expression"). The same expression at runtime yields `0`. The cast documentation says `catch` handles conversion failure and only refuses to legalise unsupported conversions or statically invalid numeric work.
- Investigate: whether required compile-time evaluation should select the catch fallback for a value-dependent conversion failure. If it should, fix it once in the shared cast fold, so constant declarations and const-function bodies agree with runtime. If the rejection is intended, document it in the cast and constant-folding references.

### Repeated work in single-file namespace validation

- Found: Core/Builder package namespace and config filename final review, 2026-11.
- Where: `ModuleNamespaceSet::for_single_file` in `src/build_system/create_project_modules/module_namespace/mod.rs` and `single_file_namespace_inventory` / `eligible_child_directories` in `src/build_system/create_project_modules/source_tree_index.rs`.
- Observed: a selected `@entry.moth` or `@mod.moth` builds a `ProjectModuleGraph` and project namespaces only to validate reserved-root claims, then discards them. The registered-prefix collision probe for an ordinary selected file can list the same parent directories again for each registered source package prefix. No user-visible cost has been measured.
- Investigate only if measurements show a material cost. Bounded single-file mode uses the graph and namespaces to validate reserved-root claims, then discards them. Later synthetic traversal resolves dependencies from inventory keys and filesystem facts, so there are no later project namespaces to reuse. For ordinary files, the prefix probe already shares the root listing. Measure repeated listings below that root before grouping prefixes by shared parents.

### Deferred catch eligibility skipped in inactive static-`if` branches

- Found: failure-fact retention review follow-up, 2026-11. Confirmed defect; pre-dates this branch. Needs a design decision; see Investigate.
- Where: `StaticIfCandidate::publish` in `src/compiler_frontend/ast/module_ast/finalization/static_if_specialization.rs` rewrites the AST before `AstBuiltinFailureSummaries::compute` in `src/compiler_frontend/ast/module_ast/finalization/finalizer.rs`, so a discarded branch never reaches failure classification.
- Observed: a catch whose eligibility needs the deferred private-call check is rejected when live but accepted in an inactive static-`if` branch. Example: `flaky(1, 2) catch then 0` where an infallible generic `flaky` returns its divisor, placed inside `if false:`. Debug and release both accept it. Syntax, type and immediate catch-eligibility checks still run while parsing, so only the deferred private-call check is lost. The same ordering exists on `main` (read from code, not re-executed there), so this is not a `bug-fixes` regression.
- Attempted fix (reverted, 2026-11): collect the discarded branch's deferred checks before selection and validate them in AST finalisation against the retained summaries, using a conservative Active/Unknown/Inactive call graph.
  - Five review rounds each found a false rejection, where an inactive branch rejected what the live form accepts. Each came from AST summaries that differ from live convergence:
    - trait-bound generic instances;
    - unmaterialised generic instances reached through ordinary callers;
    - recursion into the callable that owns the discarded region;
    - handler reachability through that owner (`helper` comparing two `probe` calls under a safety catch);
    - a caller with its own escaping failure that depends on the owner.
  - Making every such dependency `Unknown` also skips live rejections for inactive custom-error compatibility and assertion-message checks.
  - A scope-parenting slip in `create_branch` also leaked discarded `$safe_math` evidence into HIR (`inactive_static_branch_evidence_is_pruned_while_the_selected_branch_is_kept`).
- Investigate: exact verdicts need the live, pre-selection facts. Options:
  - compute failure summaries over both branches before static selection (changes summary ownership and ordering);
  - carry the deferred checks across the AST/HIR boundary and validate them after convergence as validation-only facts, which needs a decision on how this squares with "inactive code publishes no generated request, HIR, effect";
  - reuse the assertion-validation materialisation lane for discarded catches, subject to the same decision.

  `docs/docs/branching/statement-if.mtf` requires both bodies to be frontend-validated before a known `Bool` selects one, and `tests/cases/catch_and_inactive_branch_source_validation_rejected` already pins syntax and type validation in discarded branches. Add matching debug/release rejection cases.

### JS projected call arguments still retarget when a later argument rebinds the base

- Found: projected-alias backend audit, 2026-11. Confirmed by Node probe on the current branch; pre-dates it. Needs a design decision; see Investigate.
- Where: `src/compiler_frontend/hir/hir_expression/calls.rs` keeps an earlier projected or copied argument as an unevaluated place load while a later argument's nested call is flushed into the prelude before the enclosing call. The JS computed-place helpers capture their base storage when constructed (`src/backends/js/runtime/places.rs`), so the retarget comes from that construction happening after the hoisted later argument ran.
- Observed: `read_count(holder.items, replace(~holder))` returns the replacement's length. The same ordering affects whole-place copies (`read_count(copy holder, replace_holder(~holder))` returns 3 instead of 2) and nested fields (`read_count(outer.inner.items, replace_inner(~outer.inner))`). Authored index projections cannot reach this path because places only carry field projections.
- Attempted fix (reverted, 2026-11): capturing earlier operands into compiler locals in HIR before borrow checking fixed the runtime order but kept changing borrow verdicts relative to the existing behaviour. Four review rounds each found new divergences:
  - mutable-capture actor (`old = pair.first; set_first(~pair.first, stamp())` accepted);
  - alias liveness through a capture (`observe(writer.first, rewrite(~pair))` accepted);
  - copy independence (`keep(copy pair, stamp())` falsely tied to `pair`);
  - early shared borrow (`set_first(~pair.first, read_slot(writer))` rejected);
  - same-call copy read (`f(copy pair, ~pair, stamp())` accepted);
  - designated cells after scope exit;
  - a copy checked at the capture instead of at the call;
  - copy-source liveness across a branch merge;
  - `__hir_tmp_*` named in a diagnostic.

  The current borrow verdicts are computed on late-evaluation HIR, so any HIR-level capture alters what the checker sees.
- Investigate: decide where source-order operand evaluation is owned. Options:
  - borrow-check the existing HIR unchanged, then reorder operand evaluation in a post-borrow-check lowering step consumed by the JS backend (verdicts unchanged by construction);
  - redefine borrow verdicts under source-order evaluation as a deliberate language decision, then capture in HIR.

  Either way, add Node-executed cases for the projected, copied and nested-field shapes.

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

### Registered package roots below the project root are rediscovered as project sources

- Found: namespaced exports follow-up F5 audit, 2026-11. Confirmed by code trace and a fixture artefact; pre-dates this branch. Needs a design decision; see Investigate.
- Where: the project source walk in `src/build_system/create_project_modules/source_tree_index.rs` does not exclude canonical roots already registered as source packages.
- Observed: a registered package directory inside the project root is also indexed as a project-local support module, so its authored warnings appear twice; a co-located package `@entry.moth` could enter the project graph and become an active output. Directories at an exact registered prefix are also rejected as `EntryRootPackagePrefixCollision` (`src/vendor/lib` registered as `@vendor/lib`).
- Attempted fixes (reverted, 2026-11), both meant to implement the decision to exclude these roots from the project walk:
  - Skip-policy exclusion (registered canonical roots dropped from every walk). The folder vanished from consumers that still need it:
    - file values and bounded single-file provider imports lost the package boundary;
    - single-file prefix probing missed collisions;
    - case-only siblings were dropped;
    - synthetic fallback admitted package-private files;
    - descendant scans followed directory cycles;
    - first-error order changed.
  - Package-owned identity marker (root kept as a non-compiled identity with a private boundary key). Remaining divergences:
    - a `$export` route from the marker facade reaches an unfinished slot (internal error);
    - a package registered at its own folder prefix (`src/lib` as `@lib`) reports `MOTH-IMPORT-0006`;
    - self-match probes hide later real collisions;
    - single-file mode accepts the marker's normal facade and binds where the existing code reports ambiguity;
    - reserved-root diagnostics depend on the mode;
    - a package root symlink into its own module is rejected;
    - nested package validation order changes.

  The root sits inside the project tree, so it interacts with every Stage 0 consumer: routes, namespaces, reserved roots, single-file inventory, symlink validation and package indexing order.
- Investigate: decide the ownership model before implementing. Options:
  - reject a registered root inside the project walk with a typed configuration diagnostic (smallest; disallows vendoring inside `src`);
  - split module identity from compile-graph membership (ModuleRootTable keeps package roots for privacy, the graph holds project modules only), which needs a Stage 0 contract change for dense `ModuleId`;
  - keep the identity marker, but define each Stage 0 consumer's behaviour for marker roots explicitly first.

### Unused generic function bodies accept standalone `$in` helpers

- Found: runtime template helper escape fix review, 2026-11. Confirmed by probes on this branch and pre-dates the fix.
- Where: generic function body validation. A generic function's body is finalized only when an instance is materialised, so an unused generic body never reaches the final-value helper rejection in `normalize_expression_template_node`.
- Observed: `render type T |value T| -> String: return [$in style: [: red]];` with no call is accepted, while `result = render("x"); ~[:[result]]` reports MOTH-SYNTAX-0022 at the helper. The same split occurs for a runtime-dependent helper: `render type T |value T, x String| -> String: return [$in style: [x]];` is accepted while unused and rejected once instantiated.
- Investigate: validate the authored generic body once before instantiation or materialise every generic body for validation without publishing an instance. The helper shape is not type-dependent, so a pre-instantiation validation pass may suffice. Check that it does not reject generic bodies that are valid only under an instance's type arguments.

### Imported const template helper defaults lack a donor `$in` label

- Found: runtime template helper escape fix verification, 2026-11. Confirmed by a two-module probe and pre-dates this branch.
- Where: `fold_signature_default` in `src/compiler_frontend/ast/statements/functions.rs` validates the rebuilt default template, and `PublicConstTemplate` in `src/compiler_frontend/folded_value.rs` carries no donor `$in` site. The primary diagnostic correctly points at the importing reference because the stored const insert is legal in its donor, but there is no related label at the authored directive.
- Observed: `helpers/utils.moth` declares `insert #= [$in style: [: red]]`, `helpers/@mod.moth` exports `@utils insert` and `@entry.moth` uses `@helpers insert` as `render |value String = insert| -> String`. The compiler reports one MOTH-SYNTAX-0022 on the `insert` reference in the default at `@entry.moth:3:24`. It does not point to the authored `$in` at `helpers/utils.moth:2:5`.
- Investigate: carry diagnostic provenance for the donor `$in` site on public const templates, separate from semantic equality and fingerprints. Attach it as a related label and keep the primary at the invalid use site. The fallback to the use-site span is correct when no authored site is available.

### Multi-bind through a mutable alias is rejected at runtime

- Found: const multi-return phase, 2026-11. Confirmed by a runtime-only probe; pre-dates this branch.
- Where: HIR borrow checking of a multi-bind whose existing target is a mutable alias. The hidden result temporary is read while the alias is still live.
- Observed: in an ordinary function, `base ~= value + 0`, `view ~= base`, `~view, extra = pair(3)`, then `seen = view * 10 + extra` reports MOTH-BORROW-0003 at the multi-bind: "Cannot read `__hir_tmp_2` while mutable alias `view` is still needed later". The docs say updates through mutable aliases write through to their referents. The same body in a const function evaluates and writes through (`base` becomes 3), so const and runtime acceptance diverge. The diagnostic also exposes a compiler temporary name.
- Investigate: the result temporaries are fresh values and should not conflict with the alias. Check how multi-bind lowering orders the temporary reads against the alias write, and keep generated names out of user diagnostics.

### Const function bodies skip runtime borrow rules

- Found: const multi-return phase, 2026-11. Confirmed on the Phase 3 binary, so it pre-dates this phase.
- Where: const declaration preparation. A const body is executed by the typed-AST interpreter and never reaches HIR borrow checking.
- Observed: `bump #|value I32| -> I32: base ~= value; ~base = base + 1; return base;` compiles and folds, while the same body as an ordinary function reports MOTH-BORROW-0007 "Cannot mutably access immutable local `value`". The multi-bind alias entry above is a second instance of the same divergence.
- Investigate: const bodies need the same access verdicts as runtime bodies. Options: borrow-check the const body's HIR without emitting it, or run the AST-level access rules the HIR checker enforces during const preparation.

### Retained stale paths can make the next output manifest invalid

- Found: output-publication Correctness audit, 2026-11, at `d6b323082777e73663c8eacd6e03852f97d7e769`. Confirmed by a concrete source trace; the filesystem regression has not been executed.
- Where: `src/build_system/output/manifest.rs`: `remove_manifest_tracked_stale_artifacts`, `finalize_output_cleanup`, `write_build_manifest` and `parse_manifest_paths`. The current output batch is checked by portable identity in `writer.rs`, but finalisation combines managed and retained paths using native `PathBuf` equality. The reader rejects duplicate ASCII-case-folded output identities.
- Observed by trace: on a case-sensitive filesystem, start with a valid HTML dev manifest owning `page.html`. Replace that file with a directory (a stale non-regular node the cleanup must retain), then publish managed files `PAGE.html` and `removed.html` under the same owner. The new destinations are valid and distinct. The old directory is not a current managed path, so cleanup retains `page.html`. Finalisation extends the current path set with it and writes both case spellings, which the next read classifies as `RecoverableWithOwner(InvalidMetadata)`. A third build dropping `removed.html` cannot perform normal stale cleanup and loses that file's tracking when it rewrites the manifest. This violates the build output ownership and portable manifest identity invariant. Existing `failed_stale_deletion_remains_owned_in_next_manifest` and `case_variant_manifest_paths_enter_recoverable_mode_with_owner` in `src/build_system/tests/build_cleanup_tests.rs` test the two sides separately.
- Investigate: reproduce through the existing writer/cleanup harness on a case-sensitive temporary filesystem, including the third build. Establish the ownership policy for a retained native path whose portable identity conflicts with a new managed path, then enforce it before publishing a self-invalidating manifest. Do not delete a directory, guess that case variants are the same native file or silently drop a retained ownership record. A demonstrated pre-publication rejection or a valid ownership-preserving manifest representation would disprove the faulty trace; current batch-only collision checks do neither. Preserve normal case-only renames where the stale file is successfully removed, foreign-owner rejection and non-regular-node retention.

### MON reader charges map entry nodes after allocating the raw map

- Found: standalone MON codec resource-accounting Correctness audit, 2026-11, at `f6e65e4d6d84faf03ee19f3efa65c6726777823b`. Confirmed by source trace, not an executed reproduction.
- Where: `crates/moth-mon/src/reader.rs`, `Parser::parse_container_value` and `parse_map_tail` append raw key/value pairs without the extra map-entry node charge. `validate_map` charges it only after the complete raw document has been parsed. `schema.rs` charges the corresponding entry before growing a programmatic map and before copying a prepared default entry.
- Observed by trace: prepare a root record with required field `m` of type `Map<Bool, Bool>` (Rust `SchemaType::Map`), `max_nodes = 6` and otherwise default limits. Schema preparation fits (five charged nodes). Decode `(m = {true = false, false = true})`: parsing charges six nodes (root, map and four values) and constructs both raw entries despite the logical total being eight. The subsequent validation rejects the map with `NodeBudget`. With the same document followed by ` tail`, parsing still constructs both entries and returns `TrailingInput` before the additional entry charges are reached. The MON format and data-layout authority require checking the additional node cost per map entry before the affected allocation or expansion. This is a late budget check, not an unrestricted allocation or an incorrect successful decode.
- Investigate: move the entry-node charge to the reader's first and subsequent map-entry construction points, after establishing map kind and before growing the raw entry list. Do not charge collection items or empty maps as entries, and remove the matching late charge rather than charging the same logical node twice. Add a public API boundary test with the above limits and valid/trailing-input cases, an exact eight-node successful round trip and a one-entry/empty-map control. Compare the parser and programmatic/default map paths. A pre-allocation entry charge already present on both reader paths would disprove this trace; the current charge sites are in validation only.

### MON encoding may undercount variable-size record and variant path copies

- Found: standalone MON codec resource-accounting Correctness audit, 2026-11, at `f6e65e4d6d84faf03ee19f3efa65c6726777823b`. Unconfirmed accounting-policy concern from source reading.
- Where: `crates/moth-mon/src/schema.rs`, `complete_record` charges a matched field name once before cloning it into a path and the completed record; the choice arm of `complete_value` charges the variant once before cloning it into the completed value and a path. `clone_default` explicitly charges both copies, and `reader.rs` separately charges field-name path and result copies. Schema preparation intentionally has its own once-per-name policy.
- Investigate: establish whether the encoding/completion budget is intended to cover both variable-size logical copies for supplied values, as it does for copied defaults. A root record with one long identifier, a Bool value and `max_decoded_bytes` equal to the identifier's byte length is a minimal starting point: preparation fits and the completion loop has only one charge for two clones. Compare the same repeated record shape supplied directly and filled from a prepared default. If the one-charge policy for programmatic values is deliberate, retain it and clarify the invariant rather than trying to equalise unrelated reader/writer limits. No exact allocator-memory claim or guarantee that all outputs fit the same input-byte budget is implied.
