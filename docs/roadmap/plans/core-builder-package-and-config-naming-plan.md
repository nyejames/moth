# Core/Builder package namespaces and config filename implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` to implement this plan phase by phase. Track the `- [ ]` items and complete the review checkpoint after each phase.

**Goal:** Give compiler-shipped and builder-owned packages one predictable namespace and directory layout, require `@package.moth` for registered Moth-source package interfaces and rename the special project configuration file to `@config.moth`.

**Architecture:** Keep Stage 0 source indexing, the existing source/binding package registries, the selected compiler-shipped `packages/` root and the self-contained config compiler as the only owners. Change identities, registration validation and root-file classification at those boundaries. Do not introduce package discovery, forwarding aliases or new config/module semantics.

**Tech stack:** Rust build system and frontend, Moth `.moth`/`.mtf` source, compiler-shipped package assets, CLI scaffolding, Rust unit tests and `tests/cases/`.

**Prerequisite:** The scope-closing semantics and diagnostics overhaul has landed, including its source, example and test migration. Begin from that resulting source tree. This naming plan precedes further sequenced compiler work so the HTML and CSS package surface programme can resume independently afterwards.

## 1. Accepted contract

### Package names and disk layout

The canonical registered imports and compiler-shipped paths are:

- `@html` -> `@core/html`, stored in `packages/core/html/@package.moth`.
- `@styles` -> `@core/css`, stored in `packages/core/css/@package.moth`.
- `@web/site` -> `@builder/web/site`, stored in `packages/builder/web/site/@package.moth`.
- `@web/canvas` -> `@builder/web/canvas`, with its JavaScript asset under `packages/builder/web/canvas/`.
- `@web/graphics` -> `@builder/web/graphics`, with its JavaScript asset under `packages/builder/web/graphics/`.
- `@web/page` -> `@builder/web/page`, with its JavaScript asset under `packages/builder/web/page/`.

Existing canonical Core paths such as `@core/io`, `@core/math` and `@core/text` stay unchanged. `@core` and `@builder` are reserved namespace roots, not directly importable package surfaces. All registered Core packages use `@core/<name...>` and all registered Builder packages use `@builder/<group>/<name...>` or another explicit path below `@builder/`. The web grouping is authored by the HTML builder, not inferred from its selection. Other builders choose their own explicit group and cannot register flat `@foo` or `@builder` packages. ProjectLocal and Dependency packages cannot claim either reserved namespace, including case-only variants. `@project` remains a separate reserved synthetic interface.

The namespace ownership rule applies equally to `PackageBacking::MothSource` and `PackageBacking::ExternalBinding`. Registration rejects empty components, exact reserved-root registrations, invalid origin/prefix combinations and case-insensitive equal or ancestor/descendant overlaps through the existing validation owners. Local structural module/package claims cannot shadow reserved `@core`/`@builder` roots. Unrelated project-local `html/`, `css/`, `styles/` and `web/` modules or support packages remain legal. A local path claiming a complete registered package prefix remains a collision.

Every **registered Moth-source package** publishes its external API through a directly contained `@package.moth`. `@mod.moth` alone is insufficient. A co-located `@mod.moth` may still own a distinct internal normal-position scope, while the public package interface always comes from `@package.moth`. Ordinary project modules retain the existing optional root-role combinations, including normal-only and entry-only modules. Binding-backed packages still have no source facade.

The physical `packages/` directory remains the selected compiler-shipped package set beside the release executable or under the source checkout, using the existing whole-set selection policy. Source-backed package graphs stay independently indexed and compiled with their existing stable identity, origin, backing, provenance and link behaviour. A user copying a folder into a project gains its ordinary support-facade visibility, **not** Core or Builder registration, capabilities or identity. Package installation, dependency acquisition, manifest formats and automatic namespace discovery are outside this plan.

### Config and project globals

`config.moth` becomes exactly `@config.moth`, with immediate cutover and no compatibility search, warning or migration diagnostic. It is an optional build-only configuration file at the selected project root. A nested `@config.moth` stays inert to that enclosing build, as the previous nested config did. Missing configuration still uses validated defaults, while the existing selected-root file, unreadable path and symlink failure contracts remain intact.

`@config.moth` is a reserved compiler filename but **not** a `ModuleRootRole`, ordinary helper, importable module, entry, export surface or project-global namespace. The three authored module roots stay `@mod.moth`, `@package.moth` and `@entry.moth`. The meaning of an `@`-prefixed reserved filename therefore widens beyond module roots. The generic invalid-marker check must recognise this exact exception without accepting arbitrary `@*.moth` filenames or case variants. The old `config.moth` name loses its special status and otherwise follows ordinary source-file rules, including ordinary `@config` path resolution when a distinct `config.moth` helper exists. That path never resolves the build-only `@config.moth`.

Preserve the current self-contained config compiler, allowed private helper constants, `$config` input contracts, `project` record, selected-builder record, bootstrap ordering and validation. Imports, `$export`, functions, runtime work and module-root behaviour remain forbidden inside config. Project config exposes only the compiler-predefined public fields through **`@project`**, as today. Do not rename that namespace to `@config`, expose config helpers, invent global constants or interpret the config file as an empty module anchor. Projects needing a root-level normal API or shared local support topology can use an empty `@mod.moth`.

### Explicit exclusions

- Preserve HTML markup exports, inline CSS template exports, `style` attribute/slot names, `.mtf` prelude symbol names and their collision rules. This plan changes package identities, not package API breadth. A separate package-authoring effort owns full HTML/CSS coverage.
- `@core/html` and `@core/css` remain mandatory `Core/MothSource` providers of the fixed compile-time `.mtf` prelude for **every** builder, including custom SDK builders. They cannot be replaced by Builder registrations or become optional HTML-builder features.
- Preserve existing `PackageOrigin`, `PackageBacking`, stable package identities and direct dependency interfaces. Do not treat filesystem-copy equivalence as identity equivalence.
- Remove old registration names, old package paths and old config special treatment outright. Do not keep aliases, fallback imports, old-file probing, migration diagnostics or temporary compatibility implementations.

## 2. Existing owners and implementation constraints

- `src/projects/settings.rs` owns `CONFIG_FILE_NAME` and `Config::config_file_path`. `src/build_system/project_config.rs` and `src/compiler_frontend/single_source_compilation/config.rs` own bootstrap and config compilation. `src/compiler_frontend/project_globals.rs` owns synthetic `@project`. Leave its contract unchanged.
- `src/compiler_frontend/source_packages/root_file.rs` owns canonical root/config filenames and reserved marker classification. `src/build_system/create_project_modules/source_tree_index.rs` owns the one filesystem traversal and registered package root validation. Do not add a new scan or globally change normal-module root selection.
- `src/builder_surface/package_metadata.rs` owns the shared registration path validator and overlap predicate. `src/builder_surface/source_package_registry.rs`, `src/compiler_frontend/external_packages/registry.rs` and `src/builder_surface/definition.rs` own registration and complete-surface checks. Keep the origin/prefix invariant in the shared validator, not cloned in both registries.
- `src/builder_surface/definition.rs` owns `CORE_SOURCE_PACKAGE_PREFIXES`, `with_mandatory_core`, official root checks and `BUILTIN_PACKAGES_ROOT`. `src/build_system/create_project_modules/source_package_discovery.rs` consumes the selected public facade from the Stage 0 index. `src/projects/html_project/binding_packages/web/mod.rs` owns built-in web binding paths and their JavaScript resource origins.
- `src/build_system/create_project_modules/module_namespace.rs`, `source_tree_index.rs` and the existing package resolution path own project/package namespace conflicts. Update these owners and their tests instead of teaching the frontend or HTML builder a second import resolver.
- `crates/moth-cli/src/scaffold/files.rs`, `src/projects/dev_server/watch.rs`, docs source, test fixtures and benchmark source may embed config/package filenames. Change them alongside their owning phase. Do not use benchmarks as proof of correctness.

Before each phase, search the live tree for the affected names and read the exact relevant canonical sections of `docs/build-system-design.md`, `docs/compiler-design-overview.md`, `docs/developer-docs/language/overview.mtf`, the package/project-structure references and the codebase testing and validation guides. Use failing behaviour-focused tests first and keep the full tree passing at each checkpoint. Reviews should inspect code ownership, duplicate paths and stale references as well as the new contract.

## 3. Implementation phases

### Phase 1 - Rename and isolate project config

**Primary owners:** `src/projects/settings.rs`, `src/compiler_frontend/source_packages/root_file.rs`, `src/build_system/project_config.rs`, `src/build_system/create_project_modules/source_tree_index.rs`, config compiler/service callers, CLI scaffolding and dev-server watches.

- [ ] Write focused failing tests for selected-root `@config.moth`, absent defaults, invalid config contents, nested inert config, ordinary importable `config.moth`, exact-case marker classification, invalid `@Config.moth`, non-regular selected config and external/dangling symlinks. Preserve the existing config-origin span and typed failure behaviour.
- [ ] Rename the single filename constant and update bootstrap reads, direct dependency/file-value exclusions, source tree classification, single-file/root marker checks, watch invalidation and starter generation. The `@config.moth` special case must precede generic invalid-`@` marker rejection without adding `ModuleRootRole::Config`.
- [ ] Migrate authored config files, fixture writers, CLI output assertions, docs-site project config, config test helpers and benchmark setups. Remove old `config.moth` special cases and stale comments. Keep no legacy probing.
- [ ] Run the relevant `src/build_system/tests/create_project_modules_config_tests.rs`, `create_project_modules_structure_collision_tests.rs`, `src/projects/dev_server/tests/watch_tests.rs` and CLI scaffold tests, plus representative `tests/cases/` config and direct-file-reference cases.
- [ ] **Phase review:** Inspect the bootstrap handoff, classification order and the absence of invented module ownership or `@config` publication. Verify `@project` remains identical, nested config is inert and exact invalid markers still diagnose. Fix findings and rerun focused gates before committing.

**Gate:** New config files bootstrap exactly as before under their new name, missing files still default, `@project` is unchanged and the old filename is no longer a compiler special case.

### Phase 2 - Require source-package support facades

**Primary owners:** `src/build_system/create_project_modules/source_tree_index.rs`, `source_package_discovery.rs`, `src/compiler_frontend/source_packages/root_file.rs` and source-package interface tests.

- [ ] Add failing Stage 0 tests for a registered source root containing only `@mod.moth`, only `@entry.moth`, only `@package.moth` and paired `@mod.moth`/`@package.moth`. Test absent package roots, aliased roots, a valid empty support facade and separate normal/support visibility. Assert that ordinary project-root `@mod.moth` remains legal.
- [ ] Require the registered root's direct-child `@package.moth` in the existing package-directory classifier. Update the prepared public-root selection/contract to use that support facade without adding another root scan. Preserve a damaged compiler-shipped set as a typed `InfrastructureFailure` and a malformed third-party package as a typed source/structure diagnostic naming `@package.moth`.
- [ ] Rename `packages/html/@mod.moth`, `packages/styles/@mod.moth` and `packages/web/site/@mod.moth` to `@package.moth` in place for this phase. Review their `$export` declarations and helper dependencies under support-position visibility. Keep nested `@mod.moth` valid where it serves an internal module.
- [ ] Update source-package fixture roots and scoped namespace tests, including `src/build_system/tests/create_project_modules_source_discovery_tests.rs`, `src/build_system/tests/build_dependency_tests.rs`, `src/builder_surface/tests/source_package_registry_tests.rs` and `tests/builder_sdk.rs` as applicable. Exercise external imports and private-helper non-exposure, including a standalone copied project-local support folder.
- [ ] **Phase review:** Verify the rule applies only to registered `MothSource` roots, cannot silently select `@mod.moth`, does not affect binding-backed packages or ordinary modules and preserves stable package graphs and failure lanes. Fix findings and rerun focused gates before committing.

**Gate:** Every registered Moth-source package publishes through `@package.moth` and normal-only registered packages fail explicitly without breaking ordinary module roles.

### Phase 3 - Move HTML and CSS into the Core namespace

**Primary owners:** `src/builder_surface/definition.rs`, Core registrations, `packages/html/`, `packages/styles/`, `.mtf` prelude integration and project namespace resolution.

- [ ] Add failing tests for the canonical `@core/html` and `@core/css` registration paths, mandatory official filesystem roots, required `Core/MothSource` metadata and reserved/non-overridable providers. Verify `@html` and `@styles` are no longer registered. Cover independent root boundaries and exported constants.
- [ ] Move the source trees to `packages/core/html/` and `packages/core/css/` without changing their template exports, HTML attribute slots or CSS helper semantics. Update internal package references, the fixed mandatory-prefix list, installed-root joins and package-set failure messages. Keep whole-set root selection and checksum/resource identities under their existing owners.
- [ ] Update explicit import paths in compiler tests, `.moth` files, `.mtf` sources and doc-site source as needed to keep builds passing. Verify flat prelude constants from both packages still work under the built-in HTML builder and at least one custom SDK builder, with existing shadowing/collision diagnostics.
- [ ] Test project-local `html/`, `styles/` and `css/` modules/support facades alongside Core registrations, including a complete `core/html` namespace collision and a case-only variant. Confirm no accidental registered Core identity is assigned to copied local folders.
- [ ] **Phase review:** Examine fixed-prelude registration, public constant folding, exact official root checks and module namespace collisions. Check for old Core import aliases or redundant root lookup logic, then fix findings and rerun focused gates before committing.

**Gate:** All builders expose exactly `@core/html` and `@core/css` with the same implicit `.mtf` symbols, and ordinary local style-related folder names remain usable.

### Phase 4 - Group Builder packages and enforce namespace ownership

**Primary owners:** `src/projects/html_project/binding_packages/web/mod.rs`, HTML builder registration, `src/builder_surface/package_metadata.rs`, both package registries and builder-surface validation.

- [ ] Add failing registration tests for both backings and all four origins: valid nested `@core/...` for Core, valid `@builder/<explicit group>/...` for Builder, invalid flat Core/Builder names, invalid cross-origin `@core`/`@builder` claims, exact reserved roots, case-only spellings and equal/nested overlap. Include source-versus-binding overlaps and SDK Builder registrations. Reuse the shared validator and existing overlap predicate rather than duplicating checks.
- [ ] Move the source-backed `@web/site` to `packages/builder/web/site/` and register `@builder/web/site`. Move the three binding-backed asset trees to `packages/builder/web/{canvas,graphics,page}/` and register `@builder/web/{canvas,graphics,page}`. Preserve each `PackageBacking`, stable kind and actual exported signatures.
- [ ] Update embedded JavaScript source paths, package-local `README.md` locations, parsed JS module registration, stable resource origin/source identity, generated binding names and runtime emission paths together. Inspect any JS-visible package names embedded in generated calls or bindings, not just Moth import text.
- [ ] Enforce reserved root ownership in both registry types and Stage 0 project-local namespace collision handling, including project folder/package aliases and case-fold comparisons. Keep `@project` handling separate. A root `@core` or `@builder` alone must not resolve as a package. Do not infer `@builder/web` from the active builder.
- [ ] Migrate SDK definition fixtures and end-to-end imports, including `src/builder/tests/definition_tests.rs`, `src/builder_surface/tests/source_package_registry_tests.rs`, `src/projects/html_project/tests/html_project_builder_tests.rs`, builder/module namespace tests and `tests/cases/web_page_unused/input/@entry.moth`. Test runtime JS asset reachability and unused-package non-emission, a copied local `web/` facade and valid custom builder groups such as `@builder/game/...`.
- [ ] **Phase review:** Trace each package path end to end from builder registration through interface binding, stable identity, linking and emitted JS asset. Verify Core/Builder/ProjectLocal/Dependency classification stays distinct, custom builders work and no overbroad filesystem reservation rejects unrelated resources. Fix findings and rerun focused gates before committing.

**Gate:** Core and Builder registration cannot escape their respective namespace roots, and all four web packages compile or link through their grouped canonical names and physical locations.

### Phase 5 - Publish the new contracts and close the migration

- [ ] Update canonical `docs/build-system-design.md` for `@config.moth`, bootstrap ownership, registered source facade requirements and reserved package roots. Update `docs/compiler-design-overview.md` for compiler-reserved filenames versus the three module-root roles, the registered MothSource `@package.moth` rule and the fixed `@project` synthetic interface.
- [ ] Update user-facing project configuration, module/package structure, import namespace and Core/Builder package references under `docs/docs/project-structure/`, `docs/docs/packages/` and `docs/developer-docs/language/` as appropriate. Rename the docs-site Core `styles/` package documentation route to `css/` and fix navigation/page links. Refresh the cheatsheet, starter guidance, compiler-shipped package `README.md` files, package programme living plans and the package/builder progress matrix when its described state changes. Update `index.md` for all moved modules/assets.
- [ ] Search source, tests, fixtures, examples, documentation, snapshots and release/package packaging scripts for `config.moth`, `@html`, `@styles`, `@web/`, `packages/html`, `packages/styles`, `packages/web`, old `@mod.moth` package roots and flat Core/Builder registration patterns. Distinguish legitimate `@mod.moth` project modules and ordinary `style` slots from stale package references. Remove superseded paths, messages and fixture duplicates. Do not hand-edit `docs/_release/**`.
- [ ] Run focused Rust unit tests in the named owners with `cargo test -p moth --lib <filter>`, SDK and CLI crate tests, the relevant `tests/cases/` via `cargo run --quiet -p moth-cli --bin moth -- tests --case <id> --terse`, docs-source checks and the documentation release build described in `validation.mtf`. Run `just validate` at the integrated checkpoint, then `just validate-full` on the final merge-ready tree. Record executed commands, results and known environment/platform gaps. Validate installed-binary package lookup through the existing release-layout test path, not an invented resolver.
- [ ] Inspect for unnecessary compatibility branches, duplicated namespace validators, new source walks, hand-maintained redundant lookup tables, dead exports, unneeded clones and stale comments. Verify the package programme can now grow HTML/CSS APIs without changing shared compiler registration or names again.
- [ ] **Final phase review / Slice review:** Review semantic and diagnostic correctness, cross-stage owners, source/package namespace isolation, `.mtf` prelude consistency, runtime asset integrity, tests, documentation, Rust quality and removal of legacy paths. Resolve every actionable finding and rerun invalidated checks. Update progress evidence and mark affected historical audit coverage stale only under their existing policies. Delete this short-lived plan and its roadmap bullet in the implementation-completion commit, as required by the roadmap lifecycle.

**Final gate:** Only the new canonical config filename and package names remain, the compiler accepts the agreed package-root and namespace contracts, all named tests and documentation are migrated, and the complete final tree passes the appropriate merge-readiness gates. After merging, HTML/CSS package API expansion may resume in parallel with subsequent compiler roadmap items.
