# September 2026 Summary

## Diagnostic data layout / macOS Apple Silicon (6D851D)
Change since initial benchmark: baseline
Timing schema: 2
Initial: all ~33ms, Data layout ~33ms
Latest: all ~33ms, Data layout ~33ms
Case spread latest: ~21ms

## End-to-end CLI / macOS Apple Silicon (6D851D)
Change since initial benchmark: -29ms avg; 17 faster, 0 slower; 28/40 cases; workload changed: 12 cases (speed_test_check, speed_test_build, docs_check, type_stress_check, fold_stress_check, collection_stress_check, one_module_kitchen_sink_check, expression_rpn_churn_check, collection_map_borrow_churn_check, import_external_churn_check, import_external_churn_build, borrow_stress_check)
Timing schema: 2
Initial: all ~56ms, Core ~25ms, Docs ~280ms, Stress ~70ms, Module ~11ms, Borrow ~7ms
Latest: all ~32ms, Core ~16ms, Docs ~208ms, Stress ~37ms, Module ~9ms, Borrow ~5ms
Case spread latest: ~88ms
## Frontend phases / macOS Apple Silicon (6D851D)
Change since initial benchmark: baseline
Timing schema: 2
Initial: all ~155ms, Core ~34ms, Docs ~1822ms, Stress ~176ms, Module ~31ms, Parallelism ~24ms, Borrow ~17ms
Latest: all ~155ms, Core ~34ms, Docs ~1822ms, Stress ~176ms, Module ~31ms, Parallelism ~24ms, Borrow ~17ms
Case spread latest: ~450ms
---------------------

# End-to-end CLI / macOS Apple Silicon (6D851D): September 2nd - 12:02
Timing schema: 2
no measurable change: avg +1ms; 29/40 cases; workload changed: 11 cases (docs_check, module_graph_check, module_graph_build, import_fanout_check, import_fanout_build, external_js_imports_check, external_js_imports_build, module_root_stress_check, module_root_stress_build, import_external_churn_check, import_external_churn_build)
Avg: all ~56ms, Core ~25ms, Docs ~280ms, Stress ~70ms, Module ~11ms, Borrow ~7ms
Stage movement: generated AST +59ms, generated materialise +53ms, module semantics +49ms

# End-to-end CLI / macOS Apple Silicon (6D851D): September 4th - 09:40
Timing schema: 2
**+1ms avg**; 0 faster, 2 slower; 28/40 cases; workload changed: 12 cases (speed_test_check, speed_test_build, docs_check, type_stress_check, fold_stress_check, collection_stress_check, one_module_kitchen_sink_check, expression_rpn_churn_check, collection_map_borrow_churn_check, import_external_churn_check, import_external_churn_build, borrow_stress_check)
Avg: all ~57ms, Core ~24ms, Docs ~302ms, Stress ~71ms, Module ~11ms, Borrow ~8ms
Stage movement: check total +23ms, frontend +11ms, boundary compile +8ms

# Frontend phases / macOS Apple Silicon (6D851D): September 4th - 12:58
Timing schema: 2
case set changed: no comparable unchanged workloads; 42 current, 31 previous; workload changed: 31 cases (type_stress_frontend, docs_frontend, template_stress_frontend, code_highlighter_stress_frontend, fold_stress_frontend, pattern_stress_frontend, collection_stress_frontend, environment_stress_frontend, one_module_kitchen_sink_frontend, deep_scope_churn_frontend, template_render_plan_churn_frontend, constant_dag_churn_frontend, expression_rpn_churn_frontend, generic_trait_churn_frontend, collection_map_borrow_churn_frontend, module_graph_frontend, import_fanout_frontend, module_root_stress_frontend, external_js_imports_frontend, import_external_churn_frontend, module_root_role_mix_frontend, tiny_one_file_frontend, tiny_two_files_frontend, tiny_seven_files_frontend, tiny_eight_files_frontend, many_tiny_files_frontend, many_medium_files_frontend, many_markdown_assets_frontend, many_modules_one_file_each_frontend, few_modules_many_files_each_frontend, borrow_stress_frontend)
Avg: all ~155ms, Core ~34ms, Docs ~1822ms, Stress ~176ms, Module ~31ms, Parallelism ~24ms, Borrow ~17ms

# End-to-end CLI / macOS Apple Silicon (6D851D): September 4th - 12:59
Timing schema: 2
**-25ms avg**; 31 faster, 0 slower; 40/40 cases
Avg: all ~32ms, Core ~16ms, Docs ~208ms, Stress ~37ms, Module ~9ms, Borrow ~5ms
Stage movement: check total -953ms, frontend -708ms, boundary compile -679ms

# Diagnostic data layout / macOS Apple Silicon (6D851D): September 4th - 12:59
Timing schema: 2
**baseline**; 2 cases, avg ~33ms
Avg: all ~33ms, Data layout ~33ms
