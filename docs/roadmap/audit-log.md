# Audit Log

This file is the concise coverage database for structured codebase audits. It defines stable scopes and records rough freshness by audit kind. It does not store findings or process rules.

- [Audit guide](./audit-guide.md)
- [Audit-kind index](./audit-kinds/README.md)
- [Open audit findings](./open-audit-findings.md)
- [Audit reports](./audits/README.md)

## Freshness markers

| Marker | Meaning |
|---|---|
| `N` | No complete audit is recorded. |
| `P YYYY-MM AUD-####` | The report covered only part of the declared scope. |
| `C YYYY-MM AUD-####` | The report covered the complete scope and has no known material invalidation. |
| `S YYYY-MM AUD-####` | The audit was complete when recorded but material changes have made it stale. |
| `-` | This audit kind is not applicable to the scope. |

Freshness describes inspection coverage, not code quality. A current scope may still have open findings.

## Scope registry

Scope kinds, completeness and boundary rules are defined in the [audit guide](./audit-guide.md). `Default context` applies to every kind unless `Kind-specific context or exclusions` supplies a compact override such as `Correctness: frontend.ast`.

Use stable conceptual IDs. Leaf scopes own maintained implementation exactly once. Composite, contract and comparison scopes reference existing scope IDs where possible.

| Scope ID | Name | Kind | Primary coverage | Default context | Kind-specific context or exclusions |
|---|---|---|---|---|---|
| `tests.harness` | Integration test harness | Leaf | `src/compiler_tests/integration_test_runner/**` implementation plus `src/compiler_tests/{test_support,test_fs,test_diagnostics}.rs` and `frontend_pipeline_tests.rs` | `docs/src/docs/codebase/style-guide/testing.mtf`, `tests/cases/manifest.toml` | Excludes the 1,700 fixture directories under `tests/cases/*/`, which are data owned by `tests.cases` |
| `tests.support` | Unit test support modules | Comparison | Every test-only support/helper module under `src/**/tests/` and `src/**/test_support.rs`, compared against the `tests.harness` shared helpers | `docs/src/docs/codebase/style-guide/testing.mtf`, the consuming test modules in each subsystem | Compares independent per-subsystem owners for repeated machinery. Comparison does not imply the helpers should be merged. |
| `tests.cases` | Canonical integration fixtures | Leaf | `tests/cases/manifest.toml` and every `tests/cases/*/` fixture directory | `tests.harness`, `docs/src/docs/codebase/style-guide/testing.mtf` | `Redundancy: duplicate coverage between fixtures routes to Tests, not Redundancy` |

## Audit freshness

Skip `-` cells during automatic selection.

| Scope ID | Style | Comments | Correctness | Diagnostics | Tests | Redundancy | Performance | Documentation |
|---|---|---|---|---|---|---|---|---|
| `tests.harness` | `N` | `N` | `N` | `N` | `N` | `N` | `N` | `-` |
| `tests.support` | `N` | `N` | `-` | `-` | `N` | `P 2026-08 AUD-0001` | `-` | `-` |
| `tests.cases` | `-` | `-` | `-` | `-` | `N` | `N` | `-` | `-` |
