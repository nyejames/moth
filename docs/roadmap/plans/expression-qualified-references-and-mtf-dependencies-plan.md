# Expression-qualified references and explicit Moth template dependencies

> Status: Accepted design for implementation. Companion prerequisite to `docs/roadmap/plans/fixed-root-roles-implementation-plan.md`. The fixed-root work must not be declared complete until the changes and migration in this plan are complete.
>
> Reviewed source: `nyejames/moth`, PR #31 `plan/fixed-root-roles-implementation-hardening` at `7f4990dd805f98e134fe338567d6f62aa33094da` on 8 November 2026. `main` was `12b936de9989f0fbc18c30079ac7f97307ef0286`. The PR has already started internal three-root-role plumbing, with a temporary legacy combined-root bridge. Reinspect the current branch head before applying any patch. File locations describe the inspected revision, not immutable interfaces.
>
> Adopted at this path, linked as a hard prerequisite from the fixed-root plan. Implement on the existing fixed-root work branch rather than starting a second competing root migration. This document is an implementation plan, not a statement of delivered support.

## 1. Goal and boundaries

Add an explicit, one-symbol, expression-level qualified reference:

```moth
site = @settings site_name
result = @math square(12)
logo = @images/logo.svg
```

The same reference syntax works in ordinary expression positions and Moth template heads, including the implicit body of a `.mtf` file:

```text
# Welcome to [@settings site_name]
```

The expression resolves the named declaration using the **existing dependency path, module visibility and import binding rules**. Unlike an ordinary dependency clause, the qualified reference does **not** install a file-local name, alias or namespace. A repeated qualified reference is allowed and does not conflict with a same-spelled declaration from another provider.

A `.mtf` source remains an ordinary module-owned semantic source whose entire body becomes one synthetic compile-time `content #String` constant. Its explicit qualified references participate in the **same Stage 3 declaration dependency sorter** as normal constants, including same-module cycles and cross-module provider cycles. No separate `.mtf` visibility graph, template constant registry, filename convention or scope-specific ordering algorithm is permitted.

Remove implicit **same-directory facade-export constants** from `.mtf` and the general configurable implicit-template-constant provider mechanism. Preserve the compiler-standard flat HTML helper prelude, backed by the official `@html` source package. The implementation must not change the meaning of ordinary file values, relax fixed-root isolation or add arbitrary local visibility to `.mtf`.

### Authorities

Read before editing:

- `docs/compiler-design-overview.md` and `docs/build-system-design.md`, especially Stage 0 preparation, Stage 3 declaration ordering, module graphs, public interfaces, file values and source-package boundaries.
- `docs/roadmap/plans/fixed-root-roles-implementation-plan.md` and the current implementation branch diff. Keep the accepted `@mod`, `@package`, `@entry` roles and one-module/one-transaction rule.
- `docs/roadmap/plans/const-functions-plan.md` for future compile-time calls. This plan **does not** implement const functions.
- `docs/docs/packages/{dependency-clauses,dependency-paths,module-visibility,public-reexports}.mtf` and `docs/docs/resources/{file-paths,file-values}.mtf`.
- `docs/docs/moth-templates/{moth-template-files,template-scope,template-scope-basic,moth-template-limits,content-dependencies}.mtf`.
- `docs/developer-docs/style-guide/{style-guide,testing,validation}.mtf`, `moth-code-review-guide.md` and `index.md`.

## 2. Normative language design

### 2.1 Syntax and precedence

An expression-qualified reference consists of an authored `PATH` token followed on the **same physical line** by exactly one `SYMBOL` token:

```text
qualified_primary = dependency_path  selected_symbol
```

Spaces and tabs may separate the two tokens. A newline may not. An ordinary expression suffix can follow the selected symbol, subject to existing syntax and semantic rules:

```moth
name = @settings site_name
color = @settings palette.background
value = @math square(12)
conditional = if ready then @settings site_name else "Fallback"
```

- `@path symbol` is one expression primary, not two independent expressions and not a declaration that imports `symbol` for later statements.
- The provider path must satisfy the same source/provider path and visibility contracts as an explicit direct-selection dependency clause. Use an extensionless source/module/package path (`@settings`, `@lib/math`, `@html`) or an explicit-extension **registered provider** path if that provider already supports direct symbol selections. A resource file is not a provider.
- An explicit-extension file value without a selected symbol keeps its existing meaning, including `.mtf`, `.md`, resource files and `@/`. A bare extensionless path in expression position remains invalid. A selected symbol after a resource path must produce a provider/qualified-reference diagnostic, not silently reinterpret the resource as a namespace. Preserve the normal requirement that provider files must be registered and that source dependencies use an extensionless path.
- An authored `.moth` extension remains invalid in dependencies. `@/` remains a site-root value and cannot select a declaration.
- The selected symbol must be a single flat name. No multiple selections, trailing commas, `as` aliases, wildcard selections, implicit namespace value, `@path.symbol` shortcut or higher-order import object is added.
- Exactly one selected declaration is resolved per occurrence. Existing postfix operations (`(...)`, `.field`, choice construction where already legal) apply to the resolved declaration through ordinary expression machinery. Nothing new becomes callable or value-like just because it is qualified.
- The spelling does not change arithmetic/operator precedence: a qualified reference is an operand. It is *not* a string path followed by an application operator. Existing file-value path expressions remain operands too.
- Reject malformed selections with diagnostics at the path, selected symbol or first illegal suffix as appropriate. Do not fall back to a file value when an apparent qualified reference fails semantic binding.

The lexer already emits `PATH` and `SYMBOL` separately. Avoid a new token kind, a second path parser or special template-only syntax.

### 2.2 Permitted positions

Qualified references are supported wherever the parser is already expecting an ordinary value expression, including:

- Constant and runtime binding initialisers, returns, arguments, conditionals and other nested expressions.
- Function, method and constructor calls only to the extent those operations already accept the selected declaration.
- Template-head expressions in normal `.moth` templates and the synthetic `.mtf` body.
- Const-required expressions, provided the resolved target and the entire expression satisfy the existing compile-time value rules.

**Top-level statement-position qualified calls are out of scope.** The header classifier currently treats a statement-boundary `PATH` as an ordinary dependency clause. Preserve that rule and reject `@math square(12)` when it is an unassigned top-level statement. To make a top-level call, the author must use an ordinary dependency clause and then call the bound name. Parentheses make a future call-vs-clause extension syntactically feasible, but this plan does not accept it.

A `.mtf` file still contains content rather than authored top-level Moth statements. Its qualified references appear inside template heads, not as independent lines of import declarations. An unbracketed `@settings site_name` in Markdown prose remains prose.

### 2.3 Visibility, identity and binding

A qualified reference is a *single-use direct selection* through the same namespace provider that an authored direct-selection dependency clause would select:

- An ordinary `.moth` file in the same structural module may select visible declarations from an ordinary helper without requiring them to be exported.
- A `.mtf` file has the same module ownership and helper visibility as a role-neutral `.moth` file. `intro.mtf` can therefore use `[@settings site_name]` from `settings.moth` in its own module, without `@mod` or `@package`.
- Cross-module and registered-package selections require the provider's appropriate public facade/interface. Preserve the consumer-position support-package rule, namespace conflict rules, provider registration and source-package boundary isolation.
- A root-local declaration directly in `@mod`, `@package` or `@entry` is never accessible to ordinary helpers or `.mtf`, including via a qualified reference, role alias, direct special-file path or an indirectly leaking helper. The fixed-root isolation contract is unchanged.
- An entry-only child remains unimportable because it has no API surface. A support-only child remains importable only by eligible consumers through its support surface. Do not use a fallback public interface.
- A selected symbol may have the same name as a local symbol, another provider's selected symbol or a prelude symbol: the **qualified occurrence introduces no visible local name** and therefore does not participate in file-local shadowing/collision rules. Ordinary declarations and explicit dependency clauses retain the existing no-shadowing policy.
- Repeated selections of the same provider share provider identity and prepared semantic facts. Origin identity, exported aliases, type/effect/lifetime summaries and callable contracts remain those of ordinary direct-selection binding. No new public declaration identity is minted for the syntax.
- A symbol that is type-only, trait-only, method-only or otherwise invalid in an expression fails according to the existing expression/receiver rules. It is not promoted to a value. No new type-position qualification grammar is added in this plan.
- `@provider symbol` is legal only if that **specific** provider exposes `symbol` under the current consumer's access rules. It does not grant access to unrelated symbols from that provider or its private implementation.

### 2.4 `.mtf` constant and ordering contract

A `.mtf` file retains exactly one synthetic `content #String` declaration, created in `headers/moth_template_prepare.rs`. It remains compile-time-only and is prepared/sorted by the existing module compilation service.

Every qualified occurrence in the `.mtf` body's template heads is a dependency of its generated `content` constant. A selected same-module compile-time constant must sort before `content` can fold. A cross-module qualified reference must make the selected provider a graph dependency before semantic compilation of the consumer. A runtime value/function remains invalid in this const-required context, even if it can be named.

**Both dependency directions are real and must be checked.** For example:

```text
page/
    @entry.moth
    settings.moth
    intro.mtf
```

```moth
-- settings.moth
title #= @intro.mtf
```

```text
# [@settings title]
```

The synthetic `intro/content` constant requires `settings/title`, while `settings/title` requires `intro/content`. This is a **user-facing circular dependency**, not a hardcoded `.mtf` exception, an unresolved symbol, an implicit import fallback or an internal panic. The same graph must also detect self-qualified `intro.mtf` -> `@intro content` and longer cycles through multiple `.moth` and `.mtf` files.

A content source may itself be imported or referenced from elsewhere, including through an API facade. Cycles across structural modules must be caught by the existing Stage 0 module graph/provider ordering, while cycles inside one module must be caught by Stage 3 topological sorting. Keep explicit content-path `@intro.mtf` dependencies and qualified `@intro content` dependencies pointing to the **same** synthetic declaration identity. Never parse or fold the content twice.

The existing direct same-file forward-constant restriction still applies to qualified references that resolve back into the same source file. The syntax must not bypass same-file ordering, self-reference or cycle diagnostics. Independent files within a module use the normal declaration graph, not a blanket file-order rule.

The existing imported-constant and future const-function evaluation contracts apply without an extra template interpreter. A future `[@math square(12)]` becomes legal only when the const-function implementation has validated, ordered and evaluated `square` under its own accepted rules. Do not implement that future plan here or claim current runtime calls can fold.

### 2.5 HTML prelude and removal of old scope

Keep the **flat, compiler-standard HTML template prelude** for `.mtf`, so current syntax such as `[p: Text]` and `[codeblock, $code("moth"): ...]` continues to work. It must resolve to compile-time declarations exported by the existing official `packages/html/@mod.moth` source package, with proper provider identity and validation. Do not duplicate its constants in Rust or create a second HTML template implementation.

Remove both forms of the old implicit constant exposure:

1. Implicit constants exported by the unique same-directory root/facade, including the unpaired-root rule and the paired-root suppression rule.
2. Generic *builder-configurable* implicit constant imports from arbitrary packages (the `available_in_templates` / `implicit_template_scope_source_packages` feature), which become redundant when authors can select `@provider value` explicitly.

Retain only the fixed `@html` convenience prelude in `.mtf`; all non-HTML application data uses explicit qualified references. This is a deliberate language-standard exception, not a general-purpose root-visibility rule. Keep duplicate-name handling for actual flat `@html` prelude names and ordinary local bindings. A qualified reference must never collide merely because its selected name matches a prelude name.

**Current repo discrepancy to handle explicitly:** `HtmlProjectBuilder::frontend_surface` registers `@html` as a **Builder-origin** Moth source package. Its `packages/html/@mod.moth` source also imports builder-backed `@web/canvas` operations. The accepted `.mtf` HTML prelude is **compiler-standard**, not an arbitrary builder opt-in. Template scope policy and `PackageOrigin` metadata are distinct concepts.

First make the template scope policy fixed using the existing official provider. Do not blindly change `PackageOrigin::Builder` to `Core` or register all of `@html` for every builder without auditing its `@web/canvas` dependency. If global Core-origin reclassification is necessary, isolate dependent runtime helpers through the existing package architecture instead of copying constants or manufacturing an `@html` stub.

Builders that enable `.mtf` but lack the official `@html` package need a clear policy: make the standard prelude available through existing registration without importing unavailable web runtime packages, or reject unsupported builder configuration with a capability diagnostic. Never silently omit the HTML prelude while claiming a compiler-standard contract. Record the actual ownership policy in docs and final handoff.

## 3. Verified repository ownership map

These are existing implementation anchors, not instructions to introduce a parallel subsystem.

**Path syntax and expression parsing**

- `src/compiler_frontend/paths/const_paths/mod.rs`: tokenizer-facing `parse_file_path`; whitespace terminates the `PATH` token, and dependency selections remain separate tokens.
- `src/compiler_frontend/tokenizer/lexer.rs` and `tokenizer/schema.rs`: `@` dispatch and compact `TokenTag::PATH` with a `PathSyntaxId`.
- `src/compiler_frontend/ast/expressions/parse_expression_dispatch.rs`: ordinary `TokenTag::PATH` currently goes directly to `resolve_file_value`.
- `src/compiler_frontend/ast/file_value_resolution.rs`: Stage 0-backed file-value implementation; `PreparedFileReferenceClass::Extensionless` is deliberately rejected as an expression. Preserve this for **bare** extensionless paths.
- `src/compiler_frontend/ast/templates/template_head_parser/{head_parser,head_expressions}.rs`: template heads currently treat `PATH` as standalone file-value expressions. Reuse the ordinary expression parser for qualified operands instead of implementing a separate template-only qualifier.

**Preparation, discovery and binding**

- `src/compiler_frontend/headers/{file_parser,dependency_clause_syntax,file_dependency_clauses,dependency_target,dependency_paths}.rs`: source header classification, retained direct-selection syntax, exact source spans, provider path validation and dependency shell IDs.
- `src/compiler_frontend/paths/{path_syntax,file_references}.rs`: the file-local path table and classification of paths **not consumed** by dependency clauses. Extend its consumed-path contract to exclude qualified-reference occurrences; don't run the physical file-value resolver for them.
- `src/compiler_frontend/headers/{moth_template_prepare,synthetic_content_header}.rs`: one synthetic content header; `.mtf` currently has `file_dependency_clauses: Vec::new()` and retains the entire body token range for Stage 3.
- `src/compiler_frontend/utilities/token_scan.rs`: `collect_scanned_symbol_references` presently sees plain `SYMBOL` tokens. Prevent the selected `SYMBOL` after a qualified `PATH` from being mistaken for an unrelated bare initializer reference.
- `src/build_system/create_project_modules/{module_inventory,module_preparation,source_preparation,discovery_traversal,module_namespace}.rs`: one preparation/read per semantic source, the Stage 0 directory and synthetic BFS paths, provider resolution, module graph edges and exact source ownership.
- `src/build_system/create_project_modules/{file_reference_resolution,compilation/canonical}.rs`: Stage 0 physical file-value resolution, compiled source-package provider lookup and binding handoff. Qualified source imports must not be sent to the file-value resolver.
- `src/compiler_frontend/headers/binding_environment/{builder,source_dependencies,provider_dependencies}.rs`: existing direct-selected source/provider binding and scope checks. Add per-occurrence references **without inserting them into `FileVisibility` as unqualified imports**.
- `src/compiler_frontend/public_interface/dependency_bindings.rs`: `DependencyShellId` to completed provider interface mapping, provider origin deduplication and builder-selected implicit template providers. Reuse the completed interface identity path; discard the general implicit-template category only after the fixed `@html` prelude is preserved.

**Ordering and template constants**

- `src/compiler_frontend/headers/{constant_dependencies,dependency_canonicalization,ordering_hints,types,module_symbols}.rs`: declaration initializer reference facts, `LocalDeclarationOrderingHint`, canonicalization and the existing content-file hint scan.
- `src/compiler_frontend/module_dependencies.rs`: the one Stage 3 dependency graph, DFS cycle diagnostics and `ContentSourceTargets` mapping resolved `@intro.mtf` occurrences to synthetic `content` declarations.
- `src/compiler_frontend/headers/parse_file_headers.rs`: `bind_module_headers` binds providers, canonicalizes hints and adds constant dependencies before Stage 3 sorting.
- `src/compiler_frontend/module_compilation/service.rs`: canonical semantic sequence. Do not introduce another constant evaluation pass.
- `src/builder_surface/definition.rs`, `src/builder/definition.rs` and `src/projects/html_project/html_project_builder.rs`: generic template-constant provider registration and official `@html` registration.
- `src/compiler_frontend/headers/tests/moth_template_prepare_tests.rs`: substantial existing `.mtf` implicit-scope tests requiring migration, not another parallel test harness.

## 4. Implementation strategy and invariants

The minimum complete implementation has **one retained qualified-reference fact per authored occurrence**, keyed by its `(SourceId, PathSyntaxId)` and including the selected symbol, exact spans and the existing provider/dependency identity. It must survive string/path/source remapping using the existing compact IDs.

For a cross-module or source-package dependency, preserve the existing `DependencyShellId` handoff. Allocate a unique deterministic shell ordinal per source file across **both** ordinary clauses and inline qualified occurrences. A reused ordinal must never bind two providers. The final representation may reuse direct-selection retained facts, but must not clone selection lists for one symbol or reparse path strings after tokenization.

Keep two distinct concepts:

- A **provider dependency** required to discover and compile the source/interface and add any cross-module edge.
- A **qualified expression reference** to exactly one selected value from that provider, resolved at one source occurrence and **not** installed as an unqualified name.

For the same module, provider discovery reaches the target ordinary `.moth` or `.mtf` file by the existing indexed namespace. For an external provider or cross-module facade, Stage 0 retains the existing provider-shell-to-interface link. The exact selected declaration is validated and bound during header/provider binding when its semantic interface is available. Stage 3 receives a canonical path or provider-interface origin, not a guessed identifier spelling.

A per-occurrence table belonging to the already-prepared source/module is justified. A separate filesystem index, per-template package registry, second topological sorter, extra module compilation job, direct AST filesystem access or persistent duplicate cache is not.

**Critical boundary rules:**

- Identify qualified occurrences before `classify_prepared_file_references` so their `PathSyntaxId`s are consumed as dependency syntax, not filed as extensionless/resource file values.
- Stage 0 must encounter provider dependencies from qualified occurrences even in never-executed runtime branches, unused constant initializers, template heads and frozen/persistent generic source bodies. Preserve provider validation/watch behaviour and existing diagnostic budgets. Only the normal constant-required header expressions create Stage 3 **declaration-order** edges; ordinary function/start bodies require provider availability but do not become declaration graph nodes.
- Avoid a second tokenization. Use the retained source-token owner and bounded token facts during the existing per-file preparation and header scan. If necessary, extend the same generic token scanner to recognize `PATH` + same-line `SYMBOL`. The scanner decides syntax shape, **not** whether the provider/export/value is semantically valid. Filter out paths already owned by ordinary top-level dependency clauses and other non-expression contexts. Keep an occurrence index rather than rereading the same tokens at every later stage.
- Preserve a single source of truth for the selected token's ownership so `collect_scanned_symbol_references`, typed expression parsing and ordering-hint generation cannot independently treat it as a bare name.
- Bind the selected target through existing direct-selection and provider-interface validation, but write the result into the qualified occurrence map. It must not enter the file's flat `visible_source_names` or `VisibleNameRegistry` under the selected name. Qualified occurrences may reuse provider cache and semantic materialization without inventing aliases.
- For a const initializer or synthetic `.mtf` content header, insert the resolved local declaration path into its existing `local_ordering_hints`; imported/provider targets use the existing provider edge classification and no fabricated local header. Content-file references target the same generated `content` header as ordinary `@intro.mtf` file values.
- Reuse Stage 3's existing DFS and cycle diagnostic owner. There must be no special relaxation for `content`, no template-only ordering pass and no fallback on cycles. Preserve authored spans, deterministic edge order, deduplication by resolved target and the same-file forward-reference policy.
- AST expression parsing consumes the prepared qualified occurrence and lowers the selected declaration as an ordinary value/call/constructor reference. It must not do name-based root lookup or open files. Template-head parsing uses the same resolved reference helper and applies its existing constness/renderability checks.
- Keep root-visibility filtering based on the **requesting source role**, not just the module's structural role. A `.mtf` is a role-neutral helper and cannot borrow privileged root-local declarations, even when all three roots coexist.

## 5. Stable implementation phases

Each phase may be a small commit, but don't expose an incomplete public grammar or temporary `.mtf` fallback as a completed language feature. Preserve the ongoing fixed-root agent's work. Prefer source-backed gates with focused tests to broad refactors.

### Phase 0: Rebaseline the active implementation branch

1. Inspect the latest head of PR #31 and its working-tree diff. At the reviewed commit, the branch already uses `ModuleRootFiles<T>` with normal/support/entry roles, `HeaderParseOptions::root_file_ids` and a temporary `LegacyCombinedRoot` bridge. Confirm which fixed-root phases the agent has completed. Do not overwrite or back out those changes.
2. Read section 3's owners on the actual working revision. Locate direct-selection provider resolution, `PreparedFileReferenceTable` classification, `.mtf` header creation, binding and Stage 3 sorting, plus canonical and synthetic/check-only discovery.
3. Capture focused baseline tests for paths, dependency clauses, `.mtf` folding, same-module constants, source-package imports, cycle diagnostics and JS/Wasm HTML output. Record exact commands and pass/fail evidence.
4. Add a hard prerequisite reference from the existing fixed-root plan to this plan, including Phase 2 binding and Phase 4 migration gates. Do not fork a second root-role architecture or restore the legacy root prefix rules.

**Gate:** one reviewed branch state, one integration point and a baseline. No new public syntax yet.

### Phase 1: Recognize qualified occurrences during one preparation pass

1. Add a small bounded scanner for a path token immediately followed by a same-line symbol **in expression context**. The scanner may use existing retained token facts plus source spans and must never infer validity by looking up a filename. Ordinary top-level dependency clauses continue through the existing header classifier and must not be treated as expression-qualified references.
2. Record `(source, PathSyntaxId, selected StringId, path span, selected-symbol span)` and a unique `DependencyShellId` for any provider handoff. Unify the per-file shell ordinal allocator with ordinary top-level dependency clauses (or allocate inline shells after the ordinary clause count in canonical source order). This must work for `.mtf`, which has no authored dependency clauses. Use one semantic row per authored qualified reference and remap it together with the prepared source output. Do not allocate a general import-expression AST during Stage 0.
3. Integrate this scan into ordinary `.moth` header preparation and `moth_template_prepare.rs`, including nested template heads. For `.mtf`, preserve the existing `SyntheticContentPayload::MothTemplate` and single generated `content` declaration.
4. Mark each qualified path occurrence as consumed before the prepared file-reference table is built. Explicit file-value paths without selected symbols remain untouched. Ensure a qualified follower symbol does not also become a bare `InitializerReference` that might create a false local name dependency or collision.
5. Diagnose incomplete or malformed qualified selections through the ordinary typed diagnostic lane. Keep a missing selected symbol distinguishable from a valid bare **explicit-extension** file value. Preserve quoted path components and no-newline selection termination.

**Gate:** lexer and header tests prove correct path/symbol retention, remapping and non-duplication. Existing file-value, template-body tokenization and ordinary dependency syntax remain unchanged.

### Phase 2: Reuse Stage 0 dependency resolution for qualified providers

1. Extend the canonical `module_inventory.rs` loop to process each retained qualified provider dependency alongside the existing `file_dependency_clauses`. Reuse `resolve_structural_provider_reference`, indexed `DirectoryDependencyResolution` / `ModuleNamespaceSet`, provider caches, source-package boundary selection and typed failure lanes. Do not add a second module dependency traversal.
2. Queue same-module `.moth` and `.mtf` sources from qualified references in the existing BFS. Insert cross-module `ModuleId` edges in the existing `ProjectModuleGraph` and source-package interface requirements in the existing source-package dependency table. Apply the same role-sensitive support-visibility check already needed by fixed-root work.
3. For a registered explicit-extension provider, reuse its current provider-selection mechanism. A resource path such as `@images/logo.svg symbol` must not cause a resource read or become a declaration provider. Keep failed-provider diagnostics and unavailable-provider outcomes consistent with ordinary direct selections.
4. Extend the synthetic single-file discovery (`discovery_traversal.rs`) and check-only discovery paths. A synthetic application with a qualified provider dependency must reach exactly the same provider/source closure, error lane and stable identity as its explicit-import equivalent.
5. Validate that two qualified occurrences selecting the same provider don't trigger repeated tokenization, provider invocations, filesystem passes, graph edges or extra module jobs.

**Gate:** qualified references populate the existing discovery, provider and graph structures with correct ownership and no duplicate work. No AST-level provider resolution or filesystem probing exists.

### Phase 3: Per-occurrence semantic binding and shared declaration ordering

1. Use the existing direct-selection resolution path in `binding_environment` to check the requested member's visibility, declaration kind, public origin, error/effect/type summaries and source/package boundary. Factor only a narrow shared target-resolution operation if the current `resolve_and_register_selection_bindings` cannot be reused without installing a local name.
2. Store the selected resolved declaration in a per-occurrence lookup addressed by `(SourceId, PathSyntaxId)`. The lookup must remain valid across source/path ID remapping and must be borrowed by AST and Stage 3, not published as a user-visible namespace or persistent interface. Preserve stable provider identities and module-local declaration paths.
3. Integrate the qualified-selection facts with `headers/constant_dependencies.rs`, `ordering_hints.rs` and `dependency_canonicalization.rs`. The existing synthetic `.mtf` `content` header must receive local ordering edges for qualified constants in its own body. No separate content sorter.
4. Avoid the scanner's `InitializerReference` double classification: the symbol after the path is the selected name, not a second bare initializer reference. Reconcile any other token scanners or declaration default/capacity paths that need the same skip.
5. Preserve same-file forward-reference and self-reference diagnostics. Verify real same-module cycles such as `settings/title -> intro/content -> settings/title` are diagnosed by `module_dependencies.rs`, and ordinary content-value `@intro.mtf` and direct selection `@intro content` converge to one header origin.
6. Verify cross-module cycles fail through the existing structural module graph before any consumer prematurely attempts to bind an uncompiled provider. Keep diagnostic locations at the authored qualified occurrence and the opposing edge where available.

**Gate:** both dependency directions use existing ordering and cycle mechanisms. There is no implicit constant fallback and no local-name leakage from qualified references.

### Phase 4: Parse and lower qualified expression primaries once

1. Extend the ordinary `TokenTag::PATH` operand in `ast/expressions/parse_expression_dispatch.rs`: select the prepared qualified binding for a recognized path/symbol pair or retain `resolve_file_value` for a bare file-value path. Consume exactly the path and selected symbol tokens, then use existing expression suffix, callable resolution and typing. Avoid rewriting the token stream or reparsing as an authored top-level dependency clause.
2. Extend the `TokenTag::PATH` case in `template_head_parser/head_parser.rs` to use the same qualified primary path. Existing file-value head expressions continue through `push_template_head_path_expression`. Apply existing template value materialization, `$md`/`$html` formatters, slot handling, scalar renderability and const-required checks.
3. A qualified call in ordinary source obeys normal runtime semantics. In `.mtf`, a runtime call or otherwise nonconstant selection fails under existing const-required diagnostics. Defer accepting a const function call until the independently accepted const-functions feature is implemented.
4. Enforce the statement-position exclusion. In particular, an unassigned top-level `@math square(12)` remains an invalid dependency clause, not a new implicit entry action. Document the conventional explicit import plus call alternative.
5. Cover data values, calls, chained fields, conditionals, nested template heads and expected-type propagation. Ensure references inside frozen generic bodies preserve the qualified target across their existing freeze/materialization boundary rather than re-resolving a string path later.

**Gate:** ordinary and template expressions produce the same semantic declaration/value under qualified and ordinary explicit-direct-selection forms, without changing file-value syntax or adding a new HIR operation.

### Phase 5: Replace `.mtf` implicit constants with explicit qualified sources

1. Remove `collect_same_directory_public_export_constants`, `same_directory_root_file` and any now-dead root-uniqueness logic from `headers/binding_environment/builder.rs`. `.mtf` must no longer see same-directory facade constants simply because the facade exports them, even when it is the only root. It may still access a helper's constant through `[@helper value]` or an exported cross-module constant through an explicit qualified reference.
2. Remove configurable builder package template injections (`SourcePackage::available_in_templates`, `BuilderSurface::register_implicit_template_scope_source_package`, their generic `implicit_template_scope_source_packages` registry and generic provider-list plumbing) once call sites and public builder API/docs are migrated. Do not remove provider interface deduplication or cache mechanisms still used by authored imports.
3. Preserve **only** the fixed `@html` flat constant prelude for `.mtf`. Route this through the existing official compiled source-package interface with its proper origin, const classification and diagnostics. Do not duplicate its values. Resolve the current Builder-origin vs desired compiler-standard prelude policy as described in section 2.5 and record the code contract accurately.
4. Preserve `.mtf` generated self-`content` exclusion from its own *unqualified* scope if still needed by the normal source-file implementation. A qualified explicit self-reference (`@intro content`) must produce a cycle diagnostic rather than being silently filtered. Delete self-exclusion utilities only if existing normal file scope ownership makes them redundant.
5. Replace implicit local scope and collision tests in `headers/tests/moth_template_prepare_tests.rs` with explicit qualified access, missing/illegal root access and duplicate-name-without-collision tests. Keep separate HTML prelude tests and genuine prelude collision coverage.
6. Update real documentation `.mtf` content and first-party fixtures. Where a page previously relied on `@mod` solely to leak a constant into a template, remove the facade and select a normal helper explicitly when that facade has no actual public API role. Keep other public facades when they are genuinely needed.

**Gate:** `.mtf` has no general implicit application/provider constant scope, still has standard `@html` helper ergonomics and folds through the same pipeline as ordinary compile-time template expressions.

### Phase 6: Integrate with fixed-root public cutover and documentation

1. Continue the fixed-root Phase 2-4 work on the same branch. Use the new `@mod`, `@package`, `@entry` source-role inventory as soon as it is available and delete `LegacyCombinedRoot` only at the planned common cutover. Do not preserve the old combined-root fallback merely to keep `.mtf` working.
2. Reconcile `docs/roadmap/plans/fixed-root-roles-implementation-plan.md` so it explicitly depends on this plan. Replace its outdated implied `.mtf` facade-export rule with explicit qualified-reference access and the fixed HTML prelude. Ensure both acceptance gates require this work.
3. Update `docs/compiler-design-overview.md` and `docs/build-system-design.md` for a qualified expression's early provider-dependency fact, its non-binding nature, Stage 0 graph edges, Stage 3 local ordering/cycles and ordinary `.mtf` synthetic content path. Preserve one semantic transaction and immutable interface handoffs.
4. Update the public package/dependency/file-path documentation and `.mtf` template pages. Show the distinct forms `@module symbol` (qualified expression), `@module symbol, other` (ordinary top-level dependency clause in statement position, not expression) and `@file.ext` (file value). Explain same-line termination and why `@mod`/`@entry` private declarations remain inaccessible from `.mtf`.
5. Update the `.mtf` Basic docs, cheatsheet, language overview, template limits, package-builder guide and progress matrix. Remove examples suggesting implicit module-root exported constants, generic `available_in_templates` and paired-root exceptions. Clarify standard HTML prelude availability and the actual `PackageOrigin` ownership if unchanged.
6. Migrate docs page sources, actual examples, benchmarks and test fixtures as needed and check generated documentation links, routes and resource content. Do not infer import availability from rendered HTML or alter resource URL provenance.

**Gate:** final root and expression-reference designs agree throughout code and documentation. The public cutover has one consistent source surface and no compatibility aliases or temporary template scope.

### Phase 7: Validation, performance and code-bloat audit

1. Run targeted tokenizer/path, header, file reference, namespace, module ordering, `.mtf`, source-package, frozen-generic, synthetic/check-only, HTML JS/Wasm and CLI/fixture tests. Use the repository's existing test owners and commands.
2. Run `just validate`, `just validate-full` and the docs release build under the style/validation guide, plus applicable feature/platform lanes. Record actual output and any unsupported gates. Do not claim a pass from code inspection alone.
3. Audit memory and work: one tokenization per source, one candidate/source identity, one provider interface per resolved origin, no repeated source file read from qualified occurrences, one Stage 3 dependency graph and no template-only evaluation. Check deterministic ordering and diagnostic spans under parallel preparation and source/path remapping.
4. Delete old same-directory export injector, generic template-provider scaffolding, temporary migration bridges, obsolete tests and comments. Preserve only the fixed HTML prelude and the normal explicit source/provider binding machinery. Do not generalize the solution into a macro, arbitrary in-expression import expression, new root role or alternate frontend compiler.
5. Final report: phase gates with evidence, changed subsystem owners, removed code, cycle/visibility tests, documentation updates, any standard HTML package ownership discrepancy still requiring a separate architectural decision and remaining independent const-functions work.

**Gate:** implementation and documentation satisfy every contract without parallel scope, provider, graph or compiler mechanisms.

## 6. Required focused regression coverage

Prefer extending the existing test owners. Each case should catch one distinct credible failure. At a minimum:

**Lexer, grammar and ordinary expressions**

- `@math square(12)` produces PATH, SYMBOL and normal call suffix; quoted path components work, same-line whitespace works, newline between path and symbol is rejected.
- `@images/logo.svg`, `@docs/intro.mtf`, `@/` and other valid existing file values keep their types, provenance and resource/content behaviour.
- Bare extensionless `@settings` is still diagnosed in expression position; `@images/logo.svg name`, `@settings missing` and an invalid `.moth` path are diagnosed by the correct owner.
- `@math square` selects one symbol and creates no unqualified `square` binding. Qualified `@a value` and `@b value` coexist even when both names are equal and a third local `value` exists.
- An unassigned top-level `@math square(12)` remains invalid; an ordinary top-level dependency clause followed by `square(12)` retains its existing behaviour.
- Fields, nested calls, conditionals, generic invocation inference, error/effect contracts and wrong-kind/type diagnostics match direct-selection binding.

**Templates, helper visibility and cycles**

- A `.mtf` document uses private same-module `[@settings site_name]` without an `@mod` file and produces folded content. The same value is selectable inside a normal `.moth` template.
- A `.mtf` cannot see `site_name` merely because its `@mod` exported it, and cannot qualify a root-local `@mod`, `@package` or `@entry` declaration. An explicitly selected same-module helper works next to all three roots.
- Two helpers exporting `value` through different allowed interfaces do not collide when referenced as `[@a value]` and `[@b value]`. A bad provider/source path or private target receives a typed diagnostic.
- A `.mtf` remains compile-time-only: runtime-valued qualified references and non-const functions fail. Future const functions are not marked supported by these tests.
- The direct cycle `settings/title = @intro.mtf` with `intro.mtf -> @settings title` fails through Stage 3. Also test a longer `.moth -> .mtf -> .moth` cycle, self-qualified `@intro content` and a valid non-cyclic chain.
- A cross-module cycle involving a facade-exported content constant and a qualified reference fails through the existing Stage 0 module graph. An independently ordered cross-module chain succeeds.
- `@intro.mtf` as a content file value and `@intro content` as a same-module qualified selection identify the same synthetic constant and do not duplicate folding.
- Existing flat `[p: ...]` and `[codeblock, $code("moth"): ...]` still use official HTML helpers. Arbitrary builder-registered template constants are no longer injected, but `[@provider value]` works.

**Compilation boundary and regression**

- Same-module, child-normal, eligible scoped support, entry-only, support-only, registered source-package and direct external-provider selections follow existing eligibility rules.
- A qualified reference in a never-reached branch or a frozen generic body still creates the necessary provider-discovery fact, without a false Stage 3 const edge for an ordinary function body.
- Test canonical, synthetic single-file and check-only compilation, plus source package boundaries. Repeat a deterministic build and compare diagnostics/identities. Provider cache and source preparation remain one-per-source, not one-per-inline-use.
- Both HTML JS and supported Wasm output preserve compile-time content, template slot and resource-link semantics. Changing a referenced private helper or exported constant updates the right dependent content and fingerprints.
- Test invalid syntax and cyclic input under diagnostic budgets and parallel preparation. User-authored mistakes must never reach `compiler_bug!`, panic, mismatched `PathSyntaxId` or an empty-interface fallback.

## 7. Completion contract and exclusions

The work is complete when:

- One-symbol `@path symbol` qualified references are accepted in ordinary and template expression positions and use existing provider/visibility rules without adding a local binding.
- `.mtf` content is one normal synthetic constant that participates in the same declaration graph and can form and diagnose dependency cycles in both directions.
- File-value paths, direct top-level dependency declarations, fixed root isolation, source package boundaries, syntax remapping, stable origin identities and failure lanes retain their existing contracts.
- Implicit same-directory facade constant visibility and arbitrary builder-configured template constant injection are removed. The official HTML helper prelude remains available with its provenance intact.
- Fixed-root plan gates, permanent docs, targeted/full tests and code-bloat audit record the new contract and demonstrate no duplicate work.

Out of scope: top-level qualified calls, multiple selected names per inline occurrence, aliases, automatic module-wide imports, explicit dependency declarations in `.mtf`, new `.mtf` helper filenames or directives, arbitrary template-local bindings, type-position qualified references, a new dependency graph, a second const evaluator and implementing the future const-functions feature.
