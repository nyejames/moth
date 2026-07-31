# Moth language migration closeout and compiler semantic realignment

## Current state

```text
STATUS: active
CURRENT_STAGE: Stage B compiler semantic realignment
STAGE_A: complete and accepted
STAGE_W: complete and accepted
PRE_B1_CLOSEOUT: complete, full gate green at d82b86d74
VALIDATION: full gate green at d82b86d74
NEXT_ACTION: B1 remove source-authored return aliases
STAGE_C: blocked until Stage B completes
```

## Completion record

Stage A (technical documentation closeout) and Stage W (writing-style pass) are complete and accepted. The bulk migration, focused-page corrections, semantic consistency cleanup, example repairs, status notes, link audits, parity ledger and style pass all landed. Git history and the parity ledger are the detailed evidence.

## Authority during this plan

Use this order when sources disagree:

1. Explicit user decisions recorded in this plan
2. `docs/language-overview.md` as the maintained compiler-facing parity baseline
3. `docs/src/docs/codebase/memory-management/**` for formal memory semantics
4. `docs/compiler-design-overview.md` for compiler stages and artefact contracts
5. `docs/build-system-design.md` for project, module, package and builder architecture
6. `docs/src/docs/progress/@page.moth` for current implementation support
7. Accepted roadmap plans for deferred implementation
8. Current compiler code and tests as evidence of behaviour
9. Existing public pages as teaching material under review

Implementation is not automatically language design. When current code conflicts with accepted design, document the accepted contract and record the implementation gap. Stage B then removes the drift.

## Documentation ownership

- Unsuffixed `.mtf` files own complete Advanced source syntax and observable semantics.
- Paired `-basic.mtf` files teach a smaller accurate beginner surface.
- `@page.moth` files own public composition, introductions, ordering and navigation.
- The monolith remains maintained until Stage C.
- Formal compiler, build-system and memory architecture stays in its dedicated owners.
- The progress matrix owns current implementation and backend status.
- The roadmap owns sequencing and genuinely deferred implementation.

Advanced files must remain directly readable and complete. Do not compress away source-observable legality, edge cases or rejected forms merely because formal architecture exists elsewhere.

Basic files must use current valid examples and must not present deferred syntax as available today.

---

## Locked semantic decisions

These decisions are final for this plan.

### Source-authored return aliases are removed

Function return slots contain types and channels only. Source signatures have no borrowed, owned, move or parameter-alias return categories.

The compiler infers freshness and alias summaries. Public interfaces may carry inferred summaries. External binding metadata may describe foreign return aliasing.

Stage B deletes source syntax and support that exists only for authored alias returns.

### `String` has one semantic surface

Quoted slices and template-produced strings share one semantic `String` type at typed boundaries.

Construction still differs:

- quoted strings create deliberately restricted read-only slices
- templates create owned strings and are the canonical concatenation and interpolation mechanism

Construction origin must not change equality, hashing, map-key legality, call compatibility or choice and option equality.

Source `String + String` is invalid. Use `[left, right]`.

`String` supports `is` and `is not`. It does not support ordering operators or relational match patterns.

### Full-match catch-all syntax

`else =>` is the only full-match catch-all.

A bare identifier is not a general capture pattern. Option `|name|` capture and declared choice payload captures remain valid.

### Moth Template implicit scope

Same-directory module-root constants and `@html` constants do not shadow. A same-name visible constant is a collision diagnostic.

### Assertions in examples

`assert` is for impossible invariant failure, not expected recovery.

A teaching example may use `assert(false, "message")` inside `catch` when the example setup makes failure impossible and a full recovery branch would obscure the topic.

### Module and import topology

Source imports resolve from the owning module root.

`@./...` has no supported meaning.

Normal sibling modules cannot import each other directly. Shared sibling APIs use scoped `+*.moth` support packages.

### Core Text length contract

`@core/text.length` counts Unicode scalar values. The current JS lowering uses UTF-16 code units and does not yet match. Stage B6 closes this gap.

### Core Math checked Float boundary

Every `@core/math` Float result must be finite before ordinary Moth code observes it. The current JS lowering does not enforce this. Stage B6 closes this gap.

### Implicit `.mtf` scope providers

The compiler hard-codes `IMPLICIT_TEMPLATE_SCOPE_PREFIXES` and broadly supplies source providers before `.mtf` use is known. Stage B7 moves this to builder capability metadata.

---

# Stage B: compiler semantic realignment

Stage B begins only after Stage W acceptance.

Each slice updates compiler code, tests, focused docs, the monolith and the progress matrix together. Moth is early Alpha. Remove obsolete paths completely and add no compatibility syntax or legacy diagnostics.

## B1. Remove source-authored return aliases

- delete parameter-name return parsing
- delete alias-candidate syntax variants and helpers
- simplify AST and HIR return representations
- remove source-declared return-alias arrays
- retain inferred return-alias summaries
- retain explicit foreign binding alias metadata
- reuse the existing deterministic summary stabilisation infrastructure
- treat recursive or unresolved cycles conservatively as unknown
- add no compatibility parser

Search-zero checks include obsolete source-syntax owners such as `AliasCandidates` and alias-return parser helpers. General inferred and external alias metadata must remain.

## B2. Unify compiler `String` semantics

- reject source `String + String`
- delete compile-time string-add folding
- retain internal template append lowering
- make equality accept every runtime `String`
- make choice and option equality recurse through `String` consistently
- make `String` map keys use uniform content equality and hashing
- remove value-shape checks that create hidden semantic types
- keep only value metadata required for template and reactive lowering
- update backend comments and validation

## B3. Simplify full-match patterns

- remove `String` relational patterns
- retain `Int`, `Float` and `Char`
- delete general capture AST and HIR variants
- delete capture scope, exhaustiveness and backend paths
- diagnose unknown choice variants directly
- keep option `|name|` and choice payload captures
- keep `else =>` as the only catch-all
- add no legacy capture diagnostic

## B4. Align Moth Template collisions

- register `@html` and same-directory root constants through one visible-name registry
- remove overwrite precedence
- preserve both source locations
- reject collisions before AST folding
- keep unique constants from both surfaces visible
- replace precedence tests with collision tests

## B5. Close accepted non-deferred gaps

Reproduce each item before changing it:

1. option payload equality inside choices
2. cross-choice inline predicate validation
3. nested-block `return!` in error-only functions
4. block value-producing `if` with `then`
5. stored named template inserts

Add focused unit, HIR and integration coverage as appropriate.

Any accepted semantic correction discovered during Stage A, including a Core Text length contract or checked external Float boundary, must be assigned here or to an explicitly approved dedicated plan before Stage C.

## B6. Align Core binding results with Moth semantics

The documentation migration established backend-neutral contracts for Core Text and Core Math. Their current JavaScript lowering disagrees.

### Core Text

Implement the accepted contract:

```text
@core/text.length counts Unicode scalar values
```

Required work:

- replace JavaScript UTF-16 code-unit counting
- use a correct scalar-value counting implementation
- cover empty strings
- cover ASCII
- cover BMP non-ASCII
- cover non-BMP scalar values such as emoji
- cover mixed strings
- cover direct and namespace imports
- keep the return type `Int`
- preserve infallible package semantics

Expected examples:

```text
length("") == 0
length("abc") == 3
length("é") == 1
length("🦋") == 1
length("a🦋b") == 3
```

### Core Math

Implement the accepted checked Float boundary:

- every `@core/math` Float result must be finite before ordinary Moth code observes it
- reject `NaN`
- reject positive infinity
- reject negative infinity
- use the existing external Float result-validation owner
- do not add ad hoc checks independently to every helper when one shared package-return boundary can own validation
- test direct imports
- test namespace imports
- test aliases
- test representative invalid results such as `sqrt(-1.0)` and overflow from `exp`
- preserve finite valid results

### Documentation and status

For each correction:

- remove its temporary Advanced implementation-gap note
- update `docs/language-overview.md`
- update the progress matrix
- update the parity ledger
- rebuild docs
- add focused integration coverage

These corrections belong in Stage B because their contracts were finalised by the language migration.

## B7. Move implicit `.mtf` scope providers to builder capability metadata

The current implicit `@html` provider fix uses a hard-coded prefix list:

```rust
const IMPLICIT_TEMPLATE_SCOPE_PREFIXES: &[&str] = &["html"];
```

It also injects the provider into consumer modules more broadly than the semantic requirement.

Required direction:

- the active builder or package capability surface declares which source-backed packages enter `.mtf` implicit scope
- remove `IMPLICIT_TEMPLATE_SCOPE_PREFIXES` from generic build orchestration
- only modules whose semantic source set contains `.mtf` sources receive those implicit provider interfaces
- normal `.moth` consumers do not receive implicit providers merely because the builder registered them
- explicit imports remain unchanged
- unrelated source packages never become implicitly visible
- `.mtf` still receives the accepted `@html` compile-time constant surface
- same-directory root constants and builder constants continue into the later collision-validation slice

Required tests:

- `.mtf` receives `@html`
- ordinary `.moth` without import does not receive `@html`
- an unrelated source-backed package is not implicitly visible
- explicit imports continue to work
- modules without `.mtf` do not receive the implicit provider
- production provider-interface path remains covered

This task may land before or after B1, but it must remain a named Stage B item.

## Stage B validation

For every code-bearing slice:

```sh
cargo fmt
just validate
cargo run --quiet -- build docs --release
```

Also verify:

- obsolete variants and adapters are gone
- `TypeId` remains semantic type authority
- inferred alias summaries remain side-table or interface facts
- borrow analysis does not rewrite HIR
- internal template append is not source string addition
- backends do not reinterpret removed source syntax
- user failures remain structured diagnostics
- the progress matrix matches current support

---

# Stage C: final parity and authority switch

After Stage B:

1. Re-run the parity ledger against the monolith and delegated authorities.
2. Audit every Advanced file as a direct reference.
3. Audit every Basic file for truthful beginner teaching.
4. Inspect every public route.
5. Confirm deferred and outside-scope ownership.
6. Confirm the progress matrix matches the compiler.
7. Present every remaining ambiguity or mismatch to the user.

The authority switch requires explicit user approval.

Only the separate approved switch patch may:

- update `AGENTS.md`
- declare focused references authoritative
- decide whether `docs/language-overview.md` remains a consolidated legacy reference, becomes an index or is removed

---

# Required report for every slice

Report:

## Scope

- stage and workstream
- starting commit and branch
- authorities read

## Changes

- source documentation changed
- generated documentation changed
- code and tests changed when Stage B applies
- moved or deleted owners

## Semantic result

- accepted rule
- valid forms
- invalid forms
- current implementation status
- deferred dependencies

## Parity

- monolith sections reviewed
- Advanced owner
- Basic owner
- examples preserved or replaced
- parity-ledger rows updated

## Validation

- exact commands
- exact results
- routes inspected
- generated diff inspected

## Remaining uncertainty

- unresolved design decisions
- implementation conflicts
- deferred dependencies
- uninspected routes
- incomplete parity rows

Do not claim a command, build or inspection that was not performed.
