# Exact numeric types, bounded inference and unified conversion

## Status

```text
STATUS: active on the user-selected expression-refactor branch
CURRENT_SLICE: Phase 0 accepted; Phase 1 construction-owner mapping next
BLOCKERS: no external prerequisite; compiler cutover and all new acceptance rows remain pending
NEXT_ACTION: map bounded construction state and finalisation before selecting the numeric inference representation
```

**Intended repository location:** `docs/roadmap/plans/exact-numeric-types-and-bounded-inference-plan.md`

**Goal:** remove the profile-selected `Int`, `Uint` and `Float` types, make exact-width numerics the only bounded numeric identities, preserve equal-operand types in eligible arithmetic and remove minimum-width promotion floors. Add deliberately bounded numeric inference and give explicit conversions one `cast` spelling. Extend expression recovery so useful safety catches remain valid across type refactors.

**Architecture:** retain numeric spelling and unresolved numeric relationships in frontend construction state. Resolve permitted body-local constraints before numeric materialisation, conversion evidence and dependent finalisation. Publish concrete canonical types, values and operation facts through the existing AST, HIR, module and backend boundaries. Reuse the existing failure system, numeric implementations and standalone lexical/MON owners.

**Technology:** safe Rust compiler and standalone crates, Moth source fixtures, JavaScript runtime lowering, supported Wasm lowering and the existing Cargo/just validation and benchmark tools.

**Specification:** Sections 2 through 6 contain the approved design and its operational interpretation. They are self-contained so the implementing agent does not need the originating chat. Publish the durable language and architecture contracts in their permanent owners during Phase 0. The numbered work phases specify delivery, not another competing semantic authority.

**Arithmetic revision, 2026-10-08:** D15 and Section 3.4 supersede the earlier requirement to preserve minimum-32-bit arithmetic. Equal-type arithmetic now preserves its type where the operator is defined. Mixed fixed types use the revised smallest-common-domain rules. This intentionally changes narrow overflow, F16 rounding and some source/backend acceptance outcomes. All unrelated approved decisions remain in force.

**Priority:** this work interrupts the typed semantic expression refactor. That refactor remains paused until this plan is complete and its restart review has been performed. Existing accepted expression-foundation work remains part of the baseline rather than being discarded or repeated automatically.

## Navigation

| Need | Section |
|---|---|
| Approved language decisions | [Decision ledger](#2-approved-decisions-and-traceability), [numeric inference](#3-bounded-numeric-inference-contract) and [cast/recovery](#4-one-cast-and-stable-expression-recovery) |
| Services, packages and data | [Migration contracts](#5-numeric-services-bindings-and-data-boundaries) and [investigated owners](#6-investigated-repository-state-and-affected-owners) |
| Work to execute | [Twelve implementation phases](#7-implementation-sequence) |
| Coverage and regression gates | [Migration ledger](#8-coverage-migration-protocol) and [acceptance matrix](#9-mandatory-acceptance-matrix) |
| Documentation, validation and completion | [Documentation edits](#10-required-documentation-and-roadmap-edits), [validation](#11-validation-and-performance-protocol) and [completion checks](#12-final-deletion-and-completion-checklist) |
| Inspection receipts and resumable handoff | [Research provenance](#appendix-a-research-provenance-and-limitations) and [agent notes](#appendix-b-agent-handoff-and-resumable-work-notes) |

## 1. Execution contract and review priorities

### 1.1 Read and preserve the project authorities

Read `AGENTS.md`, the current style guide, the testing guide and the relevant validation sections before implementation. This is a cross-stage change, so read the compiler, build and data-layout authorities in full, then follow their language and memory routes.

Required permanent references include:

- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/compiler-data-layout-design.md`
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`
- `docs/src/developer-docs/language/overview.mtf`
- `docs/src/developer-docs/memory-management/overview.mtf` and its affected scalar, ABI, borrow and failure-edge references
- The canonical unsuffixed numbers, casts, errors, collections, maps, loops, generics, bindings, constants, external-function, configuration and MON references
- The cheatsheet, both progress matrices, current roadmap and existing benchmark methodology

Use `index.md` as a locator. Follow current paths when older review material names moved files. Apply the substance of `moth-code-review-guide.md`: delete unnecessary structure, preserve ownership and reject duplicate semantic authorities. A slice review is not a new structured audit and must not claim new audit coverage.

The user's approved changes in this plan deliberately replace the old numeric-profile, eager numeric defaulting, minimum-width arithmetic and cast-spelling contracts. D15 supersedes earlier promotion-preservation wording only within the scope stated in Section 3.4. Other existing language rules remain authoritative. Resolve unrelated conflicts through the most specific accepted authority rather than using this migration to redesign them.

### 1.2 Global constraints

Keep one current production implementation. Remove replaced APIs and update their consumers together. Preserve no public compatibility aliases, deprecated `Int` wrappers, alternate profile path or permanent feature switch. Ordinary intermediate work on the branch is not a supported dual language.

The compiler still owns semantics. Build code supplies inputs and schedules services. HIR and backends receive resolved facts. Numeric inference never becomes a builder callback, target-dependent pass or runtime mechanism.

Keep semantic type, storage layout and computation carrier separate. Removing profiles does not remove debug/release build profiles, memory-plan variants, target capability checks or ABI/layout compatibility.

Preserve checked arithmetic, finite binary-float values, exact integer comparisons, decimal scale rules, evaluation order and ordinary memory/access rules. A migration failure cannot justify a trap fallback for recoverable work, a wider accidental arithmetic domain or a weakened diagnostic.

D15 changes the selected result domain, not the checked-failure policy. A formerly widened operation may now overflow, round differently or require a previously unnecessary failure path. Preserve equivalent behaviour where the semantic contract is unchanged and classify intentional arithmetic changes explicitly in the coverage ledger. The result domain must never be widened merely to preserve an old test outcome or target capability.

Use stage-owned vectors, compact typed IDs and small borrowed descriptors where they solve a real ownership problem. Reuse existing pending-expression, type, literal, constant, call and failure owners before adding storage. Add no general Hindley-Milner engine, conversion search, trait-search inference, whole-project solver, generic allocator framework or second parser.

### 1.3 Highest-risk review areas

| Risk | Required evidence |
|---|---|
| Literal defaulting or peer resolution happens too early | Later-use, reordered-use, large-literal and cast-barrier tests with exact resolved types |
| Inference silently crosses the arithmetic restriction | Negative cases for named operands alongside positive raw-literal receiver cases |
| Old minimum-width domains survive in typing, folding or proofs | Complete fixed-type operator matrix and boundary failures in the selected result type |
| F16 work rounds only at storage or changes precision through its carrier | Exact per-operation rounding witnesses and concrete F16/F32 receiver tests |
| Changed failure facts are mistaken for backend regressions or silently trapped | D15-linked acceptance changes, supported-path parity and honest capability rejection |
| `Float` removal bypasses external validation | Runtime non-finite F64 boundary tests plus inspection of every success-extraction path |
| Catch eligibility becomes a failure effect | Separate authored-shape tests, error compatibility tests and dead-handler output tests |
| Profile-test deletion loses real numeric coverage | Assertion-level before/after coverage ledger, including exact rounding witnesses |
| Pending types or stale analysis escape publication | Final AST/HIR, generic-key, interface, proof-table and module validation tests |
| The migration damages accepted dense-HIR work | Paused-checkpoint comparison, supported backend tests and refreshed memory/performance evidence |

### 1.4 What completion excludes

This plan does not implement additional Wasm capabilities, generic or F32 Core Math APIs, the remaining compact-expression/native-result redesign, general directives, new Wiring execution, broader borrow precision, general collection inference, Moth-native MON integration, arbitrary compile-time function execution or the future constraints directives.

Existing supported features in those areas still need mechanical migration and regression protection. Future plans must be updated to consume the delivered exact-width contracts without reopening or reimplementing them.

## 2. Approved decisions and traceability

The interview settled D00 through D14. D00 records the accepted design that preceded the interview and D01-D14 record its fourteen substantive questions, including the user's expansion of recovery eligibility. D15 records the subsequent approved arithmetic revision. The ledger below states the resulting final contract, not the superseded promotion rule.

| ID | Approved decision | Primary delivery owners |
|---|---|---|
| D00 | Remove `Int`, `Uint`, `Float` and semantic profiles. Add numeric-only body-local inference, exact receiving boundaries and one `cast`. Apply the arithmetic domains revised by D15 and exclude backwards inference through named arithmetic operands. | Phases 1, 4, 5 and 6 |
| D01 | Builtin lengths, capacities and positional indices use `I32`. A later change follows a language-wide default-integer change, not an independent builder or API choice. | Phases 4 and 6 |
| D02 | Build inputs accept every fixed integer and binary float, with existing optionals. Retain CLI numeric spelling until the exact contract is known. Typed inputs require exact compatibility. Exclude Byte and Dec. | Phase 7 |
| D03 | Source-authored cast evidence supports all eleven fixed numeric targets, with infallible and fallible evidence pairs. Keep Byte and Dec outside this extension. | Phases 2 and 4 |
| D04 | Mixed raw whole and decimal literals in a non-empty collection select the binary-float family independently of item order. Bound integers do not change family. | Phase 5 |
| D05 | Decimal exponentiation uses an `I32` exponent. | Phases 4 and 6 |
| D06 | The standalone MON API makes a clean break from profile-dependent types and `Schema::with_profile`. Keep lossless Integer and Decimal data. | Phase 7 |
| D07 | Preserve current backend coverage under unchanged contracts and explicitly classify D15-caused changes. Additional Wasm capabilities belong to the later mixed-backend work. | Phases 3, 6 and 10 |
| D08 | Allow safety catch on authored arithmetic, casts and multi-call expressions even when currently infallible. Validate handlers normally before eliminating unreachable work. | Phase 2 |
| D09 | Multiple calls in receiver chains and separate operands both qualify. Calls nested inside arguments do not inflate the enclosing expression's count. Actual nested failures remain protected. | Phase 2 |
| D10 | A catch fallback contributes type constraints even when the protected expression proves infallible. Concrete types and the no-backwards-arithmetic rule remain unchanged. | Phases 2 and 5 |
| D11 | Preserve existing handler placement restrictions and the narrow constant/default cast-recovery exception. | Phase 2 |
| D12 | Receiving context takes priority over eligible peer context, then defaulting. It reaches unresolved literal arithmetic beside named operands but does not retype the named operands. | Phases 4 and 5 |
| D13 | A generic signature may connect an existing numeric unknown across parameters and results. Finalise concrete requests afterwards. Inspect no callee body and search no trait implementation to select a type. | Phase 5 |
| D14 | Core Math moves to F64 only. Generalise shared external finite-result validation and preserve the existing Math coverage before retiring profile fixtures. | Phases 3, 6 and 8 |
| D15 | Equal fixed numeric types preserve their type for eligible arithmetic. Remove the 32-bit integer and F32 float promotion floors, including unary negation. Mixed fixed types use the smallest allowed common operand domain. Check overflow and round at the selected semantic result type. Preserve the explicit division, unsigned-negation and Dec exceptions in Section 3.4. | Phases 1, 4, 6, 8, 9 and 10 |

### 2.1 Exact type inventory and defaults

The bounded numeric types are:

```text
Signed integers:    I8, I16, I32, I64
Unsigned integers:  U8, U16, U32, U64
Binary floats:      F16, F32, F64
```

`Byte` remains a distinct octet scalar without arithmetic. `Dec` and `Dec0` share the scale-zero identity. `Dec1` through `Dec256` retain their existing identities and semantics.

An otherwise unconstrained integer defaults to `I32`. An otherwise unconstrained binary float defaults to `F64`. There is no independent unsigned-literal default because the source grammar does not introduce an unsigned literal category. A concrete unsigned receiving context can select a `U*` type.

Value size does not choose a type. An unconstrained whole literal outside I32's range is diagnosed rather than retried as I64, U64 or Dec. Keep the complete spelling so an independently supplied U64 receiver can accept the complete U64 range without an I32 intermediate.

Preserve the existing numeric spelling grammar, including separators, exponent case, signed-literal rules and the absence of numeric suffixes or radix forms. Add no conversion constructor, explicit generic call syntax or new cast-target annotation. The source changes are the removed type names, bounded inference, one cast and the agreed recovery eligibility.

Removing the numeric profile removes semantic width selection by builders, packages and tooling. It does not replace that selection with a hidden per-project default setting. The default pair is a language policy with one owner.

### 2.2 Family selection and receiving conversion

Delayed inference refines integer signedness/width or binary-float precision. It does not turn a bound integer into a float or a binary-float binding into Dec.

Immediate raw literals retain destination-aware materialisation:

```moth
ratio F32 = 1
money Dec2 = 0.25
raw Byte = 255
```

A whole literal can be materialised directly as a binary float in such a context. Decimal or exponent spelling cannot initialise an integer merely because its mathematical value is integral. Preserve existing unsigned-negative and signed-minimum spelling rules.

For a local binding established from unconstrained whole-number syntax, the family is integer. For one established from decimal/exponent syntax, the family is binary float. Operator results follow their normal family rules. Existing typed operands, call results and constants retain their identities.

The ordinary receiving rules remain strict. Widening inside an operator is not a general assignment, parameter or return coercion. Fixed integer and fixed binary-float values require explicit conversion to mix. Preserve the existing exact Dec/integer operator rules without turning them into general receiving coercions.

The default I32/F64 pair selects otherwise unconstrained origins only. It is not a minimum arithmetic width. Once I8, U16 or F16 is selected, its eligible same-type operations retain that semantic result type under Section 3.4.

### 2.3 Concrete builtin contracts

Use I32 consistently for collection/string lengths, source-visible capacities, positional indices and automatically supplied collection/range position indices. Preserve the distinction between a range's counter domain and its optional positional index.

This gives source-visible container counts a maximum of 2,147,483,647, subject to stricter target/resource limits. Allocation byte products, offsets, alignment and address calculations use suitable checked physical representations. An I32 element count does not authorise an I32 byte-product overflow.

Ordinary range counters retain their selected exact numeric domain. Omitted zero and one values materialise in that domain. Preserve inclusive/exclusive endpoints, step direction, one-time header evaluation and termination before an unrepresentable successor is required.

Decimal exponentiation has the contract:

```text
DecN ^ I32 -> DecN
```

Its exponent receives integer context rather than the base's decimal scale. Negative exponents retain existing failure behaviour. Already typed other-width exponents require explicit conversion at a legal receiving site. This plan adds no cast-target syntax in operator positions.

## 3. Bounded numeric inference contract

### 3.1 Inference scope and publication boundaries

One function body is the outer inference boundary, including nested blocks, branches, loops and handlers. The implicit root runtime body follows the same rule. Every binding has one static type across all writes and control-flow paths.

Compile-time declarations resolve at their existing declaration-owned constant boundary. Later runtime consumers cannot retype a named constant. Function signatures, fields and public interfaces remain concrete under their existing declaration rules. Cross-module consumers use completed canonical facts.

Each concrete generic materialisation has its own body inference state. The generic declaration's signature and template retain their ordinary validation rules. No inference variable or pending recipe enters a public interface, canonical type identity, generated-instance key, folded public value or backend-facing HIR.

Synthetic single-file compilation, config compilation and direct-template services must use the same literal/default rules at their shorter completion boundaries. They do not acquire a whole-program inference pass or deferred state after returning.

### 3.2 Sources of constraints

Permit numeric constraints from exact annotations, assignments, concrete parameters, return slots, fields, typed container entries and eligible value-producing receivers. Copies can connect the same numeric unknown. Mutation constrains the binding's single type rather than creating a new type version.

Preserve the existing placement and explicit-receiver rules for value-producing blocks. Numeric inference operates inside already legal constructs rather than inventing a missing receiver to legalise different block syntax.

A generic signature can connect numeric unknowns already introduced by numeric expressions. Existing known type constructors can carry those numeric unknowns. A later concrete consumer may solve their numeric leaves while ordinary structure remains fixed.

All eligible uses participate before fallback. The first use in source order is not the winner. Exact conflicting receivers produce a diagnostic with both useful source locations, regardless of traversal or worker order.

A cast destination is not a source-type constraint. A trait implementation, an overload candidate, a failure contract, a proven value range, a backend capability or an optimisation opportunity is not a type-selection constraint.

### 3.3 Receiver, peer and default priority

Apply this priority before materialisation:

```text
eligible receiving context
    -> eligible local numeric peer context
    -> language default
```

Receiving context travels through unresolved literal arithmetic and grouping, including literal leaves beside named operands. It stops at references to other bindings, concrete values, call boundaries and explicit cast operands. Call arguments have their own parameter receivers.

For example:

```moth
base I32 = 10
total I64 = base + 1
```

The raw `1` becomes I64. Ordinary I32/I64 promotion produces I64. `base` remains I32. With `base + base`, both operands determine an I32 result, so the I64 receiver is rejected.

Peer context remains a local literal rule, not a backward constraint on a named numeric variable. Generalise the existing pending literal-arithmetic machinery to the exact numeric destinations that replace Uint and Float convenience contexts. Preserve comparison and logical-result boundaries, call receivers, cast barriers and the special Dec exponent edge.

Already concrete operator results are never retagged. Integer real division on concrete fixed-integer operands still produces F64. Contextual raw literals are materialised before the ordinary operator rules select a domain. Receiving context does not add an integer-division narrowing conversion.

### 3.4 Forward arithmetic only across named operands

#### Arithmetic result domains

D15 replaces the old minimum-width promotion matrix with these rules. Operator choice and resolved operand types determine the semantic result domain. The receiving context may first type raw literal leaves as described in Section 3.3, but cannot retype an already concrete operand or result.

| Operator family | Result-domain rule |
|---|---|
| Fixed integer `+`, `-`, `*`, `//`, `%` and `^` | Equal operand types produce that type. Mixed same-signedness operands use the wider width. Mixed signedness uses the smallest available signed type containing both complete operand ranges. No 32-bit minimum applies. Reject when no such type exists. |
| Fixed integer `/` | Retain the common-integer eligibility check above, then the existing conversion and real division in F64. The result is F64, including equal narrow integer operands. |
| Fixed binary-float `+`, `-`, `*`, `/`, `%` and `^` | Equal operand types produce that type. Different precisions produce the wider precision. F16 has no minimum-F32 semantic domain. |
| Unary `-` on a fixed signed integer or binary float | Preserve the operand type. Negating a signed minimum fails in that type. Preserve the sign semantics of floating zero. |
| Unary `-` on a fixed unsigned integer | Remains invalid, including a known zero. No implicit signed conversion is introduced. |
| Dec operations | Preserve existing exact/rounded per-operation scale rules, exact integer mixing and scale restrictions. `DecN ^ I32 -> DecN` remains the separate exponent contract. |

Binary floats still reject `//`. Fixed integers and binary floats still require explicit conversion to mix. Byte retains its separate non-arithmetic contract. Comparison and Boolean result types are unchanged.

Examples for the revised ordinary arithmetic domain:

```text
I8 + I8   -> I8              U8 + U8   -> U8
I8 + I16  -> I16             U8 + U16  -> U16
I8 + U8   -> I16             I16 + U8  -> I16
I8 + U16  -> I32             I16 + U16 -> I32
I32 + U32 -> I64             I64 + U32 -> I64
I64 + U64 -> invalid         I32 + U64 -> invalid
F16 + F16 -> F16             F16 + F32 -> F32
F32 + F64 -> F64             I8 / I8   -> F64
-I8       -> I8              -F16      -> F16
```

The common domain contains the complete input ranges, not every possible mathematical result of the operation. For example, U8 plus U8 remains U8 and can overflow. U8 minus U8 can underflow. Known operand values never choose a wider type to avoid a failure. Mixed-domain selection is symmetric in operand order and uses only supported widths, not an invented I9 or arbitrary-precision fallback.

Exact fixed-integer comparison remains independent of arithmetic common-type availability. I64/U64 comparisons must stay exact across their complete ranges.

#### Checked results and physical carriers

Every integer arithmetic result is checked against its selected result domain. A wider register or scratch integer may compute or validate the result, but cannot expose a wider semantic type, silently wrap, saturate or postpone a required overflow check until assignment. Preserve the existing division-by-zero, signed-minimum integer division (`//`) by minus one, remainder and invalid-exponent rules at the selected width. Real division (`/`) still returns F64 and does not inherit the narrow integer quotient-overflow check.

Every binary-float operation finishes at its selected semantic precision. In particular, F16 arithmetic rounds its result to F16 at each language-level operation and checks finiteness at that precision. F16 unary negation preserves F16. An F32 carrier is permitted only as an implementation of those F16 semantics, not as an F32 result that happens to be stored narrowly later. Preserve direct literal/conversion rounding, signed zero and subnormals. Keep the existing operation-specific numerical contracts rather than introducing a new correctly-rounded Core Math contract.

Required witnesses include an I8 operation with inputs 127 and 1 failing on addition, unary negation of I8 minimum failing and F16 maximum plus itself failing despite fitting in an F32 carrier. With concrete F16 operands, `(2048 + 1) - 2048` rounds to zero because the addition first rounds to F16. Deferring that rounding to the end would incorrectly produce one. Exercise these as genuinely runtime operations as well as the applicable constant cases.

#### Named operands remain forward-only

An operation can consume the resolved types of named operands and produce its result. A constraint on that result cannot infer the named operands backwards.

Assume the helper names below describe their exact parameter types:

```moth
value = 1 + 2
take_u64(value)
-- Accepted: the initializer is pending literal arithmetic.

base = 1
twice = base + base
take_u64(twice)
-- Rejected without an independent U64 constraint on base.
```

In the second case, `base` defaults to I32 and the operation produces I32. Choosing U64 for `base` to satisfy `twice` would violate this version's boundary.

A direct independent `take_u64(base)` can resolve `base` before forward promotion, making the corresponding result U64. Its placement relative to other eligible uses does not change the conclusion.

An I8/U8 receiving context types raw literal operands before operator selection. `small U8 = 1 + 2` is now valid: the literals and addition have type U8 and the checked constant result is three. It uses neither implicit narrowing nor a wider computation domain. A known out-of-range result such as `small U8 = 255 + 1` is a compile-time diagnostic, including under catch. Unknown runtime values fail through the existing failure mechanism instead.

Two concrete I8 operands produce I8 even under an I32 result annotation. Two concrete F16 operands similarly produce F16, not F32. Exact receiving compatibility rejects those wider annotations unless the completed result is explicitly converted. Converting the result afterwards preserves the earlier narrow checks and rounding. To request wider arithmetic, convert the operands before the operation at eligible typed receivers. A wider literal receiver beside a named operand may still select a mixed-width domain under D12.

#### Compound updates and shared range policy

Compound updates use the same revised arithmetic domain, then the existing checked write-back conversion where the destination differs. A same-type update has no artificial promotion/narrowing pair, but still checks the arithmetic and commits only on success. Preserve exact-once place/RHS evaluation and the established RHS-only catch scope. Integer `/=` retains its F64 division and specified checked conversion back, while `//=` retains integer division.

Range counters keep their existing independently selected exact domain and positional indices remain I32. Audit range typing and stepping wherever they reuse arithmetic-domain helpers so no removed 32-bit/F32 floor survives accidentally. Preserve range-specific endpoint, omitted-step, no-progress and termination rules rather than treating a range as an unrestricted arithmetic expression.

### 3.5 Known collection and nominal structure

Non-empty numeric collection literals can carry one unresolved numeric element type. A later exact collection receiver can resolve it:

```moth
values = {1, 2, 3}
take_u64s(values)
-- Accepted when the parameter is {U64}.
```

Known nested container, map and nominal/generic structure may carry eligible numeric leaves through the same relationships. Their shape, constructor identity, capacity and nonnumeric types still follow the existing rules. This is not structural type inference and does not add anonymous-record capabilities.

An empty container introduces no numeric-origin unknown and retains the existing explicit declaration/field-default typing requirements. This plan adds no new empty-container argument context. Later push, assignment or calls cannot infer an otherwise untyped empty container.

Mixed raw whole and decimal/exponent literals select the binary-float family independently of order:

```moth
values = {1, 2.5}
samples = {2.5, 1}
```

Both have an unresolved float precision and default to F64 unless constrained. Whole literals materialise directly at that precision. This does not convert a named integer:

```moth
count = 1
values = {count, 2.5}
-- Rejected: the bound count retains its integer family.
```

Typed collection elements require one compatible element type. Arithmetic promotion is not a collection common-storage-type search. Different already typed elements need explicit conversions. Maps preserve key restrictions, exact key identity, insertion order and duplicate-key checking after key materialisation.

Collection operations constrain the same element unknown through their known signatures. A write cannot change a previously selected element type. Explicit source mutability/access rules remain unchanged.

### 3.6 Generic calls and other deferred consumers

Given an existing generic signature `T -> T`, `value = identity(1)` may retain the integer unknown shared by the argument and result. `take_u64(value)` then determines U64 and the compiler completes the ordinary concrete instance.

This uses the signature alone. The compiler neither examines the implementation of `identity` nor searches trait evidence to determine T. A concrete result remains concrete. A generic parameter with no eligible numeric source does not gain unrestricted later-use inference.

Delay numeric-dependent argument compatibility, cast classification, bound-evidence validation and request publication until the necessary types are resolved. Retain argument-slot routing, exact source spans, access modes and stable callee identity from the existing parser. Do not parse arguments again after inference.

Member lookup may use known receiver structure. It must not search numeric types for one with a matching method. A consumer requiring a concrete numeric receiver waits for independent constraints/defaulting, then validates the result. Successful copying, collection access or signature-based generic forwarding must not accidentally become an early defaulting boundary.

The later generics plan owns closed compiler-defined numeric capability bounds and forwarding an already concrete expected type into an immediately nested generic call. Those are separate extensions, not delivered here. This plan preserves ordinary generic-template validation and D13's numeric-origin forwarding. It introduces no body-inferred capability bounds, operator overloading, general result-only inference or conditional generic error signatures. D15 removes equal-type narrow promotion as an obstacle to future type-preserving generic arithmetic, while integer real division and Dec scale/operator exceptions still require precise future capability contracts.

### 3.7 Solver design obligations

The concrete Rust representation is selected in Phase 1 against the actual paused checkpoint. The semantic relationships are fixed:

| Relationship | Permitted direction |
|---|---|
| Copy, assignment compatibility or the same numeric generic parameter | Shared/equality relationship between eligible numeric unknowns |
| Exact receiving requirement | Constrain that result or binding within its established family |
| Raw literal recipe | Receiving context can reach unresolved literal leaves before operator selection |
| Arithmetic with named operands | Resolved operands determine result through the ordinary promotion policy |
| Cast | Resolve source independently, then validate the conversion to the known target |
| Trait or failure requirement | Validate after numeric selection, never choose a numeric type |

A small body-local worklist and compact equality state are sufficient architectural candidates. Reuse existing structures where possible. Introducing a SAT/SMT solver or a general unification framework is outside scope.

Avoid defaulting a dependent operation result while its operands are still awaiting permitted independent constraints. First propagate explicit/equality/receiver information to a fixed point. Default independent unconstrained numeric origins according to the language policy, then complete forward computations and validate receiving constraints. Repeat only affected dependent work, not whole-body parsing or subtree materialisation.

Cycles such as a mutable binding updated from itself need bounded handling. Seed the binding from its explicit/initializer/receiving facts, default an unconstrained origin where allowed and validate the update forward. A cycle that cannot be resolved under these rules requires an annotation diagnostic, not backward promotion or a value-based width guess.

Represent a numeric unknown explicitly in construction state. Never reuse the removed builtin type IDs, `None`, `TypeId(0)` or an apparently valid I32 placeholder for it. Known composite structure with numeric leaves must remain construction-owned until it can be interned as a complete canonical type.

Keep counters and capacities checked. Bound pending storage by authored work and existing resource policy. Diagnostics need originating and conflicting constraints without copying complete expression graphs. Deterministic processing and source-oriented witness selection must not depend on hash iteration.

## 4. One cast and stable expression recovery

### 4.1 Conversion selection and delivery

The only conversion spelling is `cast expression`. Its target comes from an already eligible immediate typed receiving boundary. Record a cast only with a concrete destination supplied by that boundary. A generic numeric unknown that might become concrete later is not a substitute destination. The following remains invalid:

```moth
converted = cast source
take_u64(converted)
```

A later receiver may resolve a numeric binding, but cannot invent a missing cast destination.

The cast target does not constrain or immediately default its operand:

```moth
value = 1
take_u32(cast value)
take_i64(value)
```

Resolve `value` as I64 from the uncast call. Then classify I64-to-U32 as potentially fallible. With no independent source constraint, the corresponding whole binding defaults to I32 and I32-to-U32 is potentially fallible because negative values do not fit.

A known value of one may let the particular conversion fold. Its policy remains the policy of the complete source and target types. Preserve range, exactness, truncation, direct rounding and finite-result rules. Same-type casts and unsupported pairs remain diagnostics after resolution.

Preserve the distinction between an explicit cast and a compiler-inserted compound store conversion where it determines protected scope, commit timing or diagnostic attribution. Sharing conversion policy does not merge those source boundaries.

A potentially failing conversion participates in ordinary builtin implicit failure. Eligible catch handles it locally. A private no-error-slot function can carry its existing inferred failure lane. An escaping builtin Error failure uses a compatible declared Error! slot. A custom E! requires local recovery or explicit mapping. Public callable contracts must not silently gain escaping failure.

Ordinary declared fallible calls keep their existing propagation syntax. Removing `cast!` neither removes postfix `!` from function calls nor makes custom errors implicitly convertible.

### 4.2 Source-authored evidence

Register an infallible and fallible evidence family for each fixed integer and binary float. For I32, the method requirements are conceptually:

```text
CASTABLE_TO_I32      -> to_i32 returning I32
TRY_CASTABLE_TO_I32  -> try_to_i32 returning I32, Error!
```

Follow the established concrete receiver/conformance syntax. Reuse the existing metadata-driven registration and evidence lookup rather than eleven resolver implementations. Keep the existing Bool, String, Char and Error evidence families. Remove the Int and Float evidence names and method requirements without aliases.

Both evidence classes use plain cast. Same-file nominal conformance, generic nominal eligibility, stable evidence identities and visibility rules remain. Infallible/fallible evidence for one source and target stays mutually exclusive. Byte and Dec gain no source-authored evidence family. Generic cast targets and conversion-chain search remain excluded.

Source-authored cast methods lower as ordinary resolved source calls. Their actual failure contract determines delivery. Preserve the existing non-escaping guarantee of infallible evidence implementations rather than silently accepting an implementation whose inferred failure contradicts that evidence.

### 4.3 Authored safety-catch eligibility

Separate three facts: whether syntax permits a catch here, whether this authored expression qualifies and which recoverable failures it can actually produce.

An otherwise legally placed catch is eligible when its protected expression has at least one of:

1. A valid authored cast.
2. A valid authored arithmetic operation, including numeric unary negation.
3. More than one function/member call at the selected expression's call-count level.
4. An actual compatible typed or inferred recoverable failure producer.

Comparisons and Boolean operators alone do not count as arithmetic. Preserve the grammar's distinction between signed literal spelling and an authored unary operation rather than manufacturing arithmetic during literal materialisation. Compiler-inserted conversions, numeric helpers and hidden calls do not manufacture eligibility. A syntactic constructor is classified under its existing constructor semantics, not counted as a function merely because it uses parentheses.

Count receiver-chain calls and calls in separate operands. A call's argument expression has its own call-count level. Calls inside that argument do not increase the enclosing level. Ordinary grouping is transparent within the selected expression. An explicitly nested handler owns its own protected expression and fallback, so its hidden work is not borrowed to qualify another catch.

Arithmetic/cast qualification follows authored operations whose evaluation is part of the protected expression. The argument exclusion is about inflating the call count, not about losing protection or hiding an authored arithmetic operation. Record these relationships during ordinary parsing/construction rather than counting rendered source or lowered helper calls.

Assuming all named calls are infallible:

| Expression | Safety catch eligibility |
|---|---|
| `source.first().second()` | Allowed, two receiver-chain calls |
| `left() is right()` | Allowed, two operand calls |
| `outer(inner())` | Rejected by the multi-call rule, one call at the enclosing level |
| `outer(inner())` with a fallible inner call | Allowed by actual recoverable failure, not by call count |
| `left is right` | Rejected unless its evaluation contains another qualifying producer |
| `-value` for a valid signed numeric type | Allowed by authored arithmetic |
| A supported same-range infallible cast | Allowed by authored cast |
| A literal or a single infallible call | Rejected without another qualifying operation |

Eligibility belongs to the authored expression occurrence. A reference to a binding or constant does not inherit its initializer's arithmetic or call count. Shared folded values must not carry eligibility into unrelated occurrences.

Eligibility is retained before folding, constant substitution, inlining or optional check removal. A function becoming foldable or an arithmetic operation becoming infallible cannot invalidate an authored safety catch. The compiler emits no new default warning for these accepted handlers. A future LSP advisory is outside scope.

### 4.4 Handler typing, validation and erasure

The handler participates in ordinary receiving and numeric constraints before failure analysis decides whether it runs:

```moth
fallback U64 = 0
result = 1 + 2 catch then fallback
```

The unresolved literal arithmetic can receive U64. A completed concrete I32 operation cannot become U64 because its fallback is U64. Preserve multi-success arity and per-result compatibility under the current result representation.

When there is no error source, an authored bound handler error has builtin Error type. When declared compatible error producers exist, their error type determines the binding. Optional optimisation does not turn a declared E producer into a different handler-binding type. A syntax-only arithmetic/cast eligibility flag is not a fabricated Error producer that would make a custom E handler incompatible.

Validate handler names, types, access, casts, generic evidence, required constant expressions and all-path value/terminality requirements even when it is unreachable. Known invalid numeric work remains a source diagnostic. An unused handler cannot conceal an unsupported cast pair or incompatible fallback.

After semantic typing and the applicable failure-summary convergence establish that recovery cannot execute, remove its executable contribution through the current construction/finalisation owner. It contributes no emitted handler work, runtime captures, durable inactive generic requests, reachable backend requirements or executable link facts. Preserve source graph membership and ordinary declaration/visibility validation.

The current compiler can finish some private-failure decisions only during pre-publication HIR lane installation. Reuse that completion point where necessary. Do not prune on provisional absence of a failure summary. Revalidate any rewritten HIR and recompute affected borrow/proof/link facts before publication. Later optional lowerer optimisation may eliminate checks or blocks without changing source acceptance.

### 4.5 Existing recovery boundaries remain

Catch protects the complete selected evaluation, including receiver and argument work. Evaluate it once in source order. The first failure skips later protected work. Earlier mutations/IO remain visible. The handler is not a transaction, and a failure in the handler goes outward rather than into itself.

Preserve custom-error compatibility, postfix-exit conflicts and explicit optional handling. Preserve the restriction on catches in conditions, templates and constant expressions, including the existing narrow foldable cast exception at a constant/default receiver. This plan changes eligibility, not general placement.

A catch on a compound assignment RHS covers the RHS only. The subsequent arithmetic in the revised selected domain and any required checked store conversion retain their own failure obligations. Same-type arithmetic does not acquire a redundant narrowing step. Commit the store only on success. Explicit casts remain checked inside any later fast-math scope.

For an optional cast receiver, convert/recover inner T before normal wrapping into T?. Preserve the existing rejection of `then none` as that cast's inner fallback and the absence of implicit optional-source unwrapping.

## 5. Numeric services, bindings and data boundaries

### 5.1 Shared lexical and scalar ownership

Reuse `moth-lexical` for numeric grammar, exact spelling, destination-aware materialisation, binary precision and formatting. Remove semantic profile selection, not the reusable direct conversion algorithms that happen to live beside it.

Preserve signed minima, the complete U64 range, F16/F32/F64 direct rounding, signed zero, subnormals, finite overflow checks and exact decimal spelling. A larger host carrier is an implementation representation, not a semantic widening permission.

Audit every arithmetic consumer that currently equates a 32-bit/F32 computation carrier with the result type. Reuse shared scalar range/precision policy and existing checked algorithms with the revised domain. In F16 paths, establish that the carrier/helper implements each operation's required rounding and finite checks rather than assuming an F32 calculation plus a final store is sufficient.

One important existing witness must survive:

```text
U64 value:              9007199791611905
Direct F32 result bits: 0x5a000001
Forbidden f64 detour:   0x5a000000
```

Keep direct integer-to-F32 conversion rather than converting BigInt/u64 through f64/JavaScript Number first. Likewise preserve decimal literal midpoint tests that an f64 intermediate would collapse before F16 rounding.

### 5.2 External binary-float validation

Every supported external float result must be rounded/validated according to its resolved exact Moth type before ordinary Moth code can observe it. A raw external ABI carrier can be temporarily non-finite only inside that guarded boundary.

The migration must replace both the old selection predicate and the lowering helpers' hardcoded builtin Float allocations. Changing `result_type_ids_are_single_float` alone is insufficient. Trace `ValidateFloat` through ordinary calls, handled/propagated success extraction, direct returns, constants and supported nested result forms.

Use the resolved result type/precision and the existing external boundary classification. No package-name check or Math-only finite guard is permitted. Preserve the supported result-shape coverage. For a shape the compiler cannot validate, retain its honest capability rejection rather than passing unvalidated values.

Preserve the distinction between recoverable numeric failure and a foreign-value integrity guard. The existing source-declared builtin Error! boundary can provide its documented recovery route. Synthetic start and other trap-only integrity contexts keep their contract. Safety-catch eligibility alone never turns a fatal guard, assertion, resource exhaustion or unexpected foreign exception into recoverable implicit Error.

Preserve the existing diagnostic, infrastructure and trusted-metadata invariant failure lanes. Generalising value validation does not turn a malformed trusted registration into ordinary user data or an invalid external value into a compiler type-system panic.

Update validator, runtime helper demand, source/result type equality, precision arguments, fallible success carriers and target checks together. Validate only success payloads on a fallible external call. Preserve a valid returned Error's code, message and identity rather than replacing it with a float-integrity error.

### 5.3 Core and Builder package migration

Core Math's current constants, parameters and results become F64. Keep the existing package identity, constant table, homogeneous registration loop, inline JS expressions, access modes, arity and declared error contracts. Reuse existing `ExternalAbiType::F64` constant projection.

Preserve each mathematical contract rather than imposing a new globally correctly-rounded Math contract. Math's `round` tie rule remains its own. Exact identities, approximate transcendental bounds, radians, domain restrictions, clamp composition, signed-zero policy and underflow policy retain their documented meanings.

Additional Math precisions, generic math helpers, Math function constant evaluation and Wasm lowerings remain deferred package work. A cast wrapper around F64 does not constitute native F32 Math support.

D15 changes language arithmetic, not Core Math signatures or their approximation contracts. File-backed Core implementations, WAT package authorship and type-parametric bindings belong to the later generics/package plan and must not enter this migration.

Audit all other `NativeInt`, `NativeUint` and `NativeFloat` signature and constant registrations. Ordinary default-language contracts become their approved exact counterparts, usually I32, U32 and F64. Foreign signatures already declaring fixed widths retain those widths. Do not treat an ABI F64 carrier as proof that the Moth semantic type was F64 before this migration.

Include Core Text lengths, random/time packages, direct builder bindings, annotated project JavaScript, external constant projection, facade re-exports and helper inventories. Preserve each existing distribution, unit, finite boundary, positional argument and capability contract. This is numeric migration, not a package API expansion.

### 5.4 Build inputs

The numeric build-input domain becomes all fixed-width integers and binary floats, including the existing optional wrapping. String, Bool and Char remain. Byte and Dec remain excluded, as do the other currently excluded contract shapes.

Represent a command numeric input as validated retained numeric spelling/category until its exact receiving contract is known. Then materialise directly through the shared numeric owner. Keep quoted text explicitly String. Preserve current nonnumeric fallback, malformed-quote, duplicate-name, origin and unknown-input rules.

Examples:

```text
--input count=4_294_967_296  -> accepted by U64
--input ratio=1            -> accepted by F32 through literal materialisation
--input count=1.0          -> rejected by an integer contract
```

Already typed programmatic inputs and builder globals preserve their exact type. They are not reparsed or silently converted to match a contract. Ordinary compatible T-to-T? presence wrapping remains distinct from numerical conversion.

Keep bootstrap/source contract ordering, default validation, provenance and contract conflict detection. A late-known contract must not require a second compiler grammar or a build-side evaluator. Fingerprint the resolved exact type and canonical value, with the existing provenance rules, rather than an obsolete profile byte.

At the investigated checkpoint, implementation still contains the earlier `#Config` carrier path while the general `$config` directive architecture is queued. Migrate existing numeric carriers/resolution now and update the queued directive contract. Do not deliver the whole general-directives plan as an accidental dependency of this work.

### 5.5 Standalone MON

Remove profile-dependent schema/value variants, stored profiles, `Schema::with_profile` and their public re-exports. Preserve explicit-width variants, receiver limits and the lossless Integer/Decimal data families. Do not conflate Rust `SchemaType::Integer` with the removed Moth source `Int` identity.

Existing callers migrate according to their former selected widths. A former 64-bit Int schema becomes I64, not I32 merely because I32 is the source default. A former Float32 schema becomes F32. MON remains schema-directed literal data with no source inference engine.

Preserve exact typed round trips, finite/default validation, negative zero, integer map-key identity, insertion order, duplicate-key handling, byte spans, error paths, resource budgets and owned output lifetime. Share lexical policy while keeping the source and MON parsers independent.

Remove redundant repetitions of identical fixed-width tests across profile combinations only after showing that the width-specific and parser-specific assertions still have owners. No compatibility alias or implicit old-value-to-new-schema adaptation survives the migration.

## 6. Investigated repository state and affected owners

### 6.1 Branch distinction

Research found the default branch behind the accepted expression-refactor checkpoint. The expression branch's plan records accepted Phase 3 work and a pause after template control-flow simplification. It contains dense HIR value/place ownership that is absent or differently represented on the older default branch.

Activate on an isolated tree that preserves the user's accepted checkpoint and any required integrated changes. Record the actual base and branch relationship at activation. Do not assume an unmerged branch is already on main, discard it, force-reset it or resume its Phase 4 while this plan runs.

The research revisions are recorded in Appendix A as inspection provenance only. They are not an implementation baseline or permission to ignore newer accepted changes.

### 6.2 Confirmed high-impact findings

| Finding | Existing owner | Required change |
|---|---|---|
| Contextual arithmetic already has an indexed pending-literal path | `ast/expressions/eval_expression/result_type.rs` and `evaluator.rs` | Extend/reuse its recipes and ordered witnesses instead of a second numeric parser |
| Fixed arithmetic currently imposes minimum integer/float domains | `datatypes/numeric_operators.rs` and its `common_fixed_integer`, `fixed_float_domain`, unary/domain consumers | Replace the old floors with D15 and update failure discharge, folding and lowering together |
| Named operands currently expose a concrete expression TypeId immediately | Same result-typing owner and declaration construction | Add explicit construction-owned pending numeric relationships before materialisation |
| Generic calls eagerly infer, check evidence and record concrete requests | `ast/generic_functions/calls.rs` | Defer numeric-dependent completion while keeping parsed routing and callee identity |
| Cast spelling chooses evidence/handling today | `builtins/casts/resolution.rs`, `expression_types.rs` and HIR cast lowering | Separate conversion policy from authored syntax and reuse implicit-failure delivery |
| Catch eligibility already survives folding through a numeric flag | `ast/expressions/failure_facts.rs` | Generalise authored-shape metadata without fabricating failure effects |
| Private catch edges can be resolved/pruned after summary convergence | `hir/private_failure_lane.rs` | Reuse pre-publication completion and refresh every affected analysis |
| External validation selects builtin Float and allocates that type | `hir/hir_expression.rs`, `hir_expression/calls.rs`, `numeric.rs` | Drive selection, allocation and validation by exact result type/precision |
| CLI numeric input is materialised before contracts are consulted | `compiler_frontend/build_config.rs`, `projects/cli.rs` | Separate retained command literals from exact typed values |
| MON public and prepared schemas carry profiles independently of the compiler | `crates/moth-mon/src/model.rs`, `schema.rs` | Break the Rust API cleanly and preserve explicit schemas |
| The integration harness itself accepts numeric profiles | `integration_test_runner/expectations.rs` and its consumers | Migrate fixture meaning before deleting the profile field and driver |

Paths beginning with `ast/`, `hir/`, `datatypes/`, `builtins/` or `type_coercion/` in owner tables are relative to `src/compiler_frontend/` unless stated otherwise.

### 6.3 Owner map for implementation searches

| Area | Starting owners to inspect and follow |
|---|---|
| Lexical numeric policy | `crates/moth-lexical/src/numeric/{profile.rs,parse.rs,precision.rs,fixed_scalar.rs,decimal.rs,format.rs}`, numeric tests, source-word/reservation tables and crate exports |
| Tokens and declarations | `src/compiler_frontend/keywords.rs`, `tokenizer/`, `numeric_text/`, `declaration_syntax/type_syntax/`, `declaration_syntax/build_config_contract.rs`, header shells and constant ordering |
| Canonical types and values | `datatypes/{ids.rs,numeric_scalar.rs,numeric_operators.rs,environment.rs}`, canonical type identity, folded/public constants and type display |
| Pending expression construction | `ast/expressions/{expression.rs,expression_kind.rs,expression_rpn.rs,parse_expression.rs,parse_expression_dispatch.rs}`, `eval_expression/` |
| Receiving boundaries | `type_coercion/{parse_context.rs,compatibility.rs,string.rs}`, local declarations, mutations, returns, fields, defaults and value-producing blocks |
| Calls and generic work | `ast/expressions/{call_argument.rs,call_arguments.rs,call_validation.rs,function_calls.rs,generic_nominal_inference.rs}`, constructor owners, `ast/generic_functions/`, generated materialisation |
| Constants and templates | `ast/const_eval/`, `ast/const_values/`, module finalisation, template/TIR preparation, folding, views and runtime handoff |
| Cast evidence | `builtins/casts/{resolution.rs,targets.rs,evidence.rs,policies.rs,traits.rs}`, core trait registration and source conformance validation |
| Catch and implicit failure | `ast/statements/fallible_handling/`, `ast/expressions/{failure_facts.rs,failure_classification.rs,mutation.rs}`, public summaries and function-boundary validation |
| HIR and proofs | `hir/hir_expression.rs`, `hir/hir_expression/{calls.rs,numeric.rs,fallible/}`, `hir/private_failure_lane.rs`, `hir/failure_facts.rs`, HIR validation, numeric proof and generated-module owners |
| Builtin containers and ranges | `builtins/` collection/map signatures, range typing/folding, `hir/hir_statement/loop_lowering.rs`, backend collection/map/range runtimes |
| External surfaces | `src/builder_surface/core_packages/`, `compiler_frontend/external_packages/`, external constant import projection, annotated JavaScript provider and binding validation |
| JS | `src/backends/js/{mod.rs,js_expr.rs,runtime/,package_bindings/,tests/}`, especially numeric, collection, map, cast and formatting helpers |
| Wasm | `src/backends/wasm/{request.rs,runtime/memory.rs,...}`, numeric lowerers/validators/emitter tests, HTML-Wasm request and glue owners |
| Build orchestration | `src/build_system/build.rs`, `create_project_modules/`, config bootstrap/resolution, package compilation, immutable artifacts and fingerprint consumers |
| Project services | `src/projects/{cli.rs,check.rs,boracle.rs,settings.rs}`, HTML builder/options/compile inputs, direct-template and synthetic services |
| MON | `crates/moth-mon/src/{model.rs,schema.rs,reader.rs,writer.rs,map_keys.rs,lib.rs}`, local/public tests and compiler convenience re-export |
| Harness and tooling | `src/compiler_tests/integration_test_runner/`, `src/compiler_tests/mon_syntax_parity/`, `src/first_party_js/`, `xtask/`, feature lanes and source audits |
| Performance | `src/benchmarking/frontend.rs`, frontend instrumentation, `benchmarks/`, benchmark generators/manifests and relevant xtask commands |
| Documentation and presentation | Canonical language/developer docs, both matrices, cheatsheet, roadmap/package plans, `src/projects/html_project/styles/code.rs`, examples/scaffolds and `index.md` |

This is a starting map, not a claim that every caller was exhaustively audited during planning. Phase 0 refreshes it with repository-wide searches and the actual activation tree. A moved owner is followed, not recreated at an obsolete path.

## 7. Implementation sequence

### 7.1 Checkpoints and dependency order

Implement the numbered phases in order. Within a phase, use the smallest coherent owner-and-consumer slices and run the corresponding tests before proceeding. A table row below is a work package, not permission to omit its consumer migration or validation.

| Phase | Deliverable | Acceptance boundary |
|---|---|---|
| 0 | Paused-checkpoint preservation, baseline, coverage ledger and published contracts | Activation review |
| 1 | Bounded inference construction design and executable semantic probes | Representation/semantic gate |
| 2 | Unified cast and authored safety-catch eligibility on the current compiler | Focused feature gate |
| 3 | Shared exact-width value and external-float boundary preparation | Boundary-preservation gate |
| 4 | Canonical exact numerics, revised arithmetic domains and scalar inference cutover | Part of integrated cutover C |
| 5 | Containers, handlers, generics and service finalisation | Part of integrated cutover C |
| 6 | Builtins, Core Math, external registrations and backend migration | Part of integrated cutover C |
| 7 | Config, MON, orchestration and profile removal | Part of integrated cutover C |
| 8 | Fixture, harness, example and documentation-source migration | Integrated cutover C closes |
| 9 | Adversarial, publication, ordering and regression hardening | Full semantic review |
| 10 | Performance, documentation delivery and final validation | Merge-readiness gate |
| 11 | Expression restart reassessment, downstream handoff and retirement | Completion gate |

Phases 4 through 8 intentionally form one integrated acceptance checkpoint. Removing a builtin identity affects many Rust matches, public values, fixtures and service inputs at once. Split the work into reviewable dependent commits or local packages, but claim no supported checkpoint until those owners agree. Do not create aliases, dual parsers or ignored tests just to make an incomplete cutover look green.

This does not require running the full suite after every edit. Use focused crate/subsystem checks while the relevant package builds, record incomplete dependent work honestly and run the integrated gates when the complete path is available. Every main-bound result still needs the current full validation policy.

### Phase 0 - activate without losing accepted work

**Consumes:** this approved plan and the latest accepted paused compiler checkpoint.

**Produces:** an isolated worktree/branch, explicit expression pause, current baseline, coverage ledger, owner inventory and permanent design updates.

#### 0A. Establish the actual base

- [x] Read current local/remote branch state, working-tree changes and the expression plan's accepted checkpoints. Preserve the user's existing branch and uncommitted work.
- [x] Identify which expression phases and template changes are accepted but not merged to main. Record their relationship to the intended numeric worktree.
- [x] Use the existing `expression-refactor` branch and worktree, as explicitly selected by the user instead of a separate implementation worktree. Preserve the accepted checkpoint without resetting or overwriting other work.
- [x] Record the actual base SHA, branch, toolchains, features, build flags and source cohort under `tmp/exact-numerics/`. A research SHA in Appendix A is not a substitute.
- [x] Put the new numeric work first in the roadmap sequence and mark it active only when implementation activates.
- [x] Change the expression plan's small status capsule to paused for exact numeric/inference/conversion completion and subsequent restart reassessment. Preserve accepted checked items and their historical evidence.
- [x] Prevent overlapping edits to expression construction, numeric bindings and fixtures by other active work. Unrelated package work may continue behind stable interfaces. Do not pause unrelated programmes wholesale.

Suggested expression pause wording:

```text
STATUS: paused after the accepted checkpoint for the exact-numeric refactor
CURRENT_SLICE: preserve accepted work, no further expression implementation
BLOCKERS: exact numeric types, bounded inference, unified cast and safety catch, followed by restart reassessment
NEXT_ACTION: after numeric completion, refresh owners, lifetimes, coverage and benchmarks before proposing resumption
```

The actual accepted checkpoint belongs in its existing history/status context. Do not replace a later accepted checkpoint with the older Phase 3 snapshot merely because this plan inspected it.

#### 0B. Inventory semantics and evidence

- [x] Search tracked production code, tests, fixtures, benchmark sources, docs, scripts, schemas and feature-gated owners for the removed names and profile carriers. Classify each hit by owner and meaning.
- [x] Trace semantic identity through parsed types, TypeId interning, public canonical types, folded values, HIR constants, generated keys, casts, external ABI projection and output helpers.
- [x] Inventory current support separately for HTML-JS and HTML-Wasm. Record successful runtime cases, target rejections and actual failure-delivery routes.
- [x] Record old minimum-width arithmetic, unary negation, F16 rounding and full-domain proof cases separately. Identify which source outcomes intentionally change under D15 and which still require equivalent target coverage.
- [x] Complete the assertion-level coverage ledger described in Section 8 before modifying fixtures. Include the source revision, old profile, semantic property, backend and replacement owner. Activation review corrections now include numeric-proof/TIR-range owners and 120 generated float instances.
- [x] Run the current integration audit and selected baseline suites. Capture existing failures without editing expectations to suppress them.
- [x] Capture five matched non-recording benchmark invocations for the affected surviving cohort under Section 11. Record targeted baseline probes if the full cohort is temporarily blocked.
- [x] Inventory current `#Config` implementation versus queued `$config` semantics, current result carriers versus future native results and current external float integrity routing. These are separate migration boundaries.

Activation evidence is local under `tmp/exact-numerics/`: `activation-baseline.md`, `baseline-performance.md`, `production-owner-census.json` and `coverage-ledger.md`. The inventory records 127 explicit-profile fixtures, their 183 backend expectation blocks and 350 assertion rows, plus 459 named Rust tests/examples across 54 owners. All 136 new acceptance rows retain open replacement dispositions. Five native frontend and five CLI full-fidelity invocations passed on the unchanged compiler/workload cohort. Changed documentation workloads are excluded from the 41 frontend and 39 CLI comparable cases. All measurements are retained, including substantial unexplained variance in the first two frontend runs. Final comparison uses the median of all five invocation medians per matched case, with individual statistics reported.

Independent numeric, inference, service, architecture, rendering, native-evidence and activation-evidence reviews accepted Phase 0 after corrections. The publication passed docs checking, a 79-output release build, actual Chromium inspection of 30 affected routes and introduced-link/anchor checks. The native benchmark evidence owner passed five focused xtask tests, package formatting, all-target Clippy and full/quick command smoke. Compiler numeric behaviour and fixture migration remain unchanged at this checkpoint.

#### 0C. Publish the approved design

- [x] Update the permanent numeric, cast, catch, generic, collection and compiler/build contracts listed in Section 10 with the agreed final design.
- [x] Keep implementation support explicitly in the progress matrices. Future syntax in explanatory code blocks does not claim executable support.
- [x] Record the numeric changes required by the queued general-config, constraints, mixed-Wasm and living package owners using capability wording.
- [x] Preserve historical audit and benchmark measurements. Add current supersession/status notes only where needed, rather than rewriting history to appear current.

**Validation:** inspect roadmap and authority diffs, run the integration inventory audit, use the documentation branch gate and record baseline commands/outcomes. Review every removed or replaced design sentence for lost invariants.

**Exit:** one agreed base, no lost accepted work, a nonempty evidence ledger and explicit new contracts. The expression plan is paused in the actual working tree, not merely mentioned in this document.

### Phase 1 - select the bounded construction model

**Consumes:** current expression/declaration/generic owners and the approved inference examples.

**Produces:** one construction-state design, permitted-edge tests and a dependency/finalisation map. This phase owns the smallest new numeric inference substrate, not the postponed whole-expression storage refactor.

#### 1A. Map incomplete and completed owners

- [ ] Identify the point where local declarations currently require a concrete TypeId and where numeric spelling becomes a concrete value.
- [ ] Map every consumer that can run before body completion: field/member lookup, collection operations, constant evaluation, generic argument/evidence checking, TIR expression intake, catch typing and signature/result construction.
- [ ] Separate retained syntax/recipe ownership from completed semantic values. Determine which existing indexed RPN or pending records can serve body-local numeric resolution.
- [ ] Define how an eligible numeric binding/reference is represented before concrete completion. Keep source spans and stable declaration identity without a fake semantic type.
- [ ] Define the lifetime of pending numeric state for functions, root runtime, named constants, generic templates/instances, config and direct-template services.
- [ ] Specify the convergence point for private failure summaries and the owner that prunes unrouted handlers. Preserve the current pre-publication lane installer and analysis refresh contract where used.

#### 1B. Exercise semantic probes before broad migration

- [ ] Build focused tests for equality constraints, explicit receivers, family mismatch, defaulting and deterministic conflict witnesses.
- [ ] Include the accepted cast-source example, raw arithmetic receiver example, named-arithmetic rejection, collection element inference and signature-only identity generic.
- [ ] Probe the revised complete scalar type-pair matrix, same-type narrow results, mixed narrow domains, unary negation and per-operation F16 completion before selecting the construction representation. Keep promotion policy separate from permitted inference edges.
- [ ] Add order permutations that must preserve results: swapped independent use order, reversed collection literal order and reversed peer position.
- [ ] Exercise a mutable self-update, a long copy chain, fan-out uses and a dependency cycle. Prove termination and annotation diagnostics without backward arithmetic search.
- [ ] Compare at most two justified storage choices if the existing owners do not determine the representation. Measure pending bytes, allocations, worklist revisits and reclamation. Keep experiments local and delete losing implementations.
- [ ] Use exact fixed-width types in probes. Do not use the old Int/Float semantic identities as inference variables or introduce an unrestricted generic unknown.

#### 1C. Fix interfaces and deletion obligations

Record the selected construction interfaces, issuing ID domain, finalisation barriers and which older immediate-materialisation helpers will be removed or narrowed. Define how dependent generic/cast/handler work is resumed without reparsing.

A suitable interface separates these actions, regardless of their final Rust names:

```text
create numeric origin with spelling/category and span
connect permitted equality or exact-receiver requirement
record directed operation dependency
record deferred conversion to an independently known target
resolve permitted relationships and deterministic defaults
materialise concrete expressions and complete dependent checks
validate that no pending numeric state escapes the owner
```

**Validation:** focused solver/recipe tests plus real-source frontend probes through the existing parser. Record baseline failures for the new source cases and use local experimental slices for construction probes. Their complete source integration belongs to Phases 4/5. A unit solver that passes while source parsing still defaults early is insufficient evidence for the delivered feature.

**Exit:** the permitted directions are explicit, all approved example outcomes are fixed and a single bounded representation is selected. No new semantic choice is hidden inside a scheduling convenience.

### Phase 2 - unify casts and implement authored safety catch

**Primary owners:** `builtins/casts/`, AST cast dispatch/types, `ast/statements/fallible_handling/`, failure facts/classification, HIR cast/catch lowering, private failure lane and their tests.

#### 2A. Cast spelling and evidence

- [ ] Add tests demonstrating plain cast for supported infallible and fallible conversions, local recovery, builtin Error delivery and private inferred failure.
- [ ] Remove the special `cast!` grammar and old spelling-selected evidence requirement. Preserve ordinary postfix propagation and its exact source span.
- [ ] Select evidence by the resolved source/target pair, then choose ordinary failure delivery. Preserve unsupported/same-type/optional-source diagnostics.
- [ ] Route a fallible source-authored cast implementation through the same selected expression failure context as a builtin conversion. Preserve operand evaluation once and ordinary access/effect summaries.
- [ ] Add the fixed-width source-evidence metadata families and their conformance/visibility/duplicate/conflict tests. Keep one registration/lookup owner.
- [ ] Remove obsolete cast-only error reasons only when their semantics are truly retired. Never reuse a stable diagnostic code for a new meaning.
- [ ] Update all existing positive and negative cast fixtures. A negative `cast!`-required test becomes either a removed-syntax test or a test of the real missing recovery/public-contract rule, with a ledger entry.

#### 2B. Authored eligibility and parser boundaries

- [ ] Record enough compact authored metadata to distinguish arithmetic, cast and zero/one/multiple same-level calls. A saturating call count is a candidate, not a mandated extra field on every durable node.
- [ ] Preserve it through grouping, folding and unchanged-node reuse until catch eligibility is checked. Do not derive it from emitted HIR helpers.
- [ ] Cover method chains, calls in separate operands, nested argument calls, transparent groups, constructors, hidden conversion calls and explicit nested handlers.
- [ ] Continue to collect actual failures across the full protected evaluation even where nested calls do not count for leniency.
- [ ] Keep syntax eligibility separate from actual typed/builtin failure contributors and custom-error compatibility.
- [ ] Preserve the existing placement validator. Add no handler support to previously prohibited conditions, templates or constant contexts.

#### 2C. Handler semantics and pruning

- [ ] Permit a normally validated handler on eligible infallible expressions. Default a bound error to builtin Error only when there is no declared/actual compatible source fixing another type.
- [ ] Preserve catch fallback constraints for the later inference cutover. Concrete result/fallback mismatch remains a source error.
- [ ] Validate unreachable handlers fully. Remove executable handler contributions only after the applicable semantic/failure convergence proves them unreachable.
- [ ] Extend existing unrouted-handler pruning to syntax-eligible casts/arithmetic/multiple calls without retaining bogus public effects, generic sidecars or backend requirements.
- [ ] Keep fallback failures outward, evaluation once, mutation-before-failure visibility, custom-error rejection and compound RHS scope.
- [ ] Preserve foreign float integrity guards as their own boundary rather than converting them to general implicit failure because catch is now syntactically eligible.

**Validation:** the applicable concrete-type cast/catch rows in Section 9, AST facts tests, HIR branch/lane tests and runtime output/error ordering on HTML-JS. Numeric-dependent fallback/source-inference rows remain explicit Phase 4/5 work, not skipped tests reported as passing. Pair supported Wasm cases with equivalent semantics and unsupported cases with their actual rejection contract.

**Exit:** one cast spelling, explicit source evidence pairs and stable authored recovery eligibility. Old numeric identities may still exist until cutover, but there is one current conversion/failure implementation and no compatibility syntax path.

### Phase 3 - prepare shared exact-width and external boundaries

**Primary owners:** lexical precision/value policy, canonical scalar projection, external registration/import projection, HIR external calls/numeric validation, JS numeric helpers and current Wasm validators.

#### 3A. Reuse fixed-width data and algorithms

- [ ] Characterise existing fixed scalar storage, integer extrema, float bit payloads and direct conversion algorithms before moving code.
- [ ] Consolidate reusable range/precision/materialisation facts into the current explicit-width owners where profile code currently contains shared algorithms.
- [ ] Preserve the U64-to-F32 direct-rounding witness and F16 decimal midpoint witnesses through both constant and runtime paths where supported.
- [ ] Retain Dec's separate coefficient/scale representation, Byte's nonnumeric classification and canonical text formatting.
- [ ] Avoid a replacement "default numeric profile" object. Exact type metadata supplies width and precision.

#### 3B. Generalise external finite validation

- [ ] Add regression fixtures for non-finite raw F64 external results before replacing NativeFloat signatures.
- [ ] Trace `result_type_ids_are_single_float`, `lower_validated_external_call_expression`, `emit_validated_float_value`, fallible carrier flags, `ValidateFloat` validation and runtime/helper consumers.
- [ ] Generalise selection and allocated result types to the resolved exact binary-float identity. Retain current-block lowering when validation can branch.
- [ ] Make destination precision explicit through the established semantic type or the smallest existing boundary descriptor. Delete hardcoded builtin Float allocations rather than merely changing the predicate.
- [ ] Validate direct calls, explicit return forwarding, catch recovery and fallible success extraction. Preserve arguments before result checking and validate no error payload as a success float.
- [ ] Test the existing fatal versus source-declared Error! integrity routes, including synthetic start. Keep catch leniency from changing them.
- [ ] Reuse the existing F64 external constant projection. Check non-finite constants through their compile-time diagnostic boundary, not a new Math path.
- [ ] Review F16/F32 external cases already supported by the active tree, preserving destination rounding and finite validation. Defer unsupported shapes honestly rather than inventing coverage.

**Validation:** fixed scalar unit tests, external-boundary HIR tests and real JS runtime tests with injected NaN, both infinities, finite values and finite values that overflow after destination rounding. Preserve supported-target gating and exact error-code distinctions.

**Exit:** changing a package from NativeFloat to F64 cannot bypass validation or allocate the wrong semantic result type. This evidence is required before the Math signature migration.

### Phase 4 - exact identities and scalar inference cutover

**Part of integrated cutover C. Primary owners:** canonical types/values, lexical source words, parser/declarations, pending recipes, coercion/operator policy and body finalisation.

#### 4A. Remove semantic identity duplication

- [ ] Remove Int/Uint/Float from builtin keys, canonical identities, environment seeding, diagnostic/type syntax, numeric scalar domains and value variants.
- [ ] Migrate all identity-dependent matches and constructor helpers to existing exact fixed-scalar representations. Keep full-width payloads and distinct Byte/Dec types.
- [ ] Update canonical public value transport, optional/container nesting, aliases and generated type keys. Never equate old process-local IDs with new identities by number.
- [ ] Remove old receiving promotion from Int/Uint to Float. Replace minimum-width within-operator promotion with D15 in the shared numeric policy. Preserve explicit cast policies and strict receiving compatibility.
- [ ] Remove old source-word/highlighter builtin entries and cast trait reservations. Removed spellings gain no implicit alias. Apply normal identifier/name rules rather than a special legacy parser.
- [ ] Review newly redundant explicit casts in compiler fixtures and packages. Removed convenience identities and D15 result changes can both make a cast same-type. Delete it only after proving conversion is no longer needed and preserving operand evaluation, narrow arithmetic failure and valid catch scope/eligibility. Source same-type casts remain invalid.

#### 4B. Integrate numeric construction

- [ ] Introduce eligible numeric origins at unannotated literals/bindings without materialising a default value prematurely.
- [ ] Connect copies, ordinary assignments and explicit/parameter/return/field constraints across the current body owner.
- [ ] Feed receiving context through raw arithmetic and parentheses before peer context and defaults. Stop at the agreed boundaries.
- [ ] Keep named-operand promotion directed forward. Add negative integration cases before allowing any new backwards edge.
- [ ] Resolve all permitted uses before defaults. Preserve family restrictions and conflict witnesses independently of source traversal order.
- [ ] Handle cycles/mutation without per-assignment type versions. Use explicit or initializer facts and normal defaults, not an optimisation range estimate.
- [ ] Materialise from retained spelling exactly once at the selected destination. Preserve signed-minimum, U64, float midpoint and Dec exact-leaf checks.
- [ ] Record complete numeric domain, operator and failure facts for downstream consumers. Required constant failures remain source diagnostics.

#### 4C. Apply the arithmetic result-domain contract

- [ ] Add failing policy tests for every fixed-integer pair, every binary-float pair and each supported unary/binary operator. Assert result types independently of particular values and cover the explicit division and mixed-family exceptions.
- [ ] Update `common_fixed_integer` to choose from all supported widths, including I8/I16/U8/U16, using complete input ranges. Remove the minimum-F32 rule from float domain selection and preserve signed/float operand types under unary negation. Reuse the existing policy owner rather than adding an inference-only or backend-only promotion table.
- [ ] Make literal recipes and constant evaluators complete every operation in the selected type. Accept typed narrow literal arithmetic when valid and diagnose a known narrow overflow at the authored operation. Preserve parentheses, required rounding boundaries and the cast-source barrier.
- [ ] Update `numeric_operation_cannot_fail` and all callers to use the revised operand/result domains. Dynamic U8 plus U8 is no longer proven safe from types alone. Retain positive proof coverage with a genuinely contained mixed domain such as I8 plus U8 producing I16.
- [ ] Distinguish required semantic failure classification from optional value-range check elision. The existence of a wider carrier or a previously accepted Wasm fixture is never proof of safety in the new result type.
- [ ] Implement F16 operation completion through the existing precision/checked numeric owners. Check finiteness after destination rounding and preserve signed zero/subnormals. Verify carrier sufficiency per operator without inventing a separate Math evaluator or weakening its existing contract.
- [ ] Cover narrow intermediate overflow before later cancellation, F16 per-operation rounding before a later wider cast and known-invalid work under catch. A wider receiver or final cast cannot repair earlier narrow work.
- [ ] Audit common-domain consumers in ranges, compound updates, failure summaries and generated functions. A same-type compound update performs one checked operation and no artificial narrowing, while mixed-domain write-back keeps its own checked conversion and commit boundary.

#### 4D. Complete scalar consumers

- [ ] Update expression evaluators, constant folding, compatibility caches, type display, diagnostics and source default policies together.
- [ ] Update compound operations to the revised arithmetic domains, preserving required checked store conversion, exact-once evaluation and commit-after-success. Delete newly redundant same-type store conversions without removing arithmetic checks.
- [ ] Complete deferred casts only after source resolution, using the already known destination. Do not make a missing target legal through later use.
- [ ] Preserve optional wrapping and result arity without importing the future native-result redesign.
- [ ] Add construction/finalisation assertions and release-safe validation for pending-state leaks. Authored source errors use ordinary diagnostics, not compiler bugs.

**Validation:** the numeric inference and arithmetic rows in Section 9.1, revised domain/failure witnesses in Section 9.4, existing literal/operator/constant/cast suites and paired constant/runtime cases. The rest of cutover C must be complete before declaring repository-wide success.

### Phase 5 - containers, generic signatures and handler constraints

**Part of integrated cutover C. Primary owners:** container/nominal construction, call validation, generic inference/materialisation, catch/value blocks and shorter compiler services.

#### 5A. Preserve known structure with numeric leaves

- [ ] Extend non-empty collection inference to carry numeric element unknowns without introducing general element unknowns.
- [ ] Select binary-float family for raw mixed whole/decimal literals independently of element order. Materialise whole leaves directly at the chosen precision.
- [ ] Reject bound-integer/float-family mixing without a cast. Retain exact element compatibility for already typed values and no arithmetic storage promotion.
- [ ] Carry numeric leaves through existing known map/nominal/optional/container relationships where the source shape is already legal.
- [ ] Retain map-key classification and delay duplicate comparison until keys are concretely materialised. Preserve exact U64 keys and type identity.
- [ ] Keep empty-container diagnostics and existing fixed-capacity/hidden-anonymous-record restrictions. Do not infer an absent general type from a later use.
- [ ] Let builtin get/set/push/remove signatures use the shared element relation while ordinary access/borrow rules stay unchanged.

#### 5B. Defer generic completion precisely

- [ ] Separate parsed call identity/routing from numeric-dependent signature completion in the existing generic call owner.
- [ ] Carry the same eligible numeric unknown across a declared generic parameter/result relationship, including known container shapes.
- [ ] Preserve the ordinary immediate expected-result rules for other generic inference. Numeric unknown support is not general result-only inference.
- [ ] Record already-concrete outer-parameter forwarding into nested generic calls as later generics work. Preserve currently rejected general nested/result-only cases here, alongside D13's accepted numeric-origin forwarding.
- [ ] Validate bounds and cast evidence after exact types are chosen. Neither evidence availability nor failure handling selects types.
- [ ] Record canonical concrete materialisation requests only after resolution. Deduplicate as before and keep owner-local inference IDs out of keys.
- [ ] Preserve recursion detection, same-file visibility, defaults, access modes, evaluation order and transaction rollback.
- [ ] Validate generic calls in unreachable handlers without publishing their inactive executable requests.

#### 5C. Resolve handlers and shorter services

- [ ] Include fallback result constraints before pruning an infallible protected expression. Preserve the receiver-versus-named-arithmetic restriction.
- [ ] Keep error-binding type independent of optional optimisation and enforce compatible typed/implicit errors.
- [ ] Complete named constants at their own boundary, including constants referenced by field defaults or capacities. Later runtime uses cannot retype them.
- [ ] Make config and direct-template service completion reject pending state and use the same exact materialisation/default rules.
- [ ] Preserve TIR ownership, overlays, nested views and constant folding. Delaying a numeric expression must not release its source/span owner too early or let pending values escape as template constants.
- [ ] Finish private summary convergence before eliminating unrouted handlers. Refresh rewritten HIR and affected analyses before publication.

**Validation:** aggregate/generic/recovery cases G01-G14 and H01-H24, existing generic/constant/TIR tests, public interface round trips and nested source/MON parity where relevant. Add source-level tests rather than relying only on fabricated type terms.

**Exit contribution:** all approved later-use cases work without a generic-body search, all excluded cases still fail for their intended reason and pending state ends at every service boundary.

### Phase 6 - builtins, package signatures and backends

**Part of integrated cutover C. Primary owners:** builtin collection/map/range operations, Core/Builder registrations, external ABI projection, HIR, JS and existing Wasm owners.

#### 6A. Builtin exact contracts

- [ ] Replace source-visible Int lengths/capacities/positions with I32. Preserve range counters' independent exact domains.
- [ ] Update checked host conversion for sizes, collection/map lengths, indices, allocation multiplication and JS array index conversion. Keep physical overflow separate from source index validity.
- [ ] Implement the exact I32 Dec exponent contract in typing, folding, runtime helpers and diagnostics.
- [ ] Migrate Char/code-point conversions and other old default-type builtin pairs to their intended exact replacement. Do not make every integer/Char pair implicitly legal.
- [ ] Preserve Error.code as U32, including zero and full unsigned range. It is not a default signed index.

#### 6B. Core Math and other external registrations

- [ ] Switch Math constants, parameters and results to F64 through the existing shared table and ABI projection.
- [ ] Preserve finite validation proved in Phase 3. Inspect the final emitted runtime path rather than assuming the earlier helper refactor is sufficient.
- [ ] Update Math fixtures using exact identities or justified approximation bounds. Keep arity/type errors and namespace/import paths.
- [ ] Migrate remaining NativeInt/NativeUint/NativeFloat registrations and their wrappers consistently. Preserve fixed foreign types that were already explicit.
- [ ] Check Core Random's old precision-selected helpers, Core Text length contracts and any time/builder numeric constants. Use the chosen exact public type while retaining their existing units/distribution/error contracts.
- [ ] Keep Math richer numeric/generic support and non-JS lowerings deferred in the living package notes.

#### 6C. HIR, JS and supported Wasm

- [ ] Replace profile numeric domains and payload cases with exact scalar facts across HIR construction, validation, dense stores, traversal, remapping and debug output.
- [ ] Preserve the already delivered dense HIR layout and freeze ownership. The construction-only inference substrate does not undo Phase 3's dense value/place work.
- [ ] Update semantic safety predicates and optional numeric proof queries to the revised exact operand/result domains. Remove obsolete profile stamping while keeping facts paired to the exact immutable executable and statement identities. Retire old full-domain-safe witnesses only with D15-linked replacement coverage.
- [ ] Recompute/invalidate proofs after any pre-publication rewrite. A missing fact retains checks. Optional proof never changes cast eligibility, source validity or error contracts.
- [ ] Update JS literals, BigInt/Number carriers, fixed float rounding, cast/runtime helpers, comparisons, formatting, map keys and helper demand.
- [ ] Make narrow integer helpers check the selected result width before exposing a value. Make F16 arithmetic finish at F16 at each operation through shared rounding/finite checks, including nested expressions and compound updates. Keep wider scratch carriers private and preserve error ordering.
- [ ] Preserve full I64/U64 comparisons and direct integer-to-float rounding without a Number detour. Mixed fixed integer/float source arithmetic still requires explicit conversion.
- [ ] Update Wasm scalar type/layout/carrier queries and supported numeric/cast/formatting paths to exact types. Preserve narrow loads/stores, signed reloads, F16 storage and Error code representation.
- [ ] Audit Wasm instruction/helper selection when D15 changes a result from I32/U32/F32 to a narrow semantic type. Retain existing physical carriers where appropriate but apply the selected-width checks/rounding before use. Respect existing recoverable-failure capability gates rather than implementing new failure machinery or silently reverting the result domain.
- [ ] Update HTML mixed-target request/glue inputs without expanding partitioning or reachable numeric support. Unsupported recoverable conversion/Dec/Math operations retain precise capability diagnostics.
- [ ] Keep unsupported-feature diagnostics at the relevant authored operation rather than at an unrelated new default-type conversion.

**Validation:** existing scalar JS/Wasm matrices, revised narrow operation/rounding tests, layout/emitter tests, numeric proof tests, package fixtures and explicit external-boundary negative runtime cases. A formerly accepted narrow operation may now require recoverable failure that Wasm does not support. Record that intentional D15 consequence and the correct capability diagnostic, then retain positive coverage of still-supported operations. Neither trap substitution nor a wider semantic result may be used to preserve a nominal success count.

### Phase 7 - configuration, MON and profile plumbing removal

**Part of integrated cutover C. Primary owners:** compiler build-input vocabulary, command parsing/bootstrap, standalone crates, services/options, module/package compatibility and fingerprints.

#### 7A. Exact build inputs

- [ ] Add a validated command numeric-literal carrier distinct from already typed primitive values. Use the shared lexical grammar and retain command locations.
- [ ] Discover the exact input contract before materialising that command value. Preserve bootstrap/source-only resolution order and duplicate/unknown input diagnostics.
- [ ] Admit all fixed integer/float contract types and existing optionals in the current implementation. Preserve exclusions for Byte, Dec and unsupported shapes.
- [ ] Require already typed inputs/globals to match the exact contract, with existing optional presence wrapping only.
- [ ] Update fallback normalisation, equality/conflict checks, source/programmatic origins and semantic fingerprints.
- [ ] Migrate CLI/check/dev/single-file/benchmark input paths without duplicating numeric parsing.
- [ ] Amend the queued general-config directive design and examples to consume these numeric facts. Implement no unrelated directive grammar or project bootstrap redesign.

#### 7B. MON clean break

- [ ] Remove old Value/SchemaType variants, prepared-profile nodes, Schema profile fields and profile-selection APIs/exports.
- [ ] Update decoder, encoder, defaults, map keys, schema validation and public examples to exact widths.
- [ ] Preserve Integer/Decimal lossless text, Byte schemas and all receiver-budget/error/ownership contracts.
- [ ] Migrate public consumer tests, doctests and compiler convenience re-export tests. Keep standalone crates independently testable.
- [ ] Refactor the real-source/MON parity harness to compare equivalent exact receiving types without profiles. Preserve expected typed values before comparing parsers.
- [ ] Classify old profile-cross-product tests: keep distinct precision/range assertions, remove only redundant profile-independence repetitions.
- [ ] Preserve existing source/MON syntax-gap tests. The numeric migration does not deliver empty-map syntax, Unicode escapes or contextual choices that remain unsupported at activation.

#### 7C. Remove semantic profile selection end to end

- [ ] Remove NumericProfile from builder selection, compiler options, config services, module inputs, generated sidecars, direct-template services, benchmark inputs and backend requests.
- [ ] Remove old profile-specific fingerprint/compatibility keys after exact type/value facts are authoritative.
- [ ] Reject or invalidate persisted old-format artifacts through existing version/compatibility policy where persistence is implemented. Where persistence remains deferred, update its accepted compatibility contract only. Add no cache/serialization framework, best-effort migration or alias-based repair.
- [ ] Preserve compiler language/artifact versions, package identities, used capabilities, physical ABI/layout policies and debug/release build profiles.
- [ ] Remove no-op parameters, default-profile constructors, exported aliases and dead helper branches. Keep reusable precision algorithms under exact-width ownership.
- [ ] Review feature-gated Boracle, developer dumps, first-party helper inventory, benchmark/xtask code and tests for compile-time coverage of removed variants.

**Validation:** package-scoped lexical/MON tests and doctests, config/CLI/origin suites, shared parity tests, module/interface/fingerprint tests and affected feature lanes. Run new direct-destination CLI cases, including values outside I32 and binary midpoint spellings.

### Phase 8 - migrate fixtures, harness and executable documentation sources

**Closes integrated cutover C.** Earlier phases update local tests as they change owners. This phase completes and audits the repository-wide migration rather than postponing all testing until now.

- [ ] Resolve every ledger entry for old numeric source names, profile settings, cast! syntax, revised narrow arithmetic and changed catch eligibility. Separate D15 changes in result type, rounding and failure from ordinary syntax migration.
- [ ] Migrate integration input sources and expectations together. Preserve the purpose, runtime data, failure order and diagnostic reason of each test.
- [ ] Remove the harness numeric_profile field, parsing rules, profile builder selection and tests only after equivalent explicit-width fixtures exist.
- [ ] Make old harness profile keys fail according to the fixture schema's unsupported-field policy rather than silently ignoring them.
- [ ] Preserve coverage that deliberately exercised nondefault widths, mixed signedness, finite guards, rounding and target rejections. Relabeling every source as I32/F64 is incorrect.
- [ ] Apply the minimum-width retirement protocol in Section 8.2. Keep dedicated tests of the new narrow semantics and preserve still-relevant wide-result/failure-analysis properties with explicit wider operands or another valid witness. Never automatically add casts just to keep an old success expectation.
- [ ] Update compiler-generated Moth snippets, Rust test strings, generators, project scaffolds, first-party source packages, benchmark projects and docs' executable bindings.
- [ ] Update highlighter vocabulary and tests through the compiler-owned word classification where already shared. Preserve ordinary host-language `int`/`float` tokens in other language profiles.
- [ ] Run the suite inventory audit again and compare assertion kinds, semantic contracts, backend blocks and diagnostic coverage with the baseline ledger.
- [ ] Check every selected test filter runs the intended tests. A successful command with zero selected tests is not validation.
- [ ] Run `just validate` and affected standalone/feature/target checks on the complete cutover tree. Investigate failures by semantic cause before changing expected output.

**Exit:** the repository builds and the integrated supported paths agree on exact types, defaults, casts, catch, config and MON. No supported old source spelling or profile path remains. All removed tests have reviewed coverage dispositions.

### Phase 9 - adversarial and publication hardening

**Consumes:** the integrated exact-width compiler and complete migration ledger.

- [ ] Complete every acceptance row in Section 9 with one primary test owner and justified secondary stage/backend coverage.
- [ ] Add deterministic constraint-order, long-chain, fan-out and cyclic numeric dependency tests. Assert bounded work through counters/invariants where appropriate, not machine-speed unit-test thresholds.
- [ ] Check syntax/type errors inside unreachable handlers and ordinary inactive branches. Verify none are hidden by premature folding or handler erasure.
- [ ] Check handler erasure leaves no live capture, materialisation request, runtime helper, external capability or public failure effect. Preserve graph-active source validity separately.
- [ ] Test private call-summary convergence and recursive calls, builtin Error delivery, custom-error mapping and catch continuation routing. Validate all rewritten HIR before analysis/publication.
- [ ] Test cross-file/module/generic constants and canonical type equality without donor-local IDs or removed default-type aliases.
- [ ] Exercise overload-free signature lookup and conflicts where a tempting trait or cast choice must not select the type.
- [ ] Compare compile-time and runtime operation boundaries, including Dec and F16 per-operation rounding, same-type narrow overflow, signed minimum negation/division, unsigned underflow and mixed-domain compound write-back. Check cancellation expressions that would hide a required intermediate failure or rounding if widened or reassociated.
- [ ] Exercise development/release parity for source validity and checked results under the revised arithmetic contract. Existing optional optimisations must not change accepted syntax, required recovery or target rejection. Classify D15-caused changes from the old baseline separately from build-mode differences.
- [ ] Check finite external F64 values before storage, formatting, comparison, forwarding and nested success extraction. Inject invalid values without relying on constant folding to hide the call.
- [ ] Run the affected borrow/Boracle mechanical lanes and verify the existing reference rules are unchanged. This does not resume research campaigns or change their design.
- [ ] Apply the code review guide to all touched owners. Remove duplicate inference maps, compatibility leftovers, unused wrappers, repeated traversals and test-only production hooks.

**Exit:** the new semantics have explicit positive and negative boundaries, unchanged runtime contracts retain coverage, D15-caused matrix changes have reviewed outcomes and no pending state or stale analysis reaches a published artifact.

### Phase 10 - performance, documentation and full validation

- [ ] Execute the matched performance protocol in Section 11 on the final source cohort. Separate removed-language work from improvements in representation or algorithms.
- [ ] Investigate repeated scans, per-literal allocations, oversized pending records, delayed-constant retention and generic rework when measurements regress.
- [ ] Finish all permanent documentation, teaching examples, navigation, index and progress updates from Section 10.
- [ ] Build generated docs only through the actual compiler and inspect the generated diff. Do not manually patch release HTML or commit benchmark/test output directories.
- [ ] Update the retained benchmark report with reproducible evidence and limits, preserving historical data. Record numerical fixture adaptations used for comparisons.
- [ ] Run the repository's final full gate and any required affected lanes outside it on the exact final tree under the pinned tools.
- [ ] Recheck the coverage ledger against the final test inventory, not an earlier intermediate tree. Record the actual revision/configuration for each validation result.
- [ ] Review the final diff by owner and by language contract. Resolve all regressions or keep the plan explicitly incomplete with the actual blocker. No suppressed failures or unsupported success claims.

**Exit:** documented exact-width semantics, measured cost, complete regression evidence and passing main-bound gates on the final integrated tree.

### Phase 11 - restart reassessment and completion handoff

The numeric plan completes a dependency of the expression refactor. It does not automatically authorise expression implementation to resume.

- [ ] Refresh the expression plan's owner inventory against the delivered numeric construction, types, literal recipes, cast/failure facts and generic call completion.
- [ ] Reevaluate completed Phase 0 census/baseline and Phase 2 representation/lifetime conclusions affected by the new pending state. Preserve valid conclusions with evidence and mark superseded ones explicitly.
- [ ] Revalidate the accepted dense HIR checkpoint, including source spans, scalar payloads, side stores, remapping, stack bounds, freeze barriers and generated sidecars.
- [ ] Reassess how the future compact semantic expression representation consumes numeric construction output. Keep completed nodes fully typed and avoid migrating construction-only facts into permanent semantic payloads.
- [ ] Reassess default/constant/TIR lifetime overlap, generic request timing, safety-catch metadata, deferred private failures and unreachable-handler reclamation.
- [ ] Take a fresh post-numeric baseline. Prior measured node sizes, clone counts and performance conclusions are historical until retested on the new compiler and comparable source cohort.
- [ ] Rewrite the remaining expression Phase 4 onward work packages around delivered capabilities. Remove numeric obligations now completed here and retain unrelated native result/Core evaluation/Wiring work.
- [ ] Keep the restart status as awaiting explicit resumption after reassessment, with concrete remaining actions. Do not silently advance to Phase 4.
- [ ] Update the downstream consumers listed in Section 10, especially the mixed-Wasm and constraints plans and the living Core Math plan.
- [ ] Hand the revised operator-domain table and generic exceptions to the later generics/Core binding plan. Record explicit closed bounds and already-concrete nested expected-type forwarding there without treating either as delivered by this plan.
- [ ] Publish all unique accepted contracts and remaining follow-ups in permanent or appropriate living owners.
- [ ] Record the final activation-to-completion coverage/performance/validation summary. Retire this one-shot plan and its roadmap bullet in the implementation completion commit under repository policy.

**Exit:** the numeric refactor is complete, the expression restart has a current evidence-based plan and every deferred capability has an explicit owner rather than an accidental dependency cycle.

## 8. Coverage migration protocol

### 8.1 Preserve assertions, not just fixture names

Before changing a test, identify the behaviour that would become wrong if its assertion disappeared. Record that contract even when the fixture contains obsolete syntax.

Maintain a working ledger under `tmp/exact-numerics/coverage-ledger.md`. It must include unit tests, public-crate tests, doctests, generated suites and integration fixtures, not only `tests/cases`.

| Field | Required content |
|---|---|
| Original owner | File and test/fixture ID, with the baseline revision |
| Original context | Profile, exact input types, backend, build mode and runtime/constant path |
| Assertion | Concrete mathematical value, bit pattern, diagnostic, ordering or artifact property |
| Disposition | Preserve, adapt, consolidate, deliberately retire or existing unsupported capability |
| Replacement | Exact new test owner/ID and explicit types |
| Rationale | Why the assertion still holds or which approved semantic decision removes it |
| Evidence | Commands, executed test count, final revision and observed outcome |

A fixture may split into several owners when it combined profile selection with useful fixed-width arithmetic. Several old profile fixtures may consolidate only when their surviving assertions are genuinely identical. Keep different widths, paths, diagnostics and backends distinct where they protect different contracts.

Before deleting an original test, prove that each surviving assertion has a replacement. A temporarily absent test is an open migration item, not an acceptable final coverage reduction.

### 8.2 Profile-to-width mapping

| Old selected profile | Integer replacement | Unsigned replacement | Float replacement |
|---|---|---|---|
| Int32/Float32 | I32 | U32 | F32 |
| Int32/Float64 | I32 | U32 | F64 |
| Int64/Float32 | I64 | U64 | F32 |
| Int64/Float64 | I64 | U64 | F64 |

This maps the old test's semantic domain, not every occurrence in the program. A foreign ABI already naming F64 remains F64. A Core Math API now deliberately becomes F64 in every case. A test of removed implicit integer-to-Float receiving conversion splits into an explicit-cast numerical test and a negative test of the newly strict receiving boundary where both properties matter.

Do not preserve obsolete assertions that Int and I32 are distinct identities by inventing a substitute alias. Retire that assertion with its removed-feature rationale, while retaining ordinary exact type identity tests.

#### Minimum-width arithmetic retirement

D15 also changes existing fixed-width programs, independently of profile removal. Give each affected assertion one explicit disposition before deleting or rewriting it:

| Old property | Replacement obligation |
|---|---|
| I8/I16/U8/U16 arithmetic yields at least 32 bits | Assert the revised same-type or smallest mixed-width result and its exact narrow failure boundaries. |
| Narrow operands implicitly produce a wide result accepted by a receiver | Add a negative exact-receiver test, then retain any meaningful wide calculation using explicit operand conversion before the operation. A cast after the operation is not equivalent. |
| F16 arithmetic produces F32 and rounds only at that domain | Assert F16 completion per operation and keep independently typed F32 tests for the old wider numerical properties. |
| U8 plus U8 is full-domain infallible | Assert its new potential failure, then preserve proof/discharge coverage with a safe mixed-domain witness such as I8 plus U8 producing I16. |
| A Wasm success depends on the old wide-domain proof | Record its new source or capability outcome. Keep a separate genuine supported-path witness rather than introducing traps or an old-width fallback. |
| Compound assignment has a promoted operation followed by a narrowing failure | Keep distinct tests for same-type arithmetic failure and mixed-domain write-back failure, preserving no-commit and evaluation-order assertions. |

Consolidating obsolete minimum-width fixtures is allowed only after those replacement obligations have owners. Preserve all unrelated assertions within each fixture. Explicit widening can change the available static proof, so a numerically equivalent rewrite is not automatically an equivalent capability or proof test. Record those distinctions rather than weakening the expected diagnostic.

### 8.3 Existing high-value migration owners

| Existing owner or family | Migration obligation |
|---|---|
| `tests/cases/core_math_basic_functions` | Preserve function semantics, radians, exact selection/rounding results and justified approximation bounds under F64 |
| `tests/cases/core_math_transcendental_functions` | Preserve every function exercised and distinguish target-specific fixed points from generally approximate results |
| `tests/cases/core_math_constants` | Preserve all six constants and nearest-F64 expectations |
| `tests/cases/core_math_external_constants_const_context` | Preserve constant-context projection and absence of runtime lookup |
| `tests/cases/core_math_float_boundary_validation` | Preserve non-finite rejection, correct delivery and original Error codes |
| `tests/cases/core_math_type_errors` and `core_math_arity_error` | Preserve the intended wrong-type/arity failure, with updated exact expected type |
| `tests/cases/core_inline_expression_lowering` | Preserve inline lowering and shared external boundary behaviour |
| `tests/cases/core_math_direct_selection_dependency` and `namespace_binding_external_package_success` | Preserve direct/namespace import identity and constant use |
| `tests/cases/core_math_profile_int64_float32` and other Math profile cases | Separate new F64 Math expectations from reusable explicit F32 boundary coverage |
| `tests/cases/numeric_profile_float32_runtime` and other numeric profile cases | Preserve destination-precision operations and runtime checks using explicit F32 |
| `tests/cases/uint_wasm_scalar_int64_float32` and sibling profiles | Preserve unsigned Wasm carrier, exact comparisons, formatting and supported conversions as U64/F32 |
| `tests/cases/uint_js_execution_int64_float32_success` and `_release` | Preserve full unsigned range, JS BigInt path, rounding and build-mode parity |
| `tests/cases/uint_checked_arithmetic_int64_success` and `_release` | Preserve dynamic arithmetic/underflow/recovery and the direct-rounding witness |
| `tests/cases/uint_template_const_range` | Preserve const range results and exact counter domain without a profile |
| Numeric literal, Dec, cast, range, option, map and collection fixture families | Inventory assertion by assertion, including old implicit conversions and same-identity changes |
| `src/compiler_frontend/type_coercion/tests/contextual_tests.rs` | Move useful rounding evidence out of removed implicit-coercion tests, retain rejection of new illegal coercions |
| `src/backends/js/tests/uint_runtime.rs` | Reuse direct BigInt-to-float and unsigned helpers under exact U64 ownership |
| `src/compiler_frontend/hir/tests/float_formatting_lowering_tests.rs` | Preserve ValidateFloat/FormatFloat precision, result types and delivery distinctions |
| `src/compiler_frontend/hir/tests/checked_numeric_lowering_tests.rs` | Assert revised domains and checked edges, replace obsolete wide-domain proof witnesses and retain distinct compound arithmetic/write-back failures |
| `crates/moth-mon/src/tests/{schema_tests.rs,uint_tests.rs}` | Migrate schema/default/range assertions to exact widths before deleting profile paths |
| `crates/moth-mon/tests/` and crate doctests | Preserve public Rust consumer coverage and clean API usage |
| `src/compiler_tests/mon_syntax_parity/float_rounding.rs` | Preserve exact literal midpoint, subnormal, signed-zero and finite-limit expectations in both parsers |
| Config/CLI/origin and module-interface suites | Preserve input provenance, exact type identity, defaults, conflicts and complete publication |

Refresh this list against the activation tree. Preserve newly added relevant fixtures rather than limiting migration to the names discovered during planning.

The historical numeric boundary audits also identify range cases worth retaining: an inclusive maximum singleton must not calculate an overflowing successor, and an exclusive equal-bound float range is empty rather than a false nonprogress error. Migrate the real regression tests to explicit widths and keep audit history factual.

### 8.4 Test migration rules

Preserve exact diagnostic codes and typed reasons. When the semantic error changes intentionally, change the expected code with a decision-linked explanation. Keep source-span and secondary-label assertions that protect useful diagnostics. Do not reduce a precise negative test to generic compilation failure.

Keep runtime tests genuinely runtime. A replacement constant expression that folds away no longer tests emitted overflow, narrowing, finite validation or failure ordering. Use the existing runtime fixture mechanisms and artifact checks to prove the target operation survives.

Preserve exact-once and source-order assertions. A numerically correct final output can conceal duplicated calls, dropped effects, eager fallback evaluation or incorrectly committed mutations.

Preserve meaningful backend distinctions. A compiler target rejection, a recoverable Error, an entry error and a runtime trap are different outcomes. Do not change one to another to satisfy a migrated fixture. Keep math support and foreign integrity validation separate from numeric operator support.

Keep exact bit tests where decimal rendering loses information. Signed zero, F16/F32 midpoints and direct U64 rounding need bit-level assertions in an existing Rust/JS/ABI test owner as well as rendered output where formatting itself is contractual.

Follow the repository's manifest and harness contracts. Add no acceptance-only/smoke fixture as a substitute for semantic assertions. Avoid strict output goldens unless exact generated text is itself the intended contract. Keep HIR assertions relational rather than tied to incidental local/block IDs.

Compiler and runtime correctness tests remain distinct from benchmarks. A migrated benchmark that builds is not regression coverage.

## 9. Mandatory acceptance matrix

The rows specify contracts, not one new fixture per row. Group related assertions in a clear primary owner and use secondary unit/backend tests only when they protect a different boundary. Every row must have an owner and outcome in the final coverage ledger.

Retain the existing lexical/spacing negative suites alongside this matrix. Numeric migration does not legalise uppercase exponents, unsupported radix/suffix syntax, malformed separators or changed operator spacing.

Examples using helper names assume concrete signatures indicated by those names. Implement actual helpers with the current Moth syntax and normal visibility/error contracts. They are not new builtins.

### 9.1 Numeric inference and arithmetic boundaries

| ID | Case | Required result |
|---|---|---|
| N01 | Unconstrained whole and decimal/exponent bindings | I32 and F64 respectively, not size-selected narrow types |
| N02 | Whole literal larger than I32 without a receiver | Targeted default-range diagnostic, no retry as I64/U64/Dec |
| N03 | Same literal with later direct U64 receiver | Exact U64 materialisation without an I32 intermediate |
| N04 | `v = 1`, cast to U32 first, later uncast I64 use | Source I64, deferred I64-to-U32 policy |
| N05 | Same source used only through casts | Default I32 independently of cast targets |
| N06 | Conflicting exact U32 and I64 receivers | Deterministic conflict, no inserted conversion or first-use winner |
| N07 | Copy chain constrained at the last binding | Same exact numeric type reaches all connected copies |
| N08 | Mutable binding with multiple writes and branches | One static type, every write checked against it |
| N09 | `v = 1 + 2` with later U64 use | Raw recipe evaluates in the resulting legal U64 domain |
| N10 | `base = 1`, `twice = base + base`, later U64 use | Rejected without an independent base constraint |
| N11 | Add an independent U64 use of base to N10 | Base resolves U64, forward promotion succeeds regardless of use order |
| N12 | I32 base plus raw `1` at I64 receiver | Literal I64, result I64, base remains I32 |
| N13 | Two concrete I32 operands at I64 receiver | Rejected, no result retagging |
| N14 | U8 literal arithmetic at a U8 receiver | `1 + 2` materialises and evaluates as U8 without narrowing, while known `255 + 1` is a numeric diagnostic |
| N15 | Two concrete F16 operands versus an F32 receiver | Operation remains F16 and the exact receiver rejects it. An explicit result cast preserves prior F16 rounding, while widening operands requests F32 arithmetic |
| N16 | Bound whole integer sent to float or Dec receiver | Requires explicit conversion, no family reclassification |
| N17 | Immediate whole literal at F32/Dec/Byte receiver | Direct valid materialisation, no intermediate integer coercion |
| N18 | Parentheses, reversed peers and a literal outside a narrow peer range | Preserve receiver priority and the peer-range diagnostic, with an explicit wider receiver able to type raw leaves independently |
| N19 | Concrete integer real division versus integer division | F64 real-division result, existing truncating integer-division semantics |
| N20 | Comparisons with I64/U64 extrema | Exact comparisons without a float detour or subtraction overflow |
| N21 | Mutable self-update and unresolved numeric dependency cycle | Bounded resolution or annotation diagnostic, never a hang or backward search |
| N22 | Known constant or concrete function result | Later use cannot change its declared/completed type |
| N23 | Use in template formatting before an exact later scalar use | No accidental early default from a non-constraining formatting consumer |
| N24 | Unsupported method or trait on an unresolved numeric receiver | Resolve independently, then diagnose, no type search to find an implementation |
| N25 | Every eligible equal-type integer/float arithmetic pair | Result keeps the operand type, including I8/I16/U8/U16/F16, with explicit `/` and unsupported-operator exceptions |
| N26 | Mixed fixed-integer and float type pairs in both orders | Smallest supported common operand domain without 32-bit/F32 floors. No I64/U64 arithmetic fallback |
| N27 | Unary signed/float negation, unsigned negation and Byte arithmetic | Signed/float result type preserved, signed minimum checked, unsigned negation and Byte arithmetic rejected |
| N28 | Concrete I8 arithmetic under an I32 receiver or later I32 use | Exact mismatch, no hidden widening. Wider operand conversion works and a result cast cannot repair earlier narrow failure |
| N29 | F16 receiving context for raw literal arithmetic versus a named F16 operand and raw F32-context literal | Selected domain follows D12/D15. The raw-only F16 operation stays F16, while the explicit wider literal context can select F32 without retyping the named operand |

### 9.2 Containers, generics and service boundaries

| ID | Case | Required result |
|---|---|---|
| G01 | Non-empty whole collection with no exact consumer | Collection of I32 after fallback |
| G02 | Same collection with exact later {U64} receiver | Collection of U64 without per-element conversions |
| G03 | `{1, 2.5}` and `{2.5, 1}`, later {F32} receiver | Same F32 element type, direct literal materialisation |
| G04 | Empty collection/map without an explicitly permitted type | Existing empty-container diagnostic, no later-use shape inference |
| G05 | Bound integer mixed with raw decimal collection element | Family mismatch rather than implicit conversion |
| G06 | Already typed I32 and U64 elements | No arithmetic common-storage-type selection |
| G07 | Known nested container/map/nominal numeric leaves | Resolve eligible leaves while preserving exact existing shape/identity |
| G08 | Collection read/write constraints with mutable access | Same element type, ordinary access checks and no type changes over time |
| G09 | Signature-only identity generic called with a literal | Later exact result use resolves the shared numeric argument and one concrete instance |
| G10 | Generic result with concrete return type or no numeric origin | Preserve existing generic rules, no general later-result inference |
| G11 | Generic bound mismatch after numeric resolution | Useful call-site evidence diagnostic, no alternate numeric type search |
| G12 | Recursive/imported/re-exported generic materialisation | Stable concrete request keys, deduplication and existing recursion policy |
| G13 | Named constant, field default, capacity and direct-template service | Types complete at their proper boundaries, no escaping inference state |
| G14 | Failed body or generic transaction | No partial interface, pending type or leaked generated sidecar publication |

### 9.3 Casts and safety recovery

| ID | Case | Required result |
|---|---|---|
| C01 | Full-range contained integer conversion | Plain cast with infallible policy and correct value |
| C02 | Signed-to-unsigned and wide-to-narrow conversions | Plain cast with correct potential failure and existing error delivery |
| C03 | Known safe value for a fallible type pair | May fold/remove its check, without changing pair policy |
| C04 | Known invalid conversion under catch | Compile-time numeric diagnostic, not a fallback value |
| C05 | Cast without immediate target followed by a typed later use | Rejected, target is not inferred from that later use |
| C06 | Same-type, unsupported and optional-source casts | Existing appropriate rejection after source resolution |
| C07 | Removed cast! syntax | Rejected without accidentally becoming generic postfix propagation |
| C08 | Private helper, builtin Error! export and custom E! context | Correct inferred delivery, explicit public contract and required mapping/recovery |
| C09 | Source-authored fixed numeric cast evidence | Correct direct method call, exact target and declared fallibility |
| C10 | Both evidence classes for one pair or missing conformance | Deterministic conflict/no-evidence diagnostic |
| C11 | Same-file, visibility and generic nominal evidence | Existing evidence boundaries preserved, no structural method-shape fallback |
| C12 | Float-to-integer, float narrowing and integer-to-float | Existing truncation, finite-range and direct rounding contracts |
| C13 | Dec scale narrowing and integer/Dec conversion | Existing exact-or-fail rules and per-operation scale semantics |
| C14 | Optional target and fallback | Recover inner T then wrap, no implicit optional source unwrap or `then none` escape |

| ID | Case | Required result |
|---|---|---|
| H01 | Infallible supported cast with catch | Allowed and normally validated |
| H02 | Infallible arithmetic or unary numeric negation with catch | Allowed by authored shape |
| H03 | Two receiver-chain calls | Allowed even when both are infallible |
| H04 | Two calls in separate comparison operands | Allowed even when both are infallible |
| H05 | One infallible outer call around an infallible argument call | Rejected by the multi-call rule |
| H06 | Same shape with a recoverably fallible argument | Allowed, nested failure protected despite excluded call counting |
| H07 | Plain literal, binding/constant reference, value comparison or one infallible call | Rejected without another qualifying producer, even when a referenced initializer originally had arithmetic |
| H08 | Transparent parentheses versus separate argument levels | Grouping preserves selected-expression count, argument calls stay nested |
| H09 | Constructor and compiler-inserted helper calls | Do not manufacture function-count eligibility |
| H10 | Arithmetic/cast inside a protected argument expression | Qualification/protection follows authored evaluation, call-count exclusion stays separate |
| H11 | Folded cast/arithmetic or later type change | Eligibility retained, no new pointless-catch diagnostic |
| H12 | Fallback U64 constraining pending `1 + 2` | Infer U64 before handler erasure |
| H13 | U64 fallback against a concrete I32 operation | Type mismatch even when fallback cannot execute |
| H14 | Bound error on a safety-only handler | Error type, without adding a real builtin failure effect |
| H15 | Custom E producer plus semantically infallible arithmetic | Preserve compatible E handler, no fabricated Error conflict |
| H16 | Actual implicit Error mixed with incompatible custom E | Existing compatibility rejection |
| H17 | Unreachable handler containing invalid names, casts or constants | Normal diagnostics still emitted |
| H18 | Handler arity, terminality, optional target and placement | Existing restrictions remain independent of eligibility |
| H19 | Protected work/fallback mutations and nested failures | Exactly once, first failure, no rollback and fallback failure goes outward |
| H20 | Nested explicit catch or compiler helper inside an expression | Separate handler ownership, no call-count inflation from hidden work |
| H21 | Compound assignment with RHS catch | RHS only protected, later arithmetic/store checks remain and store commits on success |
| H22 | Fully unreachable fallback referencing runtime-only features | No emitted runtime work/helper/target gate after validated semantic elimination |
| H23 | Handler enclosing a private call whose summary resolves later | Preserve provisional work, prune only after convergence and revalidate HIR |
| H24 | Fatal foreign integrity guard or assertion under safety catch | Existing fatal contract remains, no invented recoverable failure |

### 9.4 Numerical witnesses and backend parity

| ID | Case | Required result |
|---|---|---|
| B01 | Every signed minimum and unsigned maximum | Correct literal materialisation and runtime transport, including full U64 |
| B02 | Integer overflow/underflow, zero division and invalid exponent | Existing checked failure code and first-failure order |
| B03 | Signed MIN integer division `//` by -1 versus remainder `%` by -1 | Integer quotient overflow preserved, remainder zero preserved. Real division `/` retains its F64 result |
| B04 | Same-type U8/U16 updates and mixed-domain narrow write-back | Check arithmetic in the revised selected domain and convert only where needed. No wrapping mask, duplicate narrow conversion or partial commit |
| B05 | U64 9007199791611905 to F32 | Exact destination bits 0x5a000001, never the f64-detour result |
| B06 | F16 midpoint, just-above-midpoint and finite overflow threshold | Direct destination rounding from original spelling |
| B07 | F16/F32/F64 subnormals and signed zero | Preserve accepted bit semantics through literals, constants, storage and MON |
| B08 | Dec `1 / 3 * 3`, power versus repeated multiply and exact leaf fitting | Existing per-operation rounding and scale rules, no whole-expression rational folding |
| B09 | Decimal exponent literal/group and typed wrong-width exponent | I32 exponent context, explicit conversion required for other concrete widths |
| B10 | Inclusive numeric maximum singleton and equal exclusive range | Terminate without overflowing successor, empty range not false nonprogress |
| B11 | Floating range no-progress and out-of-range next candidate | Existing guards and endpoint exceptions in each already-supported exact counter domain. Audit F16 shared arithmetic completion without a hidden F32 semantic counter |
| B12 | I32 container positions with large physical byte products | Source limit checked separately from address/allocation arithmetic |
| B13 | JS BigInt and Number carrier boundaries | Exact I64/U64 values and comparisons, no accidental coercion |
| B14 | Supported Wasm scalar memory and numeric paths | Existing signed/unsigned load/store, carrier and runtime contracts preserved |
| B15 | Unsupported Wasm recoverable cast, Dec or Math call | Relevant capability diagnostic, never a trap or silent backend switch |
| B16 | Optional numeric proof present/absent and pre-publication HIR rewrite | Same source validity and semantics, correctly paired proof or retained check |
| B17 | Debug/release builds of all critical conversions and failure cases | Same accepted types, values and failure ordering |
| B18 | Runtime I8 127 plus 1, U8 255 plus 1, U8 zero minus 1 and narrow multiplication overflow | Existing checked failure delivery in the selected narrow domain, never implicit widening or wrapping |
| B19 | Signed narrow minimum negation and integer division `//` by -1, paired remainder | Negation/integer division fail at the narrow width, remainder remains zero and known invalid work diagnoses at compile time |
| B20 | Concrete F16 `(2048 + 1) - 2048` and F16 maximum plus itself | First expression produces zero after per-operation rounding. Second fails at F16 finite overflow. An F32 carrier changes neither outcome |
| B21 | I8 `(a + b) - b` with a=127 and b=1 supplied at runtime | Addition fails before subtraction, even though a wider or reassociated calculation would fit at the end |
| B22 | Full-domain failure discharge for U8+U8 versus I8+U8 | U8 result remains potentially fallible, I16 mixed result is proven contained. Safety-catch eligibility stays independent of either fact |
| B23 | Former narrow Wasm success whose wide-domain proof disappears | Correct updated source/capability result and a separate still-supported positive witness. No added recoverable Wasm machinery or trap fallback |
| B24 | Fixed binary-float operator completion in nested expressions, returns and compound updates | Selected precision completes each operation before use, including F16 finite/subnormal/zero cases, without forcing stores to establish semantics |

### 9.5 Config, MON and external Math

| ID | Case | Required result |
|---|---|---|
| M01 | CLI U64 value outside I32 and I64 ranges | Direct accepted U64 materialisation when contract allows it |
| M02 | CLI whole spelling at F32 versus decimal spelling at integer | Source literal category rules, no early I32/F64 intermediate |
| M03 | CLI rounded overflow, midpoint, negative unsigned and malformed quotes | Correct numeric or quote diagnostic with command provenance |
| M04 | Quoted numeric String, bare none and omitted optional input | Existing explicit-string and optional absence semantics |
| M05 | Already typed F64/I64 input sent to another exact contract | Exact mismatch, no reparsing or implicit numeric conversion |
| M06 | Repeated contracts, defaults, builder globals and source-only input | Existing precedence/conflict/provenance and fingerprint contracts |
| M07 | Standalone explicit MON schema values/defaults | Typed round trip after profile API removal |
| M08 | Former profile32 and profile64 MON cases | Correct distinct exact replacements, not all language defaults |
| M09 | MON Integer/Decimal, Byte, optional/default and map data | Existing losslessness, schema exactness and map identity preserved |
| M10 | Source/MON shared rounding/parsing parity | Both independent parsers agree at the same exact destination |
| M11 | MON limits, errors, spans and owned results | Existing bounded behaviour and public error API preserved |
| M12 | Unsupported source/MON parity gaps | Continue reporting actual gaps, no accidental syntax expansion |
| M13 | Removed MON profile APIs and integration profile keys | Clean API/schema break, no silently ignored compatibility setting |

| ID | Case | Required result |
|---|---|---|
| A01 | All existing Core Math function and constant fixtures | F64 contracts with original mathematical assertion quality |
| A02 | Math constants in const contexts and through namespace/import aliases | Existing F64 projection, no runtime lookup or Math-specific constant path |
| A03 | Direct/aliased/forwarded external F64 returns of NaN and infinities | Rejected before ordinary Moth observation through existing integrity policy |
| A04 | Fallible external call with finite or non-finite success payload | Validate success only, preserve actual returned Error payload |
| A05 | Existing F32/F16 external destinations including rounded overflow | Correct precision-specific boundary without profile dependence |
| A06 | External call in synthetic start versus declared builtin Error! function | Preserve the real fatal/recoverable boundary distinction |
| A07 | Math wrong type/arity and unsupported Wasm lowering | Original semantic error and capability distinction retained |
| A08 | Former Float32 Math profile expectations | F64 Math checked separately from retained explicit F32 boundary tests |
| A09 | Other migrated NativeFloat/NativeInt/NativeUint registrations | Exact public type, existing units/distribution/formatting and validated boundary |
| A10 | Finite guard and required call/helper demand after folding/pruning | No lost necessary guard and no retained dead helper demand |

### 9.6 Publication, determinism and reuse

| ID | Case | Required result |
|---|---|---|
| X01 | Module public constants/types and generic requests | Exact canonical identity, no inference-local or donor-local IDs |
| X02 | Failed source or generated transaction | No partial semantic publication |
| X03 | Different diagnostic traversal or legal compilation order | Same inferred types and stable useful conflict witnesses |
| X04 | Deleted profile compatibility dimension | Exact facts and format/version checks prevent stale artifact reuse |
| X05 | Debug/release and JS/Wasm physical variants | Shared semantic types, separate physical/layout/capability policies |
| X06 | Handler pruning after private-lane convergence | Validated final HIR and recomputed affected summaries/proofs/link facts |
| X07 | Large mostly-folded or generic-heavy bodies | Pending numeric storage is released or bounded, no retained per-use clones |
| X08 | Paused expression-foundation checkpoint | Dense HIR, span ownership, stack checks and ordinary template behaviour preserved |

## 10. Required documentation and roadmap edits

### 10.1 Permanent semantic owners

Each entry needs explicit review, even when the final disposition is "already accurate". Update canonical references before relying on implementation behaviour as the specification. Update teaching text and examples without replacing precise contracts with vague shorthand.

| Owner | Required content change |
|---|---|
| `docs/compiler-design-overview.md` | Replace numeric-profile invariants with exact types and fixed defaults. Define construction-only inference scope, literal/receiver priority, generic/cast completion, failure separation and no-pending-state handoff. Update public values, proofs, module inputs, config/MON and external boundary text. |
| `docs/build-system-design.md` | Remove semantic profile selection/propagation from bootstrap, services, dependency compatibility and variants. Specify exact input materialisation and preserve build/physical profiles and existing ownership. |
| `docs/compiler-data-layout-design.md` | Remove MON/profile-dependent carriers and describe relevant pending numeric/source-span lifetimes. Preserve packed spans, existing diagnostics and independent MON error ownership. |
| `docs/src/docs/numbers/` | Exact inventory, defaults, bounded inference, raw/bound family distinction, receiving priority, D15 result-domain matrix, narrow checked failures, per-operation F16 rounding, I32 Dec exponent and compile/runtime parity. Update both Basic and unsuffixed references and their page composition. |
| `docs/src/docs/casts/` | One cast spelling, exact immediate target, source inference barrier, post-resolution policy, ordinary failure delivery, expanded evidence traits and preserved conversion exactness. |
| `docs/src/docs/errors/` | Authored safety eligibility, call-count levels, handler constraints/error types, unchanged placement, expression-wide protection, failure compatibility and pruning semantics. |
| `docs/src/docs/collections/` | Non-empty numeric leaf inference, mixed-literal float family, exact typed element compatibility, unchanged empty-container rules and I32 positional/length APIs. |
| `docs/src/docs/loops/` | Exact range domains, contextual omitted values, I32 position index, endpoints, no-progress and correct fixed-width examples. |
| Generic, function, binding, constant, struct and choice references | Signature-only numeric unknown forwarding, concrete public boundaries, declaration-owned constants, fields/defaults and no general later-use inference. |
| `docs/src/docs/traits/core-cast-traits.mtf` and cast evidence reference | Fixed-target method/trait inventory, signatures, conformance/visibility and mutual exclusion, with no old Int/Float aliases. |
| `docs/src/docs/project-structure/` | Exact numeric build inputs and raw versus typed input distinction. Preserve the queued directive architecture and its separate implementation status. |
| `docs/src/docs/mon/` and standalone crate docs | Clean explicit schema API, retained Integer/Decimal families, direct rounding and typed examples without with_profile. Preserve syntax/runtime integration distinctions. |
| `docs/src/docs/packages/core/math/` | F64 constants/signatures, mathematical contracts, shared finite-result boundary and existing Wasm gap. Richer/generic Math stays later work. |
| Other Core/Builder and external-function references | Exact migrated signatures, constant/return validation, lengths/units/distribution and the existing capability matrix. |
| Memory/runtime scalar and ABI references | Exact scalar metadata and physical carriers without semantic profiles. Distinguish narrow checked result domains and F16 completion from wider scratch/register carriers. Preserve checked sizes and source/backend separation. |
| Design-scope and language overview | Remove convenience-type rationale and unrestricted/eager numeric-inference statements now superseded. Keep all unrelated exclusions. |
| Cheatsheet | Compact final exact-type and inference examples, one cast and safety catch, with no implementation-status clutter beyond its standard notice. |
| Style/testing/validation guides | Remove harness numeric_profile configuration and stale example types. Preserve meaningful test selection, assertion and validation contracts. |
| Both progress matrices | Separate accepted design from delivered scalar, inference, catch, config, MON, package and backend support. Link actual evidence without claiming future Wasm support. |
| `README.md`, agent routing, `index.md`, diagrams and scaffolds | Correct affected syntax and ownership names. Preserve unrelated content and reflect moved/deleted owners. |

Within the number references, explicitly replace minimum-width promotion with equal-type preservation and the smallest mixed-domain rules. Show that I8/U8 operations can fail in their own ranges and F16 completes at each operation. Preserve exact I64/U64 comparison without a common arithmetic type, integer `/` versus `//`, unsigned negation rejection, decimal exact literal fitting and operation-level rounding. Distinguish operand widening from a result cast and the default inferred type from an arithmetic minimum width.

Within the catch references, include both accepted multiple-call examples and the rejected nested-argument-only example. Explain that eligibility is not a claim that failure exists and that safety catch is not a transaction or a handler for every fatal condition.

### 10.2 Work-plan coordination

Update these files as coordination targets, not as new implementation dependencies. A one-shot plan should name prerequisite capabilities rather than depend on another temporary plan's wording.

| File to update | Required coordination |
|---|---|
| `docs/roadmap/roadmap.md` | New numeric work first in sequence, expression refactor explicitly paused, normal unrelated programme order preserved |
| `docs/roadmap/plans/typed-semantic-expressions-plan.md` | Preserve accepted checkpoints, remove eager numeric timing assumptions and require the explicit post-numeric reassessment before resumption |
| `docs/roadmap/plans/compiler-source-token-and-diagnostic-data-layout-plan.md` | Consume exact numeric types and pending-state/span outcomes, keep its separate activation and diagnostic-layout decisions |
| `docs/roadmap/plans/compiler-diagnostics-improvement-plan.md` | Reconcile delivered numeric/catch diagnostics and retain only genuine remaining work |
| `docs/roadmap/plans/general-directives-and-project-config-plan.md` | Consume all fixed numeric input types and destination-aware command literal materialisation, remove profile bootstrap assumptions |
| `docs/roadmap/plans/constraints-and-fast-math-plan.md` | Remove convenience domains/cast! spelling, consume D15 narrow result domains and F16 rounding, refresh obsolete type-domain safety witnesses and keep explicit casts checked. Separate safety-catch eligibility from actual failures/traps |
| `docs/roadmap/plans/html_project_backend_wasm_final_implementation_plan.md` | Own all further Wasm conversion/error/Math/Dec support. Consume D15 exact result domains, F16 completion, shared finite guards and the explicitly revised rejection inventory |
| `docs/roadmap/plans/packages/core-math.md` | Replace profile-based handoff with F64-only migration, retain the coverage inventory, finite-boundary ownership and future generic/precision work |
| `docs/roadmap/plans/packages/first-party-package-programme.md` and affected package companions | Coordinate numeric registration changes and prevent parallel reintroduction of NativeFloat/profile assumptions |
| Runtime anonymous-record and collection-producing-loop plans | Consume known numeric leaf inference and current result/collection contracts without adding broader inference |
| Later generics, type-parametric Core bindings and file-backed JS/WAT plan, not yet authored | Consume D15 and its division/Dec exceptions. Own explicit compiler-defined closed bounds for all visibilities and forwarding an already concrete expected type into a nested generic call. Keep body-inferred capabilities, operator overloading and conditional error signatures outside that work |
| MON integration/static-builder and package/persistence follow-up notes | Consume exact standalone schemas and explicit compatibility facts, not removed profile APIs |

Update retained benchmark/audit notes only where necessary to mark a conclusion historical or superseded. Preserve exact old evidence rather than changing it to match the new compiler. The numeric plan's completion does not reactivate Boracle research or unrelated diagnostic work automatically.

### 10.3 Generated documentation and final diff

Build docs with the current compiler under the repository gate. Inspect generated links, examples, navigation, removed names and code highlighting. Generated output must come from the compiler, never manual HTML edits.

Review old-versus-new source documentation by semantic contract. Confirm that numeric range, failure, rounding, profile-removal, inference-boundary and cast evidence information has not disappeared through mechanical compression. Keep detailed implementation status in the progress matrices rather than embedding it throughout final-design references.

## 11. Validation and performance protocol

### 11.1 Commands and environments

The current `justfile`, pinned Rust toolchain, `.node-version` and validation guide own executable command details. Confirm them at activation. Use an up-to-date local compiler binary or the Cargo form so an older installed moth does not validate the wrong source language.

Capture tool evidence:

```bash
rustc -vV
cargo clippy -V
cargo fmt --version
node -p 'process.execPath + " " + process.version + " " + typeof Math.f16round'
```

Use the exact repository Node version for authoritative results. A host that lacks the required F16 capability is an infrastructure failure, not a reason to relax a numeric test.

Run package-scoped Rust checks/tests so the default compiler lane is not silently changed by workspace feature unification:

```bash
cargo check -p moth
cargo test -p moth
cargo test -p moth-lexical
cargo test -p moth-mon
cargo test -p moth-lexical --doc
cargo test -p moth-mon --doc
cargo test -p moth --lib mon_syntax_parity
```

Use focused subsystem/test filters during iteration, but inspect the selected test count. Filter names are chosen from the actual changed test owner, not assumed from a filename.

Examples of known existing integration selection:

```bash
cargo run --quiet -- tests --case core_math_float_boundary_validation --backend html
cargo run --quiet -- tests --case core_math_basic_functions --backend html
cargo run --quiet -- tests --case core_math_transcendental_functions --backend html
cargo run --quiet -- tests --audit
```

For new cases and backend names, use the harness's current list/manifest output before selecting them. Preserve the original fixture ID when its contract still fits, or update selection commands and the coverage ledger after a deliberate rename. Do not build a second integration runner.

### 11.2 Gate cadence

| Checkpoint | Required gate |
|---|---|
| Small implementation slice | Focused unit/integration tests and affected crate check, including a demonstrated failing regression test before its fix where applicable |
| Phase 1 representation decision | Real-source probes, construction invariant tests and measured pending-state costs |
| Important feature/integration checkpoint | `just validate` plus affected gates outside the routine set |
| Feature/configuration coverage changes | `just feature-lane-check` and the affected lanes or `just test-feature-matrix` |
| Numeric performance-sensitive checkpoint | Relevant non-recording benchmark checks and scaling evidence |
| Documentation-only checkpoint | Current documentation build and source/generated diff review |
| Final implementation ready for main | `just validate-full` on the final integrated tree, plus any required affected opt-in lanes |

Use the current feature list rather than inventing new features for this plan. Preserve default, timers, detailed timers, counters, combined instrumentation, developer output, standalone crates and xtask coverage. If borrow/Boracle extraction changed mechanically, run the relevant existing named lanes without broadening their research scope.

Documentation commands:

```bash
cargo run --quiet -- check docs --terse
cargo run --quiet -- build docs --release
```

Formatting and lint commands follow the current repository recipes, including `just fmt-check` and `just clippy`. Full-workspace checks remain governed by the final gate and feature matrix.

### 11.3 Baseline and cohort preservation

Record performance before changing the numeric source cohort. Keep original and migrated source identities with an adaptation explanation. An old Int64 workload must not become I32 merely to compile, and added casts must be accounted for when they change the measured work.

Use at least five independent matched invocations for final comparisons, following the existing frontend/expression protocol. Keep machine, toolchain, build flags, Cargo features, parallelism, cache state and workload sizes comparable. Run the accepted non-recording commands such as `just bench-frontend-check` and `just bench-check` where relevant.

Keep three comparisons distinct:

1. Baseline current compiler and its current source cohort.
2. Migrated compiler with semantically equivalent explicit-width workloads where equivalence is possible.
3. New inference/safety-catch workloads measuring the new feature's additional cost.

Removing obsolete profiles or fixtures is not an inference speedup. D15 intentionally changes narrow result domains and F16 operation completion, so a workload with different checks or rounding is not a like-for-like performance comparison. Separate that new-semantics cost from matched workloads using equivalent explicit domains. Dropping runtime validation invalidates a comparison rather than improving it.

Report type-resolution, materialisation, constant-folding, generic finalisation, failure-summary and HIR time alongside end-to-end time. Include peak/retained bytes, pending literal/constraint counts, allocations, worklist revisits, stack pressure and destruction cost. Use existing counters/timers and extend them only where they answer a specific question.

### 11.4 Required scaling workloads

Prefer existing benchmark generators and manifests. Add a small number of focused cases only where the new cost is not represented:

| Shape | Risk to measure |
|---|---|
| Long numeric copy chain with one late receiver | Repeated whole-body scans or recursive stack growth |
| One numeric origin with many independent uses | Per-use cloning and quadratic conflict tracking |
| Deep grouped literal arithmetic with a late receiver | Repeated subtree materialisation and source reparsing |
| Many independent numeric regions | Global solver overhead imposed on unrelated concrete expressions |
| Numeric collections/maps and generic forwarding | Repeated constructed-type interning and provisional specialisations |
| Many safety catches with unreachable handlers | Retained dead recipes, sidecars, captures and slow pruning |
| Mutable cycles and mixed concrete/pending operands | Nontermination or excessive fixed-point iterations |
| Mostly concrete existing program | Overhead from routing every expression through a general solver |
| Concrete narrow arithmetic and F16 operation chains | Required selected-width checks/rounding versus duplicate work or accidental wider semantics. Report this separately from equivalent wide-domain baselines |

Measure several increasing sizes under the existing scaling policy. Avoid wall-clock thresholds in correctness tests. A deterministic operation-count invariant can protect a known algorithmic bound without depending on machine speed.

The existing expression protocol treats a repeatable slowdown above 5% on an affected representative case as material. Smaller repeatable regressions still need explanation. Diagnose extra parsing, copies, caches, type queries and retained storage before accepting a representation. Do not reset benchmark baselines to hide a regression or claim an unmeasured speedup.

### 11.5 Evidence and failure reporting

Keep raw command output, revision, environment and inventory identifiers under `tmp/exact-numerics/`. Summarise validated outcomes in the normal retained report. A passing targeted command does not establish that unrun gates pass.

Distinguish expected newly rejected source, existing unsupported backend capability, an actual regression and infrastructure failure. Existing baseline failures need matched evidence and explicit unresolved status. They are not a general allowance to ship new failures.

After a final edit, merge or rebase changes the tested tree or effective configuration, rerun the invalidated checks. Full validation evidence applies to the resulting main-bound tree, not an earlier parent.

## 12. Final deletion and completion checklist

### 12.1 Search-based closure

Use a repository-wide search as a completeness aid, then classify every result semantically. The search itself is not runtime correctness evidence.

Search families include:

```text
NumericProfile, numeric_profile, with_profile, IntWidth, FloatPrecision
NativeInt, NativeUint, NativeFloat
BuiltinTypeKey, CanonicalBuiltinType, builtin_type_ids
DataType::Int, DataType::Uint, DataType::Float
NumericScalar::Int, NumericScalar::Uint, NumericScalar::Float
ExpressionKind::Int, ExpressionKind::Uint, ExpressionKind::Float
HirExpressionKind::Int, HirExpressionKind::Uint, HirExpressionKind::Float
SchemaType::Int, SchemaType::Uint, SchemaType::Float
Value::Int, Value::Uint, Value::Float
CASTABLE_TO_INT, TRY_CASTABLE_TO_INT, CASTABLE_TO_FLOAT, TRY_CASTABLE_TO_FLOAT
cast!, Int32/Float32, Int32/Float64, Int64/Float32, Int64/Float64
result_type_ids_are_single_float, validate_float_success, ValidateFloat
```

Use exact symbol/word boundaries. `Value::Integer`, generic English "float", host Rust/JavaScript types, `BinaryFloatPrecision`, an operation named ValidateFloat and build-profile settings are not automatically obsolete. Keep shared useful owners and remove only the retired semantic cases. Historical documents and deliberately negative removed-syntax tests may retain old spellings with explicit purpose.

Inspect production, test, docs, scaffold, benchmark, feature-gated and generated-code sources. Exclude build output from the initial inventory, then inspect generated docs through their separate gate. Remove dead imports, no-op options, obsolete helper exports, unused flags and stale comments along with their last consumer.

### 12.2 Completion conditions

- [ ] Every approved decision D00-D15 has a permanent contract, implementation owner and verified coverage.
- [ ] Every mandatory acceptance row has a primary test owner and outcome.
- [ ] Int/Uint/Float and semantic numeric profiles have no supported public or internal type-identity path.
- [ ] Defaults are language-owned I32/F64, with no builder-controlled substitute.
- [ ] Numeric inference respects families, explicit boundaries and forward named-operand promotion.
- [ ] Equal-type eligible arithmetic preserves the operand type, mixed domains have no minimum-width floor and unary negation follows D15. Division/Dec/unsigned exceptions remain explicit.
- [ ] Narrow integer checks and F16 per-operation rounding use the semantic result domain rather than a physical carrier. Proofs and capability decisions use those same facts.
- [ ] Intentional D15 source/backend changes have reviewed coverage dispositions, with no old-domain fallback or missing replacement assertions.
- [ ] Cast targets never choose source types or appear through illegal distant context.
- [ ] One cast spelling preserves supported conversion values and ordinary failure delivery.
- [ ] Safety catch eligibility, actual failure effects and handler placement remain separate.
- [ ] Handler typing is independent of reachability, while dead executable work is removed safely.
- [ ] Finite external F64 validation survives every migrated supported path, including Core Math.
- [ ] I32 indices/lengths, I32 Dec exponents and physical size calculations preserve their distinct contracts.
- [ ] Config materialises command literals directly and preserves exact typed input compatibility/provenance.
- [ ] Standalone MON is cleanly exact-width without losing lossless Integer/Decimal data or useful profile-era coverage.
- [ ] Unchanged JS/Wasm contracts preserve their supported behaviour, D15-caused source/capability changes are explicit and unsupported Wasm work remains accurately tracked.
- [ ] The final HIR, analysis, proof, interface and generated artifacts contain no pending or stale facts.
- [ ] Coverage dispositions preserve every surviving assertion and retire only explicitly removed semantics or proven duplicates.
- [ ] Documentation, generated output, package notes, roadmap and progress matrices agree.
- [ ] Performance and full validation evidence correspond to the final integrated tree.
- [ ] Expression restart reassessment is complete and further implementation still requires its explicit resumption.
- [ ] Unique contracts/follow-ups are transferred before this one-shot plan is retired in the completion commit.

## Appendix A. Research provenance and limitations

This plan was prepared after the user approved the design interview. Repository investigation used the live GitHub connector, including branch metadata, roadmap/standards, canonical references, focused implementation reads and broad caller/test searches.

Inspected revisions:

| Reference | Observed revision | Role in this document |
|---|---|---|
| main | `ab706091376373451afb32081c21476b56b5752a` | Default-branch search/discovery and comparison with the accepted work branch |
| expression-refactor | `9a416ab670b4afcc62e9d9e036cf2de0390cf6dd` | Focused current implementation and accepted paused Phase 3 checkpoint inspection |

These are inspection receipts, not a pinned implementation baseline. Refresh current state when activating. Code search results identify default-branch owners, while critical expression/HIR/generic/config/MON reads used the expression branch explicitly. Follow moved files and newly accepted commits.

The following inspection anchors support the main implementation findings:

| Anchor | File and inspected area |
|---|---|
| R01 | `docs/roadmap/plans/typed-semantic-expressions-plan.md`, expression-branch status, Phase 0-3 evidence, owner map, Phase 4+ responsibilities and benchmark protocol |
| R02 | `src/compiler_frontend/ast/expressions/eval_expression/result_type.rs`, indexed recipes, literal-only flags, receiver/peer frames and authored failure order |
| R03 | `src/compiler_frontend/ast/generic_functions/calls.rs`, eager signature inference, evidence validation and concrete request recording |
| R04 | `src/compiler_frontend/builtins/casts/resolution.rs` and cast/failure references, spelling-selected evidence and target restrictions |
| R05 | `src/compiler_frontend/ast/expressions/failure_facts.rs`, compact summaries, origin witnesses and retained checked-operation eligibility |
| R06 | `src/compiler_frontend/hir/private_failure_lane.rs`, summary-dependent lane rewriting, unrouted handler pruning and final HIR revalidation |
| R07 | `src/compiler_frontend/hir/hir_expression.rs`, `hir_expression/calls.rs`, `hir_expression/numeric.rs` and fallible success extraction, builtin-Float selection/allocation and integrity modes |
| R08 | `src/compiler_frontend/build_config.rs`, `src/projects/cli.rs` and config resolution/contract owners, immediate profile materialisation and typed input vocabulary |
| R09 | `crates/moth-mon/src/model.rs`, schema/default/map/read/write owners and public API tests, independent profile-bearing schema/value representation |
| R10 | `src/compiler_tests/mon_syntax_parity/float_rounding.rs`, exact midpoint/subnormal/zero witnesses and redundant profile-cross-product structure |
| R11 | `docs/roadmap/plans/packages/core-math.md`, Math registration, finite boundary, coverage ownership and target limitations |
| R12 | `src/compiler_tests/integration_test_runner/expectations.rs`, integration profile fixtures and the current testing guide's assertion/inventory rules |
| R13 | `crates/moth-lexical/src/numeric/profile.rs`, explicit precision helpers and U64-to-F32 runtime/coercion fixtures, the direct-rounding counterexample |
| R14 | Compiler/build/data-layout documents, validation/style/testing guides, roadmap and the supplied code review guide, authority and ownership constraints |

The research established affected owners and failure mechanisms. It did not execute compiler tests, measure benchmark performance or prove exhaustive coverage of every file in the future activation tree. A local network checkout was unavailable and the planning environment did not contain Rust/Cargo/just. Phase 0 and the later gates deliberately require the implementation agent to establish fresh executable evidence.

**Replacement-document provenance:** the 2026-10-08 revision applies the user-approved type-preserving arithmetic change to this supplied plan, its work packages, acceptance cases, coverage protocol and downstream coordination. It preserves the earlier research receipts and does not claim a new repository baseline or compiler test run. The implementation agent must still refresh the activation tree and validate the revised semantics under the gates above.

## Appendix B. Agent handoff and resumable work notes

Keep a compact work record under `tmp/exact-numerics/` that survives context compaction. It should contain the actual base/tree, current phase/package, accepted decisions, changed owners, tests run, unresolved failures and the next bounded action. Keep detailed logs and the coverage ledger separate rather than embedding them into every progress note.

At each checkpoint, report:

```text
Changed contract and producing owner
Consumers migrated and old code deleted
Tests and exact executed scope
Coverage ledger entries closed or still open
Documentation and downstream-plan changes
Performance evidence where relevant
Actual blockers and next bounded work package
```

A suggested commit sequence follows the phase contracts rather than fixed file counts:

```text
record numeric contracts, pause expression work and inventory evidence
unify cast evidence/delivery and source-authored safety catch
harden exact-width scalar and external finite-result boundaries
cut over exact numeric identities, revised arithmetic domains and scalar inference
complete container, generic and fallback numeric relationships
migrate builtins, packages, HIR and supported backend paths
migrate config/MON and remove semantic profile plumbing
complete fixture/harness/docs migration and integrated validation
harden ordering, publication, rounding and regression coverage
record final performance and expression restart reassessment
transfer permanent contracts and retire the numeric plan
```

Split large entries into smaller coherent owner/consumer commits as needed. Mark dependent work honestly and preserve the integrated cutover gate. An arbitrary file-count limit must not produce a fake compatibility layer or a commit advertised as complete while its semantics are inconsistent.

The completion report must identify the exact validated tree, covered backend matrix, coverage migrations, numerical witnesses, performance results and expression restart status. It must also state every unrun gate or remaining limitation plainly. Additional Wasm support and richer Core Math remain in their later owners.
