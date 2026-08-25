# Codebase Audit Guide

This document owns the rules for running a structured codebase audit: what to audit, how to record it, and what an audit may and may not change.

An audit is an **explicitly invoked** activity. It is not the Slice review in [AGENTS.md](../../AGENTS.md), which every non-trivial change ends with. The Slice review needs no scope, produces no report and never writes here. Do not block one on the other.

Focused checklists live under [audit-kinds](./audit-kinds/README.md). Reports live under [audits](./audits/README.md). Coverage lives in the [audit log](./audit-log.md). Unresolved work lives in [open findings](./open-audit-findings.md).

## What an audit is for

Two things, in this order:

1. **Produce useful findings** - concrete, evidenced problems a later task can fix.
2. **Record what was audited, where and when** - so the next audit can find the stale and never-reviewed code instead of re-reading what was just covered.

The second is why the audit log exists. An audit that finds nothing is still worth running and still records its coverage.

## Read before starting

Follow [AGENTS.md](../../AGENTS.md) task routing first. Then read:

1. The chosen checklist under [audit-kinds](./audit-kinds/).
2. The [audit log](./audit-log.md) row for the area, if it has one.
3. Open findings and any recent report for the same area.
4. The canonical authorities the area's subject matter routes to, plus the [progress matrix](../src/docs/progress/@page.moth) when current support status matters.

Do not blanket-read every design document. Follow the routing.

## Choosing what to audit

An audit run covers **one kind** and **one area**.

- **Kind** is what you are looking for. Pick it from the [audit-kind index](./audit-kinds/README.md), which owns the routing rules.
- **Area** is the code, documents or boundary you inspect. Prefer a row already in the audit log. Otherwise name one.

When the user names both, use them. When the user names one, pick the other: the least recently audited area for that kind, or the kind that row is missing. When the user names neither, pick any row with an empty or old cell, preferring code that has never been audited at all.

### Areas do not need to exist in advance

The audit log is a record of what has been audited, not a permission list. **If the area you need has no row, add one as part of your run.** State what it covers, run the audit, record the result. This is normal and expected: the log is built up by audits, not before them.

A pull request, commit range, changed-file list or plan phase is a filter for finding the area, not an area itself. Map it to the code it touches and name that.

### Sizing an area

An area is right-sized when one audit run can inspect it exhaustively and it has one clear owner. Too large means you cannot finish; too small means every run must read its siblings anyway.

Some audits are about a boundary rather than a body of code - a producer and its consumers, or two owners compared for parity. Name the boundary as the area and say what is on each side. There is no taxonomy to satisfy.

If you cannot cover the whole area, cover part of it and say which part. A partial audit that records its limits is useful. A partial audit claiming completeness is not.

## Running the audit

### 1. Reserve the report

Pick the next unused `AUD-####`, create the report from the skeleton in [audits/README.md](./audits/README.md), and add it under `Audits in progress` in [open findings](./open-audit-findings.md). Do this before inspecting anything, so concurrent runs collide visibly rather than silently.

### 2. Record the baseline

Note current validation state, known pre-existing failures, active plans touching the area, and anything that limits confidence. An audit may run against an unhealthy baseline, but it must say so and must not claim a gate passed when it did not run.

### 3. Inspect

Work through the chosen checklist. Read the area's entry points and ownership before local detail.

A directory listing, search hit count or test count is not evidence of coverage.

### 4. Challenge each finding before filing it

This is the step that separates a finding from an opinion. For each one:

- State the strongest reasonable counter-explanation and why you rejected it.
- Check whether the difference is a deliberate semantic, target or lifecycle distinction.
- Check open findings and active plans for the same root cause.
- Name the evidence that would disprove it.

Preference alone is not a finding. Performance needs measurement and attribution. Duplication needs equivalent behaviour and ownership, not similar text. Correctness needs a violated contract and a path that violates it.

### 5. File the finding

Use the report schema. Every finding needs: the problem, concrete evidence, the counter-explanation you tested, the contract or cost it violates, the root owner, and a suggested correction.

**A suggested correction does not authorise anything.** It seeds later work. Triage decides whether the finding is accepted; implementation decides the actual fix shape.

### 6. Close the run

- Mark coverage `complete` or `partial`, honestly.
- Add unresolved findings to [open findings](./open-audit-findings.md) and remove the report from `Audits in progress`.
- Update the audit log row for the area and kind you ran. Do not touch any other row.

## What an audit may change

An audit run is read-only with respect to production code, tests, benchmarks, fixtures, canonical documents and implementation status.

It may write **only**: its own report, the open-findings index, and the audit log.

## Authority

The `AGENTS.md` authority order controls. Within it:

1. Canonical design and standards documents define the accepted contract.
2. The progress matrix defines what is currently implemented.
3. The roadmap and active plans define sequencing and accepted deferral.
4. Existing tests are a fixed executable baseline.
5. Current behaviour is evidence, never authority over the above.

When two canonical documents conflict, record the exact conflict and route it to both owners. Do not pick one silently.

**Deferred is not broken.** Check the progress matrix before filing anything as a defect. Absence of deferred work is not a finding; accidental exposure of it may be.

**Do not change tests to fit a finding.** A non-test audit that needs coverage changed files a linked Tests finding instead. The same applies to documentation: link a Documentation finding rather than editing canonical prose under another kind.

## Preservation contract

Every accepted finding and every fix implementing one inherits these. A fix that breaks one is invalid, however much else it improves.

| Preserve | Meaning |
|---|---|
| Semantics | No change to accepted language, memory, compiler or build behaviour without an approved design change. |
| Support boundary | Do not report deferred work as a defect, or implement it incidentally. |
| Tests | Existing tests pass unmodified unless an accepted Tests finding names the exact change. |
| Diagnostics | Preserve or improve diagnostic identity, source context and recovery. |
| Outputs | Preserve observable output, public interfaces and artefacts unless the finding proves them wrong. |
| Determinism | Preserve deterministic identities, ordering and output. |
| Performance | No material regression, and no improvement claimed without measurement. |
| Ownership | Do not move responsibility across owners without an accepted boundary finding. |

Do not accept a fix that keeps the replaced path alive behind a wrapper, suppresses a failure instead of fixing it, moves work into a consumer when the fact belongs to a producer, or reconstructs something an earlier stage already owns.

## Findings

```text
candidate -> accepted -> fixed -> closed
```

A candidate may instead be rejected, superseded or blocked on a design decision. Reports keep the full history; the open-findings index keeps only unresolved work.

**Triage** is a separate decision after the report is complete. It confirms the evidence, the root owner and the fix boundary, then accepts, rejects or blocks. Acceptance authorises the bounded finding, not a particular patch.

**Implementation** is a separate task. Fix the root owner, delete the path you replace, and return the finding to triage if a correct fix needs a wider boundary than was accepted.

**Verification** is done by someone other than the implementer. Read the original evidence, inspect the changed boundary, confirm no parallel path survives, and confirm linked test or documentation changes were independently authorised. A green suite is necessary and not sufficient.

Use `just validate` for code-bearing fixes and the documentation release-build gate for documentation-only work. See the [validation guide](../src/developer-docs/style-guide/validation.mtf). Record exactly what ran, passed, failed and was not run.

## Recording coverage

The [audit log](./audit-log.md) holds one row per area, listing what has been audited and when.

- Only an audit run records coverage. Ordinary implementation work never does.
- Implementation may mark a row **stale** when it materially changes what that row records. Each checklist names what counts as material for its kind.
- A row with no entry for a kind has simply never been audited for it. That is the normal starting state and is exactly the signal the log exists to give.
