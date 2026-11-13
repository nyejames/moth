# Open Audit Findings

Unresolved findings only. The evidence and triage decision live in each report under [open audits](./audits/open_audits/). Once every finding in a report is resolved, move the report to [closed audits](./audits/closed_audits/) and record its resolution in the [completed audits log](./completed_audits_log.md).

- [Audit guide](./audit-guide.md)
- [Audit coverage log](./audit-log.md)
- [Audit-kind index](./audit-kinds/README.md)

## Audits in progress

None.

## Candidate findings

None.

## Accepted

Triaged and authorised for bounded investigation or correction. Implementation has not begun.

- [AUD-0018-F01: An unclosed source-directive operand hangs the compiler](./audits/open_audits/AUD-0018-source-directive-parsing.md#aud-0018-f01-an-unclosed-source-directive-operand-hangs-the-compiler)
  - `Correctness` | `frontend.source_directive_parsing`
- [AUD-0020-F01: Reachable payload-free variant values are accepted and then crash Wasm lowering](./audits/open_audits/AUD-0020-backend-feature-validation.md#aud-0020-f01-reachable-payload-free-variant-values-are-accepted-and-then-crash-wasm-lowering)
  - `Correctness` | `backends.feature_validation`
- [AUD-0017-F01: Unclosed blocks in the entry start body are accepted and swallow later statements](./audits/open_audits/AUD-0017-statement-boundary-diagnostics.md#aud-0017-f01-unclosed-blocks-in-the-entry-start-body-are-accepted-and-swallow-later-statements)
  - `Correctness` | `frontend.statement_boundary_diagnostics`
- [AUD-0019-F01: A semicolon inside a call ends the function body range](./audits/open_audits/AUD-0019-function-body-parsing.md#aud-0019-f01-a-semicolon-inside-a-call-ends-the-function-body-range)
  - `Diagnostics` | `frontend.function_body_parsing`
- [AUD-0018-F02: A known directive inside a declaration shell is reported as an unexpected token](./audits/open_audits/AUD-0018-source-directive-parsing.md#aud-0018-f02-a-known-directive-inside-a-declaration-shell-is-reported-as-an-unexpected-token)
  - `Diagnostics` | `frontend.source_directive_parsing`
- [AUD-0017-F02: A missing block closer is reported as unexpected end of file and does not name the open block](./audits/open_audits/AUD-0017-statement-boundary-diagnostics.md#aud-0017-f02-a-missing-block-closer-is-reported-as-unexpected-end-of-file-and-does-not-name-the-open-block)
  - `Diagnostics` | `frontend.statement_boundary_diagnostics`
- [AUD-0017-F04: An unclosed parenthesis at the end of an expression is reported at the start of the file](./audits/open_audits/AUD-0017-statement-boundary-diagnostics.md#aud-0017-f04-an-unclosed-parenthesis-at-the-end-of-an-expression-is-reported-at-the-start-of-the-file)
  - `Diagnostics` | `frontend.statement_boundary_diagnostics`
- [AUD-0017-F05: Expression-end expected tokens are rendered as TokenTag debug names](./audits/open_audits/AUD-0017-statement-boundary-diagnostics.md#aud-0017-f05-expression-end-expected-tokens-are-rendered-as-tokentag-debug-names)
  - `Diagnostics` | `frontend.statement_boundary_diagnostics`
- [AUD-0017-F06: Ending a binary expression reports the missing operand on the wrong side](./audits/open_audits/AUD-0017-statement-boundary-diagnostics.md#aud-0017-f06-ending-a-binary-expression-reports-the-missing-operand-on-the-wrong-side)
  - `Diagnostics` | `frontend.statement_boundary_diagnostics`
- [AUD-0017-F07: Dangling-operator tests pin the wrong operand side](./audits/open_audits/AUD-0017-statement-boundary-diagnostics.md#aud-0017-f07-dangling-operator-tests-pin-the-wrong-operand-side)
  - `Tests` | `frontend.statement_boundary_diagnostics`, linked to AUD-0017-F06

## In progress

None.

## Blocked

None.
