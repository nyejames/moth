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
- TIR exact views remain the template authority
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
- Benchmark scaling lane (hardening slice, taken out of sequence) - see **Benchmark scaling lane**
  below
- Phase B (shared environment side tables) - see **Phase B outcome** below

CURRENT_SLICE:
- Phase: F closed with no conversions taken - see **Phase F outcome**. Before it, Phase E: two
  slices landed (`19340ca29` and the environment-scope consolidation) and the stage
  re-attributed on `docs`. See **Phase E first slice, and the real-project attribution that
  followed it** and its **Consolidation slice** section. Phase D is closed; see **Phase D
  outcome** and **Phase D Slice review**
- Goal: stop building rich intermediate data around correct algorithms - and, from the landed
  slice, stop rebuilding empty per-header scaffolding that the next line overwrites
- Non-goals: no control-flow change, no new fold rules, no template TIR-construction work - that is
  `67%` of this stage on a real project (finding 13) and it belongs to the template plan

WORK_ID: const-folding-types-hotpath
BASE_REVISION: e782e79d8
STATUS: in progress - Phase D complete and reviewed; two Phase E slices landed and the stage
  re-attributed on a real project with the dominant cost handed to the template plan; the
  trait-requirement divergence closed; Phase F measured on a purpose-built fixture and closed with
  no conversions taken
VALIDATION: `just validate` passes at the trait-requirement slice - 4448 + 17 + 788 Rust tests,
  1873 integration tests, source audit 0 findings, docs clean, both scaling series within budget,
  timers erasure clean. The third series, `generic_instantiation`, was added after that run and
  fits n^1.75 against its n^1.80 ratchet. Earlier, at the move-only folding
  commit: 4446 + 17 + 788 Rust tests,
  source audit 1205 files / 0 findings, clippy 0 findings, docs check clean, 74 benchmark cases
  preflighted, nominal-members fitted n^0.97, constant-chain fitted n^0.83, timers erasure clean.
  Phase C is measured directly against a rebuilt `e782e79d8`: see **Phase C outcome** and the
  results-document section.
AUDITS: Phase B checkpoint accepted; the Phase C interim auditor was clean. A separate
  implementation review of the Phase C candidate found six items, all now fixed - one of them a
  user-diagnostic regression the interim audit did not catch, because the test that covered it had
  been retargeted at a retained copy of the deleted function. The Phase C Slice review was
  originally reported blocked on audit-scope registration; AUD-0004 established that it never was.
  The obligation is the `AGENTS.md` Slice review, which needs no registered scope - the two
  activities merely shared a name, now corrected.
BLOCKERS: None. No implementation finding or validation failure is open.
NOTES: Phase C replaces the duplicate module-constant declaration/tree handoff with one compact
  store and borrowed consumers. `docs` cost is now sharply concentrated in
  `ast.environment.constant_header_resolution` (`86.1ms`, `51%` of its AST time), which neither
  Phase B nor Phase C moved. Its ownership is split between Phase D block 3 and Phase H - see
  **Phase C outcome** finding 7. A Phase C review raised four code corrections on the folded-value
  representation, applied before Phase D; see **Phase C corrections before Phase D**. The Phase C
  Slice review is run and recorded; see **Phase C Slice review**. Phase D began by attributing the
  `86ms` rather than assuming it: `84%` is `resolve_declaration_syntax`, `7%` is the synthetic
  constant scope. See **Measured attribution, done first** in Phase D. The ownership-cleanup block
  is complete and the typed-postfix block is half complete: folding now consumes its input and the
  typing stack carries `TypeId` only, so operand clones are `0` and diagnostic materialisations
  are no longer `1:1` on any fixture. The advisory constant environment is a shared module base of
  store ids plus a per-scope overlay, so per-scope entry copying is `0` on `docs`. The
  lazy-diagnostic block was then measured and retired: `resolve_type` is a small share of the stage
  it runs in on the fixture that calls it `13924` times. See **Phase D deletions and findings** 4-8,
  and finding 9 for the correction to that share. Phase E's attribution is complete and, unlike
  every earlier candidate in this plan, it confirmed the phase rather than retiring it: member-shell
  construction is `62.5%` of `ast.environment` on `nominal_scaling_320`. See **Phase E attribution**
  findings 9-11.

NEXT_ACTION:
- **Scope decision taken.** Findings 13-15 said the remaining performance in this stage is small
  and the `67%` belongs to the template plan, so that evidence was carried into
  `post-tir-template-parser-optimization-plan.md` rather than acted on here, and the work turned to
  consolidation. Two slices landed: `19340ca29` (per-scope lookup scaffold, `1.31x` on
  `nominal_scaling_320`, `1.02x` on `docs`) and the environment-scope consolidation (findings
  16-18, net `-101` lines, performance-neutral by design).
- **The trait-requirement scope divergence is closed** (findings 21-22). It was masked by a strict
  re-resolution immediately downstream, not a live correctness bug; visibility is now installed
  from the caller that already held it, and the contract has three regression cases where it
  previously had none.
- **Phase F is closed without its conversions** (findings 23-25). Its own option one was taken - a
  generic-instantiation fixture was committed and measured - and it showed `frontend.ast.environment`
  at `n^0.50` and `0.11%` of the frontend on the worst case built for it. Nothing in Phase F is
  taken.
- **What is left, in the order the evidence supports it:**
  1. A scope decision on finding 23: generated-function materialisation rebuilds the module's
     string table once per instantiation, measured `n^1.70` and `94%` of the frontend on the new
     fixture, `35.9%` on the existing `generic-trait-churn`. The cheaper mechanism
     (`merge_delta_from` + `fork_source`) already exists and module compilation already uses it.
     This is a different subsystem from this plan's, so it is a new phase rather than a step in
     this one.
  2. Phase G and H as written. Phase E's one-shell-per-member restructure was assessed against
     the simplification bar and not taken - see finding 20; the signature two-step was assessed
     the same way - see finding 22.
- Background for that decision - what the stage actually is on a real project: `67%` template TIR
  construction
  (`Template::new_const_required_with_type_interner`, `190us` per template constant over `331` of
  them), `7%` import projection, `6%` constant-header scope construction. Everything this plan has
  left is under `2%` each. See **Phase E first slice, and the real-project attribution that
  followed it**.
- Findings 13-15 stand and every later attribution in this plan gets both a
  scaling-fixture number and a real-project number. The fixture says what a cost is; only the
  project says what share of the stage it is.
- Do not re-derive the baseline from the scaling series. `nominal_members` reports fitted
  exponents, not stage medians; the deferred item to move that series to median stage timings is
  still open and still belongs before any scaling budget is tightened.
- Read **Phase D outcome** first. It records what Phase D corrected in this plan's own earlier
  findings, which is the part most likely to be re-proposed by mistake.
- Four lazy-diagnostic items are left open under Phase D and are marked **for re-proposal, not
  execution**. If the representable-states argument for `TypeResolutionContextInputs` is worth
  acting on, it belongs with Phase F and needs its own acceptance criteria as a clarity change.

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
- `docs/roadmap/plans/post-tir-template-parser-optimization-plan.md` - carries findings 13-15 and
  owns the `67%` this plan measured but does not take
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
  `nominal_capacity_stress` has since been replaced by the four-point `nominal_members` scaling
  series; measuring that path at a single size is what hid the quadratic cost Phase A found.
- `AstCounter` variants `ConstantResolutionContextsCreated`, `ConstantsResolved`,
  `ModuleConstantDeclarationClones`,
  `ExpressionOrderingInputItems`, `ExpressionTypedStackItems`, `ExpressionFoldItems`,
  `ExpressionOperandClones`, `DiagnosticDataTypeMaterialisations`, `BranchLocalGenericRequests`.
  `ConstantPassSideTableEntriesCloned` was deleted in Phase B: it instrumented a copy that no
  longer happens.
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
   `AstModuleEnvironmentBuilder::resolve_type_declarations`. That is Phase E. The
   `nominal_members` scaling series isolates that cost from constant count.
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
   and linear. It has never seen the per-header snapshot that is actually quadratic. *Resolved in
   Phase B by deletion: that snapshot became an `Rc::clone`, so no copy survives to instrument.*
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

## Benchmark scaling lane - hardening slice

Taken out of sequence between Phase A and Phase B, because Phase A found a quadratic cost that the
whole benchmark system was structurally unable to see, and Phase B is about to claim it fixed it.

**The gap.** Every benchmark mode compares a case against its own recorded history, which detects a
*change*. A cost that has been superlinear since it was written never changes, so every comparison
reports "no measurable change" forever. The `O(n^2.03)` side-table clone survived the full suite, a
dedicated stress fixture, a complete counter inventory and a recorded optimisation baseline. It was
found by hand-regenerating a fixture at four sizes.

**What was built:**

- `[[scaling]]` series in `benchmarks/manifest.toml`, manifest schema 4. A series names a timing
  metric, a complexity budget and three or more cases with declared input sizes. Validation
  enforces at least three points, strictly increasing sizes, a positive finite budget and one
  shared runner across all points.
- `just bench-scaling` (`xtask/src/bench_scaling.rs`) fits the slope of `ln(metric)` against
  `ln(size)` by least squares and fails when it exceeds the budget. Nine unit tests cover linear,
  quadratic and flat fits, the missing-metric path and the noise floor.
- `benchmarks/nominal-scaling/nominal-scaling-{40,80,160,320}.moth`, one generated pattern at four
  sizes with a constant count fixed at four. These replace `nominal-capacity-stress.moth`, which
  measured the same path at a single size and therefore could not see the shape.
- The constant chains, which already existed at three sizes, are declared as the second series.
- A profiling run whose symbolication returned raw addresses now fails the command instead of
  presenting eight hex addresses as hot functions.

**Two failure modes are treated as failures, not passes:** a metric that was never emitted, and a
largest point too small to fit. A series that cannot answer the question must not look like one
that answered it favourably.

**What it reported when it was built**, before Phase B:

```text
Scaling series 'nominal_members' — metric frontend.ast.environment — budget n^1.25
        size     metric_ms   size step   time step
          40        23.125           -           -
          80        79.706       2.00x       3.45x
         160       293.048       2.00x       3.68x
         320      1111.980       2.00x       3.79x
  fitted n^1.86 — EXCEEDS BUDGET n^1.25

Scaling series 'constant_chain' — metric frontend.ast.total — budget n^1.25
  fitted n^0.82 — within budget
```

Phase B moved `nominal_members` to `n^0.98` and its `320` point to `39.147ms`. The lane is now in
`just validate`.

Findings worth keeping:

1. **The lane reproduces the Phase A finding independently.** Phase A measured `n^2.03` through the
   release CLI with `RAYON_NUM_THREADS=1` and a detailed-timer build; the lane measures `n^1.86`
   in-process through the frontend suite. Two different measurement paths, same conclusion. The
   absolute times differ by about `2.3x` between the two paths, so compare exponents across them,
   never milliseconds.
2. **The constant chain is confirmed fixed, by budget rather than by eye.** `n^0.82` against a
   `n^1.25` budget. Phase 0's chain superlinearity is gone and there is now a command that says so.
3. **`just bench-scaling` was deliberately kept out of `just validate` until it passed.** It failed
   when it was built, because the defect was real, and a gate must pass on every commit. It joined
   `just validate` and the CI gate list in Phase B, once `nominal_members` came within budget.
4. **A Detailed metric cannot be used by a series.** The benchmark compiler is built with
   `--features timers`, not `detailed_timers`, so `constant_header_resolution` and its siblings are
   never emitted to the suite. This is why both series fit Basic metrics.

**Known weakness, deliberately not blocking Phase D.** Each point's stage timing comes from
`average_case_observations`, so its five iterations are reduced by a mean, while ordinary benchmark
decisions weight medians. One noisy large-case iteration can pull the fitted exponent upward. With
`n^0.82` and `n^0.96` against a `n^1.25` budget there is ample headroom, so this cannot currently
produce a false failure. Move the series to median stage timings before tightening any scaling
budget.

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

- [x] Establish, per pass, which side tables it writes while iterating.
- [x] Move the five tables behind copy-on-write builder handles and take handles in
      `constant_header_scope_context`.
- [x] Hoist the snapshots that are provably read-only for their loop. *Not needed: once a handle
      costs a refcount, a shared handle is strictly better than a hoisted snapshot. The
      function-signature and trait-requirement passes take handles.*
- [x] Replace the `module_constants` linear scan with the existing
      `resolved_module_constant_paths` set or its Phase H successor.
- [x] Re-point `ConstantPassSideTableEntriesCloned` at the per-header copy, which is the one that
      matters, or delete it if no copy survives. *Deleted: no copy survives.*
- [x] Wire `just bench-scaling` into `just validate` once `nominal_members` is within budget. The
      lane is deliberately not in the gate while it fails, because the gate must pass for every
      commit.

Acceptance:

- [x] `just bench-scaling` reports `nominal_members` within its `n^1.25` budget, replacing the
      `n^1.86` it reports today - it now reports `n^0.98`
- [x] `environment_stress` and `type_stress` improve - `ast.environment` `2.59x` and `3.49x`
- [x] no pass reads a side table snapshot taken before a write it depends on
- [x] diagnostics, warning order and emitted artefacts are byte-for-byte equivalent

Checkpoint: ownership only. No value representation or control-flow change.

### Phase B outcome

Landed. Full measurements in `benchmarks/frontend-optimization-results.md`, section
**Phase B Copy-On-Write Side Tables**. Durable findings:

1. **The quadratic cost is gone and the pass is linear.** `nominal_members` moved from `n^1.86` to
   `n^0.98`; the `320`-declaration point moved from `470.036ms` to `12.150ms` of `ast.environment`,
   a `38.7x` reduction. `type_stress` improved `3.49x` and `environment_stress` `2.59x` on the same
   metric.
2. **The whole fix was one ownership change.** The `ScopeContext` side already held these tables as
   `Rc<FxHashMap<..>>`; only the builder held them by value, so every call site was already asking
   for a handle and being handed a fresh copy. Five field types and nine `Rc::make_mut` write sites.
   No pass, signature or diagnostic changed.
3. **`Rc::make_mut` clones nothing here, and only the scaling lane can keep proving that.** The
   design depends on every `ScopeContext` handle being dropped before the next write. If one ever
   escaped its loop iteration the quadratic cost would return in full, with no test failing and no
   counter moving - which is exactly how it survived unnoticed until Phase A. `just bench-scaling`
   is now in `just validate`, so this is the first defect class in the repository with a standing
   automated guard.
4. **`docs` did not move, as Phase A predicted.** `96.754ms` to `97.171ms` of `ast.environment`.
   `docs` has 2 structs, so a per-nominal-header cost was never its cost. Phase C now owns the only
   large workload the plan has not improved, and its target list should be re-measured against the
   post-Phase-B tree rather than inherited from Phase A's ranking.
5. **`generic_declarations_by_path` had two owners and now has one - and the first attempt got its
   read-only claim wrong.** The map was copied out of `ModuleSymbols` per header, so it moved into
   the builder as a handle, which also deleted a `finish_environment` parameter that existed only to
   thread the map past a consuming `self`. The first version assumed no environment pass writes it.
   Import projection does, for every imported generic nominal, so the early move left that writer
   filling a map nobody read and seven cross-module generic tests failed. Routing the writer through
   the same handle fixed it. The lesson is narrow and worth keeping: *"read by everything, written
   by nothing"* is a claim to verify by grepping the writers, not to infer from the passes you
   happen to be reading. Only the integration suite caught it, because the failure needs a module
   boundary to appear.

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

- [x] Add `ConstValueId`, `ConstValueStore` and compact module-constant rows
      (`{ declaration: DeclarationId, value: ConstValueId }`).
- [x] Define scalar, collection, record, choice, range, option and string/template-folded payloads
      required by current language support. Fallible carriers are *not* current language support:
      the coverage closure established the shape is unauthorable, so the store no longer carries a
      payload for it.
- [x] Preserve type, provenance, const-record and location facts.
- [x] Make declaration lookup return constant identity without cloning its value tree.
- [x] Change module-constant references and field access to read the store.

Consumers:

- [x] Project exported constants directly from the store into `PublicFoldedValue` once.
- [x] Move the store and rows through the AST-to-HIR boundary.
- [x] Make HIR module constants reference or consume the same store.
- [x] Update config and direct `.mtf` compilation extraction to read the shared store.
- [x] Keep advisory body-local/inferred `AstConstFacts` separate from authored module-constant
      storage, but reuse `ConstValueId` where a value is retained.

Deletions:

- [x] Replace `module_constants: Vec<Declaration>` in environment/lookups/AST contracts.
- [x] Delete the `declaration.clone()` that exists only to own the constant twice.
- [x] Delete recursive `normalize_module_constant_expression`.
- [x] Delete the AST-expression-to-HIR-constant recursive conversion.
- [x] Consolidate public and HIR conversion walkers around one borrowed store visitor.

Acceptance:

- [x] each module constant has exactly one folded-value owner
- [x] public projection and HIR consume it without reparsing or deep intermediate clones
- [x] `ast_module_constant_declaration_clones` reaches zero because the declaration-clone
      instrumentation and duplicate representation were deleted
- [x] helper-only template constants are still excluded from the HIR handoff

Checkpoint: one folded-value authority with all old conversion paths removed. Control flow unchanged.

### Phase C outcome

Implementation complete. A fresh interim auditor reviewed the complete Phase C candidate and
returned clean. The Slice review was briefly reported blocked on audit-scope registration; AUD-0004
established it never was - see the status block. `ConstValueStore` now owns one indexed folded-value graph and
declaration rows for each module constant. Scalar, aggregate, option/fallible and template
payloads preserve the type, source, const-record, value-mode and synthetic-interface facts needed
by later stages. Public-interface projection, generic materialisation, config and `.mtf` extraction,
HIR constant-pool lowering, HIR references and field access all consume that store; advisory
body-local `AstConstFacts` retain boxed resolver expressions separately.

The old module-constant lookup vector, recursive production normalizer, AST-expression-to-HIR
constant walker and duplicate declaration ownership are gone. Template projection now retains
both the structured public pieces and the scalar folded emission from the same TIR fold, including
the missing-slot-as-empty runtime case. A focused HIR test confirms real `$insert` helpers remain
out of HIR while wrapper constants remain visible.

Validation: `just validate` passes with 4445 + 17 + 788 Rust tests, 1866 integration cases, source
audit 0 findings, a clean docs check, 74 benchmark preflights, nominal-members `n^0.96`,
constant-chain `n^0.82`, and clean timer erasure.

### Phase C review

The checkpoint was reviewed against a rebuilt `e782e79d8` before Phase D. Six items were found and
fixed; the full tables and reasoning are in `benchmarks/frontend-optimization-results.md` under
**Constant Evaluation And Type-System Plan - Phase C Folded-Value Authority**. What a later phase
must not rediscover:

1. **Phase C is a small consistent win and it was measurable all along.** Against a rebuilt
   before-binary: `docs` `ast.total` `-1.4%`, `finalise.module_constant` `-5.0%`,
   `public_interface.project` `-53.7%`; `constant_chain_512` `ast.total` `-4.5%` and `hir` `-15.4%`;
   `fold_stress` `-3.7%`; `template_stress` `-1.7%`. `docs` `frontend.hir` regressed `+0.5ms`
   because `lower_module_constants` now clones each path twice to release the store borrow. A
   changed `bench-check` comparison set is a reason not to quote `bench-check`, not a reason to
   leave a deletion phase unquantified.

2. **A user diagnostic had become an internal compiler error.** `final_value_kind` and
   `TemplatePreparationOutcome` are independent: a `RenderableString` or `WrapperTemplate` template
   can carry a `Runtime` outcome. The deleted normalizer branched on the outcome and emitted
   `NonFoldableConstTemplate`; the replacement branched on the kind alone and produced a
   `CompilerError` instead. Rejection now keys off whether preparation published the template,
   which is the fact that separates a constant value from a runtime one.

3. **Three test-only parallel paths were retained instead of the dead code being deleted.** A
   second HIR module-constant lowering path with its own cycle guard; the whole of
   `normalize_constants.rs` as a self-described "test-only compatibility helper"; and
   `resolve_explicit_top_level_constant`. Each survived under `#[cfg(test)]` so its unit tests
   would keep passing against code nothing ships - and retargeting the second set at the production
   owner is exactly what exposed finding 2. The cyclic-constant test asserted a rule Stage 3 owns
   as `MOTH-RULE-0053` with 23 committed integration cases.

4. **`ConstValueRow.declaration: DeclarationId` was never read.** Every consumer joins by defining
   `InternedPath`, so the store also kept a parallel `row_paths`. Deleting one dead guard exposed
   the whole unread chain - the field, `iter_module_constant_rows`, `iter_with_ids` and a
   `#[cfg(test)] DeclarationId::from_index`. The row is now `{ path, value }` and
   `environment/declaration_table.rs` is byte-identical to its pre-Phase-C state.

5. **A whole `TypeEnvironment` was deep-cloned on the success path** so a `map_err` arm could use
   it. It clones inside the closure now.

6. **A `template_ir_store` `Ref` guard outlived its use by about 250 lines**, spanning two later
   passes that hold the same `RefCell`. Neither borrows it mutably today; the guard is now scoped.

7. **Neither Phase B nor Phase C moved the largest frontend cost, and its ownership is split.**
   `docs` `ast.environment.constant_header_resolution` is `86.135ms` - `51%` of its AST time
   and a third of the whole check - against `36.772ms` for `ast.emit.const_template_parse` and
   `17.607ms` for `ast.finalise.module_constant`. Phase A finding 5 recorded the number; this
   confirms it survived both phases (`-0.2%` across C) and states its share. Phase A also
   established that Phase D cannot be validated on `docs` at all, because `docs` has
   `ast_expression_fold_items = 0`. Phase D's first two blocks are therefore counter-verified
   deletion work on fixtures under `3ms`, while its third block - lazy diagnostics and explicit
   type-resolution views - is the only part of the remaining plan that touches the code this cost
   runs through. Weight Phase D accordingly.

   Ownership is split rather than absent, and an earlier revision of this item wrongly called it
   unowned. Phase D block 3 intersects the resolution path; Phase H owns the two constant-context
   items Phase 1 carried forward - the synthetic `AstModuleLookups` and the per-constant
   `ScopeContext` construction. Re-profile after Phase D block 3. If the cost is still dominant,
   Phase H must *measure* the synthetic-context path rather than treat it as clarity work, or the
   residual needs an explicit follow-up owner named at closeout. One concrete candidate is already
   visible in the implementation: every constant calls `ScopeContext::new`, which builds a complete
   synthetic `AstModuleLookups` of empty maps, registries and environments that the constant session
   then overwrites with its already-prepared shared handles. Phase 1 showed setup was not the main
   constant-chain cost, so do not assume this accounts for the whole `86ms` - measure it.

### Phase C coverage closure

The Phase C review left the folded-value paths under-covered. Coverage was measured empirically by
instrumenting every arm of `ConstValueStore::insert_expression`, `const_template_value_from_projection`
and the two HIR constant visitors in `hir/hir_statement/declarations.rs`, then running the whole
integration suite in one process. Every consumer boundary was already covered; every gap was a
specific *value shape*. Two of those gaps were hiding real defects.

1. **Constant `OptionSome` emitted invalid JavaScript.** The HIR const path built
   `HirVariantField { name: None, .. }`, which the JS backend renders as `{ tag: "some", 7 }`. Every
   runtime `VariantConstruct` producer interns `"value"`, and `VariantPayloadGet` reads that key
   (`backends/js/js_expr.rs`). It survived because `moth check` is frontend-only and no committed
   case referenced an optional module constant *from a body* - `constant_compile_time_optional`
   declares one and never reads it.

2. **`none` was classified `NonConst`, rejecting a documented binding form.**
   `Expression::const_value_kind_with_template_classifier` grouped `ExpressionKind::OptionNone` with
   `NoValue` and the runtime kinds, so the canonical example in
   `docs/src/docs/constants/constant-bindings.mtf` - `maybe_name #String? = none` - failed with
   `MOTH-RULE-0053`. The whole folded-value pipeline already represented the value end to end
   (`ConstValuePayload::OptionNone`, `PublicFoldedValue::OptionNone` with its import projection,
   `HirConstValue::OptionNone`); the classification was the only thing rejecting it. `none` is now a
   `Literal`, which is what made that machinery reachable at all.

Four fixtures now pin the shapes that were unreached: `constant_compile_time_range_folding`,
`constant_compile_time_char`, `constant_compile_time_optional_reference` and
`constant_compile_time_none`.

What the same measurement established about paths that stay unreached, so a later phase does not
re-derive it:

- **The store's not-published rejection is unreachable from source.** The module-constant header
  gate (`environment/constant_resolution.rs`) admits a template when `final_value_kind != NonConst`;
  the store rejects when the *outcome* is `Runtime`. For a const-evaluable kind the outcome is only
  `Runtime` if a reason was recorded on facts that stayed `const_evaluable` - the non-foldable
  conditional child wrapper set and the attached runtime slot plan. Neither is authorable in a
  module constant: every source shape that would produce one is rejected upstream by the const
  reference gate. No case in the suite, and no template in `docs`, produces the combination. The
  guard stays a user diagnostic rather than a `CompilerError` precisely because the two
  classifications are independent - if that seam ever opens, a user must not get an ICE.

- **`TemplateConstValueKind::RenderableString` is preempted, not unused.** A fully renderable
  module-constant template folds to a string *before* the store sees it, arriving as
  `ExpressionKind::StringSlice`. Only wrapper and slot-insert templates survive as `Template`
  expressions. `ConstTemplateValue::Folded` is therefore unreached in production.

- **Fallible-carrier constants are not authorable, and the store no longer models them.** The only
  AST constructor of `ExpressionKind::FallibleCarrierConstruct` is
  `Expression::result_construct_with_type_id`, itself `#[cfg(test)]` and documented as a lowering
  fixture. The coverage closure first recorded the `#[cfg(test)]` arms that handled the shape as
  "correctly gated"; that conclusion was wrong against the higher-authority style guide, which
  states that production files must not grow test-only semantic variants. Establishing the shape is
  unauthorable is precisely the argument for deleting it, not for gating it. `ConstValuePayload::
  FallibleCarrier`, `ConstValueVisit::FallibleCarrier`, `HirConstValue::Result` and
  `hir::expressions::FallibleCarrierVariant` are gone, along with their five store arms and two HIR
  visitor arms. No test constructed any of them. The store's fixture constructors moved from
  `store.rs` to `const_values/store/test_support.rs`.

- **Non-option `ConstValuePayload::Coerced` is unreachable.** The only coercion reaching a module
  constant is `T -> T?`, which `insert_expression` classifies as `OptionSome`. Numeric widening
  (`ratio #Float = 3`) retypes the literal instead of producing `ExpressionKind::Coerced`, and
  nested options are a syntax error.

- **`Range` values outside a loop header are a deferred surface.** A folded `Range` is not a
  collection-loop source (`MOTH-SYNTAX-0029`), exposes no members (`MOTH-RULE-0048`) and is not
  renderable in a template head (`MOTH-SYNTAX-0022`), so binding it is the only shape that reads
  one. First-class range values are intended but not designed. The progress matrix now carries the
  row; `constant_compile_time_range_folding` deliberately pins only that the folded value reaches
  HIR, and no further range coverage should be added until the surface is shaped.

### Phase C corrections before Phase D

A review of the Phase C checkpoint raised four code corrections, all on the folded-value
representation Phase D builds on. They were applied before Phase D started, because fixing them
afterwards would have caused churn in the code Phase D touches.

1. **Test-only semantic shapes left production files.** Covered in the coverage-closure notes
   above: the fallible-carrier payload chain was deleted rather than gated, and the store's three
   fixture constructors moved to `const_values/store/test_support.rs`.

2. **Trusted `ConstValueId`s were exposed through fallible `Option` APIs, and three callers
   invented semantics for a miss that cannot happen.** A module-constant row's id was minted by the
   store, and its value node was pushed before the row, so its metadata always exists. Yet
   `is_hir_visible` read a missing row as "not HIR visible", config extraction silently skipped the
   row, and `extract_config_declaration` reported it to the user as `NotCompileTimeConstant` at a
   default location - a compiler-corruption state rendered as a source mistake, which the
   diagnostic boundary forbids. `iter_module_constant_views` now yields `{ path, id, metadata }`
   together, so the four row consumers do no second lookup and have no miss branch to get wrong.
   `is_hir_visible` is deleted; its one caller reads `row.metadata.hir_visible`.

   Not changed: `metadata`, `payload`, `field_value` and `string_value` for ids that came from
   `value_for_path` or a payload child. Those callers already fail with `CompilerError`, which is
   correct, and the remaining `Option` returns in `build_system/project_config/validation.rs` sit
   behind an error channel that is `InvalidConfigReason`/`CompilerDiagnostic` by design. Threading
   `CompilerError` through that module is a separate change to its error plumbing, unrelated to the
   folded-value representation.

3. **The documented `+0.5ms` `docs` HIR regression is partly recovered, and measured rather than
   assumed.** `lower_module_constants` collected every row into an owned `Vec`, cloning each
   constant's `InternedPath` purely to end the store borrow, then took and restored the store again
   for every value. The store now leaves `self` once for the whole pass and the loop reads borrowed
   rows. Against a rebuilt `28ab27f0a`, `docs` `frontend.hir` goes `3.36ms -> 3.20ms`, `-4.8%` -
   which is `-0.16ms` of the `+0.535ms`. The clones were about a third of the regression, not all of
   it. The rest sits elsewhere in the new lowering path and is not worth chasing at `3ms` on a
   `260ms` compile; a later phase should not re-attribute the residual to path cloning.

4. **A Phase G fixture comment misstated the architecture.**
   `static_if_inactive_branch_generic_call` said the inactive branch's "borrow rules" still run.
   Both branches get Stage 4 validation; borrow validation is a later stage and sees only the
   selected branch, which is what Phase G and the accepted contract both say. The fixture's
   behaviour was already correct - only the prose was wrong, and Phase G may be implemented from
   that prose.

### Phase C Slice review

Run per the `AGENTS.md` required workflow: re-check ownership, duplication, stale paths and test
gaps. Two items, both fixed in the review commit.

1. **Two row iterators over the same rows.** `iter_module_constants` yielded `(path, id)` and
   `iter_module_constant_views` yielded `{ path, id, metadata }`. The tuple form had one production
   caller and eight test callers, every one of them a find-by-short-name followed by a separate
   store lookup - which is the shape the view was introduced to remove. All nine moved to the view
   and the tuple iterator is deleted, so the store has one row-iteration API.

2. **`index.md` described `const_values` as the "const fact resolver".** Phase C made that
   directory the module-local folded-value authority; the advisory resolver is now the smaller half
   of what it owns. The entry names the store, its borrowed views and the resolver.

Ownership, stale paths and test gaps were clean. The store has one owner per module and one
constructor; no deleted API left a caller behind; the coverage closure above already established
value-shape coverage empirically, so no new unit fixture was warranted.

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

**Store lifecycle - decided, do not rediscover.** An earlier revision of this phase said "make the
fold evaluator produce `ConstValueId` directly" while also gating an `ExprId` arena on evidence.
Those two instructions could not both be followed, because the store does not exist when folding
runs. `constant_fold` runs during expression construction and returns `Vec<ExpressionRpnItem>`;
`evaluate_expression` returns an `Expression`. `ConstValueStore` is built much later, in
`AstFinalizer`, from already-folded declaration `Expression`s - and it must stay there, because
template payloads need TIR views that only exist after template normalization.

So, for this phase:

- `AstFinalizer` remains the **sole** constructor of `ConstValueStore`. There is exactly one store
  per module and it comes into existence at the same point it does today.
- Only authored module constants enter the store. Body-local and inferred fold results stay in
  `AstConstFacts`; they are advisory, they are not published, and admitting them would make the
  store a general expression arena by the back door.
- A folded value remains represented in the AST as an `Expression` between folding and store
  insertion. Phase D shrinks what that fold *builds* - no diagnostic spelling, no operand clones,
  compact typed postfix - it does not change who owns the folded value.
- Do **not** add a second temporary fold-value arena that is later copied into `ConstValueStore`.
  That would recreate the duplicate representation Phase C deleted.

If a later phase wants folding to allocate directly into the store, that is a lifecycle change to
propose and measure on its own, not a side effect of a representation change.

**Measured attribution, done first - do not rediscover.** Before touching any checkbox below, the
`86ms` `constant_header_resolution` cost was split by direct instrumentation of
`resolve_constant_header` on `docs` (`545` constants, release `profiling` build,
`RAYON_NUM_THREADS=1`):

| Region | Total | Share |
|---|---|---|
| `constant_header_scope` - `ScopeContext::new` and the synthetic `AstModuleLookups` | `6.23ms` | `7.3%` |
| `resolve_declaration_syntax` - parse and fold the initializer | `71.60ms` | `83.8%` |
| `const_value_kind_with_template_classifier` | `0.055ms` | `0.06%` |
| unattributed (warning drain, loop, timing guard) | `~7.5ms` | `~8.8%` |

Three things follow, and they change what the rest of this phase and Phase H should do:

1. **The Phase H synthetic-context candidate is `7%`, not the cost.** **Phase C outcome** finding 7
   named `ScopeContext::new` as a visible candidate and said to measure rather than assume. Measured:
   `6.2ms` of `86ms`. Worth deleting on clarity grounds, not a performance phase on its own.
2. **This cost is not type resolution and not diagnostic spelling.** `docs` runs
   `ast_type_resolution_calls = 605` for `545` constants - one per constant. The lazy-diagnostic and
   explicit-type-resolution-view block below is still worth doing on its own merits, but **Phase C
   outcome** finding 7 was wrong to call it "the only part of the remaining plan that touches the
   code this cost runs through". The code this cost runs through is the template parser and the TIR
   fold reducer, which this plan does not own.
3. **The remaining `71.6ms` is template output amplification.** `docs` parses `894KB` of template
   text and folds it into `9.88MB` of output across `10853` folds, with one string-intern call per
   fold. Nested templates re-emit and re-intern their whole subtree at each enclosing level, so
   innermost text is copied once per level of nesting. Two concrete defects were found and are
   recorded under **Phase D deletions and findings** below. Fixing the amplification itself belongs
   to the template plan, not here; do not start it under this plan without re-scoping.

**Counter reading - methodological trap.** `MOTH_BENCH counter` emits **one line per module**, not
one line per project. `AstCounter` storage is a thread-local that `Ast::build` resets at the start
of every module and publishes at the end. Reading the first block of counter lines reads one
module. Every counter figure in this plan for a multi-module workload must be summed across the
lines. The single-module stress fixtures are unaffected, so the earlier `960/960` and `392/392`
ratios stand.

Ownership cleanup - **done**, counter-verified:

- [x] Make expression ordering reserve from the known input count. The output queue takes the
      input length exactly; the operator stack takes half it, which is the arity bound.
- [x] Make `constant_fold` consume its item vector. It now takes `Vec<ExpressionRpnItem>`.
- [x] Move non-foldable operands and operators back into the runtime result.
- [x] Return the sole folded operand by move.
- [x] Add focused tests proving no source or synthetic provenance is lost. Three tests in
      `const_eval/tests/constant_folding_tests.rs` pin the moved-back operands, the moved-back
      operator and the folded result to their authored locations and value modes, using
      deliberately distinct anchors so a rebuilt node would fail them.
- [x] Drive full-expression clone counters to zero in ordinary arithmetic fold paths.
      `ast_expression_operand_clones` is now `0` on every committed fixture.

Typed postfix - partly done:

- [x] Emit a compact typed postfix item carrying only semantic IDs, flags and diagnostic anchors.
      The typing stack is now `Vec<TypeId>`. `ExpressionResultType`, which paired every `TypeId`
      with a `DataType` spelling, is deleted: every operator policy already decided on `TypeId`
      alone, and the spelling had exactly one consumer.
- [x] Validate the whole typed expression before executing fold operations. Unchanged in
      ordering, but now true of a stack that carries no diagnostic payload.
- [ ] Resolve operator input/result `TypeId`s as operators leave the shunting-yard stack.
- [ ] Delete the separate rich RPN result-type scan. The scan is no longer rich, so what is left
      to delete is the second walk itself - see finding 6 below before starting.
- [ ] Return the folded scalar as a compact typed value rather than a rebuilt `Expression` tree,
      within the store lifecycle fixed below.
- [ ] Preserve reduced postfix only for runtime-dependent work.

Advisory constant environment - **measured, then rebuilt as a deletion. Done; see finding 7.**

*What it looked like when the Phase C review escalated it:* `ConstFactCollector` reconstructed
every substitutable module constant back into a rich `Expression` via `expression_for_store_value`,
stored those in `module_explicit_env`, cloned the whole environment for every function and start
body, then cloned it again for each nested `if`, block and other scope. `ConstValueEnvironment` was
`FxHashMap<InternedPath, Expression>`, and the store's bridge said outright that it existed because
the advisory resolver still speaks `Expression`. That is exactly the temporary rich reconstruction
and per-scope cloning this phase exists to remove - which is why it was escalated, and why it was
measured before it was believed.

It was instrumented first, per the checkbox below, and the pathological shape does not occur. On
`docs` (`545` module constants, `73` modules) the whole of `ConstFactCollector::collect` costs
`0.60ms` of a `168ms` AST build - `0.36%`:

| region | `docs` total |
|---|---|
| `collect_explicit_top_level_facts` | `0.497ms` |
| of which `expression_for_store_value` rebuilds (545) | `0.090ms` |
| `collect_private_and_body_local_facts` (every scope clone) | `0.105ms` |

Environment copying across the committed fixtures, by the durable counters added for this checkbox:
`docs` `78` clones / `640` entries, `speed_test` `55` / `2654`, `deep_scope_churn` `26` / `260`,
`one_module_kitchen_sink` `18` / `136`, `constant_dag_churn` `2` / `176`. `module_explicit_env` is
**per module**, so `docs` averages `8` visible constants per environment, not `545`. The
*many constants x many scopes* shape needs both in one module and no committed fixture has it.

So this block stays, but as a **deletion and single-representation change only**. Do not describe
it as a hot-path optimisation, do not add a fixture to manufacture the shape, and do not spend
effort on the copying - the copying is `0.1ms`.

- [x] Instrument the environment payload copied per scope. `ast_const_fact_environment_clones` and
      `ast_const_fact_environment_entries_cloned` are durable counters, so the claim above stays
      re-verifiable rather than resting on this prose.
- [x] Replace `ConstValueEnvironment<Expression>` reconstruction with compact stored-value and
      local-fold references. A module base plus lightweight lexical overlays, or parent-linked
      frames, is enough - do not build a persistent map architecture. Justification is one
      representation of an already-folded value, not speed. Done as a module base plus one
      lexical overlay: `ConstValueEnvironment` holds `Rc<FxHashMap<InternedPath, ConstValueId>>`
      for authored module constants and `FxHashMap<InternedPath, Expression>` for the bindings a
      scope introduced itself. Entering a scope copies the overlay and bumps an `Rc`.
- [ ] ~~Delete `expression_for_resolution`~~ - **not achievable in this phase, and the reason is
      structural.** The advisory resolver substitutes constants into `ExpressionRpnItem::Operand`
      and hands them to `constant_fold`, which is an `Expression` interface. Making the fold
      evaluator consume store values instead is the store-lifecycle change that
      **Store lifecycle - decided, do not rediscover** puts outside this phase. What did change is
      *when* it runs: it is called at the reference that consumes a value rather than once per
      module constant up front, so it is now sized by references rather than by declarations.
      Do not retry this checkbox without proposing the lifecycle change first.

Lazy diagnostics and explicit type-resolution views - **measured, and it is not a performance
item either**. `resolve_type` was instrumented before any of it was started, per rule 1. It costs
`489us` of a `38.68ms` `ast.environment` on `nominal_scaling_320`, the fixture that calls it
`13924` times - `1.3%`, about `35ns` a call. On `docs` it is roughly `20us` of a `168ms` build,
and on `type_stress` `21.7us`. There is no committed fixture on which this code is material. See
finding 8 below.

What survives is the one deletion the block named concretely; the rest is a structural argument
about representable states, which is a legitimate reason to do it but not a reason to do it here.

- [x] Remove the remaining owned `visible_declaration_ids` copy in
      `type_resolution/struct_fields.rs:316`, which exists only because
      `TypeResolutionContextInputs` carries a borrow where the scope needs a handle. Done by
      carrying `Option<&Arc<FxHashSet<InternedPath>>>` - the handle the scope already stores -
      so entering field-default evaluation shares the set instead of copying every visible path.
- [ ] Split the broad optional `TypeResolutionContextInputs` shape into explicit data views:
      immutable declaration/visibility lookup, mutable derived-type interning, optional generic
      scope, optional trait/evidence overlay, optional constant-value lookup.
- [ ] Add named constructors for module declaration, constant, body and generated contexts, so
      invalid combinations are unrepresentable or rejected at construction.
- [ ] Return `TypeId`-first results from successful lookup paths and construct diagnostic spelling
      only at the error or public-display boundary.
- [ ] Borrow resolved aliases, fields, variants and signatures instead of cloning them for
      read-only validation.

The four open items above should be **re-proposed or dropped, not executed as written**. They read
as performance work and are not; there are `59` clone sites across `type_resolution/`, and auditing
them for read-only borrows is an unbounded refactor with nothing measured behind it. If the
representable-states argument is worth acting on, it belongs with Phase F, which owns the type
environment, and it should be justified as a correctness and clarity change with its own acceptance
criteria.

Acceptance:

- [x] `ast_expression_operand_clones` and `ast_diagnostic_data_type_materialisations` fall well
      below `ast_expression_fold_items`. Measured per fixture under finding 4 below: clones are
      `0` everywhere, materialisations fall from exactly `1:1` to between `0%` and `24%`.
- [x] diagnostic text, ordering and priority are unchanged. One internal `ErrorType::Compiler`
      message lost its pre-fold input dump, because the folder no longer owns it after the loop
      begins; no user-facing diagnostic changed.
- [x] shunting yard remains the one precedence algorithm

Checkpoint: typed-postfix and lazy diagnostic data with unchanged source semantics. **Reached**,
with two of the four blocks completed as specified, one completed after measurement changed its
justification from speed to single representation, and one retired as a performance item with its
four remaining items marked for re-proposal. See **Phase D outcome**.

### Phase D deletions and findings

Recorded as they land. This phase is in progress; the full outcome is written at its checkpoint.

1. **The TIR fold cache was deleted. It took `1` hit in `11275` fold attempts.** `tir/fold_cache.rs`
   memoised `fold_exact_view` on `(TirViewIdentity, loop limit, bindings-empty)`. Measured across
   every committed template workload: `docs` `0/10853`, `template_stress` `0/78`,
   `template_render_plan_churn` `0/51`, `code_highlighter_stress` `0/1`,
   `one_module_kitchen_sink` `0/7`, `speed_test` `1/285`. The cache was not broken - a repeated
   child reference inside one template does hit, and a unit test proved it - real source simply
   almost never folds the same exact view twice. It cost a `HashMap` per fold context at three
   construction sites, a hash and probe per fold, and a `TemplateFoldResult` clone per fold.

   An intermediate experiment is worth not repeating: widening the cache to process lifetime aborts
   the compiler. `TirViewIdentity` is **module-local**, so a root id from one module resolves
   against another module's store. Any cache over this key is bounded by one `TemplateIrStore`.

   Interleaved A/B against a rebuilt `bed00e0bf`, isolated target directories, median of 9,
   `RAYON_NUM_THREADS=1`: `docs` `ast.total` `168.273 -> 166.712ms` (`-0.93%`),
   `constant_header_resolution` `86.289 -> 85.169ms` (`-1.30%`), `ast.emit.const_template_fold`
   `3.087 -> 2.666ms` (`-13.66%`); `template_stress` `ast.total` `3.043 -> 2.875ms` (`-5.53%`).

2. **The advisory constant environment is a `0.6ms` path, not a hot one.** Full numbers and the
   durable counters are under **Advisory constant environment** in the phase body above. The short
   version: the Phase C review escalated it, measurement retired it as a performance item, and the
   remaining justification is single-representation deletion. This is the plan correcting one of
   its own findings, which rule 4 requires be recorded rather than quietly dropped.

3. **`TemplateIrSummary::estimated_output_bytes` under-predicts by about `3x`, and structurally so.**
   `record_text_node` adds the node's own bytes; `record_child_template` adds nothing. A parent's
   estimate therefore excludes every child's output. On `docs`: estimated `3.24MB`, actual `9.88MB`,
   recorded miss `6.63MB`. `FoldOutputState::with_capacity(estimated_bytes)` sizes the fold buffer
   from that number, so template-heavy folds regrow their buffers. Not fixed here - propagating
   child estimates is template-plan work and the win is bounded by the reallocation cost, not by
   the `6.63MB`. Recorded so it is not re-derived from the counters.

4. **Move-only folding landed, and the `1:1` diagnostic waste is gone.** The two counters this
   phase was justified on, per fixture, before and after:

   | fixture | fold items | operand clones | | diagnostic materialisations | |
   |---|---|---|---|---|---|
   | | | before | after | before | after |
   | `fold_stress` | `960` | `780` | `0` | `960` | `0` |
   | `speed_test` | `804` | `613` | `0` | `780` | `41` |
   | `type_stress` | `81` | `54` | `0` | `75` | `25` |
   | `template_stress` | `21` | `13` | `0` | `21` | `5` |
   | `constant_chain_512` | `1533` | - | `0` | - | `0` |

   Three deletions did it. `constant_fold` takes its item vector by value, so every operand and
   operator either moves onto the fold stack or is moved back into the runtime result - the
   `to_owned()` per item is gone. `evaluate_expression` pops the sole folded operand instead of
   cloning `stack[0]`. And `ExpressionResultType` - a `(DataType, TypeId)` pair built once per
   RPN item - is deleted in favour of a `Vec<TypeId>` typing stack, because every operator policy
   already decided on `TypeId` and the `DataType` had exactly one consumer: the partial-fold
   runtime node, which now builds its spelling itself. `diagnostic_type_spelling` also returns an
   owned `DataType` that the old constructor cloned a second time.

   `ast_expression_operand_clones` survives with new call sites rather than being deleted, and
   its meaning narrowed. Two template callers - `resolve_expression_in_rpn` and
   `fold_substituted_runtime_condition` - rebuild a runtime node from the items *as they stood
   before the fold* when folding does not reduce to one value, so they must keep a copy. They are
   the only contributors left, they are `0` on every committed fixture, and both now return the
   surviving expression by move instead of cloning it.

5. **The wall-time A/B on this change is not separable from noise, and the control run is why.**
   Interleaved against `a1cc58cf2`, isolated target directories, median of 9 (7 for `docs`),
   `RAYON_NUM_THREADS=1`:

   | case | metric | before | after | delta |
   |---|---|---|---|---|
   | `constant_chain_512` | `ast.total` | `4.088ms` | `4.025ms` | `-1.55%` |
   | `constant_chain_512` | `constant_header_resolution` | `2.619ms` | `2.506ms` | `-4.31%` |
   | `speed_test` | `ast.total` | `12.092ms` | `12.128ms` | `+0.30%` |
   | `docs` (control) | `ast.total` | `166.441ms` | `165.474ms` | `-0.58%` |

   `docs` executes none of the changed code - `ast_expression_fold_items`,
   `ast_expression_typed_stack_items` and `ast_diagnostic_data_type_materialisations` are all
   `0` there, before and after - and it still moved `-0.58%`. A control that cannot move moving
   by a third of the treatment effect means the treatment effect is not measurable on these
   fixtures. **This improvement is recorded as counter-verified only**, which is what the phase
   body already permits. Do not quote the `constant_chain_512` numbers as a result.

6. **Every `docs` expression takes the single-operand fast path.** `ast_expression_typed_stack_items`
   is `0` across all `73` modules while `ast_expression_ordering_input_items` is `1322`, so every
   authored expression in the largest real workload is one RPN item and never reaches operator
   typing or folding at all. This bounds the remaining typed-postfix work: deleting the second
   RPN walk cannot help `docs`, and the fixtures where it would help are the sub-`5ms` ones
   finding 5 just showed are unmeasurable. Treat what is left of that block as a
   single-representation deletion, on the same footing as the advisory constant environment.

7. **The advisory environment now holds one representation, and per-scope copying went to zero.**
   `ConstValueEnvironment` was `FxHashMap<InternedPath, Expression>` and was rebuilt from the
   store for every module constant before any body was walked, then copied whole into every
   nested scope. It is now a shared `Rc` module base of `ConstValueId` plus a per-scope overlay
   of the bindings that scope introduced itself, so a module constant has exactly one
   representation - the store's - until a reference actually materialises one.

   `ast_const_fact_environment_entries_cloned`, by fixture:

   | workload | env clones | entries copied before | after |
   |---|---|---|---|---|
   | `docs` | `78` | `640` | `0` |
   | `speed_test` | `55` | `2654` | `14` |
   | `deep_scope_churn` | `26` | `260` | `0` |
   | `constant_dag_churn` | `2` | `176` | `0` |
   | `one_module_kitchen_sink` | `18` | `136` | - |

   The eager rebuild is gone with it: `docs` built `545` throwaway `Expression`s per module
   finalization and now builds one per reference that needs one. `ConstValueResolver::
   expression_for_store_value`, the pass-through that existed only for the eager loop, is
   deleted; the resolver holds `&ConstValueStore` and materialises at the reference.

   No wall-time claim is attached, and none should be: finding 2 measured this whole path at
   `0.6ms` of `168ms` before any of it was changed. This is the single-representation deletion
   the plan reduced the block to, and the counters are its evidence.

   Coverage gap closed while doing it: no test covered a body-local declaration that references a
   module constant, which is precisely the path this change reroutes. `const_fact_tests.rs` now
   has two - a bare reference and an arithmetic expression that folds over one.

8. **Type resolution is not a hot path on any committed fixture, including the one that calls it
   `13924` times.** The lazy-diagnostics block was the last part of Phase D still described as
   performance work, so `resolve_type` was instrumented before it was started - a reentrancy-safe
   probe that accumulates only non-nested entries, so recursion is counted once.

   | fixture | `ast_type_resolution_calls` | `resolve_type` total | enclosing stage |
   |---|---|---|---|
   | `nominal_scaling_320` | `13924` | `489us` | `38.68ms` `ast.environment` (`1.3%`) |
   | `docs` | `605` | `~20us` summed over 73 modules | `168ms` `ast.total` |
   | `type_stress` | `698` | `21.7us` | `~3ms` `ast.total` |

   About `35ns` a call. `nominal_scaling_320` was the promising case - it is the fixture the
   `nominal_members` scaling series budgets, and it makes `23x` the calls `docs` does - and it
   still comes out at `1.3%` of the stage it runs in. Whatever makes `ast.environment` cost
   `38.68ms` there, it is not `resolve_type`; that is a question for Phase E.

   > **Corrected by finding 9. The `nominal_scaling_320` row of this table is probe-inflated and
   > its percentage is wrong.** The stage is `12.28ms`, not `38.68ms`: the probe that produced
   > `489us` cost about three times the function it was measuring, and it inflated its own
   > denominator. `resolve_type` is therefore up to `4.0%` of that stage, not `1.3%`. The
   > conclusion of this finding survives the correction - `4%` is still not a hot path, and the
   > `docs` and `type_stress` rows are unaffected because their probes ran against `ast.total`,
   > a denominator thousands of times larger than the probe. What does not survive is the number
   > Phase E was pointed at. See finding 9.

   This retires the last block of Phase D as a performance item. The pattern across findings 2, 5,
   6 and 8 is consistent enough to state plainly: **every candidate this plan identified by reading
   the code turned out to be small, and the one cost that is large - template output amplification
   - was found only by measuring.** A future phase should instrument before it plans, not after.

### Phase D outcome

Phase D set out to stop building rich intermediate data around correct algorithms. It did that, and
it also spent more of its effort measuring than changing - which was the right ratio, because three
of its four blocks turned out not to be what the plan thought they were.

**What was deleted.** `constant_fold` consumes its item vector, so nothing on the ordinary
arithmetic path copies an `Expression`; `evaluate_expression` returns the sole folded operand by
move. `ExpressionResultType` is gone and the typing stack is `Vec<TypeId>`. The TIR fold cache is
gone. `ConstValueResolver::expression_for_store_value` is gone, and the advisory environment holds
a shared `Rc` base of `ConstValueId` rather than a rebuilt `Expression` per module constant.
`TypeResolutionContextInputs` carries the visibility handle the scope stores instead of a borrow it
had to copy. Expression ordering reserves exactly what it needs.

**What the counters say.** `ast_expression_operand_clones` is `0` on every committed fixture.
`ast_diagnostic_data_type_materialisations`, which was exactly `1:1` with fold items on every
fixture that folds anything, is `0` on `fold_stress` and `constant_chain_512` and between `5` and
`41` elsewhere. `ast_const_fact_environment_entries_cloned` is `0` on `docs`, `deep_scope_churn`
and `constant_dag_churn`, and `14` on `speed_test`.

**What no wall-time claim is made for, and why.** All of it. Finding 5 records the interleaved A/B
that could not clear its own control: `docs`, which executes none of the fold or typing code, moved
`-0.58%`, a third of the effect measured on the fixture that exercises the change hardest. The
phase body permitted a counter-verified result from the start, and that is what this is.

**What the phase corrected in its own plan.** Four things, all recorded rather than quietly
dropped, as rule 4 requires. **Phase C outcome** finding 7 was wrong that the lazy-diagnostic block
was the only remaining work touching the `86ms` constant pass (finding 4's attribution). The
advisory constant environment was escalated as a hot path and is `0.6ms` (finding 2). The
Phase H synthetic-context candidate is `7%` of that pass, not the pass (**Measured attribution**).
And the lazy-diagnostic block itself is not performance work at all: `resolve_type` is `1.3%` of
its enclosing stage on the fixture that calls it `13924` times (finding 8).

**The generalisation, which the next phase should act on.** Every candidate this plan identified by
reading the code came in small or at zero. The one cost that is large - template output
amplification, `71.6ms` of an `86ms` pass - was found only by measuring, and it belongs to a
different plan. Phase E should attribute `ast.environment` on `nominal_scaling_320` before it
restructures anything: `38.68ms` is real and `489us` of it is accounted for.

### Phase D Slice review

- **Ownership.** `ConstValueEnvironment` gained a second lookup method, but the two have distinct
  roles - overlay first, module base second - and `resolve_reference` is the only caller of either.
  The store remains the sole owner of a folded module constant; nothing here made a second one.
- **Duplication.** The pass-through `ConstValueResolver::expression_for_store_value` was the only
  duplicate path introduced by earlier phases and it is deleted. `ExpressionResultType` was a
  second representation of a resolved type and is deleted. No new alternative path was added.
- **Stale paths.** `index.md` needed no change: it describes directories, and both
  `const_eval` and `type_resolution` entries remain accurate. `docs/roadmap/evidence/
  test_honesty_inventory.json` still names `tir/tests/fold_cache_tests.rs`, renamed in Phase D -
  but it carries `generated_at` and `base_revision`, so it is a dated snapshot of a past run and
  the old name is correct *for that run*. It is not stale; leave it.
- **Audit log.** No audited-area row covers what this phase changed. `src/compiler_frontend/ast/**`
  is on the never-audited list; `feature.runtime_assertion_messages` covers three
  `ast/expressions/` files, none of which this phase touched. `tests.support` (AUD-0001, already
  `partial`) covers test helpers, and one helper signature changed - not a material change to what
  that audit recorded, which was helper redundancy.
- **Progress matrix.** Not edited, correctly: everything here is a refactor with no change to
  implementation status, rejection behaviour, backend coverage or test coverage of language
  features.
- **Test gaps, found and closed.** Two. Nothing pinned the provenance of operands that constant
  folding moves back into a runtime result, which is precisely what could silently become a
  reconstruction once folding consumed its input - three tests now do, using deliberately distinct
  source anchors and value modes. And nothing covered a body-local declaration that references a
  module constant, which is the path the advisory environment change reroutes - two tests now do,
  a bare reference and an arithmetic expression that folds over one.

## Phase E attribution - `ast.environment` on `nominal_scaling_320`

Phase E's entry condition was to attribute this stage before restructuring any member shell. That
is done, and it produced one correction and one confirmation.

9. **The `38.68ms` this phase was pointed at does not exist. The stage is `12.28ms`, and the
   earlier figure was the measuring probe.** Median of nine, `--profile profiling`,
   `RAYON_NUM_THREADS=1`, on a clean tree at `cc8d27f9d`:
   `frontend.ast.environment = 12.28ms`, `frontend.ast.total = 15.51ms`.

   Three explanations were tested before the probe was accepted as the cause:

   - **A counters-enabled binary.** Rebuilt with `detailed_timers,benchmark_counters` in an
     isolated target directory: `12.33ms`. Not it.
   - **The Phase D visibility-set sharing (`0c3837773`) landing after the measurement.** This was
     the plausible one - `320` structs of six fields each, previously copying the whole visible-path
     set per field-default evaluation. An interleaved A/B against a worktree built at the parent
     commit `8f4c01b88`, nine pairs: before `12.28ms`, after `12.28ms`. Not it either; this fixture
     declares no field defaults, so it never enters that path.
   - **The temporary `resolve_type` probe from finding 8**, which was compiled into the binary that
     produced `38.68ms` and is the only remaining difference. `13924` calls into a probe costing
     roughly `1.9us` each accounts for the missing `26ms` almost exactly.

   The methodological rule this earns, which is not the same as the one already recorded about
   per-module counter lines: **a probe whose per-call cost is a significant fraction of the
   function it measures corrupts the denominator as well as the numerator.** Finding 8 read
   `489us / 38.68ms` and reported `1.3%`; the honest reading is `489us / 12.28ms`, up to `4.0%`.
   The conclusion held anyway, but only by luck of margin. Attribution probes from here should be
   placed per-pass, not per-call, unless the per-call cost has been shown to be negligible against
   the callee - the Phase E probe below runs `24` marks per module rather than `13924`.

10. **Phase E's premise is correct, and it is the first candidate in this plan that measuring has
    confirmed rather than retired.** A 24-mark probe across every top-level step of
    `AstModuleEnvironmentBuilder::build`, plus function-entry guards one level down. The probe
    total reconciles with the stage to within `0.5%`, so nothing is unattributed.

    | step | ms | share of stage |
    |---|---|---|
    | `resolve_nominal_members_and_constants` | `7.39` | `58.1%` |
    | `register_nominal_shells` | `4.61` | `36.3%` |
    | `validate_nominal_generic_bound_surfaces` | `0.58` | `4.6%` |
    | the other `21` steps combined | `0.11` | `0.9%` |

    Every import projection, alias resolution, trait-definition pass, function-signature pass,
    receiver-catalog build, trait-evidence validation and public-surface build in this stage is
    together under one percent of it.

    One level down, with the nesting made explicit
    (`unresolved_choice_variants_for_header` calls `unresolved_member_syntax_to_declarations` for
    record payloads, so the choice rows contain the payload rows):

    | | ms | calls |
    |---|---|---|
    | `member_shells:Allow:StructField` | `3.09` | `320` |
    | `member_shells:Strict:StructField` | `2.36` | `320` |
    | `choice_shells:Allow` (contains the next row) | `1.33` | `80` |
    | `member_shells:Allow:ChoicePayload` | `1.32` | `240` |
    | `choice_shells:Strict` (contains the next row) | `1.16` | `80` |
    | `member_shells:Strict:ChoicePayload` | `1.15` | `240` |
    | `resolve_constructor_shells_for_constants` | `1.27` | `1` |
    | `resolve_struct_field_types` | `0.64` | `320` |
    | `resolve_choice_variant_payload_types` | `0.18` | `160` |
    | `resolve_constant_headers` | `0.05` | `1` |
    | `build_generic_parameter_scope` | `0.015` | `800` |

    **Member-shell construction, counting both passes, is `7.94ms` of `12.70ms` - `62.5%` of the
    stage.** The type resolution those shells exist to feed is `0.82ms`, `6.5%`. The scaffolding
    costs `9.7x` what the work costs. This is exactly the double construction Phase E was written
    against, and it is the dominant cost of the stage.

11. **A third of the shell cost is one scope object, rebuilt per header per pass.**
    `constant_header_scope_context` is `2.82ms` over `1120` calls - `2.5us` each, `22%` of the
    whole stage - and it is called once per `unresolved_member_syntax_to_declarations` entry, so
    twice per header. Its own breakdown:

    | | ms | share of the `2.82ms` |
    |---|---|---|
    | `ScopeContext::new` | `1.19` | `42%` |
    | the `17`-call `with_*` builder chain | `1.13` | `40%` |
    | `header.canonical_source_file` | `0.37` | `13%` |

    The builder chain is `~59ns` per `with_*` call, which is a large-struct move rather than the
    `Rc::clone` each method nominally performs: `ScopeContext` carries two `FxHashSet`s, three
    `Vec`s and a dozen handles inline, and every `with_*` takes it by value and returns it. `17`
    moves per construction, `1120` constructions.

    This gives Phase E a second target its work items did not name, and one that is independent of
    the shell restructure: even a single retained shell per member still pays this once per header
    unless the scope object is either built once per header and reused, or stops being moved
    through a by-value builder chain. Both are worth pricing separately before either is chosen -
    and note that the two passes are not trivially sharing one context, because the tables the
    scope clones (`resolved_struct_fields_by_path`, `choice_variant_shells_by_path`) are rewritten
    between them.

## Phase E first slice, and the real-project attribution that followed it

### The slice that landed - commit `19340ca29`

`ScopeContext::new` built a whole synthetic `AstModuleLookups` on every call: roughly thirty heap
allocations, including a fresh `StyleDirectiveRegistry::built_ins()` with its eight owned directive
names, plus a full empty module scaffold of maps, symbol tables and trait environments. It is
called once per header per environment pass, and every field it seeded into `ScopeShared` was
overwritten by the `with_*` chain the caller ran on the next line. No caller read it: the four
environment-time construction sites attach the real tables through the setters, and body emission
replaces the package wholesale with `with_lookups`. The only fields anything reads before either
happens are the two trait environments, and those were empty in the per-call version too.

One shared empty package now serves every scope. Two things fell out of doing it:

- The scaffold cannot hold the real declaration table. `declaration_table_mut` takes it through
  `Rc::get_mut`, so a retained clone would fail every environment-time declaration write. This is
  the constraint that makes the change work rather than a detail: it forced the placeholder to be
  genuinely empty.
- Three scope lookups were reading the declaration table through `shared.lookups.declaration_table`
  rather than through `shared.top_level_declarations`, which `ScopeShared` already carried and
  which held the same handle at every call site. Two names for one table, now one. This was found
  by the test suite, not by reading: the first grep for readers used a single-line pattern and
  missed every multi-line field chain.

`frontend.ast.environment` on `nominal_scaling_320`: `12.32ms` to `9.37ms`, `1.31x`. Median of ten,
interleaved against a worktree build of `3b811906a`, `--profile profiling`, `RAYON_NUM_THREADS=1`.
`frontend.bind_headers` and `frontend.order_declarations` build no scope contexts and moved by under
`0.5%` across the same runs. That is the only control available for a change to a universal
constructor - there is no fixture that does not execute one.

12. **A speedup measured on a binary that fails its tests is not a speedup.** The first measurement
    of this change read `12.25ms` to `4.55ms`. It was wrong: the binary was mid-change and `53`
    unit tests were failing with `CapacityNotConstant`, so the fixture was erroring out early and
    skipping most of the stage. The real figure appeared only after the suite went green. The rule
    this earns sits beside finding 9's: **validate before believing, in that order** - a large
    unexplained win is evidence of a bug before it is evidence of a win, and the size of the
    surprise is what should trigger the check.

### The attribution that matters, on a real project

The fixture number is not the number to plan against. The same change is `95.51ms` to `93.92ms` on
`docs`, `1.02x`. That gap is the finding, not a footnote, so `ast.environment` was attributed again
on `docs` - `73` modules, `545` constant headers - with the same per-pass probe discipline
(`34` marks, total overhead `0.25%` of the stage, measured).

13. **On a real project this stage is template construction, and almost nothing else.** Every row
    is a median-free single run; the shares are stable across repeats and the top row is confirmed
    by the pre-existing `constant_header_resolution` detailed timer at `83.89ms`.

    | | ms | share of the `94ms` stage |
    |---|---|---|
    | `resolve_nominal_members_and_constants` | `84.16` | `89.5%` |
    | ↳ `resolve_declaration_syntax` (`545` calls) | `71.27` | `75.8%` |
    | ↳ the initializer expression parse | `69.47` | `73.9%` |
    | ↳ `parse_template_expression` (`331` calls) | `69.13` | `73.5%` |
    | ↳ **`Template::new_const_required_with_type_interner`** | **`62.99`** | **`67.0%`** |
    | ↳ `prepare_tir_view` in `Value` mode (`325`) | `3.13` | `3.3%` |
    | ↳ `fold_prepared_template` (`284`) | `2.89` | `3.1%` |
    | the constant-header scope build (`545` calls) | `5.25` | `5.6%` |
    | every import projection combined | `6.65` | `7.1%` |
    | `register_nominal_shells` | `0.095` | `0.1%` |

    `190us` per template constant, in TIR construction. The re-preparation in `Value` mode that
    `parse_template_expression` documents as deliberate duplication is `3.3%`, so the documented
    double-prepare is not the cost - that was the hypothesis on the way in, and measuring retired
    it.

14. **Phase E's premise is true on its fixture and false on a real project.** Member-shell
    construction is `62.5%` of `ast.environment` on `nominal_scaling_320` and `0.1%` of it on
    `docs`. The fixture is not wrong - it isolates exactly what it says it isolates - but it is
    `100%` fixed-capacity struct fields driven by four constants, which is the densest possible
    nominal module and nobody's real code. Finding 10 should be read as scoped to nominal-dense
    modules from here on.

    This is not an argument for deleting the fixture or the phase. It is an argument that
    **a scaling fixture measures the cost it isolates, and only a real project can say what share
    of the stage that cost is.** Every attribution in this plan from here on gets both.

15. **The dominant cost of `ast.environment` is out of this plan's scope.** Template TIR
    construction is template-plan work; this plan's remaining phases - E's member shells, F's type
    environment and generic keys, H's declaration lanes - address none of it. That is worth stating
    plainly rather than letting the phase order imply otherwise. The honest summary of this stage
    on a large project is: `67%` template TIR, `7%` import projection, `6%` constant-header scope
    construction, `2%` environment assembly, and everything this plan has left under `2%` each.

    Findings 13-15 are recorded in
    `docs/roadmap/plans/post-tir-template-parser-optimization-plan.md` under **Evidence carried in
    from the constant-folding plan**, so its Phase 0 does not have to rediscover them. That plan
    keeps ownership and stays queued; this one does not take the work.

### Consolidation slice - one environment-time scope construction path

Taken because the attribution said the remaining performance in this stage is small, and because
four hand-written copies of one scope chain is a simplification worth making whatever it measures.
Net `-101` lines.

16. **There were two ways to install file visibility on a scope, and one of them cloned the whole
    package three times.** `function_signatures.rs` built its visibility field by field -
    `with_visible_external_symbols`, `with_visible_source_bindings`, `with_visible_type_aliases` -
    where every other pass installed the header-built `FileVisibility` as one shared handle. Each
    of those three setters ran `update_file_visibility`, which clones the entire `FileVisibility`
    and allocates a fresh `Arc`, on top of the three deep map clones at the call site. Six clones
    per function header to reproduce a package the caller already held behind an `Arc`.

    It also produced a *different* package: the piecemeal path starts from
    `FileVisibility::default()`, so `visible_namespace_records` and `visible_trait_names` were
    silently empty in every function-signature scope. Switching to `with_file_visibility` fixes
    that and the suite is unchanged, which says nothing in the corpus depended on those two fields
    being empty there. The three setters and their helper are deleted; `with_visible_declarations`
    stays, because emission and tests use it for a gate with no package behind it.

17. **The four scope chains were identical down to setter order, and the base is now written
    once.** `AstModuleEnvironmentBuilder::environment_header_scope` carries the module services and
    the four side tables that member shells, trait requirement signatures and function signatures
    all read. What the passes genuinely differ on stays at the call site, so a divergence has to be
    written down rather than hiding inside a copied chain:

    | pass | file visibility | resolved constants | choice shells |
    |---|---|---|---|
    | nominal member shells | yes | yes | yes |
    | function signatures | yes | yes | no |
    | trait requirement signatures | **no** | **no** | **no** |

    `constant_resolution.rs` keeps its own builder. Its session owns a different handle set and
    holding those handles across a pass would force copy-on-write on the very tables the
    environment rewrites between passes.

18. **The trait-requirement scope divergence is recorded, not repaired.** Trait requirement
    signatures parse in a scope with no file visibility at all, which means unrestricted
    declaration lookup on one hand and no alias, external-symbol or namespace maps on the other.
    Giving it the same visibility as every other pass passes the whole suite - it was tried - but
    it changes resolution in both directions: it would gate declaration lookup that is currently
    open, and make alias and namespace resolution work where it currently does not. Both could move
    the set of accepted programs, and this plan's phases are required to preserve that set. It
    belongs in a slice with its own tests, not inside a consolidation.

    **Performance: none, and none expected.** Interleaved A/B against `f1d17cc9a`, median of ten:
    `frontend.ast.environment` is `94.24ms` to `94.23ms` on `docs` and `9.34ms` to `9.36ms` on
    `nominal_scaling_320`. The clone saving is per function header and neither fixture is
    function-dense enough to show it. Recorded as a simplification, and it should not be cited as
    an optimisation.

19. **Two more copied preambles, collapsed the same way.** The environment passes all open with
    the same two fetches before doing any work of their own.

    | | copies before | after |
    |---|---|---|
    | the file-visibility fetch and its diagnostic conversion | `14` | `1` |
    | the eight-field `GenericParameterScopeBuildInput` block | `6` | `1` |

    `header_visibility` and `generic_parameter_scope` / `generic_parameter_scope_for_header` live on
    the builder, because the five fields the six generic-scope call sites always agreed on - the
    three visibility maps, the declaration table and the generic metadata - are the builder's, and
    the sites differed only in which parameter list and canonical map they passed. The whole
    consolidation is net `-145` lines across the two slices with no behaviour change and no
    measured time change, which is the honest description of it.

20. **The one-shell-per-member restructure was assessed against the same bar and not taken.**
    The bar for continuing Phase E was: take it if it simplifies, drop it if it does not. It does
    not. The phase's own design records a targeted capacity fixup table keyed by affected member
    and constant declaration, applied in declaration order after the constants commit - that is
    more machinery than the rebuild it replaces, not less, and it buys `0.1%` of a real project's
    environment stage.

    The redundancy is real and worth stating so it is not rediscovered: the member tables are
    written three times per struct - the `AllowUnresolvedCapacity` shell, the constructor-resolved
    shell, then the final resolved fields - and the middle write is discarded wholesale by the
    rebuild. But each write has a live consumer at the moment it is made, so there is no deletion
    available without the fixup machinery. Revisit only if a nominal-dense real project appears, or
    if the fixup table turns out to be needed for another reason.

21. **The trait-requirement scope divergence is closed: it was masked, not observable.** Finding 18
    left the divergence recorded and untouched because giving it visibility "changes resolution in
    both directions". Measuring what actually happens retires that concern.

    `resolve_trait_requirement` fetches the header's file visibility, calls
    `unresolved_trait_requirement_signature` in a scope that does not have it, then builds a
    *second*, visibility-correct `TypeResolutionContext` and calls `resolve_function_signature` on
    the result. That second call overwrites `diagnostic_type` **and** `type_id` for every parameter
    and every return slot. The permissive pass's type answers have no consumer.

    Proved rather than argued. With a temporary probe on the unresolved pass, a trait requirement
    naming a same-module type its file never imported reports
    `PROBE: unresolved_trait_requirement_signature OK` and is then rejected `MOTH-RULE-0035
    Unknown type name` by the strict pass. The unrestricted lookup is real and it is unreachable.

    Installing the caller's visibility on that scope is therefore a subset change: with it, the
    permissive pass sees exactly what the strict pass sees minus the generic parameter scope and
    trait environment, and `SignatureTypeFallbackPolicy::StrictCapacity` already absorbs the
    generic-parameter misses that produces. Evidence it is behaviour-neutral:

    | check | before | after |
    |---|---|---|
    | integration suite | `1873/1873` | `1873/1873` |
    | resolvable namespaced type in a requirement (`shapes.Label`) | accepted | accepted |
    | missing namespaced type (`shapes.Missing`) | `MOTH-RULE-0035` at `4:22`, `Unknown type name 'Missing'.` | byte-identical |

    Taken, with the visibility threaded from the caller that had already fetched it rather than
    fetched twice. Every environment-time scope now carries file visibility; the four-line "recorded
    rather than quietly repaired" comment is gone.

    **Three regression cases were added, because this contract had none.**
    `trait_requirement_unimported_type_rejected`, `trait_requirement_imported_type_success` and
    `trait_requirement_alias_type_success`. The first is the guard: nothing in the suite previously
    pinned that a requirement signature cannot name a type its file did not import.

22. **The two-step signature resolution is duplicated work, and it is not removable as written.**
    The same shape is in `resolve_function_signatures`, per function header, which is where the
    volume is: build an "unresolved" signature that fully resolves every annotation, then
    re-resolve every annotation. `return_slot_from_syntax` is the clearest case - it computes a
    `TypeId` and immediately discards it (`type_id: None`), and its `data_type` is re-resolved by
    the next step.

    It is not deletable, and the reason is worth recording so it is not re-proposed. The permissive
    pass's `type_id` has exactly two live consumers: the `type_id == STRING` test that attaches
    `ReactiveTemplateMetadata` to String parameters, and `parse_signature_default_expression`,
    which needs a resolved parameter type to check a default against. The strict pass recomputes
    neither. Feeding it `parsed_ref_to_data_type` instead is also not a drop-in: that conversion is
    lossy by design, mapping `ParsedTypeRef::This` to `DataType::Inferred`.

    So the removal is conditional on whether a member has a default and on how its annotation
    spells `String` - more machinery than the duplication, by the same bar that retired the
    one-shell-per-member restructure (finding 20). Recorded, not taken. Note also that this is not
    a measured hot path: function signatures did not appear in the `ast.environment` attribution at
    all, which places them under `2%` (finding 13).

## Phase E - Nominal member shells and capacity fixups

Goal: keep one immutable parsed member shell per struct field and choice payload, and build canonical
member definitions once.

Why now: `resolve_type_declarations` builds member shells before constants and rebuilds them after,
so the same field and variant structure is reconstructed twice per nominal. The `nominal_members`
scaling series isolates this from constant count at four sizes.

**Re-measure before implementing - done, see **Phase E attribution**.** Phase A found that `>99%`
of the cost of the pass this phase targets was the side-table snapshot, which Phase B removes. What
remains after Phase B has now been measured on its own: member-shell construction across both
passes is `7.94ms` of a `12.70ms` `ast.environment` on `nominal_scaling_320` (`62.5%`), against
`0.82ms` (`6.5%`) for the type resolution those shells feed. The double construction is not cheap,
so this phase keeps its performance framing - but the claim it is allowed to carry is a stage-time
one, measured the mandated way, not a scaling-exponent one.

> **Scoped by finding 14.** Everything below is measured on `nominal_scaling_320`, which is
> `100%` fixed-capacity struct fields. On `docs` this phase's target is `0.1%` of the stage. Read
> the phase as nominal-dense-module work and as deletion; it must not carry a large-project
> performance claim. The scope decision is open in NEXT_ACTION.

**A target the work items below do not name.** `constant_header_scope_context` is `22%` of the
stage by itself, built once per header per shell pass (finding 11). Its `ScopeContext::new` half is
already gone with `19340ca29`; what is left is the `with_*` chain and the fact that there are four
near-identical copies of that chain across the environment passes. Roughly `40%` of that is a
`17`-call `with_*` builder chain moving a large `ScopeContext` by value. Retaining one shell per
member halves the call count but does not address the per-construction cost, and the two passes
cannot trivially share one context because the tables it clones are rewritten between them. Treat
this as a separate item with its own before/after, not as something the shell restructure absorbs.

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
- [ ] Price the per-header scope-context construction separately (finding 11): either build it once
      per header and reuse it across passes, or stop moving `ScopeContext` through a by-value
      `with_*` chain. Record a before/after for whichever is chosen, and revert it if it does not
      move the stage.

Acceptance:

- [ ] `just bench-scaling` still reports `nominal_members` within budget, and its absolute times
      improve
- [ ] no member shell is constructed twice
- [ ] default-value and recursive-type diagnostics keep their locations
- [ ] the stage-time claim is measured against a worktree build of the before-commit, interleaved,
      median of at least seven, with a control fixture that does not execute the changed code -
      Phase D's withdrawn wall-time result is the standing reason this is required

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

### Phase F outcome - measured on its own worst case, and dropped

Phase F offered two honest options and said the choice belongs to whoever reaches it. Option one
was taken: commit a fixture that actually exercises generic instantiation at scale, re-measure, and
proceed only against what that shows. What it shows is that Phase F is aimed at the wrong
component, and it names the right one.

**The fixture.** `benchmarks/generic-scaling/generic-scaling-{20,40,80,160}.moth`, a new
`generic_instantiation` scaling series. `n` distinct concrete types, each driven through the same
seven generic call sites, so every site presents a substitution mapping and a generated-function
identity nothing else in the module shares. One small driver per type, so **instantiation count
grows and function body size does not** - that separation is the point, see finding 25.

The language does most of the work of bounding this. `MOTH-SYNTAX-0015` forbids nested generic
applications outright, so `Cell of Cell of T` is not a program anyone can write and substitution
cannot recurse through stacked applications. Depth is reached with structural wrappers around a
single application instead - `{Cell of T}`, `{{Cell of T}}`, `Pair of A, B`.

**Release measurement**, `profiling` profile, `RAYON_NUM_THREADS=1`, median of seven, fitted over
the `8x` range:

| metric | `n=20` | `n=160` | fitted |
|---|---|---|---|
| `build.frontend.total` | `46.30ms` | `1500.16ms` | `n^1.67` |
| **`frontend.generated.materialise`** | **`41.20ms`** | **`1403.99ms`** | **`n^1.70`** |
| `frontend.ast.total` | `5.53ms` | `27.77ms` | `n^0.78` |
| `frontend.borrow.initial` | `2.04ms` | `12.48ms` | `n^0.87` |
| `frontend.hir` | `1.76ms` | `10.44ms` | `n^0.86` |
| **`frontend.ast.environment`** | **`0.574ms`** | **`1.609ms`** | **`n^0.50`** |

At the largest point `frontend.generated.materialise` is `94.1%` of the frontend and
`frontend.ast.environment` - the whole of Phase F's target - is `0.11%` and sublinear. The step
ratios rise across the series (`2.85x`, `3.21x`, `3.55x` on the total), so the curve is still
steepening at `n=160` rather than settling.

**Phase F is therefore closed without its conversions.** Every dense-storage item and the
substitution-key canonicalisation target a component that is sublinear and immaterial on a fixture
purpose-built to be their worst case. The phase's own constraint - a less readable table that is not
hot is a regression - decides it. Nothing in Phase F is taken.

23. **Generated-function materialisation rebuilds the module's string table once per
    instantiation.** Two independent measures agree on the same exponent, which is what makes this
    a mechanism rather than a timing.

    | `n` | `string_table_full_clones` | `string_table_merge_source_entries_scanned` | entries per clone |
    |---|---|---|---|
    | 20 | `201` | `40,476` | `201` |
    | 40 | `401` | `120,896` | `302` |
    | 80 | `801` | `401,736` | `502` |
    | 160 | `1,601` | `1,443,416` | `902` |

    Clones are exactly `10n + 1` - linear, one per generated function. Entries scanned per clone is
    `~5n + 101` - **linear in module size**. The product is quadratic; measured `n^1.72`, which is
    the same curve as `frontend.generated.materialise` at `n^1.70`.

    The mechanism is two lines. `GenericTemplateArtefact::materialise_ast` opens with
    `StringTable::new()` and `merge_from(&requester_context.string_table)`;
    `MaterialisationPreparation::materialise_ast` opens with `self.string_table.clone()` and the
    same `merge_from`. `merge_from` re-interns every string in the source table and allocates a
    remap `Vec` of the same length, so each instantiation pays for the whole module's strings.

    **The cheaper mechanism already exists and is already used elsewhere.** `merge_delta_from`
    merges only the strings added after a fork's inherited prefix, and `fork_source` /
    `fork_for_module` exist so that prefix is copied once per batch - its own doc comment says so.
    Module compilation uses them: on this fixture `string_table_delta_merge_calls` is `3` and
    `string_table_fork_source_base_copies` is `3` at every size. Generated-function materialisation
    uses neither.

24. **The real-project share is `0.002%`, and that does not retire it.** Both numbers get recorded,
    per finding 15. Debug build throughout this table, taken in one pass so the shares compare to
    each other; the absolute ms do not compare to the release table above.

    | project | frontend total | `generated.materialise` | share |
    |---|---|---|---|
    | `docs` | `1605.8ms` | `0.03ms` | `0.002%` |
    | `module-graph` | `24.9ms` | `0.001ms` | `~0%` |
    | `type-stress` | `32.7ms` | `0.003ms` | `~0%` |
    | `generic-trait-churn` | `32.9ms` | `11.80ms` | **`35.9%`** |
    | `generic-scaling-160` | `4665.6ms` | `4265.3ms` | `91.4%` |

    `docs` barely uses generics, so it says nothing about this cost either way. The number that
    should decide priority is `generic-trait-churn`: `181` lines, a handful of instantiations,
    nothing adversarial about it, and already `35.9%` of its frontend. A project that uses generics
    at all pays this immediately - it does not wait for `n=160`.

25. **A single growing function body makes borrow validation quadratic, independently of
    generics.** The first version of this fixture grew the instantiation count and the size of one
    driver function together. `frontend.borrow.initial` fitted `n^2.28` on it and `n^1.14` on the
    split version at the same instantiation count, with `frontend.borrow.converge` at `95%` of it
    both times.

    So that `n^2.28` is body size, not generics, and the fixture was rebuilt to separate them. Two
    things to carry forward: any future generics fixture must hold body size fixed or it measures
    the wrong thing, and borrow convergence is superlinear in the size of a single function body -
    a separate finding, unmeasured on a real project, and not this plan's.

**Scope.** Finding 23 is a defect in generated-function materialisation, which is neither constant
folding nor the type environment, and acting on it is a new phase in a different subsystem rather
than a step in this one. The fixture, the series and the measurement are committed here so the cost
is visible and ratcheted; the fix is a scope decision for the next slice.

The `generic_instantiation` series budget is set to `n^1.80` against a measured `n^1.75`. That is a
ratchet around a known defect, not an accepted shape. Tighten it when finding 23 is fixed; do not
raise it.

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

## Phase I - Slice review and closeout

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
- TIR store, exact views and preparation
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
- a new template fold cache - the previous one was deleted in Phase D after measuring a
  `1/11275` hit rate across every committed template workload
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
