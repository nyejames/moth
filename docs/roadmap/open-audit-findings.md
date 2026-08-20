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

None.

## Awaiting verification

None.

## Resolved in this branch

- AUD-0001-F01, AUD-0001-F02 and AUD-0001-F03 were corrected on branch `test-suite-honesty`. See each finding's triage record in [AUD-0001](./audits/AUD-0001-test-support-redundancy.md).
- AUD-0001-F04 was resolved in Phase 11 of the test-suite-honesty campaign, and its design gate turned out not to apply: the evidence recorded two `build_ast` implementations with different semantics, but there was one implementation reached through four names, so no consumer could have selected the wrong one. Corrected as a pure rename to `build_ast_with_registered_types`, `immutable_reference_expr` and `inferred_type_reference_expr`, with the bare forward deleted. See the resolution note in [AUD-0001](./audits/AUD-0001-test-support-redundancy.md#aud-0001-f04-sibling-fixture-supports-export-colliding-names-with-different-semantics).
- AUD-0001-F05 was resolved in the same phase: Phase 7 decided retire rather than adopt, and Phase 11 deleted `assert_diagnostic_reason` and `error_code_counts`. No token caller was added.
