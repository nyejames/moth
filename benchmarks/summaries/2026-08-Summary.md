# August 2026 Summary

## Frontend phases / macOS Apple Silicon (6D851D)
Change since initial benchmark: no measurable change: avg 0ms; 30/30 cases
Initial: all ~65ms, core ~49ms, docs ~1030ms, stress ~38ms, module ~31ms, borrow ~17ms, parallelism ~23ms
Latest: all ~65ms, core ~50ms, docs ~1034ms, stress ~38ms, module ~31ms, borrow ~17ms, parallelism ~23ms
Case spread latest: ~181ms

## End-to-end CLI / macOS Apple Silicon (6D851D)
Change since initial benchmark: no measurable change: avg 0ms; 28/28 cases
Initial: all ~16ms, core ~16ms, docs ~170ms, stress ~10ms, module ~9ms, borrow ~5ms
Latest: all ~16ms, core ~16ms, docs ~171ms, stress ~9ms, module ~9ms, borrow ~5ms
Case spread latest: ~30ms
---------------------

# End-to-end CLI / macOS Apple Silicon (6D851D): August 2nd - 03:44
**baseline**; 28 cases, avg ~16ms
Avg: all ~16ms, core ~16ms, docs ~170ms, stress ~10ms, module ~9ms, borrow ~5ms

# Frontend phases / macOS Apple Silicon (6D851D): August 2nd - 03:45
**baseline**; 30 cases, avg ~65ms
Avg: all ~65ms, core ~49ms, docs ~1030ms, stress ~38ms, module ~31ms, borrow ~17ms, parallelism ~23ms

# End-to-end CLI / macOS Apple Silicon (6D851D): August 2nd - 05:31
**+1ms avg**; 0 faster, 1 slower; 28/28 cases
Avg: all ~17ms, core ~17ms, docs ~174ms, stress ~11ms, module ~10ms, borrow ~6ms
Stage movement: check total +18ms, check frontend +16ms, frontend module +16ms

# Frontend phases / macOS Apple Silicon (6D851D): August 2nd - 05:32
**+10ms avg**; 0 faster, 5 slower; 30/30 cases
Avg: all ~75ms, core ~60ms, docs ~1250ms, stress ~42ms, module ~33ms, borrow ~19ms, parallelism ~26ms
Stage movement: stage0 dir +245ms, frontend module +235ms, module compile +202ms

# End-to-end CLI / macOS Apple Silicon (6D851D): August 2nd - 07:19
no measurable change: avg 0ms; 28/28 cases
Avg: all ~16ms, core ~16ms, docs ~171ms, stress ~9ms, module ~9ms, borrow ~5ms
Stage movement: check total -3ms, check frontend -2ms, frontend module -1ms

# Frontend phases / macOS Apple Silicon (6D851D): August 2nd - 06:45
**-10ms avg**; 4 faster, 0 slower; 30/30 cases
Avg: all ~65ms, core ~50ms, docs ~1032ms, stress ~38ms, module ~31ms, borrow ~17ms, parallelism ~23ms
Stage movement: stage0 dir -242ms, frontend module -231ms, module compile -200ms

# Frontend phases / macOS Apple Silicon (6D851D): August 2nd - 07:20
no measurable change: avg 0ms; 30/30 cases
Avg: all ~65ms, core ~50ms, docs ~1034ms, stress ~38ms, module ~31ms, borrow ~17ms, parallelism ~23ms
Stage movement: bootstrap -2ms, bootstrap.frontend_surface -1ms
