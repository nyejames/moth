# Codebase Audit Guide

This document owns the shared rules for selecting, scoping, recording, accepting, fixing and verifying structured codebase audits.

It does not define the detailed method for each audit kind. Those rules belong in the audit-kind documents under `audit-kinds/`. Agent skills may route to these repository documents and use the audit log to select work, but the skills do not own or duplicate audit policy.

Related records:

- [Audit log](./audit-log.md) defines stable scopes and audit freshness.
- [Open audit findings](./open-audit-findings.md) indexes unresolved audit work.
- Audit reports under `audits/` own evidence, findings and completed audit records.
- [Roadmap](./roadmap.md) owns implementation order and genuinely deferred work.

An audit run is read-only with respect to production code, tests, benchmarks, fixtures, canonical design documents and implementation status. It may create or update audit reports, the audit log and the open-findings index. A separate task implements an accepted finding.

## Required authorities

Always read [AGENTS.md](../../AGENTS.md) and follow its task routing before an audit or fix.

For an audit, also read:

1. The audit-kind document.
2. The selected scope entry in the audit log.
3. Open findings and recent reports for the same scope and kind.
4. Every canonical authority routed by the affected domain.
5. The [progress matrix](../src/docs/progress/@page.moth) when current support or coverage matters.
6. The roadmap and active plans when work may be deferred, design-gated or already changing.
7. The primary implementation scope and all required context named by the scope entry.

Read the [testing guide](../src/docs/codebase/style-guide/testing.mtf) for test audits, coverage claims and any review of test ownership or assertions. Read the [validation guide](../src/docs/codebase/style-guide/validation.mtf) before accepting a fix as complete.

### Contract and evidence order

The repository authority order in `AGENTS.md` remains controlling. Within that order, classify audit evidence as follows:

1. The most specific canonical design or standards document defines the accepted contract.
2. The progress matrix defines the currently implemented support and coverage status.
3. The roadmap and active plans define sequencing, accepted deferral and design gates.
4. Existing tests form an immutable executable baseline for every non-test audit.
5. Current implementation behaviour is evidence, not authority over the sources above.

Public teaching pages, examples, cheatsheets, tests and current compiler behaviour do not override their canonical authorities.

When canonical authorities conflict, do not choose one silently. Record the exact conflict, route it to the owners of both documents and keep dependent implementation blocked until the conflict is resolved.

### Documentation and tests

Canonical documentation remains the source of truth. Existing tests remain fixed for every audit and fix except work authorised by an accepted test finding.

A non-test audit must not add, remove, rewrite, regenerate or weaken tests. When a valid code or diagnostic fix needs new or changed coverage, create a linked test finding. Both findings may be implemented together only after each has been accepted for its own change lane.

When a test conflicts with canonical documentation:

1. Do not change the test under a non-test audit.
2. Do not change production code to preserve behaviour that contradicts the canonical contract.
3. Record an authority-conflict finding with the exact sources in conflict.
4. Link the required test or documentation finding.
5. Keep implementation blocked until the conflict is resolved and accepted.

A documentation audit may correct inaccurate documentation. It cannot change accepted semantics under the label of documentation cleanup. A proposed semantic or architectural change is design-gated and requires explicit approval.

### Progress matrix

Correctness audits must use the progress matrix to distinguish defects from accepted incomplete work.

| Status | Audit interpretation |
|---|---|
| Supported | Behaviour inside the documented surface is expected to work. A mismatch is a correctness candidate. |
| Partial | Audit the implemented subset described by the row. Missing deferred edges are not automatically defects. |
| Experimental | Audit only when the selected scope includes the experimental implementation. Do not assume Alpha stability. |
| Deferred | Absence is not a correctness finding. Accidental exposure or inconsistent partial implementation may still be a finding. |

Coverage labels help prioritise test audits. They do not prove correctness or the absence of a defect.

If the matrix is ambiguous or stale, create a documentation or status finding. Do not invent the supported boundary.

## Audit scopes

The audit log records a graph of valid scopes rather than one flat directory partition.

Every scope has:

- a stable scope ID
- a primary audit scope that must be inspected exhaustively
- required context that must be read but is outside exhaustive coverage
- explicit exclusions
- one scope kind

Use architecture ownership first and filesystem layout second. A scope may contain several modules when they implement one owner. A system or contract scope may cross directories when the architecture does.

### Scope kinds

#### Leaf scope

The smallest useful ownership unit for exhaustive audit coverage.

Every production file belongs to exactly one leaf scope. Tests, fixtures and canonical documentation are linked context or owned surfaces rather than duplicate production ownership.

#### Composite scope

A group of leaf or smaller composite scopes that must be considered together for a broader system audit.

Composite scopes are valid for work such as system-wide redundancy, orchestration correctness and data-flow review.

#### Contract scope

A producer-consumer boundary whose correctness depends on both sides and the artefact passed between them.

Contract audits inspect ownership, information loss, reconstruction, validation placement and handoff invariants. They do not automatically become full audits of both systems.

#### Comparison scope

A declared group of independent owners that should be compared for repeated work, inconsistent policy or missed shared ownership.

Comparison does not imply that the implementations should be merged. Shared code is valid only when behaviour is genuinely identical and has one clear owner.

### Primary scope and required context

The primary scope is exhaustive. A complete audit must inspect every included file, module and relevant local test or support surface required by its audit kind.

Required context exists to answer whether the primary owner receives the right inputs, produces the right outputs and duplicates work owned elsewhere. Reading context does not authorise findings to expand their fix scope silently.

When a finding requires a change outside the primary scope:

1. Record the affected neighbouring scopes.
2. Explain why the local scope cannot own the root fix.
3. Classify the finding as a boundary escalation.
4. Keep the proposed write scope bounded.
5. Require separate triage before implementation crosses the declared boundary.

A scope can be closed for one audit kind and open for another. Each audit-kind document defines the valid scope sizes and required audit radius for that kind.

### Scope registry maintenance

Use stable dotted IDs based on conceptual ownership, such as `frontend.hir` or `build.output`. Do not encode a path that may move into the identity.

Update the registry when:

- production files gain no leaf owner
- a file would belong to more than one leaf scope
- ownership moves between systems
- an input or output contract changes
- a composite, contract or comparison group is no longer valid
- test ownership or required context changes materially

Structural implementation work may mark affected audit entries stale. It never promotes freshness.

## Audit kinds

The initial audit kinds are:

| Kind | Purpose |
|---|---|
| Style | Readability, organisation, API shape and compliance with implementation standards. |
| Comments | File documentation, non-local intent, stale prose and comment quality. |
| Correctness | Supported semantics, internal invariants, stage ownership and valid acceptance or rejection. |
| Diagnostics | User-error identity, source context, message quality, recovery and cascade control. |
| Tests | Behavioural coverage, test ownership, assertions, fixtures and redundant or obsolete coverage. |
| Redundancy | Repeated work, duplicate helpers, legacy paths, wrong-layer ownership and unjustified LOC. |
| Performance | Measured runtime, allocation, memory, traversal, scheduling and algorithmic efficiency. |
| Documentation | Accuracy, authority consistency, status, navigation and ownership documentation. |

Each finding has one primary kind. When one observation exposes work in another lane, create a linked finding rather than expanding the original kind.

An auditor may question any observed behaviour. It may only propose fixes allowed by its kind, scope and preservation contract.

## Selecting an audit

An explicit user-selected kind and scope always take priority. Do not run a kind whose audit-kind document does not yet exist unless the user supplies equivalent rules explicitly.

When only the kind is supplied, choose the least fresh valid scope for that kind. When only the scope is supplied, choose the least fresh valid kind for that scope. When neither is supplied, choose the least fresh valid kind and scope pair.

For every automatic selection:

1. Check the roadmap and active plans. Avoid auditing a scope that is in the middle of planned structural replacement unless the user asks for it.
2. Check open audits and findings to avoid duplicate work.
3. Prefer `N`.
4. Then prefer `S`.
5. Then prefer the oldest `P`.
6. Then prefer the oldest `C` when a fresh independent pass is useful.
7. Choose the smallest registered scope that satisfies the audit-kind document.
8. Record the chosen kind, primary scope, required context and exclusions before inspecting implementation.

When both are unspecified, select one audit kind and one registered scope. Do not turn a generic request into an unbounded multi-kind codebase review.

Freshness guides selection. It does not override active roadmap sequencing, known risk, explicit user priorities or an audit kind's valid scope requirements.

## Global preservation contract

Every accepted finding and fix inherits these rules.

| Invariant | Required preservation |
|---|---|
| Canonical semantics | Do not weaken, expand or reinterpret accepted language, memory, compiler, build or project semantics without an approved design change. |
| Current support boundary | Do not report deferred work as a defect or expose deferred behaviour incidentally. |
| Tests | Non-test findings cannot authorise any test change. Existing tests must pass unmodified unless a linked test finding was separately accepted. |
| Validation | The required final gate must pass. A failed gate is not a partial success. |
| Performance | Do not accept a material regression or claim an improvement without suitable evidence. |
| Diagnostics | Preserve or improve diagnostic identity, source context, recovery and user-error quality. |
| Outputs and artefacts | Preserve observable output, public interfaces and backend artefacts unless the accepted finding proves them incorrect. |
| Ownership | Do not move semantic or orchestration responsibility across declared owners without an accepted boundary finding. |
| Determinism | Preserve deterministic identities, diagnostics, ordering, output and cache or parallel behaviour. |
| Documentation | Non-documentation findings cannot authorise canonical or status documentation changes. Use a linked documentation finding when required. |
| Scope | Context is read-only unless an accepted boundary expansion names it as write scope. |

A fix that violates one of these invariants is invalid. Passing some tests, reducing LOC or improving one benchmark does not override the contract.

### Forbidden fix forms

Do not accept a fix that:

- changes, removes or weakens a test to accommodate production changes without an accepted test finding
- deletes a failing test or broadens an exact assertion merely to make validation pass
- changes the progress matrix or roadmap to hide broken behaviour
- rewrites canonical documentation to match an incorrect implementation
- preserves an obsolete path through a compatibility wrapper, forwarding shim or parallel API
- keeps old and new implementations active because deletion is difficult
- moves work into a consumer when the semantic fact belongs to a producer
- reparses or reconstructs facts already owned by another stage
- crosses a declared scope boundary without accepted escalation
- implements a deferred feature incidentally
- trades away diagnostic quality without explicit diagnostic scope
- accepts a material performance regression because the code is shorter or cleaner
- claims a performance improvement from code shape, timing noise or profiling alone
- treats benchmark fixtures as correctness coverage
- suppresses a failure, lint or invariant instead of fixing its cause

## Audit execution

### 1. Establish the audit record

Create one report under `audits/` using the next available ID:

```text
AUD-0001-short-description.md
```

Findings inside that report use stable IDs:

```text
AUD-0001-F01
AUD-0001-F02
```

One report represents one audit run over one primary kind and declared scope. A no-findings audit still needs a report before it can mark coverage current.

### 2. Check existing work

Before auditing implementation:

- search open findings for the same scope and kind
- read the latest linked reports for that scope and kind
- search `audits/` for relevant modules, symbols and root-cause terms
- check active roadmap plans for overlapping replacement work

Expand an existing finding when the root cause is the same. Create a linked child finding when new evidence reveals a materially broader problem. Mark an older finding superseded when a new finding replaces its explanation.

Do not silently rewrite the original evidence of an accepted or previously triaged finding. Append new evidence and record the relationship.

### 3. Establish baseline health

Record the known baseline before drawing conclusions:

- relevant test or validation state
- known pre-existing failures
- relevant benchmark stability for performance claims
- current progress-matrix status
- active changes that may invalidate the audit

An audit can continue with an unhealthy baseline, but it must name the limitation. It cannot claim that its own work passed a gate that was already failing or not run.

### 4. Inspect the scope

Read the primary scope exhaustively and the required context deeply enough to evaluate the selected kind.

Use module entry points and architecture documents to identify ownership before focusing on local details. Search adjacent paths when the kind requires comparison, duplication checks or contract validation.

Do not use a directory listing as proof of coverage. A complete audit must inspect the actual implementation and required evidence surfaces.

### 5. Record findings

A finding needs concrete evidence and a bounded preservation contract. Preference alone is not a finding.

Every candidate finding records:

- stable finding ID and title
- primary audit kind and scope
- observed problem
- concrete code or behaviour evidence
- violated authority, invariant or measurable cost
- impact
- allowed fix scope
- required read-only context
- preserved invariants
- forbidden fix forms specific to the finding
- required validation and measurement
- related, duplicate, superseded or blocked findings

A proposed direction may explain a likely root fix. It is not permission to implement an unreviewed design.

### 6. Classify cross-kind work

Route discoveries into separate findings when they require another lane.

Examples:

- A comment audit that exposes incorrect control flow creates a correctness finding.
- A correctness audit that needs new regression coverage creates a linked test finding.
- A diagnostic audit that changes expected diagnostic identity creates a linked test finding.
- A redundancy fix that moves files and makes the index inaccurate creates a linked documentation finding.
- A performance audit that finds only duplicate structure creates a redundancy finding rather than claiming speedup.
- Any audit that proposes different accepted semantics creates a design-gated finding.

### 7. Complete the audit

An audit is complete only when:

- every primary-scope file and owned surface required by the kind was inspected
- all required context was read
- the audit-kind checklist was completed
- existing reports and open findings were checked for duplication
- findings contain sufficient evidence and preservation contracts
- limitations and uninspected surfaces are explicit
- the report and open-findings index are updated
- the audit log freshness entry is updated accurately

Mark the entry partial when any required part was not covered. Do not mark an audit current because the auditor ran out of findings or context.

## Freshness

The audit log uses rough month-level freshness rather than commit pinning. The recorded month is the audit report month, including for a later stale entry.

- `N` means no complete audit is known.
- `P` means the report covered only part of the registered scope.
- `C` means the report completed the registered scope and no known material change has invalidated it.
- `S` means a material change invalidated a previously complete audit.

Freshness measures inspection coverage. It does not mean the scope is defect-free or has no open findings.

Only a complete audit report can promote an entry to `C`. A partial audit records `P`. Normal implementation work may keep an entry unchanged or mark it `S`. It cannot promote freshness.

Freshness is kind-specific. Examples:

- comment changes may stale the comment audit without staling correctness
- an algorithm rewrite may stale correctness, redundancy and performance
- a stage-interface change may stale correctness for both producer and consumer contract scopes
- test-suite restructuring may stale test coverage without staling style
- a canonical semantic change may stale correctness and documentation audits for affected scopes

The audit-kind documents define their own material invalidators.

## Finding lifecycle

Audit reports retain the complete history. The open-findings index contains only unresolved work.

```text
audit in progress
    -> candidate
    -> accepted and queued
    -> active fix
    -> awaiting verification
    -> closed
```

A candidate may instead become rejected, duplicate, superseded or blocked/design-gated.

### Candidate

The audit has recorded evidence, but the finding has not been accepted for implementation.

### Accepted and queued

The finding is valid, bounded and authorised for later implementation. Acceptance does not authorise design changes or work outside its preservation contract.

### Active fix

An implementation task owns the accepted finding. The open index links both the finding and the active implementation plan or change when one exists.

### Blocked or design-gated

The finding depends on unresolved authority conflict, design approval, another accepted finding or unavailable evidence. Do not implement around the blocker.

### Awaiting verification

Implementation is complete and the required gates have been reported. The original root cause and preservation contract still need independent review.

### Closed

Verification confirms that the root cause is removed, preserved invariants still hold and required validation passed.

The implementation agent does not mark its own finding verified. Verification belongs to a later read-only auditor or reviewer.

Rejected, duplicate and superseded findings remain in their reports but are removed from the open index.

## Implementing an accepted finding

Before editing:

1. Read the original finding and every linked finding that authorises code, tests or documentation changes.
2. Re-read current authorities and confirm that the finding is not stale or superseded.
3. Confirm the exact write scope and read-only context.
4. Establish the current test and performance baseline required by the finding.

During implementation:

- fix the root owner rather than a downstream symptom
- keep one current path and delete the replaced path
- do not broaden the task silently
- stop and return the finding to triage when the valid fix requires a new boundary, design decision or change lane
- preserve all unmodified tests
- keep performance and diagnostic evidence separate from intuition

A code fix may be implemented in the same change as linked test or documentation work only when those linked findings have been separately accepted.

If a fix breaks existing tests, regresses relevant performance, changes accepted semantics, weakens diagnostics or violates an architectural owner, revise or reject the fix. Do not update the preserved invariant to make the patch pass.

### Validation

Use the repository validation guide as the authority for the final gate.

- Code-bearing or mixed fixes end with `just validate` plus any finding-specific checks.
- Documentation-only audit records use the documentation release-build gate.
- Performance findings need the named non-recording benchmark or profiling evidence as well as the code-bearing final gate.
- Test findings need the suite policy and ownership checks required by the testing guide.
- A failed required gate remains a failed fix.

Record exactly what ran, what passed, what failed and what was not run.

## Verification

Verification is not a rerun of the implementation summary.

The verifier must:

1. Read the original evidence and preservation contract.
2. Inspect the root owner and every changed boundary.
3. Confirm that no obsolete or parallel path remains.
4. Confirm that linked test and documentation changes were independently authorised.
5. Check the reported validation and performance evidence.
6. Reproduce targeted checks where needed.
7. Confirm that the finding is fixed without weakening another invariant.
8. Close the finding or return it to the appropriate open state with new evidence.

A green test suite is required but is not enough on its own to verify architecture, redundancy, diagnostics or performance findings.

## Audit report skeleton

```markdown
# AUD-####: Audit title

- Kind: `<kind>`
- Primary scope: `<scope-id>`
- Required context: `<scope-ids or paths>`
- Coverage: `partial` or `complete`
- Reviewed: `YYYY-MM`
- Baseline: `<known validation and performance state>`

## Scope and exclusions

## Authorities read

## Existing findings checked

## Findings

### AUD-####-F01: Finding title

- State: `candidate`
- Kind: `<kind>`
- Scope: `<scope-id>`

#### Evidence

#### Violated contract or cost

#### Impact

#### Allowed fix scope

#### Read-only context

#### Must preserve

#### Forbidden fix forms

#### Required validation

#### Related findings

## No-finding checks

## Limitations

## Freshness update
```

## Open-findings entry form

Keep entries concise and link to the report that owns the evidence.

```markdown
- [AUD-####-F##: Finding title](./audits/AUD-####-short-description.md#finding-anchor)
  - `<kind>` | `<scope-id>`
```

An audit in progress links to the report as a whole. Do not copy evidence or implementation plans into the index.
