# AUD-0002: Stage 0 source discovery and preparation performance

- State: `complete`
- Kind: `Performance`
- Primary scope: `build.stage0.discovery`, `build.stage0.preparation`
- Required context: `src/build_system/create_project_modules/compilation.rs`, `src/compiler_frontend/module_compilation/**`, `src/compiler_frontend/tokenizer/**`, `docs/build-system-design.md`, `docs/compiler-design-overview.md`
- Coverage: `partial`
- Reviewed: `2026-08`
- Baseline: `just bench-check` green at 29 cases, avg ~26ms, "baseline" fingerprint (macOS Apple Silicon 6D851D, 2026-08-21). No pre-existing failures observed.
- Revision: `bc6fa561d` (clean worktree)

## Scope, context and exclusions

The user selected "Stage 0 discovery / file preparation". At inspection time no registered audit
scope covered it — the registry held only `tests.harness`, `tests.support` and `tests.cases` — so
this run was conducted against a user-selected subsystem and recorded that registry gap as
AUD-0002-F06.

F06 was subsequently accepted and implemented: `audit-log.md` now registers
`build.stage0.discovery`, `build.stage0.preparation`, `build.stage0.graph`,
`build.stage0.scheduling` and the `build.stage0` composite. This report's coverage maps onto the
first two, and the Performance cell for each is recorded as `P` (see `Freshness update`).

The registered `build.stage0.discovery` leaf is **wider** than what this run inspected: it also owns
`project_structure_diagnostics.rs`, and `module_inventory.rs` was read for its discovery loop rather
than exhaustively. That gap is one of the reasons coverage stays `partial`.

### Primary surface (exhaustive)

| File | Lines | Covered |
|---|---|---|
| `source_tree_index.rs` | 1644 | yes |
| `source_discovery.rs` | 1441 | yes |
| `module_preparation.rs` | 1172 | yes |
| `module_inventory.rs` | 695 | yes |
| `project_roots.rs` | 204 | yes |
| `source_preparation.rs` | 190 | yes |
| `source_package_discovery.rs` | 135 | yes |
| `source_loading.rs` | 130 | yes |
| `prepared_source.rs` | 91 | yes |
| `source_discovery_error.rs` | 66 | yes |
| `prepared_module.rs` | 22 | yes |
| `project_structure_diagnostics.rs` | 69 | **no** — owned by `build.stage0.discovery`, not inspected |

`module_inventory.rs` and `module_preparation.rs` were read for the discovery loop, preparation
scheduling and merge paths that the measured call path reaches, not line by line. `source_tree_index.rs`
was read in full for the traversal but its unmeasured filesystem candidates were not profiled.

### Read-only context

`compilation.rs` (the two Stage 0 entry flows and the timing spans that bound them),
`module_identity.rs` (`nearest_module_for_directory` lookup cost only), `instrumentation/frontend_counters.rs`,
`timing/enabled/schema.rs`.

### Exclusions

- `compilation.rs` wave scheduling and publication, `compiled_boundary.rs`, `module_artifact_store.rs`,
  `generated_store.rs`, `project_module_graph.rs`, `module_namespace.rs`, `project_structure_diagnostics.rs`
  — these are Stage 0 scheduling/graph/publication, not discovery or file preparation.
- Everything under `stage0.directory.compile` (semantic module compilation). One cross-scope lead is
  recorded under "Leads outside this scope" without expanding coverage.
- Backend, output and runtime cost.

## Metric, workload and baseline

- **Question:** where does Stage 0 directory discovery and file preparation spend wall time, and is
  that work necessarily serial?
- **Metric:** `stage0.directory.inventory` wall time (and its child `boundary.inventory`), from the
  in-tree timing schema.
- **Command:** `./target/release/moth check <workload> --terse`, release + `timers` (plus
  `benchmark_counters` for the counter runs).
- **Workloads:** `docs` (344 prepared files, 36,960 tokens, 72 directories, 71 module roots — the
  largest real project in the repository), plus `benchmarks/module-graph`, `benchmarks/import-fanout`
  and `benchmarks/module-root-stress` as smaller scaling points.
- **State:** warm filesystem cache; first run of each series discarded.
- **Runs:** 7 per workload, median reported.
- **Machine:** macOS Apple Silicon, 10 cores (`hw.ncpu` = 10).

### Measured baseline

| Metric | `docs` median |
|---|---|
| `command.check.total` | ~320 ms |
| `build.frontend.total` | ~305 ms |
| `stage0.directory.inventory` | **~48.5 ms** (range 47.3–54.9 over 7 runs) |
| ↳ `boundary.inventory` (accumulated) | ~41.3 ms |
| ↳ traversal + resolver setup (inventory − boundary) | ~11.5 ms |
| `frontend.prepare` (accumulated) | ~12.0 ms |
| `stage0.directory.compile` | ~258 ms |

Stage 0 discovery and preparation is **~16% of frontend wall time** and ~15% of the check command
total on the largest available workload.

### Scaling

| Workload | prepared files | `stage0.directory.inventory` median | per file |
|---|---|---|---|
| `benchmarks/module-graph` | 10 | 1.18 ms | 118 µs |
| `benchmarks/import-fanout` | 13 | 1.62 ms | 125 µs |
| `benchmarks/module-root-stress` | 8 | 1.29 ms | 161 µs |
| `docs` | 344 | 53.87 ms | 157 µs |

Cost is **linear in prepared file count** across a 34× range, at roughly 120–160 µs per file. No
super-linear scaling defect was found; the problem is the constant and the serialism, not complexity.

## Authorities read

- `AGENTS.md`
- `docs/roadmap/audit-guide.md`
- `docs/roadmap/audit-kinds/README.md` and `audit-kinds/performance.md`
- `docs/build-system-design.md`: opening authority text, `Architectural invariants`,
  `Source indexing and source sets`, `Prepared-source orchestration`,
  `Deterministic scheduling and graph outcomes`
- `docs/roadmap/audit-log.md`, `docs/roadmap/open-audit-findings.md`, `docs/roadmap/audits/README.md`

## Existing findings and active plans checked

`open-audit-findings.md` holds no candidate, accepted, active or blocked findings. The only prior
report, AUD-0001, is a `tests.support` Redundancy audit with no overlap. No active plan under
`docs/roadmap/` claims Stage 0 discovery restructuring. No duplicate root cause found.

## Findings

### Attribution evidence

Sampled profile of `docs_check` via `just profile-case docs_check terse` (profiling profile,
`detailed_timers`, forced frame pointers), 317 samples over ~320 ms (≈1 sample/ms). The tool
reported `Symbolication status: failed_raw_addresses`, so frames were symbolicated manually with
`atos -o target/profiling/moth -arch arm64 -l 0x100000000`. Inclusive shares under
`module_inventory::discover_all_modules_in_boundary` (37 samples ≈ the ~41 ms `boundary.inventory`
span):

| Call site | Samples | ≈ share of inventory |
|---|---|---|
| `module_inventory.rs:595` `syntax.prepare_source(input)` (header preparation) | 16 | ~43% |
| `module_inventory.rs:530` `string_table.fork_for_module()` | 11 | ~30% |
| ↳ callee `StringTable::fork_source` (`string_interning.rs:418`) | 10 | ~27% |
| `module_inventory.rs:534` `ProjectPathResolver::clone` + its drop glue | ~7 | ~19% |
| `module_inventory.rs:586` `prepare_owned_source_input` (read + tokenize) | 6 | ~16% |

Shares are inclusive and overlap, so they do not sum to 100%. About 25 samples in this subtree
symbolicated to implausible frames (`core::fmt::float`, `core::net::parser::read_ipv4_addr`), which
are inlining or dSYM artefacts; those were discarded rather than attributed. Every share used below
is corroborated by at least two independent signals (caller line, callee symbol, drop glue, or an
in-tree counter).

### AUD-0002-F01: Directory Stage 0 discovery and preparation is fully serial while every parallel and caching mechanism is reachable only from the single-file synthetic path

- State: `closed`
- Kind: `Performance`
- Scope: `build.stage0.discovery` (root owner), `build.stage0.preparation`
- Priority: `unassigned`

#### Evidence

`stage0.directory.inventory` costs ~48.5 ms of a ~305 ms frontend on `docs` (~16%), preparing 344
files and 36,960 tokens. All of it runs on one thread:

1. **Thread count has no effect.** `RAYON_NUM_THREADS=1` vs default on a 10-core machine, 5 runs
   each: medians 48.65 ms and 49.31 ms. Identical within noise. Nine cores are idle for the whole
   phase.
2. **Every parallel-scheduling counter reads zero on a 344-file project.** With
   `timers,benchmark_counters`: `file_preparation_serial_module_count=0`,
   `file_preparation_parallel_module_count=0`, every `file_preparation_strategy_*=0`,
   `file_preparation_input_file_count=0`, `stage0_parallel_source_load_count=0`,
   `stage0_serial_source_load_count=0`, `stage0_source_cache_hit_count=0`,
   `stage0_source_cache_miss_count=0` — while `prepared_file_count=344`.
3. **Ownership trace.** `FilePreparationStrategy::selection_for_module` is reached only through
   `ModulePreparationContext::prepare_module` (`module_preparation.rs:486`), whose only production
   caller is `compilation.rs:377`/`:386` inside `compile_single_file_frontend`. Likewise
   `source_discovery::collect_reachable_input_files` — which owns the Stage 0 source cache and the
   Rayon `load_missing_sources_parallel` path — has exactly one production caller,
   `compilation.rs:301`, also single-file. `assemble_input_files_from_inventory` says so directly:
   "Directory projects do not enter this assembly path."
4. **The directory path instead** runs `discover_modules_serial_provider_capable`
   (`module_inventory.rs:485`): a serial `for seed in seeds` loop over all 72 modules, each running
   a serial BFS that calls `prepare_owned_source_input` (read + tokenize, one file at a time) then
   `syntax.prepare_source(input)`, both against one `&mut StringTable`.

So the two workloads have inverted treatment: the synthetic single-file mode, which compiles one
module, owns a chunked Rayon scheduler, a byte-threshold policy, a source cache and a parallel
loader; the directory mode, which compiles whole projects, owns none of them.

#### Counter-evidence checked

- **Is the serialism architecturally required?** Partly. `build-system-design.md` >
  `Prepared-source orchestration` states: "Provider-backed discovery remains serial while it mutates
  shared package identities, provider caches, resolution tables or diagnostic identity. Parallel
  provider discovery requires deterministic provider deltas and remapping first." The in-code
  comment on `discover_modules_serial_provider_capable` gives the same reason. **The full fix is
  therefore design-gated** and this finding does not propose lifting that gate.
- **But the gate is not exercised on this workload.** `resolved_provider_clause_count=0` and
  `bound_namespace_clause_count=0` for `docs`; all 349 dependency clauses resolve as
  `resolved_source_package_clause_count=349`, i.e. same-module/source-package resolutions that do
  not mutate provider registries. The serial constraint is a worst-case guard that the common case
  pays for unconditionally.
- **Is per-module string-table isolation missing?** No — it already exists.
  `ModuleSyntaxDiscovery` owns a forked `StringTable` per module and merges a delta back via
  `merge_delta_from` with a recorded `base_len`. The deterministic-merge machinery that parallelism
  needs is already built and in use.
- **Is the work simply irreducible?** No. Cost is linear at ~120–160 µs per prepared file across a
  34× size range, so this is a constant-factor and concurrency problem, not an algorithmic one.
- **Would parallel preparation change diagnostics or identity?** It must not, and
  `module_preparation.rs` already demonstrates the required contract (chunk-local tables, ordered
  merge, `file_index` validation). That contract is what any fix must reuse.

#### Violated contract or cost

No canonical contract is violated — the serialism is documented and accepted. The cost is measured:
~48.5 ms (~16% of frontend, ~15% of `check` total) on one core of ten, on the largest real workload
in the repository, growing linearly with project file count.

#### Impact

Every `check`, `build` and `dev` rebuild of a directory project pays full serial discovery and
preparation. The dev-server inner loop is the most exposed consumer. Impact grows linearly with
project size, so the largest projects pay the most.

#### Root owner

`module_inventory::discover_modules_serial_provider_capable`
(`src/build_system/create_project_modules/module_inventory.rs:485`).

#### Suggested correction

Non-authorising. In increasing order of design cost:

1. **Batch the provider-independent part first (no design gate).** File reading and tokenization
   (`prepare_owned_source_input`) touch no provider state. Tokenize the module's owned
   `CompilerSemantic` candidate sources into a batch up front, in parallel, against chunk-local
   string tables using the existing `fork_source`/merge contract; let the serial BFS then consume
   that batch and keep provider resolution serial. Cost: preparing owned-but-unreachable sources
   (344 of 359 owned semantic files are reachable on `docs`, so ~4% speculative work).
2. **Partition modules by provider need.** Modules whose retained clause shells contain no
   provider-capable target could be prepared in parallel; only provider-touching modules stay on the
   serial path. Requires the deterministic provider deltas that the design document already names as
   the prerequisite.

Do not simply move the single-file `prepare_module` path onto directory modules — the two consume
different `PreparedSourceInput` variants (`Moth` vs `MothPrepared`) and the directory path's BFS
reachability is what selects the semantic source set.

#### Allowed fix scope

`module_inventory.rs`, `module_preparation.rs`, `source_discovery.rs`, `source_preparation.rs`.

#### Read-only context

`compilation.rs`, `source_tree_index.rs`, `string_interning.rs`, `docs/build-system-design.md`.

#### Must preserve

Deterministic `SourceId`/`StringId` identity and remapping; diagnostic order independent of
completion order; the semantic source set selected by BFS reachability; the serial provider-mutation
boundary; `SourceTreeIndex` as sole source ownership owner; one tokenization per file
(`token_rescan_count` must stay 0).

#### Forbidden fix forms

Keeping the serial path alive beside a new parallel path; parallelising provider resolution without
the deterministic provider deltas the design document requires; achieving speed by weakening the
ownership assertions in the BFS loop; adding a second lexical scanner.

#### Required validation or measurement

`just validate`. Before/after medians of `stage0.directory.inventory` over ≥7 runs on `docs` plus at
least two small workloads (to prove no small-project regression from scheduling overhead), and
`just bench-check`. Confirm `token_rescan_count` stays 0 and `prepared_file_count` changes only by
the intended speculative-preparation delta.

#### Dependencies and related findings

F02 and F03 are independent, cheaper wins on the same call path and should land first — they change
the baseline this finding is measured against. F05 records the misleading comments.

#### Triage record

2026-08-21 — **Accepted and resolved.** Directory Stage 0 now batches provider-independent source
read and tokenization only when a module owns at least 16 compiler-semantic candidates. Each worker
uses an independent string-table fork; the existing deterministic BFS remains serial and merges only
the source it reaches before header preparation. Structural provider resolution, ownership checks,
module scheduling and diagnostic ordering remain serial and unchanged. Candidate sets below the
threshold continue through the established direct preparation path, avoiding a new small-module
allocation and remapping path.

The selected Moth input is remapped through the complete mutable `FileTokens` lifecycle, including
its file-owned path-syntax table, before header parsing consumes dense path handles. This required a
narrow tokenizer-owner extension and a focused regression test; no second path-table owner or
parallel semantic parser was introduced. The Stage 0 orchestration fixture covers the 15/16 policy
boundary, a 16-candidate module, deterministic canonical retained order, one read per candidate,
unreachable tokenizer failures after unique-text interning, zero header preparation for unreachable
candidates, discarded diagnostics/strings and a reachable path row that survives a non-identity
remap. Token rescans remain zero.

Coordinator measurements over the required warm seven-run series were `36.385 ms` for
`stage0.directory.inventory` on `docs` (F03 baseline `36.129 ms`), `1.086 ms` for
`module-graph` (baseline `1.062 ms`) and `1.419 ms` for `import-fanout` (baseline `1.421 ms`),
with prepared-file counts unchanged at `344/10/13`. The small-workload changes are within the
observed run noise while the large directory path now overlaps provider-free preparation. `just
bench-check` (29 cases) and `just validate` passed, including feature/source-audit findings at zero,
4,405 workspace tests, 1,851 integration cases, docs validation, benchmark sanity and timer
erasure. The required coordinator interim auditor pass 3 returned `audit_clean` with no findings and
no changed files. Targeted Samply profile-stack confirmation remains unavailable because macOS
reports `Unknown(1100)`; this is an environment limitation, not a validation failure.

### AUD-0002-F02: `fork_for_module` is called per module inside the discovery loop, copying the whole string table once per module against its own API guidance

- State: `closed`
- Kind: `Performance`
- Scope: `build.stage0.discovery`
- Priority: `unassigned`

#### Evidence

`module_inventory.rs:530`, inside the `for seed in seeds` loop:

```rust
let fork = string_table.fork_for_module();
```

`StringTable::fork_for_module` (`string_interning.rs:426`) is `self.fork_source().fork_for_module()`,
and `fork_source` (`string_interning.rs:418`) builds `Arc::new(StringTableBase::from_table(self))`.
`StringTableBase::from_table` (`string_interning.rs:106`) copies **every** string into a fresh
`Box<str>` and rebuilds a full `FxHashMap`. Its own doc comment says so: "Building the shared base
copies the current table once."

`fork_for_module`'s doc comment states the required usage directly:

> "Directory/module compilation should create one `StringTableForkSource` and reuse it for all
> independent module or file workers so the inherited prefix is copied once for the batch."

The directory loop does the opposite: it constructs a new `StringTableForkSource` per module, so the
prefix is copied 72 times for `docs` instead of once.

Profile: `module_inventory.rs:530` holds 11 of 37 inventory samples (~30%), and its callee
`StringTable::fork_source (string_interning.rs:418)` holds 10 — two independent signals for the same
~10 ms.

The copy also grows: `string_table.merge_delta_from(...)` at `module_inventory.rs:677` merges each
module's delta back into the shared table inside the same loop, so iteration *i* copies the base plus
the deltas of modules 0..*i*−1. Total work is O(M²·d) in interned strings for M modules. On `docs`
the constant base dominates (`string_table_delta_entries_scanned=22049` over
`string_table_delta_merge_calls=144`), which is why measured per-file cost stays roughly linear
today; the quadratic term is latent and grows with module count.

`module_preparation.rs:458` shows the intended shape on the other path — one `fork_source()` hoisted
above the chunk loop and shared by every worker.

#### Counter-evidence checked

- **Is the per-module fork needed for correctness?** No. `merge_delta_from(other, base_len)` requires
  only that IDs below `base_len` agree between the two tables. A single hoisted base T₀ is a prefix
  of the growing table for every iteration, so that invariant holds for all modules. This is exactly
  the contract the parallel path already relies on, where all chunks fork from one base.
- **Does hoisting change observable behaviour?** It should not change identity, but it does change
  *which* strings each module re-interns locally: a module would no longer inherit strings first
  interned by an earlier module in the same loop, so it re-interns them into its own delta. The
  remap in `merge_delta_from` handles that, but it shifts work from copying to re-interning, and the
  net effect must be measured rather than assumed.
- **Is `string_table_full_clones=72` this call site?** No — that counter is on `impl Clone for
  StringTable` (`string_interning.rs:208`), a different path. `StringTableBase::from_table` performs
  equivalent work and is **not** counted at all. The 72 per-module base copies are therefore
  invisible to the existing counter set, which is why this cost was not already visible. (See F04.)
- **Is this actually cheap because `Arc` is shared?** The `Arc::clone` in
  `StringTableForkSource::fork_for_module` is cheap; the expense is constructing a *new*
  `StringTableForkSource` per module, which is the uncounted full copy.

#### Violated contract or cost

The documented usage contract of `StringTable::fork_for_module` is violated by its own caller.
Measured cost ~10 ms, ~27–30% of the `boundary.inventory` phase and ~3% of `check docs` wall time,
plus a latent O(M²) term in module count.

#### Impact

Roughly 30% of Stage 0 directory inventory time on `docs`, scaling worse than linearly as projects
gain modules. Affects every `check`, `build` and `dev` cycle.

#### Root owner

`module_inventory::discover_modules_serial_provider_capable` at
`src/build_system/create_project_modules/module_inventory.rs:530`.

#### Suggested correction

Non-authorising. Hoist one `let fork_source = string_table.fork_source();` above the `for seed in
seeds` loop and call `fork_source.fork_for_module()` per module, matching `module_preparation.rs:458`
and the API's documented usage. Verify that `merge_delta_from`'s `base_len` argument becomes the
single hoisted base length and that the debug-assertion prefix check still holds. If the
re-interning increase measured under "Required validation" outweighs the saved copies, the fallback
is to rebuild the fork source only when the shared table has grown past a threshold — but prefer the
simple hoist unless measurement forces otherwise.

#### Allowed fix scope

`module_inventory.rs`. `string_interning.rs` only if a counter is added under F04.

#### Read-only context

`string_interning.rs`, `module_preparation.rs`.

#### Must preserve

`StringId` identity and deterministic remapping; the `merge_delta_from` prefix invariant and its
debug assertions; identical diagnostics and output; `string_table_delta_non_identity_remaps`
behaviour must remain correct (it need not remain 0).

#### Forbidden fix forms

Sharing a mutable string table across the loop to avoid forking; weakening or removing the
`base_len` debug assertions to make a hoisted base validate; caching a fork source past the point
where its base is no longer a prefix of the shared table.

#### Required validation or measurement

`just validate`. Before/after medians of `stage0.directory.inventory` over ≥7 runs on `docs`, plus
`string_table_delta_entries_scanned` and `string_table_delta_non_identity_entries` before and after
to quantify the re-interning trade. Re-run the profile to confirm the `fork_source` frame drops.
`just bench-check` for whole-suite effect.

#### Dependencies and related findings

Independent of F03; both should land before F01 is measured. F04 covers the missing counter that hid
this cost.

#### Triage record

2026-08-21 — **Accepted and resolved.** `discover_modules_serial_provider_capable` now creates one
immutable `StringTableForkSource` before the directory module loop and reuses it for every
module-local fork. The mutable module tables remain independent, and the existing
`merge_delta_from` base-prefix assertions, remapping, diagnostics, graph locations and output
publication are unchanged. Coordinator measurements over seven runs improved the `docs`
`stage0.directory.inventory` median from `49.746209 ms` to `37.729667 ms`; `module-graph` remained
effectively flat (`1.170750 ms` to `1.167292 ms`) and `import-fanout` remained effectively flat
(`1.556167 ms` to `1.541417 ms`). The intended trade-off is visible in
`string_table_delta_entries_scanned` (`22049` to `27978`), while non-identity remaps and token
rescans stayed at zero. `just validate` and post-change `just bench-check` passed. The targeted
`just profile-case docs_check terse` build completed, but Samply failed with macOS `Unknown(1100)`;
profile-stack confirmation is recorded as an environment limitation. The required coordinator
auditor pass 2 returned `audit_clean` with no findings and no changed files.

### AUD-0002-F03: `ProjectPathResolver` is deep-cloned once per module inside the discovery loop

- State: `closed`
- Kind: `Performance`
- Scope: `build.stage0.discovery`
- Priority: `unassigned`

#### Evidence

`module_inventory.rs:532-535`, inside the same per-module loop:

```rust
let preparation_context = ModulePreparationContext {
    style_directives,
    project_path_resolver: Some(project_path_resolver.clone()),
};
```

`ProjectPathResolver` (`path_resolution.rs:44`) owns two `PathBuf`s, a
`PreparedSourcePackageRoots`, a `ModuleRootTable` (71 entries for `docs`, including a
`HashMap<PathBuf, ModuleRootId>`) and a `SourceFileKindRegistry`. The clone is a deep copy of all of
it, performed 72 times per `docs` check, and each copy is dropped at the end of the iteration.

Profile corroboration from three independent frames in the inventory subtree:
`ProjectPathResolver::clone (path_resolution.rs:50)` ×3, `drop_glue<ProjectPathResolver>` ×4,
`ModuleRootTable` drop glue ×2 and `HashMap<PathBuf, ModuleRootId>::clone` ×1 — together ~7 of 37
inventory samples (~19%).

The resolver is not mutated anywhere in the loop; it arrives as `&ProjectPathResolver` from the
destructured `ModuleDiscoveryContext` and is cloned solely because `ModulePreparationContext` holds
it by value.

#### Counter-evidence checked

- **Does the lifetime force an owned copy?** No. `begin_syntax_discovery<'a>(&'a self, …) ->
  ModuleSyntaxDiscovery<'a>` ties the discovery to a borrow of the context, so the context only needs
  to outlive the loop body — and `project_path_resolver` outlives the entire loop. Hoisting one
  `ModulePreparationContext` above the loop, or changing the field to
  `Option<&'a ProjectPathResolver>`, both satisfy the borrow checker.
- **Is the owned field needed by the other caller?** The single-file path
  (`compilation.rs:369`) also clones, but exactly once per command, so it is not a hot path and can
  keep whichever shape the shared type ends up with.
- **Is the clone semantically meaningful — does any consumer mutate its copy?**
  `ModulePreparationContext` exposes no mutation of `project_path_resolver`; it is read-only
  throughout preparation.
- **Is ~19% credible from 7 samples?** It is the weakest of the three measured shares. It is
  reported as approximate and its acceptance is conditioned on the before/after measurement below
  rather than on the sample count alone.

#### Violated contract or cost

No contract violated. Measured cost ~5–7 ms, ~19% of the `boundary.inventory` phase, entirely
avoidable — this is copied data that no consumer mutates.

#### Impact

~19% of Stage 0 directory inventory, scaling with module count × module-root-table size, i.e.
quadratically in the size of a flat project. Also inflates peak allocation during Stage 0.

#### Root owner

`module_inventory::discover_modules_serial_provider_capable` at
`src/build_system/create_project_modules/module_inventory.rs:534`.

#### Suggested correction

Non-authorising. Construct one `ModulePreparationContext` above the `for seed in seeds` loop and
borrow it for every module; or change `ModulePreparationContext::project_path_resolver` to
`Option<&'a ProjectPathResolver>` and thread the borrow. Prefer whichever keeps the single-file
caller readable without a second context shape.

#### Allowed fix scope

`module_inventory.rs`, `module_preparation.rs` (the `ModulePreparationContext` field type), and
`compilation.rs` only for the single-file construction site.

#### Read-only context

`path_resolution.rs`, `module_roots.rs`.

#### Must preserve

Identical path resolution results and diagnostics; the provider-independence of
`ModulePreparationContext` (it must not gain provider-interface fields while being reshaped); one
context shape shared by both callers — do not fork a directory-only variant.

#### Forbidden fix forms

Introducing `Arc<ProjectPathResolver>` to dodge the lifetime when a plain borrow suffices; creating
a second parallel context type for the directory path; making the resolver mutable to share it.

#### Required validation or measurement

`just validate`. Before/after medians of `stage0.directory.inventory` over ≥7 runs on `docs`, and a
re-run profile confirming the `ProjectPathResolver::clone` and drop-glue frames disappear from the
inventory subtree. Because the effect is small, report it against measured variance rather than
claiming a fixed percentage.

#### Dependencies and related findings

Independent of F02; both are prerequisites for a clean F01 baseline.

#### Triage record

2026-08-21 — **Accepted and resolved.** The directory-discovery boundary now constructs one
`ModulePreparationContext` before the serial module loop, so its owned `ProjectPathResolver` is
cloned once per boundary rather than once per module. Each `ModuleSyntaxDiscovery` still borrows
that context only for its current iteration and retains independent mutable string, source-file,
origin and prepared-output state. Provider resolution continues to use the original borrowed
resolver and remains serial. Coordinator measurements over seven runs improved the `docs`
`stage0.directory.inventory` median from the F02 baseline `37.729667 ms` to `36.129209 ms`;
`module-graph` moved from `1.167292 ms` to `1.062042 ms`, and `import-fanout` from `1.541417 ms` to
`1.421000 ms`. Preparation and identity counters were unchanged, including 344 prepared files,
27,978 delta entries, zero non-identity remaps and zero token rescans. `just validate` and
post-change `just bench-check` passed. The targeted `just profile-case docs_check terse` build
completed, but Samply failed with macOS `Unknown(1100)`; profile-stack confirmation is recorded
as an environment limitation. The required coordinator auditor pass returned `audit_clean` with
no findings and no changed files.

### AUD-0002-F04: Stage 0 directory read + tokenize is unmeasured, and the counters that would expose it read zero on every directory build

- State: `closed`
- Kind: `Performance`
- Scope: `build.stage0.discovery` (boundary escalation — see below)
- Priority: `unassigned`

#### Evidence

This audit could not attribute Stage 0 cost from in-tree instrumentation alone; a manual samply
profile with hand symbolication was required. Three specific gaps caused that:

1. **`frontend.prepare` excludes the read and tokenize step on the directory path.** The
   `FrontendPrepare` metric is recorded around `prepare_file_frontend_local`
   (`module_preparation.rs:906`) and `prepare_header_syntax` (`:963`). On the directory path, the
   file read and tokenization happen earlier, in `prepare_owned_source_input`
   (`module_inventory.rs:586` → `source_discovery.rs:290`), which no metric covers. Measured:
   `frontend.prepare` accumulates ~12.0 ms while `boundary.inventory` is ~41.3 ms; the profile
   attributes ~6 of 37 inventory samples to the uncovered read+tokenize step.
2. **`StringTableBase::from_table` has no counter**, although the equivalent work in `impl Clone for
   StringTable` does (`StringTableFullClones`). This is precisely why F02's 72 per-module full
   copies were invisible.
3. **Nine Stage 0 counters read zero on every directory build**, so they give a false all-clear:
   `file_preparation_serial_module_count`, `file_preparation_parallel_module_count`, the four
   `file_preparation_strategy_*` counters, `file_preparation_input_file_count`,
   `file_preparation_input_byte_count`, `file_preparation_result_merge_count`,
   `stage0_source_cache_hit_count`, `stage0_source_cache_miss_count`,
   `stage0_parallel_source_load_count`, `stage0_serial_source_load_count` and
   `stage0_source_bytes_loaded`. All are on the single-file-only path (F01). `module_count`,
   `source_file_count` and `source_byte_count` also read 0 for directory builds because
   `record_module_input_counters` is only called from `compilation.rs:333` (single-file), even
   though the directory path prepares 344 files.

#### Counter-evidence checked

- **Is this a Correctness or Tests finding instead?** No behaviour is wrong and no regression
  protection is missing; the cost of the gap is that measured Stage 0 optimisation cannot be
  attributed or verified from the supported instrumentation. `audit-kinds/README.md` routes
  measured-cost concerns to Performance, and `performance.md` §15 makes instrumentation part of this
  lane. A linked Comments finding covers the misleading prose (F05).
- **Is the timer erasure contract at risk?** No. `just timers-erasure-check` passes in the baseline
  run, and all sites named here are already behind the `timers` / `benchmark_counters` gates. Any
  added metric must stay behind the same gates.
- **Do the zero counters cost anything when disabled?** No. `counter_observation!` and
  `add_frontend_counter` expand to nothing without both features, so this is an attribution defect,
  not runtime overhead.

#### Violated contract or cost

No canonical contract. The cost is that ~57% of `stage0.directory.inventory` (~29 ms of ~48.5 ms)
falls outside every recorded metric, and the counter set actively suggests the directory path
performs no preparation work at all.

#### Impact

Any future Stage 0 performance work — including F01, F02 and F03 — cannot be verified with
`just bench-check` and the timing schema alone. This also blocks the "Required validation" sections
above from being satisfied cheaply.

#### Root owner

`module_inventory.rs` (metric placement on the directory path) and
`instrumentation/frontend_counters.rs` (counter coverage).

#### Suggested correction

Non-authorising.

- Record `FrontendPrepare`, or a new sibling metric, around `prepare_owned_source_input` so the
  directory read+tokenize step enters the schema. If a new metric is added, give it
  `TimingParent::Metric(BoundaryInventory)` and `TimingAccountingRole::Evidence` so command
  accounting is unchanged.
- Count `StringTableBase::from_table` alongside the existing `StringTableFullClones`, or extend that
  counter to cover both.
- Call `record_module_input_counters` (or an equivalent) on the directory path so `module_count`,
  `source_file_count` and `source_byte_count` are non-zero for directory builds.
- Either make the single-file-only counters clearly path-scoped in their names/summary grouping, or
  let them stay zero once F01 gives the directory path real scheduling to report.

#### Allowed fix scope

**Boundary escalation.** Only `module_inventory.rs` lies inside the primary scope
(`build.stage0.discovery`). The fix also needs `instrumentation/frontend_counters.rs`,
`timing/enabled/schema.rs`, `timing/enabled/summary.rs` and `string_interning.rs`, none of which
belong to any registered scope yet. The primary scope cannot own the root fix because the metric
schema and counter registry are centralised owners by design — adding a Stage 0-local timer would
duplicate that ownership. This finding therefore requires separate triage before implementation
crosses that boundary, and the write scope stays limited to adding metrics and counters; it
authorises no behavioural change in any of those files.

#### Read-only context

`timing.rs`, `xtask/src/timers_erasure_check.rs`, `docs/src/docs/codebase/style-guide/validation.mtf`.

#### Must preserve

Full timer erasure in no-timer builds (`just timers-erasure-check` must pass); no allocation or
clock read when the features are disabled; existing metric stable names and schema order — add,
never renumber; command accounting totals unchanged (new metrics are `Evidence`, not `Pipeline`).

#### Forbidden fix forms

Adding an ungated counter or timer to the hot path; changing an existing metric's stable name or
accounting role to make a number look better; deleting the zero counters instead of either scoping
them or making the directory path report.

#### Required validation or measurement

`just validate`, including `just timers-erasure-check` and `just feature-lane-check`. Confirm
`stage0.directory.inventory` minus its newly recorded children leaves a small, explainable
remainder. Confirm a no-timer release build's byte scan still finds no timer-only marker.

#### Dependencies and related findings

Blocks cheap verification of F01, F02 and F03. Linked to F05.

#### Triage record

2026-08-21 — **Accepted and resolved.** This boundary escalation was accepted because timer and
counter ownership is centralized. Directory `prepare_owned_source_input` now runs under the
existing `FrontendPrepare` evidence metric, with no new pipeline metric or accounting change.
`StringTableBase::from_table` now increments the distinct stable
`string_table_fork_source_base_copies` counter alongside `string_table_full_clones`, and the
directory path records `module_count`, `source_file_count` and `source_byte_count` at the
successful prepared-module boundary. The counter summary exposes the new fork-base metric in the
existing string/remap group; synthetic-only scheduling/cache counters remain unchanged and are
not falsely attributed to directory discovery. Coordinator evidence reports warmed medians of
`stage0.directory.inventory=36.396375 ms`, `boundary.inventory=26.457666 ms` and
`frontend.prepare=18.038467 ms`; directory counters report 72 modules, 344 files,
`1,019,284` bytes, 344 prepared files, 2 fork bases, zero token rescans and zero non-identity
remaps. `just bench-check` and `just validate` passed, including feature-lane/source-audit
findings at zero and timer erasure with a clean `8,150,400`-byte no-timer binary. The required
coordinator auditor pass returned `audit_clean` with no findings and no changed files.

### AUD-0002-F05: Three doc comments describe parallel and cached Stage 0 behaviour that the directory path cannot reach

- State: `candidate`
- Kind: `Comments`
- Scope: `build.stage0.preparation` (root owner), `build.stage0.discovery`
- Priority: `unassigned`

#### Evidence

Filed as a linked secondary-lane finding for F01, per `audit-kinds/README.md`.

1. `module_preparation.rs:50-52`: "benchmark checks showed tiny modules regressing under Rayon, while
   fanout-style modules **and the documentation build** still benefit from parallel file
   preparation." The documentation build records
   `file_preparation_strategy_parallel_count=0` — it never reaches this policy at all.
2. `module_inventory.rs:465-467` on `discover_modules_serial_provider_capable`: "semantic module
   compilation remains serial while **each module may parallelize file preparation**." No module on
   this path parallelises file preparation; the function's own BFS prepares files one at a time.
3. `source_loading.rs:30-32`: "Stage 0 can load cache-miss source files in Rayon workers, then
   convert any `std::io::Error` on the serial boundary." True only for synthetic single-file
   discovery; `stage0_parallel_source_load_count=0` for directory builds.

Each comment describes a capability that exists in the codebase but is unreachable from the path the
comment sits on, which is what sent this audit looking for parallel work that was not there.

#### Counter-evidence checked

- **Are the comments simply describing the type's general capability rather than this call site?**
  For (2) that reading fails outright — the sentence is attached to the directory function and says
  "each module", meaning the modules that function processes. For (1) and (3) the wording names a
  specific workload (`the documentation build`) and a specific actor (`Stage 0`), so both assert more
  than a general capability.
- **Will F01 make them true?** Possibly, which is why this is a Comments finding rather than a
  request to delete the machinery. If F01 lands first, these comments become accurate and need only
  minor scoping. If F01 is deferred, they need correcting now.

#### Violated contract or cost

`AGENTS.md`: "Remove stale comments"; concise WHAT/WHY comments must describe the code they sit on.
No runtime cost.

#### Impact

Misleads readers and auditors about where Stage 0 concurrency exists. Directly cost this audit time.

#### Root owner

`module_preparation.rs`, `module_inventory.rs`, `source_loading.rs`.

#### Suggested correction

Non-authorising. Scope each comment to the path that can actually reach the behaviour — naming the
single-file synthetic path explicitly — or, if F01 is accepted and implemented first, update them to
describe the directory path's real scheduling.

#### Allowed fix scope

Comment text in the three named files only.

#### Read-only context

None beyond the three files.

#### Must preserve

No production code, signature or behaviour change under this finding.

#### Forbidden fix forms

Deleting the comments without replacing the intent; changing code to match the comments under a
Comments finding.

#### Required validation or measurement

`just validate` (comment-only change; no measurement required).

#### Dependencies and related findings

Secondary lane of F01. Sequence after F01 if F01 is accepted, so the comments are written once.

### AUD-0002-F06: The audit scope registry has no entry covering Stage 0, so this audit can record no freshness

- State: `candidate`
- Kind: `Documentation`
- Scope: `docs/roadmap/audit-log.md`
- Priority: `unassigned`

#### Evidence

The scope registry in `audit-log.md` contains exactly three scopes — `tests.harness`,
`tests.support`, `tests.cases` — all test-owned. No leaf, composite or contract scope covers
`src/build_system/**` or `src/compiler_frontend/**`. The audit guide states that "every maintained
implementation file belongs to exactly one leaf" and requires registry review "when a maintained
implementation file has no leaf owner". Roughly 11,000 lines under
`src/build_system/create_project_modules/` alone have no owner, and the entire compiler frontend is
likewise unregistered.

Consequence for this run: the guide requires marking coverage partial and recording a scope defect
when a scope is unregistered, and forbids promoting a freshness cell that does not exist. This audit
therefore produced findings but **could not record freshness**, so the work it did is invisible to
future automatic selection and may be repeated.

#### Counter-evidence checked

- **Should this audit have registered the scope itself?** No. Registry changes go through an
  accepted Documentation finding; an audit run is read-only with respect to canonical records beyond
  its own report, the open-findings index and its own freshness cell.
- **Is this simply an incomplete rollout rather than a defect?** Either way the guide's remedy is
  the same: record the scope defect and mark coverage partial. The finding notes the state without
  asserting the rollout was wrong.

#### Violated contract or cost

`audit-guide.md` > `Scope registry maintenance`: registry review is required when a maintained
implementation file has no leaf owner.

#### Impact

Structured audits of non-test code cannot record coverage, so freshness-driven selection cannot
route work to the compiler or build system at all.

#### Root owner

`docs/roadmap/audit-log.md`.

#### Suggested correction

Non-authorising. Register at least the scopes this run touched:

| Proposed ID | Kind | Primary coverage | Default context |
|---|---|---|---|
| `build.stage0.discovery` | Leaf | `source_tree_index.rs`, `source_discovery.rs`, `source_package_discovery.rs`, `project_roots.rs`, `module_inventory.rs` | `docs/build-system-design.md`, `compilation.rs` |
| `build.stage0.preparation` | Leaf | `module_preparation.rs`, `source_preparation.rs`, `source_loading.rs`, `prepared_source.rs`, `prepared_module.rs` | `docs/compiler-design-overview.md`, `string_interning.rs` |
| `build.stage0` | Composite | `build.stage0.discovery` + `build.stage0.preparation` + a scheduling leaf for `compilation.rs`/`compiled_boundary.rs`/`module_artifact_store.rs`/`generated_store.rs` | — |
| `contract.module_compilation_handoff` | Contract | `PreparedModuleInput` → `compile_module` → `CompiledModuleArtifact` publication | both design authorities |

If those are accepted, this report's coverage maps to `build.stage0.discovery` and
`build.stage0.preparation` and could be re-recorded against them.

#### Allowed fix scope

`docs/roadmap/audit-log.md`.

#### Read-only context

`AGENTS.md`, `index.md`, `docs/roadmap/audit-guide.md`.

#### Must preserve

Stable dotted conceptual IDs that do not encode movable paths; leaf scopes owning maintained
implementation exactly once; `-` for inapplicable kinds so automatic selection skips them.

#### Forbidden fix forms

Registering one giant `build` or `compiler` leaf that no single audit could cover exhaustively;
back-dating freshness cells for scopes that have not actually been audited.

#### Required validation or measurement

Documentation release-build gate (`cargo run -- check docs --terse`), not `just validate`.

#### Dependencies and related findings

Blocked freshness recording for this report and every future non-test audit.

#### Triage record

**Accepted and resolved, 2026-08.** The user accepted this finding and directed implementation.
`docs/roadmap/audit-log.md` now registers four Stage 0 leaves and one composite:

| Registered ID | Kind |
|---|---|
| `build.stage0.discovery` | Leaf |
| `build.stage0.preparation` | Leaf |
| `build.stage0.graph` | Leaf |
| `build.stage0.scheduling` | Leaf |
| `build.stage0` | Composite |

Together the four leaves own every maintained file under
`src/build_system/create_project_modules/` exactly once, satisfying the guide's rule that each
implementation file belongs to one leaf.

Two deviations from the suggested correction, both deliberate:

1. The suggestion folded module identity, namespace and the project module graph into
   `build.stage0.discovery`. They were split into their own `build.stage0.graph` leaf instead —
   they own structural graph topology rather than filesystem discovery, and merging them would have
   made `build.stage0.discovery` too wide for one audit to cover exhaustively.
2. `contract.module_compilation_handoff` was **not** registered. Its consumer half
   (`src/compiler_frontend/module_compilation/**`) still has no leaf owner, so a contract scope
   there would reference an unregistered counterpart. It remains proposed, not registered, and the
   compiler frontend as a whole is still unregistered — this finding resolved the Stage 0 gap only,
   not the general one.

## No-finding checks

These were checked with evidence and produced no finding. Recorded so a later audit does not repeat
them.

- **No source rescanning or double tokenization.** `token_rescan_count=0`,
  `prepared_file_count=344`, `file_preparation_pass_count=344`,
  `prepared_file_invariant_validation_count=344` — exactly one preparation pass per file. The
  architectural invariant "Tokenization and declaration-shell parsing happen once" holds on the
  measured path. `PreparedSourceInput::Moth` carries retained tokens precisely so header preparation
  does not re-lex.
- **No algorithmic scaling defect.** Inventory cost is linear in prepared file count at ~120–160 µs
  per file across a 34× range (10 → 344 files). No nested scan over a growing set, no graph
  algorithm restarting per node, no repeated sort or dedup was found on the measured path. The one
  latent super-linear term found is F02's, and it is dominated by a constant today.
- **`nearest_module_for_directory` is not a hot lookup.** It walks parent directories hashing a
  `Path` per level, called once per candidate (359×). On `docs`, module roots are dense (71 roots
  over 72 directories) so the walk terminates in ~1 step. It does not appear in the profile.
- **Traversal + resolver setup is a minor share.** `stage0.directory.inventory` −
  `boundary.inventory` ≈ 11.5 ms of ~48.5 ms. The single `SourceTreeIndex::discover` walk visits 72
  directories and sees 359 files once, consistent with the design requirement of one canonical
  traversal.
- **Instrumentation has no cost when disabled.** `counter_observation!` and `timed_stage!` expand to
  nothing without their features, `record_discovery_metrics` is `allow(unused_variables)` gated, and
  `just timers-erasure-check` passed in the baseline run. F04 is about missing measurement, not
  about overhead.
- **No duplicate source reads.** `read_source_code` has one boundary and the directory BFS's
  `queued` set guarantees each `SourceId` is handed to `prepare_owned_source_input` at most once per
  module; the ownership assertion in the loop enforces it.

## Profiling candidates (not confirmed findings)

Per `performance.md` §3 and §18, code inspection without attribution yields a profiling candidate,
not a finding. These did not appear in the profile and are recorded only so a later, larger-tree
audit can measure them.

- **`SourceTreeSkipPolicy::should_skip` calls `fs::canonicalize` per directory unconditionally**
  (`source_tree_index.rs:138`), including when `canonical_output_directories` is empty, in which case
  the `any()` it feeds is always false. An `is_empty()` guard would skip a `realpath` per traversed
  directory. Estimated cost is small on `docs` (72 directories); a standalone Python measurement put
  ~780 `realpath` calls at ~13 ms, suggesting ~1 ms here. Not confirmed by the profile. It also calls
  `directory.to_path_buf()` per `binary_search` probe.
- **`entries.sort_by_key(|entry| entry.path())` (`source_tree_index.rs:588`)** allocates a fresh
  `PathBuf` per comparison, i.e. O(n log n) full-path allocations per directory rather than O(n).
  `sort_by_cached_key`, or sorting on `file_name()`, would fix it. Not visible in the profile at this
  tree size.
- **`path.is_dir()` followed by `path.is_file()` per entry** (`source_tree_index.rs:597`, `:617`) is
  up to two `stat` syscalls that `DirEntry::file_type()` supplies from `readdir`'s `d_type` for free
  on this platform. **Counter-evidence: this is not behaviour-preserving** — `is_dir()` follows
  symlinks while `file_type()` does not, so a symlinked directory would change classification. Any
  change here must be treated as a symlink-semantics decision, not a pure optimisation.
- **`fs::canonicalize` per recognized source file** (`source_tree_index.rs:672`, ~359 calls) is
  defensive against symlinked ancestors. A cheaper equivalent would canonicalize only when an entry
  or ancestor is actually a symlink. Same symlink-semantics caveat applies.

## Leads outside this scope

Observed while auditing; recorded as links, not expanded into this run and **not** used to update any
other scope's freshness.

- **Semantic module compilation is also fully serial.**
  `module_compilation_serial_count=72`, `module_compilation_parallel_task_count=0`, and
  `stage0.directory.compile` is ~258 ms — 5× the phase audited here. This belongs to a Stage 0
  scheduling scope (`compilation.rs`), not to discovery or preparation.
- **`frontend.ast.environment.constant_header_resolution` is ~92 ms of the ~107 ms AST environment
  phase** on `docs`, the single largest measured cost in the command. That is a frontend AST scope
  concern.

## Limitations

- **Coverage is `partial`, and no freshness cell was updated.** The scope is unregistered (F06), so
  there is no cell to promote; the audit guide forbids inventing one.
- **Single machine, single platform.** All measurements are macOS Apple Silicon, 10 cores, warm
  filesystem cache. F01's parallelism headroom is a property of this machine's core count; a
  2-core CI runner would see a smaller win. No Windows or Linux comparison was run, and Windows
  filesystem-metadata cost differs materially, which matters for the profiling candidates above.
- **Profile symbolication was unreliable.** `just profile-case` reported
  `failed_raw_addresses`; frames were symbolicated manually. About 25 samples in the inventory
  subtree resolved to implausible symbols (`core::fmt::float`, `core::net::parser`) and were
  discarded rather than attributed. Every share used in a finding is corroborated by at least two
  independent signals; F03's ~19% rests on the fewest samples and is the least certain.
- **317 samples for a ~320 ms run.** Adequate for the ~30% and ~43% shares, thin for anything below
  ~5%. This is why the profiling candidates above were not promoted to findings.
- **`just profile-case docs_check` exited non-zero** on its untracked-files check, because this
  audit's own report and the modified open-findings index are untracked. The profile artefact was
  produced before that check and is valid; no benchmark history was written.
- **`build`/`dev` commands were not measured.** Only `check` was profiled. Stage 0 discovery and
  preparation is shared by all three, but output settings differ, which is exactly the input to the
  `should_skip` candidate above.
- **Cold-cache behaviour was not measured.** All runs were warm. The filesystem-syscall candidates
  would be relatively more expensive cold.
- **No workload above 344 files exists in the repository**, so F02's latent O(M²) term could not be
  demonstrated empirically — only derived from the code and shown to be masked by the constant base
  at current sizes.

## Freshness update

F06 was accepted and implemented, so the scopes this run covers now exist. Two cells were updated in
`docs/roadmap/audit-log.md`:

| Scope | Kind | Recorded |
|---|---|---|
| `build.stage0.discovery` | Performance | `P 2026-08 AUD-0002` |
| `build.stage0.preparation` | Performance | `P 2026-08 AUD-0002` |

`P` rather than `C`: coverage is `partial`. `project_structure_diagnostics.rs` was not inspected,
`module_inventory.rs` and `module_preparation.rs` were not read exhaustively, the four profiling
candidates in `source_tree_index.rs` were never measured, and only `check` was profiled on one
platform. The audit guide permits only a complete report to promote a cell to `C`.

No other cell was touched. In particular the Comments cell was **not** promoted from F05 and the
Documentation cell was not promoted from F06 — freshness is kind-specific and a cross-kind
observation cannot update another kind's coverage. `build.stage0.graph` and
`build.stage0.scheduling` remain `N` across the board; this run did not inspect them.
