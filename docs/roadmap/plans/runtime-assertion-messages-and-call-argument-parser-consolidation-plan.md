# Runtime Assertion Messages and Call-Argument Parser Consolidation Plan

## Current state

```text
WORK_ID: runtime-assertion-messages-call-arguments
WORK_SOURCE: docs/roadmap/plans/runtime-assertion-messages-and-call-argument-parser-consolidation-plan.md
BASE_REVISION: cb533ced7345dc7a66cf971e7590d88c8cd84f32
IMPLEMENTATION_START_REVISION: f1c0e0cf56cf8e65af1e5eed7859f967d516663a
STATUS: active (POST_PHASE3_EXTRA_AUDIT_2 accepted; Phase 4 pending)
CURRENT_SCOPE: checkpoint the accepted second extra audit cycle, then complete the remaining Wasm target-validation, diagnostics/documentation consolidation and final-audit phases
COMPLETED: current parser, AST, HIR, analysis, backend, diagnostic, documentation and integration-case audit; all five design decisions; Phase 0 tree re-anchor, baseline, regression reproduction and contract documentation; Phase 1 call-argument owner extraction, retained slot routing, consumer migration, metadata invariant check and focused slot-retention coverage; Phase 2 frontend/HIR assertion-message ownership, lazy failure CFG, target fact, analysis coverage, diagnostics, fixtures and status reconciliation; Phase 3 JavaScript lowering, target split, runtime local/function/optional/template/cast cases, named-argument success, lazy-success chronology and emitter coverage; normal audit pass 1 correction traverses AssertFailure messages for JS cast-helper, map and reactivity metadata and reconciles assertion/reactivity/cast documentation and fixture contracts; normal audit pass 2 corrections update the Basic assertion/reactivity pages, strengthen the subscription-backed template fixture, and add map/reactive JS emitter regressions
NEXT_ACTION: create the POST_PHASE3_EXTRA_AUDIT_2 checkpoint, then implement and validate Phase 4 Wasm target-validation hardening
VALIDATION: focused assertion-message tests 6 passed; static-false CFG assertion test 1 passed; borrow fact tests 14 passed; loaded borrow metadata 1 passed; backend feature validation 8 passed; JS emitter 5 passed; HIR validation 43 passed; reachability 14 passed; assertion HTML lane 28/28; template HTML/HTML-Wasm 2/2 with exact assertion-to-snapshot assignment and live-carrier absence checks; static-true HTML/HTML-Wasm case 2/2; runtime/multiline lazy-success 1/1; propagation diagnostic cases 1/1 each; shared call-argument tests 13; assertion-focused Rust tests 99; HIR branch-lowering tests 9; docs check no errors or warnings; docs release rebuilt 70 files; `just validate` -> passed with 4420 workspace tests, 17 xtask tests, 765 feature-lane tests, 1860 integration cases, source audit 1195 files/0 findings, benchmark preflight/quick measurements and timers-erasure (no-timer binary clean, 8166960 bytes); cargo fmt and git diff checks passed; prior snapshot mutation evidence remains recorded
AUDITS: Phase 0 ownership inventory covers call_argument.rs metadata, function_calls.rs parsing and provisional routing, call_validation.rs final routing/default/type/access validation, all call-shaped consumers, every NodeKind::Assert and HirTerminator::AssertFailure consumer, assertion diagnostics, 19 tagged assertion cases plus the separate Wasm boundary, and the constant/Wasm plan handoffs; Phase 1 pass 1 found and corrected the zero-argument builtin delimiter bypass and stale checklist state; pass 2 found and corrected retained-slot/generic-receiver coverage gaps; pass 2 verification required and received final-tree validation after the new test and fixture edits; final verification found and corrected stale ownership comments in the focused tests, generic nominal inference and call validation; the clean verification audit accepted Phase 1 with no findings; Phase 2 verification audit found required source-location, fixture, invariant-coverage and status-documentation corrections, with no new high-severity issue; coordinator corrections now preserve authored postfix locations, add CFG/target/borrow/TIR-handoff/recovery coverage, refresh the rejection fixture and reconcile current-support documentation; Phase 3 focused checks and full validation passed; normal auditor pass 1 found a required JS cast-helper discovery omission plus stale assertion/reactivity/cast documentation and fixture contract prose; normal pass-2 retry found stale Basic documentation and missing map/reactive metadata coverage; focused verification found one medium integration-contract gap: the subscription-backed fixture did not bind the assertion temporary to the snapshot call; correction now requires the exact snapshot assignment, forbids the direct live-carrier assignment, and was causally mutation-checked; full validation is green; the focused-verification retry audit is clean with no new required findings; POST_PHASE3_EXTRA_AUDIT_1 inspection found two medium coverage gaps: lazy-success used compile-time true and the exact multiline assertion had no end-to-end case; corrections now use a runtime condition and multiline `assert` in the lazy-success fixture and reconcile the current-state capsule; full validation is green; the POST_PHASE3_EXTRA_AUDIT_1 retry audit is clean with no new required findings; POST_PHASE3_EXTRA_AUDIT_2 inspection found two medium coverage gaps and one low stale-plan finding: real-source `!`/`?` assertion-message diagnostics were absent, compile-time-true runtime-message elision and Wasm acceptance were not independently covered, and the re-anchor capsule named stale owners/revision/next slice; corrections add two exact diagnostic fixtures, a static-true HIR/integration target case, and current retained-slot/status facts; full validation is green; the POST_PHASE3_EXTRA_AUDIT_2 retry audit is clean with no new required findings; both explicitly requested post-Phase-3 extra audit-and-correction cycles are now complete
BLOCKERS: none. If later work rebases the active frontend ownership cleanup, re-check the module-compilation handoff and preserve one parser and one retained slot route without compatibility re-exports.
NOTES: the nine pre-existing dirty files are accepted Phase 0 plan, architecture, canonical-reference and generated-release outputs; preserve them in the first coordinator checkpoint. The progress matrix must retain an explicit Wasm gap until dynamic assertion-message evaluation is implemented there. After Phase 3 completion, run and record two additional fresh audit-plus-correction cycles as POST_PHASE3_EXTRA_AUDIT_1 and POST_PHASE3_EXTRA_AUDIT_2 before the final audit.
```

## Phase 0 re-anchor record

```text
REVISION: accepted POST_PHASE3_EXTRA_AUDIT_1 checkpoint aafb611dd plus the POST_PHASE3_EXTRA_AUDIT_2 corrections in the working tree
BRANCH: codex/runtime-assertion-messages-call-arguments
WORKTREE: /Users/aneirinjames/projects/beanstalk/moth-runtime-assertion-messages

CALL_ARGUMENT_METADATA_OWNER: src/compiler_frontend/ast/expressions/call_argument.rs
CALL_ARGUMENT_PARSER_OWNER: src/compiler_frontend/ast/expressions/call_arguments.rs::parse_call_arguments_inner
PARSE_TIME_SLOT_ROUTER: src/compiler_frontend/ast/expressions/call_arguments.rs::ParameterSlotRouter
FINAL_SLOT_ROUTER: src/compiler_frontend/ast/expressions/call_argument.rs::order_call_arguments_by_retained_slot, consumed by call_validation.rs::resolve_call_arguments
FINAL_VALIDATION_OWNER: src/compiler_frontend/ast/expressions/call_validation.rs::resolve_call_arguments
CALL_SHAPED_CONSUMERS: source and host functions, generic functions, struct constructors, choice constructors, source receiver methods, compiler-owned builtin members and generic nominal inference

ASSERT_AST_OWNER: src/compiler_frontend/ast/statements/asserts.rs parses the reserved statement and owns suffix/placement rejection; src/compiler_frontend/ast/ast_nodes.rs owns typed NodeKind::Assert { condition, message }
ASSERT_HIR_OWNER: src/compiler_frontend/hir/hir_statement.rs lowers the typed condition/message expressions; src/compiler_frontend/hir/terminators.rs owns AssertFailure { message: HirExpression, message_evaluation: HirAssertionMessageEvaluation }
ASSERT_ANALYSIS_CONSUMERS: HIR validation, display, remapping, reachability, borrow transfer/metadata, utility traversal, backend feature validation, JavaScript structured/dispatcher lowering and Wasm trap lowering
ASSERT_AST_WALKERS: const_fact_collection.rs, debug_type_validation.rs, normalize_ast.rs, reactive_templates/annotation.rs, reactive_templates/flow.rs, validate_types.rs, terminality.rs and value_production/completeness.rs
ASSERT_TEST_CONSUMERS: AST terminality/value-production/fallible-handling tests, HIR display/reachability tests and borrow fact tests

ASSERT_DIAGNOSTIC_OWNER: InvalidFallibleHandlingReason::AssertionMessageCannotEscape in compiler_messages/diagnostic_payload/types.rs; reason key in reason_keys.rs; shared call diagnostics in compiler_messages/render/calls.rs
REMOVED_DIAGNOSTIC: RuntimeMessageExpressionDeferred and invalid_builtin_call.runtime_message_expression_deferred are deleted

ASSERTION_CASES: 28 cases selected by --tag assert on HTML, including shared-call diagnostics, real assertion-message propagation diagnostics, runtime local/function/optional/template/cast cases, static/runtime lazy-success chronology and terminality cases; separate keyword-shadow cases are assert_keyword_shadow_header_error and assert_keyword_shadow_body_error; the separate Wasm boundaries are assert_static_true_message_elision and assert_template_message_snapshot
MULTILINE_REGRESSION: resolved by the shared parser; tests/cases/assert_message_lazy_success/input/@page.moth now covers newlines after the opening parenthesis and comma with a runtime-computed true condition and side-effecting message; cargo run --quiet -- tests --terse --case assert_message_lazy_success -> 1/1 correct

FOCUSED_BASELINE: assertion-message tests 6 passed; static-false CFG 1; borrow facts 14; loaded borrow metadata 1; backend feature validation 8; JS emitter 5; HIR validation 43; reachability 14; assertion HTML lane 25/25; template HTML/HTML-Wasm 2/2; cast runtime case 1/1
FULL_BASELINE: just validate -> passed; 4419 workspace tests, 17 xtask tests, 765 feature-lane tests, 1856 integration cases, docs check, benchmark preflight/quick measurements, source audit 1195 files/0 findings and timer-erasure check all passed
DOCS_RELEASE_GATE: cargo run --quiet -- build docs --release -> passed, 70 output files; the first run emitted the existing recoverable missing-manifest stale-cleanup warning and preserved stale artefacts, and the final post-edit rerun passed with the generated manifest available
DOCS_AFTER_EDIT: canonical assertion reference, compiler architecture, progress matrix, cheatsheet, reactivity and cast-target references describe JavaScript/HTML runtime support and the explicit reachable-runtime Wasm gap; release docs were regenerated through the compiler
HANDOFF_NOTES: constant-folding Phase 4C must validate assertion message expressions in both authored branches before static selection and elide inactive assertion message work; the Wasm plan must retain target validation for reachable dynamic messages and static trap lowering for default/fully folded messages
NEXT_EXACT_SLICE: complete Phase 4 Wasm target-validation/lowerer hardening and remaining integration/documentation consolidation, then run the mandatory final audit
```

## Purpose

Finish the `assert` language surface and remove the parsing duplication that caused the current multiline-message failure.

Today `assert` is correctly represented as a language-owned statement intrinsic, but it parses its own parentheses, commas, argument count, named-argument rejection, mutable-marker rejection and literal-only message token. Ordinary functions, constructors, receiver methods and builtin members already share more complete call-argument parsing and validation. The assert parser does not inherit that newline handling, so this valid current form reaches the literal check while the current token is still a newline:

```moth
assert(
    release_slots.length() is slot_capacity,
    "release slot count must match capacity"
)
```

The immediate bug is one symptom of a broader ownership problem. Call-shaped argument syntax should have one parser. `assert` should keep only the semantics that genuinely make it special:

- statement-only placement
- reserved language identity
- static terminality for a compile-time `false` condition
- lazy message evaluation on the failure edge
- unrecoverable failure
- rejection of postfix handling applied to the assert statement itself

This plan therefore has two coupled outcomes:

1. Extract and consolidate one shared call-argument parser and one slot-routing owner for every call-shaped source surface.
2. Implement runtime assertion messages through ordinary typed expressions, optional/default argument semantics, backend-neutral HIR and target-specific lowering.

The change must delete the literal-message path rather than preserving it beside the new expression path.

## Roadmap position

This plan runs after:

1. Frontend module compilation ownership cleanup

It runs before:

1. Constant evaluation, static control-flow specialisation and type-system architecture
2. The remaining queued implementation chain

This order is deliberate.

The active frontend ownership plan may move compiler entry points, semantic result types and orchestration files. Phase 0 must re-anchor rather than forcing the planning snapshot onto the changed tree.

The assertion plan should then land before the broad constant and static-control-flow plan because it:

- fixes a bug in a currently supported syntax surface
- removes duplicate parser ownership before more Stage 4 work is added
- gives `assert(true)` and `assert(false)` their final AST and HIR message representation
- lets the later static-control-flow work preserve one settled assertion contract rather than migrating the literal-only shape during a larger optimisation project

This plan does not depend on the deferred collector-free memory implementation. It must still make assertion messages ordinary HIR uses so future borrow, lifetime and memory-plan analyses need no assertion-specific exception.

## Planning snapshot and confirmed current shape

The planning audit on 2026-08-20 confirmed `main` at:

```text
cb533ced7345dc7a66cf971e7590d88c8cd84f32
```

The following shape is confirmed at that revision.

### `asserts.rs` owns a bespoke argument parser

`src/compiler_frontend/ast/statements/asserts.rs` currently:

- consumes the `assert` token
- checks and consumes `(`
- parses the condition with `create_expression_until`
- checks and consumes the comma
- accepts only `TokenKind::StringSliceLiteral` for the message
- rejects named arguments itself
- rejects mutable markers itself
- checks extra arguments itself
- checks and consumes `)`
- rejects postfix `!` and `catch`

It does not call `skip_newlines()` after `(` or after the comma. The multiline bug follows directly from that duplicate token handling.

### Shared call-argument parsing already exists under the wrong conceptual owner

`src/compiler_frontend/ast/expressions/function_calls.rs` currently owns:

- `parse_call_arguments_typed_with_expectations`
- generic call-argument parsing
- opening and closing parenthesis handling
- multiline whitespace handling
- positional and named argument syntax
- mutable access marker parsing
- expected-type and cast-target routing
- argument expression parsing

The same parser is already consumed by:

- source and host function calls
- struct constructors
- choice constructors
- source receiver methods
- compiler-owned builtin members

The parser is therefore shared call-shaped syntax, not function-call semantics. Its final owner should be named for call arguments rather than remain embedded in `function_calls.rs`.

### Slot routing is implemented twice

The current parser uses `route_argument_slot_before_value_parse` to choose a parameter slot before parsing each value so it can supply the correct expected type and cast target.

`call_validation.rs` later uses `resolve_call_argument_slots_typed` to route the same positional and named arguments again before filling defaults and validating types and access.

The two paths currently mirror each other. That is another drift risk. The final architecture must keep one slot-routing owner and retain its result through final validation.

### Default arguments and option coercion already provide the required model

`ParameterExpectation` already carries:

- a stable parameter name
- an expected type
- an access mode
- reactive-source requirements
- an optional default expression

`resolve_call_arguments` already:

- routes positional and named arguments
- fills omitted defaulted slots
- diagnoses missing required slots
- validates types
- applies contextual coercion
- validates access mode

`Expression::option_none_with_type_id` already constructs a typed `none` value and contextual coercion already supports `String` to `String?`.

No new optional-parameter mechanism is required for `assert`.

### AST stores literal text outside the expression system

`src/compiler_frontend/ast/ast_nodes.rs` currently defines:

```rust
pub struct AssertMessage {
    pub text: StringId,
}

Assert {
    condition: Expression,
    message: Option<AssertMessage>,
}
```

The dedicated payload means ordinary AST expression finalisation, reactive metadata, remapping and type validation do not naturally traverse assertion messages.

### HIR stores an owned Rust string

`src/compiler_frontend/hir/terminators.rs` currently defines:

```rust
AssertFailure {
    message: Option<String>,
}
```

HIR lowering resolves the AST `StringId` eagerly into an owned Rust `String`. The failure terminator does not carry a Moth value expression.

### Analysis paths explicitly assume there is no message expression

Current consumers include:

- HIR validation
- HIR display
- HIR remapping
- HIR reachability
- borrow transfer for terminators
- JavaScript structured and dispatcher lowering
- Wasm HIR-to-LIR lowering

The borrow checker and HIR validator currently contain comments stating that assertion messages are compile-time text and therefore need no expression traversal.

### JavaScript and Wasm have different current limits

The JavaScript backend turns an optional Rust `String` into `throw new Error(...)` and supplies `"assertion failed"` when the payload is absent.

The Wasm backend maps every `AssertFailure` directly to `Trap` and ignores the message payload. This is harmless only while messages are compile-time text with no source-visible evaluation. It becomes incorrect when message construction can execute runtime Moth code.

### Documentation and status are literal-only

The canonical assertion page, Basic page, cheatsheet and progress matrix currently state that an assertion message must be one quoted string literal. The progress matrix marks assertions as supported for frontend, HIR, JavaScript and HTML and does not claim dynamic Wasm message support.

### The integration suite already has a broad assertion lane

The current manifest includes primary and boundary cases for:

- statement acceptance
- literal message acceptance
- default and explicit failure messages
- message escaping
- static terminality
- dynamic non-terminality
- non-Bool conditions
- missing and extra arguments
- non-string messages
- named-argument rejection
- mutable-marker rejection
- expression-position rejection
- postfix `!` and `catch` rejection
- keyword shadowing
- reactive assertion-message rejection

Several of these cases encode the temporary literal-only design and must be replaced or repurposed rather than retained as historical behavior.

## Accepted interview decisions

The following decisions were agreed during the design interview and are mandatory implementation requirements.

### 1. Runtime message expressions are ordinary infallible values

The message may be any ordinary expression that resolves to the assertion message parameter type after normal contextual coercion.

Valid sources include:

- quoted strings
- local `String` values
- runtime templates
- infallible function calls returning `String`
- `String?` values
- explicit `none`

There is no assert-specific implicit stringification. An `Int`, struct or other value must be converted through the ordinary language surface before it can become a message.

The message expression is parsed, type checked and fully validated even when the condition is compile-time `true`.

Message evaluation is lazy. It executes only after the condition is known to be false.

An assertion message must not propagate `!`, `?` or another enclosing-function exit from the failure edge. A broken invariant cannot turn into a recoverable error or optional return because constructing its diagnostic text failed. Fallible work must be handled before the assertion and passed in as an infallible value.

### 2. Call-shaped argument parsing is shared

The plan must extract or establish a focused shared owner such as:

```text
src/compiler_frontend/ast/expressions/call_arguments.rs
```

Exact file names may change after Phase 0, but the ownership may not.

The shared owner handles:

- parentheses
- commas
- multiline whitespace
- positional and named syntax
- argument expression boundaries
- access-marker recognition
- expected-type routing
- cast-target routing
- parameter-slot routing

Functions, constructors, receiver methods, builtin members and `assert` must use that one parser.

`assert` remains a statement intrinsic. It does not become an ordinary function expression or symbol.

### 3. `assert` uses a compiler-owned synthetic signature

The conceptual signature is:

```moth
assert |condition Bool, message String? = none|
```

This is compiler metadata. It is not authored Moth source and does not create an importable, shadowable or first-class function value.

The shared call machinery receives two expectations:

- `condition`: required, named `condition`, type `Bool`, shared access
- `message`: named `message`, type `String?`, shared access, default `none`

A plain `String` promotes to `String?` through ordinary contextual coercion.

`none`, whether defaulted, explicit or produced at runtime, means use the default assertion-failure message.

This design reuses existing default arguments and option coercion. It must not add user-defined optional parameters without defaults or another parameter-presence system.

### 4. Named argument parity is supported

The stable argument names are `condition` and `message`.

Valid forms include:

```moth
assert(ready)
assert(ready, "must be ready")
assert(condition = ready)
assert(ready, message = "must be ready")
assert(message = "must be ready", condition = ready)
```

Ordinary call rules apply to:

- positional-before-named ordering
- duplicate arguments
- unknown names
- missing required arguments
- extra arguments
- access mode
- default filling

`assert` must not retain token-level special cases for these rules.

The canonical ordinary call grammar at implementation time also owns trailing-comma policy. `assert` must not invent a separate rule.

### 5. JavaScript completes first and Wasm fails honestly

The frontend and HIR representation remain backend-neutral.

JavaScript and HTML implement complete runtime message evaluation and presentation.

Until Wasm can execute dynamic message construction faithfully:

- default and compile-time message forms may keep lowering to a trap
- reachable messages that require runtime evaluation are rejected during target validation
- unreachable dynamic message code does not reject a build
- a compile-time `true` assertion does not retain message runtime work in HIR and therefore does not create a Wasm gap
- the Wasm lowerer must not silently discard a dynamic message if target validation is bypassed

The progress matrix must record this target gap explicitly. The future mixed JavaScript and Wasm implementation plan must also retain the missing lowering and presentation work.

## Final source contract

`assert` remains statement-only and always checked.

Accepted forms are equivalent to:

```moth
assert(condition)
assert(condition, message)
assert(condition = condition_expression)
assert(condition = condition_expression, message = message_expression)
assert(message = message_expression, condition = condition_expression)
```

Rules:

- `condition` resolves to `Bool`
- `message` resolves to `String?`
- `message` defaults to `none`
- a present message is used as the failure text
- an absent message uses `"assertion failed"`
- message construction is lazy
- message construction cannot escape through propagation
- `assert` produces no value
- failure is unrecoverable
- `assert(...)!` is invalid
- `assert(...) catch ...` is invalid
- `assert(false, ...)` is statically terminal
- a dynamic assertion is not statically terminal
- assertions remain enabled in development and release builds

`assert` is not a reactive sink. A template-backed message is read as one snapshot at the failure point. It does not establish a live subscription or mount.

## Target architecture

The final frontend flow is:

```text
body parser sees reserved assert token
-> shared call-argument parser consumes one call-shaped argument list
-> shared slot router retains condition/message slot identity
-> shared call resolver fills message = none and applies Bool/String? coercion
-> assertion semantic gate rejects escaping message propagation
-> NodeKind::Assert carries two typed expressions
-> AST finalisation traverses and normalises both expressions
-> HIR lowers the condition
   -> true: continue without evaluating the message
   -> false: evaluate the message, then terminate with AssertFailure
-> analyses consume the message as an ordinary terminal use
-> reachability records whether message evaluation is runtime-dependent
-> target validation accepts or rejects that reachable feature
-> backend lowers the validated assertion failure
```

### Shared call-argument data flow

The final shared parser must not calculate argument slots and then ask `call_validation.rs` to calculate them again.

A data-oriented target shape is:

```rust
struct ParsedCallArguments {
    arguments: Vec<CallArgument>,
    routes: Vec<ParameterSlot>,
}
```

or an equivalent representation where each parsed argument retains its resolved slot.

One router owns:

- the parameter-name index
- the positional cursor
- whether a named argument has appeared
- occupied slots
- duplicate detection
- unknown-name detection
- positional-after-named detection

The parser asks that owner for a slot before parsing the value so it can supply the slot's expected type and cast target. Final call validation consumes the retained route, fills defaults and validates type and access. It does not repeat routing.

Exact Rust names are implementation details. The one-owner invariant is not.

### AST contract

The target AST shape is conceptually:

```rust
Assert {
    condition: Expression,
    message: Expression,
}
```

Invariants:

- `condition.type_id` is `Bool`
- `message.type_id` is `String?`
- omitted messages are represented by the inserted typed `OptionNone` expression
- `AssertMessage` no longer exists
- both expressions carry their normal locations, provenance and template/reactive metadata
- every general AST expression walker traverses the message unless its responsibility deliberately concerns the condition only, such as static terminality

### HIR contract

The target failure terminator carries a Moth value rather than source-resolved Rust text.

Conceptually:

```rust
struct HirAssertMessage {
    value: HirExpression,
    evaluation: HirAssertMessageEvaluation,
}

enum HirAssertMessageEvaluation {
    Static,
    Runtime,
}

AssertFailure {
    message: HirAssertMessage,
}
```

The exact wrapper may be omitted if the then-current HIR already has one authoritative fact that distinguishes compile-time message values from runtime evaluation. Do not add a parallel boolean when an existing semantic fact is sufficient.

Required invariants:

- the value has semantic type `String?`
- `Static` is proven only for default `none` or a fully folded present string
- every uncertain case is `Runtime`
- the classification is produced once by the compiler, not re-derived from source by a backend
- the optional carrier itself owns default-versus-present behavior
- HIR remapping traverses the value
- HIR validation checks the type and classification contract
- HIR display shows the message value and evaluation class

### Lazy CFG lowering

For a dynamic condition:

```text
condition block
    |
    v
    If
   /  \
pass  fail
        |
        +-- message preludes
        +-- optional message value
        v
   AssertFailure
```

For compile-time `false`, lower the message in the current block before emitting `AssertFailure`.

For compile-time `true`, do not lower message runtime work. Parsing, typing and AST finalisation still happen before this decision.

### Analysis contract

The message is an ordinary terminal value use.

- Borrow transfer records shared reads in the message expression.
- Call statements and other preludes produced by message construction remain in the failure block.
- Reachability sees those calls only when the failure path survives HIR construction.
- Lifetime and escape analyses consume the same HIR facts as other expressions.
- No assertion-specific retained edge, REC rule or ownership category is added.
- The failure block has no successor, so message-local values do not remain live after failure.

### JavaScript contract

JavaScript lowering:

1. Evaluates the optional message exactly once.
2. Uses ordinary plain-value lowering so template-backed strings become a snapshot.
3. Reads the established option carrier rather than inventing another representation.
4. Selects the present `String` or `"assertion failed"`.
5. Throws one `Error` with that text.

Do not add a broad runtime helper unless the same behavior has another real consumer. A local temporary is preferable when it keeps evaluation exact and readable.

### Wasm contract

HIR reachability records the first reachable assertion message that requires runtime evaluation.

Wasm target validation reports a structured unsupported-backend diagnostic at the authored message location.

Default and fully folded message values may reach the current trap lowering because ignoring their presentation does not skip source-visible runtime work.

If a runtime message reaches Wasm lowering after validation, the lowerer returns `CompilerError`. It must not silently emit `Trap`.

Future Wasm work must evaluate the same HIR message value before trapping or reporting through the page runtime. It must not add a second assertion representation.

## Diagnostic ownership

Shared call diagnostics own:

- unknown named arguments
- duplicate arguments
- positional arguments after named arguments
- missing `condition`
- extra arguments
- type mismatch for `condition`
- type mismatch for `message`
- invalid mutable access markers
- malformed argument expressions

Add a truthful assert or language-intrinsic call diagnostic context if the existing contexts would render the surface as a function, constructor or builtin member incorrectly. Do not create a separate reason taxonomy for every shared call error.

Assertion-specific diagnostics own only genuinely assertion-specific behavior:

- propagation or another enclosing-function exit inside message construction
- postfix `!` on the completed assert statement
- `catch` on the completed assert statement
- statement-only placement where the surrounding parser needs a targeted diagnostic

Delete:

- `InvalidBuiltinCallReason::RuntimeMessageExpressionDeferred`
- its reason key
- its renderer branch
- tests that preserve the deferred limitation
- comments that claim assertion messages are compile-time text

Diagnostic codes may change when ownership moves from `InvalidBuiltinCall` to shared call or type diagnostics. Do not preserve an old code or wording solely because the bespoke parser produced it.

Every changed diagnostic needs exact code, reason and source-location coverage under its final owner.

## Scope boundaries and non-goals

This plan does not:

- make `assert` a normal function value
- make `assert` importable, aliasable or shadowable
- make assertions recoverable
- add panic catching or exceptions
- remove assertions from release builds
- add implicit stringification
- add user-defined optional parameters without defaults
- add a general compiler-intrinsic registry without another concrete consumer
- redesign option runtime representation
- redesign general constant evaluation
- implement Wasm message presentation
- make `assert` a reactive sink
- change general call-argument grammar beyond removing duplicate ownership
- redesign build-system or module-compilation orchestration
- add compatibility wrappers, forwarding re-exports or parallel AST/HIR payloads

If Phase 0 finds a broader call parser bug, fix it only when the shared grammar authority clearly owns it and add cross-surface regression coverage. Do not expand this plan into an unrelated call-semantics redesign.

## Documentation changes required by implementation

Documentation is part of the implementation, not optional cleanup.

### Canonical assertion reference

Update `docs/src/docs/errors/assertions.mtf` to own the complete accepted contract:

- conceptual `Bool, String? = none` parameters
- positional and named forms
- runtime expressions
- explicit and runtime `none`
- lazy failure-edge evaluation
- propagation rejection
- snapshot behavior for template-backed messages
- static terminality
- unrecoverable behavior

During Phase 0, mark runtime messages as accepted deferred until the implementation lands if the page would otherwise overstate current compiler support.

### Basic assertion page

Update `docs/src/docs/errors/assertions-basic.mtf` when JavaScript support lands. Keep the explanation short and practical.

### Cheatsheet

Replace the quoted-literal restriction with the final optional runtime message contract and include one concise named or runtime example.

### Compiler architecture

Update the relevant sections of `docs/compiler-design-overview.md` so they state:

- call-shaped argument syntax has one parser and one slot-routing owner
- `assert` is parsed through that shared syntax owner but remains a statement intrinsic
- AST owns the synthetic signature and message-effect validation
- HIR carries a lazy failure-edge message value
- target validation rejects reachable dynamic assertion messages on unsupported targets

Audit `docs/compiler-data-layout-design.md` if it names the current literal AST or HIR payload.

### Reactivity documentation

Audit the canonical reactivity pages that state `assert` is not a live sink. Preserve that rule and clarify that an assertion message takes one snapshot on failure.

### Progress matrix

When the feature lands, update the Assertions row to state:

- runtime `String?` messages and named/default arguments are supported on JavaScript and HTML
- message evaluation is lazy
- Wasm supports only default or compile-time message trap lowering
- reachable dynamic message evaluation is a structured Wasm target gap

Do not mark dynamic Wasm messages supported until the backend evaluates them faithfully.

### Wasm roadmap

Update `docs/roadmap/plans/html_project_backend_wasm_final_implementation_plan.md` or its then-current successor with the retained work:

- evaluate runtime assertion messages
- preserve optional default semantics
- route text through the page runtime when presentation is implemented
- remove the temporary target-validation gap only after lowering exists

### File index and generated docs

Update `index.md` if argument parsing moves to a new module or existing module responsibilities change.

Never edit `docs/release/**` by hand. Rebuild release documentation through the compiler.

## Existing integration cases that require review

Phase 0 must verify the current names, contracts and ownership before editing them.

### Preserve or strengthen

- `assert_statement_success`
- `assert_statement_message_success`
- `assert_false_runtime_stop`
- `assert_false_message_runtime_stop`
- `assert_message_escaping_runtime_stop`
- `assert_false_terminal_function`
- `function_assert_false_terminal`
- `assert_dynamic_not_terminal`
- `assert_condition_not_bool`
- `assert_missing_argument`
- `assert_too_many_arguments`
- `assert_non_string_message`
- `assert_expression_position`
- `assert_bang_rejected`
- `assert_catch_rejected`
- assert keyword-shadowing cases
- value-producing catch terminality cases that use `assert(false)`

### Replace or repurpose

- Replace `assert_named_argument_rejected` with named-argument success plus unknown, duplicate and ordering diagnostics owned by shared call validation.
- Keep mutable-marker rejection behavior but migrate its expected diagnostic to shared access validation.
- Replace `reactive_assert_message_rejected` with snapshot and lazy-evaluation coverage. `assert` remains a non-live sink.
- Replace any expectation for `RuntimeMessageExpressionDeferred` with runtime success, ordinary type mismatch or the targeted propagation diagnostic.

Do not keep both old and new cases for the same contract.

## Required new coverage

### Shared argument parsing

Add focused tests that prove every migrated surface shares:

- newline handling after `(`
- newline handling after commas
- named reordering
- positional-before-named enforcement
- duplicate and unknown-name detection
- default filling
- mutable access parsing
- expected-type and cast-target routing
- one retained slot route through final validation

Include regression coverage that would fail if parse-time and validation-time slot routing diverged.

### Assertion source behavior

Add end-to-end cases for:

- the exact multiline literal example that exposed the bug
- a multiline runtime template message
- a local `String` message
- an infallible message-producing function call
- `condition = ...`
- `message = ..., condition = ...`
- mixed positional and named arguments
- explicit `none`
- runtime `String?` present and absent values
- ordinary `String` promotion to `String?`
- non-string rejection through normal type diagnostics
- `!` propagation rejection inside message construction
- `?` propagation rejection inside message construction when type-correct source can express it

### Lazy evaluation

Prove separately that:

- `assert(true, side_effecting_message())` does not run message construction
- a dynamic true condition does not run message construction
- a false condition runs message construction exactly once before failure
- a compile-time true assertion contributes no runtime message call or backend feature fact
- a compile-time false assertion remains terminal after message lowering

Use runtime chronology or a narrow artifact/HIR assertion rather than broad generated-output goldens.

### Reactivity

Prove that:

- a template-backed message is accepted on JavaScript
- its current value is snapshotted only on failure
- it does not register an assertion sink or live mount
- a compile-time true assertion does not retain reactive runtime work

### HIR and analyses

Add focused tests for:

- `AssertFailure` message remapping
- HIR type validation for `String?`
- HIR display
- message preludes placed only in the failure block
- borrow facts containing message shared roots
- no message facts on the pass continuation
- call reachability from the failure block
- runtime-message reachability classification

### Target support

Add backend cases that prove:

- JavaScript evaluates and reports dynamic messages
- Wasm accepts default-message trap lowering
- Wasm accepts a fully folded literal-message trap
- Wasm rejects a reachable dynamic message with the exact unsupported-feature reason and location
- Wasm does not reject an unreachable helper containing a dynamic assertion message
- Wasm does not reject runtime message source under `assert(true, ...)` after HIR elision
- the Wasm lowerer fails internally if a dynamic message bypasses target validation

## Agent execution rules

Each phase below is one bounded review unit intended to fit inside one coding-agent context.

- Re-anchor before implementing because the prerequisite plan may move files.
- Do not start the next phase until the current phase gate is accepted.
- Keep every phase compiling and semantically honest.
- Update this plan's current-state capsule and checkboxes at every accepted checkpoint.
- Move code directly to its final owner and delete the old owner in the same phase.
- Do not keep forwarding re-exports from `function_calls.rs` after argument parsing moves.
- Do not keep literal and expression assertion payloads in parallel.
- Do not bypass shared call validation with an assert-only named/default resolver.
- Do not let a backend infer message semantics from source or AST.
- Prefer small data records and direct functions over callbacks, traits or a generic intrinsic framework.
- Keep tests under their owning modules and integration cases under `tests/cases/`.
- Update tests in the same phase as the behavior they protect.
- When a phase changes current support, update the progress matrix in that phase.
- End every non-trivial phase with an ownership audit, style-guide review and validation.

## Mandatory gate for every phase

### Ownership and architecture audit

Inspect the changed owner and adjacent consumers. Look for:

- duplicated delimiter or slot-routing logic
- old and new parser entry points coexisting
- literal and expression assertion payloads coexisting
- assertion-only copies of shared call diagnostics
- AST expressions skipped by finalisation or remapping
- HIR message values skipped by validation, analysis or reachability
- JavaScript or Wasm reconstructing source meaning
- dynamic message evaluation silently removed on a target
- reactive metadata accidentally turning `assert` into a live sink
- temporary compatibility wrappers or stale comments

### Style-guide review

Review every changed production file against the current style guide, especially:

- one clear owner per module
- focused files under the size guideline where practical
- main flows that read as named steps
- no boolean-heavy generic parser framework
- no broad trait abstraction for a small closed set of call surfaces
- no cloned argument or expression graphs added only to bridge a poor boundary
- no stale comments naming literal-only messages
- tests outside production files
- structured diagnostics for every user-authored failure

### Validation

Run focused tests first. The final command for every code-bearing phase is:

```bash
just validate
```

When a phase is documentation-only, use:

```bash
moth build docs --release
```

or:

```bash
cargo run --quiet -- build docs --release
```

At implementation start, Phase 0 must read the then-current validation guide and adopt any stronger commands that have replaced these static examples.

---

# Implementation phases

## Phase 0: Re-anchor the tree, freeze the regression and publish the accepted contract

### Context

The plan is queued behind a cross-stage frontend ownership move. Its first phase must verify the live tree and make the accepted assertion contract authoritative before production representation changes begin.

This phase does not broaden runtime support yet.

### Checklist

- [x] Resolve and record the then-current `main` revision as the implementation start revision.
- [x] Re-read `AGENTS.md`, the compiler architecture, the canonical assertion page, the cheatsheet, the progress matrix, the complete style guide, testing guidance and validation guidance.
- [x] Re-inventory the current owners of call-argument parsing, parse-time slot routing, final call validation and every call-shaped consumer.
- [x] Re-inventory every `NodeKind::Assert` match and every `HirTerminator::AssertFailure` match.
- [x] Re-inventory assertion-specific diagnostic variants, reason keys, renderers and exact integration expectations.
- [x] Re-inventory the assertion integration cases listed above and record any cases added or removed since the planning snapshot.
- [x] Reproduce the exact multiline assertion failure and record its current code, reason and source location in this plan.
- [x] Record focused baseline commands for call parsing, assertions, HIR, borrow facts, backend feature validation and JavaScript/Wasm integration.
- [x] Run the current full validation gate before production changes and record the result.
- [x] Update the canonical assertion reference with the accepted end-state contract and an explicit accepted-deferred note where current support still differs.
- [x] Update the compiler architecture with the one-parser, synthetic-signature, lazy-HIR and target-validation ownership contract.
- [x] Audit the constant-evaluation plan and Wasm plan for assumptions that this work changes, then record required later edits here.
- [x] Update this plan's current-state capsule with confirmed paths, blockers, baseline results and the next exact slice.

### Phase 0 gate

- [x] Ownership audit confirms every parser, AST, HIR, analysis, backend and diagnostic consumer is accounted for.
- [x] Style-guide review confirms the target is one concrete shared parser rather than a generic callable framework.
- [x] Documentation release build and the current full validation gate pass without production behavior changes.

Exit state: the current tree is re-anchored, the bug is frozen as a known regression and the accepted design is authoritative.

## Phase 1: Extract one shared call-argument parser and one slot-routing owner

### Context

Runtime assertion messages should not be implemented on top of the current bespoke parser. This phase first makes call-shaped argument syntax a truthful shared owner while preserving existing source behavior.

It also removes the current double implementation of parameter-slot routing.

### Checklist

- [x] Create the final focused call-argument parsing owner under `ast/expressions/` or adopt an equivalent owner found during Phase 0.
- [x] Move parenthesis, comma, newline, named-target, access-marker, expected-type and argument-expression parsing out of `function_calls.rs`.
- [x] Move generic call-argument syntax policy with the parser while keeping generic semantic inference in its existing owner.
- [x] Define one data-oriented parameter-slot router.
- [x] Make parse-time expected-type and cast-target selection consume that router.
- [x] Retain each argument's resolved slot through final validation.
- [x] Make default filling, type validation and access validation consume retained routes instead of rerunning named/positional routing.
- [x] Preserve exact argument locations, named-target locations and mutable-marker locations.
- [x] Migrate source calls, host calls, struct constructors, choice constructors, receiver methods and builtin members to the final parser API.
- [x] Keep each consumer's existing named-argument policy and diagnostic context.
- [x] Delete moved parsing and routing implementations from `function_calls.rs` and `call_validation.rs`.
- [x] Do not leave forwarding re-exports or duplicate helper names for compatibility.
- [x] Update module documentation and `index.md` for the new owner.
- [x] Add focused parser and route-retention tests for every rule listed in Required new coverage.
- [x] Run existing function, constructor, receiver, builtin, generic and cast-target integration cases to prove no source behavior changed.

### Phase 1 gate

- [x] Ownership audit finds one delimiter parser and one slot router across every call-shaped consumer.
- [x] Style-guide review confirms the shared owner is focused and does not absorb function-call completion, constructor semantics or result handling.
- [x] Focused call/generic/constructor/receiver/builtin tests and `just validate` pass.

Exit state: every existing call-shaped surface uses one robust argument parser and one routing result.

## Phase 2: Move `assert` onto the shared call contract and complete AST/HIR ownership

### Context

With shared syntax in place, `assert` can become a small semantic consumer. This phase replaces literal text with typed expressions and establishes lazy backend-neutral HIR.

Until JavaScript lowering lands in Phase 3, target validation may conservatively reject reachable runtime assertion messages on both active targets. This temporary gate is acceptable because it is explicit and semantically honest. Do not preserve the old deferred-parser diagnostic.

### Checklist

- [x] Add the compiler-owned `condition Bool, message String? = none` expectation builder.
- [x] Intern the stable names `condition` and `message` through the active string table.
- [x] Construct the default through `Expression::option_none_with_type_id` or its then-current canonical equivalent.
- [x] Add a truthful assert/language-intrinsic diagnostic context to shared call validation when needed.
- [x] Replace manual delimiter, comma, arity, named-target and mutable-marker parsing in `asserts.rs` with the shared parser and resolver.
- [x] Keep reserved-token dispatch, statement placement and completed-statement suffix rejection under the assert statement owner.
- [x] Add the assertion message effect gate that rejects `!`, `?` or another escaping control-flow form without rescanning tokens.
- [x] Preserve ordinary infallible message calls and precomputed handled values.
- [x] Change `NodeKind::Assert` to carry typed condition and message expressions.
- [x] Delete `AssertMessage`.
- [x] Update every AST walker found in Phase 0, including template normalisation, reactive metadata flow, type validation, debug validation, const-fact collection, remapping and value-production completeness where applicable.
- [x] Keep terminality dependent only on a compile-time false condition.
- [x] Change `AssertFailure` to carry the typed optional message value and the authoritative static/runtime evaluation fact.
- [x] Lower message preludes only after entering the failure block.
- [x] Preserve compile-time true elision and compile-time false terminality.
- [x] Update HIR remapping, validation, display and side-table mappings.
- [x] Update borrow transfer so the message is an ordinary shared terminal use.
- [x] Update reachability so runtime assertion-message requirements and failure-edge calls are retained.
- [x] Add a temporary structured target gate for runtime assertion messages until each target implements them.
- [x] Delete `RuntimeMessageExpressionDeferred` and all literal-only implementation comments.
- [x] Update exact diagnostic tests for shared missing, extra, named, type and access errors plus assertion-specific propagation errors.
- [x] Add focused AST, HIR, borrow and reachability tests, including static-false message CFGs, target classes, loaded metadata, raw/owned template handoffs and recovered fallible values.

### Phase 2 gate

- [x] Ownership audit finds no literal assertion payload and no assert-only argument parser or slot resolver.
- [x] Style-guide review confirms the message effect gate reads resolved semantic facts rather than token syntax.
- [x] Focused assertion/parser/AST/HIR/borrow/reachability tests and `just validate` pass.

Exit state: runtime assertion messages exist as correct lazy frontend and HIR semantics, with unsupported target execution rejected before lowering.

## Phase 3: Implement JavaScript and HTML runtime assertion messages

### Context

JavaScript already owns full `String`, option, template snapshot and `Error` behavior. This phase consumes the final HIR message contract without adding another runtime representation.

### Checklist

- [x] Lower the optional message exactly once at `AssertFailure`.
- [x] Use ordinary plain-value lowering so reactive or template-backed strings become one failure-time snapshot.
- [x] Reuse the established option carrier to select present text or `"assertion failed"`.
- [x] Preserve literal escaping through the normal JavaScript string/value path.
- [x] Preserve both structured CFG and dispatcher CFG assertion lowering.
- [x] Avoid a new global runtime helper unless another real consumer justifies it.
- [x] Remove JavaScript from the temporary runtime-assertion-message target rejection.
- [x] Add runtime cases for local strings, templates, function calls, optional present/absent values and named arguments.
- [x] Add chronology tests proving success does not evaluate the message and failure evaluates it once before throwing.
- [x] Replace the reactive rejection case with snapshot and non-sink coverage.
- [x] Preserve existing default-message, explicit-message and escaping cases.
- [x] Add JavaScript emitter tests for exact single evaluation and optional selection.

### Phase 3 gate

- [x] Ownership audit confirms JavaScript consumes HIR and existing option/template contracts without source or AST reconstruction.
- [x] Style-guide review confirms optional unwrapping and one-time evaluation are readable and local.
- [x] Focused JavaScript/HTML runtime tests and `just validate` pass.

Exit state: JavaScript and HTML support the complete accepted runtime assertion-message surface.

## Phase 4: Harden Wasm target validation and retain the future backend gap

### Context

Wasm may continue trapping for messages that need no runtime source evaluation. It may not skip dynamic message construction. This phase makes that distinction explicit and durable.

### Checklist

- [ ] Add a reachable runtime assertion-message fact with exact source location.
- [ ] Add `UnsupportedBackendFeatureReason::RuntimeAssertionMessages` or an equally narrow typed reason.
- [ ] Reject the first reachable dynamic message during Wasm target validation.
- [ ] Preserve default-message and fully folded present-message trap lowering.
- [ ] Prove unreachable helper assertions do not fail target validation.
- [ ] Prove compile-time true assertions retain no runtime message requirement.
- [ ] Make the Wasm lowerer return `CompilerError` if a dynamic message bypasses validation.
- [ ] Keep one HIR assertion representation for both targets.
- [ ] Add focused reachability, target-validation and lowerer-invariant tests.
- [ ] Add integration cases for static acceptance and dynamic rejection on Wasm.
- [ ] Update the progress matrix with the explicit dynamic-message Wasm gap.
- [ ] Update the mixed JavaScript/Wasm roadmap plan with the future evaluation and presentation work.

### Phase 4 gate

- [ ] Ownership audit confirms capability selection happens in target validation and the lowerer only enforces the validated invariant.
- [ ] Style-guide review confirms no backend-specific flag leaked into source, AST type identity or general call parsing.
- [ ] Focused Wasm validation/lowering tests and `just validate` pass.

Exit state: Wasm fails early and clearly for dynamic messages while preserving supported static trap behavior and a durable future work record.

## Phase 5: Consolidate diagnostics, integration cases and documentation

### Context

The implementation changes several previously intentional rejection cases into success cases and moves other failures under shared call diagnostics. This phase removes stale contracts and leaves one honest test and documentation surface.

### Checklist

- [ ] Audit every assert-tagged integration case and its manifest contract.
- [ ] Replace `assert_named_argument_rejected` with named success and only the missing boundary diagnostics not already owned by general call cases.
- [ ] Update mutable-marker expectations to the shared access diagnostic.
- [ ] Update non-string message expectations to the normal type mismatch diagnostic.
- [ ] Replace `reactive_assert_message_rejected` with snapshot behavior.
- [ ] Remove every expectation for `RuntimeMessageExpressionDeferred`.
- [ ] Add the exact multiline regression case.
- [ ] Add runtime local, template, function, optional and lazy-evaluation cases without duplicating one contract across many fixtures.
- [ ] Preserve statement-only, suffix-rejection and static-terminality cases.
- [ ] Run the test-suite audit and correct manifest ownership, roles and duplicate-primary findings.
- [ ] Finish the canonical assertion and Basic pages.
- [ ] Update the cheatsheet.
- [ ] Update compiler architecture and data-layout documentation where implementation names or payloads changed.
- [ ] Update reactivity wording while preserving non-live-sink semantics.
- [ ] Verify the progress matrix accurately distinguishes JavaScript support and the Wasm gap.
- [ ] Update `index.md` for moved parser ownership.
- [ ] Search source, tests and docs for stale literal-only wording, old payload names and removed diagnostic reasons.
- [ ] Rebuild generated release documentation through the compiler.

### Phase 5 gate

- [ ] Ownership audit confirms each observable assertion contract has one primary integration owner and focused unit tests protect only internal invariants.
- [ ] Style-guide review confirms docs describe the final source contract without implementation chronology.
- [ ] Test-honesty audit, documentation release build and `just validate` pass.

Exit state: tests, diagnostics, canonical docs, teaching docs, status and indexes all describe one final assertion implementation.

## Phase 6: Final audit, validation and closeout

### Context

The final phase verifies the whole change as one architecture and language surface rather than accepting locally green phases with cross-phase drift.

### Checklist

- [ ] Re-read every changed production module from its entry point.
- [ ] Re-run searches for bespoke call delimiters, duplicate slot routing, `AssertMessage`, `Option<String>` HIR message payloads, `RuntimeMessageExpressionDeferred` and literal-only assertion wording.
- [ ] Confirm every call-shaped consumer uses the final parser and route representation.
- [ ] Confirm every AST walker handles the message expression correctly.
- [ ] Confirm every HIR consumer handles the message value and evaluation class correctly.
- [ ] Confirm JavaScript lazy evaluation through runtime chronology.
- [ ] Confirm Wasm dynamic-message rejection and static-message acceptance.
- [ ] Confirm `assert(true, ...)` retains no message HIR, call reachability, reactive feature or target requirement.
- [ ] Confirm `assert(false, ...)` remains statically terminal after message lowering.
- [ ] Confirm no compatibility re-export, duplicate payload or fallback parser remains.
- [ ] Run focused assertion, call, AST, HIR, borrow, reachability, JavaScript and Wasm suites.
- [ ] Run `cargo run --quiet -- tests --audit` or its then-current replacement.
- [ ] Run `just bench-check` as a non-recording parser/frontend regression check.
- [ ] Run `just validate`.
- [ ] Run a fresh Final audit using the current audit workflow and resolve every required finding.
- [ ] Re-run affected focused gates and `just validate` after audit corrections.
- [ ] Record the accepted final commit, validation and audit result in this plan.
- [ ] Mark the plan complete and update roadmap sequencing without removing the Wasm progress gap.

### Phase 6 gate

- [ ] Final ownership audit is clean.
- [ ] Final style-guide review is clean.
- [ ] Documentation and progress status are current.
- [ ] Full validation passes on the final tree.
- [ ] Final audit reports no unresolved required findings.

Exit state: `assert` has one shared call-shaped parser, runtime optional messages on JavaScript, honest Wasm gating and no literal-only compatibility path.

---

# Completion criteria

The plan is complete only when all of the following are true.

## Parsing and calls

- [ ] One shared owner parses call-shaped delimiters, newlines, names, access markers and expression boundaries.
- [ ] One owner routes arguments to parameter slots.
- [ ] Final validation consumes retained routes instead of recalculating them.
- [ ] Functions, constructors, receiver methods, builtin members and `assert` use the shared owner.
- [ ] The exact multiline assertion example compiles.

## Source semantics

- [ ] `condition` and `message` named arguments work through ordinary rules.
- [ ] `message` behaves as `String? = none`.
- [ ] Runtime strings, templates, calls and optional values work on JavaScript.
- [ ] `none` selects the default message.
- [ ] Message evaluation is lazy and exact-once.
- [ ] Message propagation cannot escape the failure edge.
- [ ] `assert` remains statement-only, unrecoverable, always checked and statically terminal for `false`.

## Compiler representation

- [ ] `AssertMessage` is deleted.
- [ ] AST carries the typed optional expression.
- [ ] HIR carries a backend-neutral message value and authoritative runtime-evaluation fact.
- [ ] Message preludes live only on the failure path.
- [ ] HIR validation, remapping, display, borrow facts and reachability traverse the value.
- [ ] No assertion-specific memory or ownership category exists.

## Backends

- [ ] JavaScript reports runtime messages and preserves default text.
- [ ] Template-backed messages snapshot on failure and do not become live sinks.
- [ ] Wasm accepts static trap forms.
- [ ] Wasm rejects reachable dynamic message evaluation before lowering.
- [ ] Wasm lowerer fails internally rather than silently discarding an unvalidated dynamic message.
- [ ] The progress matrix and Wasm plan retain the future implementation gap.

## Diagnostics, tests and docs

- [ ] Shared call and type diagnostics own shared failures.
- [ ] Assertion-specific diagnostics own only assertion-specific behavior.
- [ ] `RuntimeMessageExpressionDeferred` is deleted everywhere.
- [ ] Obsolete rejection fixtures are replaced or removed.
- [ ] Lazy evaluation, named/default arguments, optional values, analyses and target gaps have honest coverage.
- [ ] Canonical docs, Basic docs, cheatsheet, architecture, reactivity wording, progress and indexes are current.
- [ ] Generated docs were rebuilt rather than edited.

## Closeout

- [ ] No compatibility path remains.
- [ ] No duplicate parser or slot router remains.
- [ ] No stale literal-only wording remains.
- [ ] Full validation passes.
- [ ] Final audit is clean.

## Handoff summary

The implementation should start from this invariant:

```text
assert is semantically special but syntactically ordinary
```

Keep the reserved statement token and terminal failure semantics. Reuse the same typed call-argument machinery as every other call-shaped surface. Represent the optional message as a normal Moth expression, evaluate it only on failure and let target validation describe backend limits before lowering.

The most important failure modes to guard against are:

- fixing the newline bug with local `skip_newlines()` calls while leaving the bespoke parser
- adding named/default support through a second assert-only resolver
- retaining both literal and expression message payloads
- evaluating the message before the condition
- letting propagation escape the false-assert path
- skipping message expressions in an AST or HIR walker
- silently discarding runtime message work on Wasm
- turning reactive messages into live sinks
- preserving obsolete diagnostics or fixtures for compatibility

Delete the temporary architecture. Leave one boring path.
