# Post-TIR `$md` and template-parser optimisation plan

## Purpose

Own profiling-led performance work across Moth Templates source preparation, template text storage,
template parsing, formatting, fold scheduling, incremental reuse and backend string assembly after
the final TIR architecture. This plan may optimise each established owner, but it must not create a
second template representation, preparation pass, fold entry or AST-to-HIR boundary.

## Current-state capsule

```text
ACTIVE_PLAN: docs/roadmap/plans/post-tir-template-parser-optimization-plan.md
STATUS: queued and deferred; non-blocking for the canonical implementation chain
CURRENT_SLICE: Phase 0 - capture fresh profiles and attribute one actionable owner
FINAL_TIR_REVIEW_COMMIT: 1298da468
LAST_GOOD_COMMIT: none until the first profiling or implementation slice is accepted
BRANCH: main
IMPLEMENTATION_SCOPE: Moth Templates preparation, template parser/formatter, TIR fold scheduling, cache prerequisites and backend string assembly
ACTIVATION_GATE: representative profiling or counters must identify a material bottleneck in one named owner
```

## Required authority documents

- `docs/compiler-design-overview.md` for AST-local TIR, exact views, preparation, folding and the
  neutral HIR boundary
- `docs/build-system-design.md` for canonical source/module identity, fingerprints, dependency
  graphs, incremental compilation and output ownership
- `docs/src/docs/codebase/style-guide/style-guide.mtf`, `testing.mtf` and `validation.mtf`
- `docs/src/docs/codebase/memory-management/overview.mtf` and selected leaves when source slices,
  arenas or borrowed storage change lifetime or ownership
- `docs/src/docs/progress/@page.moth` for current support and backend coverage
- `docs/roadmap/plans/canonical-module-compilation-and-scoped-packages-plan.md` for stable source and
  module identities required by persistent reuse
- `docs/roadmap/plans/html_project_backend_wasm_final_implementation_plan.md` for backend output and
  runtime-string ownership

## Final architecture constraints

Every slice must preserve these accepted owners:

- one module-scoped `TemplateIrStore`
- exact `TirViewIdentity { root, phase, context }` semantics and the established structural and
  nested-value transitions
- `prepare_tir_view` as the sole exhaustive semantic preparation owner, with explicit `Value` or
  `ConstRequired` mode
- `fold_prepared_template` as the sole template fold entry
- folded owned strings or neutral owned `runtime_handoff` payloads as the only values crossing out
  of AST
- no TIR store, identity, view, overlay or preparation type in HIR, a backend, a compiled module
  artefact or a cache boundary outside AST
- parser, formatter, fold scheduler, incremental build system, HIR runtime append and backend output
  assembly remain distinct owners even when profiles show related costs

Do not add a cache as a speculative abstraction. A cache slice starts only after its complete key,
value ownership, invalidation graph, memory bound, diagnostic policy and deterministic replay rules
are written and reviewed.

## Current owner map

- Moth Templates and raw Markdown preparation:
  - `src/compiler_frontend/pipeline.rs`
  - `src/compiler_frontend/single_source_compilation/moth_template.rs` (the direct `.mtf` fold service)
  - `src/compiler_frontend/headers/moth_template_prepare.rs`
  - `src/compiler_frontend/plain_markdown.rs`
  - `src/compiler_frontend/headers/plain_markdown_prepare.rs`
- Template construction and body parsing:
  - `src/compiler_frontend/ast/templates/create_template_node.rs`
  - `src/compiler_frontend/ast/templates/template_body_parser.rs`
  - focused parser modules under `src/compiler_frontend/ast/templates/`
- Formatter contract and Markdown formatting:
  - `src/compiler_frontend/ast/templates/formatter_contract.rs`
  - `src/compiler_frontend/ast/templates/tir/formatter_view.rs`
  - `src/compiler_frontend/ast/templates/styles/markdown/`
- TIR view, preparation, folding and cache:
  - `src/compiler_frontend/ast/templates/tir/view.rs`
  - `src/compiler_frontend/ast/templates/tir/preparation.rs`
  - `src/compiler_frontend/ast/templates/tir/fold.rs`
  - `src/compiler_frontend/ast/templates/tir/fold_cache.rs`
- Neutral runtime handoff and HIR string append:
  - `src/compiler_frontend/ast/templates/tir/handoff_materialization.rs`
  - `src/compiler_frontend/ast/templates/runtime_handoff.rs`
  - `src/compiler_frontend/hir/hir_expression/templates/`
- JavaScript and HTML output assembly:
  - `src/backends/js/runtime/strings.rs`
  - `src/backends/js/js_calls.rs`
  - `src/projects/html_project/document_shell.rs`

Phase 0 must re-read this map and replace stale paths before implementation.

## Accepted evidence at creation

The final TIR R6C checkpoint recorded six end-to-end suites at `1298da468`. Suite averages were
14.628, 14.662, 14.674, 14.848, 14.782 and 14.794ms with no consistent regression. Representative
current means were approximately 9.010ms for template stress, 6.745ms for wrapper/slot churn,
4.056ms for pattern control flow, 14.953ms for collection/control flow and 199.144ms for docs.
`just bench-report` reported no investigation candidate.

Earlier frontend-arena evidence found the template-render-plan fixture near 7ms and docs AST
emit/finalize pressure, but it did not isolate template clone or render-plan allocation pressure.
That evidence defers broad arena conversion; it is not permission to implement this plan without a
fresh profile.

## Cache and reuse key requirements

Before any parse, formatter, fold or incremental result is reused, the reviewed key must include
every semantic input relevant to that result. Depending on the owner, this includes:

- stable project, package, module and source identity
- source content digest and relevant source spans or token ranges
- source kind (`.moth`, Moth Templates or raw Markdown) and parser grammar/version
- formatter identity/version, directive registry fingerprint, whitespace policy and formatting
  configuration
- exact template root, phase and full view context for AST-local fold reuse
- preparation mode, binding/substitution identity and const-loop limit where applicable
- imported constant values, imported directive definitions and their transitive fingerprints
- project/builder configuration and selected compiler/backend versions when they affect output
- reverse-dependency and invalidation rules for every fingerprint above

The cache owner must also define eviction or bounded lifetime, error/diagnostic replay, cancellation,
parallel access, deterministic ordering and how stale entries are detected. Binding-dependent
preparation remains uncached unless binding identity is explicit and complete.

## Non-goals

- no language syntax or template semantic changes
- no second TIR representation, store, view, preparation owner or fold entry
- no TIR identity in HIR, backends or persistent module artefacts
- no broad frontend arena migration without separate evidence in its existing owner
- no replacement for canonical module identities, fingerprints or dependency graphs
- no transfer of HTML/JS/Wasm output ownership from the backend plan
- no general JavaScript minification, tree shaking or package-manager cache
- no recorded benchmark history unless the active slice explicitly authorises it

## Phase 0 - Baseline, profile and select one owner

- Refresh the owner map against the current repository and record the reviewed commit.
- Use `just bench-check` for non-recording evidence and Samply or existing detailed counters for
  attribution.
- Cover template stress, wrapper/slot churn, control-flow templates, collection templates, docs and
  Moth Templates preparation.
- Separate source preparation, parsing, formatting, preparation/folding, HIR append and backend
  assembly time.
- Record allocation, clone or byte-volume evidence only where the current instrumentation can
  measure the named owner accurately.
- Select one bounded Phase 1-6 slice or close the investigation with no change.

Acceptance:

- one material bottleneck has an evidence-backed owner, or the plan remains deferred
- no implementation follows from aggregate wall time alone
- the selected slice names its expected metric, backtrack threshold and validation route

## Phase 1 - Source-span-backed template body text

Investigate replacing eager template-body text interning with source-span or source-slice-backed
storage only when Phase 0 attributes material allocation/copy pressure to that owner.

- Keep source ownership explicit; borrowed text may not outlive its source buffer.
- Preserve exact locations, remapping, generated/inserted text and formatter anchor semantics.
- Materialise owned text at the established boundary when a source slice cannot represent generated
  or composed content.
- Do not leak borrowed source text or TIR identity beyond AST.
- Compare retained source-buffer memory against avoided interning/copy cost.

Backtrack if lifetime plumbing, retained-buffer memory or mixed owned/borrowed paths makes ownership
less clear without a measured win.

## Phase 2 - Per-template parse and development reuse

- Start with an in-process, bounded experiment before persistent reuse.
- Key source-hash reuse by stable source identity, exact content/span digest, source kind,
  parser/version/configuration and directive semantics.
- Preserve diagnostic locations and ordering on hits.
- Invalidate on source, imported directive, parser, compiler or configuration changes.
- Reuse the current TIR construction path; never deserialize or rebuild a parallel template model.

Persistent reuse cannot land before canonical source/module identities and dependency fingerprints
exist.

## Phase 3 - Formatter allocation and output reuse

- Profile Markdown formatter allocation, temporary buffers, whitespace transforms and output
  reservation separately.
- Prefer local buffer sizing/reuse before a cache when it solves the measured cost.
- A formatter-output cache must key formatter/version, exact input text/anchors, whitespace policy,
  directives/configuration and all semantic dependencies.
- Preserve one formatter contract and one TIR formatter-view owner.
- Keep plain Markdown's non-Moth path distinct from Moth Templates `$md` formatting.

Backtrack an algorithm rewrite or cache whose hit rate, memory use or diagnostic complexity does not
beat the current implementation on representative profiles.

## Phase 4 - Incremental module/template prerequisites and invalidation

- Consume canonical module/source identities, fingerprints and reverse-dependency facts from the
  build system; do not recreate them in templates.
- Define invalidation for source edits, imported constants, imported directives, project/builder
  config, parser/formatter/compiler versions and target-relevant output inputs.
- Keep AST-local fold caches module/build scoped unless a complete persistent value format and key
  are separately accepted.
- Test direct and transitive invalidation, unchanged-source hits, deletion/rename, configuration
  changes and diagnostic replay.

This phase owns template/module cache prerequisites. Source-backed package HIR reuse and
whole-project persistent semantic caches remain with canonical module/build-system design.

## Phase 5 - Profile-gated fold scheduling and parallelism

- Prove folding is a material serial bottleneck after preparation and cache behavior are measured.
- Schedule only independent exact views; keep the module store's mutation/read phases explicit.
- Use the existing `prepare_tir_view` and `fold_prepared_template` entries.
- Preserve deterministic source diagnostics, warning order, output order and cache behavior.
- Do not add shared-store races, a second cycle detector or a scheduler-specific semantic path.

Backtrack if scheduling overhead, synchronization or deterministic replay removes the measured win.

## Phase 6 - HIR and backend string assembly

- Attribute runtime append/coercion, JS helper output, JS source assembly and HTML document assembly
  separately.
- Keep HIR runtime string operations backend-neutral.
- Keep JS runtime/helper emission with the JS backend and HTML document assembly with the HTML
  project owner.
- Coordinate Wasm string representation and ABI work with the mixed HTML JavaScript/Wasm plan.
- Prefer owner-local sizing or builder improvements; do not move backend output state into TIR.

## Phase 7 - Validation and evidence review

Every implementation slice requires:

- focused unit tests for hidden key/invalidation/identity facts
- integration cases for user-visible template, Moth Templates, docs and diagnostic behavior
- backend-specific artifact or runtime assertions when output assembly changes
- `cargo fmt --all` and the appropriate focused checks during iteration
- `just validate` for every accepted code-bearing checkpoint
- `just bench-check` plus the same representative profile that justified the slice
- an intentional recorded `just bench` series only when the active plan explicitly requests
  benchmark history
- a final audit for duplicate representation, preparation, fold, cache and string-build paths

An optimisation is accepted only when correctness is unchanged, the target metric improves
consistently, memory/complexity costs are recorded and obsolete code is removed. Otherwise revert
the experiment and retain the evidence.

## Roadmap relationship

This plan is the single owner for the deferred post-TIR source-text, template-parser, formatter,
template-cache, invalidation and fold-scheduling investigations. It is queued/deferred and does not
block canonical module compilation or the ordered implementation chain. Phases that require stable
module/source identities wait for the canonical module plan; backend assembly work waits for or
coordinates with its owning backend plan.
