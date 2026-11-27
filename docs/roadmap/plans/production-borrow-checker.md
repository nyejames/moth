# Production borrow checker and Boracle development

This is the living implementation companion for Moth's production borrow checker and the operational follow-ups uncovered by Boracle. It owns current implementation status, accepted production implementation choices, observed defects, performance evidence and the ordered queue of next work. Keep it after individual fixes land. Update it whenever new Boracle evidence changes an accepted production decision or identifies a production issue.

The accepted source language and memory semantics remain authoritative in `docs/developer-docs/memory-management/`, particularly `borrow-validation/borrow-validation.mtf`. `docs/compiler-design-overview.md` owns compiler stages and the solver-independent Stage 6 handoff. The permanent Boracle reference and operational-oracle documents own their respective semantics and limitations. This plan owns implementation strategy and live status, not a competing source-language contract or a second reference solver.

## Current handoff

**Status:** BC-06 production implementation is paused for a redesign investigation. The factored production cutover failed its performance and refusal guards and was abandoned (see [Factored cutover attempt](#factored-cutover-attempt)). Stage 5 shared-operand capture landed on the existing root-granular checker. A root-granular holder-edge candidate stays uncommitted: it fixes ordinary unsafe accepts but can't accept field replacement without also accepting its negative twin.
**Next action:** Implement slice 1 of the [BC-06 redesign](./production-borrow-checker-bc06-redesign.md#slice-1-sparse-allocation-layer), the sparse allocation layer, from `7cc05c61b`. The owner approved the design with the [Slice 0 decisions](./production-borrow-checker-bc06-redesign.md#slice-0-owner-decisions). Const C4 store work runs in parallel.

The October inventory below records `AUDITED_MAIN` at
`36156b40849eecad12e69b8d4e62388672a1ecff`, the starting point of
`borrow-checker-and-boracle-development`. Do not use it as a new timing
baseline. Research packages 3-6 still await explicit activation and a
fresh source/Boracle baseline. BC-06 does not activate them or add a
Boracle path to normal compilation.

## Verified implementation inventory

This inventory records the alpha checker at `AUDITED_MAIN`, with later changes called out where relevant. It does not claim the final Stage 6 or lifetime handoff.

| Area | Confirmed implementation | Important boundary or gap |
| --- | --- | --- |
| Production entry | `borrow_checker/mod.rs::check_borrows` creates `BorrowChecker`. Normal module compilation and generated sidecars use this alpha checker. | `problem` and `last_use` are compiled only for `boracle`/tests and `boracle` itself is behind the Cargo feature. There is no default reference-solver fallback. |
| Driver and convergence | `engine.rs::run` establishes local public call summaries, analyses every function, converges parameter mutation effects and publishes a `BorrowCheckReport`. Per-function reachable blocks use a forward worklist fixed point. | This still analyses complete module functions. It does not select only changed functions after private-failure rewrites. |
| State and overlap | At `AUDITED_MAIN`, `state.rs` tracked `UNINIT`, `SLOT` and `ALIAS` modes plus value-root and direct-alias sets per local. Current state (the 2026-11 `bug-fixes` correction) adds two more per-local sets: `write_target_roots`, the storage a write through the binding reaches, and `derived_from_roots`, a must-set of alias bindings the value derives from, intersected at joins and carried through jump arguments. A projected mutable alias targets the union of its holder's designated cells and the containing allocation roots observed when the alias formed. Each `RootSet` is an optional copy-on-write `Rc<[u64]>` sized to the local count; an empty set allocates nothing. | Production overlap is based on local storage roots, with conservative projections and joins. It does not consume Boracle's typed `OriginRelations`. Accepted Alpha precision limitation: the holder-cell half of a projected alias target exists because a freshly built allocation has no root that survives joins, alias chains, calls and scope exits. Reading a field of a rebound holder while such an alias is live is therefore rejected with `MOTH-BORROW-0003` even when the allocations are independent. See "Projected alias write targets keep the holder cell as a conservative identity" in `docs/bug-reports.md`. |
| Local future use | `metadata.rs` builds visible-local masks, may/must future-use and future-assignment facts. Transfer checks ordered accesses and narrows branch-local visibility. | This is alpha local-use reasoning, not a complete origin/loan/event-sensitive production solver or loop-generation proof. |
| Mutations and calls | `transfer/access.rs` checks immutable/uninitialised updates, reads, mutable access, aliases, map and collection operations. `transfer/call_semantics.rs` resolves local, imported, generated, private and external call contracts. | Unknown provenance and dynamic storage overlap remain conservative. |
| Return summaries | `metadata.rs` stabilises local return-alias summaries, `return_alias.rs` classifies result provenance, and recursion cycles are classified `Unknown`. Public return alias is one coarse `Fresh`, `AliasParams` or `Unknown` result classification. | Multiple returned values can be unioned. Fine-grained per-result, projection, detached storage and complete retention/outlives effects are not yet delivered. |
| Borrow facts | `types.rs` contains per-statement, terminator and value facts, local function summaries, optional `Borrow`/`Transfer` decisions and advisory drop-site lists. Test/debug-only state snapshots are separately gated. | Advisory drop sites are not validated memory-plan cleanup or runtime ownership semantics. Missing optional transfer proof falls back to borrowing. |
| Generated convergence | `module_compilation/generated/convergence.rs` rechecks generated sidecars when their installed summaries change, and `refresh_private_failure_lanes` reruns `check_borrows` on the rewritten base `HirModule` when installation changes it. | The base refresh currently reruns the entire module. `Unchanged` already avoids the refresh. Preserve that fast path. |
| Boracle | Feature-gated source-to-`BorrowProblem` extraction, independent reference provenance/loans/last-use, named experiments, bounded operational oracle, differential comparisons, deterministic generator and reducer are present. | Reference research and oracle execution do not determine default acceptance, lifetime topology or physical memory strategies. |

The current worklist uses `join_in_place` for an existing successor state, reusing root-set allocations. This existing optimisation must not be described as missing. The remaining expensive copies are the block-input clone, the new successor-input clone and any repeated retained state costs. The implementation also performs a complete output-state comparison before propagating.

The alpha liveness decisions use point-specific CFG future-use facts and reachable write-through assignments, not an arbitrary cross-block traversal order. When every path redefines an alias before another read it can stop blocking mutation, including across a loop back-edge. Advisory exit candidates are recorded for reachable returns, breaks and region-exit edges. An infinite continuation does not invent a cleanup exit.

### Verified coverage and gaps

The production test owners include `borrow_checker_fact_tests.rs`, `borrow_checker_call_summary_tests.rs`, `borrow_checker_loop_tests.rs`, `borrow_checker_update_tests.rs`, `borrow_checker_scope_tests.rs`, `borrow_checker_jump_tests.rs`, `borrow_checker_drop_site_tests.rs` and `state_tests.rs`. Their checked cases cover same-statement access order, shared/mutable conflicts, rebindings and scope kills, loop back edges, path-dependent optional transfer, aggregate/map storage, fallible flow, fresh/aliased results and rejected uninitialised or immutable updates. The production `state_tests.rs` exercise in-place joins and visibility narrowing directly.

Boracle has its own tested normalised problem, reference solver, operational oracle, differential and reduction lanes. `boracle/tests/differential.rs` deliberately pins a known shared-alias disagreement. The October inventory inspected these sources without running new checks. The later BC-06 baseline records fresh measurements and targeted Boracle results, not post-cutover validation.

Production borrow access checks are implemented and mandatory. The full accepted lifetime-region and escape solver, retained-edge/frontier proofs, REC and validated physical memory plans are separate undelivered analyses. Stage 6 must supply stronger solver-independent facts before those consumers can rely on them. Existing alpha drop-site emission is not evidence of a complete collector-free ownership implementation.

## Accepted production engineering decisions

These constraints are already established by compiler/memory authorities and existing code, not speculative Boracle promotions:

- Source access checking runs on all selected backends. A GC-backed output does not weaken source legality.
- Borrow validation consumes validated HIR and publishes immutable side facts. It does not rewrite HIR, repair malformed HIR, choose allocation owners or emit physical cleanup.
- Distinguish authored binding definitions from updates, preserve exact access/evaluation order and retain success/error edge availability. A static expression ID or separate result slot is not a fresh runtime generation proof.
- Distinguish slot storage, aliases and value origins. Preserve independent `copy` results and old-alias behaviour after a fresh rebind. Missing call or projection provenance stays unknown rather than becoming independent by default.
- Path-dependent or unproven **optional** transfer remains a borrow. It is not a source error. No optimisation is allowed to introduce mandatory move semantics implicitly.
- Exported summaries use stable semantic identities. Source/private/generated summaries converge inside the compiler-owned module transaction, independently of build scheduling. Diagnostics remain deterministic.
- Keep the reference solver inspectable and independent of the optimised production algorithm. The bounded oracle can demonstrate a concrete counterexample. Bounded safe executions alone cannot prove unbounded source acceptance.
- Production changes require a matching correctness/precision test, deterministic negative coverage and measurements when motivated by performance. Do not copy reference-solver maps/sets into production merely because they are convenient for Boracle.

**Implementation decisions still open:** compact root/state representation, clone elimination or ownership reuse, selective failure-lane invalidation, the exact iterative SCC implementation, production conflict-directed refinement and richer call/loop/aggregate proof domains. These are candidates or research directions until their evidence and invariants are reviewed. Existing code and accepted semantics are not approval of any one proposed data structure.

### BC-06: Relational production provenance

**Status:** The owner approved a committed baseline, test-only proofs and the field-precise direction on `docs/const-foundation-profile-independent-semantics`. The slice 3 cutover was attempted and abandoned. The owner then chose the narrow route: keep the root-granular checker, land the Stage 5 capture and defer projection tracking to a later, more comprehensive design. That design is now under investigation in the [BC-06 redesign companion](./production-borrow-checker-bc06-redesign.md), which owns the witnesses, evidence and resume point.

**Goal:** Make Stage 6 distinguish binding cells, allocation identity, projections and permission in one production provenance model. Preserve which relationships can coexist through control-flow joins and ordinary calls. Keep the existing validated-HIR input, diagnostics and read-only borrow handoff. Do not add a production Boracle path, activate research experiments, change access timing, implement lifetime topology or change physical memory planning.

#### Evidence and acceptance controls

The original C4 blockers are unsoundness cases. After receiving `Box(items), items` from a const result, a mutation through the writable `items` binding must conflict with a live observer of the same allocation through `box.items`. It must still conflict after ordinary `retain |items {I32}| -> Box: return Box(items);`, whole-value forwarding or a runtime join that retires the branch's local names but not the shared children. A fresh box is not a fresh collection.

The prototype adds pairwise view links beside the local root sets and groups return links by exit. It accepts a direct `select` call but rejects the forwarded shape after it flattens those groups in `collect_call_results`. The original source probe put `selected.left` in a new writable box and mutated that retained referent. Baseline acceptance is not proof that this mutation is legal: it can exercise the independent read-only permission bypass. The following positive control uses an explicit graph copy. The spelling and diagnostic observations refer to the Alpha compilers used in this investigation.

```moth
Pair = |left {I32}, right {I32}|

select |flag Bool, a {I32}, b {I32}| -> Pair:
    if flag:
        return Pair(a, {2})
    ;
    return Pair({3}, b)
;

forward |flag Bool, a {I32}, b {I32}| -> Pair:
    return select(flag, a, b)
;

probe |flag Bool| -> String:
    items = {1}
    selected = forward(flag, items, items)
    owned ~= copy selected
    observed = owned.right
    ~owned.left.push(4)
    return [:left=[owned.left.length()] right=[observed.length()] original=[items.length()]]
;

~[:[probe(true)] / [probe(false)]]
```

Both executions must render `left=2 right=1 original=1`. Both compilers checked for this investigation reject at the mutation with `MOTH-BORROW-0007`. Replacing the first return with `Pair(a, b)` must instead reject: the copy preserves sharing *within* the graph, and `observed` stays live across the mutation. Replacing the `select` exits with one ordinary CFG join and one return must make the same distinction. This tests projection precision as well as summary correlation: both compilers also reject the independent `Pair({1}, {2})` with the same observer.

`runtime_call_disjoint_result_fields`, `runtime_call_fresh_box_forward_rejected`
and `runtime_call_identity_fields_rejected` encode these call-result
requirements and fail on the existing checker: it rejects the disjoint
fields and accepts both forwarded observers. Keep mutation through the
immutable input only as a permission negative. Audit other positive
fixtures that obtain access through shared parameters before keeping
their expected results.

| Ordinary source witness | Required result | Observed: retained pre-C4 CLI / linked prototype |
| --- | --- | --- |
| Start with `holder ~= Pair({1}, {0})`, capture `writer ~= holder.left`, replace `holder` with a fresh pair, read its new child and then use `writer` | Old projection stays valid and does not overlap the replacement | Both reject the replacement read with `MOTH-BORROW-0003` |
| Capture `writer ~= holder.left`, replace `holder` with `Pair({3}, writer)`, mutate `writer`, read `holder.right` later | Reject overlapping mutation through the original writable projection | Accepted / `MOTH-BORROW-0007` |
| Under a runtime Bool, `writer ~= outer` and `~writer.push(child)`; leave the branch, mutate the original `child`, then observe the stored child | Reject. The helper binding dies, not the retained relationship | Accepted / `MOTH-BORROW-0007` |
| Same branch, but store `copy child` | Accept. Both calls render `21`: source length `2`, stored or fallback length `1` | Both accept |
| `outer` holds `child`, `set` fails, its handler mutates `child` before observing the old entry | Reject. Failure did not replace the old edge | Accepted / `MOTH-BORROW-0007` |
| Put immutable `input` in writable `Box(input)`, mutate `~box.items.push(2)` | Reject for permission, even if `input` has no later use | Both accept |
| Replace the field with a fresh collection, or construct `Box(copy input)` and mutate the copy | Accept. The original input stays length `1` | Both accept |
| Update `~view, extra = pair(3)` through a scalar alias with a fresh computed incoming result | Accept and write through. All incoming results are captured first | `MOTH-BORROW-0003` naming an HIR temporary / accepted |

Node execution of the linked output confirmed `2, 1` for the copied store,
`3` for each permission control and `34` for the scalar multi-bind. Reading
the sibling directly after the push, without the separate live observer,
isolates the graph behaviour: the disjoint copy renders right length `1` on
both paths, while the copy of `Pair(a, b)` renders right length `2` on the
shared path. That direct read after the write does not test exclusivity.

The existing-target multi-bind and immutable-parameter defects remain ordinary runtime issues. Retain their reports until regressions pass on the production cutover. Fix a shared transfer or HIR producer defect at its owner, not by exempting multi-bind, temporaries or const graphs from access checks. Also check repeated result slots under ordinary `return`, `Invoke`, recovery and forwarded calls using one independently writable source and a live observer.

#### Proposed representation

Replace the four provenance-related local root sets and the prototype's extra pair graph with **interned alternatives of sparse allocation and access state** in the existing Stage 6 worklist. An alternative is an abstract store at one event, not a retained execution trace. Share identical states and project away dead observations. Start without a separate conflict solver, general predicates or a new framework for symbolic logic.

- **Bindings and access:** A live alternative gives a binding its current slot or resolved alias target. A whole-binding alias designates a cell; a projected alias designates a selector on the captured parent allocation, never the current contents of the base binding. Keep alias derivation and the permission granted by that access separate from allocation identity. A fresh result binding receives a value, not a callee cell.
- **Allocations and edges:** A value names an abstract allocation or a typed unknown. A typed selector names a cell in a parent allocation. A retained edge holds a target and its access permission. Replacing an edge requires access to its cell, not authority to mutate the old child. Mutating through an edge requires authority through that path as well as the usual loan check. Other paths can refer to the same child with different permissions. Storing a shared input, copying a binding's metadata or transferring optional responsibility never creates mutation permission. A fresh owned graph and an explicit graph copy have new nodes with independent permissions and preserve only their internal sharing.
- **Overlap and liveness:** Resolve the accessed cell, value or storage domain and the keeping-use footprint in the *same* compatible alternative. Exact field cells can be disjoint even if they contain references to the same child. Whole-aggregate reads and writes account for the paths they may observe or change. Unknown collection elements or map values overlap conservatively. Do not infer disjointness from a missing relation. Scope exits retire the names and holders exiting scope, not allocations reachable through surviving values, projections or loans.
- **Transfers:** Capture RHS, jump inputs and call arguments before ordered writes or outcome selection. Consume HIR's `Define`/`Update` and authored evaluation order. Split alternatives only where an ambiguous destination or effect changes the answer. Strongly replace an edge only for an exact target. Builtin success can add, replace or detach an edge. Failed operations preserve their pre-commit edges unless their contract says otherwise. Use resolved builtin identities, not spellings.
- **Stored observations:** A live `get` loan covers its collection or map receiver, even if the observed child and written entry would be disjoint. `remove` carries detached child provenance, not that loan. A retained shared edge can keep an observation live. An internal shared edge of an owned copy records identity but does not issue a loan by itself.

Keep the lexical and point-based use indexes as filters. For shared and
observed exclusive loans, use a backward fixed point over the same state
transitions to match each access to a compatible keeping use of the same
occurrence. Keep holder and target correspondence across canonical
renaming. Write-through uses a loan, and replacement of one holder does
not retire a surviving derived holder. Test incompatible and redefined
occurrences against a surviving one. Keep the existing conservative rule
for unobserved exclusive loans. Missing optional transfer proof means borrow.

Stage 6 depends on correct input order. The baseline below reproduces late
capture in `hir/hir_expression/calls.rs`. Review the HIR producer correction
before cutover. Test whole and projected copies, projected-place and
nested-field operands and same-call conflicts. Do not reorder operands
after access validation or introduce new exclusive reservation rules.

**Why not flat or pairwise facts?** Root unions lose both paths and binding roles. Pairwise may-share links suffice for some local queries but not for substitution, composed stores or liveness: a result can alias `a` on one exit and a different result can alias `b` on another. Substituting the same actual for both formals makes the fields appear to overlap if the links were combined. Taking transitive closure of such a union is also unsound as evidence of coexistence. Pairwise coexistence between *links* only delays the same problem when transfer needs three or more compatible facts. The state alternative keeps the equality, edge, permission and loan facts together. A demand-driven slice is a cost optimisation to consider if the measured representation needs it, not an alternate acceptance path or a second solver.

Use a three-way control in the model tests: `{A,B} | {A,C} | {B,C}` and
`{A,B,C}` expose the same individual facts and pairwise coexistence.
Only the second satisfies an operation that needs all three. Exercise this
distinction in transfer and summary composition, not just in a container's set API.

#### Native summaries and convergence

Summarise each reachable semantic completion as alternatives over a finite boundary vocabulary. Each alternative carries:

1. A conjunction of equality or disjointness conditions on typed formal pre-state locations. Split only on facts that affect transfer, not arbitrary Bool expressions. The formal and retained path determine access, not a new source precondition.
2. The jointly visible post-call graph for ordered success slots or the separate failure payload. Leaves refer to input snapshots, canonical fresh graph nodes shared by result paths or explicit unknown storage. Include changed input edges and conservative invalidation footprints under the **same** condition and outcome. These are borrow facts, not final lifetime effects.

At a call, capture actuals in source order. Substitute them into each
callee alternative and conjoin its conditions with the caller's. Normalise
exact equalities and reject contradictions such as `a = b`, `b = c`,
`a ≠ c`. Discharge only proved conditions. Unresolved conditions travel
through resulting states and forwarded summaries. They do not create
exact identities for residual nodes. A writable actual cannot upgrade the
access of a shared formal. Test a copied pair through a forwarder with both
same and distinct actuals.

Instantiate each fresh node once per compatible alternative. Resolve
snapshots against the captured pre-state, then apply one post-state and
its result bindings. Never cross-product slots from different outcomes.
An `Invoke` retires both old destination sets before defining only the
selected channel. A changed input effect must propagate even when the
pre-terminator state and result values match. Keep external ABI policy.

A whole input reference is a typed subtree anchor, not a list of the
donor's observed children. Consumers can project through that anchor using
their actual graph. Distinguish an unchanged edge, a removed edge and an
unknown edge in a post-call effect. Resolve a read of an overwritten input
path against the post-state if it follows the write. A snapshot captured
before that write instead names the pre-state. Include both in the call
composition tests.

Preserve mutation and access effects separately from final graph equality.
A call can replace an edge and restore it, or write before failure or
divergence. If symbolic destinations resolve to one cell, apply one final
value consistent with executed order. Test a zero-result store with a
later observer and a store that commits before recoverable failure, each
paired with copy. A failed `set` does not undo an earlier committed write.

Join canonical alternatives by finite inclusion or a proved containment
order. Distinguish an unsolved dependency, no discovered completion, a
completion without sharing and explicit unknown. A bottom seed cannot
skip argument checking or turn an ordinary call into Never. Preserve
checking of ordinary HIR continuations under conservative provenance
when the final summary cannot describe a completion. Only declared
`-> !` removes the continuation.

Use local callee-first SCCs inside the existing module and sidecar queue.
Converge failure routing and install private failure lanes in the compiler
transaction. Preliminary provenance cannot diagnose a borrow conflict
or publish a success report. On HIR rewrite, regenerate affected CFG/call
facts and restart affected graph summaries and callers from bottom. Do
not apply a widening check across different executable graphs. Keep
stable independent effects and published artefacts immutable. After
relational convergence, check final loans and publish only reports paired
with final HIR. Keep constraint finalisation after the last summary rebuild.
Test a mixed base/sidecar dependency cycle in the model and an authored
generated caller whose private catch changes provenance. Do not enable
recursive generic instantiation.

Use stable slot indices, canonical type and variant selectors and
canonical abstract node numbering in `PublicCallSummary`. Validate
selector types, arity, references, access and ordering before publication.
No donor-local IDs, source spans or profile fields. Reuse canonical
interface comparison and sidecar compatibility. Change a provider from
two equal independent children to one shared child: check interface
inequality and the unchanged consumer's new verdict in fresh builds.
Separately prove real sidecar reuse within one build boundary. Do not
require a cross-build cache or call a cold compile a cache hit.

#### Complexity, termination and failure policy

Build a finite observation vocabulary from validated HIR, its relevant locals and selectors, visible callable boundary types and resolved operation footprints. Keep flat field/index classes only where HIR proves their semantics. Represent recursive or untracked paths by a typed residual domain, not unbounded concatenated paths. At recurrences distinguish a fresh definition from older live values. Discard unobservable generations and canonicalize the remaining part of each graph. Use a finite conservative summary class for unbounded older instances, never claim fresh disjointness solely from a static site or a loop back edge. Prove that projection, abstraction and summary composition preserve every represented possibility and cannot turn a denied or unknown access into a permitted one.

Use canonical first occurrence from ordered live bindings, outstanding
loan targets and boundary roots to rename graph nodes after projection.
Exact references can establish equality inside one alternative. An opaque
summary node stands for a set of possible locations, not one equality
class, even when two residual paths reach the same abstract node. The proof
slice must fix the vocabulary before iteration, specify how selector
composition saturates it and show that an added fact cannot narrow a
successor transfer or an exported outcome.

Even sparse alternatives can grow exponentially. Intern selected states,
reuse the single-state case and measure whether projection and pruning
make the common path cheap. Bound selectors, guard atoms, nodes, edges,
loan transitions and surviving alternatives. Bound attempted composition,
normalisation and liveness work across requeues and lane rewrites,
including discarded attempts. Test limits with an append loop and a
duplicate-heavy caller/callee product. Derive numerical limits from the
baseline. Do not depend on a wall clock, iteration order or build profile.

Use sound deterministic residual abstraction before hitting those limits.
Keep unknown disjointness distinct from a conflict on a consistent state.
No partial publication, invented conflict, silent acceptance or indiscriminate
union on exhaustion. The proof slice can refuse outside production.
Before cutover, prove the required precision within those limits and settle
any necessary source-facing exhaustion policy under the canonical diagnostics
contract.

#### Independently reviewable slices

1. **Freeze evidence and baseline.** Reproduce the matrix on the committed
   compiler. Verify HIR operand capture, incoming values, scope exits and
   failure edges. Isolate any HIR capture correction as its own reviewed
   prerequisite. Run `just boracle` and replay supported normalized input.
   Record disagreement and refusals. Capture matchable Stage 6 measurements.
2. **Prove the bounded domain outside production.** Exercise allocation
   graphs, recency, compatible loan uses, permissions, closure under call
   substitution and local/generated convergence. Compare small states
   against concrete expectations. Test commutative joins, deterministic
   publication and the exhausted/just-under limits. Compare measured sparse
   alternatives against factored or relevance-projected storage of the same
   semantics. Return for review with exact resource bounds, where precision
   can be lost and the proposed exhaustion policy. This is a stop before
   any production rewrite, not permission to ship a partial checker.
3. **Cut over in one vertical change after approval.** Wire one producer
   through `engine.rs`, `Call`/`Invoke`/`NeverCall`, `PublicCallSummary`,
   interface validation, frozen callable/receiver/private evidence in
   `preparation_freeze.rs` and generated convergence. Adapt
   `problem/call_effects.rs` at the existing Boracle input boundary
   conservatively. Remove the superseded local roots and captured-lender
   transport, coarse native return walker, pairwise prototype and any
   proof adapters. Retain ordered HIR/CFG inputs, future-use filters,
   diagnostics, optional transfer handoff and closed external policies.
4. **Validate access and composition.** Run the affected Rust borrow,
   call-summary, generated convergence, package materialisation and
   profile-parity tests. Use `moth tests --case <id> --terse` for exact
   output and diagnostic-site pairs: sibling and shared-child mutations,
   replacement, copy, permission, scope/rebind, loop re-entry, multi-bind,
   ordered results and late private catch. Exercise direct forwarding,
   one-return joins and a mutually recursive `select`/`relay` pair with
   and without coexistent sharing. Test alternating `Invoke` outcomes
   across loop iterations. Check source-package and generated publication,
   interface equality and within-boundary reuse. Run `just boracle` after
   the adapter migration and `just validate` on the integrated runtime.
5. **Measure and close.** Match release compiler flags and workloads.
   Measure median check time, peak RSS, and attempted and retained work
   in `frontend.borrow.initial`, `frontend.borrow.converge` and
   `frontend.generated.borrow_recheck`. Include ordinary projects,
   no-alias controls, branches, shared graphs, long call chains, recursive
   SCCs, sibling loops and private-failure convergence. Set the
   performance guard from the baseline before cutover. Review deletion,
   data ownership and any lost precision. Resume C4 only after the
   correctness, precision and performance gates pass. Reserve
   `just validate-full` for final merge readiness.

Boracle informs the design, not the acceptance decision. Its typed origin relations separate identity, containment, copy and explicit unknown. Its operational oracle uses concrete generations, slot/alias transitions and directed edges, but bounded executions do not prove loop safety. The existing reference unions some possible origins; its proposed relational refinement, generations and richer call/store summaries are research, not production implementations. The known terminal shared-alias disagreement and unsupported unknown call/collection cases limit differential evidence. Keep the oracle independent and leave research activation, new last-use rules, deferred exclusive call access, richer index predicates and downstream lifetime work out of this checkpoint.

#### Committed baseline

The release compiler came from a clean `0f384a5539` archive, not the linked
pairwise prototype. It reproduced four unexpected accepts (immutable
parameter stored in a writable aggregate, a retained child across lexical
exit, a zero-result store and a store before failure) and four unexpected
rejections (disjoint copied siblings with a live observer, field
replacement, a rebound projected alias and scalar multi-bind). The
coexistent copied sibling correctly rejects at the push. Direct reads
after a write confirm graph identity, not exclusivity. Copied stores
returned `21` on success and before failure, with a missing-entry
fallback that would return `23`. An out-of-range `set` left the
original entry and collection length unchanged
(`old=1 incoming=2 size=1`). The numeric fixture confirms a committed scalar
write, not retained-edge validation.

The HIR order reproducer uses `holder ~= Box({1, 2})` and a `replace(~holder)`
call that rebinds it to `Box({4, 5, 6})`. A positional shared operand
`holder.items` or `outer.inner.items` followed by that call read the new
length `3`, not the captured length `2`. `inspect(copy holder, replace(~holder))`
also read `3`. The emitted JS calls `replace` before cloning the whole
holder. Direct and nested **projected copy** controls read the old length
as expected. Keep the HIR prerequisite separate from Stage 6. A producer
fix needs to verify the borrow verdicts around earlier shared occurrences,
not preserve verdicts from late-evaluation HIR.

`just boracle` passed on the same archive. Default `boracle --dump differential`
reported canonical reference/operational agreement and `CompleteSafe` for
the local shared-alias, Boolean reassignment and checked integer
reassignment probes (one, one and two completed executions). The same
gate covers the normalized unknown-call refusal and the known terminal
shared-alias disagreement. The successful numeric probe does not replay
the historical input from BR-06. None of these bounded runs proves
production legality or enables experiments.

A release `moth-cli` build with `detailed_timers,benchmark_counters`, Rust
1.99.0 and an Apple M1 Pro measured 18 workload inputs. The table shows
selected medians in milliseconds from five samples after one warm-up and
the maximum process RSS in MiB. A separate `MOTH_TIMERS=bench` replay
collected counters. Worklist counters describe final reports, not all
attempted work during convergence. A dash means no emitted recheck timing.

| Workload | Check | Borrow initial | Borrow converge | Generated borrow recheck | Peak RSS |
| --- | ---: | ---: | ---: | ---: | ---: |
| Alias receiver chain 1024 | 15.18 | 3.36 | 3.56 | — | 23.4 |
| Origin fanout 512 | 10.62 | 2.42 | 2.45 | — | 18.6 |
| Copy chain 512 | 5.28 | 0.59 | 0.64 | — | 16.2 |
| Narrow chains 128 | 19.72 | 1.32 | 13.15 | — | 59.5 |
| Narrow chains 512 | 184.55 | 11.64 | 160.91 | — | 837.3 |
| Generic failure chain 128 | 14.38 | 1.08 | 7.37 | <0.01 | 47.3 |
| Sibling loops 64 | 33.89 | 15.99 | 13.30 | — | 108.9 |
| Branches 64, borrow-neutral | 4.45 | 0.76 | 0.63 | — | 18.4 |

The largest narrow-chain case dominates the absolute RSS and convergence
figures. The borrow-neutral branch series is not an exponential
alternative stress test. These numbers establish a comparison baseline,
not a production performance guard. Local `tmp/bc06/baseline_measure.json`
retains the input fingerprints and raw samples. The original replay
included a worktree-supplied re-entry fixture so its result describes the
archived binary, not the fixture corpus at that revision.

#### Test-only proof and cutover decision

The standalone Rust witnesses under local `tmp/bc06/proof/` exercise
exact allocation and binding cells, captured fields, distinct repeated
allocations, retained edges, permission attenuation and copies. An access
and its keeping holder must coexist in one alternative. Call witnesses
also preserve formal permissions separately from equality, reject
contradictory guards after substitution, refuse to copy unresolved
identity, and forward ordered results and writes by outcome. A mixed
two-node dependency cycle keeps both completions and recomputes from
bottom after a lane change. The model does not consume HIR, compute
last-use across CFGs, create source-package interfaces or implement a
general projection domain.

A separate experiment compares eager expansion with an exact product of
supplied independent factors. It tests all partial queries on a small
concrete relation and a transfer that needs three facts to coexist.
Deterministic order makes the refusal boundary invariant under
permutation of supplied factors, not of rows within a factor.
Twelve independent choices fit 4,096 rows after
8,190 attempted combinations and 12 setup units. With one fewer work
unit, the last product refuses before allocation. A thirteenth choice
exceeds the row cap. A factored query for 24 choices takes 48 row probes
and 24 setup units and refuses at 71 units. These are experiment counts,
not limits for compiler inputs or a bound on allocator traffic.

| Independent choices | Eager rows retained | Eager rows generated | Eager rows probed | Factored rows retained and probed | Eager median (µs) | Factored median (µs) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 4 | 16 | 30 | 16 | 8 | 0.333 | 0.042 |
| 12 | 4,096 | 8,190 | 4,096 | 24 | 11.166 | 0.084 |
| 16 | 65,536 | 131,070 | 65,536 | 32 | 218.750 | 0.084 |

Each trial starts with the same prebuilt two-row factors. The eager
measurement includes construction, a last-row query and teardown. The
factored measurement only queries its supplied factors. Medians use 31
samples of an optimised standalone test after one warm-up. Retained
rows count payload entries, not process RSS or total peak memory.
The experiment doesn't infer independence. Dense constraints can force
one component to retain exponentially many rows: an even-parity
relation projects to every combination on each proper subset but omits
half the combined rows.

#### Stage 5 projected operands

Commit `f1c88afa7` captures explicit copies in written order. Before
Stage 5, a shared place operand was still read at the call, after later
operands ran: `inspect(holder.items, replace(~holder))` was accepted and
read the replaced value, while its explicitly sequenced form rejects with
`MOTH-BORROW-0007`.

`lower_operand_sequence` in `hir_expression/calls.rs` now binds an earlier
shared argument that loads an authored place (a user binding or a
parameter, under any implicit option `some`) before a later operand's
prelude, current-block lowering or fresh mutable argument. The binding is
exactly the explicit `name = place`: a user-origin, immutable local, so
Stage 6 needs no new rule. Diagnostics name it after the authored place,
such as `holder.items`, through a side-table display path kept apart from
backend identifiers. Receivers stay unbound because their access mode
belongs to the resolved method.

Every overlapping form now rejects at the mutable argument with
`MOTH-BORROW-0007`. That covers whole, nested, optional, fallible,
parameter and scalar operands. When a later argument mutates the root
through a live mutable alias (`replace(~writer)`), the earlier read itself
rejects with `MOTH-BORROW-0003`. Under root granularity
`inspect(pair.left, replace_right(~pair.right))` also rejects, as do the
same-call and explicit forms. That is conformant base-root conservatism
(`borrow-validation.mtf`, "Access roots and aliases"). The owner deferred
field-precise acceptance to the later provenance design. The fixture
corpus showed no changed borrow verdict (3,315 cases). The eight
`runtime_call_shared_*` fixtures pin each path and its diagnostic site.
Measured back to back with the 18 guard workloads, check and borrow
medians stayed within run-to-run noise of the pre-capture binary. Peak RSS
on `sibling-loops-64` varied from 72 to 115 MiB across repeats of the same
binary.

#### HIR-shaped proof

The standalone crate under local `tmp/bc06/hir_proof/` models validated
HIR shapes: `Define`/`Update`, place projections, written-order call
operands after Stage 5 capture, jump arguments, `Invoke` channels,
error returns and lexical scope exits. It is never compiled into Moth.
An alternative stores binding and field cells, site × recency
allocation identities, permissioned edges and loans with holder sets.
An access conflicts when its cells or reachable nodes overlap a live
loan. Overlap includes a shared child reached through distinct paths.
A slot replacement needs only the replaced cell. Block states are exact
products of factors with disjoint footprints, re-split only when
`|S| = ∏|πᵢ(S)|`. Summaries are guarded outcome alternatives with
per-channel slots and permissions, weak or strong input-edge effects,
root replacement and retained loans. They converge callee-first over
SCCs, and a lane rewrite restarts from bottom. A converged empty summary
becomes a conservative completion, so only a declared Never removes a
continuation.

There are 206 deterministic witnesses. They cover the projected-operand
shapes, disjoint and shared-child siblings in same-call, explicit and
nested forms, the evidence matrix above, the C4 `select`/`forward` and
`retain` controls, mutual recursion, three-way coexistence through
transfer and composition, failure lanes, recency across loop
iterations, and exact budget boundaries. Three adversarial audit cycles
found 25 defects. Most were unsound accepts: missing result and
stored-edge loans, strong export of a callee push, dropped root
replacement, factor splits across materialised residuals and refusals
hiding reachable conflicts. Each one now has a regression test, and the
final verification audit was clean.

Work units under the default limits (work 100,000; 1,024
alternatives, nodes and loans; 32 selectors; 4,096 edges):

| Workload | Sizes | Work |
| --- | --- | ---: |
| Independent aliasing branches, factored | 4 / 8 / 12 / 16 | 2,512 / 7,730 / 15,684 / 26,374 |
| Same, eager alternatives | 4 / 8 and above | 68,818 / refused |
| Native call depth | 4 / 8 / 16 / 32 | 318 / 562 / 1,050 / 2,026 |
| Duplicate caller and callee product | 2² / 3³ / 4³ / 4⁴ | 1,314 / 3,159 / 6,128 / 18,488 |

Branch row work grows linearly (about 53 per branch), and factor visits
grow quadratically (about 55N²). That matches the O(locals) join walk
in today's `join_in_place`. Products grow with the product size. The
append loop accepts at 839 work and refuses with a limit of 838.

Precision is lost in these places:

- Branch conditions are opaque, so a repeated `if c` reports a false conflict.
- All collection elements share one selector.
- Recursive or untracked interiors saturate to typed residuals.
- Summary and recent nodes of one type may alias.
- Formal relevance is flow-insensitive, and exceeding the root-partition cap falls back to one may-alias seed.

The model refuses, rather than guess, when copying a graph with non-exact identities, on an ambiguous multi-edge summary projection, and when a conservative completion writes over a captured argument cell. An `Unproven` refusal drops only its alternative. A reachable conflict still wins, and budget exhaustion stops at once.

#### Cutover decisions

The owner approved the slice 3 cutover on this evidence. It lands as one
production change. Durable regressions must assert the diagnostic site as
well as the verdict, at the layer that owns each behaviour: direct and
forwarded `select`, the shared-child conflict, `retain`, the failure lanes,
and refusal and exhaustion.

**Exhaustion and unproven paths.** If any reachable alternative is
`Unproven`, a successful alternative can't justify accepting the function.
A conflict established on another reachable alternative takes precedence.
If the work budget expires first, analysis stops there and reports the
limit. Either outcome gets a new deterministic borrow diagnostic that names
the function and the limit or construct. No partial success or summary is
published, and the old checker is never retried.

**Corpus comparison.** Run the whole corpus through the replacement and
compare it with the archived `0f384a5539` release checker. Explain every
changed borrow verdict against the canonical rules, especially newly
accepted programs and previously accepted programs that now exhaust a
limit. The archive has documented errors in both directions, so matching
it isn't the goal.

**Performance guard.** This is fixed before the candidate is measured. The
archived release binary was re-measured on the recording host with the 18
workloads of `tmp/bc06/baseline_measure.py`. Check-time medians moved by
up to 15 % between sessions with the same binary (narrow chains 512:
184.55 ms, then 211.65 ms). Measure the archive and the candidate
back-to-back in one session, using the same script, five samples after one
warm-up. The candidate passes only if every workload satisfies all of
these:

- median `command.check.total` ≤ max(1.25 × archive, archive + 2 ms);
- median borrow time (`frontend.borrow.initial` + `frontend.borrow.converge` + `frontend.generated.borrow_recheck`) ≤ 1.5 × archive + 1 ms;
- peak RSS ≤ 1.25 × archive + 4 MiB;
- no exhaustion or unproven diagnostic on any workload, or on any `tests/cases` fixture other than those written to exercise one.

These decisions governed the attempt below. They don't apply to the
narrowed Stage 5 capture, which changes no Stage 6 rule.

#### Factored cutover attempt

The cutover replaced the local root sets with factored alternatives of
binding and field cells, allocations, edges and loans, a new transfer and
the exhaustion diagnostic. It was abandoned before review. Against the
archived checker:

- about 274 fixtures changed verdict, about 170 of them new exhaustion
  refusals, and none was newly accepted;
- 23 of 310 borrow checker unit tests and 2 of 140 provenance tests failed;
- three of the 18 guard workloads were refused, including `narrow-chains`
  `signed` and the speed test's `build_runtime_grid`, and `borrow-stress`
  gained a false `MOTH-BORROW-0003`;
- check time reached about 3× the archive on `alias-receiver-chain-1024`
  and `aggregate-forwarding-128`.

The dominant cost was dead state. Alternatives carried every authored
binding slot, about 73 dead to 16 live in a typical row. Work per HIR unit
had a median of 89, a 99th percentile of 6,579 and a maximum of 207,000
(`numeric_profile_int64_runtime_surfaces`). Retiring dead slots eagerly
broke publication of source-root, mutation and call-return facts, which
still read them. A redesign needs liveness-scoped slots, with fact
publication taken from the state at each event rather than from retained
rows. The tracked patch and untracked files are archived under local
`tmp/bc06/cutover/final-archive/`.


## Permanent performance evidence - 2026-10-12

The following is the retained content of the temporary `borrow-checker-followup-evidence.md`. It was recorded during the exact-numeric implementation review on `expression-refactor`, corrections at `2e000a073` before the squash onto `main`. Code was re-inspected on `AUDITED_MAIN`: the major mechanisms below remain present, while current line numbers may differ after future merges. Numeric/AST work is not blamed for these costs. Equivalent pre-numeric runs exhibit the same issue.

### E1. Dominant state size, copying and repeated checking

For `L` function locals, `BorrowState` contains `L` `LocalState` entries. Each has `value_roots` and `direct_alias_roots` as independent `RootSet { words: Vec<u64>, bit_len }` buffers of `ceil(L / 64)` words. A state therefore reserves roughly `2 * L * ceil(L / 64)` words, plus `2 * L` separate nonempty heap allocations. This is quadratic in local count, independent of the number of actual alias edges. The initial review described this as `O(L^2)` bits plus many allocations.

The worklist in `engine.rs` clones a complete input state to produce each block output. For each reachable successor it clones the block output again, applies jump-argument transfer, narrows visibility and joins or installs the successor input. Existing `join_in_place` avoids an additional whole-state allocation on a join, but does not eliminate these clones. Output-state equality still compares the complete state. `kill_invisible` scans local root sets at visibility-narrowing edges. With many basic blocks and locals this produces substantial copying, allocator traffic and state-comparison work.

`refresh_private_failure_lanes` invokes `check_borrows` for the whole base HIR when private failure-lane installation changed the HIR. Before `2e000a073` this refresh was not attributed to the borrow convergence metric. It now appears in `frontend.borrow.converge` for base work and `frontend.generated.borrow_recheck` for generated sidecars. These names measure distinct sources of work. Retain `command.check.total` when comparing the instrumentation boundary.

`metadata.rs::recursive_return_alias_functions` constructs local call edges, then invokes recursive `reaches_function` independently from each function with a fresh visited set. This repeated traversal takes up to `O(F * (F + E))` graph work and has call-depth proportional to a long dependency chain. It also conservatively marks recursive return provenance `Unknown`.

### E2. Profile evidence

The 1 ms-interval macOS `sample` trace on `43c72330f` recorded a generic-failure-chain `moth check` run with **1,349 main-thread samples**. Its exact workload size was not retained (the original note guessed larger than 512 from the 1.35 s sample, explicitly inference, not measurement). Of those samples, **1,338** fell beneath `run_generated_summary_convergence` and **1,295 (96% of all samples)** beneath `refresh_private_failure_lanes -> check_borrows -> BorrowChecker::run`. Within function analysis, **602** samples were at the per-block clone site, **299 + 263** at the successor path around its clone and **56** at function-layout building.

Self-time included **481** samples in `_xzm_xzone_malloc_freelist_outlined`, **432** in `_platform_memmove`, **82** in `kill_invisible`, **63** in `_xzm_xzone_malloc_tiny`, **43** in `build_function_layout` and **34** each in `check_shared_access` and two malloc routes. The review attributed approximately **1,039 / 1,349 (77%)** samples to allocation and copying. `reaches_function` reached **947 nested frames**. Recorded physical footprint and peak were both **7.4 GB**. These are one sampled run's attribution, not a guaranteed distribution across projects.

### E3. Matched checks

Medians of five interleaved runs on one Apple Silicon macOS host. Release builds with `--features detailed_timers` compared `43c72330f` (before timing correction) with `2e000a073` (after). Units are **milliseconds**. The `frontend.borrow.converge` jump is predominantly the newly visible private-failure refresh, **not** evidence of a performance regression. Total check time is the correct before/after comparison.

| Workload | Size | Borrow converge before | Borrow converge after | Check total before | Check total after | AST total after |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Sibling loops, one collection (`r4-512.moth`) | 512 | 0 | 2268.71 | 4912.23 | 4380.14 | 3.22 |
| `narrow-chains` | 512 | 0 | 1064.07 | 1106.93 | 1086.95 | 4.39 |
| `generic-failure-chain` | 32 | 0.22 | 1.56 | 5.67 | 5.26 | 0.61 |
| `generic-failure-chain` | 128 | 1.07 | 19.39 | 32.07 | 26.51 | 1.48 |
| `generic-failure-chain` | 512 | 8.34 | 446.48 | 569.65 | 475.25 | 4.79 |
| `alias-receiver-chain` | 256 | 0 | 0.58 | 5.96 | 5.47 | 1.48 |
| `alias-receiver-chain` | 1024 | 0 | 3.91 | 29.97 | 17.29 | 4.64 |
| `alias-receiver-chain` | 4096 | 0 | 46.36 | 324.21 | 126.61 | 20.82 |
| `alias-receiver-chain` | 8192 | 0 | 180.73 | 1218.14 | 432.43 | 42.17 |
| `aggregate-forwarding` | 512 | 26.18 | 53.44 | 520.43 | 522.88 | 16.10 |
| `origin-fanout` | 512 | 0 | 11.74 | 29.18 | 29.41 | 2.39 |
| `copy-chain` | 512 | 0 | 0.74 | 5.72 | 5.15 | 1.02 |
| `speed-test.moth` | - | 0 | 7.61 | 22.15 | 22.59 | 6.74 |
| `collection-stress.moth` | - | 0 | 33.58 | 51.61 | 51.45 | 1.68 |

The visible converge metric grows **1.56 -> 19.39 -> 446.48 ms** for successive 4x generic-failure-chain sizes (approximately 12x then 23x), while the AST metric grows much more slowly. `narrow-chains-512` spends about **1064 of 1087 ms** in borrow convergence and only **4.39 ms** in AST. The 512 sibling-loop check spends **2269 ms** in the newly attributed refresh out of **4380 ms** total, with additional first-pass borrow cost not extracted into the temporary timing JSON.

The pre-numeric checkpoint `9a416ab67` took **5213 ms** on equivalent `Int` source with 512 sibling loops, recorded in `benchmarks/frontend-optimization-results.md` under *Exact numeric types: review follow-up (2026-10-12)*. A 1024-sibling-loop check was observed around 52 seconds but its raw log was not retained. **Do not cite that observation as a verified benchmark without a rerun.**

### E4. Reproduction and evidence preservation

`just validate-full` passed all 13 recorded scaling series on the corrected review tree, but those series measured `frontend.ast.total`. They provide **no borrow-cost regression gate**. The review recorded fitted AST exponents `n^0.98` for numeric alias-receiver-chain and `n^0.87` for numeric generic-failure-chain against an AST budget of `n^1.25`. These numbers do not constrain borrowing.

Reproducible checked-in sources include `benchmarks/numeric-scaling/generate.py` and its `*-{32,128,512}.moth` cases, with larger alias-receiver-chain sizes. Regenerate with:

```sh
python3 -B benchmarks/numeric-scaling/generate.py
cargo build --release --features detailed_timers
# Run moth check on each workload five times and compare medians.
# Capture frontend.borrow.initial, frontend.borrow.converge,
# frontend.generated.borrow_recheck, frontend.ast.total and command.check.total.
```

For a macOS sampling run, build with `CARGO_PROFILE_RELEASE_DEBUG=true` and use `sample <pid> 5 -file out.txt` on the active `moth check` process.

Historical raw artefacts were **local-only**, not committed and may disappear. Their identifying paths are retained so later work knows what must be reconstructed:

| Original path | Recorded purpose |
| --- | --- |
| `tmp/exact-numerics/review-followup/measure.json` | Before/after five-run medians |
| `tmp/exact-numerics/review-followup/gfc.sample.txt` | Symbolised generic-failure-chain sample |
| `tmp/exact-numerics/review-followup/validate-full.txt` | Passing full validation and AST scaling fits |
| `tmp/exact-numerics/review-followup/r4-512.moth` | 512 sibling collection loops in one function with an I32 helper |
| `tmp/exact-numerics/review-followup/alias-8192.moth` | Large alias receiver chain |
| `tmp/exact-numerics/review-followup/r5-*.moth` | Nested Pair chains, borrow-neutral AST control |
| `benchmarks/frontend-optimization-results.md` | Committed performance write-up and pre-numeric comparison |

Preserve or regenerate the sibling-loop and large-chain reproducers before changing state representation. Record exact compiler/checkpoint, hardware, build flags, input fingerprints, peak memory and sample counts. Compare both borrow passes and whole-command time, and use a no-borrow control. A post-typed-expression baseline is required before accepting claimed performance wins.

## Open work - production

A confirmed problem accepts the **need to investigate and fix** it, not the first proposed algorithm. A candidate becomes an accepted implementation only after scoped tests, semantic review and representative measurements. The ordering here is local to this companion and does not override the main roadmap's unrelated sequencing.

| ID | Status | Priority | Next production work and acceptance gate |
| --- | --- | --- | --- |
| `BC-01` | Confirmed, queued | High | Add a borrow-specific scaling and memory baseline with dedicated initial/convergence measurements and an end-to-end check guard. Reproduce E2/E3 on the branch after the typed-expression merge. Commit durable workload generation, not multi-GB raw traces. |
| `BC-02` | Confirmed, implementation open | High | Eliminate dominant per-block/per-edge dense `BorrowState` copying. Compare contiguous buffers, sparse roots, interned/shared representations and carefully proven in-place transfers. Measure allocations, bytes, memory and wall time. Preserve same-place, mixed-slot/alias and visibility-kill correctness. Existing `join_in_place` is already delivered. |
| `BC-03` | Confirmed, implementation open | High | Avoid unnecessary whole-module `check_borrows` in changed private-failure-lane refreshes. First prove affected-function and caller-summary invalidation, side-table replacement and deterministic report publication. Preserve the existing `Unchanged` fast path and ensure generated sidecar convergence is not bypassed. |
| `BC-04` | Confirmed, implementation open | High | Replace repeated recursive `reaches_function` searches with a single deterministic iterative SCC/recursion classification. Include long acyclic chains, self recursion and mutually recursive SCCs. Do not independently promote recursive result precision. |
| `BC-05` | Investigate, unmeasured | Medium | Measure repeated function cloning in `finalize_public_call_summary_effects_to_fixed_point` (`self.module.functions[function_index].clone()` each iteration) and full-state equality cost. Prefer borrowing/reuse before extra caches. Avoid adding a parallel call-graph owner without evidence. |
| `BC-06` | Baseline and finite witnesses recorded, rewrite gated | Blocks const C4 | Review the separate HIR capture prerequisite and prove a bounded HIR-shaped relational model before replacing coarse native provenance. No research activation or downstream lifetime work. |
| `BC-07` | Known capability gap, gated | Later | Complete solver-independent future/last-use and affine-transfer facts required by lifetime analysis, including loop final-iteration and detached stored-result cases. Missing optional transfer never becomes a rejection. Lifetime topology, retained-edge proofs and REC retain their own downstream ownership. |
| `BC-08` | Evidence needed | Later | Investigate broad borrow-fact compaction only after measuring fact-table cost independently of lattice cloning. Keep immutable facts outside HIR and do not merge unrelated parser/statement layouts into this effort. |
| `BC-09` | Untriaged | Later | Reassess use of folded private-constant HIR metadata in borrow analysis and temporary-local reduction after the typed-expression foundation and its reductions merge. It was a deferred compiler follow-up, not evidence of a current bug or an accepted optimiser. |
| `BC-10` | Candidate only | Later | Investigate cheap production fast paths when no aliases, live loans, exclusive access, optional-transfer candidates, recursive call-summary SCCs or structural projections exist, or when a function is scalar-only or has an acyclic CFG. These are previously recorded Boracle observations, not implemented production skips. Require sound guards and evidence before promoting any. |

### Candidate approaches for BC-02 and BC-03

The historical review proposed: (1) put all root words in one contiguous state buffer, (2) reuse a block or single-successor state in place when its lifetime allows, (3) use sparse/interned roots when sets are mostly empty, (4) recheck only functions affected by private-failure rewrite and (5) classify recursion through one iterative Tarjan/Kosaraju-style SCC traversal.

None of these five approaches was prototyped or measured in the recorded investigation. Contiguity reduces allocator churn but may leave quadratic bytes and copying. Sparse storage can trade allocation savings for worse unions. A changed function's callers may need new summaries. No implementation approach should be locked before measuring these trade-offs and proving the cross-stage invariants.

### Deferred function-boundary borrow recovery

Function-boundary borrow recovery waits for the next suitable production driver or fact-lifetime change after per-function analysis inputs, publication and required call-summary dependencies are stable and measured. It need not wait for every future precision experiment or the whole memory model, and it does not promote Boracle into the production acceptance path.

The follow-up would stop a function at its first final user-facing borrow conflict, discard or keep unreachable only its unpublished scratch and continue an independent function only with trustworthy required summaries. Recursive summary groups may need a coarser boundary. An unavailable summary blocks dependent checking without a fabricated source error or a falsely valid default summary. Conservative proof failure stays distinct from a proven conflict.

Revisit the current writes into the shared borrow report and summary convergence before changing its `?` boundaries. No success report, effect summary, generated completion or lifetime validation may consume partial failed facts. Reuse the delivered module limits and stop metadata, measure valid-path costs and test repeated or converging analyses without duplicate diagnostics. Conflict-directed precision work improves acceptance evidence rather than within-function error enumeration. Boracle keeps its independent reference and oracle roles and activation gates.

## Open work - Boracle and the production handoff

Keep reference-solver findings and downstream production relevance separate. Boracle's accepted research results belong in its permanent reference/oracle authorities. This tracker owns the decision to investigate, promote or reject their **production** implications and the practical tooling/issues that must be resolved.

| ID | Status | Issue and next evidence |
| --- | --- | --- |
| `BR-01` | Confirmed Boracle reference gap. Production impact unverified | The bounded oracle reports `StaticAcceptedRuntimeConflict` with `SoundnessFailure` when a shared-alias holder performs an exclusive write. The reference solver unconditionally excludes a holder's own loan use and misses a terminal write-through definition without a following provenance writer. This is pinned in `boracle/tests/differential.rs`. Give reference correction its own semantics/test audit. Check whether production has any analogous acceptance defect before describing it as a production bug. Canonical Boracle reference docs retain the exact rule locations. |
| `BR-02` | Historical replay failures. Revalidate after cutover | A historical 1068-source real replay at `99ab43de2` recorded two `BorrowProblem` extraction failures (before oracle execution). `tests/cases/result_match_guard_propagation_order/input/@entry.moth` produced unsorted branch targets (`BlockId(12)` then `BlockId(7)`), violating strictly increasing unique terminator targets. `tests/cases/runtime_if_reactive_metadata_preserved/input/@page.moth` produced unknown `LocalId(1)` in retired reactive-if metadata. Re-run the current corpus after the typed-expression/Reactivity V1 retirement. Fix only surviving producer bugs, not obsolete V1 semantics. |
| `BR-03` | Confirmed developer-tooling gap | `boracle/reducer.rs::reduce_problem` and `render_fixture_skeleton` exist and have tests, but `src/projects/cli.rs` has no reduction command and `boracle/service.rs` uses `OracleBounds::default()` for differential dumps. Expose reproducible bounds and class-preserving reduction through a bounded developer CLI change if still useful. No production checker or public solver switch. |
| `BR-04` | Research inactive | Typed provenance relations and the bounded oracle are completed reference capabilities. Conflict-directed refinement, loop epochs/edge last-use, call summaries/deferred exclusive access and aggregate-copy/builtin-storage provenance remain proposed packages 3-6, requiring refreshed evidence and explicit activation after merged native-result capabilities. Research acceptance alone does not promote a production solver rule. |
| `BR-05` | Deliberately deferred | The reference's `dead-exclusive-loan` experiment remains opt-in and not canonical acceptance. `MultiHolderLoan`, `RepeatedProjectionChild`, dynamic-index sibling precision and empty-union CFG joins have bounded or deferred treatment in the reference research handoff. Reopen only with an actual reduced case and named research owner. |
| `BR-06` | Historical oracle inconclusive. Revalidate | The old progress matrix recorded checked numeric reassignment with `OracleInconclusive` / `UndecidableOverlap` on the existing result-carrier projection and no completed executions. It supplied no bounded safety evidence. Infallible Boolean reassignment had recorded real-source mutation replay coverage, which should be retained as a control. Revisit after native result-channel migration, with a reduced source replay and separate proof of any improvement. |
| `BR-07` | Exploratory, inactive | [Group-aware place precision and stability effects](./boracle-next-research-plans/group-aware-place-and-stability-effects-research.md): investigate Zeta-style positive index-disjointness evidence, guards, narrow builtin access footprints and structural/iterator stability without weakening shared/exclusive access, data-race freedom or lifetime topology. Use Boracle's existing origin and conflict owners. Research activation requires a fresh checkpoint, relevant native-result prerequisites and resolution or isolation of `BR-01`. Any changed collection receiver contract, public effects or concurrency model needs separate semantic approval. |

For the older replay failures, the historical reproducer was `cargo run --features boracle -- boracle <source> --dump problem`. Its result was tied to an earlier source corpus. Do not claim those exact failures persist on a later branch without rerunning it. Resolve the reference acceptance gap before using that case as supporting evidence for a production precision change. The `BR-01` counterexample is a **known reference-model issue** and must remain visible to the differential campaign until resolved. A bounded oracle is not a replacement for a proof of universal correctness.

The proposed `$checked` deeper proof-budget surface remains a separate design/research question. It is not implementation-ready and must not alter the ordinary source safety rules. Production compilation retains no developer-visible reference/experimental mode switch.

## Working and closeout rules

1. **Record evidence first.** Give each new issue a stable `BC-` or `BR-` ID, affected owner, checkpoint, reduced reproducer and observed behaviour. Distinguish confirmed bugs, implementation gaps, design proposals and performance hypotheses.
2. **Accept a specific fix separately.** Before changing the algorithm, state preserved semantics, new invariants, correctness cases and the cheapest measured representation that solves the problem. An unchecked speed claim is not an accepted decision.
3. **Validate differences.** Run focused production tests, source fixtures, existing module/generated summary gates and applicable `just boracle` / `just boracle-campaign` gates. Preserve independent oracle comparisons, invariant failures and deterministic witnesses. Record the exact branch revision and commands actually run. Do not inherit a historical pass.
4. **Measure performance on representative shapes.** Include normal source, worst-case chain/loops, large locals, generated/private failure convergence and a borrow-neutral control. Capture both borrow stages and end-to-end cost. Avoid timing comparisons across different metric definitions.
5. **Record the outcome and trim.** On landing, replace an open item with a short accepted decision, actual validation/measurement outcome and commit/PR link. Remove superseded alternatives, copied plans and stale duplicate status. Preserve durable regression inputs and meaningful negative tests, not running logs.
6. **Respect owners.** Canonical memory/compiler docs define semantics and stage boundaries. Boracle docs define reference and oracle behaviour. This companion owns live production progress, implementation choices, follow-ups and the handoff from research. The main roadmap owns only the combined parallel entry and prerequisites. The progress matrix links here instead of repeating this inventory.

The document is intentionally permanent. Finishing an individual patch does not complete or delete the tracker. Keep only the current small queue and accepted decisions after closeout. Git records detailed historical implementation steps.

## Channels V1 research input

Before restarting concurrency-related Boracle or memory-model research, read
`docs/docs/async/channels.mtf` and the canonical memory authorities. The accepted
model requires explicit task dispatch and semantic send commits, independent
message graphs, failure-only send recovery, implicit suspension and joins,
logical endpoint leases, selection winner/loser effects and cancellation
cleanup. These extend the normalized runtime borrow problem and lifetime
requirements, not the separate const interpreter access model. The memory design
closure must establish the production prerequisites and partition early proof
from later physical planning. Research may investigate the new operations under
explicit activation; no change to accepted borrow rules or production solver
activation is implied by this documentation update.
