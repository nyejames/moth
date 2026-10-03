# AUD-0012: Optional defaults correctness

- State: `complete`
- Kind: `Correctness`
- Area: `frontend.optional_defaults` — optional parameters and fields with a present default, limited to the reproduced explicit-value override behavior.
- Coverage: `partial`
- Reviewed: `2026-10`
- Baseline: Branch `feature/mon-crate-extraction`, reviewed tip `be0405afd0f74e593d91310e5ed86f74ed574522`. The external reviewer had neither Cargo nor rustc and did not execute this handover-reported issue. The implementer later reproduced the field and function cases with `moth check` on the branch at `0f3439fda`. Only those reproductions and the cited option contract were inspected; no root cause was established.

## What was inspected

The nominal-field and function-parameter reproductions below, plus the optional-value contract example in `docs/src/docs/errors/options.mtf:7`. No compiler implementation path or broader optional-default test matrix was inspected, so the root cause remains open.

## Authorities read

`docs/roadmap/audit-guide.md`; `docs/roadmap/audit-kinds/correctness.md`; `docs/roadmap/audits/README.md`; `docs/roadmap/audit-log.md`; `docs/roadmap/open-audit-findings.md`; and `docs/src/docs/errors/options.mtf`, which defines `T?` as either `T` or absent, shows `fallback String? = "guest"`, says `none` needs an immediate optional receiver, and says optional values are not automatically unwrapped. The originating handover's `Decisions that should stay` section was also checked: signature/default changes outside the extraction are not to be absorbed incidentally.

## Existing findings and active plans checked

The audit log and open-findings index were checked. The external review's handover note described this optional-default issue but had not executed it or established its root cause. The implementer's later reproduction confirms an actionable supported-semantics defect, separate from the MON/source parity findings and not an accepted deferred feature. No existing finding owns this field/parameter behavior.

## Findings

### AUD-0012-F01: Optional receivers with present defaults reject explicit optional values

- State: `accepted`
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

Compiler signature/default handling is the root owner to investigate. The exact root cause and implementation path have not been established.

#### Suggested correction

Trace how compiler signature/default handling applies a default when a caller supplies an optional value. Preserve the default for omission, preserve explicit `none`, and preserve an explicitly supplied present optional value. Keep the correction in the compiler owner; do not change MON or broaden the extraction scope without new evidence.

#### Fix scope and preserved invariants

Open and unresolved. A later fix must cover both nominal fields and function parameters, and preserve the distinction between omitted, explicitly absent and explicitly present optional values.

#### Required validation

Add paired regressions for nominal fields and function parameters: omission retains the present default, explicit `none` remains none, and an explicitly supplied present optional retains its optional type. Verify diagnostics or resulting values at the user-visible boundary and retain the `none`-default control case.

#### Linked findings

None. This is not a MON/source-parity finding and is not deferred work.

#### Triage record

2026-10-03 — **Accepted.** The implementer's real-file reproductions confirm that present optional defaults reject both explicit `none` and an explicitly present optional value for nominal fields and function parameters, while omission succeeds. Track the compiler signature/default owner as the investigation boundary, but do not claim a root cause. This is supported-semantics work, not deferred MON parity work; the paired validation must preserve omission, absent and present-value behavior.

## Checked and clean

The omission cases succeed, which bounds the defect to explicit overrides rather than all uses of present optional defaults. A `none` default accepts explicit `none`, which provides a useful control and distinguishes the reproduced present-default case. The canonical contract explicitly says optional values are not automatically unwrapped.

## Limitations

Coverage is limited to the reproductions above and the cited option contract. No signature/default implementation trace, root cause, compiler test suite or broader parameter/field matrix was inspected. The finding remains accepted and open; no fix or validation result is claimed.
