# October 2026 Summary

## End-to-end CLI / macOS Apple Silicon (6D851D)
Change since initial benchmark: +15ms avg; 0 faster, 17 slower; 39/40 cases; workload changed: 1 case (docs_check)
Timing schema: 2
Initial: all ~23ms, Core ~23ms, Docs ~201ms, Stress ~21ms, Module ~12ms, Borrow ~9ms
Latest: all ~38ms, Core ~39ms, Docs ~221ms, Stress ~39ms, Module ~18ms, Borrow ~17ms
Case spread latest: ~68ms

---------------------

# End-to-end CLI / macOS Apple Silicon (6D851D): October 1st - 11:59
Timing schema: 2
mixed: avg -12ms; 4 faster, 22 slower; 28/40 cases; workload changed: 12 cases (docs_check, code_highlighter_stress_check, module_graph_check, module_graph_build, import_fanout_check, import_fanout_build, external_js_imports_check, external_js_imports_build, module_root_stress_check, module_root_stress_build, import_external_churn_check, import_external_churn_build)
Avg: all ~23ms, Core ~23ms, Docs ~201ms, Stress ~21ms, Module ~12ms, Borrow ~9ms
Stage movement: check total -381ms, generated materialise -362ms, module semantics -293ms

# End-to-end CLI / macOS Apple Silicon (6D851D): October 5th - 19:56
Timing schema: 2
**+15ms avg**; 0 faster, 17 slower; 39/40 cases; workload changed: 1 case (docs_check)
Avg: all ~38ms, Core ~39ms, Docs ~221ms, Stress ~39ms, Module ~18ms, Borrow ~17ms
Stage movement: frontend +543ms, boundary compile +540ms, module semantics +540ms
