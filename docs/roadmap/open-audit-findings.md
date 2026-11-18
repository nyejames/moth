# Open Audit Findings

Unresolved **formal audit** findings only. The evidence and triage decision live in each report under [open audits](./audits/open_audits/). Once every finding in a report is resolved, move the report to [closed audits](./audits/closed_audits/) and record its resolution in the [completed audits log](./completed_audits_log.md). New Correctness audits and incidental discoveries use [bug reports](../bug-reports.md), not this index.

- [Audit guide](./audit-guide.md)
- [Audit coverage log](./audit-log.md)
- [Audit-kind index](./audit-kinds/README.md)

## Audits in progress

- [AUD-0024: Numeric proof side-table comments](./audits/open_audits/AUD-0024-numeric-proofs-comments.md)
  - `Comments` | `analysis.numeric_proofs`
- [AUD-0025: Numeric proof side-table style](./audits/open_audits/AUD-0025-numeric-proofs-style.md)
  - `Style` | `analysis.numeric_proofs`

## Candidate findings

- [AUD-0022-F01: Interval operator arithmetic is implemented twice](./audits/open_audits/AUD-0022-numeric-proofs-redundancy.md#aud-0022-f01-interval-operator-arithmetic-is-implemented-twice)
  - `Redundancy` | `analysis.numeric_proofs`
- [AUD-0022-F02: JS proven narrowing re-classifies its conversion and recovers it through `expect`](./audits/open_audits/AUD-0022-numeric-proofs-redundancy.md#aud-0022-f02-js-proven-narrowing-re-classifies-its-conversion-and-recovers-it-through-expect)
  - `Redundancy` | `analysis.numeric_proofs`
- [AUD-0022-F03: Integer-domain guards repeat `NumericScalar::integer_range`](./audits/open_audits/AUD-0022-numeric-proofs-redundancy.md#aud-0022-f03-integer-domain-guards-repeat-numericscalarinteger_range)
  - `Redundancy` | `analysis.numeric_proofs`

## Accepted

None.

## In progress

None.

## Blocked

None.
