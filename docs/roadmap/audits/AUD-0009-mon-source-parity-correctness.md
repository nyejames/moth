# AUD-0009: MON source parity correctness

- State: `complete`
- Kind: `Correctness`
- Area: `contract.mon_source_parity` — compare the MON reader and schema path in `crates/moth-mon` with compiler tokenization and source construction parsing; shared lexical policy belongs to `crates/moth-lexical`.
- Coverage: `partial`
- Reviewed: `2026-10`
- Baseline: Branch `feature/mon-crate-extraction`, reviewed tip `be0405afd0f74e593d91310e5ed86f74ed574522`. The external reviewer had neither Cargo nor rustc and source-traced the paths through the connected GitHub reader. The implementer later ran the listed reproductions on the branch before fixing them; these were not reviewer-run reproductions. Scoped package/parity tests and clippy passed for the fixes.
  Final-tree gate: `just validate-full` passed on the final tree (commits `345205680`, `3c5c51ced`, `0f3439fda` plus the closeout commit that also routes the lexer's remaining inline LF/CR checks and the string-escape newline check through `moth_lexical::is_line_break`): 10/10 feature lanes; 5,453 default `moth` unit tests (5,588 with feature lanes); `moth-mon` and `moth-lexical` tests; 850 `xtask` tests; 2,002 integration cases (2,230 backend executions, 0 failures); all 82 benchmarks passed preflight; the quick perf comparison reported no regression.

## What was inspected

The MON reader's named-entry recognition, trivia and payload lookahead, reserved-name error construction, schema preparation and error construction; the compiler lexer whitespace dispatch, source nominal and choice constructor dispatch, cursor lookahead and named-entry recognizer; the existing MON syntax-parity fixtures and MON reader/schema tests. The external review also classified the changed-file inventory, extraction ownership, selected numeric changes, MON parse/validation paths, parity harness and changed gate integration. It did not exhaustively audit transitive compiler or backend paths.

## Authorities read

`docs/roadmap/audit-guide.md`; the Correctness, Redundancy and Tests checklists; `docs/src/docs/language-overview/mon-syntax.mtf` (Entries and layout; Shared construction parity); `docs/src/docs/mon/mon-format.mtf` (Reserved user names; Structs and choices; Schemas, defaults and options; Limits and first errors); `docs/compiler-data-layout-design.md` (MON handoff and retained numeric text accounting); `docs/src/docs/collections/hash-maps.mtf`; `docs/src/docs/choices/variant-construction.mtf`; `docs/src/developer-docs/style-guide/testing.mtf`; the compiler/build ownership contracts; the originating extraction plan; the supplied `moth-code-review-guide.md`; and the repository instructions.

## Existing findings and active plans checked

The audit log and open-findings index were checked. The external review's local findings C01–C03 were mapped to this report's F01–F03; its linked Tests and Redundancy findings are recorded in AUD-0010 and AUD-0011. The extraction boundary and its prohibition on incidental source-language expansion were checked. The handover's review finding IDs were local identifiers, not previously reserved repository audit IDs.

## Findings

### AUD-0009-F01: Unicode horizontal whitespace splits MON named entries

- State: `closed`
- Kind: `Correctness`

#### Evidence

The source lexer emits a logical newline only for LF and CR. Other accepted whitespace is discarded without that boundary, so the source named-entry recognizer can see the label and `=` as adjacent tokens. Before the fix, the implementer ran real-file `moth check` and `moth-mon decode` reproductions for `data #= (field<X>= 1)`: source accepted TAB, VT, FF, NEL, U+2028 and U+2029 between `field` and `=`, while source rejected LF under MOTH-RULE-0034. MON rejected VT, FF, NEL, U+2028 and U+2029 as bare names. The external reviewer source-traced the difference but did not execute it.

The finding applies to the boundary rule, not to treating all Unicode whitespace as line breaks. The source parser already treats LF and CR as line boundaries and the other listed whitespace as horizontal for this lookahead.

#### Counter-explanation tested

Treating VT, FF, NEL and Unicode separators as physical line breaks is a plausible language design, but the compiler's whitespace dispatch does not implement it. The extraction preserves source semantics and the format documents no separate MON exception for named-entry whitespace.

#### Violated contract or cost

The shared construction-parity contract requires equivalent source and MON named entries to agree on whether the label and `=` occupy one logical line. MON rejected forms the compiler accepts.

#### Root owner

Shared logical-line policy in `moth-lexical` and MON named-entry lookahead in `crates/moth-mon/src/reader.rs`.

#### Suggested correction

Use one LF/CR logical-line predicate in the compiler lexer and MON reader. Permit a MON label and `=` on one logical line with any horizontal whitespace, without changing source grammar.

#### Fix scope and preserved invariants

Resolved in commit `345205680`: shared `moth_lexical::is_line_break` treats LF and CR as line breaks, CRLF as one break, and other Unicode whitespace as horizontal; the compiler lexer and MON reader use that policy. No source grammar changed. AUD-0010-F02 records the linked boundary coverage.

#### Required validation

The parity boundary matrix covers whitespace before `=`, including each affected control/Unicode character and LF as a rejection control, across first, later, nested, nominal and choice payload fields. Scoped package/parity tests and clippy passed; `just validate-full` passed.

#### Linked findings

AUD-0010-F02.

#### Triage record

2026-10-03 — **Accepted.** Preserve the source's LF/CR line-boundary rule and align MON named-entry lookahead with it. The review's counter-explanation does not justify changing compiler semantics. Accept the linked Tests finding AUD-0010-F02 for independent boundary coverage.

2026-10-03 — **Fixed and closed.** Commit `345205680` introduced the shared `moth_lexical::is_line_break` policy and applied it to MON and compiler comment termination. The implementer's before-fix source/MON reproductions exposed the reported difference. The scoped lexical, MON and parity tests and clippy passed; `just validate-full` passed.

### AUD-0009-F02: MON constructor payload lookahead crosses source line boundaries

- State: `closed`
- Kind: `Correctness`

#### Evidence

MON's payload probe searched across whitespace and line comments for `(`. Source nominal construction instead requires `(` immediately after the type name, and source choice construction checks the token immediately after the variant name; the cursor query does not skip NEWLINE. Before the fix, the implementer reproduced these differences on real files: source rejected `Box<LF>(..)` and `Box -- c<LF>(..)` with MOTH-RULE-0053 and `Theme::Selected<LF>(value = 1)` with MOTH-RULE-0029, while MON accepted all three. The external review independently source-traced the same lookahead gap but did not run the examples.

The MON probe already existed in the base reader before crate extraction; this was a pre-existing gap left uncovered by the new parity boundary, not a compiler regression introduced by extraction.

#### Counter-explanation tested

MON deliberately differs in document framing, schema-supplied identity and quoted text. None grants extra whitespace between a constructor head and its payload. The documented adjacency restriction on `::` headers is a separate rule and does not explain this lookahead.

#### Violated contract or cost

The shared construction contract requires MON and source construction to agree on whether a payload opener follows the head on the same logical line. MON accepted layouts rejected by source.

#### Root owner

MON payload lookahead in `crates/moth-mon/src/reader.rs`, aligned with the compiler's nominal and choice constructor dispatch.

#### Suggested correction

Align MON's constructor-head boundary with source: require `(` after the head on the same logical line with only horizontal whitespace. Keep whitespace inside payloads and after `=` unchanged. Record the MON acceptance tightening rather than widening source syntax without an approved language change.

#### Fix scope and preserved invariants

Resolved in commit `345205680`. MON nominal and choice payload openers now follow the head on the same logical line with only horizontal whitespace. The acceptance tightening is recorded in `docs/src/docs/mon/mon-format.mtf` and `mon-syntax.mtf`; source grammar did not change. The local trivia cleanup is tracked separately in AUD-0011-F01.

#### Required validation

The parity matrix covers no gap, horizontal whitespace, all relevant line breaks, comments and Unicode/control whitespace between constructor heads and payload openers; it covers nominal, choice and nested cases. Reverting the head rule failed eight matrix cases and three reader tests. Scoped package/parity tests and clippy passed; `just validate-full` passed.

#### Linked findings

AUD-0010-F02; AUD-0011-F01.

#### Triage record

2026-10-03 — **Accepted.** Align MON with the existing source boundary rather than silently widening source construction. Treat the pre-extraction age of the mismatch as context, not as a supported MON exception. Accept linked boundary coverage in AUD-0010-F02 and the local reader cleanup in AUD-0011-F01.

2026-10-03 — **Fixed and closed.** Commit `345205680` tightened MON nominal and choice payload lookahead to same-logical-line horizontal whitespace, with the compatibility change recorded in the canonical MON format and syntax documents. Reverting the rule failed eight parity-matrix and three reader tests. Scoped package/parity tests and clippy passed; `just validate-full` passed.

### AUD-0009-F03: Rejected names bypass decoded-byte accounting in diagnostics

- State: `closed`
- Kind: `Correctness`

#### Evidence

`reserved_name_error` interpolated a rejected name into an owned detail string. Its nominal and choice qualifier callers passed no path segment, so the existing path charge did not run before the copy. Schema preparation likewise validated a field name and could format a reserved-name detail before charging its size. The external review traced both paths. Its illustrative bounded-input reproduction used a short schema and a reserved input qualifier thousands of underscores long, below the independent input-byte limit; schema preparation has no document input-byte limit. The implementer's C03 regressions failed against pre-fix code.

#### Counter-explanation tested

Borrowed qualifier recognition need not charge merely to inspect a name. The violation occurs when a rejected name is copied into a diagnostic. That does not imply that fixed diagnostic overhead must fit within the decoded-data budget or require process-wide out-of-memory recovery.

#### Violated contract or cost

MON's decoded-byte accounting must bound variable-size name copies made on rejected-input paths as well as successful value paths. Otherwise input-derived names can create unaccounted diagnostic allocation, and caller-supplied schema names can be copied before their budget charge.

#### Root owner

MON reserved-name error construction and schema field preparation.

#### Suggested correction

Charge a variable-size name before copying it into a path segment or error detail, or use a bounded detail. Apply the same ordering to reserved schema fields while retaining source spans and the established value path.

#### Fix scope and preserved invariants

Resolved in commit `3c5c51ced`: `reserved_name_error` charges every rejected label, variant or qualifier before copying the name into a path or detail; `prepare_fields` charges each field name once before validation, with duplicate field-name charges removed. Valid schemas retain once-per-name accounting. `docs/compiler-data-layout-design.md` now states the rule.

#### Required validation

Reserved label, variant and qualifier cases exceed the remaining budget; reserved schema fields exceed a tiny schema budget; an exact-budget nested and payload schema remains accepted. The implementer reports that these regressions failed before the fix and that scoped MON tests and clippy passed. `just validate-full` passed.

#### Linked findings

AUD-0010-F02 records the linked parity coverage follow-up.

#### Triage record

2026-10-03 — **Accepted.** Charge input-derived names before formatting them into diagnostics, and charge schema-supplied names before validation can format them. Preserve borrowed recognition, original spans, and the existing value path. Accept the linked Tests coverage in AUD-0010-F02.

2026-10-03 — **Fixed and closed.** Commit `3c5c51ced` charges each rejected name before copying it into a path or detail and charges each prepared schema field exactly once before validation. Reserved-name budget regressions failed before the fix; the exact-budget nested/payload case preserves valid accounting. Scoped MON tests and clippy passed; `just validate-full` passed.

## Checked and clean

The review retained deliberate differences in document framing, schema-supplied nominal identity, quoted text and resource limits; these do not grant the whitespace or diagnostic-accounting differences above. Numeric expected values remain independently specified in the parity suite. The compiler keeps declaration shells, call parsing, tokens, IDs, spans and diagnostics local, and the shared lexical crate remains the narrow policy boundary.

## Limitations

Coverage is partial. The review source-traced the named paths using a connected GitHub reader and did not have Cargo or rustc; its reproductions were not executed by the reviewer. The implementer later ran the relevant reproductions and scoped tests before fixes. The review did not exhaustively audit transitive compiler/backend behavior or measure performance. This report records the focused source/MON boundary and selected fixes, not a full compiler or numeric audit.
