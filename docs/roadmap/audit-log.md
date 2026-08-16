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

## Audit freshness

Skip `-` cells during automatic selection.

| Scope ID | Style | Comments | Correctness | Diagnostics | Tests | Redundancy | Performance | Documentation |
|---|---|---|---|---|---|---|---|---|
