# Moth language migration closeout and compiler semantic realignment

## Current state

```text
WORK_ID: docs-lang-mig
WORK_SOURCE: docs/roadmap/plans/docs-language-migration.md
BASE_REVISION: 820098759bc896365eb99f2239c2bd1a570132f1
STATUS: stage-c-review-ready - parity confirmed and authority switch ready for user review
CURRENT_SCOPE: Stage C writing, parity, authority and deletion changes are complete and intentionally uncommitted
STAGE_A: complete and accepted
STAGE_W: complete and accepted
PRE_B1_CLOSEOUT: complete, full gate green at d82b86d74
COMPLETED: B1 source syntax removal and JS raw-value return ABI correction; slot-backed result provenance and multi-return summary correction; B2 String semantics, HIR StringAppend, JS/Wasm content equality and map normalization; B3 pattern-surface removal; B4 template collision registry; B5 accepted language gaps; B6 Core Text and Core Math boundary corrections; B7 builder capability metadata
NEXT_ACTION: user reviews, corrects if needed and commits the uncommitted Stage C authority switch
VALIDATION: prior Stage B full gate remains recorded; Stage C cargo run --quiet -- check /tmp/docs-lang-mig-snippets.moth --terse passed; final cargo run --quiet -- build docs --release built 69 files successfully; generated raw Moth-link scan returned no matches; generated route pairing, tables, titles and links inspected; changed paths are documentation-only; git diff --check passed
AUDITS: prior Stage B final_auditor history remains accepted; Stage C final_auditor run 20260802T221557Z-5cc4649b found five documentation defects, all corrected; verification run 20260802T223752Z-92412a86 stopped on a launcher path-safety false positive with no workspace change; final_auditor run 20260802T225312Z-01793695 confirmed monolith parity, package-status alignment and all prior corrections, then found one Prelude ownership wording error; the wording was corrected and focused auditor run 20260802T230826Z-5e3b516c returned clean with no workspace changes
BLOCKERS: none
STAGE_C: user approved removal of docs/language-overview.md after parity confirmation; final changes must remain uncommitted for user review
```

## Completion record

Stage A (technical documentation closeout) and Stage W (writing-style pass) are complete and accepted. The bulk migration, focused-page corrections, semantic consistency cleanup, example repairs, status notes, link audits, parity ledger and style pass all landed. Git history and the parity ledger are the detailed evidence.

## Authority during Stages A, W and B

Use this order when sources disagree:

1. Explicit user decisions recorded in this plan
2. `docs/language-overview.md` as the maintained compiler-facing parity baseline, removed in Stage C
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
- The focused unsuffixed references replace the monolith in Stage C after the
  final parity review.
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

`@core/text.length` counts Unicode scalar values. HTML-JS uses scalar-value iteration to implement the contract; HTML-Wasm package lowering remains deferred.

### Core Math checked Float boundary

Every `@core/math` Float result must be finite before ordinary Moth code observes it. HTML-JS uses the shared external Float validation boundary; HTML-Wasm package lowering remains deferred.

### Implicit `.mtf` scope providers

The active builder declares which source-backed packages provide implicit `.mtf` scope, and generic orchestration injects those providers only for modules whose semantic source set contains `.mtf` files.

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

The documentation migration established backend-neutral contracts for Core Text and Core Math. Stage B now aligns the HTML-JS lowering with those contracts, while target-specific package lowerings remain tracked as explicit deferrals.

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
- update the owning canonical Advanced reference
- update the progress matrix
- update the parity ledger
- rebuild docs
- add focused integration coverage

These corrections belong in Stage B because their contracts were finalised by the language migration.

## B7. Move implicit `.mtf` scope providers to builder capability metadata

The former implicit `@html` provider fix used a hard-coded prefix list:

```rust
const IMPLICIT_TEMPLATE_SCOPE_PREFIXES: &[&str] = &["html"];
```

It also injected the provider into consumer modules more broadly than the semantic requirement. Stage B replaces that path with builder-declared capability metadata and semantic-source-set gating.

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

This task remains a named Stage B item and is implemented in the current closeout.

## Stage B completion record

The current Stage B slice completes the planned B2-B7 implementation work:

- B2 unifies source and runtime `String` semantics, including compiler-owned
  template append, content equality and HTML-JS map-key normalisation.
- B3 removes general full-match captures and `String` relational patterns while
  retaining option and declared choice payload captures.
- B4 registers Moth Template names through one collision-reporting registry.
- B5 closes option payload equality inside choices, cross-choice predicate
  diagnostics, nested error-only `return!`, block value-producing `if`, and
  stored named insert composition.
- B6 aligns HTML-JS text scalar counting and the shared finite-Float external
  boundary for Core Math.
- B7 moves implicit `.mtf` providers to builder capability metadata and gates
  them on semantic `.mtf` source presence.

HTML-Wasm Core Text and Core Math lowerings, non-JS String map lowering, full
relational overlap analysis and nested choice payload patterns remain explicit
target or language follow-ups recorded in the progress matrix and parity ledger.

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

The user approved the authority switch and selected removal of the monolith.

The uncommitted Stage C review patch may:

- update `AGENTS.md`
- declare focused references authoritative
- remove `docs/language-overview.md` after confirming complete focused parity

Stage C changes remain uncommitted so the user can review, correct and commit
the final authority switch.

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
