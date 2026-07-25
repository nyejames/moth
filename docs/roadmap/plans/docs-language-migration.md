# Moth language documentation completion and compiler semantic realignment

## 1. Purpose and required order

This plan completes the focused language documentation migration and then realigns the compiler with the accepted language contract.

Commit `d707782fcb92a3e73dbc7ee9371820a6344e198f` is the reviewed documentation baseline. It added useful route scaffolding and established the accepted direction for strings, matching, Moth Templates collisions, memory and design scope. It did not complete the parity migration.

The remaining work has three ordered stages:

1. **Stage A: documentation-only completion**
   - correct contradictory references
   - repair invalid examples
   - restore information that was overcompressed in Advanced files
   - finish missing source-facing owners
   - complete the parity ledger and focused-reference index
   - build and inspect the documentation
2. **Stage B: compiler semantic realignment**
   - remove obsolete syntax and implementation paths
   - make current compiler behaviour match the completed documentation contract
   - close the accepted non-deferred implementation gaps in this plan
3. **Stage C: whole-language authority review and switch**
   - run final parity review
   - request explicit user approval
   - update authority routing only in a separate approved patch

**Stage B must not begin until Stage A has passed review.**

Stage A may update the monolith, focused documentation, compiler-design wording, build-system wording, memory links, the progress matrix and generated documentation. It must not change Rust source, compiler tests, executable fixtures, manifests or build behaviour.

Moth is early Alpha. Stage B removes obsolete design completely. It does not retain compatibility syntax, deprecation periods, legacy diagnostics, dormant adapters or redundant internal data shapes.

---

## 2. Completed foundation removed from active work

The following work is complete enough to treat as foundation rather than an active task:

- the Basic and Advanced selector component
- Basic as the default documentation level
- independent concept selectors
- stable page H1 and concept H2 structure
- explicit Previous and Next navigation
- focused pairs for the existing core language routes
- the initial public Memory and Lifetimes route
- the initial public Design Scope route
- the accepted removal of general match capture from the documented design
- the accepted removal of string relational ordering from the documented design
- the accepted Moth Templates no-shadowing direction
- the accepted template-based string concatenation direction
- generated route creation for the new pages

Historical migration batches, probe diaries and landed-route narratives remain in Git history. They are not repeated in this plan.

The existing route files remain subject to final parity review. A route's existence does not prove that its Advanced content is complete or that adjacent references agree with it.

---

## 3. Authority and documentation rules

### 3.1 Authority during this plan

Use this order when sources disagree:

1. Explicit user decisions recorded in this plan
2. `docs/language-overview.md` as the maintained source-facing parity baseline
3. `docs/src/docs/codebase/memory-management/**` for formal memory semantics
4. `docs/compiler-design-overview.md` for compiler stage and artefact contracts
5. `docs/build-system-design.md` for project, package, builder and link architecture
6. The progress matrix for current compiler and backend support
7. Accepted roadmap plans for explicitly deferred implementation
8. Compiler implementation and tests as evidence of current behaviour
9. Existing public pages as teaching material under review

Implementation is not automatically language design. When accepted design and current implementation differ, Stage A documents the accepted contract and records the implementation gap accurately. Stage B then removes the gap.

### 3.2 Responsibility split

| Source | Responsibility |
|---|---|
| Unsuffixed focused `.mtf` files | Complete Advanced source syntax and observable semantics |
| Paired `-basic.mtf` files | Beginner teaching consistent with Advanced |
| Public `#page.moth` files | Composition, introductions, ordering, navigation and presentation |
| `docs/language-overview.md` | Maintained parity baseline until authority switch |
| Focused language index | Routing map to every final focused owner |
| Memory references | Formal alias, lifetime, ownership and backend-neutral memory rules |
| Compiler design | Stage ownership, semantic identities, IR and analysis contracts |
| Build-system design | Project, config, package, builder, link and output contracts |
| Progress matrix | Current implementation, rejection and backend coverage |
| Roadmap | Accepted deferred work and implementation order |

### 3.3 Advanced requirements

Each unsuffixed Advanced file must be directly readable without its page or Basic partner.

It must contain, where relevant:

- a concise contract
- canonical syntax
- exact semantic rules
- type and inference boundaries
- scope and receiving-context restrictions
- access, mutation, copy and lifetime effects
- observable evaluation-order guarantees
- important edge cases
- rejected forms
- accepted deferred forms
- outside-scope forms
- links to adjacent language and architecture owners

An Advanced file may summarise a formal compiler, build-system or memory authority. It must not omit source-observable legality merely because the analysis algorithm is documented elsewhere.

### 3.4 Basic requirements

Basic files teach the stable mental model and common current syntax.

They must:

- introduce terminology before using it
- use complete, valid examples
- avoid compiler-stage and backend internals
- avoid exhaustive edge-case inventories
- avoid presenting deferred syntax as current syntax
- remain true when Advanced detail is omitted
- link to Advanced rather than compressing complex behaviour into a false simplification

### 3.5 Current design versus current compiler

Stage A finishes the accepted language documentation before Stage B changes the compiler. Therefore some Advanced pages will temporarily describe accepted semantics that the current compiler does not yet enforce.

For each such surface:

- state the accepted language contract normatively
- add one concise implementation-gap note in the Advanced owner
- link to the progress matrix
- update the progress matrix to describe the current drift accurately
- do not repeat a status dashboard across every related page
- do not claim "the compiler rejects" a form until Stage B actually makes it reject

Basic pages should teach the accepted form and normally omit implementation archaeology.

When Stage B lands, remove the temporary implementation-gap note and update the progress matrix in the same semantic slice.

### 3.6 Editing constraints

- Follow `docs/src/docs/codebase/style-guide/style-guide.mtf`.
- Use straight apostrophes.
- Avoid em dashes.
- Use exact Moth syntax.
- Label invalid examples clearly.
- Keep every code example type coherent.
- Do not address the reader as an agent or LLM.
- Retain legitimate LLM-aware language and tooling discussion.
- Do not generate Basic prose mechanically from Advanced prose.
- Do not use automated multi-file prose replacement.
- Search and read-only inventory tooling is allowed.
- Do not edit generated HTML manually.

---

## 4. Accepted semantic decisions

These decisions are final for both documentation and later compiler work.

### 4.1 Source-authored return-alias syntax is removed

Function signatures declare return value types and return channels only.

Forms such as:

```moth
choose |first String, fallback String| -> first or fallback:
    return first
;
```

are not Moth syntax.

Return aliasing remains compiler-owned semantic information:

- source function summaries are inferred from validated bodies and calls
- public interfaces may export inferred alias and freshness facts
- external binding metadata may describe aliasing because foreign bodies are unavailable
- the information is not a source type or signature category

Stage B removes all parser, AST and HIR support that exists only for source-authored alias returns.

### 4.2 One semantic `String` surface

Quoted slices and template-produced strings share one semantic `String` type at ordinary typed boundaries.

They share:

- parameter and return compatibility
- equality
- choice and option payload equality
- collection and map value use
- `String` map-key use
- casts
- IO and external string-content boundaries
- aliases and concrete generic use
- template insertion

Construction origin must not create hidden equality, hashing, map-key or call-compatibility rules.

Quoted strings create deliberately restricted read-only slices. A mutable binding may be reassigned to another string, but that does not make the slice content mutable.

Templates create full owned string values and are the canonical source mechanism for concatenation, interpolation and structured text construction.

This plan does not add an in-place string mutation API or a second public `StringSlice` type.

### 4.3 `+` is numeric only

Source-level string concatenation uses templates:

```moth
joined = [left, right]
```

This is invalid in the accepted language:

```moth
joined = left + right
```

Internal template lowering may use a string append or concatenation operation. That implementation detail does not make source `+` valid for strings.

### 4.4 `String` has equality but no ordering operators

`String` supports `is` and `is not`.

It does not support:

- `<`
- `<=`
- `>`
- `>=`
- relational string match patterns

Future text ordering belongs in explicit Core text APIs.

### 4.5 `else =>` is the only full-match catch-all

A bare identifier is not a general capture pattern.

Valid binding patterns remain:

- option present capture with `|name|`
- declared choice payload captures
- renamed choice payload captures with `as`

An unknown bare name in choice pattern position is an unknown choice variant. A bare name in another full-match pattern position is invalid.

### 4.6 Moth Templates implicit scope does not shadow

A `.mtf` body may receive implicit compile-time constants from:

- the HTML builder package
- its same-directory module root public surface

If both surfaces expose the same visible name, compilation fails with the ordinary visible-name collision model. Same-directory constants do not override `@html` constants.

### 4.7 Assertions in teaching examples

`assert` is Moth's explicit source-level invariant-failure or panic statement. It is not normal expected-failure handling.

A documentation example may use `assert(false, "message")` inside a `catch` when:

- failure has already been made impossible by the example's setup
- the example is teaching another feature rather than error recovery
- a full recovery branch would obscure the lesson

For example:

```moth
~items.push(4) catch:
    assert(false, "unexpected push failure")
;
```

A value-producing handler may also terminate with an assertion:

```moth
first = items.get(0) catch:
    assert(false, "index was checked")
;
```

Rules for documentation examples:

- use the current literal-message syntax
- explain once in the Assertions docs that `assert` is unrecoverable
- use `Error!`, postfix propagation or meaningful `catch` recovery when failure is expected
- do not use assertions to hide a real runtime error path
- do not claim assertion checks are debug-only or release-elided unless that build-profile contract is separately accepted and documented

---

## 5. Stage A: documentation-only completion

Stage A is the next task. It blocks all compiler work in this plan.

### 5.1 Global semantic consistency pass

Perform a repository-wide review for every accepted decision in section 4.

#### String consistency

At minimum review and correct:

```text
docs/src/docs/language-overview/strings-and-characters.mtf
docs/src/docs/language-overview/strings-and-characters-basic.mtf
docs/src/docs/numbers/operators.mtf
docs/src/docs/numbers/operators-basic.mtf
docs/src/docs/functions/**
docs/src/docs/choices/choice-equality.mtf
docs/src/docs/errors/options.mtf
docs/src/docs/collections/hash-maps.mtf
docs/src/docs/templates/**
docs/src/docs/generics/**
docs/src/docs/traits/generic-trait-bounds.mtf
docs/src/docs/packages/core/**
docs/language-overview.md
docs/compiler-design-overview.md
```

Required outcomes:

- no Advanced page says plain string slices can use `+`
- no example joins strings with `+`
- no page treats template strings as a distinct unsupported equality category
- no page rejects template-produced values as `String` map keys
- no page permits string ordering
- the compiler-design document distinguishes internal template append from source binary operators
- the Strings Advanced page distinguishes restricted quoted slices from owned template-produced strings without inventing two semantic types
- current compiler drift is reported through one Advanced note and the progress matrix until Stage B lands

#### Pattern consistency

Review the monolith, Branching pages, Options pages and generated output.

Required outcomes:

- no general capture is described as valid
- `else =>` is the sole catch-all
- relational patterns list only `Int`, `Float` and `Char`
- unknown choice names are described consistently
- current compiler acceptance of removed forms is recorded as an implementation gap until Stage B

#### Moth Templates consistency

Review the monolith, Moth Templates pages, package visibility wording and generated output.

Required outcomes:

- no page documents local-over-HTML precedence
- collisions use the ordinary no-shadowing model
- the example shows an actual invalid collision and its renamed correction
- current compiler precedence behaviour is recorded as an implementation gap until Stage B

### 5.2 Repair every invalid or misleading example

Correct the examples introduced or exposed by the `d707...` review.

Required corrections include:

- mutable reassignment begins with `~=`
- a value mutated through a receiver is held in a mutable binding
- `push`, `get`, `set` and `remove` use `!`, meaningful `catch` or the assertion policy in section 4.7
- postfix `!` appears only inside a compatible fallible function
- symbolic `==` is not presented as a Moth operator
- shared aliases are not named `copy_of`
- invalid examples are commented or placed in clearly labelled invalid blocks
- examples do not depend on undeclared values, imports, methods or result channels

Recommended corrected forms:

```moth
name ~= "Priya"
name = "Aisha"
```

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

For every non-trivial new source example, provide one of:

- an existing compiler test or fixture that proves it
- a temporary focused probe compiled during the patch
- a clear `INVALID` label when it intentionally does not compile

Delete temporary probes before completion.

### 5.3 Finish the Advanced Memory and Lifetimes route

The public route exists. Its Advanced content is not yet complete enough to replace the monolith's source-facing memory section.

#### `reference-semantics.mtf`

Ensure it covers:

- existing-value reads, bindings, arguments, returns and storage as shared access
- non-lexical, control-flow-sensitive alias activity
- branches, joins and loop conservatism at a source-observable level
- no source reference constructors or lifetime annotations
- backend representation not changing source semantics

Do not reproduce borrow-checker algorithms or side-table layouts.

#### `copy-and-exclusive-access.mtf`

Ensure it covers:

- the complete semantic deep-copy contract
- preservation of internal alias topology
- preservation of same-region cycles
- no mutable sharing with the source graph
- reactive sources copied as current values rather than reactive identity
- non-copyable external resources producing diagnostics
- valid copy places and rejected computed operands
- mutable write-through aliases versus fresh mutable slots
- `~place` as exclusive access rather than move syntax
- fresh values satisfying ordinary mutable parameters without source `~`
- temporaries remaining invalid mutable receivers

#### `lifetimes-and-result-shapes.mtf`

Add the source-facing contract for:

- mandatory lifetime-topology validation versus optional ownership optimisation
- exactly one semantic lifetime owner for each allocation
- the retained-edge outlives rule
- lexical scope not defining allocation lifetime
- nearest-existing-ancestor widening on one ordered owner chain
- no lateral widening across independently ending sibling domains
- fresh result roots that may retain legal older references
- alias results and projection roots
- independent result graphs
- projections remaining rooted in their allocation family
- return and multi-return alias consequences
- same-region cycles versus invalid cross-region cycles
- reactive and builder-owned lifecycle roots
- restricted host bindings and future value-only WIT boundaries

Keep region-solving algorithms and compiler artefact details in the formal memory references.

Correct the Basic statement that every fresh value is independent. A fresh root may retain legal references. `copy` provides an independent graph.

#### `declared-memory-groups.mtf`

Preserve the complete accepted source contract, including:

- runtime-executable-body-only placement
- current or ancestor group targets only
- no sibling, child, unrelated or named builder-lifecycle targets
- groups not being values or types
- group-name collision rules
- closure on every control-flow exit
- exact `into` declaration position
- destination-scope visibility
- straight-line ancestor placement restriction
- conditional and loop alternatives
- fresh, alias and independent placement eligibility
- nested-group retained-edge rules
- no extraction or unrestricted group-to-group adoption
- invalid escapes and projection escapes
- reassignment rules for group-owned mutable bindings
- reactive-storage restrictions
- hidden result destinations not becoming source signature parameters

Correct Basic wording so group-owned values cannot outlive the group. Do not say they merely live "at least as long" as the group.

### 5.4 Repair public links

Audit source and generated links for the new Memory and Design Scope routes.

In particular:

- links from `docs/src/docs/memory/**` to public codebase pages must resolve under `/docs/codebase/**`
- public pages must not link to repository Markdown through a broken site-relative path
- use an explicit GitHub link when the destination has no public docs route
- verify every `Read next` anchor after generation
- verify Previous and Next links in both directions

### 5.5 Complete the Functions return contract

Expand the Advanced return owner to state:

- return slots contain types and channels only
- source code has no borrowed, owned, move or parameter-alias return annotation
- returning an existing value follows ordinary shared-reference semantics
- the compiler infers freshness and alias effects
- public interfaces may carry inferred summaries
- external bindings may carry explicit compiler-owned alias metadata
- the summary lattice is not source syntax

Remove every valid-looking source-authored alias-return example from all docs.

### 5.6 Finish Project Structure and Packages

The route scaffolding exists, but the final source-facing surface is incomplete.

Provide complete Advanced ownership for:

- self-contained `config.moth`
- direct project `#Import`
- source `#Import`
- `@project`
- entry-local `config:` blocks
- normal `#*.moth` module roots
- API-only `+*.moth` support roots
- the project package facade
- active versus dormant normal-root work
- directory-based routes and builder-owned artefacts
- module-root-relative imports
- support-package visibility
- dependency package boundaries
- package origin and backing
- external binding boundaries visible to authors

Split concepts when a direct-readable Advanced file would otherwise become dense. Suggested pairs remain:

```text
build-inputs.mtf
build-inputs-basic.mtf

entry-config.mtf
entry-config-basic.mtf

project-package-facade.mtf
project-package-facade-basic.mtf
```

Correct every claim that support-root runtime work or fragments merely remain inactive. Support roots and the project package facade reject top-level runtime work and page fragments.

Basic pages should teach the current simple project model. Accepted deferred config and package surfaces must be labelled clearly and kept out of the beginner path until implemented.

### 5.7 Convert Core, Builder and external package documentation

The existing Core package pages are useful, but they still mix teaching, exact contracts, current backend support and future roadmap in single-level pages.

Convert or compose them into the same Basic and Advanced model.

Advanced owners must cover:

- stable import roots and prelude policy
- stable public functions, constants and opaque types
- parameter, return, access and error contracts
- source-backed versus binding-backed behaviour visible to authors
- explicit close or teardown requirements
- restricted host-value boundaries
- unsupported source forms
- deferred package API families

The progress matrix owns current target availability. The build-system design owns provider registration and linking. The memory references own retention and external-resource lifetime rules.

At minimum cover:

```text
@core/io
@core/collections
@core/math
@core/text
@core/random
@core/time
@html
@web/canvas
annotated project-local JavaScript bindings
future value-only WIT imports
```

### 5.8 Make Design Scope a complete focused owner

The public Design Scope route must stop delegating exact completeness back to the monolith.

Advanced content must preserve:

- the exact deferred versus outside-scope distinction
- every excluded language family
- the rationale for each family
- the constrained Moth mechanism used instead
- source-visible lifetime, reference-category and ownership annotations as outside scope
- backend-specific observable semantics as outside scope
- expected failure through `Error!`, invariants through `assert` and explicit result-like domain values through ordinary choices
- the distinction between deferred-feature diagnostics and outside-design-scope diagnostics

Basic should explain the language's bias without reproducing the complete exclusion inventory.

### 5.9 Complete the focused-reference index and parity ledger

Update:

```text
docs/src/docs/codebase/language/overview.mtf
```

It must list the new Memory and Design Scope owners and every completed focused route.

Add and maintain a section-level parity ledger in this plan or an explicitly approved companion file.

Each row must record:

| Field | Meaning |
|---|---|
| Source heading or delegated authority | Original normative source |
| Advanced owner | Final unsuffixed file |
| Basic owner | Teaching file |
| Public route | Importing page |
| Related formal owner | Memory, compiler or build-system reference |
| Advanced complete | Yes or no |
| Basic complete | Yes or no |
| Important examples preserved | Yes or no |
| Implementation checked | Yes, no or not applicable |
| Current discrepancy | Description or none |
| Status | Current, implementation gap, deferred, rejected or outside scope |

A section is complete only when its Advanced owner is direct-read complete and its Basic partner remains true.

### 5.10 Stage A route checklist

| Route or owner | Remaining documentation work |
|---|---|
| Language Basics | Correct mutable string example and final string wording |
| Numbers | Remove plain-string `+` from Advanced |
| Functions | Add final return and inferred-alias contract |
| Branching | Retain accepted contract and mark temporary compiler drift |
| Choices | Remove template as a distinct unsupported equality surface |
| Errors and Options | Confirm uniform `String` and assertion wording |
| Collections and Maps | Uniform `String` keys and complete memory semantics |
| Templates | Canonical concatenation and source/internal append distinction |
| Generics and Traits | Replace remaining string `+` examples |
| Reactivity | Link to public Memory route and preserve metadata/type distinction |
| Memory and Lifetimes | Expand Advanced source contract and repair Basic inaccuracies |
| Project Structure | Add build inputs, entry config and facade ownership |
| Packages and Imports | Correct support-root legality and complete package boundaries |
| Core and Builder Packages | Convert to Basic and Advanced semantic owners |
| Moth Templates | Rewrite collision example and mark temporary compiler drift |
| Plain Markdown | Final boundary and link audit |
| Design Scope | Become the complete exact focused owner |
| Language monolith | Synchronise every accepted decision and gap note |
| Compiler design | Remove source string-concatenation implication and preserve internal append |
| Focused language index | List every final owner |
| Progress matrix | Record every pending Stage B mismatch accurately |

### 5.11 Stage A validation

Stage A is documentation-only.

Use the documentation-only final gate from the style guide:

```sh
moth build docs --release
```

or, when a suitable release compiler is unavailable:

```sh
cargo run --quiet -- build docs --release
```

Targeted iteration may use:

```sh
moth check docs
```

or focused compiler probes for examples.

Do not run `just validate` merely for a strictly documentation-only patch.

After the release build:

- inspect every changed route
- inspect generated diffs
- verify links and anchors
- verify Basic is selected by default
- verify selectors remain independent
- verify one H1 per page
- verify examples, tables and code highlighting
- verify narrow layout and dark mode where route structure changed
- confirm generated output came from source changes
- confirm generated HTML was not edited manually

### 5.12 Stage A completion gate

Stage A is complete only when:

- no focused Advanced reference contradicts another accepted owner
- no monolith rule contradicts the accepted decisions
- no architecture document implies rejected source behaviour
- every new and changed example is valid or clearly labelled invalid
- the Advanced Memory route preserves the full source-facing legality surface
- Project Structure, Packages and Core package contracts have focused owners
- Design Scope is complete without delegating exactness back to the monolith
- all public links resolve
- the focused-reference index is current
- the parity ledger is complete
- every current compiler mismatch is recorded in the progress matrix
- the documentation release build passes
- every changed route has been inspected
- the review report contains no unresolved documentation ambiguity

Only then may Stage B begin.

---

## 6. Stage B: compiler semantic realignment

Stage B begins only after Stage A approval. Each semantic slice updates compiler code, tests, focused docs, the monolith and the progress matrix together.

### 6.1 Remove source-authored return aliases

Required outcomes:

- delete parameter-name return parsing
- delete alias-return syntax variants and helpers
- remove alias-only diagnostics that no longer have another owner
- simplify AST return representation
- remove source-declared alias arrays from HIR
- remove AST-to-HIR alias-candidate transfer
- retain inferred return-alias summaries
- retain external binding alias metadata
- compute same-module summaries deterministically or through a monotone fixed point
- treat recursive or unresolved cycles conservatively as unknown
- export stable inferred summaries through public interfaces
- add no compatibility parser or legacy diagnostic

Search-zero checks include:

```text
AliasCandidates
parse_alias_return_item_syntax
source-declared return_aliases
```

General inferred and external alias metadata must remain.

### 6.2 Simplify the compiler `String` surface

Required outcomes:

- remove source `String + String` typing
- remove compile-time string-add folding
- make equality accept every runtime `String` regardless of source construction
- make option and choice equality recurse through `String` consistently
- make `String` map keys use uniform content equality and hashing
- keep compile-time path values outside runtime string operators
- remove value-shape checks that create hidden equality or map semantics
- retain value metadata needed for template and reactive lowering
- separate internal template append from source binary operators
- remove backend comments and branches that advertise source string addition

Required tests include:

- slice equality
- template equality
- slice versus template equality
- `String?` equality
- choice payload equality with both construction forms
- map keys produced by both forms
- source string `+` rejection
- mutable binding reassignment of a slice
- rejection of slice-content mutation
- template concatenation through `[left, right]`

### 6.3 Simplify patterns

Required outcomes:

- remove `String` from relational pattern subjects
- retain `Int`, `Float` and `Char`
- delete general capture AST and HIR variants
- delete whole-scrutinee capture scope construction
- delete capture lowering, validation, display and backend paths
- make unknown choice names diagnose as unknown variants
- make bare names invalid for non-choice full matches
- preserve option `|name|` and choice payload captures
- keep `else =>` as the only catch-all
- add no legacy general-capture diagnostic

Update unit tests, HIR tests, integration fixtures, backend tests and benchmarks.

### 6.4 Align Moth Templates implicit collisions

Required outcomes:

- register implicit `@html` and same-directory constants through the ordinary visible-name registry
- remove overwrite-based precedence
- retain source locations for both collision participants
- reject collisions before AST folding
- keep unique constants from both surfaces visible
- keep filtering to exported compile-time constants and const records
- keep the generated `content` constant out of its own scope
- replace precedence tests with collision tests

### 6.5 Close accepted non-deferred gaps

Reproduce each gap against current `main` before changing it. Remove a row when it is already fixed and record the evidence.

This plan owns these gaps when they still reproduce:

1. **Option payload equality inside choices**
   - `T?` supports equality when `T` supports equality
   - recursive equality remains cycle safe
2. **Cross-choice inline predicate validation**
   - a choice variant must belong to the scrutinee's nominal choice
3. **Nested-block `return!` in error-only functions**
   - terminal from every legal nested control-flow block
4. **Block value-producing `if` with `then`**
   - reaches AST, HIR and backend lowering without infrastructure failure
5. **Stored named template inserts**
   - preserve slot identity when contributed through a binding

Each correction needs focused success, rejection, HIR and integration coverage as appropriate.

### 6.6 Stage B validation

For every code-bearing semantic slice:

```sh
cargo fmt
just validate
moth build docs --release
```

Use the equivalent Cargo docs build when necessary.

Also audit that:

- obsolete variants and adapters are gone
- `TypeId` remains semantic type authority
- value metadata does not become a hidden second string type
- inferred alias facts remain side-table or interface facts
- borrow analysis does not mutate HIR
- backends do not reinterpret removed source syntax
- internal template append is not confused with source string addition
- user-facing failures remain structured diagnostics
- no compatibility shim survives

---

## 7. Stage C: final parity and authority switch

After Stage B:

1. Audit every monolith section and delegated formal authority.
2. Audit every Advanced file as a direct reference.
3. Audit every Basic file for truthfulness and learning quality.
4. Inspect every public route.
5. Confirm every deferred and outside-scope surface has one owner.
6. Confirm the progress matrix matches current implementation.
7. Confirm the focused-reference index and parity ledger are complete.
8. Present any remaining ambiguity or mismatch to the user.

The final authority switch requires explicit user approval.

Only that separate patch may:

- update `AGENTS.md`
- declare focused references authoritative
- decide whether `docs/language-overview.md` remains a consolidated legacy reference, becomes an index or is removed

No earlier patch may imply that authority has switched.

---

## 8. Required report for every slice

### Scope

- stage and workstream covered
- starting commit and branch
- authorities read

### Files

- source documentation changed
- architecture or monolith files changed
- compiler and test files changed, when Stage B
- generated files changed

### Semantic result

- accepted rule
- valid forms
- invalid forms
- current implementation status
- deferred dependencies

### Deletions and simplification

For Stage B:

- obsolete syntax removed
- variants, helpers and diagnostics removed
- similarly named retained concepts explained

### Parity

- monolith headings reviewed
- Advanced owner for every rule
- Basic owner for every concept
- important examples preserved, replaced or intentionally removed
- parity-ledger rows updated

### Validation

Report exact results of:

- targeted example probes
- documentation release build
- route inspection
- `cargo fmt` and `just validate` for Stage B

Do not claim commands or inspection that did not occur.

### Remaining uncertainty

Report every unresolved:

- documentation ambiguity
- implementation conflict
- deferred dependency
- incomplete parity row
- route not inspected fully

Do not hide uncertainty to declare a slice complete.

---

## 9. Protected files and final constraints

- `AGENTS.md` remains unchanged until the explicitly approved authority-switch patch.
- Generated HTML is never edited manually.
- Documentation prose is edited manually, file by file.
- Compiler removals do not retain legacy compatibility paths.
- The progress matrix remains the implementation-status authority.
- The roadmap remains the deferred-work authority.
- Formal memory, compiler and build-system architecture stays in its dedicated owners.
- Focused Advanced language pages retain the complete source-facing projection needed by authors and compiler contributors.
