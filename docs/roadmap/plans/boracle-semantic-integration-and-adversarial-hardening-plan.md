# Boracle semantic integration and adversarial hardening plan

## Purpose

Connect the existing Boracle extraction and analysis components into one trustworthy reference
borrow solve. This is a focused follow-up to the initial normalized-problem milestone. It hardens
the current ownership boundaries; it does not replace `BorrowProblem`, redesign the solver as a
production checker or begin lifetime-topology implementation.

The target relationship is:

```text
validated HIR
    -> normalized semantic facts
    -> origin propagation
    -> alias and loan derivation
    -> origin-aware overlap and conflict witnesses
    -> use-driven loan liveness
    -> origin- and event-aware last use
    -> conservative optional-transfer advice
```

Until this relationship is complete, Boracle output is an inspection aid for normalized input and
individual algorithms, not evidence that a Moth program should pass borrow validation.

## Current-state capsule

```text
ACTIVE_PLAN: docs/roadmap/plans/boracle-semantic-integration-and-adversarial-hardening-plan.md
STATUS: queued
CURRENT_SLICE: Phase 0 - re-anchor over current main and establish the adversarial source corpus
BLOCKERS: the initial Boracle milestone is not yet a semantic oracle; current main has subsequent test-placement changes to integrate at activation
NEXT_ACTION: activate on a clean Boracle worktree, establish the baseline, integrate current main, and run the red-first source corpus
```

The initial Boracle plan has been retired. Its implementation commits remain the historical record,
but this plan deliberately starts with a fresh activation baseline. Do not add a baseline commit to
this status block before activation; record it in the first working checkpoint instead.

## Scope and authority

This plan owns the integration and adversarial-hardening sequence for the feature-gated Boracle
reference mode. The permanent semantic authority remains:

- `docs/src/docs/codebase/memory-management/borrow-validation/boracle-reference-solver.mtf`

Read these authorities before activation and reload the relevant sections before every phase:

- `AGENTS.md`
- `docs/compiler-design-overview.md`, including its opening authority, `Architectural invariants`,
  Stage 5 HIR and validation, Stage 6 borrow validation, generated concrete functions and
  per-function link facts
- `docs/build-system-design.md`, including its opening authority, `Architectural invariants`,
  Stage 0/module compilation handoff and `Generated-function boundary`
- `docs/src/docs/codebase/memory-management/overview.mtf`
- `docs/src/docs/codebase/memory-management/access-and-aliasing/overview.mtf` and its canonical
  reference
- `docs/src/docs/codebase/memory-management/borrow-validation/overview.mtf`
- `docs/src/docs/codebase/memory-management/borrow-validation/borrow-validation.mtf`
- `docs/src/docs/codebase/memory-management/borrow-validation/boracle-reference-solver.mtf`
- `docs/src/docs/codebase/memory-management/ownership-and-drops/overview.mtf` and its canonical
  reference when optional transfer or final-use ownership is touched
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- the relevant sections of `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf` before selecting or reporting a gate
- `docs/src/docs/progress/@page.moth`
- `docs/roadmap/roadmap.md`

When a source case depends on exact syntax or a user-visible acceptance rule, also read the
canonical unsuffixed language reference selected by `docs/src/docs/codebase/language/overview.mtf`.
Do not derive source semantics from the current Alpha implementation or from a Basic teaching page.

## Locked boundaries

The following boundaries remain fixed throughout the plan:

- The Alpha checker remains the normal compiler authority. Normal compilation and `just validate`
  must not construct or run Boracle.
- Boracle reads validated HIR and writes analysis facts beside it. It does not reparse source,
  mutate HIR, repair malformed HIR, lower a second frontend or decide backend ownership.
- `BorrowProblem` remains immutable, function-local, typed by dense ID spaces and atomically
  validated before analysis.
- Boracle owns reference-level access, provenance, loan, conflict and optional-transfer facts. It
  does not assign lifetime owners, validate retained-edge topology or cycles, choose a memory
  strategy, select REC, emit drops or lower a backend.
- Missing optional-transfer proof remains conservative borrowing, never a source diagnostic.
- Unknown call provenance is conservative uncertainty. It must never be represented as fresh
  independent storage merely because a summary is unavailable.
- The reference mode remains deterministic, inspectable and deliberately unoptimised. Experiments
  must be named and cannot silently change reference results.
- No external solver dependency, public solver-selection flag, source lifetime syntax or production
  checker replacement is part of this plan.

## Semantic gate

The follow-up is complete only when a real source program can travel through the same internal
source service used by the Boracle command, produce one normalized problem, and receive a report
whose origin, loan, liveness, conflict and optional-transfer facts agree with the accepted source
semantics. Unit tests for isolated origin, last-use or loan tables are necessary but insufficient.

The minimum source corpus is:

| Case | Required reference result |
| --- | --- |
| shared alias used after mutation | conflict |
| shared alias's final use before mutation | allowed |
| copy mutated while the source alias remains live | allowed |
| old alias after a fresh source rebind | old and new origins remain separate |
| local mutable-parameter call | exclusive argument access |
| unknown result that may alias an argument | conservative overlap; no independence proof |
| aggregate field storing a source alias | projected field resolves to the source origin |
| map `get` followed by map mutation | conflict while the returned alias is live |
| map `remove` result | detached or conservative provenance, never fresh independence |
| branch-separated use and mutation | allowed when no path observes both |
| final call/store argument use | optional transfer queried after the exact consuming event |
| loop-carried alias and rebind | deterministic fixed-point result |

Phase 0 establishes these as source-service expectations and records the red baseline before the
implementation fixes. A red-first probe may fail during the phase, but no accepted checkpoint may
leave an unbounded or silently skipped red test in `just boracle`. The source cases become ordinary
passing regression tests as their owning phases land.

## Phase 0: Re-anchor and add the adversarial source corpus

### Goal

Start from the current repository rather than the retired plan's assumptions, integrate the
subsequent `main` test-placement changes, and make the semantic gaps executable.

### Work

- Confirm the active worktree, branch, clean/known-change state and the current `main` tip. Rebase
  the Boracle branch over current `main` (or perform an explicitly equivalent clean integration)
  before moving source-test ownership forward. Resolve test moves through the current test layout;
  do not restore inline production tests or create a second test harness.
- Inventory the current source service, HIR extractor, normalized-problem tests, Boracle solver
  tests, Alpha borrow tests and compiler integration harnesses. Identify one owner for each new case.
- Add a real-source Boracle corpus that uses the compiler-owned Stage 0/HIR path and the internal
  Boracle service. It must not call `from_hir` with hand-authored summaries when the case is testing
  the actual source boundary.
- Assert typed report facts and source mappings, not only dump strings. Keep expected acceptance,
  conflict witnesses, origin relationships and transfer-event identity separate so a passing dump
  cannot hide a missing semantic link.
- Run every expectation against the pre-fix branch and record the mismatch category in the phase
  checkpoint. The Alpha result is a comparison datum only; it is not the expected-answer oracle.

### Non-goals

- No solver correction is smuggled into the corpus helper.
- No test is made green by weakening its expected semantic result or by treating an empty report as
  an acceptable conservative outcome.
- No public language support claim is changed in this phase.

### Exit criteria

- The branch is cleanly based on current `main`, and the test-placement changes are preserved.
- Each row in the semantic gate has a real-source case or a documented source-boundary blocker.
- The red baseline is reproducible through a bounded, named test command.
- A phase checkpoint records the baseline, test owner, expected result and the next correction phase.

## Phase 1: Complete HIR-to-problem semantic facts

### Goal

Make the normalized problem faithfully describe validated HIR before asking any solver to infer
meaning that the extractor discarded.

### Work

- Seed entry-state origins for function parameters with a positive parameter-origin identity such
  as `ParameterOrigin(parameter_index)`. Loading a parameter must never begin with an empty origin
  set merely because no fresh definition event occurred in the function.
- Preserve source-local mutability/access facts in the HIR handoff or in an explicit normalized
  binding fact. Do not reconstruct mutability from traversal order or source text. Distinguish a
  shared alias from an exclusive alias and keep binding mutability separate from the loan issued by
  one access.
- Emit one normalized alias issuance fact for every source alias operation. Ordinary `Load`
  assignment must not silently be the only shared-looking path, and mutable alias forms must emit
  `ExclusiveAlias`/`ExclusiveAliasFromPlace` (or their one current replacement) when validated HIR
  says the access is exclusive.
- Establish one owner for source-loan derivation. The preferred shape is for extraction to retain
  alias/access/holder/use facts while the solver derives the resolved loan rows after origin flow;
  an equivalent design is acceptable only if it has one source of truth and no builder/solver
  duplicate paths. Publishing an empty loan table for a source function is not an accepted outcome.
- Add explicit parameter, result-write and per-call-argument event boundaries. Preserve evaluation
  order within a call, including argument expressions, argument accesses, receiver/index reads and
  the result write. A vector index is not an event boundary.
- Represent scope exits on the edge that loses visibility. If a binding survives one successor but
  not another, use edge-specific events or split edge blocks rather than omitting the kill from all
  paths.
- Derive first-write versus rebind meaning from CFG/dataflow state. A global builder traversal set
  must not decide whether a branch-local write is a rebind.
- Retain exact HIR/source links for all new boundaries and preserve deterministic ordering under
  repeated extraction.

### Required tests

- parameter loads have a non-empty parameter origin;
- shared and exclusive source aliases produce different normalized facts;
- source alias issuance is present without hand-authored `BorrowProblem.loans()`;
- branch-local first writes do not depend on block visitation order;
- scope-exit kills differ on the appropriate outgoing edges;
- same-call argument and result events have stable exact order;
- renumbering or reordering unrelated HIR containers does not change semantic event order.

### Exit criteria

The source corpus still may fail semantic assertions, but no failure is attributable to discarded
HIR facts, missing parameter origins, traversal-order classification or coarse event boundaries.
`BorrowProblem` validation, HIR extraction tests, `just boracle` and the full validation gate pass.

## Phase 2: Integrate origin flow with loans and overlap

### Goal

Make loan derivation and conflict checking consume the resolved origin solution rather than treating
each binding root as an independent value.

### Work

- Change the Boracle solve sequence to pass `OriginSolution` into the loan/conflict solver. Do not
  merely store origin and loan results beside one another in `BoracleReport`.
- Resolve alias and projection origins before deciding whether a loan covers an access. Structural
  place overlap remains necessary for projections within a binding; related origin overlap handles
  aliases across different binding roots.
- Treat independent copy origins and unrelated fresh origins as disjoint even when their bindings
  share a historical source or have similar place shapes.
- Give unknown/top provenance an explicit relation that overlaps every plausible related origin or
  prevents independence proofs. It must not collapse into the fresh-origin branch.
- Carry resolved origin IDs and their derivation path into `ConflictWitness` and `AccessDecision`
  so a conflict can be inspected without reverse-engineering a binding name.
- Keep disjoint fixed fields disjoint when the normalized projection semantics prove that fact, while
  preserving conservative receiver-base overlap for maps, dynamic indexes and structural mutation.

### Required tests

- a shared alias on another binding conflicts with mutation of the source;
- an exclusive alias conflicts with every overlapping shared or exclusive access;
- an explicit copy can be mutated while source aliases remain live;
- fresh rebinds separate the new value from old aliases;
- unrelated fresh origins do not conflict merely because their places have different roots;
- unknown result provenance blocks an independence-based acceptance;
- witnesses identify the related origins and not only the normalized place roots.

### Exit criteria

The source alias rows in Phase 0 produce the expected conflict/allowed outcomes through one
origin-aware solver path. Direct normalized fixtures remain useful, but no source result depends on
the fixture loan table being manually populated. `just boracle` and `just validate` pass.

## Phase 3: Implement use-driven loan liveness

### Goal

Replace reachability-until-kill with the intended reference liveness: a loan is live only where a
future use can still be reached without crossing a valid kill.

### Work

- Associate each derived loan with the observations that can use it, including uses through aliases,
  projections, aggregate children and call arguments after origin resolution.
- Compute liveness backwards from those uses over the normalized CFG. Stop at rebind, scope-exit,
  explicit and path-specific kills. Branches merge possible liveness; loops use a terminating
  fixed point over the finite normalized graph.
- Keep an issue event distinct from its first later use. A loan with no future observation must not
  remain live merely because a continuation is reachable.
- Select a keeping-alive witness that is reachable after the conflicting access and is actually on a
  path responsible for the live result. Do not use `loan.uses.first()` as a structural placeholder.
- Centralise same-call argument conflict handling. Each source conflict receives one canonical
  witness, not one witness from the ordinary access loop plus a mirrored pairwise pass.
- Keep dead-exclusive-loan behaviour as a named experiment until canonical semantics promote it; the
  reference mode must still report the evidence required by its accepted liveness rule.

### Required tests

- an unused alias loan ends before a later exclusive access;
- a future alias use keeps the loan live and rejects the mutation;
- a branch with a use and a branch without a use yields path-sensitive `MayBeUsed`/liveness facts;
- loop-carried alias uses converge deterministically;
- a kill on one outgoing edge does not kill the loan on a surviving edge;
- one same-call conflict produces one structured witness with the correct keeping use.

### Exit criteria

Loan live-point dumps agree with future-use observations rather than lexical reachability. The
source map and witness fields identify the relevant event path. `just boracle` and `just validate`
pass after the independent phase review and any correction cycle.

## Phase 4: Make last use origin- and event-aware

### Goal

Make modular last-use facts answer questions about the represented value and loan at the exact
source event, not only about the current binding place at a coarse program point.

### Work

- Convert origin-flow facts into `LastUseObservation` rows owned by the solver boundary. An access
  through an alias contributes to the represented origin(s), and a loan use contributes to its loan
  subject. Do not ask `LastUseAnalysis::from_problem` to rediscover origin meaning from places.
- Exercise `LastUseSubject::Origin` and `LastUseSubject::Loan` in the integrated report, with
  deterministic witnesses and source/event mappings.
- Make redefinition generation-aware. A later access to the same binding must not count as a future
  use of an old origin after a replacement value has killed that place generation.
- Make exact `after_event` locations first-class in the report and transfer API. A final read,
  argument access or store candidate must query immediately after its own event, not at a later point
  that merely eventually reaches `NoFutureUse`.
- Attach optional-transfer candidates to the represented origin family and exact consuming event.
  Missing, ambiguous, reactive or path-dependent proof remains borrowing.
- Retain place-level queries for structural diagnostics and later consumers, but do not use them as
  the only optional-transfer or final-use authority.

### Required tests

- an old alias remains usable after the source binding is rebound, while the new origin is separate;
- a use through another alias is attributed to the shared origin;
- a final call/store argument is `NoFutureUse` immediately after that argument event;
- the same candidate is not incorrectly accepted when a later alias use exists;
- loan-level queries identify the last keeping use;
- branch joins and loop fixed points preserve `MayBeUsed` conservatism.

### Exit criteria

The report contains integrated origin and loan last-use rows, and optional transfer decisions are
event-boundary decisions. The old later-point smoke assertion is removed or replaced by the exact
event assertion. `just boracle` and `just validate` pass.

## Phase 5: Harden calls, generated functions and stored values

### Goal

Close the source-boundary gaps that can otherwise manufacture false independence or discard stored
provenance.

### Work

- Add compiler-owned preliminary local parameter-access facts before the Boracle source boundary
  when no completed local summary is available. Consume already published imported and external
  summaries without making the CLI or Stage 0 reconstruct compiler semantics.
- Define the generated-function boundary explicitly. Boracle may consume completed generated
  sidecar summaries and external facts; where generated/local convergence is not available at this
  boundary it must retain a conservative unknown contract, never assume fresh storage.
- Model unknown call results as an explicit top/unknown provenance domain. Unknown must overlap every
  plausible argument origin or block any proof that relies on independence. Add tests for wrappers
  that return one of their arguments.
- Preserve aggregate child-origin state through construction and projection. Repeated fields that
  store one origin must remain related to that origin; a projected child must not be replaced by an
  unrelated fresh projection origin.
- Model map `get` as a shared alias to stored data and protect the receiver while the result remains
  live. Model `remove` as detached or conservatively related stored provenance, never `Fresh`.
- Preserve success/error-specific call, unwrap and fallible-flow provenance and liveness. A
  failure-only path must not keep a success-only value live.
- Keep final retained-edge ownership, detached-result lifetime regions and topology decisions out of
  Boracle; only the preliminary access/provenance relation belongs here.

### Required tests

- the real source service exercises a local mutable parameter without injecting a manual summary;
- an unavailable local summary gives shared/exclusive facts conservatively and deterministically;
- unknown result wrappers cannot prove independent mutation safe;
- aggregate field projection reaches the stored child origin, including repeated same-origin fields;
- `get` followed by mutation conflicts only while the result is live;
- `remove` never appears as a fresh independent result in the origin dump;
- fallible success/error branches retain only the provenance reachable on that branch;
- generated calls are either backed by explicit available facts or visibly unknown.

### Exit criteria

All Phase 0 source cases pass through the actual Boracle service, including calls, aggregates,
maps and fallible control flow. The service owns the boundary and does not call Alpha merely to
manufacture a summary. `just boracle` and `just validate` pass.

## Phase 6: Expand semantic properties and differential evidence

### Goal

Turn the generated and source corpus into semantic evidence rather than only deterministic table
construction checks.

### Work

- Extend bounded generated problems with expected semantic properties for copies, aliases, rebinds,
  origin overlap, liveness and final-use decisions. Keep seeds, bounds and reductions deterministic.
- Add metamorphic checks:
  - adding an unreachable use does not change legality;
  - replacing an alias with an explicit copy cannot introduce an alias conflict;
  - adding a future alias use cannot make a previously conflicting mutation appear legal;
  - a fresh rebind separates the new origin from old aliases;
  - renaming or reindexing bindings does not change semantic results;
  - splitting a branch into equivalent CFG blocks does not change the result.
- Compare against Alpha only on a deliberately shared subset. Classify every disagreement as a
  Boracle defect, Alpha limitation, input-builder defect, accepted experimental difference or
  unsettled semantic question. Do not turn agreement into proof or disagreement into an automatic
  correction.
- Add reduced-case reporting for generated failures, including the seed, normalized problem,
  origin solution, loan solution, last-use rows and witness paths.
- Keep the real-source corpus as the primary semantic gate; generated properties supplement it rather
  than replacing source-boundary coverage.

### Exit criteria

Generated tests assert bounded semantic properties and produce useful reductions. Differential
comparisons are explicitly classified and do not change reference mode without an accepted semantic
decision. `just boracle` and `just validate` pass.

## Phase 7: Correct status and close out the follow-up

### Goal

Make the permanent documentation truthful only after the integrated reference mode has passed the
semantic gate, and retire this plan in the same commit as its completed work.

### Work

- Update the Boracle design authority with the integrated solve order, unknown/top contract,
  origin-aware overlap, use-driven liveness, exact event-boundary transfer and stored-child
  provenance rules that were actually implemented.
- Update the progress matrix to distinguish the implemented normalized model and independent
  analysis scaffolds from the now-integrated semantic reference mode. Keep the Alpha checker as the
  normal authority and keep lifetime topology, REC, memory planning and the optimized production
  checker deferred.
- Update the roadmap wording so the follow-up is not presented as a production-checker plan. Do not
  add a new audit-log coverage row unless a separately invoked structured audit requires one.
- Rebuild generated documentation through the compiler; never edit `docs/release/**` directly.
- Run the final independent audit and correction cycle, complete the Slice review, then run both
  `just boracle` and `just validate`, the documentation release build and the relevant source smoke
  tests from a clean worktree.
- Delete this plan and its roadmap entry in the same completion commit. Do not mark the plan complete
  and leave it committed as a stale work item.

### Exit criteria

- The semantic gate is green for every Phase 0 case.
- Origin, loan, last-use, conflict and optional-transfer facts are connected in one deterministic
  reference solve.
- Unknown and stored-value boundaries are conservative and visibly represented.
- Phase-by-phase checkpoints, independent reviews, correction cycles and validation results are
  recoverable from Git history and the final documentation.
- The worktree is clean and the completed plan is deleted with its roadmap entry.

## Phase gates and durable evidence

Every non-trivial phase follows the same order:

1. Reload the relevant architecture, memory, testing and validation authorities.
2. Inspect the current owner and adjacent paths for duplicate or superseded logic.
3. Implement one bounded slice, using implementation-coordinator workers only for simple,
   independently reviewable work such as fixture inventory, source-case plumbing or generated
   property scaffolding. The parent retains integration and semantic decisions.
4. Run focused tests, perform the Slice review and request an independent read-only review of the
   phase boundary.
5. Correct every accepted finding before advancing the phase.
6. Run `just boracle` for every code-bearing checkpoint and `just validate` before accepting each
   completed code-bearing phase. Documentation-only checkpoints use the documentation release gate.
7. Update this plan's status block and commit the phase checkpoint with the exact commands and
   outcomes. Do not claim an audit, worker result or command that did not occur.

An independent review is not a substitute for the structured audit framework. If the user explicitly
invokes an audit, follow `docs/roadmap/audit-guide.md` and its selected guide separately. Slice
reviews do not update the audit log.

## Delegation policy

Use implementation-coordinator workers for bounded procedural slices where ownership and expected
behaviour are already clear, for example:

- moving or extending source-test fixtures after Phase 0 ownership is settled;
- adding deterministic event-boundary assertions;
- adding generated metamorphic cases with a fixed property contract;
- auditing dump determinism or source-location coverage.

Keep these decisions with the parent coordinator:

- the origin domain and unknown/top semantics;
- which stage owns preliminary call facts;
- the loan-derivation source of truth;
- liveness and optional-transfer rules;
- any change to accepted language semantics or permanent documentation.

If a configured worker provider is unavailable, record that fact in the phase checkpoint and perform
the bounded slice locally. Do not invent worker evidence.

## Non-goals

This plan must not:

- replace or invoke Boracle from the normal Alpha compiler path;
- make the Alpha checker a semantic oracle;
- implement the optimized production borrow checker;
- assign lifetime owners, validate retained edges or cycles, select REC or choose memory plans;
- make `remove` or unknown calls look independent to improve acceptance;
- add a second parser, source scanner, HIR rewriter or backend-facing IR;
- turn reactive observations into active borrow loans;
- add external solver dependencies or optimize Boracle for production speed;
- delete or weaken tests because the current source service cannot yet express their expected facts.

## Final acceptance checklist

- [ ] Current `main` integration and test ownership are recorded at activation.
- [ ] The full adversarial source corpus exists and was red-tested before corrections.
- [ ] Parameters, mutability, alias kinds, event order, edge kills and CFG rebind meaning are
      explicit in normalized input.
- [ ] Source aliases derive real shared/exclusive loans.
- [ ] Loan overlap consumes origin flow and distinguishes aliases, copies, fresh values and unknown.
- [ ] Loan liveness is bounded by reachable future uses and valid kills.
- [ ] Witnesses name the use/path that keeps a loan live and same-call conflicts are not duplicated.
- [ ] Last use consumes origin and loan observations and queries exact event boundaries.
- [ ] Optional transfer is origin-aware and conservative.
- [ ] Calls, generated boundaries, aggregates, map `get`/`remove` and fallible paths are covered.
- [ ] Generated tests assert semantic properties and differential disagreements are classified.
- [ ] Permanent docs and progress wording are accurate.
- [ ] Independent review, correction cycle, Slice review, `just boracle`, `just validate` and the
      documentation release build all pass before closeout.
- [ ] This plan and its roadmap entry are deleted in the completion commit.
