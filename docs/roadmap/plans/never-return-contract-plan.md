# Never return contract implementation plan

## Purpose

Add the explicit `-> !` Never return contract across Moth's callable, control-flow, HIR, analysis and backend boundaries before the HTML mixed JavaScript and Wasm backend implementation starts.

`-> !` states that a callable never returns control normally. It is a callable control-flow contract, not a value type. The implementation must not add a source-visible `Never` type, a Never `TypeId`, a fake `None` result, bottom-type coercion or an expression value that can stand in for another type.

This plan also replaces the current duplicated terminality logic with one AST-owned exit analysis that distinguishes value production, return to the caller, divergence and loop-local exits. Ordinary function terminality, value-producing block completeness, declared Never validation and structural true-loop proof must consume that one fact owner.

## Current-state capsule

```text
ACTIVE_PLAN: docs/roadmap/plans/never-return-contract-plan.md
STATUS: queued
CURRENT_SLICE: Phase 0 - refresh callable, terminality, HIR, analysis and backend owners
BLOCKERS: runtime anonymous records must be delivered first
NEXT_ACTION: activate after runtime anonymous records, record the live repository state and rerun the owner inventory
```

Keep this block concise. Establish the active revision, branch, worktree state and validation baseline in untracked working notes when implementation starts. Do not pin a queued plan to a commit.

## Roadmap position

This plan runs after runtime anonymous records and immediately before the HTML mixed JavaScript and Wasm backend implementation.

The order is deliberate. The Wasm implementation must consume a settled HIR function return contract and terminating Never call. It must not invent Never semantics from missing results or backend `unreachable` instructions.

At closeout, delete this plan and remove its roadmap entry in the same commit. Keep the delivered capability before the Wasm implementation in the roadmap order.

## Hard prerequisites

- runtime anonymous records are delivered and their HIR, borrow and lifetime integration is stable
- canonical module compilation and immutable public semantic interfaces are delivered
- generated concrete functions use sidecars and stable callable identities
- Stage 4 static Bool specialisation selects active AST control flow before terminality and durable executable facts
- value-producing `if`, match and catch blocks distinguish producing paths from terminating paths
- `assert(false)` already lowers through explicit unrecoverable control flow
- HIR calls use stable local, cross-module, generated and binding-backed targets
- borrow validation, lifetime analysis and link facts consume validated HIR without rewriting it
- JavaScript and the current experimental Wasm backend lower explicit HIR terminators

Name delivered capabilities rather than citing temporary plans as semantic authorities.

## Required authorities

Read these from the active worktree before implementation. Re-read the affected sections during every phase audit.

- `AGENTS.md`
- `docs/compiler-design-overview.md` in full
- `docs/build-system-design.md` opening authority, architectural invariants, generated-function boundary, entry/package linking, target validation and mixed-target sections
- `docs/src/developer-docs/language/overview.mtf`
- `docs/src/docs/functions/function-declarations.mtf`
- `docs/src/docs/functions/calls-and-access.mtf`
- `docs/src/docs/functions/returns-and-multiple-values.mtf`
- `docs/src/docs/errors/assertions.mtf`
- `docs/src/docs/branching/value-producing-if.mtf`
- the canonical pattern-matching references affected by exit analysis
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

Permanent language references and compiler architecture must own the final semantics before this plan is retired.

# Accepted design

## Source syntax

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

The bare `!` appears immediately after `->`. It is not a suffix on a type and does not reserve a keyword.

`-> !` is mutually exclusive with every success, optional and error return slot. Reject all of these with targeted signature diagnostics:

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

Do not let bare `!` fall into ordinary type parsing as a missing type. Keep `T!` as the existing final error-return slot inside an ordinary return contract.

## Callable surfaces

The Never return contract is valid anywhere Moth stores a callable contract:

- ordinary functions
- receiver methods
- named function values and interned function signatures
- generic function templates and generated concrete functions
- trait requirements and conformance matching
- exported functions and receiver methods
- cross-module and future package interfaces
- binding-backed and external functions whose provider metadata explicitly declares Never

The contract is exact. A requirement or function type that says `-> !` matches only another `-> !` contract. A body that happens to diverge does not make an ordinary `-> String` signature compatible with Never. A Never signature does not satisfy an ordinary return contract.

Changing a public callable from ordinary returns to Never, or the reverse, is a public semantic change and must alter its public-interface fingerprint.

## Never is not a value type

Moth does not add:

- a source-visible `Never` name
- a Never `TypeId`
- a Never literal or value
- a `DataType::Never`, `ParsedTypeRef::Never` or `CanonicalTypeIdentity::Never`
- variables, parameters, fields, aliases, options, collections, maps or generic arguments whose value type is Never
- `Never -> T` compatibility or bottom-type coercion
- a fake `None` or unit value for divergence
- a multi-return slot containing Never

`None` remains the unit-like representation for ordinary no-value completion. It has one trivial runtime meaning. A Never callable has no normal return at all.

The current compiler also stores named callable values as function types in `TypeEnvironment`. Such a function value may carry a function signature whose return contract is Never. The value's type is still the existing function type. This does not create a Never value. Invoking that value remains subject to the standalone-statement rule.

Each semantic layer must use an explicit `Returns` versus `Never` enum. Do not add an `is_never` boolean beside ordinary return fields and do not use an empty vector, missing type or sentinel ID to encode Never.

## Implementation divergence versus declared Never

An ordinary typed function may currently diverge:

```moth
load_name || -> String:
    assert(false, "not implemented")
;
```

Callers still see an ordinary `String` call. The body only proves that the current implementation does not fall through without satisfying its declared contract.

An explicit Never contract is stronger:

```moth
fatal |message String| -> !:
    assert(false, message)
;
```

Only an explicit `-> !` contract propagates divergence across a call boundary. This rule applies in the same file, across modules, after inlining, during constant propagation and across generated functions. Optimisation may use stronger implementation knowledge later, but source validity must not.

## Explicitness for unannotated functions

An unannotated user function normally has an ordinary no-success-value contract and may complete at the end of its body.

When its specialised active body is proven to diverge on every reachable path, reject the omitted contract. The diagnostic must explain both valid choices:

- write `-> !` when never returning is the final intended contract
- write the ordinary return type expected in the future when the current body is only an unfinished placeholder

Diagnostic intent:

```text
Function `serve` cannot complete normally.
Declare `-> !` when it is intended never to return, or declare the ordinary
return type it is expected to produce when this implementation is temporary.
```

Use a typed diagnostic payload and retain the function declaration location. The exact wording may follow current renderer conventions, but it must make `-> !` the permanent-divergence choice and an ordinary type the future-value choice.

An explicitly typed ordinary function may diverge on every current path without this diagnostic.

Apply this rule only to user-authored callable declarations with an omitted return contract. The compiler-synthesised entry `start` keeps its builder-owned ordinary contract. Root runtime work may call a Never function and make `start` diverge without requiring impossible source syntax for `start`.

## Declared Never body validation

A function declared `-> !` is valid only when every reachable path in the specialised active AST diverges.

These exits do not satisfy Never:

- falling off the end of the body
- ordinary `return`, including a bare no-value return
- `return!`
- postfix error propagation that returns an error to the caller
- postfix option propagation that returns absence to the caller
- any branch that can complete normally
- a loop that can exit through a reachable `break` targeting that loop

These operations may terminate the current AST path, but they return control to the caller or resume after the loop. They are not divergence.

Validate all authored source before static specialisation. Name resolution, type checking, generic evidence, return shape and propagation legality still apply inside a branch later removed as inactive. After Stage 4 specialisation, only active reachable paths participate in the Never proof. Unreachable tails after a proven non-continuing statement do not invalidate the contract.

## Standalone-only Never calls

A call whose declared contract is `-> !` is valid only as a standalone statement:

```moth
fatal("unreachable")
```

It may appear wherever an ordinary executable statement is legal, including branches, loops, lexical scopes and normal-root runtime work.

Reject it in every value-consuming position:

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

Use a targeted placement diagnostic such as:

```text
`fatal()` never returns and cannot be used as a value.
Call it as a standalone terminating statement instead.
```

Preserve the one shared call-shaped argument parser and parameter-slot routing owner. Do not add a second call grammar.

## Value-producing blocks

A standalone Never call may complete one branch of a block-form value producer because that branch diverges rather than producing a value:

```moth
name = if ready:
    then "Priya"
else
    fatal("name unavailable")
;
```

The Never call does not produce `String` and no coercion occurs.

The inline form remains invalid because it places the call in a value slot:

```moth
name = if ready then "Priya" else fatal("name unavailable")
```

Preserve the existing rule that at least one reachable path must produce values. A value-producing block where every branch diverges still reports the existing no-producing-path diagnostic.

## Initial divergence proof boundary

The first implementation proves divergence through exactly these structural sources:

1. An `assert` whose condition has normalised to compile-time `false`.
2. A standalone call whose published callable contract is `-> !`, subject to recursive proof safety below.
3. A conditional `loop` whose condition has normalised to compile-time `true` and which has no reachable `break` targeting that loop.
4. A lexical scope whose every reachable exit diverges.
5. An `if` with an active `else` where every reachable branch diverges.
6. An exhaustive match where every reachable arm and required default path diverges.
7. Statement sequencing where later statements apply only to paths that can still fall through.

A normalised-true loop may diverge when its body falls through or executes `continue`, because those paths start another iteration. A return path still returns to the caller. A reachable break targeting the loop makes the post-loop continuation reachable.

Nested loop control is structural:

- `break` and `continue` target the nearest loop
- a nested loop consumes its own break and continue facts
- a nested-loop break does not make an outer true loop escapable
- return and divergence facts pass outward

Collection and range loops remain conservatively non-divergent because their iteration count is finite or may be zero.

Use the existing folded Bool authority. Do not add literal-only checks in several consumers.

## Recursive proof safety

An explicit signature is a contract for callers, but a source function must not use an unvalidated cycle of Never signatures as its own proof.

The first implementation must:

- trust a Never provider module only after its interface has been validated and published
- trust explicit binding-backed Never metadata after provider validation
- permit same-module Never dependencies in an acyclic validation order
- reject a self-edge or strongly connected same-module group whose proof depends on its own unvalidated Never edges
- ignore an unreachable recursive edge after independent divergence
- issue a structured deferred-proof diagnostic for a circular proof rather than accepting it or raising `CompilerError`

Keep proof bookkeeping compiler-local. Do not expose validation state in public interfaces, HIR or backend metadata.

# Deliberately deferred proof work

The implemented feature must remain `Partial` in the progress matrix because these accepted proof extensions remain:

- direct-recursion and mutual-recursion proof for declared Never bodies when validation otherwise depends on the same unproven contracts
- richer data-flow proof that a non-literal runtime loop condition remains true forever
- broader deterministic structural, SCC or whole-program non-termination proof

These are proof extensions. They may accept more correctly declared `-> !` bodies later.

The following are final constraints, not deferred gaps:

- ordinary callable contracts never propagate divergence for source validity
- external code is non-returning only when its binding contract explicitly says `-> !`
- Never is not a first-class type or value
- Never calls are not value expressions
- bottom coercion is not part of Moth
- `-> !` cannot mix with ordinary or error return slots

Do not present those constraints as future roadmap promises.

# Intended compiler model

Exact Rust names may change. The distinctions may not.

## Neutral signature syntax

```rust
pub(crate) enum FunctionReturnContractSyntax {
    Returns(Vec<ReturnSlotSyntax>),
    Never { location: SourceLocation },
}
```

`FunctionSignatureSyntax` carries this contract instead of using `returns: Vec<_>` as the complete meaning.

## Resolved callable contract

```rust
pub(crate) enum FunctionReturnContract {
    Returns(Vec<ReturnSlot>),
    Never { location: SourceLocation },
}
```

Provide named queries for `is_never`, success returns, error return and resolved ordinary return IDs. An ordinary-return query must not silently map Never to an empty vector.

## Function type metadata

The current `TypeEnvironment` stores callable signatures inside `TypeDefinition::Function`. Replace `FunctionTypeDefinition`'s direct success and error fields with a function-type return contract when those signatures can describe Never:

```rust
pub enum FunctionTypeReturnContract {
    Returns {
        success: Box<[TypeId]>,
        error: Option<TypeId>,
    },
    Never,
}
```

The containing function type still has an ordinary `TypeId`. No `TypeId` identifies Never itself.

Function type interning, equality, hashing, substitution, display and canonical projection must include the exact return-contract variant. A named function value whose signature is Never remains a function value and can only be invoked as a standalone terminating statement.

## Public and trait interfaces

Use explicit stable vocabulary, for example:

```rust
pub(crate) enum PublicCallableReturnContract {
    Returns {
        success: Vec<PublicReturnTypeSlot>,
        error: Option<CanonicalTypeIdentity>,
    },
    Never,
}
```

Trait-local `This` return vocabulary remains scoped to trait surfaces. A trait Never contract carries no fake type identity.

## Binding-backed contracts

```rust
pub enum ExternalFunctionReturnContract {
    Returns {
        success: Vec<ExternalReturnSlot>,
        error: Option<ExternalSignatureType>,
    },
    Never,
}
```

A binding marked Never has no success aliases and no error slot. Validate that invariant before publication.

## HIR function contract

```rust
pub enum HirFunctionReturnContract {
    Returns(TypeId),
    Never,
}
```

The ordinary variant retains the existing unit, single, tuple or fallible-carrier type. Never carries no type ID and produces no ABI result.

## HIR terminating call

```rust
HirTerminator::NeverCall {
    target: CallTarget,
    args: Vec<HirExpression>,
}
```

The terminator retains the ordinary stable target and evaluated arguments. It has no result local, result type or successor.

Do not encode Never as `Call { result: None }` followed by a source-level failure. HIR owns one semantic call with no continuation. A backend may lower that operation to a call followed by defensive unreachable machinery.

# Shared control-flow exit model

Replace duplicate terminality owners with one AST-owned exit analysis. The representation must retain independent facts equivalent to:

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

A better scoped representation is allowed when needed for nested loops. Do not compress these facts into a tri-state enum.

Alternative branches union exit possibilities. Statement sequencing applies the next statement only to paths that can still fall through. Loop analysis consumes the nearest loop's break and continue facts at the loop boundary.

The analysis should answer named consumer questions:

- may an ordinary no-value function complete normally
- can an ordinary typed function fall through without returning
- is a value-producing branch complete
- does a value producer have at least one producing path
- does every reachable path in a declared Never function diverge
- can a normalised-true loop break to its continuation

Keep diagnostic policy outside the pure summary calculation where practical.

# Reviewed current repository shape

Reverify all paths at activation. These are navigation facts, not permanent architecture.

## Signature and callable owners

- `src/compiler_frontend/declaration_syntax/signature_members.rs` stores `FunctionSignatureSyntax.returns: Vec<ReturnSlotSyntax>` and parses a type before a trailing error-channel `!`
- `src/compiler_frontend/ast/statements/functions.rs` stores `FunctionSignature.returns: Vec<ReturnSlot>` and `ReturnChannel`
- `src/compiler_frontend/datatypes/definitions.rs` stores `FunctionTypeDefinition { parameters, returns, error_return }` inside `TypeDefinition::Function`
- `src/compiler_frontend/public_interface/model.rs` stores success returns and optional error return directly on public functions and receiver methods
- trait definitions and conformance matching store typed requirement return slots
- `src/compiler_frontend/external_packages/definitions.rs` stores `returns` plus `error_return_type` on external definitions and specs
- generic materialisation, import projection, receiver catalogues and reactive metadata inspect current return vectors

## AST and terminality owners

- every `Expression` owns a semantic `TypeId`
- ordinary function, method and host call expressions carry result type IDs
- `src/compiler_frontend/ast/statements/body_expr_stmt.rs` parses standalone calls through the value-expression path
- `src/compiler_frontend/ast/statements/terminality.rs` owns ordinary function terminality and recognises false assertions
- `src/compiler_frontend/ast/statements/value_production/completeness.rs` separately owns branch exits and separately recognises false assertions
- current terminality treats loops conservatively
- static Bool specialisation lives under AST finalisation

## HIR and analysis owners

- `HirFunction` currently carries `return_type: TypeId`
- HIR declaration lowering manufactures unit, tuple and fallible-carrier return types
- ordinary calls are `HirStatementKind::Call { target, args, result }`
- `HirTerminator` owns branches, returns and unrecoverable failures but has no call terminator
- HIR validation, display, remapping, reachability, borrow transfer, call summaries, problem extraction and backend validation exhaustively match current terminators
- result and lifetime summaries assume normal exit/result vocabulary even when a body currently only fails

## Backend owners

- JavaScript emits ordinary calls inside blocks and terminators through the dispatcher path
- current Wasm LIR has a result-less call statement and a `Trap` terminator that emits `unreachable`
- the current Wasm path can lower a HIR Never call to a result-less call followed by trap without redesigning the whole LIR
- the later structured Wasm implementation must preserve the semantic call plus unreachable fallback

# Scope

This plan owns:

- `-> !` syntax and diagnostics
- callable return-contract vocabulary through syntax, AST, existing function-type metadata, public interfaces, traits, generics, external packages and HIR
- exact callable compatibility and fingerprints
- standalone-only Never call parsing and AST representation
- shared exit and divergence analysis
- unannotated all-path-divergence diagnostics
- declared Never body validation
- true-loop proof with nearest-loop break handling
- HIR Never function contracts and Never-call terminators
- HIR validation, display, remapping and CFG utilities
- borrow, lifetime, call-summary, reachability and link-fact integration
- generated-sidecar integration
- Boracle problem extraction and operational representation
- JavaScript and current Wasm lowering
- explicit binding-backed and annotated external JavaScript Never contracts
- canonical language and compiler documentation
- progress matrix and roadmap status
- the downstream Wasm prerequisite and lowering contract
- generated documentation rebuild

# Non-goals

Do not add:

- a Never `TypeId`, `TypeDefinition`, builtin, `DataType`, `ParsedTypeRef` or canonical type identity
- a source keyword or named `Never` type
- `None` as divergence syntax
- bottom-type subtyping or coercion
- values whose type is Never
- expression-position Never calls
- mixed Never and ordinary/error return slots
- implicit no-return effects inferred from ordinary callable bodies at call sites
- source-visible panic expressions
- a replacement for statement-only `assert`
- recursion/SCC divergence proof in the first implementation
- arbitrary loop-invariant or whole-program termination analysis
- labelled loop control
- backend-specific source syntax
- compatibility shims for old return-vector or HIR return-type APIs

# General implementation rules

- Read the active worktree, not a cached repository snapshot.
- Preserve user-authored local changes and classify active worker branches before editing.
- Keep one callable return-contract owner per compiler layer.
- Use enums so invalid mixed states are unrepresentable.
- Do not add a Never type definition or builtin to `TypeEnvironment`. Update only the existing function-type signature payload where callable function types need the return contract.
- Keep one call-shaped parser and one parameter-slot routing owner.
- Parse and resolve arguments once, then construct either a value call or terminating statement from the resolved callable contract and source context.
- Do not let HIR, analyses, link planning or backends infer Never from an empty return vector, unit type, missing result local or all-failure body.
- Do not let source validity depend on inlining or ordinary callee implementation inspection.
- Delete replaced fields, vector-only assumptions and duplicate terminality helpers during the cutover.
- Use `CompilerDiagnostic` for malformed source and `CompilerError` for impossible post-AST/HIR states.
- Keep tests outside production files.
- Prefer integration cases for source-visible behaviour and focused unit tests for hidden invariants.
- Do not edit `docs/release/**` directly.
- Keep progress status truthful. The feature remains `Deferred` before implementation and `Partial` after core implementation because proof gaps remain.

# Mandatory phase completion protocol

Every phase is a stable accepted checkpoint. Before the next phase starts:

1. Re-read the affected authority sections plus the full style guide, testing guide and validation guide.
2. Review changed modules from their owning entry points.
3. Run a read-only phase audit for architecture ownership, semantic gaps, stale paths, diagnostics and test quality.
4. Resolve every actionable finding and rerun affected tests.
5. Perform the complete `AGENTS.md` Slice review.
6. Run targeted phase validation.
7. Run `just validate` for every code-bearing phase.
8. Run `just boracle` whenever HIR terminators, borrow problem extraction or Boracle semantics change.
9. Run `git diff --check` and inspect `git status --short`.
10. Record exact results in working notes and commit one coherent checkpoint.

A phase is not accepted while its mandatory read-only audit is unavailable or has open actionable findings. Record the blocker instead of claiming completion.

# Phase 0 - Refresh owners and baseline

## Goal

Re-anchor the plan after runtime anonymous records land. Produce a complete current owner and test inventory before semantic edits.

## Work

- [ ] Read every required authority from the active worktree.
- [ ] Record HEAD, branch, status and worktrees in untracked working notes.
- [ ] Inventory active workers and local changes touching this surface.
- [ ] Reconfirm runtime anonymous records are complete and the queued Wasm implementation has not started.
- [ ] Inventory every direct return-vector, error-return and HIR return-type consumer.
- [ ] Inventory `FunctionTypeDefinition` interning, equality, hashing, substitution, display and canonical projection.
- [ ] Inventory every call constructor and statement-expression entry point.
- [ ] Inventory terminality, branch-exit, false-assert and loop-exit classifiers.
- [ ] Inventory every exhaustive `HirTerminator` match across HIR, analyses, Boracle, JS and Wasm.
- [ ] Inventory current integration contracts and primary test ownership.
- [ ] Record baseline counts and unrelated failures without weakening gates.

Suggested searches:

```bash
rg -n 'FunctionSignatureSyntax|ReturnSlotSyntax|FunctionSignature|ReturnChannel' src tests
rg -n 'FunctionTypeDefinition|TypeDefinition::Function' src tests
rg -n 'PublicFunctionSemantics|PublicReceiverMethodSemantics|PublicTraitRequirement' src tests
rg -n 'ExternalFunctionDef|ExternalFunctionSpec|error_return_type|\.returns\b' src tests
rg -n 'HirFunction|return_type|HirTerminator|HirStatementKind::Call' src tests
rg -n 'BranchExitSummary|terminality|statically_false|assert_condition_is' src tests
rg -n 'ExpressionKind::FunctionCall|ExpressionKind::MethodCall|HostFunctionCall' src tests
rg -n 'DispatcherLoop|WasmLirTerminator::Trap|emit_call_statement' src/backends
```

## Audit

- [ ] Confirm the inventory follows semantic ownership rather than directory names.
- [ ] Confirm no newer return-contract or control-flow analysis already supersedes reviewed paths.
- [ ] Confirm every HIR terminator consumer is listed.
- [ ] Confirm test ownership and matrix wording reflect the live tree.
- [ ] Complete the mandatory phase audit and Slice review.

## Validation

```bash
cargo fmt --all -- --check
cargo test --workspace --quiet -- --format terse
cargo run --quiet -- tests --audit
just boracle
just validate
```

- [ ] Record exact results.
- [ ] Separate pre-existing failures from task-created failures.
- [ ] Commit only if the plan needs factual corrections.

# Phase 1 - Introduce callable return-contract data models

## Goal

Replace vector-only return state with explicit `Returns` versus `Never` vocabulary through every semantic layer while preserving current ordinary source behaviour. Public `-> !` syntax remains inactive until the representation is complete.

## Work

### Neutral and AST signatures

- [ ] Add neutral and resolved return-contract enums.
- [ ] Move existing ordinary slots into `Returns`.
- [ ] Preserve current success/error rules inside `Returns`.
- [ ] Add named queries for success slots, error slot, ordinary no-value completion and Never.
- [ ] Make ordinary-return queries reject or explicitly handle Never rather than returning empty collections.
- [ ] Thread the shape through declaration shells, remapping, rebinding, ordering hints and signature resolution.

### Function types

- [ ] Replace `FunctionTypeDefinition.returns` plus `error_return` with a function-type return contract.
- [ ] Keep the containing function type's ordinary `TypeId`.
- [ ] Include the return-contract variant in function-type interning, equality, hashing and cache keys.
- [ ] Preserve it through generic substitution and inherited generated environments.
- [ ] Render Never function signatures without inventing a type name.
- [ ] Include it in canonical function-type projection if the current interface uses one.
- [ ] Add named function-value tests for exact Never signature identity.

### Public, trait and generated vocabulary

- [ ] Add explicit public function and receiver-method return contracts.
- [ ] Add explicit trait requirement return contracts.
- [ ] Add explicit external function return contracts.
- [ ] Thread ordinary `Returns` through public projection, import projection, generic templates, generated materialisation, receiver catalogues and conformance matching.
- [ ] Include the variant in equality and fingerprint inputs.

### HIR function metadata

- [ ] Replace `HirFunction::return_type: TypeId` with `HirFunctionReturnContract`.
- [ ] Thread `Returns(TypeId)` through HIR registration, validation, display, fixtures and backend ABI queries.
- [ ] Add a dormant `Never` variant without mapping it to `None`.
- [ ] Remove the direct field and ambiguous compatibility accessors.

### External metadata

- [ ] Replace top-level `returns` plus `error_return_type` state with one external return contract.
- [ ] Preserve readable builder construction for ordinary functions without parallel legacy constructors.
- [ ] Make return aliases impossible on Never.
- [ ] Update registry, clone accounting, provider conversion and tests.

## Audit

- [ ] Confirm every layer uses an enum and no duplicate boolean.
- [ ] Confirm `TypeEnvironment` gained no Never type definition. Any change is confined to existing function-signature payloads.
- [ ] Confirm ordinary no-value functions remain ordinary `Returns`.
- [ ] Confirm all old direct return fields are cut over or deleted.
- [ ] Confirm fixtures construct the real shape.
- [ ] Complete the mandatory phase audit and Slice review.

## Validation

Run focused signature, datatype, public-interface, trait, external-package, HIR and backend tests, then:

```bash
cargo fmt --all
cargo test --workspace --quiet -- --format terse
cargo run --quiet -- tests --audit
just validate
```

- [ ] Confirm ordinary source behaviour is unchanged.
- [ ] Confirm benchmark history is unchanged.

# Phase 2 - Consolidate control-flow exits

## Goal

Replace duplicate terminality logic with one pure AST exit analysis. Add true-loop and divergence facts before public Never syntax depends on them.

## Work

### Shared analysis

- [ ] Add one focused AST control-flow exit module.
- [ ] Represent fallthrough, produced values, caller returns, divergence and nearest-loop exits independently.
- [ ] Implement deterministic branch union and reachable statement sequencing.
- [ ] Stop sequencing once no path can reach the next statement.
- [ ] Classify `return` and `return!` as caller returns.
- [ ] Classify normalised false assertions as divergence.
- [ ] Keep dynamic assertions fallthrough-capable.
- [ ] Preserve lexical scope.

### Branches and matches

- [ ] Preserve statement `if` terminality.
- [ ] Preserve value-producing `if`, match and catch completeness.
- [ ] Preserve exhaustiveness and required-default rules.
- [ ] Preserve the at-least-one-producing-path rule.
- [ ] Use specialised active AST only for terminality and divergence.

### Loops

- [ ] Add nearest-loop break and continue accounting without labels.
- [ ] Treat a normalised-true conditional loop with no reachable break targeting itself as non-fallthrough.
- [ ] Convert body fallthrough and continue paths into divergence at that loop boundary.
- [ ] Preserve caller returns and existing divergence through the loop.
- [ ] Let a reachable self-targeting break expose the post-loop path.
- [ ] Consume nested-loop exits at the nested boundary.
- [ ] Keep collection and range loops conservative.

### Existing consumers

- [ ] Rewrite ordinary function terminality to query the shared summary.
- [ ] Rewrite value-production completeness and reachable `then` traversal to use the shared owner.
- [ ] Delete duplicate false-assert and branch-terminal classifiers.
- [ ] Preserve current diagnostics unless precision requires a better location.
- [ ] Add pure summary tests for union, sequencing, nested loops, mixed return/diverge paths and unreachable tails.

## Audit

- [ ] Confirm there is one recursive exit-analysis owner.
- [ ] Confirm caller return and divergence remain distinct.
- [ ] Confirm diagnostic policy stays in consumers.
- [ ] Confirm nested `break` cannot escape the nearest loop.
- [ ] Confirm folded Bool authority is reused.
- [ ] Confirm unreachable statements do not affect summaries or produced-type inference.
- [ ] Complete the mandatory phase audit and Slice review.

## Validation

```bash
cargo fmt --all
cargo test --workspace --quiet terminality -- --format terse
cargo test --workspace --quiet value_production -- --format terse
cargo run --quiet -- tests --tag branching --backend html
cargo run --quiet -- tests --tag loops --backend html
cargo run --quiet -- tests --audit
just validate
```

- [ ] Preserve existing results except the approved true-loop terminality improvement.
- [ ] Do not expose `-> !` before this phase is accepted.

# Phase 3 - Add `-> !` syntax and frontend semantics

## Goal

Activate complete source syntax, body validation, exact interfaces and standalone-only call construction without manufacturing a value.

## Work

### Signature syntax

- [ ] Parse bare `!` immediately after `->` as Never.
- [ ] Require the existing function body colon after Never.
- [ ] Support bodyless trait requirement `-> !` through shared signature parsing.
- [ ] Reject commas, another `!`, following types, optional suffixes and error slots after Never.
- [ ] Reject bare `!` in parameters, fields, aliases and ordinary type positions.
- [ ] Keep `T!` as an ordinary final error slot.
- [ ] Add valid and malformed syntax tests.

### Function validation

- [ ] Resolve Never without interning a type.
- [ ] Validate Never bodies with shared active-AST exit facts.
- [ ] Reject fallthrough and every caller-returning exit.
- [ ] Keep normal frontend checking in inactive branches before specialisation.
- [ ] Diagnose all-path-divergent unannotated user functions with both contract choices.
- [ ] Exclude compiler-synthesised `start` from that explicitness diagnostic.
- [ ] Accept explicitly typed placeholder implementations that diverge.
- [ ] Prevent same-cycle Never calls from proving one another in the first implementation.

### Call construction and placement

- [ ] Add an AST statement shape for a resolved Never call. It carries target or receiver plus routed arguments but no `Expression` or `TypeId`.
- [ ] Refactor call construction to return a named value-call versus terminating-call outcome.
- [ ] Parse and validate arguments once.
- [ ] Let statement context accept the terminating outcome.
- [ ] Let every value context reject it with the targeted placement diagnostic.
- [ ] Cover free functions, methods, named function values, generic calls, imported calls and binding-backed calls through the same route.
- [ ] Preserve named arguments, defaults, mutable access and generic request emission.
- [ ] Reject postfix handling and `catch`.
- [ ] Reject Never calls in assertion arguments.
- [ ] End reachable AST statement sequencing after a Never call.

### Public interfaces and exact matching

- [ ] Project Never through exported functions and methods.
- [ ] Project Never through trait requirements with no fake type.
- [ ] Make conformance and function-type matching exact on the contract variant.
- [ ] Import Never contracts without donor-local state.
- [ ] Include the variant in interface equality and fingerprints.

### Generics

- [ ] Retain Never on generic templates.
- [ ] Infer Never-call type arguments from immediate arguments and evidence only. There is no expected result context.
- [ ] Emit concrete requests from active standalone calls.
- [ ] Preserve inactive static-branch request discard.
- [ ] Preserve Never on generated concrete signatures.
- [ ] Reject generic declarations that cannot meet existing parameter-use inference rules.

### Binding-backed contracts

- [ ] Permit explicit Never in external metadata.
- [ ] Extend `@moth.sig` parsing to accept a whole `-> !` contract.
- [ ] Reject Never metadata mixed with success returns, error returns or aliases.
- [ ] Require explicit provider metadata. Do not inspect foreign bodies.

## Audit

- [ ] Confirm bare `!` is owned only by callable return syntax.
- [ ] Confirm one call parser serves value and Never calls.
- [ ] Confirm Never calls never enter `ExpressionKind` and never get a result type.
- [ ] Confirm explicitness diagnostics distinguish permanent intent from temporary placeholders.
- [ ] Confirm only explicit contracts propagate divergence.
- [ ] Confirm trait, function-type and public matching are exact.
- [ ] Confirm generic requests come only from active standalone calls.
- [ ] Complete the mandatory phase audit and Slice review.

## Validation

Run focused parser, function, call, datatype, trait, generic, interface and external-package tests, then:

```bash
cargo fmt --all
cargo test --workspace --quiet function_parsing -- --format terse
cargo test --workspace --quiet trait -- --format terse
cargo test --workspace --quiet public_interface -- --format terse
cargo test --workspace --quiet external_packages -- --format terse
cargo run --quiet -- tests --audit
just validate
```

- [ ] Add integration diagnostics for malformed signatures, invalid bodies and value-position calls.
- [ ] Keep progress status deferred until HIR and backend work lands.

# Phase 4 - Lower Never through HIR

## Goal

Give HIR an explicit Never function contract and dedicated terminating call with ordinary target identity, argument evaluation and CFG invariants.

## Work

### HIR functions

- [ ] Lower ordinary AST returns to `HirFunctionReturnContract::Returns`.
- [ ] Lower Never to `HirFunctionReturnContract::Never`.
- [ ] Emit no implicit return for Never functions.
- [ ] Treat surviving Never fallthrough as `CompilerError` because AST should diagnose it.
- [ ] Reject return terminators in Never functions during HIR validation.
- [ ] Keep ordinary unit functions distinct.

### NeverCall

- [ ] Add the dedicated HIR terminator.
- [ ] Reuse ordinary target resolution and argument lowering.
- [ ] Evaluate receiver and arguments once in current left-to-right order.
- [ ] Preserve fresh-rvalue materialisation for mutable arguments.
- [ ] Emit no result local and no continuation block.
- [ ] Stop lowering later statements on the current path.
- [ ] Map the authored call location into side tables.
- [ ] Remap arguments and target-owned string identities correctly.

### Validation and utilities

- [ ] Validate arguments like ordinary calls.
- [ ] Resolve the target contract from local HIR, generated sidecars, public interfaces or binding metadata.
- [ ] Reject NeverCall targeting an ordinary function as `CompilerError`.
- [ ] Reject ordinary value/statement call IR targeting explicit Never as `CompilerError`.
- [ ] Mark NeverCall as a no-successor terminator.
- [ ] Update block validation, reachability, display, debug views, remapping, source mapping and structured-HIR derivation.
- [ ] Require explicit HIR return contracts in fixtures.

### Link facts

- [ ] Record NeverCall as a normal call edge.
- [ ] Collect argument resource, project-context, capability, reactive and target facts once.
- [ ] Preserve deterministic source order.
- [ ] Emit no result provenance or result resource facts.

## Audit

- [ ] Confirm HIR has one semantic operation rather than call plus source failure.
- [ ] Confirm target contract comes from explicit metadata.
- [ ] Confirm no result local, unit placeholder or Never TypeId exists.
- [ ] Confirm successor utilities and unreachable-tail lowering agree.
- [ ] Confirm argument order and side-table locations match ordinary calls.
- [ ] Confirm every exhaustive terminator match was intentionally updated.
- [ ] Complete the mandatory phase audit and Slice review.

## Validation

```bash
cargo fmt --all
cargo test --workspace --quiet hir -- --format terse
cargo test --workspace --quiet reachability -- --format terse
cargo run --quiet -- tests --audit
just boracle
just validate
```

- [ ] Confirm no uninitialised terminator or dead continuation block remains.
- [ ] Keep malformed HIR fixture failures on `CompilerError`.

# Phase 5 - Integrate analyses, summaries and generated work

## Goal

Teach every downstream semantic consumer that NeverCall is a terminating call with ordinary argument effects and no result or successor.

## Work

### Borrow validation

- [ ] Generate shared or exclusive argument accesses through existing parameter metadata.
- [ ] Apply mutation and optional final-use transfer effects before divergence where the call contract requires them.
- [ ] Create no result local, result origin, return alias or post-call state.
- [ ] End liveness at the terminator using existing block-exit rules.
- [ ] Preserve reactive invalidation and argument effects.
- [ ] Update use and metadata collectors.

### Lifetime and escape analysis

- [ ] Record argument and retained-edge effects before divergence.
- [ ] Emit no result family, provenance or detached result.
- [ ] Represent absence of normal exit explicitly where summaries need it.
- [ ] Ensure link-level summary instantiation expects no continuation.
- [ ] Preserve cleanup that occurs before the call under the validated memory plan.
- [ ] Add no Never-specific ownership mode.

### Public call summaries

- [ ] Keep parameter access and mutation summaries for Never callables.
- [ ] Represent normal-return/result state as absent through the explicit contract.
- [ ] Never use an empty result vector as unit/Never ambiguity.
- [ ] Keep generated summaries and public interfaces aligned.

### Reachability and generated functions

- [ ] Traverse NeverCall targets as ordinary call edges.
- [ ] Include generated targets requested by standalone Never calls.
- [ ] Include binding-backed imports and helper requirements.
- [ ] Traverse argument expressions for nested ordinary calls and all link facts.
- [ ] Do not traverse a nonexistent successor.

### Boracle

- [ ] Update normalized problem extraction for NeverCall.
- [ ] Preserve call argument access and call-effect events before terminal exit.
- [ ] Add a terminal representation distinct from normal return.
- [ ] Update validation, the reference solver, operational oracle, rendering, reducers and generated problem support where exhaustive.
- [ ] Keep production and oracle event ordering aligned.
- [ ] Add focused shared/mutable argument and no-successor fixtures.

## Audit

- [ ] Confirm argument effects are neither dropped nor duplicated.
- [ ] Confirm no result facts or post-call state exist.
- [ ] Confirm summaries distinguish Never from unit.
- [ ] Confirm reachability includes callee and argument dependencies.
- [ ] Confirm analyses consume HIR without source inspection.
- [ ] Confirm Boracle models call-before-divergence ordering.
- [ ] Complete the mandatory phase audit and Slice review.

## Validation

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

- [ ] Confirm no analysis accepts a normal successor after NeverCall.

# Phase 6 - Lower JavaScript and current Wasm

## Goal

Support Never on every current target before the larger Wasm rewrite. Make continuation impossible when a declared Never callee violates its runtime contract.

## Work

### JavaScript

- [ ] Emit Never functions with no source-visible return value.
- [ ] Lower NeverCall through existing target and argument call emission.
- [ ] Emit the call exactly once.
- [ ] Immediately emit a compiler-owned unrecoverable throw if control returns.
- [ ] Use a message that identifies a violated `-> !` contract without internal IDs.
- [ ] Preserve external-module imports and glue.
- [ ] Preserve argument effects and reactive invalidations before the call.
- [ ] Ensure dispatcher control cannot continue afterward.

Conceptual output:

```javascript
fatal(message);
throw new Error("Moth `-> !` function returned unexpectedly");
```

The exact helper may reuse the existing runtime-failure owner.

### Current Wasm

- [ ] Map Never function contracts to zero Wasm results.
- [ ] Lower NeverCall to a result-less call followed by the existing LIR trap path.
- [ ] Preserve target identity and argument lowering.
- [ ] Emit Wasm `unreachable` after every Never call, including imports.
- [ ] Reject unsupported targets during target validation rather than dropping calls.
- [ ] Add artefact tests proving call then unreachable order.
- [ ] Do not redesign the full Wasm LIR here.

### Target validation

- [ ] Inspect NeverCall arguments and targets like ordinary calls.
- [ ] Keep unsupported binding/feature diagnostics at the authored call.
- [ ] Do not reject Never merely because it has no result type.
- [ ] Require an accepted backend to express the defensive unreachable fallback.

### Downstream Wasm plan

- [ ] Update the queued HTML mixed JavaScript and Wasm plan's named prerequisites to require delivered Never function contracts and NeverCall HIR.
- [ ] Require future structured Wasm LIR to preserve call plus unreachable semantics.
- [ ] Retain local, cross-module, generated and binding-backed Never coverage.
- [ ] Point both plans to permanent compiler documentation rather than making this temporary plan an authority.

## Audit

- [ ] Confirm both backends consume explicit HIR facts rather than result absence.
- [ ] Confirm a returning external implementation cannot reach code HIR marked unreachable.
- [ ] Confirm fallback occurs after the call.
- [ ] Confirm target selection and external glue stay in existing owners.
- [ ] Confirm no broad Wasm redesign or compatibility adapter entered this phase.
- [ ] Complete the mandatory phase audit and Slice review.

## Validation

```bash
cargo fmt --all
cargo test --workspace --quiet js -- --format terse
cargo test --workspace --quiet wasm -- --format terse
cargo run --quiet -- tests --tag functions --backend html
cargo run --quiet -- tests --tag functions --backend html_wasm
cargo run --quiet -- tests --audit
just validate
```

- [ ] Prove JS throws if a test external Never function returns.
- [ ] Prove emitted Wasm validates and contains unreachable.
- [ ] Confirm ordinary unit calls retain previous lowering.

# Phase 7 - Complete integration coverage

## Goal

Prove the full contract across local, cross-module, function-value, generic, trait, external and control-flow boundaries with one primary owner per behaviour.

## Required valid coverage

- [ ] local Never function ending in `assert(false)`
- [ ] Never receiver method
- [ ] named function value whose signature preserves exact Never
- [ ] exported Never function imported across a module boundary
- [ ] generic Never function inferred from arguments
- [ ] generated NeverCall reachability and backend lowering
- [ ] trait requirement `-> !` with exact conformance
- [ ] annotated external JS Never contract
- [ ] normalised-true loop with no self-targeting break
- [ ] nested inner-loop break that does not invalidate an outer true loop
- [ ] block-form value producer with one `then` branch and one Never-call branch
- [ ] ordinary typed placeholder whose body only asserts false

## Required invalid signature coverage

- [ ] `-> !, String`
- [ ] `-> String, !`
- [ ] `-> Error!, !`
- [ ] `-> !!`
- [ ] bare `!` in parameters, fields, aliases and collection types
- [ ] attempted `-> None` retains its existing invalid-type rule

## Required invalid Never body coverage

- [ ] fallthrough
- [ ] bare `return`
- [ ] `return!`
- [ ] option/error propagation to caller
- [ ] one divergent branch and one completing branch
- [ ] true loop with self-targeting reachable break
- [ ] direct or mutual recursive proof cycle reports the accepted deferred-proof diagnostic

## Required invalid call-placement coverage

- [ ] declaration initializer and assignment RHS
- [ ] return value
- [ ] function, receiver and constructor argument
- [ ] operator operand and condition
- [ ] template interpolation
- [ ] collection and map element
- [ ] assertion condition and message
- [ ] inline value-producing branch
- [ ] postfix `!`, postfix `?` and `catch`

## Required explicitness coverage

- [ ] unannotated all-path false assertion
- [ ] unannotated normalised-true loop
- [ ] unannotated branches that all diverge
- [ ] diagnostic offers permanent Never and temporary ordinary-type choices
- [ ] explicitly typed equivalent is accepted
- [ ] unannotated function that may complete stays ordinary no-value
- [ ] compiler-synthesised `start` may diverge without source annotation

## Required exact-interface coverage

- [ ] trait requires Never but implementation declares ordinary returns
- [ ] trait requires ordinary returns but implementation declares Never
- [ ] function-type matching distinguishes the contracts
- [ ] public fingerprint changes when the contract changes
- [ ] same-file and cross-module callers do not infer divergence from ordinary typed callee bodies
- [ ] external function with no returns remains ordinary unit unless explicitly Never

## Test ownership

- Put user-visible syntax, diagnostics and runtime behaviour under `tests/cases/`.
- Give the Never surface one primary contract and distinct boundary/adversarial cases.
- Put pure exit facts under the AST analysis tests.
- Put HIR invariants under `src/compiler_frontend/hir/tests/`.
- Put datatype, interface, trait, external and generated invariants under their owners.
- Put backend artefact assertions under JS and Wasm tests.
- Keep Boracle semantics in its opt-in tree.
- Do not use benchmarks as correctness evidence.
- Remove or rewrite tests that encode obsolete direct return fields or duplicate terminality.

## Audit

- [ ] Map each behaviour to one primary test owner.
- [ ] Remove redundant fixtures.
- [ ] Assert stable diagnostic codes, reasons and source locations.
- [ ] Prove defensive fallback where observable.
- [ ] Avoid incidental block-ID and formatting contracts.
- [ ] Cover all callable surfaces and deferred proof failures.
- [ ] Complete the mandatory phase audit and Slice review.

## Validation

```bash
cargo fmt --all
cargo run --quiet -- tests --contract language.functions.never_return_contract
cargo run --quiet -- tests --tag functions --backend html
cargo run --quiet -- tests --tag functions --backend html_wasm
cargo run --quiet -- tests --audit
just boracle
just validate
```

Use the final contract ID selected by current manifest conventions. Do not create duplicate primary ownership.

# Phase 8 - Update permanent docs, progress and roadmap

## Goal

Move the final semantics into permanent authorities, report implemented support accurately and keep every accepted proof extension visible as future work.

This phase is documentation-only unless review exposes an implementation defect.

## Language documentation

- [ ] Update function declarations with `-> !` as a whole callable contract.
- [ ] Update returns docs to distinguish ordinary no-value completion, typed placeholders and Never.
- [ ] Update calls docs with standalone-only placement.
- [ ] Update assertions docs to explain that false assertions prove divergence but are not Never values.
- [ ] Update value-producing block docs with producing/diverging block form and invalid inline form.
- [ ] Update loop docs with the normalised-true/no-break proof boundary.
- [ ] Update trait docs with exact matching.
- [ ] Update generic docs with argument-only inference for Never calls.
- [ ] Update external binding docs with explicit `@moth.sig ... -> !`.
- [ ] Update the cheatsheet.
- [ ] Update Basic pages only where the concept belongs at that level.

## Compiler architecture

Update `docs/compiler-design-overview.md` with durable ownership for:

- callable return contracts separate from value type identity
- function types that carry Never signatures without a Never value type
- the absence of a Never `TypeId`
- shared AST exit analysis
- specialised active-AST validation
- standalone-only Never calls
- exact public, trait, generic and external contracts
- HIR function return contracts and `NeverCall`
- borrow, lifetime, reachability and link treatment
- backend call plus unreachable fallback

Review `docs/build-system-design.md`. Edit only if target validation, linking or the Wasm handoff lacks a durable boundary. Do not copy language semantics into build-system prose.

Review `index.md` and update it only if owners or paths changed.

## Progress matrix

Add or update one focused **Never return contracts** row in `docs/src/docs/progress/@page.moth`.

Before implementation, an accepted-design row may be `Deferred` with coverage `None`. After this plan lands, set it to `Partial`, not `Supported`, because accepted proof gaps remain.

The implemented row must state:

- source syntax is `-> !`
- it is a whole callable contract, not a first-class type or Never `TypeId`
- calls are standalone terminating statements and cannot be consumed as values
- explicit contracts propagate through local, function-value, cross-module, generic, trait and binding-backed surfaces
- core proof covers false assertions, explicit Never calls, normalised-true no-break loops, lexical scopes, branches and exhaustive matches
- HIR uses explicit Never function contracts and terminating calls
- JS and current Wasm emit a defensive unreachable fallback
- ordinary typed functions may currently diverge without becoming Never to callers

The same row must list every accepted proof gap to close later:

- direct and mutual recursion proof for declared Never bodies
- richer data-flow proof of permanently true runtime loop conditions
- broader deterministic structural, SCC or whole-program non-termination proof

Do not list first-class Never values, bottom coercion, value-position calls or implicit ordinary-call divergence as gaps. Those are deliberate exclusions.

Update existing Assertions, Functions and calls, Control flow, Traits, Generic functions and binding-backed package notes only where they would otherwise contradict the new row. Avoid repeating the whole contract across several rows.

## Roadmap

While queued, keep this plan immediately after runtime anonymous records and before the Wasm implementation.

At closeout:

- [ ] delete this plan and remove its queued entry in the same commit
- [ ] keep the Wasm implementation after the delivered capability
- [ ] add a `Never return proof follow-ups` subsection under deferred design if no focused owner exists
- [ ] list recursion proof, richer loop-condition proof and broader deterministic non-termination proof
- [ ] do not add final constraints as deferred work
- [ ] keep the same proof gaps visible in the progress matrix

## Downstream Wasm plan

Update its named prerequisites and required final design so it consumes:

- `HirFunctionReturnContract::Never`
- `HirTerminator::NeverCall`
- zero-result ABI for Never functions
- call plus defensive unreachable semantics
- local, cross-module, generated and binding-backed coverage

Do not cite this temporary plan as a semantic authority.

## Generated docs

- [ ] Check docs with the current compiler.
- [ ] Rebuild `docs/release/**` through the compiler.
- [ ] Review generated syntax highlighting, links and examples.
- [ ] Do not hand-edit generated output.

## Audit

- [ ] Confirm permanent docs own final semantics.
- [ ] Confirm matrix status is `Partial` after implementation and names every accepted proof gap.
- [ ] Confirm exclusions are not misreported as future promises.
- [ ] Confirm roadmap order and downstream Wasm prerequisites are correct.
- [ ] Confirm generated docs came from source.
- [ ] Complete the mandatory documentation audit and Slice review.

## Validation

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

# Phase 9 - Delete stale paths and complete final review

## Goal

Prove there is one final Never return path, no fake type/value representation and no stale vector-only or duplicate terminality assumptions.

## Stale-path searches

```bash
rg -n 'HirFunction\s*\{[^}]*return_type|\.return_type\b' src tests
rg -n 'FunctionSignature\s*\{[^}]*returns|FunctionSignatureSyntax\s*\{[^}]*returns' src tests
rg -n 'FunctionTypeDefinition\s*\{[^}]*returns|FunctionTypeDefinition\s*\{[^}]*error_return' src tests
rg -n 'PublicFunctionSemantics.*returns|PublicReceiverMethodSemantics.*returns' src tests
rg -n 'error_return_type' src tests
rg -n 'assert_condition_is_statically_false|statically_false.*assert|BranchExitSummary::TERMINATES' src tests
rg -n 'Never.*TypeId|TypeId.*Never|DataType::Never|ParsedTypeRef::Never|CanonicalTypeIdentity::Never' src tests docs --glob '!docs/release/**'
rg -n 'NeverCall|HirTerminator::NeverCall|FunctionReturnContract' src tests docs --glob '!docs/release/**'
```

Expected results:

- no old direct HIR return field remains
- no vector-only contract remains where Never is legal
- no duplicated false-assert terminality classifier remains
- no Never type identity exists
- every NeverCall match is intentional
- ordinary result-less calls remain distinct
- current and future Wasm paths preserve call plus unreachable

## Final architecture audit

Give the read-only final auditor:

- this plan
- the complete diff
- accepted interview decisions
- phase findings and resolutions
- exact validation results
- permanent docs
- progress and roadmap changes

The audit must check:

- [ ] syntax and signature exclusivity
- [ ] no first-class Never or bottom coercion
- [ ] standalone-only placement
- [ ] explicit-only interprocedural propagation
- [ ] typed placeholder behaviour
- [ ] unannotated divergence diagnostics
- [ ] shared exit ownership
- [ ] true-loop and nested-break correctness
- [ ] exact function-type, trait, generic, public and external contracts
- [ ] HIR and NeverCall invariants
- [ ] borrow, lifetime, reachability and generated facts
- [ ] Boracle parity
- [ ] JS and Wasm fallback
- [ ] no compatibility or stale path
- [ ] test ownership
- [ ] permanent documentation authority
- [ ] matrix gaps and roadmap order
- [ ] style-guide compliance

Resolve every actionable finding, rerun affected gates and obtain a fresh clean final audit.

## Final validation

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
- [ ] Confirm benchmark history changed only with separate justification.
- [ ] Confirm status contains only intended files.
- [ ] Delete this plan and its roadmap entry in the same completion commit.

# Validation summary

| Phase | Change class | Required final gate |
|---|---|---|
| Phase 0 | read-only or plan correction | baseline commands, then the gate matching any tracked edit |
| Phase 1 | code-bearing | focused data-model tests plus `just validate` |
| Phase 2 | code-bearing | control-flow tests plus `just validate` |
| Phase 3 | code-bearing | frontend/interface tests plus `just validate` |
| Phase 4 | code-bearing and HIR | HIR tests, `just boracle` and `just validate` |
| Phase 5 | code-bearing and analysis | analysis tests, `just boracle` and `just validate` |
| Phase 6 | code-bearing and backend | JS/Wasm tests plus `just validate` |
| Phase 7 | code-bearing integration | canonical suite, `just boracle` and `just validate` |
| Phase 8 | docs-only unless defects surface | docs release build, or `just validate` when mixed |
| Phase 9 | whole-plan closeout | final audit, `just boracle`, `just validate` and docs release build |

# Definition of done

The plan is complete only when:

- [ ] `-> !` parses as one whole callable return contract
- [ ] Never cannot mix with success, optional or error return slots
- [ ] no Never value type, `TypeId`, `DataType`, parsed type or canonical type identity exists
- [ ] existing function types can encode a Never callable signature without creating a Never value
- [ ] Never works for functions, methods, function values, generics, traits, exports and binding-backed calls
- [ ] callable compatibility is exact
- [ ] only explicit `-> !` propagates divergence across call boundaries
- [ ] Never calls are standalone statements only
- [ ] value-producing blocks accept producing/diverging branch mixtures without coercion
- [ ] all-diverging value producers still require a producing path
- [ ] unannotated all-path divergence requires an explicit contract choice
- [ ] explicitly typed placeholder implementations may diverge
- [ ] Never bodies reject fallthrough and caller-returning exits
- [ ] false assertions, explicit Never calls and normalised-true no-break loops prove divergence
- [ ] nested loop break targeting is correct
- [ ] one AST exit analysis serves terminality, value production and Never validation
- [ ] HIR functions carry explicit return contracts
- [ ] HIR NeverCall has a target and arguments but no result or successor
- [ ] HIR validation checks the target contract
- [ ] borrow and lifetime analyses apply arguments with no result or post-call state
- [ ] reachability and link facts retain the call edge and argument facts
- [ ] generated sidecars preserve Never
- [ ] Boracle models call effects before terminal divergence
- [ ] JavaScript emits a defensive throw after the call
- [ ] current Wasm emits a result-less call followed by unreachable
- [ ] the downstream Wasm plan preserves the delivered contract
- [ ] canonical docs own final semantics
- [ ] the progress matrix reports implemented core support as `Partial`
- [ ] every accepted proof extension is visible in both progress matrix and roadmap
- [ ] deliberate exclusions are not presented as future work
- [ ] all mandatory phase audits and Slice reviews are complete
- [ ] `just boracle`, `just validate` and docs release build pass
- [ ] plan and roadmap entry are retired together
