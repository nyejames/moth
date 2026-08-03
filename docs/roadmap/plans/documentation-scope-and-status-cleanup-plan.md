# Documentation scope, status and example-name cleanup

## Purpose

Complete a focused documentation-only cleanup after the language documentation migration.

This work must:

- preserve transparent type aliases exactly as currently designed
- distinguish accepted deferred implementation from unresolved design questions
- make the public `/docs/design-scope/` route the sole Design Scope authority
- delete the competing codebase Design Scope route
- remove outside-scope and undecided features from the progress matrix
- correct known status drift around results, aliases, traits, maps, generics and function values
- reduce user-facing internal implementation detail where it obscures the language contract
- retain useful technical depth in Advanced documentation
- apply a small, low-risk cleanup to example names

Do not recreate the deleted language-migration plan or parity ledger. Do not edit roadmap status or migration completion metadata.

## Baseline

```text
BASE_REVISION: 58432008fb5a1f7bb117dd226ccac773c25ed8c2
CHANGE_CLASS: documentation-only
COMPILER_CHANGES: none
LANGUAGE_BEHAVIOUR_CHANGES: none
REQUIRED_FINAL_GATE: documentation release build
```

Record the actual starting commit, branch and worktree state before editing.

## Implementation status

```text
WORK_ID: documentation-scope-status-cleanup
WORK_SOURCE: docs/roadmap/plans/documentation-scope-and-status-cleanup-plan.md
BASE_REVISION: d398fbeaf8752fef08b3e4d358d05165cfe38a74
STATUS: active
CURRENT_SCOPE: checkpoint and merge accepted Design Scope authority consolidation
COMPLETED: completed drift inventory; consolidated Design Scope into the public route; updated authority links; deleted source and generated codebase routes; completed exact exclusions; restored a mutually exclusive three-way classification test
NEXT_ACTION: merge this checkpoint into main, remove the isolated worktree and continue with the progress matrix
VALIDATION: cargo run --quiet -- check docs --terse passed; cargo run --quiet -- build docs --release built 68 files; git diff --check passed; old source and generated route absent
AUDITS: interim auditor findings corrected; verification confirmed inventory and function-value scope, then identified classifier overlap and stale generated output; both corrected
BLOCKERS: none
NOTES: documentation-only; main is clean at 75ccd4aaad28510308c525bc306c9a0af2b80f46 and the user authorised merging this worktree into main
```

---

# Locked decisions

## 1. Type aliases remain transparent

The following contract is final:

```moth
UserId as Int
```

- `UserId` and `Int` denote the same semantic type
- they share one `TypeId`
- values are compatible in both directions
- the alias introduces no constructor or nominal identity
- alias chains remain transparent
- fully concrete generic aliases remain supported
- `as` must not become nominal-wrapper syntax

## 2. Nominal representation types are an open design question

A possible future zero-cost nominal wrapper for primitives or other representations remains undecided.

This is a **deferred design question**, not accepted deferred implementation.

It must not:

- appear in the progress matrix
- be presented as planned
- acquire provisional syntax
- change current alias semantics
- be implemented in this task

The documentation may state:

> Moth currently has no source form for a distinct primitive-backed nominal wrapper. Such a feature remains an open design question. Type aliases will remain transparent even if a separate nominal wrapper mechanism is considered later.

A wrapper struct remains the current way to create distinct nominal identity.

## 3. Function-value scope is precise

These remain outside scope:

- general closures
- anonymous function values
- generic function values
- higher-order polymorphism
- hidden event-handler closures

A narrower future design for references to named, monomorphic, non-capturing functions remains an **open design question**.

It has no accepted syntax or implementation commitment and must not appear in the progress matrix.

## 4. Progress matrix ownership is narrow

The progress matrix owns current implementation status only for the intended language, compiler and backend design.

Allowed status categories are:

- `Supported`
- `Partial`
- `Experimental`
- `Deferred`

The matrix may describe a structured rejection when that rejection is useful coverage for an intended feature. It must not create standalone progress rows for syntax or mechanisms that are permanently outside scope.

Outside-scope features belong in Design Scope or the relevant canonical language reference.

Open design questions belong in Design Scope or a design-gated roadmap discussion. They do not belong in the progress matrix.

## 5. Public Design Scope is the sole authority

The sole Design Scope source must live under:

```text
docs/src/docs/design-scope/
```

The route's unsuffixed Advanced files collectively own:

- design principles
- classification of accepted deferred, open and outside-scope work
- the complete outside-scope inventory
- design-review questions and alternatives

The Basic files teach a smaller user-focused view with high-level rationale.

Delete the competing route under:

```text
docs/src/docs/codebase/design-scope/
```

Do not leave a redirect or compatibility page.

## 6. Advanced docs remain technical but user-facing

Advanced references may include technical rationale and backend-neutral explanations that help users understand the language.

Keep:

- observable legality
- edge cases
- runtime behaviour
- target restrictions relevant to users
- diagnostic consequences
- useful implementation-independent mental models
- concise notes about conservative analysis when users may observe false positives

Compress or relocate:

- exact Rust type names
- helper or registry names
- internal ABI descriptions
- side-table and lattice mechanics
- duplicated current-status inventories
- backend implementation details with no user-visible consequence

Do not mechanically strip all mentions of AST, HIR, borrow checking or backends. Judge each passage by whether it helps an Advanced language user.

## 7. Example-name convention

For generic people in examples, prefer these names in order:

1. `Priya`
2. `Rob`
3. `Emmy`
4. `Huw`

This is a small aesthetic cleanup, not a broad rewriting project.

Primary goal:

- replace generic example uses of `Sam` with `Rob`

Secondary goal:

- when another generic placeholder name is already being edited nearby, prefer `Priya`, `Rob`, `Emmy` or `Huw`
- old `Linus` examples may remain
- do not replace real people, historical references, package names, test identities, filenames, diagnostics, quoted source material or examples where the name carries semantic meaning
- avoid changing fixture output unless the corresponding source and expectation are deliberately updated
- do not perform an indiscriminate repository-wide name replacement

---

# Non-goals

Do not:

- change compiler behaviour
- remove the current external opaque alias restriction
- add a nominal-wrapper type
- add function values
- change `TypeId` identity rules
- reopen the language migration
- restore `docs/language-overview.md`
- recreate the migration plan or parity ledger
- edit roadmap sequencing
- modify Rust, tests, fixtures, manifests, scripts or build configuration
- manually edit generated HTML
- overcompress Advanced semantic rules
- move formal compiler or build-system architecture into the public docs
- remove useful technical explanation merely because a compiler document also discusses the topic
- replace every personal name in the repository

---

# Required reading

Read before editing:

```text
AGENTS.md
CONTRIBUTING.md
docs/compiler-design-overview.md
docs/build-system-design.md
docs/src/docs/codebase/style-guide/style-guide.mtf
docs/src/docs/codebase/style-guide/validation.mtf
docs/src/docs/codebase/language/overview.mtf
docs/src/docs/codebase/memory-management/overview.mtf

docs/src/docs/design-scope/@page.moth
docs/src/docs/design-scope/design-principles.mtf
docs/src/docs/design-scope/design-principles-basic.mtf
docs/src/docs/design-scope/deferred-and-outside-scope.mtf
docs/src/docs/design-scope/deferred-and-outside-scope-basic.mtf
docs/src/docs/design-scope/excluded-language-families.mtf
docs/src/docs/design-scope/excluded-language-families-basic.mtf

docs/src/docs/codebase/design-scope/@page.moth
docs/src/docs/codebase/design-scope/overview.mtf

docs/src/docs/progress/@page.moth
docs/src/docs/aliases/type-aliases.mtf
docs/src/docs/aliases/type-aliases-basic.mtf
docs/src/docs/functions/function-declarations.mtf
docs/src/docs/functions/returns-and-multiple-values.mtf
docs/src/docs/generics/generic-limits.mtf
docs/src/docs/traits/trait-design-scope.mtf
docs/src/docs/reactivity/reactivity-scope.mtf
docs/src/docs/collections/hash-maps.mtf
docs/src/docs/memory/lifetimes-and-result-shapes.mtf
docs/src/docs/moth-templates/template-scope.mtf
docs/src/docs/language-overview/strings-and-characters.mtf
```

Use the deleted monolith only as historical audit evidence:

```sh
git show 1548b09bee49a3e77690894ee3e75e0863b75629:docs/language-overview.md
```

It is not an authority and must not be restored.

---

# Phase 1: inventory the current drift

Before editing, search the complete repository:

```sh
rg -n \
  "codebase/design-scope|Outside Scope|Rejected|outside scope|outside-scope|\
remain deferred or outside scope|parameterized aliases|parameterised aliases|\
first-class public Result|general function-value|raw-value ABI|\
visible-name registry|compact borrow summary|borrow state" \
  AGENTS.md CONTRIBUTING.md docs index.md
```

Inventory generic example names separately:

```sh
rg -n '\bSam\b|\bLinus\b|\bAlice\b|\bAna\b|\bGrace\b|\bGollum\b' \
  README.md CONTRIBUTING.md docs packages tests
```

The name inventory is advisory. Do not assume every match should change.

Record:

- every source link to the codebase Design Scope route
- every progress-matrix row using `Rejected` or `Outside Scope`
- every progress note mixing deferred and outside-scope features
- every canonical topic page whose classification conflicts with Design Scope
- every Advanced page containing implementation detail that may need compression
- every generated route affected by the source changes
- straightforward generic `Sam` examples suitable for replacement with `Rob`

Do not edit generated files during this inventory.

---

# Phase 2: make public Design Scope complete

Keep the current three-section public route structure to avoid unnecessary anchor churn:

1. Design principles
2. Deferred, open and outside-scope classification
3. Excluded language families

Treat the three unsuffixed Advanced files together as one exhaustive authority.

## 2.1 Design principles Advanced

Update:

```text
docs/src/docs/design-scope/design-principles.mtf
```

Absorb the useful non-duplicated material from the codebase overview:

- central design bias
- preferred constrained mechanisms
- design-review questions
- distinction between compiler complexity and source complexity
- static resolution and local inspectability
- backend optimisation hidden behind stable semantics

Retain the preferred-mechanisms table.

Add the design-review questions currently held only by the codebase summary, including:

- Does this introduce a new source category when an existing mechanism is enough?
- Does it make behaviour more implicit?
- Does it need whole-program or solver-heavy reasoning?
- Does it introduce erased runtime machinery?
- Does it leak backend representation?
- Does it expose optimisation mechanics?
- Is it accepted but unimplemented, undecided or intentionally excluded?
- Does it make code harder to inspect locally?

## 2.2 Classification Advanced

Update:

```text
docs/src/docs/design-scope/deferred-and-outside-scope.mtf
```

Retitle its visible section to:

```text
Deferred, open and outside scope
```

Define three categories.

### Accepted deferred implementation

- The semantic or architectural contract is accepted.
- Implementation is absent, partial or target-limited.
- It may appear in the progress matrix.
- It may have a structured deferred-feature diagnostic.
- Its roadmap plan owns sequencing, not semantics.

Examples may include:

- declared memory groups
- nested choice payload patterns
- recursive choice types
- accepted generic external package support
- accepted package-manager foundations
- field and path reactive subscriptions

Verify each example is genuinely accepted before retaining it.

### Open design question

- No final contract has been accepted.
- The feature may or may not enter the language.
- It is not a promise.
- It must not appear in the progress matrix.
- A roadmap document or discussion does not make it accepted design.
- The compiler need not provide a "coming later" diagnostic.

Include these current open questions:

- distinct primitive-backed nominal wrapper or newtype syntax
- narrow references to named, monomorphic, non-capturing functions

State explicitly that transparent aliases remain transparent regardless of the first question.

### Outside scope

- The mechanism conflicts with the intended language model.
- It requires an explicit design-philosophy change before implementation.
- It is documented by the exhaustive excluded-family list.
- It does not appear in the progress matrix.

Retain the structured diagnostic distinction between deferred-feature and outside-design-scope failures.

Add an ownership statement:

> The progress matrix tracks implementation of accepted design. It intentionally omits open design questions and outside-scope features.

## 2.3 Excluded families Advanced

Update:

```text
docs/src/docs/design-scope/excluded-language-families.mtf
```

Make this the complete exact exclusion inventory.

Audit it against:

- the existing public file
- the codebase Design Scope overview
- the deleted monolith at `1548b09be`
- every topic-specific `Outside scope` section
- current accepted decisions in canonical language files

At minimum preserve exact coverage for:

- macros and broad metaprogramming
- general closures, anonymous callable values, generic function values and higher-order polymorphism
- dynamic trait values and trait objects
- trait inheritance, aliases, composition, defaults and associated items
- generic traits and generic trait methods
- blanket, conditional, negative, specialised and structural conformance
- type-set and underlying-type constraint systems
- operator overloading
- cross-file, builtin, imported and foreign receiver extensions
- user-extensible builtin map hashing and key semantics
- first-class public `Result` values and result pattern matching
- exceptions and catchable panic systems
- reflection, runtime type IDs and type-level inspection
- higher-kinded types, type functions, partial application and parameterized aliases
- user const generics beyond fixed collection capacity
- source-visible lifetime, reference-category and ownership annotations
- source-visible RC, weak ownership and finalizers
- backend-specific observable source semantics

Phrase function values narrowly:

> General closures, anonymous function values, generic function values and higher-order polymorphism are outside scope.

Do not claim that every possible named monomorphic function-reference design is permanently excluded.

Do not put nominal primitive wrappers in this list. They remain an open question.

When a topic-specific canonical page has an outside-scope rule missing from this inventory, add it here without deleting the useful local topic explanation.

If two current authorities conflict about whether a feature is deferred, open or outside scope, do not choose silently. Record the conflict in the final report.

## 2.4 Basic Design Scope

Update the three Basic files as a coordinated teaching layer.

### `design-principles-basic.mtf`

Keep:

- one clear mechanism over many forms
- explicit behaviour over hidden magic
- small surface area
- static resolution
- compiler optimisation behind the scenes

Use short examples showing Moth's chosen mechanisms.

### `deferred-and-outside-scope-basic.mtf`

Replace the current two-category explanation with three simple categories:

- **Accepted but not built yet**
- **Still being considered**
- **Not part of Moth's design**

Avoid "coming later" for anything that is merely undecided.

Use nominal wrappers as the primary undecided example.

### `excluded-language-families-basic.mtf`

Keep only the most important user-facing exclusions and rationale:

- no general macros
- no general closures or higher-order function values
- no dynamic trait values
- no operator overloading
- no reflection
- no explicit lifetime and ownership type system

Rename "No closures or function values" to something precise such as:

```text
No general closures or higher-order function values
```

Basic must not become an exhaustive catalogue.

## 2.5 Route introduction

Update:

```text
docs/src/docs/design-scope/@page.moth
```

Its introduction must acknowledge all three categories:

- accepted deferred implementation
- open design questions
- outside scope

Keep Basic selected by default.

Preserve existing public anchors where practical.

---

# Phase 3: remove the codebase Design Scope owner

Delete:

```text
docs/src/docs/codebase/design-scope/@page.moth
docs/src/docs/codebase/design-scope/overview.mtf
```

Do not leave a redirect, summary copy or compatibility route.

Before deleting, confirm every unique useful rule has moved into the public Advanced files.

## 3.1 Update authority references

Replace references to:

```text
docs/src/docs/codebase/design-scope/overview.mtf
/docs/codebase/design-scope/
```

with the public Design Scope authority.

Review and update at minimum:

```text
AGENTS.md
CONTRIBUTING.md
docs/compiler-design-overview.md
docs/build-system-design.md
docs/src/docs/@page.moth
docs/src/docs/codebase/@page.moth
docs/src/docs/codebase/overview.mtf
docs/src/docs/codebase/language/@page.moth
docs/src/docs/codebase/language/overview.mtf
docs/src/docs/reactivity/reactivity-scope.mtf
```

Then use `rg` to find every remaining reference.

## 3.2 Specific ownership corrections

### `AGENTS.md`

Add or tighten guidance equivalent to:

- public unsuffixed Design Scope files own accepted, open and excluded language boundaries
- open and outside-scope features must not be added to the progress matrix
- the progress matrix tracks implementation of accepted design only

### Compiler and build-system overviews

Their companion-authority lists must point to:

```text
docs/src/docs/design-scope/
```

or the precise public Advanced files.

Do not otherwise rewrite these architecture documents.

### Codebase overview

It must no longer say that `docs/src/docs/codebase/**` owns Design Scope.

A short "design in one minute" summary may remain because it serves a different audience. It must link to the public Design Scope route as the authority and must not reproduce the exact exclusion inventory.

### Codebase route

Remove the internal Design Scope entry or point it directly to:

```text
../design-scope
```

Do not retain a `/docs/codebase/design-scope/` route.

### Compiler-facing language index

Update its Design Scope section to name the public files as the sole authority.

Remove wording claiming a codebase summary remains.

### Root docs page

Remove the duplicate codebase Design Scope entry. Keep the public Design Scope link under the language/project documentation.

### Reactivity

Point its rationale link to the public Design Scope route.

Keep its local concise distinction between deferred reactivity and excluded general closure machinery.

### `CONTRIBUTING.md`

Point contributors to the public Design Scope source or route.

---

# Phase 4: narrow the progress matrix

Update:

```text
docs/src/docs/progress/@page.moth
```

## 4.1 Introductory policy

The introduction must say:

- the matrix tracks current implementation and target status for accepted design
- accepted deferred surfaces may appear
- open design questions do not appear
- outside-scope features do not appear
- permanent language exclusions live in Design Scope
- invalid-form diagnostic coverage may be mentioned inside the relevant intended feature row

Remove the current claim that the matrix tracks outside-scope surfaces.

## 4.2 Status legend

Retain:

```text
Supported
Partial
Experimental
Deferred
```

Remove:

```text
Rejected
Outside Scope
```

Do not replace them with new synonyms.

## 4.3 Section structure

Rename:

```text
Deferred, rejected, and outside-scope surfaces
```

to something such as:

```text
Accepted deferred and incomplete surfaces
```

Delete standalone rows that exist only to describe non-features.

Prefer removing or folding rows over adding replacements.

## 4.4 Required row corrections

### Results, options and multiple returns

The current row is `Partial` only because it lists first-class `Result` as deferred. First-class public `Result` values are outside scope, so they must disappear from the matrix.

Set this row to `Supported` unless a real accepted implementation gap is found.

Do not invent a missing feature to justify `Partial`.

### Type aliases

Change `Supported` to `Partial`.

Describe:

- transparent aliases to builtins and source types are supported
- collections, maps, options, imported source types and concrete generic instances are supported
- the current compiler still rejects aliases to external opaque types despite the accepted transparent-alias contract

Remove all mention of parameterized aliases from this row.

Do not mention nominal wrapper types.

### Hash maps

Keep only actual intended implementation gaps, primarily target support such as non-JS lowering.

Remove lists of permanently excluded key families, hashsets, custom hashing, map equality or other outside-scope mechanisms.

### Traits

Keep only accepted deferred trait work:

- static non-method requirements
- additional compiler-owned builtin conformance families
- broader standard trait taxonomy that remains static

Remove default methods, associated items, inheritance, composition, generic traits, dynamic traits, specialisation and similar outside-scope items from the progress notes.

Delete or fold the separate "Trait ecosystem extensions" row.

### Choices

Keep only accepted implementation gaps such as:

- nested payload patterns
- recursive choices
- accepted payload field-access or narrowing work
- accepted default surfaces, only where a canonical reference actually accepts them

Remove ambiguous wording such as "deferred or rejected".

### Functions and calls

Compress the current implementation-heavy note.

Keep:

- named/defaulted calls status
- source-authored return aliases removed
- ordinary return preserves allocation identity
- explicit `copy` creates independence
- a caller receives a separate result binding
- current multi-return alias analysis may conservatively over-approximate when that affects diagnostics

Remove:

- "raw-value ABI"
- exact internal result-slot state
- side-table implementation detail
- exhaustive test inventory from the notes column

### Moth Template content assets

Keep observable collision semantics.

Remove the internal visible-name-registry implementation description.

### Templates and style directives

Remove full CommonMark and other intentional non-goals from progress notes.

Keep current template support and accepted deferred implementation work.

### Generic rows

Delete the standalone rows for:

- explicit generic call-site application
- parameterized generic aliases
- type values and type-level compile-time evaluation
- general function-value surfaces

Fold the inference-only current rule into `Generic functions` or `Generic type infrastructure`.

Remove "file-local evidence-backed dispatch remains outside scope" from the trait-bounds status note.

### Standalone rejected rows

Delete or fold every row whose only purpose is permanent rejection, including at minimum:

- Error helper methods
- labeled scopes
- legacy and foreign-language syntax
- public panic/recover syntax

Search for any additional rows using `Rejected` and apply the same rule.

### Async and checked scopes

The current notes call these design-only or not designed. They are not accepted deferred implementation merely because tokens are reserved.

Remove them from the progress matrix unless an authoritative current document establishes an accepted source contract.

If still useful, describe them as open questions in Design Scope or leave them to their design draft and roadmap.

### Assertion extensions

Do not call these deferred:

- debug-only assertions
- catchable panic semantics

They conflict with the accepted always-checked, unrecoverable assertion contract.

Only retain genuinely accepted assertion additions, such as broader message expressions, if a canonical reference explicitly accepts them.

### Rich numeric work

Align the row with the accepted Number/Byte plan.

Do not list generic BigInt, Decimal or a broad numeric tower as deferred unless those exact surfaces are accepted.

### External JavaScript expansion

Audit each listed extension against the canonical external binding contract.

Keep only accepted future surfaces. Remove mechanisms that are excluded or undecided.

## 4.5 Matrix maintenance rule

End the page with an explicit rule:

> Keep this page focused on support, partial implementation, target gating, experimental work and accepted deferred implementation. Put open design questions and outside-scope features in Design Scope.

## 4.6 Search checks

After editing, these must return no status rows:

```sh
rg -n '\[: Rejected\]|\[: Outside Scope\]' docs/src/docs/progress/@page.moth
```

These concepts must not have standalone progress rows:

```sh
rg -n \
  'Parameterized generic aliases|Type values and type-level|General function-value surfaces|First-class public Result' \
  docs/src/docs/progress/@page.moth
```

A single link from the matrix to Design Scope is valid.

---

# Phase 5: clarify aliases and function values

## 5.1 Type aliases Advanced

Update:

```text
docs/src/docs/aliases/type-aliases.mtf
```

Preserve every transparent-alias rule.

Add a short section such as:

```text
### Alias versus nominal domain type
```

State:

- aliases improve spelling and readability
- aliases do not prevent mixing with the target type
- `UserId as Int` is not domain isolation
- use a wrapper struct today when distinct nominal identity matters
- Moth has no accepted primitive-backed nominal wrapper syntax
- such a wrapper remains an open design question
- a future wrapper design would be separate from `as`

Retain external package types in the accepted target list.

Add a short current-status note pointing to the progress matrix rather than pretending the current compiler already supports external opaque aliases.

## 5.2 Type aliases Basic

Update:

```text
docs/src/docs/aliases/type-aliases-basic.mtf
```

Make the limitation explicit:

> `UserId as Int` makes annotations clearer, but it does not stop an ordinary `Int` from being used as `UserId`.

Keep the struct comparison as the current nominal alternative.

Do not make Basic discuss `TypeId` or future syntax.

## 5.3 Function declarations Advanced

Update:

```text
docs/src/docs/functions/function-declarations.mtf
```

Replace broad wording that could permanently exclude every named callable reference.

Use:

> Source functions are named declarations and cannot currently be used as ordinary values. General closures, anonymous function values, generic function values and higher-order polymorphism are outside scope. A narrower future design for references to named, monomorphic, non-capturing functions remains an open design question and has no current syntax or implementation commitment.

## 5.4 Generic and reactivity wording

Review:

```text
docs/src/docs/generics/generic-limits.mtf
docs/src/docs/reactivity/reactivity-scope.mtf
docs/src/docs/design-scope/excluded-language-families-basic.mtf
docs/src/docs/design-scope/excluded-language-families.mtf
```

Use consistent distinctions:

- generic function values remain outside scope
- general closures and higher-order polymorphism remain outside scope
- narrow named monomorphic references remain undecided
- reactivity is not general function-value support
- no progress row is created for the undecided narrow design

---

# Phase 6: audit Advanced implementation detail

Perform a bounded pass over canonical unsuffixed user-facing references.

Do not perform another broad writing-style rewrite.

## Decision test

For each technical paragraph, ask:

1. Does it change what source is legal?
2. Does it explain observable runtime behaviour?
3. Does it explain a diagnostic users may encounter?
4. Does it help Advanced users form an accurate mental model?
5. Is it current-status information better owned by the progress matrix?
6. Is it an exact internal mechanism better owned by compiler or memory architecture?

Keep paragraphs that satisfy the first four.

Compress or relocate paragraphs that satisfy only the last two.

## Required focused files

### Function returns

In:

```text
docs/src/docs/functions/returns-and-multiple-values.mtf
```

Keep:

- fresh caller binding versus allocation identity
- ordinary return versus explicit copy
- inferred alias effects
- fallible carrier versus payload aliasing
- multiple-return alias relationships

Compress:

- ABI terminology
- exact summary-lattice implementation
- internal side-table selection

A concise note that current analysis can conservatively over-approximate multi-return aliases may remain because users can observe the resulting diagnostic conservatism.

### Lifetimes and result shapes

In:

```text
docs/src/docs/memory/lifetimes-and-result-shapes.mtf
```

Keep the semantic distinction between:

- binding slot
- allocation
- alias
- fresh root
- independent graph
- retained edge
- result-to-result aliasing

Replace internal wording such as "the borrow state therefore tracks..." with direct semantic wording.

Keep one concise statement about current conservative multi-return precision if it can affect accepted programs.

Formal analysis detail belongs in the codebase memory-management references.

### Strings

In:

```text
docs/src/docs/language-overview/strings-and-characters.mtf
```

Keep the backend-neutral rules:

- one semantic `String`
- content equality
- numeric-only `+`
- template concatenation
- quoted slices versus owned template construction
- no current mutation API

Compress repeated AST, HTML-JS and HTML-Wasm implementation detail to one short target-status note and a progress link.

### Moth Template scope

In:

```text
docs/src/docs/moth-templates/template-scope.mtf
```

Keep:

- both visible constant sources
- no shadowing
- collision diagnostic
- both source locations where available
- unique names remain visible

Remove the duplicated paragraph describing the internal "one visible-name registry".

### Progress matrix

Apply the same principle to the Functions and Moth Template rows.

## Scope guard

Do not alter detailed compiler or build-system architecture except for authority links.

Do not remove technical explanation from other Advanced pages without documenting why it was redundant or misplaced.

---

# Phase 7: example-name cleanup

This phase is intentionally low effort.

## 7.1 Primary replacements

Search documentation source for generic `Sam` examples:

```sh
rg -n '\bSam\b' README.md CONTRIBUTING.md docs packages
```

Replace generic `Sam` examples with `Rob` where the change is isolated and semantically neutral.

Preferred example sequence:

```text
Priya
Rob
Emmy
Huw
```

Examples with two people should usually use `Priya` and `Rob`.

Examples with three or four people should add `Emmy` then `Huw`.

## 7.2 Opportunistic cleanup

While editing an affected example, other generic placeholder names may be replaced with the preferred names when this improves consistency.

Do not turn this into a separate repository-wide churn pass.

Old `Linus` examples may remain.

Do not replace:

- real people
- author names
- project names
- historical examples
- package/module identifiers
- diagnostic payloads whose exact text is tested
- fixture names
- snapshot text
- quoted external material
- names with domain meaning

## 7.3 Generated and tested examples

When changing docs source:

- rebuild generated docs normally
- never edit generated HTML directly

If a documentation example is also compiled or snapshot-tested, update its corresponding expectation only when required and keep the task documentation-only. If changing it would require modifying a non-documentation fixture or test, leave the name unchanged and report it.

## 7.4 Search closeout

Report:

- number of `Sam` matches found
- number replaced with `Rob`
- number intentionally retained
- any other names changed opportunistically
- whether any `Linus` references were changed

No target count is required. Correctness and low churn matter more than complete uniformity.

---

# Phase 8: generated documentation and validation

This is documentation-only work. Follow the documentation-only gate.

During iteration, this may be used:

```sh
moth check docs --terse
```

or:

```sh
cargo run --quiet -- check docs --terse
```

Required final gate:

```sh
moth build docs --release
```

or, when no suitable binary is available:

```sh
cargo run --quiet -- build docs --release
```

Also run:

```sh
git diff --check
```

Do not run `just validate` for this strictly documentation-only slice.

## Generated output checks

Verify:

- generated HTML came from the docs build
- no generated file was edited manually
- `/docs/design-scope/` builds successfully
- `/docs/codebase/design-scope/` no longer exists
- stale generated output for that deleted route was removed
- all local links resolve
- all fragments resolve
- no source or generated page links to the deleted route
- tables render correctly
- Basic remains selected by default
- Advanced contains the complete exact classification and exclusion inventory
- there is one H1 per changed route
- example-name changes appear correctly in generated code blocks and prose

Run the existing generated link-and-fragment audit. It must report:

```text
0 broken local links
0 missing fragments
```

## Routes to inspect manually

Inspect at minimum:

```text
/docs/
/docs/design-scope/
/docs/codebase/
/docs/codebase/language/
/docs/progress/
/docs/aliases/
/docs/functions/
/docs/generics/
/docs/traits/
/docs/reactivity/
/docs/collections/
/docs/memory/
/docs/moth-templates/
```

Also inspect any route containing changed example names.

---

# Completion criteria

This cleanup is complete when:

- transparent aliases remain unchanged
- nominal wrappers are documented only as an open design question
- broad function-value exclusions are precise
- narrow named monomorphic references remain an open question
- the public Design Scope route is the sole authority
- the codebase Design Scope source and generated route are gone
- all links point to the public authority
- the public Advanced Design Scope files contain the complete inventory
- Basic Design Scope remains concise and user-focused
- accepted deferred, open and outside-scope categories are clearly distinct
- the progress matrix has no `Rejected` or `Outside Scope` status
- the progress matrix contains no standalone outside-scope rows
- first-class public `Result` is absent from progress tracking
- parameterized aliases are absent from progress tracking
- type values and broad function values are absent from progress tracking
- the Results row reflects only intended implementation status
- the Type Aliases row records the external opaque alias implementation gap
- Traits, maps, choices and other partial rows mention only accepted missing work
- user-facing Advanced pages retain full normative semantics
- internal implementation detail has been compressed only where it adds no user value
- generic example names prefer `Priya`, `Rob`, `Emmy`, then `Huw`
- straightforward `Sam` examples have been replaced with `Rob`
- no broad or risky name churn was introduced
- generated docs build successfully
- local link and fragment audits report zero failures
- the changed-file list contains documentation only

---

# Required completion report

## Repository state

- starting commit
- final commit
- branch
- initial worktree state
- final worktree state

## Design Scope

- content merged into public Advanced files
- Basic changes
- open-design category added
- codebase files deleted
- generated route removed
- authority references changed

## Progress matrix

List:

- statuses removed
- rows removed
- rows changed from Partial to Supported
- rows changed from Supported to Partial
- outside-scope wording removed
- accepted deferred rows retained
- any classification conflict found

## Topic clarifications

Report changes to:

- transparent aliases
- nominal wrapper wording
- external opaque alias status
- named-function-reference wording
- reactivity and generic function-value scope

## Technical-detail cleanup

For each changed Advanced page, state:

- detail retained
- detail compressed
- formal owner used instead
- why no semantic information was lost

## Example names

Report:

- `Sam` matches found
- `Sam` examples changed to `Rob`
- intentional retained `Sam` references
- opportunistic changes to `Priya`, `Rob`, `Emmy` or `Huw`
- any changed `Linus` references
- any name change skipped because it would require non-documentation fixture or test churn

## Validation

Report exact results for:

```text
documentation release build
git diff --check
generated link audit
generated fragment audit
```

## Routes inspected

List every inspected route.

## Remaining uncertainty

Report every unresolved:

- deferred versus open classification conflict
- outside-scope inventory disagreement
- broken or stale link
- generated stale route
- implementation gap not represented in progress
- Advanced rule that could not be safely compressed
- example name whose replacement would require non-documentation changes

Do not claim completion while the progress matrix still tracks non-features or the codebase Design Scope route still exists.
