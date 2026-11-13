# AUD-0018: Source directive parsing

- State: `complete`
- Kind: `Correctness`
- Area: `frontend.source_directive_parsing` — recognition, operand capture and attachment of source directives (`$config`, `$export`, `$doc`, `$todo`) in header preparation and statement position
- Coverage: `partial`
- Reviewed: `2026-11`
- Baseline: Source at `c62bbe560`. `just validate` was not run. Empirical checks used `./target/debug/moth` 0.1.12 (`moth check --terse` on disposable HTML projects under `./tmp/aud0018`). One probe did not return and was killed after a stack sample. No active plan owns source-directive parsing. `$feature`, `$layout`, `$test` and custom-metadata directives are open or deferred and were not treated as missing.

Template-head directive composition (`$slot`, `$in`, `$before`, `$after`, formatters) is a separate grammar and was not this run.

## What was inspected

- `src/compiler_frontend/style_directives/source.rs` — operand capture, statement-run attachment, declaration-target check, compile-time String fold
- `src/compiler_frontend/style_directives/specs.rs` and `builtins.rs` — the four source-modifier descriptors
- `src/compiler_frontend/headers/file_parser.rs` — top-level run, blank-line and same-line rules, target acceptance
- `src/compiler_frontend/headers/header_dispatch.rs` — comma after an omitted `$config` initializer
- `src/compiler_frontend/declaration_syntax/declaration_shell.rs` — initializer omission
- `src/compiler_frontend/declaration_syntax/choice.rs` — variant-list fallback
- `src/compiler_frontend/tokenizer/lexer.rs` — unknown `$name` recognition
- `src/compiler_frontend/tokenizer/cursor.rs` — EOF `advance` stays on EOF
- `src/compiler_frontend/ast/statements/body_dispatch.rs` — statement-boundary dispatch into the shared run parser
- Header and statement tests named in Checked and clean, as the executable baseline

Context read, not audited: template-head parser, export publication, config application after the contract shell exists.

## Authorities read

- `docs/docs/directives/directives.mtf` — attachment, one operand, duplicate names, `$export` / `$config` / `$doc` / `$todo` targets
- `docs/docs/directives/overview.mtf` — placement errors and the no-second-grammar rule
- `docs/compiler-design-overview.md` — Stage 1 recognition, Stage 2 attachment, source-directive effects
- `docs/docs/project-structure/build-inputs.mtf` — only a marked input may omit its initializer
- `docs/docs/packages/public-reexports.mtf` — `$export` requires a flat direct selection
- `docs/developer-docs/style-guide/style-guide.mtf`
- `docs/roadmap/audit-kinds/correctness.md`

## Existing findings and active plans checked

Open findings had no directive-parsing candidate. No roadmap plan owns this parser. Deferred `$feature`, `$layout`, `$test` and custom metadata were not filed as absent.

## Findings

### AUD-0018-F01: An unclosed source-directive operand hangs the compiler

- State: `accepted`
- Kind: `Correctness`

#### Evidence

`directives.mtf` requires an invalid operand to be diagnosed at its authored site. An unclosed template used as a `$doc` operand does not return.

Disposable `@entry.moth`:

```text
$doc [:hello
greeting #= "Hello"
```

`./target/debug/moth check --terse` did not return. After more than 80 seconds a sample of that process was inside `capture_source_directive_operand`, calling `TokenCursor::advance` and `TokenCursor::current`. The process was then killed.

The same unclosed template as an ordinary initializer does return:

```text
greeting #= [:hello
```

```text
E|MOTH-SYNTAX-0022|@entry.moth|1:20|This template is not closed before the end of the file. Add `]`.
```

`capture_source_directive_operand` walks the operand until a top-level newline, directive or EOF. It breaks on EOF only when `NestingDepth::is_top_level()` is true. An unclosed `[`, `(` or `{` leaves that depth non-zero. `TokenCursor::advance` documents that EOF is stable and never advances, and `current()` still returns that EOF token. `NestingDepth::step_tag` does not treat EOF as a closer. The loop therefore calls `advance` on the same EOF forever.

Header attachment and statement attachment both call this function.

#### Counter-explanation tested

**"Every unclosed template hangs, so this is the template scanner rather than directive capture."** Rejected. The ordinary initializer above is diagnosed at once with `MOTH-SYNTAX-0022`. The sample is in the directive operand loop, not in template parsing.

**"EOF advance is specified to stay put, and the loop is waiting for a closer tokenization has not emitted yet."** Rejected. Tokenization has finished. The cursor is already on the emitted EOF token. Stepping that tag does not close the template, and the next `current()` is the same token.

Disproof would be the `$doc [:hello` check returning a diagnostic.

#### Violated contract or cost

`directives.mtf`: invalid operands are diagnosed at the authored site. Style guide: invalid external input is a structured diagnostic, not a non-returning compiler. A two-line missing `]` prevents `moth check` from finishing.

#### Root owner

`capture_source_directive_operand` in `src/compiler_frontend/style_directives/source.rs`. The EOF break is guarded by `depth.is_top_level()`.

#### Suggested correction

Non-authorising. Stop when the cursor is on EOF even if a group is still open, and diagnose the unclosed operand. Do not call `advance` on EOF from this loop. Keep a balanced newline inside the operand.

#### Fix scope and preserved invariants

That capture loop only. Preserve top-level newline and following-directive termination, nested template directives remaining inside a balanced operand, and the ordinary unclosed-template diagnostic for non-directive expressions.

#### Required validation

`moth check` of an unclosed `$doc` template operand must return a diagnostic. The same shape with an unclosed `(` or `{` must also return. A balanced multi-line `$doc [:hello]` operand must still attach. `just validate`.

#### Linked findings

None.

### AUD-0018-F02: A known directive inside a declaration shell is reported as an unexpected token

- State: `accepted`
- Kind: `Diagnostics`

#### Evidence

`directives.mtf` and `docs/docs/directives/overview.mtf` require a known directive in the wrong context to be a placement error, distinct from an unknown name. The attachment walk does that. `Box must DISPLAY` after `$doc` reports `MOTH-SYNTAX-0021`: `` `$doc` must attach to an eligible declaration. ``

The same known `$doc` inside a shell that the attachment walk does not own is `MOTH-SYNTAX-0002`, `Unexpected token style directive `$doc``:

```text
Status ::
    $doc [:Ready state.]
    Ready,
    Failed,
;
```

```text
Box = |
    $doc [:Field docs.]
    label String,
|
```

```text
parse |$doc [:Param.] source String| -> String:
    return source
;
```

`parse_choice_shell` reaches those tokens in its non-symbol fallback and returns `unexpected_token_from_tag`. It does not ask the directive registry whether the token is a known source directive. Parameter and field shells produce the same code and wording.

#### Counter-explanation tested

**"A directive inside a variant, field or parameter list is illegal syntax, so unexpected token is the right report."** The source must be rejected. The contract still requires a known directive in the wrong place to be a placement error. The attachment walk already has that identity for a conformance. These shells drop the token into the generic unexpected-token path, so the user is not told that `$doc` attaches before the declaration rather than inside it.

Disproof would be a directive placement code, naming `$doc`, for the three sources above.

#### Violated contract or cost

Directive overview: placement errors distinguish a known invalid context from an unknown name. `MOTH-SYNTAX-0002` is neither the unknown-name diagnostic (`MOTH-SYNTAX-0010`) nor the placement diagnostic (`MOTH-SYNTAX-0021`).

#### Root owner

The declaration-shell fallbacks that treat `STYLE_DIRECTIVE` as an ordinary illegal token. The choice path is the `_` arm of `parse_choice_shell`. Field and parameter shells emit the same unexpected-token diagnostic.

#### Suggested correction

Non-authorising. When the current token is a known source directive, report the existing placement error instead of unexpected token. Do not accept the directive inside the shell, and do not attach it to a field, variant or parameter.

#### Fix scope and preserved invariants

Those shell fallbacks only. Preserve rejection, unknown-name handling for `$not_a_directive`, and `MOTH-SYNTAX-0021` on the attachment walk. Do not start parsing a directive run from inside a field or variant list.

#### Required validation

The three sources above must report a directive placement identity, not `MOTH-SYNTAX-0002`. A real unknown token in those shells must keep today's unexpected-token diagnostic. `just validate`.

#### Linked findings

None. Filed in the Diagnostics lane from this Correctness run because legality is already rejection.

## Triage (2026-11-13)

Both findings are accepted for bounded investigation after source review at `fd695d18b`. The audited implementation is unchanged from `c62bbe560`. Fresh runtime reproduction was unavailable because this checkout has no compiler binary or Rust toolchain.

- **F01 accepted.** `capture_source_directive_operand` only stops at EOF when `NestingDepth` is top-level, while `TokenCursor::advance` leaves EOF stationary. A missing inner closer therefore gives the loop no progress condition. End capture at EOF regardless of nesting and let the existing expression/template diagnostic owner identify the missing closer. Preserve balanced multiline operands.
- **F02 accepted.** `directives.mtf` explicitly requires known directives in invalid contexts to receive placement diagnostics. The declaration-shell fallback emits a generic unexpected-token diagnostic for `STYLE_DIRECTIVE`, even though directive recognition is already complete. Prefer the existing typed placement family at the shell rejection boundary and keep field, variant and parameter attachment unsupported. Check for a reusable rejection path before adding separate special cases.

## Checked and clean

- Unknown `$not_a_directive` is `MOTH-SYNTAX-0010` and lists the source directives. A known template directive in source position is a wrong-context placement error, not an unknown name. Covered by `template_head_directive_in_source_is_wrong_context` and the lexer path in `tokenize_style_directive`.
- `$export` and `$config` reject an operand, including the call spelling `$name(...)`. A whitespace-separated parenthesized `$doc` operand remains an expression. Covered by the header operand tests.
- A repeated name cites both spans. Same-line directives require whitespace. The target must start on a later line. A blank line, including between comments and under CRLF, breaks attachment. A comment-only line does not. Covered by the header attachment tests and rechecked for `$doc` under CRLF.
- `$config` attaches only to an explicitly typed compile-time binding. Omitting that initializer is accepted and later reports a missing input (`MOTH-CONFIG-0001`). An unmarked `api #String` is `MOTH-RULE-0031`. A comma after an omitted `$config` initializer is rejected at the header boundary.
- `$export` is rejected outside `@mod.moth` and `@package.moth`, and does not attach to the declaration after an invalid target. `$config` and `$export` cannot share one constant. Receiver methods, conformances, namespace dependencies and non-direct selections are rejected by the existing export tests.
- `$doc` and `$todo` stack on one declaration, including across lines inside a function. `$todo` attaches to `return`, `if`, `loop` and an assignment, and adds no runtime work in the statement tests. A directive immediately before `;` is dangling.
- A nested `$md` stays inside a `$doc` operand. A top-level `$export` after that operand is a separate directive in the run. `$doc [$md: see $export]` checks as a private constant; the inner `$export` does not become the next source directive.
- `$doc` on a multi-bind or a conformance is rejected as an ineligible target (`MOTH-SYNTAX-0021`). The contract says an eligible declaration, and neither form is in the spec's target set. Not filed.
- A `then` clause after a conditional loop is `MOTH-RULE-0051` with or without a preceding `$todo`. The directive does not change that rejection.

## Limitations

Partial. Not inspected:

- Template-head directive parsing under `src/compiler_frontend/ast/templates/template_head_parser/`.
- Whether every declaration-shell fallback, beyond choice variants, struct fields and parameters, uses unexpected token for `STYLE_DIRECTIVE`.
- Publication of `$export` and application of `$config` after a successfully attached shell.
- `just validate` was not run. Disposable checks are not a substitute for it.
