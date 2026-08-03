# Collection push fallibility split implementation plan

## Purpose

Make collection `push` semantics match the accepted language design:

- growable `{T}` push is infallible and returns no value
- fixed `{N T}` push is fallible only because the collection can be full
- both operations require explicit mutable receiver access through `~`
- one source spelling resolves statically to two compiler-owned operations
- AST owns the distinction and later stages consume it without re-inspecting source syntax or collection shape

This is a direct semantic correction. Moth is pre-release, so the implementation must replace the old unified path without compatibility aliases, forwarding helpers or legacy fallibility.

## Active context capsule

Refresh this block after every accepted slice and before compaction. Do not continue from compressed context without re-reading the authorities listed below.

```text
ACTIVE_PLAN:
- `docs/roadmap/plans/collection-push-fallibility-split-plan.md`

CURRENT_SLICE:
- Phase: Phase 0 - refresh, preserve local work and establish the baseline
- Checklist item: 0A
- Goal: re-anchor the plan in the active worktree and inventory every collection-push owner and call site
- Non-goals: no implementation edits before the current branch, local documentation work and baseline failures are understood

LAST_GOOD_COMMIT:
- `none` for task-specific validation
- GitHub planning snapshot: `ec8480a57b0bd650adecc41aa76e3445f1524599`

CURRENT_WORKTREE_STATE:
- Clean / known changes: unknown from GitHub. Preserve and classify all local changes before editing, especially the user's documentation review
- Branch: GitHub `main` at the planning snapshot. Replace with the active local branch
- Dedicated worker worktrees: none known from GitHub. Record every active worker worktree before implementation

RELEVANT_DOCS_THIS_SLICE:
- `AGENTS.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `docs/src/docs/codebase/language/overview.mtf`
- `docs/src/docs/collections/growable-collections.mtf`
- `docs/src/docs/collections/fixed-collections.mtf`
- `docs/src/docs/collections/collection-operations.mtf`
- `docs/src/docs/errors/propagation.mtf`
- `docs/src/docs/errors/catch-and-recovery.mtf`
- `docs/src/docs/bindings/mutable-bindings.mtf`
- `docs/src/docs/codebase/memory-management/overview.mtf`
- `docs/src/docs/codebase/memory-management/access-and-aliasing/overview.mtf`
- `docs/src/docs/codebase/memory-management/access-and-aliasing/access-and-aliasing.mtf`
- `docs/src/docs/progress/@page.moth`
- `docs/roadmap/roadmap.md`

RELEVANT_CODE:
- `src/compiler_frontend/datatypes/environment.rs::CollectionShape`: already exposes `element_type` and `fixed_capacity`
- `src/compiler_frontend/datatypes/environment.rs::TypeEnvironment::collection_shape`: canonical semantic shape query
- `src/compiler_frontend/builtins/mod.rs::CollectionBuiltinOp`: resolved compiler-owned collection operation identity
- `src/compiler_frontend/ast/field_access/collection_builtin.rs::parse_collection_builtin_member_typed`: current source-member resolution and fallibility owner
- `src/compiler_frontend/ast/expressions/expression_kind.rs::ExpressionKind::CollectionBuiltinCall`: AST handoff carrying the resolved operation
- `src/compiler_frontend/ast/expressions/expression.rs::collection_builtin_call_with_typed_arguments`: result-type construction for collection calls
- `src/compiler_frontend/hir/hir_expression/calls.rs::lower_collection_builtin_call_expression`: resolved AST operation to stable external call target
- `src/compiler_frontend/external_packages/ids.rs::ExternalFunctionId`: stable binding-backed call identity
- `src/builder_surface/core_packages/collections.rs::register_core_collections_package`: access and backend-lowering metadata
- `src/backends/js/runtime/collections.rs::emit_runtime_collection_helpers`: current unified runtime helper and fixed-capacity check
- `src/compiler_frontend/ast/statements/tests/collections_tests.rs`: frontend collection-call contract tests
- `src/compiler_frontend/hir/tests/hir_expression_lowering_tests.rs`: HIR target and result-lowering tests
- `src/compiler_frontend/hir/tests/reachability_tests.rs`: stable external-call reachability facts
- `src/compiler_frontend/external_packages/tests/external_packages_tests.rs`: registry contract tests
- `src/compiler_frontend/tests/external_packages_tests.rs`: frontend-facing external package tests
- `src/backends/js/tests/runtime_helpers.rs`: JavaScript helper contracts
- `tests/cases/collection_ordered_runtime_operations/`: primary growable runtime behaviour
- `tests/cases/fixed_collection_push_overflow_catch/`: fixed overflow and catch behaviour
- `tests/cases/fixed_collection_js_runtime_capacity/`: fixed capacity, removal and refill behaviour

ACCEPTANCE_CRITERIA:
- growable `{T}` push compiles only as `~items.push(value)` and returns no success value
- `catch` and postfix `!` on growable push are rejected through the existing non-fallible handling diagnostics
- fixed `{N T}` push requires `catch` or postfix `!`
- fixed push uses the existing collection capacity error when full
- growable allocation exhaustion traps or aborts and never enters `Error!`
- AST resolves growable versus fixed push once from canonical `TypeId` shape
- HIR and backends consume distinct resolved operations without re-classifying the receiver type
- the binding-backed `@core/collections` architecture remains the only lowering path
- the old unified `Push`, `CollectionPush` and `__moth_collection_push` identities are deleted
- no compatibility alias or dual runtime path remains
- growable JS push emits no result carrier and performs no fixed-capacity branch
- fixed JS push retains the result carrier and capacity check
- all repository Moth call sites are classified by receiver type and migrated correctly
- canonical docs, Basic docs, memory examples, Core package docs, progress matrix and roadmap agree
- HTML-Wasm collection lowering remains explicitly deferred and cleanly target-rejected
- generated docs are rebuilt from source rather than edited directly
- final `just validate` passes

DECISIONS_ALREADY_MADE:
- decision: growable allocation failure traps or aborts rather than returning `Error!`
  - reason: ordinary allocation exhaustion is unrecoverable and making it recoverable would force error channels across other allocating operations
  - source/user/date: user interview, 2026-08-03
- decision: both push forms keep no-value success semantics
  - reason: this changes fallibility, not the source return shape
  - source/user/date: user interview, 2026-08-03
- decision: full fixed collection push uses the existing collection error family and capacity-exceeded error
  - reason: no new error taxonomy is needed
  - source/user/date: user interview, 2026-08-03
- decision: `catch` and postfix `!` on growable push are compile-time errors
  - reason: growable push is fully infallible in source semantics
  - source/user/date: user interview, 2026-08-03
- decision: AST and HIR carry separate resolved growable and fixed push operations
  - reason: later stages must not re-inspect collection shape or rediscover source meaning
  - source/user/date: user interview, 2026-08-03
- decision: collection operations remain binding-backed through `@core/collections`
  - reason: the current owner is appropriate and this task does not justify a dedicated collection HIR statement family
  - source/user/date: user interview, 2026-08-03

BLOCKERS / RISKS:
- the user's documentation review may contain local edits that are newer than GitHub. Preserve them and integrate rather than overwrite them
- active canonical-module work may move external identity, interface or HIR files before implementation starts
- the old unified helper dynamically accepts both collection representations. A careless split could allow fixed values through the growable helper or growable values through the fixed helper
- broad search-and-replace could remove handlers from fixed pushes. Every call site must be classified from its semantic type
- docs snippets often omit surrounding type declarations. Ambiguous examples must be made explicit rather than guessed
- adding new diagnostics would duplicate the existing `CatchOnNonFallible`, `BangOnNonFallible` and unhandled-fallible machinery
- HTML-Wasm currently has no collection binding lowering. Do not accidentally claim or implement it in this plan

VALIDATION_STATE:
- last command: none run for this planning artifact
- result: not validated locally
- known unrelated failures: none identified from GitHub
- latest reported repository gate: commit `541c8f9a971df5ccb7b827d7de7ce7e251bb3261` reports a passing full gate, but this is not task-specific validation and later documentation commits exist

DOCS_IMPACT:
- progress matrix needed: yes, update the existing Collections row rather than adding a new row
- other docs stale: canonical collection references, binding examples, error examples, memory examples and Core collections package docs contain unified fallible-push wording or examples
- authorized docs updates: yes, explicitly requested by the user

NEXT_ACTION:
- complete Phase 0A by recording the active revision, branch, status, worktrees and local documentation diff, then refresh this capsule
```

## Reviewed current state

The GitHub snapshot remained at `ec8480a57b0bd650adecc41aa76e3445f1524599` when this plan was written.

Current implementation facts:

- `CollectionBuiltinOp` has one `Push` variant
- `parse_collection_builtin_member_typed` queries only the element type, creates a fallible carrier for every push and rejects every unhandled push
- AST already has canonical collection shape information through `TypeEnvironment::collection_shape`
- growable `{T}` and fixed `{N T}` are already distinct semantic types
- HIR maps every push to one `ExternalFunctionId::CollectionPush`
- `@core/collections` registers one binding-backed push function with mutable receiver access
- the JS runtime exposes one `__moth_collection_push` helper
- that helper dynamically checks fixed capacity, returns an `{ ok, err }` carrier for both shapes and succeeds for growable arrays
- fixed overflow already uses `CollectionFixedCapacityExceeded`
- collection Wasm lowering remains absent

Current documentation is internally inconsistent:

- `docs/src/docs/bindings/mutable-bindings-basic.mtf` already shows growable push without `catch`
- `docs/src/docs/bindings/mutable-bindings.mtf` still handles the same growable push
- `docs/src/docs/collections/growable-collections.mtf` explicitly says growable push remains fallible
- `docs/src/docs/collections/collection-operations.mtf` and its Basic pair state that every push is fallible
- Core collections docs describe five host functions and one fallible push helper
- the progress matrix states that all collection pushes are fallible

Treat accepted design as authoritative. Existing compiler behaviour and stale docs do not override it.

## Accepted source semantics

| Receiver type | Source form | Success | Recoverable failure | Handling rule |
|---|---|---|---|---|
| Growable `{T}` | `~items.push(value)` | no value | none | `catch` and `!` are invalid |
| Fixed `{N T}` | `~items.push(value)` | no value | full capacity | `catch` or `!` is required |

Both operations:

- require an existing mutable receiver place
- use ordinary collection element compatibility and coercion rules
- preserve the current shared, copy and optional inferred-transfer semantics for the inserted value
- remain compiler-owned receiver operations rather than source-authored methods

Growable allocation exhaustion is an unrecoverable runtime failure. Backends may trap, abort or surface their ordinary fatal allocation failure. They must not convert it into `Error!`.

## Intended internal shape

Use one source name and distinct resolved identities:

```rust
pub enum CollectionBuiltinOp {
    Get,
    Set,
    PushGrowable,
    PushFixed,
    Remove,
    Length,
}
```

Exact names may vary only when the alternative is equally explicit. Do not encode the distinction as a loose boolean on the AST node.

Centralise operation policy on the resolved enum where practical:

```rust
impl CollectionBuiltinOp {
    pub const fn requires_mutable_receiver(self) -> bool;
    pub const fn is_fallible(self) -> bool;
}
```

Expected policy:

- `Get`, `Set`, `PushFixed` and `Remove` are fallible
- `PushGrowable` and `Length` are infallible
- `Set`, `PushGrowable`, `PushFixed` and `Remove` require mutable receiver access

Stable binding-backed identities should likewise become distinct, for example:

```rust
ExternalFunctionId::CollectionPushGrowable
ExternalFunctionId::CollectionPushFixed
```

The old `CollectionPush` variant and host-name constant must be removed. Do not retain an alias.

## Scope

This plan owns:

- collection push source fallibility
- AST operation resolution from canonical collection shape
- collection operation metadata
- HIR call-target selection
- stable binding-backed push IDs and names
- Core collection package registration
- JavaScript runtime helper split
- reachability and backend-support facts for the new IDs
- compiler unit tests
- backend helper tests
- canonical integration cases
- repository Moth fixture and benchmark migration
- canonical language docs, Basic docs and examples
- progress matrix and roadmap status
- generated documentation rebuild

## Non-goals

Do not add:

- source-level overload declarations
- runtime overload dispatch
- a dedicated HIR collection statement family
- a new collection error type or diagnostic code
- `try_push`, `reserve`, `capacity` or allocation-policy APIs
- recoverable growable allocation failure
- initial-capacity hints
- fixed-to-growable or growable-to-fixed conversions
- HTML-Wasm collection lowering
- compatibility aliases for old enum variants, helper names or IDs
- benchmark history updates
- broad Core package registration frameworks

`try_push`, reserve APIs and recoverable allocation are not accepted deferred language features. Do not add progress-matrix rows or roadmap promises for them. A future proposal would need a separate design decision.

## General implementation rules

- Read the current worktree authorities, not copies from another worktree
- Preserve the user's in-progress documentation edits
- Query `TypeEnvironment::collection_shape` once in AST and carry the resolved fact forward
- Do not let HIR, reachability, package registration or the JS backend inspect source spelling to decide fallibility
- Do not let the JS backend inspect collection type to choose which push operation was meant
- Keep one current path and delete the old unified path in the same accepted implementation phase
- Reuse existing diagnostics for handling a non-fallible expression and leaving a fallible fixed push unhandled
- Keep tests outside production files
- Prefer integration cases for source-visible behaviour and focused unit tests for hidden operation IDs, carrier shape and runtime helper structure
- Do not weaken existing fixed-capacity checks or collection access rules
- Do not edit `docs/release/**` directly
- Update this plan capsule after each accepted phase and before context compaction

# Phase 0 - Refresh, preserve local work and establish the baseline

## Context

This plan was reviewed against GitHub `main`, but the user is actively editing documentation and other implementation plans are active. The first slice must re-anchor every path and preserve local work before changing semantics.

## 0A - Record the active repository state

- [ ] Read `AGENTS.md` from the active worktree
- [ ] Read every authority listed in the context capsule
- [ ] Record `git rev-parse HEAD`
- [ ] Record `git branch --show-current`
- [ ] Record `git status --short`
- [ ] Record `git worktree list --porcelain`
- [ ] Record any active worker branches that touch collection, external-package, HIR, JS runtime, docs or progress files
- [ ] Replace the planning snapshot fields in the context capsule with local facts
- [ ] Preserve all user-authored changes. Do not reset, stash, overwrite or reformat unrelated edits

## 0B - Inventory the current owners

Run local searches rather than relying on the older GitHub code-search index:

```bash
rg -n 'CollectionBuiltinOp::Push|CollectionPush|COLLECTION_PUSH_HOST_NAME|__moth_collection_push' src tests benchmarks docs --glob '!docs/release/**'
rg -n '\.push\(' tests benchmarks docs packages --glob '*.moth' --glob '*.mtf'
rg -n 'push.*fallible|fallible.*push|push failed|push failure|push.*catch|push\([^)]*\)!' docs --glob '!docs/release/**'
```

- [ ] Inventory every `CollectionBuiltinOp::Push` match
- [ ] Inventory every `ExternalFunctionId::CollectionPush` use
- [ ] Inventory the old host-name constant and runtime helper string
- [ ] Inventory package registration tests and any exact function counts
- [ ] Inventory HIR and reachability tests that assert the old ID
- [ ] Inventory backend tests that inspect the unified helper
- [ ] Inventory every Moth push call in tests, benchmarks, packages and docs
- [ ] Classify each Moth call as growable, fixed or ambiguous from its declared semantic type
- [ ] Make a short migration table in the plan capsule or phase notes. Do not rely on memory

Known starting points include:

- `tests/cases/collection_ordered_runtime_operations/`
- `tests/cases/fixed_collection_push_overflow_catch/`
- `tests/cases/fixed_collection_js_runtime_capacity/`
- `tests/cases/loop_borrow_mutation_conflict/`
- `tests/cases/collection_mutation_inside_branch/`
- `tests/cases/function_call_mutable_param_fresh_values/`
- `tests/cases/collection_mutation_through_mutable_alias/`
- `benchmarks/collection-stress.moth`
- `benchmarks/adversarial/collection-map-borrow-churn.moth`
- `benchmarks/adversarial/import-external-churn/@page.moth`

This list is not exhaustive. The local `rg` inventory is authoritative.

## 0C - Confirm current architecture has not shifted

- [ ] Confirm `CollectionShape` still carries `fixed_capacity: Option<usize>`
- [ ] Confirm transparent aliases resolve to canonical collection `TypeId`s before member lookup
- [ ] Confirm collection builtins still use `ExpressionKind::CollectionBuiltinCall`
- [ ] Confirm HIR collection builtins still lower through stable external call IDs
- [ ] Confirm `@core/collections` still owns receiver access metadata and JS lowering names
- [ ] Confirm JS growable collections are arrays and fixed collections are branded wrappers
- [ ] Confirm HTML-Wasm still has no collection operation lowering
- [ ] Confirm no other branch has already introduced a partial push split

### Stop conditions

Stop and refresh the plan before implementation when:

- canonical collection shape no longer reaches AST member lookup
- collection builtins moved to a dedicated HIR operation owner
- stable external IDs or package registration were replaced by another accepted architecture
- HTML-Wasm collection lowering landed
- another active change already split push semantics
- local documentation edits make a materially different accepted semantic claim

Do not preserve obsolete plan details through wrappers.

## 0D - Establish the baseline

Run the focused baseline first:

```bash
cargo fmt --all -- --check
cargo test --workspace --quiet -- --format terse
cargo run --quiet -- tests --tag collections --backend html
cargo run --package xtask --bin xtask -- bench-validate
```

Then run:

```bash
just validate
```

- [ ] Record every command and exact result in `VALIDATION_STATE`
- [ ] Separate task-related failures from unrelated failures
- [ ] Do not edit expectations to manufacture a green baseline
- [ ] Do not run recording benchmarks

## Phase 0 audit, style review and acceptance

- [ ] Re-read the duplication policy in `AGENTS.md`
- [ ] Confirm the planned owner remains AST member resolution
- [ ] Confirm no new syntax, type form, diagnostic family or HIR subsystem is needed
- [ ] Confirm every local user edit is preserved
- [ ] Confirm the call-site inventory distinguishes growable from fixed receivers
- [ ] Confirm the progress and roadmap impact is recorded
- [ ] Refresh the context capsule
- [ ] Commit only the plan and roadmap activation change if those are being added in this slice

### Phase 0 roadmap action

When implementation begins:

- [ ] Ensure this file exists at `docs/roadmap/plans/collection-push-fallibility-split-plan.md`
- [ ] Add one link under `docs/roadmap/roadmap.md` -> `Active implementation work`
- [ ] Do not reorder the hard dependency chain. This correction is independent of that chain

# Phase 1 - Complete the semantic and runtime cutover

## Context

This phase is one vertical cutover. It must replace the unified operation from AST through JS and migrate every executable repository call site before the phase is accepted.

Do not checkpoint a state that keeps old and new push IDs or helpers in parallel. Subtasks may be implemented incrementally in the worktree, but the accepted phase must expose one coherent path and pass the full code-bearing gate.

## 1A - Split the resolved collection operation

- [ ] Replace `CollectionBuiltinOp::Push` with explicit growable and fixed variants
- [ ] Keep `Get`, `Set`, `Remove` and `Length` unchanged
- [ ] Add or update narrow enum methods for mutable receiver requirements and fallibility
- [ ] Keep source-name mapping local to the collection builtin owner
- [ ] Avoid a loose `is_fixed` boolean on AST expressions
- [ ] Avoid a new generic builtin-method framework
- [ ] Update comments so the enum is a resolved semantic operation rather than a source token category

Preferred policy:

```rust
PushGrowable => mutable and infallible
PushFixed => mutable and fallible
```

## 1B - Resolve push from canonical collection shape in AST

In `parse_collection_builtin_member_typed`:

- [ ] Replace the element-only query with one `collection_shape(receiver_type_id)` query
- [ ] Read `element_type` from that shape
- [ ] Resolve source `push` to growable or fixed from `fixed_capacity.is_some()`
- [ ] Build growable push with no success result slots and no fallible carrier
- [ ] Build fixed push with the existing `None, Error!` internal carrier
- [ ] Resolve the builtin `Error` type only for operations that are actually fallible
- [ ] Require a handling suffix only for `Get`, `Set`, `PushFixed` and `Remove`
- [ ] Let the shared suffix parser reject `catch` or `!` applied to `PushGrowable`
- [ ] Preserve existing missing-parentheses, argument-type and receiver-mutability diagnostics
- [ ] Preserve positional-only builtin argument rules
- [ ] Preserve no-value success handling for fixed push

Do not add a push-specific non-fallible diagnostic. The existing handling reasons already own this behaviour.

## 1C - Carry distinct identities through HIR and the binding-backed package

In `src/compiler_frontend/external_packages/ids.rs`:

- [ ] Replace `ExternalFunctionId::CollectionPush` with distinct growable and fixed IDs
- [ ] Replace `COLLECTION_PUSH_HOST_NAME` with distinct host-name constants
- [ ] Update `ExternalFunctionId::name`
- [ ] Delete the old variant and constant in the same change
- [ ] Audit any stable-ID arrays, matches, debug names, hashes or fixtures for the new variants

In `src/builder_surface/core_packages/collections.rs`:

- [ ] Remove the old unified push registration
- [ ] Register growable push with mutable receiver access, shared element access, void success and JS runtime helper lowering
- [ ] Register fixed push with the same access modes and its own JS helper lowering
- [ ] Keep both Wasm lowerings absent
- [ ] Keep the current compiler-owned collection carrier bridge. Do not redesign all collection calls as public external fallible signatures in this task
- [ ] Avoid adding a general registration abstraction only to hide two readable definitions

In `lower_collection_builtin_call_expression`:

- [ ] Map `PushGrowable` directly to the growable external ID
- [ ] Map `PushFixed` directly to the fixed external ID
- [ ] Keep HIR free from collection-shape reclassification
- [ ] Confirm growable push lowers to `HirStatementKind::Call` with `result: None`
- [ ] Confirm fixed push still materialises and branches over the internal fallible carrier when handled
- [ ] Keep receiver argument access mutable for both

## 1D - Split the JavaScript runtime helpers

Replace the unified helper with two functions, for example:

```text
__moth_collection_push_growable
__moth_collection_push_fixed
```

Growable helper contract:

- [ ] Accept the already-resolved growable representation
- [ ] Append the value
- [ ] Return no carrier and no source-visible success value
- [ ] Perform no fixed-capacity lookup or branch
- [ ] Do not catch JavaScript allocation errors
- [ ] Do not convert allocation failure into `Error!`
- [ ] Do not reuse a broad accessor that would silently accept a fixed wrapper
- [ ] Let an impossible representation mismatch trap as an internal/backend failure rather than return a recoverable source error

Fixed helper contract:

- [ ] Require a valid fixed collection wrapper
- [ ] Read its `items` and `fixedCapacity`
- [ ] Return the existing invalid-collection error for malformed fixed representation if the current helper contract retains that guard
- [ ] Return `CollectionFixedCapacityExceeded` when logical length is at capacity
- [ ] Append and return the existing `{ tag: "ok", value: null }` carrier below capacity
- [ ] Preserve removal freeing one slot for a later fixed push

Shared runtime cleanup:

- [ ] Keep broad collection validation for `get`, `set` and `remove`
- [ ] Add a narrow fixed-wrapper validation helper only if needed by more than one clear branch or if it materially improves correctness
- [ ] Delete `__moth_collection_fixed_capacity` if the split leaves it unused
- [ ] Delete the old `__moth_collection_push`
- [ ] Update file-level and runtime-prelude comments from five unified operations to six binding-backed functions where relevant
- [ ] Do not make collection helper emission demand-driven in this task

## 1E - Update focused Rust tests

### Frontend operation and diagnostics

- [ ] Change growable push parsing tests to expect `PushGrowable`
- [ ] Add or update fixed push parsing tests to expect `PushFixed`
- [ ] Replace the old test that rejects unhandled growable push with an unhandled fixed push test
- [ ] Keep a test that accepts fixed push postfix propagation
- [ ] Keep a test that accepts fixed push `catch`
- [ ] Add a test that accepts growable push with no handling
- [ ] Add a test that rejects `catch` on growable push through the existing non-fallible reason
- [ ] Add a test that rejects postfix `!` on growable push through the existing non-fallible reason
- [ ] Test transparent growable and fixed collection aliases if current coverage does not prove shape-sensitive member resolution
- [ ] Test a fixed capacity supplied through a visible `#Int` constant if current push-resolution coverage does not already reach that path
- [ ] Keep tests outside production files

### HIR

- [ ] Assert growable push uses the growable external target
- [ ] Assert growable push has no result local or carrier branch
- [ ] Assert fixed push uses the fixed external target
- [ ] Assert handled fixed push retains success/error control flow
- [ ] Update every old `CollectionPush` fixture and expected debug name

### Registry and reachability

- [ ] Assert both push IDs are registered
- [ ] Assert both receiver parameters use mutable access
- [ ] Assert both element parameters use shared access
- [ ] Assert both JS lowering names are distinct
- [ ] Assert both Wasm lowerings remain absent
- [ ] Assert reachability records the two external IDs distinctly
- [ ] Update any exact function-count expectations from five to six binding-backed functions

### JS runtime

- [ ] Assert the growable helper mutates without returning a tagged carrier
- [ ] Assert the growable helper contains no fixed-capacity check
- [ ] Assert the growable helper contains no capacity-exceeded error path
- [ ] Assert the fixed helper uses the existing capacity error code and message
- [ ] Assert the fixed helper returns an ok carrier after mutation
- [ ] Assert the old unified helper is absent
- [ ] Keep or strengthen the runtime integration test that proves overflow leaves length and values unchanged
- [ ] Keep the remove-then-push refill test

Avoid brittle assertions that freeze whitespace or unrelated helper formatting. Runtime source assertions should protect semantic structure only.

## 1F - Migrate executable Moth sources and fixtures

Use the Phase 0 inventory. Do not perform a blind replacement.

For every push call:

- [ ] Resolve the receiver's semantic type from its declaration, alias or parameter
- [ ] Remove `catch` or `!` only when the receiver is growable `{T}`
- [ ] Retain handling when the receiver is fixed `{N T}`
- [ ] Convert intentionally fallible test fixtures from growable to fixed when their purpose is testing push failure or propagation
- [ ] Keep `~` on both growable and fixed receiver calls
- [ ] Make ambiguous test setup explicit rather than relying on inference readers cannot see

Required existing case updates:

- [ ] Update `collection_ordered_runtime_operations` so its growable push is unhandled and its exact runtime output remains unchanged
- [ ] Preserve `fixed_collection_push_overflow_catch`
- [ ] Preserve `fixed_collection_js_runtime_capacity`
- [ ] Review every fixed-capacity import, alias, facade and const-capacity case for the fixed operation identity
- [ ] Update collection borrow cases without changing the borrow contract they own
- [ ] Update mutable-parameter cases without weakening their access-mode contract

Add canonical user-visible diagnostics coverage if it does not already exist:

- [ ] one primary or boundary case for `catch` on growable push being rejected
- [ ] one primary or boundary case for postfix `!` on growable push being rejected
- [ ] one primary case for unhandled fixed push being rejected if no existing integration case owns it
- [ ] use existing diagnostic codes and reason keys rather than rendered-message-only matching
- [ ] assign manifest contracts and roles according to `testing.mtf`
- [ ] avoid duplicate primary ownership

Benchmark fixtures:

- [ ] Remove handling from growable pushes in benchmark source
- [ ] Retain handling in any fixed-capacity benchmark source
- [ ] Run benchmark preflight only
- [ ] Do not record benchmark history or tracked summaries

## Phase 1 audit and style-guide review

Before validation:

- [ ] Re-read `style-guide.mtf`, `testing.mtf` and `validation.mtf`
- [ ] Confirm AST is the sole shape-to-fallibility owner
- [ ] Confirm HIR does not call `collection_shape` to select a push target
- [ ] Confirm JS does not choose push semantics by inspecting source-level type metadata
- [ ] Confirm no loose boolean duplicates the resolved enum state
- [ ] Confirm no compatibility alias preserves `Push`, `CollectionPush` or `__moth_collection_push`
- [ ] Confirm the old helper and any now-unused capacity accessor are deleted
- [ ] Confirm growable push has no carrier allocation or capacity branch
- [ ] Confirm fixed overflow remains recoverable through the existing error path
- [ ] Confirm no borrow or lifetime algorithm was changed without need
- [ ] Confirm tests own behaviour at the right level and do not duplicate one another
- [ ] Confirm benchmark history is unchanged
- [ ] Review `index.md`. No edit is expected unless files move, new modules are added or locator text becomes inaccurate

Run stale-path searches:

```bash
rg -n 'CollectionBuiltinOp::Push\b|ExternalFunctionId::CollectionPush\b|COLLECTION_PUSH_HOST_NAME\b|__moth_collection_push\b' src tests benchmarks packages
```

Expected result: no old production identity remains. Matches in this plan or intentional historical text must be clearly excluded.

## Phase 1 validation and acceptance

Run targeted iteration:

```bash
cargo fmt --all
cargo test --workspace --quiet -- --format terse
cargo run --quiet -- tests --case collection_ordered_runtime_operations --backend html
cargo run --quiet -- tests --case fixed_collection_push_overflow_catch --backend html
cargo run --quiet -- tests --case fixed_collection_js_runtime_capacity --backend html
cargo run --quiet -- tests --tag collections --backend html
cargo run --quiet -- tests --audit
cargo run --package xtask --bin xtask -- bench-validate
```

Then run the required code-bearing gate:

```bash
just validate
```

- [ ] Record exact counts and results in the context capsule
- [ ] Resolve every task-related failure before accepting the phase
- [ ] Record unrelated failures with evidence rather than weakening tests
- [ ] Perform a read-only audit agent review if available
- [ ] Resolve all audit findings
- [ ] Refresh the context capsule
- [ ] Commit the complete cutover as one accepted checkpoint

Phase 1 is not accepted while any executable repository source still relies on fallible growable push.

# Phase 2 - Align documentation, status and deferred coverage

## Context

Phase 1 makes the implementation coherent. This phase updates every source authority and teaching layer, records the exact current support in the matrix and keeps deferred target work in the correct roadmap owner.

This phase should be documentation-only unless the documentation review reveals a real implementation defect. If Rust, tests, fixtures, scripts or benchmarks change, classify the phase as code-bearing and use the full gate instead.

## 2A - Update canonical collection references

### Growable collections

In `docs/src/docs/collections/growable-collections.mtf`:

- [ ] State that `~items.push(value)` is infallible
- [ ] State that it returns no value
- [ ] State that `catch` and postfix `!` are invalid
- [ ] State that allocation exhaustion is an unrecoverable trap or abort rather than `Error!`
- [ ] Remove wording that runtime allocation and backend failures must become checked language behaviour
- [ ] Use an explicit growable type in examples

### Fixed collections

In `docs/src/docs/collections/fixed-collections.mtf`:

- [ ] State that fixed push is fallible because logical length can reach capacity
- [ ] Show handling on a fixed receiver
- [ ] Explain that `remove` frees one slot
- [ ] Keep capacity as semantic type identity
- [ ] Keep fixed capacity distinct from allocation hints

### Collection operations

In `collection-operations.mtf` and `collection-operations-basic.mtf`:

- [ ] Split the push contract into growable and fixed cases
- [ ] Keep one source spelling
- [ ] Make receiver types explicit in examples
- [ ] Change shared rules to: `get`, `set`, `remove` and fixed push require handling
- [ ] State that growable push must not use handling
- [ ] Keep `length` infallible
- [ ] Keep `set` replacement-only and `remove` index semantics unchanged

## 2B - Update every cross-topic example

Review every result from the Phase 0 docs inventory. Known affected areas include:

- [ ] `docs/src/docs/bindings/mutable-bindings.mtf`
- [ ] `docs/src/docs/bindings/mutable-bindings-basic.mtf`
- [ ] `docs/src/docs/bindings/shared-access.mtf`
- [ ] `docs/src/docs/bindings/explicit-copies.mtf`
- [ ] `docs/src/docs/errors/propagation.mtf`
- [ ] `docs/src/docs/errors/catch-and-recovery.mtf`
- [ ] `docs/src/docs/memory/reference-semantics.mtf`
- [ ] `docs/src/docs/memory/reference-semantics-basic.mtf`
- [ ] `docs/src/docs/memory/copy-and-exclusive-access.mtf`
- [ ] `docs/src/docs/memory/copy-and-exclusive-access-basic.mtf`
- [ ] `docs/src/docs/codebase/memory-management/access-and-aliasing/access-and-aliasing.mtf`

Rules for examples:

- [ ] Growable mutation examples use unhandled push
- [ ] Fallible propagation and catch examples use an explicitly fixed collection or another clearly fallible operation
- [ ] Do not leave type-ambiguous `~items.push(value)!` examples
- [ ] Do not imply `~` creates fallibility. It requests exclusive access only
- [ ] Do not change the memory model, alias rules or transfer semantics
- [ ] Keep Advanced and Basic pairs consistent while preserving their different teaching depth

## 2C - Update Core collections package documentation

In `docs/src/docs/packages/core/collections/collections.mtf`:

- [ ] Change the binding-backed inventory from five functions to six
- [ ] Document separate growable and fixed host functions
- [ ] Keep both source-visible as the same `push(value)` member spelling
- [ ] Explain that AST statically selects the binding from receiver type
- [ ] Mark growable push as void and infallible
- [ ] Mark fixed push as void-success and fallible on capacity
- [ ] Keep mutable receiver access for both
- [ ] Keep direct import unsupported
- [ ] Keep HTML-Wasm lowering deferred

In `collections-basic.mtf`:

- [ ] Show a normal growable push without handling
- [ ] Add a compact fixed push example only where the distinction needs teaching
- [ ] Do not expose internal helper names unless the Advanced page owns that detail

Review package and compiler architecture docs:

- [ ] Review `docs/compiler-design-overview.md` for stale claims. An edit is expected only if it explicitly says every push is fallible or fails to preserve the AST-resolved-operation boundary
- [ ] Review `docs/build-system-design.md`. No edit is expected because package registration ownership and target orchestration do not change
- [ ] Review `index.md`. No edit is expected without moved or fundamentally re-owned files

Do not add prose merely to mention implementation names in broad architecture documents.

## 2D - Update the progress matrix

Update the existing **Collections** row in `docs/src/docs/progress/@page.moth`.

- [ ] Keep status `Supported` for the Alpha JS/HTML surface
- [ ] Keep runtime target `JS / HTML`
- [ ] Replace the claim that every push is fallible
- [ ] State that growable push is infallible and rejects handling
- [ ] State that fixed push is fallible when full
- [ ] Keep `get`, `set` and `remove` fallible
- [ ] Keep `length` infallible
- [ ] Update coverage text to mention growable/fixed AST resolution, non-fallible-handler diagnostics, HIR target split, JS helper split and fixed overflow runtime coverage
- [ ] State that HTML-Wasm collection lowering remains deferred or unsupported by target validation
- [ ] Do not add separate rows for growable push and fixed push
- [ ] Do not add rows for `try_push`, reserve APIs or recoverable allocation because those are not accepted design

## 2E - Update the roadmap and explicit deferred ownership

While this plan is active:

- [ ] Keep the plan linked under `Active implementation work`
- [ ] Do not insert it into the hard dependency chain

Under `Collection follow-ups`, add or refine one explicit deferred target item:

- [ ] State that HTML-Wasm lowering for compiler-owned collection operations remains deferred
- [ ] State that future lowering must preserve infallible growable push and fallible fixed push
- [ ] Link that work to `html_project_backend_wasm_final_implementation_plan.md` rather than creating a duplicate plan

Keep existing deferred collection items:

- default-fill syntax
- explicit fixed/growable conversion after copy and cast hardening
- profiling-justified growable initial-capacity hints

Do not add recoverable growable allocation or `try_push` as accepted follow-ups.

## 2F - Rebuild and inspect generated docs

Run the documentation release build through the current compiler:

```bash
moth build docs --release
```

Use the Cargo form when no suitable release binary exists:

```bash
cargo run --quiet -- build docs --release
```

- [ ] Do not edit `docs/release/**` manually
- [ ] Inspect the generated diff
- [ ] Inspect the Collections route
- [ ] Inspect the Bindings route
- [ ] Inspect the Errors route
- [ ] Inspect the Memory route
- [ ] Inspect the Core collections package route
- [ ] Inspect the Progress Matrix route
- [ ] Verify code highlighting and line wrapping remain readable
- [ ] Verify Advanced and Basic tabs tell compatible stories
- [ ] Verify no stale generated page still says every push is fallible

## Phase 2 audit and style-guide review

Run source searches:

```bash
rg -n 'push remains fallible|all collection pushes|push.*must.*catch|push.*must.*handled' docs --glob '!docs/release/**'
rg -n '\.push\(' docs --glob '*.mtf' --glob '*.moth' --glob '!docs/release/**'
```

- [ ] Classify every remaining handled push example as fixed
- [ ] Confirm every growable push example is unhandled
- [ ] Confirm every ambiguous snippet now establishes its collection shape
- [ ] Confirm no prose treats mutability as part of collection type identity
- [ ] Confirm no prose treats allocation failure as recoverable
- [ ] Confirm the Core package count and helper names are current
- [ ] Confirm the matrix reports current support rather than future architecture
- [ ] Confirm the roadmap owns only genuine accepted deferrals
- [ ] Apply British English and the documentation style guide
- [ ] Remove stale wording instead of preserving both descriptions
- [ ] Refresh the context capsule

## Phase 2 validation and acceptance

If the phase is strictly documentation-only, run only the required documentation gate:

```bash
moth build docs --release
```

or its Cargo equivalent.

- [ ] Record the command and result
- [ ] Confirm the changed-file list contains documentation only
- [ ] Confirm generated output came from source changes
- [ ] Resolve every broken link, code example, table or route
- [ ] Perform a read-only documentation audit if available
- [ ] Resolve all findings
- [ ] Commit the documentation, matrix and roadmap alignment checkpoint

If any non-documentation file changed, run `cargo fmt` where relevant and use `just validate` instead.

# Phase 3 - Final cross-layer audit and closeout

## Context

The final phase proves that one source spelling now has two statically resolved semantics without leaving duplicated identities, stale examples or false target claims.

Complete roadmap closeout edits before the final gates so validation covers the final repository state.

## 3A - Close the plan and roadmap state

- [ ] Mark all completed checklist items
- [ ] Set the context capsule status and next action to final audit
- [ ] Remove this plan from `Active implementation work`
- [ ] Add it under `Completed` with its final title
- [ ] Record the accepted implementation commit and docs commit in the capsule
- [ ] Record any deliberately deferred follow-up and its owner
- [ ] Do not delete implementation history from the plan

## 3B - Search for obsolete paths and semantic drift

Run:

```bash
rg -n 'CollectionBuiltinOp::Push\b|ExternalFunctionId::CollectionPush\b|COLLECTION_PUSH_HOST_NAME\b|__moth_collection_push\b' . --glob '!target/**' --glob '!docs/release/**'
rg -n 'push remains fallible|every push is fallible|all collection pushes are fallible' docs --glob '!docs/release/**'
rg -n '\.push\(' tests benchmarks packages docs --glob '*.moth' --glob '*.mtf' --glob '!docs/release/**'
```

- [ ] Confirm old identities are absent from current implementation
- [ ] Confirm every handled push is fixed
- [ ] Confirm every growable push is unhandled
- [ ] Confirm any historical references are clearly historical and not active guidance
- [ ] Confirm no generated file was hand-edited

## 3C - Architecture and complexity audit

- [ ] AST queries canonical collection shape once and resolves the operation
- [ ] AST carries an enum variant, not a loose fallibility boolean
- [ ] HIR maps the resolved variant directly to a stable target
- [ ] HIR does not inspect fixed capacity to choose semantics
- [ ] reachability sees distinct external IDs without special-case source logic
- [ ] package registration contains one definition per current helper
- [ ] JS growable helper has no carrier and no capacity branch
- [ ] JS fixed helper uses the existing capacity error
- [ ] no old helper, ID, constant, wrapper or compatibility shim remains
- [ ] no broad abstraction was added for a six-function Core package
- [ ] no duplicated validator was added without a clear representation invariant
- [ ] no borrow, lifetime or ownership rule changed
- [ ] no backend target claims support that does not exist
- [ ] comments name the current owner and behaviour

## 3D - Test-quality audit

- [ ] User-visible success and diagnostics are covered by canonical integration cases
- [ ] Unit tests protect operation IDs, carrier shape, access metadata and helper structure only where integration output cannot
- [ ] fixed overflow is tested at runtime
- [ ] remove-then-push slot recovery is tested
- [ ] growable push has an observable runtime success case
- [ ] growable `catch` and `!` rejection use stable diagnostics
- [ ] fixed unhandled push rejection remains covered
- [ ] transparent aliases and const fixed capacities cannot select the wrong operation
- [ ] no benchmark fixture is the sole correctness owner
- [ ] no redundant case duplicates an existing primary contract
- [ ] integration manifest policy passes

## 3E - Final validation

Run the complete final state gates:

```bash
cargo fmt --all -- --check
cargo run --quiet -- tests --audit
just validate
moth build docs --release
```

Use the Cargo documentation build when needed.

- [ ] Record exact command results and counts
- [ ] Confirm benchmark history and tracked summaries did not change
- [ ] Confirm generated docs are current after the final source state
- [ ] Confirm `git status --short` contains only intended files before commit
- [ ] Resolve every task-related failure
- [ ] Report environmental failures with exact evidence and remaining uncertainty

## Final audit

Use a read-only final auditor when available. Give it this plan, the final diff, the accepted decisions and all validation results.

The final audit must check:

- [ ] accepted language semantics are implemented exactly
- [ ] stage ownership matches compiler architecture
- [ ] no later stage reconstructs AST's shape decision
- [ ] no duplicate or legacy path remains
- [ ] runtime fallibility matches source fallibility
- [ ] diagnostics use existing structured lanes
- [ ] tests protect behaviour rather than incidental formatting
- [ ] documentation and examples are consistent
- [ ] progress matrix reports current support
- [ ] roadmap records only genuine deferred work
- [ ] style guide and validation guide were followed

Resolve every actionable finding, rerun affected gates and update the context capsule before declaring the plan complete.

# Validation summary by phase

| Phase | Change class | Required acceptance gate |
|---|---|---|
| Phase 0 | read-only or plan-only | baseline commands, then the applicable docs or code gate if files changed |
| Phase 1 | code-bearing | targeted tests, integration audit, benchmark preflight and `just validate` |
| Phase 2 | documentation-only unless implementation corrections occur | docs release build, route inspection and generated-diff review |
| Phase 3 | whole-plan closeout | integration audit, `just validate` and docs release build |

# Definition of done

The plan is complete only when:

- [ ] `~growable.push(value)` is accepted without handling
- [ ] growable push with `catch` is rejected
- [ ] growable push with postfix `!` is rejected
- [ ] fixed push without handling is rejected
- [ ] fixed push with `catch` or postfix `!` is accepted
- [ ] full fixed push returns the existing capacity error
- [ ] growable push returns no carrier and has no capacity branch
- [ ] fixed push retains its carrier and capacity branch
- [ ] AST owns the only growable/fixed semantic decision
- [ ] HIR and JS consume distinct resolved targets
- [ ] all old unified identities are deleted
- [ ] all repository source and fixtures use the correct handling form
- [ ] canonical docs and teaching docs agree
- [ ] the Core package docs list six internal functions
- [ ] the progress matrix distinguishes growable and fixed push
- [ ] the roadmap points HTML-Wasm collection lowering to its existing owner
- [ ] generated docs are rebuilt
- [ ] the final audit has no open findings
- [ ] the final required gates pass
