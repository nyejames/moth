# `@web/page`

`@web/page` is the HTML builder's ExternalBinding package for live browser document and viewport state. It shares the `@web` prefix with `@web/site`, a Builder/MothSource package for static compile-time URLs at `packages/web/site/`.

| Owner | Holds |
|---|---|
| This README | Package direction and deferred extensions |
| `docs/src/docs/packages/builder/` | Accepted public API and semantics |
| Packages and builders progress matrix | Current implementation status and accepted next gaps |
| `docs/compiler-design-overview.md`, `docs/build-system-design.md` | Cross-package compiler and build architecture |
| `docs/src/docs/packages/external-binding-contracts.mtf` | Annotated JavaScript binding contract |

## V1 API

Document and location snapshots:
- `url`, `path`, `origin`, `query`, `fragment`: browser location strings.
- `title`: document title.
- `is_visible`, `has_focus`: document visibility and focus.

Viewport and preferences:
- `viewport_width`, `viewport_height`: viewport dimensions in CSS pixels.
- `pixel_ratio`, `scroll_x`, `scroll_y`: pixel ratio and scroll positions.
- `prefers_dark`, `prefers_reduced_motion`: browser media preferences.

Navigation:
- `sync_navigation(root_id)`: opt-in active-link and breadcrumb synchronisation.

Each getter reads the browser on every call. `sync_navigation` requires `data-page-home`, `data-page-links` and `data-page-current` descendants under the identified root. It is idempotent, installs no listeners and must be called again after History API changes.

`page.js` implements the package and `../mod.rs` registers it. It is HTML-JS only. HTML-Wasm rejects reachable calls.

`../tests/page_tests.rs` runs `page.js` in Node against a minimal fake DOM and owns getter and `sync_navigation` behaviour. The `tests/cases/web_page_reachable`, `web_page_unused` and `web_page_missing_import_rejected` cases own compiled callers, runtime asset emission and the missing-import boundary. Generic I32 result validation belongs to the external binding glue tests, and F64 finiteness to the HIR binary-float boundary.

Event subscriptions, reactive state, general DOM access and Wiring are deferred.
