# Core time package plan

## Role and authority

`@core/time` separates elapsed time from wall-clock time. It exposes three opaque handle types -
`Duration`, `TimeMark`, `Timestamp` - and free functions over them, with Core origin, ExternalBinding
backing and no prelude alias.

Canonical source semantics belong to `docs/src/docs/packages/core/time/time.mtf` and the canonical
`Float`, `Error` and external-binding references. This living plan records the audited implementation
state, the contract gaps, the decisions a v1 needs and a verified touch map. It accepts no public API
and no semantics: every open item below must be settled with the user and published in the canonical
reference before implementation.

### Current-state capsule

```text
STATUS: design checkpoint delivered; no implementation slice accepted
CURRENT_SLICE: none - this plan is the audit result, not an approved change
BLOCKERS: eleven unsettled semantic decisions below, four of which already have a demonstrable current behaviour that a new test would silently promote to contract; the correction set also needs a decision on who owns binding error-code allocation
NEXT_ACTION: settle the decision table with the user, publish the accepted contract in `time.mtf`, then implement the validity and portability corrections before any arithmetic surface
```

## Current surface

Registered in `src/builder_surface/core_packages/time.rs`: three opaque types, seventeen functions,
no constants. Every function has a JS lowering and `wasm: None`; sixteen are inline expressions and
one is a runtime helper.

| Group | Functions | Lowering |
|---|---|---|
| Monotonic | `mark_now`, `elapsed_since`, `duration_between` | `globalThis.performance.now()`, `(globalThis.performance.now() - #0)`, `(#1 - #0)` |
| Wall clock | `timestamp_now`, `timestamp_from_unix_seconds`, `timestamp_from_unix_milliseconds`, `timestamp_from_iso_string` | `Date.now()`, `(#0 * 1000.0)`, `#0`, runtime helper |
| Duration construction | `duration_from_seconds`, `duration_from_milliseconds` | `(#0 * 1000.0)`, `#0` |
| Duration helpers | `as_seconds`, `as_milliseconds`, `is_negative`, `abs`, `clamp` | `(#0 / 1000.0)`, `#0`, `(#0 < 0)`, `Math.abs(#0)`, `Math.min(Math.max(#0, #1), #2)` |
| Timestamp helpers | `unix_seconds`, `unix_milliseconds`, `to_iso_string` | `(#0 / 1000.0)`, `#0`, `(new Date(#0)).toISOString()` |

Uniform contract facts, all read from registration: every parameter is positional-only with shared
access, every success slot is fresh, and `timestamp_from_iso_string` is the only function with an
error channel.

## Implementation notes

### Opaque handles are compile-time only

All three types register `ExternalAbiType::Handle`, which maps to no Moth scalar type
(`src/compiler_frontend/external_packages/abi.rs`) and is interned as a distinct nominal external
identity per type. In emitted JavaScript a `Duration`, `TimeMark` or `Timestamp` is a bare `number`:
nothing wraps, brands or tags it, unlike `@core/io`'s `Input`, which is a real object. The abstraction
exists in the type environment and in diagnostics, and is erased at runtime.

That erasure is why every inline lowering can do arithmetic on a handle directly, and why the unit
carried by each handle - milliseconds today - is an implementation fact rather than a stated contract.

### The one runtime helper

`src/backends/js/package_bindings/core/time.rs` emits `__moth_time_timestamp_from_iso_string`, which
calls `Date.parse`, detects failure with `Number.isNaN` and hand-builds
`{ tag: "err", value: __moth_make_error("Invalid ISO timestamp", 400, null, null) }`. It is emitted
only when the function is referenced, and is inventoried for the first-party JavaScript guard.

### Finite-result boundary

The shared boundary `__moth_float_validate` (builtin code 304) is applied by the single-Float success
gate in HIR, so it fires on exactly four Time functions: `as_seconds`, `as_milliseconds`,
`unix_seconds`, `unix_milliseconds`. Opaque-returning functions and every `Float` *argument* bypass
it, so a non-finite `Float` reaching `duration_from_seconds` is unchecked by the package.

### No operator surface

Time values support no arithmetic and no comparison. The AST arithmetic policy accepts only
`Int`/`Float` pairs, the comparison policy rejects same-type non-comparable categories, and
`TypeEnvironment::supports_runtime_equality` returns false for external types. There is no
registration hook for an operator, so any arithmetic must be a named function.

## Audited defects and their owners

Each item was read at its cited owner. Nothing here is fixed by this plan.

| # | Defect | Owner | Evidence |
|---|---|---|---|
| 1 | The parse helper returns error code `400`, which is absent from `BuiltinErrorCode` (assigned values top out at 305) and has no `default_message` entry. The canvas binding uses the same unowned family (400/404/409/500); collections, maps, casts and numeric helpers all allocate from the enum. | Time helper, but the real decision is who owns binding error-code allocation | `src/backends/js/package_bindings/core/time.rs`, `src/compiler_frontend/builtins/error_codes.rs` |
| 2 | The helper hand-writes the failed carrier instead of calling the canonical `__moth_error_result(message, code)`, which produces byte-identical output. `@core/io`'s input helper repeats the same pattern with code `500`. | Time helper; canonical owner is `src/backends/js/runtime/errors.rs` | both files read |
| 3 | `timestamp_from_iso_string` declares `Abi(Utf8Str)` but passes the raw argument to `Date.parse` without the canonical String content boundary `__moth_string_value`, which every `@core/text` helper applies. A reactive template reaching this argument coerces to `"[object Object]"` and fails as invalid input. | Time helper; boundary owner is `src/backends/js/runtime/strings.rs` | both files read |
| 4 | `to_iso_string` is documented and registered infallible, yet lowers to `(new Date(#0)).toISOString()`, which throws a host `RangeError` for a non-finite or out-of-range instant. Nothing contains that exception: the JS backend's only `try`/`catch` wraps a fallible Moth function and rethrows anything that is not the result-propagation sentinel. The path is reachable from ordinary accepted source - `timestamp_from_unix_seconds(100000000000000.0)` followed by `to_iso_string` compiles with no errors or warnings. | Time registration plus the host-exception policy gap | `time.rs`, `src/backends/js/js_function.rs`; `(new Date(1e17)).toISOString()` throws `RangeError: Invalid time value` on the harness interpreter |
| 5 | A successful parse is unvalidated: any finite `Date.parse` result becomes a `Timestamp` with no range check. | Time helper | helper body |
| 6 | Unary minus on any non-numeric value, including a `Duration`, reaches HIR as a plain `HirUnaryOp::Neg` and is reported as `MOTH-INFRA-0001` "Plain HirUnaryOp::Neg must be lowered through HirStatementKind::NumericOp" - an internal-invariant lane for ordinary user source. Reproduced on `-duration`, `-"moth"` and `-true`, so this is a general operator-policy defect, not a Time defect. | `src/compiler_frontend/ast/expressions/eval_expression/operator_policy/unary.rs` returns `Ok(operand)` unconditionally for `Negate`; rejection must happen there | `cargo run -- check` on a three-line project for each of the three operand types |
| 7 | `TimeFunctionSpec` plus a local `shared_param` helper duplicate the per-package registration shim that Math and IO also carry, while `ExternalFunctionSpec` and `external_success_returns` already exist as the shared shapes. | package registration modules | `time.rs`, `math.rs`, `io.rs`, `definitions.rs` |

## Decisions the v1 needs

The canonical reference states signatures, the monotonic sentence, the single-fallible sentence, the
access and return contract, the backend lowering list, the opaque-type prohibitions, the five
unsupported source forms and the deferral list. It states no other semantics, so every row below is
unstated there.

| # | Decision | State | Note |
|---|---|---|---|
| 1 | Accepted timestamp grammar | open, behaviour already observable | `Date.parse` accepts far more than ISO 8601: date-only strings, a space instead of `T`, surrounding whitespace and `"01/02/2024"` all parse |
| 2 | Timezone requirement | open, conflicts with the documented type | `Timestamp` is documented as a UTC instant, yet a designator-less or whitespace-padded string parses as *local* time, so the same source yields different instants on different machines |
| 3 | Invalid calendar values | open, conflicts with the documented failure promise | `"2024-02-30"` is accepted and rolled over to 1 March rather than returning through `Error!` |
| 4 | Fractional-second precision | open | `Float` in and out, with millisecond naming as the only hint |
| 5 | Representable timestamp range | open | no bound stated; no range check anywhere |
| 6 | Representable duration range | open | `abs` of the most negative representable value is unaddressed |
| 7 | Rounding policy | open | for construction, extraction and formatting |
| 8 | Finite-value invariants through opaque values | scope settled, Time application open | the language rule is canonical and the sibling Math reference states its resolution explicitly; `time.mtf` does not, and nothing covers non-finite `Float` arguments |
| 9 | Formatting failure behaviour | open, demonstrable failure mode | defect 4 above: infallible signature over a throwing host call |
| 10 | Host exception containment | open | no canonical sentence, and no mechanism: the only `try`/`catch` the JS backend emits wraps a fallible *Moth* function and rethrows anything that is not the result-propagation sentinel, so a host exception from an external call escapes even inside a fallible caller |
| 11 | Monotonic mark origin and comparison restrictions | open | monotonicity is asserted as a type purpose and never as an observable guarantee; `mark_now` is the only way to obtain a mark, so the origin may simply be documented as unobservable |
| 12 | Equality and ordering of the three types | open | already effectively false in the compiler, but undocumented and untested; map-key rejection is owned elsewhere |
| 13 | Arithmetic surface | exclusion settled, surface open | calendar types, time zones, local-time conversion, locale formatting, timers, sleep, intervals, animation callbacks and non-JS lowerings are canonically deferred; which arithmetic functions leave candidate status is a user decision |

### Already pinned by existing coverage

These four assertions can only pass because of current host behaviour on points the canonical
reference leaves open. A decision must ratify or deliberately change each.

- exact `Float` round-trips: `as_seconds(duration_from_seconds(1.5)) == 1.5`, `millis=1500`,
  `abs_sec=0.25`, `clamped_sec=0.5` and the `2.5` family in the free-function case
- the exact ISO rendering `1970-01-01T00:00:00.000Z` / `1970-01-01T00:00:01.000Z`, which is a host
  formatting consequence rather than a documented format
- acceptance of the literal `"1970-01-01T00:00:01.000Z"` and its interpretation as 1000 ms, currently
  the only authority for that spelling being valid
- `elapsed_since` and `duration_between` being non-negative, asserted as `>= 0.0` although the
  reference states only a direction

### Canonical facts with no test owner

Shared access and mutable-access rejection; return freshness; the struct-construction prohibition;
a Time-specific named-argument rejection; inline-lowering shape for `duration_from_milliseconds`,
`as_seconds`, `unix_seconds` and `timestamp_from_unix_seconds`; any non-epoch instant; any
sub-second fraction; UTC formatting independent of the machine timezone. Host-call identity is
asserted only by string presence, so a swap that leaves both `performance.now()` and `Date.now()`
somewhere in the artifact still passes.

## Proposed v1 target

Candidates only, in this order. Nothing here is accepted API.

1. **Validity and portability corrections.** Route the parse failure through the canonical carrier
   with an allocated code, apply the String content boundary, pin the accepted grammar in the
   canonical reference and reject what falls outside it, and resolve the infallible-versus-throwing
   tension for `to_iso_string`. These change no public signature except possibly that one error
   channel, and they are what makes the package portable rather than "whatever the host parser does".
2. **A recorded semantic contract.** Publish decisions 1-12 in `time.mtf`, then convert the four
   pinned assertions into contract-derived coverage and add owners for the unowned canonical facts.
3. **A small arithmetic surface.** Duration addition, subtraction and scaling, timestamp offset and
   timestamp difference are candidates; they need accepted names, signatures, saturation or failure
   behaviour and a range decision first.

Out of implementation scope: calendar types, time zones, local-time conversion, locale-aware
formatting, timers, sleep, intervals, animation scheduling and new opaque-handle infrastructure.

## Previous blockers and rejected approaches

- Reject treating the current `Date.parse` accept set as the contract. It accepts non-ISO spellings
  and local-time interpretations that contradict the documented UTC type.
- Reject a second error-carrier construction in a package helper. The canonical constructor produces
  the same output, and a package-local code outside `BuiltinErrorCode` cannot be kept stable.
- Reject operator support for the opaque types. Operators cannot be overloaded, so any arithmetic is
  a named function over handles.
- Reject exposing the handle representation. The millisecond `number` is an implementation fact; a
  source-visible unit or field would fix it permanently.
- Reject a Wasm lowering set until the semantic contract exists. Duplicating the host parser in a
  second backend would double the unspecified behaviour.

## Validation and integration coverage

Primary owners today:

| Contract | Owner |
|---|---|
| Whole-surface acceptance and HTML-Wasm rejection of a reachable call | `tests/cases/core_time_functions` |
| Duration conversion values and construction lowering shape | `tests/cases/core_time_duration_conversions_success` |
| The same duration semantics through free-function call shape | `tests/cases/core_time_duration_free_functions_success` |
| Monotonic lane, observed as non-negative deltas | `tests/cases/core_time_mark_delta_success` |
| Timestamp round-trips and ISO rendering | `tests/cases/core_time_timestamp_conversions_success` |
| Valid parse with `!` propagation and `catch` | `tests/cases/core_time_timestamp_parse_success` |
| Invalid parse reaching `catch` | `tests/cases/core_time_timestamp_parse_invalid_catch_success` |
| Namespace clause, alias selection | `tests/cases/core_time_namespace_binding_success`, `tests/cases/core_time_direct_selection_alias_success` |
| Arity, wrong-type, removed-name, opaque-field and receiver-call rejection | `tests/cases/core_time_arity_error`, `core_time_wrong_argument_type_rejected`, `core_time_old_api_rejected`, `core_time_opaque_field_access_rejected`, `core_time_raw_method_call_rejected`, `core_time_namespace_raw_method_call_rejected` |
| Unreachable JS-only call in an HTML-Wasm build | `tests/cases/core_time_unused_wasm_wrapper_success` |
| Opaque types as map keys, receiver targets, trait targets, template heads | the general language cases that use Time as their subject |

No case observes the parse error's code or message, so defect 1 is unowned by construction. The
suite is strong on numeric values and rejection codes and weak wherever the canonical reference is
silent.

## Documentation corrections awaiting approval

Identified while auditing, not applied, because Time implementation status did not change:

- the progress matrix Time row records `JS / HTML` although the lane runs html and html_wasm
  executions, where the sibling Math row uses `JS / HTML-Wasm validation`
- the same row describes "runtime smoke" coverage and implies the fallible parse contract is covered,
  while the error value is never observed
- its closing rule - that Time semantics must stay backend-neutral rather than inheriting JavaScript
  behaviour - is correct and currently unenforceable, because the parse grammar it would need is
  undefined

## History

### V0 - JS-backed handle slice

The package shipped three opaque handle types and seventeen JS-only functions: a monotonic mark lane,
a wall-clock timestamp lane with one fallible ISO parse, duration construction and helpers, and
timestamp extraction and formatting. Numeric failure of the four `Float`-returning accessors was
delegated to the shared external `Float` boundary, and reachable Time calls in HTML-Wasm were
rejected through target validation.
