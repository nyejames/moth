# AUD-0024: Numeric proof side-table comments

- State: `complete`
- Kind: `Comments`
- Area: `analysis.numeric_proofs` - the bounded-integer proof side table, its production invocations and publication, and its backend proof queries
- Coverage: `partial`
- Reviewed: `2026-11`
- Baseline: Branch `audit-guide-improvements` at `279eb7b63`. Production code is identical to `main` at `49714f1b8`, because the branch changes audit documents only. `cargo test -q -p moth --lib numeric_proof` passed (60 tests) at `e49a166e8` during AUD-0022 and was not rerun, since no code changed since then. No validation gate was run for this read-only run.

## Question

Do the area's comments state the proof table's ownership, timing, key domain and consumer contract accurately, and do the backend landmarks describe the current failure-edge pruning?

## What was inspected

Read in full:

- `src/compiler_frontend/analysis/numeric_proofs/mod.rs`, including the module header, every item doc and every inline comment.

Read at the proof-related items:

- `src/compiler_frontend/module_compilation/artefact.rs`: `ModuleExecutable` and its `numeric_proofs` field doc.
- `src/compiler_frontend/module_compilation/service.rs` lines 960-1110: the base `run_generated_summary_convergence` call, the expression freeze and the base `analyse_numeric_proofs` call.
- `src/compiler_frontend/module_compilation/generated/convergence.rs` lines 560-735 and 845-880: base and sidecar private-lane refresh, and the sidecar proof publication loop.
- `src/compiler_frontend/module_compilation/generated/materialisation.rs` lines 455-475: the empty placeholder table.
- `src/backends/js/reachability.rs` in full.
- `src/backends/js/js_statement.rs` lines 60-70 and 300-420: `PreparedInvoke`, `classify_numeric_operation`, `lower_numeric_operation`, `proven_narrowing_expression` and `integer_narrowing_is_proven`.
- `src/backends/js/emitter.rs` lines 25-125, 238-252 and 490-520: `JsEmitter` fields, `FunctionEmission`, `functions_to_emit` and the proven-narrowing cast-policy scan.
- `src/backends/js/js_function.rs` lines 364-374 and 605-635: the two Invoke branch-inspection landmarks.
- `src/backends/wasm/hir_to_lir/stmt.rs` lines 95-112 and 320-365: fast selection and `IntegerCheckMode` selection.
- `src/backends/wasm/hir_to_lir/context.rs` and `src/projects/html_project/compile_input.rs`: the `numeric_proofs` field docs.

Context read to verify comment claims, not audited:

- `FixedScalarValue::as_i64` and `as_u64` in `crates/moth-lexical/src/numeric/fixed_scalar.rs`.
- `HirExpressionKind::Cast` docs in `hir/expressions.rs`.
- `JsNumericOperationLowering::classify` fast selection in `backends/js/numeric_carrier.rs`.
- `numeric_operation_cannot_fail` and `checked_result_range` in `datatypes/numeric_operators.rs`.
- `NumericScalar::integer_range`.

## Authorities read

`AGENTS.md`; `docs/developer-docs/style-guide/style-guide.mtf` in full, especially `Comments as reading landmarks`; `docs/roadmap/audit-guide.md`; `docs/roadmap/audit-kinds/README.md`; `docs/roadmap/audit-kinds/comments.md`; `docs/roadmap/audits/README.md`; `docs/compiler-design-overview.md` `Frontend stages > Stage 5: HIR and validation > Numeric ownership`.

## Existing findings and active plans checked

AUD-0022 F01 to F03 (Redundancy) and AUD-0023 (Performance, clean) cover this area. AUD-0022-F02 already covers the three `expect` calls in `proven_narrowing_expression`, so they are not re-filed here. `docs/bug-reports.md` has no comment entry for the area. No active plan touches it.

## Findings

### AUD-0024-F01: Proof-table timing comments name HIR validation where the contract requires final converged HIR

- State: `candidate`
- Kind: `Comments`

#### Evidence

The overview says each base executable and generated sidecar "computes its own table from final validated HIR before publication", and that "Private-lane installation or handler pruning refreshes affected proof analysis when it rewrites HIR".

Three comments describe the timing only as "after HIR validation":

- The module header in `numeric_proofs/mod.rs` lines 43-44: "computed inside the compiler services after HIR validation, before publication".
- The `analyse_numeric_proofs` doc, lines 105-106: "base and generated sidecar compilation each call this once after HIR validation and before publication".
- The `ModuleExecutable.numeric_proofs` field doc in `artefact.rs`: "computed inside the compiler service after HIR validation and before publication".

HIR validation happens when HIR is built, well before convergence. After that, `run_generated_summary_convergence` (`service.rs` line 969) reaches `refresh_private_failure_lanes` (`convergence.rs` lines 603 and 633). That calls `install_private_failure_lanes`, which rewrites bodies, splits producers and calls `compact_block_ids` (`private_failure_lane.rs` line 98).

The base call at `service.rs` line 1094 sits about 125 lines after the convergence call and has no comment tying it to the converged HIR. The sidecar call has the right landmark at `convergence.rs` lines 716-717: "from the final HIR after every rewrite and summary refresh above".

#### Counter-explanation tested

"After HIR validation" is literally true of both call sites, and convergence re-runs borrow analysis. Rejected. A reader looking for the earliest valid placement would place the call after the first validation. The analysis indexes by dense post-compaction block id and seeds through success edges, so a table derived before lane installation would describe a CFG that no backend lowers. The sidecar comment shows the authors treat the converged HIR as the real constraint. The base site and the shared docs do not say so.

#### Violated contract or cost

The style guide asks comments to explain non-obvious ordering and phase joins. A future change could move the base call next to borrow analysis or HIR validation while still matching every existing comment. The table would then pair with HIR that convergence has since rewritten.

#### Root owner

`analysis/numeric_proofs/mod.rs` for the timing contract. `module_compilation/service.rs` for the base call-site landmark.

#### Suggested correction

Non-authorising. Replace "after HIR validation" in the three comments with wording such as "from the final HIR after private-lane installation and convergence". Add a one-line landmark at `service.rs` line 1094 matching the sidecar comment.

#### Fix scope and preserved invariants

Comments only. No code movement or behaviour change.

#### Required validation

Re-read `run_generated_summary_convergence` and `refresh_private_failure_lanes` to confirm the wording names the last HIR rewrite before both calls. Run `cargo check -p moth`.

#### Linked findings

None.

### AUD-0024-F02: Proof-table key documentation calls Invoke operation ids statement ids

- State: `candidate`
- Kind: `Comments`

#### Evidence

The authority keys `NumericProofs` "by the operation's HIR node identity". The table's own documentation agrees:

- The module header in `mod.rs` line 41: "the statement or Invoke operation's own `HirNodeId`".
- The field docs at lines 77-83: narrowings are "`NumericConversion` Invoke ids".
- The code inserts the terminator `id` at lines 276 and 284.

Two other comments say otherwise:

- The `ModuleExecutable.numeric_proofs` doc in `artefact.rs` calls the table "statement-keyed proven-safe operation/narrowing facts" that "pair with the immutable statement ids they lower".
- The query docs at `mod.rs` lines 87 and 94 say each fact is "recorded at `statement`". `integer_narrowing_is_safe` never holds a statement id, because every narrowing fact is an Invoke `CastOp` id.

#### Counter-explanation tested

The overview says statement-to-Invoke rewrites preserve identity, so a numeric Invoke's id was once a statement id. Rejected for narrowings. Fallible casts lower straight to `HirTerminator::Invoke` (`hir/expressions.rs` lines 182-185), so a narrowing fact has no statement origin. The publication-facing doc on `ModuleExecutable` is the one a new backend author reads first.

#### Violated contract or cost

The comment misstates the ID domain a consumer must query. A backend following it would query statement ids only. It would silently lose every narrowing and Invoke operation fact. That outcome is conservative, but nothing would surface it.

#### Root owner

`module_compilation/artefact.rs` field doc and the two `NumericProofs` query docs.

#### Suggested correction

Non-authorising. Describe the key as the operation's `HirNodeId`, whether it belongs to a statement or an Invoke terminator, matching the module header. Rename the parameter in the same change under AUD-0025-F01.

#### Fix scope and preserved invariants

Comments, plus the linked rename. Key domain and behaviour unchanged.

#### Required validation

`cargo check -p moth`. Confirm the wording against the two insertion sites in `visit_terminator`.

#### Linked findings

AUD-0025-F01 (Style rename of the `statement` parameters).

### AUD-0024-F03: JS branch-inspection landmark omits proven-narrowing Invokes

- State: `candidate`
- Kind: `Comments`

#### Evidence

`invoke_failure_edge_is_live` (`reachability.rs` lines 25-46) prunes the failure edge for two cases: scalar numeric Invokes and proven integer-narrowing cast Invokes. Its doc and `for_each_live_terminator_target` say so. Two landmarks name only the numeric case:

- The `reachability.rs` module header, lines 3-4: "pruning the failure edge of a numeric Invoke whose shared classification yields a scalar".
- `js_function.rs` lines 621-622: "A scalar numeric Invoke always takes its success edge, so its handler is not a live branch body".

The similar landmark at `js_function.rs` line 369 and the `PreparedInvoke` doc (`js_statement.rs` line 64) both name "a scalar numeric or proven-narrowing Invoke".

#### Counter-explanation tested

The predicate's own doc is complete, and the call delegates to it. Rejected. The module header is the entry point for the file's ownership. The line-621 comment sits on the match that a maintainer would edit when adding cast-specific behaviour. A reader could conclude that cast Invokes always inspect both arms and add a second cast check, duplicating the liveness owner.

#### Violated contract or cost

Two related comments now contradict each other. The style guide asks that comments stay aligned with current behaviour.

#### Root owner

`backends/js/reachability.rs` module header and `backends/js/js_function.rs` Invoke branch-inspection comment.

#### Suggested correction

Non-authorising. Name both pruned cases, matching line 369, or say "an Invoke whose failure edge the liveness owner prunes".

#### Fix scope and preserved invariants

Comments only.

#### Required validation

`cargo check -p moth`.

#### Linked findings

None.

### AUD-0024-F04: `FunctionEmission` rationale narrates implementation history

- State: `candidate`
- Kind: `Comments`

#### Evidence

The `FunctionEmission` WHY in `emitter.rs` lines 88-90 reads: "each scan previously rebuilt and sorted the same closure per function, reclassifying every numeric Invoke". The `functions_to_emit` doc at lines 241-246 already states the present constraint: one owner computes the shared live closure without each scan reclassifying numeric Invokes.

#### Counter-explanation tested

The history explains why the struct exists. Rejected. The present-tense statement at `functions_to_emit` carries the same reason, and the checklist treats implementation history recorded by Git as noise.

#### Violated contract or cost

Low. It is stale-prone prose that describes code which no longer exists.

#### Root owner

`backends/js/emitter.rs` `FunctionEmission` doc.

#### Suggested correction

Non-authorising. Delete the "previously" sentence and keep the present-tense ownership statement.

#### Fix scope and preserved invariants

Comments only.

#### Required validation

None beyond `cargo check -p moth`.

#### Linked findings

None.

## Checked and clean

- **Soundness header.** Each claim in `mod.rs` lines 11-37 matches the code:
  - The seed conditions match `visit_terminator` lines 296-302: one success result, one incoming edge, not a callable entry, a later block.
  - Fast-semantics exclusion matches line 241.
  - Checked `i128` arithmetic declines rather than wrapping, matching `prove_operation`.
  - "Both scalar backends select fast delivery before consulting it" matches `JsNumericOperationLowering::classify` (`numeric_carrier.rs` line 1096) and Wasm `stmt.rs` line 104.
- **Literal payload comment** at lines 452-453. This matches `FixedScalarValue::as_i64`, which covers signed classes only, and `as_u64`, which covers unsigned and `Octet`.
- **Cast comment** at lines 460-461. The claim "Fallible casts never appear as expressions" matches `HirExpressionKind::Cast` docs.
- **`count_incoming_edges`.** The out-of-range comment states a conservative tolerance. A missing slot never seeds, because line 298 uses the same `get`.
- **Item docs.** `IncomingEdgeFacts`, the `BlockAnalyser` field docs, the `prove_operation` divisor comments and the `Negate` domain comment are accurate.
- **Publication comments.** The materialisation placeholder comment is accurate: convergence overwrites the empty table before publication. So is the sidecar publication comment in `convergence.rs`.
- **Wasm.** The `IntegerCheckMode` comment matches the overview's `Infallible` rule. So does the `WasmLirLoweringContext.numeric_proofs` doc.
- **JS field docs.** `JsEmitter.numeric_proofs`, `HtmlModuleCompileInput.numeric_proofs`, the `classify_numeric_operation` doc, the `invoke_failure_edge_is_live` doc and the proven-narrowing cast-policy landmark in `emitter.rs` line 500 are accurate.

## Limitations

- Threading bodies in `wasm/backend.rs`, `wasm/hir_to_lir/module.rs`, `html_project_builder.rs`, `js_path.rs` and `html_project/wasm/artifacts.rs` were located by search, not read.
- `numeric_carrier.rs` was read only at the classification entry point. The proven-narrowing renderer and runtime tables were not audited.
- Test modules, including `numeric_proofs/tests/` and backend proof tests, were not inspected.
- The parent `analysis/mod.rs` header was read as context. Its stale WHY is logged in `docs/bug-reports.md` rather than filed.
