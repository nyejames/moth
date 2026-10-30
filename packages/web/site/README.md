# `@web/site`

`@web/site` is the HTML builder's source-backed package for static site URLs. Its values are compile-time strings that keep the structural site root until the builder renders the consuming page. It knows nothing about the browser or the page that imports it.

`@web/page` shares the `@web` prefix but has a different backing. It is a binding-backed package for live browser state in `src/projects/html_project/binding_packages/web/page/`.

| Owner | Holds |
|---|---|
| This README | Package boundary and deferred extensions |
| `docs/src/docs/packages/builder/` | Accepted public API and semantics |
| Packages and builders progress matrix | Current implementation status and accepted next gaps |
| `docs/build-system-design.md` | Builder source-package registration and structural URL rendering |

## Current state

- `@mod.moth` is the whole package. `root` aliases the structural site root `@/`. `url` is a const-template wrapper that joins the root and its default slot.
- `src/projects/html_project/html_project_builder.rs` registers it at the nested prefix `web/site`. It needs an explicit `@web/site` dependency and isn't in `.mtf` implicit scope.
- The root follows the consuming project's configured `origin`, so `[site.url:docs/]` renders `/moth/docs/` under `origin = "/moth"`.
- The suffix must not start with `/`. `url` composes strings only. It doesn't validate routes, encode, normalise or look up the current page.

## Deferred

Route validation, encoding, `site.title` and route matching are outside V1. Runtime document access belongs in `@web/page`.
