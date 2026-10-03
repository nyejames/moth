# AUD-0013: MON source parity spacing correctness

- State: `complete`
- Kind: `Correctness`
- Area: `contract.mon_source_parity` — compare MON reader rules with compiler tokenization and source construction parsing; this follow-up covers `=` spacing and choice-variant `::` adjacency.
- Coverage: `partial`
- Reviewed: `2026-10`
- Baseline: Branch `feature/mon-crate-extraction`, reviewed tip `8b1229c119693e070fbf565824402c75453a7162`. The external reviewer source-traced that tip. Focused validation passed: `cargo test -q -p moth --lib` (5,460 passed); `cargo test -q -p moth-mon` (passed); clippy for `moth` and `moth-mon` clean; `moth check docs --terse` clean. The final `just validate-full` result for the exact closeout tree is recorded in the closeout commit message.

## What was inspected

The external review source-traced the language's outer-spacing rule for `=`, MON named-entry and map-entry recognition, and the source/MON choice-variant separator paths. The original 156-case parity matrix used `first = 1` for its `none` case, so it did not exercise compact `=` spacing. The correctness follow-up also traced two source lexer bypasses in the same documented rule: a missing-RHS line-break boundary and a preceding `NEWLINE` token. Choice expression and match-pattern paths that skipped newlines after `::` were also traced. The closeout adds the expanded paired `=` corpus, MON reader error tests, choice-spacing parity fixtures and focused frontend tests. Other source/MON grammar differences were not exhaustively audited.

## Authorities read

`docs/roadmap/audit-guide.md`; `docs/roadmap/audit-kinds/correctness.md`; `docs/roadmap/audits/README.md`; `docs/roadmap/audit-log.md`; `docs/roadmap/open-audit-findings.md`; `docs/src/docs/numbers/operators.mtf` (outer spacing and `MOTH-SYNTAX-0031`); `docs/src/docs/mon/mon-format.mtf` (named and map entries); `docs/src/docs/language-overview/mon-syntax.mtf`; `docs/src/docs/choices/variant-construction.mtf`; and `docs/src/docs/choices/payload-patterns.mtf`.

## Existing findings and active plans checked

The audit log and open-findings index were checked. AUD-0009 established the MON/source construction-parity area and AUD-0010-F02 recorded the previous parity matrix and its coverage limits. The accepted closeout scope does not widen source grammar beyond the explicit choice-adjacency decision below; the spaced choice-declaration header remains a separate syntax form.

## Findings

### AUD-0013-F01: Source and MON entries bypass the outer-spacing contract

- State: `closed`
- Kind: `Correctness`

#### Evidence

The documented source rule requires outer whitespace around assignment `=`, under `MOTH-SYNTAX-0031` (`docs/src/docs/numbers/operators.mtf:155-158`). The external review found MON's `try_named_label` and map-entry paths accepted compact forms such as `first=1`, `first =1`, `first= 1` and `{\"a\"=1}`. The prior 156-case parity matrix missed the boundary because its `none` spacing case still formed `first = 1`.

The follow-up source trace also found two bypasses of the same documented contract. When the next character after `=` was a line-break boundary, the lexer skipped the whole spacing check, so `(first=\\n1)` escaped the required whitespace before `=`. When a map key was followed by a `NEWLINE` token before `=`, the lexer also skipped the check, so `{\"a\"\\n=1}` and `{\"a\" -- c\\n=1}` escaped the required whitespace after `=`. A missing-right-hand-side boundary may exempt the right side only; it does not waive the left-side check. A `NEWLINE` before `=` supplies leading whitespace but does not waive a missing right-side gap.

The source lexer now applies the documented rule in both cases. MON applies one `require_equals_spacing` check to named entries and map entries, always checking the left side and checking the right side unless input ends or a closing entry delimiter follows. It reports `UnexpectedToken` at `=` with the missing side or sides and the entry-field path. Missing-value errors at end of input or closing punctuation remain unchanged, `{=}` remains the empty-map form, and the writer already emits ` = `.

#### Counter-explanation tested

The source lexer intentionally treats EOF, LF/CR, comma, `)`, `]`, `}`, and `;` as missing-right-hand-side boundaries. That exception only concerns the absent right operand; it does not permit a missing space before `=`, and it does not exempt an absent space after `=` when trivia before the separator supplied its left-side whitespace. Likewise, MON's literal-data role could justify a separate compact spelling only if its contract said so; its entry notation shares the source spacing rule and the writer already uses outer spacing. Neither distinction justifies skipping the complete check.

#### Violated contract or cost

MON accepted compact entry forms the documented source rule rejects, while source itself accepted two line-boundary forms that violated that rule and differed from MON. The lexer bypasses also made the documented source spacing behavior depend on which token or character followed the separator.

#### Root owner

Source assignment-spacing recognition in `src/compiler_frontend/tokenizer/lexer.rs` and MON named-entry/map-entry spacing in `crates/moth-mon/src/reader.rs`.

#### Suggested correction

Make the source lexer check the left side regardless of a missing-RHS boundary and check the right side unless that boundary explains an absent value. Do not bypass the check when `=` follows a `NEWLINE` token. Keep one MON reader check for named and map entries with MON's matching delimiter set; preserve missing-value errors and `{=}`.

#### Fix scope and preserved invariants

Resolved in the closeout commit following `8b1229c11`. The source lexer now checks both spacing sides at the correct boundaries. MON's single `require_equals_spacing` check enforces the same rule for named and map entries while preserving the MON-specific missing-value cases. The closeout leaves `{=}`, writer output and source's documented missing-RHS behavior intact. MON acceptance tightening is recorded in `docs/src/docs/mon/mon-format.mtf` and `docs/src/docs/language-overview/mon-syntax.mtf`.

#### Required validation

The paired parity corpus has 32 fixtures: four spellings before/after/both/before-with-line-break-value across five named contexts, plus those spellings and after-line-break-key/after-comment-key at first and later map entries. `missing_equals_spacing_is_rejected_by_source_and_mon` checks source and MON. `equals_spacing_errors_report_missing_sides_at_equals` pins error identity, `=` span, missing-side detail and field path. Restoring the old source missing-RHS skip fails `first_named_spacing_missing_before_with_line_break_value`; restoring the `NEWLINE` skip fails `map_first_entry_spacing_missing_after_line_break_key`; disabling the MON reader rule fails the parity case. The focused `moth` and `moth-mon` tests and clippy passed; the final `just validate-full` result is recorded in the closeout commit message.

AUD-0010-F02 records the parity-test owner and earlier boundary matrix; AUD-0014-F01 records the shared line-break predicate used for missing-RHS boundaries.


#### Triage record

2026-10-03 — **Accepted.** Enforce the documented outer-spacing rule in both source and MON. A missing-right-hand-side boundary exempts only the right side, and a preceding newline does not waive the missing gap after `=`. Preserve MON's `{=}` form and missing-value diagnostics.

2026-10-03 — **Fixed and closed.** The closeout commit following `8b1229c11` repairs the source lexer boundary skips, applies one MON check to named and map entries, and adds the expanded paired corpus and reader errors. The 32 fixtures and source failure-sensitivity cases protect the previously missed paths. Focused tests and clippy passed; the final `just validate-full` result is recorded in the closeout commit message.

### AUD-0013-F02: Choice-variant separator spacing differs between source and MON

- State: `closed`
- Kind: `Correctness`

#### Evidence

The external review found that source choice construction accepted same-line whitespace around `::`, and expression and match-pattern paths could skip newlines or comments after the separator. MON required adjacency and documented a qualified variant as one lexical header. A line break before `::` ends the source expression first; it does not extend the qualified header. The reviewed source behavior therefore accepted forms such as `Theme :: Ready`, `Theme:: Ready` and a newline or comment after `::` that MON rejected.

The accepted correction makes adjacency the source and MON contract. The source lexer records whether the qualifier and variant touch `::`; expression and match-pattern parsing report `MOTH-SYNTAX-0031` with the missing side for a gap in a qualified header, and no longer skip newlines after the separator. Spaced choice declarations such as `Theme :: Ready, Busy;` remain valid declaration syntax and are not qualified variant construction.

#### Counter-explanation tested

The prior source parser's whitespace and newline tolerance could have been deliberate. The user explicitly chose adjacency to match the MON lexical-header contract. Declaration syntax uses `Name ::` in a different grammar position and remains spaced, so retaining that distinct form does not require whitespace around `::` in expressions or patterns.

#### Violated contract or cost

The two readers disagreed on whether same-line whitespace or trivia after `::` may split a qualified choice variant. That undermined the shared construction-parity boundary and made a single data shape depend on parser-specific trivia handling.

#### Root owner

Choice-separator tokenization and qualified-variant parsing in the compiler frontend, aligned with the MON reader's adjacency rule.

#### Suggested correction

Preserve left and right adjacency on the source `DOUBLE_COLON` token and reject gaps at both qualified-expression and match-pattern sites. Keep spaced choice-declaration headers outside this construction rule.

#### Fix scope and preserved invariants

Resolved in the closeout commit following `8b1229c11`. `DOUBLE_COLON` records left/right adjacency in descriptor-allowed token flags. Source choice expressions and match patterns report `MOTH-SYNTAX-0031` for invalid gaps, and both `skip_newlines()` paths were removed. Spaced choice declarations remain accepted. The source choice construction and payload-pattern documentation now state the adjacency contract; MON continues to use the same rule.

#### Required validation

The choice-spacing parity corpus covers gaps before and after `::`, on both sides, and after it via a line break or comment. Frontend tests cover choice expressions and match patterns; disabling the check fails five tests across the expression, match-pattern and parity sites. `moth check` smoke cases confirmed `Theme ::Ready`, `Theme:: Ready`, `Theme :: Ready`, newline/comment after `::` in payload contexts, and spaced match patterns report `MOTH-SYNTAX-0031`; adjacent forms and spaced choice declarations remain accepted. Focused tests and clippy passed; the final `just validate-full` result is recorded in the closeout commit message.

#### Linked findings

AUD-0010-F02 records the parity-test owner and earlier constructor-boundary coverage.

#### Triage record

2026-10-03 — **Accepted.** Tighten source qualified-variant construction and matching to the documented MON adjacency rule. Keep spaced choice declarations valid and preserve distinct parser error locations and gap-side details.

2026-10-03 — **Fixed and closed.** The closeout commit following `8b1229c11` adds adjacency flags to `DOUBLE_COLON`, rejects nonadjacent qualified variants in expressions and patterns, removes newline skipping after `::`, and updates the syntax references. The choice-spacing parity and frontend regressions pass, including failure-sensitivity checks. Focused tests and clippy passed; the final `just validate-full` result is recorded in the closeout commit message.

## Checked and clean

The source outer-spacing rule remains unchanged. MON's `{=}` empty-map marker and writer formatting remain unchanged. Choice declarations continue to use the spaced `Name ::` form. A line break before `::` ends the source expression first; adjacency diagnostics concern gaps within a qualified header. Source/MON differences outside these reviewed boundaries remain outside this report.

## Limitations

Coverage is partial and limited to these source/MON spacing boundaries and their focused tests and references. The external review source-traced the baseline; it did not exhaustively inspect every grammar rule or parser path. This report does not claim full language parity or report the final full-tree validation result itself.
