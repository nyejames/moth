# Constant Folding and Type-System Hot-Path Optimisation Plan

> **Repository path:**
> `docs/roadmap/plans/constant-folding-and-type-system-hot-path-optimization-plan.md`
>
> **Implementation branch:**
> `agent/constant-folding-type-system-optimization-plan`
>
> **Status:**
> Ready for implementation after the command timing correction plan is accepted and this branch is
> rebased onto it.
>
> **Planning snapshot:**
> `main` at `c77dfa0f3f5decd98ce64682d65f8977973cfb06`.

## Purpose

Reduce the dominant constant and type-resolution costs in the AST frontend by removing repeated
context construction, copied semantic state, rich intermediate clones and redundant value
representations.

The final design must make the common path data-oriented without replacing Moth's existing semantic
owners:

- Stage 3 remains the declaration-order authority
- `TopLevelDeclarationTable` remains the indexed declaration owner
- `TypeEnvironment` remains the canonical module-local type owner
- `TypeId` remains the semantic type identity
- TIR exact views and the TIR fold cache remain the template authority
- `PublicFoldedValue` remains the owned cross-module folded-value vocabulary
- HIR receives final folded values and runtime operations, never TIR or unresolved type syntax

The work is an implementation optimisation. It must not change language semantics, accepted programs,
diagnostic priority, deterministic ordering, public identities or emitted artefacts.

---

## Active context capsule

ACTIVE_PLAN:
- `docs/roadmap/plans/constant-folding-and-type-system-hot-path-optimization-plan.md`

CURRENT_SLICE:
- Phase: prerequisite gate
- Goal: rebase onto the accepted timing-schema v2 implementation and establish a reliable baseline
- Non-goals: no optimisation change before the corrected metrics are available

LAST_GOOD_COMMIT:
- `c77dfa0f3f5decd98ce64682d65f8977973cfb06`

PREREQUISITE:
- `docs/roadmap/plans/command-timing-accounting-and-reporting-correction-plan.md`
- implementation must provide module-attributed timings for constant resolution, const-template
  parse/fold and module-constant finalisation before Phase 0 records its baseline

RELEVANT_DOCS:
- `AGENTS.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
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
- `src/compiler_frontend/ast/module_ast/scope_context/`
- `src/compiler_frontend/ast/type_resolution/`
- `src/compiler_frontend/ast/type_interner.rs`
- `src/compiler_frontend/ast/const_eval/`
- `src/compiler_frontend/ast/const_values/`
- `src/compiler_frontend/ast/expressions/eval_expression/`
- `src/compiler_frontend/ast/expressions/expression_rpn.rs`
- `src/compiler_frontend/ast/module_ast/finalization/normalize_constants.rs`
- `src/compiler_frontend/datatypes/environment.rs`
- `src/compiler_frontend/datatypes/definitions.rs`
- `src/compiler_frontend/type_coercion/compatibility.rs`
- `src/compiler_frontend/folded_value.rs`
- `src/compiler_frontend/hir/hir_statement/declarations.rs`
- `src/compiler_frontend/instrumentation/`
- `benchmarks/manifest.toml`

NEXT_ACTION:
- rebase after timing correction, then execute Phase 0 without changing semantics

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

---

## Architectural invariants

- Tokenization and declaration-shell parsing happen once.
- Stage 3 dependency order is authoritative. AST must not add a constant fixpoint or rediscover
  declaration dependencies.
- Every ordinary constant and const template folds once in its defining module.
- A provider exports an owned folded value. Consumers never parse or fold provider source again.
- `TypeId` drives semantic decisions. `DataType` is parse or diagnostic data after resolution.
- Donor-local `TypeId`, declaration IDs, value IDs and store IDs never cross module interfaces.
- Constant folding preserves checked numeric failures, cast rules, finite-Float rules, template
  preparation rules and synthetic-interface provenance.
- Type and fold errors retain current source locations, diagnostic families and priority.
- TIR stays AST-local and is dropped before the completed AST leaves Stage 4.
- HIR does not become a second constant folder or type resolver.
- Capacity estimates and caches affect performance only. A miss or underestimate cannot change
  correctness.
- Parallelism, reuse and caching preserve deterministic identities and diagnostic order.

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

### 7. Type resolution uses explicit views

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

### 8. Improve `TypeEnvironment` in place

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

### 9. Canonical generic bindings are built once

Give each immutable concrete generic binding set one canonical ordered pair representation and, if
useful, a module-local `GenericBindingsId`. Substitution cache keys use that stable representation
instead of collecting and sorting an `FxHashMap` into a new boxed slice for each recursive lookup.

Reuse the current substitution cache and generic instance interning. Remove only repeated key
construction and cloned member views.

### 10. Nominal members keep one syntax shell

Keep one immutable parsed member shell per struct field or choice payload. Early nominal registration
creates identity and generic metadata only.

Constructor readiness writes resolved type slots or explicit pending fixups into a side table. Fixed
capacity expressions that depend on constants record a targeted fixup keyed by the affected member
and constant declaration. After constants resolve, complete only pending slots and write canonical
field/variant definitions once.

Do not rebuild complete `Declaration` and `ChoiceVariant` trees before and after constants. Defaults
must similarly retain one syntax owner and one final folded value.

### 11. The source/token layout plan owns token ranges

The existing compiler source/token/diagnostic data-layout plan owns compact source identities, token
stores and retained token ranges. This plan must not introduce competing `SpanId`, token-range or
source-store designs.

Before that plan lands, optimise ownership and movement around current `FileTokens` and
`SourceLocation`. After it lands, switch constant/member syntax to its canonical ranges without
changing this plan's semantic stores.

---

## Non-goals

- no new language type or coercion rule
- no general compile-time virtual machine
- no arbitrary user-function execution during constant folding
- no parallel constant evaluation before dependency and diagnostic ordering are proven safe
- no rewrite of TIR, Stage 3 or `TypeEnvironment` into competing frameworks
- no compiler-wide unsafe packing
- no full AST arena without measured evidence
- no token/source layout work already owned by the data-layout plan
- no cross-module sharing of donor-local IDs or mutable stores
- no best-effort fallback that reparses source or rebuilds semantic facts

---

## Phase 0 - Correct baseline and scaling fixtures

Prerequisite: rebase onto the accepted timing-schema v2 implementation.

- [ ] Record five independent focused frontend and end-to-end runs using the existing benchmark
      protocol.
- [ ] Capture module-attributed constant, const-template and finalisation timings.
- [ ] Run `docs`, `constant_dag_churn`, `fold_stress`, `expression_rpn_churn`, `type_stress`,
      `environment_stress` and `one_module_kitchen_sink`.
- [ ] Add committed clean benchmark workloads with the same tiny initializer repeated across at
      least 32, 128 and 512 dependency-ordered constants. Generate them deterministically if hand
      maintenance would be noisy.
- [ ] Add a capacity-dependent nominal fixture that separates constant count from member count.
- [ ] Record counters for:
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
- [ ] Use `RAYON_NUM_THREADS=1` for local frontend attribution, then repeat the normal thread identity
      to ensure no scheduling regression.
- [ ] Store concise evidence in `benchmarks/frontend-optimization-results.md`.

Checkpoint: evidence only. No semantic representation change.

## Phase 1 - Consolidate constant-resolution context

- [ ] Introduce the one module-owned `ConstantResolutionSession`.
- [ ] Borrow binding visibility and environment side tables instead of cloning them into `Rc`s.
- [ ] Reuse one `TypeCompatibilityCache` for the pass.
- [ ] Reuse the existing TIR store, warning sink and rendered-path sink.
- [ ] Refactor shared declaration/expression parser resources so top-level constants do not require
      synthetic `AstModuleLookups`.
- [ ] Delete the constant-header `ScopeContext` builder chain after the final caller migrates.
- [ ] Keep body-local constant parsing on normal lexical `ScopeContext`.
- [ ] Prove diagnostics, warning order and folded results are byte-for-byte equivalent.

Expected deletion targets:

- per-constant `ScopeContext::new`
- per-constant synthetic empty lookup maps and registries
- module-wide side-table clones created only for constant parsing
- per-constant `TypeCompatibilityCache`

Checkpoint: context consolidation with no value representation change.

## Phase 2 - Dense declaration and resolved-constant state

- [ ] Make Stage 3 final order allocate or carry stable `DeclarationId`s.
- [ ] Add direct ID operations to `TopLevelDeclarationTable`.
- [ ] Build declaration-kind lanes once from final order.
- [ ] Add `ResolvedConstantSet` with capacity equal to declaration count.
- [ ] Resolve explicit module-constant visibility through the bitset.
- [ ] Replace path-based declaration replacement with ID replacement inside ordered semantic passes.
- [ ] Remove cumulative prior-constant insertion into temporary scope frames.
- [ ] Remove linear scans of `module_constants` for explicit constant identity.
- [ ] Keep source name/path maps only at lookup and diagnostic boundaries.

Scaling acceptance:

- [ ] Setup work for the 32/128/512 constant fixtures grows approximately linearly.
- [ ] The `previous-constant IDs copied` counter reaches zero for module constants.
- [ ] No new full declaration scan appears per constant.

Checkpoint: dense IDs and state, still using current folded expression payloads.

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

Checkpoint: one folded-value authority with all old conversion paths removed.

## Phase 4 - Move-only typed postfix and folding

### 4A: ownership cleanup

- [ ] Make expression ordering reserve from the known input count.
- [ ] Make `constant_fold` consume its item vector.
- [ ] Move non-foldable operands/operators back into the runtime result.
- [ ] Return the sole folded operand by move.
- [ ] Add focused tests proving no source or synthetic provenance is lost.
- [ ] Drive full-expression clone counters to zero in ordinary arithmetic fold paths.

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

### 4C: evidence-gated operand handles

- [ ] Profile remaining runtime postfix copies.
- [ ] Introduce compact operand handles only if they materially reduce time or retained memory.
- [ ] If implemented, keep the arena module-local and move it into final AST ownership.
- [ ] Delete `#[allow(clippy::large_enum_variant)]` only when the representation genuinely no longer
      needs it.

Checkpoint each subphase independently. Do not combine the safe move-only change with an unmeasured
arena migration.

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

Checkpoint: pass/index cleanup after the main constant and type wins are already measurable.

## Phase 10 - Final audit and closeout

Run focused validation throughout. At final closeout run at minimum:

```bash
cargo test --lib compiler_frontend::ast::const_eval
cargo test --lib compiler_frontend::ast::const_values
cargo test --lib compiler_frontend::ast::type_resolution
cargo test --lib compiler_frontend::datatypes
cargo test --lib compiler_frontend::hir
just bench-frontend-check
RAYON_NUM_THREADS=1 just bench-frontend-check
just bench-check
just validate
```

At each performance checkpoint:

- [ ] run five independent benchmark invocations and compare medians
- [ ] use the same timing schema, source fingerprint, measurement fingerprint and thread identity
- [ ] run the relevant scaling fixture
- [ ] record counter and timing movement in `benchmarks/frontend-optimization-results.md`
- [ ] treat an unexplained median regression above 5% as a blocker
- [ ] require semantic/output equivalence before accepting a speed improvement

Final acceptance:

- [ ] constant setup scales linearly with constant count
- [ ] no per-constant synthetic complete lookup context remains
- [ ] no cumulative previous-constant copying remains
- [ ] each module constant has one folded-value authority
- [ ] public projection and HIR consume that authority without reparsing or deep intermediate clones
- [ ] shunting yard remains the one precedence algorithm
- [ ] rich RPN typing and fold clones are removed
- [ ] successful type resolution is `TypeId`-first and diagnostic spelling is lazy
- [ ] dense TypeEnvironment changes are evidence-backed and private
- [ ] nominal members retain one syntax shell and build canonical members once
- [ ] old redundant APIs and representations are deleted without compatibility wrappers
- [ ] all diagnostics, public identities and emitted artefacts remain stable

Do not update the progress matrix unless current language support changes. Update compiler design only
if implementation reveals an accepted ownership boundary not already stated there.

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
- TIR store, exact views, preparation and fold cache
- `PublicFoldedValue` for owned cross-module projection
- existing HIR module-constant and const-fact consumers, migrated to IDs/store access
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

### Avoid

- a second constant parser
- a second type environment
- a generic compiler-pass framework
- a new template fold cache
- source-token or span designs parallel to the data-layout plan
- speculative caches with incomplete semantic keys
- unsafe packing before ordinary ownership and algorithmic waste is removed

## Completion contract

The plan is complete only when the code has fewer semantic representations and fewer construction
paths than it started with. A faster implementation that leaves the old constant trees, context
builders or conversion walkers alive as compatibility layers does not satisfy this plan.
