# Style

Find correct Rust whose presentation or local structure makes Moth harder to read, review, extend or safely change. The goal is **obvious ownership and data flow**, not minimum lines or personal taste.

Follow the [audit guide](../audit-guide.md) for formal findings and coverage. The [style guide](../../developer-docs/style-guide/style-guide.mtf) owns implementation standards. Use the [Redundancy guide](./redundancy.md) for duplicated ownership or obsolete layers. Log incidental issues in [`docs/bug-reports.md`](../../bug-reports.md).

**Evidence bar:** show specific code, the ambiguity or unnecessary mental work it creates and a smaller, clearer alternative that preserves the same contract. `rustfmt` preferences and unsupported claims that something is simply "unidiomatic" are not findings.

## 1. Read the path as a new maintainer

Open the area's `mod.rs`, relevant file headers, public entry points and primary data types. Follow one normal path and at least one error or exceptional path. Ask:

- Can the reader locate the owner without searching through unrelated helpers?
- Do names, inputs, results and side effects tell the same story?
- Can the main function be read as a sequence of meaningful steps?
- Which detail forces a reader to jump between files, expand a dense expression or memorise unrelated state?
- Do the current tests reveal an API whose shape is easy to misuse?

Trace the real caller before recommending a new helper, type or module. A familiar Rust pattern is not automatically suitable for this compiler boundary.

## 2. Check module organisation and data shapes

### File and module boundaries

- A multi-file module's `mod.rs` maps its responsibility and the role of its children. It is not a dumping ground for implementation.
- Files group coherent tasks. Split mixed responsibilities when their changes and invariants are independent, not to enforce a line limit.
- Avoid both sprawling files with unrelated stages and tiny forwarding modules that make one operation hard to follow.
- Keep visibility and re-exports as narrow as the actual caller set requires. Test code belongs in its owning `tests/` directory.
- Names follow the actual stage and responsibility after a move. Avoid broad `utils` modules and misleading generic names.

A proposed split should name what moves, which owner retains it and the resulting input and output boundary.

### Records and APIs

- Prefer explicit data and state transitions to objects hiding several stages of behaviour.
- Group data that shares a purpose and lifetime. Avoid catch-all context objects that conceal unrelated dependencies.
- Use meaningful enums for meaningful alternatives instead of combinations of booleans or magic values.
- Use named result types where tuple positions are hard to distinguish. Keep IDs visibly tied to their issuing domain.
- Prefer borrowing for read-only access when the ownership contract allows it. Avoid accidental conversion chains at call boundaries.
- Keep traits, generics and macros when they make a **current** operation clearer, not merely because Rust supports the abstraction.

A record-oriented design is not a mandate to split every struct into parallel arrays or to wrap every value in a new type.

## 3. Inspect functions and control flow

Walk the longest or most central functions, then their callers. Look for:

- Names covering only part of a function's responsibility, overly broad arguments or functions that mix unrelated phases.
- Nested `map` / `and_then` chains hiding errors, mutable iterator closures or stateful combinators that obscure execution order.
- Large inline matches without conceptual groups, deeply nested branches and duplicated success or error paths that are hard to compare.
- Dense builder or conversion chains where named intermediate values would expose ownership and provenance.
- Long argument lists where several values share one purpose and lifetime, or context structs created solely to hide dependencies.
- Multiple tiny helpers whose names do not explain a meaningful step and whose navigation exceeds their benefit.
- Early returns or no-op branches whose meaning is unclear without tracing distant state.

Prefer explicit loops for multi-step operations, named intermediates for complex expressions and small functions that isolate real responsibilities. Keep a long cohesive transformation together when splitting it would scatter its state.

## 4. Review names, errors and local Rust habits

- Use descriptive domain terms and consistent producer-consumer naming. Avoid `ctx`, `ty` and other opaque abbreviations outside tiny scopes.
- Import long paths at module scope where useful. Avoid import aliases that obscure which stage owns a type.
- Use spacing and short landmarks for major steps, not decorative banners or commentary on obvious syntax.
- Keep user-facing `DiagnosticDraft`, operational `InfrastructureFailure` and proven internal `compiler_bug!` lanes visibly separate. Do not hide failures behind broad fallbacks.
- Examine unexplained `unwrap`, `expect`, unchecked indexing, `todo!`, debug printing and lint `allow` attributes. Verify the claimed internal invariant before classifying a style issue.
- Look for `.clone()`, `.to_owned()` and temporary buffers used to avoid clearer borrowing or moving. Check the lifetime before suggesting their removal.

If invalid input can actually crash, data can be corrupted or a failure enters the wrong lane, route a [Correctness](./correctness.md) lead to `docs/bug-reports.md`. Runtime cost claims require [Performance](./performance.md) evidence. Missing explanation of otherwise clear code belongs to [Comments](./comments.md).

## 5. Test the simplest alternative

Compare the current code with the least disruptive correction:

1. Rename a misleading function, binding or type.
2. Expose the operation with a named intermediate or straightforward branch.
3. Inline a pointless local helper, or extract one that names an actual step.
4. Narrow a parameter or introduce a meaningful stage-local state/result type.
5. Split a mixed-responsibility file without moving semantic ownership.

Ask what new concepts, indirection or duplication the alternative adds. Reject clean-looking rewrites that hide side effects, couple lifetimes or merely relocate complexity. A distinct target boundary or longer but explicit operation may be intentionally clearer.

## Findings, coverage and staleness

For each formal Style finding, provide the code location and caller context, the concrete reading or misuse cost, the strongest reason to keep the existing form, the proposed smallest correction and the invariants to preserve. Include relevant nearby examples so reviewers can compare before accepting the change. Group manifestations of one local style problem without claiming an unrelated architectural refactor.

Record which modules, APIs and control-flow paths were actually read. A file list or formatting check does not establish complete style coverage.

Coverage becomes stale when module organisation, principal APIs and types, naming conventions or the control-flow shape of substantial functions change materially.
