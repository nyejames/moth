# Frontend Optimisation Results

This file records concise evidence for the frontend arena and semantic-invariant optimisation
programme. Raw benchmark history and profiler output stay local-only under
`benchmarks/local-data/`.

## Phase 0 Baseline - 2026-06-18

### Baseline Environment

- Commit: `c263aa6cd7b3703fd6f97dfac92e42012e233585`
- Branch: `main`
- Machine: macOS Apple Silicon benchmark host `6D851D`
- OS: macOS `14.6.1` build `23G93`; Darwin `23.6.0` ARM64
- CPU/memory: Apple M1 Pro, 10 physical CPU cores, 16 GiB memory
- Rust: `rustc 1.95.0 (59807616e 2026-04-14)`, host `aarch64-apple-darwin`, LLVM `22.1.2`
- Cargo: `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`
- Just: `just 1.50.0`
- Samply: `0.13.1`

### Commands Run

- `just validate`
- `just bench-frontend-check`
- `just bench-check`
- `just bench-frontend` five recorded invocations
- `just bench` five recorded invocations, then five refreshed recorded invocations after the
  `template-stress.moth` fixture repair
- `just bench-report`
- `just profile-build`
- `samply record --save-only --output benchmarks/local-data/profiles/2026-06-18-docs-build.json.gz ./target/profiling/moth build docs --release`
- `samply record --save-only --iteration-count 5 --output benchmarks/local-data/profiles/2026-06-18-docs-build.json.gz ./target/profiling/moth build docs --release`
- `samply record --save-only --output benchmarks/local-data/profiles/2026-06-18-template-stress.json.gz ./target/profiling/moth check benchmarks/template-stress.moth`
- `samply record --save-only --output benchmarks/local-data/profiles/2026-06-18-environment-stress.json.gz ./target/profiling/moth check benchmarks/environment-stress.moth`

### Benchmark Suites

- End-to-end CLI suite: `benchmarks/cases.txt`, 16 cases across `core`, `docs`, `stress`,
  `module`, and `borrow`.
- Focused frontend suite: `benchmarks/frontend-cases.txt`, 8 cases across `core`, `docs`,
  `stress`, `module`, and `borrow`.
- Both suites use one warmup iteration and ten measured iterations per case.

The baseline repaired stale benchmark fixtures so the suites contain valid successful programs
under the current language rules: fallible collection `push` calls now use `catch`, the external
JS fixture uses free-function metadata instead of receiver-style JS annotations, the module-graph
fixture avoids a borrow alias, and the template-stress fixture uses the current default/named slot
routing shape.

Because some previous local records measured stale invalid fixtures, the first valid frontend
recorded run showed an apparent `+50ms` average movement. Treat that as a case-input correction,
not a compiler regression.

### Validation Status

- `just validate`: passed after the report and final fixture repairs. Clippy passed on native,
  Linux, and Windows targets; unit tests passed `2653/2653`; integration tests passed
  `1707/1707`; docs check passed; embedded `bench-check` passed with `+2ms avg`.
- `just bench-frontend-check`: passed on the final fixture set with `no measurable change:
  avg +3ms; 8/8 cases`.
- `just bench-check`: passed on the final fixture set with `0ms avg; 0 faster, 1 slower; 16/16 cases`.

### Latest Drilldown

`just bench-report` after the refreshed end-to-end runs:

- End-to-end CLI latest: `2026-06-18T10:08`, `no measurable change: avg -1ms; 16/16 cases`.
- Slowest end-to-end cases:
  - `check_docs`: about `185ms`; `ast_ms` about `902ms`, `ast_build_environment_ms` about `500ms`.
  - `check_benchmarks_type-stress_bst`: about `65ms`; `ast_ms` about `54ms`, `ast_build_environment_ms` about `42ms`.
  - `check_benchmarks_environment-stress_bst`: about `61ms`; `ast_ms` about `51ms`, `ast_build_environment_ms` about `39ms`.
- End-to-end stage movement: `file_prepare_ms -5ms`, `ast_ms +3ms`, `ast_finalize_ms +2ms`,
  `ast_emit_nodes_ms -1ms`; no counter movement.
- End-to-end top ratios: `fold-stress` has high `file_prepare_ms/source_file_count`;
  `type-stress` and `environment-stress` have high `ast_ms/ast_header_count`.

Focused frontend latest recorded drilldown:

- Frontend latest: `2026-06-18T01:22`, `no measurable change: avg 0ms; 8/8 cases`.
- Slowest focused frontend cases:
  - `frontend_docs`: about `437ms`; `ast_ms` about `58ms`, `ast_build_environment_ms` about `30ms`.
  - `frontend_benchmarks_type-stress_bst`: about `134ms`; `ast_ms` about `108ms`, `ast_build_environment_ms` about `83ms`.
  - `frontend_benchmarks_environment-stress_bst`: about `123ms`; `ast_ms` about `104ms`, `ast_build_environment_ms` about `80ms`.
- Frontend stage movement: `ast_ms +1ms`, `ast_build_environment_ms +1ms`; no counter movement.
- Frontend top ratios point at `type-stress`, `environment-stress`, and `collection-stress` for
  tokenization/header parsing/string-table merge-remap investigation.

Useful volume counters from the final end-to-end docs check:

- `source_file_count`: `125`
- `source_byte_count`: `527551`
- `token_count`: `41331`
- `header_count`: `1640`
- `dependency_clause_count`: `850`
- `top_level_declaration_count`: `1365`
- `template_count`: `4776`
- `module_remap_string_ids_calls`: `31`
- `string_table_merge_source_entries_scanned`: `4918`

### Samply Result

The three local Samply profile files were produced, but each contains zero samples:

- `benchmarks/local-data/profiles/2026-06-18-docs-build.json.gz`
- `benchmarks/local-data/profiles/2026-06-18-template-stress.json.gz`
- `benchmarks/local-data/profiles/2026-06-18-environment-stress.json.gz`

These files are not useful for function-level hotspot attribution. Phase 1 should use the
benchmark stage/counter evidence as the baseline. A repeated docs profile with five command
iterations still produced zero samples, so future profiling should use a longer workload or a
different profiler setup only if function-level attribution is needed before a specific refactor.

### Baseline Findings

- AST construction, especially AST environment building, is the dominant compiler-stage signal in
  docs, `type-stress`, and `environment-stress`.
- File preparation remains worth watching in `fold-stress`, `type-stress`, `environment-stress`,
  and `collection-stress`, especially tokenization/header parsing/string-table merge-remap ratios.
- Current counter comparisons show no movement after the fixture repairs; the baseline is stable
  enough to start Phase 1 stats and capacity-estimate work.
- No compiler semantics changed in Phase 0. The committed fixture changes only make benchmark
  inputs conform to current documented language rules.
- Documentation drift noted: `benchmarks/README.md` still describes the external JS benchmark as
  covering external receiver methods. Current language rules and the repaired fixture expose
  external JS functions as free functions only.

### Semantic Invariants For Optimisation Review

- No visible shadowing: scope-frame arenas can use parent-linked frames and ancestor
  redeclaration checks instead of cloned shadow stacks.
- Header parsing owns top-level discovery: token/header counts are valid capacity seeds, and AST
  must not rediscover top-level declarations.
- Dependency sorting is authoritative: AST should not grow fixpoint ordering passes for constants,
  aliases, structs, choices, or signatures.
- Header-built visibility is authoritative: body-local scope frames should reference immutable
  header/import visibility instead of copying it into children.
- One entry start path: start-specific structures should be allocated only when a start header
  exists.
- Generics resolve before HIR: generic template storage can stay AST-local and should not leak
  unresolved generic calls into HIR.
- Traits are static metadata: evidence maps can be compact stable-ID tables; no runtime trait
  object metadata is needed.
- External packages expose free functions only: avoid external receiver-method catalogs and share
  immutable external metadata where ownership permits.
- Canonical `TypeId` is semantic identity: arena nodes should carry `TypeId`s and compact IDs, not
  cloned semantic type trees.
- No closures or general function values: function-local scope arenas do not need capture
  promotion for runtime function values.
- No macro expansion language: no hygiene or repeated parse/expand/fold arena is needed.
- Borrow validation is side-table based: future borrow-fact compaction should keep facts outside
  HIR nodes.

## Phase 1 Stats And Capacity Estimates - 2026-06-18

### Scope

Phase 1 added `src/compiler_frontend/arena/` as the frontend-local owner for cheap token/header
statistics and capacity-estimate policy:

- `TokenStats` is accumulated during the existing tokenizer loop and travels with `FileTokens` and
  `FileFrontendPrepareOutput`; it carries counts only, so string-ID remapping is unaffected.
- `HeaderStats` is computed from the already-aggregated module header list and module symbol
  package, including functions, constants, structs, choices, type aliases, traits, conformances,
  trait incompatibilities, const templates, start functions, imports, generic parameters,
  signature members, choice variants, and dependency edges.
- `FrontendArenaCapacityEstimate` centralizes conservative, capped estimates for scope frames,
  declarations, expressions, expression items, statements, templates, template atoms, render
  pieces, HIR blocks/statements/expressions, and borrow facts.
- Detailed-timer counters now report the estimated scope-frame count and capped-estimate count.
  Actual scope-frame and scope-arena-capacity counters intentionally remain zero until Phase 4
  creates real scope-frame arena storage.

Parent review corrected two estimate-quality details before acceptance: map/collection delimiters
now count curly braces and commas only, and trait requirement signatures contribute to
`HeaderStats.signature_members`.

### Validation Status

- Focused unit tests:
  - `cargo test --quiet token_stats`: passed, `4/4`.
  - `cargo test --quiet header_stats`: passed, `4/4`.
  - `cargo test --quiet capacity`: passed, `49/49`.
- `just bench-frontend-check`: passed at `2026-06-18T10:59`, `no measurable change: avg 0ms`;
  `8/8` cases, stage movement `ast +9ms`, `ast env +4ms`, `ast emit +3ms`.
- `just bench-check`: passed at `2026-06-18T11:01`, `+2ms avg`; `0 faster`, `2 slower`,
  `16/16` cases, stage movement `ast env +21ms`, `ast +21ms`, `file prep +11ms`.
- `just validate`: passed after Phase 1 corrections and plan/report updates. Clippy passed on
  native, Linux, and Windows targets; unit tests passed `2667/2667`; integration tests passed
  `1707/1707`; docs check passed; embedded `bench-check` passed with `+3ms avg`.

The benchmark movement is below the plan's rollback threshold and is consistent with noise for an
instrumentation-only slice. Stats and estimates remain policy-only; they do not affect diagnostics,
ordering, lowering, type identity, or emitted artifacts.

### Audit Notes

- Stage boundaries remain intact: token stats belong to tokenization output, header stats belong to
  header aggregation output, and capacity formulas live in `arena/capacity.rs` rather than pipeline
  orchestration.
- There is no new source/token traversal. Token stats are collected in the lexer loop; header stats
  are a cheap pass over already-aggregated headers after header parsing has completed.
- No dedicated output fixture was added because stats are not semantically consumed. Existing
  integration/golden validation is the regression owner for diagnostics and output equivalence.
- The progress matrix was not updated in this slice because no language feature support changed;
  the active plan still reserves roadmap/progress documentation work for Phase 9.

## Phase 2 Adversarial Benchmark Fixtures - 2026-06-18

### Scope

Phase 2 added `benchmarks/adversarial/` with seven single-file compiler-churn fixtures and one
small HTML project fixture:

- `one-module-kitchen-sink.moth` combines imports, constants, aliases, nominal types, choices,
  traits, generics, templates, collections, maps, receivers, and external package calls.
- `deep-scope-churn.moth` targets nested function/block/loop scope-frame pressure.
- `template-render-plan-churn.moth` targets slot routing, `$children` wrappers, repeated slot
  replay, and runtime template rebuilding.
- `constant-dag-churn.moth` targets compile-time dependency sorting and constant/template folding.
- `expression-rpn-churn.moth` targets expression parsing/lowering, choice matching, mutable stack
  operations, and checked arithmetic.
- `generic-trait-churn.moth` targets generic instantiation, trait evidence, and bound-provided
  receiver calls.
- `collection-map-borrow-churn.moth` targets valid collection/map mutation, fallible operations,
  receiver calls, and borrow-checker facts.
- `import-external-churn/` targets project import fanout, core package calls, and external
  JavaScript free-function metadata.

No generator was added. The initial adversarial set is clearer as hand-authored static source, and
the committed `.moth`/`.js` files are the canonical benchmark inputs.

### Validation Status

- `just bench-frontend-check`: passed at `2026-06-18T18:16`, expanding the focused frontend suite
  to `16` cases. The expected case-set change showed `avg +4ms` on the `8/16` shared cases, with
  stage movement `ast +10ms`, `ast env +7ms`, and `ast emit +3ms`.
- `just bench-check`: passed at `2026-06-18T18:16`, expanding the end-to-end suite to `25` cases.
  The expected case-set change showed `avg +2ms` on the `16/25` shared cases, with stage movement
  `ast -23ms`, `ast env -10ms`, and `file prep +10ms`.
- `just profile-case check_benchmarks_adversarial_one-module-kitchen-sink_moth terse`: the first
  sandboxed run reached Samply but failed with `Unknown(1100)`. Rerunning the same command with
  approved escalation passed and wrote local-only artifacts under
  `benchmarks/local-data/profiles/2026-06-18T18-22-55-d82ffd27/`.
- `just validate`: passed after Phase 2 docs and plan updates. Clippy passed on native, Linux, and
  Windows targets; unit tests passed `2667/2667`; integration tests passed `1707/1707`; docs check
  passed; embedded `bench-check` passed with the expected case-set change at `avg +3ms` on the
  `16/25` shared cases.

The targeted profile observed `one-module-kitchen-sink` at about `32ms` wall time. Stage attribution
pointed to `ast_ms` at about `24ms`, with `ast_build_environment_ms` about `17ms`,
`ast_emit_nodes_ms` about `6ms`, `borrow_ms` about `2ms`, and `file_prepare_ms` about `1ms`. The
profile captured only `50` samples and remained unsymbolicated, so the stack addresses are not
useful function-level evidence. The stage/counter observations are still useful for Phase 3 and
Phase 4 targeting.

Useful counters from the profile:

- `source_file_count`: `2`
- `source_byte_count`: `7280`
- `token_count`: `1665`
- `header_count`: `61`
- `top_level_declaration_count`: `58`
- `template_count`: `92`
- `const_template_count`: `55`
- `runtime_template_count`: `37`
- `ast_function_count`: `16`
- `ast_struct_count`: `7`
- `ast_choice_count`: `2`
- `ast_generic_template_count`: `4`
- `ast_generic_instance_count`: `2`
- `borrow_statement_fact_count`: `231`
- `borrow_value_fact_count`: `525`
- `estimated_scope_frames`: `107`

### Audit Notes

- The adversarial fixtures are successful programs/projects, not diagnostic cases.
- No `dev/` or `release/` generated project output folders were added.
- The new single-file fixtures live in the existing `stress` group; the multi-file external import
  project lives in the existing `module` group.
- `benchmarks/README.md` now records the adversarial fixture purpose and corrects the older
  external JS fixture description from receiver methods to external free functions.

## Phase 3 External Package Registry Clone Reduction - 2026-06-18

### Scope

Phase 3 replaced deep external-package registry clones through the frontend, AST environment,
`ScopeContext`, `Module`, and backend consumers with a shared immutable
`Arc<ExternalPackageRegistry>` handle. The registry remains mutable only during library setup,
project config parsing, and Stage 0 external import discovery; after discovery, compiled modules
and backend lowerers share the frozen registry snapshot.

This phase also added detailed-timer clone-pressure counters for:

- `external_package_registry_clone_count`
- `external_package_definition_clone_count`
- `external_function_definition_clone_count`
- `external_symbol_path_clone_count`
- `external_abi_parameter_clone_count`

Parent review adjusted the worker patch so ownership-carrying contexts own an `Arc`, while
read-only token/header preparation and backend validation call sites borrow the underlying
`ExternalPackageRegistry` through `.as_ref()`.

### Clone Counter Movement

The worker captured baseline counter values after adding counters and before the ownership
reduction, then reran the same import-heavy cases after the reduction. Parent targeted profiles
confirmed the reduced counts after review cleanup.

| Case | Registry | Package | Function | Symbol path | ABI parameter |
|---|---:|---:|---:|---:|---:|
| `external-js-imports` before | 75 | 675 | 14,625 | 31,531 | 32,850 |
| `external-js-imports` after | 1 | 9 | 195 | 451 | 438 |
| `import-external-churn` before | 161 | 1,288 | 31,073 | 67,014 | 69,874 |
| `import-external-churn` after | 1 | 8 | 193 | 454 | 434 |

The remaining registry clone is the config/build ownership boundary where the mutable builder
library registry is frozen into an immutable frontend snapshot. Definition/path/ABI clones that
remain are registration-time ownership inside registry maps, plus owned builder-runtime metadata
that still belongs to each module's backend handoff.

### Validation Status

- `cargo check`: passed after parent cleanup.
- `cargo test --quiet external_packages`: passed, `46/46`.
- `cargo test --quiet provider_registry_tests`: passed, `15/15`.
- `cargo run -- check benchmarks/external-js-imports`: passed.
- `cargo run -- build benchmarks/external-js-imports`: passed.
- `cargo run -- check benchmarks/adversarial/import-external-churn`: passed.
- `cargo run -- build benchmarks/adversarial/import-external-churn`: passed.
- `just bench-frontend-check`: passed at `2026-06-18T18:55`, `avg -47ms` on the `8/16`
  shared cases, with stage movement `ast -296ms`, `ast env -200ms`, and `ast emit -62ms`.
- `just bench-check`: passed at `2026-06-18T18:57`, `avg -14ms` on the `16/25` shared cases,
  with stage movement `ast -721ms`, `ast env -547ms`, and `ast emit -166ms`.
- `just profile-case check_benchmarks_external-js-imports terse`: passed with approved profiler
  escalation and wrote local-only artifacts under
  `benchmarks/local-data/profiles/2026-06-18T18-58-36-8e594d03/`.
- `just profile-case check_benchmarks_adversarial_import-external-churn terse`: passed with
  approved profiler escalation and wrote local-only artifacts under
  `benchmarks/local-data/profiles/2026-06-18T18-58-45-8e594d03/`.
- `just validate`: passed after Phase 3 parent cleanup and report updates. Clippy passed on
  native, Linux, and Windows targets; unit tests passed `2667/2667`; integration tests passed
  `1707/1707`; docs check passed; embedded `bench-check` passed with the expected case-set change
  at `avg -14ms` on the `16/25` shared cases.

The targeted profiles captured only `30` and `45` samples and remained unsymbolicated, so their
raw stack addresses are not useful function-level evidence. Their stage/counter observations are
useful: `external-js-imports` now checks at about `13ms` with `ast_ms` about `3ms`, and
`import-external-churn` checks at about `20ms` with `ast_ms` about `5ms`.

### Audit Notes

- External package metadata remains immutable after Stage 0 discovery and is not exposed through a
  mutable shared handle.
- Backends still validate and lower against the exact registry used by frontend resolution.
- No backend rediscovery path or duplicate package metadata path was introduced.
- The progress matrix was not updated in this slice because no language feature support changed;
  the active plan still reserves roadmap/progress documentation work for Phase 9.

## Phase 4 ScopeFrame Arena Refactor - 2026-06-18

### Scope

Phase 4 replaced the cloned flat local-declaration state in `ScopeContext` with a typed `Vec`
arena of parent-linked scope frames:

- `ScopeArena` owns `ScopeFrame` storage and creates stable `ScopeFrameId` handles.
- Child expression, template, constant, block, branch, and loop contexts allocate child frames with
  parent IDs instead of cloning all visible locals.
- Body-local functions allocate fresh root frames and receive parameters only, preserving the
  no-closures/no-implicit-capture language invariant.
- Local lookup now returns `ScopeDeclarationRef`, which hides whether the declaration is a local
  arena-owned `Rc<Declaration>` or a borrowed top-level declaration from immutable module lookups.
- `ScopeContext::clone()` creates a shallow copy of the current frame so match arms, value arms,
  and catch-helper contexts can add captures without mutating the original frame.
- Detailed counters now report actual scope frames, scope arena capacity growth, maximum frame
  depth, local declaration insertions, lookup ancestor steps, and redeclaration ancestor checks.

The old scope-local clone counter was removed because there is no remaining flat local-declaration
clone path. Capacity preallocation from `FrontendArenaCapacityEstimate` is intentionally deferred
to Phase 5. The new actual counters show the Phase 1 estimate formulas undercount scope-heavy
fixtures, so Phase 5 should tune formulas before using them as arena capacity seeds.

### Validation Status

- Worker validation before parent review:
  - `cargo fmt`: passed.
  - `cargo check`: passed.
  - `cargo clippy`: passed.
  - `cargo test --quiet scope_context`: passed.
  - `cargo test --quiet`: passed, `2677/2677`.
  - `cargo run -- check benchmarks/environment-stress.moth`: passed.
  - `cargo run -- check benchmarks/adversarial/deep-scope-churn.moth`: passed.
  - `just validate`: passed.
- Parent validation after review corrections:
  - `cargo fmt`: passed.
  - `cargo check`: passed.
  - `cargo test --quiet scope_context`: passed, `17/17`.
  - `cargo run -- check benchmarks/environment-stress.moth`: passed.
  - `cargo run -- check benchmarks/adversarial/deep-scope-churn.moth`: passed.
  - `git diff --check`: passed.
  - `just validate`: passed. Clippy passed on native, Linux, and Windows targets; unit tests
    passed `2677/2677`; integration tests passed `1707/1707`; docs check passed; embedded
    `bench-check` passed with `avg -14ms` on `16/25` shared cases and stage movement
    `ast -725ms`, `ast env -551ms`, `ast emit -167ms`.

### Benchmark Results

- `just bench-frontend-check`: passed at `2026-06-18T20:08`, `case set changed: avg -46ms`
  on the `8/16` shared cases, `0 slower`, `8 faster`. Stage movement:
  `ast -296ms`, `ast env -199ms`, `ast emit -62ms`.
- Five recorded `just bench-frontend` invocations:
  - first recorded Phase 4 run showed `case set changed: avg -46ms` on `8/16` shared cases,
    `0 slower`, `8 faster`, with stage movement `ast -297ms`, `ast env -199ms`,
    `ast emit -63ms`;
  - the next four runs reported `no measurable change` against the accepted Phase 4 run,
    confirming stable medians within the benchmark system's rough threshold.
- Five recorded `just bench` invocations:
  - first recorded Phase 4 end-to-end run showed `case set changed: avg -14ms` on `16/25`
    shared cases, `2 slower`, `12 faster`, with stage movement `ast -724ms`,
    `ast env -552ms`, `ast emit -166ms`;
  - the next four runs reported `no measurable change` against the accepted Phase 4 run.
- Latest `just bench-report` after the five-run sequence shows:
  - End-to-end latest: `no measurable change: avg 0ms; 25/25 cases`.
  - Frontend latest: `no measurable change: avg 0ms; 16/16 cases`.
  - Remaining next-investigation ratios point at file preparation for `fold-stress`, docs,
    `type-stress`, `constant-dag-churn`, and `environment-stress`, not at scope lookup.

### Targeted Profiles And Counters

Targeted profile artifacts are local-only:

- `just profile-case check_benchmarks_environment-stress_moth terse`:
  `benchmarks/local-data/profiles/2026-06-18T20-12-00-d0b0e10e/`.
- `just profile-case check_benchmarks_adversarial_deep-scope-churn_moth terse`:
  `benchmarks/local-data/profiles/2026-06-18T20-12-08-d0b0e10e/`.
- `just profile-case check_benchmarks_adversarial_one-module-kitchen-sink_moth terse`:
  `benchmarks/local-data/profiles/2026-06-18T20-12-15-d0b0e10e/`.

The profiles captured low sample counts and still reported raw addresses rather than useful
function names. Stage and counter observations were useful:

| Case | Wall | AST | Actual frames | Estimated frames | Arena capacity |
|---|---:|---:|---:|---:|---:|
| `environment-stress` | `~28ms` | `~13ms` | `254` | `165` | `768` |
| `deep-scope-churn` | `~15ms` | `~5ms` | `210` | `108` | `376` |
| `one-module-kitchen-sink` | `~17ms` | `~7ms` | `221` | `107` | `448` |

The estimate-vs-actual gap is the main Phase 5 input. It also confirms that real scope-frame
actuals are now observable instead of staying at zero.

### Audit Notes

- No user-visible shadowing rule changed. Focused tests cover parent lookup, same-frame duplicate
  lookup, ancestor redeclaration detection, visibility-gate inheritance, function child isolation,
  and clone-frame isolation.
- AST still consumes header-built visibility through `FileVisibility`; the refactor did not add
  import rediscovery.
- Scope arena internals remain local to `ast/module_ast/scope_context/`; pipeline and build
  orchestration only record capacity-policy estimates.
- Stage-local diagnostics still use existing `CompilerDiagnostic` paths. Source locations and
  labels remain owned by existing parser/type-resolution call sites.
- No compatibility wrapper for the old flat local-declaration fields remains.
- `Rc<RefCell<ScopeArena>>` is internal to the scope-context subsystem. Lookups return
  `ScopeDeclarationRef` so no borrow guard escapes into recursive parser code.

## Phase 5 Scope Arena Capacity Tuning - 2026-06-18

### Scope

Phase 5 tuned the scope-frame capacity estimate formula, added detailed estimate/actual ratio
counters, added `ScopeArena::with_capacity`, and seeded production AST scope arenas from the
module-level `FrontendArenaCapacityEstimate`. The seeding policy is AST-owned and spends the
module scope-frame estimate once across known root function, start, generic-template-validation,
and const-template parse contexts. Dynamic generic instances and direct AST helper callers remain
unseeded and grow normally.

### Benchmark Results

- Five recorded `just bench-frontend` invocations after production seeding reported no measurable
  regression. The latest run showed `no measurable change: avg -1ms; 16/16 cases`.
- Five recorded `just bench` invocations after production seeding reported no measurable
  regression. The latest run showed `no measurable change: avg 0ms; 25/25 cases`, with small stage
  movement of `ast +7ms`, `file prep +6ms`, and `ast finalize +3ms`.
- The tracked monthly summary was updated by the recorded benchmark runs. Raw benchmark history and
  profile artifacts remain local-only under `benchmarks/local-data/`.

### Targeted Profiles And Counters

Targeted `just profile-case ... terse` runs produced local-only profile directories for docs,
template stress, and import/module fixtures. Stack samples remain mostly unsymbolicated, so the
useful evidence is the observation-pass stage and counter data:

| Case | Wall | AST / Env / Emit | Estimated frames | Actual frames | Arena capacity | Estimate / actual | Capacity / actual |
|---|---:|---:|---:|---:|---:|---:|---:|
| `docs` | `~125ms` | `363 / 107 / 211ms` | `10630` | `4363` | `16590` | `2.44x` | `3.80x` |
| `template-stress` | `~27ms` | `10 / 5 / 4ms` | `467` | `279` | `811` | `1.67x` | `2.91x` |
| `module-graph` | `~15ms` | `4 / 2 / 2ms` | `221` | `129` | `413` | `1.71x` | `3.20x` |
| `import-fanout` | `~19ms` | `5 / 3 / 2ms` | `308` | `173` | `580` | `1.78x` | `3.35x` |
| `external-js-imports` | `~13ms` | `3 / 2 / 1ms` | `144` | `88` | `260` | `1.64x` | `2.95x` |
| `import-external-churn` | `~20ms` | `5 / 2 / 2ms` | `338` | `195` | `591` | `1.73x` | `3.03x` |

No under-estimates were observed in the Phase 5 evidence set. The current formula intentionally
lands on modest over-estimation for normal and adversarial fixtures. Capacity/actual ratios range
from about `2.9x` to `3.8x`, which is acceptable for the current policy because capacity remains
bounded and semantics-neutral, but future tuning should keep an eye on the docs path before
increasing scope-frame estimates further.

### Audit Notes

- Capacity formulas remain centralized in `src/compiler_frontend/arena/capacity.rs`.
- Capacity estimates remain policy-only. If estimates are too small, the arena grows normally; if
  they are too large, the effect is bounded extra `Vec` capacity.
- The Phase 5 evidence does not satisfy the broad Phase 6 entry criteria by itself. The latest
  reports point remaining investigation toward file preparation and docs AST emission rather than
  HIR dense storage or a clear expression scratch hotspot.

## Phase 6/7 Gate Evidence - 2026-06-18

### Scope

An Ollama worker ran the Phase 6/7 gate pass after Phase 5. The worker confirmed `bench-report`
still points at file preparation and docs/type/constant-DAG paths rather than expression or
template arenas, but nested `samply record` failed inside the Ollama/Codex process with
`Unknown(1100)`. Parent-side reruns of the same `profile-case` commands succeeded, so the failure
appears isolated to the nested worker environment rather than the benchmark fixtures.

### Targeted Profiles And Counters

Targeted profile artifacts are local-only:

- `just profile-case check_benchmarks_adversarial_expression-rpn-churn_moth terse`:
  `benchmarks/local-data/profiles/2026-06-18T22-00-34-67a55dd5/`.
- `just profile-case check_benchmarks_adversarial_template-render-plan-churn_moth terse`:
  `benchmarks/local-data/profiles/2026-06-18T22-00-43-67a55dd5/`.
- `just profile-case check_docs terse`:
  `benchmarks/local-data/profiles/2026-06-18T22-00-52-67a55dd5/`.

The profiles still emitted raw-address hotspots, so stage/counter observations remain the useful
signal:

| Case | Wall | AST / Env / Emit / Finalize | HIR | Borrow | Key counters |
|---|---:|---:|---:|---:|---|
| `expression-rpn-churn` | `~16ms` | `5.0 / 2.2 / 2.1 / 0.6ms` | `0.9ms` | `3.3ms` | `template_count=57`, `hir_statement_count=177`, `borrow_state_snapshot_count=493` |
| `template-render-plan-churn` | `~15ms` | `7.0 / 3.6 / 2.4 / 1.0ms` | `0.7ms` | `1.3ms` | `template_count=128`, `runtime_template_count=16`, `hir_statement_count=178` |
| `docs` | `~149ms` | `354.8 / 98.2 / 215.2 / 38.7ms` | `8.3ms` | `4.9ms` | `template_count=4776`, `const_template_count=4771`, `module_remap_string_ids_calls=31` |

### Gate Decision

- Phase 6 remains deferred. The expression churn fixture does not show expression parsing/RPN work
  as a dominant remaining cost, and there are no dedicated expression allocation or clone counters
  showing pressure.
- Phase 7 remains deferred as a broad arena migration. The template churn fixture is small, while
  docs shows meaningful AST emit/finalize work and large template counts. That supports narrower
  docs/template attribution before any render-plan arena conversion.
- Phase 8 remains gated by the same evidence posture: HIR and borrow timings are small in the
  targeted profiles, so dense HIR storage or borrow fact compaction is not the next optimization
  target.

The next optimization evidence should focus on the repeated `bench-report` signal: file
preparation, tokenization/header parsing, string-table merge/remap, and docs AST emit attribution.

## Phase 9 Documentation And Final Decisions - 2026-06-18

- `docs/compiler-design-overview.md` now records frontend arenas as stage/module-owned
  implementation details, with capacity estimates explicitly policy-only.
- `docs/src/docs/progress/@page.moth` tracks "Frontend Arena + Semantic Invariant Optimisation" as
  `Partial`: scope-frame arenas, capacity estimates, external package clone reduction, and
  adversarial fixtures are implemented; deeper expression/template/HIR arenas remain deferred.
- `benchmarks/README.md` records the five independent invocation protocol for optimization phase
  boundaries and keeps raw history/profile rules unchanged.
- Final decision for this optimization pass: keep the implemented scope/external clone work, keep
  the conservative scope capacity formulas, and defer broader arena migrations until a future
  profile shows a specific hotspot.

## Template Optimisation Phase A0 Baseline - 2026-06-19

### Scope

Phase A0 captured the baseline for
`docs/roadmap/plans/template-optimisation-and-tir-implementation-plan.md` before adding new
template churn counters or changing template code.

Baseline branch and commit:

- Branch: `main`
- Commit: `a994e0ec7738295295c0ffb858153615072d7ad5`
- Starting worktree: clean

### Validation And Benchmark Baseline

- `just validate`: passed. This covered clippy, 2686 unit tests, 1707 integration cases, docs
  check, and validation-safe benchmark check.
- Five recorded `just bench-frontend` invocations completed. The latest focused frontend run
  reported `no measurable change: avg 0ms; 16/16 cases`, with `ast +3ms` and
  `ast env +1ms` stage movement.
- Five recorded `just bench` invocations completed. The latest end-to-end run reported
  `no measurable change: avg -1ms; 25/25 cases`, with `ast -16ms`,
  `file prep +12ms`, and `ast emit -10ms` stage movement.
- The tracked monthly summary was updated by the recorded benchmark commands. Raw local history
  and profile artifacts remain local-only under `benchmarks/local-data/`.

### Template-Heavy Baseline Cases

`just bench-report` identified `check_docs` as the slowest end-to-end case. The report still points
at docs AST and file preparation as the largest current signal rather than a single isolated
template fixture.

Latest observed template-heavy end-to-end cases:

| Case | Median | AST | Templates | Const templates | Render plans | Fallback plans |
|---|---:|---:|---:|---:|---:|---:|
| `check_docs` | `~163ms` | `~380ms` | `4788` | `4783` | `~14223` | `~5635` |
| `check_benchmarks_template-stress_bst` | `~36ms` | `~9ms` | `213` | `153` | `439` | `10` |
| `check_benchmarks_adversarial_template-render-plan-churn_bst` | `~11ms` | `~4ms` | `128` | `112` | `237` | `8` |

Targeted `just profile-case check_docs normal` wrote observation artifacts under
`benchmarks/local-data/profiles/2026-06-19T03-17-41-a994e0ec/`, but Samply failed with
`Unknown(1100)`. Because no stack samples were produced, the useful evidence is the observation
pass only: `check_docs` measured about `166ms` wall time with `ast=399ms`,
`ast_build_environment=116ms`, `ast_emit_nodes=228ms`, `ast_finalize=47ms`,
`file_prepare=50ms`, `hir=13ms`, and `borrow=7ms`.

Key `check_docs` counters from the observation pass:

- `template_count=4788`
- `const_template_count=4783`
- `runtime_template_count=5`
- `ast_template_atoms_parsed=10229`
- `ast_template_composition_passes=7083`
- `ast_template_render_plans_built=16181`
- `ast_template_fold_fallback_plan_builds=6846`
- `ast_template_fold_plan_pieces_visited=35664`
- `ast_template_render_pieces_built=44946`
- `ast_templates_folded_during_finalization=1253`

### Decision

Baseline accepted for Phase A1. The next slice should add targeted counters before changing
template behavior or reducing churn, because docs carries the large template count and fallback
plan signal while the dedicated template stress fixtures are much smaller.

## Template Optimisation Phase A1 Counters - 2026-06-19

Baseline: `bc9be0c3` (`A0`).

Change: Phase A1 counter instrumentation slice.

Suites:

- `cargo test instrumentation`
- `cargo test instrumentation --features detailed_timers`
- `cargo test compiler_frontend::ast::templates`
- `cargo test compiler_frontend::ast::templates --features detailed_timers`
- `just bench-frontend-check`
- `cargo run --features detailed_timers -- check benchmarks/adversarial/template-render-plan-churn.moth`
- `just validate`

Phase A1 adds stable AST benchmark counters only. It does not change template semantics, HIR,
backend behavior, diagnostics, or the progress matrix.

### Validation And Benchmark Check

- `just bench-frontend-check`: passed with `+6ms avg`; `0 faster`, `5 slower`, `16/16 cases`.
  Stage movement was `ast +17ms`, `ast emit +7ms`, and `ast env +7ms`.
- `just validate`: passed. Its validation-safe `bench-check` reported
  `no measurable change: avg 0ms; 25/25 cases`, with `ast +15ms`, `file prep -14ms`, and
  `ast emit +7ms`.

The small AST movement is accepted for this instrumentation phase because it adds only no-op
normal-build counter calls plus detailed-timer atomics/byte-counting, and because the full
validation-safe benchmark suite stayed inside the benchmark noise threshold.

### New Counter Baseline

The detailed-timers check on `benchmarks/adversarial/template-render-plan-churn.moth` confirmed all
new stable metric names and produced these baseline values:

| Counter | Value |
|---|---:|
| `ast_template_nested_template_parses` | `76` |
| `ast_template_body_token_visits` | `331` |
| `ast_template_text_bytes_parsed` | `1257` |
| `ast_template_fold_output_bytes` | `2840` |
| `ast_template_fold_string_intern_calls` | `62` |
| `ast_template_fold_expression_clone_requests` | `24` |
| `ast_template_fold_binding_substitutions` | `0` |
| `ast_template_content_clones_for_render_units` | `128` |
| `ast_template_content_rebuilds_after_formatting` | `39` |
| `ast_template_wrapper_vector_clones` | `170` |
| `ast_template_aggregate_plan_builds` | `0` |

Decision: accepted. Phase A2 should use these counters to distinguish capacity and render-unit
clone reductions from timing noise.

## Template Optimisation Phase A2 Capacity Hints - 2026-06-19

Baseline: `ba1a79fd` on `main`.

Change: Phase A2 capacity-threading slice.

Suites:

- `cargo test compiler_frontend::arena`
- `cargo test compiler_frontend::ast::templates`
- `cargo test instrumentation --features detailed_timers`
- `just bench-frontend-check`
- `just validate`
- five recorded `just bench-frontend` invocations
- five recorded `just bench` invocations
- `just bench-report`

Phase A2 adds a narrow `TemplateCapacityPolicy` derived from `FrontendArenaCapacityEstimate`.
Template parsing contexts now pre-size initial `TemplateContent` atom vectors from the average
estimated atoms per estimated template, clamped to `64` atoms per template. Exact local capacities,
such as `TemplateRenderPlan::from_content(content.atoms.len())`, remain unchanged. Aggregate
render-unit helper vectors now use exact local plan lengths instead of starting from `Vec::new()`.

The slice also adds `ast_template_content_estimated_atom_capacity` so detailed benchmark runs can
compare reserved template atom capacity against existing template atom counters without adding a
new traversal.

### Validation And Benchmark Check

- Focused tests passed:
  - `cargo test compiler_frontend::arena`: `18/18`.
  - `cargo test compiler_frontend::ast::templates`: `299/299`.
  - `cargo test instrumentation --features detailed_timers`: `1/1`.
- `just bench-frontend-check`: passed with `no measurable change: avg +1ms; 16/16 cases`.
- `just validate`: passed. Clippy passed on native, Linux, and Windows targets; unit tests passed
  `2688/2688`; integration tests passed `1707/1707`; docs check passed; embedded `bench-check`
  reported `no measurable change: avg +1ms; 25/25 cases`.

### Five-Run Benchmark Results

Recorded frontend run summaries:

- `+3ms avg`; `0 faster`, `2 slower`.
- `+1ms avg`; `0 faster`, `2 slower`.
- `-3ms avg`; `1 faster`, `0 slower`.
- `no measurable change: avg +2ms`.
- `+3ms avg`; `0 faster`, `2 slower`.

The rough five-run frontend median movement was about `+2ms`, which is inside benchmark noise for
this suite. The latest frontend report showed `ast_emit_nodes_ms +4ms`, `ast_ms +3ms`, and
`file_prepare_ms +1ms`.

Recorded end-to-end run summaries:

- `+3ms avg`; `0 faster`, `5 slower`.
- `no measurable change: avg -1ms`.
- `no measurable change: avg +1ms`.
- `no measurable change: avg -1ms`.
- `+2ms avg`; `0 faster`, `1 slower`.

The rough five-run end-to-end median movement was about `+1ms`, also inside benchmark noise. The
latest end-to-end report showed `ast_ms +91ms`, `ast_emit_nodes_ms +61ms`, and
`file_prepare_ms +37ms` spread across many cases. The movement alternated direction across
independent runs, so no targeted profile was taken for this phase.

Decision: accepted. The change is policy-only, validation passed, and five-run timing stayed
neutral enough for the allocation cleanup. No progress-matrix update was needed because template
language support and backend behavior did not change.

## Template Optimisation Phase A3 Fold Output Capacity Hints - 2026-06-19

Baseline: `f76eddaf` on `main`.

Change: Phase A3 fold-output capacity slice.

Suites:

- `cargo test compiler_frontend::ast::templates --lib`
- `cargo test instrumentation --features detailed_timers --lib`
- `just bench-frontend-check`
- `just validate`
- five recorded `just bench-frontend` invocations
- five recorded `just bench` invocations
- `just bench-report`

Phase A3 adds cheap render-plan output byte estimates for already-resolved text pieces and uses
those estimates to pre-size fold output buffers. The estimator counts `RenderPiece::Text` and
`RenderPiece::HeadContent`, uses known aggregate output bytes when folding aggregate wrapper plans,
and deliberately treats dynamic expressions, child templates, slots, loop-control markers, and
runtime slot sites as zero unless their output is already known. The fold path records
`ast_template_estimated_fold_output_bytes` and
`ast_template_fold_output_estimate_miss_bytes`.

Const-loop aggregate reservation is bounded: collection loops use their known const item count,
while streaming numeric range loops cap the reservation hint so the configured loop expansion limit
cannot become a large eager allocation. Formatter output builders were left unchanged because no
clean exact capacity was available without adding noisy formatter plumbing.

### Validation And Benchmark Check

- `cargo test compiler_frontend::ast::templates --lib`: `303/303`.
- `cargo test instrumentation --features detailed_timers --lib`: `1/1`.
- `just bench-frontend-check`: passed with `-4ms avg`; `2 faster`, `0 slower`, `16/16 cases`.
- `just validate`: passed. Clippy passed on native, Linux, and Windows targets; unit tests passed
  `2692/2692`; integration tests passed `1707/1707`; docs check passed; embedded `bench-check`
  reported `-5ms avg`; `10 faster`, `0 slower`, `25/25 cases`.

### Five-Run Benchmark Results

Recorded frontend run summaries:

- `-6ms avg`; `7 faster`, `0 slower`.
- `no measurable change: avg 0ms`.
- `no measurable change: avg -1ms`.
- `no measurable change: avg 0ms`.
- `no measurable change: avg +1ms`.

The rough five-run frontend median movement was about `0ms`, inside benchmark noise. The latest
frontend report showed `no measurable change: avg +1ms; 16/16 cases` with `ast_ms -1ms`.

Recorded end-to-end run summaries:

- `-5ms avg`; `10 faster`, `0 slower`.
- `no measurable change: avg 0ms`.
- `no measurable change: avg 0ms`.
- `no measurable change: avg 0ms`.
- `no measurable change: avg 0ms`.

The rough five-run end-to-end median movement was also about `0ms`, inside benchmark noise. The
latest end-to-end report showed `no measurable change: avg 0ms; 25/25 cases`, with only small
stage movement (`ast_emit_nodes_ms -1ms`, `ast_finalize_ms +1ms`, `dependency_sort_ms +1ms`).

The latest local report showed fold output byte and estimate-miss counters moving down in the
most recent comparison, but the decision is based on neutral five-run timing plus bounded capacity
hints rather than a claimed wall-time win.

Decision: accepted. The change is behavior-preserving, validation passed, and five-run timing
stayed neutral while giving template folding explicit capacity and estimate-miss instrumentation.
No progress-matrix update was needed because template language support and backend behavior did not
change.

## Phase A6 - Parser-Loop Cleanup - 2026-06-19

### Scope

Phase A6 reduced hot-loop overhead in `template_body_parser.rs` by matching token kinds by
reference instead of cloning, and by caching pre-interned `StringId`s for the `"\n"`, `"["`, and
`"]`" literals that appear on every newline and bracket token.

### Files

- `src/compiler_frontend/ast/templates/template_body_parser.rs`

### Validation Status

- `cargo fmt`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --lib compiler_frontend::ast::templates`: passed, `310/310`.
- `cargo test --lib instrumentation --features detailed_timers`: passed.
- `cargo test --quiet`: passed, `2699/2699`.
- `cargo run -- tests`: passed, `1707/1707`.
- `cargo run -- check docs`: passed, no errors or warnings.
- `just validate`: passed.
- `just bench-frontend-check`: passed, `**-4ms avg**; 6 faster, 0 slower; 16/16 cases`.

### Benchmark Results

Five recorded `just bench-frontend` runs:

- `**-4ms avg**; 6 faster, 0 slower`
- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`

The first focused-frontend run shows a measurable `-4ms` improvement; the remaining four runs are
inside benchmark noise. The `just bench-frontend-check` result also reported `**-4ms avg**`.

End-to-end `just validate` benchmark check reported `**-6ms avg**; 10 faster, 0 slower; 25/25 cases`
with stage movement `ast -14ms`, `ast emit -14ms`, `file prep +6ms`.

### Decision

Accepted. The change is behaviour-preserving, validation passed, and the consistent small
improvements in the first focused-frontend run and validation-safe checks justify the low-risk
borrow-reference and cached-intern cleanup. No new counters were added because the existing
`TemplateBodyTokenVisits` and `TemplateTextBytesParsed` counters already cover hot-loop volume.

No progress-matrix update was needed because template language support and backend behavior did not
change.

## Phase A5 - Render-Unit Rebuild and Clone Reduction - 2026-06-19

### Scope

Phase A5 reduced avoidable `TemplateContent` cloning and fallback render-plan builds in
`template_render_units.rs`. Control-flow branch, fallback, and loop body content are now moved
through `prepare_template_render_unit` rather than cloned, and aggregate piece preparation reuses
an existing authoritative render plan when one is available.

### Files

- `src/compiler_frontend/ast/templates/template_render_units.rs`
- `src/compiler_frontend/ast/templates/template.rs`

### Validation Status

- `cargo fmt`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --lib compiler_frontend::ast::templates`: passed, `310/310`.
- `cargo test --lib instrumentation --features detailed_timers`: passed.
- `cargo test --quiet`: passed, `2699/2699`.
- `cargo run -- tests`: passed, `1707/1707`.
- `cargo run -- check docs`: passed, no errors or warnings.
- `just validate`: passed.
- `just bench-frontend-check`: passed, `**-3ms avg**; 2 faster, 0 slower; 16/16 cases`.

### Benchmark Results

Five recorded `just bench-frontend` runs:

- `**-3ms avg**; 3 faster, 0 slower`
- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`

The five-run median is inside benchmark noise. The latest `just bench-report` comparison for the
focused frontend suite showed `no measurable change: avg 0ms; 16/16 cases`, with only
`borrow_ms -1ms` as a non-noise stage movement.

Counter movement in the latest focused-frontend comparison:

- `ast_template_fold_output_bytes +25%`
- `ast_template_estimated_fold_output_bytes +25%`
- `ast_template_fold_output_estimate_miss_bytes +24%`

These output-byte counters move with normal fixture variance and are not driven by the render-unit
changes.

End-to-end `just validate` benchmark check reported `**-4ms avg**; 9 faster, 0 slower; 25/25 cases`.

The adversarial `template-render-plan-churn.moth` fixture still reports
`ast_template_content_clones_for_render_units=128`, which is expected because that fixture does not
exercise control-flow content cloning.

### Decision

Accepted. The change is behaviour-preserving, validation passed, focused timing stayed neutral, and
the clone-reduction paths remove obvious duplicated work in control-flow render-unit preparation.
Wrapper-vector clones were intentionally deferred to the TIR migration because replacing them cleanly
requires wrapper-set IDs.

No progress-matrix update was needed because template language support and backend behavior did not
change.

## Phase A4 - Borrow-First Fold Binding Resolution - 2026-06-19

### Scope

Phase A4 reduced fold-time expression cloning in `template_folding.rs` by introducing a
borrow-first resolver. The common case where a template expression contains no foldable bindings
now returns a borrowed reference instead of cloning the entire expression tree.

### Files

- `src/compiler_frontend/ast/templates/template_folding.rs`
- `src/compiler_frontend/ast/templates/template_folding_tests.rs` (new)
- `src/compiler_frontend/instrumentation/ast_counters.rs`
- `src/compiler_frontend/instrumentation/tests.rs`
- `src/compiler_frontend/ast/templates/mod.rs`

### Validation Status

- `cargo fmt`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --lib compiler_frontend::ast::templates`: passed, `310/310`.
- `cargo test --lib instrumentation --features detailed_timers`: passed.
- `cargo test --quiet`: passed, `2699/2699`.
- `cargo run -- tests`: passed, `1707/1707`.
- `cargo run -- check docs`: passed, no errors or warnings.
- `just validate`: passed.
- `just bench-frontend-check`: passed, `mixed: avg 0ms; 1 faster, 5 slower; 16/16 cases`.

### Benchmark Results

Five recorded `just bench-frontend` runs:

- `mixed: avg +1ms; 2 faster, 9 slower`
- `no measurable change: avg -1ms`
- `**+2ms avg**; 0 faster, 2 slower`
- `**-2ms avg**; 1 faster, 0 slower`
- `**0ms avg**; 0 faster, 1 slower`

The rough five-run median movement is inside benchmark noise. The latest `just bench-report`
comparison for the focused frontend suite showed `0ms avg; 0 faster, 1 slower; 16/16 cases`, with
small stage movements (`ast_ms +6ms`, `ast_emit_nodes_ms +2ms`, `borrow_ms +2ms`,
`hir_ms +2ms`, `ast_build_environment_ms +2ms`).

Counter movement in the latest focused-frontend comparison:

- `ast_template_fold_output_estimate_miss_bytes +30%`
- `ast_template_fold_output_bytes +27%`
- `ast_template_estimated_fold_output_bytes +24%`

These output-byte counters move with normal fixture variance; they are not driven by the resolver
change. The new `ast_template_fold_expression_owned_rewrites` counter reads `0` on the
`template-render-plan-churn.moth` fixture, which exercises render-plan churn rather than binding
substitution.

### Decision

Accepted. The change is behavior-preserving, validation passed, focused timing stayed neutral,
and the borrow-first path gives the intended clone-reduction semantics with a new counter to
measure actual rewrites against clone requests. Tests were moved to a separate file to follow the
project style guide.

No progress-matrix update was needed because template language support and backend behavior did
not change.

## Phase B2 - TIR-Native Folding Route - 2026-06-20

### Scope

Phase B2 routes non-formatting compile-time template folding through the AST-local TIR path.
Formatter-dependent templates still use the legacy render-plan fold path until the planned TIR
formatter view lands in Phase B3, and aggregate wrapper handling keeps a narrow temporary bridge
until Phase B4 owns TIR render units.

### Files

- `src/compiler_frontend/ast/templates/tir/fold.rs`
- `src/compiler_frontend/ast/templates/tir/convert_from_template.rs`
- `src/compiler_frontend/ast/templates/tir/node.rs`
- `src/compiler_frontend/ast/templates/template_folding.rs`
- `src/compiler_frontend/instrumentation/ast_counters.rs`
- `src/compiler_frontend/ast/templates/tir/tests/fold_parity_tests.rs`

### Validation Status

- `cargo fmt`: passed in the implementation slice.
- `cargo test --lib compiler_frontend::ast::templates`: passed in the implementation slice.
- `cargo test --lib instrumentation --features detailed_timers`: passed in the implementation slice.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed in the implementation slice.
- `just validate`: passed after measurement. Clippy passed on native, Linux, and Windows targets;
  unit tests passed `2747/2747`; integration tests passed `1707/1707`; docs check passed; embedded
  `bench-check` reported `-5ms avg`; `10 faster`, `0 slower`, `25/25 cases`.

### Benchmark Results

Baseline: parent before the TIR folding route commit, `71aef350`.
Change: `6ba5104e` (`TIR - template folding IR p1`).
Suites: five recorded `just bench-frontend` runs, five recorded `just bench` runs, and
`just bench-report`.

Recorded frontend run summaries:

- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`
- `no measurable change: avg -1ms`
- `no measurable change: avg 0ms`

Recorded end-to-end run summaries:

- `-5ms avg`; `10 faster`, `0 slower`
- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`

The five-run frontend and end-to-end medians are both inside benchmark noise. Latest
`just bench-report` results:

- Frontend phases: `no measurable change: avg 0ms; 16/16 cases`, with no stage or counter
  movement.
- End-to-end CLI: `no measurable change: avg 0ms; 25/25 cases`, with `file_prepare_ms -6ms`,
  `ast_emit_nodes_ms -5ms`, and `ast_build_environment_ms +4ms`.
- End-to-end counter movement was broad run-to-run noise:
  `ast_visible_source_type_lookup_attempts -9%`,
  `ast_public_surface_validation_checks -9%`, and
  `ast_template_wrapper_applications -9%`.

Relevant template cases show the intended routing:

- `check_benchmarks_template-stress_bst`: old fold-plan fallback counters stayed at zero
  (`ast_template_fold_plan_pieces_visited=0`,
  `ast_template_fold_fallback_plan_builds=0`,
  `ast_template_fold_expression_clone_requests=0`); TIR folding recorded
  `ast_tir_fold_templates_folded=91`, `ast_tir_fold_nodes_visited=341`, and
  `ast_tir_fold_output_bytes=8299`.
- `check_benchmarks_adversarial_template-render-plan-churn_bst`: old fold-plan fallback counters
  stayed at zero; TIR folding recorded `ast_tir_fold_templates_folded=62`,
  `ast_tir_fold_nodes_visited=240`, and `ast_tir_fold_output_bytes=2840`.
- `check_benchmarks_adversarial_constant-dag-churn_bst`: old fold-plan fallback counters stayed at
  zero; TIR folding recorded `ast_tir_fold_templates_folded=50`,
  `ast_tir_fold_nodes_visited=185`, and `ast_tir_fold_output_bytes=1157`.

No targeted profile was run. The five-run suite medians are neutral, the latest frontend report
has no stage or counter movement, and the remaining end-to-end movement does not point at a
template-specific regression.

### Decision

Accepted. Phase B2 preserves template semantics, validation passed, five-run timing is neutral,
and the measured template cases show the old render-plan fallback counters are no longer active on
the non-formatting fold route while TIR fold counters record the production work.

No progress-matrix update was needed because template language support and backend behavior did
not change.

## Phase B3 - TIR-Native Formatter View - 2026-06-20

### Scope

Phase B3 routes formatter-dependent compile-time template folding through a TIR-native formatter
view. Existing formatter algorithms stay unchanged; the adapter exposes TIR body text, dynamic
expression anchors, and opaque child-template anchors as `FormatterInput`, then maps
`FormatterOutput` directly back to TIR nodes.

### Files

- `src/compiler_frontend/ast/templates/tir/formatter_view.rs`
- `src/compiler_frontend/ast/templates/tir/tests/formatter_parity_tests.rs`
- `src/compiler_frontend/ast/templates/tir/convert_from_template.rs`
- `src/compiler_frontend/ast/templates/template_folding.rs`
- `src/compiler_frontend/ast/templates/template_formatting.rs`
- `src/compiler_frontend/instrumentation/ast_counters.rs`

### Validation Status

- `cargo fmt`: passed.
- `cargo test --lib compiler_frontend::ast::templates`: passed, 368 template tests.
- `cargo test --lib instrumentation --features detailed_timers`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- Worker validation before parent corrections also passed `cargo test --quiet`, `cargo run --quiet -- tests`, `cargo run --quiet -- check docs`, and `just validate`.

### Benchmark Results

Baseline: `7799a61f` (Phase B2 measurement closure).
Change: Phase B3 TIR formatter-view slice.
Suites: five recorded `just bench-frontend` runs, five recorded `just bench` runs,
`just bench-report`, and one targeted `just profile-case check_docs normal`.

Recorded frontend run summaries:

- `no measurable change: avg +1ms`
- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`

Recorded end-to-end run summaries:

- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`

Latest `just bench-report` results:

- End-to-end CLI: `no measurable change: avg 0ms; 25/25 cases`.
- Frontend phases: `no measurable change: avg 0ms; 16/16 cases`.
- Counter movement was limited to TIR conversion/fold reductions:
  `ast_tir_fold_output_bytes -14%`,
  `ast_tir_converter_templates_converted -14%`,
  `ast_tir_templates_created -8%`,
  and `ast_tir_nodes_created -7%`.

The stable `ast_template_fold_fallback_plan_builds` metric remains emitted at zero so future
reports can keep showing the legacy render-plan fallback path is inactive. The targeted
`check_docs` profile completed (`check_docs ~130ms`, Samply ~1556ms), but symbolication reported
`failed_raw_addresses`; no function-level attribution is claimed.

### Decision

Accepted. Phase B3 preserves formatter semantics, keeps `$md` child-template opacity and
dynamic-expression anchor behavior intact, adds TIR formatter parity coverage, and keeps five-run
frontend and end-to-end medians neutral. No progress-matrix update was needed because template
language support and backend behavior did not change.

## Phase B5 - HIR Runtime Metadata from TIR - 2026-06-21

### Scope

Phase B5 moved HIR runtime slot lowering onto the AST-owned runtime-template handoff materialized
from TIR. The handoff keeps TIR IDs inside AST internals while preserving runtime slot source/site
plans, repeated slot replay, control-flow runtime template nodes, aggregate-output markers, and
reactive metadata copied after final template annotation.

### Validation Status

- `just validate`: passed. Unit tests passed `2803/2803`; integration tests passed `1707/1707`;
  docs check passed; the embedded validation-safe benchmark check reported `+9ms avg` with
  AST-stage movement.
- Five recorded `just bench-frontend` runs completed.
- Five recorded `just bench` runs completed.
- `just bench-report` completed after the recorded runs.

### Benchmark Results

Baseline/change commit: `1b18223f` (`HIR runtime slot lowering now consumes
OwnedRuntimeSlotApplicationHandoff`).

Recorded frontend run summaries:

- `+10ms avg`; `0 faster`, `5 slower`
- `no measurable change: avg 0ms`
- `-3ms avg`; `1 faster`, `0 slower`
- `+2ms avg`; `0 faster`, `1 slower`
- `mixed: avg -1ms`; `1 faster`, `1 slower`

Recorded end-to-end run summaries:

- `+6ms avg`; `0 faster`, `11 slower`
- `no measurable change: avg 0ms`
- `+1ms avg`; `0 faster`, `1 slower`
- `no measurable change: avg 0ms`
- `no measurable change: avg +4ms`

Latest `just bench-report` results:

- Frontend phases: `mixed: avg -1ms; 1 faster, 1 slower; 16/16 cases`, with small stage movement
  (`ast_build_environment_ms +2ms`, `ast_ms +2ms`, `borrow_ms +1ms`).
- End-to-end CLI: `no measurable change: avg +4ms; 25/25 cases`.
- End-to-end attribution showed AST/file-prep movement (`ast_ms +268ms`,
  `ast_emit_nodes_ms +156ms`, `ast_build_environment_ms +82ms`, `file_prepare_ms +67ms`) and no
  backend-stage movement.
- Counter movement was broad run-to-run template/finalization volume noise:
  `ast_templates_folded_during_finalization +17%`,
  `ast_module_constant_normalization_expressions_visited +17%`, and
  `ast_tir_fold_nodes_visited +13%`.

### Decision

Accepted. Phase B5 keeps HIR free of TIR IDs and formatter/directive/slot-schema parsing,
validation passed, five-run frontend and end-to-end medians are neutral, and no backend-time
regression appeared. The narrow legacy runtime-slot-plan adapter remains intentionally temporary
for B6/B7 deletion work. No progress-matrix update was needed because template language support and
backend behavior did not change.

## Phase B5 Steering Checkpoint - Boundary and Bridge Counters - 2026-06-21

### Scope

This steering slice corrected the AST/HIR handoff boundary name, made B5 bridge work visible with
narrow detailed-timer counters, and marked the legacy handoff adapters for B6/B7 deletion. Behavior
was intentionally unchanged.

### Validation Status

- `just validate`: passed. Unit tests passed `2810/2810`; integration tests passed `1707/1707`;
  docs check passed; the embedded validation-safe benchmark check reported `-5ms avg`.
- Five recorded `just bench-frontend` runs completed.
- Five recorded `just bench` runs completed.
- `just bench-report` completed after the recorded runs.

### Benchmark Results

Recorded frontend run summaries:

- `mixed: avg -1ms`; `1 faster`, `1 slower`
- `no measurable change: avg -1ms`
- `no measurable change: avg 0ms`
- `no measurable change: avg -1ms`
- `no measurable change: avg +1ms`

Recorded end-to-end run summaries:

- `-5ms avg`; `2 faster`, `0 slower`
- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`
- `no measurable change: avg 0ms`
- `0ms avg`; `2 faster`, `0 slower`

Latest `just bench-report` results:

- Frontend phases: `no measurable change: avg +1ms; 16/16 cases`, with small stage movement
  (`ast_ms +3ms`, `hir_ms +2ms`, `ast_build_environment_ms +1ms`).
- End-to-end CLI: `0ms avg; 2 faster, 0 slower; 25/25 cases`, with small stage movement
  (`ast_emit_nodes_ms -5ms`, `ast_ms -4ms`, `file_prepare_ms -4ms`).
- No backend-stage movement was reported.

B5 bridge counter snapshot:

- Frontend: `RuntimeSlotHandoffsMaterialized=3`, `RuntimeSlotHandoffTemplateClones=3`,
  `RuntimeSlotHandoffFreshTirStores=3`, `RuntimeSlotHandoffOwnedNodesMaterialized=48`,
  `RuntimeSlotHandoffLegacyAdapterCalls=0`.
- End-to-end: `RuntimeSlotHandoffsMaterialized=4`, `RuntimeSlotHandoffTemplateClones=4`,
  `RuntimeSlotHandoffFreshTirStores=4`, `RuntimeSlotHandoffOwnedNodesMaterialized=78`,
  `RuntimeSlotHandoffLegacyAdapterCalls=0`.
- Normal docs/template benchmark workloads did not call the temporary legacy adapter.

### Decision

Accepted. HIR now imports owned runtime handoff data through a neutral AST-template boundary, the
temporary B5 bridge costs are measurable, and the five-run checkpoint is neutral/bounded. B6 may
continue, but B6/B7 must reduce or delete the bridge counters and remove the legacy handoff
adapters rather than leaving a permanent dual-template system.

## Phase B6 - Direct Parser-To-TIR Emission - 2026-06-21

### Scope

Phase B6 made the template parser emit TIR nodes directly alongside the temporary legacy
`TemplateContent` path. The phase added a module-owned parser TIR store, parser draft emission for
body text, nested child templates, template control flow, slots/inserts, head output segments,
same-store head template references, wrapper metadata, diagnostic parity coverage, and B7 deletion
checkpoints for remaining old-path bridges.

### Validation Status

- `just validate`: passed after the B6 diagnostic and deletion-checkpoint slices. Unit tests passed
  `2849/2849`; integration tests passed `1707/1707`; docs check passed.
- Five recorded `just bench-frontend` runs completed.
- Five recorded `just bench` runs completed.
- `just bench-report` completed after the recorded runs.

### Benchmark Results

Recorded frontend run summaries:

- `mixed: avg +113ms`; `4 faster`, `2 slower`
- `-2ms avg`; `1 faster`, `0 slower`
- `no measurable change: avg -4ms`
- `+9ms avg`; `0 faster`, `1 slower`
- `no measurable change: avg -8ms`

Recorded end-to-end run summaries:

- `mixed: avg +39ms`; `8 faster`, `1 slower`
- `no measurable change: avg -1ms`
- `0ms avg`; `1 faster`, `0 slower`
- `+4ms avg`; `0 faster`, `1 slower`
- `mixed: avg -1ms`; `1 faster`, `1 slower`

Latest `just bench-report` results:

- Frontend phases: `no measurable change: avg -8ms; 16/16 cases`.
- End-to-end CLI: `mixed: avg -1ms; 1 faster, 1 slower; 25/25 cases`.
- The first frontend and end-to-end runs were noisy outliers. Later runs and the latest report are
  effectively neutral.

### Decision

Accepted as semantic-parity migration progress, not as a measured speedup. B6 leaves the remaining
legacy `TemplateContent` / `TemplateRenderPlan` authority paths intentionally marked for B7
deletion. Phase B7 should now move the smallest production authority surface onto TIR and remove
old-path work instead of broadening the dual representation.

## Phase B7 Final TIR Evidence Summary - 2026-06-23

### Scope

Phase B7 made TIR the primary internal representation for synced templates through incremental
widening slices (B7a through B7ac). The phase added parser-TIR finalized references, string-ID
remap through TIR, HIR legacy fallback removal, render-unit output sync, formatter attribution,
thread-local counters, control-flow body sync, and sync miss attribution counters.

### TIR Coverage

Profiled on `check_docs` with `detailed_timers`:

- Parser-TIR fold: 375 candidates, 159 hits (42.4%), 216 fallbacks
  - 186 `unsafe_has_formatter` (templates whose TIR still has `has_formatter` because sync failed)
  - 30 `ast_content_root_mismatch` (content modified after TIR build)
  - All other fold fallback reasons: 0
- Parser-TIR sync: 4788 attempts, 3105 successes (64.9%), 1683 skips
  - 944 `unresolved_slots` (templates with `$slot` atoms not yet resolved by composition)
  - 739 `child_template_missing_cross_store_proof` (child templates without same-store TIR reference)
  - All other sync skip reasons: 0

### Remaining Blockers

The sync surface cannot be widened further without addressing:

1. **Unresolved slots** (944 skips): templates with `$slot(name)` atoms cannot be synced via the
   simple finalized path because TIR's `build_finalized_simple_tir_root` only handles `Content`
   atoms. These templates need post-composition sync or TIR slot-definition representation.
2. **Cross-store proof** (739 skips): child templates from different modules or unsynced children
   cannot be proven same-store. These need cross-store TIR references or pre-composition sync.
3. **Formatter flag persistence** (186 fold fallbacks): templates that failed sync for the above
   reasons keep the parser draft's `has_formatter` flag, blocking TIR-based folding.

These are deferred to follow-up work (see `docs/roadmap/roadmap.md`).

### Validation Status

- `just validate` passed after the final B7ac slice: clippy, 2897 unit tests, 1707/1707 integration
  cases, docs check, and `bench-check` (`-20ms avg`; 25/25 cases).
- `cargo test --lib compiler_frontend::ast::templates` (496+ passed across slices).
- `cargo test --lib compiler_frontend::instrumentation::tests --features detailed_timers` (2 passed).

### Benchmark Results

`bench-check` at the final B7ac commit: `-20ms avg`; 20 faster, 0 slower; 25/25 cases.
Stage movement: `ast -2052ms`, `ast emit -1934ms`, `ast env -90ms`.

### Decision

Intermediate parser-TIR primary-path checkpoint for B7 widening. TIR is the primary internal
representation for synced templates (65% sync coverage, 42% fold coverage). Legacy
`TemplateContent` and `TemplateRenderPlan` paths remain as narrow fallbacks for templates with
unresolved slots or cross-store children. Full removal of legacy paths is now active work under
the TIR final-authority plan, which implements post-composition sync and cross-store child
materialization.

The B7 structural deletion items (replacing `Template.content` authority, removing
`TemplateContent`/`TemplateRenderPlan`/`unformatted_content`/`render_plan` fields) remain open
because they depend on the sync surface being wider. These are tracked in the plan and deferred
to follow-up work.

## TIR Finalisation Plan F0 Baseline - 2026-06-24

### Scope

F0 baseline freeze for `docs/roadmap/plans/tir-final-authority-implementation-plan.md` on the
`templates-refactor` branch. This entry records the starting counter state before Phase F2
implementation work begins.

### Baseline

- Branch: `templates-refactor`
- Commit: `44babbf6` (`next TIR work`)
- Starting worktree: clean
- `just validate`: confirmed by prior F0 checkpoint

### B7ac Counter Confirmation

`cargo run --features detailed_timers -- check docs` confirmed all B7ac parser-TIR sync and fold
counters are visible. Representative values from the largest docs module batch:

| Counter | Value |
|---|---:|
| `ast_template_parser_tir_sync_attempts` | `749` |
| `ast_template_parser_tir_sync_successes` | `597` (`79.7%`) |
| `ast_template_parser_tir_sync_skipped_unresolved_slots` | `30` |
| `ast_template_parser_tir_sync_skipped_child_template_missing_cross_store_proof` | `122` |
| `ast_template_parser_tir_fold_candidates` | `19` |
| `ast_template_parser_tir_fold_hits` | `8` (`42.1%`) |
| `ast_template_parser_tir_fold_fallbacks` | `11` |
| `ast_template_parser_tir_fold_fallback_unsafe_has_formatter` | `10` |
| `ast_template_parser_tir_fold_fallback_ast_content_root_mismatch` | `1` |
| `ast_tir_templates_created` | `22683` |
| `ast_tir_nodes_created` | `67650` |
| `ast_tir_fold_templates_folded` | `1988` |

Legacy counters still active in production:

| Counter | Value |
|---|---:|
| `ast_template_render_plans_built` | `892` |
| `ast_template_content_clones_for_render_units` | `749` |
| `ast_template_content_rebuilds_after_formatting` | `141` |
| `ast_template_wrapper_vector_clones` | `789` |
| `ast_template_fold_fallback_plan_builds` | `0` |
| `ast_runtime_render_plans_rebuilt` | `2` |

### Known Blockers

1. **Unresolved slots** (30 skips): templates with `$slot` atoms not resolved by composition.
2. **Cross-store proof** (122 skips): child templates without same-store TIR reference.
3. **Formatter flag persistence** (10 fold fallbacks): templates that failed sync keep
   `has_formatter`, blocking TIR-based folding.

### F1 Cleanup Note

F1 deleted `Ast::remap_template_ir_store_string_ids` (dead code; TIR store is always consumed
before the module-wide StringId remap boundary). `TemplateIrStore::remap_string_ids` remains as a
store-level capability for tests. The `template_ir_store` field on `Ast` carries
`#[allow(dead_code)]` until Phases F2-F8 wire production TIR consumers. No behavior change.

## Constant Evaluation And Type-System Plan - Phase 0 Baseline - 2026-08-22

Baseline freeze for
`docs/roadmap/plans/constant-folding-and-type-system-hot-path-optimization-plan.md` on the
`const-folding-and-types-optimisation` branch. Evidence only: no semantic representation or
control-flow change.

### Baseline Environment

- Commit: `3ba28c5fb907d2ee44e69a58c59d002aa6a2b384` (`prep for const optimisation`)
- Branch: `const-folding-and-types-optimisation` (identical to `main` at the time of measurement)
- Machine: macOS Apple Silicon benchmark host `6D851D`, Darwin `23.6.0` ARM64
- Timing schema: `2`. The prerequisite module-attributed constant timers already exist:
  `frontend.ast.environment.constant_header_resolution`,
  `frontend.ast.emit.const_template_parse`, `frontend.ast.emit.const_template_fold` and
  `frontend.ast.finalise.module_constant`. No rebase onto a newer timing schema was required.

### Commands Run

- `just bench-frontend` five recorded invocations, default thread identity
- `just bench` five recorded invocations, default thread identity
- `RAYON_NUM_THREADS=1 just bench-frontend` five recorded invocations
- `just bench-validate` after adding the four new workloads
- counter capture with a `timers,benchmark_counters` release binary and `MOTH_COUNTERS=summary`

The tracked monthly summary was restored between recorded runs so each invocation started from a
clean committed worktree. The raw per-case medians below come from
`benchmarks/local-data/runs.jsonl`.

### Suite Medians Across Five Runs

| Suite | Thread identity | Median suite average |
|---|---|---:|
| `frontend_phases` | default | `85.15ms` |
| `frontend_phases` | fixed 1 | `84.89ms` |
| `end_to_end_cli` | default | `24.49ms` |

Single-thread and default-thread frontend medians agree within noise on the constant-heavy cases,
so constant setup cost is not a scheduling artefact.

### Per-Case Medians (median of five run medians)

| Case | frontend wall | CLI wall |
|---|---:|---:|
| `docs` | `1540.55ms` | `286.07ms` |
| `type_stress` | `59.03ms` | `21.03ms` |
| `fold_stress` | `56.04ms` | `17.99ms` |
| `environment_stress` | `44.92ms` | `17.20ms` |
| `one_module_kitchen_sink` | `36.92ms` | `12.47ms` |
| `expression_rpn_churn` | `34.14ms` | `11.68ms` |
| `constant_dag_churn` | `29.45ms` | `10.86ms` |

### Module-Attributed Constant Timings (frontend suite, median ms)

| Case | `ast.total` | `constant_header_resolution` | `const_template_parse` | `const_template_fold` | `finalise.module_constant` |
|---|---:|---:|---:|---:|---:|
| `docs` | `1143.87` | `566.28` | `317.16` | `44.17` | `104.37` |
| `fold_stress` | `36.57` | `17.04` | `0.00` | `0.00` | `0.97` |
| `type_stress` | `33.55` | `8.53` | `0.00` | `0.00` | `0.73` |
| `environment_stress` | `28.16` | `7.30` | `0.00` | `0.00` | `0.73` |
| `constant_dag_churn` | `17.07` | `9.70` | `0.00` | `0.00` | `0.64` |
| `one_module_kitchen_sink` | `10.97` | `1.70` | `0.00` | `0.00` | `0.43` |
| `expression_rpn_churn` | `5.05` | `0.16` | `0.00` | `0.00` | `0.22` |

Constant-header resolution is about half of `frontend.ast.total` on `docs`, `fold_stress` and
`constant_dag_churn`. That confirms the plan's priority ordering without relying on the deleted
legacy detailed channel.

### New Scaling Fixtures

Four workloads and eight cases were added (`37` workloads, `68` cases total):

- `benchmarks/constant-scaling/constant-chain-32.moth`
- `benchmarks/constant-scaling/constant-chain-128.moth`
- `benchmarks/constant-scaling/constant-chain-512.moth`
- `benchmarks/nominal-capacity-stress.moth`

The chains repeat one tiny initializer (`c<i> #= c<i-1> + 1`) so constant count is the only
variable. The nominal fixture holds constant count at four while driving `240` fixed-capacity
struct fields and `36` capacity-dependent choice payload variants, separating member-shell cost
from constant cost.

`moth check` wall time on the chains, single measurement each: `32` -> `~35ms`, `128` -> `~68ms`,
`512` -> `~378ms`. Against a `~28ms` fixed floor that is roughly `7ms`, `40ms` and `350ms` of
constant work for a `4x` and `4x` growth in constant count: clearly superlinear.

### Counter Baseline

New counters added in this phase are listed with their baseline values. `ast_declaration_table_replacements`
was renamed `ast_declaration_replacements_by_path`; a by-ID replacement path does not exist yet and
arrives with the dense declaration-identity phase.

| Counter | `chain_32` | `chain_128` | `chain_512` | `nominal_capacity` | `constant_dag_churn` | `fold_stress` | `docs` |
|---|---:|---:|---:|---:|---:|---:|---:|
| `ast_constants_resolved` | `32` | `128` | `512` | `4` | `88` | `120` | `545` |
| `ast_constant_resolution_contexts_created` | `32` | `128` | `512` | `4` | `88` | `120` | `545` |
| `ast_constant_pass_prior_constant_ids_copied` | `496` | `8128` | `130816` | `310` | `4092` | `7860` | `17337` |
| `ast_constant_pass_visibility_entries_cloned` | `2208` | `33408` | `526848` | `468` | `16456` | `30840` | `24816` |
| `ast_constant_pass_side_table_entries_cloned` | `2` | `2` | `2` | `106` | `6` | `10` | `150` |
| `ast_module_constant_declaration_clones` | `32` | `128` | `512` | `4` | `88` | `120` | `545` |
| `ast_scope_contexts_created` | `65` | `257` | `1025` | `161` | `197` | `299` | `1904` |
| `ast_expression_ordering_input_items` | `95` | `383` | `1535` | `8` | `459` | `1120` | `1322` |
| `ast_expression_typed_stack_items` | `93` | `381` | `1533` | `0` | `392` | `960` | `0` |
| `ast_expression_fold_items` | `93` | `381` | `1533` | `0` | `392` | `960` | `0` |
| `ast_expression_operand_clones` | `93` | `381` | `1533` | `0` | `334` | `780` | `0` |
| `ast_diagnostic_data_type_materialisations` | `93` | `381` | `1533` | `0` | `392` | `960` | `0` |
| `ast_declaration_replacements_by_path` | `33` | `129` | `513` | `69` | `91` | `125` | `620` |
| `ast_module_constant_normalization_expressions_visited` | `32` | `128` | `512` | `4` | `108` | `142` | `2089` |
| `hir_const_value_conversions` | `32` | `128` | `512` | `4` | `108` | `142` | `2076` |
| `public_folded_value_conversions` | `0` | `0` | `0` | `0` | `0` | `0` | `6` |
| `ast_branch_local_generic_requests` | `0` | `0` | `0` | `0` | `0` | `0` | `0` |
| `hir_static_bool_if_nodes` | `0` | `0` | `0` | `0` | `0` | `0` | `0` |
| `hir_runtime_if_nodes` | `0` | `0` | `0` | `0` | `0` | `0` | `0` |
| `generic_substitution_key_builds` | `0` | `0` | `0` | `0` | `0` | `0` | `0` |

`ast_constant_pass_prior_constant_ids_copied` is exactly `C * (C - 1) / 2` on the chains. The
cumulative prior-constant copy and the per-constant `FileVisibility` clone are the two quadratic
terms the dense-identity and session phases must drive to zero.

`ast_expression_operand_clones` equals `ast_expression_fold_items` on the chains: every folded
operand is a full `Expression` clone today.

`generic_trait_churn` is the workload that will move the substitution-key counters; the constant
fixtures build no substitution keys at all.

### Static Control-Flow Freeze

Four integration cases freeze the current accepted behaviour that specialisation must preserve:

- `static_if_constant_bool_branch_selection`
- `static_if_value_producing_branch_selection`
- `static_if_branch_scope_preserved`
- `static_if_inactive_branch_generic_call`

`function_partial_if_return_rejected` already froze the current terminality rejection for
`if true: return 1 ;`, and `dynamic_if_test` already owns runtime-condition execution, so neither
was duplicated.

Three ignored intended-contract tests in `src/compiler_frontend/tests/frontend_pipeline_tests.rs`
record the accepted behaviour the current compiler does not implement. All three fail today for the
right reasons:

- `intended_compile_time_true_condition_reaches_hir_without_a_branch`: `1` branch terminator, `0`
  expected
- `intended_compile_time_false_condition_without_else_lowers_no_branch_body`: `1` branch
  terminator, `0` expected
- `intended_terminality_observes_the_selected_branch`:
  `InvalidReturnShape { reason: FunctionMayFallThrough }`

`runtime_bool_condition_lowers_one_branch_diamond` is the matching non-ignored freeze.

### Findings That Change Later Phases

1. **A constant-backed Bool condition is not a folded `Bool` at HIR.** `if enabled:` with
   `enabled #= true` reaches HIR as a reference, not `ExpressionKind::Bool`; only a literal
   `if true:` folds. The static-if owner must read the condition through the folded-value
   authority rather than pattern-matching the expression kind, otherwise it will only specialise
   literal conditions. `hir_static_bool_if_nodes` therefore counts the literal case only, and its
   baseline is `0` on every fixture except hand-written probes.
2. **Inactive generic work is materialised today.** A generic call reachable only through a
   compile-time-false branch still emits a generated function (`__moth_generated_fn_1` in the
   `static_if_inactive_branch_generic_call` artefact). The pruning mechanism already exists:
   `ScopeContext::generic_request_checkpoint` / `discard_generic_requests_since`, used by static
   `assert(true)` message discarding in `src/compiler_frontend/ast/statements/asserts.rs`. The
   static-if owner should reuse it rather than inventing a second boundary.
3. **`ast_constant_pass_side_table_entries_cloned` is small but wrongly shaped.** The five side
   tables are cloned once per module, so the count reflects table size, not constant count. The
   per-constant cost lives in `ast_constant_pass_visibility_entries_cloned` instead.
4. **Struct and choice member shells are rebuilt after constants** in
   `AstModuleEnvironmentBuilder::resolve_type_declarations`, which is the cost the nominal
   capacity fixture isolates.

## Constant Evaluation And Type-System Plan - Phase 1 Consolidated Constant Session - 2026-08-22

Phase 1 replaced the per-constant context construction in
`src/compiler_frontend/ast/module_ast/environment/constant_resolution.rs` with one module-owned
`ConstantResolutionSession`.

### Measurement Protocol

Wall times are the compiler-reported `Done in` figure from `moth check`, three consecutive runs
after one warm-up, `--release` without `benchmark_counters`. The baseline column is the same
measurement taken from a `--release` build of the stashed pre-Phase-1 tree on the same machine in
the same session, so the two columns differ only by this change. Counters come from a separate
`--release --features benchmark_counters` build with `MOTH_COUNTERS=summary`. `docs` counter
totals are summed across its per-module AST counter resets.

These are single-machine attribution measurements, not the recorded five-run benchmark protocol.

### Wall Time

| Workload | Baseline | Phase 1 | Change |
|---|---:|---:|---:|
| `constant_chain_32` | `4.73ms` | `3.58ms` | `-24%` |
| `constant_chain_128` | `12.38ms` | `8.27ms` | `-33%` |
| `constant_chain_512` | `128.10ms` | `68.69ms` | `-46%` |
| `constant_dag_churn` | `8.35ms` | `6.43ms` | `-23%` |
| `fold_stress` | `15.19ms` | `11.16ms` | `-27%` |
| `nominal_capacity_stress` | `20.27ms` | `21.21ms` | flat |
| `docs` | `274.57ms` | `274.73ms` | flat |

Growth across the chains is now `3.58 / 8.27 / 68.69` for `32 / 128 / 512` constants, against
`4.73 / 12.38 / 128.10` before. The `512` case is still superlinear: the remaining quadratic term
is not in context construction.

### Counter Movement

| Counter | `chain_32` | `chain_128` | `chain_512` | `constant_dag_churn` | `fold_stress` | `docs` |
|---|---:|---:|---:|---:|---:|---:|
| `ast_constant_pass_visibility_entries_cloned` before | `2208` | `33408` | `526848` | `16456` | `30840` | `24816` |
| `ast_constant_pass_visibility_entries_cloned` after | `69` | `261` | `1029` | `187` | `257` | `17548` |
| `ast_constant_pass_prior_constant_ids_copied` before | `496` | `8128` | `130816` | `4092` | `7860` | `17337` |
| `ast_constant_pass_prior_constant_ids_copied` after | removed | removed | removed | removed | removed | removed |

`ast_constants_resolved`, `ast_constant_resolution_contexts_created`,
`ast_module_constant_declaration_clones`, `ast_declaration_replacements_by_path` and
`ast_scope_contexts_created` are unchanged on every fixture, which is the intended result: the
same scopes are still created per constant, but they no longer rebuild module-wide state.

`ast_constant_pass_prior_constant_ids_copied` was deleted rather than reported as zero. The
cumulative copy it measured no longer exists: the environment builder owns one
`resolved_module_constant_paths` set that constant-header, nominal-member and function-signature
scopes share by handle. A counter that can only ever read zero is noise, so the Phase 2 scaling
acceptance item it belonged to is satisfied structurally instead.

### Findings That Change Later Phases

1. **Context construction was not the `docs` cost.** `docs` is flat despite `-29%` visibility
   copying, because its constants are one or two per file and its AST time is dominated by
   const-template parsing and folding rather than by constant setup. The `docs` share of
   `frontend.ast.environment.constant_header_resolution` recorded in the Phase 0 baseline is
   therefore mostly fold work, and only Phases 3 and 4 can move it.
2. **The remaining `chain_512` superlinearity is downstream of the session.** With per-constant
   context construction now `O(1)` in module state, `68.69ms` for `512` trivial constants is still
   far above `4x` the `128` case. The next candidates are declaration-table replacement by path,
   the module-constant declaration clone, and constant normalisation, all of which Phases 2 and 3
   own.
3. **`nominal_capacity_stress` is unmoved, as designed.** Member-shell reconstruction builds its
   own contexts in `AstModuleEnvironmentBuilder::unresolved_member_syntax_to_declarations` and
   still clones all five side tables per struct or choice header. That pass belongs to Phase 8.
4. **The declaration table blocks a longer-lived scope tree.** `replace_declaration` commits each
   folded constant through `Rc::get_mut`, so no scope may hold the table across a commit. A
   session that keeps one root scope alive for the whole pass, with per-constant child frames, is
   only possible once the table has ID-based replacement that does not require sole `Rc`
   ownership. Phase 2 should decide that shape deliberately rather than discovering it again.

## Constant Evaluation And Type-System Plan - Shared File Visibility - 2026-08-22

The Phase 1 evidence recorded that `constant_chain_512` was still superlinear and named
declaration-table replacement, the module-constant declaration clone and constant normalisation as
the likely causes. Profiling found none of those. The remaining quadratic term was one full
`FileVisibility` copy per header, taken in two unrelated passes.

### How It Was Found

A `--profile profiling` build was sampled with `sample` over a local 4096-constant chain. Every one
of the 1619 samples in the window landed inside `AstModuleEnvironmentBuilder::build`, 954 of them in
`validate_nominal_generic_bound_surfaces` and 953 of those in `FileVisibility::clone`. A further 663
samples were the matching drop at the end of the same loop iteration. The pass was spending
essentially all of its time copying and freeing a visibility package it read one `TypeId` from.

`AstEmitter::emit` had the same shape: a `Rc::new(visibility_for(..)?.clone())` at the top of the
header loop, taken for every header including constants, which need no scope at all.

`FileVisibility` holds eight maps keyed off the whole module's declarations, so a module with `C`
declarations in one file paid `O(C)` per header in each pass, twice.

### Change

`HeaderBindingEnvironment::file_visibility_by_source` now stores `Arc<FileVisibility>` and
`visibility_for` returns the handle. `FileVisibility::visible_declaration_paths` is itself an
`Arc<FxHashSet<InternedPath>>`, so `ScopeContext::with_file_visibility` takes one argument and
shares both the package and its declaration gate. Binding construction writes the gate through
`FileVisibility::visible_declaration_paths_mut`, which holds the sole reference and never copies.

`Arc` rather than `Rc` because the package is header-stage data that AST, dependency sorting and
trait-evidence validation all read, and the header stage has no module-local ownership rule to
lean on.

### Wall Time

`frontend.ast.total`, median of five runs, `RAYON_NUM_THREADS=1`, `--release` with
`detailed_timers`, both binaries built from the same tree in the same session and measured
interleaved.

| Workload | Phase 1 | Shared visibility | Change |
|---|---:|---:|---:|
| `constant_chain_32` | `0.82ms` | `0.64ms` | `-22%` |
| `constant_chain_128` | `5.09ms` | `1.31ms` | `-74%` |
| `constant_chain_512` | `61.73ms` | `4.23ms` | `-93%` |
| local `constant_chain_2048` | `875.44ms` | `17.06ms` | `-98%` |
| `fold_stress` | `7.24ms` | `2.89ms` | `-60%` |
| `type_stress` | `11.90ms` | `6.44ms` | `-46%` |
| `environment_stress` | `10.66ms` | `4.97ms` | `-53%` |
| `nominal_capacity_stress` | `15.43ms` | `11.11ms` | `-28%` |
| `pattern_stress` | `4.63ms` | `3.78ms` | `-18%` |
| `template_stress` | `3.80ms` | `3.12ms` | `-18%` |
| `collection_stress` | `3.44ms` | `3.26ms` | `-5%` |
| `docs` | `180.92ms` | `169.44ms` | `-6%` |

Nothing measured got slower. The 2048-constant chain is a local ad-hoc fixture under `./tmp`, not a
committed benchmark case; it exists to show the curve past the committed `512` point.

### Stage Split

| Metric | `chain_512` before | after | `chain_2048` before | after |
|---|---:|---:|---:|---:|
| `frontend.ast.total` | `68.99ms` | `4.68ms` | `902.96ms` | `17.30ms` |
| `frontend.ast.environment` | `39.19ms` | `3.28ms` | `455.50ms` | `11.88ms` |
| `frontend.ast.emit` | `28.79ms` | `0.52ms` | `442.88ms` | `1.69ms` |
| `frontend.ast.finalise` | `0.96ms` | `0.83ms` | `4.42ms` | `3.57ms` |

Constant setup now scales linearly across the committed chain fixtures: `0.64 / 1.31 / 4.23ms` for
`32 / 128 / 512`, and `17.06ms` at `2048`.

### Counters

`ast_constant_pass_visibility_entries_cloned` was deleted with the copy it measured, as
`ast_constant_pass_prior_constant_ids_copied` was in Phase 1. It only ever attributed the constant
pass's own copy, which no longer exists, and it never saw the two larger copies this change
removed. No other counter moves: the same scopes are created, they just no longer rebuild
module-wide visibility.

### Findings That Change Later Phases

1. **The Phase 1 candidate list for the remaining superlinearity was wrong on all three counts.**
   `TopLevelDeclarationTable::replace_by_path` is already one hash lookup plus an indexed store,
   the module-constant declaration clone is `O(1)` per constant, and
   `ast_module_constant_normalization_expressions_visited` is exactly one per constant. Phase 2
   should treat the by-ID replacement work as a clarity and ownership change, not as a scaling fix,
   and re-measure before claiming a scaling result from it.
2. **Profile before choosing the next representation change.** Every counter in the constant pass
   was already linear when this cost was found. Counters proved the pass they instrument was clean
   and said nothing about the two passes that dominated the module. The remaining plan phases
   should confirm their target with a profile, not with the absence of a counter.
3. **The same clone-to-satisfy-borrow shape survives elsewhere.** The function-signature pass and
   `unresolved_member_syntax_to_declarations` still rebuild `Rc::new(map.clone())` snapshots of
   `resolved_type_aliases_by_path`, `generic_declarations_by_path`, `resolved_struct_fields_by_path`
   and `nominal_type_ids_by_path` per header. That is `O(declarations)` per function, struct and
   choice. Phases 8 and 9 own it, and `nominal_capacity_stress` and `environment_stress` are the
   fixtures that will show it.
4. **`docs` moved for the first time in this plan.** `-6%` with no constant-pass change confirms
   the copy was in shared header-loop machinery, not in constant-specific code.

## Constant Evaluation And Type-System Plan - Phase A Re-Baseline And Attribution - 2026-08-22

Evidence-only phase. The single source change is the `constant_header_resolution` timing-guard
scope. Everything else here is measurement.

Binaries: `cargo build --release --features detailed_timers,benchmark_counters` for timing and
counters, and `RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile profiling` with the
same features for sampling. All timings are the median of five independent runs at
`RAYON_NUM_THREADS=1`.

### Why This Phase Existed

The Phase 0 baseline was measured when per-declaration `FileVisibility` copying dominated every AST
workload. That cost was removed in `917f7e81c`, which moved AST time by between `6%` and `98%`
depending on the fixture. No share in the old baseline survived that, so no priority in the plan was
evidence-backed any more.

### Wall Time Baseline

All values in milliseconds. `-` means the metric did not fire for that workload.

| Case | `check.total` | `ast.total` | `ast.environment` | `env.constant_header_resolution` | `ast.emit` | `emit.const_template_parse` | `ast.finalise` | `finalise.module_constant` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `docs` | 266.634 | 171.868 | 98.064 | 87.137 | 54.314 | 37.539 | 19.261 | 18.948 |
| `one_module_kitchen_sink` | 8.507 | 2.083 | 0.770 | 0.144 | 0.830 | - | 0.475 | 0.143 |
| `type_stress` | 12.204 | 6.452 | 4.337 | 0.000 | 1.325 | - | 0.780 | 0.269 |
| `environment_stress` | 8.702 | 4.958 | 3.795 | 0.000 | 0.688 | - | 0.450 | 0.257 |
| `nominal_capacity_stress` | 14.915 | 11.244 | 10.710 | 0.124 | 0.201 | - | 0.327 | 0.199 |
| `fold_stress` | 7.078 | 2.927 | 1.545 | 1.331 | 0.638 | - | 0.712 | 0.320 |
| `expression_rpn_churn` | 8.710 | 1.290 | 0.247 | 0.000 | 0.845 | - | 0.190 | 0.063 |
| `template_stress` | 8.252 | 2.962 | 1.096 | 0.887 | 1.046 | - | 0.820 | 0.271 |
| `constant_dag_churn` | 4.352 | 1.574 | 1.090 | 0.949 | 0.189 | - | 0.288 | 0.226 |
| `constant_chain_32` | 2.289 | 0.464 | 0.285 | 0.241 | 0.071 | - | 0.101 | 0.077 |
| `constant_chain_128` | 3.829 | 1.278 | 0.825 | 0.765 | 0.145 | - | 0.283 | 0.258 |
| `constant_chain_512` | 9.698 | 4.212 | 2.868 | 2.733 | 0.474 | - | 0.811 | 0.779 |

Repeating `docs`, `nominal_capacity_stress`, `type_stress`, `environment_stress` at the default
thread count reproduced every figure within run-to-run noise. These are single-module or
module-parallel workloads whose AST cost does not move with the scheduler, so no separate
fixed-thread identity is needed for later phases.

The constant chains are now close to linear in constant count. `constant_header_resolution` across
`32 -> 128 -> 512` is `0.241 -> 0.765 -> 2.733`, so a `4x` input costs `3.2x` then `3.6x`. The
chain superlinearity that opened this plan is gone.

### The Timing Guard Was Mis-Scoped, And It Mattered

The guard was declared before `resolve_constant_headers` and dropped at the end of
`resolve_nominal_members_and_constants`, so it also billed the struct-field and choice-variant
loops to a metric named for constant resolution. Measured on both scopes:

| Case | wide guard | narrow guard | `ast.environment` |
| --- | --- | --- | --- |
| `docs` | 87.336 | 87.533 | 98.340 |
| `nominal_capacity_stress` | 6.344 | 0.127 | 10.661 |
| `type_stress` | 1.940 | 0.000 | 4.458 |
| `environment_stress` | 1.434 | 0.000 | 3.862 |
| `one_module_kitchen_sink` | 0.338 | 0.154 | 0.796 |
| `constant_chain_512` | 2.712 | 2.699 | 2.917 |

For `type_stress` and `environment_stress` the metric was reporting entirely borrowed time: the
true constant cost is zero and every millisecond it showed belonged to the member-shell loops. For
`nominal_capacity_stress` `98%` of the metric was borrowed. `docs` and the constant chains were
unaffected, which is why the mis-scoping survived Phase 0 unnoticed - the fixtures that would have
exposed it were read as constant-heavy precisely because this metric said so.

### Nominal Member Resolution Is Quadratic

`nominal-capacity-stress.moth` documents itself as a deterministic generated pattern, so the same
pattern was regenerated at four sizes to get a scaling curve. The generator and fixtures are
untracked under `tmp/phaseA/`. `cap-40` reproduces the committed fixture's shape and cost.

| Buckets | `check.total` | `ast.total` | `ast.environment` | `env.constant_header_resolution` |
| --- | --- | --- | --- | --- |
| 40 | 13.363 | 9.761 | 9.295 | 0.126 |
| 160 | 136.246 | 126.726 | 124.922 | 0.343 |
| 640 | 1991.782 | 1955.258 | 1947.223 | 1.415 |
| 2560 | 44010.615 | 43853.843 | 43816.283 | 6.776 |

A `64x` input costs `4714x` in `ast.environment`, which is `O(n^2.03)`. Constant resolution over the
same range is `54x`, so it is linear and is not the cause. At 2560 nominal declarations - a large
but unremarkable module - the environment pass takes **43.8 seconds**.

### Every Counter Is Linear

Across the same four sizes, no counter in the frontend grows faster than input. The closest
candidates are exactly linear:

| Counter | n=40 | n=160 | n=640 | n=2560 | growth |
| --- | --- | --- | --- | --- | --- |
| `ast_type_resolution_calls` | 1744 | 6964 | 27844 | 111364 | `63.9x` |
| `ast_constant_pass_side_table_entries_cloned` | 102 | 402 | 1602 | 6402 | `62.8x` |
| `ast_scope_contexts_created` | 149 | 569 | 2249 | 8969 | `60.2x` |
| `ast_declaration_replacements_by_path` | 65 | 245 | 965 | 3845 | `59.2x` |

Type resolution is called a linear number of times. What grew is the cost of each call.

`ast_constant_pass_side_table_entries_cloned` deserves its own note. It instruments the
**once-per-module** snapshot Phase 1 hoisted into `resolve_constant_headers`, which is correct and
linear. It has never seen the **per-header** snapshot in `constant_header_scope_context`, which is
the quadratic one. A counter reading linear beside a quadratic wall time is not a contradiction
when the counter is pointed at the wrong copy.

### Function-Level Attribution

Sampled with macOS `sample` at 1ms against the `profiling` binary, aggregated across runs.
`samply` was tried first, through `just profile-case` and `just profile-case-symbolicated`, and
reported `failed_raw_addresses` in both modes, matching the AUD-0002 note. `sample` attaches by
process name and cannot catch a workload shorter than roughly 100ms, which is why the scaled
fixture was needed rather than the committed one.

**`cap-640`, call graph, one representative run.** Line numbers are post-guard-fix
`type_resolution.rs`.

| Path | inclusive samples |
| --- | --- |
| `resolve_nominal_members_and_constants:308` - struct-field loop | 613 |
| ... `unresolved_member_syntax_to_declarations:1053` - build scope context | 316 |
| ... `unresolved_member_syntax_to_declarations:1096` - drop it at function return | 293 |
| `resolve_nominal_members_and_constants:425` - choice-variant loop | 449 |
| ... `unresolved_member_syntax_to_declarations:1053` | 231 |
| ... `unresolved_member_syntax_to_declarations:1096` | 216 |

`609` of `613` samples in the struct-field loop, and `447` of `449` in the choice-variant loop, are
the construction and destruction of one `ScopeContext`. Actual member parsing is in the noise.

`constant_header_scope_context` deep-clones five whole-module side tables per call:

```rust
.with_resolved_type_aliases(Rc::new(self.resolved_type_aliases_by_path.clone()))
.with_generic_declarations(Rc::new(self.module_symbols.generic_declarations_by_path.clone()))
.with_resolved_struct_fields_by_path(Rc::new(self.resolved_struct_fields_by_path.clone()))
.with_choice_variant_shells_by_path(Rc::new(self.choice_variant_shells_by_path.clone()))
.with_nominal_type_ids_by_path(Rc::new(self.nominal_type_ids_by_path.clone()))
```

`resolved_struct_fields_by_path` is `FxHashMap<InternedPath, Vec<Declaration>>`, and `Declaration`
owns an `Expression` and a `DataType`, both recursive. So the clone is deep, it is `O(module)`, and
it happens once per nominal header. Self time confirms it:

| Symbol | samples |
| --- | --- |
| malloc / free family, combined | ~6565 |
| `<DataType as Clone>::clone` | 535 |
| `_platform_memmove` | 337 |
| `<Expression as Clone>::clone` | 308 |
| `_platform_memset` | 137 |
| `drop_glue<HashMap<InternedPath, Vec<Declaration>, FxBuildHasher>>` | 120 |
| `<Vec<Declaration> as Clone>::clone` | 111 |
| `<ExpressionKind as Clone>::clone` | 93 |
| `drop_glue<DataType>` | 84 |
| `drop_glue<ExpressionKind>` | 42 |

Roughly `79%` of non-idle self time is the allocator and `memmove`/`memset`. The named Rust
functions above it are the clone and drop of that snapshot.

**`docs`, 25 aggregated runs.** A different shape entirely:

| Symbol | samples |
| --- | --- |
| malloc / free family, combined | ~1988 |
| `_platform_memmove` | 240 |
| `core::hash::sip::Hasher::write` | 196 |
| `_platform_memset` | 143 |
| `hashbrown::HashMap<&_, TraitId, FxBuildHasher>::get` | 76 |
| `ast::templates::tir::summary::accumulate_nodes` | 71 |
| `BuildHasher::hash_one<ExternalTypeId>` (`RandomState`) | 58 |

`87%` of non-idle self time is allocation, deallocation and copying, but none of the callers above
it are the nominal path. `docs` has 72 modules, 545 constants, 5162 const templates and **2**
structs, so the quadratic member-shell cost cannot touch it.

### Findings That Change Later Phases

1. **The per-header side-table snapshot in `constant_header_scope_context` is the largest single
   defect in the frontend, and it is quadratic.** One call site, five map clones, once per nominal
   header. It is `>99%` of the cost of the pass that contains it and `O(n^2.03)` in module size.
   This was already written into the plan as Phase B, sized from code shape alone; the measurement
   promotes it to first place ahead of every other performance phase.
2. **The fix must be copy-on-write, not hoisting.** `resolve_type_declarations` writes
   `resolved_struct_fields_by_path` and `choice_variant_shells_by_path` inside the same loop that
   snapshots them, and headers are dependency-sorted so a later struct's fields can read an earlier
   struct's resolved fields. A snapshot hoisted out of the loop would go stale silently. Holding
   the tables behind `Rc` and writing through `Rc::make_mut` gives free reads, and the scope context
   is dropped before the next write, so the builder is the sole owner at write time and the clone
   does not happen.
3. **`docs` and the nominal fixtures need different phases, and neither substitutes for the
   other.** `docs` spends `87.1ms` of its `98.1ms` environment in constant header resolution with
   `ast_expression_fold_items = 0` and `constant_fold_attempt_count = 0`. Its constant cost is
   const-template work, not arithmetic folding. Phase D's move-only folding cannot be validated on
   `docs`, and Phase B cannot be validated on it either.
4. **Counters cannot find this class of defect, and this is the second time.** Every counter was
   linear while wall time was quadratic. The rule already in the plan is now proven twice; the
   counter that looked closest to the defect was measuring a different copy in a different pass.
5. **`ExternalPackageRegistry` hashes with SipHash.** All fifteen of its maps use
   `std::collections::HashMap` with the default `RandomState`, next door to
   `datatypes/environment.rs:92` which uses `FxHashMap` for the same key type. Only the
   `hash_one<ExternalTypeId>` samples (58, about `2%` of `docs` moth self time) are confirmed to be
   the registry; the larger 196-sample `sip::Hasher::write` is shared across all SipHash users and
   was not attributed. Worth confirming and fixing, but it is a small independent change and not
   evidence for any phase in this plan.

## Constant Evaluation And Type-System Plan - Phase B Copy-On-Write Side Tables - 2026-08-22

Phase B removed the per-header deep clone of the environment builder's side tables. Phase A
measured that clone at `O(n^2.03)` in module size and `>99%` of the cost of the pass containing it.
This section records what changed and what it bought.

### What Changed

`AstModuleEnvironmentBuilder` now owns five side tables behind `Rc` instead of by value:
`resolved_struct_fields_by_path`, `choice_variant_shells_by_path`, `resolved_type_aliases_by_path`,
`nominal_type_ids_by_path` and `generic_declarations_by_path`. Every `ScopeContext` built during
environment construction takes an `Rc::clone` handle instead of a `Rc::new(map.clone())` snapshot.
Writers go through `Rc::make_mut`.

`generic_declarations_by_path` moved out of `ModuleSymbols` into the builder at the start of
`build`, so it has one owner and one handle rather than a copy taken per header. This also deleted
a threaded `finish_environment` parameter that existed only because `self` was consumed before the
map could be moved.

The tables were already `Rc<FxHashMap<..>>` on the `ScopeContext` side. Only the builder held them
by value, so the entire fix is one ownership change plus nine `Rc::make_mut` write sites.

### Measured Effect

Release binary built with `--features timers`, `MOTH_TIMERS=full`, `RAYON_NUM_THREADS=1`, median of
7 runs. `before` is commit `b997e593a` built from a clean worktree; `after` is the same tree with
Phase B applied.

| case | metric | before (ms) | after (ms) | change |
| --- | --- | --- | --- | --- |
| `nominal_scaling_320` | `ast.environment` | `470.036` | `12.150` | **`38.7x` faster** |
| `nominal_scaling_320` | `check.total` | `491.138` | `32.575` | `15.1x` faster |
| `type_stress` | `ast.environment` | `4.276` | `1.226` | **`3.49x` faster** |
| `type_stress` | `ast.total` | `6.356` | `3.303` | `1.92x` faster |
| `environment_stress` | `ast.environment` | `4.268` | `1.647` | **`2.59x` faster** |
| `environment_stress` | `ast.total` | `5.526` | `3.125` | `1.77x` faster |
| `constant_chain_512` | `ast.total` | `4.130` | `4.138` | unchanged |
| `docs` | `ast.environment` | `96.754` | `97.171` | unchanged |
| `docs` | `check.total` | `263.381` | `265.236` | unchanged |

`docs` is unchanged, and that is the expected result rather than a disappointment. Phase A recorded
that `docs` has 72 modules, 545 constants and **2** structs, so a cost that scales with the number
of nominal headers was never its cost. Reporting Phase B as a win for `docs` would have required
ignoring what Phase A already measured.

`constant_chain_512` is unchanged for the same reason in the other direction: it has no nominal
headers, so it never paid the per-header snapshot. It is in the table to show the change is inert
where it should be inert.

### Growth Exponent

`just bench-scaling`, the lane added in the hardening slice, measures the same pass in-process:

| series | metric | before | after |
| --- | --- | --- | --- |
| `nominal_members` | `frontend.ast.environment` | `n^1.86` | `n^0.98` |
| `constant_chain` | `frontend.ast.total` | `n^0.82` | `n^0.86` |

The `nominal_members` points after the change are `5.125 / 10.037 / 19.609 / 39.147 ms` for sizes
`40 / 80 / 160 / 320`. Each doubling of input now costs almost exactly a doubling of time. The pass
is linear.

`just bench-scaling` joined `just validate` with this phase. It was deliberately kept out of the
gate while it failed, because a gate must pass on every commit.

### Findings

1. **`Rc::make_mut` never clones on these paths, and the scaling lane is the proof.** The design
   depends on every `ScopeContext` handle being dropped before the next write, so the builder is
   the sole owner at write time. If a handle ever escaped a loop iteration, `make_mut` would clone
   on every write and the quadratic cost would return silently, with no test failing and no counter
   moving. `n^0.98` is what rules that out, and the lane in `just validate` is what keeps ruling it
   out. This is the first defect class in this repository with a standing automated guard.
2. **The counter that was supposed to instrument this was deleted, not repointed.**
   `ConstantPassSideTableEntriesCloned` measured the once-per-module session snapshot, which was
   linear, while the real copy next door was quadratic. Phase B turned that snapshot into an
   `Rc::clone` as well, so no copy survives for the counter to instrument. A counter reporting a
   number for work that no longer happens is worse than no counter.
3. **Hoisting turned out to be unnecessary everywhere, not just at the quadratic site.** The plan
   held hoisting in reserve for the function-signature and trait-requirement passes, which read
   tables they never write. Once a handle costs a refcount, a shared handle is strictly better than
   a hoisted snapshot: same cost, no staleness question to answer. Both passes take handles.
4. **`generic_declarations_by_path` was not read-only, and assuming it was broke seven tests.**
   The first version of this change moved the map out of `ModuleSymbols` at the start of `build` on
   the belief that every environment pass reads it and none writes it. Import projection writes it:
   it registers metadata for each imported generic nominal, under both the local and the internal
   path. Taking the map early therefore left that writer filling a map nobody read, and imported
   generic types lost their parameter metadata - `expected T, found Int` on generic receiver
   boundaries, and `Type 'Wrapper' does not accept generic arguments` on a generic struct facade.
   The integration suite caught all seven; no unit test did, because the failure only appears
   across a module boundary. The fix routes that writer through the same handle, which is the shape
   the rest of the phase already used.
5. **The `module_constants` linear scan is gone, and it was a scan of the wrong container.**
   `is_explicit_compile_time_constant` scanned `Vec<Declaration>` once per fixed-capacity check
   during body emission. `AstModuleLookups` now carries `module_constant_paths`, the same
   `Rc<FxHashSet<InternedPath>>` the builder already maintained, and the check is a hash lookup.
   The two containers are written by one method so they cannot drift. This is not visible in the
   table above because no committed fixture combines many module constants with many fixed-capacity
   expressions; `docs` has the constants and not the capacities.

The measurements in this section were taken before finding 4 was fixed. The fix moves two `insert`
calls between two maps and changes no allocation on any measured path, so the numbers stand; the
scaling lane was re-run afterwards and reports the same `n^0.98`.

## Constant Evaluation And Type-System Plan - Phase C Folded-Value Authority - 2026-08-23

### What changed

Phase C (commit `6aa8aa513`) replaced `AstModuleLookups::module_constants: Vec<Declaration>` and
`Ast::module_constants` with one module-local `ConstValueStore`: an indexed value graph plus one
row per authored module constant. Public-interface projection, HIR constant lowering, HIR
references and const-record field access, generated materialisation, project-config extraction and
`.mtf` content extraction all read that store through one borrowed postorder visitor. The recursive
production normalizer, the AST-expression-to-HIR constant walker, the string-keyed
`const_templates_by_name` side channel and the duplicate declaration ownership are gone. The TIR
reducer now produces the scalar emission and the structured projection from the same fold, so a
template-valued constant is folded once instead of twice.

This review slice removed three test-only parallel paths the phase left behind, restored one
diagnostic boundary it had changed, and deleted a dead row field. Details below.

### Measured

Release `--features timers`, `RAYON_NUM_THREADS=1`, median of 7, isolated `CARGO_TARGET_DIR` for
each side. Before is `e782e79d8` (Phase B), after is `6aa8aa513` (Phase C).

| workload | metric | before | after | change |
| --- | --- | --- | --- | --- |
| `docs` | `frontend.ast.total` | 170.271ms | 167.950ms | -1.4% |
| `docs` | `finalise.module_constant` | 18.536ms | 17.607ms | -5.0% |
| `docs` | `public_interface.project` | 1.152ms | 0.533ms | **-53.7%** |
| `docs` | `frontend.hir` | 2.814ms | 3.349ms | +19.0% |
| `docs` | `command.check.total` | 265.293ms | 262.741ms | -1.0% |
| `constant_chain_512` | `frontend.ast.total` | 4.416ms | 4.216ms | -4.5% |
| `constant_chain_512` | `frontend.hir` | 0.467ms | 0.395ms | -15.4% |
| `constant_chain_512` | `command.check.total` | 10.216ms | 9.810ms | -4.0% |
| `fold_stress` | `frontend.ast.total` | 2.999ms | 2.887ms | -3.7% |
| `template_stress` | `frontend.ast.total` | 3.059ms | 3.008ms | -1.7% |

The Phase C checkpoint recorded no wall-time claim because the stored benchmark comparison set had
changed. That was the right call for `just bench-check`, but it left the phase unmeasured. Measured
directly against a rebuilt `e782e79d8`, Phase C is a small consistent win everywhere and a large one
in public-interface projection, which no longer walks an expression tree or consults a
string-keyed template map.

`docs` `frontend.hir` is the one metric that moved the wrong way: `+0.5ms` on a `263ms` compile.
`lower_module_constants` collected `(InternedPath, ConstValueId)` pairs into a vector before the
loop to release the store borrow, so it cloned each path twice where the old code cloned each
declaration once. It is real and it is recorded rather than rounded away.

**Partially recovered before Phase D.** The store now leaves `HirBuilder` once for the whole pass
instead of once per value, and the loop reads borrowed rows, so the owned `Vec` and its path clones
are gone. Measured the same way, median of 7 against a rebuilt `28ab27f0a`:

| workload | metric | before | after | change |
| --- | --- | --- | --- | --- |
| `docs` | `frontend.hir` | 3.36ms | 3.20ms | -4.8% |

That is `-0.16ms` of the `+0.535ms`. The path clones were about a third of the regression; the rest
is elsewhere in the new lowering path and is not worth chasing at `3ms` on a `260ms` compile. The
point of measuring was to find out how much the clones actually cost, and the answer is: less than
the attribution implied.

### Findings

1. **The phase's own acceptance box was measurable and was not measured.** `each module constant
   has exactly one folded-value owner` is a structural claim and it holds. But every other box in
   the phase - public projection consuming the store without deep intermediate clones, the deleted
   conversion walkers - has a wall-time consequence, and a rebuilt before-binary answers it in
   about twenty minutes. A changed `bench-check` comparison set is a reason not to quote
   `bench-check`, not a reason to leave a deletion phase unquantified.

2. **A user diagnostic became an internal compiler error, and only a deleted test knew.**
   Preparation returns a `final_value_kind` and an `outcome` independently: a `RenderableString` or
   `WrapperTemplate` template can still carry `TemplatePreparationOutcome::Runtime`. The old
   `normalize_module_constant_template_expression` branched on the outcome and rejected the runtime
   case with `InvalidTemplateStructureReason::NonFoldableConstTemplate`. Phase C's replacement
   branched on the kind alone, so a runtime-dependent renderable or wrapper constant fell into the
   "must have folded" arm and produced `CompilerError` - an internal invariant failure where a
   structured source diagnostic belongs. The regression was invisible because the only test that
   covered it had been retargeted at a retained `#[cfg(test)]` copy of the old function, whose own
   assertion message names the regression: *"must not report the old internal fold transformation
   error"*. Rejection now keys off whether preparation published the template at all, which is the
   fact that actually distinguishes a constant value from a runtime one.

3. **Three test-only parallel paths were retained in production files instead of the dead code
   being deleted.** Each has the same shape: production drops a path, its unit tests would fail, so
   the path survives under `#[cfg(test)]` and the tests keep passing against code nothing ships.
   - `HirBuilder::module_constant_expressions_by_name` plus `lower_test_module_constant_expression`
     - a second module-constant lowering path that chases references between constants and carries
     its own cycle guard. Production module constants reach HIR as an already-folded acyclic value
     graph containing no references. Its one test, `rejects_cyclic_module_constant_dependencies`,
     asserted a rule that Stage 3 owns and rejects as `MOTH-RULE-0053` (*"Constant initializer
     references same-file constant 'b' before it is declared"*), with 23 committed integration
     cases. Deleted; the rule keeps its real owner.
   - `finalization/normalize_constants.rs` - an entire module retained as, in its own words, a
     "Test-only template normalization compatibility helper". Its two tests now drive the
     production owner, and finding 2 is what that retargeting immediately exposed.
   - `ConstValueResolver::resolve_explicit_top_level_constant` - the production explicit-constant
     fact builder, superseded by store-derived facts and kept `#[cfg(test)]` for two tests. One
     asserted a rule the store now satisfies structurally; the other moved next to the projection
     owner it actually tests.

4. **The compact row carried a field no consumer read.** `ConstValueRow` was specified as
   `{ declaration: DeclarationId, value: ConstValueId }` and built as such, but every consumer -
   config extraction, public projection, HIR, generated materialisation, `.mtf` extraction - joins
   a module constant by its defining `InternedPath`, so the store also kept a parallel
   `row_paths: Vec<InternedPath>`. Nothing ever read the `DeclarationId`. Removing one dead
   `declaration_table.get_by_path(path).is_some()` guard in const-fact collection made the whole
   chain visible to `-D warnings` at once: the row field, `iter_module_constant_rows`,
   `TopLevelDeclarationTable::iter_with_ids` and a `#[cfg(test)] DeclarationId::from_index`, all
   added by the phase and all unread. The row is now `{ path, value }` and
   `environment/declaration_table.rs` is byte-identical to its pre-Phase-C state.

5. **A whole `TypeEnvironment` was deep-cloned on the success path to be available in an error
   closure.** `let materialisation_type_environment = type_environment.clone();` ran for every
   module so that a `map_err` arm could attach type context to a diagnostic. Cloning inside the
   closure costs the same on the error path and nothing on the success path. In a phase whose
   purpose is deleting duplicate representations, an unconditional deep clone of the module type
   environment is the wrong direction.

6. **A `RefCell` guard outlived its use by roughly 250 lines.** The `template_ir_store.borrow()`
   taken for store construction was a bare `let` in `AstFinalizer::build`, so the `Ref` lived to the
   end of the function - across const-fact collection and generated-materialisation preparation,
   both of which hold the same `Rc<RefCell<TemplateIrStore>>`. Neither takes it mutably today, so
   nothing panicked, but the guard is now scoped to the block that needs it.

### Attribution: what neither B nor C moved

`docs` remains the only large real workload, and its cost is now sharply concentrated:

| stage | ms | share of `ast.total` |
| --- | --- | --- |
| `ast.environment.constant_header_resolution` | 86.135 | 51.3% |
| `ast.emit.const_template_parse` | 36.772 | 21.9% |
| `ast.finalise.module_constant` | 17.607 | 10.5% |
| `ast.total` | 167.950 | - |

Phase A finding 5 already recorded the `87.1ms`; this confirms it survived both Phase B and Phase C
essentially untouched (`-0.2%` across C), and quantifies its share: **constant header resolution is
half of the AST time and a third of the whole `docs` check.** Phase A also already established that
Phase D cannot be validated on `docs`, because `docs` has `ast_expression_fold_items = 0` - its
constant cost is const-template work, not arithmetic folding. Taken together those two facts mean
the plan's remaining phases do not target the single largest frontend cost in the repository. That
is a sequencing decision, not an implementation defect, and it is recorded here so the next phase
starts from it rather than rediscovering it.

---

## Constant Evaluation And Type-System Plan - Phase D Attribution And Fold-Cache Deletion - 2026-08-23

Phase D opened by attributing the `86ms` `constant_header_resolution` cost the previous section left
standing, rather than proceeding on the plan's assumed weighting. The attribution changed what the
phase should do, so it is recorded before the work.

### Method

Release `profiling` profile with `detailed_timers`, `RAYON_NUM_THREADS=1`, `moth check docs`.
Attribution came from direct `Instant` probes around the three regions of
`ConstantResolutionSession::resolve_constant_header`, accumulated across all `545` module constants
in `docs`. The probes were temporary and are not in the tree. Timing deltas come from interleaved
A/B runs of two binaries built in isolated target directories - a `git worktree` at the before
commit and the working tree - alternating one run each, median of 9.

### Reading counters: one line per module, not one per project

`AstCounter` storage is a thread-local. `Ast::build` resets it at the start of every module and
publishes it at the end, and `collector::record_counter` *pushes* each publication rather than
accumulating it. `MOTH_BENCH counter` therefore emits one line per counter **per module**. On
`docs` that is 73 blocks. Reading the first block reads one module: it reports
`ast_constants_resolved = 13` where the project total is `545`, and `ast_tir_nodes_created = 0` for
a documentation site built entirely from templates.

Every counter figure quoted for a multi-module workload must be summed across its lines. The
single-module stress fixtures are unaffected, so the `960/960` and `392/392` fold ratios recorded in
the Phase A section stand as written.

### Where the 86ms goes

| region | total | share |
| --- | --- | --- |
| `constant_header_scope` (`ScopeContext::new` + synthetic `AstModuleLookups`) | 6.23ms | 7.3% |
| `resolve_declaration_syntax` (parse and fold the initializer) | 71.60ms | 83.8% |
| `const_value_kind_with_template_classifier` | 0.055ms | 0.06% |
| unattributed (warning drain, loop, timing guard) | ~7.5ms | ~8.8% |

Three consequences:

1. **The synthetic-context candidate is 7%.** The Phase C section named `ScopeContext::new` - which
   builds a complete `AstModuleLookups` of empty maps that the constant session then overwrites - as
   a visible candidate, and said to measure it rather than assume it accounted for the `86ms`. It
   accounts for `6.2ms`. It remains worth deleting for clarity; it is not a performance phase.

2. **This is not type-resolution or diagnostic-spelling cost.** `docs` runs
   `ast_type_resolution_calls = 605` against `545` constants - one per constant. Phase D's
   lazy-diagnostic block stands on its own merits, but the claim that it was "the only part of the
   remaining plan that touches the code this cost runs through" was wrong. The code this cost runs
   through is the template parser and the TIR fold reducer.

3. **The remaining 71.6ms is template output amplification.** `docs` parses `894KB` of template text
   and folds it into `9.88MB` of output over `10853` folds, interning once per fold. Nested
   templates re-emit and re-intern their whole subtree at each enclosing level, so the innermost
   text is copied once per level of nesting - an 11x amplification from parsed text to folded
   output. That belongs to the template plan, not this one.

### The TIR fold cache never hit

`tir/fold_cache.rs` memoised `fold_exact_view` on `(TirViewIdentity, loop iteration limit,
bindings-empty)`. Hit rates across every committed template workload:

| workload | fold attempts | hits |
| --- | --- | --- |
| `docs` | 10853 | 0 |
| `speed_test` | 285 | 1 |
| `template_stress` | 78 | 0 |
| `template_render_plan_churn` | 51 | 0 |
| `one_module_kitchen_sink` | 7 | 0 |
| `code_highlighter_stress` | 1 | 0 |
| **total** | **11275** | **1** |

The cache was not broken. A repeated child reference inside one template does hit, and a unit test
proved exactly that. Real source simply almost never folds the same exact view twice. What it cost
on every fold was a `HashMap` allocation per fold context at three construction sites, a key
construction, a hash and probe, and a `TemplateFoldResult` clone on insert.

One intermediate experiment is worth not repeating: widening the cache to process lifetime aborts
the compiler with an absurd allocation request. `TirViewIdentity` is **module-local**, so a root id
minted in one module resolves against another module's store. Any cache on this key is bounded by
one `TemplateIrStore`. Rerunning the experiment at module lifetime still produced `0` hits on
`docs`, which is what settled the deletion.

The cache, its key, its module, its two counters and the per-fold result clone are deleted.

### Measured

Interleaved A/B against a rebuilt `bed00e0bf`, isolated target directories, median of 9,
`RAYON_NUM_THREADS=1`.

`docs`:

| metric | before | after | delta | pct |
| --- | --- | --- | --- | --- |
| `ast.total` | 168.273 | 166.712 | -1.562 | -0.93% |
| `ast.environment` | 96.472 | 95.330 | -1.141 | -1.18% |
| `ast.environment.constant_header_resolution` | 86.289 | 85.169 | -1.119 | -1.30% |
| `ast.emit` | 54.102 | 53.444 | -0.658 | -1.22% |
| `ast.emit.const_template_fold` | 3.087 | 2.666 | -0.422 | -13.66% |
| `ast.finalise` | 17.613 | 17.644 | +0.031 | +0.18% |

`template-stress.moth`: `ast.total` `3.043 -> 2.875ms` (`-5.53%`), `ast.environment` `-3.49%`,
`ast.emit` `-3.27%`.

A caution on method, learned here: two sequential builds of the same tree in the same target
directory produced `docs` `ast.total` readings `2ms` apart, enough to invert the sign of this
result. Only the interleaved two-binary comparison is trustworthy at this magnitude.

### Found and not fixed

`TemplateIrSummary::estimated_output_bytes` under-predicts by about `3x`, structurally.
`record_text_node` adds the node's own bytes; `record_child_template` adds nothing, so a parent's
estimate excludes every child's output. On `docs`: estimated `3.24MB`, actual `9.88MB`, recorded
miss `6.63MB`. `FoldOutputState::with_capacity` sizes the fold buffer from that number, so
template-heavy folds regrow their buffers. Propagating child estimates is template-plan work, and
the win is bounded by the reallocation cost rather than by the `6.63MB` itself. Recorded so it is
not re-derived from the counters.

### The advisory constant environment is not a hot path

The Phase C review raised `ConstFactCollector` as a Phase D target: it reconstructs every
substitutable module constant into a rich `Expression`, keeps those in `module_explicit_env`, and
clones the whole environment for every function body and every nested `if`, block and scope. The
plan gained a work block for it. It was instrumented before being changed, and the instrumentation
retired it.

`docs`, 545 module constants across 73 modules:

| region | total |
| --- | --- |
| `collect_explicit_top_level_facts` | 0.497ms |
| of which `expression_for_store_value` rebuilds (545) | 0.090ms |
| `collect_private_and_body_local_facts` (every scope clone) | 0.105ms |
| **`ConstFactCollector::collect`** | **0.60ms of a 168ms AST build - 0.36%** |

Environment copying across committed fixtures, from the two durable counters added for this
checkbox:

| workload | env clones | entries copied |
| --- | --- | --- |
| `docs` | 78 | 640 |
| `speed_test` | 55 | 2654 |
| `deep_scope_churn` | 26 | 260 |
| `one_module_kitchen_sink` | 18 | 136 |
| `constant_dag_churn` | 2 | 176 |

The reason the pathological shape does not appear is that `module_explicit_env` is **per module**.
`docs` has 545 constants but 73 modules, so an environment averages 8 entries, not 545. The
*many visible constants x many lexical scopes* shape needs both in one module, and no committed
fixture has it. Manufacturing one would prove a cost the compiler does not pay.

The block stays in the plan as a single-representation deletion - two representations of one
already-folded value is still worth removing - but it is no longer a performance item, and the
counters keep that claim re-verifiable instead of resting on this paragraph.

### Move-only folding: the `1:1` diagnostic waste is gone, and the control run refuses the wall-time claim

Method: counters from a `profiling` build with `benchmark_counters`, summed across every module
line. Timings interleaved against a rebuilt `a1cc58cf2` in isolated `CARGO_TARGET_DIR`s from a
`git worktree`, `RAYON_NUM_THREADS=1`, median of 9 runs (7 for `docs`).

Three deletions in the AST expression path:

- `constant_fold` takes `Vec<ExpressionRpnItem>` by value. Every operand and operator either moves
  onto the fold stack or moves back into the runtime result; the `to_owned()` per item is gone.
- `evaluate_expression` pops the sole folded operand instead of cloning `stack[0]`.
- `ExpressionResultType` is deleted. It paired a `TypeId` with a `DataType` spelling and was built
  once per RPN item. Every operator policy already decided on `TypeId` alone, and the spelling had
  exactly one consumer - the partial-fold runtime node, which now builds its own. The typing stack
  is `Vec<TypeId>`.

| workload | fold items | operand clones before | after | materialisations before | after |
| --- | --- | --- | --- | --- | --- |
| `fold_stress` | 960 | 780 | 0 | 960 | 0 |
| `speed_test` | 804 | 613 | 0 | 780 | 41 |
| `type_stress` | 81 | 54 | 0 | 75 | 25 |
| `template_stress` | 21 | 13 | 0 | 21 | 5 |
| `constant_chain_512` | 1533 | - | 0 | - | 0 |

`ast_expression_operand_clones` was kept rather than deleted, with its meaning narrowed to "RPN
items copied whole so a caller can keep its pre-fold input". Two template callers still do that,
because their non-folding outcome rebuilds a runtime node from the items as they stood before the
fold. Both are `0` on every committed fixture, and both now return the surviving expression by
move rather than cloning it.

**The wall-time A/B is reported and then withdrawn, on the strength of its own control.**

| case | metric | before | after | delta |
| --- | --- | --- | --- | --- |
| `constant_chain_512` | `ast.total` | 4.088ms | 4.025ms | -1.55% |
| `constant_chain_512` | `constant_header_resolution` | 2.619ms | 2.506ms | -4.31% |
| `speed_test` | `ast.total` | 12.092ms | 12.128ms | +0.30% |
| `docs` (control) | `ast.total` | 166.441ms | 165.474ms | -0.58% |

`docs` executes none of the changed code. Its `ast_expression_fold_items`,
`ast_expression_typed_stack_items` and `ast_diagnostic_data_type_materialisations` are all `0`,
before and after. It still moved `-0.58%`, a third of the effect measured on the fixture that
exercises the change hardest. A control that cannot move, moving by that much, means the treatment
effect is below the noise floor of these fixtures. The improvement is therefore **counter-verified
only**, which is what the plan permitted for this phase from the start. The table above is kept
because a reader who re-runs it will get these numbers and should know they were not the basis of
the claim.

**Found while measuring: every `docs` expression is a single-operand fast path.**
`ast_expression_typed_stack_items` is `0` across all 73 modules while
`ast_expression_ordering_input_items` is `1322`. Operator typing and constant folding never run on
the largest real workload. This bounds what is left of the typed-postfix work: deleting the second
RPN walk cannot help `docs`, and the fixtures where it would help are the sub-5ms ones the control
run just showed are unmeasurable.

### The advisory constant environment now holds one representation

Follow-on to the section above, which measured this path at 0.6ms of a 168ms `docs` AST build and
demoted it from a performance item to a deletion. No wall-time claim is attached to this change,
and none should be.

`ConstValueEnvironment` was `FxHashMap<InternedPath, Expression>`, rebuilt from the store for every
module constant before any function body was walked, then copied whole into every nested lexical
scope. It is now a shared `Rc` module base of `ConstValueId` plus a per-scope overlay holding only
the bindings that scope introduced itself. A module constant therefore has exactly one
representation - the store's - until a reference actually materialises one.

| workload | env clones | entries copied before | after |
| --- | --- | --- | --- |
| `docs` | 78 | 640 | 0 |
| `speed_test` | 55 | 2654 | 14 |
| `deep_scope_churn` | 26 | 260 | 0 |
| `constant_dag_churn` | 2 | 176 | 0 |

The eager rebuild goes with it. `docs` built 545 throwaway `Expression`s per module finalization
and now builds one per reference that needs one. `ConstValueResolver::expression_for_store_value`,
the pass-through wrapper that existed only for that eager loop, is deleted.

`expression_for_resolution` itself could not be deleted, and the reason is structural rather than
incidental: the advisory resolver substitutes constants into `ExpressionRpnItem::Operand` and hands
them to `constant_fold`, which is an `Expression` interface. Making the fold evaluator consume
store values directly is a store-lifecycle change the plan puts outside this phase.

Coverage gap found and closed: no test covered a body-local declaration that references a module
constant - exactly the path this change reroutes. There are now two, a bare reference and an
arithmetic expression that folds over one.

### Type resolution is not a hot path, including on the fixture that calls it 13,924 times

The lazy-diagnostics block was the last part of the Phase D plan still described as performance
work, so `resolve_type` was instrumented before any of it was started. The probe is
reentrancy-safe - it accumulates only non-nested entries - so the recursive calls a generic
instance makes are counted once, inside their top-level entry.

| fixture | `ast_type_resolution_calls` | `resolve_type` total | enclosing stage |
| --- | --- | --- | --- |
| `nominal_scaling_320` | 13924 | 489µs | 38.68ms `ast.environment` (1.3%) |
| `docs` | 605 | ~20µs summed over 73 modules | 168ms `ast.total` |
| `type_stress` | 698 | 21.7µs | ~3ms `ast.total` |

About 35ns a call. `nominal_scaling_320` was the promising case: it is the fixture the
`nominal_members` scaling series budgets, and it makes 23x the calls `docs` does. It still comes
out at 1.3% of the stage it runs in. Whatever makes `ast.environment` cost 38.68ms there, it is
not `resolve_type`.

> **Corrected below — see "Phase E attribution".** The `nominal_scaling_320` row is probe-inflated.
> That stage is 12.28ms, not 38.68ms: the probe producing the 489µs cost roughly three times the
> function it measured and inflated its own denominator. `resolve_type` is up to 4.0% of the stage
> there, not 1.3%. The conclusion holds; the number does not. The `docs` and `type_stress` rows are
> unaffected, because their denominators are thousands of times the probe cost.

One deletion from the block did land, because it was named concretely and was a representation
mismatch rather than a guess: `TypeResolutionContextInputs` carried `visible_declaration_ids` as a
borrowed set, so entering field-default evaluation had to copy every visible path into a fresh
`Arc`. It now carries the handle the scope already stores, and shares it.

The rest of the block - splitting the broad optional inputs shape into explicit views, named
constructors, `TypeId`-first returns, borrowing resolved aliases and signatures - is left open and
marked for re-proposal rather than execution. There are 59 clone sites across `type_resolution/`,
and auditing them for read-only borrows is an unbounded refactor with nothing measured behind it.

**The pattern across this plan's measurements is worth stating on its own.** Every candidate
identified by reading the code - the synthetic constant scope, the TIR fold cache, the advisory
constant environment, the diagnostic materialisations, type resolution - came in small or at zero.
The one cost that is large, template output amplification at 71.6ms of an 86ms pass, was found only
by measuring. A future phase should instrument before it plans, not after.

## Phase E attribution: `ast.environment` on `nominal_scaling_320`

Method: `--profile profiling`, `RAYON_NUM_THREADS=1`, median of nine for stage timings, mean of the
warm runs of seven for probe attribution (the first run of each series is discarded as cold; it runs
about `1.5ms` high on this stage). Probes were temporary and are not in the committed tree.

### Correction: the `38.68ms` figure was the measuring probe

The Phase D results section recorded `ast.environment = 38.68ms` on this fixture and pointed Phase E
at it. That number does not reproduce. On a clean tree the stage is **`12.28ms`** and
`frontend.ast.total` is `15.51ms`.

Three candidate explanations were tested before the probe was accepted as the cause:

| hypothesis | test | result |
| --- | --- | --- |
| counters build inflates it | rebuilt with `detailed_timers,benchmark_counters`, isolated target dir | `12.33ms` - not it |
| the Phase D visibility-set change landed after the measurement | interleaved A/B, nine pairs, worktree built at the parent commit `8f4c01b88` | before `12.28ms`, after `12.28ms` - not it |
| the temporary `resolve_type` probe was compiled into that binary | the only remaining difference; `13924` calls at roughly `1.9us` of probe cost accounts for the missing `26ms` | accepted |

The `489us` and the `38.68ms` in the finding-8 table came from the same instrumented binary, so the
probe inflated its own denominator. `resolve_type` is up to `4.0%` of the stage, not `1.3%`. The
conclusion - type resolution is not a hot path - survives; the number Phase E was aimed at does not.

The rule this earns: **an attribution probe must be cheap relative to the function it measures, or
it corrupts the denominator as well as the numerator.** Place probes per-pass, not per-call, unless
per-call cost has been shown negligible. The Phase E probe below runs 24 marks per module.

### Where the `12.28ms` goes

Probe total reconciles with the stage to within `0.5%`, so nothing is unattributed.

| step | ms | share |
| --- | --- | --- |
| `resolve_nominal_members_and_constants` | 7.39 | 58.1% |
| `register_nominal_shells` | 4.61 | 36.3% |
| `validate_nominal_generic_bound_surfaces` | 0.58 | 4.6% |
| the other 21 steps combined | 0.11 | 0.9% |

Every import projection, alias resolution, trait-definition pass, function-signature pass,
receiver-catalog build, trait-evidence validation and public-surface build in this stage is together
under one percent of it.

One level down. The choice rows contain the payload rows, because
`unresolved_choice_variants_for_header` calls `unresolved_member_syntax_to_declarations` for record
payloads - an overlap that first showed up as an unreconcilable total and was resolved by splitting
the slots on member context.

| | ms | calls |
| --- | --- | --- |
| `member_shells:Allow:StructField` | 3.09 | 320 |
| `member_shells:Strict:StructField` | 2.36 | 320 |
| `choice_shells:Allow` *(contains the row below)* | 1.33 | 80 |
| `member_shells:Allow:ChoicePayload` | 1.32 | 240 |
| `choice_shells:Strict` *(contains the row below)* | 1.16 | 80 |
| `member_shells:Strict:ChoicePayload` | 1.15 | 240 |
| `resolve_constructor_shells_for_constants` | 1.27 | 1 |
| `resolve_struct_field_types` | 0.64 | 320 |
| `resolve_choice_variant_payload_types` | 0.18 | 160 |
| `resolve_constant_headers` | 0.05 | 1 |
| `build_generic_parameter_scope` | 0.015 | 800 |

**Member-shell construction across both passes is `7.94ms` of `12.70ms` - `62.5%`.** The type
resolution those shells exist to feed is `0.82ms`, `6.5%`. The scaffolding costs `9.7x` what the
work costs.

This is the first candidate in this plan that measuring has confirmed rather than retired. Every
earlier one - the synthetic constant scope, the TIR fold cache, the advisory environment, the
diagnostic materialisations, type resolution - came in small or at zero.

### A second target, not in the phase's work items

`constant_header_scope_context` is `2.82ms` over `1120` calls - `2.5us` each, `22%` of the whole
stage - called once per member-shell entry, so twice per header.

| | ms | share of the 2.82ms |
| --- | --- | --- |
| `ScopeContext::new` | 1.19 | 42% |
| the 17-call `with_*` builder chain | 1.13 | 40% |
| `header.canonical_source_file` | 0.37 | 13% |

The chain is `~59ns` per `with_*`, which is a large-struct move rather than the `Rc::clone` each
method nominally performs: `ScopeContext` carries two `FxHashSet`s, three `Vec`s and a dozen handles
inline, and every `with_*` takes it by value and returns it.

Retaining one shell per member halves the call count but does not touch the per-construction cost,
and the two passes cannot trivially share one context because the tables it clones
(`resolved_struct_fields_by_path`, `choice_variant_shells_by_path`) are rewritten between them. This
needs its own before/after.

## Phase E first slice: the per-scope lookup scaffold

Commit `19340ca29`. `ScopeContext::new` built a synthetic empty `AstModuleLookups` on every call -
roughly thirty heap allocations, including a fresh `StyleDirectiveRegistry::built_ins()` with eight
owned directive names - and every field it seeded into `ScopeShared` was overwritten by the `with_*`
chain the caller ran next. One shared empty package now serves every scope.

Interleaved A/B against a worktree build of `3b811906a`, isolated `CARGO_TARGET_DIR` per side,
`--profile profiling`, `RAYON_NUM_THREADS=1`, median of ten alternating runs.

### `nominal_scaling_320`

| metric | before | after | delta | ratio |
| --- | --- | --- | --- | --- |
| `frontend.ast.environment` | 12.319 | 9.373 | -2.946 | 1.31x |
| `frontend.ast.total` | 15.504 | 12.677 | -2.827 | 1.22x |
| `frontend.ast.emit` | 1.119 | 1.133 | +0.014 | 0.99x |
| `frontend.ast.finalise` | 2.030 | 2.103 | +0.073 | 0.97x |
| `frontend.bind_headers` (control) | 1.156 | 1.151 | -0.005 | 1.00x |
| `frontend.order_declarations` (control) | 2.018 | 2.050 | +0.031 | 0.98x |

`bind_headers` and `order_declarations` build no scope contexts. They are the only control
available for a change to a universal constructor: no fixture avoids executing one.

### `docs`

| metric | before | after | delta | ratio |
| --- | --- | --- | --- | --- |
| `frontend.ast.environment` | 95.514 | 93.922 | -1.592 | 1.02x |
| `frontend.ast.total` | 166.374 | 163.883 | -2.490 | 1.02x |
| `frontend.module.semantic_total` | 211.102 | 208.943 | -2.159 | 1.01x |

The gap between `1.31x` and `1.02x` is the reason for the attribution below.

### A withdrawn intermediate figure

The first reading of this change was `12.25ms` to `4.55ms`. It was measured on a binary with `53`
failing unit tests: the fixture was erroring out on `CapacityNotConstant` and skipping most of the
stage. The figure above is from the green build. A large unexplained win is evidence of a bug
before it is evidence of a win.

## Real-project attribution: `ast.environment` on `docs`

`73` modules, `545` constant headers. Per-pass probe, `34` marks, measured overhead `0.25%` of the
stage (`94.15ms` probed against `93.92ms` unprobed). The top row is independently confirmed by the
pre-existing `frontend.ast.environment.constant_header_resolution` detailed timer at `83.89ms`.

| | ms | share of the 94ms stage |
| --- | --- | --- |
| `resolve_nominal_members_and_constants` | 84.16 | 89.5% |
| ↳ `resolve_declaration_syntax` (545 calls) | 71.27 | 75.8% |
| ↳ initializer expression parse | 69.47 | 73.9% |
| ↳ `parse_template_expression` (331 calls) | 69.13 | 73.5% |
| ↳ `Template::new_const_required_with_type_interner` | 62.99 | 67.0% |
| ↳ `prepare_tir_view` in `Value` mode (325) | 3.13 | 3.3% |
| ↳ `fold_prepared_template` (284) | 2.89 | 3.1% |
| constant-header scope build (545 calls) | 5.25 | 5.6% |
| every import projection combined | 6.65 | 7.1% |
| `finish_environment` | 2.19 | 2.3% |
| `register_nominal_shells` | 0.095 | 0.1% |

`190us` per template constant, in TIR construction. `parse_template_expression` documents its
`Value`-mode re-preparation as deliberate duplication of the const-required preparation; that
re-preparation is `3.3%`, so the documented double-prepare is not the cost. That was the hypothesis
going in and measuring retired it.

The same two rows, side by side, are the point:

| | `nominal_scaling_320` | `docs` |
| --- | --- | --- |
| member-shell construction | 62.5% | 0.1% |
| template TIR construction | ~0% | 67.0% |

`nominal_scaling_320` is `100%` fixed-capacity struct fields driven by four constants. It isolates
exactly what it claims to and its number is not wrong - but a scaling fixture can only say what a
cost is, not what share of a real stage that cost holds. Both numbers from here on.

### Cross-checks on other multi-module fixtures

`resolve_nominal_members_and_constants` as a share of the probed environment total, with the
per-constant cost of `resolve_declaration_syntax`:

| fixture | modules | constants | share of stage | per constant |
| --- | --- | --- | --- | --- |
| `docs` | 73 | 545 | 89% | 130us |
| `module-graph` | 3 | 40 | 45% | 20.5us |
| `import-fanout` | 3 | 42 | 45% | 14.5us |

The shape is the same everywhere; `docs` constants cost `7-9x` more each because they are
templates.

## Generic instantiation: the Phase F fixture and what it found (2026-08-24)

Phase F of the constant-folding plan had no measurable target - every counter it was written
against was near zero on every committed fixture. The plan offered two options and this is
option one: commit a fixture that actually exercises generic instantiation at scale, re-measure,
and proceed only against what it shows.

### The fixture

`benchmarks/generic-scaling/generic-scaling-{20,40,80,160}.moth`, wired as the
`generic_instantiation` scaling series. `n` distinct concrete types, each driven through the same
seven generic call sites, so every site presents a substitution mapping and a generated-function
identity nothing else in the module shares.

Two design decisions that matter:

- **One small driver per type, not one growing body.** The first version grew the instantiation
  count and one function's body together. `frontend.borrow.initial` fitted `n^2.28` on it and
  `n^1.14` on the split version at the same instantiation count. That quadratic is body size, not
  generics. Any future generics fixture has to hold body size fixed or it measures the wrong thing.
- **Structural nesting, not nested applications.** `MOTH-SYNTAX-0015` forbids nested generic
  applications, so `Cell of Cell of T` is not a program. Depth comes from `{Cell of T}`,
  `{{Cell of T}}` and `Pair of A, B` instead.

### Release measurement

`profiling` profile, `RAYON_NUM_THREADS=1`, `MOTH_TIMERS=bench`, median of seven per point.

| metric | `n=20` | `n=40` | `n=80` | `n=160` | fitted |
| --- | --- | --- | --- | --- | --- |
| `build.frontend.total` | 46.30 | 131.74 | 423.08 | 1500.16 | `n^1.67` |
| `frontend.generated.materialise` | 41.20 | - | - | 1403.99 | `n^1.70` |
| `frontend.ast.total` | 5.53 | - | - | 27.77 | `n^0.78` |
| `frontend.borrow.initial` | 2.04 | - | - | 12.48 | `n^0.87` |
| `frontend.hir` | 1.76 | - | - | 10.44 | `n^0.86` |
| `frontend.ast.environment` | 0.574 | - | - | 1.609 | `n^0.50` |

Times in ms. At `n=160`, `frontend.generated.materialise` is `94.1%` of the frontend and
`frontend.ast.environment` - Phase F's entire target - is `0.11%`. The total's step ratios rise
across the series (`2.85x`, `3.21x`, `3.55x`), so the curve is still steepening at the largest
point.

Phase F was closed on this. Every conversion it proposed targets a component measured sublinear
and immaterial on a fixture purpose-built to be its worst case.

### The mechanism, from counters rather than timings

| `n` | `string_table_full_clones` | `string_table_merge_source_entries_scanned` | entries per clone |
| --- | --- | --- | --- |
| 20 | 201 | 40,476 | 201 |
| 40 | 401 | 120,896 | 302 |
| 80 | 801 | 401,736 | 502 |
| 160 | 1,601 | 1,443,416 | 902 |

Clones are exactly `10n + 1` - one per generated function, linear. Entries scanned per clone is
`~5n + 101` - linear in module size. The product is quadratic; measured `n^1.72`, matching
`frontend.generated.materialise` at `n^1.70`. Two independent measures, one curve.

`GenericTemplateArtefact::materialise_ast` opens with `StringTable::new()` then
`merge_from(&requester_context.string_table)`. `MaterialisationPreparation::materialise_ast` opens
with `self.string_table.clone()` then the same `merge_from`. `merge_from` re-interns every string
in the source table and allocates a remap `Vec` of the same length.

`merge_delta_from` and `fork_source` / `fork_for_module` already exist for exactly this and module
compilation already uses them - `string_table_delta_merge_calls` and
`string_table_fork_source_base_copies` are both `3` at every size on this fixture. Generated-function
materialisation uses neither.

### Real-project share

These are **debug-build** numbers, taken together in one pass so the shares are comparable to each
other. Do not read the absolute ms against the release table above.

| project | frontend total | `generated.materialise` | share |
| --- | --- | --- | --- |
| `docs` | 1605.8ms | 0.03ms | 0.002% |
| `module-graph` | 24.9ms | 0.001ms | ~0% |
| `type-stress` | 32.7ms | 0.003ms | ~0% |
| `generic-trait-churn` | 32.9ms | 11.80ms | 35.9% |
| `generic-scaling-160` | 4665.6ms | 4265.3ms | 91.4% |

`docs` barely uses generics, so its `0.002%` says nothing about this cost either way. The number
that should decide priority is `generic-trait-churn`: 181 lines, a handful of instantiations,
nothing adversarial about it, already `35.9%`. A project that uses generics at all pays this
immediately - it does not wait for `n=160`.

## Generated materialisation prefix sharing (2026-08-24)

Finding 23 was broader than its first string-table profile suggested. Removing full string-table
merges exposed full `TypeEnvironment` clones and drops as the next dominant cost, followed by
declaration-table cloning. Generated preparation now snapshots the requester type environment once,
shares inherited `TypeId` and nominal prefixes through immutable storage and keeps only a local
overlay. Top-level declaration tables form immutable inherited layers, with generated-local
replacement and append maps instead of detaching or rebuilding inherited vectors. String tables use
the existing fork-source and delta-merge contract in both Published and Preparing materialisation
paths and when generated sidecars rejoin the owning compiler.

The mechanism counters changed as intended on the four-point fixture:

| counter | `n=20` | `n=160` | post-fix shape |
| --- | --- | --- | --- |
| `string_table_full_clones` | 0 | 0 | eliminated |
| `string_table_merge_source_entries_scanned` | 476 | 3,416 | linear delta only |
| `string_table_delta_merge_calls` | 203 | 1,603 | one per materialisation boundary |
| `string_table_fork_source_base_copies` | 4 | 4 | constant per batch |
| `generated_declaration_inherited_row_copies` | 0 | 0 | structurally eliminated |

Release `profiling` profile, `RAYON_NUM_THREADS=1`, `MOTH_TIMERS=bench`, counters off, median of
seven independent invocations per point:

| metric | `n=20` | `n=40` | `n=80` | `n=160` | fitted |
| --- | --- | --- | --- | --- | --- |
| `frontend.generated.materialise` | 16.480 | 45.154 | 140.269 | 486.766 | `n^1.63` |

The same release protocol measured `frontend.generated.materialise` at `2.240ms` on
`generic-trait-churn` and `0.0021ms` on `docs`. The previous recorded figures used a debug build,
so their absolute values are not used as an A/B comparison. The scaling fixture supplies the
comparable before/after evidence: its endpoints fell from `41.20ms` / `1403.99ms` to
`16.48ms` / `486.77ms`, while the fitted exponent fell from `n^1.70` to `n^1.63`. The development
scaling gate fits `n^1.59`; its budget is tightened from `n^1.80` to `n^1.70` as a deliberately
close ratchet, not headroom.

## Data Layout Migration - Phase 0 Activation Baseline - 2026-09-04

Baseline freeze for `docs/roadmap/plans/compiler-source-token-and-diagnostic-data-layout-plan.md`.
Evidence-only phase: no compiler or language semantics changed. Every subsequent phase of that plan
compares against this section.

### Environment

| Field | Value |
| --- | --- |
| Date | 2026-09-04 |
| Branch | `token-and-diagnostic-data-layout-changes` |
| Activation commit | `b6f81fe58` |
| Prerequisite | Compiler Test Suite Hardening, delivered at `03168082d` |
| Machine | Apple M1 Pro, 10 cores |
| OS | macOS 14.6.1 (`aarch64-apple-darwin`) |
| Toolchain | `rustc 1.97.1 (8bab26f4f 2026-07-14)`, `clippy 0.1.97` |

The plan text and the architecture document both referred to a Rust 1.95 CI gate. The repository's
actual toolchain is 1.97.1 and `.github/workflows/release.yml` runs `dtolnay/rust-toolchain@stable`,
so the recorded lanes below are the current truth. The three Clippy lanes were reproduced locally by
cross-checking; `x86_64-unknown-linux-gnu` was installed for this purpose.

### Correctness and lint baseline: green

| Command | Result |
| --- | --- |
| `just validate` | pass |
| `cargo test --workspace` | 4947 + 817 + 17 pass, 0 fail |
| `cargo run -- tests --terse` | 1951/1951 correct in 10.42s |
| `cargo run -- check docs --terse` | no errors or warnings |
| `just bench-ci` | 82/82 cases passed shared preflight |
| `just bench-scaling` | within budget |
| `just timers-erasure-check` | pass |
| Clippy `aarch64-apple-darwin` (`just ci-clippy-native`) | pass, `-D warnings` |
| Clippy `x86_64-unknown-linux-gnu`, same feature set | pass, `-D warnings` |
| Clippy `x86_64-pc-windows-msvc`, same feature set | pass, `-D warnings` |
| `just bench-frontend-check` | pass |
| `just bench-check` | 40/40 cases, `-24ms avg`, 30 faster / 0 slower |

No unrelated failures. The baseline is fully green, so no component had to be reported separately.

### Temporary lint bridge, recorded with its removal owner

Two `#[allow(clippy::result_large_err)]` allowances exist and are the only reason the three Clippy
lanes pass:

| Site | Reason | Removal owner |
| --- | --- | --- |
| `src/lib.rs` | 192-byte `CompilerError` across internal `Result` boundaries | Slice 1G5 |
| `xtask/src/benchmark_execution.rs` | 224-byte `BenchmarkCaseFailure` | Slice 1G5, by shrinking the record rather than relocating the allowance |

No other lint suppression, boxing workaround or compatibility path was added during activation.

### Layout baseline: current predecessor types

Measured on `aarch64-apple-darwin` with `std::mem::size_of` / `align_of` through a throwaway probe
test, which was deleted after recording. The durable layout assertions arrive with the replacement
types in Phase 1.

| Type | size (bytes) | align | Target |
| --- | ---: | ---: | --- |
| `CharPosition` | 8 | 4 | deleted |
| `SourceLocation` | 40 | 8 | `SourceSpan`, 8 |
| `Option<SourceLocation>` | 40 | 8 | `Option<SourceSpan>`, 8 |
| `FileId` | 4 | 4 | `SourceId`, 4 |
| `SourceFileTable` | 56 | 8 | absorbed into the build-lifetime source database |
| `StringId` | 4 | 4 | unchanged |
| `InternedPath` | 24 | 8 | `PathId`, 4 |
| `Option<InternedPath>` | 24 | 8 | `Option<PathId>`, 4 |
| `PathSyntaxId` | 4 | 4 | unchanged |
| `PathSyntax` | 64 | 8 | typed source-local path cold store |
| `PathSyntaxTable` | 24 | 8 | source-owned |
| `StringTable` | 72 | 8 | frozen lookup form |
| `Token` | 64 | 8 | `TokenShape` 8 + `LocalSpan` 4 |
| `TokenKind` | 24 | 4 | `TokenShape`, 8 |
| `NumericLiteralToken` | 24 | 4 | numeric cold store |
| `FileTokens` | 232 | 8 | `SourceTokens` + short-lived `TokenCursor` |
| `DiagnosticKind` | 2 | 1 | `DiagnosticCode`, 2 |
| `DiagnosticPayload` | 112 | 8 | four fact words plus typed extras |
| `DiagnosticLabel` | 72 | 8 | `SecondaryDiagnosticLabel`, 12 |
| `CompilerDiagnostic` | 184 | 8 | `DiagnosticRecord` 32, `DiagnosticDraft` <= 48 |
| `DiagnosticBag` | 24 | 8 | move-only draft accumulator |
| `CompilerMessages` | 120 | 8 | `DiagnosticReport` / `DiagnosticReportSet` |
| `RenderTypeContext` | 640 | 8 | `DiagnosticTypeStore` |
| `TypeEnvironment` | 624 | 8 | not retained for rendering at all |
| `CompilerError` | 192 | 8 | split across the three failure lanes |

Largest inline reason types inside `DiagnosticPayload`, out of the 75 measured in
`diagnostic_payload/types.rs` (max 104 bytes, min 0 bytes):

| Reason type | size (bytes) | align |
| --- | ---: | ---: |
| `InvalidGenericInstantiationReason` | 104 | 8 |
| `InvalidConfigReason` | 40 | 8 |
| `InvalidCallShapeReason` | 32 | 8 |
| `InvalidFunctionSignatureReason` | 28 | 4 |
| `InvalidTypeAnnotationReason` | 28 | 4 |
| `DiagnosticPlace` | 24 | 8 |
| `InvalidCollectionTypeReason` | 24 | 8 |
| `InvalidDeclarationReason` | 24 | 8 |
| `InvalidGenericParameterReason` | 24 | 4 |
| `InvalidMultiBindReason` | 24 | 8 |
| `InvalidReturnShapeReason` | 24 | 8 |
| `InvalidTraitConformanceReason` | 24 | 8 |

`InvalidGenericInstantiationReason` alone sets the 112-byte floor of `DiagnosticPayload`.
`RenderTypeContext` at 640 bytes is the single largest retained-for-rendering record in the
compiler, and it exists only because durable diagnostics keep a full `TypeEnvironment` view.

`CompilerDiagnostic` at 184 bytes and `CompilerError` at 192 bytes are the direct causes of
`clippy::result_large_err`. Slice 1G5's gate is `size_of::<CompilerDiagnostic>() <= 128`.

### Migration inventory

Full searchable inventories are generated under `target/data-layout-audit/` and are deliberately not
committed. Concise summary:

| Inventory | Scale | Highest-risk owners | Owning phases |
| --- | --- | --- | --- |
| `01-source-locations.md` | 2 location primitives, 2 file-identity structs, 5 `PreparedSourceInput` variants, 6 identity/remap gateways, 271+ location-bearing records | `SourceLocation`/`CharPosition` with interned scope and overlap-as-`Equal` ordering; two coexisting ID systems (`FileId`/`SourceFileTable` and `SourceId`/`SourceTreeIndex`) with provisional-to-final rebinding; `FileTokens` -> `FileFrontendPrepareOutput` -> `Header` identity chain | 1, 2, 3 |
| `02-paths.md` | 301+ `InternedPath` rows (298+ in `compiler_frontend`), 24 inherent methods, 90+ path-keyed maps/sets, 0 `Box<Path>` | `symbols/interned_path.rs`; `paths/path_resolution.rs` mixing logical spelling with physical candidates; `headers/module_symbols.rs` prefix rebinding; `hir_side_table.rs` embedding `InternedPath` in location keys | 2, with token path rows in 3 |
| `03-tokens.md` | 94 `TokenKind` variants (8 payload-bearing), 8 `FileTokens` fields, 31 files with token-vector shapes, 141 files referencing `FileTokens` | `tokenizer/tokens.rs:241-617` coupling storage, path-table lifecycle, identity, stats, cursor and remap; `headers/types.rs:404-1451`; `generic_functions/materialisation/frozen_syntax.rs` full-stream cloning; duplicated classification matches in `parse_expression_dispatch.rs` and `body_dispatch.rs` | 3, with token-bearing diagnostics in 4 |
| `04-diagnostics.md` | 8 category wrappers, 147 stable descriptors, 127 `DiagnosticPayload` variants, 74 supporting enums, 11 label-message variants, 29 diagnostic clone sites, 72 files referencing `Box<CompilerDiagnostic>`, 38 files cloning `StringTable`, 21 `with_type_context_for_all_diagnostics` callsites | primary-location duplication into the primary label; the exhaustive payload remap/rebind walkers; `CompilerMessages.render_type_contexts` keyed by diagnostic index ranges that survive prepend/append | 4, with span work in 1 |
| `05-failure-lanes.md` | 1 `CompilerError` struct, 6 `ErrorType` variants, 7 `CompilerErrorMetadataKey` variants, 16 + 151 + 1 macro callsites, 250+ `Result<_, CompilerError>` boundaries, 2 lint allowances, 4 `catch_unwind` sites, 8 poisoned-lock recovery sites, 0 panic hooks, 2 `panic = "abort"` profile settings (root `Cargo.toml`, release and profiling) | `compiler_errors.rs:511-716` mixing message, interned identity, category, metadata and an optional `StringTable`; 151 `return_hir_transformation_error!` callsites in `hir/`; dev-server poisoned-state recovery in `build_loop.rs`/`state.rs` | 5, 6 |

Two counts in the generated `05-failure-lanes.md` inventory were wrong and are corrected above:
`CompilerErrorMetadataKey` has seven variants, not six, and the repository has two `panic = "abort"`
settings, not three. The generated file under `target/` retains the original figures.

### Stale plan facts corrected at activation

- The plan and the architecture document named a Rust 1.95 CI gate. Current toolchain is 1.97.1 and
  CI pins `stable`.
- The plan referenced `benchmarks/cases.txt` and `benchmarks/frontend-cases.txt`. No case-list text
  files exist. `benchmarks/manifest.toml` is the corpus authority and declares every `[[workload]]`
  and `[[case]]`; `xtask/src/benchmark_manifest.rs` is only its parser, and `BenchmarkSuiteKind` in
  `xtask/src/bench_types.rs` owns suite selection and history identity. The planned data-layout
  suite therefore becomes a `data_layout` case group in `benchmarks/manifest.toml` plus a third
  `BenchmarkSuiteKind`, not a `benchmarks/data-layout-cases.txt` file.
- The plan schedules the deletion of `PathTokenItem` and says `TokenKind` is widened by
  `Path(Vec<PathTokenItem>)`. That migration already happened: `PathTokenItem` does not exist and
  `TokenKind::Path` already carries a dense `PathSyntaxId` into a file-owned `PathSyntaxTable`.
  The architecture document's current-to-target row repeated the same stale claim.
- The plan named `src/build_system/build.rs::InputFile` as the duplicate source-text/path carrier.
  No `InputFile` type exists; the current carriers are the five `PreparedSourceInput` variants in
  `src/build_system/create_project_modules/prepared_source.rs` plus the frontend source variants in
  `src/compiler_frontend/pipeline.rs`.
- `docs/compiler-data-layout-design.md` still carried the pre-activation audit anchor
  `d119988861aad9732c19d945eeabeb249a7e5caa` and a conceptual context example that placed
  diagnostics inside the frozen context. Both were corrected in this phase.

### Source-size census: the `LocalSpan` start-bit gates

Scope: every compiler-visible source (`.moth`, `.mtf`, `.js`) under `benchmarks/`, `docs/src/` and
`tests/`. Markdown and the evidence report itself are excluded because they are not compiler input
and this report is modified by the census that would measure it.

4584 files, 2,354,191 bytes total, median 77 bytes, p95 1,318 bytes, p99 9,002 bytes. The largest
source is `benchmarks/nominal-scaling/nominal-scaling-320.moth` at 92,557 bytes, followed by
`docs/src/developer-docs/memory-management/boracle/boracle-operational-oracle.mtf` at 78,392 bytes.

| `LENGTH_BITS` | Inline start range | Sources at or over the limit |
| ---: | ---: | ---: |
| 8 | 16 MiB | 0 |
| 9 | 8 MiB | 0 |
| 10 | 4 MiB | 0 |
| 11 | 2 MiB | 0 |
| 12 | 1 MiB | 0 |

No candidate split suffers a single start overflow on the current corpus - the largest source is
under 0.1 MiB, which is more than an order of magnitude below the tightest candidate's 1 MiB start
range. Selection is therefore decided entirely by length overflow, which needs exact byte offsets
and is measured once Slice 1C3 supplies them. The architecture's default 22/10 split stands until
that census runs.

### Five-run repeatability, September 4th

Recorded runs (`just bench-frontend`, `just bench`, `just bench-data-layout`) each measure ten
iterations per case and keep the median. Repeatability is then measured across five independent
non-recording invocations against that stored baseline:

| Suite | Run 1 | Run 2 | Run 3 | Run 4 | Run 5 | Median |
| --- | --- | --- | --- | --- | --- | --- |
| Frontend phases, 42 cases | +1ms | 0ms | 0ms | -1ms | -1ms | 0ms |
| End-to-end CLI, 40 cases | 0ms | 0ms | 0ms | 0ms | 0ms | 0ms |

Every run reported "no measurable change" over the full case set, so the noise floor is +/-1ms on the
suite average. A later phase's regression claim must exceed that band on the same machine.

Recorded suite averages at this baseline: frontend phases all ~155ms (Core ~34ms, Docs ~1822ms,
Stress ~176ms, Module ~31ms, Parallelism ~24ms, Borrow ~17ms); end-to-end CLI all ~57ms; the
diagnosed data-layout pair ~33ms average, with directory compile ~36ms, frontend ~25ms and boundary
compile ~20ms as its top stages.

### Work explicitly deferred out of Phase 0, with its owner

| Deferred | Owner | Reason |
| --- | --- | --- |
| Exact span start/length histograms with per-candidate boundary buckets | Slice 1C3, consumed by 1C1 | The current model stores line/column, not byte offsets, so the census cannot run against today's spans. Phase 1's slice order is corrected so the byte cursor and line index (1C3) land before the encoding is selected (1C1); the census then runs against real offsets instead of a throwaway tracker. |

This deferral does not block Phase 1; it is recorded on its owning slice in the plan, and the plan's
Phase 1 slice order was changed in this phase so it is actually executable.
