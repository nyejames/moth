# Core collection sorting implementation plan

## Purpose

Add one compiler-owned `sort` member to growable and fixed collections. The source API is stable by
default, mutates in place and accepts compile-time `stable` and `memory` policies. Target-specific
algorithm names stay private so implementations can improve without changing Moth programs.

This plan starts the next focused thread of Core package work. It extends `@core/collections` and the
existing collection builtin path without adding a broad package runtime, a general algorithm
framework or a second call parser.

## Current-state capsule

```text
STATUS: queued
CURRENT_SLICE: Phase 0 - refresh collection builtin, ordering and target helper owners
BLOCKERS: mixed JavaScript and Wasm lowering must deliver final collection layouts, shared page memory and target helper integration
NEXT_ACTION: activate in an isolated worktree after the mixed backend prerequisite lands, then run Phase 0
```

Establish the active revision, branch, worktree state and validation baseline in untracked working
notes when implementation starts. Do not pin a queued plan to a commit.

## Roadmap position

This plan runs immediately after HTML mixed JavaScript and Wasm backend work and before package
dependency declarations and package-manager foundations.

Implementation runs in its own branch and worktree. It must not share generated files, temporary
state or uncommitted changes with the active diagnostics worktree.

At closeout, delete this plan and remove its roadmap entry in the same commit.

## Hard prerequisites

- function-level JavaScript and Wasm partitioning is delivered
- the HTML builder has one page-local Wasm runtime and shared memory
- fixed and growable collection layouts and operations work in HTML-Wasm
- target helper selection and reachability are explicit
- target validation rejects unsupported reachable operations before lowering
- named and default argument routing has one shared AST owner
- collection mutation, borrow validation and retained-edge summaries are stable
- Core package capability identities and fingerprints are deterministic

The plan does not block on runtime `Byte`, every `NumberN` target or a public ordering trait. Its
required initial runtime surface is `Int`, finite `Float` and `Char` on HTML-JS and HTML-Wasm.

## Required authorities

Read these from the active worktree before implementation:

- `AGENTS.md`
- `docs/compiler-design-overview.md`, especially compiler-owned symbols, call-shaped syntax, HIR,
  borrow validation, link facts, target validation and backend handoff
- `docs/build-system-design.md`, especially Core packages, mixed-target planning, physical variants,
  memory plans and page runtime memory
- `docs/src/developer-docs/language/overview.mtf`
- the canonical collection literal, fixed collection and collection operation references
- the canonical call, parameter default, numeric operator and trait scope references
- `docs/src/docs/packages/core/collections/collections.mtf`
- `docs/src/developer-docs/memory-management/overview.mtf` and its routed access, retained-edge and
  backend-lowering references
- `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf`
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`
- `docs/src/docs/progress/@page.moth`
- `docs/roadmap/roadmap.md`
- `index.md` as a locator only

## Current implementation snapshot

Refresh this snapshot during Phase 0. At the queued revision:

- collection members are compiler-owned operations resolved by the AST
- the surface contains `get`, `set`, growable `push`, fixed `push`, `remove` and `length`
- collection operations lower through stable `@core/collections` identities
- JavaScript uses arrays for growable collections and branded wrappers for fixed collections
- HTML-Wasm collection lowering is deferred
- compiler-owned builtin member arguments are positional-only
- `Int`, finite `Float` and `Char` have natural ordering
- `String` and `Bool` do not have natural ordering
- sortable collections are documented as deferred
- retained-edge analysis describes stored values by their direct obligations

Use the delivered mixed-backend owners rather than preserving any path named only by this snapshot.

# Accepted design

## Source surface

The declaration-like contract is:

```moth
sort type T |
    this ~{T},
    stable Bool = true,
    memory SortMemory = SortMemory::Automatic,
|
```

This is a compiler-owned member rather than a source function copied into each module. The same
member is available on fixed `{N T}` collections. A fixed collection sorts only its logical occupied
range and keeps its capacity.

`SortMemory` is a closed compiler-owned policy choice:

```moth
SortMemory ::
    Automatic,
    Minimal,
;
```

It uses normal nominal choice identity and unit-variant semantics. The compiler reserves the type
and makes it visible wherever compiler-owned collection members are visible. Do not encode it as a
`String`, `Bool`, integer or backend flag.

Representative calls:

```moth
~values.sort()
~values.sort(stable = false)
~values.sort(memory = SortMemory::Minimal)
~values.sort(stable = false, memory = SortMemory::Minimal)
```

Positional calls remain valid, though documentation should prefer names for policy arguments:

```moth
~values.sort(false, SortMemory::Minimal)
```

The receiver is not an authored argument slot. `this` documents its mutable receiver contract.

## Observable behaviour

`sort`:

- sorts in ascending natural order
- mutates the existing collection and returns no value
- requires explicit mutable receiver access
- is infallible in Moth source semantics
- accepts neither postfix `!` nor `catch`
- traps or aborts on allocation exhaustion instead of returning `Error!`
- leaves empty and one-element collections unchanged
- preserves logical length, fixed capacity and collection identity
- leaves every input element identity in the collection exactly once
- invokes no user code and has no re-entrant callback path

`stable = true` guarantees that elements which compare equal keep their original relative order.

`stable = false` releases that guarantee and permits a faster or lower-memory implementation. It
does not promise that equal elements will be reordered. A backend may still choose a stable
implementation.

Targets must agree on the order of non-equal elements. Equal elements follow the requested stability
contract.

## Memory policy

`SortMemory::Automatic` lets the backend choose scratch storage and algorithm details from hard facts
such as element layout, logical length, fixed capacity, stability and target capability.

`SortMemory::Minimal` asks for the lowest auxiliary-memory implementation available for the selected
stability contract. It is an optimisation preference, not a byte limit or zero-allocation promise.

V1 may map `Automatic` and `Minimal` to one implementation. That is valid only when both values are
accepted, type checked and retained in the normalised policy, documentation states that no distinct
low-memory stable path exists yet and the backend boundary can distinguish them later without a
source API change.

Do not expose scratch byte counts, allocator choices, recursion limits, run thresholds or algorithm
names as source parameters.

## Static policy requirement

`stable` and `memory` are compile-time policy arguments.

Accepted values include omitted defaults, direct literals or variants and compile-time constants
that fold to `Bool` or `SortMemory`. A runtime binding or expression that does not fold before HIR
produces a structured diagnostic.

The compiler must not emit both algorithm families and branch on a runtime policy value. The shared
call owner performs positional and named routing, duplicate detection, default insertion and normal
type checking. A focused sort-policy resolver then requires folded values and normalises them.

Existing compiler-owned members remain positional-only unless their own accepted contract changes.
Adding `sort` must not make `get`, `set`, `push`, `remove`, `length`, map members or arbitrary external
calls accept named arguments.

## Natural ordering eligibility

V1 uses existing compiler-owned natural ordering. It adds no comparator, key extractor, callback,
operator overload or public ordering trait.

One target-independent compiler owner classifies a semantic element `TypeId` into a normalised
natural-order kind. Every later consumer uses that fact instead of maintaining its own type list.
Target validation separately decides whether the selected backend supports that kind.

Required initial eligible types are:

- `Int`
- finite `Float`
- `Char`
- transparent aliases of those types

An already-delivered `NumberN` or `Byte` may join the same classification only when its canonical
natural ordering exists before Phase 2. This plan must not invent that ordering as a backend detail.

Initial ineligible types include `Bool`, `String`, structs, choices, options, collections, maps,
external opaque types and unconstrained generic parameters.

`String` stays ineligible because Moth has no accepted natural String ordering. Do not silently pick
UTF-8 byte, Unicode scalar, locale or JavaScript code-unit order.

A generic body may call `sort` only when the compiler can prove a natural order under the accepted
generic contract. V1 has no source ordering bound, so an unconstrained generic element is rejected.
A later static ordering design may remove this restriction.

The unsupported-element diagnostic carries the semantic element `TypeId`, the `sort` operation and
the call location. It must not carry a copied parse type or appear as a backend error.

## Selection boundary

Compile-time facts include:

- semantic element type and natural-order kind
- selected target element layout
- growable or fixed collection shape
- fixed capacity where present
- requested stability and memory policy
- target and profile capabilities

Runtime observations include logical length, natural runs, duplicate frequency, partition balance and
merge wins. The implementation may adapt to those observations without adding `mostly_sorted`,
`clumpy`, `entropy`, `few_unique`, range or size parameters.

## Stable algorithm baseline

The stable v1 family is a natural mergesort with Powersort merge scheduling.

It must:

- scan for natural ascending and strictly descending runs
- reverse only strictly descending runs so equal values are never reversed
- extend short runs with stable binary insertion sort
- compute merge powers with bounded integer arithmetic
- maintain the Powersort stack invariant
- merge adjacent runs stably
- reuse scratch sized to the smaller merged run where the selected layout permits it
- run in linear time on already sorted and reverse-sorted input
- preserve `O(n log n)` worst-case comparisons
- bound ordinary scratch to at most about half the logical element count

Adaptive galloping may land only when target benchmarks show a material win without making the merge
owner difficult to review. A later Driftsort-style hybrid, radix path or counting path may use this
same source API but is not part of v1.

## Unstable algorithm baseline

The unstable v1 family is an in-place introspective partition sort.

It must use insertion sort for tiny partitions, robust pivot selection, explicit equal-element
handling, bounded recursion or a compact work stack and heapsort fallback before quadratic behaviour
is possible. It must preserve `O(n log n)` worst-case behaviour without a proportional scratch
buffer.

Pattern breaking, branchless block partitioning and sorting networks land only with benchmark and
code-size evidence. Do not copy a large named implementation when a smaller introspective sorter
meets the contract.

## Thresholds and future specialisation

Thresholds are target implementation details kept in one target-local policy owner. Future versions
may add fixed small-`N` networks, radix or counting paths, a Driftsort-style stable hybrid or a
distinct low-scratch stable implementation. These require no new ordinary `sort` arguments.

## Compiler and package ownership

The collection member owner recognises `sort`, validates mutable receiver access, routes arguments,
inserts defaults, folds policies, classifies natural order and builds typed AST.

HIR carries one explicit normalised collection-sort operation containing equivalent facts to:

```rust
pub(crate) struct CollectionSortPolicy {
    pub(crate) stability: SortStability,
    pub(crate) memory: SortMemoryPolicy,
    pub(crate) order: NaturalOrderKind,
}
```

Exact Rust names may change. The facts may not be dropped, rebuilt from source or encoded as backend
helper names.

Use one focused HIR sort statement or an existing typed collection-operation representation that can
carry the same policy. Do not create both. Do not migrate every working collection operation to a
new hierarchy only to make sort look uniform.

`@core/collections` owns the stable target helper capability. Per-function link facts record reachable
sort requirements. The backend maps those requirements to an implementation without deciding source
legality or reclassifying the element type.

If the mixed backend delivers a general typed collection-operation HIR owner, extend it. Otherwise
add the smallest focused sort representation.

## Helper identity and reuse

A selected helper key includes target, element layout, natural-order kind, stability implementation,
selected memory implementation and collection representation when layout differs.

The raw source request and selected implementation are separate facts. When v1 maps both memory
policies to one helper, generated artefacts reuse it. Do not duplicate identical JS or Wasm helpers
because the source used a different no-op policy. A later distinct `Minimal` implementation becomes
part of helper and physical variant identity.

## JavaScript contract

The JavaScript path sorts the underlying item array for both collection shapes, uses explicit Moth
comparison and preserves fixed wrapper identity, capacity and logical length. It must not use
JavaScript's default lexicographic ordering.

Stable or unstable logic is selected before runtime. Helpers are emitted by reachability and shared
across call sites. A host native sort may be used only after conformance tests prove the exact Moth
order and stability contract and benchmark evidence shows it is better. Source semantics must not
depend on an engine-specific algorithm or undocumented memory use.

## Wasm contract

Use the final mixed-backend architecture. Preferred implementation order is:

1. structured backend-owned Wasm LIR helper generation
2. a Rust-authored helper compiled to Wasm only when it imports page runtime memory, consumes the
   final collection ABI and owns no independent allocator
3. handwritten WAT only for a small benchmark-proven kernel whose structured LIR is materially worse

Do not hand-write the complete stable or unstable sorter in WAT by default.

Wasm helpers operate on the final logical collection layout, use selected element size, alignment and
move facts, import page runtime memory and use the runtime allocator only for planned temporary
scratch. Stable scratch is released on every normal exit. Allocation exhaustion follows the existing
trap policy. The emitted binary is validated and helper requirements participate in reachability and
physical variant fingerprints.

An embedded helper cannot own separate memory, assume a private heap, bypass the
`ValidatedMemoryPlan` or reinterpret collection handles.

## Borrow, lifetime and retained-edge contract

`sort` is one exclusive whole-collection mutation. A live shared element result or collection alias
prevents it. The receiver requires explicit `~` on an existing mutable place. Reactive observers use
the established whole-collection invalidation rule.

Sorting is a permutation. It adds no retained edge, removes no retained edge, creates no cleanup
frontier and preserves the multiset of element obligations, element allocation identities, the
collection allocation family and its storage domain.

Scratch handles are temporary borrowed transport rather than persistent collection edges. The
implementation invokes no user code and has no recoverable failure after mutation starts, so it may
move handles through scratch without publishing intermediate topology. It must not clone an element
graph, double-drop a handle or alter REC obligations.

A representation that requires retain or release work during relocation receives one balanced
compiler-owned move plan. A backend must not infer that work from byte copies.

## Failure boundary

Policy and element diagnostics happen before HIR. Unsupported target capability fails during target
validation. Scratch allocation exhaustion traps. Malformed compiler-owned layout or helper metadata
is `CompilerError`.

There is no recoverable path that observes a partly sorted collection. An unrecoverable trap does not
promise restoration of the original order.

# Non-goals

- no public algorithm names
- no comparator, key extractor, closure or function-value design
- no public ordering trait or user-defined ordering
- no String ordering decision
- no descending-order parameter
- no data-shape, range, cardinality or size hint
- no exact scratch-byte limit or stable zero-allocation promise
- no parallel parameter or execution-policy design
- no partial sort, selection, top-k, binary search or merge API
- no general package algorithm framework
- no direct user dependency on the private Core host surface
- no independent Wasm memory or allocator for Core helpers

# Implementation rules

- Preserve one call parser and parameter-slot owner.
- Keep one natural-order classifier.
- Keep policy facts typed through AST and HIR.
- Select static policies before runtime.
- Leave working collection operations on their current path unless consolidation removes code.
- Keep thresholds local to the target implementation that consumes them.
- Use structured diagnostics for user input and unsupported element types.
- Keep tests outside production files and benchmarks outside correctness ownership.
- Review third-party provenance and licensing before adapting algorithm code.
- Add no compatibility wrappers or parallel sort representation.
- Do not edit generated documentation directly.

Each code-bearing phase ends with focused tests, the integration audit, `just validate`,
`git diff --check`, a Slice review and one coherent commit.

# Phase 0 - Refresh owners and baseline

## Goal

Re-anchor the plan after mixed JavaScript and Wasm work lands.

## Work

- [ ] Read every required authority in the isolated worktree.
- [ ] Record HEAD, branch, status and active worktrees in untracked notes.
- [ ] Confirm each hard prerequisite.
- [ ] Inventory final collection layouts, builtin parsing, AST, HIR, borrow facts, retained-edge
  summaries, link facts, target validation and helper emission.
- [ ] Inventory compiler-owned choice identity and default folding for `SortMemory`.
- [ ] Inventory natural scalar comparison classification on both targets.
- [ ] Inventory `@core/collections` registration, tests and benchmarks.
- [ ] Search for stale whole-module Wasm, private-memory or target-rejected collection paths.
- [ ] Choose the smallest single HIR representation that retains the complete sort policy.
- [ ] Record baseline `just validate` and `just bench-check` results without hiding unrelated
  failures.

Suggested searches:

```bash
rg -n 'CollectionBuiltinOp|collection_builtin|@core/collections' src docs/src/docs
rg -n 'CollectionGet|CollectionSet|CollectionPush|CollectionRemove|CollectionLength' src
rg -n 'NaturalOrder|ordering|comparison' src/compiler_frontend src/backends
rg -n 'collection.*wasm|Wasm.*collection|fixed_collection' src/backends src/projects
rg -n 'sortable collections|sort\(' docs/src/docs tests/cases
```

# Phase 1 - Add the policy type and source call contract

## Goal

Establish the complete source API before sorting reaches HIR.

## Work

- [ ] Add canonical reserved `SortMemory` identity and normal unit variants.
- [ ] Reuse normal choice representation, canonical identity, remap and display owners.
- [ ] Expose it through the compiler-owned collection surface without a direct host-package import.
- [ ] Reject redeclaration and shadowing through the reserved-name diagnostic owner.
- [ ] Add one declarative builtin signature for `sort` with named slots and defaults.
- [ ] Route arguments through the shared call parser and slot owner.
- [ ] Insert both defaults through the normal default path.
- [ ] Accept positional, named and mixed forms under ordinary routing rules.
- [ ] Keep every existing builtin positional-only.
- [ ] Require both policies to fold and normalise them to compiler enums.
- [ ] Diagnose dynamic policies with parameter identity and source location.
- [ ] Add `sort` to both collection shapes as mutable, infallible and unit-returning.
- [ ] Reject postfix `!` and `catch` through the normal infallible-call path.

## Coverage

- [ ] Omitted, positional, named and mixed policy calls
- [ ] Duplicate, unknown, out-of-order, extra and wrong-type arguments
- [ ] Direct variants and folded constants
- [ ] Runtime Bool and runtime `SortMemory` rejection
- [ ] Reserved-name collisions
- [ ] Existing builtin named-argument rejection
- [ ] Mutable receiver, fixed/growable, postfix `!` and `catch`

# Phase 2 - Classify natural order and carry sort through HIR

## Goal

Create one complete backend-neutral sort operation and its analysis facts.

## Work

- [ ] Add one target-independent natural-order query over semantic `TypeId`.
- [ ] Reuse numeric and Char comparison semantics and normalise transparent aliases.
- [ ] Admit required scalar types and reject unsupported types with one typed diagnostic family.
- [ ] Reject unconstrained generic elements without adding an ordering trait.
- [ ] Carry collection shape, element type, order and source policy in typed AST.
- [ ] Add or extend the single HIR representation chosen in Phase 0.
- [ ] Add HIR validation, display, remap, side-table and debug handling.
- [ ] Treat sort as an exclusive whole-collection mutation in borrow validation.
- [ ] Add normal reactive invalidation.
- [ ] Record a no-change retained-edge permutation summary.
- [ ] Preserve allocation-family and element identity facts.
- [ ] Add per-function sort helper requirements and target validation.
- [ ] Include selected implementation facts in runtime and physical variant identities when emitted
  artefacts differ.

## Coverage

- [ ] Eligible scalars and transparent aliases
- [ ] Grouped ineligible type families and generic parameter rejection
- [ ] Policy preserved from AST to HIR with no runtime policy value
- [ ] Live element borrow conflict and last-use release
- [ ] No retained-edge or cleanup-frontier change
- [ ] Reachability, helper deduplication and target rejection
- [ ] One natural-order owner and one HIR path

# Phase 3 - Implement stable and unstable algorithm kernels

## Goal

Build readable, bounded algorithm families before target integration.

## Stable work

- [ ] Implement natural run discovery and strict descending reversal.
- [ ] Add stable binary insertion extension for short runs.
- [ ] Implement overflow-safe merge-power calculation and Powersort stack rules.
- [ ] Implement stable adjacent merging with reusable smaller-run scratch.
- [ ] Preserve equal-key order through every path.
- [ ] Keep thresholds in one policy owner.
- [ ] Add galloping only with repeatable benchmark evidence.

## Unstable work

- [ ] Implement tiny-partition insertion sorting.
- [ ] Add robust pivot selection and explicit equal-element handling.
- [ ] Bound recursion or use a compact work stack.
- [ ] Track bad depth or imbalance and fall back to heapsort.
- [ ] Keep the implementation in place with no proportional scratch.
- [ ] Add pattern breaking or block partitioning only with evidence.

## Coverage

- [ ] Empty, singleton and threshold boundaries
- [ ] Sorted, reverse, all-equal and low-cardinality inputs
- [ ] Many short runs and highly unbalanced run sizes
- [ ] Organ-pipe, sawtooth and pivot-adversarial inputs
- [ ] Stable tagged equal-key order
- [ ] Forced unstable fallback
- [ ] Property tests for sortedness, permutation and stability
- [ ] Overflow-safe power, index and capacity calculations
- [ ] No convenience full-input clone

# Phase 4 - Add JavaScript lowering

## Goal

Run the source contract through HTML-JS with explicit Moth ordering and reachable shared helpers.

## Work

- [ ] Lower normalised policies to stable or unstable JavaScript helpers before runtime.
- [ ] Reuse one helper when both memory policies select the same implementation.
- [ ] Sort growable arrays and fixed wrapper item arrays without changing wrapper state.
- [ ] Implement explicit `Int`, finite `Float` and `Char` comparison.
- [ ] Use delivered Number or Byte comparison only when semantically classified and target-supported.
- [ ] Emit no default JavaScript lexicographic sort.
- [ ] Emit helpers only when reachable and share them across call sites.
- [ ] Remove the replaced JavaScript target rejection.

## Coverage

- [ ] One primary Moth case covering defaults and both overrides
- [ ] Exact output for random, sorted, reverse and duplicate-heavy input
- [ ] Fixed and growable runtime output
- [ ] Explicit scalar ordering rather than host coercion
- [ ] Helper presence, absence and deduplication
- [ ] No one-off algorithm body per call site

# Phase 5 - Add Wasm lowering

## Goal

Run the same contract through HTML-Wasm using final collection layouts and shared page memory.

## Work

- [ ] Choose the accepted Wasm implementation route in Phase 0 preference order.
- [ ] Generate or link helpers by element layout, order and selected algorithm.
- [ ] Operate on logical occupied elements for both collection shapes.
- [ ] Use layout-aware load, store, swap and temporary move operations.
- [ ] Use page runtime memory and allocator for stable scratch.
- [ ] Release scratch on every normal exit and keep unstable sorting free of proportional scratch.
- [ ] Preserve retained-edge and cleanup obligations while handles move.
- [ ] Include selected helper implementation in capability and physical variant identity.
- [ ] Remove the completed sort target rejection and validate emitted Wasm.
- [ ] Add no private helper memory, allocator or host comparator callback.

## Coverage

- [ ] Run the primary source under HTML and HTML-Wasm
- [ ] Cross-backend parity for non-equal ordering
- [ ] Stable tagged-key backend tests
- [ ] Fixed and growable layouts
- [ ] Stable scratch allocation and normal cleanup
- [ ] Unstable no-proportional-scratch evidence
- [ ] Helper reuse, variant separation, imports and one shared memory
- [ ] Canonical Wasm binary validation

# Phase 6 - Benchmark and prune

## Goal

Set only evidence-backed thresholds and remove speculative complexity.

## Work

- [ ] Measure tiny, small, medium and large logical lengths around each threshold.
- [ ] Measure random, sorted, reverse, natural-run, all-equal, low-cardinality and adversarial data.
- [ ] Cover supported scalar layouts and both collection representations where layout differs.
- [ ] Record elapsed time, comparisons where practical, bytes moved, peak scratch, code size and
  helper count.
- [ ] Keep raw results local and track only a concise rationale for retained choices.
- [ ] Confirm stable sorted and reverse input is effectively linear.
- [ ] Confirm unstable adversarial input reaches bounded fallback.
- [ ] Keep galloping, pattern breaking, block partitioning or sorting networks only when they earn
  their code.
- [ ] Confirm memory modes share one helper while no distinct implementation exists.
- [ ] Add no single-machine hard performance gate.

Use `just bench-check` for non-recording evidence and `just validate` for completion.

# Phase 7 - Document, review and close out

## Goal

Make the contract canonical and leave one clean Core collection path for later package work.

## Documentation

- [ ] Add the stable default and one named policy example to the Basic collection operations page.
- [ ] Add the complete sort, eligibility, infallibility and permutation contract to the Advanced
  collection operations page.
- [ ] Update Core collections docs with final helper identity and JS/Wasm support.
- [ ] Remove sortable collections from deferred package text.
- [ ] Update call docs with the explicit policy-bearing builtin exception to positional-only calls.
- [ ] Update compiler architecture with durable policy, HIR, natural-order and link-fact ownership if
  missing.
- [ ] Update build architecture only for a real missing Core helper or physical variant contract.
- [ ] Update routed memory docs with permutation and scratch transport rules.
- [ ] Update the progress matrix, cheatsheet and `index.md` where required.
- [ ] Rebuild generated docs through the compiler.

## Package-thread and Slice review

- [ ] Search for duplicate argument routing, order lists, policy enums, helper registries and target
  rejection paths.
- [ ] Keep existing collection methods on their current path unless consolidation removes code.
- [ ] Confirm no general package algorithm abstraction was added for one use.
- [ ] Delete temporary adapters, stale comments, obsolete fixtures and unused helper variants.
- [ ] Review diagnostics and tests for one clear owner per contract.
- [ ] Record only concrete follow-ups such as comparator/key design, low-scratch stable sorting,
  partial selection or binary search.
- [ ] Mark relevant audit-log rows stale only where their recorded scope materially changed.
- [ ] Run the final Slice review from `AGENTS.md`.

## Final validation

```bash
cargo fmt --all
cargo run --quiet -- tests --audit
just validate
just bench-check
git diff --check
```

Record exactly what ran and any unrelated failure. Delete this plan and its roadmap bullet in the
completion commit.

# Completion conditions

The work is complete when:

- mutable growable and fixed collections expose `sort`
- ascending stable order is the default
- `stable = false` releases only the stability guarantee
- `SortMemory::Automatic` and `SortMemory::Minimal` are typed compile-time API values
- both policies survive through backend selection even when v1 maps them to one implementation
- natural-order eligibility has one target-independent compiler owner
- v1 adds no comparator, key, ordering trait or String order
- stable and unstable implementations meet their correctness and worst-case contracts
- HTML-JS and HTML-Wasm lower supported reachable sorts
- borrow and lifetime systems model sorting as an exclusive permutation
- helper reachability and deduplication are explicit
- documentation and progress report actual support
- the Slice review finds no duplicate parser, type list, compatibility path or speculative framework
- final validation passes or unrelated failures are recorded honestly
- this plan and its roadmap entry are removed in the completion commit
