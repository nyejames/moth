# Moth Roadmap
This is the main todo list and future design / implementation roadmap for Moth.

The next major plans are kept inside [plans](docs/roadmap/plans) and linked here in top to bottom order under the `Plans` heading.

Use the [Progress Matrix](docs/src/docs/progress/@page.moth) as a reference for what is currently implemented, partially complete or deferred.

---

# Plans

## Active implementation work

- [CFG timers fixes](./plans/command-timing-accounting-and-reporting-correction-plan.md)

## Queued implementation chain

- [Test-suite honesty and infrastructure hardening](./plans/test-suite-honesty-and-infrastructure-hardening-plan.md)
- [Frontend module compilation ownership cleanup](./plans/frontend-module-compilation-ownership-cleanup-plan.md)
- [Constant evaluation, static control-flow specialisation and type-system architecture](./plans/constant-folding-and-type-system-hot-path-optimization-plan.md)
- [Path values and resource linking](./plans/path-values-and-resource-linking-plan.md)
- [Growable collections infallability](./plans/collection-push-fallibility-split-plan.md)
- [Anonymous const records](./plans/anonymous-const-records-plan.md)
- [Project config and recursive schemas](./plans/project-config-and-recursive-schemas-plan.md)
- [Build configuration values and project globals](./plans/build-configuration-values-and-project-globals-plan.md)
- [Diagnostics and tokens optimised memory layout plan](./plans/compiler-source-token-and-diagnostic-data-layout-plan.md)
- Improve the `tmp/test_brackets.mtf` error example.
- [Compiler diagnostics improvements](./plans/compiler-diagnostics-improvement-plan.md)
- [Entry-local config blocks and runtime title](./plans/entry-config-blocks-runtime-title-plan.md)
- [Number and numeric semantics](./plans/number_type_numeric_plan.md)
- [Runtime anonymous records](./plans/runtime-anonymous-records-plan.md)
- [HTML mixed JavaScript and Wasm backend](./plans/html_project_backend_wasm_final_implementation_plan.md)
- [Package dependency declarations and package-manager foundations](./plans/package-dependency-declarations-and-manager-foundations-plan.md)

The roadmap order is the implementation sequence. Individual plans may state their immediate blockers, but they do not redefine the full chain.

The package dependency plan is design-gated. Its implementation remains blocked until its declaration, alias, resolver and future package-manager boundaries are reviewed and accepted.

Diagnostics may continue independently. The queued implementation chain remains ordered by hard dependency.

Do not mark a plan active unless its current-state capsule says it is active.

---

# Deferred design and follow-ups

These items are genuinely deferred. They are not current implementation work. Each item links to its owning plan or stays here only when no plan exists yet.

## Region based memory management syntax and ownership-aware Wasm completion

Canonical memory design lives under [the memory management design docs](docs/src/docs/codebase/memory-management).

Accepted end-state grouped-memory semantics live under [declared memory groups](docs/src/docs/codebase/memory-management/declared-memory-groups). Implementation sequencing and deferred follow-up live in [grouped memory implementation roadmap](docs/roadmap/plans/grouped-memory-design.md). The work remains deferred and is not automatically active or queued merely because the design is accepted. It is a likely prerequisite to final ownership-aware Wasm completion.

Build profiles may vary optional optimisation-analysis effort and physical allocation strategy. They must run semantically equivalent mandatory borrow and lifetime-topology validation and must not change source legality.

## Post-TIR template performance follow-ups

The [post-TIR `$md` and template-parser optimisation plan](./plans/post-tir-template-parser-optimization-plan.md)
is the single deferred owner for source-span template text, parse and formatter reuse, source-hash
keys, imported-constant/directive invalidation, incremental template prerequisites, profiling-gated
parallel folding and cross-owner backend string-assembly investigation. It requires profiles and a
complete semantic key/invalidation model before any cache or scheduling implementation.

The final TIR completion plan remains the historical architecture source. Broad arena and invariant
work remains in the
[frontend optimisation plan](./plans/frontend-arena-semantic-invariant-optimization-plan.md).

## Code-block highlighting follow-ups

The built-in `$code` formatter now supports generic and plain-text blocks plus Moth,
JavaScript, TypeScript, Python, Rust, shell, HTML, Markdown, TOML, JSON, YAML, CSS, C and
SQL profiles on one shared role palette. The current baseline includes:

- an allocation-conscious single-pass byte-slice scanner
- compiler-owned Moth source-word classification
- maximal-munch Moth operators
- a general language-neutral palette shared by every profile
- bounded Moth lexical and contextual roles for contracts, functions, directives, paths and `io`

Future formats should extend the single `CodeLanguage` owner in
`src/projects/html_project/styles/code.rs`, including its aliases, comment syntax,
keyword/type rules, supported-values diagnostic and focused formatter tests.

Suggested extension order:

1. C++, Go and Java when real documentation needs justify maintaining their highlighting
   profiles.

Prefer the conventional short and long aliases where both are widely used, such as `cpp`/`c++`.
Only add a profile when its language-specific rules improve on the generic formatter; preserve
HTML escaping and add tests for aliases, comments, keywords and the rendered span classes.

Stateful Moth template-body-aware highlighting remains deferred. Full semantic or editor grammar
parity stays owned by editor tooling, not the compile-time formatter. The built-in formatter never
performs semantic symbol resolution or syntax diagnostics.

## Genuinely deferred items

- final builder selection syntax and a possible Moth-native build script system
- remote package registries, fetching, version solving, lockfiles, publishing and package-manager policy beyond the design-gated package dependency foundations plan
- persistent artefact serialisation and precompiled package caches
- explicit output transformation pipeline syntax
- cross-page browser chunk sharing beyond physical variant reuse
- direct normal-sibling dependencies if real project evidence justifies them
- broader reactivity source design
- additional target builders and capability surfaces
- profiling-backed frontend optimisations and the deferred post-TIR template investigations linked above
- future Component Model integration
- ownership optimisation deferred until after GC-first correctness
- external non-scalar constant design: string slices, collections and opaque-type external constants in const contexts
- private const and config follow-ups: consume HIR const metadata in borrow checking, temporary-local reduction and constant propagation
- `moth new` follow-ups: non-interactive `--default`, template selection, project type aliases, richer scaffold presets and optional package or dev tooling setup
- benchmarking and profiling deferred tooling: CI performance gates, public dashboards, source-backed package HIR caching, ownership, drop and ABI specialisation, JS minification and tree shaking, package-manager caching, broad Criterion benchmark suites, tracing and allocation profiler integrations, and tracked-summary counter expansion

## Reactivity follow-ups

After the initial reactivity surface:

- reactive template control flow
- field and path subscriptions
- collection item subscriptions
- expression dependency tracking
- derived reactive values
- template-owned event, action and effect syntax
- `$bind(...)`
- typed component messages
- IO sink design
- fine-grained DOM updates
- nested reactive regions
- keyed loop diffing
- HTML-Wasm support

## Hash map follow-ups

After the current scalar-keyed builtin map surface:

- Wasm runtime and lowering for the existing scalar-keyed builtin map
- possible read-only map iteration only if it does not introduce `HASHABLE`, custom equality, custom hashers, mutable entry APIs or user-defined key semantics

## Collection follow-ups

After fixed collection type constraints:

- default-fill syntax such as `{...none}` and `{...0}`
- explicit fixed and growable conversion through `copy` after cast and copy hardening
- growable initial-capacity hints only if future backend work shows they are useful

## Trait ecosystem follow-ups

After the initial trait surface:

- static non-method requirements
- compiler-owned builtin conformance facts
- diagnostics and tooling polish
- broader standard trait taxonomy that keeps traits as static contracts only

## Time package follow-ups

After the first `@core/time` JavaScript slice:

- civil and calendar types (`Date`, `TimeOfDay`, `DateTime`, `TimeZone`, `ZonedDateTime`, `Period`)
- Temporal-backed JS calendar behaviour once runtime and polyfill policy is clear
- locale-aware formatting and parsing
- local time-zone lookup
- async timers, sleep and intervals after async and task design exists
- browser animation-frame integration in a web-specific package rather than `@core/time`
- Wasm and native lowerings
- higher-precision or nanosecond timestamp representation if wider numeric ABI work lands

## Deferred package-system follow-ups

After the canvas reachability refactor:

- JS-backed external package APIs
- Wasm implementations for JS-backed packages such as `@web/canvas`
- current reachability is artefact-planning correctness, not general JS tree shaking or minification

---

# Outside Language Design Scope

These surfaces are intentionally not roadmap items unless the language philosophy is explicitly changed first:

- Dynamic trait values, trait objects, dynamic trait runtime lowering, trait aliases and composition, downcasting and reflection, associated types and constants, inheritance, generic traits and methods, and blanket, conditional, negative or specialized conformance.
- `HASHABLE`, generic builtin map keys, user-defined builtin map keys, custom hashers and comparers, `Float` map keys, language-level map equality, mutable entry APIs, fixed or capacity maps, and language hashsets.
- First-class public `Result` values, exceptions, reflection and runtime type IDs, broad type-level programming, higher-kinded types, parameterized aliases, partial type application, and general macro systems.
- User-defined cast targets, generic cast targets, external opaque cast targets, generic cast traits, and broad return-type-directed conversion.
- General closures, anonymous function values, generic function values, and higher-order polymorphism. Reactivity is the constrained UI-oriented mechanism intended to cover many closure-heavy UI patterns without adding general function-value semantics.
- Source-level target, OS, architecture or backend introspection; target-conditioned imports, declarations, exports and implementation branches; builders own target selection and platform-specific lowering, and `#Config` is not an escape hatch for target identity.

---

# Future Design Notes

## Package manager ideas

The [package dependency declarations and package-manager foundations plan](./plans/package-dependency-declarations-and-manager-foundations-plan.md) owns the design-gated declaration, alias and resolver boundary. The notes below remain exploratory package-manager policy and must not be implemented before that design review.

- Should try to prevent dependency explosion as much as possible, make adding dependencies with lots of dependencies harder or discouraged.
- Idea of "Golden" packages (and silver, bronze etc):
    1. Golden dependencies have 0 dependencies themselves (outside of std or core)
    2. Silver dependencies only have golden dependencies
    3. Bronze dependencies only have silver or gold dependencies
    4. Lead dependencies do not meet these criteria and there is additional friction and checks before they can be added to a project.
- Lead dependencies may not be eligible for the future official Moth package registry and will not be supported automatically by the package manager.
- The package manager should be extremely strict about security and other things before something can become an official "package". Maybe the source code must pass a series of quality checks and be run through various bits of compiler tooling before it can be added.
