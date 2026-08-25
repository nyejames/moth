# Tests

Looking for: real contracts with no owner, coverage under the wrong owner, and assertions too weak to catch the failure they imply.

Read [testing standards](../../src/developer-docs/style-guide/testing.mtf) in full before running this kind.

**Evidence bar:** a named contract and what does or does not protect it. Test count and line coverage prove nothing.

## Build the contract inventory first

From canonical docs and the [progress matrix](../../src/docs/progress/@page.moth), list the supported success and failure contracts, target and command variations, and the internal invariants that normal output cannot observe. Separate supported from partial, experimental and deferred - deferred surfaces do not need coverage.

## One primary owner per contract

- user-visible language or project behaviour → integration case under `tests/cases/`
- pure data or local invariant → focused unit test beside the owner
- stage-boundary orchestration → minimal pipeline or build smoke test
- backend artefact behaviour → backend-specific assertions
- hidden side-table or IR fact → unit test only when external behaviour cannot expose it
- cross-backend parity → one input, backend-specific expectations

Record contracts with no owner, contracts with several competing owners, and tests stored beside convenient implementation rather than the semantic owner.

## Manifest policy

Every case has a unique stable ID and path, meaningful tags, a valid role, and a contract unless it is a smoke case. Each contract has at most one primary case. Helper files stay inside their owning case. Paths stay relative and inside the suite root. Metadata is consumed directly, never reconstructed from names or paths.

Use the suite-audit command, but do not treat its inventory as a substitute for reading the cases.

## Positive coverage

Ordinary use, the minimal valid form, a representative non-trivial form, cross-module use where visibility or identity matters, and the command and target paths that claim support. Runtime output where behaviour is only observable after execution. Emitted artefacts where structure is part of the contract.

Avoid multiplying near-identical happy paths. One strong case may own several syntax forms of one behaviour.

## Negative and boundary coverage

Malformed syntax, invalid placement, wrong visibility, duplicate declarations, type mismatch, missing provider, boundary and empty values, overflow and capacity edges, invalid control-flow joins, borrow and alias conflicts, unsupported reachable target features, and user input that must diagnose rather than panic.

A failure case must prove the contract through stable diagnostic identity and the source context that matters - not merely that compilation failed.

## Assertion quality

The common defect is a test that passes for the wrong reason. Look for:

- "must fail" assertions where an exact code is the contract
- missing multiplicity or source-location checks where those are contractual
- success assertions that would pass on empty or degenerate output
- tests asserting implementation shape rather than observable behaviour
- coverage of a path the production entry point no longer reaches

A test that cannot fail is worse than no test, because it reads as protection.

## Benchmarks are not coverage

Benchmark fixtures never count as correctness coverage.

## Findings do not edit tests

This kind may authorise a test change; other kinds may not. When a Correctness or Diagnostics finding needs coverage, it links here and both must be accepted before either lands.

## Stale when

Supported behaviour, the manifest, test ownership, or the suite policy changes materially.
