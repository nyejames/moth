# Fixed root roles, API facades and entry separation

> Status: Accepted design, queued for implementation. This plan describes final behaviour and the one-way source migration, not current compiler support.
>
> Reviewed baseline: `main` at `12b936de9989f0fbc18c30079ac7f97307ef0286` (8 November 2026), including merged PR #30. Verify the implementation branch against this baseline. The current Rust compiler still recognises arbitrary `@*.moth` and `+*.moth` roots, treats normal roots as possible starts and selects HTML entries by root activity.
>
> Authority: this plan owns the fixed-root-role cutover. `docs/roadmap/plans/general-directives-plan.md` already delegates entry selection to it and has no `$entry` implementation phase. Keep the remaining directive work separately queued.

## 1. Purpose and design authority

Moth uses one structural module per rooted directory. Make its three possible authored root-file roles explicit and give each a separate responsibility:

- `@mod.moth` is the API-only normal module facade. It exports the surface visible through the normal parent/child module relationship.
- `@package.moth` is the API-only support facade. It exports the surface visible through the existing scoped support-package rules, or the external project package facade when it is at the selected project root.
- `@entry.moth` is the executable entry root. It owns dormant top-level runtime work and direct output that the selected builder may activate. It has no export surface.

Each is optional. Any combination is legal, including all three in one directory. A directory containing at least one of these files is exactly one structural module, not one module per role. An entry-only directory is a valid module even though it publishes no importable module API. No other authored `@` root names are permitted. Eliminate authored `+` support-root markers. Keep the special build-only filename `config.moth`; it does not create a semantic module and is not renamed to `@config.moth`.

This **supersedes** PR #30's queued `$entry` directive and the earlier accepted rule that every normal root may hold dormant runtime. There is no `$entry`, `$page` or other root-purpose directive in this final model. Future `$export` is a **declaration modifier**, restricted to `@mod.moth` and `@package.moth`, and is not an entry marker. The existing `export:` block is the working implementation pending its separately approved `$export` syntax migration. Do not invent `$export` operands or block grammar here.

This plan deliberately keeps the accepted module visibility topology, compiled immutable interface model, graph scheduling, generated-function handling, builder selection, project bootstrap and link-level validation. It changes where root executable work may appear, which root files publish API surfaces and how entries are selected.

### Design owners to read before code

Read the opening authority text and all applicable root/module, interface, entry, source-index, graph, config, compilation and fingerprint sections of:

- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/docs/project-structure/{module-roots,project-layout,project-package-facade,public-api,entry-runtime-and-fragments,html-routing-and-artifacts,entry-config,project-config}.mtf`
- `docs/docs/packages/{project-local-packages,module-visibility,dependency-paths,public-reexports}.mtf`
- `docs/docs/directives/directives.mtf`
- `docs/roadmap/plans/general-directives-plan.md`, specifically directive attachments, `$config` and the superseded `$entry` section
- `docs/roadmap/plans/built-in-testing-and-test-modules-plan.md` for the future test-only visibility exception
- `docs/developer-docs/style-guide/{style-guide,testing,validation}.mtf`, `moth-code-review-guide.md` and `index.md`

Some permanent project-layout and architecture pages still describe retired `$project`, `$html_builder` and `$entry` proposals. Reconcile these against the already-updated general-directives plan and the existing named `project`/builder configuration records. Do not restore discarded configuration directives.

## 2. Normative source and ownership contract

### 2.1 Root file vocabulary and combinations

| Recognised file | Structural role | Declares exported API | Implicit `start` | Top-level runtime and direct output |
|---|---|---|---|---|
| `@mod.moth` | Normal public facade | Yes | No | Rejected |
| `@package.moth` | Scoped support or external package facade | Yes | No | Rejected |
| `@entry.moth` | Entry-only executable root | No | Yes | Permitted |
| `config.moth` | Project bootstrap, outside the module graph | No | No | Rejected by its existing self-contained config contract |
| Other ordinary `.moth` files | Role-neutral, module-owned implementation | No direct module facade | No | No independent entry/runtime output |

There is at most one file of each special role per rooted directory. A module may have one, two or three present roles. A directory with only `config.moth` is not rooted. No implicit facade is manufactured for entry-only or package-only directories. An authored root filename never supplies the dependency name: names continue to come from the containing module directory or registered package identity.

Use the closed root filename vocabulary in the selected source tree. Reject stale `@site.moth`, `@page.moth`, `@api.moth`, arbitrary `@*.moth`, `+*.moth` and `@config.moth` with a source-structure diagnostic rather than treating them as ordinary helper files. Preserve the existing legacy `#*.moth` rejection. Check case-only variants such as `@Mod.moth` and duplicate visible/canonical claims. The `@` reservation applies to compiler-owned source filenames in the selected tree; if non-source assets with `@` prefixes exist, explicitly inspect the current resource policy before deciding whether those should be rejected as well. Do not silently reclassify an ordinary asset.

Every root claim must resolve to a distinct physical file within its structural module. Preserve symlink containment, aliased-root, duplicated-root, canonical ownership, hidden-directory, output-root exclusion and deterministic traversal rules. A symlink's visible root name determines its declared role, not its canonical target's basename. The same physical file cannot claim two root roles or be claimed by multiple structural modules.

### 2.2 One structural module and three isolated root views

- One directory has one `ModuleId`, one nearest-root ownership boundary, one canonical source inventory, one graph node, one compiler module service invocation and one atomic publish-or-diagnose outcome.
- Each present special file is a separate root-local declaration and visibility scope. None of the three `@` files can resolve declarations authored directly in either of the other two. They cannot import the other root by a direct special-file path, role alias, re-export or synthetic same-module lookup.
- Root-local declarations are accessible in their own authored root file only. They do not flow into the role-neutral environment of ordinary helper files. This is necessary: if helpers could read root-local declarations, they could mediate forbidden cross-root cycles or leak facade-private implementation details.
- Ordinary `.moth` helpers are shared implementation sources, prepared and semantically owned once. Each root may explicitly bind legal helper declarations through the module's existing source dependency mechanism. Helper-to-helper dependencies follow the existing local declaration-ordering contract. Helper files must not depend on root-local declarations.
- A root cannot acquire the declarations of another root merely by being in the same structural module. Private helpers remain private unless exported explicitly through an applicable API facade.
- Direct declarations with the same spelling in different root files are independent when their visibility scopes do not overlap. A name conflicting with a shared helper that is in scope is diagnosed normally. Public collision validation still checks overlapping *consumer* visibility scopes.
- Preserve the role-qualified stable origin of root-local declarations and the `Shared` origin of ordinary helpers. `@entry` declarations require their own entry-qualified origin so private executable identities cannot collide with normal or support facade declarations. Re-exporting one helper through both API facades preserves its defining origin but uses distinct export-binding identities.
- Same-module shared helpers are not duplicated into independently compiled root modules. Stage 3 local ordering, AST/HIR construction and borrow/lifetime analysis must not process a shared declaration as three independent definitions. Root-specific visibility views consume one authorised shared declaration authority.
- Treat any attempted cross-root dependency as a user diagnostic with both meaningful sites where available, never a fallback import and never an internal panic. Continue to validate remaining ordinary graph cycles independently. Root isolation removes one class of possible cycles, not every cycle in the project graph.

### 2.3 Normal and support facade visibility

`@mod.moth` is the normal API boundary. A parent imports a direct child module's published normal surface through the existing module path rule. No parent gains access to the child's `@entry` or `@package` root-local declarations through that import. An entry-only child has no importable surface, so a dependency on it must receive a missing-surface diagnostic. A support-only child has no normal surface, but its support interface remains importable by an eligible consumer under the existing scoped support-package rule. Never invent a normal interface or fall back from an explicitly selected surface to another.

`@package.moth` preserves the **existing** scoped support-package topology:

- The exposed package name comes from the directory.
- Eligible owner/normal sibling/descendant consumers receive only the scoped support interface under the existing consumer-position rules.
- A support facade cannot consume its own facade, an impermissible normal sibling, its parent consumer or a same-scope support sibling. Strictly outer-scope support packages and permitted private descendants retain their existing rules.
- A paired directory has both API surfaces with the same established consumer-specific selection. The import resolver selects the surface through the constructed namespace edge, never by a nearest-match or normal-then-support fallback.
- The selected project root's `@package.moth` is the external project package surface. It remains invisible to internal project modules, cannot import `@project` and cannot export facts whose public or reachable implementation transitively depends on private project globals. A project can publish a package and an entry simultaneously. Without root `@package.moth`, it has no external Moth package surface, even if it has `@mod.moth`.
- Separately compiled source packages keep their registered public surface contract. A Core/Builder source package may publish a normal `@mod` surface (for example `packages/html/@mod.moth` and `packages/web/site/@mod.moth`), while an external **project package** is assembled only from its project-root `@package`. A support-only registered source package can expose its support surface. Never require `@package` for all source-backed packages. Consumer inputs do not enter dependency configuration, and importing a package never activates its `@entry`.

Use the same source-defined public interface representation for normal and support facades. A missing facade produces no fake/empty public interface. Preserve stable origins, type/effect/lifetime summaries, provider edges and transactional publication.

**Support-owner and consumer-role rule:** for scoped support visibility, a structural module with `@mod` **or** `@entry` occupies a normal consumer/owner position. This does not manufacture a normal public interface for an entry-only module. The nearest normal-position ancestor of a support-only module may therefore be entry-only. A directory with both `@entry` and `@package` still has separate consumer roles: the entry uses normal-position rules, the support facade uses strict support-scope rules and shared helpers may import only providers legal to every root that can consume them. The support facade cannot gain entry privileges from its co-located entry. Preserve the exclusion of a support package's private descendants, its own facade and same-owner-scope support siblings. Apply the existing `SupportVisibilityTopology` / `support_is_visible_for_role` owner in `project_module_graph.rs` and the per-source guard `cross_module_support_import_is_visible` in `module_inventory.rs`. Do not add a second scope algorithm or one namespace map per root.

### 2.4 Entry-only surface

- `@entry.moth` permits private definitions, top-level runtime statements, direct compile-time fragments, runtime fragments and registered direct builder-slot contributions, as appropriate to the selected builder.
- It rejects export surfaces and export modifiers, including current `export:` blocks and the future `$export`. A declaration in `@entry` is never externally importable through `@mod` or `@package` unless its implementation is moved to a shared ordinary file and explicitly exported therefrom.
- Only `@entry` has a compiler-owned implicit `start` and the resulting dormant root metadata and fragment-collection contract. `@mod` and `@package` have no `start`, no root runtime fragments, no route metadata and no entry lifecycle. Functions defined inside those API roots are still ordinary callable runtime functions.
- `@entry` presence is **entry eligibility**, not unconditional program execution and not an indication of a physical backend. The selected builder and command decide applicable entries. Binding a module or compiling a dependency never invokes that module's entry. A builder may reject an unsupported entry type through its capability contract.
- HTML V1 selects at most one directory-derived route per `@entry` directory and derives the route only from directory position. Static-only, metadata-only and empty marked entries are eligible. Never select entries by non-empty HIR, root activity, a direct `$in`, a special page filename or rendered output length.
- `check` validates zero-entry projects and all discovered normal/support/entry roots as applicable without emitting output. HTML application `build` and `dev` require at least one suitable `@entry` anywhere in the selected project tree. No homepage is required. Development launch uses the real homepage when present, otherwise the lexicographically first existing route. It does not invent one.
- Keep the current active `start` error-channel, compile-time/runtime fragment ordering, runtime fragment staging, builder-slot validation, resource unions, reachable target checks, runtime-error presentation, JS/Wasm partition rules and link-level memory validation. A builder activates only the selected compiled entry, not other entry roots reached as dependencies.
- The entry root's start activity and metadata have a separate entry/dormant-root fingerprint from the normal and support public-interface fingerprints. Changing entry implementation must not invalidate ordinary API consumers when the public interface is unchanged.

### 2.5 Config and special single-file operations

`config.moth` stays an optional, self-contained build-only input compiled before source discovery. It retains ordinary named `project` and selected builder records, exactly typed `$config` input contracts, its existing fallback rules and the specialised config compilation service. It never provides an importable declaration or creates a root. An ignored nested `config.moth` must not establish a module boundary. Do not add per-module `@config`, project selector directives or config-file dependency graphs.

Synthetic single-file compilation is a separate explicit command path. It must not require an authored special filename or a source-level `$entry`. When a single-file application command explicitly selects an executable output, classify the synthetic root as entry-eligible by the command's existing synthetic selection contract. A declaration-only synthetic check remains valid without manufacturing an application artefact. Do not let a synthetic entry create a real filesystem-backed module API or bypass the normal source-identity and error-reporting rules.

### 2.6 Export syntax boundary

Current code uses root-scoped `export:` blocks. In the final language surface, `$export` will replace them, but its operand and attachment grammar remain separately unsettled. For this implementation:

1. Restrict existing `export:` to `@mod` and `@package` during the root-role cutover.
2. Reject `export:` in `@entry` and ordinary helpers with a root-specific diagnostic.
3. Keep validated public declarations, direct-selection re-exports, facade privacy and closed exported error/type/effect/lifetime contracts unchanged.
4. Document the accepted future rule: `$export` is allowed only in `@mod` and `@package`, never in `@entry`, helpers, templates or config.
5. When the separately accepted `$export` form is implemented, replace `export:` outright. Do not keep two supported export syntaxes, and do not invent a special `package export` or visibility-selector directive.

The general-directives plan should now have exactly two attachment families required by delivered/queued features: template-head directives and next-construct source modifiers. Remove the third module-root-directive family that existed solely for `$entry`. Preserve the contiguous next-construct attachment and one-operand grammar for `$config` and other appropriate future source directives.

## 3. Compilation and graph invariants

- Stage 0 is the single source of root/file/ownership facts. It discovers three fixed root roles and records one `ModuleId` per rooted directory before compiling bodies. No parser stage re-scans filesystem names or guesses roles from canonical symlink targets.
- Stage 0 publishes one prepared module input containing a fixed, role-keyed set of up to three root source IDs and one role-neutral owned source set. Exactly one source ID is selected as the preparation anchor when any roots exist. The anchor is not the implicit `start` or the package public surface. Do not create extra jobs, duplicate worklists or alternate public roles to satisfy a mandatory active-root field.
- Header preparation consumes Stage 0's `SourceId`-keyed root-role facts, including the visible symlink spelling. It must not guess a role from the canonical path or rely on an `entry_file_id` meaning both a preparation anchor and executable start. The compiler builds role-aware nameability facts and rejects cross-root reads before AST. Stage 3 orders shared and root-local declarations without false edges.
- The canonical compiler service constructs a single result with optional normal/support interfaces and an optional entry start/metadata. All present surfaces succeed together or diagnose together. Validate the published set against the structural root inventory in `ModuleArtifactStore` and `CompiledGraphBoundary`. No primary facade is mandatory when a module is entry-only.
- The module's validated executable and link facts can contain callable functions from both API facades, shared implementation files and the entry. A missing entry never invents `start`. An absent facade never fabricates an interface just to fit an existing non-optional field.
- Preserve immutable published artefacts, generated sidecar ownership, borrow/lifetime summaries, fixed failure lanes, source and path identity lifetimes, deterministic diagnostics, budget/scheduling cutoff, per-function link facts, output resource policy and physical-variant identities.
- Keep stable semantic identifiers separate for the shared directory, each API/root-local role and entry assembly. Re-exporting one helper through two facades keeps its shared defining origin and distinct export bindings. Renaming a root file must not change its logical module path or the origin of an ordinary helper. Distinguish an entry-local private executable from a similarly named facade function.
- Keep existing cross-package isolation and no-backend-source-parsing rules. Entry selection and package assembly consume completed compiler facts only. No post-compilation AST/TIR reconstruction.

## 4. Existing code to reuse and simplify

This map identifies current owners on the reviewed baseline. Read the actual implementation before moving a type or adding a helper. Prefer modifying these owners over introducing another role registry or graph layer.

| Owner | Required change or reuse |
|---|---|
| `src/compiler_frontend/source_packages/root_file.rs` | One closed root filename classifier and one special-file path diagnostic policy. Retain `config.moth` and legacy `#` handling. Remove the broad `@*.moth` / `+*.moth` predicates after the cutover. |
| `src/build_system/create_project_modules/source_tree_index.rs` | Extend `ClassifiedDirectoryRoots`, `DiscoveredModuleRecord`, root claims and `SourceRecord` role classification in the existing single traversal. Preserve canonical and visible paths, deterministic source IDs, symlink/hidden-output rules and owned-source indexing. Cover `bounded_module_roots_for_single_file` and `single_file_dependency_keys`. |
| `src/build_system/create_project_modules/module_identity.rs` and `src/compiler_frontend/semantic_identity.rs` | Extend `ModuleIdentityRecord` and `ModuleRootRole` with an entry role. Keep `Shared` for ordinary helpers and `SyntheticProjectContext` for `@project`. Keep one `ModuleId` per directory. |
| `src/build_system/create_project_modules/project_module_graph.rs` | Derive entry candidates from actual `@entry` presence. Adapt `SupportVisibilityTopology`, `nearest_normal_ancestor` and `support_is_visible_for_role` to normal-position `@entry` owners while retaining strict support-role checks. Reuse the same predicate for project and package indexes. |
| `src/build_system/create_project_modules/module_namespace.rs` and `module_inventory.rs` | Keep one indexed namespace, select only present `@mod`/ `@package` provider surfaces and filter by the requesting source role. Extend `cross_module_support_import_is_visible`. An entry-only child remains a structural boundary but has no normal importable API. |
| `src/build_system/create_project_modules/{module_inventory,module_preparation,source_preparation,source_package_discovery,prepared_module}.rs` | Carry the three root IDs through one seed, one preparation and one job. Preserve the check-only and separately compiled source-package paths. Do not copy the candidate-source vector for each root. |
| `src/compiler_frontend/headers/{parse_file_headers,types}.rs`, `src/compiler_frontend/module_dependencies.rs` and nameability/AST owners | Replace ambiguous active/entry file naming with explicit preparation anchor, actual entry source and role facts. Reuse retained source IDs, declaration shells, visibility and ordering. Root-local declarations stay invisible to other roots and ordinary helpers. |
| `src/compiler_frontend/module_compilation/{prepared,service,artefact,outcome,context}.rs` and `public_interface/` | Prepare all roles in one transaction. Build zero, one or two API projections only for present facades. Keep one shared HIR/borrow/lifetime/generated lane and optional entry-only start/fragment metadata. Reuse the existing public-interface draft and export-seed builders rather than duplicate them per role. |
| `src/build_system/create_project_modules/{module_artifact_store,compiled_boundary}.rs` | Replace mandatory-primary-interface assumptions with exact optional-surface validation. Preserve atomic publication, stable origins, interface lookup, generated sidecars and diagnosed/blocked outcomes. |
| `src/build_system/build.rs` and `src/projects/html_project/` | Remove only the activity-based **entry selection** predicate. Preserve `EntryAssembly`, entry reachability/resource unions, builder slots and executable `runtime_fragment_count` used by the JS and Wasm emitters. |
| `crates/moth-cli/src/scaffold/{files,start_page_scaffolding}.rs`, `packages/`, `docs/`, `tests/cases/` | Migrate authored roots, generators, real examples and existing fixtures in the same public cutover. |

### Deletion and consolidation targets

- **Delete** `ModuleRootActivity::has_html_artifact_activity` and its activity-selection tests once entry discovery uses `@entry`. Inspect remaining `ModuleRootActivity` fields before removing them: `runtime_fragment_count` is consumed by HTML JS/Wasm. Remove unused `has_non_trivial_root_body` and `const_fragment_count` storage only after checking all legitimate consumers, not by renaming the old predicate.
- **Consolidate** `public_surface_role` selection in `module_identity.rs`, graph projection and `compiled_boundary.rs` around the existing identity table. A missing public surface must be a typed absence where legal, never a fallback to `@entry` or a synthetic empty interface. Keep the specialised registered-source-package selection that legitimately chooses normal or support.
- **Remove** re-derivation of special-file roles from canonical filenames in `parse_file_headers.rs` once source IDs carry Stage 0's authoritative role, including for symlinks. Maintain a narrow standalone/synthetic file-role route instead of adding a second discovery mechanism.
- **Reuse** one existing root-file/public-export seed and `PublicInterfaceDraftBuilder` path for each *present API surface*. A small bounded iteration over the two facade roles is justified, but an extensible role framework, abstract registry, per-root compiler instance or independent entry-interface type is not.
- **Keep local** the project-root external package policy and the registered Core/Builder source-package policy. They have different legitimate consumers, and forcing them behind a misleading universal "must be `@package`" helper would break source packages.

## 5. Implementation phases

Work in this branch after reconciling upstream changes. Make focused commits at the gates below. The intermediate implementation may have internal incomplete plumbing, but the public naming cutover is one coherent checkpoint. Temporary internal adapters must be named, private and deleted at that checkpoint, never preserved as compatibility APIs.

### Phase 0: Rebaseline and inventory

1. Confirm current `main`, the merged PR #30 design and that `$entry` has no live parser/selection implementation. Reconcile only changed code locations, not the accepted contracts.
2. Read the architecture/style/testing authorities in section 1. Inspect the owners in section 4 and the current integration harness rather than treating the map as exhaustive.
3. Inventory `@*.moth`, `+*.moth`, `config.moth`, root activity and generated/scaffold source, including `packages/html/@mod.moth`, `packages/web/site/@mod.moth`, docs pages, test fixtures and source-package boundary roots. Decide per **directory** which of the three roles it needs. A previously activity-driven `@page` generally becomes `@entry`; a public API root becomes `@mod`; a `+` support root becomes `@package`. Split the old normal root when it both exports API and has root output, moving shared implementation into ordinary files.
4. Run and record baseline targeted source-discovery, package-visibility, `moth check`, HTML JS and supported Wasm tests. Record unavailable gates rather than claiming they passed.

**Gate:** one verified migration inventory and targeted baseline. No authored syntax change yet.

### Phase 1: Internal role model and structural graph

1. Keep the existing `ModuleId` and shared-helper identity. Add a distinct `Entry` role to `ModuleRootRole` only where semantic origin or root-specific permissions require it. Keep normal/support as the two public interface roles and `Shared` as helper origin. Use a compact role-keyed root inventory, not three independently compiled modules.
2. Feed that inventory through `SourceTreeIndex`, `ModuleIdentityRecord`, `ProjectModuleGraphNode`, `ModuleEntrySeed`, `PreparedModuleInput` and the source-origin table. Preserve one canonical owned-source vector, one preparation worklist and one graph node per rooted directory.
3. Separate the *preparation anchor* (one selected root source ID) from the *entry source* and each facade source. Rename existing ambiguous `entry_file_id`, `entry_path` and primary-root fields at their true owners; avoid unrelated API renames. The chosen anchor has no semantic authority beyond preparing the one structural module.
4. Keep registered Core/Builder source-package public-root selection valid for either `@mod` or `@package`, with the root package facade still required for external project package assembly. Validate physical duplicate claims and the absence of an importable API for entry-only roots.
5. Add narrow identity/role tests for three single roles, three pairs, triple roots and an unrooted directory. Confirm role-stable shared helper origins and deterministic ordering.

**Gate:** graph, identity and preparation plumbing compiles without new source-level syntax, extra graph nodes, per-root source copies or fabricated API surfaces.

### Phase 2: Root-local scope, imports and exported APIs

1. Reuse prepared declaration shells and the existing nameability owner to enforce three isolated root-local scopes. Each root may see its own declarations and legally bound shared helpers. Other roots and all ordinary helpers cannot resolve a root-local declaration, including via imports, re-exports or alias lookups. Keep same-spelling declarations legal in disjoint root scopes.
2. Restrict the currently implemented `export:` to `@mod` and `@package`. Keep its existing direct-selection re-exports, exported type/effect/lifetime validation and precise diagnostics. Reject exports in entry, ordinary helpers and config. Do not implement undecided `$export` grammar.
3. Project a public interface only for each **present** API facade using the existing export seed and draft builder. Publish two optional surfaces and the shared executable in one artefact. Update `ModuleArtifactStore::interface_for_surface`, publication-shape checks and consumers without empty-placeholder interfaces or compatibility wrapper types.
4. Keep one namespace per structural module, but authorise each cross-module support import by the **source's** root/shared role. An `@entry` source is a normal-position consumer, an `@package` source is a strict support consumer and shared helpers in a module with `@package` must obey the stricter intersection. Use the existing support topology and `cross_module_support_import_is_visible` guard. A `@mod` or `@entry` co-located with a support facade must not grant imports to that facade.
5. Extend scope and provider tests for an entry-only support owner, a support-only intermediate directory, a paired entry/support module, a triple-root module, a shared helper's restricted dependency and same-name root-local declarations. Confirm that a requested missing normal surface does not fall back to `@package` or `@entry`.

**Gate:** no direct or indirect cross-root read, illegal support import or public-surface leak. Compiling a shared helper still produces one declaration and one semantic origin.

### Phase 3: Entry-only compiler work

1. Synthesize `start` only for a real `@entry` root or an explicitly selected synthetic entry, not for an API facade. API-only roots reject direct runtime statements, top-level const/runtime fragments and direct builder slots while still allowing function bodies and API constants. Entry roots reject `export:`.
2. Keep the existing AST, HIR, numeric/borrow/lifetime and generated-function pipeline. Carry optional entry metadata and fragment insertion indexes in the existing module metadata lane. An entryless module has no dormant start or entry fragment schedule. Do not create a second executable lane or a separate entry module job.
3. Keep `Error!` startup propagation, static/runtime fragment order, builder slot semantics and the failure-path fragment-publication rule. Failure of any present facade or entry invalidates the one module transaction and both potential interfaces.
4. Test API-only, entry-only, entry/support, entry/mod and triple-root modules, including an empty entry and a metadata-only entry. Assert that all published HIR, generated requests and start identities remain consistent with the roles actually present.

**Gate:** only entry roots produce `start` or root output, and interface-only modules compile with no fake executable entry.

### Phase 4: Single public filename and selection cutover

1. Change the canonical filename recogniser to exactly `@mod.moth`, `@package.moth` and `@entry.moth`. Reject other special `.moth` filenames (`@page`, `@api`, `+package`, `@config` and legacy `#`) as authored source diagnostics, not ordinary helper files or invisible no-ops. Inspect non-`.moth` `@` resources separately and retain existing source-kind semantics unless there is a proven name collision. Preserve case-only, symlink, canonical path and non-regular-root validation.
2. Reuse Stage 0's indexed identities for direct-child `@mod` imports, scoped `@package` provider selection and the absence of an entry API. Deny direct special-file imports, private traversal and namespace fallback. Confirm registered Core/Builder source packages continue to compile with their permitted API facades.
3. Derive `ProjectModuleGraph::entry_modules()` only from project-local `@entry` presence, excluding separately compiled dependency entries. In `ProjectCompilation::from_successful_boundaries`, delete `root_activity.has_html_artifact_activity()` gating. Preserve `EntryAssembly`, reachability and resource assembly.
4. Preserve `check` with zero entries, no-homepage descendant entries and command-specific no-entry outcomes. Make explicit synthetic single-file application selection work without `$entry` or an authored special filename. Keep any standalone `@entry` file subject to the command's selected-root policy, not automatic project discovery.
5. Migrate `packages/`, `docs/`, real fixtures and CLI scaffolds in the same checkpoint. Update declarations and imports when splitting previously combined executable/export roots. Route derivation still uses directories, not the new root filename.
6. Delete temporary root adapters and obsolete activity-selection tests. Retain the working fragment counters, reporting and ownership lanes still used by backend assembly.

**Gate:** one public root vocabulary, real docs/site HTML JS build and supported Wasm build, package-only and mixed project examples, and no activity-based entry selection.

### Phase 5: Boundary hardening and regression validation

1. Exercise canonical, check-only, explicitly selected single-file and separately compiled source-package paths. Confirm entry-only, support-only and triple-root ownership, orphan-file diagnostics and no skipped root diagnostics under scheduling budgets.
2. Verify that only an external **project package** requires its project-root `@package`; registered source-backed Core/Builder packages can retain `@mod` where their public-root contract permits it. Validate source-package identity isolation, project globals restrictions and consumer input separation.
3. Validate direct special-file path rejection, case-only and extensionless collisions, symlink aliases, hidden/output exclusions, resource references, source package links and deterministic error spans. Preserve expected operational failures versus source diagnostics.
4. Reconcile the queued test-module plan from `+` test facades to test-only `@package.moth` without implementing `moth test`. Preserve reserved `tests/` ownership and production-to-test invisibility. A test support facade must never acquire normal production visibility.
5. Verify entry changes do not change API fingerprints if public facts are identical, shared helper export changes invalidate affected consumers, and private entry changes only affect executable/root/link facts. Retain the existing five fingerprint domains rather than adding one per root.
6. Extend the existing narrow subsystem and integration tests. Each new test must protect a distinct failure mechanism, not merely the spelling of a second root filename.

**Gate:** boundary, registered-package, test-design, fingerprints and deterministic compilation checks pass without duplicated source preparation or extra root-specific caches.

### Phase 6: Documentation, cleanup and completion

1. Update permanent authorities `docs/build-system-design.md` and `docs/compiler-design-overview.md` for the three roles, role-local visibility, support owner/consumer semantics, optional interfaces, entry-only start, registration of source packages and entry assembly. Update relevant `docs/docs/project-structure/` and `docs/docs/packages/` references plus their Basic counterparts. Document the final surface, not the migration as the language contract.
2. Audit `docs/roadmap/plans/general-directives-plan.md` and the canonical directive pages rather than rewriting already-correct sections: they now have **two** attachment contexts and no `$entry` implementation phase. Preserve `$config` typed input contracts and template formatter migration. Keep the future `$export` restriction to the two API facades, without introducing its grammar.
3. Update `docs/roadmap/plans/built-in-testing-and-test-modules-plan.md`, roadmap and progress matrices, cheatsheet, `index.md`, first-party package docs and scaffold examples. Fix known stale `$project`/`$html_builder` examples in project-layout docs, using existing named config records. Update links after renaming docs roots.
4. Remove obsolete classifier predicates, duplicate facade-selection logic, no-longer-used entry activity predicates, abandoned root-only visibility exceptions, stale comments and migration bridges. Do not retain previous public filename spellings in compatibility code. Leave legitimate fragment-count handling in place.
5. Follow `docs/developer-docs/style-guide/{testing,validation}.mtf` and `moth-code-review-guide.md`: focused checks at slice boundaries, `just validate` at integration checkpoints, and `just validate-full`, docs release build, applicable feature/OS gates and code-bloat audit for merge readiness. Run `rg` over implementation, docs and fixtures for old `+` roots, arbitrary `@*.moth` roots, `$entry`, `has_implicit_start` on Normal and `has_html_artifact_activity`. Retain intentional migration-diagnostic examples only.
6. Record exact commands, results and blockers. Update the roadmap/progress status only for delivered code. Report remaining separate work: `$export` syntax and `moth test`.

**Gate:** documentation and code describe one implemented contract, full applicable validation is recorded, and an independent review finds no parallel root representation or unsupported claims.

## 6. Required focused regression coverage

Use existing primary owners: `src/build_system/tests/` for discovery, graph, namespace and artefact invariants, compiler-local tests for role/nameability and root HIR, `tests/cases/` for observable programs, and existing HTML/CLI tests for output/scaffolding. Follow the test guide's admission rule. Consolidate repetitions rather than building a new root-role test framework.

| Area | Required distinct regression |
|---|---|
| Discovery | All seven non-empty root-role combinations, missing-root directory, rejected unknown/legacy root, same-physical-file alias, case-only collision and deterministic ordering |
| Scope | Same helper accessed from three roots, one shared declaration/HIR definition, root-to-root and helper-to-root rejection, same-spelling local declarations with no accidental collision |
| Visibility | A parent's child `@mod`, eligible sibling `@package`, entry-only child with no normal API, support-only private subtree and no normal-to-support fallback |
| Support owner | Support nested under entry-only normal-position owner, paired `@entry`/`@package` with strict support imports, shared-helper intersection and project/package topology parity |
| Public API | A helper re-exported through both facades with shared declaration origin but two export bindings, private nominal protection, invalid `export:` in entry or helper, exact optional interface publication |
| Registered package | Existing `packages/html/@mod.moth` and `packages/web/site/@mod.moth` remain registered and importable, while external project package assembly still requires root `@package` |
| Entry | Empty, static-only, metadata-only and runtime `@entry`, no facade start, imported entry never activates, one start per selected entry |
| Commands | Zero-entry check, descendant-only HTML build/dev, no manufactured homepage, single-file synthetic application, package-only outcome |
| Runtime and resources | JS/Wasm parity, runtime fragment insertion count, failure staging, direct `$in`, resource union and output URL context |
| Config and tests | Optional top-level `config.moth`, ignored nested config, unchanged `$config` semantics, test-only `@package` not exposed to production |
| Caching and diagnostics | Stable shared and role-local identities, entry-only edits not changing API projection, typed errors at authored sites, unchanged error-budget and symlink safety |

## 7. Completion and exclusions

Complete only when the three fixed `@` files are the sole authored root roles, every structural module compiles once, all present root-local scopes remain isolated, only `@mod` and `@package` expose APIs, only `@entry` owns root runtime and project entries are selected by its presence. Preserve explicit source-package registration policy, scoped support visibility, bootstrap config, synthetic single-file commands, immutable interface/HIR/link handoffs, test-only separation and JS/Wasm output. Remove legacy filenames, `$entry`, compatibility paths and activity-based selection together. The affected permanent documentation and focused/full validation must match delivered behaviour.

Out of scope: implementing the future `$export` grammar or `moth test` runner, per-module config, new root roles, multiple entries per directory, per-root compilation graphs, user-defined visibility directives, target selection by filenames or a new language/parser framework.
