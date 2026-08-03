# Final Memory-Management Documentation Consistency Cleanup Plan

**Status:** complete  
**Repository:** `nyejames/moth`  
**Baseline reviewed:** commit `d45244f58b662e8d58833cd4a77f1c2db0e8e31c`  
**Change class:** documentation-only  
**Primary objective:** remove the final cross-document inconsistencies after the memory-management authority migration and follow-up correction  
**Required final gate:** `moth build docs --release` or `cargo run --quiet -- build docs --release`  
**Out of scope:** Rust code, tests, fixtures, manifests, compiler behaviour, backend behaviour, runtime behaviour and new semantic design  

## Current state

ACTIVE_PLAN: none (this plan is complete)
STATUS: complete
CURRENT_SLICE: none
LAST_ACCEPTED_COMMIT: pending parent commit
WORKTREE: main; unrelated parallel compiler/src changes preserved and not staged
REQUIRED_RELOADS: startup files, this plan, current source/diff
RELEVANT_CONTEXT_NOW:
- docs: map ownership, result-freshness, build-system, compiler, canonical-module plan, educational pages, links and metadata synchronized
- code: none (documentation-only)
ACCEPTANCE_CRITERIA:
- all phase acceptance criteria satisfied
VALIDATION_STATE:
- `moth check docs`: passed
- `moth build docs --release`: passed; 65 files
DOCS_IMPACT: memory/compiler/build/language/collections/bindings/canonical-module-plan/progress/roadmap updated; `docs/release/**` regenerated
BLOCKERS_OR_OPEN_DECISIONS: none
DELEGATION_DECISION: parent-direct - documentation-only parent-owned work
NEXT_WORKER_ORDER: none
STOP_REASON: final consistency cleanup complete
NEXT_RESUME_ACTION: none; compiler behaviour changes require a separate approved implementation plan

---

## 1. Purpose

The memory-management design itself is complete. This plan performs the final deterministic synchronization pass needed before the documentation can be considered closed.

The implementing agent must:

- preserve the current canonical memory-management design;
- keep the deleted legacy monolith deleted;
- correct the remaining user-facing map ownership contradiction;
- finish migration from ambiguous `fresh` terminology to the accepted three-part result classification;
- remove mechanical defects from the build-system authority;
- align the compiler authority and active canonical-module implementation plan with the accepted lifetime artefact boundaries;
- synchronize the remaining educational pages;
- fix completion metadata and minor links;
- regenerate documentation output through the normal build;
- avoid introducing any new language, compiler, lifetime or backend design.

This is not another design exercise. Every semantic correction is fixed below.

---

## 2. Authority order

Use this order whenever documents disagree:

1. `docs/src/docs/codebase/memory-management/overview.mtf`
2. The relevant detailed memory leaf
3. `docs/compiler-design-overview.md`
4. `docs/build-system-design.md`
5. `docs/src/docs/codebase/language/overview.mtf` and its canonical memory references
6. `docs/src/docs/progress/@page.moth`
7. Roadmap plans for sequencing only
8. Educational and user-facing pages
9. Current implementation code

Current code may drift from the design. Do not edit code or weaken documentation to match drift.

---

## 3. Locked semantic rules

### 3.1 Map storage

Use exactly:

> **Maps own their entry structure. Existing keys and values stored in entries follow the ordinary shared-reference, explicit-copy and inferred-transfer rules.**

Use exactly:

> **`remove` removes the entry and returns the removed value under the normal lifetime and ownership rules.**

Do not state that maps semantically own every stored key and value as independently owned children.

Do not state that `remove` always returns an “owned value” as a source-language category.

### 3.2 Result classification

Use these terms consistently:

| Term | Meaning |
|---|---|
| **Fresh result root** | The result root allocation is newly created. It may retain legal references to older allocations. |
| **Alias result** | The result root is an existing argument, projection, external allocation or another result. |
| **Independent result graph** | The complete result graph has no mutable sharing or retained Moth reference to pre-existing storage. |

Rules:

- WIT value-only lifting produces an **independent result graph**.
- Explicit `copy` produces an **independent result graph** while preserving internal alias topology inside the copy.
- A constructor, template or computed aggregate normally produces a **fresh result root**, not necessarily an independent result graph.
- A hidden result destination is valid for a **fresh result root** when all retained edges are legal for the destination.
- Do not use bare `fresh result` where the distinction matters.
- Do not use a generic `Fresh` marker as a substitute for a WIT boundary classification.

### 3.3 Lifetime facts in artefacts

Accepted architecture:

- module executable state includes local lifetime-region and escape facts;
- public semantic interfaces include stable lifetime and effect summaries;
- generated sidecars carry generated lifetime facts and summaries;
- donor-local region IDs do not cross module boundaries;
- project/link planning instantiates local summaries into complete topology;
- target validation runs after complete project/link lifetime-topology validation.

Do not claim the deferred lifetime analysis implementation has landed.

### 3.4 External boundaries

WIT value-only calls:

- are non-consuming;
- lower arguments from shared reads;
- produce independent component values;
- lift results into independent Moth result graphs;
- transfer no Moth alias, lifetime owner, ownership state or destruction responsibility.

Restricted host bindings:

- pass ordinary values by value as non-retained inputs;
- permit mutable host access only to opaque foreign handles;
- are non-consuming for ordinary host-value crossings;
- do not use the ordinary Moth ownership ABI to transfer Moth storage.

### 3.5 Final-use child extraction

Final-use child extraction is not accepted current design.

Use:

> **Interior projections remain rooted in their containing allocation family. A proven final use may transfer the containing allocation family, but it does not detach one projected child into a new independent allocation.**

The roadmap may retain the future investigation note only.

### 3.6 Status

These remain deferred implementation:

- lifetime-region and escape validation;
- declared memory groups;
- WIT value-only component integration;
- ownership-aware backend completion.

Do not label them active or queued unless a separate approved plan changes roadmap status.

---

## 4. Definition of completion

- [x] Map ownership wording is consistent in canonical, language and user-facing collection documentation.
- [x] `remove` uses normal lifetime/ownership wording everywhere.
- [x] Ambiguous result freshness wording is removed from live design and user-facing docs.
- [x] WIT results are consistently described as independent result graphs.
- [x] Hidden destination wording consistently uses fresh result roots plus retained-edge constraints.
- [x] The build-system duplicate sentence is removed.
- [x] The module-job pipeline indentation is corrected.
- [x] Dormant root work includes local lifetime analysis.
- [x] Generated worklist ownership includes lifetime facts and summaries.
- [x] Compiler companion authority wording includes lifetime topology and declared groups.
- [x] Public interface and fingerprint terminology includes optional transfer, alias, retention, outlives and external-boundary summaries.
- [x] Generated function artefact wording includes lifetime facts and summaries.
- [x] The compiler authority includes the closed external-profile transfer override.
- [x] The active canonical-module plan matches accepted artefact lanes without claiming lifetime implementation has landed.
- [x] The active canonical-module plan status block reflects current repository state.
- [x] The remaining educational stage wording is synchronized.
- [x] Follow-up completion-plan metadata names no active plan and records the accepted commit.
- [x] Related-reading labels and links are correct.
- [x] Source searches find no unexplained contradiction.
- [x] The docs release build passes.
- [x] Generated routes and diffs are manually inspected.

---

# Phase 0 — Establish the baseline and protect scope

## Tasks

- [ ] Confirm HEAD contains commit `d45244f58b662e8d58833cd4a77f1c2db0e8e31c`.
- [ ] Record current branch, HEAD and changed-file list.
- [ ] Read:
  - `AGENTS.md`
  - `CONTRIBUTING.md`
  - `docs/src/docs/codebase/style-guide/style-guide.mtf`
  - `docs/src/docs/codebase/style-guide/validation.mtf`
  - all memory authority pages
  - `docs/compiler-design-overview.md`
  - `docs/build-system-design.md`
  - `docs/src/docs/codebase/language/overview.mtf`
  - `docs/src/docs/memory/*.mtf`
  - `docs/src/docs/progress/@page.moth`
  - `docs/roadmap/plans/canonical-module-compilation-and-scoped-packages-plan.md`
  - `docs/roadmap/plans/grouped-memory-design.md`
  - both completed memory documentation plans
- [ ] Confirm this slice contains no source, test, fixture, manifest or build-script edit.
- [ ] Preserve unrelated staged or unstaged work.
- [ ] Do not edit `docs/release/**` manually.

## Acceptance criteria

- [ ] Baseline is explicit.
- [ ] Work remains documentation-only.
- [ ] Unrelated work is untouched.

---

# Phase 1 — Correct map ownership across language and user documentation

## Files

```text
docs/src/docs/collections/hash-maps.mtf
```

## 1.1 Hash-map language reference

Replace:

```markdown
- Maps own stored keys and values.
```

with:

```markdown
- Maps own their entry structure. Existing keys and values stored in entries follow the ordinary shared-reference, explicit-copy and inferred-transfer rules.
```

Replace:

```markdown
- `remove` returns the owned removed value.
```

with:

```markdown
- `remove` removes the entry and returns the removed value under the normal lifetime and ownership rules.
```

Retain borrowed lookup keys, shared `get` results, insertion order and backend-status text.

## 1.2 User-facing hash-map page

Replace:

```markdown
Maps own their stored keys and values.
```

with:

```markdown
Maps own their entry structure. Existing keys and values stored in entries follow the ordinary shared-reference, explicit-copy and inferred-transfer rules.
```

Replace the `remove(key)` result bullet:

```markdown
- returns the owned removed value
```

with:

```markdown
- removes the entry and returns the removed value under the normal lifetime and ownership rules
```

## Acceptance criteria

- [ ] No live page says maps own all stored keys and values as independently owned children.
- [ ] No live page says `remove` always returns an “owned value.”
- [ ] Syntax and backend-status wording are unchanged.

---

# Phase 2 — Finish result-freshness terminology migration

## 2.1 Lifetime-region detailed page

File:

```text
docs/src/docs/codebase/memory-management/lifetime-regions-and-escape-validation/lifetime-regions-and-escape-validation.mtf
```

Replace:

```markdown
results are lifted into fresh Moth values
```

with:

```markdown
results are lifted into independent Moth result graphs
```

## 2.2 External-binding WIT wording

File:

```text
docs/src/docs/packages/external-binding-contracts.mtf
```

Replace:

```markdown
results lift into fresh Moth values
```

with:

```markdown
results lift into independent Moth result graphs
```

## 2.3 Declared-groups hidden result destinations

File:

```text
docs/src/docs/codebase/memory-management/declared-memory-groups/declared-memory-groups.mtf
```

Replace the ambiguous hidden-result prose with:

```markdown
Functions whose result root is fresh may allocate that root directly into a caller-selected destination. The function summary classifies the result root as fresh and records retained-edge constraints separately.

A hidden destination:

- is not part of the source signature
- is not a region parameter visible to generics or callers
- is not a source lifetime parameter
- may be ignored by a GC backend
- lets an optimising backend allocate a fresh result root directly into the caller's region when every retained edge is legal for that destination

A fresh-result-root summary and its retained-edge constraints must be backend-neutral parts of the function's semantic effect information.
```

## 2.4 Ownership page

File:

```text
docs/src/docs/codebase/memory-management/ownership-and-drops/ownership-and-drops.mtf
```

Replace:

```markdown
Hidden destination allocation may produce fresh results directly into a group.
```

with:

```markdown
Hidden destination allocation may produce a fresh result root directly in a group when every retained edge is legal for that destination.
```

## 2.5 Grouped-memory roadmap

File:

```text
docs/roadmap/plans/grouped-memory-design.md
```

Make these replacements:

- `fresh calls` → `calls with fresh result roots`
- `fresh-result functions` → `functions whose result root is fresh`
- `fresh result graph` → `fresh result root` unless total graph independence is intended
- `fresh-result summary` → `fresh-result-root summary plus retained-edge constraints`
- `hidden fresh-result destination support` → `hidden destination support for fresh result roots`

Replace:

```markdown
fresh calls may allocate directly into a hidden caller-selected destination region
```

with:

```markdown
calls whose result root is fresh may allocate that root directly into a hidden caller-selected destination region when retained-edge constraints permit it
```

## 2.6 Explicit-copy user page

File:

```text
docs/src/docs/bindings/explicit-copies-basic.mtf
```

Replace:

```markdown
Do not add `copy` to literals, constructor calls or computed expressions. Those
expressions already create fresh results.
```

with:

```markdown
Do not apply `copy` directly to literals, constructor calls or computed expressions; `copy` accepts places only. Those expressions create new result roots, but existing values stored inside them still follow ordinary shared-reference rules.
```

## 2.7 Search gate

Run:

```sh
rg -n   "\bfresh result\b|\bfresh results\b|fresh-result|fresh Moth|returns fresh storage|result is fresh"   README.md docs index.md
```

For each hit:

- use `fresh result root` for a new root;
- use `independent result graph` for `copy` and WIT lifting;
- keep `fresh value` only where graph independence is not implied.

## Acceptance criteria

- [ ] WIT results are always independent result graphs.
- [ ] Hidden destinations refer to fresh result roots and retained-edge constraints.
- [ ] No user-facing page implies every computed result graph is independent.
- [ ] Roadmap terminology is consistent.

---

# Phase 3 — Repair build-system authority defects

## File

```text
docs/build-system-design.md
```

## 3.1 Remove duplicate sentence

Under `Link planning and lifetime topology`, keep only one copy of:

```markdown
`ProjectCompilation` or the link plan conceptually carries project-level validated lifetime topology. Exact Rust shape remains open.
```

## 3.2 Fix module-job pipeline indentation

Replace the module-job block with exactly:

```text
receive retained syntax and completed provider interfaces
-> bind import shells
-> order local declarations
-> run AST semantics
-> lower and validate HIR
-> borrow-validate
-> produce local lifetime constraints, lifetime facts and exported summaries
-> return Success or Diagnosed
```

## 3.3 Complete dormant-root wording

Replace:

```markdown
Every normal module in the command's semantic graph has dormant root work fully compiled and borrow-validated. Entry assembly activates already compiled work only.
```

with:

```markdown
Every normal module in the command's semantic graph has dormant root work fully compiled, borrow-validated and locally lifetime-analysed. Entry assembly activates already compiled work only.
```

## 3.4 Complete generated-worklist ownership

Replace:

```markdown
The compiler owns generic template validation, call-site inference, request identity, generated HIR and generated borrow facts.
```

with:

```markdown
The compiler owns generic template validation, call-site inference, request identity, generated HIR, generated borrow facts, and generated lifetime facts and summaries.
```

## 3.5 Preserve one canonical pipeline

Confirm the only canonical mixed-target sequence is:

```text
entry or package roots
-> exact reachable function and effect union
-> instantiate local lifetime summaries with builder lifecycle roots
-> validate complete lifetime topology
-> target-affinity and capability analysis
-> deterministic target partition
-> validate assigned functions and permitted cross-target edges
-> lower selected functions
```

## Acceptance criteria

- [ ] Duplicate sentence removed.
- [ ] Pipeline indentation is linear.
- [ ] Dormant root work includes local lifetime analysis.
- [ ] Generated worklist includes lifetime facts and summaries.
- [ ] No competing shorter pipeline remains.

---

# Phase 4 — Complete compiler authority synchronization

## File

```text
docs/compiler-design-overview.md
```

## 4.1 Companion authority wording

Replace:

```markdown
- `docs/src/docs/codebase/memory-management/overview.mtf` for access, borrow, GC, ownership and destruction semantics
```

with:

```markdown
- `docs/src/docs/codebase/memory-management/overview.mtf` for reference semantics, borrow validation, lifetime topology, declared groups, ownership, GC and backend memory lowering
```

## 4.2 Public semantic interface

Replace:

```markdown
- mutation, possible consumption, return-alias and relevant reactive effect summaries
```

with:

```markdown
- mutation, optional transfer eligibility and effect categories
- return-alias and projection-alias summaries
- retained-parameter and outlives summaries
- external-boundary classifications
- relevant reactive effect summaries
```

## 4.3 Public-interface validation

Replace the generic `access and effect summaries` bullet with:

```markdown
- access, optional-transfer, alias, retention, outlives, external-boundary and reactive summaries
```

## 4.4 Generated concrete function artefacts

Add:

```markdown
- generated lifetime-region and escape facts
- generated exported lifetime and effect summaries
```

The final generated artefact list must include:

- request identity;
- generated-local type environment;
- validated HIR;
- borrow facts;
- lifetime facts;
- lifetime/effect summaries;
- link facts;
- fingerprints.

## 4.5 Public-interface fingerprint

Replace:

```markdown
- access and effect summaries
```

with:

```markdown
- access, optional-transfer, alias, retention, outlives and external-boundary summaries
```

Mention reactive summaries explicitly if not already covered.

## 4.6 Borrow-validation external override

Immediately after the general parameter-transfer rule, add:

```markdown
Closed external boundary profiles override this general rule. WIT value-only calls and restricted host-value crossings are non-consuming. Mutable opaque-handle access does not transfer Moth storage through the ordinary Moth ownership ABI.
```

## Acceptance criteria

- [ ] Companion authority includes lifetime topology and declared groups.
- [ ] `possible consumption` is absent from canonical interface wording.
- [ ] Generated artefacts include lifetime facts and summaries.
- [ ] Fingerprints explicitly include topology-relevant summaries.
- [ ] External profiles override the general transfer rule.

---

# Phase 5 — Align the active canonical-module implementation plan

## File

```text
docs/roadmap/plans/canonical-module-compilation-and-scoped-packages-plan.md
```

## 5.1 Refresh status block

- [ ] Update `WORKTREE` to the current branch/HEAD state.
- [ ] Remove the stale statement that memory-documentation additions are staged.
- [ ] Preserve the current active milestone and accepted compiler checkpoint.
- [ ] Do not make memory implementation active.

## 5.2 Generated-request flow

Use:

```text
module AST and validated base HIR
-> enqueue generated requests
-> materialise/deduplicate generated sidecars to a fixed point
-> borrow-validate generated functions
-> produce generated local lifetime constraints and summaries
-> borrow-validate the requesting base module using resolved generated summaries
-> produce base-module local lifetime constraints and summaries
-> finalize module interface, link facts and artefact
```

Then add:

> Complete lifetime topology is instantiated later by project/link planning over reachable functions and builder lifecycle roots. This plan preserves the accepted artefact lanes and summary boundaries without claiming that deferred lifetime-region analysis has been implemented.

## 5.3 Target `PublicSemanticInterface`

Replace `possible-consumption` wording with:

```markdown
- function mutation and optional-transfer eligibility/effect categories
- return-alias and projection-alias summaries
- retained-parameter and outlives summaries
- external-boundary classifications
- reactive summaries
```

## 5.4 Target `ModuleExecutable`

Add:

```markdown
- local lifetime-region and escape facts
```

Then add:

> The accepted artefact lane reserves these facts conceptually. The current canonical-module milestone must preserve the lane and stable handoff without claiming the deferred lifetime analysis has landed.

## 5.5 Generated sidecars

State that every generated sidecar carries:

- generated-local type context;
- HIR;
- borrow facts;
- local lifetime facts and summaries;
- link facts;
- fingerprints.

## 5.6 Fingerprints

Ensure public-interface fingerprint inputs include:

- optional-transfer summaries;
- alias/projection summaries;
- retained-parameter relationships;
- outlives constraints;
- external-boundary classifications.

## Acceptance criteria

- [ ] Active plan matches the architecture authority.
- [ ] It does not claim deferred lifetime implementation exists.
- [ ] Status block is current.
- [ ] Compiler agents cannot omit accepted lifetime lanes.

---

# Phase 6 — Synchronize remaining educational pages

## 6.1 Linking article

File:

```text
docs/src/docs/codebase/compiler-design/linking-entries-and-reachability/linking-entries-and-reachability.mtf
```

Replace:

```markdown
That work is parsed, type-checked, lowered and borrow-validated **before** any entry activates it.
```

with:

```markdown
That work is parsed, type-checked, lowered, borrow-validated and locally lifetime-analysed **before** any entry activates it.
```

## 6.2 Borrow-validation article

File:

```text
docs/src/docs/codebase/compiler-design/borrow-validation-and-drops/borrow-validation-and-drops.mtf
```

Replace:

```markdown
Diagnostics should distinguish proven-invalid topology from topology that conservative analysis cannot prove legal. Both are failures for the programmer. They are not the same failure family for the compiler's wording.
```

with:

```markdown
Borrow diagnostics distinguish concrete access conflicts from conservative access-analysis limitations. The later lifetime-region analysis separately distinguishes topology proven invalid from topology it cannot prove legal.
```

## 6.3 Verify, edit only if needed

Review:

```text
docs/src/docs/codebase/compiler-design/module-artefacts-and-reuse/module-artefacts-and-reuse.mtf
docs/src/docs/codebase/compiler-design/target-planning-and-validation/target-planning-and-validation.mtf
docs/src/docs/codebase/compiler-design/backend-lowering/backend-lowering.mtf
```

Only edit if they conflict with the final authority.

## Acceptance criteria

- [ ] Educational sequence includes local lifetime analysis.
- [ ] Borrow diagnostics do not claim topology-analysis ownership.
- [ ] Educational prose remains concise.

---

# Phase 7 — Fix metadata, links and descriptions

## 7.1 Follow-up plan state

File:

```text
docs/roadmap/plans/memory-management-documentation-follow-up-corrections-plan.md
```

After the cleanup commit is accepted, update to:

```text
ACTIVE_PLAN: none
STATUS: complete
CURRENT_SLICE: none
LAST_ACCEPTED_COMMIT: <final cleanup commit SHA>
WORKTREE: main
BLOCKERS_OR_OPEN_DECISIONS: none
NEXT_WORKER_ORDER: none
STOP_REASON: final consistency cleanup complete
NEXT_RESUME_ACTION: none; compiler behaviour changes require a separate approved implementation plan
```

Update validation state with actual final results.

If the SHA is not known before commit:

1. make corrections;
2. commit;
3. amend the plan status with the accepted SHA;
4. rebuild docs if the amendment changes generated output;
5. push the amended commit.

## 7.2 Runtime related-reading label

File:

```text
docs/src/docs/codebase/memory-management/runtime-and-backend-lowering/runtime-and-backend-lowering.mtf
```

Replace:

```markdown
group physical representation and group-to-group transfer
```

with:

```markdown
declared lifetimes, placement, exits and group-crossing restrictions
```

## 7.3 Declared-groups roadmap link

File:

```text
docs/src/docs/codebase/memory-management/declared-memory-groups/declared-memory-groups.mtf
```

Replace the raw URL with:

```markdown
- @https://github.com/nyejames/moth/blob/main/docs/roadmap/plans/grouped-memory-design.md (Grouped memory implementation roadmap): implementation sequencing and deferred investigations
```

## 7.4 Memory landing page

File:

```text
docs/src/docs/codebase/memory-management/@page.moth
```

Replace:

```markdown
Declared `group` / `into` syntax is accepted end-state design with implementation deferred. See the lifetime-regions page for the full topology model.
```

with wording equivalent to:

```markdown
Declared `group` / `into` syntax is accepted end-state design with implementation deferred. See @./declared-memory-groups (Declared memory groups) for the source contract and @./lifetime-regions-and-escape-validation (Lifetime regions and escape validation) for the general topology model.
```

## 7.5 WIT metadata wording

In the runtime WIT metadata section, replace:

```markdown
The importer requires stable:
- stable package identity
```

with:

```markdown
The importer requires:
- stable package identity
```

Keep the remaining bullets unchanged.

## Acceptance criteria

- [ ] Completed plan has no pending commit.
- [ ] No related-reading label implies V1 group transfer.
- [ ] Raw roadmap URL is replaced.
- [ ] Landing page points to both group and topology authorities.
- [ ] Metadata prose is clean.

---

# Phase 8 — Repository-wide contradiction audit

## 8.1 Map ownership

```sh
rg -n   "Maps own stored keys and values|Maps own their stored keys and values|returns the owned removed value|owned removed value"   README.md docs index.md
```

Expected: no live design or user-facing occurrences.

## 8.2 Freshness

```sh
rg -n   "\bfresh result\b|\bfresh results\b|fresh-result|fresh Moth|returns fresh storage|result is fresh"   README.md docs index.md
```

Review every hit.

## 8.3 Consumption terminology

```sh
rg -n   "possible consumption|possible-consumption|possible ownership consumption|possible parameter consumption"   docs/compiler-design-overview.md   docs/build-system-design.md   docs/roadmap/plans/canonical-module-compilation-and-scoped-packages-plan.md   docs/src/docs/codebase
```

Expected: no canonical or active-plan occurrence.

## 8.4 Generated lifetime facts

```sh
rg -n   "generated borrow facts|generated sidecar|generated function artefact|generated HIR"   docs/compiler-design-overview.md   docs/build-system-design.md   docs/roadmap/plans/canonical-module-compilation-and-scoped-packages-plan.md   docs/src/docs/codebase/compiler-design
```

Review every relevant summary.

## 8.5 Build-system duplicate and indentation

```sh
rg -n   "ProjectCompilation.*validated lifetime topology|produce local lifetime constraints"   docs/build-system-design.md
```

Expected: one topology sentence and flat pipeline indentation.

## 8.6 Group-transfer wording

```sh
rg -n   "group-to-group transfer|group physical representation and group-to-group transfer"   docs/src/docs
```

Expected: only explicit statements that V1 has no group-to-group transfer.

## 8.7 Plan metadata

```sh
rg -n   "LAST_ACCEPTED_COMMIT: pending parent commit|ACTIVE_PLAN:.*memory-management-documentation-follow-up"   docs/roadmap/plans
```

Expected: no pending commit and no completed memory plan still active.

## 8.8 Legacy monolith

```sh
rg -n   "docs/memory-management-design.md|Legacy consolidated reference"   README.md docs index.md
```

Expected: historical references only; no live authority; file remains absent.

## Acceptance criteria

- [ ] Every hit is reviewed.
- [ ] No unexplained contradiction remains.
- [ ] Historical plan text is clearly historical.

---

# Phase 9 — Build, inspect and finalize

## 9.1 Build

Run:

```sh
moth build docs --release
```

Fallback:

```sh
cargo run --quiet -- build docs --release
```

Do not hand-edit generated output.

## 9.2 Required route inspection

Inspect:

```text
/docs/codebase/memory-management/
/docs/codebase/memory-management/access-and-aliasing/
/docs/codebase/memory-management/borrow-validation/
/docs/codebase/memory-management/lifetime-regions-and-escape-validation/
/docs/codebase/memory-management/declared-memory-groups/
/docs/codebase/memory-management/ownership-and-drops/
/docs/codebase/memory-management/runtime-and-backend-lowering/
/docs/codebase/compiler-design/borrow-validation-and-drops/
/docs/codebase/compiler-design/linking-entries-and-reachability/
/docs/codebase/compiler-design/module-artefacts-and-reuse/
/docs/codebase/compiler-design/backend-lowering/
/docs/collections/
/docs/bindings/
/docs/progress/
```

Inspect the generated hash-map and explicit-copy sections specifically.

## 9.3 Generated diff

Confirm:

- no unrelated route churn;
- no literal broken table markup;
- no raw unlinked roadmap URL;
- no stale map ownership wording;
- no duplicate build-system prose;
- every generated change comes from source.

## 9.4 Final status update

Update the follow-up plan with the accepted commit SHA and validation result.

## Acceptance criteria

- [ ] Release build passes.
- [ ] Required routes render correctly.
- [ ] Generated output is source-derived.
- [ ] Final diff is documentation-only.
- [ ] Plan status is complete and accurate.

---

## 10. Final report requirements

### Files changed

List authority documents, active roadmap plan, educational pages, user-facing map/copy pages, completed plan metadata and generated routes.

### Semantic consistency corrections

Confirm:

- map entry-structure ownership wording;
- normal `remove` lifetime/ownership wording;
- fresh result root versus independent result graph;
- generated lifetime facts and summaries;
- closed external transfer override;
- no accepted child extraction;
- no V1 group-to-group transfer implication.

### Mechanical corrections

Confirm duplicate sentence removal, pipeline indentation, link fixes and plan metadata.

### Validation

State exact build command, result, search results and routes inspected.

### Out of scope

State that this cleanup did not implement lifetime analysis, groups, WIT imports, ownership-aware Wasm, compiler drift fixes or child extraction.

---

## Appendix A — Exact replacement text

### Map storage

```markdown
Maps own their entry structure. Existing keys and values stored in entries follow the ordinary shared-reference, explicit-copy and inferred-transfer rules.
```

### Map removal

```markdown
`remove` removes the entry and returns the removed value under the normal lifetime and ownership rules.
```

### WIT result

```markdown
Every result is lifted into an independent Moth result graph.
```

### Hidden result destination

```markdown
Functions whose result root is fresh may allocate that root directly into a caller-selected destination. The function summary classifies the result root as fresh and records retained-edge constraints separately.
```

### External transfer override

```markdown
Closed external boundary profiles override this general rule. WIT value-only calls and restricted host-value crossings are non-consuming. Mutable opaque-handle access does not transfer Moth storage through the ordinary Moth ownership ABI.
```

### Dormant root work

```markdown
Every normal module in the command's semantic graph has dormant root work fully compiled, borrow-validated and locally lifetime-analysed. Entry assembly activates already compiled work only.
```

### Generated compiler ownership

```markdown
The compiler owns generic template validation, call-site inference, request identity, generated HIR, generated borrow facts, and generated lifetime facts and summaries.
```

### Memory companion authority

```markdown
- `docs/src/docs/codebase/memory-management/overview.mtf` for reference semantics, borrow validation, lifetime topology, declared groups, ownership, GC and backend memory lowering
```

### Public interface summaries

```markdown
- mutation and optional transfer eligibility/effect categories
- return-alias and projection-alias summaries
- retained-parameter and outlives summaries
- external-boundary classifications
- relevant reactive effect summaries
```

### Copy page

```markdown
Do not apply `copy` directly to literals, constructor calls or computed expressions; `copy` accepts places only. Those expressions create new result roots, but existing values stored inside them still follow ordinary shared-reference rules.
```

---

## Appendix B — No-invention checklist

- [ ] No new memory semantic rule was introduced.
- [ ] No compiler code was changed.
- [ ] No lifetime implementation was claimed as landed.
- [ ] No map child was assigned implicit independent ownership.
- [ ] No `remove` operation became a source-visible ownership constructor.
- [ ] No bare `fresh` term implies graph independence where only the root is new.
- [ ] No WIT result is described as merely fresh rather than independent.
- [ ] No external call consumes ordinary values under a closed value profile.
- [ ] No final-use child extraction was accepted.
- [ ] No V1 group-to-group transfer was implied.
- [ ] No donor-local region identity crossed an interface.
- [ ] No active plan contradicted the architecture authorities.
- [ ] No generated documentation was edited manually.
- [ ] The deleted monolith was not restored.
