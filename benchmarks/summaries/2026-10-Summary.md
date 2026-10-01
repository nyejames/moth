# October 2026 Summary

## End-to-end CLI / macOS Apple Silicon (6D851D)
Change since initial benchmark: baseline
Timing schema: 2
Initial: all ~23ms, Core ~23ms, Docs ~201ms, Stress ~21ms, Module ~12ms, Borrow ~9ms
Latest: all ~23ms, Core ~23ms, Docs ~201ms, Stress ~21ms, Module ~12ms, Borrow ~9ms
Case spread latest: ~39ms

---------------------

# End-to-end CLI / macOS Apple Silicon (6D851D): October 1st - 11:59
Timing schema: 2
mixed: avg -12ms; 4 faster, 22 slower; 28/40 cases; workload changed: 12 cases (docs_check, code_highlighter_stress_check, module_graph_check, module_graph_build, import_fanout_check, import_fanout_build, external_js_imports_check, external_js_imports_build, module_root_stress_check, module_root_stress_build, import_external_churn_check, import_external_churn_build)
Avg: all ~23ms, Core ~23ms, Docs ~201ms, Stress ~21ms, Module ~12ms, Borrow ~9ms
Stage movement: check total -381ms, generated materialise -362ms, module semantics -293ms
