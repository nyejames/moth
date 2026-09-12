# Core text package plan

## Role and authority

`@core/text` is the portable Core package for deterministic `String` inspection, exact search and
non-mutating text transformation.

Canonical source semantics belong in `docs/src/docs/packages/core/text/` and the canonical `String`,
`Char`, Error and Option references. This living plan records implementation strategy, sequencing,
performance knowledge, blockers and later extensions. Implementation code never becomes the semantic
authority.

The package remains Core origin with ExternalBinding backing and has no prelude alias. It exposes free
functions until a separate language decision changes binding-backed receiver methods.

### Current-state capsule

```text
STATUS: v1 designed and queued; bounded pre-checkpoint hardening of the existing five functions in progress
CURRENT_SLICE: strengthen existing-behaviour coverage for the five shipped functions and replace `Array.from` counting in `__moth_text_length`
BLOCKERS: the v1 expansion still waits for the compiler-owned native result-slot and Core const-eval prerequisite
NEXT_ACTION: finish the hardening slice below, then after the prerequisite lands audit its final Text evaluator owner and run this plan's Phase 0 from current main
```

The package-foundation baseline this plan waited on is merged, so the compiler prerequisite is the
only remaining blocker owned here. The umbrella programme owns cross-cutting blockers for every
code-bearing phase, including the current red `just validate` clippy lane.

The hardening slice is deliberately narrow. It may strengthen existing-behaviour tests for the five
shipped functions and remove the `length` helper's temporary array, and it must leave registration
shape, inline-lowering conversion, error codes, new APIs, result slots and constant evaluation to the
phases below.

The prerequisite compiler work establishes truthful zero/one/many result slots, adds
`ExternalConstEvalOp`, adds one AST-owned Core constant-evaluation path and proves it with the existing
five Text functions. This plan extends those delivered owners. It must not add another evaluator,
callback registry, package-name dispatch path or Text-specific `ExternalFunctionId` family just for
constant evaluation.

The planning-branch description of that prerequisite currently lives at
`docs/roadmap/plans/native-result-slots-and-core-const-eval.md`. That ordinary plan is expected to be
retired after implementation. At package activation, use the merged implementation and current
canonical docs rather than preserving shapes only because that plan once named them.

## Current surface

The implemented surface is:

| Function | Result | Contract |
|---|---|---|
| `length(text)` | `Int` | Unicode scalar-value count |
| `is_empty(text)` | `Bool` | exact emptiness |
| `contains(text, substring)` | `Bool` | exact case-sensitive substring test |
| `starts_with(text, prefix)` | `Bool` | exact case-sensitive prefix test |
| `ends_with(text, suffix)` | `Bool` | exact case-sensitive suffix test |

All parameters use shared access. Scalar and Bool results are fresh values. The package is explicit
rather than preluded.

Current implementation debt relevant to this slice:

- the other four functions are one-line JS runtime wrappers that can use the existing inline external
  lowering path instead
- `src/builder_surface/core_packages/text.rs` uses a homogeneous tuple table and repeated parameter
  cloning that will become noisy once signatures, error channels and const-eval metadata diverge
- the existing Text integration fixtures assert those wrapper helper names, coupling tests to an
  implementation shape that should disappear

## Implementation notes

### V1 target surface

Add this bounded v1 expansion:

| Function | Result | Contract |
|---|---|---|
| `char_at(text, index)` | `Char, Error!` | scalar at a zero-based scalar index |
| `find(text, substring)` | `Int?` | first matching scalar index or `none` |
| `find_last(text, substring)` | `Int?` | last matching scalar index or `none` |
| `count(text, substring)` | `Int` | number of non-overlapping exact matches |
| `slice(text, start, end)` | `String, Error!` | scalar-indexed half-open range `[start, end)` |
| `trim(text)` | `String` | remove Moth whitespace from both ends |
| `trim_start(text)` | `String` | remove leading Moth whitespace |
| `trim_end(text)` | `String` | remove trailing Moth whitespace |
| `replace_all(text, substring, replacement)` | `String` | literal non-overlapping replacement |

All positions use zero-based Unicode scalar indices, the same unit as `Char` and `length`. No public
operation exposes a JavaScript UTF-16 code-unit index or a Rust UTF-8 byte index.

All String parameters use shared access. `slice`, trim functions and `replace_all` return fresh String
results and create no in-place String mutation surface.

### Target scope

This package slice implements the existing JavaScript runtime path plus compiler-owned Rust constant
evaluation. It does not add a Wasm runtime lowering.

Keep `wasm: None` for new Text functions while the Core Text Wasm path remains unsupported. Existing
target validation must continue rejecting reachable runtime Text calls on unsupported targets. A
fully folded Core Text call leaves no HIR/runtime edge, so it may compile for a target with no Text
runtime lowering under the prerequisite's established const-eval rule.

If a complete Core Text Wasm lowering has independently landed before activation, adopt that current
owner rather than deleting it. Do not broaden this package phase into Wasm implementation work.

### Index and error contract

`char_at` and `slice` are fallible because an invalid index or range is invalid input, not ordinary
absence.

Use the existing compiler-owned builtin error system. Reserve these currently free codes in
`src/compiler_frontend/builtins/error_codes.rs`:

```text
TextIndexOutOfBounds = 120
TextInvalidSliceRange = 121
```

Default messages:

```text
TextIndexOutOfBounds  -> "Text index out of bounds"
TextInvalidSliceRange -> "Text slice start exceeds end"
```

Rules:

- `char_at` returns `TextIndexOutOfBounds` for `index < 0` or `index >= length(text)`
- `slice` returns `TextIndexOutOfBounds` for a negative bound or a bound greater than `length(text)`
- `slice` returns `TextInvalidSliceRange` when `start > end`
- a valid empty slice returns an empty `String`

JavaScript uses the existing `__moth_error_result` carrier and `BuiltinErrorCode` message/code owner.
Do not build a Text-specific result envelope. Rust semantic helpers should represent these failures
with the same `BuiltinErrorCode` values so a later fallible Core evaluator can reuse the functions
without translating a second error vocabulary.

### Exact search and empty-pattern rules

Search is literal, case-sensitive and normalization-sensitive. Canonically equivalent but differently
encoded Unicode text is not silently normalized.

The empty substring has one scalar-boundary meaning:

- `contains(text, "")` is `true`
- `starts_with(text, "")` is `true`
- `ends_with(text, "")` is `true`
- `find(text, "")` is `0`
- `find_last(text, "")` is `length(text)`
- `count(text, "")` is `length(text) + 1`
- `replace_all(text, "", replacement)` inserts the replacement at every scalar boundary, including
  before the first scalar and after the last

`count` and `replace_all` use non-overlapping left-to-right matches for non-empty patterns.

Follow the existing String-to-Int representability contract. This package must not invent a separate
large-string overflow channel only for `count` or search positions.

### Moth whitespace contract

V1 trimming owns the fixed 25-scalar Unicode 17.0 `White_Space` set:

```text
U+0009..U+000D
U+0020
U+0085
U+00A0
U+1680
U+2000..U+200A
U+2028
U+2029
U+202F
U+205F
U+3000
```

The versioned source is `https://www.unicode.org/Public/17.0.0/ucd/PropList.txt`. Do not use the
moving `/latest/` path as a semantic reference.

This set is Moth-owned package data. JavaScript must not define it through `String.prototype.trim`, a
Unicode property regular expression or the host engine's Unicode database. Rust must not define it
through a version-sensitive host predicate such as `char::is_whitespace()`.

Keep the tiny predicate independently implemented in Rust and JS rather than adding a generated-data
framework for 25 scalars. Exhaustive parity tests over the owned ranges and nearby non-whitespace
scalars are the guard against drift.

### Current repo touch map and cleanup boundary

| Owner | Work |
|---|---|
| `src/builder_surface/core_packages/text.rs` | register final signatures, JS lowerings and delivered `const_eval` metadata |
| delivered Core const-eval Text owner under `src/compiler_frontend/ast/const_eval/` or its replacement | add pure Rust Text operations and dispatch existing `ExternalConstEvalOp` values |
| `src/backends/js/package_bindings/core/text.rs` | keep only non-trivial runtime helpers plus package-private scalar/whitespace helpers |
| `src/backends/js/package_bindings/core/mod.rs` | retain current Core helper inventory and first-party validation integration without adding a general helper dependency graph |
| `src/backends/js/runtime/strings.rs` | reuse `__moth_string_value` only, do not move optional `@core/text` algorithms into the always-emitted general String runtime |
| `src/compiler_frontend/builtins/error_codes.rs` | own stable Text runtime error codes |
| `tests/cases/core_text_*` and owning external-package/backend unit tests | consolidate contract, parity, reachability and diagnostic coverage |
| `docs/src/docs/packages/core/text/` and packages/builders progress | publish implemented semantics and retain deferred blockers |

Do not broaden `ExternalPackageRegistry`, HIR call representation or the general external ABI in this
package slice. The prerequisite owns result-slot and const-eval infrastructure changes.

### Registration shape

The current tuple table is appropriate only while all Text functions have the same shape. Replace it
once the package expands.

Prefer one package-local registration helper plus a small `TextFunctionSpec` if the delivered
`ExternalFunctionSpec` still requires repeated construction. The local spec should carry the real
external signature, JS lowering, error type and optional `ExternalConstEvalOp`. It must not invent a
second parameter or return type system.

Use tiny constructors such as `shared_text()` where they reduce repeated `ExternalParameter` cloning.
Do not create a cross-Core-package registration abstraction solely because Time or Math also builds
external specs.

### JavaScript lowering strategy

Every runtime helper first resolves each String argument through `__moth_string_value` exactly once
and reuses the local concrete string. This avoids repeated reactive-template snapshots and repeated
boundary work.

#### Remove trivial runtime wrappers

Change these existing functions to `ExternalJsLowering::InlineExpression` with these direct shapes:

```text
is_empty    -> (__moth_string_value(#0).length === 0)
contains    -> __moth_string_value(#0).includes(__moth_string_value(#1))
starts_with -> __moth_string_value(#0).startsWith(__moth_string_value(#1))
ends_with   -> __moth_string_value(#0).endsWith(__moth_string_value(#1))
```

Each placeholder appears exactly once, matching the current inline-expression substitution contract.
This removes four emitted helper functions and their implementation-shaped artifact assertions while
retaining the canonical String snapshot boundary.

Keep `length` and the new non-trivial operations as runtime helpers where loops, option carriers or
error carriers make an inline expression less readable.

#### Scalar scanning

Use a tight UTF-16 loop over `charCodeAt`. A leading surrogate followed by a trailing surrogate
advances by two code units and contributes one scalar. Every other valid Moth scalar advances by one
code unit.

Use one package-private helper named `__moth_text_scalar_count_prefix` when scalar-prefix counting is
shared by `length`, `find`, `find_last` and the empty-pattern `count` path. Keep `char_at` and `slice`
as specialised scans because `slice` should find both boundaries in one pass rather than call a
generic scalar-to-code-unit converter twice.

For `length`, do not use `Array.from`, spread or an iterator materialisation. For `char_at`, stop when
the requested scalar is reached. For `slice`, scan only through `end` and remember the `start` offset
on the way.

#### Exact native search

Use JavaScript's native exact search for matching:

- `includes` for `contains`
- `startsWith` for `starts_with`
- `endsWith` for `ends_with`
- `indexOf` for `find` and repeated forward search in `count`
- `lastIndexOf` for `find_last`

For valid Moth Unicode strings, exact UTF-16 search preserves exact scalar-sequence equality. Convert
only a returned code-unit position into the Moth scalar index by counting the prefix once.

Do not reimplement general substring search in generated JS.

`find` and `find_last` return the existing canonical Option carrier shape already used by
binding-backed optional results. Do not add a Text-specific option helper or sentinel value.

#### Trimming

All v1 trim whitespace is in the BMP. Use two inward `charCodeAt` scans and one package-private helper
named `__moth_text_is_whitespace` over the owned ranges. Return the original JS string when no
boundary moves and use `slice` only when trimming actually changes a boundary.

#### Literal replacement

For a non-empty search string, prefer native `replaceAll` with a functional replacement so Moth
replacement contents remain literal. JavaScript replacement-string sequences such as `$&`, `$$`,
`$`` and `$'` must never acquire special meaning.

Native `replaceAll("", replacement)` inserts at UTF-16 code-unit boundaries and can split a non-BMP
Moth scalar. Special-case the empty pattern and append one scalar at a time with the replacement
between scalar boundaries. Do not use `split("")`.

#### Package-private helper emission

Keep the existing `CoreJsHelper` inventory so `just first-party-deps` can inspect every emitted JS
body. Keep `__moth_text_scalar_count_prefix` and `__moth_text_is_whitespace` in a Text-owned static
helper inventory, but emit each manually from `emit_core_text_helpers()` only when one of its public
runtime-helper users is referenced.

Do not add dependency metadata or a generic helper graph to `CoreJsHelper`. Two explicit Text-local
`needs_*` checks are smaller and clearer.

### Rust semantic and constant-evaluation strategy

The canonical package contract defines behaviour. The Rust implementation delivered for Core const
evaluation is the compiler-owned compile-time implementation, not a second semantic authority.
Runtime JS and future native/Wasm implementations must agree with contract-derived vectors.

At activation, deepen the Text owner delivered by the prerequisite. If its first five operations are
still embedded in one broad dispatch match, extract a focused Text submodule inside the same const-eval
subsystem before adding the larger v1 surface. Do not create a parallel Core runtime library or a
second external-operation registry.

Prefer representation-native Rust primitives when they preserve the contract:

- `str::chars().count()` for scalar length
- `str::is_empty()` for emptiness
- `str::contains`, `starts_with` and `ends_with` for exact predicates
- `str::find` and `rfind` followed by scalar counting of the matched prefix
- `str::match_indices` or equivalent non-overlapping string-pattern iteration for `count`
- one `char_indices()` pass for scalar-indexed character access and slicing
- `str::replace` for literal replacement when its empty-pattern behaviour remains covered by the
  shared scalar-boundary vectors

Modern Rust already optimises `chars().count()`. Do not replace it with a custom UTF-8 scalar counter
without measured evidence.

Use the same explicit Moth whitespace set for Rust trim operations. Fallible Rust helpers should
return their successful value or the canonical Text `BuiltinErrorCode` so future fallible const eval
can reuse them directly.

All Rust operations use whatever evaluation bound the prerequisite delivers, plus the existing
concrete-text requirement and current Int range. Structural resource/site-root strings remain
unavailable for character inspection until the existing fold owner can produce concrete text.

### Constant-evaluation registration

After the prerequisite lands, extend its closed typed Text operation vocabulary only for live and
eligible functions.

Expected new v1 operations eligible under the initial no-error-channel restriction are:

- `find`
- `find_last`
- `count`
- `trim`
- `trim_start`
- `trim_end`
- `replace_all`

`char_at` and `slice` keep their correct fallible APIs. Implement their Rust semantic functions and
parity tests now, but leave their `const_eval` registration absent until the compiler has an accepted
fallible Core evaluation contract.

No package helper executes JavaScript during compilation. No external annotation opts into a Rust
operation. The evaluator dispatches through the delivered `ExternalConstEvalOp` metadata on the
resolved definition rather than by package text, helper name or dynamically assigned function ID.

### Performance evidence

Implementation inspiration only, not semantic authority:

- V8 string iteration handles surrogate pairs while advancing through UTF-16 strings
- V8 owns specialised native substring search, supporting the decision to keep exact matching native
- V8 `replaceAll` uses an integrated search/output path rather than materialising match positions
- Rust `str` exposes representation-native `chars`, `char_indices`, pattern search and replacement
- Unicode 17.0 `PropList.txt` is the versioned source for the fixed whitespace set

Exploratory Node 22.16 microbenchmarks during design found manual surrogate scans materially faster
than `Array.from` for repeated scalar counting, one-pass scalar slicing faster than repeated offset
conversion and direct trim scans faster than regex on the tested workloads. Re-run focused benchmarks
on the implementation baseline. Prefer the simpler form when differences are noise and never add
engine-specific branches from one microbenchmark.

## Design rationale worth preserving

- One scalar position unit keeps `length`, `Char`, indexing, search and slicing coherent.
- Exact operations never normalize, case-fold or apply locale rules implicitly.
- Text transformations return new String values and do not add substring mutation.
- `__moth_string_value` remains the single JS content materialisation boundary for slices, templates
  and reactive template values.
- Runtime representation details stay private to each backend.
- Optional Core package logic stays out of the always-emitted general JS String runtime.

## Current work

Implementation starts only after the native result-slot and Core const-evaluation prerequisite has
landed and this worktree has adopted it.

### Phase 0 - refresh owners and freeze contracts

- [ ] Re-read canonical String, Char, Error, Option and package references from current `main`.
- [ ] Audit the delivered `ExternalConstEvalOp`, Text Rust evaluator and final result-slot owners.
- [ ] Audit final fallible external-call, optional-return and JS helper ABI shapes.
- [ ] Re-run focused JS implementation benchmarks and retain only material conclusions.
- [ ] Freeze new signatures, error codes, empty-pattern rules and the Unicode 17.0 whitespace set in
      the canonical Text reference before implementation.
- [ ] Confirm the touch map above against current paths and remove any assumptions made obsolete by
      the prerequisite.

Exit: API, errors, evaluator ownership and target lowering owners are explicit against merged `main`.

### Phase 1 - registration cleanup and scalar primitives

- [ ] Replace the homogeneous registration tuple table with the smallest package-local spec/helper
      shape justified by the expanded signatures.
- [ ] Convert `is_empty`, `contains`, `starts_with` and `ends_with` to inline JS lowerings and delete
      their runtime helper bodies.
- [x] Replace `Array.from(...).length` with allocation-free scalar counting (delivered by the pre-checkpoint hardening slice).
- [ ] Add the canonical Text builtin error codes and reuse `__moth_error_result`.
- [ ] Add `char_at` and `slice` with strict fallible bounds and one-pass scalar-aware JS lowering.
- [ ] Add or reuse the Rust scalar implementations and shared contract vectors.
- [ ] Keep `char_at` and `slice` out of const-eval registration while error channels are unsupported.
- [ ] Verify private scalar helpers emit only when a reachable Text runtime helper needs them.

Exit: current wrapper debt is removed and scalar-aware measurement, inspection and extraction are
implemented without new shared infrastructure.

### Phase 2 - exact location and counting

- [ ] Add `find`, `find_last` and `count`.
- [ ] Use native JS search and convert only returned positions into scalar indices.
- [ ] Reuse the canonical Option carrier for search absence.
- [ ] Implement Rust equivalents through the delivered Text evaluator owner.
- [ ] Register eligible typed Core const-eval operations.
- [ ] Cover non-BMP prefixes, combining sequences, empty patterns, absent matches and repeated
      non-overlapping matches.

Exit: runtime and constant paths share one exact scalar-position contract.

### Phase 3 - trimming and literal replacement

- [ ] Add `trim`, `trim_start` and `trim_end` with the explicit Moth whitespace set in Rust and JS.
- [ ] Add `replace_all` with literal replacement and scalar-boundary empty-pattern handling.
- [ ] Register all eligible new transformations for Core const evaluation.
- [ ] Verify `$` replacement syntax remains literal and non-BMP empty-pattern replacement never
      splits a surrogate pair.
- [ ] Keep any Text-private JS helper dependency checks local to the Text emitter.

Exit: common immutable transformations are complete without host Unicode drift or generic helper
infrastructure.

### Phase 4 - closeout and test pruning

- [ ] Make `core_text_edge_cases` the primary rich runtime/Unicode contract case unless current test
      ownership has materially changed.
- [ ] Repurpose `core_text_functions` as a focused reachability/const-elimination case or delete it if
      the rich case now owns the same runtime contract.
- [ ] Remove artifact assertions that require the four deleted trivial wrapper functions.
- [ ] Keep artifact assertions only where helper reachability, folded-helper absence or ABI structure
      is the behaviour under test.
- [ ] Compare Rust and JS against contract-derived shared vectors rather than treating either
      implementation as the semantic oracle.
- [ ] Verify eligible fully folded Text calls need no runtime Text lowering on an otherwise unsupported
      target, while dynamic calls still receive the existing target diagnostic.
- [ ] Update `text.mtf`, `text-basic.mtf` and the packages/builders progress row.
- [ ] Run the package programme phase gate and final code-quality audit.
- [ ] Compress completed checklist detail after closeout while preserving durable implementation
      rationale and blockers.

Exit: `@core/text` is useful for common scalar-aware inspection, exact search and immutable
transformation with lean JS output and Rust const-eval parity where eligible.

## Known gaps and next extensions

### Collection-valued text operations

Accepted future operations include `split`, `join`, `lines` and an eventual character iteration
surface.

Blocker: stable collection-valued binding signatures or a final source-backed implementation route.
Do not expose opaque JS arrays, hidden iterator handles or temporary callback APIs.

### Fallible constant evaluation

`char_at` and `slice` have Rust semantic implementations in this slice but cannot participate in the
initial Core evaluator because it excludes error channels.

Blocker: an accepted compiler-owned fallible Core const-evaluation contract preserving normal Moth
error semantics.

### Unicode case conversion, folding and normalization

Useful future work includes lowercase, uppercase, explicit Unicode case folding and NFC/NFD/NFKC/NFKD
normalization.

Blocker: one pinned Moth-owned Unicode data/version policy plus reviewed generated first-party tables
shared across implementations. Host JavaScript or Rust Unicode versions do not define Core semantics.
Normalization remains explicit and is never inserted into equality or exact search.

### Grapheme-aware operations

Future grapheme length, iteration and slicing may be useful for UI-facing text.

Blocker: an accepted grapheme API and pinned Unicode segmentation data. Grapheme operations remain
separate from scalar-based `length`, `char_at`, search positions and `slice`.

## Longer-term candidates

- `repeat`
- `pad_start` and `pad_end`
- explicit prefix/suffix removal helpers after real use shows they reduce boilerplate
- richer pattern search after exact search is proven insufficient

Regular expressions should receive their own design and likely their own package rather than turning
`@core/text` into a regex wrapper.

Locale-sensitive collation, locale-specific casing, word segmentation and linguistic formatting
belong to a future internationalization design rather than deterministic Core Text.

## Previous blockers and rejected approaches

- Reject UTF-16 indices because they are a JavaScript representation detail.
- Reject grapheme semantics for ordinary `length` or `slice` because graphemes are a separate unit.
- Reject `Array.from` or spread on scalar hot paths because they allocate temporary collections.
- Reject reimplementing general substring search in JS when native exact search preserves semantics.
- Reject JS `trim()` and host Unicode predicates as the whitespace authority.
- Reject regex-based literal replacement and trimming for v1.
- Reject native JS empty-pattern `replaceAll` because it inserts at UTF-16 code-unit boundaries.
- Reject changing fallible APIs to `Option` only to satisfy current const-eval eligibility.
- Reject opaque collection handles for collection-valued Text APIs.
- Reject JS execution during compile-time folding.
- Reject a second Rust evaluator registry, Text-specific function-ID family or generic JS helper graph.

## Validation and integration coverage

Use contract-derived vectors across Rust evaluation and JS runtime paths. Expected values come from
Moth semantics, not by comparing one implementation against the other.

Required edge families:

- ASCII, BMP, non-BMP and combining-scalar length
- scalar positions before and after non-BMP values
- `char_at` first/last success plus negative and past-end failure with stable error codes
- `slice` empty/full/interior ranges plus out-of-bounds and reversed-range errors
- found, absent, first and last exact search
- overlapping candidate strings with non-overlapping `count`
- every empty-pattern rule
- all 25 owned whitespace scalars plus nearby non-whitespace scalars
- replacement with no matches, repeated matches, empty search, non-BMP text and replacement values
  containing `$`, `$&`, `$$`, `$`` and `$'`
- decomposed versus precomposed text proving no implicit normalization
- folded versus demonstrably runtime parity for every const-evaluable operation
- structural resource/site-root strings refusing compile-time character inspection through the
  existing concrete-text rule
- canonical Option and Error carrier use for `find` and fallible scalar operations
- helper reachability: folded-only calls remove runtime helpers and dynamic calls retain only the
  helpers they need
- inline-lowered predicates producing no `__moth_text_is_empty`, `__moth_text_contains`,
  `__moth_text_starts_with` or `__moth_text_ends_with` helper bodies
- unsupported-target parity: folded eligible calls disappear before target validation while dynamic
  Text calls remain rejected when no runtime lowering exists

Prefer one rich package scenario over one fixture per helper. Keep separate focused negative cases
only where the diagnostic or target boundary is the primary behaviour.

At every code-bearing phase run the package programme gate, including:

```bash
cargo fmt --all
cargo run --quiet -- tests --audit
just first-party-deps
just validate
git diff --check
```

Also run focused Text runtime cases and the Rust/JS constant-parity cases introduced by the native
Core evaluator owner.

## History

### V0 - minimal Text surface

The package initially shipped `length`, `is_empty`, `contains`, `starts_with` and `ends_with` on the
HTML-JS path. `length` established Unicode scalar values as the package's character-count unit.
