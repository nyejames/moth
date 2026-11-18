# Correctness

Find cases where the compiler accepts, rejects, transforms, schedules or publishes data differently from Moth's accepted contract. Audit the whole relevant path, including failures and cross-stage handoffs, not only the most visible implementation.

Follow the [audit guide](../audit-guide.md). **Correctness bugs go directly to [`docs/bug-reports.md`](../../bug-reports.md)** and audit coverage goes to the [audit log](../audit-log.md). Do not create a formal `AUD-####` report. Incidental issues outside this kind or area use the same bug queue without extending coverage.

**Evidence bar:** a confirmed bug needs a violated contract or enforced invariant and a concrete input, execution path or state trace that violates it. Record a credible unproven risk as an explicitly labelled investigation lead, not a demonstrated bug. Design preferences, unsupported assumptions and missing deferred features are not correctness findings.

## 1. Establish what must happen

Start from canonical documentation and the [progress matrix](../../docs/progress/@entry.moth). Read the chosen area's entry points, its owner and relevant tests. Establish:

- Which inputs are valid, what outputs or effects they require and what must be rejected.
- Which compiler stage validates each rule, produces each fact and owns each failure.
- Which consumers depend on the fact and what they may assume after validation.
- Which identities, ordering, immutability, lifetime and publication guarantees apply.
- Which behaviour differs legitimately by target, builder or command and which must remain identical.

Separate accepted design from current support and implementation. A feature that remains deferred is not a defect, but accepting an unsupported operation or violating a supported boundary can be. Existing tests establish the executable baseline, not permission to contradict the design.

## 2. Trace the entire owned path

Follow an input or state transition from its producer through the consumers that can observe it. Ask:

- Does validation happen before any consumer relies on it, including early exits and alternative entry paths?
- Does the responsible stage compute each semantic fact once, or can another path apply different rules?
- Can a rejected or failed producer still publish an interface, output, cache entry or apparently valid placeholder?
- Can a transformation lose an error edge, source origin, result slot, identity, ownership constraint or required ordering?
- Do CLI, tooling, direct compiler services and builders enter the same validation owner where the contract requires parity?

Inspect only the boundaries relevant to the chosen area:

- **Stage 0:** source discovery, project and package graphs, preparation scheduling and publication, not source semantics.
- **Preparation and AST:** tokenization and declaration-shell parsing run once. AST owns semantic resolution, folding, generic requests and AST-local TIR. Later stages do not reparse source or reconstruct an earlier owner's meaning.
- **HIR and analyses:** HIR carries validated backend-facing semantics. Borrow, lifetime and memory analyses consume owned facts and side tables without silently rewriting the language contract.
- **Targets and outputs:** target validation precedes lowering. Backends implement validated semantics rather than reconsidering source legality. Build and link planning consume compiler facts rather than rediscovering AST or source structure.

One wrong producer can affect several consumers. Trace to the first violated invariant before recommending a fix.

## 3. Actively search for counterexamples

For each important rule, compare the nearest valid and invalid cases. Vary only one meaningful condition at a time so an unexpected result has an identifiable cause:

- **Input shape:** empty, missing, repeated, nested, malformed, boundary-sized and unusually large inputs.
- **Control flow:** success, failure, early return, unreachable code, conditional branches, loops, retries and failed generated work.
- **Ownership:** local versus imported data, public versus private interfaces, generated versus authored sources and one module versus several.
- **Execution:** compile-time versus runtime evaluation, debug versus release, cached versus fresh work and single-worker versus parallel scheduling.
- **Capabilities:** supported versus unavailable target features, different builders and relevant `check`, `build` and `dev` paths.
- **Host inputs:** native paths, platform-specific names, non-UTF-8 components, invalid resources and operational failures where the area touches them.

Use existing focused tests and safe reproductions to distinguish a real difference from an imagined one. Do not change production code, tests or fixtures during an audit. If a minimal permanent regression test is needed, record it as the next step in the bug entry.

## 4. Check state and identity invariants

Ask whether invalid intermediate states can reach an observable boundary:

- Every semantic fact has one authoritative owner. Derived indexes or snapshots cannot drift from it.
- Published interfaces, artefacts and generated sidecars are immutable and success-only.
- Arena indexes, typed ranges, side tables and parallel stores remain within their issuing owner and checked capacity.
- Donor-local IDs are remapped before publication. Cross-module comparisons use stable semantic identities, never coincidentally equal local integers.
- Generated request and cache keys include every semantic input that changes their meaning. Failed results cannot be reused as successes.
- Dependency and lifetime graphs retain their required edge direction, topology, ordering and termination properties.
- Frozen reports retain source and type-display identity through their last use. Source paths, semantic paths and output paths are not conflated.

Look for sentinel IDs, untyped indexes, independently mutable duplicate fields, unchecked conversions and flags that permit contradictory states. Establish the broken invariant before reporting the representation itself as a defect.

## 5. Inspect failures and incomplete work

Follow both the failure itself and the state left behind:

- User-authored invalid input produces an owning `DiagnosticDraft`. Operational failures use typed `InfrastructureFailure`. Only proven-impossible internal states use `compiler_bug!`. No malformed user input should panic.
- A diagnosed module exposes no partial semantic interface. Failed work cannot leave a success-shaped result in a registry, graph, cache or builder output.
- Failed generated requests block their real consumers without poisoning unrelated independent work. An unchecked or blocked consumer is never counted as compiled.
- Recovery may continue only with trustworthy facts. A placeholder must not silently pass later validation as real evidence.
- Missing mandatory legality or topology proof rejects through the correct user-facing lane. Missing optional optimisation proof falls back conservatively without rejecting legal source.
- Error classification, propagation, source attribution and reporting order survive early exits, parallel merges and stage handoffs.

An inaccurate message or weak suggestion with otherwise correct acceptance belongs to [Diagnostics](./diagnostics.md). Wrong legality, missing rejection or a source failure reported as an internal crash belongs here, even when the eventual fix improves a diagnostic.

## 6. Determinism, concurrency and reuse

Where the audited area runs more than once or in parallel, check:

- Compilation results, diagnostic order, generated identities and manifests do not depend on worker completion or hash-map/filesystem traversal order.
- Shared registries have a single owner or a deterministic merge, with remapping complete before reading worker-produced data.
- Each module, generated request, entry activation and output is processed exactly as many times as its contract requires.
- Iterative worklists reach a justified fixed point and terminate. A silent iteration cap or premature convergence is suspicious.
- Reuse invalidates on changed providers, config, target capabilities, semantic fingerprints and layout requirements. Process-local IDs and host paths do not accidentally become persistent identities.

A missing cache or slower algorithm is not automatically a correctness defect. Incorrect cache reuse, skipped validation and nondeterministic semantic output are.

## 7. Challenge and record discoveries

Investigate suspicious paths such as `panic!`, `todo!`, `unreachable!`, unjustified `.unwrap()`, unchecked indexing, discarded `Result` values, broad fallbacks, duplicate validators, unguarded `unsafe`, arbitrary convergence limits and comments claiming unverified invariants. Each is a search lead until its behaviour or invariant is traced.

Before logging a confirmed bug:

1. Identify the exact accepted contract and relevant current support.
2. Describe the input or state path, expected result and observed or provable deviation.
3. Test the strongest plausible counter-explanation, including valid stage, target or lifecycle distinctions.
4. Check existing bug reports, active plans and historical findings for the same root cause.
5. Name the owning boundary and a focused reproduction or regression check for later work.

Write one concise `docs/bug-reports.md` entry per root cause. Use its flexible `Found`, `Where`, `Observed` and `Investigate` pattern when useful, adding a small repro or source trace when available. Say **unconfirmed** if the path has not been demonstrated. Do not turn a test assertion gap or stylistic concern into a confirmed semantic defect without evidence.

Record complete or partial coverage in `audit-log.md`, with the audited revision and exact exclusions for a partial run. Describe what was genuinely inspected, not merely searched. A clean audit still records coverage. A later fix verifies the behaviour and removes the bug entry in its resolving commit.

## Stale when

Correctness coverage becomes stale when supported semantics, principal algorithms, source preparation, producer-consumer interfaces, identity or ordering, failure classification, caching, concurrency, target validation or borrow and lifetime analysis change materially within the inspected area.
