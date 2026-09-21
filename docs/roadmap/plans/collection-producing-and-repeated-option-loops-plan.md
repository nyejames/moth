# Collection-producing and repeated option-capture loops implementation plan

## Purpose

Extend Moth's existing `loop` model in two small, composable directions:

1. Add eager collection-producing loops that use the existing terminal `then` statement to build one growable `{T}` result.
2. Add a repeated option-capture loop header that reevaluates any `T?` expression once per attempted iteration, enters the body for a present value and exits on `none`.

These features cover the common jobs that iterator `map`, `filter`, `filter_map` and simple pull cursors solve in other languages without adding a first-class iterator protocol, closures, generators or resumable generator frames.

The implementation stays frontend-owned. Both source forms lower to ordinary HIR locals, calls, branches and loop CFG. No iterator, generator, producer, suspension or loop-result concept crosses the AST-to-HIR boundary.

`yield` is not part of this design. The async design draft already reserves `yield` for explicit coroutine suspension.

## Current-state capsule

```text
status: queued, low priority
current_slice: none
blockers: roadmap order only, plus the ordinary loop, closed value-production, option-capture, collection mutation and memory-analysis owners must still be stable when this plan activates
next_action: when activated, refresh every named owner from the current worktree and run Phase 0 before changing syntax
```

## Roadmap position

Place this plan at the bottom of the `Plans` list after the current Moth-native MON integration and static asset builder item.

Suggested roadmap entry:

```markdown
- [Collection-producing and repeated option-capture loops](./plans/collection-producing-and-repeated-option-loops-plan.md) - Low priority. Add eager `{T}`-producing loops through terminal `then` and repeated `T?` option-capture loop headers, lowering both to ordinary HIR CFG and collection operations without iterator or generator abstractions.
```

The current Moth-native MON roadmap item says it should run "after every other currently listed roadmap item". Once this plan is added below it, change that wording to mean "after every roadmap item listed above it" or another wording that preserves the visible top-to-bottom order. Do not leave two entries that both claim to be last.

This plan is deliberately late. Nothing earlier should be reordered to make it convenient.

At closeout, delete this plan and remove its roadmap entry in the same commit.

## Hard prerequisites

Before activation, confirm these capabilities are already delivered and stable:

- the single `loop` keyword and its conditional, collection and range header forms
- one shared loop-header parser for statement and template loop syntax
- closed receiving value production through `then`
- declaration, assignment and return receiving-context typing for value-producing control flow
- option-present capture through `is |name|`
- ordinary growable collection construction and growable `push`
- explicit HIR CFG lowering for conditional, collection and range loops
- stable `break` and `continue` targeting for nested loops
- HIR validation, borrow validation and lifetime-region analysis over ordinary loop CFG
- target validation for ordinary collection construction and mutation
- the current async design still owns `yield` as explicit suspension syntax

No iterator protocol, generator support, closure support or async implementation is a prerequisite.

## Required authorities

Read these from the active worktree before implementation:

- `AGENTS.md`
- `docs/compiler-design-overview.md`
  - `Architectural invariants`
  - `Frontend stages > Stage 4: AST semantics`
  - `Frontend stages > Stage 4: AST semantics > Value-producing blocks and terminality`
  - `Frontend stages > Stage 5: HIR and validation`
  - `Frontend stages > Stage 6: borrow validation`
  - `Lifetime-region and escape validation`
- `docs/build-system-design.md`
  - target validation and backend handoff sections if backend capability coverage is affected
- `docs/src/docs/loops/@page.moth`
- `docs/src/docs/loops/conditional-loops.mtf`
- `docs/src/docs/loops/collection-loops.mtf`
- `docs/src/docs/loops/range-loops.mtf`
- `docs/src/docs/loops/loop-control.mtf`
- `docs/src/docs/branching/value-producing-if.mtf`
- `docs/src/docs/errors/options.mtf`
- `docs/src/docs/async/@page.moth`
- `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf`
- `docs/src/developer-docs/memory-management/overview.mtf`
- the routed borrow-validation and lifetime material for collection retention and loop liveness
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`
- `docs/src/docs/progress/@page.moth`
- `docs/roadmap/roadmap.md`

The active documentation is the authority if any source path or internal Rust name in this queued plan has moved.

# Accepted design

## Language role

`loop` remains Moth's primary iteration abstraction.

Collection-producing loops provide an eager, imperative-looking replacement for common iterator `map`, `filter` and `filter_map` pipelines while preserving normal Moth control flow.

Repeated option-capture loops provide a small pull-loop convenience for parsers, cursors, queues and other operations that naturally return `T?`.

Neither feature creates an iterable object model.

There is no built-in `Iterator` type, iterator trait, associated `Item` type, adapter chain or magic `next` method in this plan.

There is no generator or sequence-producing `yield` form. `yield` keeps its async design meaning as an explicit suspension point.

## Collection-producing loop syntax

A `loop` at an accepted closed receiving site may produce one growable collection.

```moth
values = loop items |item|:
    if item.enabled:
        then item.value
    ;
;
```

This produces a growable collection containing `item.value` for the iterations where `item.enabled` is true.

Range loops use the same form:

```moth
squares = loop 1 to 100 |number|:
    if number % 2 is 0:
        then number * number
    ;
;
```

Conditional loops may also produce a collection:

```moth
values = loop has_more():
    value = read_next()

    if keep(value):
        then value
    ;
;
```

Collection production is orthogonal to the loop source. The accepted statement-loop sources are:

- conditional loops
- collection loops
- range loops
- the repeated option-capture loop added later in this plan

Template loops remain a separate template-language feature and do not become collection-producing loops.

## Eager result semantics

A producing loop constructs its complete result before the receiving statement continues.

It is not lazy.

It does not return a cursor, iterator, generator or suspended frame.

The result type is always a growable collection:

```text
{T}
```

The v1 surface does not infer or produce:

- fixed collections
- maps
- strings
- tuples or multi-return groups
- arbitrary user-defined accumulators

A growable result uses the same allocation, failure, ownership and target support as an ordinary growable collection built with repeated `push`.

## Closed receiving sites

Producing loops use the same closed-receiver philosophy as value-producing `if`.

The v1 form is accepted where exactly one runtime value is being received:

- a runtime declaration
- a single-value assignment
- a return with exactly one receiving slot

Examples:

```moth
values = loop items |item|:
    then item
;
```

```moth
values {Int} = loop 0 to 10 |number|:
    then number
;
```

```moth
positive |items {Int}| -> {Int}:
    return loop items |item|:
        if item > 0:
            then item
        ;
    ;
;
```

The loop does not become a general expression.

It remains invalid directly inside:

- function arguments
- operator operands
- constructor arguments
- collection or map literals
- template interpolation
- expression statements
- a multi-bind receiver
- a multi-slot return receiver

Nested value-producing syntax may use its own closed receiver inside the loop body. That does not make the outer producing loop a general expression.

Compile-time constant receivers are outside the initial experiment. Do not make `#` constant evaluation own collection-producing loop execution in this plan.

## Element typing and inference

For an explicit `{T}` receiver, each `then` expression is parsed and checked with `T` as its receiving element type.

```moth
values {Float} = loop items |item|:
    then item.ratio
;
```

The result is `{Float}`. The `then` expression is checked through the ordinary contextual-coercion owner.

For an inferred declaration:

```moth
values = loop items |item|:
    then item.name
;
```

the compiler infers the element type from every frontend-reachable `then` contribution and constructs `{String}`.

Rules:

- every producing `then` carries exactly one value
- all inferred contributions must resolve to one compatible element type under the existing inferred-receiver rules
- an explicit receiver supplies the one element type before the body is checked
- a fixed collection receiver is rejected in v1
- a non-collection receiver is rejected with a focused producing-loop diagnostic
- at least one frontend-reachable `then` must exist in the producing loop
- `none` or another context-dependent value may use an explicit optional element type, but it does not gain new inference rules

Both branches of an ordinary compile-time-known `if` remain frontend-valid before static selection. Producing-loop type inference follows the same Stage 4 rule. Inactive code contributes no HIR or runtime collection mutation after specialization.

Do not infer from rendered names, diagnostic `DataType` spelling or backend shapes. Use canonical `TypeId` semantics.

## `then` means one contribution and ends the iteration path

Inside a producing loop:

```moth
then value
```

means:

1. evaluate `value`
2. append it to the hidden growable result through ordinary collection-push semantics
3. finish the current dynamic iteration
4. continue with the next iteration check or step

`then` is terminal for that iteration path.

It is not an `emit` statement that resumes at the following source statement.

This guarantees that one dynamic iteration contributes zero or one result element.

Fallthrough contributes no element.

`continue` contributes no element.

`break` returns the collection accumulated so far.

`return` and `return!` keep their existing function-exit meaning.

A trap keeps its existing meaning.

Several authored `then` statements may exist on mutually exclusive control-flow paths:

```moth
values = loop items |item|:
    if first_case(item):
        then 1
    ;

    if second_case(item):
        then 2
    ;
;
```

No executed path can contribute both. Reaching the first `then` ends that iteration before the second statement can run.

Do not add repeated emission, `yield`-style continuation or a second contribution syntax.

## `then` target boundaries

A producing loop installs its own `then` target for its body.

Normal branch and match-arm child scopes may inherit that target.

An ordinary nested loop is a barrier.

This is invalid because the inner loop is not itself producing a value for a receiver:

```moth
values = loop outer |item|:
    loop item.children |child|:
        then child
    ;
;
```

The inner ordinary loop must not let `then` escape nonlocally to the outer producing loop.

A nested producing loop gets its own target:

```moth
groups = loop rows |row|:
    selected = loop row.values |value|:
        if keep(value):
            then value
        ;
    ;

    then selected
;
```

Functions, templates, conditions and independent value-producing handlers keep their existing target barriers.

This rule is important for source readability and for HIR lowering. `then` always has one obvious nearest active production owner.

## Repeated option-capture loop syntax

The second feature adds a statement-loop header that reevaluates an optional expression:

```moth
loop ~cursor.next() is |item|:
    process(item)
;
```

The `~` above is ordinary exclusive access required by the method call. It is not part of the loop syntax.

The general form is:

```moth
loop expression is |value|:
    body
;
```

`expression` must resolve to `T?`.

Each attempted iteration does this in order:

1. evaluate `expression` exactly once
2. if the result is `none`, exit the loop
3. if the result is present, bind its payload to immutable body-local `value`
4. execute the body
5. on normal body completion, reevaluate the header expression for the next attempted iteration

This is a repeated option capture, not an iterator protocol.

Any expression with type `T?` may be used:

```moth
loop parser.next_token() is |token|:
    inspect(token)
;
```

```moth
loop queue.pop() is |job|:
    run(job)
;
```

```moth
loop read_message() is |message|:
    handle(message)
;
```

No name such as `next` receives compiler meaning.

The initial form has exactly one capture binding.

Do not add an automatic iteration index. Authors can keep an ordinary counter when they need one.

Do not generalize this syntax to arbitrary choice patterns in this plan. The termination rule is specifically `none` versus a present `T`.

## Repeated evaluation and loop control

For a repeated option-capture loop:

- normal body completion reevaluates the option expression
- `continue` immediately reevaluates the option expression
- `break` exits without another evaluation
- `return` and `return!` leave the function
- the capture binding exists only in the body
- the capture binding is immutable
- the next header evaluation happens only after the previous body path has finished

The expression is not evaluated once before the loop and cached.

It is not evaluated twice for one attempted iteration.

Source side effects therefore remain predictable.

A present value does not imply consumption, mutation or freshness. Those properties come from the expression and its normal call or value semantics.

## Repeated option capture is statement-only in v1

The shared statement/template loop-header owner must remain shared, but repeated option capture does not automatically become a template-loop feature.

Template loop parsing should either reject the new header through the shared classified result or use another narrow capability check without duplicating the header grammar.

Do not add repeated side-effecting option evaluation to TIR or template folding as part of this plan.

## Composition

The two features are orthogonal and may be combined:

```moth
tokens = loop ~scanner.next_token() is |token|:
    if token.is_significant():
        then token
    ;
;
```

This means:

- reevaluate `scanner.next_token()` once per attempted iteration
- exit on `none`
- bind a present token
- append significant tokens to the hidden growable result
- contribute at most one token per iteration

This composition is a required acceptance case.

## Conceptual lowering only

A producing loop is conceptually equivalent to a hidden accumulator:

```text
create hidden growable result
enter ordinary loop CFG

on `then value`:
    evaluate value
    call ordinary growable collection push
    continue the current loop

on loop exit:
    load hidden result
    deliver it to the closed receiver
```

A repeated option-capture loop is conceptually equivalent to:

```text
header:
    evaluate option expression once
    branch on none versus present
none:
    exit
present:
    extract payload into body-local binding
    run body
    jump back to header
```

These are semantic explanations, not source-to-source rewrites.

The compiler should not synthesize fake Moth declarations, fake `push` AST calls or a nested fake `if`, then feed that source-shaped tree back through semantic analysis.

AST owns the new source semantics once. AST-to-HIR lowering emits the existing lower-level operations directly.

# HIR and analysis contract

## No new durable HIR construct

This plan adds no durable HIR kind for:

- producing loops
- loop results
- iterator values
- cursor loops
- generators
- producers
- suspension
- collection comprehensions

After AST-to-HIR lowering, the new source forms are represented by existing concepts:

- hidden locals
- `HirExpressionKind::Collection` or the then-current ordinary empty growable collection construction
- ordinary external or builtin collection-push calls
- ordinary call and expression lowering
- ordinary option/variant tests and payload extraction
- `HirTerminator::If`
- ordinary jumps
- the existing `Break` and `Continue` terminators or their then-current normal equivalents
- existing loop CFG blocks

If the exact HIR vocabulary changes before activation, preserve this boundary rather than preserving names from this plan.

## AST representation

It is acceptable and expected for AST to retain explicit source meaning.

On the current architecture, the natural direction is to keep producing loops inside the closed `ExpressionKind::ValueBlock` family with a loop-specific AST payload.

Do not lock the final Rust enum name now.

The important contracts are:

- a producing loop is represented as closed value-producing control flow
- the AST payload retains the ordinary loop source, typed result collection identity and body
- repeated option capture is represented as one typed loop-header form
- `then` inside the producing body has one active collection-production target
- all AST finalization walkers visit the new value-block payload
- no AST form survives as a special HIR semantic

When activated, audit every exhaustive `ValueBlock` and loop-header consumer rather than fixing compiler errors one at a time.

At minimum inspect:

- type validation
- AST normalization
- static Bool specialization
- generated-request retention and discard
- const fact collection
- reactive or Wiring annotation
- assertion-message effect classification
- debug type validation
- const classification
- body-local value scanning
- HIR expression current-block classification

## Closed receiver parsing

Do not make general expression parsing accept `loop`.

Extend the existing closed receiver dispatcher so it recognizes `loop` beside the existing value-producing control-flow forms.

Keep one receiver owner for:

- declaration expected type
- assignment expected type
- return expected type
- inferred declaration behavior
- focused receiver diagnostics

A known receiver of `{T}` activates an element target of `T`.

An inferred declaration activates one single inferred element slot, then wraps the unified element type in the growable collection type.

Reject unsupported receiver shapes before they can become HIR invariants.

## Producing-loop body parsing

The current `ScopeContext` deliberately clears outer value targets when entering an ordinary loop. Preserve that barrier.

A producing loop should create its ordinary loop body context, then explicitly install its own collection-production target on that body.

Branch and match-arm descendants inherit it through the existing target propagation rule.

Do not globally change loops to inherit an outer `active_value_target`.

That would make `then` inside nested ordinary loops escape to unrelated receivers and would violate the accepted source rule.

## Production completeness

Scalar value-producing `if` and match require every non-terminating path to produce because they must deliver one scalar result.

A producing loop has a different contract:

- fallthrough is valid and means no contribution for that iteration
- `continue` is valid and means no contribution
- `break` is valid and returns the partial collection
- `then` produces one element and ends that iteration path
- at least one frontend-reachable `then` must exist somewhere in the producing body

Do not reuse scalar value-block completeness checks that reject fallthrough.

Reuse the existing branch-exit and reachable-`then` walkers where their facts match, and add the smallest loop-specific validation where the scalar contract differs.

## HIR accumulator placement

Allocate the hidden result collection in the parent receiving region, not in the loop body child region.

The result must remain valid after the loop exits.

Initialize it once before entering the loop CFG.

Treat the hidden local as compiler scaffolding in source mapping.

The produced value and the compiler-generated collection-push operation should retain enough authored span information for borrow, target or runtime diagnostics to point at the contributing `then` value when useful.

Generated initialization, merge plumbing and backedges remain compiler-generated spans under the normal HIR source-location policy.

## HIR `then` lowering

The current HIR value-production target sends scalar `then` values into hidden result locals and jumps to a merge block.

Collection production needs a second lowering behavior.

Prefer one small lowering-only target abstraction over parallel booleans or duplicated `ThenValue` dispatch.

Conceptually the active target needs to distinguish:

```text
scalar merge target:
    result locals
    merge block

collection loop target:
    result collection local
    current loop continue target
```

Exact Rust names are implementation detail.

For a collection target, lower `then value` as:

1. lower `value` exactly once
2. perform the same growable collection push operation an authored `~collection.push(value)` would use
3. terminate the path with the current loop's normal continue edge

Do not use the scalar value-block helper that materializes a place into an unconditional copy if ordinary collection `push` would retain or transfer it differently.

Collection-producing `then` follows ordinary collection insertion semantics.

## Reuse loop CFG owners

Current HIR loop lowering already owns reusable body-emitter paths for statement and template loops.

Keep one CFG owner per ordinary loop form.

Producing-loop lowering should reuse those loop CFG builders with a body emitter that installs the collection-production target.

Do not clone conditional, collection or range loop lowering to create "value loop" variants.

Repeated option capture may need one additional loop-header CFG helper, but its blocks should still use the same loop-target stack and body-lowering conventions as the existing loop forms.

## Repeated option-capture parsing

The existing loop-header classifier treats a trailing `|...|` as collection-loop binding syntax.

The new `expression is |name|` form must be recognized before that generic pipe-suffix collection path can misclassify it.

Do not add an ad hoc source-text scan.

Reuse the existing nesting-aware token cursor and the option-capture parsing owner used by `if ... is |name|`.

The parser must:

- identify one top-level `is |name|` option-capture shape
- parse the expression before `is` once
- require its semantic type to be `T?`
- create one immutable body-local capture of `T`
- preserve existing collection-loop `source |item|` grammar
- reject malformed or multi-name capture syntax with structured diagnostics
- leave ordinary `is` expressions inside nested calls or expressions untouched

Do not duplicate option pattern parsing.

## Repeated option-capture HIR lowering

Lower the header expression in the loop header block so it is reevaluated once per attempted iteration.

Use the existing option representation and payload-extraction owner.

Do not lower the source expression before entering the loop.

The generated CFG should have these semantic edges:

```text
entry -> header

header:
    evaluate T?
    none -> exit
    present -> body

body:
    normal completion -> header
    continue -> header
    break -> exit
```

If the existing option-match lowering can expose a focused helper for present-test and payload extraction without dragging in full match-arm machinery, reuse it.

Do not create a second option ABI or reconstruct variant layout in loop lowering.

## Ownership and memory semantics

Producing loops add no ownership model.

The hidden result is an ordinary fresh growable collection.

Each `then` contribution creates exactly the same semantic retention and mutation effects as ordinary insertion into that collection.

Consequences:

- there is no implicit `copy`
- aliases inserted into the result remain aliases under ordinary Moth rules
- inferred transfer may occur only where the existing borrow and lifetime analyses prove it
- retained edges are ordinary collection retained edges
- cleanup frontiers and lifetime topology use existing collection rules
- a result that escapes the loop body is owned by the receiving scope or the destination selected by normal lifetime analysis
- early `return`, propagation or trap cleanup follows the ordinary hidden-local rules
- a source collection or cursor may not be mutated in a way that violates existing live borrows merely because the mutation came from loop syntax

Repeated option capture also adds no borrow exemption.

A captured payload may be fresh, copied or aliasing according to the producing expression.

If a later reevaluation needs exclusive access to the same state, ordinary last-use and borrow validation must prove the previous payload borrow is dead or reject the program.

Do not add cursor-specific lifetime rules.

## Declared regions

When declared regions are implemented by the time this plan activates, a producing-loop result must follow the same destination-lifetime rules as any other fresh growable collection delivered to that receiver.

Do not create the hidden accumulator inside a short loop-body region and later promote it as a special case.

If the receiver has an explicit destination region, normal destination allocation and transfer rules should place or transfer the finished collection through that existing mechanism.

Update canonical declared-region documentation only if its current wording assumes every loop-produced aggregate is authored through explicit mutation.

## Link facts and target validation

A collection-producing loop naturally introduces the same reachable growable-push requirement as an equivalent manual accumulator loop.

Per-function link facts should discover that requirement from ordinary HIR calls.

Do not add a `producing_loop` capability flag.

Repeated option capture introduces no target capability beyond the operations already used by:

- its header expression
- ordinary option inspection
- its body

A backend that cannot lower growable collections or growable `push` should reject a producing loop through the same target-validation contract as equivalent explicit Moth code.

Do not add backend syntax-specific rejection.

## Backend lowering

Backends receive only ordinary validated HIR.

No JavaScript, Wasm or future backend should contain a branch for "collection-producing loop" or "repeated option-capture loop".

Backend tests may prove emitted behavior, but backend implementation should remain unchanged unless an existing ordinary HIR operation is itself missing.

Do not lower producing loops to JavaScript generators, native iterator APIs or backend-specific comprehension forms.

## Capacity reservation

Do not add source-visible capacity syntax or a producing-loop HIR capacity contract in v1.

Collection and finite range loops may expose useful upper bounds for a future allocation optimization, but reserving that capacity is not part of source semantics.

Add reservation only in a later profiling-backed optimization if the ordinary collection/runtime owner has a clean target-neutral capacity path.

Repeated option-capture and conditional loops have no general static result-size bound.

# Diagnostics

Use existing structured diagnostic families where they already own the failure.

Add focused reason variants instead of overloading unrelated value-`if` wording.

The final implementation should cover at least these failures:

- a producing loop used outside an accepted closed receiver
- a known receiver that is not one growable collection
- a fixed collection receiver in the initial feature
- an inferred producing loop with no reachable `then`
- a producing loop with no `then` at all even when the receiver type is known
- `then` with zero values
- `then` with more than one value
- incompatible inferred element types
- a produced value incompatible with explicit element type `T`
- `then` inside a nested ordinary loop where no inner production target exists
- repeated option-capture source that is not `T?`
- malformed `is |name|` syntax
- zero or more than one repeated option capture binding
- capture-name collision or invalid identifier
- repeated option capture used in a template loop when that surface remains statement-only

Keep diagnostic payloads semantic.

Do not encode rendered type names, source text or backend helper names into the payload.

When an existing stable diagnostic code already represents the same semantic family, add a new reason under it rather than manufacturing a new top-level code.

# Non-goals

- no first-class iterator type
- no `Iterator` trait or iterable protocol
- no associated `Item` type
- no iterator adapters such as `.map`, `.filter` or `.collect`
- no closure syntax
- no general function values
- no generator functions
- no generator frames
- no generator `yield`
- no lazy collection-producing loops
- no coroutine or async implementation
- no change to async `yield`
- no arbitrary choice-pattern repeated loops
- no map-producing loop result
- no fixed-collection-producing loop result
- no string-producing loop result
- no user-defined accumulator protocol
- no multiple output values per dynamic iteration
- no `then` that resumes after contribution
- no nonlocal `then` through nested ordinary loops
- no general loop expression syntax
- no producing loop inside arbitrary expression positions
- no multi-bind producing loop
- no compile-time constant collection-producing loops in the initial experiment
- no repeated option-capture template loops in the initial experiment
- no automatic index binding for repeated option capture
- no new HIR loop-result, iterator, generator or suspension kind
- no backend-specific comprehension lowering
- no source-visible capacity hint
- no automatic collection-capacity reservation requirement
- no change to ordinary collection-loop captured-length semantics
- no labelled break or continue

# Implementation rules

- Read the active worktree before using any path or enum name from this queued plan.
- Preserve one loop-header grammar owner.
- Preserve one option-present capture parser.
- Preserve the closed-receiver boundary for value-producing control flow.
- Keep ordinary nested loops as `then` barriers.
- Use one typed result element identity, not rendered type spelling.
- Keep fallthrough valid inside producing loops.
- Keep `then` terminal for its dynamic iteration.
- Lower collection insertion through the ordinary growable-push operation.
- Lower repeated option capture through the ordinary option representation.
- Do not manufacture source-shaped AST for hidden accumulator code.
- Do not add an IR form that exists only to remember source sugar.
- Keep generated HIR scaffolding spanless except where authored contribution or header spans improve diagnostics under existing policy.
- Do not special-case borrow or lifetime validation for the new syntax.
- Do not edit `docs/release/**` by hand.
- Keep tests outside production implementation files.
- Prefer integration cases for source behavior and focused unit tests for parser, AST and HIR invariants.
- Delete compatibility code rather than retaining old and new loop semantics in parallel.

Before each code-bearing phase is accepted:

1. re-read the affected authorities
2. inspect the diff read-only for duplicate paths, stale names and accidental abstraction growth
3. run focused unit and integration tests
4. run the integration test audit
5. run `just validate`
6. run `git diff --check`
7. commit one coherent phase checkpoint

# Phase 0 - Activation audit and implementation baseline

## Goal

Re-anchor this low-priority design to the compiler that exists when the plan finally activates.

## Work

- [ ] Read every required authority from the active worktree.
- [ ] Record branch, HEAD, status and active worktrees in untracked working notes.
- [ ] Confirm the hard prerequisites are delivered.
- [ ] Confirm `yield` still belongs only to async suspension design.
- [ ] Inventory every current statement and template loop-header consumer.
- [ ] Inventory the closed receiver dispatcher for declarations, assignments, multi-bind and returns.
- [ ] Inventory `ValueBlock`, `ThenValue`, active value-target state and all exhaustive value-block walkers.
- [ ] Inventory current conditional, collection and range HIR CFG owners.
- [ ] Inventory option-present capture parsing and option payload extraction in HIR.
- [ ] Inventory growable collection construction, growable push and target capability metadata.
- [ ] Inventory loop-sensitive borrow tests and collection retention/lifetime tests.
- [ ] Inventory integration fixtures for loops, value-producing control flow, options and collection mutation.
- [ ] Confirm whether any current feature added after this plan was written now overlaps the proposed syntax.
- [ ] Remove stale implementation instructions from this plan if architecture has changed, while preserving the accepted source and stage-boundary contracts.

Suggested searches:

```bash
rg -n 'ParsedLoopHeader|RangeLoop|CollectionLoop|WhileLoop|loop_headers' src/compiler_frontend
rg -n 'ValueBlock|ThenValue|active_value_target|ActiveValueProductionTarget' src/compiler_frontend
rg -n 'active_value_block_target|ValueBlockTarget|lower_.*loop' src/compiler_frontend/hir
rg -n 'OptionPresentCapture|is \|.*\||parse_scrutinee_until_is' src/compiler_frontend
rg -n 'CollectionPushGrowable|PushGrowable|CollectionLength' src
rg -n 'loop .*is \||then .*loop|value-producing' tests/cases src/compiler_frontend
```

## Validation

Run the current focused equivalents plus the full gate:

```bash
cargo fmt --all -- --check
cargo test --workspace --quiet loop -- --format terse
cargo test --workspace --quiet value_production -- --format terse
cargo run --quiet -- tests --audit
just validate
git diff --check
```

- [ ] Record unrelated baseline failures without weakening the final gate.
- [ ] Commit only if Phase 0 needs factual plan or documentation corrections.

# Phase 1 - Collection-producing loop AST semantics

## Goal

Make `loop` a valid single-value closed receiver that produces `{T}` through terminal `then`, while preserving all existing statement-loop syntax.

## Receiver work

- [ ] Extend the closed value-control-flow receiver dispatcher to recognize `loop`.
- [ ] Keep general expression parsing unaware of value loops.
- [ ] Accept runtime declaration, single assignment and single return receivers.
- [ ] Reject multi-bind, multi-slot return and arbitrary expression positions.
- [ ] Reject compile-time constant receivers for the initial experiment.
- [ ] For explicit `{T}`, derive `T` once and install it as the produced element target.
- [ ] For inferred declarations, parse single produced values then unify every reachable contribution.
- [ ] Construct the final result type as growable `{T}` through the canonical type environment.
- [ ] Reject fixed collection and non-collection known receivers with focused diagnostics.

## AST work

- [ ] Add one AST-owned producing-loop value-block shape, using the current `ValueBlock` family if that remains the closed-control-flow owner.
- [ ] Reuse the existing `ParsedLoopHeader` result rather than reparsing the loop header.
- [ ] Retain the ordinary loop source and body without creating fake accumulator statements.
- [ ] Create the ordinary loop body scope.
- [ ] Explicitly install the new collection-production target on that body only.
- [ ] Preserve existing target propagation through branch and match-arm children.
- [ ] Preserve the ordinary nested-loop barrier.
- [ ] Require exactly one expression after each producing-loop `then`.
- [ ] Require at least one frontend-reachable producing `then`.
- [ ] Allow body fallthrough, `continue`, `break`, return and propagation without scalar completeness errors.
- [ ] Reuse reachable-`then` traversal and result-type inference where their contracts match.
- [ ] Add a small loop-specific production validator rather than weakening scalar value-if completeness.
- [ ] Ensure mutually exclusive `then` branches are accepted.
- [ ] Ensure a `then` path is terminal so later statements on that dynamic path cannot contribute again.

## Finalization work

Audit and extend every current `ValueBlock` consumer.

At minimum verify:

- [ ] type validation
- [ ] AST normalization
- [ ] static Bool specialization
- [ ] provisional generic-request discard
- [ ] const classification
- [ ] body-local constant fact collection
- [ ] Wiring or reactive annotation
- [ ] assertion-message effect classification
- [ ] debug type validation
- [ ] any remap, clone or traversal code added since this plan was written

Known Bool branches inside a producing loop must still be fully frontend-validated before selection. Inactive executable work must not reach HIR.

## Focused coverage

Parser and AST tests should cover:

- [ ] inferred collection result
- [ ] explicit `{T}` result
- [ ] conditional loop production
- [ ] collection loop production
- [ ] range loop production
- [ ] filter shape with fallthrough
- [ ] several mutually exclusive `then` paths
- [ ] incompatible inferred element types
- [ ] explicit element coercion
- [ ] no producing path
- [ ] zero and multiple values after `then`
- [ ] fixed collection receiver rejection
- [ ] non-collection receiver rejection
- [ ] general-expression-position rejection
- [ ] multi-bind rejection
- [ ] nested ordinary loop target barrier
- [ ] nested producing loop target replacement
- [ ] known Bool specialization inside the body
- [ ] generic requests in inactive branches remain discarded

## Validation

```bash
cargo fmt --all
cargo test --workspace --quiet value_production -- --format terse
cargo test --workspace --quiet loop -- --format terse
cargo run --quiet -- tests --audit
just validate
git diff --check
```

# Phase 2 - Collection-producing loop HIR lowering

## Goal

Erase producing-loop source meaning into ordinary HIR collection state and loop CFG.

## Result construction

- [ ] Allocate one hidden local with the final growable `{T}` type in the parent receiving region.
- [ ] Initialize it once to the ordinary empty growable collection representation before loop entry.
- [ ] Keep the hidden local compiler-owned and out of source name lookup.
- [ ] Return a normal load of the finished collection at loop exit.
- [ ] Route that value through the existing closed receiver path.

## Production target

- [ ] Generalize the lowering-only active `then` target just enough to distinguish scalar merge production from collection-loop production.
- [ ] Avoid parallel flags or a second duplicate `ThenValue` dispatcher.
- [ ] Record or recover the producing loop's own continue target explicitly.
- [ ] Restore the outer active target after nested value control flow completes.

## `then` lowering

For a collection-production target:

- [ ] lower the produced expression exactly once
- [ ] emit the ordinary growable collection-push call against the hidden result local
- [ ] preserve ordinary collection insertion alias and transfer semantics
- [ ] do not apply scalar value-block unconditional copy materialization
- [ ] emit the normal continue edge for the producing loop
- [ ] carry useful authored location from the `then` value under existing HIR location policy

## Loop CFG reuse

- [ ] Reuse the existing conditional-loop body emitter.
- [ ] Reuse the existing collection-loop body emitter.
- [ ] Reuse the existing range-loop body emitter.
- [ ] Do not clone any full loop CFG implementation for a producing variant.
- [ ] Preserve range step behavior.
- [ ] Preserve collection captured-length behavior.
- [ ] Preserve condition reevaluation behavior.
- [ ] Preserve existing `break` and `continue` target stacks.

## HIR invariants

Add focused HIR tests proving:

- [ ] no producing-loop HIR enum variant exists
- [ ] hidden result local is outside the loop body region
- [ ] initialization happens once before entry
- [ ] `then` emits ordinary growable push
- [ ] `then` targets the same next-iteration edge as ordinary `continue`
- [ ] fallthrough reaches the same next-iteration edge without push
- [ ] explicit `continue` emits no push
- [ ] `break` reaches loop exit then exposes the partial collection
- [ ] nested producing loops use distinct accumulators and continue targets
- [ ] range production still performs checked stepping
- [ ] collection production still captures source and source length once
- [ ] scalar value-producing `if` lowering remains unchanged

## Validation

```bash
cargo fmt --all
cargo test --workspace --quiet hir -- --format terse
cargo test --workspace --quiet loop_lowering -- --format terse
cargo run --quiet -- tests --audit
just validate
git diff --check
```

# Phase 3 - Repeated option-capture loop source

## Goal

Add `loop expression is |value|:` as a small statement-loop header without introducing an iterator protocol.

## Header classification

- [ ] Extend the one shared loop-header classifier with the repeated option-capture shape.
- [ ] Recognize top-level `is |name|` before trailing-pipe collection-loop classification.
- [ ] Keep range detection unchanged.
- [ ] Keep existing collection `source |item|` syntax unchanged.
- [ ] Keep ordinary collection-typed unbound headers unchanged.
- [ ] Keep ordinary Bool conditional headers unchanged.
- [ ] Reuse nesting-aware cursor scans.
- [ ] Reuse the shared option-present capture parser and identifier validation.
- [ ] Do not duplicate `if` option-pattern grammar.
- [ ] Require exactly one capture name.
- [ ] Require the source expression to resolve to `T?`.
- [ ] Bind the present payload as immutable body-local `T`.
- [ ] Reject arbitrary choice patterns and `none` pattern spelling in this loop form.

## Statement/template boundary

- [ ] Accept repeated option capture in statement loops.
- [ ] Keep template loops on their existing source forms.
- [ ] Route a repeated option-capture template header to a focused unsupported-source diagnostic without cloning the loop grammar.
- [ ] Add no TIR representation for the new header.

## AST shape

- [ ] Add one typed loop-header result carrying the option expression and capture fact.
- [ ] Retain source spans needed for header and capture diagnostics.
- [ ] Do not represent a user-visible iterator or cursor value unless the source expression itself has such a nominal type.
- [ ] Keep the feature independent of method name.

## HIR lowering

- [ ] Create a header block that reevaluates the option expression.
- [ ] Lower the expression once per attempted iteration.
- [ ] Reuse ordinary option variant test and present-payload extraction.
- [ ] Branch `none` to loop exit.
- [ ] Bind the present payload in the body region.
- [ ] Branch present to the body.
- [ ] Route normal body completion back to the header.
- [ ] Route `continue` back to the header.
- [ ] Route `break` to exit without another header evaluation.
- [ ] Keep generated branch plumbing out of backend-specific code.
- [ ] Add no repeated-option HIR kind.

## Focused coverage

- [ ] present then none
- [ ] initially none
- [ ] repeated side-effect count proves exactly one header evaluation per attempt
- [ ] normal body completion reevaluates
- [ ] `continue` reevaluates once
- [ ] `break` does not reevaluate
- [ ] capture scope ends at loop body
- [ ] capture is immutable
- [ ] non-option source rejection
- [ ] malformed capture rejection
- [ ] multiple capture names rejection
- [ ] existing collection `|item|` parsing remains unchanged
- [ ] nested `is` inside calls does not alter loop classification
- [ ] template-loop rejection remains focused

## Validation

```bash
cargo fmt --all
cargo test --workspace --quiet loop -- --format terse
cargo test --workspace --quiet option -- --format terse
cargo run --quiet -- tests --audit
just validate
git diff --check
```

# Phase 4 - Composition, ownership and end-to-end behavior

## Goal

Prove that both features behave like their ordinary Moth equivalents across control flow, borrowing, lifetime analysis, linking and target validation.

## Composition coverage

Add a primary integration case for:

```moth
tokens = loop ~scanner.next_token() is |token|:
    if token.is_significant():
        then token
    ;
;
```

The case must prove both repeated option evaluation and eager collection production.

Also cover:

- [ ] map-style collection transformation
- [ ] filter-style collection production
- [ ] filter-map style option processing
- [ ] range transformation
- [ ] conditional producing loop
- [ ] early `break` returning a partial result
- [ ] explicit `continue` producing nothing
- [ ] nested producing loops
- [ ] producing loop returned from a function
- [ ] producing loop assigned to an existing compatible runtime receiver
- [ ] source evaluation order
- [ ] produced expression evaluation order

## Borrow and lifetime coverage

Compare against equivalent explicit accumulator code.

- [ ] inserting an alias has the same retention facts as ordinary `push`
- [ ] no implicit copy appears only because syntax used `then`
- [ ] hidden accumulator survives the body region
- [ ] result escape uses ordinary destination lifetime ownership
- [ ] early function exit cleans unfinished accumulator state normally
- [ ] iterating one collection while building a separate result obeys existing source borrow rules
- [ ] illegal source mutation during collection iteration stays illegal
- [ ] repeated option payload used only in the body can die before the next header reevaluation
- [ ] a retained payload that conflicts with the next exclusive source access receives the ordinary borrow or topology diagnostic
- [ ] no cursor-specific exemption appears in borrow or lifetime code

If declared regions are implemented:

- [ ] explicit destination-region result placement uses the ordinary allocation/transfer owner
- [ ] hidden accumulator is never promoted laterally from a loop-body region
- [ ] retained-edge facts from pushed values remain ordinary collection facts

## Reachability and target coverage

- [ ] producing loops add the ordinary growable-push link requirement
- [ ] repeated option capture adds no new capability bit
- [ ] unsupported targets reject through ordinary collection or option capability validation
- [ ] supported targets produce the same observable result as explicit accumulator code
- [ ] no backend has source-syntax branches for either feature

When multiple physical backends support growable collections at activation, use one integration input with backend-specific assertions rather than duplicated fixtures.

## Static specialization and generated work

- [ ] known Bool branches inside producing bodies are checked before selection
- [ ] inactive branches publish no push, call or generated-function link facts
- [ ] generic calls in inactive branches do not materialize sidecars
- [ ] an inferred element type remains stable when a statically inactive branch was required for frontend type validation
- [ ] repeated option header generic calls materialize normally and only once per source call site

## Integration-test ownership

Follow the current test-suite contract.

Prefer a small set of strong cases:

1. one primary producing-loop semantics case
2. one primary repeated-option-capture case
3. one composition case
4. focused diagnostic cases only where the failure is not already proven by parser unit coverage
5. backend-specific assertions only for distinct target behavior

Do not create one fixture per syntax spelling when a single realistic case proves several observable rules.

## Validation

```bash
cargo fmt --all
cargo run --quiet -- tests --audit
cargo run --quiet -- tests --terse
just validate
git diff --check
```

# Phase 5 - Canonical documentation and closeout

## Goal

Make the feature easy to learn, make its architectural boundary permanent and retire the plan.

## Compiler architecture documentation

Update `docs/compiler-design-overview.md`.

Under Stage 4 value-producing control flow, document:

- [ ] producing loops are closed receivers that produce one growable `{T}`
- [ ] `then` contributes one element and terminates the current iteration path
- [ ] loop fallthrough contributes no element
- [ ] nested ordinary loops are production barriers
- [ ] repeated option capture is a statement-loop source over repeated `T?` evaluation
- [ ] AST owns typing, target selection and capture semantics
- [ ] producing and repeated-option loop source meaning disappears during HIR lowering

Under Stage 5 HIR, document:

- [ ] both features lower to ordinary locals, collection calls, option tests and CFG
- [ ] no iterator, generator or suspension semantic enters HIR
- [ ] backend lowerers do not reconstruct either source form

Keep the design text compact and reference the general stage rule instead of repeating lowering details in several sections.

## Language documentation

Update the loop documentation so the final source model is visible in one place.

Expected changes:

- [ ] add a Basic and Advanced "Collection-producing loops" section under `docs/src/docs/loops/`
- [ ] add a Basic and Advanced or appropriately routed "Repeated option capture" section under `docs/src/docs/loops/`
- [ ] update `docs/src/docs/loops/@page.moth`
- [ ] update conditional, collection and range loop pages only where their interaction with production needs clarification
- [ ] update `loop-control.mtf` for producing-loop `then`, `continue` and `break`
- [ ] update `value-producing-if.mtf` to distinguish scalar completeness from collection-producing loop fallthrough
- [ ] update `errors/options.mtf` with repeated capture where it helps explain `T?`
- [ ] update the language cheatsheet
- [ ] update the progress matrix
- [ ] verify Basic docs stay concise and Advanced docs carry the full receiver and control-flow contract

Teach the main ergonomic use directly:

```moth
active_names = loop users |user|:
    if user.active:
        then user.name
    ;
;
```

Then show the pull form separately:

```moth
loop ~scanner.next_token() is |token|:
    inspect(token)
;
```

Do not teach a fictional iterator object as the conceptual model.

## Async documentation

Verify `docs/src/docs/async/@page.moth` still reserves `yield` for explicit coroutine suspension.

Only edit it if a short distinction is needed after the new loop docs land.

Do not redefine `yield`, add generator wording or imply collection-producing loops suspend.

## Memory documentation

Review the canonical declared-region, retained-edge and borrow-validation docs for statements that assume all loop-built aggregates are authored through an explicit mutable accumulator.

Update only factual wording affected by the new syntax.

Do not duplicate collection retention rules under a new "producing loop memory model" heading.

## Progress matrix

Record the implemented feature in the existing loop/control-flow rows or split a row only if the matrix is clearer that way.

Report backend support factually.

A target that accepts the source only because it already supports ordinary growable collection push should not be described as having a separate "comprehension backend".

## Final audits

- [ ] Search for duplicate loop-header scans.
- [ ] Search for duplicate option-capture parsers.
- [ ] Search for backend branches on producing-loop source identity.
- [ ] Search for new HIR variants that only preserve source sugar.
- [ ] Search for `yield` used as collection production terminology.
- [ ] Search for iterator or generator types added only for this feature.
- [ ] Search for production-target booleans that should be one small enum or owned state.
- [ ] Search for old diagnostics that still claim `then` only belongs to scalar value blocks.
- [ ] Confirm no template-loop behavior changed accidentally.
- [ ] Confirm no compile-time constant loop behavior changed accidentally.
- [ ] Confirm all new user-facing failures are structured diagnostics.
- [ ] Confirm tests have one primary owner per contract and no redundant fixtures.

Suggested searches:

```bash
rg -n 'ProducingLoop|Iterator|Generator|yield|active_.*target' src docs/src/docs
rg -n 'ParsedLoopHeader|parse_.*loop.*header|TYPE_PARAMETER_BRACKET' src/compiler_frontend/ast
rg -n 'OptionPresentCapture|parse_.*option.*capture|is \|' src/compiler_frontend/ast
rg -n 'CollectionPushGrowable|PushGrowable' src/compiler_frontend src/backends
rg -n 'ValueBlock::|ExpressionKind::ValueBlock' src/compiler_frontend
```

## Final validation

Run the current code-bearing final gate:

```bash
cargo fmt --all
cargo run --quiet -- tests --audit
just validate
git diff --check
```

Then build the documentation through the current release-build path if `just validate` does not already provide the final rendered-doc artifact needed for manual review:

```bash
moth build docs --release
```

or:

```bash
cargo run --quiet -- build docs --release
```

Manually inspect the loop, option and branching documentation pages.

## Completion criteria

This plan is complete only when all of the following are true:

- collection-producing loops work at the accepted single closed receivers
- the result is always one eager growable `{T}`
- inferred element typing considers every frontend-reachable producing path
- explicit `{T}` supplies element receiving context
- `then` contributes exactly one element and terminates that iteration path
- fallthrough and `continue` contribute no element
- `break` returns the partial collection
- nested ordinary loops do not capture an outer producing-loop `then`
- all ordinary statement-loop source forms can produce collections
- repeated option capture reevaluates one `T?` expression exactly once per attempted iteration
- `none` exits and present payload enters the body
- repeated option capture has one immutable binding and no implicit index
- the two features compose
- template loops do not silently gain repeated option capture
- compile-time constant receivers do not silently gain producing loops
- no iterator protocol was added
- no generator or generator `yield` was added
- async `yield` semantics are unchanged
- AST owns every new source semantic
- HIR contains only ordinary existing control-flow and collection/option operations
- borrow and lifetime analyses need no source-syntax exemption
- target validation sees the same requirements as equivalent explicit Moth code
- backends contain no source-form-specific lowering
- diagnostics are structured and receiver-aware
- integration coverage proves the observable contracts
- canonical docs and the progress matrix describe the final behavior
- the full code-bearing validation gate passes

# Closeout

In the completion commit:

1. finish the implementation, tests and canonical documentation
2. update the progress matrix
3. remove this plan's roadmap bullet
4. delete this plan file
5. keep any genuinely deferred optimization or syntax idea only if it has a concrete owner and reason

Do not leave a completed plan in the roadmap.

Possible deferred follow-ups should stay unplanned unless profiling or real source use justifies them:

- result-capacity reservation from known collection or range bounds
- compile-time constant collection-producing loops
- repeated option capture in template loops
- map-producing loops
- fixed-collection-producing loops
- a broader iteration protocol

Generators are not a follow-up to this plan. Any future suspension feature belongs with the async/coroutine design and must solve its own HIR, borrow, lifetime and runtime requirements.
