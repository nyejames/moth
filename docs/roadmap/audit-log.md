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
| `build.stage0.discovery` | Stage 0 source discovery and inventory | Leaf | `src/build_system/create_project_modules/{source_tree_index,source_discovery,source_package_discovery,project_roots,module_inventory,project_structure_diagnostics,source_discovery_error}.rs` | `docs/build-system-design.md` (`Source indexing and source sets`, `Prepared-source orchestration`), `build.stage0.graph`, `build.stage0.scheduling` | `Performance: build.stage0.preparation, plus the single-file synthetic path in compilation.rs, which owns divergent scheduling for the same work` |
| `build.stage0.preparation` | Stage 0 source preparation and handoff | Leaf | `src/build_system/create_project_modules/{module_preparation,source_preparation,source_loading,prepared_source,prepared_module}.rs` | `docs/compiler-design-overview.md` (`Compiler input and result boundary`), `src/compiler_frontend/headers/parse_file_headers.rs`, `src/compiler_frontend/symbols/string_interning.rs` | `Performance: build.stage0.discovery; string-table fork and merge cost is owned here, not by string_interning.rs` |
| `build.stage0.graph` | Stage 0 module identity, namespace and graph | Leaf | `src/build_system/create_project_modules/{module_identity,module_namespace,project_module_graph}.rs` | `docs/build-system-design.md` (`Project and package topology`), `build.stage0.discovery` | — |
| `build.stage0.scheduling` | Stage 0 wave scheduling and publication | Leaf | `src/build_system/create_project_modules/{mod,compilation,compiled_boundary,module_artifact_store,generated_store}.rs` | `docs/build-system-design.md` (`Deterministic scheduling and graph outcomes`), `src/compiler_frontend/module_compilation/**` | `Performance: both single-file and directory flows live in compilation.rs and must be measured separately` |
| `build.stage0` | Stage 0 end to end | Composite | `build.stage0.discovery`, `build.stage0.preparation`, `build.stage0.graph`, `build.stage0.scheduling` | `docs/build-system-design.md` opening authority text and `Architectural invariants` | Use for end-to-end Stage 0 cost or cross-leaf duplication. A finding owned by one leaf is recorded against that leaf, not here. |

## Audit freshness

Skip `-` cells during automatic selection.

| Scope ID | Style | Comments | Correctness | Diagnostics | Tests | Redundancy | Performance | Documentation |
|---|---|---|---|---|---|---|---|---|
| `tests.harness` | `N` | `N` | `N` | `N` | `N` | `N` | `N` | `-` |
| `tests.support` | `N` | `N` | `-` | `-` | `N` | `P 2026-08 AUD-0001` | `-` | `-` |
| `tests.cases` | `-` | `-` | `-` | `-` | `N` | `N` | `-` | `-` |
| `build.stage0.discovery` | `N` | `N` | `N` | `N` | `N` | `N` | `P 2026-08 AUD-0002` | `-` |
| `build.stage0.preparation` | `N` | `N` | `N` | `N` | `N` | `N` | `P 2026-08 AUD-0002` | `-` |
| `build.stage0.graph` | `N` | `N` | `N` | `N` | `N` | `N` | `N` | `-` |
| `build.stage0.scheduling` | `N` | `N` | `N` | `N` | `N` | `N` | `N` | `-` |
| `build.stage0` | `-` | `-` | `N` | `N` | `-` | `N` | `N` | `-` |
