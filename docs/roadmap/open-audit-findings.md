# Open Audit Findings

Unresolved **formal audit** findings only. The evidence and triage decision live in each report under [open audits](./audits/open_audits/). Once every finding in a report is resolved, move the report to [closed audits](./audits/closed_audits/) and record its resolution in the [completed audits log](./completed_audits_log.md). New Correctness audits and incidental discoveries use [bug reports](../bug-reports.md), not this index.

- [Audit guide](./audit-guide.md)
- [Audit coverage log](./audit-log.md)
- [Audit-kind index](./audit-kinds/README.md)

## Audits in progress

- [AUD-0028: Directive implementation redundancy](./audits/open_audits/AUD-0028-directive-implementation-redundancy.md)

## Candidate findings

- [AUD-0022-F01: Interval operator arithmetic is implemented twice](./audits/open_audits/AUD-0022-numeric-proofs-redundancy.md#aud-0022-f01-interval-operator-arithmetic-is-implemented-twice)
  - `Redundancy` | `analysis.numeric_proofs`
- [AUD-0022-F02: JS proven narrowing re-classifies its conversion and recovers it through `expect`](./audits/open_audits/AUD-0022-numeric-proofs-redundancy.md#aud-0022-f02-js-proven-narrowing-re-classifies-its-conversion-and-recovers-it-through-expect)
  - `Redundancy` | `analysis.numeric_proofs`
- [AUD-0022-F03: Integer-domain guards repeat `NumericScalar::integer_range`](./audits/open_audits/AUD-0022-numeric-proofs-redundancy.md#aud-0022-f03-integer-domain-guards-repeat-numericscalarinteger_range)
  - `Redundancy` | `analysis.numeric_proofs`
- [AUD-0024-F01: Proof-table timing comments name HIR validation where the contract requires final converged HIR](./audits/open_audits/AUD-0024-numeric-proofs-comments.md#aud-0024-f01-proof-table-timing-comments-name-hir-validation-where-the-contract-requires-final-converged-hir)
  - `Comments` | `analysis.numeric_proofs`
- [AUD-0024-F02: Proof-table key documentation calls Invoke operation ids statement ids](./audits/open_audits/AUD-0024-numeric-proofs-comments.md#aud-0024-f02-proof-table-key-documentation-calls-invoke-operation-ids-statement-ids)
  - `Comments` | `analysis.numeric_proofs`
- [AUD-0024-F03: JS branch-inspection landmark omits proven-narrowing Invokes](./audits/open_audits/AUD-0024-numeric-proofs-comments.md#aud-0024-f03-js-branch-inspection-landmark-omits-proven-narrowing-invokes)
  - `Comments` | `analysis.numeric_proofs`
- [AUD-0024-F04: `FunctionEmission` rationale narrates implementation history](./audits/open_audits/AUD-0024-numeric-proofs-comments.md#aud-0024-f04-functionemission-rationale-narrates-implementation-history)
  - `Comments` | `analysis.numeric_proofs`
- [AUD-0025-F01: Proof queries name operation ids `statement`](./audits/open_audits/AUD-0025-numeric-proofs-style.md#aud-0025-f01-proof-queries-name-operation-ids-statement)
  - `Style` | `analysis.numeric_proofs`
- [AUD-0025-F02: Divisor guards spell interval membership through positional endpoints](./audits/open_audits/AUD-0025-numeric-proofs-style.md#aud-0025-f02-divisor-guards-spell-interval-membership-through-positional-endpoints)
  - `Style` | `analysis.numeric_proofs`
- [AUD-0025-F03: `visit_statement` spells `HirWriteTarget` through crate paths](./audits/open_audits/AUD-0025-numeric-proofs-style.md#aud-0025-f03-visit_statement-spells-hirwritetarget-through-crate-paths)
  - `Style` | `analysis.numeric_proofs`

- [AUD-0027-F01: Signature-slot mismatch comparison is duplicated](./audits/open_audits/AUD-0027-host-contract-redundancy.md#aud-0027-f01-signature-slot-mismatch-comparison-is-duplicated)
  - `Redundancy` | `crates.moth_host.contracts`
- [AUD-0027-F02: Requirement export ordering has an unobservable tie-breaker](./audits/open_audits/AUD-0027-host-contract-redundancy.md#aud-0027-f02-requirement-export-ordering-has-an-unobservable-tie-breaker)
  - `Redundancy` | `crates.moth_host.contracts`

## Accepted

None.

## In progress

None.

## Blocked

None.
