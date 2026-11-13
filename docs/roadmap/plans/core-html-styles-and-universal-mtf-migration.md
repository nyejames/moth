# Core HTML, Styles and Universal MTF Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `@html` and `@styles` universal Core/MothSource packages and make `.mtf` a compiler-supported source kind under every builder, with a collision-free combined template helper prelude.

**Architecture:** Keep source-package identity and resolution in the existing `SourcePackageRegistry` and source-provider compilation pipeline. Register official pure-source Core roots once in the common builder surface, keep builder-only packages such as `@web/*` separate and make `.mtf` recognition independent of builder opt-ins. The `.mtf` binding path consumes the two normal Core package interfaces without synthesising another source package or adding general shadowing rules.

**Tech Stack:** Rust, Moth source packages, `moth tests`, Cargo and the HTML/SDK builders.

**Spec:** `docs/compiler-design-overview.md` (source-kind adapters and binding) and `docs/build-system-design.md` (Core/Builder source packages, capability surface and Stage 0). This plan records the accepted supersession of their old builder-only HTML/MTF policy until those authorities are updated.

## Global constraints

- `Core` is package origin, `MothSource` is backing. There is no `Standard` origin and no special package implementation path.
- `@html` and `@styles` retain their short import names, use the official `packages/` roots and are available in all compiler projects, including custom SDK builders.
- `.mtf` has exactly the same compiler-owned syntax, constant `content` contract, dependency semantics and diagnostics under all builders. Plain `.md` remains an optional builder-selected kind.
- `.mtf` implicitly sees only public compile-time constants exported by official `@html` and `@styles`. Runtime symbols, imported aliases and project-specific packages never enter that prelude.
- Duplicate exported prelude names are invalid, irrespective of whether a particular `.mtf` file uses either name. Report both package names. Normal authored visible-name collisions retain the existing diagnostic mechanism.
- The user-facing `@html` package owns HTML markup and slots. `@styles` owns `[$in style:...]` fragments. Neither owns runtime canvas wrappers, DOM bindings, Tailwind-style class generation nor component systems.
- Remove obsolete APIs and tests rather than retaining re-export shims. Do not edit generated `docs/_release` content by hand.
- Validate on the current Rust toolchain specified by the repo, using the test and validation guides. Record actual commands and failures, not inferred success.

## Review focus

1. A minimal SDK/native builder with no HTML capabilities must import `@html` and `@styles` without loading `@web/canvas`. Test in Task 2.
2. `.mtf` must work when a builder declares no extra source kinds, including nested files and direct qualified references. Test in Task 3.
3. Two official prelude packages exporting the same constant must fail deterministically with package-specific evidence. Test in Task 4.
4. An unrelated project-local `@styles` root must not silently replace the official package, including case-only variants. Test in Task 5.
5. Runtime canvas assets must still be omitted when unused and remain reachable through explicit `@web/canvas` calls. Test in Task 6.

## Current branch checkpoint

The draft foundation branch has already split the six style exports, removed `@html` canvas runtime wrappers, registered both official source roots as Core in `BuilderSurface::with_mandatory_core`, renamed the docs support package and migrated direct callers in selected docs and tests. It updates the design authorities to the accepted final contract. These edits have only static checks because the environment used to prepare this branch has no Rust toolchain. **They are unverified and must be tested and corrected, not assumed complete.**

The outstanding critical path is universal `.mtf` source-kind support and the two-provider flat constant prelude, followed by complete integration migration. The following tasks are verification and implementation obligations even where some code is already present. The agent should inspect the diff, complete each task and close the PR only after validation. Remove this plan and its roadmap entry in the completion commit after the durable contracts are confirmed in the design authorities and package documentation.

## Task 1: Separate package responsibilities and docs namespace

**Files:** `packages/html/@mod.moth`, new `packages/styles/@mod.moth`, `docs/styles/` to `docs/docs_styles/`, relevant `docs/**`, `tests/cases/**`.

**Interfaces:** `@html` retains all current markup helpers including the `<canvas>` element. `@styles` exports exactly `center`, `right`, `left`, `smaller`, `medium` and `larger` with their existing inline declaration strings and the existing `style` slot contract.

- [ ] Move only the six `[$in style:...]` constants into the new Core source facade. Delete their old declarations rather than forwarding them.
- [ ] Rename the docs project's local `@styles` support package to `@docs_styles`, updating real imports and file-resource paths. Distinguish executable references from illustrative code snippets and unrelated integration fixtures with their own local `@styles`.
- [ ] Migrate actual imports of the six helpers to `@styles`. Update package docs to name the new owner and explain slot composition. Keep broader HTML semantic hardening and new utility exports out of this slice.
- [ ] Search for all six old exports and `@styles` imports. Assert no production use of `@html`'s removed styling exports remains. Check changes with `git diff --check`.
- [ ] Commit this slice, recording if runtime validation is not available.

## Task 2: Register universal Core/MothSource packages

**Files:** `src/builder_surface/definition.rs`, `src/builder_surface/tests/source_package_registry_tests.rs`, `src/projects/html_project/html_project_builder.rs`, relevant SDK tests, `docs/build-system-design.md`.

**Interfaces:** Keep `SourcePackageRegistry::register_filesystem_root(prefix, root, PackageOrigin::Core)`. Add the two official source roots to the common builder-surface constructor. `@web/site` and binding-backed `@web/canvas` remain HTML-builder selections. No directory scanning or second package registry.

- [ ] Add or extend a focused failing registry test proving a fresh `BuilderSurface::with_mandatory_core()` contains official `html` and `styles` roots with `Core` origin and `MothSource` backing, and that SDK/HTML builders preserve these identities.
- [ ] Run the targeted `cargo test -p moth --lib source_package_registry` filter and confirm the expected failure.
- [ ] Move the official source-root registration into the common constructor and remove the duplicate `@html` registration from `HtmlProjectBuilder::frontend_surface`. Keep a single validation owner for fixed official paths and reserved root prefixes.
- [ ] Verify registered Core paths cannot be replaced or overlapped by builder or project-local packages. Update tests that assumed an initially empty Core source registry.
- [ ] Check package-origin identity changes against stable source-provider IDs, frontend fingerprints, single-file compilation and cached interface reuse. They must now identify the two roots as Core/MothSource on every builder.
- [ ] Run the focused test and `cargo check -p moth`. Commit when passing.

## Task 3: Make MTF an unconditional compiler source kind

**Files:** `src/builder_surface/source_file_kind_registry.rs`, `src/builder_surface/definition.rs`, `src/builder/definition.rs`, `src/projects/html_project/html_project_builder.rs`, source-kind tests under `src/builder_surface/tests/` and `src/build_system/tests/`, `docs/compiler-design-overview.md`.

**Interfaces:** `SourceFileKind::MothTemplate` is a fixed compiler-recognised, supported kind like `.moth`. Builders may still register additional kinds such as `PlainMarkdown`; they cannot disable, reassign or substitute `.mtf`.

- [ ] Extend an SDK/native builder integration test with a `.mtf` file value, nested `.mtf` import and `content` constant. Confirm it currently fails solely because `.mtf` is unavailable under that builder.
- [ ] Reuse the canonical compiler-owned source-kind classification. Remove HTML-builder-only `.mtf` registration and unsupported-kind policy for `.mtf`. Keep the existing Markdown builder opt-in untouched.
- [ ] Update source-kind registry tests and fixtures that manufactured `.mtf` capability states. Preserve genuine tests for duplicate extension names, wrong-case paths, source-kind collisions and unsupported `.md`.
- [ ] Run `cargo test -p moth --lib source_file_kind_registry`, the selected SDK tests and `moth tests` filtered to `.mtf` scenarios. Commit when passing.

## Task 4: Combine HTML and Styles into the single MTF constant prelude

**Files:** `src/compiler_frontend/public_interface/dependency_bindings.rs`, `src/compiler_frontend/headers/binding_environment/builder.rs`, `src/build_system/create_project_modules/compilation.rs`, `src/build_system/create_project_modules/compilation/canonical.rs`, `src/build_system/create_project_modules/compilation/single_file.rs`, `src/compiler_frontend/public_interface/tests/dependency_bindings_tests.rs`, `src/compiler_frontend/headers/tests/moth_template_prepare_tests.rs`, `src/build_system/tests/create_project_modules_package_registry_tests.rs`.

**Interfaces:** Retain ordinary completed `PublicSemanticInterface`s for both official Core roots. The internal template-prelude dependency input carries both identities. A single visible-name registry consumes their exported constant bindings and rejects collisions. No synthetic package facade, builder callback, second import resolver or template-specific AST construction.

- [ ] Add failing tests for automatic bare `p` and `center` in one `.mtf` file, a private Styles constant remaining invisible, and a runtime or type export remaining unavailable.
- [ ] Add a failing collision test with the same exported constant in both official providers. Require deterministic diagnostics naming both origins and reject before consumer body evaluation.
- [ ] Extend the existing `TemplatePrelude` dependency input from one provider to the two official sources while preserving existing canonical provider ordering, interface ownership and package-graph dependency edges. Reuse one normal public-export reader for both providers rather than duplicating `collect_html_template_prelude_constants`.
- [ ] Ensure both source packages precede `.mtf` consumers in directory and single-file compilation, including check-only compilation. Eliminate single-prefix assumptions in ordering, materialisation and the compiler-standard prelude validation.
- [ ] Run focused public-interface, header-binding and package-order tests, then integration `.mtf` fixtures. Commit when passing.
- [ ] Ensure a `.mtf` source importing ordinary project data has the same binding, errors and output under HTML and minimal SDK/native builders. Avoid importing HTML-only runtime capabilities during Core source-package materialisation.

## Task 5: Verify package namespace and facade isolation

**Files:** `src/builder_surface/tests/source_package_registry_tests.rs`, `src/builder/tests/definition_tests.rs`, integration fixtures in `tests/cases/` and the relevant path-resolution tests.

- [ ] Verify `@html` and `@styles` are independently importable by both HTML and SDK/native builders without `@web/*` registrations.
- [ ] Test exact and case-only `@styles` conflicts against registered Core roots. Expect the existing namespace conflict policy, not local precedence or hidden fallback.
- [ ] Verify the docs support facade `@docs_styles` remains local to the docs project and that its exported names never enter the compiler `.mtf` prelude.
- [ ] Confirm source-owned `@html` and `@styles` modules have correct Core/MothSource identities and never activate an entry or emit runtime artefacts just from import.
- [ ] Run targeted path/builder tests. Commit when passing.

## Task 6: Remove canvas convenience code without losing runtime semantics

**Files:** `packages/html/@mod.moth`, canvas cases under `tests/cases/html_canvas_helper_*`, `src/build_system/tests/build_dependency_tests.rs`, `src/compiler_tests/integration_test_runner/tests/execution.rs`, `docs/docs/packages/builder/html/*` and canvas documentation.

- [ ] Remove `Canvas`, `get_canvas`, `get_canvas_context` and method wrappers in `@html`, leaving the markup `canvas` helper.
- [ ] Move valid runtime callers to existing `@web/canvas` free functions and preserve error propagation, reachability and Wasm unsupported-target expectations. Retire tests whose only purpose was the deleted convenience surface.
- [ ] Test that importing/using pure `@html` under a non-web builder needs no Web binding or JS runtime asset, and that direct `@web/canvas` use under HTML still emits assets on demand.
- [ ] Run related canvas and resource-liveness integration cases. Commit when passing.

## Task 7: Documentation, status and final verification

**Files:** `docs/compiler-design-overview.md`, `docs/build-system-design.md`, `docs/docs/packages/**`, `docs/docs/moth-templates/**`, `docs/developer-docs/project-builders/**`, `docs/docs/progress/**`, `docs/roadmap/roadmap.md`, `index.md` when package layout changes.

- [ ] Replace builder-owned HTML and opt-in MTF statements in the two architectural authorities. State Core/MothSource ownership, official registration, fixed source-kind status and the exact two-package prelude/collision contract once in the owning section. Remove obsolete alternatives and repeated wording.
- [ ] Update user docs for package origins, HTML and Styles imports, `.mtf` availability, SDK builders and the local docs-facade rename. Add `@styles` to the packages-and-builders progress surface.
- [ ] Search the repository for stale `@html center`, `@html Canvas`, builder-only `.mtf`, `Standard` provenance claims and old `docs/styles` paths. Classify legitimate historical or illustrative matches rather than blindly rewriting.
- [ ] Run formatting and focused Rust/Cargo tests, affected `moth tests` lanes, `moth check docs --terse` and `just validate`. When preparing to merge, run `just validate-full` under the repo's pinned toolchain on macOS and Linux as available. Record commands and results in the PR.
- [ ] Perform the Slice review: check duplicate registration, extra provider graph work, redundant code paths, dead compatibility exports, semantic identity drift, failure diagnostics and test footprint. Update `index.md` and progress only where required.
- [ ] Delete this plan and remove its roadmap entry in the final completion commit after permanent documentation owns every accepted contract.
