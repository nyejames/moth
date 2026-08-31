# AUD-0005: Tokenizer correctness

- State: `complete`
- Kind: `Correctness`
- Area: `frontend.tokenizer` - `src/compiler_frontend/tokenizer/**` (lexical recognition, source locations, string/template delimiter context, numeric literal scanning, spacing diagnostics)
- Coverage: `complete`
- Reviewed: `2026-08`
- Baseline: `cargo test --lib tokenizer::` passed 94/94 on the audited working tree (2026-08-28). `just validate` was not run: the audit is read-only and the worktree carries in-flight path-values changes (including `tokens.rs` removing `TokenKind::DatatypePath`). Empirical behaviour was verified with `./target/release/moth check` on a minimal HTML project under `./tmp/aud0005`. No active plan touches the tokenizer (Boracle runs in a separate worktree; the compiler token/diagnostic data-layout plan sits at its activation gate, not active).

## What was inspected

Every file in the area, in full:

- `mod.rs`, `tokens.rs` (1223 lines), `lexer.rs` (1315), `numeric.rs` (160), `text_modes.rs` (282), `line_scanning.rs` (102), `newline_handling.rs` (51)
- `tests/lexer_tests.rs` (2152) and `tests/tokens_remap_tests.rs` (424) - the executable baseline

Context read but not audited: `src/compiler_frontend/keywords.rs` (identifier/keyword policy owner), `src/compiler_frontend/numeric_text/{grammar,parse}.rs` (shared numeric grammar owner), `src/compiler_frontend/style_directives/registry.rs` (merged registry policy), `src/compiler_frontend/arena/token_stats.rs`, `src/compiler_frontend/headers/header_dispatch.rs` substream construction (callers only), `src/compiler_frontend/ast` RawStringLiteral consumers (consumer side only).

Empirical checks ran through `moth check` on minimal projects: tight `<`/`>` comparisons, generic call-site syntax, bare-CR newline files, Unicode numeric characters, empty char literals, unterminated normal/code/discard template bodies.

## Authorities read

- `docs/compiler-design-overview.md`: opening authority text, `Architectural invariants`, `Frontend stages > Stage 1: tokenization`
- `docs/src/developer-docs/language/overview.mtf` (reference routing)
- `docs/src/docs/numbers/numeric-literals.mtf` - numeric literal contract
- `docs/src/docs/numbers/operators.mtf` - symbolic operator and compound-assignment spacing contract
- `docs/src/docs/language-overview/strings-and-characters.mtf` - escapes, chars, templates, raw-backtick status
- `docs/src/docs/language-overview/comments-and-naming.mtf` - comments, template-body `--` text, naming
- `docs/src/docs/generics/generic-declarations.mtf` and `type-application.mtf` + `generic-inference.mtf` - generics use `type` declarations and `of` application; explicit call-site type application is rejected
- `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf` (value/char/string orientation)
- `docs/src/docs/progress/@page.moth` rows for source kinds and string escape coverage
- `AGENTS.md`, `docs/roadmap/audit-guide.md`, `audit-kinds/README.md`, `audit-kinds/correctness.md`

## Existing findings and active plans checked

`open-audit-findings.md` has no open candidates; all recorded findings are resolved. AUD-0001 (test support), AUD-0002 (stage 0 performance), AUD-0003 (runtime assertion messages), AUD-0004 (audit framework) share no root cause with this run. Active work: Boracle (separate worktree); the compiler token/diagnostic data-layout plan is not active. The worktree's in-flight path-values changes touch `tokens.rs` (`DatatypePath` removal) and were taken as the audited state.

## Findings

### AUD-0005-F01: Tight `<`/`>` spacing suppression accepts binary comparisons that violate the operator spacing contract

- State: `candidate`
- Kind: `Correctness`

#### Evidence

`less_than_is_generic_angle_start` (`lexer.rs:411-421`) suppresses the binary-operator spacing diagnostic whenever the previous token is a `Symbol`, there is no whitespace before `<`, and the next char is uppercase. `greater_than_is_generic_angle_end` (`lexer.rs:423-431`) does the same when the next char is `(` or `,`. Both guards run before the `require_symbolic_spacing` calls at `lexer.rs:1062-1074` and `lexer.rs:1098-1110`.

The canonical contract is unconditional: "Symbolic binary operators require spaces on both sides" (`docs/src/docs/numbers/operators.mtf:54`, with `total=left+right` listed as INVALID). Generic syntax never uses angle brackets: generic declarations use the `type` keyword (`docs/src/docs/generics/generic-declarations.mtf`), instances use `of` (`type-application.mtf:19`), and explicit call-site type application such as `identity<Int>(42)` is rejected (`generic-inference.mtf:100-108`). So no legal construct requires the suppression.

Empirically, through `moth check` on a minimal project:

- `value = a<b` -> `MOTH-SYNTAX-0031` "Binary operator '<' requires whitespace on both sides." (contract enforced)
- `a Int = 1` + `B Int = 2` + `value = a<B` -> **0 errors** (only the `MOTH-RULE-0021` lowercase naming warning): accepted as a comparison, spacing contract silently violated
- `value = a>(b)` -> **0 errors, no warnings**: accepted as `a > (b)`, spacing contract silently violated
- `value = a<(b)` -> `MOTH-SYNTAX-0031` (asymmetric: the `>` guard fires on `(`, the `<` guard does not)
- `value = identity<Int>(42)` -> only `MOTH-RULE-0034` "Unknown value name 'identity'"; `value = a<B(b)` -> generic `MOTH-SYNTAX-0023` "Expected an operator before this expression." Neither is the dedicated explicit-call-site rejection the suppression presupposes.

#### Counter-explanation tested

Strongest counter-explanation: the suppression deliberately routes illegal generic call-site syntax (`identity<Int>(42)`) to a parser-owned diagnostic instead of a misleading spacing error, mirroring the documented `==` decision (`lexer.rs:711-713`). Rejected on evidence: the assumed parser-owned rejection does not exist as a dedicated diagnostic - `identity<Int>(42)` fails only at name resolution and `a<B(b)` produces a generic parse error - while the bare shapes `a<B` and `a>(b)` compile clean. Second counter-explanation: uppercase identifiers are impossible in legal code. Rejected: `B Int = 2` produces a naming warning, not an error, and the program compiles. Third: `a>(b)` might be an accepted grouped-RHS form. Rejected: `a<(b)` reports the spacing error, and the canonical rule has no grouped-RHS exception. What would disprove the finding: a canonical or parser-level rejection of `a<B`/`a>(b)` - reproduction shows the programs compile with no errors.

#### Violated contract or cost

`docs/src/docs/numbers/operators.mtf:54-68` - symbolic binary operators require whitespace on both sides. Also the Stage 1 ownership statement (`docs/compiler-design-overview.md:646`, "symbolic operator ... spacing diagnostics"): the tokenizer owns this rule and its suppression lets violating source through silently. The same tight form is rejected for lowercase operands, so the accepted surface is internally inconsistent (`a<b` errors, `a<B` compiles).

#### Root owner

`src/compiler_frontend/tokenizer/lexer.rs` - the `less_than_is_generic_angle_start` / `greater_than_is_generic_angle_end` predicates and their use in `get_token_kind`.

#### Suggested correction

Non-authorising. Either remove both suppression predicates so `<`/`>` always enforce the spacing rule, or narrow them to the complete call-application shape (symbol, `<`, uppercase list, `>` immediately followed by `(`) - and only if a parser-owned diagnostic for explicit call-site type application actually exists. The canonical rejection of explicit call-site syntax belongs to the AST/type-resolution owner; if that dedicated diagnostic is missing, that gap belongs to a separate finding under the owning stage, not to this tokenizer change.

#### Fix scope and preserved invariants

Fix touches only the two predicates and their call sites in `lexer.rs`. Preserve: spacing diagnostic identity (`MOTH-SYNTAX-0031`, construct and missing-side payloads), all currently passing tokenizer tests, and the `==`/`!=` parser-owned path (`lexer_tests.rs:1172`). Any acceptance-surface change must be reconciled with the canonical "explicit call-site syntax is rejected" contract by the implementing task.

#### Required validation

`cargo test --lib tokenizer::` plus full `just validate` (implementation task), and empirical `moth check` runs of the four probe programs above showing `a<b` and `a<B` now behave identically under the spacing rule.

#### Linked findings

The missing dedicated parser rejection of explicit call-site type application (`identity<Int>(42)` reaching name resolution) is a Stage 4/AST concern outside this area; not filed here to avoid a second area in one run.

### AUD-0005-F02: Consecutive bare-CR newlines are consumed as horizontal whitespace, drifting line and column tracking

- State: `candidate`
- Kind: `Correctness`

#### Evidence

After emitting a `Newline` token the main loop calls `consume_all_whitespace` (`lexer.rs:588-591`), which consumes every subsequent whitespace char through `TokenStream::next` (`tokens.rs:702-717`). `next` increments `line_number` only for `\n`; a `\r` consumed there just bumps `char_column`. The `\r`-aware branch (`lexer.rs:592-595`) runs only for the first `\r` of a run, because `consume_all_whitespace` has already swallowed any following `\r` as plain whitespace. `newline_handling.rs:1-3` states the policy "normalizes `\r` and `\r\n` into stable `\n`", which a consecutive bare CR never receives.

Trace: `a = 1\r\rzzzz\n` (two bare-CR line breaks). First `\r` normalises to a `Newline` token (line 2, col 0). `consume_all_whitespace` then consumes the second `\r` as horizontal whitespace (col 1, no line increment), so `zzzz` is located at line 2 col 1 instead of line 3 col 0. Empirical: `a = 1\n\nzzzz\n` reports the diagnostic at `3:5`; `a = 1\r\rzzzz\n` reports the same source at `2:6`. The drift persists for the rest of the file (one line per extra bare CR).

#### Counter-explanation tested

Counter-explanations: (1) consecutive newlines collapse to one statement boundary anyway, so location drift is cosmetic - rejected: token locations are the "first precise source-location mapping" (`lexer.rs:4`) and feed every downstream diagnostic; a file whose line breaks are bare CRs reports wrong line numbers from that point on. (2) The input is malformed - rejected: a lone `\r` is an accepted newline everywhere else in the tokenizer (unit-tested at `lexer_tests.rs:216-258`); two consecutive accepted line breaks cannot be malformed. What would disprove the finding: a second bare CR incrementing `line_number` in the trace, or the two empirical locations matching.

#### Violated contract or cost

`newline_handling.rs:1-3` newline normalization policy; Stage 1 ownership of source location tracking (`docs/compiler-design-overview.md:643`); the diagnostics preservation contract (locations feed diagnostic spans).

#### Root owner

`consume_all_whitespace` (`lexer.rs:1296-1311`) plus the Newline-token whitespace loop at `lexer.rs:584-602`: only that loop carries `\r` awareness, and it never sees a second consecutive `\r`.

#### Suggested correction

Non-authorising. Make `consume_all_whitespace` newline-aware (treat `\r` like `\n` - increment line, reset column - and collapse runs identically), or reuse `normalize_consumed_carriage_return_newline` for `\r` inside it.

#### Fix scope and preserved invariants

Fix is local to `lexer.rs` whitespace consumption. Preserve: single-`\r` and CRLF normalization (unit-tested), the one-boundary-per-newline-run collapse, all token payload values, and existing location behaviour for LF and CRLF sources.

#### Required validation

`cargo test --lib tokenizer::` plus full `just validate`; add behaviour coverage for consecutive bare-CR newlines under the Tests lane (linked, not authored here).

#### Linked findings

Linked Tests finding: no tokenizer test covers consecutive bare-CR newlines; `lexer_tests.rs:216-258` covers only single newlines per literal.

### AUD-0005-F03 (linked, Diagnostics lane): Unicode numeric characters are diagnosed as `_` separator errors

- State: `candidate`
- Kind: `Diagnostics` (linked finding; recorded without Diagnostics coverage - Correctness run)
- Area: `frontend.tokenizer` dispatch / shared numeric grammar boundary

#### Evidence

`get_token_kind` dispatches any `char::is_numeric()` char into `tokenize_numeric_literal` (`lexer.rs:1169`), but the shared grammar is ASCII-only (`is_numeric_digit` = `is_ascii_digit`, `numeric_text/grammar.rs:9-11`), and a text with zero ASCII digits yields `NumberLiteralErrorReason::InvalidSeparatorPlacement` (`numeric_text/parse.rs:73-75`). Empirically: `٣ = 2` and `b = ½` both produce `MOTH-SYNTAX-0008` "Numeric literal '٣' has an invalid `_` separator placement." - a factually false message for input containing no `_`.

#### Counter-explanation tested

Counter-explanations: rejection is correct so the wording is cosmetic - rejected: the message asserts a false fact about the user's source and misroutes a lexical-character problem to the numeric-literal lane; the correct owner diagnostic is the invalid-character path (`lexer.rs:1184`). What would disprove it: any canonical text making Unicode digits valid literal input (none found in `numeric-literals.mtf` or the progress matrix).

#### Violated contract or cost

Diagnostics accuracy: error identity and wording must match the actual source mistake (style-guide diagnostics bar; `CompilerDiagnostic` values with useful source context).

#### Root owner

The `is_numeric()` dispatch predicate in `lexer.rs:1169` admitting Unicode Nd/Nl/No chars into an ASCII-only grammar; the diagnostic text is produced by the shared `numeric_text` parse path.

#### Suggested correction

Non-authorising. Route non-ASCII `is_numeric()` chars to `invalid_character` in the dispatch, or give the shared grammar a dedicated non-ASCII-digit rejection reason. Diagnostics-kind triage owns the decision.

#### Fix scope and preserved invariants

Legality must not change: the input is rejected today and must stay rejected. Preserve `MOTH-SYNTAX-0008` identity for genuine separator mistakes and all existing numeric tests.

#### Required validation

`cargo test --lib tokenizer:: numeric_text::` plus full `just validate`; empirical `moth check` of a Unicode-digit file showing the corrected diagnostic.

#### Linked findings

None.

## Checked and clean

What was inspected and found sound:

- **Escape grammar**: quoted strings decode exactly `\\ \" \n \r \t` and reject unsupported escapes, physical-newline continuation and trailing backslash with typed, exactly-spanned diagnostics - matches `strings-and-characters.mtf` and the progress-matrix escape row; unit-tested per reason and span width.
- **Newline normalization** for single `\r` and `\r\n` in quoted strings, raw strings, template bodies and code bodies is correct in position tracking and payload (unit-tested; only the consecutive-bare-CR case in F02 is defective).
- **Numeric literal scanning**: separator placement, exponent rules, uppercase-`E` rejection, sign handling, authored-vs-normalized text preservation, and the decimal/exponent boundary all match `numeric-literals.mtf`; extensive unit coverage including signed, separated and out-of-range forms (out-of-range rejection happens at materialization by design, with authored text preserved for the diagnostic).
- **Match-arm lookahead** (`line_initial_match_arm_header`): speculative scan is position-safe (clones the char iterator), falls back conservatively to the spacing diagnostic, and rejects false arrows inside comments/strings (unit-tested across plain, guarded, multiline-guard and named-argument-guard forms).
- **Unary plus/minus rules**: `+x` rejected, `- count` rejected, attached `-count` and signed literals accepted - exactly the canonical contract.
- **Spacing diagnostics** for assignments, compound assignments and the `~=` mutable marker: all three missing-side branches unit-tested per operator; internal `~ =` spacing left to the declaration parser as documented.
- **Keyword and identifier policy**: reserved spellings, `This`/`this` distinction, wildcard `_`, `_`-prefixed symbols, attached `return!`/`cast!`, `assert`, `panic` as ordinary symbol - unit-tested; `keywords.rs` keeps one owner shared with the highlighter.
- **Style directive recognition**: merged registry cannot be overridden by builders (`registry.rs:37-72`), unknown/legacy directives rejected with a deterministic supported list; `$(`/`$` reactive routing; balanced (`$code`, `$css`, custom) and discard (`$note`, `$todo`) body modes keep brackets literal per mode.
- **Template entry and close policy**: `.mtf` implicit body rejects an unescaped outer `]`, allows nested closes, keeps `--` as text; unterminated normal, code and discard bodies are all rejected end-to-end (`MOTH-SYNTAX-0017`, verified through `moth check`), so the discard-to-Eof path does not silently accept.
- **Path-token lifecycle**: `FilePathSyntax` preparing/deferred/shared states enforce single mutable ownership with preflighted, infallible freeze commits; donor-local `PathSyntaxId` handles never escape their file-owned table; remap/rebind leave handles dense and remap only table rows (`tokens_remap_tests.rs`).
- **Failure paths**: all user-input failures return boxed `CompilerDiagnostic` values with locations; every `expect`/`panic!`/`debug_assert!` site is an internal-invariant guard behind a peek-confirmed precondition; `tokenize` constructs no partial `FileTokens` on failure.
- **Determinism**: token order is source order; registry order is stable and used for diagnostic lists; no HashMap/traversal-order dependence reaches tokens.
- **Char literals**: single Unicode scalar (including non-ASCII such as `'🦋'`); `''` rejected; no canonical char-escape contract exists to violate.

## Limitations

- `NestingDepth` (`src/compiler_frontend/utilities/token_scan.rs`) was read only as a consumed utility, not audited.
- The shared `numeric_text` grammar was audited only where tokenization consumes it (`parse_numeric_literal`); its materialization and cast paths belong to their own owners.
- Stage 2+ consumers (header dispatch, template parsing, AST) were inspected only at the tokenizer's handoff boundaries; behaviour inside them (including any missing dedicated rejection of explicit call-site type application) is out of scope for this run.
- `FileTokens::current_token_kind`/`current_location` index unchecked; every traced caller constructs non-empty streams, but no proof of unreachability for all future callers is claimed - no user-input trace was found, so no finding is filed.
- Findings were verified on the current in-flight worktree; a clean-tree reproduction is expected to match but was not separately run.