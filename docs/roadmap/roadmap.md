# Moth Roadmap

This is the main todo list and future design / implementation roadmap for Moth.

[Lappet](../../crates/lappet/README.md), Moth's native runtime and future package
infrastructure, has its own [roadmap](../../crates/lappet/roadmap.md),
[runtime design](../../crates/lappet/docs/design.md) and
[package-manager design](../../crates/lappet/docs/package-manager.md). Its work
happens in a separate isolated worktree and is not sequenced by this roadmap.
Compiler integration consumes delivered capabilities without resuming paused
compiler work. All user-facing tooling remains under `moth`.

The next major plans are kept inside [plans](docs/roadmap/plans) and linked here in top to bottom order under the `Plans` heading.

Use the [Compiler Progress Matrix](docs/src/docs/progress/@page.moth) for language, compiler, memory and backend status.
Use the [Packages and Builders Progress Matrix](docs/src/docs/progress/packages-and-builders/@page.moth) for package and project-builder status.

---

# Plans

## Ongoing in parallel 

- [First-party Core and Builder package programme](./plans/packages/first-party-package-programme.md). Partially complete and underway - active on `packages-work` branch. **ACTIVE IN PARALLEL.**

- [Boracle research](./plans/boracle-next-research-plans) and [production borrow checker development](./plans/production-borrow-checker.md): Ongoing parallel record and production follow-ups on `borrow-checker-and-boracle-development`. Research precision and migration wait for the merged typed-expression/native-result capabilities and explicit activation. Current status, accepted implementation decisions and borrow/Boracle issues live in the production companion. **ONGOING, RESEARCH RESTART PAUSED**

- [Collecting bad diagnostics examples to be improved later](./plans/bad-diagnostics-log.md): **ONGOING**

## Sequenced work

- [Compiler source, token and diagnostic data layout](./plans/compiler-source-token-and-diagnostic-data-layout-plan.md) - **ACTIVE** on `diagnostic-data-layout-changes`: Phase 4 (compact diagnostics, type-display snapshots and frozen reports) was reactivated at `719d08efe`, followed by the failure-lane, tooling-isolation and final-handoff phases. The plan's active context capsule is authoritative.

- [Explicit place writes and signature-directed call access](./plans/explicit-place-writes-and-signature-directed-call-access-plan.md) - Queued immediately after the active diagnostic data-layout work: require `~` on existing-place writes and mutable method receivers, infer ordinary call access from parameter contracts, preserve mutable aliases and Wiring capabilities, and complete strict compiler, diagnostic, documentation and fixture cutover with an audit-driven cleanup.

- [Compiler diagnostic recovery and budgets](./plans/compiler-diagnostic-recovery-and-budgets-plan.md) - Queued after compact diagnostic layout and the explicit place-write / signature-directed call-access cutover: measured ordinary concrete-body recovery, configurable per-module error allowances, a soft project scheduling cutoff and lightweight stop metadata. Preserve parallel preparation and future incremental compatibility without adding a scheduler or cache.

- [Compiler diagnostics improvements](./plans/compiler-diagnostics-improvement-plan.md) - Paused until diagnostic layout and bounded diagnostic recovery complete. Consume delivered exact numeric conflict/default-range, narrow-operation and authored safety-catch diagnostics. Native result arity and narrow propagation-context work are delivered with the compact typed-expression foundation. Resume with a fresh inventory and only the remaining catch-recovery presentation work, without rebuilding transferred fixes or restoring obsolete explicit `~` call-argument guidance.

- [HTML builder string churn reduction](./plans/html-builder-string-churn-reduction-plan.md) - Queued, blocked on frozen path identities and five-run benchmark evidence; investigation before narrow success-path fix

- [General directives and project configuration](./plans/general-directives-and-project-config-plan.md) - Queued: consume the shared MON argument owner, all fixed numeric input types and delivered destination-aware command literal materialisation for general directives, explicit $config contracts and strict $project/$html_builder configuration

- [HTML page directives and runtime title](./plans/html-page-directives-and-runtime-title-plan.md) - Queued: root-only $page, explicit root purposes, metadata cutover and browser title capability

- [Constraints foundations and fast math](./plans/constraints-and-fast-math-plan.md) - Queued: compiler-owned `$fast_math`, transitive `$safe_math` and `$infallible`, a universal numeric backend contract V1, selected fast Core Math helpers and the corresponding documentation and optimisation work. Ordinary arithmetic remains checked. Fast arithmetic has specified wrapping and trapping while preserving numeric value invariants.

- [Runtime anonymous records](./plans/runtime-anonymous-records-plan.md) - Queued: consume delivered shared MON syntax, exact numeric types and bounded known-leaf inference. Wait for the required compact typed-expression and ordinary-struct construction capabilities. Support recursive local anonymous records through ordinary hidden nominal structs, with explicit nominal children and transitive escape checks.

- [Never return contracts](./plans/never-return-contract-plan.md)

- [Reusable generics, type-parametric Core bindings and file-backed JS/WAT](./plans/generics-core-bindings-and-file-backed-js-wat-plan.md) - Queued: file-private reusable generic declarations, closed numeric-family limits, bounded nested-call context, trusted scalar-parametric Core bindings and file-backed JS/WAT implementations. Deliver generic min, max, clamp and uniformly fallible abs. Consume exact numeric semantics and existing backend capabilities without waiting for the later mixed-Wasm implementation.

- TODO plan: Struct layout directives. Consume the delivered numeric scalar sizes, natural alignment and element strides. Define compiler-owned $layout contracts, permitted field types, aggregate alignment/padding, target validation and semantic/interface fingerprints before physical memory-layout decisions and Wasm struct lowering consume them. The nominal type model stays unchanged.

- Collector free memory implementation (see below for notes). Should have its initial implementation here before Wasm backend implementation.

- [HTML mixed JavaScript and Wasm backend](./plans/html_project_backend_wasm_final_implementation_plan.md)

- TODO plan: Structural feature selection and config-only feature declarations. Add pre-graph $feature source selection, declared feature names, builder identity predicates and selection-aware graph/interface/cache validation. Promote this to a hard prerequisite before enabling multi-builder projects.

- TODO plan: Export directives. Replace export: with compiler-owned $export while preserving module-root public surfaces and re-exports. Decide prefix-only versus optional directive-block form before implementation. Keep one final visibility syntax.

- TODO plan: Test root purpose. Design $test as a non-page consumer of normal-root top-level execution, including command selection, lifecycle, reporting, failure handling and HTML-aware testing needs. Schedule after the currently queued implementation work.

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

**Borrow-checker/Boracle living-companion exception.** The production implementation tracker remains after individual patches land and may cross-link the Boracle research handoff for navigation. They are the ongoing parallel work under one roadmap entry. Only the production tracker owns live Stage 6 implementation status, accepted production implementation choices, borrow performance evidence and actionable Boracle-to-production follow-ups. Canonical memory and compiler references still own language semantics and stage contracts. Do not duplicate this tracker in roadmap deferred notes or the progress matrix.

---

# Deferred design and follow-ups

This is a bunch of notes for work that will likely be picked up in the future, but has no set design plan yet.

## Optional catch tooling advisory

Future LSP tooling may offer an optional infallible-catch advisory after failure-summary
convergence. Keep it separate from compiler warnings and source acceptance. Authored safety
catches remain legal without an actual failure producer, and every handler still receives normal
validation before unreachable executable work disappears. Consume the same
[permanent tooling boundary](../src/docs/design-scope/deferred-and-outside-scope.mtf#numeric-and-generic-extensions).

## Collector-free memory implementation

Canonical memory design lives under [the memory management design docs](docs/src/developer-docs/memory-management).

Two temporary implementation plans remain in the tree. They are work items awaiting consolidation into one replacement implementation plan, not design authorities:

- [Final memory-management redesign](./plans/final-memory-management-redesign-and-implementation-plan.md) - temporary implementation plan covering analysis, region and planning sequencing
- [Retained Edge Counting](./plans/retained-edge-counting-design-and-implementation-plan.md) - temporary implementation plan covering REC analysis, the two-bit handle ABI, counters and lowering sequencing

Where a temporary plan and a permanent authority disagree, the permanent authority wins.

## Code-block highlighting follow-ups

The built-in `$code` formatter now supports generic and plain-text blocks plus Moth,
JavaScript, TypeScript, Python, Rust, shell, HTML, Markdown, TOML, JSON, YAML, CSS, C and
SQL profiles on one shared role palette. The current baseline includes:

- an allocation-conscious single-pass byte-slice scanner
- compiler-owned Moth source-word classification
- maximal-munch Moth operators
- a general language-neutral palette shared by every profile
- bounded Moth lexical and contextual roles for contracts, functions, directives, paths and `io`
- plain Moth template bodies around highlighted heads and interpolations

Future formats should extend the single `CodeLanguage` owner in
`src/projects/html_project/styles/code.rs`, including its aliases, comment syntax,
keyword/type rules, supported-values diagnostic and focused formatter tests.

Suggested extension order:

1. C++, Go and Java when real documentation needs justify maintaining their highlighting
   profiles.

Prefer the conventional short and long aliases where both are widely used, such as `cpp`/`c++`.
Only add a profile when its language-specific rules improve on the generic formatter; preserve
HTML escaping and add tests for aliases, comments, keywords and the rendered span classes.

Directive-aware body highlighting, such as colouring a `$code` or `$md` body by the language its
directive will format, remains deferred. Full semantic or editor grammar
parity stays owned by editor tooling, not the compile-time formatter. The built-in formatter never
performs semantic symbol resolution or syntax diagnostics.

## Genuinely deferred items

- final builder selection syntax and a possible Moth-native build script system
- package acquisition, admission and registry work is tracked only in the [Lappet roadmap](../../crates/lappet/roadmap.md)
- persistent artefact serialisation and precompiled package caches
- explicit output transformation pipeline syntax
- cross-page browser chunk sharing beyond physical variant reuse
- direct normal-sibling dependencies if real project evidence justifies them
- later Wiring observation, route application and UI/runtime design beyond the bounded foundation
- additional target builders and capability surfaces
- profiling-backed frontend optimisations and the deferred post-TIR template investigations linked above
- future Component Model integration
- external non-scalar constant design: string slices, collections and opaque-type external constants in const contexts
- private const and config follow-ups: temporary-local reduction and constant propagation
- `moth new` follow-ups: non-interactive `--default`, template selection, project type aliases, richer scaffold presets and optional package or dev tooling setup
- benchmarking and profiling deferred tooling: CI performance gates, public dashboards, source-backed package HIR caching, ownership, drop and ABI specialisation, JS minification and tree shaking, package-manager caching, broad Criterion benchmark suites, tracing and allocation profiler integrations, and tracked-summary counter expansion

## Directive follow-ups

- `$async:` is the accepted future compiler-owned async block spelling. Semantics remain on the public Async draft and Wiring channel contract, not this TODO list.
- `$checked:` is the accepted future compiler-owned proof-budget block spelling. Proof-tier algorithms remain an open proposal under Design Scope and the checked-proof-budget research pages.
- `$export` block support is undecided. The queued export-directive TODO owns the later choice of prefix-only versus optional block form. Keep one final visibility syntax.
- A possible separate custom-metadata directive remains open design. `$project` stays a closed predefined-field signature in the meantime.

Permanent owners: `docs/src/docs/directives/directives.mtf`, `docs/src/docs/design-scope/`, `docs/src/docs/async/@page.moth`, and the checked-proof-budget references. Do not treat these bullets as implementation-plan files.

## Wiring follow-ups

Wiring replaces Reactivity V1. The compiler implements the bounded foundation: Wire and Route
parameter contracts, Wire identity forwarding and contextual constructor routes. Later
observation, route application and UI runtime work require their own accepted contracts. The
former reactivity follow-up list does not authorise restoring `$bind`, reactive binding modes or
subscription expansion.

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

## Source-package JavaScript provider identity

Build-system provider discovery and HTML JavaScript linking own this deferred implementation
slice. Different source boundaries that provide different bytes at the same logical `binding.js`
path still collide. The [retained collision report](./evidence/js-import-linking-and-failure-reporting-review.md#independent-provider-identity-collision)
owns the reproducer and baseline comparison, which establishes a pre-existing defect.

Carry the existing canonical stable source-package identity through the provider request, binding
identity, runtime-import identity and provider resource origin/output placement. Reuse the
injective package-prefix owner, preserving project-local output policy, alias independence and
distinct byte sources. Skipping duplicate registration cannot preserve both implementations.

Acceptance requires distinct providers for project/package and two-package same-path inputs,
correct canonical ID resolution, exact runtime values from both providers, normal output-conflict
reporting and deterministic provider/wrapper order across legal creation and registration orders.
This work remains deferred and does not reactivate paused programmes.

---

# Future Design Notes

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
