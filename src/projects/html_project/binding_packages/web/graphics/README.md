# `@web/graphics`

`@web/graphics` is the HTML builder's WebGL2 package. It gives games, simulations and 3D applications direct control over GPU resources, render state and draw calls, and it's the low-level foundation that higher-level Moth graphics and game libraries build on.

This README is the package's living development document. It holds design direction, binding prerequisites, the implementation plan and open questions. Labelled ideas are tentative.

| Owner | Holds |
|---|---|
| This README | Living design, development plan, investigations and future ideas |
| `docs/src/docs/packages/builder/graphics/` | Accepted public API and semantics |
| Packages and builders progress matrix | Current implementation status and accepted next gaps |
| `docs/compiler-design-overview.md`, `docs/build-system-design.md` | Cross-package compiler and build architecture |
| `docs/src/docs/packages/external-binding-contracts.mtf` | The annotated JavaScript binding contract the package is written against |

## Current state

- `../mod.rs` registers `@web/graphics` as a Builder-origin binding package with no public symbols.
- `graphics.js` exports nothing. Registration still parses it on every HTML build, so the asset can't drift out of the accepted annotation subset.
- The package is HTML-JS only. A future Wasm path follows the same rule as `@web/canvas`: reachable calls are rejected before lowering until a lowering exists.
- No API slice has started. Every Phase 1 operation depends on at least one binding prerequisite below.
- Fixed `U32` and `F32` scalar slots, flat named `U32` constants and compiler-owned error-code exports are delivered in the shared boundary. Shared opaque type references, nested annotation paths and bulk collection/data mapping remain unresolved.

## Role and boundary

The package stays close to WebGL2. Developers keep control of GPU resources, rendering state and draw operations. The wrapper removes Web API awkwardness without hiding the graphics model.

It isn't a scene graph, game engine or Three.js equivalent. Meshes, materials, cameras, transforms, render graphs, scene graphs, shader abstractions, asset management, batching and game-specific rendering belong to higher-level packages.

**Design quality test:** a higher-level package can add any of those without changes to `@web/graphics`, and a user who ignores every higher-level library can still write a complete WebGL2 renderer directly against it.

### Package areas

```text
@web/graphics
    graphics.gl.*      WebGL2 graphics interface (initial implementation area)
    graphics.frame.*   browser animation-frame scheduling (accepted scope, async-gated)
```

### WebGL version

WebGL2 is the only target. It's the baseline contract and lets the package expose one coherent modern API. WebGL1 compatibility is outside current scope. Reconsider it only for a concrete use case.

### Relationship to `@web/canvas`

`@web/canvas` owns canvas discovery, creation, sizing and management, Canvas 2D rendering, and images and other 2D resources. `@web/graphics` owns WebGL2 rendering. WebGL never moves into `@web/canvas`.

An idiomatic program obtains a canvas through `@web/canvas` and passes that handle to `@web/graphics` to create a context. The package must also work without `@web/canvas` for projects that author their own canvas HTML or reach the rendering target another supported way.

## Design direction

### API character

The API is curated, not a mechanical translation of the WebGL2 method list. WebGL2 concepts stay recognisable: context, shaders, programs, buffers, vertex arrays, attributes, uniforms, textures, samplers, framebuffers, renderbuffers, transform feedback, queries, sync objects, drawing and state.

The wrapper improves naming consistency, type safety, error handling, resource identity and operation shape. It replaces awkward JavaScript overloads with separate clear functions and rejects or separates unsafe or ambiguous combinations Moth can see.

It doesn't hide resource lifetime, shader compilation, program linking, buffer binding and upload, render state, draw calls, framebuffer use, texture configuration or WebGL state that's semantically meaningful. A higher-level package must be able to build a retained rendering model on top without fighting hidden policy.

### Resource model

Each WebGL resource family is a distinct opaque handle type: context, shader, program, buffer, vertex array, texture, sampler, framebuffer, renderbuffer, uniform location, query, sync object and transform feedback. There's no generic untyped graphics handle, so mixing families is a type error at the language boundary.

Families with meaningful explicit deletion get deliberate teardown functions. Browser garbage collection isn't part of the public contract. Ownership and post-delete behaviour are open questions below and must be settled before the first resource family ships.

### Errors

Use ordinary Moth failure where an operation has a result the program can act on: context acquisition, shader compilation, program linking, host-fallible resource creation and wrapper-level invalid operations that can be diagnosed coherently. Shader and program failures carry the host's info log as the error message, so nobody reproduces the JavaScript sequence that retrieves it.

State-changing calls aren't mechanically `Error!`. Wrapper validation needs one deliberate contract that decides which failures become immediate Moth errors and which low-level error and state inspection stays available for advanced use. Check the Khronos WebGL2 specification for each low-level error route before fixing that contract. Keep each verified spec fact apart from proposed wrapper policy below.

### Constants and closed domains

The public API isn't built from unexplained numeric WebGL constants. Primitive modes, buffer targets and usages, texture formats, compare functions, blend factors and similar closed domains should be clearly named Moth forms backed by named `U32` constants. That carrier is delivered under prerequisite 3. Where the final form needs more than a named constant, that part of the surface is deferred. No temporary string or opaque stand-ins.

### Numeric precision

WebGL consumes 32-bit floats for vertex data, uniforms and clear values. Fixed `F32` gives an exact profile-independent binary32 slot where the API needs it. Moth `Float` follows the boundary-wide numeric profile, so Float64 programs round at the host boundary. The canonical reference must state that rounding when the first `Float`-carrying operation ships.

### Async and callback boundary

The package is synchronous. It adds no JavaScript callbacks, promises, asynchronous image completion, retained Moth closures, timer or animation callbacks, or general host-event listeners. Synchronous WebGL2 functionality is used normally.

## Binding prerequisites

Each prerequisite below is a capability of the shared binding boundary, not a graphics-local workaround. Each accepted design belongs in `external-binding-contracts.mtf` and the compiler or build authority that owns it. Delivered prerequisites stay delivered through those owners. The graphics API waits for the unresolved shape rather than shipping around the gap.

### 1. Shared canvas identity

Unresolved. Context creation needs the canonical `CanvasElement` owned by `@web/canvas`.

- An opaque external type's identity is its package, symbol path and origin (`CanonicalBindingSymbolIdentity` in `src/compiler_frontend/external_packages/ids.rs`). Compiler type equality uses the registered `ExternalTypeId`.
- Annotated JavaScript resolves a signature type name only against built-in scalars and `@moth.opaque` declarations in the same file (`validate_type_name` in `src/projects/html_project/external_js/parser/mod.rs`). A second `@moth.opaque CanvasElement` in `graphics.js` would mint a different, incompatible type.
- Moth source already crosses this boundary. `packages/html/@mod.moth` names `@web/canvas` handle types in its own signatures, and source re-exports preserve canonical identity.

Candidate routes, tentative:

- **Cross-package type references in annotated JavaScript.** A declared reference to another registered binding package's opaque type, resolved through canonical binding identity when the builder registers packages in dependency order. This keeps one owner for `CanvasElement` and fits the request to share real identity.
- **A shared browser-element owner.** A lower `@web/*` owner for element handles that both packages consume. It needs the same cross-package resolution, so it's a later refinement rather than an alternative.

Rejected: a duplicate handle type, conversion helpers between handle types and weakening package identity rules for this one API.

Open: how a project without `@web/canvas` supplies its canvas. Element lookup by id returns the same canonical handle type, so it depends on the same resolution and doesn't need a second handle.

### 2. Nested namespaces

Unresolved. `graphics.gl.*` and `graphics.frame.*` need nested symbol paths. The registry supports them, and `@core/io` registers `io.input.*` through the path-aware Rust API. Annotated JavaScript can't: annotation names are single identifiers (`parser/comment_extractor.rs`) and `register_parsed_js_module` uses the single-component registration calls.

Tentative preference: let annotations declare a nested Moth path so `graphics.js` stays the single owner of its signatures. Rust path registration for a JavaScript-backed package would split signature ownership between Rust and the asset.

### 3. Closed-domain values

Delivered for the carrier this package uses. `@moth.sig` accepts fixed `U32` and `F32` in scalar parameters and success results through the shared fixed-scalar owner (`ExternalAbiType::Fixed(FixedScalar)`). `@moth.const` exposes one flat Moth name with fixed `U32` type from its authored literal without running JavaScript (`ExternalConstantValue::Fixed(FixedScalarValue)`). The binding contract owns the exact literal form, the rejection list and the constant-only emission rule. Native `Int`, `Uint` and `Float` keep their profile-selected meaning. The shared boundary canonicalises integer zero and rounds `F32` results once at binary32 while preserving signed zero. Closed WebGL domains cross as named `U32` constants and those names don't make domains nominally distinct. Nominal foreign enums stay pending in the binding contract and this package doesn't wait for them.

### 4. Bulk numeric and byte data

Unresolved. GPU data upload is central, so a clean data path is required for a genuinely useful v1. Annotated signatures still reject collections and options. Fixed `U32` and `F32` scalars and flat `U32` constants are delivered under prerequisite 3 but they don't carry bulk arrays. Operations that need the path:

- vertex-buffer and index-buffer upload and partial update
- texture pixel upload
- uniform arrays and matrices
- buffer and pixel readback

Design questions:

- whether a Moth collection crosses as a copy or a borrowed view, and how borrow rules bound a view's lifetime
- element-type mapping to typed arrays, including how profile-selected `Float` maps to `Float32Array`
- keeping numeric `U8` distinct from `Byte` octets, which share storage but aren't interchangeable
- readback into caller-owned mutable collections
- how the same contract later maps to Wasm linear-memory views

Rejected: opaque data handles that hide collections or byte sequences to avoid extending the boundary.

### 5. Compiler-owned error codes

Delivered. `@moth/runtime` exports named compiler-owned codes generated from `BuiltinErrorCode` (`RUNTIME_ERROR_CODE_EXPORTS` in `external_js/runtime_module_registry.rs`), and `canvas.js` now imports its host codes by name. Graphics uses the same route and adds a new code only alongside the API that first returns it, never as a speculative catalogue.

## Implementation plan

Each phase is a coherent workflow. Phases that ship public symbols update the canonical reference, the progress matrix and this README.

| Phase | Work | Needs |
|---|---|---|
| 0 | Settle unresolved prerequisites 1, 2 and 4 in their owning authorities and implement the binding capabilities | User design review per unresolved prerequisite |
| 1 | Context acquisition, drawing-buffer size, viewport, clear colour and buffers, capability and context-loss inspection | 1, 2 |
| 2 | Shaders and programs with compile and link diagnostics, program use | 2 |
| 3 | Buffers, vertex arrays, attributes, instancing state, array and indexed drawing | 2, 4. First triangle-capable slice |
| 4 | Uniforms including matrices and sampler bindings, textures, samplers and mipmaps | 2, 4 |
| 5 | Depth, stencil, blend, cull, scissor and mask state, framebuffers, renderbuffers and completeness, instanced drawing | 2 |
| 6 | Transform feedback, queries, sync, uniform buffers, pixel buffers, multiple render targets, readback | Phases 1 to 5 coherent |

Useful v1 means phases 1 to 5: a complete textured, indexed and instanced renderer written directly against the package. Richness comes from coherent workflows, not API counts.

`graphics.frame` waits for the async and channel scheduler foundation or another explicitly accepted host-reentry design. It's never implemented as a polling handle. A synchronous `frame_ready()` can't help, because Moth would already need an execution opportunity to poll it. Its design must cover browser-to-Moth wake-up, page and frame lifecycle, cancellation, repeated scheduling, re-entry while Moth is running, error propagation from later frame work, JS and Wasm target partitioning and page-local runtime ownership.

## Open questions

- **Post-delete use.** Check `deleteShader` with an attached shader, its later `detachShader` and program teardown, `deleteBuffer` with a bound buffer and later bind or update calls, and vertex-array deletion followed by reuse. Record each operation's Khronos-specified behaviour separately before deciding whether the wrapper tracks deletion, returns an error or consumes a handle.
- **Context ownership.** Check foreign-context handles in `attachShader`, `useProgram` and `bindBuffer`, and a uniform location used with another program. Keep those host rules separate from any proposed context or program identity checks in Moth handles.
- **Context loss.** Check `getError`, `isContextLost`, resource-creation results and readback operations during loss separately. Use those operation-specific facts to decide what the wrapper reports and how programs inspect loss without callbacks. A blanket no-op rule doesn't define the contract.
- **Binding state.** Decide which bind points stay explicit and where a bind-free helper removes awkwardness without hiding meaningful state.
- **Testing.** The integration harness has no browser host and no GPU. Likely owners are executable Node tests with a recording fake WebGL2 context, following `src/backends/js/tests/io_input_runtime.rs`, plus integration cases for reachability, type rejection and Wasm rejection.

## Deferred and outside scope

- WebGL1 compatibility layers
- callbacks, promises, retained closures and host-event listeners
- a polling `graphics.frame` placeholder
- public placeholder APIs for any unresolved prerequisite
- scene, material, camera, asset and engine abstractions
