# Moth code-block highlighter corrections and cleanup plan

## Purpose

Correct and simplify the built-in HTML builder `$code` highlighter after the implementation landed in `a95a2db9d5707c622876cddf857dfc5ce21fc4e7`.

The accepted architecture remains sound: compiler-owned Moth word classification, one allocation-conscious byte scanner and one language-neutral role palette. This plan fixes output corruption and overly broad contextual heuristics, strengthens the tests around source preservation, consolidates duplicated HTML escaping and removes avoidable scanner state and repeated classification paths.

This is a correctness and maintainability follow-up. It must not become a second tokenizer, parser or semantic highlighter.

## Active context capsule

ACTIVE_PLAN:
- `docs/roadmap/plans/moth-code-block-highlighter-corrections-and-cleanup-plan.md`

CURRENT_SLICE:
- Phase: 4 (complete)
- Checklist item: Final cross-phase audit and closeout
- Goal: final audit gate, full validation and plan closeout
- Non-goals: none remaining; plan accepted

LAST_GOOD_COMMIT:
- `7d9f15d1b4895e90b1f41b097d194635cbe879d4`

CURRENT_WORKTREE_STATE:
- Clean / known changes: clean at refresh. Branch `main`, HEAD `7d9f15d1b` (plan commit). No dedicated worker worktrees.
- Branch: `main` when this plan was written
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
- `docs/src/docs/packages/builder/html/html-helpers-basic.mtf`
- `docs/src/docs/packages/builder/html/html-helpers.mtf`
- `benchmarks/README.md`

RELEVANT_CODE:
- `src/projects/html_project/styles/code.rs::CodeScanner`: scanner cursor, plain-run ownership and contextual state
- `src/projects/html_project/styles/code.rs::scan_moth_directive`: currently emits the directive twice
- `src/projects/html_project/styles/code.rs::scan_moth_path`: currently emits the path twice
- `src/projects/html_project/styles/code.rs::scan_word`: flushes every plain word instead of retaining it in a batched plain run
- `src/projects/html_project/styles/code.rs::set_non_moth_lookahead`: pending role can leak across delimiters
- `src/projects/html_project/styles/code.rs::moth_word_role`: `is` and `|` heuristics are broader than their intended contexts
- `src/projects/html_project/styles/escape_html.rs`: existing HTML-project escaping owner with logic duplicated in `code.rs`
- `src/projects/html_project/tests/code_tests.rs`: focused scanner and role test owner
- `src/projects/html_project/tests/document_shell_tests.rs`: shared role CSS contract owner
- `tests/cases/html_code_highlighting/`: primary user-visible artifact contract
- `benchmarks/code-highlighter-stress.moth`: existing dedicated performance workload
- `benchmarks/manifest.toml`: existing CLI and frontend highlighter benchmark cases

ACCEPTANCE_CRITERIA:
- Removing highlighter span tags from output yields exactly the HTML-escaped input, with no duplicated, omitted or reordered source bytes.
- `$directive` and `@path` runs appear exactly once in unit, integration and generated documentation output.
- All highlighted-run helpers flush only text before the token, then emit the token once and advance the cursor through one shared emission path.
- Unhighlighted words remain part of the current plain run instead of being written one word at a time.
- `$code` and `$escape_html` use one HTML-project-owned allocation-free escaping writer.
- Non-Moth declaration roles apply only to the exact immediately following identifier and cannot leak through delimiters, comments, strings or newlines.
- Moth option captures and untyped parameters inside `|...|` are not classified as function declarations.
- `is` arms a contract only inside a generic declaration. Ordinary comparisons such as `value is MAX_SIZE` keep constants non-contract.
- Generic code blocks have no language-specific word vocabulary.
- Invalid `@@name` is not presented as a valid-looking second `@name` path.
- The accepted shared role palette, compiler-owned word classifier, maximal-munch operators and lightweight lexical-only boundary remain intact.
- User-facing `$code` documentation names the HTML builder as owner and contains valid Moth examples.
- Generated docs contain no known duplicate directive/path patterns.
- The existing highlighter benchmark shows no measurable regression after repeated non-recording runs.
- `just validate` passes after every accepted code-bearing phase.

DECISIONS_ALREADY_MADE:
- decision: Keep the compiler-owned `classify_source_word` direct-match authority unchanged unless a correction proves it wrong.
  - reason: it already removes Moth vocabulary drift without a runtime map or HTML coupling.
  - source/user/date: accepted original design and implementation review, 2026-08-03 to 2026-08-04
- decision: Keep one byte-indexed lexical scanner and one shared role palette.
  - reason: the architecture is efficient and appropriately bounded. The defects are local state and emission-owner errors.
  - source/user/date: accepted original design, 2026-08-03
- decision: Add one scanner-owned highlighted-range emission primitive.
  - reason: duplicated cursor/flush/emission bookkeeping caused the release-blocking path and directive corruption.
  - source/user/date: implementation review correction, 2026-08-04
- decision: Reuse `styles/escape_html.rs` for shared HTML escaping rather than adding another utility module.
  - reason: `$escape_html` and `$code` have the same HTML-project-owned character escaping contract.
  - source/user/date: cleanup design, 2026-08-04
- decision: Preserve lightweight lexical heuristics, but make their context explicit and exact.
  - reason: functions and contracts improve documentation readability, while broad pending state creates false positives.
  - source/user/date: accepted original design plus implementation review, 2026-08-04
- decision: Stateful Moth template-body-aware suppression remains deferred.
  - reason: it needs nested template state and is separate from these correctness fixes.
  - source/user/date: accepted original design, 2026-08-03
- decision: Do not restore, relink or update the original completed-plan entry in the roadmap.
  - reason: the user is removing the roadmap's completed section separately.
  - source/user/date: user instruction, 2026-08-04
- decision: Do not change general site link colours, documentation switch colours or inline-code styling in this correction slice.
  - reason: those aesthetic changes are unrelated to scanner correctness and should not be churned incidentally.
  - source/user/date: cleanup scope decision, 2026-08-04

BLOCKERS / RISKS:
- `main` has advanced through unrelated module-system work since the highlighter commit. Refresh paths and avoid touching parallel-owner files.
- Generated documentation is broad. Rebuild it from source and inspect semantic changes rather than editing HTML directly.
- Exact lexical heuristics must remain tolerant of invalid snippets without panicking or reporting diagnostics.
- Benchmark timings are noisy. Compare repeated non-recording medians and do not update tracked history.
- Cursor refactoring can introduce dropped or duplicated text unless source-preservation tests land first.

VALIDATION_STATE:
- last command: Phase 0 gate (focused tests, integration case, docs check, docs release build, bench-validate, repeated non-recording bench runs)
- result: all passed. `cargo test -p moth code_tests`: 37/37. `tests --case html_code_highlighting --backend html`: 1/1. `check docs --terse`: clean. `build docs --release`: 68 files, tracked output unchanged. `bench-validate`: 60/60 preflight passed.
- starting gate: `just validate` passed (ci-clippy, workspace tests, integration suite, docs check, bench-ci).
- Phase 1 (uncommitted, awaiting interim audit): `cargo test -p moth code_tests`: 40/40. Workspace tests: 4647 passed. `tests --case html_code_highlighting --backend html`: 1/1. `check docs --terse`: clean. `bench-validate`: passed. `just validate`: passed. Duplicate path/directive scans now covered by exact-once and forbidden-adjacent artifact assertions.
- Phase 1 audit disposition: interim `auditor` route contract violations twice (only enabled candidate is the Ollama DeepSeek model; first attempt emitted prose before its JSON handoff, second attempted a workspace-outside path). The substantive first-attempt review was salvaged from the runtime log: status clean, one informational finding (document_shell.rs escaping is a distinct three-char/attribute contract and should stay separate). Coordinator independently re-verified pass 1 and pass 2 checklist items, classified the document_shell finding as rejected with evidence, and accepted Phase 1.
- Phase 2 (uncommitted, awaiting checkpoint): `cargo test -p moth code_tests`: 45/45. Workspace tests: 4652 passed. `tests --case html_code_highlighting --backend html`: 1/1. `check docs --terse`: clean. `bench-validate`: passed. `just validate`: passed after one clippy collapsible-if correction. Phase 2 audit disposition: `auditor` route remains unavailable (same single candidate, no formal handoff); Coordinator completed the pass 1 and pass 2 checklist review with no required findings.
- Phase 3 (uncommitted, awaiting checkpoint): docs wording and example corrected; `tests --audit` inventory written (1667 cases); release build rebuilt 68 files; generated duplicate path/directive adjacency 0/0; old role classes 0; invalid example 0; generated HTML bytes 2,941,031 (Phase 0 baseline 2,943,860, delta -2,829 from removed duplicates); bench-check 5x no measurable change (avg 0ms); bench-frontend-check 5x avg -1ms with no slower cases; `just validate` passed. Progress matrix and roadmap: no edit, support wording remains accurate.
- Final audit: `final_auditor` route returned `audit_clean` / handoff status `pass` with no findings (run 20260804T201207Z-c66862fc). Auditor independently re-ran `git diff --check`, generated-output duplicate scans (0/0), and the generated byte count (2,941,031). Final gate re-run at HEAD d91aa0699: `tests --terse` 1817/1817; `check docs --terse` clean; `bench-validate` passed; `just validate` passed; `cargo fmt --all --check` and `git diff --check` clean.
- Phase 0 evidence: generated docs contain 94 plain-path + highlighted-path duplicates and 28 plain-directive + highlighted-directive duplicates. Total generated HTML bytes: 2,943,860. Baseline medians (recorded, unchanged highlighter workload): `code_highlighter_stress_check` 4.497ms, `code_highlighter_stress_frontend` 18.257ms; `docs_check` 186.723ms / `docs_frontend` 1271.911ms but the docs workload fingerprint changed at the plan commit, so those two are not comparable baselines. Fresh non-recording runs (5x each): bench-check showed one +5ms average run then no measurable change; bench-frontend-check showed mixed/no measurable change. Logs under `/tmp/moth-baseline-bench-*.log`.
- known unrelated failures: none

DOCS_IMPACT:
- progress matrix needed: review required, edit not expected because support status remains unchanged
- roadmap needed: none in this plan
- other docs stale: `$code` owner wording and one invalid string-concatenation example in the HTML helper docs
- authorized docs updates: this plan, HTML helper docs, relevant source comments and generated `docs/release/**` output produced by the release build

NEXT_ACTION:
- none; plan complete and closed

---

## Context capsule maintenance

After each accepted phase and before compaction:

- [ ] Update `CURRENT_SLICE`, `LAST_GOOD_COMMIT`, worktree state and `NEXT_ACTION`.
- [ ] Narrow `RELEVANT_DOCS_THIS_SLICE` and `RELEVANT_CODE` to the next phase.
- [ ] Record exact validation results and known unrelated failures.
- [ ] Tick accepted checklist items and condense completed notes to one outcome line.
- [ ] Preserve the locked decisions, acceptance criteria and deferred boundaries.
- [ ] Re-read `AGENTS.md`, this capsule and the selected authorities after compaction.

Do not continue from compressed memory alone.

---

## Current state and root causes

### Repository anchor

```text
REPOSITORY: nyejames/moth
BRANCH: main
COMMIT: a3f8a00aff344934cba83e3602c03e577f5d1b46
DATE_CHECKED: 2026-08-04
ORIGINAL_IMPLEMENTATION: a95a2db9d5707c622876cddf857dfc5ce21fc4e7
```

The implementing agent must refresh this anchor before editing.

### Accepted implementation to preserve

The current implementation correctly established:

- one compiler-owned direct match for exact Moth source words
- one byte-indexed scanner over borrowed source
- direct output writing with no `Vec<char>` or per-word owned buffer
- maximal-munch Moth operators
- one language-neutral twelve-role CSS palette
- bounded Moth functions, contracts, directives, paths, nominal names and `io`
- focused formatter tests, one primary HTML artifact case and a dedicated benchmark workload

Do not replace these owners with a tokenizer, parser, regex engine, generic lexer framework or semantic model.

### Release-blocking output corruption

`scan_moth_directive` and `scan_moth_path` advance `self.index` through the token before calling `flush_plain`. Since `plain_start` still precedes the token, `flush_plain` writes the token as plain text. The helper then writes it again inside a span.

Current generated output includes forms such as:

```html
import @core/io<span class='moth-code-string'>@core/io</span>
[$md<span class='moth-code-directive'>$md</span>:
```

This is a scanner emission-ownership bug, not a special-case path or directive bug. Fix the common bookkeeping so another token helper cannot repeat it.

### Additional correctness and complexity gaps

1. `scan_word` calls `flush_plain` before it knows whether a word needs a span. Plain identifiers are then pushed individually, so the implementation does not fully realise its claimed batched-plain-run design.
2. `code.rs` and `escape_html.rs` independently implement the same five HTML escapes.
3. `pending_word_role` has no exact target position. An anonymous JavaScript `function (value)` can cause `value` or a later word to receive the Function role.
4. A Moth identifier before any `|` is treated as a function. This misclassifies option captures and untyped parameters inside `|value|`.
5. Every Moth `is` arms contract classification. An ordinary comparison such as `value is MAX_SIZE` can colour a constant as a trait.
6. Generic-bound commas and conformance-list commas have different meanings. One undifferentiated contract-list state cannot classify both precisely.
7. `@@name` currently lets the second `@name` become a path span, visually hiding the invalid doubled prefix.
8. The Generic profile shares TypeScript's keyword branch, so generic blocks can receive language-specific keywords despite being intended as syntax-only highlighting.
9. Four separate non-Moth helpers repeat language dispatch for keywords, types, literals and declaration-name lookahead.
10. `$code` documentation calls the directive frontend-owned even though the HTML builder registers it, and one example uses unsupported `String + String` syntax.

---

## Target scanner structure

### One highlighted-range emission primitive

Every highlighted token helper must compute an end index without mutating `self.index`, then use one scanner method to:

1. flush `plain_start..token_start`
2. emit exactly `token_start..token_end` inside one role span
3. set `index` and `plain_start` to `token_end`

Recommended shape:

```rust
fn emit_highlighted_range(
    &mut self,
    output: &mut String,
    token_start: usize,
    token_end: usize,
    role: CodeHighlightRole,
) {
    debug_assert_eq!(self.index, token_start);
    debug_assert!(token_start <= token_end);
    debug_assert!(token_end <= self.bytes.len());

    self.flush_plain(output);
    push_role_span_escaped(output, role, &self.source[token_start..token_end]);

    self.index = token_end;
    self.plain_start = token_end;
}
```

Exact naming may follow local style. The invariant is fixed. Do not keep bespoke flush/emit/advance sequences in individual token helpers.

For an unhighlighted word, advance `index` to the word end and leave `plain_start` unchanged. The next highlighted token or end-of-input flush then emits the complete plain run in one write.

### One HTML escaping primitive

Make `src/projects/html_project/styles/escape_html.rs` own an allocation-free writer used by both directives:

```rust
pub(super) fn push_escaped_html_text(output: &mut String, text: &str)
```

The helper should copy safe UTF-8 slices in batches and replace only the five ASCII bytes:

```text
&  <  >  "  '
```

Because every replacement byte is ASCII, byte indexes are valid UTF-8 boundaries. Do not decode every scalar when no escaping is needed. Keep the `$escape_html` formatter as the public directive owner and delete the duplicate escape loop from `code.rs`.

### Exact next-identifier expectation

Replace loose `pending_word_role: Option<CodeHighlightRole>` with an exact byte-position expectation:

```rust
struct ExpectedWordRole {
    start: usize,
    role: CodeHighlightRole,
}
```

A profile keyword may arm this only when the next non-horizontal-whitespace source position begins an identifier. The role applies only when the scanned word starts at that exact byte. A delimiter, comment, string, newline or anonymous declaration therefore cannot leak the role to a later word.

### Explicit Moth lexical context

Keep small, data-oriented state for the two Moth-only ambiguities:

- whether the scanner is currently inside a paired `|...|` group
- generic declaration and contract-list context

Recommended contract state:

```rust
enum ContractListKind {
    Conformance,
    GenericBound,
}

enum ContractState {
    None,
    ExpectName(ContractListKind),
    AfterName(ContractListKind),
}
```

Required transitions:

- `must` -> expect a conformance contract
- `not` after `must` -> keep expecting a conformance contract
- `type` -> enter generic declaration context
- `is` -> expect a generic-bound contract only while generic declaration context is active
- contract name -> `AfterName` for the same kind
- `and` after a contract -> expect another contract of the same kind
- comma after a conformance contract -> expect another conformance contract
- comma after a generic-bound contract -> end that bound list so the next word can be a new generic parameter
- declaration boundaries reset the relevant state

Keep the existing `ALL_CAPS followed by must` lookahead for trait declarations.

Toggle the pipe-group state on each structural single `|`. An ordinary identifier followed by `|` is a Function only while outside a pipe group. Calls before `(` remain unchanged. PascalCase nominal and contract roles still take priority.

### One non-Moth word classifier

Replace `is_keyword`, `is_type_keyword`, `is_literal_word` and `set_non_moth_lookahead` with one local classification function that returns:

- current word role, if any
- optional exact-next-identifier role

Keep direct per-language matches. Do not add a hash table, registry or module hierarchy. `CodeLanguage::Generic` must return no language-specific word role.

---

## Non-goals

- no Moth syntax or semantic change
- no tokenizer, parser, AST, name resolution or diagnostics in `$code`
- no TextMate, regex, tree-sitter or new dependency
- no stateful Moth template-body-aware highlighting
- no new language profiles
- no CSS palette redesign
- no general documentation-theme changes
- no compatibility selectors or old scanner path
- no new roadmap edit or restoration of the original completed plan link
- no progress-matrix row for code highlighting
- no recorded benchmark history
- no manual edits under `docs/release/**`

---

## Test ownership

### Focused scanner tests

Owner: `src/projects/html_project/tests/code_tests.rs`

Add or strengthen:

- exact `$directive` output
- exact `@path` output
- source-preservation invariant after stripping role spans
- anonymous and interrupted non-Moth declarations
- Moth function declaration versus `|value|` capture/parameter context
- generic-bound contracts versus ordinary `is` comparisons
- generic-parameter comma versus conformance comma
- `@@name` boundary
- Generic profile has no keyword/type/literal vocabulary
- shared escaping with ASCII and Unicode

Use table-driven cases where several inputs protect one invariant. Do not add one test per keyword.

### End-to-end artifact contract

Owner: existing `tests/cases/html_code_highlighting/`

Strengthen the existing primary case. Do not add another fixture.

Require exact-once occurrences for lexemes that appear once in the input, including a directive and each import path. Add forbidden adjacent duplicate forms. Add one untyped/capture `|value|` example and one ordinary `is MAX_SIZE` comparison so artifact output protects the contextual corrections.

### Generated documentation audit

After rebuilding, search every generated HTML route for:

- plain `@path` immediately followed by the same highlighted `@path`
- plain `$directive` immediately followed by the same highlighted `$directive`
- old role class names
- invalid string-addition example

Inspect at least the aliases, templates, packages/builder/html and progress routes.

### Performance evidence

Use the existing:

```text
code_highlighter_stress_check
code_highlighter_stress_frontend
```

Run non-recording suites only. Keep raw output under `/tmp`. Compare repeated medians against the Phase 0 baseline from the same machine and current source workload.

---

## Phase sequence

| Phase | Outcome |
|---|---|
| 0 | Current failures, generated corruption and performance baselines are reproduced at the refreshed repository head |
| 1 | Highlighted-run emission and HTML escaping have one owner, duplicate output is fixed and plain runs are truly batched |
| 2 | Contextual roles are exact, non-Moth word classification is consolidated and known false positives are removed |
| 3 | User docs and generated output are corrected, performance is remeasured and status documentation is reviewed |
| 4 | Final cross-phase audit, validation and plan closeout are complete |

Do not begin a phase until the previous phase has an accepted commit and refreshed capsule.

---

# Phase 0 - Refresh, reproduce and capture baselines

## Context

The highlighter files have not changed since the implementation, but `main` has advanced through unrelated module and benchmark work. Reproduce the defects at the actual starting commit and collect comparable non-recording evidence before refactoring.

## Checklist

### Refresh repository state

- [x] Read `AGENTS.md` and this phase's authority documents.
- [x] Fetch current `main` and record `git rev-parse HEAD`.
- [x] Record `git status --short --branch` and active worktrees.
- [x] Confirm whether `code.rs`, its tests, HTML helper docs or benchmark fixture changed after `a3f8a00a`.
- [x] Update the capsule with the actual starting commit and any path drift.
- [x] Keep unrelated R5/module-system changes out of this work.

### Reproduce correctness failures

- [x] Run the focused highlighter tests and record that they currently pass despite the generated corruption.
- [x] Build the docs release from source.
- [x] Confirm at least one duplicated import path and one duplicated directive in generated HTML.
- [x] Confirm the wrong `$code` ownership wording and invalid `String + String` example remain in source docs.
- [x] Add no correction yet.

### Capture performance and size baselines

- [x] Run `just bench-validate`.
- [x] Run `just bench-check` and `just bench-frontend-check` using non-recording modes.
- [x] Repeat both suites according to the repository optimisation protocol and keep outputs under `/tmp`.
- [x] Record medians for the dedicated highlighter cases and `docs_check` in the capsule.
- [x] Record total generated HTML bytes under `docs/release/**` after a clean release rebuild.
- [x] Do not update benchmark summaries, JSONL history or tracked results.

## Audit and style review

- [x] Confirm the defect is caused by scanner cursor ownership, not template parsing, whitespace formatting or document-shell indentation.
- [x] Confirm the planned shared escape owner is `styles/escape_html.rs` and no broader utility already owns the same HTML-project contract.
- [x] Confirm no production files changed in this phase.
- [x] Run `git diff --check`.

## Validation gate

- [x] Run the focused highlighter tests.
- [x] Run `cargo run --quiet -- tests --case html_code_highlighting --backend html`.
- [x] Run `cargo run --quiet -- check docs --terse`.
- [x] Run `just validate` to establish the starting gate.
- [x] Record exact results in the capsule.
- [ ] Commit only the refreshed plan/capsule if the user-added plan needs a checkpoint.

## Acceptance

- [x] Current head and worktree state are explicit.
- [x] Both duplicate-output forms are reproduced from source.
- [x] Comparable non-recording benchmark and generated-size baselines are recorded.
- [x] No implementation correction has been mixed into the baseline slice.

---

# Phase 1 - One emission path, one escaping owner and exact source preservation

## Context

The path/directive bug exists because highlighted token helpers repeat cursor bookkeeping. This phase makes token emission structurally hard to misuse, fixes the corruption and removes duplicated escaping and unnecessary plain-word writes without changing contextual classification.

## Checklist

### Consolidate HTML escaping

- [x] Add `push_escaped_html_text` to `src/projects/html_project/styles/escape_html.rs`.
- [x] Implement it with safe UTF-8 slice copies between the five ASCII escape bytes.
- [x] Route the `$escape_html` formatter through this helper.
- [x] Route code-block plain text and role-span content through this helper.
- [x] Delete `push_escaped_text` and `push_escaped_char` from `code.rs`.
- [x] Update `escape_html.rs` file docs to describe both the shared primitive and directive wrapper.
- [x] Do not move the helper into a broad frontend or project utility module.

### Centralize highlighted-run emission

- [x] Add one `emit_highlighted_range` scanner method with explicit start/end invariants.
- [x] Refactor comments, quoted runs, delimiters, directives, paths, numbers and operators to compute local end indexes without mutating `self.index` first.
- [x] Route every highlighted range through the shared method.
- [x] Remove repeated flush/span/index/plain-start blocks from token helpers.
- [x] Keep context-state transitions adjacent to the token semantics, not hidden inside the generic emitter.

### Batch plain runs correctly

- [x] Refactor word scanning to compute the word range first.
- [x] If the word has a role, emit it through the shared range method.
- [x] If the word has no role, advance `index` and leave `plain_start` unchanged.
- [x] Confirm ordinary identifiers, punctuation and whitespace can remain in one plain run until the next span or EOF.
- [x] Preserve attached `return!` and `cast!` as one keyword range.

### Fix duplicate output

- [x] Make directive and path end scanners use local indexes or `consume_while`.
- [x] Confirm `$md`, `$slot`, `$children`, `@core/io` and `@web/canvas` each emit once.
- [x] Do not special-case known names. Keep registry-extensible directive spelling and tolerant paths.

### Strengthen tests before accepting

- [x] Add exact-output tests for one directive and one path.
- [x] Add a table-driven source-preservation test: strip known role tags and compare with independently escaped source.
- [x] Include plain identifiers, Unicode, comments, strings, directives, paths and compound operators in that table.
- [x] Add exact `$escape_html` coverage for all five special characters plus Unicode.
- [x] Update the existing artifact case with exact-once directive/path assertions and forbidden duplicate patterns.
- [x] Confirm the new tests fail on the pre-fix implementation and pass after the refactor.

## Audit and style review

- [x] Search `code.rs` for repeated `flush_plain` + `push_role_span_escaped` + cursor update sequences. Only the shared emitter should own the complete sequence.
- [x] Search the HTML project for duplicate five-character escape loops. `$code` and `$escape_html` must share one implementation.
- [x] Confirm no highlighted helper advances `self.index` before handing the original token start to the emitter.
- [x] Confirm unhighlighted words do not trigger output writes.
- [x] Review comments for concise ownership and ordering explanations.
- [x] Run `cargo fmt --all` and `git diff --check`.

## Validation gate

- [x] Run focused code highlighter and escape directive tests.
- [x] Run `cargo run --quiet -- tests --case html_code_highlighting --backend html`.
- [x] Run complete workspace tests.
- [x] Run `cargo run --quiet -- check docs --terse`.
- [x] Run `just bench-validate`.
- [x] Run `just validate`.
- [ ] Record results in the capsule, commit the accepted slice and set Phase 2 as next.

## Acceptance

- [x] The highlighter preserves every escaped source byte exactly once.
- [x] Directive and path corruption is fixed by one emission owner, not two local patches.
- [x] Plain words are batched.
- [x] HTML escaping has one HTML-project owner.
- [x] No accepted role, operator or compiler-word behavior regressed.

---

# Phase 2 - Exact contextual state and profile simplification

## Context

The remaining defects come from loose state: a role can target an unspecified future word, `|` has no opening/closing context and `is` does not distinguish generic bounds from ordinary comparisons. This phase makes each heuristic narrow and consolidates repeated non-Moth classification without adding semantic analysis.

## Checklist

### Replace loose pending-role state

- [x] Replace `pending_word_role` with `ExpectedWordRole { start, role }` or an equivalent exact target.
- [x] Add a helper returning the next non-horizontal-whitespace byte index.
- [x] Arm an expected role only when that exact position begins an identifier.
- [x] Consume it only when `word_start == expected.start`; otherwise clear it.
- [x] Confirm anonymous declarations and intervening punctuation cannot leak a role.

### Consolidate non-Moth word classification

- [x] Replace `is_keyword`, `is_type_keyword`, `is_literal_word` and `set_non_moth_lookahead` with one direct local classifier.
- [x] Return the current role and optional next-identifier role as one small value or enum.
- [x] Keep static direct matches and no allocation.
- [x] Give `CodeLanguage::Generic` no language-specific word roles.
- [x] Preserve the accepted bounded JavaScript, TypeScript, Python, Rust and shell role vocabulary only.
- [x] Do not broaden non-Moth language coverage in this slice.

### Bound Moth function declarations

- [x] Add explicit paired-pipe context to the Moth scanner state.
- [x] Toggle it on each structural single `|` delimiter.
- [x] Classify an ordinary lower-case identifier before `|` as Function only when outside a pipe group.
- [x] Keep ordinary call and method names before `(` as Function.
- [x] Preserve priority: compiler words, contracts and PascalCase nominals win before function fallback.
- [x] Confirm `render |value|` marks only `render` as Function.
- [x] Confirm `if option is |value|` leaves `value` plain.
- [x] Confirm untyped function parameters inside `|...|` remain plain.

### Bound Moth contract lists

- [x] Replace the current broad contract expectation with explicit `ContractListKind` and `ContractState` or an equally clear model.
- [x] Track generic declaration context after `type` until the declaration's structural boundary.
- [x] Arm generic contract expectation on `is` only in that context.
- [x] Keep `must` and `must not` conformance/incompatibility handling.
- [x] Make `and` continue the current contract kind.
- [x] Make comma continue conformance lists but end a generic bound before the next generic parameter.
- [x] Retain ALL_CAPS-followed-by-`must` trait declaration detection.
- [x] Reset state at newline and declaration boundaries without leaking into later source.
- [x] Confirm `value is MAX_SIZE` does not mark `MAX_SIZE` as Contract.
- [x] Confirm `render type Item is DISPLAY_TEXT |...|` does.
- [x] Confirm `type A is FIRST, B is SECOND` treats `B` as a nominal generic parameter rather than a contract.
- [x] Confirm `Label must FIRST, SECOND` marks both contracts.

### Tighten Moth path start boundaries

- [x] Require `@path` to start at a lexical boundary rather than immediately after another `@` or path/identifier continuation.
- [x] Keep tolerant path continuation and no path validation.
- [x] Make `@@name` render as two separate `@` operator spans followed by plain `name`.
- [x] Preserve imports, resource paths and Markdown-style paths in normal boundary positions.

### Add focused regressions

- [x] Add exact anonymous JavaScript function output.
- [x] Add interrupted declaration cases for delimiter, comment and newline boundaries.
- [x] Add Generic profile word-vocabulary absence coverage.
- [x] Add Moth declaration/capture pipe cases.
- [x] Add generic-bound, ordinary-`is`, conformance comma and generic comma cases.
- [x] Add exact `@@name` output.
- [x] Extend the existing integration fixture rather than adding another case.

## Audit and style review

- [x] Confirm contextual state fields describe actual lexical facts rather than vague pending booleans.
- [x] Confirm exact-position expectation removes the need for scattered pending-role resets.
- [x] Confirm one non-Moth classifier owns word role decisions.
- [x] Confirm generic and conformance comma behavior is explicit in names and tests.
- [x] Confirm malformed snippets remain tolerated and cannot panic.
- [x] Keep `code.rs` within one coherent owner. Do not split it unless the refactor demonstrably becomes harder to review.
- [x] Remove superseded state variants, helpers and comments.
- [x] Run `cargo fmt --all` and `git diff --check`.

## Validation gate

- [x] Run focused highlighter tests.
- [x] Run `cargo run --quiet -- tests --case html_code_highlighting --backend html`.
- [x] Run complete workspace tests.
- [x] Run `cargo run --quiet -- check docs --terse`.
- [x] Run `just bench-validate`.
- [x] Run `just validate`.
- [ ] Record results in the capsule, commit the accepted slice and set Phase 3 as next.

## Acceptance

- [x] No role targets an unspecified later word.
- [x] Generic blocks remain vocabulary-neutral.
- [x] Moth captures and parameters are not function declarations.
- [x] Ordinary `is` comparisons do not create trait colours.
- [x] Generic and conformance lists classify their commas correctly.
- [x] Invalid double-`@` input is not presented as a valid path.
- [ ] Repeated non-Moth word dispatch helpers are gone.

---

# Phase 3 - Documentation, generated output and performance evidence

## Context

With scanner correctness accepted, align user-facing documentation and regenerate the primary product surface. Remeasure performance and output size only after duplicate text has been removed so the evidence describes the actual implementation.

## Checklist

### Correct source documentation

- [x] Update `html-helpers.mtf` to state that `$code` is an HTML-builder style directive executed through the frontend formatter registry.
- [x] Remove the claim that it is unconditionally frontend-owned or available under every builder.
- [x] Replace `return "Hello, " + name` with valid template concatenation such as `return ["Hello, ", name]`.
- [x] Keep Basic wording terse and consistent with the advanced owner statement.
- [x] Update `styles/mod.rs` and `code.rs` file docs where they omit or misname the HTML-project owner.
- [x] Do not change unrelated theme colours, inline-code margins or font weight.

### Review status documentation

- [x] Review the existing `Templates and style directives` progress row after stronger tests land.
- [x] Do not add a new row.
- [x] Leave the row unchanged if support and coverage wording remain accurate.
- [x] Make no roadmap edit in this plan.
- [x] Do not restore or link the original completed implementation plan.

### Rebuild and inspect release docs

- [x] Run the documentation release build from source.
- [x] Do not edit generated HTML manually.
- [x] Search every route for duplicated path/directive adjacency.
- [x] Search for `moth-code-struct` and `moth-code-parenthesis`.
- [x] Search for the invalid string-addition example.
- [x] Inspect aliases, templates, packages/builder/html and progress routes in full context.
- [x] Confirm directives, paths, functions, contracts and plain source now appear once and in order.

### Recompare output size and performance

- [x] Record corrected total generated HTML bytes using the same Phase 0 method.
- [x] Separate the effect of removed duplicate text from intentional role-span overhead.
- [x] Run `just bench-check` and `just bench-frontend-check` in non-recording mode.
- [x] Repeat according to the repository optimisation protocol.
- [x] Compare dedicated highlighter and docs medians with Phase 0.
- [x] Profile only if a repeatable regression appears.
- [x] Do not update tracked benchmark history or summaries.
- [x] Record concise evidence in the capsule and final handoff.

## Audit and style review

- [x] Verify every changed user example is valid current Moth or explicitly labelled invalid.
- [x] Verify docs name HTML builder, frontend and formatter ownership accurately.
- [x] Confirm generated changes derive only from source/code corrections.
- [x] Confirm unrelated aesthetic CSS is untouched.
- [x] Confirm progress-matrix and roadmap no-edit decisions are explicit.
- [x] Run `cargo fmt --all` if Rust changed during corrections and `git diff --check`.

## Validation gate

- [x] Run focused unit tests.
- [x] Run the primary integration case.
- [x] Run `cargo run --quiet -- tests --audit`.
- [x] Run `cargo run --quiet -- build docs --release`.
- [x] Run the generated-output duplicate searches.
- [x] Run `just validate`.
- [ ] Record results in the capsule, commit the accepted slice and set Phase 4 as next.

## Acceptance

- [x] User docs describe the real owner and show valid syntax.
- [x] Generated docs contain no known duplicated path/directive output.
- [x] Generated-size evidence is corrected.
- [x] Performance is improved or shows no measurable regression.
- [ ] No roadmap or unrelated theme churn was introduced.

---

# Phase 4 - Final audit and closeout

## Context

The final gate checks the entire correction train rather than trusting phase-local tests. The goal is one understandable scanner path with exact output preservation, bounded context and no stale implementation or documentation claims.

## Checklist

### Cross-phase implementation audit

- [x] Re-read `AGENTS.md`, style, testing and validation authorities.
- [x] Review the complete diff from the Phase 0 anchor.
- [x] Confirm one compiler-owned Moth word classifier remains.
- [x] Confirm one HTML-project escape writer remains.
- [x] Confirm one highlighted-range emission primitive owns flush/span/cursor updates.
- [x] Confirm plain words stay batched.
- [x] Confirm no `pending_word_role`, duplicate word predicates or old emission paths remain.
- [x] Confirm contextual state is bounded, named and covered by tests.
- [x] Confirm no new parser, tokenizer, regex, map or allocation-heavy path was added.
- [x] Confirm old CSS role names remain absent.
- [x] Confirm no dead helpers, stale comments or broad compatibility wrappers remain.

### Test and output audit

- [x] Confirm the source-preservation invariant covers representative highlighted and plain runs.
- [x] Confirm exact-output tests own narrow scanner facts and the integration case owns user-visible HTML.
- [x] Confirm no redundant new fixture was added.
- [x] Re-run generated duplicate searches after the final rebuild.
- [x] Inspect the integration artifact and representative docs routes manually.

### Documentation and status audit

- [x] Confirm HTML helper docs, file-level docs and progress wording agree.
- [x] Confirm deferred template-body-aware highlighting remains documented as deferred.
- [x] Confirm this plan contains the final accepted commits and validation state.
- [x] Make no roadmap edit.

## Style review

- [x] Review naming, function size, vertical spacing and comments across every touched Rust file.
- [x] Prefer explicit local state and direct matches over clever combinators.
- [x] Confirm extraction into `escape_html.rs` reduced duplication without creating a broad utility.
- [x] Remove any test-only production API that is no longer justified.
- [x] Run `cargo fmt --all --check` and `git diff --check`.

## Final validation gate

- [x] Run focused highlighter and escaping tests.
- [x] Run complete workspace tests.
- [x] Run `cargo run --quiet -- tests --audit`.
- [x] Run `cargo run --quiet -- tests --terse`.
- [x] Run `cargo run --quiet -- check docs --terse`.
- [x] Run `cargo run --quiet -- build docs --release`.
- [x] Run `just bench-validate`.
- [x] Run `just validate`.
- [x] Confirm the worktree is clean after the accepted final commit.

## Final acceptance

- [x] Every escaped source byte appears exactly once in highlighted output.
- [x] Directives and paths are not duplicated anywhere in tracked docs.
- [x] Contextual roles have exact, bounded targets.
- [x] Scanner and escaping duplication is removed.
- [x] Tests protect the root invariants rather than only substring presence.
- [x] Documentation and status tracking are accurate.
- [ ] Non-recording performance evidence is acceptable.
- [ ] Full validation passes.

---

## Agent handoff format

Return a concise closeout containing:

1. starting and final commit
2. root causes corrected
3. production paths simplified or deleted
4. exact tests added or strengthened
5. generated routes inspected
6. before/after generated HTML byte counts
7. non-recording benchmark comparison
8. focused and full validation results
9. progress-matrix decision
10. remaining deferred boundary or risk
