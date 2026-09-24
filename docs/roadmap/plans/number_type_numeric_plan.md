# Numeric types and semantics implementation plan

## Status

- Status: active on numeric-type-expanding.
- Current slice: Phase 2, canonical types, profile and materialisation.
- Blockers: baseline `bench-scaling` generic-instantiation exponent exceeds its locked budget; code-bearing checkpoints require a genuine fix, not a budget increase.
- Next action: start Phase 2 HIR-reaching canonical semantics.

## Goal and delivery order

Replace the former Number-first numeric plan with one coherent numeric model.
Deliver explicit binary widths, the compilation-wide numeric profile and runtime
Byte first. Implement Number and NumberN afterwards in this same plan. Finish
with Error.code changing from Int to U32 and the documentation/test closeout.

The main roadmap owns serial order. This work runs immediately after MON v1 and
before the subsequent serial compiler plans. First-party package work remains
active in parallel under the roadmap's independent gates. Coordinate shared
numeric and binding owners rather than reopening or pausing unrelated packages.

This is an implementation plan for the maintainer-approved design below. Phase 1
publishes that design in the existing permanent authorities before executable
changes. Until implementation lands, current support remains the progress
matrix's responsibility. Retain this file path so existing navigation keeps
working, but replace the previous plan completely rather than layering onto it.

## Prerequisites and authorities

Required delivered capabilities are canonical module interfaces and generated
sidecars, explicit numeric failure paths, source-owned lossless numeric tokens,
the shared source argument owner and the public Rust-only MON codec.

Read AGENTS.md, the full implementation style guide and the relevant testing and
validation routes. Read both compiler and build authorities in full at activation.

- `docs/compiler-design-overview.md`: type identity, AST receiving boundaries,
  numeric ownership, HIR, public interfaces, generated functions, target checks
  and the Rust-only MON service.
- `docs/build-system-design.md`: bootstrap, package boundaries, target partition,
  Moth-native versus foreign linking, physical variants and compatibility.
- `docs/compiler-data-layout-design.md`: retained numeric text, source ownership,
  folded values, diagnostics and the MON data handoff.
- `docs/src/developer-docs/language/overview.mtf`: route to the canonical numeric,
  cast, loop, collection, map, Error, config and package references.
- `docs/src/docs/mon/mon-format.mtf`: the delivered literal-data contract.
- `docs/src/developer-docs/memory-management/overview.mtf`: route to physical
  layout and backend handoff contracts when those consumers are touched.
- `docs/src/docs/progress/@page.moth` and its packages-and-builders companion:
  implementation and runtime coverage, not permission to infer final semantics.

Name prerequisite capabilities in other plans. Shared durable contracts belong
in these authorities, not in links between short-lived implementation plans.

## Accepted numeric contract

### Type identity and profile

The complete source surface is:

```text
Int, Float
I8, I16, I32, I64
U8, U16, U32, U64
F16, F32, F64
Number, Number0, Number1 through Number256
Byte
```

Int and Float retain distinct canonical semantic identities. They are not aliases
for an explicit-width type, even when their physical precision matches it.
Number0 is the accepted spelling alias for Number's scale-zero identity. Reject
leading-zero scales such as Number01 and scales above 256. Byte is nominally
distinct from U8, I8 and Char and is outside numeric arithmetic.

One compiler-owned NumericProfile selects Int width and Float precision
independently from 32 or 64 bits. Default: Int32 and Float64. All four combinations
are supported and tested. The builder supplies the profile before command-input
materialisation and config compilation. Lowerers consume it rather than choosing
it. Add no source, directive or CLI profile-selection syntax in this delivery.

The profile is fixed across directly linked Moth modules, generated functions,
Core/Builder source packages and Moth-source dependencies. It never varies by
function, target partition or debug/release representation. Source dependencies
compile for that profile. Incompatible precompiled Moth artefacts require a
matching build or rejection, never silent numeric adaptation. Foreign components
remain separate value boundaries with explicit foreign types.

Different profiles may change Int overflow limits, Float rounding and folded
results. The same profile must preserve behaviour across JS and Wasm. Thread the
profile through every compiler service that types numbers, including synthetic
sources, config, direct templates and MON schema preparation using Int/Float.
Include it in semantic compilation/reuse inputs and generated, package, ABI,
layout and persisted-artefact compatibility. A private body's numeric behaviour
matters even when an exported signature contains only explicit-width types.
Use the existing fingerprint owners rather than creating a parallel cache system.

### Literals and receiving contexts

Preserve the delivered numeric grammar and lossless retained spelling. Extend
semantic materialisation rather than replacing the lexical owner or parsing all
numbers through i32/f64. An unconstrained whole literal defaults to Int. A decimal
or exponent literal defaults to Float. Small values never infer I8/U8 by size.

A direct typed boundary materialises the literal in its requested numeric type.
This allows a U64 literal above Int32's range without first constructing an Int.
Whole literals may initialise binary floats. Decimal/exponent spelling does not
initialise fixed integers, Int or Byte merely because its value is integral.
Number's exact decimal materialisation is the separate rule below.

```moth
small U8 = 200
large U64 = 18_000_000_000
half F16 = 0.5
money Number2 = 12.34
raw Byte = 255
```

An immediate concrete numeric peer may type an otherwise untyped literal before
operator promotion. Thus `channel U8` plus literal `1` produces U32. Literal
`300` cannot materialise as that U8 peer. Use an explicitly widened operand for
that expression. A receiving annotation does not retrospectively retag an
operator's result. Already typed variables and constants keep their identities.
Generic inference uses these same local rules and gains no distant conversion
search. Parentheses do not add a new receiving boundary.

Binary float literals round directly to their destination using round-to-nearest,
ties-to-even. Inexact values such as 0.1 are valid. Reject materialisation whose
rounded result is non-finite. Preserve subnormals and signed zero. Integer minima
are materialised with their sign without first rejecting their positive magnitude.
Negative values cannot initialise U* or Byte.
No radix literals, numeric suffixes or other lexical extensions are added.

### Arithmetic and comparisons

Keep the existing convenience family: Int arithmetic returns Int except `/`,
which returns Float. Mixed Int/Float arithmetic returns Float. Existing implicit
Int-to-Float conversion remains and rounds at the selected Float precision.
There is no implicit Float-to-Int conversion.

Typed Int/Float values do not implicitly mix with explicit-width values. Fixed
integers and fixed binary floats also require an explicit conversion to mix,
apart from integer `/` below. Byte never participates. These restrictions apply
to comparisons too, except the existing Int/Float comparison relationship.

For fixed integer `+`, `-`, `*`, `//`, `%` and `^`, first select a common type from
the complete operand ranges, independently of known values or receiving context:

- Same signedness: the wider operand width, with a minimum of 32 bits.
- Mixed signedness: the smallest signed type, at least 32 bits, that represents
  every value of both input types.
- No available common type: a source diagnostic. No unsigned reinterpretation
  and no implicit arbitrary-precision fallback.

```text
U8 + U8 -> U32             I8 + I16 -> I32
U8 + U16 -> U32            I16 + U16 -> I32
U32 + U64 -> U64           I32 + U32 -> I64
I32 + I64 -> I64           I64 + U32 -> I64
I64 + U64 -> invalid       I32 + U64 -> invalid
```

Compatible fixed integer `/` operands use the same common-integer eligibility
rule, then convert to F64 and divide, producing F64 in every profile. `//`
remains integer division. Division truncates toward zero and remainder follows
the dividend's sign. Preserve checked zero divisors and invalid integer exponent
behaviour. Signed minimum divided by -1 overflows its result type. Its remainder
by -1 is zero and must not accidentally inherit the division overflow check.
`^` remains exponentiation, not XOR.

Unary minus on every typed U* value is a compilation error, including U8 and a
known zero. Cast to a signed type first. There is no automatic signed promotion
for unary minus. Signed narrow integers use the corresponding I32 arithmetic
domain. I32/I64 and Int negation check their own result limits.

Comparisons between any two fixed integer types are exact, including I64/U64.
They need no arithmetic common type. Preserve negative-versus-unsigned ordering
and all 64-bit values without converting through a binary float or subtracting.

Fixed binary float arithmetic uses the wider operand precision with a minimum
of F32: F16/F16 and F16/F32 produce F32, while any F64 operand produces F64.
F32/F32 produces F32. `//` remains unavailable on binary floats. Unary negation
uses the same minimum F32 precision for fixed floats. Float retains its own
profile-selected semantic type.
Comparisons use their numeric values and treat positive and negative zero equal.
All binary float values are finite, with rounding at each semantic operation
result. Preserve zero-divisor and invalid-result failures and keep const/runtime
power and remainder behaviour aligned with the canonical operator contract.

### Conversions and checked boundaries

Implicit receiving conversions do not narrow typed numeric values. Keep the
existing Int-to-Float convenience conversion. Fixed-width implicit widening
inside operators is operator promotion, not permission for arbitrary receiving
coercions. Explicit variable conversions use the current immediate-target cast
syntax and evidence rules. Type names are not conversion functions.

Integer conversion is infallible when the complete source range fits the target
range and otherwise fallible. Byte/U8 explicit conversion is infallible in both
directions. Other Byte numeric conversions compose through U8 and are not added
as another implicit conversion lattice.

Binary integer-to-float conversions round using ties-to-even. Float-to-integer
conversion truncates toward zero and checks the resulting integer range. Float
narrowing rounds using ties-to-even and fails only if it produces a non-finite
result. Precision loss alone is not a failure. Float widening is infallible.
Classify builtin evidence by source/target policy, not an optimiser's value proof:

```text
U8 -> U16, U8 -> I16          infallible
U16 -> U8, I8 -> U8           fallible
I64 -> F64, U64 -> F32        infallible, may round
I32 -> F16                    fallible finite-range conversion
F64 -> F32, F32 -> F16        fallible finite-range conversion
F32 -> F64                    infallible
F64 -> I64                    fallible, truncate then check
```

Matching Int/I32 or Float/F64 representations still require explicit casts
between their distinct types. Same-type casts remain invalid. Plain `cast`
requires infallible evidence. `cast!` and `cast ... catch` retain their existing
propagation/recovery rules and handle conversion failure, not an operand's
separate numeric failure. A proven-safe fallible cast can fold or lose its
runtime check without making plain `cast` legal for that type pair.

Cast does not mean universal exactness: binary conversions explicitly permit the
rounding/truncation above. Integer-to-integer casts preserve value and Number
scale casts remain exact. No modulo, wrapping or saturating conversion is added.
Retain other existing Bool/Char/String/Error cast contracts. Add complete numeric
String parsing/formatting for the new numeric targets through the shared grammar
and formatting owners. Byte parsing/formatting remains an explicit U8 composition
rather than inventing a default text encoding in this plan.

Compound assignment computes in the normal promoted domain, then performs a
checked conversion to the left-hand type. It is the deliberate receiving-context
exception to explicit narrowing. Evaluate the place and right operand once and
write only after arithmetic and conversion succeed. `/=` on an integer retains
the existing `/` result and uses the specified float-to-integer conversion, not
integer-division semantics. Source programs needing integer division use `//=`.

Every ordinary operation checks its semantic result type, not the operands'
storage widths. A U8/U8 sum is a U32 value and does not require an 8-bit mask.
U32 operations still need overflow checks despite using i32 carriers. A narrow
store is permitted only after source-level range proof or checking. Source-known
numeric failure is a compile-time diagnostic even in an Error! function. Runtime
numeric and compound-conversion failures use builtin Error! when available and
otherwise the existing trap mode. Custom error channels gain no implicit adapter.

### Byte, collections, ranges and retained APIs

Byte is a one-byte octet value with equality and unsigned-octet ordering,
contextual whole literals 0..255 and explicit Byte/U8 conversion. It has no
arithmetic, unary minus, numeric promotion or range-loop domain. `{Byte}` remains
an ordinary collection. Compact physical storage does not create a new buffer
source type or a binary/string encoding API.

Map keys expand from String/Int/Bool/Char to include every I*/U* type and Byte.
Float/F*, Number and NumberN remain excluded. Each map still has one declared
key type. Contextual literals follow that type, while typed keys do not silently
cross nominal or width boundaries. Equality, hashing and MON duplicate detection
consume the same declared key contract. Sorting work consumes these ordered
scalar families without inferring arithmetic from Byte ordering.

Range loops retain the Int/Float rules. Fixed integer bounds and explicit steps
use normal integer promotion. Fixed float ranges use normal float promotion and
require explicit `by`. Omitted zero and unit-step values are contextual literals.
The value binding has the selected range domain and the optional iteration index
remains Int. Evaluate bounds/step once in source order. `by` remains a magnitude,
so descending unsigned ranges lower with checked subtraction rather than source
unsigned negation. Number/NumberN ranges are outside this delivery.

Test ascending, descending, empty and inclusive ranges at numeric endpoints.
Termination must not require constructing an out-of-range sentinel after the
last valid endpoint. Any update genuinely needed by the existing range contract
uses normal checked numeric behaviour. Retain existing loop-control semantics.

Collection/string indices, lengths, fixed-capacity constants, iteration indices
and builder limits remain Int. Preserve useful Int/Float examples and APIs.
Bounds, allocation-size and address-width checks remain separate from Int's
profile-selected range. This plan's deliberate existing-API type change is
Error.code, described below. Source-level bitwise operations, alternate-radix
literals and possible Core-package byte operations are outside scope. The package
route is not an accepted future design decision.

### Number and NumberN

Number is an arbitrary-precision integer at scale zero. Number1..Number256 are
arbitrary-precision scaled integers representing `coefficient / 10^scale`. Scale
is canonical semantic identity. Keep one exact-value/arithmetic owner, not a
Number parser beside a MON parser or one runtime per scale.

Same-scale `+` and `-` are exact. Multiplication computes exactly then rounds to
the result scale using half-even. Positive-scale `/` checks zero and rounds to
the result scale. Scale-zero `/` is invalid. Scale-zero `//` truncates toward
zero and `%` is integer remainder. Positive-scale `//` remains invalid and `%`
uses decimal remainder at that scale. Unary minus is exact. Retain `NumberN ^
Int` with non-negative exponent and half-even rounding where needed. An exponent
is not silently converted to Number. Round at every language operation result,
not only at assignment or final formatting.

Ordinary mixed Number arithmetic/comparisons can scale Int and every fixed
integer exactly into the Number operand's scale. Different Number scales require
an explicit conversion. Binary floats and Byte never participate implicitly.
Fixed integer arithmetic never automatically falls back to Number on overflow.

Whole literals materialise exactly. Decimal/exponent forms require exact scale
representability, including exact integrality for scale zero. `Number2 = 1.239`
fails rather than rounding. Integer-to-Number conversion and scale widening are
infallible. Scale narrowing and Number-to-integer are fallible exact conversions:
removed fractional digits must be zero and the integer must fit its actual
explicit/profile-selected target. String parsing is exact. Lossy Float/Number
helpers and explicit rounded scale conversion remain deferred.

Canonical Number formatting trims fractional trailing zeroes, has no exponent
notation or negative zero and includes a leading zero before a fractional value.
Templates use the common value-to-string owner. Display-width formatting remains
a separate library concern. Number is an immutable value, not a source-visible
BigInt or general numeric trait system.

### Backend contracts and layout

```text
Semantic type       Linear-memory bytes       Wasm carrier
I8/U8, Byte          1                         i32
I16/U16             2                         i32
I32/U32             4                         i32
I64/U64             8                         i64
F16                 2                         f32
F32                 4                         f32
F64                 8                         f64
Int, Float          selected profile          corresponding carrier
```

For the ordinary Wasm scalar layout, natural alignment equals scalar byte size
and a scalar collection's stride is its scalar size. Keep semantic type, storage
layout and computation carrier separate. Use signed/unsigned narrow loads and
store8/store16. A four-U8-field Pixel has four payload bytes before any separate
aggregate-layout constraint. Other targets provide their physical layout policy.
General struct offsets, tail padding, packing, `$layout`, aggregate allocation
and collector-free runtime implementation remain with their existing owners.

F16 loading converts binary16 bits to F32. Portable storage rounds/converts to
binary16 bits then stores 16 bits. Optional native half facilities can replace
helpers only with the same semantics. F16 locals and parameters remain valid
binary16 values even when represented in f32 carriers. An F32 arithmetic result
is not silently labelled F16 until the specified narrowing boundary.

JS uses exact Number-based integer values through U32, BigInt-based I64/U64 and
profile-dependent Int representation. Binary floats use Number with explicit
precision boundaries. U32 multiplication needs correct exact overflow behaviour
beyond Number's exact-integer range. Conversions from 64-bit integers to F32/F16
and F64-to-F16 must avoid unintended double rounding. Do not silently implement
direct F64-to-F16 as F64-to-F32-to-F16. Preserve subnormals and signed zero in
values. Ordinary display may retain the current zero normalisation, while MON's
round-trip encoding preserves the sign as its format contract requires.

Checked i64 arithmetic needs real overflow detection. Wasm carrier wrapping,
masked shifts, truncating stores and JS bitwise signed coercion are not source
semantics. Share numeric checks only where both backends consume identical facts.
Optional range analysis writes side tables over validated HIR and never changes
source validity, failure order or Number rounding boundaries. Start with bounded
integer/proven-narrowing facts. Preserve effectful evaluation even when a check
is removed. Broader proof algorithms and ReturnError CFG redesign are deferred.

Deliver real standalone Wasm scalar execution for fixed numerics and Byte,
including conversions, checks and memory operations. Reuse existing requests,
LIR and runtime/helper ownership. This does not implement the later HTML mixed
partition, structured-control-flow rewrite, full aggregate runtime or WIT loader.
Do not claim aggregate support from low-level scalar storage tests. Unsupported
reachable aggregate/host operations continue to receive target diagnostics.
Number runtime support is HTML-JS in this plan. Wasm rejects reachable Number
values, operations, casts and formatting until its arbitrary-precision runtime
is delivered separately. Unreachable private helpers do not fail target checks.

Foreign widths map to explicit Moth widths. WIT s8..s64/u8..u64/f32/f64 map to
I*/U*/F32/F64, not Int/Float. WIT u8 is not automatically Byte. F16 requires an
explicit foreign conversion or bit-representation contract because there is no
baseline WIT F16 primitive. Keep Moth-native semantic interfaces distinct from
WIT projections. Incoming floats are validated against Moth's finite contract.
Decouple first-party language signatures from physical ExternalAbiType carriers:
a Core function declared with Int/Float remains such a function rather than
accidentally changing to I32/F64 when foreign mappings are corrected.

### Delivered MON integration

Extend the existing `moth::mon` service. Source-owned numeric grammar is shared,
but the delivered MON literal traversal and escapes remain local. Decoding never
enters AST/HIR, evaluates an expression or receives a Moth backend dependency.
Preserve owned results, prepared schemas, fail-fast errors and resource budgets.

The activation baseline has Int(i32), Float(f64), exact Integer(String) and
Decimal(String), with a decimal schema scale limited to 18. Those live MON
Integer/Decimal names are not the inactive source Decimal scaffold. Preserve
lossless generic data and extend materialisation for all new widths, Byte and
Number scales through shared numeric policies. Widen the exact decimal scale
capacity to support 256 rather than giving Number adapters a second scale parser.
Int/Float schema preparation captures an explicit numeric profile, defaulting to
the standard profile for standalone Rust consumers. Explicit-width schemas are
profile-independent. Document the actual revised public Rust API and update its
external consumer test rather than retaining compatibility wrappers.

MON whole/decimal categories stay strict. `3.0` and `1e3` cannot decode as an
integer or Byte. Fixed floats materialise with their binary rounding contract.
Number schemas remain exact. Writer output round-trips each supported typed
value, including F16/F32 and negative zero, under the same schema/profile.
Map key validation and default duplicate checks include fixed integers and Byte.
Numeric digit/scale/output accounting remains bounded before allocation.
Automatic schemas from Moth types, `$mon`, source/runtime codec operations and
the static MON builder remain separate deferred work.

### Error.code final migration

After fixed-width support and Number's existing error paths are complete, change
the builtin Error's public field to `code U32 = 0`. Preserve `message String`,
constructor field order, error-channel identity and established numeric code
values. Negative codes and values above U32::MAX fail at the ordinary typed
boundary. Int variables require the normal explicit conversion. U32 codes above
I32::MAX are valid and must survive construction, propagation and host glue.

Update compiler-created errors, builtin cast evidence, helper literals, Core and
Builder errors, opaque/foreign error projections, public interfaces, defaults,
formatting and tests together. The Wasm carrier remains i32 with U32 semantics.
This changes the builtin runtime Error, not CompilerDiagnostic codes, process-local
IDs, the Rust MON MonErrorCode enum or every Rust integer used by diagnostics.
Retain one final Error representation and no signed-code compatibility path.

## Ownership and activation map

Refresh these locators at activation. Extend their current owners rather than
introducing one descriptor, parser or statement family per numeric type.

| Owner | Work and boundary |
|---|---|
| `src/compiler_frontend/numeric_text/` | Retain lexical scanning and source-local cold records. Add destination-aware materialisation without eager i32/f64 bottlenecks. |
| `datatypes/`, `canonical_type_identity.rs`, `module_compilation/` | Canonical widths/sign/precision/scale and compiler-owned profile inputs. Keep Int/Float identities distinct. |
| `ast/expressions/`, `ast/const_eval/`, `type_coercion/`, `builtins/casts/` | Local literal typing, promotion, cast evidence, exact/rounded conversion and folding. Share numeric value policy where genuinely identical. |
| `folded_value.rs`, `public_interface/`, generated sidecars | Complete owned scalar/Number values and profile-compatible exports/remapping. |
| `hir/numeric.rs`, HIR expressions/statements/validation and link facts | One domain/operator/failure model, explicit conversions and checked failure CFG. Derive numeric metadata from canonical types rather than retaining inconsistent copies. |
| `build_config.rs`, single-source services, `src/build_system/`, builder capability surface | Select/thread the profile before numeric inputs, preserve the narrow config domain and record compatibility. |
| `src/backends/js/`, `src/backends/wasm/`, target validators | Real lowering and demand-driven helpers. Keep target instructions out of frontend HIR. |
| `external_packages/`, Core/Builder registrations | Separate language types from foreign carriers and preserve deliberate Int/Float APIs. |
| `src/compiler_frontend/mon/`, `src/lib.rs`, public MON consumer tests | Reuse the delivered service and extend schema/value materialisation and budgets. |
| `builtins/error_type.rs`, `builtins/error_codes.rs`, JS/Wasm error helpers | Final U32 runtime Error.code migration, separate from diagnostic IDs. |

The existing HIR enum still enumerates IntAdd/FloatAdd-style operations. Replace
it with the accepted backend-neutral domain/operator/failure description rather
than multiplying enum variants by every width. Keep canonical TypeId as the type
authority. No numeric type implements a new trait-object or overload framework.

## Implementation phases

Every code phase starts with a focused failing regression or invariant test,
extends the existing owner, removes the replaced path and finishes with its
focused coverage, `just validate` and the AGENTS.md Slice review. Commit accepted
slices separately. Keep baseline SHAs, worktree state and raw measurements in
`tmp/`, not this queued plan. Do not claim a backend supported before its runtime
and boundary tests execute.

### Phase 0: Activation inventory

- [ ] Record revision/worktree state, read authorities and run baseline validation
  and non-recording `just bench-check`.
- [ ] Trace numeric literals through source, constants, public interfaces, generic
  materialisation, HIR, JS/Wasm and MON. Inventory profile-sensitive inputs and
  existing error/formatting helpers. Classify extend, replace, reuse and delete.
- [ ] Inventory every queued plan recursively for affected numeric assumptions.
  Preserve unrelated serial order and the package lane's explicit parallel state.
- [ ] Locate current runtime/artefact parity harnesses and public MON tests. Write
  the type/operator/cast expectation matrix from the contract, covering all four
  profiles. Establish how each Wasm scalar claim will be executed and inspected.

Exit: current owner map, baseline evidence and an exact contract-to-test map.

### Phase 1: Publish permanent contracts

- [x] Publish the accepted type, profile, literal, operator, cast, Byte, map/range,
  Number and Error.code contracts in the existing canonical references.
- [x] Update compiler/build/data-layout authorities for profile-dependent semantic
  inputs, physical scalar metadata and MON numeric ownership. Mark queued support
  honestly. Keep HIR, lifetime and physical-plan responsibilities unchanged.
- [x] Update paired teaching pages only where affected. Preserve useful Int/Float
  examples. Rebuild docs and complete the documentation-only gate.

Exit: implementation and downstream plans can cite permanent authorities rather
than treating this temporary file as a second language specification.

### Phase 2: Canonical types, profile and materialisation

- [ ] Add canonical fixed widths, binary precisions and non-numeric Byte. Thread
  NumericProfile through the compiler service boundaries, bootstrap and typed
  inputs before any folding. Cover defaults and all four combinations.
- [ ] Extend destination-aware literal/folded-value materialisation, signed minima,
  U64 maxima, F16/F32 bit-exact values, public projection and generic identities.
  Preserve the source-local lexical store and MON lossless data.
- [ ] Add explicit target gates for newly reachable forms until their lowerings
  exist. Exercise fields, options, collection elements, returns and generated
  functions rather than gating only visible arithmetic expressions.

Exit: canonical semantics reach HIR without eager range loss or unsupported
runtime values escaping target validation.

### Phase 3: Operators, conversions and checked HIR

- [ ] Implement the promotion/compatibility matrix, unsigned-negation rejection,
  fixed integer `/ -> F64`, integer division/remainder and fixed float precision.
- [ ] Extend cast evidence and shared numeric folding, including target-aware
  String parsing and formatting. Test conversion policy independently of codegen.
- [ ] Replace width-duplicating HIR arithmetic with canonical domain/operator/
  failure facts. Linearise casts and compound assignments with one evaluation and
  success-only writes. Update validation, display, remapping and link facts.
- [ ] Test compile-time failures separately from dynamic Error! and trap paths.
  Keep source validity independent of optional range-analysis proofs.

Exit: one backend-neutral executable numeric contract and no old Int/Float-only
operation classifier used as a hidden fallback.

### Phase 4: JavaScript fixed numeric and Byte execution

- [ ] Implement exact integer storage/arithmetic, BigInt I64/U64, profile-selected
  Int and fixed/profile float boundaries using demand-driven existing helpers.
- [ ] Implement conversions, finite checks, numeric formatting, equality/copy and
  generic/aggregate value flow. Preserve Byte's non-arithmetic status.
- [ ] Execute boundary tests, large products, double-rounding cases, negative zero,
  compound-assignment atomicity and both numeric failure modes.

Exit: supported JS programs execute the complete fixed scalar contract in every
profile without a second runtime value or error model.

### Phase 5: Wasm fixed scalar lowering

- [ ] Add or reuse i32/i64/f32/f64 operations and actual scalar size/alignment/stride
  metadata. Use signed/unsigned load8/load16 and narrow stores where appropriate.
- [ ] Implement checked 32/64-bit arithmetic, conversions, division/remainder and
  power helpers plus finite float validation. Preserve Moth failure paths instead
  of leaking host arithmetic traps where Error! recovery is required.
- [ ] Implement portable binary16 conversion and direct-rounding cases. Exercise
  parameters, locals, results, storage and arithmetic-produced F32 values.
- [ ] Run one semantic parity corpus on JS and instantiated Wasm. Inspect emitted
  memory operations and raw stored bytes. Keep Number and unsupported aggregate/
  host features explicitly target-rejected. Preserve later HTML/WIT boundaries.

Exit: real scalar execution and compact-memory evidence, without claiming the
later aggregate/runtime or mixed-target implementation has been delivered.

### Phase 6: Existing-language and MON consumers

- [ ] Extend range domains and map-key eligibility through their existing owners.
  Cover unsigned descent without negation, inclusive endpoints and unchanged Int
  indices/capacities. Add no Number ranges or first-class range design.
- [ ] Decouple foreign ABI widths from deliberate Core/Builder Int/Float language
  signatures. Test binding inputs, outputs, folded constants and finite validation
  with profile changes. Preserve opaque package-specific representations.
- [ ] Extend public MON schemas, values, profile binding and scalar adapters. Keep
  literal-only decoding, categories, budgets, default handling and map ordering.
  Build an outside-crate round-trip example using only public paths.

Exit: existing consumers understand the new types without a compatibility shim,
a second codec or unrelated source syntax/API changes.

### Phase 7: Number and NumberN

- [ ] Implement one canonical scale/value owner and exact contextual literals,
  mixed integer operations, per-operation rounding and exact cast policies.
  Generalise Number-to-Int's former i32 check to the selected destination.
- [ ] Implement HTML-JS runtime values, arithmetic, checked errors, copy/equality,
  generic flow, casts and common formatting. Add no Number-specific TIR nodes.
- [ ] Extend MON exact scale materialisation through 256 and test large exact data
  without routing it through binary floats. Preserve generic Integer/Decimal data.
- [ ] Reject reachable Number runtime work on Wasm through ordinary target checks,
  including nested types and generated functions. Retain unreachable-helper tests.
- [ ] Remove inactive source Decimal scaffolding. Keep legitimate live MON Decimal
  and implementation BigInt uses. Search dispositions distinguish those meanings.

Exit: the original Number goals are delivered on the new numeric foundation,
with a truthful Wasm deferral and one shared semantic value policy.

### Phase 8: Conservative check elision and Error.code

- [ ] Add only bounded integer range/narrowing proofs needed by the two scalar
  backends. Keep facts in side tables and preserve evaluation/failure ordering.
  Initial ReturnError lowering may retain its branch/carrier even when a predicate
  is proven safe. General CFG simplification is not part of this phase.
- [ ] Migrate builtin Error.code to U32 with default zero, preserving messages,
  constructor order and existing code values. Update every producer and consumer,
  cast, public interface and foreign projection in the same coherent slice.
- [ ] Prove codes above I32::MAX round-trip through JS/Wasm and MON's U32 schema.
  Reject negative/oversized code construction and require casts from Int variables.
  Keep compiler diagnostics and Rust MON error enums outside the migration.

Exit: one unsigned runtime error-code contract and no old signed-code fallback.

### Phase 9: Documentation sweep, validation and closeout

- [ ] Complete the documentation/comment inventory below. Review every retained
  Int/Float example rather than mechanically replacing those names.
- [ ] Run focused coverage, public MON consumer tests, `just validate`, applicable
  default/feature lanes and `moth build docs --release` (or the documented Cargo
  equivalent). Run `just bench-check` without recording benchmark history.
- [ ] Review coverage/status in both progress matrices, update real locators and
  mark only materially affected audit entries stale. Claim no new audit coverage.
- [ ] Complete the final Slice review: architecture, deletion, duplicate numeric
  owners, abstraction size, diagnostics, tests, documentation and actual gates.
- [ ] Delete this plan and its roadmap entry in the completion commit. Keep durable
  contracts and explicit deferrals, with no intermediate completed-plan state.

## Required coverage

Use table-driven subsystem tests for complete type policy and a small number of
real Moth integration cases for each distinct observable contract. Runtime cases
must take dynamic inputs so constant folding cannot stand in for backend tests.
Use one input with backend-specific expectations for parity. Test identity with
typed receivers/public interfaces, not only formatted output that erases types.

| Contract | Required observations |
|---|---|
| Identity/profile | Int differs from I32/I64, Float differs from F32/F64, Byte differs from U8/Char, all four profiles, cross-module/generated identity and incompatible profile reuse rejection. |
| Literals | Every width's min/max and adjacent rejection, signed minima, U64 above Int32 and 2^53, no inferred small type, peer literal range, decimal/exponent category rejection, F16/F32 direct rounding. |
| Integer operators | Every supported operator for signed/unsigned 32/64-bit result domains, narrow promotions, cross-width/mixed-sign matrix, I64/U64 arithmetic rejection but exact comparison. |
| Failures | Dynamic add/subtract/multiply overflow, division/remainder zero, signed-minimum/-1 distinction, exponent failures, const diagnostics versus Error! propagation versus trap, custom-channel trap mode. |
| Unary minus | Every U* type rejected even for known zero, explicit signed conversion succeeds where representable, signed minimum overflow, signed narrow promotion. |
| Casts | Complete evidence matrix, range edges in both directions, truncation toward zero, binary rounding without exactness rejection, non-finite failure, same-type rejection and local/propagated recovery. |
| Compound assignment | Promoted result followed by checked destination conversion, side-effectful place/RHS once, no write on failure and `/=` distinct from `//=`. |
| Binary floats | F16/F32/F64 and profile Float operations, ties-to-even, overflow threshold, smallest normal/subnormal, underflow to signed zero, direct F64-to-F16 (1.0004882812500002 becomes 1.0009765625) and integer-to-float double-rounding regressions. |
| Storage/ABI | One/two/four/eight-byte scalar layout, sign/zero extension, raw bytes and strides, Byte/U8 nominal separation, four-U8 payload size, canonical narrow/F16 params/results and U64 above signed i64 maximum. |
| Existing consumers | Typed fields/defaults/options/collections/maps, cross-module generics, returned values, template/cast formatting and unchanged Int/Float-oriented APIs. |
| Ranges | Promoted binding identity, explicit float step, ascending/descending, zero step, empty/inclusive endpoints, no unsigned negation, Int index overflow and no Byte/Number range domain. |
| Maps/sorting contract | Fixed integer/Byte keys, extreme-value equality/hash/order, duplicate MON keys/defaults, rejection of float/Number keys and no subtraction-based comparator. |
| MON | Outside-crate owned round trips, all widths and profiles, signed zero, scale 256, strict 3.0/1e3-to-integer rejection, bounded large exact input/default/output expansion and unchanged literal-only reading. |
| Number | Scales 0/1/256, invalid scales, exact literals, mixed fixed integers, per-operation half-even ties including negative values, exact narrowing, zero divisors, canonical formatting, generic JS execution and reachable Wasm rejection. |
| Error.code | U32 default/identity, unchanged existing codes, I32::MAX+1 and U32::MAX, negative/oversized rejection, explicit Int conversion, source/Core/backend-produced errors and normal Error! recovery. |
| Optimisation | Same result, error and side-effect order with checks retained/elided, no HIR mutation, Number rounding preserved and no optimiser-dependent source acceptance. |

These are coverage requirements, not a request for one fixture per matrix cell.
Keep one primary owner per behaviour and prune superseded i32/f64-only fixtures.
A source-text search is a sweep check, not runtime correctness evidence.

## Mandatory documentation and comment sweep

| Location | Required disposition |
|---|---|
| `docs/src/docs/numbers/` | Full type family, defaults/profile, literal inference, operator matrices, checks and Number scale/rounding. |
| `docs/src/docs/casts/` and Core cast-trait references | Immediate-target syntax, new evidence, integer exactness versus permitted binary rounding/truncation, Byte/U8 and Number exact conversions. |
| `docs/src/docs/errors/` | Error.code U32 with unchanged message/default/order and numeric failure-channel behaviour. |
| `docs/src/docs/loops/`, `collections/`, `structs/`, `choices/` | New scalar domains/keys and compact fields, with existing Int indices and nominal identity preserved. |
| `docs/src/docs/mon/`, `src/lib.rs` and MON Rustdoc/examples | Actual public schema/value/profile changes, scale limits, numeric category fidelity and still-deferred Moth-native integration. |
| Compiler/build/data-layout authorities and routed memory leaves | Profile as an early semantic input, scalar storage versus carrier, one owner per numeric fact and unchanged later aggregate/memory planning. |
| Core/Builder package docs and living plans | Deliberate Int/Float signatures versus fixed foreign widths, finite input/output, literal constants, ordered scalar eligibility and U32 runtime errors. |
| Language overview, cheatsheet, paired Basic pages, scaffolds and code highlighting | Preserve concise ordinary Int/Float teaching. Add new type spellings and correct Byte, cast and Error claims without pretending queued support is delivered. |
| `docs/roadmap/` recursively | Remove stale Number-first, numeric-Byte, fixed Int32/Float64, blanket I64 deletion and old MON sequencing assumptions. Preserve unrelated gates and active parallel work. |
| Rust module docs/comments, tests, fixtures and sample assets | Correct semantic claims alongside implementation, distinguishing host/carrier Rust types from Moth types. Remove obsolete paths instead of explaining compatibility. |
| Progress matrices, `index.md`, `audit-log.md` and generated `docs/release/**` | Update actual coverage/locations under their rules. Rebuild generated docs through the compiler and never hand-edit them. |

Search tracked text for `Int = i32`, `signed 32-bit`, `finite f64`, `Byte scaffold`,
`Byte is unsigned`, `StringFromI64`, `ExternalAbiType::I32`, `NumberN -> Int`,
`code Int`, `as_i32` and inactive `Decimal` naming. Review each hit. Legitimate
fixed-width carrier operations, Rust diagnostic integers, MON Decimal data and
historical evidence remain valid. Do not erase all i64, BigInt or Int examples.
Record retained-hit reasons and any unavailable validation lane in local notes.

## Deferred work

Keep general `$layout`/aggregate ABI, collector-free allocation, the HTML mixed
backend rewrite, WIT component loading, Wasm Number runtime, Moth-native MON,
Number ranges, lossy Number/binary-float helpers, wrapping/saturating arithmetic,
radix/bitwise syntax and speculative byte APIs outside this delivery. These
boundaries do not permit rejecting supported fixed scalar execution or leaving
a numeric-Byte scaffold behind.

## Target references

These describe target primitives, not permission to weaken the Moth contract.

- [WebAssembly core specification](https://webassembly.github.io/spec/core/): scalar carriers, numeric execution and narrow memory operations.
- [Component Model WIT types](https://github.com/WebAssembly/component-model/blob/main/design/mvp/WIT.md#types): explicit foreign numeric widths.
- [ECMAScript binary16 rounding](https://tc39.es/ecma262/2025/multipage/numbers-and-dates.html#sec-math.f16round): ties-to-even conversion and the double-rounding counterexample.
