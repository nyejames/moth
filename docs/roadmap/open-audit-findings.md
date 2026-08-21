# Open Audit Findings

This is the live index of unresolved audit work. Each entry links to its owning report under [audits](./audits/README.md). Evidence and analysis stay in the report. Closed, rejected, duplicate and superseded findings leave this index but remain in report and Git history.

- [Audit guide](./audit-guide.md)
- [Audit log](./audit-log.md)
- [Audit-kind index](./audit-kinds/README.md)

## Audits in progress

None.

## Candidate findings

- [AUD-0002-F01: Directory Stage 0 discovery and preparation is fully serial while every parallel and caching mechanism is reachable only from the single-file synthetic path](./audits/AUD-0002-stage0-discovery-preparation-performance.md#aud-0002-f01-directory-stage-0-discovery-and-preparation-is-fully-serial-while-every-parallel-and-caching-mechanism-is-reachable-only-from-the-single-file-synthetic-path)
  - `Performance` | `build.stage0.discovery` (root owner), `build.stage0.preparation`
- [AUD-0002-F03: `ProjectPathResolver` is deep-cloned once per module inside the discovery loop](./audits/AUD-0002-stage0-discovery-preparation-performance.md#aud-0002-f03-projectpathresolver-is-deep-cloned-once-per-module-inside-the-discovery-loop)
  - `Performance` | `build.stage0.discovery`
- [AUD-0002-F04: Stage 0 directory read + tokenize is unmeasured, and the counters that would expose it read zero on every directory build](./audits/AUD-0002-stage0-discovery-preparation-performance.md#aud-0002-f04-stage-0-directory-read--tokenize-is-unmeasured-and-the-counters-that-would-expose-it-read-zero-on-every-directory-build)
  - `Performance` | `build.stage0.discovery` — boundary escalation, fix reaches instrumentation and timing outside Stage 0
- [AUD-0002-F05: Three doc comments describe parallel and cached Stage 0 behaviour that the directory path cannot reach](./audits/AUD-0002-stage0-discovery-preparation-performance.md#aud-0002-f05-three-doc-comments-describe-parallel-and-cached-stage-0-behaviour-that-the-directory-path-cannot-reach)
  - `Comments` | `build.stage0.preparation` (root owner), `build.stage0.discovery`

## Accepted and queued findings

None.

## Active fixes

None.

## Blocked or design-gated findings

None.

## Awaiting verification

None.

## Resolved in this branch

- AUD-0002-F06 was accepted and implemented. `docs/roadmap/audit-log.md` now registers
  `build.stage0.discovery`, `build.stage0.preparation`, `build.stage0.graph`,
  `build.stage0.scheduling` and the `build.stage0` composite, and AUD-0002's Performance coverage is
  recorded as `P 2026-08 AUD-0002` against the first two. The compiler frontend and the proposed
  `contract.module_compilation_handoff` scope remain unregistered. See the
  [triage record](./audits/AUD-0002-stage0-discovery-preparation-performance.md#triage-record).

- AUD-0002-F02 was accepted and resolved by hoisting one immutable `StringTableForkSource` above
  the directory module loop. The coordinator validation and benchmark evidence passed, and the
  required auditor pass returned clean. See the [triage record](./audits/AUD-0002-stage0-discovery-preparation-performance.md#aud-0002-f02-fork_for_module-is-called-per-module-inside-the-discovery-loop-copying-the-whole-string-table-once-per-module-against-its-own-api-guidance).

- AUD-0001-F01, AUD-0001-F02 and AUD-0001-F03 were corrected on branch `test-suite-honesty`. See each finding's triage record in [AUD-0001](./audits/AUD-0001-test-support-redundancy.md).
- AUD-0001-F04 was resolved in Phase 11 of the test-suite-honesty campaign, and its design gate turned out not to apply: the evidence recorded two `build_ast` implementations with different semantics, but there was one implementation reached through four names, so no consumer could have selected the wrong one. Corrected as a pure rename to `build_ast_with_registered_types`, `immutable_reference_expr` and `inferred_type_reference_expr`, with the bare forward deleted. See the resolution note in [AUD-0001](./audits/AUD-0001-test-support-redundancy.md#aud-0001-f04-sibling-fixture-supports-export-colliding-names-with-different-semantics).
- AUD-0001-F05 was resolved in the same phase: Phase 7 decided retire rather than adopt, and Phase 11 deleted `assert_diagnostic_reason` and `error_code_counts`. No token caller was added.
