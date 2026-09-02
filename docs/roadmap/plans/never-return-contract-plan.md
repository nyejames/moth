# Never return contract implementation plan

## Purpose

Add the explicit `-> !` Never return contract across Moth's callable, control-flow, HIR, analysis and backend boundaries before the HTML mixed JavaScript and Wasm backend implementation starts.

`-> !` states that a callable never returns control normally. It is a control-flow contract, not a value type. The implementation must not add a source-visible `Never` type, a Never `TypeId`, a fake `None` result, bottom-type coercion or an expression value that can stand in for another type.

This plan also consolidates the current duplicated terminality logic into one control-flow exit analysis that distinguishes producing a value, returning to the caller and diverging. The same facts must serve ordinary function terminality, value-producing block completeness, declared Never validation and statically infinite loop proof.

## Current-state capsule

```text
ACTIVE_PLAN: docs/roadmap/plans/never-return-contract-plan.md
STATUS: queued
CURRENT_SLICE: Phase 0 - refresh callable, terminality, HIR, analysis and backend owners
BLOCKERS: runtime anonymous records must be delivered first
NEXT_ACTION: activate after runtime anonymous records, record the live repository state and rerun the current owner inventory
```

Keep this block concise. Establish the active revision, branch, worktree state and validation baseline in working notes when implementation starts. Do not pin a queued plan to a commit.

## Roadmap position and prerequisites

This plan runs after runtime anonymous records and immediately before the HTML mixed JavaScript and Wasm backend implementation.

Hard prerequisites:

- runtime anonymous records are delivered and their HIR, borrow and lifetime integration is stable
- canonical module compilation and immutable public semantic interfaces are delivered
- generated concrete functions use sidecars and stable callable identities
- Stage 4 static Bool specialization selects active AST control flow before terminality and durable executable facts
- value-producing `if`, match and catch blocks already distinguish producing paths from terminating paths
- assertion messages and `assert(false)` already lower through explicit failure control flow
- HIR calls use stable local, module-private, cross-module, generated and binding-backed targets
- borrow validation, lifetime-region analysis and per-function link facts consume validated HIR without rewriting it
- JavaScript and the current experimental Wasm backend both lower explicit HIR terminators

Name these delivered capabilities rather than citing the plans that implemented them.

## Required authorities

Read these from the active worktree before implementation and re-read the affected sections during every phase audit:

- `AGENTS.md`
- `docs/compiler-design-overview.md` in full because this work crosses syntax, AST, public interfaces, HIR, borrow validation, lifetime summaries, link facts and backend handoff
- `docs/build-system-design.md` opening authority, architectural invariants, generated-function boundary, entry/package link planning, target validation and HTML mixed-target sections
- `docs/src/developer-docs/language/overview.mtf`
- `docs/src/docs/functions/function-declarations.mtf`
- `docs/src/docs/functions/calls-and-access.mtf`
- `docs/src/docs/functions/returns-and-multiple-values.mtf`
- `docs/src/docs/errors/assertions.mtf`
- `docs/src/docs/branching/value-producing-if.mtf`
- the canonical pattern-matching references when match exit analysis changes
- `docs/src/docs/loops/conditional-loops.mtf`
- `docs/src/docs/loops/loop-control.mtf`
- `docs/src/docs/traits/trait-requirements.mtf`
- `docs/src/docs/traits/conformance.mtf`
- `docs/src/docs/generics/generic-declarations.mtf`
- `docs/src/docs/generics/generic-inference.mtf`
- `docs/src/docs/packages/external-binding-contracts.mtf`
- `docs/src/developer-docs/memory-management/overview.mtf` and its routed borrow, lifetime and external-boundary leaves
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`
- `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf`
- `docs/src/docs/progress/@page.moth`
- `docs/roadmap/roadmap.md`

The permanent language references and compiler architecture become the semantic authorities before this plan is retired. This plan records the implementation sequence and accepted design while work is queued.

## Accepted language contract

### Source syntax

A callable that never returns normally declares one whole return contract:

```moth
fatal |message String| -> !:
    assert(false, message)
;
```

```moth
serve || -> !:
    loop true:
        handle_request()
    ;
;
```

The bare `!` appears immediately after `->`. It is not a suffix on a type and does not reserve a new keyword.

`-> !` is mutually exclusive with every success, optional and error return slot. These forms are invalid:

```moth
bad || -> String, !:
;

bad || -> Error!, !:
;

bad || -> !, String:
;

bad || -> !!:
;
```

Use a targeted function-signature diagnostic at the conflicting token. Do not let `-> !` enter the ordinary return-list parser as a missing type or as `Error!` with an omitted error type.

### Callable surfaces

The Never return contract is available anywhere Moth stores a callable contract:

- ordinary functions
- receiver methods
- generic function templates and generated concrete functions
- trait requirements and conformance matching
- exported functions and receiver methods
- cross-module and future package interfaces
- binding-backed and external functions whose provider metadata explicitly declares the contract

It is an exact callable contract. A required `-> !` method is satisfied only by an implementation whose signature is also `-> !`. A body that happens to diverge does not make an ordinary `-> String` signature compatible with `-> !`, and a `-> !` method does not satisfy an ordinary return contract.

### Never is not a value type

Moth does not add:

- a source-visible `Never` name
- a Never `TypeId`
- a Never literal or value
- variables, parameters, fields, collection elements or generic arguments of Never type
- `Never -> T` compatibility or bottom-type coercion
- a fake `None` or unit value for divergence
- a multi-return slot containing Never

`None` remains the unit-like no-value representation used by ordinary functions that may complete. It has one trivial runtime meaning. A Never return contract has no normal return at all. Do not conflate them.

Each compiler layer should represent the distinction as a callable return-contract enum, not as a boolean attached to an ordinary return vector and not as a sentinel type ID.

### Implementation divergence versus declared Never

An ordinary declared return type remains a valid future-facing API even when its current implementation diverges:

```moth
load_name || -> String:
    assert(false, "not implemented")
;
```

Callers continue to treat `load_name()` as a normal `String` call. Its current body divergence only proves that the function does not fall through without returning its declared value.

An explicit Never contract is stronger:

```moth
fatal |message String| -> !:
    assert(false, message)
;
```

Only this explicit contract tells callers that control cannot continue after the call. Same-file visibility, inlining, constant propagation and whole-program knowledge must not infer this source-validity fact from an ordinary callable's implementation.

### Explicitness for unannotated functions

An unannotated function normally has an ordinary no-success-value contract and may complete at the end of its body.

When its specialised active body is proven to diverge on every reachable path, reject the omitted return contract and require the author to choose explicitly:

- write `-> !` when never returning is the final intended contract
- write the ordinary type the function is expected to return when the current body is only an unfinished placeholder

Example diagnostic intent:

```text
Function `serve` cannot complete normally.
Declare `-> !` when it is intended never to return, or declare the ordinary
return type it is expected to produce when this implementation is temporary.
```

Use a typed diagnostic payload and retain the function declaration location. The final rendered wording may follow current diagnostic conventions, but it must present both choices and clearly prefer `-> !` for permanently divergent behaviour.

An explicitly typed ordinary function may diverge on every current path without this diagnostic.

This explicitness diagnostic applies only to user-authored callable declarations with an omitted return contract. The compiler-synthesised entry `start` keeps its builder-owned ordinary contract and cannot be annotated in source. Root runtime work may still call a Never function and make `start` diverge without manufacturing a source declaration or requiring impossible `start -> !` syntax.

### Declared Never body validation

A function declared `-> !` is valid only when every reachable path in the specialised active AST diverges.

These do not satisfy `-> !`:

- falling off the end of the body
- ordinary `return`, including a no-value return
- `return!`
- postfix error propagation that returns an error to the caller
- postfix option propagation that returns absence to the caller
- any branch that can complete normally
- a loop that can exit through a reachable `break` targeting that loop

Those operations may terminate the current AST path, but they return control to the caller or resume after the loop. They are not divergence.

Validate all authored source normally before static specialization. Name resolution, type checking, generic evidence, return shape and propagation legality still apply inside a branch that later becomes inactive. After normal Stage 4 Bool specialization, only active reachable paths participate in the Never proof. Unreachable tails after a proven non-continuing statement do not invalidate the contract.

### Statement-only Never calls

A call whose declared contract is `-> !` is valid only as a standalone statement:

```moth
fatal("unreachable")
```

It may appear wherever an ordinary executable statement is legal, including nested branches, loops, lexical scopes and normal-root runtime work.

It is invalid in every value-consuming expression position:

```moth
value String = fatal()
return fatal()
consume(fatal())
if fatal():
    io.line("unreachable")
;
label = [: [fatal()]]
items = {fatal()}
```

This restriction applies recursively inside calls, operators, constructors, casts, templates, collections, maps, conditions, assertion arguments and every other expression owner.

A Never call cannot be followed by postfix `!`, postfix `?` or `catch`. It has no success, error or optional value to handle.

The parser and AST should provide a targeted placement diagnostic such as:

```text
`fatal()` never returns and cannot be used as a value.
Call it as a standalone terminating statement instead.
```

Preserve the one shared call-shaped argument parser and parameter-slot routing owner. Do not add a second call grammar for Never calls.

### Value-producing blocks

A standalone Never call may complete one branch of a block-form value producer because that branch diverges instead of producing a value:

```moth
name = if ready:
    then "Priya"
else
    fatal("name unavailable")
;
```

The Never call does not produce a `String` and no coercion occurs. The value block remains valid because one path produces the receiver value and the other cannot reach the receiver.

The inline form remains invalid when it places a Never call in a value slot:

```moth
name = if ready then "Priya" else fatal("name unavailable")
```

Preserve the existing rule that at least one reachable path must produce values. A value-producing block where every branch diverges still reports the existing no-producing-path diagnostic rather than inventing a result type.

## Initial divergence proof boundary

The first implementation proves divergence through exactly these structural sources:

1. An `assert` whose condition has normalised to compile-time `false`.
2. A standalone call whose declared callable contract is `-> !`, except where accepting that call would form an unvalidated recursive proof cycle.
3. A conditional `loop` whose condition has normalised to compile-time `true` and which has no reachable `break` targeting that loop.
4. A lexical scope whose every reachable exit diverges.
5. An `if` with an active `else` where every reachable branch diverges.
6. An exhaustive match where every reachable arm and required default path diverges.
7. Statement sequencing where later statements are considered only on paths that can still fall through.

A true conditional loop may diverge even when its body reaches the end or executes `continue`, because those paths start the next iteration. A path that returns to the caller remains a return path. A reachable break targeting the loop makes the post-loop continuation reachable.

Nested loop control is structural:

- `break` and `continue` target the nearest loop
- a nested loop consumes its own break and continue exits
- a break in a nested loop does not make an outer true loop escapable
- return and divergence facts pass outward through nested loops

Collection and range loops remain conservatively non-divergent because their iteration count is finite or may be zero under the current source contract.

Use the same folded Bool authority and specialised AST fact that ordinary static `if` already uses. Do not add literal-only matchers in several consumers.

### Recursive proof safety

An explicit Never signature is a contract for callers, but a source function must not use an unvalidated cycle of Never signatures as its own proof. The first implementation should validate source Never bodies over the active Never-call graph with these rules:

- a Never call to an already validated provider module or explicit binding-backed contract may prove divergence
- source provider modules are trusted only after their own module compilation has validated and published the contract
- same-module source functions may use an acyclic dependency order
- a self-edge or strongly connected group whose proof depends on one of its own unvalidated Never edges receives a structured deferred-proof diagnostic
- an unreachable recursive edge after independent divergence does not participate in the proof
- no recursive proof failure may become `CompilerError` or silently accept the cycle

Keep this proof bookkeeping compiler-local. Do not expose recursive proof state in public interfaces, HIR types or backend metadata.

## Deliberately deferred proof extensions

The core implementation must leave these accepted proof gaps visible in the progress matrix after the feature lands:

- direct-recursion and mutual-recursion proof for declared Never bodies where validation would otherwise depend circularly on the same unproven contracts
- richer data-flow proof that a non-literal runtime loop condition remains true forever
- broader structural non-termination proof beyond the accepted true-loop, explicit Never-call and branch rules
- whole-program or SCC-based divergence proof where it can remain deterministic and does not change ordinary callable contracts implicitly

These are proof extensions only. They may accept more correctly declared `-> !` bodies in the future.

The following are deliberate final constraints, not deferred gaps:

- divergence never propagates through an ordinary callable contract for source-validity decisions
- external code is non-returning only when its binding contract explicitly says `-> !`
- Never is not a first-class type or value
- Never calls are not value expressions
- bottom coercion is not part of Moth
- `-> !` cannot be mixed with ordinary or error return slots

Do not list those constraints in the roadmap or progress matrix as work expected to land later.

## Control-flow exit model

The current compiler has separate terminality owners and a broad `TERMINATES` fact that groups returning to the caller with unrecoverable failure. Replace that duplication with one AST-owned control-flow exit analysis.

The exact Rust type may vary, but it must preserve independent facts equivalent to:

```rust
pub(crate) struct ControlFlowExitSummary {
    pub(crate) can_fall_through: bool,
    pub(crate) produces_value: bool,
    pub(crate) returns_to_caller: bool,
    pub(crate) diverges: bool,
    pub(crate) breaks_current_loop: bool,
    pub(crate) continues_current_loop: bool,
}
```

Use a better scoped representation if the live AST requires it. Do not compress these facts into a tri-state enum and do not add a pile of unrelated booleans to parser contexts. The analysis owner should provide named queries for each consumer:

- ordinary no-value function may complete normally
- ordinary typed function has no reachable fallthrough
- value-producing branch has no fallthrough and either produces or otherwise exits
- value producer has at least one reachable producing path
- declared Never function has divergence on every reachable path and no return-to-caller path
- true loop has no reachable break targeting itself

Alternative branches union exit possibilities. Statement sequencing applies the next statement only to paths that can still fall through. Loop analysis consumes the nearest loop's break and continue facts at the loop boundary.

Keep diagnostic policy outside the pure summary calculation where practical. One analysis should compute facts, while function terminality and value-production validators select their own typed diagnostics.

## Intended semantic data shape

Use explicit enums at every layer so invalid mixed states are unrepresentable.

### Neutral signature syntax

Conceptual shape:

```rust
pub(crate) enum FunctionReturnContractSyntax {
    Returns(Vec<ReturnSlotSyntax>),
    Never { location: SourceLocation },
}
```

`FunctionSignatureSyntax` carries this contract instead of treating `returns: Vec<_>` as the complete return meaning.

### Resolved AST callable contract

Conceptual shape:

```rust
pub(crate) enum FunctionReturnContract {
    Returns(Vec<ReturnSlot>),
    Never { location: SourceLocation },
}
```

`FunctionSignature` should expose named queries such as `is_never`, `success_returns`, `error_return` and ordinary resolved return IDs without forcing callers to pattern-match raw vectors repeatedly. Queries for ordinary returns must reject or explicitly handle Never rather than silently returning empty vectors that make Never look like unit.

### Public and trait interfaces

Public functions, receiver methods and trait requirements need explicit stable return-contract vocabulary, for example:

```rust
pub(crate) enum PublicCallableReturnContract {
    Returns {
        success: Vec<PublicReturnTypeSlot>,
        error: Option<CanonicalTypeIdentity>,
    },
    Never,
}
```

Trait-local `This` return vocabulary remains scoped to trait requirement surfaces. A trait Never contract carries no `TraitSurfaceTypeIdentity`.

The public-interface fingerprint includes the exact return-contract variant. Changing `-> String` to `-> !` or the reverse is a public semantic change.

### Binding-backed contracts

Binding metadata needs the same distinction, for example:

```rust
pub enum ExternalFunctionReturnContract {
    Returns {
        success: Vec<ExternalReturnSlot>,
        error: Option<ExternalSignatureType>,
    },
    Never,
}
```

Provider annotations may spell `-> !`. A binding marked Never has no success alias metadata and no error slot. Validate the provider definition before publishing it.

### HIR function contracts

Replace `HirFunction::return_type: TypeId` with an explicit function return contract, conceptually:

```rust
pub enum HirFunctionReturnContract {
    Returns(TypeId),
    Never,
}
```

The ordinary variant continues to carry the existing unit, single, tuple or fallible-carrier type. Never carries no type ID and produces no ABI result.

### HIR terminating calls

Add a dedicated terminator, conceptually:

```rust
HirTerminator::NeverCall {
    target: CallTarget,
    args: Vec<HirExpression>,
}
```

Exact naming may follow current HIR conventions. The variant must retain the ordinary stable call target and evaluated arguments. It has no result local, no result type and no successor.

Do not encode this as an ordinary `Call { result: None }` followed by a source-level failure terminator. The call itself owns the no-continuation contract. A backend may lower it to a target call followed by defensive unreachable machinery, but HIR must retain one semantic operation.

## Reviewed current compiler state

Reverify every item at activation. These are navigation facts, not permanent architecture.

### Signature and callable owners

- `src/compiler_frontend/declaration_syntax/signature_members.rs` stores `FunctionSignatureSyntax.returns: Vec<ReturnSlotSyntax>` and parses a type before treating a trailing `!` as the error channel.
- `src/compiler_frontend/ast/statements/functions.rs` stores `FunctionSignature.returns: Vec<ReturnSlot>` with `ReturnChannel::Success` or `ReturnChannel::Error`.
- `src/compiler_frontend/public_interface/model.rs` stores success returns and an optional error return directly on public functions and receiver methods. Trait requirements carry a vector of typed return slots.
- `src/compiler_frontend/traits/definitions.rs` and conformance matching store typed requirement returns and channels.
- `src/compiler_frontend/external_packages/definitions.rs` stores `returns` plus `error_return_type` on external function definitions and specs.
- generated template materialisation, import projection, public interface construction and reactive metadata each inspect the existing return vectors.

### AST calls and terminality

- every `Expression` owns a semantic `TypeId`
- ordinary function, method and host call expressions carry `result_type_ids`
- `src/compiler_frontend/ast/statements/body_expr_stmt.rs` parses standalone calls through the value-expression path and then filters which expressions may stand alone
- `src/compiler_frontend/ast/statements/terminality.rs` owns ordinary function terminality and recognises literal-false assertions
- `src/compiler_frontend/ast/statements/value_production/completeness.rs` separately owns branch exit summaries and separately recognises literal-false assertions
- loops remain conservative in current terminality analysis
- static Bool specialization occurs under `src/compiler_frontend/ast/module_ast/finalization/`

### HIR and analysis

- `HirFunction` currently carries one `return_type: TypeId`
- HIR declaration lowering manufactures unit, tuple and fallible carrier return types from AST return slots
- ordinary calls are `HirStatementKind::Call { target, args, result }`
- `HirTerminator` owns branches, returns, runtime failures and assertion failures but has no call terminator
- HIR validation, display, remapping, reachability, borrow transfer, call summaries, problem extraction and backend validation exhaustively match current terminators
- public call and lifetime summaries assume normal exit/result vocabulary even when a body currently has only failure exits

### Backends

- JavaScript emits ordinary calls inside blocks and emits terminators through the dispatcher path
- the JavaScript backend can reuse ordinary target/argument call lowering, then emit a defensive compiler-owned throw if a Never callee returns
- current Wasm LIR already has a result-less call statement and a `Trap` terminator that emits `unreachable`
- before the larger Wasm redesign, a HIR Never call can lower to a result-less LIR call followed by `Trap`
- the later structured Wasm implementation must preserve the semantic call plus unreachable fallback without reconstructing Never from a missing result type

## Scope

This plan owns:

- `-> !` source syntax and diagnostics
- callable return-contract vocabulary through syntax, AST, public interfaces, traits, generics, external packages and HIR
- exact callable compatibility and fingerprints
- standalone-only Never call parsing and AST representation
- one shared control-flow exit and divergence analysis
- unannotated all-path-divergence diagnostics
- declared Never body validation
- structural true-loop divergence proof with nearest-loop break handling
- HIR Never function contracts and Never-call terminators
- HIR validation, display, remapping and CFG utilities
- borrow, lifetime, reachability, link-fact and generated-sidecar integration
- Boracle problem extraction and operational representation where the HIR terminator affects it
- JavaScript and current Wasm lowering
- binding-backed and annotated external JavaScript Never contracts
- canonical language and compiler documentation
- progress matrix and roadmap status
- downstream Wasm plan prerequisite and lowering contract
- generated documentation rebuild

## Non-goals

Do not add:

- a Never `TypeId`, `DataType`, `ParsedTypeRef` or `CanonicalTypeIdentity`
- a source keyword or named `Never` type
- `None` as a spelling for divergence
- bottom-type subtyping or coercion
- Never expressions, variables, parameters, fields, aliases, options, collections, maps or generic arguments
- expression-position Never calls
- mixed Never and ordinary/error return slots
- implicit no-return effects inferred from ordinary function bodies at call sites
- source-visible panic expressions
- a replacement for the statement-only `assert` intrinsic
- recursion/SCC divergence proof in the first implementation
- arbitrary loop invariant or whole-program termination analysis
- labelled loop control
- backend-specific source syntax
- compatibility shims for old return-vector or HIR return-type APIs

## General implementation rules

- Re-read the current worktree, not a cached repository snapshot.
- Preserve user-authored local changes and classify active worker branches before editing.
- Keep one callable return-contract owner per compiler layer.
- Use enums to make `Returns` versus `Never` explicit. Do not add `is_never: bool` beside ordinary return fields.
- Keep `TypeEnvironment` unchanged by this feature except where generic existing code needs a query. Never must not enter type identity.
- Keep one call-shaped parser and one parameter-slot routing owner.
- Parse and resolve call arguments once, then select value-expression or terminating-statement construction from the resolved callable contract and use context.
- Do not let HIR, borrow analysis, link planning or backends infer Never from an empty return vector, a unit type, a missing result local or an all-failure body.
- Do not let source validity depend on inlining or implementation inspection across an ordinary call boundary.
- Delete replaced fields, vector-only assumptions and duplicate terminality helpers in the same phase that cuts consumers over.
- Use structured `CompilerDiagnostic` values for every malformed signature, invalid call placement and body-contract failure.
- Keep test-only constructors and fixtures under their test owners.
- Prefer integration cases for user-visible behaviour and focused unit tests for hidden contract, exit-summary, HIR and backend invariants.
- Do not edit generated files under `docs/release/**` directly.
- Do not mark the progress row supported until the full source, analysis and backend path is present.

## Mandatory phase completion protocol

Every phase is an accepted checkpoint, not a loose batch of edits. Before the next phase starts:

1. Re-read the affected authority sections and the full style guide.
2. Review every changed module from its `mod.rs` or owning entry point.
3. Run a read-only phase audit focused on architecture ownership, semantic gaps, stale paths, diagnostics and test quality. This is an implementation audit, not a registered audit-framework run unless the user separately requests one.
4. Resolve every actionable audit finding and rerun affected targeted tests.
5. Perform the full `AGENTS.md` Slice review.
6. Run the phase's targeted validation commands.
7. Run `just validate` for every code-bearing phase. Run the documentation release-build gate for a strictly documentation-only phase.
8. Run `just boracle` in every phase that changes HIR terminators, borrow-problem extraction or Boracle semantics because the opt-in lane is not part of `just validate`.
9. Run `git diff --check` and confirm only intended files changed.
10. Record exact results in working notes and commit the phase as one coherent checkpoint.

A phase is not accepted while its mandatory read-only audit is unavailable or has open actionable findings. Record the blocker rather than claiming completion.

# Phase 0 - Refresh current owners and establish the baseline

## Goal

Re-anchor the plan in the active worktree after runtime anonymous records land. Produce a complete owner and test inventory before changing semantic data.

## Implementation checklist

- [ ] Read every required authority from the active worktree.
- [ ] Record `git rev-parse HEAD`, branch, `git status --short` and `git worktree list --porcelain` in untracked working notes.
- [ ] Inventory active workers and local changes touching signatures, AST calls, terminality, HIR, analysis, public interfaces, external packages, JS, Wasm, docs, roadmap or progress.
- [ ] Preserve all user-authored and unrelated changes. Do not reset, stash or reformat them.
- [ ] Reconfirm that runtime anonymous records are complete and the queued Wasm implementation has not started.
- [ ] Inventory every direct `FunctionSignature.returns`, public return-vector, trait return-vector, external `returns/error_return_type` and `HirFunction.return_type` consumer.
- [ ] Inventory every expression-call constructor and every statement-expression entry point.
- [ ] Inventory every terminality, branch-exit, false-assert and loop-exit classifier.
- [ ] Inventory every exhaustive `HirTerminator` match across validation, display, remapping, reachability, borrow analysis, lifetime analysis, Boracle, JS and Wasm.
- [ ] Inventory current integration contracts and primary owners for functions, returns, assertions, loops, value-producing blocks, traits, generics, modules and external JS.
- [ ] Record baseline test counts and any unrelated failures without weakening gates.

Suggested searches:

```bash
rg -n 'FunctionSignatureSyntax|ReturnSlotSyntax|FunctionSignature|ReturnChannel' src tests
rg -n 'PublicFunctionSemantics|PublicReceiverMethodSemantics|PublicTraitRequirement' src tests
rg -n 'ExternalFunctionDef|ExternalFunctionSpec|error_return_type|\.returns\b' src tests
rg -n 'HirFunction|return_type|HirTerminator|HirStatementKind::Call' src tests
rg -n 'BranchExitSummary|terminality|statically_false|assert_condition_is' src tests
rg -n 'ExpressionKind::FunctionCall|ExpressionKind::MethodCall|HostFunctionCall' src tests
rg -n 'DispatcherLoop|WasmLirTerminator::Trap|emit_call_statement' src/backends
```

## Phase 0 audit and style-guide review

- [ ] Confirm the inventory follows semantic ownership rather than directory names alone.
- [ ] Confirm no existing callable return-contract enum already solves part of the task.
- [ ] Confirm no newer static-control-flow analysis supersedes the reviewed terminality paths.
- [ ] Confirm the proposed enum cutover will remove invalid mixed states rather than wrap old vectors.
- [ ] Confirm every current HIR terminator consumer is accounted for.
- [ ] Confirm test ownership and progress-matrix wording reflect the live tree.
- [ ] Perform the mandatory read-only phase audit and Slice review.

## Phase 0 validation and acceptance

Run baseline commands without changing tracked files:

```bash
cargo fmt --all -- --check
cargo test --workspace --quiet -- --format terse
cargo run --quiet -- tests --audit
just boracle
just validate
```

- [ ] Record exact results and counts.
- [ ] Separate pre-existing failures from task-created failures.
- [ ] Update the capsule's current slice only after the baseline is understood.
- [ ] Commit only if the refreshed plan itself needed factual corrections.

# Phase 1 - Replace vector-only return state with callable return contracts

## Goal

Establish explicit `Returns` versus `Never` vocabulary through every semantic layer while preserving current source behaviour. This phase is a data-model cutover, not the public `-> !` syntax activation.

## Implementation checklist

### Neutral and AST signatures

- [ ] Add one neutral signature return-contract enum.
- [ ] Add one resolved AST callable return-contract enum.
- [ ] Move ordinary return slots into the `Returns` variant.
- [ ] Keep return-slot channel rules unchanged inside `Returns`.
- [ ] Add named queries for ordinary success types, error type, no-value completion and future Never detection.
- [ ] Reject accidental ordinary-return queries on Never rather than treating it as an empty return list.
- [ ] Thread the new shape through declaration shells, remapping, source rebinding, ordering hints and AST signature resolution.
- [ ] Do not accept `-> !` source syntax yet unless the whole dormant representation can round-trip safely without exposing partial behaviour.

### Public, trait and generated vocabulary

- [ ] Add explicit public function and receiver-method return-contract vocabulary.
- [ ] Add explicit trait requirement return-contract vocabulary.
- [ ] Add explicit external function return-contract vocabulary.
- [ ] Thread ordinary `Returns` through public-interface projection, import projection, generic template retention, generated materialisation, receiver catalogues and conformance matching.
- [ ] Include the return-contract variant in equality and fingerprint inputs.
- [ ] Keep all current ordinary signatures semantically identical.

### HIR function metadata

- [ ] Replace `HirFunction::return_type: TypeId` with `HirFunctionReturnContract::Returns(TypeId)` plus a dormant `Never` variant.
- [ ] Thread ordinary return types through HIR declaration registration, validation, displays, test fixtures, JS return lowering, Wasm signatures and all backend ABI queries.
- [ ] Do not map dormant Never to `None` or another type ID.
- [ ] Remove the old direct field and every compatibility accessor that would preserve its ambiguous shape.

### External metadata

- [ ] Replace `returns` plus `error_return_type` as top-level external-function state with one explicit return contract.
- [ ] Preserve builder-friendly construction for ordinary functions without adding parallel legacy constructors.
- [ ] Reject a Never contract carrying return alias metadata by construction.
- [ ] Update external package registration, clone accounting, provider conversion and tests.

## Phase 1 audit and style-guide review

- [ ] Confirm `Returns` and `Never` are enums at each real boundary, not a shared broad type that leaks donor-local identities.
- [ ] Confirm no `is_never` boolean duplicates an enum variant.
- [ ] Confirm `TypeEnvironment`, `ParsedTypeRef`, `DataType` and canonical type identity gained no Never state.
- [ ] Confirm ordinary no-value functions still use the ordinary `Returns(NoneTypeId)` path in HIR.
- [ ] Confirm every old direct return vector or `HirFunction.return_type` consumer was cut over or deleted.
- [ ] Confirm no compatibility wrapper preserves the old API.
- [ ] Confirm test fixtures use the new real shape rather than test-only shortcuts.
- [ ] Perform the mandatory read-only phase audit and Slice review.

## Phase 1 validation and acceptance

Run focused signature, interface, trait, external package, HIR and backend tests, then:

```bash
cargo fmt --all
cargo test --workspace --quiet -- --format terse
cargo run --quiet -- tests --audit
just validate
```

- [ ] Confirm all existing ordinary source behaviour is unchanged.
- [ ] Confirm benchmark history and tracked summaries are unchanged.
- [ ] Resolve all task-related failures before accepting the phase.

# Phase 2 - Consolidate control-flow exits and divergence proof

## Goal

Replace duplicate terminality logic with one pure AST exit analysis while preserving existing diagnostics. Add the internal facts required for declared Never and true-loop proof before public syntax depends on them.

## Implementation checklist

### Shared exit analysis

- [ ] Create one focused AST control-flow analysis module with clear ownership documentation.
- [ ] Represent fallthrough, produced values, caller returns, divergence and nearest-loop exits independently.
- [ ] Implement deterministic alternative-branch union and reachable statement sequencing.
- [ ] Stop scanning a sequence after no path can reach the next statement.
- [ ] Classify `return` and `return!` as caller returns, not divergence.
- [ ] Classify normalised false assertions as divergence.
- [ ] Keep dynamic assertions as fallthrough-capable.
- [ ] Classify existing runtime failure constructs only where they are source-visible AST exits. Do not infer backend traps from later IR.
- [ ] Consume lexical-scope exits without losing their child-scope semantics.

### Branches and matches

- [ ] Preserve existing statement `if` terminality.
- [ ] Preserve value-producing `if`, match and catch completeness.
- [ ] Preserve match exhaustiveness rules and required default handling.
- [ ] Preserve the existing at-least-one-producing-path rule.
- [ ] Keep known-Bool specialization as the only owner that removes inactive branches.
- [ ] Analyze only the specialised active AST for terminality while keeping earlier frontend validation unchanged.

### Loops

- [ ] Add nearest-loop break and continue accounting without introducing labels or source-visible IDs.
- [ ] Treat a normalised-true conditional loop with no reachable break targeting itself as non-fallthrough.
- [ ] Convert body fallthrough and continue paths into divergence at that loop boundary.
- [ ] Preserve return-to-caller paths through the loop.
- [ ] Preserve divergence paths through the loop.
- [ ] Let a reachable break targeting the loop expose post-loop fallthrough.
- [ ] Ensure nested loops consume their own break and continue facts.
- [ ] Keep collection and range loops conservative.

### Existing consumers

- [ ] Rewrite ordinary function terminality to query the shared summary.
- [ ] Rewrite value-production completeness and reachable `then` traversal to use the shared reachability owner.
- [ ] Delete duplicate false-assert and branch-terminal classifiers.
- [ ] Keep existing user-facing diagnostics stable unless a more precise source location is required.
- [ ] Add pure unit tests for summary union, sequencing, nested loops, mixed return/diverge paths and unreachable tails.

## Phase 2 audit and style-guide review

- [ ] Confirm there is one exit-analysis owner and no second recursive AST walk deciding the same semantics.
- [ ] Confirm return-to-caller and divergence remain distinct.
- [ ] Confirm value-production diagnostics select policy from shared facts rather than being embedded in the analyzer.
- [ ] Confirm nested `break` cannot escape its nearest loop.
- [ ] Confirm true-loop proof uses normalised Bool authority rather than a new source-text or literal scanner.
- [ ] Confirm unreachable statements cannot influence summaries or inferred produced types.
- [ ] Confirm existing assertion-message and static-if owners were not duplicated.
- [ ] Perform the mandatory read-only phase audit and Slice review.

## Phase 2 validation and acceptance

Run focused terminality, value-production, assertion, match and loop tests, then:

```bash
cargo fmt --all
cargo test --workspace --quiet terminality -- --format terse
cargo test --workspace --quiet value_production -- --format terse
cargo run --quiet -- tests --tag branching --backend html
cargo run --quiet -- tests --tag loops --backend html
cargo run --quiet -- tests --audit
just validate
```

- [ ] Confirm previously accepted and rejected programs keep the same result except for any explicitly approved true-loop terminality improvement.
- [ ] Do not expose `-> !` until this phase is accepted.

# Phase 3 - Add `-> !` syntax, callable semantics and statement-only AST calls

## Goal

Activate the complete frontend and semantic interface contract. Source parsing, body validation and call placement must all use the explicit return-contract vocabulary without manufacturing a value.

## Implementation checklist

### Signature syntax

- [ ] Parse bare `!` immediately after `->` as `FunctionReturnContractSyntax::Never`.
- [ ] Require the body-opening colon after a function Never contract.
- [ ] Support bodyless trait requirement `-> !` through the shared signature parser.
- [ ] Reject commas, another `!`, a following type, optional suffixes and error slots after Never with targeted diagnostics.
- [ ] Reject `!` in parameter, field, alias and ordinary type annotation positions through existing type diagnostics.
- [ ] Keep `T!` as the final error-return slot inside ordinary `Returns`.
- [ ] Add syntax tests for every valid and malformed boundary.

### Function declaration validation

- [ ] Resolve `-> !` without requesting or interning a type.
- [ ] Validate explicit Never bodies using the shared specialised-AST exit analysis.
- [ ] Reject active fallthrough at the body end.
- [ ] Reject active ordinary returns at their authored locations.
- [ ] Keep existing return-shape and propagation diagnostics in all authored branches before specialization.
- [ ] Diagnose user-authored unannotated functions whose active body is all-path divergent and present both explicit contract choices.
- [ ] Exclude the compiler-synthesised entry `start` from this diagnostic while allowing its root work to diverge.
- [ ] Allow explicitly typed ordinary functions whose active body currently diverges.
- [ ] Keep ordinary typed function terminality unchanged.
- [ ] Prevent a declared Never function from using its own or a mutually recursive unvalidated contract as circular proof in the first implementation.

### Standalone call parsing

- [ ] Introduce an AST statement shape for a resolved Never call. It must carry the same target/receiver and routed argument facts needed by ordinary call lowering, without an `Expression` wrapper or `TypeId`.
- [ ] Refactor shared call construction to return a named resolved outcome such as value call versus terminating call.
- [ ] Let statement context accept the terminating outcome and emit the Never-call AST node.
- [ ] Let every value context reject the terminating outcome with one targeted diagnostic.
- [ ] Cover free functions, receiver methods, generic calls, imported calls and binding-backed calls through the same route.
- [ ] Preserve named arguments, defaults, mutable access and generic request emission.
- [ ] Reject postfix handling and `catch` on a Never call.
- [ ] Reject Never calls in assertion conditions and messages before assertion-specific HIR.
- [ ] Ensure a Never call ends reachable AST statement sequencing.

### Public interfaces and exact matching

- [ ] Project Never through exported free-function and receiver-method interfaces.
- [ ] Project Never through trait requirements without a fake return type.
- [ ] Make conformance matching exact on the return-contract variant.
- [ ] Import Never callable contracts without donor-local state.
- [ ] Include the variant in public-interface equality and fingerprinting.
- [ ] Treat changing ordinary returns to Never as a semantic interface change.

### Generics and generated requests

- [ ] Retain Never on generic function templates.
- [ ] Infer generic arguments for Never calls from immediate call arguments and evidence only. There is no expected result context.
- [ ] Emit concrete requests from active standalone Never calls.
- [ ] Preserve inactive static-branch request discard.
- [ ] Ensure generated concrete signatures retain Never exactly.
- [ ] Reject malformed generic declarations whose parameters cannot meet the existing public-shape usage rules.

### Binding-backed and annotated JS contracts

- [ ] Allow explicit Never in compiler-owned external function metadata.
- [ ] Extend `@moth.sig` parsing to accept a whole `-> !` contract.
- [ ] Keep external signature restrictions otherwise unchanged.
- [ ] Reject external Never definitions that also provide success returns, error returns or return aliases.
- [ ] Require explicit provider metadata. Do not inspect foreign source bodies to infer Never.

## Phase 3 audit and style-guide review

- [ ] Confirm bare `!` is parsed only by callable return-contract syntax and remains unavailable as an ordinary type.
- [ ] Confirm one call argument parser serves value calls and Never calls.
- [ ] Confirm no Never call is stored in `ExpressionKind` or assigned a `TypeId`.
- [ ] Confirm diagnostics distinguish permanent `-> !` intent from temporary typed placeholders.
- [ ] Confirm only explicit Never contracts propagate across call boundaries.
- [ ] Confirm trait and public-interface matching is exact.
- [ ] Confirm generic requests are emitted only from active standalone calls.
- [ ] Confirm external bindings require explicit metadata.
- [ ] Confirm no temporary private-only or same-module-only semantic restriction was introduced.
- [ ] Perform the mandatory read-only phase audit and Slice review.

## Phase 3 validation and acceptance

Run focused parser, function, call, trait, generic, public-interface and external-package tests, then:

```bash
cargo fmt --all
cargo test --workspace --quiet function_parsing -- --format terse
cargo test --workspace --quiet trait -- --format terse
cargo test --workspace --quiet public_interface -- --format terse
cargo test --workspace --quiet external_packages -- --format terse
cargo run --quiet -- tests --audit
just validate
```

- [ ] Add integration diagnostics for malformed signatures, body contract violations and value-position calls.
- [ ] Confirm every new diagnostic uses stable payload identity and precise source locations.
- [ ] Do not update the progress matrix to supported until HIR and backends are complete.

# Phase 4 - Lower Never contracts and calls through HIR

## Goal

Give HIR an explicit no-return function contract and dedicated terminating call while preserving ordinary call identity, argument evaluation and CFG invariants.

## Implementation checklist

### HIR function contracts

- [ ] Lower AST ordinary returns to `HirFunctionReturnContract::Returns`.
- [ ] Lower AST Never to `HirFunctionReturnContract::Never`.
- [ ] Emit no implicit return for Never functions.
- [ ] Treat surviving fallthrough in a Never function as an internal AST-to-HIR invariant failure because AST should have diagnosed it.
- [ ] Make HIR validation reject return terminators in Never functions.
- [ ] Make HIR validation reject a Never function whose reachable block graph has a normal return or unfinished terminator.
- [ ] Keep ordinary no-value functions distinct and still emit ordinary unit return where required.

### Never-call terminator

- [ ] Add the dedicated `NeverCall` HIR terminator.
- [ ] Reuse ordinary call target resolution and argument lowering.
- [ ] Evaluate receiver and arguments once in normal left-to-right call order.
- [ ] Preserve fresh-rvalue materialisation for mutable arguments.
- [ ] Emit no result local and no continuation block.
- [ ] Stop lowering later statements on the current path.
- [ ] Map the authored call location into the HIR side table.
- [ ] Remap every argument and target-owned string identity correctly.

### HIR validation and utilities

- [ ] Validate Never-call arguments like ordinary call arguments.
- [ ] Resolve the target's callable return contract from local HIR, generated sidecars, public interfaces or binding metadata as appropriate.
- [ ] Reject a `NeverCall` whose target is not explicitly Never as `CompilerError` because well-formed AST cannot produce it.
- [ ] Reject an ordinary call statement or expression targeting an explicit Never contract as `CompilerError`.
- [ ] Update terminator successor queries so NeverCall has no successors.
- [ ] Update block validation, reachability roots, display, debug views, remapping, source mapping and structured-HIR derivation.
- [ ] Update test fixture constructors to require an explicit HIR return contract.
- [ ] Do not add a fake Never expression or result local for fixture convenience.

### HIR call facts

- [ ] Record NeverCall as a normal call edge in per-function link facts.
- [ ] Collect resource, project-context, capability, reactive and target-gated facts from its arguments and target exactly once.
- [ ] Preserve deterministic source order.
- [ ] Keep no result provenance or result resource facts because no result exists.

## Phase 4 audit and style-guide review

- [ ] Confirm HIR has one semantic Never-call operation rather than call plus source failure.
- [ ] Confirm the target contract is validated from explicit metadata, not inferred from result absence.
- [ ] Confirm no result local, tuple, unit value or Never TypeId exists.
- [ ] Confirm CFG successor utilities and unreachable-tail lowering agree.
- [ ] Confirm call argument order and side-table locations match ordinary calls.
- [ ] Confirm HIR display and test helpers expose the real contract rather than hiding it.
- [ ] Confirm every exhaustive terminator match was intentionally updated.
- [ ] Perform the mandatory read-only phase audit and Slice review.

## Phase 4 validation and acceptance

Run focused HIR lowering, validation, display and reachability tests, then:

```bash
cargo fmt --all
cargo test --workspace --quiet hir -- --format terse
cargo test --workspace --quiet reachability -- --format terse
cargo run --quiet -- tests --audit
just boracle
just validate
```

- [ ] Confirm no uninitialized terminator or dead continuation block remains after NeverCall lowering.
- [ ] Confirm HIR failures caused by malformed test fixtures are `CompilerError`, not user diagnostics.

# Phase 5 - Integrate borrow, lifetime, link and generated analyses

## Goal

Make every downstream semantic consumer understand NeverCall as a terminating call with ordinary argument effects and no result or successor.

## Implementation checklist

### Borrow validation

- [ ] Generate shared or exclusive argument accesses using the same callable parameter metadata as ordinary calls.
- [ ] Apply mutation and optional final-use transfer effects before the path terminates where the existing call contract requires them.
- [ ] Do not create a result local, result origin, return alias or post-call state.
- [ ] End loan/access liveness at the terminator according to existing block-exit rules.
- [ ] Keep reactive invalidation and argument side effects observable even though no continuation exists.
- [ ] Update statement/terminator metadata collectors and use scanners.

### Lifetime and escape analysis

- [ ] Record argument and retained-edge effects that occur before divergence.
- [ ] Emit no result provenance, detached result or outgoing result family.
- [ ] Represent the absence of a normal exit explicitly in local and exported summaries where required.
- [ ] Ensure project/link summary instantiation never expects a post-call continuation from NeverCall.
- [ ] Preserve cleanup and destruction on paths that execute before the call according to validated HIR and memory planning.
- [ ] Do not invent a special runtime ownership mode for Never.

### Public call summaries

- [ ] Keep parameter access and mutation summaries for Never callables.
- [ ] Mark normal-return/result summary state as absent through explicit callable contract or exit facts.
- [ ] Avoid empty result vectors that could be confused with ordinary unit returns.
- [ ] Ensure generated summaries and public interfaces agree on Never.

### Reachability and link facts

- [ ] Traverse NeverCall targets as ordinary call edges.
- [ ] Include generated targets raised by standalone Never calls.
- [ ] Include binding-backed runtime imports and helper requirements.
- [ ] Traverse argument expressions for nested ordinary calls, resources, reactive facts and target features.
- [ ] Do not traverse a nonexistent successor.

### Boracle

- [ ] Update normalized problem extraction for NeverCall.
- [ ] Preserve call argument access and call-effect events before the terminal exit.
- [ ] Add a terminal problem representation that cannot be mistaken for a normal return.
- [ ] Update the bounded operational oracle, validation, rendering, reducers and generated problem support where the terminator vocabulary is exhaustive.
- [ ] Keep the reference solver and operational oracle semantically aligned on the new event order.
- [ ] Add focused fixtures for shared/mutable arguments, retained argument effects and no successor.

## Phase 5 audit and style-guide review

- [ ] Confirm argument effects are neither dropped nor applied twice.
- [ ] Confirm no result facts or post-call state exist.
- [ ] Confirm exported summaries distinguish Never from unit return.
- [ ] Confirm reachability includes the callee and argument dependencies.
- [ ] Confirm memory analysis consumes HIR facts without source inspection.
- [ ] Confirm Boracle models the same call-before-terminal ordering as production HIR.
- [ ] Confirm no backend or analysis infers Never from `result: None`.
- [ ] Perform the mandatory read-only phase audit and Slice review.

## Phase 5 validation and acceptance

Run focused borrow, lifetime, call-summary, reachability, generated and Boracle tests, then:

```bash
cargo fmt --all
cargo test --workspace --quiet borrow_checker -- --format terse
cargo test --workspace --quiet call_summary -- --format terse
cargo test --workspace --quiet generated -- --format terse
cargo test --workspace --quiet reachability -- --format terse
just boracle
cargo run --quiet -- tests --audit
just validate
```

- [ ] Confirm the opt-in Boracle lane passes independently of `just validate`.
- [ ] Confirm no analysis accepts an impossible normal successor after NeverCall.

# Phase 6 - Add JavaScript and current Wasm lowering

## Goal

Lower the explicit HIR contract on every current target before the larger Wasm backend rewrite begins. Preserve a defensive hard stop if a declared Never callee violates its contract at runtime.

## Implementation checklist

### JavaScript

- [ ] Emit Never functions with no source-visible return value.
- [ ] Lower a NeverCall through the existing target and argument call-emission owner.
- [ ] Emit the call once.
- [ ] Immediately emit a compiler-owned throw if control returns from the call.
- [ ] Use an invariant message that identifies a violated `-> !` contract without exposing internal IDs.
- [ ] Preserve external-module import/glue handling for binding-backed Never calls.
- [ ] Preserve reactive invalidations and argument effects that happen before the call.
- [ ] Ensure dispatcher control cannot continue to another block after NeverCall.

Conceptual emitted shape:

```javascript
fatal(message);
throw new Error("Moth `-> !` function returned unexpectedly");
```

The exact helper or emitted text may use the existing runtime-failure owner. It must remain compiler-owned and unrecoverable.

### Current Wasm backend

- [ ] Map HIR Never function contracts to zero Wasm results.
- [ ] Lower NeverCall to a result-less direct/import call followed by the existing LIR trap/unreachable path.
- [ ] Preserve argument lowering and call target identity.
- [ ] Reject unsupported Never-call targets during target validation rather than dropping the call.
- [ ] Emit `unreachable` after every Never call, including binding-backed calls.
- [ ] Add artefact tests that prove call then unreachable ordering.
- [ ] Do not redesign the whole Wasm LIR in this plan.

### Target validation

- [ ] Inspect NeverCall arguments and targets exactly like ordinary calls.
- [ ] Keep unsupported binding or feature diagnostics rooted at the authored call.
- [ ] Do not reject Never merely because it has no result type.
- [ ] Verify that a backend which accepts the call can express an unreachable fallback.

### Downstream Wasm contract

- [ ] Update the queued HTML mixed JavaScript and Wasm backend plan's named prerequisites to require the delivered Never return contract and HIR NeverCall terminator.
- [ ] Require the future structured Wasm LIR to preserve call plus unreachable semantics.
- [ ] Require the future Wasm tests to retain local, cross-module, generated and binding-backed NeverCall coverage.
- [ ] Do not link one plan as a semantic authority. Name the delivered capability and point both plans at permanent compiler documentation.

## Phase 6 audit and style-guide review

- [ ] Confirm JS and Wasm consume explicit HIR Never facts rather than result absence.
- [ ] Confirm a returning external Never implementation cannot reach code HIR marked unreachable.
- [ ] Confirm the defensive fallback is emitted after, not before or instead of, the call.
- [ ] Confirm backend target selection and external glue remain in their current owners.
- [ ] Confirm no whole-Wasm redesign or compatibility adapter entered this plan.
- [ ] Confirm artefact tests protect semantics without freezing unrelated formatting.
- [ ] Perform the mandatory read-only phase audit and Slice review.

## Phase 6 validation and acceptance

Run focused JS, Wasm, backend validation and integration tests, then:

```bash
cargo fmt --all
cargo test --workspace --quiet js -- --format terse
cargo test --workspace --quiet wasm -- --format terse
cargo run --quiet -- tests --tag functions --backend html
cargo run --quiet -- tests --tag functions --backend html_wasm
cargo run --quiet -- tests --audit
just validate
```

- [ ] Confirm emitted JavaScript throws if a test external Never function returns.
- [ ] Confirm emitted Wasm validates and contains the required unreachable path.
- [ ] Confirm ordinary functions and result-less unit calls retain their previous lowering.

# Phase 7 - Complete user-visible integration coverage

## Goal

Prove the complete source contract across local, cross-module, generic, trait, external and control-flow boundaries with one clear primary owner per behaviour.

## Required integration coverage

### Valid declarations and calls

- [ ] local `fatal || -> !` ending in `assert(false)`
- [ ] Never receiver method
- [ ] exported Never function imported and called from another module
- [ ] generic Never function whose type arguments are inferred from arguments
- [ ] generated NeverCall reachability and backend lowering
- [ ] trait requirement `-> !` with exact conforming method
- [ ] annotated external JS `@moth.sig ... -> !`
- [ ] true conditional loop with no reachable break satisfying a Never body
- [ ] nested inner-loop break not invalidating outer true-loop divergence
- [ ] block-form value producer with one `then` branch and one Never-call branch
- [ ] ordinary typed placeholder body that only asserts false

### Invalid signatures

- [ ] `-> !, String`
- [ ] `-> String, !`
- [ ] `-> Error!, !`
- [ ] `-> !!`
- [ ] Never used as a parameter, field, alias or collection element
- [ ] `None` used as an attempted explicit return type remains invalid under its existing rule

### Invalid Never bodies

- [ ] active fallthrough
- [ ] active bare `return`
- [ ] active `return!`
- [ ] active option/error propagation to the caller
- [ ] one divergent branch and one completing branch
- [ ] true loop with a reachable break targeting itself
- [ ] recursive or mutually recursive proof cycle receives the accepted deferred-proof diagnostic rather than an internal error or unsound acceptance

### Invalid call placement

- [ ] declaration initializer
- [ ] assignment RHS
- [ ] return value
- [ ] ordinary function argument
- [ ] receiver or constructor argument
- [ ] operator operand
- [ ] condition
- [ ] template interpolation
- [ ] collection/map element
- [ ] assertion condition or message
- [ ] inline value-producing branch
- [ ] postfix `!`, postfix `?` and `catch`

### Explicitness diagnostic

- [ ] unannotated all-path false assertion
- [ ] unannotated all-path true loop
- [ ] unannotated structured branches that all diverge
- [ ] diagnostic offers permanent `-> !` and temporary ordinary-type choices
- [ ] explicitly typed equivalent is accepted
- [ ] unannotated function that may complete remains an ordinary no-value function
- [ ] compiler-synthesised `start` may diverge without an impossible source annotation

### Exact interfaces

- [ ] trait requires Never but implementation declares ordinary type
- [ ] trait requires ordinary type but implementation declares Never
- [ ] public-interface fingerprint changes when return contract changes
- [ ] same-file callers do not infer divergence from an ordinary typed callee body
- [ ] cross-module callers do not infer divergence from an ordinary typed callee body
- [ ] external function with no returns remains ordinary unit unless explicitly marked Never

## Test ownership

- Put user-visible syntax, diagnostics and runtime behaviour under `tests/cases/`.
- Use one primary contract for the Never return surface and boundary/adversarial cases for distinct failures.
- Put pure exit-summary tests under the AST control-flow analysis test module.
- Put HIR shape and invariant tests under `src/compiler_frontend/hir/tests/`.
- Put public-interface, trait, external package and generated-sidecar invariants under their owning test directories.
- Put JS and Wasm artefact assertions under their backend test owners.
- Keep Boracle semantics in the opt-in Boracle test tree.
- Do not use benchmark fixtures as correctness evidence.
- Remove or rewrite old tests that encode vector-only return state, duplicated false-assert terminality or `HirFunction.return_type` directly.

## Phase 7 audit and style-guide review

- [ ] Map each accepted behaviour to one primary test owner.
- [ ] Confirm no fixture duplicates another without protecting a distinct boundary.
- [ ] Confirm diagnostics assert stable codes/reasons and source locations.
- [ ] Confirm runtime tests prove the defensive fallback where observable.
- [ ] Confirm HIR tests do not make incidental block IDs or formatting contractual.
- [ ] Confirm cross-module, generic, trait and external surfaces are all covered.
- [ ] Confirm deferred recursion and richer loop proof are rejected cleanly and recorded as gaps.
- [ ] Perform the mandatory read-only phase audit and Slice review.

## Phase 7 validation and acceptance

Run targeted canonical cases and the complete suite:

```bash
cargo fmt --all
cargo run --quiet -- tests --contract language.functions.never_return_contract
cargo run --quiet -- tests --tag functions --backend html
cargo run --quiet -- tests --tag functions --backend html_wasm
cargo run --quiet -- tests --audit
just boracle
just validate
```

Use the final contract ID selected by the live manifest conventions. Do not create duplicate primary ownership.

# Phase 8 - Update permanent documentation, progress and roadmap

## Goal

Move the accepted contract into permanent authorities, report the implemented core accurately and keep every accepted proof extension visible as deferred work.

This phase is documentation-only unless its review exposes an implementation defect. If Rust, tests, fixtures, scripts or manifests change, treat it as code-bearing and run `just validate`.

## Canonical language documentation

- [ ] Update `function-declarations.mtf` with `-> !` as a whole callable return contract.
- [ ] Update `returns-and-multiple-values.mtf` to distinguish ordinary no-value completion, typed placeholders and Never.
- [ ] Update `calls-and-access.mtf` with standalone-only Never calls and invalid value contexts.
- [ ] Update `assertions.mtf` to explain that `assert(false)` proves divergence but remains a statement rather than a Never value.
- [ ] Update `value-producing-if.mtf` with a block-form producing/diverging example and the invalid inline value form.
- [ ] Update conditional-loop and loop-control references with the normalised-true/no-break proof boundary.
- [ ] Update trait requirement and conformance references with exact Never matching.
- [ ] Update generic declaration/inference references with argument-only inference for Never calls.
- [ ] Update external binding contracts with explicit `@moth.sig ... -> !` and the no-success/no-error rule.
- [ ] Update the cheatsheet with compact syntax, placement and proof rules.
- [ ] Update paired Basic pages only where the concept belongs at that teaching level.

## Compiler and build architecture

Update `docs/compiler-design-overview.md` with durable ownership for:

- callable return-contract vocabulary separate from type identity
- the absence of a Never `TypeId`
- shared AST control-flow exit analysis
- specialised active-AST Never validation
- statement-only Never calls
- public interface and exact trait/binding contracts
- `HirFunctionReturnContract`
- `HirTerminator::NeverCall`
- borrow, lifetime and link-fact treatment
- backend call plus unreachable fallback

Review `docs/build-system-design.md`. Edit only if target validation, link facts or the downstream Wasm handoff lack a durable contract. Do not copy language semantics into build-system prose.

Review `index.md`. Update it only if files move, a new subsystem module is added or locator text becomes materially inaccurate.

## Progress matrix

Add or update one focused **Never return contracts** row in `docs/src/docs/progress/@page.moth`.

Before implementation starts, the row may be `Deferred` with coverage `None` if the accepted design is recorded early. After this plan lands, set it to `Partial`, not `Supported`, because accepted proof extensions remain.

The implemented-state row must say:

- source syntax is `-> !`
- it is a whole callable contract, not a first-class type or `TypeId`
- calls are standalone terminating statements and cannot be consumed as values
- explicit contracts propagate through local, cross-module, generic, trait and binding-backed surfaces
- core divergence proof covers false assertions, explicit Never calls, normalised-true conditional loops without a reachable break, lexical scopes, branches and exhaustive matches
- HIR uses an explicit Never function contract and terminating call
- JavaScript and current Wasm lowering include a defensive unreachable fallback
- ordinary typed functions may currently diverge without becoming Never to callers

The same row must list these accepted future gaps:

- direct and mutual recursion proof for declared Never bodies
- richer data-flow proof of permanently true runtime loop conditions
- broader structural or whole-program non-termination proof

Do not list first-class Never values, bottom coercion, expression-position calls or implicit ordinary-call divergence as future gaps. Those are deliberate exclusions.

Update the existing **Assertions**, **Functions and calls**, **Control flow**, **Traits**, **Generics** and external-binding notes only where they would otherwise contradict the new row. Avoid duplicating the whole Never contract across several rows.

## Roadmap

While queued, keep this plan immediately after runtime anonymous records and before the HTML mixed JavaScript and Wasm backend plan.

At implementation closeout:

- [ ] delete this plan and remove its queued roadmap entry in the same completion commit
- [ ] keep the downstream Wasm plan after the delivered capability
- [ ] add a **Never return proof follow-ups** subsection under deferred design if no existing focused section owns the proof gaps
- [ ] list direct/mutual recursion proof, richer loop-condition proof and broader deterministic non-termination proof there
- [ ] do not add final constraints as deferred work
- [ ] ensure the progress matrix repeats the accepted proof gaps so users can see current support without reading the roadmap

## Generated documentation

- [ ] Check docs with the current compiler.
- [ ] Rebuild `docs/release/**` through the compiler.
- [ ] Review the generated diff for the new syntax, code highlighting and link integrity.
- [ ] Do not hand-edit generated output.

## Phase 8 audit and style-guide review

- [ ] Confirm permanent docs, not this temporary plan, own the final semantics.
- [ ] Confirm Advanced and Basic pages remain truthful at their chosen depth.
- [ ] Confirm matrix status is `Partial` and names every accepted proof gap.
- [ ] Confirm deliberate exclusions are not misreported as future promises.
- [ ] Confirm roadmap ordering and downstream Wasm prerequisites are correct.
- [ ] Confirm generated docs came only from the source build.
- [ ] Perform the mandatory read-only documentation audit and Slice review.

## Phase 8 validation and acceptance

For a strictly documentation-only phase:

```bash
cargo run --quiet -- check docs --terse
cargo run --quiet -- build docs --release
git diff --check
```

If any code-bearing file changes, also run:

```bash
just validate
```

- [ ] Resolve every docs warning, broken example and generated-diff mismatch.
- [ ] Confirm no implementation claim exceeds tested support.

# Phase 9 - Delete stale paths and complete final review

## Goal

Prove the repository has one final Never return path, no fake type/value representation and no stale vector-only or duplicate terminality assumptions.

## Stale-path and ownership searches

Run focused searches and inspect every match:

```bash
rg -n 'HirFunction\s*\{[^}]*return_type|\.return_type\b' src tests
rg -n 'FunctionSignature\s*\{[^}]*returns|FunctionSignatureSyntax\s*\{[^}]*returns' src tests
rg -n 'PublicFunctionSemantics.*returns|PublicReceiverMethodSemantics.*returns' src tests
rg -n 'error_return_type' src tests
rg -n 'assert_condition_is_statically_false|statically_false.*assert|BranchExitSummary::TERMINATES' src tests
rg -n 'Never.*TypeId|TypeId.*Never|DataType::Never|ParsedTypeRef::Never|CanonicalTypeIdentity::Never' src tests docs --glob '!docs/release/**'
rg -n 'NeverCall|HirTerminator::NeverCall|FunctionReturnContract' src tests docs --glob '!docs/release/**'
```

Expected outcomes:

- no old direct HIR return-type field remains
- no vector-only callable contract remains where Never is legal
- no duplicated false-assert terminality classifier remains
- no Never type identity exists
- every NeverCall match is intentional and exhaustive
- ordinary result-less calls remain distinct from Never calls
- current Wasm and future Wasm plan wording both preserve call plus unreachable

## Final architecture audit

Give a read-only final auditor:

- this plan
- the complete final diff
- the accepted interview decisions
- the phase audit findings and resolutions
- exact validation results
- the permanent documentation updates
- the progress and roadmap changes

The final audit must check:

- [ ] exact source syntax and signature exclusivity
- [ ] no first-class Never type or bottom coercion
- [ ] standalone-only call placement
- [ ] explicit-only interprocedural propagation
- [ ] typed-placeholder behaviour
- [ ] unannotated divergence diagnostics
- [ ] shared control-flow exit ownership
- [ ] true-loop and nested-break correctness
- [ ] exact trait, generic, public and external contracts
- [ ] HIR contract and NeverCall invariants
- [ ] borrow, lifetime, reachability and generated facts
- [ ] Boracle parity
- [ ] JS and Wasm defensive fallback
- [ ] no compatibility or stale path
- [ ] correct test ownership
- [ ] permanent documentation authority
- [ ] progress matrix gaps and roadmap ordering
- [ ] style-guide compliance

Resolve every actionable finding, rerun affected phase gates and obtain a fresh clean final audit.

## Final validation

Run the complete final state gates:

```bash
cargo fmt --all -- --check
cargo run --quiet -- tests --audit
just boracle
just validate
cargo run --quiet -- build docs --release
git diff --check
```

- [ ] Record exact results and counts.
- [ ] Confirm generated docs are current.
- [ ] Confirm benchmark history and tracked summaries changed only if an independently justified benchmark update was required.
- [ ] Confirm `git status --short` contains only intended completion files.
- [ ] Delete this plan and remove its roadmap entry in the same completion commit.

## Definition of done

The plan is complete only when:

- [ ] `-> !` parses as one whole callable return contract
- [ ] Never cannot mix with success, optional or error return slots
- [ ] no Never type, value, `TypeId`, `DataType`, parsed type or canonical type identity exists
- [ ] explicit Never works for functions, methods, generics, traits, exports and binding-backed calls
- [ ] callable compatibility is exact
- [ ] only explicit `-> !` propagates divergence across call boundaries
- [ ] Never calls are standalone statements only
- [ ] value-producing blocks accept producing/diverging branch mixtures without coercion
- [ ] all-diverging value producers still require at least one producing path
- [ ] unannotated all-path divergence requires an explicit contract choice
- [ ] explicitly typed placeholder implementations may diverge
- [ ] declared Never bodies reject fallthrough and caller-returning exits
- [ ] false assertions, explicit Never calls and normalised-true no-break loops prove divergence
- [ ] nested loop break targeting is correct
- [ ] one AST exit analysis serves terminality, value production and Never validation
- [ ] HIR functions carry explicit return contracts
- [ ] HIR NeverCall has a target and arguments but no result or successor
- [ ] HIR validation checks the target contract
- [ ] borrow and lifetime analyses apply argument effects with no result or post-call state
- [ ] reachability and link facts retain the call edge and argument facts
- [ ] generated sidecars preserve Never
- [ ] Boracle models call effects before terminal divergence
- [ ] JavaScript emits a defensive throw after the call
- [ ] current Wasm emits a result-less call followed by unreachable
- [ ] the downstream Wasm plan names and preserves the delivered contract
- [ ] canonical docs own the final semantics
- [ ] the progress matrix reports implemented core support as `Partial`
- [ ] every accepted proof extension is visible in the progress matrix and roadmap
- [ ] deliberate exclusions are not presented as future work
- [ ] all mandatory phase audits and Slice reviews are complete
- [ ] `just boracle`, `just validate` and the docs release build pass
- [ ] the plan and roadmap entry are retired together
