# BC-06 redesign investigation

This companion to [BC-06: Relational production provenance](./production-borrow-checker.md#bc-06-relational-production-provenance) records the investigation into projection-precise ordinary borrow checking. It ends at design review. It holds the reduced witnesses, expected outcomes, reproduction commands and decisive measurements, so another session can resume without the local `tmp/bc06/` directory.

**Owner direction (2026-11-25):** production implementation is paused. Investigate the design and return one reviewable design and implementation sequence. The holder-edge candidate stays uncommitted and preserved. Before this direction the two plans disagreed: the BC-06 handoff said "resume const C4 on the existing checker", while [C4](./const-functions-evaluation-foundation-and-semantics-plan.md#c4-integrate-required-evaluation-and-close-publication) stayed paused until ordinary access checking preserves correlated sharing and permission. This investigation now gates both.

**Approved contract decisions:** `hashmap_alias_struct_field_param_return` and the ordinary `runtime_detached` function must reject at their mutation with `MOTH-BORROW-0007`. The ordinary `runtime_retained` function must accept and render `22`. Fixture manifests change only in the slice whose rule changes them. The owner approved the design with the [Slice 0 decisions](#slice-0-owner-decisions).

## Resume point

| Item | Value |
| --- | --- |
| Branch | `docs/const-foundation-profile-independent-semantics` |
| Published baseline | `7cc05c61bf5d4b6de23abfeec8a67d9623be971d` |
| Candidate | Set aside. The holder-edge diff on that base (9 files, +697/-63) is preserved as the reference `tmp/bc06/holder_edges_candidate.patch` (sha256 `289e0a74…6d83d81`). |
| Toolchain | rustc and cargo 1.99.0, Apple M1 Pro, Node v24.21.0 for rendered output |
| Sessions complete | 1 (Sections 1-2: evidence and witnesses), 2 (Section 3: fact-lifetime ledger), 3 (Section 4: calls and publication), 4 (Sections 5-7: C4 consumption, design choice and implementation sequence) |
| Last established result | Root-granular holder edges can't tell the field replacement witness from its negative twin. They keep a removed element's edge, an edge stored by a failed `set` and an edge replaced through a mutable alias. Facts are recorded at the event, so a binding may retire only after nothing reachable names its identity. Today's call summary is one parameter-index union per function, so no checker can separate W7a from W7b. Boracle's reference and oracle both reject W3 by modelling a stored reference as a place loan. C4 loses sharing twice, at the per-slot freeze and at per-occurrence lowering. The candidate fails the performance guard on 5 of the 18 workloads with identical work counters, so dense per-local edge sets are ruled out. The chosen design adds a sparse allocation layer to the existing checker. See the conclusions of [Session 1](#session-1-conclusions) and Sections [3](#section-3-conclusions), [4](#section-4-conclusions), [5](#section-5-conclusions) and [6](#section-6-conclusions). |
| Next task | Slice 1, the sparse allocation layer, from `7cc05c61b`. C4's freeze transaction, rollback and import witnesses run in parallel. |
| Next unanswered question | Can slice 1 meet its witness row within the unchanged performance guard? |

## 1. Evidence inventory

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
5. **Other data.** Slot order and types keep `validate_call_results`. Structural strings and resources keep their existing visits and charges, and a shared definition is charged once. Imported provider values keep tree projection unless the owner extends the C1 sharing contract across packages. Scalar-returning functions such as `const_detached` and `const_retained` publish no graph, so they stay independent of this work.

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
| Collection elements | All elements of one collection share one field path | W1c moves one element out with `remove`, which is a strong update of the result |

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
