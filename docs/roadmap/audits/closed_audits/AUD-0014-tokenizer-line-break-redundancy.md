# AUD-0014: Tokenizer line-break predicate redundancy

- State: `complete`
- Kind: `Redundancy`
- Area: `frontend.tokenizer` — line-break boundary classification in `src/compiler_frontend/tokenizer/lexer.rs`, focused on `character_is_missing_rhs_boundary` and its shared lexical policy.
- Coverage: `partial`
- Reviewed: `2026-10`
- Baseline: Branch `feature/mon-crate-extraction`, reviewed tip `8b1229c119693e070fbf565824402c75453a7162`. The external reviewer source-traced that tip. Focused validation passed: `cargo test -q -p moth --lib` (5,460 passed); `cargo test -q -p moth-mon` (passed); clippy for `moth` and `moth-mon` clean; `moth check docs --terse` clean. The final `just validate-full` result for the exact closeout tree is recorded in the closeout commit message.

## What was inspected

The external review traced `character_is_missing_rhs_boundary` and the shared `moth_lexical::is_line_break` policy used by adjacent lexer boundary checks. The relevant comparison was the line-break classification, not the helper's local comma, bracket, brace and semicolon cases. The report covers this helper and its direct shared-policy boundary, not all tokenizer utilities.

## Authorities read

`docs/roadmap/audit-guide.md`; `docs/roadmap/audit-kinds/redundancy.md`; `docs/roadmap/audits/README.md`; `docs/roadmap/audit-log.md`; `docs/roadmap/open-audit-findings.md`; the shared logical-line policy in `moth-lexical`; and the earlier parity finding AUD-0009-F01, which aligned MON line-boundary handling with that policy.

## Existing findings and active plans checked

The audit log and open-findings index were checked. AUD-0009-F01 already records the shared LF/CR logical-line owner and its use by the MON reader and compiler lexer. AUD-0013-F01 uses the same boundary classification for source missing-right-hand-side spacing checks. This finding covers the duplicate LF/CR implementation in one lexer helper; it does not reopen the accepted line-break semantics or claim a runtime-performance issue.

## Findings

### AUD-0014-F01: Lexer boundary helper repeats shared line-break policy

- State: `closed`
- Kind: `Redundancy`

#### Evidence

Before the correction, `character_is_missing_rhs_boundary` in `src/compiler_frontend/tokenizer/lexer.rs` checked LF and CR directly alongside punctuation. The compiler lexer already uses `moth_lexical::is_line_break` for other line-boundary decisions, and that shared predicate owns which characters form logical line breaks. The duplicated LF/CR condition could drift if the shared policy changed. The correction now calls `is_line_break` and keeps punctuation classification local.

#### Counter-explanation tested

A short local character match can look clearer and avoids a helper call. That is not a separate responsibility here: the line-break predicate already exists as the shared policy and the lexer already calls it. The local punctuation list remains appropriate because it is specific to this right-hand-side boundary.

#### Violated contract or cost

This duplicated one logical-line rule in the lexer. A policy change would require coordinated edits and could leave symbolic-boundary handling inconsistent. No measured performance cost is claimed.

#### Root owner

`character_is_missing_rhs_boundary` in `src/compiler_frontend/tokenizer/lexer.rs`, with `moth_lexical::is_line_break` as the shared line-break owner.

#### Suggested correction

Delegate line-break classification to `moth_lexical::is_line_break` and retain only the punctuation test in the local helper.

#### Fix scope and preserved invariants

Resolved in the closeout commit following `8b1229c11`. The helper now combines `is_line_break` with its existing punctuation boundary cases. LF/CR behavior and the comma, parenthesis, bracket, brace and semicolon boundaries remain intact; no broader lexer abstraction or performance claim was added.

#### Required validation

Run the lexer and compiler unit tests that exercise symbolic right-hand-side boundaries, then run clippy for `moth`. The focused `cargo test -q -p moth --lib` run passed with 5,460 tests, and clippy for `moth` was clean. The final `just validate-full` result is recorded in the closeout commit message.

#### Linked findings

AUD-0009-F01 records the shared logical-line policy; AUD-0013-F01 records the related assignment-spacing boundary.

#### Triage record

2026-10-03 — **Accepted.** Keep the shared logical-line rule under `moth-lexical`; remove the local LF/CR duplicate while retaining this helper's distinct punctuation boundaries.

2026-10-03 — **Fixed and closed.** The closeout commit following `8b1229c11` routes `character_is_missing_rhs_boundary` through `moth_lexical::is_line_break`. The focused `moth` tests and clippy passed. No performance improvement is claimed; the final `just validate-full` result is recorded in the closeout commit message.

## Checked and clean

The punctuation boundaries remain local to the lexer helper. `is_line_break` remains the shared authority for logical line breaks, including in the MON/source boundary covered by AUD-0009-F01. No timing or allocation measurement was needed for this ownership correction.

## Limitations

Coverage is partial and limited to one lexer boundary helper and its shared predicate. The external review source-traced the code and did not measure runtime or inspect all tokenizer line-handling paths. This report does not claim a general tokenizer redundancy audit.
