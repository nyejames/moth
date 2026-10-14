# Reusable generics, type-parametric Core bindings and file-backed JS/WAT

> **For agentic workers:** implement the numbered work packages with the repository's agent workflow. Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` when available. Preserve the review and validation gates below. This document authorises neither immediate activation nor changes outside its scope.

**Goal:** establish Moth's final V1 generic declaration model, explicit closed numeric-family limits, bounded contextual generic calls and trusted scalar-parametric Core bindings. Add file-backed JavaScript and annotated WAT authoring, then exercise the shared mechanisms through generic Core Math selection and absolute-value functions.

**Architecture:** reusable generic declarations are file-private compile-time metadata. Each owning function or nominal declaration creates its own canonical parameters. Frontend validation checks declared limits and result relationships without inspecting callees or searching implementations. Concrete source specialisations use existing generated-function ownership. Concrete binding instances use validated semantic signatures, target implementations and explicit ABI adapters. JS/WAT files own implementation code, not language semantics.

**Technology:** safe idiomatic Rust, existing compiler-owned type/expression/numeric stores, Moth source fixtures, JavaScript, WebAssembly text and binary validation, the current Cargo/just tooling and the existing Node integration harness.

**Specification:** Sections 2 through 8 contain the approved design and the explicit operational rules needed to implement it. The decision ledger identifies the interview decisions and their supersession. Phase 0 publishes the new contracts into permanent language and architecture owners before implementation relies on them. The worker does not need the originating conversation.

**Intended repository path:** `docs/roadmap/plans/generics-core-bindings-and-file-backed-js-wat-plan.md`

## Status and sequencing

```text
STATUS: queued, implementation not started
CURRENT_SLICE: Phase 0 activation pending
BLOCKERS: normal serial roadmap position and the prerequisite capabilities below
NEXT_ACTION: activate explicitly after Never return contracts, then refresh owners and establish evidence
```

Insert the following single entry in `docs/roadmap/roadmap.md`, immediately after the existing Never return contracts entry:

```markdown
- [Reusable generics, type-parametric Core bindings and file-backed JS/WAT](./plans/generics-core-bindings-and-file-backed-js-wat-plan.md) - Queued: file-private reusable generic declarations, closed numeric-family limits, bounded nested-call context, trusted scalar-parametric Core bindings and file-backed JS/WAT implementations. Deliver generic min, max, clamp and uniformly fallible abs. Consume exact numeric semantics and existing backend capabilities without waiting for the later mixed-Wasm implementation.
```

This position is before the currently listed struct-layout, collector-free memory and mixed-Wasm work. Those later programmes are not prerequisites for this entire plan. Deliver the scalar file-backed path without requiring general mixed-target partitioning, managed Wasm aggregates or a new allocator. Unsupported representation-dependent instances remain explicitly target-gated.

At initial activation, remove the deferred heading `## Generics, type-parametric Core bindings and file-backed JS/WAT` and its associated placeholder paragraphs. Transfer its numeric invariants and scope limits into the permanent owners as specified in Phase 0. Preserve adjacent headings, especially `## Optional catch tooling advisory`, and preserve the permanent `Numeric and generic extensions` handoff. Deletion is idempotent if the placeholder has already been consolidated.

Authoring or inserting this plan does not resume a paused earlier plan. Do not mark this entry active until the user activates implementation at its serial position. Establish a fresh baseline then rather than pinning the research revision as an activation prerequisite.

## Navigation

| Need | Section |
|---|---|
| Philosophy, approval record and exclusions | [Execution](#1-execution-contract) and [decisions](#2-decision-ledger-and-explicit-supersession) |
| Reusable declarations, scope and nominal types | [Reusable declarations](#3-reusable-generic-declarations) |
| Numeric limits, operators and literals | [Closed families](#4-closed-numeric-family-limits) |
| Generic call inference and specialisation | [Call completion](#5-bounded-generic-inference-and-concrete-completion) |
| Trusted binding architecture | [Parametric bindings](#6-type-parametric-core-and-builder-bindings) |
| JS/WAT files, annotations and adapters | [File-backed authoring](#7-file-backed-jswat-authoring) |
| Mandatory Core Math delivery | [Math contract](#8-core-math-delivery-and-semantics) |
| Current repository owners | [Owner map](#9-repository-investigation-and-implementation-owners) |
| Staged implementation work | [Phases](#10-implementation-sequence) |
| Regression migration and acceptance cases | [Migration](#11-coverage-migration-protocol) and [acceptance matrix](#12-mandatory-acceptance-matrix) |
| Documentation and downstream coordination | [Documentation edits](#13-required-documentation-and-roadmap-edits) |
| Performance and validation | [Validation](#14-validation-and-performance-protocol) |
| Completion and deferred work | [Completion](#15-completion-and-deferred-boundaries) |
| Research provenance | [Inspection receipt](#appendix-a-research-provenance-and-implementation-limits) |

## 1. Execution contract

### 1.1 Authority and prerequisite capabilities

Read `AGENTS.md` from the activation worktree. Follow its routing and the current code review guide. Architectural changes require the relevant authorities and adjacent handoffs, not just their examples.

Required permanent owners include:

- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/compiler-data-layout-design.md`
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`
- `docs/src/developer-docs/language/overview.mtf` and its relevant unsuffixed references
- `docs/src/developer-docs/memory-management/overview.mtf` and its routed call, return-provenance, external-boundary and physical-ABI references
- `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf`
- Both progress matrices, current roadmap and relevant retained performance reports

Consume these capabilities when they are delivered at activation:

1. Exact numeric types and D15 type-preserving operator semantics, bounded numeric-origin inference, unified cast and stable safety catch.
2. The earlier typed-expression, dense-HIR and native-result contracts, including source-owned spans and concrete generated-function publication. Refresh actual implementation rather than assuming an old node representation.
3. The previously scheduled diagnostic and directive foundations, with their current failure lanes and source syntax.
4. Never return contracts and their established terminality rules.
5. Existing external-provider registration, artifact ownership, target validation and supported Wasm scalar execution.

Consume any delivered trusted Core constant evaluator without constructing another evaluator. Adding evaluation of arbitrary Moth, JavaScript or WAT source is outside this plan.

The explicit user-approved changes below supersede old generic declaration syntax, constructor-only inference and the previous exclusion of closed compiler-owned numeric limits. They do not supersede unrelated numeric, memory, result, visibility or source-selection contracts.

### 1.2 Global implementation constraints

Keep one production implementation. Moth is pre-release, so remove replaced grammar, APIs, bindings, documentation and test assumptions instead of keeping aliases, feature switches or compatibility interpreters.

The frontend owns source meaning, limit validation, generic relationships and failure contracts. Build code schedules providers and artifacts. Backends implement already selected semantics. Neither builders nor file annotations may redefine operators, select source numeric widths or supply arbitrary type-checking callbacks.

Use stage-owned dense data, typed IDs, borrowed views and immutable published records. Share the limit classifier, argument routing, literal materialisation, known-shape relationships and concrete signature substitution. Do not introduce a general unification solver, a second nominal type system or a registry of unrestricted semantic callbacks.

Semantic type, storage representation and physical ABI remain distinct. A carrier wide enough for a value does not widen its source type. The public contract and target implementation are separate facts.

User source and file errors receive structured diagnostics with exact source context. Operational IO/tool failures and broken trusted metadata follow the existing distinct failure lanes. Untrusted WAT or malformed annotations must not panic the compiler.

Use British English and basic ASCII in authored documentation. Keep the terms **generic**, **reusable generic declaration** and **limit** consistent. Reserve **constraints** for the separately owned constraints feature.

### 1.3 Review focus

| Risk | Required defence |
|---|---|
| File-level generic identity leaks across declarations or consumers | Owner-local parameter tests, public-interface round trips and colliding donor/requester names |
| A body, literal, trait search or backend silently narrows a limit | Universal-validation negatives and unchanged API fingerprints under body-only edits |
| Delayed nested calls default or execute in discovery order | Reordered sibling tests, exact-once runtime traces and bounded worklist counters |
| Removing constructor inference loses nontrivial old coverage | Assertion-level migration ledger and explicit contextual replacements |
| Comments or file location grant privileged Core ABI access | Ordinary/trusted profile separation, metadata-forgery and adapter rejection tests |
| WAT introduction creates a dependency cycle with later backend work | A complete memory-independent scalar execution path and precise missing-capability cases |
| Core selection loses numeric precision, Dec provenance or declared failure | Full-width integer cases, narrow float bit checks, alias/lifetime tests and uniform abs error handling |

Every code-bearing slice ends with the repository Slice review. Structured audit coverage is not created by implementing this plan.

## 2. Decision ledger and explicit supersession

| ID | Final decision | Delivery and primary acceptance |
|---|---|---|
| G01 | Numeric generic eligibility is explicit at every visibility. The body validates the authored limit and never infers or narrows it. | Phases 2/3, L03-L04, P08 |
| G02 | Use compiler-owned numeric families, not user-defined operator traits or per-operator conformance. | Phases 3/6, L01-L02, B05 |
| G03 | A generic operator result must have a uniform existing-parameter or concrete-type relationship. No computed `Promote<A, B>` type. | Phase 3, L05-L16 |
| G04 | Raw literals may receive generic numeric context only when valid for every admitted type. Retain spelling until concrete materialisation. | Phase 3, L20-L24 |
| G05 | Forward already concrete expected types into nested generic calls, including information independently supplied by sibling arguments. Preserve source evaluation order. | Phase 4, I01-I18 |
| G06 | New trusted parametric bindings are scalar-numeric initially, with concrete supporting slots and uniform declared error contracts. Generic containers remain ordinary Moth-source capabilities, not a new binding ABI. | Phase 6, B01-B04 |
| G07 | Replace signature-local generic introduction with reusable declarations at file scope. The declarations are always file-private. | Phases 1/2, D09, D15-D22, P01-P07 |
| G08 | `Item #Type`, `Number = NUMERIC` and `Number #Type = NUMERIC` are valid declaration forms. `Type` without compile-time mode is diagnosed. | Phase 1, D01-D06 |
| G09 | V1 permits zero or one limit. No trait lists, alternatives, conjunctions, generic copying or generic aliasing. | Phases 1/5, D07-D08, N15 |
| G10 | A repeated generic name means the same parameter within one signature. Different names mean independent parameters, even with identical limits. Each owning declaration and concrete instantiation is independent. | Phase 2, D15-D20, P07 |
| G11 | Structs and choices use an explicit ordered list: `Pair of First, Second = ...` or `Maybe of Item :: ...`. | Phases 2/5, N01-N05, N14-N16 |
| G12 | A generic constructor needs an explicitly applied receiving type. Argument-only generic construction is removed. `of` stays in type positions, not function-call type arguments. | Phase 5, N06-N13 |
| G13 | WAT annotations reuse JS Moth signature semantics. Imports, runtime capabilities and non-native adapters are explicit and validated. File authorship is not an unrestricted foreign ABI. | Phases 7/8, A01-A14, W01-W22 |
| G14 | Core algorithms may live in JS/WAT files. Trusted registration connects semantic operations to target implementations without requiring Moth-source wrappers. | Phases 6-9, B07-B14, A14, M16 |
| G15 | Mandatory consumers are generic `min`, `max`, `clamp` and `abs`. Only `abs` adds a uniform public `Error!` slot. | Phase 9, M01-M20 |
| G16 | Conditional generic error signatures, general effect-polymorphism and generic error-presence directives remain outside scope. | Phases 4/6/9, B13, M10-M11, M17 |
| G17 | Further Core generic opportunities, precision expansion, decimal algorithms and generic constants are recorded as later work, not delivered incidentally. | Phases 9/11, M20 and Section 15.3 |
| G18 | Place this entry immediately after Never return contracts. Remove the old generics deferred heading at initial activation without removing its permanent handoff or adjacent tooling work. | Phase 0 and Section 13.3 |

The family names are `NUMERIC`, `INTEGER`, `SIGNED_INTEGER`, `FLOAT`, `DEC`, `INT_16_PLUS`, `INT_32_PLUS` and `FLOAT_32_PLUS`.

Superseded discussion is not an alternate implementation option:

- `BINARY_FLOAT` becomes `FLOAT`, and `DECIMAL` becomes `DEC`.
- `type T is LIMIT` is replaced by reusable declarations, not retained as shorthand.
- `A = Number`, `A as Number` and `Number as A` do not copy a generic in V1.
- Lists such as `FLOAT, INT_32_PLUS, DEC` and `SIGNED_INTEGER and INT_32_PLUS` are outside V1. Do not add compound families merely to preserve those examples.
- Automatic private-only numeric limits and a public/private hybrid are rejected.
- Removing `of`, discovering nominal parameter order from fields and mandatory temporary bindings for all generic arguments are rejected.
- The numeric migration is already separately owned. This plan never restores minimum-I32 or minimum-F32 arithmetic promotion.

## 3. Reusable generic declarations

### 3.1 Grammar and classification

The accepted forms are:

```moth
Item #Type
Number = NUMERIC
OtherNumber #Type = NUMERIC
Textual #Type = DISPLAY_TEXT
```

`DISPLAY_TEXT` in the final line denotes one ordinarily visible trait, not a value. A single limit may be a compiler-owned numeric family or a visible ordinary trait. The limit initializer is one resolved name under existing trait-name/dependency-binding rules, not an arbitrary expression.

`#Type` is a declaration annotation with no runtime type or value representation. It does not produce an ordinary folded constant. An inferred limit declaration has the same compile-time-only status even without an explicit `#Type` annotation.

Reject:

```moth
Item Type
Number Type = NUMERIC
Number = FLOAT, INT_32_PLUS
Number = SIGNED_INTEGER and INT_32_PLUS
Number = FLOAT or DEC
Alias = Number
Alias as Number
Number as Alias
```

The first two diagnostics explain that `Type` is compile-time-only and suggest `#Type`. The list/conjunction/alternative diagnostics identify the one-limit V1 rule. The final three reject copying or aliasing a reusable generic declaration rather than treating it as an ordinary value or concrete type alias.

A declaration without a limit requires `#Type`. Empty parentheses, `none`, a concrete type such as `I32`, a function call and a runtime expression cannot initialise one. Mutable generic declarations, assignment after declaration, destructuring and declaration modifiers that imply input/runtime values are invalid.

This extension neither creates first-class traits nor permits `Type`-valued fields, parameters, returns, options, collections, maps, config inputs or MON values. A generic name is usable only in the type positions described below.

### 3.2 Names, locality and visibility

Generic names follow ordinary type-name casing and no-shadowing rules. Family identifiers are recognised at their exact uppercase spellings and cannot be redefined as user families or conformed to by user types. Adding them reserves no lowercase or title-case equivalents. Keep the existing Dec-family name protection while making `DEC` a legal limit reference. `Decimal` and `DECIMAL` gain no new reservation.

`Type` is recognised as the compile-time generic declaration annotation. Removing the old `type` introducer does not justify deleting `is` or `and` from unrelated language constructs. Equality, matching and Boolean syntax keep their current owners.

Reusable generic declarations exist only at top level in ordinary source files that permit declarations. They are not declared inside functions, fields, loops, branches, templates, config or export declaration groups. They produce no root runtime work. They are available through the file's header/declaration binding, not an executable-order lookup or a value initialiser evaluation. There is no adjacency rule or new textual-before-use rule for these metadata declarations. Retained local ordering resolves their referenced limits before use.

They cannot be imported, exported, re-exported, selected from a namespace or injected into sibling-file visibility, including within the same module. Two files may each declare `Number = NUMERIC` independently. Each file needing a generic declares it locally.

Recommend declaring a single-use generic immediately before its consumer. Reusing it elsewhere in the same file or choosing fresh descriptive names with the same limit remains legal. Do not emit a new default warning merely for choosing one style over another.

An exported function or nominal type can use a file-private generic declaration. Its public interface publishes owner-local parameters and resolved limits, not the file-private declaration symbol. An ordinary trait used as a public parameter's limit still has to meet existing public visibility and evidence rules. Generic privacy cannot conceal a private trait in an exported API.

### 3.3 Identity and ownership

There are three distinct concepts:

| Concept | Identity and lifetime |
|---|---|
| Reusable declaration | File-local name and immutable optional limit metadata |
| Parameter of a function or nominal type | Canonical identity owned by that declaration and its parameter slot |
| Concrete specialisation | The owning declaration plus its concrete canonical arguments and any required evidence |

The reusable declaration itself never becomes concrete. Its use creates or refers to the appropriate owner-local parameter, and only a concrete instance substitutes that parameter.

Within one signature, all occurrences of the same reusable declaration refer to one parameter. Different reusable declarations remain different parameters even when their limits match. Independence permits different concrete types but does not require them to differ.

```moth
Left #Type
Right #Type

first |left Left, right Right| -> Left:
    return left
;
```

This signature relates the result to `Left`, not to `Right`. It can be instantiated with two equal or two different concrete types. A result annotated with a third generic name would be a third independent parameter, not an inferred common type.

A reusable declaration used by two functions creates independent parameters in those functions. One function being instantiated as I32 does not constrain another function, nor a later instantiation of the first function.

A runtime call's arguments are selected at compile time for that concrete call. V1 introduces no dynamic type selection, type erasure or runtime trait dispatch.

### 3.4 Function signatures and bodies

Functions collect their owner-local parameters from the declared signature, including permitted nested type shapes and the optional error type. Scan parameters in their declared order, including an existing hidden receiver slot where applicable, then success returns and the error slot. First occurrence establishes a deterministic internal slot and later occurrences reuse it. This order is metadata, not new call-site type-argument syntax.

```moth
Item #Type

identity |value Item| -> Item:
    return value
;

count I32 = 42
text String = "Priya"
count_copy = identity(count)
text_copy = identity(text)
```

`identity` has separate I32 and String instances. Reusing `Item` in another function creates that function's own parameter.

```moth
Number = NUMERIC

add |left Number, right Number| -> Number, Error!:
    return left + right
;

greater |left Number, right Number| -> Bool:
    return left > right
;
```

Both functions reuse the declaration but own independent parameters. The two arguments of each call must resolve to the same exact type. Operators do not introduce a common-type search at generic call boundaries.

A body may refer to parameters established by its signature in local annotations and existing legal type shapes. A body-only use of another reusable generic does not silently add a parameter. Diagnose it and point to the missing signature relationship. A top-level ordinary value declaration such as `value Number = 1` does not create a generic owner or an alternative bounded-literal inference mechanism.

Return-only parameters remain possible where an immediate concrete receiving context can solve them under Section 5. A limit alone never chooses their type. A generic error parameter preserves the existing explicit error-slot rules and does not make the presence of that slot conditional.

Ordinary source traits continue to use their established `This` and method requirements. This plan does not add generic traits, generic trait methods, defaults, associated types, associated constants or blanket conformance.

### 3.5 Generic structs and choices

Nominal declarations explicitly list previously declared generics in order:

```moth
First #Type
Second #Type

Pair of First, Second = |
    first First,
    second Second,
|

item Pair of I32, String = Pair(42, "hello")
```

```moth
Item #Type

Maybe of Item ::
    Some |value Item|,
    Empty,
;

present Maybe of String = Maybe::Some("hello")
absent Maybe of String = Maybe::Empty
```

The list creates the nominal declaration's own parameters. Repeated uses in fields or variant payloads refer to those parameters. The list is mandatory whenever generic names occur in the nominal field/payload contract. Its order is stable independently of field or variant ordering.

Reject duplicate entries, an entry that is not a reusable generic declaration, a listed parameter unused by the nominal contract and a generic reference missing from the list. A nominal declaration does not acquire generic parameters from method bodies, default expressions or constructor calls.

`of` remains head-directed. Its ordinary type-application meaning does not change Wire/Route capability syntax or permit arbitrary new heads to define a type grammar.

Existing nested-inline-`of` restrictions remain. A concrete alias can name a fully applied nested type where the ordinary syntax requires one. Parameterised aliases, partial application and aliases of reusable generic declarations remain excluded.

Aligned receiver methods continue to use the nominal's corresponding parameter contract, with exact position/limit alignment and no extra independently generic receiver parameters. For example:

```moth
Item #Type

Box of Item = |
    value Item,
|

get |this Box of Item| -> Item:
    return this.value
;
```

Preserve same-file nominal receiver ownership, mutable access rules and the rejection of methods specialised on one concrete generic instance. Free functions can use independently named parameters in applied nominal types according to their signatures.

### 3.6 Explicit contextual generic construction

An authored generic nominal constructor requires a receiving type whose nominal identity and full type arguments are established independently of that constructor's arguments. The receiver can be an explicit local annotation, return slot, field, default, collection element or already resolved call parameter. In a generic body it may be an explicitly applied symbolic type such as `Pair of First, Second`, whose owner-local parameters are already established.

```moth
item Pair of I32, String = Pair(42, "hello")
```

is valid. These are rejected:

```moth
item = Pair(42, "hello")

count I32 = 42
name String = "hello"
item = Pair(count, name)
```

Concrete argument types alone no longer supply the generic nominal application. Do not keep an argument-inference fallback behind the new syntax.

The receiver need not be repeated in the constructor's spelling. `Pair(...)` and `Maybe::Empty` remain ordinary construction syntax. `Pair of I32, String(...)` is not new call syntax. Concrete aliases are type names, not new constructors.

A concrete typed parameter can supply context to `consume_pair(Pair(...))`. An unresolved generic outer parameter cannot do so until independent receiving or sibling evidence resolves it. A constructor's own arguments cannot break that cycle. A typed collection element can provide the same context. Both empty and payload-bearing choice variants obey the rule.

The following function uses the `First`, `Second` and `Pair` declarations in Section 3.5:

```moth
make_pair |first First, second Second| -> Pair of First, Second:
    result Pair of First, Second = Pair(first, second)
    return result
;
```

A call to `make_pair` can infer its arguments from the function signature. Its already resolved result is an ordinary concrete instance. This is not argument-only nominal construction and remains valid without another redundant local annotation.

### 3.7 Required teaching contrasts

The converted generic documentation must show, with positive and negative examples:

- One name repeated versus two fresh names with the same limit.
- One reusable declaration used by several functions versus one function instantiated several times.
- Function signature ownership versus nominal ordered parameter ownership.
- Unrestricted `#Type`, numeric-family limits and an ordinary trait limit.
- A generic body using T versus a body-only generic that is absent from the signature.
- File-local declarations used by exported functions versus forbidden import/export of those declarations.
- Explicit contextual nominal construction versus inferred generic function calls.
- A concrete generic instance such as `Pair of I32, String` versus the reusable generic declarations themselves.

Use standalone examples with their necessary declarations. Do not explain locality as permanent concretisation or imply that two generic names must select different types.

## 4. Closed numeric-family limits

### 4.1 Inventory and membership

| Limit | Complete V1 membership |
|---|---|
| `NUMERIC` | Eight fixed integer types, F16/F32/F64 and all canonical Dec scales |
| `INTEGER` | I8, I16, I32, I64, U8, U16, U32, U64 |
| `SIGNED_INTEGER` | I8, I16, I32, I64 |
| `FLOAT` | F16, F32, F64 |
| `DEC` | Dec/Dec0 through Dec256, with Dec and Dec0 one canonical type |
| `INT_16_PLUS` | I16, U16, I32, U32, I64, U64 |
| `INT_32_PLUS` | I32, U32, I64, U64 |
| `FLOAT_32_PLUS` | F32, F64 |

The names are closed compiler-owned families, not ordinary traits with implementable requirements. User nominal numeric wrappers do not join them through fields, conversion methods or matching operator-like methods. Transparent aliases of concrete members retain the existing underlying identity.

Byte, Char and Bool are excluded. Numeric comparisons produce ordinary Bool values, which can use Boolean operators normally. The families add no truthiness or Boolean operator overloading.

Membership describes semantic types, not target register width, allocation size, architecture or measured speed. `INT_32_PLUS` means the currently supported listed widths, not invented wider types or a promise to admit future types without reviewing their contracts. Dec scale remains independent of binary width.

V1 has one limit per declaration. There is no user-defined union, intersection, subtraction, alias or parameterised family expression. The compiler may use closed set relationships internally. For example, SIGNED_INTEGER implies INTEGER, and FLOAT_32_PLUS implies FLOAT and NUMERIC. This permits signature-based forwarding to a weaker family without adding source trait inheritance or multiple-limit syntax.

### 4.2 Universal declaration validation

A body is valid only if its operations, required literal materialisations, call relationships and declared results are valid for every type admitted by its explicit limit. Invalid work diagnoses the declaration with the limiting operation and a useful counterexample type. It does not reduce the declared set.

An unrestricted `#Type` permits only the existing unconstrained generic operations. An ordinary trait limit permits its established method/evidence behaviour. Neither automatically becomes a numeric family because of the body or the types used by existing callers.

A declared limit validates selected call types rather than selecting them. Keep the exact-numeric rule that trait availability, limit membership, runtime value range, error handling and target capabilities do not choose a concrete type. An independently selected I32 argument passed to a FLOAT-only generic is rejected, not converted or retried as F64.

Use one compiler numeric-family owner for membership, known family implication, operator obligations and diagnostics. Reuse the delivered numeric operator and literal policies. Do not generate every possible specialisation to validate a template or build an arbitrary Boolean constraint solver.

### 4.3 Preserved arithmetic contract

The permanent numeric operator table is the authority. In particular:

```text
I8 + I8       -> I8
U8 + U8       -> U8
I8 + I16      -> I16
I8 + U8       -> I16
I32 + U32     -> I64
I64 + U64     -> invalid arithmetic
F16 + F16     -> F16
F16 + F32     -> F32
same integer / same integer -> F64
DecN ^ I32    -> DecN
```

Equal supported types retain their width for ordinary type-preserving operations. Mixed integers use the smallest available common domain for their complete input ranges. Results can still overflow that domain. Fixed floats complete rounding and finite checks at each selected operation precision, including F16. Signed negation keeps its type and checks the signed minimum. Unsigned and Byte negation remain invalid.

Preserve integer `/` versus `//`, binary-float `//` rejection, exact cross-width integer comparison, Dec scale-specific division and remainder, exact integer/Dec mixing and the I32 decimal exponent. Generic support is not permission to replace any exception with a uniform but different numeric rule.

### 4.4 Permitted generic result relationships

An unresolved generic operation can produce:

1. One existing owner-local generic parameter, when that relationship is valid for every admitted substitution.
2. One concrete type, when every admitted substitution produces that type.

The condition applies to intermediate expressions as well as the function's final result. There are no public or internal escaping computed types such as `Promote<A, B>`, no associated result types and no return-type-driven backwards arithmetic through named operands.

| Declarations and operands | Expression | Template result |
|---|---|---|
| Same N limited to NUMERIC | `a + b` | N |
| Same N limited to INTEGER | `a / b` | F64 |
| Same N limited to FLOAT | `a / b` | N |
| Same N limited to DEC | `a / b` | Invalid over the full family because Dec0 does not support it |
| Independent A/B limited to INTEGER | `a > b` | Bool, with exact concrete comparisons |
| Independent A/B limited to SIGNED_INTEGER | `a + b` | Rejected because its result is not uniformly A, B or one concrete type |
| A limited to SIGNED_INTEGER and an explicitly concrete I64 operand | `a + wide` | I64, provided ordinary typing validates the expression |
| Independent A/B limited to FLOAT | `a > b` | Bool |
| Independent A/B limited to FLOAT | `a + b` | Rejected for the unresolved wider-result relationship |
| Same N limited to NUMERIC | `a > b and b > c` | Bool |
| Same N limited to NUMERIC | `-a` | Rejected because unsigned members forbid negation |
| Same N limited to SIGNED_INTEGER | `-a` | N, with checked failure where required |
| Same N limited to DEC and concrete I32 exponent | `a ^ exponent` | N |
| Same N limited to NUMERIC | `a ^ b` | Rejected because the Dec exponent contract is not `(N, N)` |

Both generic parameters must independently meet the declared guarantees. Two names with the same family do not promise equal widths, signedness or decimal scales. In particular, two independently DEC-limited operands do not automatically share a scale.

A concrete operation after specialisation retains its ordinary mixed-width rules. Template validation is intentionally more restricted than general type-level result computation. The diagnostic should recommend using one shared generic parameter, an appropriate concrete type or a narrower already available family, not inventing an implicit cast.

### 4.5 Literals and constants

Raw literal spelling can receive an active generic numeric parameter at an eligible receiving site or local numeric peer. Check its validity across the declared family at declaration time, then materialise directly from that spelling at the selected type during concrete specialisation.

```moth
Number = NUMERIC

increment |value Number| -> Number, Error!:
    return value + 1
;
```

Every NUMERIC member can receive the raw whole literal 1. Addition still follows the selected type's normal failure contract.

```moth
Count = INT_32_PLUS

microseconds |seconds Count| -> Count, Error!:
    return seconds * 1_000_000
;
```

The family admits the embedded constant but does not prove all products fit. This illustrates why minimum-width families are useful.

| Receiving generic limit | Literal | Required declaration outcome |
|---|---|---|
| NUMERIC | `1` | Valid |
| INTEGER | `127` | Valid across signed and unsigned members |
| INTEGER | `128` or `256` | Invalid because I8 cannot receive it |
| INT_16_PLUS | `1000` | Valid |
| INT_32_PLUS | `1_000_000` | Valid |
| FLOAT | `0.1` | Valid, with each destination's normal rounding |
| NUMERIC | `0.5` | Invalid because integer members cannot receive decimal spelling |
| DEC | `0.5` | Invalid because scale zero cannot fit it exactly |
| FLOAT | A spelling outside F16's finite materialisable range | Invalid over FLOAT even if it fits F32/F64 |
| Unrestricted #Type | A literal requiring unknown T materialisation | Invalid without another legal concrete receiver |

Do not confuse the universal literal bound with runtime range. INTEGER does not make every I64 instantiation an eight-bit calculation. The embedded literal must be admissible everywhere, while operations use the instantiated type.

An ordinary constant or already typed binding retains its concrete type. It does not become a raw generic literal because its value is known. Respect the numeric plan's distinction between immediate literal context and a bound integer whose family is already established. Cast destinations remain concrete immediate receivers, and generic cast targets remain excluded.

The generic validator may retain a compact symbolic operation/literal obligation in its own template construction state. It must not insert a fake concrete numeric TypeId or retain a whole per-type AST. Reuse family extrema, scale rules and direct materialisation checks. Avoid enumerating combinations of unrelated generic parameters.

### 4.6 Ordinary trait limits and closed family implication

Ordinary user traits remain explicit same-file nominal conformance with ordinary visibility and canonical evidence. No method-name matching substitutes for evidence. A single trait limit does not gain numeric operators because some currently visible implementations happen to be numeric.

Closed family implication is an internal, deterministic membership relation. A FLOAT_32_PLUS parameter can be forwarded to a FLOAT parameter and an INTEGER parameter to a NUMERIC parameter. NUMERIC cannot be forwarded to INTEGER without an independently valid relationship that covers every admitted type. No callee body is inspected to relax its declared limit.

The same declared-limit rules apply to free functions, nominal types, aligned receivers and trusted parametric bindings. Public interfaces retain limit kind explicitly, distinguishing a builtin family from a canonical user trait identity.

## 5. Bounded generic inference and concrete completion

### 5.1 Existing behaviour to retain

Generic calls use ordinary call syntax. Concrete type arguments come from declared signature relationships, independently typed arguments, eligible immediate receiving context and the delivered numeric-origin exception.

A resolved function result is valid argument evidence without an intermediate binding. A temporary name is neither required nor sufficient to make a type concrete. Template validation may connect symbolic owner-local parameters, while executable requests must be concrete before publication.

```text
read_count() -> I32
identity(T) -> T
identity(identity(read_count())) -> I32
```

No function body is consulted to determine these relationships.

The existing numeric-origin case remains:

```moth
value = identity(1)
take_u64(value)
```

The declared parameter/result equality can carry the numeric origin until its permitted body-local uses select U64. This does not give a result-only nonnumeric generic an unrestricted later-use solver.

### 5.2 New expected-type forwarding

An already concrete expected type may be forwarded to a nested generic call's success result. Its source can be an annotation, field/default, return, already known call parameter or an independently typed sibling argument that resolves the outer signature.

Assume `empty` has signature `() -> {T}`, `consume` takes `{I32}` and `pick` has `(T, T) -> T`:

```moth
consume(empty())

known {I32} = {1, 2}
first = pick(empty(), known)
second = pick(known, empty())
```

All three are valid under the new rules. Both sibling orders establish the same type. The compiler does not evaluate `known` or another argument early to discover its type.

```moth
unknown = pick(empty(), empty())
```

remains underdetermined without an independent concrete receiver. This is valid with the receiver supplied:

```moth
resolved {I32} = pick(empty(), empty())
```

Forwarding may traverse further nested calls only through already declared signature relationships and known type structure. There is no arbitrary syntactic depth-one exception, but storage and work are bounded by authored calls and edges. A call still unresolved after permitted evidence is exhausted receives an annotation diagnostic.

### 5.3 Evidence priority and conflicts

Treat concrete evidence as a requirement, not an overload ranking. Independent argument and receiving types must agree on repeated generic parameters. An I32 argument does not become I64 because the destination expects I64. Numeric raw-literal context follows its separate receiving-before-peer-before-default policy.

Collect independently available sibling evidence before finalising dependent nested calls. Retain parsed expressions, parameter-slot routing, access modes, source spans and authored handling. Resume dependent work without parsing the arguments a second time.

Type requirements can be processed in a deterministic order different from runtime evaluation order. Generated request tickets and diagnostics retain authored attribution. Call evaluation, temporary lifetimes, mutations and error short-circuiting remain in source order.

Distinct generic parameters receive no equality merely because their limits match. A default value, a trait implementation, a possible overload, target availability or an optimisation opportunity does not select a type. Function defaults retain their existing typing and missing-argument rules rather than becoming new speculative inference seeds.

### 5.4 Scope boundaries

Keep these exclusions explicit:

- No body-inferred generic limits or callable return contracts.
- No generic function values, explicit call-site type arguments, higher-order polymorphism or user type functions.
- No general later-use inference for a result-only generic lacking a numeric origin.
- No backwards inference through arithmetic on named operands.
- No numeric family reclassification of an already bound origin.
- No type discovery from member availability, conformance search or conversion search.
- No inference across module bodies, template runtime machinery or target lowering.
- No type variable created to rescue an untyped empty collection at a currently prohibited source site.
- No source constructor retry as a different construct after a failed parse.

Expected type propagation follows success slots with the proper arity and structure. It does not merge an error channel into a success type, flatten multiple results into a tuple or fabricate a generic error type. Cast target-to-source flow stays blocked.

### 5.5 Completion and failure

Extend the delivered ready/retained call and known-shape machinery. Numeric origins remain numeric-origin state. A waiting nested call lacking an origin is a separate construction obligation waiting for concrete evidence, not an unrestricted numeric unknown or a fake I32 value.

A completion record needs only its resolved callee identity, retained arguments/slot mapping, independently known receiving facts, source/handling metadata and dependencies needed to resume it. Reuse existing records when their invariant fits. Avoid parallel call graphs or a second universal expression store.

Once types are selected, validate ordinary compatibility, numeric limits, trait evidence, cast policy and the declared success/error signature. Then complete the instance request or concrete binding target. No pending local ID enters a canonical type key, public interface, HIR or persisted artifact.

Preserve deduplication, recursion detection, generated sidecar ownership, source identity rebasing and atomic publication. A failed source or generated transaction leaves no partially published instance. Hardening must also cover chains of different instances and request growth, not only re-entry of the identical key.

Conditional generic error signatures are excluded. A declared `Error!` slot exists for every instance even when some implementations cannot fail. Source calls still use the ordinary propagation/recovery syntax. Private implicit numeric failure and public-boundary discharge keep their existing rules.

## 6. Type-parametric Core and Builder bindings

### 6.1 Public semantics and trust boundary

A trusted binding has one Moth-visible declaration identity, a semantic signature with linked parameters, explicit limits, access/return facts and a declared failure contract. Target implementation options are separate records.

The new mechanism admits numeric scalar parameters/results, including Dec as a semantic scalar. Concrete supporting slots and the existing uniform error channel are permitted. Generic container, callback, arbitrary aggregate and higher-kinded binding signatures are outside this delivery. Existing separately owned collection builtins remain intact.

Ordinary Moth-source algorithms can use numeric generics inside already supported collection and nominal shapes and call scalar Core operations. They do not require a generic foreign container ABI.

Builtin family names in registration refer to the same closed semantic owner used by source generic declarations. A builder cannot register a new family, change membership, override an operator or add an arbitrary predicate that the type solver executes.

### 6.2 Signature and implementation representation

The implementation should distinguish:

| Fact | Owner |
|---|---|
| Semantic signature patterns and declared limits | Compiler-owned binding declaration |
| Inferred concrete parameter mapping | Frontend call completion |
| Canonical concrete binding instance and effect contract | Compiler-owned resolved target metadata |
| Available target implementation and ABI adapter | Trusted provider/builder registration and target planning |
| JS/WAT file bytes, imports and artifact placement | Provider/build artifact owners |
| Instruction emission or runtime helper calls | Target lowerer |

Reuse existing signature slot/type constructors, canonical identities and substitution. A small typed pattern for references to declaration parameters is sufficient. Keep the restricted foreign ABI vocabulary separate from Moth semantic type patterns.

A concrete instance is keyed by canonical binding declaration, concrete type arguments and relevant semantic contract/evidence identities. Implementation hashes and ABI/feature versions belong in the physical compatibility facts. Local registry indices and absolute file paths are not persistent identities.

Resolve declaration and concrete types before target implementation lookup. Select implementations by stable IDs and validated type facts, never by parsing `min_i64` or a package name. Reject duplicate or overlapping mappings without order-dependent precedence. A missing target implementation is a capability diagnostic, not permission to select a different numeric type.

The first version uses explicit trusted mappings and closed shared classifiers. It introduces no source overloading, arbitrary specialisation language, WAT substitution macros or callable Rust hook for general type resolution.

### 6.3 HIR and generated work

A parametric binding instance need not materialise a Moth-source body or generate a source sidecar just to imitate source generics. Reuse the current resolved external/call-target path or extend it narrowly with the concrete instance identity.

Use an ordinary typed call when it carries all necessary semantics. Introduce a dedicated intrinsic operation only when existing calls cannot express a real ownership, control-flow or validation contract. Do not add one HIR statement variant per Math function or per target.

All result slots, error continuations, alias facts and exact numeric types are concrete at HIR handoff. Frontend generic validation and completed source specialisation use the same binding declaration contract. Backends receive no unresolved limit proof or source-level generic search.

### 6.4 Access, effects and values

The selected implementation must preserve exactly-once arguments, source order, short-circuiting and the declared result relationship. Inline templates cannot duplicate an effectful argument. Conversions, optional physical packing and error construction belong to explicit adapters or ordinary lowering, not textual substitution that changes source evaluation.

All four Math operations have shared source parameters. Value-result semantics do not imply a fresh unrelated allocation for every implementation. A Dec result may retain an operand representation or produce a new coefficient. Record the correct conservative provenance and retention facts through the existing ABI/memory owners. Never claim Fresh solely because the old F64 registrations did.

Trusted files are not automatically verified algorithms. Binary validation proves structural Wasm validity, not numerical accuracy, purity or ownership correctness. Registration contracts require runtime differential tests and the normal review of trusted implementation code.

Compile-time eligibility, if already supported for a binding, is an independent semantic capability. Preserve existing eligible/refused behaviour through resolved instance metadata. This plan does not broaden Math function constant evaluation or execute JS/WAT during compilation. A target implementation file is never a compile-time evaluator.

## 7. File-backed JS/WAT authoring

### 7.1 Shared signature semantics, separate host syntax

Reuse the existing `@moth.sig` signature grammar and its diagnostics through one host-neutral annotation-signature owner. JS and WAT retain separate lexical/structural parsing because their comments, strings, exports and modules differ.

An ordinary annotated foreign signature stays concrete. Numeric-family names and reusable generic names cannot appear as foreign value types. Trusted Rust registration maps concrete implementation signatures or explicitly declared physical adapters to one parametric public Core operation.

Existing JS `@moth.opaque` and `@moth.const` behaviour remains owned by their existing contracts. Sharing `@moth.sig` does not add WAT constants, generic foreign types, arbitrary JS exports or first-class type values. V1 WAT authoring is function-focused.

The semantic signature keeps positional parameters, existing access spelling, zero or one success slot and an optional final builtin Error! channel. Multiple machine results used by a registered error adapter are physical ABI details, not additional source success slots. Concrete supporting types are accepted only with a defined adapter for that profile.

The ordinary scalar WAT path must support I32, U32, I64, U64, F32 and F64. Bool and Char use their compiler-owned validation adapters if the existing scalar ABI supplies them. String, opaque, optional, collection, Dec and narrow-storage signatures require explicitly delivered adapters and are not added to the ordinary WAT profile merely because the parser recognises their names. A valid language type with no adapter receives a precise profile/capability diagnostic.

An Error! annotation cannot invent a physical error convention. Accept it only with a registered adapter implementing the existing result-channel ABI for that target. Otherwise report the missing capability. Do not assume a global last-error slot, a particular multi-value tuple or a memory layout from the comment alone.

### 7.2 Annotation association

Use the same Moth-facing name and signature body in both hosts. For example, a concrete JS implementation can carry:

```javascript
/** @moth.sig min_i32 |left I32, right I32| -> I32 */
export function minI32(left, right) {
    return left < right ? left : right;
}
```

The equivalent WAT authoring form is:

```wat
(module
  ;; @moth.sig min_i32 |left I32, right I32| -> I32
  (func $min_i32 (export "min_i32")
    (param $left i32) (param $right i32) (result i32)
    local.get $left
    local.get $right
    local.get $left
    local.get $right
    i32.lt_s
    select))
```

This is a concrete implementation export, not the public generic Math declaration. Trusted registration associates it with the I32 instance. An ordinary external import exposes only its annotated concrete name under the restricted profile.

For WAT, attach a signature annotation to the immediately following function declaration or explicit function-export declaration in the same module, allowing intervening whitespace and non-annotation comments. Inline export and separate `(export ... (func ...))` forms resolve to the same final function identity. Do not infer association from a similar function name. Diagnose a dangling annotation, a non-function target, conflicting duplicate annotations or ambiguous multiple exports rather than guessing.

Preserve raw byte spans when removing comment delimiters/prefixes. Handle CRLF, Unicode in comments and escaped string contents without character-count offsets. Annotation-like text inside strings or unrelated comments must not create exports. Use retained host syntax to resolve function/type-use/export identities. Do not write a regex-based WAT parser or rescan source during lowering.

### 7.3 WAT preparation and validation

Add WAT through the existing external import-provider and artifact channels, not as a Moth module parser. A `.wat` dependency resolves one source file and publishes a validated concrete binding surface. A file used as a resource remains under ordinary file-value rules rather than acquiring import semantics from its extension alone.

Use the existing Wasm tooling family. The research tree already depends on wasmparser and wasm-encoder. Add only the compatible `wat`/`wast` functionality needed for text parsing, retained spans and assembly, under the repository's dependency policy. Pin versions at implementation rather than copying the research tool version blindly. No new Wasm engine or handwritten instruction parser is required.

Preparation must:

1. Load source once through the provider-owned source/snapshot path.
2. Parse WAT and associate annotations using retained syntax/comment information.
3. Assemble the module and validate the actual binary with the selected explicit feature set.
4. Resolve annotated exports through actual function and type indices, including imported-function index offsets and explicit type uses.
5. Check source semantic signatures against registered physical ABI adapters.
6. Resolve declared imports against registered package/runtime capabilities.
7. Publish immutable export, adapter, dependency, byte/source-map and artifact facts only after validation succeeds.

Text assembly alone is not validation. Do not use all features accepted by a library default as the advertised Moth target profile. Unsupported proposals receive capability diagnostics. Ordinary WAT files contain a core module, not WAST scripts or components. V1 adds no WASI loader or unrestricted module resolver.

An authored module start function, unregistered host import or memory/table facility without the required registered lifecycle/capability is rejected. Initialisation is owned by the builder's normal lifecycle, not silently executed at compiler load time. Module-internal implementation structure may be accepted under validated registered capabilities, but exported non-function facilities do not become Moth values through `@moth.sig`.

### 7.4 Physical ABI and privileged adapters

Explicit trusted registration describes target export identity, physical parameter/result types, conversions, finite/range validation, access, return provenance, error delivery and required runtime/memory capabilities.

Wasm i32/i64 carriers do not encode Moth signedness. I32 versus U32 and I64 versus U64 remain distinct semantic signatures. Preserve the complete U64 range without a JavaScript Number detour. At a JS-hosted Wasm boundary, bit-pattern reinterpretation specified by the adapter is distinct from a source numeric cast or permission for wrapping arithmetic.

F32/F64 successful payloads undergo the shared exact-precision finite-value validation. Narrow and F16 trusted instances may use wider carriers only with an adapter preserving their actual semantic value constraints. Dec never becomes F64 to make an implementation available.

The normal external JS Number-based I64/U64 boundary retains its existing safe-integer restrictions. A trusted Core BigInt representation or a native Wasm i64 mapping is a separately registered profile, not an automatic widening of ordinary annotated JS privileges.

Rust registration describes and connects adapters. It need not contain the mathematical algorithm. Reuse shared conversion/error/runtime helpers rather than writing one bridge per public function. Memory-requiring helpers consume the compiler's validated memory/layout contract. They cannot hard-code internal pointers, assume an allocator, install a competing Moth runtime or retain Moth references through an ordinary non-retaining foreign profile.

### 7.5 Delivery before the later mixed-Wasm programme

The scalar WAT path must execute in an existing supported host lane. Preparing a file and then rejecting every use is not completion. At minimum, a source-level annotated scalar WAT dependency must run through generated project artifacts and the canonical runtime harness.

Prefer existing Wasm external-call/module integration when available. If the scheduled mixed backend is not yet available, implement the narrow HTML binding-asset route: a validated scalar WAT module is assembled to a Wasm asset, instantiated through builder-owned glue before its synchronous exports are used and connected through concrete adapters. This is foreign module consumption, not automatic partitioning of arbitrary Moth functions. Reuse existing runtime lifecycle and artifact owners rather than adding a second page bootstrap.

Trusted Core WAT helpers use the existing compatible lowerer/import route. Unsupported physical target/profile combinations remain diagnosed and recorded for later backend work. The plan does not require every Core instance to lower in both targets when its required representation or error ABI is absent.

No temporary always-JS implementation may satisfy a WAT execution test. Assert that the Wasm asset/export actually executes. Nor may this plan claim that JS-hosted WAT consumption implements general Moth mixed-target lowering.

### 7.6 Files, reachability and invalidation

File-backed implementations keep Core or Builder origin and their normal binding-backed identity. Changing source location does not change a Core operation into an ordinary user JS package or give users access to private helper exports.

Allow explicit registered helper dependencies across implementation files. This does not enable arbitrary JS import graphs or unrestricted filesystem/network import resolution for ordinary external packages. All actual imports must be declared, available and type-compatible. Missing imports, cycles outside the supported dependency policy and undeclared helpers are diagnosed.

Retain every source as an input/watch dependency according to normal graph rules. Executable reachability controls emitted assets and helpers, not whether an authored dependency is structurally valid. An unused malformed imported file remains a source/import diagnostic. A valid but unreachable unsupported runtime instance follows existing target-reachability rules.

Deduplicate emitted implementation files and concrete helper instances by stable identities. Include source content, dependency closure, semantic signature, adapter contract and target feature/ABI facts in the existing appropriate fingerprint domains. A helper-only edit must invalidate affected output even when its public signature is unchanged.

Source maps, error labels and runtime attribution identify the implementation source/export where appropriate, alongside the Moth call or annotation. Keep full TypeEnvironments out of retained render-only contexts. New source kinds must preserve locked span and diagnostic record sizes.

## 8. Core Math delivery and semantics

### 8.1 Mandatory public surface

The new trusted parametric mechanism delivers exactly these four generic functions:

| Function | Shared generic limit | Public signature relationship |
|---|---|---|
| min | NUMERIC | `(T, T) -> T` |
| max | NUMERIC | `(T, T) -> T` |
| clamp | NUMERIC | `(T, T, T) -> T` |
| abs | NUMERIC | `(T) -> T, Error!` |

All occurrences of T in one call mean one exact type. The functions do not promote different argument types to find a match. Arguments use shared access. They are not receiver methods or operator overloads.

The remaining existing Math functions and the six constants remain F64. The package identity, import forms and positional argument rules stay unchanged. Other package expansion, generic constants, decimal transcendental algorithms, additional precisions for those remaining functions and new public helper names are deferred.

The four functions are mandatory consumers of the file-backed implementation and parametric registration work, not handwritten parser special cases. Existing compact F64 registrations may stay where moving them gives no semantic or organisational benefit. A purely mechanical move of surrounding file-backed implementation code must preserve all existing function contracts and does not expand this plan's library scope.

### 8.2 Selection semantics

`min` and `max` select the numeric minimum or maximum under the existing same-type comparison contract. `clamp(value, lower, upper)` preserves the current composition `min(max(value, lower), upper)`. When lower exceeds upper, upper wins. Do not add a range-order error or make clamp fallible.

Use exact integer comparisons, including full I64/U64 values within their separate instances. Dec comparison uses exact coefficients at the selected scale. Selected values preserve their exact type and value without rerounding or reformatting through a float.

Finite binary floats remain finite. The existing Math contract does not specify the sign of a zero result, so this plan does not create a new cross-target signed-zero tie guarantee. Test permitted signed-zero outcomes and finite/type invariants, not accidental host tie-breaking. Nonzero representable selection results remain exact. This does not weaken the language's independent signed-zero arithmetic or storage rules.

A narrow storage type is fully supported when its representation is available. Selection must not widen U8 or F16 to a different source result type. Internal carriers remain private implementation details.

### 8.3 Absolute value and uniform failure

For every admitted type, abs returns the non-negative magnitude when representable in that same type. A signed integer minimum has no such magnitude in its own range and returns the existing appropriate builtin numeric overflow Error through the declared channel. Unsigned abs is identity. Finite float and exact Dec abs use their own representations without precision loss.

The declared Error! slot exists for every specialisation. An unsigned or floating instance being mathematically infallible does not remove the public slot or make an unhandled source call legal. Optimisation can remove unreachable physical work only under existing result/failure rules.

```moth
@core/math abs

Number = NUMERIC

magnitude |value Number| -> Number, Error!:
    return abs(value)!
;
```

Keep Error identity and numeric codes with their existing owners. No Math-only error carrier, implicit custom-error conversion or type-dependent signature is introduced.

Signed minimum failure is ordinary declared recoverable failure, not a Wasm trap used as a substitute. A missing target Error adapter must remain a capability diagnostic, even if a particular tested value would not fail. Declared error slots cannot disappear merely to fit a target ABI.

### 8.4 Required implementation coverage

HTML-JS must execute the four operations for each numeric representation already supported at activation, including the exact Dec family if it remains supported there. The source family is NUMERIC even on a target that lacks a representation.

WAT implementations must execute the directly supported scalar selection kernels. Extend abs only through an available registered result/error adapter. Exercise every other supported representation path, and retain precise rejection tests for unavailable Dec, F16 or error-delivery combinations. State the resulting matrix explicitly in the package progress row.

Numeric storage width alone does not require one source implementation file per width or one concrete registration per Dec scale. Share verified kernels or helpers where their semantics and adapters are identical. Instantiate on demand and keep canonical concrete types distinct.

Use source-authored generic wrappers in multi-file and separately registered source-package tests. This proves that ordinary Moth package authors can consume the binding contracts and that provider evidence, local generic names and target implementations survive publication.

Existing Math function constant evaluation remains governed by its separate accepted capability. This plan does not activate arbitrary Math-call folding, evaluate JS/WAT source at compile time or claim new transcendental accuracy.

## 9. Repository investigation and implementation owners

### 9.1 Research findings to refresh at activation

Inspection of the newly pushed main established these starting points. They are research observations, not a fixed future checkout requirement.

| Finding | Concrete inspected owner | Consequence for this plan |
|---|---|---|
| Exact numeric semantics are now on main and the old numeric plan is retired | Canonical numeric/generic references and roadmap | Consume delivered D15 and numeric-origin inference rather than reimplementing them |
| Generic calls have ready and retained lanes | `ast/generic_functions/calls.rs` | Extend `ParsedCallArguments`, retained calls and completion rather than reverting to eager calls |
| Source generic calls and nominal construction share numeric-origin linking | `ast/generic_functions/numeric_origins.rs` | Preserve shared known-shape substitution while removing argument-only nominal inference |
| Nested concrete expected-type forwarding is still explicitly excluded | `docs/src/docs/generics/generic-inference.mtf` and `GenericCallExpectedContext` | This plan owns the deliberate extension and corresponding negative-test conversion |
| Shared host signatures still use JS-specific parser data and diagnostics | `external_js/parser/signature_parser.rs` and `parsed_js_module.rs` | Extract the semantic signature layer while keeping host scanners local |
| Ordinary JS numeric annotations have a narrower adapter profile than Moth numerics | `packages/external-binding-contracts.mtf` | Preserve that boundary when trusted Core gains broader representations |
| Math is a homogeneous concrete F64 registration with inline JS | `builder_surface/core_packages/math.rs` | Replace only the four selected public signatures and their implementation path |
| External providers already carry stable asset identity and runtime imports | `builder_surface/external_import_providers/provider.rs` | Extend the existing artifact/provider route for assembled WAT |
| Wasm parser and encoder dependencies exist, but a WAT source-provider contract does not | `Cargo.toml` and external binding reference | Reuse tooling and add a small text front door, not a new engine or full linker programme |
| Roadmap placement after Never is before the mixed backend | `docs/roadmap/roadmap.md` | The WAT scalar delivery must not wait on that later programme |

Paths beginning with `ast/`, `datatypes/`, `headers/`, `hir/` or `public_interface/` below are relative to `src/compiler_frontend/` unless otherwise stated.

### 9.2 Owner map

| Area | Starting paths and responsibilities |
|---|---|
| Words and reservations | `crates/moth-lexical/src/{words.rs,identifier.rs}`, compiler `keywords.rs`, tokenizer word/type tags |
| Declaration grammar | `declaration_syntax/generic_parameters.rs`, declaration/type/function/struct/choice shells, `headers/` preparation and retained syntax |
| File visibility and local order | `headers/binding_environment/`, `module_dependencies.rs`, bound declaration tables |
| Generic parameter identities | `datatypes/generic_parameters.rs`, `datatypes/generic_bindings.rs`, `datatypes/environment.rs`, canonical type identity |
| Signature and nominal resolution | `ast/type_resolution/`, `ast/expressions/generic_nominal_inference.rs`, `constructor_views.rs`, `struct_instance.rs`, `choice_constructor.rs` |
| Calls and waiting work | `ast/expressions/{call_arguments.rs,call_argument.rs,call_validation.rs,source_function_calls.rs,function_calls.rs}`, `ast/generic_functions/{calls.rs,numeric_origins.rs}` |
| Numeric construction and shapes | `ast/numeric_construction.rs`, `ast/numeric_known_shape.rs` and their current submodules |
| Numeric operators and literals | `datatypes/numeric_operators.rs`, `ast/expressions/eval_expression/`, `ast/const_eval/`, `numeric_text/`, shared lexical numeric leaves |
| Traits and evidence | `traits/`, `ast/generic_bounds.rs`, generic receiver/bound-method resolution and core cast evidence |
| Template and generated ownership | `ast/generic_functions/{templates.rs,body_rules.rs,instances.rs,materialisation/}`, `module_compilation/generated/` |
| Published APIs | `public_interface/`, `semantic_identity.rs`, `canonical_type_identity.rs`, public call summaries and import projection |
| Binding metadata | `external_packages/{abi.rs,definitions.rs,registry.rs}`, canonical binding identity and capability validation |
| Host annotations | `src/projects/html_project/external_js/parser/{comment_extractor.rs,export_scanner.rs,import_scan.rs,signature_parser.rs,parsed_js_module.rs,binding.rs}` |
| JS provider/glue | `src/projects/html_project/external_js/{package_registration.rs,runtime_glue/}` and the existing provider entry point |
| Provider and asset contracts | `src/builder_surface/external_import_providers/{provider.rs,cache.rs,...}`, runtime package metadata and source registration |
| Core registration and implementations | `src/builder_surface/core_packages/math.rs`, `src/backends/js/package_bindings/`, JS numeric runtime and first-party JS inventory |
| Wasm consumption | `src/backends/wasm/`, `src/projects/html_project/` Wasm requests/glue and shared external target validation |
| HIR and memory | Dense HIR call/result storage, validation, reachability, return provenance, private-failure lane and validated physical memory/ABI owners |
| Build publication and reuse | `src/build_system/create_project_modules/`, generated store, provider artifacts, entry assembly, output manifests and dev-server watch/reuse |
| Correctness tests | `tests/cases/`, source generic tests, provider/parser tests, `src/compiler_tests/`, standalone lexical/MON suites |
| Performance | `benchmarks/`, `src/benchmarking/`, instrumentation, xtask scaling/validation and feature-lane metadata |
| Documentation | Generic/type/trait/cast/numeric references, developer authorities, external-binding/Core Math references, both matrices, cheatsheet, roadmap and index |

Refresh all paths at activation. Extend a moved owner rather than recreating an obsolete module at a familiar old path.

### 9.3 New responsibilities, not speculative structures

The expected new responsibilities are a reusable generic declaration record, a closed numeric-family classifier, bounded waiting-call completion, a parametric binding signature/instance record, a shared annotation-signature parser and a WAT provider with validated asset facts.

Before adding a file or type, find the existing owner that can absorb the responsibility. Suggested placements are within the compiler's declaration/generic/binding modules, a shared external-signature module below the provider clients and an HTML WAT provider beside the current JS provider. These are placement candidates, not a requirement to create six broad frameworks.

Keep family metadata and generic declaration syntax separate from ordinary constant values. Keep physical target mapping separate from semantic generic signatures. Keep host parsing separate from compiler inference. These boundaries are mandatory even if the chosen file layout differs.

## 10. Implementation sequence

### 10.1 Work-package policy

Each package has a producing owner, consumers, tests and deletion obligations. Add its primary failing regression examples before the implementation. Use focused checks during intermediate work and the broader gates only at the stated checkpoints.

The frontend syntax/identity/inference cutover touches many fixtures. Phases 1 through 5 form one coherent frontend acceptance checkpoint. Reviewable dependent commits are allowed, but an incomplete cutover is not a supported dual-language release. Do not keep the old grammar, skip tests or introduce compatibility APIs to advertise a false green checkpoint.

Package/provider work then builds on that frontend. The complete plan finishes with integrated source, package, WAT, documentation and performance gates.

| Phase | Deliverable |
|---|---|
| 0 | Activation, exact roadmap cleanup, authority publication and baseline inventory |
| 1 | Reusable declaration shells, exact-case names and file-local scope |
| 2 | Owner-local generic identity, explicit nominal `of` and public projection |
| 3 | Numeric-family operator and literal validation |
| 4 | Bounded nested-call context and concrete completion |
| 5 | Constructor receiving cutover, trait/receiver migration and frontend gate |
| 6 | Trusted scalar-parametric binding contracts and concrete instances |
| 7 | Shared annotation semantics and file-backed JS infrastructure |
| 8 | WAT preparation, explicit adapters and scalar execution |
| 9 | Generic Core Math consumers and cross-package use |
| 10 | Adversarial, publication and regression hardening |
| 11 | Performance, permanent documentation, downstream handoff and retirement |

### Phase 0 - activate and establish the authoritative baseline

**Consumes:** the serial activation decision, current main and this approved specification.

**Produces:** one isolated implementation tree, permanent approved contracts, an owner/test ledger and an evidence baseline.

- [ ] Read the current repository rules and routed authorities. Verify that the prerequisite capabilities are actually delivered at activation, distinguishing accepted design from implementation status.
- [ ] Establish the working-tree, branch, toolchain and effective feature/parallelism configuration. Preserve unrelated uncommitted work and accepted earlier checkpoints. Record the activation revision in untracked working notes.
- [ ] Confirm the linked queued entry is immediately after Never return contracts. Mark it active there only after activation, without moving it to a separate heading or reordering surrounding work.
- [ ] Remove the exact old `## Generics, type-parametric Core bindings and file-backed JS/WAT` placeholder section. Stop deletion at the next same-level heading. Preserve the optional catch advisory and all unrelated notes.
- [ ] Transfer the placeholder's D15, numeric-origin, explicit-limit and exclusion facts into the permanent generic/design-scope/architecture references. Keep the existing `Numeric and generic extensions` heading as the stable handoff.
- [ ] Publish Sections 3 through 8 into the appropriate permanent semantic owners. Mark implementation support separately. Replace contradictory old exclusions only within the approved scope.
- [ ] Inventory every old generic introducer, multiple-bound use, constructor-only inference case, generic public projection and annotated signature path. Separate ordinary `type`/`is`/`and` text from retired grammar.
- [ ] Create the assertion-level migration ledger from Section 11. Include syntax, imports, defaults, mutable arguments, optional/error results, constructors, retained generic semantic contexts, numeric origins, MON and first-party package tests.
- [ ] Record current JS/Wasm success and rejection matrices. Identify the minimum memory-independent WAT execution route so no dependency on later mixed-Wasm work is introduced.
- [ ] Capture the current integration inventory and targeted baseline suites. Record failures honestly rather than changing expectations.
- [ ] Capture matched non-recording benchmark runs for representative generic, numeric, provider and mostly concrete programs. Keep new feature workloads separate from equivalent old/new source cohorts.

**Review gate:** an explicit contract-to-test map, no orphaned placeholder information and no circular prerequisite. Source changes beyond the agreed specification require user approval, not an agent's silent adjustment.

### Phase 1 - reusable declarations and lexical scope

**Primary owners:** shared word/identifier policy, declaration shells, header preparation, file visibility and local ordering.

**Produces:** immutable file-local generic declaration descriptors with exact spans and resolved declaration roles.

- [ ] Add positive and negative source cases D01-D14 from Section 12 before implementing the grammar.
- [ ] Add `#Type` and single-limit declaration forms through the ordinary retained declaration syntax owner. Parse the source once. Do not turn the generic initializer into a general constant expression.
- [ ] Retain a typed distinction between a reusable generic declaration, a concrete type/alias, a trait and an ordinary constant/value. Diagnose misuse at the correct source position.
- [ ] Enforce file scope, compile-time-only mode and zero/one limit. Reject mutable forms, Type without `#`, lists, conjunction, alternatives and generic copying/aliasing.
- [ ] Recognise exact-case family names without introducing case-folded reservations. Integrate `DEC` with its existing reservation rules and keep `Decimal`/`DECIMAL` free from new reservation.
- [ ] Exclude generic declarations from dependency-bindable/exportable file surfaces. Add same-module sibling imports, explicit export and namespace-selection rejection tests.
- [ ] Resolve forward metadata references with retained header/order facts rather than a second discovery scan. Preserve ordinary constant source-order rules separately.
- [ ] Remove the retired signature-local `type` parser/reservation path once consumers migrate. Preserve `is`, `and` and `of` uses outside the replaced grammar.
- [ ] Update source-word highlighting and shared identifier tests, including standalone MON consumers where the lexical inventory is shared.

**Validation:** declaration/word/visibility tests and real source fixtures. Confirm that no runtime statement, constant payload or bogus TypeId is produced for a generic declaration.

### Phase 2 - owner-local parameters, nominal order and public interfaces

**Primary owners:** type environment, signature resolution, nominal construction metadata, canonical identities, public projection and generated templates.

**Produces:** canonical per-owner parameters with explicit limits, stable nominal argument order and correctly projected public contracts.

- [ ] Add independent-owner, repeated-name and multi-instantiation tests before changing parameter registration.
- [ ] Build function parameter lists from declared signature references only. Use stable first-occurrence ordering and preserve ordinary type/return/error/access structure.
- [ ] Register each owner-local parameter independently of the reusable file-local declaration. Keep the declaration's source link for diagnostics without using it as the substitution identity across functions.
- [ ] Implement `StructName of GenericNames = ...` and `ChoiceName of GenericNames :: ...` using previously declared generics. Diagnose missing, duplicate, non-generic and unused list entries.
- [ ] Preserve explicit positional type application independently of field/variant order. Do not infer nominal list order from body scanning.
- [ ] Carry limits into public interfaces, canonical generic keys, receiver contracts and imported template materialisation. Exported generic APIs must not export their reusable declaration symbols.
- [ ] Check private-trait leakage independently from reusable-generic privacy. Preserve ordinary visible evidence and source-private implementation closure.
- [ ] Update aligned receiver binding to the nominal parameter contract. Reject extra receiver generics, concrete-instance methods and incompatible limit alignment.
- [ ] Preserve generic body annotations and reject body-only generic parameter creation. Do not interpret an ordinary local annotation as a new generic owner.
- [ ] Update fingerprints so parameter relationships and resolved limits are semantic. Local generic spelling and diagnostic spans must not become arbitrary backend identity inputs.
- [ ] Rebase and round-trip interfaces through independent source-package contexts with colliding names and local IDs. Failed projection leaves no partial published interface.

**Validation:** D15-D22, P01-P10, existing alias/nominal/receiver tests and generated-sidecar remap tests. Check field reorder versus parameter order with an observable construction/access result.

### Phase 3 - numeric families, operators and generic literals

**Primary owners:** compiler generic-limit classifier, canonical operator table, template body validation, numeric text/materialisation and trait-bound checking.

**Produces:** universally validated symbolic numeric operations and literal recipes with no inferred limit narrowing.

- [ ] Add the family inventory, exact-case and subset-forwarding tests. Canonicalise Dec/Dec0 once and preserve Byte/Char/Bool exclusion.
- [ ] Implement a closed typed limit representation that distinguishes a builtin family from a canonical user trait. Use one membership/implication owner across source and trusted bindings.
- [ ] Check operator admissibility and its uniform result relationship against the authored family. Produce an existing parameter or concrete result, never a public computed type.
- [ ] Validate all intermediate operations. Reject mixed independent generic widths where no uniform result can be represented, even if a later comparison returns Bool.
- [ ] Preserve D15, integer real division, Dec scale/exponent rules, exact comparisons and per-operation F16 completion. Reuse the actual operator owner rather than a second approximate table.
- [ ] Validate raw literals against every admitted type using shared lexical/numeric rules. Retain spelling for selected-type materialisation and preserve direct F16/F32 rounding.
- [ ] Distinguish raw literals from typed constants and prior bindings. Reject an out-of-family literal at declaration time without shrinking the limit.
- [ ] Permit ordinary Bool combinations of comparison results without creating Boolean polymorphism or truthiness.
- [ ] Preserve ordinary trait method/evidence behaviour. No user conformance to numeric families, implicit primitive trait declarations or generic cast targets.
- [ ] Add targeted counterexample diagnostics for unsupported operations, invalid result relationships and family-wide literal failure. Point to the declaration, limit and offending source operation where useful.

**Validation:** L01-L24 and the preserved numeric suites. Compare each supported concrete specialisation's result type and behaviour with the corresponding ordinary concrete expression. Template validation must not instantiate every member.

### Phase 4 - concrete expected context and sibling inference

**Primary owners:** shared argument routing, generic calls/numeric-origin linking, pending completion, known shapes and generated request publication.

**Produces:** bounded order-independent completion of nested calls using independent concrete evidence.

- [ ] Add I01-I18 with both sibling orders and real runtime side effects before extending inference.
- [ ] Separate waiting for concrete expected context from numeric-origin state. Reuse retained call arguments and known structure without creating unrestricted inferred types.
- [ ] Collect ready sibling type facts and an independently known receiver, apply declared signature relationships and enqueue only dependent calls whose evidence changes.
- [ ] Forward the resulting concrete expected success type into the inner call with exact arity/shape. Repeat through nested declared relationships without examining callee bodies.
- [ ] Finish parameter compatibility, limits, ordinary trait evidence and error handling after the relevant types are selected. Concrete conflicts cannot trigger a type search or implicit conversion.
- [ ] Preserve numeric-origin later-use inference and the no-backwards-named-arithmetic rule. Keep cast target-to-source barriers and typed-binding families unchanged.
- [ ] Keep source evaluation order, authored propagation locations, receiver access, temporary lifetimes and error short-circuiting independent of analysis order.
- [ ] Complete each retained argument once. Remove obsolete eager rejection/scheduling paths rather than adding retry parsing.
- [ ] Preserve source request tickets, canonical instance keys, deduplication and failed-transaction rollback. No pending record or local inference ID may cross publication.
- [ ] Bound unresolved cycles and long dependency chains using the current resource policy and a deterministic annotation diagnostic. Do not search candidate specialisations to break them.

**Validation:** source calls, generic and non-generic outer callees, imported instances, return-only nested calls, fields and conditional result contexts, exact-once runtime traces and operation-count scaling probes.

### Phase 5 - contextual nominal construction and frontend migration gate

**Primary owners:** constructor receiving sites, fields/defaults/choices, ordinary call validation, generic fixtures and documentation examples.

**Produces:** one final generic source surface with required nominal receiving applications and complete migrated frontend coverage.

- [ ] Add N01-N16 before removing argument-only nominal inference.
- [ ] Require a known explicitly applied nominal receiver before accepting generic construction. Permit independently resolved parameter/return/field/element contexts and symbolic applied receivers in valid generic bodies.
- [ ] Remove the path that selects a generic nominal instance solely from its constructor arguments. Retain shared argument routing, default checking and known-shape completion used by legitimate contexts.
- [ ] Preserve empty variants and empty containers with explicit types. Keep constructors distinct from aliases and from generic function calls returning concrete instances.
- [ ] Ensure nested constructors can wait for independently resolved sibling/outer context without using their own argument types as the forbidden fallback.
- [ ] Migrate old function, struct, choice and receiver syntax through production-generated snippets, test sources, source packages, examples, docs and benchmark generators.
- [ ] For every removed constructor-inference fixture, retain its other contract assertions with explicit receiving types and add the corresponding intended rejection owner.
- [ ] Recheck optional/multi-result calls, generic error parameters, cast evidence, public privacy, same-file method conformance and inactive validated bodies.
- [ ] Run the integrated frontend cutover gate, including the assertion-level ledger and relevant feature lanes. Remove old parser entry points, documentation and transitional metadata.

**Exit:** all new declaration/operator/call rules work through the real frontend, every old syntax occurrence has a classified disposition and no dual generic language remains.

### Phase 6 - trusted parametric binding declarations

**Primary owners:** external package declarations, canonical binding identity, call completion, target validation and call/effect lowering.

**Produces:** semantic parametric binding declarations and concrete instances sharing the normal frontend contracts.

- [ ] Add B01-B14 using test-only registered bindings before modifying Core Math.
- [ ] Extend the semantic binding signature with linked numeric parameters and explicit limits. Reuse source slot/type relationships rather than broadening the physical foreign ABI enum into a second Moth type system.
- [ ] Resolve concrete instances through existing generic/known-shape machinery. Keep one family classifier and one compatibility/evidence policy.
- [ ] Separate semantic instance identity from target implementation and adapter identity. Validate duplicate declarations, malformed mappings and missing implementations deterministically.
- [ ] Carry access, result provenance and uniform error slots into ordinary concrete HIR calls/invokes. Reuse external target paths before adding a new intrinsic representation.
- [ ] Preserve stable package identity through aliases, re-exports and independently compiled source generic wrappers. Do not special-case direct `@core/math` spelling in typing or code generation.
- [ ] Keep ordinary annotated bindings concrete and restricted. Test that user annotations cannot claim a trusted profile or numeric family parameter.
- [ ] Test typed error handling and a mathematically infallible instance of a uniformly fallible binding. Source signatures remain unchanged after optimisation.
- [ ] Propagate existing const-evaluation capability/refusal metadata without adding target-source evaluation or arbitrary callbacks.

**Validation:** compiler-registered binding tests, concrete call/return facts, target rejection, canonical interface transport and source-level wrappers across packages.

### Phase 7 - shared annotation semantics and file-backed JavaScript

**Primary owners:** current JS annotation parser, host-neutral signature owner, provider artifacts, trusted registration, runtime glue and first-party dependency inventory.

**Produces:** one annotation-signature parser and an explicit file-backed implementation path reusable by Core and ordinary providers without mixing trust profiles.

- [ ] Inventory the current parser, source spans, annotation/export association and exact JS subset before extraction. Preserve `@moth.opaque` and literal-U32 `@moth.const` behaviour.
- [ ] Add paired signature-body tests for the intended shared grammar and diagnostics. Extract common parameter/return syntax, type-name facts and error-slot validation without duplicating host scanners.
- [ ] Replace JS-specific diagnostic/source wrappers only where the shared owner requires it. Preserve exact JS source spans and existing externally visible reasons.
- [ ] Retain or improve byte-based positions for multibyte source and CRLF. Do not copy complete source buffers merely to associate each annotation.
- [ ] Register file-backed implementation exports with explicit semantic/physical adapter profiles. An ordinary import of the same source does not inherit Core privileges.
- [ ] Extend registered helper dependencies and emitted asset deduplication through existing metadata. Keep module discovery, source validation and output reachability distinct.
- [ ] Preserve validation of Number-based foreign integer adapters, BigInt trusted mappings, F32/F64 ingress, Error payloads and exactly-once snapshots.
- [ ] Update first-party dependency audit inputs and packaging so shipped files are available outside the repository checkout without absolute-path assumptions.
- [ ] Remove replaced inline algorithm strings and duplicated parser paths only when their consumers have moved. A small declarative registration table remains appropriate.

**Validation:** existing external JS parser/provider/runtime suites, A01-A14, dependency/asset invalidation tests and a concrete file-backed Core test binding.

### Phase 8 - WAT source provider, adapters and runtime path

**Primary owners:** WAT provider, shared annotation parser, wasmparser/encoder tooling, runtime asset/glue owners and trusted physical mapping.

**Produces:** validated annotated scalar WAT imports that execute through the supported project path, with explicit unsupported-capability boundaries.

- [ ] Select compatible Wasm text tooling under the dependency policy. Record why reuse of the established tooling is preferable to a bespoke lexer/parser or external CLI requirement.
- [ ] Add W01-W22 for valid text, annotation association, signature/index validation, feature restrictions, imports, spans and malformed input.
- [ ] Parse and retain the WAT structural/annotation information once, assemble the binary and perform explicit binary validation before publishing a provider surface.
- [ ] Map inline and separate function exports, named/numeric indices, explicit type uses and imported function offsets correctly. Resolve physical signatures from validated facts.
- [ ] Validate ordinary concrete signatures against exact registered adapters. Reject unsupported value shapes and undeclared error conventions without inventing a representation.
- [ ] Resolve registered imports and helper files explicitly. Reject implicit runtime facilities, unregistered start/memory/table lifecycle work and unsupported module kinds.
- [ ] Implement the memory-independent scalar execution path described in Section 7.5. Integrate asynchronous module preparation into the existing builder lifecycle while preserving synchronous source calls after readiness.
- [ ] Preserve signed/unsigned i32/i64 bit interpretation, full U64 values, finite floats and ordinary error/integrity distinctions through the selected adapter.
- [ ] Retain byte/source maps, artifact ownership, output conflicts and dependency invalidation. Register failure paths atomically and emit no partial package after a bad annotation/module.
- [ ] Test multiple imports of the same module, helper-only edits, removed exports and unused valid assets. Assert actual Wasm execution rather than only validating bytes.
- [ ] Record source-level and direct-Wasm lowerer coverage separately. Missing managed representations or error adapters remain precise target rejections and downstream work.

**Exit:** at least the directly supported scalar WAT subset runs from real Moth imports and project artifacts. The implementation has neither a hidden dependency on the later mixed backend nor a claim that general mixed-target support has been delivered.

### Phase 9 - generic Core Math consumers

**Primary owners:** Core Math registration, new file-backed helper sources, shared numeric/runtime policy and concrete binding instances.

**Produces:** generic min/max/clamp and uniformly fallible abs with the supported runtime matrix.

- [ ] Add M01-M20 covering exact values, all numeric families, distinct type conflicts, error handling and target rejections before changing the public signatures.
- [ ] Register one public declaration per selected operation with NUMERIC and the approved linked type/result contract.
- [ ] Move the selected algorithms into organised JS/WAT files with explicit trusted mappings. Keep semantic ownership in compiler/package metadata rather than deriving it from annotation names.
- [ ] Implement exact selection for Number/BigInt/Dec representations without precision-loss conversion. Preserve clamp's upper-wins behaviour for reversed limits.
- [ ] Implement same-type abs with signed minimum overflow returned through Error!. Preserve the declared error slot for unsigned, floating and Dec instances.
- [ ] Reuse shared Error code and value construction. Test preservation of Error identity, code and message rather than creating a package-local carrier.
- [ ] Specify and validate result provenance/retention for Dec, including results that outlive their argument bindings. Preserve normal access and resource-exhaustion rules.
- [ ] Use existing target capabilities for WAT instances and reject unavailable representations/error adapters honestly. Keep all remaining Math functions/constants F64.
- [ ] Add source-authored generic wrapper packages that exercise the declarations, family implication, nested-call context and public instance metadata rather than only direct Core calls.
- [ ] Convert all existing abs call sites with explicit handling. A type-specific successful path does not excuse an omitted public error handler.
- [ ] Retain existing Math tests for the other functions, constants, imports, numeric integrity and unsupported targets. Move useful assertions out of replaced fixtures before removing them.

**Validation:** real JS and WAT execution, exact scalar and decimal witnesses, retained old F64 numerical contracts and multi-package use. Record the explicit supported/rejected instance matrix.

### Phase 10 - hardening and completeness review

**Consumes:** the integrated frontend, binding and WAT implementation.

**Produces:** adversarial evidence and a closed migration ledger.

- [ ] Complete every acceptance row in Section 12 with a primary owner and observed result.
- [ ] Exercise declaration reorder, same-named generics in different files, imported method evidence and public aliases/re-exports with independent identity domains.
- [ ] Test all accepted/rejected syntax in required constant/default, inactive branch and unreachable handler contexts. Validated dead code must not become unvalidated code.
- [ ] Test late failure-summary convergence and dead-handler pruning without leaking generic requests, helpers, target requirements or old proof facts.
- [ ] Add long waiting-call chains, sibling fan-out, ambiguous cycles and many independent generic bodies. Confirm deterministic completion and bounded storage without candidate enumeration.
- [ ] Stress malformed WAT/comments/imports, oversized source/annotation structures and mismatched binary signatures through existing resource/diagnostic policy. Include non-ASCII prefixes and CRLF.
- [ ] Check source graph input validity independently of runtime output liveness. Dead code cannot conceal malformed imported source or undeclared imports.
- [ ] Verify target rejection never becomes silent JS fallback, a trap substitute or an unchecked numeric conversion.
- [ ] Run source and artifact scans for old syntax, generic copying/list support, duplicate parsers, hardcoded Core mappings and stale public metadata.
- [ ] Apply the Slice review and relevant guide to each changed owner. Delete redundant wrappers, unused state, superseded tests and obsolete implementation paths.

**Exit:** the full contract has positive and negative coverage, every retained identity is correctly owned and all remaining limitations have explicit owners.

### Phase 11 - validation, permanent handoff and retirement

- [ ] Execute the performance protocol in Section 14 with matched equivalent and new-feature cohorts. Preserve prior measured budgets and report any new cost separately.
- [ ] Finish the permanent documentation work in Section 13 and build generated documentation through the compiler. Inspect rendering of the new source forms and WAT code blocks.
- [ ] Update both progress matrices with actual language, binding and target support, including deliberate constructor rejection and remaining WAT representation limits.
- [ ] Record later Core-library opportunity work in the living package programme/Math owner without expanding this delivery. Keep generic constants, broader numeric families and container bindings deferred.
- [ ] Update later backend, MON integration, layout, package and tooling owners to consume the new contracts without reimplementing them or depending on this temporary plan's wording.
- [ ] Run `just validate-full` and any required affected lanes outside it on the final integrated tree with the pinned tools. Rerun invalidated evidence after final changes or rebase.
- [ ] Complete the final Slice review, search-based closure and assertion-level ledger. Distinguish current support from tests that were not run.
- [ ] Transfer all unique accepted decisions into permanent owners. Delete this one-shot plan and its roadmap entry in the completion commit. Preserve living package follow-ups and historical validation evidence.

**Completion is not activation of later work.** It neither resumes the mixed backend nor adds future library APIs automatically.

## 11. Coverage migration protocol

### 11.1 Preserve semantic assertions through the syntax cutover

Create an assertion-level ledger under `tmp/generics/coverage-ledger.md` before editing existing fixtures. Include subsystem tests, generated source strings, public crate tests, integration cases, documentation examples and benchmark inputs. A fixture name or passing test count is not evidence that its original assertion survived.

| Ledger field | Required content |
|---|---|
| Original owner | File, case/test identity and inspected revision |
| Original contract | The value, type relationship, failure, source span, visibility or artifact property being asserted |
| Original configuration | Target, features, source/package layout and runtime versus constant path |
| Disposition | Preserve, adapt, split, consolidate, deliberately retire or retain as a capability rejection |
| Replacement | Exact test/case and assertion owner |
| Decision | The rule in this plan that justifies an intentional behaviour change |
| Evidence | Commands, executed scope, resulting tree and observed outcome |

Migrate existing generic syntax to explicit reusable declarations without weakening its relationships. A formerly local `type T` parameter becomes a file-local reusable declaration plus an owner-local parameter established by its signature. Use fresh names where one file's older functions declared unrelated same-spelled parameters with different limits. Never merge their identities merely to minimise line changes.

A fixture that previously succeeded through argument-only nominal inference changes in two ways. Preserve its original constructor, field, default, ownership or backend assertion using an explicit applied receiver, then add or identify a negative case for the now-rejected inference form. Replacing a whole successful construction test with a syntax rejection loses its previous coverage.

Existing multiple-trait-limit tests need an explicit disposition. V1 rejects that source form. Preserve independently useful method/evidence assertions through separate single-limit helpers or an explicitly authored ordinary trait containing the required methods when that is a legitimate equivalent test. Such a trait needs normal explicit conformances. Never silently drop a required method, invent a composite compiler family, infer conformance structurally or leave multiple limits working through a parser accident.

Keep tests for traits, `is` comparisons, Boolean `and`, concrete type aliases and ordinary compile-time `#` declarations that are unrelated to the removed generic syntax. A word search is a locator, not permission for a global replacement.

### 11.2 High-value existing owners

Refresh exact case IDs from the active manifest. These inspected paths and families are starting points rather than a complete future inventory.

| Existing owner or family | Migration obligation |
|---|---|
| `tests/cases/generic_fn_facade_private_impl_nested_success/` | Preserve nested private helper materialisation through a facade, with new file-local generic declarations in each provider file |
| `tests/cases/generic_fn_facade_direct_selection_success/` and its alias/namespace siblings | Preserve stable public callable identity independently of local import spelling |
| `tests/cases/generic_fn_facade_expected_result_collection_success/` | Preserve result-directed inference with an explicit concrete collection receiver |
| `tests/cases/generic_receiver_source_package_facade_success/` | Preserve aligned receiver parameters and package-owned nominal identity |
| `tests/cases/generic_fn_facade_private_receiver_nested_success/` | Preserve private receiver evidence and public trait visibility rather than importing reusable generic names |
| Generic constructor, named/default argument and concrete alias cases | Supply applied receivers while retaining routing, omitted-default and nominal identity assertions |
| Generic parameter conflict, body validation and value-use rejection cases | Keep typed reasons, both relevant source locations and the distinction between a declaration and an executable instance |
| `src/compiler_frontend/ast/generic_functions/tests/` | Preserve frozen syntax, canonical evidence, donor contexts and declaration/call-site diagnostic ownership |
| `src/build_system/tests/compile_project_frontend_tests/package_materialisation_tests.rs` or its moved test owner | Preserve real source-package compilation, nested request convergence, colliding source identities and failure-summary evidence |
| Numeric-origin and known-shape tests | Preserve late literal resolution, source-ordered receiver witnesses and the directed arithmetic barrier |
| `src/projects/html_project/external_js/parser/tests/` | Transfer shared signature tests without losing host-specific comment/export rejection coverage |
| External JS registration, runtime glue and import-provider tests | Preserve ordinary foreign carrier limits, error integrity, once-only evaluation and asset identity |
| `tests/cases/core_math_basic_functions/` and transcendental siblings | Adapt the four changed functions without losing unrelated F64 numerical, domain and approximation assertions |
| Math constants, namespace, arity, type and finite-boundary cases | Keep the six F64 constants and existing boundary semantics, including checks no longer selected by old default-type identities |
| Existing abs use sites | Add authored propagation or recovery and preserve their numerical purpose, rather than hiding the new error channel in a wrapper |
| Standalone lexical and MON suites | Preserve independent literal-data behaviour while shared reserved-name/highlighting policy changes |
| `src/first_party_js/`, dependency guards and helper-inventory tests | Register new implementation files and their actual imports, with no unlisted third-party runtime dependency |
| Benchmark manifests and source generators | Preserve equivalent work when replacing syntax, while measuring new inference and binding work separately |

The repository's test location rules govern moved tests. Do not create new multi-module pipeline tests under production owners merely because an older inspected test resides there.

### 11.3 Assertions and execution

Use real Moth integration programs for language and package behaviour. Use subsystem tests for invisible invariants such as parameter ownership, failed transaction rollback, compact range validity and absence of provisional IDs in durable metadata.

Keep runtime witnesses dynamic. A folded `abs` or selected constant does not test a WAT function, an Error adapter or a runtime branch. Assert that the relevant implementation survives into the artifact and executes, then assert its value or error behaviour.

Preserve distinct outcomes: a source diagnostic, target-capability diagnostic, recoverable Error, fatal integrity failure and WebAssembly trap are not interchangeable. An expected rejected target is useful coverage when the corresponding contract is genuinely unsupported. It is not a substitute for the required scalar execution path.

Use exact integer/decimal values and binary bit patterns where meaningful. Tolerances belong to genuinely approximate unchanged Math operations, not min/max/clamp or integral abs. Preserve Math's documented unspecified zero-sign policy instead of choosing one backend's tie result as the new universal contract.

Use the existing exact, ordered and exact-once output assertions to catch repeated arguments, reordered calls, lost mutations and eager fallback evaluation. Retain negative tests for unused generic declarations with invalid limits, invalid public contracts and malformed imported implementation files even when runtime code is unreachable.

Group related matrix rows under a clear primary contract. Secondary coverage is justified by a different semantic stage, target or hidden invariant. Do not add one largely redundant fixture per table row. Run the suite inventory before and after migration and investigate lost assertions and newly weakened expectation modes.

## 12. Mandatory acceptance matrix

The matrix records required outcomes, not a count of new fixtures. Each row needs a primary owner and disposition in the coverage ledger. Example function names below stand for real declarations with the shown signatures, not new builtins. New source examples use the syntax in Sections 3 and 4.

### 12.1 Declaration grammar and reusable identity

| ID | Case | Required result |
|---|---|---|
| D01 | `Item #Type` | One unbounded file-private reusable generic declaration, no runtime/constant value |
| D02 | `Number = NUMERIC` | Infer the declaration category from one valid limit name |
| D03 | `Number #Type = NUMERIC` | Same limit semantics as D02 with explicit compile-time annotation |
| D04 | `Item Type` or `Number Type = NUMERIC` | Compile-time-only diagnostic with the `#Type` correction |
| D05 | A declaration with no limit and no `#Type` | Ordinary missing/invalid declaration diagnostic, never an implicit generic |
| D06 | Limit given as a concrete type, `none`, call, template or arbitrary expression | Reject non-limit initialisation rather than evaluate a type value |
| D07 | Two limits separated by comma, `and` or `or` | Reject the one-limit V1 violation without a partial accepted declaration |
| D08 | Copy/alias attempts involving a reusable declaration | Reject `A = Number` and generic `as` forms, preserving concrete aliases separately |
| D09 | Generic declaration inside a function, template, config, field or export group | File-scope/file-private placement diagnostic with useful source context |
| D10 | Mutable generic declaration or later assignment | Reject runtime/mutation semantics, no reassigned metadata |
| D11 | Family spelling and ordinary names in other cases | Exact uppercase family recognition, no new lowercase/title-case reservation |
| D12 | `DEC`, `Dec`, `Decimal` and `DECIMAL` | Correct family/type distinction, existing Dec protection retained, latter names not newly reserved |
| D13 | Duplicate reusable names and collisions with visible types/traits | Existing no-shadowing rules and both declaration locations |
| D14 | Old function `type` introducer and old bound list syntax | Deliberate removed-syntax diagnostics, without changing `is`/`and` elsewhere |
| D15 | One generic name repeated in two parameters and the result | One shared owner-local parameter, equal exact types required per call |
| D16 | Two freshly declared names with identical limits | Independent parameters, equal and different concrete choices both permitted |
| D17 | One reusable name used in two different functions | Independent parameter owners and independent specialisations |
| D18 | One function called at two exact numeric types | Separate concrete instances, declaration remains non-concrete |
| D19 | A third generic name only in a return slot | Independent return parameter, not a computed common type |
| D20 | Generic referenced only inside a body, absent from its signature | Reject undeclared body parameter rather than change the public signature |
| D21 | Valid reusable declaration placed away from or after its consumer | Header/local ordering resolves metadata without an adjacency rule |
| D22 | Generic declarations in a support module or ordinary source file | No synthetic runtime activity and no leaked dependency-bindable generic symbols |

### 12.2 Public contracts and cross-package ownership

| ID | Case | Required result |
|---|---|---|
| P01 | Exported function using a file-private numeric generic | Publish callable-owned parameter and limit, not the reusable symbol |
| P02 | Import/select/re-export the reusable generic itself | Reject even between files of one module |
| P03 | Two files both declaring `Number = NUMERIC` | No collision or shared substitution across their file-local declarations |
| P04 | Public callable with a private ordinary trait limit | Public-surface leak diagnostic, no concealment through generic privacy |
| P05 | Imported public trait used as a local generic's single limit | Ordinary evidence visibility and explicit conformance rules |
| P06 | Facade aliases and chained re-exports of generic callables/nominals | Preserve canonical owner, parameter order and limits independently of spelling |
| P07 | Provider/requester files with colliding local TypeIds and source names | Rebased canonical identities and correct donor/call diagnostic labels |
| P08 | Public body-only edit versus declared limit edit | Body-only change preserves signature facts, limit change affects the public contract |
| P09 | Failed source or generated materialisation | No partial public interface, reusable symbol export or sidecar publication |
| P10 | Aligned generic receiver through a package facade | Nominal-owned parameters and evidence remain aligned, no extra method parameter owner |

### 12.3 Numeric limits, universal operator typing and literals

| ID | Case | Required result |
|---|---|---|
| L01 | Complete membership of all eight named families | Exact approved fixed types/scales and alias identity, no Byte/Char/Bool leakage |
| L02 | User attempts to implement, redefine or extend a numeric family | Reject without structural or method-shape conformance |
| L03 | Unbounded generic used in arithmetic/ordering | Missing explicit limit diagnostic, not a body-inferred family |
| L04 | One NUMERIC parameter used twice by `+`, `-` or `*` with result T | Validate the uniform T relationship and normal possible failure |
| L05 | Same INTEGER parameter with `/` | Concrete F64 result, rejecting a promised T result |
| L06 | Same INTEGER parameter with `//`, `%` or `^` | Uniform T result with established divisor/exponent failures |
| L07 | Same FLOAT parameter with `/` and attempted `//` | T real division succeeds, integer division rejects |
| L08 | Same DEC parameter with `/` or `//` | Reject universally invalid family operation, accounting for scale zero versus positive scales |
| L09 | Same DEC parameter with `%` and with exponent I32 | Preserve scale-specific remainder and approved I32 exponent result relationship |
| L10 | Same NUMERIC parameter with unary minus or same-type exponentiation | Reject where admitted unsigned/decimal members violate the required operation contract |
| L11 | Same SIGNED_INTEGER or FLOAT parameter with unary minus | Same-type result, including narrow overflow/rounding rules |
| L12 | Independent INTEGER parameters in comparisons | Bool result and exact signed/unsigned extrema ordering |
| L13 | Independent SIGNED_INTEGER parameters in addition | Reject computed result relationship even though every pair has a concrete operation |
| L14 | Rejected varying arithmetic used inside a comparison | Reject the intermediate computation, not only inspect final Bool |
| L15 | SIGNED_INTEGER parameter with concrete I64 arithmetic operand | Concrete I64 result when the complete declared family proves that relationship |
| L16 | Concrete mixed-width operations inside a generic body | Ordinary D15 policy, no blanket ban merely because the containing function is generic |
| L17 | Comparisons combined by `and`, `or` and `not` | Normal Bool result and short-circuit evaluation, no numeric truthiness |
| L18 | Passing a narrower family to a callee with a broader family | Prove declared family inclusion without source conjunction or body inspection |
| L19 | Passing NUMERIC to a FLOAT-limited callee without a narrower declaration | Reject at template validation, never select types from downstream requirements |
| L20 | Generic-context `1`, INTEGER `127` and INTEGER `128`/`256` | Universal success for valid leaves and an I8 range counterexample for invalid ones |
| L21 | INT_16_PLUS `1_000` and INT_32_PLUS `1_000_000` | Universal literal materialisation, arithmetic still checked in each selected type |
| L22 | FLOAT `0.1` and a literal overflowing F16 but not F32 | Precision-specific legal rounding in the first case, declaration rejection in the second |
| L23 | DEC `0.5`, NUMERIC `0.5` and an unbounded generic numeric literal | Reject according to scale, spelling or absent numeric guarantee without narrowing limits |
| L24 | Concrete binding/literal distinction and generic cast target attempt | Bound constants retain exact types, raw literal recipes stay contextual, generic cast targets remain excluded |

Use separate witness assertions within L04 and L11 for I8, U8 and F16. A declaration validated using I32/F64 only cannot establish universal correctness. Test ordinary trait limits separately from families, including method dispatch, a missing conformance and the absence of numeric operator permission.

### 12.4 Bounded call completion

| ID | Case | Required result |
|---|---|---|
| I01 | Concrete-return inner call passed to an identity generic | Resolve outward without requiring a named intermediate |
| I02 | Generic call with explicit immediate result receiver | Substitute missing parameters from the declared result relationship |
| I03 | `consume(empty())` with concrete `{I32}` parameter | Forward the concrete expected type into the nested call |
| I04 | `pick(empty(), known)` with known `{I32}` | Complete the waiting argument from independently established sibling type |
| I05 | I04 with arguments reversed | Same inferred types and diagnostics, source-order runtime evaluation |
| I06 | `pick(empty(), empty())` with no receiver | Annotation diagnostic, no default for a nonexistent numeric origin |
| I07 | I06 with explicit `{I32}` result receiver | Resolve both nested calls from concrete expected context |
| I08 | Existing `identity(1)` followed by direct U64 use | Preserve numeric-origin sharing and one completed concrete instance |
| I09 | Multiple pending numeric call arguments plus independent concrete witnesses | Complete permitted relationships before defaults, with no first-use winner |
| I10 | Conflicting concrete siblings or concrete argument/result expectations | Type conflict, no implicit promotion or candidate choice |
| I11 | Cast around a pending generic result | Target does not constrain the source and cannot invent a missing generic argument |
| I12 | Limit/evidence/backend availability appears to favour a type | Resolve independently, then validate or diagnose without type search |
| I13 | Long nested chain with one independently concrete receiver | Bounded readiness propagation, each argument parsed once |
| I14 | Waiting calls with cyclic or absent evidence | Deterministic annotation diagnostic rather than nontermination or speculative specialisation |
| I15 | Named arguments/defaults and compatible receiver-method calls | Preserve existing slot routing, required defaults and receiver alignment |
| I16 | Effectful arguments and an early failure among waiting calls | Source order, exactly once and correct short-circuit failure continuation |
| I17 | Generic-to-generic symbolic forwarding during template validation | No premature concrete requirement and no executable instance publication |
| I18 | Failed/pruned call completion and recursive distinct instance shapes | No stranded tickets or leaked IDs, retained recursion/resource policy and useful diagnostics |

For I03-I07, use a real generic `empty` returning `{Item}` and a real source `pick` relating its arguments/result. These cases do not allow an untyped empty literal to become a generic variable outside the call-owned receiving rule.

### 12.5 Nominal declarations and construction

| ID | Case | Required result |
|---|---|---|
| N01 | `Pair of First, Second = ...` | Explicit ordered nominal parameter list bound to existing reusable declarations |
| N02 | `Maybe of Item :: ...` | Equivalent ordered choice parameter contract |
| N03 | Bare `Pair First, Second` or old `Pair type ...` declaration | Reject replaced declaration spelling |
| N04 | Missing, duplicate, non-generic or unused listed parameter | Precise nominal declaration diagnostic |
| N05 | Reorder fields while keeping the `of` list unchanged | Stable type-argument interpretation and public parameter identity |
| N06 | Explicit `Pair of I32, String` receiving a constructor | Correct exact fields and ordinary argument routing |
| N07 | Unannotated Pair construction using already typed arguments | Reject argument-only generic constructor inference |
| N08 | Concrete parameter, return, field, element or default as receiver | Permit the constructor through that existing explicit applied type |
| N09 | Payload-free generic choice variant with/without applied receiver | Resolve from the explicit type or diagnose missing context |
| N10 | Concrete type alias used as receiver | Resolve transparently while alias remains non-callable |
| N11 | Symbolic `Pair of First, Second` receiver in an established generic body | Validate declared parameter relationships and specialise concretely later |
| N12 | Generic constructor nested under an unresolved outer generic parameter | Wait for permitted concrete/applied receiver, never infer its own type from payloads |
| N13 | Function returning a fully determined generic instance | Ordinary inferred local result remains valid, distinct from bare construction |
| N14 | Applied nominal with independent/repeated reusable parameters | Preserve equality/independence and exact nominal identity |
| N15 | Nested inline `of`, partial application and reusable-generic alias attempts | Preserve existing restrictions, no incidental type-level language |
| N16 | Generic receiver aligned list versus extra/changed/concrete-instance list | Accept aligned ownership and reject unsupported receiver extensions |

### 12.6 Trusted parametric binding contracts

| ID | Case | Required result |
|---|---|---|
| B01 | Scalar-parametric binding with repeated T inputs/result | One semantic declaration and correctly linked exact type substitution |
| B02 | Concrete supporting Bool/I32 slots and uniform Error slot | Ordinary ordered signature semantics, no fabricated aggregate type |
| B03 | Numeric scalar versus its physical carrier, including Dec | Keep semantic identity separate from ABI and runtime representation |
| B04 | Unsupported container-parametric signature | Reject the new binding shape without restricting legal Moth-source generics |
| B05 | Source generic calls the Core binding with a provably compatible limit | Validate from declared contracts and materialise only requested concrete instances |
| B06 | Duplicate/overlapping implementation entries for a concrete instance | Registration failure, no overload priority or insertion-order winner |
| B07 | Same canonical instance imported through multiple aliases/modules | Deduplicate semantic instance and emitted helper requirements |
| B08 | Same semantic type on different physical targets | Same source contract, separate validated implementation/ABI facts |
| B09 | Unsupported reachable instance versus a valid inactive instance | Accurate target gating from final executable work, without changing source eligibility |
| B10 | Failure after a provisional instance was reserved | Atomic rollback and no partial public binding registry result |
| B11 | File annotation tries to select privileged limits/adapters | Reject forged privileges rather than trust spelling or file location |
| B12 | Return provenance for selection of an input-backed Dec value | Correct alias/retention facts and valid use after caller bindings end |
| B13 | Declared Error slot on an infallible mathematical specialisation | Call handling remains mandatory, physical optimisation does not erase the contract |
| B14 | Instance signature/limit changes and implementation-only changes | Correct public versus implementation/runtime invalidation, no source-local IDs in keys |

### 12.7 Shared annotations and JavaScript preservation

| ID | Case | Required result |
|---|---|---|
| A01 | Identical `@moth.sig` payload in JS and WAT comments | One parsed signature meaning before host/adapter validation |
| A02 | Zero/one success return, final Error! and typed positional parameters | Preserve the current ordinary signature subset |
| A03 | Multi-success, optional/container, callback, receiver and generic ordinary annotations | Existing restricted-profile rejection, with no Core privilege leak |
| A04 | Renamed host export versus Moth-facing annotation name | Binding uses authored Moth identity and retains concrete export mapping |
| A05 | Duplicate/dangling annotation or duplicate Moth/export identity | Structured diagnostic and no partially registered package |
| A06 | Non-ASCII text preceding annotation and CRLF input | Exact UTF-8 byte source ranges under both extractors |
| A07 | Annotation-like text inside strings or irrelevant comments | No false declaration or source ownership |
| A08 | Existing supported JS export forms and unsupported forms | Preserve host parser contract during common signature extraction |
| A09 | Existing `@moth.const U32` and constant-only dependency | Original literal subset, constant projection and no unnecessary runtime asset |
| A10 | Existing opaque types and mutable opaque access | Preserve package-specific identity and existing restricted reference contract |
| A11 | Ordinary JS I64/U64 adapter edge values | Retain safe-Number boundary limits, no silent BigInt profile expansion |
| A12 | Ordinary JS F32/F64 invalid/rounded-success values | Existing resolved-precision checks and fatal versus declared Error delivery |
| A13 | Required named runtime imports and first-party helper inventory | Same explicit dependency ownership and no copied Error code table |
| A14 | Trusted file-backed JS implementation selected for a Core instance | Correct internal representation without routing through the restrictive ordinary foreign adapter |

### 12.8 WAT authoring, validation and execution

| ID | Case | Required result |
|---|---|---|
| W01 | Comment-annotated concrete scalar function imported by Moth | Parse, assemble, validate, register and execute the actual Wasm export |
| W02 | Inline function export and separate explicit function export | Correct association to the annotated export and final function type |
| W03 | Type-use declaration and imported function index offsets | Validate the actual binary function index/signature rather than assume source position |
| W04 | Valid WAT text producing an invalid typed module | Wasm validation diagnostic, not successful assembly reported as validity |
| W05 | Moth signature parameter/result count or carrier mismatch | Reject before execution, label annotation and associated function/export |
| W06 | Signed and unsigned I32/U32 at carrier boundaries | Exact domain and bit interpretation, no accidental signed reinterpretation |
| W07 | I64/U64 full-range transport and comparison | Preserve all bits, including unsigned upper half, without Number conversion |
| W08 | F32/F64 finite values, subnormals and signed zero | Correct representation and exact boundary handling |
| W09 | Non-finite raw float success | Existing integrity policy, never an ordinary valid Moth float |
| W10 | Explicit supported Error adapter versus absent adapter | Correct success/error delivery when available, capability diagnostic otherwise |
| W11 | Wasm trap versus declared recoverable Error | Distinct observable outcomes, no automatic trap-to-Error policy |
| W12 | Unregistered import or incompatible imported function | Precise dependency/signature diagnostic with no guessed runtime |
| W13 | Explicit helper file import and helper-only edit | Resolve once through registered dependency graph and invalidate affected implementation |
| W14 | Undeclared memory/table/start or unavailable managed adapter | Reject incompatible lifecycle/ABI requirement before executing module |
| W15 | Annotation requesting Dec or another non-native representation | Require an explicit trusted adapter, no guessed pointer/signature mapping |
| W16 | Component, WAST script or disabled Wasm feature | Reject outside the selected core-module feature profile |
| W17 | Nested comments, quoted strings and annotation-like decoys | Host-correct extraction with exact source offsets |
| W18 | Oversized/malformed source, annotation or type structure | Bounded resource handling and typed diagnostics/infrastructure lanes |
| W19 | Same file imported more than once or under aliases | Stable identity and one required module preparation, no duplicate effects |
| W20 | Valid unused dependency and malformed unused dependency | Omit unused runtime emission but preserve graph/input validity checking |
| W21 | Builder startup readiness and effectful Moth calls | Module available before synchronous calls, correct source order and exactly once |
| W22 | Unsupported direct-Wasm link route versus supported foreign asset route | Report the actual route/target capability, without claiming mixed-backend support or silently changing an explicit unsupported target |

W22 does not authorise choosing an unadvertised fallback. The builder explicitly owns and advertises the foreign-asset execution route described in Section 7.5. Target-specific rejection remains mandatory for any route it cannot implement.

### 12.9 Core Math dogfooding

| ID | Case | Required result |
|---|---|---|
| M01 | min/max at every fixed integer type, including extrema | Exact selected same-type value |
| M02 | min/max at F16/F32/F64 | Original finite value selected without unnecessary rerounding, documented zero-sign freedom retained |
| M03 | min/max at Dec0, a fractional scale and Dec256 | Exact coefficient/scale selection, no binary-float detour |
| M04 | clamp below, inside and above limits | Exact composition with the original argument type |
| M05 | clamp with lower greater than upper | Upper value wins without a new error |
| M06 | Different concrete operand types or scales | Exact type conflict, no mixed arithmetic promotion to choose storage/result |
| M07 | abs on each signed integer minimum | Correct existing numeric overflow Error, not wrap or trap substitution |
| M08 | abs on other signed values and complete unsigned ranges | Same-type exact magnitude/identity with uniform declared Error slot |
| M09 | abs on finite binary floats and Dec | Same-type mathematical magnitude and existing resource rules |
| M10 | abs without authored propagation/recovery | Missing declared-error handling diagnostic even for unsigned/float/Dec |
| M11 | abs Error caught, propagated or explicitly mapped into custom error | Existing call/error contracts, correct code/message/value identity |
| M12 | Effectful min/max/clamp/abs arguments | Source-order exactly-once evaluation across file-backed implementations |
| M13 | Source-authored generic helper calls each Core operation | Declared limit validation and no arbitrary primitive trait conformance |
| M14 | Cross-package wrapper, imported aliases and nested concrete context | Correct canonical instance/evidence identity and agreed call completion |
| M15 | Generic Math literal contexts, including narrow rejection | Universal definition-time literals and normal call-site literal materialisation remain distinct |
| M16 | WAT scalar min/max/clamp on supported exact types | Real execution and parity for specified results |
| M17 | WAT abs with supported/unsupported declared Error ABI | Uniform source contract with correct execution or precise capability rejection |
| M18 | Missing Dec WAT representation and unreachable use | Source contract remains valid, target gate follows final executable reachability |
| M19 | Selected Dec result used after original arguments leave scope | Correct ownership, retention and cleanup with no dangling value |
| M20 | Remaining Math functions and six constants | Existing F64 API, approximation and boundary tests unchanged, no new generic constants |

### 12.10 Cross-cutting regression obligations

Beyond the named rows, preserve these properties wherever the new paths touch them:

- Both branches of ordinary static control flow and unreachable catch handlers receive the required source/type validation before executable work is pruned.
- Inactive generic calls produce no durable executable request, target capability or helper after the correct summary-convergence boundary.
- Published HIR, borrow/proof tables and link facts remain paired to the same final immutable executable after any rewrite.
- Generic limits never become constraints directives, runtime Bool tests or source-visible physical target predicates.
- Map keys, anonymous records, Wire/Route parameters and fixed capacities retain their independent eligibility rules. A numeric family does not automatically admit generic map keys or previously forbidden anonymous shapes.
- Native result-slot arity, Never/terminality, optional handling and same-file trait conformance retain their permanent contracts.
- Matching source semantics on debug/release and supported targets produces the same accepted types and specified values. Optional optimisation cannot remove a declared error-handling requirement.

## 13. Required documentation and roadmap edits

### 13.1 Permanent semantic and architecture owners

Publish final-design contracts in their permanent owners during Phase 0 and keep implementation claims in the progress matrices. Migrate teaching examples as the executable syntax becomes available. The plan is a temporary delivery owner, not a permanent second language reference.

| Owner | Required edit |
|---|---|
| `docs/src/docs/generics/generic-declarations.mtf` and its Basic companion | Replace signature-local `type` introduction with all three reusable declaration forms, one-limit rules, compile-time-only `#Type`, file privacy and near-use style guidance |
| `docs/src/docs/generics/generic-inference.mtf` and its Basic companion | Explain immediate concrete receivers, sibling evidence, numeric-origin sharing, unresolved calls and the exact absence of body/trait/backend type search |
| `docs/src/docs/generics/type-application.mtf` | Distinguish nominal `of` parameter introduction from applied type use, preserve ordered arguments and document required constructor receiving context |
| `docs/src/docs/generics/generic-instances.mtf` | Owner-local parameter identity, independent instances, aligned receiver methods, explicit construction and public nominal contracts |
| `docs/src/docs/generics/generic-limits.mtf` | Eight exact-case numeric families, ordinary single trait limit, universal operator/result/literal rules and V1 exclusions |
| Generic `@page.moth` and Basic page composition | Present the learning sequence below and link the detailed rules without duplicating a second capability catalogue |
| `docs/src/docs/traits/generic-trait-bounds.mtf` and relevant trait pages | Convert source syntax and terminology to reusable generic declarations with one trait limit, preserve explicit conformance and explain that numeric families are compiler-owned limits rather than implementable traits |
| Struct, choice, function and receiver references | New nominal declaration syntax, applied constructor receivers, signature-derived function parameters, aligned receiver rules and examples for empty/payload-free construction |
| `docs/src/docs/aliases/type-aliases.mtf` | Preserve concrete aliases and explain that reusable generic copying/aliasing is excluded, with no reversal of ordinary alias syntax |
| Binding and constant references | Separate generic declaration metadata from ordinary runtime or folded values, including the specific `#Type` role and invalid `Type` forms |
| Numeric, cast and error references | Show generic uses of existing exact semantics without changing D15, generic-cast exclusions or ordinary explicit call-error handling |
| `docs/src/docs/design-scope/deferred-and-outside-scope.mtf` | Replace the provisional Numeric and generic extensions handoff with the accepted V1 design and precise remaining extensions |
| `docs/src/docs/design-scope/excluded-language-families.mtf` | Narrow previous blanket exclusions only for this closed design, retaining user-defined type-set languages, generic aliases, reflection, body-inferred limits, operator overloading and conditional error signatures as excluded/outside this work |
| `docs/src/docs/packages/external-binding-contracts.mtf` | Common annotation semantics, ordinary JS/WAT capability profiles, trusted registration distinction, explicit adapters and exact failure/integrity boundaries |
| A focused WAT topic under `docs/src/docs/packages/`, with package page composition | End-to-end scalar file, Moth import, signature validation, helper imports, target capability and unsupported representation examples |
| `docs/src/docs/packages/core/math/math.mtf` | Generic min/max/clamp/abs, same-type parameters, uniform abs Error!, unchanged clamp and zero-sign policy, target matrix and remaining F64 surface |
| `docs/compiler-design-overview.md` | Generic declaration preparation, limit checking, concrete call readiness, source versus binding instantiation, canonical identities and fully typed HIR handoff |
| `docs/build-system-design.md` | File-backed provider inputs, explicit helper closure, readiness, asset and module ownership, target/adapter selection and invalidation without a mixed-backend dependency |
| `docs/compiler-data-layout-design.md` | New source kinds/comment span ownership and uncommon declaration/diagnostic data, preserving locked common record sizes and exact byte spans |
| Routed memory/call/ABI references | Scalar binding access and return provenance, especially Dec selection, explicit Wasm adapters and the ordinary-versus-trusted boundary |
| `docs/src/developer-docs/language/overview.mtf` and compiler/build teaching pages | Route to the new permanent owners and remove stale signature-local generic examples |
| `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf` | Compact complete examples for declarations, numeric limits, Pair/Maybe `of`, contextual calls and rejected shorthand, with no implementation-status clutter |
| Both progress matrices | Distinguish accepted syntax, implemented limits/inference, Core binding support, actual JS/WAT execution and unsupported representation/target cases |
| Standalone lexical/MON documentation and tests | Describe only relevant shared naming changes, without claiming that `#Type` or generic declarations are MON data |
| Highlighter, editor grammar, source-word tables and docs code blocks | Update `#Type`, family names and `of` context, remove old generic keyword presentation and add enough WAT highlighting for readable examples without semantic inference in the formatter |
| `README.md`, contributor/agent routes, examples, scaffolds and `index.md` | Correct changed syntax and real owner paths, preserving unrelated project guidance |

Preserve ordinary trait declaration/conformance terminology where it remains accurate. The public generic restriction is called a **limit**, while existing Rust owner names containing `bound` may remain when renaming them would add churn without clarifying a changed responsibility.

### 13.2 Required teaching sequence

Expand generic documentation with complete examples rather than only a new syntax table. Each example must show or identify the declarations and receiving contracts that make it valid.

1. An unbounded identity function using `Item #Type`, called for a concrete integer and a string. Explain that the declaration remains reusable.
2. A same-name two-parameter function and a conflicting-type call. Contrast it with two fresh names that can select different types or the same type.
3. Reuse one numeric declaration in several functions without sharing their concrete instantiations. Show a fresh nearby declaration with the same limit as a readability alternative.
4. NUMERIC addition with Error!, INTEGER real division returning F64, FLOAT division and a universally invalid operation rejected at the generic declaration.
5. Literal `1` under NUMERIC, rejected INTEGER `256`, accepted INT_16_PLUS `1_000` and accepted INT_32_PLUS `1_000_000`. Explain literal representability separately from arithmetic overflow and algorithmic precision.
6. A Pair and a payload-free Maybe variant with ordered `of` parameters and explicit applied receiving types. Show the rejected argument-only constructor form next to its correction.
7. A generic function returning `Pair of First, Second`, distinguishing its inferred call result from a direct untyped constructor.
8. A nested `empty()` call solved by a concrete outer parameter and both sibling argument orders. Show the unresolved double-empty form and annotation correction.
9. A small provider/consumer package example. The provider exports a generic function, the consumer calls it, and neither imports the provider's reusable generic declaration.
10. A source-authored NUMERIC helper calling generic Core Math, including explicit handling of abs. Explain that file-backed implementation is invisible to the ordinary source contract.

Show file-local generic privacy and invalid `Type` at least once in the Basic material, not only buried in the advanced reference. Keep deeper result-relation restrictions and ABI details in focused advanced sections.

### 13.3 Roadmap and later-owner coordination

The authoring/insertion change adds the queued plan bullet immediately after Never return contracts. **Phase 0 activation** removes the exact obsolete deferred heading and paragraphs if still present. Preserve the optional catch tooling heading and the permanent Numeric and generic extensions anchor. The latter is updated rather than deleted, so incoming links keep their meaning.

Coordinate these existing owners by capability, not by creating links between temporary implementation plans:

| Coordination target | Required handoff |
|---|---|
| Main roadmap | Exact insertion point, activation-only placeholder cleanup and no movement of unrelated entries |
| Living first-party package programme and Core Math companion | Four delivered parametric consumers, file-backed ownership, uniform abs handling and a separate list of later generic opportunities |
| Later mixed-Wasm implementation work | Existing scalar WAT/provider path, explicit unsupported managed/error/Dec instances, adapter contract and source/backend identity separation |
| Later struct-layout and collector-free memory work | Concrete binding access/provenance and memory adapter requirements, without inventing a private Core allocator |
| Package dependency/manager work | Canonical callable identities, file-private reusable declarations and implementation dependency fingerprints, without making those declarations importable |
| Moth-native MON integration | Concrete nominal parameter/application changes and unchanged requirement that schemas describe concrete types, not reusable generic metadata |
| Later constraints and fast-math consumers | Explicit limits are separate from directives and inferred effect summaries, with no change to checked ordinary operations or the uniform public abs error slot |
| Existing nominal, loop, Wiring and source-kind work | Updated examples and producers consuming the same `of`, signature and known-shape rules, without expanding their independent scope |
| LSP and editor follow-ups | Resolved limits shown in hover/documentation, optional advisories remain tooling work and never default compiler warnings |

A future richer Math pass should identify useful generic algorithms from real package use. Record candidates and prerequisites without adding new approved APIs through an implementation note. Generic constants, decimal transcendental accuracy, extra numeric families and generic container bindings remain distinct design questions.

### 13.4 Generated documentation and status honesty

Rebuild generated documentation through the compiler and inspect links, code blocks, page composition and obsolete syntax. Never edit release HTML by hand.

At early documentation checkpoints, clearly separate final approved design from current executable support. A syntax migration example is not proof that a parser/backend supports it. Progress rows name actual source tests, metadata checks and target execution or rejection, not the intended family membership alone.

Where changing this work materially invalidates an existing audit row, mark it stale under the audit policy. An implementation review cannot claim a new completed structured audit.

## 14. Validation and performance protocol

### 14.1 Activation evidence

Use the active repository's pinned Rust and Node versions, `justfile`, feature-lane policy and test manifest. The research versions and command shapes below identify owners, not permission to ignore newer authoritative recipes.

Record the actual activation branch/tree, `rustc -vV`, `cargo clippy -V`, `cargo fmt --version`, Node executable/version, Cargo features, target, concurrency and cache state under `tmp/generics/`. Run the pre-change suite inventory and targeted baseline before replacing syntax or helpers.

The Node capability probe is:

```bash
node -p 'process.execPath + " " + process.version + " " + typeof Math.f16round'
```

Authoritative runs use the repository's exact `.node-version`. A missing host capability is infrastructure failure, not a reason to weaken F16 or Wasm integration tests.

### 14.2 Validation commands and cadence

Use the narrowest existing test owners during iteration. Verify that filters selected the intended tests and report the executed count. A successful command selecting zero tests is not evidence.

Representative commands, checked against their activation-time owners:

```bash
cargo check -p moth
cargo test -p moth --lib generic_functions
cargo test -p moth-lexical
cargo test -p moth-mon
cargo test -p moth-lexical --doc
cargo test -p moth-mon --doc
cargo run --quiet -- tests --audit
cargo run --quiet -- tests --list --tag generics
cargo run --quiet -- check docs --terse
cargo run --quiet -- build docs --release
```

The `generics` tag and library filter must be confirmed from the actual manifest/test list. Where they do not exist or fail to select the changed owners, use discovered exact case IDs or test names. Do not add a second runner to make an assumed command work.

| Checkpoint | Required evidence |
|---|---|
| One implementation work package | Focused failing regression/probe, the minimal implementation, matching passing tests and affected crate check |
| Public identity, call-completion or ABI change | Relevant real multi-file/package integration plus subsystem invariant tests |
| WAT provider milestone | Module validation and actual generated-artifact execution, including negative source/ABI cases |
| Major integrated feature boundary | `just validate` plus affected lanes outside its current scope |
| Shared lexical, source-kind or standalone API impact | Package-scoped lexical/MON tests and current feature/platform lanes |
| Documentation-only branch checkpoint | Current documentation build and source/generated diff review |
| Final main-bound implementation | `just validate-full` on the final integrated tree plus any required affected opt-in lanes |

Use the current `just fmt-check`, `just clippy`, workspace and feature recipes rather than inventing new lint switches. Include first-party dependency/helper inventory checks when implementation files, imports or parser dependencies change. Authoritative validation must cover the normal build as well as relevant instrumentation and developer-output configurations.

Verify the generated Wasm with the explicitly selected feature profile. Executing a module in Node is not a substitute for compiler-side validation, and validating it is not a substitute for runtime numerical and error assertions. Test missing imports, invalid binary types and real traps independently of Moth Error delivery.

A completed implementation has matched evidence for JS and supported WAT paths. Capability-rejected paths have exact rejection tests and are identified as such. No report may combine them into an undifferentiated passing runtime count.

### 14.3 Performance baselines and scaling

Preserve three cohorts:

1. Existing generic programs, mechanically migrated with equivalent semantics.
2. New context-forwarding and reusable-declaration workloads.
3. Parametric Core and file-backed JS/WAT programs, measuring frontend cost, implementation preparation, emitted artifacts and runtime behaviour separately.

An argument-inferred constructor changed to an explicit receiver can remove work. Record that adaptation rather than attributing the difference entirely to faster generic inference. A file-backed helper can change call and asset overhead even when its mathematical output is identical. Include startup/module preparation and recurring call cost separately.

Use at least five independent matched final measurements on the same machine, toolchain, feature/profile, concurrency and cache policy, following the repository's current benchmark methodology. Preserve raw logs outside committed source. Existing historical generic scaling exceptions remain historical evidence, not permission to loosen a regression budget.

Required workload shapes include:

| Shape | Cost or invariant to measure |
|---|---|
| One file-private generic reused by many functions | No shared concrete state and no repeated cloning of the complete reusable declaration |
| Many files with same-spelled private declarations | Linear ownership/binding work and stable identities under parallel schedules |
| Long nested waiting-call chain with one concrete witness | No repeated parsing or full-chain rescans for every readiness change |
| Sibling fan-out and reordered concrete witnesses | Bounded dependency updates and deterministic conflict selection |
| Ambiguous cycles without numeric origins | Timely annotation failure, no speculative candidate enumeration |
| Many NUMERIC operations and Dec scales | Shared family/operator/literal validation, no Cartesian product of all concrete instantiations |
| Repeated concrete source and Core calls | Canonical instance/helper deduplication with no per-use signature cloning |
| Large retained but inactive generic branches/handlers | Correct release of provisional work and no output/helper growth |
| Multi-file implementation dependency closure | Parse/validate once per relevant content/configuration, correct targeted invalidation |
| Concrete non-generic program | No general solver overhead imposed on unrelated calls or expressions |

Measure signature binding, limit validation, known-shape substitution, pending call storage, readiness updates, template validation, concrete materialisation and publication independently where existing counters permit. Include allocation and retained-capacity costs, store destruction, helper preparation and final output bytes rather than only record size or one microbenchmark.

Use deterministic operation-count invariants to protect known algorithmic bounds. Do not add machine-dependent wall-clock assertions to correctness tests. Complexity claims must account for the actual dependency graph and emitted unique instances, not only the number of source statements.

A repeatable slowdown above the repository's current materiality threshold requires investigation. The inspected expression protocol uses 5% as a material representative-case regression, with smaller repeated regressions still requiring explanation. Preserve the stricter activation-time policy if it changes. Do not reset baselines, expand budgets or drop adverse workloads to make the plan pass.

### 14.4 Review and reporting

Review each phase for ownership, source contracts, deletion obligations and test gaps. Review the final diff both by producing owner and by end-to-end feature. Explicitly check that the declaration parser, type environment, public interface, generated-function path, binding registry, runtime adapters and documentation express the same rules.

After the final code or documentation change, rerun invalidated gates on the actual resulting tree. Record the final source revision and configuration. Report infrastructure failures, existing target gaps, intentional source rejections and regressions separately.

Keep a concise resumable note under `tmp/generics/` naming the current work package, accepted decisions, changed owners, tests run, outstanding ledger entries and next bounded action. Keep detailed logs and the coverage ledger in separate files. Context compaction is not permission to continue from remembered syntax without rereading the active authorities.

## 15. Completion and deferred boundaries

### 15.1 Search-based closure

Search production, tests, generators, examples, scaffolds, documentation, standalone crates and feature-gated paths for:

```text
type T, type A, GenericTraitBound, trait_bounds
GenericParameter, GenericParameterList, GenericCallExpectedContext
numeric_origins, numeric_known_shape, DeferredGenericCall
GenericFunctionInstanceKey, GeneratedFunctionIdentity
Type, #Type, NUMERIC, INTEGER, SIGNED_INTEGER, FLOAT, DEC
INT_16_PLUS, INT_32_PLUS, FLOAT_32_PLUS
BINARY_FLOAT, DECIMAL, generic copying, generic alias
ExternalSignatureType, ExternalFunctionSpec, ExternalFunctionLowerings
InlineExpression, RuntimeFunction, ExternalModuleExport
@moth.sig, @moth.const, @moth.opaque, SignatureParseInput
RuntimeAssetIdentity, RequiredRuntimeImport, BuilderRuntimePackageMetadata
min, max, clamp, abs, Error!
```

Classify every hit. A Rust type, English word, host-language keyword, historical evidence, concrete alias or deliberate negative fixture is not an obsolete feature merely because its spelling matches. Remove old supported generic grammar and duplicate signature parsers, not unrelated language or host syntax.

Inspect unused wrappers, stale "generic external calls are unsupported" diagnostics, embedded algorithm strings and target adapters along with their consumers. Keep direct instruction lowerings and small literal metadata when they serve the selected implementation, rather than forcing all compiler logic into an implementation file.

### 15.2 Final conditions

- [ ] Every decision G01-G18 has a permanent semantic contract, implementation owner and verified evidence.
- [ ] All three reusable declaration forms work, bare Type is diagnosed and no generic becomes a runtime or ordinary folded value.
- [ ] File-private reusable identities remain separate from owner-local parameters and concrete specialisations.
- [ ] Repeated names, independent names, owner reuse and aligned receiver identity match the documented examples.
- [ ] One-limit V1 restrictions are enforced, including rejection of conjunctions, alternatives and generic copying/aliasing.
- [ ] Numeric family names, membership and exact-case policy match the approved inventory.
- [ ] Bodies and literals validate universally without changing the declared family or computing new type-level result expressions.
- [ ] Nominal declarations use ordered `of` parameters and generic construction requires an applied receiving context.
- [ ] Function calls preserve permitted numeric-origin inference and support concrete outer/sibling context without argument reparse or runtime reordering.
- [ ] No pending call/type/evidence identity escapes into published interfaces, HIR or concrete instance keys.
- [ ] Cross-module and source-package specialisation, trait evidence and diagnostic provenance remain correct.
- [ ] Trusted scalar-parametric bindings reuse compiler-owned semantic rules while physical implementations/adapters remain explicit and target-specific.
- [ ] Ordinary JS/WAT annotations share one signature meaning and cannot grant privileged capabilities.
- [ ] Real Moth imports execute the mandatory scalar WAT path, with validation, import, readiness, ownership and invalidation coverage.
- [ ] Missing Wasm representations/error adapters have precise diagnostics and are not hidden by traps, unchecked conversions or an undeclared fallback.
- [ ] Generic min/max/clamp and uniformly fallible abs are delivered with exact supported numerical behaviour and correct access/return provenance.
- [ ] Remaining Math functions/constants retain their F64 contracts and useful existing regression assertions.
- [ ] Every matrix row has a coverage-ledger owner and outcome, and every removed test assertion has a reviewed disposition.
- [ ] Canonical references, teaching material, source examples, generated documentation and both matrices agree.
- [ ] The old deferred generics roadmap heading is removed at activation, its permanent handoff survives and the entry remains at the requested serial position until retirement.
- [ ] Final performance and full-validation evidence describe the actual integrated tree and supported/rejected target matrix.
- [ ] The final Slice review is complete, obsolete code is deleted and this one-shot plan is retired in its completion commit.

### 15.3 Deliberately later work

Record these as separate follow-up design or implementation opportunities, with their actual accepted/open status rather than a promise of support:

- Additional Core algorithms that benefit from the delivered generic model, selected from real package use.
- F16/F32 or decimal extensions to the remaining Math functions, including numerical accuracy and failure contracts.
- Decimal mathematical constants and approximate decimal algorithms that never pretend an F64 round trip supplies higher precision.
- Generic constants, additional useful closed family names and specifically justified positive-scale decimal limits.
- Multiple simultaneous or alternative limits and reuse/copying/aliasing of generic declarations. File-private locality remains the V1 design, not a temporary accident.
- Generic container/aggregate bindings and their complete layout, mutation, ownership and allocation contracts.
- More ordinary WAT foreign shapes, managed representations and Error ABI support through their assigned backend/runtime owners.
- WIT component loading, broader external dependency graphs and source-level general generic external declarations.
- Optional LSP presentation/advisories and further source ergonomics that preserve explicit public contracts.

Body-inferred limits, user-defined numeric families, operator overloading, unrestricted type computation, general return-only inference, arbitrary compile-time foreign execution and conditional generic error signatures are not implementation shortcuts for these follow-ups.

## Appendix A. Research provenance and implementation limits

### A.1 Inspection receipt

Repository inspection for this document used the live GitHub connector after the user's latest push. The observed `main` revision was `36156b40849eecad12e69b8d4e62388672a1ecff`, committed on 12 October 2026. This is a research receipt, not the future activation baseline or a pinned prerequisite.

The revision contains the exact numeric cutover and D15 contract. Focused reads confirmed the new ready/retained generic-call path, numeric-origin signature linker, fixed-scalar external metadata, F64 Core Math registration and the retained later-generics roadmap placeholder.

GitHub code search still returned some paths from the older indexed parent during this inspection. Those results were used as locators only. Critical current facts were checked by fetching the observed main revision explicitly. The implementation agent must repeat current repository-wide searches at activation, including files created or moved since this receipt.

### A.2 Principal inspected owners

| Receipt | Current inspected source or reference | Relevance |
|---|---|---|
| R01 | `AGENTS.md` and `docs/roadmap/roadmap.md` | Authority, test/validation workflow, serial insertion point and exact placeholder removal |
| R02 | `docs/compiler-design-overview.md` | Generic publication, source-owned metadata, numeric handoff, native result direction, call/proof and Wiring boundaries |
| R03 | `docs/build-system-design.md` and `docs/compiler-data-layout-design.md` | Provider ownership, source/physical identity, capabilities, locked spans and diagnostic lanes |
| R04 | `docs/src/developer-docs/style-guide/{style-guide,testing,validation}.mtf` | Small coherent changes, primary test ownership, assertion preservation and final gate policy |
| R05 | `src/compiler_frontend/ast/generic_functions/calls.rs` | Shared instance/template-validation lanes, ready/retained arguments, deferred requests and source-ordered tickets |
| R06 | `src/compiler_frontend/ast/generic_functions/numeric_origins.rs` | Signature-only numeric origins, known-shape relationships, receiver evidence and deferred nominal linking |
| R07 | Generic declaration, instance, inference, limit and type-application references plus canonical generic parameter/template owners | Old syntax, current nominal inference and stable declaration-owned parameter identities |
| R08 | `docs/src/docs/numbers/operators.mtf` and exact numeric handoff references | D15 invariants and arithmetic exceptions consumed rather than redesigned |
| R09 | `src/builder_surface/core_packages/math.rs` and `docs/src/docs/packages/core/math/math.mtf` | Current 31-function F64 registration, six constants, clamp/zero/failure contracts and missing Wasm implementations |
| R10 | `src/projects/html_project/external_js/parser/` and `signature_parser.rs` | Shared-parser extraction opportunity and intentionally restricted ordinary foreign signature |
| R11 | `docs/src/docs/packages/external-binding-contracts.mtf` | Current JS annotation subset, exact adapters, constant-only assets and fatal versus declared Error delivery |
| R12 | `src/builder_surface/external_import_providers/provider.rs` | Provider kinds, logical identities, runtime asset records and explicit runtime imports |
| R13 | `Cargo.toml` and `index.md` | Existing wasmparser/wasm-encoder tooling family, standalone crates, features and ownership locators |
| R14 | Existing facade/receiver/generic materialisation cases and prior focused generic source reads | Regression starting points, not a claim that every future affected test has been audited |

The WebAssembly core text-format and validation specifications and the official wasm-tools parser/validator documentation were also consulted for host-format boundaries. They define WAT parsing, core module types, exports/imports and binary validation. They do not define Moth source signatures, generic limits or the trusted runtime adapter policy.

External reference starting points:

- WebAssembly core text modules: `https://webassembly.github.io/spec/core/text/modules.html`
- WebAssembly core module validation: `https://webassembly.github.io/spec/core/valid/modules.html`
- wasmparser validator API: `https://docs.rs/wasmparser/latest/wasmparser/struct.Validator.html`
- wasm-tools repository: `https://github.com/bytecodealliance/wasm-tools`

Select compatible assembler/validator dependencies against the actual activation-time toolchain and repository pins. These references are not a request to upgrade dependencies or enable every newly available Wasm feature.

### A.3 Evidence limits and handoff

This planning session inspected sources and contracts through the connector. A local network checkout was unavailable. It did not execute the Moth compiler, validate generated Wasm, run the repository suites or measure performance. Commit-reported prior validation remains historical evidence for that commit, not evidence that this future feature has been implemented or tested.

Phase 0 therefore requires a fresh executable baseline, complete owner/consumer inventory and assertion-level coverage ledger. The acceptance cases specify work to implement and prove, not tests claimed to pass today. The final source syntax in this document is approved design and intentionally differs from the current parser.

At each implementation checkpoint, report the changed contract, producing owner, migrated consumers, removed old path, test scope/outcome, coverage-ledger status and remaining bounded action. Completion reports must name the exact validated tree, supported JS/WAT matrix, deferred adapters, documentation handoff and performance evidence without implying unrun checks passed.
