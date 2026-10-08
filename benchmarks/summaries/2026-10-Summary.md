# October 2026 Summary

## Frontend phases / macOS Apple Silicon (6D851D)
Change since initial benchmark: no measurable change: avg -2ms; 42/42 cases
Timing schema: 2
Initial: all ~97ms, Core ~27ms, Docs ~1372ms, Stress ~96ms, Module ~29ms, Parallelism ~19ms, Borrow ~28ms
Latest: all ~95ms, Core ~26ms, Docs ~1320ms, Stress ~95ms, Module ~29ms, Parallelism ~19ms, Borrow ~27ms
Case spread latest: ~243ms

## End-to-end CLI / macOS Apple Silicon (6D851D)
Change since initial benchmark: mixed: avg +1ms; 3 faster, 2 slower; 38/40 cases; workload changed: 2 cases (docs_check, code_highlighter_stress_check)
Timing schema: 2
Initial: all ~23ms, Core ~23ms, Docs ~201ms, Stress ~21ms, Module ~12ms, Borrow ~9ms
Latest: all ~24ms, Core ~20ms, Docs ~212ms, Stress ~23ms, Module ~12ms, Borrow ~10ms
Case spread latest: ~46ms
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

# End-to-end CLI / macOS Apple Silicon (6D851D): October 6th - 19:42
Timing schema: 2
**-12ms avg**; 17 faster, 0 slower; 39/40 cases; workload changed: 1 case (docs_check)
Avg: all ~26ms, Core ~23ms, Docs ~210ms, Stress ~25ms, Module ~12ms, Borrow ~10ms
Stage movement: frontend -420ms, boundary compile -418ms, module semantics -417ms

# Frontend phases / macOS Apple Silicon (6D851D): October 8th - 13:43
Timing schema: 2
mixed: avg -76ms; 16 faster, 6 slower; 25/42 cases; workload changed: 17 cases (docs_frontend, code_highlighter_stress_frontend, module_graph_frontend, import_fanout_frontend, module_root_stress_frontend, external_js_imports_frontend, import_external_churn_frontend, module_root_role_mix_frontend, tiny_one_file_frontend, tiny_two_files_frontend, tiny_seven_files_frontend, tiny_eight_files_frontend, many_tiny_files_frontend, many_medium_files_frontend, many_markdown_assets_frontend, many_modules_one_file_each_frontend, few_modules_many_files_each_frontend)
Avg: all ~97ms, Core ~27ms, Docs ~1372ms, Stress ~96ms, Module ~29ms, Parallelism ~19ms, Borrow ~28ms
Stage movement: generated materialise -1989ms, single-file frontend -1681ms, frontend -1680ms

# End-to-end CLI / macOS Apple Silicon (6D851D): October 8th - 13:46
Timing schema: 2
**-1ms avg**; 4 faster, 0 slower; 38/40 cases; workload changed: 2 cases (docs_check, code_highlighter_stress_check)
Avg: all ~25ms, Core ~20ms, Docs ~220ms, Stress ~23ms, Module ~12ms, Borrow ~10ms
Stage movement: module semantics -66ms, boundary compile -63ms, frontend -58ms

# Frontend phases / macOS Apple Silicon (6D851D): October 8th - 13:47
Timing schema: 2
no measurable change: avg -2ms; 42/42 cases
Avg: all ~95ms, Core ~28ms, Docs ~1350ms, Stress ~94ms, Module ~29ms, Parallelism ~18ms, Borrow ~27ms
Stage movement: frontend -67ms, boundary compile -60ms, module semantics -58ms

# End-to-end CLI / macOS Apple Silicon (6D851D): October 8th - 13:48
Timing schema: 2
**0ms avg**; 1 faster, 0 slower; 40/40 cases
Avg: all ~25ms, Core ~21ms, Docs ~217ms, Stress ~24ms, Module ~12ms, Borrow ~10ms
Stage movement: single-file frontend +6ms, check total +4ms, boundary inventory -3ms

# Frontend phases / macOS Apple Silicon (6D851D): October 8th - 13:49
Timing schema: 2
**-2ms avg**; 2 faster, 0 slower; 42/42 cases
Avg: all ~93ms, Core ~26ms, Docs ~1298ms, Stress ~93ms, Module ~29ms, Parallelism ~18ms, Borrow ~27ms
Stage movement: frontend -92ms, boundary compile -79ms, module semantics -75ms

# End-to-end CLI / macOS Apple Silicon (6D851D): October 8th - 13:51
Timing schema: 2
no measurable change: avg 0ms; 40/40 cases
Avg: all ~24ms, Core ~20ms, Docs ~212ms, Stress ~23ms, Module ~12ms, Borrow ~10ms
Stage movement: check total +1ms, module semantics +1ms, frontend +1ms

# Frontend phases / macOS Apple Silicon (6D851D): October 8th - 13:50
Timing schema: 2
no measurable change: avg 0ms; 42/42 cases
Avg: all ~93ms, Core ~26ms, Docs ~1315ms, Stress ~92ms, Module ~29ms, Parallelism ~18ms, Borrow ~27ms
Stage movement: single-file frontend -17ms, boundary inventory +16ms, directory compile +14ms

# Frontend phases / macOS Apple Silicon (6D851D): October 8th - 13:52
Timing schema: 2
**+2ms avg**; 0 faster, 1 slower; 42/42 cases
Avg: all ~95ms, Core ~26ms, Docs ~1320ms, Stress ~95ms, Module ~29ms, Parallelism ~19ms, Borrow ~27ms
Stage movement: frontend +80ms, boundary compile +73ms, module semantics +71ms
