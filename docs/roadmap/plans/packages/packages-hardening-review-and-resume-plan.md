# Package hardening review and parallel-work resume plan

Review date: 12 September 2026

Repository: `nyejames/moth`

Working branch: `packages-and-builder-progress-plan`

## Start here

Keep the delivered Text optimisation and the dependency-guard coverage. Finish the small correction pass below, then activate `@core/math` as the next package. Start with its existing contracts, tests and registration owner. Prepare a bounded expansion proposal before registering new public functions.

The next batch is:

1. Establish fresh validation on the rebased branch and preserve its compiler repair.
2. Resolve the remaining scheduling contradiction and tighten the hardening tests.
3. Audit, harden and simplify the existing Math package.
4. Implement an approved scalar Math expansion through existing registration and JS lowering.
5. Prepare the Time package design checkpoint without starting an unreviewed time API expansion.

The earlier conversation's suggested Math functions are candidates, not an accepted public API. The programme still requires the v1 scope to be settled with the user. Existing-behaviour hardening and preparation of that decision can proceed first. [S3]

This file is a standalone review and execution handoff. It does not replace canonical package semantics, register a new audit campaign or certify the whole compiler. Read current worktree authorities before acting. Keep this handoff local unless the user asks to commit it. Durable implementation decisions belong in the existing package programme and the activated living package plan.

## 1. Reviewed snapshot and limits

| Item | Reviewed value |
|---|---|
| Package branch tip | `bac7413639ce589b6d59f1decedfd3bac77bed1b` |
| Diagnostics tip | `34bd000d56182a81dcf3e29effde7b0fe6d21e1f` |
| Merge base | The same diagnostics commit |
| Relationship | Package branch is nine commits ahead and zero behind |
| Package-only diff | 18 changed files |
| Compiler checkpoint | Phase 2 complete-path interning is recorded as delivered. Phase 3 token-store work is next |

These SHAs identify an actual reviewed snapshot. They are evidence, not instructions to reset the worktree or pin a future living plan. Refresh the comparison when starting work. GitHub does not show uncommitted local changes. [S1], [S2]

The review concentrated on the changed Text helper and fixtures, the dependency-guard owner and added tests, package scheduling and validation records, and the one compiler-source change. Math was inspected as the next work item. This was not an exhaustive audit of the existing Math, Time, Canvas or package infrastructure implementations. Ancillary documentation and locator edits in the comparison are not evidence of new package runtime capabilities.

### Validation performed in this review

The updated `__moth_text_length` body was copied from the reviewed source and executed in Node `v22.16.0` with an identity `__moth_string_value` stub. It passed **106,472 checks with zero mismatches** and exactly one materialisation call per invocation.

The checks included explicit contract vectors, every single UTF-16 code unit, all ordered sequences of two, three and four units from a 13-value surrogate-boundary set, and 10,000 deterministic generated strings. The representation-equivalence checks compared against the old `Array.from(text).length` behaviour. Lone-surrogate probes tested preservation of the old JS behaviour, not permission for invalid Unicode scalar values in Moth source.

This checks the isolated algorithm. It does not execute the real String runtime, compile Moth fixtures, check emitted-helper reachability or establish Rust correctness. Cargo and Just were unavailable in this review environment. No Moth integration suite, `just validate`, feature matrix or browser test was run here.

The optimisation commit reports Node speedups of 1.6x to 2.9x on its measured longer-string workloads. Those are the author's recorded measurements, not independently reproduced performance results from this review. Preserve that distinction. [S4], [S5]

## 2. Review verdict

**The Text algorithm is suitable to retain. The branch needs a small completion and test-quality pass before the next package phase.** No new functional defect was found in the scalar scan during source inspection and the isolated checks. Full branch acceptance remains dependent on fresh validation.

The hardening preserves the five-function public surface, resolves content once, counts a valid surrogate pair once and advances on every iteration. It keeps the existing helper name and reference-gated emission. The new fixtures add case-sensitive misses, combining characters, non-BMP boundaries, exact-match checks and empty-text cases. Exact output is stronger than the previous substring-only expectation. [S4], [S6], [S7], [S8]

The new guard tests also have distinct value. They exercise the mapping of require, dynamic import and re-export diagnostics into the dependency rule, and verify that an inventoried source is inspected and reported under its label rather than merely counted. Retain those contracts. The documented exclusion of host-driven loading is deliberate and should not become another scanner-expansion task. [S9], [S10]

### Finding R1: A blanket package pause remains below the corrected capsule

**Priority:** Medium, resolve before resuming implementation.

**Where:** `docs/roadmap/plans/packages/first-party-package-programme.md`, under `Compiler foundation checkpoint before package implementation`. Also review the opening purpose and scheduling wording in `native-result-slots-and-core-const-eval.md` for the same ambiguity.

The programme's capsule and roadmap now allow isolated existing-ABI package work during diagnostics Phases 2 and 3. The later checkpoint section still says to pause package expansion and to resume the programme only from `main` containing the compiler checkpoint. An agent following that section can park Math despite the explicit permission above it. [S3], [S11], [S12]

**Fix:** Make the prerequisite capability-specific throughout the programme. The checkpoint gates the Text v1 expansion and any slice needing native result slots or Core constant evaluation. It does not gate unrelated current-ABI Math hardening. Keep the deliberate synchronisation pause when the shared result representation actually changes.

Keep the roadmap as the scheduling owner. Update the existing wording rather than adding a competing parallel-work plan or another status section. Preserve the Wiring and native-result-slot checkpoint order.

**Acceptance:** No active instruction says to park all package work until native result slots while another says the current-ABI lane may proceed.

### Finding R2: The Text behaviour fixtures still pin generated implementation details

**Priority:** Medium, finish as part of the hardening correction pass.

**Where:**

- `tests/cases/core_text_edge_cases/expect.toml`
- `tests/cases/core_text_functions/expect.toml`
- The corresponding `input/src/@page.moth` files

Both manifests still require all five helper definitions. They also pin exact generated calls, including literal arguments and `__moth_read` structure. Both fixtures exercise all five functions, so those assertions do not establish that unused Text helpers are absent. They largely duplicate successful execution and couple ordinary behaviour to the lowering shape. This debt predates the patch, remains after it and is explicitly recorded in the Text plan. [S6], [S7], [S8]

**Fix:** Keep the new semantic cases and exact output. Remove call-spelling assertions that do not own a genuine backend contract. Give helper reachability one focused owner that exercises a strict subset of the package and checks both required presence and unused-helper absence. Search existing coverage first. Keep supported-target and unsupported-target expectations under an explicit owner.

Retain the distinction between exact predicates and Unicode/empty-pattern edges where it helps test ownership. Consolidate overlapping cases only after inspecting the manifest's contract and role fields. Avoid creating a separate fixture for every function.

Use a top-level output template instead of `io.line` when touching these cases, as the testing guide directs for non-console behaviour. Preserve the exact expected output and verify it through the harness. [S13]

The current external calls are runtime operations. Future Core constant evaluation can remove literal calls, so the later evaluator checkpoint needs separate runtime and folded coverage. A literal assigned to a binding is not a permanent runtime test barrier. Establish runtime dependence through an existing supported mechanism when that checkpoint lands, not through a new harness hook now.

**Acceptance:** Behaviour tests can survive a semantics-preserving lowering change, while a separate reachability contract still catches unwanted emission.

### Finding R3: The new guard mapping test depends on diagnostic prose

**Priority:** Low, fix in the same correction batch.

**Where:** `xtask/src/first_party_deps/tests.rs`, function `rejects_require_dynamic_import_and_re_export_module_loading`.

The test checks a total of three findings, then identifies each form through rendered-message fragments identifying require and re-export forms. The contract is the typed rule mapping, not those phrases. This creates avoidable sensitivity to the diagnostic work running in parallel. [S9]

**Fix:** Use a small table of independently scanned source snippets. For each, assert one finding, `FirstPartyDepsRule::UnapprovedModuleImport` and the expected source label. This catches a missing mapping arm without matching explanatory prose. Keep any separate requirement that a rejected module's identity appears in a user-facing message under its existing test owner.

Keep the inventory-label regression. It protects a different path and is not redundant with physical-file scanning. Avoid a new test framework or changes to the scanner's diagnostic representation.

### Finding R4: The longer Text scan needs readable source formatting

**Priority:** Low, polish while touching the helper rather than creating another phase.

**Where:** `src/backends/js/package_bindings/core/text.rs`, `CORE_TEXT_JS_HELPERS` entry for `__moth_text_length`.

A one-expression helper has become a multi-statement loop stored on one very long Rust string line. That makes the surrogate bounds and progress invariant unnecessarily hard to review. [S4], [S21]

**Fix:** Use the existing raw multiline string style for this non-trivial JS body, with a short explanation that a valid UTF-16 pair contributes one scalar. Keep the algorithm, helper inventory and emitter unchanged. There is no need for a new source file, scalar utility module, helper graph or further micro-optimisation.

### Integration observation: Preserve and upstream the compiler repair

The package-only diff contains a real compiler fix outside the package owners. In the diagnostics baseline, the `timers` arm of `compile_check_only_job` passes `prepared` before `path_base_len`. The callee expects `path_base_len: usize` before `prepared: PreparedModule`. The package branch corrects that order. [S15]

This is a justified checkpoint repair, not a Text feature or permission for wider compiler cleanup. Keep it isolated and report it to the diagnostics owner so the next published checkpoint includes it. Avoid reapplying it if a later checkpoint already has the correction.

It also explains why the recorded default-library checks are insufficient evidence for the imported checkpoint. The bad call is feature-gated. Exercise both default and timer-enabled builds, then the standard feature matrix.

### Validation observation: Refresh the red-gate record rather than trusting either old claim

The programme records 102 lint findings, including 96 `result_large_err` findings and six path-work lints. Its accepted interim procedure permits existing-ABI work to proceed while reporting a failed full gate and separate `just validate-common` evidence. The same document explicitly says this is not a closed full gate. [S3]

The current diagnostics capsule records selected compile and subsystem checks for Phase 2. Its full green `just validate` record belongs to the older Phase 1 checkpoint. Neither record certifies this rebased package tip. [S2]

Treat the exact lint count and causes as recorded observations to recheck. Keep implementation status, current validation status and capability prerequisites separate. Do not call the branch green because the helper probe passes or because Phase 1 once passed.

## 3. Required reading and ownership

Start with current `AGENTS.md`. Resolve the supplied code-review guide's older file locations through that file rather than restoring old documentation paths. [S14]

For this batch, read the style guide, testing guide and validation guide, the programme and active package plan, both progress matrices, and the relevant routes in:

- `docs/compiler-design-overview.md`, especially binding-backed symbols, call targets and target-contract validation
- `docs/build-system-design.md`, especially package classification, builder-selected availability and external JS emission
- `docs/src/developer-docs/language/overview.mtf` and its canonical String, Char, Float, call, error and constant references

Use `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf` for orientation, not for deciding semantic edge cases. Read `docs/compiler-data-layout-design.md` for incoming representation checkpoints. The reviewed state already uses `PathId` and has deleted `InternedPath`. Future package work consumes that representation directly. [S2], [S14]

The queued numeric work adds the fixed-scale `Number`/`NumberN` family and a deferred Byte runtime path. It preserves existing Int and Float behaviour. A current Float-only Math slice neither implements that plan nor inherits Number's half-even rounding rule automatically. [S16]

## 4. Resume phases

### P0. Establish the rebased validation baseline

**Goal:** Know what this worktree can actually build and which failures belong to the imported compiler checkpoint.

- [ ] Inspect the branch, dirty state and current commits. Preserve unrelated user work.
- [ ] Compare against the exact diagnostics checkpoint already incorporated. Record the current SHAs in local notes, not in the new living plan's status capsule.
- [ ] Confirm the `compile_check_only_job` argument-order repair remains present and arrange its upstream handoff.
- [ ] Run default and timer-enabled library checks, the two Text cases, dependency-guard tests and the integration audit.
- [ ] Attempt `just validate`. If the documented external lint blocker remains, run `just validate-common` separately and classify every failure.
- [ ] Run `just test-feature-matrix` for this imported representation checkpoint. A feature-coverage inventory does not execute the feature lanes.

Useful commands, from this worktree:

```bash
git status --short
git branch --show-current
git rev-parse HEAD
git log -1 --oneline origin/diagnostic-data-layout-changes
git merge-base HEAD origin/diagnostic-data-layout-changes
git diff --stat origin/diagnostic-data-layout-changes...HEAD

cargo fmt --all -- --check
cargo check -p moth --lib
cargo check -p moth --lib --features timers
cargo test -p xtask --quiet first_party_deps -- --format terse
cargo run --quiet -- tests --case core_text_edge_cases
cargo run --quiet -- tests --case core_text_functions
cargo run --quiet -- tests --audit
just validate
```

When the full gate stops on the documented external lint lane, run these as separate commands rather than hiding a failure with a shell fallback:

```bash
just validate-common
just test-feature-matrix
```

Refresh remote tracking refs before using them when needed. Inspect incoming changes before integrating another checkpoint. These commands do not authorise a reset, force push, automatic stash or mid-phase merge.

**Exit:** Current command outcomes and blockers are recorded. New compile errors, runtime failures or package-introduced lints are resolved before package implementation proceeds. The recorded allowance covers the inherited lint blocker, not arbitrary failing tests or a broken build. Design-only work may continue while an external implementation blocker is escalated.

### P1. Complete the hardening correction pass

**Goal:** Leave a clean starting point without extending the Text API.

- [ ] Resolve R1's contradictory scheduling wording. Preserve the existing capability gates and checkpoint order.
- [ ] Resolve R2's test ownership and generated-call coupling. Retain all meaningful semantic coverage.
- [ ] Resolve R3's prose-sensitive guard test.
- [ ] Apply R4's readability improvement without changing the scan.
- [ ] Update only affected coverage, status and package-plan wording. Describe unresolved validation honestly.
- [ ] Run focused checks, the applicable full gate and the slice review below.

Keep the scheduling/documentation change independently reviewable from the test/helper correction where practical. Documentation-only work uses the docs release-build gate. Mixed code and documentation uses the code-bearing gate.

**Production boundary:** `src/backends/js/package_bindings/core/text.rs` only. The guard's production scanner and the compiler repair need no additional implementation here.

**Exit:** The hardening findings are fixed or have a specific accepted deferral. The five existing Text signatures remain unchanged. Wrapper inlining, registration redesign, new Text errors, new APIs and constant evaluation stay in the later Text plan.

### P2. Activate Math and harden its existing surface

**Goal:** Produce useful implementation work immediately without relying on new compiler capabilities.

Create `docs/roadmap/plans/packages/core-math.md` when starting this phase. Use the programme's existing ten-part living-plan structure. Record Math's earlier activation and the reason in the umbrella tracker. Keep the main roadmap linked to the umbrella rather than adding each package as a separate roadmap entry. [S3]

#### A. Inventory before adding functions

Trace the current 18 functions and three constants through canonical docs, registration, the shared Float result boundary and their actual test owners. Classify each as covered, missing coverage, unspecified semantics or a confirmed defect. Keep the implementation inventory compact rather than copying the public reference into the plan. [S17], [S18]

The existing contract to preserve includes:

- Core origin, ExternalBinding backing and explicit dependency clauses
- positional-only Float inputs with shared access and fresh Float results
- infallible package signatures with numeric failure handled by the existing enclosing failure mode
- no NaN or infinity observable as a valid Moth Float
- compile-time scalar constants and runtime JS function lowerings
- unsupported reachable Wasm calls rejected through existing target validation

Trace the finite-result guard rather than adding a Math-specific duplicate. A builtin `Error!` enclosing function can recover through the existing numeric error lane. An infallible or custom-error enclosing function and root runtime work follow the existing trap policy. Adding `Error!` to each Math signature would change the contract. [S18]

#### B. Strengthen current behaviour coverage

Use existing integration owners where possible, including `core_math_float_boundary_validation`. Inspect current manifest metadata before creating or consolidating cases.

Cover ordinary composition, direct/aliased/namespace calls, arity and type rejection, constants, supported-target execution and unsupported reachable target use. Concentrate on missing contract families rather than repeating package plumbing for every function.

Extend meaningful finite and invalid-domain cases across the existing roots, logs, powers and exponentials. Preserve the current error identity and recovery policy. Test the failure boundary where the failing result is produced, before a later operation could conceal it.

Add one coherent runtime scenario combining current functions with ordinary Moth control flow and output. Use exact output for small deterministic results. For approximate transcendental values, prefer a justified error bound with a deterministic pass/fail result over a long exact decimal spelling. The canonical numeric contract controls the assertion, not whichever engine supplied a sample answer.

Audit these semantic questions explicitly: radians, rounding ties, signed zero, `atan2` quadrants, underflow, domain endpoints, reversed clamp bounds and precision expectations across future backends. Where the canonical documents are silent, record the observed implementation separately from the proposed rule. Avoid silently making current host behaviour authoritative through a new golden test.

#### C. Simplify registration without adding infrastructure

The existing `math.rs` builds owned lowering strings and parameter vectors, then clones them while iterating. Its local parameter constructor also receives a name that it ignores. Remove avoidable copies and misleading parameter-name plumbing. Construct final registry-owned values once. [S17]

Keep one package-local table and straightforward registration flow. Use a small named descriptor only if it improves readability. Preserve registration order and the public metadata. There is no need for a new cross-Core registration framework, macro DSL, unary/binary dispatch hierarchy or Math runtime module. Trivial scalar functions already use `ExternalJsLowering::InlineExpression`.

**Expected touch map:**

| Owner | Permitted work |
|---|---|
| `src/builder_surface/core_packages/math.rs` | Existing registration cleanup and later approved scalar additions |
| Existing Math cases under `tests/cases/` | Semantic and target coverage |
| `tests/cases/manifest.toml` | Narrow ownership/tag changes for those cases |
| Existing external-package/backend test owners | Only hidden metadata or boundary invariants not observable end to end |
| `docs/src/docs/packages/core/math/` | Approved contract clarification and teaching updates |
| `docs/roadmap/plans/packages/core-math.md` | Living implementation plan and open decisions |
| Umbrella tracker and package progress matrix | Activation and actual coverage/status changes |

**Exit:** Existing Math behaviour has stronger coverage, registration is smaller or clearer, and remaining API/semantic choices are ready for review. This phase does not require Core constant evaluation or Wasm implementation.

### P3. Complete an approved scalar Math expansion

**Entry condition:** The user has accepted the bounded function list and unresolved semantic choices from P2. Until then, prepare the proposal and continue only existing-behaviour work or the Time design task.

Use two proposed groups rather than automatically adding everything previously suggested:

| Proposed group | Candidates | Required decisions |
|---|---|---|
| First expansion | `asin`, `acos`, `atan`, `cbrt`, two-argument `hypot`, `expm1`, `log1p` | Domains, units, finite-result failures, signed-zero behaviour and numerical test bounds |
| Optional later expansion | `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh` | Common-use justification, endpoint behaviour and overflow/finite-result tests |

The constant candidates `SQRT_2`, `LN_2` and `LN_10` also need deliberate acceptance. Keep convenience conversions, statistics, vector/matrix APIs, integer math, generic numeric APIs and specialised algorithms outside this batch.

Before implementing an accepted group, publish its accepted contract in the canonical Math reference. Add a short Basic explanation only where it helps teach the new use case. A candidate in this handoff is not sufficient authority to change public semantics.

Implementation rules:

- [ ] Reuse the current scalar external ABI, return metadata and shared finite-Float boundary.
- [ ] Use direct inline JS lowerings where they preserve the full accepted contract. Keep argument evaluation and placeholder use consistent with the existing lowering contract.
- [ ] Keep `hypot` fixed-arity for this proposal. A host's variadic form is not permission to add variadic external signatures.
- [ ] Keep `wasm: None` while no lowering exists. Use existing unsupported-target diagnostics.
- [ ] Exercise every new function end to end by group closeout, including meaningful failure/recovery cases for domain-sensitive functions.
- [ ] Keep no third-party runtime dependencies and no copied/vendor math implementation.
- [ ] Update the living plan, package docs and coverage row in the same accepted slice.

**Constant-evaluation boundary:** Defer Math folding until the shared evaluator is delivered and Math's precision and numeric-failure behaviour have an accepted compile-time contract. The presence of an eligible scalar signature alone does not settle that contract. Add no Math-local interpreter, callback registry, package-name dispatch, speculative evaluator enum or permanent placeholder metadata. [S12]

**Exit:** The approved group is implemented and reviewed with no shared compiler representation changes. Stop after the accepted scope instead of treating the host Math object as an API checklist.

### P4. Prepare the Time design checkpoint

**Goal:** Leave the next package ready for a deliberate decision, not a second speculative implementation branch.

Activate `core-time.md` only when starting this task. Audit the current Duration, TimeMark and Timestamp contracts against their registration, runtime helper and tests. Use the same compact living-plan structure.

Prioritise validity and portability before convenience arithmetic. The current ISO helper calls `Date.parse` directly and constructs the result carrier itself. Review its accepted input grammar, use of the canonical String content boundary and reuse of the current error/carrier owner. Treat these as audit questions until verified against current contracts and runtime code. [S19]

The review should settle or explicitly leave for user decision:

- accepted timestamp grammar, timezone requirements, fractional precision and invalid calendar values
- representable timestamp range, duration range and rounding policy
- preservation of finite-value invariants through opaque returns and arithmetic
- formatting failure behaviour and host exception containment
- monotonic mark origin and comparison restrictions
- a small useful arithmetic surface over the existing value types

Duration addition/subtraction/scaling, timestamp offsets and timestamp differences are candidates, not approved function names or signatures. Keep calendar types, time zones, timers, async, animation scheduling and new opaque-handle infrastructure out of implementation scope.

**Deliverable:** A decision-ready Time plan with concrete source evidence, contract gaps, a small proposed v1 target and a verified existing-ABI touch map. Implement only separately approved corrections or additions.

### P5. Close out this batch

Run the required validation and slice reviews. Compress completed checklist detail in living plans, leaving current behaviour, rationale and real blockers. Keep detailed execution history in Git.

Report one of these outcomes accurately for each slice:

| Outcome | Meaning |
|---|---|
| Accepted | Relevant reviews and the full required gate passed |
| Implemented, full gate externally blocked | Focused checks and required interim lanes passed, but the recorded external blocker still prevents full acceptance |
| Design ready | Proposal and evidence are complete, public API approval is pending |
| Blocked | A required capability or unresolved failure prevents the next implementation step |

The final handoff names the current package commit, incorporated diagnostics checkpoint, changed files, tests actually run, unresolved findings and one next action. It also says whether new API scope was approved or remains a proposal.

## 5. Safe parallel-work rules

### One writer and a narrow change surface

Use one writer per worktree. Read-only subagents can inventory contracts, inspect tests or review a bounded diff. Additional implementation writers need separate worktrees and an explicit integration owner. Avoid concurrent commits, staging or formatting in the same worktree.

Keep normal writes within the active package owners, its tests and its documentation. Route defects in paths, tokens, source ownership, diagnostic payloads, external ABI representation, AST/HIR, target-validation architecture and graph construction to their existing owners. The already identified argument-order repair is a narrow exception to retain and upstream, not an expanding cleanup mandate.

Reuse the current integration harness, JS inventory, runtime-module registry and validation recipes. A package task should not create another package system or test framework.

### Integrate checkpoints between slices

Finish or explicitly block the current bounded slice before merging/rebasing another diagnostics checkpoint. Record pre-sync and post-sync commits locally, inspect the incoming shared API changes, resolve the migration and re-run validation before further package implementation.

Adopt new shared representations directly. Keep no compatibility wrapper around a superseded PathId, token, diagnostic, HIR or return-slot shape. Recheck the final diff against the incorporated checkpoint so diagnostic work is not mistaken for package-owned changes.

The roadmap's sequence remains data-layout Phase 3, Wiring foundations, native result slots/Core const evaluation, then data-layout Phase 4. Shared-capability consumers refresh at those boundaries. In particular, the Text String materialisation owner must be rechecked when Wiring removes the old reactive behaviour. Preserve current behaviour until that owner actually changes. [S11], [S12]

After an incoming representation or feature-sensitive change, use the standard feature matrix as well as the normal gate. A package-only documentation edit does not need a fresh compiler-wide feature campaign unless the baseline itself changed.

### Preserve validation honesty

`just validate` currently runs `ci-clippy-native`, then `validate-common`. The common recipe includes feature-lane coverage, source and dependency audits, workspace tests, integration tests, docs checking, benchmark checks, scaling and timer erasure. It does not replace Clippy or execute the full feature matrix. [S10]

Keep the allowed inherited lint failure visible. Re-run it after each relevant sync instead of copying an old count. New package failures are not covered by that allowance. Fix inherited shared-source failures in coordination with their owner rather than boxing diagnostics, suppressing lints or mass-formatting unrelated in-flight work.

Use `just bench-check` for non-recording compiler performance evidence when appropriate. The Text scan optimisation affects generated-program execution, so compiler timing alone cannot validate its runtime speedup. Keep raw probes local and separate reported measurements from rerun results.

Run validation without another agent editing tracked files. The benchmark part of the gate checks worktree changes. Documentation-only work uses `cargo run --quiet -- build docs --release` or a verified up-to-date compiler equivalent. Review generated changes and keep unrelated output churn out of the package patch. [S10], [S22]

## 6. Slice review and stop conditions

At every non-trivial slice boundary, review correctness and contracts first, then duplication, ownership, readability and tests. Use the supplied `moth-code-review-guide.md` with current AGENTS routing. Resolve findings, re-run affected checks and repeat until the slice is ready or has an explicit blocker.

Before closing a slice, verify:

- [ ] Every public behaviour change has an approved canonical contract and a primary integration owner.
- [ ] Shared access, fresh returns, numeric failures and target rejection are preserved.
- [ ] Unused helpers/assets remain absent where reachability is contractual.
- [ ] No duplicate registry, scanner, evaluator, fallback path or compatibility adapter was added.
- [ ] Test assertions protect behaviour or a genuine hidden invariant rather than current source spelling.
- [ ] The package-only diff stays within its declared owners, including after checkpoint integration.
- [ ] Validation claims name actual commands, configuration and outcomes.
- [ ] Documentation and progress describe implemented or accepted work without promoting proposals to support claims.

Stop the implementation portion when it needs a new public value representation, generic external signatures, changed numeric failure policy, unaccepted time semantics, new language syntax, shared compiler representation work or an unapproved API. Record the precise missing capability and continue with a bounded design deliverable where useful.

## 7. Queue after this batch

Math is the next implementation package. Time is the next design checkpoint. Keep the remainder as a queue rather than creating placeholder plan files. Random's current bound policy is already documented and should not be reopened as an implementation bug. [S20]

| Candidate | Appropriate next action | Boundary |
|---|---|---|
| `@core/random` | Audit bounds, equal bounds, full-range behaviour and deterministic test strategy | Existing inclusive integer bounds and reversed-bound swapping are documented behaviour. Seeded reproducibility or a change to that policy needs approval |
| `@web/canvas` | Audit current drawing workflows, wrappers and coverage before expansion | Preserve the current synchronous host-binding boundary. Avoid async/image-loading or Wiring workarounds |
| `@core/json` | Investigate whether a useful v1 has a correct representable value model | Design only until recursive values, collection access and number/error rules are settled. Avoid opaque-handle APIs created solely to evade an unavailable final representation |
| Collections and IO | Leave implementation with their accepted owning plans and design checkpoints | Preserve sorting/mixed-backend prerequisites, retained-edge semantics and the unresolved IO/prelude scope |

## 8. Evidence and source map

References below point to the reviewed package commit unless stated otherwise. Use them to reproduce this review, then re-read the current worktree before implementation.

[S1]: https://github.com/nyejames/moth/compare/34bd000d56182a81dcf3e29effde7b0fe6d21e1f...bac7413639ce589b6d59f1decedfd3bac77bed1b
[S2]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/docs/roadmap/plans/compiler-source-token-and-diagnostic-data-layout-plan.md
[S3]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/docs/roadmap/plans/packages/first-party-package-programme.md
[S4]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/src/backends/js/package_bindings/core/text.rs
[S5]: https://github.com/nyejames/moth/commit/ce23d39b4f50ed20e74595c1bc05eb7b88055885
[S6]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/tests/cases/core_text_edge_cases/expect.toml
[S7]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/tests/cases/core_text_functions/expect.toml
[S8]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/docs/roadmap/plans/packages/core-text.md
[S9]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/xtask/src/first_party_deps/tests.rs
[S10]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/justfile
[S11]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/docs/roadmap/roadmap.md
[S12]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/docs/roadmap/plans/native-result-slots-and-core-const-eval.md
[S13]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/docs/src/developer-docs/style-guide/testing.mtf
[S14]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/AGENTS.md
[S15]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/src/build_system/create_project_modules/compilation/canonical.rs
[S16]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/docs/roadmap/plans/number_type_numeric_plan.md
[S17]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/src/builder_surface/core_packages/math.rs
[S18]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/docs/src/docs/packages/core/math/math.mtf
[S19]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/src/backends/js/package_bindings/core/time.rs

| Reference | Evidence |
|---|---|
| [S1] | Exact package-only comparison and incorporated checkpoint |
| [S2] | Phase 2 status, Phase 3 next action and historical versus current validation |
| [S3] | Current parallel-work permission, remaining global-pause wording, activation rules and interim validation status |
| [S4], [S5] | Shipped Text scan and the author's equivalence/performance record |
| [S6], [S7], [S8] | Exact-output hardening, remaining generated-code coupling and Text scope boundaries |
| [S9] | New guard mapping and inventory-inspection tests |
| [S10] | Executable validation recipes and their feature coverage limits |
| [S11], [S12] | Shared checkpoint order and compiler-owned constant-evaluation contract |
| [S13], [S14] | Test policy, current routing, slice reviews and authority rules |
| [S15] | Check-only call repair and callee argument order |
| [S16] | Number/NumberN scope and preservation of existing Int/Float behaviour |
| [S17], [S18] | Current Math registration and canonical public contract |
| [S19] | Current Time ISO helper, a starting point for the next audit |

[S20]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/docs/src/docs/packages/core/random/random.mtf
[S21]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/docs/src/developer-docs/style-guide/style-guide.mtf
[S22]: https://github.com/nyejames/moth/blob/bac7413639ce589b6d59f1decedfd3bac77bed1b/docs/src/developer-docs/style-guide/validation.mtf

Additional authorities: [S20] preserves Random's current bound policy. [S21] owns source readability and abstraction standards. [S22] owns gate selection, failure reporting and the dependency guard's lexical/manual-review boundary.

The diagnostics-side version of the repaired call is available at:

https://github.com/nyejames/moth/blob/34bd000d56182a81dcf3e29effde7b0fe6d21e1f/src/build_system/create_project_modules/compilation/canonical.rs#L802-L816
