# Moth language migration closeout and compiler semantic realignment

## Current state

```text
STATUS: active
CURRENT_STAGE: Stage A technical documentation closeout
LAST_REVIEWED_DOCS_COMMIT: 604eb03c3b9b0ece7189990742109aec83934ec0
NEXT_ACTION: repair the docs source module graph, then complete the remaining correctness audit
STAGE_A_BLOCKER: docs release builds currently stop at the invalid docs style module layout
STAGE_B: blocked until Stage A and the later writing-style pass are reviewed and accepted
STAGE_C: blocked until compiler semantic realignment is complete
```

Commit `604eb03c3b9b0ece7189990742109aec83934ec0` completed most of the bulk documentation migration. It added the missing Advanced memory detail, Project Structure concept pairs, Core package Basic and Advanced pages, Design Scope coverage and progress-matrix drift notes.

The remaining work is a closeout, not another bulk migration. It must correct the docs source graph, remove the remaining semantic contradictions, repair invalid examples, finish exact package and scope contracts, build the site and prove parity.

Historical migration phases remain in Git history. Do not append implementation diaries or repeat completed work in this plan.

## Required order

Work proceeds in this order:

1. **Stage A: technical documentation closeout**
2. **Stage A review and acceptance**
3. **Stage W: focused writing-style pass**
4. **Stage W review and acceptance**
5. **Stage B: compiler semantic realignment**
6. **Stage C: final parity review and authority switch**

Do not start Stage W while technical correctness remains open.

Do not start Stage B until the user has accepted both Stage A and Stage W.

Do not update `AGENTS.md` or declare the focused references authoritative before Stage C receives explicit approval.

---

## Authority during this plan

Use this order when sources disagree:

1. Explicit user decisions recorded in this plan
2. `docs/language-overview.md` as the maintained compiler-facing parity baseline
3. `docs/src/docs/codebase/memory-management/**` for formal memory semantics
4. `docs/compiler-design-overview.md` for compiler stages and artefact contracts
5. `docs/build-system-design.md` for project, module, package and builder architecture
6. `docs/src/docs/progress/#page.moth` for current implementation support
7. Accepted roadmap plans for deferred implementation
8. Current compiler code and tests as evidence of behaviour
9. Existing public pages as teaching material under review

Implementation is not automatically language design. When current code conflicts with accepted design, document the accepted contract and record the implementation gap. Stage B then removes the drift.

## Documentation ownership

- Unsuffixed `.mtf` files own complete Advanced source syntax and observable semantics.
- Paired `-basic.mtf` files teach a smaller accurate beginner surface.
- `#page.moth` files own public composition, introductions, ordering and navigation.
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

---

# Stage A: technical documentation closeout

Stage A is documentation-only. It may change files under `docs/**`, generated docs and documentation indexes. It must not change Rust, tests, fixtures, manifests, scripts or compiler behaviour.

Avoid broad stylistic rewriting during Stage A. Change prose only where correctness, completeness or immediate clarity requires it. The dedicated writing pass comes later.

## A1. Repair the docs source module graph

This is the first task because it blocks every docs check and release build.

### Convert the styles directory to a support package

Do not convert `docs/src/styles/docs.moth` into a normal `#docs.moth` module. That would make `styles` a sibling normal module and preserve the invalid sibling-import topology.

Replace it with a scoped support package:

```text
docs/src/styles/+package.moth
```

Required shape:

- keep private imports outside `export:`
- retain private helper declarations outside `export:` when consumers do not need them
- place the actual shared style API inside one strict `export:` block
- export only declarations used by documentation consumers or intentionally re-exported by `docs/src/#page.moth`
- preserve `Palette`, shared themes, layout components, documentation-level controls and pagers where they remain public API
- remove unused imports and declarations discovered by the move
- do not add a compatibility module or forwarding file

Update imports across `docs/src/**`:

```moth
import @styles { ... }
```

Remove all `@styles/docs` imports.

Update the grouped re-export in `docs/src/#page.moth` to use `@styles`.

Update `index.md` for the moved source owner.

### Canonicalise docs source imports

Audit every Moth import under `docs/src/**`.

Replace route-local forms such as:

```moth
import @./io
import @./build-inputs
```

with the correct owning-module-root-relative import identity.

Required outcomes:

- zero supported docs imports use `@./...`
- no import uses parent traversal
- child module and support package boundaries are not bypassed
- route-local `.mtf` sources resolve through their owning root
- source imports are extensionless
- generated public routes continue to import only exported provider surfaces

### Unblock and iterate the compiler check

After the graph conversion, run:

```sh
cargo run --quiet -- check docs --terse
```

Fix every newly exposed docs graph, import, visibility, API-only root or semantic error before moving on.

Do not describe `styles/docs.moth` as the sole blocker until the check reaches completion.

## A2. Complete semantic consistency

### Uniform `String` cleanup

Review at least:

```text
docs/language-overview.md
docs/compiler-design-overview.md
docs/src/docs/language-overview/**
docs/src/docs/numbers/**
docs/src/docs/functions/**
docs/src/docs/choices/**
docs/src/docs/errors/**
docs/src/docs/collections/**
docs/src/docs/templates/**
docs/src/docs/generics/**
docs/src/docs/traits/**
docs/src/docs/packages/core/**
```

Required outcomes:

- no page permits source string `+`
- no example joins strings with `+`
- no page lists templates as an unsupported equality payload
- no page lists templates as a distinct invalid map-key type
- all runtime `String` values share equality and map-key semantics
- no page permits string ordering or relational string patterns
- internal template append remains clearly separate from source binary operators
- current compiler drift remains recorded in the progress matrix until Stage B lands

Delete the remaining monolith and focused-reference contradictions rather than qualifying them.

### Return contract cleanup

Delete every remaining reference to an authored alias-candidate return slot.

The Functions and Errors references must agree that:

- success return slots contain types
- one final fallible slot may carry a channel
- source signatures have no alias candidate category
- freshness and aliasing are inferred compiler facts

Do not leave obsolete syntax as an edge case or legacy diagnostic note.

### Pattern and Moth Template consistency

Confirm across the monolith, focused pages and progress matrix that:

- `else =>` is the only catch-all
- bare general capture is removed from accepted design
- relational patterns support only `Int`, `Float` and `Char`
- Moth Template implicit names collide rather than shadow
- each accepted-but-not-yet-implemented rule has one concise implementation-gap note

Rewrite the Moth Template collision example as an explicit invalid example followed by a corrected renamed export.

## A3. Repair examples and code profiles

Every non-trivial example must be valid current Moth unless it is clearly labelled invalid or accepted deferred syntax.

Correct the known defects:

- mutable reassignment starts with `~=`
- values used through mutable receivers are held in mutable bindings
- `push`, `get`, `set` and `remove` use postfix `!`, meaningful `catch` or an allowed invariant assertion
- postfix `!` appears only inside a compatible fallible function
- Core IO construction does not use top-level `io.input.new()!`
- project examples declare every referenced name
- entry `config:` section records use `#=`
- explanatory region notation uses a plain text code profile rather than Moth source highlighting
- invalid examples are visibly labelled
- shared aliases are not named as copies

Use concise invariant handlers where appropriate:

```moth
independent ~= copy original

~independent.push(4) catch:
    assert(false, "unexpected push failure")
;
```

```moth
first = original.get(0) catch:
    assert(false, "known valid index")
;
```

For each new or corrected non-trivial example, provide one of:

- an existing compiler fixture that proves the form
- a temporary focused probe run during the patch
- an explicit `INVALID` or `ACCEPTED DEFERRED` label

Delete temporary probes before completion.

## A4. Correct Project Structure status and examples

The new Build Inputs, Entry Config and Project Package Facade pairs exist. Finish their contracts.

Required corrections:

- fix undeclared names in `entry-config.mtf`
- use valid `html #= |...|` syntax inside entry config examples
- keep `@project` explicit rather than implicitly injected
- keep facade restrictions and project-context provenance precise
- state that support roots and project facades reject top-level runtime work and fragments
- preserve module-root-relative imports and support-package visibility

Build inputs, `@project` and entry-local `config:` remain queued implementation work.

Their Basic pages must not teach them as available current syntax. Either:

- remove the deferred surface from the beginner path for now, or
- lead with an unmistakable accepted-deferred warning and use future-tense explanation

Advanced pages may own the accepted end-state contract but must link to the progress matrix for current support.

## A5. Correct Core, Builder and external package contracts

Cross-check each Advanced package page against compiler package registration, the language monolith and the progress matrix.

### Core IO

Document the exact registered input surface:

```text
new
update
close
key_down
key_pressed
key_released
last_key_pressed
last_key_released
pointer_down
pointer_pressed
pointer_released
pointer_x
pointer_y
last_pointer_pressed
last_pointer_released
```

Remove invented or stale names such as `key_held`, `pointer_up` and `pointer_held` unless the compiler actually registers them at the reviewed checkpoint.

Wrap fallible handle creation in a compatible function or local `catch` example.

### Core Math

Remove backend-defined Float semantics.

Moth-level non-finite results follow the checked numeric contract. If the current external lowering does not enforce that contract, document the implementation gap in the progress matrix and assign its correction to Stage B or a dedicated numeric plan.

### Prelude

Do not describe a different alias name as shadowing `io`.

A separate alias introduces another local namespace. A same-name collision follows the ordinary no-shadowing model.

### Core Text

Define one backend-neutral unit for `text.length`.

Audit the runtime helper, `Char` semantics and existing language design first. Do not canonise JavaScript UTF-16 length merely because it is the current lowering.

If no accepted rule exists, stop this item and present the user with an explicit design decision. Do not mark Stage A complete while the public Advanced contract remains backend-dependent.

Once decided:

- document the exact unit in Basic and Advanced
- record current implementation support in the progress matrix
- assign any compiler or runtime correction to Stage B or a dedicated plan

### Remaining package surfaces

Audit and complete source-facing ownership for:

```text
@core/collections
@core/random
@core/time
@html
@web/canvas
annotated project-local JavaScript bindings
future value-only WIT imports
```

Advanced package pages must state stable names, parameter access, return and error contracts, opaque resource rules, teardown requirements, unsupported source forms and deferred surfaces.

The progress matrix owns backend availability. Package pages must not turn current JavaScript implementation details into backend-dependent language semantics.

## A6. Complete the public Design Scope route

The public route under `docs/src/docs/design-scope/**` must become the complete source-facing owner.

Do not rely on the codebase summary alone.

Required outcomes:

- `excluded-language-families.mtf` contains every outside-scope family and rationale
- source-visible lifetime, reference-category and ownership annotations are included
- backend-dependent observable semantics are included
- first-class results, expected errors and invariant assertions remain distinguished
- `deferred-and-outside-scope.mtf` preserves the distinct deferred-feature and outside-design-scope diagnostic lanes
- the public Advanced pages stop sending readers back to the monolith for the missing exact list
- the codebase summary links to the public owner without competing with it

Basic pages should explain the language's bias without presenting the full exclusion inventory.

## A7. Complete index, parity and link ownership

### Focused language index

Update:

```text
docs/src/docs/codebase/language/overview.mtf
```

It must list:

- the public Memory and Lifetimes owners
- the public Design Scope owners
- Build Inputs, Entry Config and Project Package Facade pairs
- Core package Basic and Advanced owners
- every other focused owner completed during closeout

Do not claim every listed file is in final shape until Stage A and Stage W are accepted.

### Parity ledger

Create a compact companion ledger:

```text
docs/roadmap/plans/docs-language-migration-parity-ledger.md
```

Record one row per monolith section or delegated formal authority with:

- source heading or authority
- Advanced owner
- Basic owner
- public route
- related formal owner
- examples preserved
- current implementation status
- remaining discrepancy
- completion state

The ledger is audit evidence, not a prose diary. Keep entries terse.

### Links and route ownership

Audit source links and generated hrefs for every changed route.

Required outcomes:

- progress links resolve to `/docs/progress/`, not a codebase path
- public codebase links stay under `/docs/codebase/`
- no public link targets a repository Markdown file through an invalid site-relative URL
- `Read next` links and anchors resolve
- Previous and Next links work in both directions
- Basic pages link to the Advanced panel or route correctly
- no route-local import or public link relies on `@./...`

## A8. Complete Stage A validation

After all corrections:

```sh
cargo run --quiet -- check docs --terse
cargo run --quiet -- build docs --release
```

The release build is the required final gate. The check command is the fast preflight.

Then inspect:

- every changed route
- generated `docs/release/**` diffs
- Basic as the default selection
- independent selector behaviour
- one H1 per page
- heading and anchor stability
- code highlighting
- tables
- links and pagers
- narrow layout
- dark mode
- generated output provenance

At minimum inspect the changed routes for:

- Memory and Lifetimes
- Design Scope
- Project Structure
- Packages and Imports
- every Core package page
- Numbers
- Functions
- Branching
- Choices
- Collections and Maps
- Moth Templates

Do not edit generated HTML manually.

## Stage A acceptance gate

Stage A is ready for user review only when:

- docs source checking passes
- the release build passes
- the styles support package is canonical
- no supported import uses `@./...`
- no focused or monolith reference contradicts a locked semantic decision
- examples are valid or clearly labelled
- Core package contracts match registered APIs
- no package page leaks backend-defined semantics into the language contract
- the public Design Scope route is complete
- the focused index is current
- the parity ledger is complete
- every changed route has been inspected
- the report states exact commands and remaining uncertainty

Stage A completion requires explicit user acceptance after review.

---

# Stage W: writing-style pass

Stage W begins only after Stage A technical correctness is complete, validated and accepted.

This is a separate documentation-only phase. Do not mix it into the technical closeout.

## Goals

Review the complete focused language documentation for:

- clear beginner progression in Basic files
- complete direct-reading contracts in Advanced files
- concise wording without semantic compression
- consistent terminology
- natural paragraph and sentence rhythm
- precise headings and transitions
- removal of accidental repetition
- examples introduced before edge cases
- clear separation of current, deferred, rejected and outside-scope behaviour
- consistent British English and repository style-guide rules

## Constraints

- do not remove a normative rule to shorten a page
- do not merge distinct edge cases into vague summary prose
- do not move formal compiler, build-system or memory architecture into public language pages
- do not turn Basic pages into status dashboards
- do not change accepted semantics silently
- flag any newly discovered design ambiguity before rewriting around it
- keep the parity ledger updated when ownership moves

## Validation

Run the documentation release build again and inspect every route changed by the style pass.

Stage W completion requires a separate user review and acceptance.

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
