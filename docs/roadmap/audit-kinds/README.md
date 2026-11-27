# Audit Kinds

One audit run covers one kind. Read the [audit guide](../audit-guide.md) first - it owns scope, recording, preservation and validation. These files own **what to look for**. Correctness uses the lightweight [bug queue](../../bug-reports.md) and coverage log, while other kinds retain formal reports.

| Kind | Looking for |
|---|---|
| [Correctness](./correctness.md) | Wrong acceptance, rejection, panic, invariant or handoff |
| [Diagnostics](./diagnostics.md) | Right legality, wrong error identity, context, wording or recovery |
| [Performance](./performance.md) | Measured time or memory cost, plus unmeasured efficiency leads without speed claims |
| [Redundancy](./redundancy.md) | Duplicated ownership, obsolete paths, unearned layers |
| [Tests](./tests.md) | Missing, misplaced or weak coverage of a real contract |
| [Style](./style.md) | Code that is correct but hard to read, review or extend |
| [Comments](./comments.md) | Missing, stale or noisy local intent |
| [Documentation](./documentation.md) | Design drift, inaccurate or missing context, poor navigation or prose quality |

## Picking the kind

Choose by the **observed problem**, not the likely patch:

- wrong result, panic or broken invariant → Correctness
- correct result, poor error → Diagnostics
- measured cost, or provably avoidable work without a speed claim → Performance
- duplicated ownership, obsolete paths or unearned structure → Redundancy
- a contract nothing protects → Tests
- unclear code → Style
- unclear intent around clear code → Comments
- inaccurate, incomplete, redundant or unclear documentation → Documentation

An unrelated discovery belongs in [bug reports](../../bug-reports.md), even when it concerns another audit kind. Record the location, observed concern and next investigation, then resume the original scope. It does not create another audit run or record coverage for the other kind. A later focused audit may take it up formally.

A broad sweep is several separate runs, not one mixed report.

## Before filing an in-scope finding

1. Check the canonical owner and the progress-matrix status. Deferred is not broken.
2. Inspect the whole path, not the first suspicious line.
3. State the strongest counter-explanation and why it fails.
4. Check open findings and active plans for the same root cause.
5. Name what would disprove the finding.

Each checklist adds kind-specific evidence requirements on top of these. Incidental leads need only a concrete reason for suspicion and explicit uncertainty, not a complete investigation.
