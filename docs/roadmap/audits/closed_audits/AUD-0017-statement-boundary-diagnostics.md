# AUD-0017: Statement-boundary and scope-close diagnostics

- State: `complete`
- Kind: `Diagnostics`
- Area: `frontend.statement_boundary_diagnostics` — user diagnostics for statement terminators, scope closers and expression ends, and the diagnostic construction path those reports use
- Coverage: `partial`
- Reviewed: `2026-11`
- Baseline: Source at `c62bbe560`. `just validate` was not run. Empirical diagnostics used a debug `moth` rebuilt with `cargo build --bin moth` (dev profile, finished in 26s) and `./target/debug/moth check` on disposable HTML projects under `./tmp/aud0017`. The existing `target/release/moth` (00:23) predates the top-level stray-closer rejection and was not used as evidence. No active plan owns statement-boundary diagnostics. `docs/docs/design-scope/` and the progress matrix do not defer unclosed-block rejection or these wording gaps. Broader statement recovery remains deferred (`docs/compiler-design-overview.md`, `Ordinary-body diagnostic recovery`); findings below do not ask for that recovery.

## What was inspected

Production owners, read for the closer and expression-end paths rather than every line of each file:

- `src/compiler_frontend/ast/statements/diagnostics.rs` — `unexpected_scope_close`, `unexpected_statement_token`
- `src/compiler_frontend/ast/statements/body_dispatch.rs` — `END` / `EOF` arms of `parse_function_body_statements`
- `src/compiler_frontend/ast/cursor.rs` — `current_span` exhaustion fallback
- `src/compiler_frontend/ast/expressions/parse_expression.rs` — group-end missing `)`, stop-token formatting
- `src/compiler_frontend/ast/expressions/parse_expression_dispatch.rs` — `GroupFrame::opening_span`, delimiter break
- `src/compiler_frontend/ast/expressions/eval_expression/result_type.rs` — binary operand-side pops
- `src/compiler_frontend/headers/header_dispatch.rs` — `capture_function_body_range_from_cursor`
- `src/compiler_frontend/headers/file_parser.rs` — `executable_block_depth`
- `src/compiler_frontend/headers/trait_headers.rs` and `src/compiler_frontend/declaration_syntax/choice.rs` — declaration closers, checked empirically
- `src/compiler_frontend/utilities/token_scan.rs` — `unclosed_delimiter_eof_diagnostic`, `OpenConstruct`, initializer EOF
- `src/compiler_frontend/compiler_messages/render/syntax.rs` and `render/payload.rs` — closer and EOF wording
- `src/compiler_frontend/compiler_messages/diagnostics/families/templates.rs` — `missing_closing_bracket` opener label
- `src/compiler_frontend/syntax_errors/statement_position.rs` — common statement-position mistakes
- `tests/cases/diagnostic_top_level_stray_scope_close/`
- `tests/cases/grouped_dangling_operator_numeric_chain_rejected/`
- `src/compiler_frontend/ast/expressions/tests/eval_expression_tests.rs` — `malformed_groups_keep_group_operand_diagnostics`

Context read, not audited: diagnostic schema and failure lanes, ordinary-body recovery, style-guide diagnostic rules, block/statement/loop/function closer contracts.

## Authorities read

- `docs/docs/language-overview/blocks-and-statements.mtf` — `:` opens a block, `;` closes it, a semicolon does not terminate an ordinary statement, indentation does not define scope
- `docs/docs/branching/statement-if.mtf`, `docs/docs/loops/conditional-loops.mtf`, `docs/docs/functions/function-declarations.mtf` — one final semicolon closes the construct
- `docs/compiler-design-overview.md` — `Diagnostics and deterministic identity`, `Ordinary-body diagnostic recovery`
- `docs/compiler-data-layout-design.md` — `Failure architecture`, diagnostic draft/record flow, secondary labels
- `docs/developer-docs/style-guide/style-guide.mtf` — `Diagnostics`
- `docs/roadmap/audit-kinds/diagnostics.md`

## Existing findings and active plans checked

Open findings had no unresolved candidates. AUD-0005 and AUD-0014 touch tokenizer spacing and line breaks, not block closers. No roadmap plan owns this boundary. Deferred statement recovery was checked and is not used below as a reason to accept a missing or misleading diagnostic.

## Findings

### AUD-0017-F01: Unclosed blocks in the entry start body are accepted and swallow later statements

- State: `closed`
- Kind: `Correctness`

#### Evidence

`blocks-and-statements.mtf` says `;` closes the active block and indentation does not define scope. A function body that omits `;` is rejected. The same omission in the implicit entry start is accepted, and tokens after the unclosed block are parsed inside it.

Disposable `@entry.moth`, `./target/debug/moth check --terse`:

```text
ready ~= false
if ready:
    secret = "hidden"
io.line(secret)
```

Accepted, no diagnostic. The same source with a `;` after `secret = "hidden"` reports `MOTH-RULE-0034` at `io.line(secret)`: `Unknown value name 'secret'`.

```text
count ~= 0
loop count < 3:
    item = 1
io.line([: [item]])
```

Accepted. With a `;` after `item = 1`, the use of `item` reports `MOTH-RULE-0034`.

```text
count ~= 0
loop count < 3:
    ~count += 1
continue
```

Accepted. With a `;` before `continue`, the same `continue` reports `MOTH-RULE-0042`: `Continue statements can only be used inside loops.`

The same unclosed `if` inside a function is rejected:

```text
compute ||:
    if ready:
        io.line("yes")
    io.line("no")
;
```

`MOTH-SYNTAX-0017` at the final `;`, `Unexpected end of file, expected ';'`.

`parse_function_body_statements` treats `TokenTag::EOF` as a successful end of the body (`body_dispatch.rs`). Header capture of a function body does not: `capture_function_body_range_from_cursor` counts opened and closed scopes and reports EOF when they do not meet. `file_parser.rs` tracks `executable_block_depth` only to tell headers from start-body tokens. It does not diagnose a non-zero depth at EOF. Start-body statements therefore keep an open `if` or `loop` and parse what follows as part of that block.

#### Counter-explanation tested

**"The implicit start has no authored opener, so EOF closes its nested blocks."** Rejected. If EOF closed the `if`, `secret` would be out of scope, as it is when `;` is written. The unclosed form resolves `secret` and accepts `continue` after the loop. EOF does not close the block; parsing stops and the open block is published.

**"This is the deferred broader statement recovery."** Rejected. That deferral is about continuing after a reported error and not rescanning for a function boundary. It does not accept an unclosed block. No design-scope or progress-matrix row defers start-body closer checks.

Disproof would be a canonical rule that entry-start nested blocks are closed by EOF, or a test that intentionally accepts the unclosed form above. Neither was found.

#### Violated contract or cost

`blocks-and-statements.mtf`: `;` closes the active block. Omitting it changes which statements run and which names are in scope, with no diagnostic. A later reader cannot tell a forgotten closer from intentional nesting.

#### Root owner

`parse_function_body_statements` EOF arm, for bodies that are not pre-balanced by header capture. Function bodies are pre-balanced; the entry start body is not. `file_parser.rs` already counts `executable_block_depth` and does not report it.

#### Suggested correction

Non-authorising. Reject an entry start body that reaches EOF with a block still open, using the same closer identity as a function body and naming the open block. Do not invent an implicit closer, and do not add statement-by-statement recovery.

#### Fix scope and preserved invariants

Bounded to start-body closer accounting. Preserve: no required closer for the synthesized start itself; legality of a block that is closed with `;`; function-body capture; the existing top-level stray-`;` rejection (`MOTH-SYNTAX-0030`). Do not rescan function boundaries.

#### Required validation

A `tests/cases/` pair: unclosed entry `if` and `loop` must fail with a stable closer code and a span on the opener or the missing closer, and the closed forms must keep today's scope errors. `just validate`.

#### Linked findings

None. Filed in the Correctness lane from this Diagnostics run because legality is wrong, not merely the wording.

### AUD-0017-F02: A missing block closer is reported as unexpected end of file and does not name the open block

- State: `closed`
- Kind: `Diagnostics`

#### Evidence

Where a missing `;` is rejected, the report is `MOTH-SYNTAX-0017` at the truncation point, with no secondary label on the block that is still open.

Missing function closer:

```text
compute || -> I32:
    return 1
```

```text
E|MOTH-SYNTAX-0017|@entry.moth|2:13|Unexpected end of file, expected ';'
```

The caret is the end of `return 1`. The opening `:` is not labelled.

Inner block steals the function closer:

```text
ready ~= true
compute || -> I32:
    if ready:
        return 1
    return 0
;
```

```text
E|MOTH-SYNTAX-0017|@entry.moth|7:2|Unexpected end of file, expected ';'
```

Line 7 is the `;` the user wrote. The caret is the column after it. The message asks for `;` and does not say that this `;` closed the `if` on line 4, leaving `compute` open.

Unclosed trait declaration:

```text
NAMED must:
    name |~This| -> String
```

```text
E|MOTH-SYNTAX-0017|@entry.moth|2:27|Unexpected end of file, expected ';'
```

No label on `must:`.

A missing template `]` does the opposite. `missing_closing_bracket` (`templates.rs`) puts the primary span on EOF and a `TemplateStartsHere` label on the opening `[`.

The closer scans already walk the openers and then drop that context:

- `capture_function_body_range_from_cursor` counts `scopes_opened` / `scopes_closed` and, on EOF, calls `unclosed_delimiter_eof_diagnostic` with fallback `";"`. It does not keep opener spans.
- `unclosed_delimiter_eof_diagnostic` finds the innermost unclosed template, `(`, or `{` and keeps the opener span only for a template. For `(` and `{` it keeps the expected character and discards the span. It does not track `:` / `;` at all.
- Initializer `OpenConstruct::ValueIfBlock` and `CatchBlock` expect `";"` but store no span. EOF reports `unexpected_end_of_file` at the EOF token only.

#### Counter-explanation tested

**"Expected ';' at EOF is the accurate report, and an opener label would only add noise."** Rejected for the stolen-closer case: the user did write `;`, and the caret sits on the following column asking for another one. The template path already treats the opener as required context, not noise. A file with several open blocks cannot be repaired from "expected ';'" at EOF alone.

**"The header scanner must not retain opener spans because recovery is deferred."** Rejected. Recording the open colon during the scan that already visits it is diagnostic context, not statement recovery. The scan already distinguishes `if` / `loop` / `catch` / match / region colons so it does not invent scopes.

Disproof would be a rendered secondary label on the open `:` for the sources above, or a canonical rule that closer diagnostics must not name the opener.

#### Violated contract or cost

Style-guide diagnostics: exact span when authored source owns the error, and related-site context where it explains the failure. Diagnostics checklist: state what failed, distinguish expected from found, suggest a stable correction. `MOTH-SYNTAX-0017` plus `";"` does not say which block is open or which `;` closed an inner block.

#### Root owner

`capture_function_body_range_from_cursor` for function bodies, `unclosed_delimiter_eof_diagnostic` for the shared EOF fallback, and the initializer `OpenConstruct` stack for value-producing `if` / `catch` closers. All three can see the opener and none of them put it on the draft. `trait_headers.rs` has the same EOF-expected-`;` shape without an opener label.

#### Suggested correction

Non-authorising. Keep the rejection. Put a secondary label on the still-open block's colon, and when a `;` closed an inner block while an outer one remains open, say so at that `;` instead of asking for `;` at the following EOF column. Follow the `missing_closing_bracket` label shape. Do not add statement recovery or a second scan of source text.

#### Fix scope and preserved invariants

Diagnostic construction and label text only, on the scans that already balance these delimiters. Preserve `MOTH-SYNTAX-0017` for genuine unexpected EOF where no closer is known, template-opener labels, and the match-arm colon suppression already documented in the function-body scanner. A stolen closer must not be reparsed as a statement terminator.

#### Required validation

Focused header and initializer tests plus disposable `moth check` of a missing function `;`, a stolen `if` closer, and a missing trait `;`. Each must name the open construct. `just validate`.

#### Linked findings

AUD-0017-F01 is the start-body case this report does not cover, because that case is accepted rather than reported badly. AUD-0017-F04 is the same missing-opener failure for `)`, with a fabricated file-start span.

### AUD-0017-F03: A semicolon used as a statement terminator is not identified, so the follow-on error blames the wrong token

- State: `rejected`
- Kind: `Diagnostics`

#### Evidence

`;` closes the active block. A semicolon after one statement inside a function therefore closes the function. That is the right legality. The diagnostic the user then sees does not name that semicolon or the rule.

```text
compute ||:
    total = 1 + 2;
    io.line("still inside?")
;
```

```text
Invalid statement position [MOTH-SYNTAX-0030]
  --> @entry.moth:4:1
  4 | ;
      ^
  Unexpected ';' at the top level. It does not close any open block.
```

The blamed `;` is the one the user wrote to close the function. The `;` after `2`, which closed it, is not labelled. The message does not say that `;` ends the block rather than the statement.

```text
compute || -> I32:
    total = 1 + 2;
    return total
;
```

```text
E|MOTH-RULE-0042|@entry.moth|3:5|Return statements can only be used inside functions.
```

The semicolon is not mentioned. The error names a consequence of the early close.

`statement_position.rs` has typed common mistakes for `==`, `let`, `//`, `match`, and signature `()`. It has no case for `;` used as a statement terminator. `unexpected_scope_close` top-level wording says the `;` closes nothing; it does not say an earlier `;` already closed the function.

A one-statement function closed by `;` is legal and must stay silent. The gap is only the follow-on diagnostic after that `;` has already closed the function and later tokens were authored as if the function were still open.

#### Counter-explanation tested

**"The top-level `;` really closes nothing, so MOTH-SYNTAX-0030 is the right report."** The token it points at does not close a block. That does not explain why. The stable cause is the earlier `;`, which the header scanner already treated as the function closer. A user who put a terminator after the assignment and a closer after the function is told their closer is stray.

**"Recognising a statement terminator would require the deferred statement recovery."** Rejected. The header scanner already decided which `;` closed the function. The start-body diagnostic can carry that span as a related label without reparsing the body or changing what is accepted.

Disproof would be a related label on the function-closing `;`, or wording that states the block-closer rule, on the sources above.

#### Violated contract or cost

Diagnostics checklist: explain why, name the relevant token, suggest the stable correction. Style guide: related-site context where it adds information. The current reports send the user to edit `return` or the final `;`, not the semicolon that closed the function.

#### Root owner

`capture_function_body_range_from_cursor` decides the closer and retains no diagnostic fact for it. `body_dispatch.rs` then reports a later `END` in `ContextKind::Module` as `unexpected_scope_close` top-level, or a later `return` is rejected as outside a function, with no access to the closer span.

#### Suggested correction

Non-authorising. When a function closer is followed by start-body tokens that produce a diagnostic, label the `;` that closed the function and state that `;` closes the block, not the statement. Do not warn on a legal one-statement function. Do not recover the following statements back into the function.

#### Fix scope and preserved invariants

The function-closer span and the start-body diagnostic that already fires. Preserve acceptance of `name ||: statement ;`, the `MOTH-SYNTAX-0030` rejection of a `;` that closes nothing, and match-arm semicolon identity (`MOTH-SYNTAX-0028`).

#### Required validation

Disposable or `tests/cases/` coverage for a statement-terminator `;` followed by another statement and a final `;`, and for the same shape whose next statement is `return`. A one-statement function must still check clean. `just validate`.

#### Linked findings

None.

### AUD-0017-F04: An unclosed parenthesis at the end of an expression is reported at the start of the file

- State: `closed`
- Kind: `Diagnostics`

#### Evidence

```text
-- opener is not line 1

compute || -> I32:
    return (1 + 2
;
```

```text
Missing closing delimiter [MOTH-SYNTAX-0012]
  --> @entry.moth:1:1
  1 | -- opener is not line 1
      ^
  Missing closing delimiter ')'
```

The `(` is on line 4. The caret is byte 0 of the file.

`collect_expression_fragment_into` reports the missing `)` only after the cursor has left the token window, and it passes `current_span()` (`parse_expression.rs`). `AstCursor::current_span` returns `LocalSpan::source_start()` when the cursor is exhausted and no synthetic EOF span is installed (`cursor.rs`). `GroupFrame` already stores `opening_span` when the `(` is pushed (`parse_expression_dispatch.rs`) and that span is not used on this path.

The same fallback can attach any end-of-window diagnostic that calls `current_span()` after exhaustion. This run proved it for the missing-`)` path above.

#### Counter-explanation tested

**"The span is the function, and column 1 is a stand-in because the body range has no leftover token."** Rejected. Line 1 of the repro is a comment, not the function. The group frame holds the `(` span. A missing token is not a reason to invent the file start; the template EOF path labels the opener and points the primary span at the truncation.

Disproof would be a caret on the `(` or on the truncation token for the source above.

#### Violated contract or cost

Diagnostics checklist: a user-facing diagnostic whose authored source owns the error has a useful `SourceSpan` for the exact byte range. A file-start span is acceptable only when no narrower owner exists. Here the opener span exists and is discarded. The rendered caret claims the comment is the unclosed parenthesis.

#### Root owner

`collect_expression_fragment_into`'s exhausted-cursor `missing_closing_delimiter` call, together with `AstCursor::current_span`'s `source_start` fallback.

#### Suggested correction

Non-authorising. Report the innermost `GroupFrame::opening_span`, with the truncation token as the primary or the label, matching `missing_closing_bracket`. Do not use `current_span()` once the cursor is exhausted unless a synthetic EOF span was installed for that window.

#### Fix scope and preserved invariants

The group-exhaustion diagnostic and the cursor fallback it depends on. Preserve in-window spans, and do not retarget diagnostics that still have a real current token.

#### Required validation

A parse test or disposable `moth check` of an unclosed `(` after a leading comment, asserting the span is the `(` or the truncation, not byte 0. `just validate`.

#### Linked findings

AUD-0017-F02. `unclosed_delimiter_eof_diagnostic` computes a `(` opener span and discards it on the header EOF path; this finding is the expression-parser path that fabricates file start instead.

### AUD-0017-F05: Expression-end expected tokens are rendered as TokenTag debug names

- State: `closed`
- Kind: `Diagnostics`

#### Evidence

```text
compute ||:
    values = {1, 2
;
```

```text
E|MOTH-SYNTAX-0017|@entry.moth|2:18|Unexpected end of file, expected 'TokenTag(25), TokenTag(21), TokenTag(16)'
```

`create_expression_until_with_policy` and the earlier stop-token window in `parse_expression.rs` format each stop token with `{token:?}` and intern that string as the expected delimiter of `unexpected_end_of_file`. `TokenTag` is a `u16` newtype (`tokenizer/schema.rs`) with no user-facing display. The renderer prints the interned string unchanged (`unexpected_end_of_file_message`).

The same `{token:?}` join is at both window-validation sites (the seeded fragment path and `create_expression_until_with_policy`).

#### Counter-explanation tested

**"The stop list is diagnostic payload, and Debug is good enough until a renderer exists."** Rejected. The string is the user-facing expected-delimiter fact. The checklist forbids internal compiler names in the message. A user cannot correct `TokenTag(25)`.

Disproof would be a message naming `}` or the other real stop spellings for the source above.

#### Violated contract or cost

Diagnostics checklist, message content: avoid internal compiler names and indexes. The owner knows the stop tags and still publishes their Debug form.

#### Root owner

The stop-token formatting in `parse_expression.rs`, which stores Debug text in a `StringFact` the renderer is required not to reinterpret.

#### Suggested correction

Non-authorising. Map stop tags to the source spellings the user would type before interning the expected-delimiter fact. If several stops are legal, name them as source tokens, not as one Debug list. Do not teach the renderer to parse `TokenTag(N)`.

#### Fix scope and preserved invariants

Those formatting sites only. Preserve `MOTH-SYNTAX-0017` and the existing expected-delimiter fact. Do not change which token stops the expression.

#### Required validation

Disposable `moth check` of the unclosed `{` source must not contain `TokenTag`. A unit test on the formatted delimiter is not sufficient by itself. `just validate`.

#### Linked findings

None.

### AUD-0017-F06: Ending a binary expression reports the missing operand on the wrong side

- State: `closed`
- Kind: `Diagnostics`

#### Evidence

```text
compute || -> I32:
    return 1 +
;
```

```text
E|MOTH-SYNTAX-0024|@entry.moth|2:14|Missing left-hand operand for operator '+'.
```

The `+` has its left operand. The right operand is missing because the newline and the following `;` ended the expression (`dispatch_delimiter_token` breaks out of the expression). The caret on `+` is the right token; the side is wrong.

`result_type.rs` pops the right operand first. If that pop succeeds and the left pop fails, it reports `BinaryLeft`. One present operand is therefore always described as the right-hand side, whether the source was `1 +` or `+ 1`.

`tests/cases/grouped_dangling_operator_numeric_chain_rejected/input/@entry.moth` is `value = (((1 + ) + 1) + 1)` and `expect.toml` requires `Missing left-hand operand for operator '+'`. `malformed_groups_keep_group_operand_diagnostics` pins `OperatorOperandPosition::BinaryLeft` for `value = 2 * (1 + )`.

#### Counter-explanation tested

**"The stack pop is the definition of the side, and the tests record that definition."** The pop order is an implementation artefact. For `1 +` and `1 + )` the authored operand is on the left. The message states a false fact about the source. Pinning that fact is why AUD-0017-F07 exists; it is not a reason the message is right.

**"The `;` is incidental and this is a general operator bug, outside statement boundaries."** The user-visible failure is how an expression reports its end. The same misclassification fires when `)` or `;` ends the expression. It belongs in this area.

Disproof would be source in which `1 +` has no left operand, or a canonical rule that a dangling infix operator is reported as missing its left operand.

#### Violated contract or cost

Diagnostics checklist: distinguish expected from found, and do not state a false correction. A user told the left operand is missing will edit `1`, not supply the right operand.

#### Root owner

The binary arm of expression result typing in `result_type.rs`. It cannot tell a missing left from a missing right once a single operand has been popped as the right.

#### Suggested correction

Non-authorising. Classify a binary operator with one preceding operand as missing its right operand, and a binary operator with no preceding operand as missing its left. Keep the operator span. Update the pins named in AUD-0017-F07 in the same change.

#### Fix scope and preserved invariants

Operand-side selection only. Preserve `MOTH-SYNTAX-0024`, the operator span, and the unary missing-operand path. Do not change expression acceptance.

#### Required validation

The two pinned fixtures after F07 is accepted, plus `return 1 +` ended by `;` reporting the right-hand side. `just validate`.

#### Linked findings

AUD-0017-F07.

### AUD-0017-F07: Dangling-operator tests pin the wrong operand side

- State: `closed`
- Kind: `Tests`

#### Evidence

`src/compiler_frontend/ast/expressions/tests/eval_expression_tests.rs` `malformed_groups_keep_group_operand_diagnostics` asserts `OperatorOperandPosition::BinaryLeft` for `value = 2 * (1 + )`.

`tests/cases/grouped_dangling_operator_numeric_chain_rejected/expect.toml` requires the message `Missing left-hand operand for operator '+'` for `(((1 + ) + 1) + 1)`.

Both sources have a left operand. A fix of AUD-0017-F06 cannot land while these pins remain.

#### Counter-explanation tested

**"The pins protect the implementation's pop order, which is the contract."** They protect a false user-facing side. The useful contract in the unit test is that the diagnostic stays on the dangling `+` inside the group rather than moving to the outer `*`. That assertion can stay. The side assertion cannot.

Disproof would be a language rule that `1 +` is missing its left operand.

#### Violated contract or cost

A failure fixture must prove the contract, not freeze a wrong message. These pins would reject the correction.

#### Root owner

The two fixtures named above.

#### Suggested correction

Non-authorising. Expect the right-hand operand message and `BinaryRight` for `1 +` / `1 + )`. Keep the assertion that the primary span is the inner `+`.

#### Fix scope and preserved invariants

Only those expectations. Do not weaken the span or the code (`MOTH-SYNTAX-0024`).

#### Required validation

The updated unit test and HTML case, then `just validate`. Lands with AUD-0017-F06.

#### Linked findings

AUD-0017-F06.

## Triage (2026-11-13)

Source review against `fd695d18b`, which retains the audited `c62bbe560` implementation. No compiler binary or Rust toolchain was available for fresh reproduction. The original disposable probes remain the executable evidence. These decisions authorise bounded investigation, not a patch design.

- **F01 accepted.** `body_dispatch.rs` accepts EOF for any statement context, while entry start capture tracks `executable_block_depth` only to distinguish nested runtime items. No remaining closer check exists. Preserve implicit start's own EOF termination while requiring explicit nested blocks to close.
- **F02 accepted.** Function header capture knows scope depth but retains no colon spans. The common delimiter scanner retains an opener only for templates. Improve diagnostics at the owning scans, without inventing a source-wide rescan. The proposed special wording for a closer consumed by an inner block is contingent on being able to identify that closer precisely.
- **F03 rejected.** Both reported tokens are genuinely invalid in their parsed contexts: the first `;` legally ends the function and a later top-level `;` closes no active block, while `return` outside a function is invalid. Moth uses explicit delimiters and permits top-level executable statements after a function. Nothing in the source proves that the earlier semicolon was intended as a statement terminator rather than the function closer. Attaching blame to it would guess intent and can mislead for independently authored top-level errors. The existing `MOTH-SYNTAX-0030` and `MOTH-RULE-0042` reports correctly identify the first illegal token. This does not block future diagnostic improvements supported by unambiguous evidence.
- **F04 accepted.** Exhausted `AstCursor::current_span` falls back to byte zero while `GroupFrame` retains the opening `(` span. Repair the group-exhaustion diagnostic rather than changing the global cursor fallback for unrelated callers.
- **F05 accepted.** Both expression-boundary paths interpolate `TokenTag` through `Debug`, and the rendered error has no translation layer. Use one source-spelling formatter owned by expression parsing if a current helper can be reused.
- **F06 accepted.** The binary RPN consumer pops the sole authored left operand into its right slot, then reports a missing left operand. Keep operand-side classification near the expression owner that can distinguish the authored side, without changing evaluation order.
- **F07 accepted, linked to F06.** The two assertions expressly freeze the wrong side and would block its correction. Retain their operator-span and diagnostic-code checks.

## Resolution (2026-11-13)

**F01 fixed.** The EOF arm of `parse_function_body_statements` now owns the closer check for authored nested blocks. Each nested-body producer passes its block colon: statement `if`/`else` and option-capture `if`, every loop form, value-producing `if` blocks, and block-form statement `catch` handlers. EOF inside one of those bodies reports `MOTH-SYNTAX-0017` expected `;`, with an `unclosed block starts here` label on the colon. The compiler-owned entry start, header-balanced function bodies and match arms pass no opener and keep their existing EOF handling. An unclosed entry block therefore no longer swallows later statements into its scope. Closed blocks keep the existing `MOTH-RULE-0034` and `MOTH-RULE-0042` reports. The header-side `executable_block_depth` was considered and not used: a check there would preempt existing, more precise match, `else:` and initializer diagnostics. The AST owner already holds the opener span and fires only where parsing used to accept the source.

**F02 fixed.** `MOTH-SYNTAX-0017` gains three secondary-label roles: `UnclosedBlockStartsHere`, `UnclosedDelimiterStartsHere` and `InnerBlockCloser`. Header function-body capture now keeps a stack of open-scope colon spans in place of its open and close counters, and records the last top-level `;` that closed a nested scope. At EOF it reports the unmatched `(` or `{` when one exists. Otherwise it labels the innermost open colon. The `this ';' closed the inner block` label appears only when the function body is the sole remaining scope, the only case where that closer is identified exactly, as Triage requires. `unclosed_delimiter_eof_diagnostic` now takes the opener the existing scan found, so `(` and `{` openers are labelled at no rescan cost. That covers signature-member defaults and AUD-0018's source-directive operand capture. The initializer open-construct scan records each opener's span as it visits it. Value-`if` and `catch` blocks label their colon, and template openers keep `MOTH-SYNTAX-0022`. Trait declarations label the `must:` colon. The kind-only `innermost_open_construct` helper and its two unit tests were removed. Existing `eof_in_*_reports_innermost_*` tests still cover innermost precedence. Focused tests now bind each label role to its site, because integration secondary-label assertions bind only the site.

**F04 fixed.** The exhausted-window branch of `collect_expression_fragment_into` reports the innermost `GroupFrame::opening_span` instead of `AstCursor::current_span()`, whose exhausted fallback is byte 0. The global cursor fallback is unchanged. After AUD-0019, the report's original repro is rejected earlier by nesting-aware function-body capture, which reports the missing `)` at EOF. Both implementer and verifier probed widely, including with a debugger breakpoint, and found no user source that still reaches the branch. It is now a defensive owner. A focused bounded-view test ends a window mid-group with its opener away from byte 0. That test fails on the old code.

**F05 fixed.** A single `stop_token_spellings` formatter, owned by expression parsing, renders stop tags through the schema-owned `TokenTag::descriptor().text()`. All four EOF sites in the two expression-boundary paths use it. `value = if 1` now reports `expected 'then, :'` where it used to report `expected 'TokenTag(75), TokenTag(23)'`.

**F06 fixed.** `ExpressionRpnItem::Operator` now records `preceded_by_operand`, the authored fact the RPN consumer can no longer recover. It is set where operators are pushed, and compound assignment sets it to `true`. One helper, `binary_missing_operand_position`, decides the missing side for both the binary arm and the lone-operator fast path. `return 1 +` now reports `Missing right-hand operand`, `* 1` reports left, and unary diagnostics are unchanged.

**F07 fixed.** `malformed_groups_keep_group_operand_diagnostics` now expects `BinaryRight` for `2 * (1 + )` and gains a `( * )` → `BinaryLeft` block. `grouped_dangling_operator_numeric_chain_rejected` now expects `Missing right-hand operand`. Operator spans and `MOTH-SYNTAX-0024` assertions are retained.

**F03** remains rejected as triaged and was not changed.

**F01, F02 and F04–F07 verified and closed.** Independent verifiers, not the implementers, reviewed each lane:

- **F06/F07:** two rounds. Round 1 found that the lone-operator fast path still hard-coded `BinaryRight`. It now uses the shared helper.
- **F04/F05:** one clean round. Three nits were noted and not acted on. The unreachable `let Some(end_tag) else` branches are pre-existing. The prose spellings for `newline` and `end of file` cannot reach these sites today. `'then' or ':'` would read better than `'then, :'`.
- **F01/F02:** three rounds. Round 1 found that unclosed entry-start `catch:` handlers were still accepted. It also found untested header label roles, a trailing manifest newline, a missing progress-matrix clause and over-visible helpers. Round 2 found two problems: no regression test for the `InnerBlockCloser` precision guard, and an unreachable `None` arm in the trait-header opener. All were fixed.

A final verifier reviewed the combined tree: base `f62e4f26b`, with the F04–F07 lanes and the F01/F02 diff on top. It returned clean after checking the manifest reconciliation, the absence of cross-lane interaction and the earlier lanes' repros. It ran the full library suite (7035 passed), the full integration suite (3409/3409), clippy and the format check. On its advice, the coordinator then reworded two fixture comments so they describe the observable contract, which is rejection. The coordinator's `just validate` result is recorded in the closeout commit message. Statement-boundary diagnostics changed materially, so the audit-log row is marked stale.

## Checked and clean

- A top-level `;` that closes no block is rejected as `MOTH-SYNTAX-0030` / `invalid_statement_position.unexpected_scope_close_at_top_level`, with wording that names `;`. Confirmed on the current debug binary against `tests/cases/diagnostic_top_level_stray_scope_close`. The stale release binary still accepted that fixture; that is a binary skew, not current source.
- Match arms have their own closer rule and diagnostic: `MOTH-SYNTAX-0028`, `invalid_match_arm.semicolon_delimiter`, telling the user not to close arms with semicolons.
- Missing template `]` labels the opening bracket. That path is the pattern the block-closer reports lack, not a defect found here.
- Common statement-position mistakes (`==`, `let`, `//`, `match`, signature `()`) are typed reasons with source-token wording. The hole is the statement-terminator semicolon, not that mechanism.
- Function, trait, and value-producing-`if` omission of `;` is rejected. The defect there is context, filed as F02, not acceptance.
- `dispatch_delimiter_token` ending an expression at `;` is the right boundary. The defects are the missing-`)` span, the Debug stop list, and the operand side, not the decision to stop.

## Limitations

Partial. Not inspected exhaustively:

- Signature-member, generic-parameter, and other bare `unexpected_end_of_file(None, …)` sites. Several render `Unexpected end of file` with no expected token. They were not reproduced and are not findings of this run.
- `UnexpectedScopeCloseInExpression` and `UnexpectedScopeCloseInTemplate`. The arms exist in `body_dispatch.rs`. Production statement-dispatch callers use function, branch, loop, match-arm, and module contexts. No user path to those two arms was found.
- Type, borrow, HIR, backend, and build diagnostics outside this boundary.
- Whether choice-closer `expected '|'` at EOF should label the opening `=` or `|`. One choice probe showed `MOTH-SYNTAX-0017` at the last token and was not filed separately.
- `just validate` was not run. Disposable checks are not a substitute for it.
