# Synthetic entry function identity

> For implementation agents: work in reviewed slices and follow the repository agent workflow, style guide and code review guide. This plan frees `start` as an ordinary user identifier without changing module activation.

**Goal:** replace the named synthetic `start` declaration and its source-path lookups with an actual compiler-owned entry identity. HIR allocates a `FunctionId` and keeps it in `HirModule.start_function`.

**Architecture:** header preparation keeps the existing typed `HeaderKind::StartFunction` and the captured entry-body tokens. AST distinguishes entry work from authored functions with a typed node. HIR registers that node directly as one function with `HirFunctionOrigin::EntryStart`. No user-spellable string or synthetic `PathId` identifies the entry.

## Status and placement

- Status: queued. Implementation has not started.
- Activation: explicit approval after the preceding diagnostic improvements. This entry must not silently pause or reactivate another plan.
- Current slice: Phase 0, refresh the live code and test baseline.
- Closeout: retire this plan and its roadmap item after validation and durable documentation changes.

The companion PR has already removed the unrelated `fn` reservation and the statement-position heuristic that blocked its use as a function name. Don't repeat that work in this plan. Verify it at the final test gate.

## Authority and fixed contracts

Read `AGENTS.md`, `docs/compiler-design-overview.md`, `docs/build-system-design.md`, `docs/compiler-data-layout-design.md`, the relevant developer style, testing and validation guides, the language cheatsheet, the normal-module and entry-runtime references, both progress matrices and `index.md`.

Preserve these existing semantics:

- Only normal roots and eligible synthetic single-file roots have an implicit entry. Support modules, package facades and API-only imported roots don't.
- A normal root can contain dormant runtime work and runtime template fragments. The builder selects and activates the entry once. Runtime fragments retain source order.
- The implicit entry retains its existing builtin `Error!` contract and success fragment slots. The compiler validates dormant work before publication.
- `config.moth` remains a special compile-time source. Authored runtime statements still receive typed config diagnostics instead of silently executing.
- A user declaration named `start` has ordinary scope, collision, call and export semantics. Calling it never activates the implicit entry.
- The HTML/Wasm `moth_start` export is a separate target ABI spelling. Keep its host bootstrap and return ABI unchanged.

No new source syntax, global entry registry, cross-stage `FunctionId` or synthetic name namespace is authorised.

## Evidence and owners to inspect

The plan was researched against these current owners. Refresh all references at activation and search for any newly introduced callers.

| Owner | Current issue or established responsibility |
|---|---|
| `src/projects/settings.rs` | Defines `IMPLICIT_START_FUNC_NAME = "start"` |
| `src/compiler_frontend/symbols/compiler_symbols.rs` | Preseeds the spelling in each string table |
| `src/compiler_frontend/headers/file_state.rs` | Captures start-body token ranges and emits a typed header |
| `src/compiler_frontend/headers/module_symbols.rs` | Fabricates the `source_file/start` declaration |
| `src/compiler_frontend/headers/symbol_collection.rs` | Rejects authored root declarations named `start` |
| `src/compiler_frontend/ast/module_ast/emission/emitter.rs` | Emits an ordinary named `NodeKind::Function` for the entry |
| `src/compiler_frontend/ast/module_ast/finalization/finalizer.rs` | Recreates the entry path for validation |
| `src/compiler_frontend/ast/module_ast/finalization/const_fact_collection.rs` | Compares paths to classify root-private constants |
| `src/compiler_frontend/single_source_compilation/config.rs` | Checks the function name to reject authored runtime config work |
| `src/compiler_frontend/hir/hir_statement/declarations.rs` | Searches named HIR functions to identify the entry |
| `src/compiler_frontend/hir/module.rs` and metadata | Already store `Option<FunctionId>` and `EntryStart` |
| `src/compiler_frontend/compiler_messages/` | Owns the `ImplicitStart` reserved-name diagnostic |
| JS, Wasm and HTML build paths | Consume entry role/ID and generate their own external symbols |

Check the compiler-symbol tests, AST/HIR fixtures, generated-function and public-interface projections, dependency graph, borrow/lifetime summaries, source ordering, backend symbol lookups and the architecture-boundary source audit. Do not equate historical source name assumptions with accepted semantic requirements.

## Identity rules

1. The typed header marker is the identity before AST. It has no dependency edge, no user declaration symbol and no public interface entry.
2. AST must retain an explicit entry role independently of name spelling. Prefer a distinct `NodeKind::EntryStart` carrying the existing function signature/body shape. Reuse the same body parser and validator as ordinary functions rather than duplicating them.
3. `FunctionId` is allocated once during HIR declaration preparation. It is a HIR-local identity and must not be guessed, preallocated or cached in the header or AST.
4. Assign `HirModule.start_function` directly from the typed AST entry when HIR creates the function. Mark `HirFunctionOrigin::EntryStart`. Reject missing or duplicate entries for a root requiring one and reject entries in non-entry modules.
5. Remove the scan through `functions_by_name` and every name-component comparison that decides entry identity. Ordinary `start` user functions remain in ordinary lookup maps.
6. Never replace the old name with another hidden source-like name: no `#start`, `_`, magic string ID, `PathId::ROOT` sentinel, shadow exception or inaccessible generated declaration.
7. Separate debug presentation from semantic identity. A debug display may say "entry" but no validation, export, lowering or linking decision depends on that label.
8. Preserve exactly one HIR origin and ID for the entry. A user-authored `start` function is another ordinary `FunctionId` with `Normal` origin.
9. Maintain the existing typed source spans and diagnostics. Missing compiler-owned entry bookkeeping is an infrastructure error, never a fabricated user error.

## Implementation sequence

### Phase 0 - establish baseline and choose the narrow AST handoff

- Record the active revision, toolchain and existing targeted test outcomes in untracked working notes.
- Audit every `IMPLICIT_START_FUNC_NAME` use, every path or name comparison to `start` and every source symbol registration or synthetic path producer.
- Inspect all exhaustive AST `NodeKind` matches, functions that rely on callable paths and the current HIR function allocation. Confirm the smallest typed AST entry form. A dedicated `EntryStart` kind is preferred, but an equivalent typed field is acceptable only if it reduces churn without keeping a synthetic source name.
- Record required tests in their existing suites. Prove the desired distinction before deleting the compatibility guard.

### Phase 1 - preserve headers and change AST ownership

- Keep the existing `HeaderKind::StartFunction` token-sequence lane and dependency sort position. Delete the fabricated declaration from module symbol collection.
- Emit a compiler-owned entry AST kind with the existing implicit signature, error contract, return behavior and runtime body. Reuse current function-body parsing and transformation logic.
- Audit static-`if` specialization, terminality, assertion handling, constant evaluation, function failure summaries, source-span projection and generated-function handling. These must process the entry exactly once.
- Replace entry-path comparisons in constant-fact collection with typed entry recognition. Keep direct entry-body declarations as private top-level facts and nested declarations as body-local facts.
- Replace name checks in `config.moth` compilation with the entry kind. Preserve rejection of authored runtime work, successful compile-time configuration and original diagnostic spans.
- Keep the synthetic entry out of declaration visibility, public exports, imports and generic declaration identity.

### Phase 2 - assign HIR identity directly

- Register the typed AST entry as an ordinary HIR function body, allocate its `FunctionId`, set `HirModule.start_function` immediately and classify it as `EntryStart`.
- Delete `resolve_start_function` and any name-index or parent/component scans used only to discover the entry. Regular authored functions keep their usual mapping and lookup.
- Audit HIR validation, CFG entry block, implicit success/error slots, borrowed values, static analysis, reachability, link facts and generated-call summaries. Entry metadata must not acquire a public callable origin.
- Remove synthetic source-path requirements from HIR side tables when they aren't needed. Keep diagnostics usable with the original entry-body spans and the typed role, not a fake declaration path.
- Verify the JS and Wasm code generators select the entry by HIR ID/origin. Preserve externally observed `moth_start` naming, runtime fragments and failure propagation.

### Phase 3 - release the user name and delete old scaffolding

- Remove `IMPLICIT_START_FUNC_NAME`, the fixed string-table preseed, synthetic start declaration paths and all obsolete ID/path assumptions.
- Delete the active-root reservation in `validate_declared_name` and the now-dead `ReservedNameOwner::ImplicitStart` diagnostic, rendering branch and tests.
- Remove any special-case path equality or shadow workaround that solely protected the old synthetic entry. Prefer fewer branches and maps, not a compatibility adapter.
- Update the architecture and module-entry references with the typed representation. Preserve final language docs: `start` is an ordinary identifier, `fn` was freed in this PR, and the reserved-names reference excludes both.
- Check the full diff for unused types, duplicate body walkers, stale helper APIs and unnecessary allocations. Remove dead code instead of retaining migration fallbacks.

### Phase 4 - test, validate and retire

Use existing unit tests and integration fixtures. Cover each case with direct typed assertions where possible.

| Case | Expected result |
|---|---|
| Authored root function `start` called from top-level work | Resolves to the authored callable |
| Authored `start` plus implicit entry and runtime template | Two distinct HIR IDs, exactly one entry |
| Authored `start` exported and imported by another module | Normal exported call, no synthetic export |
| Root-local value or nested binding named `start` | Ordinary name rules, no special rejection |
| Duplicate user names in one scope | Ordinary collision diagnostic |
| Support, facade and API-only roots | No entry ID or entry-origin function |
| Synthetic single-file root | Exactly one entry with existing error/fragment contract |
| `config.moth` containing runtime statements | Same typed config rejection as before |
| Entry private constants and nested constants | Correct top-level versus local const facts |
| Normal root fallible work and runtime fragments | Stable failure behavior and fragment order |
| JS/HTML and Wasm runtime execution | Entry called once, stable `moth_start` ABI |
| Repeated compilation and remapping | Deterministic typed entry identity per module |
| Authored `fn` function declaration and call | Prior reservation removal still valid |

- Run targeted tests first, then `just fmt-check`, `just clippy`, `just validate-common` and `just validate-full` when feasible. In particular, run the established JS/Wasm entry integration cases and docs check.
- Refresh the source and docs index for `IMPLICIT_START_FUNC_NAME` and stale name-based entry identity. Only descriptive strings and host ABI spellings should remain.
- If function preparation, name tables or ID allocation change in a perf-sensitive path, compare existing frontend benchmark cases. This change should remove work, not introduce another scan, cache or retained map.
- Record the exact validation evidence and implementation revision. Update progress, remove the finished roadmap entry and retire this plan under the standard closeout process.

## Acceptance gate

Success means a user can declare, call and export `start` anywhere an ordinary identifier is permitted, while the compiler's implicit entry has no source-level name or namespace path. Header and AST use typed entry roles, HIR owns one dedicated `FunctionId` and backend ABI behavior is unchanged. No sentinel spelling, duplicate entry lookup or extra identity registry remains.
