# Performance Audit

Read the [Codebase Audit Guide](../audit-guide.md) before using this guide. The repository validation and benchmark rules remain authoritative. A performance audit requires measured evidence for claims about speed, memory, allocation, throughput or scaling.

A performance audit is read-only. It records measured bottlenecks, scaling risks and bounded optimisation opportunities. It does not authorise semantic changes, weaker diagnostics, nondeterminism, test changes or speculative cache layers.

## Purpose and boundary

Use this audit to answer where time and memory are spent, why that work occurs and which owner can remove it without changing behaviour.

The audit covers:

- compile, check, build, dev and backend latency
- throughput and scaling with files, modules, declarations, HIR size and entries
- allocation, cloning and retained-memory cost
- repeated traversals and data transformations
- data layout and access patterns
- scheduling, parallelism and deterministic merge overhead
- caching, reuse and invalidation cost
- source, diagnostic, template, HIR, backend and output hot paths
- emitted runtime and artefact overhead where a benchmark owns it
- instrumentation and timer-erasure cost

Route these concerns elsewhere:

- repeated or obsolete structure without measured cost -> Redundancy
- behavioural defect -> Correctness
- poor readability -> Style
- weak benchmark or missing regression owner -> Tests
- user-error quality -> Diagnostics
- stale benchmark documentation -> Documentation

## Valid scopes

- A leaf scope is valid when profiling identifies a local hot path.
- A composite scope is the default for end-to-end subsystem cost.
- A contract scope is valid for repeated conversion, copying or scheduling across a boundary.
- A comparison scope is valid for backend, command, source-kind or algorithm alternatives measured under the same workload.
- A single file is not a complete performance scope unless the metric and call path are fully local.

A complete audit must name the metric, workload, baseline and evidence source before proposing optimisation findings.

## Audit procedure

### 1. Define the question and metric

Start with one concrete performance question.

Examples:

- why does frontend time scale faster than source size?
- which stage owns peak allocations during a full docs build?
- why is incremental dev rebuilding unaffected modules?
- which HIR operation dominates JS lowering?
- why does diagnostic rendering retain excessive path or string memory?

Define:

- command and profile
- target and builder
- workload or benchmark case
- cold, warm or incremental state
- latency, throughput, allocation, peak memory, artefact size or runtime metric
- expected scale dimension
- machine and relevant environment when results will be compared

Do not start by searching for `.clone()` and calling the result a performance audit.

### 2. Establish a usable baseline

Record:

- exact non-recording benchmark or timing command
- number of runs and warm-up policy
- current validation state
- benchmark stability and observed variance
- relevant feature flags such as `timers` or `detailed_timers`
- input size and repository state
- known background noise or environmental limits
- current memory and output measurements where relevant

Use medians or another justified robust summary. A single run is attribution evidence only when the effect is overwhelmingly large and still needs confirmation.

Do not update tracked benchmark history during an audit unless explicitly requested.

### 3. Attribute cost before proposing a fix

Use the narrowest available evidence:

- stage timers
- detailed counters
- CPU profiles
- allocation profiles
- heap snapshots
- call counts
- generated artefact inspection
- benchmark scaling across controlled input sizes
- targeted instrumentation kept outside the final code unless separately accepted

Identify:

- the hot call path
- the owner of the work
- frequency
- per-item cost
- allocation or retained-data shape
- whether time is serial, parallel, blocked or duplicated
- whether the cost is expected by the semantic contract

Profiling identifies where time is spent. It does not prove which change is correct.

### 4. Check algorithmic complexity and scaling

For hot paths, determine the expected complexity in terms of real compiler dimensions:

- files and source bytes
- tokens and declaration shells
- modules and graph edges
- visible symbols and exported items
- types and generic instances
- AST, TIR and HIR nodes
- CFG blocks and places
- entries, targets and reachable functions
- diagnostics and source labels
- assets and output records

Check for:

- nested scans over the same growing set
- repeated linear lookup inside loops
- graph algorithms restarting from every node
- repeated sorting or deduplication
- fixed-point loops with avoidable rescans
- accidental quadratic string or template assembly
- path canonicalisation repeated per use
- map or set choices with poor key construction cost
- unbounded recursion or stack depth

Measure at more than one input size before claiming a scaling class where practical.

### 5. Inventory traversals and repeated work

List every traversal on the measured path.

For each traversal, record:

- owner
- input size
- produced facts
- filtering or sorting performed
- whether a later pass repeats the same work
- whether the data could carry a useful derived index
- whether merging passes would mix semantic owners

Look for:

- source rescans and reparsing
- repeated visibility or dependency lookup
- repeated canonical-type projection
- several HIR passes extracting overlapping facts
- repeated reachability computation
- repeated template traversal over the same exact view
- backend rediscovery of helper or capability needs
- repeated manifest or filesystem inventory

A traversal reduction is valid only when it preserves ownership and keeps the resulting pass understandable. Route structural duplication without measured impact to Redundancy.

### 6. Audit allocations and cloning

On the hot path, inspect:

- owned strings, paths and diagnostic prose
- collection growth and repeated reallocation
- whole-vector or map cloning
- per-node boxes and small heap objects
- temporary collections used for one pass
- repeated `collect()` calls
- conversion between owned and borrowed representations
- reference-counted wrappers used only for convenience
- worker-local results cloned before deterministic merge
- generated artefacts retaining data after the owning stage
- output buffers and string concatenation

For each allocation, decide whether it is:

- required ownership transfer
- deliberate immutable sharing
- avoidable API friction
- a cache or retained index
- transient scratch data
- target output that must remain owned

Count or profile important allocations before claiming an improvement.

### 7. Review data layout and access patterns

Apply data-oriented design to measured access patterns.

Check whether:

- hot loops traverse contiguous vectors or scattered heap objects
- frequently accessed fields are grouped without dragging cold data through every pass
- IDs index dense tables where the domain is dense and stable
- sparse maps are used only where key sparsity or lifetime requires them
- side tables align cleanly with the owning IR or arena
- enum size and rare payloads inflate hot collections
- cloned strings or paths could become interned IDs with render-time lookup
- structure-of-arrays or split hot/cold records would materially improve the measured path
- object-style trait dispatch prevents batch processing or inlining
- dynamic allocation per node could become arena or store ownership
- table lookups repeat key hashing that a local ID could avoid

Do not propose a data-layout rewrite from intuition alone. Include expected access benefits, memory trade-offs and migration cost.

### 8. Review lookup and interning cost

Inspect hot symbol, type, path, string and identity lookups.

Check for:

- repeated hashing of owned strings
- rendered-name comparison instead of semantic IDs
- canonical-to-local projection repeated without a valid local cache
- temporary key allocation
- maps used where sorted vectors or dense indexing better match access
- linear scans on frequently queried member lists
- interner locks or clones on parallel paths
- full string-table cloning at ordinary module boundaries
- repeated path normalisation or filesystem metadata reads
- cache keys that rebuild large composite values per lookup

Preserve identity correctness and deterministic remapping. A faster lookup that confuses ID domains is invalid.

### 9. Review scheduling and parallelism

For measured parallel paths, check:

- task granularity versus scheduling overhead
- serial bottlenecks and lock contention
- duplicate work across workers
- shared mutable registries that prevent scaling
- canonical merge and remap cost
- load balance between ready jobs
- unnecessary waiting between independent stages
- thread creation or pool setup on short commands
- parallel work that increases allocations or peak memory more than it reduces latency
- completion-order dependence hidden by later sorting

Parallelism must preserve deterministic identity, diagnostics and output. Do not trade correctness for throughput.

Also check whether a serial data-oriented pass over contiguous data is faster and simpler than many tiny tasks.

### 10. Review caching, incremental reuse and invalidation

Where the measured path repeats work, inspect existing or proposed reuse.

Check that:

- a cache avoids measured work rather than merely moving it
- key construction is cheaper than recomputation
- every semantic and compatibility input appears in the key
- invalidation granularity matches public, implementation, root, runtime and documentation facts
- failed or diagnosed results are not reused unsafely
- cache lifetime and memory retention are bounded
- project-specific facts do not contaminate reusable package artefacts
- warm-path speed does not create unacceptable cold-path cost
- persistent serialisation does not depend on process-local IDs
- dev reuse and command policy share one architecture

A cache with an incomplete semantic key is a Correctness defect, not an optimisation.

### 11. Review diagnostics and failure-path cost

Where invalid input or large error sets are measured, check:

- path and prose cloning
- type rendering repeated for the same context
- duplicate diagnostics produced by repeated consumers
- sorting and remapping complexity
- recovery that continues expensive work after trust is lost
- oversized payloads retained across stages
- full rendered strings built before final output
- successful-path cost added solely for rare diagnostics

Do not weaken source context, stable identity or recovery quality to improve speed. Consider cold-path storage or delayed rendering instead.

### 12. Review templates, constants and generated work

When relevant, inspect:

- repeated TIR preparation, folding or exact-view traversal
- duplicate parse or formatter work
- string assembly complexity
- imported constant or template reuse
- generated request deduplication and fixed-point scheduling
- concrete generic materialisation repeated across entries
- base artefact cloning into generated sidecars
- helper-only artefacts retained after finalisation
- runtime template operations emitted when folding was possible

Respect TIR's AST-local boundary and generated artefact ownership. Caching a representation past its valid phase is not acceptable.

### 13. Review backend and runtime cost

For JS, Wasm or HTML artefact performance, check:

- selected-function reachability and dead emission
- helper and capability emission deduplication
- wrapper and cross-target call overhead
- string and template runtime helpers
- checked numeric operations
- map and collection runtime paths
- page runtime and memory instantiation
- duplicate external JavaScript glue
- artefact size and parse cost
- output write and skip-unchanged behaviour

Measure generated artefact or runtime behaviour with a representative contract. Do not infer runtime speed only from generated source size.

### 14. Review filesystem and output cost

Inspect:

- repeated directory inventory
- redundant metadata or canonicalisation calls
- source file reads and hashing
- output conflict checks
- manifest loading and writing
- skip-unchanged comparisons
- stale cleanup scans
- asset copying and deduplication
- dev-server watch invalidation

Preserve path safety, ownership and deterministic output. Avoid global caches that ignore project or profile identity.

### 15. Review instrumentation overhead

Check that:

- timers and counters disappear from no-timer builds where required
- timer-only strings, fields and environment checks are correctly gated
- instrumentation does not allocate or take clocks when disabled
- detailed metrics do not alter execution order or ownership
- benchmark-only hooks stay outside production hot paths unless gated
- measurements do not accidentally include setup work outside the intended metric
- nested timers account for wall time and accumulated work consistently

Instrumentation correctness issues may need a linked Correctness finding.

### 16. Evaluate trade-offs

For every candidate optimisation, record:

- expected metric improvement
- evidence supporting the cause
- added code, state and invalidation complexity
- memory versus time trade-off
- cold versus warm path effect
- serial versus parallel effect
- target-specific versus shared effect
- review and maintenance cost
- fallback if evidence does not reproduce

Prefer deletion of work over caching it. Prefer one batch pass over object-by-object indirection when the data access pattern supports it. Prefer simple proven improvements over broad speculative infrastructure.

### 17. Define verification before recommending the fix

Every finding must name:

- benchmark or workload
- baseline command
- metric and expected direction
- minimum practical evidence for acceptance
- correctness and validation gates
- memory or artefact-size guardrails
- variance handling
- platforms or targets that need comparison

Avoid arbitrary percentage thresholds unless the benchmark is stable enough and the project has accepted that policy.

### 18. Form the finding

A performance finding must state:

- measured symptom
- workload and baseline
- attribution evidence
- root owner
- suspected unnecessary work or poor data shape
- proposed bounded change
- expected metric effect
- semantic, diagnostic, deterministic and memory constraints
- required before-and-after measurements
- linked Redundancy, Tests or Correctness findings

Code inspection alone may create a profiling candidate, not a confirmed performance finding.

## Valid findings

Valid performance findings include:

- measured hot traversal or repeated computation
- algorithmic scaling inconsistent with the required operation
- avoidable high-volume allocation or cloning
- data layout that demonstrably harms a hot access pattern
- lock, scheduling or merge overhead that limits measured parallelism
- invalidation causing unnecessary recompilation
- backend or output emission doing measured unreachable or duplicate work
- instrumentation cost present when disabled
- cache overhead greater than saved recomputation

## Kind-specific preservation rules

A performance fix must preserve:

- accepted semantics and current support boundaries
- every existing test unchanged unless a linked Tests finding is accepted
- diagnostic identity, source context and recovery
- deterministic identity, ordering and output
- borrow and lifetime legality
- backend parity
- memory safety and bounded retention
- architecture ownership

A material regression in another relevant metric requires explicit approval. A faster patch that adds stale-cache risk or obscures ownership is invalid.

## Freshness invalidators

Mark a performance audit stale when the scope receives material changes to:

- algorithms or principal traversals
- data layout, allocation or interning
- scheduling, parallelism or deterministic merge
- caching and invalidation
- benchmark workloads or timer definitions
- backend or runtime emission
- output and filesystem flow

A comment or naming change does not stale performance. A semantic change may require a new baseline even when the implementation path is otherwise unchanged.

## Completion checklist

A complete performance audit confirms that:

- one metric, workload and baseline were defined
- variance and environment were recorded
- cost was attributed before fixes were proposed
- algorithmic scaling, traversals, allocations and layout were checked
- lookup, scheduling, reuse and invalidation were checked where relevant
- diagnostics, templates, backends, runtime and output were checked where relevant
- instrumentation overhead was checked
- every finding includes before-and-after evidence requirements
- speculative structural concerns were routed to Redundancy
- semantics, tests, diagnostics, determinism and memory safety remain explicit constraints
