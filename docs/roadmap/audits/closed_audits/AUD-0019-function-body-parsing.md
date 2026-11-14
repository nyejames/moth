# AUD-0019: Function body parsing

- State: `complete`
- Kind: `Correctness`
- Area: `frontend.function_body_parsing` — statement parsing inside function bodies after the header scanner captures the body range, including nested block closers and whether later statements are absorbed by an unclosed inner block
- Coverage: `partial`
- Reviewed: `2026-11`
- Baseline: Source at `c62bbe560`. `just validate` was not run. Empirical checks used `./target/debug/moth` 0.1.12 (`moth check --terse` on disposable HTML projects under `./tmp/aud0019`). No active plan owns function-body capture. Declared regions remain deferred and were not treated as missing.

Match-arm bodies, multi-bind statements and generic or const function bodies were not read exhaustively. Receiver methods use the same capture function and were not probed separately.

## What was inspected

- `src/compiler_frontend/headers/header_dispatch.rs` — `capture_function_body_range_from_cursor` and its call after signature parsing
- `src/compiler_frontend/ast/statements/body_dispatch.rs` — function-body statement loop, `END` and `ELSE` termination
- `src/compiler_frontend/ast/statements/branching.rs` — statement `if` / `else` handoff and same-line `else if` rejection
- `src/compiler_frontend/tokenizer/storage.rs` — token-range end is exclusive
- `src/compiler_frontend/tokenizer/schema.rs` — `if` does not continue an expression; comparison operators do

Context read, not audited: match-arm construction past the closer, loop-header classification, terminality, and the entry start body already covered by AUD-0017.

## Authorities read

- `docs/docs/language-overview/blocks-and-statements.mtf` — `:` opens a block, `;` closes the active block, and the earlier semicolon closes an `if` while the final semicolon closes the function
- `docs/docs/functions/function-declarations.mtf` — `:` opens the body and `;` closes it
- `docs/docs/branching/statement-if.mtf` — one semicolon closes an `if` / `else`; same-line `else if` is not syntax
- `docs/docs/branching/pattern-matching.mtf` — one semicolon closes a match; arms do not use their own semicolons
- `docs/docs/branching/value-producing-if.mtf` — block form uses the same colon and semicolon
- `docs/docs/loops/conditional-loops.mtf` and `range-loops-basic.mtf` — `loop ... :` ... `;`
- `docs/compiler-design-overview.md` — a function body is a bounded token range; AST does not rescan for the boundary
- `docs/docs/progress/@entry.moth` — functions are partial, statement `if` is supported, declared regions are deferred
- `docs/roadmap/audit-kinds/correctness.md`

## Existing findings and active plans checked

AUD-0017-F01 already owns silent acceptance of an unclosed block in the entry start body. AUD-0017-F02 already owns a missing function closer, and an inner block that steals the function closer, being reported as unexpected end of file without naming the open block. AUD-0017-F04 owns the file-start span when a genuinely unclosed `)` exhausts an expression window. No roadmap plan owns this capture loop. Those findings were not refiled.

## Findings

### AUD-0019-F01: A semicolon inside a call ends the function body range

- State: `closed`
- Kind: `Diagnostics`

#### Evidence

`blocks-and-statements.mtf` says `;` closes the active block and does not terminate an ordinary statement. A semicolon inside a call is not a block closer.

```text
show ||:
    io.line("x"; "y")
;
```

```text
E|MOTH-SYNTAX-0002|@entry.moth|1:1|Unexpected token end of file.
```

`capture_function_body_range_from_cursor` starts at one open scope and increments `scopes_closed` on every `TokenTag::END`. It does not consult `NestingDepth`, which is stepped only after that decision. The first semicolon makes the counts equal, so `body_end` is that token's index and the loop stops. `TokenRange` end is exclusive, so the semicolon is not in the range handed to AST, and neither is the `)` or the function's own closer. The statement parser then sees the end of its window while the call is open and reports end of file. The caret is byte 0, not the semicolon.

The same shape with an unknown callee reports only the unknown name, so the semicolon is still not identified:

```text
show || -> I32:
    return helper(1; 2)
;
```

```text
E|MOTH-RULE-0034|@entry.moth|2:12|Unknown value name 'helper'.
```

#### Counter-explanation tested

**"This is AUD-0017-F04: any unclosed call reports at the file start."** F04's proved path is `MOTH-SYNTAX-0012` for a missing `)` that is still inside the body window. Here the window ends at the inner semicolon, so AST never receives that token or the `)`. The rendered code is `MOTH-SYNTAX-0002`. Repairing the exhausted-cursor span would still say end of file at the truncation, because the semicolon is outside the range.

**"A semicolon always closes the function, so ending the range there is the language rule."** Rejected. The contract closes the active block. The active block is the function; the semicolon is inside the call's parentheses. A semicolon in template text does not end the range: `io.line([: Hello; world])` followed by another statement in the same function is accepted.

Disproof would be a diagnostic on the semicolon itself, with the `)` and the function's own `;` still inside the captured body.

#### Violated contract or cost

Right rejection, wrong identity and span. The user is told the file ended at byte 0. The authored semicolon, which the scanner treated as the function closer, is not mentioned. Tokens after it are not parsed as part of the function, so a later syntax error in that function can be dropped behind this end-of-file report.

#### Root owner

The `TokenTag::END` arm of `capture_function_body_range_from_cursor` in `src/compiler_frontend/headers/header_dispatch.rs`.

#### Suggested correction

Non-authorising. Count `;` as a block closer only at top-level nesting, the same way the expression scanner already ignores a semicolon inside parentheses. Leave the inner semicolon in the body range so AST can report it at its own span. Do not change the rule that a top-level `;` closes the active block.

#### Fix scope and preserved invariants

That closer decision only. Preserve balanced `if`, `loop`, `catch` and match ranges, template semicolons remaining text, and rejection of a missing function closer. Do not start accepting a semicolon inside a call.

#### Required validation

`io.line("x"; "y")` inside a function must report the semicolon at its span, not end of file at byte 0. A following statement in that function must still be inside the captured range. The canonical nested-`if` function and a semicolon inside template text must still check. `just validate`.

#### Linked findings

None at filing. Filed in the Diagnostics lane from this Correctness run because the source is rejected. Implementation added AUD-0019-F02 as a linked Tests finding.

### AUD-0019-F02: Unclosed-group tests pin the truncated-body diagnostic

- State: `closed`
- Kind: `Tests`

#### Evidence

Two tests pin the diagnostic produced when a function body is cut at a `;` inside an open group:

- `tests/cases/grouped_unclosed_numeric_chain_rejected` (`value = (((1 + 1 + 1 + 1`, then `return value`, then `;`) expected `MOTH-SYNTAX-0002` `` Unexpected token `return` ``.
- `src/projects/tests/check_tests.rs` `unclosed_numeric_groups_2048_reject_within_the_default_windows_main_thread_stack` expected `MOTH-SYNTAX-0002` for 2048 unclosed groups.

With F01's top-level-only closer, the function's own `;` is inside the open group and does not end the body. Header capture then reaches EOF and reports `MOTH-SYNTAX-0017` `expected ')'`.

#### Counter-explanation tested

**"The old diagnostic was better because it pointed near the error."** Its identity is worse: it names `return` without saying a `)` is missing. Keeping it would need a second closer rule for the case where a group is open, which F01 removes. The span moves to EOF. AUD-0017-F02 owns the opener label that restores locality.

#### Violated contract or cost

These pins would reject the accepted F01 correction.

#### Root owner

The two tests named above.

#### Suggested correction

Expect `MOTH-SYNTAX-0017` and `expected ')'`. Keep the single-diagnostic and stack-budget assertions.

#### Fix scope and preserved invariants

Only those expectations. The deep-nesting test still requires one error and no native stack overflow on the default Windows main-thread stack.

#### Required validation

Full `cargo test -p moth --lib` and the integration suite.

#### Linked findings

AUD-0019-F01.

## Triage (2026-11-13)

**F01 accepted** after source review at `fd695d18b`. `capture_function_body_range_from_cursor` counts every `TokenTag::END` as a block closer before consulting its tracked `NestingDepth`. An illegal semicolon inside a call can therefore cut off the retained function-body token range before AST sees the actual offending token. The nested-delimiter guard belongs at this existing header capture decision. Keep the semicolon illegal inside a call and retain the complete function body for an accurate AST diagnostic. The original empirical check was not rerun because this checkout lacks a compiler binary and Rust toolchain.

**Implementation scope decisions (2026-11-13, coordinator).** Implementation surfaced three consequences of the nesting-aware closer. They were decided as follows:

- **Block headers inside a group.** A nesting-aware `;` alone made a header keyword inside a group, such as `io.line(load() catch: then "x")`, open a scope that no top-level `;` closes. That reported EOF instead of `MOTH-RULE-0051`. Authorised: block-header keywords and colons open scopes only at top-level nesting, matching the closer.
- **`;` inside a collection literal.** A body that is no longer cut at the inner `;` would have newly accepted `values = {1; 2}` in a function, because the literal entry window silently discarded tokens after an early stop. That acceptance already existed at the entry top level. `collection-literals.mtf` separates entries with commas only. Authorised: the literal owner rejects `;` at its own span. A leading `;` entry previously reached the expression parser's empty-fragment invariant (`MOTH-INFRA-0001`) and is now `MOTH-SYNTAX-0002`.
- **Unclosed group before the function closer.** Accepted trade-off. The body now runs to EOF, so the report is `MOTH-SYNTAX-0017` naming the missing `)` or `}` at EOF, and diagnostics in later declarations of that file are not reported. Its identity is better than the old `0002`/`0012` at the following token or byte 0. AUD-0017-F02 owns the opener label. The two test pins are AUD-0019-F02, accepted.

## Resolution (2026-11-13)

**F01 fixed.** In `capture_function_body_range_from_cursor`, a `;` closes the active block only at top-level `NestingDepth`, and `if` / `catch` / `loop` headers and their colons open a scope only at top-level nesting. An illegal `;` inside a call stays in the retained body range, and AST reports it at its own span. `io.line("x"; "y")` now gives `MOTH-SYNTAX-0002` `` Unexpected token `;` `` at 2:16 instead of end of file at 1:1. Receiver methods share the capture. Collection and map literal entries go through `create_complete_expression_until`, which reports a token that an early-stopping entry would otherwise discard. `;`, `:`, `=>` and `else` inside `{...}` are now named at their spans.

**F02 fixed.** Both pins expect `MOTH-SYNTAX-0017` `expected ')'`. The stack-budget test keeps its purpose.

Regression coverage: header test `semicolon_inside_call_stays_in_captured_function_body`, collection test `rejects_statement_terminator_in_curly_literal_entries`, and cases `function_body_semicolon_inside_call_rejected`, `function_body_balanced_blocks_template_semicolon_success`, `catch_inside_call_argument_rejected`, `collection_literal_semicolon_in_function_body_rejected` and `collection_literal_semicolon_at_top_level_rejected`.

**F01 and F02 verified and closed.** Independent verifiers, not the implementer, ran two rounds. Round 1 found a unit test the implementer had not run, which now belongs to F02, and a widening case that also passed on the parent binary. That case now uses the form that fails on the parent and passes with the fix. Round 2 returned clean after re-running the repros, the full `cargo test -p moth --lib`, the full integration suite, clippy and the format check. The coordinator's `just validate` result is recorded in the closeout commit message. Function-body capture changed materially, so the audit-log row is marked stale. This also changes the reproduction path of AUD-0017-F04 and F05; see that report.

## Checked and clean

- The canonical nested `if` is accepted, and the `return` after the `if`'s semicolon stays in the function. An `if` whose semicolon is omitted is rejected (`MOTH-SYNTAX-0017`). That wording gap is already AUD-0017-F02, including the case where the written `;` closed the `if` and left the function open. Not refiled.
- `else` before the semicolon attaches to the `if`. A semicolon before `else` is `MOTH-RULE-0042`, not a second branch. Same-line `else if`, once the outer closers balance, is `MOTH-RULE-0042` and names the unsupported form. The documented nested `if` inside `else` is accepted.
- A statement match, a range loop, a block `catch`, and a value-producing `if` each keep a following statement in the function when their own semicolon is present.
- An operator-continued condition (`if count >` then `limit:`) opens one scope. The following statement stays in the function.
- A semicolon in template text does not close the function.
- An empty `name ||: ;` is accepted. A declaration after the function closer stays outside the function. An extra semicolon after that closer is `MOTH-SYNTAX-0030` at top level, not a second function closer.
- A nested function declaration is `MOTH-RULE-0043` at the inner name. A line-initial `name:` inside a function is the deferred-region diagnostic `MOTH-DEFERRED-0001`, not a captured block.

## Limitations

Partial. Not inspected:

- Match-arm body parsing beyond the shared semicolon that closes the match.
- Multi-bind statements, const-function bodies, and generic-function bodies.
- Receiver methods, which call the same capture function, were not probed on their own.
- `just validate` was not run. Disposable checks are not a substitute for it.
