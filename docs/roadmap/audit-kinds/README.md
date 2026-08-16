# Audit Kinds

These documents define the focused procedures for structured codebase audits. Read the [Codebase Audit Guide](../audit-guide.md) first. It owns scope selection, authority, preservation, report lifecycle, triage, freshness and verification.

The documents in this directory are repository policy, not agent skills. An audit skill may select a kind and scope, then route to these files. It must not copy or replace their rules.

Implementation-facing audits prefer explicit data ownership, stage-local passes, tables, arenas, side tables and immutable artefacts over object-style hierarchies. Do not force a data-layout change when it weakens readability, ownership or measured performance.

| Kind | Guide | Default complete scope | Required evidence |
|---|---|---|---|
| Style | [style.md](./style.md) | Leaf | Concrete readability, API or data-shape cost against the style guide |
| Comments | [comments.md](./comments.md) | File for partial coverage, leaf for complete coverage | Missing, stale, misleading or noisy local intent |
| Correctness | [correctness.md](./correctness.md) | Leaf or contract | Exact supported contract or invariant plus a violating path or state trace |
| Diagnostics | [diagnostics.md](./diagnostics.md) | Diagnostic-owning leaf, composite or contract | Diagnostic identity, context, propagation, rendering or recovery evidence |
| Tests | [tests.md](./tests.md) | Behaviour owner plus every test surface | Contract-to-test ownership and assertion evidence |
| Redundancy | [redundancy.md](./redundancy.md) | Composite or comparison | Equivalent ownership or behaviour, not textual similarity alone |
| Performance | [performance.md](./performance.md) | Measured call path, leaf or composite | Metric, workload, baseline and attribution evidence |
| Documentation | [documentation.md](./documentation.md) | Authority plus its dependent documents | Exact authority, status, routing, example or terminology conflict |

## Choosing the primary kind

Each finding has one primary kind. Use the observed failure, not the likely patch shape. These rules control when a focused guide discovers an issue owned by another lane:

- wrong acceptance, rejection, panic or invariant state -> Correctness
- correct legality but wrong diagnostic identity, source context, wording or recovery quality -> Diagnostics
- correct behaviour but duplicate ownership, legacy structure or unnecessary layers -> Redundancy
- correct structure but poor local readability -> Style
- missing or weak regression protection -> Tests
- measured time or memory cost -> Performance
- inaccurate public, status or canonical prose -> Documentation
- missing, stale or noisy implementation comments -> Comments

Create linked findings for secondary lanes. A broad codebase sweep is a campaign of separate kind-and-scope reports, not one mixed report.

## Shared finding threshold

Before filing under any kind:

1. Check the canonical owner and current support status.
2. Inspect the complete primary path and required context.
3. Test the strongest reasonable counter-explanation.
4. Check open findings, recent reports and active plans for the same root cause.
5. Name the evidence that proves the concern and what would disprove it.
6. Keep unavailable evidence as a limitation and mark coverage partial where required.

The focused guides add kind-specific checks and evidence requirements.
