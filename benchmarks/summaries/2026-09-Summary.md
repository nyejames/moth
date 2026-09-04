# September 2026 Summary

## End-to-end CLI / macOS Apple Silicon (6D851D)
Change since initial benchmark: +1ms avg; 0 faster, 2 slower; 28/40 cases; workload changed: 12 cases (speed_test_check, speed_test_build, docs_check, type_stress_check, fold_stress_check, collection_stress_check, one_module_kitchen_sink_check, expression_rpn_churn_check, collection_map_borrow_churn_check, import_external_churn_check, import_external_churn_build, borrow_stress_check)
Timing schema: 2
Initial: all ~56ms, Core ~25ms, Docs ~280ms, Stress ~70ms, Module ~11ms, Borrow ~7ms
Latest: all ~57ms, Core ~24ms, Docs ~302ms, Stress ~71ms, Module ~11ms, Borrow ~8ms
Case spread latest: ~172ms

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
