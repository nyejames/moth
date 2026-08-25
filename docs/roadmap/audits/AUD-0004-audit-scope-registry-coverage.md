# AUD-0004: Audit scope registry coverage and ownership

- State: `complete`
- Kind: `Documentation`
- Primary scope: the scope registry and freshness matrix in `docs/roadmap/audit-log.md`. This subject is itself unregistered; see `Limitations`.
- Required context: `docs/roadmap/audit-guide.md`, `docs/roadmap/audit-kinds/documentation.md`, `docs/roadmap/audit-kinds/README.md`, `docs/roadmap/audits/README.md`, `docs/roadmap/open-audit-findings.md`, AUD-0001, AUD-0002, AUD-0003, `docs/compiler-design-overview.md` (`Architectural invariants`, `Public semantic interfaces`, `Frontend stages > Stage 4: AST semantics > Constants, build configuration and const records`, `Stage 5: HIR and validation`, `Compiler implementation map`), `docs/roadmap/plans/constant-folding-and-type-system-hot-path-optimization-plan.md` (`Phase C review`, `Phase C coverage closure`), `docs/src/docs/progress/@page.moth`, `docs/src/developer-docs/style-guide/validation.mtf`, `.claude/skills/audit/SKILL.md`, `AGENTS.md`, `index.md`
- Coverage: `partial`
- Reviewed: `2026-08`
- Baseline: no gate was failing at the start of this run. This report changes documentation only, so the documentation release-build gate applies rather than `just validate`. Branch `const-folding-and-type-system-hot-path-optimization` is mid-plan at Phase C, so the frontend surface named in the proposed rows is actively changing.
- Revision: `54d5ab1a7`

## Why this audit exists

A Phase C `final_auditor` run for the constant-folding plan could not begin. The registry holds no
leaf, contract or composite scope covering the frontend folded-value pipeline, so selection step 6
of the audit guide — "Record the selected scope, required context, kind overrides and exclusions
before inspection" — could not be satisfied. That agent declined to invent ownership metadata,
because the guide requires registry defects to be corrected "through an accepted Documentation
finding". This report is that finding, filed under an explicit user-selected kind and scope, which
the guide says takes priority over automatic selection.

## Scope, context and exclusions

Primary subject: the `Scope registry` and `Audit freshness` tables in `docs/roadmap/audit-log.md`,
their governing rules in `docs/roadmap/audit-guide.md` (`Audit scopes`, `Scope kinds`,
`Primary scope and required context`, `Scope registry maintenance`, `Selecting an audit`,
`Freshness`), and the consistency of both against the repository they claim to partition.

Inspected as required context, not as audited coverage:

- The three existing reports, read for how each obtained the scope it needed.
- `docs/compiler-design-overview.md`, read for the stage-ownership authority needed to propose
  scope boundaries that follow architecture rather than directory layout.
- The constant-folding plan's `Phase C review` and `Phase C coverage closure` sections, read to
  identify exactly which code the blocked audit needed to cover.
- The frontend folded-value implementation, read at module-doc and ownership level only — enough to
  place leaf boundaries, not enough to audit the code. No correctness claim about that code appears
  in this report.

Explicit exclusions:

- The Phase C correctness audit itself. It is downstream of this work and was not run.
- Every audit-kind guide other than `documentation.md`. Their content is not the subject.
- A full partition of the compiler. Finding F02 bounds and defers that deliberately.
- The audit skill files. Observations about them are recorded under `Leads outside this scope`
  without expanding coverage or claiming a freshness cell.

## Coverage inventory

Inspected exhaustively:

| Surface | Result |
|---|---|
| `docs/roadmap/audit-log.md`, all 58 lines: 11 registry rows, 11 freshness rows, marker table | inspected |
| Every repository path cited by a registry row (39 Rust files, 4 documents, 1 manifest, plus 6 glob roots) | resolved against the worktree, one failure |
| Every canonical heading cited by a registry row (11 heading references across two design authorities) | all 11 resolve |
| Every scope ID cross-reference between the registry table and the freshness table | 11 of 11 consistent, no orphan |
| `docs/roadmap/audit-guide.md` rules governing the registry | inspected |
| `docs/roadmap/audit-kinds/documentation.md`, all 18 procedure steps | applied, applicability recorded below |
| `docs/roadmap/audits/README.md` skeleton and open-findings entry format | inspected |
| `docs/roadmap/open-audit-findings.md` | inspected |
| Git history of `docs/roadmap/audit-log.md` (6 commits, all rows attributed to an originating commit) | inspected |

Surveyed, not inspected exhaustively:

- `src/**` file and line inventory, by mechanical count, to establish the proportion of maintained
  implementation without a leaf owner. Counts are file-level; no implementation file was audited.
- The folded-value implementation modules, at module-doc and dependency level.

## Authorities read

- `AGENTS.md` — task routing, authority order, documentation-change rules, final-audit order.
- `docs/roadmap/audit-guide.md` — complete.
- `docs/roadmap/audit-kinds/documentation.md` — complete; the applied procedure.
- `docs/roadmap/audit-kinds/README.md`, `docs/roadmap/audits/README.md`,
  `docs/roadmap/open-audit-findings.md`, `docs/roadmap/audit-log.md`.
- `docs/roadmap/audits/AUD-0001-test-support-redundancy.md`,
  `AUD-0002-stage0-discovery-preparation-performance.md`,
  `AUD-0003-runtime-assertion-messages-call-arguments.md`.
- `docs/compiler-design-overview.md` — `Architectural invariants`, `Compiler input and result
  boundary`, `Public semantic interfaces`, `Frontend stages > Stage 4: AST semantics > Constants,
  build configuration and const records`, `Stage 5: HIR and validation`, `Compiler implementation
  map`.
- `docs/roadmap/plans/constant-folding-and-type-system-hot-path-optimization-plan.md` — `Phase C
  review`, `Phase C coverage closure`.
- `docs/src/docs/progress/@page.moth` — searched for audit-framework status rows; there are none.
- `docs/src/developer-docs/style-guide/validation.mtf` — `Documentation-only release-build gate`,
  `Fast iteration commands`, `Generated documentation`, `Documentation-only completion checklist`.
- `.claude/skills/audit/SKILL.md` and `.agents/skills/audit/SKILL.md`.
- `index.md` — searched for audit-framework references.

## Existing findings and active plans checked

`open-audit-findings.md` holds no candidate, accepted, active, blocked or awaiting-verification
finding. Its `Resolved in this branch` section records AUD-0002-F06, the closest prior root cause.

**Relationship to AUD-0002-F06.** F06 recorded the same invariant breach for Stage 0 and was
accepted and implemented. It is closed, and its triage record states its own bound explicitly: "the
compiler frontend as a whole is still unregistered — this finding resolved the Stage 0 gap only,
not the general one." F01 of this report is therefore not a duplicate and not a re-litigation. It
is the linked child the guide describes: "Create a linked child when new evidence proves a
materially broader problem." The new evidence is that the gap has now blocked a second, different
audit under a different kind, and that four subsequent registration events have all been
per-audit rather than systematic.

No active plan under `docs/roadmap/` claims ownership of the audit framework or the registry. The
constant-folding plan is the consumer that this defect blocks, not its owner.

## Applicability of the Documentation audit-kind procedure

The kind guide has 18 procedure steps. Recorded per the run instruction rather than skipped
silently.

| Step | Applied | Note |
|---|---|---|
| 1. Map the authority hierarchy | yes | See below. |
| 2. Check authority declarations and routing | yes | Load-bearing. Findings F01, F02; lead L01. |
| 3. Compare canonical documents for consistency | partial | The audit framework's canonical owners are `audit-guide.md` and `audit-log.md`. Compared for consistency with each other and with `audits/README.md` and `audit-kinds/documentation.md`. The compiler and build design authorities were compared only where the registry cites them. |
| 4. Check accepted end state versus current status | **not applicable** | The audit framework has no progress-matrix row and no supported/partial/deferred surface. `docs/src/docs/progress/@page.moth` was searched and contains no audit-framework entry. Freshness is not implementation status. |
| 5. Check roadmap and plan ownership | yes | Load-bearing. `roadmap.md` contains no audit-framework entry; the framework is deliberately not a roadmap item. The constant-folding plan does not redefine audit policy. Finding F02 records that no owner sequences registry build-out. |
| 6. Check implementation maps and repository paths | yes | Load-bearing. Every registry path and heading verified. Finding F03. |
| 7. Check public teaching pages | **not applicable** | The registry has no Basic/Advanced or teaching page, and none exists under `docs/src/docs/` for the audit framework. |
| 8. Check the language cheatsheet and compact references | **not applicable** | The registry is not a language surface. |
| 9. Check README and project-level summaries | **not applicable** | Neither `README.md` nor `CONTRIBUTING.md` describes the audit framework; `index.md` does not reference it. Nothing to be false about. |
| 10. Check examples and code blocks | partial | The registry contains no Moth code examples. The one fenced block in `audit-guide.md` (`Correctness: frontend.ast, analysis.borrow`) is an override-syntax example; it is checked in F01's counter-evidence because both scope IDs it names are unregistered. |
| 11. Check terminology and naming drift | yes | Scope-kind vocabulary (`Leaf`, `Composite`, `Contract`, `Comparison`) is used consistently in registry and guide. Marker vocabulary (`N`/`P`/`C`/`S`/`-`) is consistent. One capitalisation drift between the guide's kind names and the skill's is cosmetic and not filed. Dotted-ID convention is followed by all 11 rows. |
| 12. Check restrictions, exclusions and open questions | yes | The registry's `Exclusions` column is used by 5 of 11 rows and each exclusion is truthful. AUD-0002-F06's deliberate non-registration of `contract.module_compilation_handoff` is recorded in its triage record but nowhere in the registry, which is part of F02. |
| 13. Check progress coverage claims | **not applicable** | Freshness markers are inspection-coverage records, not test-coverage claims. The guide states this explicitly. No `tests/cases/manifest.toml` comparison is meaningful here. |
| 14. Check documentation ownership and duplication | yes | Load-bearing. Registry-versus-guide ownership is clean: the log states it "does not store findings or process rules" and routes kind and completeness rules to the guide. Duplication was found in the skill files, recorded as lead L01. |
| 15. Check links, navigation and generated output | yes | All four intra-framework relative links in `audit-log.md` resolve. All registry-cited repository paths verified (F03 is the one failure). No generated output is involved: `docs/roadmap/**` is not part of the `docs` release build. |
| 16. Review prose quality without changing meaning | yes | The registry prose is direct, rule-first and British English. No prose finding. |
| 17. Classify the finding | yes | F01 navigation/ownership correction; F02 navigation/ownership correction with a bounded design element, filed as `blocked` on that element; F03 navigation/ownership correction. |
| 18. Form the finding | yes | Each finding names affected documents, the canonical source of truth, the exact missing content, the proposed documentation-only correction and whether it is design-gated. No example or generated output needs rebuilding. |

### Step 1: the mapped authority hierarchy

| Concern | Owner |
|---|---|
| Audit policy: selection, scoping, evidence, lifecycle, freshness semantics | `docs/roadmap/audit-guide.md` |
| The scope graph and rough freshness records | `docs/roadmap/audit-log.md` |
| Per-kind procedure and invalidators | `docs/roadmap/audit-kinds/<kind>.md` |
| Report format, naming, open-findings entry shape | `docs/roadmap/audits/README.md` |
| Live unresolved work | `docs/roadmap/open-audit-findings.md` |
| Stage ownership the registry must follow | `docs/compiler-design-overview.md`, `docs/build-system-design.md` |
| Task routing into all of the above | `AGENTS.md` |
| Selection and conduct of one run | `.claude/skills/audit/SKILL.md` (routes; does not own) |

Each concern has exactly one owner and each document states what it does not own. The hierarchy is
sound. The defect is not in who owns the registry; it is in what the registry contains.

## Findings

### AUD-0004-F01: The scope registry covers 4.6% of maintained implementation, so a directed audit of the frontend folded-value pipeline cannot record coverage

- State: `candidate`
- Kind: `Documentation`
- Scope: `docs/roadmap/audit-log.md`
- Priority: `unassigned`

#### Evidence

`audit-guide.md` > `Scope kinds` states the invariant directly:

> **Leaf**: the smallest useful ownership unit for exhaustive coverage. Every maintained
> implementation file belongs to exactly one leaf.

`audit-guide.md` > `Scope registry maintenance` states the review trigger:

> Review the registry when: a maintained implementation file has no leaf owner

Measured against the worktree at `54d5ab1a7`:

| Measure | Value |
|---|---|
| Registered scopes | 11 |
| Of which Leaf | 7 (`tests.harness`, `tests.cases`, four `build.stage0.*`) |
| Of which Composite / Contract / Comparison | 4 |
| Production `.rs` files under `src/` (excluding `src/**/tests/**`, `test_support.rs`, `src/compiler_tests/`) | 791 |
| Production lines under `src/` on the same basis | 245,118 |
| Production files owned by a leaf | 20 — every file in `src/build_system/create_project_modules/`, and nothing else |
| Production lines owned by a leaf | 11,228 |
| **Production implementation with no leaf owner** | **771 files (97.5%), 233,890 lines (95.4%)** |

Only `tests.harness` and `tests.cases` cover the rest of the registry's surface, and both are test
tooling and fixture data. `tests.support` is a Comparison scope, which under the guide's own kind
definitions confers no leaf ownership.

Whole maintained systems have no leaf owner at all:

| Unowned area | Production files | Production lines |
|---|---|---|
| `src/compiler_frontend/` | 576 | 192,002 |
| `src/backends/` | 75 | 13,495 |
| `src/projects/` | 71 | 15,622 |
| `src/build_system/` outside `create_project_modules/` | 13 | 4,681 |
| `src/builder_surface/` | 19 | 2,290 |
| `src/timing/` + `src/timing.rs` | 12 | 5,038 |
| `src/benchmarking/` | 3 | 673 |
| `xtask/` (a workspace member with maintained implementation, which the guide's `Scope kinds` explicitly says may own leaves) | 56 | 40,643 |

The three directories named by the blocked Phase C audit are confirmed unowned:

- `src/compiler_frontend/ast/const_values/` — 4 production files, 1,388 lines. No registry row
  names any file in it, under any scope kind.
- `src/compiler_frontend/ast/module_ast/finalization/` — 13 production files, 6,768 lines. Four of
  them (`finalizer.rs`, `normalize_ast.rs`, `validate_types.rs`, `const_fact_collection.rs`) appear
  as the producer endpoint of `contract.assertion_message_runtime_handoff`. A Contract scope audits
  "a producer-consumer boundary and the artefact passed between them ... without becoming a full
  audit of both systems", so it is not a leaf owner. The other nine files, including
  `public_const_templates.rs` — which is where Phase C's user-diagnostic-to-ICE regression was found
  — are named nowhere.
- `src/compiler_frontend/hir/` — 70 production files, 22,029 lines. Three surfaces
  (`hir_statement.rs`, `reachability.rs`, `validation/**`) appear as the consumer endpoint of the
  same Contract scope. `hir/constants.rs`, `hir/const_facts.rs`, `hir/hir_builder.rs` and
  `hir/hir_statement/declarations.rs` — every module the Phase C review names — are named nowhere.

**The consequence, which is the point of this finding.**

1. *Automatic selection cannot reach the code at all.* `audit-guide.md` > `Selecting an audit`
   operates entirely over freshness cells: skip `-`, prefer `N`, then `S`, then oldest `P`, then
   oldest `C`. Unregistered code has no cell, so it is not merely low priority — it is
   unreachable. 95.4% of maintained implementation is invisible to the selection algorithm.

2. *A directed audit of that code cannot record coverage either.* This is the harder failure and
   the one that actually blocked Phase C. A user- or plan-directed run bypasses selection, but it
   still hits step 6 of `Selecting an audit` ("Record the selected scope ... before inspection"),
   and it still hits `Freshness` ("Only a complete report can promote an entry to `C`"). With no
   cell to promote, a directed audit can produce findings and cannot record that it happened. That
   is exactly what AUD-0002 reported: "This audit therefore produced findings but **could not
   record freshness**, so the work it did is invisible to future automatic selection and may be
   repeated."

3. *The registry has only ever grown ad hoc, one audit at a time.* `git log` on
   `docs/roadmap/audit-log.md` returns six commits, and every scope row is attributable to the
   audit that needed it:

   | Commit | Subject | Rows added |
   |---|---|---|
   | `c534b1dd9` | Add codebase audit log skeleton | table headers only, zero scopes |
   | `b9cf2d3e4` | Harden and integrate codebase audit framework | column renames, zero scopes |
   | `357dbab31` | test: harden suite honesty and validation infrastructure | the 3 `tests.*` rows, with AUD-0001's `P` cell |
   | `990f309d8` | stage 0 module discovery audit findings | the 5 `build.stage0*` rows, with AUD-0002's two `P` cells |
   | `82d22abc2` | assert() with normal function call parsing | the 2 `contract.assertion_*` rows and `feature.runtime_assertion_messages_call_arguments`, with AUD-0003's `C` cell |

   No commit has ever added a scope that an in-flight audit did not immediately need. Every scope
   in the registry was registered to unblock one specific report, and in two of the three cases the
   freshness cell was written in the same commit as the row.

4. *The pattern is now on its second recurrence, and this time it stopped an audit rather than
   degrading one.* AUD-0002 ran, found six real findings, and only afterwards discovered it could
   not record them. The Phase C `final_auditor` did not get that far: the audit skill it ran under
   instructs, at `Resolve the request` step 3, "If the request necessarily spans several registered
   scopes and no registered scope covers it, stop and name the separate scopes or missing
   composite/contract scope. Do not invent an untracked scope or claim broad coverage." The
   framework behaved exactly as designed and the run produced nothing.

#### Counter-evidence checked

Four counter-explanations were tested, three of which came from the run instruction. Each is
recorded with what was checked and whether it was accepted.

**C1. Per-audit ad hoc registration is the intended lightweight design, not a defect.**
*Partly accepted, and it changes what the finding claims.* The evidence in point 3 above is exactly
as consistent with a deliberate lazy-registration policy as with neglect, and `audit-guide.md` >
`Audit scopes` supports the lightweight reading: "The audit log records a graph of valid scopes
rather than one flat directory partition", and `Scope registry maintenance` frames registry work as
*review triggers* rather than a one-off partitioning exercise. AUD-0002-F06 anticipated this and
declined to assert the rollout was wrong. I accept the design intent. I do not accept that it
discharges the defect, for one reason that is checkable rather than a matter of taste: lazy
registration only works if an audit that discovers a missing scope can register it and proceed. It
cannot — see F02. The design produces a deadlock at exactly the moment it is needed, and the
deadlock has now fired twice. So this finding does **not** claim that incremental registration is
wrong. It claims that 95.4% unowned, combined with the deadlock, has a concrete cost that has now
been paid twice.

**C2. "Every maintained implementation file belongs to exactly one leaf" describes a target state
for registered scopes, not a coverage obligation over all of `src/`.**
*Rejected, on the guide's own wording.* Under the narrow reading the sentence would mean "no two
registered leaves may overlap", which is a *disjointness* rule. Three checks contradict that
reading:

1. The sentence says "every maintained implementation file", not "every file in a registered
   scope". A disjointness rule would be phrased over scopes; this is phrased over files.
2. `Scope registry maintenance` lists as its first review trigger "a maintained implementation file
   has no leaf owner". Under the narrow reading that trigger could never fire — an unregistered
   file is simply outside the system. The trigger only has meaning if unowned maintained files are
   a registry defect.
3. The next sentence carves out an explicit exception: "Test cases, fixtures, benchmarks, canonical
   documents and generated outputs may be attached surfaces without becoming duplicate production
   owners." A rule that only governed registered scopes would not need to exempt whole categories
   of repository content from ownership.

The narrow reading also cannot be reconciled with the existing registry, which uses the broad
reading in practice: AUD-0002-F06's triage record justifies the four-way Stage 0 split by stating
that "together the four leaves own every maintained file under
`src/build_system/create_project_modules/` exactly once, satisfying the guide's rule that each
implementation file belongs to one leaf". The maintainer applied the broad reading when
implementing the last accepted registry finding.

**C3. A plan-directed audit could legitimately proceed under a filter without a registered scope.**
*Rejected, and this is the counter-explanation that most needed testing, because it would make the
Phase C block an auditor error rather than a registry defect.* `audit-guide.md` > `Scope kinds`
closes it directly:

> A pull request, branch, commit range, roadmap item or changed-file list is a selection filter,
> not a scope kind. **Map the affected files and contracts to registered scopes.** Use an existing
> composite or separate reports when several scopes are involved. A changed-area review cannot mark
> a scope `C` unless it inspects the complete registered scope and satisfies the selected kind
> guide.

A filter is explicitly required to resolve onto registered scopes; it is not an alternative to
them. The skill restates the same rule at `Resolve the request`. And the freshness system gives the
same answer from the other end: a filter-only run has no cell, so even a genuinely exhaustive pass
over the changed files could record nothing. The blocked `final_auditor` was correct to stop.

**C4. Is `frontend.ast` already registered under a name I missed?**
*Rejected by inspection.* Every one of the 11 scope IDs was read and every path in the
`Primary coverage` and `Default context` columns was resolved against the worktree. No row names
any file under `src/compiler_frontend/ast/const_values/`. Separately, `audit-guide.md` >
`Primary scope and required context` contains a worked example of a kind-specific override —
`Correctness: frontend.ast, analysis.borrow` — and **neither `frontend.ast` nor `analysis.borrow`
is a registered scope ID.** The guide's illustrative example uses IDs that do not exist. That is
weak evidence for how the registry was expected to look, and direct evidence that the guide was
written expecting a frontend partition that was never created.

#### Violated contract or cost

`docs/roadmap/audit-guide.md` > `Scope kinds`: "Every maintained implementation file belongs to
exactly one leaf." 771 of 791 production implementation files belong to none.

`docs/roadmap/audit-guide.md` > `Scope registry maintenance`: registry review is required "when a
maintained implementation file has no leaf owner". The trigger is met at a scale of 233,890 lines
and has been met continuously since the framework was introduced.

Cost: structured audit coverage cannot be selected for, recorded against, or accumulated over
95.4% of the maintained codebase. Two audits have now paid it — AUD-0002 lost its freshness record,
and the Phase C `final_auditor` produced no report at all.

#### Impact

The audit framework functions as designed only on the 4.6% of the codebase it has been pointed at.
For everything else it is write-only: work can be done, but not recorded, not accumulated and not
routed to. Repeat audits of the same surface cannot be detected, and stale coverage cannot be
distinguished from absent coverage.

The immediate blocked consumer is the constant-folding plan's Phase C final audit, which the plan
requires and `AGENTS.md` requires of every non-trivial implementation plan ("Every non-trivial
implementation plan must end with the Final audit").

#### Root owner

`docs/roadmap/audit-log.md`. The invariant is stated by `docs/roadmap/audit-guide.md`, but the
guide is correct as written; the registry is what fails to satisfy it.

#### Suggested correction

**Non-authorising.** Registering these rows is implementation of an accepted finding, not part of
this run.

Register three scopes covering the module-constant folded-value owner and its producer-to-consumer
boundary — the surface the blocked Phase C audit needs, and no more. The proposal deliberately
mirrors the structure the maintainer already accepted for AUD-0003: one leaf that owns the code, a
contract for the handoff, and a feature composite that a plan's final audit can select as its
primary scope.

##### Proposed registry rows

| Scope ID | Name | Kind | Primary coverage | Default context | Kind-specific context or exclusions |
|---|---|---|---|---|---|
| `frontend.const_values` | Module-local folded constant values | Leaf | `src/compiler_frontend/ast/const_values/{mod,facts,resolver,store}.rs` | `docs/compiler-design-overview.md` (`Frontend stages > Stage 4: AST semantics > Constants, build configuration and const records`, `Public semantic interfaces`), `src/compiler_frontend/ast/const_eval/mod.rs`, `src/compiler_frontend/ast/module_ast/environment/constant_resolution.rs` | Owns const-ness classification, the module-local folded value graph and const-fact payloads. The shared constant folder in `ast/const_eval/` and the TIR template reducer are separate owners read as context. `Performance: contract.folded_constant_handoff, src/compiler_frontend/ast/module_ast/environment/constant_resolution.rs, which owns the constant-header resolution cost the const-folding plan measures separately` |
| `contract.folded_constant_handoff` | Folded constant AST-to-consumer handoff | Contract | `src/compiler_frontend/ast/const_values/store.rs` and `src/compiler_frontend/ast/module_ast/finalization/{const_fact_collection.rs,public_const_templates.rs}` -> `src/compiler_frontend/folded_value.rs` and `src/compiler_frontend/public_interface/direct_projection.rs`, and -> `src/compiler_frontend/hir/{constants.rs,const_facts.rs,hir_builder.rs,hir_statement/declarations.rs,validation/metadata.rs}` | `docs/compiler-design-overview.md` (`Architectural invariants`, `Public semantic interfaces`, `Stage 4: AST semantics > Constants, build configuration and const records`, `Stage 5: HIR and validation`), `frontend.const_values` | Audits the boundary only: whether donor-local `ConstValueId`, `InternedPath` and `TypeId` identity is dropped before publication, whether the two consumers reconstruct facts the producer already owns, where the const-evaluable/publishable classification split is validated, and which lane a rejection uses. It is not a full audit of the public interface or of HIR. Integration fixtures and generated release documentation are attached evidence, not alternate semantic owners. |
| `feature.module_constant_folded_values` | Module-constant folded values end to end | Composite | `frontend.const_values`, `contract.folded_constant_handoff` | Both child entries, `docs/compiler-design-overview.md` opening authority text and `Architectural invariants`, `docs/roadmap/plans/constant-folding-and-type-system-hot-path-optimization-plan.md` | End-to-end review for the active constant-folding plan, including its Phase C final audit. Integration fixtures, the progress matrix rows for constants, and the plan's own measurement records are attached review surfaces. A finding owned by one child is recorded against that child, not here. |

##### Proposed freshness rows

| Scope ID | Style | Comments | Correctness | Diagnostics | Tests | Redundancy | Performance | Documentation |
|---|---|---|---|---|---|---|---|---|
| `frontend.const_values` | `N` | `N` | `N` | `N` | `N` | `N` | `N` | `-` |
| `contract.folded_constant_handoff` | `-` | `-` | `N` | `-` | `-` | `-` | `-` | `-` |
| `feature.module_constant_folded_values` | `-` | `-` | `N` | `N` | `-` | `N` | `N` | `-` |

##### Justification for every `-`

The guide warns that "an invalid pair left as `N`" corrupts automatic selection. The inverse error
is equally real: a `-` on an applicable pair hides available work permanently. Each `-` below is
justified individually, and each `N` is asserted as genuinely available work.

`frontend.const_values` — **Documentation `-`.** The leaf owns Rust implementation and no
documentation surface. The constants contract is owned by `docs/compiler-design-overview.md`, which
would be audited under a documentation scope for that authority, not through this leaf. Every code
leaf currently in the registry carries `-` here for the same reason; this row is consistent with
all seven of them. All seven other kinds are `N` and genuinely available: the leaf has maintained
implementation (Style, Comments), user-visible semantics reached through it (Correctness), a
diagnostic lane (`ConstValueStoreError::Diagnostic` wrapping `CompilerDiagnostic`, plus the
infrastructure split at `store.rs:312`) (Diagnostics), an owned test module at
`const_values/tests/` (Tests), a documented history of parallel paths (Redundancy), and a live
performance plan targeting it (Performance).

`contract.folded_constant_handoff` — **Style, Comments, Tests, Redundancy, Performance,
Documentation all `-`.** A Contract scope audits "ownership, information loss, reconstruction,
validation placement and handoff invariants without becoming a full audit of both systems". Style,
Comments and Tests are per-file lanes: they belong to the leaf owning each endpoint file, and
running them here would double-own files that `frontend.const_values` already owns and that a
future HIR and public-interface leaf will own. Redundancy and Performance likewise attach to the
owning leaf, because a fix for either would edit files inside a leaf rather than the boundary
itself. **Documentation `-`** because the contract owns no prose. **Diagnostics `-`** because the
diagnostics on this path are constructed in `public_const_templates.rs` and routed through
`store.rs`, both of which sit inside leaves; the boundary carries a lane classification, and
whether that classification is right is a Correctness question about validation placement, which
this scope does own. This is the same shape as both registered contract rows, which carry `N` for
Correctness and `-` for everything else.

`feature.module_constant_folded_values` — **Style, Comments, Tests, Documentation `-`; Correctness,
Diagnostics, Redundancy, Performance `N`.** This follows the `build.stage0` composite precedent
exactly, which is the only other non-feature composite in the registry. Style, Comments and Tests
are per-file lanes with no meaningful composite-level question — a composite audit of them would
just be the union of its children and would double-record coverage. Documentation `-` for the same
reason as its children. The four `N` cells are asserted as real available work and each is
evidenced by the plan's own `Phase C review`: a diagnostic that became a `CompilerError` across the
boundary (Diagnostics), three retained test-only parallel paths spanning producer and consumer
(Redundancy), measured cost that moves between producer and consumer
(`lower_module_constants` regressing `frontend.hir` by `+0.5ms` while `finalise.module_constant`
improved `-5.0%`) (Performance), and the two real defects found by the coverage closure work
(Correctness). Each of those four is a question that only exists at composite level, which is what
makes the composite worth registering rather than being a bookkeeping wrapper.

##### Justification of the leaf boundary against the guide's test

The guide's test: "A leaf is too broad when one audit cannot inspect it exhaustively or it contains
several independent owners. It is too narrow when every meaningful audit must always read several
siblings as one owner."

*Not too broad.* `frontend.const_values` is 4 files and 1,388 lines — smaller than every registered
Stage 0 leaf, and roughly a quarter of `build.stage0.discovery`, which AUD-0002 covered in a single
run. One audit can read it exhaustively. It has one owner: `mod.rs` declares it, `facts.rs` defines
the fact payloads, `resolver.rs` decides const-ness, `store.rs` holds the resulting value graph.
`store.rs`'s own doc comment states the single-ownership claim — "the only folded-value
representation retained across the AST finalization boundary" — and `mod.rs` states the shared-owner
rationale: "config validation, AST finalization, and HIR metadata all need one shared source of
truth for const-ness instead of duplicating evaluation logic".

*Not too narrow.* The three non-trivial files are mutually inseparable. `resolver.rs` imports
`ConstValueId` and `ConstValueStore` from `store.rs` and `AstConstFactValue` from `facts.rs`;
`facts.rs` imports `ConstValueId` from `store.rs`. No audit of any one of them can proceed without
the other two, which is precisely the guide's too-narrow condition — so they must be one leaf, and
they are.

*Why the boundary stops where it does.* Three adjacent surfaces were considered and deliberately
excluded from the leaf:

- `ast/const_eval/mod.rs` (1,022 lines) is the shared constant folder that `resolver.rs` calls. It
  is a general expression-folding owner used well beyond module constants, so folding it in would
  introduce a second independent owner. It is context.
- `ast/module_ast/environment/constant_resolution.rs` (233 lines) owns the module-scoped
  constant-header session. Its own doc comment scopes it to header dependency order and explicitly
  disclaims body-local constants. It belongs to a future environment-builder leaf, and the
  constant-folding plan measures its cost (`ast.environment.constant_header_resolution`) as a
  separate line item. It is context, and is named in the Performance override because that plan's
  `Phase C review` finding 7 records it as "an unowned cost this plan should either adopt or
  explicitly hand off".
- `ast/module_ast/finalization/` (6,768 lines across 13 files) is where const facts are collected
  and public const templates are projected. It contains several independent owners — AST
  normalisation, type validation, reactive template annotation, debug type validation — and at
  nearly 7,000 lines it is too broad for one leaf as a unit. Registering it correctly requires
  partitioning it, which is F02's work, not this finding's. The two files this pipeline actually
  crosses (`const_fact_collection.rs`, `public_const_templates.rs`) are therefore reached through
  the Contract scope as producer endpoints, not claimed as leaf coverage.

##### Known inconsistency in the proposal, stated rather than hidden

`contract.folded_constant_handoff` names endpoint files that no leaf owns — in
`finalization/`, `public_interface/` and `hir/`. AUD-0002-F06's triage record refused to register
`contract.module_compilation_handoff` for exactly this reason: "Its consumer half ... still has no
leaf owner, so a contract scope there would reference an unregistered counterpart."

The registry contains the opposite precedent, and it is the one that works.
`contract.assertion_message_runtime_handoff` is registered today and names endpoints in
`finalization/`, `hir/` and `src/backends/`, none of which has a leaf owner — and AUD-0003 ran
against it to `C` coverage. A contract scope with unregistered endpoints is therefore known to be
usable in practice, and is what unblocked the last plan-final audit.

This proposal follows the registered precedent rather than the refused one, because the alternative
is to block Phase C behind the frontend partition in F02. Triage owns this decision. If the
maintainer prefers AUD-0002-F06's stricter rule, the consequence is explicit: `frontend.const_values`
alone can be registered now, and Phase C's final audit must then wait on F02 or run against the
leaf only, with reduced coverage.

#### Allowed fix scope

`docs/roadmap/audit-log.md` only — three rows in the `Scope registry` table and three rows in the
`Audit freshness` table.

#### Read-only context

`docs/roadmap/audit-guide.md`, `docs/roadmap/audit-kinds/**`, `docs/roadmap/audits/**`,
`docs/compiler-design-overview.md`, `docs/roadmap/plans/constant-folding-and-type-system-hot-path-optimization-plan.md`,
`AGENTS.md`, `index.md`, and the implementation paths named in the proposed rows.

#### Must preserve

- Stable dotted conceptual IDs that do not encode a path that may move.
- Leaf scopes owning maintained implementation exactly once. Registering these rows must not create
  a file owned by two leaves.
- `-` on genuinely inapplicable pairs so automatic selection skips them, and `N` on every
  applicable pair so available work stays visible.
- `N` on every newly registered cell. No cell may be back-dated to `P` or `C`; no audit has run
  against these scopes.
- The registry's existing 11 rows, unchanged by this finding.

#### Forbidden fix forms

- Registering one broad `frontend` or `compiler` leaf that no single audit could cover exhaustively.
- Back-dating any freshness cell, including recording this report against a newly created cell.
- Marking a kind `-` merely because no audit of it is planned. `-` means inapplicable, not
  unscheduled.
- Registering the proposed contract scope while deleting or narrowing
  `contract.assertion_message_runtime_handoff` to avoid overlapping endpoint citations. Contract
  scopes may share endpoint files; only leaves may not.
- Editing `audit-guide.md` to weaken the "exactly one leaf" sentence so the registry satisfies it.
  That is the `rewrites canonical documentation to match incorrect implementation` form, applied to
  documentation.

#### Required validation or measurement

Documentation release-build gate per `docs/src/developer-docs/style-guide/validation.mtf` >
`Documentation-only release-build gate`: `moth build docs --release`, or
`cargo run --quiet -- build docs --release`. Not `just validate`.

After registration, confirm by inspection that: every proposed path resolves in the worktree; no
file appears in two Leaf rows; the freshness table has exactly one row per registry row.

#### Dependencies and related findings

- Linked child of AUD-0002-F06 (closed). Same root cause, materially broader evidence.
- Blocks the constant-folding plan's Phase C final audit.
- Depends on AUD-0004-F02 only if triage adopts the stricter contract rule described above.
- F03 is independent and may be fixed in the same change.

#### Triage record

Not yet triaged.

### AUD-0004-F02: The audit workflow has no route by which a directed audit of unregistered code can register its scope, so such a run deadlocks

- State: `candidate`
- Kind: `Documentation`
- Scope: `docs/roadmap/audit-guide.md`, `docs/roadmap/audit-log.md`
- Priority: `unassigned`

#### Evidence

This is the finding that makes F01 a defect rather than a backlog item, and it is filed separately
because its owner is the guide rather than the registry.

Four rules interlock into a cycle:

1. `audit-guide.md` opening: "An audit run is read-only with respect to production code, tests,
   benchmarks, fixtures, canonical design documents and implementation status. It may create or
   update its report, the audit log and the open-findings index."
2. `audit-guide.md` > `Primary scope and required context`: "If a registered scope is incomplete,
   overlaps another leaf or assigns the wrong owner, do not claim complete coverage. Record the
   scope defect, **update the registry through an accepted Documentation finding** and mark the
   audit partial."
3. `audit-guide.md` > `Triage`: "Triage is a separate decision after the report is complete."
4. `audit-guide.md` > `Selecting an audit` step 6: "Record the selected scope, required context,
   kind overrides and exclusions before inspection."

An auditor directed at unregistered code must satisfy (4) before inspecting. It cannot, because no
scope exists. Rule (2) sends it to file a Documentation finding, which under (3) cannot be accepted
within the same run, and under (1) the auditor may not simply write the row itself. So the directed
audit cannot start, and the only way through is a separate report, separate triage and separate
implementation — three runs before the originally requested audit can begin.

Rule (1) is genuinely ambiguous on this point and the ambiguity is load-bearing. It grants audits
permission to "update ... the audit log", which read alone would authorise adding a scope row.
Rule (2) then routes registry changes through accepted findings. Two audits have now read this
pair and both resolved it the conservative way — AUD-0002-F06's counter-evidence section records
"Should this audit have registered the scope itself? No. Registry changes go through an accepted
Documentation finding", and the blocked Phase C `final_auditor` reached the same conclusion
independently. Consistent conservative reading by two independent agents is evidence that the
conservative reading is the natural one, and that the permission in (1) is understood to cover
freshness cells only, not scope rows. The guide does not say so.

The deadlock is reinforced downstream. `.claude/skills/audit/SKILL.md` > `Resolve the request`
step 3 instructs a run to "stop and name the separate scopes or missing composite/contract scope.
Do not invent an untracked scope or claim broad coverage", and `Select the kind and scope`
instructs "If the registry or freshness matrix has no valid candidate, stop and report that the
audit framework needs a registered applicable scope." The skill correctly routes to the guide's
rule; the rule is what has no exit.

Observed cost, twice:

- AUD-0002 ran to completion and could not record freshness. Its own report states the work "is
  invisible to future automatic selection and may be repeated."
- The Phase C `final_auditor` stopped before inspection and produced no report at all. Three runs
  (this report, its triage, its implementation) are now required before the audit that was
  originally requested can begin.

Separately, no document owns registry build-out. `docs/roadmap/roadmap.md` contains no
audit-framework entry — the framework is deliberately outside roadmap sequencing, which is
defensible for policy documents but leaves the 233,890 unowned lines from F01 with no owner and no
sequence. AUD-0002-F06's triage record notes that `contract.module_compilation_handoff` "remains
proposed, not registered", but that decision lives only in a closed finding inside a report.
Nothing in `audit-log.md` records that a proposed scope was considered and declined, so the next
auditor to reach that boundary will rediscover it.

#### Counter-evidence checked

**Is the three-run cost actually correct and intended?** *Partly.* The guide's separation of audit
from triage from implementation is deliberate and sound — it is what stops an auditor from
authorising its own write scope, and it should not be removed. The defect is not that registration
requires acceptance. It is that the guide provides no bounded exception for the one case where the
missing artefact is the precondition of the run itself, and no guidance on what a directed auditor
should do instead. Compare `Selecting an audit`, which handles every other selection situation
explicitly. This gap is the only one with no stated outcome.

**Could the auditor register a scope under rule (1)'s "update ... the audit log" permission?**
*Checked and rejected as the current reading.* See the two-agent evidence above. But the
ambiguity is real, and resolving it in either direction is a valid correction — the finding does
not require the exception to be granted, only that the guide say which reading is right.

**Is this the same finding as F01?** *No.* F01's owner is `audit-log.md` and its fix is data. F02's
owner is `audit-guide.md` and its fix is a process rule. Fixing F01 alone leaves the deadlock armed
for the next unregistered subsystem, which is 95.4% of the codebase. Fixing F02 alone leaves Phase C
still blocked. They are filed separately so triage can accept them independently.

**Is the roadmap-ownership half of this really a finding, or just an absence?** *Filed as part of
this finding rather than separately, because it has the same consequence.* An absence of ownership
would be inert if the registry were complete. Given F01, it means nothing will close the gap except
another blocked audit.

#### Violated contract or cost

`docs/roadmap/audit-guide.md` > `Audit workflow` provides a complete procedure for every situation
except the one where the selected scope does not exist. `Selecting an audit` step 6 is
unsatisfiable for unregistered code, and no step tells the auditor what to do about it.

`docs/roadmap/audit-kinds/documentation.md` > step 2 requires that documents state "how to route
narrower tasks" and "where deferred and design-pending work belongs". The guide routes registry
defects to an accepted Documentation finding but does not state that the originating run must
therefore stop, nor what the requester should do next.

#### Impact

Every future audit directed at any of the 95.4% of unowned implementation hits the same wall. The
cost is three sequential runs per subsystem before the requested audit can begin, and the failure
mode is silent — the auditor stops correctly, but the requester learns only that the audit did not
happen.

#### Root owner

`docs/roadmap/audit-guide.md`, sections `Audit scopes` and `Audit workflow`.

#### Suggested correction

**Non-authorising.** Two documentation-only corrections, plus one design element that is
deliberately blocked.

*Correction 1 — resolve the ambiguity in the read-only rule.* State explicitly, in
`audit-guide.md` > `Primary scope and required context`, whether an audit run may add a registry row
for the scope it was directed at. Either answer is a valid fix. If the answer is no, add the
corresponding workflow outcome: a directed audit of unregistered code files a Documentation finding
proposing the rows and stops, and the report is a scope-proposal report rather than a coverage
report. If the answer is yes, bound it — the run may add rows for its own directed scope with all
cells `N`, may not promote any cell in the same run, and must record the addition as a finding for
retrospective triage.

*Correction 2 — record declined and proposed scopes in the registry.* Add a short section to
`audit-log.md` listing scopes that were proposed by a finding and deliberately not registered, with
the finding ID and the reason. It would currently hold one entry:
`contract.module_compilation_handoff`, declined by AUD-0002-F06's triage because its consumer half
is unregistered. This is bookkeeping, not policy, and it stops the next auditor rediscovering a
settled decision.

*Design element, blocked.* Whether the registry should be partitioned systematically — one pass
that gives every maintained file a leaf owner — rather than incrementally is a design proposal
about how the framework should work. Per `documentation.md` step 17, design proposals "remain
blocked until explicitly approved. Do not implement them as documentation cleanup." It is recorded
here and not proposed. The scale it would have to cover is F01's table: 771 files across seven
systems plus `xtask`.

#### Allowed fix scope

`docs/roadmap/audit-guide.md` (the two named sections) and `docs/roadmap/audit-log.md` (one new
section). No change to any existing registry or freshness row under this finding.

#### Read-only context

`docs/roadmap/audit-kinds/**`, `docs/roadmap/audits/**`, `.claude/skills/audit/SKILL.md`,
`.agents/skills/audit/SKILL.md`, `AGENTS.md`.

#### Must preserve

- The separation of audit, triage and implementation. An auditor must not gain authority to accept
  its own findings.
- The rule that only a complete report promotes a freshness cell, and that no run may promote a
  cell it created.
- The existing authority split: `audit-guide.md` owns policy, `audit-log.md` owns data.

#### Forbidden fix forms

- Granting audit runs general write access to the registry.
- Allowing a run to register a scope and record coverage against it in the same run.
- Resolving the deadlock by deleting the "exactly one leaf" invariant.
- Implementing the blocked design element as part of this correction.
- Duplicating the resolved rule into the skill files instead of routing to the guide.

#### Required validation or measurement

Documentation release-build gate: `moth build docs --release`, or
`cargo run --quiet -- build docs --release`. Not `just validate`.

#### Dependencies and related findings

- Explains the consequence asserted by F01. F01 is the data defect; F02 is the process defect.
- Related to AUD-0002-F06 (closed), whose counter-evidence section is evidence for the current
  reading of the ambiguous rule.
- The blocked design element depends on nothing and blocks nothing; it is recorded, not queued.

#### Triage record

Not yet triaged.

### AUD-0004-F03: The `tests.harness` row cites a test file at a path it no longer occupies

- State: `candidate`
- Kind: `Documentation`
- Scope: `docs/roadmap/audit-log.md`
- Priority: `unassigned`

#### Evidence

The `tests.harness` registry row declares its primary coverage as:

> `src/compiler_tests/integration_test_runner/**` implementation plus
> `src/compiler_tests/{test_support,test_fs,test_diagnostics}.rs` and `frontend_pipeline_tests.rs`

The brace expansion places `frontend_pipeline_tests.rs` in `src/compiler_tests/`. It is not there.
`src/compiler_tests/` contains exactly `integration_test_runner/`, `test_diagnostics.rs`,
`test_fs.rs` and `test_support.rs`. The file lives at
`src/compiler_frontend/tests/frontend_pipeline_tests.rs` (24,008 bytes), is declared at
`src/compiler_frontend/mod.rs:122`, and three other documents cite it correctly at that path:
`index.md:174`, the constant-folding plan at line 193, and
`benchmarks/frontend-optimization-results.md:1609`. The registry row is the only reference using
the stale location.

`xtask/src/architecture_boundary/tests.rs:158` also asserts against the old
`src/compiler_tests/frontend_pipeline_tests.rs` path, but as a synthetic string input to a rule
function rather than as a filesystem reference, so it is not a second instance of this defect and
is out of scope for this run.

This was found by resolving every path cited by the registry against the worktree — 39 Rust files,
4 documents, 1 manifest and 6 glob roots. It is the only path that fails to resolve. All 11 canonical heading references also
resolve: `Source indexing and source sets`, `Prepared-source orchestration`, `Project and package
topology`, `Deterministic scheduling and graph outcomes`, `Generated-function boundary` and
`Architectural invariants` in `docs/build-system-design.md`; `Compiler input and result boundary`,
`Generated concrete functions`, `Frontend stages > Stage 4: AST semantics`, `Templates and TIR` and
`Stage 5: HIR and validation` in `docs/compiler-design-overview.md`.

#### Counter-evidence checked

**Is the brace expansion meant to cover only the first three names, with the fourth path-relative
to somewhere else?** *No.* The row supplies no other directory, and the connective "and" places the
fourth file in the same list as the braced three. A reader following the row cannot locate the file.

**Was the file deleted rather than moved?** *No.* It exists, is 24KB, is declared as a module by
`compiler_frontend/mod.rs`, and was modified by the current branch's Phase C work.

**Does this affect any recorded coverage?** *No.* `tests.harness` is `N` for every applicable kind,
so no report has claimed to cover this file under this row. The defect is navigational only, which
is why it is filed as a low-severity separate finding rather than folded into F01.

#### Violated contract or cost

`docs/roadmap/audit-kinds/documentation.md` > step 6: "file and directory names match the
repository" and "moved or renamed modules are updated".

`docs/roadmap/audit-guide.md` > `Scope registry maintenance` lists "ownership moves between
systems" as a review trigger. This file moved from the integration-test tree to the frontend tree,
which is an ownership move the registry did not follow.

#### Impact

An auditor selecting `tests.harness` cannot locate one of its four declared primary surfaces, and
would either omit it or conclude it was deleted. Because the file now lives under
`src/compiler_frontend/tests/`, it also falls inside the `tests.support` comparison scope's stated
surface ("Every test-only support/helper module under `src/**/tests/`"), so the stale path
additionally obscures which of the two scopes owns it.

#### Root owner

`docs/roadmap/audit-log.md`, the `tests.harness` row.

#### Suggested correction

**Non-authorising.** Correct the path to `src/compiler_frontend/tests/frontend_pipeline_tests.rs`.

Triage should also decide which scope owns it, because the move changed its neighbourhood: it is a
frontend stage-boundary test that `index.md` describes as covering "one stage at a time, for
handoffs a stage-local test cannot see", which is closer to a frontend test owner than to the
integration harness. If it stays in `tests.harness`, add an explicit note that this one file lives
outside `src/compiler_tests/`, so the next reader does not treat it as a typo. If it moves to
`tests.support`, remove it from the `tests.harness` row rather than citing it in both.

#### Allowed fix scope

`docs/roadmap/audit-log.md`, the `tests.harness` row and, if triage reassigns ownership, the
`tests.support` row.

#### Read-only context

`index.md`, `src/compiler_frontend/mod.rs`, `src/compiler_frontend/tests/`,
`docs/src/developer-docs/style-guide/testing.mtf`.

#### Must preserve

Each file owned by exactly one scope. Do not cite the file in both rows.

#### Forbidden fix forms

Moving the test file to match the documentation. This is a Documentation finding; production and
test files are read-only to it, and `audit-guide.md` > `Documentation and tests` states that a
non-test audit "must not add, remove, rewrite, regenerate or weaken tests".

#### Required validation or measurement

Documentation release-build gate. Confirm by inspection that the corrected path resolves.

#### Dependencies and related findings

Independent of F01 and F02; may be fixed in the same change.

#### Triage record

Not yet triaged.

## No-finding checks

Checked with evidence, produced no finding. Recorded so a later audit does not repeat them.

- **Registry-to-guide authority split is clean.** `audit-log.md` opens by stating it "does not
  store findings or process rules" and routes "Scope kinds, completeness and boundary rules" to the
  guide. The guide does not restate any scope row. No duplicated authority between them.
- **Every canonical heading cited by the registry resolves.** All 11 references across
  `docs/build-system-design.md` and `docs/compiler-design-overview.md` were matched exactly, at the
  heading level cited. No anchor drift.
- **The freshness and registry tables are consistent.** Both contain the same 11 scope IDs in the
  same order. No orphan row in either direction, no ID appearing twice.
- **Every existing `-` cell is defensible.** All 44 were checked against the kind guides.
  `tests.cases` correctly excludes Style, Comments, Correctness, Diagnostics and Performance — it is
  fixture data, not implementation. `tests.support` correctly excludes Correctness and Diagnostics
  as a Comparison scope over test-only helpers. The universal `-` in the Documentation column for
  code scopes is consistent: documentation is owned by the documents, not by code leaves.
- **Every existing `N` cell is genuinely available work.** No cell was found where the kind guide
  would be inapplicable but the cell reads `N`. No corrupted automatic selection in the existing
  rows.
- **All four intra-framework links in `audit-log.md` resolve** (`audit-guide.md`,
  `audit-kinds/README.md`, `open-audit-findings.md`, `audits/README.md`), and no circular routing
  was found: the guide routes to the kinds index and the log; the log routes back to the guide for
  rules only.
- **Dotted-ID convention is followed.** All 11 IDs are conceptual (`build.stage0.discovery`,
  `contract.assertion_message_runtime_handoff`) rather than path-encoded, satisfying "Do not encode
  a path that may move into the identity". The proposed IDs follow the same convention.
- **No generated-output ownership problem.** `docs/roadmap/**` is not part of the `docs` release
  build and produces no output under `docs/release/**`. Nothing in the audit framework is generated
  or edited-as-generated.
- **Registry prose meets repository conventions.** British English, rules before caveats, direct
  active voice, no filler. No prose finding.
- **`index.md` correctly does not describe the audit framework.** It is a file and module locator;
  `AGENTS.md` routes audit tasks to `docs/roadmap/audit-guide.md` directly. No competing authority.
- **The progress matrix correctly contains no audit-framework row.** Searched
  `docs/src/docs/progress/@page.moth`; the matrix tracks implementation of accepted language and
  compiler design, and the audit framework is neither.

## Leads outside this scope

Recorded without expanding coverage or claiming a freshness cell, per `audit-guide.md` > `Scope
kinds`.

- **L01: the audit skill duplicates repository audit policy rather than routing to it, in two
  byte-identical copies.** `.claude/skills/audit/SKILL.md` and `.agents/skills/audit/SKILL.md` are
  identical files. Each states "Repository audit documents own all policy, evidence thresholds,
  preservation rules, report format, freshness and lifecycle. This skill only selects and conducts
  one audit", and then restates the selection precedence table, the six automatic-selection steps,
  the seven workflow steps, the counter-explanation checklist and the finding schema — all of which
  `audit-guide.md` owns. It also adds two rules the guide does not contain: a tie-break preferring
  "current documented risk or recent material change, then the smallest scope that can be
  completed, then lexical scope ID for determinism", and an instruction to remove the report from
  `Audits in progress` on close. `documentation.md` step 2 names "skills or agent instructions
  duplicating repository policy instead of routing to it" as a check, and step 14 names "audit
  skills route to audit documents rather than copying their policy". This is a real drift surface
  in three places at once. It is not filed as a finding because the skill files are not the
  registry and belong to a documentation scope over the audit framework as a whole — which does not
  exist, which is F01's problem recurring. A future `docs.audit_framework` scope should own it.

- **L02: generated release documentation is one commit stale.** Running the documentation gate for
  this report regenerated `docs/release/docs/progress/index.html` with a seven-line addition. The
  diff is not caused by this run: HEAD `54d5ab1a7` ("docs(progress): record range values outside
  loop headers as deferred") edited `docs/src/docs/progress/@page.moth`, while
  `docs/release/docs/progress/index.html` was last rebuilt at `82d22abc2`, four commits earlier. The
  generated page therefore does not yet carry the deferred `Range` row that the progress source
  added. `validation.mtf` > `Documentation-only completion checklist` requires that "Documentation
  was rebuilt when required". The regenerated file was reverted rather than committed, because this
  audit's write scope is its own report, `open-audit-findings.md` and `audit-log.md` only, and
  because the rebuild belongs to whoever owns the progress-matrix change. `documentation.md` step 15
  names generated-output currency as an in-scope check, but the owner here is the progress matrix,
  not the registry, so this is a lead rather than a finding.

## Validation

`docs/src/developer-docs/style-guide/validation.mtf` > `Documentation-only release-build gate`
applies: every changed file is documentation, so `just validate` must not be run and was not run.

**Ran and passed.** `./target/release/moth build docs --release`, using this worktree's release
binary. Exit `0`, 72 modules compiled, no diagnostic. The build was run from this worktree only;
`which moth` resolves to a different worktree and was deliberately not used, per `AGENTS.md`.

Completion checklist, per the validation guide:

| Check | Result |
|---|---|
| The changed-file list contains documentation only | yes — `docs/roadmap/audits/AUD-0004-audit-scope-registry-coverage.md` (new) and `docs/roadmap/open-audit-findings.md` (modified). No Rust, test, build, manifest, script, benchmark, fixture or configuration file changed. |
| The documentation release build succeeds | yes |
| Changed routes, links, tables and examples were inspected | yes — this report's tables and its 40-odd relative and anchor links, and the three new anchors added to `open-audit-findings.md`, were checked against the `audits/README.md` entry format |
| Generated output was produced from source changes | not applicable to this run's changes — `docs/roadmap/**` is not part of the `docs` release build and produces no output under `docs/release/**` |
| No generated HTML was edited manually | yes |

**Not run, deliberately:** `just validate`, Clippy, unit tests, integration tests, benchmark checks
and `cargo fmt`. The validation guide states directly: "Do not additionally run `just validate`,
Clippy, unit tests, integration tests or benchmark checks for a documentation-only slice."

One pre-existing condition was surfaced by the gate and is recorded as lead L02: the build
regenerated `docs/release/docs/progress/index.html` because that page is one progress-source commit
stale. The regenerated file was reverted, leaving the worktree's generated output exactly as found.

## Limitations

- **The subject of this audit is itself unregistered, which this report cannot fix.** There is no
  registry row for `docs/roadmap/audit-log.md` and no freshness cell for a Documentation audit of
  it. This report therefore audits a surface it has no authority to record coverage against — the
  same circularity F02 describes, applied to the framework's own documents. The report is real
  evidence and can be triaged; it is not, and cannot be, recorded coverage. Registering a
  `docs.audit_framework` scope covering `audit-guide.md`, `audit-log.md`, `audit-kinds/**`,
  `audits/README.md`, `open-audit-findings.md` and the skill files would close this, and is
  deliberately not proposed here — F01 is already at the limit of what one finding should propose,
  and proposing a scope whose only purpose is to record this report would be self-serving.
- **Coverage is `partial`, and would be even if a scope existed.** Three grounds. First, the guide
  requires it: "If a registered scope is incomplete ... record the scope defect ... and mark the
  audit partial." Second, four of the kind guide's 18 steps were not applicable and two more were
  applied partially, so the completion checklist cannot be fully satisfied. Third, the documents
  that depend on the registry — the eight audit-kind guides, the two skill copies — were read for
  routing and ownership but not audited exhaustively, and `documentation.md` > `Valid scopes` states
  that "A complete documentation audit must identify the authority owner and inspect every in-scope
  dependent document".
- **The unowned-surface measurement is mechanical.** The 791/245,118 figures come from file and
  line counts with test paths excluded by pattern (`*/tests/*`, `test_support.rs`,
  `src/compiler_tests/`). A file containing only `#[cfg(test)]` content outside those patterns
  would be miscounted as production. The margin does not affect the finding: even at a generous
  discount the unowned share stays above 90%.
- **No implementation was audited.** The folded-value modules were read at module-doc, import and
  ownership level, enough to place leaf boundaries. This report makes no correctness, performance
  or redundancy claim about that code, and the Phase C findings quoted from the plan are cited as
  the plan's evidence, not re-verified here.
- **The proposed boundaries are untested by use.** No audit has run against
  `frontend.const_values`, `contract.folded_constant_handoff` or
  `feature.module_constant_folded_values`. The guide's too-broad/too-narrow test was applied by
  inspection of file sizes, dependency direction and module doc comments; the first audit to run
  against them may find a boundary needs moving. That is expected and is why the rows are a
  suggested correction rather than a registration.
- **The branch is mid-plan.** `const-folding-and-types-optimisation` is between Phase C and Phase D.
  The frontend surface named by the proposed rows is actively changing, and Phase D is scheduled to
  touch `folded_value.rs` and the type-resolution views. Rows registered now may need their paths
  revisited after Phase D.

## Freshness update

**No freshness cell was updated, and none could be.**

The audited subject — the scope registry in `docs/roadmap/audit-log.md` — has no registry row and
therefore no row in the freshness matrix. `audit-guide.md` > `Freshness` permits promotion only of
an existing entry, and this run has no authority to create one: F02 records that registry rows are
added through accepted findings, not by audit runs, and this report is bound by the same rule it
reports.

`docs/roadmap/audit-log.md` is therefore unchanged by this run. The 11 existing scope rows, the 11
freshness rows and all 88 cells are exactly as they were at `54d5ab1a7`.

If F01 is accepted and the three proposed scopes are registered, they enter at `N` for every
applicable kind. This report must not be recorded against any of them: it audited the registry, not
the folded-value pipeline, and back-dating a cell to cover it would be the exact forbidden fix form
F01 names.

---

## Outcome

All three findings were accepted and resolved on branch `const-folding-and-types-optimisation`, but
not in the shape the suggested corrections proposed.

F01 and F02 were resolved by **simplifying the framework instead of extending the registry**. The
maintainer's judgement was that a two-table registry gating 3,758 lines of policy over 11 scopes and
three completed audits had the ratio backwards, and that the deadlock F02 identified was a symptom
of the registry being a permission list rather than a record.

The scope registry is now a coverage ledger. An audit adds the row for the area it covers as part of
its run, so no registration step exists that can deadlock. The Leaf/Composite/Contract/Comparison
taxonomy is gone - an audit names its area and, for a boundary audit, what sits on each side. The
separate registry and freshness tables are one table, removing the sync burden. A `Never audited`
section carries this report's 771-of-791 measurement, which is the signal F01 correctly showed the
old structure could not give.

This report's proposed scopes (`frontend.const_values`, `contract.folded_constant_handoff`,
`feature.module_constant_folded_values`) were therefore not registered. Under the new model the
Phase C audit names the area it covers and records it on completion.

F03 was fixed as reported: `tests.harness` now cites the real path of
`frontend_pipeline_tests.rs`.

Two of this report's incidental observations were also acted on. Lead L01 (the two byte-identical
skill copies duplicating policy the guide owns) is resolved - the skill is now selection and routing
only, and the duplicated precedence rules are gone. The `frontend.ast` / `analysis.borrow` phantom
example found while testing counter-explanation 4 no longer exists, since the guide no longer
carries a scope-override syntax.

The root cause of the original block turned out to be narrower than either agent's reading. The
constant-folding plan's obligation was the `AGENTS.md` **Final audit**, a self-review checklist that
never required a registered scope. Sharing a name with this framework is what sent two agents
looking for one. That section is now called **Slice review**, and both documents state the
distinction.

