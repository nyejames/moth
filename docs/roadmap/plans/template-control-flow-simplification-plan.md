# Template control-flow simplification implementation plan

> **For agentic workers:** Carry this out as a deletion-first compiler refactor. Use the repository's current planning, testing, validation and code-review guidance. Prefer the smallest complete representation after each removal. Do not preserve compatibility shims for deleted template syntax.
>
> **Activation:** Execute this plan at the next stable compiler checkpoint. Do not pin this plan to the repository state that existed when it was written. At activation, establish the actual baseline in working notes, refresh every path and owner listed below and adapt the mechanical steps to the delivered representation without restoring superseded code.
>
> **Intended repository location while active:** `docs/roadmap/plans/template-control-flow-simplification-plan.md`
>
> **Plan lifecycle:** If this plan is committed while active, add it to the top of the appropriate sequenced-work position in `docs/roadmap/roadmap.md`. Remove the plan and roadmap bullet in the completion commit after all durable design decisions have been moved into canonical documentation.

## Status

```text
STATUS: active, language simplification approved
CURRENT_SLICE: Phase 4 accepted; Phase 5 next
BLOCKERS: none; committed package fixes synchronized both ways at the parser checkpoint
NEXT_ACTION: replace runtime branch chains and finish HIR emission ownership
```

## Goal

Reduce template control flow to a deliberately small surface:

- a single-arm template `if`
- option-present capture in that single-arm `if`
- template `loop` using the ordinary loop-header families already supported by the compiler
- no template `else`
- no template `else if`
- no template `break`
- no template `continue`

The implementation must remove the old feature machinery completely rather than leave dormant branch-chain, fallback, sentinel, loop-control-signal or compatibility infrastructure behind.

The resulting compiler should represent template conditional inclusion directly, represent repetition directly and use ordinary Moth code for richer decision-making. The change should leave the template pipeline smaller across parsing, TIR, folding, runtime handoff and HIR lowering.

## Accepted language contract

### Retained template `if`

A template `if` has exactly one selected body:

```moth
[if visible:
    Visible
]
```

The condition may be:

- an ordinary `Bool` condition
- the existing option-present capture form

Example:

```moth
[if maybe_name is |name|:
    Hello [name]
]
```

A false condition produces structural no-output.

There is no template fallback arm.

### Retained template `loop`

Retain the ordinary template loop header families already supported at activation. At the planning checkpoint these are:

- conditional loop headers
- collection loop headers
- range loop headers

Examples:

```moth
[loop items |item, index|:
    [index]: [item]
]

[loop 0 to 10 |value|:
    [value]
]
```

A template loop has no template-local `break` or `continue`.

This plan does not redesign ordinary Moth loop syntax. If the ordinary loop header surface changes before activation, templates should continue to reuse the then-current accepted ordinary loop header owner rather than restoring an older shape to match this plan.

### Structural no-output remains semantic

Preserve the existing distinction between structural no-output and an emitted empty string.

At minimum:

- a false single-arm template `if` produces structural no-output
- a false conditional template loop produces structural no-output
- a zero-iteration collection or range template loop produces structural no-output
- output-conditioned wrappers remain suppressed when their child structurally emits nothing
- loop aggregate wrappers remain suppressed when the loop emits nothing

Do not simplify this contract away while deleting the old control-flow variants.

### Lazy runtime semantics remain

A runtime template conditional must evaluate its body only when selected.

A runtime loop must evaluate each body iteration lazily through ordinary HIR control flow.

Option-present capture must retain its existing capture scope and lazy selected-body behaviour.

### Const-required semantics remain

Const-required templates still validate the retained conditional and loop structure according to the canonical const-template contract.

Do not add arbitrary source-function interpretation to replace removed template fallback syntax. A `.mtf` or other const-required template cannot assume that an ordinary runtime helper function is compile-time callable.

### Rich control flow belongs in code

The documentation should explicitly establish the design boundary:

- template `if` is conditional inclusion
- template `loop` is repetition
- richer branching and decision-making belong in ordinary Moth code
- runtime templates may insert a function result where that is appropriate
- const-required templates must use the compile-time mechanisms actually supported by Moth rather than acquiring a new evaluator as compensation for removing `else`

## Removed surface

Remove all support for these template-body markers:

```moth
[else]
[else if condition]
[break]
[continue]
```

Remove the internal concepts that exist only to support them, including:

- template `else` sentinels
- template `else if` sentinels
- template loop-control sentinels
- branch-chain fallback bodies
- branch-chain vectors used only for `else if`
- authored `else` marker metadata
- template loop-control node kinds
- template loop-control helper kinds
- loop-control signal propagation through constant folding
- loop-control flush state in runtime slot lowering
- old orphan, duplicate, malformed, inline and loop-body template-control diagnostics
- tests whose only purpose is the removed behaviour

Discontinued syntax must not acquire a compatibility parser or a long-lived migration diagnostic subsystem. It should fail through the normal grammar and diagnostic path of the surviving language.

## Non-goals

This plan does not:

- remove or change ordinary source-language `else`
- remove or change ordinary source-language `break` or `continue`
- redesign ordinary statement or value-producing `if`
- add template match arms
- add a ternary operator
- add a new template expression language
- add arbitrary compile-time source-function execution
- redesign slot semantics
- redesign template wrappers
- redesign Markdown formatting
- redesign ordinary HIR loop lowering
- redesign the borrow checker
- redesign a backend
- implement collection-producing loops or repeated option-capture loops
- retain compatibility aliases or adapters for the removed template surface

## Architectural target

The target representation should be direct.

Conceptually, TIR should have a single conditional shape rather than a branch chain:

```text
Conditional {
    selector
    selector expression site
    body
}
```

The exact Rust spelling may differ if the stable checkpoint has already refactored TIR, but these properties are required:

- one selector
- one body
- no branch vector
- no fallback
- no `else` marker
- no loop-control node

The neutral AST-to-HIR runtime handoff should mirror the same single conditional rather than manufacture a branch-chain vocabulary.

Constant folding should return only normal template output state. It must not thread a possible `break` or `continue` signal through every recursive fold.

Runtime HIR template lowering should use ordinary `if` and loop CFG helpers for the retained features, with no template-specific loop-control jump or runtime-slot flush path.

---

# Phase 0 - Activate on the stable checkpoint and refresh the owner map

## Purpose

The codebase is undergoing adjacent expression, TIR and compiler-foundation work. Current paths are navigation starting points, not an instruction to recreate an older representation.

## Required authority read

Before editing code, read the current versions of:

- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/src/developer-docs/language/overview.mtf`
- `docs/src/docs/templates/template-control-flow.mtf`
- `docs/src/docs/templates/template-control-flow-basic.mtf`
- `docs/src/docs/constants/const-templates.mtf`
- `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf`
- `docs/src/docs/progress/@page.moth`
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`
- `docs/src/developer-docs/memory-management/overview.mtf`
- `docs/roadmap/roadmap.md`
- `.agents/skills/moth-code-sniffer/SKILL.md` (the supplied review guide, as confirmed by the user)

Read any routed template, HIR, testing or memory references those documents require for the changed owners.

## Establish the real baseline

Record in working notes, not in this plan file:

```bash
git status --short
git rev-parse HEAD
git branch --show-current
```

Start from the requested stable checkpoint on a dedicated branch or worktree.

Run the stable checkpoint's appropriate baseline gate. Unless current repository guidance has changed, use:

```bash
just validate
cargo run --quiet -- tests --list --tag templates
just bench-check
```

The benchmark run is non-recording evidence. Record the template-heavy surviving cases, especially the equivalents of:

- `benchmarks/template-stress.moth`
- `benchmarks/adversarial/template-render-plan-churn.moth`

Do not reset a benchmark baseline after the removal to hide regressions.

## Refresh the implementation inventory

Run repository-wide searches before editing:

```bash
rg -n \
  'BranchChain|TemplateIrBranch|OwnedRuntimeTemplateBranch|TemplateElseMarker|ElseSentinelPolicy|DirectElseMarker|TemplateBodyControlContext|TemplateLoopControlKind|LoopControlSignal|RuntimeSlotLoopControlFlush|TemplateEmission::Break|TemplateEmission::Continue|TemplateBodyEmission::Break|TemplateBodyEmission::Continue' \
  src docs tests benchmarks index.md

rg -n '\[else|\[break\]|\[continue\]' docs/src tests/cases benchmarks
```

Also search for the rendered diagnostic reason names and messages discovered from the actual `InvalidTemplateStructureReason` owner.

Classify every hit as:

- remove
- adapt to single conditional
- preserve because it belongs to ordinary Moth control flow
- historical/generated and regenerated from an owning source
- already removed by prerequisite work

Do not edit based only on the owner map below.

## Current navigation owner map to reverify

At plan-writing time the main owners included:

### Parser and template control metadata

- `src/compiler_frontend/ast/templates/mod.rs`
- `src/compiler_frontend/ast/templates/create_template_node.rs`
- `src/compiler_frontend/ast/templates/template_body_parser.rs`
- `src/compiler_frontend/ast/templates/template_body_sentinels.rs`
- `src/compiler_frontend/ast/templates/template_head_parser/head_parser.rs`
- `src/compiler_frontend/ast/templates/template_head_parser/control_flow_suffix.rs`
- `src/compiler_frontend/ast/templates/template_control_flow/mod.rs`
- `src/compiler_frontend/ast/templates/template_control_flow/types.rs`

### TIR construction and consumers

- `src/compiler_frontend/ast/templates/tir/node.rs`
- `src/compiler_frontend/ast/templates/tir/store/control_flow.rs`
- `src/compiler_frontend/ast/templates/tir/construction_context.rs`
- `src/compiler_frontend/ast/templates/template_render_units.rs`
- `src/compiler_frontend/ast/templates/template_control_flow/validation.rs`
- `src/compiler_frontend/ast/templates/tir/summary.rs`
- `src/compiler_frontend/ast/templates/tir/subtree_copy.rs`
- `src/compiler_frontend/ast/templates/tir/formatter_view.rs`
- `src/compiler_frontend/ast/templates/tir/expression_sites.rs`
- `src/compiler_frontend/ast/templates/tir/expression_constness.rs`
- `src/compiler_frontend/ast/templates/tir/expression_overlays.rs`
- `src/compiler_frontend/ast/templates/tir/wrapper_sets.rs`
- `src/compiler_frontend/ast/templates/tir/slot_plan.rs`
- `src/compiler_frontend/ast/templates/tir/slot_layout.rs`
- `src/compiler_frontend/ast/templates/tir/slot_composition/schema.rs`
- `src/compiler_frontend/ast/templates/template_slots/runtime_plan/sites.rs`

### Preparation and folding

- `src/compiler_frontend/ast/templates/template.rs`
- `src/compiler_frontend/ast/templates/template_folding.rs`
- `src/compiler_frontend/ast/templates/tir/preparation.rs`
- `src/compiler_frontend/ast/templates/tir/fold/reducer.rs`
- `src/compiler_frontend/ast/templates/tir/fold/control_flow.rs`
- `src/compiler_frontend/ast/templates/tir/fold/wrappers.rs`
- `src/compiler_frontend/ast/templates/tir/fold/estimate.rs`
- `src/compiler_frontend/ast/templates/doc_fragments.rs`
- `src/compiler_frontend/ast/expressions/expression.rs`
- `src/compiler_frontend/ast/expressions/parse_expression_templates.rs`
- `src/compiler_frontend/ast/module_ast/environment/constant_resolution.rs`
- `src/compiler_frontend/ast/module_ast/environment/config_resolution.rs`
- `src/compiler_frontend/ast/module_ast/emission/emitter.rs`
- `src/compiler_frontend/ast/module_ast/finalization/template_helpers.rs`
- `src/compiler_frontend/ast/module_ast/finalization/public_const_templates.rs`

### Runtime handoff and HIR lowering

- `src/compiler_frontend/ast/templates/runtime_handoff.rs`
- `src/compiler_frontend/ast/templates/tir/handoff_materialization.rs`
- `src/compiler_frontend/hir/hir_expression/templates/mod.rs`
- `src/compiler_frontend/hir/hir_expression/templates/control_flow.rs`
- `src/compiler_frontend/hir/hir_expression/templates/render_append.rs`
- `src/compiler_frontend/hir/hir_expression/templates/append_context.rs`
- `src/compiler_frontend/hir/hir_expression/templates/slot_application.rs`

### AST finalization and semantic walkers

Reverify all `OwnedRuntimeTemplateNode` walkers, including the current equivalents of:

- `src/compiler_frontend/ast/expressions/failure_classification.rs`
- `src/compiler_frontend/ast/module_ast/finalization/validate_types.rs`
- `src/compiler_frontend/ast/module_ast/finalization/debug_type_validation.rs`
- `src/compiler_frontend/ast/module_ast/finalization/normalize_ast.rs`

If Reactivity V1 or another prerequisite has removed or replaced any of these paths, use the delivered owner and do not restore the old path.

### Diagnostics

- `src/compiler_frontend/compiler_messages/diagnostic_payload/types.rs`
- `src/compiler_frontend/compiler_messages/diagnostic_payload/reason_keys.rs`
- `src/compiler_frontend/compiler_messages/render/templates.rs`

## Exit check

- [x] The real checkpoint is recorded in working notes.
- [x] Current authorities have been read.
- [x] Every old feature identifier has an owner and disposition.
- [x] Current tests using removed syntax have been inventoried.
- [x] Current docs and future plans referring to removed syntax have been inventoried.
- [x] The stable checkpoint passes its baseline gate before this work changes it.

Commit only if plan activation itself requires a roadmap/status commit. Otherwise begin the implementation slices on the feature branch.

---

# Phase 1 - Establish the new test contract before deleting implementation

## Purpose

Protect the retained surface and make the desired removals observable before restructuring internals.

Follow `testing.mtf`: user-visible language behaviour belongs primarily in integration cases. Keep focused unit tests only for TIR or parser invariants that integration output cannot expose.

## Add or rewrite the primary retained-behaviour cases

Ensure there is one clear primary integration owner for each contract below.

### Single-arm Boolean `if`

Cover:

- runtime true selects the body
- runtime false emits no body
- compile-time true folds to the body
- compile-time false folds to structural no-output
- a false condition suppresses an output-conditioned wrapper
- selected body work remains lazy at runtime

Prefer adapting existing cases when they already own the contract.

### Option-present capture

Cover:

- present option selects the body and binds the payload
- absent option emits structural no-output
- capture is visible only inside the selected body
- runtime lowering remains lazy
- const-required behaviour stays within the currently accepted contract

Do not retain `else` only to test absence.

### Basic loops without loop control

Retain focused primary cases for:

- collection loop
- range loop
- conditional loop if that header remains part of the accepted loop surface
- zero-iteration structural no-output
- aggregate wrapper applies once when output exists
- aggregate wrapper is skipped when output does not exist
- runtime slot application inside a loop
- const range/collection folding
- const-loop expansion limit
- const conditional-loop rejection/acceptance rules that remain part of the canonical contract

## Add focused rejection coverage for removed body-control syntax

The compiler currently recognises the removed markers specially. The desired implementation should no longer do that.

Add the minimum integration coverage needed to prove that the surviving template grammar rejects:

```moth
[else]
[else if condition]
[break]
[continue]
```

Do not preserve the old orphan/duplicate/malformed/inline reason taxonomy in the new expected output.

Prefer the normal parser diagnostic identity produced by the surviving grammar. If a dedicated current-grammar diagnostic is truly needed to avoid an unrelated or misleading parse, it must describe the present grammar rather than act as a compatibility diagnosis for deleted syntax.

`else if` may share the same primary rejection owner as `else` if both fail at the same first token and a focused parser unit test is enough to protect the longer spelling.

## Current integration fixtures to classify

At plan-writing time, searches found these case families. Reinventory them at activation.

### Cases containing template `else`

Examples include:

- `const_template_else_if_branch_chain`
- `const_template_if_imported_source_const_bool`
- `const_template_if_source_const_bool`
- `html_wasm_runtime_template`
- `template_const_if_inactive_runtime_branch_rejected`
- `template_control_flow_literal_body_else_rejected`
- `template_control_flow_markdown_branch_isolation`
- `template_control_flow_runtime_laziness`
- `template_else_body_orphan`
- `template_else_duplicate`
- `template_else_if_after_else`
- `template_else_if_option_present_capture`
- `template_else_if_runtime_chain`
- `template_else_inline_not_standalone`
- `template_else_inside_loop_without_nested_if`
- `template_else_orphan`
- `template_if_bool_compile_time_false_else`
- `template_if_bool_compile_time_true`
- `template_if_bool_runtime`
- `template_if_nested_else_binding`
- `template_if_option_capture_not_visible_in_else`
- `template_if_option_present_capture`
- `template_runtime_branch_fallback_wrapped_output`
- `template_runtime_branch_slot_application`
- `template_runtime_slot_in_branch_and_loop`
- `template_runtime_wrapper_slot_inside_fallback`

Delete cases whose only contract is removed syntax.

Rewrite mixed-purpose cases around a single-arm conditional only when they protect a distinct surviving contract such as lazy evaluation, slot routing, wrapper suppression, Markdown branch isolation or source-constant folding.

### Cases containing template `break`

Examples include:

- `template_loop_break_before_output_runtime`
- `template_loop_break_outside_rejected`
- `template_loop_const_break`
- `template_loop_literal_body_break_rejected`
- `template_loop_nested_if_break_runtime`
- `template_loop_output_then_break_runtime`
- `template_runtime_loop_aggregate_wrapper_regression`
- `template_runtime_slot_break_inside_loop`
- `template_runtime_slot_loop_control_outside_loop_rejected`

Delete pure loop-control cases.

Rewrite a mixed aggregate-wrapper regression only if it owns a distinct retained wrapper or loop contract that is not already covered elsewhere.

### Cases containing template `continue`

Examples include:

- `template_loop_const_continue`
- `template_loop_continue_before_output_runtime`
- `template_loop_continue_outside_rejected`
- `template_loop_nearest_nested_runtime`
- `template_loop_nested_if_continue_runtime`
- `template_loop_output_then_continue_runtime`
- `template_runtime_loop_aggregate_wrapper_regression`
- `template_runtime_slot_continue_inside_loop`
- `template_runtime_slot_in_branch_and_loop`
- `template_runtime_slot_output_then_continue_inside_loop`

Apply the same delete-or-rewrite rule.

If prerequisite work has already removed some of these fixtures, do not recreate them.

## Unit and internal test owners to update

Reinventory template tests that assert the old shapes. Current likely owners include:

- `src/compiler_frontend/ast/templates/tests/create_template_node/control_flow_tests.rs`
- `src/compiler_frontend/ast/templates/tests/create_template_node/handler_runtime_tests.rs`
- `src/compiler_frontend/ast/templates/tests/create_template_node/head_tests.rs`
- `src/compiler_frontend/ast/templates/tests/create_template_node/template_head_tests.rs`
- `src/compiler_frontend/ast/templates/tests/create_template_node/parser_tir_tests.rs`
- `src/compiler_frontend/ast/templates/tests/create_template_node/parser_tir_malformed_tests.rs`
- `src/compiler_frontend/ast/templates/tir/tests/construction_tests.rs`
- `src/compiler_frontend/ast/templates/tir/tests/builder.rs`
- `src/compiler_frontend/ast/templates/tir/tests/builder_tests.rs`
- `src/compiler_frontend/ast/templates/tir/tests/store_tests.rs`
- `src/compiler_frontend/ast/templates/tir/tests/subtree_copy_tests.rs`
- `src/compiler_frontend/ast/templates/tir/tests/view_tests.rs`
- `src/compiler_frontend/ast/templates/tir/tests/preparation_tests.rs`
- `src/compiler_frontend/ast/templates/tir/tests/fold_view_tests.rs`
- `src/compiler_frontend/ast/templates/tir/tests/fold_final_view_tests.rs`
- `src/compiler_frontend/ast/templates/tir/tests/hir_handoff_tests.rs`
- `src/compiler_frontend/ast/templates/tir/tests/expression_traversal_tests.rs`
- `src/compiler_frontend/ast/templates/tir/tests/slot_composition_tests.rs`
- `src/compiler_frontend/ast/templates/tir/tests/wrapper_context_construction_tests.rs`
- `src/compiler_frontend/ast/templates/tir/tests/wrapper_context_fold_tests.rs`
- `src/compiler_frontend/ast/templates/template_slots/runtime_plan/tests/sites_tests.rs`
- `src/compiler_frontend/hir/tests/hir_expression_lowering_tests.rs`
- relevant AST finalization tests

Delete unit tests that merely preserve the old branch-chain or signal representation.

Keep or rewrite focused internal tests for invariants that remain important:

- one conditional has one selector site
- expression overlays reach the conditional selector and body correctly
- subtree copy preserves the conditional selector/body
- slot expansion and wrapper-context traversal visit the conditional body
- runtime handoff preserves the selector/body without TIR identities
- structural no-output remains correct
- loop aggregation remains correct

## Manifest maintenance

For removed or renamed integration cases:

- update `tests/cases/manifest.toml`
- preserve one primary owner per surviving contract
- remove obsolete contract IDs
- do not leave an unreferenced fixture directory
- run the suite inventory audit after the case set stabilises

## Focused pre-implementation check

Run the new rejection tests and confirm they fail for the expected reason: the current compiler still accepts or specially handles syntax that this plan removes.

Run retained cases and confirm they pass before code changes.

Commit the test-contract slice separately if useful:

```text
test: define simplified template control flow surface
```

---

# Phase 2 - Remove body sentinels and simplify parser state

## Purpose

The body parser should parse body content and nested templates. It should no longer implement a second control-flow marker protocol.

## Delete the sentinel subsystem

If still present, delete:

```text
src/compiler_frontend/ast/templates/template_body_sentinels.rs
```

Remove its module declaration from `src/compiler_frontend/ast/templates/mod.rs`.

Delete all types and helpers that exist only for direct body markers, including the current equivalents of:

- `ElseSentinelPolicy`
- `LoopControlSentinelPolicy`
- `TemplateBodyControlContext`
- `DirectElseMarker`
- `DirectLoopControlMarker`
- `BodySentinelTarget`
- `TemplateBodyBoundary::Else`
- `TemplateBodyBoundary::ElseIf`
- sentinel boundary and whitespace helpers
- orphan/duplicate/inline marker helpers
- active template-loop depth tracking

If `TemplateBodyBoundary` has no remaining meaning after this deletion, delete the type entirely and make body parsing return ordinary success when the closing boundary is consumed.

Do not keep a one-variant boundary enum.

## Simplify nested-template parsing options

Remove `control_context` or its stable-checkpoint equivalent from:

- `NestedTemplateParseOptions`
- `TemplateBodyParseRequest`
- recursive nested-template construction
- `create_template_node.rs`

A nested template should no longer inherit body-control sentinel state because there is no such state.

## Simplify the hot body loop

In `template_body_parser.rs` or its delivered replacement:

- remove `classify_direct_else_marker`
- remove `handle_direct_else_marker`
- remove `classify_direct_loop_control_marker`
- remove `handle_loop_control_marker`
- remove any special branch before normal nested-template parsing that exists only for these markers

A `TEMPLATE_HEAD` token in a normal template body should either:

- be consumed as literal balanced text when the owning formatter deliberately suppresses child templates
- enter ordinary nested-template parsing

There should be no third sentinel path.

Preserve literal-body semantics. If child-template-suppressed bodies intentionally treat bracketed source as literal text, keep that formatter contract rather than forcing removed keywords through a child parser that is disabled by design.

## Simplify single-arm `if` parsing

Change the current branch-chain body parser to a one-body parser.

The target flow should be equivalent to:

```text
take selector and then-context from parsed head
-> create body TIR construction context
-> parse body to the template close
-> finalize body root
-> record one conditional control-flow node
```

Delete:

- branch accumulation vectors
- selector/context mutation loops
- `branch_starts_after_else_if`
- fallback state
- fallback marker state
- `parse_fallback_branch`
- `parse_else_if_branch_header`
- `ParsedElseIfBranch`
- `ParsedFallbackBranch`
- `branch_selector_and_context_from_parsed_if_header` if it becomes unused
- else-specific whitespace trimming

The initial template-head parser already owns parsing of the first `if` header. Do not retain a second path for parsing another `if` after `else`.

## Simplify `TemplateIfBodyParseInput`

Keep only the facts needed for one conditional:

- selector
- selected-body scope/context
- source span/provenance needed downstream

Delete `else_context`.

In `template_head_parser/control_flow_suffix.rs`, stop allocating an unused else branch scope.

Option-present capture still needs its selected-body scope. Preserve that scope exactly.

## Simplify loop body parsing

Remove loop-control context entry and active-loop-depth propagation.

A template loop body should parse exactly like a normal structured body under its loop binding scope, then record one loop body node.

Do not touch ordinary source-loop control parsing.

## Remove special `else` template-head parsing if obsolete

At plan-writing time `template_head_parser/head_parser.rs` had a dedicated `TokenTag::ELSE` branch producing `ElseInTemplateHead`.

After `else` has no template meaning, prefer deleting that special case and allowing the normal current-grammar unexpected-token path to reject it.

Apply the same principle to `break` and `continue`: do not add new template-specific head branches merely to preserve old diagnostics.

Keep a special branch only if the normal parser would otherwise accept the token as a different valid construct or produce a materially incorrect diagnostic. Document that current-grammar reason locally if such a special case remains.

## Focused validation

Run:

```bash
cargo fmt --all
cargo test -p moth --lib template -- --format terse
cargo run --quiet -- tests --case <single-arm-if-primary-case>
cargo run --quiet -- tests --case <option-capture-primary-case>
cargo run --quiet -- tests --case <basic-loop-primary-case>
cargo run --quiet -- tests --case <removed-else-case>
cargo run --quiet -- tests --case <removed-break-case>
cargo run --quiet -- tests --case <removed-continue-case>
```

Use the actual case IDs selected in Phase 1.

## Exit check

Removing the sentinel producer made loop-control TIR and runtime nodes dead under the warning policy. Their nodes, helper classification, trim paths and runtime-slot flush machinery were retired in this checkpoint. The one remaining helper outcome is direct `SlotInsertHelper`. Branch-chain representation, fold-signal deletion and runtime-emission type ownership remain assigned to Phases 3, 4 and 5.

- [x] No body sentinel module remains.
- [x] No nested-template control context remains.
- [x] No branch-chain parsing loop remains.
- [x] No fallback parser remains.
- [x] No template loop-depth sentinel state remains.
- [x] Single-arm Boolean `if` parses.
- [x] Option-present capture parses with correct scope.
- [x] Existing loop header families parse.
- [x] Removed markers fail through the surviving grammar.

Commit:

```text
refactor: simplify template control flow parsing
```

---

# Phase 3 - Replace branch-chain and loop-control TIR with direct conditional TIR

## Purpose

The internal representation must reflect the smaller language. Keeping a one-element `BranchChain` would retain the wrong abstraction and unnecessary allocation/traversal logic.

## Replace `BranchChain`

In `tir/node.rs` or its delivered owner, replace the branch-chain node with a direct conditional node.

Preferred conceptual shape:

```rust
Conditional {
    selector: TemplateBranchSelector,
    selector_site_id: ExpressionSiteId,
    body: TemplateIrNodeId,
}
```

Keep the source span on the enclosing node unless the delivered representation has a better established provenance owner.

Delete:

- `TemplateIrBranch`
- `Vec<TemplateIrBranch>` storage
- fallback body storage
- `TemplateElseMarker`
- `else_marker`
- branch indexes used only to address `else if` arms

Do not replace one vector with another side table unless an independently measured requirement needs it. One conditional has one selector and one body.

## Delete TIR `LoopControl`

Remove the loop-control node variant and `TemplateLoopControlKind` if no non-template owner uses it.

This removal is template-specific. Ordinary AST/HIR loop-control kinds for source statements remain untouched.

## Simplify construction APIs

Replace the current equivalent of:

```text
record_branch_chain(branches, fallback, else_marker, span)
```

with a direct API for one conditional.

Prefer letting the TIR construction owner allocate the selector expression-site ID if this removes parser knowledge without duplicating another site-allocation owner.

Do not move expression-site allocation if other parser nodes have a consistent established convention. Reverify `next_expression_site_id` uses at activation and preserve one owner.

Delete `record_loop_control`.

## Simplify control-flow body replacement

In `tir/store/control_flow.rs` or its delivered owner:

- replace `Branch { index }` and `Fallback` body-addressing state with one `ConditionalBody`
- keep `LoopBody`
- remove branch index validation
- remove fallback replacement
- rename internal diagnostics and comments from branch chain to conditional

If a two-variant `ControlFlowBodyKind` adds no value after the change, consider replacing it with two clear direct methods. Follow the style guide: keep an enum only when it explains meaningful state.

## Simplify render-unit preparation

In `template_render_units.rs`:

- stop collecting a vector of branch bodies
- stop extracting a fallback
- delete `prepare_branch_chain_render_units`
- prepare one conditional body
- keep loop body preparation and aggregate wrapper handling

Review whether the conditional-body and loop-body preparation helpers can share local code. Do not force an abstraction if their wrapper semantics differ.

## Update every structural TIR consumer

Use exhaustive compiler errors to find all matches, then perform an explicit source search after compilation succeeds.

Current owners to reverify include:

- `template_control_flow/validation.rs`
- `tir/wrapper_sets.rs`
- `tir/slot_plan.rs`
- `tir/slot_layout.rs`
- `tir/slot_composition/schema.rs`
- `tir/subtree_copy.rs`
- `tir/formatter_view.rs`
- `tir/expression_sites.rs`
- `tir/expression_constness.rs`
- `tir/expression_overlays.rs`
- `tir/summary.rs`
- `tir/preparation.rs`
- `tir/fold/*`
- `tir/handoff_materialization.rs`
- `template_slots/runtime_plan/sites.rs`

For each owner:

- replace branch-vector traversal with one selector/body
- remove fallback traversal
- remove else-marker copying
- remove loop-control leaf handling
- preserve exact-view and overlay rules
- preserve selector expression-site remapping
- preserve slot/wrapper traversal through the conditional body
- preserve cycle detection identity

### Specific invariants

Expression traversal must visit:

1. the effective conditional selector expression
2. the conditional body

Expression overlays must continue to address the selector through its stable `ExpressionSiteId`.

Subtree copy must allocate/remap the selector site according to the same site-identity rules as before.

Formatter traversal must format child templates inside the conditional body without inventing a fallback path.

Slot layout, slot-plan conversion and runtime-site injection must descend into the one conditional body.

Wrapper-context collection must descend into the conditional body.

Summary metadata must still classify `Conditional` and `Loop` as control flow.

## Focused validation

Run the affected TIR tests and integration cases.

At minimum:

```bash
cargo fmt --all
cargo test -p moth --lib tir -- --format terse
cargo test -p moth --lib template -- --format terse
cargo run --quiet -- tests --case <single-arm-if-primary-case>
cargo run --quiet -- tests --case <runtime-slot-conditional-case>
```

## Exit search

```bash
rg -n \
  'BranchChain|TemplateIrBranch|TemplateElseMarker|else_marker|record_branch_chain|record_loop_control|TemplateIrNodeKind::LoopControl|TemplateLoopControlKind' \
  src/compiler_frontend/ast/templates
```

Every result must be removed or explicitly justified as unrelated text. Do not leave stale comments.

Phase 3 removed TIR branch vectors, fallback storage and marker metadata. At that checkpoint runtime `BranchChain` remained assigned to the Phase 5 handoff owner and its tests, while `TemplateLoopControlKind` remained assigned to Phase 4's fold-signal channel. There was no TIR loop-control node or producer.

- [x] One selector, stable expression site and body per conditional.
- [x] Construction owns selector-site allocation and enclosing-node spans.
- [x] Render-unit preparation retains conditional head and loop aggregate semantics.
- [x] Structural consumers retain exact views, site remapping, slots and wrappers.
- [x] Preparation and folding preserve const validation, capture scope and provenance.
- [x] Focused tests and template integration checks pass.
- [x] Independent construction, structural-consumer and folding audits accepted.

Commit:

```text
refactor: use direct conditional template TIR
```

---

# Phase 4 - Delete loop-control signal propagation from preparation and constant folding

## Purpose

This is one of the main codebase simplifications unlocked by the language removal.

Today a possible template `break` or `continue` can travel through template preparation, recursive folding, wrapper folding and finalization boundaries. Once template loop control is impossible, the signal channel should cease to exist.

## Remove loop-control template classification

In the current `template.rs` and preparation owners:

- remove `TemplateConstValueKind::LoopControlSignal`
- remove `TemplateHelperKind::LoopControl`
- remove top-level detection of a loop-control TIR node
- remove any `Helper(LoopControl)` result
- remove internal errors for an unconsumed or escaped loop-control signal

Update ordinary expression const-kind mapping so it no longer contains a loop-control-template special case.

## Collapse helper classification if it has one remaining meaning

At plan-writing time `TemplateHelperKind` had only:

- `LoopControl`
- `SlotInsert`

After deleting loop control, a one-variant helper-kind enum is likely redundant.

At activation, reverify all variants and consumers.

If `SlotInsert` is still the only helper kind:

- delete `TemplateHelperKind`
- replace `TemplatePreparationOutcome::Helper(TemplateHelperKind)` with a direct outcome such as `SlotInsertHelper`
- replace `FinalizedTemplateValue::Helper(TemplateHelperKind)` with the narrowest direct representation
- simplify matches in finalization, emission and public const-template projection

Do not collapse it if prerequisite work has introduced another semantically real helper category.

This is a required investigation, not an optional style tweak.

## Collapse `TemplateEmission`

Change constant-fold emission from:

```text
NoOutput
Output(value)
Break(optional_output)
Continue(optional_output)
```

to only:

```text
NoOutput
Output(value)
```

Delete helpers that exist to combine string output with a control signal.

Current examples to remove or simplify include the equivalent of:

- `template_emission_from_output_and_signal`
- `signal_kind`
- `Option<TemplateLoopControlKind>` traversal results

## Simplify the TIR fold reducer

Change recursive fold traversal so nodes no longer return a possible loop-control signal.

Preferred conceptual flow:

```text
fold node into output state -> Result<(), TemplateError>
```

rather than:

```text
fold node into output state -> Result<Option<TemplateLoopControlKind>, TemplateError>
```

Sequence traversal should visit all children in order. There is no template-control early return.

Delete:

- early sequence exit for `Break`/`Continue`
- loop-control-node reducer branches
- helper folding for loop-control nodes
- signal arguments to emission construction
- signal propagation through nested child templates
- impossible internal errors for a signal escaping an aggregate wrapper

Preserve ordinary compiler error propagation and ordinary source-level terminality. This change removes only the template-local structural signal.

## Simplify conditional folding

Rename the current branch-chain folding path to a singular conditional path.

The retained logic should be:

1. obtain the effective selector from its expression site
2. evaluate the Boolean or option-present selector
3. merge selector provenance
4. fold the body only when selected
5. return structural no-output when unselected

For option capture, push the captured payload binding only while folding the selected body, then restore the previous binding stack.

There is no next branch and no fallback.

## Simplify loop folding

Range and collection loops should fold every iteration until normal source iteration completion or the const-loop limit.

Delete:

- `iteration_signal`
- `Break` matching
- `Continue` matching
- loop-iteration signal return values

The loop-iteration helper should only:

- install iteration bindings
- fold the body
- restore bindings
- merge provenance
- append output if present

Conditional const-loop termination rules remain unchanged.

## Simplify wrapper folding

Remove signal-aware wrapper logic.

Review `tir/fold/wrappers.rs` for:

- signal return values
- break/continue preservation through conditional wrappers
- aggregate-wrapper signal errors
- signal/output merge helpers

After the change wrappers need only distinguish structural no-output from output.

## Simplify all const-template consumers

Remove impossible `Break`/`Continue` match arms and their internal-error paths from the current equivalents of:

- `ast/templates/doc_fragments.rs`
- `ast/expressions/parse_expression_templates.rs`
- `ast/module_ast/environment/constant_resolution.rs`
- `ast/module_ast/environment/config_resolution.rs`
- `ast/module_ast/emission/emitter.rs`
- `ast/module_ast/finalization/template_helpers.rs`
- `ast/module_ast/finalization/public_const_templates.rs`

A complete fold now has only the two legitimate output states.

Do not replace removed branches with wildcard arms that hide a future invalid state. Keep matches exhaustive over the smaller enum.

## Focused validation

```bash
cargo fmt --all
cargo test -p moth --lib template -- --format terse
cargo test -p moth --lib const -- --format terse
cargo run --quiet -- tests --case <const-single-arm-if-case>
cargo run --quiet -- tests --case <const-loop-case>
cargo run --quiet -- tests --case <zero-output-wrapper-case>
```

Use the actual current test filters and case IDs.

## Exit search

```bash
rg -n \
  'LoopControlSignal|TemplateHelperKind::LoopControl|TemplateEmission::Break|TemplateEmission::Continue|TemplateLoopControlKind|unconsumed loop-control|escaped the nearest template loop' \
  src
```

No template-control signal path should remain.

The helper classification and loop-control nodes were already retired in Phase 2. This checkpoint removes the remaining fold-signal channel and impossible consumer arms. Independent review also found a pre-existing loss of consumed loop-body provenance at aggregate completion. The owning fold now transfers those facts before either output completion path, with regression coverage for emitted bodies, visited no-output selectors and zero-iteration exclusion.

- [x] Fold emissions contain only structural no-output or output.
- [x] Recursive node, conditional, loop and wrapper folds carry no signal state.
- [x] Selected option and loop bindings restore before errors propagate.
- [x] Wrappers retain output modes, structural anchors and projection order.
- [x] Const consumers use exhaustive matches without escape-signal errors.
- [x] Consumed loop-body provenance survives aggregate completion.
- [x] Focused validation and independent review, including correction verification, pass.

Commit:

```text
refactor: remove template loop-control fold signals
```

---

# Phase 5 - Simplify runtime handoff, HIR conditional lowering and slot application

## Purpose

Runtime templates should cross the AST/HIR boundary with the same small semantic shape. HIR should no longer carry template-only branch chains or special loop-control plumbing.

## Replace runtime branch chains with one conditional

In `runtime_handoff.rs` or the delivered neutral handoff owner, replace:

```text
BranchChain {
    branches
    fallback
    else_marker
}
```

with a singular conditional:

```text
Conditional {
    selector
    body
}
```

Keep any required source span.

Delete `OwnedRuntimeTemplateBranch` if it has no other purpose.

Update the immutable and mutable handoff walkers to recurse through one conditional body.

## Simplify TIR handoff materialization

In `tir/handoff_materialization.rs`:

- materialize one effective selector
- materialize one body
- stop allocating a branch vector
- stop allocating a fallback `Box`
- stop cloning an else marker
- remove loop-control-node materialization

Preserve the exact effective selector overlay lookup.

## Simplify HIR template dispatch

Update `hir/hir_expression/templates/mod.rs` and `control_flow.rs`:

- treat the singular conditional as structured runtime control flow
- remove `LoopControl` from control-flow classification
- delete validation that exists only to reject a top-level runtime-template loop-control node
- retain `ConditionalWrapper` and `Loop` handling

## Replace recursive branch-chain lowering

In `render_append.rs`:

Delete the current equivalents of:

- `OwnedRuntimeBranchChainAppend`
- branch-index recursion
- `append_owned_runtime_template_branch_chain`
- `append_owned_runtime_template_branch_chain_from_index`
- `append_owned_runtime_option_present_branch_chain_arm`
- fallback append helper

Add one direct conditional append operation.

### Boolean selector

Reuse the ordinary HIR `if` body-emitter owner.

The selected body emitter appends the conditional body.

The false emitter performs no body work.

Preserve runtime emitted-output tracking so a false conditional does not cause a parent output-conditioned wrapper to render.

### Option-present selector

Reuse the existing option-present HIR branch helper.

The present path binds the payload and appends the body.

The absent path performs no body work.

Do not add another option representation.

## Delete runtime template loop-control lowering

Delete the current equivalents of:

- `OwnedRuntimeTemplateNode::LoopControl`
- `emit_template_loop_control`
- special `append_nested_runtime_template_control_flow` branch for loop control
- any use of ordinary HIR `emit_break_to_current_loop` or `emit_continue_to_current_loop` that exists only for template nodes

Ordinary source loop lowering still uses those HIR operations and must remain untouched.

## Delete runtime-slot loop-control flush plumbing

This is a required adjacent simplification.

At plan-writing time runtime slot lowering carried a `RuntimeSlotLoopControlFlush` context so a contribution that emitted output before `[break]` or `[continue]` could replay its wrapper before jumping to the surrounding loop.

With template loop control removed, delete:

- `RuntimeSlotLoopControlFlush`
- `RuntimeTemplateAppendContext.loop_control_flush`
- `with_loop_control_flush`
- `flush_runtime_slot_application_for_loop_control`
- CFG blocks created only to flush before template loop control
- early slot-contribution returns for `Break` or `Continue`

Simplify the contribution pipeline accordingly.

At plan-writing time `RuntimeSlotContributionResult.emission` existed to return one of those signals. Reverify its consumers.

If it is no longer needed:

- remove the field
- let the result carry only the actual surviving facts such as emitted-output state and wrapper policy
- make contribution-content append return `Result<()>` when its returned emission is otherwise ignored

Do not leave a two-state field that is never queried.

## Move runtime emission bookkeeping to its real owner

At plan-writing time `TemplateBodyEmission` was defined under AST template-control-flow metadata but used only by HIR runtime template append code.

After removing `Break` and `Continue`, reverify all uses.

If the remaining `NoOutput`/`Output` state is HIR-only:

- remove it from `ast/templates/template_control_flow/types.rs`
- define a clearly named HIR-local enum near runtime append lowering, for example `RuntimeTemplateEmission`
- keep the enum rather than replacing it with a boolean if the names make wrapper and structural-output control flow clearer

This removes an AST-to-HIR ownership leak and is a required review item.

If another legitimate AST consumer exists at activation, keep the type in the narrowest shared neutral owner instead.

## Update AST finalization and semantic walkers

Replace branch-vector walking with singular conditional-selector handling in every current `OwnedRuntimeTemplateNode` consumer.

Current likely owners include:

- `ast/expressions/failure_classification.rs`
- `ast/module_ast/finalization/validate_types.rs`
- `ast/module_ast/finalization/debug_type_validation.rs`
- `ast/module_ast/finalization/normalize_ast.rs`

Preserve:

- failure classification of the one selector
- type validation of the one selector
- inactive assertion-message normalization in the one selector
- recursive handoff walking through the body

Remove `LoopControl` leaf cases.

## Prove no downstream analysis or backend special case remains

Search the borrow checker, lifetime analysis, link facts and both backends:

```bash
rg -n \
  'TemplateLoopControl|LoopControlSignal|RuntimeSlotLoopControl|OwnedRuntimeTemplateBranch|BranchChain|template loop control|template break|template continue' \
  src/compiler_frontend/analysis \
  src/compiler_frontend/hir \
  src/backends \
  src/build_system \
  src/projects
```

Expected result: template-specific branching and loop control should disappear before ordinary backend-facing HIR. Do not create borrow-checker or backend cleanup work without an actual hit.

If a current backend or analysis still has a template-specific feature flag or helper that became unreachable, delete it and add/update the narrow test that owned it.

## Focused validation

```bash
cargo fmt --all
cargo test -p moth --lib hir_expression_lowering -- --format terse
cargo test -p moth --lib template -- --format terse
cargo run --quiet -- tests --case <runtime-single-arm-if-case> --backend html
cargo run --quiet -- tests --case <runtime-option-capture-case> --backend html
cargo run --quiet -- tests --case <runtime-loop-case> --backend html
cargo run --quiet -- tests --case <runtime-slot-loop-case> --backend html
```

Run HTML-Wasm variants only for cases whose retained contract and current target support make them meaningful.

Commit:

```text
refactor: simplify runtime template control flow
```

---

# Phase 6 - Remove obsolete diagnostics and prune the old test surface

## Purpose

The compiler should describe the language that exists, not preserve a taxonomy for deleted template syntax.

## Remove obsolete diagnostic reasons

At plan-writing time `InvalidTemplateStructureReason` included many reasons that exist only because body sentinels were a supported grammar.

Reverify and remove the current equivalents of:

- `ElseInTemplateHead` if ordinary current-grammar parsing can own the failure
- `OrphanTemplateElse`
- `OrphanTemplateElseIf`
- `OrphanTemplateBreak`
- `OrphanTemplateContinue`
- `DuplicateTemplateElse`
- `TemplateElseIfAfterElse`
- `MalformedTemplateElse`
- `MalformedTemplateElseIf`
- `MalformedTemplateBreak`
- `MalformedTemplateContinue`
- `MissingTemplateElseIfCondition`
- `InlineTemplateElse`
- `InlineTemplateElseIf`
- `InlineTemplateBreak`
- `InlineTemplateContinue`
- `TemplateElseInLiteralBody`
- `TemplateElseIfInLiteralBody`
- `TemplateLoopControlInLiteralBody`
- `TemplateElseInLoopBody`
- `TemplateElseIfInLoopBody`

Delete their:

- reason-key mappings
- render arms
- dedicated parser construction
- unit tests
- integration assertions

Do not reuse a retired reason key for a different meaning.

## Preserve the diagnostics that describe surviving syntax

Retain and update only as needed:

- missing template `if` condition
- missing template loop header
- control-flow suffix ordering/finality
- unsupported match-style template control flow
- const-required conditional/loop diagnostics
- const loop expansion limit
- ordinary unexpected-token diagnostics

If wording says "branch chain", "fallback" or "loop-control signal", update it to the smaller current model.

## Prune old fixtures instead of rewriting all of them

Follow the testing guide:

- delete tests tied only to deleted implementation paths
- preserve duplicates only when they protect a distinct stage, backend, source location or invariant
- keep one primary owner per behaviour
- do not preserve obsolete fixtures for historical compatibility

Remove deleted fixture entries from `tests/cases/manifest.toml`.

Run:

```bash
cargo run --quiet -- tests --audit
cargo run --quiet -- tests --list --tag templates
```

Review the resulting template contract inventory. Do not leave a contract family whose only primary case was deleted.

## Exit search

```bash
rg -n \
  'OrphanTemplateElse|OrphanTemplateElseIf|OrphanTemplateBreak|OrphanTemplateContinue|DuplicateTemplateElse|TemplateElseIfAfterElse|MalformedTemplateElse|MalformedTemplateElseIf|MalformedTemplateBreak|MalformedTemplateContinue|MissingTemplateElseIfCondition|InlineTemplateElse|InlineTemplateElseIf|InlineTemplateBreak|InlineTemplateContinue|TemplateElseInLiteralBody|TemplateElseIfInLiteralBody|TemplateLoopControlInLiteralBody|TemplateElseInLoopBody|TemplateElseIfInLoopBody' \
  src tests
```

Expected result: zero live implementation/test references.

Commit:

```text
test: prune removed template control-flow coverage
```

---

# Phase 7 - Update canonical documentation and adjacent plans

## Purpose

Publish the simplified language as final design and remove architectural wording that still assumes branch chains, fallbacks or template loop-control signals.

## Canonical user-facing documentation

Update:

- `docs/src/docs/templates/template-control-flow.mtf`
- `docs/src/docs/templates/template-control-flow-basic.mtf`
- `docs/src/docs/constants/const-templates.mtf`
- `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf`

Review and update when affected:

- `docs/src/docs/templates/template-basics.mtf`
- `docs/src/docs/moth-templates/moth-template-limits.mtf`
- other canonical template pages found by the activation search

### Required documentation message

State the model directly:

- template `if` is single-arm conditional inclusion
- template option capture is single-arm conditional inclusion with a scoped payload binding
- template `loop` repeats content
- template `else`, `else if`, `break` and `continue` are not part of the language
- richer decision-making belongs in ordinary Moth code

Avoid spending paragraphs cataloguing removed historical behaviour. The canonical docs should describe the final language.

Use examples instead of prose where that is clearer.

Do not imply that runtime function extraction is a universal replacement inside const-required `.mtf` sources. Explain the boundary accurately and briefly.

## Canonical compiler architecture

Update `docs/compiler-design-overview.md`, especially `Templates and TIR`.

Remove wording that requires:

- branch chains
- branches plus fallbacks
- else markers
- loop-control helpers/signals

Describe the final structure generically enough to survive routine Rust type renames:

- TIR retains a single conditional selector/body
- TIR retains loop header/body structure
- exact-view traversal covers conditional bodies and loops
- const preparation validates retained reachable control-flow structure
- runtime finalization preserves lazy conditional and loop bodies

Keep TIR AST-local and neutral runtime handoff boundaries unchanged.

## Developer compiler documentation

Update the current equivalent of:

- `docs/src/developer-docs/compiler-design/templates-and-tir/templates-and-tir.mtf`

Remove branch-chain/fallback/loop-control descriptions and diagrams.

Do not turn the developer page into a source-file inventory.

## Progress matrix

Update:

- `docs/src/docs/progress/@page.moth`

The Templates row should report the surviving implemented surface:

- single-arm Boolean conditional
- option-present capture
- template loops
- slots/wrappers/directives as otherwise unchanged

Remove claims that `else`, `else if`, structural break or structural continue are supported.

Do not add "removed" history to the canonical language reference. A brief progress note is acceptable only if the matrix's current-status format needs it.

## Repository index

Update `index.md` if the module map still names:

- `template_body_sentinels.rs`
- branch-chain ownership
- loop-control-specific template owners

Keep the index navigational.

## Review adjacent future plans

Search all live roadmap plans for assumptions about the removed template surface:

```bash
rg -n \
  'template.*else|template.*break|template.*continue|BranchChain|TemplateElseMarker|TemplateLoopControl|loop-control signal|branch chain|fallback branch' \
  docs/roadmap
```

Edit a future plan only when it would otherwise instruct a future agent to depend on or restore the removed feature.

Do not rewrite references to ordinary source-language `break`, `continue` or `else`.

Likely review targets include plans that discuss:

- template parser optimisation
- compiler diagnostics
- terminality/Never analysis
- typed semantic expression/TIR migration

The current Never plan, for example, contains substantial ordinary loop `break`/`continue` analysis. That is not template loop control and must remain.

## Generated documentation

Do not hand-edit `docs/release/**`.

Rebuild through the normal docs build:

```bash
cargo run --quiet -- build docs --release
cargo run --quiet -- check docs --terse
```

Review the generated diff to make sure removed template syntax no longer appears in generated template/const/cheatsheet pages.

## Documentation search

After source docs are updated:

```bash
rg -n '\[else|\[break\]|\[continue\]' docs/src
```

Every remaining hit must be:

- ordinary Moth control flow outside templates
- an explicitly invalid example that the canonical docs still need
- unrelated literal text

Do not leave historical template examples by accident.

Commit:

```text
docs: define minimal template control flow
```

---

# Phase 8 - Mandatory post-removal simplification and optimisation investigation

## Purpose

The initial removal will make additional code obviously redundant. This phase is mandatory. Do not stop at "tests pass".

Use the code-review guide to inspect the resulting system as if the old feature had never existed.

## 8.1 Dead representation and state audit

Search for old concepts and suspicious survivors:

```bash
rg -n \
  'BranchChain|branch_chain|fallback|TemplateIrBranch|OwnedRuntimeTemplateBranch|TemplateElse|ElseSentinel|LoopControlSignal|TemplateLoopControl|RuntimeSlotLoopControl|loop_control_flush|TemplateEmission::Break|TemplateEmission::Continue|TemplateBodyControlContext' \
  src/compiler_frontend/ast/templates \
  src/compiler_frontend/hir/hir_expression/templates
```

For `fallback`, classify each hit because the word has unrelated legitimate uses elsewhere.

Questions:

- Is any enum now one-variant?
- Is any context struct carrying a field whose only consumer was removed?
- Is any helper called by one site merely because it used to abstract multiple branch kinds?
- Is any `Vec` now always length one?
- Is any `Option` now always `None` or always `Some`?
- Is any traversal returning state that no caller reads?
- Is any copied/cloned metadata no longer needed?
- Is any source span retained solely for a deleted marker?
- Is any "control-flow" abstraction now better expressed directly as conditional or loop handling?
- Did any test builder keep methods for constructing deleted nodes?

Delete or simplify those cases.

## 8.2 Preparation/helper audit

Recheck the helper and final-value model.

If `TemplateHelperKind` has become a wrapper around only slot insertion, collapse it as described in Phase 4.

Review whether any `TemplateConstValueKind` variant exists only because loop-control helpers used to be representable.

Ensure preparation has one direct classification path for the remaining values.

Do not build a broader helper abstraction in anticipation of hypothetical future template controls.

## 8.3 HIR runtime append audit

Recheck:

- `RuntimeTemplateAppendContext`
- slot contribution result structs
- nested runtime-control-flow append helpers
- control-flow detection helpers

Delete fields and helper layers made redundant by removing loop-control flushing and branch recursion.

In particular, investigate whether a helper named for "nested runtime template control flow" still owns any special behaviour after loop-control special casing disappears. If it merely delegates to the general append path, delete it.

Keep explicit `NoOutput`/`Output` state if it still explains wrapper emission. Do not reduce a meaningful enum to a boolean just to save a line.

## 8.4 TIR render-unit and store audit

Review whether the old generic branch/fallback body-addressing abstraction still exists.

Prefer:

- one direct conditional body
- one direct loop body

over:

- index-based branch selection
- generic "branch or fallback" helpers
- tuple-heavy extraction of optional body categories

Review clones introduced only to avoid borrowing a branch vector. A single node ID can often be copied directly before a mutable store phase.

## 8.5 Analysis and backend audit

Prove there is no redundant work downstream.

Inspect:

- AST finalization
- failure/effect classification
- HIR validation
- borrow problem extraction
- borrow checker
- lifetime/escape analysis
- reachability/link facts
- backend feature validation
- JavaScript lowering
- Wasm lowering

The expected result is that templates become ordinary HIR control flow before those generic analyses.

If there are no template-specific hits, record that result and make no artificial changes.

If a special case exists solely for removed template `else`/loop-control semantics, delete it and preserve its surviving contract with the narrowest test.

## 8.6 Allocation and traversal audit

The removal should naturally eliminate:

- branch vectors for single conditional templates
- fallback boxes
- fallback/branch traversal loops
- loop-control signal `Option`s
- runtime-slot loop-control context state
- some temporary CFG blocks in runtime slot lowering

Do not claim a performance improvement from code deletion alone.

Use the surviving template benchmarks to check for regressions:

```bash
just bench-check
```

Compare the same surviving workload cohort against the activation baseline.

If a template-heavy case regresses repeatably or a remaining traversal looks expensive, use the repository profiling workflow on that case before changing an algorithm.

Do not add caching, parallelism or a new compact representation without evidence. Those belong to their existing owners.

## 8.7 Test bloat audit

Use the testing guide to compare the final suite against the simplified contract.

For each template-control test:

- identify its primary semantic owner
- delete redundant representation-shaped tests
- keep stage-local tests only for hidden invariants
- remove obsolete fixture helpers and builders
- remove manifest cases that no longer own a distinct contract

Run:

```bash
cargo run --quiet -- tests --audit
```

## 8.8 Documentation and comment audit

Search for stale terminology in:

- Rust module docs
- WHAT/WHY comments
- compiler error text
- canonical docs
- developer docs
- index
- roadmap plans

No comment should claim:

- branch-chain control flow
- fallback template branches
- nearest active template loop for body markers
- loop-control signals
- wrapper flush before template break/continue

Keep ordinary source-control comments intact.

## Required post-removal report in the agent handover

Before final validation, write a concise working-note report containing:

- what redundant types/functions/files were deleted beyond the obvious syntax path
- which possible simplifications were investigated and deliberately kept
- whether any borrow/backend/link analysis had template-specific work to delete
- benchmark comparison for the surviving template cohort
- any real follow-up found that is outside this plan

Do not commit a permanent audit document unless repository policy requires one.

Commit any justified cleanup from this phase separately:

```text
refactor: clean up simplified template pipeline
```

---

# Phase 9 - Mandatory style-guide and code-quality review

## Purpose

Perform a full review after the implementation is structurally complete, not while the old and new representations coexist.

The review must use `.agents/skills/moth-code-sniffer/SKILL.md` (the supplied review guide, as confirmed by the user) and current style/testing/validation authorities.

## Required review reads

Fully read the current:

- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/src/developer-docs/memory-management/overview.mtf`
- relevant canonical template docs

## Audit the final diff

Review every changed Rust file and the resulting owning modules for:

- duplicated or near-duplicated helpers
- unnecessary wrapper functions
- one-caller abstractions left from the old design
- stale compatibility paths
- stale marker/signal comments
- avoidable `Vec`, `Box`, `Option` or clone use
- long parameter lists made obsolete by deleted state
- parser/TIR/HIR stage-boundary leaks
- broad visibility that can be narrowed
- dead code not caught by Clippy
- internal `expect`/`unwrap` assumptions no longer justified
- diagnostics that use an infrastructure failure for user syntax
- tests that assert incidental indexes or old representation details
- files that still manage several unrelated concepts
- opportunities to delete code instead of extracting another abstraction

Explicitly answer the code-review guide's core questions in working notes.

### Required decisions

For every significant duplicate or helper under review, classify it as:

- remove
- merge
- move
- split
- leave local

If left local, state why abstraction would make ownership less clear.

## Architecture check

Confirm:

- AST owns template semantics and TIR
- TIR remains AST-local
- runtime handoff contains neutral owned data only
- HIR receives no TIR IDs or views
- HIR runtime template emission bookkeeping is not owned by AST unless it is truly shared
- ordinary source-loop analysis was not accidentally coupled to template syntax
- builders/backends do not interpret template source structure

## Test-quality check

Confirm:

- user-visible retained behaviour has integration coverage
- internal TIR shape has only focused invariant coverage
- deleted feature tests are actually deleted
- failure cases assert stable diagnostics rather than rendered prose unless prose is contractual
- manifest contract ownership remains coherent
- no benchmark is being used as a correctness test

## Resolve findings

Fix all in-scope review findings before the final gate.

For a legitimate out-of-scope issue:

- do not expand this plan silently
- report it in the handover with exact owner and reason
- add it to an existing owner only if current roadmap policy requires that action

Commit review corrections if needed:

```text
refactor: address template simplification review
```

---

# Phase 10 - Final validation, documentation verification and plan retirement

## Focused final checks

Run focused tests first so failures are attributable:

```bash
cargo fmt --all
cargo test -p moth --lib template -- --format terse
cargo test -p moth --lib tir -- --format terse
cargo test -p moth --lib hir_expression_lowering -- --format terse
cargo run --quiet -- tests --list --tag templates
cargo run --quiet -- tests --tag templates --backend html
cargo run --quiet -- tests --audit
cargo run --quiet -- check docs --terse
cargo run --quiet -- build docs --release
```

Adapt filters to the current suite if names changed.

Run directly affected HTML-Wasm cases where the retained feature is supported and already has that backend contract.

## Source absence checks

Run:

```bash
rg -n \
  'TemplateElseMarker|ElseSentinelPolicy|DirectElseMarker|TemplateBodyControlContext|TemplateIrBranch|OwnedRuntimeTemplateBranch|TemplateLoopControlKind|LoopControlSignal|RuntimeSlotLoopControlFlush|TemplateEmission::Break|TemplateEmission::Continue|TemplateBodyEmission::Break|TemplateBodyEmission::Continue' \
  src tests

rg -n '\[else|\[break\]|\[continue\]' docs/src
```

Inspect every result.

Also search generated docs for stale supported-syntax examples after rebuilding.

## Routine checkpoint

Run:

```bash
just validate
```

Fix all failures.

## Final merge gate

On the final integrated tree run:

```bash
just validate-full
```

Record:

- tested revision
- commands run
- any relevant environment override
- benchmark comparison
- docs build result

Do not claim an unrun validation family passed.

## Completion acceptance criteria

The plan is complete only when all are true:

### Language surface

- [ ] Template `if` has one body and no fallback.
- [ ] Option-present capture remains supported.
- [ ] Template loops remain supported without template `break` or `continue`.
- [ ] Removed body markers are rejected by the surviving grammar.
- [ ] Ordinary source `else`, `break` and `continue` are unaffected.
- [ ] Structural no-output semantics are preserved.
- [ ] Runtime selected-body laziness is preserved.
- [ ] Const-required semantics are preserved without a new source-function evaluator.

### Parser

- [ ] `template_body_sentinels.rs` or its replacement sentinel subsystem is gone.
- [ ] No template body-control context or active template-loop sentinel depth remains.
- [ ] No else-if or fallback parser remains.
- [ ] No template loop-control marker parser remains.

### TIR

- [ ] Conditional TIR is singular.
- [ ] No branch vector exists solely for template `else if`.
- [ ] No fallback TIR body exists.
- [ ] No authored else marker metadata exists.
- [ ] No template loop-control TIR node exists.
- [ ] Structural TIR walkers handle only the retained conditional and loop shapes.

### Folding and preparation

- [ ] No loop-control helper classification remains.
- [ ] No loop-control const-value classification remains.
- [ ] Constant-fold emission contains only no-output/output.
- [ ] Fold traversal carries no possible loop-control signal.
- [ ] Loop folding performs ordinary iteration without signal matching.
- [ ] Wrapper folding has no signal propagation.

### Runtime handoff and HIR

- [ ] Runtime handoff has one conditional body and no fallback.
- [ ] `OwnedRuntimeTemplateBranch` or equivalent branch-chain carrier is gone.
- [ ] No runtime loop-control node exists.
- [ ] No runtime-slot loop-control flush context exists.
- [ ] No template-specific HIR break/continue emission exists.
- [ ] Surviving runtime output-state bookkeeping lives in its narrowest owner.

### Diagnostics and tests

- [ ] Old orphan/duplicate/malformed/inline template body-control reasons are gone.
- [ ] Removed syntax has focused current-grammar rejection coverage.
- [ ] Obsolete success and diagnostic fixtures are deleted.
- [ ] Surviving behaviours have one clear primary test owner.
- [ ] Test manifest and audit are clean.

### Documentation

- [ ] Canonical template control-flow docs describe only the retained surface.
- [ ] Const-template docs use no removed syntax.
- [ ] Cheatsheet uses no removed syntax.
- [ ] Compiler architecture contains no branch-chain/fallback/loop-control requirement.
- [ ] Progress matrix reports the simplified supported surface.
- [ ] Developer TIR docs and index match current ownership.
- [ ] Generated docs have been rebuilt.
- [ ] Adjacent future plans do not instruct later work to restore removed template semantics.

### Quality and performance

- [ ] Post-removal simplification audit completed.
- [ ] Style-guide/code-review audit completed.
- [ ] No stale compatibility layer remains.
- [ ] No unjustified one-variant enum or unused context field remains.
- [ ] No avoidable vector/fallback allocation remains for single conditionals.
- [ ] Surviving template benchmarks show no unexplained regression.
- [ ] `just validate` passes.
- [ ] `just validate-full` passes on the final integrated tree.

## Plan retirement

Move any durable design information discovered during implementation into the canonical language/compiler docs before completion.

If this plan was committed under `docs/roadmap/plans/`, delete it and remove its roadmap bullet in the completion commit according to roadmap policy.

The final agent handover should include:

- final commit/revision
- concise summary of deleted language/compiler machinery
- retained template surface
- tests deleted, rewritten and added
- documentation updated
- post-removal cleanup findings
- benchmark comparison
- final style/code-quality review result
- validation commands and outcomes
- any real out-of-scope follow-up
