# Core time package plan

## Role and authority

`@core/time` separates elapsed time from wall-clock time. It exposes three opaque handle types -
`Duration`, `TimeMark`, `Timestamp` - and free functions over them, with Core origin, ExternalBinding
backing and no prelude alias.

Canonical source semantics belong to `docs/src/docs/packages/core/time/time.mtf` and the canonical
`Float`, `Error` and external-binding references. That reference now states the package's semantic
contract, so this living plan records implementation strategy, the inherited defects the slice
exposed but does not own, and coverage ownership. It cannot accept public API or semantics.

### Current-state capsule

```text
STATUS: semantic contract published and implemented; corrections and the accepted arithmetic surface delivered
CURRENT_SLICE: none - the contract, the four corrections and the five arithmetic functions are complete
BLOCKERS: a Wasm lowering set still waits for a target decision; the repeated same-name `catch` binder defect below is inherited and owned outside this package; the umbrella programme owns the inherited red `just validate` gate
NEXT_ACTION: none required; a Wasm lowering set is the next candidate and needs its own accepted decision
```

## Current surface

Registered in `src/builder_surface/core_packages/time.rs`: three opaque types, twenty-two functions,
no constants. Every function has a JS lowering and `wasm: None`; twenty are inline expressions and
two are runtime helpers.

| Group | Functions | Lowering |
|---|---|---|
| Monotonic | `mark_now`, `elapsed_since`, `duration_between` | `globalThis.performance.now()`, `(globalThis.performance.now() - #0)`, `(#1 - #0)` |
| Wall clock | `timestamp_now`, `timestamp_from_unix_seconds`, `timestamp_from_unix_milliseconds`, `timestamp_from_iso_string` | `Date.now()`, `(#0 * 1000.0)`, `#0`, runtime helper |
| Duration construction | `duration_from_seconds`, `duration_from_milliseconds` | `(#0 * 1000.0)`, `#0` |
| Duration helpers | `as_seconds`, `as_milliseconds`, `is_negative`, `abs`, `clamp`, `duration_add`, `duration_subtract`, `duration_scale` | `(#0 / 1000.0)`, `#0`, `(#0 < 0)`, `Math.abs(#0)`, `Math.min(Math.max(#0, #1), #2)`, `(#0 + #1)`, `(#0 - #1)`, `(#0 * #1)` |
| Timestamp helpers | `unix_seconds`, `unix_milliseconds`, `to_iso_string`, `timestamp_offset`, `timestamp_difference` | `(#0 / 1000.0)`, `#0`, runtime helper, `(#0 + #1)`, `(#1 - #0)` |

Uniform contract facts, all read from registration: every parameter is positional-only with shared
access, every success slot is fresh, and `timestamp_from_iso_string` and `to_iso_string` are the two
functions with an error channel.

## Implementation notes

### Opaque handles are compile-time only

All three types register `ExternalAbiType::Handle`, which maps to no Moth scalar type
(`src/compiler_frontend/external_packages/abi.rs`) and is interned as a distinct nominal external
identity per type. In emitted JavaScript a `Duration`, `TimeMark` or `Timestamp` is a bare `number`:
nothing wraps, brands or tags it, unlike `@core/io`'s `Input`, which is a real object. The abstraction
exists in the type environment and in diagnostics, and is erased at runtime. That erasure is why
every arithmetic function can be a one-expression inline lowering over the millisecond value.

### The two runtime helpers

`src/backends/js/package_bindings/core/time.rs` builds `__moth_time_timestamp_from_iso_string` and
`__moth_time_to_iso_string`. Both apply compiler-owned failure lanes: the parse helper applies the
canonical String content boundary `__moth_string_value`, matches the published grammar with one
anchored regular expression, validates the calendar and offset fields, truncates fractional digits
toward zero, converts a numeric offset and range-checks the instant; the render helper tests the
renderable window before truncating and never lets `toISOString()` throw. Failure goes through the
canonical `__moth_error_result(message, code)` with `BuiltinErrorCode::TimeInvalidTimestampText`
(310) or `TimeTimestampOutOfRange` (311), so the package never hard-codes an error code of its own:
the numbers in the helper source are interpolated from that enum alongside the window bounds.

Both bodies are built once in a `LazyLock` because they interpolate those codes and the window
bounds. The same inventory feeds emission and the first-party JavaScript guard, so the two consumers
cannot drift.

Instant construction goes through `setUTCFullYear` rather than `Date.UTC`, which remaps years 0 to 99
into the 1900s and would silently accept a four-digit year the grammar allows.

### The renderable window

The window is the four-digit year range, `0000-01-01T00:00:00.000Z` to `9999-12-31T23:59:59.999Z`,
held as two `i64` consts in the helper module. It is not the host limit: `toISOString()` accepts
±8 640 000 000 000 000 ms but switches to an expanded-year spelling such as `+275760-09-13` outside
the four-digit range, which is neither the published format nor re-parseable by this package. Both
helpers apply the same window, so every string this package renders parses back to the instant it
names. The pair is not an inverse in the other direction: parsing canonicalises the offset and the
extra fractional digits, and rendering truncates a fractional millisecond, so a rendered string
recovers its instant while a parsed instant need not reproduce its original text.

### Finite-result boundary

The shared boundary `__moth_float_validate` (builtin code 304) is applied by the single-Float success
gate in HIR, so it fires on exactly four Time functions: `as_seconds`, `as_milliseconds`,
`unix_seconds`, `unix_milliseconds`. Opaque-returning functions and every `Float` *argument* bypass
it. That is why duration arithmetic overflow is contractually observable at the read rather than at
the arithmetic: `duration_scale` can produce a host infinity, and the boundary rejects it when the
duration is read.

### No operator surface

Time values support no arithmetic and no comparison. The AST arithmetic policy accepts only
`Int`/`Float` pairs, the comparison policy rejects same-type non-comparable categories, and
`TypeEnvironment::supports_runtime_equality` returns false for external types. There is no
registration hook for an operator, so all five arithmetic functions are named functions.

## Design rationale worth preserving

- Handle erasure is why every inline lowering can do arithmetic on a handle directly, and why the
  unit each handle carries - milliseconds today - is an implementation fact rather than a stated
  contract. Exposing that unit or a field would fix it permanently.
- Operators cannot be overloaded and no registration hook exists for one, so every arithmetic
  addition is a named function over handles rather than a syntax change.
- The finite-result boundary is a shared language rule applied at one gate, not a package feature.
  Extending validity to `Float` *arguments* would be a boundary decision, not a Time helper edit.
- Binding error codes belong to the compiler-owned enum. A package-local code cannot be kept stable
  and cannot carry a `default_message`.
- Host behaviour is not a contract. The grammar, the field rules, the truncation rule and the window
  were published before the parser was written, and the parser matches them instead of delegating
  acceptance to `Date.parse`.
- The window is narrower than the host limit on purpose: a rendered string that cannot be parsed back
  would make the two fallible functions non-inverse for no user-visible benefit.

## Current work

None. The published contract, the four corrections it exposed and the accepted arithmetic surface are
all delivered and covered.

## Known gaps and next extensions

### Inherited defects this slice exposed and does not own

| # | Defect | Owner | Evidence |
|---|---|---|---|
| 1 | Two `catch` handlers in one function that use the same binder name fail with `MOTH-INFRA-0001` "Local 'err' is already declared in this function scope", an internal-invariant lane for legal source. The canonical catch reference makes the binding visible only inside the handler and the shadowing reference permits reuse across disjoint scopes, so sibling handlers should be free to share a name. The AST interns the binder against the parent statement scope before the handler child frame exists, and HIR allocates it with `allocate_named_local` without `with_temporary_local_bindings`, so the name also leaks for the rest of the function. Every multi-handler case in the tree avoids the shape by using distinct names, so nothing owns it. | `src/compiler_frontend/ast/statements/fallible_handling/catch_handler.rs` for the path identity, `src/compiler_frontend/hir/hir_expression/fallible/catch.rs` for the unscoped local | `catch_handler.rs:186-191`, `catch.rs:178-186`, `hir_builder.rs:795-803`; the scoped pattern already exists in `match_captures.rs:240-244` |
| 2 | `@core/io`'s input helper still hand-builds its failed carrier with the unowned code 500, and the canvas binding uses the same unowned 400/404/409/500 family. Time no longer does. | those package bindings | `src/backends/js/package_bindings/core/io.rs`, the canvas binding |
| 3 | The shared `Float` boundary traps outside a fallible context, so a non-finite external result ends the program rather than returning `Error!`. That is the language-wide rule, not a Time behaviour, and the contract now states it explicitly instead of promising that nothing aborts. | the numeric boundary lane in HIR and the JS runtime | `hir_expression/numeric.rs`, `src/backends/js/runtime/numeric.rs` |

### Canonical facts with no test owner

Shared access and mutable-access rejection; return freshness; the struct-construction prohibition;
a Time-specific named-argument rejection; any non-epoch instant through the inline extraction
helpers. Host-call identity for `mark_now` and `timestamp_now` is asserted only by string presence,
so a swap that leaves both `performance.now()` and `Date.now()` somewhere in the artifact still
passes.

### Next extensions, in order

Candidates only. Nothing here is accepted API.

1. **A Wasm lowering set.** Now that the grammar, the calendar rules, the truncation rule and the
   window are published rather than inherited from the host parser, a second backend can implement
   the same contract instead of duplicating unspecified behaviour. It needs a target decision first.
2. **Duration range rules.** `abs` of the most negative representable value and the saturation
   behaviour of the arithmetic functions are still unstated; the contract currently describes exact
   millisecond arithmetic and defers overflow to the read boundary.

## Longer-term candidates

Out of implementation scope: calendar types, time zones, local-time conversion, locale-aware
formatting, timers, sleep, intervals, animation scheduling and new opaque-handle infrastructure.

## Previous blockers and rejected approaches

- Reject treating the `Date.parse` accept set as the contract. It accepted non-ISO spellings and
  local-time interpretations that contradicted the documented UTC type, so the published grammar is
  matched explicitly instead.
- Reject the host renderable limit as the window. Its expanded-year spelling is not the published
  format and does not parse back.
- Reject a second error-carrier construction in a package helper. The canonical constructor produces
  the same output, and a package-local code outside `BuiltinErrorCode` cannot be kept stable.
- Reject operator support for the opaque types. Operators cannot be overloaded, so any arithmetic is
  a named function over handles.
- Reject exposing the handle representation. The millisecond `number` is an implementation fact; a
  source-visible unit or field would fix it permanently.
- Reject a cross-Core registration abstraction. Package-local tables are the settled shape, and
  Time's redundant `shared_param` and spec-mirror layer was removed rather than generalised.
- Reject containing host exceptions with a new backend guard. Making the helpers incapable of
  throwing removes the need, and a per-call guard would cost every external call.

## Validation and integration coverage

Primary owners:

| Contract | Owner |
|---|---|
| Whole-surface acceptance and HTML-Wasm rejection of a reachable call | `tests/cases/core_time_functions` |
| Duration conversion values and construction lowering shape | `tests/cases/core_time_duration_conversions_success` |
| The same duration semantics through free-function call shape | `tests/cases/core_time_duration_free_functions_success` |
| Monotonic lane, observed as non-negative deltas | `tests/cases/core_time_mark_delta_success` |
| Timestamp round-trips and ISO rendering | `tests/cases/core_time_timestamp_conversions_success` |
| Valid parse with `!` propagation and `catch` | `tests/cases/core_time_timestamp_parse_success` |
| Invalid parse reaching `catch` | `tests/cases/core_time_timestamp_parse_invalid_catch_success` |
| Grammar rejection classes, each observing code 310 | `tests/cases/core_time_timestamp_parse_grammar_rejected`, `core_time_timestamp_parse_lowercase_designator_rejected`, `core_time_timestamp_parse_whitespace_rejected`, `core_time_timestamp_parse_date_only_rejected` |
| Calendar rejection, and the time-of-day, offset-hour and offset-minute ranges rejected independently so neither offset field can hide behind the other | `tests/cases/core_time_timestamp_parse_invalid_calendar_day_rejected`, `core_time_timestamp_parse_field_out_of_range_rejected` |
| Numeric-offset conversion in both signs, and fractional truncation proved by a fourth digit that would carry into the next second under rounding | `tests/cases/core_time_timestamp_parse_offset_fraction_success` |
| The renderable window on both paths: accepted endpoints, endpoint round-trips and the post-offset rejection | `tests/cases/core_time_timestamp_parse_window_boundary` |
| Rendering rejection outside the window, including the fractional edge that proves the check precedes truncation | `tests/cases/core_time_to_iso_string_out_of_range_rejected` |
| Arithmetic values, argument order and composition | `tests/cases/core_time_duration_arithmetic_success` |
| Arithmetic overflow: the scaled duration stays an ordinary opaque value on an infallible path, then becomes code 304 where it is read as a `Float` | `tests/cases/core_time_duration_overflow_at_float_boundary` |
| Namespace clause, alias selection | `tests/cases/core_time_namespace_binding_success`, `tests/cases/core_time_direct_selection_alias_success` |
| Arity, wrong-type, removed-name, opaque-field and receiver-call rejection | `tests/cases/core_time_arity_error`, `core_time_wrong_argument_type_rejected`, `core_time_old_api_rejected`, `core_time_opaque_field_access_rejected`, `core_time_raw_method_call_rejected`, `core_time_namespace_raw_method_call_rejected` |
| Unreachable JS-only call in an HTML-Wasm build | `tests/cases/core_time_unused_wasm_wrapper_success` |
| Opaque types as map keys, receiver targets, trait targets, template heads | the general language cases that use Time as their subject |

Rules for this package: every expected value comes from the published contract, and a rejection case
observes the returned error's `code` rather than a message, so the two Time codes each have an owner.
Host formatting is asserted only where the contract fixes the format.

This slice could not close the mandatory `just validate` gate: the inherited data-layout base fails
the all-targets lint build and two feature lanes. The programme plan owns that record. Focused
evidence was `cargo run -- tests --tag time` (29/29), `--tag core-packages` (42/42), the full
`cargo run -- tests` at the inherited failure count, `cargo run -- check docs --terse`,
`cargo check -p moth --lib` and `rustfmt --check` on every touched Rust file.

The final review closed three coverage gaps and two contract overstatements. The offset fields were
only exercised through the combined `+99:99` sample, which either half of the range check could
reject alone, so the case now rejects `+24:00` and `+00:60` independently and accepts `-05:30`. The
only long fraction was `.123456789`, which truncation and rounding both resolve to `123`
milliseconds, so `.9999Z` was added: rounding would carry into the next second. The overflow case
observed nothing before the failing accessor, so it now reads the scaled duration on an infallible
path through `is_negative` first, proving the overflowed value is an ordinary opaque `Duration` and
that code 304 comes from the `Float` read. The contract's unqualified exact-arithmetic sentence was
corrected, because adding one millisecond to `1e16` milliseconds returns the same duration, and the
`clamp` reversed-bound result is now stated the way `math.mtf` states its own. Helper-definition
artifact assertions were removed from the semantic and rejection cases, leaving
`core_time_functions` as the single owner of helper emission.

## History

### V0 - JS-backed handle slice

The package shipped three opaque handle types and seventeen JS-only functions: a monotonic mark lane,
a wall-clock timestamp lane with one fallible ISO parse, duration construction and helpers, and
timestamp extraction and formatting. Numeric failure of the four `Float`-returning accessors was
delegated to the shared external `Float` boundary, and reachable Time calls in HTML-Wasm were
rejected through target validation.

### V1 - published semantics, corrected boundaries and an arithmetic surface

An audit found that the package's observable behaviour was whatever `Date.parse` and `toISOString()
` did, including local-time interpretation of designator-less text, calendar rollover, an unowned
error code, a missing String content boundary and a reachable host `RangeError` from an infallible
signature. The contract was published first, then the parser was rewritten against it, `to_iso_string`
became fallible over a validating helper, and the registration module collapsed to one descriptor
table. The same slice added the five accepted arithmetic functions and fixed an unrelated
operator-policy defect that reported unary minus on a non-numeric operand through the
internal-invariant lane.
