# BC-06 redesign investigation

This companion to [BC-06: Relational production provenance](./production-borrow-checker.md#bc-06-relational-production-provenance) records the investigation into projection-precise ordinary borrow checking. It ends at design review. It holds the reduced witnesses, expected outcomes, reproduction commands and decisive measurements, so another session can resume without the local `tmp/bc06/` directory.

**Owner direction (2026-11-25):** production implementation is paused. Investigate the design and return one reviewable design and implementation sequence. The holder-edge candidate stays uncommitted and preserved. Before this direction the two plans disagreed: the BC-06 handoff said "resume const C4 on the existing checker", while [C4](./const-functions-evaluation-foundation-and-semantics-plan.md#c4-integrate-required-evaluation-and-close-publication) stayed paused until ordinary access checking preserves correlated sharing and permission. This investigation now gates both.

**Approved contract decisions:** `hashmap_alias_struct_field_param_return` and the ordinary `runtime_detached` function must reject at their mutation with `MOTH-BORROW-0007`. The ordinary `runtime_retained` function must accept and render `22`. Fixture manifests stay unchanged until the design is approved.

## Resume point

| Item | Value |
| --- | --- |
| Branch | `docs/const-foundation-profile-independent-semantics` |
| Published baseline | `7cc05c61bf5d4b6de23abfeec8a67d9623be971d` |
| Candidate | Uncommitted holder-edge diff on that base: 9 files, +697/-63, preserved locally as `tmp/bc06/holder_edges_candidate.patch` (sha256 `289e0a74…6d83d81`). No untracked files outside the ignored `tmp/`. |
| Toolchain | rustc and cargo 1.99.0, Apple M1 Pro, Node v24.21.0 for rendered output |
| Sessions complete | 1 (Sections 1-2: evidence and witnesses), 2 (Section 3: fact-lifetime ledger) |
| Last established result | Root-granular holder edges can't tell the field replacement witness from its negative twin. They keep a removed element's edge, an edge stored by a failed `set` and an edge replaced through a mutable alias. Published facts are recorded at the event, so retiring a binding is safe only after nothing reachable still names its identity. See [Session 1 conclusions](#session-1-conclusions) and [Section 3 conclusions](#section-3-conclusions). |
| Next task | Section 4: trace one result through a real call, the public summary and the publication boundaries, using W7 and the `retain` twin. |
| Next unanswered question | What is the smallest summary that keeps per-exit, per-field correlation for `select`, and how does it substitute the same actual twice? |

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

## Investigation queue

Sections 4-7 of the assignment still run against the same witnesses:

- Section 4: caller and callee traces through summaries, interface export, generated convergence and the Boracle adapter.
- Section 5: what C4 publication must consume.
- Section 6: design comparison and recovery of the 18-input benchmark manifest. `tmp/bc06/baseline_measure.json` keeps the input fingerprints and raw samples.
- Section 7: the ordered implementation sequence for owner approval.
