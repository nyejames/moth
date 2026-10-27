# Compiler diagnostic recovery and budgets implementation plan

> For coding agents: implement one reviewed slice at a time. Use the repository's execution workflow, with `superpowers:executing-plans` or `superpowers:subagent-driven-development` when available. Checkboxes record work, not permission to skip the phase gates.

**Goal:** report a small set of trustworthy independent errors without materially slowing successful compilation or building a general semantic recovery system.

**Architecture:** fail fast inside an ordinary concrete function body, then optionally check another independently usable body. Reuse compiler-owned diagnostic storage, module transactions and build-owned scheduling. Bound recovery with a strict module allowance, a separate AST work guard and a soft command-wide scheduling threshold.

**Tech stack:** the existing Rust compiler, retained source tokens, typed expression/TIR owners, diagnostic infrastructure and repository test/benchmark tools. Add no dependencies.

**Spec:** the maintainer-approved interview contract in `Accepted contract` below. Publish its durable rules into the named permanent authorities before implementing them. This document contains the decisions needed by an agent without access to the conversation.

## Status and activation

- Status: queued after diagnostic improvements, design interview confirmed.
- Current slice: not started.
- Blockers: compiler diagnostics improvements are ahead in the roadmap. Compact diagnostic records, type-display snapshots, three failure lanes and canonical diagnostic aggregation are delivered.
- Next action: refresh the activation tree and run Phase 0 after diagnostic improvements complete.

The main roadmap places this work after compiler source/token/diagnostic data layout and diagnostic improvements. The later general-directives/configuration cutover is not a prerequisite. Deliver the two settings using the configuration system present at activation, then preserve them through that later cutover.

The repository inspection used for authoring found the compact typed-expression foundation merged, its later reductions awaiting merge and the source-token and diagnostic-layout migration still gated. This is navigation context, not current implementation evidence when this plan activates. Record the actual starting revision and toolchain in working notes. Do not pin a future implementation baseline in this status block or reactivate another plan implicitly.

## Required reading

Read `AGENTS.md` and the complete style guide at `docs/src/developer-docs/style-guide/style-guide.mtf`. Read `docs/compiler-design-overview.md`, `docs/build-system-design.md` and `docs/compiler-data-layout-design.md` in full because this work crosses stage, storage and orchestration boundaries.

Also read:

- `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf` and the canonical language references routed by `docs/src/developer-docs/language/overview.mtf` for calls, parameters/defaults, constants, exact numerics, errors and templates touched by recovery.
- `docs/src/docs/project-structure/project-config.mtf` and `build-inputs.mtf` for configuration syntax and bootstrap ownership.
- The testing guide's admission/ownership, diagnostic assertions, concurrency, fixtures and benchmark sections, plus the validation guide's scope, documentation, performance and failure-handling rules.
- `docs/roadmap/roadmap.md`, both progress matrices and `index.md`. Use the index as a locator, not a semantic authority.
- The memory overview and routed borrow-validation reference when recording the deferred Stage 6 handoff. Research activation and solver changes remain outside this plan.

Follow the most specific permanent authority and explicit maintainer decisions. Resolve a genuine later conflict before coding. Replace moved navigation anchors in working notes rather than restoring removed APIs. Historical `CompilerMessages`, `CompilerError` and parser shapes below are not implementation targets after diagnostic layout finishes.

## Accepted contract

### Recovery boundaries

1. Keep each failed ordinary concrete function body fail-fast. Report its first primary error with useful related labels, then abandon the remaining body.
2. Continue only with another body whose required facts remain trustworthy. Use existing sorted headers, signatures, visibility and bounded token cursors. Do not rescan for a function boundary or recover statement by statement.
3. A valid signature remains usable when its body fails. A caller's wrong argument type can therefore be independent. Withhold checks that require unavailable body-derived failure, effect or other semantic facts. If that distinction needs substantial tracking, stop the affected recovery path instead.
4. Module environment construction, declaration/signature completion, generic-template validation, generated materialisation, const-template producers and implicit `start` retain their existing fail-fast boundaries. Body-local nested constructs are part of their containing body, not extra recovery roots. A concrete receiver method is eligible only through the same ordinary concrete-function owner and isolation proof.
5. Templates or generic calls inside an eligible ordinary body do not automatically make it a new recovery category. Their shared mutations still require the isolation proof below. No separate recovery inside template or generated-function compilation is added.
6. Any AST error rejects the complete module. Skip module AST finalisation, HIR lowering, borrowing, generated completion and later stages that require a valid AST. No successful partial module, interface, generated sidecar or semantic resource lane escapes.
7. Preserve earlier collected diagnostics when a later source failure ends recovery, within the module allowance. Operational failures and compiler bugs still abort their owning compilation through the delivered separate lanes. Budget exhaustion never conceals them. Source-capacity exhaustion or a failed shared-store construction also ends its owning stage even when it is correctly classified as a user diagnostic.

After the first body error, optional diagnostic discovery admits only eligible bodies. Headers needing no further semantic work can be passed over. If proceeding requires a non-eligible semantic producer or an unavailable prerequisite, end recovery there. This is a deliberately conservative cutoff, not authority to run arbitrary AST work to reach a later function.

### Configuration and counting

| Setting | Source type | Omitted default | Present value | Explicit `none` |
|---|---|---|---|---|
| `max_errors_per_module` | `I32?` | `5` | Strict module primary-error allowance | Disable this count limit |
| `max_errors_per_project` | `I32?` | `30` | Soft project-request scheduling threshold | Disable this threshold |

Present values must be in `1..=2147483647`. Preserve ordinary exact-type checking. Reject zero, negatives, out-of-range values and incompatible types with the configuration owner's typed diagnostics. Omission and explicit `none` are different. Both settings are independently removable. Do not introduce a magic numeric unlimited value, project-size scaling or an automatically divided quota.

Validate once through the existing schema and config application path. Use compact compiler-owned options, such as `Option<NonZeroU32>` after validation, without putting the project configuration object into the frontend. The frontend receives the module limit. The command/scheduler owns the project threshold. Do not allocate storage in proportion to a user-supplied maximum.

These are compiler-control settings. Preserve the existing fixed-only configuration policy for such controls. They do not create implicit source input contracts, new semantic types or source-visible scheduling choices. Keep these new controls out of `@project` semantic metadata and semantic fingerprints so changing a diagnostic limit cannot change program meaning or invalidate successful semantic artefacts. Audit any existing automatic config-field projection instead of assuming that separation already exists.

Count primary user errors only. Warnings, attached notes, secondary labels, blocked dependants and stop metadata consume no slots. This plan does not create a warning-budget system or change ordinary warning policy. A shared provider diagnosis counts once, not once per consumer. Generated failures belong to the requesting module transaction, not a fresh allowance. Check-only units attributed to one canonical module share its allowance rather than resetting it for every orphan file.

The module allowance spans that module's attempted stages and preparation lanes. It does not reset at each file, function, stage or generated request. Serial recovery stops when the allowance is exhausted. Parallel file preparation may generate speculative extras in already-running jobs. Merge in canonical source/diagnostic order and retain no more than the module allowance, preserving complete diagnostic groups and their identity contexts.

### Soft project cutoff and concurrency

Before and during module discovery, use the project threshold without inventing module identities for early errors. Bootstrap uses the compiler default until valid project settings exist. Apply the validated settings before semantic scheduling. Invalid configuration cannot partially install its own limits. Fresh requests and reloads reset counts and omitted settings to their defaults.

After discovery, each module independently applies its allowance. The invoking command counts returned primary errors across its selected project and package work once. A dependency package's own module setting may govern its modules, but its project threshold cannot reset the invoking command's total. Config/bootstrap diagnostics have no manufactured semantic module owner.

At a normal scheduler admission boundary, if the accumulated returned error count meets or exceeds the project threshold, stop admitting new compilation work. Already-running jobs complete under their own policies and return normally. Retain all their module-bounded results. The project total may exceed 30 even in serial execution when one returned batch crosses the threshold. **There is no hard project result cap, global error quota, project-level truncation or forced cancellation.**

For example, a returned total of 28 followed by a module containing five retained errors yields 33 retained errors. Stop admitting further modules, but retain results from other jobs already admitted. Do not reduce that module's allowance to two or discard its last three errors.

Use existing scheduler ownership and deterministic result merging, not atomics or locks in tokenizer, expression or diagnostic hot paths. The threshold is checked between actual work admissions, not after eagerly enqueueing the entire remaining project and calling that fail-fast. A cheap job description is not running semantic work.

Parallel admission must remain deterministic. Settle existing deterministic work groups and merge their results canonically before threshold-dependent admission of another group. Future parallel-module scheduling must define deterministic group membership and admission independently of completion races. Do not add that scheduler here. If such a scheduler exists at activation, integrate with its real admission/settlement contract and test it. Sorting race-selected diagnostics afterwards is insufficient.

Return and finalise all live source-span owners, identity deltas and diagnostic contexts even when results are omitted by a module cap or later work is not scheduled. Keep existing generated-request deduplication and atomic publication. Normal successful module scheduling remains unchanged apart from cheap policy state at its owner.

### Deterministic AST recovery-work guard

Include this guard in the implementation, not just as a possible follow-up. It applies independently of both configurable error counts and starts only after the first eligible AST body diagnosis.

Use already-retained body-token range or sequence lengths as a deterministic work proxy. Charge the full length before admitting each additional body, including a body that turns out to be valid or stops early. Stop when the next body would exceed the remaining allowance. Do not skip an oversized body to search for smaller ones later, reorder functions, inspect tokens twice or add per-expression clock checks.

The first failed body belongs to normal compilation and is not charged as extra recovery work. The guard does not interrupt a body already admitted. Existing numeric, template, allocation and recursion limits remain in force. Token accounting bounds additional admitted syntax, not wall-clock time, expansion volume or all semantic complexity. Benchmark costly templates and generics separately.

Select one compiler-owned positive token allowance through Phase 2 measurements and document the evidence. It is not project-size dependent and has no new public configuration setting. `none` on either error-count setting does not disable this work guard. Use synthetic small allowances in tests, not production values chosen to satisfy a fixture.

### Lightweight stop metadata and publication

Extend existing outcomes with the smallest useful budget-stop metadata. Distinguish module error allowance, AST recovery-work guard and project scheduling cutoff. Store it beside reports/outcomes, never in each expression, token or common diagnostic record. Normal success should allocate nothing for this metadata.

A budget-stopped request, a diagnosed module and a dependency-blocked module have different meanings. Do not mark unvisited work successful or invent a source error for it. Prefer request-level stop state plus module-local facts only where needed. Introduce a per-slot not-checked state only if the graph's existing finalisation invariants actually require it.

Current graph validation expects final module outcomes. Audit and update that proof explicitly. A partially visited graph cannot pass the complete-graph validator unchanged, and a stopped graph cannot become `ProjectCompilation`. A stopped flag is valid only after a real user error and actual skipped work. Reaching a numeric limit exactly on the last available unit need not claim further work was skipped.

Metadata describes shortened diagnostic discovery, not successful completion of every later compiler stage. Even an uncapped AST diagnosis leaves downstream validation unperformed. Preserve that distinction in tooling APIs. Renderers can report known hidden generated diagnostics separately from code never checked. Do not claim a count of undiscovered errors.

### Incremental and tooling boundaries

Limits are per request. Reused successful immutable modules consume no error allowance. Never use an old successful artefact for a changed module whose current compilation failed or was skipped. Independent valid successes may remain reusable under the existing publication/cache contract, but the failed request remains failed and emits no partial project output.

Changing limits does not invalidate successful semantic artefacts. Future diagnostic replay must validate source/dependency identity, policy and coverage separately. A report truncated at five errors is not complete after the allowance is raised. Cached diagnostics that are legitimately replayed count once in the new request. Do not implement diagnostic caching or a semantic cache in this plan. Test real reuse only when it is present and demonstrable at activation, otherwise record the integration as deferred.

CLI/LSP display limits remain separate. The proposed display default of about five and an all-generated-errors option are later tooling work, not new flags in this plan. Consume lightweight stop metadata through existing report paths, with only the minimum truthful incomplete-check indication needed by those paths. An all-generated-errors view never promises analysis the compiler skipped.

## Global engineering constraints

- Prefer deletion, moves and existing owners over new wrappers, registries or passes. No generalized recovery framework, placeholder/error types, speculative type facts or new whole-function dependency graph.
- No whole environment, expression store, TIR store or identity-table clone per body or per recovered error. Capture display facts through the delivered diagnostic owner while the required types and identities are live.
- Local recovery decisions use stage/ownership facts, not rendered prose, severity alone or a global table mapping diagnostic codes to recovery policies.
- Retain one parser, constant evaluator and canonical module service. No diagnostic-only compilation pipeline and no partially valid HIR.
- Scope cleanup or a small append checkpoint is allowed only with a proved lifetime contract. Broad rollback, per-body full snapshots and successful-path copy-on-write solely for recovery are outside the cost budget.
- No runtime or code-generation behaviour changes. Ordinary source validity, evaluation order and accepted language rules are unchanged.
- Performance claims need matched evidence. Do not disguise extra checking as a speedup or use diagnostic formatting improvements to hide slower recovery.

## Review focus

- A failed body leaves expression, constant, TIR or generic-ticket state that a later body can observe. Phase 2 isolation tests own this risk.
- A caller has a valid signature but lacks callee-body facts. Phase 2 tests require the independent argument error without a secondary inferred-failure diagnostic.
- Several preparation workers fail in one module. Phase 3 tests require deterministic module truncation and live source contexts.
- A returned batch crosses the soft project threshold while other jobs run. Phase 3 tests require retained overshoot, stopped admissions and honest graph outcomes.
- Omitted, absent, invalid or reloaded limits acquire the wrong meaning. Phase 1 and Phase 4 tests require independent defaults, request reset and no semantic configuration leakage.

For each slice, add or extend the owning regression first, run it against the predecessor to demonstrate the relevant failure, implement the smallest change, rerun it and review before committing. Typical focused commands are `cargo test -p moth diagnostic_limits`, `cargo test -p moth diagnostic_recovery` and `cargo test -p moth diagnostic_stop_metadata`. Require a nonzero executed test count. Register public scenarios in the existing manifest and run their selected `moth tests --case` entries through the current runner. Do not create new validation recipes merely to rename these commands.

## Starting owner map

These are inspected navigation anchors, not fixed APIs for the post-layout implementation. Phase 0 replaces moved anchors with their current owners.

| Responsibility | Starting locations and observed shape |
|---|---|
| Module AST sequence | `src/compiler_frontend/ast/mod.rs` runs environment construction, emission and finalisation. |
| Body boundary | `ast/module_ast/emission/emitter.rs` propagates errors out of `emit()` and returns rich boundary messages from `emit_function()`. Each header already owns bounded syntax. |
| Expression and constant lifetime | `ast/module_ast/build_context.rs`, `ast/module_ast/finalization/` and `ast/mod.rs`. The merged compact typed-expression foundation gives `AstPhaseContext` shared expression and constant stores beside TIR. |
| Shared body services | `ast/module_ast/scope_context.rs`, its `builders.rs` and `lookup.rs`, `ast/type_interner.rs`, `ast/statements/body_dispatch.rs` and numeric completion owners. Inspect generic tickets and completed-body infallibility facts. |
| Diagnostic storage | `src/compiler_frontend/compiler_messages/` and the data-layout authority. Reuse the delivered replacement for predecessor bags, reports and render contexts. |
| Module transaction | `src/compiler_frontend/module_compilation/{options,service,outcome,stages}.rs` and generated completion. |
| Configuration | `src/builder_surface/{definition,config_schema}.rs`, `src/build_system/project_config.rs`, `project_config/validation.rs`, `src/projects/settings.rs` and `src/compiler_frontend/project_globals.rs`. |
| Preparation/discovery | `src/build_system/create_project_modules/{module_preparation,source_preparation,source_discovery,module_inventory}.rs`. Distinguish indexed discovery from parallel preparation. |
| Scheduling and outcomes | `create_project_modules/compilation/{canonical,single_file}.rs`, `compilation.rs`, `module_artifact_store.rs` and `compiled_boundary.rs`. Semantic module publication was serial at inspection. |
| Public command/report consumers | `src/projects/{check,cli}.rs`, dev-server report paths and the delivered diagnostic renderers. |
| Evidence | `benchmarks/README.md`, `benchmarks/frontend-optimization-results.md`, existing frontend instrumentation and `justfile`. |
| Deferred borrowing | `src/compiler_frontend/analysis/borrow_checker/{engine,metadata,types}.rs` and module generated convergence. Use the existing living production companion for follow-ups. |

Use `src/compiler_frontend/ast/tests/` for local recovery/isolation invariants and the diagnostic owner's tests for scalar budget rules. Public behaviour belongs in manifest-backed `tests/cases/`. Use `src/compiler_tests/` only for cross-stage result, graph or scheduling assertions that the manifest cannot express. Extend existing primary owners before creating `diagnostic_recovery_tests.rs` or a `diagnostic_recovery` harness module. Do not move unrelated legacy tests during this work.

## Phase 0 - Activation, authority and baseline

**Deliverable:** a current owner/isolation inventory, contract updates and a reproducible comparison baseline. No recovery enabled yet.

- [ ] Confirm the prerequisite capabilities are merged. Read every required authority and inspect the actual default/feature paths. Record revision, Rust/Node versions, compiler build, host, thread settings and known baseline failures in working notes.
- [ ] Recheck expression ownership, diagnostic snapshots, module outcomes, source preparation, generated completion, scheduler concurrency and actual cross-build reuse. Distinguish a declared capability from an executing implementation.
- [ ] Publish the accepted recovery policy in the compiler architecture, scheduling/counting rules in the build architecture and configuration contract in the canonical project-config reference. Keep storage details under the data-layout authority. Preserve existing independent contracts rather than rewriting those documents wholesale.
- [ ] Inventory every mutation reachable from an eligible body: scope/numeric scratch, expression builder, constant store, TIR views/overlays, type interning/cache, generic-request ledger, evidence and infallibility facts, resource uses, warnings and source spans. Classify each as body-local discard, complete append-only fact, small rollback with proved ownership or unsafe-to-continue.
- [ ] Prove that retained valid facts cannot expose failed-body state to the next body. Audit cache insertion order and in-progress markers, not just container ownership. Dynamic TIR expression IDs and generic ticket positions are particularly sensitive.
- [ ] Identify body-dependent diagnostic sites. Prefer the already-required omission of later finalisation to new suppression state. A small failure-path-only identity set or existing readiness state is acceptable when needed. No new graph or clean-path tracking pass is authorised.
- [ ] Inspect the canonical test inventory and select existing primary cases to extend. Characterise the current first-error result and stage cutoff before changing it.
- [ ] Use existing non-recording benchmarks to record clean modules, first-body failure with valid suffix, several independent errors, templates/generics and multi-module fanout. Measure compiler execution separately from building the Rust compiler.

### Phase 0 audit, style review and validation

- [ ] Audit the owner matrix against actual writers/readers, source identities and report lifetimes. A body is not isolated merely because its local scope drops.
- [ ] Perform the Slice/style review. Reject new abstractions that only wrap current owners and remove unsupported assumptions inherited from predecessor APIs.
- [ ] Run the activation `just validate` and affected baseline lanes. Use the documentation release-build gate for authority changes. Record exact failures and unrun checks. A broken baseline is not permission to weaken gates.
- [ ] Approve only the smallest viable recovery boundary. If ordinary bodies need broad snapshots, a new dependency graph or uncertain rollback, stop the recovery slice and record the blocker. Budget/config work may proceed independently, but recovery is not marked delivered.

## Phase 1 - Limits, configuration and report contract

**Deliverable:** one validated policy projection and lightweight stop metadata, with existing compilation behaviour otherwise preserved.

- [ ] Add focused tests `diagnostic_limits_defaults_and_none` and `diagnostic_limits_invalid_values`. Assert omission gives 5/30, each `none` is independent, 1 and I32 maximum are valid, and zero/negative/wrong-type/out-of-range values fail without proportional allocation.
- [ ] Extend the current project schema, config application and options projection. Use the exact public names above. Keep defaults in one owner and keep the two controls out of semantic globals and fingerprints. Update a real config fixture through the current syntax, not only a constructed Rust options value.
- [ ] Add `diagnostic_limits_reload_resets_state`: set custom limits, reload with omissions, then explicit `none`. Counts reset per invocation and settings do not leak from a previous successful or failed load. Test invalid bootstrap with default project policy.
- [ ] Extend the delivered diagnostic/outcome owner with cheap primary-error accounting and stop reasons. Reuse an existing count if authoritative. Do not repeatedly scan a growing warning/error list or widen common diagnostic records.
- [ ] Add `diagnostic_stop_metadata_preserves_failure_lanes`: budget metadata is not a source diagnostic, warnings/labels consume no allowance, an infrastructure failure remains fatal and a compiler bug is never demoted.
- [ ] Thread policy through named config, module, direct-template and synthetic services only where each needs it. A source-backed module has one allowance across its producers. Independent identity domains retain separate reports even when the command adds their counts.

### Phase 1 audit, style review and validation

- [ ] Audit omission versus explicit absence, full I32 boundaries, bootstrap ordering, config reloads and source-visible projection. Verify the project threshold has not become a per-module remaining quota.
- [ ] Review type sizes, allocation sites and ownership. Remove obsolete option/constructor paths rather than leaving defaulted adapters. Empty policy/report state allocates nothing.
- [ ] Run the focused budget/config tests and selected manifest cases, `just fmt-check` and `just clippy`. Run `just validate` at this API integration boundary and the relevant documentation gate. Require nonzero selected test counts.
- [ ] Record matching clean-path timing/allocation evidence before enabling recovery. The compact diagnostic architecture's layout assertions still pass.

## Phase 2 - Bounded ordinary-body AST recovery

**Deliverable:** useful independent body diagnostics with no invalid semantic handoff and a measured recovery-work allowance.

- [ ] Add `diagnostic_recovery_independent_bodies`. Use valid predeclared signatures with an unknown local in one body and a bad argument/return type in another. Assert the ordered complete primary-error set, exact source facts and no consequential errors from the first body.
- [ ] Return the delivered narrow typed body failure to the emitter. Replace eager per-body report construction with local accumulation and one terminal diagnostic handoff. Preserve all attached source/type evidence. Infrastructure/bug paths stay distinct.
- [ ] Keep the normal first-body path unchanged. On diagnosis, drop that body's construction roots and perform only the isolation action proven in Phase 0. Complete unused interned entries may stay until module discard. Unresolved cached entries, exposed generic requests or mutated shared templates may not affect later reads.
- [ ] Avoid blanket vector truncation: existing AST roots, TIR overlays, type-display facts or ticket ranges may still address the suffix. Prefer leaving safe append-only entries until the failed module drops. If this cannot be proved, retain fail-fast behaviour for the affected path.
- [ ] Add `diagnostic_recovery_failed_callee_signature`. A valid signature still produces an independent wrong-argument diagnostic in another body. A check needing unavailable callee-body facts produces no follow-on error. No fake successful or infallible callee is installed.
- [ ] Add `diagnostic_recovery_shared_state_isolation`. Exercise numeric completion, TIR/slots or wrappers, generic calls and resources before a body fails, then an independent later body. Assert no stale local lookup, ticket collision, phantom generated request or changed valid signature. Use the smallest cases that distinguish the actual mutation paths.
- [ ] Enforce the module count before admitting another body. Retain one primary diagnostic per failed body. Preserve earlier errors if a later nonrecoverable source failure is returned. Fatal operational/internal failures terminate normally even after user errors exist.
- [ ] Implement the token work guard at this same boundary. Add `diagnostic_recovery_work_boundary` with a test allowance and body lengths for exact fit, next-body overflow and a successful body consuming the allowance. Test disabled count limits while the work guard remains active.
- [ ] Keep non-eligible producers fail-fast and end optional discovery when one must run. Add `diagnostic_recovery_scope_boundary` for a generic template, implicit start or const-template boundary without enabling recovery inside it.
- [ ] Add `diagnostic_recovery_skips_later_stages`. Once emission has any errors, return the diagnosed outcome without finalisation, generated materialisation, HIR, borrow or backend entry. Check root/resource/publication absence through the canonical service, not a fake parser success.
- [ ] Compare at least three candidate positive token allowances using measured body sizes. Choose the smallest that provides the tested useful independent-error coverage within the performance and memory gates below. Record the chosen constant, workload coverage, extra work and rejected alternatives. Production selection remains fixed, not a runtime project-size heuristic.

### Phase 2 audit, style review and validation

- [ ] Audit every shared-state classification and failure path after the actual patch. Inspect calls to failed bodies, nested templates, numeric deferrals and all stores retained by diagnostics. No live identity or display fact is dropped early.
- [ ] Perform the Slice/style review, explicitly counting new helpers, structs, scans and clean-path allocations. Delete superseded eager diagnostic wrappers at the affected boundary. Retain no second parser or recovery implementation.
- [ ] Run focused AST/isolation tests, public negative cases and repaired-source positive controls. Run `just validate` at the phase boundary and affected feature lanes under the current validation guide.
- [ ] Meet the clean-build, limit-one and bounded-work performance gates. If extra checking is too costly, reduce the admitted recovery scope/allowance and repeat. Record a failed feasibility gate honestly instead of silently accepting cascades or widening the cost budget.

## Phase 3 - Preparation, scheduling and incomplete graph results

**Deliverable:** module allowances apply consistently, the project threshold stops new admissions and all retained results keep valid contexts.

- [ ] Extend existing preparation aggregation with `diagnostic_recovery_parallel_preparation`. More than five malformed independent files produce the same first five retained diagnostic groups in canonical order with one or several workers and deliberately reordered completion. Retain and finalise every returned span owner, including discarded diagnostic producers.
- [ ] Keep already-running preparation workers independent. Apply the cap before later semantic stages are admitted. Where preparation admits further chunks incrementally, stop admissions at existing deterministic settlement boundaries rather than enqueueing the suffix then discarding everything.
- [ ] Test canonical and check-only units sharing one module allowance. Provider blocking follows real required interfaces. A filename, stage transition, generated request or orphan unit cannot reset the count.
- [ ] Add `diagnostic_recovery_project_soft_cutoff`. Inject a small threshold into the real scheduler owner and return module-sized batches that overshoot it. Assert all returned module-bounded diagnostics remain, queued work is not started, already-admitted work finishes and warnings/blocked dependants do not advance the count. Cover `none` and a shared failed provider counted once.
- [ ] Charge results once at the command-owned aggregation/admission boundary, including selected source-package work. Preserve one total through package transitions and fallback discovery. Keep module limits independent even near the threshold.
- [ ] Audit every admission path: directory, synthetic single-file, packages, check-only work and later validation jobs that the command would otherwise schedule. Stop new optional work after the cutoff without skipping required cleanup or disguising a fatal failure.
- [ ] Update graph/result validation for intentional incomplete visitation. Prefer existing outcome state plus lightweight stop metadata. If untouched slots require a new explicit state, add one at their current owner with a documented invariant, not a second graph or per-function registry.
- [ ] Add `diagnostic_recovery_incomplete_graph_is_not_success`. Skipped slots cannot satisfy providers, linkability or backend gates. A failed provider blocks dependants without source cascades. Independent completed successes remain internally valid and correctly paired with contexts.
- [ ] Preserve canonical ordering before threshold-dependent scheduling decisions. For existing parallel execution, test reordered completions through the actual scheduler. For future module parallelism, document the required deterministic admission/settlement rule without adding a scheduler prototype or claiming nonexistent concurrency coverage.

### Phase 3 audit, style review and validation

- [ ] Audit source-owner return paths, string/path remaps, discarded reports, generated deduplication, package counts and graph completion proofs. A soft threshold is not implemented as result truncation or a global worker quota.
- [ ] Review scheduler and report changes for unnecessary queues, atomics, locks, scans and shadow counters. Keep the success-only project boundary intact.
- [ ] Run focused preparation/graph/command tests, real multi-module manifest cases and serial/parallel preparation variants. Run `just validate`, applicable feature lanes and platform-sensitive source-context cases.
- [ ] Measure clean scheduling, early cutoff, late cutoff and parallel overshoot. Record both wall-clock time and total admitted/executed work. Report paths still limited by existing discovery or batch granularity rather than promising instantaneous cancellation.

## Phase 4 - Integration, performance acceptance and documentation

**Deliverable:** a validated implementation with truthful status, preserved future handoffs and no temporary recovery machinery.

- [ ] Exercise `check` and `build` through their public paths with default, custom and absent limits. A diagnosed build writes no new partial project. Synthetic operations retain their documented config policy. The standalone MON data-service error contract is unchanged.
- [ ] Verify stop metadata reaches the current report consumers without stale spans or overclaiming coverage. Distinguish frontend display truncation from module generation truncation and project scheduling termination. Add no five-error display cap or all-errors command here.
- [ ] Add `diagnostic_recovery_request_policy_isolation`: changing limits or running a new request does not leak counts or old failed state. If real module reuse exists, prove an actual cache hit before testing non-invalidation and failed-provider rejection. Otherwise leave concrete incremental integration as a named deferred test, not a simulated cache or supported matrix claim.
- [ ] Run the full performance matrix below against the post-layout activation baseline and the limit-one mode. Preserve acceptance criteria and existing scaling budgets. Move concise evidence into the existing indexed benchmark report and keep raw captures untracked.
- [ ] Complete the exact documentation/status/handoff changes in the next section. Rebuild generated documentation through the compiler when its source changes. Never hand-edit `docs/release/**`.
- [ ] Remove temporary probes, unused policy paths, obsolete wrappers and duplicate tests. Keep only instrumentation justified by ongoing regression coverage and erase its overhead in ordinary builds.

### Phase 4 audit, style review and validation

- [ ] Perform a whole-change correctness and anti-cascade audit from config/bootstrap through report lifetime and project publication. Recheck every accepted contract and exact test set, including deliberately unsupported recovery roots.
- [ ] Perform the final Slice/style review. Justify every new abstraction, allocation and traversal. Confirm no speculative dependency graph, error type, semantic cache, parallel scheduler or source feature was added.
- [ ] Run `just validate-full` on the final integrated tree before merge, plus the documentation release build and specifically affected opt-in lanes. Report tested revision/configuration and all failures. Existing policy requires fresh evidence if the landing tree changes. Boracle is run only when its actual inputs or boundary coverage were affected, not as an implied research activation.
- [ ] Record residual restrictions and deferred work before retiring this plan. Delete it and remove its roadmap bullet in the same completion commit only after delivered behaviour and accepted residual scope are recorded in permanent owners.

## Performance evidence and acceptance

Use the existing benchmark engine, metric schema and `benchmarks/frontend-optimization-results.md`. Add one indexed recovery section, not another runner or permanent evidence document. Raw profiles belong in untracked `tmp/` or the existing report output locations.

Compare identical compiler revisions except for the patch, release/profile/features, source corpus and thread settings. Start with at least five interleaved measured runs after equivalent warm-up. Record medians, spread and absolute times. Repeat with more runs if noise could hide the acceptance margin. Use stage attribution separately from uninstrumented command latency. Never record tracked benchmark history just to investigate this change.

| Workload | Required observation |
|---|---|
| Clean small/large, many tiny bodies, template-heavy and generic-heavy modules | No extra recovery allocations or context snapshots. Target no repeatable regression beyond noise. A repeatable median regression above 2% in an affected clean workload requires correction or explicit maintainer acceptance, not an automatic waiver. |
| Same early error with module allowance 1 | Retained first diagnostic matches the original fail-fast policy. Measure failure-path plumbing separately from extra coverage. No per-error full-context copies. |
| Two nearby independent body failures | Both useful primary errors appear when budgets permit. Report incremental time and allocation cost rather than applying a clean-build percentage to intentionally extra work. |
| First body fails, followed by increasing valid suffixes | Admitted recovery tokens remain within the chosen allowance. Time spent in earlier token/header work may still grow with the file, so attribute AST extra-work counters rather than claiming constant whole-command time. |
| Expensive templates, numeric completion and generic calls within a token-sized body | Retain existing semantic resource bounds. Reject recovery choices with disproportionate extra latency or peak memory. Token count alone is not proof. |
| Five errors reached early and both count limits disabled | Count cutoff and independent token cutoff each stop the correct boundary. Disabled limits do not trigger large reservations. |
| Multi-module graphs with serial/parallel preparation and early/late project cutoff | Same module diagnostic selection and valid contexts, fewer new admissions after the threshold, all already-running returned results retained. No global coordination cost in workers. |
| Real incremental reuse, when available | Unchanged successful modules are actually reused. Policy-only changes do not invalidate semantic artefacts. Skipped/failed changed modules never reuse stale success. |

Use existing counters where available. Add only necessary feature-gated observations for additional bodies/tokens admitted after the first error, retained/discarded primary counts, scheduling admissions after cutoff and diagnostic-context copying/retention. Successful-path assertions and owner-size checks complement timings. A nominal 2% bound is an investigation ceiling, not a claim that a regression is free.

If no useful recovery allowance meets the latency, memory and complexity requirements, keep the safe fail-fast path, record the rejected candidate and leave the recovery item blocked for maintainer disposition. Do not mark the planned feature supported merely because limits and metadata shipped. Optional transfer or proof optimisations and diagnostic presentation changes cannot be used to make this comparison look better.

## Documentation, roadmap and deliberately deferred work

The planning PR adds this queued plan and its roadmap position. It does not mark runtime support as delivered. Implementation performs these updates at the phase where the facts become true:

| Owner | Required change |
|---|---|
| `docs/compiler-design-overview.md` | Add stage-owned diagnostic recovery under diagnostics and Stage 4. Record valid-signature use, absent-body-fact suppression, the work guard and success-only handoff. Preserve broader stage boundaries. |
| `docs/build-system-design.md` | Specify strict module allowances, soft request scheduling cutoff, pre-discovery fallback, canonical admission/aggregation, package/check-only accounting and explicit incomplete graph handling. Define diagnostic policy as separate from semantic reuse identity. |
| `docs/compiler-data-layout-design.md` | Describe only the minimal report stop metadata and its source/type/identity lifetime. Preserve locked common-record sizes, one diagnostic store and the three failure lanes. Do not duplicate recovery policy here. |
| `docs/src/docs/project-structure/project-config.mtf` | Document current working syntax for both `I32?` settings, 5/30 defaults, positive-value validation and independent `none` semantics. Explain that the project value is a soft cutoff, not a maximum result count. Keep final-design syntax clearly separated from current support. |
| `docs/roadmap/roadmap.md` | Keep normal ordering: compact diagnostic layout, diagnostic improvements, then this recovery work. Mark activation in the existing bullet. Do not reactivate paused work or duplicate the production borrow tracker. Remove this bullet on accepted completion. |
| Diagnostic presentation work | Consume the delivered bounded-recovery, module-limit and stop-metadata capabilities when this work completes. Preserve remaining presentation scope and avoid rebuilding those controls. Use permanent contract references. |
| `docs/roadmap/plans/general-directives-and-project-config-plan.md` | Keep a short migration reminder for both setting names, optional I32 types, defaults, validation, build-only ownership and `none` semantics. Extend its later closed signature, config application, options projection, docs and regression tests rather than dropping the settings or maintaining two syntax paths. |
| `docs/src/docs/progress/@page.moth` | At implementation, update the existing diagnostic/recovery status or add one compact accepted-implementation row. Name ordinary concrete-body recovery and its limits, without implying all-AST or borrow recovery. Record narrow/rejected gates truthfully. A prose-only planning PR changes no supported status. |
| `docs/src/docs/progress/packages-and-builders/@page.moth` | Update an existing configuration/command coverage row only when actual config or public command support changes. Do not add duplicate compiler diagnostic coverage. |
| `docs/roadmap/plans/production-borrow-checker.md` | Record a deferred function-boundary recovery follow-up in the living companion, with the prerequisites and constraints below. Keep historical measurements and the live production queue intact. |
| `index.md` and test ownership metadata | Update moved/new implementation owners and genuinely changed responsibilities. Register one primary owner for each new public test contract. Avoid a second diagnostic architecture inventory. |
| Benchmark report and audit records | Add measured recovery evidence to the existing indexed report. Mark materially affected old audit coverage stale under the audit guide. A Slice review is not a new formal audit run. |

### Deferred borrowing handoff

Borrow recovery belongs with the next suitable production driver/fact-lifetime change after per-function analysis inputs, publication and required call-summary dependencies are stable and measured. It need not wait for every future precision experiment or the whole memory model. This plan neither enables it nor promotes Boracle into the production acceptance path.

The follow-up should stop a function at its first final user-facing borrow conflict, discard or keep unreachable only its unpublished scratch and continue an independent function only with trustworthy required summaries. Recursive summary groups may need a coarser boundary. An unavailable summary blocks dependent checking without a fabricated source error or a falsely valid default summary. Preserve the distinction between conservative proof failure and a proven conflict.

Revisit current writes into the shared borrow report and summary convergence before changing its `?` boundaries. No success report, effect summary, generated completion or lifetime validation may consume partial failed facts. Reuse module limits and stop metadata, measure valid-path costs and test repeated/converging analyses without duplicate diagnostics. Conflict-directed precision work improves acceptance evidence, not within-function error enumeration. Preserve Boracle's independent reference/oracle roles and activation gates.

### Other deliberate deferrals

Broader AST recovery, statement recovery, generic-template/const-template/start recovery, parallel semantic-module scheduling, persistent or incremental diagnostic caches and CLI/LSP display limits remain outside this implementation. Record the supported subset and future prerequisites in existing owners. Add no dormant flags or speculative structures for them. Public tooling may eventually display about five generated errors and offer the rest, but it must continue to distinguish generated results from analysis never performed.
