# HTML builder `@web/site` and `@web/page` V1

Status: in progress
Current slice: Final review
Blockers: none known
Next action: run the final review lanes and integration audit, then delete this plan and its roadmap entry

## Goal

Provide two HTML-builder-only packages through existing registration systems:

- `@web/site` is Builder/MothSource. It composes static site URLs with the existing structural site-root value. It has no knowledge of the consuming page or browser.
- `@web/page` is Builder/ExternalBinding. It reads live browser document and viewport state and offers one opt-in navigation initialiser through the existing annotated JavaScript package pipeline.

Complete the documentation sidebar across the language and developer documentation sections. Render the component statically, initialise its active navigation and breadcrumb from compiled Moth runtime code and remove the redundant introductory breadcrumbs from the page bodies. Preserve the committed sidebar layout and collapse behaviour.

V1 requires no new compiler evaluation phase, TIR extension, special contextual constant, builder metadata injection, general DOM API, event subscription, Wiring feature or implicit package-import side effect. The separate builder-declared contextual-constants research draft is already tracked in `docs/roadmap/roadmap.md`. Leave it deferred and do not duplicate, move or recreate it.

## Authority and baseline

Read `AGENTS.md`, `HUMANS.md`, the repository style/testing/validation guides, `docs/compiler-design-overview.md`, `docs/build-system-design.md` and the canonical documentation for packages, structural URLs, module roots, HTML entry runtime and HTML routing. Check the current worktree and any ongoing package-programme changes before modifying shared code. Use the code review guide to audit the final diff.

Relevant implementation owners:

- `src/projects/html_project/html_project_builder.rs` registers the HTML builder surface.
- `src/builder_surface/source_package_registry.rs` registers source-backed packages. `packages/html/@mod.moth` is the existing precedent.
- `src/projects/html_project/binding_packages/web/mod.rs` registers annotated JS assets. `web/canvas/` is the existing precedent.
- `src/projects/html_project/structural_url_renderer.rs` already renders `SiteRoot` in the consuming output context.
- `src/projects/html_project/document_shell.rs` places the generated runtime script after the static page body.
- `docs/src/styles/sidebar.moth`, `navbar.moth` and `+package.moth` are committed source. Many `docs/src/docs/**` roots already insert `#[sidebar]`. The sidebar still contains its original `<script>` for active-link marking.

Start with `git status`, a baseline `moth check docs --terse`, `moth build docs --release` and relevant focused tests. Preserve unrelated local edits. Don't restore downloadable sidebar variants or regenerate the committed navigation from a second link catalogue.

The currently delivered page metadata uses `page_title` and related constants. The separately planned `$page(...)` migration must neither be implemented nor blocked here. Neither package reads those constants.

## 1. Public API

Both packages require explicit `@web/site` or `@web/page` dependency clauses. Neither is preluded or added to `.mtf` implicit scope. Their names share the `@web` prefix but have distinct source and binding backings.

### `@web/site`: static URL helpers

Implement `packages/web/site/@mod.moth` with two exported compile-time values:

- `root #String` aliases the existing structural `[@/]` site-root URL.
- `url #String` is a const-template wrapper with a default slot. It concatenates the site root and an explicitly supplied site-relative suffix.

Intended use (confirm the final syntax against the compiler):

```moth
@web/site

home #= site.root
getting_started #= [site.url:docs/getting-started/]
```

The suffix must not start with `/`. `url` performs simple string/template composition, not route validation, percent encoding, normalisation or current-page lookup. Site-root rendering follows the builder's existing origin policy. The helper must retain structural pieces through imports and template composition, including non-root deployment such as `/moth/`.

Implement this with ordinary Moth source and existing `SiteRoot` handling. No new Rust URL helper, string-folding lane, synthetic interface or package-specific intrinsic. Prove the composed helper with a cross-module fixture before adding further surface. A compiler limitation that prevents the accepted API is a blocker to report, not permission to build a replacement evaluator.

This deliberately small package is the static-site namespace foundation. Don't add speculative APIs duplicating `@html`, runtime document access, `site.title`, `site.route` or `site.matches_route`.

### `@web/page`: live browser snapshots

Implement a Builder/ExternalBinding package at `src/projects/html_project/binding_packages/web/page/page.js` with the following exact annotated Moth-facing free functions:

```text
url()                    -> String   window.location.href
path()                   -> String   window.location.pathname
origin()                 -> String   window.location.origin
query()                  -> String   window.location.search (including ?)
fragment()               -> String   window.location.hash (including #)
title()                  -> String   document.title
viewport_width()         -> I32      window.innerWidth in CSS pixels
viewport_height()        -> I32      window.innerHeight in CSS pixels
pixel_ratio()            -> F64      window.devicePixelRatio
scroll_x()               -> F64      window.scrollX
scroll_y()               -> F64      window.scrollY
is_visible()             -> Bool     document.visibilityState == "visible"
has_focus()              -> Bool     document.hasFocus()
prefers_dark()           -> Bool     matchMedia("(prefers-color-scheme: dark)").matches
prefers_reduced_motion() -> Bool     matchMedia("(prefers-reduced-motion: reduce)").matches
sync_navigation(root_id String) -> Bool, Error!
```

Every getter reads the browser on each call. These are browser-only synchronous snapshots, not static build values or reactive subscriptions. URL query and fragment retain their browser delimiters when present. Use the actual browser APIs without guessed defaults, cached import-time values or silent numeric truncation. The existing external-binding ABI validates returned scalar values. Test invalid/out-of-range host values against that boundary rather than adding redundant conversion layers to each getter. A browser-free environment is not a supported execution target for these functions.

Keep the annotations within the existing scalar/opaque ABI. No callbacks, collections, options, receivers, async exports, new runtime imports or compiler intrinsics. Only referenced functions should demand emitted runtime assets.

### `sync_navigation` contract

This is an opt-in helper for a conventional static navigation tree, not a general DOM facade. It takes one root element ID and requires these descendants:

- `[data-page-home]`: a home anchor that supplies the breadcrumb's first link
- `[data-page-links]`: a navigation element containing candidate anchor links
- `[data-page-current]`: an initially empty breadcrumb container, which is hidden until a label is available

The root element itself uses `id="docs-sidebar"` in the docs integration. The helper does not know that ID, the word `Documentation` or any docs route.

On each call:

1. Reject an empty root ID as invalid argument and absent root/required descendants as resource-not-found. Use the existing `@moth/runtime` typed-error functions and error-code constants. A missing matching link is normal and never an error.
2. Resolve each candidate anchor and the home anchor using the browser URL parser relative to `document.baseURI`. Compare only same-origin pathnames with `window.location`, ignoring query and fragment. Normalise terminal `/index.html`, terminal `/` and a missing terminal slash for directory-style paths. Preserve percent-encoded path identity and case; never decode a `%2F` into a path separator. Compare exact paths for active state and slash-bounded ancestor paths for breadcrumbs, never arbitrary string prefixes.
3. Clear stale `aria-current` values on managed links and the home anchor. Mark the home anchor current on the home route, otherwise the first exact navigation match. Return `true` for either exact match and `false` for an unlisted route. Leave unrelated links untouched.
4. Build a breadcrumb from the home label, all distinct matching ancestor links ordered from shallowest to deepest, then the current page. Use the exact link text for a matched page. For an unlisted current page use the first document `<h1>` text, otherwise `document.title` without site-brand stripping heuristics. Do not duplicate the home or final label. Generate the breadcrumb using DOM nodes and `textContent` (including separators) rather than `innerHTML`. Hide the current-label container on the home route.
5. Be idempotent when called again after a History API location change. Never install listeners or navigate. Browser history changes require the caller to invoke it again.

Keep DOM traversal scoped to the specified root except for the intentional current-page `<h1>`/title fallback. The entire operation should live in `page.js` with only small local path and DOM helpers where they remove real repetition. Avoid an extra public DOM package or docs-specific JavaScript asset. The page's document title may differ from the heading, especially when the HTML builder adds title prefixes/postfixes; prefer an authored `<h1>` when no matching link supplies the name.

The HTML remains navigable if JavaScript is unavailable: only the enhanced breadcrumb label and active-link state are missing. The runtime initializer must not be coupled to the sidebar's checkbox-based collapse behaviour.

## 2. Implementation sequence

### Phase 0 - Verify the concrete contracts

- Confirm `@web/site` and `@web/page` are unclaimed, supported as sibling package paths and compatible with the current `BuilderSurface` overlap rules.
- Verify package export syntax for a structural site-root constant and slot-bearing const wrapper, annotated `Bool, Error!` returns and normal Moth function calls from a page root.
- Check exactly which docs roots already contain `#[sidebar]` and which do not. Include `docs/src/docs/**` and `docs/src/developer-docs/**`, including nonstandard root names such as `@cheatsheet.moth`. Exclude the main site homepage `docs/src/@page.moth` unless the existing site design calls for the documentation sidebar there.
- Check representative nested breadcrumbs, especially `docs/packages/core/time/`, `docs/packages/builder/`, `docs/progress/packages-and-builders/` and `developer-docs/compiler-design/...`. Record existing pages lacking an `<h1>` or a navigable ancestor in the sidebar before choosing final link additions.

Phase 0 findings:

- `@web/site` doesn't overlap `@web/canvas`, `@web/graphics` or `@web/page`, and no package claims `@web` itself. Stage 0 discovery, stable identity, import-surface matching and output prefixes already accept a multi-component prefix. `SourcePackageRegistry::register_filesystem_root` still rejects `/`. `ProjectPathResolver` matches and strips only the first dependency component. The project-entry collision check compares single folder names. Phase 1a generalises those three owners and doesn't add a second registry or resolver.
- `root #String = @/`, slot-wrapper const templates, namespace-qualified template heads, `|root_id String| -> Bool, Error!` annotations and `@moth/runtime` typed errors all use existing syntax. No current test proves that `SiteRoot` survives a package export and cross-module composition, so Phase 2 owns that proof.
- The external-binding glue validates I32 results by range. F64 results cross as raw carriers, and the HIR binary-float boundary owns their finite check. Bool and String returns pass through unchecked. `page.js` returns browser numbers unchanged, so Phase 2 relies on those existing generic owners for numeric validation and makes no claim about Bool or String validation.
- The sidebar has 37 navigation links plus the Documentation home link. 35 `docs/src/docs/**` roots insert it, including `@cheatsheet.moth`. `progress/packages-and-builders/` doesn't, and neither does any `docs/src/developer-docs/**` root. `docs/src/docs/@page.moth` and the site homepage stay without it.
- `#[title]` renders an `<h1>`. The language-docs roots render a heading. Developer-docs topic headings come from imported content. Package leaves under `packages/core/` and `packages/builder/` have no exact sidebar link, and nothing links `packages/builder/` itself. Developer-docs topic pages fall under linked `compiler-design/`, `memory-management/` or `developer-docs/` ancestors.

### Phase 1a - Nested builder source package prefixes

- Allow slash-separated builder source prefixes in `SourcePackageRegistry` while keeping shared package-path validation. Reject a prefix that equals, contains or nests inside another registration, using the same overlap rule as source/binding validation.
- Make `ProjectPathResolver` match the registered prefix by whole leading components and join only the remaining suffix.
- Make the project-entry collision check reject an entry-root-relative folder path equal to a registered prefix instead of comparing a single folder name. Ancestor-only folders stay allowed.
- Add focused coverage for nested prefix resolution, compile-time paths under a nested package and collisions.

### Phase 1 - Register and implement both packages

- Register the source root `@web/site` in `HtmlProjectBuilder::frontend_surface()` using `BuilderSurface::builtin_source_package_root`. Register `@web/page` by adding one entry to the existing `WEB_BINDING_PACKAGES` list. Reuse the current parser, runtime asset metadata, import-map, reachability and target validation paths. Don't introduce any parallel registry, special source resolver or JS bootstrap path.
- Implement the `@web/site` constants/template helper and the `@web/page` annotated JS getters and navigation initializer. Follow the existing source-backed and binding-backed package conventions without copying boilerplate that the registration systems already own.
- Add `packages/web/site/README.md` and `src/projects/html_project/binding_packages/web/page/README.md`. Each states the shared ownership boundary, V1 API, implementation location, test owner and deferred extensions. Keep the distinction concise and consistent, rather than maintaining a second full API reference in each README.

### Phase 2 - Focused package validation

- Prove `@web/site` constant/template import and composition through the real compiler, including nested package imports, the `SiteRoot` structural piece and the configured `/moth/` origin. Ensure importing the pure source package emits no browser asset.
- Prove `@web/page` direct and namespace imports, no prelude/implicit `.mtf` visibility, correct Builder-origin identity and unused/reachable runtime asset behaviour. Existing package-collision and unsupported-target machinery must handle its registration.
- Test getters using the existing JS execution test conventions with mutable fake browser state. Cover return values, read-on-each-call behaviour, missing query/fragment and numeric ABI validity. Avoid introducing a new general browser test framework.
- Test `sync_navigation` with a minimal DOM fixture: home and exact pages, nested ancestors, unlisted routes, `/index.html` and no-slash aliases, encoded components, case, same-origin filtering, query/fragment, deployment subpath, repeated calls and changed routes. Assert marker cleanup, breadcrumb text/node safety, `Bool` results and typed failures. Use a stronger targeted DOM test only if the current Node harness cannot model the necessary platform semantics.
- Compile a genuine Moth caller and inspect emitted runtime assets/import maps separately when the Node integration harness cannot execute import maps. Exercise HTML-JS success and the existing HTML-Wasm unsupported-call diagnostic. Do not weaken harness checks or manufacture Wasm support.

### Phase 3 - Finalise the committed sidebar

- Edit `docs/src/styles/sidebar.moth` in place. Keep its 37 existing links, inline CSS constants, embedded selector/media rules, light/dark colours and checkbox collapse. Remove only the inline `<script>` and the now-obsolete script commentary. Keep its CSS `aria-current="page"` styling and reuse the existing `docs-sidebar-links` navigation ID.
- Give the sidebar root `id="docs-sidebar"`, add the three `data-page-*` hooks and add a small accessible breadcrumb container beside the existing Documentation home link. Use the existing style/markup structure instead of replacing the component or making its generated HTML runtime-dependent.
- Define one ordinary Moth `init_docs_sidebar` function alongside the sidebar in `sidebar.moth` that invokes `page.sync_navigation("docs-sidebar")` and propagates typed errors. Export it alongside `sidebar` through the existing `@sidebar` re-export inside `docs/src/styles/+package.moth`. Do not create a dedicated wrapper module or copy JavaScript into the HTML component.
- Update each intended page root to import `sidebar` and `init_docs_sidebar`, render `#[sidebar]` once after `#[navbar]` and call `init_docs_sidebar()` once as top-level runtime work. Confirm the generated entry executes after the static fragment exists. If any root cannot do so under current compiler rules, investigate its supported entry contract rather than introducing implicit execution.
- Remove the leading `Documentation / ...` or `Developers / ...` breadcrumb from the first content section in each migrated root. Preserve headings, substantive prose, Markdown structural links, code examples and independent Previous/Next navigation elsewhere on the page. Preserve the authored language and developer-docs sections as distinct URL trees.
- Add `@web/site` and `@web/page` canonical documentation pages under `docs/src/docs/packages/builder/` using existing package-page conventions. Link them from the Builder packages overview and add the minimal relevant sidebar links for navigating the package hierarchy. The docs landing-page index and sidebar serve different navigation purposes; maintain their existing respective presentations rather than creating a new shared catalogue or duplicating route lists in Rust.

### Phase 4 - Validate, document and clean up

- Run `moth check docs --terse`, `moth build docs --release`, applicable package-scoped Rust tests, `just ci-gate-docs`, format/style checks and first-party JS dependency inventory checks according to the repository validation guide. Record any unrelated baseline failures separately.
- Inspect generated pages and runtime assets in development and release. Verify the authored sidebar has no `<script>` and no `MOTH-SYNTAX-0005` warning, package JS is emitted only as required and page initialisation comes from the normal Moth entry. A generated builder script is expected, not a forbidden authored inline script.
- Browser-check language docs and developer docs at the site root and configured subpath, direct deep links, nested/unlisted routes, browser history reinitialisation, active-link highlighting, breadcrumb structure, keyboard/screen-reader behaviour, mobile and desktop collapse, print, light/dark themes and no-JavaScript fallback.
- Update canonical package-origin/backing and builder-package references, the package/builders progress matrix and any affected usage examples. Document the runtime-vs-static URL distinction and the opt-in nature of `sync_navigation`. Do not change the already-tracked contextual-constants draft or roadmap item.
- Audit the final diff for unnecessary Rust/JS/Moth helpers, duplicate URL parsing, copied navigation catalogues, stale sidebar script/slots, accidental edits to unrelated styles and weak tests. Remove experimental scaffolding and compatibility code for APIs first introduced in this work. Do not manually edit generated `docs/release/**` files or commit build artefacts contrary to repository policy.

## Acceptance

- Both packages are HTML-builder-only, registered through existing backings and usable with explicit Moth dependencies. `@web/site` remains static and `@web/page` remains browser-runtime-only.
- The source-backed site URL helper uses the current structural `SiteRoot` path without new compiler evaluation mechanics. Browser getters and navigation use the existing annotated JS binding and error contracts.
- The shared sidebar is a constant HTML component with no authored JavaScript tag and no per-page title/path slots. Runtime initialisation is a normal, explicit Moth call and requires no page-specific JavaScript.
- Language and developer documentation pages consistently show the sidebar. Current and ancestor breadcrumbs are correct wherever matching navigation links exist, with an H1/title fallback for unlisted pages. Exact active links receive `aria-current="page"` and no stale markers persist after another explicit sync.
- Old introductory breadcrumbs are removed without deleting page headings, important prose or Previous/Next navigation. The committed navbar and sidebar styling and all existing links remain functional.
- Focused compiler and runtime tests, docs build and the repository's applicable validation gates pass. No contextual-constant implementation or duplicate roadmap work is introduced.
