# Boracle Reference Solver Initial Implementation Plan

> **Repository path:** `docs/roadmap/plans/boracle-reference-solver-implementation-plan.md`
>
> **Permanent design authority:** `docs/src/developer-docs/memory-management/borrow-validation/boracle-reference-solver.mtf`
>
> **Status:** Active on the `boracle` branch and worktree. Phase 0 is complete and audited. The current alpha borrow checker remains the normal compiler authority. Phase 0 changed no compiler behaviour itself, but the branch carries `7571448bf`, a separately approved fix to the alpha checker's future-use liveness, authored and validated on `main`. Phase 0 working notes live in this worktree's untracked `tmp/boracle-phase0-notes.md` and hold the owner map, baseline record, semantic-case inventory and audit record.
>
> **Current slice:** Phase 2 complete; the immutable normalized problem vocabulary, atomic validator and direct semantic fixture seam are checkpointed. Phase 3 is next.
>
> **Blockers:** None for the initial reference-solver work. Do not start the production borrow-checker replacement, lifetime-topology implementation or REC integration in this plan.
>
> **Coordinator:** `WORK_ID=boracle-reference-solver`, baseline `a92effb1d`; Phase 1 is checkpointed at `080a18b74`, and the Phase 2 audit and mandatory gate are complete.
>
> **Next action:** Define the HIR-to-problem owner and exact event ordering for Phase 3 without invoking it from the alpha checker path.

## Purpose

Build **Boracle**, Moth's permanent slow reference borrow solver and borrow-analysis laboratory.

Boracle has two linked roles:

1. It is an executable reference model for Moth borrow legality, value provenance, loan liveness and last-use reasoning.
2. It is a proving ground for stronger safe analyses that may later shape the production borrow checker and downstream memory optimisations.

The project starts with explicit `copy` semantics, binding-independent value origins, storage generations, normalized places, path-sensitive loans and a modular last-use analysis. These are the minimum concepts needed to escape the current alpha checker's `LocalId`-centred root approximation.

Boracle is deliberately slow. Readability, inspectability, determinism and semantic clarity outrank runtime, memory use and compact representation. It must not use external solver libraries. Straightforward Rust graph traversal, explicit fixed-point loops and ordinary deterministic collections are preferred even when they are much slower than a production implementation.

The first implementation milestone ends when the permanent reference facility, its shared input model, its deterministic validation command and its initial semantic corpus are usable. Boracle's research lifecycle does not end there. Future borrow-checker work may add experiments, cases, queries and analysis outputs without reopening this initial plan.

## Authority and reading order

Before each implementation slice, read these files from the active Boracle worktree, not another worktree:

1. `AGENTS.md`
2. this plan and its current phase
3. `docs/src/developer-docs/memory-management/borrow-validation/boracle-reference-solver.mtf`
4. `docs/compiler-design-overview.md`
5. `docs/src/developer-docs/memory-management/overview.mtf`
6. `docs/src/developer-docs/memory-management/access-and-aliasing/access-and-aliasing.mtf`
7. `docs/src/developer-docs/memory-management/borrow-validation/borrow-validation.mtf`
8. `docs/src/developer-docs/memory-management/ownership-and-drops/ownership-and-drops.mtf`
9. `docs/src/developer-docs/style-guide/style-guide.mtf`
10. the relevant sections of `docs/src/developer-docs/style-guide/testing.mtf`
11. `docs/src/developer-docs/style-guide/validation.mtf` before choosing or reporting a phase gate
12. `docs/src/docs/progress/@page.moth`
13. `docs/roadmap/roadmap.md`
14. the current borrow-checker, HIR, CLI, feature-matrix and test owners touched by the slice

When a slice changes exact language semantics rather than only analysis precision, also read the canonical unsuffixed language references routed by `docs/src/developer-docs/language/overview.mtf`.

The permanent Boracle design document owns the subsystem contract. This plan owns the initial implementation sequence. The progress matrix owns current support. Compiler behaviour does not override accepted design.

## Locked decisions

These decisions came from the design discussion and user interview. Do not reopen them inside implementation unless the user explicitly changes them.

| Decision | Required implementation meaning |
|---|---|
| Permanent reference solver | Boracle remains in the compiler after the production checker exists. It is not a disposable prototype. |
| Cargo feature | `boracle` controls whether the reference solver and its developer tooling are compiled. |
| Default checker | Enabling or disabling the feature must not silently change the normal compiler's borrow semantics. The current alpha checker remains authoritative until a later production-checker project replaces it. |
| Shared substrate | The normalized borrow-analysis input model and stable common result vocabulary are permanent shared infrastructure outside the `boracle` feature gate. Normal compilation must not construct or run them unless an explicit Boracle path requests them. |
| Reference and experiments | Reference mode follows accepted Moth semantics. Named experimental modes may investigate stronger proofs, but no experiment silently changes reference behaviour. |
| Solver style | Use clear Rust, explicit relations, graph traversals and deterministic sets/maps. Avoid Datalog engines, Datafrog and other external solver dependencies. |
| Performance | Boracle performance is irrelevant except that it must terminate on its bounded validation corpus. Do not optimise it at the cost of clarity. |
| Diagnostics | Boracle produces structured reasons and witness data. It does not create a parallel user-facing diagnostic renderer or taxonomy. |
| Last use | Last-use and future-use reasoning becomes a first-class modular analysis product, not a helper hidden inside mutable-access transfer. |
| Lifetime and REC boundary | Boracle does not assign lifetime owners, validate retained-edge topology, choose memory strategies or select REC. It may later host clearly marked experiments about facts those systems could consume. |
| Developer execution | A feature-gated internal developer command can run real Moth source through Boracle and dump its analysis. This is not a stable public CLI contract. |
| Validation command | `just boracle` owns deterministic Boracle validation. It is deliberately excluded from `just validate`, the standard feature matrix and ordinary CI gates unless the user later changes that policy. |
| Initial milestone | The plan stops after the usable reference facility is established. It does not implement or formally hand off to an optimised production solver. |

## Current repository baseline to re-check at activation

The current repository has a substantive but transitional alpha checker under:

```text
src/compiler_frontend/analysis/borrow_checker/
```

Its current shape includes:

- a forward per-function CFG fixed-point engine
- dense local indexes and bitset-backed root sets
- local states expressed as uninitialised, slot, alias or joined slot/alias states
- may/must future-use summaries
- shared and exclusive access conflict checks
- optional transfer advice that falls back to borrowing when proof is unavailable
- local, imported, generated and external call summaries
- conservative field, index, collection and map alias treatment
- reactive invalidation facts
- advisory drop candidates

The current checker already distinguishes `HirExpressionKind::Copy` from `Load`, but copy independence is mostly represented by the absence of propagated alias roots. The reference redesign must make the source read and independent result provenance separate positive facts.

The current progress matrix correctly marks borrow validation and local last-use analysis as partial. It names loop final-iteration facts, detached stored-result transfer, the complete affine-transfer contract and memory-plan-driven drop authority as gaps.

The current validation system also assumes every Cargo feature belongs to the standard feature matrix and native Clippy uses `--all-features`. Adding `boracle` without changing this policy would still compile it during normal validation. Phase 1 must add an explicit opt-in feature-lane class and exclude Boracle from the standard all-feature Clippy path.

Treat these facts as a navigation baseline only. Re-read the active worktree before editing because the borrow checker and validation infrastructure may have moved.

## Initial target architecture

During this project, the architecture is deliberately asymmetric:

```text
validated HIR
    |
    +-> current alpha borrow checker
    |       normal compiler authority
    |       existing behaviour preserved
    |
    +-> BorrowProblem builder
            explicit Boracle/developer/test path only
            |
            +-> Boracle reference solver
                    #[cfg(feature = "boracle")]
```

The future production solver is not part of this plan. It will eventually consume the same normalized problem or a proven evolution of it, but no production architecture is locked merely because Boracle uses one representation.

Recommended initial module shape:

```text
src/compiler_frontend/analysis/borrow_checker/
|-- mod.rs                         current alpha entry remains authoritative
|-- ...                            current alpha implementation
|-- problem/
|   |-- mod.rs                     structural map and public stage-local surface
|   |-- ids.rs                     dense typed IDs
|   |-- control_flow.rs            points, events, edges and ordering
|   |-- places.rs                  normalized places and overlap facts
|   |-- origins.rs                 value-origin and provenance events
|   |-- accesses.rs                access, alias and call-effect events
|   |-- builder.rs                 validated HIR to BorrowProblem extraction
|   |-- validation.rs              internal problem invariants
|   `-- tests/                     hand-authored and extraction tests
|-- last_use/
|   |-- mod.rs                     common vocabulary and ownership boundary
|   |-- alpha.rs                   behaviour-preserving current checker adapter if migrated
|   |-- facts.rs                   future-use and final-use outputs
|   `-- tests/
`-- boracle/                       #[cfg(feature = "boracle")]
    |-- mod.rs                     orchestration and ownership map
    |-- solver.rs                  explicit reference solve sequence
    |-- provenance.rs              origin-flow reference analysis
    |-- loans.rs                   loan issue, propagation, kill and liveness
    |-- last_use.rs                independent readable reference algorithm
    |-- conflicts.rs               place-overlap access legality
    |-- witnesses.rs               structured derivations and paths
    |-- experiments.rs             named non-canonical investigations
    |-- dump.rs                    deterministic developer output
    `-- tests/
```

Exact filenames may change when the active code makes a clearer split obvious. The ownership split may not change:

- normalized input and stable common facts are outside the feature gate
- Boracle-only solving, traces, experiments and dumps are feature-gated
- the alpha checker remains the default
- tests stay outside production implementation files
- `mod.rs` files remain structural maps rather than large implementation owners

## BorrowProblem contract

`BorrowProblem` is a normalized immutable analysis input over one validated function. It is not a second semantic IR and must not acquire backend or lifetime-planning authority.

It should contain enough explicit information that neither Boracle nor a future solver has to rediscover HIR meaning:

- dense function-local program points in semantic evaluation order
- basic blocks, predecessor/successor edges, entries and exits
- normalized places and projection paths
- fresh value-origin events
- shared alias events
- exclusive alias events
- copy events with separate source-read and independent-result facts
- assignment and rebinding events
- aggregate construction and stored-value events
- call argument access/effect facts
- call result provenance inputs
- return, error, break, continue and fallible control-flow events
- local visibility and region-scope metadata required to end bindings without assigning lifetime owners
- source and HIR identity needed for tests, dumps and later diagnostic witness reconstruction

### Program points

A statement ID alone is too coarse. Multiple argument reads, place-index expressions and result writes can happen within one statement. The builder must create points or ordered events fine-grained enough to preserve Moth evaluation order and same-statement conflicts.

The initial model should favour explicit event points over clever implicit ordering. Every event should have a deterministic stable order within its function and retain the HIR/source location that produced it.

### Places

`PlaceId` identifies an interned semantic place, not a lifetime owner or physical address.

The initial overlap policy should support:

- exact local places
- field projection paths
- conservative dynamic index overlap under one collection base
- conservative map and growable collection mutation at the receiver base
- explicit evidence when two places are provably disjoint

Borrow-place precision is separate from allocation-family splitting. Stage 6 may prove two fields non-overlapping while later lifetime analysis still keeps them in one allocation family.

### Value origins

`ValueOriginId` identifies one abstract source-semantic storage lineage or fresh-definition class. It must not mean physical allocation, heap object or lifetime region.

The model must distinguish:

- the binding currently naming a value
- the origin represented by that value
- a fresh redefinition of the same binding
- an alias preserving an older origin
- an independent copy creating a new origin
- a projection preserving a relationship to a base origin
- a join carrying several possible origins

Distinct fresh definition sites create distinct origins. Repeated execution of one definition site in a loop initially uses one abstract origin class plus explicit loop-carried flow. Boracle must investigate whether that abstraction needs a generation or epoch refinement before any production representation is chosen.

### Loans

`LoanId` identifies one shared or exclusive source-semantic access capability. A loan records:

- where it was issued
- its access kind
- the place and origins it covers
- which bindings or values can use it
- where it is used
- where it is killed or becomes unusable
- which points it is live at

A loan is not a runtime reference count, lifetime region or cleanup owner.

### Validation

Construct `BorrowProblem` atomically and validate it before either solver consumes it. Missing points, malformed edges, unknown places, impossible event order, out-of-range IDs or inconsistent source mappings are `CompilerError` invariants.

The builder must not repair malformed HIR. HIR validation still owns HIR coherence.

## Boracle result contract

Reference solving should return one structured report that can be inspected without parsing prose.

Conceptual contents:

```text
BoracleReport
    reference rule set identity
    problem identity
    origin/provenance facts
    loan issue and liveness facts
    last-use and future-use facts
    access decisions
    structured conflicts
    structured witness paths
    named experiment outcomes
```

The report should retain deterministic IDs and source/HIR links. Human dumps derive from this report. Tests should assert typed facts rather than parse display text unless the display format itself is under test.

A successful reference solve must not require building expensive diagnostic prose. On failure, structured witnesses should identify at least:

- the rejected access point
- the overlapping place or origin
- the conflicting loan
- the loan issue point
- one keeping-alive use or path reason
- the relevant source locations

The shared diagnostic layer may later turn those facts into user-facing diagnostics. Boracle does not create a competing diagnostic code or renderer.

## Reference and experimental modes

Boracle has one default reference mode and zero or more named experiments.

Reference mode:

- follows canonical accepted Moth semantics
- is deterministic
- is the only mode eligible to become a future production-solver oracle
- may become more precise only after the canonical design adopts the rule

Experimental mode:

- starts from the reference rules
- enables one or more named hypotheses through typed enums, not loose strings or boolean-heavy APIs
- records exactly which rule changed and why a result differs
- never changes reference snapshots
- never updates the progress matrix as implemented language support

Promotion procedure for an experimental rule:

1. State the semantic hypothesis and safety argument.
2. Add positive, negative and adversarial Boracle cases.
3. Search for counterexamples with bounded generated problems.
4. Review the change against Moth's canonical memory and language contracts.
5. Get explicit user approval for any semantic change.
6. Update the canonical documentation and reference mode in one accepted slice.
7. Keep the old experiment only when it still represents a useful alternative investigation.

## `copy` as the first model family

The first complete semantic family must make this split explicit:

```text
copy source
    source access: shared read of source origins
    result provenance: independent fresh origin
```

The result must share no mutable origin with the source. The source may still have live aliases. Copying an alias copies the value graph it observes rather than retaining that alias.

Initial cases must cover:

- source use after copy
- destination mutation while source aliases remain live
- copying through an alias
- copying a field projection
- copying before and after a source rebind
- destination rebind after copy
- copy in branches and joins
- copy in loops
- repeated internal aliases inside a copied aggregate
- the boundary between borrow-level independence and later group-only cycle legality

AST/type checking still owns whether a graph is copyable. Lifetime analysis later owns destination regions and the explicit-group requirement for copied cycles. Boracle owns the access and provenance facts between those stages.

## Modular last-use contract

Last-use analysis becomes a named reusable owner with clear inputs and outputs.

At minimum it answers, for an origin or loan at a program point:

```text
NoFutureUse
MayBeUsed
MustBeUsed
```

The reference implementation should also retain witnesses:

- one later use for `MustBeUsed`
- one use path and one no-use path for path-dependent `MayBeUsed`
- the explored exits for `NoFutureUse`

Consumers remain separate:

- borrow validation asks whether an alias is still active
- optional transfer asks whether every relevant path has no later source use
- later lifetime work asks for final observation and capable-source facts
- retained-edge frontier work may ask whether a surviving source can recreate an edge

Failure to prove optional transfer is never a source diagnostic. The operation remains a borrow.

The current alpha checker's may/must future-use implementation should be moved behind a dedicated last-use module only through a behaviour-preserving slice with exact regression coverage. Do not force the alpha checker to consume `BorrowProblem` in this project. Temporary coexistence is acceptable and must be documented as replacement debt rather than hidden through adapters.

## Feature, validation and CLI contract

### Cargo feature

Add:

```toml
[features]
boracle = []
```

Gate Boracle-only modules and tooling with `#[cfg(feature = "boracle")]`. Do not gate the shared problem model or common last-use/result vocabulary.

### Opt-in feature lane

The existing feature matrix assumes every feature belongs to a standard lane and native Clippy uses `--all-features`. Change the validation infrastructure to distinguish:

- standard lanes run by `just test-feature-matrix`
- opt-in developer lanes recognized by coverage checks but run only by their named command

`boracle` belongs only to the opt-in lane `just boracle`.

The feature-lane coverage report must still prove that:

- `boracle` is declared
- every `cfg(feature = "boracle")` refers to a declared feature
- the feature has an owned executing command
- the command is classified as opt-in and is not reported as having run during the standard matrix

Do not weaken coverage for ordinary features.

### Standard Clippy exclusion

Replace the normal `--all-features` Clippy invocation with an explicit standard feature set that excludes opt-in features. Keep full warning denial. `just boracle` runs the Boracle-enabled Clippy lane separately.

The validation guide and testing guide must explain this exception. The exception is for deliberately expensive developer systems, not a general way to evade feature coverage.

### `just boracle`

The command owns the complete deterministic Boracle gate. Once all phases are implemented it should run, in a stable order:

1. formatting check
2. Boracle-enabled Clippy with warnings denied
3. Boracle unit and subsystem tests
4. `BorrowProblem` validation and HIR extraction tests
5. the curated copy, origin, loan and last-use corpus
6. bounded deterministic generated-problem tests
7. future production-vs-Boracle differential tests when a production solver exists
8. one internal CLI smoke analysis over real Moth source

Unbounded fuzzing, large stress campaigns and exploratory experiment sweeps remain separate manual commands.

`just boracle` is not called by `just validate`, `test-feature-matrix` or standard CI gates.

### Internal developer command

Add a feature-gated internal command with behaviour equivalent to:

```text
moth boracle <path>
    --dump problem
    --dump origins
    --dump loans
    --dump last-use
    --dump witnesses
    --experiment <name>
```

Exact option spelling may follow the current CLI parser's clearest internal convention. The behavioural rules are fixed:

- absent from builds without `boracle`
- clearly marked internal and unstable
- does not replace normal `check`
- runs a compiler-owned service rather than assembling frontend stages in CLI code
- can stop after validated HIR and Boracle analysis
- can inspect a source case even when the alpha checker would reject the same borrow pattern
- prints deterministic output suitable for debugging and optional snapshots

Initial source mode may support synthetic single-file compilation first. Project/module graph support is an allowed later Boracle extension and must not inflate the initial milestone unless required by the initial corpus.

## Test strategy

Use several independent layers so the production solver and Boracle cannot share one hidden bug.

### Hand-authored BorrowProblem tests

These are the clearest semantic owner. Build small problems directly in test-only fixture code and assert exact origins, loans, liveness, last-use decisions and witnesses.

Production files must not gain test-only constructors. Fixture builders belong under the relevant test directory.

### HIR extraction tests

Assert that validated HIR becomes the intended points, places and events. These tests catch shared-input bugs that differential solver tests cannot find.

Prefer structural facts over exact numeric IDs unless deterministic ID assignment is the invariant.

### Source corpus

Use real Moth snippets where source semantics and lowering matter. Keep Boracle-specific source cases under a clear test owner and do not pretend the alpha checker's result defines the expected answer.

### Bounded generated problems

Generate small deterministic CFGs and event sequences without external fuzzing libraries. Compare solver invariants, experiment deltas and future production results. Record the seed and reduced problem on failure.

### Differential comparison

Before the production solver exists, comparisons against the alpha checker are investigative only:

- agreement increases confidence
- disagreement creates a reduced case and classification task
- the alpha checker is not an oracle
- Boracle is not automatically correct when it disagrees with canonical docs

After a production solver exists, reference-mode Boracle becomes the differential authority for the problem subset both support.

## Investigation programme

The initial implementation should make these questions easy to explore. It does not need to settle every question before the first milestone.

| Investigation | Initial status |
|---|---|
| `copy` source-read versus independent result provenance | Required reference behaviour |
| fresh rebinding while old aliases remain live | Required reference behaviour |
| multiple value origins for one binding across branches | Required reference behaviour |
| loop-carried origin generations | Required investigation before production origin representation is locked |
| dead unused exclusive aliases ending immediately | Named experiment until canonical semantics confirm it |
| path-separated conflicting accesses | Required reference behaviour where no path observes both |
| disjoint struct fields | Required reference place precision where type/operation semantics prove disjointness |
| dynamic collection indexes | Conservative reference baseline, stronger precision optional |
| map and growable collection mutation | Conservative receiver-base baseline |
| same-statement and multi-argument overlap | Required reference behaviour |
| aggregate-stored aliases | Required reference behaviour |
| fallible success/error paths and catches | Required reference behaviour |
| multi-return per-result provenance | Required investigation, no public summary migration forced by this plan |
| recursive call summary SCCs | Required investigation, not a production summary commitment |
| detached stored results from `remove` | Investigation only, final retained-edge semantics remain later work |
| loop final-iteration transfer facts | Investigation only unless a clear reference rule lands |
| reactive subscriptions | Must remain read-only observability metadata, not active loans |
| diagnostic witness quality | Required structured evidence, shared rendering deferred |
| production fast paths | Record opportunities only. Do not optimise Boracle. |

## Documentation, roadmap and matrix contract

This work is internal but changes the compiler's permanent analysis architecture. Documentation updates are required and must distinguish implemented tooling from deferred production semantics.

### At activation

Update `docs/roadmap/roadmap.md`:

- add the Boracle implementation plan under active implementation work when implementation starts
- describe it as isolated reference-solver research that does not reorder the queued implementation chain
- do not cite another plan as its authority
- keep the production borrow-checker replacement as deferred work with no implementation plan implied

### During implementation

Add and maintain:

- `docs/src/developer-docs/memory-management/borrow-validation/boracle-reference-solver.mtf` as the permanent subsystem authority
- a special developer task route in `docs/src/developer-docs/memory-management/overview.mtf`
- an optional developer reference in `borrow-validation/overview.mtf`
- Stage 6 and implementation-map wording in `docs/compiler-design-overview.md` once the shared model exists
- accurate alpha/Boracle ownership wording in `borrow-validation/borrow-validation.mtf`
- feature-lane rules in `docs/src/developer-docs/style-guide/testing.mtf`
- validation exclusions and `just boracle` in `docs/src/developer-docs/style-guide/validation.mtf`
- Boracle task routing in `AGENTS.md`
- `index.md` only when the new module paths need location updates

Do **not** add Boracle to the public memory-management `@page.moth` detailed-page list. It is a special developer authority, not a public language teaching page.

### Progress matrix

Update the existing **Borrow validation and local last-use analysis** row rather than adding a user-facing language feature row.

At the relevant phases, record:

- the alpha checker remains the default compiler authority
- Boracle exists only under the `boracle` developer feature
- the shared normalized problem and modular last-use substrate are implemented when they actually are
- Boracle coverage is deterministic and owned by `just boracle`
- the production advanced checker remains deliberately deferred
- lifetime-region inference, retained-edge analysis, REC integration and memory-plan-driven cleanup remain deliberately deferred
- experimental Boracle acceptances are not current compiler support

Do not mark borrow validation fully supported merely because Boracle can prove a case the alpha checker rejects.

### Initial milestone closeout

When the initial facility is accepted:

- remove this plan and its roadmap entry in the same completion commit, following roadmap policy
- retain the permanent design document
- retain a short roadmap note that the optimised production checker remains deferred pending Boracle findings
- retain any open investigations in the design document or a dedicated durable research record, not in a completed implementation plan
- do not create a formal handoff plan to the production solver

## Non-goals

This plan must not:

- replace the normal alpha checker
- change which Moth programs normal builds accept or reject
- implement an optimised production solver
- make Boracle a correctness fallback for compiler failures
- assign allocation-family lifetime owners
- validate retained-edge topology or cycles
- choose affine, region, group, REC or GC representations
- emit drops or mutate HIR
- expose source lifetime, reference or move syntax
- add a stable public solver-selection CLI
- add external solver or logic-programming dependencies
- optimise Boracle data structures for speed
- add Boracle to standard validation or CI
- treat comparison with the alpha checker as proof
- edit generated files under `docs/release/**` directly
- keep compatibility adapters after a new internal Boracle API replaces an earlier branch-local shape

## General agent rules

- Work only in the active Boracle worktree.
- Preserve unrelated local changes. Never reset, stash or overwrite them without explicit user instruction.
- Re-read current owners before every phase. Search before adding a type, pass, helper or fixture abstraction.
- Keep the current alpha checker behaviour stable. A discovered bug becomes a reduced Boracle case and documented finding unless a fix is strictly required by the active phase and separately approved.
- Use descriptive names. Prefer explicit structs and enums over tuples and booleans.
- Keep production files free of test-only APIs.
- Keep Boracle reference algorithms independent from current alpha algorithms where practical.
- Do not benchmark Boracle runtime. Measure only accidental default-build or default-checker regressions caused by shared infrastructure.
- Every phase ends with the mandatory audit, style review and validation procedure below.
- Update the plan's status block and phase checklist only with work actually completed and commands actually run.
- Do not claim a structured repository audit occurred unless the user explicitly invoked the audit framework.

## Mandatory phase completion procedure

Every phase has the same non-optional completion sequence.

1. **Implementation review**
   - Read every changed module from its entry point.
   - Re-check ownership, deleted paths, feature gating and default-checker isolation.
   - Search adjacent code for duplicate or stale logic.

2. **Independent phase audit**
   - Use a fresh read-only auditor agent where available.
   - Give it the phase goal, permanent design authority, active diff and validation evidence.
   - Require it to inspect correctness, architecture, missing tests, feature leakage, stale docs and over-complexity.
   - This is a plan-local implementation audit, not the registered structured audit framework unless explicitly requested.

3. **Correction cycle**
   - Resolve every required finding.
   - Re-run the focused audit until it reports no required findings.
   - Record deferred observations only when they are genuinely outside the phase and have a durable owner.

4. **Style-guide and Slice review**
   - Re-read the codebase style guide.
   - Apply the complete `AGENTS.md` Slice review.
   - Check file headers, module maps, naming, comments, vertical spacing, diagnostics lanes, test placement and absence of test-only production APIs.

5. **Validation**
   - Run the phase-specific commands listed below.
   - Report exact commands and results.
   - Do not substitute a broad old CI result for current local evidence.

6. **Documentation and status**
   - Update the permanent design document when an accepted contract changed.
   - Update the progress matrix only when implementation or coverage status changed.
   - Update the roadmap and this plan's current status.
   - Mark relevant audit-log rows stale when the implementation materially changed an area they record. Do not record new audit coverage outside an audit run.

### Minimum validation by change shape

| Change shape | Minimum phase validation |
|---|---|
| Documentation-only activation edits | documentation release build and changed-route inspection |
| `boracle`-gated implementation only | targeted tests, formatting check and `just boracle` |
| Shared always-compiled problem/result types | targeted default tests, `just boracle` and `just validate` |
| Current alpha last-use refactor | focused alpha borrow tests, `just boracle` and `just validate` |
| Feature-lane, Clippy or validation infrastructure | focused xtask tests, `just boracle` and `just validate` |
| CLI or compiler-service integration | focused CLI/service tests, source-mode Boracle smoke, `just boracle` and `just validate` |
| Initial milestone closeout | documentation release build, `just boracle` and `just validate` |

The phase can require more. It cannot require less than the row matching its widest change.

# Phase 0 - Activate, inventory and freeze the behavioural baseline

## Goal

Re-anchor the plan in the dedicated worktree, preserve all local work and build a precise inventory before adding new architecture.

## Tasks

- [x] Read the authority list from the active worktree.
- [x] Record the active branch, revision, worktree list and `git status --short` in working notes, not as a pinned pre-activation SHA in this committed plan.
- [x] Confirm the dedicated worktree does not contain unrelated implementation changes that would make audit attribution unclear.
- [x] Inventory every current borrow-checker file, public entry point, consumer and test owner.
- [x] Inventory current last-use construction and every consumer of `FutureUseKind`, optional transfer and advisory drop facts.
- [x] Inventory HIR place, value, statement, terminator, source-location and call-summary owners.
- [x] Inventory `show_borrow_checker`, compiler developer logging and current CLI extension points.
- [x] Inventory Cargo features, feature-matrix coverage, native Clippy and `just validate` ownership.
- [x] Run the current focused alpha borrow-checker tests and record the baseline result.
- [x] Add or identify reduced current cases for known `copy`, rebind, projection, loop and multi-return limitations without changing alpha behaviour.
- [x] Classify each case as accepted current behaviour, known alpha bug, expected conservatism or unsettled future rule.

Suggested searches:

```bash
rg -n 'FutureUseKind|classify_move_decision|OptionalTransferStatus|advisory_drop_sites' src
rg -n 'HirExpressionKind::Copy|direct_value_provenance|roots_for_place' src/compiler_frontend
rg -n 'show_borrow_checker|borrow_log' src Cargo.toml justfile xtask
rg -n 'Borrow validation and local last-use analysis' docs/src/docs/progress
```

## Required outputs

- active owner map in plan working notes
- baseline command/result record
- initial semantic-case inventory
- no normal compiler behaviour change

## Mandatory phase gate

Apply the full mandatory completion procedure.

Minimum validation:

```bash
cargo test -p moth --quiet borrow_checker -- --format terse
cargo run --quiet -- check docs --terse
```

If activation edits change only docs, use the documentation release-build gate before accepting the phase.

# Phase 1 - Establish the feature, opt-in lane and empty permanent seam

## Goal

Create a compilable feature-gated Boracle module and its dedicated validation command without running a solver or affecting normal compilation.

## Tasks

- [x] Add `boracle = []` to `Cargo.toml`.
- [x] Add an empty, documented `borrow_checker/boracle/mod.rs` behind the feature.
- [x] Add the shared `problem/` and `last_use/` module seams outside the feature gate with no normal runtime invocation.
- [x] Add an opt-in feature-lane class to `xtask/src/feature_matrix.rs` or the smallest clearer owner.
- [x] Keep standard feature coverage strict while recognizing `boracle -> just boracle` as opt-in.
- [x] Change normal Clippy from `--all-features` to an explicit standard feature set that excludes Boracle.
- [x] Add Boracle-enabled Clippy to `just boracle`.
- [x] Add a minimal Boracle compile/test smoke so the command proves the feature really executes.
- [x] Add machine-readable feature-lane coverage fields that distinguish standard and opt-in lanes.
- [x] Update testing and validation docs in the same slice.
- [x] Add source checks or unit tests proving standard matrix execution excludes the opt-in lane without weakening ordinary feature coverage.

The empty command may initially run only formatting, Boracle Clippy and the smoke test. Later phases extend it until it owns the complete deterministic suite.

## Required outputs

- declared `boracle` feature
- `just boracle` executes feature-gated code
- standard `just validate` and feature matrix do not compile or execute Boracle
- feature-lane coverage still rejects undeclared or uncovered standard features
- no solver selection or normal borrow change

## Mandatory phase gate

Apply the full mandatory completion procedure.

Minimum validation:

```bash
cargo test -p xtask --quiet feature_matrix -- --format terse
just boracle
just validate
```

# Phase 2 - Define BorrowProblem and hand-authored semantic fixtures

## Goal

Land the permanent normalized input vocabulary and prove its invariants without coupling it to HIR extraction yet.

## Tasks

- [x] Add dense typed IDs for points, places, origins, loans, uses and any other proven entity.
- [x] Define immutable CFG blocks, ordered events, edges, entries and exits.
- [x] Define normalized places and projection elements.
- [x] Define fresh, alias, exclusive-alias, copy, rebind, aggregate and call-effect events.
- [x] Keep source/HIR mapping optional only where a hand-authored fixture has no source owner.
- [x] Add `BorrowProblem::validate` or one equivalent atomic validator.
- [x] Add deterministic debug formatting that never relies on hash iteration order.
- [x] Add test-only fixture builders under `problem/tests/`.
- [x] Add direct problems for copy, old aliases across rebind, branches, joins, loops, field projections and same-statement access order.
- [x] Assert malformed problems fail through the internal compiler-error lane.
- [x] Keep Boracle orchestration as a stub that can accept and print a validated problem.

Prefer `BTreeMap` and `BTreeSet` in Boracle-facing fixture and debug structures where ordering clarity wins. Dense vectors are fine where the ID-to-row relationship is clearer.

## Required outputs

- stable problem vocabulary
- atomic invariant validation
- readable deterministic dumps
- direct semantic fixtures independent of HIR and the alpha checker

## Mandatory phase gate

Apply the full mandatory completion procedure.

Minimum validation:

```bash
cargo test -p moth --quiet borrow_problem -- --format terse
just boracle
just validate
```

# Phase 3 - Extract BorrowProblem from validated HIR

## Goal

Build one normalized problem from HIR without changing the current alpha checker or making Boracle a normal compilation stage.

## Tasks

- [ ] Define the exact point/event ordering for statements, expression reads, call arguments, result writes and terminators.
- [ ] Intern normalized places once per function.
- [ ] Preserve field projections while keeping indexes, maps and growable collections conservative where required.
- [ ] Emit fresh origins for literals, constructors, templates, calls and other fresh-producing operations.
- [ ] Emit separate source-read and result-origin events for `Copy`.
- [ ] Emit alias and rebind events from assignments according to HIR value meaning.
- [ ] Emit aggregate and stored-child relationships without treating construction as an implicit copy.
- [ ] Import call access/effect and preliminary result-alias facts from existing stable summaries.
- [ ] Preserve fallible, return, break, continue and match control flow.
- [ ] Record visibility/scope exits needed to stop local use without assigning memory lifetimes.
- [ ] Validate the completed problem before returning it.
- [ ] Add HIR extraction tests for every event family.
- [ ] Add source-to-HIR-to-problem tests for the initial Moth corpus.
- [ ] Prove normal compilation does not invoke the builder.

Do not let the builder infer facts from rendered names or source syntax. Consume validated HIR and explicit side tables.

## Required outputs

- one HIR-to-problem owner
- exact event ordering
- complete initial event coverage
- extraction tests independent from solver tests
- no default-path execution

## Mandatory phase gate

Apply the full mandatory completion procedure.

Minimum validation:

```bash
cargo test -p moth --quiet borrow_problem -- --format terse
cargo test -p moth --quiet hir -- --format terse
just boracle
just validate
```

# Phase 4 - Make last-use a modular reference analysis

## Goal

Create a first-class last-use/future-use contract and harden it with an independent readable Boracle algorithm.

## Tasks

- [ ] Define the shared last-use input and output vocabulary.
- [ ] Implement Boracle future-use analysis through explicit CFG traversal or fixed-point sets.
- [ ] Produce `NoFutureUse`, `MayBeUsed` and `MustBeUsed` with structured witnesses.
- [ ] Handle branches, joins, loops, breaks, continues, returns and fallible exits.
- [ ] Distinguish origin use, loan-holder use and mere binding visibility.
- [ ] Add direct cases for all-path final use, path-dependent use and loop-carried use.
- [ ] Add metamorphic tests such as inserting an unreachable use or deleting a final use.
- [ ] Move current alpha future-use vocabulary and helpers behind a dedicated `last_use` owner where this can be done without semantic change.
- [ ] Keep the alpha algorithm and Boracle algorithm independently implemented.
- [ ] Preserve exact alpha diagnostics and optional-transfer outcomes during the refactor.
- [ ] Document any temporary duplicated alpha/reference analysis clearly.

If moving the alpha implementation would introduce semantic change or broad risk, stop after extracting common vocabulary and record the remaining move as explicit alpha replacement debt. Do not force the current checker onto `BorrowProblem`.

## Required outputs

- modular last-use contract
- independent Boracle reference analysis
- witness-producing results
- hardened branch and loop coverage
- unchanged alpha behaviour

## Mandatory phase gate

Apply the full mandatory completion procedure.

Minimum validation:

```bash
cargo test -p moth --quiet borrow_checker_loop -- --format terse
cargo test -p moth --quiet borrow_checker_fact -- --format terse
cargo test -p moth --quiet last_use -- --format terse
just boracle
just validate
```

# Phase 5 - Solve value origins, rebinding and copy provenance

## Goal

Make Boracle explicitly answer which abstract origins each value can represent at each point.

## Tasks

- [ ] Implement fresh-origin introduction.
- [ ] Implement shared alias propagation without creating a fresh origin.
- [ ] Implement mutable alias propagation separately from binding mutability.
- [ ] Implement copy as a shared source read plus an independent result origin.
- [ ] Implement fresh rebinding so old aliases retain the old origin.
- [ ] Propagate origin sets through branches and joins.
- [ ] Handle projections and aggregate values without collapsing every value to its binding local.
- [ ] Handle call results from supplied summary facts.
- [ ] Add explicit trace output showing why a value has each possible origin.
- [ ] Add the complete initial copy corpus.
- [ ] Add rebind-generation cases, including loops.
- [ ] Introduce a named loop-generation experiment if definition-site origins lose necessary precision.
- [ ] Keep cycle placement and copyability checks outside Boracle's authority.

## Required outputs

- positive provenance facts rather than absence-based freshness
- binding-independent origins
- explicit copy independence
- reduced loop-generation findings
- deterministic provenance traces

## Mandatory phase gate

Apply the full mandatory completion procedure.

Minimum validation:

```bash
cargo test -p moth --quiet --features boracle boracle_provenance -- --format terse
cargo test -p moth --quiet --features boracle borrow_problem_copy -- --format terse
just boracle
```

Run `just validate` too if this phase changes shared default-path types or current alpha code.

# Phase 6 - Solve loan liveness and access conflicts

## Goal

Implement the readable reference legality solver over explicit loans, places, origins and program points.

## Tasks

- [ ] Issue shared and exclusive loans from normalized events.
- [ ] Track holders and future uses separately from lexical visibility.
- [ ] Implement explicit loan kills and unreachable-path handling.
- [ ] Compute live loans at every relevant point.
- [ ] Check shared access against live overlapping exclusive loans.
- [ ] Check exclusive access against every other live overlapping loan.
- [ ] Use normalized place overlap, not only base-local equality.
- [ ] Preserve conservative collection/map overlap.
- [ ] Respect same-statement event order and overlapping call arguments.
- [ ] Produce structured conflict witnesses.
- [ ] Add path-separated cases that avoid false conflicts.
- [ ] Add dead-exclusive-loan as a named experiment until canonical semantics adopt it.
- [ ] Add adversarial loop, join and rebind cases.
- [ ] Ensure optional transfer still consults last-use facts and falls back to borrowing.

## Required outputs

- complete initial reference legality solve
- point-local loan liveness
- projection-aware overlap
- structured conflict reasons
- no diagnostic prose dependency

## Mandatory phase gate

Apply the full mandatory completion procedure.

Minimum validation:

```bash
cargo test -p moth --quiet --features boracle boracle_loans -- --format terse
cargo test -p moth --quiet --features boracle boracle_conflicts -- --format terse
just boracle
```

Run `just validate` too if shared/default code changed.

# Phase 7 - Cover calls, results, aggregates, fallible flow and reactivity

## Goal

Extend the reference model across the difficult language boundaries needed for realistic Moth programs.

## Tasks

- [ ] Consume local, imported, generated and external access summaries.
- [ ] Project call-result aliases through arguments.
- [ ] Model fresh, alias and unknown preliminary result cases without pretending final lifetime provenance is solved.
- [ ] Investigate per-result provenance for multiple returns in Boracle-owned facts.
- [ ] Investigate recursive call-summary SCC solving without changing public interfaces prematurely.
- [ ] Model aggregate-stored aliases and same-origin repeated fields.
- [ ] Model map `get` as a temporary shared alias to the receiver base.
- [ ] Model `set`, insertion and aggregate construction under ordinary shared/copy/final-use rules.
- [ ] Model `remove` only to the extent borrow legality and preliminary detached-result provenance require.
- [ ] Preserve success and error control-flow separation.
- [ ] Treat reactive subscriptions as read-only observability metadata, not active loans.
- [ ] Block optional transfer of stable observable reactive roots where accepted semantics require it.
- [ ] Add source and hand-authored cases for every boundary.

Do not publish final retained-edge, detached-result, cardinality, outlives or cleanup-frontier summaries from Boracle in this phase.

## Required outputs

- realistic call and aggregate coverage
- fallible-path precision
- explicit research findings for multi-result and recursion
- correct reactive boundary
- no lifetime or REC ownership leakage

## Mandatory phase gate

Apply the full mandatory completion procedure.

Minimum validation:

```bash
cargo test -p moth --quiet borrow_checker_call_summary -- --format terse
cargo test -p moth --quiet --features boracle boracle_calls -- --format terse
cargo test -p moth --quiet --features boracle boracle_aggregates -- --format terse
cargo test -p moth --quiet --features boracle boracle_reactivity -- --format terse
just boracle
```

Run `just validate` if public/shared summary code changed.

# Phase 8 - Add developer source execution, dumps and experiment control

## Goal

Make Boracle practical for interactive compiler investigation without exposing a stable user feature.

## Tasks

- [ ] Add a feature-gated internal Boracle command.
- [ ] Route it through one compiler-owned analysis service.
- [ ] Support single-file source analysis at minimum.
- [ ] Allow Boracle-only analysis to reach validated HIR without requiring alpha-checker acceptance.
- [ ] Add deterministic dump selections for problem, origins, loans, last use, conflicts and witnesses.
- [ ] Add named experiment selection through typed configuration.
- [ ] Include the active rule-set and experiment names in every report.
- [ ] Keep default `check`, build and project compilation unchanged.
- [ ] Add CLI parser, service-boundary and output snapshot tests.
- [ ] Add one `just boracle` real-source smoke.
- [ ] Document the command as internal and unstable only in developer documentation.

Do not let CLI code tokenize, parse, lower or assemble Stage 6 itself. The compiler owns the service.

## Required outputs

- usable real-source investigation path
- deterministic structured dumps
- named experiment control
- no stable public CLI commitment
- no alpha fallback or default routing change

## Mandatory phase gate

Apply the full mandatory completion procedure.

Minimum validation:

```bash
cargo test -p moth --quiet --features boracle boracle_cli -- --format terse
cargo test -p moth --quiet --features boracle boracle_service -- --format terse
just boracle
just validate
```

# Phase 9 - Add generated problems and close the initial implementation milestone

## Goal

Prove the reference facility is durable, independently testable and ready for open-ended future investigation.

## Tasks

- [ ] Add a bounded deterministic problem generator using no external fuzzing dependency.
- [ ] Generate small acyclic and cyclic CFG shapes, branches, joins, loops, rebinds, copies and loan events.
- [ ] Check internal invariants and reference consistency properties.
- [ ] Compare with the alpha checker only on the explicitly shared supported subset and classify every disagreement.
- [ ] Add future differential hooks that can compare a production solver without redesigning the test harness.
- [ ] Ensure failing generated cases print a stable seed and reduced problem representation.
- [ ] Review all Boracle APIs for research extensibility without broad abstraction.
- [ ] Remove branch-local scaffolding, duplicate fixture paths and stale names.
- [ ] Complete the permanent design documentation and repository routing updates.
- [ ] Update the progress matrix accurately.
- [ ] Update the roadmap to leave production checker, lifetime topology and REC integration deferred.
- [ ] Run the final independent audit and correction cycle.
- [ ] Run both final gates.
- [ ] Remove this plan and its active roadmap entry in the completion commit once the user accepts the initial milestone.

## Initial milestone acceptance criteria

- `boracle` is a permanent opt-in Cargo feature.
- `just boracle` runs the complete deterministic Boracle suite and is excluded from normal validation.
- normal builds do not compile Boracle or run `BorrowProblem` extraction.
- the current alpha checker remains the default and its behaviour is unchanged except for separately approved fixes.
- `BorrowProblem` has validated points, places, origins, accesses, copy events and CFG facts.
- Boracle has independent readable provenance, loan-liveness, conflict and last-use algorithms.
- `copy` source access and independent result provenance are explicit.
- rebinding can preserve old origins through aliases.
- branches, joins and loops have reference coverage.
- structured witnesses exist for conflicts and failed final-use proof.
- reference and experimental rules cannot be confused.
- an internal command can inspect real Moth source.
- bounded generated problems protect the solver.
- permanent documentation explains the system and ongoing investigation programme.
- production checking, lifetime topology, REC and memory planning remain deferred.
- `just boracle` passes.
- `just validate` passes.
- the documentation release build passes.

## Mandatory phase gate

Apply the full mandatory completion procedure, then run:

```bash
just boracle
just validate
cargo run --quiet -- build docs --release
```

Inspect the generated documentation diff and confirm the Boracle design file is routed only as special developer material, not presented as a public language page.

## Durable state after this plan

After the initial plan is removed, Boracle remains open-ended infrastructure.

Future work may add:

- new semantic experiments
- reduced regressions from compiler development
- richer witnesses
- additional source-mode support
- project and module analysis
- production differential comparison
- optimisation-equivalence checks
- lifetime, frontier or REC-elision investigations clearly outside reference borrow legality

None of those require Boracle to become the production solver. The permanent design authority and tests, not this initial implementation plan, own its continuing life.
