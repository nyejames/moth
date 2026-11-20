# Performance

Find unnecessary compiler time, allocations and memory use through measurement and inspection of algorithms, data flow and Rust ownership. Prefer **removing work and simplifying code** over adding complex optimisations.

Follow the [audit guide](../audit-guide.md) for formal findings, coverage and allowed writes. Use the [validation guide](../../developer-docs/style-guide/validation.mtf) and current `justfile` as the authority for benchmark commands and build settings. Record unrelated defects or unconfirmed leads in [`docs/bug-reports.md`](../../bug-reports.md) without expanding audit coverage.

## Two kinds of useful findings

**Measured performance finding:** show a cost or regression with a workload, metric, baseline, repeated observations and an owning implementation. State how a later fix should be measured. A profile alone cannot prove a proposed change helps.

**Structural efficiency finding:** demonstrate avoidable work, such as a redundant traversal, unnecessary clone or temporary allocation. Explain why removing it simplifies the implementation. Label the impact **unmeasured** without claiming savings. A suspicious `.clone()` without an ownership trace is only a lead.

Both categories must preserve semantics, diagnostics, determinism and stage ownership. The evidence required to **accept a correction** depends on whether it removes complexity or introduces more of it. See [Acceptance thresholds](#acceptance-thresholds).

## 1. Select a question and workload

Start with a specific question such as "why does frontend time grow disproportionately with module count?" or "does this stage rebuild a symbol index its producer already owns?" Identify the stage, current owner, likely input dimensions and intended metric.

- Choose an existing representative benchmark where possible, not a synthetic case that only rewards one proposed change.
- Define the command, target, profile, input size and warm or cold state. Distinguish end-to-end latency, compiler-stage time, peak memory, retained storage and allocation counts.
- Record the relevant compiler revision, hardware, toolchain, feature flags and thread count. Keep them fixed for comparisons.
- Look for both common workloads and stress cases. A change benefiting one tiny case while regressing realistic projects needs justification.

## 2. Use Moth's non-recording tools

Choose the narrowest relevant command from the current `justfile`:

- `just bench-ci` runs a bounded non-recording sanity set after standard-case preflight.
- `just bench-check` runs the full CLI benchmark check without recording history.
- `just bench-frontend-check` runs the full frontend benchmark check without recording history.
- `just bench-scaling` checks declared scaling series and growth budgets.
- `just bench-validate` preflights benchmark cases and builds a release compiler. It does not replace correctness validation.
- `just bench-data-layout-check` covers the non-recording data-layout benchmark lane when representation size or memory matters.
- `just profile-case <case>` profiles a selected benchmark case. `just profile` and their `-symbolicated` variants cover broader profiling selection.
- `just profile-build` builds with profiling symbols, frame pointers and detailed timers.
- `just validate-perf` runs standard non-recording benchmark and scaling checks at the appropriate checkpoint.
- `just timers-erasure-check` verifies the no-timer release-artifact contract when instrumentation changes.

`just bench` and `just bench-frontend` can update recorded benchmark history. Do not use recording commands or commit generated measurements during a routine audit. Keep raw profiles and scratch measurements local. If a specific command changes, follow the `justfile` and validation guide rather than copying an old invocation.

CLI benchmarks use the release compiler. Frontend and scaling measurements run in-process in the development-built `xtask`. **Do not directly compare these configurations** as if they measured the same executable. Keep the configured Rayon thread count, build flags and toolchain stable. Run or describe only the checks relevant to the chosen hypothesis.

## 3. Establish a trustworthy baseline

- Repeat non-recording measurements under comparable conditions. Record run count, median, spread and any conspicuous outliers or host noise.
- Fix workload, compiler build, environment, thread count and cache state between comparable runs. Distinguish cold discovery from warm reuse.
- Use timers and `detailed_timers` to locate expensive stages. Use `benchmark_counters` or memory probes only where their instrumentation measures the intended question.
- Compare controlled input sizes before asserting scaling behaviour. Name the dimension growing: files, tokens, modules, graph edges, symbols, generic instances, IR nodes, diagnostics or reachable functions.
- Treat one noisy run or a percentage change within normal variance as inconclusive. Record a plausible lead without presenting it as a measured regression.

Measurements identify cost, not correctness. Skipping required work invalidates a faster result.

## 4. Attribute the work to its owner

Narrow the evidence in order: benchmark or stage timer, focused counter, profile, then the relevant call graph and data path.

Ask:

- How frequently does the operation execute and what grows its input size?
- Which producer owns the information being computed and which consumers need it?
- Does a measured hotspot perform useful semantic work, or reconstruct something already available?
- Does setup, filesystem inventory, compilation, merging, rendering or cleanup dominate the selected metric?
- Could instrumentation, optimisation level, cache state or thread scheduling explain the observation?

Use Samply or the repository's profiling workflow for CPU attribution where available. Allocation and memory claims need a relevant counter or profile, not just a code-level guess. State the measurement's limitations, including when a profile observes only one thread or a selected phase.

## 5. Hunt for avoidable work

Inspect measured hotspots and nearby opportunities for simpler data flow.

### Algorithms and repeated passes

- Linear lookup inside loops, nested scans over growing sets and repeated search through visible symbols or dependency edges.
- Sorting, deduplication or canonicalisation repeated for each consumer rather than at the owning boundary.
- Graph and fixed-point algorithms that restart from scratch, rescan unchanged work or have avoidable repeated convergence.
- Repeated source reading, tokenization, semantic resolution, type projection, template traversal or HIR passes extracting the same fact.
- Backend rediscovery of helper reachability, repeated filesystem inventory and repeated path canonicalisation.
- String or template building that repeatedly copies a growing buffer.

Show the repeated execution path. Derive likely complexity in the actual growing dimension and test multiple sizes before asserting a growth class. If the main defect is duplicated ownership without measurable cost, a structural efficiency or Redundancy finding is appropriate.

### Allocations, borrowing and Rust interfaces

- `.clone()`, `.to_owned()` or `to_string()` on whole collections, strings, paths and hot-path records.
- Repeated `collect()`, `Vec` rebuilding, intermediate maps, per-node boxes and temporary conversion buffers.
- APIs that take owned values only to read them, forcing callers to clone or allocate.
- Data cloned before a worker merge or across a stage boundary where moving, borrowing or a narrower result would suffice.
- `Arc`, `Rc` or other shared-ownership machinery introduced for convenience rather than a real lifetime requirement.
- Diagnostic prose or path text allocated early instead of carrying interned and typed facts to rendering.

Trace ownership and lifetimes. Clones may preserve independent mutation or a published snapshot. Avoid exchanging one copy for tangled lifetimes, synchronisation or another abstraction.

### Layout, lookups and concurrency

- Hot loops that jump through many allocations instead of walking relevant dense data.
- Repeated hashing of owned strings, temporary lookup keys, rendered-name comparison instead of valid typed IDs and maps used where dense indexing fits.
- Large common records or enums whose size multiplies over many retained nodes.
- Excessive interner locking, duplicate worker-local results, costly remapping, task granularity below scheduling overhead or parallel work that increases peak memory without useful speedup.

Data layout must follow the whole measured access and retention pattern, including construction, temporary overlap and destruction. Never trade valid ID domains, deterministic output or semantic ownership for a local lookup shortcut.

### Caches and instrumentation

- Can the compiler stop performing work instead of caching its result?
- Would a cache key be cheaper than recomputation and contain every semantic compatibility input?
- Does a counter, timer or profile hook allocate, take clocks or alter execution order when disabled?
- Does the no-timer build erase measurement overhead as the documented contract requires?

An incomplete cache key is a Correctness issue. Another cache or parallel work queue increases state and lifecycle complexity and needs strong measured justification.

## 6. Acceptance thresholds

Judge the **whole proposed change**, not only its fastest benchmark case.

### Deletion and simplification: lower threshold

Prefer corrections that remove redundant work, allocations, state or indirection while making ownership and behaviour clearer. Accept a well-evidenced structural cleanup when correctness checks pass and there is no meaningful evidence of a performance or memory regression. **A measurable speedup is not required** to justify clearer, smaller Rust.

Use affected non-recording benchmarks where cost or scaling could materially change. Treat small differences within measurement noise as inconclusive, not proof of an improvement or a reason to reject an otherwise sound simplification. Avoid inventing a benchmark for every harmless local cleanup.

### Additional code or complexity: higher threshold

Require convincing, repeatable benchmark evidence before recommending a new cache, specialised fast path, custom data structure, extra pass, broad abstraction or concurrency machinery for speed. The benefit should matter on representative workloads and exceed normal variance. Account for memory, startup, cleanup, determinism and maintenance cost.

If a more complex solution provides only a speculative or marginal win, retain the simpler design. A benchmark improvement alone does not override an architectural invariant.

Both classes still require appropriate correctness and documentation gates. Name the checks that could overturn the recommendation.

## Evidence, coverage and staleness

For a **measured finding**, record the exact non-recording command, workload, baseline revision, configuration, repeated metrics and variance, attributed owner, likely cause, counter-explanation and proposed verification. Separate observed cost from projected benefit.

For a **structural efficiency finding**, record the precise repeated or unnecessary operation, callers, ownership trace, reason it can be removed, expected simplification and regression checks. Mark timing and memory benefits unmeasured. Include the strongest reason to keep the existing code.

Report which workloads, profiles, target variants, stages and code paths were actually inspected. A profile sample or search inventory does not by itself establish complete coverage. Mark the audit partial where relevant work remains uninspected. Incidental issues go to `docs/bug-reports.md` without creating coverage for another kind.

Coverage becomes stale when measured paths, algorithms, representations, caching, parallelism or instrumentation change materially, or when benchmark workloads and configurations invalidate the recorded attribution.
