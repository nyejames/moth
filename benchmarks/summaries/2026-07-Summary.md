# July 2026 Summary

## Frontend phases / macOS Apple Silicon (6D851D)
Change since initial benchmark: baseline
Initial: all ~47ms, core ~53ms, docs ~494ms, stress ~40ms, module ~30ms, borrow ~23ms, parallelism ~21ms
Latest: all ~47ms, core ~53ms, docs ~494ms, stress ~40ms, module ~30ms, borrow ~23ms, parallelism ~21ms
Case spread latest: ~84ms

## End-to-end CLI / macOS Apple Silicon (6D851D)
Change since initial benchmark: baseline
Initial: all ~18ms, core ~17ms, docs ~191ms, stress ~11ms, module ~10ms, borrow ~7ms
Latest: all ~18ms, core ~17ms, docs ~191ms, stress ~11ms, module ~10ms, borrow ~7ms
Case spread latest: ~34ms
---------------------

# End-to-end CLI / macOS Apple Silicon (6D851D): July 8th - 09:55
case set changed: avg 0ms on 26/28 shared cases; 2 slower, 1 faster
Avg: all ~21ms, core ~5ms, docs ~228ms, stress ~16ms, module ~13ms, borrow ~10ms

# End-to-end CLI / macOS Apple Silicon (6D851D): July 19th - 05:59
**-6ms avg**; 22 faster, 0 slower; 28/28 cases
Avg: all ~15ms, core ~4ms, docs ~196ms, stress ~8ms, module ~9ms, borrow ~7ms
Stage movement: reachable discovery +587ms, import resolve +412ms, ast -364ms

# End-to-end CLI / macOS Apple Silicon (6D851D): July 19th - 06:00
no measurable change: avg 0ms; 28/28 cases
Avg: all ~15ms, core ~5ms, docs ~201ms, stress ~8ms, module ~8ms, borrow ~6ms
Stage movement: import resolve -16ms, reachable discovery -13ms, frontend module -9ms

# End-to-end CLI / macOS Apple Silicon (6D851D): July 26th - 22:39
**baseline**; 28 cases, avg ~18ms
Avg: all ~18ms, core ~17ms, docs ~191ms, stress ~11ms, module ~10ms, borrow ~7ms

# Frontend phases / macOS Apple Silicon (6D851D): July 26th - 22:39
**baseline**; 30 cases, avg ~47ms
Avg: all ~47ms, core ~53ms, docs ~494ms, stress ~40ms, module ~30ms, borrow ~23ms, parallelism ~21ms

