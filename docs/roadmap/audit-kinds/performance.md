# Performance

Looking for: measured time, allocation or memory cost, attributed to an owner.

**Evidence bar:** a metric, a workload, a baseline and attribution. Code inspection alone produces a profiling candidate, not a finding.

Do not start by searching for `.clone()` and calling the result a performance audit.

## Define the question

One concrete question - "why does frontend time scale faster than source size?", not "is this slow?". Fix the command, profile, target, workload, cold or warm state, and the metric.

## Baseline

Record the exact non-recording command, run count, observed variance, relevant feature flags (`timers`, `detailed_timers`), input size and machine. Use medians. A single run is attribution evidence only when the effect is overwhelming, and still needs confirmation.

Do not update tracked benchmark history during an audit.

## Attribute before proposing

Use the narrowest evidence available: stage timers, counters, CPU and allocation profiles, call counts, scaling across controlled input sizes. Identify the hot path, its owner, frequency, per-item cost, and whether the cost is expected by the semantic contract.

Profiling says where time goes. It does not say which change is correct.

## Complexity

Express expected complexity in real compiler dimensions - files, tokens, modules, graph edges, visible symbols, types, generic instances, IR nodes, CFG blocks, reachable functions, diagnostics.

Look for nested scans over the same growing set, linear lookup inside loops, graph algorithms restarting per node, repeated sorting or deduplication, fixed-point loops rescanning avoidably, accidental quadratic string or template assembly, and path canonicalisation repeated per use.

Measure at more than one input size before claiming a scaling class.

## Repeated work

Inventory every traversal on the measured path: owner, input size, facts produced, and whether a later pass repeats it.

Common repeats: source rescans, repeated visibility or dependency lookup, repeated canonical-type projection, several HIR passes extracting overlapping facts, repeated reachability computation, repeated template traversal over the same exact view, backend rediscovery of helper needs, repeated filesystem inventory.

Reducing traversals is valid only when ownership survives it. Structural duplication without measured impact is Redundancy.

## Allocation

On the hot path: owned strings, paths and diagnostic prose; repeated `collect()`; whole-collection clones; per-node boxes; conversions between owned and borrowed forms; reference counting used for convenience; worker-local results cloned before merge.

Classify each as required ownership transfer, deliberate sharing, avoidable API friction, a retained index, or transient scratch. Count or profile before claiming an improvement.

## Data layout

Judge against the measured access pattern. Consider whether hot loops walk contiguous data, whether dense stable domains could index dense tables, whether interned IDs could replace cloned strings, and whether enum size inflates hot collections.

Do not propose a layout rewrite from intuition. Include expected benefit, memory trade-off and migration cost.

## Lookup and interning

Repeated hashing of owned strings; rendered-name comparison instead of ID comparison; temporary key allocation; maps where a sorted vector or dense index fits the access pattern; interner locks on parallel paths; full string-table cloning at ordinary module boundaries.

A faster lookup that confuses ID domains is invalid.

## Parallelism and caching

Check task granularity against scheduling overhead, serial bottlenecks, duplicate work across workers, merge and remap cost, and whether parallelism costs more peak memory than it saves latency. Determinism is not negotiable for throughput.

For caches: the key must be cheaper than recomputation and must contain every semantic input. A cache with an incomplete key is a Correctness defect, not an optimisation. Prefer deleting work over caching it.

## Instrumentation

Timers and counters must vanish from no-timer builds. Instrumentation must not allocate or take clocks when disabled, or alter execution order. Check the measurement does not include setup outside the intended metric.

## Every finding names its verification

Benchmark, baseline command, metric and direction, minimum evidence for acceptance, correctness gates, memory guardrails and variance handling - before the fix is proposed.

## Stale when

The measured path, its algorithms, data structures, caching, parallelism or instrumentation change materially. New workloads or benchmark cases may also invalidate a prior attribution.
