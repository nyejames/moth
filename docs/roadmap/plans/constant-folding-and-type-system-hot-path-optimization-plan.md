# Constant Evaluation, Static Control-Flow Specialisation and Type-System Architecture Plan

> **Repository path:**
> `docs/roadmap/plans/constant-folding-and-type-system-hot-path-optimization-plan.md`
>
> **Implementation branch:**
> `agent/constant-evaluation-static-if-type-system-plan`
>
> **Status:**
> Ready for implementation. The command timing accounting and reporting correction plan this entry
> once waited on was deleted from the roadmap on 2026-08-18, so the sequencing gate is satisfied by
> absence. The timing requirement it carried survives as this plan's own prerequisite below.
>
> **Planning snapshot:**
> `main` at `7a3649d2e35668d11b55746835ac1cb2a7c1bb07`.

## Purpose

Build one compact, durable constant-evaluation and type-resolution architecture, remove the dominant
constant and type-system hot-path costs and add Stage 4 static specialisation of ordinary `if`
statements whose conditions are known compile-time `Bool` values.

This plan has two coupled outcomes:

1. Replace repeated context construction, copied semantic state, rich intermediate clones and
   redundant value representations with data-oriented module-owned stores and indexed views.
2. Make compile-time evaluation reduce ordinary Bool control flow before HIR so later compiler
   systems process only the executable branch selected for the configured build.

The final design must improve the common path without replacing Moth's existing semantic owners:

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

Most phases are implementation optimisations and must preserve accepted programs, diagnostics,
public identities and emitted artefacts. Phase 4C is the one deliberate semantic expansion: both
branches remain frontend-valid source, but a compile-time-known Bool condition selects one branch
before HIR, borrow validation, lifetime analysis, link facts, target validation and backend lowering.

---

## Active context capsule

ACTIVE_PLAN:
- `docs/roadmap/plans/constant-folding-and-type-system-hot-path-optimization-plan.md`

CURRENT_SLICE:
- Phase: 2 (dense declaration and resolved-constant state)
- Goal: carry `DeclarationId` into AST environment work and replace path-based declaration
  replacement with ID replacement inside ordered semantic passes
- Non-goals: no semantic control-flow change before the explicit Phase 4C gate

LAST_GOOD_COMMIT:
- Phase 1 commit on `const-folding-and-types-optimisation`

PREREQUISITE:
- Satisfied. Timing schema `2` already carries the module-attributed
  `frontend.ast.environment.constant_header_resolution`,
  `frontend.ast.emit.const_template_parse`, `frontend.ast.emit.const_template_fold` and
  `frontend.ast.finalise.module_constant` metrics, so no rebase was needed.

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
- `src/compiler_frontend/module_dependencies.rs`
- `src/compiler_frontend/headers/types.rs`
- `src/compiler_frontend/headers/binding_environment/`
- `src/compiler_frontend/ast/module_ast/environment/`
- `src/compiler_frontend/ast/module_ast/finalization/`
- `src/compiler_frontend/ast/module_ast/scope_context/`
- `src/compiler_frontend/ast/statements/branching.rs`
- `src/compiler_frontend/ast/statements/terminality.rs`
- `src/compiler_frontend/ast/statements/value_production/`
- `src/compiler_frontend/ast/type_resolution/`
- `src/compiler_frontend/ast/type_interner.rs`
- `src/compiler_frontend/ast/const_eval/`
- `src/compiler_frontend/ast/const_values/`
- `src/compiler_frontend/ast/expressions/eval_expression/`
- `src/compiler_frontend/ast/expressions/expression_rpn.rs`
- `src/compiler_frontend/ast/generic_functions/`
- `src/compiler_frontend/datatypes/environment.rs`
- `src/compiler_frontend/datatypes/definitions.rs`
- `src/compiler_frontend/type_coercion/compatibility.rs`
- `src/compiler_frontend/folded_value.rs`
- `src/compiler_frontend/hir/hir_statement/declarations.rs`
- `src/compiler_frontend/hir/tests/hir_branch_lowering_tests.rs`
- `src/compiler_frontend/instrumentation/`
- `benchmarks/manifest.toml`

NEXT_ACTION:
- execute Phase 2 and preserve current semantics until the mandatory Phase 4C review gate

---

## Phase 0 outcome and implementation notes

Phase 0 completed on 2026-08-22. Evidence lives in
`benchmarks/frontend-optimization-results.md` under
`Constant Evaluation And Type-System Plan - Phase 0 Baseline - 2026-08-22`. The facts below change
how later phases must be implemented, so they are recorded here rather than only in the evidence
file.

### Measured baseline highlights

- Constant-header resolution is about half of `frontend.ast.total` on `docs` (`566ms` of `1144ms`),
  `fold_stress` (`17.0ms` of `36.6ms`) and `constant_dag_churn` (`9.7ms` of `17.1ms`).
- `ast_constant_pass_prior_constant_ids_copied` is exactly `C * (C - 1) / 2`: `496` / `8128` /
  `130816` for the 32 / 128 / 512 constant chains. `ast_constant_pass_visibility_entries_cloned`
  is `2208` / `33408` / `526848` for the same chains.
- `moth check` on the chains costs roughly `7ms` / `40ms` / `350ms` above a `~28ms` floor, so
  constant setup is clearly superlinear today.
- `ast_expression_operand_clones` equals `ast_expression_fold_items` on the chains: every folded
  operand is currently a full `Expression` clone.

### Findings that constrain later phases

1. **A constant-backed Bool condition is not a folded `Bool` at HIR.** `if enabled:` with
   `enabled #= true` reaches HIR as a reference expression; only a literal `if true:` folds to
   `ExpressionKind::Bool`. The JavaScript backend resolves the module constant later, which is why
   the emitted code reads `if (true)`. The Phase 4C specialisation owner must read the condition
   through the folded-value authority (module constant row / `ConstValueStore`), not by matching
   `ExpressionKind::Bool`. The `hir_static_bool_if_nodes` counter deliberately measures the literal
   case only, so it is the post-4C invariant counter, not a candidate census.
2. **Reuse the existing generic-request pruning boundary.** `ScopeContext::generic_request_checkpoint`
   and `ScopeContext::discard_generic_requests_since` already exist and are used by static
   `assert(true)` message discarding in `src/compiler_frontend/ast/statements/asserts.rs`. Phase 4C
   must reuse that mechanism for inactive branches rather than adding a second boundary.
   `src/compiler_frontend/ast/statements/branching.rs` already brackets its branch bodies with a
   checkpoint under `benchmark_counters` for the `ast_branch_local_generic_requests` counter.
3. **Inactive generic work is materialised today.** A generic call reachable only through a
   compile-time-false branch still emits a generated function into the artefact. Phase 4C's
   acceptance must assert its absence.
4. **`HirBuilder::lower_if_with_body_emitters` in
   `src/compiler_frontend/hir/hir_statement/control_flow.rs` is the single HIR `if`-diamond owner**
   for both statement `if` and runtime template `if`. It carries `record_hir_branch_condition_kind`,
   which is where the "no statically decided `if` reaches HIR" assertion belongs.
5. **Member shells are rebuilt after constants** in
   `AstModuleEnvironmentBuilder::resolve_type_declarations`
   (`src/compiler_frontend/ast/module_ast/environment/type_resolution.rs`), which is the Phase 8
   target. `benchmarks/nominal-capacity-stress.moth` isolates that cost from constant count.
6. **`resolve_constant_headers` clones five module side tables once per module**, not per constant,
   so `ast_constant_pass_side_table_entries_cloned` reflects table size. The genuine per-constant
   cost was the `Rc::new(file_visibility.clone())` in the constant-header parser, measured by
   `ast_constant_pass_visibility_entries_cloned`. Phase 1 moved that copy to once per source file.

### Assets Phase 0 created

- Benchmark workloads `constant_chain_32`, `constant_chain_128`, `constant_chain_512` and
  `nominal_capacity_stress`, each with a `_check` and a `_frontend` case. The manifest inventory
  test in `xtask/src/benchmark_manifest/tests.rs` and the counts in `benchmarks/README.md` were
  updated in the same change.
- New `AstCounter` variants: `ConstantResolutionContextsCreated`, `ConstantsResolved`,
  `ConstantPassPriorConstantIdsCopied` (deleted in Phase 1 with the copy it measured),
  `ConstantPassVisibilityEntriesCloned`,
  `ConstantPassSideTableEntriesCloned`, `ModuleConstantDeclarationClones`,
  `ExpressionOrderingInputItems`, `ExpressionTypedStackItems`, `ExpressionFoldItems`,
  `ExpressionOperandClones`, `DiagnosticDataTypeMaterialisations`, `BranchLocalGenericRequests`.
  `DeclarationTableReplacements` was renamed `DeclarationReplacementsByPath`; the by-ID counterpart
  belongs to Phase 2, which introduces the by-ID replacement path.
- New `FrontendCounter` variants: `GenericSubstitutionKeyBuilds`,
  `GenericSubstitutionKeySortedPairs`, `PublicFoldedValueConversions`, `HirConstValueConversions`,
  `HirStaticBoolIfNodes`, `HirRuntimeIfNodes`.
- Integration cases `static_if_constant_bool_branch_selection`,
  `static_if_value_producing_branch_selection`, `static_if_branch_scope_preserved` and
  `static_if_inactive_branch_generic_call`. `function_partial_if_return_rejected` already froze the
  current terminality rejection and `dynamic_if_test` already owns runtime-condition execution, so
  neither was duplicated.
- Ignored intended-contract tests in `src/compiler_frontend/tests/frontend_pipeline_tests.rs`:
  `intended_compile_time_true_condition_reaches_hir_without_a_branch`,
  `intended_compile_time_false_condition_without_else_lowers_no_branch_body` and
  `intended_terminality_observes_the_selected_branch`, plus the non-ignored freeze
  `runtime_bool_condition_lowers_one_branch_diamond`. Phase 4C enables the ignored three.

### Measurement protocol note

Recorded runs (`just bench`, `just bench-frontend`) require a clean committed worktree and rewrite
the tracked monthly summary, so five consecutive recorded invocations need
`git checkout -- benchmarks/summaries/` between them. Fixed-thread recorded runs
(`RAYON_NUM_THREADS=1`) never touch the tracked summary. Per-case medians come from
`benchmarks/local-data/runs.jsonl`, which read-only `bench-check` modes do not write.

---

## Accepted static control-flow contract

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
same ordinary folded-constant and static-if path defined here:

```moth
analytics #Config of Bool = false

if analytics:
    send_analytics()
;
```

This plan does not implement `#Config`, CLI build-input parsing or project-global interfaces. It
establishes the general static-`if` behaviour that the queued build-configuration-values plan will
consume without a config-specific AST node, branch pass or HIR operation.

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
- an unknown/runtime condition remains an ordinary `if`

The selected branch retains its authored lexical scope. Specialisation must not hoist branch-local
bindings into the parent scope or flatten control-flow ownership merely because the runtime test is
removed.

The specialised active AST becomes the authority for:

- function terminality
- durable generated-function requests
- executable effect, access and project-context summaries
- HIR construction
- borrow validation
- lifetime-region and escape analysis
- per-function link facts and reachability
- target-contract validation
- backend lowering and emitted runtime code

The inactive branch contributes none of those downstream products. HIR must never receive an `if`
whose condition is already a known compile-time Bool.

### Stable source structure

Compile-time values may specialise executable behaviour. They do not change source structure.

`#Config` and ordinary compile-time Bool conditions must not control:

- dependency clauses or provider graph edges
- package resolution
- source discovery or semantic source sets
- declaration existence
- exported declaration existence
- receiver-method existence
- trait or conformance existence
- module or package facade topology

This keeps Stage 0 graph construction, Stage 3 declaration order, declaration identity and module
interfaces structurally stable across configured builds. Public semantic summaries and executable
fingerprints may still change when the active body has different effects, calls or lifetime facts.

### Platform-agnostic source boundary

Moth source does not inspect compilation targets, operating systems, architectures, backend choice or
runtime-platform identity. Platform integration belongs to project builders, builder packages,
external packages and backend capability surfaces.

The static-`if` system is for typed application and build configuration, not target-dependent source.
Builders must not recreate target conditional compilation by supplying synthetic values such as
`target_os`, `target_arch`, `is_wasm`, `is_javascript` or equivalent backend identity flags through
the future `#Config` surface.

A change to this boundary requires an explicit Moth design-philosophy review. It is not a deferred
extension of this plan.

---

## Evidence at the planning snapshot

A representative release documentation build reported:

- AST environment: about `64.32ms`
- constant-header resolution: about `53.7ms` after excluding the duplicate outer timer
- const-template parsing: about `21.1ms`
- const-template folding: about `3.2ms`
- module-constant normalisation: about `8.0ms`

The legacy detailed channel prints synchronously and duplicates the constant timer, so these numbers
are directional rather than a valid benchmark baseline. They are enough to prioritise constant setup
and representation work. Phase 0 must replace them with schema-v2 measurements.

The current implementation also shows the static-control-flow gap that Phase 4C must close:

- AST branching parses both bodies and always creates `NodeKind::If` for an ordinary Bool condition
- HIR branch tests currently lower even literal `true` and `false` conditions into runtime CFG
  diamonds
- function terminality intentionally does not evaluate conditions beyond structurally folded
  `assert(false)`
- generic requests and other durable side products need an ownership audit so a pruned branch cannot
  keep downstream work alive

The current code also exposes concrete structural costs independent of timing noise:

| Current pattern | Cost or complexity |
|---|---|
| Fresh `ScopeContext` and synthetic `AstModuleLookups` per top-level constant | Allocates and populates body-oriented machinery for a declaration-order constant pass |
| Copy all previously resolved constant IDs into each new constant scope | Cumulative `O(C²)` insertion work for `C` constants |
| Clone aliases, generic metadata, nominal members, nominal IDs and trait environment for the constant pass | Rebuilds immutable views already owned by the environment builder |
| Clone `FileVisibility` and its visible declaration set for short-lived contexts | Repeats binding-owned state instead of borrowing it |
| Store each constant as a cloned declaration in both declaration table and `module_constants` | Duplicates a rich expression tree and path metadata |
| Recursively normalise module constants into new `Expression` trees, then recursively convert them again for HIR | Three value representations for one already-folded fact |
| Build infix items, shunting-yard RPN, a type stack and a folding stack over full `Expression` operands | Multiple linear passes and full-expression clones around an efficient ordering algorithm |
| Carry and clone diagnostic `DataType` through hot type stacks | Builds cold display data on successful paths where `TypeId` is sufficient |
| Rebuild nominal member declaration shells before and after constants | Reparses or reconstructs the same field and variant structure to resolve capacity dependencies |
| Scan `sorted_headers` repeatedly for each declaration kind | Repeated broad branching and poor locality as the language grows |
| Clone fields, variants or signatures before read-only generic-bound validation | Copies semantic trees to satisfy local borrow structure |
| Build and sort boxed generic substitution mappings per cache key | Repeats canonicalisation that can be owned once by the binding set |
| Lower compile-time-known Bool conditions into ordinary HIR branches | Sends inactive executable work through every later compiler system and backend |
| Validate terminality before static branch selection | Rejects functions whose specialised active body is provably terminal |

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
- Type and fold errors retain current source locations, diagnostic families and priority. Phase 4C
  intentionally removes only downstream diagnostics belonging exclusively to inactive executable
  work.
- TIR stays AST-local and is dropped before the completed AST leaves Stage 4.
- Platform and backend identity do not enter source-level constant conditions through compiler- or
  builder-provided target flags.
- Capacity estimates and caches affect performance only. A miss or underestimate cannot change
  correctness.
- Parallelism, reuse and caching preserve deterministic identities, diagnostics and output order.

---

## Locked end-state decisions

### 1. One module constant-resolution session

Replace the per-constant body-oriented context construction with one
`ConstantResolutionSession` owned by `AstModuleEnvironmentBuilder` for the complete dependency-ordered
constant pass.

The session borrows:

- the declaration table
- binding-owned file visibility
- resolved aliases and generic declaration metadata
- nominal type IDs and member shells
- trait metadata
- external package and project-path services
- the module TIR store
- warning and rendered-path sinks

It owns or mutably borrows:

- the module `TypeEnvironment`
- one `TypeCompatibilityCache`
- resolved-constant state
- the module-local folded-value store

The exact name may change. The ownership may not. Do not create one `Rc` graph per constant and do not
build a synthetic complete `AstModuleLookups` package before the real environment exists.

The shared declaration and expression parsers remain the syntax owners. Split the resources they
consume into narrow lookup and mutation views rather than creating a second constant parser.

### 2. Dense declaration identity through Stage 3 and AST

Carry `DeclarationId` from final Stage 3 order into AST environment work. Add direct ID-based
`get`, `get_mut` and `replace` operations to `TopLevelDeclarationTable` and use path/name indexes only
for source lookup.

Build compact ordered lanes once:

```text
all declarations in dependency order
aliases
nominals
constants
traits and conformances
functions
```

Each lane stores declaration/header IDs, not cloned headers. Kind-specific passes iterate their lane
while dependency-sensitive passes retain the complete order.

Builtins and generated construction appends must allocate IDs through the table owner so indexes and
lanes cannot drift.

### 3. Resolved constants use a bitset, not copied path sets

Add a dense `ResolvedConstantSet` keyed by `DeclarationId`. A constant becomes visible to later
constants when its value is committed in dependency order.

This set replaces:

- copying every prior constant ID into each new scope frame
- scanning `module_constants` to decide whether a declaration is an explicit compile-time constant
- path hashing for resolution-state checks when a declaration ID is already known

Body-local `#` constants remain scope-frame facts. Do not conflate module declaration order with
body-local lexical scope.

### 4. One module-local folded-value store

Introduce a Vec-backed `ConstValueStore` with compact `ConstValueId` handles for module-local folded
values. Start with a straightforward maintainable representation and use ranges or side arenas for
variable-length collections, records and choice payloads. Do not begin with unsafe packing or a
compiler-wide AST arena.

A stored value retains the facts required by current consumers:

- canonical local `TypeId`
- folded scalar or aggregate payload
- synthetic-interface provenance
- source location or diagnostic anchor where required
- const-record classification where applicable

Strings use module-local `StringId`. Aggregate children use `ConstValueId`. Public projection copies
an exported value once into `PublicFoldedValue`, converting local type/value identities to canonical
owned interface facts.

The completed AST moves the store and module-constant rows into HIR ownership. HIR references the
same folded values or consumes them move-only into its final constant pool. It must not reconstruct
an intermediate normalised AST tree first.

### 5. Module constants are indexed rows, not duplicate declarations

Store module constants as compact rows such as:

```rust
struct ModuleConstant {
    declaration: DeclarationId,
    value: ConstValueId,
}
```

The declaration table remains the name, type and visibility authority. The constant row/store remain
the folded-value authority. Do not retain a second `Vec<Declaration>` containing cloned expressions.

A reference to a module constant resolves to its declaration and `ConstValueId`. Runtime lowering can
load the constant by ID without first materialising a deep `Expression` clone.

### 6. Keep shunting yard, replace the rich work around it

The existing shunting-yard algorithm is correct and remains the precedence owner.

First make the current path move-only:

- consume input vectors
- reserve output/operator capacity from known item count
- move operands and operators through folding
- return a fully folded operand without cloning it
- keep diagnostic spelling out of the successful type stack

Then combine ordering and operator typing into one typed-postfix builder. Resolve an operator's
`TypeId` result when it leaves the operator stack and emit compact typed postfix data. Complete type
validation before evaluating foldable operations so current type-error-before-fold-error priority is
preserved.

The fold evaluator consumes typed postfix once and returns either:

- `ConstValueId` for a fully folded result
- a reduced runtime expression/postfix payload for surviving runtime work

A full `ExprId` arena is evidence-gated. Implement it only if Phase 4 counters or profiles show that
move-only full `Expression` operands remain material. Do not introduce an arena to satisfy an
architectural preference alone.

### 7. Static Bool control flow is specialised once in Stage 4

Add one AST-finalisation owner for ordinary `if` specialisation after constant values and expression
types are final, but before terminality and durable executable summaries are committed.

The owner consumes the existing folded-value authority. It must not:

- run a second evaluator
- inspect source tokens again
- identify `#Config` declarations specially
- add a config-specific AST or HIR node
- rebuild branch scopes or reparse branch bodies

For statement `if`:

- known `true` selects the authored `then` body in its existing lexical scope
- known `false` selects the authored `else` body in its existing lexical scope
- known `false` without `else` produces no executable statements while preserving valid source and
  diagnostic identity
- runtime/unknown conditions retain the ordinary `NodeKind::If`

For value-producing `if`:

- both branches first satisfy the normal receiving arity and type rules
- a known condition selects the corresponding already-validated value block
- the selected value enters normal expression/value lowering without a runtime branch or hidden
  merge local

Do not generalise this phase into match reduction, loop unrolling, cross-function constant
propagation or a compile-time virtual machine.

Durable generic requests and executable metadata must be published from, or filtered against, the
specialised active AST. Type checking may validate a generic call in an inactive branch, but that
branch must not cause concrete materialisation or generated sidecar work.

Terminality runs after specialisation. A function may therefore become provably terminal when its
only active configured branch returns, while a runtime condition retains the existing conservative
all-path rule.

### 8. Type resolution uses explicit views

Split the broad optional `TypeResolutionContextInputs` shape into explicit data views:

- immutable declaration and visibility lookup
- mutable derived-type interning
- optional generic scope
- optional trait/evidence overlay
- optional constant-value lookup

Provide named constructors for the real context classes, such as module declaration, constant,
body and generated materialisation. Invalid combinations should be unrepresentable or rejected at
construction, not represented by many unrelated `Option` fields.

Successful resolution returns or carries `TypeId`. Diagnostic spelling is produced lazily at the
error/render boundary. Do not cache rendered names as semantic facts.

### 9. Improve `TypeEnvironment` in place

Do not replace `TypeEnvironment`. It already owns dense `TypeId` storage and canonical interning.
Optimise its hot tables in place:

- seed capacities from existing `FrontendArenaCapacityEstimate` and header statistics
- replace reverse maps keyed by dense local IDs with vectors where absence can be represented
  explicitly
- keep forward structural/path interning maps as hash maps
- store generic parameter type IDs and bounds in dense ID-indexed vectors
- store `NominalTypeId -> TypeId` as a dense vector
- store `TypeId -> canonical identity` as an ID-indexed optional vector if profiling confirms its
  current reverse hash map is material
- represent cached generic instance fields/variants as arena ranges or boxed slices with borrowed
  query views rather than independent growable vectors

Every conversion requires size, lookup and benchmark evidence. Do not make a less readable table
more compact when it is not hot.

### 10. Canonical generic bindings are built once

Give each immutable concrete generic binding set one canonical ordered pair representation and, if
useful, a module-local `GenericBindingsId`. Substitution cache keys use that stable representation
instead of collecting and sorting an `FxHashMap` into a new boxed slice for each recursive lookup.

Reuse the current substitution cache and generic instance interning. Remove only repeated key
construction and cloned member views.

### 11. Nominal members keep one syntax shell

Keep one immutable parsed member shell per struct field or choice payload. Early nominal registration
creates identity and generic metadata only.

Constructor readiness writes resolved type slots or explicit pending fixups into a side table. Fixed
capacity expressions that depend on constants record a targeted fixup keyed by the affected member
and constant declaration. After constants resolve, complete only pending slots and write canonical
field/variant definitions once.

Do not rebuild complete `Declaration` and `ChoiceVariant` trees before and after constants. Defaults
must similarly retain one syntax owner and one final folded value.

### 12. The source/token layout plan owns token ranges

The existing compiler source/token/diagnostic data-layout plan owns compact source identities, token
stores and retained token ranges. This plan must not introduce competing `SpanId`, token-range or
source-store designs.

Before that plan lands, optimise ownership and movement around current `FileTokens` and
`SourceLocation`. After it lands, switch constant/member syntax to its canonical ranges without
changing this plan's semantic stores.

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

---

## Phase 0 - Correct baseline, semantic freeze and scaling fixtures

Prerequisite: rebase onto the accepted timing-schema v2 implementation.

- [x] Record five independent focused frontend and end-to-end runs using the existing benchmark
      protocol.
- [x] Capture module-attributed constant, const-template and finalisation timings.
- [x] Run `docs`, `constant_dag_churn`, `fold_stress`, `expression_rpn_churn`, `type_stress`,
      `environment_stress` and `one_module_kitchen_sink`.
- [x] Add committed clean benchmark workloads with the same tiny initializer repeated across at
      least 32, 128 and 512 dependency-ordered constants. Generate them deterministically if hand
      maintenance would be noisy.
- [x] Add a capacity-dependent nominal fixture that separates constant count from member count.
- [x] Add or freeze focused static-control-flow cases for:
  - literal `true` and `false` statement `if`
  - constant-backed Bool conditions
  - `if` with and without `else`
  - value-producing `if`
  - scope preservation after branch selection
  - terminality changed by a known condition
  - a generic call owned only by the future inactive branch
  - borrow, lifetime, link and target work owned only by the future inactive branch
  - a runtime Bool condition that must continue to lower as CFG
- [x] Where the intended Phase 4C behaviour differs from the current compiler, add clearly named
      ignored intended-contract tests rather than weakening current assertions early.
- [x] Record counters for:
  - constants resolved
  - constant sessions and `ScopeContext`s created
  - previous-constant IDs copied
  - visibility/map entries cloned
  - compatibility-cache lookups/hits
  - declaration replacements by path and by ID
  - infix, typed-postfix and fold item counts
  - full `Expression` and `DataType` clones/materialisations
  - module-constant normalisation nodes visited
  - public and HIR folded-value conversions
  - generic substitution key builds and sorted pairs
  - static-Bool `if` candidates
  - runtime `if` nodes reaching HIR
  - generated requests attributed to branch-local call sites
- [x] Use `RAYON_NUM_THREADS=1` for local frontend attribution, then repeat the normal thread identity
      to ensure no scheduling regression.
- [x] Store concise evidence in `benchmarks/frontend-optimization-results.md`.

Checkpoint: evidence and intended-contract tests only. No semantic representation or control-flow
change.

## Phase 1 - Consolidate constant-resolution context

- [x] Introduce the one module-owned `ConstantResolutionSession`.
- [x] Borrow binding visibility and environment side tables instead of cloning them into `Rc`s.
- [x] Reuse one `TypeCompatibilityCache` for the pass.
- [x] Reuse the existing TIR store, warning sink and rendered-path sink.
- [ ] Refactor shared declaration/expression parser resources so top-level constants do not require
      synthetic `AstModuleLookups`.
- [ ] Delete the constant-header `ScopeContext` builder chain after the final caller migrates.
- [x] Keep body-local constant parsing on normal lexical `ScopeContext`.
- [x] Prove diagnostics, warning order and folded results are byte-for-byte equivalent.

Expected deletion targets:

- per-constant `ScopeContext::new`
- per-constant synthetic empty lookup maps and registries
- module-wide side-table clones created only for constant parsing
- per-constant `TypeCompatibilityCache`

Checkpoint: context consolidation with no value representation or control-flow change.

### Phase 1 outcome and implementation notes

Phase 1 completed on 2026-08-22. Evidence lives in
`benchmarks/frontend-optimization-results.md` under
`Constant Evaluation And Type-System Plan - Phase 1 Consolidated Constant Session - 2026-08-22`.

`ConstantResolutionSession` in
`src/compiler_frontend/ast/module_ast/environment/constant_resolution.rs` now owns the module view
the whole constant pass reads: the five side tables, the trait environment, project services, the
TIR store, the rendered-path sink, one `TypeCompatibilityCache`, and one prepared
`FileVisibility` package per source file. `resolve_constant_headers` drives it and commits each
folded constant. `ConstantHeaderParseContext` and `parse_constant_header_declaration` are deleted.

Two supporting ownership changes carry most of the measured win:

- `ScopeContext::visible_declaration_ids` and
  `ScopeFrame::explicit_compile_time_constant_declarations` are now shared copy-on-write handles.
  Every child scope in the compiler shared these sets by clone before; only a scope that actually
  declares a local now pays for a private copy.
- `AstModuleEnvironmentBuilder` owns one `resolved_module_constant_paths` set, updated by
  `push_module_constant`. Constant-header, nominal-member and function-signature scopes all take a
  handle to it instead of copying every prior constant path.

Measured result: `-46%` on `constant_chain_512`, `-27%` on `fold_stress`, `-23%` on
`constant_dag_churn`, flat on `docs` and `nominal_capacity_stress`.
`ast_constant_pass_visibility_entries_cloned` falls from `526848` to `1029` on the `512` chain.

Two checkboxes stay open, deliberately:

1. **The synthetic `AstModuleLookups` per constant remains.** `ScopeContext::new` builds it, and it
   holds the declaration table, so it cannot be prepared once for the pass. See finding 2 below.
2. **The constant-header `ScopeContext` builder chain still has callers.**
   `AstModuleEnvironmentBuilder::constant_header_scope_context` (member shells) and the
   function-signature pass both build `ContextKind::ConstantHeader` scopes from live side tables
   that are still being mutated when they run, so neither can share the constants session. Phases 8
   and 9 own those passes.

Findings that constrain later phases:

1. **`ast_constant_pass_prior_constant_ids_copied` was deleted, not zeroed.** The cumulative copy
   it measured no longer exists. The Phase 2 scaling acceptance item naming that counter is
   satisfied structurally; do not reintroduce the counter to check the box.
2. **The declaration table's `Rc::get_mut` commit path is the real constraint on session shape.**
   `AstModuleEnvironmentBuilder::replace_declaration` requires sole `Rc` ownership, so no scope may
   hold the table across a constant commit. That is why the session prepares everything except the
   table and still builds one `ScopeContext` per constant. Phase 2's ID-based replacement work
   should decide deliberately whether the table gains a commit path that tolerates live readers; if
   it does, the session collapses to one root scope with per-constant child frames and the
   remaining per-constant `ScopeContext::new` disappears with it.
3. **`docs` is not a constant-setup workload.** It stayed flat despite a `29%` drop in visibility
   copying, because its constants are one or two per file and its AST cost is const-template
   parsing and folding. The Phase 0 reading that constant-header resolution is about half of
   `frontend.ast.total` on `docs` is real, but that half is fold work owned by Phases 3 and 4, not
   context construction.
4. **`constant_chain_512` is still superlinear after the session.** `68.69ms` for `512` trivial
   constants is well above `4x` the `128` case. The remaining candidates are path-based declaration
   replacement, the per-constant module-constant declaration clone and constant normalisation,
   which Phases 2 and 3 own.

## Phase 2 - Dense declaration and resolved-constant state

- [ ] Make Stage 3 final order allocate or carry stable `DeclarationId`s.
- [ ] Add direct ID operations to `TopLevelDeclarationTable`.
- [ ] Build declaration-kind lanes once from final order.
- [ ] Add `ResolvedConstantSet` with capacity equal to declaration count.
- [ ] Resolve explicit module-constant visibility through the bitset.
- [ ] Replace path-based declaration replacement with ID replacement inside ordered semantic passes.
- [x] Remove cumulative prior-constant insertion into temporary scope frames. Phase 1 replaced it
      with one shared `resolved_module_constant_paths` handle.
- [ ] Remove linear scans of `module_constants` for explicit constant identity.
- [ ] Keep source name/path maps only at lookup and diagnostic boundaries.

Scaling acceptance:

- [ ] Setup work for the 32/128/512 constant fixtures grows approximately linearly.
- [x] The `previous-constant IDs copied` counter reaches zero for module constants. Phase 1
      deleted both the copy and the counter.
- [ ] No new full declaration scan appears per constant.

Checkpoint: dense IDs and state, still using current folded expression payloads and control flow.

## Phase 3 - Module-local folded-value authority

### Store and rows

- [ ] Add `ConstValueId`, `ConstValueStore` and compact module-constant rows.
- [ ] Define scalar, collection, record, choice, range, option/fallible and string/template-folded
      payloads required by current language support.
- [ ] Preserve type, provenance, const-record and location facts.
- [ ] Make declaration lookup return constant identity without cloning its value tree.
- [ ] Change module-constant references and field access to read the store.

### Consumers

- [ ] Project exported constants directly from the store into `PublicFoldedValue` once.
- [ ] Move the store and rows through the AST-to-HIR boundary.
- [ ] Make HIR module constants reference or consume the same store.
- [ ] Update config and direct `.mtf` compilation extraction to read the shared store.
- [ ] Keep advisory body-local/inferred `AstConstFacts` separate from authored module-constant
      storage, but reuse `ConstValueId` where a value is retained.

### Delete old representations

- [ ] Replace `module_constants: Vec<Declaration>` in environment/lookups/AST contracts.
- [ ] Delete `declaration.clone()` solely for table plus module-vector ownership.
- [ ] Delete recursive `normalize_module_constant_expression` once the store receives final values
      directly.
- [ ] Delete the AST-expression-to-HIR-constant recursive conversion after HIR consumes the store.
- [ ] Consolidate public and HIR conversion walkers around one borrowed store visitor where their
      output vocabularies differ.

Checkpoint: one folded-value authority with all old conversion paths removed. Control-flow behaviour
remains unchanged until Phase 4C.

## Phase 4 - Typed constant evaluation and static control-flow specialisation

### 4A: ownership cleanup

- [ ] Make expression ordering reserve from the known input count.
- [ ] Make `constant_fold` consume its item vector.
- [ ] Move non-foldable operands/operators back into the runtime result.
- [ ] Return the sole folded operand by move.
- [ ] Add focused tests proving no source or synthetic provenance is lost.
- [ ] Drive full-expression clone counters to zero in ordinary arithmetic fold paths.

Checkpoint: move-only ownership cleanup with byte-for-byte equivalent diagnostics and artefacts.

### 4B: typed-postfix builder

- [ ] Resolve operator input/result `TypeId`s as operators leave the shunting-yard stack.
- [ ] Emit a compact typed postfix item with only semantic IDs, flags and diagnostic anchors needed
      by the fold evaluator.
- [ ] Keep diagnostic `DataType` construction lazy.
- [ ] Validate the whole typed expression before executing fold operations so diagnostic priority
      remains unchanged.
- [ ] Delete the separate rich RPN result-type scan.
- [ ] Make the fold evaluator produce `ConstValueId` directly.
- [ ] Preserve reduced postfix only for runtime-dependent work.

Checkpoint: typed-postfix and folded-value authority with unchanged source semantics.

### 4C: static Bool `if` specialisation

This is the mandatory semantic review gate. Do not combine it with operand-arena work or another
performance refactor.

- [ ] Add one named Stage 4 specialisation owner after full branch frontend validation and final
      constant values, before terminality, durable generated work and HIR.
- [ ] Read a condition's known Bool from the existing typed folded-value authority. Add no second
      evaluator or config-specific lookup.
- [ ] Specialise statement `if` according to the accepted contract while preserving the selected
      branch's lexical scope, source locations and statement order.
- [ ] Specialise value-producing `if` only after both branches satisfy receiving arity and type
      rules.
- [ ] Leave runtime and unknown Bool conditions as ordinary `NodeKind::If` values.
- [ ] Run terminality over the specialised active AST.
- [ ] Ensure inactive branch calls do not publish generated-function requests or generated sidecar
      work.
- [ ] Ensure inactive branch code contributes no HIR, borrow facts, lifetime facts, executable
      effects, link facts, target requirements or backend output.
- [ ] Preserve syntax, name, visibility, type, generic-evidence, cast, const-evaluation and
      value-production diagnostics from both authored branches.
- [ ] Preserve stable declaration, type and function identities. Static specialisation changes
      executable bodies and derived summaries, not source declaration identity.
- [ ] Record branch-selection dependencies in implementation, effect/link and root fingerprints
      through the existing fingerprint owners. Do not add a parallel static-if fingerprint.
- [ ] Assert that HIR contains no branch terminator for a statically decided `if`.
- [ ] Assert that a runtime Bool condition still produces the established HIR branch/merge shape.
- [ ] Enable and complete the Phase 0 intended-contract tests.

Review gate acceptance:

- [ ] both authored branches remain frontend-valid source
- [ ] selected branch scope is unchanged
- [ ] terminality observes selected control flow
- [ ] inactive durable generic requests are absent
- [ ] all downstream compiler systems receive only active executable work
- [ ] source graph, declarations, exports and package topology are unchanged
- [ ] no platform/backend conditional mechanism has entered source

Checkpoint: accepted static-control-flow semantics and focused end-to-end validation. Artefact changes
are expected and must match the selected active branch exactly.

### 4D: evidence-gated operand handles

- [ ] Profile remaining runtime postfix copies after 4A through 4C.
- [ ] Introduce compact operand handles only if they materially reduce time or retained memory.
- [ ] If implemented, keep the arena module-local and move it into final AST ownership.
- [ ] Delete `#[allow(clippy::large_enum_variant)]` only when the representation genuinely no longer
      needs it.

Checkpoint each subphase independently. Do not combine a measured operand representation change with
the Phase 4C semantic gate.

## Phase 5 - Type-resolution context and lazy diagnostics

- [ ] Split immutable lookup, mutable type interning and optional semantic overlays into explicit
      views.
- [ ] Add named constructors for module declaration, constant, body and generated contexts.
- [ ] Remove impossible `Option` combinations and repeated wide initialisers.
- [ ] Return `TypeId`-first results from successful lookup paths.
- [ ] Construct diagnostic spelling only when producing a diagnostic or public display fact.
- [ ] Borrow resolved aliases, fields, variants and signatures instead of cloning them for read-only
      validation.
- [ ] Centralise visibility lookup by file once per header/declaration pass.
- [ ] Add a lookup cache only when counters prove repeated identical resolution under the same
      visibility and generic scope. Its key must include every semantic context dimension.

Deletion targets:

- repeated `TypeResolutionContextInputs` boilerplate
- context setters that exist only to assemble constant-header lookup state
- successful-path `DataType` clones used only in case of a later error
- clone-to-iterate generic-bound validation paths

Checkpoint: explicit contexts and lazy diagnostic data.

## Phase 6 - `TypeEnvironment` hot-table improvements

Apply one table change per checkpoint with layout/query tests and benchmark evidence.

- [ ] Add capacity-aware construction from existing frontend estimates.
- [ ] Convert `NominalTypeId -> TypeId` to dense storage.
- [ ] Convert generic parameter `ID -> TypeId` and `ID -> bounds` to dense storage.
- [ ] Evaluate `TypeId -> canonical identity` dense optional storage.
- [ ] Store generic instance field/variant views in compact immutable ranges or slices.
- [ ] Keep forward structural, path and canonical-identity interning maps hashed.
- [ ] Confirm every query returns borrowed data unless ownership is required at a stage boundary.
- [ ] Record memory and lookup effects. Revert conversions that add complexity without measurable
      benefit.

Do not expose physical table layout outside `TypeEnvironment` query methods.

## Phase 7 - Generic substitution key consolidation

- [ ] Canonicalise each concrete generic binding set once.
- [ ] Reuse its ordered pair slice or `GenericBindingsId` in recursive substitution.
- [ ] Change substitution-cache keys to avoid collecting, sorting and boxing the same mapping for
      each source `TypeId`.
- [ ] Keep cache scope module-local.
- [ ] Preserve deterministic ordering and current conflict diagnostics.
- [ ] Measure generic-trait and type-stress workloads before accepting the new key shape.

Checkpoint: substitution key only. Do not mix with generic semantics or materialisation changes.

## Phase 8 - Nominal member shell and capacity fixups

- [ ] Define the single retained field/variant member shell and its resolution slots.
- [ ] Register nominal identities and generic metadata without constructing unresolved declaration
      value trees.
- [ ] Resolve constructor-required member types into slots before constant evaluation.
- [ ] Record only constant-dependent capacity/default fixups.
- [ ] Apply fixups in declaration order after their constants commit.
- [ ] Build canonical `FieldDefinition` and `ChoiceVariantDefinition` arrays once.
- [ ] Move those arrays into `TypeEnvironment` and expose borrowed views.
- [ ] Delete the early and late reconstruction of member declarations/choice variants.
- [ ] Preserve default-value diagnostics and recursive-type validation locations.

Checkpoint separately for structs and choices if either surface becomes broad.

## Phase 9 - Declaration-lane and environment pass cleanup

- [ ] Migrate aliases, nominals, constants, traits and functions to their prebuilt ID lanes.
- [ ] Keep complete declaration order for passes whose semantics depend on it.
- [ ] Remove repeated `for header in sorted_headers { match kind ... }` scans that no longer own
      ordering.
- [ ] Consolidate environment final assembly so each side table moves once into its final owner.
- [ ] Replace clone-to-satisfy-borrow patterns with field splitting, temporary `mem::take` or narrow
      query methods where ownership remains clear.
- [ ] Keep orchestration readable. Do not hide the phase order in a generic pass framework.

Checkpoint: pass/index cleanup after the main constant, static-control-flow and type wins are already
measurable.

## Phase 10 - Final audit and closeout

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
- [ ] require semantic/output equivalence before accepting a speed improvement

At the Phase 4C semantic checkpoint:

- [ ] compare output to the accepted static-control-flow contract rather than the old artefact bytes
- [ ] record the intended HIR, generated-work, borrow/lifetime, link and target-validation deltas
- [ ] prove runtime conditions retain the existing semantics
- [ ] prove both authored branches retain frontend diagnostics
- [ ] run focused integration cases for active and inactive downstream failures

Final acceptance:

- [ ] constant setup scales linearly with constant count
- [ ] no per-constant synthetic complete lookup context remains
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
- [ ] Phase 4C changes executable artefacts and downstream diagnostics only where the inactive branch
      is deliberately absent

Phase 4C changes current language/compiler support. Its implementation closeout must update the
compiler architecture, progress matrix and user-facing constant/control-flow documentation. The
queued build-configuration-values plan must integrate `#Config of Bool` through the same ordinary constant and `if`
path without adding another specialisation owner.

---

## Simplification and reuse audit

### Reuse directly

- `TopLevelDeclarationTable` and its existing `DeclarationId`
- Stage 3 dependency order
- `FrontendArenaCapacityEstimate` and header/token statistics
- `TypeEnvironment` interning and substitution caches
- `TypeCompatibilityCache`
- binding-owned `FileVisibility`
- scope-frame arenas for body-local declarations only
- existing `NodeKind::If`, branch scopes and value-producing `if` validation
- existing AST finalisation boundary, extended with one named static-control-flow owner
- existing terminality validation, moved after specialisation
- TIR store, exact views, preparation and fold cache
- `PublicFoldedValue` for owned cross-module projection
- existing HIR module-constant and const-fact consumers, migrated to IDs/store access
- existing HIR branch lowering for runtime conditions only
- current benchmark manifest, profiles, counters and five-run protocol

### Remove after migration

- constant-header synthetic `AstModuleLookups`
- constant-header `ScopeContext` setter chains
- per-constant compatibility caches
- copied prior-module-constant path sets
- duplicate `Vec<Declaration>` module-constant ownership
- recursive normalised-expression reconstruction for module constants
- duplicate AST-to-public and AST-to-HIR value-tree interpretation
- rich RPN item cloning
- eager diagnostic `DataType` stacks
- repeated generic substitution map sorting/boxing
- duplicate nominal member shell reconstruction
- repeated broad header scans made obsolete by declaration lanes
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

## Completion contract

The plan is complete only when the compiler has fewer semantic representations, fewer construction
paths and one explicit Stage 4 boundary between fully validated source and specialised executable
control flow.

A faster implementation that leaves the old constant trees, context builders or conversion walkers
alive as compatibility layers does not satisfy the plan. A static-`if` implementation that skips
frontend validation, leaks platform policy into source, retains inactive downstream work or adds a
parallel config-specific branch path also does not satisfy it.
