# Open Audit Findings

Unresolved audit work. Evidence stays in the owning report under [audits](./audits/README.md). Rejected, superseded and closed findings leave this index but remain in their report and in Git history.

- [Audit guide](./audit-guide.md)
- [Audit log](./audit-log.md)
- [Audit-kind index](./audit-kinds/README.md)

## Audits in progress

None.

## Candidate findings

Filed, not yet triaged.

- [AUD-0005-F01: Tight `<`/`>` spacing suppression accepts binary comparisons that violate the operator spacing contract](./audits/AUD-0005-tokenizer-correctness.md#aud-0005-f01-tight--spacing-suppression-accepts-binary-comparisons-that-violate-the-operator-spacing-contract)
  - `Correctness` | `frontend.tokenizer`
- [AUD-0005-F02: Consecutive bare-CR newlines are consumed as horizontal whitespace, drifting line and column tracking](./audits/AUD-0005-tokenizer-correctness.md#aud-0005-f02-consecutive-bare-cr-newlines-are-consumed-as-horizontal-whitespace-drifting-line-and-column-tracking)
  - `Correctness` | `frontend.tokenizer`
- [AUD-0005-F03 (linked, Diagnostics lane): Unicode numeric characters are diagnosed as `_` separator errors](./audits/AUD-0005-tokenizer-correctness.md#aud-0005-f03-linked-diagnostics-lane-unicode-numeric-characters-are-diagnosed-as-_-separator-errors)
  - `Diagnostics` | `frontend.tokenizer`

## Accepted

Triaged and authorised. Not yet fixed.

None.

## In progress

Being fixed now.

None.

## Blocked

Waiting on a design decision.

None.

## Resolved in this branch

- AUD-0002-F01 was accepted and resolved by batching provider-independent directory source
  read/tokenize work for sufficiently large owned-source sets while preserving serial reachability
  and provider resolution. The correction also remaps the mutable file-owned path-syntax table
  before header parsing and adds focused coverage for speculative tokenizer failures, deterministic
  order, exact-once reads and threshold behaviour. Full validation, benchmark checks and the
  required coordinator auditor pass returned clean. See the [triage record](./audits/AUD-0002-stage0-discovery-preparation-performance.md#aud-0002-f01-directory-stage-0-discovery-and-preparation-is-fully-serial-while-every-parallel-and-caching-mechanism-is-reachable-only-from-the-single-file-synthetic-path).

- AUD-0002-F05 was accepted and resolved by scoping the synthetic scheduler and cache-loader
  comments to their reachable single-file paths and describing the directory batch with precise
  source-kind and serial-boundary terminology. The required full validation and coordinator auditor
  pass returned clean. See the [triage record](./audits/AUD-0002-stage0-discovery-preparation-performance.md#aud-0002-f05-three-doc-comments-describe-parallel-and-cached-stage-0-behaviour-that-the-directory-path-cannot-reach).

- AUD-0002-F06 was accepted and implemented. `docs/roadmap/audit-log.md` now registers
  `build.stage0.discovery`, `build.stage0.preparation`, `build.stage0.graph`,
  `build.stage0.scheduling` and the `build.stage0` composite, and AUD-0002's Performance coverage is
  recorded as `P 2026-08 AUD-0002` against the first two. The compiler frontend and the proposed
  `contract.module_compilation_handoff` scope remain unregistered. See the
  [triage record](./audits/AUD-0002-stage0-discovery-preparation-performance.md#aud-0002-f06-the-audit-scope-registry-has-no-entry-covering-stage-0-so-this-audit-can-record-no-freshness).

- AUD-0002-F02 was accepted and resolved by hoisting one immutable `StringTableForkSource` above
  the directory module loop. The coordinator validation and benchmark evidence passed, and the
  required auditor pass returned clean. See the [triage record](./audits/AUD-0002-stage0-discovery-preparation-performance.md#aud-0002-f02-fork_for_module-is-called-per-module-inside-the-discovery-loop-copying-the-whole-string-table-once-per-module-against-its-own-api-guidance).

- AUD-0002-F03 was accepted and resolved by hoisting one `ModulePreparationContext` (and its owned
  `ProjectPathResolver`) above the directory module loop. The coordinator validation and benchmark
  evidence passed, and the required auditor pass returned clean. See the [triage record](./audits/AUD-0002-stage0-discovery-preparation-performance.md#aud-0002-f03-projectpathresolver-is-deep-cloned-once-per-module-inside-the-discovery-loop).

- AUD-0002-F04 was accepted and resolved through the central timing and counter owners: directory
  read/tokenize now contributes to `frontend.prepare`, directory input facts are reported, and
  `StringTableBase::from_table` has stable counter coverage. Full validation and the required
  auditor pass returned clean. See the [triage record](./audits/AUD-0002-stage0-discovery-preparation-performance.md#aud-0002-f04-stage-0-directory-read--tokenize-is-unmeasured-and-the-counters-that-would-expose-it-read-zero-on-every-directory-build).

- AUD-0001-F01, AUD-0001-F02 and AUD-0001-F03 were corrected on branch `test-suite-honesty`. See each finding's triage record in [AUD-0001](./audits/AUD-0001-test-support-redundancy.md).
- AUD-0001-F04 was resolved in Phase 11 of the test-suite-honesty campaign, and its design gate turned out not to apply: the evidence recorded two `build_ast` implementations with different semantics, but there was one implementation reached through four names, so no consumer could have selected the wrong one. Corrected as a pure rename to `build_ast_with_registered_types`, `immutable_reference_expr` and `inferred_type_reference_expr`, with the bare forward deleted. See the resolution note in [AUD-0001](./audits/AUD-0001-test-support-redundancy.md#aud-0001-f04-sibling-fixture-supports-export-colliding-names-with-different-semantics).
- AUD-0001-F05 was resolved in the same phase: Phase 7 decided retire rather than adopt, and Phase 11 deleted `assert_diagnostic_reason` and `error_code_counts`. No token caller was added.

- AUD-0004-F01 and AUD-0004-F02 were accepted and resolved by simplifying the audit framework rather
  than by extending the registry. The scope registry is now a coverage ledger: an audit adds the row
  for the area it covers as part of its run, so there is no longer a registration step that can
  deadlock, and no scope taxonomy to satisfy before inspection. A `Never audited` section makes the
  unowned surface visible, which is the signal F01 showed the old two-table registry could not give.
  See the [report](./audits/AUD-0004-audit-scope-registry-coverage.md).
- AUD-0004-F03 was accepted and fixed. `tests.harness` now cites
  `src/compiler_frontend/tests/frontend_pipeline_tests.rs` at its real path.

