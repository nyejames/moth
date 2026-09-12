# Moth Roadmap

This is the main todo list and future design / implementation roadmap for Moth.

The next major plans are kept inside [plans](docs/roadmap/plans) and linked here in top to bottom order under the `Plans` heading.

Use the [Compiler Progress Matrix](docs/src/docs/progress/@page.moth) for language, compiler, memory and backend status.
Use the [Packages and Builders Progress Matrix](docs/src/docs/progress/packages-and-builders/@page.moth) for package and project-builder status.

---

# Plans

- [Compiler source, token and diagnostic data layout](./plans/compiler-source-token-and-diagnostic-data-layout-plan.md) - Phase 1 is complete on main. Continue Phases 2 and 3 on diagnostic-data-layout-changes. Will pause after phase 3 completion and be resumed later on (see below). Its `clippy::result_large_err` removal also gates every code-bearing package phase, because `just validate` currently fails in `ci-clippy-native` on that lint plus a few ordinary lint findings in the in-flight path-table fork work.
- [First-party Core and Builder package programme](./plans/packages/first-party-package-programme.md) - Phase 0 foundations and Phase 1 workflow activation are merged. A bounded pre-checkpoint hardening slice for the five existing `@core/text` functions runs on `packages-and-builder-progress-plan`; it adds no package API. Further package phases are blocked on the native result-slot checkpoint and on the red `just validate` clippy lane owned by the data-layout plan, so decide whether a code-bearing package phase may run against that failing gate before restarting package work.
- [Wiring V1: reactivity removal and semantic foundations](./plans/wiring-v1-cleanup-and-foundations.md) - Run on its own branch after data-layout Phase 3 and before the native result-slot checkpoint. Merge the accepted work before data-layout Phase 4 resumes.
- [Native result slots and Core constant evaluation](./plans/native-result-slots-and-core-const-eval.md) - Run after the Wiring checkpoint and before data-layout Phase 4. Package implementation that requires these capabilities remains blocked until they are merged.
- [Boracle research plans](./plans/boracle-next-research-plans): Will be ongoing in parallel on its own branch `boracle-research` after native result slots plan completes.
- [Compiler source, token and diagnostic data layout](./plans/compiler-source-token-and-diagnostic-data-layout-plan.md) - Resume Phase 4 onward on diagnostic-data-layout-changes after the Wiring and native result-slot checkpoints are merged and the continuation branch is rebased onto main.
- [Automatic Markdown heading section links](./plans/automatic-markdown-section-links-plan.md)
- [Compiler diagnostics improvements](./plans/compiler-diagnostics-improvement-plan.md) - Paused until the diagnostics and tokens layout plan completes; resume at Phase 4.1c afterward
- [HTML builder string churn reduction](./plans/html-builder-string-churn-reduction-plan.md) - Queued, blocked on frozen path identities and five-run benchmark evidence; investigation before narrow success-path fix
- [Windows ci failures further investigation](./plans/test-suite-honesty-exposed-failures.md)
- Improve the `tmp/test_brackets.mtf` error example.
- [General directives and project configuration](./plans/general-directives-and-project-config-plan.md) - Queued: shared directive syntax, explicit $config contracts and strict $project/$html_builder configuration
- [HTML page directives and runtime title](./plans/html-page-directives-and-runtime-title-plan.md) - Queued: root-only $page, explicit root purposes, metadata cutover and browser title capability
- [Number and numeric semantics](./plans/number_type_numeric_plan.md)
- [Runtime anonymous records](./plans/runtime-anonymous-records-plan.md)
- [Never return contracts](./plans/never-return-contract-plan.md)
- TODO plan: Struct layout directives. Define compiler-owned $layout contracts, permitted field types, alignment/padding, target validation and semantic/interface fingerprints before physical memory-layout decisions and Wasm struct lowering consume them. The nominal type model stays unchanged.
- Collector free memory implementation (see below for notes). Should have its initial implementation here before Wasm backend implementation.
- [HTML mixed JavaScript and Wasm backend](./plans/html_project_backend_wasm_final_implementation_plan.md)
- TODO plan: Structural feature selection and config-only feature declarations. Add pre-graph $feature source selection, declared feature names, builder identity predicates and selection-aware graph/interface/cache validation. Promote this to a hard prerequisite before enabling multi-builder projects.
- TODO plan: Export directives. Replace export: with compiler-owned $export while preserving module-root public surfaces and re-exports. Decide prefix-only versus optional directive-block form before implementation. Keep one final visibility syntax.
- [Package dependency declarations and package-manager foundations](./plans/package-dependency-declarations-and-manager-foundations-plan.md)
- TODO plan: Test root purpose. Design $test as a non-page consumer of normal-root top-level execution, including command selection, lifecycle, reporting, failure handling and HTML-aware testing needs. Schedule after the currently queued implementation work.

Parallel branches rebase after the initial squash merge, after shared compiler input or representation checkpoints land and before accepting a slice that consumes those changes. Re-run the owning validation gate after each rebase. Parallel scheduling does not remove package-specific prerequisites or permit compatibility wrappers around superseded compiler APIs.

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
implemented surface as `Partial`. The remaining expression-scratch and HIR/borrow-fact
compaction investigations are deferred until profiling shows material pressure and have no current
implementation plan. Create a focused plan only when that evidence exists.

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
- Runtime reactive `if` metadata can leave a local unresolved. The source is `tests/cases/runtime_if_reactive_metadata_preserved/input/@page.moth`, and extraction fails with `unknown HIR local LocalId(1)`.

## Boracle reduction reachability

`reduce_problem` and `render_fixture_skeleton` are implemented, audited and covered by the reducer tests, and the operational oracle authority documents the pass order, the preserved classification and the rendered fixture skeleton. Nothing outside the tests calls either of them, so a developer who follows the disagreement workflow reduces a problem by writing a test rather than by running a command.

Making reduction reachable is a CLI slice with two parts. The Boracle dump vocabulary in `src/projects/cli.rs` needs a reduction arm, and the differential service needs bound inputs, because `service.rs` hard-codes `OracleBounds::default()` and exposes no bound flags. Reduction is only useful when the caller can choose the bounds the reduced result must preserve.

No plan owns this. The bounded operational oracle plan deliberately excluded CLI changes from its generator and reducer slices, and its completion criteria require a reducer that preserves the disagreement class rather than a reachable command.

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

Wiring replaces Reactivity V1. The queued foundation removes reactive bindings, subscriptions and implicit live-string behaviour. Later observation, route application and UI runtime work require explicit accepted contracts. The former reactivity follow-up list does not authorise restoring `$bind`, reactive binding modes or subscription expansion.

## Hash map follow-ups

After the current scalar-keyed builtin map surface:

- Wasm runtime and lowering for the existing scalar-keyed builtin map
- possible read-only map iteration only if it does not introduce `HASHABLE`, custom equality, custom hashers, mutable entry APIs or user-defined key semantics

## Collection follow-ups

After fixed collection type constraints:

- default-fill syntax such as `{...none}` and `{...0}`
- explicit fixed and growable conversion through `copy` after cast and copy hardening
- growable initial-capacity hints only if future backend work shows they are useful

Separately deferred:

- HTML-Wasm collection lowering, owned by the [HTML mixed JavaScript and Wasm backend plan](./plans/html_project_backend_wasm_final_implementation_plan.md). Future collection lowering must preserve infallible growable push and fallible fixed push.

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
- The package manager should be extremely strict about security and other things before something can become an official "package". Maybe the source code must pass a series of quality checks and be run through various bits of compiler tooling before it can be added.
