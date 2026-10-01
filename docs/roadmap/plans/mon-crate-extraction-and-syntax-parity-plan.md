# MON crate extraction and syntax parity

## Goal

Expose MON as an independent Rust crate without pulling the Moth compiler into its consumers. Give MON and Moth one owner for shared identifier, reserved-word and numeric-text rules. Keep their parsers independent and verify their common construction notation through real cross-parser tests.

**Status:** Ready for activation.
**Current slice:** 0, baseline and contract inventory.
**Blockers:** None known. Establish the actual baseline when work starts.
**Next action:** Activate before other serial roadmap work resumes.

**Architecture:** `moth` depends on `moth-mon` and `moth-lexical`. `moth-mon` depends on `moth-lexical`. The two extracted crates have no dependency on the compiler, its identity tables or its semantic stages.

**Technology:** The existing Rust workspace, toolchain, numeric implementations, Cargo test suites and `xtask` gates. Reuse the existing `ryu` dependency for formatting. Add no parser framework, procedural macro, property-testing framework or arbitrary-precision dependency to the extracted crates.

**Specification:** The accepted contracts below, together with the canonical authorities listed next. This plan records the extraction and parity decisions approved in the originating discussion. Publish their durable contracts in the existing documentation before retiring this file.

## Prerequisites and scope

The expanded numeric implementation is already delivered. Consume its fixed-width integers, F16/F32/F64, Byte, profile-aware Int/Float, exact decimal data and public MON profile API. The current source-facing exact-decimal names are `Dec` and `Dec0` through `Dec256`. Internal Rust names such as `NumberScale` and `NumberValue` remain valid implementation names.

The delivered MON codec, shared source call-argument parser and retained declaration-shell parsers are prerequisites, not replacements to build again.

This work includes crate extraction, shared lexical ownership, cross-parser tests, demonstrated MON parity fixes, bounded codec cleanup and affected documentation/tooling. Preserve the public numeric shapes and data-service behaviour. The deliberate corrections are reserved-name rejection, the named-entry line boundary and consistent invalid-default error projection. Newly exposed low-level APIs also need checked public preconditions.

The following remain outside this work:

- Moth-native encode/decode, automatic schema extraction, `$mon`, static asset generation and backend/runtime codec integration.
- Delivery of currently deferred source syntax for `{=}`, Unicode escapes or contextual `::Variant`. Their existing source gaps remain explicitly tested and tracked, not claimed as completed parity.
- Declaration-parser extraction, a universal syntax tree, a shared expression parser, a general cursor framework or another compiler pipeline.
- Runtime anonymous-record implementation, declaration semantics, operator promotion, cast eligibility, numeric arithmetic or physical layout redesign.
- Streaming, public borrowed document views, cyclic schemas, Serde integration, public pretty-printing modes, registry publication or a new version/migration service.

## Reading routes

Read current files from the active worktree. Earlier attachments and discussion snippets do not establish current numeric APIs or support.

Read `AGENTS.md`, `docs/src/developer-docs/style-guide/style-guide.mtf`, the testing and validation guides beside it and `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf`. The cheatsheet is orientation rather than the owner of edge-case semantics.

For this cross-boundary change, read the compiler and build design documents in full. Also read the data-layout design, the memory-management overview, the progress matrices and the roadmap's plan-maintenance rules. Follow `docs/src/developer-docs/language/overview.mtf` to the canonical numeric, naming, string, collection, map, struct and choice references.

The principal permanent owners are:

| Authority | Relevant contract |
| --- | --- |
| `docs/compiler-design-overview.md` | Semantic ownership, retained syntax, numeric materialisation, compiler-private stages and the Rust MON boundary. |
| `docs/build-system-design.md` | Service orchestration, numeric-profile selection, MON tooling and ordinary resource/output ownership. |
| `docs/compiler-data-layout-design.md` | MON data handoff, input lifetimes, byte spans, bounded allocation and separate error boundaries. |
| `docs/src/docs/language-overview/mon-syntax.mtf` | Shared construction notation, grouping, named entries and receiving-context differences. |
| `docs/src/docs/language-overview/comments-and-naming.mtf` | Identifier and reserved-name policy. |
| `docs/src/docs/mon/mon-format.mtf` | Literal-data grammar, qualifiers, defaults, schemas, numbers, limits and encoding. |
| `docs/src/developer-docs/style-guide/testing.mtf` | Test ownership, meaningful assertions, source-text audit rules and feature lanes. |
| `docs/src/developer-docs/style-guide/validation.mtf` | Targeted checkpoint checks and final merge evidence. |
| `docs/roadmap/roadmap.md` | Immediate sequencing, deferred MON source capabilities and plan retirement. |

Use `index.md` for navigation. Apply the supplied `moth-code-review-guide.md` to the changed code's duplication, ownership, diagnostics, tests and abstraction shape. The final Slice review is separate from a formal audit run.

## 1. Accepted final contracts

### 1.1 Dependency and public API boundaries

The dependency direction is:

```text
moth ------> moth-mon ------> moth-lexical
  |                              ^
  +------------------------------+

game engine ------> moth-mon
```

`moth-mon` owns literal reading, schemas, defaults, validation, encoding, owned values, limits and public errors. A standalone consumer supplies complete text or values and a prepared schema. The codec performs no file IO, project discovery, module compilation or expression evaluation.

`moth-lexical` owns shared text rules and the small numeric vocabulary needed to materialise and format their values. It is not a compiler tokenizer or a general numeric runtime. Its numeric leaves may own widths, scalar identities, precision, exact scalar carriers and decimal spelling facts because those are real shared inputs and outputs. Compiler type lookup, arithmetic and allocation-heavy decimal values stay above it.

Keep `moth::mon` as a deliberate supported convenience re-export:

```rust
pub use moth_mon as mon;
```

This is the same API and the same Rust types, not a second implementation or a forwarding layer. Document this explicitly so the project's general prohibition on compatibility shims does not become a reason to add wrappers or preserve old private modules. The canonical crate documentation belongs to `moth-mon`.

Preserve access to `NumericProfile`, `IntWidth` and `FloatPrecision` through both `moth_mon` and `moth::mon`. Both consumers use the same underlying shared types. The compiler still owns selection of one profile for its compilation boundary.

### 1.2 What source parity means

MON is the data-only profile of the shared Moth construction notation. For a supported common literal form in the same language/format revision, equivalent receiving types/schemas and the same numeric profile must agree on syntax acceptance and the represented typed value.

This is not an assertion that an arbitrary MON document can be pasted anywhere in a Moth program. Document framing, supplied schema information and legal source receiving contexts matter. For example, an anonymous source record does not implicitly become a nominal struct merely because a MON schema can receive anonymous record data as that struct.

Shared punctuation does not combine declaration parsing with supplied-value parsing. The compiler's existing call-argument owner continues to handle source calls, constructors and named records. Its declaration-shell owner continues to serve headers and body-local declarations. MON uses neither owner.

Each mismatch belongs to one of four explicit categories:

| Category | Required treatment |
| --- | --- |
| Common literal form | Assert the expected result in both parsers, then compare typed observations. |
| Common invalid form | Assert the intended rejection reason in each parser. |
| Intentional data/source distinction | Name the contract and assert both outcomes independently. |
| Accepted but undelivered source capability | Keep a named executable gap case and a progress/roadmap reference to the missing capability. It is not permanent permission to diverge. |

Resource exhaustion and unsupported source receiving contexts are not grammar comparisons. Use fixtures within both receivers' limits when proving parity.

### 1.3 Shared identifier and reserved-word policy

Move the existing identifier character rules and reservation policy into one neutral owner. Preserve their current meaning:

- Identifier start is underscore or a Unicode alphabetic character. Continuation is underscore or a Unicode alphanumeric character.
- Identifier spelling stays case-sensitive. No Unicode normalisation, XID substitution or ASCII-only restriction is introduced.
- User-name reservation strips leading underscores and performs the existing ASCII-insensitive whole-name check.
- The Dec reservation covers the bare name and every ASCII-digit suffix, including invalid type spellings such as `dec01` and `dec257`. `DecBox` remains an ordinary identifier. Valid Dec type recognition remains a separate check for canonical case and scale.

Keep three questions distinct: whether text has identifier shape, which exact source word it denotes and whether it is legal as a user-chosen name. In particular, `true`, `false` and `none` remain literal tokens in value position. They are not legal record labels or nominal names. Reserved words inside quoted String data or string map keys remain ordinary data.

Use one shared inventory for exact word identities and their reservation facts. Preserve intentional reserved-only names such as `fn` and `config`. Preserve the distinction between `this` and `This` and between literal and builtin-type spellings. Dec names remain symbol tokens in the compiler, with type recognition at the existing owner.

The compiler maps neutral word identities to `TokenTag` and consumes their presentation class for highlighting. The shared crate knows no `TokenTag`. Naming-style warnings and structured compiler diagnostic construction remain compiler-local.

A future reservation change is also a MON format compatibility change. Update the shared rule, conformance tests and release-facing documentation together. Applications still own stored-data versions and migrations. This introduces no hidden grammar-version switch or automatic repair.

Apply the user-name predicate to MON record labels, supplied named arguments, nominal qualifiers, choice names, variant names and schema field/payload names. Apply it during schema preparation and text reading. Programmatic values/defaults cannot bypass it during validation and encoding. Invalid source names use the compiler's existing diagnostic lane. MON keeps its own error projection.

### 1.4 Shared named-entry boundary

The source notation already requires a field label and its `=` on the same logical line. Adopt that same rule in MON at the implicit root, explicit root, nested records and qualified payloads.

Spaces between the label and `=` remain valid. A line comment that consumes the rest of the label's line cannot join that label to `=` on a later line. Newlines after `=`, before the first entry and between comma-separated entries remain valid. Commas remain mandatory separators and one trailing comma remains valid.

A failed named-entry probe must restore its cursor without losing a real lexical error. Once a name is recognised as a label, a reserved-name failure cannot be swallowed and retried as a positional value or constructor.

This is an intentional MON acceptance tightening, not a compiler grammar expansion. Add release-facing documentation for the newly rejected spellings.

### 1.5 Numeric preservation

Retain the merged numeric system intact:

| Concern | Contract to preserve |
| --- | --- |
| Int/Float profile | All four 32/64-bit combinations. Standalone default remains Int32/Float64. Prepared schemas capture the profile immutably. |
| Public MON carriers | `Value::Int(i64)`, `Value::Float(f64)`, explicit integer variants, F16/F32/F64 carriers, Byte and existing exact-text Integer/Decimal variants. |
| Fixed identity | Int, I32 and I64 remain distinct even when widths match. Byte remains distinct from U8 and Char. Preserve `FixedScalar` ordering/discriminants used by compiler type seeding. |
| Integer literals | Whole-number spelling only, exact signed minima, complete U64 range and rejection of negative unsigned/Byte spelling including `-0`. |
| Binary literals | Destination rounding with ties to even, finite-result checks, subnormals and signed zero. Preserve the F16 exact midpoint correction and direct F32 parsing. |
| Decimal data | Scale is `u16`, validated from 0 through 256. Preserve exact-fit checks, trailing-zero/exponent meaning and the cheap zero path. |
| Formatting | Retain precision-specific shortest round trips. Ordinary Moth number-to-string formatting may print negative zero as `0`, while MON encoding preserves it as `-0.0`. |
| Map keys | Preserve all currently supported key families and their declared type identity. Binary floats and exact decimals remain ineligible. |

`SchemaType::Integer` and `SchemaType::Decimal` are Rust data-family names. They do not introduce Moth source types named Integer/Decimal or numeric constructor forms such as `U64(42)`.

Grammar validation and destination selection remain distinct. The compiler's receiving context chooses a type. MON's prepared schema chooses a destination. Their common materialisation helper executes that choice without a second numeric grammar or a fixed i32/f64 bottleneck.

Preserve source String-cast eligibility separately from literal receiving. In particular, moving the Byte literal materialiser does not authorise a String-to-Byte cast.

### 1.6 Bounded ownership and failure

Preserve owned output trees, immutable reusable schemas, input-independent successful values, original-input half-open byte spans and complete-result-or-first-error publication.

Keep the current limits and accounting, including the depth ceiling of 64, map-entry node charges, default expansion and numeric scratch charges. Validate capacities before allocation/expansion. A failed schema preparation still releases rejected deep tails without recursive cleanup overflowing the stack.

The extracted public error boundary remains `MonError`, not compiler diagnostics or infrastructure failures. Add normal `Display` and `std::error::Error` support without retaining input, compiler context or pre-rendered excerpts. The structured code, path, span and detail remain available independently.

This plan promises bounded codec policy and checked arithmetic, not a new process-wide out-of-memory recovery guarantee or a custom allocator.

## 2. Inspection findings to resolve

These are code-inspection findings, not claims of executed regression tests. Recheck the named owners at activation and record changed facts in local working notes.

| Finding | Current evidence | Required disposition |
| --- | --- | --- |
| Numeric extraction crosses more than `numeric_text`. | `mon/mod.rs`, `reader.rs`, `schema.rs` and `writer.rs` import fixed scalars, profiles, precision and decimal-scale policy from compiler datatypes. | Extract the actual neutral dependency closure. A file move plus copied numeric enums is insufficient. |
| Exact decimal text facts share a file with arbitrary-precision arithmetic. | `datatypes/number.rs` contains `NumberScale`, `NormalizedDecimalFacts`, `effective_decimal_scale` and `NumberValue`/BigInt arithmetic. | Split the text facts from the compiler value/arithmetic owner. Both consumers retain one decimal-facts implementation. |
| Identifier policy is duplicated and MON lacks source reservations. | `keywords.rs`, `symbols/identifier_policy.rs`, MON reader identifier helpers and `schema.rs::is_identifier`. | Share character/word/name predicates and enforce them at every MON name-bearing boundary. |
| Named-entry trivia differs. | MON `try_named_label` skips line breaks before `=`, while `call_arguments.rs::classify_parenthesized_expression` and `mon-syntax.mtf` reject that boundary. | Add a failing cross-parser regression, then restrict MON's label lookahead. |
| Extraction exposes formerly internal numeric preconditions. | `FixedScalarValue::binary_float` relies on a precision `debug_assert`. The formatter assumes a precision-exact carrier. Numeric parsing stores unchecked u32 digit counts. | Review newly public safe entry points. Make invalid external input a checked failure while preserving the fast valid-input path. |
| Map duplicate detection carries unnecessary simultaneous state. | `mon/mod.rs::MapKeyIndex` contains thirteen HashSets even though each prepared map has one key type. | Replace it with one schema-selected index representation. Preserve type checks, borrowed lookup, hashing policy and accounting. |
| Public codec tests are attached to the compiler package. | `tests/mon_public_api.rs` contains the expanded public API/profile suite. | Move the suite to `moth-mon`, split by responsibility where useful and retain only a small compiler re-export test. |
| Root module mixes public models and implementation machinery. | MON `mod.rs` also owns budgets, map indexes and fixed-value conversions. | Make the new `lib.rs` an API/ownership map. Move existing coherent responsibilities into private leaf modules without inventing a framework. |
| Moving files can remove audit and routine lint coverage. | `source_audit.rs` scans only `src` and `xtask/src`. Routine formatting/clippy select the root package. Other gates have package/root inventories. | Include the new packages at the same time as extraction. Verify coverage with failing gate fixtures. |
| Documentation contains physical-owner and status claims that become stale. | Compiler/build/MON references and `index.md` describe `compiler_frontend/mon` and a compiler-library-only API. | Update the permanent owners and status matrices separately, preserving all numeric and failure contracts. |

The numeric default-error projection also needs a narrow correction. In `schema.rs::validate_decimal_text`, scale failure calls `schema_error` directly while neighbouring value validation uses `CompletionContext`. Route the scale error through the existing `value_error(context, ...)` owner: an inexact default reports InvalidDefault and an inexact supplied value reports NumericScale. Add regressions for both before changing the call. Budget failures remain budget errors in either context.

## 3. Ownership and file moves

Create exactly two workspace crates. Keep coherent leaf files together rather than treating the illustrative filenames below as a reason for extra abstractions.

```text
crates/
  moth-lexical/
    Cargo.toml
    src/
      lib.rs
      identifier.rs
      words.rs
      numeric/
        mod.rs
        grammar.rs
        parse.rs
        format.rs
        binary16.rs
        profile.rs
        fixed_scalar.rs
        precision.rs
        decimal.rs
        tests/
  moth-mon/
    Cargo.toml
    src/
      lib.rs
      model.rs
      error.rs
      budget.rs
      map_keys.rs
      numeric.rs
      reader.rs
      schema.rs
      writer.rs
      tests/
    tests/
      public_api.rs
      support/
src/
  compiler_frontend/
    numeric_text/           # Retained compiler token/store adapters only.
    declaration_syntax/     # Stays compiler-local.
    ast/expressions/        # Existing call and expression owners stay here.
  compiler_tests/
    mon_syntax_parity/
```

If a reader/schema split becomes necessary for readability, split syntax traversal from receiving validation inside that same subsystem. Preserve the existing private raw-value handoff. This is not a streaming rewrite or a new public syntax tree.

### Move into `moth-lexical`

| Existing owner | Moved responsibility |
| --- | --- |
| `numeric_text/grammar.rs` | Numeric character and separator/exponent rules. |
| `numeric_text/token.rs` | Only source-independent kind/sign/exponent-sign vocabulary. Compiler tokens remain behind. |
| `numeric_text/parse.rs` | Whole-spelling classification/normalisation and token-free Int/Float/fixed-scalar materialisation. |
| `numeric_text/binary16.rs` | Existing binary16 rounding and exact decimal midpoint logic, with its tests. |
| `numeric_text/format.rs` | Precision-aware finite-float formatting and `FloatFormatError`. |
| `datatypes/numeric_profile.rs` | The existing profile/width/precision value types and their pure operations. Preserve fingerprint encoding. |
| `datatypes/fixed_scalar.rs` | Existing neutral scalar identities, exact carrier and range/precision facts. |
| `datatypes/numeric_scalar.rs` | Only `BinaryFloatPrecision` and its pure rounding operations. |
| `datatypes/number.rs` | `NumberScale` and the borrowed normalised-decimal facts used for exact scale and coefficient preparation. |
| `keywords.rs`, `symbols/identifier_policy.rs` | Neutral source words, identifier character rules and reserved user-name matching. |
| `compiler_messages/diagnostic_payload/types.rs` | The numeric-text reason vocabulary actually returned by the shared parser. Compiler diagnostic payloads/renderers stay local. |

Give the borrowed decimal facts the minimal read access needed by `NumberValue::from_normalized`. Preserve one analysis of coefficient/exponent facts in that path. Exposing a borrowed fact view does not justify a second normaliser, a general decimal AST or a NumberValue clone in MON.

Move or derive the fixed-float-to-precision mapping once beside the neutral scalar vocabulary. MON and compiler numeric domains consume it instead of retaining parallel F16/F32/F64 matches.

### Keep in `moth`

Retain `NumericLiteralToken`, `NumericLiteralStore`, StringId interning/remapping and token-based materialisation adapters. Keep `NumericScalar` and its TypeId/environment projections, promotion/conversion policy, `NumericOperator`, `NumberValue`, arbitrary-precision arithmetic and compiler numeric diagnostics.

Token-based adapters do useful work: resolve retained spelling and pass its known sign/destination into the shared service. They are not permission to keep empty copies of moved modules or forwarding functions whose only purpose is preserving old import paths.

Keep all compiler source/span stores, declaration shells, call argument routing, AST/TIR/HIR and build orchestration in their current semantic owners. Update imports directly. Preserve compact layouts and identity ordering where existing consumers rely on them.

### Dependency hygiene

Start both new packages at version `0.1.0` with the workspace's edition and lint policy. Use path dependencies for in-repository development. Include the license/readme metadata needed for independent use, but leave publishing and release automation outside this plan.

Move `ryu` to the package that uses it and remove the root direct dependency only if no root caller remains. Keep num-bigint, num-integer and num-traits with compiler arithmetic. Keep the current resolver, toolchain, root profiles, shared lockfile and validation resource settings unchanged.

No dependency, build script, development dependency or test utility in either extracted crate may pull in `moth` or `xtask`. Cross-parser coverage lives above both crates in the compiler's tests.

## 4. Cross-parser test design

### 4.1 Harness and oracle

Place the cross-system harness under `src/compiler_tests/mon_syntax_parity/` and register it under `cfg(test)` in `src/lib.rs`. Use the real compiler source path and the public `moth_mon` API.

Each fixture carries a stable case name, literal source text, the necessary Moth declarations/receiving context, an equivalent prepared MON schema, numeric profile and explicit expected outcomes. Add only the source/root envelope needed by each service. Preserve the literal's original bytes, whitespace, sign and qualifiers.

Reuse existing compiler source-test infrastructure and normal compilation/preparation services. If inspecting a typed argument or folded value requires a frontend-local adapter, keep it test-only beside that owner and return only the observations the suite needs. Do not widen production stage visibility, assemble raw stages from build code or use hand-constructed ASTs to prove parser behaviour.

For ordinary function calls, inspect the supplied argument values and their retained slot mapping. Calling an arbitrary source function at compile time is not needed to prove argument parsing and must not be added for the harness.

Assert each side against an independent expected result before comparing them. Two parsers agreeing with each other is insufficient when they share a faulty helper. Reuse existing scalar/field observations where available rather than adding a general serialisation bridge or a third production value model.

Compare declared numeric identity, exact integer values, float bits including signed zero, exact decimal coefficient/scale where representable, field/payload association and relevant ordering. Whole-value `PartialEq` on f64 payloads alone cannot prove signed-zero preservation. Source `Dec` observations should not be obtained by parsing their formatted display text.

Compiler-executing cases must participate in the existing instrumentation test fence where current source fixtures require it. Reuse `compiler_frontend::instrumentation::lock_counter_test`, which delegates to the timing-owned lock. Acquire it at one test/fixture boundary, not recursively in nested helpers. Keep pure lexical and codec tests independent of compiler instrumentation.

A diagnostic assertion names the intended error category/reason and useful source location. Compiler and MON error codes and message wording need not match. Adjust wrapper offsets for byte-span comparisons. A panic, infrastructure failure or unrelated type error never satisfies an expected syntax rejection.

### 4.2 Required corpus

| Family | Required cases and observations |
| --- | --- |
| Identifier shape | ASCII, underscores, Unicode alphabetic starts, alphanumeric continuation, invalid starts, combining/non-alphanumeric characters and UTF-8 boundary spans. Preserve the existing character policy rather than inventing normalisation. |
| Reserved names | Exact keywords, leading underscores, mixed case, fixed scalar names and Dec-family shadows. Include `__LoOp`, `_U64`, `dec01`, `dec257`, `DecBox`, `configuration` and `_configuration`. Test labels, qualifiers, variants and schema names. |
| Word roles | `true`, `false` and `none` as values succeed in valid receivers. The same spellings as user names fail. Quoted `"if"`/`"true"` data and string map keys remain valid. |
| Named-entry trivia | Horizontal spacing, newline after `=`, comments between comma-separated entries, trailing comma and EOF. Reject label-to-`=` line breaks, including CRLF and comment boundaries. Test first and later fields plus implicit/explicit roots. |
| Record/group shape | Empty and single-field const records, recursive records, duplicate labels, missing values/commas, trailing input and no positional anonymous fields. Grouped source expressions are classified separately from MON records. |
| Nominal construction | Explicit matching qualifier, wrong qualifier with identical fields, positional-then-named payloads, reordered named arguments, duplicate positional/named slots, unknown fields, wrong arity and nested qualified children. |
| Choice data | Qualified unit/payload variants, unknown variant, wrong choice qualifier, duplicate payload fields and payload ordering. Isolate contextual leading `::` as a named source-support gap. |
| Numeric spelling | `1_000`, `1_0.0_1`, lowercase exponents with signed/separated exponent digits, malformed separators, uppercase `E`, leading `+`, missing fraction/exponent digits and trailing junk. Preserve whole versus decimal/exponent categories. |
| Integer destinations | Every I*/U* minimum/maximum and adjacent rejected value, I64 minimum, U64 maximum and values above 2^53. Test negative unsigned/Byte spelling including `-0`, Byte 0/255/256 and decimal/exponent rejection. |
| Profiles | Run the profile-sensitive corpus under all four NumericProfiles. Fixed-width/Byte results are profile-independent. Keep Int/I32 and Float/F32 identity distinct under matching widths. |
| Binary rounding | F16/F32 midpoint neighbours, exact ties, finite maxima, overflow thresholds, subnormals and positive/negative zero. Compare bits and avoid wider-intermediate expectations. |
| Exact decimals | Scales 0, 1, 2, 18, 19, 255 and 256, rejected 257 schema scale, exact/inexact fractional values, trailing zeroes, positive/negative exponents and very large zero exponents. Keep huge nonzero expansions out of ordinary compiler parity cases. |
| Collections/maps | Common typed containers, nesting, decoded duplicate keys, escaped-equivalent string keys, numeric separator-equivalent keys, fixed-key identity and forbidden float/decimal key schemas. Isolate `{=}` source support. |
| Defaults/options | Equivalent declared defaults, exact missing-field insertion, required optional fields, explicit none and no deep merge. Test each receiving contract rather than assuming all source anonymous records have a schema. |
| Strings/characters | Common quoted escapes, quote/backslash boundaries, Unicode literal characters, delimiter/comment text inside strings and exact Char scalar count. Explicitly separate MON raw-newline preservation and source Unicode-escape gaps. |
| Source-only work | Valid source arithmetic, references, templates and ordinary calls with MON rejection before evaluation. Qualified nominal data is not a user-function invocation. |
| Writer consumption | Encode with MON, decode with MON and feed compatible emitted literal fragments into real source receivers. Assert typed values, not identical text formatting. Classify contextual-choice/escape output through the named source gaps. An omitted nominal qualifier is a schema/source receiving distinction, not permission to add implicit nominal construction to Moth. |

Use explicit independent numeric expectations for difficult cases. For example, binary16 `1.00048828125` is a tie that rounds to 1.0. The slightly greater spelling `1.0004882812500000000001` rounds to 1.0009765625. Both need separate source and MON assertions. Preserve existing numeric regression cases instead of replacing them with a single round-trip assertion.

Supplement the curated corpus with small deterministic combinations of whitespace, nesting, qualifiers and literal spellings. Bound the generated count and print a reproducing case name/text on failure. No random failing seed, ignored test or unbounded Cartesian product belongs in the normal gate.

### 4.3 Intentional differences and current gaps

Keep one small named table in the test module and a matching concise documentation inventory. It must distinguish:

- MON document envelopes, including implicit/empty roots, from expression fragments.
- Schema-supplied information from source inference and explicit nominal construction requirements.
- Literal data from source expressions, bindings, imports, mutable argument access and declaration syntax.
- MON raw quoted-text preservation and its supported escape contract.
- Current undelivered source support for explicit empty maps, contextual choice construction and Unicode escapes.
- Representation/resource limits for exact-text data versus materialised compiler values.

Every executable gap case asserts its actual current outcome and names the missing capability. A newly implemented capability should make that case fail until it is promoted into common parity coverage. No wildcard allowlist, automatic expected-failure acceptance or skipping the entire choice/string/map family is acceptable.

The permanent references explain intended semantics. Progress and roadmap text carry implementation gaps. A label/newline or reservation mismatch is a required fix here, not an additional exception.

## 5. Implementation slices

Execute serially in an isolated feature worktree. Each non-trivial slice follows test-first development for changed behaviour, focused verification and the Slice review. Record exact commands/results in working notes. Intermediate commits do not each require full merge validation.

### Slice 0: Establish the baseline and publish the contract

**Files:** Permanent authorities from the reading routes, `docs/roadmap/roadmap.md` and local `tmp/` working notes.

- [ ] Record the active revision, worktree status, toolchain and available integration prerequisites locally. Recheck the inspection findings against that tree.
- [ ] Run the existing MON public API tests, MON/numeric subsystem tests and `just validate`. Diagnose baseline failures separately from extraction work without weakening gates.
- [ ] Inventory existing test counts, crate dependency edges and gate scan roots. Use Cargo metadata and owning test modules, not test names guessed from this plan.
- [ ] Record representative compiler tokenisation and MON record/numeric/map timings before editing. Keep the input, toolchain, profile and repetition policy in local notes so slice 7 can make a like-for-like comparison. Use existing performance tooling or a small temporary caller, not a new committed benchmark framework.
- [ ] Read canonical string, map and choice contracts before assigning current cases to common syntax or a named gap.
- [ ] Publish the narrow accepted ownership, name-reservation, named-entry and parity contracts in their permanent owners. Explicitly preserve the pending source capabilities.
- [ ] Register this work first in the serial roadmap order. Keep the activated baseline SHA out of the committed plan and in working notes.

**Exit:** A reproducible baseline and explicit contracts for every intended behaviour change. No unexplained widening of source or MON grammar.

### Slice 1: Extract the neutral numeric dependency closure

**Files:** `crates/moth-lexical/`, root Cargo files, current `numeric_text/`, the numeric datatype files in section 3 and numeric diagnostic reason imports.

- [ ] Create the library package with workspace lints and the existing formatting dependency. Add the workspace member and root dependency.
- [ ] Move grammar, sign/kind vocabulary, binary16 conversion and precision-aware formatting with their focused tests.
- [ ] Move the existing profile, fixed-scalar and precision leaves. Preserve exact identity order, bit payloads, signed-zero handling and profile fingerprint bytes.
- [ ] Separate NumberScale/borrowed decimal facts from NumberValue arithmetic. Keep the same facts feeding MON scale checks and compiler coefficient preparation, without copying the algorithm or rescanning merely to bridge crates.
- [ ] Move the numeric-text reason vocabulary to the shared service. Keep compiler diagnostic codes, contextual payloads and renderer policy in their existing owners.
- [ ] Leave token/StringTable adapters and NumberValue-producing functions in the compiler. Update all imports, including casts, build-input materialisation, highlighting and numeric backend consumers of moved facts.
- [ ] Remove superseded leaf files and empty import-preservation facades. Retain an adapter only when it resolves compiler-owned data or applies compiler-owned receiving policy.
- [ ] Add regression tests for any changed public precondition checks before implementing them. Preserve direct F32 rounding and F16 midpoint correction.
- [ ] Extend affected source/gate inventories for this new member immediately, using the existing gate owners described in slice 6.

**Verify:** `cargo test -p moth-lexical`, the existing compiler numeric/profile/Dec test filters and targeted affected source cases. Check `cargo tree -p moth-lexical --edges normal,build` for only intended dependencies. Run `cargo check -p moth` in the normal no-feature lane.

**Exit:** One numeric spelling/materialisation/formatting owner, no compiler dependency in the new crate and no arbitrary-precision implementation moved into it.

### Slice 2: Extract word and identifier policy

**Files:** `crates/moth-lexical/src/identifier.rs`, `words.rs`, compiler `keywords.rs`, `symbols/identifier_policy.rs`, tokenizer, type-annotation and highlighter consumers/tests.

- [ ] Move the character predicates, reserved shadow matching and Dec-family reservation into neutral functions.
- [ ] Build one source-word inventory whose exact recognition and reservation metadata share ownership. A small local declarative macro is acceptable only if it removes real repeated inventory structure. Keep matching allocation-free and avoid a runtime registry.
- [ ] Leave compiler token mapping and naming diagnostics local. Derive highlighting categories from the shared classification rather than adding another word list.
- [ ] Preserve reserved-only words and Dec's symbol-token treatment. Keep canonical Dec type-name recognition distinct from the broader reservation predicate.
- [ ] Test valid identifiers, reserved variants and near misses against independently written expected cases. Verify all current dedicated source-word tokens still receive their exact payload/category.
- [ ] Update compiler callers directly and delete duplicate spelling/predicate owners. Preserve surrounding declaration and call semantics.

**Verify:** New lexical tests, compiler keyword/identifier/type-spelling tests and affected source/highlighter cases. Exercise exact `this`/`This`, true/True and Dec-family boundaries.

**Exit:** Shared name laws with one inventory, no compiler token model in the leaf crate and no unrelated source reservation change.

### Slice 3: Extract the codec and its public coverage

**Files:** `crates/moth-mon/`, `src/compiler_frontend/mon/`, frontend module declarations, `src/lib.rs`, `tests/mon_public_api.rs` and Cargo files.

- [ ] Move reader, schema, writer and tests into `moth-mon`. Import only the neutral numeric/lexical APIs and the standard library.
- [ ] Move public models, errors, budgets, numeric conversions and map-key support out of the former large root into their coherent private leaves. Keep `lib.rs` focused on the public surface and ownership documentation.
- [ ] Preserve the expanded public Value/SchemaType variants and profile API. Re-export the shared numeric-profile types from `moth-mon`.
- [ ] Move the external public API suite into the new package. Split its large numeric/profile and resource/schema sections where that improves navigation, with small local support rather than a new test library.
- [ ] Replace the compiler facade with the single crate re-export and remove `compiler_frontend::mon` completely.
- [ ] Retain one small compiler-facing smoke/type-identity test that passes a schema/value across the direct and re-exported paths. Do not run two copies of the full codec suite.
- [ ] Remove unused private writer/schema exports and their lint allowances after checking actual uses. Keep any required pretty writer coverage private.
- [ ] Move public API examples to the crate docs using `moth_mon`. Add a simple independent-consumer example.
- [ ] Extend package/gate coverage for the second member in this slice.

**Verify:** `cargo test -p moth-mon`, `cargo test -p moth-mon --doc`, the compiler facade test and dependency-tree checks. Existing codec numeric/profile/default/budget tests must execute in the standalone package.

**Exit:** The engine can depend only on `moth-mon`. The compiler facade is the same API with no parallel owner.

### Slice 4: Close identifier and named-entry parity gaps

**Files:** MON reader/schema name validation, shared identifier consumers, new cross-parser test module, MON format/naming documentation and affected codec tests.

- [ ] Add paired failing tests for reserved labels, qualifiers and variants, including fixed widths and Dec-family shadows.
- [ ] Add tests rejecting invalid names in schemas, nested payloads, supplied programmatic values and defaults. Confirm quoted data/map keys remain unrestricted by identifier reservation.
- [ ] Add paired newline-before-`=` regressions for root, nested record and nominal payload forms, plus positive newline-after-`=` cases.
- [ ] Replace MON-local identifier predicates with the shared owner. Validate names at the correct semantic positions, keeping true/false/none as literals rather than potential reserved nominal constructors.
- [ ] Correct named-label lookahead without parsing an expression or swallowing a committed failure. Preserve exact error byte ranges and useful paths.
- [ ] Apply the same label boundary in implicit and explicit documents. Update affected positive fixtures that previously depended on the divergent syntax.
- [ ] Use existing MonError categories where appropriate: InvalidIdentifier for authored name errors and InvalidSchema for invalid schema names. Preserve the established default/budget projections.

**Verify:** `cargo test -p moth --lib mon_syntax_parity`, standalone MON tests and the relevant source integration cases. Establish actual typed diagnostic expectations from the owning diagnostic vocabulary rather than accepting any failure.

**Exit:** The demonstrated gaps are fixed in code and tests, not reclassified as exceptions.

### Slice 5: Complete the typed conformance corpus

**Files:** `src/compiler_tests/mon_syntax_parity/`, narrowly required frontend test adapters and the existing source integration case owners.

- [ ] Complete the corpus in section 4 with named tests for identifiers, list structure, constructors, choices, numbers and text boundaries.
- [ ] Exercise both parsers with the same literal bytes and equivalent receivers. Observe source typed arguments/values through existing compiler data rather than a new MON parser bridge.
- [ ] Add all four numeric profiles, fixed-width boundary values, F16/F32 rounding regressions and scale-256 exact decimal coverage.
- [ ] Add explicit expected results, not merely pairwise equality. Compare float bits and exact typed integer values without display-string conversions.
- [ ] Add writer-to-reader/source consumption tests for the supported overlap, including qualified-input validation before writer qualifier omission.
- [ ] Add source-only-expression rejection cases and executable named cases for existing source-support gaps. Test available common forms within each family rather than skipping the family.
- [ ] Add bounded deterministic syntax perturbations. Keep setup reusable per compatible schema/profile and avoid compiling unrelated whole projects per lexical case.
- [ ] Use the existing instrumentation guard for compiler-executing fixtures and verify the suite alongside timer/counter tests. Add no global state, ad hoc lock or blanket serialisation of the standalone codec suite.
- [ ] Review overlap with existing tests. Keep secondary tests only when they protect the cross-crate boundary rather than repeat a local numeric algorithm test.

**Verify:** The complete parity module in `cargo test -p moth --lib mon_syntax_parity` and the existing workspace unit gate. Temporarily perturb one MON-only parsing decision in a local uncommitted experiment and prove the suite fails for the intended case, then restore it before recording final results.

**Exit:** The real compiler and public codec are both exercised. No production path parses both forms and no new public compiler API exists solely for tests.

### Slice 6: Harden the standalone boundary and preserve tool coverage

**Files:** Shared numeric constructors/parse/format helpers, MON error/budget/map-key leaves, `xtask` gate owners, `justfile`, `.github` validation configuration where needed and testing documentation.

- [ ] Add Display/Error implementations for MonError. Test normal `?` propagation into an ordinary Rust error boundary and access to structured data after input is dropped.
- [ ] Audit newly public low-level numeric operations for preconditions previously protected only by compiler visibility or debug assertions. Reject invalid scalar/precision inputs with typed failures. Keep normalised-input preconditions explicit and reuse existing validated parsed facts rather than introduce reparsing or a general proof-wrapper framework.
- [ ] Guard numeric input/count representation before reservation and accumulation can overflow. Test the arithmetic/length boundary directly without allocating gigabytes or adding a configurable production counter limit solely for tests.
- [ ] Correct the inexact-decimal default path through `value_error` as specified in section 2. Test InvalidDefault for an inexact default, NumericScale for an inexact supplied value and the original budget category when a budget is exhausted first. Review adjacent paths for the same context bypass without creating a second error mapper.
- [ ] Replace the thirteen-set MapKeyIndex struct with one schema-selected enum/index. Preserve homogeneous key validation before indexing, exact numeric identities, collision-safe hashing, insertion order and charge-before-clone behaviour. Avoid a tagged allocation per key or conversion through display strings.
- [ ] Retain tests for all supported key families, separator/escape-equivalent duplicates and mismatched programmatic key variants. Compare index size and representative map behaviour before/after without making an incidental size a permanent public contract.
- [ ] Recheck new failure paths for bounded rejected-schema/default cleanup and absence of partial output. Keep the private raw-parse/validation split unless an independently demonstrated defect requires a local correction.
- [ ] Extend source-audit, feature-lane and honesty-audit package/root inventories through their existing owners. Cover both extracted source trees and their applicable tests. Preserve deterministic fail-closed walks and report actual scanned roots.
- [ ] Add gate self-tests with an intentional violation or misspelled feature in a new-crate fixture. A scanner returning zero findings after ignoring the crate is not success.
- [ ] Preserve routine formatting/clippy coverage for code moved out of moth. Select moth and both new libraries explicitly in routine recipes, leaving the broader workspace/all-target gates distinct.
- [ ] Keep workspace unit tests and add explicit package-scoped coverage where feature unification could hide the compiler's no-timer lane. The new crates acquire no timer dependency or gratuitous Cargo feature.
- [ ] Update explicit package eviction lists to include the new members where the command promises workspace eviction. Automatic maintenance still preserves Cargo build artifacts.
- [ ] Verify dependency direction from Cargo metadata, including development/build edges and renamed packages. Reuse the existing architecture/gate machinery rather than create a second audit command family.

**Verify:** Package-scoped tests/checks, source audit, feature-lane check, honesty audit and gate self-tests. Confirm the changed routine formatting/clippy commands inspect both extracted packages. Run `just validate` at this integrated checkpoint.

**Exit:** Extraction has not weakened public error handling, resource policy or repository coverage.

### Slice 7: Documentation, standalone use and performance review

**Files:** Permanent documentation table below, crate readmes/doc comments, `index.md`, progress matrices and roadmap text.

- [ ] Complete the documentation changes and inspect the before/after diff for lost ownership, numeric, default, span or failure contracts.
- [ ] Compile and run a small consumer outside the workspace membership using only a path dependency on `moth-mon`. Use a local `tmp/mon-consumer` package with its own empty `[workspace]` table and a separate target directory to avoid nested Cargo locks or benchmark interference.
- [ ] The consumer prepares a profile-selected schema, round-trips fixed-width/Byte and F16/F32 values, checks a structured error and uses ordinary Rust error propagation. It imports no compiler crate or private module.
- [ ] Inspect the consumer's Cargo dependency graph. The compiler, xtask, backend, file-watching and arbitrary-precision arithmetic dependencies must be absent.
- [ ] Compare representative compiler tokenisation and MON parse/encode behaviour with the activation baseline using the same toolchain/profile/input. Include a small record, a numeric-heavy nested record and a map workload. Report measured allocation/runtime effects only when measured.
- [ ] Use existing performance infrastructure and non-recording commands. Investigate a reproducible regression before adding inline attributes or changing compilation profiles. Add no broad benchmark framework for this move.
- [ ] Rebuild generated documentation through the compiler when the repository expects it. Never edit `docs/release` by hand.

**Exit:** Independent use is exercised, docs describe the intended final boundary and no unexplained performance regression remains.

### Slice 8: Final verification and Slice review

- [ ] Run the final commands in section 7 on the integrated tree. Fix failures in their owners and rerun any evidence invalidated by later changes.
- [ ] Read each changed module from its entry point. Confirm one owner per rule and no reverse compiler dependency, duplicate numeric implementation or abandoned private path.
- [ ] Audit API/abstraction shape, diagnostic context, public safety, default/budget accounting, map-key ownership and exact numeric preservation.
- [ ] Review test ownership and coverage. Confirm the public codec suite, actual-source parity corpus, re-export identity test and gate fixtures execute in their intended packages/configurations.
- [ ] Confirm docs/index/progress changes obey their separate ownership rules. Mark materially affected prior audit coverage stale, without claiming a new structured audit was run.
- [ ] Preserve the plan's unique accepted contracts in permanent owners, then delete this plan and remove its roadmap bullet in the completion commit. Keep all still-undelivered MON source capabilities under their existing permanent/status owners.
- [ ] Report exact validation results, dependency evidence, performance observations and any remaining unrelated blocker. A Slice review does not authorise merging or claim a separate formal audit.

## 6. Documentation changes

| File or owner | Change |
| --- | --- |
| Compiler design overview | Define physical crate ownership and allowed dependency direction. Keep the source shared-argument and declaration owners compiler-local. Clarify shared numeric leaf facts versus compiler typing/arithmetic. Document the supported convenience re-export. |
| Build design | Replace compiler-library-only descriptions of MON. Preserve caller-owned IO, profile selection and ordinary resource/output planning. Crate extraction creates no new builder, command or source kind. |
| Data-layout design, MON data handoff | Update the codec owner/path while preserving caller-borrowed input, owned results, exact spans, allocation accounting and the public MonError projection. |
| `language-overview/mon-syntax.mtf` | State the common notation contract with equivalent receiving context, independent parsers and conformance tests. Keep grouping, declarations, nominal construction and record semantics distinct. |
| `language-overview/comments-and-naming.mtf` | Identify the shared identifier/reserved-name law, including fixed numeric and Dec-family reservations. Preserve lexical validity versus style-warning distinctions. |
| `mon/mon-format.mtf` | Describe standalone Rust use, strict reserved user names and the same-line label/equals rule. Keep literals, data framing, escapes, defaults, supported numeric carriers and receiver limits precise. |
| MON teaching page and examples | Show `moth_mon` for independent Rust use. Preserve `moth::mon` as a convenience for existing compiler clients. Include a typed/profile example and an InvalidIdentifier example. |
| Numeric references and cheatsheet | Update only affected ownership/API descriptions and stale public Number/NumberN wording where encountered. Use Dec/DecN for current source syntax. Avoid a broad internal Rust rename. |
| Crate readmes and rustdoc | Explain inputs, immutable schema reuse, owned values, error traits, supported numeric profiles and data-only restrictions. Keep the full API example in the canonical crate rather than duplicated long facade docs. |
| Testing/validation guides and CONTRIBUTING | Document the real-source parity suite, package-scoped checks, supported gap cases and changed scan/package coverage. Keep gate cadence and profiles unchanged. |
| `index.md` | Add the two crate maps, move numeric/codec ownership descriptions and locate the cross-parser harness. Remove stale private paths. |
| Progress matrices | Record standalone crate availability, lexical hardening and cross-parser coverage. Keep language-runtime codec support and undelivered source syntax separate. Do not mark them delivered by the Rust move. |
| Roadmap and deferred MON notes | Insert this plan first. Describe later MON work as consuming the standalone codec and shared policies. Keep source-support gaps explicit and remove this work item when completed. |

Architecture documents describe the accepted final design, not phase progress or exhaustive Rust API inventories. The progress matrices and work notes describe delivery. Resolve only conflicts within this plan's scope and preserve unrelated accepted design.

## 7. Validation and completion criteria

Use targeted checks during implementation. New package names and the new parity module below are planned deliverables. Existing integration case IDs and compiler test filters are selected from the actual inventory at activation. Confirm filtered commands execute the intended tests. A successful command that selects zero tests is not verification.

```bash
cargo test -p moth-lexical
cargo test -p moth-mon
cargo test -p moth-mon --doc
cargo check -p moth
cargo test -p moth --lib mon_syntax_parity
cargo tree -p moth-mon --edges normal,build
cargo tree -p moth-lexical --edges normal,build
cargo fmt --all --check
cargo clippy -p moth-lexical -p moth-mon --all-targets
just source-audit
just feature-lane-check
just test-honesty-audit
just validate
```

The independent consumer is a one-time final boundary check, not a recursive Cargo invocation inside every unit test:

```bash
cargo run --manifest-path tmp/mon-consumer/Cargo.toml --target-dir target/mon-consumer-smoke --offline
cargo tree --manifest-path tmp/mon-consumer/Cargo.toml --edges normal,build
```

Run it after the normal dependency fetch/build has populated the required local cache. Missing cached dependencies are an environment issue, not evidence for or against crate independence.

The final merge-readiness gate is:

```bash
just validate-full
```

It applies to the final integrated tree, including documentation and retirement changes. Follow the current validation guide for platform/feature coverage and the integration runtime prerequisite. Report unavailable checks honestly rather than substituting weaker assertions or reducing the gate.

Completion requires all of the following:

- The engine-facing dependency builds independently and shares the same types exposed through `moth::mon`.
- Both parsers consume one identifier/reserved-word and numeric-text implementation. Declaration and expression parsing remain compiler-local.
- All merged numeric profiles, widths, exact decimal scales and data carriers survive the extraction unchanged.
- Demonstrated reservation and named-entry mismatches are fixed with real-source cross-parser regressions.
- The conformance suite checks expected typed values and intended rejections, with bounded explicit differences/gaps rather than silent skips.
- The standalone error, resource-limit, default and map contracts remain complete and tested.
- The new crate sources/tests are included in the appropriate lint, feature and audit gates.
- Permanent docs preserve every accepted contract and distinguish independent Rust availability from future language/runtime support.
- Superseded private owners, copied rule tables, unnecessary exports and duplicated test suites are removed.
- Final validation and Slice review have evidence for the resulting tree.

## Roadmap insertion text

Place this entry first under `# Plans`, ahead of the currently queued post-numerics work. Mark it active on the bullet when execution starts.

```markdown
- [MON crate extraction and syntax parity](./plans/mon-crate-extraction-and-syntax-parity-plan.md) - Extract standalone moth-mon and the shared moth-lexical owner after expanded numerics. Unify identifier/reserved-word policy, preserve profile-aware numeric materialisation, add real compiler/codec parity coverage and harden MON plus workspace validation before other serial work resumes.
```
