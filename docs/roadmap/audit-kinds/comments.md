# Comments

Find missing, misleading, stale or noisy explanations of **why the code is shaped as it is**. A reader should understand important local intent without tracing every distant producer and consumer.

Follow the [audit guide](../audit-guide.md) for formal findings and coverage. The [style guide](../../developer-docs/style-guide/style-guide.mtf) owns comment standards. Record incidental bugs in [`docs/bug-reports.md`](../../bug-reports.md) and return to this kind.

**Evidence bar:** identify the precise fact, invariant or decision a reader cannot reliably recover, or the false statement they would trust. A request for more comments without a named purpose is not a finding.

## 1. Map the intent before judging the prose

Read the area's `mod.rs`, file headers, key types and execution path. Check the relevant architecture and current ownership where a comment makes a non-local claim. Ask:

- What does this module own and exclude? What do its callers supply and consumers assume?
- Which constraints determine data representation, ordering, stage placement or lifetime?
- Which branches, failure paths and fallbacks would look accidental without context?
- Which previous migrations or fixes might have left outdated explanations?

Assess comments against the **current contract and code**, not their apparent plausibility. A search hit for `TODO`, `SAFETY`, `temporary` or a renamed stage is a lead until verified.

## 2. Check explanations at the right level

### Module and file entry points

- Every handwritten Rust source file starts with concise module documentation, including small files. Keep an obvious leaf file's header short rather than omitting it.
- A multi-file module's `mod.rs` explains its responsibility, important exclusions, file roles and principal data flow. It should guide a new reader to the right owner.
- Headers explain purpose and relevant neighbours, not merely repeat the filename, list every item or copy an architecture document.
- Comments use current type, module and stage names. They do not claim ownership of work performed elsewhere.

### Types, records and identities

Check important structs, enums, IDs, tables, side tables and results. Can a reader tell:

- What the value represents, which stage owns it and when it is valid or immutable?
- Whether fields are authoritative, derived, cached or aligned with another store?
- Which ID domain permits comparisons and whether values may cross module, worker or persistent boundaries?
- Why a non-obvious layout, invariant, lifetime or ownership choice is required?

Explain a real constraint at its declaration or nearest enforcing operation. Field-by-field narration of clear names adds no value.

### Execution and failure paths

Inspect major functions, matches, loops, stage handoffs and unusual branches for landmarks that explain:

- The overall steps, why their order matters and where authority changes.
- Why a validation or error belongs here rather than at a later consumer.
- Why a fallback is conservative, a branch is intentionally empty or independent work may continue after failure.
- Where deterministic order is recovered after parallel processing.
- Which assumptions justify `unwrap`, `expect`, unchecked indexing, `unreachable!` or `unsafe`.

The code must still enforce its invariants. A comment cannot make an unsafe or incorrect path valid.

## 3. Hunt for high-value non-local intent

Focus on statements whose reason lives outside the current function:

- A producer publishes a fact once because consumers must not reconstruct source semantics.
- A local ID must be remapped before another module can compare it.
- A failed producer must not publish a partial interface or cache success.
- A mandatory proof rejects input while an optional optimisation proof falls back.
- A clone preserves independent ownership or a frozen snapshot beyond its source lifetime.
- Similar-looking target paths remain separate because their capabilities or failure contracts differ.

For each, ask whether the comment identifies the **specific constraint** and its local consequence. For example, `// Needed for safety` is too vague to protect an invariant. A comment naming the value's lifetime and the consumer that outlives its producer is actionable.

## 4. Remove noise and correct stale claims

Look for:

- Comments that narrate syntax, repeat names or describe a trivial getter, loop or branch.
- Large banners, changelog prose, abandoned plans or implementation history that Git already records.
- TODOs without a current action, reference or relevant remaining limitation.
- Claims of safety, performance or temporary compatibility without a verifiable reason.
- Descriptions of deleted passes, renamed types, old paths, changed stage order or ownership that moved.
- Comments promising immutability, caching, recovery or deterministic order that the code no longer provides.

Prefer deleting redundant prose. Prefer an obvious rename or simpler control flow when the comment only compensates for unclear code. Route that structural change to [Style](./style.md) or [Redundancy](./redundancy.md) rather than demanding a paragraph to explain it.

## 5. Challenge and report findings

Before filing a Comments finding, check whether nearby code or module documentation already conveys the fact and whether the proposed wording will remain accurate through ordinary changes. The strongest counter-explanation may be that a comment would duplicate an enforceable type or a more authoritative document.

For each formal finding, record:

- Exact source location and the comment or code that needs context.
- Missing or false intent, with the producer, consumer or invariant that establishes it.
- How a future reader could misunderstand or break the code.
- The smallest useful comment action: add a local explanation, correct wording or delete noise. Route a needed rename to Style.
- Why existing nearby context is insufficient and how to verify the proposed statement.

A demonstrated broken invariant is a [Correctness](./correctness.md) issue. Misleading public or canonical prose belongs to [Documentation](./documentation.md). Log such out-of-scope discoveries in `docs/bug-reports.md` without widening this run. Audits never edit code or comments themselves.

## Stale when

Module ownership, significant types or identities, stage ordering, principal control flow, invariants or the terminology explained by local comments change materially.
