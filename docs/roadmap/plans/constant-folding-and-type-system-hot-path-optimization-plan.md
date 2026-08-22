# Constant Evaluation, Static Control-Flow Specialisation and Type-System Architecture Plan

> **Repository path:**
> `docs/roadmap/plans/constant-folding-and-type-system-hot-path-optimization-plan.md`
>
> **Implementation branch:**
> `const-folding-and-types-optimisation`
>
> **Status:**
> In progress. Phase 0, Phase 1 and the shared file-visibility slice are complete. Rewritten on
> 2026-08-22 after profiling invalidated the prioritisation the original phase order was built on.

## Purpose

Build one compact, durable constant-evaluation and type-resolution architecture, remove the
dominant constant and type-system hot-path costs, and add Stage 4 static specialisation of ordinary
`if` statements whose conditions are known compile-time `Bool` values.

Two coupled outcomes:

1. Replace repeated context construction, copied semantic state, rich intermediate clones and
   redundant value representations with data-oriented module-owned stores and indexed views.
2. Make compile-time evaluation reduce ordinary Bool control flow before HIR so later compiler
   systems process only the executable branch selected for the configured build.

The design must improve the common path without replacing Moth's existing semantic owners:

- Stage 3 remains the declaration-order authority
- `TopLevelDeclarationTable` remains the indexed declaration owner
- `TypeEnvironment` remains the canonical module-local type owner
- `TypeId` remains the semantic type identity
- TIR exact views and the TIR fold cache remain the template authority
- `ConstValueStore` becomes the module-local folded-value authority
- `PublicFoldedValue` remains the owned cross-module folded-value vocabulary
- Stage 4 owns static Bool control-flow specialisation after full frontend validation
- HIR receives final folded values and already-specialised executable control flow, never TIR,
  unresolved type syntax or a statically decided `if`

Every phase except Phase G is an implementation optimisation and must preserve accepted programs,
diagnostics, public identities and emitted artefacts. Phase G is the one deliberate semantic
expansion.

---

## Current state

ACTIVE_PLAN:
- `docs/roadmap/plans/constant-folding-and-type-system-hot-path-optimization-plan.md`

BRANCH:
- `const-folding-and-types-optimisation`

COMPLETED:
- Phase 0 (baseline, scaling fixtures, semantic freeze) - commit `ccf25d166`
- Phase 1 (module-owned constant resolution session) - commit `4e421a5a8`
- Shared file visibility (taken out of sequence after profiling) - commit `917f7e81c`
- Phase A (re-baseline and attribution) - see **Phase A outcome** below

CURRENT_SLICE:
- Phase: B (shared environment side tables)
- Goal: remove the per-header deep clone of five whole-module side tables in
  `constant_header_scope_context`, measured in Phase A as `O(n^2.03)` in module size and `>99%`
  of the cost of the pass that contains it
- Non-goals: no semantic change, no control-flow change, no member-shell restructuring (that is
  Phase E, and it must be re-measured after this phase lands)

NEXT_ACTION:
- execute Phase B against the copy-on-write design recorded in that phase, then re-measure the
  nominal scaling curve before starting Phase E

PHASE_ORDER:
- A (re-baseline) -> B (shared side tables) -> C (folded-value authority) -> D (move-only folding
  and lazy diagnostics) -> E (nominal member shells) -> F (type environment and generic keys) ->
  G (static Bool `if`, mandatory semantic gate) -> H (declaration lanes and pass cleanup) ->
  I (closeout)
- Confirmed unchanged by Phase A. B was already first among the performance phases and the
  measurement promoted it further: it is the only quadratic cost found in the frontend. C and D
  keep their places because they are deletion phases whose value does not depend on the profile.
  E moved in scope rather than position - Phase B removes most of what E was going to pay for, so
  E re-measures first and may shrink to a correctness phase. F is now known to be unmeasurable at
  current fixture scale and is re-scoped in place. G, H and I keep their positions: G depends on
  C, H is clarity work that should follow the representation changes it touches, and I is the
  closeout.

RELEVANT_DOCS:
- `AGENTS.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/src/docs/design-scope/design-principles.mtf`
- `docs/roadmap/plans/build-configuration-values-and-project-globals-plan.md`
- `docs/roadmap/plans/frontend-arena-semantic-invariant-optimization-plan.md`
- `docs/roadmap/plans/compiler-source-token-and-diagnostic-data-layout-plan.md`
- `benchmarks/README.md`
- `benchmarks/frontend-optimization-results.md`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`

RELEVANT_CODE:
- `src/compiler_frontend/headers/binding_environment/`
- `src/compiler_frontend/ast/module_ast/environment/`
- `src/compiler_frontend/ast/module_ast/finalization/`
- `src/compiler_frontend/ast/module_ast/scope_context/`
- `src/compiler_frontend/ast/statements/branching.rs`
- `src/compiler_frontend/ast/statements/terminality.rs`
- `src/compiler_frontend/ast/type_resolution/`
- `src/compiler_frontend/ast/const_eval/`
- `src/compiler_frontend/ast/const_values/`
- `src/compiler_frontend/ast/expressions/expression_rpn.rs`
- `src/compiler_frontend/datatypes/environment.rs`
- `src/compiler_frontend/type_coercion/compatibility.rs`
- `src/compiler_frontend/folded_value.rs`
- `src/compiler_frontend/hir/hir_statement/declarations.rs`
- `src/compiler_frontend/instrumentation/`
- `benchmarks/manifest.toml`

---

## How this plan is run

Each phase is a checkpoint, a commit and a natural context compaction point. A phase is finished
when its work items are ticked, its acceptance holds, `just validate` passes, and its **Outcome**
subsection is written in this file. Write outcomes for the reader who arrives with no memory of the
session: what changed, what it measured, and what a later phase must not rediscover.

Rules learned from the work so far. These are not style preferences, they cost real time:

1. **Profile before choosing a representation change.** The plan's original ordering was derived
   from a Phase 0 baseline dominated by a cost nobody had attributed. Three phases of planned work
   were pointed at candidates that turned out to be `O(1)`.
2. **A counter proves the pass it instruments is clean. It says nothing about the module.** Every
   constant-pass counter read linear while two uninstrumented passes owned the whole runtime. Do
   not accept "the counters are flat" as evidence that a phase is done.
3. **Do not record a scaling claim for work that was not measured to scale.** State plainly when a
   phase is an ownership or clarity change.
4. **Record findings that contradict the plan, including the plan's own earlier findings.** The
   superseded Phase 1 candidate list is left in this file with its correction, not deleted.
5. **Deletion is the deliverable.** A faster implementation that leaves the old builder, tree or
   walker alive as a second path does not satisfy the phase.

---

## Completed work and durable findings

### Phase 0 - baseline, scaling fixtures and semantic freeze

Completed 2026-08-22, commit `ccf25d166`. Evidence:
`benchmarks/frontend-optimization-results.md`, section
`Constant Evaluation And Type-System Plan - Phase 0 Baseline - 2026-08-22`.

Assets it created, all still in use:

- Benchmark workloads `constant_chain_32`, `constant_chain_128`, `constant_chain_512` and
  `nominal_capacity_stress`, each with a `_check` and a `_frontend` case, plus the manifest
  inventory test in `xtask/src/benchmark_manifest/tests.rs` and the counts in `benchmarks/README.md`.
- `AstCounter` variants `ConstantResolutionContextsCreated`, `ConstantsResolved`,
  `ConstantPassSideTableEntriesCloned`, `ModuleConstantDeclarationClones`,
  `ExpressionOrderingInputItems`, `ExpressionTypedStackItems`, `ExpressionFoldItems`,
  `ExpressionOperandClones`, `DiagnosticDataTypeMaterialisations`, `BranchLocalGenericRequests`.
- `FrontendCounter` variants `GenericSubstitutionKeyBuilds`, `GenericSubstitutionKeySortedPairs`,
  `PublicFoldedValueConversions`, `HirConstValueConversions`, `HirStaticBoolIfNodes`,
  `HirRuntimeIfNodes`.
- Integration cases `static_if_constant_bool_branch_selection`,
  `static_if_value_producing_branch_selection`, `static_if_branch_scope_preserved` and
  `static_if_inactive_branch_generic_call`.
- Ignored intended-contract tests in `src/compiler_frontend/tests/frontend_pipeline_tests.rs`:
  `intended_compile_time_true_condition_reaches_hir_without_a_branch`,
  `intended_compile_time_false_condition_without_else_lowers_no_branch_body` and
  `intended_terminality_observes_the_selected_branch`, plus the non-ignored freeze
  `runtime_bool_condition_lowers_one_branch_diamond`. Phase G enables the ignored three.

Findings that still constrain later phases:

1. **A constant-backed Bool condition is not a folded `Bool` at HIR.** `if enabled:` with
   `enabled #= true` reaches HIR as a reference expression; only a literal `if true:` folds to
   `ExpressionKind::Bool`. Phase G must read the condition through the folded-value authority, not
   by matching `ExpressionKind::Bool`. `hir_static_bool_if_nodes` measures the literal case only,
   so it is the post-G invariant counter, not a candidate census.
2. **Reuse the existing generic-request pruning boundary.** `ScopeContext::generic_request_checkpoint`
   and `ScopeContext::discard_generic_requests_since` already exist and are used by static
   `assert(true)` message discarding in `src/compiler_frontend/ast/statements/asserts.rs`.
   `src/compiler_frontend/ast/statements/branching.rs` already brackets its branch bodies with a
   checkpoint under `benchmark_counters`. Phase G must reuse that mechanism, not add a second one.
3. **Inactive generic work is materialised today.** A generic call reachable only through a
   compile-time-false branch still emits a generated function into the artefact. Phase G's
   acceptance must assert its absence.
4. **`HirBuilder::lower_if_with_body_emitters` in
   `src/compiler_frontend/hir/hir_statement/control_flow.rs` is the single HIR `if`-diamond owner**
   for both statement `if` and runtime template `if`. It carries `record_hir_branch_condition_kind`,
   which is where the "no statically decided `if` reaches HIR" assertion belongs.
5. **Member shells are rebuilt after constants** in
   `AstModuleEnvironmentBuilder::resolve_type_declarations`. That is Phase E.
   `benchmarks/nominal-capacity-stress.moth` isolates that cost from constant count.
6. **The Phase 0 timing baseline is superseded.** It was recorded when a per-header
   `FileVisibility` copy dominated every AST workload, so its attribution shares are not usable for
   prioritisation. Phase A replaces it. The counter values it recorded remain valid as counts.

### Phase 1 - consolidate constant-resolution context

Completed 2026-08-22, commit `4e421a5a8`. Evidence:
`benchmarks/frontend-optimization-results.md`, section
`Constant Evaluation And Type-System Plan - Phase 1 Consolidated Constant Session - 2026-08-22`.

`ConstantResolutionSession` in
`src/compiler_frontend/ast/module_ast/environment/constant_resolution.rs` owns the module view the
whole constant pass reads: the five side tables, the trait environment, project services, the TIR
store, the rendered-path sink, one `TypeCompatibilityCache` and one canonical file scope per source
file. `resolve_constant_headers` drives it and commits each folded constant.
`ConstantHeaderParseContext` and `parse_constant_header_declaration` are deleted.

Supporting ownership changes:

- `ScopeContext::visible_declaration_ids` and
  `ScopeFrame::explicit_compile_time_constant_declarations` are shared copy-on-write handles. Only a
  scope that actually declares a local pays for a private copy.
- `AstModuleEnvironmentBuilder` owns one `resolved_module_constant_paths` set, updated by
  `push_module_constant` and shared by handle with the constant-header, nominal-member and
  function-signature scopes.

Findings:

1. **`ast_constant_pass_prior_constant_ids_copied` was deleted, not zeroed.** The cumulative copy it
   measured no longer exists. Do not reintroduce it to satisfy a checkbox.
2. **The declaration table's `Rc::get_mut` commit path constrains session shape.**
   `AstModuleEnvironmentBuilder::replace_declaration` requires sole `Rc` ownership, so no scope may
   hold the table across a constant commit. That is why the session still builds one `ScopeContext`
   per constant. Phase H decides deliberately whether the table gains a commit path that tolerates
   live readers. If it does, the session collapses to one root scope with per-constant child frames
   and the remaining per-constant `ScopeContext::new` disappears with it.
3. **`docs` is not a constant-setup workload.** Its constants are one or two per file. The Phase 0
   reading that constant-header resolution is about half of `frontend.ast.total` on `docs` is fold
   work owned by Phases C and D, not context construction.
4. **~~The remaining `chain_512` superlinearity is declaration-table replacement, the
   module-constant declaration clone, or constant normalisation.~~ Superseded.** All three are
   `O(1)` per constant. See the shared file-visibility findings below.

Two Phase 1 checkboxes remain open and are carried into Phase H:

- top-level constants still require a synthetic `AstModuleLookups`, built inside `ScopeContext::new`
- the constant-header `ScopeContext` builder chain still has two callers, the member-shell pass and
  the function-signature pass, which read side tables that are still being written while they run

### Shared file visibility - out-of-sequence slice

Completed 2026-08-22, commit `917f7e81c`. Evidence:
`benchmarks/frontend-optimization-results.md`, section
`Constant Evaluation And Type-System Plan - Shared File Visibility - 2026-08-22`.

Taken before Phase 2 because a profile of the residual `constant_chain_512` superlinearity
attributed effectively the whole AST pass to a cost no phase was pointed at.
`AstModuleEnvironmentBuilder::validate_nominal_generic_bound_surfaces` and `AstEmitter::emit` each
cloned a whole `FileVisibility` per header, including for constants that need no scope. On a local
4096-constant chain, 1617 of 1619 samples in the environment pass were that copy and its drop.

`HeaderBindingEnvironment::file_visibility_by_source` now stores `Arc<FileVisibility>` and
`visibility_for` returns the handle. `FileVisibility::visible_declaration_paths` is itself an
`Arc<FxHashSet<InternedPath>>`, so `ScopeContext::with_file_visibility` takes one argument and
shares the package and its declaration gate together. Binding construction writes the gate through
`FileVisibility::visible_declaration_paths_mut`, which holds the sole reference and never copies.
`Arc` rather than `Rc` because this is header-stage data read by AST, dependency sorting and
trait-evidence validation.

`frontend.ast.total`, median of five interleaved runs, `RAYON_NUM_THREADS=1`:

| Workload | Before | After |
|---|---:|---:|
| `constant_chain_128` | `5.09ms` | `1.31ms` |
| `constant_chain_512` | `61.73ms` | `4.23ms` |
| local 2048-constant chain | `875.44ms` | `17.06ms` |
| `fold_stress` | `7.24ms` | `2.89ms` |
| `environment_stress` | `10.66ms` | `4.97ms` |
| `type_stress` | `11.90ms` | `6.44ms` |
| `nominal_capacity_stress` | `15.43ms` | `11.11ms` |
| `docs` | `180.92ms` | `169.44ms` |

Findings:

1. **Phase 1's candidate list was wrong on all three counts.** `replace_by_path` is one hash lookup
   plus an indexed store, the module-constant declaration clone is `O(1)`, and module-constant
   normalisation visits exactly one expression per constant. The dense-`DeclarationId` work is an
   ownership and clarity change, not a scaling fix, and is now Phase H.
2. **Counters located nothing here.** They instrument the pass Phase 1 rewrote; the cost was in two
   passes with no counters at all.
3. **The same clone-to-satisfy-borrow shape survives in the side tables.** That is Phase B.
4. **`docs` moved for the first time in this plan**, which confirms the copy was in shared
   header-loop machinery rather than constant-specific code.
5. **`ast_constant_pass_visibility_entries_cloned` was deleted** with the copy it measured, for the
   same reason as the Phase 1 counter deletion.

---

## Accepted static control-flow contract

This section is accepted design. Phase G implements it. Changing it requires a Moth design review,
not a plan edit.

### Ordinary `if` is the only source form

Moth does not add a `#Config if` statement or a second conditional-compilation grammar. An ordinary
`if` consumes an ordinary semantic `Bool` value:

```moth
enabled #= false

if enabled:
    perform_optional_work()
;
```

The queued build-configuration-values plan defines `#Config of T`. A configured Bool enters the
same ordinary folded-constant and static-`if` path defined here:

```moth
analytics #Config of Bool = false

if analytics:
    send_analytics()
;
```

This plan does not implement `#Config`, CLI build-input parsing or project-global interfaces. It
establishes the general static-`if` behaviour that the queued plan will consume without a
config-specific AST node, branch pass or HIR operation.

### Both branches remain valid source

Static specialisation is not textual preprocessing and does not create an unparsed inactive language
island.

Before branch selection, both branches must complete the ordinary Stage 4 frontend work required by
their source form, including:

- token and structural syntax validity
- name and visibility resolution
- type checking and coercion
- generic inference and evidence validation
- cast and constant-expression validation
- value-producing branch arity and receiving-type validation
- source-level diagnostics and stable frontend warnings

An unknown name or type error in the branch that will later be inactive remains a diagnostic. Static
specialisation removes executable work, not source correctness requirements.

### Specialisation happens before executable compiler systems

After both branches are frontend-valid and the condition has a final folded value:

- a known `true` condition selects the `then` branch
- a known `false` condition selects the `else` branch, or an empty scoped result when no `else`
  exists
- an unknown or runtime condition remains an ordinary `if`

The selected branch retains its authored lexical scope. Specialisation must not hoist branch-local
bindings into the parent scope or flatten control-flow ownership merely because the runtime test is
removed.

The specialised active AST becomes the authority for function terminality, durable generated-function
requests, executable effect/access/project-context summaries, HIR construction, borrow validation,
lifetime-region and escape analysis, per-function link facts and reachability, target-contract
validation, and backend lowering.

The inactive branch contributes none of those downstream products. HIR must never receive an `if`
whose condition is already a known compile-time Bool.

### Stable source structure

Compile-time values may specialise executable behaviour. They do not change source structure.
`#Config` and ordinary compile-time Bool conditions must not control dependency clauses or provider
graph edges, package resolution, source discovery or semantic source sets, declaration existence,
exported declaration existence, receiver-method existence, trait or conformance existence, or
module/package facade topology.

This keeps Stage 0 graph construction, Stage 3 declaration order, declaration identity and module
interfaces structurally stable across configured builds. Public semantic summaries and executable
fingerprints may still change when the active body has different effects, calls or lifetime facts.

### Platform-agnostic source boundary

Moth source does not inspect compilation targets, operating systems, architectures, backend choice
or runtime-platform identity. Platform integration belongs to project builders, builder packages,
external packages and backend capability surfaces.

The static-`if` system is for typed application and build configuration, not target-dependent source.
Builders must not recreate target conditional compilation by supplying synthetic values such as
`target_os`, `target_arch`, `is_wasm`, `is_javascript` or equivalent backend identity flags through
the future `#Config` surface.

A change to this boundary requires an explicit Moth design-philosophy review. It is not a deferred
extension of this plan.

---

## Architectural invariants

- Tokenization and declaration-shell parsing happen once.
- Stage 0 provider graphs and semantic source sets do not depend on constant or `#Config` values.
- Stage 3 dependency order is authoritative. AST must not add a constant fixpoint or rediscover
  declaration dependencies.
- Every ordinary constant and const template folds once in its defining module.
- A provider exports an owned folded value. Consumers never parse or fold provider source again.
- `TypeId` drives semantic decisions. `DataType` is parse or diagnostic data after resolution.
- Donor-local `TypeId`, declaration IDs, value IDs and store IDs never cross module interfaces.
- Constant folding preserves checked numeric failures, cast rules, finite-Float rules, template
  preparation rules and synthetic-interface provenance.
- Both branches of an ordinary `if` complete frontend syntax, name and type validation before a
  compile-time-known Bool selects executable control flow.
- Static specialisation preserves lexical scope and source identity.
- Function terminality and every durable executable side product are computed from the specialised
  active AST.
- A pruned branch contributes no HIR, generated sidecar work, borrow/lifetime facts, link facts,
  target requirements or backend code.
- HIR does not become a second constant folder or type resolver and never receives a statically
  decided `if`.
- Type and fold errors retain current source locations, diagnostic families and priority. Phase G
  intentionally removes only downstream diagnostics belonging exclusively to inactive executable
  work.
- TIR stays AST-local and is dropped before the completed AST leaves Stage 4.
- Platform and backend identity do not enter source-level constant conditions through compiler- or
  builder-provided target flags.
- Capacity estimates and caches affect performance only. A miss or underestimate cannot change
  correctness.
- Parallelism, reuse and caching preserve deterministic identities, diagnostics and output order.
- Header-stage binding data is shared by handle, never copied per declaration. Any pass that needs
  an owned `FileVisibility` must justify it.

---

## Non-goals

- no `#Config` parser, CLI input implementation or project-global interface in this plan
- no `#Config if` syntax or second conditional-control-flow category
- no conditional dependency clauses, declarations, exports, receiver methods, traits or conformances
- no target, operating-system, architecture or backend introspection in source
- no textual preprocessing or skipped syntax/name/type validation for inactive branches
- no new language type or coercion rule
- no general compile-time virtual machine
- no arbitrary user-function execution during constant folding
- no match reduction, loop unrolling or broad CFG partial evaluation
- no parallel constant evaluation before dependency and diagnostic ordering are proven safe
- no rewrite of TIR, Stage 3 or `TypeEnvironment` into competing frameworks
- no compiler-wide unsafe packing
- no full AST arena without measured evidence
- no token/source layout work already owned by the data-layout plan
- no cross-module sharing of donor-local IDs or mutable stores
- no best-effort fallback that reparses source or rebuilds semantic facts
- **no representation change without a profile that names the cost it removes**

---

## Phase A - Re-baseline and attribution

Goal: replace the superseded Phase 0 baseline and choose the order of Phases B to F from measured
attribution rather than from the original plan's assumptions.

Why now: two consecutive slices moved AST time by between `6%` and `98%` depending on workload. Every
share in the Phase 0 baseline was measured against a dominant cost that no longer exists, so no
current priority in this plan is evidence-backed.

Targets: `benchmarks/manifest.toml` fixtures, `--profile profiling` builds, `MOTH_TIMERS=full`,
`MOTH_COUNTERS=summary`.

Known constraints:

- Recorded runs (`just bench`, `just bench-frontend`) require a clean committed worktree and rewrite
  the tracked monthly summary, so consecutive recorded invocations need
  `git checkout -- benchmarks/summaries/` between them. Fixed-thread runs (`RAYON_NUM_THREADS=1`)
  never touch the tracked summary. Per-case medians come from `benchmarks/local-data/runs.jsonl`,
  which read-only `bench-check` modes do not write.
- `--release` strips symbols. Use `--profile profiling` for any `sample` run.
- `frontend.ast.environment.constant_header_resolution` over-measures: its timing guard is declared
  before `resolve_constant_headers` and drops at the end of
  `resolve_nominal_members_and_constants`, so it also covers the struct-field and choice-variant
  loops. Treat it as an upper bound until the guard is narrowed.

Work items:

- [x] Record five independent runs per case for `docs`, `one_module_kitchen_sink`, `type_stress`,
      `environment_stress`, `nominal_capacity_stress`, `fold_stress`, `expression_rpn_churn`,
      `template_stress`, `constant_dag_churn` and the three constant chains.
- [x] Repeat at the normal thread identity to confirm no scheduling regression.
- [x] Capture module-attributed constant, const-template and finalisation timings.
- [x] Narrow the `constant_header_resolution` timing guard so its span matches its name.
- [x] Sample `--profile profiling` builds and produce a ranked function-level attribution table.
      Done for `docs` and for a scaled regeneration of `nominal_capacity_stress`; see the outcome
      note on why the committed small fixtures could not be sampled directly.
- [x] Record current values for every `AstCounter` and `FrontendCounter` this plan added.
- [x] Write the attribution into `benchmarks/frontend-optimization-results.md`.
- [x] Confirm or reorder `PHASE_ORDER` in this file, with one sentence of justification per move.

Acceptance:

- [x] every later performance phase in this file names a cost that appears in the Phase A table
- [x] any phase whose target does not appear is explicitly re-scoped as clarity work or dropped

Checkpoint: evidence only. No source change beyond the timing-guard fix.

### Phase A outcome

Full tables, profiles and counter series are in `benchmarks/frontend-optimization-results.md`
under **Constant Evaluation And Type-System Plan - Phase A Re-Baseline And Attribution**. What a
later phase must not rediscover:

1. **`constant_header_scope_context` is quadratic, and it is the largest defect in the frontend.**
   It deep-clones five whole-module side tables per nominal header. Regenerating
   `nominal-capacity-stress.moth`'s documented pattern at 40, 160, 640 and 2560 buckets gives
   `ast.environment` of `9.295ms`, `124.922ms`, `1947.223ms` and `43816.283ms`: a `64x` input costs
   `4714x`, or `O(n^2.03)`. In the call graph, `609` of `613` samples in the struct-field loop and
   `447` of `449` in the choice-variant loop are the construction and destruction of that one
   `ScopeContext`. This is Phase B and it is now the top priority.
2. **The timing guard really was mis-scoped, and it had misled the plan.** Measured on both
   scopes, `constant_header_resolution` fell from `6.344ms` to `0.127ms` on
   `nominal_capacity_stress`, from `1.940ms` to `0.000ms` on `type_stress`, and from `1.434ms` to
   `0.000ms` on `environment_stress`. `docs` and the constant chains did not move. Those three
   fixtures had no constant cost at all; the metric was reporting the member-shell loops under a
   constant-resolution name, which is part of why the plan originally read them as constant-heavy.
3. **Counters cannot find this class of defect. Second confirmation.** Across the four scaled
   sizes no frontend counter grows faster than input; the closest are exactly linear
   (`ast_type_resolution_calls` `63.9x` for a `64x` input). Type resolution is *called* a linear
   number of times - the cost per call is what grew.
4. **`ast_constant_pass_side_table_entries_cloned` is pointed at the wrong copy.** It instruments
   the once-per-module snapshot Phase 1 hoisted into `resolve_constant_headers`, which is correct
   and linear. It has never seen the per-header snapshot that is actually quadratic.
5. **`docs` and the nominal fixtures need different phases and neither substitutes for the other.**
   `docs` is 72 modules, 545 constants, 5162 const templates and **2** structs, so Phase B cannot
   help it. Its `87.1ms` of constant header resolution sits alongside `ast_expression_fold_items =
   0` and `constant_fold_attempt_count = 0`, so its constant cost is const-template work, not
   arithmetic folding - Phase D cannot be validated on it either.
6. **Allocation dominates both shapes.** `79%` of non-idle self time on the nominal fixture and
   `87%` on `docs` is the allocator plus `memmove`/`memset`. Every phase in this plan that deletes
   a duplicate representation is also deleting allocator traffic, which is the main reason C and D
   keep their positions without a profile of their own.
7. **Tooling: `samply` cannot symbolicate this binary and `sample` cannot catch a short run.**
   `just profile-case` and `just profile-case-symbolicated` both report `failed_raw_addresses`,
   matching the AUD-0002 note. macOS `sample` symbolicates correctly but attaches by process name
   and misses anything under roughly 100ms, which is why the scaled fixture was necessary. The
   generator and the harness scripts are preserved untracked under `tmp/phaseA/`.
8. **Unrelated lead, recorded so it is not lost.** `ExternalPackageRegistry` hashes all fifteen of
   its maps with `std::collections::HashMap` and the default `RandomState`, next to
   `datatypes/environment.rs:92` which uses `FxHashMap` for the same key type. Only the
   `hash_one<ExternalTypeId>` samples (about `2%` of `docs` moth self time) are confirmed to be the
   registry. It is a small independent change and it is not evidence for any phase here.

## Phase B - Shared environment side tables

Goal: remove the per-header `Rc::new(map.clone())` snapshots of the environment builder's side
tables. Phase A measured this as the only quadratic cost in the frontend and the largest single
defect in it.

Why now: measured, not inferred. `O(n^2.03)` in module size, `>99%` of the cost of the pass that
contains it, and `43.8 seconds` of `ast.environment` for a 2560-declaration module. It is one call
site with five clones.

Targets:

- `type_resolution.rs`, `constant_header_scope_context` - the confirmed quadratic site. Called once
  per struct and once per choice header through `unresolved_member_syntax_to_declarations`, and it
  deep-clones all five of `resolved_type_aliases_by_path`, `generic_declarations_by_path`,
  `resolved_struct_fields_by_path`, `choice_variant_shells_by_path` and `nominal_type_ids_by_path`.
  `resolved_struct_fields_by_path` is `FxHashMap<InternedPath, Vec<Declaration>>` and `Declaration`
  owns a recursive `Expression` and `DataType`, so each clone is deep as well as `O(module)`.
- `function_signatures.rs:155,165`, once per generic function header. Same shape, not separately
  measured - no committed fixture has enough generic function headers to show it.
- `traits.rs:539-544`, once per trait requirement. Same shape, not separately measured.
- `scope_context/lookup.rs:155`, `is_explicit_compile_time_constant` linear-scans
  `lookups.module_constants` for every fixed-capacity check during body emission.

Known constraints:

- These tables are **still being written while the loops that snapshot them run**.
  `resolve_type_declarations` writes `resolved_struct_fields_by_path` and
  `choice_variant_shells_by_path` inside the same loop that snapshots them, and headers are
  dependency-sorted so a later struct's fields legitimately read an earlier struct's resolved
  fields. A snapshot hoisted out of the loop would go stale and silently change resolution.
- **Copy-on-write is therefore the shape for the quadratic site, not hoisting.** Hold the tables
  behind `Rc` in the builder and write through `Rc::make_mut`. Reads become free. The
  `ScopeContext` that borrows a handle is dropped before the next write, so the builder is the sole
  owner at write time and `make_mut` does not clone. Hoisting stays available for the
  function-signature pass, which reads four tables it never writes.
- `ConstantResolutionSession` already holds these tables correctly. Do not build a second session
  type for the member-shell or signature passes; either widen the existing one or hoist.
- Do not let the drop cost hide. Phase A found the snapshot's destruction (`293` and `216` samples)
  to be almost exactly as expensive as its construction (`316` and `231`). A change that removes
  the clone but leaves an owned per-header structure behind has only fixed half of it.

Work items:

- [ ] Establish, per pass, which side tables it writes while iterating.
- [ ] Move the five tables behind copy-on-write builder handles and take handles in
      `constant_header_scope_context`.
- [ ] Hoist the snapshots that are provably read-only for their loop.
- [ ] Replace the `module_constants` linear scan with the existing
      `resolved_module_constant_paths` set or its Phase H successor.
- [ ] Re-point `ConstantPassSideTableEntriesCloned` at the per-header copy, which is the one that
      matters, or delete it if no copy survives. It currently measures the once-per-module
      snapshot and reads linear while the real copy is quadratic.
- [ ] Decide whether a scaled nominal fixture joins `benchmarks/manifest.toml`. Phase A needed one
      to see this at all and had to generate it untracked; without a committed equivalent the
      recorded suite cannot regression-test the fix. Raise it rather than deciding silently - it
      changes the tracked benchmark surface.

Acceptance:

- [ ] the regenerated nominal scaling curve is linear in module size, replacing `O(n^2.03)`
- [ ] `environment_stress`, `type_stress` and `nominal_capacity_stress` improve
- [ ] no pass reads a side table snapshot taken before a write it depends on
- [ ] diagnostics, warning order and emitted artefacts are byte-for-byte equivalent

Checkpoint: ownership only. No value representation or control-flow change.

## Phase C - Module-local folded-value authority

Goal: give each module constant one folded-value owner, and delete the three representations of one
already-folded fact.

Why now: the largest remaining structural duplication, and Phase G depends on it. This is a deletion
phase whose value does not depend on the Phase A ranking. Phase A confirmed its position: `docs`
spends `18.9ms` of its `19.3ms` finalisation in `finalise.module_constant`, and `87%` of `docs`
self time is allocator and copy traffic, so removing a duplicate representation of an
already-folded fact is removing allocations on the one large real workload.

Targets: `src/compiler_frontend/ast/const_values/`, `src/compiler_frontend/folded_value.rs`,
`finalization/normalize_constants.rs`, `finalization/public_const_templates.rs`,
`hir/hir_statement/declarations.rs`, `environment/lookups.rs`.

Known constraints:

- `AstModuleLookups::module_constants` is a second `Vec<Declaration>` alongside the declaration
  table. Both are read: `normalize_constants.rs` and `public_const_templates.rs` iterate the vector,
  `lookup.rs` scans it for constant identity. Every consumer must move before the vector goes.
- Template-valued constants classify through `classify_template_from_effective_tir` and carry
  module-local reference, phase and overlay identity. The store must retain that, not flatten it.
- `is_helper_only_template_value` filters `$insert(..)` helper constants out of the HIR handoff.
  That filter is a real semantic rule, not an optimisation, and must survive the migration.

Store and rows:

- [ ] Add `ConstValueId`, `ConstValueStore` and compact module-constant rows
      (`{ declaration: DeclarationId, value: ConstValueId }`).
- [ ] Define scalar, collection, record, choice, range, option/fallible and string/template-folded
      payloads required by current language support.
- [ ] Preserve type, provenance, const-record and location facts.
- [ ] Make declaration lookup return constant identity without cloning its value tree.
- [ ] Change module-constant references and field access to read the store.

Consumers:

- [ ] Project exported constants directly from the store into `PublicFoldedValue` once.
- [ ] Move the store and rows through the AST-to-HIR boundary.
- [ ] Make HIR module constants reference or consume the same store.
- [ ] Update config and direct `.mtf` compilation extraction to read the shared store.
- [ ] Keep advisory body-local/inferred `AstConstFacts` separate from authored module-constant
      storage, but reuse `ConstValueId` where a value is retained.

Deletions:

- [ ] Replace `module_constants: Vec<Declaration>` in environment/lookups/AST contracts.
- [ ] Delete the `declaration.clone()` that exists only to own the constant twice.
- [ ] Delete recursive `normalize_module_constant_expression`.
- [ ] Delete the AST-expression-to-HIR-constant recursive conversion.
- [ ] Consolidate public and HIR conversion walkers around one borrowed store visitor.

Acceptance:

- [ ] each module constant has exactly one folded-value owner
- [ ] public projection and HIR consume it without reparsing or deep intermediate clones
- [ ] `ast_module_constant_declaration_clones` reaches zero
- [ ] helper-only template constants are still excluded from the HIR handoff

Checkpoint: one folded-value authority with all old conversion paths removed. Control flow unchanged.

## Phase D - Move-only folding and lazy diagnostics

Goal: stop building rich intermediate data around a correct algorithm. Keep shunting yard as the one
precedence owner.

Why now: `ast_diagnostic_data_type_materialisations` equals `ast_expression_fold_items` exactly, on
every fixture that folds anything - `960/960` on `fold_stress`, `392/392` on `constant_dag_churn`,
`45/45` on `expression_rpn_churn`, `9/9` on `generic_trait_churn`. Every successful fold builds
diagnostic spelling it never uses. `ast_expression_operand_clones` runs at about `0.81` per fold
item, so most folded operands are also full `Expression` clones. All of this is counted and
provably wasted, and none of it depends on the Phase A ranking to be worth removing.

**Scope honesty, from Phase A.** No committed fixture makes this cost large in absolute terms:
every fixture with non-zero fold items has an `ast.total` under `3ms`, and `docs` - the only large
real workload - folds nothing at all (`ast_expression_fold_items = 0`). So this is a deletion and
representation phase whose justification is the proven `1:1` waste, not a measured share of a
profile. Either commit a scaled folding fixture as part of this phase or record the improvement as
counter-verified only. Do not claim a wall-time scaling result the fixtures cannot support.

Targets: `src/compiler_frontend/ast/expressions/expression_rpn.rs`,
`src/compiler_frontend/ast/const_eval/`, `src/compiler_frontend/ast/type_resolution/`,
`src/compiler_frontend/type_coercion/compatibility.rs`.

Known constraints:

- Type validation must complete before fold evaluation so the current type-error-before-fold-error
  priority is preserved. This is observable in existing diagnostic tests.
- A full `ExprId` arena is evidence-gated. Implement it only if the Phase A profile or the counters
  after this phase show that move-only full `Expression` operands remain material. Do not introduce
  an arena to satisfy an architectural preference.

Ownership cleanup:

- [ ] Make expression ordering reserve from the known input count.
- [ ] Make `constant_fold` consume its item vector.
- [ ] Move non-foldable operands and operators back into the runtime result.
- [ ] Return the sole folded operand by move.
- [ ] Add focused tests proving no source or synthetic provenance is lost.
- [ ] Drive full-expression clone counters to zero in ordinary arithmetic fold paths.

Typed postfix:

- [ ] Resolve operator input/result `TypeId`s as operators leave the shunting-yard stack.
- [ ] Emit a compact typed postfix item carrying only semantic IDs, flags and diagnostic anchors.
- [ ] Validate the whole typed expression before executing fold operations.
- [ ] Delete the separate rich RPN result-type scan.
- [ ] Make the fold evaluator produce `ConstValueId` directly.
- [ ] Preserve reduced postfix only for runtime-dependent work.

Lazy diagnostics and explicit type-resolution views:

- [ ] Split the broad optional `TypeResolutionContextInputs` shape into explicit data views:
      immutable declaration/visibility lookup, mutable derived-type interning, optional generic
      scope, optional trait/evidence overlay, optional constant-value lookup.
- [ ] Add named constructors for module declaration, constant, body and generated contexts, so
      invalid combinations are unrepresentable or rejected at construction.
- [ ] Return `TypeId`-first results from successful lookup paths and construct diagnostic spelling
      only at the error or public-display boundary.
- [ ] Borrow resolved aliases, fields, variants and signatures instead of cloning them for
      read-only validation.
- [ ] Remove the remaining owned `visible_declaration_ids` copy in
      `type_resolution/struct_fields.rs:316`, which exists only because
      `TypeResolutionContextInputs` carries a borrow where the scope needs a handle.

Acceptance:

- [ ] `ast_expression_operand_clones` and `ast_diagnostic_data_type_materialisations` fall well
      below `ast_expression_fold_items`
- [ ] diagnostic text, ordering and priority are unchanged
- [ ] shunting yard remains the one precedence algorithm

Checkpoint: typed-postfix and lazy diagnostic data with unchanged source semantics.

## Phase E - Nominal member shells and capacity fixups

Goal: keep one immutable parsed member shell per struct field and choice payload, and build canonical
member definitions once.

Why now: `resolve_type_declarations` builds member shells before constants and rebuilds them after,
so the same field and variant structure is reconstructed twice per nominal.
`nominal_capacity_stress` isolates this from constant count.

**Re-measure before implementing.** Phase A found that `>99%` of the cost of the pass this phase
targets was the side-table snapshot, which Phase B removes. Whatever remains here after Phase B is
the true cost of building shells twice, and it has never been measured on its own. Start this phase
by re-running the scaled nominal curve and recording what is left. If the double construction turns
out to be cheap, this becomes a correctness and clarity phase - one shell per member is still the
right shape - and it must be recorded as such rather than carrying a scaling claim.

Targets: `environment/type_resolution.rs` (`unresolved_member_syntax_to_declarations`,
`resolve_constructor_shells_for_constants`, `resolve_type_declarations`),
`type_resolution/struct_fields.rs`, `datatypes/definitions.rs`.

Known constraints:

- The rebuild exists because fixed-capacity expressions in member types depend on constants that are
  not resolved when shells are first needed. The replacement must record a targeted fixup, not defer
  the whole shell.
- Default-value diagnostics and recursive-type validation locations must keep their current source
  anchors.

Work items:

- [ ] Define the single retained field/variant member shell and its resolution slots.
- [ ] Register nominal identities and generic metadata without constructing unresolved declaration
      value trees.
- [ ] Resolve constructor-required member types into slots before constant evaluation.
- [ ] Record only constant-dependent capacity and default fixups, keyed by affected member and
      constant declaration.
- [ ] Apply fixups in declaration order after their constants commit.
- [ ] Build canonical `FieldDefinition` and `ChoiceVariantDefinition` arrays once, move them into
      `TypeEnvironment` and expose borrowed views.
- [ ] Delete the early and late reconstruction of member declarations and choice variants.

Acceptance:

- [ ] `nominal_capacity_stress` improves
- [ ] no member shell is constructed twice
- [ ] default-value and recursive-type diagnostics keep their locations

Checkpoint separately for structs and choices if either surface becomes broad.

## Phase F - Type environment and generic substitution keys

Goal: make `TypeEnvironment`'s hot tables dense where profiling justifies it, and canonicalise each
concrete generic binding set once.

Why now: last of the performance phases, and the one most at risk of unjustified complexity. Apply
one table change per checkpoint with layout/query tests and benchmark evidence.

**Re-scoped by Phase A: this phase currently has no measurable target.** The counters it is written
against are near zero on every committed fixture. `generic_trait_churn`, the fixture built for it,
reports `generic_substitution_key_builds = 12`, `type_environment_substitute_type_id_calls = 89` and
`type_environment_substitution_cache_lookups = 12`. Nothing in the Phase A attribution table points
here. Two honest options, and the choice belongs to whoever reaches this phase:

- Commit a fixture that actually exercises generic instantiation at scale, re-measure, and proceed
  only against what that shows.
- Drop the dense-storage conversions entirely and keep only the substitution-key canonicalisation,
  on the grounds that repeatedly collecting, sorting and boxing the same mapping is waste
  regardless of its current share.

Do not perform the dense-storage conversions on the strength of this plan alone. The phase's own
constraint - that a less readable table which is not hot is a regression - now applies to the whole
phase, not just to individual conversions within it.

Targets: `src/compiler_frontend/datatypes/environment.rs`,
`src/compiler_frontend/datatypes/generic_parameters.rs`,
`src/compiler_frontend/ast/generic_functions/`.

Known constraints:

- Do not replace `TypeEnvironment`. It already owns dense `TypeId` storage and canonical interning.
- Keep forward structural, path and canonical-identity interning maps hashed.
- Do not expose physical table layout outside `TypeEnvironment` query methods.
- Revert any conversion that adds complexity without measurable benefit. A less readable table that
  is not hot is a regression.

Type environment:

- [ ] Add capacity-aware construction from existing `FrontendArenaCapacityEstimate` and header
      statistics.
- [ ] Convert `NominalTypeId -> TypeId` to dense storage.
- [ ] Convert generic parameter `ID -> TypeId` and `ID -> bounds` to dense storage.
- [ ] Evaluate `TypeId -> canonical identity` dense optional storage.
- [ ] Store generic instance field/variant views in compact immutable ranges or slices.
- [ ] Confirm every query returns borrowed data unless ownership is required at a stage boundary.
- [ ] Record memory and lookup effects for each conversion.

Generic substitution keys:

- [ ] Canonicalise each concrete generic binding set once, with a module-local `GenericBindingsId`
      if useful.
- [ ] Reuse its ordered pair slice in recursive substitution.
- [ ] Change substitution-cache keys to avoid collecting, sorting and boxing the same mapping for
      each source `TypeId`.
- [ ] Keep cache scope module-local and preserve deterministic ordering and conflict diagnostics.

Acceptance:

- [ ] `GenericSubstitutionKeyBuilds` and `GenericSubstitutionKeySortedPairs` fall materially
- [ ] `type_stress` and generic-trait workloads improve
- [ ] every dense conversion cites its own evidence, and unjustified ones are reverted

Checkpoint one table or key change at a time.

## Phase G - Static Bool `if` specialisation

**This is the mandatory semantic review gate.** Do not combine it with any performance refactor.

Goal: implement the accepted static control-flow contract above.

Why here: it needs a single folded-value authority to read the condition from, which Phase C
provides. Everything before it is optimisation; this phase changes what the compiler can do.

Targets: one new Stage 4 specialisation owner in `ast/module_ast/finalization/`,
`ast/statements/branching.rs`, `ast/statements/terminality.rs`,
`hir/hir_statement/control_flow.rs`.

Known constraints: Phase 0 findings 1 to 4 above. In particular, read the condition through the
folded-value authority rather than by matching `ExpressionKind::Bool`, and reuse
`ScopeContext::generic_request_checkpoint` rather than adding a second pruning boundary.

Work items:

- [ ] Add one named Stage 4 specialisation owner after full branch frontend validation and final
      constant values, before terminality, durable generated work and HIR.
- [ ] Read a condition's known Bool from the existing typed folded-value authority. Add no second
      evaluator and no config-specific lookup.
- [ ] Specialise statement `if` while preserving the selected branch's lexical scope, source
      locations and statement order.
- [ ] Specialise value-producing `if` only after both branches satisfy receiving arity and type
      rules.
- [ ] Leave runtime and unknown Bool conditions as ordinary `NodeKind::If` values.
- [ ] Run terminality over the specialised active AST.
- [ ] Ensure inactive branch calls publish no generated-function requests or generated sidecar work.
- [ ] Ensure inactive branch code contributes no HIR, borrow facts, lifetime facts, executable
      effects, link facts, target requirements or backend output.
- [ ] Preserve syntax, name, visibility, type, generic-evidence, cast, const-evaluation and
      value-production diagnostics from both authored branches.
- [ ] Preserve stable declaration, type and function identities.
- [ ] Record branch-selection dependencies in implementation, effect/link and root fingerprints
      through the existing fingerprint owners. Do not add a parallel static-if fingerprint.
- [ ] Assert that HIR contains no branch terminator for a statically decided `if`.
- [ ] Assert that a runtime Bool condition still produces the established HIR branch/merge shape.
- [ ] Enable and complete the three ignored Phase 0 intended-contract tests.

Review gate acceptance:

- [ ] both authored branches remain frontend-valid source
- [ ] selected branch scope is unchanged
- [ ] terminality observes selected control flow
- [ ] inactive durable generic requests are absent
- [ ] all downstream compiler systems receive only active executable work
- [ ] source graph, declarations, exports and package topology are unchanged
- [ ] no platform/backend conditional mechanism has entered source

Checkpoint: accepted static-control-flow semantics and focused end-to-end validation. Artefact
changes are expected and must match the selected active branch exactly.

Closeout for this phase must update the compiler architecture documentation, the progress matrix and
user-facing constant/control-flow documentation, because it changes current language support.

## Phase H - Declaration lanes and environment pass cleanup

Goal: dense declaration identity and readable pass structure. **This is an ownership and clarity
phase, not a scaling fix.** Its original scaling justification was measured away by the shared
file-visibility slice.

Why last of the implementation phases: it touches the same passes as B, C and E, and doing it after
them means it cleans the final shape rather than a shape three phases will change again.

Targets: `environment/declaration_table.rs`, `environment/builder.rs`,
`environment/type_resolution.rs`, `scope_context.rs`.

Known constraints:

- `replace_declaration` commits through `Rc::get_mut`, so no scope may hold the declaration table
  across a constant commit. This phase decides deliberately whether the table gains a commit path
  that tolerates live readers.
- `ScopeDeclarationRef::Shared(&'a Declaration)` hands out a borrow tied to the context lifetime, so
  the table cannot simply move behind a `RefCell`.
- If the commit path does gain live readers, the constant session collapses to one root scope with
  per-constant child frames, and the two open Phase 1 items close with it.

Work items:

- [ ] Make Stage 3 final order allocate or carry stable `DeclarationId`s.
- [ ] Add direct ID-based `get`, `get_mut` and `replace` to `TopLevelDeclarationTable`, and use
      path/name indexes only for source lookup.
- [ ] Build compact ordered declaration-kind lanes once from final order, storing IDs, not headers.
- [ ] Add `ResolvedConstantSet` keyed by `DeclarationId` and resolve explicit module-constant
      visibility through it.
- [ ] Replace path-based declaration replacement with ID replacement inside ordered semantic passes.
- [ ] Migrate aliases, nominals, constants, traits and functions to their lanes, keeping complete
      declaration order for passes whose semantics depend on it.
- [ ] Remove repeated `for header in sorted_headers { match kind ... }` scans that no longer own
      ordering.
- [ ] Decide the declaration-table commit path, then close or explicitly re-defer the two open
      Phase 1 items: the synthetic per-constant `AstModuleLookups`, and the constant-header
      `ScopeContext` builder chain.
- [ ] Consolidate environment final assembly so each side table moves once into its final owner.
- [ ] Ensure builtins and generated construction appends allocate IDs through the table owner so
      indexes and lanes cannot drift.

Acceptance:

- [ ] no per-constant synthetic complete lookup context remains, or its survival is justified here
- [ ] `ast_declaration_replacements_by_path` reaches zero for ordered semantic passes
- [ ] orchestration stays readable; the phase order is not hidden in a generic pass framework
- [ ] no scaling claim is recorded for this phase unless a measurement supports one

Checkpoint: index and pass cleanup with no behaviour change.

## Phase I - Final audit and closeout

Run focused validation throughout. At final closeout run at minimum:

```bash
cargo test --lib compiler_frontend::ast::const_eval
cargo test --lib compiler_frontend::ast::const_values
cargo test --lib compiler_frontend::ast::type_resolution
cargo test --lib compiler_frontend::ast::statements::branching
cargo test --lib compiler_frontend::ast::statements::terminality
cargo test --lib compiler_frontend::datatypes
cargo test --lib compiler_frontend::hir
just bench-frontend-check
RAYON_NUM_THREADS=1 just bench-frontend-check
just bench-check
just validate
```

At each performance-only checkpoint:

- [ ] run five independent benchmark invocations and compare medians
- [ ] use the same timing schema, source fingerprint, measurement fingerprint and thread identity
- [ ] run the relevant scaling fixture
- [ ] record counter and timing movement in `benchmarks/frontend-optimization-results.md`
- [ ] treat an unexplained median regression above 5% as a blocker
- [ ] require semantic and output equivalence before accepting a speed improvement

At the Phase G semantic checkpoint:

- [ ] compare output to the accepted static-control-flow contract rather than the old artefact bytes
- [ ] record the intended HIR, generated-work, borrow/lifetime, link and target-validation deltas
- [ ] prove runtime conditions retain the existing semantics
- [ ] prove both authored branches retain frontend diagnostics
- [ ] run focused integration cases for active and inactive downstream failures

Final acceptance:

- [ ] constant setup scales linearly with constant count
- [ ] no per-declaration copy of header-stage binding data remains
- [ ] no cumulative previous-constant copying remains
- [ ] each module constant has one folded-value authority
- [ ] public projection and HIR consume that authority without reparsing or deep intermediate clones
- [ ] shunting yard remains the one precedence algorithm
- [ ] rich RPN typing and fold clones are removed
- [ ] successful type resolution is `TypeId`-first and diagnostic spelling is lazy
- [ ] dense `TypeEnvironment` changes are evidence-backed and private
- [ ] nominal members retain one syntax shell and build canonical members once
- [ ] both static-`if` branches are frontend validated before selection
- [ ] a statically decided `if` never reaches HIR
- [ ] inactive branches produce no generated sidecars, borrow/lifetime facts, link facts, target
      requirements or backend code
- [ ] runtime Bool conditions preserve ordinary `if` behaviour
- [ ] branch lexical scope and source identity remain intact
- [ ] terminality consumes specialised active control flow
- [ ] source graphs, declaration identities and structural public surfaces do not depend on
      compile-time conditions
- [ ] no platform/backend conditional source mechanism exists
- [ ] old redundant APIs and representations are deleted without compatibility wrappers
- [ ] optimisation-only phases preserve diagnostics, public identities and emitted artefacts
- [ ] Phase G changes executable artefacts and downstream diagnostics only where the inactive branch
      is deliberately absent
- [ ] every phase in this file carries an Outcome subsection with its measurement

The queued build-configuration-values plan must integrate `#Config of Bool` through the same
ordinary constant and `if` path without adding another specialisation owner.

---

## Simplification and reuse audit

### Reuse directly

- `TopLevelDeclarationTable` and its existing `DeclarationId`
- Stage 3 dependency order
- `FrontendArenaCapacityEstimate` and header/token statistics
- `TypeEnvironment` interning and substitution caches
- `TypeCompatibilityCache`
- binding-owned `Arc<FileVisibility>`, shared by handle
- `ConstantResolutionSession` as the one constant-pass owner
- `AstModuleEnvironmentBuilder::resolved_module_constant_paths`
- scope-frame arenas for body-local declarations only
- existing `NodeKind::If`, branch scopes and value-producing `if` validation
- existing AST finalisation boundary, extended with one named static-control-flow owner
- existing terminality validation, moved after specialisation
- TIR store, exact views, preparation and fold cache
- `PublicFoldedValue` for owned cross-module projection
- existing HIR module-constant and const-fact consumers, migrated to IDs and store access
- existing HIR branch lowering for runtime conditions only
- `ScopeContext::generic_request_checkpoint` for inactive-branch pruning
- current benchmark manifest, profiles, counters and five-run protocol

### Remove after migration

- per-header side-table snapshots in the signature, member-shell and trait passes
- the `module_constants` linear scan for explicit constant identity
- duplicate `Vec<Declaration>` module-constant ownership
- recursive normalised-expression reconstruction for module constants
- duplicate AST-to-public and AST-to-HIR value-tree interpretation
- rich RPN item cloning
- eager diagnostic `DataType` stacks
- repeated generic substitution map sorting and boxing
- duplicate nominal member shell reconstruction
- repeated broad header scans made obsolete by declaration lanes
- constant-header synthetic `AstModuleLookups` and its `ScopeContext` setter chains
- HIR `if` diamonds whose conditions are already known compile-time Bool values
- inactive-branch generated requests and executable side products
- pre-specialisation terminality assumptions

### Avoid

- a second constant parser
- a second type environment
- a generic compiler-pass framework
- a new template fold cache
- a `#Config`-specific branch representation
- textual preprocessing or conditionally parsed source
- conditional provider graph or declaration topology
- platform/backend identity flags in source
- general CFG partial evaluation disguised as static `if` folding
- source-token or span designs parallel to the data-layout plan
- speculative caches with incomplete semantic keys
- unsafe packing before ordinary ownership and algorithmic waste is removed
- a representation change chosen from a counter rather than a profile

## Completion contract

The plan is complete only when the compiler has fewer semantic representations, fewer construction
paths and one explicit Stage 4 boundary between fully validated source and specialised executable
control flow.

A faster implementation that leaves the old constant trees, context builders or conversion walkers
alive as compatibility layers does not satisfy the plan. A static-`if` implementation that skips
frontend validation, leaks platform policy into source, retains inactive downstream work or adds a
parallel config-specific branch path also does not satisfy it.
