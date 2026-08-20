# Test-suite honesty: exposed-failure ledger

Failures that Patch A of the
[test-suite honesty and infrastructure hardening plan](./test-suite-honesty-and-infrastructure-hardening-plan.md)
exposed by strengthening a contract. Every entry is a real defect in the subsystem it names, not a
test to relax. The ledger is evidence, not an expected-failure mechanism: the runner must never be
taught to accept an entry, and no later roadmap plan starts while an entry is open.

Phase 10 clears each entry by fixing the owning subsystem, then records the validating commit.
Delete this file when the final entry closes.

## Open entries

### EF-0001: assigned top-level templates are counted as page fragments

| Field | Value |
| --- | --- |
| Case | `borrow_checker_string_memory` (backend `html`) |
| Command | `cargo run --quiet -- tests --case borrow_checker_string_memory --terse` |
| Environment | macOS 23.6.0 arm64, default features, default thread count (10 cores) |
| Exposed by | Phase 7, promoting the case from `success_contract = "acceptance_only"` to `rendered_output_exact` |
| Classification | compiler/build defect |
| Correction owner | frontend header parsing — `src/compiler_frontend/headers/top_level_classifier.rs` |
| Status | open |
| Validating commit | — |

**Intended contract.** `docs/src/docs/project-structure/entry-runtime-and-fragments.mtf` states that
only a *direct* top-level template in the entry-selected module root becomes a page fragment, and
that "Assigned or returned templates are not page fragments by themselves". A module root whose only
direct top-level template is one fragment must therefore mount exactly one runtime slot and produce
exactly one fragment insert.

**Previously masked condition.** The case was whole-case acceptance-only, so it asserted only that
compilation succeeded. Nothing observed the emitted page structure or the runtime output, so the
extra slots were invisible.

**Newly observed result.** The fixture assigns two top-level templates (`buffer ~= [:initial
content]` and its rebinding) beside one direct top-level fragment. The emitted page carries three
`<div id="moth-slot-N">` mount points while `moth_start_fn0()` returns one fragment, so the mount
loop inserts two empty strings and the captured runtime output gains two trailing empty events:

```text
first difference at byte 112
expected "borrow_checker_string_memory greeting=… buffer=updated content"
actual   "borrow_checker_string_memory greeting=… buffer=updated content\n\n"
```

**Root cause.** `top_level_classifier.rs` maps `TokenKind::TemplateHead` to
`HeaderFileItem::RuntimeTemplate` with no `at_statement_boundary` guard, unlike every other
statement-level classification in that match. A template head that opens the right-hand side of a
top-level assignment is therefore classified as a top-level runtime fragment and increments
`runtime_fragment_count`, which `render_entry_fragments` turns into a mount slot. Narrow
reproduction: a module root containing only `buffer ~= [:initial content]` emits one
`moth-slot-0` even though `moth_start_fn0()` returns an empty fragment array.

**Do not** satisfy this entry by removing the top-level template binding from the fixture or by
weakening the assertion to `rendered_output_contains`. Both hide the defect the case now reports.

### EF-0002: const-required construction prepares the same composed view twice

| Field | Value |
| --- | --- |
| Test | `compiler_frontend::ast::templates::create_template_node::create_template_node_tests::head_tests::const_required_construction_preparation_is_reused_by_folding` |
| Command | `cargo test -p moth --features benchmark_counters const_required_construction_preparation_is_reused_by_folding -- --test-threads=1` |
| Environment | macOS 23.6.0 arm64, `benchmark_counters`, reproduced identically at one thread and at default parallelism |
| Exposed by | Phase 8, giving the `counters` and `timers-counters` lanes an executing command |
| Classification | pending review: compiler defect or an expectation that names more than it can require |
| Correction owner | `src/compiler_frontend/ast/templates/create_template_node.rs` and `src/compiler_frontend/ast/templates/template_control_flow/validation.rs` |
| Status | open |
| Validating commit | — |

**Previously masked condition.** The failure was recorded in the Phase 0 baseline as a
"pre-existing failure" of the `benchmark_counters` lane, and no command ran that lane. Phase 8's
lane matrix runs it, so the failure now reports on every matrix run.

**Observed result.** `AstCounter::TirPreparationAttempts` reads 2 where the test requires 1:

```text
assertion `left == right` failed: const-required construction and folding should prepare the view once
  left: 2
 right: 1
```

**Root cause.** `Template::new_const_required` prepares the same composed view twice.
`new_nested_template` prepares it at its Stage 6 in `TemplatePreparationMode::Value`
(`create_template_node.rs:403`), and `validate_const_required_template_control_flow` then prepares
the same root, phase and context again in `TemplatePreparationMode::ConstRequired`
(`template_control_flow/validation.rs:48`). Folding is not the second walk: `fold_prepared_template`
consumes the returned preparation and never calls `prepare_tir_view`, which is the part the test's
name is about and the part that is working.

**The decision this entry carries.** Two preparation modes walking one view is either work that
should be done once — the const-required facts derived from the single `Value` preparation — or two
genuinely different walks, in which case the counter contract is 2 and the test asserts a number it
was never entitled to. Phase 9 reviews which, and only then may the assertion change.

**Do not** satisfy this entry by relaxing the assertion to `>= 1`, by deleting the counter, or by
dropping the `counters` and `timers-counters` lanes from the matrix. The first two remove the only
observation of the duplicate walk; the third puts the failure back where nothing runs it.

