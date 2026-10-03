# AUD-0012: Optional defaults correctness

- State: `complete`
- Kind: `Correctness`
- Area: `frontend.optional_defaults` — optional parameters and fields with a present default, limited to the reproduced explicit-value override behavior.
- Coverage: `partial`
- Reviewed: `2026-10`
- Baseline: Branch `feature/mon-crate-extraction`, reviewed tip `be0405afd0f74e593d91310e5ed86f74ed574522`. The external reviewer had neither Cargo nor rustc and did not execute this handover-reported issue. The implementer later reproduced the field and function cases with `moth check` on the branch at `0f3439fda`. Only those reproductions and the cited option contract were inspected; no root cause was established.

## What was inspected

The nominal-field and function-parameter reproductions below, plus the optional-value contract example in `docs/src/docs/errors/options.mtf:7`. That audit run inspected no compiler implementation path, so it did not establish the root cause. The later correction is recorded in AUD-0012-F01 and is not a new complete audit.

## Authorities read

`docs/roadmap/audit-guide.md`; `docs/roadmap/audit-kinds/correctness.md`; `docs/roadmap/audits/README.md`; `docs/roadmap/audit-log.md`; `docs/roadmap/open-audit-findings.md`; and `docs/src/docs/errors/options.mtf`, which defines `T?` as either `T` or absent, shows `fallback String? = "guest"`, says `none` needs an immediate optional receiver, and says optional values are not automatically unwrapped. The originating handover's `Decisions that should stay` section was also checked: signature/default changes outside the extraction are not to be absorbed incidentally.

## Existing findings and active plans checked

The audit log and open-findings index were checked. The external review's handover note described this optional-default issue but had not executed it or established its root cause. The implementer's later reproduction confirms an actionable supported-semantics defect, separate from the MON/source parity findings and not an accepted deferred feature. No existing finding owns this field/parameter behavior.

## Findings

### AUD-0012-F01: Optional receivers with present defaults reject explicit optional values

- State: `closed`
- Kind: `Correctness`

#### Evidence

The contract example `fallback String? = "guest"` in `docs/src/docs/errors/options.mtf:7` is a present optional default. On branch `feature/mon-crate-extraction` at `0f3439fda`, the implementer ran `moth check` against real files and observed:

- `Outer = | name String? = "guest" |` followed by `data #= Outer(name = none)` reports MOTH-RULE-0053.
- With `n String? = "Priya"`, `data = Outer(name = n)` reports MOTH-TYPE-0001.
- Omitting the field with `Outer()` is accepted.
- A field default of `name String? = none` accepts an explicitly supplied `none`.
- `greet |name String? = "guest"| -> String:` with `greet(none)` reports MOTH-RULE-0053; supplying a `String?` argument reports MOTH-TYPE-0001.

The evidence establishes rejection of both explicit absent and present optional values for a receiver with a present default, while omission still applies the default. It does not establish the compiler root cause.

#### Counter-explanation tested

The examples could reflect a rule distinguishing a present default from a `none` default, but the language contract says an optional can hold `T` or absence, and the default belongs to omission. The same rejection occurs for nominal fields and function parameters, so this is not a MON-specific syntax difference. The optional value is not being implicitly unwrapped under the documented contract.

#### Violated contract or cost

A caller cannot explicitly supply either supported optional state to a parameter or field whose optional receiver has a present default. That prevents a supplied value from overriding the default and contradicts the documented optional type contract.

#### Root owner

The original audit named compiler signature/default handling as the owner to investigate and did not establish the implementation path. The later correction trace identifies `signature_member_to_declaration` in `src/compiler_frontend/ast/statements/functions.rs`.

With a present default, that function stored the expression returned by `parse_signature_default_expression`. A present `"guest"` has natural type `String`. The expected `String?` guided parsing but did not insert the `String -> String?` coercion. Later resolution read `Declaration.value`'s type as the member type, so an explicit optional argument was checked against `String`. That is why `none` reported `MOTH-RULE-0053` and a `String?` binding reported `MOTH-TYPE-0001`, while omission still applied the default.

#### Suggested correction

Trace how compiler signature/default handling applies a default when a caller supplies an optional value. Preserve the default for omission, preserve explicit `none`, and preserve an explicitly supplied present optional value. Keep the correction in the compiler owner; do not change MON or broaden the extraction scope without new evidence.

#### Fix scope and preserved invariants

Production correction: `47f04024d`. Boundary regression expansion: `df85e38bc`. MON/source parity regression checkpoint: `523bcf512`. Closed on `optionals-fix` from baseline `a211991ba1e885f5255088f384f682799836aa77`. The correction is `normalise_resolved_signature_default`, called from `signature_member_to_declaration` only when the declared destination is resolved. It coerces the parsed default through the existing `coerce_expression_to_explicit_type_boundary`, stores the declared diagnostic spelling on that node, and restores the authored member access mode. Coercion nodes otherwise keep `DataType::Inferred` spelling, and later resolution still re-derives `TypeId` from `diagnostic_type`, so the retained spelling has to stay aligned with the selected type. It is not a second type authority. Numeric literal coercion rebuilds the node as immutable, which is why the member marker is restored after that rewrite.

The three callers pass an existing `TypeMismatchContext` into that same function: function-signature lowering, struct-field declaration lowering, and member-shell rebuilding in `src/compiler_frontend/ast/module_ast/environment/type_resolution.rs`. They do not coerce the default again. Unresolved annotations stay on the existing deferred path. No-default `NoValue` and an already-typed `none` are unchanged. MON, argument routing and omission selection are unchanged.

Regression evidence at that revision, toolchain `rustc 1.99.0`:

- `cargo test -p moth --lib optional_present_default` passed 11 tests.
- `cargo test -p moth --lib mon_syntax_parity` passed 30 tests.
- `cargo test -p moth-mon` passed 99 tests.
- Primary HTML case `optional_present_defaults` expects the exact rendered omission, absent, present, empty and typed-override states for both a function and a struct.
- Invalid `String? = 42` function and struct cases expect `MOTH-TYPE-0001`.
- A disposable skip of `normalise_resolved_signature_default`, restored before `523bcf512`, failed all 11 `optional_present_default` tests and exactly the five new same-field parity fixtures. Omission, `"Priya"` and `""` lost the optional layer and observed `String`. Explicit `none` was rejected with `MOTH-RULE-0053` `NoneLiteralRequiresOptionalTypeContext`. The typed `rob #String? = "Rob"` override was rejected with `MOTH-TYPE-0001` `ConstructorArgument`. Existing parity fixtures did not fail.

The completion tree's `just validate-full` result is recorded outside this report so the tested tree is not edited to record its own result.

#### Required validation

The paired regressions now exist: semantic identity checks, the primary rendered-output case, boundary, import, numeric and rejection cases, and same-field MON/source parity. The exact-tree `just validate-full` gate remains the completion check and is not claimed here.

#### Triage record

2026-10-03 — **Accepted.** The implementer's real-file reproductions confirm that present optional defaults reject both explicit `none` and an explicitly present optional value for nominal fields and function parameters, while omission succeeds. Track the compiler signature/default owner as the investigation boundary, but do not claim a root cause. This is supported-semantics work, not deferred MON parity work; the paired validation must preserve omission, absent and present-value behavior.

2026-10-03 — **Fixed.** `signature_member_to_declaration` now normalises a resolved present default to the declared member type before that expression becomes the declaration's type carrier. The production correction is `47f04024d`; boundary coverage is `df85e38bc`; the MON/source parity checkpoint is `523bcf512`. Independent verification is still required before closure.

2026-10-03 — **Closed.** An independent read-only review, not the implementer, confirmed the root owner, the single normalisation boundary and the absence of a parallel default path. The finding leaves the unresolved index. The original audit coverage stays partial and is marked stale because the producer behaviour it recorded has changed. `just validate-full` remains the completion gate and is not claimed in this report.

## Checked and clean

The omission cases succeed, which bounds the defect to explicit overrides rather than all uses of present optional defaults. A `none` default accepts explicit `none`, which provides a useful control and distinguishes the reproduced present-default case. The canonical contract explicitly says optional values are not automatically unwrapped.

## Limitations

The original audit coverage remains the reproductions above and the cited option contract. It did not inspect the implementation, and this report does not claim a new complete structured audit of `frontend.optional_defaults`. The fix record in AUD-0012-F01 is later implementation evidence, verified and closed by a reviewer other than the implementer.
