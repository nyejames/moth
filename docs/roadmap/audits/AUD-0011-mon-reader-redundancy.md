# AUD-0011: MON reader trivia-walk redundancy

- State: `complete`
- Kind: `Redundancy`
- Area: `crates.moth_mon.reader` — trivia scanning, constructor payload lookahead and consumed-end state in `crates/moth-mon/src/reader.rs`.
- Coverage: `partial`
- Reviewed: `2026-10`
- Baseline: Branch `feature/mon-crate-extraction`, reviewed tip `be0405afd0f74e593d91310e5ed86f74ed574522`. The external reviewer had neither Cargo nor rustc and source-traced the reader through the connected GitHub reader. The implementer later ran the relevant reader/parity tests on the branch; those were not reviewer-run. Scoped package/parity tests and clippy passed for the fixes.
  Final-tree gate: `just validate-full` passed on the final tree (commits `345205680`, `3c5c51ced`, `0f3439fda` plus the closeout commit that also routes the lexer's remaining inline LF/CR checks and the string-escape newline check through `moth_lexical::is_line_break`): 10/10 feature lanes; 5,453 default `moth` unit tests (5,588 with feature lanes); `moth-mon` and `moth-lexical` tests; 850 `xtask` tests; 2,002 integration cases (2,230 backend executions, 0 failures); all 82 benchmarks passed preflight; the quick perf comparison reported no regression.

## What was inspected

`crates/moth-mon/src/reader.rs`, particularly `skip_space_and_comments`, `payload_opens_after_trivia`, `parse_identifier_value`, `parse_optional_payload` and `last_consumed_end`; related reader tests and the constructor-boundary parity cases. The comparison remained local to the MON reader and its callers.

## Authorities read

`docs/roadmap/audit-guide.md`; `docs/roadmap/audit-kinds/redundancy.md`; `docs/roadmap/audits/README.md`; `docs/roadmap/audit-log.md`; `docs/roadmap/open-audit-findings.md`; `docs/src/docs/language-overview/mon-syntax.mtf`; `docs/src/docs/mon/mon-format.mtf`; `docs/src/developer-docs/style-guide/testing.mtf`; the compiler/build ownership contracts; the originating extraction plan; the supplied `moth-code-review-guide.md`; and repository instructions.

## Existing findings and active plans checked

The audit log and open-findings index were checked. The external review's local redundancy finding R01 was mapped to F01. The construction-boundary correction in AUD-0009-F02 had to settle before changing the probe; AUD-0010-F02 records its boundary and reader regression coverage. The review explicitly treated this as local readability/ownership cleanup, not a measured performance issue.

## Findings

### AUD-0011-F01: MON payload lookahead duplicates its trivia scanner

- State: `closed`
- Kind: `Redundancy`

#### Evidence

`skip_space_and_comments` and `payload_opens_after_trivia` independently recognized whitespace and `--` comments. On a successful payload parse, the probe found `(` across that trivia and the parser then consumed the same trivia again. The probe's non-consuming behavior was useful, but it did not require an independent scanner: a cursor checkpoint could run the owning trivia operation and restore the position when the probe did not commit.

The nearby `last_consumed_end` `Option` also represented an impossible state for a successfully parsed payload, whose end position is positive. This is a local state-shape issue in the same edited reader path, not a performance claim.

#### Counter-explanation tested

A separate non-consuming lookahead can protect the cursor when no payload follows. That behavior is a real distinction, but it does not justify a second whitespace/comment recognizer: checkpoint and restore preserve the no-commit position while reusing the owning scan. Exact cursor positions, spans, comment termination and the newly settled constructor-head boundary still require tests.

#### Violated contract or cost

The reader had two owners for the same trivia rules and repeated the successful-path walk. A future whitespace or comment change could update one scanner but not the other, and the parser then rescanned accepted trivia. The redundant `Option` also allowed an impossible no-end state to propagate past a successful payload.

#### Root owner

The MON reader's trivia operation and payload lookahead in `crates/moth-mon/src/reader.rs`.

#### Suggested correction

Reuse the existing trivia operation under a cursor checkpoint; restore the cursor when the probe does not commit. Once the constructor continuation rule is settled, remove the duplicate scanner and represent a successful payload's end directly rather than as an optional value.

#### Fix scope and preserved invariants

Resolved in commit `345205680`, alongside the accepted constructor-boundary correction. The payload probe reuses the existing trivia walk through a checkpoint, and `last_consumed_end` no longer carries an impossible `Option`. The change stays local to the MON reader; no shared compiler/MON cursor framework or performance claim was introduced.

#### Required validation

Keep tests for exact source spans, keyword openers, comments, cursor restoration on a failed probe and the constructor-head layout. Scoped MON/parity tests and clippy passed; `just validate-full` passed.

#### Linked findings

AUD-0009-F02; AUD-0010-F02.

#### Triage record

2026-10-03 — **Accepted.** Once AUD-0009-F02 settled the constructor-head rule, the duplicate trivia walk had no distinct ownership contract. Authorise a local checkpoint-based reuse while preserving non-commit cursor position, source spans and comment handling; do not introduce a cross-parser cursor abstraction. The nearby impossible `Option` may be removed with this same local edit.

2026-10-03 — **Fixed and closed.** Commit `345205680` removed the duplicate trivia scanner by reusing the owning walk under a checkpoint and removed the impossible `last_consumed_end` `Option`. Exact-span and keyword-opener reader tests, plus the linked constructor-boundary parity tests, passed with scoped package tests and clippy. No performance improvement is claimed; `just validate-full` passed.

## Checked and clean

Separate compiler and MON parsers remain justified by their distinct input and ownership boundaries. This finding does not recommend a shared parser, cross-crate cursor framework or global utility module. The retained checkpoint preserves the useful non-consuming probe behavior without duplicating MON's trivia policy.

## Limitations

Coverage is partial and limited to the MON reader path named above and its focused tests. The external reviewer inspected source without Cargo or rustc and did not execute the recommendation. No runtime or allocation benchmark was run; the issue and correction concern duplicate ownership/readability, not measured performance. Other reader helpers and compiler cursor paths were not audited for redundancy.
