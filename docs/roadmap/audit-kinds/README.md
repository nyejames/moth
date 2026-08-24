# Audit Kinds

One audit run covers one kind. Read the [audit guide](../audit-guide.md) first - it owns scope, recording, findings, preservation and validation. These files own only **what to look for**.

| Kind | Looking for |
|---|---|
| [Correctness](./correctness.md) | Wrong acceptance, rejection, panic, invariant or handoff |
| [Diagnostics](./diagnostics.md) | Right legality, wrong error identity, context, wording or recovery |
| [Performance](./performance.md) | Measured time or memory cost |
| [Redundancy](./redundancy.md) | Duplicated ownership, obsolete paths, unearned layers |
| [Tests](./tests.md) | Missing, misplaced or weak coverage of a real contract |
| [Style](./style.md) | Code that is correct but hard to read, review or extend |
| [Comments](./comments.md) | Missing, stale or noisy local intent |
| [Documentation](./documentation.md) | Inaccurate public, canonical, status or routing prose |

## Picking the kind

Choose by the **observed problem**, not the likely patch:

- wrong result, panic or broken invariant → Correctness
- correct result, poor error → Diagnostics
- measured cost → Performance
- unmeasured repeated work or dead structure → Redundancy
- a contract nothing protects → Tests
- unclear code → Style
- unclear intent around clear code → Comments
- prose that misleads → Documentation

A finding discovered under one kind that belongs to another becomes a **linked finding** in that lane. It does not expand the current run or record coverage for the other kind.

A broad sweep is several separate runs, not one mixed report.

## Before filing under any kind

1. Check the canonical owner and the progress-matrix status. Deferred is not broken.
2. Inspect the whole path, not the first suspicious line.
3. State the strongest counter-explanation and why it fails.
4. Check open findings and active plans for the same root cause.
5. Name what would disprove the finding.

Each checklist adds kind-specific evidence requirements on top of these.
