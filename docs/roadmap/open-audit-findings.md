# Open Audit Findings

This is the live index of unresolved audit work. Each entry links to its owning report under [audits](./audits/README.md). Evidence and analysis stay in the report. Closed, rejected, duplicate and superseded findings leave this index but remain in report and Git history.

- [Audit guide](./audit-guide.md)
- [Audit log](./audit-log.md)
- [Audit-kind index](./audit-kinds/README.md)

## Audits in progress

None.

## Candidate findings

None.

## Accepted and queued findings

None.

## Active fixes

None.

## Blocked or design-gated findings

- [AUD-0001-F04: Sibling fixture supports export colliding names with different semantics](./audits/AUD-0001-test-support-redundancy.md#aud-0001-f04-sibling-fixture-supports-export-colliding-names-with-different-semantics)
  - `Redundancy` | `tests.support`
  - Gated on the TypeId-first migration decision. Owned by [test-suite-honesty plan](./plans/test-suite-honesty-and-infrastructure-hardening-plan.md) Phase 11 item 3.
- [AUD-0001-F05: Two dead assertion helpers retained behind `#[allow(dead_code)]`](./audits/AUD-0001-test-support-redundancy.md#aud-0001-f05-two-dead-assertion-helpers-retained-behind-allowdead_code)
  - `Redundancy` | `tests.support`
  - Suppressions justified; the delete-or-adopt decision is owned by [test-suite-honesty plan](./plans/test-suite-honesty-and-infrastructure-hardening-plan.md) Phase 7 item 8, then Phase 11 item 2.

## Awaiting verification

None.

## Resolved in this branch

- AUD-0001-F01, AUD-0001-F02 and AUD-0001-F03 were corrected on branch `test-suite-honesty`. See each finding's triage record in [AUD-0001](./audits/AUD-0001-test-support-redundancy.md).
