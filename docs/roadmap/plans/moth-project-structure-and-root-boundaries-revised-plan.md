# Project structure and root boundaries

## Status and scope

- **Status:** implemented on branch `project-structure-changes`; selected-directory roots, optional config, hidden source exclusion, hidden outputs, scaffold migration and paired normal/support semantic publication are complete. Final integration audit, full validation and closeout passed.
- **Goal:** compile a selected directory directly, with optional configuration, structural modules that may have separate `@` and `+` interfaces, mechanically excluded `_` directories and safe hidden output roots.
- **Implementation owners:** the existing Stage 0 source index, module identity and graph, module namespaces, compiler semantic compilation, config boundary and build-owned output policy.
- **Follow-ups:** a separate compiler-owned directive/configuration plan and a separate HTML `$entry` migration. Neither directive change belongs in this implementation.

This plan supersedes the conflicting project-root, `entry_root`, root-count and project-facade clauses in the current build and compiler design documents once the corresponding changes land. Implement the new design in the permanent authorities and production code together. Do not leave two functioning models behind.

### Confirmed decisions

1. **One structural module per rooted directory.** One `ModuleId`, one canonical source owner and one semantic compilation transaction, with isolated normal and support root scopes when both exist.
2. **One support-facade model.** A `+` root at the selected project directory is an ordinary support surface and may be assembled as the external package facade. It is not a special root-file kind, regardless of whether `config.moth` or `@` exists.
3. **Optional config, visible scaffold.** Directory compilation accepts a missing `config.moth`. `moth new` always writes a small `config.moth` containing an explicit `project.name`. This is a usability choice, not a compilation requirement.
4. **Unrooted files remain unowned.** Files outside any structural module are not scheduled or assigned an implicit module. Existing check-only semantics for otherwise eligible unreferenced files **inside an owned module** remain intact.
5. **Universal output-name normalization.** Any valid single-component directory output name without an initial `_` gets one, not just the built-in `dev` and `release` names.
6. **Universal structural rules for source packages.** Core, Builder and dependency source-package boundaries use the same directory/root model in their own independent source graphs.

## Authorities and preparation

Read the active worktree's `AGENTS.md`, `docs/developer-docs/style-guide/style-guide.mtf`, its testing and validation companions, the Moth cheatsheet and `moth-code-review-guide.md`. Read `docs/compiler-design-overview.md` and `docs/build-system-design.md` in full because this changes Stage 0, public-interface identity, scheduling and build ownership. Follow the routed references in those authorities, including `docs/compiler-data-layout-design.md` and the project structure, module visibility, package, path and resource reference pages. Read the accepted project/package and HTML entry plans where their contracts meet this change. Use `index.md` only for navigation.

Start from the active `main` commit in a dedicated branch/worktree. Record the revision and run the applicable baseline tests before moving source layouts. Check existing first-party projects, package registries, the HTML builder, CLI scaffold and incremental rebuild behavior. The repository paths under **Implementation anchors** below were checked against `main` during plan review and must be rechecked if they move.

## 1. Selected directory and configuration boundary

A directory command such as `moth check website/engine` selects `website/engine` as both the **build root** and the top of the discoverable source tree. There is no configurable `src/` entry directory beneath it. A nested module can be compiled independently by selecting its directory as a new root.

Only `<selected-root>/config.moth` applies. If absent, the build system and selected builder create validated effective defaults without synthesizing a source file, span or input declaration. If present, this one self-contained compiler-owned config source is compiled through the existing config service. A nested `config.moth` is inert to the enclosing build, including when its directory has modules that the enclosing build otherwise visits. It is a reserved file, not a module helper or import. No configuration is inherited from ancestor directories.

The command still selects the builder, profile, tooling overlays and physical target intent. Builder settings apply to the **whole selected compilation** and are not inherited or overridden by nested modules. Preserve the currently operational config grammar until the separate directive cutover. A missing builder-specific setting can use its declared default. A selected builder that actually requires an explicit setting must issue a targeted diagnostic rather than invent a value.

An independent build of a nested directory has a distinct package and configuration context from the enclosing project. Stable identities and cached artefacts must include the relevant boundary facts. External source packages keep independent identities, config/input namespaces and indices. Explicit single-file compilation remains a separate, synthetic mode and does not search for ancestor configs.

### Project name, version and globals

- Default `project.name` to the exact final component of the selected directory if no explicit name is provided. Validate that spelling against the canonical Moth project identifier rules. Preserve valid case and spelling. For an invalid name, request an explicit valid `project.name` in `config.moth`. The compiler must not silently rewrite the directory name.
- Version is genuinely optional. No config, an omitted version or explicit optional absence produces an unversioned project. No implicit `0.1.0`. The synthetic `@project.version` interface represents absence with the language's option semantics.
- Keep the compiler-owned synthetic `@project` identity and interface independently of any real project-root `+` facade. Project globals are not declarations owned by a source module or a back door into an external package interface.
- Generated project metadata cannot satisfy `$config` or other typed build-input contracts. Only explicit input declarations create those contracts. Preserve existing input typing, provider precedence, fingerprints, diagnostic allowances and fixed-only validation.
- Remove `entry_root` as a user field, in-memory configuration member, public project field and output/HTML routing dependency. An authored legacy field must receive an actionable migration diagnostic. Do not store a redundant `entry_root = "."` or accept it as an alias.

### Scaffold behavior

`moth new` **always generates** `config.moth`, even when the directory name would make a config file unnecessary. Include a minimal explicit project name using the then-current operational syntax, for example:

```moth
project #= (
    name = "my_project",
)
```

Use the real selected scaffold name, not the illustrative name above. Do not emit `entry_root`, an invented version, empty author/license boilerplate or redundant HTML defaults. Add only builder fields that the scaffold actually needs to produce a functional starter. The user may delete the generated file when directory-name defaults suffice. Its presence must not change otherwise identical effective semantics.

The current CLI supports display names such as `my-site` and derives a valid identifier. Preserve a smooth creation flow: put the exact resulting valid identifier explicitly in `config.moth`, make the chosen value visible in scaffold output and allow an explicit user-supplied valid name where the CLI supports one. This **scaffold-time, visible choice** is separate from directory compilation, which never silently sanitizes an inferred name. Reject an invalid explicit identifier or fail with a useful prompt/diagnostic if no valid name can be chosen. Do not generate invalid config.

Generate a root-level `@*.moth` starter instead of `src/@*.moth`. Update `.gitignore` to `_dev` and `_release`, remove old `/dev` generated-output assumptions and ensure the scaffold can run `check`, `dev` and `build` with its emitted config. The existing HTML page metadata remains in the starter until the later `$entry` migration.

### Example tree

```text
website/
  config.moth           # generated by moth new, optional when compiling
  @site.moth            # normal root
  +site.moth            # optional support/public package surface
  helpers.moth          # shared owned source
  notes.moth            # owned only if this directory has a root
  pages/
    @pages.moth
    config.moth         # ignored by a website/ build
  engine/
    @engine.moth
    +api.moth           # stem need not match the normal root
    implementation.moth
  loose/
    helper.moth         # within the enclosing @site structural module
  _prototype/
    @prototype.moth     # excluded unless _prototype/ is selected directly
  _dev/
  _release/
```

`moth check website/` uses only `website/config.moth`. `moth check website/pages/` independently uses `pages/config.moth`. `moth check website/_prototype/` deliberately compiles the selected hidden root but still excludes hidden descendants of that root. The `loose/helper.moth` file has an enclosing `@site` module and retains that module's ordinary ownership and check-only rules. For the distinct unrooted case, consider a selected `workspace/` with `notes.moth` and `pages/@pages.moth` but no root file in `workspace/`: `notes.moth` has no enclosing module and is not scheduled. The nested `pages` module remains eligible.

## 2. One structural module and two isolated surfaces

Every directory with at least one root marker contains **one structural module**. It may have at most one `@*.moth` normal root and at most one `+*.moth` support root. Each marker's stem is cosmetic and does not define the module's identity, package name or pairing. Two `@` or two `+` files in one directory are diagnostics naming both files. A directory with neither marker does not own a module, even if it contains ordinary source.

### Ownership and stable identities

- Assign one dense `ModuleId` per rooted canonical directory **per compilation boundary**, independent of whether the directory has `@`, `+` or both. Graph nodes and compile waves are structural-module-based, not root-file-based.
- Keep one canonical `SourceRecord` and one owning module for each eligible shared helper file. Give helper declarations stable **role-neutral** module origins. Reaching a helper from both root surfaces must not duplicate parsing/semantic ownership, HIR, generated bodies, borrow facts or diagnostics.
- Give root-private declarations and exported declarations on each root **role-qualified** stable origin identities. A normal root's public `value` and a support root's public `value` can coexist because their defining roots differ. An exported alias of a shared helper retains the helper's defining semantic identity, with the selected public surface recorded separately where required.
- Compile both root scopes and the shared helpers in one semantic module transaction. Shared declarations are registered once. Each root has its own isolated declaration/visibility environment and its own explicit export projection. Do not merge root scopes or give one priority over the other.
- Publish the normal and support interfaces as distinct immutable outputs only from a successful module transaction. Any diagnostic that invalidates the structural module prevents publication of **both** surfaces. Consumers must never observe a partially published paired module.
- Stable fingerprints, generated-function identities and incremental invalidation must reflect shared helper changes and the affected public surfaces without incorrectly depending on cosmetic root filenames. Never use `ModuleId` as a persistent identity.

These are invariants, not a request to create a parallel compiler pipeline. Extend the existing module preparation and public-interface owners to accept the two role scopes and single shared source set. Prefer a compact structural record holding optional root-file/surface facts over parallel `ModuleIdentityRecord` values pretending to be independent modules.

### Local dependency and execution rules

- Shared helper files can import other owned helper files through existing same-module dependency rules. They cannot access, import or re-export either local root's declarations, whether private or public.
- The normal and support roots can each depend on shared helpers. They cannot access or import one another, including via direct root-file paths, alias paths, package lookup or re-export. A root's public surface is not implicitly added to the structural module's local namespace.
- Only the `@` root can own top-level runtime activity, direct output templates and dormant start/entry bodies. The `+` root is API-only. An ordinary helper does not gain a standalone runtime entry.
- Detect name collisions **within a genuinely shared declaration scope** as usual. Identically named declarations local to different isolated roots are allowed and remain distinct. No implicit shadowing or root-symbol precedence.

### Consumer-position-based visibility

Consider this structure:

```text
app/
  @app.moth
  engine/
    @engine.moth
    +engine.moth
    shared.moth
    internal/
      @internal.moth
  ui/
    @ui.moth
    panel/
      @panel.moth
```

The parent `@app` can import the **normal** `engine` interface, never its paired support interface. A normal consumer in the sibling `ui` branch and its eligible descendants can import the **support** `engine` interface under the existing support package rules, never the private normal child interface. Within `engine`, the paired `@engine`, `+engine`, shared helpers and private descendants cannot import `+engine` as a package or see its exports implicitly.

The `+engine` root may depend on permitted shared helpers and private descendants. Its dependency edges must still obey existing support rules, including no import of lateral consumer branches or same-owner support siblings and no path through a descendant back into its own interface. Reject direct and indirect cycles through the **existing graph cycle validator** after role-specific legality is checked. Import path resolution selects an explicit permitted surface by consumer position, not by fallback or search order. Preserve existing case-collision and unrelated package-claim diagnostics.

An **unpaired** `+` module remains an API-only source package with its existing parent-visible support behavior. The paired-root parent exclusion does not silently change standalone support-package imports. A `+` without a local `@` must continue working.

## 3. Unify the project facade and external source packages

A real `+` at the **selected build root** is an ordinary support surface over that structural module. Its eligibility as an external package facade is determined by existing package assembly, public-interface and provenance rules, **not** by adjacency to `config.moth`, a special graph node or a third `ModuleRootRole`. It works when there is no config and whether or not the same directory has an `@` root. Internal project modules must not acquire privileged access to the external facade. Its public contract cannot leak compiler-only `@project` or other project-private provenance.

Preserve the distinct synthetic `@project` service identity. The current `ProjectPackageFacade` role is used in both filesystem classification and synthetic project-context identity. Remove its filesystem/graph meaning and represent synthetic identity in its owning semantic domain rather than retaining a fake physical facade solely to construct `@project`.

Apply the same structural module/root-count/role-scoping model to separately indexed **Core, Builder and dependency source packages**. Each package keeps its own `SourceTreeIndex`, package identity, source ownership and graph. No project input/config namespace crosses this boundary. The package's public root is selected **once by Stage 0 package-boundary metadata**: use the root `+` interface when one exists, otherwise preserve the valid existing normal-root-only package surface. A support-only package root is valid and API-only. In a paired package root, the `@` root remains its internal/normal interface and external consumers receive the declared `+` public surface. This boundary selection is not an import-time fallback.

Reject invalid package-root configurations with a typed diagnostic, not the current single-root `expect` assumption. Reuse the common module identity, namespace and visibility code for projects and packages, with only the true boundary/prefix and publication differences remaining.

## 4. Unrooted source files and source ownership

Stage 0 inventories recognized source files as needed, but classifies files outside every rooted module as **unowned**. Do not schedule, compile or expose those files via an implicit module, even for `check`. In an owned module, preserve the existing check-only path for otherwise eligible unreferenced source files and retain their normal owner identity. A file in an unrooted subdirectory may still be owned by its nearest **enclosing** rooted module. Ownership follows the canonical nearest-enclosing-root topology rather than requiring a root in the file's immediate directory.

An explicit import that would cross a structural boundary, target an unowned file, access an illegal role surface or import a special root/config file must produce the corresponding structured diagnostic. No resolver fallback should manufacture a module or admit the file merely because the filesystem path exists. If an otherwise valid selected tree contains no eligible modules, report an accurate no-source/no-artefact outcome according to the command rather than silently inventing one.

## 5. Hidden directories and path access

A directory component **below** the selected build root whose final filename starts with ASCII `_` is excluded from the entire compilation. This applies to normal and support roots, helper files, `.mtf`, resources, check-only inventory, external providers, dependency resolution, incremental watches and other discovered inputs. `_`, `_prototype` and `__internal` are hidden. `my_dir` is not. A file named `_helpers.moth` in a visible directory remains eligible.

An explicitly selected build root is exempt from the hiding rule **for that root component only**. A separately targeted `_prototype` may compile but its own nested `_` subdirectories remain hidden. No enclosing project can import into the excluded root as a back door.

- Reject explicit imports, tracked file values and resources that traverse a hidden directory. Keep the original spelling/span for a useful exclusion diagnostic without adding a source, dependency or watcher edge.
- Check both lexical path components and canonical ancestry. A visible symlink alias cannot reveal a hidden physical subtree. Existing containment rules continue to reject aliases outside the selected package/build boundary. Handle non-existing targets with existing validated lexical ancestry rather than silently probing alternative candidates.
- Apply the same effective directory-exclusion facts before compiler source preparation, provider discovery, orphan inventory and dev watcher registration. Watcher updates must not reintroduce excluded content after a filesystem change.
- Retain independently justified non-source skips such as `.git`, `target`, `node_modules` and `.cache`. Once generated folders are migrated, stop skipping visible `dev`, `release`, `dist` and `build` by name. Those may contain legitimate source.

Own the inclusion/exclusion decision in Stage 0's existing `SourceTreeSkipPolicy` and canonical source index. Feed consumers its validated facts. Do not implement another hidden-source resolver in the compiler frontend or a generic filesystem policy abstraction for this one rule.

## 6. Output naming, validation and ownership

Default directory outputs are `_dev` and `_release` under the selected build root, unless the selected builder's validated policy specifies otherwise. **Every directory output root must be hidden from the selected source tree**. All builders use the same output-name rule:

- If the configured output path is exactly one valid relative directory component, prefix `_` once when it does not already start with `_`: `dev` -> `_dev`, `dist` -> `_dist`, `artifacts` -> `_artifacts`, `_cache` -> `_cache`. This must be idempotent and performed once in the output owner.
- If an explicit relative output path has several components, preserve its spelling but require an underscore-prefixed **directory** component that encloses the emitted root: `_generated/site` and `generated/_site` are valid. `generated/site` is invalid with an actionable diagnostic.
- Reject absolute paths where not authorized, `..` escapes, invalid components, paths resolving to the build root and paths whose canonical or lexical ancestry would expose output as visible source. Validate symlink aliases, root overlaps and cross-profile/builder conflicts. Normalize **before** containment and manifest checks.

The selected output roots now legitimately lie **inside** the build/source root as hidden subtrees. Revise the old containment rule that rejected all output under `entry_root`. Reject true source/output overlap and dangerous aliasing, not ordinary hidden output containment. Validate selected output paths early enough that Stage 0 never ingests generated content. Preserve established guarantees for validation before writing output bytes and for directory creation and symlink races.

The `_` prefix only controls source discovery. It grants **no right to overwrite or delete files**. The output manifest, `OutputOwner` builder/profile identity, managed extensions, validated output records, conflict detection and stale cleanup remain the sole authorities for output ownership. Do not treat pre-existing unowned files or another owner's manifest as disposable. Preserve necessary legacy-manifest **read** compatibility during migration without extending old output placement behavior indefinitely.

A nested independent project may emit `_dev` inside its own directory. Its parent compilation excludes those generated files mechanically without reading the nested config or adopting the nested output manifest. Update HTML route calculation and document/resource path ownership to be relative to the selected directory, without accidentally changing intended published URLs.

## 7. Command outcomes and entry activation

- **`check`:** validates discovered rooted normal and support modules, their reachable dependencies and eligible check-only owned sources. It does not require config, runtime entry or artifact emission. A support-only/API-only tree is valid. Unowned source files are not scheduled.
- **`build`:** requires a builder-supported artefact selection, such as an activated normal root or explicitly requested package output. A support root alone does not create a runtime entry, but package assembly remains possible when the selected builder supports it.
- **`dev`:** requires a usable runtime entry for the chosen builder, which may be an eligible descendant normal root. A support-only tree or a dormant-only tree without an active entry gets a targeted no-entry diagnostic.

Entry activation, source-tree root, module directory identity, public interface and output route remain different facts. Keep current HTML `$page`/page metadata behavior working until its own approved cutover. Do not implement `$entry`, `$project`, `$builder`, `$config` or `$module` here.

## 8. Implementation anchors and removal work

These paths are grounded in the `main` repository audit and are navigation, not an instruction to preserve the current data layout.

### Stage 0 discovery, identity and graph

- `src/build_system/create_project_modules/source_tree_index.rs` currently stores root-file identities, treats mixed `@`/`+` as an error, has a `discover_project_package_facade` path and hard-skips `dev`, `release`, `dist` and `build`. **Replace** this with one structural module per rooted directory, optional root-role fields and the common hidden-directory exclusion. Delete project-facade discovery and the special registration/owned-source paths rather than maintaining them beside the new index.
- `src/build_system/create_project_modules/module_identity.rs` currently represents a root file as `ModuleIdentityRecord` with a role-qualified `StableModuleOriginIdentity`. Its `ModuleId` and root lookup tables are root-oriented. **Separate** role-neutral structural/helper origin from role-specific public/root origin. Have directory lookups return the permitted interface target or structural owner explicitly. Eliminate facade-specific sorting and directory special cases.
- `src/build_system/create_project_modules/project_module_graph.rs` currently creates nodes and compile waves per root identity and gives the facade a separate node/entry path. **Rebase** graph nodes on structural `ModuleId`, keep one existing provider-before-consumer/cycle validator and record root-surface legality before adding edges. Keep the existing common support-visibility function and its minimal topology views rather than creating another visibility policy framework.
- `src/compiler_frontend/semantic_identity.rs` defines `ModuleRootRole::{Normal, Support, ProjectPackageFacade}` and currently uses the facade role for synthetic `@project` context. **Remove** the physical facade role and preserve synthetic context with a distinct compiler-owned identity. Update exported declaration and generated-function identities as required for shared helpers and independent root surfaces.

### Namespace, packages and semantic compilation

- `src/build_system/create_project_modules/module_namespace.rs` currently maps `NamespaceEntry::CrossModule { target_module_id }` without identifying the chosen surface. **Resolve the allowed normal/support surface when Stage 0 builds namespace entries**, not through frontend fallback. Consolidate the near-duplicate project/package namespace loops and cross-module target insertion only where the common behavior is real. Keep project/package boundary distinctions explicit.
- `src/build_system/create_project_modules/source_package_discovery.rs` currently projects one required package root file and assumes it exists. **Project the single selected package public surface** from structural root facts, including paired and support-only roots. Use structured invalid-package-root diagnostics instead of an unchecked assumption for user-provided package structures.
- The current module preparation, semantic compiler and public-interface publisher expect one root-owned module. **Extend them in place** for shared helpers and two isolated root declaration scopes. Ensure one compiler transaction, one HIR owner and two interface projections rather than compiling each root and attempting to merge the results. Update the generated-function and fingerprint handoffs that embed module origins.
- `src/compiler_frontend/source_packages/root_file.rs` retains the `@`/`+` marker classifier. **Reuse it**, changing only what structural indexing requires. Do not add a second marker scanner or pair root filenames by stem.

### Configuration, output, routing and CLI

- `src/build_system/create_project_modules/config_boundary.rs`, `src/build_system/project_config/validation.rs` and `src/projects/settings.rs`: **remove** `entry_root` and legacy required config/version assumptions. Compute directory-name defaults once, preserve optional-version types and keep `@project` independent of physical facade code.
- `src/build_system/output/output_roots.rs`, `src/build_system/output/policy.rs`, `src/build_system/output/output_path.rs` and `src/build_system/output/manifest.rs`: **centralize** single-component underscore normalization, hidden-root validation and output ownership. Delete the obsolete `entry_root` containment veto but retain manifest safety, profile conflicts and existing reader compatibility.
- `src/projects/html_project/output_plan.rs`: **rebase** relative HTML routing on selected build root. Verify URL equivalence in migrated fixtures rather than silently changing site routes.
- `crates/moth-cli/src/scaffold/start_page_scaffolding.rs` and `crates/moth-cli/src/scaffold/target.rs`: **replace** `src/@page.moth`, verbose default config and `/dev` ignore paths. Always generate a small `config.moth` with explicit valid name. Preserve an understandable named scaffold flow for paths like `my-site`.

### Code size and ownership guardrails

1. **Remove**, rather than rename, the project-level facade discovery, distinct graph node, private lookup exceptions and duplicate root ownership records.
2. **Merge** the repeated project/package namespace population only after encoding the real boundary differences. Prefer one shared function in the existing subsystem over additional traits or config objects.
3. **Reuse** the existing source-tree skip policy and output validator as separate owners. Pass validated facts between them. A new monolithic path-policy layer would blur filesystem discovery and output safety.
4. **Delete** `entry_root` consumers and stale source-path transforms at the cutover. Do not keep an internal field whose only meaning is the build root or a compatibility scanner for `src/`.
5. **Keep local** narrow builder-specific artifact and routing behavior. It does not become shared merely because output naming is common.
6. **Inspect** source inventory copying and compile job assembly while moving to structural modules. Delete redundant vectors/reconstruction rather than adding a second per-surface copy of every helper.

## 9. Implementation sequence

Each phase must end in a coherent, testable state. Group mechanically inseparable changes in a single mergeable slice. Record unexpected contract conflicts before widening scope, and update permanent design documentation alongside the code that implements the new rule.

### Phase 0 - Baseline and authority update

- Record the exact `main` revision, important first-party layouts, source-package registration and current `check`, `build`, `dev` and scaffold results.
- Read the required authorities. Locate all `entry_root`, required config/version, root-file `ModuleId`, facade exceptions, output/manifest placement, hardcoded folder skip, resource watcher and old scaffold references.
- Update the affected architectural invariants in the compiler/build authorities, distinguishing accepted design from current implementation status. Record the external/public migration changes.
- Choose one compact integration fixture that exercises a paired root, nested config, unrooted file, hidden source, custom output and external package consumer. Prefer extending current scenario owners.

### Phase 1 - Hidden paths and safe hidden output roots

- Add component and canonical-ancestry `_` exclusion to existing Stage 0 source policy. Apply it to explicit imports, providers, file values/resources, orphan selection and dev watch facts.
- Add typed hidden-path diagnostics. Test path aliases and boundary escapes without admitting hidden sources.
- Normalize and validate all directory output roots in the existing output owner. Allow safe hidden outputs inside the selected source tree while preserving manifest ownership and conflict checks.
- Switch defaults to `_dev` and `_release`, update relevant fixtures and remove legacy visible-output hard skips once all paths are migrated.
- Preserve check/build functionality and verify output files are not discovered on repeated builds.

### Phase 2 - Directory root, optional config and scaffold

- Make selected directory the only directory build/source root. Migrate HTML routes, output paths, CLI commands, fixtures and first-party projects from `src/` assumptions.
- Support absent config and validate computed directory name and optional version. Keep present empty/minimal configs legal. Publish consistent `@project` defaults without a source span or input contract.
- Remove `entry_root` across the operational pipeline, with targeted diagnostics for authored legacy fields. Remove duplicate settings and config lookup paths at this cutover.
- Always scaffold a minimal name-bearing `config.moth`, a root-level normal module and hidden output ignore rules. Validate fresh project `check`, `dev` and `build`.
- Keep nested configs inert to parent builds and active only for explicitly selected nested roots.

### Phase 3 - Structural module and isolated root surfaces

- Make Stage 0 assign one `ModuleId` and one shared owned source set per rooted directory with optional normal/support root facts. Reject duplicate same-role roots.
- Adapt stable origins, compilation preparation, root-scoped declaration registration and public-interface projection so helpers compile once and `@`/`+` scopes stay isolated.
- Adapt namespace entries, imported surface selection, graph dependencies and cycle validation to the consumer-position contract. Preserve standalone `+` behavior and forbid mutual paired-root imports.
- Unify project-root `+` with ordinary support-root discovery. Remove `ProjectPackageFacade` filesystem/graph treatment while retaining synthetic project globals.
- Apply the same structural rules and single public-surface selection to independently indexed Core, Builder and dependency source packages. Remove single-root assumptions.
- Demonstrate one shared helper semantic identity, one HIR/body and one set of diagnostics under paired imports, including invalid-module atomic publication.

### Phase 4 - Migration, audits and handoff

- Update public project-structure, module/package visibility, project config, build inputs, path/resource, HTML routing and package facade references. Keep final design authorities authoritative, and update progress only for completed behavior.
- Migrate first-party examples, docs site, CI/benchmark commands, tests and user-visible CLI/help text. Verify route and package name changes deliberately.
- Audit for duplicate source scans, root-file oriented graph leftovers, facade special cases, stale `entry_root`, `src/` scaffolds, old `/dev` ignores, output-prefix duplication and unnecessary context structs/traits.
- Update the separate compiler-owned directive/config plan and HTML `$entry` plan to consume the final boundary model. Do not implement them here.
- Run the selected final validation gate from the repository testing/validation guide and record the evidence in the implementation handoff.

## 10. Required regression coverage

Extend existing test owners before creating new ones. Prefer realistic project integration tests for observable behavior and focused Rust unit tests for identity, namespace, path and diagnostic invariants. Tests must assert success or typed failure and, where relevant, structural facts rather than merely the absence of a crash.

### Compilation boundary, config and scaffold

- No config, minimal scaffold config, empty valid config, malformed config and a builder requiring an explicit setting. No phantom spans, imports or watch dependencies.
- Valid and invalid selected-directory names, exact spelling preservation, explicit name overrides and current `my-site` scaffold naming. A generated config always contains an explicit, valid name and no invented version.
- Optional version absent, present and explicitly absent, including correctly typed `@project.version` and no synthesized `0.1.0`.
- Nested config ignored by parent and active when nested directory is selected. Config and input scope never cross project or source-package boundaries.
- Computed project globals never satisfy same-named explicit input contracts.
- Retired `entry_root` diagnosed without falling back to `src/`. Synthetic single-file compilation remains distinct.
- Root-level scaffold can `check`, `dev` and `build`. Deleting config from a valid-name scaffold preserves its effective project identity and build behavior.

### Module identities, helpers and public interfaces

- Zero roots leaves recognized files unowned and unscheduled. One normal root, one support root and a paired root are legal. Duplicate same-role roots diagnose both files. Cosmetic filenames do not change module identity.
- Paired root can re-export a shared helper from both surfaces, with **one** owner, stable helper declaration identity, semantic compilation, HIR representation and diagnostic emission. Confirm no duplicate generated functions or borrow state.
- Root scopes may independently declare/export the same name. Shared-local collisions remain errors. Helpers and paired roots cannot import or access either root's declarations in prohibited directions.
- Parent imports paired normal, sibling imports paired support and eligible descendant sibling imports support. Private descendants, owner scope and the paired normal root cannot import that support interface.
- Unpaired `+` preserves its existing parent-visible package behavior. Support roots remain API-only, including at the selected build root.
- Support-root dependency restrictions prevent direct and indirect cycles, without hanging or publishing a partial interface. One root's diagnostic prevents both interfaces being published from the failed structural module.
- Source-package root `@`, `+` and paired forms select exactly one external surface. Core/Builder/dependency boundaries never leak their identifiers or input values into a consumer project.
- Stable public identities, path-case collisions, fingerprints and repeat/parallel build ordering remain deterministic.

### Excluded paths and outputs

- `_`, `_prototype`, `__internal` and nested hidden directories excluded. Ordinary filenames `_helper.moth` remain eligible and visible `dev`, `release`, `dist`, `build` may contain source.
- Explicit selection of hidden build root works. Hidden descendants remain excluded. Enclosing builds cannot import its files or accidentally watch it.
- Imports, tracked resources, file providers, lexical `..` paths and symlink aliases cannot reveal an excluded or out-of-boundary subtree. Diagnostics retain useful path/span context but add no graph/source edges.
- Single-component outputs normalize exactly once for arbitrary names and are idempotent. Multicomponent paths require an enclosing `_` segment. Path escapes and visible source aliasing fail.
- Nested hidden output is accepted. Build-root/output equivalence, incompatible profile roots, foreign manifests, pre-existing unowned files and cross-builder ownership cannot be overwritten or cleaned up.
- Generated `.moth`, `.mtf`, `.md`, HTML, JS, Wasm and MON files do not appear as sources/resources on cold discovery or dev rebuild. A nested build's output does not contaminate a parent build.

### Commands and real projects

- `check` accepts API-only or dormant-only projects without entry. `build` honors selected builder artefact capabilities. `dev` diagnoses a genuine no-entry project and accepts eligible descendant entries.
- Project root `@` with external `+` assembles a public package without duplicate module compilation or leaking project-private facts. A support-only external package facade works without config.
- Moved first-party docs and example projects preserve intentional HTML routes, assets, package imports and generated output ownership. Validate a repeated incremental build and semantic cache reuse.
- Platform-specific path cases, deterministic diagnostics and stable output ordering receive focused regression assertions where supported.

## 11. Completion and deferred work

The migration is complete when:

1. A selected directory compiles without `src/`, `entry_root` or authored `config.moth`, while scaffolding still visibly creates `config.moth` with an explicit valid name.
2. Nested directories compile independently, nested configs do not inherit, unrooted files stay unscheduled and hidden subtrees are unreachable from enclosing builds.
3. A rooted directory supports separate `@` and `+` interfaces through one semantic module compilation, with no duplicated helper identities, unauthorized visibility or partially published interfaces.
4. Build-root `+` and source-package `+` use the same support surface rules, without a special physical facade role or bypass of package provenance validation.
5. Output names are consistently hidden, safe within the source root and governed by the existing manifest/conflict owners. First-party projects, CLI and docs have migrated.
6. Applicable validation passes and the code-review audit finds no redundant old discovery, compiler or configuration path retained solely for compatibility.

**Deferred:** compiler-owned `$project`, `$builder` and `$config` syntax, builder-defined directive removal, `$module`, `$entry` and HTML page metadata migration, module-local config, feature-selection changes, package publication/version policy, new backend output semantics and unrelated template/language features. The directive plan must accept optional config and version and no `entry_root`. The HTML entry plan must use the new structural root model and retain the rule that only normal roots can own entries.
