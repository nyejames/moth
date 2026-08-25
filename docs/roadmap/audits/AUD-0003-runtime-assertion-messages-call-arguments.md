# AUD-0003: Runtime assertion messages and call-argument consolidation

- State: `complete`
- Kind: `Correctness`
- Primary scope: `feature.runtime_assertion_messages_call_arguments`
- Required context: `contract.assertion_message_runtime_handoff`, `contract.assertion_call_argument_handoff`, `tests.cases`, the assertion and shared-call test owners, `docs/compiler-design-overview.md`, `docs/build-system-design.md`, `docs/src/developer-docs/style-guide/style-guide.mtf`, and the feature implementation plan
- Coverage: `complete`
- Reviewed: `2026-08`
- Baseline: focused suites, direct integration regressions, the schema-8 test audit, documentation checks, release documentation, `just bench-check` and `just validate` were green on the corrected tree
- Revision: `1d0a68207ce0ac3b9b20ae1d25a9825ae2350424`

## Scope, context and exclusions

This final architecture and correctness audit reviewed `origin/main...1d0a68207ce0ac3b9b20ae1d25a9825ae2350424` after the external-review correction pass. It covered the static-true assertion-message finalization/discard boundary, the AST-to-HIR handoff, generated-request and downstream-fact publication, assertion-message control-flow effects, exhaustive owned-runtime-node classification, the shared call-argument parser and retained-slot route, the explicit reachable HTML-Wasm limitation, and the affected documentation and plan state.

The audit did not authorize or perform a merge or squash. It also did not change `main`.

## Coverage inventory

- `normalize_ast.rs`, `finalizer.rs`, `validate_types.rs` and `const_fact_collection.rs`: static-true messages are fully frontend-normalized and then replaced with a typed inert `none` before downstream publication.
- `assertion_message_effects.rs`: loop-control escape classification, canonical traversal ownership and exhaustive `OwnedRuntimeTemplateNode` matching.
- Shared call parsing and retained parameter-slot routing, including the dynamic generic-request activity matrix.
- HIR validation, reachability, borrow facts, backend feature validation, JavaScript assertion lowering and Wasm assertion validation/lowering.
- Direct and integration cases for static-true elision, runtime-template snapshots, HTML-Wasm reachability, and assertion-message `!`/`?` rejection.
- Canonical, teaching, architecture, build, progress, generated release documentation and the implementation plan.

## Authorities read

- `AGENTS.md`
- `docs/compiler-design-overview.md`: architectural invariants, Stage 4 AST semantics, Templates and TIR, Generated concrete functions and HIR handoff material
- `docs/build-system-design.md`: architectural invariants and Generated-function boundary
- `docs/src/developer-docs/style-guide/style-guide.mtf`, testing guidance and validation guidance
- `docs/roadmap/audit-guide.md`, the Correctness audit guide, `docs/roadmap/audit-log.md`, `docs/roadmap/open-audit-findings.md` and the relevant implementation plan
- The changed assertion implementation, shared call parser, tests and integration fixtures

## Existing findings and active plans checked

The open-findings index contains no unresolved finding for this feature. Earlier post-Phase-3 and post-rebase audit findings were recorded in the implementation plan and corrected before this final audit. The feature audit scopes were registered before the final-auditor retry.

## Findings

No required correctness, ownership, diagnostic, backend, test, documentation or compatibility-path finding remains.

## No-finding checks

- A compile-time-true runtime template is parsed and semantically validated, then discarded before the completed AST/HIR executable boundary; no TIR identity, runtime handoff, reactive fact, generated request, link fact, target fact or backend message work survives.
- Static-true, static-false and dynamic generic-request activity remains the required discard/retain/retain matrix.
- Message-local loop control remains local, while depth-zero control is guarded as an internal invariant after parser/control-flow proof showed the source shape is rejected.
- The owned runtime-handoff classifier has no wildcard fallback; expression-bearing variants retain explicit semantic handling and structural variants are explicitly non-effects.
- The shared call-argument parser and retained-slot route remain singular, with no compatibility re-export or duplicate semantic owner.
- JavaScript/HTML runtime messages remain supported and lazy; reachable runtime HTML-Wasm messages remain explicitly unsupported, while unreachable static-true messages are accepted.
- The test audit reports zero hard findings and zero duplicate-primary findings. Its 15 adversarial-only, 72 backend-only and 3 mixed-role advisories are pre-existing boundary cases outside this feature slice and are recorded in the plan.

## Limitations

The final-auditor launcher was read-only for audit metadata and artifact-writing gates, so it could not independently reserve this report, update the freshness cell or rerun write-capable validation. The coordinator supplied the clean-tree validation evidence, and this report records the final auditor's clean result and exact audited revision. No implementation changes were made after that audited revision; the remaining closeout edits are audit/plan/roadmap metadata.

## Freshness update

The `Correctness` cell for `feature.runtime_assertion_messages_call_arguments` is promoted to `C 2026-08 AUD-0003`. The two contract child cells remain `N` because this report records the complete composite audit rather than separate leaf reports.
