# September 2026 Summary

## End-to-end CLI / macOS Apple Silicon (6D851D)
Change since initial benchmark: baseline
Timing schema: 2
Initial: all ~56ms, Core ~25ms, Docs ~280ms, Stress ~70ms, Module ~11ms, Borrow ~7ms
Latest: all ~56ms, Core ~25ms, Docs ~280ms, Stress ~70ms, Module ~11ms, Borrow ~7ms
Case spread latest: ~170ms

---------------------

# End-to-end CLI / macOS Apple Silicon (6D851D): September 2nd - 12:02
Timing schema: 2
no measurable change: avg +1ms; 29/40 cases; workload changed: 11 cases (docs_check, module_graph_check, module_graph_build, import_fanout_check, import_fanout_build, external_js_imports_check, external_js_imports_build, module_root_stress_check, module_root_stress_build, import_external_churn_check, import_external_churn_build)
Avg: all ~56ms, Core ~25ms, Docs ~280ms, Stress ~70ms, Module ~11ms, Borrow ~7ms
Stage movement: generated AST +59ms, generated materialise +53ms, module semantics +49ms
