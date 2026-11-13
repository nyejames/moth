# Const functions follow-up report

Status: implemented on `const-functions`
Baseline: `b9795e824`
Follow-up source: `tmp/const-functions-implementation-review-followup.md`

## Outcome

The Stage 4 typed-AST evaluator remains the execution owner. This follow-up adds the missing
release-safety proof, removes the silent early-preparation deferral, reuses completed provider const
origins before rebasing, shares resolved member maps inside one materialisation transaction, narrows
release preparation to called bodies, and makes configured budget diagnostics numerically explicit.

No HIR interpreter, second ownership solver, persistent cache or process-global materialisation cache
was added.

## Reproduced defects and corrections

### Release folding lost aggregate alias topology (F-01)

Reproduced with an ordinary function that stored one local collection in two record fields and
returned the record. Debug lowered the call and printed `2` after mutating one shared field; release
folded the call into a constant record whose two fields were lowered as independent `[1]` arrays and
printed `1`.

Correction:

- Release candidates admit only `ImmutableOwned` parameters and a single infallible result whose
  parameter and result types are fixed scalars, Bool or Char.
- A candidate body may not contain a compile-time template operation, which keeps resource-bearing
  bodies out of the speculative path.
- Body expression provenance must be a subset of the provenance carried by returned expressions.
  A project/config dependency used only by a discarded local therefore refuses the candidate rather
  than disappearing from link facts with the erased call.
- Calls are still rewritten only when every argument folds to a constant, so a shared parameter
  value has no caller runtime place to alias.
- Ordinary aggregate and String calls remain runtime calls. Structural result support is deferred
  until the executor can prove returned aggregate freshness and resource provenance.

Regressions:

- `release_ordinary_aggregate_alias_parity`: release prints the debug result `2`.
- `release_ordinary_scalar_alias_mutation_parity`: scalar result remains correct when a local
  aggregate is mutated.
- `const_function_release_folding_success` continues to prove scalar direct and transitive folding.

### Early const preparation silently deferred on receiver-catalog diagnostics (F-02)

Reproduced in two forms:

- A valid const free function plus an unrelated valid receiver method failed as
  `Const function ... cannot be executed` because the early receiver catalog could not find the
  nominal source file and the body preparation returned `Ok(())` without installing an executable.
- A receiver method that collided with a struct field reported the same generic const failure
  instead of its real invalid-receiver diagnostic.

Correction:

- Source nominal file rows are registered before both the constant-dependent alias prefix walk and
  the whole-module constant walk.
- The prefix walk has its own registration because it can prepare a const body before the main walk.
- `prepare_const_function_body` now propagates a receiver-catalog diagnostic instead of silently
  deferring. With source rows present, remaining catalog diagnostics are authored declaration errors.
- The stale "Slice 4 bounded HIR executor" comment was corrected to the typed-AST executor.

Regressions:

- `const_function_unrelated_receiver_success`
- `const_function_receiver_declaration_reports_source_error`
- `const_function_prefix_alias_receiver_success`

### Repeated provider imports rebased before reuse (F-03)

Correction:

- `materialised_const_provider_origins` is now an origin-to-consumer-entry-path map.
- A repeated import aliases the prepared entry declaration, signature and executable before registry
  lookup or provider-context rebase.
- Cold lookup uses the published `row_index` instead of rescanning provider rows.
- The reachable private closure still materialises only once and unrelated provider bodies stay out.
- Resolved struct-field and type-alias maps are wrapped in `Rc` after accumulation, so every body in
  one transaction shares one handle instead of cloning map content per body.
- The simple reachable-closure scan is retained. Its bound is the provider row count times closure
  depth; no workload justified a new index.

Evidence:

- `repeated_import_aliases_install_one_provider_const_closure` now asserts one rebase and one retained
  provider context for the origin, in addition to one body preparation and zero unrelated-body
  preparations. Before the reorder the same fixture performed a rebase per alias.

### Release preparation did work with no reachable calls (F-04)

Correction: a single recursive typed-expression walk collects ordinary call targets, then a worklist
prepares only called bodies and the ordinary callees they name. Uncalled bodies are not compiled into
speculative executables. Constant calls stay on the required-call path.

Measurement: a generated release workload with 10,000 uncalled ordinary functions and no eligible
call completed in 0.10 s, equal to its dev-profile baseline (warm runs, same compiler binary). A
second generated workload declared 10,000 scalar identities and called all of them with runtime
arguments; warm release builds measured 0.13-0.19 s against 0.18-0.21 s for the dev profile. The
call-target gate therefore removes idle per-function work without a measurable cost for ordinary
runtime calls. Debug builds still perform no speculative source-function execution.

### Const execution ownership split (F-05)

`const_execution/mod.rs` is now a 472-line navigator. The implementation moved into one-owner
modules: `eligibility.rs` (721), `rewrite.rs` (871), `execute.rs` (1,187), `places.rs` (1,064) and
`values.rs` (434). `budget.rs` remains limits/charging only and `tests.rs` is unchanged. The tree has
one executable type, one eligibility policy, one value store and one executed-frame machine; no
traits, callback adapters or second operation enums were added.

### Final-review corrections

Final review and integration audits found provenance and unit defects. All are corrected before
closeout:

- The provenance proof unioned all return expressions. A `@project`-dependent const call on one
  branch and a literal on another therefore passed even though the replacement inherits only the
  executed value's provenance. Eligibility now requires the body provenance to be carried by every
  return statement, and `release_refuses_candidate_with_branch_local_project_dependency` proves the
  release call stays at runtime.
- A candidate's ordinary-call occurrence does not carry its callee body's project dependency. An
  outer candidate could discard a helper result in a dead branch and fold to a literal, making the
  helper unreachable. Candidate proofs now retain body and return provenance; retention propagates
  callee provenance through the acyclic candidate graph and removes a candidate whose every return
  does not carry the transitive set.
  `release_refuses_outer_candidate_with_transitive_callee_dependency` keeps the outer call at
  runtime while the direct helper calls may fold. The dependency order is a callee-first DFS;
  `dependency_order_finishes_shared_callee_before_its_caller` pins the shared-callee graph shape,
  and an incomplete traversal refuses the candidate instead of silently dropping provenance.
- A return expression using short-circuit logic can evaluate to the left operand while a
  project-dependent right subtree is skipped; the executor does not transfer that skipped
  subtree's provenance onto the result. Eligibility now refuses a return whose skipped short-circuit
  subtree carries provenance. `release_refuses_short_circuit_return_with_skipped_dependency` keeps
  `probe(false)` at runtime.
- More generally, an executed value can be selected from a branch, index or projection whose own
  metadata omits a dependency present in the authored body. Retention now returns each candidate's
  effective transitive provenance, and every release replacement merges it before inheriting the
  evaluated value's metadata. `release_replacement_merges_candidate_provenance` covers the merge;
  this is the authoritative safety net for selected-result provenance, while the per-return and
  short-circuit proof remains conservative.
- Budget advice rendered every work hard limit as steps, every retained-memory hard limit as
  MiB, and structural output pieces as bytes. Limit refusals now carry a `ConstBudgetUnit`
  (`steps`, `MiB`, retained values, structural pieces, call frames,
  expression/handler/projection levels), and the unit test asserts configured and structural
  hard limits. `structural_piece_refusal_names_pieces` drives `FoldOutputState::charge_piece`
  directly, so the source-level piece refusal, not only a hand-built refusal value, is covered.

### Consumer reuse invalidation (F-06)

No warm consumer semantic reuse exists in the current source: the build path recompiles consumers, and
the existing provider-edit tests compare fresh compilations. No dormant cache digest was added.

The gate is recorded in `docs/developer-docs/compiler-design/module-artefacts-and-reuse/module-artefacts-and-reuse.mtf`:
before reuse is enabled, publication must expose one compiler-owned digest over the exported const
body and its transitive private helper/value closure, using stable semantic identity and without
publishing private declarations. A body-only or private-helper edit then invalidates only consumers
that folded the affected origin.

### Diagnostics and deferred cleanup (F-07)

- Configured memory limits render as MiB and configured work limits render as steps, both in required
  errors and release Notes. The numeric limit is a structured diagnostic fact.
- `release_optimisation_budget_note` exercises the release Note path and runtime fallback.
- `const_function_execution_limit_rejected` asserts the effective `1000000 steps` value.
- `configured_budget_facts_and_advice_name_typed_units` asserts typed facts, MiB advice and the
  rendered message.
- `const_function_collection_read_memory_rejected` executes repeated real `length()` reads until the
  memory allowance is exhausted, covering the scratch-vector charge.
- Competing unsupported statement and earlier catch-handler sites remain fail-fast rather than
  globally source-sorted. This affects only post-parse executor forms; changing it needs one combined
  rejection collector, not a second diagnostic pass.
- The historical completion report moved to `docs/roadmap/evidence/`; the active plan is already
  retired.

## Validation

- `cargo test -q -p moth --lib`: 7019 passed, 0 failed.
- `target/debug/moth tests --terse`: 3373/3373 correct.
- `target/debug/moth tests --tag const --terse`: 112/112 correct.
- `cargo clippy -q -p moth`: clean.
- `target/debug/moth check docs --terse`: clean.
- Manual release/debug JS execution reproduced the alias defect (`2` vs `1`) and the fix (`2` vs `2`).
- Manual release budget probe printed one deduplicated Note with `(100 steps)` and
  `const_work_limit`.

- `just validate`: passed (workspace tests, feature-lane coverage, source and dependency audits,
  integration, docs).
- `just validate-full`: passed on the final candidate (workspace feature matrix, test-honesty
  audit, recording-free performance checks, all 14 scaling series within budget, timer-erasure
  check). The cache-maintain soft-budget notice for `target/` is environmental and did not fail
  the gate.
- Final integration audit: pass after all provenance corrections; no actionable patch-introduced
  defect.

## Final merge audit follow-up

### Gate A: reused signature identity and release speculation scope

- Reproduced the alias identity defect: a reused imported signature kept parameter paths under the
  original consumer entry, not the requested alias. `const_materialisation.rs` now remaps each
  `FunctionParameter.id` under `imported_local_path`, preserving type, mode, default, binding span
  and contract. The alias declaration takes the original entry declaration's `binding_span` or
  `None`; it never borrows a parameter span. A test-only invariant checks parameter parents for
  every fresh and reused materialisation, and the new provider fixture covers positional, omitted
  default and named alias calls with exact folded values plus one preparation/rebase.
- Reproduced release-only speculative work: with `const_work_limit = 3`, an unrelated
  `items.get(...)` expression consumed the same budget and produced a second Note. Release
  speculation now carries a per-step auxiliary flag; template, collection-read and fallible-unwrap
  folding run only inside argument subtrees of calls with a retained executable. Field projection
  is gated the same way in release, while required and interpreted const-body rewriting keep the
  previous folding. The tight fixture now emits exactly one Note at the eligible `count(20)` call;
  the unrelated source-collection reads stay in release HIR. The ample-budget fixture keeps
  debug/release folded output equal, and a constant-field candidate argument proves auxiliary
  argument folding still works.

### Gate A validation

- `cargo test -q -p moth --lib const_alias_signature_identity_tests`: 1 passed.
- `cargo test -q -p moth --lib release_speculation_scope_tests`: 2 passed.
- `cargo test -q -p moth --lib release_`: 26 passed.
- `target/debug/moth tests --tag const --terse`: 112/112 correct.
- Independent audits: alias identity clean; release scope clean, with candidate-argument auxiliary
  coverage added before the phase checkpoint.

### Slice B: redundant Stage 3/4 scans, signature snapshots and executor clones

- `walk_stage_three_declarations` now reuses its computed `module_has_const_functions`; the
  duplicate ordered scan is deleted. Source nominal files are seeded once in
  `register_nominal_identities`, and `register_source_nominal_files` plus both calls are deleted.
- `ScopeShared` now holds the builder's live `Rc<RefCell<FxHashMap<...>>>` early const signature
  table. Scope construction shares one handle instead of cloning the growing map per declaration.
  `source_callable_signature` returns `Cow` (canonical borrowed, early row cloned individually);
  evaluation reads through one borrow. Measured copied entries at 100/500/2,000 const functions:
  15,050/375,250/6,001,000 -> 0/0/0. Focused timing: 39->31 ms, 175->123 ms, 1,842->1,005 ms.
  Receiver catalog measured 0/11/176 ms; not dominant and left uncached.
- `Step::CompleteCall` borrows the executable row. `execute_frames` borrows each instruction and
  clones only a selected body `Rc` or loop binding spec needed for a new frame. Instrumented
  workload: 66 executed instructions, deep instruction clones 66 -> 0, executable clones 16 -> 0,
  allocations 623 -> 589.

### Slice B validation

- `cargo test -q -p moth --lib const_execution`: 17 passed.
- `scope_context_tests` 23, `type_resolution_tests` 32, release scope 2, alias identity 1 passed.
- `cargo build -q -p moth-cli && target/debug/moth tests --tag const --terse`: 112/112 correct.
- `cargo clippy -q -p moth`: clean.
- Independent audits: Stage 4 clean; executor borrow clean.

### Slice B cleanup: rewrite API and import ownership

- `rewrite_release_calls` now returns only the deduplicated budget Notes; the unused fold counter
  and its finalizer binding are deleted.
- `fold_templates` + `release` booleans are replaced by `RewriteMode::{RequiredExpression,
  InterpretedBody, Release}`. This preserves the distinct required-expression path (no auxiliary
  template/collection/fallible folding), interpreted-body completion, and release argument scoping.
- Test-only forwarding imports are removed from `const_execution/mod.rs`; unit tests import
  budget, eligibility and rewrite owners directly, and `budget.rs` imports its value/error helpers
  from the sibling owner. Stale/duplicate comments are removed and the release docs now describe
  reachable bodies and Notes only.
- An audit caught an initial mode collapse that wrongly enabled auxiliary folding in the
  required-expression path; the three-mode correction was re-audited clean and the full 3,373
  integration cases pass.

### Slice B cleanup validation

- `cargo test -q -p moth --lib const_execution`: 17 passed.
- `cargo test -q -p moth --lib release_speculation_scope_tests`: 2 passed.
- `cargo clippy -q -p moth`: clean.
- `target/debug/moth tests --tag const --terse`: 112/112.
- `target/debug/moth tests --terse`: 3373/3373.

### Slice C: one provenance ownership policy

- Removed the per-return subset gate, the short-circuit skipped-operand refusal, the return-value
  collectors and the `return_provenances` field. The acyclic candidate closure still propagates
  each candidate's complete effective transitive provenance, and every successful release
  replacement merges that set. The merger is now the single authority for dependencies erased with
  a folded ordinary call.
- Branch-local `@project`, transitive ordinary callee and skipped short-circuit fixtures now assert
  the candidate outer call is actually erased (`debug_calls > 0`, `release_calls == 0`) and that
  surviving HIR still reaches ProjectContext from the synthetic start function. This proves the
  replacement carries the dependency rather than a surviving call doing so.
- Source-`$config` and constant-projection variants were not retained as HIR comparisons: debug
  retains no reachable runtime ProjectContext link fact for those compile-time forms, so they cannot
  distinguish the replacement merger from the old gate.

### Slice C validation

- `cargo test -q -p moth --lib release_`: 26 passed.
- Focused audits: consolidation clean; corrected erased-call assertions clean.

### Slice D: cold provider closure and diagnostics/catch identity

- Measured a cold provider import with one selected export and 1,000 unrelated const rows. Full
  context clone/remap copied 1,002 rows (about 1.19 MB payload) and took 923 us private / 1,360 us
  exported; total cold import was 3.03 / 5.04 ms. Selecting reachability from published row indices
  before copying reduces the copy to 2 rows (about 2.4 KB) and 16 us; import falls to 1.73 / 2.77 ms.
  Two exports: 4.83 -> 2.50 ms, still two rebases. One hundred aliases: 2.91 -> 1.68 ms, one rebase,
  alias reuse remains 2-5 us. The selected closure uses the existing origin/row index and adds no
  cache or publication contract. Fallback without a const identity arc rebases generically and then
  selects the same indices.
- Fixed safety-limit failures now populate the existing typed numeric-limit fact with the effective
  bound, rather than omitting it when no setting exists. The compact diagnostic family already uses
  all four fact words, so the unit cannot be added without a schema change; the hard-limit sentence
  stays generic, while release advice still spells the unit. The limitation is recorded here rather
  than hidden behind pre-rendered text.
- `const_function_repeated_handler_success` now covers two rewritten catches with distinct handlers
  and a successful read that must not recover. Same-span authored ambiguity could not be constructed;
  the compiled span fallback remains the identity handoff for rebuilt catch expressions.
- The cold import reuse probe now records selected rows, not retained full contexts, so its
  assertion matches the new materialisation contract.

### Slice D validation

- `cargo test -q -p moth --lib const_execution`: 17 passed.
- `cargo test -q -p moth --lib repeated_import`: 3 passed; alias identity 1 passed.
- Focused diagnostic tests and `const_function_repeated_handler_success` pass.
- `target/debug/moth tests --tag const --terse`: 112/112.
- `target/debug/moth tests --terse`: 3373/3373.
- `cargo clippy -q -p moth`: clean.
- Audits: provider closure clean (test-gap note addressed by selected-row probe); diagnostics/catch
  clean.

## Final merge gate

Candidate `dc9b2a9fc` (base `6335d615c`; checkpoints `bae9d038d`, `b665526e4`, `cfc0fc108`,
`930170b1f`, `dc9b2a9fc`) passed `just validate-full`: formatting, clippy and feature lanes, source
and dependency audits, unit/integration/docs gates, benchmark CI, all 14 scaling series within
budget and the timer-erasure check. The const-tagged integration suite remained 112/112 and the
unfiltered suite 3373/3373. Two fresh final lanes (correctness/regression and
architecture/ownership/documentation) passed, followed by a fresh cross-scope integration audit that
also passed. No required correction remains. Deferrals recorded above are unchanged.
