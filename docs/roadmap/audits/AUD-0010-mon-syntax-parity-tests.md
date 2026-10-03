# AUD-0010: MON syntax parity tests

- State: `complete`
- Kind: `Tests`
- Area: `tests.mon_syntax_parity` — cross-parser parity fixtures under `src/compiler_tests/mon_syntax_parity/**`, codec-local cases under `crates/moth-mon/src/tests/**`, and dependency-direction fixtures in `xtask/src/first_party_deps/tests.rs`.
- Coverage: `partial`
- Reviewed: `2026-10`
- Baseline: Branch `feature/mon-crate-extraction`, reviewed tip `be0405afd0f74e593d91310e5ed86f74ed574522`. The external reviewer had neither Cargo nor rustc and source-traced the fixtures through the connected GitHub reader. The implementer later executed reproductions and tests on the branch; those were not reviewer-run. Scoped package/parity/xtask tests and clippy passed for the fixes.
  Final-tree gate: `just validate-full` passed on the final tree (commits `345205680`, `3c5c51ced`, `0f3439fda` plus the closeout commit that also routes the lexer's remaining inline LF/CR checks and the string-escape newline check through `moth_lexical::is_line_break`): 10/10 feature lanes; 5,453 default `moth` unit tests (5,588 with feature lanes); `moth-mon` and `moth-lexical` tests; 850 `xtask` tests; 2,002 integration cases (2,230 backend executions, 0 failures); all 82 benchmarks passed preflight; the quick perf comparison reported no regression.

## What was inspected

The parity fixture system and its source-observation helpers, especially `gaps.rs`, `support.rs`, `named_entries.rs`, `perturbations.rs`, `nominal.rs` and `choices.rs`; MON reader/schema unit-test owners; and first-party dependency-direction fixtures and checks in `xtask`. The external review compared the parity assertions with the source and MON receiving contexts and identified test-boundary gaps. This was not an audit of all compiler, codec or xtask tests.

## Authorities read

`docs/roadmap/audit-guide.md`; `docs/roadmap/audit-kinds/tests.md`; `docs/roadmap/audits/README.md`; `docs/roadmap/audit-log.md`; `docs/roadmap/open-audit-findings.md`; `docs/src/developer-docs/style-guide/testing.mtf` (including MON and shared lexical coverage); `docs/src/docs/language-overview/mon-syntax.mtf`; `docs/src/docs/mon/mon-format.mtf`; `docs/src/docs/collections/hash-maps.mtf`; `docs/src/docs/choices/variant-construction.mtf`; the compiler/build ownership contracts; the originating extraction plan; and the repository instructions.

## Existing findings and active plans checked

The audit log and open-findings index were checked. The external review's local findings T01–T03 were mapped to this report's F01–F03. Its linked correctness findings C01–C03 and redundancy finding R01 are tracked in AUD-0009 and AUD-0011. The originating extraction plan's four parity classifications and restriction against incidental source-language expansion were checked. These audit IDs were reserved for the linked repository reports; the review's T identifiers were local.

## Findings

### AUD-0010-F01: Deferred-feature gap fixtures lack equivalent typed source receivers

- State: `closed`
- Kind: `Tests`

#### Evidence

Two existing gap fixtures supplied MON with receiving information that their source side lacked. `state = ::Ready` gave MON a `Theme` schema, while source used an anonymous const-record envelope with no `Theme`-typed receiver. `scores = {=}` gave MON a `String`-to-`Int` map schema, while the source case used an anonymous const-record envelope with no typed runtime receiver; that could make the source side fail for an unrelated receiver or constness reason even if the unsupported-syntax assertion changed. The accepted design fixes container kind: `{=}` is an empty map and `{}` remains a collection regardless of receiver. Source `{=}` remains unimplemented.

#### Counter-explanation tested

An exact current diagnostic assertion can catch some behavior changes, but it does not prove source/MON parity under equivalent receivers or prove that an intended deferred capability works. The fixture needs a valid source context without implementing the deferred feature.

#### Violated contract or cost

Parity fixtures must compare equivalent receiving information and must not pin a failure that could persist for an unrelated type or constness error. Otherwise the gap suite cannot become a valid positive parity case when the feature ships.

#### Root owner

Source overrides and receiver setup in the parity fixture system, including `gaps.rs` and its observation helpers.

#### Suggested correction

Use the existing source override and typed observer. Give contextual choices a concrete choice-typed field or binding; give the empty-map gap a runtime binding with explicit key/value types so the fixture exercises the deferred source `{=}` form with a typed receiver. Do not implement contextual choices or `{=}` as part of this test correction.

#### Fix scope and preserved invariants

Resolved in commit `0f3439fda`. The choice case uses `Holder = | state Theme |` and `data = Holder(state = ::Ready)`. The map case binds `scores {String = Int} = {=}` in a runtime declaration before passing it through `MapHolder(scores = scores)`, giving the deferred source spelling its intended typed receiver. Source `{=}` remains unsupported; no deferred feature was implemented. `MapHolder(scores = {})` was checked separately and reports MOTH-TYPE-0002 on the current compiler. That observation is not a defect against the accepted design: `{}` is a collection regardless of receiving context, while `{=}` is the accepted empty-map spelling and remains undelivered in source. The progress matrix separately records the current compiler's contextual `{}` behavior.

#### Required validation

The corrected cases must preserve the intended gap diagnostic while source provides valid, equivalent receiving context, and must not weaken neighboring assertions. Scoped parity tests and clippy passed; `just validate-full` passed.

#### Linked findings

AUD-0010-F02 strengthens the surrounding boundary coverage.

#### Triage record

2026-10-03 — **Accepted.** The current failures can arise from missing source receiver/type context, so they cannot establish the intended parity gap. Authorise fixture-context correction only; do not implement the deferred choice or empty-map syntax.

2026-10-03 — **Fixed and closed.** Commit `0f3439fda` gives the contextual choice a typed `Holder` receiver and the empty map an explicitly typed runtime binding passed through `MapHolder`. The direct constructor argument `MapHolder(scores = {})` was verified to produce MOTH-TYPE-0002, so the binding preserves the required context rather than assuming constructor-slot inference. Scoped parity tests and clippy passed; `just validate-full` passed.

### AUD-0010-F02: Parity tests miss decisive token boundaries and receivers

- State: `closed`
- Kind: `Tests`

#### Evidence

The existing deterministic perturbation generated 54 cases and varied whitespace after `=` and commas, with selected nested and qualified examples. It did not vary trivia before `=` or between a constructor head and its payload opener, where the source and MON readers had different rules. The suite's explicit expected-value assertions could only protect forms present in its corpus. The review also identified the invalid receiver contexts in F01 and called for selected nested numeric/profile placements rather than multiplying every punctuation case across every profile.

#### Counter-explanation tested

Strong expected-value and diagnostic assertions protect outcomes for generated cases; they cannot expose a token boundary or receiver state never generated. A larger arbitrary case count or a full Cartesian product would not target the missing interactions and is unnecessary.

#### Violated contract or cost

Cross-parser parity coverage must independently establish common acceptance/rejection, supported restrictions and typed-value outcomes at the boundaries that distinguish the parsers. Missing these cases let accepted behavior diverge without a paired regression.

#### Root owner

The deterministic parity fixture generator and MON reader-local tests.

#### Suggested correction

Extend the existing fixture system with a bounded table across trivia position,
trivia class, receiving surface and receiving context. Supply independently
expected values and rejection reasons; retain explicit gap cases and exact
diagnostic spans rather than asking one parser to define the other's
expectations. Reuse signed-minimum, U64-maximum, narrow-integer rejection,
F16/F32 rounding, signed-zero and exact-decimal cases in selected nested typed
placements. Exercise all four numeric profiles where Int/Float receiving
changes matter; do not repeat punctuation-only cases under every profile.

#### Fix scope and preserved invariants

Resolved in commit `0f3439fda`. The bounded matrix has 156 cases and varies trivia before/after `=`, around commas, after openers, before closers and between constructor heads and payload openers. It covers none, space, tab, LF, CR, CRLF, comments, VT, FF, NEL, U+2028, U+2029 and NBSP across first/later/nested fields and nominal/choice payloads, plus one call-argument case. Six nested numeric/profile cases reuse existing boundaries. MON reader tests assert exact spans and keyword-opener behavior. Correct typed receivers are used where source and MON behavior should correspond; missing-context rejections remain separate. Reverting the constructor-head rule failed eight matrix and three reader tests; reverting horizontal-whitespace behavior failed 35 matrix and two reader tests. C03 accounting regressions were also retained and failed before their fix. No property-testing framework or source grammar expansion was added.

#### Required validation

Run the bounded parity cases and MON reader tests, then verify failure sensitivity for the corrected head and horizontal-whitespace rules. The implementer reports the scoped package/parity tests and clippy passed; `just validate-full` passed.

#### Linked findings

AUD-0009-F01, AUD-0009-F02 and AUD-0009-F03; AUD-0011-F01.

#### Triage record

2026-10-03 — **Accepted.** Add focused token-boundary and receiver coverage to the existing deterministic fixture owner. Preserve independent expected values, exact diagnostic reasons/spans and the existing profile-sensitive numeric expectations. Accept linked Correctness findings AUD-0009-F01–F03 and the reader cleanup's regression coverage in AUD-0011-F01.

2026-10-03 — **Fixed and closed.** Commit `0f3439fda` added the 156-case bounded matrix, one call-argument case, six nested numeric/profile cases, exact-span and keyword-opener reader tests, and valid receivers for the deferred-feature gaps. Failure-sensitivity checks showed that reverting the head rule failed eight matrix and three reader tests, and reverting horizontal whitespace failed 35 matrix and two reader tests. The C03 regressions failed before their production fix. Scoped package/parity/xtask tests and clippy passed; `just validate-full` passed.

### AUD-0010-F03: Dependency guard lacks optional and inactive-target fixtures

- State: `closed`
- Kind: `Tests`

#### Evidence

The dependency guard checks canonical package names across dependency kinds, but its tests did not cover an optional renamed forbidden dependency or a forbidden declaration in a target-specific section that is inactive for the current target. The implementation was not shown to mishandle either case; this is a test-coverage gap, not an established dependency-direction defect.

#### Counter-explanation tested

The current implementation may already handle optional and target-specific Cargo declarations correctly. That possibility is why this finding authorises fixtures only, not a production guard change. A positive `moth-mon` to `moth-lexical` dependency case remains relevant.

#### Violated contract or cost

The guard's canonical dependency-identity and dependency-kind behavior lacked regression evidence for optional renamed dependencies and target-specific declarations, leaving two supported manifest forms unprotected.

#### Root owner

Dependency-direction fixtures in `xtask/src/first_party_deps/tests.rs`.

#### Suggested correction

Add small fixtures for an optional renamed forbidden dependency and a forbidden dependency under an inactive target section. Assert canonical identity and dependency kind using the existing harness; keep the valid `moth-mon` to `moth-lexical` case.

#### Fix scope and preserved invariants

Resolved in commit `0f3439fda`: both optional-renamed and `cfg(any())` inactive-target forbidden-dependency fixtures were added. They passed against unchanged production code, confirming this was coverage hardening only. The positive dependency-direction case remains.

#### Required validation

Run the existing `first_party_deps` test owner and retain the positive case. Scoped xtask tests and clippy passed; `just validate-full` passed.

#### Linked findings

None. This is independent of the MON parser parity findings.

#### Triage record

2026-10-03 — **Accepted.** Add fixtures for both manifest forms, but do not change production dependency-guard logic without evidence of a defect. Preserve canonical-name and dependency-kind checks and the positive allowed edge.

2026-10-03 — **Fixed and closed.** Commit `0f3439fda` added optional-renamed and inactive-target forbidden-dependency fixtures. Both passed against unchanged guard code. Scoped xtask tests and clippy passed; `just validate-full` passed.

## Checked and clean

The parity harness uses real source and MON inputs with independently supplied expected values; numeric identity, float-bit and diagnostic-reason assertions remain stronger than acceptance-only checks. The update preserves the existing independent oracle and does not introduce a parser-derived expected value, framework, or deferred source-language implementation. `moth-mon` remains a standalone consumer of `moth-lexical`; the allowed dependency edge remains covered.

## Limitations

Coverage is partial. The review focused on the named parity cases, MON unit-test owners and dependency-direction fixtures, not every test under the compiler, codec or xtask trees. The external reviewer had neither Cargo nor rustc and did not execute reproductions; the implementer later ran them and the scoped tests. This report does not claim exhaustive parity across all grammar, numeric profiles, targets or consumers. The final `just validate-full` result is recorded in the baseline above.
