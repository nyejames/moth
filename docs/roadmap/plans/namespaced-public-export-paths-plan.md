# Namespaced Public Export Paths Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task by task. Track work with `- [ ]` checkboxes. Every phase has a mandatory correctness, style and validation gate.

**Status:** Queued. First in the normal sequenced roadmap, ahead of Core Math fast counterparts.

**Goal:** Allow `$export` to publish explicitly named declarations and direct-selection reexports under an optional, relative public namespace path, while preserving source privacy, existing package APIs and explicit dependency resolution.

**Architecture:** Extend the existing source-directive attachment and retained header facts with one optional export route. Thread that route through the existing public-export maps, stable semantic interfaces and Stage 0 namespace resolution. A nested route remains a view of its owning facade, not a new module, package, registration or compilation boundary. Preserve the original declaration identity and the existing compile, link and cache owners.

**Tech Stack:** Rust, Moth frontend and build system, source-backed package graphs, public semantic interfaces, the manifest-driven Moth integration runner and canonical `.mtf` documentation.

**Spec:** [Accepted language contract](#accepted-language-contract) in this document, agreed with Nye on 18 November 2026. The permanent package, directive, compiler and build-system references listed below remain authoritative for unchanged semantics.

**Repository destination:** `docs/roadmap/plans/namespaced-public-export-paths-plan.md`.

## Global constraints

- Read `AGENTS.md`, `index.md`, `docs/developer-docs/style-guide/style-guide.mtf`, its testing and validation guides and the routed canonical language references before writing code. Read the relevant compiler and build design sections and producer/consumer implementations in full. Re-check owners against the current branch before editing.
- The existing `$export` syntax, valid target categories, attachment rules, visibility and privacy checks remain the baseline. This plan adds an optional path operand, not another export construct.
- Extend every facade where `$export` is valid today. On the inspected baseline these are `@mod.moth` and `@package.moth`. Do not make `$export` legal in `@entry.moth`, helpers, `@config.moth`, templates or other currently disallowed contexts.
- Preserve all existing Core, Builder and project package source files and published APIs. Do not reorganise `@core/html` or other packages in this plan. New package examples belong in test fixtures and documentation.
- Keep source-backed module namespace records shallow. A consumer explicitly imports the nested provider path, never discovers children by member access on the parent namespace. No new global namespace registration, file-system publication, import fallback or path precedence.
- Keep source/package boundaries, root-local isolation, support visibility, project config isolation, reserved `core`/`builder` roots and no-bypass rules. `@foo/private` must not become accessible simply because a public `@foo/forms` route exists.
- Preserve stable source declaration origins and all existing public-surface validation, including types, receiver methods, generics, trait evidence, constants, effects, error contracts, access/lifetime summaries and project provenance. A route changes the public binding and its fingerprint, not the declaration's origin.
- Use one compiler-owned public-export route fact from preparation through publication. Reuse `PathId` only within its owning boundary and stable owned paths in cross-module interfaces. Do not retain multiple independent route indexes or copy complete declaration interfaces for each path.
- TDD and small coherent commits. Avoid compatibility shims, duplicate parsers, unbounded cloning, speculative registries and architecture drift. Use typed `DiagnosticDraft` with exact authored/related spans for invalid source.
- Documentation uses concise British English, basic ASCII punctuation and the surrounding reference style. Do not modify generated `docs/_release/**` files directly.

## Accepted language contract

### Syntax and location

Inside a facade published at `@core/html`:

```moth
$export
@internal/table_helpers table

$export @tables
@internal/table_helpers cells, table_heading

$export @tables/layout
TableLayout ::
    columns I32
;
```

The resulting explicit dependencies are:

```moth
@core/html table
@core/html/tables cells, table_heading
@core/html/tables/layout TableLayout
```

- `$export` with no operand retains its existing root destination.
- `$export @tables` publishes the following eligible target in `@<facade>/tables`. `$export @tables/layout` publishes at `@<facade>/tables/layout`. Any valid nesting depth is allowed.
- The operand is one **public suffix path**, relative to the declaring facade's public identity. Its `@` spelling is not a dependency to resolve and never grants access to a source file or package. Reuse the supported source provider-path spelling and component validation, including canonical segment and case rules, without adding a second path language. Reject empty paths, `.`, `..`, disallowed extensions, malformed/extra operands and attempts to escape the owning facade, with a span on the offending operand. No path evaluation, namespace expressions or inferred physical roots.
- The directive continues to attach to exactly one eligible declaration or a strict direct-selection dependency clause. In a selection clause the **same route** applies to every explicitly selected name. Existing leaf `as` aliases determine the final published names. The route does not rename a selected leaf.
- Existing unmarked imports stay private. Bare namespace clauses, namespace aliases, whole-module and wildcard reexports remain invalid with `$export`. Receiver methods still travel with their exported receiver type and cannot be separately selected or exported.
- Each annotated declaration or direct-selection clause has one export destination. A repeated `$export` in one directive run remains a duplicate-directive diagnostic, including when its operands differ. Do not add multi-route operands, export lists or an origin-wide ban on the same underlying symbol being independently reexported through separately authored clauses where today's alias rules permit it.
- Distinct declarations may use the same public route. A public name is unique **within its route**, not necessarily across different routes. Duplicate `(facade, route, public name)` claims report both authored sites. Different routes under the same facade may contain the same leaf name.

### Resolution, collisions and visibility

- An import of `@core/html/tables cells` resolves the `tables` public surface of **the same** `@core/html` package. There is no separately registered `@core/html/tables` package, graph, module, artefact or entry.
- A bare explicit dependency on `@core/html/tables` may bind a shallow namespace record of that route's directly exported members, subject to the existing namespace import rules. Importing `@core/html` does **not** provide `html.tables.cells`. A prefix that has only deeper routes does not acquire synthetic members or a magically importable whole subtree.
- Different route declarations merge by exact public route and expose only their explicitly exported symbols. A public route may have both its own members and deeper child routes. These are independently named providers, not nested namespace-record members.
- Reject ambiguous visible namespace identities, including conflicts between a virtual route and an already public child module/support facade/provider path, same-path registered package or conflicting alias, with both sources where available. Keep existing case-only collision policy and no precedence or fallback. Registration's existing reserved-root and ancestor/descendant package rules stay intact.
- A private physical directory or helper may share a spelling with an exported route. It remains private to its original compilation boundary. Public resolution must distinguish consumer visibility from private internal file lookup and never fall through to private sources when a nested export member is missing.
- Reexporting from an existing nested public route requires explicit selections. Chained reexports preserve the original source or binding-backed symbol identity and do not grant transitive access to the source package's private subpaths.
- The existing `.mtf` prelude policy stays **explicit** and continues selecting its current root-level symbols. A new nested route is not automatically flattened into the prelude or installed as a namespace. Adding one deliberately later would require a separate compiler policy change, outside this plan.
- Export route changes participate in the existing public-interface fingerprint and consumer invalidation. Pure route reshuffling must not change the underlying declaration origin, backend ABI, source package identity, physical resource location, runtime dependencies or entry activation.

### Deliberately excluded

- Whole-namespace reexports, wildcard/public glob exports or automatic subtree forwarding.
- Multiple destinations on one `$export` target, syntax for public alias lists and a general redesign of `$export` boilerplate.
- Automatic access such as `html.tables.cells`, recursive source namespace records or synthetic empty namespaces for parent-only route prefixes.
- New package registrations, nested package manifests, package acquisition/versioning, transitive package admission or new project-relative import syntax.
- Any migration of existing first-party package exports or expansion of the `.mtf` prelude.

## Architectural ownership and likely files

Confirm these names at activation. Reuse their current owners and adapt names if the repository has evolved.

- Source directive operand grammar and retention: `src/compiler_frontend/style_directives/specs.rs`, `builtins.rs`, `source.rs`, `src/compiler_frontend/headers/file_parser.rs`, `types.rs` and `file_dependency_clauses.rs`. The current `SourceModifierOperand::None` rejects every `$export` operand, while `HeaderExportMode` carries only public/private. Preserve the single header syntax preparation path.
- Public export collection: `src/compiler_frontend/headers/public_exports.rs`, `module_symbols.rs`, `binding_environment/public_export_resolution.rs` and `binding_environment/namespace_bindings.rs`. Currently `PublicExportEntry` identifies an export by a flat name plus target. Public route must become part of the key while preserving the existing target variants and authored spans.
- Public interface projection and stable identity: `src/compiler_frontend/semantic_identity.rs`, `src/compiler_frontend/public_interface/model.rs`, `export_projection.rs`, their consumers and fingerprint/equality owners. `ExportBinding` and `PublicBindingExport` currently carry flat public names. Represent the route separately from the selected leaf, with owned canonical spelling across boundaries. Keep all declaration/type/origin projection code shared.
- Stage 0 resolver and package facade routing: `src/build_system/create_project_modules/module_namespace.rs` and adjacent source-package discovery/project structure modules, plus `src/builder_surface/source_package_registry.rs`. Stage 0 currently treats descendant source-package paths as private. Extend only declared public route lookups, retaining exact registered package ownership and the existing collision machinery.
- Diagnostics: owning typed import/directive/public-export families under `src/compiler_frontend/compiler_messages/diagnostics/families/` and appropriate build-system structure diagnostics. Extend the existing reason family where possible. Never format ad hoc user errors or panic on invalid input.
- Tests: `src/compiler_frontend/headers/tests/source_directives_tests.rs`, `parse_file_headers_export_tests.rs`, existing header/public-interface and namespace tests, `src/build_system/tests/build_dependency_tests.rs`, relevant `src/compiler_tests/` scenarios and `tests/cases/manifest.toml` with focused `tests/cases/` fixtures. Find the current test owner before adding a new file.
- Permanent docs for the implementation phase: `docs/docs/directives/directives.mtf`, `docs/docs/project-structure/public-api.mtf`, `docs/docs/packages/public-reexports.mtf`, `dependency-clauses.mtf`, `dependency-paths.mtf`, `module-visibility.mtf`, `docs/compiler-design-overview.md`, `docs/build-system-design.md` and the cheatsheet. Progress rows live in `docs/docs/progress/@entry.moth` and `docs/docs/progress/packages-and-builders/@entry.moth`.

## Review focus

These five likely failures require dedicated tests in their owning phases, not just manual review.

1. **Private path bypass:** A public route that happens to match a private source directory must not reveal unexported members, even when consumers import a deeper suffix or switch from selected to bare namespace imports.
2. **Cross-boundary routing:** Registered `@core/*` or `@builder/*` source package prefixes, project-local support facades and external project packages must route to their owning facade and not become new registrations or reach into the consumer's module index.
3. **Public identity drift:** Changing only an exported path must change consumer binding/interface fingerprints while preserving canonical declaration origins, generic/receiver metadata and link identities.
4. **Import ambiguity:** Case-only route collisions, collisions with a visible child module or reserved registered package and duplicate public names within one route must be rejected deterministically without fallback.
5. **Unexpected prelude and reachability:** New nested exports must not enter `.mtf` automatically, trigger dormant entries or introduce unexpected foreign runtime assets when unused.

---

## Phase 0: Establish the baseline and exact owners

**Deliverable:** A checked implementation map and baseline tests, without production behaviour changes.

### Task 0.1: Inventory the current contract

- [ ] Read the references in Global constraints. Inspect how the directive token parser retains source annotations, how header dependency clauses are built and how `PublicExportEntry` and `ExportBinding` become module interfaces.
- [ ] Follow one existing `@mod.moth` reexport and one registered `@package.moth` source package from Stage 0 resolution through the frontend, runtime linking and an integration fixture. Record the current mutation points and eliminate proposed duplicate owners.
- [ ] Inspect existing diagnostic reason families, collision checks, path validation helpers, source/package boundary classification, first-party provider loading and `.mtf` prelude registration. Identify the precise existing helpers that the feature will reuse.
- [ ] Inspect existing header/namespace/package integration coverage and manifest conventions. Choose the narrowest existing owners for each positive and negative test. Avoid new test harness abstractions.
- [ ] Confirm the first-party source-package-root contract on the branch, including required `@package.moth`. Do not let stale older docs or planned alternative syntax override live canonical authority.

### Task 0.2: Pin baseline and tests that must stay unchanged

- [ ] Run existing focused `$export` directive, reexport, namespace, source-package collision and selected integration tests. Record exact commands, counts and results in working notes, not in this queued plan.
- [ ] Define a small set of failing source examples for the new route syntax, one module-facade consumer and one source-backed package consumer. Keep cases in working notes until their implementing phase, then add them as executable tests. Never commit skipped or permanently failing fixtures.
- [ ] Confirm existing first-party package paths and public export lists remain unchanged throughout the plan.

**Phase 0 mandatory gate**

- [ ] Audit the owner map against `AGENTS.md`, both design authorities and the current style guide. Reject a plan to introduce a second parser, registry or public-interface model.
- [ ] Re-run the selected existing tests, review the unchanged diff and record evidence. Resolve discovered baseline bugs outside this feature through `docs/bug-reports.md`, rather than quietly expanding scope.
- [ ] Commit only if useful fixture or documentation scaffolding was created. Stop on unresolved cross-stage ownership conflicts.

## Phase 1: Parse and retain one optional export path

**Deliverable:** `$export @route` has precise, retained header syntax and diagnostics without weakening existing `$export` attachment or target validation.

### Task 1.1: Define the single typed route operand

- [ ] Add focused failing tests in `src/compiler_frontend/headers/tests/source_directives_tests.rs` for no operand, one route, multi-component route, stacked `$doc`/`$export` on one target and the same directive applied to a multi-selection dependency clause.
- [ ] Add focused failing diagnostics for trailing garbage, multiple operands, missing/invalid segment, `.`, `..`, an unsupported file extension, blank-line/dangling attachment, ineligible target, duplicate `$export` with distinct destinations and unsupported bare namespace reexport. Assert exact existing or newly added typed reasons and primary/secondary spans.
- [ ] Extend the `SourceModifierOperand` contract with one dedicated optional **path** form for `$export`, distinct from the compile-time `String` expression operands used by `$doc` and `$todo`. Parse using the canonical path syntax/token owner and validate component rules at header preparation. `SourceDirectiveAnnotation` must retain one typed path and its source span without rescanning later.
- [ ] Thread the parsed route onto eligible authored `Header` and `RetainedDependencyClause` facts through the existing single attachment code. Keep `HeaderExportMode` only for current public/private semantics unless combining that fact with the route actually reduces duplication. Ensure any path IDs are remapped through normal merge/remap steps.
- [ ] Verify that `$export` without an operand remains byte-for-byte equivalent in its exported surface, diagnostics and attachment behaviour.

### Task 1.2: Preserve declaration and direct-selection eligibility

- [ ] Write negative tests for `$export @forms` on an entry/root runtime statement, helper file, method, trait conformance, template, config source and bare namespace clause, using the applicable existing rejection policy.
- [ ] Implement only the path operand plumbing needed by the following phase. Do not broaden eligible declaration categories, add path-target evaluation or create output route metadata in the build system.
- [ ] Run the relevant header and directive unit tests and `cargo check -p moth`.

**Phase 1 mandatory gate**

- [ ] Audit token ownership, annotation attachment, error provenance, path validation, remapping and future import availability. Remove duplicate syntax walkers and unused intermediates.
- [ ] Review all touched Rust against `style-guide.mtf`: comments, function length, types, allocations, diagnostics and stage boundaries.
- [ ] Run focused Rust tests, formatting checks and the affected integration controls. Record exact passes and diagnostic reasons. Commit the stable parser/header slice before moving on.

## Phase 2: Publish namespaced exports through existing facades

**Deliverable:** Normal and support facades expose namespaced declarations through their existing public interface and same-owner module routing.

### Task 2.1: Key public entries by route plus leaf name

- [ ] Write failing header and interface tests for root exports, `@forms Button`, `@forms/validation validate`, two distinct symbols sharing `@forms`, the same leaf in different routes and duplicate leaf names in the **same** route.
- [ ] Extend `PublicExportEntry` and `PublicExportCollector` to carry an interned, validated public-route suffix and use `(route, exported leaf)` for collision and lookup. Keep the origin/provider target representation unchanged. Retain authored route span for duplicate/collision diagnostics.
- [ ] Update both declaration and direct-selection export collection paths in `public_exports.rs` to derive the route from the prepared directive fact. Preserve existing two-pass collection and deterministic ordering, without re-opening the source or inventing a route registry.
- [ ] Ensure public-interface projection (`ExportBinding`, `PublicBindingExport`, diagnostic provenance and the completed interface) includes the stable owned route. Keep declaration origins independent of route names and alias spellings. Update equality and fingerprint sources only through the existing canonical public interface.
- [ ] Add source root unit tests that prove the *same* declaration origin under a changed public suffix and a changed public binding/fingerprint.

### Task 2.2: Resolve explicit module-local public routes

- [ ] Add an end-to-end project fixture with a normal module facade exporting into `@forms`, a support facade exporting into `@shared/validation` and consuming modules that use direct-selection clauses. Assert actual JS runtime output or another observable API result, not only compilation.
- [ ] Extend `public_export_resolution.rs`, namespace binding and relevant Stage 0 module namespace facts to resolve only exact public route plus selected leaf, or an exact bare public route with its shallow members. Publish the route from the already-prepared header/interface owner before dependent graph edges are scheduled, without source rescanning or speculative filesystem probes. Parent bare namespace imports must continue exposing only root-level members.
- [ ] Reject unexported names, attempts to walk into a private child module and any access that would cross an illegal support-facade scope. Keep the existing no-parent/no-sibling rules and root-local scope isolation.
- [ ] Reuse the existing collision diagnostics for a visible concrete child path versus a virtual public route, including case-only variants. Allow a matching **private** physical folder where no public identity conflicts. Keep a visible root route and its deeper child route legal when they contribute different bindings.
- [ ] Verify bare `@module/forms as forms` uses the existing shallow namespace-record mechanism without recursively materialising nested route children.

**Phase 2 mandatory gate**

- [ ] Audit exact-path resolution and every private-boundary fallback, with both consumer and exporting-root positions. Re-check collisions against the current `ModuleNamespaceSet` and compare public route matching with physical source lookup.
- [ ] Inspect new route storage for parallel maps, donor-owned IDs crossing interfaces, arbitrary strings beside existing interning and unnecessary copies.
- [ ] Run targeted parser, interface, namespace and module integration cases, formatting and Clippy for changed Rust. Commit only when normal and support facade imports work end to end.

## Phase 3: Registered source packages and chained reexports

**Deliverable:** Core/Builder source packages, independently compiled project packages and chained exports work with the same path contract and original symbol identities.

### Task 3.1: Extend registered source-package boundaries

- [ ] Add a registered source-package test fixture with a real `@package.moth` and an internal child facade plus an unrelated private source folder that shares the exported route name. Compile an outside consumer selecting `@core/example/forms Button` and a consumer importing the bare route as a shallow namespace. Assert runtime behaviour.
- [ ] Extend `module_namespace.rs` registered-prefix routing so an external consumer can select a declared public suffix beneath one exact owning source-package root. Keep the existing package prefix registry untouched, including ancestor/descendant registration rejection and reserved namespace origin checks.
- [ ] Extend package public-export lookup to match the public route **inside the exact registered prefix**. Internal package sources keep their original private file resolution, while external consumers never fall through to those private files on a miss.
- [ ] Add negative tests for an unexported leaf, forged deeper private implementation path, missing exact route, a case-only conflicting registration and two public namespace claims visible to one consumer. Assert structured diagnostics and responsible spans.
- [ ] Exercise a project-external `@package.moth` facade and one Builder-backed source package fixture where the existing test infrastructure permits it. They must use the same path selection and validation logic, not a special Core-only branch.

### Task 3.2: Preserve reexport origins and distinct targets

- [ ] Add tests for `$export @forms` on one clause selecting several leaves and `as` aliases, another clause selecting from an external binding-backed package, a source-backed provider and a two-hop source facade chain. Include generics, a nominal type with receiver methods and a constant where existing fixture APIs make these representative.
- [ ] Make the route part of **where** a selected binding is published, not **what** it selects. Preserve stable source origins, binding-backed canonical symbol IDs, type/evidence validation and all receiver method restrictions.
- [ ] Verify distinct selected names may merge into one route, duplicate public leaf names in the same route are rejected and an ordinary unmarked dependency has no public route, even if its provider is already public.
- [ ] Verify a selected dependency on another package's nested route is a normal provider edge. It must not grant transitive package discovery, a private source escape or a new package ownership identity.
- [ ] Cover missing/erroring provider interfaces, reexport cycles and rejected receiver-only selections through existing typed diagnostic and graph-cycle systems.

**Phase 3 mandatory gate**

- [ ] Audit provider-prefix matching, public/private boundary separation, external binding identities, multi-hop origin preservation, type projection and graph scheduling. Confirm package graph IDs remain boundary-local.
- [ ] Check whether any old flat-name assumptions or duplicate selectors remain in parser, binding or interface consumers. Remove obsolete special-case code instead of adding translation shims.
- [ ] Run targeted source package, external-binding, reexport, backend-linking and collision tests. Run formatting and Clippy on the affected code and record the results. Commit this complete cross-package slice.

## Phase 4: Operational correctness, reuse and regression coverage

**Deliverable:** Namespaced exports behave correctly through project compilation, incremental reuse, prelude policy and observable runtime linking.

### Task 4.1: Validate public interface reuse and runtime linkage

- [ ] Add a repeated-build test that first demonstrates successful semantic artefact reuse, then changes **only** an exported route. Assert that the public-interface fingerprint changes, an affected consumer is rebuilt and an unrelated consumer is reused when permitted.
- [ ] In the same test family, change only a private implementation body without changing the public route or interface. Assert that semantic consumers are reused according to the existing reuse contract. Do not infer cache reuse from a cold recompile or skipped writer output.
- [ ] Cover source declaration stable origins, exported nominal types, generic templates, receiver surface calls, folded constants and aliases in the emitted runtime or canonical interface data. Keep all donor-local IDs within their compilation boundary.
- [ ] Assert that an API-only facade with nested routes creates no executable entry or page artefact and that a dormant `@entry.moth` in a provider is not activated. Unused external or JavaScript-backed reexports must not force runtime glue or provider assets.
- [ ] Assert `.mtf` continues to receive only the compiler's explicitly chosen prelude bindings. A nested export must remain unavailable without an explicit dependency clause. Existing `@core/html` and `@core/css` root prelude behaviour must remain unchanged.

### Task 4.2: Consolidate positive and negative integration coverage

- [ ] Keep one primary success fixture and a small number of separate negative fixtures in `tests/cases/` under new IDs such as `namespaced_public_exports` and `namespaced_public_exports_invalid`. Register each through `tests/cases/manifest.toml` and preserve ownership rules. Extend an existing owner when that avoids redundant fixtures.
- [ ] Positive matrix: root and nested paths, repeated route merging, distinct duplicate leaf names across routes, direct declarations, multi-selection reexports, leaf aliasing, named module and support facade, registered source package, external project package, foreign binding, shallow bare namespace and multi-hop reexport. Assert runtime values or precise public interface state.
- [ ] Negative matrix: invalid path spelling, bad directive placement/operand, duplicate directive destination, duplicate `(route, leaf)`, case-only conflict, public provider route collision, forbidden sibling/parent access, private-directory bypass, unknown member/route, illegal whole namespace, receiver-only selection and reexport cycle. Assert exact diagnostic codes, typed reasons and source/secondary locations where relevant.
- [ ] Verify deterministic diagnostics, interface equality and provider ordering across repeated runs and reversed legal discovery/registration orders. Avoid tests tied to incidental hash-map iteration, local IDs or backend-generated text.
- [ ] Run affected unit and integration lanes through their actual runtime target. Assert executable results. Use benchmarks only if measurements indicate material parsing/resolution or compilation regressions, not as a replacement for correctness.

**Phase 4 mandatory gate**

- [ ] Conduct a cross-stage correctness audit: source path validation, module/package namespace visibility, exact provider selection, no resolver fallthrough, semantic origin stability, fingerprint invalidation, target/backend reachability, asset emission and prelude boundaries.
- [ ] Conduct a redundancy/style audit following `style-guide.mtf` and `moth-code-review-guide.md` if available in the implementation environment. Search for code bloat, duplicate indexes, repeated path parsing, unnecessary allocation, weak tests, misleading comments and transitional or stale paths.
- [ ] Run `just validate` at this integration checkpoint, plus targeted additional package/runtime cases and relevant platform/feature lanes. Resolve failures before proceeding and commit a passing integrated slice. Do not claim `just validate-full` unless it was run.

## Phase 5: Canonical docs, matrix, final review and completion

**Deliverable:** The implemented extension has one accurate permanent language contract, updated status records and a merge-ready branch with no existing-package API migrations.

### Task 5.1: Update permanent references, examples and progress

- [ ] Update `docs/docs/directives/directives.mtf` for the optional path operand, syntax, attachment, same-target diagnostic and prohibition on namespace/wildcard reexports. Replace the old operand-free-only statement rather than appending a contradictory exception.
- [ ] Update `docs/docs/project-structure/public-api.mtf` and `docs/docs/packages/public-reexports.mtf` with root/nested examples, relative route ownership, explicit selection, aliases, duplicate/collision rules, same-route merging, private source separation and receiver method behaviour.
- [ ] Update `docs/docs/packages/dependency-paths.mtf`, `dependency-clauses.mtf`, `module-visibility.mtf` and relevant project-local package references so public routes are documented as explicit provider paths under the original facade. Keep source namespace records shallow and make it clear that `@core/html as html` does not enable `html.tables.cells`.
- [ ] Update the cheatsheet only to the extent needed for the delivered syntax. Update `docs/compiler-design-overview.md` and `docs/build-system-design.md` in their owning sections to define one source directive fact, its interface identity/fingerprint and the Stage 0 public route vs private package resolution boundary. Do not copy this plan into the permanent docs.
- [ ] Update `docs/docs/progress/@entry.moth` in the existing API facade export/reexport and namespace/dependency rows, with status and measured test coverage that reflect *delivered* support. Update `docs/docs/progress/packages-and-builders/@entry.moth` source-backed/package namespace rows only where their status, coverage or notes actually change. Do not mark unrelated binding-backed APIs or runtime targets as supported.
- [ ] Add concise roadmap deferred notes under the relevant existing deferred design section: future broader `$export` facade-boilerplate redesign, multiple public destinations/aliases on a single target, whole namespace forwarding and recursive source namespace/member access are **not** authorised by this feature. Revisit only with actual use cases and a separately accepted design. Keep unrelated and speculative ideas out of matrices.
- [ ] Preserve current `packages/core/**`, `packages/builder/**` and project package public APIs. Do not migrate `@core/html` tables. The later package rewrites can adopt routes explicitly as a separate change.

### Task 5.2: Final verification, review and handoff

- [ ] Run the documentation gate from `validation.mtf`: `moth check docs --terse`, then a documentation release build (`moth build docs --release` or the documented Cargo equivalent). Inspect links, examples and generated output without directly editing `docs/_release/**`.
- [ ] Re-run targeted header, resolver, public-interface, package, alias, prelude, JS runtime, negative diagnostic and incremental tests against the final tree. Run `just validate`, then `just validate-full` for merge readiness. Repeat invalidated checks after final corrections.
- [ ] Perform the mandatory `AGENTS.md` **Slice review**: validate every accepted contract in this plan, inspect changed modules for stage ownership and duplicates, review diagnostic and test honesty, audit Rust style/readability and confirm no obsolete behaviour or compatibility shims survived.
- [ ] Inspect the final diff for accidental public package changes, generated artefacts, unrelated files, stale claims and missing matrix/reference updates. Confirm `index.md` changes are needed only if module/file ownership changed, and do not invent new audit-log coverage. Log unrelated bugs in `docs/bug-reports.md`.
- [ ] Record commands, results, known limits and reviewed test coverage in the implementation PR handoff. Stop and request Nye's decision if a remaining issue would change the accepted language contract rather than guessing.
- [ ] On completion, **delete this plan and remove its roadmap bullet in the same finishing commit**, following `docs/roadmap/roadmap.md`. The permanent docs and matrices retain the delivered design and any accepted deferred work. Core Math fast counterparts then become the next sequenced roadmap item.

**Final acceptance criteria**

- [ ] `$export` without a path works exactly as before, while `$export @forms` and `$export @forms/validation` publish explicit declarations and direct-selection reexports under those paths.
- [ ] Explicit selected and bare-route dependencies work through normal, support, external project and registered Core/Builder source-backed facade boundaries, without exposing private files or recursive parent namespace members.
- [ ] Public routes merge distinct names, detect duplicate or case-conflicting visible identities and preserve all existing source-target and foreign-symbol origin, type and diagnostic guarantees.
- [ ] No registered package/physical module is manufactured, no new prelude binding appears and no existing Core or Builder package export path changes.
- [ ] Runtime tests, stable public interface/fingerprint tests, incremental reuse evidence, deterministic diagnostic tests, style audit, documentation checks, `just validate` and merge-readiness `just validate-full` all pass for the final code-bearing tree.

## Execution guidance

Use one active implementation phase at a time. For every task: add a test that fails for the intended reason, run that test, implement the smallest complete change, rerun the test and commit after its owning phase gate. Do not keep failing or skipped tests across a phase boundary. A phase may be split into smaller reviewable commits, but its mandatory gate must pass before starting the next phase.

If the current repository has moved an owner or substantially changed export syntax, first map the new owner and adapt these file references without changing the accepted semantics. An architecture conflict that requires a second namespace system or expands `$export` eligibility is a blocker requiring Nye's approval, not permission to over-engineer the implementation.
