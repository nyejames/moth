# AUD-0008: Numeric boundary regression tests

- State: `complete`
- Kind: `Tests`
- Area: `contract.numeric_profile.regression_tests` — fixed-scalar template, const-template range and external-JS fallible wrapper regression owners.
- Coverage: `partial`
- Reviewed: `2026-09`
- Baseline: `numeric-type-expanding` at `f67270a0c`; `just validate` passed before correcting AUD-0007-F01–F03. Existing tests encode current range/nonprogress and wrapper projection behavior. The original untracked audit handoff remains unchanged.

## What was inspected

`tests/cases/manifest.toml` case ownership; `tests/cases/{template_head_rejects_option_value,cast_numeric_text_byte_render_rejected,cast_numeric_text_fixed_render_success,template_const_range_inclusive_exclusive_edges,template_const_float_range_non_progress_rejected,template_const_float_range_descending_success}/` source and expectation paths; the Int64 numeric-profile integration configuration; `src/projects/html_project/external_js/tests/runtime_glue_tests.rs`; `tests/cases/external_numeric_profile_error_code/` including its authored provider JS and expectation. The relevant AST/HIR range and template owners, JS glue generator, progress matrix and rendered-output harness limitations were context, not a complete Tests audit of those subsystems.

## Authorities read

`docs/src/developer-docs/style-guide/testing.mtf`; `docs/roadmap/audit-kinds/tests.md`; `docs/roadmap/audit-guide.md`; `docs/roadmap/plans/number_type_numeric_plan.md`; `docs/src/docs/loops/range-loops.mtf`; `docs/src/docs/errors/error-values-basic.mtf`; repository instructions.

## Existing findings and active plans checked

AUD-0007-F01–F03 are linked correctness candidates; K1 has its own Phase 8 gate. Numeric plan Phases 5–9 remain active.

## Findings

### AUD-0008-F01: Optional fixed-scalar template interpolation lacks a source-diagnostic owner

- State: `fixed`
- Kind: `Tests`

#### Evidence

`tests/cases/template_head_rejects_option_value/` checks a `String?` template value; `cast_numeric_text_byte_render_rejected/` checks direct Byte rejection. `cast_numeric_text_fixed_render_success/` checks explicit casts to String and Wasm gating, not direct U8 interpolation. No existing fixture detects the `Option<U8>` path in AUD-0007-F01, which currently compiles into `MOTH-INFRA-0001` rather than the source-level unsupported-template diagnostic.

#### Counter-explanation tested

Generic option rejection and successful fixed-scalar casts are related but do not exercise the fixed-scalar optional unwrap followed by HIR numeric-to-string lowering. The existing Byte rejection already protects Byte and should not be duplicated.

#### Violated contract or cost

The AST must reject unsupported source interpolation without an internal compiler error; the suite currently cannot distinguish F01 from a passing generic-option or explicit-cast case.

#### Root owner

Canonical template integration cases under `tests/cases/`.

#### Suggested correction

Add one `Option<U8>` template-head rejection case with the stable `MOTH-SYNTAX-0022` unsupported-type reason and authored location; add direct U8 interpolation to an appropriate existing successful fixed-render case. Keep the Byte case and the current Wasm gate.

#### Fix scope and preserved invariants

Only the fixture(s) and canonical manifest; no AST-only substitute for user-facing diagnostics.

#### Required validation

Run the focused HTML cases and `just validate`; assert actual rendered fixed value and source diagnostic identity/location.

#### Linked findings

AUD-0007-F01.

#### Triage record

2026-09-27 — **Accepted.** The existing `String?` rejection, Byte rejection and cast-to-String success cases do not exercise `Option<U8>` or direct U8 interpolation. Authorise one integration diagnostic owner with code, reason and source location and a direct-U8 output assertion in the existing fixed-render case. Preserve the Wasm feature gate and Byte case; linked Correctness finding AUD-0007-F01 is accepted independently.

2026-09-27 — **Accepted and resolved.** `cast_numeric_text_optional_u8_render_rejected` asserts source diagnostic code, unsupported-template reason and location. The existing fixed-render case checks direct U8 output alongside explicit casts and retains its HTML-Wasm feature gate at the updated authored expression span. Direct Byte and optional String rejections are unchanged. Focused cases and the complete 2085/2085 integration suite passed under `just validate`; independent Tests review found no required correction.

### AUD-0008-F02: Empty Float const range is pinned as a non-progress error

- State: `fixed`
- Kind: `Tests`

#### Evidence

`tests/cases/template_const_float_range_non_progress_rejected/input/src/@page.moth:1` uses `1.0 to 1.0 by 0.0000000000000001` with an **exclusive** end; its expectation requires `MOTH-SYNTAX-0022`. The empty range must perform no updates. `template_const_range_inclusive_exclusive_edges/` asserts only unequal small Int bounds and omits the equal/Int64-terminal cases in AUD-0007-F02.

#### Counter-explanation tested

The step is too small to progress from 1.0 at Float precision, but an empty range never evaluates a successor. A valid non-progress rejection needs a nonempty range with another required iteration.

#### Violated contract or cost

The present negative fixture preserves the incorrect early non-progress check and would fail on a valid F02 fix. Equal exclusive and inclusive bounds and the Int64 maximum are unprotected.

#### Root owner

Existing const-template range integration cases: the non-progress diagnostic case and inclusive/exclusive edge case.

#### Suggested correction

Replace the negative case's empty range with a genuinely nonterminal stalled Float range, retaining stable diagnostic identity. Add equal exclusive/inclusive and Int64 maximum endpoint successes to the existing edge owner or a distinct Int64-profile boundary fixture.

#### Fix scope and preserved invariants

Existing fixtures/expectations and the manifest only if a new numeric-profile case is needed; retain descending coverage and iteration-limit policy.

#### Required validation

Run the focused HTML range cases at both Int widths and Float32; assert deterministic output and stable nonprogress diagnostic, then `just validate`.

#### Linked findings

AUD-0007-F02.

#### Triage record

2026-09-27 — **Accepted.** The current exclusive `1.0 to 1.0` fixture is empty by the documented range contract and cannot be used as a nonprogress diagnostic owner. Move its far bound beyond the first value to require a stalled update at Float precision. Test equal exclusive and inclusive Float32 ranges and Int64 maximum/minimum inclusive and descending endpoints in real template fixtures, preserving exact output assertions and the current iteration budget. Linked Correctness finding AUD-0007-F02 is accepted independently.

2026-09-27 — **Accepted and resolved.** The existing negative fixture now uses a nonempty `1.0 to 2.0` Float range that needs a stalled successor. The equal-bound edge fixture uses a tiny nonzero Float32 step and checks the exclusive marker is absent and the inclusive marker occurs exactly once. A new Int64/Float32 endpoint fixture checks both i64 extrema and adjacent ascending/descending counters with exact-once HTML assertions; the mixed Float32 cursor unit checks diagnostic kind independently of constructor/advance timing. Focused cases and `just validate` passed (2086/2086 integration); independent Tests review found and required two test-strength corrections, and fresh verification accepted both.

### AUD-0008-F03: Standard external error-code test pins a permissive expression

- State: `fixed`
- Kind: `Tests`

#### Evidence

`src/projects/html_project/external_js/tests/runtime_glue_tests.rs:335-339` requires the generated `typeof error.code === "number" ? error.code : 0` expression. This only verifies implementation text and directly conflicts with AUD-0007-F03's Int32 range invariant. The Int64 test at `:344-388` executes the generated wrapper in Node, but the standard-profile path has no equivalent behavior check. `tests/cases/external_numeric_profile_error_code/` inspects HTML artifacts rather than executing the module graph; the rendered-output harness rejects module imports.

#### Counter-explanation tested

The Int64 execution test protects BigInt conversion, not the standard carrier. The integration artifact assertion cannot observe a foreign result in this harness and is not runtime coverage.

#### Violated contract or cost

A passing source-text assertion makes the out-of-domain Int32 code appear tested while fractions, infinities and out-of-range values pass through the emitted wrapper.

#### Root owner

Executable generated-wrapper tests in `src/projects/html_project/external_js/tests/runtime_glue_tests.rs`.

#### Suggested correction

Delete the permissive expression pin and execute the standard wrapper in Node with valid signed Int32 endpoints, invalid fractional/nonfinite/out-of-range/string values and thrown errors. Keep the existing Int64 carrier test; do not claim external module runtime coverage from the HTML artifact test.

#### Fix scope and preserved invariants

Rust test owner only. Keep canonical Moth `Error` field projection and existing zero fallback for invalid foreign codes; later U32 migration remains separate.

#### Required validation

Run generated-wrapper Node tests and `just validate`.

#### Linked findings

AUD-0007-F03.

#### Triage record

2026-09-27 — **Accepted.** The source-text assertion pins the exact permissive expression and cannot detect an out-of-domain generated Error. Replace it with executable canonical message and caught-error coverage; add a Node-executed standard-profile wrapper test for signed Int32 endpoints, fractional/nonfinite/out-of-range/string/undefined codes and caught exceptions. Preserve the existing Int64 execution test and the documented zero fallback. Linked Correctness finding AUD-0007-F03 is independently accepted.

2026-09-27 — **Accepted and resolved.** Removed the old source-expression/canonical-field pin and now execute the generated standard wrapper in Node. Assertions cover both signed Int32 endpoints, valid negative and zero codes, adjacent and noninteger invalids, canonical message/code fields, a thrown exception and a successful value; the Int64 Node wrapper test remains. Focused fallible-wrapper tests passed 5/5 and `just validate` passed (5475 unit, 2086/2086 integration); independent correctness and test-strength reviews were clean.

## Checked and clean

Direct Byte interpolation already has a source-level unsupported-type diagnostic and location. Explicit fixed-scalar cast formatting has HTML-JS output and the current HTML-Wasm gate. The existing const-template range edge case covers unequal exclusive/inclusive bounds, and the Int64 wrapper test executes real generated JS with a BigInt code and invalid carrier fallback. These do not cover F01–F03's missing transitions.

## Limitations

Only these linked template/range/foreign-boundary cases and their nearest owners were read. Other fixture families and backend test modules were not audited for Tests coverage. No tests or commands ran during this read-only audit; the reported passing `just validate` was executed separately at the unchanged baseline.
