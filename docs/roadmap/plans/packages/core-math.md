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
STATUS: activated early for existing-ABI work; the accepted scalar expansion is delivered
CURRENT_SLICE: none - the expansion, its published numerical contract and its coverage are complete
BLOCKERS: compile-time folding waits for the Core const-eval prerequisite; the inherited data-layout base fails `just validate`, so code-bearing slices report focused evidence instead of a closed gate
NEXT_ACTION: none required; a Wasm lowering set and const-eval folding are the next candidates, each needing its own accepted contract
```

Math was activated ahead of `@core/random`, `@core/time` and the queued `@core/text` v1 slice because
its existing surface needs no new compiler capability: every function is already registered, lowers
through the existing inline JS path and is validated by the shared `Float` boundary. The umbrella
tracker records that reason.

## Current surface

Thirty-one functions and six constants are registered, all on the existing external ABI. The
canonical reference owns the signature table and the numerical contract; the
implementation-relevant facts are:

- `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `log`, `log2`, `log10`, `log1p`, `exp`, `expm1`,
  `sqrt`, `cbrt`, `abs`, `floor`, `ceil`, `round`, `trunc`, `sinh`, `cosh`, `tanh`, `asinh`,
  `acosh`, `atanh` are unary; `atan2(y, x)`, `pow(base, exponent)`, `hypot(x, y)`, `min(a, b)`,
  `max(a, b)` are binary; `clamp(x, min, max)` is ternary.
- Every parameter and result is ABI `F64`, Moth `Float`, positional-only, shared access, fresh
  result. No function declares an error channel.
- `PI`, `TAU`, `E`, `SQRT_2`, `LN_2` and `LN_10` are compile-time `Float` constants that fold into
  expressions and produce no runtime lookup.
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

### Phase D - accepted scalar expansion (delivered)

The user accepted the whole candidate list at once: `asin`, `acos`, `atan`, `cbrt`, two-argument
`hypot`, `expm1`, `log1p`, `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh`, plus the constants
`SQRT_2`, `LN_2` and `LN_10`. Each one reuses the existing scalar ABI through the same table row
shape, so registration grew by thirteen rows and three constant entries and nothing else changed:
the parameter constructor, return metadata, error channel and `wasm: None` all come from the
unchanged loop. `hypot` stayed fixed-arity.

The numerical contract the expansion needed was settled with the user first and published in the
canonical reference as `### Numerical contract` before any code landed. It states the radian angle
unit, the domain list and the shared-boundary failure lane, `round` ties toward positive infinity,
`clamp` as the `min(max(x, min), max)` composition with the reversed-bound result, the `atan2` angle
range, `hypot`'s overflow avoidance, signed zero as outside the contract, underflow staying finite
with the subnormal-or-zero choice unspecified, and the precision statement with its exact-result
examples and the nearest-`Float` constant rule.

## Known gaps and next extensions

### How coverage now rests on the contract

Every assertion that previously depended on canonical silence now rests on a published sentence, so
the earlier pinned list is retired. Two properties worth remembering when the contract next changes:

- The radian bounds in `core_math_basic_functions` and `core_math_transcendental_functions` are
  satisfiable only under radians, so a non-radian decision would invalidate them rather than merely
  loosen them.
- `core_math_float_boundary_validation` still fixes `exp(1.0)` to the exact decimal
  `2.718281828459045`. That predates the precision statement, which promises exactness only for
  results that are exactly representable, and `exp(1)` is not one of them. It is an inherited
  assertion on a target-defined approximation and should become a bounded check when that case is
  next touched.

### Coverage gaps that are not defects

- No case exercises arity or wrong-type rejection for an individual new function; `core_math_type_errors`
  and `core_math_arity_error` own those shapes generically through `sin` and `pow`.
- No case exercises `sinh` or `cosh` overflow. The overflow lane itself is owned through `exp`.
- Constant coverage stays emission-only and unordered: `core_math_constants` asserts the six emitted
  decimals in the artifact and observes no rendered output.

### Other extensions

- A Wasm lowering set, with an accepted numerical contract rather than a JS transliteration.
- Compile-time folding once the Core const-eval prerequisite and a precision contract exist.
- Interaction with the queued fixed-scale `Number`/`NumberN` family. A Float-only Math slice neither
  implements that plan nor inherits its rounding rule.
- `ExternalConstantDef::data_type` is registered but never read by production code, so a constant's
  declared ABI type is unenforced: mutating `PI`'s type survives the whole suite. Either the field
  gains a consumer in the constant path or it should go; the owner is the external-package registry,
  not this package.

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
| Runtime behaviour of the 18 originally registered functions, plus the `round` tie rule, the reversed-bound `clamp` result and the four `atan2` quadrants | `tests/cases/core_math_basic_functions` |
| Runtime behaviour of the 13 functions added by the expansion, including the overflow-avoiding `hypot` sample | `tests/cases/core_math_transcendental_functions` |
| Emitted values of the six constants | `tests/cases/core_math_constants`, which asserts the six emitted decimals in the artifact only. Its `io.line` calls are emitted but never observed, because the case declares no rendered-output expectation, so nothing about runtime output is checked either way |
| Inline lowering shape | `tests/cases/core_inline_expression_lowering`, plus the `sqrt` and `exp` forms in `tests/cases/core_math_float_boundary_validation` |
| Constant folding in a const context | `tests/cases/core_math_external_constants_const_context` |
| Non-finite results and builtin-error recovery, plus HTML-Wasm rejection | `tests/cases/core_math_float_boundary_validation` |
| Type and arity rejection | `tests/cases/core_math_type_errors`, `tests/cases/core_math_arity_error` |
| Dependency, alias, namespace and facade selection | the existing dependency and namespace-binding cases |

Rules for this package: expected values come from the canonical contract wherever it speaks;
transcendental results use a justified error bound with a deterministic pass or fail; exactly
representable results use exact rendered output. Every function needs at least one sample that no
plausible mis-registration survives, which means a fixed point such as `asinh(0)` or `cosh(0)` is
never the only sample for its function.

The expansion could not close the mandatory `just validate` gate either. After this branch merged
the published data-layout work the inherited failure is narrower: library tests pass, six of eight
feature lanes pass, and `ci-clippy-native` plus the `timers-counters` and `dev-output` lanes stay red
on data-layout files. The programme plan owns that record. Focused evidence for the expansion is
`cargo run -- tests --tag math` (14/14), `--tag core-packages` (31/31),
`cargo run -- check docs --terse`, `cargo check -p moth --lib` and `rustfmt --check` on `math.rs`.
Both slices are mutation-proved: the hardening slice proved every registered function, constant,
arity, ABI type and access kind has a killed mutant, and the expansion proved the new coverage by
rewriting the `asinh` lowering to `Math.sinh(#0)`, which failed the transcendental case on
`approximate_asinh=false` alone before the row was restored.

## History

### V0 - scalar Float surface on the JS path

The package shipped 18 inline-lowered `Float` functions and three compile-time constants for
HTML-JS, with numeric failure delegated to the shared external `Float` boundary and HTML-Wasm use
rejected through target validation.

### V1 - accepted scalar expansion with a published numerical contract

The package grew to 31 functions and six constants on the same scalar ABI, and the canonical
reference gained the numerical contract that expansion required: angle unit, per-function domains,
the rounding tie rule, the `clamp` composition, the `atan2` range, `hypot`'s overflow avoidance,
signed zero and underflow as deliberately unspecified, and the precision rule.
