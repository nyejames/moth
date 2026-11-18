# Codebase Audit Guide

This document owns the rules for running a structured codebase audit: what to audit, how to record it, and what an audit may and may not change.

An audit is an **explicitly invoked** activity. It is not the Slice review in [AGENTS.md](../../AGENTS.md), which every non-trivial change ends with. The Slice review needs no scope, produces no report and never writes here. Do not block one on the other.

Focused checklists live under [audit-kinds](./audit-kinds/README.md). Every audit records coverage in the [audit log](./audit-log.md). **Correctness audits** record bugs in [bug reports](../bug-reports.md) and need no separate report. Other kinds use formal reports under [audits](./audits/README.md), with unresolved work in [open findings](./open-audit-findings.md) and resolution history in the [completed audits log](./completed_audits_log.md).

Any audit may also log an unrelated issue in `docs/bug-reports.md` without opening another audit or extending its coverage. The bug log is a lightweight investigation queue, not a second formal findings index.

## What an audit is for

Two things, in this order:

1. **Produce useful findings** - concrete, evidenced problems a later task can fix.
2. **Record what was audited, where and when** - so the next audit can find the stale and never-reviewed code instead of re-reading what was just covered.

The second is why the audit log exists. An audit that finds nothing is still worth running and still records its coverage.

## Read before starting

Follow [AGENTS.md](../../AGENTS.md) task routing first. Then read:

1. The chosen checklist under [audit-kinds](./audit-kinds/).
2. The [audit log](./audit-log.md) row for the area, if it has one.
3. [Bug reports](../bug-reports.md), relevant formal open findings, recent reports and earlier resolutions when checking for duplicate work.
4. The canonical authorities the area's subject matter routes to, plus the [progress matrix](../docs/progress/@entry.moth) when current support status matters.

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

### 1. Select the recording lane

For **Correctness**, use the lightweight lane: inspect the area, log bugs in `docs/bug-reports.md` and record coverage directly in `audit-log.md`. Do not reserve an `AUD-####`, create a report or add entries to the formal open-findings and completed-audits logs. Coordinate overlapping runs with active agents since no report reservation exists.

For **every other kind**, reserve the next unused `AUD-####` across both report folders. Create a report under `audits/open_audits/` from the [report skeleton](./audits/README.md) and add it under `Audits in progress` in [open findings](./open-audit-findings.md). Reserve before inspecting so concurrent formal runs collide visibly.

### 2. Establish the baseline

Check current validation state, known pre-existing failures, active plans touching the area and anything that limits confidence. Formal reports record this baseline. Correctness audits preserve material limitations in their coverage note or the relevant bug entry. Never claim a gate passed when it did not run.

### 3. Inspect

Work through the chosen checklist. Read the area's entry points and ownership before local detail.

A directory listing, search hit count or test count is not evidence of coverage.

### 4. Challenge each in-scope finding before filing it

This is the step that separates a finding from an opinion. For each one:

- State the strongest reasonable counter-explanation and why you rejected it.
- Check whether the difference is a deliberate semantic, target or lifecycle distinction.
- Check open findings and active plans for the same root cause.
- Name the evidence that would disprove it.

Preference alone is not a finding. Claims of faster execution or lower memory use need measurement and attribution. A Performance audit may also record demonstrably unnecessary work as unmeasured structural cleanup without claiming a speedup. Duplication needs equivalent behaviour and ownership, not similar text. A confirmed correctness bug needs a violated contract and a path that violates it. A credible but unproven concern may be logged for investigation if its uncertainty is explicit.

### 5. Record the result

**Correctness:** add a concise entry to `docs/bug-reports.md` for each distinct unresolved root cause. Name where it occurs, the observed or suspected wrong behaviour, the violated contract when established and the next useful investigation or reproduction. Mark unconfirmed hypotheses clearly. Check for an existing entry first and extend it rather than duplicating it. A clean audit produces no bug entry.

**Other kinds:** use the formal report schema. Each finding needs the problem, concrete evidence, the counter-explanation tested, the contract or cost it violates, the root owner and a suggested correction. Route an out-of-kind discovery through the incidental issue rule below.

**A suggested correction does not authorise anything.** Formal findings require triage before implementation. Bug-log entries require independent investigation before their fix scope is chosen.

### 6. Close the run

- Mark coverage `complete` or `partial`, honestly.
- Update the audit log row for the area and kind you ran. Do not touch any other row.
- **Correctness:** record the audited revision and, for partial coverage, the exact inspected subset and exclusions in the audit log. The area's `Covers` definition supplies the full-coverage boundary. No report or archive entry follows, even when no bugs were found.
- **Other kinds:** add unresolved findings to [open findings](./open-audit-findings.md) and remove the report from `Audits in progress`. If no unresolved findings remain, move it to `audits/closed_audits/`, record its closeout in the [completed audits log](./completed_audits_log.md) and update incoming links.

## Incidental discoveries in any task

An audit can expose a plausible bug, documentation gap, duplicated path, test weakness or efficiency concern unrelated to its chosen kind or area. **Record the lead in `docs/bug-reports.md` and continue the original audit.** Other implementation and review tasks may use the same queue. Do not discard an observation merely because it falls outside the current scope.

- Check for an existing entry first. Name the relevant code or document, what looked wrong and a concrete starting point for verification.
- Distinguish reproduced defects from unconfirmed suspicions. A brief, accurate lead is enough. Do not require full audit-level proof or an `AUD-####` report before recording it.
- Do not expand the current audit, claim coverage for another kind or fix the unrelated issue during this run.
- Do not fill the queue with ungrounded preferences or search hits. Record a specific observable concern, inconsistency or suspicious execution path.

The next task investigating the entry establishes the evidence, design authority and fix scope. If that work becomes a formal audit, its own report and coverage rules apply. Avoid tracking the same unresolved root cause independently in both queues.

## What an audit may change

An audit run is read-only with respect to production code, tests, benchmarks, fixtures, canonical documents and implementation status.

It may write **only** `docs/bug-reports.md` and the audit coverage log, plus its own report, the formal open-findings index and the report closeout records when the kind uses formal reports. These documentation-only writes record evidence, not implementations.

## Authority

The `AGENTS.md` authority order controls. Within it:

1. Canonical design and standards documents define the accepted contract.
2. The progress matrix defines what is currently implemented.
3. The roadmap and active plans define sequencing and accepted deferral.
4. Existing tests are a fixed executable baseline.
5. Current behaviour is evidence, never authority over the above.

When two canonical documents conflict, record the exact conflict and route it to both owners. Do not pick one silently.

**Deferred is not broken.** Check the progress matrix before filing anything as a defect. Absence of deferred work is not a finding; accidental exposure of it may be.

**Do not change tests to fit a finding.** A non-test audit that notices a separate test or documentation problem records an incidental lead in `docs/bug-reports.md`. A focused Tests or Documentation audit follows its own formal findings process. Do not edit tests or canonical prose under another kind.

## Preservation contract

Every accepted formal finding and every fix taken from the bug queue inherits these. A fix that breaks one is invalid, however much else it improves.

| Preserve | Meaning |
|---|---|
| Semantics | No change to accepted language, memory, compiler or build behaviour without an approved design change. |
| Support boundary | Do not report deferred work as a defect, or implement it incidentally. |
| Tests | Preserve coverage of accepted contracts. A verified bug fix may update an expectation proven wrong and add regression tests in the same scoped task. Unrelated test changes need their own accepted Tests finding. |
| Diagnostics | Preserve or improve diagnostic identity, source context and recovery. |
| Outputs | Preserve observable output, public interfaces and artefacts unless the finding proves them wrong. |
| Determinism | Preserve deterministic identities, ordering and output. |
| Performance | No material regression, and no improvement claimed without measurement. |
| Ownership | Keep each fact with its accepted owner. Review any boundary change against the canonical design and the approved fix scope. |

Do not accept a fix that keeps the replaced path alive behind a wrapper, suppresses a failure instead of fixing it, moves work into a consumer when the fact belongs to a producer, or reconstructs something an earlier stage already owns.

## Formal findings and resolution

This lifecycle applies to kinds that create `AUD-####` reports. Correctness bugs and incidental leads instead follow the lightweight queue in `docs/bug-reports.md`. Investigate and verify those entries before fixing them, then remove each entry in the commit that resolves it. Git history retains the removed text. Historical correctness reports keep their existing IDs and archives without migration.

```text
candidate -> accepted -> fixed -> closed
```

A candidate may instead be rejected, superseded or blocked on a design decision. Reports keep the full history. The open-findings index keeps only unresolved work. Record the evidence and reason for a rejected or superseded finding in its report before removing it from the index.

**Triage** is a separate decision after the report is complete. It confirms the evidence, the root owner and the fix boundary, then accepts, rejects or blocks. Acceptance authorises the bounded finding, not a particular patch.

**Implementation** is a separate task. Fix the root owner, delete the path you replace, and return the finding to triage if a correct fix needs a wider boundary than was accepted.

**Verification** is done by someone other than the implementer. Read the original evidence, inspect the changed boundary, confirm no parallel path survives, and check that regression tests enforce the verified contract. Unrelated test or documentation changes need separate authority. A green suite is necessary and not sufficient.

### Closing and archiving

- Keep a report in `audits/open_audits/` while any finding remains a candidate, accepted, in progress, blocked or fixed awaiting verification. A completed *run* may still have open findings.
- Once every finding is closed, rejected or superseded, move the report to `audits/closed_audits/`. A clean run with no findings also closes immediately. Preserve its ID, findings, evidence and triage history.
- Remove all of its entries from the open-findings index. Add one report entry and a concise resolution or no-findings record to `completed_audits_log.md`. Link to the moved report and retain useful validation evidence.
- Keep `audit-log.md` as the coverage ledger. Archiving does not change coverage or freshness. Update navigational links to the new location, without rewriting historical evidence paths inside reports.

Older reports archived before this folder rule may retain `fixed` or other historical status labels even where the resolution log records their closeout. Do not silently reopen them or rewrite their original evidence. Apply the verification gate to new closeouts.

Choose targeted checks, `just validate` or `just validate-full` by the branch checkpoint and affected behaviour. Use the documentation release-build gate for documentation-only branch checkpoints. Follow the [validation guide](../developer-docs/style-guide/validation.mtf) and record exactly what ran, passed, failed and was not run.

## Recording coverage

The [audit log](./audit-log.md) holds one row per area, listing what has been audited and when.

- Only an audit run records coverage. Adding an incidental issue or fixing a bug never does.
- Formal kinds link their report ID. New Correctness runs record the inspected revision directly, without an `AUD-####`. Include clear exclusions for partial runs. See the [audit log format](./audit-log.md).
- Implementation may mark a row **stale** when it materially changes what that row records. Each checklist names what counts as material for its kind.
- A row with no entry for a kind has simply never been audited for it. That is the normal starting state and is exactly the signal the log exists to give.
