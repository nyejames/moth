# Moth TIR Deep Audit, Corrections and Simplification Plan

## Current state

```text
WORK_ID: tir-corrections
WORK_SOURCE: docs/roadmap/plans/tir-corrections-and-simplification-plan.md
BASE_REVISION: c77dfa0f3f5decd98ce64682d65f8977973cfb06
STATUS: active
CURRENT_SCOPE: Phase 0 complete
COMPLETED: Phase 0 tests and missing TIR counters; no production refactors
NEXT_ACTION: begin Phase 1A style-guide comment policy
VALIDATION: just validate passed (ci-clippy native/linux/windows; cargo test workspace 4259 pass + 6 ignored; tests 1845/1845; docs check clean; bench-ci preflight 60/60; timers-erasure clean)
AUDITS: auditor pass 1 audit_clean for Phase 0
BLOCKERS: none
NOTES: wrapper-once already holds; copied sites, derived identity, cycle-as-CompilerError, and wrapper-slot injection reproduced and are ignored until Phase 2; user source with wrapper slots inside runtime control flow is rejected as MOTH-SYNTAX-0022
```

## Audit scope

This revision audits the completed TIR implementation on the current connected `nyejames/moth` default branch, including `src/compiler_frontend/ast/templates/tir/**`, template construction, slot planning, AST finalization, reactive-template metadata, the neutral AST-to-HIR handoff and the HIR template entry points. The review is governed by `docs/compiler-design-overview.md`, the canonical template references selected by `docs/src/docs/codebase/language/overview.mtf`, `docs/src/docs/codebase/style-guide/style-guide.mtf`, `docs/src/docs/codebase/style-guide/testing.mtf` and the profile-gated `docs/roadmap/plans/post-tir-template-parser-optimization-plan.md`. The architecture is fundamentally good: TIR is AST-local, exact views are established and HIR consumes neutral owned data. The remaining work is a substantial consolidation pass with several confirmed invariant defects, two probable user-visible slot/wrapper bugs, repeated semantic walks and migration-era scaffolding. This was a static source audit through the read-only GitHub connector, so findings marked **probable** must begin with a failing regression test before implementation.

The target end state remains:

```text
parser emission
-> one module-local TemplateIrStore
-> exact TirView transitions
-> one exhaustive preparation result
-> fold or neutral owned runtime handoff
-> HIR
```

No TIR store, ID, view, overlay or preparation type may cross into completed AST, HIR, borrow validation or a backend.

## Top findings

### 1. Derived template operations can lose exact identity

**Issue**

Several transforms rebuild a `TemplateIr` or `TemplateTirChildReference` from partial fields rather than deriving a new version from the complete source identity.

Confirmed examples:

- Nested slot expansion in `tir/slot_composition/schema.rs` rebuilds a child template with `TemplateIr::new`, drops `conditional_child_wrapper_set` and `runtime_slot_plan`, then resets the child reference to `Parsed` with an empty context.
- Other `TemplateIr::new` call sites manually copy a subset of fields and are vulnerable to the same drift.
- `subtree_copy.rs` refreshes `DynamicExpression` site IDs but preserves branch-selector and loop-header expression-site IDs even when the copy mode requests fresh identities.

**Why it matters**

The accepted design says exact view identity is `root + phase + context`, and every expression-bearing site is module-unique. Losing context or reusing site IDs can make overlays address the wrong expression, drop wrapper or slot behavior and cause a copied tree to alias the source tree's normalization state.

**Evidence / affected area**

- `tir/slot_composition/schema.rs`
  - `expand_tir_slot_placeholders_from_node`
- `tir/subtree_copy.rs`
  - `copy_tir_node_with_active_slot_plan`
- all production `TemplateIr::new` call sites
- all production `TemplateTirChildReference::new` call sites used during derivation

**Recommended fix**

Introduce checked derivation APIs and make them the only way to version existing TIR:

```rust
impl TemplateIr {
    fn derived_with_root(
        &self,
        root: TemplateIrNodeId,
        summary: TemplateIrSummary,
    ) -> Self;
}

impl TemplateTirChildReference {
    fn with_root(self, root: TemplateIrId) -> Self;
    fn with_context(self, context: TemplateViewContext) -> Self;
}
```

A root transform preserves style, semantic kind, side-table links, phase and context unless the named phase/context owner explicitly changes them. Replace the `preserve_expression_site_ids: bool` parameter with an explicit copy mode and one `ExpressionSiteRemap` that covers dynamic expressions, branch selectors and every loop-header site.

### 2. Wrapper and runtime-slot paths have probable correctness drift

**Issue**

Two paths appear inconsistent with their own structural contracts.

1. **Probable double wrapper application.** Control-flow render-unit preparation structurally applies inherited `$children(..)` wrappers to direct body children. Final template construction then attaches the same inherited wrapper set as a wrapper-context overlay across the final branch/loop tree.

2. **Probable missing runtime fill injection.** Runtime wrapper-site planning discovers slot targets inside branches and loops, but `build_tir_wrapper_render_pieces` only substitutes direct `Slot`, `Sequence` and `ChildTemplate` shapes. A branch or loop root with a target slot is copied whole with the slot still unresolved.

**Why it matters**

The first can emit nested wrappers twice. The second can silently drop runtime fill content inside a control-flow wrapper. Both are user-visible semantic failures around exactly the slot and structural-no-output behavior TIR was introduced to preserve.

**Evidence / affected area**

- `tir/render_unit.rs`
  - `apply_inherited_child_wrappers_to_body_root`
- `tir/slot_composition/child_wrappers.rs`
  - `wrap_tir_node_in_wrappers`
- `create_template_node.rs`
  - `prepare_control_flow_render_units`
  - `attach_wrapper_context_overlay`
- `template_slots/runtime_plan/sites.rs`
  - `try_build_child_wrapper_site_pieces_from_tir_id`
  - `build_tir_wrapper_render_pieces`

**Recommended fix**

Add end-to-end regressions before changing code.

- Choose wrapper-context overlays as the single direct-child wrapper owner if the double-application test fails. Delete the control-flow-only structural wrapper path rather than adding an "already wrapped" flag.
- Replace the partial runtime wrapper-piece builder with one recursive, slot-injection-aware TIR copy transform that handles `BranchChain`, `Loop`, nested child templates and sequences consistently. Prefer reusing the same injection semantics as owned handoff materialization rather than maintaining another partial interpretation.

### 3. Final semantic classification still has multiple owners

**Issue**

`preparation.rs`, `classification.rs` and runtime control-flow validation independently walk effective TIR to answer overlapping questions:

- const evaluability
- unresolved slots
- resolved slot sources
- escaped insert helpers
- runtime reasons
- wrapper foldability
- malformed authority

`classification.rs` contains raw and view-aware const walkers, slot walkers and insert walkers. Preparation then repeats much of the same structure. Some runtime planning calls a boolean constness API that converts malformed TIR into ordinary `false`.

**Why it matters**

This violates the design rule that each semantic fact has one source owner. It increases compile work and creates inconsistent failure behavior. A broken store must produce `CompilerError`, not look like valid runtime dependence.

**Evidence / affected area**

- `tir/preparation.rs`
- `tir/classification.rs`
- `template_control_flow/validation.rs`
- `template_slots/runtime_plan/sources.rs`
- `ast/const_values/resolver.rs`
- `create_template_node.rs`

**Recommended fix**

Make `prepare_tir_view` return one complete result:

```rust
struct TemplatePreparation {
    identity: TirViewIdentity,
    facts: TemplatePreparationFacts,
    outcome: TemplatePreparationOutcome,
}

enum TemplatePreparationOutcome {
    Foldable,
    Runtime(RuntimeTemplateReason),
    Helper(TemplateHelperKind),
}
```

`TemplatePreparationFacts` owns all final kind-refresh, slot and insert facts. Folding and handoff consume the exact preparation proof. Move only expression-kind constness rules that are also needed before a complete view into a narrow `expression_constness.rs` owner. Delete the broad final classifier and fold runtime control-flow artifact validation into preparation.

Cycles through exact view identities must be `CompilerError`. Do not classify a cycle as ordinary runtime work because the owned handoff is a recursive tree and cannot represent a cyclic graph safely.

### 4. Slot composition repeatedly rediscovers the same facts

**Issue**

One head-chain layer can currently:

1. collect wrapper schema to detect a receiver
2. route fill content
3. collect schema again to choose runtime or structural handling
4. structurally expand slots
5. retain only wrapper/fill IDs
6. reroute the same fill during overlay allocation
7. collect placeholders again for overlay entries or runtime planning

Schema discovery and ordered placeholder collection also maintain parallel recursive match trees.

**Why it matters**

This is real duplicated work in a hot template path, not superficial similarity. It also spreads ownership across `schema.rs`, `contributions.rs`, `head_chain.rs`, `overlays.rs`, runtime slot planning and preparation.

**Evidence / affected area**

- `tir/slot_composition/head_chain.rs`
  - `is_tir_receiver`
  - `resolve_tir_chain_layer`
- `tir/slot_composition/schema.rs`
- `tir/slot_composition/overlays.rs`
- `template_slots/runtime_plan/mod.rs`
- `template_slots/runtime_plan/sites.rs`
- `tir/preparation.rs`
- `tir/fold.rs`
- `tir/handoff_materialization.rs`

**Recommended fix**

Create one slot-specific owner, not a universal visitor:

```text
tir/slot_layout.rs
```

A single fallible, cycle-guarded walk produces:

```rust
struct TirSlotLayout {
    schema: TirSlotSchema,
    placeholders: Vec<TirSlotPlaceholderRef>,
    loose_fill_target: Option<SlotKey>,
}
```

Thread the first routing result through runtime detection, structural expansion and overlay construction. Replace `SlotResolutionComposition { wrapper, fill }` with a resolved composition value that carries the already-routed contributions and layout. Do not retain a temporary fill template only so a later phase can reroute it.

Start by eliminating repeated walks inside one composition operation. Any durable per-template layout side table is a separate profile-gated slice after immutable template versioning and measured evidence. It must not become an unreviewed cache.

### 5. `fold.rs` has three near-duplicate reducers and one weak abstraction

**Issue**

Normal folding, slot-fill injection and aggregate-output injection each maintain a large `TemplateIrNodeKind` match with near-identical sequence, child, branch, loop, wrapper and signal handling. A generic closure hides only part of loop behavior while leaving the match trees duplicated. Branch selection and effective selector/header projection are also repeated.

**Why it matters**

This is the largest line-count and drift risk in TIR. Any new node kind or semantic correction must be implemented three times, while the generic closure makes the control flow harder to follow without owning a real subsystem boundary.

**Evidence / affected area**

- `tir/fold.rs`
  - `fold_tir_node_into_buffer`
  - `fold_tir_wrapper_node_with_child_output`
  - `fold_tir_aggregate_wrapper_node`
  - branch and loop helpers
  - capacity estimators

**Recommended fix**

Restructure dataflow instead of extracting more forwarding helpers:

```rust
enum FoldInsertion<'a> {
    None,
    Slot {
        key: &'a SlotKey,
        output: StringId,
    },
    Aggregate {
        output: StringId,
    },
}
```

Use one recursive reducer parameterized by the insertion behavior. Centralize branch selection and loop evaluation once. Keep fold-specific traversal local to fold. Do not replace it with a generic TIR visitor shared with handoff or metadata collection.

If the rewritten owner remains large, deepen the module:

```text
tir/fold/
  mod.rs
  reducer.rs
  control_flow.rs
  wrappers.rs
  output.rs
```

### 6. The parser construction stack is over-layered

**Issue**

The production path is effectively:

```text
TemplateConstructionContext
-> TemplateParserIrBuilderState
-> temporary TemplateIrBuilder
-> TemplateIrStore
```

`TemplateConstructionContext` and parser builder state mostly forward calls, while parser state repeatedly creates a short-lived `TemplateIrBuilder`. `TemplateIrBuilder` also carries test-only convenience methods.

**Why it matters**

The layers do not encode separate lifetimes, invariants or consumers. They add names, forwarding methods and comments while making parser ownership harder to see.

**Evidence / affected area**

- `tir/construction_context.rs`
- `tir/parser_builder_state.rs`
- `tir/builder.rs`
- `template_body_parser.rs`
- `create_template_node.rs`

**Recommended fix**

Merge parser builder state into one parser-facing construction owner. Delete production `TemplateIrBuilder` after moving fixture conveniences to `tir/tests/support`. Keep only narrow construction operations and narrow queries needed by render-unit preparation. Make `finish` consume the construction state and use its stored location rather than accepting duplicate final arguments.

Replace saturating text byte-length conversion with a checked internal error or a wider stored type. Route control-flow summary changes through the same summary owner as all other node records.

### 7. Store APIs expose invalid and silent states

**Issue**

`TemplateIrStore` vectors are accessed or mutated directly in several modules. Some checked operations return `bool`, some silently ignore invalid IDs and expression overlay allocation uses assertions. Runtime slot planning creates an empty plan entry and fills it later.

**Why it matters**

Malformed internal authority can be hidden, partially committed or converted into a panic. The style guide requires internal failures to use `CompilerError` and forbids user-driven compiler panics.

**Evidence / affected area**

- `tir/store.rs`
  - public crate-visible vectors
  - `set_template_kind`
  - `set_node_reactive_subscription`
  - overlay allocation
- `tir/control_flow_roots.rs`
- `template_slots/runtime_plan/mod.rs`
- `template_slots/runtime_plan/sites.rs`
- test support that mutates vectors directly

**Recommended fix**

Make store collections private. Add checked mutation/commit APIs:

```rust
fn template_mut(&mut self, id: TemplateIrId) -> Result<&mut TemplateIr, CompilerError>;
fn node_mut(&mut self, id: TemplateIrNodeId) -> Result<&mut TemplateIrNode, CompilerError>;
fn set_node_reactive_subscription(...) -> Result<(), CompilerError>;
fn allocate_expression_overlay(...) -> Result<TirExpressionOverlayId, CompilerError>;
fn commit_slot_plan(...) -> Result<TemplateSlotPlanId, CompilerError>;
```

Delete `control_flow_roots.rs` if it remains a forwarding layer after checked store methods move to the real owner. Build complete slot-plan data before commit, or use an explicit reservation object that cannot be observed until committed.

Counter overflow `expect` calls may remain only where the input is compiler-owned, practically unreachable and documented as an internal capacity invariant. Assertions over IDs derived from user-authored structure should become `Result`.

### 8. Summary and overlay representations admit contradictory facts

**Issue**

`TemplateIrSummary` stores overlapping count and boolean facts, some call sites check both, and derived templates can keep conservative summaries that underestimate new fill content. Its recomputation helper silently ignores missing nodes and has no cycle guard.

Overlay payloads have similar state-shape problems:

- test-only `Unresolved`
- `TirSlotResolution` stores a redundant slot key
- `Resolved { sources: Vec<_> }` permits an empty source list
- wrapper context combines an optional set and skip boolean into contradictory combinations
- duplicate overlay keys are not uniformly rejected
- lookups are linear
- `TemplateViewContext::merge` silently overwrites dimensions even though slot contexts require payload merging

**Why it matters**

Contradictory metadata creates brittle fast paths and makes performance facts unreliable. First-match overlay behavior can hide malformed data. Generic last-wins context merging is too weak for semantic dimensions with different merge rules.

**Evidence / affected area**

- `tir/summary.rs`
- `tir/overlays.rs`
- `tir/store.rs`
- `tir/slot_composition/overlays.rs`
- `tir/wrapper_sets.rs`
- summary construction in parser, copy and composition paths

**Recommended fix**

- Remove mirrored summary flags where counts already own the fact.
- Make summary recomputation fallible and cycle-guarded.
- Recompute a derived template's summary from its actual new root.
- Remove the test-only slot-resolution state.
- Represent resolved sources as a non-empty value.
- Replace wrapper-context booleans with an enum such as `SkipParent` or `Apply { set, mode }`.
- Delete the redundant resolution key unless production validates it against the placeholder.
- Sort occurrence/site keyed overlay entries, reject duplicates and use binary search. Keep a flat vector because overlays are small and deterministic. Revisit a hash map only with benchmark evidence.
- Replace generic context merge with dimension-specific constructors and composition functions.

### 9. Expression and reactive metadata traversal has grown into several parallel subsystems

**Issue**

`tir/expression_payload_walker.rs` owns production exact-view reads, nested-expression worklists, overlay normalization, slot-plan root extraction and substantial test-only mutation logic. Reactive annotation implements another environment-aware TIR expression-site traversal. Reactive metadata implements another full TIR traversal plus an owned-handoff traversal.

There are repeated implementations of:

- expression site extraction
- loop-header site matching
- child view transitions
- runtime slot-plan root enumeration
- active/completed cycle sets
- expression-overlay replacement
- cloning effective expression maps

**Why it matters**

The files are broad and the repeated structural coverage can drift. At the same time, a universal visitor would be worse because preparation, folding, reactive scope tracking and handoff have different semantic outputs.

**Evidence / affected area**

- `tir/expression_payload_walker.rs`
- `module_ast/finalization/reactive_templates/annotation.rs`
- `templates/reactive_template_metadata.rs`
- `module_ast/finalization/normalize_ast.rs`
- type-validation consumers
- `tir/slot_plan.rs`

**Recommended fix**

Extract only the truly shared facts:

1. `tir/expression_sites.rs`
   - read-only expression-site traversal
   - exact child/helper transitions
   - loop-header site matching
   - optional scope events for branch capture and loop binding boundaries

2. `tir/expression_overlays.rs`
   - collect and compose normalized overlay payloads
   - deterministic keyed replacement
   - no test mutator

3. slot-plan owner
   - one checked iterator or snapshot for contribution and site render roots

Keep preparation, fold, handoff and reactive reducers purpose-specific. Move the raw mutator and structural collector to test support or delete tests that only preserve those seams. Replace Vec linear dedup and cloned authority maps with keyed accumulation and layered lookup.

Split `reactive_template_metadata.rs` into deep submodules if it remains broad:

```text
templates/reactive_template_metadata/
  mod.rs
  tir.rs
  owned_handoff.rs
```

### 10. Runtime handoff contains clear legacy residue and avoidable state swapping

**Issue**

The mutable owned-handoff walker still emits `HandoffAfterBody`, but its comments say the old recursive `Style` payload is gone and both consumers intentionally do nothing. TIR handoff materialization swaps a mutable "current view" in and out while recursively cloning whole nodes and plans.

**Why it matters**

The event is dead compatibility scaffolding. Implicit current-view state makes recursion less local and increases the chance of restoring the wrong authority after an error.

**Evidence / affected area**

- `templates/runtime_handoff.rs`
  - `OwnedRuntimeTemplateWalkMutEvent`
- `module_ast/finalization/normalize_ast.rs`
- `module_ast/finalization/reactive_templates/annotation.rs`
- `tir/handoff_materialization.rs`

**Recommended fix**

Delete `HandoffAfterBody` and both no-op branches. Keep separate immutable and mutable handoff walkers because Rust borrowing makes a fake generic abstraction worse.

Make TIR handoff recursion accept the current `&TirView` explicitly. Snapshot only the owned fields needed after recursive calls. Add exact-view cycle detection before materializing a recursive owned tree.

### 11. Tests are strong in volume but too implementation-shaped in places

**Issue**

The TIR test directory has broad coverage, but several modules directly preserve test-only production methods, fake semantic states, placeholder IDs and the soon-to-be-redundant classifier. Some user-visible slot/wrapper combinations lack end-to-end assertions.

**Why it matters**

The testing guide requires one primary owner per behavior, integration coverage for visible behavior and unit tests for hidden invariants. Tests that pin internal vectors or convenience APIs make simplification harder without increasing semantic confidence.

**Evidence / affected area**

- `tir/tests/view_tests.rs`
- `tir/tests/classification_tests.rs`
- `tir/tests/expression_payload_walker_tests.rs`
- `tir/tests/overlays_tests.rs`
- `tir/tests/slot_composition_tests.rs`
- `tir/tests/hir_handoff_tests.rs`
- `tests/cases/` template cases

**Recommended fix**

Move malformed-store, ID, overlay, exact-view and preparation tests to focused unit owners. Move output, diagnostic and slot/wrapper semantics to canonical integration cases. Merge classifier tests into preparation tests. Delete tests whose only purpose is retaining a removed helper or state.

### 12. Comment density and migration narration are materially obscuring the code

**Issue**

Many files apply WHAT/WHY blocks mechanically to obvious accessors, constructors, fields, small enums and forwarding methods. Some comments narrate removed atom paths, old recursive `Style` payloads, temporary adapters or prior architecture. Other subtle behavior, such as expression-site remapping and exact identity preservation during derivation, lacks a single authoritative invariant comment.

**Why it matters**

The style guide says comments should explain role, ownership, ordering, failure and non-obvious data flow without restating syntax. Excess comments increase maintenance cost and make the few important invariants harder to find.

**Evidence / affected area**

The problem is broad across:

- `tir/view.rs`
- `tir/node.rs`
- `tir/store.rs`
- `tir/overlays.rs`
- `tir/builder.rs`
- `tir/summary.rs`
- `tir/fold.rs`
- `tir/handoff_materialization.rs`
- `tir/expression_payload_walker.rs`
- `tir/slot_composition/**`
- `template_slots/runtime_plan/**`
- `templates/runtime_handoff.rs`
- `templates/doc_fragments.rs`

**Recommended fix**

Update the style guide first, then run a dedicated comment-only phase after semantic owners settle. File-level docs are the primary architecture location. Keep detailed item comments for complex algorithms, phase joins, failure lanes and subtle invariants. Delete syntax restatement, migration chronology and comments that only justify a forwarding layer being removed.

## Refactor plan

### Execution rules

- Work one bounded slice at a time.
- Start every confirmed or probable bug slice with a regression.
- Preserve stable diagnostic codes, primary locations and output ordering.
- Delete the obsolete owner in the same slice as its replacement.
- Do not add compatibility wrappers, dual APIs or parallel representations.
- Do not combine broad comment cleanup with semantic changes.
- Do not create a universal TIR visitor.
- Do not absorb source-span storage, persistent caches, fold parallelism or backend string assembly from `post-tir-template-parser-optimization-plan.md`. Those remain profile-gated.
- Run focused tests after each slice, `just validate` at every code-bearing checkpoint and `just bench-check` for traversal, copying, formatting, folding or allocation changes.

### Phase 0: Freeze regressions and baseline evidence

**Target area**

Current behavior, malformed-store lanes and representative cost counters.

**Concrete change**

1. Record the reviewed repository commit.
2. Run:
   ```bash
   cargo fmt --all -- --check
   cargo test --quiet --lib compiler_frontend::ast::templates
   cargo run --quiet -- tests
   just bench-check
   ```
3. Capture current TIR counters for:
   - templates and nodes created
   - copy passes
   - slot schema/layout walks
   - contribution routing calls
   - overlay lookup counts
   - preparation nodes visited
   - fold cache hits and misses
   - handoff nodes materialized
4. Add failing regressions for:
   - control-flow direct child wrappers applied exactly once
   - a runtime wrapper slot inside a branch
   - a runtime wrapper slot inside a loop
   - copied branch and loop expression-site independence
   - nested child slot expansion preserving phase, context, wrapper set and slot plan
   - exact-view child cycle rejected before handoff

**Action**

Add tests and counters only. Do not refactor production code yet.

**Ordering**

This phase blocks every later semantic change. Probable findings that do not reproduce are downgraded and documented rather than "fixed" speculatively.

**Phase 0 result**

Reviewed at `c77dfa0f3f5decd98ce64682d65f8977973cfb06`.

Existing AST counters already covered templates/nodes created, preparation visits, fold-cache hits/misses and handoff materializations. Phase 0 added `TirCopyPasses`, `TirSlotSchemaWalks`, `TirContributionRoutingCalls` and `TirOverlayLookups`.

Regression outcomes:

- Direct `$children` wrappers around a runtime `if` child wrap exactly once. Kept live as `template_runtime_children_wrapper_applied_once`.
- Wrapper slots inside runtime branch/loop bodies are rejected today with `MOTH-SYNTAX-0022`. Live boundary cases record that diagnostic. The TIR transform still copies those roots without injecting fill; ignored unit tests own that hole for Phase 2E.
- Copied branch/loop expression sites reuse source IDs. Ignored until Phase 2B.
- Nested slot expansion resets phase/context and drops wrapper set and slot plan. Ignored until Phase 2A.
- Exact-view child cycles classify as `Runtime(ChildTemplateCycle)`. Ignored until Phase 2C. Handoff has no cycle guard and is ignored because it would recurse.

### Phase 1: Tighten the comment and test-boundary standards

#### 1A. Update the style guide

**Target area**

`docs/src/docs/codebase/style-guide/style-guide.mtf`

**Concrete change**

State explicitly:

- File-level docs are the primary place for subsystem role, ownership and detailed WHAT/WHY context.
- Detailed item comments are for complex functions, stage joins, ordering, invariants, unusual diagnostics and non-obvious data flow.
- Tiny accessors, constructors, fields, forwarding methods and obvious private helpers need no comment or one short sentence.
- WHAT/WHY headings are optional, not a template.
- Migration history belongs in Git.
- Comments must not justify obsolete adapters or compatibility paths.
- Test-only implementation belongs with tests.

**Action**

Rewrite the comment guidance. Do not remove the existing comment-positive stance.

**Ordering**

Land before the broad comment phase, but do not begin source cleanup until Phase 9.

#### 1B. Move test-only implementation out of production

**Target area**

Every `#[cfg(test)]` under `ast/templates/tir/**`.

**Concrete change**

- Move view source-location lookup and node-keyed conveniences to `tir/tests/support`.
- Move raw expression mutation/collection helpers to test support or delete their tests.
- Remove test-only semantic variants and fields.
- Move fixture constructors from production `builder.rs`, `node.rs` and `overlays.rs`.
- Keep only external test module declarations and narrowly required visibility.

**Action**

Move or remove.

**Ordering**

Complete before deleting `TemplateIrBuilder`, `classification.rs` or fake overlay states.

### Phase 2: Repair identity, cycle and wrapper/slot correctness

#### 2A. Add complete derived-template APIs

**Target area**

`tir/node.rs`, `tir/refs.rs`, `tir/store.rs` and all transforms that version templates.

**Concrete change**

- Add `TemplateIr::derived_with_root`.
- Add reference-preserving root/context methods.
- Audit every production `TemplateIr::new` call.
- A new authored template may use `TemplateIr::new`.
- A derived template must use a source-preserving API.
- Recompute summary from the derived root instead of copying stale counts.

**Action**

Rewrite derived construction and remove manual partial copies.

**Ordering**

Land before fixing nested slot expansion and formatter versioning.

#### 2B. Make expression-site copy policy complete

**Target area**

`tir/subtree_copy.rs`, `tir/store.rs`

**Concrete change**

Replace the boolean copy flag with:

```rust
enum ExpressionSiteCopyMode {
    Fresh,
    Preserve,
}
```

Implement one remapper for:

- `DynamicExpression.site_id`
- `TemplateIrBranch.selector_site_id`
- `TemplateLoopHeaderExpressionSites`

Use `Fresh` for independent copied trees. Preserve only when the copied structure remains under the same complete overlay authority and a focused test proves that identity sharing is required.

**Action**

Rewrite and delete direct site-ID preservation logic.

**Ordering**

Requires Phase 0 copy regressions. Complete before overlay indexing changes.

#### 2C. Reject exact-view cycles

**Target area**

`tir/preparation.rs`, `tir/handoff_materialization.rs`, `tir/fold.rs`

**Concrete change**

- Remove `RuntimeTemplateReason::ChildTemplateCycle`.
- Return `CompilerError` on active exact-view re-entry.
- Add a handoff cycle guard as defense in depth.
- Keep DAG sharing valid by removing a view from the active set after a completed branch. A completed-set optimization may skip repeated immutable work where output semantics permit it.
- Ensure preparation continues validating all reachable non-cyclic authority after finding runtime dependence.

**Action**

Rewrite cycle handling.

**Ordering**

Must land before preparation/classification consolidation.

#### 2D. Choose one inherited-wrapper owner

**Target area**

`tir/render_unit.rs`, `tir/slot_composition/child_wrappers.rs`, `tir/wrapper_sets.rs`, `create_template_node.rs`

**Concrete change**

After the Phase 0 regression confirms current behavior:

- Make wrapper-context overlays the sole owner for direct-child inherited wrappers.
- Remove structural direct-child wrapping from control-flow render-unit preparation.
- Retain structural wrapper construction only for genuinely owned wrapper trees, such as explicit slot expansion or aggregate wrapper materialization.
- Preserve `IfChildEmits` behavior for false branches, absent fallbacks, zero-iteration loops and control-flow signals.
- Remove any "already wrapped" or suppression flag introduced by the migration.

**Action**

Remove duplicated structural path and simplify dataflow.

**Ordering**

Complete before refactoring slot composition or fold wrappers.

#### 2E. Fix runtime wrapper injection through control flow

**Target area**

`template_slots/runtime_plan/sites.rs`, `tir/subtree_copy.rs` or a new slot-injection transform under `tir/slot_composition/`.

**Concrete change**

- Replace `build_tir_wrapper_render_pieces` with a recursive transform that injects the selected slot through all schema-reachable shapes.
- Cover sequence, branch, fallback, loop body, aggregate wrapper and nested child templates.
- Preserve exact child reference identity.
- Do not copy a control-flow root whole when its layout says it contains the target slot.
- Delete `slot_key_for_node` if the unified transform makes it redundant.

**Action**

Rewrite the partial transform.

**Ordering**

Requires derived-template APIs and expression-site remapping.

#### 2F. Align repeated default-slot behavior with the language authority

**Target area**

`tir/slot_composition/schema.rs`, template diagnostics and integration docs/tests.

**Concrete change**

`docs/src/docs/templates/template-slots.mtf` says repeated slots replay the same
contribution. Resolve the current code's rejection of a second default slot
against that authority.

Binding decision for this plan:

- Repeated default, named and positional slot occurrences are valid replay sites.
- Schema records unique target keys while placeholder layout records every occurrence.
- Remove the `MultipleDefaultSlots` diagnostic if no newer accepted design authority contradicts the language overview.
- Preserve unknown-target and loose-content diagnostics.

**Action**

Remove stale rejection and add replay tests.

**Ordering**

Decide and test before consolidating slot layout. If maintainers choose a narrower rule instead, update the language authority in the same slice and record the explicit exception.

### Phase 3: Make the store and template versions safe

#### 3A. Encapsulate store collections

**Target area**

`tir/store.rs` and direct vector users.

**Concrete change**

- Make `templates`, `nodes`, wrapper sets, slot plans and overlays private.
- Add checked borrowed access and mutation APIs.
- Replace direct indexed mutation in formatter, runtime slot planning, validation support and control-flow helpers.
- Keep malformed-store construction in a dedicated test support builder rather than production visibility.

**Action**

Move mutation ownership into the store.

**Ordering**

Land before plan transaction and formatter changes.

#### 3B. Replace silent and boolean writes

**Target area**

`tir/store.rs`

**Concrete change**

- `set_template_kind -> Result<(), CompilerError>`
- `set_node_reactive_subscription -> Result<(), CompilerError>`
- checked control-flow body and aggregate-wrapper replacement
- checked template-root version creation
- checked overlay allocation
- reject duplicate and out-of-range overlay keys before commit

**Action**

Rewrite APIs and remove caller-side boolean interpretation.

#### 3C. Commit complete runtime slot plans

**Target area**

`template_slots/runtime_plan/mod.rs`, `sites.rs`, `tir/store.rs`

**Concrete change**

Build sources and sites in a local draft, then allocate one complete `TemplateSlotPlan`. If self-references require a stable ID during construction, add an explicit reservation guard that:

- is not visible through ordinary `get_slot_plan`
- must be committed exactly once
- rolls back or remains unreachable on error

Consume vectors instead of cloning source plans into the final side table.

**Action**

Rewrite invalid intermediate state.

**Ordering**

Requires private store vectors.

#### 3D. Remove thin control-flow forwarding modules

**Target area**

`tir/control_flow_roots.rs` and any remaining one-line wrappers.

**Concrete change**

Move checked mutation to the store/control-flow owner. Keep a separate module only if it owns a real algorithm or invariant, not only forwarding names.

**Action**

Merge or delete.

### Phase 4: Collapse parser construction and make summaries truthful

#### 4A. Collapse the construction stack

**Target area**

`tir/construction_context.rs`, `tir/parser_builder_state.rs`, `tir/builder.rs`

**Concrete change**

- Merge parser state and construction context into one parser-facing type.
- Store the mutable store handle, root children, summary, control-flow node and location once.
- Call store allocation directly from that owner.
- Move `TemplateIrBuilder` fixture conveniences to tests.
- Replace broad `.builder()` exposure with narrow methods such as `root_children()` and `control_flow_node_id()`.
- Make `finish(self, style, kind, phase)` consume the state.

**Action**

Merge and delete layers.

**Ordering**

After store APIs settle. Before broad comment cleanup.

#### 4B. Fix byte length and depth accounting

**Target area**

Parser construction and `TemplateIrSummary`.

**Concrete change**

- Replace `u32` saturation with checked conversion or store `usize` if the field is only an in-memory byte count.
- Track real recursive depth for parser-emitted control flow and derived copies.
- Route control-flow, slot, child and expression records through summary methods only.
- Add debug assertions that parser summary and recomputed summary agree.

**Action**

Rewrite summary updates.

#### 4C. Remove mirrored summary facts

**Target area**

`tir/summary.rs` and all readers.

**Concrete change**

Inventory every field reader, then:

- remove `has_slots` if `slot_count` fully owns structural slots
- represent runtime slot sites separately from unresolved slots
- remove `has_insert_contributions` if the count owns the fact
- derive `is_const_evaluable_shape` from authoritative counts/flags or delete it once preparation owns semantic constness
- keep only capacity and cheap structural facts that have measured callers

Make `summarize_existing_root` return `Result<TemplateIrSummary, CompilerError>` with node and template cycle guards.

**Action**

Remove or rewrite.

**Ordering**

Coordinate with preparation consolidation so summary is never treated as semantic proof.

### Phase 5: Consolidate slot layout and composition dataflow

#### 5A. Create one slot-layout owner

**Target area**

New `tir/slot_layout.rs`, replacing duplicate walkers in `slot_composition/schema.rs` and `helpers.rs`.

**Concrete change**

One cycle-guarded walk returns:

- unique schema targets
- ordered placeholder occurrence refs
- loose-fill target
- structural `has_slots`

Use the actual placeholder location for slot diagnostics. Do not clone complete placeholders when IDs, key, wrapper-set IDs and location refs are sufficient.

**Action**

Move and merge.

**Ordering**

Requires repeated-slot decision and truthful derived identities.

#### 5B. Route once and carry the result

**Target area**

`slot_composition/head_chain.rs`, `contributions.rs`, `overlays.rs`, `helpers.rs`

**Concrete change**

Replace:

```rust
SlotResolutionComposition {
    wrapper_reference,
    fill_reference,
}
```

with a resolved value carrying:

- wrapper reference
- slot layout
- routed contributions
- any built source template IDs or direct source refs
- structural expansion result or runtime-plan decision

Use that value for:

- runtime dependence decision
- structural slot expansion
- overlay entry construction
- runtime plan materialization

Do not reroute in overlay allocation. Do not build a fill template solely to preserve reroute input.

**Action**

Restructure dataflow and delete second routing path.

#### 5C. Reconsider global merged slot overlays and wrapper copies

**Target area**

`slot_composition/child_wrappers.rs`, `helpers.rs`, `overlays.rs`, `TirView::structural_transition`

**Concrete change**

Prototype per-application slot context on the derived `ChildTemplate` reference:

- each wrapper application retains its structural root
- its child reference carries the slot-resolution context for that application
- structural transition uses that referenced slot dimension
- no global merged overlay needs fresh copied occurrence IDs

If the regression suite proves equivalent semantics, delete:

- `copy_tir_wrapper_template_with_fresh_slot_occurrence_ids`
- global duplicate-occurrence merge work for independent child views
- deep wrapper copies made only for occurrence identity

If exact root-overlay semantics require global flattening, keep the copy but document why per-view context is insufficient and record benchmark evidence. Do not leave both designs.

**Action**

Restructure or explicitly retain with evidence.

**Ordering**

After route-once dataflow and exact identity fixes. This is a bounded architecture slice, not mandatory if the prototype fails.

#### 5D. Use one error boundary

**Target area**

`slot_composition/schema.rs`, helpers, contributions and overlays.

**Concrete change**

- Reuse `TemplateError` for diagnostic versus infrastructure lanes.
- Delete `SlotSchemaError` and repetitive conversion matrices.
- Convert to `TemplateSlotError` only at the template-slot subsystem boundary.
- Never convert `CompilerError` into a user diagnostic and later reconstruct a compiler error from rendered payload.
- Preserve boxed errors where required for enum/result size.

**Action**

Merge error ownership.

### Phase 6: Make preparation the sole final semantic owner

#### 6A. Publish complete preparation facts

**Target area**

`tir/preparation.rs`

**Concrete change**

Return one `TemplatePreparation` with identity, facts and exclusive outcome. Facts include:

- shape const evaluability
- unresolved slot occurrences
- resolved slot sources
- escaped insert helpers
- wrapper foldability
- runtime slot plan/site presence
- reactive dependence
- final `TemplateConstValueKind`

Use exact view identity as the cycle key. Do not redundantly pair `TemplateIrId` with `TirViewIdentity`.

**Action**

Rewrite result shape.

#### 6B. Migrate all final consumers

**Target area**

- `create_template_node.rs`
- `ast/const_values/resolver.rs`
- `module_ast/finalization/template_helpers.rs`
- `template_control_flow/validation.rs`
- doc fragment folding
- TIR fold and handoff entry points

**Concrete change**

- Kind refresh consumes preparation facts.
- Const value resolution consumes preparation facts.
- Const-required validation matches the preparation outcome.
- Fold and handoff validate the proof identity once.
- Runtime artifact validation uses preparation facts rather than two further subtree scans.

**Action**

Move ownership and remove repeated traversal.

#### 6C. Delete or narrow `classification.rs`

**Target area**

`tir/classification.rs`

**Concrete change**

Move shared expression-kind rules to `tir/expression_constness.rs`. Move narrow pre-view structural queries to the specific construction owner that needs them. Delete:

- broad effective-view classifier
- separate slot/insert tree scans
- test-only `StructuralHeadFunction` policy
- bool APIs that suppress malformed authority

**Action**

Delete the old owner.

**Ordering**

Only after every production caller uses preparation or a narrower named query.

### Phase 7: Consolidate expression-site and overlay handling

#### 7A. Split the broad expression walker

**Target area**

`tir/expression_payload_walker.rs`

**Concrete change**

Create:

```text
tir/expression_sites.rs
tir/expression_overlays.rs
```

`expression_sites.rs` owns exact-view expression-site discovery and optional branch/loop scope events. `expression_overlays.rs` owns keyed collection, precedence and replacement.

Move mutation-only test paths to `tir/tests/support`.

**Action**

Split by responsibility.

#### 7B. Remove cloned authority maps and quadratic dedup

**Target area**

Expression overlay collector and reactive annotation.

**Concrete change**

- Accumulate by `ExpressionSiteId` in `FxHashMap` or ordered map, then sort once for deterministic storage.
- Represent overlay precedence as a small layered lookup chain rather than cloning the full map at each structural child.
- Pass a `TirView` directly instead of root, phase and context triples.
- Borrow child/branch vectors during read-only walks.

**Action**

Rewrite local data structures.

#### 7C. Centralize expression overlay composition

**Target area**

`normalize_ast.rs`, reactive annotation and TIR overlay owner.

**Concrete change**

Add one checked operation:

```rust
fn replace_expression_overlay_entries(
    store: &mut TemplateIrStore,
    base: TemplateViewContext,
    replacements: impl IntoIterator<Item = (ExpressionSiteId, Box<Expression>)>,
) -> Result<TemplateViewContext, CompilerError>;
```

It preserves untouched entries, rejects duplicate replacement IDs and returns a context with only the expression dimension changed. Delete duplicate merge code and generic context merge use.

**Action**

Move shared logic to the overlay owner.

#### 7D. Split reactive metadata by representation

**Target area**

`templates/reactive_template_metadata.rs`

**Concrete change**

Deepen the module into TIR and owned-handoff reducers. Reuse the shared expression-site and slot-plan-root owners but keep reactive metadata accumulation local.

**Action**

Split, not extract a cross-repository framework.

### Phase 8: Rewrite fold and simplify handoff

#### 8A. Unify fold reducers

**Target area**

`tir/fold.rs`

**Concrete change**

- Introduce `FoldInsertion`.
- Use one node reducer.
- Use one branch selection helper.
- Use one loop reducer that receives insertion behavior explicitly.
- Unify output estimation for the same structural modes.
- Remove `FoldTraversalInput` if it only forwards to `TirView`.
- Split into a `fold/` submodule tree only after duplication is removed.

**Action**

Rewrite and delete three parallel match trees.

**Ordering**

Requires preparation facts and stable wrapper ownership.

#### 8B. Narrow fold context to fields actually consumed

**Target area**

`template_folding.rs`, `tir/fold.rs`, finalization helpers.

**Concrete change**

The TIR reducer currently binds project services to an unused tuple to keep them "part of the contract". Separate the project-aware outer folding services from the TIR reducer context if no reducer operation uses path resolution or formatting configuration.

A likely shape:

```rust
struct TirFoldContext<'a> {
    string_table: &'a mut StringTable,
    bindings: Vec<TemplateFoldBinding>,
    loop_iteration_limit: usize,
    cache: TirFoldCache,
}
```

Keep project-aware services in the caller that actually folds path-sensitive AST expressions. Do not move them into TIR through a no-op field.

**Action**

Split wrong-layer context.

#### 8C. Make handoff recursion explicit

**Target area**

`tir/handoff_materialization.rs`

**Concrete change**

- Pass `&TirView` through recursion.
- Remove swap/restore current-view state.
- Borrow nodes and slot plans, snapshot only owned fields needed after recursion.
- Use preparation's cycle proof and a defensive active-view set.
- Consume resolved non-empty source lists directly.
- Keep slot injection and ordinary materialization in one recursive owner.

**Action**

Rewrite state flow.

#### 8D. Delete runtime-handoff legacy events

**Target area**

`templates/runtime_handoff.rs`, normalization and reactive annotation.

**Concrete change**

Remove `HandoffAfterBody`. The mutable callback receives nodes only. Delete no-op consumer branches and stale comments about recursive style templates.

**Action**

Remove dead scaffolding.

### Phase 9: Remove formatter, render-unit and composition adapter state

#### 9A. Make TIR template versions immutable after publication

**Target area**

`tir/formatter_view.rs`, store and template construction.

**Concrete change**

Formatter output creates a new template version or returns a new root to a named versioning owner. Do not mutate an existing published `TemplateIr.root`.

Parser in-progress control-flow nodes may remain mutable until `finish`, but the mutation boundary must be explicit and inaccessible through ordinary finalized store reads.

**Action**

Rewrite versioning.

#### 9B. Add node-root formatter and composition entries

**Target area**

`tir/formatter_view.rs`, `tir/render_unit.rs`, `template_render_units.rs`, head-chain composition.

**Concrete change**

- Format a body node root without pushing a temporary `TemplateIr`.
- Compose a candidate node root without a durable scratch template.
- Pass minimal style, head-prefix count and context facts.
- Retain a new `TemplateIr` only when a durable reference points to it.

**Action**

Remove adapter entries.

#### 9C. Replace whole-record clones with narrow snapshots

**Target area**

Formatter, render-unit, fold, handoff, wrapper and slot planning.

**Concrete change**

For every `TemplateIr`, `TemplateIrNode`, wrapper-set or slot-plan clone:

1. identify whether owned output or mutation requires ownership
2. if not, snapshot IDs, locations and copied scalar fields only
3. if yes, leave the clone and add no explanatory comment unless the reason is non-obvious

**Action**

Rewrite clones case by case. Do not chase `clone()` counts mechanically.

### Phase 10: Full comment review and cleanup

This is a dedicated phase. It starts only after semantic owners, file splits and APIs are stable.

#### 10A. File-level ownership pass

Review:

- `tir/mod.rs`
- every surviving `tir/*.rs`
- `tir/fold/**`
- `tir/slot_composition/**`
- `template_slots/runtime_plan/**`
- `templates/runtime_handoff.rs`
- `templates/reactive_template_metadata/**`
- `templates/doc_fragments.rs`
- `create_template_node.rs`
- `template_render_units.rs`
- direct HIR template entry modules

Each file header must state concisely:

- what the file owns
- what it deliberately does not own
- the later consumer or stage boundary
- one or two critical invariants when relevant

Delete stale module maps or update them in the same slice as file moves.

#### 10B. Item comment pass

Keep detailed comments for:

- exact view transition semantics
- phase transitions
- expression-site and occurrence identity
- slot routing order and replay
- structural-no-output wrapper behavior
- cycle and malformed-authority handling
- non-obvious mutation/commit boundaries
- diagnostic lane decisions
- fold insertion semantics
- AST-to-HIR neutrality

Shorten or remove comments on:

- getters and setters
- obvious constructors
- enum variants whose name is complete
- vector push/extend helpers
- direct delegation
- visible match arms
- "this returns X" restatement
- historical atom/content/registry implementations
- temporary migration paths that no longer exist
- Clippy explanations that can be a concise `reason = ...`

#### 10C. Missing invariant comments

Add concise WHAT/WHY context where the audit found subtle behavior but no single owner:

- why derived templates preserve complete side-table links and context
- when expression-site IDs are fresh versus preserved
- why cycle re-entry is an internal error
- why wrapper application is occurrence-contextual
- why resolved slot sources are non-empty
- why a consumer-specific traversal remains local instead of using a universal visitor
- why mutable and immutable owned-handoff walkers remain separate

#### 10D. Comment acceptance

Run:

```bash
rg 'WHAT:|WHY:' \
  src/compiler_frontend/ast/templates/tir \
  src/compiler_frontend/ast/templates/template_slots/runtime_plan \
  src/compiler_frontend/ast/templates/runtime_handoff.rs \
  src/compiler_frontend/ast/templates/reactive_template_metadata
```

This is an inspection list, not a zero-hit gate.

Acceptance is a read-through:

- important invariants are easy to find
- obvious code is not narrated
- no migration chronology remains
- no comment describes a deleted owner
- the code remains understandable without relying on this plan

### Phase 11: Test consolidation and final hardening

#### 11A. Merge redundant unit coverage

- Move final classification matrices into preparation tests.
- Delete view tests for removed test-only accessors.
- Delete fake `Unresolved` overlay tests.
- Delete builder tests that only preserve a deleted production fixture API.
- Delete raw mutator tests when production no longer exposes mutation.
- Consolidate repeated malformed-ID tests through a narrow test store builder while retaining one test per distinct error boundary.
- Keep separate tests where the boundary is different: view construction, preparation, fold proof, handoff proof and store allocation.

#### 11B. Strengthen artifact and integration assertions

For visible behavior, prefer:

- exact output
- contains exactly once
- ordered fragments
- explicit absence
- stable diagnostic code/reason
- primary and related source locations

Avoid assertions on:

- process-local numeric IDs
- internal vector length unless it is the invariant
- Debug formatting
- helper function names
- incidental template/node allocation order

#### 11C. Add performance regression evidence

Add counters or focused benchmarks for:

- one slot layout walk per composition layer
- one contribution routing pass per layer
- overlay lookup scaling
- TIR templates/nodes created by repeated wrappers
- copied expression sites
- fold reducer node visits by insertion mode
- handoff node clones/materializations
- formatter scratch template count
- nested template depth

Do not add a permanent counter unless it answers a concrete future regression question.

## Test plan

### Missing coverage

1. **Exact identity preservation**
   - nested slot expansion preserves phase and all context dimensions
   - derived templates preserve wrapper set and runtime slot plan
   - copied dynamic, branch and loop sites receive independent IDs
   - source overlay does not affect a fresh copied tree

2. **Cycle safety**
   - child-template self-cycle
   - two-template cycle
   - wrapper cycle
   - resolved-slot-source cycle
   - nested template value cycle
   - each fails through `CompilerError`, never panic or runtime classification

3. **Wrapper semantics**
   - direct child under runtime `if` is wrapped exactly once
   - false branch and no-else produce no wrapper
   - zero-iteration loop produces no wrapper
   - output before `break`/`continue` remains wrapped exactly once
   - `$fresh` suppresses only the immediate parent

4. **Runtime slots inside structural shapes**
   - target slot in branch then/fallback
   - target slot in loop body/aggregate wrapper
   - target slot in nested child template
   - repeated site replay evaluates each contribution source once
   - named-only wrapper rejects loose fill with the stable diagnostic

5. **Overlay invariants**
   - duplicate site/occurrence IDs rejected
   - out-of-range IDs rejected
   - sorted lookup returns the correct payload
   - resolved source list cannot be empty
   - contradictory wrapper context cannot be constructed
   - context composition changes only the named dimension

6. **Summary invariants**
   - parser summary equals recomputed summary
   - copied summary equals recomputed summary
   - composed summary includes contributed content
   - missing nodes and cycles return `CompilerError`
   - summary fast paths never change behavior

7. **Doc fragments**
   - note and todo are stripped without folding
   - doc is folded once
   - kind lookup occurs once per statement
   - non-foldable doc preserves diagnostic code and location

8. **Store failure lanes**
   - invalid reactive subscription write fails
   - invalid template kind write fails
   - incomplete slot plan is never visible
   - malformed overlay allocation fails without panic
   - oversized text byte length does not silently saturate

### Redundant coverage to remove or merge

- `classification_tests.rs` final-disposition tests after preparation owns facts
- `view_tests.rs` source-location convenience tests not used by production
- overlay tests for production-impossible test-only states
- expression mutator tests after the mutator moves to test support or is deleted
- builder API tests after construction collapses
- multiple tests that assert the same missing-ID behavior through identical store lookup paths
- unit tests duplicating canonical integration output without protecting a hidden invariant

### Integration cases to add

Create or extend canonical `tests/cases/` scenarios for:

- nested `$children(..)` wrappers around runtime branches and loops
- control-flow-contained wrapper slots
- repeated default, named and positional slot replay
- nested child wrappers with `$fresh`
- reactive dynamic content inside slot contributions and inherited wrappers
- const and runtime variants of the same slot/wrapper shape
- malformed slot target diagnostics
- doc comments with nested foldable templates
- a deep but legal template tree that compiles without recursion failure

Each scenario should be self-contained and use real Moth source.

### Artifact assertions

Where a backend or HIR test is required, assert semantic structure rather than incidental text dumps:

- runtime slot source count and deterministic order
- each source referenced by expected site pieces
- conditional wrapper node only where structural output is conditional
- no TIR ID or store type in owned handoff/HIR
- expected runtime append/control-flow shape
- exact emitted HTML/JS fragments only where user-visible output is the contract

Goldens that only show "some output exists" should gain exact or contains-in-order assertions.

### Regression tests for discovered defects

The following tests are release blockers for this plan:

- derived child reference retains non-empty expression/slot/wrapper context
- nested child retains `conditional_child_wrapper_set` and `runtime_slot_plan`
- copied branch/loop sites do not collide with source sites
- exact-view cycle errors before handoff
- inherited wrappers are not doubled on control-flow children
- runtime slot fill reaches a branch/loop-contained target
- missing TIR authority in runtime constness planning is `CompilerError`, not `false`
- repeated default slot behavior matches the language authority

## Doc drift

### Repeated slot semantics

`docs/src/docs/templates/template-slots.mtf` states that repeated slots replay
the same contribution. `TirSlotSchema::record_key` currently rejects a second
default slot. Resolve this contradiction in favour of the accepted language
authority unless a newer explicit design decision exists. Update implementation
tests and remove or revise the stale diagnostic.

### Stale migration narration

Remove or update references to:

- atom-level `compose_template_head_chain_atoms` when no such owner remains
- old recursive wrapper templates stored on `Style`
- test-only structural paths described as production
- "temporary" templates that remain in the durable store
- summaries described as accurate when the code intentionally preserves conservative underestimates
- direct-child wrapper ownership comments that conflict with both structural wrapping and overlay wrapping being active

### TIR educational owner map

After module splits, update:

- `docs/src/docs/codebase/compiler-design/templates-and-tir/templates-and-tir.mtf`
- `tir/mod.rs` module map
- any roadmap owner maps naming deleted files

Keep the educational explanation focused on accepted architecture, not implementation history.

### Roadmap separation

Do not fold this correction plan into `post-tir-template-parser-optimization-plan.md`.

This plan owns:

- correctness
- single semantic ownership
- removal of confirmed duplicate work
- invalid-state elimination
- line-count reduction through deletion and consolidation
- comment and test cleanup

The post-TIR optimization plan continues to own profile-gated source-span storage, persistent reuse, formatter caches, fold scheduling and backend string assembly.

## Duplication decisions

Use these decisions during implementation reviews.

| Similarity | Decision | Reason |
|---|---|---|
| Three fold node reducers | Restructure into one reducer with insertion mode | Same semantics and same output owner |
| Slot schema and placeholder walkers | Move into one slot-layout owner | Same structural coverage and same slot subsystem |
| Slot routing repeated before overlay allocation | Carry first result through dataflow | A helper would not remove the repeated work |
| Expression-site traversal in type validation, normalization and reactive annotation | Share a narrow expression-site/scope event owner | Same payload discovery, different reducers |
| Preparation, classification and runtime artifact scans | Preparation owns the complete facts | One semantic fact must have one owner |
| `TemplateTirReference`, child reference and wrapper reference | Keep distinct types | Similar fields but different transition semantics |
| Immutable and mutable owned-handoff walkers | Keep local duplication | Borrowing shapes differ and a generic visitor would obscure mutation |
| Fold, handoff, formatter and reactive metadata traversals | Keep purpose-specific | Outputs and ordering rules differ materially |
| Small typed ID newtypes | Use a local macro only for boilerplate | Distinct types are valuable, repeated impl text is not |
| Construction context, parser state and builder facade | Merge | No independent invariant or caller justifies three layers |
| Runtime slot plan draft and final plan | One explicit transaction | Partial visibility is invalid, but a draft is useful locally |
| Wrapper deep copies for global occurrence IDs | Prefer per-application view context if proven | Exact views already represent contextual identity |

## Hard gates

Inspect each result. Some are zero-hit gates, others are review lists.

```bash
# Test-only production seams
rg '#\[cfg\(test\)\]' src/compiler_frontend/ast/templates/tir --glob '!tests/**'

# Removed semantic/scaffolding states
rg 'TirSlotResolutionKind::Unresolved|is_unresolved|fn unresolved' \
  src/compiler_frontend/ast/templates

rg 'HandoffAfterBody|ChildTemplateCycle' \
  src/compiler_frontend/ast/templates

# Placeholder or incomplete identity
rg 'ExpressionSiteId::new\(0\)|preserve_expression_site_ids' \
  src/compiler_frontend/ast/templates

# Old broad semantic owner
rg 'classify_effective_tir_view_template|TirTemplateClassification' \
  src/compiler_frontend/ast

# Repeated rerouting carrier
rg 'SlotResolutionComposition|fill_reference' \
  src/compiler_frontend/ast/templates

# Direct store-vector access outside the store and test support
rg '\.(templates|nodes|slot_plans|wrapper_sets|expression_overlays|slot_resolution_overlays|wrapper_context_overlays)\b' \
  src/compiler_frontend/ast/templates \
  --glob '!tir/store.rs' \
  --glob '!tir/tests/**'

# Generic context merge
rg '\.merge\(.*TemplateViewContext|TemplateViewContext::merge|context\.merge' \
  src/compiler_frontend/ast/templates

# Scratch template narration and stale migration comments
rg 'temporary TIR template|temporary template|atom-level|old .* path|previous local' \
  src/compiler_frontend/ast/templates \
  docs/src/docs/codebase/compiler-design/templates-and-tir

# Comment inspection, not a zero-hit gate
rg 'WHAT:|WHY:' \
  src/compiler_frontend/ast/templates/tir \
  src/compiler_frontend/ast/templates/template_slots/runtime_plan \
  src/compiler_frontend/ast/templates/runtime_handoff.rs \
  src/compiler_frontend/ast/templates/reactive_template_metadata
```

Review every remaining broad clone:

```bash
rg '\.clone\(\)|\.cloned\(\)|to_owned\(\)' \
  src/compiler_frontend/ast/templates/tir \
  src/compiler_frontend/ast/templates/template_slots/runtime_plan
```

This is not a zero-clone goal. Record why each remaining whole-record clone is required.

## Final validation and acceptance

Run:

```bash
cargo fmt --all
just validate
cargo run --quiet -- tests --audit
just bench-check
```

Use a fixed targeted benchmark set covering:

- nested template depth
- wrapper depth and ordering
- repeated wrapper application
- named, positional and repeated default slots
- runtime template `if` and loops
- runtime slot applications
- expression overlays
- reactive templates
- custom `$md`
- Moth Templates and docs

Record benchmark history only when repository policy calls for an intentional recorded comparison.

Accept the completed correction work only when:

- exact derived template identity is preserved by construction
- copied expression sites are either all fresh or all deliberately preserved
- cycles fail before fold/handoff recursion
- inherited wrappers have one owner
- runtime slot injection covers every schema-reachable structural shape
- preparation is the sole complete final semantic owner
- slot layout and contribution routing are not repeated within one composition
- the parser construction stack has one meaningful owner
- store mutation cannot silently fail or expose partial plan state
- summaries and overlays cannot represent contradictory facts
- fold has one node reducer per output semantics, not three drifting match trees
- runtime handoff contains no legacy post-body event
- test-only algorithms and semantic states no longer live in production files
- comments follow the tightened policy and explain only durable architecture
- user-visible output and stable diagnostics remain unchanged except for explicitly corrected drift
- full validation passes
- representative performance is neutral or improved
