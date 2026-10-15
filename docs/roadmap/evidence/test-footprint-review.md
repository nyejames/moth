# Test footprint implementation evidence

## Baseline and measurement rules

Work started on `link-fixes-and-test-cleanup` at `bf84a6c6da188820a549edd37effadce2e2c744f`, with a clean staged, unstaged and untracked worktree. The implementation uses this checkout rather than the plan's historical `expression-refactor` revision. Other roadmap programmes remain paused.

The local toolchain reports Rust `1.99.0` (`b940084d7`, aarch64-apple-darwin), Clippy `0.1.99` and rustfmt `1.10.0-stable`. Authoritative runtime commands explicitly select `/Users/aneirinjames/.nvm/versions/node/v24.21.0/bin` on `PATH`, matching `.node-version`. The initial default executable reported Node `26.10.0`; it supplied no runtime acceptance evidence.

Rust measurements use `tokei 14.0.0` code lines, excluding blanks and comment-only lines. Embedded Moth fixture strings count as Rust fixture code. Test counts include every literal `#[test]` attribute, including feature- and platform-gated definitions. The test-directory scope includes Rust files under a `tests` path component or named `tests.rs` within `src`, `crates/moth-lexical`, `crates/moth-mon` and `xtask/src`.

Moth measurements count nonblank physical `.moth` lines except lines whose first non-whitespace characters are `--`, including config and helpers and excluding generated `dev` and `release` directories. Expectation measurements exclude blank lines and comment-only `#` lines. These explicit physical-line rules support same-scope before/after comparison. They do not reconstruct the unavailable counting method behind the supplied 328,410 Rust LOC estimate or its 78,981-line whole-corpus figure.

| Baseline scope | Noncomment LOC | Test definitions or cases |
|---|---:|---:|
| Rust test directories, 491 files | 319,605 | 7,115 test definitions |
| Ten selected Rust review files | 38,455 | 665 test definitions |
| Canonical Moth sources, 5,473 files | 44,340 | 2,395 cases |

| Selected Rust file | Baseline noncomment LOC | Baseline test definitions |
|---|---:|---:|
| `frontend_pipeline_tests.rs` | 4,698 | 74 |
| `frozen_body_tests.rs` | 4,328 | 46 |
| `slot_composition_tests.rs` | 2,720 | 54 |
| `parser_tir_tests.rs` | 2,350 | 46 |
| JS `control_flow.rs` | 2,428 | 18 |
| `stage0_filesystem_identity_tests.rs` | 2,971 | 75 |
| `hir_expression_lowering_tests.rs` | 4,518 | 63 |
| `hir_validation_tests.rs` | 4,635 | 87 |
| Boracle oracle `tests/mod.rs` | 5,430 | 111 |
| `diagnostic_model_tests.rs` | 4,377 | 91 |

| Existing integration case | Baseline Moth LOC | Baseline expectation LOC |
|---|---:|---:|
| `value_if_block_single_predicate` | 99 | 33 |
| `value_if_multi_bind_inferred_single_predicate` | 104 | 34 |
| `control_flow` | 39 | 4 |
| `adversarial_choice_payload_control_flow` | 94 | 9 |
| `adversarial_multi_file_helper_chain` | 16 | 4 |

## Baseline execution and inventories

| Command at `bf84a6c6` | Observed result |
|---|---|
| `cargo run --quiet -- tests --audit` | 2,395 cases, 2,786 backend executions, zero hard suite-policy violations |
| `just test-honesty-audit` | 1,520 scanned files, including 820 test-owning files, zero hard findings, zero source or feature-lane findings, 208 suite advisories |
| `cargo run --quiet -- tests --case <id> --terse`, for each of the five cases above | Each passed 1/1, including actual emitted JavaScript execution |

Advisory and weak-assertion review counts supply investigation leads, not deletion permission.

Generated reports remain under `target/test-reports/`. Ignored `tmp/test-footprint/baseline-integration_suite_inventory.json` and `baseline-test_honesty_inventory.json` preserve their activation snapshots before later commands replace them. The same directory retains `baseline-rust-tokei.json` and `baseline-measurements.json`, including per-file Moth counts. The tracked August honesty inventory remains historical and unchanged.

## Phase 0 ownership decisions

`test-footprint-contracts.json` records all 665 selected unit IDs, their actual assertion contracts, direct fixture owners, lanes and initial dispositions. A mechanical comparison matched every recorded ID to its baseline definition and corrected one transcribed Oracle name. The ledger preserves baseline locations rather than following subsequent line shifts.

| Classification | Candidate or owner | Decision and retained failure boundary |
|---|---|---|
| Confirmed overlap | Two `value_if_*` presence/ordered assertion pairs | Merge repeated presence declarations only. Preserve both ordered sequences and their unique case markers. |
| Confirmed overlap | `control_flow` second range sum | Extend in place with a loop/helper/nested-decision result. Preserve original if, nested-if and match outcomes. |
| Confirmed gap | Choice-payload adversarial case | Execute unused handler branches, observe each loop variant and run actual recovery. Compare backend representation owners before changing artefact assertions. |
| Confirmed gap | Transitive helper chain | Extend the same three-file chain with input-dependent results. Direct imports, aliases and module initialisation own different contracts. |
| Confirmed overlap | Two parser/TIR subsets, one HIR template pair and one Oracle alias-write pair | Review the exact survivor mappings in the ledger during the owning implementation phases. |
| Likely overlap - inspect | One JS jump-argument subset and four Stage 0 fixture pairs | Compare complete fixtures and assertions before consolidation. Preserve destination binding, facade counts, raw-name error precedence, nearest ownership and typed duplicate-root reasons. |
| Fixture-only test - remove candidate | `expression_test_builder_produces_valid_hir_module_metadata` | Only checks test-builder wiring and bare validation success. Preserve real lowered-module acceptance in `valid_module_passes_explicit_validation` and every malformed-HIR metadata/provenance contract. |
| Distinct contract - keep | Frontend pipeline, frozen materialisation and TIR slot composition | Preserve all stage-handoff, origin/lifetime, schema/routing/replay and malformed-input owners. Existing helpers already centralise useful setup. |
| Distinct contract - keep | HIR validation, diagnostic model and remaining Oracle cases | Preserve malformed-IR/numeric-domain, payload/span/remapping, capability/generation/alias and bounded-execution contracts. |
| Distinct contract - keep | Debug/release fixed-numeric siblings and protected exact-output cases | Preserve both profile executions and exact failure/evaluation-order assertions. Repeated source bytes do not prove redundant behaviour. |
| Missing interaction | Fulfilment ledger | Admit one small HTML scenario for stock conservation across repeated, partial/rejected and returned orders with a caught helper error. Existing collection, arithmetic-atomicity and borrow cases do not assert this domain invariant. |

The miniature report generator remains unadmitted. Existing template owners cover its constituent operations, and no distinct executable regression has yet justified another case. Phase 3 first assesses the ledger against a valid mutation.

The baseline documentation checkpoint command `cargo run --quiet -- build docs --release` built 81 output files. The read-only inventory phase changed no production or test sources.

The independent Phase 0 audit required two evidence corrections: distinguish test-owning files from test counts, and validate the complete Oracle module rather than filtering out its three differently named reference cross-checks. Both corrections now appear above and in the ledger. Its helper-test review also identified the fixture-only removal candidate. These findings changed no executable sources.

## Phase 1 assertion-only cleanup

Both `value_if_*` cases retain their unique presence marker and their complete ordered output sequence. The removed presence fragments already occur in the unchanged ordered list. A throwaway TOML comparison found every other parsed backend field identical. No source fixture, backend block or diagnostic changed.

| Existing case and surviving owner | Presence fragments removed | Expectation LOC before / after | Moth LOC before / after |
|---|---:|---:|---:|
| `value_if_block_single_predicate`, `language.expressions.value_if_block_single_predicate_acceptance` | 12 | 33 / 21 | 99 / 99 |
| `value_if_multi_bind_inferred_single_predicate`, `language.expressions.value_if_multi_bind_inferred_single_predicate_acceptance` | 13 | 34 / 21 | 104 / 104 |

The selected HTML runtime commands each passed 1/1. `just validate` passed formatting, default Clippy, feature coverage, source/dependency audits, workspace tests including 6,128 compiler tests, all 2,786 integration executions and the docs check. All commands selected pinned Node `24.21.0`. Canonical case count and Rust test counts remain unchanged.

The independent presence/order audit accepted the phase without findings after tracing both assertion fields into the same runtime-output evaluator.

## Phase 2 existing integration owners

| Case and unchanged contract | Newly observed failure surface | Moth LOC before / after |
|---|---|---:|
| `control_flow`, `language.control_flow.if_nested_match_execution` | Nested helper returns produce `2,4,9,6` inside a loop, skipping input `4`. Exact output preserves the original branch/match results and sum `10`, with linked total `21`. | 39 / 53 |
| `adversarial_choice_payload_control_flow`, `language.choices.payload_control_flow_chain_executes` | Exact output observes small/large/failure/pending handlers, payloads from every loop iteration, separate counts, original nested/imported results and actual `zero:0` recovery exactly once. | 94 / 131 |
| `adversarial_multi_file_helper_chain`, `language.modules.transitive_helper_chain_executes` | Exact `A-hello-B\|C-world-D\|missing-again-B` observes two input-dependent prefixes and absent-option recovery through the original three-source chain. | 16 / 30 |

Each changed case passed its selected HTML runtime command immediately after its fixture handoff. The complete canonical integration run passed 2,786/2,786 with 2,395 cases. `cargo run --quiet -- check docs --terse` reported no errors or warnings. Expectation LOC remains `4`, `9` and `4` respectively. No Rust test, manifest role, backend/profile or diagnostic expectation changed.

The choice case keeps its two named-payload constructor snippets because the compared unit carrier owner does not prove those field structures. The removed tag comparisons and generic `if (` snippets survive in `choice_match_lowers_to_structured_if_with_choice_tags`; `emits_structured_if_without_dispatcher` owns conditional strategy. `literal_match_uses_dispatcher_when_cfg_contains_cycle` retains dispatcher `while (true)` and switch structure. Runtime output does not replace these backend contracts.

Fresh independent semantic and backend-ownership audits both accepted Phase 2 without findings. The changed progress route also rebuilt through `cargo run --quiet -- build docs --release`, producing 81 outputs.

## Phase 3 fulfilment ledger and discovered header defect

`fulfilment_ledger_runtime` adds one HTML primary for `language.scenarios.fulfilment_stock_conservation`. Existing collection, catch-order, arithmetic-atomicity and borrow owners do not check conservation across this domain-level sequence: request `4`, partially fulfil request `8`, reject request `1` from empty stock, return `3`, fulfil request `2` and reject request `0`. The event ledger preserves original error codes/messages and exact order. Per-event assertions protect conservation and nonnegative stock; exact output protects accepted/partial/rejected counts, final balances and an independent copied snapshot.

The first fixture draft used reserved variant name `Return`, which correctly failed with `MOTH-RULE-0039`. Renaming it `Restock` preserved the domain event. The valid multiline collection then exposed a compiler defect: `MOTH-RULE-0002` at line 30 treated `Order::Request(4)` as another `Order` declaration. The existing multiline struct-constructor fixture did not exercise qualified choice constructors.

The header walk now reuses allocation-free `NestingDepth` while retaining runtime tokens. Nested expression newlines no longer trigger declaration/dependency discovery. Declaration and template handlers still own the ranges they consume. The walk preserves EOF for malformed-expression diagnostics rather than creating an infrastructure failure. The existing `start_function_retains_segmented_source_runs_in_order_with_eof` test now checks collection/map/grouped/call content, retained path and delimiter order, subsequent genuine declarations and a true duplicate's source span. It adds no test definition or second parser.

After the repair, the ledger runtime command passed 1/1 and all 309 selected header tests passed. A temporary valid mutation debited one stock unit before returning the empty-stock error. The caller recovered the original error, then the per-event conservation assertion failed at runtime: 0/1 correct, unexpected uncaught Moth Error `assertion failed`. This proves an assertion detects mutation surviving a recovered helper error, not just successful compilation. The mutation was removed.

Parent review removed final source assertions that duplicated the exact output, retaining the independent per-event invariants. The real multiline syntax remains in the permanent fixture. HTML-Wasm aggregate, collection, mutable-parameter and recoverable-error lowering remains outside this scenario's supported execution lane. The report-generator candidate remains unadmitted rather than adding a second programme without distinct mutation evidence.

The restored ledger passed 1/1. `cargo run --quiet -- tests --audit` reported 2,396 cases and 2,787 backend executions, and the full integration run passed 2,787/2,787. The new fixture adds 71 noncomment Moth lines across source and config, with four expectation lines. `cargo fmt -p moth` passed. A direct `cargo run --quiet -- check tests/cases/fulfilment_ledger_runtime/input --terse` reported no errors or warnings, and the changed progress page rebuilt with 81 documentation outputs.

The production audit found one required P2 correction: the new nested-token path accepted an unclosed nested template in an imported root, whose discarded runtime body has no AST consumer. Extending the existing imported-root test reproduced the lost diagnostic before correction. Nested templates now use the existing structural capture and retain their complete range without publishing a top-level fragment. The regression requires one `MOTH-SYNTAX-0017`, typed missing `]` and exact EOF span for imported/active plain/nested forms. A positive existing test also verifies nested collection/group templates leave the direct fragment count at one. After correction, all 309 header tests, the ledger 1/1 and the full 2,787 integration executions passed again.

The independent ledger-admission audit accepted the scenario without findings. Fresh focused production verification accepted the imported-template correction and single-pass ownership with no remaining required finding. Its positive nested-template extension checks fragment count rather than exact retained tokens or immediately following declarations; control-flow inspection and the existing segmented-retention test provide the supporting evidence for those properties.

## Phase 4 unit ownership checkpoints

### Frontend pipeline

Keep all 74 original `frontend_pipeline_tests.rs` IDs at 4,698 noncomment LOC. Its shared `failure_project` helper already centralises useful repeated setup. The similar implicit-failure fixtures assert different producer facts, CFG joins, success/error arity, recursion, write-back, float guards and borrow outcomes. Integration output cannot replace those handoff facts, and no equivalent same-boundary rows justified a table conversion or deletion. `cargo test -p moth --lib compiler_frontend::tests::frontend_pipeline_tests -- --format terse` passed 74/74.

The independent frontend-cluster audit accepted retention without required correction. This preserves existing coverage rather than proving stable-origin joins through the harness's empty origin lookup or HIR narrowing from an AST-signature assertion. The documentation checkpoint built 81 outputs.
