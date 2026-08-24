# Audit Log

What has been audited, where, and when. This is a record, not a permission list.

- [Audit guide](./audit-guide.md)
- [Audit-kind index](./audit-kinds/README.md)
- [Open audit findings](./open-audit-findings.md)
- [Audit reports](./audits/README.md)

## How to use this

**Looking for work?** Scan for areas with no entry for a kind, or an old one. Never-audited code is the best default target.

**Finished an audit?** Add or update the row for the area you covered. Record only the kind you ran.

**Area not listed?** Add it. An audit registers the area it needs as part of its run - see the audit guide. Do not stop because a row is missing.

**Changed code materially?** Mark the affected row stale. Ordinary implementation never records new coverage.

## Entry format

Each entry is `Kind YYYY-MM AUD-####`, optionally followed by a qualifier:

| Qualifier | Meaning |
|---|---|
| *(none)* | The whole area was inspected for that kind. |
| `partial` | Only part of the area was inspected. The report says which part. |
| `stale` | Was complete when recorded, but the area has changed materially since. |

Coverage is not quality. An audited area may still have open findings.

## Audited areas

| Area | Covers | Audited |
|---|---|---|
| `tests.harness` | `src/compiler_tests/integration_test_runner/**`, `src/compiler_tests/{test_support,test_fs,test_diagnostics}.rs`, `src/compiler_frontend/tests/frontend_pipeline_tests.rs`. Excludes the fixture directories, which `tests.cases` owns. | — |
| `tests.support` | Test-only support and helper modules under `src/**/tests/` and `src/**/test_support.rs` | Redundancy 2026-08 AUD-0001 `partial` |
| `tests.cases` | `tests/cases/manifest.toml` and every `tests/cases/*/` fixture | — |
| `build.stage0` | `src/build_system/create_project_modules/**` - source discovery, preparation, module identity and graph, wave scheduling and publication | Performance 2026-08 AUD-0002 `partial` |
| `feature.runtime_assertion_messages` | Assertion messages and call arguments end to end: `ast/expressions/{call_arguments,call_argument,call_validation}.rs` and `ast/statements/asserts.rs` through AST finalization and HIR validation into the JS and Wasm backends | Correctness 2026-08 AUD-0003 `stale` |
| `docs.audit_framework` | This file, `audit-guide.md`, `audit-kinds/**`, `open-audit-findings.md` and `audits/**` | Documentation 2026-08 AUD-0004 `partial` |

## Never audited

Areas with no row above and no coverage of any kind. This list is deliberately coarse - it exists so the gap is visible, not to partition the codebase in advance. Take one, name the part you can actually cover, and add a row.

AUD-0004 measured 771 of 791 production `.rs` files as having no owner under the registry taxonomy that preceded this log. That figure is a historical ownership measurement, not a recount of the areas below under the current model.

- `src/compiler_frontend/tokenizer/**` - tokenization
- `src/compiler_frontend/ast/**` - AST semantics, constant folding, templates and TIR
- `src/compiler_frontend/hir/**` - HIR lowering and validation
- `src/compiler_frontend/headers/**`, `symbols/**`, `module_compilation/**`
- `src/compiler_frontend/compiler_messages/**` - diagnostic construction and rendering
- `src/backends/**` - JS and Wasm lowering, backend feature validation
- `src/build_system/**` outside `create_project_modules/`
- `src/projects/**`
- `docs/roadmap/**` outside `docs.audit_framework` - plans, the roadmap and benchmark records
