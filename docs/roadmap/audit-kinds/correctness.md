# Correctness

Looking for: the area accepting, rejecting, transforming or handing off data differently from the accepted contract.

Judge against canonical documents and the [progress matrix](../../src/docs/progress/@page.moth), never against current behaviour alone. A missing deferred feature is not a defect.

**Evidence bar:** an exact violated contract or invariant, plus a path or state trace that violates it. A design preference is not a correctness finding.

## Establish the contract first

Before reading implementation: valid inputs and required outputs, invalid inputs and the stage that must reject them, which owner creates each semantic fact, and what ordering or identity guarantees apply. Note the tests that form the executable baseline.

## Data flow and invariants

- Each fact computed once, by its owner; later stages do not reparse source or reconstruct what an earlier stage owns.
- Validation runs before the consumers relying on it.
- Immutable artefacts are not mutated after publication.
- Successful artefacts contain no errors; diagnosed providers expose no partial interface.
- Arena, table, side-table and parallel-vector alignment holds.
- Graph shape, ordering and acyclicity hold.
- Data representation makes invalid states hard to construct - watch boolean clusters and independently mutable parallel fields.

## Failure paths

- Malformed user input cannot panic. User-authored failures become `CompilerDiagnostic`; impossible internal states use `CompilerError`.
- Failure leaves no partially published mutable state.
- A blocked consumer is not semantically compiled; a failed generated request blocks only its real consumers.
- Recovery does not fabricate placeholder facts that later look valid.
- Missing mandatory proof rejects; missing optional optimisation proof falls back conservatively.

## Stage ownership

Check the boundaries the area actually touches, not all of them:

- Stage 0 owns discovery, graph and scheduling, not source semantics.
- Tokenization and declaration-shell parsing happen once.
- AST owns semantic resolution, folding and generic requests. TIR stays AST-local.
- HIR is the first backend-facing IR. Borrow and lifetime analyses read validated HIR and write side tables without rewriting it.
- Target validation runs before lowering. Backends do not reconsider source legality or project topology.
- Build and link planning consume compiler-owned facts rather than scanning source or AST.

## Identity and remapping

- Donor-local IDs never escape through public interfaces.
- Stable identities derive from semantic ownership, not source order or thread completion.
- Remapping completes before any consumer reads worker-produced data.
- IDs are not compared or used to index across unrelated domains.
- Cached or serialised forms never treat process-local IDs or absolute paths as identity.
- Generated request keys include every semantic input needed for uniqueness.

Look for sentinel IDs, untyped integer indexes and conversions that discard ownership.

## Determinism and concurrency

- Diagnostics, identities, artefacts and manifests keep stable order regardless of worker completion.
- `HashMap` or filesystem traversal order never reaches deterministic output unnormalised.
- Only independent work runs concurrently; shared registries have one serial owner or a deterministic merge.
- Loops and fixed-point worklists terminate under a documented condition, not an arbitrary cap.
- Entry activation, generated work and output emission happen exactly once.

## Caching

A missing cache is not a defect. Incorrect reuse is.

- Keys contain every semantic and compatibility input.
- Failed or diagnosed artefacts are never reused as successful data.
- Stale data cannot survive a changed provider identity, config, target capability or layout.

## Suspicious paths

`panic!`, `todo!`, `unreachable!`, `.unwrap()` and unchecked indexing on user-influenced paths; ignored `Result` or `Option`; fallbacks that silently accept unknown states; duplicate validators with different rules; `unsafe` with unenforced assumptions; comments claiming an invariant the code does not enforce; retries or convergence caps with no contract.

Each needs a trace to an observable result or broken invariant before it becomes a finding.

## Tests are evidence, not a target

Read them to establish the baseline. Note where implementation bypasses the tested owner, or a passing test asserts too weakly to catch the defect. File a linked Tests finding - do not change tests here.

## Stale when

Supported semantics, principal algorithms, producer-consumer interfaces, identity or ordering logic, error classification, caching, concurrency, target validation, or borrow and lifetime analysis change materially.
