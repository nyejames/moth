# Core math package plan

## Role and authority

`@core/math` is the portable Core package for scalar `Float` mathematics: trigonometry, logarithms,
exponentials, roots, rounding and bounded selection.

Canonical source semantics belong to `docs/src/docs/packages/core/math/` and the canonical `Float`,
`Error` and dependency references. This living plan records implementation strategy, sequencing,
coverage ownership, open semantic questions and blockers. Implementation code never becomes the
semantic authority, and this plan cannot accept public API.

The package keeps Core origin with ExternalBinding backing and no prelude alias, so every use needs
an explicit dependency clause.

### Current-state capsule

```text
STATUS: activated early for existing-ABI work; current surface hardened, no v1 expansion accepted
CURRENT_SLICE: existing-behaviour coverage for all 18 registered functions plus registration cleanup
BLOCKERS: the proposed function expansion waits for a user decision on scope and on the unspecified numerical semantics below; compile-time folding waits for the Core const-eval prerequisite
NEXT_ACTION: settle the expansion list and the open semantic questions with the user, then publish accepted contracts in the canonical reference before implementing
```

Math was activated ahead of `@core/random`, `@core/time` and the queued `@core/text` v1 slice because
its existing surface needs no new compiler capability: every function is already registered, lowers
through the existing inline JS path and is validated by the shared `Float` boundary. The umbrella
tracker records that reason.

## Current surface

Eighteen functions and three constants are registered, all on the existing external ABI. The
canonical reference owns the signature table; the implementation-relevant facts are:

- `sin`, `cos`, `tan`, `log`, `log2`, `log10`, `exp`, `sqrt`, `abs`, `floor`, `ceil`, `round`,
  `trunc` are unary; `atan2(y, x)`, `pow(base, exponent)`, `min(a, b)`, `max(a, b)` are binary;
  `clamp(x, min, max)` is ternary.
- Every parameter and result is ABI `F64`, Moth `Float`, positional-only, shared access, fresh
  result. No function declares an error channel.
- `PI`, `TAU` and `E` are compile-time `Float` constants that fold into expressions and produce no
  runtime lookup.
- Each function lowers to one `ExternalJsLowering::InlineExpression` over the host `Math` object;
  `clamp` lowers to `Math.min(Math.max(x, min), max)`. No Math runtime helper module exists.
- No function has a Wasm lowering, so HTML-Wasm rejects any reachable Math call before lowering.

## Implementation notes

### Registration shape

`src/builder_surface/core_packages/math.rs` owns one package-local table plus a registration loop.
It is the right shape while every function has the same homogeneous signature. Replace it only when
signatures actually diverge - an error channel, a non-Float parameter or const-eval metadata - and
then with the smallest package-local spec, not a cross-Core registration framework.

### Finite-result boundary

Math does not own numeric failure. A non-finite external `Float` result is rejected by the shared
boundary: `__moth_float_validate` in `src/backends/js/runtime/numeric.rs` returns builtin error code
304, and `src/compiler_frontend/hir/hir_expression/numeric.rs` selects the enclosing function's
failure mode. A builtin `Error!` enclosing function recovers through that channel; an infallible
function, a custom fallible channel and entry-selected root code trap.

Never add a Math-specific finite guard, and never add `Error!` to a Math signature: that would change
the accepted contract from "infallible signature with a shared boundary" to "fallible package".

### Target support

`src/backends/external_package_validation.rs` rejects the first reachable external call without a
lowering for the selected target, with `MOTH-RULE-0058`. Math relies on that path; an unreachable
Math call in a Wasm build stays legal. A Wasm lowering is future work and needs its own accepted
numerical contract, not a transliteration of the JS expressions.

### Constant evaluation boundary

Math folding waits for the shared Core const-eval prerequisite **and** for an accepted compile-time
contract covering precision and numeric failure. An eligible scalar signature alone does not settle
that contract. Add no Math-local interpreter, callback registry, package-name dispatch or
placeholder evaluator metadata.

## Design rationale worth preserving

- `NaN`, positive infinity and negative infinity are never valid Moth `Float` values, so the package
  can keep infallible signatures without hiding invalid results.
- The host `Math` object is not an API checklist. Each addition needs a common use and an accepted
  contract.
- Inline lowerings keep trivial scalar calls free of runtime helper bodies and of reachability
  bookkeeping.
- Semantics the canonical reference does not state are open questions, not "whatever the current
  engine does". Where coverage cannot avoid depending on one, record it in the pinned list below
  instead of treating the assertion as settled contract.

## Current work

### Phase A - inventory before adding functions (delivered)

Every registered function and constant was traced through registration, the canonical reference, the
shared `Float` boundary and its actual test owners. Findings that shaped this plan:

- runtime coverage existed only for `sin`, `cos`, `exp`, `pow`, `sqrt` and `clamp`; twelve functions
  had none, and the constants had emitted-literal coverage only;
- rejection coverage existed only for a wrong-type `sin` call and a missing `pow` argument;
- the non-finite lane and the HTML-Wasm rejection each had one clear owner already;
- registration allocated each lowering string and parameter vector, then cloned both per entry, and
  passed parameter names into a closure that discarded them;
- no confirmed implementation defect was found.

### Phase B - existing-behaviour coverage (delivered)

Coverage now exercises all 18 functions: exactly representable results use exact rendered output,
transcendental results use a justified error bound computed in Moth, and the non-finite lane owns
logarithm-of-zero and logarithm-of-negative alongside the existing negative `sqrt` and overflowing
`exp`. Where the canonical reference is silent, the assertion is recorded in the pinned list below
rather than claimed as contract. Constant coverage is unchanged and remains emission-only.

### Phase C - registration cleanup (delivered)

Registry-owned values are constructed once, the discarded parameter-name plumbing is gone, and
registration order, names, arity, ABI types, access kinds, return metadata and lowering strings are
unchanged.

### Phase D - accepted scalar expansion (blocked on a user decision)

Implement only after the user accepts a bounded function list and the open semantic questions below,
and only after the accepted contract is published in the canonical reference. Reuse the current
scalar external ABI, return metadata and shared finite boundary; use inline lowerings where they
preserve the full contract; keep `wasm: None` while no lowering exists; exercise every new function
end to end by group closeout.

## Known gaps and next extensions

### Unspecified semantics that need a decision

The canonical reference is silent on all of these, so none of them is an accepted contract.

Unasserted, because a test written today would make current host behaviour authoritative:

- tie-breaking for `round`
- whether negative zero is preserved, normalised or unobservable
- `atan2` quadrant and signed-zero mapping
- underflow and subnormal behaviour for `exp`, `pow` and the logarithms
- whether `clamp` requires `min <= max`, and what reversed bounds mean

Already pinned by delivered coverage, so a decision here must either ratify the assertion or change
it deliberately:

- angle units for `sin`, `cos`, `tan` and `atan2`, and whether `PI`/`TAU` establish that unit. The
  error bounds in `core_math_basic_functions` are satisfiable only under radians. They rest on the
  canonical Backend-behaviour statement that HTML-JS lowers to a host `Math` expression, not on an
  accepted target-independent angle contract. A Wasm lowering or a non-radian decision changes them.
- exact endpoint behaviour for the logarithms: `core_math_float_boundary_validation` fixes
  logarithm-of-zero and logarithm-of-negative as non-finite results recovered as builtin code 304.
- precision: the transcendental checks fix a one-micro-unit absolute error bound; `log2(8)`,
  `log10(100)` and `pow(2, 3)` are asserted exactly; and `core_math_float_boundary_validation` fixes
  `exp(1.0)` to the exact decimal `2.718281828459045`. The host specification allows
  implementation-approximated results for all of those.

### Proposed expansion candidates

Candidates only. They are not accepted API and must not be implemented or documented as public
behaviour before the user accepts them.

| Group | Candidates | Decisions the group needs |
|---|---|---|
| First | `asin`, `acos`, `atan`, `cbrt`, two-argument `hypot`, `expm1`, `log1p` | domains, angle units, finite-result failure, signed zero, numerical test bounds |
| Later | `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh` | common-use justification, endpoint behaviour, overflow coverage |
| Constants | `SQRT_2`, `LN_2`, `LN_10` | deliberate acceptance and precision expectation |

`hypot` stays fixed-arity in any accepted proposal. A host variadic form is not permission to add a
variadic external signature.

### Other extensions

- A Wasm lowering set, with an accepted numerical contract rather than a JS transliteration.
- Compile-time folding once the Core const-eval prerequisite and a precision contract exist.
- Interaction with the queued fixed-scale `Number`/`NumberN` family. A Float-only Math slice neither
  implements that plan nor inherits its rounding rule.

## Longer-term candidates

- unit conversions such as degrees and radians helpers, once an angle-unit contract exists
- statistics over collection-valued inputs, after collection-valued binding signatures land
- vector and matrix surfaces, which are a separate package design rather than Math growth
- integer-oriented math, which belongs with the numeric plan's integer contracts

## Previous blockers and rejected approaches

- Reject adding `Error!` to Math signatures: the shared `Float` boundary already owns numeric
  failure, and a fallible signature would change the accepted contract.
- Reject a Math-specific finite-result guard duplicating `__moth_float_validate`.
- Reject a Math runtime helper module, registration macro DSL or unary/binary dispatch hierarchy for
  trivial inline scalar calls.
- Reject golden or exact-decimal assertions for unspecified semantics, which would silently promote
  host behaviour to contract.
- Reject variadic external signatures to mirror host variadic functions.

## Validation and integration coverage

Primary owners:

| Contract | Owner |
|---|---|
| Runtime behaviour of the 18 registered functions | `tests/cases/core_math_basic_functions` |
| Emitted values of `PI`, `TAU` and `E` | `tests/cases/core_math_constants`, which asserts the emitted decimals only and runs no output |
| Inline lowering shape | `tests/cases/core_inline_expression_lowering`, plus the `sqrt` and `exp` forms in `tests/cases/core_math_float_boundary_validation` |
| Constant folding in a const context | `tests/cases/core_math_external_constants_const_context` |
| Non-finite results and builtin-error recovery, plus HTML-Wasm rejection | `tests/cases/core_math_float_boundary_validation` |
| Type and arity rejection | `tests/cases/core_math_type_errors`, `tests/cases/core_math_arity_error` |
| Dependency, alias, namespace and facade selection | the existing dependency and namespace-binding cases |

Rules for this package: expected values come from the canonical contract wherever it speaks;
transcendental results use a justified error bound with a deterministic pass or fail; exactly
representable results use exact rendered output. Where an assertion necessarily depends on
semantics the canonical reference leaves open, record it in the pinned list above rather than
treating it as settled contract.

## History

### V0 - scalar Float surface on the JS path

The package shipped 18 inline-lowered `Float` functions and three compile-time constants for
HTML-JS, with numeric failure delegated to the shared external `Float` boundary and HTML-Wasm use
rejected through target validation.
