# August 2026 Summary

## End-to-end CLI / macOS Apple Silicon (6D851D)
Change since initial benchmark: case set changed: no comparable unchanged workloads; 40 current, 29 previous; workload changed: 29 cases (root_single_file_check, speed_test_check, speed_test_build, docs_check, template_stress_check, code_highlighter_stress_check, type_stress_check, fold_stress_check, pattern_stress_check, collection_stress_check, environment_stress_check, one_module_kitchen_sink_check, deep_scope_churn_check, template_render_plan_churn_check, constant_dag_churn_check, expression_rpn_churn_check, generic_trait_churn_check, collection_map_borrow_churn_check, module_graph_check, module_graph_build, import_fanout_check, import_fanout_build, external_js_imports_check, external_js_imports_build, module_root_stress_check, module_root_stress_build, import_external_churn_check, import_external_churn_build, borrow_stress_check)
Timing schema: 2
Initial: all ~25ms, Core ~31ms, Docs ~287ms, Stress ~15ms, Module ~13ms, Borrow ~7ms
Latest: all ~54ms, Core ~25ms, Docs ~263ms, Stress ~69ms, Module ~11ms, Borrow ~7ms
Case spread latest: ~165ms

## Frontend phases / macOS Apple Silicon (6D851D)
Change since initial benchmark: baseline
Initial: all ~73ms, Core ~55ms, Docs ~1276ms, Stress ~37ms, Module ~32ms, Parallelism ~28ms, Borrow ~20ms
Latest: all ~73ms, Core ~55ms, Docs ~1276ms, Stress ~37ms, Module ~32ms, Parallelism ~28ms, Borrow ~20ms
Case spread latest: ~220ms
---------------------

# End-to-end CLI / macOS Apple Silicon (6D851D): August 4th - 07:32
**baseline**; 29 cases, avg ~16ms
Avg: all ~16ms, Core ~16ms, Docs ~187ms, Stress ~9ms, Module ~9ms, Borrow ~6ms

# Frontend phases / macOS Apple Silicon (6D851D): August 4th - 07:50
**baseline**; 31 cases, avg ~73ms
Avg: all ~73ms, Core ~55ms, Docs ~1276ms, Stress ~37ms, Module ~32ms, Parallelism ~28ms, Borrow ~20ms

# End-to-end CLI / macOS Apple Silicon (6D851D): August 22nd - 14:36
Timing schema: 2
**baseline**; 29 cases, avg ~25ms
Avg: all ~25ms, Core ~31ms, Docs ~287ms, Stress ~15ms, Module ~13ms, Borrow ~7ms

# End-to-end CLI / macOS Apple Silicon (6D851D): August 24th - 20:59
Timing schema: 2
case set changed: no comparable unchanged workloads; 40 current, 29 previous; workload changed: 29 cases (root_single_file_check, speed_test_check, speed_test_build, docs_check, template_stress_check, code_highlighter_stress_check, type_stress_check, fold_stress_check, pattern_stress_check, collection_stress_check, environment_stress_check, one_module_kitchen_sink_check, deep_scope_churn_check, template_render_plan_churn_check, constant_dag_churn_check, expression_rpn_churn_check, generic_trait_churn_check, collection_map_borrow_churn_check, module_graph_check, module_graph_build, import_fanout_check, import_fanout_build, external_js_imports_check, external_js_imports_build, module_root_stress_check, module_root_stress_build, import_external_churn_check, import_external_churn_build, borrow_stress_check)
Avg: all ~54ms, Core ~25ms, Docs ~263ms, Stress ~69ms, Module ~11ms, Borrow ~7ms
