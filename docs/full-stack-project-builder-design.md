# Moth Full-Stack Project Builder Design

> **Future design direction.** This document records the accepted architectural direction for a comprehensive Moth full-stack web project builder. It complements `compiler-design-overview.md` and `build-system-design.md` without expanding either document into application-framework design.
>
> This document does not define source syntax. Request directives, response-contract spelling, template wiring syntax, authentication APIs and other user-facing forms remain open design. Examples in this document are architectural, not proposed Moth syntax.
>
> The full-stack builder may eventually be built into the main Moth project or live as a separate Rust project that consumes the Moth compiler and build system as a library. The architecture must support either placement.

## Goals

The full-stack builder extends the existing HTML project model rather than replacing it with a separate application framework.

The intended defaults are:

- HTML first
- multi-page application routing first
- server rendering only where runtime data requires it
- static output where the same page can be resolved at build time
- progressive enhancement for browser interaction
- server-driven HTML fragments for ordinary interactive application flows
- isolated client-side Moth only where interaction genuinely benefits from local execution
- explicit typed data APIs where a client or external consumer needs data instead of HTML

The builder should make the common web path small and safe without hiding the network boundary or introducing a client component runtime as the default architecture.

## Core application model

A Moth web application is primarily a set of explicit request roots over ordinary Moth modules.

A request root is a remotely reachable application entry selected by the full-stack builder. Ordinary functions remain ordinary functions and are never exposed to the network merely because they are exported or reachable from another function.

Request roots cover three main roles:

1. **Pages** respond with a complete HTML document.
2. **Actions** respond to browser interaction, usually with an HTML fragment or redirect.
3. **APIs** respond with typed data or an explicit HTTP-level result.

These roles should reuse one request and routing system rather than becoming independent framework subsystems.

Conceptually:

```text
                         request root
                              |
             +----------------+----------------+
             |                |                |
            page            action             API
             |                |                |
       full document      HTML fragment     typed data
                            redirect          status
```

The exact source representation of a request root and its response contract is deferred.

## Response contracts

A request root has an explicit response contract. The response contract tells the builder how the result may be consumed and which runtime machinery is required.

The important conceptual response families are:

- complete HTML document
- HTML fragment
- redirect
- typed data
- status or empty response

These are semantic builder contracts, not a commitment to specific Moth types or directive names.

A complete-document response participates in page routing and document assembly. A fragment response may participate in browser DOM replacement. A typed-data response crosses a serialization boundary. A redirect participates in navigation rather than DOM insertion or typed decoding.

The compiler and builder should reject mismatched uses before output is emitted. For example, a template interaction that expects replaceable HTML must not target a typed-data route.

## Statically linked hypermedia interactions

The ordinary interactive model should follow the useful parts of HTMX while replacing runtime string coordination with compiler-known identities.

A template should be able to describe that a browser event invokes a known request root, supplies known arguments and applies the resulting HTML according to a declared update policy.

The authored program should not need to coordinate a raw endpoint string with a separately declared handler when both sides belong to the same Moth project.

The compiler and builder should be able to validate:

- the referenced request root exists
- the request is available to browser-originated interaction
- supplied values satisfy the request input contract
- the request kind permits the intended operation
- its response contract is compatible with the requested browser update
- required builder capabilities are available

The emitted browser representation can remain ordinary HTML attributes and a small generic runtime. Generated URLs, handler IDs and transport details are physical builder output rather than source-level semantic identity.

This preserves the hypermedia model:

```text
browser event
     |
     v
explicit request
     |
     v
server Moth
     |
     v
Moth template
     |
     v
HTML fragment
     |
     v
browser update
```

No virtual DOM, hidden component instance or live-template object is implied.

## Safe mutation defaults

State-changing browser requests should be safe by default.

The builder should own a conservative default policy for browser-originated mutations, including same-origin enforcement and CSRF protection where the deployment model requires it. A mutation should not become unsafe merely because the application author omitted repetitive transport configuration.

Incoming values remain hostile external input. The request boundary validates and decodes them before ordinary Moth application code receives typed values.

Authentication and authorization remain distinct from input validation. The host and builder may provide typed request context or authentication capabilities, but application policy still decides whether an authenticated caller may perform an operation.

Unsafe or unusually low-level HTTP behaviour may exist when justified, but it must be explicit rather than the default path.

## Explicit APIs instead of a separate server-function model

The full-stack model does not require a distinct general-purpose server-function primitive.

Typed API routes should be expressive enough to cover the cases that make server functions attractive:

- typed request inputs
- typed results
- generated serialization and decoding
- compiler-known route identity
- generated client-side request support
- shared diagnostics and validation
- no manually coordinated endpoint strings inside the same application

The semantic operation remains visibly remote. A request from browser Moth should not pretend to have the same behaviour as an ordinary local function call.

If client-side Moth later needs server-function-like ergonomics, the builder may generate typed client bindings over existing typed request roots. That is an ergonomic projection of the request system, not another remotely callable function category.

External consumers continue to use explicit HTTP contracts. Internal generated bindings must not make the physical route or wire format part of the request root's semantic identity.

## MPA-first routing and rendering

Normal navigation remains the primary query mechanism.

A page request loads server data when required, runs ordinary Moth application logic and produces HTML. A statically resolvable page may instead be emitted at build time.

Partial interaction should normally request an HTML fragment rather than transferring application data only for client code to rebuild the same interface.

Typed data APIs become the right boundary when data itself is the intended product, including:

- an interactive client island with substantial local state
- a non-browser client
- an external integration
- a public API
- a case where returning HTML would couple unrelated consumers to presentation

This keeps the default browser architecture small without making HTML the only possible response.

## Client islands

Client-side execution is an escalation path rather than the application baseline.

Use browser-side Moth for interaction that benefits materially from local execution, such as:

- canvas and graphics
- games
- drag operations
- rich editors
- low-latency keyboard interaction
- complex local state
- animation

The surrounding page remains ordinary HTML. Hypermedia interactions remain available outside and around islands.

The exact island declaration, hydration model and component semantics are open design. Existing Moth templates remain strings and do not implicitly become hydratable component instances.

## Server and browser target split

Server Moth and browser Moth are separate physical execution environments even when they share source declarations.

The intended architecture is:

```text
                     shared Moth modules
                            |
                +-----------+-----------+
                |                       |
          server request roots      browser roots
                |                       |
            Moth -> Wasm          HTML builder output
                |                  HTML + JS/Wasm
                |                       |
          Rust runtime host            browser
                |
          HTTP and platform APIs
```

Server-side Moth is expected to compile to Wasm and execute inside a host runtime controlled by the project builder.

Browser-side Moth continues to use the HTML builder's browser output model and may lower to JavaScript or Wasm according to the applicable target and capability rules.

Sharing source does not imply shared memory, shared runtime state or identical target capabilities.

## Rust host boundary

The server runtime host is expected to be written in Rust and use the Moth compiler and build system as libraries.

The host owns platform work that should not become ambient Moth language behaviour, including:

- socket and HTTP server integration
- TLS integration
- task scheduling and runtime execution
- request and connection lifecycle
- process environment and secret access
- authentication and session adapters
- storage and platform bindings
- deployment-specific capabilities

Moth server code sees only the typed capabilities the builder intentionally exposes.

The exact Wasm engine is not architectural. Wasmer, Wasmtime or another suitable runtime may implement the same host contract.

The exact repository placement is also open. The full-stack builder may live in the main Moth repository or as an independent Rust project. Moth's public builder API should make either choice practical.

## Request lifecycle and Wasm instances

A request may introduce a builder-defined request lifecycle root for lifetime and memory validation.

This does not require one Wasm instance per HTTP request.

The host may use per-request instances, pooled instances, long-lived instances or another execution strategy provided that it preserves:

- Moth's validated lifetime and ownership rules
- request isolation
- server capability restrictions
- deterministic request-bound cleanup where required
- absence of accidental state leakage between logically isolated requests

Instance policy is a physical runtime choice, not source semantics.

## Compiler, builder and host responsibilities

The compiler owns language meaning and validated program facts:

- parsing and typing
- canonical type and function identity
- constant and template folding
- HIR
- borrow validation
- lifetime and escape analysis
- backend-neutral link facts
- diagnostics for language and target violations

The build system and full-stack builder own application assembly:

- request-root registration and selection
- route planning
- page, action and API response contracts
- server and browser root assignment
- lifecycle roots
- target and capability plans
- static versus runtime page planning
- generated browser request metadata
- output and resource planning

The runtime host owns platform execution:

- HTTP transport
- server process integration
- host imports
- privileged capabilities
- request runtime context
- deployment adapters

These boundaries should remain explicit if the builder is moved to a separate repository.

## Network and security boundary

Every remotely callable request root is an explicit security boundary.

The final design must preserve these invariants:

- ordinary function visibility never implies network exposure
- every remotely callable root is explicitly registered or declared for that purpose
- incoming values are decoded and validated at the receiving boundary
- mutation requests use conservative browser security defaults
- authentication state does not replace authorization checks
- generated endpoint obscurity provides no security
- server-only values and capabilities cannot cross into browser output accidentally
- references, host handles and process-local identities cannot cross a value transport boundary as if they were portable data
- transport failures remain distinct from application-domain failures

## Serialization

Typed-data requests require a concrete wire representation, but the serialization format is not part of the request root's semantic identity.

The builder should be free to select an appropriate encoding while preserving the declared Moth value contract.

MON is a natural candidate for Moth-to-Moth data exchange once Moth-native MON integration exists, but this document does not require MON. The semantic boundary is typed value to typed value.

External HTTP APIs may require an explicitly selected interoperable representation such as JSON. Public API protocol design remains distinct from internal Moth transport.

## Relationship to existing HTML project semantics

The full-stack builder extends the HTML project model.

It should reuse rather than duplicate:

- Moth modules and packages
- template folding
- page and document assembly
- resource identities and placement
- target validation
- browser JS/Wasm lowering
- builder-owned package surfaces
- source-kind support
- output ownership and manifests
- development rebuild infrastructure

Templates remain ordinary strings. Static fragments stay compiler-produced folded values. Runtime templates execute through normal Moth runtime code and produce strings. Full-stack behaviour must not introduce a hidden live-template representation to make request handling work.

## Builder API requirement

This design places an important requirement on the compiler and build system even before the full-stack builder is implemented.

A Rust project builder must be able to consume Moth as a library without depending on unstable compiler internals.

The public builder boundary should eventually make it practical to:

- declare a builder capability surface
- register builder-owned source and binding packages
- register supported compiler-known source kinds
- register constrained builder directives
- register external import providers
- describe builder configuration
- compile a project through the canonical graph and compiler services
- consume immutable entry, request and package assemblies
- add builder-defined callable roots and lifecycle roots
- request validated target lowering without reconstructing compiler analysis
- consume folded builder metadata and template fragments
- plan resources and output records through build-owned services
- return output artefacts without taking ownership of compiler internals

This public boundary should be strong enough for an out-of-tree builder even if the first full-stack implementation stays in the main repository.

## Non-goals

The full-stack direction does not imply:

- an SPA-first architecture
- a required client component framework
- transparent RPC as ordinary function calls
- remotely exposing exported functions by default
- a virtual DOM
- hidden reactive state inside strings or templates
- a second application type system or serialization schema language
- direct ambient networking primitives in the Moth language
- a particular Rust web framework
- a particular Wasm runtime engine
- one Wasm instance per request
- a commitment that the full-stack builder lives inside the compiler repository

## Open design

The following remain deliberately unsettled:

- request-root source syntax
- whether request roots use directives, declarations or builder package facilities
- exact response-contract representation
- route parameter syntax
- template interaction syntax
- browser update and swap vocabulary
- authentication and session APIs
- request-context representation
- typed data serialization
- public API codec selection
- client-island declaration and lifecycle
- client hydration or resumability, if either is ever needed
- builder/server development workflow
- Wasm host ABI
- Wasm runtime engine
- server instance pooling and reuse policy
- deployment adapters
- whether the full-stack builder ships in the Moth repository or independently

These details should be designed from concrete implementation requirements rather than inferred from existing JavaScript frameworks.

## Direction summary

The full-stack Moth builder should remain an HTML application builder first.

Pages, hypermedia actions and typed APIs share one explicit request-root model. Templates may statically reference request identities so the compiler can validate browser/server wiring before ordinary HTTP machinery is emitted. State-changing browser interactions are safe by default. Typed APIs cover data-oriented client and external use cases without requiring a separate server-function primitive.

Server Moth runs as hosted Wasm behind a Rust-owned platform boundary. Browser Moth remains an explicit browser target used only where local interaction warrants it.

The result should preserve the web's visible request and HTML model while using Moth's compiler knowledge to remove stringly typed wiring, repetitive validation and avoidable framework glue.
