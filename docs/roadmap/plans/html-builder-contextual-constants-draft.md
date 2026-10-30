# Builder-declared contextual constants - deferred design draft

Status: research only, deferred
Activation: reconsider only after the working `@web/site` and `@web/page` V1 packages and the docs sidebar runtime migration provide evidence of a real cost or usability problem
Implementation: none authorised by this draft

## Motivation

Some builder-produced values are deterministic for one output artefact, but not known while reusable source-backed packages or their constant templates are compiled. Examples include the generated page route, effective initial page title, canonical URL and a test of whether a route matches the generated page. Today, such UI can be constructed by passing explicit values into constant-template slots or by using an HTML-builder runtime JavaScript package such as `@web/page`.

A future optimisation might allow these narrowly declared values to resolve at output assembly, producing static HTML while preserving one shared compilation of reusable components. The motivation is reducing runtime JavaScript and repetitive caller arguments, not adding a general third evaluation mode to every language expression.

## Non-goals

- No general language-level contextual constants.
- No automatic deferral of arbitrary functions, loops, effectful calls or runtime expressions.
- No re-compilation of shared source-backed packages for each consuming page.
- No dependence of package public interfaces, module graphs, source visibility, type identities or source diagnostics on the selected output page.
- No special case solely for the Moth docs sidebar.

## Tentative constraints

1. A **builder must explicitly register** every contextual value or operation it provides. Moth source cannot arbitrarily declare a value contextual. No source-facing syntax is accepted yet.
2. Each registered operation is pure and deterministic given a single immutable builder output context. The context has a named owner, bounded lifetime and stable provenance; it never reads live browser state.
3. Ordinary constant expressions retain their current meaning and fold as early as possible. Only explicitly recognised contextual operations may defer. There is no automatic dependency-based promotion of unrelated expressions.
4. A contextual result must be fully resolved before publishing an output artefact. A value escaping into a context without an eligible builder output target receives an informative diagnostic.
5. Shared package compilation and the public const-template representation stay independent of any consuming artefact. If contextual operations are carried through TIR or folded strings, they use one typed, portable mechanism and one existing semantic owner, not a separate template interpreter.
6. Builder-specific policy remains outside the frontend. The compiler may model an explicit contextual operation as a capability but must not embed HTML route semantics in generic AST/HIR stages.
7. Output-specific resolution must have a clear cache/fingerprint invalidation contract and bounded work proportional to actual specialisation needs.

## Research directions

- Compare extending existing structural string pieces (text-only values) against a narrowly typed constant-template operation representation (which could support `Bool` route comparisons).
- Check whether context-dependent `if` needs TIR-owned conditional preservation or a post-fold specialisation stage, and how to prevent arbitrary deferred evaluation from leaking in.
- Investigate interactions with imported `PublicConstTemplate`, slot routing, structural resource URLs, title metadata fallbacks, output variants, CSS/HTML escaping and composed resource-bearing values.
- Examine whether an explicit per-page build input to reusable templates is already ergonomic enough. Determine what an optimised static solution would remove in generated JS size, startup latency, accessibility or authoring cost.
- Compare with the accepted project configuration model: `$config` resolves inputs before semantic compilation while output context, if introduced, would become available later. This is an analogy, not an API commitment or a reason to duplicate config machinery.
- Audit error/diagnostic ownership for unsupported contexts, cycles, unsupported operations, source spans and target-specific output resolution.

## Activation evidence

Reopen design only with a demonstrated reusable component and measured or documented cost that cannot be handled cleanly with explicit slot arguments, ordinary static strings or the working `@web/page` runtime API. Obtain an accepted exact public API, minimal semantic representation, test plan and compilation/assembly ownership contract before any implementation.

## Status boundary

This document deliberately proposes **no syntax**, registry flag, IR node or implementation task. It records constraints for a later design discussion. The `@web/site` and `@web/page` V1 plan must complete independently.
