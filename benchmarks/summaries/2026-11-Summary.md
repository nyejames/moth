# November 2026 Summary

## End-to-end CLI / macOS Apple Silicon (6D851D)
Change since initial benchmark: +2ms avg; 0 faster, 4 slower; 49/74 cases; workload changed: 25 cases (root_single_file_check, speed_test_check, speed_test_build, docs_check, template_stress_check, code_highlighter_stress_check, constant_chain_32_check, constant_chain_128_check, constant_chain_512_check, nominal_scaling_40_check, nominal_scaling_80_check, nominal_scaling_160_check, nominal_scaling_320_check, one_module_kitchen_sink_check, template_render_plan_churn_check, module_graph_check, module_graph_build, import_fanout_check, import_fanout_build, external_js_imports_check, external_js_imports_build, module_root_stress_check, module_root_stress_build, import_external_churn_check, import_external_churn_build)
Timing schema: 2
Initial: all ~21ms, Core ~14ms, Docs ~163ms, Stress ~22ms, Module ~11ms, Borrow ~8ms
Latest: all ~23ms, Core ~16ms, Docs ~168ms, Stress ~23ms, Module ~11ms, Borrow ~8ms
Case spread latest: ~36ms

---------------------

# End-to-end CLI / macOS Apple Silicon (6D851D): November 2nd - 14:12
Timing schema: 2
**0ms avg**; 1 faster, 0 slower; 49/74 cases; workload changed: 25 cases (speed_test_check, speed_test_build, docs_check, template_stress_check, code_highlighter_stress_check, type_stress_check, pattern_stress_check, collection_stress_check, environment_stress_check, generic_scaling_20_check, generic_scaling_40_check, generic_scaling_80_check, generic_scaling_160_check, one_module_kitchen_sink_check, deep_scope_churn_check, template_render_plan_churn_check, collection_map_borrow_churn_check, module_graph_check, module_graph_build, import_external_churn_check, import_external_churn_build, borrow_stress_check, numeric_mutable_cycles_32_check, numeric_mutable_cycles_128_check, numeric_mutable_cycles_512_check)
Avg: all ~21ms, Core ~14ms, Docs ~163ms, Stress ~22ms, Module ~11ms, Borrow ~8ms
Stage movement: bootstrap +13ms, module semantics -10ms, boundary compile -10ms

# End-to-end CLI / macOS Apple Silicon (6D851D): November 5th - 13:55
Timing schema: 2
**+2ms avg**; 0 faster, 8 slower; 63/74 cases; workload changed: 11 cases (docs_check, module_graph_check, module_graph_build, import_fanout_check, import_fanout_build, external_js_imports_check, external_js_imports_build, module_root_stress_check, module_root_stress_build, import_external_churn_check, import_external_churn_build)
Avg: all ~24ms, Core ~15ms, Docs ~177ms, Stress ~24ms, Module ~12ms, Borrow ~9ms
Stage movement: check total +110ms, boundary compile +108ms, module semantics +106ms

# End-to-end CLI / macOS Apple Silicon (6D851D): November 10th - 12:34
Timing schema: 2
**-3ms avg**; 8 faster, 0 slower; 57/74 cases; workload changed: 17 cases (speed_test_check, speed_test_build, docs_check, template_stress_check, code_highlighter_stress_check, one_module_kitchen_sink_check, template_render_plan_churn_check, module_graph_check, module_graph_build, import_fanout_check, import_fanout_build, external_js_imports_check, external_js_imports_build, module_root_stress_check, module_root_stress_build, import_external_churn_check, import_external_churn_build)
Avg: all ~21ms, Core ~14ms, Docs ~154ms, Stress ~22ms, Module ~10ms, Borrow ~8ms
Stage movement: check total -110ms, frontend -105ms, single-file frontend -105ms

# End-to-end CLI / macOS Apple Silicon (6D851D): November 11th - 19:30
Timing schema: 2
**-1ms avg**; 1 faster, 0 slower; 73/74 cases; workload changed: 1 case (docs_check)
Avg: all ~22ms, Core ~14ms, Docs ~155ms, Stress ~22ms, Module ~11ms, Borrow ~8ms
Stage movement: check total -44ms, frontend -39ms, single-file frontend -37ms

# End-to-end CLI / macOS Apple Silicon (6D851D): November 13th - 09:32
Timing schema: 2
no measurable change: avg +1ms; 73/74 cases; workload changed: 1 case (docs_check)
Avg: all ~23ms, Core ~16ms, Docs ~168ms, Stress ~23ms, Module ~11ms, Borrow ~8ms
Stage movement: check total +36ms, single-file frontend +36ms, frontend +35ms
