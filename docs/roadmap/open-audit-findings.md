# Open Audit Findings

Unresolved audit work. Evidence stays in the owning report under [audits](./audits/README.md). Rejected, superseded and closed findings leave this index but remain in their report and in Git history.

- [Audit guide](./audit-guide.md)
- [Audit log](./audit-log.md)
- [Audit-kind index](./audit-kinds/README.md)

## Audits in progress

None.

## Candidate findings

Filed, not yet triaged.

None.

## Accepted

Triaged and authorised. Not yet fixed.

- [AUD-0012-F01: Optional receivers with present defaults reject explicit optional values](./audits/AUD-0012-optional-defaults-correctness.md#aud-0012-f01-optional-receivers-with-present-defaults-reject-explicit-optional-values)
  - Correctness | `frontend.optional_defaults`

## In progress

Being fixed now.

None.

## Blocked

Waiting on a design decision.

None.

## Resolved in this branch
- AUD-0013-F01 and AUD-0013-F02 were accepted and resolved by aligning source and MON named/map
  entries with the documented outer-spacing rule, including source line-boundary cases, and by
  enforcing adjacency around `::` in choice construction and patterns. Spaced choice declarations
  remain valid. The 32-case equals parity corpus, reader errors and choice-spacing parity/frontend
  regressions passed with focused package tests and clippy. The exact-tree `just validate-full`
  result is recorded in the closeout commit message. See the [AUD-0013-F01 report](./audits/AUD-0013-mon-source-parity-spacing-correctness.md#aud-0013-f01-source-and-mon-entries-bypass-the-outer-spacing-contract)
  and [AUD-0013-F02 report](./audits/AUD-0013-mon-source-parity-spacing-correctness.md#aud-0013-f02-choice-variant-separator-spacing-differs-between-source-and-mon).

- AUD-0014-F01 was accepted and resolved by routing `character_is_missing_rhs_boundary` through
  the shared `moth_lexical::is_line_break` predicate while retaining its punctuation cases.
  Focused `moth` tests and clippy passed; the exact-tree `just validate-full` result is recorded
  in the closeout commit message. See the [AUD-0014 report](./audits/AUD-0014-tokenizer-line-break-redundancy.md#aud-0014-f01-lexer-boundary-helper-repeats-shared-line-break-policy).

- AUD-0015-F01 was accepted and resolved by removing the unreachable duplicate choice-payload
  default check from `prepare_field`; `prepare_fields` remains the single policy owner. Focused
  `moth-mon` tests and clippy passed; the exact-tree `just validate-full` result is recorded in
  the closeout commit message. See the [AUD-0015 report](./audits/AUD-0015-mon-schema-preparation-redundancy.md#aud-0015-f01-choice-payload-defaults-are-rejected-twice-in-schema-preparation).

- AUD-0016-F01 was accepted and resolved by aligning empty-container references with the accepted
  `{=}`-map / `{}`-collection design while keeping current implementation status separate. The
  documentation gate passed; the exact-tree `just validate-full` result is recorded in the
  closeout commit message. See the [AUD-0016 report](./audits/AUD-0016-empty-container-documentation.md#aud-0016-f01-empty-container-documentation-conflates-current-behavior-with-accepted-design).

- AUD-0009-F01, AUD-0009-F02 and linked AUD-0010-F02 were accepted and resolved by sharing
  the LF/CR logical-line rule, tightening MON constructor payload lookahead to the same logical
  line, and reusing the MON reader's trivia walk under a cursor checkpoint. The lexer grammar
  stayed unchanged; MON's acceptance tightening was recorded. The bounded parity matrix and
  reader tests protect the whitespace and constructor boundaries, exact spans and keyword
  openers; scoped package/parity tests and clippy passed; `just validate-full` passed. See the
  [AUD-0009-F01 triage record](./audits/AUD-0009-mon-source-parity-correctness.md#aud-0009-f01-unicode-horizontal-whitespace-splits-mon-named-entries),
  [AUD-0009-F02 triage record](./audits/AUD-0009-mon-source-parity-correctness.md#aud-0009-f02-mon-constructor-payload-lookahead-crosses-source-line-boundaries),
  [AUD-0010-F02 triage record](./audits/AUD-0010-mon-syntax-parity-tests.md#aud-0010-f02-parity-tests-miss-decisive-token-boundaries-and-receivers)
  and [AUD-0011-F01 triage record](./audits/AUD-0011-mon-reader-redundancy.md#aud-0011-f01-mon-payload-lookahead-duplicates-its-trivia-scanner).

- AUD-0009-F03 was accepted and resolved by charging each rejected label, variant and
  qualifier before copying it into a path or error detail, and charging each schema field once
  before validation. Reserved-name budget regressions failed before the fix; an exact-budget
  nested and payload schema remains accepted. Scoped MON tests and clippy passed; `just validate-full` passed. See the
  [AUD-0009-F03 triage record](./audits/AUD-0009-mon-source-parity-correctness.md#aud-0009-f03-rejected-names-bypass-decoded-byte-accounting-in-diagnostics).

- [AUD-0010-F01: Deferred-feature gap fixtures lack equivalent typed source receivers](./audits/AUD-0010-mon-syntax-parity-tests.md#aud-0010-f01-deferred-feature-gap-fixtures-lack-equivalent-typed-source-receivers) was accepted and resolved by giving the contextual-choice gap a typed `Holder` receiver and the source `{=}` gap a typed runtime binding.
  The source `{=}` case remains a deferred-feature gap. `MapHolder(scores = {})` still reports
  MOTH-TYPE-0002 on the current compiler; that observation is not a defect against the accepted
  design, where `{}` is always a collection. Scoped parity tests and clippy passed;
  `just validate-full` passed. See the [AUD-0016-F01 documentation finding](./audits/AUD-0016-empty-container-documentation.md#aud-0016-f01-empty-container-documentation-conflates-current-behavior-with-accepted-design).

- AUD-0010-F03 was accepted and resolved by adding optional-renamed and inactive-target
  forbidden-dependency fixtures to the existing guard tests. Both passed against unchanged
  production code, so this remained coverage-only. Scoped xtask tests and clippy passed; `just validate-full` passed. See the
  [AUD-0010-F03 triage record](./audits/AUD-0010-mon-syntax-parity-tests.md#aud-0010-f03-dependency-guard-lacks-optional-and-inactive-target-fixtures).

- AUD-0007-F03 and linked AUD-0008-F03 were accepted and resolved by applying the shared
  safe-integer/profile-bound projection to standard external `Error.code` values before creating
  a Moth Error. Generated standard and Int64 fallible wrappers execute in Node: signed endpoints
  and invalid external codes exercise numeric fallback; the source-expression pin was deleted.
  `just validate` passed. See the [Correctness triage record](./audits/AUD-0007-numeric-profile-boundary-correctness.md#aud-0007-f03-standard-profile-external-errors-admit-out-of-domain-errorcode)
  and [Tests triage record](./audits/AUD-0008-numeric-boundary-regression-tests.md#aud-0008-f03-standard-external-error-code-test-pins-a-permissive-expression).

- AUD-0007-F02 and linked AUD-0008-F02 were accepted and resolved by stopping const-template
  Int64 counters at their final valid bound before overflow, treating equal/empty Float bounds
  without an unnecessary progress check, and preserving nonterminal stalled-step diagnostics.
  Int64 extrema/near-end directions and Float32 equal bounds have exact-output integration
  assertions; the negative fixture now requires an actual successor. `just validate` passed.
  See the [Correctness triage record](./audits/AUD-0007-numeric-profile-boundary-correctness.md#aud-0007-f02-int64-const-template-ranges-require-an-overflowed-terminal-successor)
  and [Tests triage record](./audits/AUD-0008-numeric-boundary-regression-tests.md#aud-0008-f02-empty-float-const-range-is-pinned-as-a-non-progress-error).

- AUD-0007-F01 and linked AUD-0008-F01 were accepted and resolved by classifying direct fixed
  scalars (not options) in AST template validation and HIR numeric text lowering, removing the
  unused option-unwrapping helper, adding a source-level optional-U8 rejection fixture and
  asserting direct U8 output. The authored diagnostic is now `MOTH-SYNTAX-0022`; HTML-Wasm's
  feature gate and Byte rejection remain. `just validate` passed. See the
  [Correctness triage record](./audits/AUD-0007-numeric-profile-boundary-correctness.md#aud-0007-f01-optional-fixed-scalars-escape-template-renderability-checks)
  and [Tests triage record](./audits/AUD-0008-numeric-boundary-regression-tests.md#aud-0008-f01-optional-fixed-scalar-template-interpolation-lacks-a-source-diagnostic-owner).

- AUD-0005-F01 was accepted and resolved by deleting the generic-angle spacing suppressors so `<`
  and `>` always enforce `MOTH-SYNTAX-0031`. Template-tag suppressions in non-Normal mode remain.
  Tight `identity<Int>(42)` is now a tokenizer spacing error; the dedicated `MOTH-RULE-0057`
  fixture uses spaced `identity < Int > (42)`. `just validate` passed. See the
  [triage record](./audits/AUD-0005-tokenizer-correctness.md#aud-0005-f01-tight--spacing-suppression-accepts-binary-comparisons-that-violate-the-operator-spacing-contract).

- AUD-0005-F03 was accepted and resolved by dispatching unsigned and signed numerics on
  `is_ascii_digit()` so Unicode Nd/Nl/No fall through to `invalid_character` (`MOTH-SYNTAX-0007`).
  Genuine `_` separator mistakes still use `MOTH-SYNTAX-0008`. `just validate` passed. See the
  [triage record](./audits/AUD-0005-tokenizer-correctness.md#aud-0005-f03-linked-diagnostics-lane-unicode-numeric-characters-are-diagnosed-as-_-separator-errors).

- AUD-0005-F02 was closed as superseded. TokenStream no longer tracks line and column; diagnostic
  positions come from byte spans plus `source::line_index`, which treats each bare CR as a line
  boundary. Empirical `moth check` of `a = 1\\n\\nzzzz` and `a = 1\\r\\rzzzz` both report
  `MOTH-RULE-0031` at `@page.moth:3:5`. See the [triage record](./audits/AUD-0005-tokenizer-correctness.md#aud-0005-f02-consecutive-bare-cr-newlines-are-consumed-as-horizontal-whitespace-drifting-line-and-column-tracking).

- AUD-0006-F06 was split: the missing authored-`start` valid-body fixture is accepted as part of
  F01. The proposed rewrite of ~51 `compile_project_frontend` harness sites to
  `CompilerSymbolSet::preseeded_table` was rejected — those tests have no numeric `StringId`
  assertions, `tests/cases/` already runs production preseeding, and `compiler_symbols_tests`
  already pin the preseed prefix. See the [triage record](./audits/AUD-0006-frontend-symbols-correctness.md#aud-0006-f06-linked-tests-lane-no-rust-harness-test-drives-the-frontend-with-a-preseeded-string-table-and-the-only-authored-start-fixture-cannot-reach-f01).

- AUD-0006-F01 was accepted and resolved by rejecting ActiveModuleRoot authored `start` as
  `MOTH-RULE-0039` (`ReservedNameOwner::ImplicitStart`) in `validate_declared_name`, skipping
  `HeaderKind::StartFunction`, keeping helper-file `start` legal, and renaming
  `this_local_declaration_error` off `start` so it still reports exactly `MOTH-RULE-0040`.
  The unread `CompilerOwnedDeclaration.kind` field was deleted. See the
  [triage record](./audits/AUD-0006-frontend-symbols-correctness.md#aud-0006-f01-a-user-authored-root-level-start-function-reaches-the-compiler-invariant-lane-and-is-reported-as-a-compiler-bug).

- AUD-0006-F02 was accepted and resolved by aligning identifier naming predicates with Unicode
  letter case. `café` and `naïve_value` no longer warn `MOTH-RULE-0021`. See the
  [triage record](./audits/AUD-0006-frontend-symbols-correctness.md#aud-0006-f02-linked-diagnostics-lane-ascii-only-naming-predicates-warn-on-legal-unicode-identifiers-telling-the-user-to-use-the-convention-they-already-used).

- AUD-0006-F03 was accepted and resolved by deleting unused `StringIdRemap::has_non_identity_after`
  and the two tests that only pinned it. See the
  [triage record](./audits/AUD-0006-frontend-symbols-correctness.md#aud-0006-f03-linked-redundancy-lane-stringidremaphas_non_identity_after-has-no-production-caller-and-its-only-consumers-are-the-tests-that-pin-it).

- AUD-0006-F04 was accepted and resolved by importing the crate `StringTableResolver` in
  `path_interner/frozen.rs` and deleting the private duplicate. See the
  [triage record](./audits/AUD-0006-frontend-symbols-correctness.md#aud-0006-f04-linked-redundancy-lane-path_internerfrozeners-defines-a-private-duplicate-of-the-crates-stringtableresolver-trait).

- AUD-0006-F05 was accepted and resolved by deleting unused `CompilerSymbolIds` /
  `PreseededStringTable` and returning `StringTable` from `preseeded_table`. See the
  [triage record](./audits/AUD-0006-frontend-symbols-correctness.md#aud-0006-f05-linked-redundancy-lane-the-typed-compilersymbolids-accessors-have-no-production-consumer).


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

