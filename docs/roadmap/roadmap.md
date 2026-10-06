# Moth Roadmap

This is the main todo list and future design / implementation roadmap for Moth.

The next major plans are kept inside [plans](docs/roadmap/plans) and linked here in top to bottom order under the `Plans` heading.

Use the [Compiler Progress Matrix](docs/src/docs/progress/@page.moth) for language, compiler, memory and backend status.
Use the [Packages and Builders Progress Matrix](docs/src/docs/progress/packages-and-builders/@page.moth) for package and project-builder status.

---

# Plans

## Ongoing in parallel 

- [First-party Core and Builder package programme](./plans/packages/first-party-package-programme.md). Partially complete and underway. **ACTIVE IN PARALLEL.**

- [Boracle research plans](./plans/boracle-next-research-plans): Resume on `boracle-research` only after merged dense HIR/native result-channel capabilities and the explicit research activation gate. Mechanical representation/extraction migration belongs to the compiler foundation, not a new precision experiment. **PAUSED**

## Sequenced work

- [Typed semantic expressions and dense compiler foundations](./plans/typed-semantic-expressions-plan.md) - **QUEUED.** Start immediately after the ongoing Dec contextual-materialisation correction and the separately planned Uint addition have merged. Consolidates the outstanding dense HIR/capacity, Wiring V1 and native result/Core constant-evaluation work. Retire Reactivity V1 first, investigate compact layouts and lifetimes, then deliver dense HIR, the coupled typed-expression/native-result cutover, reduced type/fold work, the bounded Wire/Route foundation and trusted Core evaluation. These are named capability checkpoints, not a claim that the superseded plans were implemented. Eligible package work stays active in parallel and consumes its specific merged prerequisites.

- [Compiler source, token and diagnostic data layout](./plans/compiler-source-token-and-diagnostic-data-layout-plan.md) - Resume Phase 4 onward on `diagnostic-data-layout-changes` only after explicit reactivation. Consume merged MON/numeric prerequisites including Dec and Uint, compact typed expressions, dense HIR/native result channels, V1 retirement, the bounded Wiring foundation and Core constant evaluation. Run the fresh owner/lifetime inventory, verify transferred diagnostic capabilities and preserve the locked packed-span and diagnostic decisions. The plan's reactivation gate remains authoritative.

- [Compiler diagnostics improvements](./plans/compiler-diagnostics-improvement-plan.md) - Paused until diagnostic layout completes. Native result arity and the narrow propagation-context work move into the earlier typed-expression cutover. Resume with a fresh inventory and the remaining catch-recovery diagnostics, without rebuilding those transferred fixes.

- [HTML builder string churn reduction](./plans/html-builder-string-churn-reduction-plan.md) - Queued, blocked on frozen path identities and five-run benchmark evidence; investigation before narrow success-path fix

- Improve the `tmp/test_brackets.mtf` error example.

- [General directives and project configuration](./plans/general-directives-and-project-config-plan.md) - Queued: consume the shared MON argument owner for general directives, explicit $config contracts and strict $project/$html_builder configuration

- [HTML page directives and runtime title](./plans/html-page-directives-and-runtime-title-plan.md) - Queued: root-only $page, explicit root purposes, metadata cutover and browser title capability

- [Constraints foundations and fast math](./plans/constraints-and-fast-math-plan.md) - Queued: compiler-owned `$fast_math`, transitive `$safe_math` and `$infallible`, a universal numeric backend contract V1, selected fast Core Math helpers and the corresponding documentation and optimisation work. Ordinary arithmetic remains checked. Fast arithmetic has specified wrapping and trapping while preserving numeric value invariants.

- [Runtime anonymous records](./plans/runtime-anonymous-records-plan.md) - Queued after shared MON syntax and unified numeric semantics, including fixed widths, Byte and Dec. Support recursive local anonymous records through ordinary hidden nominal structs, with explicit nominal children and transitive escape checks.

- [Never return contracts](./plans/never-return-contract-plan.md)

- TODO plan: Struct layout directives. Consume the delivered numeric scalar sizes, natural alignment and element strides. Define compiler-owned $layout contracts, permitted field types, aggregate alignment/padding, target validation and semantic/interface fingerprints before physical memory-layout decisions and Wasm struct lowering consume them. The nominal type model stays unchanged.

- Collector free memory implementation (see below for notes). Should have its initial implementation here before Wasm backend implementation.

- [HTML mixed JavaScript and Wasm backend](./plans/html_project_backend_wasm_final_implementation_plan.md)

- TODO plan: Structural feature selection and config-only feature declarations. Add pre-graph $feature source selection, declared feature names, builder identity predicates and selection-aware graph/interface/cache validation. Promote this to a hard prerequisite before enabling multi-builder projects.

- TODO plan: Export directives. Replace export: with compiler-owned $export while preserving module-root public surfaces and re-exports. Decide prefix-only versus optional directive-block form before implementation. Keep one final visibility syntax.

- [Package dependency declarations and package-manager foundations](./plans/package-dependency-declarations-and-manager-foundations-plan.md)

- TODO plan: Test root purpose. Design $test` as a non-page consumer of normal-root top-level execution, including command selection, lifecycle, reporting, failure handling and HTML-aware testing needs. Schedule after the currently queued implementation work.

- TODO plan: Moth-native MON integration and static asset builder. Schedule after every roadmap item listed above this entry. Consume the standalone `moth-mon` codec, its expanded numeric schemas and shared `moth-lexical` policies for compiler-owned `$mon` convenience, automatic schemas from ordinary Moth types, checked anonymous-record generation, explicit source encode/decode operations and backend integration. Complete still-undelivered source parity for `{=}`, Unicode escapes and contextual `::Variant` construction. Add the static `.mon` project builder through normal output ownership. Exact directive and command syntax remain design work. Standalone Rust availability delivers none of these source/runtime or builder capabilities.

- [Collection-producing and repeated option-capture loops](./plans/collection-producing-and-repeated-option-loops-plan.md) - Low priority. Add eager `{T}`-producing loops through terminal `then` and repeated `T?` option-capture loop headers, lowering both to ordinary HIR CFG and collection operations without iterator or generator abstractions.

## Adding and maintaining plans

This roadmap owns the normal serial order. A plan owns its own work and nothing else. Mark a plan as active after the plan bullet point itself, don't create a new heading or move the plan bullet elsewhere.

**Name prerequisites, do not link them.** State what must already be delivered and what it gives you - "extensionless dependency clauses and the retained path syntax table", not a path to the plan that built them. A plan file is a work item with a short life. Naming the capability keeps a plan readable after its prerequisite is gone, and lets the chain be reordered or have new work inserted without editing every downstream plan.

**Do not pin a commit before work starts.** No baseline SHA in a status block. A plan that has not begun has nothing to be a baseline for, and the pin is stale by the time anyone reads it. Establish the baseline when the plan activates and keep it in the working notes, not the committed file. Referring to a specific commit is fine once work is underway or complete, where it records what actually happened.

**Keep the status block small.** Status, current slice, blockers, next action. Blockers name the missing work, using the same capability wording as the prerequisites. Everything else belongs in Git history.

**Delete a plan in the commit that completes it.** Do not mark it complete and commit that, then delete it later - the intermediate state is a file that claims to be work and is not. Removing it in the completion commit means the commit that finished the work is also the commit that retired the plan, so Git history alone answers "when did this land". A deleted plan is recoverable if it is genuinely needed again.

**Do not cite a plan from another plan.** When two plans genuinely share a contract, that contract belongs in a canonical document under `docs/src/docs/` or `docs/*-design.md`, and both plans point there. If it is not stable enough to be canonical, it is not stable enough for another plan to depend on the wording.

**Package programme exception.** The umbrella and package-level plans under `docs/roadmap/plans/packages/` are living implementation companions rather than ordinary short-lived plans. They may link one another and specialised package implementation plans in that directory. Package-level plans remain after a current version completes so later work retains implementation rationale, quirks and blockers. Completed checklists should be removed or compressed. A specialised one-shot plan may still be retired after its durable decisions and follow-ups move into the living package plan. Canonical package documentation remains the semantic authority. The main roadmap links only the umbrella programme, and the normal deletion and no-plan-link rules still apply everywhere else.

---

# Deferred design and follow-ups

This is a bunch of notes for work that will likely be picked up in the future, but has no set design plan yet.

## MON language integration and static builder

MON means Moth Object Notation. MON syntax names the shared argument/value-construction notation. The MON format contains literal data: its reader never evaluates expressions, even when those expressions could fold at compile time. The format and standalone Rust `moth-mon` codec, also available through `moth::mon`, are delivered separately from deferred language integration.

The final roadmap follow-up adds Moth-native usage and the static MON project
builder. It consumes `moth-mon`, the shared `moth-lexical` identifier/reservation
and numeric policies, MON syntax notation and ordinary Moth types. MON literal
traversal and escape decoding remain local to the codec. This follow-up adds no
second MON parser, Moth-expression dependency for literal reading, explicit
serialisation traits or second schema language. Compiler-owned `$mon`
convenience remains accepted direction, but its exact invocation syntax stays
undefined. Source typing, automatic schema extraction, backend operations and
builder commands remain undelivered. Consume the fixed-width, profile-aware
and exact Dec schema support without reimplementing numeric materialisation.

Preserve the accepted source parity decisions: `{=}` is an empty map, `{}` remains
a collection even at a map receiving context, and `::Variant(...)` uses a known
expected choice type. MON Unicode escapes remain local to the MON format; source
Unicode-escape support is explicitly deferred. Track undelivered source
portions rather than assuming support from the data reader.

The static builder emits compile-time-known `.mon` assets through ordinary build output ownership. Evaluated runtime templates remain ordinary serialisable Strings for consuming builders. Templates acquire no hidden schema, wire, route or subscription. Resource-bearing strings require concrete builder-assigned characters before MON encoding. Applications own schema versions, migrations and resource interpretation.

Dynamic schemas, streaming, public borrowed views and additional canonical byte representations wait for concrete use cases. Canonical bytes are a serialiser policy, not a restriction on MON semantics. Publish durable contracts in the language and compiler/build authorities, with Rust tooling, source integration and builder support tracked separately in the progress matrices.

## Collector-free memory implementation

Canonical memory design lives under [the memory management design docs](docs/src/developer-docs/memory-management).

Two temporary implementation plans remain in the tree. They are work items awaiting consolidation into one replacement implementation plan, not design authorities:

- [Final memory-management redesign](./plans/final-memory-management-redesign-and-implementation-plan.md) - temporary implementation plan covering analysis, region and planning sequencing
- [Retained Edge Counting](./plans/retained-edge-counting-design-and-implementation-plan.md) - temporary implementation plan covering REC analysis, the two-bit handle ABI, counters and lowering sequencing

Where a temporary plan and a permanent authority disagree, the permanent authority wins.

## Post-TIR template performance follow-ups

The [post-TIR `$md` and template-parser optimisation plan](./plans/post-tir-template-parser-optimization-plan.md)
is the single deferred owner for source-span template text, parse and formatter reuse, source-hash
keys, imported-constant/directive invalidation, incremental template prerequisites, profiling-gated
parallel folding and cross-owner backend string-assembly investigation. It requires profiles and a
complete semantic key/invalidation model before any cache or scheduling implementation.

The final TIR completion plan remains the historical architecture source. The initial frontend arena
and semantic-invariant optimisation programme is complete; its evidence remains in
`benchmarks/frontend-optimization-results.md`, and the progress matrix continues to report the
implemented surface as `Partial`. Semantic expression/scratch separation and dense HIR
expression/place storage now belong to the queued compiler foundation above. That work also owns
unchanged-root reuse, constant handoffs and repeated type/fold work at the migrated boundaries.
The deferred post-TIR owner retains independent cache, key, source-span text and scheduling
investigations, refreshed against the delivered representation. Broad borrow-fact compaction and
unrelated parser/statement storage changes still require evidence and their own bounded scope.

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

## Boracle real-source replay gaps

The last recorded replay sweep, at checkpoint `99ab43de2`, covered 1068 sources and left two failures. The current `tests/cases/` corpus has changed since that checkpoint, so later sources are unmeasured and these counts are historical rather than a current sweep. Both recorded failures are problem-extraction defects rather than oracle defects, and both also fail the `problem` dump, so extraction and validation reject them before the oracle runs. The subcommand exists only under the `boracle` feature, so each reproduction below runs as `cargo run --features boracle -- boracle <source> --dump problem`.

- A `match` with guards can produce unsorted branch targets. The source is `tests/cases/result_match_guard_propagation_order/input/@page.moth`, and validation fails with `terminator target blocks references must be strictly sorted and unique: BlockId(12) then BlockId(7)`, so the builder's target mapping does not preserve the ascending unique order that problem validation requires.
- Runtime reactive `if` metadata can leave a local unresolved. The source is `tests/cases/runtime_if_reactive_metadata_preserved/input/@page.moth`, and extraction fails with `unknown HIR local LocalId(1)`. Reactivity retirement will classify this historical case with its owner: remove V1-only coverage and preserve any surviving control-flow contract in ordinary source coverage, rather than repair an obsolete observation model.

## Boracle reduction reachability

`reduce_problem` and `render_fixture_skeleton` are implemented, audited and covered by the reducer tests, and the operational oracle authority documents the pass order, the preserved classification and the rendered fixture skeleton. Nothing outside the tests calls either of them, so a developer who follows the disagreement workflow reduces a problem by writing a test rather than by running a command.

Making reduction reachable is a CLI slice with two parts. The Boracle dump vocabulary in `src/projects/cli.rs` needs a reduction arm, and the differential service needs bound inputs, because `service.rs` hard-codes `OracleBounds::default()`. Reduction is only useful when the caller can choose the bounds the reduced result must preserve.

No plan owns this. The bounded operational oracle plan deliberately excluded CLI changes, and its completion criteria require a reducer that preserves the disagreement class, not a reachable command.

## Genuinely deferred items

- final builder selection syntax and a possible Moth-native build script system
- remote package registries, fetching, version solving, lockfiles, publishing and package-manager policy beyond the design-gated package dependency foundations plan
- persistent artefact serialisation and precompiled package caches
- explicit output transformation pipeline syntax
- cross-page browser chunk sharing beyond physical variant reuse
- direct normal-sibling dependencies if real project evidence justifies them
- later Wiring observation, route application and UI/runtime design beyond the bounded foundation
- additional target builders and capability surfaces
- profiling-backed frontend optimisations and the deferred post-TIR template investigations linked above
- future Component Model integration
- external non-scalar constant design: string slices, collections and opaque-type external constants in const contexts
- private const and config follow-ups: consume HIR const metadata in borrow checking, temporary-local reduction and constant propagation
- `moth new` follow-ups: non-interactive `--default`, template selection, project type aliases, richer scaffold presets and optional package or dev tooling setup
- benchmarking and profiling deferred tooling: CI performance gates, public dashboards, source-backed package HIR caching, ownership, drop and ABI specialisation, JS minification and tree shaking, package-manager caching, broad Criterion benchmark suites, tracing and allocation profiler integrations, and tracked-summary counter expansion

## Directive follow-ups

- `$async:` is the accepted future compiler-owned async block spelling. Semantics remain on the public Async draft and Wiring channel contract, not this TODO list.
- `$checked:` is the accepted future compiler-owned proof-budget block spelling. Proof-tier algorithms remain an open proposal under Design Scope and the checked-proof-budget research pages.
- `$export` block support is undecided. The queued export-directive TODO owns the later choice of prefix-only versus optional block form. Keep one final visibility syntax.
- A possible separate custom-metadata directive remains open design. `$project` stays a closed predefined-field signature in the meantime.

Permanent owners: `docs/src/docs/directives/directives.mtf`, `docs/src/docs/design-scope/`, `docs/src/docs/async/@page.moth`, and the checked-proof-budget references. Do not treat these bullets as implementation-plan files.

## Wiring follow-ups

Wiring replaces Reactivity V1. The queued compiler foundation removes reactive bindings,
subscriptions and implicit live-string behaviour before expression storage changes. It implements
the accepted parameter identity and constructor-route contracts only after native result channels
and compact typed nodes are in place. Later observation, route application and UI runtime work
require their own accepted contracts. The former reactivity follow-up list does not authorise
restoring `$bind`, reactive binding modes or subscription expansion.

## Hash map follow-ups

After the scalar-keyed builtin map surface, including fixed integer and Byte keys
from the numeric checkpoint:

- Wasm runtime and lowering for the existing scalar-keyed builtin map
- possible read-only map iteration only if it does not introduce `HASHABLE`, custom equality, custom hashers, mutable entry APIs or user-defined key semantics

## Collection follow-ups

After fixed collection type constraints:

- default-fill syntax such as `{...none}` and `{...0}`
- explicit fixed and growable conversion through `copy` after cast and copy hardening
- growable initial-capacity hints only if future backend work shows they are useful

Separately deferred:

- HTML-Wasm collection lowering, owned by the [HTML mixed JavaScript and Wasm backend plan](./plans/html_project_backend_wasm_final_implementation_plan.md). Future collection lowering must preserve infallible growable push, fallible fixed push and delivered compact numeric/Byte scalar strides. Scalar storage support does not imply completed aggregate allocation or collection runtime support.

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
- higher-precision or nanosecond timestamp representation only after its own package contract is accepted. Fixed-width numeric support alone does not change Duration or Timestamp representation

## Deferred package-system follow-ups

After the canvas reachability refactor:

- JS-backed external package APIs
- Wasm implementations for JS-backed packages such as `@web/canvas`
- current reachability is artefact-planning correctness, not general JS tree shaking or minification

---

# Future Design Notes

## Package manager ideas

The [package dependency declarations and package-manager foundations plan](./plans/package-dependency-declarations-and-manager-foundations-plan.md) owns the design-gated declaration, alias and resolver boundary. The notes below remain exploratory package-manager policy and must not be implemented before that design review.

- Should try to prevent dependency explosion as much as possible, make adding dependencies with lots of dependencies harder or discouraged.
- Idea of "Golden" packages (and silver, bronze etc):
    1. Golden dependencies have 0 dependencies themselves outside first-party Core packages.
    2. Silver dependencies only have golden dependencies.
    3. Bronze dependencies only have silver or gold dependencies.
    4. Lead dependencies do not meet these criteria and there is additional friction and checks before they can be added to a project.
- Lead dependencies may not be eligible for the future official Moth package registry and will not be supported automatically by the package manager.
- The package manager should be extremely strict about security and other things before something can become an official "package". Maybe the source code must pass a series of quality checks and be run through various bits of compiler tooling before it can be added to a project.

## Possible additional constraints

The constraints foundation deliberately implements only `$fast_math`, `$safe_math` and
`$infallible`. It need not grow into a general effects system. Future restrictions require
their own design and cost justification, not reserved syntax or speculative compiler machinery.

Explore `$pure`, `$no_io` and possibly `$no_wrap` later. A no-wrap restriction could allow fast
trapping arithmetic while prohibiting possible modular overflow. Its exact proof rules are open.
Purity needs a contract for observable mutation, ambient reads and nondeterminism. Isolate
side-effect origins in designated compiler- or builder-provided packages with explicit trusted
contracts. Arbitrary opaque foreign code cannot establish absence guarantees. Moth-source
packages remain analysable through ordinary canonical summaries.

The progress matrix should retain a labelled future-design reminder, separate from accepted
implementation rows. These candidate names are not implemented, accepted directive syntax or
permissions granted to source. Preserve the restriction-oriented name **constraints**.
