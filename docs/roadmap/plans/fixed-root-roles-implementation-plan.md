# Fixed root roles, API facades and entry separation

> Status: Accepted design for implementation planning, based on the project-lead decisions in the design discussion of 7 November 2026. This document specifies the target behaviour, migration and verification work. It does not claim that the changes are implemented.
>
> Baseline: `nyejames/moth`, PR #30 `docs/directive-attachment-rules` at `3c0264bc0fd8b335b7dabfd90dc092f254a5d0b7`. Recheck the branch head and merged prerequisites before editing. Source locators below were checked against that branch, but Rust names and line positions may change.
>
> Destination: `docs/roadmap/plans/fixed-root-roles-and-entry-plan.md`. Integrate this plan with `docs/roadmap/plans/general-directives-plan.md` before activating the queued general-directives work. PR #30 is documentation-only and has not implemented `$entry`.

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

The PR branch's `docs/docs/project-structure/project-layout.mtf` still contains stale `$project`/`$html_builder` examples even though the PR's accepted config contract uses named records. Reconcile this documented contradiction as part of the plan. Don't reinstate retired directive-config forms.

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
- Dependencies compile in their own package boundaries. Consumer inputs do not enter dependency config, dependency entry roots are never activated by importing the package and a dependency needs the valid external facade its package contract requires.

Use the same source-defined public interface representation for normal and support facades. A root absent from the directory produces no fake/empty public interface. Preserve stable origins, public type/effect/lifetime summaries, provider edges and transactional publication.

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
- Stage 0 publishes one prepared module input with optional source IDs for each of the three roles plus one role-neutral owned source set. Do not create three independent module jobs or duplicate worklists for a paired/triple-root directory.
- Header preparation retains file roles per source ID, including source-file-vs-root privileges. The compiler builds role-aware local nameability/visibility facts and rejects root-to-root lookup before AST. Stage 3 orders shared and root-local declarations without adding false cross-root edges.
- The canonical compiler service constructs a single complete module semantic result with optional normal interface, optional support interface and optional entry metadata/executable start. All three are published together or none are published. A failed entry invalidates the module's API facades and vice versa.
- The module's validated executable and link facts can contain callable functions from both API facades, shared implementation files and the entry. A missing entry never invents `start`. An absent facade never fabricates an interface just to fit an existing non-optional field.
- Preserve immutable published artefacts, generated sidecar ownership, borrow/lifetime summaries, fixed failure lanes, source and path identity lifetimes, deterministic diagnostics, budget/scheduling cutoff, per-function link facts, output resource policy and physical-variant identities.
- Separate stable semantic identifiers for the shared directory, root surface, exposed declaration and entry assembly. Role and logical module path must distinguish authored root-local declarations. Cosmetic filename migration must not incidentally make shared helper declarations into duplicate or unstable origins.
- Keep existing cross-package isolation and no-backend-source-parsing rules. Entry selection and package assembly consume completed compiler facts only. No post-compilation AST/TIR reconstruction.

## 4. Repository impact map

This is a locator map, not authority to add new layers. Check the actual current code on the implementation branch before editing.

**Stage 0 and root file classification**

- `src/compiler_frontend/source_packages/root_file.rs`: currently recognises arbitrary `@*.moth`, `+*.moth`, `config.moth` and legacy `#` names. It is the shared filename policy owner.
- `src/build_system/create_project_modules/source_tree_index.rs`: the single traversal, `ClassifiedDirectoryRoots`, `DiscoveredModuleRecord`, root claims, canonical path checks, source inventory and `SourceTreeDiscoveryStats`.
- `src/build_system/create_project_modules/module_identity.rs`: `ModuleId`, `ModuleIdentityRecord`, role classifier, stable root origins and currently pair-only root slots.
- `src/build_system/create_project_modules/project_module_graph.rs`: `ProjectModuleGraphNode`, entry candidates, role surfaces, adjacency and deterministic compile waves.
- `src/build_system/create_project_modules/module_namespace.rs`: direct-child normal imports, scoped support namespace insertion, consumer-position surface selection and special-file bypass diagnostics.
- `src/build_system/create_project_modules/{module_inventory,module_preparation,prepared_module,source_discovery,source_package_discovery}.rs`: one module seed, prepared source IDs, optional support root, check-only paths and source-package boundary validation.
- `src/build_system/create_project_modules/{source_tree_index,project_module_graph,module_namespace}.rs` may already carry stable coverage and invariants. Extend rather than replicate.

**Compiler semantics and publication**

- `src/compiler_frontend/semantic_identity.rs`: `ModuleRootRole` currently contains `Normal`, `Support`, `Shared` and `SyntheticProjectContext`. `Normal` currently implies `start`. Add a distinct entry classification and move `has_implicit_start` to it. Retain `Shared` for helper origin.
- `src/compiler_frontend/source_module_origin.rs`: source ID to role-qualified stable origin map.
- `src/compiler_frontend/headers/parse_file_headers.rs`: `HeaderParseOptions`, active root, paired support root and `FileRole` assignment. The old `entry_file_id` parameter means the selected compiler root file, not necessarily an authored executable entry. Avoid carrying this overloaded name into the new model.
- `src/compiler_frontend/module_compilation/{prepared,service,artefact,outcome,context}.rs`: one canonical module service, prepared active/support roots, export seeds, optional support projection and currently mandatory primary `PublicSemanticInterface`.
- `src/compiler_frontend/public_interface/`, `src/compiler_frontend/module_dependencies/` and header/nameability owners: re-export projection, root-scoped export policy, same-module visibility and cross-surface isolation.
- `src/build_system/create_project_modules/module_artifact_store.rs`: expects mandatory primary interface and optional support. Adapt to optional API facades without fake empty public interfaces.
- `src/compiler_frontend/ast/`, HIR root construction and function-origin callers: restrict root-runtime compilation and start synthesis to `@entry` while leaving API function bodies legal.

**Linking, builders and tooling**

- `src/build_system/build.rs`: `ProjectCompilation::from_successful_boundaries`, graph `entry_modules()` selection and its current `root_activity.has_html_artifact_activity()` filter, `EntryAssembly`, `ProjectPackageAssembly` and reachability/resource planning.
- `src/projects/html_project/`: HTML shell, direct named slots, route derivation and mixed JS/Wasm. Keep builder metadata, document defaults, resource unions and runtime/error semantics.
- `src/build_system/project_config.rs`, `src/compiler_frontend/single_source_compilation/config.rs`: preserve project bootstrap and named-record extraction.
- `crates/moth-cli/src/scaffold/{files,start_page_scaffolding}.rs`: `moth new html` filenames and starter content.
- `packages/` and `docs/`: first-party Moth source roots and support packages that need migration.
- `tests/cases/`, `src/build_system/tests/`, `src/compiler_frontend/tests/`, `crates/moth-cli/src/scaffold/tests/`: existing integration and unit tests. Use actual nearby test owners rather than creating an overlapping harness.

## 5. Implementation phases

Keep a single feature/worktree branch based on the latest integrated PR #30 and any prerequisite changes. Prefer small commits at the named phase gates, with a single deliberate **public-syntax cutover** rather than two simultaneously supported filesystem syntaxes. A compiler-internal temporary bridge is allowed only if explicitly named, private and deleted in the cutover phase. Do not add persistent compatibility shims.

### Phase 0: Rebaseline and lock the plan

1. Inspect the current branch and reconcile the PR #30 changes against the latest `main`. Confirm that `$entry` has no source parser or builder implementation yet. If implementation has landed in the meantime, remove it during the cutover rather than preserving it.
2. Read canonical compiler/build, root, config, export, package, entry, test and style authorities listed in section 1. Inspect `index.md` and the actual code owners above.
3. Produce a reference inventory of current `@*.moth`, `+*.moth`, `config.moth`, proposed `$entry`, root activity and test fixture uses. Record the exact migration map by directory. Identify the source package entry roots and any tool-generated sources.
4. Resolve only implementation-level ambiguities that cannot be derived from sections 2 and 3. Do not change the approved role, visibility or lifecycle contracts by choosing a convenient Rust representation.
5. Save baseline targeted test and build results, including first-party docs site, scoped package behaviour and single-file compilation. No baseline claim without a command/result.

**Gate:** a source-backed inventory and an agreed role map exist. No source syntax change yet.

### Phase 1: Establish one internal root-role authority

1. Replace the pair-only structural root record internally with a compact three-role presence model. Use one clear role vocabulary and one classifier. Keep role-qualified origins and one role-neutral shared owner.
2. Thread three optional root source IDs through the index, graph, module seed and prepared compilation input. Preserve the one-directory/one-job invariant, source candidate interning and deterministic IDs.
3. Centralise public surface selection as an explicit option: normal and support interfaces may each be absent, and an entry is never an importable API. Never infer the provider role from whichever file happens to be first.
4. Update symlink/root claim and collision representation without adding parallel directory indexes or filesystem passes. Retain the visible authored path and canonical physical path separately.
5. Introduce internal role tests for all eight possible presence combinations, including the no-root case, without yet accepting the new filenames as an alternate public syntax.

**Gate:** role plumbing compiles under the existing source surface while preserving the previous observable results. Any temporary bridge is documented locally for mandatory deletion in Phase 4.

### Phase 2: Enforce isolated root-local visibility and API publication

1. Make header preparation issue root-specific declaration ownership and visibility. Ensure ordinary helper declarations remain shared, while direct declarations in one root cannot enter another root's lookup, ordering or public projection.
2. Prevent ordinary helper files from resolving declarations defined directly inside any root. Keep legal helper-to-helper and root-to-helper nameability.
3. Restrict export parsing and exposure to normal/support facades in the target role model. The interim authored `export:` block remains supported only in those two root files. Reject any export attempt from entry, helper and config sources.
4. Preserve one normal and one support interface type, with optional presence in the publication carrier. Update public-interface seed generation, package-surface checks and `ModuleArtifactStore::interface_for_surface`. Remove assumptions that every module has a public interface.
5. Preserve one semantic transaction, HIR and generated sidecar lane for the structural module. Verify a shared helper exported through two facades has one defining declaration origin and two correctly scoped export bindings.
6. Add focused negative tests for direct cross-root reads, indirect helper reads, direct-root-path imports, cross-root re-exports, leaked type identities, same-spelling independent root-local declarations and support visibility collisions.

**Gate:** isolated public projections and helper ownership are covered by compiler tests. No accidental cross-root provider edge is produced. Avoid widening scope through a special same-directory fallback.

### Phase 3: Make executable work entry-only

1. Move implicit `start` synthesis to the entry root role. The active root chosen for internal compilation must no longer be conflated with the executable root or public facade. Name internal fields accordingly.
2. Parse and validate the `@entry` root as the sole legal source of top-level runtime work and direct fragments/slots. API-only roots reject those constructs with a role-specific diagnostic while allowing normal runtime function bodies.
3. Move finished entry metadata, root activity, static/runtime fragment positions and `start` outcome to the optional entry lane of the completed module artefact. An entryless module must not contain a hidden `start` or fragment activation plan.
4. Retain `Error!` source propagation, runtime fragment staging, compile-time fragment storage, selected builder-slot contracts and HIR/borrow/lifetime/target validation of all authored code.
5. Ensure entry failure invalidates the paired facade publication and vice versa, with no partial results. Preserve source spans and one diagnostic owner.

**Gate:** compiler-module service tests cover entry-only, facade-only, two-facade and triple-root modules. Start/fragment identities originate only from the entry. No duplicate HIR for shared helpers.

### Phase 4: Atomic public-syntax and entry-selection cutover

This phase is the first public change and must land as one coherent branch checkpoint. It may contain several implementation commits, but no intermediate commit is advertised as a supported filesystem language.

1. Activate exactly `@mod.moth`, `@package.moth` and `@entry.moth` in `root_file.rs` and Stage 0 discovery. Remove recognition of arbitrary `@*.moth` and `+*.moth` as canonical roots. Diagnose obsolete/unknown special names precisely.
2. Update namespace discovery so only `@mod` supplies the normal surface, only `@package` supplies support and neither namespace resolves `@entry`. Reject an import of an entry-only module when a normal API is requested. Preserve nearest-child/sibling support visibility and no fallback.
3. Make `ProjectModuleGraph::entry_modules()` mean exactly project-local modules containing `@entry.moth`. Never include API-only modules or dependency package entries.
4. In `ProjectCompilation::from_successful_boundaries`, delete the `root_activity.has_html_artifact_activity()` entry gate. Select the compiled entry presence and builder capability. Preserve static-only and metadata-only output. Leave the existing `EntryAssembly` reachability and resource union implementation intact as far as possible.
5. Preserve project package facade assembly, but select only `@package` at the selected project root. A root `@mod` or `@entry` never acts as the external package facade.
6. Reconcile synthetic single-file compilation as an explicit selection path and update HTML no-entry/no-homepage command results. Preserve directory-derived routes and runtime error handling.
7. Migrate real source files, test fixtures, compiled first-party packages, docs page roots and CLI scaffolds in the same cutover. An old root must neither silently compile as a helper nor be recognised in parallel. Update import diagnostics to describe the directory surface rather than an authored root filename.
8. Delete the private migration bridge from Phase 1. Remove obsolete root filename classification, activity-selection and `$entry` references in production code.

**Gate:** a whole first-party HTML project builds in JS mode and in the repository-supported Wasm mode. Package-only and mixed application/package projects compile. The migration has one public naming contract, no compatibility fallback and no activity-based entry selection.

### Phase 5: Harden command, discovery, package and test boundaries

1. Check entry-only descendants, support-only descendants, paired facade modules and triple-root modules in canonical, check-only and source-package compilation paths. Verify orphan `.moth` files remain diagnosed and never create independent entries.
2. Validate source package boundary admission: a consumed external source package needs its prescribed public `@package` facade. Its internal `@entry` never activates in the consumer. Its independently selected project build may activate that entry.
3. Verify no root-only file is imported as a direct physical Moth file value or as a path bypassing facade boundaries. Preserve extensionless collisions, case folding, symlink safety, hidden outputs and source-kind/provider behaviour.
4. Update the future test-module plan: test support facades use `@package.moth` in their test-only scope, not `+*.moth`. Keep recognised `tests/` units distinct from production modules, including unrooted test-only units. Production cannot discover or bind test support. Do **not** implement the future `moth test` runner in this phase.
5. Check incremental reuse and fingerprint invariants: touching entry-only work does not change API fingerprints, touching shared helper public facts invalidates affected facade consumers, config changes retain their current dependency behaviour and output changes do not mutate semantic identities.
6. Run focused integration, filesystem identity, source-package, CLI scaffold and resource tests. Ensure diagnostics use typed reasons, authored source sites and the correct failure lane.

**Gate:** structural discovery, source-package selection, test isolation specification and reuse checks hold at the appropriate boundaries. No duplicated scans, shadow namespace or per-root module job exists.

### Phase 6: Canonical documentation and full migration audit

This is part of implementation completion, not optional follow-up documentation.

1. Update `docs/build-system-design.md` architectural invariants, project topology, roots, support/project package visibility, source indexing, source sets, command policies, entry candidates/selection, synthetic single-file selection, and fingerprint/link planning descriptions.
2. Update `docs/compiler-design-overview.md` invariants, module root semantic roles, source identities, compiler input/result, optional public interfaces, `start`, header roles, public projection and metadata/link lanes. Explicitly specify that only entry roots synthesize `start`.
3. Update canonical `docs/docs/project-structure/` pages and Basic pages for module roots, layout, API, facade, runtime, routing, configuration, builder slots and scaffold examples. Correct old assertions that root suffixes are cosmetic or that normal roots have dormant runtime.
4. Update `docs/docs/packages/` pages and Basic pages for local support packages, module visibility, dependency paths and public re-exports. Explain normal/support consumer-position resolution without fallback.
5. Update `docs/docs/directives/` and companion overview/Basic docs to remove `$entry`, the three-attachment-context model and root-only placement matrix. Preserve the `$config` and template-head grammar, accurate support labels and the accepted future `$export` restriction.
6. Rewrite the accepted `docs/roadmap/plans/general-directives-plan.md` so its scope is `$config`, existing template formatter migration and the remaining general-source directive machinery. Remove the entire `$entry` implementation phase, acceptance tests and final condition. Point to this plan for the finished root-role contract. Preserve named `project`/builder config records, not obsolete `$project` or `$html_builder` directives.
7. Reconcile `docs/roadmap/plans/built-in-testing-and-test-modules-plan.md`, `docs/roadmap/roadmap.md`, the compiler progress matrix (`docs/docs/progress/@page.moth`), packages/builders matrix, cheatsheet, `index.md`, first-party package READMEs and `moth new` instructions. Keep status/progress in the matrices, not final contract prose.
8. Search documentation, examples, Rust comments and fixtures for `+package`, `+shared`, `@page`, `@site`, `@anything`, arbitrary normal root suffix, `has_implicit_start` on `Normal`, `$entry`, `$page` and old activity-based entry selection. Keep intentional historical references only when explicitly labelled as obsolete and useful for migration diagnostics.
9. Verify docs links and render/build the docs site with the new root filenames and package paths. Do not silently rewrite source-relative file values, resource paths or the final output routes during migration.

**Gate:** permanent architecture and public language references describe exactly one final surface. No conflicting accepted plan claims remain and the docs release build succeeds.

### Phase 7: Final technical and code-size audit

1. Follow `docs/developer-docs/style-guide/validation.mtf` for the final gate. Run focused tests, `just validate`, `just validate-full`, the docs release build and the required feature/platform lanes for changed behavior. Do not claim anything was executed unless logs confirm it.
2. Audit every touched owner with `moth-code-review-guide.md`. Remove old classifier predicates, obsolete role fallback, abandoned root-activity heuristics, parallel per-root module storage, redundant adapters and stale comments.
3. Inspect `source_tree_index`, `module_identity`, `module_namespace`, `module_inventory`, `module_compilation`, `module_artifact_store` and `build.rs` for repeated root classification or duplicated declaration trees. Collapse superficial wrappers and preserve one source of truth per fact.
4. Check user-input-triggerable panics, deterministic ordering, exact spans, namespace collision diagnostics, symlink/physical path invariants, memory growth, dropped temporary preparation state and missing/error outcomes.
5. Update the roadmap/progress status and report each validation command's outcome, any unsupported target, final changed-file inventory and remaining independently queued work (`$export` and `moth test`).

**Gate:** all applicable gates pass or any exceptional blocker is documented precisely, with no claimed completed feature or passed check unsupported by evidence.

## 6. Required regression matrix

Prefer extending the existing primary test owner. The real project integration harness is `tests/cases/`; focused Rust tests belong beside Stage 0/compiler/build owners under their existing `tests/` modules. Match the repository testing guide: every new case should catch a distinct credible failure rather than merely vary filenames.

| Contract | Positive cases | Negative / regression cases |
|---|---|---|
| Role presence | Each single role, each pair and triple, plus no-root directory | Duplicate/alias root claims and unknown `@`/legacy `+` names |
| Module identity | One directory `ModuleId`, one shared helper origin, three role origins | Three jobs for one directory, duplicate helper declarations, unstable shared identity |
| Local visibility | Root sees its own declarations and explicitly bound helpers | `@mod` reading `@entry`, `@package` reading `@mod`, shared helper reading any root-local declaration |
| Export scope | Same helper re-exported through both facades with correct export-binding identity | `export:`/future `$export` in entry or helper, private nominal leakage, bypassing facade |
| Parent/support visibility | Direct child normal API, visible sibling support API, project external package | Importing entry-only module, sibling binding normal facade illegally, same-scope support dependencies, implicit fallback |
| Entry semantics | Runtime, static, empty and metadata-only `@entry`, dormant imported entry | API facade runtime fragments, unmarked activity selecting entry, imported entry execution |
| Command routing | Zero-entry `check`, descendant-only HTML `build`, no-homepage dev fallback | No-entry HTML `build`/`dev`, manufactured homepage, route from root basename |
| Builder and runtime | JS and HTML-Wasm variants, direct root `$in` slots, resource URL contexts, start error path | Publishing runtime fragments after failed start, missing CSS resource from metadata-only entry |
| Bootstrap and single-file | Optional `config.moth`, named records, source `$config`, explicitly selected synthetic entry | Nested config creates module, invalid `@config`, single-file root requires authored `$entry` |
| Test isolation | Test-only support root direction remains documented and separately selectable | Production importing `tests/@package`, test helper accidentally publishes production ABI |
| Determinism and caching | Same source tree yields stable IDs, diagnostics, interfaces and outputs across repeat runs | Case/symlink collision nondeterminism, entry edit changes API fingerprint without exported change |
| Migration and tooling | First-party packages, docs, CLI scaffold, check-only owners and `moth tests` harness | Old `@site` accepted silently, legacy `+` path treated as helper, synthetic package selected as entry |

Use typed diagnostic reason/code assertions with appropriate primary and related sites. Include source-level, graph-level and observable builder/runtime checks where the failure path differs. Source tree performance must remain one traversal per boundary, with no new per-root candidate-copy or repeated tokenization of shared helpers. Retain documented compiler/resource work limits.

## 7. Completion contract and exclusions

This plan is complete when:

- Every authored source directory recognises only the three fixed `@` roles and permits any unique combination.
- `@mod` and `@package` are API-only, `@entry` is entry-only and root-local declarations are mutually invisible, including through ordinary shared helpers.
- Ordinary module implementation files are owned and semantically prepared once, and the structural module publishes all present surfaces atomically.
- Only `@mod` and `@package` can export. Current `export:` remains the temporary operational spelling only until the separate `$export` migration.
- Only `@entry` has an implicit `start` and becomes a builder entry candidate. HTML entries are explicit and no longer inferred from activity.
- `config.moth`, external package isolation, support-package consumer-position rules, tests isolation, synthetic single-file semantics, link validation and JS/Wasm output contracts remain intact.
- The old `$entry`, obsolete root-name spellings, `+` support roots, stale documentation and compatibility code are removed in the same public cutover.
- Focused tests and final repo validation demonstrate those contracts with specific recorded results.

Out of scope: implementing `$export`'s undecided operand/block form, the `moth test` runner, per-module configuration, builder selection from source, arbitrary new root kinds, multiple entries per directory, multiple independently compiled modules for one directory, user-defined visibility rules, source targets selected by filenames and a new module graph/parser/framework. A future feature may add a reserved filename only through a separate accepted design and its own source ownership contract.
