# AUD-0021: Constraint selected-evidence correctness

- State: `complete`
- Kind: `Correctness`
- Area: `contract.constraints.selected_evidence` - selected calls and fast arithmetic from AST folding through final-HIR convergence, generated instances and guarantee verification
- Coverage: `partial`
- Reviewed: `2026-11`
- Baseline: `constraints` at `abc903852`, compared with the preceding `850539cc1` and the original base `d80f56343`. The implementation agent reported `just validate-full` passed (7452 library tests and 3568 integration cases). This independent audit lacked a Rust/Cargo toolchain, so it did not rerun those tests. `git diff --check 850539cc1..abc903852` was clean.

## What was inspected

- `src/compiler_frontend/ast/const_values/const_execution/{rewrite,state}.rs`, `ast/mod.rs` and `ast/module_ast/finalization/finalizer.rs`: erased-call recording at successful release folds, call ownership, catch containment and release-only rewrite boundary.
- `src/compiler_frontend/hir/{constraint_verification,failure_facts}.rs`, `hir/constraint_facts/analysis.rs`, `hir/hir_builder/metadata.rs` and `hir/hir_expression/constraint_evidence.rs`: selected and live-call ledgers, skipped-statement ranges, global generated-instance resolution, witness ordering, source-domain labels and nested boundary diagnostics.
- `src/compiler_frontend/module_compilation/generated/{constraint_convergence,convergence}.rs`: phase-B summary write-back, source-owner handoff and verification invocation.
- `src/compiler_frontend/style_directives/{source,specs,mod}.rs`, `ast/statements/body_dispatch.rs`, `ast/module_ast/emission/emitter.rs` and the changed generic template/materialisation carriers: independent guarantee spans through authored and generated functions.
- Diagnostic family and draft/boundary source-owner APIs, `source/span.rs`, relevant frontend/HIR unit tests and sampled new constraints integration fixture inputs and expected diagnostics. The changed tests exercise generated-to-generated witness hops, cross-module labels, stacked directives, release-folded fast/trap/failure calls and catch-subject handling.

## Authorities read

- `docs/compiler-design-overview.md`: compiler-owned semantic facts, generated sidecars and diagnostic identity.
- `docs/developer-docs/numeric-backend-contract/numeric-backend-contract.mtf`: selected-source evidence, convergence, debug/release parity and guarantee labels.
- `docs/docs/directives/constraints.mtf`: meaning, placement and proof policy of `$fast_math`, `$safe_math` and `$infallible`.
- `docs/developer-docs/compiler-design/compile-time-semantics/compile-time-semantics.mtf`: release folding policy.
- `docs/developer-docs/style-guide/style-guide.mtf`, relevant `docs/developer-docs/style-guide/testing.mtf` sections, `docs/roadmap/audit-guide.md` and `docs/roadmap/audit-kinds/correctness.md`.

## Existing findings and active plans checked

The prior constraints merge review's two required corrections are implemented: generated witness resolution searches all completed sidecars, and each authored guarantee carries its own diagnostic span. Neither root cause remains in the reviewed path. The original constraints plan is completed and removed from the roadmap. Numeric optimisation is the next planned item; sharing arithmetic implementations belongs there, not in this audit. Existing open audit findings have no overlapping constraints root cause.

## Findings

None demonstrated within the reviewed scope.

## Checked and clean

- Generated call targets resolve through one identity-indexed map over final HIR functions in base and completed sidecars. Provider-only bodies remain summary leaves. A generated-to-generated witness can cross modules while retaining the originating module on each call-hop and operation label.
- Primary labels come from the specific `$safe_math` or `$infallible` declaration. A statement inheriting a whole-function guarantee is not diagnosed twice. The previous blanket sidecar-label ownership pass is removed.
- Release folding captures an erased ordinary call after a successful candidate evaluation, keyed to its owning callable. Final-HIR projection resolves that target using the established call-target mapper. Static-branch filtering, skipped-handler and post-terminator ranges distinguish selected fast semantics from live trap/failure hazards. Catch containment suppresses escaping-failure evidence without suppressing trap evidence.
- A directly handled call remains unfolded, preserving its catch/failure structure. Nested calls under a catch subject may still fold, with containment retained in the erased-call record.
- The code removes both `too_many_arguments` allowances by splitting functions along existing ownership, without adding context objects. The code now has one helper for call trap/failure hazard recording and one set of authored guarantee spans. No correction requiring a new abstraction was identified.
- The test fixtures pin debug/release expected outcomes and diagnostic spans across HTML and HTML-Wasm, except the documented caught-call success case where HTML-Wasm rejects an unsupported external `io.line` call. These assertions were inspected but not executed here.

## Limitations

- This is a partial audit of selected-evidence correctness. It is not an exhaustive review of all AST constant evaluation, generic materialisation, source diagnostics, backends, performance or integration fixtures.
- `cargo`, `rustc` and the compiler's `just validate-full` gate were unavailable in this environment. Passing test counts are the implementation agent's handover, not an independent execution result. The branch's exact diff passed `git diff --check`.
- A release-folded call with no source span is conservatively classified as live because the skipped-source matcher has no position to compare. This can theoretically reject a `$infallible` guarantee in release that debug accepts, contrary to the documented parity goal. No reachable source-level reproduction was established. It is recorded in `docs/bug-reports.md` for a targeted proof or correction, not asserted as a reproduced regression.
- Two compiler crashes reported by the implementation agent also reproduce on original base `d80f56343` without constraints: an unreachable generic call after return and the mutable-local `~` effect combination. They are outside this selected-evidence audit and are tracked separately in `docs/bug-reports.md`.
