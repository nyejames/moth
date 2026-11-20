# AUD-0026: HTML runtime-glue assembly redundancy

- State: `complete`
- Kind: `Redundancy`
- Area: `projects.html_runtime_glue` - external-JS runtime-glue path, import-map and module assembly in `src/projects/html_project/external_js/runtime_glue/`
- Coverage: `complete`
- Reviewed: `2026-11`
- Baseline: `main` at `72330758a` (`2026-11-20`), clean checkout before audit. The Rust toolchain and `moth` executable are unavailable in this environment, so the focused `cargo test -p moth --lib runtime_glue -- --nocapture` command could not start (`cargo: command not found`). No pre-audit gate was claimed. The existing bug queue includes a separate unconfirmed Windows template-resource path concern.

## What was inspected

All six production files under `src/projects/html_project/external_js/runtime_glue/`: `mod.rs`, `exports.rs`, `source.rs`, `paths.rs`, `import_map.rs` and `runtime_modules.rs`. The review traced referenced external function IDs into registry validation, per-entry source and import-map creation, deterministic generated paths and build-wide runtime-module publication.

Relevant assertions were read in `src/projects/html_project/external_js/tests/runtime_glue_tests.rs`, including no-demand cases, metadata errors, generated wrapper evaluation, numeric adapter boundaries, path encoding, runtime-module emission and import-map deduplication. `runtime_emission_plan.rs` and its tests, `runtime_module_registry.rs`, the `js_path.rs` call, `html_project_builder.rs` output assembly and shared-path exception, and the `structural_url_renderer.rs` / `build_system/output/output_path.rs` path owners were traced as context. They were not audited as separate areas.

## Authorities read

`AGENTS.md`; `docs/roadmap/audit-guide.md`; `docs/roadmap/audit-kinds/{README, redundancy}.md`; `docs/roadmap/audits/README.md`; `docs/developer-docs/style-guide/{style-guide,testing,validation}.mtf`; `docs/docs/cheatsheet/moth-language-cheatsheet.mtf`; `docs/compiler-design-overview.md` (authority, invariants, per-function link facts and backend handoff); `docs/build-system-design.md` (authority, invariants, HTML project builder > External JavaScript, Resource linking and output placement); `docs/docs/packages/external-binding-contracts.mtf`; and `docs/docs/progress/packages-and-builders/@entry.moth`.

## Existing findings and active plans checked

`docs/roadmap/audit-log.md` has no prior row for this area. `docs/roadmap/open-audit-findings.md`, `docs/bug-reports.md` and `docs/roadmap/completed_audits_log.md` have no unresolved finding with this root owner. The resource-path hypothesis in the bug queue concerns `moth_template/input.rs`, not generated runtime glue. The namespaced-public-export-paths and Core/WAT binding plans were checked for planned changes to runtime demand. Their future work does not invalidate the existing per-entry/build-wide ownership split.

## Findings

None. The suspected duplication did not meet the checklist's equivalent-responsibility and simplification bar.

## Checked and clean

- **Selected demand stays entry-local.** `collect_referenced_exports` validates referenced IDs, applicable JS lowering, package provenance and a matching JS runtime asset before `generate_module_glue` emits a wrapper. Its no-demand return creates no glue or import map. The tests cover both absent and registered-but-unreferenced exports and inconsistent metadata.
- **Source generation has one wrapper owner.** `source.rs` shares numeric adapter selection and success-result conversion across infallible and fallible wrappers. Separate error-envelope and Never wrappers preserve distinct foreign contracts. The apparently identical output of the binary-float and identity adapters is intentional: float results are rounded/validated at the downstream HIR boundary, while the adapter classification still rejects unsupported multi-slot shapes.
- **Entry and build scans serve different unions.** `import_map.rs` reads the current entry's required bare specifiers, while `runtime_emission_plan.rs` deduplicates all selected entries' assets and module requirements. Routing both through one shared collection would either broaden an entry's import map or force build-level emission to repeat entry policy. The two collectors do not independently decide source dependencies.
- **Path owners have distinct inputs.** `paths.rs::relative_url_path` builds browser module import URLs from native emitted-file paths. `StructuralUrlRenderer::render_resource_origin` handles validated structural resources, case-folded output identities, context artefacts and site-origin policy. They share the output owner's URL-segment encoder, but extracting a common relative-path abstraction would need extra context and failure modes without removing an identical policy owner.
- **Deduplication is intentional at two levels.** `runtime_module_output_path` is reused for preflight reservation and final emission, so emission cannot invent another name after collision validation. Glue paths hash the final source and `html_project_builder.rs::is_shared_glue_output` permits only identical generated-JS bytes to share a path. Provider assets and ordinary resource outputs retain strict conflict handling.
- **Small local sorting remains local.** `source.rs` sorts import paths and aliases after collecting them, with explicit deduplication. A tree-map rewrite could remove some temporary key cloning but would not eliminate an ownership layer or meaningful policy, so it is not a formal finding.

## Limitations

- This run covered runtime-glue redundancy, not source-level JavaScript legality, generated-code correctness, the wider JS backend, general output-path security or the template input pipeline. No kind coverage is claimed for those surfaces.
- Rust tests, compiler execution and the documentation release-build gate were unavailable because `cargo`, `rustc` and `moth` were not installed. Existing Node-driven Rust test bodies were inspected but not executed. This constrains validation evidence, not the code-path coverage claim.
- No performance measurements were taken and no speed or memory improvement is claimed.
