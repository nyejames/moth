# AUD-0007: Numeric profile boundary correctness

- State: `complete`
- Kind: `Correctness`
- Area: `contract.numeric_profile.frontend_runtime` — selected numeric profile, folded values and checked operations across frontend, build and HTML-JS runtime boundaries.
- Coverage: `partial`
- Reviewed: `2026-09`
- Baseline: Branch `numeric-type-expanding`, pinned numeric range `bf65cb05b4ecb2536655794ea2ce0313304562f3..08c62a0307d23794a2be4c8453943efa1bfec5f6`, current correction checkpoint `f67270a0c`. `just validate` passed (5,475 unit tests and 2,084/2,084 integration cases); HTML numeric-profile cases passed 5/5 and deterministic Core Random boundary cases passed 1/1 at each Float precision. The user-owned handoff remains untracked and unchanged.

## What was inspected

The pinned `git diff --name-status` range above contains **620 changed-file records** (three rename records each count once). The reverse commit log separates preceding planning/design commits from numeric implementation. Four disjoint patch-review lanes accounted for every record by inspecting its semantic changes, tracing mechanical edits through their owners, or naming an out-of-boundary change. The grouped counts below are file records, not claims that each file received an end-to-end audit:

| Changed paths | Records | Patch classification and reason |
|---|---:|---|
| `src/compiler_frontend/{ast,builtins,datatypes,numeric_text,type_coercion}/**`, `folded_value.rs` | 161 | Inspected numeric typing, pending/generic literal materialisation, folding, casts, const/template ranges, public scalar representation and direct versus optional scalar policy. Mechanically traced remaining exhaustive fixed-scalar visitors, profile propagation and test-only updates. `ast/templates/template_head_parser/directive_args.rs` changes positional slot-index parsing, outside this numeric boundary. |
| Other `src/compiler_frontend/**`, `src/benchmarking/frontend.rs` | 84 | Inspected HIR operations/failure modes, AST-to-HIR conversions, target validation, config/profile fingerprints, generated compilation, diagnostics, provenance and single-source template/config paths. Mechanically traced fixture updates and fixed-scalar leaves in borrow/IR visitors. Two MON files retain standard widths pending Phase 6, outside this current profile-runtime boundary. |
| `src/{build_system,projects,backends,first_party_js,compiler_tests/integration_test_runner}/**` | 101 | Inspected build/config/template profile handoff, JS carrier/runtime/foreign glue, selected Wasm feature gates and rendered-output terminal protocol. Mechanically traced remaining fixture/API threading and deliberately gated Wasm emission changes. |
| `tests/cases/**` | 259 | Checked the changed 86 case groups: 84 surviving groups have registered IDs and config/source/expectation triplets, two retired groups have no live registrations. The three renames, added auxiliary sources and manifest entries were reconciled. Inspected representative dynamic arithmetic, conversion, trap, generic, profile and Wasm-gate assertions; remaining fixture triplets were mechanically matched to manifest/backend/profile selection and the executable suite. |
| `docs/src/**`, `docs/roadmap/plans/number_type_numeric_plan.md` | 15 | Compared changed contract/support statements with the progress matrix and active plan. Eight changed documentation pages still describe fixed-width/Byte runtime values as gated on both backends or do not distinguish the HTML-JS delivery; documentation correction belongs to the plan's Phase 9 sweep, not this Correctness audit. |

The following **disjoint disposition rules** apply only to the 620 records returned by the pinned command. Expand braces as literal path alternatives; a rename keeps one record under its destination path. Check the outside set first, then the inspected set; every other changed path is **mechanically traced** through its patch hunk, owner/call path, manifest registration or scoped full-suite execution as described in the table. “Inspected” means relevant semantic sections and their changed hunks were read, not the complete file. This deliberately classifies shallowly reviewed files as mechanical, even when a reviewer also looked at selected lines. The rules make the remaining review depth recoverable without storing a raw generated inventory.

**Outside numeric boundary (three paths):** `src/compiler_frontend/ast/templates/template_head_parser/directive_args.rs` changes positional slot-key parsing; `src/compiler_frontend/mon/{reader.rs,writer.rs}` retain standard-width MON behavior for the separate Phase 6 migration.

**Inspected semantic owners (exact path sets):**

- AST literals/folds/operators: `src/compiler_frontend/ast/const_eval/mod.rs`, `src/compiler_frontend/ast/const_values/{resolver.rs,store.rs}`, `src/compiler_frontend/ast/expressions/eval_expression/{evaluator.rs,result_type.rs,operator_policy/{arithmetic.rs,comparison.rs,shared.rs,unary.rs}}`, `src/compiler_frontend/ast/expressions/{call_arguments.rs,call_validation.rs,choice_constructor.rs,generic_nominal_inference.rs,mutation.rs,struct_instance.rs}`. These own C1, checked folding and contextual value typing.
- AST generated values/templates: `src/compiler_frontend/ast/generic_functions/materialisation/{artefact_emit.rs,preparation_freeze.rs,sidecar_build.rs}`, `src/compiler_frontend/ast/module_ast/environment/builder/import_projection/values.rs`, `src/compiler_frontend/ast/templates/{template_renderability.rs,template_control_flow/const_folding.rs}`, `src/compiler_frontend/ast/type_resolution/collections.rs`. These own generic profile transport, folded-value reconstruction and F01/F02.
- Shared scalar/cast policy: `src/compiler_frontend/{datatypes/{environment.rs,fixed_scalar.rs,numeric_operators.rs,numeric_profile.rs},folded_value.rs,numeric_text/{binary16.rs,format.rs,parse.rs},builtins/casts/{evidence.rs,policies.rs},type_coercion/contextual.rs}`. These own direct versus optional type identity, precision, cast evidence and text conversion.
- HIR/front-end boundary: `src/compiler_frontend/hir/{hir_expression/{numeric.rs,runtime/tree_lowering.rs,templates/render_append.rs},numeric.rs,validation/blocks.rs}`, `src/compiler_frontend/{build_config.rs,pipeline.rs,public_interface/interface_validation.rs,module_compilation/generated/materialisation.rs,single_source_compilation/{config.rs,moth_template.rs}}`. These own operation/failure validation, profile inputs and generated sidecars.
- Build/runtime boundary: `src/build_system/{build.rs,create_project_modules/{compilation.rs,config_boundary.rs,mod.rs},project_config.rs}`, `src/projects/{check.rs,cli.rs,html_project/{external_js/runtime_glue/source.rs,moth_template/compile.rs,html_project_builder.rs}}`, `src/backends/{backend_feature_validation.rs,js/{numeric_carrier.rs,js_expr.rs,js_statement.rs,emitter.rs,runtime/{numeric.rs,casts.rs},package_bindings/core/{random.rs,text.rs}},wasm/backend.rs}`, `src/compiler_tests/integration_test_runner/{assertions/rendered_output.rs,expectations.rs,fixture.rs,execution.rs}`. These own selected-profile propagation, JS carrier checks, F03 and trap observations.
- Behavioral fixtures: `tests/cases/{compound_assignment_atomicity_runtime,fixed_width_integer_checked_boundaries,fixed_width_binary_float_checked_boundaries,fixed_width_cast_runtime_recovery,numeric_profile_int64_float32_conversion,fixed_width_reachable_value_js_runs_wasm_gated}/{expect.toml,input/src/@page.moth}`. Their source and expectation hunks assert observable results or target diagnostics. Their config files and all other case records, including deleted paths and manifest entries, remain mechanically traced by the canonical case inventory.
- Documentation: every changed `docs/src/**` path and `docs/roadmap/plans/number_type_numeric_plan.md`; compared against the progress matrix, named support status and deferred phases.

Reproduce the complete file inventory with `git diff --name-status bf65cb05b4ecb2536655794ea2ce0313304562f3 08c62a0307d23794a2be4c8453943efa1bfec5f6`. The pinned patch, not a directory listing or a passing-suite total, supplies the file classification. Current code was read at the candidate checkpoint in the numeric materialisation, range, template, runtime and external-error owners named below.

## Authorities read

`docs/roadmap/plans/number_type_numeric_plan.md`; `docs/roadmap/plans/numerics-slice4c-audit.md` (original user-owned handoff); `docs/src/docs/progress/@page.moth`; `docs/src/docs/loops/range-loops.mtf`; `docs/src/docs/errors/error-values-basic.mtf`; `docs/roadmap/audit-guide.md`; `docs/roadmap/audit-kinds/correctness.md`; `docs/roadmap/audits/README.md`; repository instructions.

## Existing findings and active plans checked

Original handoff C1/C2 and linked T1/T2, R1/R2 and linked T3, K1/K2; numeric plan Phases 5–9. C1, C2, R1 and R2 were corrected and independently audited at `de3829543`, `359fe68ac`, `8f62942f3` and `19959a4ae`. K1 is reproduced under Int32/Int64 and explicitly gated for Phase 8 by `f67270a0c`; it is not a new finding here. K2 is a bounded evidence limit, not a missing source feature. The original report remains the unchanged handoff evidence.

## Findings

### AUD-0007-F01: Optional fixed scalars escape template renderability checks

- State: `fixed`
- Kind: `Correctness`

#### Evidence

`src/compiler_frontend/ast/templates/template_renderability.rs:40-42` calls `TypeEnvironment::fixed_scalar_of`, which unwraps `Option<T>` (`datatypes/environment.rs:1494-1496`). Template-head validation therefore accepts an `Option<U8>` as directly renderable. `src/compiler_frontend/hir/hir_expression/templates/render_append.rs:1454-1466` repeats the unwrap and emits `NumericToString(U8)` with an `Option<U8>` source. A disposable HTML `moth check` of `maybe U8? = 200` followed by `[:optional value=[maybe]]` reproduced `MOTH-INFRA-0001` (`Cast policy NumericToString(Fixed(U8)) source type does not match the policy source type`), rather than a source diagnostic. The disposable project was removed.

#### Counter-explanation tested

Options may render through a distinct template policy, but the explicit template allowlist names direct scalar/text types only; `Option<Int>` does not gain scalar rendering, and the HIR cast validates its exact source type. The optional unwrap helper exists for destination classification, not string conversion.

#### Violated contract or cost

User-authored unsupported template input reaches an internal compiler error. The AST should reject it before HIR conversion.

#### Root owner

AST template renderability; the matching HIR scalar conversion must retain the same direct-scalar invariant.

#### Suggested correction

Classify only direct fixed numeric scalars in both owners. Preserve Byte rejection and valid direct fixed-scalar formatting.

#### Fix scope and preserved invariants

`ast/templates/template_renderability.rs`, `hir/hir_expression/templates/render_append.rs` and the template integration owner. Keep existing explicit cast policy and source diagnostics.

#### Required validation

Assert a source-level rejection for `Option<U8>` template interpolation and successful direct `U8` interpolation on HTML-JS; exercise the same source boundary for optional `Byte`. Run `just validate`.

#### Linked findings

AUD-0008-F01 proposes source-diagnostic and direct-scalar template regression coverage. Both linked findings require triage acceptance before code or tests change. Existing optional `String` and Byte rejection fixtures do not cover this optional fixed-scalar HIR escape.

#### Triage record

2026-09-27 — **Accepted.** The real compiler reports `MOTH-INFRA-0001` for an optional fixed-scalar template chunk; `fixed_scalar_of` unwraps its option in the AST allowlist and HIR cast emission. The direct-scalar `fixed_scalar` accessor already preserves the intended boundary. Correct both owners, keeping Byte excluded and direct numeric rendering intact. Linked Tests finding AUD-0008-F01 is accepted independently; source diagnostics and rendered output must prove the user-visible fix.

2026-09-27 — **Accepted and resolved.** Both AST allowlisting and HIR numeric text lowering now call exact-type `fixed_scalar`, and the unused option-unwrapping `fixed_scalar_of` method was removed. `moth check` reports `MOTH-SYNTAX-0022` on authored optional U8 at `@page.moth:3:19` rather than `MOTH-INFRA-0001`; a disposable optional Byte source likewise reports `MOTH-SYNTAX-0022` at `@page.moth:2:19`. Direct U8 renders `200` through the existing numeric text policy; Byte rejection and HTML-Wasm's `MOTH-RULE-0064` gate remain. Focused integration cases and all four numeric profiles passed; `just validate` passed (5475 unit, 2085/2085 integration, docs check, clippy, benchmarks and scaling). Independent phase reviews of semantic boundaries and regression ownership returned clean.

### AUD-0007-F02: Int64 const-template ranges require an overflowed terminal successor

- State: `candidate`
- Kind: `Correctness`

#### Evidence

`src/compiler_frontend/ast/templates/template_control_flow/const_folding.rs:241-251` increments the Int cursor with `checked_add` before returning the current value, even on the final inclusive iteration. The Float cursor similarly checks progress after emitting its terminal counter (`:275-291`), and its constructor checks progress before detecting an empty/equal-end range (`:189-195`). A disposable HTML Int64/Float64 case with `#[loop 9223372036854775807 to & 9223372036854775807 |counter|: [counter]]` expected one iteration but compilation rejected it as `MOTH-SYNTAX-0022` (“bounds must fold to numeric values”). The disposable fixture and manifest entry were removed.

#### Counter-explanation tested

A range may need a checked update between iterations, but equal inclusive bounds need exactly one iteration, and no update is needed after the final endpoint. The i64 cursor currently masks the same case at Int32 maximum because its storage can advance beyond that profile; this does not establish correct Int64 termination.

#### Violated contract or cost

`docs/src/docs/loops/range-loops.mtf:37,93-97` and the numeric plan's endpoint rule require the last valid inclusive endpoint to complete normally without an out-of-range sentinel.

#### Root owner

Const template range cursor termination in `ast/templates/template_control_flow/const_folding.rs`.

#### Suggested correction

Determine terminal status before computing a successor, while still checking genuinely needed updates, progression and the configured iteration limit.

#### Fix scope and preserved invariants

Cursor stepping and existing const-template range tests. Preserve descending direction, exclusive/empty bounds, Float precision, nonprogress rejection for nonterminal ranges and iteration budgets.

#### Required validation

Run Int64 maximum and minimum singleton/near-end descending cases, equal Float32 inclusive/empty bounds and a nonterminal stalled Float control. Execute template cases and `just validate`.

#### Linked findings

AUD-0008-F02 proposes endpoint successes and a meaningful nonterminal-stall control. Both linked findings require triage acceptance before code or tests change. The current `template_const_float_range_non_progress_rejected` input is `1.0 to 1.0` with an exclusive end, so its expected failure conflicts with the empty-range contract.

### AUD-0007-F03: Standard-profile external errors admit out-of-domain `Error.code`

- State: `candidate`
- Kind: `Correctness`

#### Evidence

`src/projects/html_project/external_js/runtime_glue/source.rs:112-114` uses `typeof error.code === "number" ? error.code : 0` for the standard Int32 profile, then inserts the result directly into builtin `Error.code` (`:142-145`). Isolated Node execution of this expression accepts `1.5`, `Infinity`, `2147483648` and `-2147483649` as codes; all fail the Int32 domain check. The adjacent Int64 branch uses `Number.isSafeInteger` plus profile bounds. `src/projects/html_project/external_js/tests/runtime_glue_tests.rs:335-339` pins the permissive generated expression, not the value contract.

#### Counter-explanation tested

The successful-value external ABI gate does not validate compiler-owned Error payloads. `__moth_error_code` checks a carrier type, not its numeric range. The plan's later `U32` migration does not make nonintegral or nonfinite values valid today.

#### Violated contract or cost

The current `Error.code` field has type `Int` and the default profile rejects values above `2147483647` (`docs/src/docs/errors/error-values-basic.mtf:14-16`); foreign wrapper output bypasses that invariant.

#### Root owner

External-JS fallible result wrapper's Error projection.

#### Suggested correction

Apply the same integer/profile-range check to standard-profile codes before constructing a Moth Error. Preserve the current zero fallback for invalid codes and existing negative Int codes; leave the later U32 migration to Phase 8.

#### Fix scope and preserved invariants

`src/projects/html_project/external_js/runtime_glue/source.rs` and executable wrapper tests; keep successful-value ABI validation and non-standard carriers unchanged.

#### Required validation

Execute generated standard and Int64 fallible wrappers in Node with valid endpoints, fractional/nonfinite/out-of-range/string inputs and thrown errors. Run `just validate`.

#### Linked findings

AUD-0008-F03 proposes executable wrapper value-domain coverage. Both linked findings require triage acceptance before code or tests change. The current Rust test asserts the permissive generated expression, which must be deleted rather than pinned to a replacement spelling.

## Checked and clean

The corrected C1 path materialises direct/optional `Float` receivers and immediate peers at selected Float precision instead of profile Int range; four-profile HTML cases passed. The C2 emitted Core Random helper preserves the half-open interval under Float32 rounding while Float64 remains unchanged; the deterministic endpoint cases passed. R1's terminal protocol is shared across HTML and Wasm adapters without reclassifying engine failures as Moth traps; R2's Text length helper selects its return expression without incidental string replacement. Config inputs and source defaults use the selected profile; module/frontend/build/template owners pass it to generated materialisation, JS lowering and target validation. HIR checks numeric operation domains and failure carriers. The nine changed Wasm paths keep reachable fixed-width values and non-standard profiles gated pending Phase 5.

## Limitations

Coverage is partial for the named boundary: every pinned changed-file record was classified by patch role, but mechanical traces are not deep per-file proofs; no unmodified numeric consumer inventory, exhaustive floating-point proof, engine-compatibility matrix or comprehensive borrow/CFG audit was run. `just validate` exercises the suite at this checkpoint, not the proposed corrections F01–F03. K1 stays a tracked Phase 8 conformance gap. Phase 5 Wasm scalar execution, Phase 6 MON/foreign ABI adaptation and Phase 7 Number are deliberate gates, not findings. A metadata fingerprint without explicit profile bytes has no current cross-profile cache consumer, so no correctness defect was established. The eight stale support-status pages identified above need the plan's Phase 9 documentation sweep; this Correctness run does not edit canonical docs or claim a Documentation audit.
