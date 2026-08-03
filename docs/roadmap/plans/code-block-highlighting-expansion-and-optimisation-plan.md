# Moth code-block highlighting expansion and optimisation plan

## Purpose

Expand and optimise the built-in HTML builder `$code` formatter, with the main emphasis on `$code("moth")`.

The finished highlighter should make Moth documentation examples look deliberate and readable while keeping compilation fast, generated HTML compact and ownership narrow. It remains a lightweight lexical formatter. It must not become a second compiler tokenizer, parser, semantic analyser or copy of the VS Code TextMate grammar.

This plan is designed for implementation in accepted, reviewable phases. Each phase is a coherent coding-agent-sized slice and ends with its own audit, style-guide review and validation gate.

## Active context capsule

ACTIVE_PLAN:
- `docs/roadmap/plans/code-block-highlighting-expansion-and-optimisation-plan.md`

CURRENT_SLICE:
- Phase: 0
- Checklist item: Refresh the baseline, install the plan and add the performance workload
- Goal: establish current repository state and reproducible correctness/performance evidence before changing the highlighter
- Non-goals: no production highlighter changes in this slice

LAST_GOOD_COMMIT:
- `6d1c78cb21be52b993bac7b600855ee91fd135cf`

CURRENT_WORKTREE_STATE:
- Clean / known changes: local worktree state was not observable through the GitHub connector when this plan was written. Record it before editing.
- Branch: `main` at plan creation
- Dedicated worker worktrees: none known

RELEVANT_DOCS_THIS_SLICE:
- `AGENTS.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/language-overview.md`
- `docs/src/docs/codebase/language/overview.mtf`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `docs/src/docs/progress/@page.moth`
- `docs/roadmap/roadmap.md`
- `benchmarks/README.md`

RELEVANT_CODE:
- `src/compiler_frontend/keywords.rs`: canonical exact source-word-to-token mapping
- `src/compiler_frontend/tests/keyword_tests.rs`: existing keyword-policy test owner
- `src/compiler_frontend/numeric_text/grammar.rs`: non-allocating Moth numeric character predicates
- `src/compiler_frontend/builtins/error_type.rs::ERROR_TYPE_NAME`: canonical builtin `Error` spelling
- `src/compiler_frontend/external_packages/ids.rs::IO_NAMESPACE_NAME`: canonical prelude `io` spelling
- `src/projects/html_project/styles/code.rs`: sole `$code` language/profile, scanner, escaping and wrapper owner
- `src/projects/html_project/tests/code_tests.rs`: focused code-formatter/highlighter test owner
- `src/projects/html_project/moth-css-core.css`: shared code-token presentation palette
- `src/projects/html_project/document_shell.rs`: injects the core code palette into generated documents
- `src/projects/html_project/tests/document_shell_tests.rs`: CSS/document-shell assertion owner
- `tests/cases/manifest.toml`: canonical integration-case inventory
- `benchmarks/manifest.toml`: canonical benchmark workload and case inventory

ACCEPTANCE_CRITERIA:
- Moth exact source words come from one compiler-owned allocation-free classification match.
- `$code("moth")` highlights all current compiler-tokenised keywords, including reserved deferred words, with the agreed general palette.
- Planned words that are not current tokens, including `group` and `into`, are not highlighted as current keywords.
- The scanner is one byte-indexed pass over borrowed source slices.
- Production highlighting performs no full `Vec<char>` copy, no per-word `String` construction, no runtime hash lookup and no regex or parser invocation.
- Compound Moth operators are emitted as one span through maximal munch.
- Plain identifiers remain unwrapped.
- CSS role names are language-neutral and shared by every code profile.
- Focused unit tests, one HTML artifact integration case and a dedicated non-recording benchmark workload protect the result.
- The dedicated benchmark and broad docs benchmark show improvement or no measurable regression.
- User-facing docs, the existing progress-matrix row and the existing roadmap follow-up section accurately describe the implemented and deferred boundaries.
- `just validate` passes after every accepted code-bearing phase.

DECISIONS_ALREADY_MADE:
- decision: Compiler-tokenised deferred words use the normal keyword style.
  - reason: highlighting reserved syntax should stay visually simple and must not add a second deferred-status palette.
  - source/user/date: user interview, 2026-08-03
- decision: Compiler-owned source-word classification is shared from `src/compiler_frontend/keywords.rs`.
  - reason: the tokenizer and highlighter must not maintain separate current Moth word lists.
  - source/user/date: user accepted recommendation, 2026-08-03
- decision: Shared source-word lookup is allocation-free and uses a direct `match` or equivalent compiler-optimised dispatch.
  - reason: runtime `HashMap` setup and linear table scans are unnecessary for a small static vocabulary.
  - source/user/date: user accepted recommendation with explicit performance requirement, 2026-08-03
- decision: Add bounded contextual Moth highlighting for contracts, nominal names, functions, directives, import paths and `io`.
  - reason: these categories produce a large visual improvement without semantic analysis.
  - source/user/date: user accepted recommendation, 2026-08-03
- decision: Refactor the scanner to an allocation-conscious byte-slice single pass.
  - reason: optimisation and future-proofing are part of this plan, not a later cleanup.
  - source/user/date: user instruction, 2026-08-03
- decision: All code languages use one general semantic palette.
  - reason: CSS must describe reusable token roles, not language-specific categories.
  - source/user/date: user instruction, 2026-08-03
- decision: No separate deferred/reserved role is added.
  - reason: `async`, `yield`, `checked` and `block` should look like ordinary keywords.
  - source/user/date: user instruction, 2026-08-03
- decision: Stateful Moth template-body-aware highlighting is deliberately deferred.
  - reason: it requires nested lexical state and is separable from the high-value word, symbol and identifier expansion.
  - source/user/date: interview agreement, 2026-08-03

BLOCKERS / RISKS:
- The tracked generated docs contain many highlighted blocks, so class renames and added spans will produce a broad generated diff.
- Contextual heuristics can create false positives if they are not narrowly ordered and tested.
- ALL_CAPS source constants such as `PI`, `TAU` and `E` must not be coloured as contracts solely because of casing.
- More spans can increase generated HTML size even when scanning is faster.
- The benchmark system is deliberately rough and has no hard regression threshold. Use repeated non-recording runs and profiling when movement is visible.
- `main` may move before implementation. Phase 0 must refresh every path and assumption before editing.

VALIDATION_STATE:
- last command: none run by this plan artifact
- result: not started
- known unrelated failures: none recorded in this plan. Refresh from the current worktree.

DOCS_IMPACT:
- progress matrix needed: yes, update the existing `Templates and style directives` row only
- other docs stale: the HTML builder helper docs and roadmap code-block follow-up section need updates
- authorised docs updates: plan file, roadmap, progress matrix, relevant HTML helper docs and generated `docs/release/**` output produced by the release build

NEXT_ACTION:
- refresh `main`, record local worktree state, copy this plan into the repository and complete Phase 0

---

## Capsule maintenance rules

The active context capsule is the reload boundary for every agent.

After every accepted phase and before compaction:

- [ ] Update `CURRENT_SLICE` to the exact next checklist item.
- [ ] Replace `LAST_GOOD_COMMIT` with the accepted commit.
- [ ] Record branch, worktree changes and any dedicated worktrees.
- [ ] Narrow `RELEVANT_DOCS_THIS_SLICE` and `RELEVANT_CODE` to the next phase.
- [ ] Record the last validation command, result and any unrelated failures.
- [ ] Record pending generated-doc or matrix work under `DOCS_IMPACT`.
- [ ] Set `NEXT_ACTION` to one exact review or implementation step.
- [ ] Tick accepted checklist items.
- [ ] Compress completed implementation notes to one concise outcome line. Do not paste logs, diffs or long test output into the plan.
- [ ] Preserve all locked decisions, acceptance criteria and deferred boundaries.
- [ ] Commit the refreshed plan with the accepted slice.

Do not continue from compressed memory alone. Re-read `AGENTS.md`, this capsule and the relevant authorities after compaction.

---

## Current state at plan creation

### Repository anchor

```text
REPOSITORY: nyejames/moth
BRANCH: main
COMMIT: 6d1c78cb21be52b993bac7b600855ee91fd135cf
DATE_CHECKED: 2026-08-03
```

The implementing agent must refresh this anchor before changing files.

### Current `$code` implementation

`src/projects/html_project/styles/code.rs` currently:

- owns `CodeLanguage`, aliases, comment prefixes, formatter creation and HTML highlighting
- supports `Generic`, `Text`, `Moth`, `JavaScript`, `TypeScript`, `Python`, `Rust` and `Shell`
- receives body text after the shared template whitespace pass
- escapes HTML in plain-text mode
- preserves opaque formatter anchors
- wraps output in `<code class='codeblock'>...</code>`
- scans by collecting the complete source into `Vec<char>`
- accumulates each plain word into a mutable `String`
- allocates another escaped `String` when each word is flushed
- recognises comments, quoted strings, obvious number runs, brackets, single-character operators and a small static word list
- emits each character of a compound operator as a separate span
- uses punctuation incompletely as a word boundary
- gives every non-generic PascalCase word the `moth-code-struct` class
- maintains a local incomplete Moth keyword list which includes invalid current Moth word `in`

### Current Moth lexical authority

`src/compiler_frontend/keywords.rs` already owns the exact source spelling to `TokenKind` mapping. Its current exact classes include:

- ordinary language keywords and reserved words
- word operators
- value literals
- builtin and singleton type spellings

The highlighter does not consume that authority yet.

### Current test coverage

`src/projects/html_project/tests/code_tests.rs` currently covers:

- a generic syntax smoke test
- Moth comment and one keyword
- JavaScript, Python and TypeScript examples
- end-of-file word flushing
- quoted strings
- HTML escaping
- plain-text escaping

It does not cover:

- complete Moth word groups
- compound Moth operators
- punctuation boundaries
- exponent numbers
- functions, directives, paths or contracts
- shared palette role names
- Unicode identifiers through the optimised scanner
- generated HTML artifact output

`src/compiler_frontend/tests/keyword_tests.rs` already owns keyword-policy tests and is the right place to extend compiler-owned word classification coverage.

### Current presentation palette

`src/projects/html_project/moth-css-core.css` currently defines:

- `comment`
- `keyword`
- `string`
- `number`
- `operator`
- `struct`
- `type`
- `parenthesis`

The class prefix `moth-code-` is a valid project namespace. The role suffixes `struct` and `parenthesis` are narrower than the actual cross-language semantics.

### Current roadmap, matrix and benchmark shape

`docs/roadmap/roadmap.md` already contains a `Code-block highlighting follow-ups` section. It owns future profile expansion and currently recommends TOML and JSON first.

`docs/src/docs/progress/@page.moth` already has a `Templates and style directives` row. A new highlighter row would duplicate that owner.

`benchmarks/README.md` currently records 32 workloads and 58 cases. `benchmarks/manifest.toml` already contains template and formatter-heavy stress workloads, but no workload isolates `$code` scanning.

---

## Locked design

### 1. One compiler-owned Moth source-word classification

Add a neutral classification beside `keyword_token_kind`.

Recommended shape:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceWordClass {
    Keyword,
    WordOperator,
    Literal,
    BuiltinType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClassifiedSourceWord {
    pub(crate) token_kind: TokenKind,
    pub(crate) class: SourceWordClass,
}

pub(crate) fn classify_source_word(text: &str) -> Option<ClassifiedSourceWord>
```

Exact names may be adjusted for local style, but the ownership and data shape are fixed:

- one direct string `match` constructs both `TokenKind` and neutral word class
- `keyword_token_kind` delegates to the shared classifier and returns only the token
- the HTML highlighter consumes only the neutral class
- HTML class names do not enter `keywords.rs`
- no runtime map, set, lazy initialiser, perfect-hash dependency or linear scan
- `attached_bang_keyword_token_kind` remains the authority for `return!` and `cast!`

### 2. Final general semantic palette

Every code language maps tokens into the same role vocabulary.

| Role | Final CSS class | Typical use |
|---|---|---|
| Comment | `moth-code-comment` | line comments |
| Keyword | `moth-code-keyword` | control, declaration and reserved words |
| Literal | `moth-code-literal` | booleans, null/none-like values |
| String | `moth-code-string` | strings, chars and path-like runs |
| Number | `moth-code-number` | numeric literals |
| Operator | `moth-code-operator` | symbolic and word operators |
| Type | `moth-code-type` | builtin scalar and singleton types |
| Nominal | `moth-code-nominal` | structs, choices, classes, enums and other named types |
| Delimiter | `moth-code-delimiter` | brackets and structural delimiters |
| Function | `moth-code-function` | function declarations, calls and method names |
| Directive | `moth-code-directive` | Moth template directives and future language annotations/pragmas |
| Contract | `moth-code-contract` | Moth traits, Rust traits and TypeScript interfaces |

Rules:

- keep the `moth-code-` namespace
- rename `moth-code-struct` to `moth-code-nominal`
- rename `moth-code-parenthesis` to `moth-code-delimiter`
- delete the old selectors and variables after every producer and test is migrated
- do not leave compatibility selectors
- namespace CSS variables as `--moth-code-*` rather than broad names such as `--keyword`
- preserve the old colours for renamed roles unless a deliberate palette adjustment improves contrast
- select distinct new colours for function, literal, directive and contract roles
- ensure every colour remains readable on the current dark code-block background
- do not create any Moth-only role class

Plain source text has no span.

### 3. Exact Moth word mapping

#### Keyword role

```text
import
export
if
return
catch
then
else
block
checked
cast
as
type
of
must
this
This
async
yield
loop
to
by
break
continue
copy
assert
```

Attached forms:

```text
return!
cast!
```

The attached `!` is part of the same keyword span when source adjacency matches the compiler helper.

#### Operator role

```text
is
not
and
or
```

#### Literal role

```text
true
false
none
```

#### Type role

```text
Int
Float
Bool
String
Char
None
True
False
Error
```

`Error` must use `ERROR_TYPE_NAME` rather than a duplicated string literal where the dependency boundary remains clean.

#### Explicit exclusions

These are not current exact source keywords and must not be coloured as such:

```text
in
fn
group
into
where
```

`group` and `into` remain accepted deferred design. Highlighting must not make them look implemented.

### 4. Exact Moth symbolic coverage

Use maximal munch. Check longer forms before their prefixes.

#### Compound assignment and declaration forms

```text
//=
+=
-=
*=
/=
%=
^=
#=
~=
$=
```

`$=` is a visual compound form even though the compiler may represent it with existing token pieces.

#### Structural and control forms

```text
=>
->
::
..
>>
<<
```

#### Arithmetic and comparison forms

```text
//
<=
>=
=
+
-
*
/
%
^
<
>
```

#### Access, effect and source markers

```text
~
#
$
!
?
&
@
```

Do not invent invalid compound operators:

- `==` is not one Moth operator
- `!=` is not one Moth operator
- `&&` is not one Moth operator
- `||` is a pair of function/parameter delimiters, not a logical operator

#### Delimiters and punctuation

Highlight as delimiters:

```text
:
;
|
(
)
[
]
{
}
```

Treat comma and dot as lexical boundaries. They may remain plain to avoid low-value span inflation.

### 5. Bounded Moth contextual rules

These are lexical presentation heuristics, not semantic facts.

#### Nominal names

- classify non-builtin PascalCase identifiers as `Nominal`
- cover structs, choices, variants, aliases, generic parameters and external opaque types
- apply builtin word classification before this heuristic
- apply contract classification before nominal fallback

#### Contracts

Do not colour every ALL_CAPS identifier as a contract.

Colour an ALL_CAPS identifier as `Contract` only when bounded lexical context supports it:

- it is followed by optional horizontal whitespace and `must`
- it is the next contract name after `must`
- it is the next contract name after generic-bound `is`
- it follows `and` inside an active contract list
- it follows `not` inside `must not` incompatibility syntax
- it follows a comma while the scanner remains inside a contract list

Reset the lightweight contract-list state at clear structural boundaries such as a new line, `:`, `;`, `|`, `=`, `->` or another declaration boundary.

This rule must keep ordinary ALL_CAPS constants such as `PI`, `TAU` and `E` out of the contract role unless contract context exists.

#### Functions

Classify a non-keyword ordinary identifier as `Function` when:

- the next non-horizontal-whitespace character is `(`, or
- the next non-horizontal-whitespace character is `|`

Priority rules:

- builtin words win first
- contracts win before functions
- PascalCase constructor/type names remain nominal even before `(`
- ordinary variables and fields remain plain
- method names such as `render` in `value.render()` use the function role

#### Directives

Classify `$` plus a lowercase or underscore-led identifier as one `Directive` span:

```text
$md
$code
$slot
$insert
$children
$html
$css
```

Do not use a fixed directive-name list. Builder registries can add directive names.

Do not classify these as directives:

```text
$Type
$=
$(
```

#### Import and resource paths

Classify a Moth `@`-prefixed path-like run as one String-role span:

```text
@core/math
@web/canvas
@vendor/drawing.js
@project
```

Rules:

- scan a tolerant path-like run, do not validate source
- stop at whitespace, group punctuation or another `@`
- do not turn invalid `@@name` into one valid-looking path token
- keep grouped import braces and aliases outside the path span
- reuse a narrow existing non-allocating path-character predicate if one already owns the required boundary
- do not call the path parser or allocate an interned path

#### Builtin namespace

Classify `IO_NAMESPACE_NAME` with the Type role only when it is used as a namespace prefix before `.`.

A user variable named `io` without namespace access remains plain.

### 6. Shared non-Moth palette adoption

This plan is Moth-focused. Do not expand every other language into a complete grammar.

Apply the common role vocabulary where existing or cheap lexical rules already support it:

- JavaScript and TypeScript boolean/null-like words use `Literal`
- Python `True`, `False` and `None` use `Literal`
- Rust `true` and `false` use `Literal`
- JavaScript/TypeScript `function name`, Python `def name`, Rust `fn name` and shell `function name` may use `Function`
- TypeScript `interface Name` and Rust `trait Name` may use `Contract`
- class/struct/enum-like names use `Nominal`
- separate the current TypeScript and Rust type-word branches so Rust no longer inherits TypeScript-only words
- do not perform a broad keyword inventory expansion for non-Moth profiles in this plan

The `Directive` role may have only Moth producers initially. Its meaning remains general enough for future annotations, pragmas and preprocessor directives.

### 7. Scanner performance contract

The production scan must:

- make one forward pass over `&str`
- track UTF-8 byte indexes
- use ASCII byte dispatch for ASCII punctuation, operators and keywords
- decode Unicode only while advancing through a non-ASCII string, char or identifier
- batch unhighlighted source ranges and escape them directly into the final output
- write highlighted ranges directly into the final output
- produce one owned output `String` per formatter text piece
- avoid a second `format!` allocation for the opening `<code>` wrapper
- preserve opaque formatter pieces without flattening
- use `CodeLanguage` and small enums, not trait objects
- use static class-name strings
- avoid regex, parser, tokenizer and `StringTable` work beyond resolving formatter input
- avoid wrapping ordinary identifiers or unstyled punctuation
- emit one span for one compound token

The following must be removed from the production path:

- full `source.chars().collect::<Vec<char>>()`
- mutable per-word `String`
- per-word escaped `String`
- per-character operator spans for recognised compound operators
- duplicated Moth current-word lists
- old `struct` and `parenthesis` role names
- broad unnamespaced CSS custom properties for the code palette

---

## Non-goals

Do not:

- use the Moth tokenizer to highlight code
- invoke AST parsing, type checking, name resolution or diagnostics
- report malformed snippet diagnostics
- add TextMate, regex, tree-sitter or another dependency
- copy the VS Code grammar into the compiler
- add semantic distinction between structs, choices, variants and aliases
- wrap every variable, field or punctuation character
- add stateful Moth template-body suppression in this plan
- add HTML, CSS or Markdown embedding inside a Moth snippet
- implement TOML, JSON, YAML or another new `CodeLanguage`
- redesign the style-directive registry
- change `$code` syntax, aliases, escaping or whitespace contracts
- change Moth language semantics
- update compiler architecture documents unless implementation reveals an actual ownership conflict
- edit `docs/release/**` by hand
- add a progress-matrix row for highlighting
- mark deferred language features supported because their reserved spelling is coloured
- modify the VS Code highlighting repository

---

## Test and evidence ownership

### Compiler lexical tests

Owner:

```text
src/compiler_frontend/tests/keyword_tests.rs
```

Protect:

- exact `SourceWordClass`
- exact `TokenKind`
- classifier/tokenizer agreement
- case sensitivity
- deliberate exclusions
- attached bang keyword authority

### Highlighter algorithm tests

Owner:

```text
src/projects/html_project/tests/code_tests.rs
```

Protect:

- role selection
- span boundaries
- escaping
- Unicode preservation
- maximal munch
- contextual Moth heuristics
- bounded non-Moth role reuse
- no span around plain identifiers

Prefer table-driven tests grouped by one contract. Do not create one function per keyword.

### CSS/document-shell tests

Owner:

```text
src/projects/html_project/tests/document_shell_tests.rs
```

Protect:

- every shared role selector is present
- selectors use the expected namespaced CSS variable
- old selectors are absent
- `.codeblock` whitespace and overflow rules remain intact

Do not assert exact colour values unless a colour itself becomes a deliberate stable contract.

### End-to-end artifact test

Create one primary canonical case:

```text
tests/cases/html_code_highlighting/
```

Suggested metadata:

```toml
[[case]]
id = "html_code_highlighting"
path = "html_code_highlighting"
tags = ["integration", "html", "templates", "formatting"]
contract = "builder.html.code_highlighter_lexical_roles"
role = "primary"
```

The HTML artifact must prove:

- Moth keywords, word operators, literals and builtin types
- `Error`
- reserved deferred words use keyword style
- planned non-token words remain plain
- compound operators use one span
- functions, methods, directives, paths, contracts and nominal names
- ALL_CAPS constants are not all misclassified as contracts
- plain variables remain unwrapped
- one non-Moth profile consumes the same function or contract role
- old class names are absent
- output order and escaping remain correct

Use narrow `artifact_assertions` with `must_contain`, `must_not_contain`, `must_contain_in_order` and exact-once checks where useful. Do not add a full-file golden.

Use only the HTML backend unless another target protects a distinct contract. This output is compile-time HTML presentation, so an acceptance-only HTML-Wasm duplicate is not justified.

### Benchmark evidence

Create one benchmark workload:

```text
benchmarks/code-highlighter-stress.moth
```

Add two non-quick cases:

```text
code_highlighter_stress_check
code_highlighter_stress_frontend
```

Use group `stress` and expectation `clean`.

The fixture should contain roughly 25-50 KiB of representative `$code("moth")` body text, including:

- complete current word groups
- comments, strings, chars and Unicode
- Moth numeric forms
- compound operators
- function declarations and calls
- traits and conformances
- choices and nominal types
- imports, grouped imports and directives
- enough plain identifiers to expose accidental span inflation

The fixture is performance evidence only. Correctness belongs in unit and integration tests.

Do not mark the cases `quick`. `bench-ci` may preflight them, but this task must not silently enlarge the measured quick subset.

---

## Phase sequence

| Phase | Coherent outcome |
|---|---|
| 0 | Plan installed, current baseline recorded and a dedicated highlighter benchmark workload exists |
| 1 | Compiler-owned source-word classification is canonical and the Moth highlighter no longer owns a duplicate current-word list |
| 2 | The production scanner is byte-slice based, allocation-conscious and uses correct token boundaries and maximal munch |
| 3 | The shared palette is language-neutral and Moth contextual highlighting is complete |
| 4 | End-to-end docs, matrix, roadmap and benchmark evidence are updated once |
| 5 | Final cross-phase audit, cleanup and plan closeout are complete |

Do not begin a phase until the previous phase has an accepted commit and refreshed capsule.

---

# Phase 0 - Baseline, plan activation and benchmark workload

## Context and reasoning

Optimisation work needs an authored workload before production changes. Adding it first gives the old and new scanner the same source fingerprint and prevents post-hoc benchmark design from favouring the new implementation.

This phase must not change `code.rs`, keyword policy or CSS.

## Checklist

### Refresh repository state

- [ ] Read `AGENTS.md` and every document listed in this phase's capsule.
- [ ] Fetch or pull current `main`.
- [ ] Record `git rev-parse HEAD`.
- [ ] Record `git status --short --branch`.
- [ ] Record active branch and any worker worktrees.
- [ ] Compare the new head with `6d1c78cb21be52b993bac7b600855ee91fd135cf`.
- [ ] Re-open every relevant code and docs path if `main` moved.
- [ ] Update the capsule before editing if any assumption changed.

### Install and activate the plan

- [ ] Add this plan at `docs/roadmap/plans/code-block-highlighting-expansion-and-optimisation-plan.md`.
- [ ] Add the plan link at the top of `Active implementation work` in `docs/roadmap/roadmap.md`.
- [ ] Do not reorder or clean up unrelated active plans.
- [ ] Add a short link from the existing `Code-block highlighting follow-ups` section to this active plan while it is in progress.
- [ ] Set `CURRENT_SLICE` to the benchmark checklist item.

### Add the benchmark workload

- [ ] Add `benchmarks/code-highlighter-stress.moth`.
- [ ] Keep the fixture clean, deterministic and free of warnings.
- [ ] Use `$code("moth")` so the actual HTML-builder formatter runs during frontend work.
- [ ] Use representative, readable Moth source-like text rather than random generated noise.
- [ ] Keep ordinary Moth code outside the balanced `$code` body minimal.
- [ ] Add workload `code_highlighter_stress` to `benchmarks/manifest.toml`.
- [ ] Add `code_highlighter_stress_check` as a non-quick CLI `check` case in group `stress`.
- [ ] Add `code_highlighter_stress_frontend` as a non-quick frontend `dev` case in group `stress`.
- [ ] Do not add a build case unless profiling proves output writing is needed to execute the formatter.
- [ ] Update `benchmarks/README.md` inventory counts from 32 workloads / 58 cases to 33 workloads / 60 cases.
- [ ] Do not update tracked benchmark summaries or local raw history.

### Capture baseline evidence

- [ ] Run `just bench-validate`.
- [ ] Run `just bench-frontend-check` and save concise output under `/tmp`.
- [ ] Run `just bench-check` and save concise output under `/tmp`.
- [ ] Repeat both non-recording suites five independent times if practical, following the benchmark optimisation protocol.
- [ ] Record the median/high-level result for the dedicated case and `docs_check` in the capsule.
- [ ] Do not commit raw output.
- [ ] Record the total byte size of generated HTML under `docs/release/**` for later comparison without changing the generated files.
- [ ] Record current `src/projects/html_project/styles/code.rs` line count for later complexity review.

## Phase 0 audit gate

- [ ] Confirm the benchmark fixture exercises the current production `$code` path.
- [ ] Confirm benchmark source is not being used as correctness coverage.
- [ ] Confirm workload and case IDs are stable authored identities.
- [ ] Confirm the new cases are not in the quick measured subset.
- [ ] Confirm no production highlighter file changed.
- [ ] Confirm roadmap edits are limited to plan activation.

## Phase 0 style-guide review

- [ ] Review fixture naming and comments for readability.
- [ ] Remove repetitive filler from the fixture while retaining sufficient input bytes.
- [ ] Confirm no temporary output or benchmark history is tracked.
- [ ] Run `git diff --check`.

## Phase 0 validation gate

- [ ] Run `cargo fmt --all` if any Rust file changed unexpectedly.
- [ ] Run `just bench-validate`.
- [ ] Run `cargo run --quiet -- check benchmarks/code-highlighter-stress.moth --terse`.
- [ ] Run `just validate`.
- [ ] Record exact command results in the capsule.
- [ ] Commit the accepted baseline slice.
- [ ] Refresh the capsule with the accepted commit and Phase 1 next action.

## Phase 0 acceptance

- [ ] Plan is present and active in the roadmap.
- [ ] One dedicated workload and two non-quick cases exist.
- [ ] Baseline performance and generated-size evidence is recorded locally.
- [ ] Repository is clean after the accepted commit.

---

# Phase 1 - Canonical compiler-owned source-word classification

## Context and reasoning

The current highlighter cannot be expanded safely while Moth words are duplicated inside `code.rs`. The compiler keyword module already owns exact source spelling. This phase makes it return neutral presentation categories without coupling the compiler to HTML.

The scanner remains structurally unchanged in this phase. The goal is one word authority, not the performance rewrite.

## Checklist

### Add the neutral classifier

- [ ] Add `SourceWordClass` or an equivalently clear neutral enum to `src/compiler_frontend/keywords.rs`.
- [ ] Include exactly `Keyword`, `WordOperator`, `Literal` and `BuiltinType`.
- [ ] Add `ClassifiedSourceWord` or an equivalent named result carrying `TokenKind` and `SourceWordClass`.
- [ ] Implement `classify_source_word` with one direct string `match`.
- [ ] Make `keyword_token_kind` delegate to the classifier.
- [ ] Keep `is_keyword` delegating through the current token function.
- [ ] Preserve `attached_bang_keyword_token_kind`.
- [ ] Do not create a parallel static list or a second lookup.
- [ ] Do not change `RESERVED_KEYWORD_SHADOWS` unless its existing contract is independently wrong.
- [ ] Update module/function comments to name the shared tokenizer/highlighter ownership.

### Apply exact classes

- [ ] Map all exact keyword spellings listed in the locked design to `Keyword`.
- [ ] Map `is`, `not`, `and` and `or` to `WordOperator`.
- [ ] Map `true`, `false` and `none` to `Literal`.
- [ ] Map `Int`, `Float`, `Bool`, `String`, `Char`, `None`, `True` and `False` to `BuiltinType`.
- [ ] Map `async`, `yield`, `checked` and `block` to ordinary `Keyword`.
- [ ] Keep `in`, `fn`, `group`, `into` and `where` unclassified.
- [ ] Verify every returned `TokenKind` remains identical to the old tokenizer mapping.

### Wire the Moth highlighter to the shared classifier

- [ ] Import the neutral classifier into `src/projects/html_project/styles/code.rs`.
- [ ] Remove the local Moth branch from `is_keyword`.
- [ ] Remove the local Moth branch from `is_type_keyword`.
- [ ] Map neutral classes through current presentation roles for this phase.
- [ ] Map `WordOperator` to the existing operator role.
- [ ] Preserve current literal visual treatment until the general palette phase.
- [ ] Use `attached_bang_keyword_token_kind` to keep `return!` and `cast!` together when possible without changing the scanner architecture yet.
- [ ] Remove invalid Moth keyword `in` from built-in output.
- [ ] Do not touch non-Moth vocabulary beyond compilation fixes required by the new API.

### Extend focused tests

- [ ] Expand `src/compiler_frontend/tests/keyword_tests.rs` with one table per neutral class.
- [ ] Cover every exact compiler-tokenised word at least once through grouped tables.
- [ ] Assert classifier `TokenKind` equals `keyword_token_kind`.
- [ ] Assert exact case sensitivity.
- [ ] Assert `in`, `fn`, `group`, `into` and `where` return `None`.
- [ ] Cover `return`/`cast` attached-bang authority.
- [ ] Expand `src/projects/html_project/tests/code_tests.rs` with grouped Moth word-role tests.
- [ ] Prove `async`, `yield`, `checked` and `block` use the ordinary keyword class.
- [ ] Prove word operators use the operator class.
- [ ] Prove `in`, `group` and `into` remain plain.

## Phase 1 audit gate

- [ ] Search for every current Moth word list in `code.rs` and adjacent formatter files.
- [ ] Confirm no duplicate current-word table remains.
- [ ] Confirm the compiler module contains no CSS names or HTML knowledge.
- [ ] Confirm tokenizer output did not change.
- [ ] Confirm the highlighter does not call `keyword_token_kind` and then perform a second string lookup.
- [ ] Confirm no runtime collection or dependency was added.

## Phase 1 style-guide review

- [ ] Use descriptive type and field names.
- [ ] Keep the direct match readable with grouped comments.
- [ ] Keep test tables explicit and concise.
- [ ] Avoid macros or generic table frameworks.
- [ ] Update stale comments that claim `code.rs` owns Moth keyword spellings.
- [ ] Run `cargo fmt --all`.
- [ ] Run `git diff --check`.

## Phase 1 validation gate

- [ ] Run `cargo test --workspace --quiet keyword_policy -- --format terse`.
- [ ] Run `cargo test --workspace --quiet code_highlighter -- --format terse`, or the narrowest actual test filter.
- [ ] Run the complete workspace unit tests.
- [ ] Run `cargo run --quiet -- check docs --terse`.
- [ ] Run `just validate`.
- [ ] Record exact results in the capsule.
- [ ] Commit the accepted lexical-authority slice.
- [ ] Refresh the capsule with the accepted commit and Phase 2 next action.

## Phase 1 acceptance

- [ ] One direct match owns current Moth source-word classification.
- [ ] The tokenizer and highlighter consume that same match.
- [ ] Current tokenizer semantics are unchanged.
- [ ] The highlighter no longer owns an independent Moth keyword/type list.

---

# Phase 2 - Byte-slice scanner, token boundaries and maximal munch

## Context and reasoning

With vocabulary drift removed, replace the allocation-heavy scanner. This phase establishes the long-lived scanner architecture and fixes low-level lexical boundaries. It does not add the full contextual palette yet.

The scanner remains tolerant. It identifies presentation runs and never decides whether source is valid Moth.

## Checklist

### Refactor formatter output construction

- [ ] Make `CodeTemplateFormatter::format` allocate one output `String` per text piece.
- [ ] Push the opening `<code class='codeblock'>` tag directly into the first text output.
- [ ] Scan or escape directly into that output.
- [ ] Preserve opaque pieces in their existing order.
- [ ] Append `</code>` to the last text output exactly once.
- [ ] Remove the `format!` wrapper allocation around an already-built highlighted string.
- [ ] Keep plain-text mode on the same direct escaping output path.

### Add the scanner state

- [ ] Introduce one small `CodeScanner`/`CodeHighlighter` state or equivalent local data owner in `code.rs`.
- [ ] Store borrowed source, `CodeLanguage`, current byte index and mutable output.
- [ ] Keep the scanner formatter-local.
- [ ] Do not create a reusable compiler lexer abstraction.
- [ ] Do not add trait objects.
- [ ] Keep `code.rs` as the single owner in this plan. Do not split modules unless the phase audit proves the file has become unreviewable.

### Scan borrowed slices

- [ ] Dispatch ASCII using `source.as_bytes()`.
- [ ] Decode UTF-8 only when a non-ASCII scalar must be advanced.
- [ ] Track plain-run start and append escaped source slices in batches.
- [ ] Add `push_escaped_text(output, source_slice)`.
- [ ] Add a narrow helper for opening/closing a role span using static class names.
- [ ] Avoid creating temporary escaped strings.
- [ ] Avoid copying identifiers into owned buffers.
- [ ] Preserve Unicode source exactly after HTML escaping.

### Implement lexical run scanning

- [ ] Detect line comments before operator scanning.
- [ ] Detect quoted strings and chars, preserving escape pairs.
- [ ] Tolerate unterminated strings by highlighting to end of input without diagnosing.
- [ ] Detect identifiers from borrowed byte ranges.
- [ ] Use punctuation as a boundary even when punctuation remains unstyled.
- [ ] Detect brackets and structural delimiters.
- [ ] Preserve all whitespace exactly after the existing pre-format whitespace pass.

### Improve numbers

- [ ] Add a Moth numeric scanner over borrowed bytes.
- [ ] Reuse `numeric_text::grammar` predicates where they do not force allocation.
- [ ] Recognise integer digits and legal underscore positions as one tolerant run.
- [ ] Recognise a decimal fraction.
- [ ] Recognise lowercase `e` and optional exponent sign.
- [ ] Cover `1`, `1_000`, `1.5`, `1e6`, `1e-6` and `1.0e+21`.
- [ ] Keep leading `-` as an operator in this lexical formatter.
- [ ] Do not validate range, separator placement or finiteness.
- [ ] Keep non-Moth numeric behaviour stable unless the shared scanner requires a clear bug fix.

### Add maximal-munch operators

- [ ] Add a language-profile operator-end helper using leading-byte dispatch.
- [ ] Match every locked Moth compound form before its prefix.
- [ ] Emit one operator span per compound form.
- [ ] Keep `==`, `!=`, `&&` and `||` from becoming invented Moth operators.
- [ ] Treat `::` and `..` as compound forms.
- [ ] Treat commas and dots as word boundaries.
- [ ] Keep unstyled punctuation as plain escaped output rather than adding low-value spans.

### Delete the old path

- [ ] Delete `Vec<char>` source collection.
- [ ] Delete the mutable `word: String`.
- [ ] Delete `flush_word`.
- [ ] Delete per-word `escape_html -> String` use from production.
- [ ] Delete character-index string/number scanners superseded by byte-range helpers.
- [ ] Delete the one-character-only operator emission path once maximal munch owns it.
- [ ] Remove unused imports such as `CharacterParsing` if no longer needed.
- [ ] Update file-level WHAT/WHY docs to describe the byte-slice scanner.

### Extend scanner tests

- [ ] Add exact-output tests for one-span compound operators.
- [ ] Add punctuation-boundary tests for `String,`, `Status::Ready,` and `io.line`.
- [ ] Add exponent-number tests.
- [ ] Add Unicode identifier, Unicode char and Unicode string tests.
- [ ] Add escaped HTML tests across plain, string and comment runs.
- [ ] Add EOF tests for identifier, number, comment and unterminated string paths.
- [ ] Assert plain identifiers remain unwrapped.
- [ ] Retain one regression example for every existing language profile.
- [ ] Avoid asserting internal byte indexes or helper names.

### Compare performance

- [ ] Run the dedicated frontend benchmark after the refactor.
- [ ] Run the dedicated CLI check benchmark after the refactor.
- [ ] Compare against the Phase 0 baseline.
- [ ] Inspect `docs_check` and existing `template_stress` movement.
- [ ] Run `just profile-case code_highlighter_stress_frontend` or the documented equivalent if a measurable regression appears.
- [ ] Do not add permanent timing instrumentation unless existing timing owners cannot attribute a real regression.
- [ ] Record concise conclusions in the capsule only.

## Phase 2 audit gate

- [ ] Inspect production code for hidden full-source copies.
- [ ] Confirm one output allocation per formatter text piece.
- [ ] Confirm plain runs are batched.
- [ ] Confirm no per-token heap allocation.
- [ ] Confirm operator matching is bounded by leading byte rather than a full linear list at every position.
- [ ] Confirm Unicode advancement cannot split UTF-8.
- [ ] Confirm malformed snippet text cannot panic.
- [ ] Confirm opaque pieces remain structurally preserved.
- [ ] Confirm old scanner helpers are deleted rather than wrapped.

## Phase 2 style-guide review

- [ ] Keep scanner control flow explicit and easy to profile.
- [ ] Use named helpers for string, number, identifier and operator end indexes.
- [ ] Avoid deeply nested iterator/combinator chains.
- [ ] Keep comments focused on UTF-8 boundaries, allocation policy and maximal-munch ordering.
- [ ] Keep function size under control. Extract only real scanner responsibilities.
- [ ] Run `cargo fmt --all`.
- [ ] Run `git diff --check`.

## Phase 2 validation gate

- [ ] Run focused code highlighter unit tests.
- [ ] Run complete workspace unit tests.
- [ ] Run `cargo run --quiet -- tests --case html_code_highlighting --backend html` only if the case already exists at this point.
- [ ] Run `cargo run --quiet -- check docs --terse`.
- [ ] Run `just bench-validate`.
- [ ] Run `just validate`.
- [ ] Record benchmark and validation results in the capsule.
- [ ] Commit the accepted scanner slice.
- [ ] Refresh the capsule with the accepted commit and Phase 3 next action.

## Phase 2 acceptance

- [ ] Production scanner is byte-indexed and single-pass.
- [ ] Full-source and per-word allocations are gone.
- [ ] Compound Moth operators use one span.
- [ ] Punctuation no longer corrupts word classification.
- [ ] Moth exponent numbers are one numeric run.
- [ ] Dedicated benchmark is not measurably slower.

---

# Phase 3 - General palette and complete Moth contextual highlighting

## Context and reasoning

The scanner can now support richer roles without extra passes or token buffers. This phase adds the final shared palette, Moth contextual heuristics and bounded role reuse in other languages.

This is the main user-visible implementation slice.

## Checklist

### Finalise role vocabulary

- [ ] Define one `CodeHighlightRole` or equivalently named enum in `code.rs`.
- [ ] Include every final role from the locked palette.
- [ ] Map each role to one static CSS class string.
- [ ] Make language profiles return roles rather than constructing class names.
- [ ] Keep Plain/no-role output span-free.
- [ ] Do not store CSS strings in compiler keyword policy.

### Generalise CSS

- [ ] Rename `.moth-code-struct` to `.moth-code-nominal`.
- [ ] Rename `.moth-code-parenthesis` to `.moth-code-delimiter`.
- [ ] Add `.moth-code-function`.
- [ ] Add `.moth-code-literal`.
- [ ] Add `.moth-code-directive`.
- [ ] Add `.moth-code-contract`.
- [ ] Rename every code palette variable to `--moth-code-*`.
- [ ] Keep renamed-role colours stable unless contrast review warrants adjustment.
- [ ] Choose distinct, readable colours for the four new roles.
- [ ] Verify contrast against the current code-block background in both site colour schemes.
- [ ] Delete old selectors and variables.
- [ ] Search the complete repository for old class names and broad old variables.

### Add complete Moth word roles

- [ ] Map `SourceWordClass::Keyword` to Keyword.
- [ ] Map `SourceWordClass::WordOperator` to Operator.
- [ ] Map `SourceWordClass::Literal` to Literal.
- [ ] Map `SourceWordClass::BuiltinType` to Type.
- [ ] Consume adjacent `!` for `return!` and `cast!`.
- [ ] Import and recognise `ERROR_TYPE_NAME` as Type.
- [ ] Do not add another current-word string table.

### Add Moth nominal and contract rules

- [ ] Add PascalCase nominal fallback.
- [ ] Add a cheap ALL_CAPS shape predicate with no allocation.
- [ ] Add a small Moth scan context for contract lists.
- [ ] Recognise `TRAIT must:` declarations.
- [ ] Recognise concrete `Type must TRAIT`.
- [ ] Recognise `type A is TRAIT and OTHER_TRAIT`.
- [ ] Recognise `TRAIT must not OTHER_TRAIT`.
- [ ] Recognise comma-separated conformance/contract lists where current syntax permits them.
- [ ] Reset context at explicit structural boundaries.
- [ ] Prove `PI`, `TAU` and `E` remain non-contract outside contract context.
- [ ] Keep `This` on the compiler-owned keyword path.

### Add function roles

- [ ] Highlight Moth function declarations before `|`.
- [ ] Highlight zero-parameter function declarations before `||`.
- [ ] Highlight function calls before `(`.
- [ ] Highlight method names before `(` after `.`.
- [ ] Apply compiler word, contract and nominal priority before function fallback.
- [ ] Keep PascalCase constructors nominal.
- [ ] Keep ordinary variables and fields plain.

### Add directives, paths and builtin namespace

- [ ] Highlight `$` plus lowercase/underscore identifier as Directive.
- [ ] Keep `$Type`, `$=` and `$(` out of the directive rule.
- [ ] Highlight complete `@` path-like runs with the String role.
- [ ] Stop path scanning before a second `@`.
- [ ] Keep grouped import braces and aliases separate.
- [ ] Import `IO_NAMESPACE_NAME`.
- [ ] Highlight `io` with Type only when followed by `.`.
- [ ] Keep a plain variable named `io` unwrapped.

### Apply bounded role reuse to other languages

- [ ] Map existing JavaScript/TypeScript null/boolean words to Literal.
- [ ] Map Python `True`, `False` and `None` to Literal.
- [ ] Map Rust `true` and `false` to Literal.
- [ ] Split TypeScript and Rust type-word logic.
- [ ] Remove TypeScript-only words from the Rust type branch.
- [ ] Add a small accurate Rust primitive-type set only if required to replace the incorrect shared branch.
- [ ] Highlight JavaScript/TypeScript function declaration names where cheap.
- [ ] Highlight Python `def` names where cheap.
- [ ] Highlight Rust `fn` names where cheap.
- [ ] Highlight shell `function` names where cheap.
- [ ] Highlight TypeScript interface names and Rust trait names with Contract.
- [ ] Do not expand broad non-Moth keyword inventories.

### Expand focused tests

- [ ] Add one table-driven test for every Moth compiler word class.
- [ ] Add one test for deferred compiler-tokenised words using Keyword.
- [ ] Add one negative test for planned non-token words.
- [ ] Add one maximal-munch table for every compound Moth form.
- [ ] Add function declaration/call/method tests.
- [ ] Add nominal constructor and choice-variant tests.
- [ ] Add trait declaration, bound, conformance and incompatibility tests.
- [ ] Add ALL_CAPS constant negative tests.
- [ ] Add directive and reactive-marker contrast tests.
- [ ] Add import-path and invalid-double-`@` boundary tests.
- [ ] Add `io` namespace and plain-`io` variable tests.
- [ ] Add one non-Moth Function-role test.
- [ ] Add one non-Moth Contract-role test.
- [ ] Add one non-Moth Literal-role test.
- [ ] Update old expected class names.
- [ ] Keep ordinary identifiers span-free in every relevant assertion.

### Add CSS/document-shell tests

- [ ] Add or extend a document-shell test that enumerates every final role selector.
- [ ] Assert each selector references its namespaced `--moth-code-*` variable.
- [ ] Assert old `struct` and `parenthesis` selectors are absent.
- [ ] Preserve the existing `.codeblock` overflow and whitespace assertions.

### Add the canonical integration case

- [ ] Add `tests/cases/html_code_highlighting/input/@page.moth`.
- [ ] Use a real `@html` codeblock wrapper and `$code("moth")`.
- [ ] Include all major Moth roles in one readable snippet.
- [ ] Include `async`, `yield`, `checked` and `block`.
- [ ] Include `group` and `into` as plain source-like identifiers without making the fixture invalid outside the balanced body.
- [ ] Include `PI` and one real trait declaration.
- [ ] Include `Error`, `io.line`, a method call, a function declaration, a directive and an import path.
- [ ] Include `#=`, `~=`, `$=`, `//=`, `::`, `=>`, `->` and `..`.
- [ ] Add one small Rust or TypeScript code block proving the shared Contract/Function palette.
- [ ] Add `expect.toml` with HTML artifact assertions.
- [ ] Add the case to `tests/cases/manifest.toml` as the primary owner.
- [ ] Assert new classes, ordering and exact-once compound forms.
- [ ] Assert old classes are absent.
- [ ] Do not add a golden.

### Check output size and performance

- [ ] Build a representative temporary docs release output or use the tracked release build locally for inspection.
- [ ] Compare generated HTML byte totals with Phase 0.
- [ ] Investigate any double-digit total HTML growth before acceptance.
- [ ] Confirm plain identifiers are not generating spans.
- [ ] Run the dedicated benchmark and compare with Phase 0 and Phase 2.
- [ ] Profile any visible regression.
- [ ] Record concise results in the capsule.

## Phase 3 audit gate

- [ ] Confirm CSS roles are language-neutral.
- [ ] Confirm no old class or variable remains in source.
- [ ] Confirm no Moth current-word list was reintroduced.
- [ ] Confirm contextual state is bounded and resets deterministically.
- [ ] Confirm no heuristic performs semantic lookup.
- [ ] Confirm all-caps constants remain protected.
- [ ] Confirm ordinary identifiers remain plain.
- [ ] Confirm added non-Moth logic is bounded to palette adoption.
- [ ] Confirm generated markup uses one span per meaningful token run.

## Phase 3 style-guide review

- [ ] Review role names for clarity.
- [ ] Keep profile branches grouped by language.
- [ ] Keep Moth context state as a small enum/struct rather than loose booleans.
- [ ] Avoid a broad configurable highlighter framework.
- [ ] Avoid macro-generated keyword tables.
- [ ] Keep comments focused on ordering and false-positive prevention.
- [ ] Run `cargo fmt --all`.
- [ ] Run `git diff --check`.

## Phase 3 validation gate

- [ ] Run focused keyword and code highlighter unit tests.
- [ ] Run document-shell CSS tests.
- [ ] Run `cargo run --quiet -- tests --case html_code_highlighting --backend html`.
- [ ] Run `cargo run --quiet -- tests --audit`.
- [ ] Run complete workspace unit tests.
- [ ] Run `cargo run --quiet -- check docs --terse`.
- [ ] Run `just bench-validate`.
- [ ] Run `just validate`.
- [ ] Record exact results in the capsule.
- [ ] Commit the accepted presentation slice.
- [ ] Refresh the capsule with the accepted commit and Phase 4 next action.

## Phase 3 acceptance

- [ ] Final general palette exists.
- [ ] Moth highlighter covers the agreed lexical and contextual surface.
- [ ] Other languages consume the same roles where cheap.
- [ ] End-to-end HTML artifact proves the visible contract.
- [ ] Performance and generated-size evidence remain acceptable.

---

# Phase 4 - Documentation, roadmap, matrix and final performance evidence

## Context and reasoning

Keep generated-document churn in one deliberate phase after production output stabilises. This phase documents the current lexical contract, records deliberately deferred work in its existing owner and produces the tracked documentation release once.

No new scanner feature belongs in this phase unless a docs build exposes a correctness bug.

## Checklist

### Update user-facing HTML builder docs

Primary expected owners:

```text
docs/src/docs/packages/builder/html/html-helpers.mtf
docs/src/docs/packages/builder/html/html-helpers-basic.mtf
```

- [ ] Add a concise `$code` section if no existing canonical section owns it.
- [ ] Document supported aliases:
  - [ ] `text` / `txt`
  - [ ] `moth`
  - [ ] `javascript` / `js`
  - [ ] `typescript` / `ts`
  - [ ] `python` / `py`
  - [ ] `rust` / `rs`
  - [ ] `bash` / `sh` / `shell`
- [ ] Show one `$code("moth")` example.
- [ ] State that code is HTML-escaped.
- [ ] State that highlighting is lexical presentation, not validation or semantic analysis.
- [ ] State that all profiles use one shared palette.
- [ ] Keep implementation details such as byte indexes and CSS enum names out of user docs.
- [ ] Review `docs/language-overview.md` and the current template-directive authority.
- [ ] Change the language overview only if its existing `$code` description became inaccurate.
- [ ] Do not add deferred feature prose to Basic teaching content unless users would otherwise misunderstand current output.

### Update the progress matrix

Update only the existing `Templates and style directives` row.

- [ ] Extend its coverage summary to mention focused `$code` formatter, escaping, lexical-role and HTML artifact coverage.
- [ ] Add a concise note that `$code("moth")` uses compiler-owned current word classification, lexical token roles and shared code presentation classes.
- [ ] State that it does not perform semantic symbol resolution.
- [ ] Mention stateful template-body-aware suppression as a deferred roadmap follow-up only if the note remains concise.
- [ ] Do not add a new row.
- [ ] Do not change any language feature status because a reserved word is highlighted.
- [ ] Do not mark `group`, `into`, async behaviour or another deferred feature supported.

### Update the roadmap

While work remains active:

- [ ] Keep the plan link under `Active implementation work`.
- [ ] Keep the existing `Code-block highlighting follow-ups` section as the single deferred owner.

After implementation is accepted:

- [ ] Remove the plan from `Active implementation work`.
- [ ] Add a concise linked completion item under `Completed`.
- [ ] Rewrite `Code-block highlighting follow-ups` to record the new baseline:
  - [ ] allocation-conscious single-pass scanner
  - [ ] compiler-owned Moth word classification
  - [ ] maximal-munch operators
  - [ ] general shared palette
  - [ ] bounded Moth lexical/contextual roles
- [ ] Retain the existing future-language profile order beginning with TOML and JSON.
- [ ] Add an explicit deferred bullet for stateful Moth template-body-aware highlighting.
- [ ] Add an explicit non-goal that full semantic/editor grammar parity remains owned by editor tooling, not the compile-time formatter.
- [ ] Keep semantic symbol resolution and syntax diagnostics out of the built-in formatter.
- [ ] Do not create a separate roadmap plan for template-body awareness unless future implementation work is actively selected.

### Regenerate tracked docs

- [ ] Run `cargo run --quiet -- build docs --release`.
- [ ] Do not edit generated HTML manually.
- [ ] Confirm generated output came from source and compiler changes.
- [ ] Inspect at least:
  - [ ] `docs/release/docs/traits/index.html`
  - [ ] `docs/release/docs/functions/index.html`
  - [ ] `docs/release/docs/casts/index.html`
  - [ ] `docs/release/docs/errors/index.html`
  - [ ] `docs/release/docs/loops/index.html`
  - [ ] `docs/release/docs/async/index.html`
  - [ ] the generated HTML builder helper route
  - [ ] `docs/release/docs/codebase/style-guide/index.html`
- [ ] Verify function, contract, directive, literal, nominal and delimiter colours are visually distinct.
- [ ] Verify Moth reserved deferred words use Keyword.
- [ ] Verify ordinary variables do not create excessive spans.
- [ ] Verify `PI`, `TAU` and `E` are not systematically coloured as contracts.
- [ ] Verify compound operators are one span in generated HTML.
- [ ] Search for `moth-code-struct` and `moth-code-parenthesis`. Expect zero source/generated matches.
- [ ] Search for every final class. Expect representative generated matches.
- [ ] Compare total HTML bytes against Phase 0.
- [ ] Record and explain the final size delta.
- [ ] Treat unexplained double-digit total growth as a blocker.

### Complete performance evidence

- [ ] Run `just bench-validate`.
- [ ] Run `just bench-frontend-check` five independent times and capture outputs under `/tmp`.
- [ ] Run `just bench-check` five independent times and capture outputs under `/tmp`.
- [ ] Compare medians for:
  - [ ] `code_highlighter_stress_frontend`
  - [ ] `code_highlighter_stress_check`
  - [ ] `docs_check`
  - [ ] `template_stress_check`
- [ ] Require improvement or no measurable regression for the dedicated case.
- [ ] Investigate any broad docs regression with targeted profiling.
- [ ] Do not record local benchmark history or update tracked monthly summaries.
- [ ] Add no performance claim stronger than the evidence supports.

### Refresh plan status

- [ ] Tick every accepted checklist item.
- [ ] Set capsule to Phase 5 final audit.
- [ ] Record the accepted implementation commits.
- [ ] Record docs release result, byte delta and benchmark conclusion.
- [ ] Record any known unrelated validation issue exactly.

## Phase 4 audit gate

- [ ] Confirm user docs describe observable behaviour rather than implementation internals.
- [ ] Confirm roadmap owns deferred work.
- [ ] Confirm matrix owns current support only.
- [ ] Confirm no deferred language feature status changed.
- [ ] Confirm generated docs were not manually edited.
- [ ] Confirm benchmark README counts match manifest inventory.
- [ ] Confirm broad generated diffs are explained by class and highlighting changes.

## Phase 4 style-guide review

- [ ] Apply British English.
- [ ] Remove filler, generic transitions and duplicate conclusions.
- [ ] Keep Basic docs concise.
- [ ] Keep Advanced docs precise.
- [ ] Keep roadmap bullets implementation-oriented and matrix notes status-oriented.
- [ ] Run `git diff --check`.

## Phase 4 validation gate

- [ ] Run `cargo fmt --all`.
- [ ] Run complete workspace unit tests.
- [ ] Run `cargo run --quiet -- tests --terse`.
- [ ] Run `cargo run --quiet -- tests --audit`.
- [ ] Run `cargo run --quiet -- check docs --terse`.
- [ ] Run `cargo run --quiet -- build docs --release`.
- [ ] Run `just bench-validate`.
- [ ] Run `just validate`.
- [ ] Record exact results in the capsule.
- [ ] Commit the accepted docs/status/performance slice.
- [ ] Refresh the capsule with the accepted commit and Phase 5 next action.

## Phase 4 acceptance

- [ ] User docs explain `$code`.
- [ ] Existing matrix row accurately reflects support and coverage.
- [ ] Existing roadmap section accurately owns deferred work.
- [ ] Generated docs are rebuilt and inspected.
- [ ] Final benchmark evidence is recorded without overstating precision.

---

# Phase 5 - Final audit and closeout

## Context and reasoning

The final audit is independent of the phase-level checks. Review the complete change from the Phase 0 base to current head. Look for architecture drift, duplicated data, excessive markup and subtle false positives that local phase reviews may miss.

Do not close the plan while any audit finding remains open.

## Checklist

### Full diff review

- [ ] Compare Phase 0 base commit with current head.
- [ ] Review every production, test, benchmark and docs file changed.
- [ ] Confirm the final file set is no broader than required.
- [ ] Review line-count movement in `code.rs`.
- [ ] Confirm complexity increased only where it buys an explicit role or performance property.
- [ ] Confirm no broad highlighter framework or configuration layer was introduced.
- [ ] Confirm no compatibility selectors, wrappers or old APIs remain.
- [ ] Confirm no unrelated cleanup was mixed in.

### Ownership audit

- [ ] Confirm `keywords.rs` owns exact compiler-tokenised Moth words.
- [ ] Confirm `code.rs` owns lexical presentation only.
- [ ] Confirm CSS owns visual presentation only.
- [ ] Confirm the style-directive registry still owns directive availability.
- [ ] Confirm the highlighter does not consult AST, HIR, imports, symbols or type environments.
- [ ] Confirm roadmap, matrix and user docs each stay within their documented ownership.

### Performance audit

- [ ] Search for `Vec<char>` in the highlighter.
- [ ] Search for per-word `String` construction.
- [ ] Search for runtime maps/sets or regex use.
- [ ] Inspect every hot-loop helper for hidden `.to_owned()`, `.collect()` or `format!`.
- [ ] Confirm class lookup uses static strings.
- [ ] Confirm plain runs are batched.
- [ ] Confirm compound tokens are not split into redundant spans.
- [ ] Confirm output size evidence is acceptable.
- [ ] Confirm dedicated benchmark evidence is acceptable.

### Correctness audit

- [ ] Cross-check exact compiler words against `classify_source_word`.
- [ ] Cross-check symbolic forms against current tokenizer tokens and language authority.
- [ ] Confirm invalid `in`, `==`, `!=`, `&&` and logical `||` were not added as Moth tokens.
- [ ] Confirm `group` and `into` remain deliberately unhighlighted as current keywords.
- [ ] Confirm `async`, `yield`, `checked` and `block` use Keyword.
- [ ] Confirm `Error` and `io` use canonical constants where practical.
- [ ] Confirm ALL_CAPS constant false positives are covered.
- [ ] Confirm Unicode and HTML escaping paths are safe.
- [ ] Confirm malformed snippet text cannot panic.

### Test-quality audit

- [ ] Confirm one primary integration contract owns user-visible HTML.
- [ ] Confirm unit tests protect algorithmic or classification invariants.
- [ ] Confirm no redundant fixture duplicates the same contract.
- [ ] Confirm no benchmark is cited as correctness evidence.
- [ ] Confirm artifact assertions are narrow and whitespace/class sensitive where required.
- [ ] Confirm test names describe behaviour rather than implementation helpers.

### Documentation audit

- [ ] Confirm the progress matrix has no new row.
- [ ] Confirm reserved highlighting did not change feature statuses.
- [ ] Confirm the roadmap explicitly retains template-body awareness as deferred.
- [ ] Confirm full editor/semantic highlighting is not promised.
- [ ] Confirm generated docs contain no old CSS roles.
- [ ] Confirm the active plan moved to Completed only after validation.

### Resolve findings

- [ ] Write each finding as a concrete issue with severity and owner.
- [ ] Fix all correctness, ownership, duplication, performance and test gaps.
- [ ] Keep fixes narrow.
- [ ] Re-run every affected targeted test.
- [ ] Re-run `just validate`.
- [ ] Re-run the docs release build when output changed.
- [ ] Re-run the dedicated benchmark when hot-loop code changed.
- [ ] Do not waive findings through comments or lint allowances.

### Close the plan

- [ ] Set plan status to complete.
- [ ] Set `CURRENT_SLICE` to `complete`.
- [ ] Set `NEXT_ACTION` to `none`.
- [ ] Record final good commit.
- [ ] Record final validation and benchmark conclusions.
- [ ] Condense completed phase notes without deleting decisions or acceptance criteria.
- [ ] Commit the final plan refresh.

## Phase 5 validation gate

- [ ] `cargo fmt --all`
- [ ] `git diff --check`
- [ ] `cargo test --workspace --quiet -- --format terse`
- [ ] `cargo run --quiet -- tests --terse`
- [ ] `cargo run --quiet -- tests --audit`
- [ ] `cargo run --quiet -- check docs --terse`
- [ ] `cargo run --quiet -- build docs --release`
- [ ] `just bench-validate`
- [ ] `just validate`

## Final completion criteria

- [ ] Every locked interview decision is implemented.
- [ ] Every phase has an accepted commit and refreshed capsule.
- [ ] One compiler-owned direct match supplies Moth source-word classes.
- [ ] `$code("moth")` covers the agreed word, symbol and contextual categories.
- [ ] Every code language uses the same general palette vocabulary.
- [ ] Old role names are deleted.
- [ ] Scanner is byte-slice based and allocation-conscious.
- [ ] Plain identifiers remain unwrapped.
- [ ] Dedicated benchmark shows no measurable regression.
- [ ] Generated docs size remains acceptable.
- [ ] Unit, integration, docs and benchmark validation pass.
- [ ] Roadmap, matrix and user docs are current.
- [ ] Stateful template-body-aware highlighting remains explicitly deferred.
- [ ] Final audit has no open findings.

---

## Validation command reference

Use the exact current commands from `docs/src/docs/codebase/style-guide/validation.mtf` and the `justfile`. Refresh this table if the repository changes.

### Fast local iteration

```bash
cargo fmt --all
cargo test --workspace --quiet keyword_policy -- --format terse
cargo test --workspace --quiet code_highlighter -- --format terse
cargo run --quiet -- tests --case html_code_highlighting --backend html
cargo run --quiet -- check docs --terse
just bench-validate
git diff --check
```

Test filters are illustrative. Use the narrowest actual current test name without skipping broader phase gates.

### Full code-bearing phase gate

```bash
just validate
```

### Final docs generation

```bash
cargo run --quiet -- build docs --release
```

### Non-recording performance evidence

```bash
just bench-frontend-check
just bench-check
```

Do not use `just bench` or `just bench-frontend` unless the user explicitly authorises recorded benchmark history and tracked-summary updates.

---

## Risk register

### Keyword authority accidentally changes tokenisation

Mitigation:

- return the same `TokenKind` from the shared direct match
- make existing token tests exhaustive
- compare old/new tokenizer output through the complete suite
- do not change shadow-keyword policy in the same phase

### Scanner becomes another lexer framework

Mitigation:

- keep one formatter-local scanner
- no public token stream
- no diagnostics
- no source locations
- no parser integration
- no extensible trait/plugin framework

### Contextual false positives

Mitigation:

- explicit precedence
- bounded lookahead/state
- ALL_CAPS constants as negative tests
- plain identifiers left unwrapped
- no semantic claims in docs

### CSS becomes language-specific

Mitigation:

- role names describe general syntax categories
- at least Function, Literal and Contract are exercised by a non-Moth profile
- no Moth-specific selectors
- path and builtin namespace reuse existing general roles

### Generated HTML becomes bloated

Mitigation:

- no plain-identifier spans
- no plain punctuation spans
- one compound token per span
- batch plain runs
- record total docs HTML byte delta
- treat unexplained double-digit growth as a blocker

### Compile-time benchmark is noisy

Mitigation:

- dedicated workload
- in-process and CLI cases
- five independent non-recording runs where practical
- compare docs and template stress cases too
- profile only visible regressions
- make no stronger claim than the benchmark supports

### Roadmap and matrix imply deferred language support

Mitigation:

- matrix updates only the existing formatter row
- roadmap owns template-body awareness
- reserved words use Keyword without feature status changes
- planned non-token words stay plain

---

## Required phase handoff format

After each phase, return:

```markdown
## Phase handoff

PHASE:
- ...

ACCEPTED_COMMIT:
- ...

SUMMARY:
- ...

PRODUCTION_FILES:
- ...

OBSOLETE_PATHS_REMOVED:
- ...

TESTS_ADDED_OR_UPDATED:
- ...

BENCHMARK_EVIDENCE:
- ...

DOCS_AND_STATUS:
- ...

AUDIT:
- findings:
- resolved:
- remaining:

VALIDATION:
- command:
- result:

KNOWN_UNRELATED_FAILURES:
- ...

NEXT_ACTION:
- ...
```

Do not claim a phase is accepted until its audit and required validation gate pass.
