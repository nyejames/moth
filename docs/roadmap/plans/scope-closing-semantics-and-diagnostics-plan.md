# Scope-closing semantics and diagnostics implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` to implement this plan task by task. Track progress with `- [ ]` checkboxes and review each completed slice.

**Goal:** Make source scope closers follow one predictable physical-line contract and give precise, useful diagnostics for misplaced or missing semicolons and related scope boundaries.

**Architecture:** Enforce layout when the existing header, declaration or AST parser recognises a real scope opener or closer. Retain the minimum opener and last-closer facts needed for diagnostics instead of rescanning source or creating another parsing pass. Use the existing source line index, typed diagnostic families and integration runner.

**Tech stack:** Rust frontend, source/token and diagnostic stores, Moth source and `.mtf` templates, `tests/cases/` and focused Rust tests.

**Spec:** The accepted contract in section 1 supersedes the current permissive closer placement. Publish it to the canonical unsuffixed language references during implementation. Read `AGENTS.md`, the style and testing guides, `docs/compiler-design-overview.md`, `docs/compiler-data-layout-design.md` and the canonical references routed by `docs/developer-docs/language/overview.mtf` before editing.

## Global constraints

- A semicolon closes a recognised source construct, never an ordinary statement. Indentation does not determine scope or change program execution.
- Preserve the existing grammar for `if`/`else`, match arms, inline value forms, functions, methods, loops, choice declarations, traits and `catch` blocks. Do not activate deferred declared regions or new scope forms.
- Keep source-token classification in its existing owner. Do not impose these rules lexically on template bodies, Markdown, strings or comments, and do not build a second whole-file delimiter scan.
- Preserve one parser path and one producer for each diagnostic fact. Remove superseded counters or helpers when an opener stack replaces them. No migration mode or legacy syntax allowance.
- Use typed `DiagnosticDraft` family constructors, stable codes, reason keys and exact spans. User-authored syntax must not panic or become `InfrastructureFailure`.
- Write a failing test for each changed semantic boundary, confirm the failure, implement the narrowest fix and rerun the focused test. Use the documented validation checkpoints, not an unconditional full gate after every edit.

## Review focus

1. **Arm ownership:** `else io.line("waiting");` remains valid even when the original `if:` occupies an earlier line, while an ordinary `then` statement never grants that exception.
2. **Early capture:** A `;` inside `(...)`, `{...}` or a template must not prematurely terminate an enclosing function's retained token range, nor become a legitimate closer.
3. **Physical lines:** LF, CRLF, bare CR, blank lines, Unicode text and comment-only lines must give identical layout decisions for equivalent code.
4. **Declaration paths:** Inline choices, empty traits, const functions and receiver methods must obey the same rule as executable statements, including trailing content after their closers.
5. **Recovery:** A wrong closer or missing outer closer must report its authored cause with related opener context, without inventing a scope from indentation or flooding the user with consequences.

## 1. Accepted source semantics

### Opener and closer placement

1. Every semicolon-terminated source construct has one structural opening site: the `:` that begins its body, or `::` for a choice declaration. The same rule applies to currently supported `must:`, `catch:` and other block-form owners. Opening punctuation inside expressions, types or templates does not count as a source scope opener.
2. **At most one semicolon-terminated scope may open on a physical source line.** This includes nesting. A second active nested opener on that line is illegal, even if its eventual closing semicolon appears elsewhere. Independent constructs cannot share a line either because code after the first closer is forbidden.
3. A scope may close inline when its opening site and closing `;` occupy the same physical line. Otherwise its `;` must appear on a later line with only indentation or whitespace before it.
4. Branch continuations are the deliberate exception to the opening-site test, **not additional independently terminated scopes**. The final `;` of an `if` may share the line with the `else` introducing its terminal body. The final `;` of a match may share the line with the terminal arm's `=>`, subject to the existing requirement that arm headers begin on a new logical line. Without such an active terminal arm, the original opening-site rule applies. This never licenses a `;` at the end of an intermediate arm or after an ordinary `then` statement.
5. After **every** source scope closer `;`, the remainder of that physical line may contain only whitespace or an ordinary `--` comment. Another closer, a new statement or declaration, or any other authored code is invalid. A consecutive `;;` is always invalid. A comment may follow a valid closer but cannot precede one on the same line because `--` consumes the rest of that line.
6. Blank lines and whitespace inside a block remain legal. An empty inline block and an empty multiline block remain legal. These layout constraints do not make indentation significant for grouping, name visibility or execution.

Examples of valid source:

```moth
ready_check || -> Bool: return true;

if ready:
    io.line("ready")
else io.line("waiting"); -- one closer for the complete if

if ready:
    loop 0 to 3 |index|: io.line([: [index]])
    ;
;

Status :: Ready, Waiting;

SERIALIZABLE must:
;
```

Examples the implementation must reject:

```moth
run ||: if ready: io.line("ready");; -- nested openers on one line

run ||:
    io.line("ready"); -- closer shares a line with a statement, not its opener

run ||:
    io.line("ready")
; io.line("after") -- authored content after the closer

if ready: io.line("ready");; -- extra closer, even for an inline block
```

The new rule intentionally rejects previously legal code such as a multiline function with its `;` after its final statement. A user who intended a statement terminator instead of a scope closer should receive a targeted explanation, not a consequence blamed on a later token.

### Diagnostic contract

- Report the first provable structural error at its authored token. For a second opener on the line, point to that opener and label the first. For a closer following an ordinary statement on a multiline scope, point to `;` and label the active opener. For content following a legal closer, point to the first trailing token and label the `;`.
- An unclosed scope reports the innermost still-open construct, its opening span, the expected `;` and a useful EOF or enclosing-boundary primary span. If a visible `;` closed an inner construct rather than the outer one, label that inner closer too. Do not claim which scope the author intended to close.
- Preserve recognised user errors within parentheses, collections and calls. Report `;` at its actual position as invalid inside the delimiter, with the relevant `(`/`{` context where available, rather than truncating the function at that token or reporting a fabricated file-start EOF.
- Prefer an actionable, specific diagnosis over a generic token failure. Add `MOTH-SYNTAX-0041` (`InvalidScopeLayout`) with distinct typed reasons for a nested same-line opener, a closer after code, trailing code and stacked closers. Add `MOTH-SYNTAX-0042` (`UnclosedSourceScope`) for a proven missing source closer, with the missing construct as typed evidence. These codes are unused in the inspected baseline. Preserve `MOTH-SYNTAX-0017` for unrelated expected-delimiter EOF, `MOTH-SYNTAX-0028` for an intermediate match-arm delimiter and `MOTH-SYNTAX-0030` for an isolated unmatched top-level `;`. A `;;` directly after a legal closer takes the more specific `MOTH-SYNTAX-0041` reason and labels the first `;`, rather than generating two errors. Recheck code allocation when the plan activates.
- Indentation may support an additional **suggestion** about a likely omitted `;` only after a real structural error has been established. It must never determine scope legality, silently insert a closer, or suppress the source-accurate diagnostic.

## 2. Existing owners and change boundaries

- **Tokens and lines:** `src/compiler_frontend/tokenizer/lexer.rs` emits `END` and `NEWLINE` tokens and discards code comments. `src/compiler_frontend/source/line_index.rs` owns source-offset-to-physical-line mapping for LF, CRLF and CR. Use existing `SourceSpan`/`LocalSpan` positions and a borrowed line index or the retained newline structure, without storing line-number strings or a duplicate line table.
- **Stage 2 executable capture:** `src/compiler_frontend/headers/header_dispatch.rs::capture_function_body_range_from_cursor` already balances nested scopes using `NestingDepth` and opened/closed counters. It owns function/method source-range accuracy and the structural EOF failure. Preserve its match-arm-colon suppression while retaining opener spans and checking that `END` only closes a source scope outside grouping. `src/compiler_frontend/headers/file_parser.rs` owns entry-start token routing and its `executable_block_depth`.
- **Stage 2 declaration paths:** `src/compiler_frontend/declaration_syntax/choice.rs` and `src/compiler_frontend/headers/trait_headers.rs` consume their own declaration closers. They must use the same source-line rule at their existing delimiter decisions, rather than rely on AST to see tokens it never receives.
- **Stage 4 executable bodies:** `src/compiler_frontend/ast/statements/body_dispatch.rs` owns statement contexts, legal body termination and EOF. `src/compiler_frontend/ast/statements/branching.rs` owns `if`/`else` and match-arm transitions. Value-block and catch handling also meet semicolons through `src/compiler_frontend/utilities/token_scan.rs::OpenConstruct`.
- **Expressions and error facts:** `src/compiler_frontend/ast/expressions/parse_expression.rs`, `parse_expression_dispatch.rs`, `eval_expression/result_type.rs` and `src/compiler_frontend/ast/cursor.rs` own expression-end diagnostics and their spans. Typed family declarations live under `src/compiler_frontend/compiler_messages/diagnostics/families/`, reason keys in `diagnostics/reason_keys.rs` and wording in `render/syntax.rs` or `render/payload.rs`.

**Primary test owners:** Extend `src/compiler_frontend/headers/tests/parse_file_headers_span_tests.rs`, `parse_file_headers_entry_runtime_tests.rs`, `parse_file_headers_signature_choice_tests.rs` and `parse_file_headers_trait_tests.rs` for header/declaration boundaries. Extend `src/compiler_frontend/ast/statements/tests/branching_tests.rs`, `fallible_handling_tests.rs`, `diagnostics_tests.rs` and `src/compiler_frontend/ast/expressions/tests/eval_expression_tests.rs` for parsed bodies and expression errors. Preserve the existing integration owners `diagnostic_top_level_stray_scope_close`, `diagnostic_match_semicolon_between_arms` and `grouped_dangling_operator_numeric_chain_rejected`. Add genuinely new cases for new externally visible error families, rather than duplicating equivalent layouts in many cases.

Implement the layout decision at the existing parser's recognised opener/closer event. If two paths must make the same comparison, share only a small source-line/placement predicate with a clear frontend owner, and keep opener stacks local to the parser that needs them. Do not globally reject raw `COLON`/`END` tokens before their actual syntactic role is known. A capture pass may preserve boundary facts for AST, but must not issue the same layout error again at another stage.

## 3. Implementation tasks

### Task 0 - Activation and regression inventory

- [ ] Start from the current roadmap baseline and record the exact revision in working notes, not this queued plan. Map every producer and consumer of source `END`, `COLON`, `DOUBLE_COLON`, `ELSE` and `FAT_ARROW`, distinguishing executable and declaration contexts from template and grouping contexts.
- [ ] Inventory existing `tests/cases/`, compiler unit tests, core packages, docs source and `.mtf` code examples that rely on an inline closer following a multiline body or on same-line nested scopes. Classify them as intended success, intentionally invalid source or documentation-only examples. Record the before/after fixture counts and syntax decisions.
- [ ] Review AUD-0017-F01/F02/F04-F07 and AUD-0019-F01 against current source. These overlap this plan. AUD-0017-F03 remains historically rejected under the previous grammar, but its example becomes invalid under the new accepted contract. Leave unrelated audit reports to the broader audit-correction lane.

**Gate:** The migration and owner inventory exists before semantic changes. No scanner, test or diagnostic owns the same source occurrence twice.

### Task 1 - Scope capture, opener identities and EOF correctness

**Owners:** `headers/header_dispatch.rs`, `headers/file_parser.rs`, `utilities/token_scan.rs` and their existing test modules.

- [ ] Add failing focused tests: a semicolon in `io.line("x"; "y")` cannot end function capture, a following valid statement stays in the retained range, nested `if` and match ranges balance, a template's literal `;` remains body text, and a missing function or entry-start nested closer fails instead of silently accepting swallowed statements.
- [ ] Add `tests/cases/diagnostic_unclosed_entry_scope/` covering an unclosed entry `if` and one loop in a compact test program, plus `diagnostic_semicolon_inside_call/` proving an illegal in-call semicolon reaches AST at its true span. Keep separate files only if the first error prevents the runner from observing the second case.
- [ ] Correct the `END` decision in function capture to respect `NestingDepth` and preserve the illegal in-group token for AST. Where the existing counter loses unmatched-opener information, replace it with a minimal parser-local stack retaining the opener kind/span and latest inner closer. Do not carry parallel open/closed counters and a redundant stack. Retain match-arm suppression and correct half-open `TokenRange` boundaries.
- [ ] Reject EOF with an open nested block in entry-start work while leaving the synthetic entry itself unclosed by design. Consume Stage 2 facts in Stage 4 where available. Handle missing function, loop, `if`, match, catch, choice and trait closers in their actual existing owning paths, with accurate innermost opener context.
- [ ] Run the focused header, start-body, choice and trait tests before continuing. Commit the structural range/EOF slice separately from wording changes when it has an independent passing gate.

**Gate:** Incorrect semicolons inside grouping cannot steal source ranges and an unclosed executable scope cannot reach successful HIR.

### Task 2 - Enforce physical-line semantics across every scope owner

**Owners:** Existing Stage 2 declaration/header consumers and Stage 4 statement, branch, value-block and catch consumers.

- [ ] Add table-driven failing tests for one-line function, loop, `if`, block catch, choice and trait declarations, plus multiline variants with a standalone closer. Exercise valid `else body;`, a final match `=> body;`, empty scopes, comments after closers and whitespace-only/comment-only lines.
- [ ] Add failing tests for `function ||: if x: call();;`, nested `if`/loop/catch openers sharing a line, `call();` inside a multiline scope, `; next()`, `;;`, and two declarations on one line. Assert the first offending opener, closer or trailing token, not merely compilation failure.
- [ ] Add one `tests/cases/scope_closing_inline_valid/` scenario covering inline functions, an `if` with same-line `else` closer and inline choice/trait declarations. Add focused negative integration cases `diagnostic_nested_inline_scope/`, `diagnostic_multiline_closer_after_code/` and `diagnostic_code_after_scope_closer/`, with the other combinations covered by parser-local tables.
- [ ] At recognised opening and closing events, compare physical lines using existing source positions. Reject a second semicolon-terminated opener on the same physical line and enforce the effective-terminal-branch exceptions from section 1. Require a multiline closer's line prefix to contain only whitespace, then require its line suffix to contain only whitespace or a comment. Use the existing token stream and line index for these checks, not a fresh scan of source or per-scope source strings.
- [ ] Ensure declaration shells and implicit entry work apply the same contract without duplicate AST validation. Recheck currently unsupported constructs such as declared regions without silently enabling them. Test split multiline headers, CRLF, bare CR and multibyte column spans.
- [ ] Run targeted parser and integration cases, then commit this language-semantics slice.

**Gate:** Valid and invalid shapes in section 1 behave alike across functions, root statements, choices, traits, match arms and catch blocks. No special `;;` allowance remains.

### Task 3 - Typed scope diagnostics and useful related sites

**Owners:** `compiler_messages/diagnostics/families/choices_control_flow.rs`, `token_lexical.rs`, `diagnostics/reasons.rs`, `diagnostics/reason_keys.rs`, `render/syntax.rs`, `render/payload.rs` and the emitting parser owners.

- [ ] Add failing diagnostic-family tests for typed layout reasons: `NestedScopeOpenerOnLine`, `MultilineCloserAfterCode`, `ContentAfterCloser` and `AdditionalCloserOnLine`. Give genuinely unclosed source scopes a typed reason naming the construct and expected closer. Distinguish unexpected top-level close, bad match-arm delimiter, and unexpected `;` in an expression without recoding their existing identities.
- [ ] Pin `MOTH-SYNTAX-0041` for each layout reason and `MOTH-SYNTAX-0042` for unmatched source scope EOF. Assert one deterministic diagnostic for `;;` where the second semicolon follows a valid closer, while an isolated unmatched `;` keeps `MOTH-SYNTAX-0030`. Preserve distinct match-arm and expression-delimiter errors.
- [ ] Implement narrow family constructors carrying the primary span and optional related opener, branch opener and last inner closer. Add stable reason keys and render specific, beginner-friendly text: "`;` closes a scope, not a statement", "place this closer on its own line" and "move the nested scope opener to a new line" as appropriate. Avoid raw `TokenTag(N)`, speculative declarations or a generic "unexpected EOF" when an open source scope is known.
- [ ] Use existing draft secondary-label and frozen-report mechanisms. Point the primary at the bad `;` or second opener where it exists, at the trailing token for content after a closer, and at actual truncation for an unclosed construct. Label the exact opener and the inner closer when they explain why the outer scope remains open. Preserve source remapping and deterministic label order.
- [ ] Add `tests/cases/` diagnostic assertions with exact code, reason, path, primary line and selected secondary labels. In particular test a mistakenly placed statement-terminating `;` followed by `return` or a later final `;`, a missing outer function closer, missing entry `if`/`loop` closer and empty marker trait EOF.
- [ ] Run diagnostics-family and integration checks. Commit diagnostic schema, wording and emission together.

**Gate:** A new Moth user can locate the offending semicolon or missing scope from the first diagnostic, including when an inner closer consumed the only visible `;`.

### Task 4 - Expression, match and error-order corrections

**Owners:** `ast/expressions/parse_expression.rs`, `parse_expression_dispatch.rs`, `eval_expression/result_type.rs`, `ast/statements/branching.rs`, `ast/statements/diagnostics.rs` and the nearest tests.

- [ ] Add failing cases for `f(1; 2)`, `(1; 2)`, `{1; 2}`, a misplaced `;` after a grouped expression, and `;` inside a match between arms. Assert the authored token span and the existing match-arm identity, rather than a truncated body or file-start EOF.
- [ ] Correct exhausted-cursor group diagnostics to use the existing `GroupFrame::opening_span` and real truncation location, never `LocalSpan::source_start()` for missing `)`. Replace both expression-stop `{token:?}` render paths with actual user-facing token spellings. Preserve the expected delimiter semantics and group nesting.
- [ ] Correct the infix missing-operand classification so `1 +` and `1 + )` report a missing right operand. Update the existing `grouped_dangling_operator_numeric_chain_rejected` fixture and `eval_expression_tests.rs::malformed_groups_keep_group_operand_diagnostics` in the same change, retaining their exact operator-span checks.
- [ ] Ensure pre-existing specific diagnostics win over speculative scope errors and don't cascade across truncated ranges. Keep `MOTH-SYNTAX-0028` for semicolons between match arms and `MOTH-SYNTAX-0030` for genuinely unmatched top-level `;`. Run focused expression/branch tests and selected end-to-end cases.

**Gate:** Existing AUD-0017-F04-F07 and AUD-0019-F01 scenarios report their real cause with correct spans and no debug-token names.

### Task 5 - Investigate bounded missing-closer suggestions

**Owners:** The existing unclosed-scope diagnostic and the parser-local opener metadata. No new recovery pass.

- [ ] Collect adversarial examples of a real missing `;`: apparent dedent after an `if`, nested scopes with one visible closer, a trailing `else`, unrelated comments/blank lines and a long flat block with unusual indentation. Contrast them with valid deliberately irregular indentation, tab/space mixtures and same-line branch closers.
- [ ] Prototype one bounded diagnostic-only candidate: when EOF or another proven structural boundary leaves a scope open, use its opener and retained line positions to identify an earlier line whose indentation plausibly signals the intended close. Emit at most one optional "Did you mean to close ... before this line?" note with a labelled candidate. Require clear evidence, such as a completed body line followed by a lower-indented non-branch statement. An ambiguous result yields only the definite missing-closer diagnostic.
- [ ] Test that the suggestion never changes acceptance, source scopes, primary spans, recovery order or stable error identity. It must be absent for ambiguous indentation and for a correctly terminated scope. If the heuristic cannot meet these criteria, remove the prototype and record that investigation and its counterexamples in the permanent diagnostics guidance rather than ship a noisy guess.
- [ ] Explore and document additional bounded recovery opportunities encountered during testing, especially misplaced `;` that stole an enclosing close. Implement only evidence-backed improvements within existing parser owners and add focused tests for each retained hint.

**Gate:** Every shipped heuristic improves an established structural error without pretending whitespace defines scope. Unproven hints remain unimplemented.

### Task 6 - Migrate sources, publish semantics and validate

- [ ] Update all affected authored `.moth` programs, core packages, build fixtures and `tests/cases/` expectations. Preserve program behaviour and runtime outputs. Review `.mtf` documents and embedded Moth code examples independently from their literal template text. Reject remaining invalid examples rather than exempting tests or generated input.
- [ ] Publish the exact opening/closing contract, branch exceptions and examples in `docs/docs/language-overview/blocks-and-statements.mtf`, then update the precise affected canonical function, branching, choice, trait, catch and loop references. Add one concise explanation for users familiar with statement-terminating semicolons. Refresh the cheatsheet and progress matrix only where their stated semantics or implementation status change. Do not edit `docs/_release/**` directly.
- [ ] Check the focused Rust units with `cargo test -p moth --lib <filter>` and affected integration cases with `cargo run --quiet -- tests --case <id> --terse`. Use `just validate` at major checkpoints, the docs source/build gate from `validation.mtf`, then `just validate-full` for final merge readiness. Record actual commands, feature/platform exclusions and failures. Include malformed-source and real-source checks on both available host platforms.
- [ ] Inspect and remove obsolete closer counters, duplicate scan/recovery helpers, unused reason variants, redundant fixture copies and parallel rendering paths. Review touched functions for oversized state, cloned token text, indirection and comments that explain syntax instead of ownership. Preserve all distinct regression coverage.
- [ ] Complete the **Slice review** across semantic correctness, diagnostics, test honesty, API/stage ownership, performance and Rust readability. Obtain independent verification of each affected accepted audit finding before closing it. Update overlapping audit reports, the open-findings index and completed-audit records following `docs/roadmap/audit-guide.md`. Mark audit coverage stale where this implementation materially changes the recorded area, but do not claim a new audit run. Close a report only when all its findings have reached the required terminal states. Delete this plan and its roadmap entry in the same commit that completes the implementation.

**Final gate:** The complete source tree follows the accepted semicolon contract, all supported constructs produce useful opener-aware diagnostics, missing source scopes cannot succeed silently, and the relevant Rust, Moth and documentation gates pass on the final tree.
