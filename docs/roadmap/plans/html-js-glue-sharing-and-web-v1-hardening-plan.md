# HTML JS glue sharing and web package V1 hardening

Status: active follow-up on `web-site-and-page-v1`
Current slice: Phase 3 - final audit and handoff
Blockers: none known
Next action: run the final review lanes, then hand off for the user's independent review

## Goal

Finish the `@web/site`, `@web/page` and docs sidebar integration, then emit one compiler-generated JavaScript glue module for every distinct glue source in an HTML build. Preserve page-specific entry scripts, stable browser imports and strict output-path ownership.

This work remains on `web-site-and-page-v1`. Keep it available for a final independent review before retiring the plan or merging the branch.

## Authorities and boundaries

Read `moth-code-review-guide.md` if available, `docs/src/developer-docs/style-guide/style-guide.mtf`, its testing and validation companions, `docs/compiler-design-overview.md` and `docs/build-system-design.md`. Use the canonical package pages under `docs/src/docs/packages/builder/` for public API contracts. The existing HTML output writer and resource planner remain the authority for collision and cleanup policy.

- Keep the accepted `@web/site` and `@web/page` public surfaces. No contextual constants, new template folding stage, generic DOM API, reactive listeners or HTML-Wasm support for `@web/page`.
- `@web/site` stays source-backed and static. `@web/page` stays an explicitly imported browser binding package. The sidebar stays a constant template with an explicit runtime initializer.
- No new output registry, asset identity system, general JavaScript chunking or rework of authored-resource deduplication.
- Keep ordinary page entry artefacts distinct. Deduplicate only compiler-generated glue under `_moth/js/glue/`.
- Preserve error diagnostics, filename safety, case-insensitive output identity checks, provider asset reachability and deterministic build order.
- Do not touch unrelated ongoing work or merge the branch. Keep the final plan and implementation available for user review.

## Confirmed baseline evidence

- The branch emits 81 glue files under `docs/release/_moth/js/glue/`, all 1,047 bytes and all the same Git blob. Each wraps `syncNavigation` and imports the already shared `_moth/js/page.js`.
- `external_js/runtime_glue/paths.rs::glue_module_output_path` hashes `module.metadata.entry_point`, not generated source. `runtime_glue/mod.rs::generate_module_glue` already builds the complete glue source before choosing the output filename.
- `html_project_builder.rs` rejects repeated generated output paths in its `output_path_owners` check when collecting per-page files. `resource_output_plan.rs::reserve_builder_output_path` already accepts repeated reservations of one builder-owned path. Its `plan_one_origin` collision at lines 419-435 is for distinct resource origins and must not be relaxed.
- `src/build_system/output/writer.rs` rejects duplicate output destinations and tracks generated files through the `.moth_manifest` lifecycle. Sharing must happen before writer preparation.
- The 80 nested root pages under `docs/src/docs/` and `docs/src/developer-docs/` inspected in this audit have both `#[sidebar]` and `init_docs_sidebar()!`. `docs/src/docs/@page.moth` has neither. The site homepage `docs/src/@page.moth` renders `#[sidebar]` but does not initialize it.
- Existing tests cover browser snapshots, route aliases, encoded paths, nested breadcrumbs, fallible DOM errors, source-package nested prefixes and different per-page glue wrapper selections. Retain those contracts.

## Phase 0 - code audit and tidy-up

Complete and validate this phase before editing generated glue naming or build-wide output collection.

1. Recheck the branch against the authorities. Inspect the directly touched files and neighbouring owners: `packages/web/site/`, `src/projects/html_project/binding_packages/web/page/`, `src/projects/html_project/binding_packages/web/tests/`, `docs/src/styles/`, registered `@web` package metadata and nested-prefix resolution, HTML builder output assembly, runtime glue generation, output writing and manifest cleanup. Report only concrete defects or opportunities with a smaller implementation. Reuse, consolidate or delete before adding helpers.
2. Finish page coverage. Add the shared sidebar and one `init_docs_sidebar()!` invocation to `docs/src/docs/@page.moth`, following the established `@styles` import and `#[navbar]` ordering. Keep the sidebar already present on `docs/src/@page.moth` and initialize it once. Verify that every authored page that renders the sidebar also runs its initializer and that no page calls the initializer without a sidebar. Keep per-page `page_title` and `page_description` metadata unchanged. Preserve standalone Previous/Next navigation.
3. Audit `page.js::syncNavigation` against its documented contract. Preserve slash-bounded ancestor matching, `/index.html` aliases, case and escape sensitivity, same-origin checks, safe `textContent` DOM construction, no listeners and repeatable updates after History API changes. Reduce redundant code only where it actually becomes clearer. `findNavigationHook` is a trivial single-use wrapper that can be inlined. Keep `routeKey`, `anchorRouteKey` and `isAncestorKey` local because each owns a separate rule. Avoid general DOM-validation infrastructure or duplicate route algorithms in Moth code.
4. Strengthen focused failure and boundary tests where coverage is actually missing. Check root and required-hook errors, navigation refresh after route changes, home and unlisted routes, same-origin/subpath handling and valid breadcrumb ordering. Existing `page_tests.rs` covers many of these: extend it rather than repeat cases. A malformed required home anchor or invalid hook shape must not cause an unhandled JavaScript exception. If a stricter hook contract is needed, document the smallest check and matching typed error. Keep the fake DOM minimal and use the shared Node harness.
5. Audit `@web/site`'s two exports, the nested `web/site` prefix matching and `@web/page` registration for extra aliases, duplicated prefix logic, stale comments or unneeded adapters. Keep the accepted nested-prefix support and current package ownership. Do not refactor Stage 0 or namespace binding without a demonstrated issue. Clarify the `site.url` suffix rule in package docs as a caller convention, since the helper concatenates strings and performs no validation.
6. Audit the remaining sidebar styles, navigation hooks, exports and docs links for dead or duplicated code. Prefer deleting stale markup and unused styles rather than introducing another shared layer. Validate the actual generated docs pages, including both documentation and developer-documentation roots and the site homepage. Avoid formatting-only changes across unrelated pages.

Phase 0 gate: focused Node tests, package and builder tests and `moth check docs --terse` pass. Record which audit candidates were deliberately left unchanged and why. Do not add work based only on stylistic preference.

Phase 0 outcome:

- Docs root and homepage now render and initialise the sidebar. All 83 sidebar pages call `init_docs_sidebar()!` exactly once. The homepage is unlisted, so its breadcrumb reads "Documentation / What is Moth?" under the documented unlisted-page rule.
- `page.js` compares scheme and host instead of `origin`, because opaque origins serialise as "null" and let `file:` pages match `mailto:` links. `findNavigationHook` was inlined. New Node cases cover a malformed home, document-base resolution, other ports and `file:` pages.
- `source_package_root_for_dependency` reuses `match_package_prefix` on resolved components. Dead empty-path guards, a stale test name and a stale overlap comment were removed. `site.url` docs state the leading-slash caller convention.
- Deliberately unchanged:
  - The `homeKey !== null` guard states that an unresolvable home never matches.
  - No stricter hook shape check, since malformed hooks degrade without throwing.
  - One element carrying two hooks is not validated, since that needs DOM identity checks the plan excludes.
  - `listed = page.sync_navigation(...)!` stays because its comment explains the ignored result.
  - `is_source_package_public_export_path` keeps its pre-existing lenient length check, since it only orders edges.
  - Developer-docs breadcrumbs start at the single Documentation home by design.

## Phase 1 - content-addressed glue output

1. **Hash the final glue source.** In `external_js/runtime_glue/mod.rs`, generate `glue_source` exactly as today, then compute a deterministic digest of its final UTF-8 bytes and name the output `_moth/js/glue/module-<digest>.js`. A source-identical wrapper selection must receive the same path regardless of entry point, page order or project location. Different generated JS must receive distinct paths except in the unlikely case of a digest collision, which the output collector must reject by comparing bytes. Use an existing suitable stable hashing facility if available. Do not use `DefaultHasher` or a process-random seed, and do not add a general content-addressed storage framework.
2. **Remove unnecessary page identity from glue paths.** `paths.rs::glue_module_output_path` should accept the generated source or its digest, not `&Module`. The package asset import map currently uses the page-derived glue path only to calculate relative imports. All glue modules live in one directory, so compute those imports relative to that shared directory, with a single relative-URL policy. The actual filename must not enter the generated source and create a hash cycle. Remove or repurpose `external_js/path_identity.rs` if its path hash is no longer used. Avoid preserving an obsolete path-based helper for compatibility.
3. **Share only identical generated glue at the HTML builder output boundary.** Extend the existing per-build output-path ownership map to locate the previously emitted `OutputFile` efficiently, instead of adding a parallel glue registry. When a second compiler-generated glue file claims the same path, require `FileKind::Js` and exact source equality, retain the first file and omit the duplicate. Any different content under the same hash-derived path is an internal collision error. Keep all other duplicate-file errors unchanged, including HTML, Wasm, runtime module and authored-provider destinations.
4. **Preserve preflight and link semantics.** Each page still imports exactly the wrapper names it requires, using `relative_url_path` from its own HTML location to the shared emitted glue path. The glue still imports only its selected provider exports and its `@moth/runtime` dependencies. Continue calling `reserve_builder_output_path` for every output before deduplication so provider-resource conflicts are caught early. Leave `HtmlResourceOutputPlan::plan_one_origin`, resource identity, deferred provider emission and the writer's global duplicate-destination rule unchanged.
5. **Keep existing error ownership.** Reuse the builder's existing duplicate-output diagnostic for non-glue conflicts and report a typed internal infrastructure failure for a genuine digest collision. Include the digest/path and competing entry origins without dumping JS bodies into diagnostics. Do not silently pick a winner when source bytes differ.

## Phase 2 - regression coverage and generated outputs

Extend existing tests under `src/build_system/tests/build_dependency_tests.rs` and adjacent HTML output tests. Prefer behaviour and resulting artefacts over snapshots or hard-coded digests.

Required cases:

- Two or more pages selecting identical external JS functions emit exactly one glue file. Every page imports that file using the correct relative URL, including nested routes. Reversing page discovery/order or changing entry names does not change the emitted glue name.
- Two pages selecting different functions or provider assets still emit separate, correctly linked glue files. Keep `build_html_project_shared_support_provider_keeps_page_wrappers_isolated` and its exact wrapper checks.
- Two same-named generated paths with different JS contents fail explicitly. Exercise the collision policy without relying on finding a natural hash collision.
- A page with no reachable external module exports emits no glue file. Unused `@web/page` bindings remain unreachable and do not emit `page.js` or an import map.
- Development and release wrapper differences are reflected in source-derived output identity. Import-map paths and `@moth/runtime` dependencies still resolve correctly.
- Rebuilding with different selected external functions removes obsolete generated glue through the existing `.moth_manifest` cleanup, without deleting unowned files. Test both clean and successive builds, not merely the in-memory `Project` output list.
- Authored-resource collisions and duplicate non-glue builder outputs still fail under their existing strict rules. Include output-path case/alias safety where the writer already owns it.

Regenerate `docs/release` with `moth build docs --release`. The new output should contain one glue module for the shared `sync_navigation` selection, subject to any other genuinely distinct selections found by the build. Remove obsolete tracked glue copies. Inspect several depth levels of HTML documents to verify imports and navigation markup. Do not edit generated HTML or glue files by hand.

## Phase 3 - final audit and handoff

1. Review all changed code against the code review guide for duplicate hashing, redundant maps, arbitrary helper extraction, stale path-based comments, target-specific leakage and unnecessary allocations. Inspect the full resulting diff, not only the new tests.
2. Run focused tests after each phase, then `just validate` and the required docs check/build gates. Report exact commands, passing or failing outcomes and any environment limitations. A failed gate is a blocker, not a claimed pass.
3. Measure emitted glue file count and total bytes before and after the change. Confirm one shared glue file for the docs navigation case, valid imports and no obsolete files in the final generated release tree. Avoid claiming broader bundle-size or browser performance improvements without measurement.
4. Update only the package README and canonical docs whose accepted behaviour actually changed. Preserve the separate deferred contextual-constants proposal and its existing roadmap entry.
5. Commit work on `web-site-and-page-v1`, leave this plan available and provide a concise review handoff listing changed files, tests, remaining limitations and any justified deferred work. **Do not merge the branch, retire this plan or mark the V1 follow-up closed before the user's final review.**

## Acceptance criteria

- The docs root and homepage have consistent sidebar initialisation. Every static sidebar remains usable without requiring contextual-constant semantics.
- `@web/site` and `@web/page` public APIs and HTML-JS/HTML-Wasm boundaries remain intact.
- Byte-identical generated glue is emitted once per build and every consuming page references it correctly. Distinct glue, collision diagnostics and authored-resource protections remain correct.
- Output cleanup removes stale hash-named glue while preserving unrelated files.
- Focused tests, documentation builds and `just validate` pass with an inspectable diff. The branch is ready for independent review but is not merged or closed.
