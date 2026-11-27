# BC-06 redesign investigation

This companion to [BC-06: Relational production provenance](./production-borrow-checker.md#bc-06-relational-production-provenance) records the investigation into projection-precise ordinary borrow checking and its implementation checkpoints. It holds the reduced witnesses, expected outcomes, reproduction commands and decisive measurements.

**Initial owner direction (2026-11-25):** Pause production implementation until the design and implementation sequence have been reviewed. Preserve the uncommitted holder-edge candidate. The BC-06 handoff had said to resume C4 on the existing checker, while [C4](./const-functions-evaluation-foundation-and-semantics-plan.md#c4-integrate-required-evaluation-and-close-publication) needed ordinary access checking to preserve correlated sharing and permission.

**Approved contract decisions:** `hashmap_alias_struct_field_param_return` and the ordinary `runtime_detached` function must reject at their mutation with `MOTH-BORROW-0007`. The ordinary `runtime_retained` function must accept and render `22`. Fixture manifests change only in the slice whose rule changes them. The owner approved the design with the [Slice 0 decisions](#slice-0-owner-decisions).

## Resume point

| Item | Value |
| --- | --- |
| Branch | `docs/const-foundation-profile-independent-semantics` |
| Stage 5 comparator | `7cc05c61bf5d4b6de23abfeec8a67d9623be971d` |
| Slice 1 | The verified cap-free release build is `a08f2d820e3ed`. Its parent `0e95895fd5` is the user-pushed review base, not the Slice 1 checkpoint. |
| Toolchain | rustc and cargo 1.99.0, Apple M1 Pro, Node v24.21.0 for rendered output |
| Sessions complete | 1 (Sections 1-2: evidence and witnesses), 2 (Section 3: fact-lifetime ledger), 3 (Section 4: calls and publication), 4 (Sections 5-7: C4 consumption, design choice and implementation sequence) |
| Last established result | B1 (`f2390602e`) remains the accepted checkpoint. The B2 draft handles the focused fresh-value replacements but still misses caller-visible alias acquisition and stores on mixed complete/unresolved exits. |
| Next task | Review the [revised capture sequence](#proposed-sequence) before resuming production implementation. No Stage 5 or B2 checkpoint, no release guard and no shared C4 runtime HIR lowering. Preserve both C4 fixture expectations. |
| Latest evidence | [Retarget and mixed-exit controls](#retarget-and-mixed-exit-controls) pin summary failures. The [Stage 5 architecture review](#stage-5-architecture-review) compares frozen B1, pre-capture B2 and explicit sequencing. |

### Approved B1 correctness differential: canonical permission meet

When `~slot = place` retargets an independent mutable slot, it acquires an
alias to the operand's place. Follow the canonical
[access rules](../../developer-docs/memory-management/access-and-aliasing/access-and-aliasing.mtf):
the resolved place permits mutation only if every binding, alias link and
projection restriction along the path permits it. An existing write-through
alias or projected destination only reads the operand's value, so it doesn't
need mutation permission on that operand.

`catch |err|:` introduces an immutable binding. Mutable compiler scratch
doesn't grant authority through `err` or through an alias such as
`saved Failure = err`. B1 accepted those catch retargets because it checked
the scratch's mutability. The owner approved rejecting `~code = err.code`
and `~msg = saved.message` with `MOTH-BORROW-0007`, as for retargeting to
`pair.left` through an immutable `pair`. This corrects B1 rather than
relaxing the source rules.

Correct affected corpus fixtures case by case with explicit `copy` where
they intend independent value replacement, as in `~code = copy err.code`.
80 existing runtime fixtures needed this. Each edit only adds `copy` to
the retargeting operand and leaves expectations and rendered output
unchanged. The fixture commit keeps the original inputs in history. The
recorded pre-change verdict snapshot and the archived comparator evidence
attribute each changed verdict to the permission correction or to the
fixture edit. Seven new fixtures pin the rule for ordinary bindings,
builtin and custom catch bindings, saved error aliases, explicit copies,
write-through from read-only operands and compiler transport. The
benchmark thresholds don't change.

### Retarget and mixed-exit controls

C1 and the canonical [access reference](../../developer-docs/memory-management/access-and-aliasing/access-and-aliasing.mtf)
settle the operation, independently of JavaScript. A `HirWriteTarget::AssignPlace` with a
`ValueKind::Place` operand lets an independent mutable slot acquire an alias. An existing
alias or projected destination instead writes the operand's value through. A produced value
does not acquire an alias. Const `execute_assign` and `LiveStorage::rebind_to_place` make
this distinction for local writes. Const doesn't admit mutable formals, so it isn't a
call-boundary oracle for these cases.

**Escaping scalar cell.** This uses `I32` deliberately: there is no heap allocation
provenance to stand in for cell identity.

```moth
seed |first ~I32, second ~I32|:
    local ~= 2
    ~first = local
    ~second = first
;

first ~= 0
second ~= 1
seed(~first, ~second)
observed = second
~first = 9
~[:seen=[observed] first=[first]]
```

B1 and the B2 draft accept. The draft renders `seen=9 first=9`. Both caller bindings
lead to one cell, so the write conflicts with the live shared observation. Inline the
two assignments at the call site, with the same fresh `local`, and B2 rejects the
write with `MOTH-BORROW-0007`. The positive control creates `other ~= 2` in `seed`
and assigns `~second = other`. B1 and B2 accept and the draft renders
`seen=2 first=9`. Equal contents do not identify a cell. Moving allocation
provenance alone, making each retarget fresh independently, or using the callee's
lexical local ID at all call sites cannot preserve this pair.

**Forwarded role transition.** Execute `set_from(~dest, ~first)` followed by
`set_from(~dest, ~second)`, via a one-call forwarder, where `set_from` performs only
`~dest = source`. With a slot destination and `first` an interior field, the first
call captures the field and the second writes `second`'s value into that field.
It must not redirect `dest` to `second`. A subsequent `observed = dest` followed by
a write to `first`'s field while `observed` lives must reject. B1 and B2 accept
this case and the draft renders `9`. A summary needs a current binding role at
composition, not two unordered retarget rows interpreted against entry mode.

```moth
Cell = |value I32|

set_from |dest ~I32, source ~I32|:
    ~dest = source
;

forward |dest ~I32, source ~I32|:
    set_from(~dest, ~source)
;

twice |dest ~I32, first ~I32, second ~I32|:
    forward(~dest, ~first)
    forward(~dest, ~second)
;

slot ~= 1
incoming ~= Cell(2)
later ~= 3
twice(~slot, ~incoming.value, ~later)
observed = slot
~incoming.value = 9
~[:[observed]]
```

**Mixed success lane.** Both success paths exist. Only the path through `remove`
publishes the `dest.items` store.

```moth
Box = |items {I32}|
Holder = |boxes {Box}|

seed |dest ~{Box}, source ~{Box}|:
    ~dest = source
;

guard |dest ~Box, incoming {I32}, skip Bool| -> I32, Error!:
    if skip:
        return 0
    ;
    empty ~{Box} = {}
    holder ~= Holder(empty)
    source ~= {Box({10})}
    seed(~holder.boxes, ~source)
    removed ~= ~holder.boxes.remove(0) catch |err|:
        return! err
    ;
    ~dest.items = incoming
    ~removed.items = {2}
    return 1
;

probe |skip Bool| -> I32:
    incoming ~= {1}
    dest ~= Box({0})
    status = guard(~dest, incoming, skip) catch:
        return 9
    ;
    ~incoming.push(3)
    return status * 10 + dest.items.length()
;

result = probe(false)
~[:[result]]
```

B1 and B2 accept and the B2 draft renders `12`. Replace `seed(~holder.boxes, ~source)`
with `~holder.boxes = source`: B2 reports `MOTH-BORROW-0007` on `~incoming.push(3)`.
Graph extraction knows the first success exit, but the missed projected store leaves
the `remove` without element provenance. The later write through `removed` stalls
the second exit and suppresses its already-executed `dest.items` store. The caller
visits the success continuation, but checks it with incomplete effects. Adding
opaque *result* rows cannot recover this effect: the result is scalar.

Keep structural reachability and provenance completeness separate even within a
lane. An unsolved dependency can be graph bottom during convergence. A stable
unresolved operation needs conservative effects, not invented freshness,
unreachable flow or a certificate that a store didn't occur. A fix must keep
complete effects and cover missing ones on the same lane, then recheck caller
lifetimes, allocation and permissions. Don't remove strong exact direct updates
or publish irreversible opaque rows for transient recursive bottom.

The independent design review identified three implementation constraints:

1. Capture a whole wrapper, a projected location and the incoming value as distinct
   facts. Compose binding postconditions in operation order, with explicit entry
   roles and a preserved alternative unless coverage proves otherwise. A call's
   result receiving write runs after its binding effects. A returned value does
   not carry a callee binding cell.
2. Publish only observable escaped cells. Canonical summary-local identities need
   to represent shared scalar cells as well as aggregate storage. Instantiate
   identities per call and widen repeated occurrences to a finite nonsingleton
   class. Never allocate a new identity from an ever-growing forwarding path.
   Preserve the cell's reference permissions and its contents separately.
3. Mandatory body checking must cover feasible entry roles, not just summary
   replay. `engine.rs` currently initializes all formals as slots. Argument
   mutability doesn't prove a caller wrapper is a slot. Acquiring an exclusive
   place on the slot branch and reading a value to store on the alias branch
   require different access. Reuse the existing direct transfer rules instead
   of teaching an unrelated call-only checker.

Independent Boracle replay still finds the direct escaping-cell conflict.
The called escape and forwarding controls formerly reported `CompleteSafe`
because scalar binding effects were missing from normalized input. Both now
report `Inconclusive(OpaqueCallStores)` without consulting production acceptance.
The mixed `guard` still yields `OpaqueCallStores` after one completed execution.
The [inactive research proposal](./boracle-next-research-plans/README.md#inactive-proposal-consolidate-packages-36-after-bc-06-and-const-publication)
records the future replay work.

### Typed binding-transition model

**Status:** Both independent reviews found required changes. The revised
reference model responds to their counterexamples. The sparse implementation
contract and HIR operand capture still need evidence before production cutover.

#### Storage and access

| Concept | Identity and contents |
| --- | --- |
| Name | A HIR local denotes a current binding wrapper. A definition creates a new dynamic occurrence. A write doesn't. |
| Binding wrapper | A typed handle with a slot or alias role. `Slot(Cell)` names its independent storage. `Alias(Place, access)` follows a whole wrapper or a captured projected location. It doesn't cache the terminal cell. |
| Storage cell | Either a slot cell, owned by one wrapper, or an edge `(Allocation, selector)`. Holds a value, not another binding wrapper. An exact field selector names one edge of one exact allocation. An element/key family isn't a singleton. |
| Value | Scalar with no allocation, or a reference to an allocation and its retained access. Capturing or returning a value doesn't transport its wrapper. Equal scalar or aggregate values don't identify storage. |
| Access | The permission to use a place, the retained permission on a stored allocation and the derivation of a live loan are separate. Resolve by restricting permissions along the chain. Don't derive allocation mutation authority from writable storage, or a loan exemption from target equality. |

A slot and its owned cell have a one-to-one identity. Reuse the wrapper key for the
slot cell in storage, but don't equate operations on the wrapper with operations
on the cell. Keep the existing sparse identities for heap allocations and
projections. A root load can follow a live wrapper, while a projected load
captures a position in the allocation evaluated at that point. Replacing a
separate slot doesn't mutate an old allocation retained by another value.
Replacing an edge changes that edge, not the detached allocation. A live
projection still overlaps a replacement of the location it observes.

Share the write plan: capture operands, resolve roles and locations, specify
the required accesses and describe the state change. Mandatory validation
checks those accesses before accepting the change. Extraction evaluates the
state change without using the borrow verdict. Applying a summary installs a
postcondition, not a reclassified HIR assignment. Host calls and compiler
builtins keep their own access and value contracts. They don't retarget source
wrappers merely because the same code replays their stores.

| Destination and operand | Access and state change |
| --- | --- |
| Independent slot, producing a value | Check replacement of the slot; store the captured value. Leave the role and detached allocation alone. Result receiving uses this operation or defines a new wrapper. |
| Independent slot, assigning a place | Check the slot and the incoming place's exclusive authority under the direct assignment rules. Install a link to the incoming place, not a copy of its current allocation. |
| Existing alias, assigning a place or value | Read the operand as a value and check the resolved destination for writing. Keep the immediate link and store in the terminal cell. |
| Projected destination, assigning a place or value | Evaluate the containing allocation and selector before storing the captured value. Same resolved-location overlap as a projected argument. Don't rebind the base wrapper. |
| Direct self-update | Use the HIR self-update operation's read/write checks and write-back. A plain store from a place that resolves to the same slot does not add an alias cycle. Check the alias/write branch normally. |
| Definition or local rebinding at a definition site | Install a new wrapper. Don't change the wrapper retained by earlier observers. A definition from a place keeps the permissions and direct checks of its declared alias role. |
| Compiler-owned transport | Use the HIR destination origin to preserve carried value, loan and permission facts without issuing a source-level exclusive alias. Catch scratch definitions and jump plumbing don't implement authored place assignment. |

Capture whole mutable actuals as their existing wrappers, projections as fixed
locations passed via an alias wrapper, and produced values as independent
temporary slots. A projected or temporary wrapper can itself escape through
another actual. A projected wrapper already acts as an alias. A temporary
wrapper has no pre-existing caller name to update. Evaluate arguments once in
authored order and maintain an earlier access across later argument evaluation.
For `use(~holder.inner.item, replace(~holder))`, the earlier mutable projection
must designate the original location and participate in the later access check.
In the baseline, `hir_expression/calls.rs::capture_earlier_operands` stages
copies and shared places but leaves this mutable projection unevaluated.
Capturing a whole wrapper doesn't pin its current allocation.

The baseline B2 draft accepts the following and renders `9`, not the value
`2` at the originally designated place. Capturing that place earlier must
also expose the overlapping second mutable access to the checker.

```moth
Leaf = |item I32|
Holder = |inner Leaf|

replace |holder ~Holder| -> I32:
    ~holder = Holder(Leaf(9))
    return 0
;

observe |item ~I32, number I32| -> I32:
    return item + number
;

holder ~= Holder(Leaf(2))
result = observe(~holder.inner.item, replace(~holder))
~[:[result]]
```

The Stage 5 candidate extends the existing operand prelude with an ordinary
mutable alias definition and a place-valued reload. An rvalue reload would
pass the value in a fresh slot, losing write-through and caller destination
identity. Builtins and the AST supply receiver access modes. The existing
projection overlap query applies both at acquisition and when a captured
alias reaches the call. The ordinary alias rules let a child exercise its
parent access, but not the reverse.

The `language.functions.call_mutable_projections_keep_authored_places` contract
checks named routing, mutable receivers, alias chains, two independent
allocations, shared children and both fallible paths. The isolated B1
compiler with the capture correction agrees on the scalar, receiver and
fallible controls. Frozen B1 already rejects the independent-sibling receiver
case. The Stage 5 candidate preserves the existing B2 acceptance there and
rejects its shared-child control.

An untimed counter replay with frozen B1 and the isolated B1 capture build
found identical HIR statement, block and function counts across all 18
fixed workloads and the separate int64 case. The dedicated capture control
adds six statements (`72` to `78`), one alias definition per pinned
projection, with blocks (`40`) and functions (`8`) unchanged. This does
not change the formal release guard or explain the int64 timing regression.

**Separate whole-binding access gap.** Frozen B1 and the capture candidate
accept `observe(~slot, replace(~slot))` with `slot ~= 2`, where both callees
take a mutable `I32` place and `replace` writes `9`. The earlier exclusive
access should cover the later call. An explicit alias pin demonstrates the
overlap, but lowering a whole argument to that alias would change a slot
entry into write-through. Keep the original wrapper role and track access
timing separately from location capture.

#### Stage 5 capture gate

```moth
Pair = |left I32, right I32|

peek |value I32| -> I32:
    return value
;

put |value ~I32, number I32|:
    ~value = number + 0
;

pair ~= Pair(1, 2)
put(~pair.left, peek(pair.right))
~[:[pair.left],[pair.right]]
```

**Scalar sibling read.** The later read is disjoint from the captured scalar place. Pre-capture
B2 accepts and renders `2,2`. The Stage 5 candidate instead emits
`MOTH-BORROW-0003` at `peek(pair.right)`, treating the live capture as
exclusive access to the whole `pair`. Replacing `peek(pair.right)` with
`peek(pair.left)` creates overlapping access. Pre-capture B2 also accepts that
twin, but the capture candidate correctly rejects it. Frozen B1 rejects
both twins with `MOTH-BORROW-0007` at the mutable call argument. It cannot
serve as the precision oracle for this pair.

The storage layer omits scalar alias cell identity in `capture_local_value`.
Shared call and expression checks see the capture's coarse enclosing root,
not which scalar field the capture protects. Do not restore acceptance by
skipping the capture or suppressing the conflict for scalar reads. The next
candidate must distinguish the two locations and continue to reject reads
of the whole aggregate. This prerequisite has not passed and does not
authorise integration of the call summary correction.

**Multiple possible holders.** `choose` returns one of two independent
objects. The observer and capture project different fields in every feasible
case:

```moth
Pair = |left {I32}, right {I32}|

choose |flag Bool, first ~Pair, second ~Pair| -> Pair:
    if flag:
        return first
    ;
    return second
;

next || -> I32:
    return 7
;

probe |flag Bool| -> I32:
    first ~= Pair({1}, {2})
    second ~= Pair({3}, {4})
    pair ~= choose(flag, ~first, ~second)
    observed = pair.left
    ~pair.right.push(next())
    return observed.length()
;

~[:[probe(true)],[probe(false)]]
```

Pre-capture B2 accepts. The candidate rejects `pair.right` with
`MOTH-BORROW-0007` because the captured reload loses projection overlap
evidence without a singleton cell. Overlap can use a complete set of possible
locations. Only a strong update requires a definite singleton. Retain
coverage separately from cardinality; a missing case still prevents proving
disjointness.

**Collection stability.** A lookup borrows the collection's entry even
when the value stored there has its own allocation:

```moth
Bucket = |scores {String = {I32}}|

bucket ~= Bucket({"k" = {1}})
score = bucket.scores.get("k")!
scores ~= bucket.scores
~scores.set("k", {2, 3})!
~[:[score.length()]]
```

Frozen B1 and pre-capture B2 reject the explicit alias with
`MOTH-BORROW-0007`. The candidate accepts it. The refined allocation
query drops the live `get` result because an element does not reach its
containing map. The projected call `~bucket.scores.set(key(), {2, 3})!`
with `key()` returning `"k"` already slipped past pre-capture B2 and
still passes the candidate. Do not treat the element's value as detached:
the const storage model and ordinary builtin contract distinguish a
borrowed `get` location from the value that `remove` detaches. Test
collection stability, replacement of the destination cell and mutation
through its value as separate access footprints.

**Access evidence for the isolated correction.** The controls in
`tests/cases/runtime_capture_*` compare authored captures with explicit
projected aliases and separately evaluated operands. Judge three independent
facts:

| Fact | What it proves | What it does not prove |
| --- | --- | --- |
| Exact destination | Every path designates the same singular cell. | That the previous value's allocation is also mutated. |
| Complete target coverage | The possible destinations include every feasible target. Disjointness requires every pair to be disjoint. | A strong update when more than one target remains, or disjointness when a target may be missing. |
| Receiver stability | A live lookup result protects the map's entry structure, including when the returned value is scalar or stored in a separate allocation. | That the result points to the map allocation, or that a detached `remove` result holds the same loan. |

Scalar capture needs the projected cell address, not an allocation for the
scalar. A whole-aggregate read still overlaps it. The const interpreter's
`EdgeLocation` keeps the owning allocation, selector and structural generation
separate from the value. This helps explain why a region or allocation proof
cannot replace a check for observable entry stability. Const invalidation
and runtime exclusivity still use different validation rules.

The next candidate passed all 15 sequenced controls and the original mutable
and shared capture contracts. It still failed independent correctness review
and `mutable_projected_alias_conservatively_blocks_rebound_holder_read`
(`263/264` borrow checker unit tests). The latter pins conservative holder-cell
overlap when an alias survives a rebinding. Do not change its expectation in
the capture correction. Restore the conservative check where the refinement
cannot justify changing that accepted behaviour.

The following reductions use the declarations from the controls above. For
the scalar-store case add `tag {I32}` to `Pair` and construct it from an
existing collection so the allocation layer records an edge. For the named
sibling case use collection fields and a `peek` that returns their length.

| Reduction before the final use | Canonical verdict | Candidate |
| --- | --- | --- |
| `~pair.right = 4` then `put(~pair.left, peek(pair.right))` | Accept. A scalar write does not remove the field's cell. | Rejects the `right` read with `0003`. |
| `right = pair.right` then `put(~pair.left, peek(right))` | Accept. Naming the independent child does not widen its read. | Rejects `right` with `0003`. |
| `raw = bucket.scores.get("k")!` then `score = identity(raw)`, `~bucket.scores.set(key(), {2, 3})!` then use `score` | Reject. An identity return still observes the lookup. | Accepts. |
| `row = bucket.scores.get("k")!` then `item = row.get(0)!`, `~bucket.scores.clear()` then use `item` | Reject. With `scores {String = {{I32}}}`, the second lookup protects both receivers. | Accepts. |
| `dest ~= {0}`, `writer ~= dest`, `score = bucket.scores.get("k")!`, `~writer = score`, `~bucket.scores.clear()` then use `writer` | Reject. The writer observes the result stored through its cell. | Accepts. |
| `score ~= bucket.scores.get("k")!`, `~bucket.scores.clear()`, `~score = {0}` then use `score` | Accept. The definite replacement does not use the old lookup result. | Rejects `clear` with `0007`. |

The pre-capture B2 binary accepts all six. Frozen B1 rejects all six, including
three legal cases. Those coarse B1 verdicts do not establish that B1 transports
lookup dependencies precisely. The architecture review below adds explicit twins
and tests the two findings that had only been inferred from code.

The candidate and the isolated B1 backport both match all 29 archived B1
witness receipts. The candidate also matches the pre-capture B2 verdicts
and outputs for the eight archived `w7` and `w8` controls. The isolated B1
backport matches all six lookup controls and the four conflicting scalar
controls. It still rejects the scalar sibling positive at the outer call
because its result carrier has coarse overlap roots. It still rejects both
ambiguous-holder positives. These are validation limits, not evidence that
the prerequisite passed. Leave the unchanged performance guard unrun until
the correctness findings are resolved.

#### Stage 5 architecture review

Treat authored order and conflict precision as separate questions. The Stage 5
lowering correctly stages an ordinary mutable projected alias before a later
prelude and passes a `Place` reload. This fixes an observable timing defect, not
just a diagnostic difference. Frozen B1 and pre-capture B2 accept the delayed
projection in the `observe` reduction above but reject its explicitly pinned
twin. The isolated B1 capture build and the draft reject both at the later
overlapping operand. Named arguments, receivers, alias-backed projections and
both fallible directions agree on that point. The reversed-order and
independent-write controls remain legal. A source `NeverCall` receiver probe
fails `MOTH-RULE-0054` on every compared binary, so it does not validate borrow
transfer. Keep the Stage 5 HIR checks for that path.

The table lists `authored / explicitly sequenced` verdicts where there are
two forms. The sequence binds the same projected place before evaluating the
later operand, without `copy`. For lookup edits without a later prelude, the
explicit form names the receiver alias before editing. `Accept` means `moth
check --terse` passed, not that the program was executed. Draft verdicts refer
to the snapshot that failed the independent review, before restoring the
rebound-holder check. The comparators are the archived B1 release
`b1-define-local-a08f2d820e3e/moth` and the frozen pre-capture B2
`target-strict/debug/moth` from `960cc17ed`.

| Control | Required | B1 | Pre-capture B2 | Capture draft | Classification |
| --- | --- | --- | --- | --- | --- |
| Scalar sibling, no preceding store | Accept both | Reject / Reject | Accept / Reject | Accept / Accept | B1 coarse precision limit. The draft fixes one pair but not the next two. |
| Scalar sibling after `~pair.right = 4` | Accept both | Reject / Reject | Accept / Reject | Reject / Reject | New rejection of the authored spelling against B2. Existing explicit-alias limit. All accept the sequenced read and direct field store without the pin. |
| Shared read of named heap sibling | Accept both | Reject / Reject | Accept / Reject | Reject / Reject | New rejection of the authored spelling against B2. Existing explicit-alias limit. B2 and the draft accept the read followed by direct `push` without the pin. |
| Same scalar field or whole aggregate read | Reject both | Reject / Reject | Accept / Reject | Reject / Reject | Authored capture fixes an existing B2 soundness defect. |
| Ambiguous, all holder alternatives disjoint | Accept both | Reject / Reject | Accept / Reject | Accept / Accept | Existing B1 and explicit B2 precision limits. The B1 port still rejects both. |
| Ambiguous, one arm shares a child | Reject both | Reject / Reject | Reject / Reject | Reject / Reject | Soundness control. Complete possible targets do not establish disjointness here. |
| Live `get`, then edit same receiver | Reject both | Reject / Reject | Accept / Reject | Reject / Reject | Draft repairs the B2 authored gap. Scalar `get` also rejects on all snapshots. |
| `identity(raw)` or `row.get(0)!`, then edit | Reject both | Reject / Reject | Accept / Reject | Accept / Accept | Existing B2 authored defect and new losses of explicit rejection. The identity result and nested lookup must both preserve the outer loan. |
| `~writer = score`, then edit | Reject both | Reject / Reject | Accept / Reject | Accept / Reject | Existing B2 authored defect. The explicit twin rejects *at alias acquisition*, which does not prove that write-through transported the loan. |
| Edit then definitely overwrite unused `score` | Accept both | Reject / Reject | Accept / Reject | Reject / Reject | New authored false positive against B2. All accept the direct edit when moved after the overwrite. |
| `writer ~= items`, then `value = writer.get(0)!`, then use `value` and `writer` | Accept | Accept | Reject | Reject | Existing B2 precision regression against B1. Treat a known lookup as receiver-derived, not as an opaque result that might be fresh. |
| Store `score` in `Saved(score)`, then edit and use `saved.item` | Reject both | Accept / Accept | Accept / Accept | Accept / Accept | Soundness defect already present in B1. Remains when `~score = {0}` retires its original name. Omitting the field use or definitely overwriting the field first stays legal. |
| Use captured `score` only in a CFG jump | Reject both | Reject / Reject | Reject / Reject | Reject / Reject | Not a reproduced defect. `metadata.rs` records the jump read and its in-block order. Dropping the jump use permits the edit. |
| Whole `observe(~slot, replace(~slot))` | Reject overlapping authored accesses | Accept / Reject | Accept / Reject | Accept / Reject | Existing timing defect. A manual alias exposes the conflict but is not a general rewrite for whole slots because it changes the entry role. |
| Live projected alias after holder rebinding | Reject under the accepted B1 rule | Reject | Reject | Accept | New loss of an accepted conservative check. The alias still observes the old field, not the replacement. |

`score` in the aggregate control means `bucket.scores.get("k")!` and
`Saved` has `item {I32}`. The other-holder variant declares `score` mutable
and overwrites it after construction. For the positive unused-field control
give `Saved` a `count I32` field and read only `saved.count`. The jump
reduction uses a `String = I32` map in a function returning `I32, Error!`.
One arm yields `then score, change(~bucket.scores)`, where `change` clears
the map and returns `0`. The explicit arm binds `saved = score` before
calling `change` and yields `then saved, other_value`. Inlining `clear`
also rejects. `then 0, other_value` accepts.

Direct assignment through a named scalar sibling alias (`left ~= pair.left`,
`right ~= pair.right`, `~right = 7`, then observe both) still rejects.
B1 and pre-capture B2 reject the second alias acquisition. The draft permits
that step, then its root-only local update check rejects the write. The
draft accepts `~pair.right = 7` with only `left` live. This exposes an
existing transfer precision limit, not a need for a second capture rule.
The direct field store's singular-ID check does not also check holder
coverage. That remains an encoding proof obligation, not a reproduced
source defect.

For the accepted rebound-holder contract, keep the `write_target_roots`
fallback on shared reads unless the draft can prove disjoint fields in one
common singular holder occurrence. Comparing an old field with a new
allocation cannot discharge that check. This restriction adds no state,
does not test whether a local came from the compiler and does not demand
a singleton for the ordinary multi-target mutation overlap query.

After restoring the fallback, 264/264 borrow checker unit tests,
both rebound-holder fixtures, all six mutable capture fixtures and all
15 footprint and stability controls pass. The six original review
reductions and their sequenced twins retain the verdicts in the table.

##### Lookup dependency lifecycle

The canonical access reference says `get` borrows stored data and prevents
mutation of the receiver while the result remains live. The collection and
map contracts identify the receiver and success channel. Const storage's
`EdgeLocation { owner, index, structure }` explains why neither the
element's allocation nor a local name identifies what the borrow protects.
Const generations validate later concrete reads. They do not replace static
runtime exclusivity or prove that an abstract target set covers all paths.

| Step | Current owner and gap | Consolidation |
| --- | --- | --- |
| Acquire | `CallStorage` and `capture_builtin_element_result` know `Get` and `Remove`. The draft emits `KeepReceiverStable` beside a heap-only `Bind`. `CallLane::derive_result` loses known `Get` derivation when it uses `Opaque(Args([0]))`. | Produce one value snapshot for successful lookup, including receiver-entry dependency, inherited dependencies and the correct permission and derivation. Failure does not acquire it. `Remove` does not acquire a new receiver loan, but preserves dependencies already carried by its payload. |
| Read, alias and project | `resolve_path`/`step_resolved` resolve allocations and permissions. `capture_local_value` copies loans keyed by the spelling's root. A nested lookup sees only its immediate receiver. | Follow the resolved cell and selected value occurrence. A projection inherits dependencies needed to reach that occurrence and retains its own. An explicit `copy` uses the source before creating independent storage. |
| Store, construct and return | `CapturedValue`, `StoredValue`, construction edges and lane results carry allocations and views. `LayerAliasWrite::Whole` and `::Cells` separately mirror only some facts. `return_alias` paths and scalar result transfer can omit an entire loan. | Carry the same snapshot through direct and builtin stores, aggregate edges, call actuals and success/error results. Return a value or retained edge, not a callee wrapper. A write through a cell leaves the wrapper relationship intact. An independent holder of the old value retains the old dependency. |
| Join and transport | Jump transfer already snapshots all incoming values before simultaneously redefining destinations. `join_layer` unions the draft loan rows, independently of value and role alternatives. | Union possible dependencies on the corresponding value alternatives, retain must derivation only where proved and keep a distinct unresolved alternative. Do not use co-occurrence invented by merging to prove disjointness or detachment. |
| Retire | `retire_binding_allocation`, `rename_base` and `narrow_layer` remove or re-key rows by local index. The inclusive `future_use_kind` counts writes and reads of replacement values. `BlockLiveness` already has definition kills and outgoing jump reads. | Reuse CFG liveness and ordered access/definition facts to ask whether this old value can still be observed before a definite kill. Cover reads through another holder, aliases, retained projections, results and pending jumps. A weak update or a write that reads the old value is not a kill. Storage retirement for escape and physical planning stays separate from borrow activity. |

Entry stability, projection cells and heap reachability are different
footprints. Keep one ordinary overlap query with the access surface supplied
by HIR and builtin contracts: cell access or replacement, referent access,
and exclusive receiver or entry-structure access. Replacing a separate
slot that held a map does not by itself mutate that map. An entry edit
through any alias still conflicts with a live lookup. Target equality
alone cannot supply either derivation or mutation authority.

##### Proposed sequence

1. **Keep the HIR evidence, not a transfer checkpoint.** Keep the existing
   `lower_operand_sequence` and the authored-order HIR tests as an isolated
   candidate. Use B1 to compare actual operand order, write-through and
   intentional conflicts. Do not change its conflict model to make the
   isolated build accept additional programs. Do not claim that unchanged
   B1 can implement the scalar and multiple-holder positive controls.
2. **Prove the sparse typed representation without new capture transfer.**
   Use existing sparse allocation identities and selectors. Resolve wrappers,
   typed slot and projected cells, and their value and permission
   alternatives separately. Make the same resolution and write plan serve
   ordinary access and summary extraction, without depending on the legality
   verdict. Reuse the existing materialisation and liveness owners. A
   non-escaping scalar with no observable alias, capture or retained lookup
   keeps the cheap path. Test the representation used by production, not
   just a separate prototype.
3. **Replace summary and transfer together before enabling capture.** Let
   one contract represent lane-specific binding transitions and stored value
   postconditions. Use captured entry actuals, observable escaped cells with
   canonical per-call identities, and a preserve alternative whenever
   replacement is not definite. Keep proven unreachable, pending convergence
   and executable unresolved exits distinct. Compare order permutations and
   independent recomputation for recursion, forwarding, generated functions
   and mixed lanes. Finite port and occurrence identities, may-union,
   must-intersection and validated monotone transfer must imply equal least
   fixed points without iteration limits or history-dependent fallbacks.
   Reuse the metadata dependency queue and invalidate HIR-dependent evidence
   when private failure lanes change the graph. Keep transition validation
   and compare incremental refresh with full reconstruction over the final HIR.
4. **Integrate authored capture with ordinary access.** A projected actual
   fixes its cell and preserves permissions, while a whole mutable actual
   preserves its wrapper role. Validate its exclusive access at the authored
   point without wrapping an independent slot as an alias. Exercise the
   same overlap query for direct source, explicit pins and compiler pins.
   Validate the capture, assignment, escape and effect witnesses on both
   outcome lanes. Keep the conservative rebound-holder witness unless an
   independently reviewed model change explicitly supersedes it.
5. **Run the unchanged gates.** Independent correctness, architecture,
   redundancy, performance and coverage reviews first. Preserve the original
   corpus and B1/B2 witnesses, including assigned-alias store and all nine
   retarget controls. Require Boracle, integrated validation and 18/18 on
   the unchanged release guard. Use the original comparators in one
   measurement session, account for capture HIR counters and report the
   separate int64 comparison against B1. Keep shared C4 runtime HIR lowering
   disabled and its two fixture expectations unchanged until B2 passes.

Delete, rather than keep in sync, the following machinery when its
replacement passes the semantic controls:

- Replace `AliasCell` and the duplicated `LocalState` destination identity
  (`direct_alias_roots`, `write_target_roots`, `designated_cells` and
  `write_storage_cells`) with the sparse wrapper/cell relation. Keep a
  scalar fast-path root bitset if it is a projection of the same facts, not
  another source of destination identity.
- Replace `resolve_place_cell`, `place_cells`, ad hoc coarse root waivers
  and same-call root pairs with one resolved footprint. Keep actual
  must-derivation, not the `reborrow_uses_parent` shortcut from coarse
  write roots. Remove `ResolvedPlace.complete` as a shared certificate
  for both addresses and allocation contents.
- Replace `AllocationLayer::stability`, `KeepReceiverStable`, per-local
  stability copying in `LocalLayerValue::Bound` and the jump side payload
  with the ordinary value snapshot. Retain existing sparse allocation
  edges and correlated sharing evidence where needed. Receiver stability
  is not an ordinary allocation edge or a substitute for the removal
  detachment proof.
- Replace split `LayerAliasWrite` publication and `InvokeLayerOp::StoreCells`
  plus `refresh_alias_referents` with the same typed-cell update. Extend
  `CallOutputPath` and lane facts rather than adding a second effect
  contract. Remove empty-path role inference and lane-wide fallback once
  complete and unresolved alternatives have an explicit representation.

The [Boracle reference solver](../../developer-docs/memory-management/boracle/boracle-reference-solver.mtf)
and [operational oracle](../../developer-docs/memory-management/boracle/boracle-operational-oracle.mtf)
retain these reductions as research evidence. Neither should consume
production legality decisions. Keep the corrected `OpaqueCallStores`
refusal and leave the broader Boracle plan inactive.

#### Entry roles and permissions

Ordinary function signatures don't restrict a mutable formal to one entry
role. Validate the body for every role its signature admits: a slot, a root
alias or a projected alias, including combinations admitted by the exclusivity
and permission checks. Use the same rule for public, private and generated
ordinary functions. A compiler-internal restricted carrier would need a
producer-validated HIR restriction, not inference from the calls seen so far.
For `put |dest ~I32, source I32|: ~dest = source`, an alias entry could write
through with a shared read of `source`. Its existence doesn't excuse the
invalid exclusive acquisition on the admitted slot entry. Invalid access
is a diagnostic, not an unreachable row.

Different parameter indices don't themselves prove disjoint storage.
Exclusive argument checks rule out admitted conflicting presentations, not
all shared values under disjoint wrappers. Check the argument's documented
access footprint and the body operation separately. Cell/edge replacement
checks authority on the containing location. Traversing the old stored
reference to mutate its allocation needs that reference's permission too.
Reuse one resolved place and overlap query, not one interchangeable policy
for every kind of access.

Actual substitution identifies storage but doesn't grant callee authority.
Record the access on each returned or stored path, and intersect a retained
edge's restriction with the actual path's authority. Keep scope-local
binding permissions distinct from permissions which an edge retains.
Respect the accepted raw `return value` contract: the result transports the
input value without the temporary shared formal's wrapper restriction.
Its captured actual authority and any pre-existing retained path
restrictions still apply. The receiving declaration supplies the new
wrapper's permission, not authority to mutate an immutable actual.
`projected_write_through_call_returned_holder_publishes_parameter_mutation`
pins raw transport of a writable actual; `runtime_call_callee_local_binding_mutable`
pins fresh owned output, not permission recovered from a shared input.

In contrast, a new aggregate edge installed from a shared formal keeps the
retained read-only restriction. `wrap |items {I32}| -> Box: return Box(items)`
followed by `actual ~= {1}; wrapped ~= wrap(actual); ~wrapped.items.push(2)`
currently accepts in the draft. Even though the actual is writable, the
edge's read-only provenance must not be promoted on replay. Keep this
paired with both an explicit copy and raw identity return.

#### One postcondition per feasible alternative

The summary is a relation from a typed entry interface to lane postconditions.
Entry observations distinguish wrapper role, immediate place, permission,
terminal location and value provenance as needed by the function. Compute rows
under a consistent equality partition of the observed input places and
allocations, or use an equivalent guarded relation. Specialise a row to its
guard *before* executing its writes. For a mutable `pair` with `pair.left` and
`pair.right` holding a shared child, two writes to `.items` have one location
in the equal-child row and two locations in the disjoint row. The equal-child
row already contains the second write's value. Applying two unordered updates
to equal targets cannot recover it. Unknown equality permits both outcomes,
subject to known inequalities and permission constraints.

Each alternative has a sparse final delta over the projected interface:
changed wrapper links, changed cell contents or erasures, returned **values**
and the reachable internal identities that these facts retain. An exact map
clear explicitly erases both key and element families. Erasing one possible
element from a collapsed family isn't a whole-family erase. Compare changes
against entry facts, not against the state of another exit. Omit a location
only when that row proves `Preserve`. Two possible members of the same family
have the same abstract class but needn't be identical. An omitted location
in an unresolved footprint isn't evidence of `Preserve`. Record uncertainty
with affected typed locations or a conservative permission-bounded footprint.
Keep known changes outside that footprint. A complete empty row denotes a
real return, unlike a lane with no rows.

Compose by substituting a callee's entry observations against the **current**
state and taking its post-state before the next operation. Instantiate each exit
against the same captured predecessor. Apply links, cell values and retained
edges as one postcondition, without evaluating the reads again or imposing order
on the delta's keys. Install receiving destinations afterwards. For example,
two `set_from` operations on a slot can output one new link and a write to its
terminal location. They can't output two independent new links. A write to a
cell and a rewrite of one wrapper can both survive if subsequent operations
observe them through different paths. Result values may retain the allocation
read before a later store even though they don't carry its wrapper.
For an invoke, retire both prior outcome names without deleting their
retained storage. Install only the selected destinations. An unresolved
alternative starts from its own prefix post-state, not from call entry;
suffix uncertainty can weaken that state's facts only within its footprint.
Test that independently on the success and error lanes.

Replay and observation are separate. Keep success `get` stability loans and
keeping uses, including when the result is scalar, across catch and forwarding
where they are retained. `map_get_alias_blocks_set_when_used_after_mutation`
needs this evidence despite the lack of a returned allocation. A failed
lookup doesn't issue the success loan. A successful `remove` instead carries
detached value provenance under its existing contract.

For ordinary value transport, derive the receiving view from the selected
source snapshot and live holder occurrence, not from allocation equality.
Keep that relation through forwarding, receiving writes and re-keying.
Award the exemption only when every feasible source proves derivation.
An opaque alternative can't borrow another row's proof.
`mutable_alias_ancestor_view_read_accepted` reads a result of `identity(child)`
while `child` remains live. Reading via `parent` instead is a foreign access
despite reaching the same allocation.

An executed write that restores the entry state leaves an empty final delta.
A write before a terminal call might not appear on any exit. Preserve
mutation attribution from executed accesses independently of replay, including
the existing terminating-call witness. Retention for optional argument
transfer follows binding links and loans as well as returned allocation
references. Neither equal final contents nor no return implies no write or
no retention.

For a definite replacement, every compatible feasible alternative must cover the
same singleton target with replacement and no unresolved alternative can
preserve it. A row may strongly update a singleton *inside that row*. Joining
complete and incomplete alternatives never retroactively changes how one row
was computed. A weak update retains possible old values of a nonsingleton
target. A sparse encoding can factor alternatives if it preserves their
meaning. In particular, a may-write list without an identity/preserve
alternative is not a complete representation.

#### Escape and recurrence

Export no callee locals by spelling or raw HIR index. Project the reachable
graph from changed caller links and cells and returned allocation values. An
unexposed alias wrapper that can't be addressed and no longer changes role may
be contracted into its immediate target, retaining restrictions and proof of
derivation, if no access or caller entry observation distinguishes it. Never
flatten a link past a slot that can later acquire a different place. A local
name dying removes its name and loans as dictated by liveness, not a wrapper,
cell or allocation that another live handle retains. Keep scalar captured
cells when reached in this graph, even though they have no allocation.

Use a typed, canonical fresh-component identity inside an exit. A canonical
component may correspond to several output occurrences if they designate the
same object. Distinct live exact components of the same type are distinct
identities even with equal contents. Instantiate each component once per
invocation. For separate static call or definition sites keep separate keys.
For repeated dynamic occurrence at a site, age the previous recent component
into a nonsingleton family and re-key all surviving references together before
installing a new recent component. A family can imply possible overlap, not
definite equality or a strong update. Exact current-call sharing survives this
ageing. Never form a key by appending the call stack or an alias path.

Derive a fixed identity vocabulary for each HIR snapshot, before iteration:

- Entry keys are parameter wrappers, selected input terminals and canonical
  type-layout selectors (including one `Elem`/`MapKey` per collection level).
  A boundary link to an input key remains a reference to that actual storage
  on import. Never age or freshen an entry-owned family.
- Allocate exact exported identities only for named output ports that can
  designate a singleton: observable caller wrapper targets, result slots and
  noncollapsed typed paths/locations selected at the boundary. Each port has
  one identity per guarded row. Identical ports share the first canonical port
  in their equality class. Include the private terminal wrapper reachable from
  a caller wrapper port. There can be at most one such terminal per caller
  wrapper after stable internal links contract. The count of exact fresh
  components depends on these ports, not callee summary size.
- Below a collapsed selector, unnamed exported fresh objects use
  nonsingleton invocation-owned families keyed by storage kind and type.
  If the same exact component is also reachable under a collapsed selector,
  record that possible edge alongside the family. Don't export a new exact
  component per member or per recursive return. A recursive function adding
  a fresh collection under `Elem` on each return can add family edges, not
  component keys. A whole-family key alone never proves identity equality.
- Import exact components as recent instances indexed by the current
  operation site and canonical output port. Import fresh families under that
  operation site and their fixed kind/type key, without promotion to recent.
  Export by applying the same port and family quotient to the reachable graph.
  Inside a function use fixed definition and operation sites, both recency
  classes and known layout selectors. Recurrence of a wrapper link to an old
  class forms an edge, not a path segment.

This quotient bounds identities even when the number of reachable concrete
objects exceeds the number of live names. Transfer keeps may-edges of class
members on folding and doesn't let class equality grant a strong update or
an access exemption. No call-depth, operation-count or state-count limit
defines completion. To avoid
allocating graph state for plain scalar code, a HIR index can exclude
functions or operations which cannot introduce an observable link or mutate a
tracked location. Absence of a heap result or heap argument isn't that proof.
An observable escape fact says what can outlive its spelling and requires
semantic lifetime validation under the separate [lifetime topology contract](
../../developer-docs/memory-management/lifetime-regions-and-escape-validation/lifetime-regions-and-escape-validation.mtf).
It neither validates a borrow nor decides stack, heap, region or collection.

#### Convergence contract

Use inclusion of feasible lane alternatives and monotone transfer between them.
For a fixed HIR snapshot, the reference state is the collecting powerset of a
finite typed graph domain. Each direct transfer maps each input alternative to
zero or more outputs independently. An impossible pattern case, terminal
operation or stable proof of no call return contributes no output. During
convergence an unsolved dependency contributes provisional bottom, not that
proof. A reachable operation with missing provenance maps to one or more
conservative alternatives, not zero outputs. Stable unresolved alternatives
come from missing contracts or unmodelled operations. Don't convert transient
recursive bottom into an irreversible opaque atom. If a required producer
cannot prove coverage, carry the conservative alternative across the call
and validate it, or reject the incomplete contract. Never skip a check. Known and unresolved
alternatives can coexist on either outcome lane.

Join by union, prune states only by proven semantic containment and use the
same projection and re-keying for a given input graph regardless of the
worklist order. All destructive updates and recency ageing happen within an
alternative before join. These conditions give monotone transfer on the
powerset, finite height and the same least fixed point on any fair queue.
Extraction and validation share operation classification, not diagnostic
control flow. Extraction evaluates the conservative storage transfer
and retains reachability even if mandatory validation will reject an
operation. Validation checks every admitted entry with final contracts
and rejects publication on failure. An invalid exclusive acquisition
cannot make its lane look non-returning.

For independent effects, the sparse representation should avoid the Cartesian
product of exit deltas. A candidate uses a fixed footprint partition within
each lane and guard: correlate locations in one component when a dependency
or compatible alias can affect target selection, access, a result or coverage.
Treat unknown overlap as a dependency. Partition from the HIR and entry
contract, not from the order in which summary rows arrive. Keep shared guard
facts where independent components use them and join each component by set
union. Avoid generating full worlds for disjoint components. This factoring
doesn't make the worst case of a single correlated component disappear.

Let `C` denote the join normalisation after projecting to this representation.
Require exact factored union or `C(C(A) ∪ B) = C(A ∪ B)` for all batches,
plus monotone transfer on the resulting domain. Monotonicity and sound
coverage by themselves don't justify incremental sparse join: a rule which
adds a may-share row only when it sees two choices arrive in different batches
changes the eventual diagnostic when the caller first sees those choices
together. For fixed product projections, re-expanding their nonempty
components and projecting again adds no component values, proving this
closure law. That proof requires guard compatibility to factor at the chosen
boundary. Show that in the candidate implementation and test incremental vs
batched delivery as well as recursive queue order. Don't use a row limit.
Production access diagnostics check all reachable alternatives using final
contracts. They are not an input to summary convergence or to Boracle.

The reduced collecting prototype used three mutable wrappers, one captured
field, four scalar payload symbols and mutually recursive summary equations.
Both function orders reached the same 69,456 success rows in six visits
and no error rows in two visits. Inclusion held at every replacement. It
also replayed two ordered assignments to a captured field, a complete
identity return alongside a known-effect/incomplete return and shared vs
distinct fresh components before and after ageing. These are checks of the
reference equations, not bounds or measurements for the production
representation. A separate reduction checked that adding up to 999 objects
under a collapsed selector adds no identity keys after entering the family
and that the shared-child guard preserves the last writer. The independent
product reduction checked all 256 ordered batch pairs for two binary
components. Twenty-four independent binary choices needed 48 component
rows, not 16,777,216 worlds. No reduction establishes a production
partition for alias-sensitive writes and transported permissions.

Reuse the metadata queue after private lane installation. Keep signature and
layout contracts that survive the rewrite. Re-index rewritten bodies and reset
graph-dependent postconditions for the rewritten functions and their transitive
callers; keep the unaffected dependencies as seeds. Use the same dependency
edges for invalidation and propagation, including generated functions.
Transition validation applies between facts about the same graph, not across
invalidation. Compare the result against a cold fixed point on the final HIR.

#### Consolidation boundary

- Replace cell identity encoded in `direct_alias_roots`, `write_target_roots`
  and projected `alias_cells` with the resolution of one typed place. Derive the
  slot/alias distinction from its wrapper state rather than maintaining a
  parallel mode. Keep independent facts for initialisation and guaranteed loan
  derivation. Keep old report fields only as projections of the canonical
  state if a consumer still uses them.
- Allocation children still belong to the sparse allocation graph. Replace the
  special empty-path `Store` interpretation, `InvokeLayerOp::StoreCells` and
  caller refresh from root guesses with the same location/value representation
  for immediate calls and invoke edges. Root and projected cells differ in
  identity but don't need separate store effect machinery. Keep existing result
  sharing facts only where they express a correlation the postcondition can't
  derive directly.
- Replace `structural_only`'s whole-lane fallback and the post-graph
  reachability scan with per-alternative coverage and typed unknown locations.
  Scalar calls with no observable transition, and known scalar value-only
  writes where all observable storage facts stay unchanged, keep their cheap
  structural path. Never infer that contract from the result type.
- Reuse the public summary validation boundary for types, selectors, permissions,
  stable IDs, lane coverage and substitution. Boracle may consume an explicit
  semantic fact, not a production borrow acceptance decision. Its current
normalised form cannot represent caller-visible wrapper transitions, the
associated store footprints or their uncertainties. An alias to a caller
field needs a refusal just as an escaped scalar cell does. Dispatch on the
selected outcome's contract rather than payload type or fresh escape.

The fixtures are registered under `language.borrows.call_binding_effects_preserve_escaped_cell_identity`,
`language.borrows.call_binding_effects_compose_in_execution_order` and
`language.borrows.call_binding_effects_preserve_mixed_exit_stores`.
Each has a called rejection, an inlined rejection and a called positive
control. The scalar positive also checks distinct invocations of one helper
with the same local initialiser. The mixed-lane positive passes an explicit
copy, so the old caller collection stays independent. Use `moth tests
--contract <id>` for each trio, or `moth boracle
tests/cases/<id>/input/@entry.moth --dump differential` with the feature
enabled. The baseline B2 draft passes the six controls and accepts the
three negative calls. No B2 checkpoint, final release guard or C4 cutover
follows from these runs.

### Caller-cell stop

The draft uses the existing `Store` endpoint. An empty target path replaces the
mutable actual's cell. A nonempty target path writes into the allocation the
actual held at entry. The path inside the stored value has the same meaning
for both. Capture argument values and destinations before replaying a lane.
Replaying a may-store keeps the old value as an alternative. The presence of
a store row does not prove that every returning execution wrote.

**Correctness gate failed on place-valued assignment.**
`tmp/bc06-review/b2/param_place_assignment/@entry.moth` calls the same function
with a whole slot and with a field. `~dest = source` followed by
`~dest = {3, 4, 5}` renders `3 3 / 3 2` on B1. The whole actual becomes an
alias of `source`, so the second write reaches `source`. The projected
actual already writes through, so it changes the captured field and leaves
`source` alone. This distinction comes from `__moth_param_binding` and
`__moth_assign_borrow`, not from allocation sharing.

`tmp/bc06-review/b2/param_retarget_live/@entry.moth` observes that new alias
after the call. B1 and the B2 replacement prototype both accept and render
`3`. The direct assignment control in `local_retarget_live` rejects the
shared read at `4:10` with `MOTH-BORROW-0003`. A cell may hold the same
allocation as another cell without aliasing that cell. Treating retargeting
as a value store loses the difference on the next read or write. Preserve
the observed runtime distinction unless the language owner approves a
different contract. A sound summary must identify the possible destination
wrapper, the source cell and whether the destination was already an alias,
including when a local cell escapes through the retargeted wrapper.

```moth
set_from |dest ~{I32}, source ~{I32}|:
    ~dest = source
;

first ~= {1}
second ~= {2, 3}
set_from(~first, ~second)
observed = second
~first = {4, 5, 6}
~[:[observed.length()]]
```

Replace `set_from(~first, ~second)` with `~first = second` for the direct
control.

Allocation extraction may defer a path with an unresolved holder rather than
publish a store that the next iteration would retract. Mandatory borrow
validation cannot treat that graph bottom as dead control flow. The draft
distinguishes structurally possible returns from allocation evidence. Final
review must also cover a lane with one complete exit and another unresolved
exit. Nonempty rows alone don't prove that the lane includes every effect.

Boracle source mode still stops before production convergence. It can
report opaque stores as inconclusive without using the checker verdict.
Modelling caller-cell transitions in its normalized graph remains a separate
research task.

The saved draft passed 21 allocation-summary tests, both hard-dead call
controls, the positive caller-cell fixture, the original skipped-continuation
rejection and the assigned-alias-store rejection. `moth check docs --terse`
passed. Logs are in `tmp/bc06-review/b2/gates/caller-cell-focused.log` and
`caller-cell-retarget.log`. The correctness failure stopped the final
independent reviews, Boracle, the release guard and integrated validation.
The draft is not a B2 checkpoint.

### B2 resume stop

**Correctness gate failed.** `tmp/bc06-review/b2/whole_param_replacement/@entry.moth`
starts with an empty `boxes`, fills it through `seed(~boxes)`, removes the
first box, and writes through `removed.items`. B1 rejects the subsequent
conflicting `~child.push(2)` at `26:6` (`MOTH-BORROW-0007`); resumed B2
accepts and the generated JS renders `result=2`. The comparison and
runtime output are in `tmp/bc06-review/b2/gates/resume-whole-param.log`.
The instrumented B2 binary's SHA-256 is
`01347dbebb6c06568ea1d587f24bf660a2ccc7950566d78cf242d66e4a379105`.

A whole assignment to a mutable parameter replaces its graph-local
cell but publishes no caller-cell store. The wrapper then analyses the
collection as empty and the later field write sees an unresolved holder.
It publishes an unreachable success lane, and the diagnostic pass skips
the real continuation. The holder rule must not turn an unmodelled
store into proof that an edge cannot execute. A correction must
distinguish replacement of the caller's cell from mutation of its old
referent and preserve local rebinding, both lane effects and recursive
transition validation.

**Retained candidate.** Private failure-lane installation consumes
signature and failure facts only; the existing metadata queue extracts
allocation facts on final HIR and refreshes affected callers without a
provisional whole-module allocation pass. Cold recomputation tests
cover local and generated recursion, handler pruning, and stores on
both lanes. The review also found and corrected a missing fresh
`Error.message` path. Direct assignment accepts proven-disjoint sibling
cells; overlapping projections and live whole-aggregate observers still
reject in the focused controls. The assigned-alias store regression
still rejects in its focused check.

Boracle remains independent. B1's generic bootstrap guessed `Fresh`
without body evidence; source mode correctly reports `MissingSummary`.
A mutable heap argument without a retained contract and collection
`push` can still be explicitly inconclusive. The terminating-call
control uses normalized call facts, and a push-free source control
retains the self-update coverage. The 266-test Boracle library lane
passed before the final typed-source changes; the resultless scalar
mutation control passed afterward. The complete final recipe was not
verified.

The resumed 18-input release guard, the paired B1 timings and separate
int64 comparison, and final integrated validation were not run after the
correctness failure. No B2 commit. The uncommitted candidate is
preserved under `tmp/bc06-review/b2/handoff/`.

### Slice 2 stop

**Candidate.** Uncommitted on top of `f2390602e`. The measured release
build (`detailed_timers,benchmark_counters`) has SHA-256
`8872167ac31d8a633137b4bca7ec5cebe74b73228bb5e94516b30942233ac3b1`.
Formatting and two Clippy signature fixes landed after that build and
don't change behaviour. `tmp/bc06-review/b2/gates` holds every log.

**Monotone summary transfer.** Summary extraction separates five cases.
A pending or unreachable callee lane binds nothing, so the path stays
incomplete until widening resolves it. Unknown provenance is an opaque
origin and never supports a strong update. A definite write, erase,
push or map update through a holder that no allocation reaches yet
leaves the path incomplete. It never becomes a no-op. Ambiguous alias
targets, callee store rows and opaque stores are may-writes, so an
absent alternative adds nothing. Strong updates need an exact origin:
one source, no repeated site, and no element or key selector on the
cell, the parameter path or the fresh output route (a store target's
route included). A plain mutable slot that receives a place becomes
that place's alias, as at its definition. A control-flow join keeps the
slot as an alternative, and each projected alias keeps its own holder,
keyed by acquisition site. Definition-order tests over recursive,
mutual and late-resolving callees reach identical summaries.

**Functional gates.** The library suite passed 7,926 tests. The Clippy
and formatting gates pass. The integration gate passed 3,920 of 3,922
cases. The two failures are the known C4 cases
`runtime_call_fresh_box_forward_rejected` and
`runtime_call_identity_fields_rejected`. All 29 B1 witnesses match the
recorded set. The Slice 2 row passes: W7a and W7c render
`left=2 right=1 original=1` for both flags, `runtime_call_disjoint_result_fields`
accepts, and W7b, W7d and both store fixtures reject.

Against B1, 8 of the 3,323 original inputs change:

- `runtime_call_disjoint_result_fields` now accepts, as the Slice 2 row requires.
- `runtime_call_shared_sibling_place_operand_rejected` now accepts and
  renders `12`, under the approved W7a decision. It's renamed to
  `runtime_call_shared_sibling_place_operand`. A direct assignment
  `~pair.right = {3, 4}` with a live `left = pair.left` still rejects on
  B1 and on the candidate. That inconsistency stays open.
- The three `mutable_projected_alias_*` pairs now report `MOTH-BORROW-0007`
  when the mutable projection is taken (`19:22`, `19:11` and `23:11`),
  not `MOTH-BORROW-0003` at the later read. `snap` may alias the holder
  on the joined path, so the first conflicting access moves earlier.
  The rejections still depend on the join keeping that alternative.

New fixtures cover recursive and mutual holder writes, an assignment
that acquires an alias, and a callee-local binding that must not
restrict the caller. B1 accepts `runtime_call_assigned_alias_store_rejected`,
which the candidate rejects at `14:6`.

**Accepted imprecision.** Nested sharing that isn't transitive, such as
A–B on one exit and B–C on another inside one value, flattens child
projections and can report false A–C sharing. This only causes false
rejections, and no test asserts acceptance.

**Release guard.** These rows fail against the archive limits:

| Input | Check ms (limit) | Borrow ms (limit) | RSS MiB (limit) |
| --- | --- | --- | --- |
| `speed-test` | 12.645 → 18.400 (15.806) | 3.837 → 8.644 (6.755) | 21.406 → 23.203 (30.758) |
| int64 (extra) | 10.697 → 22.203 (13.372) | 6.236 → 17.120 (10.354) | 21.094 → 30.406 (30.367) |

B1 measured 13.675 and 13.398 ms check time on the same two inputs in
the paired run. Summary changes and comparisons match B1 on both. The
extra time sits in `frontend.borrow.converge`. Checked arithmetic gives
a private function an error lane, so lane installation rewrites the
base module and schedules a summarising visit. B1 also refreshed after
installation but didn't count it, which explains the counter moving
from 0 to 1. The refresh is needed because the error lanes widen. The
avoidable cost is re-extracting every function's summary from scratch.
Reusing the local metadata queue, seeded with the previous summaries
and the rewritten functions, would remove it. That tuning is past the
guard, so the slice stops here.

**Boracle.** Three source tests fail:

- `boracle_source_generic_call_exposes_generated_typed_boundary`: source
  mode snapshots HIR before convergence, and the generated call's result
  origin is now `MissingSummary`, not `Fresh`. The likely cause is that
  the seeded generated summary stays pending there now that the
  constructed-return special case is gone. This isn't yet confirmed.
- `boracle_source_never_call_records_arguments_before_terminal_exit` and
  `boracle_source_self_update_preserves_slot_before_a_different_place_update`:
  normalised calls now carry `opaque_stores`. Source mode passes no
  local summaries, so an unknown callee with a `~` argument, or a
  mutating collection builtin, makes the oracle return
  `Inconclusive(OpaqueCallStores)`. B1 assumed those calls stored
  nothing.

**Slice 1 pause.** The sparse draft accepts W1c and renders `20`. Its
dynamic-index control renders `20 / 21`, while W1 and W1b still reject
the push at `8:15` against `owner`. The success edge empties the
singleton group and the join keeps the detachment only when both lanes
prove it. The option-binding witness now rejects at `18:22`.

An intermediate build passed 208 borrow-checker unit tests. A later CLI
build changed two verdicts across the original 3,323 inputs: the approved
`runtime_detached` rejection at `46:15` and the hashmap rejection at
`21:2`. The draft splits the removal test into a rendering case and an
exact-site rejection. Those tests and both hashmap backend expectations
passed on that build.

The instrumented prototype before the detachment and copy changes passed
all 18 guard workloads with the same recorded counters as `7cc05c61b`.
`tmp/bc06/measure_s6_slice1.json` holds the samples and counters. The
extra int64 case, outside the guard, exceeded the check-time threshold.
The latest draft hasn't run the guard, Boracle or integrated validation.
Don't treat the prototype measurements as a Slice 1 checkpoint.

**Resumed sparse candidate (base `ca4184d043`).** The 210 borrow checker
tests include duplicate-child removal from a collection and from a map,
and a copy that preserves a shared child reached by a lazy field. The
Slice 1 witness row passed on the draft. The original corpus produced no
new accepts. The hashmap fixture changed to the approved rejection at
`21:2`. Splitting the removal fixture retained its positive and moved
the approved rejection to its own case at `8:15`.

The capped candidate passed all 18 release guard workloads and the extra
int64 case. No refusals. Counters matched the recorded baseline. The
candidate SHA-256 is `563f21ea784aadd53c371db483c682ac00e7275ad1664b777103fda4e5cc5087`.
`tmp/c4/resume.DaDFcBSG/slice1_guard_measured.json` holds the comparison.
These measurements are not a checkpoint for a cap-free checker.

`just boracle` passed the ordinary borrow-problem and last-use suites,
the Boracle library suite, and the CLI suites. The previously recorded
witness disagreements on W3 and L4 are separate from this recipe.

`just validate` passed formatting, Clippy and the workspace unit tests.
The integration gate passed 3,895 of 3,898 backend cases. The two
C4 runtime rejects (`runtime_call_fresh_box_forward_rejected` and
`runtime_call_identity_fields_rejected`) unexpectedly accepted.
The Slice 2 positive `runtime_call_disjoint_result_fields` rejected at
`10:12` with `MOTH-BORROW-0007`. Their expectations are unchanged.

**Convergence gate.** `tmp/c4/resume.DaDFcBSG/opaque-cycle` uses the
ordinary function `wrap |items {I32}| -> Link`, which returns a new
`Link(items)`. A loop repeatedly assigns `~cursor = wrap(cursor.items)`.
The capped draft renders `1`. An isolated build with the numerical
projection-depth cap removed did not finish checking in 12 seconds.
The documented Stage 5 comparator `7cc05c61b` accepts the loop
without errors or warnings.
The alternative witness using a recursive struct and choice is
rejected earlier with `MOTH-RULE-0033`, so the recurrence does not
require recursive source data.

Opaque `AliasParams` binds the `Link` result to the collection argument's
allocation identity. The next iteration projects `items` from that
collection as if it were a `Link`, creating a longer path to nonexistent
storage. Removing the identity instead would lose real retained-child
provenance. A type-derived depth threshold does not repair that fact.

The owner moved unconditional result-root and retained-field facts into
Slice 1. For `Link(items)`, the callee's construction proves the `Link`
root is fresh and `result.items` retains the actual `items` allocation
with its existing access permission. Types constrain legal projections;
they don't prove freshness or sharing. Use stable abstract allocation
sites across worklist visits. If exact facts are unavailable, keep
possible sharing in a finite typed graph. Conditional sharing across
exits, per-lane stores and the remaining call summaries stay in Slice 2.

**Cap-free candidate (base `f4d191d662`).** The reduced loop checks
without the cap. The registered positive also exercises a callee with
disagreeing exits and renders `1 / 1 / 2`, and the permission negative
rejects at `10:13` with `MOTH-BORROW-0007` (`immutable_place`). The
Slice 1 witness row and affected backend manifests passed. The 3,323
original inputs had no newly accepted program and only the approved
hashmap rejection at `21:2`.

The release guard passes **16 of 18** inputs. The binary was built with
`detailed_timers,benchmark_counters` (SHA-256
`54e2dd2a8165949b23adec89645b0d83d182d816743428fdd880f7ba2b4704d9`).
`tmp/c4/resume.DaDFcBSG/typed_b1_guard_trial.json` records matched
archive, baseline and candidate samples. No refusals; the counters match
`7cc05c61b`, and the separate int64 input passed.

| Workload and metric | Archive | Guard | Candidate |
| --- | ---: | ---: | ---: |
| `aggregate-forwarding-128` check | 16.195 ms | 20.243 ms | 24.393 ms |
| `aggregate-forwarding-128` borrow | 8.135 ms | 13.203 ms | 15.960 ms |
| `narrow-chains-128` peak RSS | 59.734 MiB | 78.668 MiB | 81.156 MiB |

The forwarding workload repeatedly calls generated `identity` on
collections and maps. Its unproven result goes through the new opaque
call graph. That graph and the per-result retirement scan are leads to
profile, not established explanations for the overruns. The audit also
found an independent error: an exact option payload result can name an
argument that the coarse `Fresh` summary caused transfer not to capture.
Resume with that error tracked; do not accept the slice on the earlier
capped guard or relax the existing thresholds.

The sparse draft remains uncommitted. `f4d191d662` checkpoints the
independent C4 import and const materialisation path only. Shared
runtime lowering is off.

**Repaired cap-free candidate (base `f4d191d662`).** The release compiler
`0c33f7115017f206a7b7b51eed5adf417591ce5541da1e0161566f23d99d2513`
keeps option payload provenance and argument permissions, preserves
possible stored children when an exact result is unavailable, and
distinguishes weak destination updates from the writer's replacement.
`tmp/bc06-review/repair-status.md` records the candidate sources,
review outcomes, and gate receipts. The 222 owning Rust tests, 29
source witnesses, nine affected backend fixtures and `just boracle`
passed. The original 3,323 inputs have only the approved hashmap
rejection and no new acceptances. `just validate` passed 3,900 of 3,903
integration cases, with the same three later-owned failures above.

The repaired full guard passes **16 of 18** inputs, with check-time
misses on `origin-fanout-128` (12.632 ms / 9.950 ms) and
`aggregate-forwarding-32` (21.551 ms / 19.794 ms). The separate int64
case misses check time (12.934 ms / 12.166 ms). A focused replay did
not reproduce the two formal misses, but did reproduce the previous
`aggregate-forwarding-128` miss. The paused profile attributed
5.96 ms to allocation retirement, including 3.47 ms in the observer
scan, versus 0.24 ms in opaque result helpers. The narrower scan
reduced paired borrow time from 46.492 ms to 43.780 ms; the paired
limit was 37.223 ms. `narrow-chains-128` passed the repaired RSS gate.
The old RSS miss was not explained by measured live allocations:
the dominant dense state and HIR allocations were unchanged.

Do not checkpoint B1 against the capped result or the focused replay.
Retirement elision needs proof that no old root, sparse descendant or
captured value is observable, including at repeated definitions. Keep
the cap-free design, existing thresholds and later fixture expectations.

**Retirement-elision trial (base `f4d191d662`).** The owner approved
elision at heap call definitions only when entry-state facts prove
observable equivalence. The release compiler `c9cb1ba7c051` captures
the incoming result once and keeps normal retirement on all
inconclusive paths, including loop revisits. Invoke keeps its
existing edge transfer. The paired loop control rejects the write
to the first retained child at `21:23` and renders
`iterations=2 before=1/1 after=1/2` for the independent second child.

The 222 owning Rust tests, 29 B1 witnesses and 11 affected backend
fixtures passed. The original 3,323 inputs retain the same lone
approved hashmap rejection and no new acceptances. The unchanged
release guard passes **16 of 18**, with identical Stage 5 counters.
`aggregate-forwarding-128` misses check (23.110 / 20.807 ms) and
borrow (14.007 / 13.259 ms). `sibling-loops-64` misses peak RSS
(102.094 / 99.664 MiB). The extra int64 case misses check time
(13.870 / 12.986 ms) outside the guard.

`just boracle` passed. With the default timing environment,
`just validate` passed formatting, Clippy, audits and workspace
tests and reached 3,902 of 3,905 integration cases. The three
failures remain `runtime_call_fresh_box_forward_rejected`,
`runtime_call_identity_fields_rejected` and
`runtime_call_disjoint_result_fields`, all owned by later slices.
The standalone docs check passed.

The exact-binary replay reproduces the aggregate miss. Its
remaining sampled retirement (3.19 ms/run) comes from assignment
transfer, not call-result transfer. The replay does not reproduce
the sibling-loop RSS miss. That workload has the same live malloc
peak before and after elision, and the dominant dense state clones
use the same 50,380,800 B as the archive. Do not change storage or
relax the guard on this evidence. See
`tmp/bc06-review/retirement-status.md` for the fingerprint, all
receipts and the attributed profiles. No B1 checkpoint.

**Ordinary heap definition candidate (base `0e95895fd5`).** The
pre-edit census on `aggregate-forwarding-128` found 1,534 paid
retirements across 767 eligible ordinary sites, with 773 additional
executions already using an empty sparse layer. The shared proof
now covers the first heap definition in a predecessor-free entry
block. It checks the incoming state and skips retirement only if
capture did not materialise the old root. Loop revisits, `Update`
and `Invoke` keep the original transfer. Tests compare state and
published facts with unconditional retirement, keep an old projected
child across re-entry and exercise the post-capture veto.

The release build `a08f2d820e3ed` passes the unchanged guard **18 of
18**, with matching Stage 5 counters. In a separate paired replay,
`aggregate-forwarding-128` check time fell from 22.234 to 18.767 ms
and borrow time from 13.945 to 10.634 ms. The int64 case, outside the
guard, still misses check time (12.916 / 12.373 ms). Observed
`sibling-loops-64` spikes track private dirty allocator pages even
as live bytes fall; they do not justify a storage redesign.

All 225 owning Rust tests, 29 witnesses and 11 affected backend
fixtures passed. Across the original 3,323 case IDs, only
`hashmap_alias_struct_field_param_return` changed, with the approved
rejection at `21:2`. `just boracle` and the standalone docs check
passed. `just validate` reached 3,902 of 3,905 integration cases and
exited 1: `runtime_call_fresh_box_forward_rejected` and
`runtime_call_identity_fields_rejected` still accept (C4);
`runtime_call_disjoint_result_fields` still rejects with
`MOTH-BORROW-0007` at `10:12` (Slice 2). The expectations are unchanged.
See `tmp/bc06-review/define-local-status.md` for the exact build,
measurements and audit evidence.

## 1. Evidence inventory

The source witnesses and diagnostic locations below use the Alpha grammar of the measured revisions. The syntax migration preserves their required access, provenance and output behaviour.

Every comparison below names its binary. "Reproduced" means re-run this session.

| Label | Compiler | Fixture tree | Status |
| --- | --- | --- | --- |
| Baseline | Release `moth-cli` built from a detached worktree at `7cc05c61b` | `tests/cases` at `7cc05c61b` (3,323 inputs) | Reproduced |
| Candidate | Release `moth-cli` from `7cc05c61b` plus the holder-edge diff | Same tree. The diff touches no fixture. | Reproduced |
| Archive | The retained `0f384a5539` release checker, the BC-06 [committed baseline](./production-borrow-checker.md#committed-baseline) | Witness sources only | Reproduced on the witnesses. Its timing and corpus numbers stay historical. |
| Proof receipt | `e235a04c5` documents the HIR-shaped proof | None | Historical |
| Value-root prototype | The reverted constructor value-root union, compared with a Stage 5 recording of 3,315 inputs | Pre-`7cc05c61b` tree | Historical. The comparison names the four positives the holder edges were meant to restore. |
| Factored cutover | Abandoned replacement, archived under local `tmp/bc06/cutover/final-archive/` | `0f384a5539` comparator | Historical |

The candidate's Rust unit tests don't compile. Six `LocalState` literals in `tests/borrow_checker_update_tests.rs` and `tests/state_tests.rs` lack the new `stored_roots` and `stored_field_roots` fields. No unit-test result exists for the candidate.

### Corpus delta

The comparator records `moth check <case>/input --terse` for every case and compares the error multiset (code, file, line, column) and the exit status:

```bash
python3 tmp/bc06/corpus_verdicts.py record --compiler <baseline-moth> --out tmp/bc06/corpus_7cc.json
python3 tmp/bc06/corpus_verdicts.py record --compiler target/release/moth --out tmp/bc06/corpus_holder_candidate.json
python3 tmp/bc06/corpus_verdicts.py compare tmp/bc06/corpus_7cc.json tmp/bc06/corpus_holder_candidate.json
```

Three of 3,323 cases change. A full comparison of every error line, including message text, finds no other difference. Nothing is newly accepted.

| Case | Baseline | Candidate | Classification |
| --- | --- | --- | --- |
| `hashmap_alias_struct_field_param_return` | Accepts | `0007` at 21:2, "Cannot mutably access `scores` because `p` may alias the same value." | Expected rejection |
| `const_function_alias_remove_mutation_parity` | Accepts | `0007` at 46:15, "Cannot mutably access `detached` because `owner` may alias the same value." | Expected rejection of `runtime_detached` |
| `const_function_projected_alias_composition_parity` | Accepts | `0007` at 48:21, "Cannot mutably access `retained` because `root` may alias the same value." | False rejection of `runtime_retained` |

`moth check` only compiles each case's configured default backend. The hashmap manifest also expects `MOTH-RULE-0064` (`unsupported_backend_feature.fixed_width_scalar_values`) on `html_wasm`. The baseline reaches that capability check. The candidate stops at the borrow error first, so `moth tests --case hashmap_alias_struct_field_param_return --backend html_wasm` reports a code mismatch. A backend-aware corpus comparison needs `moth tests` per declared backend, not `moth check`.

### Restored positives and unsound controls

The value-root prototype rejected six positives. The holder edges restore three outright and move the other three to the sites above.

| Case | Value-root prototype | Candidate |
| --- | --- | --- |
| `adversarial_struct_collection_result_interop` | `0007` at 12:2 | Accepts |
| `uint_map_key_success` | `0007` at 31:16 | Accepts |
| `uint_map_key_release` | `0007` at 31:16 | Accepts |
| `const_function_alias_remove_mutation_parity` | `0007` at 45:18 (the `remove`) | `0007` at 46:15 (the push), approved |
| `const_function_projected_alias_composition_parity` | `0007` at 47:17 (the replacement) | `0007` at 48:21 (the push), false rejection |
| `hashmap_alias_struct_field_param_return` | `0007` at 21:2 | Same, approved |

The two C4 fixtures fold `pair #||`. Their ordinary twins replace it with `pair ||`. An earlier report claimed that the candidate fixes "both ordinary unsound controls". Only one was unsound on the baseline:

| Source | Baseline | Archive | Candidate |
| --- | --- | --- | --- |
| `runtime_call_fresh_box_forward_rejected`, ordinary twin | Accepts, renders `2` | Accepts, renders `2` | `0007` at 15:6 |
| `runtime_call_identity_fields_rejected`, ordinary twin | `0007` at 15:6 | `0007` at 15:6 | `0007` at 15:6 |
| Both committed `pair #||` fixtures | Accept, render `1` | Not run | Accept, render `1` |

The const results render `1` because the baseline materialises each result slot as its own allocation. Accepting them is consistent with that runtime output but contradicts the approved C1 publication contract. They become real conflicts only after C4 publishes shared allocations.

The candidate also rejects these unsafe accepts from the baseline. The first three render a mutation of storage that another live path observes. The fourth mutates through an immutable parameter. The fifth replaces `holder.items` through `replace(~holder)` while an option still aliases the old field:

| Source shape | Baseline render | Candidate |
| --- | --- | --- |
| `box = Box(shared)`, `~shared.push(2)`, read `box.items` | `2` | `0007` at the push |
| The same constructed in both arms of a value `if` with multi-bind | `2` | `0007` at the push |
| `box ~= Box(shared)`, `~box.items.push(2)`, read `shared` | `2` | `0007` at the push |
| Immutable `items` stored in writable `Box(items)`, then `~box.items.push(2)` | `2` | "Cannot mutably access immutable local `items`." |
| `maybe {I32}? = holder.items`, then `replace(~holder)` while `maybe` is live | `10` | `0007` at the call |

## 2. Witness set

Each witness below is a one-function scratch extraction with the fixture's spans preserved. The `@config.moth` is `project #= (name = "test_project")` and `html #= ()`. Run them with:

```bash
for d in tmp/bc06/witness/w*/; do bash tmp/bc06/witness/run.sh <moth-binary> "$d"; done
```

`run.sh` prints the first `--terse` diagnostic, or builds with `--release` and evaluates the emitted scripts in Node.

| Witness | Required | Baseline | Archive | Candidate | Class on candidate |
| --- | --- | --- | --- | --- | --- |
| W1 `runtime_detached`, `remove` succeeds | `0007` at the push (fixture 46:15) | Accepts, `220` | Accepts, `220` | `0007` 8:15, names `owner` | Expected rejection |
| W1b `runtime_detached`, `remove(5)` takes the catch | No conflict on this path. The static verdict follows W1. | Accepts, `121` | Accepts, `121` | `0007` 8:15 | Expected (path-insensitive) |
| W1c W1 without `owner` | Accept, `20` | Accepts, `20` | Not run | `0007` 7:15, names `entries` | False rejection |
| W2 `const_detached` | Accept, `220` | `220` | `220` | `220` | Unchanged C2/C3 control |
| W3 `runtime_retained` | Accept, `22` | Accepts, `22` | Accepts, `22` | `0007` 13:21 (fixture 48:21) | False rejection |
| W3c `const_retained` | Accept, `22` | `22` | `22` | `22` | Unchanged control |
| W4 negative twin of W3 | `0007` at the push | Accepts, `22` | Accepts, `22` | `0007` 13:21 | Right verdict for the wrong reason |
| Hashmap, `html` | `0007` at 21:2 naming `p` | Accepts, `linus=7` | Not run | `0007` 21:2 | Expected rejection |
| Hashmap, `html_wasm` | Manifest decision pending | `MOTH-RULE-0064` | Not run | `0007` 21:2 | Stage 6 now runs first |
| `runtime_call_disjoint_result_fields` | Accept, `left=1 right=2 second=1` | `0007` 10:12 | Not run | `0007` 10:12 | False rejection (deferred precision) |
| W7a forwarded `select`, copied, disjoint arms | Accept, both flags `left=2 right=1 original=1` | `0007` 19:12 | `0007` 19:12 | `0007` 19:12 | False rejection |
| W7b W7a with `return Pair(a, b)` in the first arm | `0007` at the push | `0007` 19:12 | `0007` 19:12 | `0007` 19:12 | Right verdict for the wrong reason |
| W7c W7a through one value `if` join and one return | Accept | `0007` 21:12 | `0007` 21:12 | `0007` 21:12 | False rejection |
| W7d W7c with the shared arm | `0007` at the push | `0007` 21:12 | `0007` 21:12 | `0007` 21:12 | Right verdict for the wrong reason |
| W8 C4 fixtures and ordinary twins | `0007` at 15:6 for all four | See [above](#restored-positives-and-unsound-controls) | | | |

None of the three compilers lets the select witnesses run, so the required renders come from the earlier Node runs recorded in the [BC-06 evidence controls](./production-borrow-checker.md#evidence-and-acceptance-controls).

### Reduced sources

W1 is lines 41-49 of `const_function_alias_remove_mutation_parity/input/@entry.moth` (line offset 38). W2 is the same body declared `const_detached #||`. W1b changes `remove(0)` to `remove(5)`.

```moth
Owner = |items {I32}|

runtime_detached || -> I32:
    child ~= {1}
    owner ~= Owner(child)
    entries ~= {Owner(child)}
    detached ~= ~entries.remove(0) catch then Owner({0})
    ~detached.items.push(2)
    return owner.items.length() * 100 + detached.items.length() * 10 + entries.length()
;

~[:[runtime_detached()]]
```

W1c drops `owner`, keeps the removal and renders `20`:

```moth
runtime_detached || -> I32:
    child ~= {1}
    entries ~= {Owner(child)}
    detached ~= ~entries.remove(0) catch then Owner({0})
    ~detached.items.push(2)
    return detached.items.length() * 10 + entries.length()
;
```

W3 is lines 3-9 and 44-50 of `const_function_projected_alias_composition_parity/input/@entry.moth` (offset 35 for the function). W3c declares it `const_retained #||`. W4 replaces line 12 with `~root.inner = Inner(root.inner.values)`. The replacement installs a fresh `Inner` that still holds the original collection, so `retained.inner.values` and `root.inner.values` share it again. W4 renders `22` on the baseline, the same as W3, so only the verdict separates them.

```moth
Inner = |
    values {I32},
|

Outer = |
    inner Inner,
|

runtime_retained || -> I32:
    root ~= Outer(Inner({1}))
    retained ~= Outer(root.inner)
    ~root.inner = Inner({3, 4})
    ~retained.inner.values.push(2)
    return retained.inner.values.length() * 10 + root.inner.values.length()
;

~[:[runtime_retained()]]
```

W7a is the `select`/`forward`/`probe` source in the [BC-06 evidence controls](./production-borrow-checker.md#evidence-and-acceptance-controls). W7b replaces `return Pair(a, {2})` with `return Pair(a, b)`. W7c and W7d replace the body of `select` with one join:

```moth
select |flag Bool, a {I32}, b {I32}| -> Pair:
    chosen Pair = if flag:
        then Pair(a, {2})
    else
        then Pair({3}, b)
    ;
    return chosen
;
```

### Event traces on the candidate

The traces read the lowered HIR from a `show_hir,show_borrow_checker` debug build and the candidate's transfer rules. Runtime columns name the cell and the allocation it reaches. Allocations carry a number in order of creation.

**W3 and W4.** `Inner#1` holds collection `C`.

| Line | HIR event | Runtime state | Candidate fact |
| --- | --- | --- | --- |
| 10 | `root` defined from `Outer { inner: Inner { values: [1] } }` | `root.inner → Inner#1 → C` | `root` has no holder edges |
| 11 | `retained` defined from `Outer { inner: [v5] root.inner }`, a projected load | `retained.inner → Inner#1` | `stored_field_roots(retained) = {root}` |
| 12 (W3) | Assignment to place `root.inner` from `Inner { values: [3, 4] }` | `root.inner → Inner#2 → D`. Nothing in `root` reaches `Inner#1` any more. | `FieldCell` access skips field holders at the first hop, so the write is accepted. The edge `retained → root` stays. |
| 12 (W4) | Assignment to `root.inner` from `Inner { values: root.inner.values }` | `root.inner → Inner#3 → C` | Same as W3, plus the projected load of `root` |
| 13 | `~retained.inner.values.push(2)`, projected mutable receiver | W3: mutates `C` only. W4: mutates `C`, still reachable from `root`. | `children_of(retained) ∋ root`. `root` is live at line 14, so the push conflicts. |
| 14 | Reads `root.inner.values` | Last keeping use of `root` | |

The first faulty event is line 12 in W3: the candidate keeps an obsolete relationship. Its edge names the holder root, not the cell `retained.inner` or the allocation `Inner#1`. An exact replacement of `root.inner` can't retire it. W4's rejection comes from the same stale edge, not from the re-shared `C`. Its diagnostic is identical to W3's.

**W1 and W1c.**

| Line | HIR event | Runtime state | Candidate fact |
| --- | --- | --- | --- |
| 5 | `owner` defined from `Owner { items: child }` | `owner.items → C` | `stored_roots(owner) = {child}` |
| 6 | `entries` defined from a collection literal holding `Owner { items: child }` | `entries[0] → Owner#2 → C` | `stored_roots(entries) = {child}`, through the nested construction |
| 7 | `invoke __moth_collection_remove(entries, 0)`, ok edge to bb4, error edge to the catch arm, both jump to bb3 | Success: `entries` is empty, `detached → Owner#2 → C`. Catch: `detached → Owner#3 → C′`. | Ok edge: `stored_roots(detached) ⊇ children_of(entries) = {child}`. The edge `entries → child` stays. The join unions both arms. |
| 8 | `~detached.items.push(2)` | Success path mutates `C`, which `owner` reaches | `children_of(detached) ∋ child`. `holders_of(child)` includes `owner`, which is live, so it conflicts. |
| 9 | Reads `owner.items` and `entries.length()` | Last keeping uses | |

W1 rejects for the right reason. W1c exposes the second faulty event at line 7: a successful `remove` keeps `entries → child`. In W1c the conflicting holder is `entries`, and it no longer reaches `C` on the success path. The root-granular edge can't express "this element left", and a collection with two elements holding `child` would still reach it.

**W7a.** In the callee, `select` returns `Pair { left: a, right: [2] }` from bb4 and `Pair { left: [3], right: b }` from bb6. The candidate's `return_alias.rs` merges the loads inside each construction into one `AliasParams {a, b}` summary for all exits. `forward` projects it through its arguments. In `probe`, `owned ~= copy selected` gets a fresh root, and `observed = owned.right` makes `observed` alias the root `owned`. The push through `owned.left` then conflicts with `observed`. The first lost distinction is the projected load at line 18, inside `probe` and before any summary matters. The summary loses the per-exit, per-field correlation that W7a and W7b need. That loss only becomes observable after field-precise transfer exists.

**W8 `retain`.** On the baseline `return_alias.rs` classifies `return Box(value)` as `Fresh`, so `forwarded` has no relationship with `observer`. That makes the ordinary twin an unsound accept. The candidate classifies a construction by its surface-typed loads, so it returns `AliasParams {value}`, and the mutation of `original` conflicts with `forwarded`.

### Session 1 conclusions

1. The required W3 acceptance and W4 rejection can't both come from root-granular edges. A design has to know which cell of the holder reaches which allocation. An exact cell replacement must retire the old edge, and the replacing value must supply the new edges. This matches the cell/allocation/edge model in the [HIR-shaped proof](./production-borrow-checker.md#hir-shaped-proof), not the holder-edge representation.
2. Successful `remove` must retire the removed element's edge when the collection holds no other path to the child. With one selector for every element, this is a precision question with no required witness yet. W1c is the candidate witness. Section 4 decides whether it becomes a requirement.
3. No current compiler separates W7a from W7b, or W7c from W7d. Projection precision rejects all four before summary correlation applies. Section 4 has to evaluate summaries assuming field-precise transfer, then pick witnesses that observe correlation through a projection.
4. The candidate's `Fresh`-to-`AliasParams` change for constructed returns fixes a real ordinary unsound accept that's independent of C4. Any design keeps that rule or replaces it with something stronger.
5. The `html_wasm` hashmap verdict needs an owner decision on the manifest once the design lands. A borrow rejection now precedes the capability error.

## 3. Local transfer, liveness and publication

Four read-only traces of the baseline (`git show 7cc05c61b:<path>`) and the candidate fed this section. Paths are relative to `src/compiler_frontend/analysis/borrow_checker/` unless stated. Line numbers refer to the candidate working tree.

### Places and writes

- A HIR place is `HirPlace { root: LocalId, projections }` (`hir/places.rs`). Projections are `Field(FieldId)` or `Index(HirValueId)` only. Options are values (`VariantConstruct`, `VariantPayloadGet`), never place steps. Authored source can't write an index place: collection writes go through `.set`.
- Projections never reach checker state. `roots_for_place` (`transfer/access.rs`) returns `effective_roots(root)` and ignores the projection. `place_cell_root` only accepts an unprojected `SLOT`. Every `RootSet` is a bitmask over function-local binding indices. The candidate's `stored_field_roots` records only that a value came through *some* projection of a root.
- The baseline's projected write (`~root.inner = X`) checks and records the root's cells and value roots and changes no state. The candidate checks with `AliasExclusivity::FieldCell`, then unions `X`'s children into the root's stored sets. Edges only grow.
- Whole-binding replacement is the one strong update. `apply_slot_rebinding` clears all four baseline sets. The candidate calls `retarget_stored_root` first, so other holders move their edge to a remaining holder or drop it. A joined `SLOT|ALIAS` state and a write through a mutable alias union into the referent's cells and don't retarget. Self-assignment (`~x = x`) changes nothing in either tree.

### Stored values and calls

- Collection builtins are ordinary `CallTarget::External` calls whose access and return alias come from `ExternalFunctionDef` (`builder_surface/core_packages/collections.rs`). `get` returns `AliasArgs([0])`, so the result aliases the receiver. `push`, `set` and `remove` take a mutable receiver and return `Fresh`. Maps use `HirMapOp` directly with the same shape. The candidate's `CallStorage` is the only checker code that reads `CollectionBuiltinOp`.
- Fallible map calls lower to `HirFallibleOperation::MapOp`. Fallible collection calls take the external result-call route. Both become `Invoke`. `engine.rs::apply_invoke_edge_transfer` resets every result local, then installs the captured results on the success edge and a fresh slot for the error value on the failure edge. Argument effects, and the candidate's stored-into-receiver edges, happen before the split, so they reach both edges.
- No code in either tree cuts a receiver's edge on `remove`, `clear` or a failed store. The candidate copies the receiver's children onto a removed result and leaves the receiver unchanged.
- `tests/borrow_checker_fact_tests.rs::map_remove_result_is_fresh_owned` removes a string. It asserts that the result is a `SLOT`, not an `ALIAS`, that its `value_roots` exclude `scores` and that the result keeps its own provenance. Snapshots don't expose stored sets, so it says nothing about nested topology or children.

### Liveness

- `metadata.rs` builds whole-local, per-block bitsets: may and must future use, may-assign (no must form) and the visible-local mask per lexical region, plus per-local order keys for the last use in a block and in the function. Writes count as uses.
- `is_local_active_for_alias_conflict` (`transfer/access/conflicts.rs`) decides whether a candidate local blocks an access. A `SLOT` is active while it has any future use. An `ALIAS` stays active past linear expiry if a CFG use or a future assignment follows, and an unused mutable alias stays active to the end of its scope. The candidate only widens *which* locals are candidates, through `holders_of`. Activity is still the holder name's future use.
- `engine.rs` builds a successor state in this order: clone the predecessor output, apply `Invoke` edge results, transport jump arguments simultaneously as `slot_with_value_roots(effective_roots(source))` plus the derivation ledger, then `kill_invisible` for the successor's region, then `join_in_place` (may-union, must-intersect for `derived_from_roots`). Jump transport drops modes, cells, write targets and the candidate's stored sets.
- `kill_invisible` on the baseline resets a departing local and prunes every reference to it. The candidate first runs `relink_departing_roots`, which renames a departing root to one representative and copies its stored children there. `direct_alias_roots`, `write_target_roots` and `derived_from_roots` are pruned, never relinked.
- The Boracle `last_use` module answers event-level, subject-aware future-use queries with witnesses. Only `boracle/report.rs` consumes it. It never feeds acceptance.

The code implements four separate ideas with two mechanisms:

| Idea | Mechanism | Conflation |
| --- | --- | --- |
| A name leaves scope | Region mask, `kill_invisible` | Also deletes the allocation identity, because identity is the binding index |
| A loan's last keeping use | Order keys and may/must future use of the *name* | A holder that's never read again can't block a mutation of the child it holds, even while another live name reaches that holder |
| An allocation reachable through another holder | Candidate stored sets only | Monotone: "was ever stored", with no field or element multiplicity |
| A fact a consumer still needs | Facts recorded at each event | Roots in a fact are binding indices, so an identity retired before the event disappears from the fact |

### Liveness and store witnesses

The four `l*` witnesses live beside the Section 2 witnesses. The store witnesses are earlier BC-06 probes under `tmp/bc06/`.

| Witness | Required | Baseline | Candidate | Class on candidate |
| --- | --- | --- | --- | --- |
| L1 `box = Box(child)` inside a loop body, push after the loop | Accept, `4` | Accepts, `4` | Accepts, `4` | Correct |
| L2 `~child.push(i)` then `~keeper = Box(child)` in a loop, read `keeper` after | `0007` at the push | Accepts, `3` | `0007` 7:10 naming `keeper` | Correct, back edge carries the holder |
| L3 `box = Box(child)`, return `box` on one branch, push on the other | Accept, `1 / 2` | Accepts | Accepts | Correct |
| L4 `holder ~= Box(child)`, `writer ~= holder`, `~writer = Box({5})`, push `child`, read `holder` | Accept, `12` | Accepts, `12` | `0007` 8:6 naming `holder` | False rejection: write through an alias doesn't retire the edge |
| `failed_set`: `~outer.set(5, incoming) catch:` then `~incoming.push(10)` in the handler | Accept, `old=1 incoming=2 size=1` | Accepts | `0007` 17:10 naming `outer` | False rejection: a failed store adds an edge |
| `success_store`: callee `store(~rows, child)` pushes `child`, caller pushes `child` and reads `rows` | `0007` at the push | Accepts, `22` | Accepts, `22` | Unsafe accept: no interprocedural store summary |
| `failure_store`: the same store then `return!`, mutation in the handler | `0007` at the push | Accepts, `22` | Accepts, `22` | Unsafe accept, as above |
| `lexical_dynamic`: `writer ~= outer`, `~writer.push(child)` in a branch, push `child`, read `outer` | `0007` at the push | Accepts, `22 / 21` | `0007` 8:6 | Correct |
| `permission_write`: immutable `input` stored in `Box`, push through the box | Reject for permission | Accepts, `2` | "Cannot mutably access immutable local `input`." | Correct |

```moth
Box = |items {I32}|

probe || -> I32:
    child ~= {1}
    holder ~= Box(child)
    writer ~= holder
    ~writer = Box({5})
    ~child.push(2)
    return holder.items.length() * 10 + child.length()
;
```

### Fact-lifetime ledger

"Identity" is the key a fact uses. "Requeue" is a revisit of the block by the worklist.

| Fact | Meaning | Producer and event | Identity scope | Consumers | Requeue or failure | Earliest safe retirement |
| --- | --- | --- | --- | --- | --- | --- |
| `LocalState.mode` | Binding is a slot, an alias, both or uninitialised | Define, assign, jump transport, `Invoke` edge | Binding index, per block state | Access checks, `record_advisory_drop_sites` (reads only the `SLOT` bit), snapshots | Recomputed per visit, may-union at joins. Failure edges reset result locals. | Region exit, after drop sites read it |
| `value_roots` | Which bindings' values this value may alias | Loads, call result alias, jump transport | Binding index | Conflict candidates, statement and value fact roots, optional transfer | As above | When no live name or edge can still reach the identity, not when its own name dies |
| `direct_alias_roots`, `write_target_roots` | Cells a write through this binding reaches | Alias formation, projected mutable alias | Binding index | `write_storage_cells`, `written_roots`, then the mutation summary | As above. Pruned, never relinked, at region exit. | Same as `value_roots`. A parameter's identity must outlive every alias that can write to it. |
| `derived_from_roots` | Must-set of aliases this value came from | Alias derivation, jump transport | Binding index | Access exemption, joins | Must-intersect at joins | Not published, so any time it's no longer consulted |
| Candidate `stored_roots`, `stored_field_roots` | Holder reaches a child root, whole or through a projection | Construction, projected store, builtin insert, `remove` result | Binding index | `holders_of`, `children_of`, so conflicts and mutable argument roots | Union only. Not transported by jumps. Relinked at region exit. Kept on failure edges. | Never in the candidate. A correct design retires on exact replacement, successful removal and failed stores. |
| Metadata liveness | Future use, future assignment, visibility, order keys | Computed once per function | Binding index, block, order key | Activity, `classify_move_decision` | Static | Function end |
| `StatementBorrowFact` | Shared, mutable and written roots at a statement | `StatementAccessTracker` during transfer | `HirNodeId` | `metadata.rs::finalize_public_call_summary_effects`, tests | Last visit wins. The last visit uses the fixed-point input, so it's final. | After the mutation summary converges |
| `TerminatorBorrowFact` | Same for terminators | Same | `BlockId` | Tests | Same | Same |
| `ValueBorrowFact` | Classification, roots and optional transfer for one HIR value | `ValueFactBuffer` per block visit | `HirValueId`, module-wide | Mutation summary (call argument and map receiver roots), tests | Replaced per block on revisit. Merged across final contexts by sorted union and `merge_completed_use`. | After the mutation summary converges |
| `PublicCallSummary` | Parameter access, mutation, transfer, return alias and hazard bits | `metadata.rs` and `return_alias.rs` | `FunctionId`, then stable interface identities | Interface finalisation, private failure lanes, constraint checks, generated convergence, artefact equality | Recomputed to a fixed point | Owned by the module artefact |
| `advisory_drop_sites` | Owned slots to drop on exits | `record_advisory_drop_sites` from out states | `BlockId` and binding index | `backends/wasm/hir_to_lir/ownership.rs` (`DropIfOwned`) | Read from final out states | Owned by the report |
| Developer snapshots | Modes and roots per state | Test and `show_borrow_checker` builds | Binding index | Tests | Final states | Owned by the report |

No production consumer outside the checker reads statement, terminator or value facts, or `optional_transfer`. Mutation and return information leave the checker only through `PublicCallSummary`, and ownership only through drop sites.

**A source binding dies while a child survives.** `inline_ctor_branch` declares `shared ~= {1}` inside one arm of a value `if` and ends the arm with `then Box(shared), shared`. The jump transports both results, then `kill_invisible` removes `shared`. The baseline prunes `shared` from both destinations, so `box` and `items` share nothing and `~items.push(2)` is accepted. It renders `2`. The candidate's relink renames `shared` to the representative `items` and moves the departing temporary's children to `box`, so the push conflicts with `box`. The name died, the allocation didn't, and only a fact keyed by something other than the name survives the exit.

**Why the factored cutover broke publication.** Its archive retired dead alias holders eagerly and kept owned slots "for source root and mutation fact attribution until edge retirement" (`tmp/bc06/cutover/final-archive/tracked.patch`, lines 606-607). The cutover logs still show `projected_alias_write_at_join_publishes_parameter_mutation` publishing `NoWrite` for a written parameter, a missing map receiver fact during summary publication and `shared_expression_row_merges_branch_roots_in_either_successor_order` publishing extra roots. Facts are already recorded at the event. They lose information when the identity a later event needs was retired with an earlier name.

**One HIR value in two blocks, visited in either order.** Value facts merge by sorted, deduplicated union, a classification join and `merge_completed_use`, which keeps `Borrow` if any context borrows. These are commutative and associative, and each block contributes only its final visit, so the published row doesn't depend on visit order. `tests/borrow_checker_fact_tests.rs` pins this with `shared_expression_row_merges_branch_roots_in_either_successor_order`, `retained_result_and_transfer_observations_merge_across_final_cfg_contexts`, `actual_cfg_rows_preserve_all_transfer_and_ignore_absent_contexts`, `actual_cfg_transfer_and_ordinary_definition_merge_conservatively` and `catch_failed_argument_keeps_optional_transfer_consistent_with_handler_use`. Value facts are keyed module-wide while their roots are function-local. No test covers a row shared across functions.

### Section 3 conclusions

1. Identity has to outlive names. Any design that retires state at a name's last use needs an identity for each allocation or cell that stays valid while some live name or edge reaches it, and facts must record those identities at the event. Retiring dead slots is safe only under that split.
2. Every false rejection the candidate adds comes from a missing strong update: exact field replacement (W3), successful removal (W1c), a write through a mutable alias (L4) and a failed store (`failed_set`). Each needs the edge keyed by the cell that holds it, and the failure lanes need effects applied per edge, not before the split.
3. Activity follows the name, not the loan. A holder counts only while its own name has a future use. That's sound for the current witnesses because every observation goes through a named holder. Section 4 must check whether a summary-returned edge can make an unnamed holder matter.
4. Interprocedural stores (`success_store`, `failure_store`) stay unsafe accepts in both trees. They need a summary effect that names the parameter receiving the store.

## 4. Calls and publication boundaries

Three read-only traces covered `return_alias.rs`, `public_call_summary.rs`, the interface and generated-convergence owners and the Boracle adapter. Neither the summary type nor any publication owner differs between the baseline and the candidate. The candidate changes only how `return_alias.rs` classifies a constructed return.

### Today's summary

`metadata.rs::build_public_call_summaries` builds one summary per function before any transfer and stabilises it over recursion. `return_alias.rs::classify_function_return_alias` visits every reachable `Return` and merges each returned expression into one value: `Fresh < AliasParams(sorted parameter indices) < Unknown`. A parameter load reports the whole parameter, whatever its projection. A returned local takes a flow-insensitive union of all its writers. Separate success slots share one union, and `ReturnError` exits contribute nothing. `project_alias_summary_through_arguments` maps a callee's indices through the caller's own argument classifications. Every merge sorts and deduplicates, so the result doesn't depend on traversal order, and the lattice height per function is at most the parameter count plus two. `stabilize_return_alias_summaries` checks only inequality and stops at a cap. The monotone transition validator runs only in generated convergence.

| Function | Baseline | Candidate |
| --- | --- | --- |
| `select`, disjoint arms | `Fresh` | `AliasParams([1, 2])` |
| `select`, one arm `Pair(a, b)` | `Fresh` | `AliasParams([1, 2])` |
| `forward` | `Fresh` | `AliasParams([1, 2])` |
| `retain` | `Fresh` | `AliasParams([0])` |

At the call `forward(flag, items, items)`, `transfer_call_result_alias` unions the value roots of arguments 1 and 2. Both are `{items}`, so `selected` becomes a slot whose value aliases `items`. It holds no stored edge. On the baseline `selected` is root-free. Neither form records which field came from which argument, or that the two fields never share within one exit. The two `select` variants get identical summaries.

`PublicCallSummary` has no store effect. A callee that pushes its shared argument into its mutable parameter publishes only `mutation = Writes` for the receiver. If it then takes `return!`, its alias summary is `Unknown` because no `Return` is reachable. That's why `success_store` and `failure_store` stay unsafe accepts.

### The minimum summary W7 needs

Write each exit as an alternative that maps every result slot and field path to its source:

| Callee | Alternative | `left` | `right` |
| --- | --- | --- | --- |
| `select`, disjoint | exit 1 | parameter `a` (shared) | fresh |
| | exit 2 | fresh | parameter `b` (shared) |
| `select`, one arm | exit 1 | parameter `a` (shared) | parameter `b` (shared) |
| | exit 2 | fresh | parameter `b` (shared) |
| `forward` | each `select` alternative, substituted through `forward`'s arguments | | |

Substituting `items` for both `a` and `b` gives, for the disjoint `select`, `{left: items, right: fresh}` or `{left: fresh, right: items}`. Neither alternative puts one allocation under both fields, so `copy selected` produces a graph with no internal sharing, and pushing through `owned.left` can't reach `observed`. For the one-arm variant, exit 1 becomes `{left: items, right: items}`. The copy preserves that sharing inside the new graph, so `observed` aliases the pushed collection and the push must reject. The value-`if` join in W7c and W7d produces the same two alternatives if the join keeps them apart, and today's union if it merges them.

So the summary needs, per outcome lane:

- guarded alternatives, one per distinguishable exit, after deduplication
- for each alternative, a map from (result slot, field path) to a parameter, a parameter's field path or fresh storage, with the edge's permission
- per-parameter effects for each lane: mutation as today, plus "parameter `i`'s value is stored into parameter `j`", so a store that happens before `return!` stays visible on the failure lane.

Exactness and bounds:

| Property | Rule |
| --- | --- |
| Exact | Direct parameter loads and constructions in each exit, per field path |
| Conservative | Opaque branch conditions, so exits aren't correlated with arguments. Collection elements share one path. Recursive and over-deep types collapse to the whole value. |
| Growth | Alternatives per summary are capped. Past the cap, all alternatives merge into one whose field map is the union, which is today's `AliasParams` per field. Merging is sound, deterministic and loses only precision, so it needs no refusal. |
| Order | Alternatives are canonicalised: sorted field maps, sorted alternatives, duplicates and subsumed alternatives removed. Equal inputs give equal summaries in any traversal order. |
| Recursion | A call group starts every member at "no alternatives" and iterates callee-first to a fixed point. Alternatives only grow until the cap merges them, so the iteration is monotone and terminates. |
| Two invocations | Each call site instantiates fresh identities for its results, so two calls of the same function never share a fresh allocation. |

### Publication boundaries

- **Interface export.** `public_interface/local_finalization.rs` stores each exported concrete free function's or receiver method's whole `PublicCallSummary` under its stable `OriginFunctionId`. The summary contains only parameter positions, so no function-local ID crosses. `validate_public_call_summary` checks the parameter count, declared access, `Shared ⇒ NoWrite` and a canonical, in-range alias set. Interface closure requires every publisher of one origin to agree exactly. Generic templates, private functions and generated sidecars publish no record here.
- **Generic materialisation.** `ast/generic_functions/materialisation/preparation_freeze.rs` keeps only the summaries that evidence requirements name, and requires imported and published copies to be equal. Generated functions start from a signature bootstrap. After the sidecar's own borrow analysis, `exact_generated_sidecar_summary` replaces it.
- **Generated convergence.** `module_compilation/generated/convergence.rs` reschedules callers only on a validated `Widened` transition. `base_summary_changes` maps widened base functions to stable identities and enqueues only the generated nodes that name them. `install_convergence_summaries` skips a re-borrow when installed summaries are unchanged. After the queue drains, `refresh_private_failure_lanes` reruns `check_borrows` on rewritten HIR when installation changed it. It doesn't validate a transition or reschedule callers afterwards, and it doesn't write refreshed summaries back to the generated transaction. A summary that depends on failure lanes would need that path to revalidate.
- **Call kinds.** `call_semantics.rs::resolve_call_semantics` reads `Local` summaries from the current report, `CrossModule` from imported summaries, `ModulePrivate` and `Generated` from the HIR maps, and `External` from the package registry. Receiver methods resolve to one of these targets. `Call` and `Invoke` apply the return alias, `Invoke` only on the success edge. `NeverCall` applies argument effects only.
- **Cost of a richer summary.** 22 files name `FunctionReturnAliasSummary`. None serialises it, since publication uses structural equality. The changes fall on the classifier, transfer, transition validation, interface agreement, the external ABI conversion (`ExternalHirReturnAlias`) and Boracle's `CallResultProvenance` mirror.

### Boracle

`problem/call_effects.rs` translates a call into ordered arguments with shared or exclusive access and one result origin shared by every result slot. It drops mutation, transfer, store targets and field paths. Map `remove` becomes `Unknown(OpaqueExternal)`. In source mode the Boracle service passes no local summaries, so every local call becomes `Unknown(MissingSummary)` and the oracle stops as inconclusive. A direct construction does produce field edges (`AggregateField`), but nothing crosses a call.

The differential dumps used a release build of `7cc05c61b` with `--features boracle`:

```bash
moth boracle tmp/bc06/boracle_runs/<witness>.moth --dump differential
```

| Witness | Function with the behaviour | Reference static result | Oracle | Meaning |
| --- | --- | --- | --- | --- |
| W3 | `runtime_retained` | Conflict at the replacement `~root.inner = Inner({3, 4})`, against the loan from `Outer(root.inner)`, kept by the push | `RuntimeConflict` | Disagrees with the approved contract |
| W4 | `runtime_retained_twin` | Conflicts at the replacement and the push | `RuntimeConflict` | Right verdict |
| L4 | `probe` | Three conflicts | `RuntimeConflict` | Disagrees with the expected acceptance |
| W1, W1c | `runtime_detached` | None | Inconclusive: `remove` result is `SummaryUnknown` | No evidence |
| W7a | `probe` | One conflict | Inconclusive: `MissingSummary` | No evidence |
| B1 permission negative | `marshal` | None | `CompleteSafe` | Doesn't model permission through aggregates |

Both Boracle lanes treat a stored reference as a loan of the source *place*, so replacing `root.inner` conflicts with `retained`. Moth stores allocations by value. C2 states that "storing a writable allocation by value does not retain the projection through a later-replaced source field", and `access-and-aliasing.mtf` says a fresh binding can hold a reference to an existing allocation without aliasing its source binding cell. Boracle can't serve as acceptance evidence for W3, L4 or permission. The existing pinned disagreement, `boracle_differential_shared_alias_definition_write_through_is_a_conflict_candidate` in `boracle/tests/differential.rs`, is a separate case: the reference accepts an exclusive defining write through a shared alias that the oracle executes as a conflict.

### Permission and open bug reports

| Witness | Required | Baseline | Candidate |
| --- | --- | --- | --- |
| B1 `marshal \|items {I32}\|` stores `items` in `box ~= Box(child = items)` and pushes through `box.child` | Reject for permission, even with no later use | Accepts, renders `22` | "Cannot mutably access immutable local `items`." at 5:10 |
| B1p as B1, but `~box.child = {7}` before the push | Accept, renders `21` | Accepts, `21` | Same rejection at 6:10. False rejection: the replacement doesn't retire the edge. |

| Report | Reproducer | Baseline / candidate | Classification |
| --- | --- | --- | --- |
| Binding a place into an option drops its alias provenance | `maybe {I32}? = holder.items`, then `replace(~holder)` | Accepts, `10` / rejects at 18:22 | Shared prerequisite. Option `some` is an aggregate construction, so the edge model owns it. Owner: BC-06. |
| Multi-bind through a mutable alias is rejected at runtime | `view ~= base`, `~view, extra = pair(3)` | `MOTH-BORROW-0003` at 7:18 naming `__hir_tmp_1`, in both | Independent. The multi-bind result temporaries are read while the alias is live in both trees. Owner: HIR multi-bind lowering and Stage 6 temporary classification. |
| Ordinary struct construction bypasses an immutable parameter's access | B1 | Accepts / rejects | Shared prerequisite. Permission must travel with the cell-keyed edge, or B1p stays rejected. Owner: BC-06. |
| Projected alias write targets keep the holder cell as a conservative identity | `writer ~= holder.first`, `~holder = Pair({2}, {3})`, read `holder.first`, then `writer` | `MOTH-BORROW-0003` at 7:19 in both | Shared prerequisite. It needs the explicit allocation identity from [Section 3 conclusion 1](#section-3-conclusions). Owner: BC-06. |

### Failure policy

| Outcome | Policy |
| --- | --- |
| A real conflict | The existing access diagnostic at the access, such as `MOTH-BORROW-0007` or `MOTH-BORROW-0003`. An established conflict takes precedence over every other outcome. |
| An incomplete mandatory proof | Fail closed: no acceptance and no partial summary. An established conflict still takes precedence. The chosen design has no hard cutoff, so every transfer finishes with a sound result and this outcome can't arise. Adding a hard analysis limit reopens the diagnostic decision with the owner. Any new code must skip `MOTH-BORROW-0008`, which is historical. |
| An unproven optional transfer | Falls back to borrowing. Never a source error. |
| Missing compiler metadata | The infrastructure-failure contract, as today when a reachable statement lacks a published fact in `metadata.rs::finalize_public_call_summary_effects`. |

A sound merge, such as a weak update or a union at a join, isn't an incomplete proof. When the finished analysis shows that two paths may alias, the existing access diagnostic is accurate.

### Section 4 conclusions

1. W7a and W7b differ only in which fields share inside one exit. A summary keyed by parameter index alone can't express that. The smallest sufficient summary keeps per-lane, per-exit alternatives mapping result field paths to parameter paths, with permission, plus per-lane store effects.
2. Publication carries summaries opaquely by stable origin and compares them structurally. A richer summary costs match and validation updates in about 22 files, with no wire format to migrate. The failure-lane refresh path needs transition validation before summaries can depend on lanes.
3. Boracle can't arbitrate W3, L4, permission or any local call in source mode. Its place-loan model disagrees with the approved stored-by-value contract. Acceptance evidence for the redesign must come from Node execution of witnesses and the canonical rules, with Boracle used only where its model agrees.
4. The permission positive B1p fails on the candidate for the same reason as W3, which makes cell-keyed edges with strong update on exact replacement the common prerequisite for W3, B1p, L4, `failed_set` and the option and projected-alias bug reports.

## 5. What C4 publication must consume

A read-only trace covered `ast/const_values/const_execution/{rewrite,values}.rs`, `ast/const_values/store.rs`, `hir/hir_expression.rs`, `hir/hir_statement/declarations.rs`, the JavaScript expression emitter and the local C4 prototype (`tmp/bc06-prototype/tracked.patch`). Paths are relative to `src/compiler_frontend/`.

### Where sharing is lost today

| Step | Code | Effect |
| --- | --- | --- |
| Outer boundary | `rewrite.rs::finish_call` validates results with `validate_call_results`, then calls `freeze_live_value` once per slot | Each slot gets a fresh allocation memo, so `return items, items` freezes two independent rows |
| Freeze | `values.rs::freeze_live_value` memoises `(AllocationId, shape)` within one call | `return Pair(a, a)` freezes one row for `a` inside the slot |
| Commit | `values.rs::commit_call_result` emits one `Constant` row, or a result block of `Constant` rows | Slot order and types are already validated |
| Lowering | `store.rs::fold_value` visits every edge with no memo. `declarations.rs::lower_const_store_expression` builds a fresh `Collection` or `StructConstruct` per occurrence. | Even a shared row becomes separate runtime allocations |
| Emission | The JavaScript backend prints aggregates as inline literals in the enclosing function | Every evaluation builds fresh allocations. Nothing is hoisted. |

`ConstValueStore::insert_value` always appends, so two equal collections from distinct allocations never merge. Imported folded values are projected as trees (`import_projection/values.rs::import_folded_value`), so a provider's internal sharing doesn't cross an import. `finish_call` inserts frozen rows directly. The rollback path, `ConstValueStore::stage_values`, isn't used there, so a failure after the first insert can leave rows behind.

### Witnesses

| Witness | Required | Baseline | Candidate | What it shows |
| --- | --- | --- | --- | --- |
| P1 `make #\|\| -> {I32}: return {1}`, `probe` pushes into `make()` and is called twice | Renders `22` | `22` | `22` | Each evaluation materialises fresh storage |
| P2 `make #\|\|` returns `Pair(a, b)` with two equal `{1}` collections, push through `left` | Renders `21` | `21` | `21` | Equal contents stay distinct |
| P3 `make #\|\|` returns `Pair(a, a)`, push through `left`, read `right`. An ordinary twin does the same. | Both render `2` once C4 publishes sharing | Const `1`, ordinary `2` | Same | The const result loses intra-slot sharing today |
| P4 ordinary `Pair(a, a)`, `owned ~= copy source`, push through `owned.left` | Renders `21` | `21` | `21` | `copy` keeps internal sharing and detaches from the source |
| H1 hand-lowered publication: `shared ~= {1}`, then `box Box, items ~{I32} = Box(shared), shared` in a value `if`, `forwarded = identity(box)`, push `items`, read `forwarded` | `0007` at the push | Accepts, renders `2` | `0007` 15:6 | The prototype's lowering needs the construction-edge rule |
| H2 as H1 with an immutable `shared {I32} = {1}` | `0007` | "Cannot mutably access immutable local `shared`." | Same | An immutable temporary turns into a permission failure |
| H3 as H1 without `forwarded` | Accept, renders `2` | Accepts | Accepts | The legal shape stays legal with a writable temporary |
| H4 as H3 with an immutable temporary | Accept | Permission rejection at 14:6 | Same | An immutable temporary rejects a legal program |

### Contract at each boundary

1. **Interpreter to store.** One freeze per call result group, across every ordered slot, inside `stage_values`. The memo maps each live allocation to one shared definition row. Later occurrences become rows that point at that definition. Storage IDs, generations and live handles stay inside the interpreter. Equal contents in different allocations stay different rows. A failed freeze publishes nothing. The prototype's `freeze_live_values`, `FreezeScratch` and `ValueBlock::ConstantResults` implement this.
2. **Store to HIR.** One lowering context per slot group. The first occurrence of a shared definition becomes a `DefineLocal` of a temporary in the prelude, and every occurrence becomes `Load(temporary)`. Lowering happens at the use site, so each runtime evaluation defines its own temporaries and gets fresh allocations. The prototype's `lower_const_store_expression_in_graph` does this. Match guards need the prototype's definitions-only prelude so a guard stays side-effect free.
3. **Permission.** A published graph is fresh owned data. The temporary must be writable, which `allocate_temp_local` already gives a `CompilerTemp`. Each receiving binding's own declaration decides mutation, as the C1 decisions require. H2 and H4 show that an immutable temporary would turn sharing into a permission error.
4. **Stage 6.** It sees ordinary HIR: a writable temporary, constructions that load it and result slots that load it. No const-specific rule is allowed. The same ordinary path then has to reject H1 and accept H3. That needs construction edges (the candidate rule or its replacement), identity that survives the scope exit of a value-`if` arm (Section 3 conclusion 1) and the call summaries from Section 4 when the folded graph flows through `identity` or `retain`.
5. **Other data.** Slot order and types keep `validate_call_results`. Structural strings and resources keep their existing visits and charges, and a shared definition is charged once. Supported imported folded values preserve references within one owned graph and remap them into consumer storage. Scalar-returning functions such as `const_detached` and `const_retained` publish no graph, so they stay independent of this work.

### Dependency list

| Work | Depends on | Can start independently |
| --- | --- | --- |
| BC-06 cell-keyed edges with strong update (W3, W4, B1p, L4, W1c, `failed_set`) | Section 6 design choice | Yes |
| BC-06 per-exit summary alternatives and store effects (W7, `success_store`) | Cell-keyed edges, so a field path has something to name | After the edge slice |
| C4 freeze transaction and shared rows in the store | Nothing in Stage 6 | Yes, behind the existing non-sharing lowering |
| C4 shared lowering through prelude temporaries | Construction edges and scope-exit identity in Stage 6, or H1 and the committed C4 fixtures become unsafe accepts | No |
| C4 failed-publication rollback through `stage_values` | Nothing | Yes |
| C5 checkpoint and measurements | C4 shared lowering | No |
| Phase 5 immutable receiver methods | C5 | No |

These ordinary cases must pass before shared folded results are exposed: H1 and H3, both committed `runtime_call_*_rejected` fixtures and their ordinary twins, `inline_ctor_branch`, W3 and W4, B1 and B1p and the existing `runtime_call_shared_*` operand fixtures.

### Section 5 conclusions

1. C4 loses sharing at the per-slot freeze and again at per-occurrence lowering. The prototype fixes both with one freeze transaction per slot group and prelude temporaries. That changes HIR shape, not Stage 6 rules.
2. The prototype's lowering is exactly the H1 shape. The baseline accepts it unsoundly and the candidate rejects it correctly, so shared lowering can't land before BC-06 ships construction edges and scope-exit identity.
3. The temporary must be writable fresh storage. This settles the open question about fresh materialisation permissions: permission comes from the receiving binding, never from the temporary.
4. The store transaction, shared rows and rollback can proceed in parallel with BC-06, since they don't change emitted HIR until the lowering switches.

## 6. Design choice

### Performance evidence

The 18-input manifest is recovered from `tmp/bc06/baseline_measure.py` and its output `tmp/bc06/baseline_measure.json`. Every input's SHA-256 still matches the recording. Twelve inputs are `benchmarks/numeric-scaling/<shape>-<size>.moth` for `alias-receiver-chain` 256 and 1024, `origin-fanout` 128 and 512, `copy-chain` 128 and 512, `aggregate-forwarding` 32 and 128, `narrow-chains` 128 and 512 and `generic-failure-chain` 32 and 128. Two are `benchmarks/borrow-stress.moth` and `benchmarks/speed-test.moth`, which contains `build_runtime_grid`. The script generates the last four into `tmp/bc06/scaling/`:

- `branches-N`: `probe` takes `N` `Bool` flags and declares `total ~= 0`, then runs `N` statements of the form `if flag_i: ~total = 1 ;`, then returns `total`.
- `sibling-loops-N`: `probe` declares `total ~= 0`, runs `N` statements of the form `loop 0 to 2: ~total = 1 ;`, then returns `total`.

Both are generated for `N` of 16 and 64. The fixture `tests/cases/numeric_profile_int64_runtime_surfaces`, the cutover's worst unit, was measured as a nineteenth input outside the guard.

`tmp/bc06/measure_s6.py` imports that manifest and measures all three compilers back-to-back, interleaved per input. Each compiler gets one warm-up and five samples, with `MOTH_TIMERS=bench`, then one counter replay with `MOTH_COUNTERS=summary`. The binaries are release builds with `detailed_timers` and `benchmark_counters`: the archive `0f384a5539`, the baseline `7cc05c61b` and the candidate on that base. Raw results are in `tmp/bc06/measure_s6.json`.

Medians in milliseconds and peak RSS in MiB. Borrow time is `frontend.borrow.initial` + `frontend.borrow.converge` + `frontend.generated.borrow_recheck`.

| Input | Archive check / borrow / RSS | Baseline | Candidate | Candidate guard |
| --- | --- | --- | --- | --- |
| alias-receiver-chain-256 | 5.09 / 1.00 / 16.3 | 5.16 / 1.02 / 16.6 | 5.42 / 1.36 / 16.6 | Pass |
| alias-receiver-chain-1024 | 15.54 / 7.17 / 23.3 | 15.48 / 7.04 / 20.6 | 22.22 / 13.30 / 22.7 | Fails check and borrow |
| origin-fanout-128 | 4.03 / 0.65 / 15.4 | 4.12 / 0.67 / 15.3 | 4.59 / 0.86 / 15.5 | Pass |
| origin-fanout-512 | 10.82 / 5.22 / 18.6 | 11.13 / 5.39 / 18.5 | 12.73 / 7.16 / 18.5 | Pass |
| copy-chain-128 | 3.33 / 0.26 / 14.7 | 3.22 / 0.25 / 14.5 | 3.39 / 0.34 / 14.5 | Pass |
| copy-chain-512 | 5.19 / 1.21 / 16.0 | 4.96 / 1.19 / 15.8 | 6.07 / 2.04 / 15.9 | Pass |
| aggregate-forwarding-32 | 5.50 / 1.19 / 16.5 | 5.70 / 1.19 / 16.3 | 5.92 / 1.60 / 16.4 | Pass |
| aggregate-forwarding-128 | 16.54 / 8.28 / 21.4 | 16.59 / 8.26 / 21.4 | 24.55 / 15.65 / 20.7 | Fails check and borrow |
| narrow-chains-128 | 19.17 / 13.86 / 77.8 | 19.64 / 14.51 / 77.7 | 24.20 / 18.76 / 93.6 | Fails check |
| narrow-chains-512 | 197.30 / 184.37 / 731.1 | 184.67 / 172.25 / 850.6 | 261.70 / 249.40 / 826.9 | Fails check |
| generic-failure-chain-32 | 5.10 / 1.29 / 17.8 | 5.19 / 1.26 / 17.7 | 5.22 / 1.45 / 18.5 | Pass |
| generic-failure-chain-128 | 15.06 / 8.81 / 47.4 | 15.22 / 8.91 / 47.2 | 17.11 / 11.01 / 55.9 | Pass |
| borrow-stress | 4.02 / 0.70 / 15.7 | 4.08 / 0.69 / 15.7 | 4.11 / 0.80 / 16.0 | Pass |
| speed-test | 12.96 / 3.91 / 21.3 | 13.09 / 3.92 / 22.8 | 13.60 / 4.67 / 24.8 | Pass |
| branches-16 | 2.96 / 0.28 / 14.7 | 3.08 / 0.28 / 14.4 | 2.98 / 0.30 / 14.6 | Pass |
| branches-64 | 4.66 / 1.39 / 18.4 | 4.73 / 1.40 / 18.2 | 4.82 / 1.69 / 19.7 | Pass |
| sibling-loops-16 | 5.44 / 2.39 / 18.0 | 5.50 / 2.46 / 17.7 | 5.85 / 2.85 / 19.1 | Pass |
| sibling-loops-64 | 34.33 / 29.57 / 95.9 | 36.57 / 31.60 / 86.3 | 46.26 / 42.39 / 123.9 | Fails check |
| int64 runtime surfaces (outside the guard) | 10.17 / 6.03 / 21.1 | 10.20 / 6.14 / 20.9 | 12.90 / 8.83 / 22.4 | Would fail check |

The baseline passes every guard. The candidate fails five of the 18. Its counter replay matches the baseline exactly on every input: worklist iterations, joins, statement visits and conflict checks. The whole slowdown is per operation. `LocalState` gains two dense `RootSet` buffers of `ceil(L / 64)` words per local, so every clone, join and comparison does more work. `sibling-loops-64` has no aliasing at all and still pays 34 % more borrow time. A design that adds per-local dense sets for edges can't pass the guard, and allocation identities would make those sets larger.

### Approaches ruled out

| Approach | Evidence |
| --- | --- |
| Root-granular holder edges, the candidate as it stands | W3 and W4 get the same diagnostic. Five guard failures with identical counters. |
| One parameter-index union per function summary | W7a and W7b get identical summaries ([Section 4](#the-minimum-summary-w7-needs)) |
| Allocation identity sets alone, with no sharing relation | W7a's call result holds `items` in `left` on one exit and in `right` on the other. A copy that reads only identity sets must assume `left` and `right` may share, so W7a still rejects at the push. |
| Factored per-exit alternatives in the caller's state | The cutover's representation. It changed about 274 verdicts and reached about 3× check time on two inputs. Alternatives grow exponentially and need a refusal diagnostic for a budget. |
| Retiring state at the last use of a name | Breaks fact publication ([Section 3](#fact-lifetime-ledger)) |
| Boracle as the acceptance arbiter | Disagrees with the approved W3 contract ([Section 4](#boracle)) |

### The two designs compared

**Extension.** Keep the existing worklist, `LocalState` root sets, fact publication and drop sites. Add one sparse allocation layer beside them. It records which allocation each projected cell holds and which allocations each allocation stores, with a permission per edge. Call summaries gain per-field sources and a conditional sharing relation. Binding-level aliasing keeps today's rules, so the only verdicts that move are the ones the new layer decides.

**Replacement.** Replace the root sets with liveness-scoped cells, allocations and edges, and take every fact from the state at its event. It needs the same allocation identity, sharing relation and summaries. It also rebuilds binding-level aliasing, transfer and publication. No witness distinguishes it from the extension. Its gain is removing dead state, the cost that E1 in [the BC-06 plan](./production-borrow-checker.md#e1-dominant-state-size-copying-and-repeated-checking) describes. That cost exists on the archive and isn't required by any witness or guard.

| Criterion | Extension | Replacement |
| --- | --- | --- |
| Separates W3 from W4, B1 from B1p | Yes, through cell strong update | Yes |
| Separates W7a from W7b, W7c from W7d | Yes, through the sharing relation | Yes |
| Verdicts reopened | Only constructs with stored children, projections and calls returning aggregates | All 3,323 corpus cases. The cutover moved about 274. |
| Cost on code without stored children | Zero entries in the layer, so baseline cost | Unknown until built. The cutover was up to 3×. |
| Fact publication and drop sites | Unchanged | Rebuilt and re-proved |
| Needs a refusal diagnostic | No. Every fallback is a sound weak update or merge. | Yes, for its alternative budget |

The extension is chosen. The replacement stays a possible later performance project under the same guard, independent of BC-06 correctness.

### Chosen design

**Identities.** An allocation identity is an allocation site: a construction, collection literal, `copy`, call result field or map result, plus one entry identity per parameter field path and per cell whose contents were never written in the function. Entry identities are minted lazily on first projection. A site inside a loop names every instance it creates, so a conflict check treats two instances as possibly the same allocation. That's conservative and sound.

**Cells.** A cell is a local plus a field path. Collection elements share one path. The layer keeps `cell → allocation set` only for cells that a construction, projection or field write has touched.

**Edges.** An edge is `(holder allocation, field path, held allocation, permission)`. Construction adds one edge per child, with the child's access permission. A `~` binding of an immutable local's allocation never gains write permission through an edge.

**Sharing pairs.** Each holder allocation also keeps the pairs of its field paths that may hold one allocation in the same execution. A construction adds a pair when two children's identity sets intersect. Joins take unions. `copy` mints fresh identities for the copied graph and shares one identity between two paths only when the source holds that pair. It never derives sharing by intersecting identity sets, because a call result's sets mix exits.

**Strong update.** A write to exactly one cell replaces that cell's allocation set. That covers a field write, `~alias = value` when the alias names exactly one cell, and a map `remove` that moves the element's identity into its result. A write whose target is several cells, or a cell inside a loop-site allocation, adds without removing. `set` applies its store edges on the success edge of the `Invoke` only.

**Conflict rule.** A mutation through a path reaches the allocations that path names. It conflicts when another live binding, or an edge held by an allocation reachable from another live binding, reaches one of those allocations. It's also rejected when the path crossed an edge without write permission. Reachability is computed at the access, not stored. Edge sets live in the block state as sorted vectors. Joins are sorted unions and comparisons are slice comparisons.

**Retirement.** At block exits the layer drops allocations no live local can reach. A dead local's name disappears, but its allocations stay while anything live reaches them. Facts are still published from the state at the event, so retirement can't change a published fact.

**Call summaries.** For each success slot and each lane, the summary maps each result field path to a set of sources: a parameter field path or fresh storage. It also records conditional sharing pairs. A pair `(path p, path q, if parameters i and j alias)` means the two result fields may hold one allocation when the caller passes aliasing arguments for `i` and `j`, and an unconditional pair means they always may. The callee derives pairs from its own sharing pairs and from children that come from two parameters. Joins and multiple exits take unions. Per-lane store effects say "parameter `i`'s value is stored in parameter `j` at path `p`". A call site instantiates sources from argument identities, mints fresh identities per call, applies store effects on the matching lane edge and records the result's sharing pairs whose conditions hold for the actual arguments. The summary has no alternative cap: its size is bounded by result paths and parameter paths, and recursion converges because every component only grows.

For W7a, the disjoint `select` has no pair, so `copy selected` gives `owned.left` and `owned.right` distinct identities and the push through `owned.left` can't reach `observed`. For W7b, the shared arm adds `(left, right, if a and b alias)`. `forward` passes `items` for both, so the copy shares one identity and the push rejects. W7c and W7d get the same results through the value `if` join.

**Failure policy.** As [Section 4](#failure-policy) says. Every transfer finishes with a sound result, so the extension needs no new diagnostic code. If the analysis ever stops before establishing a mandatory result, it fails closed. A hard cutoff would reopen the diagnostic decision.

### Accepted imprecision

The owner accepted these four limits for this implementation. Each stays a conservative rejection and covers only the shape described. If a required positive falls inside one, the slice stops and returns to the owner rather than widening the limit or dropping the positive.

| Limit | Scope | Required positives it must not reach |
| --- | --- | --- |
| Mixed exits in an uncopied call result | Reading one field of a call result while mutating another field of the same result, with no `copy` in between, when one argument reaches both fields on different exits | W7a and W7c copy first. `runtime_call_disjoint_result_fields` uses distinct arguments. |
| Loop allocation sites | Two allocations created by one site on different iterations of a loop are treated as possibly the same | L1 and L3 keep one allocation per iteration live at a time |
| Ambiguous alias writes | `~alias = value` when the alias may name more than one cell is a weak update | L4 writes through an alias that names exactly one cell |
| Collection elements | All elements of one collection share one field path. A successful `remove` clears that path only when the checker can prove no other element remains. | W1c removes the only element. The fix must preserve disjointness from the result across its catch join. |

### What happens to the candidate

| Candidate piece | Fate |
| --- | --- |
| `HirExpressionKind::is_aggregate_construction` in `hir/expressions.rs` | Kept |
| `constructed_children` and the construction-children rule in `transfer/access.rs` | Kept as the source of construction edges, rewritten to target the layer |
| Constructed returns classified as `AliasParams` in `return_alias.rs` | Kept until slice 2 replaces the summary. It fixes the ordinary `fresh_box` unsound accept. |
| The `Remove` rule in `transfer/access/statement.rs` | Replaced by strong update of the result cell |
| `stored_roots`, `stored_field_roots`, `store_children_into`, `relink_departing_roots`, `departing_representative`, `rename_stored_root` and `candidate_names_root` | Deleted. Dense per-local sets fail the guard. |
| Store edges applied before the `Invoke` split | Deleted. Stores apply per lane edge. |
| The six test literals that don't compile | Moot after the deletions |

### Section 6 conclusions

1. Dense per-local edge state can't meet the guard: the candidate's counters match the baseline, yet five workloads fail.
2. Identity sets need a sharing relation, or `copy` can't tell W7a from W7b. The relation fits in the call summary as conditional pairs, which replaces the per-exit alternatives Section 4 proposed and needs no cap.
3. The extension keeps every binding-level rule, fact and consumer. The replacement offers only performance gains that the guard doesn't require, at the cost of reopening the whole corpus.

## 7. Implementation sequence

Every slice lands as one reviewed change on top of the previous one. Durable regressions assert the diagnostic site as well as the verdict, at the layer that owns the behaviour. Fixture manifests change only in the slice whose rule changes them.

### Slice 0: Owner decisions

Recorded from the owner's review of revision `0c5e64c9f`:

| Question | Decision |
| --- | --- |
| The extension and its conservative rejections | Approved, with sparse allocation and field facts in the existing checker. The four limits above are accepted with their scope stated, so they can't absorb a required positive. The dense candidate is set aside because it failed 5 of the 18 guards, including one without aliasing. |
| No new borrow diagnostic code | Confirmed while every transfer completes with a sound result. A finished analysis that finds a possible alias uses the existing diagnostic. Stopping before a mandatory result fails closed, with no acceptance and no partial summary. A future hard cutoff reopens this decision. |
| Imported constants as trees | Approved only where sharing is unobservable. An import-specific change in observable behaviour is declined. C1 requires a runtime materialisation to preserve sharing within its graph, while each materialisation may still be fresh. C4 must test an exported folded value separately from an imported const function evaluated by the consumer, each with one graph holding the same child twice and one holding two equal, distinct children. If a supported consumer can tell the local and imported forms apart, C4 preserves that distinction across the interface. The public representation needn't change before slice 1. |
| C4 store work in parallel | Approved. One freeze transaction covers the ordered result slots, with shared definitions and rollback if a later slot fails. Tests must show that pre-existing store rows survive and no partial result becomes visible. C4 also checks what the existing tree walkers do with a compact graph that references one child repeatedly, since charging each definition once doesn't bound a consumer that expands every occurrence. Shared runtime lowering waits until the ordinary checker passes its controls. |

Implementation returns to the owner if a required positive can't be preserved soundly, a hard analysis limit becomes necessary or an imported value shows a sharing difference the proposed interface can't represent. The performance guard is unchanged.

The [C4 import witness](./const-functions-evaluation-foundation-and-semantics-plan.md#c4-integrate-required-evaluation-and-close-publication) exposed a difference inside a const consumer. The owner approved graph-local allocation definitions in exported folded values and consumer-owned remapping. Independent materialisations remain fresh. This resolves the import stop and allows Slice 1 to resume in parallel. Shared runtime lowering remains gated on both checker slices passing on their implementing revisions.

The approved contract decisions at the top of this document already cover `runtime_detached`, `runtime_retained` and the hashmap fixture.

### Witness ownership

Each witness belongs to the first slice whose rule decides it. Later slices must keep it.

| Owner | Must accept | Must reject at the recorded site |
| --- | --- | --- |
| Slice 1 | W3 `22`, W1c `20`, L1 `4`, L3 `1 / 2`, L4 `12`, `failed_set`, B1p `21`, B2 projected alias with a fresh holder, H3 `2`, P1 `22`, P2 `21`, P4 `21` and the ordinary form of P3 `2` | W1, W1b, W4, L2, B1, H1, `inline_ctor_branch`, `lexical_dynamic`, `permission_write`, the option-binding report (`tmp/stage5/optbind`), the ordinary twins of both `runtime_call_*_rejected` fixtures, `hashmap_alias_struct_field_param_return` and ordinary `runtime_detached` |
| Slice 2 | W7a and W7c with `left=2 right=1 original=1` for both flags, `runtime_call_disjoint_result_fields` with `left=1 right=2 second=1`, and `retain` and `forward` callers without conflicting uses | W7b, W7d, `success_store` and `failure_store` |
| C4 shared lowering | P3's const form rendering `2`, H3 through the real lowering, and the import witnesses wherever sharing is observable | The const forms of `runtime_call_fresh_box_forward_rejected` and `runtime_call_identity_fields_rejected`, and H1 through the real lowering |
| Outside BC-06 | The multi-bind report (`tmp/bc06/rebind_ordinary_multibind`) belongs to HIR multi-bind lowering | |

The const forms of the two committed `runtime_call_*_rejected` fixtures and `runtime_call_disjoint_result_fields` fail on `7cc05c61b`, as the BC-06 plan records. They stay failing until their owner lands.

### Slice 1: Sparse allocation layer

| Item | Content |
| --- | --- |
| Files | `analysis/borrow_checker/state.rs` (new layer type beside `LocalState`, sorted-vector joins and equality), `engine.rs` (join, block-exit retirement), `transfer/access.rs` (`transfer_assign_target` strong update, construction edges, `copy`), `transfer/access/statement.rs` (map `remove` and `set`, `transfer_invoke` per-lane stores, `transfer_never_call`), `transfer/access/conflicts.rs` (`check_mutable_access` reachability and edge permission), `hir/expressions.rs` (`is_aggregate_construction`), `return_alias.rs` (the candidate's constructed-return rule) |
| Prerequisite | Slice 0 decisions, recorded above. Start from `7cc05c61b`, not from the candidate. Port the kept pieces by hand, with `tmp/bc06/holder_edges_candidate.patch` as the reference. |
| Deletions | The candidate pieces listed as deleted above. Discard the uncommitted candidate once its kept pieces land. |
| Witnesses | The slice 1 row of [Witness ownership](#witness-ownership) |
| Corpus | `corpus_verdicts.py record` and `compare` against `tmp/bc06/corpus_7cc.json`. Explain every changed verdict against the canonical rules and update the manifests that the approved decisions change. |
| Performance | `measure_s6.py` with the slice build in place of the candidate. All 18 inputs must pass the guard against the archive, and the counters must be reported. |
| Stop | Any guard failure, any unexplained newly accepted program or any change to a published fact outside the layer's constructs. Report and stop, don't tune past the guard. |

### Slice 2: Field-precise call summaries

| Item | Content |
| --- | --- |
| Files | `return_alias.rs` (`FunctionReturnAliasSummary` becomes per-slot field sources with sharing pairs, plus `classify_function_return_alias`, `merge_return_alias` and `project_alias_summary_through_arguments`), `public_call_summary.rs` (per-lane store effects, `validate_public_call_summary`), `metadata.rs` (`build_public_call_summaries`, `stabilize_return_alias_summaries` as a validated monotone fixed point), `call_semantics.rs`, `transfer/access/statement.rs` (`transfer_call_result_alias` instantiation), `public_interface/interface_validation.rs`, `public_interface/local_finalization.rs`, `module_compilation/generated/convergence.rs` (transition validation, plus revalidation and rescheduling after `refresh_private_failure_lanes`), the external ABI `ExternalHirReturnAlias` and Boracle's `CallResultProvenance` |
| Prerequisite | Slice 1 |
| Deletions | The constructed-return `AliasParams` special case, now covered by field sources. The inequality-only stabilisation cap. |
| Witnesses | The slice 2 row of [Witness ownership](#witness-ownership), and every slice 1 witness still holds |
| Corpus and performance | As slice 1. Also report `convergence_summary_changes` and `convergence_summary_comparisons` against the baseline. |
| Stop | A summary transition the validator rejects, a convergence count growing without a matching new summary component, or a guard failure |

### Slice 3 onwards: C4

C4 owns these. They're listed for ordering only:

- the freeze transaction, shared rows and rollback through `stage_values`, after slice 0 and in parallel with slices 1 and 2, with the tests and the tree-walker check that [Slice 0](#slice-0-owner-decisions) requires
- the import witnesses from Slice 0, before any change to the public representation
- shared lowering through writable prelude temporaries, after slice 2, owning the C4 row of [Witness ownership](#witness-ownership)
- C5 measurements, then Phase 5.

### Validation sequence

For each slice, in order:

1. `cargo test -q -p moth --lib borrow_checker` and the owning layer's new regressions.
2. The witness runner, `bash tmp/bc06/witness/run.sh <compiler> <witness directories>`, with every witness in the slice's tables.
3. The corpus comparison and the performance guard, as the slice table says.
4. `just boracle` as the independent lane. Its known disagreements on W3 and L4 are recorded here and don't block.
5. `just validate` as the integrated checkpoint.
6. `just validate-full` before merge, then Slice review.

Record the revision, the witness results, the corpus differences and the guard table in this document's resume point after each slice.
