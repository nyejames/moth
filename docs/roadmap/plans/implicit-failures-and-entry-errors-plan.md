# Implicit failures and entry errors implementation plan

## Status

- Status: active. Private inferred failure materialises at builtin Error!. Wasm rejects recoverable numeric failure as its own target diagnostic. An export or custom error slot that would let implicit failure escape is a source diagnostic with a witness, not a trap. start Error! is not implemented.
- Current slice: Phase 4 remaining public-surface and compatibility checks.
- Blockers: none for local export or custom-slot rejection.
- Next action: prove re-exports, package facades and foreign projections cannot publish an unchecked implicit failure, then invalidate stale summaries and old artefacts.

## Goal and authority

**Goal:** Make language-defined numeric failure recoverable by default, keep
module interfaces explicit and preserve terse top-level programs through a
compiler-supplied Error! entry boundary.

**Architecture:** Infer one closed built-in failure effect inside a semantic
module. Resolve expression handlers and function boundaries in the frontend,
lower them to explicit success/failure control flow and let builders consume
an ordinary typed entry outcome. Reuse existing result slots, numeric checks,
Error construction, diagnostics and dev-server presentation.

**Implementation surface:** Rust compiler, JavaScript and WebAssembly lowerers,
HTML entry assembly, dev-server client/server reporting and Moth documentation.
No new dependency or general effect framework is required.

**Design reference:** The accepted contract in this file records the
maintainer's decisions from the design discussion. It is the primary reference
for this change until every rule has been transferred into its permanent
owner. This is an explicit, scoped supersession authorised by the maintainer,
not permission for arbitrary roadmap prose to override language references.
Phase 1 publishes the contract. Final closeout verifies the transfer before
retiring this plan.

The later decisions in the discussion are binding:

- Implicit failures and built-in Error! may share one expression-wide catch,
  including failures while evaluating a cast operand.
- Resource exhaustion remains outside recoverable failure. It is excluded
  language design, not a pending allocation or stack effect.
- The implicit start has built-in Error!, handled by its generated caller.
- A grouped if: handler, rewind/rollback and broader effects are speculative
  future ideas only. They are not implemented or reserved by this plan.
- Explicit wrapping and native-trap substitution for assertion handlers remain
  unsettled future work. This plan supplies neither an unchecked mode nor new
  panic syntax.

### Exact supersession

| Previous rule | Replacement |
|---|---|
| Runtime numeric failure uses Error! only when declared, otherwise trap mode | Numeric checks produce implicit failure. Its enclosing handler or function contract determines delivery, never wrapping or an implicit panic. |
| Arithmetic cannot be caught directly | Existing catch handles a complete eligible expression, including arithmetic and inferred-failure private calls. |
| A call catch excludes argument failures | The selected expression includes receiver and argument evaluation as well as the call. |
| cast expression catch handles only conversion failure | It handles compatible failures throughout the operand and conversion. |
| start has no error slot and top-level typed errors require local recovery | start has the built-in Error! slot. Ordinary compatible postfix ! is legal at the top level. |
| HTML startup has only successful runtime fragment output | Startup returns success or Error. Runtime fragments are published only after success. |

All other contracts remain unless this file explicitly changes them. In
particular, preserve numeric promotion, canonical type identity, receiving
coercions, cast evidence, source-known numeric diagnostics, ordered result
slots, memory safety and source/module visibility.

## Sequencing and permanent references

The main roadmap owns serial order. Insert this work directly after Numeric
types and semantics, before subsequent serial compiler work. Complete the
whole numeric delivery first, not only its fixed-width slices. Keep the
first-party package programme's independently active work intact and coordinate
shared owners rather than pausing unrelated packages.

Required delivered capabilities are canonical module compilation, resolved
public interfaces, existing typed error continuations, checked numeric
operations, the complete numeric family and numeric profile, Error.code U32,
HTML runtime fragment assembly and the dev server.

Later dense-HIR work, native multi-result refactoring, Wiring, page-purpose
syntax and full Wasm aggregate/memory delivery are not prerequisites. Adapt the
representations present at activation while preserving their accepted final
contracts. This plan must not implement those unrelated features to complete
error handling. Future work consumes the permanent contracts published here.

Read AGENTS.md, the style guide and the routed testing/validation guides. Read
both architecture documents in full at activation. Relevant permanent owners:

- `docs/compiler-design-overview.md`: module compilation, semantic interfaces,
  generated functions, result slots, numeric ownership and backend handoffs.
- `docs/build-system-design.md`: entry assembly, HTML runtime, dev tooling,
  output ownership and compatibility.
- `docs/compiler-data-layout-design.md`: source identity, diagnostic records and
  separation of source diagnoses, infrastructure failures and compiler bugs.
- `docs/src/developer-docs/language/overview.mtf`: canonical language routes.
- `docs/src/docs/errors/`: Error values, error returns, propagation, catch,
  assertions, options and runtime-failures-and-termination.mtf.
- `docs/src/docs/numbers/`, `casts/`, `functions/`, `loops/` and `branching/`:
  exact source and receiving rules.
- `docs/src/docs/project-structure/` and `packages/`: module/export, entry,
  foreign-binding and public-surface contracts.
- `docs/src/developer-docs/memory-management/overview.mtf`: follow its routes
  for error exits, aliases, cleanup and successful mutation commits.
- `docs/src/docs/progress/@page.moth` and its packages-and-builders companion:
  current support, never an alternative definition of final semantics.

Name prerequisite capabilities rather than linking other temporary plans.
Keep the activation baseline and changing source-path inventory in working
notes, not in this queued plan's status block.

## Accepted language contract

### 1. Three distinct outcomes

A successful expression produces its ordinary zero, one or multiple success
slots. Moth does not create tuples or a first-class Result carrier for them.

An **implicit built-in failure** stops a checked computation without producing
its success slots. It is compiler-owned control flow, not a source type,
user-defined effect or throwable value. Private functions without an explicit
error slot infer whether this outcome can escape and propagate it within the
same semantic module without a postfix ! at every internal call.

A **typed error** is the value in an explicitly declared final E! return slot.
It remains ordinary Moth error data. Calls require an applicable catch or
existing explicit propagation. Returning Error.code = 0 is still an error.

**Unrecoverable termination** includes assertion failure, fatal resource
limits and unexpected host/runtime faults. Moth catch does not intercept these
as implicit failure or typed error. A host may report a fault without making
it recoverable or proving that it came from a compiler bug.

There is no source fails annotation, public effect row, user throw operation,
exception hierarchy, dynamic handler lookup or continuation API.

### 2. Closed implicit-failure scope

Use the delivered numeric operation and failure classifications, with one
frontend-owned catalogue. The following semantic checks enter implicit failure:

| Check | Required meaning |
|---|---|
| Bounded integer arithmetic and signed negation | Fail when the mathematical result is outside the operation's promoted result domain. |
| Division and remainder | Preserve zero-divisor checks and the numeric family's valid-operand rules. Signed minimum // -1 overflows. Signed minimum % -1 remains zero. |
| Exponentiation | Preserve invalid exponent and bounded result checks, including the Dec family's non-negative integer exponent rule. |
| Binary floating-point results | Preserve finite-value checks after semantic rounding. Finite rounding, subnormals and underflow to finite zero are not failures. |
| Compound assignment | Check ordinary arithmetic and the separate conversion back to the target type. Write only after both succeed. |
| Range loops | Preserve invalid runtime step checks and every required checked update. Finishing a loop must not require an overflowing endpoint sentinel. |

This changes failure delivery, not which arithmetic results are valid. Preserve
Int/Float identities and all numeric profiles. U8 + U8 has result U32, so 256
is not an overflow of that expression. Narrowing it back to U8 is a separate
boundary. Preserve Dec scale rounding/exactness rules and Byte's lack of
arithmetic. Check results against semantic domains, not Wasm carrier widths.

Explicit fallible casts retain their declared built-in Error! outcome. Collection
get/set/remove, fixed push, map operations and other existing typed APIs retain
their declared error contracts. They do not become implicit merely because the
compiler implements them. Ordinary false Bool values and none are not failures.

Keep existing external-value validation contracts. A foreign implementation
violating its declared value contract or throwing unexpectedly does not gain
an automatic recoverable adapter through this plan.

### 3. Resource and panic boundary

Growable push retains its no-error-slot source contract and fatal allocation
exhaustion policy. Runtime string/template construction, copies, map growth,
Dec storage and other allocations do not acquire recoverable effects.
Stack exhaustion, physical storage/capacity limits, host termination and
unexpected runtime-integrity faults remain outside this system unless an
existing individual API already declares an ordinary recoverable limit error.

This exclusion does not remove allocator, address, bounds or capacity checks
needed for memory safety. Failed allocation never becomes valid storage and
physical size arithmetic never silently wraps. Recovery and completed cleanup
are not guaranteed after fatal termination.

assert remains the only source mechanism for deliberately requesting panic.
It is statement-only, always checked and assert(false, ...) is terminal.
A local catch may end with assert(false, message). Export closure does not
mean trap-free execution and does not prohibit explicit assertions.

An assertion's condition is evaluated normally. Failure while computing that
condition is not a false assertion result. Preserve the message's lazy
failure-edge evaluation and prohibition on escaping evaluation. Detect inferred
failure in assertion messages too. Handle fallible message work beforehand and
pass an infallible value. There is no new panic effect or panic-on-failure
function directive.

### 4. Function boundary table

| Function contract | Treatment of escaping implicit failure |
|---|---|
| Private, no explicit error slot | Infer and propagate it to the same-module caller. |
| Built-in Error! slot, private or exported | Automatically materialise the corresponding Error and return it through that slot. A local catch takes precedence. |
| Custom E! slot, private or exported | Require local recovery or explicit mapping to E. No automatic conversion and no hidden third channel. |
| Exported, no error slot | Require all implicit failure to be discharged before returning. A remaining possibility is a source diagnostic. |
| Compiler-generated start | Use the built-in Error! rule and pass the typed outcome to its generated host caller. |

The table applies to zero and multiple success returns as well as one result.
A custom-error function must satisfy its rule even when currently private.
Returning an error from one branch does not discharge a different unchecked
failure path. A handler that itself can fail contributes failure to the
surrounding function.

Built-in recognition uses canonical type identity. A transparent alias of
Error is still Error. An unrelated struct named similarly, a wrapper or a
matching method shape earns no privilege. User traits, error conversion hooks
and generic adapter searches are excluded.

Compilation establishes each callee's meaning independently. An Error! caller
does not retroactively recompile a callee's arithmetic under a different mode.
A private bare-failure callee already has checked semantics. The caller merely
routes the reported outcome.

### 5. The boundary is semantic, not physical

A module is the directory-scoped semantic compilation unit, not an ordinary
source file, a backend partition or a generated Wasm binary. Private calls
between ordinary files in that module may propagate implicit failure.

Every callable exposed through a module's public surface must satisfy the
export rule. This includes exported receiver methods, public generic
functions, support-package facades, external package facades and re-exports.
A private helper reached by an exported function contributes to that function's
check. Export validation is not postponed until some downstream caller uses it.

Generation, inlining, linking or materialising a generic body in a consumer
must not erase the originating public contract. Validate public generic
contracts using their declared requirements and validate concrete generated
work before publication. No effect parameter or generic failure polymorphism
is added. Private specialised work may use internal propagation while its
exposed callable still discharges failure at the declared boundary.

Preserve existing trait requirement and conformance checking. A method's typed
error contract cannot be widened secretly through conformance or a generic
bound. Reuse resolved call/evidence facts and include any concrete implicit
failure in the enclosing body's analysis.

Moth/foreign crossings expose only declared supported results. This feature
adds no raw failure channel to WIT or JavaScript binding signatures. Unsupported
foreign result shapes remain ordinary capability diagnostics. Existing source
visibility and acyclic module rules remain unchanged.

### 6. Expression-wide catch grammar

catch terminates the current ordinary expression at a lower precedence than
its operators. The parser collects the complete protected expression before
finalising its handling contract. Parentheses are not required around the
protected expression.

```moth
value = left * right + offset catch:
    then 0
;
```

The handler covers both arithmetic operations above. It covers evaluation of
receivers, arguments, nested call results and conversions that belong to that
expression. It does not attach just to the last call, operand or operator.

Respect existing comma, parenthesis, newline, template and statement boundaries.
Do not consume a previous argument or a previous statement. Grouping an operand
without a catch inside it does not shrink an outer handler's scope. Authors
can split work into separate handled statements when they need finer recovery.

Reuse existing inline and block catch syntax:

```moth
value = left * right catch then 0

value = left * right catch |err|:
    io.warn(err.message)
    then 0
;
```

This generalises the protected expression, not the placement of arbitrary
statement bodies. Preserve existing allowed receiving sites, inline-line
rules and restrictions on catch bodies in conditions, templates and constants.
No general block expression, anonymous function or new grouped handler is added.

### 7. Catch compatibility and materialisation

Classify the unhandled outcomes of the protected expression after accounting
for nested handling and the existing explicit propagation rules. Infallible
parts contribute nothing. Type equality is canonical, including transparent
aliases.

| Remaining producer kinds | Result |
|---|---|
| Implicit built-in failure only | Accept. A bound err is built-in Error. |
| One or more built-in Error! outcomes only | Accept. Preserve the selected returned Error value. |
| Implicit failure mixed with built-in Error! | Accept under the same built-in Error handling policy. |
| One custom error type E, with no implicit failure | Accept. A bound err has exactly type E. |
| Distinct typed error types, including E and Error | Reject and identify both producers. |
| Custom E! mixed with implicit failure | Reject and suggest splitting or explicit conversion. |

Unbound catch need not construct a source-observable Error for an implicit
failure. Bound catch materialises the appropriate built-in Error. Existing
typed errors preserve their original message, code and identity/provenance
rather than being replaced with a generic failure description.

Retain useful internal cause information when failure crosses private calls.
A may-fail summary bit is not enough to construct the eventual Error. Reuse
closed runtime failure classifications and the existing built-in Error mapping.
Keep user-visible message/code behaviour consistent across JS and Wasm. Extend
that one mapping only where the delivered catalogue lacks a required cause,
with a documented code and parity tests. Do not create a public Failure type,
mandatory stack trace, new Error fields or a diagnostic-ID-as-error-code scheme.

A checked operation, an inferred-failure private call or a typed error producer
makes catch eligible. A literal or an ordinary infallible call alone does not:
report the existing invalid-catch diagnostic rather than creating an effect.
An eligible handler does not become invalid merely because constant folding or
later proof removes its runtime failure path. Do not use backend optimisation
to decide catch legality or its error type.

### 8. Evaluation and explicit propagation

Evaluate protected work once in ordinary source order. The first actual failure
or typed error skips the rest of that expression and enters its handler. The
handler replaces the whole expression's success slots, not one internal
operand. Preserve Boolean short-circuiting and other existing lazy evaluation.

Earlier successful mutations and IO remain visible. A failing operation retains
its existing individual commit guarantees, but a catch is not a transaction.
Temporary values on abandoned paths still follow normal cleanup and borrowing
rules. A handler cannot read a success local that was never initialised.

The error binding is immutable, scoped to the handler and subject to normal
no-shadowing rules. Each reachable recovery path supplies the receiving arity
and types with then, returns normally, returns through E! or ends in a proven
terminal assertion. A no-success operation permits a handler that falls through.
A value-required handler cannot silently fall through.

Failure in the handler goes outward. It never re-enters the same handler.
An ordinary Error! function absorbs such escaping built-in failure as usual.

Explicit typed errors still require catch or postfix !. Under a whole-expression
catch, compatible typed calls feed their successful values into the surrounding
expression without per-call !. Outside a catch, the old explicit propagation
rules remain. In particular, start having Error! does not make typed calls
implicitly propagate.

Preserve the existing conflict rules for mixing an explicitly propagating !,
cast! or optional ? with catch on the same expression. Do not retarget an
explicit function-return escape to the nearest handler. Split such work into
statements rather than inventing an exception to propagation scope. Propagation
inside a called function remains part of that callee's ordinary contract.

### 9. Casts and statement-owned checks

cast expression catch covers the complete operand evaluation and the conversion.
An operand overflow and a conversion Error! may share the handler. This is an
intentional change from cast-only recovery.

```moth
narrow |left U32, right U32| -> U8:
    value U8 = cast left + right catch then 0
    return value
;
```

The example recovers both U32 addition overflow and failure to narrow its result
to U8. It does not mean that U32 arithmetic itself uses an 8-bit domain.

Keep the immediate receiving target and static cast-evidence rules. An outer
catch supplies handling permission for an eligible fallible cast. An infallible
cast inside a protected expression can still have a failing operand. A catch
does not legalise unsupported type pairs, plain fallible casts without handling,
implicit narrowing or backend-dependent conversions. cast! remains the explicit
cast propagation form with a compatible built-in Error! function slot.

Assignment and compound assignment remain statements, not general expressions.
A catch on their right-hand expression protects that expression only. A later
compound arithmetic operation or conversion back into the target remains a
separate implicit-failure source. Evaluate the target place and right operand
once and commit no write until every required check succeeds. Diagnostics should
identify a remaining write-back failure rather than blame an already handled
operand.

To handle a complete compound update locally without a grouped block, put it
in a private helper and catch that call, or compute/check the replacement in
separate statements before assignment. Use the same approach for statement-owned
range checks. Do not add catch-after-loop or catch-after-arbitrary-statement
syntax as a shortcut.

### 10. Constants and proof

Statically known invalid numeric work remains a source diagnostic, including
inside Error! functions and expressions followed by catch. Required constants,
config values, defaults and compile-time templates do not gain a runtime
failure escape. Do not move invalid constant work to runtime to make it catchable.

Use the existing semantic validation and static-control-flow rules. Validate
both branches of ordinary static if as required before selecting active work.
Inactive work contributes no executable failure summary after that selection.
No new whole-program constant evaluator or path-sensitive range solver is required.

Initial semantic discharge uses conservative backend-independent facts already
available from types, constants and static control flow. A programmer guard or
assertion is not an unchecked assumption. An unproved possibility still requires
handling at a closed boundary. Diagnostics distinguish possible failure from a
claim that execution always fails.

Separate semantic acceptance from optional check elision. Release optimisation
may spend more effort proving checks unnecessary than debug optimisation, but
both accept the same source for the same numeric profile and expose the same
error contracts. Elision preserves evaluation, rounding, first-failure order
and required diagnostics. No user mode or directive disables checks by trust.

Native trap substitution for an explicit assertion handler is future work.
This plan keeps the checked failure and authored assertion semantics, including
message evaluation. It does not claim that panic-on-overflow removes a check.

## Source examples to publish and test

These examples describe the accepted new behaviour, not present compiler support.
Compile their equivalents as fixtures when the owning implementation lands.

### Private propagation and public conversion

```moth
product |left Int, right Int| -> Int:
    return left * right
;

export:
    checked_product |left Int, right Int| -> Int, Error!:
        return product(left, right)
    ;

    product_or_zero |left Int, right Int| -> Int:
        return product(left, right) catch then 0
    ;
;
```

An otherwise identical exported product without Error! or catch is rejected
when multiplication may overflow. Merely calling it with small arguments in
one consumer does not repair its public contract.

### Explicit custom mapping and panic handling

```moth
GeometryError ::
    InvalidDimensions,
;

area |width Int, height Int| -> Int, GeometryError!:
    result = width * height catch:
        return! GeometryError::InvalidDimensions
    ;
    return result
;

required_product |left Int, right Int| -> Int:
    return left * right catch:
        assert(false, "product must fit")
    ;
;
```

### Built-in mixed catch, including call arguments

```moth
load_amount |text String| -> Int, Error!:
    return cast! text
;

amount_or_zero |text String, quantity Int, fee Int| -> Int:
    return load_amount(text) * quantity + fee catch then 0
;
```

The catch handles load_amount's returned Error and both arithmetic operations.
A custom error type returned by load_amount would instead require splitting or
mapping because arithmetic contributes implicit built-in failure.

### Top-level propagation

```moth
load_name |missing Bool| -> String, Error!:
    if missing:
        return! Error("Missing name")
    ;
    return "Priya"
;

name = load_name(false)!
[: Hello [name]]
```

The snippet is root-body content under the project's applicable entry rules.
The synthesised start supplies Error! without an authored function wrapper.
Keep purpose/metadata syntax aligned with whichever entry surface is delivered
at activation. This feature does not implement page-purpose migration.

## Entry and host contract

### 1. Fixed implicit start contract

Every executable normal-root start, including a selected synthetic single-file
root, has one built-in Error! slot. Conceptually it returns String, Error!.
Preserve the existing success-fragment slots and insertion metadata rather than
forcing a new concatenated string, tuple or public Result carrier.

start remains compiler-synthesised, non-exported and unavailable to ordinary
source calls or dependency binding. It executes only for its selected entry.
Importing a provider never runs its dormant start. Support modules and API-only
package facades still have no start. A private function does not become an
entry merely because it can fail.

Top-level implicit failure automatically becomes Error. Existing typed Error!
can propagate with !. Custom typed errors require explicit recovery/conversion.
Local catch continues to override automatic conversion. Do not add arbitrary
error-to-String coercion or a special script-only error policy.

A failure ends the invocation's remaining work. It does not produce successful
runtime fragments, substitute an empty String or turn an error into normal
output. A successful entry with no runtime fragments still follows the ordinary
empty-output representation.

### 2. Ownership and other hosts

The compiler owns the signature, checks, failure mapping and explicit result
control flow. The builder owns the generated caller and terminal presentation.
It consumes the typed result rather than reinterpreting HIR or catching normal
Moth errors as host exceptions.

The generated caller handles every entry outcome. On error it reports an
unsuccessful invocation once and stops normal startup completion. There is no
automatic retry, later-statement continuation or rollback. Propagation and
materialisation themselves do not print at every layer.

A terminal execution host reports to standard error and exits unsuccessfully.
Use a nonzero process status independently of Error.code, whose default zero
is still valid error data. An embedding host delivers the failed entry outcome
to its owner. A test runner records runtime failure unless the expected outcome
matches it. These are contracts for existing applicable consumers, not a mandate
to build a new CLI runner, server-side renderer or embedding API.

Compile-time diagnostics, compiler operational failures, compiler bugs and a
compiled program returning Error remain different lanes. Reporting a browser
entry failure does not retroactively fail the completed compilation.

### 3. HTML runtime publication

The HTML builder emits a runtime startup wrapper. Runtime start is not executed
during semantic compilation. In the accepted mixed-target design it remains
JavaScript-owned and can call supported Wasm functions through generated glue.

On success, publish runtime fragments in their planned positions and order,
then finish ordinary startup. Stage all invocation-owned runtime fragment
results until success. On error, discard those unpublished results and follow
the terminal error presentation path. Never read undefined success slots on an
error edge. Preserve the current compile-time fragment/insertion-index model.

Already emitted static HTML remains separate and available in ordinary output.
Publication staging is not a transaction over the DOM or application state.
Earlier explicit IO, document-title changes and host calls remain done.
Do not add automatic retries or describe failed application state as repaired.

Development presentation shows a clear entry error and reports it to the browser
console. Release presentation uses a small generic startup-failure notice,
preserving static content and avoiding raw application Error messages in public
page content. Detailed dev diagnostics are not shipped as production reporting.
Use escaped text or text nodes, never interpret an Error message as HTML.
The fallback is builder-owned and independent of the failed application template.
No configurable recovery framework or user callback API is added.

A wholly static page does not gain mandatory runtime code solely because the
conceptual start contract permits Error!. Runtime wrappers/helpers remain
reachability-driven. Dev-server hot-reload instrumentation keeps its existing
separate policy. An error handler for start covers this invocation only, not
later event callbacks or future asynchronous tasks.

## Development-server reporting

### 1. Required entry-error path

In moth dev, a start Error must be unmistakable even when the static page is
otherwise successful. Use the existing compiler-error presentation as the
visual owner: a prominent dev error view covers the affected page content and
retains hot reload. Keep runtime and compilation titles/categories distinct.
Do not silently settle for a console-only report.

Report in both the affected browser and the server terminal. Reuse dev-client
injection, origin-aware routing and error-page styles/rendering rather than
introducing another overlay framework. Add only the small browser-to-server
report path needed for terminal visibility. The report carries the entry,
build identity and invocation identity plus bounded category/message/code data
and available source information.

The protocol is dev-only and same-origin. Treat browser input as untrusted.
Validate kind, shape, U32 code, current build and known entry. Bound payloads and
per-client reporting, escape display text and terminal control characters, and
reject malformed or stale reports without panicking the server. Respect the
configured origin prefix. Do not accept arbitrary HTML or compiler-ID payloads.
Reporting failures must not recursively invoke the reporter.

A runtime report does not change global build success, block other entries or
make other tabs receive a compile-error page. Deduplicate by build, entry and
invocation. Ignore old-build reports and clear the originating view on a fresh
page invocation through normal reload. Do not clear an existing failed
invocation merely because a compiler rebuild succeeded somewhere else.
Network loss may prevent terminal delivery, so retain the local browser view
and console report without retrying application startup.

### 2. Narrow unexpected startup faults

Include faults that escape the generated startup lifecycle wrapper, including
owned Wasm instantiation/startup work, in a separate reporting category. This
covers only work actually invoked or awaited by that wrapper. It adds no Moth
async surface, global window.onerror hook or global unhandled-rejection capture.
Unrelated scripts, browser extensions and later callbacks remain outside scope.

Use three distinct outcomes:

| Category | Classification rule |
|---|---|
| Entry Error | start explicitly returned its declared Error! outcome. |
| Assertion failure | Compiler-owned assertion identity identifies the fault. Preserve it through generated calls/glue. |
| Unexpected startup fault | Another exception/trap escaped the owned wrapper. It may come from generated code, a host binding or runtime setup. |

Never identify an assertion or compiler bug through message-text matching.
Do not label every RuntimeError or JavaScript exception as a compiler bug.
Unknown faults remain unknown startup faults. Preserve bounded debugging detail
when available, without promising recovery or a diagnostic after total host
termination. Normal typed errors use explicit result branches, not this fault
interception path.

## Compiler implementation contracts

### 1. One producer for semantic failure facts

Extend the existing expression/type and function-summary owners. Record:

- the normal success-slot shape
- pending implicit-failure contributors and their stable source witnesses
- any unhandled typed error producer and its canonical type
- the selected handler or function-boundary disposition

These are retained semantic facts, not reparsed text or a second type system.
Do not replace TypeId with spelling comparisons. Do not manufacture source
Result types or a general effect-row representation.

Resolve same-module call propagation to a conservative fixed point, including
supported recursive call components and generated work. Handled failure does
not escape. Error! and custom E! boundaries do not forward a bare effect.
Preserve a deterministic bounded witness chain for diagnostics. A missing
summary is never treated as proof of no failure.

The canonical module service owns analysis ordering and convergence. Complete
facts and validate exposed boundaries before publishing the semantic interface.
Build code cannot install summaries or rerun local semantic stages. Generated
sidecars follow the same contract. Reuse retained typed AST and the current
normalisation/lowering sequence rather than reparsing signatures or rescanning
source after HIR.

### 2. HIR, ABI and cleanup

HIR makes checked operations, failure edges, handler joins and materialisation
explicit. A private inferred-failure function uses one internal failure lane
instead of a typed error lane, never both as escaping function outcomes. A typed
function exposes its existing single E! lane after handling/conversion.

Keep ordered result slots and edge-specific initialisation. Success values do
not exist on failure edges and error values do not exist on success edges.
Handlers join each required result slot through the existing control-flow
representation. No fabricated aggregate extraction path or global last-error
variable is permitted.

Private cross-backend calls inside the same semantic module may need generated
internal transport. That is not a new public or foreign result contract. Reuse
ABI owners to encode status/cause or Error results and preserve source ordering.
Source-module boundaries stay closed even if the linker later inlines them.

Include failure edges in borrowing, alias/provenance analysis, retained-edge
summaries and cleanup planning. Earlier committed mutations remain visible.
Failed compound writes and existing fallible collection operations preserve
their own old-state guarantees. Partial temporaries are cleaned up according
to the ordinary memory plan. The feature neither requires GC nor bypasses
borrow/lifetime validation on an error path.

### 3. Backends and compatibility

JS and Wasm consume frontend-selected numeric domains and failure dispositions.
They do not infer them from the surrounding function name or Error spelling.
Reuse the numeric checkpoint's checked operations and helpers. Catching an
implicit failure does not permit bare JS numeric coercion, BigInt exception
handling or Wasm trapping instructions to substitute for the prescribed result.

Implement parity for the numeric and typed-error subset supported at activation.
Unsupported reachable combinations get the existing target diagnostic, not a
trap fallback or silent semantic weakening. Full Wasm aggregate/memory work
remains separately scheduled. The plan cannot claim complete Wasm runtime
coverage where those required facilities are still unavailable.

Update existing public, implementation, dormant-entry, runtime-dependency and
physical compatibility facts where these semantics affect them. No parallel
effect fingerprint or new general cache is added. Private failure changes can
invalidate callers/entry output even when a public signature stays the same.
Pre-change compiled artefacts are not silently interpreted under the new ABI.

## Diagnostics contract

Each new source error uses the established structured diagnostic lane. Preserve
source spans, canonical types and a typed reason. Render prose at the normal
renderer boundary. Malformed programs, handler mismatches and unsupported
capabilities are never compiler bugs or invariant panics.

| Situation | Required explanation and evidence | Primary corrective guidance |
|---|---|---|
| Export without E! has escaping implicit failure | Name the exported callable, point to its signature/export exposure and show the direct operation or deterministic private-call witness. State that failure may occur and cannot cross the boundary unrepresented. | Add built-in Error! or handle the failing computation locally. |
| Custom E! body leaves implicit failure | Show the custom slot and failing source. Explain that automatic conversion exists only for built-in Error, not E. | catch and return! an E value, recover locally or deliberately change the API to Error!. |
| Mixed incompatible catch producers | Show both source expressions and their error types/kinds. Distinguish allowed Error-plus-implicit mixing from disallowed custom mixing. | Split the expression or explicitly map errors. |
| Typed error has no handler/propagation | Point to that call/cast rather than silently treating it as implicit failure. | Use catch or compatible existing !. |
| Catch or propagation is misplaced | Identify the actual expression boundary and conflicting operator/context. | Move work to an allowed receiving statement or split it. |
| Handler result is incomplete | Show required arity/types and a reachable non-producing branch. | Produce with then, return, return! or use a valid terminal branch. |
| Checked compound write-back still escapes | Distinguish RHS recovery from the later arithmetic/conversion and label the target type. | Use Error!, a handled helper or separate checked computation and commit. |
| Assertion message can escape | Show the message expression and the escaping implicit/typed/optional operation. | Handle it beforehand and pass its infallible result. |
| Static numeric invalidity or unsupported target | Preserve its existing category and precise source reason. | Correct the value/type or supported capability, not catch an impossible compile-time value. |

Do not suggest wrapping, unchecked mode, resource recovery, a grouped if:
handler or a public fails marker. Assertion is an explicit user choice, not
the default repair suggestion. Do not advise adding Error! beside an existing
custom E! slot and thereby create two error channels.

Avoid a diagnostic cascade at every private hop. Retain the original source
cause and enough call context to explain the closed boundary. Keep ordering
stable across parallel compilation and generated specialisation. Invalid
exports are diagnosed even when an entry does not execute them.

## Change-owner map

Confirm exact files at activation because numeric work may reorganise internals.
The following existing owners are starting points, not permission to create
parallel subsystems:

| Owner | Responsibility in this delivery |
|---|---|
| `src/compiler_frontend/ast/expressions/` and `ast/statements/` | Expression completion, catch scope, typed producer composition, receiving rules and compound/loop boundaries. |
| `src/compiler_frontend/ast/module_ast/` | Function/start contracts, active-body summaries and normalisation. |
| `src/compiler_frontend/headers/`, `declaration_syntax/` and `datatypes/` | Retained signature and canonical Error identity integration where required. |
| `src/compiler_frontend/builtins/` and numeric semantic owners | Closed failure causes and shared Error mapping, preserving numeric operations. |
| `src/compiler_frontend/module_compilation/` and `public_interface/` | Fixed-point orchestration, generated work and closed export validation. |
| `src/compiler_frontend/hir/` and memory-analysis consumers | Explicit continuations, result/handler validation, error-edge provenance and cleanup. |
| `src/compiler_frontend/compiler_messages/` | Typed reasons, source witnesses and actionable rendering. |
| `src/backends/` and `src/projects/html_project/` | Numeric delivery, ABI/glue, staged fragments and terminal startup handling. |
| `src/projects/dev_server/dev_client.rs`, `http.rs`, `state.rs`, `error_page.rs` and `sse.rs` | Scoped client reports, presentation reuse, terminal delivery and retained reload. |
| `tests/cases/`, subsystem tests and `src/compiler_tests/` | Source behavior, internal invariants, backend results and browser/server integration. |
| `docs/src/docs/` and permanent compiler/build references | Accepted contracts, Basic/Advanced teaching, examples and truthful progress. |

## Documentation migration obligations

The advanced `docs/src/docs/errors/runtime-failures-and-termination.mtf` reference
already exists and is wired into the Errors page. Maintain it rather than
creating another catalogue. Keep its rapid-development completeness disclaimer,
backend-comparison qualifications and explicit resource-exhaustion exclusion.

Transfer all accepted rules into the following permanent owners. Each update
includes paired Basic material, page introductions and displayed examples where
affected, without duplicating the full design in every page.

| Permanent area | Required correction |
|---|---|
| errors/error-values, error-returns, propagation, catch-and-recovery and assertions | Internal failure versus typed data, Error privilege, whole-expression scope, compatible mixing, custom mapping and explicit panic. |
| numbers/checked-arithmetic, operators and related numeric pages | Default failure delivery, unchanged result-domain checks and conservative proof. Remove default trap claims for included numeric failure. |
| casts/cast-syntax, fallible-casts and cast evidence teaching | Operand-plus-conversion catch scope, allowed built-in mixing and unchanged target/evidence rules. |
| functions, receiver methods, generics, traits and public API/export pages | Module-local propagation and closed exported signatures, including generated/conformance routes. |
| loops, collections and maps | Statement-owned numeric checks versus unchanged typed API errors and fatal allocation limits. |
| project-structure/module-roots, entry-runtime-and-fragments and HTML output pages | start Error!, top-level !, success-only runtime publication and generated terminal handling. |
| compiler-design-overview and compiler data-layout references | Failure fact ownership, result lanes, materialisation, semantic summaries and separation from compiler diagnostic/infrastructure/bug lanes. |
| build-system-design and dev-server documentation | Entry outcomes, presentation ownership, per-invocation reports and no global build-state poisoning. |
| design-scope and directive references | Resource recovery excluded. Grouped handling, rewind, richer effects and wrapping remain uncertain ideas. $checked remains the separate proposed borrow-proof budget. |
| language cheatsheet, language overview, README and getting-started examples | Remove superseded examples/claims and teach terse top-level propagation without declaring unimplemented support. |
| progress matrices | Track actual frontend, backend, entry and dev-report support separately while implementation proceeds. |

Search all documentation, examples, first-party source packages and fixtures for
old numeric trap fallback, infallible start, cast-only catch and call-only catch
assumptions. Classify each occurrence instead of blanket textual replacement.
Preserve tests of genuinely fatal resources/assertions. A code-block renderer
accepting text is not proof that its Moth example compiles or executes correctly.
Use real source fixtures for the new teaching examples.

Do not rewrite generated docs/release files by hand. Update source references,
regenerate through the existing documentation build and obey the repository's
rules for recording generated output. Later plans must consume the published
contract and must not restore earlier infallible-entry or trap-default assumptions.

## Implementation phases

Each phase has a reviewable deliverable. Follow the repository's test-first
workflow for code changes: add a focused failing case, confirm its intended
failure, make the smallest complete change, run owning tests and review the
changed path. Coordinate adjacent consumers before committing a semantic cutover.
Combine tightly coupled edits rather than add compatibility shims, fake summaries
or alternate public modes just to make an intermediate commit look complete.

### Phase 0 - Activation inventory

- [ ] Confirm the complete numeric prerequisite and record the activation baseline
  in working notes. Read the authority routes and current progress.
- [ ] Inventory numeric failure sources/mapping, parser/catch owners, call-summary
  sequencing, ABI/result shapes, start synthesis, fragment publication and the
  dev-server reporting/presentation path.
- [ ] Record the actual JS/Wasm coverage and the exact test selectors supported
  by the current test command. Identify shared first-party package work.
- [ ] Create a contract-to-owner/test checklist using this plan's numbered rules
  and acceptance matrix. Record documentation conflicts to replace in Phase 1.
- [ ] Run the existing baseline gates. Preserve unrelated failures as evidence
  rather than silently relaxing gates or pinning a stale SHA in this file.

Exit: every changed semantic fact has a producing owner and every consumer,
existing fixture family and unsupported backend case is identified.

### Phase 1 - Publish permanent accepted design

Acceptance coverage: P01 and the documentation migration table.

- [ ] Apply the documentation migration table, starting with canonical error,
  numeric/cast, compiler/build and entry contracts.
- [ ] Update teaching examples and the existing advanced termination reference.
  Mark accepted-but-undelivered behavior through the progress/status surfaces.
- [ ] Remove conflicting old final-design statements. Retain implementation
  notes only in their status owners and narrowly identified migration notes.
- [ ] Run the documentation release-build gate and inspect rendered tables,
  source examples, links and entry/error pages.

Exit: permanent references state one coherent target model. This plan remains
the complete work contract until closeout verifies every transferred rule.

### Phase 2 - Failure facts and expression handling

Acceptance coverage: F01, F03, C01-C11, D01 and D04 at the frontend/semantic level.

- [x] Add parser/typing cases for unparenthesised whole-expression catch,
  arguments/receivers, multiple compatible typed calls and cast operands.
- [x] Extend the current expression completion/receiving owner to retain pending
  producer facts until the outer catch is known. Resolve compatibility once.
- [x] Add private-call inferred failure summaries and deterministic convergence
  inside the canonical module service, including supported recursion and
  generated work. Reuse existing canonical type and call identities.
- [x] Resolve nearest local handling, automatic Error! conversion and custom E!
  rejection without introducing a third escaping function channel.
- [x] Preserve propagation, option, assertion-message, constant and statement
  boundary rules. Add focused tests for each accepted/rejected combination.

Exit: one typed semantic representation describes every handler and function
outcome. No source-defined result wrapper, failure trait or new directive exists.

### Phase 3 - Explicit lowering and backend delivery

Acceptance coverage: F01, F04-F07, C01-C11, B01, B03 and M01 at runtime/HIR level.

- [ ] Add HIR continuation and materialisation cases before modifying lowering.
  Include failed arguments, abandoned temporaries, handler failures and failed
  compound write-back.
- [ ] Lower expression work once in order, join normal success slots and encode
  internal failure/typed errors through the current ABI owners.
- [ ] Thread new error edges through validation, borrowing, result provenance,
  retention summaries and cleanup. Preserve individual commit guarantees.
- [ ] Update supported JS and Wasm paths together. Remove superseded numeric
  trap-default dispatch and helpers made obsolete by the new delivery model.
- [ ] Add parity fixtures and explicit target-rejection fixtures for unavailable
  combinations. Keep resource faults and assertions outside normal catch.

Exit: supported generated programs implement the new outcomes and ordering.
Unimplemented backend facilities are diagnosed, not disguised as numeric traps.

### Phase 4 - Public boundary validation and diagnostics

Acceptance coverage: F02-F03, D02-D04 and M02.

- [ ] Add export/custom-slot rejection cases with expected labels, reason and
  useful repair guidance, including private-call chains and receiver exposure.
- [ ] Validate exposed source and generated contracts before interface
  publication. Cover re-exports, package surfaces and foreign projections.
- [ ] Install structured diagnostics for the complete diagnostic table. Keep
  bounded deterministic witness paths and eliminate redundant private-hop spam.
- [ ] Update the existing interface/implementation/entry compatibility owners
  and tests for stale summaries, generated identities and old artefacts.

Exit: a successful published module cannot leak an unrepresented recoverable
failure, and diagnostics explain both the source and the valid repairs.

### Phase 5 - Fallible start and HTML publication

Acceptance coverage: E01-E05.

- [ ] Add top-level cases for implicit arithmetic failure, propagated Error!,
  local recovery, custom-error mismatch and an Error whose code is zero.
- [ ] Give every applicable synthetic start its built-in Error! slot without
  altering dependency activation, API-only roots or success-slot representation.
- [ ] Make the generated HTML caller branch on entry outcome and publish all
  invocation-owned runtime fragments only on success.
- [ ] Implement terminal reporting and the generic release fallback independently
  of application templates. Preserve static content and earlier explicit IO.
- [ ] Update current harnesses/consumers to inspect outcomes, not only stdout or
  fragments. Test once-only execution and absence of retries or partial output.

Exit: short top-level programs remain terse and a failed invocation is never
reported as successful initialization.

### Phase 6 - Dev browser and terminal reports

Acceptance coverage: V01-V04.

- [ ] Add dev-client/server tests for returned entry Error, report correlation,
  stale builds, duplicate delivery, unrelated tabs and retained hot reload.
- [ ] Reuse the existing dev error presentation for a prominent page-local view.
  Add the bounded same-origin reporting path and terminal formatting.
- [ ] Add the narrowly scoped startup-fault wrapper and typed assertion
  identification. Preserve three distinct categories without message heuristics.
- [ ] Test hostile text, malformed reports, unavailable transport, configured
  origin prefixes and reporting failures. Verify no global browser-error hooks
  or global build-failure transition was introduced.
- [ ] Verify production output has no dev endpoint/client reporting dependency.

Exit: a start Error is obvious in moth dev, stays local to its invocation and
never gets confused with a successful build, compiler diagnosis or unrelated JS.

### Phase 7 - Conservative proof and complete migration

Acceptance coverage: B02 and P01, then the full migration inventory.

- [ ] Reuse conservative semantic facts to discharge only established safe
  paths. Add positive and negative export-proof tests with runtime operands.
- [ ] Preserve optional check elision in its owning optimisation layer. Compare
  debug/release acceptance and runtime outcomes under the same numeric profile.
- [ ] Update affected tests, first-party packages, runnable examples and generated
  fixture expectations. Avoid blanket Error! additions that hide intended API
  recovery, and avoid replacing recoverable tests with assertions.
- [ ] Recheck the full documentation migration table and current support claims.
  Measure non-recording performance only after semantic tests pass.

Exit: source acceptance and public contracts are target/build-profile stable,
and the codebase/examples consistently use the accepted model.

### Phase 8 - Final cross-scope review and retirement

Acceptance coverage: every row in the acceptance matrix.

- [ ] Run the acceptance matrix below across supported backends/profiles and
  the repository's complete code-bearing gate, then the docs release build.
- [ ] Perform the AGENTS.md Slice review and a cross-stage ownership review.
  Check duplicate handlers/error mapping, stale numeric trap fallbacks, public
  ABI changes, failure-edge cleanup and reporting-scope leaks.
- [ ] Confirm every design rule in this file exists in a permanent owner and
  every outcome has a real test. Record genuine future ideas as unsettled, not
  silently dropped implementation obligations.
- [ ] Update progress matrices and index entries where facts changed. Mark
  affected audit records stale according to repository policy, without claiming
  a new formal audit merely from implementation testing.
- [ ] Remove this plan and its roadmap bullet in the completion commit, after
  transferring durable decisions. Do not commit a completed placeholder plan.

## Required acceptance matrix

Use existing primary owners where they already cover the behavior. Proposed
new source-case families belong under tests/cases/implicit_failure_* and
entry_error_*. These names are organisational suggestions, not assertions that
those directories already exist. Keep internal representation tests local and
use current compiler integration/dev-server harnesses for cross-owner behavior.

| ID | Observable contract to prove |
|---|---|
| F01 | Private multiply helper propagates through at least two calls and materialises the original cause at Error!. No ! is needed at those bare-failure calls. |
| F02 | The same possibly overflowing helper behind an exported success-only wrapper is rejected with a signature label and a source/call witness. |
| F03 | Custom E! rejects unhandled implicit failure even in a private function. catch plus return! E succeeds with only one error lane. |
| F04 | Local recovery and a terminal assert handler discharge the effect. Assert remains unrecoverable and checked in release. |
| F05 | Overflow, zero division/remainder, signed minimum // -1, valid minimum % -1, invalid exponents and non-finite float results keep their distinct outcomes. |
| F06 | U8 + U8 yielding 256 is valid U32. A later failing U8 compound write-back leaves the original target unchanged. |
| F07 | Required range updates fail normally, while inclusive endpoint completion constructs no out-of-range sentinel. |
| C01 | left * right + offset catch handles either arithmetic failure without parentheses. Each operation runs at most once in source order. |
| C02 | A failing receiver/argument prevents the outer call from executing. Its failure reaches the outer whole-expression catch. |
| C03 | Multiple Error! producers and implicit failure share one catch. Typed errors preserve original message/code, including code zero. |
| C04 | Multiple same-custom-type producers can share catch when no implicit failure remains. Custom-plus-implicit and two distinct typed errors are rejected. |
| C05 | cast left + right catch recovers operand overflow and narrowing error. Invalid cast evidence remains a source error. |
| C06 | Bound implicit catch receives Error. Unbound catch recovers without requiring an observable Error construction. |
| C07 | Zero/one/multiple success recovery obeys arity, scope and terminality. An uninitialised success value is never exposed on failure. |
| C08 | Handler failure escapes outward once. Earlier committed mutations remain. Later operands and short-circuited work do not execute. |
| C09 | Commas, parentheses, inline-line limits, templates, conditions, constants and explicit !/? conflicts retain their intended expression/placement boundaries. |
| C10 | RHS catch does not swallow a later compound write-back failure. A caught private update helper handles the complete update. |
| C11 | Same-canonical Error aliases receive built-in treatment. Nominal lookalikes and custom wrappers do not. |
| D01 | Source-known invalid arithmetic remains a compile-time diagnostic despite catch or Error!. Valid constants/finite rounding retain current rules. |
| D02 | Export witnesses are deterministic across parallel/repeated compilation and supported recursive/generated graphs. Missing summaries never imply infallibility. |
| D03 | Exposed methods, generic instances, re-exports and package/foreign surfaces cannot leak a bare failure through an alternate route. |
| D04 | Implicit failure in an assertion message is diagnosed as an escaping message computation. An infallible prepared message remains lazy at assertion failure. |
| E01 | Single-file and directory entries accept top-level Error! propagation and implicit failure without an authored start wrapper. Imports never execute provider start. |
| E02 | Entry failure after producing an early runtime fragment publishes none of that invocation's runtime fragments. Static HTML remains separate. |
| E03 | Successful fragments retain insertion order and execute once. A wholly static page acquires no error-runtime dependency solely from the conceptual signature. |
| E04 | Returning Error with code zero is failure. A failed entry is distinct from compile failure, assertion and host/runtime fault in the harness. |
| E05 | Release fallback renders fixed safe text, not application Error HTML. Earlier explicit IO persists and startup is not retried. |
| V01 | Dev entry Error covers otherwise successful static content, reaches the terminal once and keeps hot reload connected. |
| V02 | Build/entry/invocation correlation rejects stale reports and isolates other tabs/pages. The global build state stays successful. |
| V03 | Identified assertion and unknown startup fault are distinct from returned Error. An unrelated script fault or later callback is not intercepted. |
| V04 | Hostile diagnostic text, malformed/oversized reports, origin-prefix deployment and transport failure do not inject markup, panic the server or recurse in reporting. |
| B01 | Supported JS/Wasm implementations agree on values, causes, first-failure order and cleanup under all delivered numeric profiles. Unsupported paths diagnose explicitly. |
| B02 | Debug/release accept the same source and public contracts. Proof-based check elision changes no observable outcome. |
| B03 | Growable allocation exhaustion and host/stack faults are not catchable Moth failure. Test classification with safe targeted fault injection, not destructive machine exhaustion. |
| M01 | Borrow/provenance and retained-edge obligations remain valid through handled and propagated failures, including abandoned temporaries and failed mutations. |
| M02 | Existing interface/entry/physical compatibility invalidates changed semantics without a second cache or donor-local identity leak. |
| P01 | Canonical and Basic docs, cheatsheet, runnable examples and progress agree. The advanced termination reference remains linked and qualified. |

Include boundary values for every supported numeric family and all four
Int/Float profile combinations where applicable. Use runtime inputs to test
runtime failure rather than accidentally exercising constant diagnostics.
Assert error codes/types and observable state, not only that some error occurred.
A skipped or unsupported lane is reported separately, never counted as passing.

## Final gates and deferred scope

For implementation, use the current repository validation guide and justfile.
The required code-bearing gate is `just validate`. Also run
`moth build docs --release`, or `cargo run --quiet -- build docs --release`
when using Cargo. Targeted cases and static document checks do not replace these
gates. Record exact failures and unavailable tooling honestly. Keep benchmark
history unchanged unless separately requested.

The plan/roadmap-only preparation PR has the documentation-only gate. It does
not claim the implementation or any new language example is supported already.

Deferred and uncertain, with no syntax commitment: explicit wrapping, other
wrap/trap controls, native-trap optimisation of assertion handlers, grouped
handling such as conditionless if:, possible rewind paired with constrained
effects and richer effect restrictions. $checked remains a separate proposed
borrow-proof-budget feature and has no numeric meaning here.

Excluded rather than deferred: recoverable allocation/stack/environment
exhaustion, unsafe check bypasses, automatic backend-dependent arithmetic,
user-defined failure-conversion traits, public bare-failure signatures, a third
escaping error channel and general exception recovery. No implementation slice
may reintroduce these through convenience helpers or silent backend fallbacks.
