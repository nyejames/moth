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
