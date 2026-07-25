# Number and numeric semantics implementation plan

## Purpose

Implement the unified `Number` and `NumberN` high-precision numeric family through frontend, AST const folding, HIR, backend validation and HTML-JS lowering. Scaffold `Byte` in frontend and HIR with backend and runtime support deliberately deferred. Add the first conservative numeric check-elision pass.

## Current-state capsule

```text
ACTIVE_PLAN: docs/roadmap/plans/number_type_numeric_plan.md
STATUS: queued
CURRENT_SLICE: Phase 0 - refresh current numeric, TIR, HIR, target-validation and JS owners
LAST_GOOD_COMMIT: none until the first implementation slice is accepted
POST_TIR_REVIEW_COMMIT: 1298da468
BRANCH: main
IMPLEMENTATION_SCOPE: frontend types, AST folding, HIR numeric ops, JS lowering, backend validation
```

## Hard prerequisites

- final TIR one-store/exact-view architecture and post-TIR roadmap review accepted at `1298da468`
- canonical module artefacts so per-function link facts and target validation roots exist

## Required authority documents

- `docs/compiler-design-overview.md` for numeric ownership, HIR numeric domain, target validation and per-function link facts
- `docs/build-system-design.md` for build-owned target assignment and root selection
- `docs/language-overview.md` for source syntax
- `docs/src/docs/codebase/style-guide/style-guide.mtf`, `testing.mtf` and `validation.mtf`
- `docs/src/docs/progress/#page.moth` for current support

## Required architecture alignment

The plan must consume these accepted architecture contracts. Each references where the full contract lives.

- one-store exact-view TIR (see `docs/compiler-design-overview.md` "Stage 4: AST semantics" and "Templates and TIR")
- common value-to-string semantics consumed by templates and runtime lowering (see `docs/compiler-design-overview.md` "Numeric ownership")
- `Number` rounding after every language-level numeric operation result (see `docs/compiler-design-overview.md` "Numeric ownership")
- HIR numeric domain, operator and failure mode rather than backend helper names or one duplicated statement family per target (see `docs/compiler-design-overview.md` "Numeric ownership")
- generated concrete functions in sidecars, not by mutating base modules (see `docs/compiler-design-overview.md` "Generated concrete functions")
- per-function link facts as the compiler's linking authority (see `docs/compiler-design-overview.md` "Per-function link facts")
- selected target assignments from build-owned entry or package link planning (see `docs/build-system-design.md` "Target-validation roots")
- compiler-owned target validation over explicit roots and assignments (see `docs/compiler-design-overview.md` "Target-contract validation")
- build-owned root selection, command policy and partition strategy (see `docs/build-system-design.md` "Target-validation roots")
- interface and implementation fingerprint distinctions (see `docs/compiler-design-overview.md` "Fingerprints and reuse facts")
- JS-local numeric optimisation facts until another target needs a shared owner (see `docs/compiler-design-overview.md` "Numeric ownership")

## Plan-local design to retain

Retain accepted Number decisions:

- one `Number` and `NumberN` fixed-scale family
- exact scale identity
- explicit scale conversions
- exact `Int` interaction
- half-even rounding where the accepted Number design requires rounding
- no hidden literal rounding
- exact cast policy
- canonical string formatting
- `Byte` scaffold
- clean target rejection until lowerings exist

Present type and operator rules as compact lists rather than tables.

## Type family

- `Number` is fixed scale 0, arbitrary-precision integer
- `Number0` is an accepted alias of scale 0
- `Number1` through `Number256` are fixed-scale arbitrary-precision decimals
- `Number01` is an invalid leading-zero scale
- `Number257` and above are invalid over `MAX_NUMBER_SCALE`
- `Byte` is unsigned 8-bit integer, `0..255`
- `MAX_NUMBER_SCALE` is 256 for the current source surface

## Runtime value model

`NumberN` represents:

```text
semantic value = scaled_integer / 10^N
```

Compiler-side value:

```rust
pub(crate) struct NumberValue {
    pub(crate) scaled_integer: BigInt,
    pub(crate) scale: NumberScale,
}
```

## Operators

- `+` exact for all scales
- `-` exact for all scales
- `*` exact internal result, then round to scale `N`
- `/` invalid for scale 0, use `//`, `%` or cast to positive-scale `NumberN`
- `/` checked divide-by-zero and round to scale `N` for positive scales
- `//` integer division truncating toward zero for scale 0
- `//` invalid initially for positive scales
- `%` integer remainder for scale 0
- `%` checked decimal remainder for positive scales, result `NumberN`
- unary `-` exact for all scales
- `^ Int` non-negative exponent only, round to scale `N` where needed

Rounding mode is round half to even. Rounding and canonicalisation happen at every source and HIR numeric operation result boundary.

## Mixed operands

- `NumberN op NumberN` valid when scales match
- `NumberN op Int` valid with exact implicit scaling of `Int`
- `NumberN op NumberM` with `N != M` invalid, explicit scale cast required
- `NumberN op Float` invalid, deferred lossy helper required
- `NumberN ^ Int` valid
- `NumberN ^ NumberN` invalid

## Literals

- whole-number literal to `Number` or `NumberN` is contextual, exact and infallible
- decimal literal to `NumberN` is contextual and exact only
- `Number2 = 1.239` is a diagnostic with no hidden literal rounding
- decimal or exponent literal to `Number` is valid only when exactly integral after grammar materialisation

## Casts

- `Int -> NumberN` infallible exact cast, multiply by `10^N`
- `NumberN -> Int` fallible, discarded scale digits must be zero and value must fit `i32`
- `NumberN -> NumberM` with `M > N` infallible scale widening
- `NumberN -> NumberM` with `M < N` fallible exact scale narrowing, no rounding
- `NumberN -> String` infallible canonical decimal formatting
- `String -> NumberN` fallible exact parse using Moth numeric text grammar
- `Float -> NumberN` deferred named helper or method
- `NumberN -> Float` deferred named helper or method

`cast` is exact conversion. Rounding to narrower scale is a future named helper, not a cast.

## Formatting

Default `NumberN -> String` and template interpolation use canonical decimal formatting:

- trim redundant fractional trailing zeroes
- never emit exponent notation
- never emit negative zero
- emit a leading `0` before fractional values
- fixed-width display such as `1.20` is deferred to a named helper

## Byte scaffold

- `Byte` type and contextual literal validation are in scope
- `Byte` arithmetic is not in scope
- runtime backend representation, casts, formatting, collections, storage specialisation, ABI and binary buffers are deferred
- reachable runtime `Byte` values must be rejected before JS or Wasm lowering

## Non-goals

- no Wasm Number runtime or lowering
- no `Byte` runtime support
- no lossy `Float <-> NumberN` cast policy
- no fixed-width formatting or non-default rounding helpers
- no broader numeric library APIs
- no user-defined numeric traits or operator overloading

## Risks and blockers

- dynamic Number casts can make the existing static cast table noisy if implemented as scale-pair enumeration
- runtime `Byte` can leak into JS or Wasm lowerers unless backend validation is added before any Byte HIR value reaches lowering
- numeric analysis should not be added to the compiled module handoff until more than one backend consumes it

## Implementation phases

Each phase must leave one coherent path. Reference `docs/compiler-design-overview.md` "Numeric ownership" for the layer-by-layer contract.

### Phase 1: Refresh current numeric, TIR, HIR, target-validation and JS owners

Context: refresh all code anchors before implementation. Final TIR owners are fixed by the
`1298da468` review; consume them without reopening template architecture.

- Confirm the `1298da468` post-TIR review and canonical module artefacts are accepted.
- Record `git rev-parse HEAD`, branch and `git status --short`.
- Refresh current `numeric_text`, `TypeEnvironment`, HIR numeric, JS lowering and backend validation owners.
- Run baseline `just validate` and record results.

### Phase 2: Add canonical Number type identity and exact compiler values

Context: `NumberN` must be a canonical type, not a tokenizer feature. Establish semantic identity first.

See `docs/compiler-design-overview.md` "Type identity" for the `TypeEnvironment` contract.

- Add `NumberScale(pub u16)` and `MAX_NUMBER_SCALE = 256`.
- Replace inactive `Decimal` builtin key with `Number` at the same slot.
- Add `BuiltinTypeConstructor::Number { scale: NumberScale }` for positive scales.
- Canonicalise `NumberScale(0)` to `builtins.number`.
- Render scale 0 as `Number`, positive scales as `NumberN`.
- Resolve `Number`, `Number0`, `Number1` through `Number256`. Reject leading-zero suffixes and scales over `MAX_NUMBER_SCALE`.
- Resolve `Byte` as a new builtin.
- Reserve `Number`, `Number[0-9]+` and `Byte` in the type namespace.
- Add `src/compiler_frontend/numeric_values/` with `NumberValue` (signed `BigInt` scaled integer and `NumberScale`), exact parsing, canonical formatting, round-half-to-even, scale conversion and arithmetic.
- Remove inactive `Decimal` comments, aliases and compatibility paths.

### Phase 3: Add literals, folding, casts and common formatting

Context: AST can now represent Number and Byte values and materialise contextual literals. Template formatting must be shared.

See `docs/compiler-design-overview.md` "Numeric ownership" for the value-to-string contract.

- Add AST expression variants for `Number` and `Byte` values.
- Add contextual literal materialisation: whole numbers to `Number` or `NumberN` (exact), decimals to `NumberN` (exact only), `Byte` `0..255` only.
- Add one shared value-to-string formatter used by `fold_prepared_template`, const/cast folding and runtime lowering; do not add another TIR fold or template-formatting path.
- Do not add Number-specific TIR node kinds.
- Add scale-aware builtin cast policy with a single cast policy context struct (not scale-pair enumeration).
- Implement const-foldable Number casts: `Int -> NumberN`, `NumberN -> Int`, `NumberN -> NumberM` widening and narrowing, `NumberN -> String`, `String -> NumberN`.
- Reject `Float <-> NumberN` casts with diagnostics pointing to future lossy helpers.
- Add AST Number operator type checking: same-scale, exact `Int` participation, cross-scale diagnostics, Float diagnostics, scale-zero `/` diagnostic, positive-scale `//` diagnostic, `NumberN ^ Int` only.
- Add constant folding through `numeric_values` with per-operation rounding.
- Add equality and comparison: same-scale valid, `Number` and `Int` valid through exact scaling, cross-scale invalid, `Number` and `Float` invalid.

### Phase 4: Refactor HIR numeric operations to domain, operator and failure mode

Context: refactor HIR numeric shape before adding Number runtime lowering. Preserve existing Int and Float behaviour.

See `docs/compiler-design-overview.md` "Numeric ownership" for the HIR contract.

- Replace old `HirNumericOp` enum variants with `HirNumericDomain` (Int, Float, Number with scale), `HirNumericOperator` (Add, Subtract, Multiply, Divide, IntDivide, Modulo, Power, Negate) and `HirNumericOp` struct.
- Leave `Byte` out of arithmetic domains.
- Update all Int and Float call sites: AST and HIR classification, range-loop counters, HIR display, validation, reachability, JS helper mapping and tests.
- Preserve `NumericFailureMode` unchanged.
- Search and remove stale old variant names (`IntAdd`, `FloatAdd`, etc).

### Phase 5: Add per-function numeric link facts and target validation

Context: keep backend lowerers from seeing unsupported target features. Use per-function link facts and explicit validation roots.

See `docs/compiler-design-overview.md` "Per-function link facts" and "Target-contract validation".

- Record numeric operations in `ModuleLinkFacts` per function.
- Add a reusable backend validation helper that scans reachable blocks and expressions with a `TypeEnvironment` predicate.
- Wasm rejects reachable Number literals, ops, casts and formatting.
- JS rejects reachable runtime `Byte` values.
- Wasm rejects reachable runtime `Byte` values.
- Unreachable private Number or Byte helpers do not fail builds for selected roots.
- Use `CompilerDiagnostic::unsupported_backend_feature` or a more precise typed diagnostic.

### Phase 6: Add JavaScript runtime representation and helpers

Context: wire Number HIR and JS runtime together as one vertical implementation slice.

- Add JS Number runtime helpers: constructor, brand and scale validation, literal construction, canonical formatter, scale widening and narrowing, Int-to-Number and Number-to-Int, String-to-Number parser, arithmetic helpers and half-even rounding.
- Use the existing carrier contract for checked failures (`ReturnError` and `Trap` modes).
- Update JS expression lowering for Number literals.
- Update JS statement lowering for Number `NumericOp`.
- Update JS cast lowering for scale-aware Number policies.
- Update JS value-to-string and template interpolation to recognise branded Number values.
- Update JS clone, copy and equality helpers (Number is immutable).

### Phase 7: Add focused JS check-elision side tables

Context: add optimisation facts without changing HIR or user diagnostics. The current source surface has one consumer: JS lowering.

See `docs/compiler-design-overview.md` "Numeric ownership" for the side-table contract.

- Add `src/compiler_frontend/analysis/numeric/` with range facts and transfer.
- Invoke analysis inside JS lowering before emission, not in the compiled module handoff.
- JS lowering elides checks only when: `NumericFailureMode::Trap`, domain is `Int` and fact is `ProvenSafe`.
- Do not elide checks for `ReturnError`, Float, Number rounding, Number divide-by-zero or Byte.
- Do not include user-facing `ProvenFailure` diagnostics in the current source surface.
- Keep numeric analysis facts as side tables that do not mutate HIR.

### Phase 8: Add generated-function and cross-module coverage

Context: generated concrete functions may use Number operations and need the same validation and lowering.

See `docs/compiler-design-overview.md` "Generated concrete functions" for the sidecar contract.

- Ensure generated function sidecars carry numeric link facts.
- Ensure target validation includes reachable generated functions.
- Ensure JS lowering handles Number in generated sidecars.
- Add cross-module generic coverage tests with Number.

### Phase 9: Migrate docs, tests and progress rows

Context: documentation, tests and implementation status must agree.

- Update `docs/language-overview.md` with Number type family, operators, casts and formatting.
- Update `docs/compiler-design-overview.md` for `numeric_values` owner, HIR numeric domain and numeric analysis side-table ownership.
- Update progress matrix rows for `Number`, `NumberN`, numeric check elision, `Byte` scaffold, Number follow-ups and Wasm deferral.
- Add required integration cases: Number literals and diagnostics, arithmetic, mixed Int and Number, cross-scale and Float diagnostics, casts, Error recovery vs trap, Wasm rejection, Byte scaffold, Int check-elision.
- Rebuild generated documentation through the compiler.

### Phase 10: Delete legacy Number and duplicate formatting paths

Context: the refactor is not complete while old Decimal naming or duplicate template formatting paths remain.

- Delete inactive `Decimal` naming, comments and compatibility paths.
- Delete old `HirNumericOp` enum variants.
- Delete any duplicate value-to-string formatting found by the current ownership review; final TIR has no legacy template path.
- Route allocation, formatter-output caching and backend string-building performance ideas to `post-tir-template-parser-optimization-plan.md` rather than implementing them as Number compatibility work.
- Delete whole-module backend gate assumptions for numeric types (use per-function link facts and explicit validation roots).
- Search and confirm no stale `Decimal`, `BigInt`, `IntAdd`, `FloatAdd` or duplicate template-formatting references remain.

## Old owners and paths to remove

- inactive `Decimal` naming, comments and compatibility paths
- old `HirNumericOp` enum variants such as `IntAdd` and `FloatAdd`
- duplicate value-to-string formatting outside the shared current owner
- whole-module backend gate assumptions for numeric types

## Required tests

Cover:

- Number type resolution, display and scale bounds
- contextual Number and Byte literals
- const Number arithmetic with per-operation rounding
- runtime Number arithmetic on HTML-JS
- mixed Int and Number arithmetic
- cross-scale and Float diagnostics
- Number casts including exact narrowing success and failure
- String to Number parse success and failure
- canonical formatting and template interpolation
- Number Wasm rejection with structured diagnostics
- Byte scaffold diagnostics and backend rejection
- HIR numeric domain and operator shape
- JS check-elision for proven-safe trap-mode Int only

## Documentation and progress-matrix impact

- update `docs/language-overview.md` with Number type family, operators, casts and formatting
- update `docs/compiler-design-overview.md` for `numeric_values` owner, HIR numeric domain and numeric analysis side-table ownership
- progress matrix rows: `Number` and `NumberN`, numeric check elision, `Byte` scaffold, Number follow-ups, Wasm deferral

## Validation requirements

Each code-bearing phase runs:

```bash
cargo fmt
just validate
```

Run the documentation release build when source docs change.

## Final architecture audit

Before marking this plan complete, verify:

- `Number` and `NumberN` resolve, fold, lower and run on HTML-JS
- `Number` rounding happens after every language-level numeric operation result
- HIR numeric ops use domain, operator and failure mode
- per-function link facts carry numeric operations
- target validation rejects unsupported reachable Number on Wasm
- JS check elision is side-table only and does not mutate HIR
- `Byte` is scaffold-only with clean backend rejection
- no `Decimal` compatibility or duplicate template-formatting path remains

## Deliberately deferred work

### Wasm

- Number runtime representation and helpers
- Number casts and formatting
- Byte runtime representation
- Wasm use of numeric analysis facts

### Number helpers

- lossy `Float -> NumberN` and `NumberN -> Float` helpers
- fixed-width formatting
- non-default rounding helpers
- scale-narrowing with rounding
- broader numeric library APIs

### Optimisation

- `ReturnError` CFG simplification for proven-safe numeric operations
- Float finite-proof check elision
- Number rounding and check fusion where equivalent
- broader range analysis over loops, relational patterns and collection facts
- numeric facts shared with Wasm

### Byte

- JS and Wasm runtime representation
- collections, storage specialisation and ABI
- binary buffer integration
- formatting, parsing and arithmetic policy
