# Resource-only Roots Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task by task. Track work with `- [ ]` checkboxes. Every phase has a mandatory correctness audit, redundancy and style review and validation gate.

**Status:** Queued after Release call folding. Do not start until Namespaced public export paths is merged into the implementation baseline. Core Math fast counterparts follows this plan.

**Goal:** Let a deliberately marked directory expose ordinary file resources to its scoped sibling modules and descendants through existing explicit-extension `@` paths, without a facade, export declarations or relaxed source-module privacy.

**Architecture:** `@resources.moth` is a comment-only directory marker discovered by Stage 0. It creates a scoped resource namespace and a physical-resource ownership boundary, but no semantic module, public interface, entry, package assembly or compiled declarations. Reuse the support-package consumer-visibility and collision policies, the existing resolved file-reference handoff and the existing resource origin, byte-source, URL, output and invalidation pipeline.

**Tech stack:** Rust, Stage 0 source tree and module namespaces, frontend retained path/file-value facts, resource linking, Moth HTML builder and `tests/cases` integration runner.

**Accepted specification:** The accepted contract below records the project lead's decisions. Promote its enduring rules into the canonical references in Phase 5. The existing authorities are `docs/build-system-design.md` (project topology, source sets, physical resolution, resource linking), `docs/compiler-design-overview.md` (Stage 2 and Stage 4 file values), `docs/docs/resources/file-paths.mtf`, `file-values.mtf`, `docs/docs/packages/project-local-packages.mtf`, `module-visibility.mtf`, `dependency-paths.mtf` and `docs/docs/project-structure/module-roots.mtf`. The delivered namespaced public export work must be in the baseline before implementation, but its plan is not a semantic dependency or enduring authority.

## Global constraints and implementation policy

- Read `AGENTS.md`, the complete `docs/developer-docs/style-guide/style-guide.mtf`, the Moth cheatsheet, relevant canonical unsuffixed language references and routed sections of both design authorities. Read `testing.mtf` and `validation.mtf` before choosing checks. Inspect the implementation and current progress matrices rather than assuming this plan's likely file paths remain exact.
- Use the normal sequenced roadmap. Activate this plan only after the public export path work is delivered on the branch being developed. Resolve any namespace ownership or collision changes from that work in Phase 0. Do not implement either feature through a parallel resolver.
- Moth has one source index and one path-resolution authority. Reuse existing resource facts, source identity and diagnostics. Do not introduce a generic virtual filesystem, resource import provider, public-resource interface, builder-specific directory scan or second parser.
- Prefer deletion and extension of existing owners over new structures. Avoid generic abstraction, compatibility shims, transitional paths, speculative configuration, parallel caches, whole-project implicit imports and changes to package acquisition.
- Preserve direct paths and qualified declaration imports as distinct syntax classes. Resource references always use explicit file extensions. A resource root never adds symbols to local scope or imports files as source declarations.
- Keep project/package graph ownership, stable origins, path containment, symlink and case validation, output conflicts, project-relative resource placement and demand-driven emission. Changes to visibility must not change resource bytes, URLs or module interfaces for already legal references.
- Treat non-resource source-kind paths exactly as today unless a resolved path is explicitly within an eligible resource-only root. A `.moth` path outside this surface still has `MothFileHasNoValue`. A `.mtf` or `.md` path outside it still means compiled content.
- All user-authored failures are typed diagnostics with relevant file and use-site spans. No user-caused panic. No silent resolution fallback, provider priority or accepted case-only ambiguity.
- Use red/green tests per independently testable task, focused tests at each stable commit and documented phase gates. Do not commit skipped, permanently failing or acceptance-only tests where stronger assertions are possible.

## Accepted user-visible contract

### Marker, scope and spelling

- An exact, case-sensitive file named `@resources.moth` declares a resource-only root for its containing directory. It may contain whitespace and ordinary Moth code comments (`--`), including comment-only lines. Any other source tokens, directives, declarations, runtime code or template syntax are diagnosed. It is never executed, imported, read as a resource or made part of a semantic source set. No configurable content language or aliases are introduced.
- This is a reserved marker outside the three semantic root roles (`@mod.moth`, `@package.moth`, `@entry.moth`) and outside build-only `@config.moth`. It does not create a `ModuleId`, a public semantic interface, `start`, a route, a source package or a fourth compiler compilation lane. Recognize its exact spelling before the generic invalid-`@*.moth` check. Invalid case variants retain the existing invalid-root diagnostic family.
- The directory name supplies the public root name. For example, `site/assets/@resources.moth` makes `@assets/logo.svg` and `@assets/fonts/body.woff2` available to eligible consumers. Ordinary nested directories below the resource root remain traversable. Do not add a reserved `@resources` import, a project-relative alias or a second URL syntax.
- A resource root is scoped exactly like a structural `@package.moth` support surface: use the nearest normal-position ancestor and the established consumer-position rules for its owning project or source-package boundary. It is visible to that owner and eligible normal sibling branches and their descendants, not to ancestors outside the scope, unrelated branches, private implementation branches that support visibility excludes or consumers in a different package compilation boundary.
- At the selected build root there is no named resource-root path to publish. The marker is legal and may affect local resource ownership, but it creates no anonymous global import and gives descendant modules no implicit project-root access. The normal ergonomic shape is a named child directory such as `assets/`.
- A `@resources.moth` directory may coexist with `@mod.moth`, `@package.moth` or `@entry.moth`. File-value syntax selects the resource surface and dependency/qualified-symbol syntax selects the applicable semantic facade. This exact co-location is a deliberate disambiguation, not permission for different directories to claim an overlapping visible name. Keep existing public-route and source-package collision rules, including case-only collisions and reserved Core/Builder roots. Reject ambiguous distinct visible claims with both locations, never first-match precedence.

### Which files are exposed

- The resource surface exposes eligible ordinary files directly and through its unrooted subdirectories. Exposure is by a *referenced path*, not eager compilation or copying of every file. An ordinary file keeps its bytes and extension. Missing files, directory targets, excluded `_` directories, containment escapes and symlink escapes retain the established diagnostic/skip policies.
- In a **resource-only** directory, `.moth`, `.mtf` and `.md` are ordinary raw resource files when reached through the resource namespace, always with the full extension. Their contents are neither compiled, type checked, Markdown-rendered nor returned to source code. The `String` is a tracked resource URL anchor and the builder copies the original bytes. This does not implement source-file reflection or runtime file-content reads.
- If the **same directory** also contains `@mod.moth`, `@package.moth` or `@entry.moth`, `.moth`, `.mtf` and `.md` files owned by that semantic module are **not exposed as resources** through that directory's resource surface. They retain ordinary source/content semantics in their own valid contexts and may be exposed via separately authored facade declarations, never by implicit raw-resource access. Apply the same protective rule to a co-located reserved build-only `@config.moth` without making that file a module root. Other ordinary resource extensions remain exposable in the paired directory.
- Compiler marker files (`@resources.moth`, `@mod.moth`, `@package.moth`, `@entry.moth`, `@config.moth`) are never resource targets. Unknown special `@*.moth` marker spellings remain invalid source structure where current discovery diagnoses them and cannot be laundered as raw assets. Do not expose an invalid marker to avoid its diagnostic.
- A nested semantic module, support package or independently marked resource root ends an enclosing resource root's traversal. A parent cannot import a nested private file via an explicit-extension suffix. The nested root has its own normal scoped visibility when eligible. Unmarked subdirectories do not establish a boundary.
- A `.moth`, `.mtf` or `.md` file exclusively owned by a resource-only root is not also a normal helper, semantic content source or check-only orphan of its nearest ancestor module. Co-located semantic roots keep their normal owned/check-only source classification. This ownership rule prevents raw example files from unexpectedly acquiring compiler errors.
- Preserve the existing compiler source roots and dependency clause rules. A path into a resource root cannot bind `content`, import declarations, be resolved extensionlessly or produce a fallback provider. An internal package's resources do not become directly importable across an external project/source-package graph boundary: expose their anchored strings through the package's explicit facade where required.

### Values, output and identity

- A resource-only-root path is one ordinary `String` with a structural Resource piece, identical to an ordinary static asset path. It has one stable canonical physical source, owner-relative logical resource origin and authored use location. Multiple consumers of the same physical logical asset resolve to the same semantic origin. Do not key it to the importing module, the spelling alias, page route or absolute host path.
- Referencing it establishes a Stage 0 input/watch interest. Only an origin reachable through selected entry/package exports, functions, fragments or builder slots causes emission. Unreachable assets are not read, hashed or copied. The output remains at its project-relative path for project-local resources and existing package-prefixed path for source/dependency package resources.
- The builder resolves a URL relative to the consuming output artefact. For example, an asset at `site/assets/logo.svg` used in `site/page/about/index.html` renders as `../../assets/logo.svg`. Ordinary site-root `@/` retains its separate origin-aware navigation semantics, and `@/assets/logo.svg` remains invalid as a file path.
- Byte changes invalidate affected output without forcing semantic recompilation where only resource bytes changed. Missing-target watch interests, resource unions, output conflict checks and dev-server rebuilds use existing owners. No eager resource tree enumeration, separate copy loop or resource-specific generated HTML.

### Explicitly outside this plan

- `@config.moth` lists of global asset folders, arbitrary project-root/ancestor access, `..`, absolute filesystem paths and a new general import language.
- Alias maps, `include`/`ignore` lists, per-extension configuration, resource transforms, globs, directory values, automatic directory copying, content-based URLs or resource glob exports.
- Importing raw resource bytes into Moth as text, source reflection, parsing resource `.md`/`.mtf`, executing `.moth` under resource-only ownership, or turning resources into module declarations.
- Hidden/special marker import, dynamic `String`-to-path lookup, live runtime file IO, recursive namespace records and new cross-package acquisition or reexport privileges.
- Automatic registration of public routes into the `.mtf` prelude or an implicit flattening of namespaced public export paths.

## Concrete acceptance example

```text
site/
  @entry.moth
  assets/
    @resources.moth           -- optional comments inside
    logo.svg
    example.moth
    guide.md
    views/sample.mtf
  page/about/@entry.moth
```

`site/page/about/@entry.moth` may use all four resource paths as tracked URLs:

```moth
logo = @assets/logo.svg
source_sample = @assets/example.moth
guide = @assets/guide.md
sample = @assets/views/sample.mtf

[$html:<img src="[logo]" alt="Site logo">]
```

The `.moth`, `.md` and `.mtf` files in this example are emitted unchanged as file resources and do not contribute declarations or compiled content. Adding `site/assets/@mod.moth` must make those three source-kind files unavailable *as raw resources* while retaining `logo.svg` as an accessible resource. It must not make the other module's private declarations visible to the page.

## Likely owners and test locations

Confirm after the prerequisite merges. Reuse these existing entry points and tests rather than introducing a parallel subsystem.

- Marker discovery, reserved-name validation, ownership and source/check set: `src/build_system/create_project_modules/source_tree_index.rs`, `module_inventory.rs`, `project_roots.rs`, `discovery_traversal.rs`, `source_discovery.rs`, `discovery_owned_source.rs` and `project_structure_diagnostics.rs`.
- Scoped visibility and exact path claims: `src/build_system/create_project_modules/module_namespace/mod.rs`, its route publication and collision owner `module_namespace/route_publication.rs`, `source_tree_index.rs`, `project_module_graph.rs` and the existing source-package namespace collision checks. Consume the *delivered* namespaced public export route lookup without treating resource roots as those routes.
- Retained reference kinds and physical validation: `src/compiler_frontend/headers/file_parser.rs`, frontend path/token ownership, `src/build_system/create_project_modules/file_reference_resolution.rs`, `resource_inputs.rs`, `prepared_source.rs` and the current AST file-value resolver. Keep one resolved outcome rather than another resolver in AST.
- Linking and output: `src/build_system/resource_unions.rs`, `src/build_system/output/`, HTML project builder's resource plan and the existing dev-server resource watching logic.
- Unit tests: `src/build_system/tests/create_project_modules_source_tree_index_tests.rs`, `create_project_modules_structure_collision_tests.rs`, `create_project_modules_file_reference_tests.rs`, `create_project_modules_source_discovery_tests.rs`, `build_dependency_tests.rs`, `resource_inputs_tests.rs` and `resource_union_tests.rs`. Use existing frontend file-value tests and diagnostic families when a frontend fact changes.
- End-to-end tests: a compact resource-root case plus focused rejection cases under `tests/cases/`, registered in `tests/cases/manifest.toml` with observable HTML output, copied file byte content, absence and targeted diagnostic assertions. Prefer extending existing fixtures if they own the same contract.

## Review focus

The phase tasks below must explicitly test these five high-risk boundaries:

1. **Source-kind laundering:** `.moth`, `.mtf` and `.md` under an unpaired resource root are raw assets, not compiled or check-only sources, while paired-root and ordinary-source paths keep their old meaning.
2. **Private module bypass:** Nested module roots, scoped support packages and source-package compilation boundaries cannot be traversed to expose private files or declarations.
3. **Scope and collision drift:** Support consumer-position rules, case-only collisions, co-located role disambiguation and newly delivered virtual public export paths never introduce precedence/fallback or a duplicate graph owner.
4. **Resource identity and emission:** One asset used by multiple entries or through multiple legal paths has one origin and output, correct page-relative URLs, no unused emission and correct byte-only invalidation.
5. **Diagnostic/source lifetime:** Invalid marker content, special filenames, symlink escapes, missing files and output conflicts preserve typed diagnostics, source spans, watches and deterministic order.

---

## Phase 0: Baseline, merge prerequisite and implementation map

**Deliverable:** A verified post-export-path baseline and a narrow change map. No new observable language behaviour.

### Task 0.1: Read the exact owners

- [ ] Confirm Namespaced public export paths is merged into the target branch and its new route/collision facts are the sole namespace authority. If not, stop feature implementation and retain this queued plan.
- [ ] Read the global routed references and the current `AGENTS.md` Slice review policy. Walk source index creation, module ownership, scoped package namespace construction, retained physical references, resource source publication and output emission from a real existing fixture.
- [ ] Document which existing types represent directory root claims, consumer positions, file-reference outcomes and resource origins. Identify which are extended or replaced and which code is removed. Do not create a second path classification table.
- [ ] Check `docs/docs/progress/@entry.moth`, `docs/docs/progress/packages-and-builders/@entry.moth` and current `tests/cases/manifest.toml` for baseline feature status and coverage. Do not infer implementation from canonical design prose.

### Task 0.2: Record baseline checks

- [ ] Run focused source-tree, module namespace, source-kind/file-value, resource input/output and namespaced export path tests. Record commands and results in branch working notes (not hard-coded commit IDs in this queued plan).
- [ ] Define red/green cases for a resource-only directory, one paired module/resource directory and one invalid boundary. Do not commit permanently failing tests before their implementing phase.

**Phase 0 mandatory audit / style / validation gate**

- [ ] Review ownership, drift from merged export routes, test gaps and proposed new structures. Independently challenge whether an existing helper can own every new fact and whether any unneeded public API would be introduced.
- [ ] Review the implementation map against the style guide and the code-review guide; remove speculative abstractions. Confirm baseline focused tests and unchanged diff. Record pass/fail evidence, limitations and the next phase's owner map. Stop on an unresolved authority conflict.

## Phase 1: Marker discovery, lexical validation and source ownership

**Deliverable:** Stage 0 recognises comment-only `@resources.moth` markers with correct ownership and no API/entry side effects. Existing resource paths are not yet cross-module visible.

### Task 1.1: Recognise and validate the marker

- [ ] Add failing focused tests for an empty marker, whitespace and several `--` comment lines, with valid marker content accepted at any eligible source-tree depth. Add rejection tests for a constant, a template, a directive, invalid UTF-8, a case variant such as `@Resources.moth` and a marker symlink outside the selected boundary. Assert typed diagnostics with useful marker spans.
- [ ] Extend the existing reserved file classifier to recognise exactly `@resources.moth`, before generic special filename validation. Validate permitted content using the existing compiler tokenizer/lexical comment rules, not a new line/comment parser or special template mode. Keep marker source snapshot and diagnostic ownership correct.
- [ ] Reuse existing root-claim and canonical-path machinery, but publish a resource-only directory fact rather than a new semantic module root. Assert no `ModuleId`, public interface, semantic source record, entry or route appears solely because of the marker.

### Task 1.2: Establish exclusive resource ownership

- [ ] Add failing tests for a resource-only tree containing `.moth`, `.mtf`, `.md`, `.svg` and nested unrooted folders. Assert source/check-only sets exclude the three source-like files until a co-located semantic root explicitly owns them. Assert known module roots nested inside the tree remain independent boundaries.
- [ ] Teach `OwnedSourceSet`, `SemanticSourceSet` and check-only inventory classification about resource-only ownership. Do not eagerly register every asset as an input. Existing source declaration discovery elsewhere must remain unchanged.
- [ ] Cover co-located `@entry.moth`, `@mod.moth`, `@package.moth` and project-root `@config.moth`, preserving semantic ownership and stopping accidental raw exposure of source-kind files. Cover hidden `_` subtree skip, canonical symlinks and invalid `@*.moth` markers without silencing existing diagnostics.

**Phase 1 mandatory audit / style / validation gate**

- [ ] Perform correctness and regression audit of marker diagnostics, root classification and source/check-only ownership. Perform a redundancy and style-guide review of touched discovery code, new allocations, comments, failure lanes and test honesty. Remove duplicate scans or unused state.
- [ ] Run `cargo fmt --all -- --check`, `cargo check -p moth`, focused source-tree/index/structure tests and affected integration controls. Record exact commands/results and commit only a stable green slice. Do not start Phase 2 with unresolved findings.

## Phase 2: Scoped resource namespace and collision enforcement

**Deliverable:** Consumers can resolve a visible resource root by its directory name without getting declarations, a synthetic public interface or broader filesystem access.

### Task 2.1: Reuse support scope rules

- [ ] Add failing cases for `site/assets/@resources.moth` used from `site/@entry.moth`, `site/page/about/@entry.moth`, a normal sibling module and a nested eligible consumer. Add rejected cases for an unrelated branch, ancestor above the nearest normal owner, a support-private subtree and a separately compiled source/dependency package.
- [ ] Extend namespace inventory with the resource-only claim as a distinct target category, reusing the established support consumer-position computation. Resolve the exact resource-root identity and physical owner before any file-kind result. Preserve stable resource identity across consumers and avoid a public facade, graph compilation job or provider-supplied symbol table.
- [ ] Require explicit-extension resource syntax. Assert bare `@assets` clauses, `@assets name` qualified selection, `@assets/child` extensionless paths and `@/assets/logo.svg` never gain resource namespace meaning. Ordinary source/provider syntax stays as currently documented.

### Task 2.2: Enforce boundaries and ambiguity policy

- [ ] Add failing collision tests for two distinct resource roots whose importable claims overlap, case-only duplicates, a visible normal/support module with the same claim, a virtual public export route and reserved `core`/`builder` names. Assert both locations and deterministic outcomes. Add positive co-location tests for one directory with both `@resources.moth` and each existing semantic root role.
- [ ] Reuse namespace collision validation from the post-export-path implementation. Keep exact co-located root roles distinguishable by file-value versus symbol/dependency syntax. Do not allow identical names in different physical directories through fallback or precedence.
- [ ] Add negative traversal tests for nested normal modules, nested support packages, nested resource markers, symlink aliases and foreign canonical paths. Reuse existing path containment, case and ownership checks; never jump through a nested boundary just because the target extension is a resource extension.

**Phase 2 mandatory audit / style / validation gate**

- [ ] Audit the new claim against the namespaced export path resolver, interface privacy, case folding, source-package boundaries and deterministic diagnostics. Inspect every consumer of the namespace enum for a forgotten default/fallback branch.
- [ ] Review code size, duplicate lookup maps, clear ownership comments and source-database lifetimes against the style guide. Run focused namespace/collision/dependency tests, `cargo check -p moth` and formatting checks. Record evidence and commit a green slice before Phase 3.

## Phase 3: Contextual source-kind classification and raw file values

**Deliverable:** An explicitly extended path resolved through an eligible resource root yields the existing structural Resource `String`, including `.moth`, `.mtf` and `.md` where the directory is resource-only.

### Task 3.1: Preserve one path fact through Stage 2 and Stage 0

- [ ] Add failing tests that compare `@assets/sample.moth`, `@assets/page.mtf` and `@assets/guide.md` against ordinary `@sample.moth`, `@page.mtf` and `@guide.md`. Assert the former are raw Resource anchors only when the selected root is resource-only and the latter keep `MothFileHasNoValue` or compiled `content` semantics.
- [ ] Change retained path classification only as needed to allow owner-aware resolution before the final file-value kind is committed. In particular, the current unconditional `.moth` `SourceKindNoFileValue` shortcut must remain for ordinary paths but must not discard a candidate that Stage 0 can prove belongs to an eligible resource-only root. Do not start physically probing ordinary `.moth` paths that remain invalid by syntax.
- [ ] Produce one validated resolved reference outcome with owner, resource origin and use span. Route it through the existing AST Resource-piece projection. Do not create a second URL type, a new content wrapper or separate HIR/backend lowering.

### Task 3.2: Defend paired roots and source-kind invariants

- [ ] Add rejection tests showing all three source-kind extensions in a directory paired with `@entry`, `@mod` or `@package` are inaccessible *as raw resources*, even when imported from another eligible module. For each, prove that an ordinary non-source asset in that same directory still works.
- [ ] Assert `@resources.moth` and all compiler root/config markers never become resource file values. Assert a nested normal module cannot be traversed for `any.ext`, even if its filename looks ordinary.
- [ ] Check whether the source index would still compile, check or source-watch raw `.moth`/`.mtf`/`.md` files in an exclusively resource-owned subtree. Remove any overlapping semantic membership, preserving only physical-resource input/watch interest for authored references. Test real invalid Moth syntax stored in `sample.moth` succeeds as a copied resource, not as source.

**Phase 3 mandatory audit / style / validation gate**

- [ ] Audit file-kind precedence, stage ownership, raw-content noncompilation, source IDs/remapping, missing-file diagnostics and config-file prohibitions. Audit cases where source type and resource type disagree before and after a co-located-root edit.
- [ ] Review the smallest possible change to existing classification and resolver code. Run focused frontend file-value tests, file-reference discovery tests, `cargo check -p moth`, formatting and direct `.mtf`/`.md` controls. Commit only after all phase failures and review findings are resolved.

## Phase 4: Resource output, incremental behaviour and adversarial integration

**Deliverable:** Shared resources behave identically to existing tracked assets through output, URL contexts, reuse and diagnostics.

### Task 4.1: Prove output semantics and stable identity

- [ ] Add a single rich multi-entry HTML integration case where an asset in `assets/@resources.moth` is used by `@entry.moth`, `page/about/@entry.moth`, a static template and a builder slot. Assert one emitted asset at `assets/logo.svg`, correct root-page and nested-page `src` values, exact bytes and no duplicate output/manifest entry.
- [ ] Include `.moth`, `.mtf` and `.md` raw resource copies in that scenario and assert byte-for-byte output, including markup and source syntax that would be invalid if parsed. Verify they are not emitted when unused. Add a separate negative integration case for a paired root and nested private resource boundary.
- [ ] Confirm the existing output/resource linker consumes the same stable origin and physical byte source. Reuse the existing conflict checks when a generated output claims the same path. Keep output placement project-relative and source-package resources package-prefixed.

### Task 4.2: Incremental, error and platform lanes

- [ ] Add focused rebuild tests that first establish actual semantic/physical reuse, then change only resource bytes, delete/recreate a referenced resource and change `@resources.moth` or co-located root status. Assert correct invalidation, missing-file watch interest, updated copies, no stale origin and no needless consumer semantic recompilation for byte-only edits.
- [ ] Test resource path collision, output conflict, invalid canonical symlink, case mismatch, invalid marker source and nested root denial. Assert diagnostic codes/reasons and primary/related source locations where defined. Repeat representative discovery in reversed creation/inventory order to check determinism.
- [ ] Exercise the JS/HTML path and any other builder path that already accepts ordinary resource references, without inventing new backend support. Run selected feature lanes under the current validation policy. Verify source-package and project graphs do not share a resource claim by accident.

**Phase 4 mandatory audit / style / validation gate**

- [ ] Independently audit identity stability, end-to-end privacy, emitted files, incremental invalidation, missing watches, conflict ordering and diagnostics. Perform a code-bloat/style audit against current resource owners, deleting redundant paths introduced during earlier phases.
- [ ] Run the focused integration cases (`cargo run --quiet -- tests --case <case-id> --backend html`), resource and output subsystem tests, `just validate` at this cross-system checkpoint and affected feature lanes. Record commands/results, findings and any explicitly unrun targets. Commit a green integrated slice.

## Phase 5: Permanent documentation, matrix and completion

**Deliverable:** One permanent documented resource-root contract, honest implementation coverage and a reviewed merge-ready branch.

### Task 5.1: Update canonical references (required when implementation lands)

- [ ] Update `docs/docs/resources/file-paths.mtf` and `file-values.mtf` with the exact marker, comment-only content, scoped `@assets/foo.ext` syntax, raw `.moth`/`.mtf`/`.md` exception, co-located-root exclusion, nested boundary rules, byte copying, URL rendering and the continuing invalidity of `@/assets/foo.ext`. Replace any statement that says every explicit `.moth` file value is always rejected without qualifying the resource-root exception.
- [ ] Update `docs/docs/project-structure/module-roots.mtf` and `project-layout.mtf` to recognize the marker outside the three normal roots and outside `@config`, including its non-module ownership and co-location policy. Update `docs/docs/packages/project-local-packages.mtf`, `module-visibility.mtf` and `dependency-paths.mtf` for the support-like scope, file-only export, nested boundaries and collision policy. Include a public source example, not a new language/import syntax.
- [ ] Update `docs/build-system-design.md` in `Architectural invariants`, `Source indexing and source sets`, `Project and package topology`, `Physical reference resolution` and `Resource linking and output placement`. Update `docs/compiler-design-overview.md` in Stage 2 retained reference classification and Stage 4 `File values and resources`. Ensure each fact has one owner and old unconditional source-kind assertions are rewritten, not contradicted in a later paragraph.
- [ ] Update `docs/docs/cheatsheet/moth-language-cheatsheet.mtf` only if a short example of the new legal file path is justified. Update the user-facing documentation navigation and its Basic pair only where necessary for existing site structure. Do not edit generated HTML under `docs/_release/**` directly. Check `index.md` for a real moved/new module owner, not merely documentation changes.

### Task 5.2: Record current and deferred status precisely

- [ ] Update the `Resource`/file-value and source discovery rows in `docs/docs/progress/@entry.moth`, or add a narrowly titled `Resource-only roots` row if existing rows cannot express this feature clearly. Mark **Supported** only for delivered and tested frontend/project/HTML behaviour, with actual targeted/broad coverage and any genuinely unsupported builder/backend lanes. Do not mark source reflection or runtime file reading as delivered.
- [ ] Update `docs/docs/progress/packages-and-builders/@entry.moth` in existing project-local package/HTML resources rows only when their coverage or runtime support changes. The new marker is not a general package or registered source-package implementation. Keep matrices factual for the merged tree and do not add speculative features as supported.
- [ ] Keep the optional marker content features (aliases, allow/ignore lists and file-type filtering), configurable global folders, arbitrary ancestor paths, globs, raw text reads, resource transforms and new package semantics **deliberately deferred/outside this implementation**. Document these as excluded in the permanent resource language pages; add a short accepted-deferred note under `docs/docs/design-scope/deferred-and-outside-scope.mtf` only if needed to correct existing exclusions. Do not create implementation placeholders, matrix rows for every idea or a speculative roadmap task without an approved use case.
- [ ] During implementation, keep this plan as the active roadmap item immediately after Release call folding. Do not let Core Math fast counterparts run ahead. When all phases and the final gates pass, remove this plan file **and its roadmap bullet in the same finishing commit**. Core Math fast counterparts then becomes the next normal sequenced item, with its dependency wording updated only if necessary. Preserve accepted semantics in canonical documents before deleting the plan.

### Task 5.3: Final validation and mandatory Slice review

- [ ] Run the documentation gate: `moth check docs --terse` and `moth build docs --release` (or the documented Cargo alternative). Inspect changed routes, links and examples. Ensure generated output is produced only by the compiler.
- [ ] Run final targeted unit and registered integration cases, formatting, `just validate` and **`just validate-full`** for the final integrated tree before recommending merge. Re-run invalidated gates after later changes. Record exact commands, counts, failures, platform limits and validation evidence rather than claiming a gate that did not run.
- [ ] Perform the `AGENTS.md` Slice review across all phases: compare the final diff to every accepted rule, audit source/privacy boundary correctness, stage ownership, duplicate state/resolvers, errors/source spans, real test assertions and Rust style. Review `docs/bug-reports.md`, the audit log's stale-row rules and the status matrices. Resolve in-scope issues before claiming completion. Record concrete unrelated leads without expanding scope.

**Phase 5 mandatory audit / style / validation gate**

- [ ] Obtain an independent correctness/regression review and a redundancy/style review of the final diff where reviewer tooling is available. Reconcile every finding against the project authorities and the accepted contract. Do not substitute green tests for an architecture/privacy review.
- [ ] Confirm documentation release output, all selected final gates and the full-validation gate on the same final tree. Check only the intended files changed, including the final roadmap retirement. Report any unrun gate as unrun. Then finish the branch and request final PR review.

## Mandatory gate protocol for every phase

A phase is incomplete until its audit, style and validation checks pass. At every phase boundary:

1. Run a **correctness and regression audit** of the touched ownership boundary, including a negative/hostile test review, diagnostics and stage handoff. Challenge apparent bugs against the accepted design and existing coverage.
2. Run a **redundancy and style-guide audit**: review the touched Rust against `docs/developer-docs/style-guide/style-guide.mtf` and `moth-code-review-guide.md` if available, remove parallel implementations and stale paths, and confirm comments explain ownership and non-obvious reasons.
3. Run the **applicable validation** from `docs/developer-docs/style-guide/validation.mtf` with an explicit scope and recorded outcome. Use focused checks for intermediate slices, `just validate` for broad integration and `just validate-full` for final merge readiness. Review test assertions against `testing.mtf`.
4. Fix all in-scope findings before proceeding. Record actual checks, pass/fail evidence and deferred/blocked issues in working notes or the repo's required issue lane. If a **formal structured audit** is separately invoked, follow `docs/roadmap/audit-guide.md` and its read-only audit/report workflow; do not mislabel the phase Slice reviews as formal audit-log coverage.
5. Commit only a coherent reviewed slice, with no failing/skipped regression fixtures or redundant scaffolding. Do not start the next phase on an unresolved blocker.

## Final acceptance criteria

- A named `@resources.moth` directory exposes ordinary referenced static assets to exactly its scoped eligible consumers without a package facade or export declarations.
- Empty and code-comment-only markers are accepted, invalid content produces a typed diagnostic and the marker is not importable or executable.
- `.moth`, `.mtf` and `.md` are raw tracked resource files under an unpaired resource-only root when spelled with their extensions. Co-located semantic root files protect these source kinds from raw-resource exposure.
- Nested root boundaries, private module paths, cross-package graph isolation, reserved names, case collisions and symlink containment cannot be bypassed.
- No new semantic module, public interface, backend string type, global asset configuration, parser, registry or copying loop is introduced.
- Resource origins, demand-driven emission, URL rendering, byte-only invalidation and diagnostics reuse the existing pipeline with integration evidence.
- Canonical documents and progress matrices reflect delivered behaviour, speculative extensions remain explicitly outside scope and the roadmap advances only after the final reviewed and validated implementation.
