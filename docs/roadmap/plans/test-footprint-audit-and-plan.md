# Moth test footprint audit and conservative improvement plan

Status: active, Phase 4 HIR and diagnostics accepted, next borrow and numeric oracles
Repository: `nyejames/moth`, branch `expression-refactor`, commit `1343c64ecbc5433b546c3ad31cf5c175cf51f184`
Policy companion: `docs/src/developer-docs/style-guide/testing.mtf` (revised replacement supplied separately)

## Objective and decision rule

Reduce test maintenance and repeated setup while improving the compiler's ability to catch real, interacting Moth language regressions. Rich integration scenarios and precise unit tests serve different purposes. A longer integration fixture is not evidence that a local invariant is covered. There is no LOC or test-count target.

A test is admitted when it protects a credible distinct regression, has an assertion that would fail for that regression and adds protection not already provided by an existing test that can be extended. A test is removed only after identifying its exact surviving contract and failure paths. Prefer extending, merging and deleting over adding a new harness or general fixture abstraction.

This document separates **observed evidence**, **proposed safe edits** and **work requiring further comparison**. No tests have been changed or executed as part of this review. Source and suite counts may differ from the attached local LOC baseline because they were measured on the named remote commit.

## Baseline and scope

The supplied concentration brief estimates 328,410 nonblank, noncomment Rust unit/test-helper LOC, 78,981 LOC in the Moth integration corpus and 44,143 Moth source LOC within that corpus. These are size estimates, not test counts or measures of value. The Rust unit footprint is roughly 4.2 times the entire integration corpus. The reported first four Rust concentration areas amount to roughly 165k LOC: AST, Stage 0, borrow/Boracle/numeric analyses and HIR.

On `expression-refactor` at the commit above, direct inspection of the repository tree and integration manifest shows:

| Observed inventory | Count |
|---|---:|
| Canonical `tests/cases/` manifest entries and `expect.toml` files | 2,390 |
| `.moth` files within canonical case trees, including config and helpers | 5,438 |
| Manifest roles: primary / boundary / backend / adversarial / smoke | 1,626 / 595 / 144 / 22 / 3 |
| Cases tagged `diagnostics` | 1,199 |
| Cases tagged `runtime` | 278 |
| Cases tagged `bug-hunt` | 18 |
| Median total `.moth` file bytes per case, including config | 235 bytes |
| Cases with under 1 KB total `.moth` bytes | 2,203 of 2,390 |
| Exact duplicate non-config `.moth` blob groups across case directories | 102 groups, about 69 KB repeated bytes |

Tags overlap and the median is **not** an estimate of distinct semantics tested. Many negative diagnostic cases are intentionally very small. A 1 KB threshold is a discovery heuristic, not a quality limit. The identical source files are frequently intentionally repeated for debug/release or profile-specific expectations. Preserve their separate runs unless a measured simpler alternative is proven.

The checked-in `docs/roadmap/evidence/test_honesty_inventory.json` dates from 2026-08-21. It records earlier test-assertion hardening but is **not** a current redundancy inventory. Generate fresh inventories before pruning.

## Verified findings and first candidate decisions

### 1. Duplicate runtime assertions inside existing positive integration cases

**Confirmed overlap.** Both of these expectation files list nearly the same fragments in `rendered_output_contains` and `rendered_output_contains_in_order`:

- `tests/cases/value_if_block_single_predicate/expect.toml`
- `tests/cases/value_if_multi_bind_inferred_single_predicate/expect.toml`

An in-order fragment assertion already requires each listed fragment to occur. The duplicated `contains` entry adds no further presence protection. Keep any unique fixture identification or behavior assertion that is not present elsewhere.

**Decision: merge expectation assertions within each existing case.** Keep the ordered sequence and unique assertions. Do not merge the two cases merely because both use `if`: the single-result and multi-result receiving boundaries are distinct. Confirm before/after expectation parsing and runtime output are unchanged. This is an assertion-only first patch with low risk.

### 2. An equivalent repeated loop within one basic integration scenario

**Confirmed overlap.** `tests/cases/control_flow/input/src/@page.moth` runs two independent `loop 1 to 5` sums, `sum` and `total`, with the same body. The output expects both to be `10`. It also contains isolated simple/nested `if` and match examples. The second loop currently adds little distinct detection.

**Decision: extend instead of add.** Replace the repeated loop with a small linked computation in the same fixture. For example, make a helper return a value based on nested conditions and accumulate it inside a loop, including both branch outcomes and a continuation or early-return path. Keep the original branch and match values observable. Assert the combined final result, not just a fixed output marker. Compare related `loop_*` and `value_if_*` cases first to avoid duplicating their specific contracts.

Do this as a separate change from assertion-only pruning because it changes behavior and expected output.

### 3. A rich adversarial scenario has important unobserved paths

**Confirmed gap in the authored scenario.** `tests/cases/adversarial_choice_payload_control_flow/input/src/@page.moth` defines `handle_result`, including a nested `if` for successful values above 10, but never calls it. The loop runs through `Pending`, `Success` and `Failure` values yet overwrites `summary` and asserts only its final `fail` state. The only `read_task` invocation succeeds, so its recovery branch does not run. The expectation also requires generated-JS snippets such as `.tag === 0` and `while (`.

**Decision: extend and strengthen in place.** Exercise `handle_result` with successful small/large values, failure and pending values. Track or output independent counts and an ordered trace for every loop variant. Call `read_task` in both success and error-recovery paths. Update `rendered_output_exact` or equally discriminating ordered/exactly-once checks. This should reveal changes to earlier branches even when the last iteration still ends in `Failure`.

**Conditional review:** emitted JavaScript substrings are brittle unless exact tag representation or a particular lowering form is an intentional backend contract. Check the owning backend tests before removing these artifact assertions. If they only restate semantic effects, replace them with stronger runtime assertions. Preserve a narrow backend structural assertion if it guards a distinct documented invariant.

### 4. Multi-file adversarial case is a minimal successful chain

**Confirmed weakness.** `adversarial_multi_file_helper_chain` compiles `@page.moth -> helper/compose.moth -> helper/prefix.moth` and checks only that a single rendered output contains `A-hello-B`. It is a useful multi-file import smoke case but does not stress repeated calls, alternate paths, typed values or multiple helper results.

**Decision: extend if distinct from existing multi-file cases.** Add two or three inputs that take different nested paths through the same modules, preferably a conditional helper and an optional or fallible result. Assert an exact sequence of outputs. Preserve the original three-file dependency and its boundary semantics. Compare `multi_file_module`, `complex_module_structure` and the import/alias cases before choosing the specific interactions. Avoid turning it into a second module discovery test suite.

### 5. Existing integration cases that deserve protection, not automatic pruning

- `implicit_failure_catch_evaluation_order`: checks eleven execution traces, error codes and short-circuit order with exact runtime output. It demonstrates high-value interaction assertions.
- `compound_assignment_atomicity_runtime`: checks failure codes, unchanged targets and exactly-once right-hand side evaluation across scalar and field updates.
- `hashmap_js_runtime_contract`: checks map insertion, replacement, deletion and scalar key boundaries including fixed-width keys with exact runtime output.
- `value_if_multi_bind_inferred_single_predicate`: covers different success-slot shapes, nested branches, early returns and typed optionals. Consolidate redundant expectation declarations, not its distinct receiving behavior.
- `static_if_branch_scope_preserved`: already exercises nested statically selected scope. Do not introduce another test merely because it lacks a new "nested if" example elsewhere.

Keep these unless a same-contract comparison finds proven redundancy. Consider using their assertion quality as a model when enriching other cases.

### 6. Identical source files can protect different build profiles

**Confirmed legitimate duplication.** The `.moth` source blob is identical across both `fixed_width_integer_checked_boundaries` and its `_release` case (10,049 bytes each), and across `fixed_width_binary_float_checked_boundaries` and `_release` (9,508 bytes each). Their expectations differ in `flags = ["release"]`. Other exact duplicate source groups follow the same pattern.

**Decision: keep both executions.** Release-specific arithmetic behavior and target diagnostics are distinct boundaries. Do not merge or remove profile siblings as redundant. A single fixture with multiple executions of the same backend/profile would require runner changes and review of report identity, selection and case ownership. Do not build that infrastructure solely to save source bytes. Revisit only if profiling finds a material repeated maintenance burden across many pairs.

## Unit-test concentration: inspect, not delete by size

The following are **specific review clusters**, not confirmed removable tests. Counts are Rust source lines and `#[test]` functions observed on the named branch, not code LOC or final before/after counts.

| File or owner | Observed shape | Initial disposition and investigation |
|---|---|---|
| `src/compiler_frontend/tests/frontend_pipeline_tests.rs` | ~5,024 lines, 74 tests | **Keep stage-handoff owners.** `compiles_single_file_program_through_borrow_check` asserts exact synthetic-start provenance and borrow facts. `compiles_multi_file_dependency_program_through_borrow_check` asserts helper origin and summary existence. `implicit_failure_hir_two_numeric_operations_sequence_success_and_share_one_handler` checks the CFG join and shared handler structure. Compare the many `implicit_failure_*` functions for repeated *same* edge/failure facts, then table-drive only equivalent rows. |
| `src/compiler_frontend/hir/tests/hir_validation_tests.rs` | ~4,882 lines, 84 tests | **Keep malformed-IR invariants.** Dangling dense child IDs, CFG edges and invalid numeric domains cannot be replaced by accepted source programs. Already uses a table for some invalid numeric-domain cases. Inspect opportunities to reuse existing local HIR builders without losing individual invalid states. |
| `src/compiler_frontend/hir/tests/hir_expression_lowering_tests.rs` | ~4,895 lines, 63 tests | **Keep stage-specific lowering facts.** Examine near-duplicate AST/IR fixture setup alongside frontend pipeline tests. Merge only exact repeated assertion semantics. Inspect retained template handoff and result-slot cases before making any conversion. |
| `src/compiler_frontend/ast/generic_functions/tests/frozen_body_tests.rs` | ~4,528 lines, 46 tests | **Keep resource/identity seam.** Donor-retained syntax, path identities and generated sidecar remapping are invisible to output-only fixtures. Check repeated resource materialisation fixtures for local consolidation, not deletion of distinct origin or lifetime transitions. |
| `src/compiler_frontend/ast/templates/tir/tests/slot_composition_tests.rs` | ~3,173 lines, 54 tests | **Merge repetitive construction where useful.** The default, named, positional and mixed slot schema cases are semantically distinct but repeat store/builder setup. Reuse an existing narrow local fixture builder or table if it actually shortens and clarifies the cases. Keep schema, replay and phase distinctions. |
| `src/compiler_frontend/ast/templates/tests/create_template_node/parser_tir_tests.rs` | ~2,671 lines, 46 tests | **Keep parser/TIR distinction.** Compare repeated simple node-shape assertions to richer composed cases, but retain precise parser phase/origin assertions. Check for obsolete syntax after template simplification before removing anything. |
| `src/backends/js/tests/control_flow.rs` | ~2,593 lines, 18 tests | **Keep backend strategy contracts.** `emits_structured_if_without_dispatcher` checks structured JS output while `falls_back_to_dispatcher_for_cfg_cycle` checks cyclic fallback. Equivalent runtime output cannot distinguish them. Review repeated hand-written HIR builders and the structured/dispatcher merge pair for local consolidation using existing `support` helpers. No general graph DSL. |
| `src/build_system/tests/stage0_filesystem_identity_tests.rs` | ~3,516 lines, 75 tests | **Keep distinct filesystem contracts.** UTF-8 rejection, canonical paths, boundary ownership and platform-specific IO errors are not one repeated test. Consider table-driving equivalent invalid path spellings only when the same error owner and exact assertions apply. |
| `src/compiler_frontend/analysis/borrow_checker/boracle/oracle/tests/mod.rs` | ~5,995 lines, 111 tests | **Keep independent oracle proof coverage.** Capability generations, conflicting aliases, malformed problems and bounded exhaustive behavior protect a separate verification mechanism. Review helper duplication only after identifying which oracle invariant each test owns. Never replace this suite with a Moth runtime example. |
| `src/compiler_frontend/compiler_messages/tests/diagnostic_model_tests.rs` | ~4,489 lines, 88 tests | **Keep diagnostic schema/rendering.** Typed reason identity, spans, remapping and rendering are distinct from one user-facing diagnostic result. Consolidate only genuinely equal setup/expected-reason rows and preserve exact payload assertions. |

**Inspection priority:** start with frontend pipeline, template/AST positive tests and JS/backend tests where externally visible behavior already has a strong integration owner. Inspect HIR validation, numeric/borrow analysis and Boracle primarily for duplication of setup and stale implementation-specific tests. Their high LOC alone supplies no deletion rationale.

## New end-to-end scenarios: small in number, rich in interactions

Add a new scenario only when a search of current manifest contracts and source proves there is no suitable existing scenario to extend. Target a complete miniature application, not a sampler of unrelated syntax forms. The attached Ember Relay program (~1,165 source lines) is an exceptional upper-bound example and is not a recommended normal fixture size. Each new case should have one coherent observable outcome, several paths and a small set of independently checked invariants.

### Candidate A: fulfilment ledger (highest priority if uncovered)

A deterministic batch-fulfilment program spanning a small number of functions and, only if useful, a second source file. It has stock records, order choices and an event ledger. It applies requests, partial or rejected fulfilments and returns. Decisions nest inside a loop; helper calls mutate collections and use explicit copies/borrows; a fallible operation is handled across a function boundary. The run checks **stock conservation**, accepted/rejected counts, error recovery and order-sensitive event output. Include repeated updates to the same record and a later read after an unrelated mutation, where legal.

Primary risk: a source program that individually compiles each feature but miscompiles a nested combination or loses state at a branch/failure merge. Prefer `rendered_output_exact` for a stable compact event summary. Choose backend blocks according to current supported target capabilities. The suite contract should name the cross-feature behavior, not try to claim primary ownership for each constituent syntax form.

**Admission gate:** inspect `control_flow`, `implicit_failure_catch_evaluation_order`, `compound_assignment_atomicity_runtime`, `hashmap_js_runtime_contract` and relevant `borrow_*` tests first. If an existing fixture can add the same valuable interaction coherently, extend it rather than creating a ledger case.

### Candidate B: small data-to-report compiler (only after A)

A realistic report generator: take several typed records, filter/group in functions and build a nested string/HTML template with a separate formatting helper. Exercise runtime/compile-time values, options, numeric summaries and nested template components without using complex template control flow. Assert precise rendered content, escaping where relevant and absent output from excluded entries.

Primary risk: values are typed and controlled correctly in code, but template composition, TIR and backend output disagree when nesting, loops and formatted values interact. Inspect existing `template_*` integration contracts and `slot_composition_tests.rs` first. Avoid putting obsolete template `else` constructs back into coverage.

Do not start both new cases at once. Build one and judge its value using actual bugs or realistic mutations that its assertions would catch. Add B only if it protects a demonstrably different cross-stage interaction.

## Agent-ready implementation sequence

### Phase 0 - record exact baseline and ownership map (read-only)

1. Select and record the git SHA being audited. Reconcile the local LOC brief with the actual checked-out source branch rather than mixing sizes across revisions.
2. Run `cargo run --quiet -- tests --audit` and `just test-honesty-audit`. Preserve generated JSON inventories, noting that the checked-in August honesty inventory is historical.
3. Inventory the unit test names, assertion contracts and direct fixture dependencies for each selected candidate cluster. Cross-index against `tests/cases/manifest.toml` contracts, nearby unit tests and actual `expect.toml` assertions.
4. For every candidate, record: file(s), test ID, test level, asserted invariant, expected failure path, backend/profile, existing owner, overlapping owner, decision (`keep`, `extend`, `merge`, `promote`, `remove`), reason and required validation.
5. Capture **nonblank, noncomment LOC and test counts before editing** at the same scope used for after measurements. Ignore changes in fixture layout that only shift code between files.

Deliverable: a compact evidence table, with `confirmed overlap`, `likely overlap - inspect` and `distinct contract - keep` separated. No production or test deletion in this phase.

### Phase 1 - assertion-only and same-fixture cleanup

1. Deduplicate ordered/presence assertions in the two `value_if_*` `expect.toml` files named above, preserving all unique expected fragments.
2. Search only related expectation blocks for that *same provable redundancy*. Inspect each pair manually. Do not turn contains/in-order duplication into a generic runner abstraction or new lint without material recurrence.
3. For small repeated Rust test setups, use existing helpers and table loops only when rows have equivalent setup and remain identifiable by case on failure.
4. Run selected integration cases and relevant Rust tests, then `just validate` at the slice boundary. Record the changed test/LOC counts and surviving contract IDs.

Deliverable: low-risk cleanup with no removed behavior and no altered accepted diagnostics.

### Phase 2 - strengthen existing integration owners

1. Upgrade `control_flow` to replace duplicate loop work with linked nested behavior and a discriminating result.
2. Extend `adversarial_choice_payload_control_flow` to execute its unused nested handler and recovery path and expose intermediate variant paths. Decide on generated-code assertions only after checking backend intent.
3. Extend `adversarial_multi_file_helper_chain` if the relevant multi-file contract is not already covered more strongly by another case.
4. Retain strong existing exact-output cases; do not reimplement the same behavior in new tests. Run each changed case after each fixture edit, followed by the complete canonical integration suite.

Deliverable: greater interaction coverage with no growth in canonical case count unless a separate contract was genuinely missing.

### Phase 3 - one new mini-program, then reassess

1. Search the manifest for a genuine multi-feature gap and select Candidate A or a better evidence-backed alternative.
2. Write a small executable Moth program with deterministic inputs, multiple branch outcomes, a checked invariant and exact observable results. Include a negative/error path that actually executes at runtime.
3. Add one manifest case with an appropriate unique contract and role. Use a new `primary` only if the scenario genuinely owns a new behavior contract; otherwise use an `adversarial` secondary role on an existing family as appropriate.
4. Prefer tests under existing supported backends. Where one backend cannot execute a feature, assert its exact intended typed target rejection rather than forcing parity.
5. Run targeted case, `--audit`, complete integration suite and relevant feature lanes. Evaluate whether the case finds a real issue, a previously uncovered mutation or a credible regression, rather than merely compiling successfully.

Deliverable: one maintainable integration program with richer interaction checks, or a documented decision to extend an existing case instead.

### Phase 4 - deliberate Rust unit cluster consolidation

Audit one ownership cluster per change, in this order: frontend pipeline, AST templates/generics, JS backend, Stage 0 scaffolding, HIR lowering and validation, borrow/Boracle/numeric oracles. The later clusters have a stricter keep bias.

For each cluster: inventory every test and its assertions, identify exact overlaps, choose local table-driven edits or deletion of obsolete assertions, record surviving IDs and compare before/after LOC. **Never remove a unit merely because a Moth integration case mentions the same feature.** Keep internal-state and malformed-input cases even when a rich program provides external coverage.

Run affected unit tests after each coherent edit. Run `just boracle` or `just boracle-campaign` for affected oracle work, standard feature/platform lanes for gates they touch and `just validate-full` on the final integrated tree prepared for merging. Investigate failures before accepting changed expected values or weakened diagnostic contracts.

### Phase 5 - report measured outcome

Report removals and merges as proof-backed decisions, not percentages:

| Required entry | Meaning |
|---|---|
| Original / surviving test IDs | What still exercises each contract |
| Exact failure paths retained | Which success, failure, diagnostic and target boundaries remain |
| Before/after noncomment LOC and test counts | Same scope, same branch, same counting rules |
| Integration outcomes and supported backends | Which executable paths are now observed |
| Commands and revision actually tested | Enough to reproduce the verification |
| New gaps or known limitations | What remains untested and why |

Any unproven overlap stays a candidate, not an approved deletion. The desired output is a smaller, more useful verification system without concealed edge-case losses.

## References inspected

- [Canonical integration manifest](https://github.com/nyejames/moth/blob/1343c64ecbc5433b546c3ad31cf5c175cf51f184/tests/cases/manifest.toml)
- [Frontend pipeline stage-boundary tests](https://github.com/nyejames/moth/blob/1343c64ecbc5433b546c3ad31cf5c175cf51f184/src/compiler_frontend/tests/frontend_pipeline_tests.rs)
- [HIR validation tests](https://github.com/nyejames/moth/blob/1343c64ecbc5433b546c3ad31cf5c175cf51f184/src/compiler_frontend/hir/tests/hir_validation_tests.rs)
- [JavaScript control-flow emitter tests](https://github.com/nyejames/moth/blob/1343c64ecbc5433b546c3ad31cf5c175cf51f184/src/backends/js/tests/control_flow.rs)
- [Boracle oracle tests](https://github.com/nyejames/moth/blob/1343c64ecbc5433b546c3ad31cf5c175cf51f184/src/compiler_frontend/analysis/borrow_checker/boracle/oracle/tests/mod.rs)
- [Integration tests policy](https://github.com/nyejames/moth/blob/1343c64ecbc5433b546c3ad31cf5c175cf51f184/docs/src/developer-docs/style-guide/testing.mtf)
- [Validation gate policy](https://github.com/nyejames/moth/blob/1343c64ecbc5433b546c3ad31cf5c175cf51f184/docs/src/developer-docs/style-guide/validation.mtf)
- Supplied `moth-test-concentration-audit-brief(1).md` and `test(2).moth` (Ember Relay)
