# Group-aware place precision and stability-effect research

Status: proposed research, inactive

Owner: Boracle experiments. Production adoption requires a separate accepted decision.

Companions: [Boracle research sequence](./README.md), [production borrow checker tracker](../production-borrow-checker.md)

## Purpose

Investigate whether group-aware place provenance, precise write footprints and structural stability effects can make Moth's borrow checker accept more *provably non-conflicting* operations. The primary inspiration is Zeta's indexed disjointness and capacity-preserving mutation experiments. Valen, Nick Smith's group borrowing, Ante, Carbon and Rust provide complementary design lessons.

This is research material, not an activated implementation package or a change to Moth source semantics. The initial goal is stronger disjointness proofs within Moth's accepted shared/exclusive model, not Valen-style freely mutable aliases.

A successful result identifies a reduced program previously rejected through conservative overlap, proves that its actual accesses do not conflict on any reachable path and preserves every lifetime-topology obligation. An improved physical plan or backend optimisation is a separate measurable outcome.

## Non-negotiable safety and ownership boundaries

- Maintain memory safety and freedom from data races. Shared and exclusive accesses to overlapping live storage remain incompatible. Two concurrent unsynchronised operations are legal only if their actual read/write footprints are compatible or another accepted synchronisation contract establishes safety.
- A stable address is not immutable storage. Proving that an element was not moved or destroyed never authorises a write overlapping an active shared loan or another exclusive loan.
- A proof of different element *slots* does not prove that values stored in those slots reference independent allocations. Moth aggregates may retain the same child allocation through multiple fields or elements. Follow actual projection and alias provenance when operations reach that child.
- Borrow validation remains mandatory on all backends and profiles. GC cannot bypass access checks or mandatory lifetime-topology validation.
- Every allocation family retains one semantic lifetime owner. Retained-edge outlives rules, possible coexistence, declared-region cycles and cleanup-frontier requirements remain unchanged. A reference's containment path is not necessarily its allocation's lifetime-owner path.
- Preserve existing `copy`, alias/write-through, no-shadowing, parameter access and optional affine-transfer semantics. An unproved optional transfer remains a borrow. An unknown overlap, external effect or call result must not become a disjointness proof.
- Borrow checking consumes validated HIR and writes immutable side facts. It does not rewrite HIR, choose physical layouts, split lifetime families, select REC or change backend cleanup. Field-sensitive *access* precision can be valid without physical allocation-family splitting.
- No unsafe escape, unchecked compiler assertion, runtime alias registry, general theorem prover, source reference-mode syntax or universal effect language is approved by this note.
- Stronger source acceptance needs a sound proof under the canonical Moth rules. Any proposed operation whose *access contract* differs from current collection or mutable-receiver semantics requires a separate language-design decision before promotion.

In particular, Valen's permission for independently mutable aliases to the same live object is outside this research's acceptance goal. Its group invalidation rule alone does not establish data-race freedom.

## Three kinds of facts to keep separate

| Fact | Question | Owning analysis |
| --- | --- | --- |
| Access footprint | Which semantic places may be read or written, and can they overlap live loans? | Borrow validation and its existing origins/place relations |
| Stability or invalidation effect | Could an operation remove, relocate, replace or change traversal of storage observed later? | Candidate borrow-level operation facts, based on defined source operations |
| Persistent-edge or lifetime effect | Can an operation create, remove or re-create retained references, and when can their owners be reclaimed? | Existing lifetime-topology, retained-edge and memory-planning owners |

Potential borrow-level detail to test includes writes to known fields, indexed element slots, collection metadata, a collection's backing-store identity and variant payloads. Stability should distinguish element address, element existence, traversal/order and membership. These are candidate semantic distinctions, not a request for a new enum or global effect framework.

An operation can preserve element addresses while changing length and iterator traversal. It can modify a value without moving its allocation. It can destroy a retained edge even when the containing collection object survives. No single `Stable` boolean captures all of these cases.

### Required access proof

For two potentially simultaneous unsynchronised operations A and B, a disjointness argument must establish:

```text
writes(A) intersects (reads(B) union writes(B)) = empty
writes(B) intersects reads(A) = empty
```

Relevant operations also need lifetime and reference-stability proofs. The footprint conditions alone do not establish safe retention, absence of invalidated iterators or compliance with current Moth parameter access contracts.

Path or index evidence must be valid at the actual access event. Reassignment, aliasing, a call that changes the relevant collection structure, an unknown callee or a CFG join can invalidate earlier evidence.

## Investigation directions

### 1. Zeta-style indexed disjointness - first priority

Zeta experiments with proving independence of indexed places through constant indices, loop ranges, arithmetic and runtime guards. Its author also exposes more precise function-place access annotations. The useful lesson is to strengthen positive overlap/disjointness evidence while retaining exclusive mutable access.

Start with the narrowest cases:

1. Distinct constant indices into the same known fixed collection, subject to existing index and place semantics.
2. A dominating `i != j` guard for unchanged indices and the same known collection instance.
3. Simple monotone index ranges such as `j > i` when the proof is valid across the relevant loop back-edges.
4. Otherwise fall back to conservative dynamic-index overlap.

The source sketch below expresses the analysis question, not approved Moth syntax:

```text
if i != j:
    exclusive_access(items[i])
    exclusive_access(items[j])
```

Necessary counterexamples include equal indices, changed indices after the guard, unknown aliases for the collection base, a branch that skips the guard, overflow or bounds uncertainty and loop-carried changes. Distinct indexed places also must not be used to infer independence of an aliased child allocation reached through both elements.

Reuse Boracle's existing `OriginRelations` and `OriginOverlapDecision` authority and its explicit `DynamicIndex` or other precision-loss reasons. Record positive evidence, never interpret a missing relation as disjointness. The reference solver's current conservative dynamic-index baseline remains the baseline until a named experiment has passed review.

### 2. Zeta's capacity-preserving append as a stability case

Zeta describes `push_within_capacity`, which changes an uninitialised element slot and collection length without relocating the backing buffer. A borrow of an existing element might remain structurally valid across such a mutation.

Investigate the *proof obligation*, not the literal API or its implementation:

- Capacity must be sufficient on the successful operation path.
- No existing element moves or is destroyed and the backing-store identity stays stable.
- Writes to the new slot and length are accurately represented. Existing live shared or exclusive access still cannot overlap those writes.
- Any iterator's traversal, length snapshot and membership semantics must be checked separately from address stability.
- Error paths and capacity failure must preserve the promised effects and the ordinary evaluation/commit ordering.

Current Moth collection mutations require exclusive receiver access and `get` protects its receiver base. Therefore a stable-address guarantee *alone* cannot legalise an append across a live element alias. Accepting that pattern may require a narrower compiler-known builtin access contract or a future safe collection API. That is a separate semantic review, not a Boracle-only acceptance patch.

Zeta's sample uses unsafe low-level initialisation. Moth must not reproduce its unsafe assertion or uninitialised-write escape as source syntax.

### 3. Condition and CFG-sensitive access

Extend candidate-conflict slicing with domination and path-compatible facts. Two accesses that only overlap in a path-insensitive union need not conflict if no reachable execution contains both live capabilities. Track how guards, writes, rebindings, success/error edges and loop generations affect the proof.

Build on the inactive conflict-directed and loop-generation proposals instead of another independent solver. Deep analysis should run only for an actual uncertain conflict. Bounded unrolling and a bounded operational oracle can find counterexamples but cannot prove arbitrary loops safe.

### 4. Iterator and projection stability

Classify iterator contracts before proposing new borrow rules. An iterator may depend on element address, existence, traversal cursor, order, membership or a temporarily lent element. Structural stability for one property does not imply the others.

Research cases include mutation of disjoint fields during traversal, fixed-size element mutation, iterator last use before structural change and lending one element at a time. A shared loan to an element still blocks overlapping mutation while active. Do not imply that these iterator APIs currently exist or that every in-place mutation preserves iterator semantics.

### 5. Small interprocedural footprint summaries

Explore whether a narrowly scoped summary can distinguish reading one projection, writing one projection and structurally invalidating descendants at a call. Preserve evaluation order, exact outcome-specific effects, source/generated summary convergence and the immutable public-interface boundary.

Start with compiler-known builtins. Unknown or externally bound calls remain conservative under their closed access contracts. Do not expose donor-local place, HIR, region or allocation-family IDs in public summaries. Do not turn every function signature into a detailed copy of its implementation.

Do not duplicate retention and frontier effects already owned by later lifetime analysis. If a proposed summary affects source-visible acceptance, review its canonical interface and module compatibility consequences explicitly.

### 6. Parallel disjoint access - future validation use

The same footprints could inform a future parallel execution model. Require a separate accepted concurrency and synchronisation contract. Prove read/write non-overlap of the *actual reached storage*, including child references shared between disjoint container slots.

No thread-local shortcut, reference stability flag or single-thread oracle result is a data-race proof. Keep this as a future use of validated facts, not permission to add concurrency syntax or accept otherwise conflicting accesses today.

### 7. Optimisation opportunities, independently measured

Possible consumers include more precise no-alias evidence for proven disjoint places, fewer conservative reloads of unaffected fields, more precise optional final-use transfer and better identification of operations that cannot re-create retained edges.

Each requires its own proof and benchmark. Stable address is not `noalias`. Access precision cannot select physical cleanup, and neither the borrower nor the backend may deduce REC obligations from a new footprint classifier.

## Lessons from related designs

| Source | Useful research lesson | Warning or rejected direction |
| --- | --- | --- |
| Nick Smith / Verdagon's 2025 group borrowing | Child-group invalidation focuses on independently destructible descendants | Original group isolation assumes stronger containment than Moth's arbitrary retained edges. Do not equate group membership with region ownership |
| Valen's October 2026 prototype | Explicit group paths and invalidation sets can model much finer relationships | Aliasable mutation is a different source model. Unimplemented type-stability and planned RC/generational extensions are not validated Moth techniques |
| Zeta's experimental borrow checker | Positive indexed disjointness and no-relocation operations deserve concrete tests | Author-reported implementation is not a proof of soundness. Its unsafe constructs and multi-place source annotations are not Moth design commitments |
| Carbon's 2026 memory-safety design | Separate place sets and invalidation effects, with bounded use of flow sensitivity | Carbon revised earlier approaches and warned against effect annotations that expose excessive implementation detail. Its published 2026 design was not yet implemented as of the July 17 project discussion |
| Ante's stable mutable reference experiments | Differentiate shape-preserving mutation from destruction and address invalidation | Reconsidered `!stable`/`imm` variants show that extra modes can be restrictive and complicate aliasing. An immutable-looking wrapper alone does not guarantee `Send`/`Sync` or race safety |
| Rust's 2026 borrow-checker roadmap | Conditional borrowing, view types, lending iterators and Polonius offer compatible precision directions | A roadmap is not delivered stable behaviour. Do not import Rust lifetime-in-type machinery into Moth |
| Fang's mutation/stability proposal | A stable versus invalidating mutation distinction can guide effect vocabulary | Its proposed `mut`/`mut!` reference capabilities and cyclic graph model are not evidence that Moth needs new syntax or can relax exclusivity |

Treat project blog posts, talks and prototypes as research inputs at their stated dates. Update these notes when implementations or counterexamples provide new evidence, not when a proposal merely acquires a new name.

## Research activation and validation

This note proposes a future investigation sequence, not an added active package 7 or an implicit extension to packages 3-6.

1. **Re-anchor and isolate.** Verify the current branch, merged dense-HIR/native-result prerequisites, existing `BorrowProblem` producer, oracle coverage and canonical memory authorities. Resolve or isolate the tracked `BR-01` shared-alias/exclusive-write reference soundness gap before trusting positive experimental acceptance.
2. **Freeze examples.** Record small source/HIR and normalized positive and negative fixtures for indexed disjointness, field projection, collection mutation, error edges, rebindings and loop paths. Record the existing production and reference verdicts separately.
3. **Try a named experiment.** Add targeted positive disjointness evidence and structured conflict witnesses inside Boracle's existing owner. Preserve ordinary reference-mode behaviour and conservative unknown cases. Compare with an independent bounded operational oracle where the shape is supported.
4. **Evaluate stability effects separately.** Classify selected compiler-known operations without promoting source acceptance. Identify precisely which current receiver-access rules would need a language review for cases such as `push_within_capacity`.
5. **Expand only on evidence.** Consider iterator contracts and narrow call summaries after local proofs survive counterexamples. Consider parallelism only after a separately accepted concurrency model.
6. **Record outcomes.** Report proof cost, candidate conflicts eliminated, newly accepted fixtures, false negatives, oracle counterexamples, diagnostics and effect-summary growth. Measure compile time, memory and generated runtime separately. Either reject the idea or promote a scoped design into its canonical owner and the living production companion.

Minimum negative coverage must include equal or changing indices, aliased collection bases, repeated retained child aliases, unknown call results, map mutation/rehash, variant replacement, collection removal/clear/reallocation, capacity failures, iterator traversal invalidation, CFG joins, loops, fallible commits and concurrent accesses that target the same allocation.

Passing bounded oracle exploration is necessary evidence for an experimental acceptance change where applicable, not sufficient proof of soundness. Unsupported oracle shapes remain inconclusive. No promotion may depend on introducing user-visible unsafe operations, unproven runtime uniqueness, hidden receiver-contract changes or a collector fallback.

## Sources

These links document other projects' stated designs and experiments. They are research references, not Moth authorities.

- [Nick Smith and Evan Ovadia, Group Borrowing (2025)](https://verdagon.dev/blog/group-borrowing)
- [Evan Ovadia, Valen's Memory Safety: A New Kind of Borrow Checking (2026)](https://verdagon.dev/blog/valen-group-borrowing)
- [Zeta borrow-checker exploration, indexed disjointness and push_within_capacity](https://gist.github.com/FlameyosSnowy/a49a35ba006238be55d8c3e410c31061)
- [Jake Fecher, Ante: Stable, Mutable References](https://antelang.org/blog/stable_mutable_refs/)
- [Carbon memory safety deep dive, part 3](https://chandlerc.blog/slides/2026-memory-safety-deep-3/)
- [Carbon July 2026 implementation-status discussion](https://github.com/carbon-language/carbon-lang/discussions/7497)
- [Rust 2026: The Borrow Checker Within](https://goals.rust-lang.org/2026/roadmap-borrow-checker-within.html)
- [Fang: Cyclic References and mutation effects](https://gist.github.com/devast8a/128988bf7dde7911a237e3e37a9e94de)

Canonical Moth owners remain `docs/compiler-design-overview.md`, `docs/src/developer-docs/memory-management/borrow-validation/borrow-validation.mtf`, `docs/src/developer-docs/memory-management/boracle/boracle-reference-solver.mtf` and the lifetime/retained-edge references routed by `docs/src/developer-docs/memory-management/overview.mtf`.
