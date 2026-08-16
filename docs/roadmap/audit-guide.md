# Codebase Audit Guide

This document owns the shared rules for selecting, scoping, recording, triaging, fixing and verifying structured codebase audits.

Focused procedures live under [audit-kinds](./audit-kinds/README.md). Reports live under [audits](./audits/README.md). Agent skills may select work and route to these documents, but they do not own or duplicate audit policy.

Related records:

- [Audit log](./audit-log.md) defines stable scopes and rough freshness.
- [Open audit findings](./open-audit-findings.md) indexes unresolved work.
- [Roadmap](./roadmap.md) owns implementation order and genuinely deferred work.

An audit run is read-only with respect to production code, tests, benchmarks, fixtures, canonical design documents and implementation status. It may create or update its report, the audit log and the open-findings index. A separate task implements accepted findings.

A broad sweep is an audit campaign made from separate kind-and-scope reports. Do not combine several kinds into one mixed report whose coverage cannot be tracked independently.

## Required authorities

Follow [AGENTS.md](../../AGENTS.md) and its task routing. This guide assumes those mandatory reading rules have already been applied.

For an audit, also read:

1. The selected audit-kind guide.
2. The selected scope entry in the audit log.
3. Open findings and recent reports for the same scope and kind.
4. Every canonical authority routed by the affected domain.
5. The [progress matrix](../src/docs/progress/@page.moth) when current support or coverage matters.
6. The roadmap and active plans when work may be deferred, design-gated or already changing.
7. The primary scope and all required context named by the scope entry.

Read the complete [testing standards](../src/docs/codebase/style-guide/testing.mtf) for test audits, coverage claims and reviews of test ownership or assertions. Read the [validation guide](../src/docs/codebase/style-guide/validation.mtf) before accepting a fix as complete.

## Authority and evidence

The repository authority order in `AGENTS.md` remains controlling. Within it:

1. The most specific canonical design or standards document defines the accepted contract.
2. The progress matrix defines current implementation and coverage status.
3. The roadmap and active plans define sequencing, accepted deferral and design gates.
4. Existing tests form an immutable executable baseline for every non-test audit.
5. Current implementation behaviour is evidence, not authority over the sources above.

Public teaching pages, examples, cheatsheets, tests and compiler behaviour do not override their canonical authorities.

When canonical authorities conflict, do not choose one silently. Record the exact conflict, route it to both document owners and keep dependent work blocked until the conflict is resolved.

### Documentation and tests

Canonical documentation remains the source of truth. Existing tests remain fixed except when an accepted Tests finding authorises an exact test-owned change.

A non-test audit must not add, remove, rewrite, regenerate or weaken tests. When a valid code or diagnostic fix needs new or changed coverage, create a linked Tests finding. The findings may be implemented together only after each is accepted for its own lane.

When a test conflicts with canonical documentation:

1. Do not change the test under a non-test audit.
2. Do not change production code to preserve behaviour that contradicts the canonical contract.
3. Record the exact authority conflict.
4. Create or link the required Tests or Documentation finding.
5. Keep implementation blocked until the conflict is resolved and accepted.

A Documentation audit may correct inaccurate documentation. It cannot change accepted semantics under the label of cleanup. Semantic or architectural change remains design-gated.

### Progress matrix

Correctness and Tests audits must use the progress matrix to separate defects and coverage gaps from accepted incomplete work.

| Status | Audit interpretation |
|---|---|
| Supported | Behaviour inside the documented surface is expected to work. A mismatch is a correctness candidate. |
| Partial | Audit the implemented subset named by the row. Missing deferred edges are not automatically defects. |
| Experimental | Audit only when the selected scope includes the experimental implementation. Do not assume Alpha stability. |
| Deferred | Absence is not a correctness finding. Accidental exposure or inconsistent partial implementation may still be a finding. |

Coverage labels guide test-audit priority. They do not prove correctness or the absence of a defect.

If the matrix is ambiguous or stale, create a Documentation finding. Do not invent the supported boundary.

## Audit scopes

The audit log records a graph of valid scopes rather than one flat directory partition.

Every scope has:

- a stable conceptual ID
- one scope kind
- a primary surface that must be inspected exhaustively
- default required context that must be read but is outside exhaustive coverage
- kind-specific context or applicability overrides where needed
- explicit exclusions

Use architectural ownership first and filesystem layout second. A scope may contain several modules when they implement one tightly coupled owner. A system or contract scope may cross directories when the architecture does.

### Scope kinds

- **Leaf**: the smallest useful ownership unit for exhaustive coverage. Every maintained implementation file belongs to exactly one leaf. Test cases, fixtures, benchmarks, canonical documents and generated outputs may be attached surfaces without becoming duplicate production owners. Test, benchmark and documentation tooling may own leaves when they contain maintained implementation.
- **Composite**: a group of leaf or smaller composite scopes that must be considered together for broader system work. Reference child IDs rather than repeating paths.
- **Contract**: a producer-consumer boundary and the artefact passed between them. It audits ownership, information loss, reconstruction, validation placement and handoff invariants without becoming a full audit of both systems.
- **Comparison**: independent owners compared for parity, repeated work or inconsistent policy. Comparison does not imply that code should be merged.

A leaf is too broad when one audit cannot inspect it exhaustively or it contains several independent owners. It is too narrow when every meaningful audit must always read several siblings as one owner.

A pull request, branch, commit range, roadmap item or changed-file list is a selection filter, not a scope kind. Map the affected files and contracts to registered scopes. Use an existing composite or separate reports when several scopes are involved. A changed-area review cannot mark a scope `C` unless it inspects the complete registered scope and satisfies the selected kind guide.

### Primary scope and required context

The primary scope is exhaustive. Required context is read deeply enough to judge inputs, outputs, ownership and repeated work, but it does not count as audited coverage.

The scope registry records default context. When one kind needs a different radius, use a compact kind-prefixed override, for example:

```text
Correctness: frontend.ast, analysis.borrow
Redundancy: frontend.ast, backend.js
```

Use `-` in the freshness matrix when a kind is not applicable. Do not leave an invalid pair as `N`, because automatic selection treats `N` as available work.

When a finding needs changes outside the primary scope:

1. Name every affected neighbouring scope.
2. Explain why the primary scope cannot own the root fix.
3. Classify the finding as a boundary escalation.
4. Keep the proposed write scope bounded.
5. Require separate triage before implementation crosses the boundary.

If a registered scope is incomplete, overlaps another leaf or assigns the wrong owner, do not claim complete coverage. Record the scope defect, update the registry through an accepted Documentation finding and mark the audit partial.

### Scope registry maintenance

Use stable dotted IDs based on conceptual ownership, such as `frontend.hir` or `build.output`. Do not encode a path that may move into the identity.

Review the registry when:

- a maintained implementation file has no leaf owner
- a file would belong to more than one leaf
- ownership moves between systems
- an input or output contract changes
- a composite, contract or comparison group becomes invalid
- test ownership or required context changes materially
- a kind becomes applicable or inapplicable

Structural work must review affected freshness cells and mark them stale when the relevant kind guide's invalidators apply. Normal implementation work never promotes freshness.

## Selecting an audit

Use the [audit-kind index](./audit-kinds/README.md) to select the primary kind and evidence threshold.

An explicit user-selected kind and scope always take priority. Do not run a kind whose guide does not exist unless the user supplies equivalent rules explicitly.

When only the kind is supplied, choose the least fresh applicable scope. When only the scope is supplied, choose the least fresh applicable kind. When neither is supplied, choose one least fresh applicable pair.

For automatic selection:

1. Check roadmap work and active plans. Avoid scopes under structural replacement unless requested.
2. Check audits in progress and open findings to avoid duplicate work.
3. Skip `-` cells.
4. Prefer `N`, then `S`, then the oldest `P`, then the oldest `C` when another independent pass is useful.
5. Choose the smallest registered scope that satisfies the kind guide.
6. Record the selected scope, required context, kind overrides and exclusions before inspection.

Freshness guides selection. It does not override explicit priorities, known risk, active sequencing or the radius required by the kind guide.

## Global preservation contract

Every accepted finding and fix inherits these constraints.

| Invariant | Required preservation |
|---|---|
| Canonical semantics | Do not weaken, expand or reinterpret accepted language, memory, compiler, build or project semantics without an approved design change. |
| Current support boundary | Do not report deferred work as a defect or expose deferred behaviour incidentally. |
| Tests | Non-test findings cannot authorise test changes. Existing tests pass unmodified unless an accepted linked Tests finding names the exact change. |
| Validation | The required final gate passes. A failed gate is not a partial success. |
| Performance | Do not accept a material regression or claim improvement without suitable evidence. |
| Diagnostics | Preserve or improve diagnostic identity, source context, recovery and user-error quality. |
| Outputs and artefacts | Preserve observable output, public interfaces and backend artefacts unless the accepted finding proves them incorrect. |
| Ownership | Do not move semantic or orchestration responsibility across owners without an accepted boundary finding. |
| Determinism | Preserve deterministic identities, diagnostics, ordering, output and cache or parallel behaviour. |
| Documentation | Non-documentation findings cannot authorise canonical or status-document changes. Use a linked Documentation finding. |
| Scope | Required context is read-only unless accepted triage expands the write scope. |

A fix that violates one of these constraints is invalid. Passing some tests, reducing LOC or improving one benchmark does not override the contract.

A performance change is material when it exceeds credible measurement variance or creates a meaningful regression on a representative workload. When evidence cannot distinguish regression from noise, keep the finding unresolved rather than accepting the trade-off by intuition.

### Forbidden fix forms

Do not accept a fix that:

- changes, removes or weakens tests to accommodate production changes without an accepted Tests finding
- deletes a failing test or broadens an exact assertion merely to make validation pass
- changes the progress matrix or roadmap to hide broken behaviour
- rewrites canonical documentation to match incorrect implementation
- preserves an obsolete path through a compatibility wrapper, forwarding shim or parallel API
- keeps old and new implementations active because deletion is difficult
- moves work into a consumer when the fact belongs to a producer
- reparses or reconstructs facts already owned by another stage
- crosses a scope boundary without accepted escalation
- implements a deferred feature incidentally
- trades away diagnostic quality without accepted Diagnostics scope
- accepts a material performance regression because code became shorter or cleaner
- claims performance improvement from code shape, timing noise or profiling alone
- treats benchmark fixtures as correctness coverage
- suppresses a failure, lint or invariant instead of fixing its cause

## Audit workflow

### 1. Reserve the report

Choose the next report ID, create its skeleton under `audits/` and add it to `Audits in progress` before inspecting implementation. This reserves the ID and exposes concurrent work.

One report represents one run over one primary kind and registered scope. A no-findings audit still needs a report before it can mark coverage current. Naming and the report skeleton live in [audits/README.md](./audits/README.md).

### 2. Check existing and changing work

Before auditing implementation:

- search open findings for the same scope and kind
- read the latest reports for that pair
- search `audits/` for relevant modules, symbols and root-cause terms
- check active plans and recent structural work
- identify earlier findings that should be expanded instead of duplicated

Expand an existing finding when the root cause is the same. Create a linked child when new evidence proves a materially broader problem. Mark an older finding superseded when a new finding replaces its explanation.

Do not silently rewrite accepted or previously triaged evidence. Append evidence and record the relationship.

### 3. Establish baseline and coverage inventory

Record:

- current progress-matrix status
- relevant validation, test and benchmark state
- known pre-existing failures
- active changes that limit confidence or freshness
- every primary file, owned surface and required context path to inspect
- explicit exclusions and unavailable evidence

A revision SHA is optional report metadata, not part of the freshness system.

An audit may continue with an unhealthy or partly unknown baseline, but it must name the limitation. It cannot claim a gate passed when it was already failing or not run.

### 4. Inspect using the kind guide

Read the primary scope exhaustively and required context deeply enough to apply the selected procedure.

Use module entry points and architecture documents to establish ownership before local details. Search alternate entry points, callers, consumers, tests and comparison owners required by the guide.

A directory listing, search-result count or test count is not proof of coverage.

### 5. Challenge provisional findings

Before filing:

- test the strongest reasonable counter-explanation
- inspect relevant callers, alternate paths and owners
- check whether the difference is an intentional semantic, target, lifecycle or failure distinction
- check previous findings and plans for the same root cause
- identify evidence that would disprove the claim
- mark coverage partial when required evidence is unavailable

For duplication, prove behaviour and ownership are equivalent before recommending sharing. For correctness, trace the violated outcome or invariant. For performance, measure the symptom and attribute the cost. Preference alone is not a finding.

### 6. Record bounded findings

Every candidate records:

- stable ID, title, state, primary kind and scope
- observed problem and concrete evidence
- counter-evidence or alternative explanation checked
- violated authority, invariant or measurable cost
- impact and root owner
- suggested correction, marked as non-authorising
- allowed fix scope and required read-only context
- preserved invariants and forbidden fix forms
- required validation or measurement
- dependencies, sequencing and related findings

A suggested correction may seed an implementation plan. It does not approve a design or broaden the write scope.

### 7. Complete the audit

An audit is complete only when:

- every listed primary surface was inspected
- all required context was read
- the kind procedure and completion checklist were completed
- existing reports and open findings were checked for duplication
- provisional findings were challenged
- findings contain enough evidence and bounded preservation contracts
- limitations and uninspected surfaces are explicit
- the report and open-findings index are updated
- the audit-log cell is updated accurately

Mark the audit partial when any required part was not covered. Do not mark it current because the auditor found nothing else or ran out of context.

## Triage

Triage is a separate decision after the report is complete. It does not edit production code.

For each candidate:

1. Confirm authority and current support boundary.
2. Confirm the primary kind and registered scope.
3. Confirm evidence, counter-evidence and root ownership.
4. Confirm fix boundary and preserved invariants.
5. Check duplicate, superseding and dependency relationships.
6. Decide: accepted, rejected, duplicate, superseded or blocked/design-gated.
7. For accepted work, assign priority, dependencies and linked change lanes.

Acceptance authorises only the bounded finding. It does not make the suggested patch shape mandatory when implementation finds a safer root fix inside the same contract.

## Finding lifecycle

Reports retain complete history. The open-findings index contains only unresolved work.

```text
audit in progress
    -> candidate
    -> accepted and queued
    -> active fix
    -> awaiting verification
    -> closed
```

A candidate may instead become rejected, duplicate, superseded or blocked/design-gated.

The implementation agent does not verify its own finding. Rejected, duplicate and superseded findings remain in reports but leave the open index.

## Implementing an accepted finding

Before editing:

1. Read the finding and every linked finding that authorises code, test or documentation changes.
2. Re-read current authorities and confirm the finding is not stale or superseded.
3. Confirm the exact write scope and read-only context.
4. Establish the test and performance baselines required by the finding.

During implementation:

- fix the root owner rather than a downstream symptom
- keep one current path and delete the replaced path
- prefer explicit data-oriented flow and stage-owned artefacts over object-style indirection
- do not broaden the task silently
- return the finding to triage when a valid fix needs a new boundary, design decision or change lane
- preserve every test not covered by an accepted Tests finding
- keep performance and diagnostic evidence separate from intuition

Code, test and documentation work may share a change only when every linked finding has been accepted.

If a fix breaks preserved tests, regresses relevant performance, changes accepted semantics, weakens diagnostics or violates an owner, revise or reject it. Do not edit the preserved invariant to make the patch pass.

### Validation

Use the validation guide as the final-gate authority.

- Code-bearing or mixed fixes end with `just validate` plus finding-specific checks.
- Documentation-only reports and fixes use the documentation release-build gate.
- Performance fixes need named non-recording measurements as well as the code-bearing gate.
- Test fixes need the suite policy and ownership checks required by the testing standards.
- A failed required gate remains a failed fix.

Record exactly what ran, passed, failed and was not run.

## Verification

Verification is not a summary of the implementation task.

The verifier must:

1. Read the original evidence, countercheck and preservation contract.
2. Inspect the root owner and every changed boundary.
3. Confirm no obsolete or parallel path remains.
4. Confirm linked test and documentation changes were independently authorised.
5. Check validation and performance evidence.
6. Reproduce targeted checks where needed.
7. Confirm the finding is fixed without weakening another invariant.
8. Apply freshness invalidators to every affected scope and kind.
9. Close the finding or return it to the right open state with new evidence.

A green suite is required but is not enough by itself to verify architecture, redundancy, diagnostics or performance findings.

Implementing a finding does not automatically stale its originating audit. Apply the kind-specific invalidators. Narrow verified corrections may keep `C` when all affected checklist areas were rechecked. Material restructuring becomes `S` unless a complete audit was rerun.

## Freshness

Marker meanings live in the [audit log](./audit-log.md). Freshness uses rough month-level records rather than required commit pinning.

The month and report ID identify the most recent audit record, including an entry later marked stale. Freshness measures inspection coverage. It does not mean the scope is defect-free or has no open findings.

Only a complete report can promote an entry to `C`. A partial report records `P`. Implementation and verification may retain a cell or mark it `S`. They never promote it without completing the audit-kind procedure.

Freshness is kind-specific. Each kind guide defines its material invalidators.
