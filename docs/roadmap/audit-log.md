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

Each entry is `Kind YYYY-MM AUD-####`, optionally followed by one or more qualifiers. `partial`
and `stale` may be combined when the inspected subset has since changed materially:

| Qualifier | Meaning |
|---|---|
| *(none)* | The whole area was inspected for that kind. |
| `partial` | Only part of the area was inspected. The report says which part. |
| `stale` | The recorded coverage has changed materially since that audit. |

Coverage is not quality. An audited area may still have open findings.

## Audited areas

| Area | Covers | Audited |
|---|---|---|
| `tests.harness` | `src/compiler_tests/integration_test_runner/**`, `src/compiler_tests/{test_support,test_fs,test_diagnostics}.rs`, `src/compiler_frontend/tests/frontend_pipeline_tests.rs`. Excludes the fixture directories, which `tests.cases` owns. | — |
| `tests.support` | Test-only support and helper modules under `src/**/tests/` and `src/**/test_support.rs` | Redundancy 2026-08 AUD-0001 `partial` `stale` |
| `tests.cases` | `tests/cases/manifest.toml` and every `tests/cases/*/` fixture | — |
| `build.stage0` | `src/build_system/create_project_modules/**` - source discovery, preparation, module identity and graph, wave scheduling and publication | Performance 2026-08 AUD-0002 `partial` `stale` |
| `feature.runtime_assertion_messages` | Assertion messages and call arguments end to end: `ast/expressions/{call_arguments,call_argument,call_validation}.rs` and `ast/statements/asserts.rs` through AST finalization and HIR validation into the JS and Wasm backends | Correctness 2026-08 AUD-0003 `stale` |
| `frontend.tokenizer` | `src/compiler_frontend/tokenizer/**` - lexer, tokens, numeric scanning, text modes, line scanning, newline handling, and the tokenizer test files | Correctness 2026-10 AUD-0005 `stale`; Redundancy 2026-10 AUD-0014 `partial` |
| `frontend.symbols` | `src/compiler_frontend/symbols/**` - string interning with fork/merge/freeze, complete-path interning (`PathId`, `PathInternerBuilder`, `PathTable`), `InternedPath`, identifier and reserved-name policy, compiler-owned symbol preseeding, dependency identities, and the symbols test files | Correctness 2026-09 AUD-0006 `stale` |
| `contract.numeric_profile.frontend_runtime` | Numeric-profile and folded-value handoff across frontend, build/config/template services, HTML-JS lowering, foreign Error projection and integration evidence. Excludes full Wasm scalar execution and unmodified consumers. | Correctness 2026-09 AUD-0007 `partial` `stale` |
| `contract.numeric_profile.regression_tests` | Template fixed-scalar, const-template range and external-JS fallible wrapper regression cases and Rust generated-wrapper tests. Excludes the remainder of `tests.cases` and backend tests. | Tests 2026-09 AUD-0008 `partial` `stale` |
| `contract.mon_source_parity` | MON reader/schema in `crates/moth-mon` versus compiler tokenizer and parser source construction; shared lexical policy in `crates/moth-lexical` | Correctness 2026-10 AUD-0009 `partial`; Correctness 2026-10 AUD-0013 `partial` |
| `tests.mon_syntax_parity` | `src/compiler_tests/mon_syntax_parity/**`, `crates/moth-mon/src/tests/**` and `xtask/src/first_party_deps/tests.rs` | Tests 2026-10 AUD-0010 `partial` |
| `crates.moth_mon.reader` | `crates/moth-mon/src/reader.rs` trivia walking, payload lookahead and consumed-end state | Redundancy 2026-10 AUD-0011 `partial` |
| `frontend.optional_defaults` | Optional parameters and fields with a present default; this audit inspected only the reproduced explicit-value override behavior | Correctness 2026-10 AUD-0012 `partial` |
| `crates.moth_mon.schema` | Field validation and preparation in `crates/moth-mon/src/schema.rs`, including the `prepare_fields` to `prepare_field` handoff | Redundancy 2026-10 AUD-0015 `partial` |
| `docs.empty_container_contract` | Accepted empty-map and empty-collection semantics across public references, status and related examples | Documentation 2026-10 AUD-0016 `partial` |

## Never audited

Areas with no row above and no coverage of any kind. This list is deliberately coarse - it exists so the gap is visible, not to partition the codebase in advance. Take one, name the part you can actually cover, and add a row.

AUD-0004 measured 771 of 791 production `.rs` files as having no owner under the registry taxonomy that preceded this log. That figure is a historical ownership measurement, not a recount of the areas below under the current model.

- `src/compiler_frontend/ast/**` - AST semantics, constant folding, templates and TIR
- `src/compiler_frontend/headers/**`, `module_compilation/**`
- `src/compiler_frontend/compiler_messages/**` - diagnostic construction and rendering
- `src/backends/**` - JS and Wasm lowering, backend feature validation
- `src/build_system/**` outside `create_project_modules/`
- `src/projects/**`
- `docs/roadmap/**` outside `docs.audit_framework` - plans, the roadmap and benchmark records
