# Runtime template structural helper escape diagnostics - quick fix plan

> **For agentic workers:** Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` to implement this plan. Track steps with `- [ ]`.

**Status:** Queued. Small correctness and diagnostic cleanup with no prerequisite.

**Goal:** Diagnose any template slot or directive helper that escapes compile-time composition into a runtime value, while preserving valid constant helpers and fully resolved runtime templates.

**Architecture:** Reuse the existing AST-owned TIR preparation and finalization path. Preparation must retain structural-helper facts independently of whether a nested expression makes the template runtime-dependent. The final value boundary rejects unresolved helpers before HIR sees a neutral runtime template handoff. No runtime slot value type, new compiler pass or parallel TIR walk is required.

**Tech stack:** Rust frontend, typed compiler diagnostics, Moth integration cases and canonical `.mtf` documentation.

**Specification:** `docs/docs/templates/template-slots.mtf`, `docs/docs/templates/template-directives.mtf`, `docs/docs/constants/const-templates.mtf` and `docs/compiler-design-overview.md` under `Frontend stages > Stage 4: AST semantics > Templates and TIR`. This plan records the accepted rule that structural template directives resolve during compilation even when their content executes at runtime.

## Required behaviour

- A final runtime `String` may contain runtime expressions and routed runtime slot content, but must not retain an unresolved `$in` helper, slot declaration or other structural directive artifact.
- A nonconstant binding or ordinary function must be diagnosed when its template value would escape with such an artifact. Diagnose functions even when no call executes them.
- A compile-time constant may retain an open wrapper or a stored `$in` for later valid composition. A constant template that fully resolves its helpers may fold normally.
- An unresolved helper is never made legal by a `String` return annotation, a runtime marker or an unused binding.
- Existing resolved semantics remain intact: absent slot content renders the documented default or empty value, `$supplied` presence follows routing and a guard resolved to no output is not an escaped artifact.
- The active entry's direct constant builder-root `$in` remains a separate supported destination. An ordinary or runtime template must not acquire builder-slot routing through aliasing.
- Const functions may eventually construct these shapes only if their evaluation and subsequent composition resolve them at compile time. Do not extend const-function syntax, evaluators or runtime capabilities in this fix.

## Current owners and suspected gap

Reconfirm against the implementation branch before editing:

- `src/compiler_frontend/ast/templates/tir/preparation.rs`: `prepare_tir_view_in_scope` derives the final kind and outcome, while `record_runtime` retains the first reason. A dynamic expression in a `SlotInsert` root can yield `NonConst` with `RuntimeExpression` rather than a helper classification. `TemplatePreparationFacts` already carries escaped-insert information, but its meaning for an escaped root needs checking.
- `src/compiler_frontend/ast/module_ast/finalization/normalize_ast.rs`: `normalize_expression_template_node` rejects `SlotInsertHelper` and `materialize_runtime_template_handoff_for_hir` rejects a `Runtime(SlotContribution)`. Those checks must not depend on the first runtime reason when structural helpers remain.
- `src/compiler_frontend/ast/templates/create_template_node.rs` and `src/compiler_frontend/ast/templates/template_control_flow/validation.rs` already check some escaped inserts. `src/compiler_frontend/ast/statements/functions.rs` checks helper values in constant parameter defaults. Reuse or consolidate existing facts and diagnostics rather than adding another scanner.
- `src/compiler_frontend/compiler_messages/render/templates.rs` renders `HelperOutsideWrapperSlot` and `InsertOutsideParentSlot`. Keep these typed diagnostic families unless a genuinely distinct source rule requires another reason.

The explanation above is a source-inspection hypothesis, not an executed reproduction. Verify it first. Avoid broadening the rule to valid runtime template expressions, resolved runtime slot plans or legal constant wrappers.

## Task 1 - Reproduce the boundary and protect both sides

**Test owners:** `src/compiler_frontend/ast/module_ast/finalization/tests/normalize_ast_runtime_tests.rs`, relevant `src/compiler_frontend/ast/templates/tir/tests/` and existing `tests/cases/` template scenarios.

- [ ] Reproduce the reported ordinary function returning `[$in style:...]` with a runtime-dependent interpolation and a `String` return, including an unused function. Add a failing real-source case that asserts a typed helper/slot diagnostic at the authored insert rather than silent acceptance or a generic nonconstant error.
- [ ] Reproduce a nonconstant declaration or local binding retaining the same unresolved helper. Check both a direct value and a nested expression where an earlier runtime reason may mask the helper.
- [ ] Extend the existing final-view unit test to cover a dynamic root `SlotInsert`. Assert that preparation/finalization cannot classify its escaped helper as an ordinary runtime `String`.
- [ ] Reuse existing success cases for stored constant inserts and wrapper application. Add one meaningful runtime case where a function returns plain CSS `String` content and a direct `$in style` supplies an imported wrapper's slot.
- [ ] Preserve the existing unknown named target diagnostic, a resolved runtime `if` or loop contribution, slot defaults, guard-only presence and direct constant builder-root inserts. Extend current scenario owners rather than creating one fixture per syntax variation.
- [ ] Run the focused cases before the fix and record the red reproduction and currently passing controls.

**Gate:** A real Moth program demonstrates the silent escape, and tests distinguish that bug from supported composition.

## Task 2 - Fix the existing AST/TIR boundary

**Production owners:** `src/compiler_frontend/ast/templates/tir/preparation.rs` and `src/compiler_frontend/ast/module_ast/finalization/normalize_ast.rs`, with adjacent owners changed only where required by a proved gap.

- [ ] Make preparation's escaped structural-helper identity independent of runtime-dependence classification. Preserve the distinction between a standalone escaped helper and contributions already routed into an owning template.
- [ ] Reject any unresolved helper at the existing final runtime-value boundary, including function returns and nonconstant bindings, before constructing the neutral HIR handoff. Use existing preparation facts and authored source spans. Do not add a second TIR walker, new runtime representation or parser-side duplicate rule.
- [ ] Keep the constant-helper path open until legal compile-time composition finishes. Check constant parameter defaults and nested-value normalization for bypasses without rejecting valid stored inserts.
- [ ] Produce a typed source diagnostic explaining that `$in` and other structural directives require compile-time resolution. Prefer the existing helper/slot reason and include the authored helper span. Advise placing the insert in the receiving template and returning ordinary `String` content from a runtime function.
- [ ] Rerun the red tests and positive controls. Check that finalization, rather than HIR or a backend, owns the rejection.

**Gate:** Escaped structural helpers cannot cross the AST-to-HIR boundary. Ordinary runtime content and resolved constant/runtime composition continue to work.

## Task 3 - Document, validate and retire

**Documentation owners:** `docs/docs/templates/template-slots.mtf` and, only where the existing description needs clarification, `docs/docs/templates/template-directives.mtf` and `docs/docs/constants/const-templates.mtf`.

- [ ] Add a concise explicit rule and one user-facing example distinguishing invalid runtime function-produced `$in` from a runtime function producing CSS text passed through a direct `$in style` inside the receiver. State that constant stored inserts are legal only while compilation can subsequently resolve them.
- [ ] Keep the compiler overview's TIR invariant consistent without duplicating the full user-facing explanation. Update `docs/docs/progress/@entry.moth` if rejection coverage changes.
- [ ] Run focused frontend unit tests, the affected `moth tests` scenarios and documentation checks. At the relevant checkpoints follow `docs/developer-docs/style-guide/validation.mtf`, including the required final `just validate-full` evidence before merging into `main`.
- [ ] Perform the repository Slice review. Remove redundant checks, stale comments and duplicate fixture coverage. Check diagnostic spans and ensure no structural TIR state leaks into HIR.
- [ ] In the same commit that completes the fix, delete this plan and remove its entry from `docs/roadmap/roadmap.md`. Do not commit a completed plan as outstanding work.

**Gate:** The regression is demonstrated red then green, positive controls remain green, the user-facing contract is documented and the roadmap contains no finished-plan residue.
