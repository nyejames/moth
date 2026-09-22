# MON v1: Rust-facing compiler tooling

## Status

- Status: active, Phase 3 accepted; the first delivery remains Rust tooling only.
- Current slice: Phase 4, public Rust integration and bounded hardening.
- Blockers: none for this Rust-only checkpoint. Moth-native operations, source-parity changes and the static builder remain deferred.
- Next action: export the narrow Rust service and run the external consumer and hardening checks.

## Purpose and delivery boundary

Deliver Moth Object Notation as a small, strict data codec exposed by the `moth` Rust library. A separate Rust game engine needs it for initial save files and other data before Moth's runtime language and Wasm backend are ready. Early use should harden the format without adding a second external format dependency to that engine.

This plan runs immediately after shared MON syntax and nested const records. It implements in-process Rust document encoding, nested-value encoding and schema-checked document decoding. It is not a Moth language integration or project-builder implementation.

The narrowed delivery supersedes the earlier proposal to implement the codec, Moth operations and a static asset builder together. Keep the accepted eventual language contract, but mark it as deferred wherever it is not delivered here.

| Deliver here | Preserve for later integration |
|---|---|
| Literal MON grammar, owned data and static schema services callable from Rust | Compiler-owned `$mon` convenience, with invocation syntax still undecided |
| Explicit document and nested-value writers, strict complete-document reader | Moth-side encode/decode operations and checked anonymous-record receiving contexts |
| Defaults, eligibility, qualifiers, numeric conversion checks and bounded failures | Automatic schema extraction from ordinary Moth types and generic instances |
| Reuse of compiler lexical, argument and literal-data owners | Source-level `{=}` cutover, Unicode escape support and contextual `::Variant` construction where not already delivered |
| Native Rust consumer example and public API tests | Static `.mon` project builder, CLI/scaffolding/source-file integration and output ownership |
| No JSON/RON/KDL intermediate or required external codec API | Moth runtime code generation, Wasm/JS bindings and engine UI lifecycle integration |

Rust calls execute at host runtime. That is not a claim that compiled Moth programs can call MON services. A `.mon` file is data, not a new compilable Moth source kind in this delivery. The codec accepts text supplied by its caller and performs no file IO, project discovery, source imports or build execution.

## Sequencing and activation

The roadmap owns serial order. The required interlude is shared MON syntax, this Rust-facing MON checkpoint, Wiring, native result slots/Core constant evaluation and explicit data-layout Phase 4 reactivation. Existing package pauses remain in force.

V1 does not wait for general directives, runtime anonymous records, the later numeric implementation, collector-free memory or the completed Wasm backend. It neither implements those features early nor replaces their owners. Retain exact numeric data independently of whether a corresponding Moth runtime type has landed.

At activation, record the full revision, worktree status and baseline commands in local working notes. Refresh current paths, source/token ownership and diagnostic APIs. Do not pin a speculative baseline in this queued plan. Use the shared argument owner actually delivered by the preceding checkpoint rather than restoring its old parser structure.

Place the separate Moth-native integration and static-builder follow-up at the end of the roadmap. Completion of this plan leaves that entry and its deferred progress rows intact.

## Required reading and authorities

Read `AGENTS.md`, the full style guide, the language cheatsheet, both architecture references, `docs/compiler-data-layout-design.md`, both progress matrices and the roadmap's plan-maintenance rules. Use `docs/src/developer-docs/language/overview.mtf` to find the canonical unsuffixed references for MON syntax, records, struct construction, choice construction, collections/maps, numerics, strings/characters, options, defaults, templates and resources.

Read the testing and validation routes before selecting coverage or final gates. Read the memory overview and its value/copy routes for the distinction between data equality and allocation identity. Use `index.md` only to locate current owners.

The maintainer's interview and later Rust-only scope decision supply the accepted changes. Phase 1 publishes those changes into permanent authorities before implementation. Thereafter, the authorities own semantics and this plan owns delivery. A stale statement that MON input is merely const-required does not permit expression evaluation in its reader.

## Accepted format contract

### 1. Shared syntax and literal data

MON syntax names Moth's shared argument/value-construction notation. Moth source may use expressions in that notation. The MON format is a restricted literal-data language using that syntax.

Every value in a completed document must be a supported literal form. Reading MON never evaluates arithmetic, templates, calls, casts, field projections, variables or constant references. Reject those forms even when the compiler could fold them. Rejection happens before expression evaluation, not after evaluating and inspecting a result.

Constructor-shaped record and choice literals are data forms, not executable calls. Their arguments recursively obey the literal restriction. For example, `Widget::Text("Gold")` may describe literal choice data, while `Widget::Text(make_label())` is invalid MON.

A folded const template is an ordinary String that an encoder can quote. An evaluated runtime template is also an ordinary serialisable String. An unevaluated template is not a serialisable value. Compile time versus runtime concerns the producer and never changes document validity. V1 does not implement either producer inside compiled Moth.

### 2. One root record

A complete document contains exactly one record. Its outer parentheses may be omitted. These forms have the same record meaning:

```text
title = "Strategy",
window = (width = 1280, height = 720),
```

```text
(
    title = "Strategy",
    window = (width = 1280, height = 720),
)
```

A lone scalar, collection or choice is not a document. Put it in a named root field. The explicit empty root is `()`. Empty and comments-only input denotes an empty root record. Validation still checks every required field.

Use commas between entries at every level, with an optional trailing comma. Newlines are whitespace, not an alternative separator. Root fields resemble declarations but create no bindings, scope or references to earlier fields. After an explicit root, only permitted whitespace and comments may remain. Multiple roots and concatenated documents are invalid.

The file and in-memory document grammars are identical. Root elision uses the existing cursor and original offsets, not a copied string with synthetic parentheses that shifts diagnostics.

### 3. Recursive data values

Support records, ordered collections, maps, quoted strings, characters, booleans, numeric literal data, `none`, literal choices and qualified nominal record data. Nest these recursively subject to resource limits and the receiving schema.

Record fields have unique identifier labels and explicit `= value` entries. They are named-only and carry no annotations, declaration qualifiers, mutable markers or shorthand initialisers. `()` is an empty record, never a tuple or unit value.

MON collections may contain different literal kinds and different record shapes. The receiving schema enforces its element type. This changes no Moth collection typing rule and adds no `Any` type to Moth.

The data format represents value trees. Repeated shared source values are encoded at each occurrence. Decoding establishes no preserved alias relationship or allocation identity. Cyclic values fail encoding. Finite values of recursive schemas are distinct from cyclic values, but this work adds no recursive Moth type feature. Applications encode graph links as ordinary identifiers and reconstruct them explicitly.

### 4. Maps and explicit emptiness

Retain the distinct map form:

```text
scores = {"Priya" = 10, "Rob" = 12},
empty_scores = {=},
empty_items = {},
empty_record = (),
```

`{=}` always means an empty map and `{}` always means an empty collection. A map schema rejects `{}`. A collection schema rejects `{=}`. Schema context never changes a container's kind.

Maps preserve insertion order. Keys use the supported Moth map-key families: String, Int, Bool and Char. Bare names are references, not key shorthand, and are invalid as MON map keys. The schema checks the declared key/value types. Map entries and collection items cannot be mixed.

Reject duplicate keys rather than retaining the first or last value. Compare decoded keys under the receiving key contract, so different escape or numeric spellings cannot evade uniqueness. String and Char remain distinct, and Bool is not a numeric key. Record labels remain identifiers, while map keys are data. A string-keyed map does not become a record.

The same explicit empty-map distinction is accepted for eventual Moth source. This Rust-only delivery does not silently change source `{}` contextual behaviour. Record that source cutover as deferred unless another authorised syntax change has already delivered it.

### 5. Literal structs and choices

Both nested record forms are accepted:

```text
size = (width = 1280, height = 720),
size = Size(width = 1280, height = 720),
```

These are alternative examples, not duplicate fields in one document. Anonymous records use named entries. Qualified nominal record literals follow the shared constructor argument rules. They do not invoke user code or create a Moth structural-conversion rule.

Choice data uses Moth-shaped variants rather than an invented tagged-record mapping:

```text
widgets = {
    ::Text("Gold"),
    Widget::Progress(value = 0.4),
    ::Ready,
},
```

A unit variant has no parentheses. A payload variant has parentheses. Positional arguments precede named arguments, target slots are routed once and every required slot is supplied exactly once. Preserve the ordinary choice rule that payload fields have no defaults. Unknown variants, duplicate payload fields, wrong arity and payload type mismatches fail.

The receiving schema determines the expected nominal type. `::Text(...)` uses that expected choice. An explicit `Widget` or `Size` qualifier is an additional checked assertion, never ignored and never used to select another schema. Matching payload shapes do not make unrelated nominal types interchangeable. A schema-free internal parse may preserve qualifiers without claiming they have been validated.

V1 uses schema-declared names with the shared identifier grammar. An unknown or mismatching explicit name fails. Freeze the exact accepted qualifier spelling and collision rules before code is written. Avoid global registries, imported source namespaces and implicit short-name lookup. Future source aliases are resolved by their source producer, not by standalone data reading.

Normal typed encoding writes unqualified named records and unqualified choices with named payload fields. It checks any supplied qualifier before omitting redundant text. The root remains the ordinary unqualified document record.

Contextual `::Variant` source construction is a later Moth feature unless already implemented independently. Accepting literal data here does not enable untyped variant-name searches in Moth source.

### 6. Numeric data and strict conversion

Use the common numeric lexical owner. Retain a lossless numeric literal representation until the destination is known. Never route all numbers through `f64`, an Int token payload or display formatting.

Whole-number spelling and decimal/exponent spelling remain distinguishable. Signed numeric literals are allowed without allowing general unary expressions. Decimal data such as `3.0` fails an Int target even when mathematically integral. Range checks are strict. Formatting cannot rewrite it as `3` and change which targets accept it.

Finite Float materialisation follows the numeric authority. Reject non-finite source values and conversion results. Float encoding must round-trip the supported Float value, including its specified signed-zero behaviour, and preserve the decimal category when needed.

Exact Number/NumberN mapping uses the destination's declared scale and exact conversion rules. A scale-two target accepts exactly representable `1.2` but rejects `1.239` without rounding. Source type identity and decimal scale need not be recoverable from an untyped document. Encoding and decoding under the same supported schema preserve the data value.

V1 must retain and round-trip large integer and exact decimal data before the later Moth numeric runtime exists. Reuse the numeric token/store and lexical helpers. Perform only bounded literal normalisation, range and scale checks needed for the codec, not a second arithmetic runtime. Publish which native schema materialisations are implemented. Unsupported concrete compiler-type adapters fail explicitly rather than narrowing or converting through Float.

Phase 0 fixes and tests the exact exponent-to-integer policy, separator rules, signed-zero handling and scale limits against the shared numeric authority. These lexical/conversion details are acceptance gates, not permission to guess from Rust parsing defaults or reopen the accepted strictness.

### 7. Strings and characters

Strings use double quotes and Char uses single quotes. Decode characters as exactly one Unicode scalar value. Preserve string contents without Unicode normalisation or implicit line-ending conversion. Templates are producer syntax only and are not accepted string literals in MON.

Support lossless encoding of every supported Unicode string and character, including control characters through a shared Unicode escape. Publish the exact escape spelling, malformed-escape rules and quote handling before implementation. Reuse one escape owner rather than implementing MON-only replacement chains. Invalid Unicode scalar escapes and invalid input encoding fail.

Keep any undelivered Moth-source Unicode escape change explicitly deferred. Sharing the decoding primitive does not automatically widen the source language. Arbitrary byte buffers and binary encodings are outside v1.

### 8. Static schemas and strict records

Schemas define ordinary data shapes, field types, nominal identities, choice variants and explicit default values. They do not define a second expression language. Records are closed. Every unknown field fails, including a misspelling of a field whose correct name has a default. Maps provide the explicit dynamic-key surface.

The accepted eventual Moth schema authority is an ordinary Moth type and its field defaults. Construction receives normal type checking. An explicit anonymous-record MON boundary may check serialised shape without granting implicit structural conversion to nominal types. Static schema importing, exporting and forwarding should reuse ordinary Moth mechanisms when that later integration is implemented.

V1 supplies a small immutable Rust schema description for host callers. The engine defines its save-data contract once and supplies it explicitly. Reuse existing compiler-owned type/field data where its ownership and dependency direction fit, but do not require a module compilation, expose donor-local TypeIds or force hosts to manufacture a mutable TypeEnvironment merely to decode a save.

Rust has no automatic field inspection promised by this plan. A schema-checked owned MON value is the initial codec result. An engine may map that value to its native struct using ordinary Rust functions. Demonstrate this in the public consumer example. Automatic derives for arbitrary Rust types, procedural macros, Serde traits and a broad reflection framework are not prerequisites. Automatic extraction from Moth types belongs to later integration.

Validate a static schema and its defaults when preparing it, before it processes documents. Reuse the immutable prepared schema across calls. Fix schema misuse through a structured failure, not a panic. V1 adds no schema files, remote schema loading, runtime schema editing, registry or predicate DSL.

### 9. Required fields, options and defaults

Optionality permits `none`. Only an explicit field default permits omission. A missing optional field without a default is an error. Missing fields, explicit `none` and empty values remain distinct.

Defaults are prevalidated data, not runtime factories. Apply a default only at the exact missing field. A supplied `none` is validated as `none` and never replaced by a default. A supplied nested record is validated as that record, not deep-merged with the parent's default record.

For a parent field `layout` with no default, omission fails even when every field inside the Layout schema has a default. Supplying `layout = ()` permits those inner defaults to apply. If `layout` itself has a default, omission uses that complete value.

`()` remains an empty record at the format level. It becomes a completed default-valued record only when its receiving schema permits every omission. The reader never invents schema fields in an untyped parse.

Encode ordinary present optional values directly and absence as `none`. Reject automatic mappings that would lose distinguishable nested optional states. Eligibility is checked before accepting an empty or absent instance, not inferred from that convenient value. Normal typed encoding emits all completed fields, including explicit `none` and values equal to defaults.

### 10. Eligibility and application rules

Derive eligibility transitively from the complete concrete schema. Every field, element, map entry and variant payload must have a supported mapping. An unsupported member makes the enclosing shape ineligible even when the current instance does not use it. Check concrete generic specialisations when compiler-derived schemas are later available.

Use no mandatory Moth serialisation trait or per-type enablement marker. Unsupported application types use an ordinary serialisable description and explicit conversion functions. V1 adds no rename/skip/flatten attributes, custom codec hooks, constructor execution or arbitrary user predicates inside decoding.

Functions, wires, routes, closures, live handles, unresolved resource pieces and deferred computations have no ordinary data encoding. A completed runtime-generated String is eligible. Moth fallible-return carriers do not become serialised control flow, and hidden compiler/runtime identity never becomes document data.

The codec checks structural and type constraints. Receiver-owned ordinary functions check game rules, asset existence, registered actions and cross-field invariants. Publish live state only after those checks succeed. A successful producer-side check never removes validation at a receiving boundary.

### 11. Deterministic writing without canonical-byte semantics

Write typed record fields in schema declaration order. Preserve collection and map order. Prefer named choice payloads and anonymous named records in generated output. Pretty and compact modes differ only in whitespace and use the same writer.

Given the same ordered value, schema, options and encoder version, output is deterministic. V1 makes no promise that all semantically equivalent documents have identical bytes. Later canonical encodings belong to an optional serialiser policy, not restrictions on valid MON semantics.

Formatting authored text is distinct from typed encoding. It cannot silently fill omitted fields, reorder maps, remove unchecked qualifiers or erase numeric categories. A comment-preserving formatter and canonical hashing/signing facility are not required by v1.

### 12. Resources and versioning

MON carries resource identifiers as ordinary strings. Resolve any compiler structural Resource/SiteRoot pieces before encoding their final characters. V1 performs no path discovery, URL generation, filesystem checks or resource loading. An unresolved compiler resource value fails conversion to concrete MON data. No string scan attempts to reconstruct dependencies.

The receiver supplies the schema. There is no mandatory schema header, reserved universal version field or automatic schema discovery. Applications may declare an ordinary `format_version` field and explicit migrations. MON grammar support is a parser contract, separate from an application's schema version. Closed schemas never silently fall back to another version or repair mismatches.

## Rust service contract

Expose a narrow compiler-library surface, preferably `moth::mon`, without making AST, HIR or the whole frontend public. Exact Rust names are fixed in Phase 0 after inspecting current owners.

| Operation | Contract |
|---|---|
| Encode document | Receive a record value and its prepared static schema. Return one complete ordinary UTF-8 MON String or a structured failure. |
| Encode nested value | Receive any eligible value and its schema. Return its complete literal text, including string quotes or container delimiters, through the same writer. |
| Decode document | Receive one complete document and its prepared record schema. Validate, apply defaults and return an owned schema-checked record or the first structured failure. |

Encoding a String always encodes string data. It never guesses that text resembles MON and should be inserted raw or decoded. Nested encoded fragments remain ordinary strings, with no trusted marker, hidden schema or deferred computation. Composition requires final document validation.

Decoded values can outlive and release the input text without a caller-retained backing buffer. Internal borrowing during parsing is allowed. The default public API has no borrowed document views and does not expose a partially built result.

The Rust boundary must be usable from another crate with only a Moth dependency and ordinary data conversion code. It needs no compiler command, source project, backend, JSON adapter or Bevy dependency. Availability is not an assertion that Moth itself has no transitive dependencies.

## Bounds, failures and ownership

Consume complete documents and fail fast. Incremental input, streams and multiple-document framing remain deferred.

Apply finite configurable budgets to input bytes, nesting, node/entry counts, numeric digits/materialisation, decoded data and encoded output. Check before the affected allocation or expansion. Include default expansion and repeated shared-value expansion. Decoder and result destruction must not overflow the native stack for accepted depth limits. Document the supported ceiling and do not provide an unsafe unlimited switch by accident.

Limits are receiver resource policy, not grammar dialects. A valid document may exceed a receiver's budget. Report that distinctly from syntax and schema errors. Choose documented defaults from the representative corpus during implementation. Use checked size arithmetic and the existing operational allocation-failure policy, without claiming recovery from process-level exhaustion that the allocator cannot provide.

Return one structured first error with a stable reason/code, original input byte range and field/element path where available. A missing field points to its containing record, not fabricated source text. Preserve both locations for useful duplicate diagnostics without collecting unrelated later failures. Render messages only at a caller's diagnostic boundary.

Use the current compiler diagnostic, source and infrastructure lanes. Define the narrow public Rust projection needed to make errors usable without exposing internal identity tables. Keep authored data failures, unsupported schemas, resource limits and internal invariant failures distinguishable. This service does not start the later compact-diagnostics migration or add a competing global error taxonomy.

Both operations publish only complete successful results. On failure, discard partial values and output buffers. If the internal writer supports caller-owned buffers, its public contract must either roll back to the original length or keep that facility private. The complete String-returning operations remain the initial guarantee.

## Ownership and implementation constraints

Start with `src/lib.rs`, `src/compiler_frontend/numeric_text/`, token/string decoding, the delivered argument-list owner, constant-value/type owners, source storage and the diagnostic boundary. Read adjacent callers and tests before adding code. These are locators, not APIs to preserve.

Use one shared delimiter/separator/named-entry traversal and existing numeric/escape recognition. Add a focused literal-data receiving policy that cannot invoke expression evaluation. Source expression semantics remain with AST. MON decoding does not build executable AST/HIR, resolve declarations or run constant folding just to reject expressions afterwards.

Where the current shared owner mixes syntax traversal with AST-specific state, separate the smallest genuinely shared responsibility and delete displaced duplicate paths. Do not create a generic parser framework, runtime callback registry, cloned token windows, repeated rescans or independent MON grammar implementation. Literal-only input is a distinct receiving contract, not a compatibility mode for old Moth syntax.

Keep one owned data representation and one prepared schema representation only where required by the public Rust use case. Validate into the final owned data where practical. Avoid a mandatory chain of syntax tree, generic value tree, validated value tree and host tree. A temporary parse representation needs a concrete consumer and lifetime justification.

Prefer contiguous storage, borrowed field views and direct matches. Avoid boxed per-scalar nodes, redundant wrapper layers, per-value hash maps for fixed schemas and caches without measurements. Reuse existing helpers only when they preserve the literal, numeric and diagnostic contracts. No second arithmetic evaluator, JSON/RON/KDL conversion or new dependency merely for convenience.

## Implementation phases

Each code-bearing phase includes focused coverage, removal of superseded paths, the required validation gate and the `AGENTS.md` Slice review. Commit accepted phases separately. Keep working notes and raw measurements under `tmp/`.

### Phase 0: Activation and a concrete consumer contract

1. Verify the syntax checkpoint and prerequisite cleanups, refresh authorities and record baseline validation and non-recording timings.
2. Trace public Rust exports, shared argument traversal, retained numeric text, string decoding, schema/type facts, source ownership and error rendering. Record the extend/move/delete decisions.
3. Write the smallest external-style Rust save example against proposed signatures: construct a static schema, encode a record, decode it, release the source String and convert the result to native save data. Include a choice, nested record, map, explicit `none`, defaults and an exact large numeric value.
4. Fix the public schema/value boundary, qualifier spelling, numeric category conversions and Unicode escape grammar. Publish representative accepted/rejected examples and a source-versus-MON availability table. These are narrow implementation design gates, not a request for a new schema framework.
5. Identify any source-syntax change that would be triggered accidentally by lexical reuse. Keep it outside this delivery and record its accepted future parity rule rather than silently enabling it.

Exit: a reviewed consumer-sized API and ownership map with no dependency on later source or backend work. All remaining lexical/API choices above have a written disposition before their code phase starts.

### Phase 1: Publish permanent contracts

1. Complete the documentation inventory below, publishing the accepted MON format and the Rust-only boundary before adding executable support.
2. Correct const-required/const-only wording where it describes serialised input. Keep const evaluation as a source producer responsibility.
3. Record the exact availability of every shared syntax addition and numeric materialisation. Keep future source features and static-builder status Deferred.
4. Keep the roadmap ordering and the end-of-roadmap integration follow-up consistent. Run the documentation release-build gate.

Exit: one durable format authority, one Rust API/ownership contract and truthful status. The plan is no longer the only place explaining MON.

### Phase 2: Shared literal reader and static schema validation

1. Extend or narrowly refactor the existing lexical and argument owners. Implement implicit-root framing, records, collections, `{=}` maps, literal scalars and struct/choice data.
2. Preserve numeric categories and original input positions. Reject expressions before evaluation. Keep no source/module lookup capability in the data reader.
3. Prepare and validate immutable schemas and defaults. Implement closed fields, argument routing, nominal qualifier checks, eligibility, exact-field defaults and strict conversions.
4. Decode into owned values and return the first structured failure. Enforce budgets during parsing, schema traversal and default expansion.
5. Add focused format/schema and diagnostic tests, plus regression tests for any touched shared source parser behaviour.

Exit: the complete document decoder satisfies the accepted subset through the intended public boundary, without source evaluation or partial results.

### Phase 3: Writers and data round trips

1. Implement the document writer and nested-value writer over the same literal encoding owner.
2. Encode quotes, escapes, exact numeric categories, explicit empty containers and named payload fields correctly. Validate programmatically supplied values and qualifiers rather than trusting string-shaped numeric data.
3. Apply deterministic field ordering, completed defaults and map order. Add pretty/compact output without a separate semantic path.
4. Enforce output limits and tree-only semantics. Do not add alias registries when the chosen owned tree representation cannot contain cycles.
5. Add semantic round trips, rejection round trips and numeric/Unicode boundary coverage. Test fragment composition followed by complete-document validation.

Exit: successful encoded output is valid MON under its schema, and supported data round-trips without identity or byte-canonical promises.

### Phase 4: Public Rust integration and bounded hardening

1. Export the narrow service from the `moth` library and finish executable Rustdoc/examples for static schema construction, save encoding, loading and errors. Reuse an existing public API test pattern where available.
2. Run an external-style consumer build that imports only public paths. Pin the tests so crate-private access cannot conceal a broken dependency API. Keep the example independent of the actual engine and Bevy.
3. Verify input-lifetime independence, schema reuse, first-error determinism, default expansion limits, malformed/deep inputs and output rollback guarantees. Exercise wide and deep documents separately.
4. Measure valid and early/late-invalid input, nested collections/maps, escape-heavy strings, numeric limits and save-sized documents. Record throughput, allocation/peak-memory evidence where available and growth behaviour. Compare shared-source benchmarks before and after changes.
5. Remove redundant conversions, copied grammar, unused extension hooks and duplicate tests identified by the review. Improve measured issues through existing owners rather than starting a general performance programme.

Exit: another Rust crate can use MON for a save round trip with no source compiler execution, and the documented limits protect both success and rejection paths.

### Phase 5: Validation and closeout

1. Recheck the full design matrix below, public examples and precise documentation inventory. Confirm no Moth directive, runtime opcode, CLI builder or `.mon` source-import integration slipped in.
2. Run `just validate`, the public Rust consumer test, relevant default-feature coverage and `moth build docs --release` or its documented Cargo equivalent. Use `just bench-check` for the focused non-recording comparison. Report failures and unavailable lanes exactly.
3. Update the core progress matrix to reflect the delivered Rust surface and actual coverage only. Leave Moth operations, source parity gaps and the static builder Deferred. Update source locators and mark affected audit entries stale without claiming a new audit.
4. Perform the final Slice review, including deletion, duplication, public API size, ownership, diagnostics, test ownership and dependency review.
5. Delete this completed plan and its roadmap entry in the completion commit. Keep the late integration note and permanent contracts. Do not commit an intermediate completed-plan state.

## Required acceptance coverage

Use one primary owner per behaviour. Pure codec cases belong beside the subsystem. Public-boundary smoke tests belong in the existing Rust integration harness, with an actual outside-crate visibility check. Changed Moth source behaviour belongs under `tests/cases/`, but this plan should introduce no new MON language operation to test there.

| Contract | Required evidence |
|---|---|
| Root framing | Implicit/explicit equivalence, empty/comments-only root, missing commas, trailing comma, scalar-root rejection and trailing second-root rejection |
| Literal-only reading | Reject names, arithmetic, foldable calls, casts, templates, projections and references inside every aggregate and payload without invoking evaluation |
| Data/container distinctions | Nested heterogeneous data with schema enforcement, `()`/`{}`/`{=}`, map-versus-collection rejection and order preservation |
| Fields and maps | Unknown/missing/duplicate fields, duplicate escaped keys, malformed key forms and typed key equality |
| Structs and choices | Qualified/unqualified success, mismatched qualifier, unknown variant, unit/payload distinction, positional/named routing and required payloads |
| Defaults/options | Required optional omission fails, explicit `none`, empty record defaults, exact missing-field defaults, no deep merge and ambiguous optional eligibility rejection |
| Numbers | Large exact integers/decimals, Int overflow, `3.0` to Int rejection, scale loss, exponent/separator grammar, finite Float round trip and signed zero |
| Unicode | Quotes, slashes, escapes, control characters, non-ASCII scalars, invalid scalar escapes and Char arity |
| Eligibility and trees | Unsupported nested member rejected even when absent/empty, shared data repeated, cyclic host data rejected if representable and no identity promise |
| Encoding | Nested value quoting, complete document validity, defaults emitted, deterministic ordering and pretty/compact semantic parity |
| Bounds and failures | Before-allocation checks, deep/wide inputs, default/output expansion, numeric work limits, deterministic first error and no partial public result |
| Rust consumer | Only public imports, reusable static schema, source String dropped after decode, native save conversion and no engine/JSON dependency |
| Resources/versioning | Ordinary strings stay strings, unresolved anchors rejected at conversion and no filesystem/schema/version auto-discovery |

A benchmark is not correctness coverage. Add bounded generated round-trip and malformed-input testing only through an existing suitable harness, with deterministic seeds and useful reduced failures. A new fuzzing platform or fixture framework is not a prerequisite.

## Precise documentation updates

Phase 0 resolves renamed locations from the activation tree and records the final inventory. The proposed new paths below are destinations, not claims that these pages already exist. Reuse the canonical MON route published by the syntax checkpoint where it already provides the same owner, and avoid competing references.

| Document or owner | Required change |
|---|---|
| `docs/src/docs/mon/mon-format.mtf` and its paired `mon-format-basic.mtf` | Publish the literal format, root framing, all value forms, validation/default rules, numeric fidelity, qualifiers, limits and format-versus-schema distinction. Basic shows a small save record and strict failures. State Rust-only availability prominently. |
| The canonical MON syntax reference delivered by the preceding checkpoint | Link the format authority. Explain that full source expressions and literal MON input share syntax but have different receiving contracts. Correct const-only wording without restricting ordinary source expressions. |
| `docs/src/docs/mon/@page.moth` and `overview.mtf`, or their delivered equivalents | Route the syntax and format pages with one owner each. Page files own presentation. The overview is a reference route, not page content. |
| `docs/src/developer-docs/language/overview.mtf` and `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf` | Add precise navigation and a compact source-syntax/data-format distinction. Do not present `$mon`, `{=}` source enforcement, Unicode escapes or contextual choices as implemented unless their source paths actually landed. |
| `docs/compiler-design-overview.md` | Document the narrow Rust MON service, shared lexical/argument owners, literal-only boundary, static schema ownership, error lane and absence of AST/HIR/TIR or module compilation in decoding. Update implementation locators. |
| `docs/compiler-data-layout-design.md` | Specify the MON input snapshot/cursor/span lifetime, retained numeric text, public error context and owned-result handoff using the active representation. Preserve Phase 3 invariants without starting later diagnostic storage work. |
| `docs/build-system-design.md` | State that the codec is a library service with no command, source kind, builder registration or output writes. Preserve the later static builder's normal output-ownership contract. Resource materialisation precedes encoding. |
| `docs/src/docs/collections/collection-literals.mtf` and `hash-maps.mtf` | Distinguish implemented source empty-container inference from accepted future `{=}`/strict `{}` source parity. Reference MON's delivered distinct forms without claiming the source migration happened. |
| `docs/src/docs/language-overview/strings-and-characters.mtf` | Document the shared Unicode escape contract and exact source availability. Keep MON literal strings separate from source templates. |
| `docs/src/docs/choices/variant-construction.mtf`, `choice-declarations.mtf` and `docs/src/docs/structs/construction-and-fields.mtf` | Explain literal payload/qualifier mapping by reference, preserve nominal identity and state contextual `::Variant` source availability. Keep choice payload defaults excluded. |
| `docs/src/docs/numbers/numeric-literals.mtf`, `numeric-types.mtf` and relevant cast references | State the difference between source contextual typing, lossless MON numeric text and strict target conversion. Track unavailable Number/Byte source/backend adapters honestly. |
| `docs/src/docs/errors/options.mtf` and `docs/src/docs/functions/parameters-and-defaults.mtf` | Cross-reference explicit `none`, omission only through a declared default and exact-field default application. Avoid changing unrelated source optional/default rules. |
| `docs/src/docs/templates/template-basics.mtf`, `docs/src/docs/constants/const-templates.mtf` and `docs/src/docs/resources/file-values.mtf` | Explain that evaluated strings are serialisable, raw template syntax is not MON and concrete resource text is required. No automatic MON interpolation or runtime producer export is delivered. |
| `docs/src/docs/design-scope/design-principles.mtf` and `deferred-and-outside-scope.mtf` | Record strict literal data, ordinary-type parity and small Rust tooling. Separate accepted late Moth integration from dynamic schema design and other open extensions. |
| `src/lib.rs` and the MON public module's Rustdoc, with an executable consumer example | Document the actual API, owned lifetimes, static schema preparation, limits, first errors, public paths and native Rust conversion boundary. No fictional derive or `$mon` examples. |
| `docs/src/docs/progress/@page.moth` | Separate Rust codec status/coverage from deferred Moth-native schemas, directives and runtime operations. Status follows delivered tests, not plan acceptance. |
| `docs/src/docs/progress/packages-and-builders/@page.moth` | Keep the MON static project builder Deferred with no command or package implementation implied by the Rust codec. |
| `docs/roadmap/roadmap.md` | Keep Rust v1 immediately after the syntax checkpoint and update the Phase 4 interlude wording. Keep Moth-native integration and the static builder at the end. Remove only this plan's entry at closeout. |
| Adjacent queued plans when their activation instructions name the interlude | Refresh prerequisite wording to include the accepted MON Rust tooling checkpoint. Refer to delivered capabilities or permanent authorities, not this temporary plan. Preserve their independent scope. |
| `index.md` and `docs/roadmap/audit-log.md` | Update real module/file locations after implementation. Mark materially affected audits stale without inventing audit coverage. |

Update paired teaching pages only where their text is affected. Search comments, Rustdoc, embedded examples, fixtures and scaffolds for conflicting MON terminology. Generated `docs/release/**` content is rebuilt through the compiler, never hand-edited.

## Deferred integration contract

The final roadmap follow-up owns compiler `$mon` convenience, exact invocation syntax, automatic static schemas from ordinary Moth types, checked anonymous-record generation, explicit source encode/decode operations, backend support and the static `.mon` asset builder. It also owns any still-undelivered source parity for `{=}`, Unicode escapes and expected-type `::Variant` construction. Those additions reuse this codec and the shared syntax owners rather than reimplementing MON.

The later static builder generates compile-time-known assets through normal build output ownership. Runtime template producers remain ordinary compiled functions returning Strings under their consuming builder's explicit lifecycle contract. No hidden template subscription, schema, wire or route is introduced.

Dynamic schema construction/loading, public borrowed views, streaming, binary formats, graph identity preservation, automatic migrations, custom serialisation traits, field-transformation frameworks and byte-canonical output remain outside v1. Add them only for a concrete accepted use case.
