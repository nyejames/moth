# Checked proof-budget integration note

Status: design awareness only

This note is not an implementation plan and does not accept source syntax. It records one future proposal that every remaining Boracle research package should preserve.

Canonical proposal docs:

- `docs/src/docs/design-scope/checked-proof-budget-blocks-proposal.mtf`
- `docs/src/developer-docs/memory-management/checked-proof-budget-research/checked-proof-budget-research.mtf`

## Proposal boundary

The working idea is an opt-in lexical proof-budget scope, currently called a checked block, that permits much deeper static analysis in exchange for substantially slower compilation.

It must mean:

```text
same source safety rules
same runtime meaning
same ordinary fast analysis first
larger deterministic refinement budget when needed
```

It must not mean:

- code outside the scope is less checked
- a second source safety mode
- a public switch between independent borrow checkers
- a wall-clock timeout
- bounded execution used as acceptance proof
- source control over solver algorithms or numeric budgets
- a runtime checked-block operation

The name and syntax remain open.

## Shared requirements for every package

Current packages do not implement checked syntax. They should leave the following capabilities available:

1. Coarse analysis remains a complete first stage.
2. Expensive work starts from a candidate conflict or another explicit unresolved proof obligation.
3. Precision losses and refusal reasons remain typed.
4. State, path and obligation counts remain deterministic and inspectable.
5. Normal and deeper effort limits can be represented separately later.
6. Exhausting a limit never produces acceptance.
7. A deep result can publish compact proven facts without leaking the whole refined state.
8. Reference and experiment results remain distinguishable.
9. Cross-module reasoning continues to use stable semantic summaries.
10. Borrow legality, lifetime topology and physical memory planning remain separate owners.
11. Production fast paths remain possible even when Boracle keeps explicit slow state.
12. Adding a future checked marker to already accepted code must not make it invalid.

## Package 2: bounded operational oracle

The oracle should be able to test acceptance deltas from both ordinary and future deep refinement experiments.

Requirements:

- record the selected rule set and experiment set on every comparison
- retain complete traces and deterministic bounds
- classify truncation as `Inconclusive`
- never treat complete bounded safety as static proof
- make it possible to compare which refinement effort tier produced a static result
- reduce likely checked-tier counterexamples into durable normalized cases

The oracle remains test infrastructure. It cannot become the implementation of a checked scope.

## Package 3: conflict-directed relational refinement

This package is the main architectural foundation for the proposal.

It should preserve a future effort model such as:

```text
coarse
normal refinement
deep refinement
```

Requirements:

- run coarse solving before every refinement tier
- slice from candidate conflicts rather than refining whole successful functions
- record whether pairwise facts, normal alternatives or deep alternatives were required
- use deterministic state limits
- refuse rather than discharge when a limit is reached
- keep every confirmed witness path-compatible
- allow compact must-alias, must-disjoint and capability facts to leave the refined slice
- avoid hard-coding the source spelling into solver state

The first implementation may expose only an internal effort enum or report category. It should not add a `CheckedBlock` HIR operation.

## Package 4: loop generations and edge last use

Loop analysis may provide some of the clearest reasons for a future deep tier.

Requirements:

- keep the small normal generation abstraction useful on its own
- make delayed widening or larger generation state an explicit later effort choice
- never accept from bounded unrolling alone
- retain written induction or fixed-point arguments for promoted rules
- record when state widening, no-use cycles or loop bounds prevented proof
- keep physical lifetime epochs outside Boracle

A future checked tier may permit more loop-refinement rounds or richer invariants, but the reference rule must remain sound without that source feature.

## Package 5: call summaries and deferred exclusive access

Future deep analysis may instantiate richer summaries in a caller's exact path state.

Requirements:

- keep per-result and outcome-sensitive facts stable and compiler-owned
- preserve separate compilation
- do not open arbitrary dependency HIR as caller-local control flow
- record summary specialisation and SCC effort deterministically
- let unknown boundaries remain conservative
- keep deferred exclusive activation as an independently reviewed rule

A checked caller may eventually authorise more summary specialisation. It must not bypass public semantic interfaces.

## Package 6: aggregate copy and builtin storage provenance

This package should keep ordinary fixed-place rules cheap and explicit.

Requirements:

- fixed fields and fixed indexes remain the normal precision target
- dynamic indexes and map entries remain conservative in the first slice
- preserve typed reasons that could later feed value-sensitive deep refinement
- keep deep copy graph facts independent from allocation-family planning
- keep builtin storage effects compiler-known

A later checked-tier experiment may investigate narrow index inequality or path predicates. That is not part of the first storage package.

## Later lifetime and memory consumers

The source proposal may eventually authorise deeper analysis beyond borrow conflicts, but it does not merge analysis owners.

Possible later consumers include:

- last-use and optional-transfer precision
- lifetime-region and escape proofs
- cleanup-frontier completion
- retained-edge liveness
- proof that REC is unnecessary

Borrow validation still does not choose lifetime owners. Lifetime validation still does not choose physical strategies. The memory planner still does not change source legality.

## Activation checklist

When one of the remaining packages is copied into `docs/roadmap/plans/` and activated:

- read both canonical proposal docs above
- add a short proposal-awareness note to the active plan
- state whether each new capability belongs to the coarse, normal-refinement or potential deep-refinement tier
- use deterministic semantic limits rather than elapsed time
- include proof-effort observations in promotion reports
- keep checked source syntax out of scope unless a separate accepted design plan exists
- review whether compact results can be consumed after the refined scope without retaining full state

## Promotion rule

No analysis becomes checked-block semantics automatically.

Promotion requires:

```text
named Boracle experiment
-> adversarial corpus
-> bounded counterexample search
-> static soundness argument
-> measured deterministic proof cost
-> source-design review
```

The proposal should remain open until Boracle demonstrates a useful class of safe programs that need a deliberately expensive tier and the compiler can explain those proofs clearly.
