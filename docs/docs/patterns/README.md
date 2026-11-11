# Moth Patterns: authoring and design guide

This README guides humans and agents editing `docs/docs/patterns/`. The pages
are a user-facing introduction to building larger Moth programs and an
**authority for intended composition patterns**. They describe the accepted
final design, including work not yet implemented. They are not an alternate
language grammar or a status matrix.

## Why this section exists

Show the whole programming model with realistic, modest examples. A Moth
website is ordinary modules, types, functions and templates, plus builder-owned
capabilities. Its component library should be possible in normal Moth source.
A developer ought to see where each piece lives and why, without first
learning a separate framework.

Organise the reader's journey around:

1. `websites/`: entries, module roots, scoped support packages and shared content.
2. `components/`: template composition, const functions, typed component
   values and builder-owned resource assembly.
3. `browser-behaviour/`: runtime DOM operations, explicit activation and
   future typed event delivery.

The `@entry.moth` files are site pages. Paired `*-basic.mtf` and
unsuffixed `.mtf` files present each pattern through the existing `doc_level`
Basic/Advanced component. Expand with another page only when a distinct
medium/large-project pattern has a useful real example. Update the landing
page, docs index, sidebar, pager and this map together.

## Architectural direction

### Source is the composition system

- Use normal nominal types, values, const functions, functions, collections,
  choices and templates. Do not invent a `$component` directive, macros,
  closures or a second HTML-specific expression language.
- Use local `$slot` / `$in` for immediate template composition. Ordinary
  slots do not search ancestor templates and their contribution routing does
  not escape bindings or function results.
- Use const functions to construct reusable values and whole pages. One
  ordinary component-result record such as `HtmlPart` can carry body HTML
  and explicit resource dependencies. An `html_page()`-style const function
  composes these records. The helper names and exact signatures are
  **illustrative proposals**, not fixed shipped `@html` API.
- Direct entry templates `[...]` are const-required by default. Only
  `~[...]` creates a runtime top-level fragment. `#[]` is removed. An
  ordinary `#=` binding still declares a compile-time value.
- Use plain `.mtf` content assets for authored Markdown and reusable
  static text. Their implicit `content` constant uses the fixed HTML template
  prelude. They do not inherit declarations from a containing entry.

### Builders own output sinks

- Only an active `@entry.moth` can supply direct top-level builder slots.
  Its direct `[$in title:...]`, `[$in stylesheets:...]` and other
  registered destinations are explicit output contributions, not ordinary
  ancestor slot lookups.
- Component authors declare the resources their result needs. The page
  explicitly forwards them to the builder. Builders own resource validation,
  output placement, URL contexts and stylesheet deduplication by stable
  resource origin. Source Strings do not compare unresolved resources by
  rendered URL.
- The current HTML `stylesheets` slot accepts exactly one tracked CSS
  resource per insert. Passing a concatenated structural String containing
  several CSS origins is an **intended extension to investigate** and is not
  current source acceptance. Examples using it must say so. A collection-
  valued builder handoff is an alternative still open, not a settled syntax.
- The current `head` slot deduplicates *complete direct contributions*
  after URL rendering. Concatenating several head snippets into one String
  loses the per-snippet boundary: do not claim independent head/script
  deduplication. A final component representation must preserve required
  contribution boundaries or explicitly keep head contributions separate.
- Reusable CSS belongs in imported `.css` resources. Inline `$css` is
  for a small instance-specific style value, not a general stylesheet
  injection mechanism. Avoid raw script and stylesheet tags as an API for
  component behaviour.

### A project has explicit ownership boundaries

- The selected project directory is its build and source root. Optional
  `config.moth` is build-only.
- A directory may have `@mod.moth` for a normal public API,
  `@package.moth` for its scoped support API and `@entry.moth` for
  executable page work. These are the only authored module-root filenames.
- Public declarations use the `$export` source modifier in the appropriate
  facade. Entry roots do not export and imports do not activate entry work.
- Use one scoped support package for UI components reused by several entries
  when its visibility topology permits it. Do not imply that sibling support
  packages can import each other or that entries can import another entry.
- Source paths in `.moth` resolve from their owning module root. Build a
  project tree that obeys the current support-package visibility rules.

### Runtime is explicit and typed

- Constructing or rendering an HTML String does not mount a DOM object, create
  a subscription, start a listener or trigger a rerender. Ordinary runtime
  functions activate behaviour after the page's elements exist.
- `@web/page` owns page location and navigation capabilities. The intended
  `@web/dom` owns generic synchronous DOM operations with opaque handles,
  expected-absence handling and checked host failures. The documentation
  sidebar's navigation algorithm should become Moth source rather than a
  permanent JS helper.
- The intended `@web/events` requires Channel delivery, structured async,
  host-to-Moth ingress, scheduling and lifetime ownership. Browser events
  become ordinary typed messages. `Route of ...` supplies constructor
  completion, **not** a destination or callback. `Wire of T` preserves a
  binding's identity, **not** a subscription.
- There is no accepted final user-facing event subscription API yet. Do not
  fabricate concrete `listen()`, `mount()`, `on_change()` or callback
  signatures and call them official. Show the message/data flow as a diagram
  or labelled conceptual code instead.
- Prefer native HTML/CSS interactions for simple states (for example a
  checkbox-driven menu) over unnecessary DOM scripts. Browser bindings
  provide generic primitives and source packages own application logic.

### Reliability and performance remain ordinary Moth semantics

- Typed parameters, nominal records, choices, explicit mutation and mandatory
  borrow/lifetime-topology validation apply to components and event state.
  Neither a foreign DOM handle nor host GC gives Moth code unrestricted
  ownership or longer lifetimes.
- Use `Error!`, `!`, `catch` and Options for their established contracts.
  Missing DOM targets and failed operations should be distinguishable.
  Assertions express invariants, not routine missing browser elements.
- Const functions must be deterministic and statically eligible. Runtime-only
  DOM and event operations belong in ordinary functions.
- Constant templates and function results retain structural resource and
  site-root pieces. Resolution belongs to the consuming builder context.
- Avoid broad hidden framework state, implicit ambient slot effects and
  automatic component lifecycle semantics. A later runtime component system
  must earn its complexity from real applications.

## Writing pages

Write for a programmer who has completed the language basics and is now
organising a real project. Follow the Rust book's progression: a small useful
example first, then explain what it teaches. Keep sentences short and name
the concrete owning file or function. Use ASCII punctuation, no Oxford commas
and no Markdown tables. Avoid repetitive caveats in every paragraph.

Each pattern page should contain:

- A short entry introduction explaining what the pattern builds and why.
- A **Basic** example showing a plausible directory layout or small program,
  the few essential imports and how a page uses it.
- An **Advanced** explanation of dependency boundaries, tradeoffs, relevant
  compiler/builder ownership, error/lifetime behaviour and what to avoid.
- Links to the canonical language, packages and project docs for exact rules.
- A clear local note before conceptual code that uses unimplemented features.
  Use present tense for settled semantics. Use "planned", "proposed" or
  "illustrative" for unsettled package APIs.
- A meaningful next-page link or entry pager.

Prefer complete, idiomatic examples over disconnected signatures. A code block
should have one clear purpose, no obsolete syntax and no invented type
conversions. A proposed declaration can be useful, but label it as proposed and
do not imply it compiles on the current alpha. Avoid offering two equally
prominent ways to do the same common task.

## Sources of truth and review checklist

Before editing technical content, read `AGENTS.md`, the current cheatsheet,
the relevant unsuffixed references and the current roadmap/progress matrix:

- `docs/docs/project-structure/`, `docs/docs/packages/` and
  `docs/build-system-design.md` for module/facade/root topology and output.
- `docs/docs/templates/`, `docs/docs/constants/` and
  `docs/compiler-design-overview.md` for TIR, constant semantics and slots.
- `docs/docs/directives/directives.mtf` for directive attachment/ownership.
- `docs/docs/wiring/wiring.mtf` for Wire, Route and Channel constraints.
- `docs/docs/packages/builder/` and
  `docs/docs/packages/external-binding-contracts.mtf` for @html, @web/page,
  DOM prerequisites and JS ABI limits.
- `docs/developer-docs/memory-management/overview.mtf` for safe sharing,
  lifetime ownership and builder/host boundaries.
- `docs/roadmap/` for accepted work that has not reached implementation.
- `docs/docs/progress/` for what the compiler can run today.

This section is the **pattern-level design direction**, not a replacement for
canonical feature syntax or build authority. When a proposal conflicts with
a settled contract, correct the pattern or explicitly record the gap for
design review. Never silently change a language rule in an illustrative guide.

Before publishing changes, confirm:

1. Syntax uses `[...]` / `~[...]` for direct entry fragments, `#|...|`
   for future const functions and `$export` with the three current roots.
2. Code imports legal packages through their correct facade and respects
   same-module versus cross-module visibility.
3. `.mtf` examples use documented source/formatting rules and links.
4. Examples do not promise automatic component resource hoisting or
   event subscription before the underlying builder/runtime contract exists.
5. DOM behaviour remains ordinary runtime work, after static output exists.
6. Basic and Advanced sections complement rather than repeat each other.
7. Sidebar and entry links resolve to real pages. Run the documentation gate
   from `docs/developer-docs/style-guide/validation.mtf` when available.
8. Any new public API suggestion is explicitly labelled and discussed with
   the project lead before it becomes an accepted package contract.

## Planned next work

This guide intentionally precedes two separate design/implementation plans:
one for a minimal `@web/dom` synchronous package and one for deferred
`@web/events`/Channel host ingress. Do not treat their implementation as part
of this documentation PR. The component library API is also a future
`@html` source-package design task.
