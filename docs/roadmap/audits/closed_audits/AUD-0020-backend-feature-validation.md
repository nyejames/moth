# AUD-0020: Backend feature validation

- State: `complete`
- Kind: `Correctness`
- Area: `backends.feature_validation` — target-contract validation in `src/backends/backend_feature_validation.rs` before JS/Wasm lowering
- Coverage: `partial`
- Reviewed: `2026-11`
- Baseline: 2026-11-13 tree (no git used in this run per the task instruction, so no baseline SHA is recorded). `just validate` was not run. Empirical checks used `./target/debug/moth` 0.1.12 (`moth build --html-wasm` on disposable projects under `./tmp/aud0020*`). No active plan owns backend feature validation: the HTML mixed-JS/Wasm plan needs major review before activation, and constraints/fast-math work that touches Wasm arithmetic is queued, not active.

Only the Wasm arms were inspected. The `Js` arm is an intentional no-op. `src/backends/external_package_validation.rs` is a separate owner and was not audited.

## What was inspected

- `src/backends/backend_feature_validation.rs` — the full dispatch (`validate_hir_backend_feature_support`, `validate_hir_linked_function_targets`), every Wasm gate, the shared `first_unsupported_*` occurrence traversal, `BackendTypeFacts`, and `search_statement` / `search_terminator` / `search_expression`
- `src/backends/wasm/hir_to_lir/expr.rs` — `lower_expression` variant/collection/cast/concat arms and `is_empty_string_collection`, as the lowering mirror each shape rule claims
- `src/backends/wasm/hir_to_lir/stmt.rs` — checked integer/float lowering, range candidates, float format/validate, call-callee mapping
- `src/backends/wasm/hir_to_lir/terminator.rs` — assertion-message, `NeverCall`, `Match` arms
- `src/backends/wasm/hir_to_lir/function.rs` and `context.rs` — local ABI allocation (`lower_type_to_abi`) and the loop-step trap guard
- `src/compiler_frontend/hir/reachability.rs` — runtime-feature retention (`RuntimeFeatureUseCollector`, statement/terminator/Invoke collection) and `HirBackendSelection`
- `src/compiler_frontend/hir/validation/blocks.rs` and `validation/expressions.rs` — the validated-HIR invariants the gates rely on (cast policy, NumericOp domains, float modes, `StringAppend` shape)
- `src/projects/html_project/html_project_builder.rs` — gate ordering: external support, per-module feature support, then linked targets
- `src/backends/tests/backend_feature_validation_tests.rs` — the executable baseline (gate matrix, precedence, reachability tests)
- Fixture census: all `MOTH-RULE-0064` reason pins under `tests/cases/` (165 fixtures covering 23 of 26 reasons; see Checked and clean)

Context read, not audited: `external_package_validation.rs` (ordering only), `comparison.rs` operator policy, template `coerce_expression_to_string`, `emit/instructions.rs` comparison guard.

## Authorities read

- `docs/compiler-design-overview.md` — `Per-function link facts` (reachability from specialised HIR, type environment where link facts are insufficient), `Target-contract validation` (reachable unsupported features are structured user diagnostics; unreachable code imposes no requirements; assertion messages needing runtime construction require a faithful target), `Backend-facing compiler handoff` (lowerers receive validated inputs; accepted shapes must lower)
- `docs/docs/progress/@entry.moth` — Choices (JS/HTML only, no Wasm lowering claimed), arithmetic/casts (Wasm rejects recoverable failure and fixed-scalar aggregates), templates, Wiring (Wasm rejects routes), named arguments (Wasm rejects mutable parameters)
- `docs/docs/cheatsheet/moth-language-cheatsheet.mtf` — template interpolation spelling, for probe construction only
- `docs/roadmap/audit-kinds/correctness.md`

## Existing findings and active plans checked

Open findings AUD-0017-F01..F07, AUD-0018-F01..F02 and AUD-0019-F01 all own frontend statement/directive/body parsing, a different area with no shared root cause. No roadmap plan owns this validator. Deferred work (declared regions, memory planning, reusable generics, mixed-Wasm lowering) was not filed as absent.

## Findings

### AUD-0020-F01: Reachable payload-free variant values are accepted and then crash Wasm lowering

- State: `closed`
- Kind: `Correctness`

#### Evidence

`search_expression` records `VariantPayloadValues` only when a `VariantConstruct` has non-empty fields (`backend_feature_validation.rs`). A payload-free construction — a unit choice variant or `Option` `none` — is accepted whenever its *type* also passes the structural gates (no fixed scalar, no `Dec`, no `Error`, no direct generic instance).

Three minimal disposable projects, each built with `moth build <dir> --html-wasm`:

```text
Reading ::
    Empty,
    Done,
;
value = Reading::Empty
~[:hello]
```

```text
Wasm Generation Bug
...
Wasm validation failed: type mismatch: expected i32, found i64 (at offset 0x10f)
  Stage: Backend lowering
```

```text
value String? = none
~[:hello]
```

```text
Wasm Generation Bug
...
Wasm validation failed: type mismatch: expected i32, found i64 (at offset 0x10f)
  Stage: Backend lowering
```

Using the value instead of just binding it fails differently but still outside any user diagnostic:

```text
value = Reading::Empty
same = value is Reading::Empty
```

```text
Wasm emission type mismatch in comparison: lhs WasmLirLocalId(1) is Handle, rhs WasmLirLocalId(5) is I64
  Stage: Backend lowering
```

Root mechanism: lowering has no coherent representation for these values. `lower_expression` emits a payload-free construction as an `I64` tag temporary, while `lower_type_to_abi` maps every choice/option type to `Handle` (`HeapAllocated`), so the defining local, call arguments and loaded operands are `Handle`s. Assignment then joins an `I64` producer to a `Handle` local (caught only by final Wasm validation), and comparison joins a `Handle` load to an `I64` construction (caught only by the emission guard). Passing one as a call argument fails the same way (`passed` probe: `type mismatch: expected i32, found i64` at `0x145`).

The reachability boundary itself works: the same choice type constructed only inside an uncalled helper builds successfully, and `Some("hello")` is correctly rejected as `VariantPayloadValues`. Only reachable *payload-free* constructions slip through.

#### Counter-explanation tested

**"Unit variants are supported and lowering's I64 tag is correct; the bug is only the local ABI mapping, so this belongs to Wasm lowering."** Rejected. Lowering disagrees with itself per use site (locals/args `Handle`, fresh constructions `I64`), so no single-carrier reading makes acceptance correct; a choice type with payload variants elsewhere must still hold payload values in the same locals. The validator's own contract (`validate_wasm_unsupported_shapes`: "a shape accepted here must lower") is what fails observably. Triage may still fix this on the lowering side with a designed representation, but validator rejection is the bounded correction consistent with the progress matrix, which claims Choices only for JS/HTML.

**"This is the known deferred aggregate/choice Wasm work, so filing it reports deferred work as a defect."** Rejected. The deferred work is *lowering* those values; the defect is that validation *accepts* them and the user gets a backend-lowering crash instead of the structured `MOTH-RULE-0064` diagnostic the target-validation contract requires. Accidental exposure of deferred surface is filable; absence of the lowering is not what is filed.

Disproof would be any of these probes producing either valid Wasm or a structured `MOTH-RULE-0064` diagnostic.

#### Violated contract or cost

`Target-contract validation`: unsupported reachable features must be structured user diagnostics. Instead, legal source the frontend accepts crashes the Wasm backend with an internal error (`Wasm validation failed`, `Wasm emission type mismatch`), violating the validator's "accepted shapes must lower" rule and the user/compiler-bug failure-lane boundary. Three distinct crash signatures share this root cause.

#### Root owner

The `VariantConstruct` arm of `search_expression` in `src/backends/backend_feature_validation.rs`, which exempts empty-field constructions from `VariantPayloadValues`.

#### Suggested correction

Non-authorising. Reject reachable payload-free `VariantConstruct` values in the shapes gate, while preserving the two assertion-message carve-outs: the existing `Folded` arm (which already skips the carrier) and the `Default` carrier (currently visited harmlessly only because empty fields record nothing — it must be skipped once empty constructions reject, since default messages lower to a static string). Do not change the rule that unreachable constructions impose no requirement.

#### Fix scope and preserved invariants

That gate arm plus the `AssertFailure` terminator carve-out only. Preserve: default/folded assertion messages still lowering on Wasm; `Some` payload rejection keeping its current reason; unreachable helpers still not gating; deterministic authored-first diagnostic choice. Do not invent a Wasm variant representation or touch `lower_type_to_abi`.

#### Required validation

Each probe above must report `MOTH-RULE-0064` at an authored span instead of a backend-lowering failure. Default, folded and runtime assertion-message behaviour on Wasm must be unchanged (accepted, accepted, rejected respectively). The uncalled-helper probe must still build. `just validate`.

#### Linked findings

None.

## Triage (2026-11-13)

**F01 accepted** after source review at `fd695d18b`. The reachable-shape check only records `VariantPayloadValues` when a `VariantConstruct` has fields. The Wasm lowerer emits an `I64` for an empty constructor, while `lower_type_to_abi` assigns `Handle` to that option/choice type. This proves incompatible carriers at the handoff, consistent with the report's three build failures. Reject unsupported reachable constructions before lowering. Preserve the folded and default assertion-message exceptions because their lowerer paths never consume the variant carrier. Do not implement variant storage as part of the audit follow-up. Fresh execution was unavailable without a compiler binary or Rust toolchain.

## Resolution (2026-11-13)

**F01 fixed.** The `VariantConstruct` arm of `search_expression` no longer exempts empty-field constructions. A payload-bearing construction keeps `VariantPayloadValues`. A payload-free one (unit choice variant, `Option` `none`) records the new `PayloadFreeVariantValues` reason (`unsupported_backend_feature.payload_free_variant_values`, rendered "payload-free variant values"), so the user receives `MOTH-RULE-0064` at the authored construction instead of a backend-lowering failure. A new reason was needed because `VariantPayloadValues` renders "variant payload fields", which is false for a value without fields, and its wording is pinned for `Some` by `cast_optional_success_wraps_inner_value`. The `AssertFailure` arm now skips the `Default` message carrier explicitly, because `terminator.rs` lowers that message as a static string without reading the carrier. The `Folded` carve-out is unchanged. No lowering, `lower_type_to_abi` or JS change.

Regression cases `wasm_payload_free_choice_variant_rejected` (reachable unit choice binding) and `wasm_payload_free_option_none_rejected` (`none` as a call argument) pin the reason and authored line, with HTML-JS success lanes. Existing tests were not modified.

An earlier verifier objected that `Reading::Empty is Reading::Done`, two fresh constructions compared, lowered before the fix and is now rejected. The coordinator did not accept this. The progress matrix claims choices only for JS/HTML, no passing HTML-Wasm case constructs a choice or option value, and carving out a constant comparison would keep accidental exposure of deferred surface behind a context-sensitive exception. The final verifier agreed.

**F01 verified and closed.** An independent verifier, not the implementer, re-ran the original probes and the call-argument probe on a rebuilt debug compiler. Each reports `MOTH-RULE-0064` at an authored span. A reachable `return`, an omitted default parameter, value-`if` branches and a compile-time `#=` constant used at runtime are also gated. Struct fields, collections and match arms are still intercepted by their existing parent reasons. An uncalled helper and an uncalled default parameter still build. Default and folded assertion messages still build, and runtime messages are still rejected with `RuntimeAssertionMessages`. The verifier ran clippy, the format check, `cargo test -p moth` for `backends::`, `compiler_messages` and `integration_test_runner`, and `moth tests --backend html_wasm --terse` (508/508). The coordinator's `just validate` result is recorded in the closeout commit message. Implementation changed target validation materially, so the audit-log row is marked stale.

## Checked and clean

- Gate ordering and priority: routes, fixed scalars, `Dec`, assertions, cross-module calls, maps, casts, numerics, range candidates, floats, loop-step failures, generics, errors, fallible flow, mutable parameters, mutable aliases, shapes. Narrower type/operation reasons keep priority over later gates (pinned by `wasm_mutable_parameter_gate_preserves_existing_type_reason_precedence` and the builder running linked targets last).
- Reachability boundary: unreachable helpers impose no requirement. Probed (uncalled function returning a choice builds) and pinned by unit tests for unreachable numerics, floats, generics and assertion messages.
- Numeric delivery: trap-mode vs `Invoke` classification is exact. Narrow-integer domains, F16 operations, `IntegerDivide` on float domains and F16 range candidates are rejected as capability limits; supported trap-mode integer/float operations stay accepted (unit-test matrix plus lowering's matching `compiler_bug!` arms).
- Casts: only evidence-approved infallible expression conversions pass; lowering re-verifies fallibility and type identity and returns infrastructure errors on drift, so the validator is the user-facing gate and lowering is defense in depth.
- Cross-module calls: statement, `Invoke` and `NeverCall` sites are all scanned, and the `compiler_bug!` fallback is unreachable by construction (reachability facts exist only for selected blocks; those three positions are the only `CallTarget` sites).
- Fixture census: 165 `MOTH-RULE-0064` fixtures pin 23 of 26 reasons. `CrossModuleCalls`, `FloatFormatting` and `FloatBoundaryValidation` have no `tests/cases` pin and are covered by Rust unit tests instead.
- Fallible control flow: `Invoke`, `ReturnError` and fallible signatures are all covered with authored-span preference and spanless fallback.
- Mutable parameters and aliases: entry-block parameter resolution, the direct-self-update exemption and the `ValueKind::Place` test are sound. `Copy` lowers as `RValue`, so explicit copies pass (probed: the `copy` variant builds while the alias fixture `mutable_local_alias_wasm_rejected` pins rejection).
- Unsupported-shape walker: exhaustive over selected statements, terminators and expression trees through the shared child walker; `PushRuntimeFragment`, match scrutinee/pattern/guard and `Invoke` operands are all visited. `Bool`/`Char` template chunks are rejected (`wasm_bool_template_chunk_rejected` fixture plus probes); `Byte` chunks never arrive (frontend `MOTH-SYNTAX-0022`, probed); narrow-integer chunks are accepted and lower (probed `U8`/`I8`/`U16`/`I16`); nested `StringAppend` chains are fully descended.
- `BackendTypeFacts`: cycle-safe. Open-cycle visits are never memoised, so recursive nominal types recompute rather than cache provisional facts; scalar/error/number facts still propagate to the outermost querier.
- `Dec`: literals, comparisons and conversions are all rejected before lowering's `compiler_bug!` arms via the `NumberValues` shape scan and the cast gate.

## Limitations

Partial. Not inspected:

- `src/backends/external_package_validation.rs` (separate owner; only its call order in the builder was read).
- The `Js` target arm beyond noting it is an intentional no-op.
- Per-fixture verification of all 165 `MOTH-RULE-0064` cases; the census above plus sampled shape fixtures (`wasm_bool_template_chunk_rejected`, `wasm_choice_match_rejected`, `wasm_non_empty_collection_rejected`, `wasm_unary_not_rejected`) stand in for exhaustive fixture review.
- Nested generic-instance occurrences (a generic instance inside a constructed or nominal type): the gate predicate matches direct `GenericInstance` types only. Nested occurrences were reasoned against construction paths (any construction needs an already-rejected outer shape) but not driven to an empirical verdict; reusable generics are queued, not delivered, so no source producer was available to probe.
- `just validate` was not run. Disposable `moth build --html-wasm` probes are not a substitute for it.
