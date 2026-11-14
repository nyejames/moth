# AUD-0015: MON schema preparation redundancy

- State: `complete`
- Kind: `Redundancy`
- Area: `crates.moth_mon.schema` — schema field validation and preparation in `crates/moth-mon/src/schema.rs`, focused on the `prepare_fields` to `prepare_field` handoff.
- Coverage: `partial`
- Reviewed: `2026-10`
- Baseline: Branch `feature/mon-crate-extraction`, reviewed tip `8b1229c119693e070fbf565824402c75453a7162`. The external reviewer source-traced that tip. Focused validation passed: `cargo test -q -p moth --lib` (5,460 passed); `cargo test -q -p moth-mon` (passed); clippy for `moth` and `moth-mon` clean; `moth check docs --terse` clean. The final `just validate-full` result for the exact closeout tree is recorded in the closeout commit message.

## What was inspected

The external review traced choice-payload field preparation through `prepare_fields` and `prepare_field`, including the default-policy check, function parameter and the private call site. It compared the owning `allow_defaults` decision and error path with the second branch. The review did not audit every schema-preparation policy or error path.

## Authorities read

`docs/roadmap/audit-guide.md`; `docs/roadmap/audit-kinds/redundancy.md`; `docs/roadmap/audits/README.md`; `docs/roadmap/audit-log.md`; `docs/roadmap/open-audit-findings.md`; and the schema-preparation ownership and error behavior in `crates/moth-mon/src/schema.rs`.

## Existing findings and active plans checked

The audit log and open-findings index were checked. AUD-0009-F03 also touches MON schema preparation, but owns decoded-byte charging before schema validation. This finding concerns only duplicated choice-payload default policy; it does not change name charging, failure ordering, default completion or resource accounting.

## Findings

### AUD-0015-F01: Choice-payload defaults are rejected twice in schema preparation

- State: `closed`
- Kind: `Redundancy`

#### Evidence

Before the correction, `prepare_fields` enforced `allow_defaults == false` by rejecting a field with a default before it called `prepare_field`. `prepare_field` then repeated the choice-payload default-policy check, even though the private helper had no independent caller that could bypass `prepare_fields`. The second branch was unreachable for choice-payload fields and duplicated the same policy and error construction.

#### Counter-explanation tested

A check inside `prepare_field` could protect that helper if another caller supplied fields directly. The call graph has one caller, `prepare_fields`, which has already enforced the policy before delegation. Keeping the second check therefore adds no independent safety boundary and leaves two places to update if the policy changes.

#### Violated contract or cost

The same schema rule had two owners within one preparation path. That duplicated validation and retained a parameter and branch whose choice-payload path could not be reached, increasing maintenance and obscuring which function owns the policy.

#### Root owner

Field-default validation in `crates/moth-mon/src/schema.rs`, owned by `prepare_fields` before it delegates to `prepare_field`.

#### Suggested correction

Keep choice-payload default rejection in `prepare_fields`, which knows whether defaults are allowed. Remove the redundant check and parameter from `prepare_field`.

#### Fix scope and preserved invariants

Resolved in the closeout commit following `8b1229c11`. `prepare_fields` is now the single owner of the choice-payload default policy; the parameter and unreachable branch were removed from `prepare_field`. Default rejection and its diagnostic path remain unchanged, as do name charging and completion behavior.

#### Required validation

Run the `moth-mon` tests that exercise schema defaults and choice-payload preparation, then run clippy for `moth-mon`. The focused `cargo test -q -p moth-mon` run passed, and clippy for `moth-mon` was clean. The final `just validate-full` result is recorded in the closeout commit message.

#### Linked findings

None. AUD-0009-F03 concerns schema budget charging, not default-policy ownership.

#### Triage record

2026-10-03 — **Accepted.** Retain one owner for choice-payload default rejection. `prepare_fields` already validates the policy before its only `prepare_field` call, so remove the duplicate branch and parameter without moving or widening schema validation.

2026-10-03 — **Fixed and closed.** The closeout commit following `8b1229c11` removes the duplicate policy check and parameter from `prepare_field`. `prepare_fields` remains the single owner. The focused `moth-mon` tests and clippy passed; the final `just validate-full` result is recorded in the closeout commit message.

## Checked and clean

Struct and record field defaults continue to be prepared through the existing `prepare_fields` path. Choice payload defaults remain rejected before `prepare_field`; schema field-name charging and other default completion behavior remain outside this change.

## Limitations

Coverage is partial and limited to the `prepare_fields`/`prepare_field` handoff and choice-payload default policy. The external review source-traced this path but did not audit all schema types, budget failure states or default-value conversions. This report does not claim a full schema correctness or redundancy audit.
