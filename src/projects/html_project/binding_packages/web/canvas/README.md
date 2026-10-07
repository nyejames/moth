# `@web/canvas`

`@web/canvas` is the HTML builder's Canvas 2D package. It owns HTML canvas discovery, creation, sizing and management, Canvas 2D rendering, and images and other 2D-specific resources.

This README is the package's living development document. It holds design direction, binding limits, deferred surfaces and future ideas. Labelled ideas are tentative.

| Owner | Holds |
|---|---|
| This README | Living design, development plan, investigations and future ideas |
| `docs/src/docs/packages/builder/canvas/` | Accepted public API and semantics |
| Packages and builders progress matrix | Current implementation status and accepted next gaps |
| `docs/compiler-design-overview.md`, `docs/build-system-design.md` | Cross-package compiler and build architecture |
| `docs/src/docs/packages/external-binding-contracts.mtf` | The annotated JavaScript binding contract the package is written against |

## Current state

- `canvas.js` is the whole package: seven opaque handle types and free functions covering element management, context state, rectangles, paths, text, gradients, patterns, image drawing, image data and pixel access.
- `../mod.rs` registers it as a Builder-origin binding package. Reachable calls emit `canvas.js`, generated glue and the `@moth/runtime` import map entry.
- `packages/html/@mod.moth` wraps it in the source-owned `Canvas` struct with receiver methods. Binding-backed packages can't expose receivers, so method-style ergonomics live there.
- The package is HTML-JS only. The Wasm backend rejects reachable calls before lowering.

## Role and direction

Canvas is deliberately allowed broader growth than most first-party packages. Visual and animation-heavy programs are useful compiler, runtime and language stress tests. Prefer coherent drawing, state, path, transform, text, image and pixel workflows over a mechanical copy of the Canvas API.

Grow `@html` wrappers only where a source-owned method or helper composition clearly improves Moth usage.

### Relationship to `@web/graphics`

WebGL belongs in `@web/graphics`, never here. `@web/canvas` stays the owner of the canvas element and its sizing. Graphics context creation should take the canonical `CanvasElement` from this package, which first needs annotated JavaScript to reference another package's opaque type. The graphics README records that prerequisite and its candidate routes. Don't duplicate the element handle to work around it.

## Implementation notes

- The binding ABI is concrete, synchronous and scalar or opaque only. Browser overloads and union source types become explicitly named wrappers, such as the `to_data_url*` and `create_image*` families.
- `create_image` returns the element immediately while the browser keeps loading it. Drawing stays fallible through `assertLoadedImage` until loading completes.
- Pixel channels cross as `Int` values clamped to 0 through 255 because the annotation subset has no `U8`. Any future binary-data design keeps numeric `U8` channels distinct from `Byte` octets. They share one-byte storage but aren't interchangeable, and this package doesn't settle a Core byte API or bitwise syntax.
- Handles have no teardown functions. The host JavaScript runtime manages their lifetime.

## Known gaps

- **Error codes.** Failures still hand-build the unowned 400, 404, 409 and 500 family instead of compiler-owned `BuiltinErrorCode` values. Static assets can't interpolate codes the way Core helpers do, so the fix needs a route for assets to name them, such as `@moth/runtime` exports. `@web/graphics` needs the same route.

## Deferred surfaces

Each waits for a binding capability rather than a canvas-local workaround.

- **Async image loading.** Needs callbacks, promises or an event and listener model.
- **`toBlob` and `captureStream`.** Need callback and stream values with a Moth ABI shape. `to_data_url*` is available now but returns the whole encoded canvas as one in-memory string.
- **Typed-array pixel access and arbitrary line-dash arrays.** Need collection or typed-data signatures. This is the same bulk-data prerequisite `@web/graphics` records, and both packages should consume one design.
- **`Path2D`, `DOMMatrix`, `OffscreenCanvas`, `ImageBitmap` and video sources.** Each becomes its own opaque API rather than being forced through the scalar 2D wrapper surface. Tentative considerations:
  - `Path2D` gives reusable path objects and fits the current opaque-handle model.
  - `DOMMatrix` overlaps the scalar transform calls. Decide whether it replaces or complements them before adding it.
  - `ImageBitmap` creation is promise-based, so it waits for async support.
  - `OffscreenCanvas` mainly pays off with worker rendering, which needs an accepted worker and scheduling story.
  - Video sources need a media-element owner, which is outside this package today.

## Next work

The package programme's canvas phase targets expansion across the workflows above. Its closeout needs at least one substantial visual-program integration case, runtime asset reachability checks and failure coverage for unavailable handles and invalid operations.
