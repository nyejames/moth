# Audit Log

This file is the concise coverage database for structured codebase audits. It defines stable audit scopes and records the rough freshness of each audit kind. It does not store findings or process rules.

- [Audit guide](./audit-guide.md)
- [Open audit findings](./open-audit-findings.md)
- Completed audit reports live under `audits/`.

## Freshness markers

| Marker | Meaning |
|---|---|
| `N` | No complete audit is recorded. |
| `P YYYY-MM AUD-####` | The audit covered only part of the declared scope. |
| `C YYYY-MM AUD-####` | The audit covered the complete scope and has no known material invalidation. |
| `S YYYY-MM AUD-####` | The audit was complete when recorded but material changes have made it stale. |

Freshness describes audit coverage, not code quality. A current scope may still have open findings.

## Scope registry

Scope kinds and boundary rules are defined in the [audit guide](./audit-guide.md).

| Scope ID | Name | Kind | Primary coverage | Required context | Exclusions |
|---|---|---|---|---|---|

## Audit freshness

| Scope ID | Style | Comments | Correctness | Diagnostics | Tests | Redundancy | Performance | Documentation |
|---|---|---|---|---|---|---|---|---|
