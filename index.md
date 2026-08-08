# Moth codebase index for quick navigation

Flow: [projects](src/projects/) → [build_system](src/build_system/) → [compiler_frontend](src/compiler_frontend/) → tokenizer → headers → module_dependencies → ast → hir → borrow_checker → backends/projects.

## Root

- [moth CLI entry](src/main.rs)
- [crate module surface](src/lib.rs)
- [timing facade](src/timing.rs) and [enabled runtime](src/timing/enabled/runtime.rs): compile-erasing timing/counter entry points, immutable process configuration, command/raw session channels and inactive fast-path policy.
- [Moth source packages](packages/): compiler-shipped source-backed packages.
    - [@html Builder package](packages/html/@mod.moth): HTML helper templates (`canvas`, `p`, `h1`-`h6`, `div`, `table`, etc.) and the `Canvas`/`get_canvas` wrapper. Internal helpers live in [packages/html/private_helpers.moth](packages/html/private_helpers.moth).
    - [Core binding packages](src/builder_surface/core_packages/): compiler-owned operations and prelude visibility policy.
- [validate/bench/docs workflow](justfile)
- [contributor workflow and validation commands](CONTRIBUTING.md)

## Project/build shell

- [user commands and project modes](src/projects/): kw — cli, check, build, dev, new, routing.
    - [cli.rs](src/projects/cli.rs): dispatches commands.
    - [check.rs](src/projects/check.rs): frontend-only diagnostics.
    - [routing.rs](src/projects/routing.rs): path/origin rules.
    - [settings.rs](src/projects/settings.rs): Config/defaults/known paths.
    - [repl.rs](src/projects/repl.rs): template-focused REPL placeholder.
    - [dev_server](src/projects/dev_server/): HTTP/SSE/watch rebuild loop. kw: serve, hot reload.
    - [html_project](src/projects/html_project/): HTML builder and HTML-Wasm integration. kw: shell, assets, wasm.
- [builder boundary above frontend](src/build_system/): kw — config, modules, artifacts, cleanup.
    - [build.rs](src/build_system/build.rs): build_project, BuildResult, `Module` container with explicit executable, link-fact and compiler-metadata lanes, success-only `ProjectCompilation` over retained graph boundaries, compiler/backend orchestration.
    - [project_config.rs](src/build_system/project_config.rs) + [project_config/](src/build_system/project_config/): config.moth parse/validate through frontend+AST.
    - [path_validation.rs](src/build_system/path_validation.rs): project path policy checks.
    - [utils.rs](src/build_system/utils.rs): shared builder helpers.
    - [create_project_modules](src/build_system/create_project_modules/): Stage 0 module/source discovery.
        - [frontend_orchestration.rs](src/build_system/create_project_modules/frontend_orchestration.rs): per-module frontend pipeline.
        - [module_inventory.rs](src/build_system/create_project_modules/module_inventory.rs), [source_discovery.rs](src/build_system/create_project_modules/source_discovery.rs), [source_scanning.rs](src/build_system/create_project_modules/source_scanning.rs), [prepared_source.rs](src/build_system/create_project_modules/prepared_source.rs), [prepared_source_store.rs](src/build_system/create_project_modules/prepared_source_store.rs), [prepared_module.rs](src/build_system/create_project_modules/prepared_module.rs): reachable module inventory, single-file traversal and lexical/import extraction, state-safe retained source and module-syntax inputs, and one project-boundary prepare-once store indexed by `SourceId`.
        - [source_package_discovery.rs](src/build_system/create_project_modules/source_package_discovery.rs), [module_artifact_store.rs](src/build_system/create_project_modules/module_artifact_store.rs), [compiled_boundary.rs](src/build_system/create_project_modules/compiled_boundary.rs): source-package registration, one independent boundary index per selected package, dense module artefact store with per-module outcome slots, and retained project/source-package graph boundaries for the frontend outcome.
        - [source_tree_index.rs](src/build_system/create_project_modules/source_tree_index.rs), [module_identity.rs](src/build_system/create_project_modules/module_identity.rs), [module_namespace.rs](src/build_system/create_project_modules/module_namespace.rs), [project_module_graph.rs](src/build_system/create_project_modules/project_module_graph.rs): one canonical source traversal, a central dense `SourceId`/`SourceRecord` inventory with closed semantic/provider classification and indexed directory-provider lookup, boundary-aware namespace resolution, project module graph with dependency edges by local IDs, deterministic module assignment, structural ancestry, scoped support visibility and construction adjacency frozen into deterministic compile waves.
        - [source_loading.rs](src/build_system/create_project_modules/source_loading.rs), [compilation.rs](src/build_system/create_project_modules/compilation.rs): source load and current per-entry compilation.
        - [project_structure_diagnostics.rs](src/build_system/create_project_modules/project_structure_diagnostics.rs): structured layout, root and name-conflict diagnostics; project and package collision discovery lives in `source_tree_index.rs`.
        - [project_roots.rs](src/build_system/create_project_modules/project_roots.rs): project/entry roots.
        - [source_discovery_error.rs](src/build_system/create_project_modules/source_discovery_error.rs): diagnostic boundary.
    - [output](src/build_system/output/): validated output plans, portable path policy, prepared artifact writing, write orchestration, and stale manifest cleanup.
- [builder surface](src/builder_surface/): core packages, external import providers and package metadata.
    - [core_packages/](src/builder_surface/core_packages/): prelude, io, math, collections, text, random and time.
    - [external_import_providers/](src/builder_surface/external_import_providers/): provider registry and resolution table.

## Frontend stage map

- [frontend module map](src/compiler_frontend/mod.rs); [CompilerFrontend driver](src/compiler_frontend/pipeline.rs)
- [tokenizer](src/compiler_frontend/tokenizer/): lex source/templates into tokens. kw: TokenizeMode, SourceLocation.
- [headers](src/compiler_frontend/headers/): retained import/declaration shells, local ordering hints, interface binding, start-body split and ModuleSymbols. kw: facade, visibility.
- [declaration_syntax](src/compiler_frontend/declaration_syntax/): shared declaration/type shell parsers. kw: signatures, ParsedTypeRef.
- [module_dependencies.rs](src/compiler_frontend/module_dependencies.rs): topological header ordering. kw: dependency edges, cycles.
- [compiler_messages](src/compiler_frontend/compiler_messages/): CompilerDiagnostic/CompilerError/rendering plus the self-contained [ModuleDiagnostics](src/compiler_frontend/compiler_messages/module_diagnostics.rs) owner that separates diagnosed module failures from typed infrastructure errors. kw: diagnostic codes, labels, module outcomes.
- [symbols](src/compiler_frontend/symbols/): StringId, InternedPath, compiler symbols, naming policy.
- [paths](src/compiler_frontend/paths/): import/path normalization/format/resolution. kw: @imports, source roots.
- [source_packages](src/compiler_frontend/source_packages/): package-root registration and public import boundaries.
- [semantic_identity.rs](src/compiler_frontend/semantic_identity.rs): stable package, module, declaration, trait, evidence and callable origin identities plus `ExportBinding`.
- [canonical_type_identity.rs](src/compiler_frontend/canonical_type_identity.rs): owned cross-module identities and fallible `TypeId` projection for builtin, source nominal, binding-backed opaque, constructed and exported generic-parameter type shapes.
- [public_interface/](src/compiler_frontend/public_interface/): declaration-centric direct-interface model, export/type/receiver/trait/evidence projection and post-borrow local finalization. kw: `DirectExportSeed`, `CallableSeed`, `PublicInterfaceDraft`, `LocalPublicInterface`.
- [folded_value.rs](src/compiler_frontend/folded_value.rs): one owned backend-neutral folded-value vocabulary and converter shared by exported constants and retained defaults.
- [module_metadata.rs](src/compiler_frontend/module_metadata.rs): named HIR-lowering result, resolved documentation metadata, rendered-path handoff and non-HIR metadata validation.
- [external_packages](src/compiler_frontend/external_packages/): virtual package registry, external IDs. kw: @core, @web, opaque.
- [builtins](src/compiler_frontend/builtins/): compiler-owned types/ops/casts/runtime error metadata.
- [style_directives](src/compiler_frontend/style_directives/): frontend+builder template directive registry.
- [datatypes](src/compiler_frontend/datatypes/): DataType parse spelling + TypeEnvironment/TypeId semantic identity.
- [type_coercion](src/compiler_frontend/type_coercion/): compatibility/contextual/string coercion.
- [value_mode.rs](src/compiler_frontend/value_mode.rs): access modes (frontend root, shared by coercion and lowering).
- [traits](src/compiler_frontend/traits/): trait definitions, evidence, syntax helpers.
- [numeric_text](src/compiler_frontend/numeric_text/): numeric literal text parsing.
- [plain_markdown.rs](src/compiler_frontend/plain_markdown.rs): plain-markdown source handling outside template pipeline.
- [syntax_errors](src/compiler_frontend/syntax_errors/): shared syntax error construction.
- [utilities](src/compiler_frontend/utilities/): small frontend-local helpers.
- [keywords.rs](src/compiler_frontend/keywords.rs): reserved-word tables.
- [arena](src/compiler_frontend/arena/): AST/HIR allocation arenas and capacity budgeting.
- [instrumentation](src/compiler_frontend/instrumentation/): compile-time counters and frontend stats.
- [const_eval](src/compiler_frontend/ast/const_eval/): AST-stage const expression folding (RPN stack evaluator).

## AST stage

- [Stage 4 entry](src/compiler_frontend/ast/mod.rs): env build → emission → finalization.
- [module_ast orchestration](src/compiler_frontend/ast/module_ast/)
    - [environment](src/compiler_frontend/ast/module_ast/environment/): declarations, aliases, nominal types, signatures, constants.
        - [import projection](src/compiler_frontend/ast/module_ast/environment/builder/import_projection/): reachability-driven provider nominals, folded values, callables, and durable canonical interning.
    - [emission](src/compiler_frontend/ast/module_ast/emission/): function/start/body emission.
    - [finalization](src/compiler_frontend/ast/module_ast/finalization/): normalize constants/templates, const facts, type validation.
    - [scope_context](src/compiler_frontend/ast/module_ast/scope_context/): visibility/local declarations/diagnostic sinks.
- [type_resolution](src/compiler_frontend/ast/type_resolution/): parsed type syntax → TypeId.
    - [context.rs](src/compiler_frontend/ast/type_resolution/context.rs): state.
    - [resolve_type.rs](src/compiler_frontend/ast/type_resolution/resolve_type.rs): orchestration + diagnostic TypeId bridge.
    - [lookup.rs](src/compiler_frontend/ast/type_resolution/lookup.rs): names/namespaces/trait-name rejection.
    - [aliases.rs](src/compiler_frontend/ast/type_resolution/aliases.rs): alias re-resolution.
    - [collections.rs](src/compiler_frontend/ast/type_resolution/collections.rs): fixed capacity.
    - [maps.rs](src/compiler_frontend/ast/type_resolution/maps.rs): map key/nesting.
    - [generics.rs](src/compiler_frontend/ast/type_resolution/generics.rs): nominal instances.
    - [signatures.rs](src/compiler_frontend/ast/type_resolution/signatures.rs), [struct_fields.rs](src/compiler_frontend/ast/type_resolution/struct_fields.rs), [choice_variants.rs](src/compiler_frontend/ast/type_resolution/choice_variants.rs), [recursive_types.rs](src/compiler_frontend/ast/type_resolution/recursive_types.rs).
- [expressions](src/compiler_frontend/ast/expressions/): parsing/type checking/calls/constructors/mutation/options/namespaces.
- [field_access](src/compiler_frontend/ast/field_access/): fields, receiver calls, collection/map builtins.
- [statements](src/compiler_frontend/ast/statements/): bodies, declarations, returns, loops, matches, catch, value production.
- [templates](src/compiler_frontend/ast/templates/): template parse/compose/fold/format/render plans/slots/control flow/reactive metadata.
    - [template_head_parser](src/compiler_frontend/ast/templates/template_head_parser/): directives, subscriptions, suffix control flow.
    - [template_control_flow](src/compiler_frontend/ast/templates/template_control_flow/): const eval/folding/validation/remap.
    - [template_slots](src/compiler_frontend/ast/templates/template_slots/): slot schema, contributions, runtime plan construction.
    - [styles](src/compiler_frontend/ast/templates/styles/): directive-owned formatters (markdown, raw, whitespace).
    - [template_types.rs](src/compiler_frontend/ast/templates/template_types.rs), [template_folding.rs](src/compiler_frontend/ast/templates/template_folding.rs).
    - [template_render_units.rs](src/compiler_frontend/ast/templates/template_render_units.rs), [template_renderability.rs](src/compiler_frontend/ast/templates/template_renderability.rs).
    - [create_template_node.rs](src/compiler_frontend/ast/templates/create_template_node.rs), [top_level_templates.rs](src/compiler_frontend/ast/templates/top_level_templates.rs), [doc_fragments.rs](src/compiler_frontend/ast/templates/doc_fragments.rs), [error.rs](src/compiler_frontend/ast/templates/error.rs).
    - [runtime_handoff.rs](src/compiler_frontend/ast/templates/runtime_handoff.rs): neutral owned AST-to-HIR template handoff vocabulary; [reactive_template_metadata.rs](src/compiler_frontend/ast/templates/reactive_template_metadata.rs): exact-view and owned-handoff reactive metadata traversal.
    - [tir](src/compiler_frontend/ast/templates/tir/): Template IR — AST-local authoritative template representation. kw: TemplateIrStore.
        - [store.rs](src/compiler_frontend/ast/templates/tir/store.rs), [ids.rs](src/compiler_frontend/ast/templates/tir/ids.rs), [node.rs](src/compiler_frontend/ast/templates/tir/node.rs), [summary.rs](src/compiler_frontend/ast/templates/tir/summary.rs), [validation.rs](src/compiler_frontend/ast/templates/tir/validation.rs): central owned storage + shape metadata.
        - [builder.rs](src/compiler_frontend/ast/templates/tir/builder.rs), [parser_builder_state.rs](src/compiler_frontend/ast/templates/tir/parser_builder_state.rs): parser-facing direct TIR emission.
        - [refs.rs](src/compiler_frontend/ast/templates/tir/refs.rs), [view.rs](src/compiler_frontend/ast/templates/tir/view.rs): durable module-local references plus exact view identity, effective reads and structural transitions.
        - [expression_payload_walker.rs](src/compiler_frontend/ast/templates/tir/expression_payload_walker.rs): shared read-only expression-payload traversal.
        - [classification.rs](src/compiler_frontend/ast/templates/tir/classification.rs): exact-view TIR shape queries.
        - [preparation.rs](src/compiler_frontend/ast/templates/tir/preparation.rs): exact-view semantic preparation for foldable, runtime and helper values.
        - [fold.rs](src/compiler_frontend/ast/templates/tir/fold.rs), [formatter_view.rs](src/compiler_frontend/ast/templates/tir/formatter_view.rs), [render_unit.rs](src/compiler_frontend/ast/templates/tir/render_unit.rs): TIR-native fold, format and render-unit preparation.
        - [slot_plan.rs](src/compiler_frontend/ast/templates/tir/slot_plan.rs), [slot_composition/](src/compiler_frontend/ast/templates/tir/slot_composition/), [wrapper_sets.rs](src/compiler_frontend/ast/templates/tir/wrapper_sets.rs): slot routing and wrapper reuse.
        - [handoff_materialization.rs](src/compiler_frontend/ast/templates/tir/handoff_materialization.rs): owned runtime-template trees for HIR lowering.
- [generic_functions](src/compiler_frontend/ast/generic_functions/): generic templates, calls, inference, instances, diagnostics.
- [const_values](src/compiler_frontend/ast/const_values/): const fact resolver.
- [generic_bounds.rs](src/compiler_frontend/ast/generic_bounds.rs): static bound evidence checks.

## HIR + analysis

- [backend-facing semantic IR](src/compiler_frontend/hir/): kw — CFG, locals, TypeId, reachability.
    - [hir_builder](src/compiler_frontend/hir/hir_builder/), [hir_builder.rs](src/compiler_frontend/hir/hir_builder.rs): AST → HIR lowering state.
    - [hir_expression](src/compiler_frontend/hir/hir_expression/), [hir_statement](src/compiler_frontend/hir/hir_statement/): lowering implementation owners.
    - [validation](src/compiler_frontend/hir/validation/): executable-HIR internal invariant checks only; non-HIR module metadata is validated by [module_metadata.rs](src/compiler_frontend/module_metadata.rs).
    - [reachability.rs](src/compiler_frontend/hir/reachability.rs): function/block/external/map/runtime-cast feature facts.
    - [reactivity.rs](src/compiler_frontend/hir/reactivity.rs): HIR reactive metadata.
- [borrow_checker](src/compiler_frontend/analysis/borrow_checker/): HIR side-table access and advisory optional-transfer facts. kw — exclusivity, optional transfer, aliases.
    - [engine.rs](src/compiler_frontend/analysis/borrow_checker/engine.rs): fixed-point flow.
    - [transfer.rs](src/compiler_frontend/analysis/borrow_checker/transfer.rs), [transfer/](src/compiler_frontend/analysis/borrow_checker/transfer/): access policy.
    - [state.rs](src/compiler_frontend/analysis/borrow_checker/state.rs): lattice.
    - [diagnostics.rs](src/compiler_frontend/analysis/borrow_checker/diagnostics.rs).

## Backends

- [reachable unsupported-feature checks](src/backends/backend_feature_validation.rs)
- [external call/package backend support](src/backends/external_package_validation.rs)
- [shared backend error surface](src/backends/error_types.rs)
- [JS backend](src/backends/js/): HIR → JS. kw — readable JS, GC baseline, reachable emission.
    - [emitter.rs](src/backends/js/emitter.rs), [js_expr.rs](src/backends/js/js_expr.rs), [js_statement.rs](src/backends/js/js_statement.rs), [js_function.rs](src/backends/js/js_function.rs), [js_calls.rs](src/backends/js/js_calls.rs), [output.rs](src/backends/js/output.rs), [reachability.rs](src/backends/js/reachability.rs)
    - [runtime](src/backends/js/runtime/): helpers for strings/maps/casts.
- [Wasm backend](src/backends/wasm/): experimental core Wasm. kw — HIR→LIR, linear memory, emit.
    - [backend.rs](src/backends/wasm/backend.rs): Wasm backend driver and request handling.
    - [hir_to_lir](src/backends/wasm/hir_to_lir/): semantic lowering to Wasm LIR.
    - [lir](src/backends/wasm/lir/): Wasm-neutral low IR.
    - [emit](src/backends/wasm/emit/): binary emission/sections/validation.
    - [runtime](src/backends/wasm/runtime/): imports/memory/strings.
- [HTML-Wasm artifact plan](src/projects/html_project/wasm/): bootstrap/export roots.

## HTML project

- [BackendBuilder implementation](src/projects/html_project/html_project_builder.rs)
- [HTML document assembly](src/projects/html_project/output_plan.rs), [page_metadata.rs](src/projects/html_project/page_metadata.rs), [document_shell.rs](src/projects/html_project/document_shell.rs), [document_config.rs](src/projects/html_project/document_config.rs)
- [compile_input.rs](src/projects/html_project/compile_input.rs), [diagnostics.rs](src/projects/html_project/diagnostics.rs), [js_path.rs](src/projects/html_project/js_path.rs), [path_policy.rs](src/projects/html_project/path_policy.rs), [style_directives.rs](src/projects/html_project/style_directives.rs): build inputs/policy.
- [styles](src/projects/html_project/styles/): $html/$css/$escape_html/$code validation/rendering.
- [external_js](src/projects/html_project/external_js/): provider-backed JS imports, runtime modules/assets/glue.
- [binding_packages](src/projects/html_project/binding_packages/): builder-owned binding packages for HTML projects.
    - [@web/canvas binding package](src/projects/html_project/binding_packages/web/canvas/): built-in JS canvas asset (`canvas.js`) and `@web/canvas` registration. Used by the `@html` canvas helpers.
- [moth_template](src/projects/html_project/moth_template/): direct .mtf compile/extract support.
- [tracked_assets.rs](src/projects/html_project/tracked_assets.rs): copied assets.
- [new_html_project](src/projects/html_project/new_html_project/): scaffold command.

## Tests/tooling

- [integration test runner](src/compiler_tests/integration_test_runner/): manifest fixtures, expectations, execution, and assertion-family owners under [assertions](src/compiler_tests/integration_test_runner/assertions/).
- [integration fixtures](tests/cases/): expect.toml backend matrices.
- [subsystem unit tests](src/): `*/tests` and module tests throughout src/.
- [in-process compiler benchmark API](src/benchmarking/): for xtask/dev tooling.
- [benchmark/report/check/profile tooling](xtask/)
- [perf cases/data/summaries](benchmarks/): local-data ignored.

## Docs

- [docs entry point](docs/): comprehensive compiler and language documentation.
- [compiler design overview](docs/compiler-design-overview.md)
- [build system design overview](docs/build-system-design.md)
- [language semantics reference index](docs/src/docs/codebase/language/overview.mtf)
- [memory management design](docs/src/docs/codebase/memory-management/overview.mtf)
- [codebase style guide](docs/codebase-style-guide.md)
- [docs website source](docs/src/docs/); [generated output](docs/release/)
- [language support progress matrix](docs/src/docs/progress/@page.moth)
- [planned work and implementation plans](docs/roadmap/)
